//! What the rule graph must get right.
//!
//! Each of these is a way a form engine can be wrong without failing: the
//! submission is accepted, the report runs, and the number at the bottom is
//! not the number that was collected.

use rxeval::rules::{referenced_paths, ViolationKind};
use rxeval::{parse, Binding, Fixed, Instance, Rules};

fn env() -> Fixed {
    Fixed {
        today: "2026-08-14".into(),
        now: "2026-08-14T09:30:00.000-03:00".into(),
    }
}

fn binding(path: &str) -> Binding {
    Binding::new(path)
}

fn rules(bindings: Vec<Binding>) -> Rules {
    Rules::new(bindings).unwrap_or_else(|e| panic!("building rules: {e}"))
}

#[test]
fn a_constraint_judges_the_answer_it_was_given() {
    let mut age = binding("/data/age");
    age.constraint = Some(parse(". >= 0 and . <= 120").unwrap());
    age.constraint_message = Some("age must be between 0 and 120".into());
    let rules = rules(vec![age]);

    let ok = Instance::from_xml("<data><age>42</age></data>").unwrap();
    assert!(rules.check(&ok, &env()).is_empty());

    let bad = Instance::from_xml("<data><age>140</age></data>").unwrap();
    let found = rules.check(&bad, &env());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, ViolationKind::Constraint);
    assert!(found[0].describe().contains("between 0 and 120"));

    // no answer is not a constraint failure — that is what required is for
    let empty = Instance::from_xml("<data><age/></data>").unwrap();
    assert!(rules.check(&empty, &env()).is_empty());
}

#[test]
fn required_fires_only_when_the_question_is_asked() {
    let mut consent = binding("/data/consent");
    consent.required = Some(parse("true()").unwrap());
    let mut name = binding("/data/name");
    name.required = Some(parse("true()").unwrap());
    name.relevant = Some(parse("/data/consent = 'yes'").unwrap());
    let rules = rules(vec![consent, name]);

    // consent refused: the name was never asked, so its absence is correct
    let refused = Instance::from_xml("<data><consent>no</consent><name/></data>").unwrap();
    assert!(
        rules.check(&refused, &env()).is_empty(),
        "an unasked question was reported as unanswered"
    );

    // consent given: now the blank name is a real omission
    let given = Instance::from_xml("<data><consent>yes</consent><name/></data>").unwrap();
    let found = rules.check(&given, &env());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, ViolationKind::Required);
    assert!(found[0].node_path.ends_with("/name"));
}

#[test]
fn relevance_cascades_from_the_group_down() {
    let mut group = binding("/data/details");
    group.relevant = Some(parse("/data/consent = 'yes'").unwrap());
    let mut phone = binding("/data/details/phone");
    // the child's own relevance says yes, but its group says no
    phone.relevant = Some(parse("true()").unwrap());
    phone.required = Some(parse("true()").unwrap());
    phone.constraint = Some(parse("string-length(.) = 11").unwrap());
    let rules = rules(vec![group, phone]);

    let hidden = Instance::from_xml(
        "<data><consent>no</consent><details><phone>123</phone></details></data>",
    )
    .unwrap();
    assert!(
        rules.check(&hidden, &env()).is_empty(),
        "a rule was enforced on an answer inside a hidden group"
    );

    let shown = Instance::from_xml(
        "<data><consent>yes</consent><details><phone>123</phone></details></data>",
    )
    .unwrap();
    let found = rules.check(&shown, &env());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, ViolationKind::Constraint);
}

