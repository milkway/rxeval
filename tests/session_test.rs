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

/// A constraint message may be a translation key rather than a sentence.
///
/// Handing back `jr:itext('/data/age:jr:constraintMsg')` puts the form's own
/// plumbing in front of the person the message was written for — which is
/// exactly what a real form did on screen before this was fixed.
#[test]
fn a_translated_message_arrives_translated() {
    const TRANSLATED: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml" xmlns:jr="http://openrosa.org/javarosa">
      <h:head>
        <model>
          <itext>
            <translation lang="Português (pt)" default="">
              <text id="/data/age:jr:constraintMsg">
                <value>Idade fora do intervalo aceito.</value>
              </text>
            </translation>
          </itext>
          <instance><data id="t"><age/></data></instance>
          <bind nodeset="/data/age" type="int" constraint=". &lt; 130"
                jr:constraintMsg="jr:itext('/data/age:jr:constraintMsg')"/>
        </model>
      </h:head>
      <h:body/>
    </h:html>"#;

    let mut session = Session::new(TRANSLATED, clock()).unwrap();
    session.set("/data/age", "200").unwrap();
    let outcome = session.recompute();
    assert_eq!(
        outcome.invalid,
        vec![(
            "/data/age".to_string(),
            "Idade fora do intervalo aceito.".to_string()
        )]
    );
}

// ---------------------------------------------------------------------------
// Repeats
// ---------------------------------------------------------------------------

const HOUSEHOLD: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml" xmlns:jr="http://openrosa.org/javarosa">
  <h:head>
    <model>
      <instance>
        <data id="household">
          <chefe/>
          <morador jr:template="">
            <nome/>
            <idade/>
            <maior/>
            <trabalha/>
          </morador>
          <morador>
            <nome/>
            <idade/>
            <maior/>
            <trabalha/>
          </morador>
          <total_adultos/>
          <meta><instanceID/></meta>
        </data>
      </instance>
      <bind nodeset="/data/morador/nome" type="string" required="true()"/>
      <bind nodeset="/data/morador/idade" type="int" required="true()"/>
      <bind nodeset="/data/morador/maior" type="int" calculate="if(../idade &gt;= 18, 1, 0)"/>
      <bind nodeset="/data/morador/trabalha" type="string" relevant="../idade &gt;= 14"/>
      <bind nodeset="/data/total_adultos" type="int" calculate="sum(/data/morador/maior)"/>
    </model>
  </h:head>
  <h:body><repeat nodeset="/data/morador"/></h:body>
</h:html>"#;

/// The template is the form's blueprint, not somebody's answer.
///
/// Left in the instance it counts as a row, validates as a row, and is
/// submitted as a row of blanks — an extra resident in every household.
#[test]
fn the_blueprint_row_is_not_a_row() {
    let session = Session::new(HOUSEHOLD, clock()).unwrap();
    assert_eq!(session.repeat_counts().get("/data/morador"), Some(&1));
    let xml = session.instance_xml();
    assert!(!xml.contains("template"), "{xml}");
}

/// Each row calculates for itself. Before positional paths existed, all
/// rows shared one path, so one row's answer decided every row's result.
#[test]
fn each_row_is_its_own() {
    let mut session = Session::new(HOUSEHOLD, clock()).unwrap();
    session.add_row("/data/morador").unwrap();
    assert_eq!(session.add_row("/data/morador").unwrap(), 3);

    session.set("/data/morador[1]/idade", "40").unwrap();
    session.set("/data/morador[2]/idade", "9").unwrap();
    session.set("/data/morador[3]/idade", "22").unwrap();
    let outcome = session.recompute();

    assert_eq!(session.get("/data/morador[1]/maior"), "1");
    assert_eq!(session.get("/data/morador[2]/maior"), "0");
    assert_eq!(session.get("/data/morador[3]/maior"), "1");
    // and the total reads all three
    assert_eq!(session.get("/data/total_adultos"), "2");

    // relevance is per row too: a nine-year-old is not asked about work
    assert_eq!(
        outcome.relevant.get("/data/morador[2]/trabalha"),
        Some(&false)
    );
    assert_eq!(
        outcome.relevant.get("/data/morador[3]/trabalha"),
        Some(&true)
    );

    // and so is a missing answer: three rows, three unanswered names
    let missing: Vec<&String> = outcome
        .missing
        .iter()
        .filter(|p| p.ends_with("/nome"))
        .collect();
    assert_eq!(missing.len(), 3, "{:?}", outcome.missing);
}

/// A new row lands among its own kind, not at the end of the document.
#[test]
fn a_new_row_goes_where_rows_go() {
    let mut session = Session::new(HOUSEHOLD, clock()).unwrap();
    session.add_row("/data/morador").unwrap();
    session.set("/data/chefe", "Ana").unwrap();
    session.set("/data/morador[2]/nome", "Bia").unwrap();

    let xml = session.instance_xml();
    let moradores = xml.find("<morador>").unwrap();
    let total = xml.find("<total_adultos").unwrap();
    let meta = xml.find("<meta>").unwrap();
    assert!(moradores < total && total < meta, "{xml}");
    assert_eq!(xml.matches("<morador>").count(), 2, "{xml}");
    // and it is a blank row, not a copy of the one before it
    assert!(xml.contains("<nome>Bia</nome>"), "{xml}");
}

