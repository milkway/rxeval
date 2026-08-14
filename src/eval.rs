//! Evaluating a parsed expression against an instance.

use crate::parser::{Axis, BinaryOp, Expr, NameTest, Step};
use crate::tree::{Instance, NodeId};

/// The four XPath types.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Boolean(bool),
    Number(f64),
    String(String),
    /// Always in document order, without duplicates.
    NodeSet(Vec<NodeId>),
}

impl Value {
    /// XPath `string()`. A node-set becomes the string-value of its *first*
    /// node in document order — not a join, which is the intuition that
    /// quietly turns a repeat into one long answer.
    pub fn to_string_value(&self, instance: &Instance) -> String {
        match self {
            Value::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
            Value::Number(n) => format_number(*n),
            Value::String(s) => s.clone(),
            Value::NodeSet(nodes) => nodes
                .first()
                .map(|n| instance.string_value(*n))
                .unwrap_or_default(),
        }
    }

    /// XPath `boolean()`. An empty node-set is false however many empty
    /// strings it would have produced.
    pub fn to_boolean(&self, _instance: &Instance) -> bool {
        match self {
            Value::Boolean(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            Value::NodeSet(nodes) => !nodes.is_empty(),
        }
    }

    /// XPath `number()`. Anything unparseable is NaN, and NaN compares
    /// false against everything — including itself.
    pub fn to_number(&self, instance: &Instance) -> f64 {
        match self {
            Value::Boolean(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Number(n) => *n,
            Value::String(s) => string_to_number(s),
            Value::NodeSet(_) => string_to_number(&self.to_string_value(instance)),
        }
    }
}

pub fn string_to_number(text: &str) -> f64 {
    text.trim().parse::<f64>().unwrap_or(f64::NAN)
}

/// XPath's number formatting, which is not Rust's: whole numbers carry no
/// decimal point, and there is no exponent notation in the range forms use.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n == n.trunc() && n.abs() < 1e21 {
        return format!("{}", n as i64);
    }
    let text = format!("{n}");
    text
}

/// Where evaluation happens: which node is `.`, and where it sits among its
/// siblings for `position()` and `last()`.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub node: NodeId,
    pub position: usize,
    pub size: usize,
}

impl Context {
    pub fn at(node: NodeId) -> Self {
        Context {
            node,
            position: 1,
            size: 1,
        }
    }
}

/// Anything the evaluator needs that the instance alone cannot answer:
/// secondary instances for `instance()`, and the values `today()` and
/// `now()` should return.
///
/// Time is injected rather than read from the clock so that evaluating the
/// same form twice gives the same answer — a test that drifts at midnight
/// is worse than no test.
pub trait Environment {
    fn secondary_instance(&self, _id: &str) -> Option<&Instance> {
        None
    }
    /// The label a choice list gives to `value` for the question at
    /// `question_path`. Answered by whoever holds the form; a bare
    /// evaluator has no choice lists and says so.
    fn choice_label(&self, _value: &str, _question_path: &str) -> Option<String> {
        None
    }
    /// ISO date for `today()`.
    fn today(&self) -> String;
    /// ISO datetime for `now()`.
    fn now(&self) -> String;
}

/// An environment with no secondary instances and a fixed clock.
pub struct Fixed {
    pub today: String,
    pub now: String,
}

impl Environment for Fixed {
    fn today(&self) -> String {
        self.today.clone()
    }
    fn now(&self) -> String {
        self.now.clone()
    }
}

pub type EvalResult = Result<Value, String>;

