//! Document options (a Rerail extension): typed values a `define` declares with defaults and a
//! `config` may set, which the document reads with `$option` before it is validated, and which an
//! ast query's `when.option` gates on. A port that differs by configuration is one document; a
//! configuration is a `config` (a variant's patch), and the catalog stores the document resolved.
//!
//! `policy-schema.ts` resolves documents the same way (`resolvePolicyOptions`); the two are tested
//! in lockstep.

use crate::policy_dsl::json::{invalid, object, reject_unknown};
use napi::Result;
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

/// What an option holds.
#[derive(Clone, Copy, PartialEq)]
enum OptionType {
    Boolean,
    String,
    Number,
    Integer,
    Array(ItemType),
}

#[derive(Clone, Copy, PartialEq)]
enum ItemType {
    String,
    Number,
    Integer,
    Boolean,
}

/// One declared option: its type, allowed values and default.
struct Declaration {
    kind: OptionType,
    nullable: bool,
    allowed: Option<Vec<Value>>,
}

/// The document's options, each with its effective value (its `config` value, else its default).
#[derive(Default)]
pub(super) struct Options {
    values: BTreeMap<String, Value>,
}

const DECLARATION_KEYS: &[&str] = &[
    "type",
    "items",
    "enum",
    "nullable",
    "default",
    "description",
];
const REFERENCE_KEYS: &[&str] = &[
    "$option", "cases", "default", "escape", "map", "join", "template", "spread",
];

/// Whether a name may name an option: a letter or `_`, then letters, digits, `_`, `-` and `.`.
fn is_option_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn declaration(value: &Value, path: &str) -> Result<(Declaration, Value)> {
    let fields = object(value, path)?;
    reject_unknown(fields, DECLARATION_KEYS, path)?;
    let item = |name: &str| match name {
        "string" => Some(ItemType::String),
        "number" => Some(ItemType::Number),
        "integer" => Some(ItemType::Integer),
        "boolean" => Some(ItemType::Boolean),
        _ => None,
    };
    let kind = match fields.get("type").and_then(Value::as_str) {
        Some("boolean") => OptionType::Boolean,
        Some("string") => OptionType::String,
        Some("number") => OptionType::Number,
        Some("integer") => OptionType::Integer,
        Some("array") => OptionType::Array(match fields.get("items") {
            None => ItemType::String,
            Some(items) => items.as_str().and_then(item).ok_or_else(|| {
                invalid(format!(
                    "{path}/items must equal string, number, integer or boolean"
                ))
            })?,
        }),
        _ => {
            return Err(invalid(format!(
                "{path}/type must equal boolean, string, number, integer or array"
            )));
        }
    };
    if fields.contains_key("items") && !matches!(kind, OptionType::Array(_)) {
        return Err(invalid(format!(
            "{path}/items: only an array option has items"
        )));
    }
    let nullable = match fields.get("nullable") {
        None => false,
        Some(Value::Bool(nullable)) => *nullable,
        Some(_) => return Err(invalid(format!("{path}/nullable must be true or false"))),
    };
    let mut declared = Declaration {
        kind,
        nullable,
        allowed: None,
    };
    if let Some(allowed) = fields.get("enum") {
        let values = allowed
            .as_array()
            .filter(|values| !values.is_empty())
            .ok_or_else(|| invalid(format!("{path}/enum must be a non-empty array")))?;
        let mut canonical = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            let value = canonical_value(value, &format!("{path}/enum/{index}"))?;
            let scalar = match kind {
                OptionType::Array(items) => item_matches(items, &value),
                _ => scalar_matches(kind, &value),
            };
            if !scalar {
                return Err(invalid(format!(
                    "{path}/enum/{index} is not a value of the option's type"
                )));
            }
            canonical.push(value);
        }
        declared.allowed = Some(canonical);
    }
    let default_path = format!("{path}/default");
    let default = canonical_value(
        fields
            .get("default")
            .ok_or_else(|| invalid(format!("{path}/default is required")))?,
        &default_path,
    )?;
    check_value(&declared, &default, &default_path)?;
    Ok((declared, default))
}

/// JavaScript's `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// An option's number as every entrypoint reads it: the IEEE-754 double JavaScript parses, spelled
/// as an integer when it is integral (`1.0` is `1`, `-0` is `0`). `None` for an integer beyond
/// ±`MAX_SAFE_INTEGER`: a parsed JavaScript document has already rounded such a number (as
/// 9007199254740993 to 9007199254740992), so no entrypoint can tell which one was written.
/// `policy-options.ts` (`checkNumbers`) refuses the same numbers.
fn canonical_number(number: &Number) -> Option<Number> {
    let safe = |integer: i64| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&integer);
    if let Some(integer) = number.as_i64() {
        return safe(integer).then(|| number.clone());
    }
    if number.is_u64() {
        return None;
    }
    let double = number.as_f64().filter(|double| double.is_finite())?;
    if double.fract() != 0.0 {
        return Some(number.clone());
    }
    // Integral and finite: exact as an i64 within the safe range, and refused beyond it.
    let integer = (double.abs() <= MAX_SAFE_INTEGER as f64).then_some(double as i64)?;
    Some(Number::from(integer))
}

