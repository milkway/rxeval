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
    /// The XForm body and its translations, kept for `jr:choice-name()`:
    /// answering it means finding the question's choice list and then the
    /// label of one item.
    body: Option<Instance>,
    itext: BTreeMap<String, String>,
}

impl Form {
    pub fn parse(xform: &str) -> Result<Self, String> {
        let document = Instance::from_xml(xform).map_err(|e| format!("XForm XML: {e}"))?;
        Ok(Form {
            rules: from_xform(xform)?,
            secondary: secondary_instances(xform)?,
            itext: translations(&document),
            body: Some(document),
        })
    }

    /// Check a submission. `clock` supplies `today()` and `now()`.
    pub fn check(&self, submission: &Instance, clock: Clock) -> Vec<Violation> {
        let model = self.model_of(submission);
        let env = FormEnvironment {
            secondary: &self.secondary,
            form: self,
            model: &model,
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

    /// Run the form's calculations and report what they produce.
    ///
    /// Nothing is written to `submission`: the values come back by path and
    /// the caller decides. Calculations that feed each other still see each
    /// other, because the intermediate writes happen in this function's own
    /// copy of the model — in the dependency order [`Rules`] worked out
    /// when the form was parsed.
    ///
    /// The second half of the pair is expressions that could not be
    /// evaluated. Those are form bugs rather than answer problems, which is
    /// why they travel separately: nobody filling in a form can fix one.
    pub fn calculations(
        &self,
        submission: &Instance,
        clock: &Clock,
    ) -> (BTreeMap<String, String>, Vec<(String, String)>) {
        let mut model = self.model_of(submission);
        let mut computed = BTreeMap::new();
        let mut failed = Vec::new();

        for index in self.rules.calculation_order.clone() {
            let binding = &self.rules.bindings[index];
            let Some(calculate) = binding.calculate.clone() else {
                continue;
            };
            let path = binding.path.clone();
            let nodes = {
                let env = FormEnvironment {
                    secondary: &self.secondary,
                    form: self,
                    model: &model,
                    clock: clock.clone(),
                };
                self.rules.nodes_for(&path, &model, &env)
            };
            for node in nodes {
                let result = {
                    let env = FormEnvironment {
                        secondary: &self.secondary,
                        form: self,
                        model: &model,
                        clock: clock.clone(),
                    };
                    evaluate(&calculate, &model, Context::at(node), &env)
                        .map(|value| value.to_string_value(&model))
                };
                match result {
                    Ok(text) => {
                        model.node_mut(node).value = text.clone();
                        let where_ = model.path_of(node);
                        let where_ = where_
                            .strip_prefix(MODEL_ROOT_PATH)
                            .unwrap_or(&where_)
                            .to_string();
                        computed.insert(where_, text);
                    }
                    Err(why) => failed.push((path.clone(), why)),
                }
            }
        }
        (computed, failed)
    }

    /// Which nodes the form asks about, by path.
    ///
    /// A path missing from the map has no relevance expression of its own
    /// and no irrelevant ancestor: it is always asked.
    pub fn relevance(
        &self,
        submission: &Instance,
        clock: &Clock,
    ) -> (BTreeMap<String, bool>, Vec<Violation>) {
        let model = self.model_of(submission);
        let env = FormEnvironment {
            secondary: &self.secondary,
            form: self,
            model: &model,
            clock: clock.clone(),
        };
        let mut problems = Vec::new();
        let by_node = self.rules.relevance(&model, &env, &mut problems);
        let mut by_path = BTreeMap::new();
        for (node, shown) in by_node {
            let path = model.path_of(node);
            let path = path.strip_prefix(MODEL_ROOT_PATH).unwrap_or(&path);
            by_path.insert(path.to_string(), shown);
        }
        for problem in &mut problems {
            problem.node_path = problem
                .node_path
                .strip_prefix(MODEL_ROOT_PATH)
                .unwrap_or(&problem.node_path)
                .to_string();
        }
        (by_path, problems)
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

    /// The label a question's choice list gives to one value.
    ///
    /// Two shapes exist and both appear in real forms: choices written out
    /// in the body, and an `<itemset>` pointing at a lookup table. Either
    /// way the label may be a translation key rather than text, which is
    /// why the itext table is carried alongside.
    fn choice_label(&self, model: &Instance, value: &str, question_path: &str) -> Option<String> {
        let body = self.body.as_ref()?;
        let control = find_control(body, question_path)?;

        // choices spelled out in the body
        for item in body.children(control) {
            if body.node(item).name != "item" {
                continue;
            }
            let item_value = body
                .children(item)
                .into_iter()
                .find(|c| body.node(*c).name == "value")
                .map(|c| body.string_value(c))
                .unwrap_or_default();
            if item_value.trim() != value.trim() {
                continue;
            }
            let label = body
                .children(item)
                .into_iter()
                .find(|c| body.node(*c).name == "label")?;
            return Some(self.resolve_label(body, label, None, model));
        }

        // choices drawn from a lookup table
        let itemset = body
            .children(control)
            .into_iter()
            .find(|c| body.node(*c).name == "itemset")?;
        let nodeset = attribute_of(body, itemset, "nodeset")?;
        let value_ref = body
            .children(itemset)
            .into_iter()
            .find(|c| body.node(*c).name == "value")
            .and_then(|c| attribute_of(body, c, "ref"))
            .unwrap_or_else(|| "name".to_string());

        let expr = crate::parser::parse(&nodeset).ok()?;
        let root = model.root()?;
        let env = FormEnvironment {
            secondary: &self.secondary,
            form: self,
            model,
            clock: Clock {
                today: String::new(),
                now: String::new(),
            },
        };
        let items = match evaluate(&expr, model, Context::at(root), &env) {
            Ok(Value::NodeSet(nodes)) => nodes,
            _ => return None,
        };
        let label_ref = body
            .children(itemset)
            .into_iter()
            .find(|c| body.node(*c).name == "label")
            .and_then(|c| attribute_of(body, c, "ref"));

        for item in items {
            let holds = model
                .children(item)
                .into_iter()
                .find(|c| model.node(*c).name == value_ref)
                .map(|c| model.string_value(c))
                .unwrap_or_default();
            if holds.trim() != value.trim() {
                continue;
            }
            return Some(match &label_ref {
                Some(reference) => self.resolve_reference(reference, model, item),
                // No label declared: the value is the best name there is.
                None => holds,
            });
        }
        None
    }

    /// A `<label ref="…"/>` on an item written out in the body.
    fn resolve_label(
        &self,
        body: &Instance,
        label: NodeId,
        item: Option<NodeId>,
        model: &Instance,
    ) -> String {
        match attribute_of(body, label, "ref") {
            Some(reference) => self.resolve_reference(
                &reference,
                model,
                item.unwrap_or_else(|| model.root().unwrap_or(NodeId(0))),
            ),
            None => body.string_value(label),
        }
    }

    /// `jr:itext('id')` becomes the translated text; anything else is an
    /// expression evaluated against the item it labels.
    fn resolve_reference(&self, reference: &str, model: &Instance, item: NodeId) -> String {
        let reference = reference.trim();
        if let Some(rest) = reference.strip_prefix("jr:itext(") {
            let inner = rest.trim_end_matches(')').trim();
            // a literal id, or an expression naming one
            let id = match inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                Some(literal) => literal.to_string(),
                None => match crate::parser::parse(inner) {
                    Ok(expr) => {
                        let env = FormEnvironment {
                            secondary: &self.secondary,
                            form: self,
                            model,
                            clock: Clock {
                                today: String::new(),
                                now: String::new(),
                            },
                        };
                        evaluate(&expr, model, Context::at(item), &env)
                            .map(|v| v.to_string_value(model))
                            .unwrap_or_default()
                    }
                    Err(_) => String::new(),
                },
            };
            return self.itext.get(&id).cloned().unwrap_or(id);
        }
        match crate::parser::parse(reference) {
            Ok(expr) => {
                let env = FormEnvironment {
                    secondary: &self.secondary,
                    form: self,
                    model,
                    clock: Clock {
                        today: String::new(),
                        now: String::new(),
                    },
                };
                evaluate(&expr, model, Context::at(item), &env)
                    .map(|v| v.to_string_value(model))
                    .unwrap_or_default()
            }
            Err(_) => String::new(),
        }
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
    form: &'a Form,
    model: &'a Instance,
    clock: Clock,
}

impl Environment for FormEnvironment<'_> {
    fn secondary_instance(&self, id: &str) -> Option<&Instance> {
        self.secondary.get(id)
    }

    fn choice_label(&self, value: &str, question_path: &str) -> Option<String> {
        self.form.choice_label(self.model, value, question_path)
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

fn attribute_of(document: &Instance, node: NodeId, name: &str) -> Option<String> {
    document
        .attributes(node)
        .into_iter()
        .find(|a| document.node(*a).name == name)
        .map(|a| document.node(a).value.clone())
}

/// The body control bound to a path. Forms write the reference with stray
/// spaces often enough that comparing them trimmed is the only way to find
/// anything.
fn find_control(body: &Instance, path: &str) -> Option<NodeId> {
    let wanted = path.trim();
    let root = body.root()?;
    let mut nodes = vec![root];
    nodes.extend(body.descendants(root));
    nodes.into_iter().find(|node| {
        matches!(
            body.node(*node).name.as_str(),
            "select1" | "select" | "input" | "upload" | "range" | "odk:rank" | "rank"
        ) && attribute_of(body, *node, "ref").is_some_and(|r| r.trim() == wanted)
    })
}

/// The default translation, as id → text.
///
/// The first language wins: a violation report shows one string, and
/// picking the form's own default is closer to what its author reads than
/// any other choice this crate could make.
fn translations(document: &Instance) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(root) = document.root() else {
        return out;
    };
    let mut nodes = vec![root];
    nodes.extend(document.descendants(root));
    let Some(translation) = nodes
        .iter()
        .find(|node| document.node(**node).name == "translation")
    else {
        return out;
    };
    for text in document.children(*translation) {
        if document.node(text).name != "text" {
            continue;
        }
        let Some(id) = attribute_of(document, text, "id") else {
            continue;
        };
        // <value> holds the string, sometimes several for different forms
        // of the same label; the first is the plain one.
        if let Some(value) = document
            .children(text)
            .into_iter()
            .find(|c| document.node(*c).name == "value")
        {
            out.insert(id, document.string_value(value));
        }
    }
    out
}
