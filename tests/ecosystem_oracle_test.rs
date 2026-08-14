//! Differential test against the reference implementation.
//!
//! Every expression in `tests/oracle/corpus.txt` was put to Enketo's
//! `openrosa-xpath-evaluator` — the engine that runs in every Enketo web
//! form — and its answers are recorded in `tests/oracle/expected.json`.
//! This puts the same expressions to rxeval and compares.
//!
//! The PSU test proves this engine agrees with the client that collected
//! one dataset. This one asks a different question: does it implement the
//! same language as the rest of the ecosystem? A form written elsewhere,
//! against another server, has to mean the same thing here.
//!
//! Regenerate the expectations with `scripts/openrosa-oracle.mjs`, which
//! needs Node. The result is committed so this test does not.

use std::collections::BTreeMap;

use rxeval::{parse, Context, Fixed, Instance, Value};

/// Cases where the two engines are meant to differ, with the reason.
///
/// An empty list would be the goal but not the truth; what matters is that
/// every difference is here on purpose, named, rather than discovered later
/// in someone's data.
fn deliberate_divergences() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        // Enketo follows the specification: a pattern matches any part of
        // the value unless anchored. JavaRosa anchors always
        // (getodk/javarosa#531), and JavaRosa is what runs on the devices
        // whose submissions this engine checks — agreeing with the spec
        // here would flag answers the collecting app accepted.
        (
            "regex(\"a12345678901b\", \"[0-9]{11}\")",
            "anchored like JavaRosa, unanchored like the spec and Enketo",
        ),
        (
            "regex(\"abc123\", \"[0-9]+\")",
            "anchored like JavaRosa, unanchored like the spec and Enketo",
        ),
        // XPath 1.0 defines substring by position: characters at p where
        // start <= p < start + length, counting from 1. Enketo instead
        // reads a start of zero or less as an offset from the end, so
        // substring("hello", 0, 3) gives it "o" where the specification
        // gives "he". Forms do not write a non-positive start on purpose,
        // and following the spec is what JavaRosa does.
        (
            "substring(\"hello\", 0, 3)",
            "XPath position semantics; Enketo reads start <= 0 from the end",
        ),
        // A container's value is every answer below it. Both engines agree
        // on that; they differ on the layout between them, because this one
        // keeps values and not the whitespace a serializer would insert.
        (".", "same answers, without the indentation between them"),
        (
            "once(/data/age)",
            "same answers, without the indentation between them",
        ),
        // With one node-set argument both join every value. With more than
        // one, Enketo keeps joining while XPath — and JavaRosa — convert
        // each argument through string(), which takes the first node.
        (
            "concat(/data/household/resident/nome, \"!\")",
            "string() of a node-set is its first node; Enketo joins them all",
        ),
    ])
}

#[derive(Debug, PartialEq)]
enum Answer {
    Boolean(bool),
    Number(String),
    String(String),
    NodeSet(Vec<String>),
    Refused,
}

fn read_json(text: &str) -> BTreeMap<String, serde_json::Value> {
    serde_json::from_str(text).expect("the recorded expectations are JSON")
}

