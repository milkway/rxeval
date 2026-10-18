# cran-comments

## New submission

rxeval evaluates the logic embedded in ODK/OpenRosa XForms — the
`constraint`, `required`, `relevant` and `calculate` rules that survey
questionnaires carry — against collected submissions, from R. The engine is
the rxeval Rust crate (same author), compiled into the package.

## Rust

The package follows CRAN's "Using Rust" guidance:

* `SystemRequirements: Cargo (Rust's package manager), rustc >= 1.65.0, xz`.
* All crate dependencies are vendored inside the tarball
  (`src/rust/vendor.tar.xz`); the build is fully offline (`cargo build
  --offline`) and never reaches the network.
* Compilation is limited to two jobs (`-j 2`), and `CARGO_HOME` is set to a
  directory inside the build tree, so nothing is written to user
  directories; cargo and rustc versions are reported during configure.
* The authors and licenses of the vendored crates are acknowledged in
  `Authors@R` (role "cph") and listed per crate in `inst/AUTHORS`.

## Test environments

* local macOS 15 (aarch64), R 4.6.0, rustc 1.97.0

## R CMD check results

0 errors | 0 warnings | 2 notes

* "New submission" — this is the package's first submission.
* "Skipping checking HTML validation: 'tidy' doesn't look like recent
  enough HTML Tidy" — an artefact of the local machine's HTML Tidy, not of
  the package.