pub fn evaluate(
    expr: &Expr,
    instance: &Instance,
    context: Context,
    env: &dyn Environment,
) -> EvalResult {
    match expr {
        Expr::Number(n) => Ok(Value::Number(*n)),
        Expr::Literal(s) => Ok(Value::String(s.clone())),
        Expr::Variable(name) => Err(format!(
            "variable ${name}: XForms has no variables, so this expression \
             cannot be what the form meant"
        )),
        Expr::Negate(inner) => {
            let value = evaluate(inner, instance, context, env)?;
            Ok(Value::Number(-value.to_number(instance)))
        }
        Expr::Union(left, right) => {
            let a = node_set(evaluate(left, instance, context, env)?, "union")?;
            let b = node_set(evaluate(right, instance, context, env)?, "union")?;
            let mut all = a;
            all.extend(b);
            Ok(Value::NodeSet(sorted_unique(all, instance)))
        }
        Expr::Binary { op, left, right } => binary(*op, left, right, instance, context, env),
        Expr::Path { absolute, steps } => {
            let start = if *absolute {
                match instance.root() {
                    Some(root) => root,
                    None => return Ok(Value::NodeSet(Vec::new())),
                }
            } else {
                context.node
            };
            // An absolute path names the root element itself: /data/age
            // starts at data and steps to age.
            let mut current = vec![start];
            if *absolute {
                if let Some(first) = steps.first() {
                    if let NameTest::Named(name) = &first.test {
                        if instance.node(start).name == *name && first.axis == Axis::Child {
                            let rest = &steps[1..];
                            let mut nodes =
                                apply_predicates(vec![start], &first.predicates, instance, env)?;
                            for step in rest {
                                nodes = walk(&nodes, step, instance, env)?;
                            }
                            return Ok(Value::NodeSet(nodes));
                        }
                    }
                }
            }
            for step in steps {
                current = walk(&current, step, instance, env)?;
            }
            Ok(Value::NodeSet(current))
        }
        Expr::Filter {
            base,
            predicates,
            steps,
        } => {
            // `instance('x')/...` steps into another document, so the walk
            // has to continue there rather than in the primary instance.
            //
            // Checked before evaluating the base: instance() is not a
            // function that returns a value, it names a document, and
            // handing it to the function table would fail on the one form
            // shape that matters most — every cascading select is written
            // this way.
            let (target, mut nodes) = match &**base {
                Expr::Function { name, args } if name == "instance" => {
                    let id = match args.first() {
                        Some(Expr::Literal(s)) => s.clone(),
                        Some(other) => {
                            evaluate(other, instance, context, env)?.to_string_value(instance)
                        }
                        None => return Err("instance() needs an id".into()),
                    };
                    // In the same document first. A form's lookup tables sit
                    // beside its primary instance inside one model, and that
                    // is what lets a predicate like
                    // `item[name = /data/linha]` reach back out to the
                    // answer it filters on. Resolving into a separate
                    // document strands that path — and mixes node ids from
                    // two arenas, which nothing here would catch.
                    match instance.instance_named(&id) {
                        Some(root) => (instance, vec![root]),
                        None => {
                            let secondary = env.secondary_instance(&id).ok_or_else(|| {
                                format!(
                                    "instance('{id}') is not loaded — the form expects a \
                                     secondary instance the evaluator was not given"
                                )
                            })?;
                            let root = match secondary.root() {
                                Some(root) => root,
                                None => return Ok(Value::NodeSet(Vec::new())),
                            };
                            (secondary, vec![root])
                        }
                    }
                }
                _ => {
                    let value = evaluate(base, instance, context, env)?;
                    (instance, node_set(value, "a path base")?)
                }
            };
            nodes = apply_predicates(nodes, predicates, target, env)?;
            for step in steps {
                nodes = walk(&nodes, step, target, env)?;
            }
            Ok(Value::NodeSet(nodes))
        }
        Expr::Function { name, args } => crate::functions::call(name, args, instance, context, env),
    }
}

fn node_set(value: Value, what: &str) -> Result<Vec<NodeId>, String> {
    match value {
        Value::NodeSet(nodes) => Ok(nodes),
        other => Err(format!(
            "{what} needs a node-set, got {}",
            type_name(&other)
        )),
    }
}

pub fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Boolean(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::NodeSet(_) => "a node-set",
    }
}

fn sorted_unique(mut nodes: Vec<NodeId>, instance: &Instance) -> Vec<NodeId> {
    nodes.sort_by_key(|n| instance.document_order(*n));
    nodes.dedup();
    nodes
}

