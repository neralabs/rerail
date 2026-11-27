//! ECMAScript's project files as rule inputs (decision 0710), registered with the repository reader at load
//! (`facts::repository::install_project_reader`); the core names none of them. Each is declared, with its placement,
//! in the TSX library (`tsx/project.rerail`).
//!
//! - `tsconfig(dir, base_url)`, `tsconfig_path(dir, pattern, target)`: each `tsconfig.json` or `jsconfig.json`'s
//!   `compilerOptions`, read as the resolver reads them (`tsconfig::Chains`: get-tsconfig's `extends` chain, each base
//!   read once for every configuration of the tree, JSONC as TypeScript reads it), `baseUrl` from the root (each
//!   configuration's against its own folder; the configuration's folder where none is set), each `paths` target as
//!   written.
//! - `tsconfig_for(path, dir)`: a document's nearest `tsconfig.json`, else `jsconfig.json`.
//! - `npm_ignored(path)`: each file under a `package.json` (its nearest) that npm leaves out of that package:
//!   eslint-plugin-n's `getNpmignore(path).match`, as `package_manifest::Manifest::publishes` reads the package's
//!   `files` and `main`, else its folder's `.npmignore`, else its `.gitignore`. A file under no package, or under one
//!   whose `package.json` does not parse, has no row.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::fs::{HeldFiles, View};
use super::json::Json;
use crate::code_rule_dsl::facts::repository::{ProjectReader, Tree};
use crate::code_rule_dsl::package_manifest::Manifest;

/// The reader ECMAScript's adapter registers.
pub const READER: ProjectReader = ProjectReader { relations: RELATIONS, rows };

const RELATIONS: &[(&str, &[&str])] = &[
    ("tsconfig", &["dir", "base_url"]),
    ("tsconfig_for", &["path", "dir"]),
    ("tsconfig_path", &["dir", "pattern", "target"]),
    ("npm_ignored", &["path"]),
];

/// The names a project configuration has, the first found in a folder governing it.
const NAMES: [&str; 2] = ["tsconfig.json", "jsconfig.json"];

fn rows(tree: &Tree<'_>) -> Vec<(&'static str, Vec<Vec<String>>)> {
    let (tsconfig, tsconfig_for, tsconfig_path) = configurations(tree);
    vec![("tsconfig", tsconfig), ("tsconfig_for", tsconfig_for), ("tsconfig_path", tsconfig_path), ("npm_ignored", npm_ignored(tree))]
}

/// A file's folder (from the root; `""` at the root).
fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The nearest of `folders` at or above `path`'s folder.
fn nearest<'a, T>(folders: &'a BTreeMap<String, T>, path: &str) -> Option<(&'a str, &'a T)> {
    let mut d = folder(path);
    loop {
        if let Some((k, v)) = folders.get_key_value(d) {
            return Some((k.as_str(), v));
        }
        if d.is_empty() {
            return None;
        }
        d = folder(d);
    }
}

/// `tsconfig`, `tsconfig_for` and `tsconfig_path`.
#[allow(clippy::type_complexity)]
fn configurations(tree: &Tree<'_>) -> (Vec<Vec<String>>, Vec<Vec<String>>, Vec<Vec<String>>) {
    let name = |f: &str| f.rsplit_once('/').map_or(f, |(_, n)| n).to_owned();
    let rank = |f: &str| NAMES.iter().position(|c| *c == name(f));
    let mut configs: BTreeMap<String, String> = BTreeMap::new();
    for f in tree.files {
        if let Some(r) = rank(f)
            && configs.get(folder(f)).is_none_or(|have| rank(have) > Some(r))
        {
            configs.insert(folder(f).to_owned(), f.clone());
        }
    }
    let view = View::new(Arc::new(HeldFiles::default()), Some(tree.root));
    let (mut tsconfig, mut tsconfig_path) = (Vec::new(), Vec::new());
    let mut chains = super::tsconfig::Chains::new(&view, true);
    for (dir, file) in &configs {
        let Some(options) = chains.of(&view.absolute(file)) else { continue };
        let option = |key: &str| options.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let base = option("baseUrl").and_then(Json::as_str).map(|url| view.relative(url).unwrap_or(url).to_owned()).unwrap_or_else(|| dir.clone());
        tsconfig.push(vec![dir.clone(), base]);
        for (pattern, targets) in option("paths").and_then(Json::entries).into_iter().flatten() {
            for target in targets.as_array().into_iter().flatten().filter_map(Json::as_str) {
                tsconfig_path.push(vec![dir.clone(), pattern.clone(), target.to_owned()]);
            }
        }
    }
    let tsconfig_for = tree.docs.iter().filter_map(|path| nearest(&configs, path).map(|(dir, _)| vec![path.clone(), dir.to_owned()])).collect();
    (tsconfig, tsconfig_for, tsconfig_path)
}

/// `npm_ignored`.
fn npm_ignored(tree: &Tree<'_>) -> Vec<Vec<String>> {
    let manifests: BTreeMap<String, Option<Manifest>> = tree
        .packages
        .iter()
        // Every package.json, read or not: npm's nearest manifest decides, and one it cannot read decides nothing
        // (`None`) rather than handing its files to the package above (review F9).
        .map(|(dir, json)| (dir.clone(), manifest(tree.root, dir, json.as_ref())))
        .collect();
    tree.files
        .iter()
        .filter(|path| nearest(&manifests, path).and_then(|(_, m)| m.as_ref()).is_some_and(|m| m.publishes(path) == Some(false)))
        .map(|path| vec![path.clone()])
        .collect()
}

