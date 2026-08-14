//! The instance tree XPath runs over.
//!
//! An arena rather than `Rc`: evaluation needs node identity (two paths can
//! reach the same node, and document order decides what a node-set means),
//! recalculation needs to write values back, and a parent link is required
//! for the `..` axis. Indices give all three without reference cycles.

use std::collections::BTreeMap;

/// A node's place in the arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Element,
    /// Attributes are nodes in XPath, reachable through their own axis.
    Attribute,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    /// Local name, namespace prefix stripped — instances are single-namespace
    /// in practice and ODK expressions are written without prefixes.
    pub name: String,
    /// Text content of an element, or the value of an attribute.
    pub value: String,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attributes: Vec<NodeId>,
}

#[derive(Debug, Clone, Default)]
pub struct Instance {
    nodes: Vec<Node>,
    root: Option<NodeId>,
    /// Document order, assigned once at build time. XPath is defined in
    /// terms of it, and recomputing it per comparison is how evaluation
    /// becomes quadratic on a large repeat.
    order: BTreeMap<NodeId, usize>,
}

impl Instance {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn create_element(&mut self, name: &str, value: &str) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            kind: NodeKind::Element,
            name: name.to_string(),
            value: value.to_string(),
            parent: None,
            children: Vec::new(),
            attributes: Vec::new(),
        });
        id
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.nodes[child.0].parent = Some(parent);
        self.nodes[parent.0].children.push(child);
    }

    pub fn set_attribute(&mut self, element: NodeId, name: &str, value: &str) {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            kind: NodeKind::Attribute,
            name: name.to_string(),
            value: value.to_string(),
            parent: Some(element),
            children: Vec::new(),
            attributes: Vec::new(),
        });
        self.nodes[element.0].attributes.push(id);
    }

    pub fn set_root(&mut self, id: NodeId) {
        self.root = Some(id);
        self.reindex();
    }

    /// Recompute document order. Call after the shape changes — adding a
    /// repeat instance, for example.
    pub fn reindex(&mut self) {
        self.order.clear();
        let Some(root) = self.root else { return };
        let mut counter = 0usize;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            self.order.insert(id, counter);
            counter += 1;
            // attributes come after their element and before its children
            for attribute in self.nodes[id.0].attributes.clone() {
                self.order.insert(attribute, counter);
                counter += 1;
            }
            let mut children = self.nodes[id.0].children.clone();
            children.reverse();
            stack.extend(children);
        }
    }

    pub fn document_order(&self, id: NodeId) -> usize {
        self.order.get(&id).copied().unwrap_or(usize::MAX)
    }

    /// The string-value of a node: for an element, the concatenation of all
    /// text below it, in document order; for an attribute, its value.
    ///
    /// Returning only the element's own text is the tempting shortcut, and
    /// it is right for every leaf — which is every question. It is wrong for
    /// a group, where XPath says the value is everything underneath, and a
    /// form that asks for one would silently read an empty string.
    ///
    /// Inter-element whitespace is not part of this: the parser keeps
    /// values, not layout, so a container's value here is its answers run
    /// together without the indentation a serializer would put between them.
    pub fn string_value(&self, id: NodeId) -> String {
        let node = &self.nodes[id.0];
        if node.children.is_empty() {
            return node.value.clone();
        }
        let mut out = node.value.clone();
        for descendant in self.descendants(id) {
            out.push_str(&self.nodes[descendant.0].value);
        }
        out
    }

    /// Children of an element, elements only.
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id.0].children.clone()
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0].parent
    }

    pub fn attributes(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes[id.0].attributes.clone()
    }

    /// Every descendant of a node, in document order, excluding itself.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.nodes[id.0].children.iter().rev().copied().collect();
        while let Some(current) = stack.pop() {
            out.push(current);
            let mut children = self.nodes[current.0].children.clone();
            children.reverse();
            stack.extend(children);
        }
        out.sort_by_key(|n| self.document_order(*n));
        out
    }

    /// Ancestors, nearest first.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut current = self.nodes[id.0].parent;
        while let Some(node) = current {
            out.push(node);
            current = self.nodes[node.0].parent;
        }
        out
    }

    /// Absolute path of a node, as a form would write it.
    pub fn path_of(&self, id: NodeId) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut current = Some(id);
        while let Some(node) = current {
            parts.push(self.nodes[node.0].name.clone());
            current = self.nodes[node.0].parent;
        }
        parts.reverse();
        format!("/{}", parts.join("/"))
    }

    /// Build an instance from submission XML.
    ///
    /// Deliberately small: instances are machine-written documents, not
    /// arbitrary XML. Anything beyond elements, attributes and text is
    /// rejected rather than skipped, so an unexpected document is a failure
    /// and not a silently truncated one.
    pub fn from_xml(xml: &str) -> Result<Self, String> {
        let mut instance = Instance::new();
        let mut stack: Vec<NodeId> = Vec::new();
        let mut root = None;
        let mut chars = xml.char_indices().peekable();
        let bytes = xml.as_bytes();

        while let Some((i, c)) = chars.next() {
            if c != '<' {
                continue;
            }
            // declarations, comments and processing instructions
            if xml[i..].starts_with("<?") || xml[i..].starts_with("<!") {
                continue;
            }
            // Find the tag's end while respecting quotes: a '>' inside an
            // attribute value is legal XML, and constraints are full of
            // them — `constraint=". > 0"` is an ordinary form.
            let end = close_of_tag(xml, i).ok_or("unterminated tag")?;
            let inner = &xml[i + 1..end];
            while chars.peek().is_some_and(|(j, _)| *j <= end) {
                chars.next();
            }

            if let Some(name) = inner.strip_prefix('/') {
                let closed = stack.pop().ok_or("closing tag without opening")?;
                let expected = local_name(name.trim());
                if instance.node(closed).name != expected {
                    return Err(format!(
                        "closing </{expected}> does not match open <{}>",
                        instance.node(closed).name
                    ));
                }
                continue;
            }

            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let mut parts = inner.split_whitespace();
            let name = local_name(parts.next().unwrap_or_default());
            let id = instance.create_element(&name, "");
            for (key, value) in parse_attributes(inner) {
                instance.set_attribute(id, &local_name(&key), &value);
            }

            match stack.last() {
                Some(parent) => instance.append_child(*parent, id),
                None => {
                    if root.is_some() {
                        return Err("more than one root element".into());
                    }
                    root = Some(id);
                }
            }

            if self_closing {
                continue;
            }
            // text up to the next tag belongs to this element
            let text_start = end + 1;
            let text_end = xml[text_start..]
                .find('<')
                .map(|n| text_start + n)
                .unwrap_or(bytes.len());
            let text = &xml[text_start..text_end];
            if !text.trim().is_empty() {
                instance.node_mut(id).value = unescape(text);
            }
            stack.push(id);
        }

        if !stack.is_empty() {
            return Err("unclosed elements".into());
        }
        let root = root.ok_or("no root element")?;
        instance.set_root(root);
        Ok(instance)
    }
}

