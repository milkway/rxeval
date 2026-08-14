//! rxeval, from Python.
//!
//! The same three questions the R package asks — will this form travel,
//! what do its rules say about this submission, what does this expression
//! mean — plus a `Session`, for filling a form rather than judging one.
//!
//! Python is where ODK forms are already written: pyxform is Python, and a
//! form is usually built by a script that then hands the XLSForm to a
//! server. The check that matters most belongs in that script, before the
//! form is published, not after a week of fieldwork.
//!
//! Paths and XML are both accepted everywhere a document is wanted. A
//! caller holding XML should not have to write it to a file first, and a
//! caller holding a path should not have to read it.

use std::path::Path;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// Read a document from whatever the caller had: a path, or the XML itself.
fn document(text: &str, what: &str) -> PyResult<String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('<') {
        return Ok(text.to_string());
    }
    std::fs::read_to_string(Path::new(text))
        .map_err(|e| PyValueError::new_err(format!("{what}: could not read '{text}': {e}")))
}

/// The clock the engine sees.
///
/// Left unset it comes from the submission's own metadata, so a date rule
/// is judged against the day the work was done rather than the day of the
/// check — otherwise a submission that was valid on collection starts
/// failing later, and the report changes while the data does not.
fn clock(instance: &engine::Instance, today: Option<&str>, now: Option<&str>) -> engine::Clock {
    let from_instance = |name: &str| -> Option<String> {
        let root = instance.root()?;
        instance
            .descendants(root)
            .into_iter()
            .find(|node| instance.node(*node).name == name)
            .map(|node| instance.string_value(node))
            .filter(|value| !value.trim().is_empty())
    };
    engine::Clock {
        today: today
            .map(String::from)
            .or_else(|| from_instance("today"))
            .or_else(|| from_instance("end").map(|end| end[..10.min(end.len())].to_string()))
            .unwrap_or_else(|| "1970-01-01".to_string()),
        now: now
            .map(String::from)
            .or_else(|| from_instance("end"))
            .or_else(|| from_instance("start"))
            .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string()),
    }
}

/// Will this form mean the same thing on a tablet and in a browser?
///
/// Returns one dict per finding: the bind it belongs to, which rule, the
/// construct at fault, where it breaks, what happens, and what to write
/// instead. An empty list means the form travels.
#[pyfunction]
#[pyo3(signature = (form))]
fn check_form(py: Python<'_>, form: &str) -> PyResult<Py<PyList>> {
    let xml = document(form, "form")?;
    let issues = engine::check_form(&xml).map_err(PyRuntimeError::new_err)?;

    let out = PyList::empty(py);
    for issue in issues {
        let row = PyDict::new(py);
        // The whole sentence first: `describe` reads every field, and
        // handing the fields over one at a time would move them out from
        // under it.
        row.set_item("says", issue.describe())?;
        row.set_item("path", issue.path)?;
        row.set_item("rule", issue.rule)?;
        row.set_item("expression", issue.expression)?;
        row.set_item("construct", issue.construct)?;
        row.set_item("breaks", issue.breaks.describe())?;
        row.set_item("effect", issue.effect)?;
        row.set_item("suggestion", issue.suggestion)?;
        out.append(row)?;
    }
    Ok(out.unbind())
}

/// What a form's own rules say about a submission.
///
/// A rule the engine could not evaluate comes back as `not-evaluated`,
/// never as a pass: a rule that did not run has not been satisfied, and
/// reporting it as clean would be the one failure worth avoiding.
#[pyfunction]
#[pyo3(signature = (form, submission, today=None, now=None))]
fn submission_findings(
    py: Python<'_>,
    form: &str,
    submission: &str,
    today: Option<&str>,
    now: Option<&str>,
) -> PyResult<Py<PyList>> {
    let form_xml = document(form, "form")?;
    let instance_xml = document(submission, "submission")?;

    let parsed = engine::Form::parse(&form_xml).map_err(PyRuntimeError::new_err)?;
    let instance = engine::Instance::from_xml(&instance_xml).map_err(PyRuntimeError::new_err)?;
    let findings = parsed.check(&instance, clock(&instance, today, now));

    let out = PyList::empty(py);
    for finding in findings {
        let row = PyDict::new(py);
        row.set_item("path", &finding.node_path)?;
        row.set_item(
            "kind",
            match &finding.kind {
                engine::ViolationKind::Constraint => "constraint",
                engine::ViolationKind::Required => "required",
                engine::ViolationKind::Calculation { .. } => "calculation",
                engine::ViolationKind::Failed(_) => "not-evaluated",
            },
        )?;
        row.set_item("says", finding.describe())?;
        out.append(row)?;
    }
    Ok(out.unbind())
}

