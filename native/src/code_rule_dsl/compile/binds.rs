use super::ast_rule::{compile_ast_rule, validate_ast_rule};
use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
use super::utils::{UtilIds, UtilScope};
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::language::SourceType;
use crate::code_rule_dsl::language::kind_ids;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{
    AstMatcher, BindingCount, BindingFilter, Binds, Redefines, ScopeName,
};
use crate::code_rule_dsl::model::Limits;
use crate::code_rule_dsl::scope::options::{EsGlobals, LanguageOptions};
use crate::code_rule_dsl::scope::{Form, ScopeKind};
use napi::Result;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Filters over a binding, in `binds` and `resolves.binding`.
pub(super) const FILTER_KEYS: &[&str] = &[
    "form",
    "scope",
    "used",
    "referenced",
    "nestedUse",
    "exported",
    "latest",
    "outer",
    "bindings",
    "builtin",
    "directives",
];

/// The filters that take a boolean.
const BOOLEAN_FILTER_KEYS: &[&str] = &[
    "used",
    "referenced",
    "nestedUse",
    "exported",
    "latest",
    "outer",
    "builtin",
];

const FORMS: &[(&str, Form)] = &[
    ("assignment", Form::Assignment),
    ("annotated", Form::Annotated),
    ("annotation", Form::Annotation),
    ("augmented", Form::Augmented),
    ("named", Form::Named),
    ("for", Form::For),
    ("with", Form::With),
    ("except", Form::Except),
    ("import", Form::Import),
    ("definition", Form::Definition),
    ("parameter", Form::Parameter),
    ("global", Form::Global),
    ("match", Form::Match),
    ("typeParameter", Form::TypeParameter),
    ("typeAlias", Form::TypeAlias),
    ("implicit", Form::Implicit),
    ("deletion", Form::Deletion),
    ("field", Form::Field),
    ("method", Form::Method),
    ("accessor", Form::Accessor),
];

const PYTHON_SCOPES: &[(&str, ScopeName)] = &[
    ("module", ScopeName::Kind(ScopeKind::Module)),
    ("class", ScopeName::Kind(ScopeKind::Class)),
    ("function", ScopeName::Kind(ScopeKind::Function)),
    ("lambda", ScopeName::Kind(ScopeKind::Lambda)),
    ("comprehension", ScopeName::Kind(ScopeKind::Comprehension)),
    ("typeParameters", ScopeName::Kind(ScopeKind::TypeParams)),
];

/// TypeScript's scopes as eslint-scope names them: `global` is a script's top level, `module` a
/// module's or CommonJS file's; `functionName` holds a named function expression's own name.
const TYPESCRIPT_SCOPES: &[(&str, ScopeName)] = &[
    ("global", ScopeName::Global),
    ("module", ScopeName::Kind(ScopeKind::Module)),
    ("function", ScopeName::Kind(ScopeKind::Function)),
    ("class", ScopeName::Kind(ScopeKind::Class)),
    ("block", ScopeName::Kind(ScopeKind::Block)),
    ("namespace", ScopeName::Kind(ScopeKind::Namespace)),
    ("typeParameters", ScopeName::Kind(ScopeKind::TypeParams)),
    ("functionName", ScopeName::Kind(ScopeKind::FunctionName)),
];

/// The forms a PHP variable is bound in.
const PHP_FORMS: &[(&str, Form)] = &[
    ("assignment", Form::Assignment),
    ("augmented", Form::Augmented),
    ("for", Form::For),
    ("except", Form::Except),
    ("parameter", Form::Parameter),
    ("global", Form::Global),
    ("static", Form::Static),
];

/// PHP's variable scopes: the file's top level (`global`), a function or method, a closure or
/// arrow function.
const PHP_SCOPES: &[(&str, ScopeName)] = &[
    ("global", ScopeName::Kind(ScopeKind::Module)),
    ("function", ScopeName::Kind(ScopeKind::Function)),
    ("closure", ScopeName::Kind(ScopeKind::Lambda)),
];

/// Names in a filter's `form` or `scope`: one name or a non-empty list, each one of `known`.
fn names<T: Copy>(value: &Value, path: &str, known: &[(&str, T)], what: &str) -> Result<Vec<T>> {
    let items = match value {
        Value::String(_) => std::slice::from_ref(value),
        Value::Array(items) if !items.is_empty() => items.as_slice(),
        _ => {
            return Err(invalid(format!(
                "{path} must be a {what} name or a non-empty array of them"
            )));
        }
    };
    items
        .iter()
        .map(|item| {
            let name = item.as_str().unwrap_or_default();
            known
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, value)| *value)
                .ok_or_else(|| {
                    let expected = known
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    invalid(format!(
                        "{path}: `{item}` is not a {what}; expected one of {expected}"
                    ))
                })
        })
        .collect()
}

