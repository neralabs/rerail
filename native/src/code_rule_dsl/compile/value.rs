use crate::code_rule_dsl::json::{bounded_text, invalid, object, reject_unknown};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{AstMatcher, Bound, Reading, ValueTest};
use crate::code_rule_dsl::model::Limits;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::value::Value;
use crate::code_rule_dsl::value::text_regex::TextRegex;
use napi::Result;
use serde_json::Value as Json;
use std::sync::Arc;

const VALUE_KEYS: &[&str] = &[
    "eq",
    "type",
    "constant",
    "startsWith",
    "regex",
    "bitsAny",
    "gt",
    "ge",
    "lt",
    "le",
    "truthy",
    "follow",
];

/// Prefixes one `startsWith` may list.
const MAX_PREFIXES: usize = 1024;

/// Type names `type` accepts: Python's `type(v).__name__`, TypeScript's `typeof` with `null` and
/// `array` told apart from `object`, and `regexp` within it.
pub(crate) const PYTHON_TYPES: &[&str] = &[
    "int", "float", "complex", "str", "bytes", "bool", "NoneType", "ellipsis", "tuple", "list",
    "set", "dict",
];
pub(crate) const TYPESCRIPT_TYPES: &[&str] = &[
    "string",
    "number",
    "bigint",
    "boolean",
    "null",
    "undefined",
    "array",
    "object",
    "regexp",
];

/// PHP `get_debug_type` names Rerail can establish for constant expressions.
pub(crate) const PHP_TYPES: &[&str] = &["int", "float", "string", "bool", "null", "array"];

