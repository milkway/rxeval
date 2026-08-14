//! A form being filled in.
//!
//! [`Rules`] answers questions about a finished submission: is this valid,
//! what does it calculate to, which nodes were relevant. That is the right
//! shape for a server checking what arrived, and the wrong shape for a
//! screen someone is typing into, where the same questions have to be
//! answered again after every keystroke and the answers have to be applied
//! rather than reported.
//!
//! A [`Session`] holds the instance, applies calculations in dependency
//! order, and reports what changed. It is deliberately the same engine:
//! a form that behaves one way while being filled and another way when it
//! arrives is worse than one that is wrong in a single consistent way,
//! because only the first kind produces data nobody can explain.
//!
//! ## What it does not do
//!
//! It does not decide what a question looks like, what order questions are
//! asked in, or what a language is called. Those are the form *body*'s
//! business and a renderer's; this module knows the model — paths, values,
//! and the expressions binding them.

use std::collections::{BTreeMap, BTreeSet};

use crate::eval::{evaluate, Context, Value};
use crate::rules::{Clock, Form, Violation};
use crate::tree::{Instance, NodeId};

/// A form and the answers so far.
pub struct Session {
    form: Form,
    instance: Instance,
    clock: Clock,
    /// One blank row per repeat, by the repeat's path.
    ///
    /// An XForm carries these inside the instance, marked `jr:template`.
    /// They are a form's blueprint and not a respondent's answer, so they
    /// are lifted out here: left in place they would be counted as a row,
    /// validated as a row, and submitted as a row of empty answers.
    templates: BTreeMap<String, Instance>,
}

/// What the engine says about the form as it currently stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// Paths whose value the form computed, and what it computed.
    ///
    /// Only paths that *changed* appear: a renderer redrawing every
    /// calculated field on every keystroke would fight the cursor.
    pub calculated: BTreeMap<String, String>,
    /// Paths that are relevant, and paths that are not. A path missing
    /// from this map has no relevance expression and is always asked.
    pub relevant: BTreeMap<String, bool>,
    /// Paths that must be answered and are not, given current relevance.
    pub missing: Vec<String>,
    /// Answers the form's own rules reject, with the message to show.
    pub invalid: Vec<(String, String)>,
    /// Expressions that could not be evaluated at all. These are form bugs,
    /// not answer problems, and they are kept separate for that reason: an
    /// enumerator can fix an answer and can do nothing about a broken
    /// `relevant`.
    pub failed: Vec<(String, String)>,
    /// How many rows each repeat has, by the repeat's path.
    pub repeats: BTreeMap<String, usize>,
}

impl Session {
    /// Start filling in a form, from the XForm the server serves.
    pub fn new(xform: &str, clock: Clock) -> Result<Self, String> {
        let form = Form::parse(xform)?;
        let mut instance = blank_instance(xform)?;
        let templates = lift_templates(&mut instance);
        Ok(Session {
            form,
            instance,
            clock,
            templates,
        })
    }

    /// Resume a partly-filled form.
    pub fn resume(xform: &str, instance_xml: &str, clock: Clock) -> Result<Self, String> {
        let form = Form::parse(xform)?;
        let mut instance =
            Instance::from_xml(instance_xml).map_err(|e| format!("saved instance: {e}"))?;
        // A saved instance has no templates — they were lifted before it was
        // ever shown — so they come from the form, which is where they
        // belong anyway.
        let mut blank = blank_instance(xform)?;
        let templates = lift_templates(&mut blank);
        let _ = lift_templates(&mut instance);
        Ok(Session {
            form,
            instance,
            clock,
            templates,
        })
    }

    /// Answer a question.
    ///
    /// The path is created if the blank instance did not have it — a form
    /// whose template omits a node still has to be able to hold its answer.
    pub fn set(&mut self, path: &str, value: &str) -> Result<(), String> {
        match self.node_at(path) {
            Some(node) => {
                self.instance.node_mut(node).value = value.to_string();
                Ok(())
            }
            None => self.create_at(path, value),
        }
    }

    /// The current answer, or the empty string for an unanswered question.
    pub fn get(&self, path: &str) -> String {
        self.node_at(path)
            .map(|node| self.instance.string_value(node))
            .unwrap_or_default()
    }