fn boolean(object: &Map<String, Value>, key: &str, path: &str) -> Result<Option<bool>> {
    object
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid(format!("{path}/{key} must be true or false")))
        })
        .transpose()
}

/// The grammar-independent checks of a filter's keys.
pub(super) fn validate_filter(object: &Map<String, Value>, path: &str) -> Result<()> {
    // Every family's names; each family's compile narrows them to its own.
    if let Some(form) = object.get("form") {
        let mut known = FORMS
            .iter()
            .chain(PHP_FORMS)
            .map(|(name, _)| (*name, ()))
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        known.retain(|(name, _)| seen.insert(*name));
        names(form, &format!("{path}/form"), &known, "binding form")?;
    }
    if let Some(scope) = object.get("scope") {
        let mut known = PYTHON_SCOPES
            .iter()
            .chain(TYPESCRIPT_SCOPES)
            .chain(PHP_SCOPES)
            .map(|(name, _)| (*name, ()))
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        known.retain(|(name, _)| seen.insert(*name));
        names(scope, &format!("{path}/scope"), &known, "scope kind")?;
    }
    for key in BOOLEAN_FILTER_KEYS {
        boolean(object, key, path)?;
    }
    if let Some(bindings) = object.get("bindings") {
        binding_count(bindings, &format!("{path}/bindings"))?;
    }
    if let Some(directives) = object.get("directives")
        && directives.as_str() != Some("eslint")
    {
        return Err(invalid(format!(
            "{path}/directives must be \"eslint\" (honor /* exported */ comments)"
        )));
    }
    Ok(())
}

/// A filter's `bindings`: `min` and `max` as `count` takes them, and an optional `within`, a
/// capture's bare name as `compare` takes it.
fn binding_count(value: &Value, path: &str) -> Result<BindingCount> {
    let count = object(value, path)?;
    reject_unknown(count, &["within", "min", "max"], path)?;
    let bound = |name: &str| -> Result<Option<usize>> {
        count
            .get(name)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| invalid(format!("{path}/{name} must be a non-negative integer")))
            })
            .transpose()
    };
    let (min, max) = (bound("min")?, bound("max")?);
    if min.unwrap_or(0) == 0 && max.is_none() {
        return Err(invalid(format!(
            "{path} must set min to at least 1, or set max; otherwise every binding passes"
        )));
    }
    let min = min.unwrap_or(0);
    if max.is_some_and(|max| max < min) {
        return Err(invalid(format!("{path}: min must not exceed max")));
    }
    let within = count
        .get("within")
        .map(|within| {
            let name = within.as_str().unwrap_or_default();
            let valid = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_';
            if name.is_empty() || !name.chars().all(valid) {
                return Err(invalid(format!(
                    "{path}/within: `{within}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
                )));
            }
            Ok(name.to_owned())
        })
        .transpose()?;
    Ok(BindingCount { within, min, max })
}

