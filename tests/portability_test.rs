//! What the portability checker must catch.
//!
//! Each case is a form that reads fine, passes review, and behaves
//! differently — usually silently — depending on where it runs. The
//! expected behaviour of each engine was measured, not assumed: see
//! `tests/ecosystem_oracle_test.rs`.

use rxeval::portability::{check_form, Breaks};

fn form(binds: &str, body: &str) -> String {
    format!(
        r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml"
                   xmlns:jr="http://openrosa.org/javarosa">
  <h:head><model>
    <instance><data id="f">
      <a/><b/><cpf/>
      <household><resident><idade/></resident></household>
    </data></instance>
    {binds}
  </model></h:head>
  <h:body>{body}</h:body>
</h:html>"#
    )
}

fn repeat_body() -> &'static str {
    r#"<repeat nodeset="/data/household/resident">
         <input ref="/data/household/resident/idade"><label>idade</label></input>
       </repeat>"#
}

#[test]
fn a_positional_predicate_reads_as_empty_on_a_tablet() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string"
                 calculate="/data/household/resident[2]/idade"/>"#,
        repeat_body(),
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert_eq!(issues[0].breaks, Breaks::Collect);
    assert!(issues[0].construct.contains("[2]"), "{:?}", issues[0]);
    assert!(
        issues[0]
            .suggestion
            .as_deref()
            .unwrap()
            .contains("position() = 2"),
        "{:?}",
        issues[0]
    );
    // the message has to say where, and that nothing will look wrong
    let text = issues[0].describe();
    assert!(text.contains("/data/a"), "{text}");
    assert!(text.contains("calculate"), "{text}");
    assert!(text.contains("silently"), "{text}");
}

#[test]
fn functions_only_one_engine_has() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string" calculate="floor(1.9) + ceiling(1.1)"/>
           <bind nodeset="/data/b" type="string" calculate="substring(/data/cpf, 2, 3)"/>
           <bind nodeset="/data/cpf" type="string" constraint="last() &gt; 1"/>"#,
        "",
    );
    let issues = check_form(&xform).unwrap();
    let constructs: Vec<&str> = issues.iter().map(|i| i.construct.as_str()).collect();
    assert!(constructs.contains(&"floor()"), "{constructs:?}");
    assert!(constructs.contains(&"ceiling()"), "{constructs:?}");
    assert!(constructs.contains(&"substring()"), "{constructs:?}");
    assert!(constructs.contains(&"last()"), "{constructs:?}");
    assert!(issues.iter().all(|i| i.breaks == Breaks::Collect));
    // substr counts differently, and saying so is the whole point of the hint
    let substring = issues
        .iter()
        .find(|i| i.construct == "substring()")
        .unwrap();
    let hint = substring.suggestion.as_deref().unwrap();
    assert!(hint.contains("substr("), "{hint}");
    assert!(hint.contains("counts from 0"), "{hint}");
}

#[test]
fn the_other_direction_too() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string" calculate="enclosed-area(/data/b)"/>"#,
        "",
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].breaks, Breaks::WebForms);
    assert!(issues[0].describe().contains("web form"), "{:?}", issues[0]);
}

/// The worst kind: both engines run it and mean different things, so
/// nothing fails anywhere and the data quietly disagrees with itself.
#[test]
fn an_unanchored_pattern_means_two_things() {
    let xform = form(
        r#"<bind nodeset="/data/cpf" type="string" constraint="regex(., '[0-9]{11}')"/>"#,
        "",
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert_eq!(issues[0].breaks, Breaks::Differently);
    let text = issues[0].describe();
    assert!(text.contains("whole value"), "{text}");
    assert!(text.contains("anywhere"), "{text}");
    // grouped, so the anchors keep holding if the pattern later grows a bar
    assert_eq!(
        issues[0].suggestion.as_deref().unwrap(),
        "'^(?:[0-9]{11})$'"
    );

    // an anchored one travels, and must not be reported
    let anchored = form(
        r#"<bind nodeset="/data/cpf" type="string" constraint="regex(., '^[0-9]{11}$')"/>"#,
        "",
    );
    assert!(check_form(&anchored).unwrap().is_empty());
}

#[test]
fn comparing_a_field_that_repeats() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string"
                 relevant="/data/household/resident/idade = 7"/>"#,
        repeat_body(),
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert_eq!(issues[0].breaks, Breaks::Collect);
    assert!(issues[0].construct.contains("repeats"), "{:?}", issues[0]);

    // the same comparison on a field that does not repeat is fine
    let plain = form(
        r#"<bind nodeset="/data/a" type="string" relevant="/data/b = 7"/>"#,
        repeat_body(),
    );
    assert!(check_form(&plain).unwrap().is_empty());
}

