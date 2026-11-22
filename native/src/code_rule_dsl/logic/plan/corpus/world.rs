//! The file system a program reads and the documents it loads (t21 E1, E2): the generic half of typing against
//! libraries. No rule here knows a language; a language's `env.rerail` and `modules.rerail` say what to read.
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

use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use super::front;
use crate::code_rule_dsl::logic::rr::{FILE_SYSTEM, WORLD};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Documents loaded at most, unless `env("load.budget", n)` says otherwise.
const BUDGET: usize = 4000;
/// The request of a load, and the relation of what was loaded.
pub const LOAD: &str = "load";
pub const LOADED: &str = "loaded";

/// The relations the outputs read, with the rules that decide which keys are requested: a request's answer that is
/// needed makes its request a root too, until no answer adds one (`analyses::needed`, by the version's rules and natives).
pub(super) fn needed(v: &crate::code_rule_dsl::logic::program_image::Version) -> BTreeSet<String> {
    use crate::code_rule_dsl::logic::program_image::{About, RelId, analyses};
    let about = About::of(v);
    let mut roots: BTreeSet<String> = v.output_names().into_iter().collect();
    loop {
        let need = analyses::needed(v, &about, roots.iter().filter_map(|r| v.rel(r)));
        let mut needed: BTreeSet<String> = (0..v.rel_end()).filter(|&i| need[i as usize]).map(|i| v.rel_name(RelId(i)).to_owned()).collect();
        needed.extend(roots.iter().cloned());
        let previous = roots.len();
        roots.extend(WORLD.iter().filter(|(answer, req)| needed.contains(*answer) && !needed.contains(*req)).map(|(_, req)| (*req).to_owned()));
        if roots.len() == previous {
            return needed;
        }
    }
}

/// Whether the program reads the file system or loads documents.
/// `prog` is the program's fields (a version's), `shape` its rules'.
pub fn wanted(prog: &(impl ProgramFields + ?Sized), shape: &super::shape::Shape) -> bool {
    std::iter::once(LOAD).chain(WORLD.iter().map(|(_, req)| *req)).any(|req| asks(prog, shape, req))
}

/// Whether `prog` makes request `req`: by a rule, or as facts (a constant key lowers to one: `dir_entry("sub", …)`
/// asks with the fact `rr_dir_req("sub")`, `rr::lower`'s `world_demand`).
fn asks(prog: &(impl ProgramFields + ?Sized), shape: &super::shape::Shape, req: &str) -> bool {
    shape.made.contains(req) || prog.has_fact_rows(req)
}

/// The requests a program makes, each with what answers it, before a load's per-file rows are added
/// (`corpus::loading`): a directory's entries, a file's lines and a load's `loaded` row.
pub fn answers(f: &crate::code_rule_dsl::logic::plan::fields::Fields<'_>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (req, rel) in WORLD.iter().map(|(rel, req)| (*req, *rel)).chain([(LOAD, LOADED)]) {
        // By a rule, or as facts, as `asks` counts them (a constant key lowers to a request fact). A language's
        // answer gives only the relations the program declares (0327: a program that reads `c_include` makes no
        // `c_define` rows).
        let language = !FILE_SYSTEM.iter().any(|(r, _)| *r == rel) && rel != LOADED;
        if (f.derives(req) || f.has_facts(req)) && (!language || f.arity(rel).is_some()) {
            out.entry(req.to_owned()).or_default().push(rel.to_owned());
        }
    }
    out
}

/// A loaded document: its path as the rule gave it, and the module it is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Loaded {
    pub path: String,
    pub module: String,
    pub(super) grammar: String,
}

#[derive(Default)]
pub struct World {
    git: Option<super::git::Git>,
    inventory_filter: Option<crate::git_workspace::snapshot::InventoryFilter>,
    observations: BTreeMap<(String, String), String>,
    roots: Vec<PathBuf>,
    root: PathBuf,
    dirs: BTreeMap<String, Vec<(String, bool)>>,
    files: BTreeMap<String, (Vec<String>, String)>,
    git_file_storage: BTreeMap<String, String>,
    digests: BTreeMap<String, String>,
    /// The files a language's answer read whole (`bytes`), each with its content digest.
    read: BTreeMap<String, String>,
    loads: BTreeMap<String, Loaded>,
    refused: BTreeSet<String>,
    budget: usize,
    /// The documents loaded from beyond the corpus root (the libraries'), which the budget bounds. A source file of
    /// the run's own tree is always loaded (phase 1, `sources.rerail`).
    beyond: usize,
    /// Loads past the budget, or in another grammar than the run's, not taken.
    pub(super) dropped: usize,
    /// Of those, the loads past the budget: where it binds, what is loaded depends on the order loads are taken in.
    pub over_budget: usize,
    /// Documents loaded this run and taken back since (`unload`): out of the budget, their rows still held where
    /// they were made, so a load asked again takes them back in place (`reload`).
    taken: BTreeMap<String, Loaded>,
}

