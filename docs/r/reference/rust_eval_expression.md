# Evaluate one XPath expression against a submission.

For working out what a rule actually does before writing it into a form.

## Usage

``` r
rust_eval_expression(expression, submission, today, now)
```

## Arguments

- expression:

  The XPath expression.

- submission:

  Path to a submission, or its XML.

- today, now:

  What `today()` and `now()` should answer.

## Value

A length-one character vector. An expression the engine cannot evaluate
raises an error rather than returning a plausible value.
