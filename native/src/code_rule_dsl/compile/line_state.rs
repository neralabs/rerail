//! Bounded, policy-defined state and indentation filters for source-line queries.
use crate::code_rule_dsl::json::{bounded_text, invalid, object, reject_unknown, required_str};
use crate::code_rule_dsl::model::{RegexIndentation, RegexLineState, RegexLineTransition};
use napi::Result;
use regex::Regex;
use serde_json::Value;

pub(super) fn pattern(value: &Value, path: &str, maximum: usize) -> Result<Regex> {
    let text = bounded_text(Some(value), path, "maxRegexPatternBytes", maximum)?;
    Regex::new(text).map_err(|error| invalid(format!("{path}: {error}")))
}

pub(super) fn state(value: &Value, path: &str, maximum: usize) -> Result<RegexLineState> {
    let spec = object(value, path)?;
    reject_unknown(spec, &["initial", "emit", "transitions"], path)?;
    let initial = required_str(value, "initial", path)?;
    let transitions = spec
        .get("transitions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{path}/transitions must be an array")))?;
    if transitions.is_empty() || transitions.len() > 64 {
        return Err(invalid(format!(
            "{path}/transitions requires 1 to 64 transitions"
        )));
    }
    let mut names = vec![initial.to_owned()];
    let mut intern = |name: &str| -> Result<usize> {
        if name.is_empty() || name.len() > 64 {
            return Err(invalid(format!(
                "{path}: state names require 1 to 64 bytes"
            )));
        }
        if let Some(at) = names.iter().position(|known| known == name) {
            return Ok(at);
        }
        if names.len() >= 16 {
            return Err(invalid(format!("{path} allows at most 16 states")));
        }
        names.push(name.to_owned());
        Ok(names.len() - 1)
    };
    intern(initial)?;
    let transitions = transitions
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let at = format!("{path}/transitions/{i}");
            let transition = object(value, &at)?;
            reject_unknown(transition, &["from", "to", "pattern", "count", "skip"], &at)?;
            let from = intern(required_str(value, "from", &at)?)?;
            let to = intern(required_str(value, "to", &at)?)?;
            let pattern = pattern(
                transition.get("pattern").unwrap_or(&Value::Null),
                &format!("{at}/pattern"),
                maximum,
            )?;
            let mut min = 1;
            let mut max = None;
            let mut odd = None;
            if let Some(value) = transition.get("count") {
                let count = object(value, &format!("{at}/count"))?;
                reject_unknown(count, &["min", "max", "parity"], &format!("{at}/count"))?;
                let number = |key: &str| -> Result<Option<u64>> {
                    count
                        .get(key)
                        .map(|v| {
                            v.as_u64().ok_or_else(|| {
                                invalid(format!("{at}/count/{key} must be non-negative"))
                            })
                        })
                        .transpose()
                };
                min = number("min")?.unwrap_or(1);
                max = number("max")?;
                if max.is_some_and(|max| min > max) {
                    return Err(invalid(format!("{at}/count: min exceeds max")));
                }
                odd = match count.get("parity").and_then(Value::as_str) {
                    None if !count.contains_key("parity") => None,
                    Some("odd") => Some(true),
                    Some("even") => Some(false),
                    _ => return Err(invalid(format!("{at}/count/parity must be odd or even"))),
                };
            }
            let skip = match transition.get("skip") {
                None => false,
                Some(Value::Bool(value)) => *value,
                _ => return Err(invalid(format!("{at}/skip must be boolean"))),
            };
            Ok(RegexLineTransition {
                from,
                to,
                pattern,
                min,
                max,
                odd,
                skip,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let emit_names = spec
        .get("emit")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{path}/emit must be an array")))?;
    if emit_names.is_empty() || emit_names.len() > 16 {
        return Err(invalid(format!("{path}/emit requires 1 to 16 states")));
    }
    let mut emit = vec![false; names.len()];
    for name in emit_names {
        let at = names
            .iter()
            .position(|known| Some(known.as_str()) == name.as_str())
            .ok_or_else(|| invalid(format!("{path}/emit names an unknown state")))?;
        emit[at] = true;
    }
    Ok(RegexLineState {
        initial: 0,
        emit,
        transitions,
    })
}

pub(super) fn indentation(value: &Value, path: &str, maximum: usize) -> Result<RegexIndentation> {
    let spec = object(value, path)?;
    reject_unknown(spec, &["excludeUnder", "reset", "ignore"], path)?;
    let read = |key: &str| {
        pattern(
            spec.get(key).unwrap_or(&Value::Null),
            &format!("{path}/{key}"),
            maximum,
        )
    };
    let ignore = spec
        .get("ignore")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{path}/ignore must be an array")))?;
    if ignore.len() > 64 {
        return Err(invalid(format!("{path}/ignore allows at most 64 patterns")));
    }
    Ok(RegexIndentation {
        exclude_under: read("excludeUnder")?,
        reset: read("reset")?,
        ignore: ignore
            .iter()
            .enumerate()
            .map(|(i, v)| pattern(v, &format!("{path}/ignore/{i}"), maximum))
            .collect::<Result<_>>()?,
    })
}
