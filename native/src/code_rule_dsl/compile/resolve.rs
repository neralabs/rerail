use super::binds::{FILTER_KEYS, compile_filter, name_kinds, validate_filter};
use super::utils::UtilScope;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::language::{SourceType, kind_ids};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{AstMatcher, BindingTest, Resolves, Unresolved};
use crate::code_rule_dsl::scope::options::{
    DEFAULT_PYTHON_MINOR, EsGlobals, LanguageOptions, PythonOptions,
};
use crate::code_rule_dsl::scope::{Lookup, php_names};
use napi::Result;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Names one `resolves` list may hold.
pub(super) const MAX_RESOLVE_NAMES: usize = 1024;
/// Bytes one name may hold.
pub(super) const MAX_RESOLVE_NAME_BYTES: usize = 256;

const RESOLVES_KEYS: &[&str] = &[
    "global",
    "import",
    "imported",
    "prefix",
    "this",
    "directives",
    "declared",
    "scriptDeclarations",
    "sloppyFunctions",
    "relative",
    "binding",
    "unresolved",
    "write",
    "beforeLocal",
    "seenBefore",
];

/// The nodes of a binding `resolves.binding` sub-rules may match.
pub(super) const BINDING_KEYS: &[&str] = &["site", "value", "annotation"];

/// How `resolves.binding` reads the binding, beside its sub-rules: `stopAtGlobal` (Ruff's
/// `lookup_symbol`), `only` (Ruff's `only_binding`) and `destructuredValue` (TypeScript: whether a
/// destructured name carries its literal element as its value).
const BINDING_FLAGS: &[&str] = &["stopAtGlobal", "only", "destructuredValue"];

/// The sub-rule over a TypeScript variable's definition names and references.
pub(super) const OCCURRENCE_KEY: &str = "occurrence";

