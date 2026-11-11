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

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    #[test]
    fn malformed_layouts_are_rejected() {
        let reject = |layout: Value| {
            rejection(compile_typescript_rule(
                json!({ "kind": "identifier", "layout": layout }),
            ))
        };
        let at = "/clauses/0/forbid/ast/rule/layout";
        for (layout, expected) in [
            (
                json!({}),
                format!("{at} must hold span, start, before or after"),
            ),
            (
                json!({ "lines": { "eq": 0 } }),
                format!("unknown field `lines` at {at}"),
            ),
            (json!({ "span": 1 }), format!("{at}/span must be an object")),
            (
                json!({ "span": {} }),
                format!("{at}/span must set eq, min or max"),
            ),
            (
                json!({ "span": { "eq": -1 } }),
                format!("{at}/span/eq must be a non-negative integer"),
            ),
            (
                json!({ "span": { "eq": 1, "max": 2 } }),
                format!("{at}/span: eq cannot be combined with min or max"),
            ),
            (
                json!({ "start": { "min": 3, "max": 2 } }),
                format!("{at}/start: min must not exceed max"),
            ),
            (
                json!({ "before": { "breaks": { "eq": 0 } } }),
                format!("{at}/before/to must be \"token\", \"sibling\" or \"capture.<NAME>\""),
            ),
            (
                json!({ "before": { "to": "parent", "breaks": { "eq": 0 } } }),
                format!("{at}/before/to must be \"token\", \"sibling\" or \"capture.<NAME>\""),
            ),
            (
                json!({ "before": { "to": "capture.$A", "breaks": { "eq": 0 } } }),
                format!("{at}/before/to: `$A` is not a capture name (A-Z, 0-9, _, without `$`)"),
            ),
            (
                json!({ "after": { "to": "token" } }),
                format!("{at}/after must count breaks, blank or both"),
            ),
            (
                json!({ "after": { "to": "token", "comments": 1, "breaks": { "eq": 0 } } }),
                format!("{at}/after/comments must be a boolean"),
            ),
            (
                json!({ "after": { "to": "capture.A", "edge": true, "breaks": { "eq": 0 } } }),
                format!(
                    "{at}/after/edge: edge chooses the neighbouring token or sibling; a capture names its node"
                ),
            ),
            (
                json!({ "after": { "to": "token", "lines": 1, "breaks": { "eq": 0 } } }),
                format!("unknown field `lines` at {at}/after"),
            ),
        ] {
            assert_eq!(reject(layout.clone()), expected, "{layout}");
        }
    }

    #[test]
    fn a_gap_runs_to_a_single_capture_bound_where_it_runs() {
        let reject = |ast: Value| rejection(compile_typescript_ast(ast));
        let gap = |name: &str| json!({ "to": format!("capture.{name}"), "breaks": { "eq": 0 } });
        assert_eq!(
            reject(
                json!({ "rule": { "pattern": "f($A)", "not": { "pattern": "g($B)" }, "layout": { "before": gap("B") } } })
            ),
            "/clauses/0/forbid/ast/rule/layout/before/to: `$B` is never bound where this layout runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        assert_eq!(
            reject(json!({ "rule": { "pattern": "f($$$A)", "layout": { "after": gap("A") } } })),
            "/clauses/0/forbid/ast/rule/layout/after/to: `$$$A` captures a node list; a gap runs to a single node"
        );
        assert_eq!(
            reject(
                json!({ "rule": { "pattern": "f($A)", "layout": { "after": gap("T") } },
                "transform": { "T": { "substring": { "source": "$A", "startChar": 1 } } } })
            ),
            "/clauses/0/forbid/ast/rule/layout/after/to: `$T` is a transform variable, text; to reads a node"
        );
        compiled(
            json!({ "rule": { "kind": "identifier", "follows": { "pattern": "$P" }, "layout": { "before": gap("P") } } }),
        );
        // A parsed tree has no lines of its own.
        let python = clause_document(
            json!({ "id": "c", "violation": violation(), "forbid": { "ast": { "language": "python",
            "rule": { "kind": "string", "parse": { "grammar": "percentFormat", "has": { "kind": "placeholder", "layout": { "span": { "eq": 0 } } } } } } } }),
        );
        assert_eq!(
            rejection(compile_document_with(
                python,
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities")
            )),
            "/clauses/0/forbid/ast/rule/parse/has/layout: `layout` is not supported inside parse (percentFormat trees)"
        );
    }
}
