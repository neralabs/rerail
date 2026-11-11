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
pub(super) const LOADED_DOC: &str = "loaded_doc";

/// Whether the program reads the file system or loads documents.
pub(super) fn wanted(prog: &front::Program) -> bool {
    prog.rules.iter().any(|r| r.head.rel == LOAD || WORLD.iter().any(|(_, d)| *d == r.head.rel))
}

/// The requests a program makes, each with what answers it, before a load's per-file rows are added
/// (`corpus::loading`): a directory's entries, a file's lines and a load's `loaded` row.
pub(super) fn answers(prog: &front::Program) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<_,_> = WORLD
        .iter()
        .map(|(rel, req)| (*req, *rel))
        .chain([(LOAD, LOADED)])
        .filter(|(req, _)| prog.rules.iter().any(|x| x.head.rel == *req))
        .map(|(req, rel)| (req.to_owned(), vec![rel.to_owned()]))
        .collect();
    // Publish identity rows only when a consumer reads them. The canonical
    // `loaded` contract remains unchanged for existing programs.
    let identities=prog.outputs.iter().any(|r|r==LOADED_DOC) || prog.rules.iter()
        .flat_map(|r|&r.body).filter_map(|l|l.atom()).any(|a|a.rel==LOADED_DOC);
    if identities && let Some(rows)=out.get_mut(LOAD) { rows.push(LOADED_DOC.to_owned()); }
    out
}

pub(super) struct CorpusPaths {
    pub(super) known: BTreeSet<String>,
    pub(super) documents: BTreeMap<String, Vec<i64>>,
}

/// A loaded document: its path as the rule gave it, and the module it is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Loaded {
    pub(super) path: String,
    pub(super) module: String,
    pub(super) grammar: String,
}

/// Each accepted logical request has an answer. Only a fresh physical document needs parsing.
pub(super) struct LoadAnswer {
    pub(super) document: Loaded,
    pub(super) fresh: bool,
}

#[derive(Clone, Debug)]
struct DirectoryEntry {
    name: String,
    is_file: bool,
    is_symlink: bool,
    bytes: i64,
}

#[derive(Default)]
pub(super) struct World {
    roots: Vec<PathBuf>,
    root: PathBuf,
    dirs: BTreeMap<String, Vec<(String, bool)>>,
    metadata: BTreeMap<String, Vec<DirectoryEntry>>,
    snapshot_metadata: bool,
    dir_requests: BTreeSet<String>,
    metadata_requests: BTreeSet<String>,
    files: BTreeMap<String, Vec<String>>,
    file_errors: BTreeMap<String, super::inputs::Failure>,
    read_errors: bool,
    host: BTreeMap<String, Option<String>>,
    loads: BTreeMap<String, Loaded>,
    load_targets: BTreeMap<String, String>,
    refused: BTreeSet<String>,
    budget: usize,
    /// Loads past the budget, or in another grammar than the run's, not taken.
    pub(super) dropped: usize,
}