/// Compiles a validated filter for one grammar. Ruff's binding order and flags (`latest`,
/// `outer`) are Python's, and PHP's from its flow; ESLint's directive comments (`directives`) and
/// configured globals (`builtin`) are TypeScript's. `bindings` counts a scope's bindings of the
/// name in all three: Ruff's `scope.get_all`, eslint-scope's `defs`, PHP's binding sites.
pub(super) fn compile_filter(
    object: &Map<String, Value>,
    language: GrammarDialect,
    path: &str,
    utils: &UtilScope,
) -> Result<BindingFilter> {
    let family = Family::of(language);
    match family {
        Family::Python => {
            if object.contains_key("directives") {
                return Err(invalid(format!(
                    "{path}/directives: /* exported */ comments are ESLint's; Python has none"
                )));
            } else if object.contains_key("builtin") {
                return Err(invalid(format!(
                    "{path}/builtin: configured globals are ESLint's; Python has none"
                )));
            }
        }
        Family::Ecma => {
            for key in ["latest", "outer"] {
                if object.contains_key(key) {
                    return Err(invalid(format!(
                        "{path}/{key}: `{key}` reads Python's binding order and declarations; TypeScript has none"
                    )));
                }
            }
        }
        // Blade binds the variables of the PHP its template embeds.
        Family::Php | Family::Blade => {
            let refused = [
                (
                    "exported",
                    "Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports)",
                ),
                (
                    "builtin",
                    "ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to `$_GET` writes the superglobal)",
                ),
                (
                    "directives",
                    "ESLint's /* exported */ comments; PHP exports no variables",
                ),
            ];
            if let Some((key, what)) = refused.iter().find(|(key, _)| object.contains_key(*key)) {
                return Err(invalid(format!(
                    "{path}/{key}: `{key}` reads {what}; {} has none",
                    php_family_name(family)
                )));
            }
        }
    }
    let (forms, scopes) = match family {
        Family::Python => (FORMS, PYTHON_SCOPES),
        Family::Ecma => (FORMS, TYPESCRIPT_SCOPES),
        Family::Php | Family::Blade => (PHP_FORMS, PHP_SCOPES),
    };
    Ok(BindingFilter {
        forms: object
            .get("form")
            .map(|form| names(form, &format!("{path}/form"), forms, "binding form"))
            .transpose()?,
        scopes: object
            .get("scope")
            .map(|scope| names(scope, &format!("{path}/scope"), scopes, "scope kind"))
            .transpose()?,
        used: boolean(object, "used", path)?,
        referenced: boolean(object, "referenced", path)?,
        nested_use: boolean(object, "nestedUse", path)?,
        exported: boolean(object, "exported", path)?,
        latest: boolean(object, "latest", path)?,
        outer: boolean(object, "outer", path)?,
        bindings: object
            .get("bindings")
            .map(|bindings| binding_count(bindings, &format!("{path}/bindings")))
            .transpose()?,
        builtin: boolean(object, "builtin", path)?,
        directives: object.contains_key("directives"),
        language: Some(language),
        options: (family == Family::Ecma)
            .then(|| {
                utils.options.clone().or_else(|| {
                    Some(Arc::new(LanguageOptions::new(
                        EsGlobals::get().latest,
                        SourceType::Module,
                        [],
                    )))
                })
            })
            .flatten(),
    })
}

/// The kinds a binding's name node has: Python's identifiers and import items; TypeScript's
/// identifiers, type names, enum members and shorthand patterns.
pub(super) fn name_kinds(language: GrammarDialect) -> BTreeSet<u16> {
    match Family::of(language) {
        Family::Python => kind_ids(language, &["identifier", "dotted_name"]),
        Family::Php | Family::Blade => kind_ids(language, &["variable_name"]),
        Family::Ecma => kind_ids(
            language,
            &[
                "identifier",
                "type_identifier",
                "property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
        ),
    }
}

/// A family whose variables are PHP's, as refusals name it.
fn php_family_name(family: Family) -> &'static str {
    match family {
        Family::Blade => "blade",
        Family::Php | Family::Python | Family::Ecma => "php",
    }
}

/// The keys of a `binds` object.
fn binds_keys() -> Vec<&'static str> {
    FILTER_KEYS
        .iter()
        .chain(BINDING_KEYS)
        .chain(&["redefines", OCCURRENCE_KEY])
        .copied()
        .collect()
}

/// The keys of a `redefines` object: `binds`' own but `redefines`, and how the two relate.
fn redefines_keys() -> Vec<&'static str> {
    FILTER_KEYS
        .iter()
        .chain(BINDING_KEYS)
        .chain(&["sameScope", "sameImport", "every"])
        .copied()
        .collect()
}

