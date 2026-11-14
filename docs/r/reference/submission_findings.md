# What a form's own rules say about a submission

Evaluates the questionnaire's `relevant`, `constraint`, `required` and
`calculate` against collected data, and reports what does not hold.

## Usage

``` r
submission_findings(form, submission, today = NULL, now = NULL)
```

## Arguments

- form:

  Path to an XForm, or the XML itself.

- submission:

  Path to a submission, or its XML.

- today, now:

  What `today()` and `now()` should answer. Left `NULL`, they come from
  the submission's own metadata, so a date rule is judged against the
  day the work was done rather than the day of the check — otherwise a
  submission that was valid on collection starts failing later, and the
  report changes while the data does not.

## Value

A data frame of `path`, `kind` (`constraint`, `required`, `calculation`,
`not-evaluated`) and `says`. Zero rows means every rule held.

## Details

A rule the engine could not evaluate comes back as `not-evaluated`,
never as a pass: a rule that did not run has not been satisfied, and
reporting it as clean would be the one failure worth avoiding.

## Examples

``` r
form <- system.file("extdata", "visit.xml", package = "rxeval")
submission <- system.file("extdata", "visit-submission.xml", package = "rxeval")
findings <- submission_findings(form, submission)
findings[, c("path", "kind")]
#>                     path        kind
#> 1 /data/youngest_entered calculation
#> 2        /data/residents  constraint
#> 3       /data/respondent    required
```
