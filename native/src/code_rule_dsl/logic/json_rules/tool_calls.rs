//! A JSON tool-call rule (a conversation rule whose `sample.boundary` is `tool-call`, `docs/tool-call-rules.md`) as a
//! `.scope tool_calls` policy (`docs/rule-syntax/tool-calls.md`): its `match` a rule over the run's tool calls, its
//! finding at the call.
//!
//! It translates the forms the shipped tool-call rules use: `field: "input.<path>"` with `regex`, and `all`, `any` and
//! `not` over them; `sample.source` `tool:*`, or `tool:<name>` for one tool. A leaf is `tool_input(c, "<path>", s)`
//! and `match` of the regex on s. The JSON's regex searches (`rules.rs`, `Regex::is_match`) and `match` is whole-string,
//! so the pattern is `(?s:.*)(?:R)(?s:.*)`: its anchors still read the whole argument, and a newline is any character
//! around it. A missing or non-string argument has no `tool_input` row, so a leaf on it is false, as in the product.

use super::lower::{R, ident};
use super::value::{Value, q};

/// Whether `doc` is a JSON tool-call rule.
pub fn is_tool_call_rule(doc: &Value) -> bool {
    doc.get("sample").and_then(|s| s.get("boundary")).and_then(Value::as_str) == Some("tool-call")
}

/// The `.rerail` text of the tool-call rule `doc`, in the engine's dialect.
pub fn translate(doc: &Value) -> R<String> {
    let id = doc.get("id").and_then(Value::as_str).ok_or("a tool-call rule has an `id`")?;
    let source = doc.get("sample").and_then(|s| s.get("source")).and_then(Value::as_str).unwrap_or_default();
    let tool = match source.strip_prefix("tool:") {
        Some("*") => "_".to_owned(),
        Some(name) if !name.is_empty() => q(name),
        _ => return Err(format!("sample.source {source:?}: a tool-call rule reads tool:<name> or tool:*")),
    };
    let condition = doc.get("match").ok_or("a tool-call rule has a `match`")?;
    let mut vars = 0;
    let body = formula(condition, "/match", &mut vars, 1)?;
    let mode = doc.get("mode").and_then(Value::as_str).unwrap_or("log");
    let message = doc.get("action").and_then(|a| a.get("content")).and_then(Value::as_str).unwrap_or_default();
    let rel = ident(id);
    let code = if rel == id { String::new() } else { format!(" code {}", q(id)) };
    // A message's braces are text, not placeholders (`findings::message_parts`).
    let message = message.replace('{', "{{").replace('}', "}}");
    Ok(format!(
        ".policy {} revision {}\n.mode {mode}\n.scope tool_calls\n\n// {rel}(c): a finding at the tool call c, from the JSON rule's `match`.\n.violation {rel}(at: symbol){code} {}\n{rel}(c) :-\n    tool_call(c, {tool}),\n    {body}.\n",
        q(id),
        doc.get("revision").and_then(|r| match r {
            Value::Int(n) => Some(n.to_string()),
            _ => None,
        }).unwrap_or_else(|| "1".to_owned()),
        q(&message),
    ))
}

/// A condition as a body formula over the call `c`, each leaf with a fresh string variable; `depth` the indent of
/// its continuation lines.
fn formula(condition: &Value, path: &str, vars: &mut usize, depth: usize) -> R<String> {
    let o = condition.as_obj().ok_or_else(|| format!("{path}: a condition is an object"))?;
    let indent = "    ".repeat(depth + 1);
    let each = |key: &str, vars: &mut usize| -> R<Vec<String>> {
        let items = o.get(key).and_then(Value::as_list).filter(|l| !l.is_empty()).ok_or_else(|| format!("{path}/{key}: a non-empty list"))?;
        items.iter().enumerate().map(|(k, c)| formula(c, &format!("{path}/{key}/{k}"), vars, depth + 1)).collect()
    };
    if o.key_set_is(&["all"]) {
        return Ok(format!("(\n{indent}{}\n{})", each("all", vars)?.join(&format!(",\n{indent}")), "    ".repeat(depth)));
    }
    if o.key_set_is(&["any"]) {
        return Ok(format!("(\n{indent}{}\n{})", each("any", vars)?.join(&format!("\n{};\n{indent}", "    ".repeat(depth))), "    ".repeat(depth)));
    }
    if o.key_set_is(&["not"]) {
        let inner = formula(o.get("not").expect("not"), &format!("{path}/not"), vars, depth)?;
        return Ok(format!("!{}", if inner.starts_with('(') { inner } else { format!("({inner})") }));
    }
    let field = o.get("field").and_then(Value::as_str).ok_or_else(|| format!("{path}: a condition is `field` with a test, or `all`, `any` or `not`"))?;
    let key = field.strip_prefix("input.").filter(|k| !k.is_empty()).ok_or_else(|| format!("{path}/field {field:?}: not translated (the shipped tool-call rules read `input.<path>`)"))?;
    let (Some(re), true) = (o.get("regex").and_then(Value::as_str), o.key_set_is(&["field", "regex"])) else {
        return Err(format!("{path}: not translated (the shipped tool-call rules test `input.<path>` with `regex`)"));
    };
    *vars += 1;
    let s = format!("s{vars}");
    Ok(format!("tool_input(c, {}, {s}), match({}, {s})", q(key), q(&format!("(?s:.*)(?:{re})(?s:.*)"))))
}
