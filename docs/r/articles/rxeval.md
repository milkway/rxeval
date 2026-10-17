# From a questionnaire to its findings

A survey’s quality rules live inside the form — `constraint`,
`required`, `calculate` — and they are written once, in the spreadsheet,
by the person who knows what a plausible answer looks like. The problem
is that nothing in an analysis script runs them: the data frame you read
back is whatever the devices sent, and the rules that were supposed to
guard it are XML the script never opens.

rxeval runs them. This walkthrough uses the small household form shipped
with the package, and a submission with problems planted on purpose.

\
[`library`](https://rdrr.io/r/base/library.html)`(`[`rxeval`](https://milkway.github.io/rxeval/)`)`\
[`library`](https://rdrr.io/r/base/library.html)`(`[`dplyr`](https://dplyr.tidyverse.org)`)`\
\
`form``       ``<-`` `[`system.file`](https://rdrr.io/r/base/system.file.html)`(``"extdata"``, ``"visit.xml"``, package ``=`` ``"rxeval"``)`\
`submission`` ``<-`` `[`system.file`](https://rdrr.io/r/base/system.file.html)`(``"extdata"``, ``"visit-submission.xml"``, package ``=`` ``"rxeval"``)`

## Before the field: will the form travel?

ODK forms are evaluated by two implementations — JavaRosa inside Collect
and KoboCollect, Enketo inside web forms — and neither is a superset of
the other. The gaps do not announce themselves: an expression Collect
cannot evaluate usually yields nothing rather than an error, so the form
fills in, the interview finishes, and a column comes back empty.

[`form_portability()`](https://milkway.github.io/rxeval/r/reference/form_portability.md)
reads the form the way both engines would:

\
[`form_portability`](https://milkway.github.io/rxeval/r/reference/form_portability.md)`(``form``)`` ``|>`\
`  `[`as_tibble`](https://tibble.tidyverse.org/reference/as_tibble.html)`(``)`\
`#> ``# A tibble: 1 × 8`\
`#>   path                 rule  expression construct breaks effect suggestion says `\
`#>   ``<chr>``                ``<chr>`` ``<chr>``      ``<chr>``     ``<chr>``  ``<chr>``  ``<chr>``      ``<chr>`\
`#> ``1`` /data/youngest_ente… calc… /data/res… last()    Colle… JavaR… count() o… /dat…`

The verdict worth filtering for is `breaks`. `"both, differently"` is
the worst value it takes — nothing fails anywhere, and the two engines
quietly compute different answers. Here the form has a milder, common
case: a `calculate` built on
[`last()`](https://dplyr.tidyverse.org/reference/nth.html), which Enketo
evaluates and JavaRosa does not have at all.

\
[`form_portability`](https://milkway.github.io/rxeval/r/reference/form_portability.md)`(``form``)`` ``|>`\
`  `[`filter`](https://dplyr.tidyverse.org/reference/filter.html)`(``breaks`` ``==`` ``"Collect / KoboCollect"``)`` ``|>`\
`  `[`select`](https://dplyr.tidyverse.org/reference/select.html)`(``path``, ``construct``, ``suggestion``)`\
`#>                     path construct            suggestion`\
`#> 1 /data/youngest_entered    last() count() of the repeat`

Every issue carries a `suggestion` when there is one to give: a rewrite
that means one thing in both engines. Zero rows means the form travels.

## After the field: what do the form’s own rules say?

[`submission_findings()`](https://milkway.github.io/rxeval/r/reference/submission_findings.md)
evaluates the questionnaire’s rules against collected data and reports
what does not hold:

\
`findings`` ``<-`` `[`submission_findings`](https://milkway.github.io/rxeval/r/reference/submission_findings.md)`(``form``, ``submission``)`\
\
`findings`` ``|>`\
`  `[`as_tibble`](https://tibble.tidyverse.org/reference/as_tibble.html)`(``)`\
`#> ``# A tibble: 3 × 3`\
`#>   path                   kind        says                                       `\
`#>   ``<chr>``                  ``<chr>``       ``<chr>``                                      `\
`#> ``1`` /data/youngest_entered calculation ``"``/data/youngest_entered: holds \"\", but t…`\
`#> ``2`` /data/residents        constraint  ``"``/data/residents: More than 20 residents n…`\
`#> ``3`` /data/respondent       required    ``"``/data/respondent: required, and unanswere…`

Three findings, three different kinds of trouble:

\
`findings`` ``|>`\
`  `[`count`](https://dplyr.tidyverse.org/reference/count.html)`(``kind``)`\
`#>          kind n`\
`#> 1 calculation 1`\
`#> 2  constraint 1`\
`#> 3    required 1`

- a **constraint** the device should have enforced and, on this
  submission, did not — 25 residents against a rule of at most 20;
- a **required** question that came back unanswered;
- a **calculation** whose stored value is not what the form computes —
  the [`last()`](https://dplyr.tidyverse.org/reference/nth.html) issue
  found above, no longer a prediction but a hole in a column.

A rule the engine could not evaluate comes back as
`kind = "not-evaluated"`, never as a pass: a rule that did not run has
not been satisfied, and reporting it as clean would be the one failure
worth avoiding.

Over a whole round, the same call is one `lapply` away:

\
[`list.files`](https://rdrr.io/r/base/list.files.html)`(``"submissions"``, full.names ``=`` ``TRUE``)`` ``|>`\
`  `[`lapply`](https://rdrr.io/r/base/lapply.html)`(``\``(``s``)`` `[`submission_findings`](https://milkway.github.io/rxeval/r/reference/submission_findings.md)`(``form``, ``s``)`` ``|>`` `[`mutate`](https://dplyr.tidyverse.org/reference/mutate.html)`(``file ``=`` `[`basename`](https://rdrr.io/r/base/basename.html)`(``s``)``)``)`` ``|>`\
`  `[`bind_rows`](https://dplyr.tidyverse.org/reference/bind_rows.html)`(``)`` ``|>`\
`  `[`count`](https://dplyr.tidyverse.org/reference/count.html)`(``path``, ``kind``, sort ``=`` ``TRUE``)`

## Trying a rule by hand

[`eval_expression()`](https://milkway.github.io/rxeval/r/reference/eval_expression.md)
answers what one expression does before it goes into the spreadsheet —
against a real submission, in the same dialect the devices speak:

\
[`eval_expression`](https://milkway.github.io/rxeval/r/reference/eval_expression.md)`(``"count(/data/resident[age < 18])"``, ``submission``)`\
`#> [1] "1"`\
\
[`eval_expression`](https://milkway.github.io/rxeval/r/reference/eval_expression.md)`(``"if(/data/residents > 20, 'needs a note', 'fine')"``, ``submission``)`\
`#> [1] "needs a note"`

An expression the engine cannot evaluate raises an error rather than
returning a plausible value — the same rule as everywhere else in
rxeval: silence is never a verdict.

## Where the rules come from

Every portability rule in the engine was decided by putting the same
expression to JavaRosa and to Enketo’s evaluator and reading the two
answers — the method, and the receipts, are on the [project
site](https://milkway.github.io/rxeval/).
