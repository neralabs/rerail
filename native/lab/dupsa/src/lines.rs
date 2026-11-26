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
    read_with(root, &files(root, include, ext), &mut LineDict::new())
}

/// The normalised lines' ids (an interned text). A dictionary a run before saved is read in place: the lines by id,
/// and the ids in the lines' order, for a lookup by binary search. A line it lacks gets the next id.
pub struct LineDict {
    saved: Option<(&'static [u32], &'static [u8], &'static [u32])>,
    added: HashMap<String, u32>,
    pub next: u32,
}

impl Default for LineDict {
    fn default() -> Self {
        Self::new()
    }
}

impl LineDict {
    pub fn new() -> Self {
        Self { saved: None, added: HashMap::new(), next: 0 }
    }

    fn saved_line(&self, id: u32) -> &[u8] {
        let (off, bytes, _) = self.saved.unwrap();
        &bytes[off[id as usize] as usize..off[id as usize + 1] as usize]
    }

    pub fn id(&mut self, line: &str) -> u32 {
        if let Some((_, _, sorted)) = self.saved {
            if let Ok(k) = sorted.binary_search_by(|&i| self.saved_line(i).cmp(line.as_bytes())) {
                return sorted[k];
            }
        }
        if let Some(&id) = self.added.get(line) {
            return id;
        }
        let id = self.next;
        self.next += 1;
        self.added.insert(line.to_owned(), id);
        id
    }

    /// The dictionary as bytes (`store` arrays): each line's start by id, the lines, the ids in the lines' order.
    pub fn write(&self, out: &mut Vec<u8>) {
        assert!(self.saved.is_none(), "a dictionary is saved once, from the run that built it");
        let mut by_id = vec![""; self.next as usize];
        for (l, &id) in &self.added {
            by_id[id as usize] = l;
        }
        let (mut off, mut bytes) = (vec![0u32], Vec::new());
        for l in &by_id {
            bytes.extend_from_slice(l.as_bytes());
            off.push(bytes.len() as u32);
        }
        let mut sorted: Vec<u32> = (0..self.next).collect();
        sorted.sort_unstable_by(|&a, &b| by_id[a as usize].cmp(by_id[b as usize]));
        crate::store::put(out, &off);
        crate::store::put(out, &bytes);
        crate::store::put(out, &sorted);
    }

    pub fn read(b: &'static [u8], at: &mut usize) -> Self {
        let off = crate::store::view::<u32>(b, at);
        let bytes = crate::store::view::<u8>(b, at);
        let sorted = crate::store::view::<u32>(b, at);
        Self { saved: Some((off, bytes, sorted)), added: HashMap::new(), next: off.len() as u32 - 1 }
    }
}

/// The base's lines as a run before saved them, read in place: per document its first unit in `ids` and `lines`.
pub struct MappedLines {
    pub paths: Vec<String>,
    pub offsets: &'static [u32],
    pub ids: &'static [u32],
    pub lines: &'static [u32],
    pub distinct: u32,
}

impl Docs for MappedLines {
    fn count(&self) -> usize {
        self.paths.len()
    }
    fn doc_len(&self, d: usize) -> usize {
        (self.offsets[d + 1] - self.offsets[d]) as usize
    }
    #[inline(always)]
    fn at(&self, d: usize, j: usize) -> u32 {
        self.ids[self.offsets[d] as usize + j]
    }
    fn upper(&self) -> u32 {
        self.distinct.saturating_sub(1)
    }
}

impl MappedLines {
    /// A line documents' paths, ids and line numbers as bytes (`store` arrays).
    pub fn write(docs: &LineDocs, out: &mut Vec<u8>) {
        crate::store::put(out, docs.paths.join("\n").as_bytes());
        let mut off = vec![0u32];
        for ids in &docs.ids {
            off.push(off.last().unwrap() + ids.len() as u32);
        }
        crate::store::put(out, &off);
        crate::store::put(out, &docs.ids.concat());
        crate::store::put(out, &docs.lines.concat());
        crate::store::put(out, &[docs.distinct]);
    }

    pub fn read(b: &'static [u8], at: &mut usize) -> Self {
        let paths = std::str::from_utf8(crate::store::view::<u8>(b, at)).expect("paths");
        let paths = if paths.is_empty() { Vec::new() } else { paths.split('\n').map(str::to_owned).collect() };
        let offsets = crate::store::view::<u32>(b, at);
        let ids = crate::store::view::<u32>(b, at);
        let lines = crate::store::view::<u32>(b, at);
        let distinct = crate::store::view::<u32>(b, at)[0];
        Self { paths, offsets, ids, lines, distinct }
    }
}

/// The files `rels` under `root`, their lines numbered by `dict` (shared between batches, so that a line has one id
/// in every batch; new lines get the next).
pub fn read_with(root: &Path, rels: &[String], dict: &mut LineDict) -> LineDocs {
    let mut docs = LineDocs { paths: Vec::new(), ids: Vec::new(), lines: Vec::new(), distinct: 0 };
    let mut norm = String::new();
    for rel in rels.iter().cloned() {
        // A file that is not UTF-8 is not text.
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else { continue };
        let hash = hash_comments(&rel);
        let (mut ids, mut lines) = (Vec::new(), Vec::new());
        for (k, raw) in text.split('\n').enumerate() {
            if !normalise(raw, hash, &mut norm) {
                continue;
            }
            ids.push(dict.id(&norm));
            lines.push(k as u32 + 1);
        }
        docs.paths.push(rel);
        docs.ids.push(ids);
        docs.lines.push(lines);
    }
    docs.distinct = dict.next;
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
