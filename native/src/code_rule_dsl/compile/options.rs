use super::config::Options;
use super::resolve::{MAX_RESOLVE_NAME_BYTES, is_identifier};
use crate::code_rule_dsl::json::bounded_text;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::language::adapter::QueryWhen;
use crate::code_rule_dsl::language::{
    FileCondition, JsxRuntime, PackageCondition, SourceType, TextCondition,
};
use crate::code_rule_dsl::scope::options::{
    DEFAULT_PYTHON_MINOR, EsGlobals, GlobalSetting, LanguageOptions, PythonOptions, PythonTyping,
    is_ts_lib, python_builtins, ts_lib_globals,
};
use crate::code_rule_dsl::semver::Range;
use napi::Result;
use serde_json::{Map, Value};

const LANGUAGE_OPTIONS_KEYS: &[&str] = &[
    "ecmaVersion",
    "sourceType",
    "globals",
    "parserOptions",
    "jsxPragma",
    "jsxFragmentName",
    "lib",
];

/// typescript-eslint's `jsxPragma` or `jsxFragmentName`: an identifier, or `null` for none.
fn jsx_name(value: Option<&Value>, key: &str, path: &str) -> Result<Option<Option<String>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(Some(None)),
        Value::String(name) if is_identifier(name) && name.len() <= MAX_RESOLVE_NAME_BYTES => {
            Ok(Some(Some(name.clone())))
        }
        _ => Err(invalid(format!(
            "{path}/{key} must be an identifier of at most {MAX_RESOLVE_NAME_BYTES} bytes, or null"
        ))),
    }
}

/// Globals one `languageOptions` may configure: room for real environment unions (ESLint's browser,
/// node and worker sets together hold a few thousand), still bounded.
pub(super) const MAX_LANGUAGE_GLOBALS: usize = 16_384;

/// ESLint's `normalizeEcmaVersionForLanguageOptions` over the versions the table has: `"latest"`,
/// 3, 5, 6 to 17 (2015 to 2026) or a year.
fn ecma_version(value: Option<&Value>, path: &str) -> Result<u16> {
    let Some(value) = value else {
        return Ok(EsGlobals::get().latest);
    };
    ecma_year(value, &format!("{path}/ecmaVersion"))
}

/// One `ecmaVersion` value as its year (3 and 5 stay as they are).
fn ecma_year(value: &Value, path: &str) -> Result<u16> {
    let table = EsGlobals::get();
    let year = match value {
        Value::String(latest) if latest == "latest" => Some(table.latest),
        Value::Number(number) => number
            .as_u64()
            .and_then(|number| u16::try_from(number).ok())
            .map(|number| match number {
                3 | 5 => number,
                6..2015 => number + 2009,
                _ => number,
            })
            .filter(|year| table.version(*year).is_some()),
        _ => None,
    };
    year.ok_or_else(|| {
        let last = table.latest;
        invalid(format!(
            "{path} must be \"latest\", 3, 5, 6 to {}, or 2015 to {last}",
            last - 2009
        ))
    })
}

/// `sourceType`; `None` where the query leaves it to each file (`SourceType::of_file`).
fn source_type(value: Option<&Value>, path: &str) -> Result<Option<SourceType>> {
    match value.map(|value| value.as_str()) {
        None => Ok(None),
        Some(Some("script")) => Ok(Some(SourceType::Script)),
        Some(Some("module")) => Ok(Some(SourceType::Module)),
        Some(Some("commonjs")) => Ok(Some(SourceType::CommonJs)),
        Some(_) => Err(invalid(format!(
            "{path}/sourceType must be \"script\", \"module\" or \"commonjs\""
        ))),
    }
}

/// ESLint's `normalizeConfigGlobal`, aliases included.
fn setting(value: &Value) -> Option<GlobalSetting> {
    match value {
        Value::Bool(true) => Some(GlobalSetting::Writable),
        Value::Bool(false) | Value::Null => Some(GlobalSetting::Readonly),
        Value::String(text) => match text.as_str() {
            "off" => Some(GlobalSetting::Off),
            "true" | "writable" | "writeable" => Some(GlobalSetting::Writable),
            "false" | "readonly" | "readable" => Some(GlobalSetting::Readonly),
            _ => None,
        },
        _ => None,
    }
}

