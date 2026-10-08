//! `formatArgs`: a format call's arguments no field of its template consumes, and its fields no
//! argument fills.

use super::ast_rule::{compile_ast_rule, validate_ast_rule};
use super::parse::{capture_name, compile_sub_rule, validate_sub_rule};
use super::utils::{UtilIds, UtilScope};
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
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

/// Why a family does not read a style's format calls: the style is another family's.
fn foreign_style(family: Family, style: Style) -> Option<&'static str> {
    match (family, style) {
        (Family::Python, Style::Brace | Style::Percent)
        | (Family::Php, Style::Printf | Style::Scanf)
        // Blade's format calls are those of the PHP its template embeds (`blade::view`).
        | (Family::Blade, Style::Printf | Style::Scanf) => None,
        (Family::Python, Style::Printf) => Some(
            "`printf` reads PHP's printf-family calls; Python's printf formats are style `percent`",
        ),
        (Family::Python, Style::Scanf) => {
            Some("`scanf` reads PHP's sscanf and fscanf calls; Python has no scanf formats")
        }
        (Family::Php, Style::Percent) => Some(
            "`percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
        ),
        (Family::Php, Style::Brace) => Some(
            "`brace` reads Python's `str.format` calls; PHP has no such templates (its printf-family calls are style `printf`)",
        ),
        (Family::Blade, Style::Percent) => Some(
            "`percent` reads Python's `%` formats; blade embeds PHP, whose printf-family calls are style `printf`",
        ),
        (Family::Blade, Style::Brace) => Some(
            "`brace` reads Python's `str.format` calls; blade embeds PHP, which has no such templates (its printf-family calls are style `printf`)",
        ),
        (Family::Ecma, _) => Some("formatArgs reads Python and PHP format calls"),
    }
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
    if let Some(reason) = foreign_style(Family::of(language), style) {
        return Err(invalid(format!("{path}/style: {reason}")));
    }
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
    // PHP's formats (and those of the PHP a Blade template embeds) are read as PHPStan reads a
    // constant string: through concatenation, casts and the one binding that reaches a variable.
    let reading = match Family::of(language) {
        Family::Php | Family::Blade => {
            Some(Reading::new(language, utils.options.clone(), utils.python.clone()).following())
        }
        Family::Python | Family::Ecma => None,
    };
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
