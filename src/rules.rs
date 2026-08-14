//! The rules a form carries, and what they say about a filled instance.
//!
//! A form is not only a list of questions: it is a small program. `relevant`
//! decides what is asked, `calculate` derives values from other answers,
//! `constraint` and `required` decide what counts as a valid answer. Those
//! expressions reference each other, so the order they run in is a
//! dependency graph — and getting that order wrong does not crash anything,
//! it just produces a different form than the one that was designed.

use std::collections::{BTreeMap, BTreeSet};

use crate::eval::{evaluate, Context, Environment, Value};
use crate::parser::{Expr, Step};
use crate::tree::{Instance, NodeId};

/// One binding: everything the form says about one path.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Absolute path in the instance, e.g. `/data/household/resident/age`.
    pub path: String,
    pub relevant: Option<Expr>,
    pub calculate: Option<Expr>,
    pub constraint: Option<Expr>,
    pub required: Option<Expr>,
    pub readonly: Option<Expr>,
    pub constraint_message: Option<String>,
    pub required_message: Option<String>,
    /// Where in the spreadsheet this came from, for error messages.
    pub row: usize,
}

impl Binding {
    pub fn new(path: impl Into<String>) -> Self {
        Binding {
            path: path.into(),
            relevant: None,
            calculate: None,
            constraint: None,
            required: None,
            readonly: None,
            constraint_message: None,
            required_message: None,
            row: 0,
        }
    }
}

/// Every binding of a form, with the order its calculations must run in.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub bindings: Vec<Binding>,
    /// Indices into `bindings`, calculations first and in dependency order.
    calculation_order: Vec<usize>,
}

/// What a rule found wrong, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// The binding's path, as the form wrote it.
    pub path: String,
    /// The node's actual path, which differs inside a repeat.
    pub node_path: String,
    pub kind: ViolationKind,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViolationKind {
    /// The answer is present and the constraint rejects it.
    Constraint,
    /// The question is relevant, required, and unanswered.
    Required,
    /// The stored value is not what the form's own calculation produces.
    ///
    /// Recomputing on the server catches a device that was out of date, a
    /// form republished after collection began, or a submission that did
    /// not come from a form at all.
    Calculation { stored: String, computed: String },
    /// The expression could not be evaluated. Reported, never assumed
    /// away: a rule that did not run has not passed.
    Failed(String),
}

impl Violation {
    pub fn describe(&self) -> String {
        let where_ = &self.node_path;
        match &self.kind {
            ViolationKind::Constraint => match &self.message {
                Some(m) => format!("{where_}: {m}"),
                None => format!("{where_}: the answer does not satisfy the form's constraint"),
            },
            ViolationKind::Required => match &self.message {
                Some(m) => format!("{where_}: {m}"),
                None => format!("{where_}: required, and unanswered"),
            },
            ViolationKind::Calculation { stored, computed } => {
                format!("{where_}: holds {stored:?}, but the form calculates {computed:?}")
            }
            ViolationKind::Failed(why) => format!("{where_}: rule could not be evaluated — {why}"),
        }
    }
}

impl Rules {
    /// Order the calculations so each runs after what it reads.
    ///
    /// A cycle is an error rather than an iteration limit: a form whose
    /// calculations feed each other has no defined answer, and picking one
    /// by stopping after N rounds would make the result depend on N.
    pub fn new(bindings: Vec<Binding>) -> Result<Self, String> {
        let mut by_path: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, binding) in bindings.iter().enumerate() {
            by_path.insert(binding.path.as_str(), i);
        }

