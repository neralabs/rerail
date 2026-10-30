//! Per-query language options for TypeScript and JavaScript, modelled on ESLint's
//! `languageOptions`: which names are globals (`ecmaVersion`, `sourceType`, `globals`) and what
//! top-level `this` is (`sourceType`).

use crate::code_rule_dsl::language::SourceType;

use super::{Directives, TYPE, VALUE};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Mutex, OnceLock, PoisonError};

/// ESLint's globals per `ecmaVersion` and for `commonjs`, generated offline from a pinned ESLint
/// (`scripts/generate/scope/es-globals.mjs`).
const ES_GLOBALS_JSON: &str = include_str!("data/es_globals.generated.json");

#[derive(Deserialize)]
struct EsGlobalsFile {
    latest: u16,
    commonjs: BTreeMap<String, GlobalSetting>,
    versions: BTreeMap<String, BTreeMap<String, GlobalSetting>>,
}

pub(crate) struct EsGlobals {
    /// ESLint's `LATEST_ECMA_VERSION`, what `"latest"` selects.
    pub(crate) latest: u16,
    commonjs: BTreeMap<String, GlobalSetting>,
    versions: BTreeMap<u16, BTreeMap<String, GlobalSetting>>,
}

impl EsGlobals {
    pub(crate) fn get() -> &'static Self {
        static TABLE: OnceLock<EsGlobals> = OnceLock::new();
        TABLE.get_or_init(|| {
            let file: EsGlobalsFile =
                serde_json::from_str(ES_GLOBALS_JSON).expect("es_globals.generated.json is valid");
            EsGlobals {
                latest: file.latest,
                commonjs: file.commonjs,
                versions: file
                    .versions
                    .into_iter()
                    .map(|(year, names)| (year.parse().expect("a year"), names))
                    .collect(),
            }
        })
    }

    /// The globals of a year-based `ecmaVersion` (`None` when the table has no such year).
    pub(crate) fn version(&self, year: u16) -> Option<&BTreeMap<String, GlobalSetting>> {
        self.versions.get(&year)
    }
}

/// typescript-eslint's lib globals per TypeScript `lib`, generated offline from its pinned scope
/// manager (`scripts/generate/scope/ts-lib-globals.mjs`).
const TS_LIB_GLOBALS_JSON: &str = include_str!("data/ts_lib_globals.generated.json");

#[derive(Deserialize)]
struct TsLibsFile {
    libs: BTreeMap<String, TsLib>,
}

#[derive(Deserialize)]
struct TsLib {
    /// The libs this one includes.
    libs: Vec<String>,
    /// Each variable's namespaces: `"type"`, `"value"` or `"typeValue"`.
    variables: BTreeMap<String, String>,
}

/// The lib table, by lib name.
fn ts_libs() -> &'static BTreeMap<String, TsLib> {
    static TABLE: OnceLock<BTreeMap<String, TsLib>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let file: TsLibsFile = serde_json::from_str(TS_LIB_GLOBALS_JSON)
            .expect("ts_lib_globals.generated.json is valid");
        file.libs
    })
}

/// Whether typescript-eslint knows the lib `name` (lower case).
pub(crate) fn is_ts_lib(name: &str) -> bool {
    ts_libs().contains_key(name)
}

/// The global variables typescript-eslint's scope manager defines for `libs` (lower case, known)
/// and every lib they include, with the namespaces (`VALUE`, `TYPE`) each is valid in, merged as
/// its `populateGlobalsFromLib` merges them.
pub(crate) fn ts_lib_globals<'a>(libs: impl IntoIterator<Item = &'a str>) -> HashMap<Box<str>, u8> {
    let table = ts_libs();
    let mut pending = libs.into_iter().collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut globals: HashMap<Box<str>, u8> = HashMap::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name) {
            continue;
        }
        let Some(lib) = table.get(name) else { continue };
        pending.extend(lib.libs.iter().map(String::as_str));
        for (variable, spaces) in &lib.variables {
            let spaces = match spaces.as_str() {
                "value" => VALUE,
                "type" => TYPE,
                _ => VALUE | TYPE,
            };
            *globals.entry(variable.as_str().into()).or_default() |= spaces;
        }
    }
    globals
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GlobalSetting {
    Readonly,
    Writable,
    Off,
}

