//! The file system a program reads and the documents it loads (t21 E1, E2): the generic half of typing against
//! libraries. No rule here knows a language; a language's `env.rr` and `modules.rr` say what to read.
//!
//! A program may read, as relations:
//! - `dir_entry(dir, name, is_dir)`: the entries of a directory;
//! - `file_line(path, i, text)`: the lines of a file, from 1;
//!
//! and ask for documents with `load(path, module, grammar)`: the file at `path`, parsed in `grammar`, as module
//! `module` (its resolution's answer, which names the document as a file of the corpus is named by its path). A
//! loaded document's path is the file's canonical path, and `loaded(path, module, grammar)` says which documents
//! were loaded, by that path. A loaded document is evaluated like any other.
//!
//! Reading `dir_entry` or `file_line` asks for it: `rr::lower` derives `rr_dir_req(dir)` and `rr_file_req(path)` from
//! the literals that bind the key, as `parsed` asks for a guest (decision 0055). The requests are answered inside the
//! fixpoint (decision 0065 §2a): the cross-file part's cone of requests runs first and pauses for each wave's new
//! requests; a directory is listed, a file read and a document loaded (through the chunk path, as any file is), and
//! the answers are the next wave's rows. The rest of the cross-file part then runs once over every document. This
//! module is the part that touches the file system: what may be read, reading it, and naming a loaded document.
//!
//! Only paths under the corpus root, or under a directory a setting names (`env(k, v)` with v a directory, relative
//! to the root or absolute), are read. A path elsewhere, or missing, has no entries or lines.

use super::front;
use crate::code_rule_dsl::logic::rr::WORLD;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Documents loaded at most, unless `env("load.budget", n)` says otherwise.
const BUDGET: usize = 4000;
/// The request of a load, and the relation of what was loaded.
pub(super) const LOAD: &str = "load";
pub(super) const LOADED: &str = "loaded";

/// Whether the program reads the file system or loads documents.
pub(super) fn wanted(prog: &front::Program) -> bool {
    prog.rules.iter().any(|r| r.head.rel == LOAD || WORLD.iter().any(|(_, d)| *d == r.head.rel))
}

/// The requests a program makes, each with what answers it, before a load's per-file rows are added
/// (`corpus::loading`): a directory's entries, a file's lines and a load's `loaded` row.
pub(super) fn answers(prog: &front::Program) -> BTreeMap<String, Vec<String>> {
    WORLD
        .iter()
        .map(|(rel, req)| (*req, *rel))
        .chain([(LOAD, LOADED)])
        .filter(|(req, _)| prog.rules.iter().any(|x| x.head.rel == *req))
        .map(|(req, rel)| (req.to_owned(), vec![rel.to_owned()]))
        .collect()
}

/// A loaded document: its path as the rule gave it, and the module it is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Loaded {
    pub(super) path: String,
    pub(super) module: String,
    pub(super) grammar: String,
}

#[derive(Clone, Debug)]
struct DirectoryEntry {
    name: String,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
    bytes: i64,
}

#[derive(Default)]
pub(super) struct World {
    roots: Vec<PathBuf>,
    root: PathBuf,
    dirs: BTreeMap<String, Vec<DirectoryEntry>>,
    files: BTreeMap<String, Vec<String>>,
    host: BTreeMap<String, Option<String>>,
    loads: BTreeMap<String, Loaded>,
    refused: BTreeSet<String>,
    budget: usize,
    /// Loads past the budget, or in another grammar than the run's, not taken.
    pub(super) dropped: usize,
}

impl World {
    /// A demanded host value, cached for this execution, including absent values.
    /// Host values are world inputs, never persisted as program facts or memo keys.
    pub(super) fn host(&mut self, key: &str) -> Option<String> {
        self.host_with(key, |key| match key {
            "cpu_count" => cpu_count().map(|n| n.to_string()),
            _ => key.strip_prefix("env:").and_then(|name| std::env::var(name).ok()),
        })
    }