fn globals(value: Option<&Value>, path: &str) -> Result<Vec<(String, GlobalSetting)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let path = format!("{path}/globals");
    let globals = value
        .as_object()
        .ok_or_else(|| invalid(format!("{path} must be an object")))?;
    if globals.len() > MAX_LANGUAGE_GLOBALS {
        return Err(invalid(format!(
            "{path} lists {} names; at most {MAX_LANGUAGE_GLOBALS} are allowed",
            globals.len()
        )));
    }
    globals
        .iter()
        .map(|(name, value)| {
            if !is_identifier(name) || name.len() > MAX_RESOLVE_NAME_BYTES {
                return Err(invalid(format!(
                    "{path}: `{name}` is not an identifier of at most {MAX_RESOLVE_NAME_BYTES} bytes"
                )));
            }
            let setting = setting(value).ok_or_else(|| {
                invalid(format!(
                    "{path}/{name} must be \"readonly\", \"writable\" or \"off\" (or an ESLint alias)"
                ))
            })?;
            Ok((name.clone(), setting))
        })
        .collect()
}

/// ESLint's `parserOptions.ecmaFeatures` that change scoping and strictness: `globalReturn` and
/// `impliedStrict` (both default false). Nothing else of `parserOptions` is read.
fn ecma_features(value: Option<&Value>, path: &str) -> Result<(bool, bool)> {
    let Some(value) = value else {
        return Ok((false, false));
    };
    let path = format!("{path}/parserOptions");
    let parser = object(value, &path)?;
    reject_unknown(parser, &["ecmaFeatures"], &path)?;
    let Some(features) = parser.get("ecmaFeatures") else {
        return Ok((false, false));
    };
    let path = format!("{path}/ecmaFeatures");
    let features = object(features, &path)?;
    reject_unknown(features, &["globalReturn", "impliedStrict"], &path)?;
    let flag = |key: &str| match features.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(invalid(format!("{path}/{key} must be true or false"))),
    };
    Ok((flag("globalReturn")?, flag("impliedStrict")?))
}

/// An ast query's `languageOptions` (TypeScript and JavaScript), with ESLint's defaults for what
/// it leaves out: the latest `ecmaVersion`, no extra `globals`, no `ecmaFeatures`. Without a
/// `sourceType` each file has its own (`SourceType::of_file`), as ESLint's default config gives
/// `.cjs` files `commonjs` and the rest `module`.
pub(crate) fn compile_language_options(value: &Value, path: &str) -> Result<LanguageOptions> {
    let path = format!("{path}/languageOptions");
    let options = object(value, &path)?;
    reject_unknown(options, LANGUAGE_OPTIONS_KEYS, &path)?;
    let ecma_version = ecma_version(options.get("ecmaVersion"), &path)?;
    let source_type = source_type(options.get("sourceType"), &path)?;
    // espree: "sourceType 'module' is not supported when ecmaVersion < 2015". Without a
    // `sourceType`, most files are modules.
    if source_type.is_none_or(|source_type| source_type == SourceType::Module)
        && ecma_version < 2015
    {
        return Err(invalid(format!(
            "{path}: sourceType \"module\" (the default) needs ecmaVersion 2015 or later; ESLint refuses it"
        )));
    }
    let (global_return, implied_strict) = ecma_features(options.get("parserOptions"), &path)?;
    let mut options = LanguageOptions::new(
        ecma_version,
        source_type,
        globals(options.get("globals"), &path)?,
    );
    options.global_return = global_return;
    options.implied_strict = implied_strict;
    if let Some(pragma) = jsx_name(value.get("jsxPragma"), "jsxPragma", &path)? {
        options.jsx_pragma = pragma;
    }
    if let Some(fragment) = jsx_name(value.get("jsxFragmentName"), "jsxFragmentName", &path)? {
        options.jsx_fragment_name = fragment;
    }
    if let Some(libs) = value.get("lib") {
        options.lib = ts_lib_globals(lib_names(libs, &path)?.iter().map(String::as_str));
    }
    Ok(options)
}

/// typescript-eslint's `parserOptions.lib`: lib names, lower-cased as its scope manager reads them,
/// each one it knows.
fn lib_names(value: &Value, path: &str) -> Result<Vec<String>> {
    let path = format!("{path}/lib");
    let Value::Array(items) = value else {
        return Err(invalid(format!("{path} must be an array of lib names")));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let name = item.as_str().map(str::to_lowercase).unwrap_or_default();
            if is_ts_lib(&name) {
                Ok(name)
            } else {
                Err(invalid(format!(
                    "{path}/{index}: `{}` is not a lib typescript-eslint 8.70.1 knows",
                    item.as_str()
                        .map_or_else(|| item.to_string(), str::to_owned)
                )))
            }
        })
        .collect()
}