impl World {
    /// The roots a request may read: the corpus root, and each setting's value that is a directory.
    pub fn new(root: &Path, prog: &(impl ProgramFields + ?Sized)) -> World {
        let (roots, budget) = Self::roots(root, prog);
        // Watched before the first wave (`watch`); asked again here, each is one this run asked already.
        Self::watch_roots(root, &roots);
        World { roots, root: root.to_path_buf(), budget, ..World::default() }
    }

    /// Watches the roots a request of `prog` may read (decision 0105), before the first wave: watching a root reads
    /// its part of the observer's store, which no wave may ask (decision 0104). A `World` made inside the waves then
    /// finds each asked already (Q150: a warm run faulted, "the observer's store not asked inside the waves").
    pub fn watch(root: &Path, prog: &(impl ProgramFields + ?Sized)) {
        Self::watch_roots(root, &Self::roots(root, prog).0);
    }

    /// What may be read, watched (decision 0105): a directory listed or a file read that the journal says is
    /// unchanged is not statted. Not phase 0's root, up front: an empty placeholder (`phase0`); a read under it is
    /// watched as it comes.
    fn watch_roots(root: &Path, roots: &[PathBuf]) {
        let placeholder = super::phase0::active().then(|| dunce::canonicalize(root).ok()).flatten();
        let watched: Vec<PathBuf> = roots.iter().filter(|r| Some(*r) != placeholder.as_ref()).cloned().collect();
        super::observe(|o| o.watch(&watched));
    }

    /// The roots (`new`) and the load budget `prog`'s settings give.
    fn roots(root: &Path, prog: &(impl ProgramFields + ?Sized)) -> (Vec<PathBuf>, usize) {
        let canonical = |p: &Path| crate::git_workspace::snapshot::canonicalize(p).ok();
        let mut roots: Vec<PathBuf> = canonical(root).into_iter().collect();
        let mut budget = BUDGET;
        for row in prog.fact_rows("env") {
            if let [front::Value::Str(k), front::Value::Str(v)] = row.as_slice() {
                if k == "load.budget" {
                    budget = v.parse().unwrap_or(budget);
                }
                if let Some(dir) = canonical(&root.join(v.as_str())).filter(|p| crate::git_workspace::snapshot::is_dir(p)) {
                    roots.push(dir);
                }
            }
        }
        (roots, budget)
    }

    pub fn git(&mut self) -> &super::git::Git {
        self.git.get_or_insert_with(|| super::git::Git::new(&self.root))
    }

    pub fn consulted_lookup(&mut self, operation: &str, domain: &str, value: &str, digest: String) {
        let key = serde_json::to_string(&(domain, value)).expect("string tuple");
        self.observations.insert((operation.to_owned(), key), digest);
    }

    pub fn git_revision(&mut self, operation: &str, key: &str, expression: &str) -> (String, &'static str) {
        let (id, status) = self.git().resolve(operation, key, expression);
        self.consulted_lookup(operation, key, expression, format!("{status}:{id}"));
        (id, status)
    }

    pub fn git_object(&mut self, repo: &str, id: &str) -> Vec<super::git::Record> {
        let records = self.git().records(repo, id);
        let status = records.iter().find(|record| matches!(record.field, "storage" | "error"))
            .map_or("unchecked", |record| record.first.as_str());
        self.consulted_lookup(super::git::OBJECT, repo, id, status.to_owned());
        records
    }

    pub fn inventory_child(&mut self, directory: &str, name: &str, mode: i64, raw: bool) -> Result<Option<String>, String> {
        let is_dir = mode == 16384;
        if !is_dir && !(32768..36864).contains(&mode) {
            return Ok(None);
        }
        if self.inventory_filter.is_none() {
            self.inventory_filter = Some(crate::git_workspace::snapshot::inventory_filter(&self.root)?);
        }
        crate::code_rule_dsl::prof::count("git.path_filters", 1);
        let filter = self.inventory_filter.as_ref().unwrap();
        Ok(if raw { filter.invalid_child(directory, name, is_dir) } else { filter.child(directory, name, is_dir) })
    }

