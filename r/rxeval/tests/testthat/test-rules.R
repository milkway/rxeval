form_xml <- '<h:html xmlns="http://www.w3.org/2002/xforms"
                     xmlns:h="http://www.w3.org/1999/xhtml"
                     xmlns:jr="http://openrosa.org/javarosa">
  <h:head><model>
    <instance><data id="t"><idade/><dobro/><nome/></data></instance>
    <bind nodeset="/data/idade" type="int" constraint=". &lt;= 120"
          jr:constraintMsg="idade impossivel"/>
    <bind nodeset="/data/dobro" type="int" calculate="/data/idade * 2"/>
    <bind nodeset="/data/nome" type="string" required="true()"
          relevant="/data/idade &gt;= 18"/>
  </model></h:head><h:body/></h:html>'

submission <- function(idade, dobro, nome = "") {
  sprintf('<data id="t"><idade>%s</idade><dobro>%s</dobro><nome>%s</nome>
           <meta><instanceID>uuid:x</instanceID></meta></data>', idade, dobro, nome)
}

test_that("a submission that satisfies every rule reports nothing", {
  findings <- submission_findings(form_xml, submission(30, 60, "Maria"))
  expect_equal(nrow(findings), 0)
})

test_that("a constraint is reported in the form author's own words", {
  findings <- submission_findings(form_xml, submission(200, 400, "Maria"))
  expect_true("constraint" %in% findings$kind)
  expect_match(paste(findings$says, collapse = " "), "idade impossivel")
})

test_that("a derived value is recomputed and disagreement is reported", {
  findings <- submission_findings(form_xml, submission(30, 999, "Maria"))
  calc <- findings[findings$kind == "calculation", ]
  expect_equal(nrow(calc), 1)
  expect_match(calc$says, "999")
  expect_match(calc$says, "60")
})

test_that("an unasked question is not reported as unanswered", {
  # a minor: the name is not relevant, so its absence is correct
  expect_equal(nrow(submission_findings(form_xml, submission(7, 14))), 0)
  # an adult: now the blank name is a real omission
  findings <- submission_findings(form_xml, submission(30, 60))
  expect_equal(findings$kind, "required")
})

test_that("expressions evaluate against a submission", {
  data <- submission(30, 60, "Maria")
  expect_equal(eval_expression("/data/idade", data), "30")
  expect_equal(eval_expression("/data/idade >= 18", data), "true")
  expect_equal(eval_expression("concat(/data/nome, '!')", data), "Maria!")
})

test_that("what the engine cannot evaluate raises", {
  expect_error(
    eval_expression("pulldata('a','b','c','d')", submission(30, 60)),
    regexp = "not implemented"
  )
})