/// Compiled `languageOptions`: the effective configured globals, as ESLint's
/// `applyLanguageOptions` merges them (ecmaVersion builtins, then CommonJS, then `globals`).
#[derive(Debug)]
pub(crate) struct LanguageOptions {
    /// The year-based `ecmaVersion` (3 and 5 stay as they are).
    pub(crate) ecma_version: u16,
    /// The query's `sourceType`; `None` leaves it to each file (`SourceType::of_file`), which the
    /// readers below take as `file`.
    pub(crate) source_type: Option<SourceType>,
    /// `moduleSystem: "package-type"`: without a `sourceType`, a file's module system is the one
    /// its extension and its package's `type` give it (`SourceType::by_package_type`), not its own
    /// (`SourceType::of_file`, Node's syntax detection where `type` is unset, a module without a
    /// manifest). The readers below take whichever the query reads as `file`
    /// (`file_source_type_for`).
    pub(crate) package_type_module_system: bool,
    /// The ecmaVersion builtins and the configured `globals`; CommonJS's are added by
    /// `declares` for a CommonJS file (no builtin shares a name with them).
    pub(crate) globals: HashMap<String, GlobalSetting>,
    /// `parserOptions.ecmaFeatures.globalReturn`: the top level is a function scope (eslint-scope's
    /// `nodejsScope`), so it is no global scope, and ESLint's no-eval reads its `this` by
    /// strictness.
    pub(crate) global_return: bool,
    /// `parserOptions.ecmaFeatures.impliedStrict`: all code is strict (from ES5).
    pub(crate) implied_strict: bool,
    /// typescript-eslint's `jsxPragma` (default `React`): the first JSX element or fragment whose
    /// scope sees a variable of this name reads it. `None` references nothing.
    pub(crate) jsx_pragma: Option<String>,
    /// typescript-eslint's `jsxFragmentName` (default none): the same for fragments.
    pub(crate) jsx_fragment_name: Option<String>,
    /// typescript-eslint's `lib` globals (default none), each with the namespaces it is valid in.
    pub(crate) lib: HashMap<Box<str>, u8>,
}

/// typescript-eslint's default `jsxPragma`.
pub(crate) const DEFAULT_JSX_PRAGMA: &str = "React";

impl LanguageOptions {
    pub(crate) fn new(
        ecma_version: u16,
        source_type: Option<SourceType>,
        configured: impl IntoIterator<Item = (String, GlobalSetting)>,
    ) -> Self {
        let table = EsGlobals::get();
        let mut globals = HashMap::new();
        for (name, setting) in table.version(ecma_version).into_iter().flatten() {
            globals.insert(name.clone(), *setting);
        }
        globals.extend(configured);
        Self {
            ecma_version,
            source_type,
            package_type_module_system: false,
            globals,
            global_return: false,
            implied_strict: false,
            jsx_pragma: Some(DEFAULT_JSX_PRAGMA.to_owned()),
            jsx_fragment_name: None,
            lib: HashMap::new(),
        }
    }

    /// The source type in a file whose own is `file`: the query's, else the file's.
    pub(crate) fn source_type(&self, file: SourceType) -> SourceType {
        self.source_type.unwrap_or(file)
    }

    /// Whether the top level is eslint-scope's global scope: a script's, unless `globalReturn`
    /// makes it a function.
    pub(crate) fn global_top_level(&self, file: SourceType) -> bool {
        self.source_type(file) == SourceType::Script && !self.global_return
    }

    /// Whether strict mode exists (eslint-scope's `isStrictModeSupported`: from ES5).
    pub(crate) fn strict_mode(&self) -> bool {
        self.ecma_version >= 5
    }

    /// Whether all code is strict: a module, or `impliedStrict` (from ES5).
    pub(crate) fn all_strict(&self, file: SourceType) -> bool {
        self.strict_mode() && (self.source_type(file) == SourceType::Module || self.implied_strict)
    }

    /// Whether blocks and switch bodies are scopes: eslint-scope nests them from ecmaVersion 6.
    pub(crate) fn block_scopes(&self) -> bool {
        self.ecma_version >= 2015
    }

    /// Whether `name` is a declared global for a reference that reads `spaces`: a lib variable
    /// valid in one of them (typescript-eslint resolves those by namespace, and neither `off`
    /// removes one), or a declared global of any namespace (`declares`).
    pub(crate) fn declares_in(
        &self,
        name: &str,
        spaces: u8,
        directives: Option<&Directives>,
        file: SourceType,
    ) -> bool {
        self.lib.get(name).is_some_and(|lib| lib & spaces != 0)
            || self.declares(name, directives, file)
    }

    /// Whether `name` is a declared global: `/* global */` comments (when honored) decide the
    /// names they mention, configuration the rest (ESLint's `addDeclaredGlobals`).
    pub(crate) fn declares(
        &self,
        name: &str,
        directives: Option<&Directives>,
        file: SourceType,
    ) -> bool {
        if let Some(directives) = directives {
            if directives.off.contains(name) {
                return false;
            }
            if directives.declared.contains(name) {
                return true;
            }
        }
        let commonjs = || {
            (self.source_type(file) == SourceType::CommonJs)
                .then(|| EsGlobals::get().commonjs.get(name))
                .flatten()
        };
        self.globals
            .get(name)
            .or_else(commonjs)
            .is_some_and(|setting| *setting != GlobalSetting::Off)
    }
}

