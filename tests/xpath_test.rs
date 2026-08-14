//! What the evaluator must get right, stated as expectations.
//!
//! Several of these encode behaviour that reads wrong out loud. They are
//! the places a hand-written engine drifts from JavaRosa, and drifts
//! quietly: the form still fills in, the answer is just different.

use rxeval::{eval_str, Fixed, Instance, Value};

fn instance() -> Instance {
    Instance::from_xml(
        r#"<data id="survey" version="3">
             <age>42</age>
             <income>1500.5</income>
             <consent>yes</consent>
             <services>water power</services>
             <blank/>
             <household>
               <resident><name>Ana</name><age>34</age></resident>
               <resident><name>Bia</name><age>7</age></resident>
               <resident><name>Caio</name><age>19</age></resident>
             </household>
           </data>"#,
    )
    .unwrap()
}

fn env() -> Fixed {
    Fixed {
        today: "2026-08-14".into(),
        now: "2026-08-14T09:30:00.000-03:00".into(),
    }
}

fn eval(expression: &str) -> Value {
    eval_str(expression, &instance(), &env()).unwrap_or_else(|e| panic!("{expression:?}: {e}"))
}

fn string(expression: &str) -> String {
    eval(expression).to_string_value(&instance())
}

fn number(expression: &str) -> f64 {
    eval(expression).to_number(&instance())
}

fn boolean(expression: &str) -> bool {
    eval(expression).to_boolean(&instance())
}

#[test]
fn paths_and_the_shapes_forms_use() {
    assert_eq!(string("/data/age"), "42");
    assert_eq!(string("/data/household/resident[2]/name"), "Bia");
    assert_eq!(number("count(/data/household/resident)"), 3.0);
    assert_eq!(string("/data/@id"), "survey");
    assert_eq!(string("//name"), "Ana");
    assert_eq!(number("count(//age)"), 4.0); // the root's and three residents'
    assert_eq!(string("/data/missing"), "");
    assert_eq!(number("count(/data/missing)"), 0.0);
}

#[test]
fn a_numeric_predicate_means_position() {
    // [1] is "the first one", not "the one whose value is 1"
    assert_eq!(string("/data/household/resident[1]/name"), "Ana");
    assert_eq!(string("/data/household/resident[last()]/name"), "Caio");
    assert_eq!(
        string("/data/household/resident[position() = 2]/name"),
        "Bia"
    );
    // and a predicate's position counts within the filtered set
    assert_eq!(string("/data/household/resident[age > 10][1]/name"), "Ana");
    assert_eq!(number("count(/data/household/resident[age > 10])"), 2.0);
}

#[test]
fn comparison_against_a_node_set_is_existential() {
    // true when ANY resident is 7, which is not what it reads like
    assert!(boolean("/data/household/resident/age = 7"));
    assert!(!boolean("/data/household/resident/age = 99"));
    // and so `!=` is not the negation of `=`: both can be true at once
    assert!(boolean("/data/household/resident/age != 7"));
}

#[test]
fn string_of_a_node_set_is_the_first_node_not_a_join() {
    assert_eq!(string("/data/household/resident/name"), "Ana");
    assert_eq!(string("concat(/data/household/resident/name, '!')"), "Ana!");
    // joining is a separate function, asked for on purpose
    assert_eq!(
        string("join(', ', /data/household/resident/name)"),
        "Ana, Bia, Caio"
    );
}

#[test]
fn numbers_format_as_xpath_says_not_as_rust_does() {
    assert_eq!(string("1 + 1"), "2"); // not "2.0"
    assert_eq!(string("3 div 2"), "1.5");
    assert_eq!(string("/data/income * 2"), "3001");
    assert_eq!(string("number('abc')"), "NaN");
    // NaN compares false against everything, itself included
    assert!(!boolean("number('abc') = number('abc')"));
    assert!(!boolean("number('abc') > 0"));
    assert!(!boolean("number('abc') <= 0"));
}

#[test]
fn an_unanswered_question_still_has_a_node() {
    // A node-set is true when it is not empty, whatever the nodes hold. So
    // an unanswered question is true in boolean context, because its
    // element is there and empty — which is why forms are told to write
    // `${q} != ''` rather than bare `${q}`.
    assert!(boolean("/data/blank"));
    assert_eq!(string("/data/blank"), "");
    assert!(!boolean("/data/blank != ''"));

    // absent is a different thing from empty: no node at all
    assert!(!boolean("/data/missing"));
    assert_eq!(number("count(/data/missing)"), 0.0);
    // and absent is not the number zero — a comparison over no nodes is
    // false, so neither this nor its opposite holds
    assert!(!boolean("/data/missing = 0"));
    assert!(!boolean("/data/missing != 0"));

    assert!(boolean("/data/age"));
}

#[test]
fn openrosa_selects() {
    assert!(boolean("selected(/data/services, 'water')"));
    assert!(boolean("selected(/data/services, 'power')"));
    assert!(!boolean("selected(/data/services, 'wat')"));
    assert_eq!(number("count-selected(/data/services)"), 2.0);
    assert_eq!(string("selected-at(/data/services, 1)"), "power");
    assert_eq!(string("selected-at(/data/services, 9)"), "");
}

#[test]
fn boolean_from_string_is_not_boolean() {
    // XPath calls any non-empty string true; ODK's version does not
    assert!(boolean("boolean('0')"));
    assert!(!boolean("boolean-from-string('0')"));
    assert!(boolean("boolean-from-string('1')"));
    assert!(boolean("boolean-from-string('true')"));
    assert!(!boolean("boolean-from-string('no')"));
}

