use ast_grep_core::meta_var::MetaVariable;
use napi::{Error, Result, Status};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) fn meta_name(variable: &MetaVariable) -> String {
    match variable {
        MetaVariable::Capture(name, _) | MetaVariable::MultiCapture(name) => name.clone(),
        MetaVariable::Dropped(_) => "_".to_owned(),
        MetaVariable::Multiple => "$$$".to_owned(),
    }
}

pub(super) fn canonicalize_value(value: Value) -> Value {
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

pub(super) fn reject_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
    path: &str,
) -> Result<()> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid(format!("unknown field `{key}` at {path}")));
    }
    Ok(())
}

pub(super) fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| invalid(format!("{path} must be an object")))
}

pub(super) fn required_str<'a>(value: &'a Value, key: &str, path: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{path}/{key} must be a string")))
}

pub(super) fn non_empty_str<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| invalid(format!("{path} must be a non-empty string")))
}

/// A pattern source whose UTF-8 length is within the named `capabilities.limits` entry.
pub(super) fn bounded_text<'a>(
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

pub(super) fn required_u64(value: &Value, key: &str, path: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("{path}/{key} must be a non-negative integer")))
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn invalid(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

pub(super) fn json_error(error: serde_json::Error) -> Error {
    invalid(error.to_string())
}
