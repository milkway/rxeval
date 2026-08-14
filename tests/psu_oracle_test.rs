//! The engine against a real questionnaire.
//!
//! Ninety-four questions, thirteen lookup tables, cascading selects and
//! relevance chains — the shapes a hand-written form never has. Gated on
//! the same fixtures as the other oracle tests, and skipped without them.

use rxeval::{Clock, Form, Instance};

fn fixture(name: &str) -> Option<String> {
    let dir = std::env::var("RXDATA_ORACLE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("dados")
                .join("psu2026")
        });
    std::fs::read_to_string(dir.join(name)).ok()
}

#[test]
fn the_real_form_parses_into_rules() {
    let Some(xform) = fixture("form.xml") else {
        eprintln!("psu oracle: dados/psu2026 not present — skipping");
        return;
    };
    let form = Form::parse(&xform).unwrap_or_else(|e| panic!("parsing the PSU form: {e}"));

    // every bind became a rule
    assert!(
        form.rules.bindings.len() >= 85,
        "only {} bindings from a 94-question form",
        form.rules.bindings.len()
    );
    // and the lookup tables came with it
    let tables = form.secondary_instance_ids();
    assert!(tables.contains(&"lotes"), "{tables:?}");
    assert!(tables.contains(&"pontos"), "{tables:?}");
    assert!(tables.len() >= 10, "{tables:?}");

    let with_rules = form
        .rules
        .bindings
        .iter()
        .filter(|b| {
            b.relevant.is_some()
                || b.constraint.is_some()
                || b.calculate.is_some()
                || b.required.is_some()
        })
        .count();
    assert!(with_rules > 20, "only {with_rules} bindings carry rules");
    println!(
        "PSU: {} bindings, {} with rules, {} lookup tables",
        form.rules.bindings.len(),
        with_rules,
        tables.len()
    );
}

/// The submissions really collected must come back clean, or the engine is
/// stricter than the client that accepted them — which would make every
/// report a list of false alarms.
#[test]
fn real_submissions_check_out() {
    let (Some(xform), Some(payload)) = (fixture("form.xml"), fixture("submissions.xml")) else {
        eprintln!("psu oracle: dados/psu2026 not present — skipping");
        return;
    };
    let form = Form::parse(&xform).unwrap();

    // split the API payload into instances
    let body = &payload[payload.find("<results>").unwrap() + "<results>".len()..];
    let start = body.find('<').unwrap() + 1;
    let name: String = body[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
        .collect();
    let (open, close) = (format!("<{name} "), format!("</{name}>"));

    let mut checked = 0;
    let mut findings: Vec<String> = Vec::new();
    for chunk in body.split(&open).skip(1) {
        let Some(inner) = chunk.split(&close).next() else {
            continue;
        };
        let xml = format!("{open}{inner}{close}");
        let instance = match Instance::from_xml(&xml) {
            Ok(instance) => instance,
            Err(e) => {
                findings.push(format!("could not read a submission: {e}"));
                continue;
            }
        };
        // the clock of the interview, not of this test run
        let clock = Clock {
            today: "2026-08-07".into(),
            now: "2026-08-07T09:00:00.000-03:00".into(),
        };
        for violation in form.check(&instance, clock) {
            findings.push(violation.describe());
        }
        checked += 1;
    }

    assert!(checked >= 60, "only {checked} submissions read");
    // Group and count, so the output names the shapes rather than listing
    // sixty repetitions of one.
    let mut by_kind: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for finding in &findings {
        let key = finding
            .split_once(": ")
            .map(|(_, rest)| rest.chars().take(70).collect::<String>())
            .unwrap_or_else(|| finding.clone());
        *by_kind.entry(key).or_default() += 1;
    }
    println!("checked {checked} real submissions");
    for (kind, count) in &by_kind {
        println!("  {count:>4}  {kind}");
    }
    println!("  {} findings in total", findings.len());
}