/// Removing a row removes that row, and the ones after it move up — which
/// is what makes a positional path mean anything after an edit.
#[test]
fn removing_a_row_renumbers_the_rest() {
    let mut session = Session::new(HOUSEHOLD, clock()).unwrap();
    session.add_row("/data/morador").unwrap();
    session.add_row("/data/morador").unwrap();
    session.set("/data/morador[1]/nome", "Ana").unwrap();
    session.set("/data/morador[2]/nome", "Bia").unwrap();
    session.set("/data/morador[3]/nome", "Cid").unwrap();

    assert_eq!(session.remove_row("/data/morador", 2).unwrap(), 2);
    assert_eq!(session.get("/data/morador[1]/nome"), "Ana");
    assert_eq!(session.get("/data/morador[2]/nome"), "Cid");
    assert!(!session.instance_xml().contains("Bia"));

    assert!(session.remove_row("/data/morador", 9).is_err());
}

/// A form put down mid-interview comes back with the same rows.
#[test]
fn rows_survive_being_put_down() {
    let mut session = Session::new(HOUSEHOLD, clock()).unwrap();
    session.add_row("/data/morador").unwrap();
    session.set("/data/morador[1]/idade", "40").unwrap();
    session.set("/data/morador[2]/idade", "9").unwrap();
    session.recompute();
    let saved = session.instance_xml();

    let mut resumed = Session::resume(HOUSEHOLD, &saved, clock()).unwrap();
    assert_eq!(resumed.repeat_counts().get("/data/morador"), Some(&2));
    assert_eq!(resumed.get("/data/morador[2]/idade"), "9");
    // and it can still grow, which needs the template the saved instance
    // never carried
    assert_eq!(resumed.add_row("/data/morador").unwrap(), 3);
}

/// The verdict says how many rows there are, because the page that draws
/// them has no other way to know — and a count that is never reported is a
/// repeat that never appears.
#[test]
fn the_verdict_counts_the_rows() {
    let mut session = Session::new(HOUSEHOLD, clock()).unwrap();
    assert_eq!(session.recompute().repeats.get("/data/morador"), Some(&1));
    session.add_row("/data/morador").unwrap();
    assert_eq!(session.recompute().repeats.get("/data/morador"), Some(&2));
    session.remove_row("/data/morador", 1).unwrap();
    assert_eq!(session.recompute().repeats.get("/data/morador"), Some(&1));
}

// ---------------------------------------------------------------------------
// pulldata
// ---------------------------------------------------------------------------

const LOOKUP: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml">
  <h:head>
    <model>
      <instance>
        <data id="lookup">
          <codigo/>
          <nome_lote/>
          <empresa/>
          <ausente/>
          <meta><instanceID/></meta>
        </data>
      </instance>
      <instance id="lotes">
        <root>
          <item><codigo>AR-01</codigo><nome>Articulação Norte</nome><empresa>Viação A</empresa></item>
          <item><codigo>AR-02</codigo><nome>Articulação Sul</nome><empresa>Viação B</empresa></item>
        </root>
      </instance>
      <bind nodeset="/data/nome_lote" type="string"
            calculate="pulldata('lotes', 'nome', 'codigo', /data/codigo)"/>
      <bind nodeset="/data/empresa" type="string"
            calculate="pulldata('lotes', 'empresa', 'codigo', /data/codigo)"/>
      <bind nodeset="/data/ausente" type="string"
            calculate="pulldata('lotes', 'nome', 'codigo', 'ZZ-99')"/>
    </model>
  </h:head>
  <h:body/>
</h:html>"#;

/// `pulldata` reads a row of a lookup table by one of its columns.
///
/// Neither reference implements it: JavaRosa answers "cannot handle
/// function 'pulldata'" and Enketo's evaluator "Unknown function". It is
/// ODK Collect's own runtime handler, and pyxform — and rxform — leave the
/// call in the expression for it to find. So this is written from the
/// documented behaviour, which is worth knowing when reading it.
#[test]
fn pulldata_reads_a_row_by_a_column() {
    let mut session = Session::new(LOOKUP, clock()).unwrap();
    session.set("/data/codigo", "AR-02").unwrap();
    let outcome = session.recompute();

    assert_eq!(session.get("/data/nome_lote"), "Articulação Sul");
    assert_eq!(session.get("/data/empresa"), "Viação B");
    // A row that is not there is an empty answer and not a failure: a form
    // calls this on every recalculation, long before the answer it queries
    // by exists.
    assert_eq!(session.get("/data/ausente"), "");
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

    // and it follows the answer it queries by
    session.set("/data/codigo", "AR-01").unwrap();
    session.recompute();
    assert_eq!(session.get("/data/nome_lote"), "Articulação Norte");
}

