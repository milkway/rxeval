form <- function(binds) {
  sprintf('<h:html xmlns="http://www.w3.org/2002/xforms"
                   xmlns:h="http://www.w3.org/1999/xhtml">
    <h:head><model>
      <instance><data id="t"><cpf/><a/><b/></data></instance>
      %s
    </model></h:head><h:body/></h:html>', binds)
}

test_that("a form that travels reports nothing", {
  travels <- form('<bind nodeset="/data/cpf" type="string"
                         constraint="regex(., \'^(?:[0-9]{11})$\')"/>')
  issues <- form_portability(travels)
  expect_s3_class(issues, "data.frame")
  expect_equal(nrow(issues), 0)
  # the columns exist even with no rows, so downstream code does not
  # have to special-case the happy path
  expect_true(all(c("path", "breaks", "suggestion", "says") %in% names(issues)))
})

test_that("an unanchored pattern is reported as meaning two things", {
  issues <- form_portability(
    form('<bind nodeset="/data/cpf" type="string" constraint="regex(., \'[0-9]{11}\')"/>')
  )
  expect_equal(nrow(issues), 1)
  expect_equal(issues$breaks, "both, differently")
  # and the rewrite groups the pattern, or it would change meaning the
  # moment the pattern grows an alternation
  expect_equal(issues$suggestion, "'^(?:[0-9]{11})$'")
})

test_that("a missing suggestion is NA, not an empty string", {
  issues <- form_portability(
    form('<bind nodeset="/data/a" type="string" calculate="enclosed-area(/data/b)"/>')
  )
  expect_equal(nrow(issues), 1)
  expect_equal(issues$breaks, "Enketo web forms")
  expect_true(is.na(issues$suggestion))
})

test_that("a form that cannot be read raises rather than returning nothing", {
  expect_error(form_portability("<h:html>"), regexp = "XForm|match|XML")
  expect_error(form_portability("/no/such/file.xml"), regexp = "cannot read")
})
