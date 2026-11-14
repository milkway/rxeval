# Evaluate one XPath expression against a submission

For working out what a rule does before writing it into a form.

## Usage

``` r
eval_expression(expression, submission, today = NULL, now = NULL)
```

## Arguments

- expression:

  The XPath expression, in the ODK/OpenRosa dialect.

- submission:

  Path to a submission, or its XML.

- today, now:

  What `today()` and `now()` should answer.

## Value

A length-one character vector. An expression this engine cannot evaluate
raises an error rather than returning a plausible value.

## Examples

``` r
submission <- system.file("extdata", "visit-submission.xml", package = "rxeval")
eval_expression("count(/data/resident)", submission)
#> [1] "2"
eval_expression("/data/residents > count(/data/resident)", submission)
#> [1] "true"
```
