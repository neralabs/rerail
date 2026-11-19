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
use super::value::{Obj, Value, q};

/// Whether `doc` is a JSON tool-call rule.
pub fn is_tool_call_rule(doc: &Value) -> bool {
    doc.get("sample").and_then(|s| s.get("boundary")).and_then(Value::as_str) == Some("tool-call")
}

/// The `.rerail` text of the tool-call rule `doc`, in the engine's dialect.
pub fn translate(doc: &Value) -> R<String> {
    let id = doc.get("id").and_then(Value::as_str).ok_or("a tool-call rule has an `id`")?;
    let tool = tool(doc)?;
    let condition = doc.get("match").ok_or("a tool-call rule has a `match`")?;
    let body = formula(condition, "/match", &mut 0, 1)?;
    let rel = ident(id);
    Ok(format!(
        "{}\n// {rel}(c): a finding at the tool call c, from the JSON rule's `match`.\n{}\n{rel}(c) :-\n    tool_call(c, {tool}),\n    {body}.\n",
        header(doc, id),
        violation(doc, id, &rel),
    ))
}

/// The tool the rule's `sample.source` reads: one by name (`tool:Bash`), or any (`tool:*`, `_`).
fn tool(doc: &Value) -> R<String> {
    let source = doc.get("sample").and_then(|s| s.get("source")).and_then(Value::as_str).unwrap_or_default();
    match source.strip_prefix("tool:") {
        Some("*") => Ok("_".to_owned()),
        Some(name) if !name.is_empty() => Ok(q(name)),
        _ => Err(format!("sample.source {source:?}: a tool-call rule reads tool:<name> or tool:*")),
    }
}

/// The policy's directives: its id and revision (1 where the rule has none), its mode, its scope.
fn header(doc: &Value, id: &str) -> String {
    let revision = match doc.get("revision") {
        Some(Value::Int(n)) => n.to_string(),
        _ => "1".to_owned(),
    };
    let mode = doc.get("mode").and_then(Value::as_str).unwrap_or("log");
    format!(".policy {} revision {revision}\n.mode {mode}\n.scope tool_calls\n", q(id))
}

/// The finding's `.violation`: at the call, the rule's id its code, its `action.content` the message, whose braces
/// are text, not placeholders (`findings::message_parts`).
fn violation(doc: &Value, id: &str, rel: &str) -> String {
    let code = if rel == id { String::new() } else { format!(" code {}", q(id)) };
    let message = doc.get("action").and_then(|a| a.get("content")).and_then(Value::as_str).unwrap_or_default();
    format!(".violation {rel}(at: symbol){code} {}", q(&message.replace('{', "{{").replace('}', "}}")))
}

/// A condition as a body formula over the call `c`, each leaf with a fresh string variable; `depth` the indent of
/// its continuation lines.
fn formula(condition: &Value, path: &str, vars: &mut usize, depth: usize) -> R<String> {
    let o = condition.as_obj().ok_or_else(|| format!("{path}: a condition is an object"))?;
    match o.0.as_slice() {
        [(key, items)] if key == "all" || key == "any" => group(key, items, path, vars, depth),
        [(key, inner)] if key == "not" => negation(inner, path, vars, depth),
        _ => leaf(o, path, vars),
    }
}

/// `all` (a conjunction) or `any` (a disjunction) of its conditions, in parentheses.
fn group(key: &str, items: &Value, path: &str, vars: &mut usize, depth: usize) -> R<String> {
    let items = items.as_list().filter(|l| !l.is_empty()).ok_or_else(|| format!("{path}/{key}: a non-empty list"))?;
    let parts = items.iter().enumerate().map(|(k, c)| formula(c, &format!("{path}/{key}/{k}"), vars, depth + 1)).collect::<R<Vec<_>>>()?;
    let (indent, outer) = ("    ".repeat(depth + 1), "    ".repeat(depth));
    let separator = if key == "all" { format!(",\n{indent}") } else { format!("\n{outer};\n{indent}") };
    Ok(format!("(\n{indent}{}\n{outer})", parts.join(&separator)))
}

/// `not`: the negation of its condition, parenthesised where it is not already.
fn negation(inner: &Value, path: &str, vars: &mut usize, depth: usize) -> R<String> {
    let inner = formula(inner, &format!("{path}/not"), vars, depth)?;
    Ok(if inner.starts_with('(') { format!("!{inner}") } else { format!("!({inner})") })
}

/// `{field: "input.<path>", regex}`: the argument at the path, matched where the regex finds it.
fn leaf(o: &Obj, path: &str, vars: &mut usize) -> R<String> {
    let field = o.get("field").and_then(Value::as_str).ok_or_else(|| format!("{path}: a condition is `field` with a test, or `all`, `any` or `not`"))?;
    let key = field.strip_prefix("input.").filter(|k| !k.is_empty()).ok_or_else(|| format!("{path}/field {field:?}: not translated (the shipped tool-call rules read `input.<path>`)"))?;
    let re = o.get("regex").and_then(Value::as_str).filter(|_| o.key_set_is(&["field", "regex"])).ok_or_else(|| format!("{path}: not translated (the shipped tool-call rules test `input.<path>` with `regex`)"))?;
    *vars += 1;
    let s = format!("s{vars}");
    Ok(format!("tool_input(c, {}, {s}), match({}, {s})", q(key), q(&format!("(?s:.*)(?:{re})(?s:.*)"))))
}