    /// The file `asked` names, when it lies under a root: relative to the corpus root, or absolute.
    /// Resolved by phase 0's observer: where the journal says nothing changed, with no `realpath` (decision 0105).
    /// A load's request is resolved once (`resolve`) and its path handed to `bounded`, `loaded_as`, `reload` and
    /// `load`: each resolution takes the observer's lock and, where the journal cannot vouch, a `realpath`.
    pub fn resolve(&self, asked: &str) -> Option<PathBuf> {
        self.allowed(asked)
    }

    /// Each of `asked`'s paths, resolved as [`World::resolve`] does, their `realpath`s at once on the pool.
    pub fn resolve_all(&self, asked: &[String]) -> Vec<Option<PathBuf>> {
        let paths: Vec<PathBuf> = asked.iter().map(|a| self.root.join(a)).collect();
        let canon = crate::content_store::observe::Observer::canonical_all(|f| super::observe(|o| f(o)), &paths);
        canon.into_iter().map(|c| c.filter(|path| self.roots.iter().any(|r| path.starts_with(r)))).collect()
    }

    fn allowed(&self, asked: &str) -> Option<PathBuf> {
        let path = super::observe(|o| o.canonical(&self.root.join(asked)))?;
        self.roots.iter().any(|r| path.starts_with(r)).then_some(path)
    }

