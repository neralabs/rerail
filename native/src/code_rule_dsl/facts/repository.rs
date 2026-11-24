//! `core.rerail`'s repository inputs (decision 0046): facts about the repository that are not source code, one reader
//! per kind of file, made once a run (not per batch) and bound beside the batches' relations.
//!
//! - `option(name, v)` and `option_item(name, i, v)`: the policy's `options` (`rerail.policy/v1`), as rrlab reads
//!   them: a list gives one `option` row per element and `option_item` rows with positions; booleans are `"true"` and
//!   `"false"`; nested objects are flattened with `/`. An ESLint-style array (`["all", {…}]`) gives `option`,
//!   `option2`, … for its strings, and merges its objects.
//! - `file(path)`: every file under the root, the host's ignored directories (`git_workspace::ignored_directories`:
//!   `.git`, `node_modules`, `target`, …) skipped.
//! - `package(dir, name)`, `package_field(dir, key, v)`: each `package.json`'s name and string-valued members, keys
//!   as paths from the root (`engines/node`); `package_for(path, dir)`: a document's nearest `package.json`;
//!   `package_export(name, subpath, target)`: a named `package.json`'s `exports`, one row per target string.
//! - `tsconfig(dir, base_url)`, `tsconfig_path(dir, pattern, target)`: each folder's `tsconfig.json` (else its
//!   `jsconfig.json`) as TypeScript reads it (comments, trailing commas, `extends` followed): its `baseUrl` from the
//!   root, and its `paths`, one row per target, relative to `dir` with `baseUrl` (else the folder of the
//!   configuration that sets `paths`) applied, so `@join(dir, target)` is the path the target names;
//!   `tsconfig_for(path, dir)`: a document's nearest such folder.
//!
//! `git_*` and `signed_list` are not read yet (open item O80). `input_error`'s `missing` rows are made where the
//! `.input` files are (`logic::plan::corpus::external_inputs`).

use super::vtab::Relation;
use crate::code_rule_dsl::json::strip_jsonc;
use crate::code_rule_dsl::logic::strings::functions::join;
use crate::content_store::{ContentStore, StoreResult};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// At most this many files are listed; a larger tree's `file` rows are cut there.
const MAX_FILES: usize = 200_000;
/// How many `extends` deep a configuration chain is followed; a cycle stops it before.
const MAX_EXTENDS: usize = 8;

/// Each `package.json` by its folder, and its value where it parses.
type Packages = BTreeMap<String, Option<Value>>;
/// Each folder holding a project configuration, and the file's name: `tsconfig.json`, else `jsconfig.json`.
type Configs = BTreeMap<String, String>;