    fn host_with(&mut self, key: &str, lookup: impl FnOnce(&str) -> Option<String>) -> Option<String> {
        self.host.entry(key.to_owned()).or_insert_with(|| lookup(key)).clone()
    }

    /// The roots a request may read: the corpus root, and each setting's value that is a directory.
    pub(super) fn new(root: &Path, prog: &front::Program) -> World {
        let canonical = |p: &Path| std::fs::canonicalize(p).ok();
        let mut roots: Vec<PathBuf> = canonical(root).into_iter().collect();
        let mut budget = BUDGET;
        for row in prog.facts.get("env").into_iter().flatten() {
            if let [front::Value::Str(k), front::Value::Str(v)] = row.as_slice() {
                if k == "load.budget" {
                    budget = v.parse().unwrap_or(budget);
                }
                if let Some(dir) = canonical(&root.join(v)).filter(|p| p.is_dir()) {
                    roots.push(dir);
                }
            }
        }
        World { roots, root: root.to_path_buf(), budget, ..World::default() }
    }

    /// The file `asked` names, when it lies under a root: relative to the corpus root, or absolute.
    fn allowed(&self, asked: &str) -> Option<PathBuf> {
        let path = std::fs::canonicalize(self.root.join(asked)).ok()?;
        self.roots.iter().any(|r| path.starts_with(r)).then_some(path)
    }

    /// Read each requested directory once. Both public projections share these
    /// rows; metadata is not a filesystem callback for each rule or node.
    fn read_directory(&mut self, asked: &str) {
        if self.dirs.contains_key(asked) || self.refused.contains(asked) { return; }
        let Some(path) = self.allowed(asked) else {
            self.refused.insert(asked.to_owned());
            return;
        };
        let mut entries: Vec<DirectoryEntry> = std::fs::read_dir(&path)
            .map(|it| it.filter_map(|e| e.ok()).map(|e| {
                let metadata = e.path().metadata().ok();
                DirectoryEntry {
                    name: e.file_name().to_string_lossy().into_owned(),
                    is_dir: metadata.as_ref().is_some_and(|m| m.is_dir()),
                    is_file: metadata.as_ref().is_some_and(|m| m.is_file()),
                    is_symlink: e.file_type().is_ok_and(|t| t.is_symlink()),
                    bytes: metadata.as_ref().map_or(0, |m| m.len() as i64),
                }
            }).collect())
            .unwrap_or_default();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        self.dirs.insert(asked.to_owned(), entries);
    }

    /// A directory's entries, sorted (none when it is refused or missing).
    pub(super) fn dir(&mut self, asked: &str) -> Vec<(String, bool)> {
        self.read_directory(asked);
        self.dirs.get(asked).into_iter().flatten().map(|e| (e.name.clone(), e.is_dir)).collect()
    }

    /// File type, link flag, and followed-target size from that same listing.
    pub(super) fn metadata(&mut self, asked: &str) -> Vec<(String, bool, bool, i64)> {
        self.read_directory(asked);
        self.dirs.get(asked).into_iter().flatten().map(|e| (e.name.clone(), e.is_file, e.is_symlink, e.bytes)).collect()
    }