    /// A directory's entries, sorted (none when it is refused or missing).
    pub fn dir(&mut self, asked: &str) -> Vec<(String, bool)> {
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

    /// A Git blob needs a checked read between matching storage observations. An empty stamp
    /// makes an incomplete read ineligible for reuse, even when the tree still names the blob.
    fn read_file(&mut self, asked: &str, path: &Path) -> Option<(Vec<u8>, [u8; 32])> {
        use crate::git_workspace::snapshot;
        if !snapshot::contains(path) {
            return super::observe(|o| o.read(path));
        }
        let before = snapshot::file_stamp(path);
        let result = super::observe(|o| o.read(path));
        let stamp = before.filter(|s| result.is_some() && snapshot::file_stamp(path).as_ref() == Some(s));
        self.git_file_storage.insert(asked.to_owned(), stamp.unwrap_or_default());
        result
    }

    /// A file's lines (none when it is refused or missing).
    pub fn file(&mut self, asked: &str) -> Vec<String> {
        if let Some((lines, _)) = self.files.get(asked) {
            return lines.clone();
        }
        let read = self.allowed(asked).and_then(|p| self.read_file(asked, &p));
        let Some((text, digest)) = read.and_then(|(bytes, digest)| String::from_utf8(bytes).ok().map(|text| (text, digest))) else {
            self.refused.insert(asked.to_owned());
            return Vec::new();
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<String> = text.split_terminator('\n').map(str::to_owned).collect();
        self.files.insert(asked.to_owned(), (lines.clone(), hex(&digest)));
        lines
    }

    /// A file's bytes, for a language's answer (`answerers`); none when it is refused or missing. Consulted as a
    /// file's lines are: by its digest.
    pub fn bytes(&mut self, asked: &str) -> Option<Vec<u8>> {
        let Some((bytes, digest)) = self.allowed(asked).and_then(|p| self.read_file(asked, &p)) else {
            self.refused.insert(asked.to_owned());
            return None;
        };
        self.read.insert(asked.to_owned(), hex(&digest));
        Some(bytes)
    }

    /// A file's content digest (none when it is refused or missing): the observer's, read only where it cannot vouch
    /// for the file.
    pub fn digest(&mut self, asked: &str) -> Option<String> {
        if let Some(d) = self.digests.get(asked) {
            return Some(d.clone());
        }
        let read = self.allowed(asked).filter(|p| crate::git_workspace::snapshot::is_file(p)).and_then(|path| {
            if crate::git_workspace::snapshot::contains(&path) {
                if let Some((_, digest)) = self.files.get(asked) { return Some(digest.clone()); }
                return self.read_file(asked, &path).map(|(_, digest)| hex(&digest));
            }
            Some(bytes_digest(&path)).filter(|digest| !digest.is_empty())
        });
        let Some(d) = read else {
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
    pub fn canonical_corpus<'a>(&self, paths: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
        let paths: Vec<&String> = paths.into_iter().collect();
        let joined: Vec<PathBuf> = paths.iter().map(|p| self.root.join(p)).collect();
        let canon = crate::content_store::observe::Observer::canonical_all(|f| super::observe(|o| f(o)), &joined);
        let mut out = BTreeSet::new();
        for (p, c) in paths.into_iter().zip(canon) {
            if let Some(c) = c {
                out.insert(c.to_string_lossy().into_owned());
            }
            out.insert(p.clone());
        }
        out
    }

    /// The document a load gave earlier in this run, if any, by its request's path (`resolve`): a load asked again
    /// is answered alike.
    pub fn loaded_as(&self, path: Option<&Path>) -> Option<Loaded> {
        self.loads.get(path?.to_string_lossy().as_ref()).cloned()
    }

    /// `path` is `asked` resolved (`resolve`).
    pub fn load(&mut self, asked: &str, path: Option<&Path>, module: &str, grammar: &str, run_grammar: &str, corpus: &BTreeSet<String>) -> Option<Loaded> {
        if corpus.contains(asked) || self.refused.contains(asked) {
            return None;
        }
        let Some(path) = path.filter(|p| crate::git_workspace::snapshot::is_file(p)).map(|p| p.to_string_lossy().into_owned()) else {
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
            self.over_budget += 1;
            return None;
        }
        let identity = if crate::git_workspace::snapshot::contains(&self.root) {
            Path::new(&path).strip_prefix(&self.root).map(super::slashed).unwrap_or_else(|_| super::slashed(Path::new(&path)))
        } else {
            // Spelled with `/` on every OS, as the rules split and join it (`corpus::slashed`).
            super::slashed(Path::new(&path))
        };
        let loaded = Loaded { path: identity, module: module.to_owned(), grammar: grammar.to_owned() };
        self.beyond += usize::from(!own);
        self.loads.insert(path, loaded.clone());
        Some(loaded)
    }

    /// Whether `path` is under the corpus root: a file of the run's own tree, which the budget does not bound.
    fn own(&self, path: &str) -> bool {
        self.roots.first().is_some_and(|r| Path::new(path).starts_with(r))
    }

    /// The documents taken back this run and not loaded again: not the run's documents.
    pub fn taken_paths(&self) -> impl Iterator<Item = &String> {
        self.taken.values().map(|loaded| &loaded.path)
    }

    /// Whether a load is one the budget bounds, by its request's path (`resolve`): a file beyond the corpus root, not
    /// loaded yet this run.
    pub fn bounded(&self, path: Option<&Path>) -> bool {
        path.is_some_and(|p| {
            let p = p.to_string_lossy();
            !self.own(&p) && !self.loads.contains_key(p.as_ref())
        })
    }

    /// Takes back every document loaded from beyond the corpus root this run: a cone that starts again in levels
    /// (decision 0145) asks them again, level by level. How many.
    pub fn take_back_libraries(&mut self) -> usize {
        let paths: Vec<String> = self.loads.keys().filter(|p| !self.own(p)).cloned().collect();
        paths.iter().filter(|p| self.unload(p)).count()
    }

    /// Takes back the document loaded from `path` this run (decision 0145, a load taken back): its place in the
    /// budget is free again. Whether it was loaded.
    pub fn unload(&mut self, path: &str) -> bool {
        let path = self.root.join(path).to_string_lossy().into_owned();
        let Some(loaded) = self.loads.remove(&path) else { return false };
        self.beyond -= usize::from(!self.own(&path));
        self.taken.insert(path, loaded);
        true
    }

    /// The document of a request's path (`resolve`) taken back this run, loaded again when the budget allows: it is
    /// the same document, with the rows it was given (`unload`).
    pub fn reload(&mut self, path: Option<&Path>) -> Option<Loaded> {
        let path = path?.to_string_lossy().into_owned();
        if !self.taken.contains_key(&path) {
            return None;
        }
        let own = self.own(&path);
        if !own && self.beyond >= self.budget {
            self.dropped += 1;
            self.over_budget += 1;
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
    pub fn consulted(&self) -> Vec<(String, String, String)> {
        let mut out: Vec<_> = self.observations.iter().map(|((op, key), value)| (op.clone(), key.clone(), value.clone())).collect();
        for (asked, entries) in &self.dirs {
            out.push(("dir".to_owned(), asked.clone(), listing_digest(entries)));
        }
        for (asked, (_, digest)) in &self.files {
            out.push(("file".to_owned(), asked.clone(), digest.clone()));
        }
        for (asked, digest) in self.read.iter().filter(|(a, _)| !self.files.contains_key(*a)) {
            out.push(("file".to_owned(), asked.clone(), digest.clone()));
        }
        for (asked, stamp) in &self.git_file_storage {
            out.push(("git_file_storage".to_owned(), asked.clone(), stamp.clone()));
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
        self.changed(consulted).is_none()
    }

    /// The first of `consulted` that would be answered otherwise now, with its answer now (`None` for a kind of
    /// request no longer asked).
    pub(super) fn changed<'c>(&mut self, consulted: &'c [(String, String, String)]) -> Option<(&'c (String, String, String), Option<String>)> {
        consulted.iter().find_map(|c| {
            let now = self.observation(&c.0, &c.1);
            (now.as_ref() != Some(&c.2)).then_some((c, now))
        })
    }

    pub(super) fn observation(&mut self, what: &str, path: &str) -> Option<String> {
        Some(match what {
            "dir" => listing_digest(&self.dir(path)),
            "file" | "digest" => self.allowed(path).map(|p| bytes_digest(&p)).unwrap_or_default(),
            "git_file_storage" => crate::git_workspace::snapshot::file_stamp(&self.allowed(path)?)?,
            "load" => bytes_digest(Path::new(path)),
            "refused" => self.probe(path),
            super::analysis_history::REQUEST => {
                let (kind, key) = serde_json::from_str::<(String, String)>(path).ok()?;
                super::analysis_history::requested(&kind, &key).ok()?.1
            }
            super::git::REVISION | super::git::REPO_REVISION | super::git::OBJECT => {
                let (repo, arg) = serde_json::from_str::<(String, String)>(path).ok()?;
                self.git().observation(what, &repo, &arg)?
            }
            _ => return None,
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

    pub fn summary(&self, pauses: usize) -> String {
        let entries: usize = self.dirs.values().map(Vec::len).sum();
        let lines: usize = self.files.values().map(|(lines, _)| lines.len()).sum();
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
pub fn store_path(path: &str) -> &str {
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
pub(in crate::code_rule_dsl::logic::plan) fn bytes_digest(path: &Path) -> String {
    super::observe(|o| o.digest(path)).map(|d| hex(&d)).unwrap_or_default()
}

/// Each file's `bytes_digest`, at once: the observer answers what it holds, and the files it must read are read and
/// hashed on the pool, outside its lock, then recorded. One at a time, a run's first pass over PostHog's 29,885
/// files read and hashed each on the main thread, under the observer's lock (1.9 s on an M3 Max, the pool idle).
pub fn bytes_digests(paths: &[PathBuf]) -> Vec<String> {
    bytes_digests_held(paths, None)
}

/// `bytes_digests`, the bytes read kept in `hold` for the run's builders: the run's one read of each (decision 0300).
pub fn bytes_digests_held(paths: &[PathBuf], hold: Option<&super::texts::Sources>) -> Vec<String> {
    use crate::content_store::observe::{DigestRead, DigestStep};
    use rayon::prelude::*;
    let steps: Vec<DigestStep> = super::observe(|o| paths.iter().map(|p| o.digest_step(p)).collect());
    // A blob several paths share is read and hashed once: its first path's step answers the rest.
    let mut first_by_blob = std::collections::HashMap::new();
    let representatives: Vec<_> = steps.iter().enumerate().map(|(i, step)| match step {
        DigestStep::Read(read) if let Some(key) = read.blob_key() => *first_by_blob.entry(key).or_insert(i),
        _ => i,
    }).collect();
    let hashed: Vec<_> = crate::code_rule_dsl::logic::profile::on_cores(|| {
        steps
            .par_iter()
            .zip(paths.par_iter())
            .enumerate()
            .map(|(i, (s, path))| match s {
                DigestStep::Read(r) if representatives[i] == i => {
                    let bytes = r.read()?;
                    let hashed = DigestRead::hashed(&bytes);
                    let mut git_text = None;
                    if let Some(hold) = hold {
                        if let Some(s) = r.stat() {
                            hold.keep(path, crate::content_store::FileStat { size: s.size, mtime_ns: s.mtime_ns, inode: s.ino }, bytes);
                        } else {
                            git_text = Some(bytes);
                        }
                    }
                    Some((hashed, git_text))
                }
                _ => None,
            })
            .collect()
    });
    if let Some(hold) = hold {
        for (path, &first) in paths.iter().zip(&representatives) {
            if let Some((_, Some(bytes))) = &hashed[first] {
                hold.keep_git(path, bytes);
            }
        }
    }
    super::observe(|o| {
        steps
            .into_iter()
            .zip(representatives)
            .map(|(s, h)| match s {
                DigestStep::Answered(d) => d,
                DigestStep::Read(r) => o.digest_read(r, hashed[h].as_ref().map(|(digest, _)| *digest)),
            })
            .map(|d| d.map(|d| hex(&d)).unwrap_or_default())
            .collect()
    })
}