/// The package in `dir` with its folder's `.npmignore` and `.gitignore`; `None` where its package.json (`json`) does
/// not parse to an object, so that neither it nor its folder's ignore files decide which files npm leaves out (review
/// F10).
fn manifest(root: &std::path::Path, dir: &str, json: Option<&serde_json::Value>) -> Option<Manifest> {
    let Some(serde_json::Value::Object(_)) = json else { return None };
    let read = |name: &str| std::fs::read_to_string(root.join(dir).join(name)).ok();
    let bytes = std::fs::read(root.join(dir).join("package.json")).unwrap_or_default();
    Some(Manifest::parse(dir.to_owned(), &bytes).with_ignore_files(read(".npmignore"), read(".gitignore")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_of(files: &[(&str, &str)]) -> (std::path::PathBuf, Vec<String>, BTreeMap<String, Option<serde_json::Value>>) {
        let root = std::env::temp_dir().join(format!("rerail-ecma-project-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut packages = BTreeMap::new();
        for (p, t) in files {
            let to = root.join(p);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, t).unwrap();
            if p.ends_with("package.json") {
                packages.insert(folder(p).to_owned(), serde_json::from_str(t).ok());
            }
        }
        let mut listed: Vec<String> = files.iter().map(|(p, _)| (*p).to_owned()).collect();
        listed.sort();
        (root, listed, packages)
    }

    /// Each configuration's `baseUrl` from the root, inherited through `extends` against the configuration that sets
    /// it; `paths` targets as written; a folder's `tsconfig.json` over its `jsconfig.json`; each document's nearest.
    /// The files are JSONC: a comment after a trailing comma (review B1) does not drop the configuration.
    #[test]
    fn a_tsconfig_is_read_as_the_resolver_reads_it() {
        let (root, files, packages) = tree_of(&[
            ("base/tsconfig.base.json", "{\n  \"compilerOptions\": { \"baseUrl\": \"src\", },\n}\n"),
            ("web/tsconfig.json", "{\n  // the app\n  \"extends\": \"../base/tsconfig.base.json\",\n  \"compilerOptions\": {\n    \"paths\": { \"@/*\": [\"src/*\", \"gen/*\"], }, // aliases\n  },\n}\n"),
            ("web/jsconfig.json", "{ \"compilerOptions\": { \"baseUrl\": \"other\" } }"),
            ("lib/jsconfig.json", "{ \"compilerOptions\": {} }"),
            ("web/src/a.ts", ""),
            ("lib/x/b.js", ""),
            ("c.ts", ""),
        ]);
        let docs = ["web/src/a.ts", "lib/x/b.js", "c.ts"].map(str::to_owned);
        let (tsconfig, tsconfig_for, tsconfig_path) = configurations(&Tree { root: &root, files: &files, docs: &docs, packages: &packages });
        let rows = |v: &[&[&str]]| v.iter().map(|r| r.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(tsconfig, rows(&[&["lib", "lib"], &["web", "base/src"]]));
        assert_eq!(tsconfig_path, rows(&[&["web", "@/*", "src/*"], &["web", "@/*", "gen/*"]]));
        assert_eq!(tsconfig_for, rows(&[&["web/src/a.ts", "web"], &["lib/x/b.js", "lib"]]));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Each file under a package.json is asked of its nearest package's publication (`files`, else `.npmignore`, else
    /// `.gitignore`); a file under no package has no row.
    #[test]
    fn the_files_npm_leaves_out_of_their_package_are_npm_ignored() {
        let (root, files, packages) = tree_of(&[
            ("lib/package.json", r#"{"name": "lib", "files": ["dist"]}"#),
            ("lib/dist/a.js", ""),
            ("lib/test/a.js", ""),
            ("cli/package.json", r#"{"name": "cli"}"#),
            ("cli/.npmignore", "scripts/\n"),
            ("cli/index.js", ""),
            ("cli/scripts/build.js", ""),
            ("tools/x.js", ""),
            // A package.json npm cannot read is still the nearest one: its folder's files are no longer `lib`'s to
            // decide (review F9).
            ("lib/sub/package.json", "{not json"),
            ("lib/sub/x.js", ""),
        ]);
        let ignored = npm_ignored(&Tree { root: &root, files: &files, docs: &[], packages: &packages });
        let ignored: Vec<&str> = ignored.iter().map(|r| r[0].as_str()).collect();
        assert!(ignored.contains(&"lib/test/a.js") && ignored.contains(&"cli/scripts/build.js"), "{ignored:?}");
        assert!(!ignored.iter().any(|p| ["lib/dist/a.js", "lib/package.json", "cli/index.js", "cli/package.json", "tools/x.js", "lib/sub/x.js"].contains(p)), "{ignored:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A package.json that does not parse decides nothing about which of its folder's files npm leaves out, its
    /// folder's ignore files included (review F10): a file its `.gitignore` lists is not npm_ignored on that account,
    /// where the same `.gitignore` beside a package.json that parses leaves it out.
    #[test]
    fn an_unparsed_package_json_leaves_its_folders_ignore_files_undecided() {
        let (root, files, packages) = tree_of(&[
            ("lib/package.json", r#"{"name": "lib"}"#),
            ("lib/.gitignore", "dist\n"),
            ("lib/dist/a.js", ""),
            ("lib/sub/package.json", "{not json"),
            ("lib/sub/.gitignore", "dist\n"),
            ("lib/sub/dist/a.js", ""),
        ]);
        let ignored = npm_ignored(&Tree { root: &root, files: &files, docs: &[], packages: &packages });
        let ignored: Vec<&str> = ignored.iter().map(|r| r[0].as_str()).collect();
        assert!(ignored.contains(&"lib/dist/a.js"), "{ignored:?}");
        assert!(!ignored.iter().any(|p| p.starts_with("lib/sub/")), "{ignored:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