/// An identifier: ASCII letters, digits, `_` and `$`, not starting with a digit.
pub(super) fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A PHP class, function or constant name: labels joined by `\`, with an optional leading `\`.
fn is_php_name(name: &str) -> bool {
    name.strip_prefix('\\')
        .unwrap_or(name)
        .split('\\')
        .all(php_names::is_label)
}

fn names<'a>(value: &'a Value, path: &str, dotted: bool) -> Result<Vec<&'a str>> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid(format!("{path} must be an array of names")))?;
    if items.is_empty() {
        return Err(invalid(format!("{path} must not be empty")));
    }
    if items.len() > MAX_RESOLVE_NAMES {
        return Err(invalid(format!(
            "{path} lists {} names; at most {MAX_RESOLVE_NAMES} are allowed",
            items.len()
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let name = item
                .as_str()
                .ok_or_else(|| invalid(format!("{path}/{index} must be a string")))?;
            // A dotted name is Python's; PHP names are labels joined by `\`, fully qualified or
            // not. Each family refuses the other's shape when the rule compiles.
            let valid = if dotted {
                name.split('.').all(is_identifier) || is_php_name(name)
            } else {
                is_identifier(name)
            };
            if !valid || name.len() > MAX_RESOLVE_NAME_BYTES {
                let shape = if dotted {
                    "a dotted name (identifiers joined by `.`) or a PHP name (labels joined by `\\`)"
                } else {
                    "an identifier"
                };
                return Err(invalid(format!(
                    "{path}/{index}: `{name}` is not {shape} of at most {MAX_RESOLVE_NAME_BYTES} bytes"
                )));
            }
            Ok(name)
        })
        .collect()
}

/// The grammar-independent checks of a `resolves` object.
pub(super) fn validate_resolves(value: &Value, path: &str) -> Result<()> {
    let resolves = object(value, path)?;
    reject_unknown(resolves, RESOLVES_KEYS, path)?;
    if let Some(write) = resolves.get("write") {
        if write.as_str() != Some("last") {
            return Err(invalid(format!("{path}/write must be \"last\"")));
        }
        if let Some(other) = resolves
            .keys()
            .find(|key| !matches!(key.as_str(), "write" | "binding"))
        {
            return Err(invalid(format!(
                "{path}/{other}: `write` asks where ESLint reports a variable; filter it with \
                 `binding`, and write other `resolves` terms in a resolves of their own"
            )));
        }
        if resolves.get("binding").is_none() {
            return Ok(());
        }
    }
    if let Some(binding) = resolves.get("binding") {
        let binding_path = format!("{path}/binding");
        let rules = object(binding, &binding_path)?;
        let keys = BINDING_KEYS
            .iter()
            .chain(FILTER_KEYS)
            .chain(BINDING_FLAGS)
            .chain(&[OCCURRENCE_KEY])
            .copied()
            .collect::<Vec<_>>();
        reject_unknown(rules, &keys, &binding_path)?;
        for flag in BINDING_FLAGS {
            if rules.get(*flag).is_some_and(|value| !value.is_boolean()) {
                return Err(invalid(format!("{binding_path}/{flag} must be a boolean")));
            }
        }
        validate_filter(rules, &binding_path)?;
        if let Some(other) = resolves
            .keys()
            .find(|key| !matches!(key.as_str(), "binding" | "write"))
        {
            return Err(invalid(format!(
                "{path}/{other}: `binding` asks about the name's own file-local binding; write other \
                 `resolves` terms in a resolves of their own"
            )));
        }
        return Ok(());
    }
    if let Some(seen) = resolves.get("seenBefore") {
        if seen != &Value::Bool(true) {
            return Err(invalid(format!("{path}/seenBefore must be true")));
        }
        let Some(imported) = resolves.get("imported") else {
            return Err(invalid(format!(
                "{path}/seenBefore needs `imported` modules"
            )));
        };
        for (index, module) in names(imported, &format!("{path}/imported"), true)?
            .into_iter()
            .enumerate()
        {
            if module.contains('.') {
                return Err(invalid(format!(
                    "{path}/imported/{index}: `{module}` is not a top-level module; `seenBefore` asks \
                     about modules by their first name, as Ruff's `seen_module` does"
                )));
            }
        }
        if let Some(other) = resolves
            .keys()
            .find(|key| !matches!(key.as_str(), "seenBefore" | "imported"))
        {
            return Err(invalid(format!(
                "{path}/{other}: `seenBefore` asks whether an import of a listed module was seen; \
                 write other `resolves` terms in a resolves of their own"
            )));
        }
        return Ok(());
    }
    if let Some(before) = resolves.get("beforeLocal") {
        if before != &Value::Bool(true) {
            return Err(invalid(format!("{path}/beforeLocal must be true")));
        }
        if let Some(other) = resolves.keys().find(|key| *key != "beforeLocal") {
            return Err(invalid(format!(
                "{path}/{other}: `beforeLocal` asks about a read before the function binds the name; \
                 write other `resolves` terms in a resolves of their own"
            )));
        }
        return Ok(());
    }
    if let Some(unresolved) = resolves.get("unresolved") {
        if unresolved != &Value::Bool(true) && unresolved.as_str() != Some("star") {
            return Err(invalid(format!(
                "{path}/unresolved must be true or \"star\""
            )));
        }
        if let Some(other) = resolves
            .keys()
            .find(|key| !matches!(key.as_str(), "unresolved" | "directives"))
        {
            return Err(invalid(format!(
                "{path}/{other}: `unresolved` asks about names with no binding; write other \
                 `resolves` terms in a resolves of their own"
            )));
        }
        if let Some(directives) = resolves.get("directives")
            && directives.as_str() != Some("eslint")
        {
            return Err(invalid(format!(
                "{path}/directives must be \"eslint\" (honor /* global */ comments)"
            )));
        }
        return Ok(());
    }
    let global = resolves.get("global");
    let import = resolves.get("import");
    let imported = resolves.get("imported");
    let this = resolves.get("this");
    if global.is_none() && import.is_none() && imported.is_none() && this.is_none() {
        return Err(invalid(format!(
            "{path} must list `global` names, `import` names, `imported` names, `this`, \
             `unresolved`, a `binding`, `write`, or `beforeLocal`"
        )));
    }
    let global_any = global == Some(&Value::Bool(true));
    let import_any = import == Some(&Value::Bool(true));
    if let Some(imported) = imported {
        names(imported, &format!("{path}/imported"), true)?;
    }
    if let Some(relative) = resolves.get("relative") {
        if !matches!(relative.as_str(), Some("never" | "text")) {
            return Err(invalid(format!(
                "{path}/relative must be \"never\" or \"text\""
            )));
        }
        if imported.is_none() {
            return Err(invalid(format!(
                "{path}/relative: only `imported` names import sites; list some"
            )));
        }
    }
    if let Some(prefix) = resolves.get("prefix") {
        if prefix != &Value::Bool(true) {
            return Err(invalid(format!("{path}/prefix must be true")));
        }
        if import_any {
            return Err(invalid(format!(
                "{path}/prefix: `import: true` matches every qualified name; it lists none to prefix"
            )));
        }
        if import.is_none() && imported.is_none() {
            return Err(invalid(format!(
                "{path}/prefix: only `import` and `imported` names are dotted; list some"
            )));
        }
    }
    if let Some(this) = this
        && !matches!(this.as_str(), Some("global" | "globalScope"))
    {
        return Err(invalid(format!(
            "{path}/this must be \"global\" or \"globalScope\""
        )));
    }
    if let Some(global) = global.filter(|_| !global_any) {
        if !global.is_array() {
            return Err(invalid(format!(
                "{path}/global must be true or an array of names"
            )));
        }
        names(global, &format!("{path}/global"), false)?;
    }
    if let Some(import) = import.filter(|_| !import_any) {
        if !import.is_array() {
            return Err(invalid(format!(
                "{path}/import must be true or an array of names"
            )));
        }
        names(import, &format!("{path}/import"), true)?;
    }
    let declared = resolves.get("declared");
    if let Some(declared) = declared {
        if !declared.is_boolean() {
            return Err(invalid(format!("{path}/declared must be true or false")));
        }
        if global_any {
            return Err(invalid(format!(
                "{path}/declared: `global: true` asks for declared globals and builtins; it has no names to undeclare"
            )));
        }
        if global.is_none() {
            return Err(invalid(format!(
                "{path}/declared: only `global` names are declared; list some"
            )));
        }
    }
    if let Some(sloppy) = resolves.get("sloppyFunctions") {
        if sloppy != &Value::Bool(true) {
            return Err(invalid(format!("{path}/sloppyFunctions must be true")));
        }
        if this.and_then(Value::as_str) != Some("global") {
            return Err(invalid(format!(
                "{path}/sloppyFunctions: only `this: \"global\"` reads a function's `this`"
            )));
        }
    }
    if let Some(script) = resolves.get("scriptDeclarations") {
        if script != &Value::Bool(true) {
            return Err(invalid(format!("{path}/scriptDeclarations must be true")));
        }
        if global.is_none() {
            return Err(invalid(format!(
                "{path}/scriptDeclarations: only `global` names resolve to declarations; list some"
            )));
        }
    }
    if let Some(directives) = resolves.get("directives") {
        if directives.as_str() != Some("eslint") {
            return Err(invalid(format!(
                "{path}/directives must be \"eslint\" (honor /* global */ comments)"
            )));
        }
        if global.is_none() {
            return Err(invalid(format!(
                "{path}/directives: /* global */ comments only add `global` names; list some"
            )));
        }
        if declared == Some(&Value::Bool(false)) {
            return Err(invalid(format!(
                "{path}/directives: /* global */ comments decide what is declared, which `declared: false` ignores"
            )));
        }
    }
    Ok(())
}

