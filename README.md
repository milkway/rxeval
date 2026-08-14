# rxeval

Evaluate ODK/OpenRosa XForms logic — `relevant`, `constraint`, `required`,
`calculate` — in Rust, with no dependencies.

A form is a small program. Until something runs it, a server can only take
a device's word for what it collected, and a form author can only find out
what their rules do by going to the field.

```rust
let form = rxeval::Form::parse(&xform_xml)?;
let submission = rxeval::Instance::from_xml(&submission_xml)?;

for violation in form.check(&submission, clock) {
    println!("{}", violation.describe());
    // /data/idade: idade deve estar entre 0 e 120
    // /data/total: holds "99", but the form calculates "5"
}
```

## Nothing guesses

An expression this crate cannot evaluate is an error that names itself,
never a default. A form engine that answers an unknown function with an
empty string, or a failed `relevant` with `false`, hides a question and
reports success — and the damage arrives in the data weeks later.

**Not implemented, and therefore refused by name:** `indexed-repeat`,
`current`, `pulldata`, `randomize`, `uuid`, `digest`, `date`, `date-time`,
`decimal-date-time`, `decimal-time`, `area`, `distance`, `checklist`,
`weighted-checklist`, `position-in-repeat`.

## Validated against both engines the ecosystem runs on

Not against a reading of the specification: against JavaRosa, inside ODK
Collect and KoboCollect, and Enketo's evaluator, inside every web form.
Their answers to 117 expressions are committed as fixtures, so the test
suite needs neither Java nor Node.

The finding was that **there is no single ecosystem language**. On 21 of
those 117 the two references disagree with each other, and neither is a
superset of the other:

| | JavaRosa | Enketo |
|---|---|---|
| `floor`, `ceiling`, `substring`, `last`, `//` | ✗ | ✓ |
| `enclosed-area`, `geofence`, `base64-decode`, `is-selected` | ✓ | ✗ |
| `resident[2]` | matches nothing | works |
| comparing a repeated field | refused | existential |
| `regex(., '[0-9]{11}')` | whole value | anywhere in it |

Where they part ways this crate follows the collecting engine when its
answer already shaped the data, and XPath elsewhere. Every choice is
listed with its reason in `tests/ecosystem_oracle_test.rs`, and the test
fails if a recorded divergence stops diverging — the note explaining it
would have become a lie.

## Telling an author what will not travel

Those differences do not announce themselves. An expression Collect cannot
evaluate usually yields nothing rather than an error: the form fills in,
the interview finishes, and a column comes back empty.

```rust
for issue in rxeval::check_form(&xform_xml)? {
    println!("{}", issue.describe());
}
// /data/cpf (constraint): regex() with the unanchored pattern "[0-9]{11}"
//   — JavaRosa requires the whole value to match while Enketo accepts a
//   match anywhere in it … Write '^(?:[0-9]{11})$' instead.
```

## Where it runs

On a server, checking what arrived. Compiled to WebAssembly, driving a web
form. Bound from R, exercising a questionnaire before anyone goes to the
field. Enketo's engine is browser-only JavaScript — it does not even
implement XPath, it wraps the browser's — and JavaRosa's is JVM-only.
Neither travels.

`regex()` needs an engine, so it sits behind a default feature: a
size-constrained build can drop it and get an honest refusal rather than a
wrong answer.

## Family

- [`rxform`](https://crates.io/crates/rxform) — spreadsheet → XForm
- `rxdata` — submission → typed records, exports
- `rxeval` — this one: the rules

## License

BSD-2-Clause.