        // edges: a calculation depends on the bindings it reads
        let mut dependencies: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); bindings.len()];
        for (i, binding) in bindings.iter().enumerate() {
            let Some(calculate) = &binding.calculate else {
                continue;
            };
            for path in referenced_paths(calculate, &binding.path) {
                if let Some(j) = by_path.get(path.as_str()) {
                    if *j != i {
                        dependencies[i].insert(*j);
                    }
                }
            }
        }

        let calculations: Vec<usize> = (0..bindings.len())
            .filter(|i| bindings[*i].calculate.is_some())
            .collect();

        // depth-first topological sort, reporting the cycle it walked into
        let mut order = Vec::new();
        let mut state = vec![0u8; bindings.len()]; // 0 unvisited, 1 on stack, 2 done
        fn visit(
            i: usize,
            dependencies: &[BTreeSet<usize>],
            bindings: &[Binding],
            state: &mut [u8],
            order: &mut Vec<usize>,
            trail: &mut Vec<usize>,
        ) -> Result<(), String> {
            match state[i] {
                2 => return Ok(()),
                1 => {
                    let start = trail.iter().position(|t| *t == i).unwrap_or(0);
                    let cycle: Vec<&str> = trail[start..]
                        .iter()
                        .map(|t| bindings[*t].path.as_str())
                        .collect();
                    return Err(format!(
                        "these calculations depend on each other, so none of them \
                         has a defined value: {} → {}",
                        cycle.join(" → "),
                        bindings[i].path
                    ));
                }
                _ => {}
            }
            state[i] = 1;
            trail.push(i);
            for j in &dependencies[i] {
                if bindings[*j].calculate.is_some() {
                    visit(*j, dependencies, bindings, state, order, trail)?;
                }
            }
            trail.pop();
            state[i] = 2;
            order.push(i);
            Ok(())
        }
        for i in calculations {
            visit(
                i,
                &dependencies,
                &bindings,
                &mut state,
                &mut order,
                &mut Vec::new(),
            )?;
        }

        Ok(Rules {
            bindings,
            calculation_order: order,
        })
    }

    /// Check a filled instance against every rule.
    ///
    /// The order is the one a form engine uses, and it is not arbitrary:
    /// relevance first, because an irrelevant question has no answer to
    /// constrain and cannot be required; then calculations, so a constraint
    /// that reads a derived value sees the derived value.
    pub fn check(&self, instance: &Instance, env: &dyn Environment) -> Vec<Violation> {
        let mut violations = Vec::new();
        let relevance = self.relevance(instance, env, &mut violations);

        for &i in &self.calculation_order {
            let binding = &self.bindings[i];
            let Some(calculate) = &binding.calculate else {
                continue;
            };
            for node in self.nodes_for(&binding.path, instance, env) {
                if relevance.get(&node) == Some(&false) {
                    continue;
                }
                match evaluate(calculate, instance, Context::at(node), env) {
                    Ok(value) => {
                        let computed = value.to_string_value(instance);
                        let stored = instance.string_value(node);
                        if stored != computed {
                            violations.push(Violation {
                                path: binding.path.clone(),
                                node_path: instance.path_of(node),
                                kind: ViolationKind::Calculation { stored, computed },
                                message: None,
                            });
                        }
                    }
                    Err(why) => violations.push(Violation {
                        path: binding.path.clone(),
                        node_path: instance.path_of(node),
                        kind: ViolationKind::Failed(why),
                        message: None,
                    }),
                }
            }
        }

        for binding in &self.bindings {
            for node in self.nodes_for(&binding.path, instance, env) {
                // An irrelevant question is not asked, so it is neither
                // required nor constrained — and ODK clears its value, so
                // enforcing either would fail every well-formed submission
                // that skipped a branch.
                if relevance.get(&node) == Some(&false) {
                    continue;
                }
                let answered = !instance.string_value(node).trim().is_empty();

                if let Some(required) = &binding.required {
                    match evaluate(required, instance, Context::at(node), env) {
                        Ok(value) if value.to_boolean(instance) && !answered => {
                            violations.push(Violation {
                                path: binding.path.clone(),
                                node_path: instance.path_of(node),
                                kind: ViolationKind::Required,
                                message: binding.required_message.clone(),
                            });
                        }
                        Err(why) => violations.push(Violation {
                            path: binding.path.clone(),
                            node_path: instance.path_of(node),
                            kind: ViolationKind::Failed(why),
                            message: None,
                        }),
                        _ => {}
                    }
                }

                // A constraint judges an answer. With no answer there is
                // nothing to judge — that is what `required` is for.
                if let (Some(constraint), true) = (&binding.constraint, answered) {
                    match evaluate(constraint, instance, Context::at(node), env) {
                        Ok(value) if !value.to_boolean(instance) => violations.push(Violation {
                            path: binding.path.clone(),
                            node_path: instance.path_of(node),
                            kind: ViolationKind::Constraint,
                            message: binding.constraint_message.clone(),
                        }),
                        Err(why) => violations.push(Violation {
                            path: binding.path.clone(),
                            node_path: instance.path_of(node),
                            kind: ViolationKind::Failed(why),
                            message: None,
                        }),
                        _ => {}
                    }
                }
            }
        }

        violations
    }

    /// Relevance of every node a binding names, with the cascade applied.
    ///
    /// A question inside an irrelevant group is irrelevant however its own
    /// expression evaluates. Skipping the cascade is the classic way to
    /// enforce a constraint on an answer the enumerator was never shown.
    pub fn relevance(
        &self,
        instance: &Instance,
        env: &dyn Environment,
        violations: &mut Vec<Violation>,
    ) -> BTreeMap<NodeId, bool> {
        let mut own: BTreeMap<NodeId, bool> = BTreeMap::new();
        for binding in &self.bindings {
            let Some(relevant) = &binding.relevant else {
                continue;
            };
            for node in self.nodes_for(&binding.path, instance, env) {
                match evaluate(relevant, instance, Context::at(node), env) {
                    Ok(value) => {
                        own.insert(node, value.to_boolean(instance));
                    }
                    Err(why) => {
                        // A relevance that cannot be evaluated is reported,
                        // and the question treated as asked. Hiding it
                        // instead would silently drop the answer.
                        own.insert(node, true);
                        violations.push(Violation {
                            path: binding.path.clone(),
                            node_path: instance.path_of(node),
                            kind: ViolationKind::Failed(why),
                            message: None,
                        });
                    }
                }
            }
        }

        let mut effective: BTreeMap<NodeId, bool> = BTreeMap::new();
        for binding in &self.bindings {
            for node in self.nodes_for(&binding.path, instance, env) {
                let mut relevant = *own.get(&node).unwrap_or(&true);
                if relevant {
                    for ancestor in instance.ancestors(node) {
                        if own.get(&ancestor) == Some(&false) {
                            relevant = false;
                            break;
                        }
                    }
                }
                effective.insert(node, relevant);
            }
        }
        effective
    }

    /// Every node a binding path names — one per repeat instance.
    fn nodes_for(&self, path: &str, instance: &Instance, env: &dyn Environment) -> Vec<NodeId> {
        let Ok(expr) = crate::parser::parse(path) else {
            return Vec::new();
        };
        let Some(root) = instance.root() else {
            return Vec::new();
        };
        match evaluate(&expr, instance, Context::at(root), env) {
            Ok(Value::NodeSet(nodes)) => nodes,
            _ => Vec::new(),
        }
    }
}