    /// Run the form's logic and apply what it derives.
    ///
    /// Calculations run first, in dependency order, and their results are
    /// written into the instance — that is the difference between this and
    /// checking a finished submission, which only reports the disagreement.
    pub fn recompute(&mut self) -> Outcome {
        let mut outcome = Outcome::default();

        let (computed, failed) = self.form.calculations(&self.instance, &self.clock);
        outcome.failed = failed;
        for (path, value) in computed {
            if self.get(&path) != value {
                let _ = self.set(&path, &value);
                outcome.calculated.insert(path, value);
            }
        }

        let (relevance, problems) = self.form.relevance(&self.instance, &self.clock);
        outcome.relevant = relevance;
        for problem in problems {
            outcome
                .failed
                .push((problem.node_path.clone(), problem.describe()));
        }

        // An unanswered question nobody was asked is the normal state of
        // most of a form; saying so on every keystroke is noise.
        let hidden: BTreeSet<String> = outcome
            .relevant
            .iter()
            .filter(|(_, shown)| !**shown)
            .map(|(path, _)| path.clone())
            .collect();

        for violation in self.form.check(&self.instance, self.clock.clone()) {
            let path = violation.node_path.clone();
            if hidden.contains(&path) {
                continue;
            }
            use crate::rules::ViolationKind::*;
            match &violation.kind {
                Required => outcome.missing.push(path),
                Constraint => outcome.invalid.push((
                    path,
                    violation
                        .message
                        .clone()
                        .unwrap_or_else(|| "this answer is not allowed".into()),
                )),
                Failed(why) => outcome.failed.push((path, why.clone())),
                // The calculations above just ran; a disagreement with them
                // would be the failure recorded there, not a finding here.
                Calculation { .. } => {}
            }
        }
        outcome.repeats = self.repeat_counts();
        outcome
    }

    /// How many rows each repeat currently has.
    ///
    /// A repeat with no rows is a real state — the form asked for a list and
    /// the list is empty — and it is different from a repeat whose single
    /// row is blank.
    pub fn repeat_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for path in self.templates.keys() {
            counts.insert(path.clone(), self.rows_of(path).len());
        }
        counts
    }

    /// Add a row to a repeat. Returns how many rows there are now.
    ///
    /// The row is a copy of the form's own template, placed after the last
    /// existing row rather than at the end of its parent: appended blindly
    /// it would land after `meta` and after every question that follows the
    /// repeat, putting both the submission and every positional path out of
    /// order.
    pub fn add_row(&mut self, path: &str) -> Result<usize, String> {
        let template = self
            .templates
            .get(path)
            .ok_or_else(|| format!("'{path}' is not a repeat in this form"))?
            .clone();
        let template_root = template
            .root()
            .ok_or_else(|| format!("the template for '{path}' is empty"))?;

        let rows = self.rows_of(path);
        let copied = self.instance.adopt(&template, template_root);
        match rows.last() {
            Some(last) => self.instance.insert_after(*last, copied),
            None => {
                // No rows yet: the row goes where the repeat itself sits in
                // the form, which is under the parent the path names.
                let parent_path = path.rsplit_once('/').map(|(head, _)| head).unwrap_or("");
                let parent = match parent_path {
                    "" => self.instance.root(),
                    other => self.node_at(other),
                }
                .ok_or_else(|| format!("'{path}' has nowhere to hang"))?;
                self.instance.append_child(parent, copied);
            }
        }
        self.instance.reindex();
        Ok(self.rows_of(path).len())
    }

    /// Remove one row, counted from 1 as XPath counts.
    pub fn remove_row(&mut self, path: &str, position: usize) -> Result<usize, String> {
        let rows = self.rows_of(path);
        if position == 0 || position > rows.len() {
            return Err(format!(
                "'{path}' has {} row(s); there is no row {position}",
                rows.len()
            ));
        }
        self.instance.detach(rows[position - 1]);
        self.instance.reindex();
        Ok(self.rows_of(path).len())
    }

    /// The nodes that are the rows of a repeat, in document order.
    fn rows_of(&self, path: &str) -> Vec<NodeId> {
        let Some((parent_path, name)) = path.rsplit_once('/') else {
            return Vec::new();
        };
        let parent = match parent_path {
            "" => self.instance.root(),
            other => self.node_at(other),
        };
        let Some(parent) = parent else {
            return Vec::new();
        };
        self.instance
            .children(parent)
            .into_iter()
            .filter(|child| self.instance.node(*child).name == name)
            .collect()
    }

    /// The choices a question offers right now.
    ///
    /// `None` when the question's choices are written out in the form and
    /// therefore never change; the renderer already has those.
    pub fn choices(&self, path: &str) -> Option<Vec<(String, String)>> {
        self.form.choices(&self.instance, path)
    }

    /// The instance as it would be submitted.
    pub fn instance_xml(&self) -> String {
        let mut out = String::from("<?xml version='1.0' ?>");
        if let Some(root) = self.instance.root() {
            write_element(&self.instance, root, &mut out);
        }
        out
    }

    /// Every violation, for the moment someone presses send. Unlike
    /// [`Self::recompute`], this reports irrelevant nodes too — a value
    /// sitting in a question nobody was asked is worth knowing about
    /// before it is sent, not after.
    pub fn check_all(&self) -> Vec<Violation> {
        self.form.check(&self.instance, self.clock.clone())
    }

    fn node_at(&self, path: &str) -> Option<NodeId> {
        let expr = crate::parser::parse(path).ok()?;
        let root = self.instance.root()?;
        let env = crate::eval::Fixed {
            today: self.clock.today.clone(),
            now: self.clock.now.clone(),
        };
        match evaluate(&expr, &self.instance, Context::at(root), &env) {
            Ok(Value::NodeSet(nodes)) => nodes.first().copied(),
            _ => None,
        }
    }

    /// Create a path the template did not have, one element at a time.
    fn create_at(&mut self, path: &str, value: &str) -> Result<(), String> {
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Err(format!("'{path}' names nothing"));
        }
        // A positional step names a row that exists. If it did not resolve,
        // the row is not there — and creating one would mean inventing an
        // element literally called `morador[3]`, which is not a name, and
        // producing a submission no parser will accept. Rows come from
        // `add_row`, which copies the form's own template.
        if let Some(indexed) = parts.iter().find(|part| part.contains('[')) {
            return Err(format!(
                "'{path}' points at {indexed}, which does not exist — add the row first \
                 with add_row()"
            ));
        }
        let leaf = parts.pop().expect("checked above");
        let mut here = self
            .instance
            .root()
            .ok_or_else(|| "the instance has no root".to_string())?;
        // The first part is the root itself.
        if !parts.is_empty() && self.instance.node(here).name == parts[0] {
            parts.remove(0);
        }
        for part in parts {
            let existing = self
                .instance
                .children(here)
                .into_iter()
                .find(|child| self.instance.node(*child).name == part);
            here = match existing {
                Some(child) => child,
                None => {
                    let child = self.instance.create_element(part, "");
                    self.instance.append_child(here, child);
                    child
                }
            };
        }
        let leaf_node = self.instance.create_element(leaf, value);
        self.instance.append_child(here, leaf_node);
        self.instance.reindex();
        Ok(())
    }
}

