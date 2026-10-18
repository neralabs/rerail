use crate::code_rule_dsl::json::{bounded_text, invalid, object, reject_unknown};
use crate::code_rule_dsl::matcher::{Reading, Separator, Source, StringCase, Trans, Transform};
use crate::code_rule_dsl::model::Limits;
use ast_grep_core::meta_var::MetaVariable;
use napi::Result;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// A metavariable name: A-Z, 0-9 and `_`, as captures are named.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A transformation's `source` (`$NAME` or `$$$NAME`, upstream's `parse_meta_var`), `default`, the
/// text an unbound source reads as, and `value`, whether a capture's value is read (with `reading`)
/// rather than its source text. Whether `value`'s source is a capture is checked with the query
/// (`check_value_sources`).
fn source(
    body: &Map<String, Value>,
    at: &str,
    limits: &Limits,
    reading: &Reading,
) -> Result<Source> {
    let default = body
        .get("default")
        .map(|value| string_within_limit(value, &format!("{at}/default"), limits))
        .transpose()?;
    let variable = variable(body.get("source"), &format!("{at}/source"))?;
    let value = match body.get("value") {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => {
            if matches!(variable, MetaVariable::MultiCapture(_)) {
                return Err(invalid(format!(
                    "{at}/value: a value is read from a single capture (`$NAME`)"
                )));
            }
            Some(reading.clone())
        }
        Some(_) => return Err(invalid(format!("{at}/value must be a boolean"))),
    };
    Ok(Source {
        variable,
        default,
        value,
    })
}

fn string_within_limit(value: &Value, path: &str, limits: &Limits) -> Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid(format!("{path} must be a string")))?;
    if text.len() > limits.max_source_bytes {
        return Err(invalid(format!(
            "{path} exceeds maxSourceBytes ({})",
            limits.max_source_bytes
        )));
    }
    Ok(text.to_owned())
}

fn variable(value: Option<&Value>, path: &str) -> Result<MetaVariable> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{path} is required: `$NAME` or `$$$NAME`")))?;
    let variable = match text.strip_prefix("$$$") {
        Some(name) if is_name(name) => MetaVariable::MultiCapture(name.to_owned()),
        _ => match text.strip_prefix('$') {
            Some(name) if is_name(name) => MetaVariable::Capture(name.to_owned(), false),
            _ => {
                return Err(invalid(format!(
                    "{path}: `{text}` should be a $-prefixed metavariable (`$NAME` or `$$$NAME`)"
                )));
            }
        },
    };
    Ok(variable)
}