/// Names `typingModules` or `extendGenerics` may list.
const MAX_TYPING_NAMES: usize = 256;

/// A Python ast query's `languageOptions`: `pythonVersion`, `"3.7"` to `"3.15"`, the versions the
/// pinned builtins table has (Ruff's default, 3.10, when omitted); `typingModules` and
/// `extendGenerics`, Ruff's `lint.typing-modules` and `lint.pyflakes.extend-generics`, each a list
/// of dotted names.
pub(crate) fn compile_python_options(value: &Value, path: &str) -> Result<PythonOptions> {
    let path = format!("{path}/languageOptions");
    let options = object(value, &path)?;
    reject_unknown(
        options,
        &["pythonVersion", "typingModules", "extendGenerics"],
        &path,
    )?;
    let minor = match options.get("pythonVersion") {
        Some(version) => python_minor(version, &format!("{path}/pythonVersion"))?,
        None => DEFAULT_PYTHON_MINOR,
    };
    let typing = PythonTyping {
        modules: dotted_names(
            options.get("typingModules"),
            &format!("{path}/typingModules"),
        )?,
        generics: dotted_names(
            options.get("extendGenerics"),
            &format!("{path}/extendGenerics"),
        )?,
    };
    Ok(PythonOptions {
        minor,
        typing: (typing != PythonTyping::default()).then(|| typing.intern()),
    })
}