/// A declared or configured option value with its numbers canonical (`canonical_number`); an
/// integer beyond JavaScript's safe range is refused.
fn canonical_value(value: &Value, path: &str) -> Result<Value> {
    match value {
        Value::Number(number) => canonical_number(number).map(Value::Number).ok_or_else(|| {
            invalid(format!(
                "{path}: {} is not exactly representable as a JavaScript number",
                json_text(value)
            ))
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| canonical_value(item, &format!("{path}/{index}")))
            .collect::<Result<_>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

/// A number's text as JavaScript's `String(number)` writes it: the shortest digits that
/// round-trip, positional from 1e-6 up to 1e21 and exponential (`1e+21`, `1.5e-7`) outside.
fn number_text(number: &Number) -> String {
    let Some(double) = number.as_f64().filter(|_| number.is_f64()) else {
        return number.to_string();
    };
    // Rust's `{:e}` writes the shortest round-trip digits as `d[.ddd]e<exponent>`.
    let scientific = format!("{:e}", double.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("`{:e}` writes an exponent");
    let digits = mantissa.replace('.', "");
    let count = digits.len() as i32;
    let point = exponent
        .parse::<i32>()
        .expect("`{:e}` writes an integer exponent")
        + 1;
    let text = if count <= point && point <= 21 {
        format!("{digits}{}", "0".repeat((point - count) as usize))
    } else if 0 < point && point <= 21 {
        let (whole, fraction) = digits.split_at(point as usize);
        format!("{whole}.{fraction}")
    } else if -6 < point && point <= 0 {
        format!("0.{}{digits}", "0".repeat(-point as usize))
    } else {
        let (first, rest) = digits.split_at(1);
        let dot = if rest.is_empty() { "" } else { "." };
        let sign = if point > 0 { "+" } else { "-" };
        format!("{first}{dot}{rest}e{sign}{}", (point - 1).abs())
    };
    if double < 0.0 {
        format!("-{text}")
    } else {
        text
    }
}

/// A value's JSON text as JavaScript's `JSON.stringify` writes it.
fn json_text(value: &Value) -> String {
    match value {
        Value::Number(number) => number_text(number),
        Value::Array(items) => {
            let items = items.iter().map(json_text).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

fn item_matches(items: ItemType, value: &Value) -> bool {
    match items {
        ItemType::String => value.is_string(),
        ItemType::Number => value.is_number(),
        ItemType::Integer => value.as_f64().is_some_and(|number| number.fract() == 0.0),
        ItemType::Boolean => value.is_boolean(),
    }
}

fn scalar_matches(kind: OptionType, value: &Value) -> bool {
    match kind {
        OptionType::Boolean => item_matches(ItemType::Boolean, value),
        OptionType::String => item_matches(ItemType::String, value),
        OptionType::Number => item_matches(ItemType::Number, value),
        OptionType::Integer => item_matches(ItemType::Integer, value),
        OptionType::Array(_) => false,
    }
}

/// Checks a value against an option's declaration.
fn check_value(declared: &Declaration, value: &Value, path: &str) -> Result<()> {
    if value.is_null() {
        return if declared.nullable {
            Ok(())
        } else {
            Err(invalid(format!(
                "{path}: null is not allowed; the option is not nullable"
            )))
        };
    }
    let allowed = |value: &Value| {
        declared
            .allowed
            .as_ref()
            .is_none_or(|allowed| allowed.contains(value))
    };
    match declared.kind {
        OptionType::Array(items) => {
            let values = value
                .as_array()
                .ok_or_else(|| invalid(format!("{path} must be an array")))?;
            for (index, item) in values.iter().enumerate() {
                if !item_matches(items, item) || !allowed(item) {
                    return Err(invalid(format!(
                        "{path}/{index} is not an allowed item of the option"
                    )));
                }
            }
        }
        kind => {
            if !scalar_matches(kind, value) {
                return Err(invalid(format!(
                    "{path} is not a value of the option's type"
                )));
            }
            if !allowed(value) {
                return Err(invalid(format!(
                    "{path} is not one of the option's enum values"
                )));
            }
        }
    }
    Ok(())
}

/// The key a value selects in `cases`: a string's text, anything else's JSON text as JavaScript
/// writes it (`json_text`). Option values are canonical, so `1.0` selects the case keyed `1`, as
/// on the TypeScript path; a key spelled `1.0` is selected by no value.
fn case_key(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => json_text(other),
    }
}

/// A value as text for `map`, `join` and `template`: a string's text, a scalar's JSON text as
/// JavaScript writes it.
fn text(value: &Value, path: &str) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(_) | Value::Bool(_) => Ok(json_text(value)),
        _ => Err(invalid(format!(
            "{path}: the option's value is not text (a string, a number or a boolean)"
        ))),
    }
}

/// What a `$option` reference stands for: a value, or items spread into the array holding it.
enum Resolved {
    Value(Value),
    Spread(Vec<Value>),
}

impl Options {
    fn get(&self, name: &str, path: &str) -> Result<&Value> {
        self.values.get(name).ok_or_else(|| {
            invalid(format!(
                "{path}: `{name}` is not an option this document declares"
            ))
        })
    }

    /// Replaces every `$option` reference in `value` (at `path`).
    fn substitute(&self, value: &Value, path: &str) -> Result<Value> {
        if value.get("$option").is_some() && value.get("spread") == Some(&Value::Bool(true)) {
            return Err(invalid(format!(
                "{path}/spread: only an array's item may spread"
            )));
        }
        match self.reference(value, path)? {
            Some(Resolved::Value(value)) => Ok(value),
            Some(Resolved::Spread(_)) => Err(invalid(format!(
                "{path}/spread: only an array's item may spread"
            ))),
            None => self.substitute_inner(value, path),
        }
    }

    fn substitute_inner(&self, value: &Value, path: &str) -> Result<Value> {
        match value {
            Value::Object(fields) => {
                let mut out = Map::with_capacity(fields.len());
                for (key, child) in fields {
                    out.insert(
                        key.clone(),
                        self.substitute(child, &format!("{path}/{key}"))?,
                    );
                }
                Ok(Value::Object(out))
            }
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let item_path = format!("{path}/{index}");
                    match self.reference(item, &item_path)? {
                        Some(Resolved::Value(value)) => out.push(value),
                        Some(Resolved::Spread(values)) => out.extend(values),
                        None => out.push(self.substitute_inner(item, &item_path)?),
                    }
                }
                Ok(Value::Array(out))
            }
            other => Ok(other.clone()),
        }
    }

    /// A `$option` reference's value, or `None` where `value` is no reference.
    fn reference(&self, value: &Value, path: &str) -> Result<Option<Resolved>> {
        let Some(fields) = value
            .as_object()
            .filter(|fields| fields.contains_key("$option"))
        else {
            return Ok(None);
        };
        reject_unknown(fields, REFERENCE_KEYS, path)?;
        let name = fields
            .get("$option")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(format!("{path}/$option must name an option")))?;
        let option = self.get(name, &format!("{path}/$option"))?;
        let spread = match fields.get("spread") {
            None => false,
            Some(Value::Bool(spread)) => *spread,
            Some(_) => return Err(invalid(format!("{path}/spread must be true or false"))),
        };
        let resolved = if let Some(cases) = fields.get("cases") {
            let cases = object(cases, &format!("{path}/cases"))?;
            for key in ["escape", "map", "join", "template"] {
                if fields.contains_key(key) {
                    return Err(invalid(format!(
                        "{path}/{key}: cases selects a value; it has no {key}"
                    )));
                }
            }
            let key = case_key(option);
            let (chosen, chosen_path) = match cases.get(&key) {
                Some(chosen) => (chosen, format!("{path}/cases/{key}")),
                None => (
                    fields.get("default").ok_or_else(|| {
                        invalid(format!("{path}/cases: no case for `{key}` and no default"))
                    })?,
                    format!("{path}/default"),
                ),
            };
            self.substitute(chosen, &chosen_path)?
        } else {
            if fields.contains_key("default") {
                return Err(invalid(format!(
                    "{path}/default: only cases have a default"
                )));
            }
            self.transform(option.clone(), fields, path)?
        };
        if !spread {
            return Ok(Some(Resolved::Value(resolved)));
        }
        match resolved {
            Value::Array(items) => Ok(Some(Resolved::Spread(items))),
            _ => Err(invalid(format!(
                "{path}/spread: the value spread must be an array"
            ))),
        }
    }

    /// `escape`, then `map`, `join` and `template`, each where present.
    fn transform(
        &self,
        mut value: Value,
        fields: &Map<String, Value>,
        path: &str,
    ) -> Result<Value> {
        let string_field = |key: &str| {
            fields
                .get(key)
                .map(|field| {
                    field
                        .as_str()
                        .ok_or_else(|| invalid(format!("{path}/{key} must be a string")))
                })
                .transpose()
        };
        match string_field("escape")? {
            None => {}
            Some("regex") => {
                let escape = |value: &Value| -> Result<Value> {
                    Ok(Value::String(regex::escape(&text(value, path)?)))
                };
                value = match &value {
                    Value::Array(items) => {
                        Value::Array(items.iter().map(escape).collect::<Result<_>>()?)
                    }
                    other => escape(other)?,
                };
            }
            Some(other) => {
                return Err(invalid(format!(
                    "{path}/escape: unknown escape `{other}`; expected regex"
                )));
            }
        }
        if let Some(template) = string_field("map")? {
            let items = value.as_array().ok_or_else(|| {
                invalid(format!("{path}/map: the option's value is not an array"))
            })?;
            value = Value::Array(
                items
                    .iter()
                    .map(|item| Ok(Value::String(template.replace("{}", &text(item, path)?))))
                    .collect::<Result<_>>()?,
            );
        }
        if let Some(separator) = string_field("join")? {
            let items = value.as_array().ok_or_else(|| {
                invalid(format!("{path}/join: the option's value is not an array"))
            })?;
            let texts = items
                .iter()
                .map(|item| text(item, path))
                .collect::<Result<Vec<_>>>()?;
            value = Value::String(texts.join(separator));
        }
        if let Some(template) = string_field("template")? {
            value = Value::String(template.replace("{}", &text(&value, path)?));
        }
        Ok(value)
    }

    /// Whether `when.option` holds: every named option's value meets its condition (so `{}` holds).
    pub(super) fn holds(&self, when: &Value, path: &str) -> Result<bool> {
        let conditions = object(when, path)?;
        let mut holds = true;
        for (name, condition) in conditions {
            let value = self.get(name, path)?;
            holds &= condition_holds(value, condition, &format!("{path}/{name}"))?;
        }
        Ok(holds)
    }
}

/// A condition on an option's value: the value itself (equality), or one of `{ "in": [...] }`,
/// `{ "includes": v }` (an array option holds `v`) and `{ "not": condition }`.
fn condition_holds(value: &Value, condition: &Value, path: &str) -> Result<bool> {
    let Some(fields) = condition.as_object() else {
        return Ok(value == condition);
    };
    if fields.len() != 1 {
        return Err(invalid(format!(
            "{path} must be a value or one of {{ in }}, {{ includes }}, {{ not }}"
        )));
    }
    let (key, operand) = fields.iter().next().expect("one field");
    match key.as_str() {
        "in" => Ok(operand
            .as_array()
            .ok_or_else(|| invalid(format!("{path}/in must be an array")))?
            .contains(value)),
        "includes" => Ok(value
            .as_array()
            .ok_or_else(|| invalid(format!("{path}/includes: the option is not an array")))?
            .contains(operand)),
        "not" => Ok(!condition_holds(value, operand, &format!("{path}/not"))?),
        other => Err(invalid(format!(
            "{path}: unknown condition `{other}`; expected in, includes or not"
        ))),
    }
}

/// The document's options, and the document with every `$option` reference replaced. The defines'
/// `options` and `config` are read as written; `format`, `id` and `revision` too. A reference among
/// the checks stands for a check or, spread, for several; never for a define.
pub(super) fn resolve(document: &Value) -> Result<(Value, Options)> {
    let mut declared = BTreeMap::new();
    let mut values = BTreeMap::new();
    let checks = document
        .get("clauses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    // Defines are the checks written as such: a reference among the checks is resolved with the
    // options, so it cannot declare them.
    let defines = checks.iter().enumerate().filter_map(|(index, check)| {
        if check.get("$option").is_some() {
            return None;
        }
        Some((format!("/clauses/{index}/define"), check.get("define")?))
    });
    for (path, define) in defines.clone() {
        let Some(options) = define.get("options") else {
            continue;
        };
        let options_path = format!("{path}/options");
        for (name, value) in object(options, &options_path)? {
            let option_path = format!("{options_path}/{name}");
            if !is_option_name(name) {
                return Err(invalid(format!(
                    "{option_path}: an option's name starts with a letter or _ and holds letters, digits, _, - and ."
                )));
            }
            if declared.contains_key(name) {
                return Err(invalid(format!(
                    "{option_path}: option `{name}` is already declared by another define"
                )));
            }
            let (declaration, default) = declaration(value, &option_path)?;
            declared.insert(name.clone(), declaration);
            values.insert(name.clone(), default);
        }
    }
    for (path, define) in defines {
        let Some(config) = define.get("config") else {
            continue;
        };
        let config_path = format!("{path}/config");
        for (name, value) in object(config, &config_path)? {
            let value_path = format!("{config_path}/{name}");
            let declaration = declared.get(name).ok_or_else(|| {
                invalid(format!(
                    "{value_path}: `{name}` is not an option this document declares"
                ))
            })?;
            let value = canonical_value(value, &value_path)?;
            check_value(declaration, &value, &value_path)?;
            values.insert(name.clone(), value);
        }
    }
    let options = Options { values };
    let Some(root) = document.as_object() else {
        return Ok((document.clone(), options));
    };
    let mut resolved = Map::with_capacity(root.len());
    for (key, value) in root {
        let path = format!("/{key}");
        let value = match key.as_str() {
            "format" | "id" | "revision" => value.clone(),
            "clauses" => match value.as_array() {
                Some(checks) => {
                    let mut out = Vec::with_capacity(checks.len());
                    for (index, check) in checks.iter().enumerate() {
                        let check_path = format!("{path}/{index}");
                        let items = match options.reference(check, &check_path)? {
                            None => {
                                out.push(resolve_check(&options, check, &check_path)?);
                                continue;
                            }
                            Some(Resolved::Value(value)) => vec![value],
                            Some(Resolved::Spread(values)) => values,
                        };
                        if items.iter().any(|item| item.get("define").is_some()) {
                            return Err(invalid(format!(
                                "{check_path}: a define is written among the clauses, not made by a $option reference"
                            )));
                        }
                        out.extend(items);
                    }
                    Value::Array(out)
                }
                None => value.clone(),
            },
            _ => options.substitute(value, &path)?,
        };
        resolved.insert(key.clone(), value);
    }
    Ok((Value::Object(resolved), options))
}

/// A check with its references replaced; a define's `options` and `config` stay as written.
fn resolve_check(options: &Options, check: &Value, path: &str) -> Result<Value> {
    let Some(define) = check.get("define").and_then(Value::as_object) else {
        return options.substitute(check, path);
    };
    let mut resolved = Map::new();
    for (key, value) in check
        .as_object()
        .expect("a check holding a define is an object")
    {
        let value = if key == "define" {
            let mut body = Map::new();
            for (field, value) in define {
                let value = if matches!(field.as_str(), "options" | "config") {
                    value.clone()
                } else {
                    options.substitute(value, &format!("{path}/define/{field}"))?
                };
                body.insert(field.clone(), value);
            }
            Value::Object(body)
        } else {
            options.substitute(value, &format!("{path}/{key}"))?
        };
        resolved.insert(key.clone(), value);
    }
    Ok(Value::Object(resolved))
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::test_support::*;
    use serde_json::{Value, json};

    fn document(options: Value, config: Value, checks: Value) -> Value {
        let mut all =
            vec![json!({ "id": "options", "define": { "options": options, "config": config } })];
        all.extend(checks.as_array().expect("checks").iter().cloned());
        json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": all })
    }

    fn query(id: &str, ast: Value) -> Value {
        let mut ast = ast;
        ast["language"] = json!("typescript");
        json!({ "id": id, "forbid": { "ast": ast }, "violation": finding() })
    }

    fn compile(document: Value) -> napi::Result<crate::policy_dsl::CompiledPolicyDsl> {
        compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    fn check_ids(document: Value, source: &str) -> Vec<String> {
        let compiled = compile(document).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                let span = &finding["span"];
                let (start, end) = (
                    span["startByte"].as_u64().expect("start"),
                    span["endByte"].as_u64().expect("end"),
                );
                format!(
                    "{} {}",
                    finding["clauseId"].as_str().expect("id"),
                    &source[start as usize..end as usize]
                )
            })
            .collect()
    }

    /// no-unused-vars' shape: an ignore pattern that is no filter until set, a check per `args`
    /// value, and scopes that `vars` narrows.
    fn unused(config: Value) -> Value {
        document(
            json!({
                "vars": { "type": "string", "enum": ["all", "local"], "default": "all" },
                "args": { "type": "string", "enum": ["all", "after-used", "none"], "default": "after-used" },
                "varsIgnorePattern": { "type": "string", "nullable": true, "default": null },
            }),
            config,
            json!([
                query(
                    "vars",
                    json!({
                        "rule": {
                            "kind": "identifier",
                            "inside": { "kind": "variable_declarator", "field": "name" },
                            "all": [
                                { "any": [
                                    { "inside": { "kind": "function_declaration", "stopBy": "end" } },
                                    { "$option": "vars", "cases": { "all": { "kind": "identifier" }, "local": { "regex": "[^\\s\\S]" } } },
                                ] },
                                { "$option": "varsIgnorePattern", "cases": { "null": [] }, "default": [{ "not": { "regex": { "$option": "varsIgnorePattern" } } }], "spread": true },
                            ],
                        },
                    })
                ),
                query(
                    "args",
                    json!({
                        "rule": { "kind": "identifier", "inside": { "kind": "required_parameter" } },
                        "when": { "option": { "args": { "in": ["all", "after-used"] } } },
                    })
                ),
            ]),
        )
    }

    #[test]
    fn options_resolve_references_before_the_document_is_validated() {
        let source = "const a = 1;\nconst _b = 2;\nfunction f(p) { const c = 3; }\n";
        assert_eq!(
            check_ids(unused(json!({})), source),
            ["vars a", "vars _b", "args p", "vars c"]
        );
        assert_eq!(
            check_ids(
                unused(json!({ "vars": "local", "varsIgnorePattern": "^_", "args": "none" })),
                source
            ),
            ["vars c"]
        );
        assert_eq!(
            check_ids(unused(json!({ "varsIgnorePattern": "^_" })), source),
            ["vars a", "args p", "vars c"]
        );
    }

    #[test]
    fn references_escape_map_join_and_fill_templates() {
        let options = json!({
            "allowed": { "type": "array", "default": ["a.b", "c"] },
            "prefix": { "type": "string", "default": "x" },
        });
        let checks = json!([query(
            "names",
            json!({
                "rule": { "kind": "identifier", "any": [
                    { "regex": { "$option": "allowed", "escape": "regex", "join": "|", "template": "^(?:{})$" } },
                    { "regex": { "$option": "prefix", "template": "^{}_" } },
                ] },
            })
        )]);
        let source = "a.b; ab; c; x_1; y_1;\n";
        assert_eq!(
            check_ids(document(options.clone(), json!({}), checks.clone()), source),
            ["names c", "names x_1"]
        );
        // `map` fills a template per item: here a `scope` list.
        let mut mapped = document(options, json!({ "allowed": ["src", "lib"] }), checks);
        mapped["scope"] =
            json!({ "include": { "$option": "allowed", "map": "{}/**" }, "exclude": [] });
        let compiled = compile(mapped).unwrap_or_else(|error| panic!("{}", error.reason));
        assert!(
            compiled.in_scope("src/a.ts".to_owned()) && compiled.in_scope("lib/b.ts".to_owned())
        );
        assert!(!compiled.in_scope("test/c.ts".to_owned()));
    }

    #[test]
    fn a_reference_among_the_checks_spreads_checks_into_them() {
        let options = json!({ "on": { "type": "boolean", "default": true } });
        let spread = |checks: Value| json!({ "$option": "on", "cases": { "false": [] }, "default": checks, "spread": true });
        let class = query("c", json!({ "rule": { "kind": "class_declaration" } }));
        let number = query("n", json!({ "rule": { "kind": "number" } }));
        let source = "class A {}\n1;\n";
        let on = document(options.clone(), json!({}), json!([spread(json!([class]))]));
        assert_eq!(check_ids(on, source), ["c class A {}"]);
        let both = |config: Value| {
            document(
                options.clone(),
                config,
                json!([spread(json!([class])), number]),
            )
        };
        assert_eq!(check_ids(both(json!({})), source), ["c class A {}", "n 1"]);
        assert_eq!(check_ids(both(json!({ "on": false })), source), ["n 1"]);
        // A reference that stands for one check, unspread, is that check.
        let single = document(
            options.clone(),
            json!({}),
            json!([{ "$option": "on", "cases": { "true": class, "false": number } }]),
        );
        assert_eq!(check_ids(single, source), ["c class A {}"]);
        for (document, reason) in [
            (
                document(
                    options.clone(),
                    json!({ "on": false }),
                    json!([spread(json!([class]))]),
                ),
                "/clauses must hold a clause besides define: a define reports nothing",
            ),
            (
                document(
                    options.clone(),
                    json!({}),
                    json!([
                        spread(
                            json!([{ "id": "d", "define": { "options": { "x": { "type": "boolean", "default": true } } } }])
                        ),
                        class,
                    ]),
                ),
                "/clauses/1: a define is written among the clauses, not made by a $option reference",
            ),
            (
                document(
                    options,
                    json!({}),
                    json!([
                        { "$option": "on", "define": { "options": {} } },
                        class,
                    ]),
                ),
                "unknown field `define` at /clauses/1",
            ),
        ] {
            assert_eq!(rejection(compile(document)), reason);
        }
    }

    #[test]
    fn when_option_gates_a_query_on_the_documents_options() {
        let options =
            json!({ "allow": { "type": "array", "enum": ["methods", "arrows"], "default": [] } });
        let checks = |when: Value| {
            json!([query(
                "q",
                json!({ "rule": { "kind": "number" }, "when": { "option": when } })
            )])
        };
        for (config, when, reports) in [
            (json!({}), json!({ "allow": [] }), true),
            // No condition holds for every value: the neutral gate a reference can choose.
            (json!({}), json!({}), true),
            (
                json!({}),
                json!({ "allow": { "includes": "methods" } }),
                false,
            ),
            (
                json!({ "allow": ["methods"] }),
                json!({ "allow": { "includes": "methods" } }),
                true,
            ),
            (
                json!({ "allow": ["methods"] }),
                json!({ "allow": { "not": { "includes": "methods" } } }),
                false,
            ),
            (
                json!({ "allow": ["arrows"] }),
                json!({ "allow": { "in": [["arrows"], ["methods"]] } }),
                true,
            ),
        ] {
            let ids = check_ids(document(options.clone(), config, checks(when)), "1;\n");
            assert_eq!(!ids.is_empty(), reports, "{ids:?}");
        }
    }
    #[test]
    fn integral_fractional_options_select_the_same_case_as_javascript() {
        let options = json!({ "level": { "type": "number", "default": 1.0 } });
        let selected = query(
            "selected",
            json!({
                "rule": {
                    "$option": "level",
                    "cases": { "1": { "kind": "number" } },
                },
            }),
        );
        assert_eq!(
            check_ids(document(options, json!({}), json!([selected])), "1;\n"),
            ["selected 1"]
        );
    }

    #[test]
    fn integral_integer_options_accept_decimal_and_exponent_forms() {
        let checks = json!([query("number", json!({ "rule": { "kind": "number" } }))]);
        let scalar = json!({ "level": { "type": "integer", "default": 1.0 } });
        assert_eq!(
            check_ids(
                document(scalar, json!({ "level": 1e0 }), checks.clone()),
                "1;\n"
            ),
            ["number 1"]
        );
        let items = json!({ "levels": { "type": "array", "items": "integer", "default": [1.0] } });
        assert_eq!(
            check_ids(document(items, json!({ "levels": [1e0] }), checks), "1;\n"),
            ["number 1"]
        );
    }

    #[test]
    fn integer_array_enum_normalizes_its_items() {
        let options = json!({
            "levels": { "type": "array", "items": "integer", "enum": [1, 2], "default": [1] }
        });
        assert_eq!(
            check_ids(
                document(
                    options,
                    json!({}),
                    json!([query("number", json!({ "rule": { "kind": "number" } }))]),
                ),
                "1;\n",
            ),
            ["number 1"]
        );
    }

    #[test]
    fn nullable_integer_array_keeps_null_for_option_cases() {
        let options = json!({ "levels": { "type": "array", "items": "integer", "nullable": true, "default": null } });
        let selected = query(
            "selected",
            json!({
                "rule": {
                    "$option": "levels",
                    "cases": {
                        "null": { "kind": "number" },
                        "default": { "kind": "identifier" }
                    }
                }
            }),
        );
        assert_eq!(
            check_ids(document(options, json!({}), json!([selected])), "1;\n"),
            ["selected 1"]
        );
    }

    #[test]
    fn integer_options_reject_out_of_range_decimal_numbers() {
        assert_eq!(
            rejection(compile(document(
                json!({ "level": { "type": "integer", "default": 1e20 } }),
                json!({}),
                json!([query("number", json!({ "rule": { "kind": "number" } }))]),
            ))),
            "/clauses/0/define/options/level/default: 100000000000000000000 is not exactly representable as a JavaScript number"
        );
    }

    #[test]
    fn option_numbers_are_the_javascript_numbers_every_entrypoint_reads() {
        let options = json!({
            "level": { "type": "number", "default": 1.0 },
            "count": { "type": "integer", "enum": [3], "default": 3.0 },
            "tiny": { "type": "number", "default": 1.5e-7 },
            "ratio": { "type": "number", "default": 0.00001 },
            "zero": { "type": "number", "default": -0.0 },
        });
        let cases = |id: &str, option: &str, key: &str| {
            query(
                id,
                json!({ "rule": { "$option": option, "cases": { key: { "kind": "number" } } } }),
            )
        };
        let checks = json!([
            cases("case", "level", "1"),
            cases("count", "count", "3"),
            cases("exponent", "tiny", "1.5e-7"),
            cases("zero", "zero", "0"),
            query(
                "text",
                json!({ "rule": { "kind": "number", "regex": { "$option": "ratio", "template": "^{}$" } } })
            ),
            query(
                "version",
                json!({ "rule": { "kind": "identifier", "regex": { "$option": "level", "template": "^v{}$" } } })
            ),
        ]);
        // The defaults as written: `1.0`, `3.0`, `1.5e-7`, `0.00001` and `-0.0` read as
        // JavaScript reads them.
        let mut ids = check_ids(
            document(options.clone(), json!({}), checks.clone()),
            "0.00001;\nv1;\n",
        );
        ids.sort();
        assert_eq!(
            ids,
            [
                "case 0.00001",
                "count 0.00001",
                "exponent 0.00001",
                "text 0.00001",
                "version v1",
                "zero 0.00001",
            ]
        );
        // An enum holds `1`, so `1.0` configures it, as a number or an integer.
        for kind in ["number", "integer"] {
            let options = json!({ "level": { "type": kind, "enum": [1, 2], "default": 2 } });
            assert_eq!(
                check_ids(
                    document(
                        options,
                        json!({ "level": 1.0 }),
                        json!([cases("case", "level", "1")])
                    ),
                    "1;\n"
                ),
                ["case 1"],
                "{kind}"
            );
        }
        let beyond = json!(9_007_199_254_740_993u64);
        let rule = json!([query("q", json!({ "rule": { "kind": "number" } }))]);
        for (options, config, reason) in [
            (
                json!({ "n": { "type": "integer", "default": beyond } }),
                json!({}),
                "/clauses/0/define/options/n/default: 9007199254740993 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "integer", "default": -9_007_199_254_740_992i64 } }),
                json!({}),
                "/clauses/0/define/options/n/default: -9007199254740992 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "array", "items": "integer", "enum": [1, beyond], "default": [] } }),
                json!({}),
                "/clauses/0/define/options/n/enum/1: 9007199254740993 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "integer", "default": 1 } }),
                json!({ "n": beyond }),
                "/clauses/0/define/config/n: 9007199254740993 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "number", "default": 1e21 } }),
                json!({}),
                "/clauses/0/define/options/n/default: 1e+21 is not exactly representable as a JavaScript number",
            ),
        ] {
            assert_eq!(
                rejection(compile(document(options, config, rule.clone()))),
                reason
            );
        }
    }

    #[test]
    fn options_and_references_are_checked_where_they_are_written() {
        let rule = query("q", json!({ "rule": { "kind": "number" } }));
        let string = json!({ "s": { "type": "string", "default": "a" } });
        for (document, reason) in [
            (
                document(
                    json!({ "s": { "type": "text", "default": "a" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/type must equal boolean, string, number, integer or array",
            ),
            (
                document(
                    json!({ "s": { "type": "string" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default is required",
            ),
            (
                document(
                    json!({ "s": { "type": "string", "default": 1 } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default is not a value of the option's type",
            ),
            (
                document(
                    json!({ "s": { "type": "string", "enum": ["a"], "default": "b" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default is not one of the option's enum values",
            ),
            (
                document(
                    json!({ "s": { "type": "array", "items": "integer", "default": [1.5] } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default/0 is not an allowed item of the option",
            ),
            (
                document(
                    json!({ "s": { "type": "string", "default": null } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default: null is not allowed; the option is not nullable",
            ),
            (
                document(
                    json!({ "1s": { "type": "string", "default": "a" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/1s: an option's name starts with a letter or _ and holds letters, digits, _, - and .",
            ),
            (
                document(string.clone(), json!({ "t": "a" }), json!([rule])),
                "/clauses/0/define/config/t: `t` is not an option this document declares",
            ),
            (
                document(string.clone(), json!({ "s": true }), json!([rule])),
                "/clauses/0/define/config/s is not a value of the option's type",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "t" } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/$option: `t` is not an option this document declares",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "spread": true } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/spread: only an array's item may spread",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "cases": { "b": "x" } } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/cases: no case for `a` and no default",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "join": "|" } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/join: the option's value is not an array",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "suffix": "x" } } })
                    )]),
                ),
                "unknown field `suffix` at /clauses/1/forbid/ast/rule/regex",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "kind": "number" }, "when": { "option": { "t": "a" } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/when/option: `t` is not an option this document declares",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "kind": "number" }, "when": { "option": { "s": { "startsWith": "a" } } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/when/option/s: unknown condition `startsWith`; expected in, includes or not",
            ),
            (
                json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": [
                    { "id": "a", "define": { "options": string } },
                    { "id": "b", "define": { "options": string } },
                    rule,
                ] }),
                "/clauses/1/define/options/s: option `s` is already declared by another define",
            ),
        ] {
            assert_eq!(rejection(compile(document)), reason);
        }
        // An option's own declaration is read as written: a reference there is its default's text.
        compile(document(
            json!({ "s": { "type": "string", "default": "$option" } }),
            json!({}),
            json!([rule]),
        ))
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }
}
