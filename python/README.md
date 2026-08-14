# rxeval

Run the logic inside an ODK/OpenRosa form — `relevant`, `constraint`,
`required`, `calculate` — from Python. The engine is Rust with no
dependencies; this is a binding to it.

A form is a small program. Until something runs it, a server can only take
whatever a device sends and hope the device was right.

Python is where ODK forms are already built — pyxform is Python — so the
check that matters most belongs in the script that builds the form, before
it is published, rather than after a week of fieldwork.

```sh
pip install rxeval
```

## Will this form travel?

The ODK ecosystem has two evaluation engines: **JavaRosa**, inside ODK
Collect and KoboCollect, and **Enketo**'s `openrosa-xpath-evaluator`, inside
web forms. Neither is a superset of the other, and the gaps do not announce
themselves — an expression Collect cannot evaluate usually yields nothing
rather than an error, so the form fills in, the interview finishes, and a
column comes back empty.

`check_form` reads a form and says which of its rules will behave
differently, before anyone collects with it:

```python
import rxeval

for issue in rxeval.check_form("survey.xml"):
    print(issue["says"])
```

```
/data/resident[2]/name (relevant): a bare positional predicate — JavaRosa
  returns nothing for [n]; use [position() = n] on Collect / KoboCollect.
  Write [position() = 2] instead.
/data/total (calculate): /p/morador/maior, which this form's instance has no
  node for — the path matches nothing, so the rule reads an empty node-set:
  a calculation comes out 0, a comparison comes out false, and a relevant
  hides its question for the whole of fieldwork, identically on both
  engines. Write /data/morador/maior instead.
```

Each issue is a dict: `path`, `rule`, `expression`, `construct`, `breaks`,
`effect`, `suggestion`, and `says` — the whole sentence, already written.
An empty list means the form travels.

Every rule behind this was decided by putting the same expression to both
reference engines and reading the two answers. Of 126 expressions in the
corpus the two agree on 99 and **disagree with each other on 26**, each
recorded with the side rxeval follows and why.

## What the rules say about a submission

```python
findings = rxeval.submission_findings("survey.xml", "submission.xml")
for f in findings:
    print(f["kind"], f["path"], f["says"])
```

`kind` is `constraint`, `required`, `calculation` or `not-evaluated`. A rule
the engine could not evaluate comes back as `not-evaluated`, never as a
pass: a rule that did not run has not been satisfied, and reporting it as
clean would be the one failure worth avoiding.

`today` and `now` default to the submission's own metadata, so a date rule
is judged against the day the work was done — otherwise a submission that
was valid on collection starts failing later, and the report changes while
the data does not.

## One expression, for working it out

```python
rxeval.eval_expression("count(/data/resident)", "submission.xml")   # '3'
```

An expression this engine cannot evaluate raises, rather than returning a
plausible value.

## Filling a form, not only judging one

```python
s = rxeval.Session("survey.xml", today="2026-08-14", now="2026-08-14T09:00:00Z")
s.set("/data/age", "9")

out = s.recompute()
out["calculated"]   # {'/data/adult': 'no', '/data/years_left': '9'}
out["relevant"]     # {'/data/guardian': True, ...}
out["missing"]      # ['/data/guardian']
out["invalid"]      # [] — rejected answers, with the form's own message

s.add_row("/data/resident")
s.instance_xml()    # what would be submitted
```

Calculations run in dependency order, so one feeding another settles in a
single pass, and only paths whose value actually changed are reported.

Every function takes either a path or the XML itself: a caller holding XML
should not have to write it to a file first, and a caller holding a path
should not have to read it.

## Links

- Documentation and the full story: <https://milkway.github.io/rxeval/>
- Source: <https://github.com/milkway/rxeval>
- The Rust crate: <https://crates.io/crates/rxeval>
- [rxform](https://github.com/milkway/rxform), which turns an XLSForm
  spreadsheet into the form this runs

## Licence

BSD-2-Clause.
