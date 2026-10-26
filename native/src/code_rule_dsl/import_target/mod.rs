//! Import and require targets, as eslint-plugin-n 18.4.1 reads them at default options
//! (`lib/util/import-target.js`, `visit-import.js`, `visit-require.js`, `get-try-extensions.js`,
//! `get-typescript-extension-map.js`, `get-tsconfig.js`, `get-resolve-paths.js`): a specifier's
//! module type and name, its import style, the file enhanced-resolve resolves it to (or why it
//! cannot), and what the plugin's rules read of it: the file extension `file-extension-in-import`
//! expects, whether npm publishes a target file (`check-publish.js`), and whether the module is
//! one the importing package lists (`check-publish.js`, `check-extraneous.js`).
//!
//! Resolution reads the files the evaluation holds and, below the workspace's root, the files on
//! disk (`fs::View`): a bare specifier resolves only where its package is installed there.

mod builtins;
mod entrypoints;
pub(crate) mod fs;
mod path;
mod resolver;
mod json;
mod tsconfig;

pub(crate) use builtins::is_builtin;
use fs::View;
use parking_lot::Mutex;
use resolver::{Alias, Options, Resolved};
use json::Json;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tsconfig::Tsconfig;

/// `ImportTarget`'s module types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ModuleType {
    Unknown,
    Relative,
    Absolute,
    Node,
    Npm,
    Http,
    Data,
}

impl ModuleType {
    pub(crate) const NAMES: [&'static str; 7] =
        ["unknown", "relative", "absolute", "node", "npm", "http", "data"];

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "unknown" => Self::Unknown,
            "relative" => Self::Relative,
            "absolute" => Self::Absolute,
            "node" => Self::Node,
            "npm" => Self::Npm,
            "http" => Self::Http,
            "data" => Self::Data,
            _ => return None,
        })
    }

    /// `ImportTarget#getModuleType`.
    fn of(name: &str) -> Self {
        let bytes = name.as_bytes();
        let relative = match bytes {
            [b'.', rest @ ..] => {
                let rest = rest.strip_prefix(b".").unwrap_or(rest);
                matches!(rest, [] | [b'/' | b'\\', ..])
            }
            _ => false,
        };
        if relative {
            Self::Relative
        } else if matches!(bytes, [b'/' | b'\\', ..]) {
            Self::Absolute
        } else if is_builtin(name) {
            Self::Node
        } else if name.starts_with("data:") {
            Self::Data
        } else if name.starts_with("http://") || name.starts_with("https://") {
            Self::Http
        } else if npm_name_prefix(name) {
            Self::Npm
        } else {
            Self::Unknown
        }
    }
}

/// Whether `name` starts as `/^(@[\w~-][\w.~-]*\/)?[\w~-][\w.~-]*/` matches.
fn npm_name_prefix(name: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '~' || c == '-';
    let mut rest = name;
    if let Some(scoped) = rest.strip_prefix('@')
        && scoped.starts_with(word)
    {
        let end = scoped
            .find(|c: char| !(word(c) || c == '.'))
            .unwrap_or(scoped.len());
        if scoped[end..].starts_with('/') {
            rest = &scoped[end + 1..];
            if rest.starts_with(word) {
                return true;
            }
            rest = name;
        }
    }
    rest.starts_with(word)
}

/// `ImportTarget`'s module styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Style {
    Import,
    Require,
    Type,
}

/// What eslint-plugin-n's `ImportTarget` knows of one target.
#[derive(Debug)]
pub(crate) struct Target {
    pub(crate) module_type: ModuleType,
    pub(crate) module_name: Option<String>,
    pub(crate) style: Style,
    /// `filePath`: the resolved path (absolute), a relative or absolute target's own path where
    /// it does not resolve, the specifier of a data or http import; `None` otherwise.
    pub(crate) file_path: Option<String>,
    /// `resolveError`: why resolution failed, where it did.
    pub(crate) error: Option<String>,
    /// The importing file's absolute path.
    importer: String,
    /// The specifier up to its first `!`.
    name: String,
}

/// `stripImportPathParams`: the specifier up to its first `!`.
pub(crate) fn strip_params(specifier: &str) -> &str {
    specifier.split('!').next().unwrap_or(specifier)
}