/// Ruff's module-level builtins and magic globals per Python minor version, generated offline
/// from a pinned Ruff (`scripts/generate/scope/python_builtins.py`).
const PYTHON_BUILTINS_JSON: &str = include_str!("data/python_builtins.generated.json");

#[derive(Deserialize)]
struct PythonBuiltinsFile {
    versions: BTreeMap<String, BTreeSet<String>>,
}

/// The builtins of each `3.N` Ruff targets, keyed by `N`.
pub(crate) fn python_builtins() -> &'static BTreeMap<u8, BTreeSet<String>> {
    static TABLE: OnceLock<BTreeMap<u8, BTreeSet<String>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let file: PythonBuiltinsFile = serde_json::from_str(PYTHON_BUILTINS_JSON)
            .expect("python_builtins.generated.json is valid");
        file.versions
            .into_iter()
            .map(|(version, names)| {
                let minor = version
                    .strip_prefix("3.")
                    .and_then(|minor| minor.parse().ok());
                (minor.expect("a 3.N version"), names)
            })
            .collect()
    })
}

/// Ruff's default target version, what a Python `languageOptions` without `pythonVersion` selects.
pub(crate) const DEFAULT_PYTHON_MINOR: u8 = 10;

/// Ruff's `lint.typing-modules` (modules whose members Ruff reads as `typing`'s) and
/// `lint.pyflakes.extend-generics` (further generics, whose subscripts' slices are type
/// definitions), each as its names.
#[derive(Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct PythonTyping {
    pub(crate) modules: Vec<Vec<Box<str>>>,
    pub(crate) generics: Vec<Vec<Box<str>>>,
}

impl PythonTyping {
    /// The one shared copy of these settings: a table's `Reading` holds it and compares by value.
    /// Settings come from compiled code rules, so the distinct ones are few.
    pub(crate) fn intern(self) -> &'static Self {
        static INTERNED: OnceLock<Mutex<HashSet<&'static PythonTyping>>> = OnceLock::new();
        let mut interned = INTERNED
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = interned.get(&self) {
            return found;
        }
        let leaked: &'static Self = Box::leak(Box::new(self));
        interned.insert(leaked);
        leaked
    }

    /// Ruff's `match_typing_qualified_name` past `typing` itself: `member` of a typing module.
    pub(crate) fn module_member(&self, name: &[String], member: &str) -> bool {
        self.modules.iter().any(|module| {
            name.len() == module.len() + 1
                && name.last().is_some_and(|last| last == member)
                && name
                    .iter()
                    .zip(module)
                    .all(|(found, part)| found == part.as_ref())
        })
    }

    /// The last name of `name` when it starts with a typing module's names (Ruff's
    /// `match_annotated_subscript` reads such a member by its last name).
    pub(crate) fn module_tail<'n>(&self, name: &'n [String]) -> Option<&'n str> {
        self.modules
            .iter()
            .any(|module| {
                name.len() >= module.len()
                    && name
                        .iter()
                        .zip(module)
                        .all(|(found, part)| found == part.as_ref())
            })
            .then(|| name.last().map(String::as_str))
            .flatten()
    }

    /// Whether `name` is one of the extra generics.
    pub(crate) fn generic(&self, name: &[String]) -> bool {
        self.generics.iter().any(|generic| {
            generic.len() == name.len()
                && generic
                    .iter()
                    .zip(name)
                    .all(|(part, found)| part.as_ref() == found)
        })
    }

    /// Whether `last` is the last name of an extra generic: a subscript that may be one.
    pub(crate) fn generic_member(&self, last: &str) -> bool {
        self.generics
            .iter()
            .any(|generic| generic.last().is_some_and(|name| name.as_ref() == last))
    }
}

/// Compiled Python `languageOptions`: the target version (`3.minor`) and Ruff's typing settings.
#[derive(Debug)]
pub(crate) struct PythonOptions {
    pub(crate) minor: u8,
    /// `None` when neither setting lists a name.
    pub(crate) typing: Option<&'static PythonTyping>,
}

impl PythonOptions {
    pub(crate) fn new(minor: u8) -> Self {
        Self {
            minor,
            typing: None,
        }
    }

    /// Whether `name` is a builtin or magic global of the target version.
    pub(crate) fn declares(&self, name: &str) -> bool {
        python_builtins()
            .get(&self.minor)
            .is_some_and(|names| names.contains(name))
    }