fn walk(
    from: &[NodeId],
    step: &Step,
    instance: &Instance,
    env: &dyn Environment,
) -> Result<Vec<NodeId>, String> {
    let mut out = Vec::new();
    for node in from {
        let candidates: Vec<NodeId> = match step.axis {
            Axis::Child => instance.children(*node),
            Axis::Parent => instance.parent(*node).into_iter().collect(),
            Axis::Self_ => vec![*node],
            Axis::Attribute => instance.attributes(*node),
            Axis::Descendant => instance.descendants(*node),
            Axis::DescendantOrSelf => {
                let mut all = vec![*node];
                all.extend(instance.descendants(*node));
                all
            }
            Axis::Ancestor => instance.ancestors(*node),
            Axis::AncestorOrSelf => {
                let mut all = vec![*node];
                all.extend(instance.ancestors(*node));
                all
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => {
                let Some(parent) = instance.parent(*node) else {
                    continue;
                };
                let siblings = instance.children(parent);
                let position = siblings.iter().position(|s| s == node).unwrap_or(0);
                if step.axis == Axis::FollowingSibling {
                    siblings[position + 1..].to_vec()
                } else {
                    siblings[..position].to_vec()
                }
            }
            Axis::Following | Axis::Preceding => {
                let mine = instance.document_order(*node);
                let root = instance.root().unwrap_or(*node);
                let mut all = vec![root];
                all.extend(instance.descendants(root));
                all.retain(|n| {
                    let theirs = instance.document_order(*n);
                    if step.axis == Axis::Following {
                        theirs > mine && !instance.descendants(*node).contains(n)
                    } else {
                        theirs < mine && !instance.ancestors(*node).contains(n)
                    }
                });
                all
            }
        };
        let matched: Vec<NodeId> = candidates
            .into_iter()
            .filter(|c| match &step.test {
                NameTest::Any => true,
                NameTest::Named(name) => instance.node(*c).name == *name,
            })
            .collect();
        out.extend(apply_predicates(matched, &step.predicates, instance, env)?);
    }
    Ok(sorted_unique(out, instance))
}

/// Predicates filter a node-set, and each node's `position()` is its place
/// in *that* set — not in the document. A numeric predicate is shorthand
/// for `position() = n`, which is why `[1]` selects the first sibling and
/// not the node numbered one.
fn apply_predicates(
    nodes: Vec<NodeId>,
    predicates: &[Expr],
    instance: &Instance,
    env: &dyn Environment,
) -> Result<Vec<NodeId>, String> {
    let mut current = nodes;
    for predicate in predicates {
        let size = current.len();
        let mut kept = Vec::new();
        for (i, node) in current.iter().enumerate() {
            let context = Context {
                node: *node,
                position: i + 1,
                size,
            };
            let value = evaluate(predicate, instance, context, env)?;
            let keep = match value {
                Value::Number(n) => n == (i + 1) as f64,
                other => other.to_boolean(instance),
            };
            if keep {
                kept.push(*node);
            }
        }
        current = kept;
    }
    Ok(current)
}

fn binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    instance: &Instance,
    context: Context,
    env: &dyn Environment,
) -> EvalResult {
    // `and` and `or` short-circuit, which matters: a form guards a
    // comparison with a check that the value exists.
    match op {
        BinaryOp::And => {
            let a = evaluate(left, instance, context, env)?;
            if !a.to_boolean(instance) {
                return Ok(Value::Boolean(false));
            }
            let b = evaluate(right, instance, context, env)?;
            return Ok(Value::Boolean(b.to_boolean(instance)));
        }
        BinaryOp::Or => {
            let a = evaluate(left, instance, context, env)?;
            if a.to_boolean(instance) {
                return Ok(Value::Boolean(true));
            }
            let b = evaluate(right, instance, context, env)?;
            return Ok(Value::Boolean(b.to_boolean(instance)));
        }
        _ => {}
    }

    let a = evaluate(left, instance, context, env)?;
    let b = evaluate(right, instance, context, env)?;

    match op {
        BinaryOp::Equal | BinaryOp::NotEqual => Ok(Value::Boolean(compare(op, &a, &b, instance))),
        BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
            let x = a.to_number(instance);
            let y = b.to_number(instance);
            let result = match op {
                BinaryOp::Less => x < y,
                BinaryOp::LessEqual => x <= y,
                BinaryOp::Greater => x > y,
                _ => x >= y,
            };
            Ok(Value::Boolean(result))
        }
        BinaryOp::Add => Ok(Value::Number(a.to_number(instance) + b.to_number(instance))),
        BinaryOp::Subtract => Ok(Value::Number(a.to_number(instance) - b.to_number(instance))),
        BinaryOp::Multiply => Ok(Value::Number(a.to_number(instance) * b.to_number(instance))),
        BinaryOp::Divide => Ok(Value::Number(a.to_number(instance) / b.to_number(instance))),
        BinaryOp::Modulo => Ok(Value::Number(a.to_number(instance) % b.to_number(instance))),
        BinaryOp::And | BinaryOp::Or => unreachable!("short-circuited above"),
    }
}

/// Comparison against a node-set is existential, and `!=` is **not** the
/// negation of `=`.
///
/// Both are "does some node satisfy this", so against ages 34, 7 and 19,
/// `age = 7` and `age != 7` are true at the same time. Implementing `!=` as
/// `not(=)` compiles, passes any test written with a single-node set, and
/// then silently inverts the meaning of every multi-node comparison a form
/// makes — which is most guards over a repeat.
fn compare(op: BinaryOp, a: &Value, b: &Value, instance: &Instance) -> bool {
    let holds = |x: &str, y: &str| match op {
        BinaryOp::Equal => x == y,
        _ => x != y,
    };
    let holds_num = |x: f64, y: f64| match op {
        BinaryOp::Equal => x == y,
        _ => x != y,
    };
    match (a, b) {
        (Value::NodeSet(left), Value::NodeSet(right)) => left.iter().any(|l| {
            let lv = instance.string_value(*l);
            right.iter().any(|r| holds(&lv, &instance.string_value(*r)))
        }),
        (Value::NodeSet(nodes), other) | (other, Value::NodeSet(nodes)) => match other {
            Value::Number(n) => nodes
                .iter()
                .any(|node| holds_num(string_to_number(&instance.string_value(*node)), *n)),
            // Against a boolean the node-set converts as a whole, so this
            // one really is a single comparison.
            Value::Boolean(b) => {
                let set = Value::NodeSet(nodes.clone()).to_boolean(instance);
                match op {
                    BinaryOp::Equal => set == *b,
                    _ => set != *b,
                }
            }
            _ => {
                let text = other.to_string_value(instance);
                nodes
                    .iter()
                    .any(|n| holds(&instance.string_value(*n), &text))
            }
        },
        (Value::Boolean(_), _) | (_, Value::Boolean(_)) => {
            let (x, y) = (a.to_boolean(instance), b.to_boolean(instance));
            match op {
                BinaryOp::Equal => x == y,
                _ => x != y,
            }
        }
        (Value::Number(_), _) | (_, Value::Number(_)) => {
            holds_num(a.to_number(instance), b.to_number(instance))
        }
        _ => holds(&a.to_string_value(instance), &b.to_string_value(instance)),
    }
}