impl World {
    /// A demanded host value, cached for this execution, including absent values.
    /// Host values are world inputs. Dependency snapshots retain only their digests.
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
        // What the requests may read is watched (decision 0105): a directory listed or a file read that the
        // journal says is unchanged is not statted.
        super::observe(|o| o.watch(&roots));
        let snapshot_metadata = prog.rules.iter().any(|r| r.head.rel == "rr_dir_metadata_req");
        let read_errors = prog.rules.iter().any(|r| r.head.rel == "rr_file_error_req") || prog.facts.contains_key("rr_file_error_req");
        World { roots, root: root.to_path_buf(), budget, snapshot_metadata, read_errors, ..World::default() }
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
        // Adopt Final's phase-0 observer for the common names/type projection.
        let entries = super::observe(|o| o.listing(&path)).map(|l| l.as_ref().clone()).unwrap_or_default();
        self.dirs.insert(asked.to_owned(), entries);
        // Programs consuming both projections observe one snapshot. Directory-only
        // programs retain the observer's cheap names/type listing.
        if self.snapshot_metadata { self.read_metadata(asked); }
    }

    /// A directory's entries, sorted (none when it is refused or missing).
    pub(super) fn dir(&mut self, asked: &str) -> Vec<(String, bool)> {
        self.dir_requests.insert(asked.to_owned());
        self.read_directory(asked);
        self.dirs.get(asked).cloned().unwrap_or_default()
    }

    /// File type, link flag, and followed-target size from that same listing.
    pub(super) fn metadata(&mut self, asked: &str) -> Vec<(String, bool, bool, i64)> {
        self.metadata_requests.insert(asked.to_owned());
        self.read_metadata(asked);
        self.metadata.get(asked).into_iter().flatten().map(|e| (e.name.clone(), e.is_file, e.is_symlink, e.bytes)).collect()
    }

    /// Child size/type can change without changing the parent directory stat.
    /// Refresh one metadata batch per requested directory, only when demanded.
    fn read_metadata(&mut self, asked: &str) {
        if self.metadata.contains_key(asked) { return; }
        self.read_directory(asked);
        if self.metadata.contains_key(asked) { return; }
        let Some(path) = self.allowed(asked) else { return; };
        let names = self.dirs.get(asked).cloned().unwrap_or_default();
        let paths: Vec<_> = names.iter().map(|(name, _)| path.join(name)).collect();
        let stats = super::observe(|o| o.metadata_batch(&paths));
        let entries = names.into_iter().zip(stats).map(|((name, _),(kind,stat,is_symlink))| DirectoryEntry {
            name, is_file: kind == crate::content_store::observe::Kind::File, is_symlink, bytes: stat.size,
        }).collect();
        self.metadata.insert(asked.to_owned(), entries);
    }

    /// A file's lines (none when it is refused or missing).
    pub(super) fn file(&mut self, asked: &str) -> Vec<String> {
        if let Some(lines) = self.files.get(asked) {
            return lines.clone();
        }
        let Some(path) = self.allowed(asked) else {
            self.refused.insert(asked.to_owned());
            self.files.insert(asked.to_owned(), Vec::new());
            return Vec::new();
        };
        // Retain phase-0's shared bytes. Only a failed observer read needs an OS
        // read to recover its error code; successful files are not read twice.
        let result = match super::observe(|o| o.read(&path)) {
            Some((bytes, _)) => super::inputs::decode_repository_bytes(bytes, self.read_errors),
            None => super::inputs::read_repository_file(&self.root, path.to_str().unwrap_or(asked), self.read_errors),
        };
        let text = match result {
            Ok(text) => text,
            Err(error) => {
                self.file_errors.insert(asked.to_owned(), error);
                self.files.insert(asked.to_owned(), Vec::new());
                self.refused.insert(asked.to_owned());
                return Vec::new();
            }
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = text.split_terminator('\n').map(str::to_owned).collect();
        self.files.insert(asked.to_owned(), lines.clone());
        lines
    }

    /// Typed failures share the requested file's read and dependency snapshot.
    pub(super) fn file_error(&mut self, asked: &str) -> Option<Vec<front::Value>> {
        self.file(asked);
        self.file_errors.get(asked).and_then(|error| super::inputs::read_error_row(asked, error))
    }

    /// A load's document: the file's canonical path, when it is new, allowed, in the run's grammar and within the
    /// budget. A file of the corpus is a document already, so its load takes nothing. A loaded document is named by
    /// its canonical path: the same file is one document, and its key and memo entries are the same in every
    /// workspace.
    /// The corpus's documents as canonical paths, as `load` compares them, with their given names too.
    pub(super) fn canonical_corpus<'a>(&self, paths: impl IntoIterator<Item = (i64,&'a String)>, identities: bool) -> CorpusPaths {
        let mut out = CorpusPaths {known:BTreeSet::new(),documents:BTreeMap::new()};
        for (d,p) in paths {
            if let Ok(c) = std::fs::canonicalize(self.root.join(p)) {
                let canonical=c.to_string_lossy().into_owned();
                if identities { out.documents.entry(canonical.clone()).or_default().push(d); }
                out.known.insert(canonical);
            }
            out.known.insert(p.clone());
        }
        out
    }

    pub(super) fn load(&mut self, asked: &str, module: &str, grammar: &str, run_grammar: &str, corpus: &BTreeSet<String>) -> Option<LoadAnswer> {
        if self.refused.contains(asked) {
            return None;
        }
        let Some(path) = self.allowed(asked).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()) else {
            self.dropped += 1;
            self.refused.insert(asked.to_owned());
            return None;
        };
        // Physical work is shared; the answer for a different module or path is still required.
        let fresh = !corpus.contains(asked) && !corpus.contains(&path) && !self.loads.contains_key(&path);
        if grammar != run_grammar || fresh && self.loads.len() >= self.budget {
            self.dropped += 1;
            self.refused.insert(asked.to_owned());
            return None;
        }
        let loaded = Loaded { path: path.clone(), module: module.to_owned(), grammar: grammar.to_owned() };
        self.load_targets.insert(asked.to_owned(), path.clone());
        if fresh { self.loads.insert(path, loaded.clone()); }
        Some(LoadAnswer { document: loaded, fresh })
    }

    /// What the run's requests consulted, each as (what, path, digest): a directory's entries, a file's lines, a
    /// loaded document's bytes, and for a path refused or missing what is there now (decision 0083 §2). A run whose
    /// consulted things are all as they were would ask, load and derive the same (`still`).
    pub(super) fn consulted(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for asked in &self.dir_requests {
            if let Some(entries) = self.dirs.get(asked) {
                out.push(("dir".to_owned(), asked.clone(), listing_digest(entries)));
            }
        }
        for asked in &self.metadata_requests {
            if let Some(entries) = self.metadata.get(asked) {
                out.push(("metadata".to_owned(), asked.clone(), metadata_digest(entries)));
            }
        }
        for (key,value) in &self.host {
            out.push(("host".to_owned(),key.clone(),host_digest(value.as_deref())));
        }
        for asked in self.files.keys() {
            out.push(("file".to_owned(), asked.clone(), self.allowed(asked).map(|p| bytes_digest(&p)).unwrap_or_default()));
        }
        for path in self.loads.keys() {
            out.push(("load".to_owned(), path.clone(), bytes_digest(Path::new(path))));
        }
        for (asked,path) in &self.load_targets {
            out.push(("load_target".to_owned(), asked.clone(), load_target_digest(Path::new(path))));
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
                "metadata" => {
                    self.read_metadata(path);
                    metadata_digest(self.metadata.get(path).map_or(&[][..],Vec::as_slice))
                },
                "host" => host_digest(self.host(path).as_deref()),
                "file" => self.allowed(path).map(|p| bytes_digest(&p)).unwrap_or_default(),
                "load" => bytes_digest(Path::new(path)),
                "load_target" => self.allowed(path).map(|p| load_target_digest(&p)).unwrap_or_default(),
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn listing_digest(entries: &[(String,bool)]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for (name,is_dir) in entries {
        h.update(name.as_bytes());
        h.update([0, u8::from(*is_dir)]);
    }
    hex(&h.finalize())
}

fn metadata_digest(entries: &[DirectoryEntry]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for entry in entries {
        h.update(entry.name.as_bytes());
        h.update([0,u8::from(entry.is_file),u8::from(entry.is_symlink)]);
        h.update(entry.bytes.to_le_bytes());
    }
    hex(&h.finalize())
}

fn host_digest(value: Option<&str>) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update([u8::from(value.is_some())]);
    if let Some(value) = value { h.update(value.as_bytes()); }
    hex(&h.finalize())
}

fn load_target_digest(path: &Path) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(path.as_os_str().as_encoded_bytes());
    h.update([0]);
    h.update(bytes_digest(path).as_bytes());
    hex(&h.finalize())
}

