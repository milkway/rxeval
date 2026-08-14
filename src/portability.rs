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
}

impl Breaks {
    pub fn describe(&self) -> &'static str {
        match self {
            Breaks::Collect => "Collect / KoboCollect",
            Breaks::WebForms => "Enketo web forms",
            Breaks::Differently => "both, differently",
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
        format!(
            "{where_}: {} — {} on {}.{fix}",
            self.construct,
            self.effect,
            self.breaks.describe()
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
