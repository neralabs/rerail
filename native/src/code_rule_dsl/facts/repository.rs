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
//! - A language's project files (decision 0710): the relations of each `ProjectReader` a language registers at load
//!   (`install_project_reader`), over the same walk (`Tree`). The core names none of them: ECMAScript's adapter reads
//!   `tsconfig.json` and npm's publication (`import_target::project`).
//!
//! `git_*` and `signed_list` are not read yet (open item O80). `input_error`'s `missing` rows are made
//! where the `.input` files are (`logic::plan::corpus::external_inputs`).

use super::vtab::Relation;
use crate::content_store::{ContentStore, StoreResult};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// At most this many files are listed; a larger tree's `file` rows are cut there.
const MAX_FILES: usize = 200_000;

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
    /// Each registered reader's relations the program reads: (name, columns, rows).
    pub(crate) project: Vec<ProjectRelation>,
}

/// A relation a project reader makes: its name, its columns and its rows.
pub type ProjectRelation = (&'static str, &'static [&'static str], Vec<Vec<String>>);

/// What a project reader reads (decision 0710): the scanned root, every file listed (from the root, sorted), the
/// documents', and each `package.json` by its folder (`None` where it does not parse).
pub struct Tree<'a> {
    pub root: &'a Path,
    pub files: &'a [String],
    pub docs: &'a [String],
    pub packages: &'a BTreeMap<String, Option<Value>>,
}

/// A language's project files read as relations (decision 0710), registered by its adapter at load: the relations it
/// makes, each with its columns, and their rows over a `Tree`. Each relation is declared, with its placement, in the
/// language's library.
#[derive(Clone, Copy)]
pub struct ProjectReader {
    pub relations: &'static [(&'static str, &'static [&'static str])],
    pub rows: fn(&Tree<'_>) -> Vec<(&'static str, Vec<Vec<String>>)>,
}

static READERS: std::sync::RwLock<Vec<ProjectReader>> = std::sync::RwLock::new(Vec::new());

/// Registers a language's project reader (at load; again, it is kept once).
pub fn install_project_reader(reader: ProjectReader) {
    let mut readers = READERS.write().unwrap_or_else(|e| e.into_inner());
    if !readers.iter().any(|r| std::ptr::eq(r.relations, reader.relations)) {
        readers.push(reader);
    }
}

/// The columns of a relation a registered reader makes (`core::input_columns`' fallback).
pub fn project_columns(rel: &str) -> Option<&'static [&'static str]> {
    let readers = READERS.read().unwrap_or_else(|e| e.into_inner());
    readers.iter().flat_map(|r| r.relations.iter()).find(|(n, _)| *n == rel).map(|(_, c)| *c)
}

/// Whether a registered reader makes `rel`.
pub fn project_relation(rel: &str) -> bool {
    project_columns(rel).is_some()
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

/// `file`, `package`, `package_field` and `package_for` rows of the tree under `root`, for documents `docs` (paths
/// relative to it). A language's own project files are its registered reader's (`project_rows`, decision 0710).
pub(crate) fn tree(root: &Path, docs: &[String], rows: &mut Rows) -> BTreeMap<String, Option<Value>> {
    let mut packages: BTreeMap<String, Option<Value>> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in sorted_entries(&dir) {
            tree_entry(root, &entry, &mut stack, &mut packages, rows);
        }
    }
    rows.file.sort();
    package_rows(&packages, rows);
    package_for_rows(docs, &packages, rows);
    packages
}

/// The rows of each registered reader that makes a relation the program reads (`reads`), over the walked tree.
fn project_rows(tree: &Tree<'_>, reads: &dyn Fn(&str) -> bool, rows: &mut Rows) {
    let readers: Vec<ProjectReader> = READERS.read().unwrap_or_else(|e| e.into_inner()).clone();
    for reader in readers.iter().filter(|r| r.relations.iter().any(|(n, _)| reads(n))) {
        let mut made = (reader.rows)(tree);
        for (name, columns) in reader.relations {
            let got = made.iter_mut().find(|(n, _)| n == name).map(|(_, r)| std::mem::take(r)).unwrap_or_default();
            debug_assert!(got.iter().all(|row| row.len() == columns.len()), "{name}: a row of {} columns", columns.len());
            rows.project.push((name, columns, got));
        }
    }
}

/// A folder's entries in name order; none where it cannot be read.
fn sorted_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries
}

/// One entry of the tree: a folder the host's walk enters goes on `stack`; a file is a `file` row (up to
/// `MAX_FILES`), and a `package.json` is read into `packages`, by its folder.
fn tree_entry(root: &Path, entry: &std::fs::DirEntry, stack: &mut Vec<PathBuf>, packages: &mut BTreeMap<String, Option<Value>>, rows: &mut Rows) {
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
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
        packages.insert(dir, std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()));
    }
    rows.file.push(rel);
}

/// `package`, `package_field` and `package_export` of each `package.json` that parses to an object.
fn package_rows(packages: &BTreeMap<String, Option<Value>>, rows: &mut Rows) {
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
    match v {
        Value::Object(o) if o.keys().any(|k| k.starts_with('.')) => {
            o.iter().filter(|(k, _)| k.starts_with('.')).for_each(|(k, x)| export_targets(k, x, out));
        }
        other => export_targets(".", other, out),
    }
}