/// A file's bytes' digest ("" when it cannot be read).
pub(super) fn bytes_digest(path: &Path) -> String {
    super::observe(|o| o.digest(path)).map(|d| hex(&d)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_error_and_file_lines_share_one_snapshot_and_keep_error_bytes() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-file-error");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("bad.txt"), [b'a',0xff]).unwrap();
        let program=front::parse(".decl rr_file_error_req(p: symbol)\nrr_file_error_req(\"bad.txt\").").unwrap();
        let mut world=World::new(&root,&program);
        assert!(world.file("bad.txt").is_empty());
        std::fs::write(root.join("bad.txt"),"changed\n").unwrap();
        let row=world.file_error("bad.txt").unwrap();
        assert_eq!((&row[1],&row[4],&row[5],&row[6]),(&front::Value::Str("InvalidUtf8".into()),&front::Value::Str("61ff".into()),&front::Value::Int(1),&front::Value::Int(2)));
        assert!(world.file("bad.txt").is_empty());
        assert!(world.file_error("missing").is_none());
        std::fs::create_dir(root.join("folder")).unwrap();
        let error=world.file_error("folder").unwrap();
        assert!(matches!(error[2],front::Value::Int(n) if n>0));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_logical_load_answers_while_one_physical_document_is_admitted() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-load-alias");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "value = 1\n").unwrap();
        let mut world = World::new(&root, &front::parse("").unwrap());
        let first = world.load("a.py","first","python","python",&BTreeSet::new()).unwrap();
        assert!(first.fresh);
        let second = world.load("a.py","second","python","python",&BTreeSet::new()).unwrap();
        assert!(!second.fresh);
        assert_eq!(first.document.path, second.document.path);
        assert_eq!(second.document.module, "second");
        assert_eq!(world.loads.len(), 1);
        let mut existing = World::new(&root, &front::parse("").unwrap());
        let corpus = existing.canonical_corpus([(7,&"a.py".to_owned())],true);
        assert!(!existing.load("a.py","corpus-alias","python","python",&corpus.known).unwrap().fresh);
        assert_eq!(corpus.documents[&first.document.path],vec![7]);
        assert!(existing.loads.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn consulted_load_targets_reject_retargeted_links_with_identical_bytes() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-load-target");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["a.py","b.py"] { std::fs::write(root.join(name), "value = 1\n").unwrap(); }
        std::os::unix::fs::symlink("a.py",root.join("alias.py")).unwrap();
        let program = front::parse("").unwrap();
        let mut before = World::new(&root,&program);
        assert!(before.load("alias.py","module","python","python",&BTreeSet::new()).is_some());
        let consulted = before.consulted();
        assert!(World::new(&root,&program).still(&consulted));
        std::fs::remove_file(root.join("alias.py")).unwrap();
        std::os::unix::fs::symlink("b.py",root.join("alias.py")).unwrap();
        assert!(!World::new(&root,&program).still(&consulted));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn consulted_metadata_rejects_a_changed_file_size() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-metadata-dependency");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::write(root.join("pkg/a.py"), "abc").unwrap();
        let program = front::parse("").unwrap();
        let mut before = World::new(&root,&program);
        assert_eq!(before.metadata("pkg")[0].3,3);
        let consulted = before.consulted();
        assert!(World::new(&root,&program).still(&consulted));
        std::fs::write(root.join("pkg/a.py"), "changed size").unwrap();
        let changed = World::new(&root,&program).still(&consulted);
        std::fs::remove_dir_all(root).unwrap();
        assert!(!changed,"a metadata consumer must not retain stale byte weights");
    }

    #[test]
    fn consulted_host_inputs_reject_changed_values_and_absence() {
        for (old,new) in [(Some("first"),Some("second")),(None,Some(""))] {
            let key = "env:RERAIL_TEST_CONSULTED_HOST";
            let mut before = World::default();
            before.host_with(key, |_| old.map(str::to_owned));
            let consulted = before.consulted();
            let mut equal = World::default();
            equal.host_with(key, |_| old.map(str::to_owned));
            assert!(equal.still(&consulted));
            let mut changed = World::default();
            changed.host_with(key, |_| new.map(str::to_owned));
            assert!(!changed.still(&consulted),"a demanded host answer changed");
        }
    }

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
    fn directory_only_requests_do_not_stat_each_child() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-listing-only");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "abc").unwrap();
        let program = front::parse(".decl rr_dir_req(path: symbol)\n.decl ask(path: symbol)\nask(\".\").\nrr_dir_req($P) :- ask($P).").unwrap();
        let mut world = World::new(&root, &program);
        assert_eq!(world.dir("."), vec![("a.py".to_owned(), false)]);
        assert!(world.metadata.is_empty(), "a names/type consumer must not construct metadata rows");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_projections_share_one_listing_but_new_runs_refresh_it() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world-directory-cache");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::write(root.join("pkg/a.py"), "abc").unwrap();
        let program = front::parse(".decl rr_dir_metadata_req(path: symbol)\n.decl ask(path: symbol)\nask(\"pkg\").\nrr_dir_metadata_req($P) :- ask($P).").unwrap();
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
