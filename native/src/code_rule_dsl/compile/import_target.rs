//! `importTarget`: an ECMA rule's condition on the import or require target a specifier node
//! names (`import_target`).

use crate::code_rule_dsl::import_target::{ModuleType, Style};
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::matcher::{AstMatcher, ImportTargetTest, Reading, Semantic};
use crate::code_rule_dsl::scope::options::LanguageOptions;
use napi::Result;
use serde_json::Value;
use std::sync::Arc;

/// The facts `importTarget` may condition on, each `true` or `false`.
const FLAGS: [&str; 8] = [
    "builtin",
    "resolved",
    "failed",
    "extensionSpelled",
    "published",
    "devDependency",
    "declared",
    "aliased",
];

/// The grammar-independent checks of an `importTarget` object: `via` (`import` or `require`),
/// `moduleType` (a type or a non-empty list of them), and the boolean facts.
pub(super) fn validate_import_target(value: &Value, path: &str) -> Result<()> {
    let target = object(value, path)?;
    let keys = ["via", "moduleType"]
        .into_iter()
        .chain(FLAGS)
        .collect::<Vec<_>>();
    reject_unknown(target, &keys, path)?;
    if !matches!(
        target.get("via").and_then(Value::as_str),
        Some("import" | "require")
    ) {
        return Err(invalid(format!(
            "{path}/via must be \"import\" (an import or export source, or an import()) or \"require\" (a require() or require.resolve() argument): the style a target has where no ancestor decides it"
        )));
    }
    if let Some(types) = target.get("moduleType") {
        module_types(types, &format!("{path}/moduleType"))?;
    }
    for flag in FLAGS {
        if target.get(flag).is_some_and(|value| !value.is_boolean()) {
            return Err(invalid(format!("{path}/{flag} must be true or false")));
        }
    }
    Ok(())
}

fn module_types(value: &Value, path: &str) -> Result<Vec<ModuleType>> {
    let names = match value {
        Value::String(name) => vec![name.as_str()],
        Value::Array(names) if !names.is_empty() => names
            .iter()
            .map(|name| {
                name.as_str()
                    .ok_or_else(|| invalid(format!("{path} must list module types")))
            })
            .collect::<Result<_>>()?,
        _ => {
            return Err(invalid(format!(
                "{path} must be a module type or a non-empty list of them"
            )));
        }
    };
    names
        .into_iter()
        .map(|name| {
            ModuleType::parse(name).ok_or_else(|| {
                invalid(format!(
                    "{path}: `{name}` is not a module type; expected one of {}",
                    ModuleType::NAMES.join(", ")
                ))
            })
        })
        .collect()
}

/// Compiles a validated `importTarget` for `language`, which must be an ECMA grammar.
pub(super) fn compile_import_target(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
) -> Result<AstMatcher> {
    if Family::of(language) != Family::Ecma {
        return Err(invalid(format!(
            "{path}: import targets are eslint-plugin-n's, read in JavaScript and TypeScript files"
        )));
    }
    let flag = |name: &str| value.get(name).and_then(Value::as_bool);
    Ok(AstMatcher::Semantic(Semantic::ImportTarget(Box::new(
        ImportTargetTest {
            reading: Reading::new(language, options.cloned(), None),
            via: match value.get("via").and_then(Value::as_str) {
                Some("require") => Style::Require,
                _ => Style::Import,
            },
            module_types: value
                .get("moduleType")
                .map(|types| module_types(types, &format!("{path}/moduleType")))
                .transpose()?,
            builtin: flag("builtin"),
            resolved: flag("resolved"),
            failed: flag("failed"),
            extension_spelled: flag("extensionSpelled"),
            published: flag("published"),
            dev_dependency: flag("devDependency"),
            declared: flag("declared"),
            aliased: flag("aliased"),
        },
    ))))
}