/// The kinds a reference has in a grammar: what `global` and `binding` resolve.
fn reference_kinds(language: GrammarDialect) -> BTreeSet<u16> {
    match Family::of(language) {
        Family::Python => kind_ids(language, &["identifier"]),
        Family::Php | Family::Blade => kind_ids(language, &["variable_name"]),
        Family::Ecma => kind_ids(
            language,
            &[
                "identifier",
                "type_identifier",
                "shorthand_property_identifier",
                "shorthand_property_identifier_pattern",
                "undefined",
            ],
        ),
    }
}

/// A family as refusals name it.
fn family_name(family: Family) -> &'static str {
    match family {
        Family::Python => "Python",
        Family::Ecma => "TypeScript",
        Family::Php => "php",
        Family::Blade => "blade",
    }
}

/// How a validated `resolves.binding` object reads the binding, for one grammar.
fn binding_lookup(binding: &Value, language: GrammarDialect, path: &str) -> Result<Lookup> {
    let flag = |name: &str| binding.get(name).and_then(Value::as_bool) == Some(true);
    let lookup = Lookup {
        stop_at_global: flag("stopAtGlobal"),
        only: flag("only"),
    };
    let family = Family::of(language);
    match family {
        Family::Python => {}
        // Blade's `@php` blocks run inside the view's function as PHP's do inside a function.
        Family::Php | Family::Blade => {
            // PHP's flow already stops at a `global` statement: the read's binding is the
            // statement's. Which top-level binding the global variable holds depends on when the
            // function runs, so reading through it is refused.
            if binding.get("stopAtGlobal") == Some(&Value::Bool(false)) {
                let family = family_name(family);
                return Err(invalid(format!(
                    "{path}/stopAtGlobal: a {family} read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so {family} has no lookup through it"
                )));
            }
        }
        Family::Ecma if lookup.stop_at_global => {
            return Err(invalid(format!(
                "{path}/stopAtGlobal: `global` statements are Python's; TypeScript has none"
            )));
        }
        Family::Ecma => {}
    }
    Ok(lookup)
}