/// A list of dotted names (`airflow.typing_compat`), each as its names.
fn dotted_names(value: Option<&Value>, path: &str) -> Result<Vec<Vec<Box<str>>>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid(format!("{path} must be a list of dotted names")))?;
    if items.len() > MAX_TYPING_NAMES {
        return Err(invalid(format!(
            "{path} lists {} names; at most {MAX_TYPING_NAMES} are allowed",
            items.len()
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .filter(|name| {
                    name.len() <= MAX_RESOLVE_NAME_BYTES
                        && name.split('.').all(|part| is_identifier(part) && !part.contains('$'))
                })
                .map(|name| name.split('.').map(Box::from).collect())
                .ok_or_else(|| {
                    invalid(format!(
                        "{path}/{index} must be a dotted name (identifiers joined by .) of at most {MAX_RESOLVE_NAME_BYTES} bytes"
                    ))
                })
        })
        .collect()
}

/// One `pythonVersion` value, `"3.7"` to `"3.15"`, as its minor version.
fn python_minor(version: &Value, path: &str) -> Result<u8> {
    let table = python_builtins();
    let minor = version
        .as_str()
        .and_then(|version| version.strip_prefix("3."))
        .filter(|minor| !minor.starts_with('0') && !minor.starts_with('+'))
        .and_then(|minor| minor.parse::<u8>().ok())
        .filter(|minor| table.contains_key(minor));
    minor.ok_or_else(|| {
        let first = table.keys().next().copied().unwrap_or_default();
        let last = table.keys().next_back().copied().unwrap_or_default();
        invalid(format!(
            "{path} must be a version from \"3.{first}\" to \"3.{last}\""
        ))
    })
}

/// A Python ast query's `when`: the versions it applies to, `{ "pythonVersion": { "min", "max" } }`,
/// read against its own `languageOptions`, else Ruff's default (3.10). Every key must hold.
/// Whether the query applies.
pub(crate) fn compile_python_when(
    value: &Value,
    path: &str,
    python: Option<&PythonOptions>,
    document: &Options,
) -> Result<bool> {
    let path = format!("{path}/when");
    let when = object(value, &path)?;
    reject_unknown(when, &["pythonVersion", "option"], &path)?;
    let applies = document_holds(when, &path, document)?;
    let version = u16::from(python.map_or(DEFAULT_PYTHON_MINOR, |python| python.minor));
    let within = version_within(when, &path, "pythonVersion", version, |value, at| {
        python_minor(value, at).map(u16::from)
    })?;
    Ok(applies && within)
}

/// A TypeScript ast query's `when`: the versions it applies to, `{ "ecmaVersion": { "min", "max" } }`,
/// `sourceType` (one or a list) and `ecmaFeatures` (`globalReturn`, `impliedStrict`: each must
/// equal the query's), read against its own `languageOptions`, else ESLint's defaults (the latest,
/// no features). Every key must hold. Whether the query applies, and the source types a file must
/// have where the query leaves `sourceType` to each file (`SourceType::of_file`).
pub(crate) fn compile_ecma_when(
    value: &Value,
    path: &str,
    options: Option<&LanguageOptions>,
    document: &Options,
) -> Result<QueryWhen> {
    let path = format!("{path}/when");
    let when = object(value, &path)?;
    reject_unknown(
        when,
        &["ecmaVersion", "sourceType", "ecmaFeatures", "option"],
        &path,
    )?;
    let mut applies = document_holds(when, &path, document)?;
    let mut file_source_types = None;
    if let Some(types) = when.get("sourceType") {
        let types = when_source_types(types, &format!("{path}/sourceType"))?;
        match options.and_then(|options| options.source_type) {
            Some(source_type) => applies &= types.contains(&source_type),
            None => file_source_types = Some(types),
        }
    }
    if let Some(features) = when.get("ecmaFeatures") {
        let path = format!("{path}/ecmaFeatures");
        let features = object(features, &path)?;
        reject_unknown(features, &["globalReturn", "impliedStrict"], &path)?;
        if features.is_empty() {
            return Err(invalid(format!(
                "{path} must set `globalReturn`, `impliedStrict` or both"
            )));
        }
        for (name, actual) in [
            (
                "globalReturn",
                options.is_some_and(|options| options.global_return),
            ),
            (
                "impliedStrict",
                options.is_some_and(|options| options.implied_strict),
            ),
        ] {
            match features.get(name) {
                None => {}
                Some(Value::Bool(expected)) => applies &= *expected == actual,
                Some(_) => {
                    return Err(invalid(format!("{path}/{name} must be true or false")));
                }
            }
        }
    }
    let version = options.map_or(EsGlobals::get().latest, |options| options.ecma_version);
    let within = version_within(when, &path, "ecmaVersion", version, ecma_year)?;
    Ok((applies && within, file_source_types))
}

/// The `when` of an ast query whose language has no versions or settings (SQL): the document's
/// options alone, `{ "option": ... }`. Whether the query applies.
pub(crate) fn compile_option_when(value: &Value, path: &str, document: &Options) -> Result<bool> {
    let path = format!("{path}/when");
    let when = object(value, &path)?;
    reject_unknown(when, &["option"], &path)?;
    document_holds(when, &path, document)
}

/// A `when`'s `option`, the document's options (a Rerail extension): whether each named option
/// meets its condition.
fn document_holds(when: &Map<String, Value>, path: &str, document: &Options) -> Result<bool> {
    match when.get("option") {
        Some(option) => document.holds(option, &format!("{path}/option")),
        None => Ok(true),
    }
}

/// Whether `version` lies in a `when`'s `key`, `{ "min", "max" }` as `bound` reads each, both
/// inclusive; `true` without one.
fn version_within(
    when: &Map<String, Value>,
    path: &str,
    key: &str,
    version: u16,
    bound: impl Fn(&Value, &str) -> Result<u16>,
) -> Result<bool> {
    let Some(range) = when.get(key) else {
        return Ok(true);
    };
    let path = format!("{path}/{key}");
    let range = object(range, &path)?;
    reject_unknown(range, &["min", "max"], &path)?;
    if range.is_empty() {
        return Err(invalid(format!("{path} must set `min`, `max` or both")));
    }
    let read = |name: &str| -> Result<Option<u16>> {
        range
            .get(name)
            .map(|value| bound(value, &format!("{path}/{name}")))
            .transpose()
    };
    let (min, max) = (read("min")?, read("max")?);
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(invalid(format!(
            "{path}: min is after max, so the query never applies"
        )));
    }
    Ok(min.is_none_or(|min| min <= version) && max.is_none_or(|max| version <= max))
}

/// `when.sourceType`: one source type or a non-empty list of them.
fn when_source_types(value: &Value, path: &str) -> Result<Vec<SourceType>> {
    let one = |value: &Value| match value.as_str() {
        Some("script") => Some(SourceType::Script),
        Some("module") => Some(SourceType::Module),
        Some("commonjs") => Some(SourceType::CommonJs),
        _ => None,
    };
    let types = match value {
        Value::Array(items) if !items.is_empty() => items.iter().map(one).collect(),
        Value::Array(_) => None,
        value => one(value).map(|one| vec![one]),
    };
    types.ok_or_else(|| {
        invalid(format!(
            "{path} must be \"script\", \"module\" or \"commonjs\", or a non-empty list of them"
        ))
    })
}