/// The rows, as strings, before they are interned.
#[derive(Default, Debug, PartialEq)]
pub struct Rows {
    pub option: Vec<(String, String)>,
    pub option_item: Vec<(String, i64, String)>,
    pub(crate) file: Vec<String>,
    pub(crate) package: Vec<(String, String)>,
    pub(crate) package_field: Vec<(String, String, String)>,
    pub(crate) package_for: Vec<(String, String)>,
    pub(crate) package_export: Vec<(String, String, String)>,
    pub(crate) tsconfig: Vec<(String, String)>,
    pub(crate) tsconfig_for: Vec<(String, String)>,
    pub(crate) tsconfig_path: Vec<(String, String, String)>,
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The run's options over the defaults its programs declare (`.options {"name": {"default": …}, …}`): an option the
/// run does not set has its declared default, so a rule's option reads the same whether or not its caller passes
/// options. A run option that is not an object (none given) leaves only the defaults.
pub fn with_declared(settings: &[(String, String)], opts: &Value) -> Value {
    let mut merged = serde_json::Map::new();
    for (_, text) in settings.iter().filter(|(k, _)| k == "options") {
        let Ok(Value::Object(decl)) = serde_json::from_str::<Value>(text) else { continue };
        for (name, spec) in decl {
            if let Some(d) = spec.get("default") {
                merged.entry(name).or_insert_with(|| d.clone());
            }
        }
    }
    if merged.is_empty() {
        return opts.clone();
    }
    if let Value::Object(given) = opts {
        merged.extend(given.clone());
    }
    Value::Object(merged)
}

/// `option` and `option_item` of a policy's `options`.
pub fn options(opts: &Value, rows: &mut Rows) {
    let merged;
    let opts = match opts {
        Value::Array(list) => {
            let mut m = serde_json::Map::new();
            let mut k = 0;
            for x in list {
                match x {
                    Value::Object(o) => m.extend(o.clone()),
                    other => {
                        m.insert(if k == 0 { "option".to_owned() } else { format!("option{}", k + 1) }, other.clone());
                        k += 1;
                    }
                }
            }
            merged = Value::Object(m);
            &merged
        }
        other => other,
    };
    fn walk(prefix: &str, v: &Value, rows: &mut Rows) {
        match v {
            Value::Object(o) => {
                for (k, x) in o {
                    walk(&if prefix.is_empty() { k.clone() } else { format!("{prefix}/{k}") }, x, rows);
                }
            }
            Value::Array(list) => {
                for (i, x) in list.iter().enumerate() {
                    rows.option.push((prefix.to_owned(), scalar(x)));
                    rows.option_item.push((prefix.to_owned(), i as i64, scalar(x)));
                }
            }
            other => rows.option.push((prefix.to_owned(), scalar(other))),
        }
    }
    if !opts.is_null() {
        walk("", opts, rows);
    }
}

/// Every string member of `v`, with its path from the root (JSON pointer tokens: `~` as `~0`, `/` as `~1`).
fn strings(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
    let key = |k: &str| {
        let k = k.replace('~', "~0").replace('/', "~1");
        if prefix.is_empty() { k } else { format!("{prefix}/{k}") }
    };
    match v {
        Value::Object(o) => o.iter().for_each(|(k, x)| strings(&key(k), x, out)),
        Value::Array(list) => list.iter().enumerate().for_each(|(i, x)| strings(&key(&i.to_string()), x, out)),
        Value::String(s) => out.push((prefix.to_owned(), s.clone())),
        _ => {}
    }
}

/// `file`, the `package*` and the `tsconfig*` rows of the tree under `root`, for documents `docs` (paths relative to
/// it).
pub(crate) fn tree(root: &Path, docs: &[String], rows: &mut Rows) {
    let mut packages = Packages::new();
    let mut configs = Configs::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in sorted_entries(&dir) {
            tree_entry(root, &entry, &mut stack, &mut packages, &mut configs, rows);
        }
    }
    rows.file.sort();
    package_rows(&packages, rows);
    rows.package_for = nearest(docs, |d| packages.contains_key(d));
    tsconfig_rows(root, &configs, &packages, rows);
    rows.tsconfig_for = nearest(docs, |d| configs.contains_key(d));
}

/// A folder's entries in name order; none where it cannot be read.
fn sorted_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries
}

/// One entry of the tree: a folder the host's walk enters goes on `stack`; a file is a `file` row (up to
/// `MAX_FILES`), a `package.json` is read into `packages`, and a `tsconfig.json` or `jsconfig.json` is noted in
/// `configs`, by its folder.
fn tree_entry(root: &Path, entry: &std::fs::DirEntry, stack: &mut Vec<PathBuf>, packages: &mut Packages, configs: &mut Configs, rows: &mut Rows) {
    let name = entry.file_name().to_string_lossy().into_owned();
    let path = entry.path();
    let Ok(kind) = entry.file_type() else { return };
    if kind.is_dir() {
        stack.extend((!crate::git_workspace::ignored_directory(&name)).then_some(path));
        return;
    }
    if rows.file.len() >= MAX_FILES {
        return;
    }
    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    if name == "package.json" {
        packages.insert(parent(&rel).to_owned(), std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()));
    }
    // A folder's entries come in name order, so its `tsconfig.json` replaces its `jsconfig.json`.
    if name == "tsconfig.json" || name == "jsconfig.json" {
        configs.insert(parent(&rel).to_owned(), name);
    }
    rows.file.push(rel);
}

