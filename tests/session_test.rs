//! Filling in a form, one answer at a time.
//!
//! The engine already knew how to judge a finished submission. These tests
//! are about the other direction: a form being typed into, where the
//! question is not "is this valid" but "what should the screen show now".

use rxeval::{Clock, Session};

fn clock() -> Clock {
    Clock {
        today: "2026-08-14".into(),
        now: "2026-08-14T09:00:00.000-03:00".into(),
    }
}

const FORM: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml">
  <h:head>
    <model>
      <instance>
        <data id="visit">
          <age/>
          <adult/>
          <guardian/>
          <years_left/>
          <total/>
          <meta><instanceID/></meta>
        </data>
      </instance>
      <bind nodeset="/data/age" type="int" required="true()"
            constraint=". &gt; 0 and . &lt; 130"
            jr:constraintMsg="idade fora do intervalo"
            xmlns:jr="http://openrosa.org/javarosa"/>
      <bind nodeset="/data/adult" type="string" calculate="if(/data/age &gt;= 18, 'yes', 'no')"/>
      <bind nodeset="/data/guardian" type="string" relevant="/data/adult = 'no'" required="true()"/>
      <bind nodeset="/data/years_left" type="int" calculate="18 - /data/age"/>
      <bind nodeset="/data/total" type="int" calculate="/data/years_left * 2"/>
    </model>
  </h:head>
  <h:body/>
</h:html>"#;

#[test]
fn a_calculation_reaches_the_one_that_reads_it_in_the_same_pass() {
    let mut session = Session::new(FORM, clock()).unwrap();
    session.set("/data/age", "10").unwrap();
    let outcome = session.recompute();

    // years_left is 8, and total reads years_left — both settle in one
    // pass, because the calculations run in dependency order rather than
    // in the order the form happens to list them.
    assert_eq!(session.get("/data/years_left"), "8");
    assert_eq!(session.get("/data/total"), "16");
    assert_eq!(
        outcome.calculated.get("/data/total").map(String::as_str),
        Some("16")
    );
}

#[test]
fn only_what_changed_comes_back() {
    let mut session = Session::new(FORM, clock()).unwrap();
    session.set("/data/age", "10").unwrap();
    session.recompute();

    // Nothing moved, so nothing is reported. A renderer that redrew every
    // calculated field on every keystroke would fight the cursor.
    let again = session.recompute();
    assert!(again.calculated.is_empty(), "{:?}", again.calculated);

    session.set("/data/age", "11").unwrap();
    let outcome = session.recompute();
    // adult recomputes — 11 is still under 18 — but lands on the same
    // text, so it is not reported. Reported means moved, not ran.
    assert_eq!(
        outcome.calculated.keys().collect::<Vec<_>>(),
        vec!["/data/total", "/data/years_left"]
    );
}

#[test]
fn a_question_nobody_is_asked_is_not_a_missing_answer() {
    let mut session = Session::new(FORM, clock()).unwrap();
    session.set("/data/age", "40").unwrap();
    let outcome = session.recompute();

    // guardian is required, and unanswered, and irrelevant — an adult has
    // no guardian. Reporting it would train people to ignore the report.
    assert_eq!(outcome.relevant.get("/data/guardian"), Some(&false));
    assert!(
        !outcome.missing.iter().any(|p| p == "/data/guardian"),
        "{outcome:?}"
    );

    // and the moment the answer makes it relevant, it is missing
    session.set("/data/age", "10").unwrap();
    let outcome = session.recompute();
    assert_eq!(outcome.relevant.get("/data/guardian"), Some(&true));
    assert!(
        outcome.missing.iter().any(|p| p == "/data/guardian"),
        "{outcome:?}"
    );
}

#[test]
fn a_rejected_answer_carries_the_forms_own_message() {
    let mut session = Session::new(FORM, clock()).unwrap();
    session.set("/data/age", "200").unwrap();
    let outcome = session.recompute();
    assert_eq!(
        outcome.invalid,
        vec![(
            "/data/age".to_string(),
            "idade fora do intervalo".to_string()
        )]
    );
}

#[test]
fn what_it_would_send_is_xml_the_server_accepts() {
    let mut session = Session::new(FORM, clock()).unwrap();
    session.set("/data/age", "9").unwrap();
    session
        .set("/data/guardian", "Maria & João <bebê>")
        .unwrap();
    session.set("/data/meta/instanceID", "uuid:abc").unwrap();
    session.recompute();

    let xml = session.instance_xml();
    assert!(
        xml.starts_with("<?xml version='1.0' ?><data id=\"visit\">"),
        "{xml}"
    );
    // the answer is escaped, so an ampersand in a name does not end the form
    assert!(
        xml.contains("<guardian>Maria &amp; João &lt;bebê&gt;</guardian>"),
        "{xml}"
    );
    assert!(xml.contains("<instanceID>uuid:abc</instanceID>"), "{xml}");
    assert!(xml.contains("<adult>no</adult>"), "{xml}");
    // and it parses back into the same answers
    let resumed = Session::resume(FORM, &xml, clock()).unwrap();
    assert_eq!(resumed.get("/data/guardian"), "Maria & João <bebê>");
}

#[test]
fn a_form_can_be_put_down_and_picked_up() {
    let mut session = Session::new(FORM, clock()).unwrap();
    session.set("/data/age", "9").unwrap();
    session.recompute();
    let saved = session.instance_xml();

    let mut resumed = Session::resume(FORM, &saved, clock()).unwrap();
    assert_eq!(resumed.get("/data/years_left"), "9");
    let outcome = resumed.recompute();
    // nothing recalculates on resume: the saved answers already agree with
    // the form, which is what makes an interrupted interview trustworthy
    assert!(outcome.calculated.is_empty(), "{:?}", outcome.calculated);
    assert_eq!(outcome.relevant.get("/data/guardian"), Some(&true));
}
