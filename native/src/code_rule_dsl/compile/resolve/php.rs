//! PHP's `resolves` (`FlowResolves`): PHP-Parser's name resolution and PHPStan's variable flow.

use super::*;
use crate::code_rule_dsl::scope::php_names;

/// Compiles a validated `resolves` for PHP. Over class, function and constant names (`import`,
/// `prefix`): the name PHP-Parser's `NameResolver` resolves it to (`scope/php_names.rs`); over
/// the names `use` clauses write (`imported`, `prefix`): what each imports. Over variables:
/// `global`, a superglobal; `unresolved: true`, a read no path defines, as PHPStan's
/// `variable.undefined` reports it (`scope/php.rs`). `binding` compiles apart
/// (`compile_binding`). Blade resolves the names and variables of the PHP its template embeds.
pub fn compile_php_resolves(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    language: GrammarDialect,
    path: &str,
) -> Result<AstMatcher> {
    let keys = object(value, path)?;
    gate_present_keys(pack, value, path, FLOW_RESOLVES_FEATURES)?;
    let variables_asked = php_variables_asked(keys, path)?;
    let unresolved = gated_unresolved(pack, value, path)?;
    let resolves = php_resolves(pack, value, language, path, variables_asked, unresolved)?;
    Ok(AstMatcher::Semantic(Semantic::Resolves(Box::new(resolves))))
}

/// Whether a PHP `resolves` asks about variables rather than class, function and constant names;
/// one that asks about both is refused.
fn php_variables_asked(keys: &Map<String, Value>, path: &str) -> Result<bool> {
    let names_asked = keys.contains_key("import") || keys.contains_key("imported");
    let variables_asked = keys.contains_key("global") || keys.contains_key("unresolved");
    if names_asked && variables_asked {
        return Err(invalid(format!(
            "{path}: `import` and `imported` resolve class, function and constant names; `global` and `unresolved` resolve variables; one `resolves` asks one of them"
        )));
    }
    Ok(variables_asked)
}

/// The `Resolves` of a checked PHP `resolves`.
fn php_resolves(
    pack: &dyn language_pack::LanguagePack, value: &Value,
    language: GrammarDialect,
    path: &str,
    variables_asked: bool,
    unresolved: Option<Unresolved>,
) -> Result<Resolves> {
    let global_any = value.get("global") == Some(&Value::Bool(true));
    let globals = php_superglobals(value.get("global").filter(|_| !global_any), path)?;
    let import_any = value.get("import") == Some(&Value::Bool(true));
    let imports = listed_php_names(value.get("import").filter(|_| !import_any), path, "import")?;
    let imported = listed_php_names(value.get("imported"), path, "imported")?;
    let reference_kinds = if variables_asked {
        kind_ids(language, &["variable_name"])
    } else {
        kind_ids(language, php_names::REFERENCE_KINDS)
    };
    Ok(Resolves {
        globals: globals.into_iter().map(|name| format!("${name}")).collect(),
        imports: imports.into_iter().collect(),
        attribute_kinds: BTreeSet::new(),
        directives: false,
        language,
        reference_kinds,
        paren_kinds: kind_ids(language, pack.scope_paren_kinds()),
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
    })
}

/// The superglobals `{path}/global` lists, refusing the first that is none.
fn php_superglobals<'a>(listed: Option<&'a Value>, path: &str) -> Result<Vec<&'a str>> {
    let globals = listed_names(listed, path, "global", false)?;
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
    Ok(globals)
}

/// The PHP names `{path}/{key}` lists, refusing the first in another family's shape.
fn listed_php_names(listed: Option<&Value>, path: &str, key: &str) -> Result<Vec<String>> {
    let listed = listed_names(listed, path, key, true)?;
    if let Some((index, name)) = listed
        .iter()
        .enumerate()
        .find(|(_, name)| name.contains('.') || name.contains("::"))
    {
        let shape = if name.contains('.') {
            "a dotted name"
        } else {
            "a Rust path"
        };
        return Err(invalid(format!(
            "{path}/{key}/{index}: `{name}` is {shape}; PHP names join labels with `\\`"
        )));
    }
    Ok(listed.into_iter().map(str::to_owned).collect())
}