/// The target strings of one `exports` subpath, through condition objects and fallback arrays.
fn export_targets(subpath: &str, v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::String(t) => out.push((subpath.to_owned(), t.clone())),
        Value::Array(list) => list.iter().for_each(|x| export_targets(subpath, x, out)),
        Value::Object(o) => o.values().for_each(|x| export_targets(subpath, x, out)),
        _ => {}
    }
}

/// `package_for` of each document: the nearest folder above it that holds a `package.json`.
fn package_for_rows(docs: &[String], packages: &BTreeMap<String, Option<Value>>, rows: &mut Rows) {
    for path in docs {
        let mut d = path.rsplit_once('/').map_or("", |(d, _)| d);
        loop {
            if packages.contains_key(d) {
                rows.package_for.push((path.clone(), d.to_owned()));
                break;
            }
            if d.is_empty() {
                break;
            }
            d = d.rsplit_once('/').map_or("", |(p, _)| p);
        }
    }
}

/// The repository inputs a program reads (`reads`), interned in `store`, by relation.
pub fn relations(store: &ContentStore, root: &Path, docs: &[String], opts: &Value, reads: &dyn Fn(&str) -> bool) -> StoreResult<Vec<(&'static str, Relation)>> {
    let mut rows = Rows::default();
    if reads("option") || reads("option_item") {
        options(opts, &mut rows);
    }
    let projects = READERS.read().unwrap_or_else(|e| e.into_inner()).iter().flat_map(|r| r.relations.iter()).any(|(n, _)| reads(n));
    if projects || ["file", "package", "package_field", "package_for", "package_export"].iter().any(|r| reads(r)) {
        let packages = tree(root, docs, &mut rows);
        if projects {
            let files = std::mem::take(&mut rows.file);
            project_rows(&Tree { root, files: &files, docs, packages: &packages }, reads, &mut rows);
            rows.file = files;
        }
    }
    let mut texts: Vec<&str> = Vec::new();
    rows.option.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.option_item.iter().for_each(|(a, _, b)| texts.extend([a.as_str(), b.as_str()]));
    texts.extend(rows.file.iter().map(String::as_str));
    rows.package.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.package_field.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    rows.package_for.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.package_export.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    rows.project.iter().flat_map(|(_, _, r)| r.iter().flatten()).for_each(|x| texts.push(x.as_str()));
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
    for (name, columns, made) in &rows.project {
        let interned: Vec<Vec<i64>> = made.iter().map(|row| row.iter().map(|_| sym()).collect()).collect();
        make(name, columns, interned);
    }
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

    /// `package_export`: one row per target string of a named package's `exports`, through conditions and fallback
    /// arrays (`null` gives none); a string or a conditions object is the subpath "."; a package with no name has none.
    #[test]
    fn the_tree_gives_package_exports() {
        let root = std::env::temp_dir().join(format!("rerail-repo-exports-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        for (p, t) in [
            ("ui/package.json", r#"{"name": "@acme/ui", "exports": {"./button": "./src/button.ts", ".": {"types": "./index.d.ts", "default": ["./index.js", null]}}}"#),
            ("cfg/package.json", r#"{"name": "cfg", "exports": "./main.js"}"#),
            ("anon/package.json", r#"{"exports": "./x.js"}"#),
        ] {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        let mut rows = Rows::default();
        tree(&root, &[], &mut rows);
        let _ = std::fs::remove_dir_all(&root);
        let mut exports = rows.package_export.clone();
        exports.sort();
        let three = |v: &[(&str, &str, &str)]| v.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect::<Vec<_>>();
        assert_eq!(exports, three(&[("@acme/ui", ".", "./index.d.ts"), ("@acme/ui", ".", "./index.js"), ("@acme/ui", "./button", "./src/button.ts"), ("cfg", ".", "./main.js")]));
    }

    /// Decision 0710: a registered project reader's relations are made over the walked tree when the program reads
    /// one of them, with the columns the reader states, and not otherwise.
    #[test]
    fn a_registered_project_reader_makes_its_relations_over_the_tree() {
        const RELATIONS: &[(&str, &[&str])] = &[("test_reader_file", &["path", "folder"])];
        install_project_reader(ProjectReader {
            relations: RELATIONS,
            rows: |tree| vec![("test_reader_file", tree.files.iter().map(|f| vec![f.clone(), f.rsplit_once('/').map_or("", |(d, _)| d).to_owned()]).collect())],
        });
        assert_eq!(project_columns("test_reader_file"), Some(&["path", "folder"][..]));
        let root = std::env::temp_dir().join(format!("rerail-reader-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        for p in ["a/x.ts", "b.ts"] {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let mut rows = Rows::default();
        let packages = tree(&root, &[], &mut rows);
        let files = rows.file.clone();
        project_rows(&Tree { root: &root, files: &files, docs: &[], packages: &packages }, &|r| r == "file", &mut rows);
        assert!(rows.project.is_empty(), "not read, not made");
        project_rows(&Tree { root: &root, files: &files, docs: &[], packages: &packages }, &|r| r == "test_reader_file", &mut rows);
        let made: Vec<&ProjectRelation> = rows.project.iter().collect();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].2, [vec!["a/x.ts".to_owned(), "a".to_owned()], vec!["b.ts".to_owned(), String::new()]]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