/// Which of an ast query's `file` keys its language's family reads (`Adapter::PACKAGES`,
/// `Adapter::DECLARATION_FILES`, `Adapter::PACKAGE_MANIFESTS`, `Adapter::JSX_RUNTIMES`).
pub(super) struct FileKeys {
    /// The module path keys: `module`, `moduleName`, `package`, `inPackage`.
    pub(super) packages: bool,
    /// `declaration`.
    pub(super) declarations: bool,
    /// `package` as conditions on the nearest `package.json`.
    pub(super) manifests: bool,
    /// `jsxRuntime`.
    pub(super) jsx_runtimes: bool,
}

/// An ast query's `file`: conditions on the file. Every language has `path`, a `regex` that must
/// match the path the host sends, a `notRegex` that must not, or both. A family with declaration
/// files has `declaration`, whether the path names one. A family reading package manifests has
/// `package`, conditions on the file's nearest `package.json` (`bin`, `nodeVersion.subsetOf`,
/// `dependsOn`, `path`); one with JSX runtimes `jsxRuntime`, the runtimes the file's JSX may be
/// one of. A family naming modules from package markers has its module path, the host's `module`
/// or the stem fallback: `module`, `moduleName` and `package` hold text conditions over the dotted
/// names, the last one and the first one (in a package), and `inPackage` whether the host sent
/// the path.
pub(super) fn compile_file_condition(
    value: &Value,
    path: &str,
    language: &str,
    keys: FileKeys,
    max_regex_bytes: usize,
) -> Result<FileCondition> {
    let path = format!("{path}/file");
    let file = object(value, &path)?;
    reject_unknown(
        file,
        &[
            "path",
            "declaration",
            "module",
            "moduleName",
            "package",
            "inPackage",
            "jsxRuntime",
        ],
        &path,
    )?;
    // `package` is also a manifest family's, over the nearest `package.json`.
    for key in ["module", "moduleName", "package", "inPackage"] {
        if !keys.packages && (!keys.manifests || key != "package") && file.contains_key(key) {
            return Err(invalid(format!(
                "{path}/{key}: module paths are Python's; {language} has none"
            )));
        }
    }
    if !keys.declarations && file.contains_key("declaration") {
        return Err(invalid(format!(
            "{path}/declaration: declaration files are TypeScript's; {language} has none"
        )));
    }
    if !keys.jsx_runtimes && file.contains_key("jsxRuntime") {
        return Err(invalid(format!(
            "{path}/jsxRuntime: JSX runtimes are TypeScript's; {language} has none"
        )));
    }
    if file.is_empty() {
        let mut allowed = vec!["`path`", "`declaration`"];
        if keys.packages {
            allowed.extend(["`module`", "`moduleName`", "`package`", "`inPackage`"]);
        } else if keys.manifests {
            allowed.push("`package`");
        }
        if keys.jsx_runtimes {
            allowed.push("`jsxRuntime`");
        }
        let last = allowed.pop().unwrap_or_default();
        return Err(invalid(format!(
            "{path} must set {} or {last}",
            allowed.join(", ")
        )));
    }
    let text = |key: &str| text_condition(file.get(key), &format!("{path}/{key}"), max_regex_bytes);
    let flag = |key: &str| flag_value(file.get(key), &format!("{path}/{key}"));
    // `package` is a text condition over a module path, or conditions on a manifest; a family
    // reads one of the two.
    let shaped = |names: &[&str]| {
        file.get("package")
            .and_then(Value::as_object)
            .is_some_and(|package| names.iter().any(|name| package.contains_key(*name)))
    };
    let text_shaped = shaped(&["regex", "notRegex"]);
    let manifest_shaped = shaped(&["bin", "nodeVersion", "dependsOn", "path", "serverGraph"]);
    let (manifest, package) = match file.get("package") {
        Some(_) if keys.manifests && text_shaped => {
            return Err(invalid(format!(
                "{path}/package: {language}'s package reads the nearest package.json: set bin, nodeVersion, dependsOn, path or serverGraph"
            )));
        }
        Some(value) if keys.manifests => (
            Some(package_condition(
                value,
                &format!("{path}/package"),
                max_regex_bytes,
            )?),
            None,
        ),
        Some(_) if keys.packages && manifest_shaped => {
            return Err(invalid(format!(
                "{path}/package: {language}'s package is a regex or notRegex over the top package's name"
            )));
        }
        _ => (None, text("package")?),
    };
    Ok(FileCondition {
        path: text("path")?,
        declaration: flag("declaration")?,
        module: text("module")?,
        module_name: text("moduleName")?,
        package,
        in_package: flag("inPackage")?,
        manifest,
        jsx_runtime: file
            .get("jsxRuntime")
            .map(|value| jsx_runtimes(value, &format!("{path}/jsxRuntime")))
            .transpose()?,
    })
}