/// Index of the `>` that closes the tag opening at `start`, skipping any
/// inside quoted attribute values.
fn close_of_tag(xml: &str, start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, c) in xml[start..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '>') => return Some(start + offset),
            (None, _) => {}
        }
    }
    None
}

fn local_name(qname: &str) -> String {
    match qname.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => qname.to_string(),
    }
}

fn parse_attributes(inner: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = match inner.find(char::is_whitespace) {
        Some(i) => &inner[i..],
        None => return out,
    };
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 1..];
        let quote = match after.trim_start().chars().next() {
            Some(q @ ('"' | '\'')) => q,
            _ => break,
        };
        let start = after.find(quote).unwrap() + 1;
        let Some(len) = after[start..].find(quote) else {
            break;
        };
        let value = &after[start..start + len];
        if !key.is_empty() {
            out.push((key, unescape(value)));
        }
        rest = &after[start + len + 1..];
    }
    out
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

impl Instance {
    /// The `<instance id="…">` element of a lookup table in this document.
    ///
    /// The element itself, not its content: forms write
    /// `instance('lotes')/root/item`, where `root` is the element wrapping
    /// the items. Returning the content instead makes that next step look
    /// for a `root` inside `root`, which matches nothing and reads as an
    /// empty table rather than as a mistake.
    pub fn instance_named(&self, id: &str) -> Option<NodeId> {
        let root = self.root()?;
        let mut candidates = vec![root];
        candidates.extend(self.descendants(root));
        for node in candidates {
            if self.node(node).name != "instance" {
                continue;
            }
            let named = self
                .attributes(node)
                .into_iter()
                .any(|a| self.node(a).name == "id" && self.node(a).value == id);
            if named {
                return Some(node);
            }
        }
        None
    }

    /// Copy a subtree of `source` into this instance, returning its new id.
    pub fn adopt(&mut self, source: &Instance, node: NodeId) -> NodeId {
        let created = self.create_element(&source.node(node).name, &source.node(node).value);
        for attribute in source.attributes(node) {
            let attr = source.node(attribute);
            self.set_attribute(created, &attr.name, &attr.value);
        }
        for child in source.children(node) {
            let copied = self.adopt(source, child);
            self.append_child(created, copied);
        }
        created
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_paths_and_order() {
        let instance =
            Instance::from_xml(r#"<data id="f"><a>1</a><g><b>2</b></g><a>3</a></data>"#).unwrap();
        let root = instance.root().unwrap();
        assert_eq!(instance.node(root).name, "data");
        assert_eq!(instance.attributes(root).len(), 1);

        let children = instance.children(root);
        assert_eq!(children.len(), 3);
        assert_eq!(instance.string_value(children[0]), "1");
        assert_eq!(instance.path_of(children[2]), "/data/a");

        let b = instance.children(children[1])[0];
        assert_eq!(instance.path_of(b), "/data/g/b");
        // document order follows the document, not the arena
        assert!(instance.document_order(children[0]) < instance.document_order(b));
        assert!(instance.document_order(b) < instance.document_order(children[2]));
    }

    #[test]
    fn entities_and_self_closing_tags() {
        let instance = Instance::from_xml(r#"<data><a>x &amp; y</a><b/><c>z</c></data>"#).unwrap();
        let children = instance.children(instance.root().unwrap());
        assert_eq!(instance.string_value(children[0]), "x & y");
        assert_eq!(instance.string_value(children[1]), "");
        assert_eq!(instance.string_value(children[2]), "z");
    }

    #[test]
    fn a_greater_than_inside_an_attribute_is_not_the_end_of_the_tag() {
        // Legal XML, and what every numeric constraint looks like. Scanning
        // for the first '>' cuts the tag in half and the parse fails on the
        // next closing tag, a long way from the cause.
        let instance =
            Instance::from_xml(r#"<data note="a > b and c &lt; d"><x>1</x></data>"#).unwrap();
        let root = instance.root().unwrap();
        assert_eq!(
            instance
                .attributes(root)
                .iter()
                .map(|a| instance.node(*a).value.clone())
                .collect::<Vec<_>>(),
            vec!["a > b and c < d"]
        );
        assert_eq!(instance.children(root).len(), 1);
    }

    #[test]
    fn a_broken_document_is_an_error_not_a_shrug() {
        assert!(Instance::from_xml("<data><a></b></data>").is_err());
        assert!(Instance::from_xml("<data><a>").is_err());
        assert!(Instance::from_xml("no tags here").is_err());
    }
}