/// Compiles `resolves.binding` (and `resolves.write`) from its lookup, filters and compiled
/// sub-rules.
pub(super) fn compile_binding(
    binding: &Value,
    path: &str,
    language: GrammarDialect,
    [site, value, annotation]: [Option<AstMatcher>; 3],
    occurrence: Option<AstMatcher>,
    utils: &UtilScope,
    write: bool,
) -> Result<AstMatcher> {
    let family = Family::of(language);
    if occurrence.is_some() && family == Family::Python {
        return Err(invalid(format!(
            "{path}/occurrence: a variable's occurrences are TypeScript's; Python has none"
        )));
    }
    if write && family != Family::Ecma {
        let resolves = path.strip_suffix("/binding").unwrap_or(path);
        let detail = match family {
            Family::Php => {
                "ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which `binds: { used: false }` matches"
            }
            Family::Blade => {
                "ESLint reports an unused variable once, at its last write; blade reads its embedded PHP, whose usage oracle, Psalm, reports each unused assignment, which `binds: { used: false }` matches"
            }
            Family::Python | Family::Ecma => {
                "where ESLint reports a variable is TypeScript's; Python has none"
            }
        };
        return Err(invalid(format!("{resolves}/write: {detail}")));
    }
    let lookup = binding_lookup(binding, language, path)?;
    let destructured_value = binding.get("destructuredValue").and_then(Value::as_bool);
    if destructured_value.is_some() && family == Family::Python {
        return Err(invalid(format!(
            "{path}/destructuredValue: destructured values are TypeScript's; a Python unpacked name keeps its element"
        )));
    }
    let filter = compile_filter(object(binding, path)?, language, path, utils)?;
    Ok(AstMatcher::Semantic(Semantic::Binding(Box::new(
        BindingTest {
            lookup,
            language,
            destructured_value: destructured_value.unwrap_or(true),
            write_last: write,
            name_kinds: if write {
                name_kinds(language)
            } else {
                BTreeSet::new()
            },
            reference_kinds: reference_kinds(language),
            paren_kinds: kind_ids(language, &["parenthesized_expression"]),
            options: utils.options.clone(),
            python: utils.python.clone(),
            filter,
            site,
            value,
            annotation,
            occurrence,
        },
    ))))
}

