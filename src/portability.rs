//! Will this form mean the same thing on a tablet and in a browser?
//!
//! It often will not, and nothing in either tool says so. The two engines
//! the ecosystem runs on — JavaRosa inside ODK Collect and KoboCollect,
//! Enketo inside web forms — implement different languages. Neither is a
//! superset of the other, and the gaps do not announce themselves: an
//! expression Collect cannot evaluate usually yields nothing rather than an
//! error, so the form fills in, the interview finishes, and a column comes
//! back empty.
//!
//! Every rule here was found by putting the same expression to both engines
//! and reading the two answers — see `tests/ecosystem_oracle_test.rs`.

use std::collections::BTreeSet;

use crate::parser::{Axis, Expr, NameTest, Step};

/// Which engine stumbles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breaks {
    /// JavaRosa: the tablets. The expensive one — data is being collected
    /// while it goes wrong.
    Collect,
    /// Enketo: web forms.
    WebForms,
    /// Both run it, and mean different things by it. The worst kind,
    /// because nothing fails.
    Differently,
    /// Both run it and both do the same nothing. Not a portability problem
    /// at all — a form problem, which travels perfectly and is wrong
    /// everywhere it goes. Kept in the same report because it is found the
    /// same way and matters more.
    Everywhere,
}

impl Breaks {
    pub fn describe(&self) -> &'static str {
        match self {
            Breaks::Collect => "Collect / KoboCollect",
            Breaks::WebForms => "Enketo web forms",
            Breaks::Differently => "both, differently",
            Breaks::Everywhere => "both, identically and silently",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Issue {
    /// The bind this expression belongs to.
    pub path: String,
    /// Which rule: relevant, calculate, constraint, required.
    pub rule: String,
    pub expression: String,
    /// The construct at fault, as a form author would name it.
    pub construct: String,
    pub breaks: Breaks,
    pub effect: String,
    pub suggestion: Option<String>,
}

impl Issue {
    pub fn describe(&self) -> String {
        let where_ = format!("{} ({})", self.path, self.rule);
        let fix = match &self.suggestion {
            Some(s) => format!(" Write {s} instead."),
            None => String::new(),
        };
        // The engine clause reads differently for a fault that is not
        // about engines at all: "on both, identically and silently" tacked
        // onto the end of a sentence about an empty node-set is a riddle.
        let engines = match self.breaks {
            Breaks::Everywhere => "identically on both engines".to_string(),
            Breaks::Differently => "and the two engines differ".to_string(),
            other => format!("on {}", other.describe()),
        };
        format!(
            "{where_}: {} — {}, {engines}.{fix}",
            self.construct, self.effect
        )
    }
}

/// Read every expression in a form and report what will not travel.
pub fn check_form(xform: &str) -> Result<Vec<Issue>, String> {
    let document = crate::tree::Instance::from_xml(xform).map_err(|e| format!("XForm XML: {e}"))?;
    let root = document
        .root()
        .ok_or_else(|| "the XForm has no root element".to_string())?;

    // Paths that can hold more than one node: comparing one against a value
    // is what JavaRosa refuses.
    let mut repeats: Vec<String> = Vec::new();
    let mut nodes = vec![root];
    nodes.extend(document.descendants(root));
    for node in &nodes {
        if document.node(*node).name != "repeat" {
            continue;
        }
        if let Some(nodeset) = document
            .attributes(*node)
            .into_iter()
            .find(|a| document.node(*a).name == "nodeset")
            .map(|a| document.node(a).value.clone())
        {
            repeats.push(nodeset.trim().to_string());
        }
    }

    let shape = Shape::of(&document);

    let mut issues = Vec::new();
    for node in &nodes {
        if document.node(*node).name != "bind" {
            continue;
        }
        let attribute = |name: &str| {
            document
                .attributes(*node)
                .into_iter()
                .find(|a| document.node(*a).name == name)
                .map(|a| document.node(a).value.clone())
                .filter(|v| !v.trim().is_empty())
        };
        let Some(path) = attribute("nodeset").or_else(|| attribute("ref")) else {
            continue;
        };
        for rule in [
            "relevant",
            "calculate",
            "constraint",
            "required",
            "readonly",
        ] {
            let Some(expression) = attribute(rule) else {
                continue;
            };
            let Ok(parsed) = crate::parser::parse(&expression) else {
                // A form this crate cannot parse is a different problem, and
                // `from_xform` is where it gets reported.
                continue;
            };
            let mut found = Vec::new();
            inspect(&parsed, &repeats, &mut found);
            check_paths(&parsed, &path, &shape, &mut found);
            for (construct, breaks, effect, suggestion) in found {
                issues.push(Issue {
                    path: path.trim().to_string(),
                    rule: rule.to_string(),
                    expression: expression.trim().to_string(),
                    construct,
                    breaks,
                    effect,
                    suggestion,
                });
            }
        }
    }
    Ok(issues)
}

type Finding = (String, Breaks, String, Option<String>);

fn inspect(expr: &Expr, repeats: &[String], out: &mut Vec<Finding>) {
    match expr {
        Expr::Path { steps, .. } => {
            inspect_steps(steps, repeats, out);
        }
        Expr::Filter {
            base,
            predicates,
            steps,
        } => {
            inspect(base, repeats, out);
            for predicate in predicates {
                inspect_predicate(predicate, repeats, out);
            }
            inspect_steps(steps, repeats, out);
        }
        Expr::Function { name, args } => {
            check_function(name, args, out);
            for arg in args {
                inspect(arg, repeats, out);
            }
        }
        Expr::Binary { op, left, right } => {
            use crate::parser::BinaryOp::*;
            if matches!(
                op,
                Equal | NotEqual | Less | LessEqual | Greater | GreaterEqual
            ) {
                for side in [left, right] {
                    if let Some(path) = absolute_path_of(side) {
                        if crosses_repeat(&path, repeats) {
                            out.push((
                                format!("comparing {path}, which repeats"),
                                Breaks::Collect,
                                "JavaRosa refuses to compare a field that repeats against a \
                                 value, so the whole rule fails"
                                    .into(),
                                Some(format!(
                                    "count({path}[. = …]) > 0, or indexed-repeat() to name \
                                     one instance"
                                )),
                            ));
                        }
                    }
                }
            }
            inspect(left, repeats, out);
            inspect(right, repeats, out);
        }
        Expr::Union(left, right) => {
            inspect(left, repeats, out);
            inspect(right, repeats, out);
        }
        Expr::Negate(inner) => inspect(inner, repeats, out),
        Expr::Number(_) | Expr::Literal(_) | Expr::Variable(_) => {}
    }
}

fn inspect_steps(steps: &[Step], repeats: &[String], out: &mut Vec<Finding>) {
    for step in steps {
        if step.axis == Axis::DescendantOrSelf && step.test == NameTest::Any {
            out.push((
                "the // shorthand".into(),
                Breaks::Collect,
                "JavaRosa supports only child steps, so the path matches nothing".into(),
                Some("the full path from the instance root".into()),
            ));
        }
        for predicate in &step.predicates {
            // A bare number is XPath's shorthand for position(), and the one
            // JavaRosa does not implement: it matches nothing at all, so the
            // question reads as unanswered rather than as an error.
            if let Expr::Number(n) = predicate {
                out.push((
                    format!(
                        "the positional predicate [{}]",
                        crate::eval::format_number(*n)
                    ),
                    Breaks::Collect,
                    "JavaRosa matches nothing for a bare [n], so this silently reads as empty"
                        .into(),
                    Some(format!("[position() = {}]", crate::eval::format_number(*n))),
                ));
            }
            inspect_predicate(predicate, repeats, out);
        }
    }
}

fn inspect_predicate(predicate: &Expr, repeats: &[String], out: &mut Vec<Finding>) {
    inspect(predicate, repeats, out);
}

fn check_function(name: &str, args: &[Expr], out: &mut Vec<Finding>) {
    match name {
        // Present in Enketo and in XPath, absent from JavaRosa's table of
        // 77 functions — verified against the bytecode, not guessed.
        "last" => out.push((
            "last()".into(),
            Breaks::Collect,
            "JavaRosa has no last(), so the rule fails".into(),
            Some("count() of the repeat".into()),
        )),
        // pulldata is not an XForms function at all: ODK Collect registers
        // it as its own handler at runtime, and pyxform leaves the call in
        // the expression for it to find. Enketo's evaluator has no such
        // handler, so a form built on pulldata cannot be filled on the web
        // at all — which is a different and larger problem than a rule that
        // computes something else.
        "pulldata" => out.push((
            "pulldata()".into(),
            Breaks::WebForms,
            "pulldata() is ODK Collect's own function, not an XForms one. Enketo does not \
             have it, so every rule that calls it fails there"
                .into(),
            Some("instance('file')/root/item[key = …]/column, which both engines evaluate".into()),
        )),
        // Both engines have these and neither agrees with the other on what
        // comes back. The geometry is the same; how much of it is reported
        // is not, and area() is not even computed the same way.
        "distance" => out.push((
            "distance()".into(),
            Breaks::Differently,
            "Enketo rounds the answer to two decimals and JavaRosa does not, so a rule \
             comparing a distance against a threshold can fall either side of it"
                .into(),
            Some("round(distance(…)) or a comparison with room to spare".into()),
        )),
        "area" => out.push((
            "area()".into(),
            Breaks::Differently,
            "JavaRosa projects the shape onto a plane and Enketo uses a spherical formula, \
             and Enketo also rounds to two decimals. Over a city block the two agree within \
             a square metre; over a degree they differ by a tenth of a percent"
                .into(),
            Some("round() the result, or compare with a margin".into()),
        )),
        "floor" => out.push((
            "floor()".into(),
            Breaks::Collect,
            "JavaRosa has no floor()".into(),
            Some("int() for a positive number, or -int(-x) to round down".into()),
        )),
        "ceiling" => out.push((
            "ceiling()".into(),
            Breaks::Collect,
            "JavaRosa has no ceiling()".into(),
            Some("-int(-x), or int(x) + 1 when x has a fraction".into()),
        )),
        "substring" => out.push((
            "substring()".into(),
            Breaks::Collect,
            "JavaRosa has substr() but not substring()".into(),
            Some(
                "substr(), remembering it counts from 0 and its end is exclusive: \
                 substring(s, 2, 3) is substr(s, 1, 4)"
                    .into(),
            ),
        )),
        // The reverse direction: JavaRosa-only functions.
        "enclosed-area" | "geofence" | "extract-signed" | "base64-decode" | "is-selected" => out
            .push((
                format!("{name}()"),
                Breaks::WebForms,
                "Enketo does not implement it, so a web form cannot open this".into(),
                None,
            )),
        // Runs in both and means different things: the pattern is anchored
        // on the tablet and not in the browser. A form checking a CPF with
        // [0-9]{11} accepts "abc12345678901xyz" in a web form and rejects it
        // on a tablet.
        "regex" => {
            let Some(Expr::Literal(pattern)) = args.get(1) else {
                // A pattern assembled from an answer cannot be read here.
                // Saying so beats saying nothing: it is the one case where
                // this check has no opinion.
                out.push((
                    "regex() with a pattern that is not written out".into(),
                    Breaks::Differently,
                    "the pattern is built at runtime, so whether it anchors cannot be \
                     checked here — the two engines still differ on unanchored ones"
                        .into(),
                    None,
                ));
                return;
            };
            let bare = !pattern.starts_with('^') || !pattern.ends_with('$');
            let alternation = has_top_level_alternation(pattern);
            let fix = format!("'^(?:{pattern})$'");
            if bare {
                out.push((
                    format!("regex() with the unanchored pattern {pattern:?}"),
                    Breaks::Differently,
                    "JavaRosa requires the whole value to match while Enketo accepts a \
                     match anywhere in it, so the same answer passes in one and fails \
                     in the other"
                        .into(),
                    Some(fix),
                ));
            } else if alternation {
                // `^a|b$` reads as anchored and is not: the anchors bind to
                // the branches either side of the bar, so the middle
                // alternatives float free. JavaRosa anchors the lot anyway;
                // Enketo does not, and the two part ways again.
                out.push((
                    format!("regex() with alternation outside the anchors: {pattern:?}"),
                    Breaks::Differently,
                    "the ^ and $ bind only to the first and last alternatives, so this \
                     is anchored in JavaRosa and loose in Enketo despite looking anchored"
                        .into(),
                    Some(fix),
                ));
            }
        }
        // Casing that only one engine forgives.
        "boolean-from-string" => {
            if let Some(Expr::Literal(value)) = args.first() {
                if value.eq_ignore_ascii_case("true") && value != "true" {
                    out.push((
                        format!("boolean-from-string({value:?})"),
                        Breaks::Differently,
                        "JavaRosa accepts any casing, Enketo only lowercase 'true'".into(),
                        Some("'true' in lowercase".into()),
                    ));
                }
            }
        }
        _ => {}
    }
}

/// The absolute path an expression names, when it names exactly one.
fn absolute_path_of(expr: &Expr) -> Option<String> {
    let Expr::Path { absolute, steps } = expr else {
        return None;
    };
    if !absolute {
        return None;
    }
    let mut parts = Vec::new();
    for step in steps {
        match (&step.axis, &step.test) {
            (Axis::Child, NameTest::Named(name)) if step.predicates.is_empty() => {
                parts.push(name.clone())
            }
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| format!("/{}", parts.join("/")))
}

/// Does this path pass through a repeat, and so possibly name many nodes?
fn crosses_repeat(path: &str, repeats: &[String]) -> bool {
    repeats.iter().any(|repeat| {
        let repeat = repeat.trim_end_matches('/');
        path.starts_with(&format!("{repeat}/")) || path == repeat
    })
}

/// Is there a `|` at the top level of this pattern — outside every group and
/// character class?
///
/// It matters because alternation binds looser than anchoring: `^a|b$` is
/// "starts with a" or "ends with b", not "is exactly a or b". A pattern
/// written that way looks anchored to a reader and to a naive check.
fn has_top_level_alternation(pattern: &str) -> bool {
    let mut depth = 0usize;
    let mut in_class = false;
    let mut escaped = false;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => depth += 1,
            ')' if !in_class => depth = depth.saturating_sub(1),
            '|' if !in_class && depth == 0 => return true,
            _ => {}
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Paths that name nothing
// ---------------------------------------------------------------------------
//
// A form's instance is fixed when the form is written, so a path in a rule
// either names a node in it or names nothing at all — and naming nothing is
// silent. XPath answers an empty node-set, which `sum()` reads as 0,
// `count()` as 0, a comparison as false, and a `relevant` as "do not ask".
// So a calculation quietly produces zero, a question is hidden for the
// whole of fieldwork, and a constraint never rejects anything. Both engines
// agree perfectly, and both are useless.
//
// Nothing in the ecosystem reports this. pyxform and rxform check the
// spreadsheet's own references; an XPath written out by hand — the root
// name misspelled, a group renamed, a field deleted — goes straight
// through.

/// What the form's instance actually contains.
struct Shape {
    /// Every element path in the primary instance, without positions.
    paths: BTreeSet<String>,
    /// The ids of every `<instance>` the form declares.
    tables: BTreeSet<String>,
    /// The primary instance's root element name, for a clearer message when
    /// the root itself is what was misspelled.
    root: Option<String>,
}

impl Shape {
    fn of(document: &crate::tree::Instance) -> Shape {
        let mut paths = BTreeSet::new();
        let mut tables = BTreeSet::new();
        let mut root_name = None;

        let Some(root) = document.root() else {
            return Shape {
                paths,
                tables,
                root: None,
            };
        };
        let mut nodes = vec![root];
        nodes.extend(document.descendants(root));

        let mut primary_seen = false;
        for node in nodes {
            if document.node(node).name != "instance" {
                continue;
            }
            let id = document
                .attributes(node)
                .into_iter()
                .find(|a| document.node(*a).name == "id")
                .map(|a| document.node(a).value.clone());
            match id {
                Some(id) => {
                    tables.insert(id);
                }
                None if !primary_seen => {
                    // The first instance with no id is the primary one.
                    primary_seen = true;
                    if let Some(template) = document.children(node).into_iter().next() {
                        root_name = Some(document.node(template).name.clone());
                        walk(document, template, "", &mut paths);
                    }
                }
                None => {}
            }
        }
        Shape {
            paths,
            tables,
            root: root_name,
        }
    }

    /// Whether a path names anything. Attributes count: `/data/@id` is a
    /// node a form may legitimately read.
    fn holds(&self, path: &str) -> bool {
        let path = path.split('@').next().unwrap_or(path).trim_end_matches('/');
        self.paths.contains(path)
    }
}

fn walk(
    document: &crate::tree::Instance,
    node: crate::tree::NodeId,
    prefix: &str,
    into: &mut BTreeSet<String>,
) {
    let here = format!("{prefix}/{}", document.node(node).name);
    into.insert(here.clone());
    for child in document.children(node) {
        walk(document, child, &here, into);
    }
}

/// Report every absolute path in an expression that the instance has no
/// node for.
fn check_paths(expr: &Expr, from: &str, shape: &Shape, out: &mut Vec<Finding>) {
    // A form whose instance could not be read at all would report every
    // path as missing, which is noise rather than a finding.
    if shape.paths.is_empty() {
        return;
    }
    let mut wanted = Vec::new();
    gather(expr, from, true, &mut wanted);

    let mut seen = BTreeSet::new();
    for path in wanted {
        if !seen.insert(path.clone()) || shape.holds(&path) {
            continue;
        }
        // A path into a lookup table is checked by its table's name: an
        // external CSV has no inline shape to check the rest against.
        let root = path.split('/').nth(1).unwrap_or_default();
        if shape.tables.contains(root) {
            continue;
        }

        // A path that does exist beats a hint about the root: a form
        // author reading "/data/morador/maior" sees the fix at once.
        let suggestion = nearest(&path, &shape.paths)
            .map(|near| near.to_string())
            .or_else(|| match &shape.root {
                Some(name) if root != name => Some(format!(
                    "a path under /{name} — that is this form's instance root, not /{root}"
                )),
                _ => None,
            });
        out.push((
            format!("{path}, which this form's instance has no node for"),
            Breaks::Everywhere,
            "the path matches nothing, so the rule reads an empty node-set: a calculation \
             comes out 0, a comparison comes out false, and a relevant hides its question \
             for the whole of fieldwork"
                .into(),
            suggestion,
        ));
    }
}

/// Collect the paths whose meaning does not depend on where they sit.
///
/// An absolute path names the same node wherever it appears, so it can
/// always be checked. A relative one is read from the context node, and
/// inside a predicate that node is whatever the step selected — in
/// `instance('linhas')/root/item[name = /data/x]/lote`, `name` is a child
/// of the table's item and has nothing to do with the bind. Checking it
/// against the form's own instance reports a fault that is not there, and
/// a checker that cries wolf gets switched off.
///
/// So relative paths are checked only where the context is the bound node:
/// at the top level of the expression, which is where `../idade` lives.
fn gather(expr: &Expr, from: &str, at_top: bool, out: &mut Vec<String>) {
    match expr {
        Expr::Path { absolute, steps } => {
            if *absolute || at_top {
                if let Some(path) = crate::rules::referenced_paths(
                    &Expr::Path {
                        absolute: *absolute,
                        steps: steps.clone(),
                    },
                    from,
                )
                .into_iter()
                .next()
                {
                    out.push(path);
                }
            }
            for step in steps {
                for predicate in &step.predicates {
                    gather(predicate, from, false, out);
                }
            }
        }
        Expr::Filter {
            base,
            predicates,
            steps,
        } => {
            gather(base, from, false, out);
            for predicate in predicates {
                gather(predicate, from, false, out);
            }
            for step in steps {
                for predicate in &step.predicates {
                    gather(predicate, from, false, out);
                }
            }
        }
        Expr::Function { args, .. } => {
            for arg in args {
                gather(arg, from, at_top, out);
            }
        }
        Expr::Binary { left, right, .. } | Expr::Union(left, right) => {
            gather(left, from, at_top, out);
            gather(right, from, at_top, out);
        }
        Expr::Negate(inner) => gather(inner, from, at_top, out),
        Expr::Number(_) | Expr::Literal(_) | Expr::Variable(_) => {}
    }
}

/// The known path closest to a mistyped one, when there is an obvious
/// candidate — a suggestion is worth more than a complaint.
fn nearest<'a>(wanted: &str, known: &'a BTreeSet<String>) -> Option<&'a String> {
    let leaf = wanted.rsplit('/').next()?;
    let mut matches = known
        .iter()
        .filter(|path| path.ends_with(&format!("/{leaf}")));
    let first = matches.next()?;
    // Two candidates and this would be a guess, not a suggestion.
    match matches.next() {
        Some(_) => None,
        None => Some(first),
    }
}