#[test]
fn a_form_that_travels_reports_nothing() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string" calculate="concat(/data/b, '-')"/>
           <bind nodeset="/data/b" type="int" constraint=". &gt;= 0 and . &lt;= 120"/>
           <bind nodeset="/data/cpf" type="string" constraint="regex(., '^[0-9]{11}$')"
                 relevant="/data/b > 18"/>"#,
        repeat_body(),
    );
    assert!(
        check_form(&xform).unwrap().is_empty(),
        "{:#?}",
        check_form(&xform).unwrap()
    );
}

/// The real questionnaire, which has been in the field.
#[test]
fn the_psu_form() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("dados")
        .join("psu2026");
    let Ok(xform) = std::fs::read_to_string(dir.join("form.xml")) else {
        eprintln!("portability: dados/psu2026 not present — skipping");
        return;
    };
    let issues = check_form(&xform).unwrap();
    println!("PSU 2026: {} portability issue(s)", issues.len());
    for issue in &issues {
        println!("  {}", issue.describe());
    }
}

/// The rewrite this suggests has to be one that actually holds. Wrapping a
/// pattern in bare anchors is the obvious advice and it is wrong: with
/// alternation, `^a|b$` means "starts with a, or ends with b".
#[test]
fn the_suggested_rewrite_survives_alternation() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string"
                 constraint="regex(., '9[0-9]{8}|[2-5][0-9]{7}')"/>"#,
        "",
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1, "{issues:#?}");
    let fix = issues[0].suggestion.as_deref().unwrap();
    assert!(
        fix.contains("(?:"),
        "the fix must group the alternation, or it changes the meaning: {fix}"
    );
    assert_eq!(fix, "'^(?:9[0-9]{8}|[2-5][0-9]{7})$'");
}

/// A pattern that reads as anchored and is not, because the bar binds
/// looser than the anchors. This one used to pass unnoticed.
#[test]
fn anchors_that_only_look_like_anchors() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string" constraint="regex(., '^abc|def$')"/>"#,
        "",
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert!(
        issues[0].construct.contains("outside the anchors"),
        "{:?}",
        issues[0]
    );
    assert_eq!(
        issues[0].suggestion.as_deref().unwrap(),
        "'^(?:^abc|def$)$'"
    );

    // alternation safely inside a group is fine, which is what the real
    // PSU phone rule does
    let grouped = form(
        r#"<bind nodeset="/data/a" type="string"
                 constraint="regex(., '^(9[0-9]{8}|[2-5][0-9]{7})$')"/>"#,
        "",
    );
    assert!(check_form(&grouped).unwrap().is_empty());

    // and a bar inside a character class is not alternation at all
    let class = form(
        r#"<bind nodeset="/data/a" type="string" constraint="regex(., '^[a|b]+$')"/>"#,
        "",
    );
    assert!(check_form(&class).unwrap().is_empty());
}

/// A pattern built at runtime cannot be read, and saying so beats silence.
#[test]
fn a_pattern_that_is_not_written_out() {
    let xform = form(
        r#"<bind nodeset="/data/a" type="string" constraint="regex(., /data/b)"/>"#,
        "",
    );
    let issues = check_form(&xform).unwrap();
    assert_eq!(issues.len(), 1, "{issues:#?}");
    assert!(issues[0].suggestion.is_none());
    assert!(
        issues[0].describe().contains("built at runtime"),
        "{:?}",
        issues[0]
    );
}

/// A form built on `pulldata` cannot be filled on the web at all, and one
/// using `area` or `distance` gets a different number in each engine. Both
/// are worth saying at publish time rather than after a week of fieldwork.
#[test]
fn geography_and_pulldata_are_reported() {
    let xform = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml">
      <h:head><model>
        <instance><data id="p"><t/><a/><b/><c/></data></instance>
        <bind nodeset="/data/a" calculate="pulldata('lotes', 'nome', 'codigo', /data/t)"/>
        <bind nodeset="/data/b" calculate="distance(/data/t)"/>
        <bind nodeset="/data/c" calculate="area(/data/t)"/>
      </model></h:head><h:body/></h:html>"#;
    let issues = rxeval::check_form(xform).unwrap();
    let found: Vec<(&str, &str)> = issues
        .iter()
        .map(|i| (i.construct.as_str(), i.breaks.describe()))
        .collect();
    assert!(
        found.contains(&("pulldata()", "Enketo web forms")),
        "{found:?}"
    );
    assert!(
        found.contains(&("distance()", "both, differently")),
        "{found:?}"
    );
    assert!(
        found.contains(&("area()", "both, differently")),
        "{found:?}"
    );
}
