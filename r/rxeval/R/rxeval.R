#' Will this form mean the same thing on a tablet and in a browser?
#'
#' The two engines the ODK ecosystem runs on implement different languages:
#' JavaRosa inside Collect and KoboCollect, Enketo inside web forms.
#' Neither is a superset of the other, and the gaps do not announce
#' themselves — an expression Collect cannot evaluate usually yields
#' nothing rather than an error, so the form fills in, the interview
#' finishes, and a column comes back empty.
#'
#' Every rule behind this was found by putting the same expression to both
#' engines and reading the two answers.
#'
#' @param form Path to an XForm, or the XML itself.
#' @return A data frame with one row per issue. `breaks` says which side
#'   stumbles: `"Collect / KoboCollect"`, `"Enketo web forms"`, or
#'   `"both, differently"` — the last being the worst, since nothing fails
#'   anywhere. `suggestion` is a rewrite that means one thing in both, or
#'   `NA` when there is none to give. Zero rows means the form travels.
#' @examples
#' \dontrun{
#' issues <- form_portability("survey.xml")
#' issues[issues$breaks == "both, differently", c("path", "says")]
#' }
#' @export
form_portability <- function(form) {
  stopifnot(is.character(form), length(form) == 1L)
  rust_form_portability(form)
}

#' What a form's own rules say about a submission
#'
#' Evaluates the questionnaire's `relevant`, `constraint`, `required` and
#' `calculate` against collected data, and reports what does not hold.
#'
#' A rule the engine could not evaluate comes back as `not-evaluated`, never
#' as a pass: a rule that did not run has not been satisfied, and reporting
#' it as clean would be the one failure worth avoiding.
#'
#' @param form Path to an XForm, or the XML itself.
#' @param submission Path to a submission, or its XML.
#' @param today,now What `today()` and `now()` should answer. Left `NULL`,
#'   they come from the submission's own metadata, so a date rule is judged
#'   against the day the work was done rather than the day of the check —
#'   otherwise a submission that was valid on collection starts failing
#'   later, and the report changes while the data does not.
#' @return A data frame of `path`, `kind` (`constraint`, `required`,
#'   `calculation`, `not-evaluated`) and `says`. Zero rows means every rule
#'   held.
#' @examples
#' \dontrun{
#' findings <- submission_findings("survey.xml", "submission.xml")
#' table(findings$kind)
#' }
#' @export
submission_findings <- function(form, submission, today = NULL, now = NULL) {
  stopifnot(is.character(form), is.character(submission))
  rust_submission_findings(form, submission, today, now)
}

#' Evaluate one XPath expression against a submission
#'
#' For working out what a rule does before writing it into a form.
#'
#' @param expression The XPath expression, in the ODK/OpenRosa dialect.
#' @param submission Path to a submission, or its XML.
#' @param today,now What `today()` and `now()` should answer.
#' @return A length-one character vector. An expression this engine cannot
#'   evaluate raises an error rather than returning a plausible value.
#' @examples
#' \dontrun{
#' eval_expression("count(/data/resident)", "submission.xml")
#' }
#' @export
eval_expression <- function(expression, submission, today = NULL, now = NULL) {
  stopifnot(is.character(expression), length(expression) == 1L)
  rust_eval_expression(expression, submission, today, now)
}