#[test]
fn substring_counts_from_one_and_substr_from_zero() {
    assert_eq!(string("substring('hello', 2, 3)"), "ell");
    assert_eq!(string("substr('hello', 2, 3)"), "l");
    assert_eq!(string("substr('hello', 0, 2)"), "he");
    // substr takes negative offsets from the end
    assert_eq!(string("substr('hello', -2)"), "lo");
}

#[test]
fn round_takes_decimal_places_the_way_odk_means_it() {
    assert_eq!(number("round(1.5)"), 2.0);
    assert_eq!(number("round(1.2345, 2)"), 1.23);
    assert_eq!(number("round(1235.5, -2)"), 1200.0);
    assert_eq!(number("int(1.9)"), 1.0);
    assert_eq!(number("int(-1.9)"), -1.0);
}

#[test]
fn min_and_max_over_nothing_are_not_zero() {
    assert_eq!(number("max(/data/household/resident/age)"), 34.0);
    assert_eq!(number("min(/data/household/resident/age)"), 7.0);
    assert!(number("max(/data/missing)").is_nan());
    assert!(number("min(/data/missing)").is_nan());
    assert_eq!(number("sum(/data/household/resident/age)"), 60.0);
}

#[test]
fn if_evaluates_only_the_branch_it_takes() {
    assert_eq!(string("if(/data/age > 18, 'adult', 'child')"), "adult");
    // the untaken branch would divide by zero, and must not be reached
    assert_eq!(number("if(/data/blank != '', 1 div /data/blank, -1)"), -1.0);
}

#[test]
fn and_or_short_circuit() {
    // the right side would error if it ran: the guard exists for that
    assert!(!boolean("/data/missing and unknownfunction()"));
    assert!(boolean("/data/age or unknownfunction()"));
}

#[test]
fn relative_paths_and_parents() {
    let instance = instance();
    let env = env();
    let expr = rxeval::parse("../name").unwrap();
    // second resident's age node as context
    let residents = match eval_str("/data/household/resident[2]/age", &instance, &env).unwrap() {
        Value::NodeSet(nodes) => nodes,
        other => panic!("expected a node-set, got {other:?}"),
    };
    let value =
        rxeval::evaluate(&expr, &instance, rxeval::Context::at(residents[0]), &env).unwrap();
    assert_eq!(value.to_string_value(&instance), "Bia");
}

#[test]
fn time_comes_from_the_environment_not_the_clock() {
    assert_eq!(string("today()"), "2026-08-14");
    assert_eq!(string("now()"), "2026-08-14T09:30:00.000-03:00");
}

/// The rule the crate exists to keep: what it cannot do, it refuses to do.
#[test]
fn what_is_missing_says_so_instead_of_answering() {
    for expression in [
        "pulldata('x', 'y', 'z', 'w')",
        "indexed-repeat(/data/a, /data/b, 1)",
        "current()/age",
        "nosuchfunction(1)",
        "$var",
        // the day and month names depend on the form's language, which a
        // bare evaluator does not have
        "format-date-time(now(), '%a')",
        // and a choice label needs a choice list
        "jr:choice-name(/data/consent, '/data/consent')",
    ] {
        let outcome = eval_str(expression, &instance(), &env());
        assert!(
            outcome.is_err(),
            "{expression:?} produced {:?} instead of refusing",
            outcome.unwrap()
        );
    }
}

#[test]
fn errors_name_the_function_that_is_missing() {
    let message = eval_str("pulldata('a', 'b', 'c', 'd')", &instance(), &env()).unwrap_err();
    assert!(message.contains("pulldata"), "{message}");
    assert!(message.contains("not implemented"), "{message}");

    let message = eval_str("count('a string')", &instance(), &env()).unwrap_err();
    assert!(message.contains("count()"), "{message}");
    assert!(message.contains("a string"), "{message}");
}

#[test]
fn dates_format_with_odks_codes() {
    assert_eq!(
        string("format-date-time('2026-08-07T06:31:13.834-03:00', '%Y%m%d%H%M%S')"),
        "20260807063113"
    );
    assert_eq!(
        string("format-date-time('2026-08-07T06:31:13.834-03:00', '%3')"),
        "834"
    );
    // a date with no clock reads as midnight
    assert_eq!(
        string("format-date('2026-01-09', '%Y-%m-%d')"),
        "2026-01-09"
    );
    assert_eq!(string("format-date('2026-01-09', '%e/%n/%y')"), "9/1/26");
    // the offset is not applied: the local time is already what is written,
    // and shifting it would move the interview to a moment it did not happen
    assert_eq!(
        string("format-date-time('2026-08-07T23:30:00.000-03:00', '%d %H')"),
        "07 23"
    );
    // nothing in, nothing out
    assert_eq!(string("format-date('', '%Y')"), "");
    // and a value that is not a date says so
    assert!(eval_str("format-date('ontem', '%Y')", &instance(), &env()).is_err());
}

#[test]
fn regex_is_anchored_the_way_the_devices_do_it() {
    // JavaRosa matches the whole value even without anchors, and JavaRosa is
    // what collected the data. The spec says otherwise — getodk/javarosa#531
    assert!(boolean("regex('12345678901', '[0-9]{11}')"));
    assert!(!boolean("regex('a12345678901b', '[0-9]{11}')"));
    // explicit anchors change nothing, which is the point
    assert!(boolean("regex('12345678901', '^[0-9]{11}$')"));
    assert!(!boolean("regex('123', '[0-9]{11}')"));
    // a pattern this engine cannot build is refused, not treated as no match
    assert!(eval_str("regex('x', '(?<=a)b')", &instance(), &env()).is_err());
}
