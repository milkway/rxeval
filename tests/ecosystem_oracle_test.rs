//! Differential test against both reference implementations.
//!
//! Every expression in `tests/oracle/corpus.txt` is put to the two engines
//! the ecosystem actually runs on:
//!
//! - **JavaRosa**, inside ODK Collect and KoboCollect — the code that
//!   evaluated these forms on the tablets, and so the one whose answers
//!   already shaped the data this server stores.
//! - **Enketo**'s `openrosa-xpath-evaluator`, inside every web form.
//!
//! Their answers are committed, so this test needs neither Java nor Node;
//! regenerate them with `scripts/JavarosaOracle.java` and
//! `scripts/openrosa-oracle.mjs`.
//!
//! The PSU test shows this engine agrees with the client that collected one
//! dataset. This asks the harder question, and the answer turns out to be
//! that there is no single ecosystem language to adhere to: the two
//! references disagree with each other on a long list of expressions, and
//! JavaRosa refuses constructs Enketo evaluates happily. What this test
//! holds is that every difference is a decision on record.

use std::collections::BTreeMap;

use rxeval::{parse, Context, Fixed, Instance, Value};

/// Cases where the two engines are meant to differ, with the reason.
///
/// An empty list would be the goal but not the truth; what matters is that
/// every difference is here on purpose, named, rather than discovered later
/// in someone's data.
fn deliberate_divergences() -> BTreeMap<&'static str, &'static str> {
    // Empty, and that is the claim: there is no expression where the two
    // reference engines agree and this one does not. Every difference on
    // record is a place where they disagree with each other — see
    // `reference_disagreements`. An entry here would mean deliberately
    // speaking a third dialect, which would need a very good reason.
    BTreeMap::new()
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

/// Where the two references themselves disagree, and which one this engine
/// follows.
///
/// This is the useful half of the exercise. A form written against Enketo
/// and run on Collect will behave differently at every one of these, and
/// nothing in either tool says so.
fn reference_disagreements() -> BTreeMap<&'static str, (&'static str, &'static str)> {
    BTreeMap::from([
        // JavaRosa has no bare positional predicate: `resident[2]` matches
        // nothing there, while `[position() = 2]` works. A form using the
        // shorthand reads fine, passes review, and silently collects
        // nothing on a tablet.
        (
            "/data/household/resident[2]/nome",
            (
                "enketo",
                "JavaRosa returns nothing for a bare [n]; use [position() = n]",
            ),
        ),
        (
            "/data/household/resident[last()]/nome",
            ("enketo", "JavaRosa has no last()"),
        ),
        // JavaRosa implements neither the descendant-or-self shorthand nor
        // substring(); Enketo has both. Following Enketo makes this engine
        // a superset, which costs nothing: a form that avoids them runs
        // everywhere, and one that uses them at least means something here.
        ("//nome", ("enketo", "JavaRosa has no // axis")),
        ("count(//idade)", ("enketo", "JavaRosa has no // axis")),
        (
            "substring(\"hello\", 2)",
            ("enketo", "JavaRosa has no substring(); it has substr()"),
        ),
        (
            "substring(\"hello\", 2, 3)",
            ("enketo", "JavaRosa has no substring(); it has substr()"),
        ),
        (
            "substring(\"hello\", 1.5, 2.6)",
            ("enketo", "JavaRosa has no substring(); it has substr()"),
        ),
        (
            "substring(\"hello\", 0, 3)",
            (
                "xpath",
                "JavaRosa has no substring(); Enketo reads start <= 0 from the end",
            ),
        ),
        (
            "concat(/data/household/resident/nome, \"!\")",
            (
                "xpath",
                "JavaRosa refuses a node-set here; Enketo joins every value",
            ),
        ),
        // A container's value: everything below it, which is what Enketo
        // and XPath say. JavaRosa gives a group no value at all.
        // A container's value is every answer below it — XPath's rule.
        // Enketo adds the whitespace its DOM holds between elements;
        // JavaRosa gives a group no value at all. This engine keeps the
        // answers and not the layout.
        (
            ".",
            (
                "xpath",
                "Enketo includes the layout whitespace; JavaRosa gives nothing",
            ),
        ),
        (
            "once(/data/age)",
            (
                "xpath",
                "Enketo includes the layout whitespace; JavaRosa gives nothing",
            ),
        ),
        // JavaRosa refuses to compare a multi-node set against a number.
        // XPath — and Enketo — read it existentially: true when some node
        // matches. A form guarding on a repeat depends on that reading.
        (
            "/data/household/resident/idade = 7",
            ("enketo", "JavaRosa refuses a multi-node comparison"),
        ),
        (
            "/data/household/resident/idade != 7",
            ("enketo", "JavaRosa refuses a multi-node comparison"),
        ),
        // Comparing against something that is not there: XPath says no node
        // satisfies it, so false. JavaRosa reads the absent node as an
        // empty string and answers true.
        (
            "/data/missing != 0",
            ("enketo", "JavaRosa reads a missing node as an empty string"),
        ),
        // A node-set is true when it is not empty, whatever it holds.
        // JavaRosa looks at the value instead, so an unanswered question
        // is false there and true here.
        (
            "boolean(/data/blank)",
            (
                "enketo",
                "JavaRosa judges the value, not whether the node exists",
            ),
        ),
        // And the one where the collecting engine wins: JavaRosa accepts
        // any casing, Enketo only lowercase. A form that wrote "TRUE" was
        // already recorded as true on the tablet.
        (
            "boolean-from-string(\"TRUE\")",
            (
                "javarosa",
                "case-insensitive; Enketo accepts only lowercase",
            ),
        ),
        // Functions JavaRosa simply does not have.
        ("floor(1.9)", ("enketo", "JavaRosa has no floor()")),
        ("ceiling(1.1)", ("enketo", "JavaRosa has no ceiling()")),
        // Half away from zero, as XPath and Enketo do; JavaRosa rounds
        // half up, which sends -1.5 to -1.
        (
            "round(-1.5)",
            (
                "enketo",
                "JavaRosa rounds half up; XPath rounds half away from zero",
            ),
        ),
        // The one where this engine sides with JavaRosa against both the
        // specification and Enketo, because the tablets are what collected
        // the data being checked.
        (
            "regex(\"a12345678901b\", \"[0-9]{11}\")",
            (
                "javarosa",
                "anchored, against the spec and Enketo — getodk/javarosa#531",
            ),
        ),
        (
            "regex(\"abc123\", \"[0-9]+\")",
            (
                "javarosa",
                "anchored, against the spec and Enketo — getodk/javarosa#531",
            ),
        ),
    ])
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
    let enketo =
        read_json(&std::fs::read_to_string(dir.join("expected.json")).expect("Enketo's answers"));
    let javarosa = read_json(
        &std::fs::read_to_string(dir.join("javarosa-expected.json")).expect("JavaRosa's answers"),
    );
    let env = Fixed {
        today: "2026-08-14".into(),
        now: "2026-08-14T09:30:00.000-03:00".into(),
    };
    let ours_diverges = deliberate_divergences();
    let they_disagree = reference_disagreements();

    let mut all_three = 0;
    let mut split = Vec::new();
    let mut surprises = Vec::new();
    let mut compared = 0;

    for (expression, enketo_entry) in &enketo {
        if expression == "$meta" {
            continue;
        }
        compared += 1;
        let theirs = reference_answer(enketo_entry);
        let jr = javarosa.get(expression).map(reference_answer);
        let ours = ours(expression, &instance, &env);

        let matches_enketo = agree(&ours, &theirs, &instance);
        let matches_javarosa = jr.as_ref().is_some_and(|a| agree(&ours, a, &instance));
        let references_agree = jr.as_ref().is_some_and(|a| agree(a, &theirs, &instance));

        if references_agree {
            // One language, and this engine has to speak it.
            if matches_enketo {
                all_three += 1;
                if ours_diverges.contains_key(expression.as_str()) {
                    surprises.push(format!(
                        "{expression}\n     listed as a deliberate divergence, but all three now \
                         agree — remove the entry"
                    ));
                }
            } else {
                match ours_diverges.get(expression.as_str()) {
                    Some(why) => split.push(format!("{expression}  (both references agree; we differ: {why})")),
                    None => surprises.push(format!(
                        "{expression}\n     both references say: {theirs:?}\n     rxeval:            {ours:?}"
                    )),
                }
            }
            continue;
        }

        // The references disagree. Following one of them is a decision, and
        // it has to be written down.
        match they_disagree.get(expression.as_str()) {
            Some((side, why)) => {
                let followed = match *side {
                    "javarosa" => matches_javarosa,
                    "enketo" => matches_enketo,
                    // neither: we follow XPath where both wander off
                    _ => !matches_enketo && !matches_javarosa,
                };
                if followed {
                    split.push(format!("{expression}  (follows {side}: {why})"));
                } else {
                    surprises.push(format!(
                        "{expression}\n     recorded as following {side} ({why}), but does not\n     \
                         enketo:   {theirs:?}\n     javarosa: {jr:?}\n     rxeval:   {ours:?}"
                    ));
                }
            }
            None => surprises.push(format!(
                "{expression}\n     the two references disagree and nothing says which we follow\n     \
                 enketo:   {theirs:?}\n     javarosa: {jr:?}\n     rxeval:   {ours:?}"
            )),
        }
    }

    println!(
        "ecosystem oracle: {compared} expressions | {all_three} where both references agree and \
         so do we | {} where they part ways",
        split.len()
    );
    for note in &split {
        println!("  {note}");
    }

    assert!(
        surprises.is_empty(),
        "{} expression(s) undecided or wrong:\n\n  {}\n",
        surprises.len(),
        surprises.join("\n\n  ")
    );
    assert!(
        all_three > 80,
        "only {all_three} expressions had both references agreeing; the fixtures may be stale"
    );
}