/// `isTypescript`: the importing file's extension.
fn is_typescript(importer: &str) -> bool {
    matches!(path::extname(importer), ".ts" | ".tsx" | ".cts" | ".mts")
}

/// A TypeScript extension map (`get-typescript-extension-map.js`): the JavaScript extension a
/// TypeScript one compiles to, and the TypeScript ones a JavaScript one stands for.
struct ExtensionMap {
    forward: &'static [(&'static str, &'static str)],
    backward: &'static [(&'static str, &'static [&'static str])],
}

const DEFAULT_MAP: ExtensionMap = ExtensionMap {
    forward: &[("", ".js"), (".ts", ".js"), (".cts", ".cjs"), (".mts", ".mjs"), (".tsx", ".js")],
    backward: &[(".js", &[".ts", ".tsx"]), (".cjs", &[".cts"]), (".mjs", &[".mts"])],
};
const PRESERVE_MAP: ExtensionMap = ExtensionMap {
    forward: &[("", ".js"), (".ts", ".js"), (".cts", ".cjs"), (".mts", ".mjs"), (".tsx", ".jsx")],
    backward: &[(".js", &[".ts"]), (".cjs", &[".cts"]), (".mjs", &[".mts"]), (".jsx", &[".tsx"])],
};
const EXPLICIT_MAP: ExtensionMap = ExtensionMap {
    forward: &[],
    backward: &[],
};

/// `getTypescriptExtensionMap` at default options: the nearest tsconfig.json's (explicit with
/// `allowImportingTsExtensions`, else by its `jsx`), else `preserve`.
fn extension_map(tsconfig: Option<&Tsconfig>) -> &'static ExtensionMap {
    let Some(tsconfig) = tsconfig else {
        return &PRESERVE_MAP;
    };
    if tsconfig
        .option("allowImportingTsExtensions")
        .is_some_and(Json::truthy)
    {
        return &EXPLICIT_MAP;
    }
    match tsconfig.jsx().as_deref() {
        Some("react" | "react-jsx" | "react-jsxdev" | "react-native") => &DEFAULT_MAP,
        _ => &PRESERVE_MAP,
    }
}

const JS_EXTENSIONS: &[&str] = &[".js", ".json", ".node", ".mjs", ".cjs"];
const TS_EXTENSIONS: &[&str] = &[".js", ".ts", ".mjs", ".mts", ".cjs", ".cts", ".json", ".node"];

/// `getTryExtensions` at default options.
fn try_extensions(importer: &str, tsconfig: Option<&Tsconfig>) -> &'static [&'static str] {
    if is_typescript(importer)
        && tsconfig.is_some_and(|tsconfig| tsconfig.flag("allowImportingTsExtensions"))
    {
        TS_EXTENSIONS
    } else {
        JS_EXTENSIONS
    }
}

/// `removeTrailWildcard`: a `paths` key or target without trailing `/`, `\` and `*`.
fn remove_trail_wildcard(text: &str) -> &str {
    text.trim_end_matches(['/', '\\', '*'])
}

