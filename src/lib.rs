//! rxeval — evaluate ODK/OpenRosa XForms logic.
//!
//! The third piece beside [`rxform`](https://crates.io/crates/rxform)
//! (spreadsheet → XForm) and `rxdata` (submission → typed data): the rules
//! a form carries — `relevant`, `constraint`, `required`, `calculate` — are
//! written in XPath, and until something evaluates them a server can only
//! take a device's word for what it collected.
//!
//! One engine, three places. On the server it checks what arrived. Compiled
//! to WebAssembly it drives a web form. Bound from R it lets a
//! questionnaire's logic be exercised before anyone goes to the field.
//! Enketo's engine is browser-only JavaScript and JavaRosa's is JVM-only;
//! neither travels.
//!
//! **What an expression can do.** These are XForms expressions, not a
//! scripting language: the grammar is XPath 1.0, the only inputs are the
//! instance tree and the environment handed in, and evaluation performs no
//! I/O, spawns nothing and cannot reach the host. A hostile expression can
//! waste time or return a wrong answer; it cannot escape. `eval_str` is
//! named for XPath evaluation, and shares nothing with the `eval` of
//! interpreted languages.
//!
//! **Nothing here guesses.** An expression this crate does not understand is
//! an error, never a default. A form engine that returns `false` for a
//! `relevant` it failed to parse hides a question and reports success, and
//! the damage shows up in the data weeks later.
pub mod eval;
pub mod functions;
pub mod parser;
pub mod portability;
pub mod rules;
pub mod session;
pub mod tree;

pub use eval::{evaluate, Context, Environment, Fixed, Value};
pub use parser::{parse, Expr};
pub use portability::{check_form, Breaks, Issue};
pub use rules::{Binding, Clock, Form, Rules, Violation, ViolationKind};
pub use session::{Outcome, Session};
pub use tree::{Instance, NodeId};

/// Parse and evaluate one expression against an instance, with `.` at the
/// root. The convenience path; a form engine keeps parsed expressions.
pub fn eval_str(
    expression: &str,
    instance: &Instance,
    env: &dyn Environment,
) -> Result<Value, String> {
    let expr = parse(expression)?;
    let root = instance
        .root()
        .ok_or_else(|| "the instance has no root element".to_string())?;
    evaluate(&expr, instance, Context::at(root), env)
}
