# Which expressions in a form will behave differently on a tablet than in a browser.

Which expressions in a form will behave differently on a tablet than in
a browser.

## Usage

``` r
rust_form_portability(form)
```

## Arguments

- form:

  Path to an XForm, or the XML itself.

## Value

A data frame with one row per issue: `path`, `rule`, `expression`,
`construct`, `breaks`, `effect`, `suggestion`, `says`. Zero rows means
the form travels.