/// `getTSConfigAliases`: each `compilerOptions.paths` key, wildcard trimmed, to its targets,
/// resolved against the configuration's folder.
fn tsconfig_aliases(tsconfig: Option<&Tsconfig>) -> Vec<Alias> {
    let Some(tsconfig) = tsconfig else {
        return Vec::new();
    };
    let Some(paths) = tsconfig.option("paths").and_then(Json::entries) else {
        return Vec::new();
    };
    let folder = path::dirname(&tsconfig.path);
    paths
        .iter()
        .map(|(name, targets)| Alias {
            name: remove_trail_wildcard(name).to_owned(),
            targets: Some(
                targets
                    .as_array()
                    .map(|targets| {
                        targets
                            .iter()
                            .filter_map(Json::as_str)
                            .map(|target| path::resolve(folder, remove_trail_wildcard(target)))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
        })
        .collect()
}

/// The `false` fallback of the plugin's built-in table.
static FALLBACK: std::sync::LazyLock<Vec<Alias>> = std::sync::LazyLock::new(|| {
    builtins::PLUGIN_BUILTINS
        .iter()
        .map(|name| Alias {
            name: (*name).to_owned(),
            targets: None,
        })
        .collect()
});

/// The per-file facts the targets of one importing file share.
struct Importer {
    tsconfig: Option<Tsconfig>,
    typescript: bool,
}

/// What one view reads once: each importing file's configuration, each package folder's
/// manifest, and each target.
#[derive(Default)]
pub(crate) struct Memo {
    importers: Mutex<HashMap<String, Arc<Importer>>>,
    packages: Mutex<HashMap<String, Option<Arc<Package>>>>,
    targets: Mutex<HashMap<(String, String, Style), Arc<Target>>>,
}

/// A package.json as eslint-plugin-n's `getPackageJson` reads it, with what npm publishes of
/// its folder.
struct Package {
    /// The folder, absolute.
    folder: String,
    data: Arc<Json>,
    /// The manifest with its folder's ignore files (`Manifest::publishes`), when the folder lies
    /// at or below the root.
    manifest: Option<crate::code_rule_dsl::package_manifest::Manifest>,
}

impl Package {
    fn names(&self, field: &str) -> impl Iterator<Item = &str> {
        self.data
            .get(field)
            .and_then(Json::entries)
            .into_iter()
            .flat_map(|names| names.iter().map(|(name, _)| name.as_str()))
    }

    fn lists(&self, field: &str, name: &str) -> bool {
        self.names(field).any(|listed| listed == name)
    }
}

impl View {
    fn importer(&self, importer: &str) -> Arc<Importer> {
        if let Some(found) = self.memo.importers.lock().get(importer) {
            return Arc::clone(found);
        }
        let typescript = is_typescript(importer);
        let found = Arc::new(Importer {
            tsconfig: Tsconfig::of_file(self, importer),
            typescript,
        });
        self.memo
            .importers
            .lock()
            .insert(importer.to_owned(), Arc::clone(&found));
        found
    }

    /// The package.json in `folder` (absolute), where it parses to an object.
    fn package_in(&self, folder: &str) -> Option<Arc<Package>> {
        if let Some(found) = self.memo.packages.lock().get(folder) {
            return found.clone();
        }
        let found = resolver::package_json(self, folder).map(|data| {
            let manifest = self.relative(folder).map(|relative| {
                let file = |name: &str| self.text(&path::join(folder, name)).map(|text| (*text).clone());
                let bytes = self
                    .text(&path::join(folder, "package.json"))
                    .map(|text| text.as_bytes().to_vec())
                    .unwrap_or_default();
                crate::code_rule_dsl::package_manifest::Manifest::parse(relative.to_owned(), &bytes)
                    .with_ignore_files(file(".npmignore"), file(".gitignore"))
            });
            Arc::new(Package {
                folder: folder.to_owned(),
                data,
                manifest,
            })
        });
        self.memo
            .packages
            .lock()
            .insert(folder.to_owned(), found.clone());
        found
    }

    /// `getPackageJson`: the nearest package.json at or above the folder of `file` that parses to
    /// an object, never above the root.
    fn package_of(&self, file: &str) -> Option<Arc<Package>> {
        let mut folder = path::dirname(file).to_owned();
        loop {
            if let Some(package) = self.package_in(&folder) {
                return Some(package);
            }
            if folder == self.base() || folder == "/" {
                return None;
            }
            folder = path::dirname(&folder).to_owned();
        }
    }
}

/// `ImportTarget#getModuleName`.
fn module_name(name: &str, module_type: ModuleType) -> Option<String> {
    let first = |text: &str, count: usize| text.split('/').take(count).collect::<Vec<_>>().join("/");
    match module_type {
        ModuleType::Npm => Some(if name.starts_with('@') {
            first(name, 2)
        } else {
            first(name, 1)
        }),
        ModuleType::Node => Some(first(name.strip_prefix("node:").unwrap_or(name), 1)),
        _ => None,
    }
}

/// The target `specifier` (its text, `!` parameters and all) names from the file at `importer`
/// (workspace-relative), in `style`.
pub(crate) fn target(view: &View, importer: &str, specifier: &str, style: Style) -> Arc<Target> {
    let importer = view.absolute(importer);
    let name = strip_params(specifier).to_owned();
    let key = (importer, name, style);
    if let Some(found) = view.memo.targets.lock().get(&key) {
        return Arc::clone(found);
    }
    let (importer, name, _) = &key;
    let module_type = ModuleType::of(name);
    let found = Arc::new(resolve_target(view, importer, name, module_type, style));
    view.memo.targets.lock().insert(key, Arc::clone(&found));
    found
}

/// `ImportTarget#getFilePath`.
fn resolve_target(
    view: &View,
    importer: &str,
    name: &str,
    module_type: ModuleType,
    style: Style,
) -> Target {
    let target = |file_path, error| Target {
        module_type,
        module_name: module_name(name, module_type),
        style,
        file_path,
        error,
        importer: importer.to_owned(),
        name: name.to_owned(),
    };
    if matches!(module_type, ModuleType::Data | ModuleType::Http) && style == Style::Import {
        return target(Some(name.to_owned()), None);
    }
    let file = view.importer(importer);
    let mut conditions = vec!["node", "require"];
    match style {
        Style::Import => conditions.push("import"),
        Style::Type => conditions.extend(["import", "types"]),
        Style::Require => {}
    }
    let mains = style == Style::Require || matches!(module_type, ModuleType::Npm | ModuleType::Node);
    let mut options = Options {
        conditions,
        main_fields: if mains { vec!["main"] } else { Vec::new() },
        main_files: if mains { vec!["index"] } else { Vec::new() },
        extensions: try_extensions(importer, file.tsconfig.as_ref())
            .iter()
            .map(|extension| (*extension).to_owned())
            .collect(),
        fallback: FALLBACK.clone(),
        ..Options::default()
    };
    if file.typescript {
        options.alias = tsconfig_aliases(file.tsconfig.as_ref());
        options.extension_alias = extension_map(file.tsconfig.as_ref())
            .backward
            .iter()
            .map(|(extension, aliases)| {
                (
                    (*extension).to_owned(),
                    aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                )
            })
            .collect();
    }
    let base = path::dirname(importer);
    let mut error = None;
    match resolver::resolve(view, &options, base, name) {
        Ok(Resolved::Path(resolved)) => return target(Some(resolved), None),
        Ok(Resolved::Ignored) => {}
        Err(message) => error = Some(message),
    }
    let fallback = matches!(module_type, ModuleType::Relative | ModuleType::Absolute)
        .then(|| path::resolve(base, name));
    target(fallback, error)
}

impl Target {
    /// `file-extension-in-import`'s check at its default style (`always`): whether the specifier
    /// ends with the extension the rule expects for the file it names. `None` for a target that
    /// is not a relative or absolute path.
    pub(crate) fn extension_spelled(&self, view: &View) -> Option<bool> {
        if !matches!(self.module_type, ModuleType::Relative | ModuleType::Absolute) {
            return None;
        }
        let file_path = self.file_path.as_deref()?;
        let importer = view.importer(&self.importer);
        let map = extension_map(importer.tsconfig.as_ref());
        let try_extensions = try_extensions(&self.importer, importer.tsconfig.as_ref());
        let current = path::extname(&self.name);
        let mut actual = path::extname(file_path).to_owned();
        let mut actual_path = file_path.to_owned();
        let mut directory_import = false;
        if view.is_directory(file_path) {
            if let Some(index) = index_extension(view, file_path, try_extensions) {
                directory_import = true;
                actual_path = path::join(file_path, &format!("index{index}"));
                actual = index;
            }
        } else if !view.is_file(file_path) {
            let basename = path::basename(&self.name);
            let existing = existing_extensions(view, file_path, Some(basename));
            let found = try_extensions
                .iter()
                .find(|extension| existing.iter().any(|existing| existing == *extension))
                .map(|extension| (*extension).to_owned())
                .or_else(|| existing.first().cloned());
            if let Some(found) = found {
                actual_path = path::join(path::dirname(file_path), &format!("{basename}{found}"));
                actual = found;
            }
        }
        let forward = |file: &str, fallback: &str| -> String {
            let extension = path::extname(file);
            if importer.typescript
                && let Some((_, javascript)) =
                    map.forward.iter().find(|(typescript, _)| *typescript == extension)
            {
                return (*javascript).to_owned();
            }
            fallback.to_owned()
        };
        let mut expected = forward(&actual_path, &actual);
        if current.is_empty()
            && actual.is_empty()
            && !directory_import
            && let Some(index) = index_extension(view, file_path, try_extensions)
        {
            actual_path = path::join(file_path, &format!("index{index}"));
            expected = forward(&actual_path, &index);
        }
        Some(current == expected)
    }

    /// `check-publish.js`'s `isPrivateFile`, negated: for a target with no module name, whether
    /// npm publishes the file it names, as the importing file's package (its `files`, `main` and
    /// ignore files) decides: a path outside that package is not published. A target naming no
    /// file is read at the root (upstream's working directory). `None` for a target with a module
    /// name, an importing file without a package.json, or a target the package's own folder.
    pub(crate) fn published(&self, view: &View) -> Option<bool> {
        if self.module_name.is_some() {
            return None;
        }
        let package = view.package_of(&self.importer)?;
        let file = self.file_path.as_deref().unwrap_or(view.base());
        let relative = path::relative(&package.folder, file);
        if relative.is_empty() {
            return None;
        }
        if relative == ".." || relative.starts_with("../") {
            return Some(false);
        }
        let manifest = package.manifest.as_ref()?;
        let workspace = view.relative(file)?;
        manifest.publishes(workspace)
    }

    /// `check-publish.js`'s `isDevPackage` (no `allowModules`): the module is in the importing
    /// file's package's `devDependencies` and not in its `dependencies`, `peerDependencies` or
    /// `optionalDependencies`. `None` for a target with no module name, or an importing file
    /// without a package.json.
    pub(crate) fn dev_dependency(&self, view: &View) -> Option<bool> {
        let name = self.module_name.as_deref()?;
        let package = view.package_of(&self.importer)?;
        Some(
            package.lists("devDependencies", name)
                && !["dependencies", "peerDependencies", "optionalDependencies"]
                    .iter()
                    .any(|field| package.lists(field, name)),
        )
    }

    /// `check-extraneous.js`'s dependency set: the module is the importing file's package's own
    /// name or one it lists in any dependency field, or one the workspace root package that
    /// claims it lists (`workspaces`, or `pnpm-workspace.yaml`); a type import also counts its
    /// `@types` package. `None` for a target with no module name, or an importing file without a
    /// package.json.
    pub(crate) fn declared(&self, view: &View) -> Option<bool> {
        let name = self.module_name.as_deref()?;
        let package = view.package_of(&self.importer)?;
        let mut names = BTreeSet::new();
        collect_package_names(&package, &mut names);
        if let Some(root) = workspace_root(view, &package) {
            collect_package_names(&root, &mut names);
        }
        Some(
            names.contains(name)
                || (self.style == Style::Type && names.contains(&types_package(name))),
        )
    }

    /// `ImportTarget#hasTSAlias`: in a TypeScript file, the specifier starts with one of its
    /// tsconfig's `paths` keys, wildcard trimmed.
    pub(crate) fn aliased(&self, view: &View) -> bool {
        let importer = view.importer(&self.importer);
        importer.typescript
            && tsconfig_aliases(importer.tsconfig.as_ref())
                .iter()
                .any(|alias| self.name.starts_with(alias.name.as_str()))
    }
}

/// `getExistingExtensions`: the extensions of the files in the folder of `file` whose name
/// without extension is `basename` (else the name of `file` without its extension).
fn existing_extensions(view: &View, file: &str, basename: Option<&str>) -> Vec<String> {
    let basename = basename.unwrap_or_else(|| path::stem(file));
    view.names(path::dirname(file))
        .iter()
        .filter(|name| path::stem(name) == basename)
        .map(|name| path::extname(name).to_owned())
        .collect()
}

/// `getIndexExtension`: the extension of the folder's `index` file, preferring the try
/// extensions' order.
fn index_extension(view: &View, folder: &str, try_extensions: &[&str]) -> Option<String> {
    if !view.is_directory(folder) {
        return None;
    }
    let existing = existing_extensions(view, &path::join(folder, "index"), None);
    try_extensions
        .iter()
        .find(|extension| existing.iter().any(|existing| existing == *extension))
        .map(|extension| (*extension).to_owned())
        .or_else(|| existing.first().cloned())
}

/// `getPackageNames`.
fn collect_package_names(package: &Package, names: &mut BTreeSet<String>) {
    if let Some(name) = package.data.get("name").and_then(Json::as_str) {
        names.insert(name.to_owned());
    }
    for field in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        names.extend(package.names(field).map(str::to_owned));
    }
}

/// `getTypesPackageName`.
fn types_package(name: &str) -> String {
    match name.strip_prefix('@') {
        Some(scoped) => {
            let mut parts = scoped.split('/');
            let scope = parts.next().unwrap_or("");
            let name = parts.next().map_or("undefined", |name| name);
            format!("@types/{scope}__{name}")
        }
        None => format!("@types/{name}"),
    }
}

/// `getWorkspacePackageJsons`: the nearest package.json above `package`'s folder whose
/// `workspaces` (or pnpm-workspace.yaml `packages`) claim it.
fn workspace_root(view: &View, package: &Package) -> Option<Arc<Package>> {
    let mut folder = path::dirname(&package.folder).to_owned();
    loop {
        if let Some(candidate) = view
            .package_of(&path::join(&folder, "__placeholder__.js"))
            .filter(|candidate| candidate.folder == folder)
            && claims(view, &candidate, &package.folder)
        {
            return Some(candidate);
        }
        if folder == view.base() || folder == "/" {
            return None;
        }
        folder = path::dirname(&folder).to_owned();
    }
}

/// `isWorkspacePackage`: whether `root`'s workspace patterns match `folder` (relative to it).
fn claims(view: &View, root: &Package, folder: &str) -> bool {
    let relative = path::relative(&root.folder, folder);
    if relative.is_empty() || relative == ".." || relative.starts_with("../") {
        return false;
    }
    let mut patterns = Vec::new();
    match root.data.get("workspaces") {
        Some(Json::Array(items)) => patterns.extend(items.iter().map(Json::to_js_string)),
        Some(workspaces @ Json::Object(_)) => {
            if let Some(Json::Array(items)) = workspaces.get("packages") {
                patterns.extend(items.iter().map(Json::to_js_string));
            }
        }
        _ => {}
    }
    patterns.extend(pnpm_patterns(view, &root.folder));
    let mut matched = false;
    for raw in &patterns {
        let (negated, pattern) = match raw.strip_prefix('!') {
            Some(pattern) => (true, pattern),
            None => (false, raw.as_str()),
        };
        let pattern = pattern.replace('\\', "/");
        let pattern = pattern.strip_prefix("./").unwrap_or(&pattern);
        let pattern = pattern.trim_end_matches('/');
        if pattern.is_empty() {
            continue;
        }
        let Some(matcher) = crate::code_rule_dsl::npm_ignore::workspace_matcher(pattern) else {
            continue;
        };
        if matcher.is_match(&relative).unwrap_or(false) {
            if negated {
                return false;
            }
            matched = true;
        }
    }
    matched
}

/// `getPnpmWorkspacePatterns`: the `packages` of the folder's pnpm-workspace.yaml.
fn pnpm_patterns(view: &View, folder: &str) -> Vec<String> {
    let Some(text) = view.text(&path::join(folder, "pnpm-workspace.yaml")) else {
        return Vec::new();
    };
    let Ok(document) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text) else {
        return Vec::new();
    };
    match document.get("packages") {
        Some(serde_yaml_ng::Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| match item {
                serde_yaml_ng::Value::String(text) => Some(text.clone()),
                serde_yaml_ng::Value::Number(number) => Some(number.to_string()),
                serde_yaml_ng::Value::Bool(flag) => Some(flag.to_string()),
                serde_yaml_ng::Value::Null => Some("null".to_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

thread_local! {
    /// The view of the evaluation running on this thread (`ViewSlot`).
    static VIEW: RefCell<Option<Arc<View>>> = const { RefCell::new(None) };
}

/// Installs a view for the evaluations run while it is held, unless one is installed already
/// (a snapshot's evaluation inside a workspace's reads the workspace's).
pub(crate) struct ViewSlot {
    previous: Option<Option<Arc<View>>>,
}

impl ViewSlot {
    pub(crate) fn install(view: Option<Arc<View>>) -> Self {
        let Some(view) = view else {
            return Self { previous: None };
        };
        Self {
            previous: Some(VIEW.replace(Some(view))),
        }
    }

    /// Installs `make()`'s view where none is installed.
    pub(crate) fn install_unless_present(make: impl FnOnce() -> Arc<View>) -> Self {
        if VIEW.with_borrow(Option::is_some) {
            return Self { previous: None };
        }
        Self::install(Some(make()))
    }
}

impl Drop for ViewSlot {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            VIEW.set(previous);
        }
    }
}

/// The view installed on this thread, if any.
pub(crate) fn current_view() -> Option<Arc<View>> {
    VIEW.with_borrow(Clone::clone)
}

#[cfg(test)]
mod tests;
