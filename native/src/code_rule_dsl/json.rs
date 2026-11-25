use ast_grep_core::meta_var::MetaVariable;
use napi::{Error, Result, Status};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn meta_name(variable: &MetaVariable) -> String {
    match variable {
        MetaVariable::Capture(name, _) | MetaVariable::MultiCapture(name) => name.clone(),
        MetaVariable::Dropped(_) => "_".to_owned(),
        MetaVariable::Multiple => "$$$".to_owned(),
    }
}

pub fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_value(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_value).collect()),
        other => other,
    }
}

pub fn reject_unknown(object: &Map<String, Value>, allowed: &[&str], path: &str) -> Result<()> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid(format!("unknown field `{key}` at {path}")));
    }
    Ok(())
}

pub fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| invalid(format!("{path} must be an object")))
}

pub fn required_str<'a>(value: &'a Value, key: &str, path: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{path}/{key} must be a string")))
}

pub fn non_empty_str<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| invalid(format!("{path} must be a non-empty string")))
}

/// A pattern source whose UTF-8 length is within the named `capabilities.limits` entry.
pub fn bounded_text<'a>(
    value: Option<&'a Value>,
    path: &str,
    limit_name: &str,
    maximum: usize,
) -> Result<&'a str> {
    let text = non_empty_str(value, path)?;
    if text.len() > maximum {
        return Err(invalid(format!(
            "{path} is {} bytes; capabilities.limits.{limit_name} allows at most {maximum}",
            text.len()
        )));
    }
    Ok(text)
}

pub fn required_u64(value: &Value, key: &str, path: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("{path}/{key} must be a non-negative integer")))
}

pub fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
    hex
}

pub fn invalid(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

pub fn json_error(error: serde_json::Error) -> Error {
    invalid(error.to_string())
}

/// A JSON-with-comments text as plain JSON, as TypeScript reads a configuration file (`tsconfig.json`): without a
/// leading byte-order mark, comments and trailing commas. Comments go first, so a trailing comma before a comment and
/// a closing bracket is still dropped. The module graph, the import resolver and the repository's `tsconfig` rows
/// read it alike.
pub fn strip_jsonc(text: &str) -> String {
    // TypeScript drops a leading byte-order mark, as editors on Windows write one.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let without_comments = strip_outside_strings(text, |bytes, at| match bytes[at..] {
        [b'/', b'/', ..] => Some(
            bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |end| at + end),
        ),
        [b'/', b'*', ..] => Some(
            bytes[at + 2..]
                .windows(2)
                .position(|pair| pair == b"*/")
                .map_or(bytes.len(), |end| at + 2 + end + 2),
        ),
        _ => None,
    });
    strip_outside_strings(&without_comments, |bytes, at| {
        (bytes[at] == b','
            && matches!(
                bytes[at + 1..]
                    .iter()
                    .find(|byte| !byte.is_ascii_whitespace()),
                Some(b'}' | b']')
            ))
        .then_some(at + 1)
    })
}

/// `text` without the spans `skip` names outside string literals: given the bytes and an offset,
/// it returns where a span to drop starting there ends.
fn strip_outside_strings(text: &str, skip: impl Fn(&[u8], usize) -> Option<usize>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut kept = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            at += 1;
            while at < bytes.len() && bytes[at] != b'"' {
                at += if bytes[at] == b'\\' { 2 } else { 1 };
            }
            at = (at + 1).min(bytes.len());
        } else if let Some(end) = skip(bytes, at) {
            out.push_str(&text[kept..at]);
            out.push(' ');
            at = end.min(bytes.len());
            kept = at;
        } else {
            at += 1;
        }
    }
    out.push_str(&text[kept..]);
    out
}