/// The absolute paths an expression reads.
///
/// Relative paths are resolved against the binding they appear in, so
/// `../age` inside `/data/household/resident/name` means
/// `/data/household/resident/age` — which is how a dependency inside a
/// repeat is found at all.
pub fn referenced_paths(expr: &Expr, from: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect(expr, from, &mut out);
    out
}

fn collect(expr: &Expr, from: &str, out: &mut Vec<String>) {
    match expr {
        Expr::Path { absolute, steps } => {
            if let Some(path) = path_text(*absolute, steps, from) {
                out.push(path);
            }
            for step in steps {
                for predicate in &step.predicates {
                    collect(predicate, from, out);
                }
            }
        }
        Expr::Filter {
            base,
            predicates,
            steps,
        } => {
            collect(base, from, out);
            for predicate in predicates {
                collect(predicate, from, out);
            }
            for step in steps {
                for predicate in &step.predicates {
                    collect(predicate, from, out);
                }
            }
        }
        Expr::Function { args, .. } => {
            for arg in args {
                collect(arg, from, out);
            }
        }
        Expr::Binary { left, right, .. } | Expr::Union(left, right) => {
            collect(left, from, out);
            collect(right, from, out);
        }
        Expr::Negate(inner) => collect(inner, from, out),
        Expr::Number(_) | Expr::Literal(_) | Expr::Variable(_) => {}
    }
}