/// The folder of a path from the root; "" for the root's own files.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `package`, `package_field` and `package_export` of each `package.json` that parses to an object.
fn package_rows(packages: &Packages, rows: &mut Rows) {
    for (dir, json) in packages {
        let Some(json @ Value::Object(o)) = json else { continue };
        if let Some(Value::String(name)) = o.get("name") {
            rows.package.push((dir.clone(), name.clone()));
            let mut targets = Vec::new();
            exports(o.get("exports").unwrap_or(&Value::Null), &mut targets);
            rows.package_export.extend(targets.into_iter().map(|(s, t)| (name.clone(), s, t)));
        }
        let mut fields = Vec::new();
        strings("", json, &mut fields);
        rows.package_field.extend(fields.into_iter().map(|(k, v)| (dir.clone(), k, v)));
    }
}

/// The targets of a `package.json`'s `exports`, by subpath: an object whose keys start with "." maps subpaths, and
/// anything else is the targets of ".". A subpath's targets are its strings, through condition objects and fallback
/// arrays, each one row; `null` gives none.
fn exports(v: &Value, out: &mut Vec<(String, String)>) {
    fn targets(subpath: &str, v: &Value, out: &mut Vec<(String, String)>) {
        match v {
            Value::String(t) => out.push((subpath.to_owned(), t.clone())),
            Value::Array(list) => list.iter().for_each(|x| targets(subpath, x, out)),
            Value::Object(o) => o.values().for_each(|x| targets(subpath, x, out)),
            _ => {}
        }
    }
    match v {
        Value::Object(o) if o.keys().any(|k| k.starts_with('.')) => {
            o.iter().filter(|(k, _)| k.starts_with('.')).for_each(|(k, x)| targets(k, x, out));
        }
        other => targets(".", other, out),
    }
}

/// Each document's nearest folder at or above it that `holds` (`package_for`, `tsconfig_for`).
fn nearest(docs: &[String], holds: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in docs {
        let mut d = parent(path);
        loop {
            if holds(d) {
                out.push((path.clone(), d.to_owned()));
                break;
            }
            if d.is_empty() {
                break;
            }
            d = parent(d);
        }
    }
    out
}

/// A configuration's `baseUrl` and `paths`, as its `extends` chain leaves them.
#[derive(Default)]
struct Compiler {
    /// `baseUrl`, from the root.
    base_url: Option<String>,
    /// The folder of the configuration that sets `paths`, and each pattern's targets as written.
    paths: Option<(String, Vec<(String, Vec<String>)>)>,
}

impl Compiler {
    /// `other`'s options over these: a later base's over an earlier one's, a file's own over its bases'.
    fn apply(&mut self, other: Compiler) {
        if other.base_url.is_some() {
            self.base_url = other.base_url;
        }
        if other.paths.is_some() {
            self.paths = other.paths;
        }
    }
}

/// `tsconfig` and `tsconfig_path` of each folder's configuration. A `paths` target is relative to `baseUrl`, else to
/// the folder of the configuration that sets `paths` (TypeScript's `pathsBasePath`); its row is the same path from
/// the configuration's own folder.
fn tsconfig_rows(root: &Path, configs: &Configs, packages: &Packages, rows: &mut Rows) {
    for (dir, name) in configs {
        let compiler = compiler(root, &join(dir, name), packages, &mut Vec::new());
        if let Some(base_url) = &compiler.base_url {
            rows.tsconfig.push((dir.clone(), base_url.clone()));
        }
        let Some((set_in, paths)) = compiler.paths else { continue };
        let base = compiler.base_url.unwrap_or(set_in);
        for (pattern, targets) in paths {
            rows.tsconfig_path.extend(targets.iter().map(|t| (dir.clone(), pattern.clone(), relative(dir, &join(&base, t)))));
        }
    }
}