/// Compiles a validated `resolves` for one grammar.
pub(super) fn compile_resolves(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<AstMatcher> {
    if Family::of(language).reads_php() {
        return compile_php_resolves(value, language, path);
    }
    let directives = value.get("directives").is_some();
    let this_global = value.get("this").is_some();
    let global_any = value.get("global") == Some(&Value::Bool(true));
    let import_any = value.get("import") == Some(&Value::Bool(true));
    let unresolved = match value.get("unresolved") {
        Some(Value::Bool(true)) => Some(Unresolved::Unbound),
        Some(_) => Some(Unresolved::Star),
        None => None,
    };
    let before_local = value.get("beforeLocal").is_some();
    if before_local && language != GrammarDialect::Python {
        return Err(invalid(format!(
            "{path}/beforeLocal reads Python's binding order; TypeScript has none"
        )));
    }
    if unresolved == Some(Unresolved::Star) && language != GrammarDialect::Python {
        return Err(invalid(format!(
            "{path}/unresolved: \"star\" asks whether a Python star import binds the name; TypeScript has none"
        )));
    }
    // Asking about every name needs the declared set: the query's options or their defaults.
    let defaults = global_any || unresolved.is_some();
    let default_python = (defaults && language == GrammarDialect::Python && python.is_none())
        .then(|| Arc::new(PythonOptions::new(DEFAULT_PYTHON_MINOR)));
    let python = python.or(default_python.as_ref());
    let default_options = (defaults && language != GrammarDialect::Python && options.is_none())
        .then(|| {
            Arc::new(LanguageOptions::new(
                EsGlobals::get().latest,
                SourceType::Module,
                [],
            ))
        });
    let options = options.or(default_options.as_ref());
    if this_global && language == GrammarDialect::Python {
        return Err(invalid(format!(
            "{path}/this: `this` is TypeScript's; Python has none"
        )));
    }
    if this_global && options.is_none() {
        return Err(invalid(format!(
            "{path}/this: the global object depends on languageOptions.sourceType; declare languageOptions"
        )));
    }
    let declared = value.get("declared").and_then(Value::as_bool);
    if declared.is_some() && options.is_none() && python.is_none() {
        return Err(invalid(format!(
            "{path}/declared: the declared globals come from languageOptions; declare languageOptions"
        )));
    }
    let script_declarations = value.get("scriptDeclarations").is_some();
    if script_declarations && language == GrammarDialect::Python {
        return Err(invalid(format!(
            "{path}/scriptDeclarations: scripts are ESLint's; Python has none"
        )));
    }
    if script_declarations && options.is_none() {
        return Err(invalid(format!(
            "{path}/scriptDeclarations: whether a file is a script comes from languageOptions.sourceType; declare languageOptions"
        )));
    }
    if directives && language == GrammarDialect::Python {
        return Err(invalid(format!(
            "{path}/directives: /* global */ comments are ESLint's; Python has none"
        )));
    }
    for key in ["import", "imported"] {
        let Some(listed) = value.get(key) else {
            continue;
        };
        if language != GrammarDialect::Python {
            return Err(invalid(format!(
                "{path}/{key}: import resolution is Python-only for now; TypeScript rules can use `global`"
            )));
        }
        if let Some((index, name)) = listed
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .enumerate()
            .find(|(_, name)| name.contains('\\'))
        {
            return Err(invalid(format!(
                "{path}/{key}/{index}: `{name}` is a PHP name; Python imports are dotted names"
            )));
        }
    }
    let imported = value
        .get("imported")
        .map(|imported| names(imported, &format!("{path}/imported"), true))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let seen_before = value.get("seenBefore").is_some();
    let dotted_kinds = if imported.is_empty() || seen_before {
        BTreeSet::new()
    } else {
        kind_ids(language, &["dotted_name"])
    };
    let globals = value
        .get("global")
        .filter(|_| !global_any)
        .map(|global| names(global, &format!("{path}/global"), false))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let imports = value
        .get("import")
        .filter(|_| !import_any)
        .map(|import| names(import, &format!("{path}/import"), true))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let reference_kinds = match language {
        GrammarDialect::Python if !imports.is_empty() || import_any => {
            kind_ids(language, &["identifier", "attribute"])
        }
        _ => reference_kinds(language),
    };
    let attribute_kinds = kind_ids(language, &["attribute"]);
    let paren_kinds = kind_ids(language, &["parenthesized_expression"]);
    let this_kinds = if this_global {
        kind_ids(language, &["this"])
    } else {
        BTreeSet::new()
    };
    Ok(AstMatcher::Semantic(Semantic::Resolves(Box::new(
        Resolves {
            globals,
            imports,
            attribute_kinds,
            directives,
            language,
            reference_kinds,
            paren_kinds,
            options: options.cloned(),
            python: python.cloned(),
            this_global,
            this_scope: value.get("this").and_then(Value::as_str) == Some("globalScope"),
            sloppy_functions: value.get("sloppyFunctions").is_some(),
            this_kinds,
            imported,
            declared: declared.unwrap_or(true),
            script_declarations,
            prefix: value.get("prefix").is_some(),
            relative_text: value.get("relative").and_then(Value::as_str) == Some("text"),
            dotted_kinds,
            global_any,
            import_any,
            unresolved,
            string_kinds: if unresolved.is_some() && language == GrammarDialect::Python {
                kind_ids(language, &["string", "concatenated_string"])
            } else {
                BTreeSet::new()
            },
            before_local,
            seen_before,
            item: false,
        },
    ))))
}

/// PHP's superglobals, by the names `resolves.global` lists them (without `$`).
const PHP_SUPERGLOBALS: &[&str] = &[
    "GLOBALS", "_SERVER", "_GET", "_POST", "_FILES", "_COOKIE", "_SESSION", "_REQUEST", "_ENV",
];

/// The `resolves` keys PHP has no counterpart for, each with why.
const PHP_REFUSED_RESOLVES: &[(&str, &str)] = &[
    (
        "this",
        "`this` asks whether JavaScript's `this` is the global object; PHP's `$this` never is: outside an object context it is undefined, which `unresolved: true` reports",
    ),
    (
        "sloppyFunctions",
        "sloppy-mode functions are JavaScript's; PHP has no strict and sloppy modes",
    ),
    (
        "directives",
        "/* global */ comments are ESLint's; PHP's superglobals are fixed, and a function declares a global with a `global` statement, which `binding` reads",
    ),
    (
        "declared",
        "declared globals come from ESLint's and Ruff's configuration; PHP's superglobals are always declared, and no languageOptions configure others",
    ),
    (
        "scriptDeclarations",
        "scripts are ESLint's; a PHP file has no script mode, and a superglobal is never a file's declaration",
    ),
    (
        "relative",
        "a `use` statement names what it imports fully qualified (`use namespace\\A;` does not parse); `import` resolves a `namespace\\A` reference",
    ),
    (
        "seenBefore",
        "Ruff's `seen_module` asks whether a module was imported before; a PHP `use` only aliases a name for the rest of its namespace and loads nothing",
    ),
    (
        "beforeLocal",
        "a PHP function sees no variable of an enclosing scope, so a read before the function binds the name is undefined, which `unresolved: true` reports",
    ),
];

/// Compiles a validated `resolves` for PHP. Over class, function and constant names (`import`,
/// `prefix`): the name PHP-Parser's `NameResolver` resolves it to (`scope/php_names.rs`); over
/// the names `use` clauses write (`imported`, `prefix`): what each imports. Over variables:
/// `global`, a superglobal; `unresolved: true`, a read no path defines, as PHPStan's
/// `variable.undefined` reports it (`scope/php.rs`). `binding` compiles apart
/// (`compile_binding`).
fn compile_php_resolves(value: &Value, language: GrammarDialect, path: &str) -> Result<AstMatcher> {
    // Blade resolves the names and variables of the PHP its template embeds.
    let family = family_name(Family::of(language));
    let keys = object(value, path)?;
    if let Some((key, why)) = PHP_REFUSED_RESOLVES
        .iter()
        .find(|(key, _)| keys.contains_key(*key))
    {
        return Err(invalid(format!("{path}/{key}: {why}; {family} has none")));
    }
    let names_asked = keys.contains_key("import") || keys.contains_key("imported");
    let variables_asked = keys.contains_key("global") || keys.contains_key("unresolved");
    if names_asked && variables_asked {
        return Err(invalid(format!(
            "{path}: `import` and `imported` resolve class, function and constant names; `global` and `unresolved` resolve variables; one `resolves` asks one of them"
        )));
    }
    let unresolved = match value.get("unresolved") {
        Some(Value::Bool(true)) => Some(Unresolved::Unbound),
        Some(_) => {
            return Err(invalid(format!(
                "{path}/unresolved: {family} policies ask `unresolved: true`"
            )));
        }
        None => None,
    };
    let global_any = value.get("global") == Some(&Value::Bool(true));
    let globals = value
        .get("global")
        .filter(|_| !global_any)
        .map(|global| names(global, &format!("{path}/global"), false))
        .transpose()?
        .unwrap_or_default();
    if let Some((index, name)) = globals
        .iter()
        .enumerate()
        .find(|(_, name)| !PHP_SUPERGLOBALS.contains(name))
    {
        return Err(invalid(format!(
            "{path}/global/{index}: `{name}` is no PHP superglobal; expected one of {}",
            PHP_SUPERGLOBALS.join(", ")
        )));
    }
    let import_any = value.get("import") == Some(&Value::Bool(true));
    let listed_names = |key: &str, listed: Option<&Value>| -> Result<Vec<String>> {
        let listed = listed
            .map(|listed| names(listed, &format!("{path}/{key}"), true))
            .transpose()?
            .unwrap_or_default();
        if let Some((index, name)) = listed
            .iter()
            .enumerate()
            .find(|(_, name)| name.contains('.'))
        {
            return Err(invalid(format!(
                "{path}/{key}/{index}: `{name}` is a dotted name; PHP names join labels with `\\`"
            )));
        }
        Ok(listed.into_iter().map(str::to_owned).collect())
    };
    let imports = listed_names("import", value.get("import").filter(|_| !import_any))?;
    let imported = listed_names("imported", value.get("imported"))?;
    let reference_kinds = if variables_asked {
        kind_ids(language, &["variable_name"])
    } else {
        kind_ids(language, php_names::REFERENCE_KINDS)
    };
    Ok(AstMatcher::Semantic(Semantic::Resolves(Box::new(
        Resolves {
            globals: globals.into_iter().map(|name| format!("${name}")).collect(),
            imports: imports.into_iter().collect(),
            attribute_kinds: BTreeSet::new(),
            directives: false,
            language,
            reference_kinds,
            paren_kinds: kind_ids(language, &["parenthesized_expression"]),
            options: None,
            python: None,
            this_global: false,
            this_scope: false,
            sloppy_functions: false,
            this_kinds: BTreeSet::new(),
            imported: imported.into_iter().collect(),
            declared: true,
            script_declarations: false,
            prefix: value.get("prefix").is_some(),
            relative_text: false,
            dotted_kinds: BTreeSet::new(),
            global_any,
            import_any,
            unresolved,
            string_kinds: BTreeSet::new(),
            before_local: false,
            seen_before: false,
            item: false,
        },
    ))))
}

#[cfg(test)]
mod tests;