fn path_text(absolute: bool, steps: &[Step], from: &str) -> Option<String> {
    use crate::parser::{Axis, NameTest};
    // A relative path starts at the context node — the bound node itself,
    // not its parent. So `../age` on /data/resident/adult climbs from
    // adult to resident and lands on /data/resident/age. Starting a level
    // up instead lands on /data/age, which in a repeat is the difference
    // between this instance and none.
    let mut parts: Vec<String> = if absolute {
        Vec::new()
    } else {
        from.split('/').skip(1).map(String::from).collect()
    };
    for step in steps {
        match (&step.axis, &step.test) {
            (Axis::Child, NameTest::Named(name)) => parts.push(name.clone()),
            (Axis::Parent, _) => {
                parts.pop()?;
            }
            (Axis::Self_, _) => {}
            // A wildcard or an axis that fans out does not name one path,
            // and guessing at one would invent a dependency.
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

// ---------------------------------------------------------------------------
// From an XForm
// ---------------------------------------------------------------------------

/// Read the rules out of an XForm's `<bind>` elements.
///
/// The XForm rather than the spreadsheet, deliberately. Its binds already
/// carry absolute XPath — `${age}` has become `/data/age` — so nothing here
/// re-implements that expansion and drifts from the tool that performed it.
/// More to the point, the XForm is the artifact the device was handed: what
/// this checks is what the client evaluated, not a parallel reading of the
/// author's intent.
pub fn from_xform(xml: &str) -> Result<Rules, String> {
    let document = Instance::from_xml(xml).map_err(|e| format!("XForm XML: {e}"))?;
    let root = document
        .root()
        .ok_or_else(|| "the XForm has no root element".to_string())?;

    let mut bindings = Vec::new();
    let mut nodes = vec![root];
    nodes.extend(document.descendants(root));
    for node in nodes {
        if document.node(node).name != "bind" {
            continue;
        }
        let attribute = |name: &str| -> Option<String> {
            document
                .attributes(node)
                .into_iter()
                .find(|a| document.node(*a).name == name)
                .map(|a| document.node(a).value.clone())
                .filter(|v| !v.trim().is_empty())
        };
        let Some(path) = attribute("nodeset").or_else(|| attribute("ref")) else {
            return Err("a <bind> has no nodeset".to_string());
        };
        let parse_maybe = |source: Option<String>, what: &str| -> Result<Option<Expr>, String> {
            match source {
                None => Ok(None),
                Some(text) => crate::parser::parse(&text)
                    .map(Some)
                    .map_err(|e| format!("{what} of {path}: {e} — in expression {text:?}")),
            }
        };

        let mut binding = Binding::new(&path);
        binding.relevant = parse_maybe(attribute("relevant"), "relevant")?;
        binding.calculate = parse_maybe(attribute("calculate"), "calculate")?;
        binding.constraint = parse_maybe(attribute("constraint"), "constraint")?;
        binding.readonly = parse_maybe(attribute("readonly"), "readonly")?;
        // `required` is an expression, not a flag: forms write
        // `${consent} = 'yes'` as often as `true()`.
        binding.required = parse_maybe(attribute("required"), "required")?;
        binding.constraint_message = attribute("constraintMsg");
        binding.required_message = attribute("requiredMsg");
        bindings.push(binding);
    }

    if bindings.is_empty() {
        return Err("the XForm declares no binds, so it carries no rules".into());
    }
    Rules::new(bindings)
}

// ---------------------------------------------------------------------------
// A form, ready to check submissions against
// ---------------------------------------------------------------------------

/// The element this crate wraps a submission in while checking it. Named
/// after the XForms element it stands for, and stripped from reported paths.
const MODEL_ROOT: &str = "model";
const MODEL_ROOT_PATH: &str = "/model";

/// An XForm's rules together with the lookup tables it reads.
///
/// Cascading selects are written as `instance('lotes')/root/item[...]`, and
/// those tables live inside the XForm itself. Without them every such
/// expression fails, which on a real questionnaire means most of them.
pub struct Form {
    pub rules: Rules,
    secondary: BTreeMap<String, Instance>,
}

impl Form {
    pub fn parse(xform: &str) -> Result<Self, String> {
        Ok(Form {
            rules: from_xform(xform)?,
            secondary: secondary_instances(xform)?,
        })
    }

    /// Check a submission. `clock` supplies `today()` and `now()`.
    pub fn check(&self, submission: &Instance, clock: Clock) -> Vec<Violation> {
        let model = self.model_of(submission);
        let env = FormEnvironment {
            secondary: &self.secondary,
            clock,
        };
        self.rules
            .check(&model, &env)
            .into_iter()
            .map(|mut violation| {
                // Paths are reported as the form writes them, without the
                // wrapper this check builds around the submission.
                violation.node_path = violation
                    .node_path
                    .strip_prefix(MODEL_ROOT_PATH)
                    .unwrap_or(&violation.node_path)
                    .to_string();
                violation
            })
            .collect()
    }

    /// The submission and the form's lookup tables in one document, which is
    /// how an XForms model is actually shaped — and what a predicate needs
    /// in order to filter a table by an answer.
    fn model_of(&self, submission: &Instance) -> Instance {
        let mut model = Instance::new();
        let root = model.create_element(MODEL_ROOT, "");
        model.set_root(root);
        if let Some(primary) = submission.root() {
            let copied = model.adopt(submission, primary);
            model.append_child(root, copied);
        }
        for (id, table) in &self.secondary {
            let Some(table_root) = table.root() else {
                continue;
            };
            let _ = id;
            let copied = model.adopt(table, table_root);
            model.append_child(root, copied);
        }
        model.reindex();
        model
    }

    pub fn secondary_instance_ids(&self) -> Vec<&str> {
        self.secondary.keys().map(String::as_str).collect()
    }
}

/// What `today()` and `now()` answer.
///
/// Passed in rather than read from the system clock: a constraint like
/// `. <= today()` has to be judged against the day the interview happened,
/// not the day someone re-ran the check. Otherwise a submission that was
/// valid on collection starts failing later, and the report changes without
/// the data changing.
#[derive(Debug, Clone)]
pub struct Clock {
    pub today: String,
    pub now: String,
}

struct FormEnvironment<'a> {
    secondary: &'a BTreeMap<String, Instance>,
    clock: Clock,
}

impl Environment for FormEnvironment<'_> {
    fn secondary_instance(&self, id: &str) -> Option<&Instance> {
        self.secondary.get(id)
    }
    fn today(&self) -> String {
        self.clock.today.clone()
    }
    fn now(&self) -> String {
        self.clock.now.clone()
    }
}

/// The lookup tables declared inside an XForm.
///
/// A secondary instance is an `<instance>` carrying an `id`; the primary one
/// has no id of its own — its child element does. Reading them out means
/// `instance('lotes')` resolves without anyone loading anything.
pub fn secondary_instances(xform: &str) -> Result<BTreeMap<String, Instance>, String> {
    let document = Instance::from_xml(xform).map_err(|e| format!("XForm XML: {e}"))?;
    let Some(root) = document.root() else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    let mut nodes = vec![root];
    nodes.extend(document.descendants(root));
    for node in nodes {
        if document.node(node).name != "instance" {
            continue;
        }
        let Some(id) = document
            .attributes(node)
            .into_iter()
            .find(|a| document.node(*a).name == "id")
            .map(|a| document.node(a).value.clone())
        else {
            continue;
        };
        // The wrapper comes along: `instance('x')/root/item` steps from the
        // <instance> element into the <root> it contains.
        out.insert(id, subtree(&document, node));
    }
    Ok(out)
}

fn subtree(source: &Instance, from: NodeId) -> Instance {
    fn copy(source: &Instance, node: NodeId, target: &mut Instance) -> NodeId {
        let created = target.create_element(&source.node(node).name, &source.node(node).value);
        for attribute in source.attributes(node) {
            let attr = source.node(attribute);
            target.set_attribute(created, &attr.name, &attr.value);
        }
        for child in source.children(node) {
            let copied = copy(source, child, target);
            target.append_child(created, copied);
        }
        created
    }
    let mut target = Instance::new();
    let root = copy(source, from, &mut target);
    target.set_root(root);
    target
}
