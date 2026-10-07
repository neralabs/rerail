use super::ast_rule::{compile_ast_rule, validate_ast_rule};
use super::utils::{UtilIds, UtilScope};
use crate::policy_dsl::json::{invalid, reject_unknown};
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::matcher::{AstMatcher, NthChild, NthPosition, parse_position};
use crate::policy_dsl::model::Limits;
use napi::Result;
use serde_json::Value;

/// An `nthChild` value as upstream reads it: a number, an `An+B` string, or `{ position, ofRule,
/// reverse }`; its position and its `ofRule`, if any.
fn shape<'a>(value: &'a Value, path: &str) -> Result<(NthPosition, Option<&'a Value>, bool)> {
    let position = |value: &Value, path: &str| -> Result<Option<NthPosition>> {
        match value {
            Value::Number(number) => Ok(number
                .as_u64()
                .and_then(|offset| i32::try_from(offset).ok())
                .map(|offset| NthPosition { step: 0, offset })),
            Value::String(text) => parse_position(text)
                .map(Some)
                .map_err(|reason| invalid(format!("{path}: {reason}"))),
            _ => Ok(None),
        }
    };
    let malformed = || {
        invalid(format!(
            "{path} must be a non-negative integer, an An+B string, or {{ position, ofRule, reverse }}"
        ))
    };
    let Value::Object(object) = value else {
        return Ok((position(value, path)?.ok_or_else(malformed)?, None, false));
    };
    reject_unknown(object, &["position", "ofRule", "reverse"], path)?;
    let at = format!("{path}/position");
    let position = match object.get("position") {
        None => return Err(invalid(format!("{at} is required"))),
        Some(value) => position(value, &at)?.ok_or_else(|| {
            invalid(format!(
                "{at} must be a non-negative integer or an An+B string"
            ))
        })?,
    };
    let reverse = match object.get("reverse") {
        None => false,
        Some(Value::Bool(reverse)) => *reverse,
        Some(_) => return Err(invalid(format!("{path}/reverse must be a boolean"))),
    };
    Ok((position, object.get("ofRule"), reverse))
}

pub(super) fn validate_nth_child(
    value: &Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    if let (_, Some(rule), _) = shape(value, path)? {
        validate_ast_rule(rule, &format!("{path}/ofRule"), limits, utils)?;
    }
    Ok(())
}

pub(super) fn compile_nth_child(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    let (position, rule, reverse) = shape(value, path)?;
    let of_rule = rule
        .map(|rule| compile_ast_rule(rule, language, &format!("{path}/ofRule"), gaps, utils))
        .transpose()?;
    // An `ofRule` this grammar cannot express counts no sibling, so no node has a position.
    if matches!(of_rule, Some(AstMatcher::Never)) {
        return Ok(AstMatcher::Never);
    }
    Ok(AstMatcher::NthChild(Box::new(NthChild {
        position,
        of_rule,
        reverse,
    })))
}