/// The configuration `file` (from the root) over its `extends` chain; `chain` holds the files that extend it, so a
/// cycle ends where it closes.
fn compiler(root: &Path, file: &str, packages: &Packages, chain: &mut Vec<String>) -> Compiler {
    let mut out = Compiler::default();
    if chain.len() > MAX_EXTENDS || chain.iter().any(|f| f == file) {
        return out;
    }
    let Some(Value::Object(config)) = std::fs::read_to_string(root.join(file)).ok().and_then(|t| serde_json::from_str(&strip_jsonc(&t)).ok()) else {
        return out;
    };
    let dir = parent(file);
    let bases = match config.get("extends") {
        Some(Value::String(base)) => vec![base.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    chain.push(file.to_owned());
    for base in bases.into_iter().filter_map(|b| extends_path(root, dir, b, packages)) {
        out.apply(compiler(root, &base, packages, chain));
    }
    chain.pop();
    let options = config.get("compilerOptions");
    let mut own = Compiler::default();
    if let Some(Value::String(base_url)) = options.and_then(|o| o.get("baseUrl")) {
        own.base_url = Some(join(dir, base_url));
    }
    if let Some(Value::Object(paths)) = options.and_then(|o| o.get("paths")) {
        let strings = |t: &Value| t.as_array().map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
        own.paths = Some((dir.to_owned(), paths.iter().map(|(p, t)| (p.clone(), strings(t))).collect()));
    }
    out.apply(own);
    out
}

/// The configuration file an `extends` entry of a configuration in `dir` names, from the root, as TypeScript finds
/// it: a relative path (with `.json` added where the path as written is no file), else a package's file: the entry's
/// path in the nearest `node_modules` at or above `dir` that holds the package, else in the workspace's package of
/// that name (`package`). A package named alone is the file its `package.json`'s `tsconfig` names, else its
/// `tsconfig.json`. None for a package found nowhere.
fn extends_path(root: &Path, dir: &str, base: &str, packages: &Packages) -> Option<String> {
    let json = |p: String| if p.ends_with(".json") || root.join(&p).is_file() { p } else { format!("{p}.json") };
    if base.starts_with('.') {
        return Some(json(join(dir, base)));
    }
    let scoped = usize::from(base.starts_with('@'));
    let mut parts = base.splitn(scoped + 2, '/');
    let name = parts.by_ref().take(scoped + 1).collect::<Vec<_>>().join("/");
    let mut d = dir;
    let installed = loop {
        let candidate = join(d, &format!("node_modules/{name}"));
        if root.join(&candidate).is_dir() {
            break Some(candidate);
        }
        if d.is_empty() {
            break None;
        }
        d = parent(d);
    };
    let workspace = || packages.iter().find(|(_, json)| json.as_ref().and_then(|j| j.get("name")).and_then(Value::as_str) == Some(name.as_str())).map(|(d, _)| d.clone());
    let package = installed.or_else(workspace)?;
    let rest = parts.next().map(str::to_owned).unwrap_or_else(|| {
        let manifest: Option<Value> = std::fs::read(root.join(&package).join("package.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
        manifest.as_ref().and_then(|m| m.get("tsconfig")).and_then(Value::as_str).unwrap_or("tsconfig.json").to_owned()
    });
    Some(json(join(&package, &rest)))
}

/// `path` from the folder `dir` (both from the root): `@join(dir, relative(dir, path))` is `path`.
fn relative(dir: &str, path: &str) -> String {
    let from: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let steps: Vec<&str> = std::iter::repeat_n("..", from.len() - common).chain(to[common..].iter().copied()).collect();
    if steps.is_empty() { ".".to_owned() } else { steps.join("/") }
}

/// The repository inputs a program reads (`reads`), interned in `store`, by relation.
pub fn relations(store: &ContentStore, root: &Path, docs: &[String], opts: &Value, reads: &dyn Fn(&str) -> bool) -> StoreResult<Vec<(&'static str, Relation)>> {
    let mut rows = Rows::default();
    if reads("option") || reads("option_item") {
        options(opts, &mut rows);
    }
    if ["file", "package", "package_field", "package_for", "package_export", "tsconfig", "tsconfig_for", "tsconfig_path"].iter().any(|r| reads(r)) {
        tree(root, docs, &mut rows);
    }
    let mut texts: Vec<&str> = Vec::new();
    rows.option.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.option_item.iter().for_each(|(a, _, b)| texts.extend([a.as_str(), b.as_str()]));
    texts.extend(rows.file.iter().map(String::as_str));
    rows.package.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.package_field.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    rows.package_for.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.package_export.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    rows.tsconfig.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.tsconfig_for.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.tsconfig_path.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    let atoms = store.intern_atoms(&texts)?;
    let mut next = atoms.iter().map(|&a| i64::from(a));
    let mut sym = || next.next().expect("an atom per string");
    let mut out = Vec::new();
    let mut make = |name: &'static str, columns: &[&str], rows: Vec<Vec<i64>>| {
        let mut r = Relation::new(columns);
        rows.iter().for_each(|row| r.push(row));
        out.push((name, r));
    };
    let option: Vec<Vec<i64>> = rows.option.iter().map(|_| vec![sym(), sym()]).collect();
    make("option", &["name", "v"], option);
    let item = rows.option_item.iter().map(|(_, i, _)| {
        let name = sym();
        vec![name, *i, sym()]
    });
    let item: Vec<Vec<i64>> = item.collect();
    make("option_item", &["name", "i", "v"], item);
    let file: Vec<Vec<i64>> = rows.file.iter().map(|_| vec![sym()]).collect();
    make("file", &["path"], file);
    let package: Vec<Vec<i64>> = rows.package.iter().map(|_| vec![sym(), sym()]).collect();
    make("package", &["dir", "name"], package);
    let field: Vec<Vec<i64>> = rows.package_field.iter().map(|_| vec![sym(), sym(), sym()]).collect();
    make("package_field", &["dir", "key", "v"], field);
    let pfor: Vec<Vec<i64>> = rows.package_for.iter().map(|_| vec![sym(), sym()]).collect();
    make("package_for", &["path", "dir"], pfor);
    let export: Vec<Vec<i64>> = rows.package_export.iter().map(|_| vec![sym(), sym(), sym()]).collect();
    make("package_export", &["name", "subpath", "target"], export);
    let tsconfig: Vec<Vec<i64>> = rows.tsconfig.iter().map(|_| vec![sym(), sym()]).collect();
    make("tsconfig", &["dir", "base_url"], tsconfig);
    let tfor: Vec<Vec<i64>> = rows.tsconfig_for.iter().map(|_| vec![sym(), sym()]).collect();
    make("tsconfig_for", &["path", "dir"], tfor);
    let tpath: Vec<Vec<i64>> = rows.tsconfig_path.iter().map(|_| vec![sym(), sym(), sym()]).collect();
    make("tsconfig_path", &["dir", "pattern", "target"], tpath);
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn options_flatten_as_rrlab_reads_them() {
        let mut rows = Rows::default();
        options(&serde_json::json!({"max": 3, "allow": ["a", "b"], "o": {"x": true}}), &mut rows);
        assert_eq!(rows.option, [("allow", "a"), ("allow", "b"), ("max", "3"), ("o/x", "true")].map(|(a, b)| (a.to_owned(), b.to_owned())));
        assert_eq!(rows.option_item, [("allow".to_owned(), 0, "a".to_owned()), ("allow".to_owned(), 1, "b".to_owned())]);
        let mut rows = Rows::default();
        options(&serde_json::json!(["always", {"ignoreCase": false}]), &mut rows);
        assert_eq!(rows.option, [("ignoreCase", "false"), ("option", "always")].map(|(a, b)| (a.to_owned(), b.to_owned())));
    }

    #[test]
    fn the_tree_gives_files_and_packages() {
        let root = std::env::temp_dir().join(format!("rerail-repo-{}-{:?}", std::process::id(), std::thread::current().id()));
        for (p, t) in [("web/package.json", r#"{"name": "@a/web", "engines": {"node": ">=18"}, "n": 1}"#), ("web/src/a.ts", ""), ("node_modules/x/b.js", ""), ("vendor/v.php", ""), ("c.py", "")] {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        let mut rows = Rows::default();
        tree(&root, &["web/src/a.ts".to_owned(), "c.py".to_owned()], &mut rows);
        assert_eq!(rows.file, ["c.py", "web/package.json", "web/src/a.ts"]);
        assert_eq!(rows.package, [("web".to_owned(), "@a/web".to_owned())]);
        assert!(rows.package_field.contains(&("web".to_owned(), "engines/node".to_owned(), ">=18".to_owned())));
        assert!(!rows.package_field.iter().any(|(_, k, _)| k == "n"));
        assert_eq!(rows.package_for, [("web/src/a.ts".to_owned(), "web".to_owned())]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `tsconfig*` as TypeScript reads a configuration: comments and trailing commas, `extends` (relative, a
    /// workspace package's file, a package named alone through its `tsconfig` field, a cycle), `baseUrl` from its
    /// configuration's folder, `paths` targets from `baseUrl` (else the folder that sets `paths`) as paths from the
    /// governing folder; a `jsconfig.json` where there is no `tsconfig.json`. `package_export` per target string.
    #[test]
    fn the_tree_gives_tsconfigs_and_exports() {
        let root = std::env::temp_dir().join(format!("rerail-repo-ts-{}-{:?}", std::process::id(), std::thread::current().id()));
        let files = [
            ("web/tsconfig.json", "{\n  // the app\n  \"extends\": \"./tsconfig.base\",\n  \"compilerOptions\": {\n    \"paths\": { \"@/*\": [\"./*\"], \"~lib\": [\"../shared/lib/index.ts\"], }, /* own */\n  },\n}\n"),
            ("web/tsconfig.base.json", r#"{"extends": "@acme/tsconfig/base.json", "compilerOptions": {"baseUrl": "src"}}"#),
            ("web/src/a.ts", ""),
            ("packages/tsconfig/package.json", r#"{"name": "@acme/tsconfig", "tsconfig": "base.json", "exports": {"./base.json": "./base.json", ".": {"types": "./index.d.ts", "default": ["./index.js", null]}}}"#),
            ("packages/tsconfig/base.json", r##"{"compilerOptions": {"paths": {"#x/*": ["x/*"]}}}"##),
            ("api/tsconfig.json", r#"{"extends": "@acme/tsconfig"}"#),
            ("docs/jsconfig.json", r##"{"compilerOptions": {"paths": {"#d/*": ["lib/*"]}}}"##),
            ("docs/x.js", ""),
            ("cyc/tsconfig.json", r#"{"extends": "./a.json", "compilerOptions": {"baseUrl": "."}}"#),
            ("cyc/a.json", r#"{"extends": "./tsconfig.json", "compilerOptions": {"baseUrl": "nowhere"}}"#),
            ("README.md", ""),
        ];
        for (p, t) in files {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        let mut rows = Rows::default();
        tree(&root, &["web/src/a.ts".to_owned(), "docs/x.js".to_owned(), "README.md".to_owned()], &mut rows);
        let _ = std::fs::remove_dir_all(&root);
        let two = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let three = |v: &[(&str, &str, &str)]| v.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect::<Vec<_>>();
        assert_eq!(rows.tsconfig, two(&[("cyc", "cyc"), ("web", "web/src")]));
        assert_eq!(rows.tsconfig_for, two(&[("web/src/a.ts", "web"), ("docs/x.js", "docs")]));
        assert_eq!(rows.tsconfig_path, three(&[
            ("api", "#x/*", "../packages/tsconfig/x/*"),
            ("docs", "#d/*", "lib/*"),
            ("web", "@/*", "src/*"),
            ("web", "~lib", "shared/lib/index.ts"),
        ]));
        let mut exports = rows.package_export.clone();
        exports.sort();
        assert_eq!(exports, three(&[("@acme/tsconfig", ".", "./index.d.ts"), ("@acme/tsconfig", ".", "./index.js"), ("@acme/tsconfig", "./base.json", "./base.json")]));
        assert_eq!(relative("web", "web"), ".");
    }
}
