//! `core.rr`'s repository inputs (decision 0046): facts about the repository that are not source code, one reader
//! per kind of file, made once a run (not per batch) and bound beside the batches' relations.
//!
//! - `option(name, v)` and `option_item(name, i, v)`: the policy's `options` (`rerail.policy/v1`), as rrlab reads
//!   them: a list gives one `option` row per element and `option_item` rows with positions; booleans are `"true"` and
//!   `"false"`; nested objects are flattened with `/`. An ESLint-style array (`["all", {…}]`) gives `option`,
//!   `option2`, … for its strings, and merges its objects.
//! - `file(path)`: every file under the root, `.git`, `node_modules`, `__pycache__`, `.venv`, `venv`, `target`, `dist`
//!   and `build` skipped.
//! - `package(dir, name)`, `package_field(dir, key, v)`: each `package.json`'s name and string-valued members, keys
//!   as paths from the root (`engines/node`); `package_for(path, dir)`: a document's nearest `package.json`.
//!
//! `tsconfig*`, `git_*`, `signed_list` and `input_error` are not read yet (open item O80).

use super::vtab::Relation;
use crate::content_store::{ContentStore, StoreResult};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const SKIP: [&str; 8] = [".git", "node_modules", "__pycache__", ".venv", "venv", "target", "dist", "build"];

/// At most this many files are listed; a larger tree's `file` rows are cut there.
const MAX_FILES: usize = 200_000;

/// The rows, as strings, before they are interned.
#[derive(Default, Debug, PartialEq)]
pub(crate) struct Rows {
    pub(crate) option: Vec<(String, String)>,
    pub(crate) option_item: Vec<(String, i64, String)>,
    pub(crate) file: Vec<String>,
    pub(crate) package: Vec<(String, String)>,
    pub(crate) package_field: Vec<(String, String, String)>,
    pub(crate) package_for: Vec<(String, String)>,
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `option` and `option_item` of a policy's `options`.
pub(crate) fn options(opts: &Value, rows: &mut Rows) {
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

/// `file`, `package`, `package_field` and `package_for` of the tree under `root`, for documents `docs` (paths
/// relative to it).
pub(crate) fn tree(root: &Path, docs: &[String], rows: &mut Rows) {
    let mut packages: BTreeMap<String, Option<Value>> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            if rows.file.len() >= MAX_FILES {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if name == "package.json" {
                let dir = rel.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
                packages.insert(dir, std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()));
            }
            rows.file.push(rel);
        }
    }
    rows.file.sort();
    for (dir, json) in &packages {
        let Some(json @ Value::Object(o)) = json else { continue };
        if let Some(Value::String(name)) = o.get("name") {
            rows.package.push((dir.clone(), name.clone()));
        }
        let mut fields = Vec::new();
        strings("", json, &mut fields);
        rows.package_field.extend(fields.into_iter().map(|(k, v)| (dir.clone(), k, v)));
    }
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
pub(crate) fn relations(store: &ContentStore, root: &Path, docs: &[String], opts: &Value, reads: &dyn Fn(&str) -> bool) -> StoreResult<Vec<(&'static str, Relation)>> {
    let mut rows = Rows::default();
    if reads("option") || reads("option_item") {
        options(opts, &mut rows);
    }
    if ["file", "package", "package_field", "package_for"].iter().any(|r| reads(r)) {
        tree(root, docs, &mut rows);
    }
    let mut texts: Vec<&str> = Vec::new();
    rows.option.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.option_item.iter().for_each(|(a, _, b)| texts.extend([a.as_str(), b.as_str()]));
    texts.extend(rows.file.iter().map(String::as_str));
    rows.package.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.package_field.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    rows.package_for.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
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
    Ok(out)
}

#[cfg(test)]
mod tests {
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
        for (p, t) in [("web/package.json", r#"{"name": "@a/web", "engines": {"node": ">=18"}, "n": 1}"#), ("web/src/a.ts", ""), ("node_modules/x/b.js", ""), ("c.py", "")] {
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
}