/// The blank instance an XForm carries as its template.
fn blank_instance(xform: &str) -> Result<Instance, String> {
    let document = Instance::from_xml(xform).map_err(|e| format!("XForm XML: {e}"))?;
    let root = document.root().ok_or("the XForm has no root element")?;
    // The first <instance> under <model>: the primary one, by definition.
    let primary = document
        .descendants(root)
        .into_iter()
        .find(|node| document.node(*node).name == "instance")
        .ok_or("the XForm has no primary instance")?;
    let template = document
        .children(primary)
        .into_iter()
        .next()
        .ok_or("the XForm's primary instance is empty")?;
    let mut instance = Instance::new();
    let copied = instance.adopt(&document, template);
    instance.set_root(copied);
    instance.reindex();
    Ok(instance)
}

fn write_element(instance: &Instance, node: NodeId, out: &mut String) {
    let name = &instance.node(node).name;
    out.push('<');
    out.push_str(name);
    for attribute in instance.attributes(node) {
        out.push(' ');
        out.push_str(&instance.node(attribute).name);
        out.push_str("=\"");
        escape_into(&instance.node(attribute).value, out);
        out.push('"');
    }
    let children = instance.children(node);
    if children.is_empty() {
        let value = &instance.node(node).value;
        if value.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        escape_into(value, out);
    } else {
        out.push('>');
        for child in children {
            write_element(instance, child, out);
        }
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

fn escape_into(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Take the `jr:template` rows out of an instance, keyed by their path.
///
/// The attribute arrives here as `template`: the parser keeps local names,
/// and instances are single-namespace in practice.
fn lift_templates(instance: &mut Instance) -> BTreeMap<String, Instance> {
    let mut found = BTreeMap::new();
    let Some(root) = instance.root() else {
        return found;
    };
    let templates: Vec<NodeId> = instance
        .descendants(root)
        .into_iter()
        .filter(|node| {
            instance
                .attributes(*node)
                .into_iter()
                .any(|a| instance.node(a).name == "template")
        })
        .collect();

    for node in templates {
        // The path is taken before detaching, and without the position: a
        // template is the blueprint for every row, not for one of them.
        let path = instance.path_of(node);
        let path = match path.rfind('[') {
            Some(at) if path.ends_with(']') => path[..at].to_string(),
            _ => path,
        };
        let mut lifted = Instance::new();
        let copied = lifted.adopt(instance, node);
        lifted.set_root(copied);
        // The copy carries the marker; a row made from it must not, or the
        // next lift would take the row for a template.
        let markers: Vec<NodeId> = lifted
            .attributes(copied)
            .into_iter()
            .filter(|a| lifted.node(*a).name == "template")
            .collect();
        lifted
            .node_mut(copied)
            .attributes
            .retain(|a| !markers.contains(a));
        found.insert(path, lifted);
        instance.detach(node);
    }
    instance.reindex();
    found
}
