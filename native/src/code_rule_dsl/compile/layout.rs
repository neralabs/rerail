//! `layout`: the node's lines, its first byte, and the lines between it and a neighbour.

use super::parse::capture_name;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{AstMatcher, Gap, Layout, LineContext, LineCount, Neighbour};
use napi::Result;
use serde_json::Value;

/// A count of lines or bytes: `{ eq }` or `{ min, max }`, non-negative integers.
fn line_count(value: &Value, path: &str) -> Result<LineCount> {
    let bounds = object(value, path)?;
    reject_unknown(bounds, &["eq", "min", "max"], path)?;
    let number = |key: &str| -> Result<Option<u64>> {
        bounds
            .get(key)
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| invalid(format!("{path}/{key} must be a non-negative integer")))
            })
            .transpose()
    };
    let (eq, min, max) = (number("eq")?, number("min")?, number("max")?);
    match (eq, min, max) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(invalid(format!(
            "{path}: eq cannot be combined with min or max"
        ))),
        (Some(eq), None, None) => Ok(LineCount {
            min: eq,
            max: Some(eq),
        }),
        (None, None, None) => Err(invalid(format!("{path} must set eq, min or max"))),
        (None, min, max) => {
            let min = min.unwrap_or(0);
            if max.is_some_and(|max| min > max) {
                return Err(invalid(format!("{path}: min must not exceed max")));
            }
            Ok(LineCount { min, max })
        }
    }
}

/// `before` or `after`: `to`, `comments`, `edge`, `breaks` and `blank`.
fn gap(value: &Value, path: &str) -> Result<Gap> {
    let gap = object(value, path)?;
    reject_unknown(gap, &["to", "comments", "edge", "breaks", "blank"], path)?;
    let to = match gap.get("to").and_then(Value::as_str) {
        Some("token") => Neighbour::Token,
        Some("sibling") => Neighbour::Sibling,
        Some(to) if to.starts_with("capture.") => Neighbour::Capture(
            capture_name(
                &Value::String(to["capture.".len()..].to_owned()),
                &format!("{path}/to"),
            )?
            .to_owned(),
        ),
        _ => {
            return Err(invalid(format!(
                "{path}/to must be \"token\", \"sibling\" or \"capture.<NAME>\""
            )));
        }
    };
    let flag = |key: &str| -> Result<bool> {
        match gap.get(key) {
            None => Ok(false),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(_) => Err(invalid(format!("{path}/{key} must be a boolean"))),
        }
    };
    let (comments, edge) = (flag("comments")?, flag("edge")?);
    if matches!(to, Neighbour::Capture(_)) {
        for key in ["comments", "edge"] {
            if gap.contains_key(key) {
                return Err(invalid(format!(
                    "{path}/{key}: {key} chooses the neighbouring token or sibling; a capture names its node"
                )));
            }
        }
    }
    let count = |key: &str| {
        gap.get(key)
            .map(|value| line_count(value, &format!("{path}/{key}")))
            .transpose()
    };
    let (breaks, blank) = (count("breaks")?, count("blank")?);
    if breaks.is_none() && blank.is_none() {
        return Err(invalid(format!("{path} must count breaks, blank or both")));
    }
    Ok(Gap {
        to,
        comments,
        edge,
        breaks,
        blank,
    })
}

fn layout(value: &Value, path: &str) -> Result<Layout> {
    let layout = object(value, path)?;
    reject_unknown(
        layout,
        &[
            "span",
            "start",
            "before",
            "after",
            "line",
            "previousLine",
            "context",
        ],
        path,
    )?;
    if layout.is_empty() {
        return Err(invalid(format!(
            "{path} must hold span, start, before or after"
        )));
    }
    let count = |key: &str| {
        layout
            .get(key)
            .map(|value| line_count(value, &format!("{path}/{key}")))
            .transpose()
    };
    let side = |key: &str| {
        layout
            .get(key)
            .map(|value| gap(value, &format!("{path}/{key}")))
            .transpose()
    };
    let text = |key: &str| -> Result<Option<regex::Regex>> {
        layout
            .get(key)
            .map(|value| {
                let pattern = value
                    .as_str()
                    .ok_or_else(|| invalid(format!("{path}/{key} must be a regex string")))?;
                regex::Regex::new(pattern)
                    .map_err(|error| invalid(format!("{path}/{key}: {error}")))
            })
            .transpose()
    };
    Ok(Layout {
        span: count("span")?,
        start: count("start")?,
        before: side("before")?,
        after: side("after")?,
        line: text("line")?,
        previous_line: text("previousLine")?,
        context: layout
            .get("context")
            .map(|value| -> Result<LineContext> {
                let at = format!("{path}/context");
                let spec = object(value, &at)?;
                reject_unknown(spec, &["before", "after", "through", "regex"], &at)?;
                let number = |key: &str| -> Result<usize> {
                    match spec.get(key) {
                        None => Ok(0),
                        Some(v) => v
                            .as_u64()
                            .filter(|n| *n <= 64)
                            .map(|n| n as usize)
                            .ok_or_else(|| {
                                invalid(format!("{at}/{key} must be an integer from 0 to 64"))
                            }),
                    }
                };
                let regex = |key: &str| -> Result<regex::Regex> {
                    let pattern = spec
                        .get(key)
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid(format!("{at}/{key} must be a regex string")))?;
                    regex::Regex::new(pattern).map_err(|e| invalid(format!("{at}/{key}: {e}")))
                };
                Ok(LineContext {
                    before: number("before")?,
                    after: number("after")?,
                    through: spec.get("through").map(|_| regex("through")).transpose()?,
                    regex: regex("regex")?,
                })
            })
            .transpose()?,
    })
}

pub(super) fn validate_layout(
    value: &Value,
    path: &str,
    limits: &crate::code_rule_dsl::model::Limits,
) -> Result<()> {
    for key in ["line", "previousLine"] {
        if value
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|pattern| pattern.len() > limits.max_regex_pattern_bytes)
        {
            return Err(invalid(format!(
                "{path}/{key}: regex exceeds maxRegexPatternBytes"
            )));
        }
    }
    for key in ["regex", "through"] {
        if value
            .get("context")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .is_some_and(|p| p.len() > limits.max_regex_pattern_bytes)
        {
            return Err(invalid(format!(
                "{path}/context/{key}: regex exceeds maxRegexPatternBytes"
            )));
        }
    }
    layout(value, path).map(|_| ())
}

pub(super) fn compile_layout(value: &Value, path: &str) -> Result<AstMatcher> {
    Ok(AstMatcher::Semantic(Semantic::Layout(Box::new(layout(
        value, path,
    )?))))
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