    /// Python 3.14 evaluates annotations lazily (PEP 649), so Ruff reads them after the module
    /// as under `from __future__ import annotations`.
    pub(crate) fn defers_annotations(&self) -> bool {
        self.minor >= 14
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_eslints_pinned_data() {
        let table = EsGlobals::get();
        assert_eq!(table.latest, 2026);
        let years = table.versions.keys().copied().collect::<Vec<_>>();
        let mut expected = vec![3, 5];
        expected.extend(2015..=2026);
        assert_eq!(years, expected);
        let has = |year: u16, name: &str| table.version(year).is_some_and(|t| t.contains_key(name));
        assert!(!has(5, "Reflect") && has(2015, "Reflect"));
        assert!(!has(2019, "globalThis") && has(2020, "globalThis"));
        assert!(!has(3, "JSON") && has(5, "JSON"));
        assert!(has(2026, "Temporal") && !has(2025, "Temporal"));
        assert_eq!(
            table
                .commonjs
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["exports", "global", "module", "require"]
        );
        assert_eq!(table.commonjs["exports"], GlobalSetting::Writable);
    }

    #[test]
    fn the_lib_table_is_typescript_eslints_pinned_data() {
        assert_eq!(ts_libs().len(), 119);
        assert!(is_ts_lib("dom") && is_ts_lib("esnext.full") && is_ts_lib("lib"));
        assert!(!is_ts_lib("DOM") && !is_ts_lib("es2027"));
        let dom = ts_lib_globals(["dom"]);
        assert_eq!(dom.get("NodeListOf"), Some(&TYPE));
        assert_eq!(dom.get("NodeList"), Some(&(TYPE | VALUE)));
        // `dom` includes `es2015`, which includes `es5`. `Promise` is only a type in every lib
        // (ESLint's globals hold the value), `WeakMap` both in `es2015.collection`.
        assert_eq!(dom.get("Promise"), Some(&TYPE));
        assert_eq!(dom.get("WeakMap"), Some(&(TYPE | VALUE)));
        assert_eq!(dom.get("PropertyKey"), Some(&TYPE));
        assert!(!dom.contains_key("document"));
        // `lib` (TypeScript's default library) is `es5`, `dom`, `webworker.importscripts` and
        // `scripthost`.
        let default = ts_lib_globals(["lib"]);
        assert!(default.contains_key("ActiveXObject") && default.contains_key("NodeListOf"));
        assert_eq!(ts_lib_globals(["es5"]).get("Promise"), Some(&TYPE));
        assert!(ts_lib_globals(Vec::<&str>::new()).is_empty());
    }

    #[test]
    fn the_python_table_is_ruffs_pinned_data() {
        let table = python_builtins();
        assert_eq!(
            table.keys().copied().collect::<Vec<_>>(),
            (7..=15).collect::<Vec<_>>()
        );
        let at = |minor: u8, name: &str| PythonOptions::new(minor).declares(name);
        assert!(at(7, "print") && at(7, "__name__") && at(7, "__file__"));
        assert!(!at(9, "aiter") && at(10, "aiter"));
        assert!(!at(10, "ExceptionGroup") && at(11, "ExceptionGroup"));
        assert!(!at(13, "__annotate__") && at(14, "__annotate__"));
        assert!(at(14, "__cached__") && !at(15, "__cached__"));
        assert!(!at(15, "display") && !at(15, "reveal_type"));
        assert!(!PythonOptions::new(13).defers_annotations());
        assert!(PythonOptions::new(14).defers_annotations());
    }

    #[test]
    fn configuration_and_directives_merge_like_eslint() {
        let module = SourceType::Module;
        let options = LanguageOptions::new(
            5,
            Some(module),
            [
                ("String".to_owned(), GlobalSetting::Off),
                ("jQuery".to_owned(), GlobalSetting::Readonly),
            ],
        );
        assert!(options.declares("Array", None, module));
        assert!(!options.declares("Reflect", None, module));
        assert!(!options.declares("String", None, module));
        assert!(options.declares("jQuery", None, module));
        assert!(!options.declares("require", None, module));
        let mut directives = Directives::default();
        directives.declared.insert("Reflect".to_owned());
        directives.declared.insert("String".to_owned());
        directives.off.insert("jQuery".to_owned());
        assert!(options.declares("Reflect", Some(&directives), module));
        assert!(options.declares("String", Some(&directives), module));
        assert!(!options.declares("jQuery", Some(&directives), module));
        let commonjs = LanguageOptions::new(2026, Some(SourceType::CommonJs), []);
        assert!(
            commonjs.declares("require", None, module)
                && commonjs.declares("Temporal", None, module)
        );
        // Without a `sourceType` the file's decides; configured globals still win.
        let unpinned =
            LanguageOptions::new(2026, None, [("module".to_owned(), GlobalSetting::Off)]);
        assert!(!unpinned.declares("require", None, module));
        assert!(unpinned.declares("require", None, SourceType::CommonJs));
        assert!(!unpinned.declares("module", None, SourceType::CommonJs));
    }
}
