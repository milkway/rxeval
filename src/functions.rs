//! The function library: XPath 1.0's, plus the OpenRosa extensions.
//!
//! A name this table does not carry is an error naming itself. That rule is
//! the point of the crate: a form engine that answers a function it does not
//! know — with an empty string, with false — produces a form that looks like
//! it worked.

use crate::eval::{evaluate, format_number, string_to_number, Context, Environment, Value};
use crate::parser::Expr;
use crate::tree::Instance;

type Result<T> = std::result::Result<T, String>;

pub fn call(
    name: &str,
    args: &[Expr],
    instance: &Instance,
    context: Context,
    env: &dyn Environment,
) -> Result<Value> {
    let arg = |i: usize| -> Result<Value> {
        let expr = args
            .get(i)
            .ok_or_else(|| format!("{name}() needs an argument in position {}", i + 1))?;
        evaluate(expr, instance, context, env)
    };
    let text = |i: usize| -> Result<String> { Ok(arg(i)?.to_string_value(instance)) };
    let number = |i: usize| -> Result<f64> { Ok(arg(i)?.to_number(instance)) };
    let boolean = |i: usize| -> Result<bool> { Ok(arg(i)?.to_boolean(instance)) };
    let arity = |expected: &[usize]| -> Result<()> {
        if expected.contains(&args.len()) {
            Ok(())
        } else {
            Err(format!(
                "{name}() takes {} argument(s), got {}",
                expected
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(" or "),
                args.len()
            ))
        }
    };

    match name {
        // ---- node-set
        "count" => {
            arity(&[1])?;
            match arg(0)? {
                Value::NodeSet(nodes) => Ok(Value::Number(nodes.len() as f64)),
                other => Err(format!(
                    "count() counts nodes, and was given {}",
                    crate::eval::type_name(&other)
                )),
            }
        }
        "position" => {
            arity(&[0])?;
            Ok(Value::Number(context.position as f64))
        }
        "last" => {
            arity(&[0])?;
            Ok(Value::Number(context.size as f64))
        }
        "name" | "local-name" => {
            arity(&[0, 1])?;
            let node = if args.is_empty() {
                Some(context.node)
            } else {
                match arg(0)? {
                    Value::NodeSet(nodes) => nodes.first().copied(),
                    other => {
                        return Err(format!(
                            "{name}() names a node, and was given {}",
                            crate::eval::type_name(&other)
                        ))
                    }
                }
            };
            Ok(Value::String(
                node.map(|n| instance.node(n).name.clone())
                    .unwrap_or_default(),
            ))
        }

        // ---- string
        "string" => {
            arity(&[0, 1])?;
            if args.is_empty() {
                Ok(Value::String(instance.string_value(context.node)))
            } else {
                Ok(Value::String(text(0)?))
            }
        }
        "concat" => {
            let mut out = String::new();
            for i in 0..args.len() {
                out.push_str(&text(i)?);
            }
            Ok(Value::String(out))
        }
        "string-length" => {
            arity(&[0, 1])?;
            let value = if args.is_empty() {
                instance.string_value(context.node)
            } else {
                text(0)?
            };
            Ok(Value::Number(value.chars().count() as f64))
        }
        "normalize-space" => {
            arity(&[0, 1])?;
            let value = if args.is_empty() {
                instance.string_value(context.node)
            } else {
                text(0)?
            };
            Ok(Value::String(
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            ))
        }
        "contains" => {
            arity(&[2])?;
            Ok(Value::Boolean(text(0)?.contains(&text(1)?)))
        }
        "starts-with" => {
            arity(&[2])?;
            Ok(Value::Boolean(text(0)?.starts_with(&text(1)?)))
        }
        "ends-with" => {
            arity(&[2])?;
            Ok(Value::Boolean(text(0)?.ends_with(&text(1)?)))
        }
        "substring-before" => {
            arity(&[2])?;
            let haystack = text(0)?;
            let needle = text(1)?;
            Ok(Value::String(match haystack.find(&needle) {
                Some(i) => haystack[..i].to_string(),
                None => String::new(),
            }))
        }
        "substring-after" => {
            arity(&[2])?;
            let haystack = text(0)?;
            let needle = text(1)?;
            Ok(Value::String(match haystack.find(&needle) {
                Some(i) => haystack[i + needle.len()..].to_string(),
                None => String::new(),
            }))
        }
        // XPath counts from 1 and rounds its arguments; ODK's substr()
        // counts from 0 and takes an exclusive end. Both exist, and mixing
        // them up shifts every extracted code by one character.
        "substring" => {
            arity(&[2, 3])?;
            let value: Vec<char> = text(0)?.chars().collect();
            let start = number(1)?;
            let count = if args.len() == 3 {
                number(2)?
            } else {
                f64::INFINITY
            };
            let from = (start.round() - 1.0).max(0.0);
            let to = if count.is_infinite() {
                value.len() as f64
            } else {
                (start.round() - 1.0 + count.round()).max(0.0)
            };
            let from = (from as usize).min(value.len());
            let to = (to as usize).min(value.len());
            Ok(Value::String(if from >= to {
                String::new()
            } else {
                value[from..to].iter().collect()
            }))
        }
        "substr" => {
            arity(&[2, 3])?;
            let value: Vec<char> = text(0)?.chars().collect();
            let len = value.len() as f64;
            let normalize = |i: f64| {
                if i < 0.0 {
                    (len + i).max(0.0)
                } else {
                    i.min(len)
                }
            };
            let from = normalize(number(1)?) as usize;
            let to = if args.len() == 3 {
                normalize(number(2)?) as usize
            } else {
                value.len()
            };
            Ok(Value::String(if from >= to {
                String::new()
            } else {
                value[from..to].iter().collect()
            }))
        }
        "translate" => {
            arity(&[3])?;
            let value = text(0)?;
            let from: Vec<char> = text(1)?.chars().collect();
            let to: Vec<char> = text(2)?.chars().collect();
            Ok(Value::String(
                value
                    .chars()
                    .filter_map(|c| match from.iter().position(|f| *f == c) {
                        Some(i) => to.get(i).copied(),
                        None => Some(c),
                    })
                    .collect(),
            ))
        }

        // ---- boolean
        "boolean" => {
            arity(&[1])?;
            Ok(Value::Boolean(boolean(0)?))
        }
        "not" => {
            arity(&[1])?;
            Ok(Value::Boolean(!boolean(0)?))
        }
        "true" => {
            arity(&[0])?;
            Ok(Value::Boolean(true))
        }
        "false" => {
            arity(&[0])?;
            Ok(Value::Boolean(false))
        }
        // ODK's, and not the same as boolean(): "0" and "false" are false
        // here, where XPath calls any non-empty string true.
        "boolean-from-string" => {
            arity(&[1])?;
            let value = text(0)?;
            Ok(Value::Boolean(
                value == "1" || value.eq_ignore_ascii_case("true"),
            ))
        }

        // ---- number
        "number" => {
            arity(&[0, 1])?;
            if args.is_empty() {
                Ok(Value::Number(string_to_number(
                    &instance.string_value(context.node),
                )))
            } else {
                Ok(Value::Number(number(0)?))
            }
        }
        "sum" => {
            arity(&[1])?;
            match arg(0)? {
                Value::NodeSet(nodes) => Ok(Value::Number(
                    nodes
                        .iter()
                        .map(|n| string_to_number(&instance.string_value(*n)))
                        .sum(),
                )),
                other => Err(format!(
                    "sum() adds up a node-set, and was given {}",
                    crate::eval::type_name(&other)
                )),
            }
        }
        "floor" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.floor()))
        }
        "ceiling" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.ceil()))
        }
        // ODK adds a second argument for decimal places; XPath's round has
        // one and goes to the nearest integer.
        "round" => {
            arity(&[1, 2])?;
            let value = number(0)?;
            if args.len() == 1 {
                return Ok(Value::Number(value.round()));
            }
            let places = number(1)?;
            let factor = 10f64.powf(places.trunc());
            Ok(Value::Number((value * factor).round() / factor))
        }
        "int" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.trunc()))
        }
        "abs" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.abs()))
        }
        "pow" => {
            arity(&[2])?;
            Ok(Value::Number(number(0)?.powf(number(1)?)))
        }
        "log" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.ln()))
        }
        "log10" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.log10()))
        }
        "exp" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.exp()))
        }
        "sqrt" => {
            arity(&[1])?;
            Ok(Value::Number(number(0)?.sqrt()))
        }
        "min" | "max" => {
            // Over a node-set or a list; an empty one is NaN, not zero,
            // because "no answers" is not "zero".
            let mut values: Vec<f64> = Vec::new();
            for i in 0..args.len() {
                match arg(i)? {
                    Value::NodeSet(nodes) => values.extend(
                        nodes
                            .iter()
                            .map(|n| string_to_number(&instance.string_value(*n))),
                    ),
                    other => values.push(other.to_number(instance)),
                }
            }
            if values.is_empty() || values.iter().any(|v| v.is_nan()) {
                return Ok(Value::Number(f64::NAN));
            }
            Ok(Value::Number(if name == "min" {
                values.into_iter().fold(f64::INFINITY, f64::min)
            } else {
                values.into_iter().fold(f64::NEG_INFINITY, f64::max)
            }))
        }

        // ---- OpenRosa selects
        "selected" => {
            arity(&[2])?;
            let haystack = text(0)?;
            let needle = text(1)?;
            Ok(Value::Boolean(
                haystack.split_whitespace().any(|s| s == needle.trim()),
            ))
        }
        "selected-at" => {
            arity(&[2])?;
            let haystack = text(0)?;
            let index = number(1)?;
            if index < 0.0 {
                return Ok(Value::String(String::new()));
            }
            Ok(Value::String(
                haystack
                    .split_whitespace()
                    .nth(index as usize)
                    .unwrap_or_default()
                    .to_string(),
            ))
        }
        "count-selected" => {
            arity(&[1])?;
            Ok(Value::Number(text(0)?.split_whitespace().count() as f64))
        }
        "join" => {
            arity(&[2])?;
            let separator = text(0)?;
            match arg(1)? {
                Value::NodeSet(nodes) => Ok(Value::String(
                    nodes
                        .iter()
                        .map(|n| instance.string_value(*n))
                        .collect::<Vec<_>>()
                        .join(&separator),
                )),
                other => Ok(Value::String(other.to_string_value(instance))),
            }
        }

        // ---- control
        "if" => {
            arity(&[3])?;
            // only the taken branch is evaluated: the other may divide by a
            // zero the condition exists to avoid
            if boolean(0)? {
                arg(1)
            } else {
                arg(2)
            }
        }
        "coalesce" => {
            arity(&[2])?;
            let first = text(0)?;
            if first.is_empty() {
                Ok(Value::String(text(1)?))
            } else {
                Ok(Value::String(first))
            }
        }
        "once" => {
            arity(&[1])?;
            // Meaningful only while filling a form, where it means "compute
            // this if it has no value yet". Evaluating a stored submission,
            // the value is already there.
            Ok(Value::String(instance.string_value(context.node)))
        }

        // ---- time
        "today" => {
            arity(&[0])?;
            Ok(Value::String(env.today()))
        }
        "now" => {
            arity(&[0])?;
            Ok(Value::String(env.now()))
        }

        // ---- named but not implemented
        //
        // Listed by name so the message says which piece is missing rather
        // than "unknown function", and so nobody mistakes the gap for a
        // feature that silently returned nothing.
        "pulldata" | "jr:choice-name" | "choice-name" | "indexed-repeat" | "current"
        | "randomize" | "uuid" | "digest" | "format-date" | "format-date-time" | "date"
        | "date-time" | "decimal-date-time" | "decimal-time" | "area" | "distance" | "regex"
        | "checklist" | "weighted-checklist" | "position-in-repeat" => Err(format!(
            "{name}() is not implemented yet — rxeval refuses to guess at a \
             value the form will act on"
        )),

        other => Err(format!("unknown function {other}()")),
    }
}

/// Number formatting is shared with the evaluator, and re-exported here so
/// callers testing functions do not reach across modules.
pub fn number_to_string(n: f64) -> String {
    format_number(n)
}
