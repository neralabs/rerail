use super::resolve::{MAX_RESOLVE_NAME_BYTES, is_identifier};
use crate::policy_dsl::json::bounded_text;
use crate::policy_dsl::json::{invalid, object, reject_unknown};
use crate::policy_dsl::language::{FileCondition, SourceType, TextCondition};
use crate::policy_dsl::scope::options::{
    DEFAULT_PYTHON_MINOR, EsGlobals, GlobalSetting, LanguageOptions, PythonOptions, PythonTyping,
    is_ts_lib, python_builtins, ts_lib_globals,
};
use napi::Result;
use serde_json::Value;

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

fn source_type(value: Option<&Value>, path: &str) -> Result<SourceType> {
    match value.map(|value| value.as_str()) {
        None => Ok(SourceType::Module),
        Some(Some("script")) => Ok(SourceType::Script),
        Some(Some("module")) => Ok(SourceType::Module),
        Some(Some("commonjs")) => Ok(SourceType::CommonJs),
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
/// it leaves out: the latest `ecmaVersion`, a `module`, no extra `globals`, no `ecmaFeatures`.
pub(super) fn compile_language_options(value: &Value, path: &str) -> Result<LanguageOptions> {
    let path = format!("{path}/languageOptions");
    let options = object(value, &path)?;
    reject_unknown(options, LANGUAGE_OPTIONS_KEYS, &path)?;
    let ecma_version = ecma_version(options.get("ecmaVersion"), &path)?;
    let source_type = source_type(options.get("sourceType"), &path)?;
    // espree: "sourceType 'module' is not supported when ecmaVersion < 2015".
    if source_type == SourceType::Module && ecma_version < 2015 {
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
pub(super) fn compile_python_options(value: &Value, path: &str) -> Result<PythonOptions> {
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

/// An ast query's `when`: the versions it applies to, `{ "pythonVersion": { "min", "max" } }`
/// (Python) or `{ "ecmaVersion": { "min", "max" } }` (TypeScript), both bounds inclusive; for
/// TypeScript also `sourceType` (one or a list) and `ecmaFeatures` (`globalReturn`,
/// `impliedStrict`: each must equal the query's). It reads the query's own `languageOptions`,
/// else the defaults (Python 3.10; ESLint's latest, a module, no features). Every key must hold.
/// Whether the query applies.
pub(super) fn compile_when(
    value: &Value,
    path: &str,
    python: bool,
    options: Option<&LanguageOptions>,
    python_options: Option<&PythonOptions>,
    document: &super::config::Options,
) -> Result<bool> {
    let path = format!("{path}/when");
    let when = object(value, &path)?;
    let key = if python {
        "pythonVersion"
    } else {
        "ecmaVersion"
    };
    let keys: &[&str] = if python {
        &[key, "option"]
    } else {
        &[key, "sourceType", "ecmaFeatures", "option"]
    };
    reject_unknown(when, keys, &path)?;
    // The document's options (a Rerail extension): each named option meets its condition.
    let mut applies = match when.get("option") {
        Some(option) => document.holds(option, &format!("{path}/option"))?,
        None => true,
    };
    if !python {
        let source_type = options.map_or(SourceType::Module, |options| options.source_type);
        if let Some(types) = when.get("sourceType") {
            applies &=
                when_source_types(types, &format!("{path}/sourceType"))?.contains(&source_type);
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
    }
    let Some(range) = when.get(key) else {
        return Ok(applies);
    };
    let path = format!("{path}/{key}");
    let range = object(range, &path)?;
    reject_unknown(range, &["min", "max"], &path)?;
    if range.is_empty() {
        return Err(invalid(format!("{path} must set `min`, `max` or both")));
    }
    let bound = |name: &str| -> Result<Option<u16>> {
        range
            .get(name)
            .map(|value| {
                let at = format!("{path}/{name}");
                if python {
                    python_minor(value, &at).map(u16::from)
                } else {
                    ecma_year(value, &at)
                }
            })
            .transpose()
    };
    let (min, max) = (bound("min")?, bound("max")?);
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(invalid(format!(
            "{path}: min is after max, so the query never applies"
        )));
    }
    let version = if python {
        u16::from(python_options.map_or(DEFAULT_PYTHON_MINOR, |options| options.minor))
    } else {
        options.map_or(EsGlobals::get().latest, |options| options.ecma_version)
    };
    Ok(applies && min.is_none_or(|min| min <= version) && max.is_none_or(|max| version <= max))
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

/// An ast query's `file`: conditions on the file. Every language has `path`, a `regex` that must
/// match the path the host sends, a `notRegex` that must not, or both. TypeScript has
/// `declaration`, whether the path names a declaration file. Python has its module path, the
/// host's `module` or the stem fallback: `module`, `moduleName` and `package` hold text conditions
/// over the dotted names, the last one and the first one (in a package), and `inPackage` whether
/// the host sent the path.
pub(super) fn compile_file_condition(
    value: &Value,
    path: &str,
    language: &str,
    max_regex_bytes: usize,
) -> Result<FileCondition> {
    let path = format!("{path}/file");
    let python = language == "python";
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
        ],
        &path,
    )?;
    for key in ["module", "moduleName", "package", "inPackage"] {
        if !python && file.contains_key(key) {
            return Err(invalid(format!(
                "{path}/{key}: module paths are Python's; {language} has none"
            )));
        }
    }
    if python && file.contains_key("declaration") {
        return Err(invalid(format!(
            "{path}/declaration: declaration files are TypeScript's; {language} has none"
        )));
    }
    if file.is_empty() {
        return Err(invalid(if python {
            format!(
                "{path} must set `path`, `declaration`, `module`, `moduleName`, `package` or `inPackage`"
            )
        } else {
            format!("{path} must set `path` or `declaration`")
        }));
    }
    let text = |key: &str| -> Result<Option<TextCondition>> {
        let Some(value) = file.get(key) else {
            return Ok(None);
        };
        let at = format!("{path}/{key}");
        let condition = object(value, &at)?;
        reject_unknown(condition, &["regex", "notRegex"], &at)?;
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
    };
    let flag = |key: &str| match file.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(invalid(format!("{path}/{key} must be true or false"))),
    };
    Ok(FileCondition {
        path: text("path")?,
        declaration: flag("declaration")?,
        module: text("module")?,
        module_name: text("moduleName")?,
        package: text("package")?,
        in_package: flag("inPackage")?,
    })
}