/// Evaluate one expression against a submission, for working out what a
/// rule does before writing it into a form.
///
/// An expression this engine cannot evaluate raises, rather than returning
/// a plausible value.
#[pyfunction]
#[pyo3(signature = (expression, submission, today=None, now=None))]
fn eval_expression(
    expression: &str,
    submission: &str,
    today: Option<&str>,
    now: Option<&str>,
) -> PyResult<String> {
    let instance_xml = document(submission, "submission")?;
    let instance = engine::Instance::from_xml(&instance_xml).map_err(PyRuntimeError::new_err)?;
    let parsed = engine::parse(expression).map_err(PyRuntimeError::new_err)?;
    let at = instance
        .root()
        .ok_or_else(|| PyRuntimeError::new_err("the submission has no root element"))?;
    let when = clock(&instance, today, now);
    let env = engine::Fixed {
        today: when.today,
        now: when.now,
    };
    let value = engine::evaluate(&parsed, &instance, engine::Context::at(at), &env)
        .map_err(PyRuntimeError::new_err)?;
    Ok(value.to_string_value(&instance))
}

/// A form being filled in.
///
/// Holds the instance, applies what the form derives, and reports what
/// moved. The same engine that judges the submission when it arrives, so a
/// form cannot behave one way while it is being filled and another way
/// afterwards.
#[pyclass]
struct Session {
    inner: engine::Session,
}

#[pymethods]
impl Session {
    /// Start a form. `instance` resumes a partly-filled one.
    #[new]
    #[pyo3(signature = (form, instance=None, today=None, now=None))]
    fn new(
        form: &str,
        instance: Option<&str>,
        today: Option<&str>,
        now: Option<&str>,
    ) -> PyResult<Self> {
        let xml = document(form, "form")?;
        let when = engine::Clock {
            today: today.unwrap_or("1970-01-01").to_string(),
            now: now.unwrap_or("1970-01-01T00:00:00.000Z").to_string(),
        };
        let inner = match instance {
            Some(saved) => {
                let saved = document(saved, "instance")?;
                engine::Session::resume(&xml, &saved, when)
            }
            None => engine::Session::new(&xml, when),
        }
        .map_err(PyRuntimeError::new_err)?;
        Ok(Session { inner })
    }

    /// Answer a question.
    fn set(&mut self, path: &str, value: &str) -> PyResult<()> {
        self.inner.set(path, value).map_err(PyValueError::new_err)
    }

    /// The current answer, or the empty string.
    fn get(&self, path: &str) -> String {
        self.inner.get(path)
    }

    /// Add a row to a repeat; returns how many there are now.
    fn add_row(&mut self, path: &str) -> PyResult<usize> {
        self.inner.add_row(path).map_err(PyValueError::new_err)
    }

    /// Remove one row, counted from 1 as XPath counts.
    fn remove_row(&mut self, path: &str, position: usize) -> PyResult<usize> {
        self.inner
            .remove_row(path, position)
            .map_err(PyValueError::new_err)
    }

    /// Run the form's logic and apply what it derives.
    ///
    /// Returns what is asked, what was calculated (only where it changed),
    /// what is missing, what was rejected, and how many rows each repeat
    /// has.
    fn recompute(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let outcome = self.inner.recompute();
        let out = PyDict::new(py);

        let calculated = PyDict::new(py);
        for (path, value) in &outcome.calculated {
            calculated.set_item(path, value)?;
        }
        out.set_item("calculated", calculated)?;

        let relevant = PyDict::new(py);
        for (path, shown) in &outcome.relevant {
            relevant.set_item(path, shown)?;
        }
        out.set_item("relevant", relevant)?;

        out.set_item("missing", &outcome.missing)?;
        out.set_item(
            "invalid",
            outcome
                .invalid
                .iter()
                .map(|(path, why)| (path.clone(), why.clone()))
                .collect::<Vec<_>>(),
        )?;
        out.set_item(
            "failed",
            outcome
                .failed
                .iter()
                .map(|(path, why)| (path.clone(), why.clone()))
                .collect::<Vec<_>>(),
        )?;

        let repeats = PyDict::new(py);
        for (path, count) in &outcome.repeats {
            repeats.set_item(path, count)?;
        }
        out.set_item("repeats", repeats)?;
        Ok(out.unbind())
    }

    /// The choices a filtered question offers right now, or `None` when its
    /// list is fixed.
    fn choices(&self, py: Python<'_>, path: &str) -> PyResult<Option<Py<PyList>>> {
        let Some(list) = self.inner.choices(path) else {
            return Ok(None);
        };
        let out = PyList::empty(py);
        for (value, label) in list {
            let row = PyDict::new(py);
            row.set_item("value", value)?;
            row.set_item("label", label)?;
            out.append(row)?;
        }
        Ok(Some(out.unbind()))
    }

    /// The instance as it would be submitted.
    fn instance_xml(&self) -> String {
        self.inner.instance_xml()
    }

    fn __repr__(&self) -> String {
        format!("<rxeval.Session {} bytes>", self.inner.instance_xml().len())
    }
}

#[pymodule]
fn rxeval(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_function(wrap_pyfunction!(check_form, module)?)?;
    module.add_function(wrap_pyfunction!(submission_findings, module)?)?;
    module.add_function(wrap_pyfunction!(eval_expression, module)?)?;
    module.add_class::<Session>()?;
    Ok(())
}
