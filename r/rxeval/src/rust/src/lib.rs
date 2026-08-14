//! R bindings for the rxeval engine.
//!
//! Three questions a form author actually asks, answered from the console
//! where they already work:
//!
//! - will this form mean the same thing on a tablet as in a browser?
//! - what do the form's own rules say about this submission?
//! - what does this expression evaluate to, against this data?
//!
//! Everything comes back as a data frame, because the next thing anyone
//! does with an answer in R is filter it.

use extendr_api::prelude::*;

/// Turn a Rust error into an R condition rather than a value.
///
/// A checker that returns an empty result when it could not run is
/// indistinguishable from one that ran and found nothing — which is the
/// failure this whole crate exists to avoid.
fn or_stop<T>(result: Result<T, String>) -> T {
    match result {
        Ok(value) => value,
        Err(why) => throw_r_error(why),
    }
}

/// Read a form from a path, or accept the XML directly.
///
/// Guessing between the two is safe here: an XForm always starts with a
/// tag, and no path does.
fn read_form(form: &str) -> Result<String, String> {
    let trimmed = form.trim_start();
    if trimmed.starts_with('<') {
        return Ok(form.to_string());
    }
    std::fs::read_to_string(form).map_err(|e| format!("cannot read {form}: {e}"))
}

/// Which expressions in a form will behave differently on a tablet than in
/// a browser.
///
/// @param form Path to an XForm, or the XML itself.
/// @return A data frame with one row per issue: `path`, `rule`,
///   `expression`, `construct`, `breaks`, `effect`, `suggestion`, `says`.
///   Zero rows means the form travels.
#[extendr]
fn rust_form_portability(form: &str) -> Robj {
    let xml = or_stop(read_form(form));
    let issues = or_stop(rxeval::check_form(&xml));

    let mut path = Vec::new();
    let mut rule = Vec::new();
    let mut expression = Vec::new();
    let mut construct = Vec::new();
    let mut breaks = Vec::new();
    let mut effect = Vec::new();
    let mut suggestion = Vec::new();
    let mut says = Vec::new();
    for issue in &issues {
        path.push(issue.path.clone());
        rule.push(issue.rule.clone());
        expression.push(issue.expression.clone());
        construct.push(issue.construct.clone());
        breaks.push(issue.breaks.describe().to_string());
        effect.push(issue.effect.clone());
        // NA, not "", when there is nothing to suggest: a missing
        // suggestion is not an empty one.
        suggestion.push(match &issue.suggestion {
            Some(text) => Rstr::from(text.clone()),
            None => <Rstr>::na(),
        });
        says.push(issue.describe());
    }

    data_frame_of(vec![
        ("path", Strings::from_values(path).into_robj()),
        ("rule", Strings::from_values(rule).into_robj()),
        ("expression", Strings::from_values(expression).into_robj()),
        ("construct", Strings::from_values(construct).into_robj()),
        ("breaks", Strings::from_values(breaks).into_robj()),
        ("effect", Strings::from_values(effect).into_robj()),
        ("suggestion", Strings::from_values(suggestion).into_robj()),
        ("says", Strings::from_values(says).into_robj()),
    ])
}

/// What a form's own rules say about a submission.
///
/// @param form Path to an XForm, or the XML itself.
/// @param submission Path to a submission, or its XML.
/// @param today,now What `today()` and `now()` should answer. Defaults to
///   the submission's own metadata, so a date rule is judged against the
///   day the work was done rather than the day of the check.
/// @return A data frame: `path`, `kind`, `says`. Zero rows means the
///   submission satisfies every rule the engine could evaluate — and a
///   rule it could not evaluate appears as kind `not-evaluated`, never as
///   a pass.
#[extendr]
fn rust_submission_findings(
    form: &str,
    submission: &str,
    today: Nullable<String>,
    now: Nullable<String>,
) -> Robj {
    let form_xml = or_stop(read_form(form));
    let submission_xml = or_stop(read_form(submission));
    let parsed = or_stop(rxeval::Form::parse(&form_xml));
    let instance = or_stop(rxeval::Instance::from_xml(&submission_xml));

    let metadata = |name: &str| -> Option<String> {
        let root = instance.root()?;
        instance
            .children(root)
            .into_iter()
            .find(|c| instance.node(*c).name == name)
            .map(|c| instance.string_value(c))
            .filter(|v| !v.trim().is_empty())
    };
    let clock = rxeval::Clock {
        today: match today {
            Nullable::NotNull(value) => value,
            Nullable::Null => metadata("today")
                .or_else(|| metadata("end").map(|e| e.chars().take(10).collect()))
                .unwrap_or_else(|| "1970-01-01".into()),
        },
        now: match now {
            Nullable::NotNull(value) => value,
            Nullable::Null => metadata("end")
                .or_else(|| metadata("start"))
                .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".into()),
        },
    };

    let findings = parsed.check(&instance, clock);
    let mut path = Vec::new();
    let mut kind = Vec::new();
    let mut says = Vec::new();
    for finding in &findings {
        path.push(finding.node_path.clone());
        kind.push(
            match &finding.kind {
                rxeval::ViolationKind::Constraint => "constraint",
                rxeval::ViolationKind::Required => "required",
                rxeval::ViolationKind::Calculation { .. } => "calculation",
                rxeval::ViolationKind::Failed(_) => "not-evaluated",
            }
            .to_string(),
        );
        says.push(finding.describe());
    }

    data_frame_of(vec![
        ("path", Strings::from_values(path).into_robj()),
        ("kind", Strings::from_values(kind).into_robj()),
        ("says", Strings::from_values(says).into_robj()),
    ])
}

/// Evaluate one XPath expression against a submission.
///
/// For working out what a rule actually does before writing it into a
/// form.
///
/// @param expression The XPath expression.
/// @param submission Path to a submission, or its XML.
/// @param today,now What `today()` and `now()` should answer.
/// @return A length-one character vector. An expression the engine cannot
///   evaluate raises an error rather than returning a plausible value.
#[extendr]
fn rust_eval_expression(
    expression: &str,
    submission: &str,
    today: Nullable<String>,
    now: Nullable<String>,
) -> String {
    let xml = or_stop(read_form(submission));
    let instance = or_stop(rxeval::Instance::from_xml(&xml));
    let env = rxeval::Fixed {
        today: match today {
            Nullable::NotNull(value) => value,
            Nullable::Null => "1970-01-01".into(),
        },
        now: match now {
            Nullable::NotNull(value) => value,
            Nullable::Null => "1970-01-01T00:00:00.000Z".into(),
        },
    };
    let value = or_stop(rxeval::eval_str(expression, &instance, &env));
    value.to_string_value(&instance)
}

/// Build a data frame with the right row count even when it is empty.
fn data_frame_of(columns: Vec<(&str, Robj)>) -> Robj {
    let rows = columns
        .first()
        .map(|(_, column)| column.len())
        .unwrap_or(0);
    let mut list = List::from_names_and_values(
        columns.iter().map(|(name, _)| *name),
        columns.iter().map(|(_, column)| column.clone()),
    )
    .expect("columns and names match")
    .into_robj();
    list.set_attrib("class", "data.frame").ok();
    // Compact row names, the form R itself uses for a fresh frame.
    list.set_attrib("row.names", (1..=rows as i32).collect::<Vec<i32>>())
        .ok();
    list
}

extendr_module! {
    mod rxeval;
    fn rust_form_portability;
    fn rust_submission_findings;
    fn rust_eval_expression;
}