/// Compiles the sub-rules of a `binds` or `redefines` object; `None` when one cannot match in this
/// grammar.
fn compile_sub_rules(
    object: &Map<String, Value>,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<Option<[Option<AstMatcher>; 3]>> {
    let mut rules = Vec::with_capacity(BINDING_KEYS.len());
    for key in BINDING_KEYS {
        rules.push(
            object
                .get(*key)
                .map(|sub| compile_ast_rule(sub, language, &format!("{path}/{key}"), gaps, utils))
                .transpose()?,
        );
    }
    if rules
        .iter()
        .any(|rule| matches!(rule, Some(AstMatcher::Never)))
    {
        return Ok(None);
    }
    Ok(Some(
        <[_; 3]>::try_from(rules).unwrap_or_else(|_| unreachable!("three binding keys")),
    ))
}

pub(super) fn validate_binds(
    value: &Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    let binds = object(value, path)?;
    reject_unknown(binds, &binds_keys(), path)?;
    validate_filter(binds, path)?;
    for key in BINDING_KEYS.iter().chain(&[OCCURRENCE_KEY]) {
        if let Some(sub) = binds.get(*key) {
            validate_ast_rule(sub, &format!("{path}/{key}"), limits, utils)?;
        }
    }
    if let Some(redefines) = binds.get("redefines") {
        let redefines_path = format!("{path}/redefines");
        let redefines = object(redefines, &redefines_path)?;
        reject_unknown(redefines, &redefines_keys(), &redefines_path)?;
        validate_filter(redefines, &redefines_path)?;
        for key in ["sameScope", "sameImport", "every"] {
            boolean(redefines, key, &redefines_path)?;
        }
        for key in BINDING_KEYS {
            if let Some(sub) = redefines.get(*key) {
                validate_ast_rule(sub, &format!("{redefines_path}/{key}"), limits, utils)?;
            }
        }
    }
    Ok(())
}

/// Compiles a validated `binds` for one grammar.
pub(super) fn compile_binds(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    let binds = object(value, path)?;
    if let Some(redefines) = binds.get("redefines").and_then(Value::as_object) {
        let redefines_path = format!("{path}/redefines");
        if language == GrammarDialect::Python && redefines.contains_key("every") {
            return Err(invalid(format!(
                "{redefines_path}/every: Python's redefines reads the one binding shadowed; `every` is TypeScript's"
            )));
        }
        if redefines.contains_key("sameImport") {
            match Family::of(language) {
                Family::Python => {}
                Family::Php => {
                    return Err(invalid(format!(
                        "{redefines_path}/sameImport: qualified import names are Python's; a PHP variable is never bound by an import"
                    )));
                }
                Family::Blade => {
                    return Err(invalid(format!(
                        "{redefines_path}/sameImport: qualified import names are Python's; a blade variable is never bound by an import"
                    )));
                }
                Family::Ecma => {
                    return Err(invalid(format!(
                        "{redefines_path}/sameImport: qualified import names are Python's; TypeScript has none"
                    )));
                }
            }
        }
    }
    if language == GrammarDialect::Python && binds.contains_key(OCCURRENCE_KEY) {
        return Err(invalid(format!(
            "{path}/occurrence: a variable's occurrences are TypeScript's; Python has none"
        )));
    }
    let filter = compile_filter(binds, language, path, utils)?;
    let Some([site, value, annotation]) = compile_sub_rules(binds, language, path, gaps, utils)?
    else {
        return Ok(AstMatcher::Never);
    };
    let occurrence = binds
        .get(OCCURRENCE_KEY)
        .map(|sub| {
            compile_ast_rule(
                sub,
                language,
                &format!("{path}/{OCCURRENCE_KEY}"),
                gaps,
                utils,
            )
        })
        .transpose()?;
    if matches!(occurrence, Some(AstMatcher::Never)) {
        return Ok(AstMatcher::Never);
    }
    let redefines = match binds.get("redefines") {
        None => None,
        Some(redefines) => {
            let redefines_path = format!("{path}/redefines");
            let redefines = object(redefines, &redefines_path)?;
            let Some([site, value, annotation]) =
                compile_sub_rules(redefines, language, &redefines_path, gaps, utils)?
            else {
                return Ok(AstMatcher::Never);
            };
            Some(Box::new(Redefines {
                filter: compile_filter(redefines, language, &redefines_path, utils)?,
                same_scope: boolean(redefines, "sameScope", &redefines_path)?,
                same_import: boolean(redefines, "sameImport", &redefines_path)?,
                every: redefines.get("every") == Some(&Value::Bool(true)),
                site,
                value,
                annotation,
            }))
        }
    };
    // TypeScript's private names (`#a`) are bindings of their class body.
    let mut kinds = name_kinds(language);
    if Family::of(language) == Family::Ecma {
        kinds.extend(kind_ids(language, &["private_property_identifier"]));
    }
    Ok(AstMatcher::Semantic(Semantic::Binds(Box::new(Binds {
        language,
        name_kinds: kinds,
        options: utils.options.clone(),
        python: utils.python.clone(),
        filter,
        site,
        value,
        annotation,
        occurrence,
        redefines,
    }))))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod ruff;

#[cfg(test)]
mod eslint;