fn trans(value: &Value, path: &str, limits: &Limits, reading: &Reading) -> Result<Trans> {
    let Value::Object(entry) = value else {
        return Err(invalid(format!(
            "{path} must be an object with one of replace, substring, convert, line, lines or count; \
             the string form is not supported"
        )));
    };
    reject_unknown(
        entry,
        &[
            "replace",
            "substring",
            "convert",
            "line",
            "lines",
            "count",
            "rewrite",
        ],
        path,
    )?;
    if entry.contains_key("rewrite") {
        return Err(invalid(format!(
            "{path}/rewrite: rewrite is not supported yet; use replace, substring or convert"
        )));
    }
    let [(kind, body)] = entry.iter().collect::<Vec<_>>()[..] else {
        return Err(invalid(format!(
            "{path} must hold exactly one of replace, substring, convert, line, lines or count"
        )));
    };
    let at = format!("{path}/{kind}");
    let body: &Map<String, Value> = object(body, &at)?;
    Ok(match kind.as_str() {
        "replace" => {
            reject_unknown(body, &["source", "default", "value", "replace", "by"], &at)?;
            let pattern = bounded_text(
                body.get("replace"),
                &format!("{at}/replace"),
                "maxRegexPatternBytes",
                limits.max_regex_pattern_bytes,
            )?;
            let regex = Regex::new(pattern)
                .map_err(|_| invalid(format!("{at}/replace: Parsing text matcher fails.")))?;
            let by = string_within_limit(
                body.get("by")
                    .ok_or_else(|| invalid(format!("{at}/by must be a string")))?,
                &format!("{at}/by"),
                limits,
            )?;
            Trans::Replace {
                source: source(body, &at, limits, reading)?,
                regex,
                by,
            }
        }
        "substring" => {
            reject_unknown(
                body,
                &["source", "default", "value", "startChar", "endChar"],
                &at,
            )?;
            let index = |key: &str| -> Result<Option<i32>> {
                body.get(key)
                    .map(|value| {
                        value
                            .as_i64()
                            .and_then(|index| i32::try_from(index).ok())
                            .ok_or_else(|| invalid(format!("{at}/{key} must be an integer")))
                    })
                    .transpose()
            };
            Trans::Substring {
                source: source(body, &at, limits, reading)?,
                start: index("startChar")?,
                end: index("endChar")?,
            }
        }
        "line" => {
            reject_unknown(body, &["source", "default", "edge"], &at)?;
            let end = match body.get("edge").map(|edge| edge.as_str()) {
                None | Some(Some("start")) => false,
                Some(Some("end")) => true,
                Some(_) => {
                    return Err(invalid(format!("{at}/edge must be \"start\" or \"end\"")));
                }
            };
            Trans::Line {
                source: source(body, &at, limits, reading)?,
                end,
            }
        }
        "lines" | "count" => {
            reject_unknown(body, &["source", "default"], &at)?;
            let source = source(body, &at, limits, reading)?;
            if kind == "lines" {
                Trans::Lines { source }
            } else {
                Trans::Count { source }
            }
        }
        _ => {
            reject_unknown(
                body,
                &["source", "default", "value", "toCase", "separatedBy"],
                &at,
            )?;
            let choice = |value: &Value, key: &str, names: &[&str]| {
                invalid(format!(
                    "{at}/{key}: unknown value {value}; expected one of {}",
                    names.join(", ")
                ))
            };
            let cases = StringCase::NAMES.map(|(name, _)| name);
            let case = body
                .get("toCase")
                .ok_or_else(|| invalid(format!("{at}/toCase is required")))
                .and_then(|value| {
                    StringCase::NAMES
                        .iter()
                        .find(|(name, _)| value.as_str() == Some(*name))
                        .map(|(_, case)| *case)
                        .ok_or_else(|| choice(value, "toCase", &cases))
                })?;
            let separators = body
                .get("separatedBy")
                .map(|value| {
                    let names = Separator::NAMES.map(|(name, _)| name);
                    value
                        .as_array()
                        .ok_or_else(|| invalid(format!("{at}/separatedBy must be an array")))?
                        .iter()
                        .map(|item| {
                            Separator::NAMES
                                .iter()
                                .find(|(name, _)| item.as_str() == Some(*name))
                                .map(|(_, separator)| *separator)
                                .ok_or_else(|| choice(item, "separatedBy", &names))
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;
            Trans::Convert {
                source: source(body, &at, limits, reading)?,
                case,
                separators,
            }
        }
    })
}

/// An ast query's `transform`: names to transformations, ordered so that each source is computed
/// before it is read. A cycle is refused, as upstream refuses it. `reading` is how a `value`
/// source's value is read (the query's language and options); a `value` source must be a capture,
/// not another transform variable.
pub(super) fn compile_transform(
    value: &Value,
    path: &str,
    limits: &Limits,
    reading: &Reading,
) -> Result<Transform> {
    let entries = object(value, path)?;
    let mut parsed = BTreeMap::new();
    for (name, entry) in entries {
        if !is_name(name) {
            return Err(invalid(format!(
                "{path}/{name}: `{name}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
            )));
        }
        parsed.insert(
            name.clone(),
            trans(entry, &format!("{path}/{name}"), limits, reading)?,
        );
    }
    for (name, trans) in &parsed {
        if trans.reads_value() && parsed.contains_key(trans.source_name()) {
            let kind = entries[name]
                .as_object()
                .and_then(|entry| entry.keys().next())
                .map_or("", String::as_str);
            return Err(invalid(format!(
                "{path}/{name}/{kind}/value: `${}` is a transform variable, which is text; read the value of a capture",
                trans.source_name()
            )));
        }
        // Lines and counts read a capture's node, which a transform variable (text) has not.
        if trans.reads_node() && parsed.contains_key(trans.source_name()) {
            return Err(invalid(format!(
                "{path}/{name}: `${}` is a transform variable, text; line, lines and count read a capture",
                trans.source_name()
            )));
        }
    }
    let mut steps = Vec::with_capacity(parsed.len());
    let mut done = BTreeSet::new();
    for name in parsed.keys() {
        let mut stack = Vec::new();
        let mut next = name.as_str();
        while !done.contains(next) {
            let Some(trans) = parsed.get(next) else {
                break;
            };
            if stack.contains(&next) {
                return Err(invalid(format!(
                    "{path}/{next}: `{next}` has a cyclic dependency"
                )));
            }
            stack.push(next);
            next = trans.source_name();
        }
        for key in stack.into_iter().rev() {
            done.insert(key);
            steps.push((key.to_owned(), parsed[key].clone()));
        }
    }
    Ok(Transform {
        steps,
        max_output_bytes: limits.max_source_bytes,
    })
}