#[test]
fn calculations_run_in_dependency_order_not_declaration_order() {
    // total depends on subtotal, which depends on price — declared backwards
    let mut total = binding("/data/total");
    total.calculate = Some(parse("/data/subtotal * 2").unwrap());
    let mut subtotal = binding("/data/subtotal");
    subtotal.calculate = Some(parse("/data/price + 10").unwrap());
    let price = binding("/data/price");
    let rules = rules(vec![total, subtotal, price]);

    let instance = Instance::from_xml(
        "<data><total>40</total><subtotal>20</subtotal><price>10</price></data>",
    )
    .unwrap();
    assert!(
        rules.check(&instance, &env()).is_empty(),
        "{:?}",
        rules
            .check(&instance, &env())
            .iter()
            .map(|v| v.describe())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_stored_calculation_that_disagrees_with_the_form_is_reported() {
    let mut total = binding("/data/total");
    total.calculate = Some(parse("/data/a + /data/b").unwrap());
    let rules = rules(vec![total, binding("/data/a"), binding("/data/b")]);

    let tampered = Instance::from_xml("<data><a>2</a><b>3</b><total>99</total></data>").unwrap();
    let found = rules.check(&tampered, &env());
    assert_eq!(found.len(), 1);
    match &found[0].kind {
        ViolationKind::Calculation { stored, computed } => {
            assert_eq!(stored, "99");
            assert_eq!(computed, "5");
        }
        other => panic!("expected a calculation mismatch, got {other:?}"),
    }
}

#[test]
fn calculations_that_feed_each_other_are_refused_at_build_time() {
    let mut a = binding("/data/a");
    a.calculate = Some(parse("/data/b + 1").unwrap());
    let mut b = binding("/data/b");
    b.calculate = Some(parse("/data/a + 1").unwrap());

    let error = Rules::new(vec![a, b]).unwrap_err();
    assert!(error.contains("depend on each other"), "{error}");
    assert!(error.contains("/data/a"), "{error}");
    assert!(error.contains("/data/b"), "{error}");
}

#[test]
fn every_repeat_instance_is_checked_on_its_own() {
    let mut age = binding("/data/resident/age");
    age.constraint = Some(parse(". < 120").unwrap());
    let rules = rules(vec![age]);

    let instance = Instance::from_xml(
        "<data>
           <resident><age>34</age></resident>
           <resident><age>200</age></resident>
           <resident><age>19</age></resident>
         </data>",
    )
    .unwrap();
    let found = rules.check(&instance, &env());
    assert_eq!(found.len(), 1, "expected exactly the middle resident");
    assert_eq!(found[0].path, "/data/resident/age");
}

#[test]
fn a_rule_inside_a_repeat_reads_its_own_instance() {
    // the classic: `../` must mean this resident, not the first one
    let mut check = binding("/data/resident/adult");
    check.calculate = Some(parse("if(../age >= 18, 'yes', 'no')").unwrap());
    let rules = rules(vec![check, binding("/data/resident/age")]);

    let instance = Instance::from_xml(
        "<data>
           <resident><age>34</age><adult>yes</adult></resident>
           <resident><age>7</age><adult>no</adult></resident>
         </data>",
    )
    .unwrap();
    assert!(
        rules.check(&instance, &env()).is_empty(),
        "{:?}",
        rules
            .check(&instance, &env())
            .iter()
            .map(|v| v.describe())
            .collect::<Vec<_>>()
    );

    // and a wrong one is caught in the instance it belongs to
    let wrong = Instance::from_xml(
        "<data>
           <resident><age>34</age><adult>yes</adult></resident>
           <resident><age>7</age><adult>yes</adult></resident>
         </data>",
    )
    .unwrap();
    let found = rules.check(&wrong, &env());
    assert_eq!(found.len(), 1);
    assert!(matches!(found[0].kind, ViolationKind::Calculation { .. }));
}

#[test]
fn a_rule_that_cannot_run_is_reported_not_assumed() {
    let mut field = binding("/data/x");
    field.constraint = Some(parse("pulldata('t', 'c', 'k', .) = 'sim'").unwrap());
    let rules = rules(vec![field]);

    let instance = Instance::from_xml("<data><x>2026-01-01</x></data>").unwrap();
    let found = rules.check(&instance, &env());
    assert_eq!(found.len(), 1);
    match &found[0].kind {
        ViolationKind::Failed(why) => assert!(why.contains("pulldata"), "{why}"),
        other => panic!("expected a failure to evaluate, got {other:?}"),
    }
}

#[test]
fn relative_dependencies_resolve_against_the_binding() {
    // `../age` written on /data/resident/adult means /data/resident/age
    let expr = parse("if(../age >= 18, 'yes', 'no')").unwrap();
    let paths = referenced_paths(&expr, "/data/resident/adult");
    assert!(
        paths.contains(&"/data/resident/age".to_string()),
        "{paths:?}"
    );

    // an absolute path is itself
    let expr = parse("/data/total + 1").unwrap();
    assert_eq!(
        referenced_paths(&expr, "/data/x"),
        vec!["/data/total".to_string()]
    );

    // a wildcard names no single path, and must not invent one
    let expr = parse("count(/data/*)").unwrap();
    assert!(referenced_paths(&expr, "/data/x").is_empty());
}

// ---------------------------------------------------------------------------
// From a real XLSForm, through the XForm it generates
// ---------------------------------------------------------------------------

fn sheet(rows: &[&[&str]]) -> rxform::xls::Sheet {
    rxform::xls::sheet_from_rows(
        rows.iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect()),
    )
}

/// Spreadsheet → XForm → rules, which is the path a real form takes.
fn rules_of(workbook: &rxform::xls::Workbook, name: &str) -> Result<Rules, String> {
    let survey = rxform::parser::parse(workbook, name).map_err(|e| e.to_string())?;
    let xform = rxform::xform::generate(&survey).map_err(|e| e.to_string())?;
    rxeval::rules::from_xform(&xform)
}

#[test]
fn rules_come_out_of_the_form_the_device_was_given() {
    let workbook = rxform::xls::Workbook {
        survey: sheet(&[
            &[
                "type",
                "name",
                "label",
                "relevant",
                "constraint",
                "required",
                "calculation",
                "constraint_message",
            ],
            &[
                "integer",
                "idade",
                "Idade",
                "",
                ". >= 0 and . <= 120",
                "yes",
                "",
                "idade entre 0 e 120",
            ],
            &[
                "begin group",
                "adulto",
                "Adulto",
                "${idade} >= 18",
                "",
                "",
                "",
                "",
            ],
            &[
                "text",
                "cpf",
                "CPF",
                "",
                "string-length(.) = 11",
                "yes",
                "",
                "CPF tem 11 digitos",
            ],
            &["end group", "", "", "", "", "", "", ""],
            &[
                "calculate",
                "faixa",
                "",
                "",
                "",
                "",
                "if(${idade} >= 60, 'idoso', 'nao')",
                "",
            ],
        ]),
        settings: sheet(&[&["form_id", "form_title"], &["cadastro", "Cadastro"]]),
        ..Default::default()
    };
    let rules = rules_of(&workbook, "cadastro").unwrap();

    // a minor: the group is hidden, so the CPF rule must not fire
    let minor = Instance::from_xml(
        "<data><idade>7</idade><adulto><cpf/></adulto><faixa>nao</faixa></data>",
    )
    .unwrap();
    let found = rules.check(&minor, &env());
    assert!(
        found.is_empty(),
        "rules fired on a hidden group: {:?}",
        found.iter().map(|v| v.describe()).collect::<Vec<_>>()
    );

    // an adult with a short CPF, reported in the form author's own words
    let adult = Instance::from_xml(
        "<data><idade>30</idade><adulto><cpf>123</cpf></adulto><faixa>nao</faixa></data>",
    )
    .unwrap();
    let found = rules.check(&adult, &env());
    assert_eq!(
        found.len(),
        1,
        "{:?}",
        found.iter().map(|v| v.describe()).collect::<Vec<_>>()
    );
    assert!(found[0].describe().contains("11 digitos"), "{:?}", found[0]);

    // an impossible age
    let impossible = Instance::from_xml(
        "<data><idade>200</idade><adulto><cpf>12345678901</cpf></adulto><faixa>nao</faixa></data>",
    )
    .unwrap();
    let found = rules.check(&impossible, &env());
    assert!(
        found.iter().any(|v| v.describe().contains("entre 0 e 120")),
        "{:?}",
        found.iter().map(|v| v.describe()).collect::<Vec<_>>()
    );

    // and the derived value is recomputed from the answers that feed it
    let wrong_band = Instance::from_xml(
        "<data><idade>70</idade><adulto><cpf>12345678901</cpf></adulto><faixa>nao</faixa></data>",
    )
    .unwrap();
    let found = rules.check(&wrong_band, &env());
    assert!(
        found
            .iter()
            .any(|v| matches!(v.kind, ViolationKind::Calculation { .. })),
        "{:?}",
        found.iter().map(|v| v.describe()).collect::<Vec<_>>()
    );
}

#[test]
fn an_expression_the_engine_cannot_evaluate_is_named_with_its_path() {
    let xform = r#"<h:html xmlns:h="http://www.w3.org/1999/xhtml" xmlns:jr="http://openrosa.org/javarosa">
      <h:head><model>
        <instance><data id="f"><x/></data></instance>
        <bind nodeset="/data/x" type="int" constraint=". &gt;"/>
      </model></h:head>
      <h:body/>
    </h:html>"#;
    let error = rxeval::rules::from_xform(xform).unwrap_err();
    assert!(error.contains("constraint"), "{error}");
    assert!(error.contains("/data/x"), "{error}");
}

#[test]
fn a_form_without_binds_says_so() {
    let xform = r#"<h:html xmlns:h="http://www.w3.org/1999/xhtml">
      <h:head><model><instance><data id="f"><x/></data></instance></model></h:head>
      <h:body/>
    </h:html>"#;
    assert!(rxeval::rules::from_xform(xform)
        .unwrap_err()
        .contains("no binds"));
}