/// A table the form never loaded is not the same as a row that is not
/// there, and a form author staring at a blank answer needs to know which.
#[test]
fn a_missing_table_says_so() {
    const MISSING: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml">
      <h:head><model>
        <instance><data id="m"><x/></data></instance>
        <bind nodeset="/data/x" type="string"
              calculate="pulldata('nunca_carregada', 'a', 'b', 'c')"/>
      </model></h:head><h:body/></h:html>"#;
    let mut session = Session::new(MISSING, clock()).unwrap();
    let outcome = session.recompute();
    let (path, why) = outcome.failed.first().expect("a reported failure");
    assert_eq!(path, "/data/x");
    assert!(why.contains("nunca_carregada"), "{why}");
    assert!(why.contains("nothing has loaded it"), "{why}");
}

// ---------------------------------------------------------------------------
// Geography
// ---------------------------------------------------------------------------

/// Distance and area, against numbers taken from JavaRosa 5.1.0 — whose
/// bytecode was read to get the constants right. The earth is a sphere of
/// radius 6_378_100 m, distance is the spherical law of cosines, and area
/// is a shoelace over a planar projection.
#[test]
fn geography_matches_the_engine_on_the_tablets() {
    const GEO: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml">
      <h:head><model>
        <instance><data id="g"><trecho/><quadra/><vazio/><metros/><m2/></data></instance>
        <bind nodeset="/data/metros" type="decimal" calculate="distance(/data/trecho)"/>
        <bind nodeset="/data/m2" type="decimal" calculate="area(/data/quadra)"/>
      </model></h:head><h:body/></h:html>"#;
    let mut session = Session::new(GEO, clock()).unwrap();
    session
        .set(
            "/data/trecho",
            "-23.5505 -46.6333 760 5;-23.5605 -46.6433 762 5;-23.5705 -46.6333 758 5",
        )
        .unwrap();
    session
        .set(
            "/data/quadra",
            "-23.5505 -46.6333 0 0;-23.5505 -46.6233 0 0;-23.5605 -46.6233 0 0;\
             -23.5605 -46.6333 0 0;-23.5505 -46.6333 0 0",
        )
        .unwrap();
    session.recompute();

    let metres: f64 = session.get("/data/metros").parse().unwrap();
    let square: f64 = session.get("/data/m2").parse().unwrap();
    assert!((metres - 3020.19014542737).abs() < 1e-9, "{metres}");
    assert!((square - 1135931.14588564).abs() < 1e-6, "{square}");

    // A shape that has not been captured yet is worth nothing, not a
    // failure: the form asks for its area on every recalculation.
    session.set("/data/quadra", "").unwrap();
    let outcome = session.recompute();
    assert_eq!(session.get("/data/m2"), "0");
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
}

/// A polygon closes itself, because every coordinate is measured from the
/// first point and the segment back to it contributes nothing. A form that
/// forgot to repeat its first point gets the same answer as one that did.
#[test]
fn a_shape_closes_itself() {
    const GEO: &str = r#"<h:html xmlns="http://www.w3.org/2002/xforms" xmlns:h="http://www.w3.org/1999/xhtml">
      <h:head><model>
        <instance><data id="g"><aberto/><fechado/><a/><f/></data></instance>
        <bind nodeset="/data/a" type="decimal" calculate="area(/data/aberto)"/>
        <bind nodeset="/data/f" type="decimal" calculate="area(/data/fechado)"/>
      </model></h:head><h:body/></h:html>"#;
    let mut session = Session::new(GEO, clock()).unwrap();
    let aberto = "-23.55 -46.63 0 0;-23.55 -46.62 0 0;-23.56 -46.62 0 0;-23.56 -46.63 0 0";
    session.set("/data/aberto", aberto).unwrap();
    session
        .set("/data/fechado", &format!("{aberto};-23.55 -46.63 0 0"))
        .unwrap();
    session.recompute();
    assert_eq!(session.get("/data/a"), session.get("/data/f"));
    assert!(session.get("/data/a").parse::<f64>().unwrap() > 1_000_000.0);
}

/// A positional path names a row that exists.
///
/// Answering `/data/morador[3]/idade` when there are two rows used to
/// create an element called `morador[3]` — not a name, and a submission no
/// parser will accept. Found from Python, on the first call that reached
/// past the end.
#[test]
fn a_row_that_is_not_there_is_an_error_and_not_an_invention() {
    let mut session = Session::new(HOUSEHOLD, clock()).unwrap();
    assert_eq!(session.repeat_counts().get("/data/morador"), Some(&1));

    let refused = session.set("/data/morador[3]/idade", "22").unwrap_err();
    assert!(refused.contains("morador[3]"), "{refused}");
    assert!(refused.contains("add_row"), "{refused}");
    assert!(
        !session.instance_xml().contains("morador["),
        "{}",
        session.instance_xml()
    );

    // and with the rows there, it lands where it was asked to
    session.add_row("/data/morador").unwrap();
    session.add_row("/data/morador").unwrap();
    session.set("/data/morador[3]/idade", "22").unwrap();
    assert_eq!(session.get("/data/morador[3]/idade"), "22");
}
