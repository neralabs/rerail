//! The file system a program reads and the documents it loads (t21 E1, E2): the generic half of typing against
//! libraries. No rule here knows a language; a language's `env.rr` and `modules.rr` say what to read.
//!
//! A program may read, as relations:
//! - `dir_entry(dir, name, is_dir)`: the entries of a directory;
//! - `file_line(path, i, text)`: the lines of a file, from 1;
//! - `file_digest(path, c)`: the SHA-256 of a file's bytes, hex (what a cache key names a file's content by);
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

#[derive(Default)]
pub(super) struct World {
    roots: Vec<PathBuf>,
    root: PathBuf,
    dirs: BTreeMap<String, Vec<(String, bool)>>,
    files: BTreeMap<String, Vec<String>>,
    digests: BTreeMap<String, String>,
    loads: BTreeMap<String, Loaded>,
    refused: BTreeSet<String>,
    budget: usize,
    /// The documents loaded from beyond the corpus root (the libraries'), which the budget bounds. A source file of
    /// the run's own tree is always loaded (phase 1, `sources.rr`).
    beyond: usize,
    /// Loads past the budget, or in another grammar than the run's, not taken.
    pub(super) dropped: usize,
    /// Documents loaded this run and taken back since (`unload`): out of the budget, their rows still held where
    /// they were made, so a load asked again takes them back in place (`reload`).
    taken: BTreeMap<String, Loaded>,
}

