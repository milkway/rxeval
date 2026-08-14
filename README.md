# rxeval

[![crates.io](https://img.shields.io/crates/v/rxeval.svg)](https://crates.io/crates/rxeval)
[![docs.rs](https://img.shields.io/docsrs/rxeval)](https://docs.rs/rxeval)
[![license: BSD-2-Clause](https://img.shields.io/badge/license-BSD--2--Clause-blue.svg)](LICENSE)

Runs the logic inside an ODK/OpenRosa form — `relevant`, `constraint`,
`required`, `calculate` — in Rust, with no dependencies. On a server, in a
browser through WebAssembly, and in R.

A form is a small program. Until something runs it, a server can only take
whatever a device sends and hope the device was right. rxeval is that
something: it parses XPath 1.0 with the OpenRosa extensions, evaluates it
over a form instance, and answers what the form says.

It is a companion to [rxform](https://github.com/milkway/rxform), which turns
an [XLSForm](https://xlsform.org) spreadsheet into the
[XForm](https://getodk.github.io/xforms-spec/) this crate then runs.

## Two engines, one ecosystem, no single language

ODK forms are evaluated by two implementations: **JavaRosa**, inside ODK
Collect and KoboCollect, and **Enketo**'s `openrosa-xpath-evaluator`, inside
web forms. Neither is a superset of the other, and the gaps do not announce
themselves — an expression Collect cannot evaluate usually yields nothing
rather than an error, so the form fills in, the interview finishes, and a
column comes back empty.

Every rule in this crate was decided by putting the same expression to both
engines and reading the two answers. Of 126 expressions in the corpus, the
two references agree on 99 and **disagree with each other on 26**. Each
disagreement is recorded in the test suite with the side rxeval follows and
why:

| expression | JavaRosa | Enketo |
| --- | --- | --- |
| `resident[2]/name` | nothing at all | the second one |
| `last()`, `floor()`, `ceiling()`, `substring()` | absent | present |
| `//name` | no `//` axis | descendant search |
| `regex("a12345678901b", "[0-9]{11}")` | anchored → false | unanchored → true |
| `round(-1.5)` | −1 (half up) | −2 (away from zero) |
| `boolean-from-string("TRUE")` | true | false |
| `distance()`, `area()` | full precision | rounded to 2 decimals |

The test that produces this is `tests/ecosystem_oracle_test.rs`; regenerate
the reference answers with `scripts/openrosa-oracle.mjs` and
`scripts/JavarosaOracle.java`.

## Portability checking

Because the two languages differ, a form can be correct in one place and
quietly wrong in the other. rxeval reads a form and says so before anyone
collects with it:

```rust
for issue in rxeval::check_form(&xform)? {
    println!("{}", issue.describe());
}
```

```
/data/resident[2]/name (relevant): a bare positional predicate —
  JavaRosa returns nothing for [n]; use [position() = n] on Collect /
  KoboCollect. Write [position() = 2] instead.
/data/total (calculate): /p/morador/maior, which this form's instance has
  no node for — the path matches nothing, so the rule reads an empty
  node-set: a calculation comes out 0, a comparison comes out false, and a
  relevant hides its question for the whole of fieldwork, identically on
  both engines. Write /data/morador/maior instead.
```

That last one is not a portability problem at all: it travels perfectly and
is wrong everywhere it goes. It is reported here because it is found the
same way and matters more.

## Filling a form, not only judging one

`Rules` answers questions about a finished submission. `Session` is the other
direction — a form being typed into, where the same questions have to be
answered again after every keystroke and the answers applied rather than
reported:

```rust
let mut session = rxeval::Session::new(&xform, clock)?;
session.set("/data/age", "9")?;

let outcome = session.recompute();
outcome.calculated;   // paths whose value the form derived, and what it derived
outcome.relevant;     // which questions are asked, by path
outcome.missing;      // required and unanswered, given current relevance
outcome.invalid;      // rejected, with the form's own message
outcome.repeats;      // how many rows each repeat has

session.add_row("/data/resident")?;
session.instance_xml();   // what would be submitted
```

Calculations run in dependency order, so one feeding another settles in a
single pass. Only paths whose value actually changed are reported — a
renderer redrawing every calculated field on every keystroke fights the
cursor.

## The same engine in three places

![rxeval architecture](https://raw.githubusercontent.com/milkway/rxeval/main/docs/architecture.svg)

One implementation, compiled three ways. A web form that asked a server what
its own rules mean needs a connection for every keystroke, which rules out
the place survey work happens — a bus stop, a doorway, a basement. Compiling
the same Rust to WebAssembly removes the network from the interview without
introducing a second implementation to drift from the first.

- **Native**, for a server checking what arrived.
- **WebAssembly**, for a browser filling a form offline.
- **R**, through [extendr](https://extendr.github.io/), for checking a
  questionnaire from an analysis script before anyone goes to the field.

## Install

```toml
[dependencies]
rxeval = "0.1"
```

Without the regex engine — smaller, for WebAssembly builds that do not check
pattern constraints:

```toml
rxeval = { version = "0.1", default-features = false }
```

A build without `regex` refuses `regex()` rather than guessing at it: a rule
that did not run is not a rule that passed.

## What it evaluates

XPath 1.0 — paths, predicates, axes, the node-set/string/number/boolean
conversions and their comparison rules — plus the OpenRosa function library:

`selected`, `selected-at`, `count-selected`, `jr:choice-name`, `once`,
`coalesce`, `if`, `int`, `round`, `pow`, `log`, `exp`, `abs`, `min`, `max`,
`sum`, `count`, `count-non-empty`, `position`, `boolean-from-string`,
`regex`, `substr`, `string-length`, `concat`, `join`, `translate`,
`normalize-space`, `contains`, `starts-with`, `ends-with`, `date`,
`format-date`, `format-date-time`, `decimal-date-time`, `today`, `now`,
`instance`, `pulldata`, `area`, `distance`, and the rest of the core library.

Deliberately refused, rather than guessed at: `indexed-repeat`, `current`,
`randomize`, `uuid`, `digest`, `checklist`, `position-in-repeat`. Each one
errors by name.

**Geography is measured, not read.** `area()` is not the spherical-excess
formula a textbook suggests: JavaRosa projects the points onto a plane and
takes the shoelace of that, with the earth a sphere of radius 6 378 100 m —
not WGS84's 6 378 137 — and `distance()` is the spherical law of cosines
rather than the haversine. The constants came from reading `GeoUtils` with
`javap -c` after four variants of the textbook formula were wrong in the
ninth decimal.

## Citing

See [`CITATION.cff`](CITATION.cff).

## Licence

BSD-2-Clause. See [`LICENSE`](LICENSE).
