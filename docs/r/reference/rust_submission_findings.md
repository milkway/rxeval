# What a form's own rules say about a submission.

What a form's own rules say about a submission.

## Usage

``` r
rust_submission_findings(form, submission, today, now)
```

## Arguments

- form:

  Path to an XForm, or the XML itself.

- submission:

  Path to a submission, or its XML.

- today, now:

  What `today()` and `now()` should answer. Defaults to the submission's
  own metadata, so a date rule is judged against the day the work was
  done rather than the day of the check.

## Value

A data frame: `path`, `kind`, `says`. Zero rows means the submission
satisfies every rule the engine could evaluate — and a rule it could not
evaluate appears as kind `not-evaluated`, never as a pass.