impl World {
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
        // What the requests may read is watched (decision 0105): a directory listed or a file read that the
        // journal says is unchanged is not statted.
        super::observe(|o| o.watch(&roots));
        World { roots, root: root.to_path_buf(), budget, ..World::default() }
    }

    /// The file `asked` names, when it lies under a root: relative to the corpus root, or absolute.
    /// Resolved by phase 0's observer: where the journal says nothing changed, with no `realpath` (decision 0105).
    fn allowed(&self, asked: &str) -> Option<PathBuf> {
        let path = super::observe(|o| o.canonical(&self.root.join(asked)))?;
        self.roots.iter().any(|r| path.starts_with(r)).then_some(path)
    }

    /// A directory's entries, sorted (none when it is refused or missing).
    pub(super) fn dir(&mut self, asked: &str) -> Vec<(String, bool)> {
        if let Some(entries) = self.dirs.get(asked) {
            return entries.clone();
        }
        let Some(path) = self.allowed(asked) else {
            self.refused.insert(asked.to_owned());
            return Vec::new();
        };
        // Phase 0's observer: listed again only when the directory's stat changed.
        let entries: Vec<(String, bool)> = super::observe(|o| o.listing(&path)).map(|l| l.as_ref().clone()).unwrap_or_default();
        self.dirs.insert(asked.to_owned(), entries.clone());
        entries
    }

    /// A file's lines (none when it is refused or missing).
    pub(super) fn file(&mut self, asked: &str) -> Vec<String> {
        if let Some(lines) = self.files.get(asked) {
            return lines.clone();
        }
        let Some(text) = self.allowed(asked).and_then(|p| super::observe(|o| o.read(&p))).and_then(|(bytes, _)| String::from_utf8(bytes).ok()) else {
            self.refused.insert(asked.to_owned());
            return Vec::new();
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = text.split_terminator('\n').map(str::to_owned).collect();
        self.files.insert(asked.to_owned(), lines.clone());
        lines
    }

    /// A file's content digest (none when it is refused or missing): the observer's, read only where it cannot vouch
    /// for the file.
    pub(super) fn digest(&mut self, asked: &str) -> Option<String> {
        if let Some(d) = self.digests.get(asked) {
            return Some(d.clone());
        }
        let Some(d) = self.allowed(asked).filter(|p| p.is_file()).map(|p| bytes_digest(&p)).filter(|d| !d.is_empty()) else {
            self.refused.insert(asked.to_owned());
            return None;
        };
        self.digests.insert(asked.to_owned(), d.clone());
        Some(d)
    }

    /// A load's document: the file's canonical path, when it is new, allowed, in the run's grammar and within the
    /// budget. A file of the corpus is a document already, so its load takes nothing. A loaded document is named by
    /// its canonical path: the same file is one document, and its key and memo entries are the same in every
    /// workspace.
    /// The corpus's documents as canonical paths, as `load` compares them, with their given names too.
    pub(super) fn canonical_corpus<'a>(&self, paths: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for p in paths {
            if let Some(c) = super::observe(|o| o.canonical(&self.root.join(p))) {
                out.insert(c.to_string_lossy().into_owned());
            }
            out.insert(p.clone());
        }
        out
    }

    /// The document a load of `asked` gave earlier in this run, if any: a load asked again is answered alike.
    pub(super) fn loaded_as(&self, asked: &str) -> Option<Loaded> {
        let path = self.allowed(asked)?;
        self.loads.get(path.to_string_lossy().as_ref()).cloned()
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
        let own = self.own(&path);
        if grammar != run_grammar {
            self.dropped += 1;
            self.refused.insert(asked.to_owned());
            return None;
        }
        // Past the budget is not refused for good: a load taken back frees its place, and a load asked again then
        // is weighed again.
        if !own && self.beyond >= self.budget {
            self.dropped += 1;
            return None;
        }
        let loaded = Loaded { path: path.clone(), module: module.to_owned(), grammar: grammar.to_owned() };
        self.beyond += usize::from(!own);
        self.loads.insert(path, loaded.clone());
        Some(loaded)
    }

    /// Whether `path` is under the corpus root: a file of the run's own tree, which the budget does not bound.
    fn own(&self, path: &str) -> bool {
        self.roots.first().is_some_and(|r| Path::new(path).starts_with(r))
    }

    /// Takes back the document loaded from `path` this run (decision 0108, a load taken back): its place in the
    /// budget is free again. Whether it was loaded.
    pub(super) fn unload(&mut self, path: &str) -> bool {
        let Some(loaded) = self.loads.remove(path) else { return false };
        self.beyond -= usize::from(!self.own(path));
        self.taken.insert(path.to_owned(), loaded);
        true
    }

    /// The document of `asked` taken back this run, loaded again when the budget allows: it is the same document,
    /// with the rows it was given (`unload`).
    pub(super) fn reload(&mut self, asked: &str) -> Option<Loaded> {
        let path = self.allowed(asked)?.to_string_lossy().into_owned();
        if !self.taken.contains_key(&path) {
            return None;
        }
        let own = self.own(&path);
        if !own && self.beyond >= self.budget {
            self.dropped += 1;
            return None;
        }
        let loaded = self.taken.remove(&path)?;
        self.beyond += usize::from(!own);
        self.loads.insert(path, loaded.clone());
        Some(loaded)
    }

    /// What the run's requests consulted, each as (what, path, digest): a directory's entries, a file's lines, a
    /// loaded document's bytes, and for a path refused or missing what is there now (decision 0083 §2). A run whose
    /// consulted things are all as they were would ask, load and derive the same (`still`).
    pub(super) fn consulted(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for (asked, entries) in &self.dirs {
            out.push(("dir".to_owned(), asked.clone(), listing_digest(entries)));
        }
        for asked in self.files.keys() {
            out.push(("file".to_owned(), asked.clone(), self.allowed(asked).map(|p| bytes_digest(&p)).unwrap_or_default()));
        }
        for (asked, d) in &self.digests {
            out.push(("digest".to_owned(), asked.clone(), d.clone()));
        }
        for path in self.loads.keys() {
            out.push(("load".to_owned(), path.clone(), bytes_digest(Path::new(path))));
        }
        for asked in &self.refused {
            out.push(("refused".to_owned(), asked.clone(), self.probe(asked)));
        }
        out
    }

    /// Whether each of `consulted` (`consulted`) is as it was: the requests would be answered alike. Phase 0's
    /// observer answers each by a `stat`, reading only what changed.
    pub(super) fn still(&mut self, consulted: &[(String, String, String)]) -> bool {
        consulted.iter().all(|(what, path, digest)| {
            let now = match what.as_str() {
                "dir" => listing_digest(&self.dir(path)),
                "file" | "digest" => self.allowed(path).map(|p| bytes_digest(&p)).unwrap_or_default(),
                "load" => bytes_digest(Path::new(path)),
                "refused" => self.probe(path),
                _ => return false,
            };
            now == *digest
        })
    }

    /// What a refused path is now: outside the roots or missing, a file, or a directory.
    fn probe(&self, asked: &str) -> String {
        use crate::content_store::observe::Kind;
        match self.allowed(asked).map(|p| super::observe(|o| o.kind(&p))) {
            None | Some(Kind::Missing) => "none",
            Some(Kind::File) => "file",
            Some(Kind::Dir) => "dir",
            Some(Kind::Other) => "other",
        }
        .to_owned()
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

/// A document's path as the content store keys it: a loaded document's canonical path without its root.
pub(super) fn store_path(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn listing_digest(entries: &[(String, bool)]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for (name, is_dir) in entries {
        h.update(name.as_bytes());
        h.update([0, u8::from(*is_dir)]);
    }
    hex(&h.finalize())
}

/// A file's bytes' digest ("" when it cannot be read), from phase 0's observer: read only when its stat changed.
pub(super) fn bytes_digest(path: &Path) -> String {
    super::observe(|o| o.digest(path)).map(|d| hex(&d)).unwrap_or_default()
}
