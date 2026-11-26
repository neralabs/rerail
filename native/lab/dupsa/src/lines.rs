//! Jeff's text: every file under the scope read as text, each line normalised, the lines that count numbered by
//! their normalised text (an interned text: the line's id, written once where it is computed).
//!
//! As `no-duplicate-blocks.rerail` states it:
//! - each line's whitespace runs become one space, and the line is trimmed;
//! - a line counts unless it is blank, or starts with `//`, `/*` or `*`, or with `#` in a file whose comments start
//!   with `#` (Python, shell, YAML, TOML, Ruby, …);
//! - left out: node_modules, `.d.ts`, minified and map files, pack rules and candidates, licences and notices, lock
//!   files, fixtures and snapshots, expected and generated files, and binaries.

use crate::multi::Docs;
use std::collections::HashMap;
use std::path::Path;

pub struct LineDocs {
    pub paths: Vec<String>,
    /// Per document: each counted line's id, and its line number in the file (from 1).
    pub ids: Vec<Vec<u32>>,
    pub lines: Vec<Vec<u32>>,
    pub distinct: u32,
}

impl Docs for LineDocs {
    fn count(&self) -> usize {
        self.ids.len()
    }
    fn doc_len(&self, d: usize) -> usize {
        self.ids[d].len()
    }
    #[inline(always)]
    fn at(&self, d: usize, j: usize) -> u32 {
        self.ids[d][j]
    }
    fn upper(&self) -> u32 {
        self.distinct.saturating_sub(1)
    }
}

const SKIPPED_DIRS: &[&str] = &[".git", "node_modules", ".venv", "target", "dist"];

const BINARY: &[&str] =
    &["png", "jpg", "gif", "ico", "svg", "wasm", "gz", "zip", "sqlite", "bin", "woff", "woff2", "ttf", "pdf"];

/// Jeff's `.scope exclude` list, on a path relative to the root.
pub fn excluded(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let (dirs, name) = parts.split_at(parts.len() - 1);
    let name = name[0];
    let dir_is = |d: &str| dirs.contains(&d);
    if dir_is("node_modules") || dir_is("licenses") || dir_is("LICENSES") || dir_is("fixtures") || dir_is("__snapshots__") || dir_is("generated") {
        return true;
    }
    if parts.len() >= 4 && parts[0] == "packs" && (parts[2] == "rules" || parts[2] == "candidates") {
        return true;
    }
    if name.starts_with("LICENSE") || name.starts_with("NOTICE") || name == "package-lock.json" {
        return true;
    }
    if [".d.ts", ".min.js", ".map", ".lock", ".snap"].iter().any(|s| name.ends_with(s)) {
        return true;
    }
    if name.contains(".expected.") || name.contains(".generated.") {
        return true;
    }
    name.rsplit_once('.').is_some_and(|(_, ext)| BINARY.contains(&ext))
}

/// Jeff's `hash_comments`: a file whose comments start with `#`, by its extension.
pub fn hash_comments(rel: &str) -> bool {
    const EXT: &[&str] = &["py", "pyi", "sh", "bash", "zsh", "yml", "yaml", "toml", "rb", "pl", "r", "cfg", "ini", "conf", "properties"];
    rel.rsplit_once('.').is_some_and(|(_, ext)| EXT.contains(&ext))
}

/// The line normalised (whitespace runs to one space, trimmed), or `None` when it does not count.
pub fn normalise(raw: &str, hash: bool, into: &mut String) -> bool {
    into.clear();
    let mut space = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !into.is_empty() {
                into.push(' ');
            }
            space = false;
            into.push(c);
        }
    }
    !(into.is_empty() || into.starts_with("//") || into.starts_with("/*") || into.starts_with('*') || (hash && into.starts_with('#')))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(kind) = e.file_type() else { continue };
        let path = e.path();
        if kind.is_dir() {
            if !SKIPPED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()) {
                walk(root, &path, out);
            }
        } else if kind.is_file() {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
        }
    }
}

/// The files under `root` whose relative path starts with one of `include` (all when empty), less the excluded.
/// With `ext` not empty, only files of those extensions.
pub fn files(root: &Path, include: &[String], ext: &[String]) -> Vec<String> {
    let mut all = Vec::new();
    walk(root, root, &mut all);
    all.retain(|p| (include.is_empty() || include.iter().any(|i| p.starts_with(i.as_str()))) && !excluded(p));
    all.retain(|p| ext.is_empty() || p.rsplit_once('.').is_some_and(|(_, e)| ext.iter().any(|x| x == e)));
    all
}

pub fn read(root: &Path, include: &[String], ext: &[String]) -> LineDocs {
    let mut ids_of: HashMap<String, u32> = HashMap::new();
    let mut docs = LineDocs { paths: Vec::new(), ids: Vec::new(), lines: Vec::new(), distinct: 0 };
    let mut norm = String::new();
    for rel in files(root, include, ext) {
        // A file that is not UTF-8 is not text.
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else { continue };
        let hash = hash_comments(&rel);
        let (mut ids, mut lines) = (Vec::new(), Vec::new());
        for (k, raw) in text.split('\n').enumerate() {
            if !normalise(raw, hash, &mut norm) {
                continue;
            }
            let next = ids_of.len() as u32;
            let id = *ids_of.entry(norm.clone()).or_insert(next);
            ids.push(id);
            lines.push(k as u32 + 1);
        }
        docs.paths.push(rel);
        docs.ids.push(ids);
        docs.lines.push(lines);
    }
    docs.distinct = ids_of.len() as u32;
    docs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalising() {
        let mut s = String::new();
        assert!(normalise("   let  x =\t1;  ", false, &mut s));
        assert_eq!(s, "let x = 1;");
        assert!(!normalise("  // c", false, &mut s));
        assert!(!normalise("   * doc", false, &mut s));
        assert!(!normalise(" \t ", false, &mut s));
        assert!(normalise("# heading", false, &mut s));
        assert!(!normalise("# comment", true, &mut s));
    }

    #[test]
    fn excludes() {
        assert!(excluded("packs/bandit/rules/b110.json"));
        assert!(!excluded("packs/bandit/src/x.ts"));
        assert!(excluded("src/a/LICENSE.md"));
        assert!(excluded("test/fixtures/a.ts"));
        assert!(excluded("src/x.d.ts"));
        assert!(excluded("src/logo.svg"));
        assert!(!excluded("src/a.ts"));
    }
}