fn reference_answer(entry: &serde_json::Value) -> Answer {
    match entry["type"].as_str().unwrap_or_default() {
        "boolean" => Answer::Boolean(entry["value"].as_bool().unwrap_or_default()),
        "number" => Answer::Number(match &entry["value"] {
            serde_json::Value::String(s) => s.clone(),
            other => number_text(other.as_f64().unwrap_or(f64::NAN)),
        }),
        "string" => Answer::String(entry["value"].as_str().unwrap_or_default().to_string()),
        "nodeset" => Answer::NodeSet(
            entry["value"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default(),
        ),
        _ => Answer::Refused,
    }
}

/// Both engines print numbers their own way; compare what they mean.
fn number_text(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // enough places to catch a real difference, few enough to ignore the
    // last bit of a float
    if n == 0.0 {
        // -0.0 and 0.0 are the same number, and only one of them reads
        // like an answer
        return "0".to_string();
    }
    let rounded = format!("{:.10}", n);
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn ours(expression: &str, instance: &Instance, env: &Fixed) -> Answer {
    let Ok(expr) = parse(expression) else {
        return Answer::Refused;
    };
    let Some(root) = instance.root() else {
        return Answer::Refused;
    };
    match rxeval::evaluate(&expr, instance, Context::at(root), env) {
        Err(_) => Answer::Refused,
        Ok(Value::Boolean(b)) => Answer::Boolean(b),
        Ok(Value::Number(n)) => Answer::Number(number_text(n)),
        Ok(Value::String(s)) => Answer::String(s),
        Ok(Value::NodeSet(nodes)) => Answer::NodeSet(
            nodes
                .iter()
                .map(|n| instance.string_value(*n))
                .collect::<Vec<_>>(),
        ),
    }
}

/// A string and a one-node node-set of the same text are the same answer.
///
/// XPath converts a node-set the moment anything asks it for a string, and
/// which of the two an engine hands back is an implementation detail; the
/// value a form goes on to use is not.
fn agree(a: &Answer, b: &Answer, instance: &Instance) -> bool {
    let flatten = |answer: &Answer| -> Option<String> {
        match answer {
            Answer::String(s) => Some(s.clone()),
            Answer::NodeSet(values) => Some(values.first().cloned().unwrap_or_default()),
            _ => None,
        }
    };
    let _ = instance;
    match (a, b) {
        (Answer::NodeSet(x), Answer::NodeSet(y)) => x == y,
        (Answer::String(_) | Answer::NodeSet(_), Answer::String(_) | Answer::NodeSet(_)) => {
            flatten(a) == flatten(b)
        }
        _ => a == b,
    }
}

#[test]
fn rxeval_means_what_the_ecosystem_means() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("oracle");
    let instance = Instance::from_xml(
        &std::fs::read_to_string(dir.join("instance.xml")).expect("the shared instance"),
    )
    .expect("the shared instance parses");
    let expected = read_json(
        &std::fs::read_to_string(dir.join("expected.json")).expect("the recorded expectations"),
    );
    let env = Fixed {
        today: "2026-08-14".into(),
        now: "2026-08-14T09:30:00.000-03:00".into(),
    };
    let divergences = deliberate_divergences();

    let mut agreed = 0;
    let mut on_purpose = Vec::new();
    let mut surprises = Vec::new();

    for (expression, entry) in &expected {
        let theirs = reference_answer(entry);
        let ours = ours(expression, &instance, &env);
        let via = entry["via"].as_str().unwrap_or("openrosa");

        if agree(&ours, &theirs, &instance) {
            agreed += 1;
            // A divergence that stopped diverging is worth knowing about:
            // the note explaining it is now a lie in the source.
            if divergences.contains_key(expression.as_str()) {
                surprises.push(format!(
                    "{expression}\n     listed as a deliberate divergence, but the two now agree \
                     — remove the entry"
                ));
            }
            continue;
        }

        match divergences.get(expression.as_str()) {
            Some(why) => on_purpose.push(format!("{expression}  ({why})")),
            None => surprises.push(format!(
                "{expression}\n     reference [{via}]: {theirs:?}\n     rxeval:      {ours:?}"
            )),
        }
    }

    println!(
        "ecosystem oracle: {agreed} of {} expressions agree, {} differ on purpose",
        expected.len(),
        on_purpose.len()
    );
    for note in &on_purpose {
        println!("  by design: {note}");
    }

    assert!(
        surprises.is_empty(),
        "{} expression(s) mean something different here than in Enketo:\n\n  {}\n",
        surprises.len(),
        surprises.join("\n\n  ")
    );
    assert!(
        agreed > 100,
        "only {agreed} expressions were compared; the corpus may not have loaded"
    );
}