    /// A file's lines (none when it is refused or missing).
    pub(super) fn file(&mut self, asked: &str) -> Vec<String> {
        if let Some(lines) = self.files.get(asked) {
            return lines.clone();
        }
        let Some(text) = self.allowed(asked).and_then(|p| std::fs::read_to_string(p).ok()) else {
            self.refused.insert(asked.to_owned());
            return Vec::new();
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = text.split_terminator('\n').map(str::to_owned).collect();
        self.files.insert(asked.to_owned(), lines.clone());
        lines
    }

    /// A load's document: the file's canonical path, when it is new, allowed, in the run's grammar and within the
    /// budget. A file of the corpus is a document already, so its load takes nothing. A loaded document is named by
    /// its canonical path: the same file is one document, and its key and memo entries are the same in every
    /// workspace.
    /// The corpus's documents as canonical paths, as `load` compares them, with their given names too.
    pub(super) fn canonical_corpus<'a>(&self, paths: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for p in paths {
            if let Ok(c) = std::fs::canonicalize(self.root.join(p)) {
                out.insert(c.to_string_lossy().into_owned());
            }
            out.insert(p.clone());
        }
        out
    }

    pub(super) fn load(&mut self, asked: &str, module: &str, grammar: &str, run_grammar: &str, corpus: &BTreeSet<String>) -> Option<Loaded> {
        if corpus.contains(asked) || self.refused.contains(asked) {
            return None;
        }
        let Some(path) = self.allowed(asked).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()) else {
            self.dropped += 1;
            self.refused.insert(asked.to_owned());
            return None;
        };
        // `corpus` is canonical (`canonical_corpus`): a workspace file reached through an import is not loaded again.
        if corpus.contains(&path) || self.loads.contains_key(&path) {
            return None;
        }
        if grammar != run_grammar || self.loads.len() >= self.budget {
            self.dropped += 1;
            self.refused.insert(asked.to_owned());
            return None;
        }
        let loaded = Loaded { path: path.clone(), module: module.to_owned(), grammar: grammar.to_owned() };
        self.loads.insert(path, loaded.clone());
        Some(loaded)
    }

    pub(super) fn summary(&self, pauses: usize) -> String {
        let entries: usize = self.dirs.values().map(Vec::len).sum();
        let lines: usize = self.files.values().map(Vec::len).sum();
        format!(
            "requests answered in {pauses} pauses of the fixpoint: {} directories ({entries} entries), {} files ({lines} lines), {} documents loaded; {} paths refused or missing, {} loads not taken",
            self.dirs.len(),
            self.files.len(),
            self.loads.len(),
            self.refused.len(),
            self.dropped
        )
    }
}

/// Match the OS online-processor count, rather than the process's affinity limit.
fn cpu_count() -> Option<usize> {
    #[cfg(unix)]
    {
        // SAFETY: sysconf has no pointer arguments and only reads host metadata.
        let count = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
        (count > 0).then_some(count as usize)
    }
    #[cfg(not(unix))]
    { std::thread::available_parallelism().ok().map(usize::from) }
}

/// A document's path as the content store keys it: a loaded document's canonical path without its root.
pub(super) fn store_path(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_values_and_absence_are_read_once_per_execution() {
        let mut world = World::default();
        assert_eq!(world.host_with("env:present", |_| Some("first".into())), Some("first".into()));
        assert_eq!(world.host_with("env:present", |_| panic!("read twice")), Some("first".into()));
        assert_eq!(world.host_with("env:missing", |_| None), None);
        assert_eq!(world.host_with("env:missing", |_| panic!("absence read twice")), None);
        let mut next = World::default();
        assert_eq!(next.host_with("env:present", |_| Some("changed".into())), Some("changed".into()));
    }

    #[test]
    fn directory_projections_share_one_listing_but_new_runs_refresh_it() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-directory-cache");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::write(root.join("pkg/a.py"), "abc").unwrap();
        let program = front::parse("").unwrap();
        let mut world = World::new(&root, &program);
        assert_eq!(world.dir("pkg"), vec![("a.py".to_owned(), false)]);
        std::fs::write(root.join("pkg/a.py"), "changed").unwrap();
        std::fs::write(root.join("pkg/b.py"), "new").unwrap();
        assert_eq!(world.metadata("pkg"), vec![("a.py".to_owned(), true, false, 3)]);
        let mut refreshed = World::new(&root, &program);
        assert_eq!(refreshed.metadata("pkg"), vec![("a.py".to_owned(), true, false, 7), ("b.py".to_owned(), true, false, 3)]);
        assert_eq!(refreshed.dir("pkg"), vec![("a.py".to_owned(), false), ("b.py".to_owned(), false)]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
