//! `formatArgs`: a format call's arguments no field of its template consumes, and its fields no
//! argument fills.

use super::ast_rule::{compile_ast_rule, validate_ast_rule};
use super::parse::{capture_name, compile_sub_rule, validate_sub_rule};
use super::utils::{UtilIds, UtilScope};
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use crate::code_rule_dsl::language::adapter::{
    Adapter, FormatArgFeature, FormatArgRules, gate, with_adapter,
};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{
    AstMatcher, Capture, FormatArgs, Reading, Style, parse_exported,
};
use crate::code_rule_dsl::model::Limits;
use napi::Result;
use serde_json::Value;
use std::collections::BTreeSet;

const KEYS: &[&str] = &["style", "string", "args", "unused", "missing"];

fn style(value: &Value, path: &str) -> Result<Style> {
    match non_empty_str(value.get("style"), &format!("{path}/style"))? {
        "brace" => Ok(Style::Brace),
        "percent" => Ok(Style::Percent),
        "printf" => Ok(Style::Printf),
        "scanf" => Ok(Style::Scanf),
        other => Err(invalid(format!(
            "{path}/style: unknown value `{other}`; expected brace, percent, printf or scanf"
        ))),
    }
}

/// Whether `A`'s format strings follow their bindings (`FormatArgRules::FOLLOWS_STRINGS`), once
/// `A` reads `style`'s format calls.
fn follows_strings<A: Adapter>(path: &str, style: Style) -> Result<bool> {
    gate(
        &format!("{path}/style"),
        <A::FormatArgs as FormatArgRules>::feature(FormatArgFeature::Style(style)),
    )?;
    Ok(<A::FormatArgs as FormatArgRules>::FOLLOWS_STRINGS)
}

/// The grammar-independent checks of a `formatArgs` object.
pub(super) fn validate_format_args(
    value: &Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    let test = object(value, path)?;
    reject_unknown(test, KEYS, path)?;
    let style = style(value, path)?;
    for key in ["string", "args"] {
        let name = test
            .get(key)
            .ok_or_else(|| invalid(format!("{path}/{key} is required")))?;
        capture_name(name, &format!("{path}/{key}"))?;
    }
    if !test.contains_key("unused") && !test.contains_key("missing") {
        return Err(invalid(format!("{path} must hold unused, missing or both")));
    }
    if let Some(unused) = test.get("unused") {
        validate_ast_rule(unused, &format!("{path}/unused"), limits, utils)?;
    }
    if let Some(missing) = test.get("missing") {
        validate_sub_rule(
            missing,
            &format!("{path}/missing"),
            style.grammar(),
            false,
            limits,
            utils,
        )?;
    }
    Ok(())
}

/// Compiles a validated `formatArgs` for one grammar.
pub(super) fn compile_format_args(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    let style = style(value, path)?;
    let follows = with_adapter!(Family::of(language), A => follows_strings::<A>(path, style))?;
    let unused = value
        .get("unused")
        .map(|unused| compile_ast_rule(unused, language, &format!("{path}/unused"), gaps, utils))
        .transpose()?;
    let missing = value
        .get("missing")
        .map(|missing| {
            compile_sub_rule(
                missing,
                style.grammar(),
                &format!("{path}/missing"),
                utils.transform.as_ref(),
                None,
                gaps,
            )
        })
        .transpose()?;
    let mut exports = BTreeSet::new();
    if let Some(missing) = &missing {
        parse_exported(missing, &mut exports);
    }
    if matches!(unused, Some(AstMatcher::Never)) {
        return Ok(AstMatcher::Never);
    }
    let name = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let reading = follows
        .then(|| Reading::new(language, utils.options.clone(), utils.python.clone()).following());
    Ok(AstMatcher::Semantic(Semantic::FormatArgs(Box::new(
        FormatArgs {
            style,
            string: name("string"),
            args: name("args"),
            unused,
            missing,
            exports: exports.into_iter().collect(),
            reading,
        },
    ))))
}

/// Every `formatArgs` in a rule, with its path, wherever it sits.
pub(super) fn format_args_sites<'a>(
    value: &'a Value,
    path: &str,
    sites: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}/{key}");
                if key == "formatArgs" {
                    sites.push((child_path.clone(), child));
                }
                format_args_sites(child, &child_path, sites);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                format_args_sites(item, &format!("{path}/{index}"), sites);
            }
        }
        _ => {}
    }
}

/// Checks a `formatArgs`' captures against what the query binds: the string is a single-node
/// capture; a `.format` call's arguments a `$$$` capture of its argument list.
pub(super) fn check_captures(
    test: &Value,
    path: &str,
    variables: &std::collections::BTreeMap<String, Capture>,
) -> Result<()> {
    let name = |key: &str| test.get(key).and_then(Value::as_str).unwrap_or_default();
    let (string, args) = (name("string"), name("args"));
    match variables.get(string) {
        Some(Capture::Single) => {}
        Some(Capture::Multiple) => {
            return Err(invalid(format!(
                "{path}/string: `$$${string}` captures a node list; the format string is one node"
            )));
        }
        None => {
            return Err(invalid(format!(
                "{path}/string: `${string}` is not bound by the rule, its utils, or its constraints"
            )));
        }
    }
    match (
        variables.get(args),
        test.get("style").and_then(Value::as_str),
    ) {
        (None, _) => Err(invalid(format!(
            "{path}/args: `${args}` is not bound by the rule, its utils, or its constraints"
        ))),
        (Some(Capture::Single), Some("brace")) => Err(invalid(format!(
            "{path}/args: a `.format` call's arguments are a list; capture them with `$$${args}`"
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod php;
#[cfg(test)]
mod ruff;
#[cfg(test)]
mod tests;
