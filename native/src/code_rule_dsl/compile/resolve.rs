use super::binds::{FILTER_KEYS, compile_filter, validate_filter};
use super::utils::UtilScope;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::language::adapter::{
    Refusal, ScopeFeature, gate,
};
use crate::code_rule_dsl::language::kind_ids;
use crate::code_rule_dsl::language::{Family, GrammarDialect, language_not_supported};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{AstMatcher, BindingTest, Resolves, Unresolved};
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::Lookup;
use napi::Result;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use crate::code_rule_dsl::language::adapter::pack as language_pack;

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
        .all(crate::code_rule_dsl::identifier::php_label)
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
            // A dotted name is Python's; a path whose identifiers `::` joins, Rust's; PHP names are
            // labels joined by `\`, fully qualified or not. Each family refuses the others' shapes
            // when the rule compiles.
            let valid = if dotted {
                name.split('.').all(is_identifier)
                    || name.split("::").all(is_identifier)
                    || is_php_name(name)
            } else {
                is_identifier(name)
            };
            if !valid || name.len() > MAX_RESOLVE_NAME_BYTES {
                let shape = if dotted {
                    "a dotted name (identifiers joined by `.`), a Rust path (identifiers joined by `::`) or a PHP name (labels joined by `\\`)"
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

/// The refusal at `{at} {message}` of a family's verdict whose message runs on from its key.
fn gate_run_on(at: &str, verdict: std::result::Result<(), Refusal>) -> Result<()> {
    verdict.map_err(|refusal| match refusal {
        Refusal::Refused(message) => invalid(format!("{at} {message}")),
        Refusal::NotSupportedYet(message) => language_not_supported(format!("{at} {message}")),
    })
}

/// How a validated `resolves.binding` object reads the binding, for the family `S` reads.
fn binding_lookup(pack: &dyn language_pack::LanguagePack, binding: &Value, path: &str) -> Result<Lookup> {
    let flag = |name: &str| binding.get(name).and_then(Value::as_bool) == Some(true);
    let lookup = Lookup {
        stop_at_global: flag("stopAtGlobal"),
        only: flag("only"),
    };
    // Blade's `@php` blocks run inside the view's function as PHP's do inside a function.
    if binding.get("stopAtGlobal") == Some(&Value::Bool(false)) {
        gate(
            &format!("{path}/stopAtGlobal"),
            pack.scope_feature(ScopeFeature::ThroughGlobal),
        )?;
    }
    if lookup.stop_at_global {
        gate(
            &format!("{path}/stopAtGlobal"),
            pack.scope_feature(ScopeFeature::StopAtGlobal),
        )?;
    }
    Ok(lookup)
}

/// Compiles `resolves.binding` (and `resolves.write`) from its lookup, filters and compiled
/// sub-rules.
pub(super) fn compile_binding(
    binding: &Value,
    path: &str,
    language: GrammarDialect,
    rules: [Option<AstMatcher>; 3],
    occurrence: Option<AstMatcher>,
    utils: &UtilScope,
    write: bool,
) -> Result<AstMatcher> {
    { let pack = language_pack::pack(Family::of(language)); compile_binding_in(pack, 
        binding, path, language, rules, occurrence, utils, write,
    ) }
}

/// `compile_binding`, for the family `S` reads.
fn compile_binding_in(
    pack: &dyn language_pack::LanguagePack, binding: &Value,
    path: &str,
    language: GrammarDialect,
    [site, value, annotation]: [Option<AstMatcher>; 3],
    occurrence: Option<AstMatcher>,
    utils: &UtilScope,
    write: bool,
) -> Result<AstMatcher> {
    gate_occurrence_and_write(pack, path, occurrence.is_some(), write)?;
    let lookup = binding_lookup(pack, binding, path)?;
    let destructured_value = gated_destructured_value(pack, binding, path)?;
    let filter = compile_filter(pack, object(binding, path)?, language, path, utils)?;
    Ok(AstMatcher::Semantic(Semantic::Binding(Box::new(
        BindingTest {
            lookup,
            language,
            destructured_value: destructured_value.unwrap_or(true),
            write_last: write,
            name_kinds: kind_ids_if(write, language, pack.scope_name_kinds()),
            reference_kinds: kind_ids(language, pack.scope_reference_kinds()),
            paren_kinds: kind_ids(language, pack.scope_paren_kinds()),
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

/// Gates `resolves.binding.occurrence` and `resolves.write` on the family `S` reads.
fn gate_occurrence_and_write(
    pack: &dyn language_pack::LanguagePack, path: &str,
    occurrence: bool,
    write: bool,
) -> Result<()> {
    if occurrence {
        gate(
            &format!("{path}/occurrence"),
            pack.scope_feature(ScopeFeature::Occurrence),
        )?;
    }
    if write {
        let resolves = path.strip_suffix("/binding").unwrap_or(path);
        gate(
            &format!("{resolves}/write"),
            pack.scope_feature(ScopeFeature::LastWrite),
        )?;
    }
    Ok(())
}

/// `resolves.binding.destructuredValue`, gated on the family `S` reads.
fn gated_destructured_value(pack: &dyn language_pack::LanguagePack, binding: &Value, path: &str) -> Result<Option<bool>> {
    let destructured_value = binding.get("destructuredValue").and_then(Value::as_bool);
    if destructured_value.is_some() {
        gate(
            &format!("{path}/destructuredValue"),
            pack.scope_feature(ScopeFeature::DestructuredValue),
        )?;
    }
    Ok(destructured_value)
}

/// The native symbol ids of `kinds` when `asked`; none otherwise.
fn kind_ids_if(asked: bool, language: GrammarDialect, kinds: &[&str]) -> BTreeSet<u16> {
    if asked {
        kind_ids(language, kinds)
    } else {
        BTreeSet::new()
    }
}

/// Compiles a validated `resolves` for one grammar.
pub(super) fn compile_resolves(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<AstMatcher> {
    { let pack = language_pack::pack(Family::of(language)); {
        
        if pack.scope_flow() {
            match FLOW_RESOLVES.get() {
                Some(compile) => compile(pack, value, language, path),
                None => Err(invalid(format!("{path}: no family registered resolves over its flow"))),
            }
        } else {
            compile_lexical_resolves(pack, value, language, path, options, python)
        }
    } }
}

/// Compiles a validated `resolves` for a family whose reads a lexical scope resolves.
fn compile_lexical_resolves(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    language: GrammarDialect,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<AstMatcher> {
    let unresolved = gate_lexical_keys(pack, value, path)?;
    // Asking about every name needs the declared set: the query's settings or their defaults.
    let defaults = value.get("global") == Some(&Value::Bool(true)) || unresolved.is_some();
    let declared_python = defaults.then(|| pack.scope_ruff_options(python)).flatten();
    let python = declared_python.as_ref().or(python);
    let declared_options = defaults.then(|| pack.scope_eslint_options(options)).flatten();
    let options = declared_options.as_ref().or(options);
    check_global_keys(pack, value, path, options, python)?;
    check_imports(pack, value, path)?;
    let resolves = lexical_resolves(pack, value, language, path, unresolved, options, python)?;
    Ok(AstMatcher::Semantic(Semantic::Resolves(Box::new(resolves))))
}

/// Gates the `resolves` keys some lexical family has no counterpart for, and returns what
/// `unresolved` asks for.
fn gate_lexical_keys(pack: &dyn language_pack::LanguagePack, value: &Value, path: &str) -> Result<Option<Unresolved>> {
    if value.get("beforeLocal").is_some() {
        gate_run_on(
            &format!("{path}/beforeLocal"),
            pack.scope_feature(ScopeFeature::BeforeLocal),
        )?;
    }
    let unresolved = gated_unresolved(pack, value, path)?;
    // Keys a family without their counterpart refuses (`ScopeRules::feature`).
    gate_present_keys(pack, 
        value,
        path,
        &[
            ("sloppyFunctions", ScopeFeature::SloppyFunctions),
            ("declared", ScopeFeature::Declared),
            ("relative", ScopeFeature::Relative),
            ("seenBefore", ScopeFeature::SeenBefore),
        ],
    )?;
    Ok(unresolved)
}

/// What `resolves.unresolved` asks for, gated on the family `S` reads.
fn gated_unresolved(pack: &dyn language_pack::LanguagePack, value: &Value, path: &str) -> Result<Option<Unresolved>> {
    let unresolved = match value.get("unresolved") {
        Some(Value::Bool(true)) => Some(Unresolved::Unbound),
        Some(_) => Some(Unresolved::Star),
        None => None,
    };
    if unresolved == Some(Unresolved::Star) {
        gate(
            &format!("{path}/unresolved"),
            pack.scope_feature(ScopeFeature::StarImports),
        )?;
    }
    Ok(unresolved)
}

/// Gates each key of `features` that `value` holds on its feature, in order.
fn gate_present_keys(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    path: &str,
    features: &[(&str, ScopeFeature)],
) -> Result<()> {
    for (key, feature) in features {
        if value.get(*key).is_some() {
            gate(&format!("{path}/{key}"), pack.scope_feature(*feature))?;
        }
    }
    Ok(())
}

/// Checks the `resolves` keys over globals: `this`, `declared`, `scriptDeclarations` and
/// `directives`.
fn check_global_keys(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<()> {
    check_this(pack, value, path, options)?;
    check_declared(value, path, options, python)?;
    check_script_declarations(pack, value, path, options)?;
    gate_present_keys(pack, value, path, &[("directives", ScopeFeature::GlobalComments)])
}

/// Gates `this` and refuses it without the `languageOptions` its global object depends on.
fn check_this(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
) -> Result<()> {
    let this_global = value.get("this").is_some();
    if this_global {
        gate(&format!("{path}/this"), pack.scope_feature(ScopeFeature::This))?;
    }
    if this_global && options.is_none() {
        return Err(invalid(format!(
            "{path}/this: the global object depends on languageOptions.sourceType; declare languageOptions"
        )));
    }
    Ok(())
}

/// Refuses `declared` without the settings that declare the globals.
fn check_declared(
    value: &Value,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<()> {
    let declared = value.get("declared").and_then(Value::as_bool);
    if declared.is_some() && options.is_none() && python.is_none() {
        return Err(invalid(format!(
            "{path}/declared: the declared globals come from languageOptions; declare languageOptions"
        )));
    }
    Ok(())
}

/// Gates `scriptDeclarations` and refuses it without the `languageOptions` that say whether a
/// file is a script.
fn check_script_declarations(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    path: &str,
    options: Option<&Arc<LanguageOptions>>,
) -> Result<()> {
    let script_declarations = value.get("scriptDeclarations").is_some();
    if script_declarations {
        gate(
            &format!("{path}/scriptDeclarations"),
            pack.scope_feature(ScopeFeature::ScriptDeclarations),
        )?;
    }
    if script_declarations && options.is_none() {
        return Err(invalid(format!(
            "{path}/scriptDeclarations: whether a file is a script comes from languageOptions.sourceType; declare languageOptions"
        )));
    }
    Ok(())
}

/// Gates `import` and `imported` and refuses the names they list in another family's shape.
fn check_imports(pack: &dyn language_pack::LanguagePack, value: &Value, path: &str) -> Result<()> {
    for key in ["import", "imported"] {
        let Some(listed) = value.get(key) else {
            continue;
        };
        gate(&format!("{path}/{key}"), pack.scope_feature(ScopeFeature::Imports))?;
        if pack.scope_import_paths() && key == "imported" {
            return Err(invalid(format!(
                "{path}/imported: `imported` matches the names Python's import statements write; a Rust `use` path is no read, and `import` lists the paths reads name through it"
            )));
        }
        check_import_shapes(pack, listed, path, key)?;
    }
    Ok(())
}

/// Refuses the first name `{path}/{key}` lists in another family's import shape.
fn check_import_shapes(pack: &dyn language_pack::LanguagePack, listed: &Value, path: &str, key: &str) -> Result<()> {
    for (index, name) in listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .enumerate()
    {
        let refusal = if pack.scope_import_paths() {
            import_path_refusal(name)
        } else {
            dotted_import_refusal(name)
        };
        if let Some(refusal) = refusal {
            return Err(invalid(format!("{path}/{key}/{index}: {refusal}")));
        }
    }
    Ok(())
}

/// Why a family whose imports are paths refuses `name`, if it does.
fn import_path_refusal(name: &str) -> Option<String> {
    if name.contains('\\') {
        Some(format!(
            "`{name}` is a PHP name; Rust paths join identifiers with `::`"
        ))
    } else if name.contains('.') {
        Some(format!(
            "`{name}` is a dotted name; Rust paths join identifiers with `::`"
        ))
    } else if matches!(name.split("::").next(), Some("self" | "super")) {
        Some(format!(
            "`{name}` starts at a module; a listed path starts at a crate (`crate::`, `std::`), and reads through `self::` and `super::` are normalised to `crate::` paths"
        ))
    } else {
        None
    }
}

/// Why a family whose imports are dotted names refuses `name`, if it does.
fn dotted_import_refusal(name: &str) -> Option<String> {
    if name.contains('\\') {
        Some(format!(
            "`{name}` is a PHP name; Python imports are dotted names"
        ))
    } else if name.contains("::") {
        Some(format!(
            "`{name}` is a Rust path; Python imports are dotted names"
        ))
    } else {
        None
    }
}

/// The names `{path}/{key}` lists; none when `listed` is absent.
fn listed_names<'a>(
    listed: Option<&'a Value>,
    path: &str,
    key: &str,
    dotted: bool,
) -> Result<Vec<&'a str>> {
    Ok(listed
        .map(|listed| names(listed, &format!("{path}/{key}"), dotted))
        .transpose()?
        .unwrap_or_default())
}

/// `listed_names`, as the set a lexical `Resolves` holds.
fn listed_name_set(
    listed: Option<&Value>,
    path: &str,
    key: &str,
    dotted: bool,
) -> Result<BTreeSet<String>> {
    Ok(listed_names(listed, path, key, dotted)?
        .into_iter()
        .map(str::to_owned)
        .collect())
}

/// The `Resolves` of a checked lexical `resolves`, with the declared settings it reads.
fn lexical_resolves(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    language: GrammarDialect,
    path: &str,
    unresolved: Option<Unresolved>,
    options: Option<&Arc<LanguageOptions>>,
    python: Option<&Arc<PythonOptions>>,
) -> Result<Resolves> {
    let this_global = value.get("this").is_some();
    let global_any = value.get("global") == Some(&Value::Bool(true));
    let import_any = value.get("import") == Some(&Value::Bool(true));
    let imported = listed_name_set(value.get("imported"), path, "imported", true)?;
    let seen_before = value.get("seenBefore").is_some();
    let dotted_kinds = kind_ids_if(
        !imported.is_empty() && !seen_before,
        language,
        pack.scope_dotted_name_kinds(),
    );
    let globals = listed_name_set(
        value.get("global").filter(|_| !global_any),
        path,
        "global",
        false,
    )?;
    let imports = listed_name_set(
        value.get("import").filter(|_| !import_any),
        path,
        "import",
        true,
    )?;
    let attribute_kinds = kind_ids(language, pack.scope_attribute_kinds());
    let reference_kinds = lexical_reference_kinds(pack, 
        language,
        &attribute_kinds,
        !imports.is_empty() || import_any,
    );
    Ok(Resolves {
        globals,
        imports,
        attribute_kinds,
        directives: value.get("directives").is_some(),
        language,
        reference_kinds,
        paren_kinds: kind_ids(language, pack.scope_paren_kinds()),
        options: options.cloned(),
        python: python.cloned(),
        this_global,
        this_scope: value.get("this").and_then(Value::as_str) == Some("globalScope"),
        sloppy_functions: value.get("sloppyFunctions").is_some(),
        this_kinds: kind_ids_if(this_global, language, pack.scope_this_kinds()),
        imported,
        declared: value
            .get("declared")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        script_declarations: value.get("scriptDeclarations").is_some(),
        prefix: value.get("prefix").is_some(),
        relative_text: value.get("relative").and_then(Value::as_str) == Some("text"),
        dotted_kinds,
        global_any,
        import_any,
        unresolved,
        string_kinds: kind_ids_if(unresolved.is_some(), language, pack.scope_annotation_string_kinds()),
        before_local: value.get("beforeLocal").is_some(),
        seen_before,
        item: false,
    })
}

/// The kinds of a lexical family's reads; an import resolves a read's attribute chain too.
fn lexical_reference_kinds(
    pack: &dyn language_pack::LanguagePack, language: GrammarDialect,
    attribute_kinds: &BTreeSet<u16>,
    imports_asked: bool,
) -> BTreeSet<u16> {
    let mut reference_kinds = kind_ids(language, pack.scope_reference_kinds());
    if imports_asked {
        reference_kinds.extend(attribute_kinds.iter().copied());
    }
    reference_kinds
}

/// PHP's superglobals, by the names `resolves.global` lists them (without `$`).
const PHP_SUPERGLOBALS: &[&str] = &[
    "GLOBALS", "_SERVER", "_GET", "_POST", "_FILES", "_COOKIE", "_SESSION", "_REQUEST", "_ENV",
];

/// The `resolves` keys a family whose reads follow the file's flow checks, in order, each the
/// feature it asks for.
const FLOW_RESOLVES_FEATURES: &[(&str, ScopeFeature)] = &[
    ("this", ScopeFeature::This),
    ("sloppyFunctions", ScopeFeature::SloppyFunctions),
    ("directives", ScopeFeature::GlobalComments),
    ("declared", ScopeFeature::Declared),
    ("scriptDeclarations", ScopeFeature::ScriptDeclarations),
    ("relative", ScopeFeature::Relative),
    ("seenBefore", ScopeFeature::SeenBefore),
    ("beforeLocal", ScopeFeature::BeforeLocal),
];

/// What compiles a validated `resolves` of a family whose reads its flow resolves (`LanguagePack::scope_flow`: PHP's).
pub(crate) type FlowResolves =
    fn(&dyn language_pack::LanguagePack, &Value, GrammarDialect, &str) -> Result<AstMatcher>;

static FLOW_RESOLVES: std::sync::OnceLock<FlowResolves> = std::sync::OnceLock::new();

/// Registers what compiles a flow family's `resolves` (once, at load).
pub(crate) fn install_flow_resolves(compile: FlowResolves) {
    let _ = FLOW_RESOLVES.set(compile);
}

pub(crate) mod php;
#[cfg(test)]
mod tests;