/// A `regex`, `notRegex` or both.
fn text_condition(
    value: Option<&Value>,
    at: &str,
    max_regex_bytes: usize,
) -> Result<Option<TextCondition>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let condition = object(value, at)?;
    reject_unknown(condition, &["regex", "notRegex"], at)?;
    if condition.is_empty() {
        return Err(invalid(format!(
            "{at} must set `regex`, `notRegex` or both"
        )));
    }
    let regex = |name: &str| -> Result<Option<regex::Regex>> {
        let Some(value) = condition.get(name) else {
            return Ok(None);
        };
        let at = format!("{at}/{name}");
        let pattern = bounded_text(Some(value), &at, "maxRegexPatternBytes", max_regex_bytes)?;
        regex::Regex::new(pattern)
            .map(Some)
            .map_err(|error| invalid(format!("{at}: {error}")))
    };
    Ok(Some(TextCondition {
        regex: regex("regex")?,
        not_regex: regex("notRegex")?,
    }))
}

fn flag_value(value: Option<&Value>, at: &str) -> Result<Option<bool>> {
    match value {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(invalid(format!("{at} must be true or false"))),
    }
}

/// A TypeScript query's `file.package`: `bin` (true or false), `nodeVersion: { subsetOf }` (an
/// npm semver range), `dependsOn` (a non-empty list of package names), `path` (a text
/// condition) and `serverGraph` (true or false); at least one.
fn package_condition(value: &Value, at: &str, max_regex_bytes: usize) -> Result<PackageCondition> {
    let package = object(value, at)?;
    reject_unknown(
        package,
        &["bin", "nodeVersion", "dependsOn", "path", "serverGraph"],
        at,
    )?;
    if package.is_empty() {
        return Err(invalid(format!(
            "{at} must set `bin`, `nodeVersion`, `dependsOn`, `path` or `serverGraph`"
        )));
    }
    let node_version = package
        .get("nodeVersion")
        .map(|value| {
            let at = format!("{at}/nodeVersion");
            let version = object(value, &at)?;
            reject_unknown(version, &["subsetOf"], &at)?;
            version
                .get("subsetOf")
                .and_then(Value::as_str)
                .and_then(Range::parse)
                .ok_or_else(|| invalid(format!("{at}/subsetOf must be an npm semver range")))
        })
        .transpose()?;
    let depends_on = package
        .get("dependsOn")
        .map(|value| {
            value
                .as_array()
                .filter(|names| !names.is_empty())
                .and_then(|names| {
                    names
                        .iter()
                        .map(|name| {
                            name.as_str()
                                .filter(|name| !name.is_empty())
                                .map(str::to_owned)
                        })
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| {
                    invalid(format!(
                        "{at}/dependsOn must be a non-empty list of package names"
                    ))
                })
        })
        .transpose()?;
    Ok(PackageCondition {
        bin: flag_value(package.get("bin"), &format!("{at}/bin"))?,
        node_version,
        depends_on,
        path: text_condition(package.get("path"), &format!("{at}/path"), max_regex_bytes)?,
        server_graph: flag_value(package.get("serverGraph"), &format!("{at}/serverGraph"))?,
    })
}

/// `file.jsxRuntime`: a non-empty list of `"react"`, `"other"` and `"unknown"`.
fn jsx_runtimes(value: &Value, at: &str) -> Result<Vec<JsxRuntime>> {
    value
        .as_array()
        .filter(|runtimes| !runtimes.is_empty())
        .and_then(|runtimes| {
            runtimes
                .iter()
                .map(|runtime| runtime.as_str().and_then(JsxRuntime::parse))
                .collect()
        })
        .ok_or_else(|| {
            invalid(format!(
                "{at} must be a non-empty list of \"react\", \"other\" and \"unknown\""
            ))
        })
}