fn strings<'a>(value: &'a Json, path: &str) -> Result<Vec<&'a str>> {
    let items = match value {
        Json::String(item) => return Ok(vec![item.as_str()]),
        Json::Array(items) => items,
        _ => {
            return Err(invalid(format!(
                "{path} must be a string or an array of strings"
            )));
        }
    };
    if items.is_empty() {
        return Err(invalid(format!("{path} must not be empty")));
    }
    if items.len() > MAX_PREFIXES {
        return Err(invalid(format!(
            "{path} lists {} strings; at most {MAX_PREFIXES} are allowed",
            items.len()
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .ok_or_else(|| invalid(format!("{path}/{index} must be a string")))
        })
        .collect()
}

/// The grammar-independent checks of a `value` object.
pub(super) fn validate_value(value: &Json, path: &str, limits: &Limits) -> Result<()> {
    let test = object(value, path)?;
    reject_unknown(test, VALUE_KEYS, path)?;
    if let Some(follow) = test.get("follow")
        && !follow.is_boolean()
    {
        return Err(invalid(format!("{path}/follow must be true or false")));
    }
    if test.keys().all(|key| key == "follow") {
        return Err(invalid(format!(
            "{path} must hold a predicate: eq, type, constant, startsWith, regex, bitsAny, gt, ge, lt, le or truthy"
        )));
    }
    if let Some(eq) = test.get("eq")
        && !matches!(
            eq,
            Json::String(_) | Json::Number(_) | Json::Bool(_) | Json::Null
        )
    {
        return Err(invalid(format!(
            "{path}/eq must be a string, number, boolean or null"
        )));
    }
    if let Some(types) = test.get("type") {
        strings(types, &format!("{path}/type"))?;
    }
    for key in ["constant", "truthy"] {
        if let Some(flag) = test.get(key)
            && !flag.is_boolean()
        {
            return Err(invalid(format!("{path}/{key} must be true or false")));
        }
    }
    if let Some(prefixes) = test.get("startsWith") {
        let prefix_path = format!("{path}/startsWith");
        for (index, prefix) in strings(prefixes, &prefix_path)?.into_iter().enumerate() {
            bounded_text(
                Some(&Json::String(prefix.to_owned())),
                &format!("{prefix_path}/{index}"),
                "maxRegexPatternBytes",
                limits.max_regex_pattern_bytes,
            )?;
        }
    }
    if let Some(regex) = test.get("regex") {
        let source = bounded_text(
            Some(regex),
            &format!("{path}/regex"),
            "maxRegexPatternBytes",
            limits.max_regex_pattern_bytes,
        )?;
        TextRegex::new(source)
            .ok_or_else(|| invalid(format!("{path}/regex: Parsing text matcher fails.")))?;
    }
    if let Some(bits) = test.get("bitsAny")
        && bits.as_u64().is_none_or(|bits| bits == 0)
    {
        return Err(invalid(format!(
            "{path}/bitsAny must be a positive integer"
        )));
    }
    for key in ["gt", "ge", "lt", "le"] {
        if let Some(bound) = test.get(key)
            && bound.as_f64().is_none_or(f64::is_nan)
        {
            return Err(invalid(format!("{path}/{key} must be a number")));
        }
    }
    Ok(())
}

/// A JSON scalar as a value of the grammar's language: a JSON integer is a Python `int`, a
/// JavaScript number, a PHP `int` within 64 bits (a `float` past them); strings are `str`, UTF-16
/// or PHP's UTF-8 bytes.
fn literal(value: &Json, language: GrammarDialect) -> Value {
    let family = Family::of(language);
    match value {
        Json::String(text) => match family {
            Family::Python => Value::Str(text.clone()),
            Family::Ecma => Value::Utf16(text.encode_utf16().collect()),
            Family::Php | Family::Blade => Value::PhpStr(text.as_bytes().to_vec()),
        },
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Null => Value::None,
        Json::Number(number) => match (family, number.as_i64(), number.as_u64()) {
            (Family::Python | Family::Php | Family::Blade, Some(integer), _) => Value::int(integer),
            (Family::Python, None, Some(integer)) => Value::int(integer),
            _ => Value::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        _ => Value::Unknown,
    }
}

/// Compiles a validated `value` for one grammar.
pub(super) fn compile_value(
    value: &Json,
    language: GrammarDialect,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<AstMatcher> {
    let mut reading = Reading::new(language, options.cloned(), python.cloned());
    if value.get("follow").and_then(Json::as_bool) == Some(true) {
        reading = reading.following();
    }
    compile_value_with(value, language, path, reading)
}

/// Compiles a validated `value` whose nodes are read as `reading` reads them.
pub(super) fn compile_value_with(
    value: &Json,
    language: GrammarDialect,
    path: &str,
    reading: Reading,
) -> Result<AstMatcher> {
    let (known, language_name) = match Family::of(language) {
        Family::Python => (PYTHON_TYPES, "Python"),
        Family::Ecma => (TYPESCRIPT_TYPES, "TypeScript"),
        // Blade's values are those of the PHP its template embeds.
        Family::Php | Family::Blade => (PHP_TYPES, "PHP"),
    };
    let types = value
        .get("type")
        .map(|types| strings(types, &format!("{path}/type")))
        .transpose()?
        .unwrap_or_default();
    for (index, name) in types.iter().enumerate() {
        if !known.contains(name) {
            return Err(invalid(format!(
                "{path}/type/{index}: `{name}` is not a {language_name} type name; expected one of {}",
                known.join(", ")
            )));
        }
    }
    let starts_with = value
        .get("startsWith")
        .map(|prefixes| strings(prefixes, &format!("{path}/startsWith")))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|prefix| literal(&Json::String(prefix.to_owned()), language))
        .collect();
    let regex = value
        .get("regex")
        .and_then(Json::as_str)
        .map(|source| {
            TextRegex::new(source)
                .ok_or_else(|| invalid(format!("{path}/regex: Parsing text matcher fails.")))
        })
        .transpose()?;
    let bounds = [
        ("gt", Bound::Gt),
        ("ge", Bound::Ge),
        ("lt", Bound::Lt),
        ("le", Bound::Le),
    ]
    .into_iter()
    .filter_map(|(key, bound)| {
        value
            .get(key)
            .map(|limit| (bound, literal(limit, language)))
    })
    .collect();
    Ok(AstMatcher::Semantic(Semantic::Value(Box::new(ValueTest {
        reading,
        eq: value.get("eq").map(|eq| literal(eq, language)),
        types: types.into_iter().map(str::to_owned).collect(),
        constant: value.get("constant").and_then(Json::as_bool),
        starts_with,
        regex,
        bits_any: value.get("bitsAny").and_then(Json::as_u64),
        bounds,
        truthy: value.get("truthy").and_then(Json::as_bool),
    }))))
}

#[cfg(test)]
mod tests;
