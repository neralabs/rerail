//! Phase 0's observer: what is at a path now, without reading what has not changed (t23).
//!
//! Every path the engine reads goes through here: a rule file and what it includes, a corpus file, a directory a
//! resolution rule lists, a library document a rule loads. The observer answers four questions:
//!
//! - `kind(path)`: missing, a file, a directory, or something else;
//! - `digest(path)`: the sha256 of a file's bytes;
//! - `read(path)`: the bytes and their digest;
//! - `listing(dir)`: a directory's entries, each with whether it is a directory.
//!
//! An answer is reused while the path's stat is the one recorded with it, by one `stat` call, and is otherwise
//! computed again. A stat is dev, inode, size, mtime and ctime, all in nanoseconds; ctime changes on any write, so a
//! write that restores the size and mtime (`touch -r`) is still seen. An answer is trusted only when the path last
//! changed at least [`RACY_NS`] before the answer was recorded (git's racy rule): a file written in the same tick as
//! its recording may have changed after it. A directory's listing is reused while the directory's own stat is the
//! same: adding, removing or renaming an entry changes the directory's mtime.
//!
//! Answers are kept in memory for the process, and in `<content store>.files` across processes (SQLite, written in
//! one transaction by `flush`). The memory may be ahead of the file (decision 0090's lagging writes).
//!
//! **Validation.** A stat can lie: a copy that keeps the mtime, a file system with coarse times. Each process hashes
//! a rotating slice of the files whose stat it trusts (one in [`VALIDATE_EVERY`], by path and a counter the store
//! keeps), and counts a mismatch. `RERAIL_VALIDATE_EVERY=1` checks every file, `0` none.
//!
//! **The journal** (decision 0105). Under a root a run watches (`watch`: the corpus root, the directories its
//! requests may read), the observer does not `stat` a path it has an answer for. It asks the system's journal once
//! per run what changed under the root since the run before (`journal::since`), forgets its answers under each path
//! named, and trusts the rest: an unchanged corpus costs no `stat` of its files. Where the journal cannot say (no
//! earlier run, another volume, events dropped), the root's answers are checked by `stat` as below, and the answers
//! that run did not check are forgotten at its end, so the next run's trust holds.
//!
//! **Git.** Inside a Git work tree, a tracked file that is clean against HEAD and whose stat equals its index entry's
//! holds its committed blob's bytes. Its digest is the blob's, when the store has seen those bytes before
//! (`git_alias`): a fresh clone or another worktree of the same repository is then not hashed.

use crate::content_store::RACY_NS;
use rusqlite::{Connection, params};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One trusted file in this many is hashed again each process (`RERAIL_VALIDATE_EVERY`).
pub const VALIDATE_EVERY: u64 = 64;

/// What is at a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Missing,
    File,
    Dir,
    Other,
}

impl Kind {
    fn code(self) -> i64 {
        match self {
            Kind::Missing => 0,
            Kind::File => 1,
            Kind::Dir => 2,
            Kind::Other => 3,
        }
    }
    fn of_code(code: i64) -> Kind {
        match code {
            1 => Kind::File,
            2 => Kind::Dir,
            3 => Kind::Other,
            _ => Kind::Missing,
        }
    }
}

/// A path's stat, as the observer compares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Stat {
    pub dev: i64,
    pub ino: i64,
    pub size: i64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
}

impl Stat {
    #[cfg(unix)]
    fn of(m: &std::fs::Metadata) -> Stat {
        use std::os::unix::fs::MetadataExt as _;
        let ns = |s: i64, n: i64| s.saturating_mul(1_000_000_000).saturating_add(n);
        Stat { dev: m.dev() as i64, ino: m.ino() as i64, size: m.len() as i64, mtime_ns: ns(m.mtime(), m.mtime_nsec()), ctime_ns: ns(m.ctime(), m.ctime_nsec()) }
    }
    #[cfg(not(unix))]
    fn of(m: &std::fs::Metadata) -> Stat {
        let mtime_ns = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
        Stat { size: m.len() as i64, mtime_ns, ctime_ns: mtime_ns, ..Stat::default() }
    }
}

/// A directory's entries: (name, is a directory), sorted by name.
pub type Listing = Arc<Vec<(String, bool)>>;

/// What the observer knows of a path: its stat when observed, and the answer computed then.
#[derive(Clone, Debug)]
struct Entry {
    kind: Kind,
    stat: Stat,
    /// A file's bytes' sha256, or a directory's listing's.
    digest: [u8; 32],
    listing: Option<Listing>,
    recorded_ns: i64,
}

impl Entry {
    /// Whether the answer still holds for a path whose stat is `now`: the same stat, and recorded safely after the
    /// path last changed.
    fn holds(&self, now: &Stat) -> bool {
        self.stat == *now && self.stat.mtime_ns < self.recorded_ns - RACY_NS && self.stat.ctime_ns < self.recorded_ns - RACY_NS
    }
}

/// What the observer did, for profiles and benchmarks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// `stat` calls.
    pub stats: u64,
    /// Answers reused on a stat.
    pub reused: u64,
    /// Files read and hashed.
    pub hashed: u64,
    /// Bytes hashed.
    pub bytes: u64,
    /// Directories listed by `read_dir`.
    pub listed: u64,
    /// Answers trusted unchanged by the journal, with no `stat`.
    pub trusted: u64,
    /// Answers the journal named as changed, and forgot.
    pub forgotten: u64,
    /// Paths resolved to their canonical form by the file system (`realpath`), not answered by the journal.
    pub resolved: u64,
    /// Trusted files hashed again to check the stat (`VALIDATE_EVERY`).
    pub validated: u64,
    /// Of those, files whose bytes had changed under the same stat.
    pub mismatched: u64,
    /// Digests taken from a clean Git blob's alias, unread.
    pub git: u64,
}

impl Counts {
    pub fn since(&self, before: &Counts) -> Counts {
        Counts {
            stats: self.stats - before.stats,
            trusted: self.trusted - before.trusted,
            forgotten: self.forgotten - before.forgotten,
            resolved: self.resolved - before.resolved,
            reused: self.reused - before.reused,
            hashed: self.hashed - before.hashed,
            bytes: self.bytes - before.bytes,
            listed: self.listed - before.listed,
            validated: self.validated - before.validated,
            mismatched: self.mismatched - before.mismatched,
            git: self.git - before.git,
        }
    }
}

impl std::fmt::Display for Counts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} stats, {} paths resolved, {} trusted by the journal ({} forgotten as changed), {} answers reused, {} files hashed ({} KB), {} directories listed, {} checked again ({} changed under the same stat), {} from Git blobs",
            self.stats,
            self.resolved,
            self.trusted,
            self.forgotten,
            self.reused,
            self.hashed,
            self.bytes / 1024,
            self.listed,
            self.validated,
            self.mismatched,
            self.git
        )
    }
}

/// The observer of one store (or of none: `RERAIL_TAPE_CACHE=off`, answers kept for the process only).
pub struct Observer {
    /// What the store held as the observer opened, and its writer (`Kept`): `None` without a store.
    db: Option<Kept>,
    entries: HashMap<PathBuf, Entry>,
    /// Paths whose entry changed since the last `flush`.
    dirty: Vec<PathBuf>,
    /// Git blob → sha256 of its bytes, seen this process (and in `git_alias`).
    aliases: HashMap<Vec<u8>, [u8; 32]>,
    /// Each path's canonical form (`canonical`), kept as `canonical:<path>` records (`run_record`), and those new
    /// since the last `flush`.
    canon: HashMap<PathBuf, PathBuf>,
    new_canon: Vec<(PathBuf, PathBuf)>,
    new_aliases: Vec<(Vec<u8>, [u8; 32])>,
    epoch: u64,
    validate_every: u64,
    pub counts: Counts,
    /// The working directory, for relative paths: an entry is kept by its absolute path.
    cwd: PathBuf,
    /// The Git work trees this run has discovered (`git`): a clean file's digest may come from its blob.
    git: Vec<(PathBuf, Arc<crate::git_workspace::GitWorkspace>)>,
    /// The roots watched (`watch`), each with whether the journal said what changed under it, and the run it was
    /// asked for (`flushes` then): a root is asked again when a later run watches it. A run inside a run (phase 0's)
    /// leaves the outer run's roots as they are.
    roots: Vec<Root>,
    flushes: u64,
    /// Runs begun and not yet flushed (`begin`, `flush`; a run inside a run counts twice). A root is asked of the
    /// journal only inside a run, and its answer trusted until the run ends: a read outside every run (a rule file
    /// read again by a process that runs nothing between) checks by `stat`, as without a journal.
    open: u32,
    /// The roots the store has a cursor for (`journal`): a path under one is watched as the run first reads it.
    stored_roots: Vec<PathBuf>,
    /// Where the journal's cookies are written: beside the store.
    cookies: Option<PathBuf>,
}

/// A root a run watches.
struct Root {
    path: PathBuf,
    /// The run it was asked of the journal for.
    asked: u64,
    /// Its canonical form: a canonical answer under it is under the root.
    real: PathBuf,
    /// The journal's cursor as it was asked (`mark`): what it names later is after this ask.
    cursor: Option<String>,
    /// The paths resolved this run under a root in stat mode: their canonical answers are kept, the rest dropped.
    resolved: HashSet<PathBuf>,
    /// The journal said what changed under it since the run before: the answers kept under it hold.
    trusted: bool,
    /// The cursor to store at the run's end, once the answers it did not check are forgotten (not trusted only).
    pending: Option<(Option<[u8; 16]>, String)>,
    /// The paths under it this run checked by `stat` or recorded (not trusted only).
    checked: HashSet<PathBuf>,
}

fn now_ns() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64)
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3))
}

fn listing_digest(entries: &[(String, bool)]) -> [u8; 32] {
    let mut h = Sha256::new();
    for (name, is_dir) in entries {
        h.update(name.as_bytes());
        h.update([0, u8::from(*is_dir)]);
    }
    h.finalize().into()
}

fn encode_listing(entries: &[(String, bool)]) -> Option<Vec<u8>> {
    if entries.iter().any(|(n, _)| n.contains('\n')) {
        return None;
    }
    Some(entries.iter().map(|(n, d)| format!("{}{n}", if *d { 'd' } else { 'f' })).collect::<Vec<_>>().join("\n").into_bytes())
}

fn decode_listing(bytes: &[u8]) -> Option<Vec<(String, bool)>> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split('\n').map(|l| Some((l.get(1..)?.to_owned(), l.starts_with('d')))).collect()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS observed(path TEXT PRIMARY KEY, kind INTEGER NOT NULL, dev INTEGER, ino INTEGER, size INTEGER,
  mtime_ns INTEGER, ctime_ns INTEGER, digest BLOB, listing BLOB, recorded_ns INTEGER) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS git_alias(blob BLOB PRIMARY KEY, digest BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS observer_meta(key TEXT PRIMARY KEY, value INTEGER) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS run_record(key TEXT PRIMARY KEY, consulted TEXT NOT NULL, outputs BLOB NOT NULL) WITHOUT ROWID;";

impl Observer {
    /// An observer over the store at `path` (`<path>` is the P0 store itself), or over none. Everything the store
    /// keeps is read here, at once (decision 0104: a run reads before its first wave); what changes is written behind
    /// the run by the observer's writer, and never read back: the observer's memory is ahead of its file.
    fn open(path: Option<&Path>) -> Observer {
        let (mut entries, mut aliases, mut canon) = (HashMap::new(), HashMap::new(), HashMap::new());
        let mut db = path.map(|p| {
            let (records, meta) = read_all(p, &mut entries, &mut aliases, &mut canon);
            Kept { records, meta, writer: Writer::open(p) }
        });
        // Each process checks another slice of the trusted files.
        let epoch = db
            .as_mut()
            .map(|k| {
                let e = match k.meta.get("epoch") {
                    Some(rusqlite::types::Value::Integer(i)) => *i,
                    _ => 0,
                } + 1;
                k.meta.insert("epoch".to_owned(), rusqlite::types::Value::Integer(e));
                k.writer.send(|db| {
                    let _ = db.execute("INSERT INTO observer_meta VALUES ('epoch', 1) ON CONFLICT(key) DO UPDATE SET value = value + 1", []);
                });
                e
            })
            .unwrap_or(0) as u64;
        let validate_every = std::env::var("RERAIL_VALIDATE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(VALIDATE_EVERY);
        let cookies = path.and_then(|p| p.parent()).map(Path::to_path_buf);
        let stored_roots: Vec<PathBuf> = db
            .as_ref()
            .map(|k| k.meta.keys().filter(|k| k.starts_with("journal:")).filter_map(|k| k.splitn(3, ':').nth(2)).map(PathBuf::from).collect())
            .unwrap_or_default();
        Observer { db, entries, dirty: Vec::new(), aliases, new_aliases: Vec::new(), canon, new_canon: Vec::new(), epoch, validate_every, counts: Counts::default(), cwd: std::env::current_dir().unwrap_or_default(), git: Vec::new(), roots: Vec::new(), flushes: 0, open: 0, stored_roots, cookies }
    }

    /// Waits until everything this observer queued is in its file: a process's end, or a test's next observer.
    pub fn written(&self) {
        if let Some(k) = &self.db {
            k.writer.written();
        }
    }

    /// `path` made absolute, as entries are kept.
    fn abs(&self, path: &Path) -> PathBuf {
        if path.is_absolute() { path.to_owned() } else { self.cwd.join(path) }
    }

    /// The entry kept for `path`: everything kept was read as the observer opened (`open`).
    fn known(&mut self, path: &Path) -> Option<Entry> {
        self.entries.get(path).cloned()
    }

    /// Loads every kept entry under `dir` at once: nothing to do, as everything kept was read as the observer opened.
    pub fn preload(&mut self, _dir: &Path) {}

    /// The answer kept for `path` when the journal says nothing changed there since it was recorded (decision 0105):
    /// used as it is. A write after the answer was recorded, even in the same tick, came after the cursor the
    /// recording run took, so the journal names it: neither the racy rule nor a stat check is needed.
    fn trusted(&mut self, path: &Path) -> Option<Entry> {
        self.watch_above(path);
        if !self.roots.iter().any(|r| r.trusted && r.asked == self.flushes && path.starts_with(&r.path)) {
            return None;
        }
        let e = self.known(path)?;
        self.counts.trusted += 1;
        Some(e)
    }

    fn stat(&mut self, path: &Path) -> Option<(Kind, Stat)> {
        if let Some(e) = self.trusted(path) {
            return Some((e.kind, e.stat));
        }
        if self.missing(path) {
            return None;
        }
        if let Some(root) = self.roots.iter_mut().find(|r| !r.trusted && path.starts_with(&r.path)) {
            root.checked.insert(path.to_owned());
        }
        static SHOW: OnceLock<bool> = OnceLock::new();
        if *SHOW.get_or_init(|| std::env::var_os("RERAIL_OBSERVED_PATHS").is_some()) {
            let under = self.roots.iter().find(|r| path.starts_with(&r.path)).map(|r| if r.trusted { "trusted root, no answer" } else { "untrusted root" }).unwrap_or("no root");
            eprintln!("rerail: stat {} ({under})", path.display());
        }
        self.counts.stats += 1;
        let m = std::fs::metadata(path).ok()?;
        let kind = if m.is_file() {
            Kind::File
        } else if m.is_dir() {
            Kind::Dir
        } else {
            Kind::Other
        };
        Some((kind, Stat::of(&m)))
    }

    fn remember(&mut self, path: &Path, entry: Entry) {
        if let Some(root) = self.roots.iter_mut().find(|r| !r.trusted && path.starts_with(&r.path)) {
            root.checked.insert(path.to_owned());
        }
        self.entries.insert(path.to_owned(), entry);
        self.dirty.push(path.to_owned());
    }

    /// What is at `path` now (following symlinks): one `stat`, or none where the journal says nothing changed
    /// (decision 0105): the answer kept, or for a path its directory's kept listing does not hold, missing.
    pub fn kind(&mut self, path: &Path) -> Kind {
        let path = &self.abs(path);
        self.stat(path).map_or(Kind::Missing, |(k, _)| k)
    }

    /// Whether `path` is missing by its directory's kept listing, where the journal says nothing changed (decision
    /// 0105): a name the listing does not hold.
    /// The listing is kept for the next run: a directory probed for a name is listed once, and its listing answers
    /// later probes; a directory that is missing itself is missing by its own directory's listing.
    fn missing(&mut self, path: &Path) -> bool {
        // A path through `.` or `..` is not a name in its directory's listing.
        if !plain(path) {
            return false;
        }
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return false };
        if !self.roots.iter().any(|r| r.trusted && r.asked == self.flushes && dir.starts_with(&r.path)) || self.known(path).is_some() {
            return false;
        }
        let listing = match self.known(dir).filter(|e| e.kind == Kind::Dir).and_then(|e| e.listing) {
            Some(l) => l,
            None if self.missing(dir) => return true,
            None => match self.listing(dir) {
                Some(l) => l,
                None => return false,
            },
        };
        let name = name.to_string_lossy();
        if listing.iter().any(|(n, _)| n.as_str() == name) {
            return false;
        }
        self.counts.trusted += 1;
        true
    }

    /// `path`'s canonical form (`std::fs::canonicalize`: absolute, symlinks and `..` resolved; `None` when nothing is
    /// there). Under a root the journal speaks for (decision 0105), the answer kept is used as it is: a symlink made,
    /// moved or retargeted under the root is an event that forgets it. Elsewhere, one `realpath`, kept for the next run.
    pub fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        let path = self.abs(path);
        self.watch_above(&path);
        let asked = self.flushes;
        let trusted = |p: &Path, roots: &[Root]| roots.iter().any(|r| r.trusted && r.asked == asked && (p.starts_with(&r.path) || p.starts_with(&r.real)));
        if plain(&path) && trusted(&path, &self.roots) {
            // An answer through a symlink is trusted only where its target is watched too.
            if let Some(c) = self.kept_canonical(&path).filter(|c| trusted(c, &self.roots)) {
                self.counts.trusted += 1;
                return Some(c);
            }
            if self.missing(&path) {
                return None;
            }
        }
        if let Some(root) = self.roots.iter_mut().find(|r| !r.trusted && path.starts_with(&r.path)) {
            root.resolved.insert(path.clone());
        }
        self.counts.resolved += 1;
        let c = std::fs::canonicalize(&path).ok()?;
        if self.canon.get(&path) != Some(&c) {
            self.canon.insert(path.clone(), c.clone());
            self.new_canon.push((path, c.clone()));
        }
        Some(c)
    }

    /// The canonical form kept for `path` (read with everything kept, as the observer opened).
    fn kept_canonical(&mut self, path: &Path) -> Option<PathBuf> {
        self.canon.get(path).cloned()
    }

    /// Whether this trusted file is one this process checks again.
    fn checks(&self, path: &Path) -> bool {
        self.validate_every != 0 && (fnv(path.as_os_str().as_encoded_bytes()).wrapping_add(self.epoch)).is_multiple_of(self.validate_every)
    }

    /// The sha256 of the file at `path` (`None`: not a readable file), read only when its stat changed.
    pub fn digest(&mut self, path: &Path) -> Option<[u8; 32]> {
        let path = &self.abs(path);
        if let Some(e) = self.trusted(path).filter(|e| e.kind == Kind::File) {
            self.counts.reused += 1;
            return Some(e.digest);
        }
        let started = now_ns();
        let (kind, now) = self.stat(path)?;
        if kind != Kind::File {
            return None;
        }
        if let Some(e) = self.known(path).filter(|e| e.kind == Kind::File && e.holds(&now)) {
            if !self.checks(path) {
                self.counts.reused += 1;
                return Some(e.digest);
            }
            self.counts.validated += 1;
            let (_, digest) = self.hash(path, now, started)?;
            if digest != e.digest {
                self.counts.mismatched += 1;
            }
            return Some(digest);
        }
        // A clean Git file whose stat is its index entry's holds its blob: the digest of bytes seen before under
        // that blob, unread; or read once, and the blob remembered.
        // Git's work tree root is canonical: so is the path compared with it (one `realpath`, in Git runs only).
        let canonical = if self.git.is_empty() { None } else { std::fs::canonicalize(path).ok() };
        let blob = canonical.as_ref().and_then(|c| {
            self.git.iter().find_map(|(_, g)| {
                let relative = c.strip_prefix(&g.worktree_root).ok()?.to_str()?;
                g.clean_blob_unread(relative)
            })
        });
        if let Some(blob) = blob {
            let key = blob.key_bytes();
            if let Some(d) = self.alias(&key) {
                self.counts.git += 1;
                self.remember(path, Entry { kind: Kind::File, stat: now, digest: d, listing: None, recorded_ns: started });
                return Some(d);
            }
            let (_, d) = self.hash(path, now, started)?;
            self.record_alias(&key, d);
            return Some(d);
        }
        self.hash(path, now, started).map(|(_, d)| d)
    }

    /// Starts a run: the Git work trees are discovered again, as they may have changed since, and each root is
    /// asked of the journal again (`watch`).
    pub fn begin(&mut self) {
        self.git.clear();
        self.open += 1;
    }

    /// Watches `roots` for this run (decision 0105): asks the journal what changed under each since the run that
    /// last watched it, and forgets the answers kept under what it names. The rest are trusted with no `stat` until
    /// the run ends. A root the journal cannot speak for is checked by `stat`, and its unchecked answers forgotten
    /// at the run's end (`flush`). Without a store there is nothing kept across runs to trust.
    pub fn watch(&mut self, roots: &[PathBuf]) {
        // Without the journal, every path is checked by its `stat`, as it is where the journal cannot say.
        if !journal_on() {
            return;
        }
        let (Some(_), Some(cookies)) = (self.db.as_ref(), self.cookies.clone()) else { return };
        if self.open == 0 {
            return;
        }
        for root in roots {
            let root = self.abs(root);
            if self.roots.iter().any(|r| r.asked == self.flushes && root.starts_with(&r.path)) {
                continue;
            }
            // Asked for an earlier run: asked again (its pending cursor, if any, was settled at that run's end).
            self.roots.retain(|r| r.path != root);
            if !self.stored_roots.contains(&root) {
                self.stored_roots.push(root.clone());
            }
            let volume = super::journal::volume(&root);
            // A cursor is of the volume it was taken on: kept under the volume's id with the root.
            let cursor = volume.and_then(|v| self.db.as_ref()?.meta.get(&cursor_key(&root, v)).cloned());
            // A cursor is the journal's own text (an FSEvents id, a Watchman clock); one kept as a number is an id.
            let cursor = cursor.and_then(|c| match c {
                rusqlite::types::Value::Text(t) => Some(t),
                rusqlite::types::Value::Integer(i) => Some(i.to_string()),
                _ => None,
            });
            match super::journal::since(std::slice::from_ref(&root), cursor.as_deref(), &cookies) {
                super::journal::Since::Changed { paths, cursor: next } => {
                    self.forget(&root, &paths, volume, &next);
                    self.roots.push(Root { real: std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: Some(next), path: root, asked: self.flushes, trusted: true, pending: None, checked: Default::default(), resolved: Default::default() });
                }
                super::journal::Since::Lost { cursor, .. } => {
                    let pending = cursor.map(|c| (volume, c));
                    self.roots.push(Root { real: std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: pending.as_ref().map(|(_, c)| c.clone()), path: root, asked: self.flushes, trusted: false, pending, checked: Default::default(), resolved: Default::default() });
                }
            }
        }
    }

    /// Watches `roots` and gives the journal's cursor for each as it was asked: a record made after this, of what is
    /// under them, holds while the journal names no change in what it read since (`changed_since`). `None` where the
    /// journal has no cursor to give.
    pub fn mark(&mut self, roots: &[PathBuf]) -> Option<Vec<String>> {
        self.watch(roots);
        roots
            .iter()
            .map(|root| {
                let root = self.abs(root);
                self.roots.iter().find(|r| r.asked == self.flushes && root.starts_with(&r.path))?.cursor.clone()
            })
            .collect()
    }

    /// Each root asked of the journal this run, with its cursor as it was asked: what a later run's `changed_since`
    /// names under them is what changed after this run read them.
    pub fn watched(&self) -> Vec<(PathBuf, String)> {
        self.roots.iter().filter(|r| r.asked == self.flushes).filter_map(|r| Some((r.path.clone(), r.cursor.clone()?))).collect()
    }

    /// What the journal names as changed under each of `roots` since its cursor in `cursors` (`mark`'s), decision
    /// 0105: `None` where it cannot say.
    pub fn changed_since(&mut self, roots: &[PathBuf], cursors: &[String]) -> Option<Vec<PathBuf>> {
        let cookies = self.cookies.clone().filter(|_| self.db.is_some())?;
        if roots.len() != cursors.len() {
            return None;
        }
        let mut named = Vec::new();
        for (root, cursor) in roots.iter().zip(cursors) {
            match super::journal::since(std::slice::from_ref(root), Some(cursor), &cookies) {
                super::journal::Since::Changed { paths, .. } => named.extend(paths),
                super::journal::Since::Lost { .. } => return None,
            }
        }
        Some(named)
    }

    /// Watches the root above `path` when no root this run watches holds it: the stored root above it (one the
    /// store has a cursor for), else its directory, which a later run then trusts. So every path read is under a
    /// root, and asked of the journal once a run.
    fn watch_above(&mut self, path: &Path) {
        if self.db.is_none() || self.roots.iter().any(|r| r.asked == self.flushes && path.starts_with(&r.path)) {
            return;
        }
        let root = match self.stored_roots.iter().filter(|r| path.starts_with(r)).min_by_key(|r| r.as_os_str().len()) {
            Some(r) => r.clone(),
            None => match path.parent() {
                Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
                _ => return,
            },
        };
        self.watch(std::slice::from_ref(&root));
    }

    /// Forgets the answers kept for each of `paths` and under it, and for its directory (whose listing it is in), in
    /// memory and in the store, with `root`'s cursor moved to `cursor` in the same transaction.
    fn forget(&mut self, root: &Path, paths: &[PathBuf], volume: Option<[u8; 16]>, cursor: &str) {
        // A path named is forgotten with what is under it (a directory moved or removed takes its files); its
        // directory only for its own answer, the listing the path is in.
        let gone: Vec<PathBuf> = paths.to_vec();
        let listings: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        let under = |key: &Path| gone.iter().any(|g| key.starts_with(g)) || listings.iter().any(|l| key == l);
        let before = self.entries.len();
        self.entries.retain(|k, _| !under(k));
        let forgotten = (before - self.entries.len()) as u64;
        self.canon.retain(|k, _| !gone.iter().any(|g| k.starts_with(g)));
        self.new_canon.retain(|(k, _)| !gone.iter().any(|g| k.starts_with(g)));
        if let Some(k) = self.db.as_mut() {
            if let Some(v) = volume {
                k.meta.insert(cursor_key(root, v), rusqlite::types::Value::Text(cursor.to_owned()));
            }
            let (gone, listings, at) = (gone.clone(), listings.clone(), volume.map(|v| (cursor_key(root, v), cursor.to_owned())));
            k.writer.send(move |db| {
                let Ok(tx) = db.transaction() else { return };
                {
                    let Ok(mut del) = tx.prepare_cached("DELETE FROM observed WHERE path = ?1 OR (path >= ?2 AND path < ?3)") else { return };
                    for g in &gone {
                        let at = g.to_string_lossy();
                        let _ = del.execute(params![at, format!("{at}/"), format!("{at}/\u{10ffff}")]);
                    }
                    let Ok(mut canon) = tx.prepare_cached("DELETE FROM run_record WHERE key = ?1 OR (key >= ?2 AND key < ?3)") else { return };
                    for g in &gone {
                        let at = format!("{CANONICAL}{}", g.display());
                        let _ = canon.execute(params![at, format!("{at}/"), format!("{at}/\u{10ffff}")]);
                    }
                    let Ok(mut one) = tx.prepare_cached("DELETE FROM observed WHERE path = ?1") else { return };
                    for l in &listings {
                        let _ = one.execute([l.to_string_lossy()]);
                    }
                    if let Some((key, cursor)) = at {
                        let _ = tx.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
                    }
                }
                let _ = tx.commit();
            });
        }
        self.counts.forgotten += forgotten;
    }

    /// The Git work tree holding `root`, discovered once per run (`None`: not in one, or Git cannot read it).
    pub fn git(&mut self, root: &Path) -> Option<Arc<crate::git_workspace::GitWorkspace>> {
        let root = std::fs::canonicalize(self.abs(root)).ok()?;
        if let Some((_, g)) = self.git.iter().find(|(under, _)| root.starts_with(under)) {
            return Some(Arc::clone(g));
        }
        // Only the part of the work tree under `root`: status over a whole large repository costs seconds.
        let g = Arc::new(crate::git_workspace::discover_under(&root).ok()?);
        self.git.push((root, Arc::clone(&g)));
        Some(g)
    }

    /// The bytes of the file at `path` and their sha256 (`None`: not a readable file). Always reads.
    pub fn read(&mut self, path: &Path) -> Option<(Vec<u8>, [u8; 32])> {
        let path = &self.abs(path);
        let started = now_ns();
        let (kind, now) = self.stat(path)?;
        if kind != Kind::File {
            return None;
        }
        self.hash(path, now, started)
    }

    fn hash(&mut self, path: &Path, stat: Stat, started: i64) -> Option<(Vec<u8>, [u8; 32])> {
        let bytes = std::fs::read(path).ok()?;
        self.counts.hashed += 1;
        self.counts.bytes += bytes.len() as u64;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let changed = self.entries.get(path).is_none_or(|e| e.digest != digest || e.stat != stat || e.kind != Kind::File);
        if changed || self.entries.get(path).is_some_and(|e| !e.holds(&stat)) {
            self.remember(path, Entry { kind: Kind::File, stat, digest, listing: None, recorded_ns: started });
        }
        Some((bytes, digest))
    }

    /// The entries of the directory at `dir` (`None`: not a readable directory), listed only when its stat changed.
    /// An entry is a directory when what it names is one, symlinks followed.
    pub fn listing(&mut self, dir: &Path) -> Option<Listing> {
        let dir = &self.abs(dir);
        if let Some(listing) = self.trusted(dir).filter(|e| e.kind == Kind::Dir).and_then(|e| e.listing) {
            self.counts.reused += 1;
            return Some(listing);
        }
        let started = now_ns();
        let (kind, now) = self.stat(dir)?;
        if kind != Kind::Dir {
            return None;
        }
        if let Some(listing) = self.known(dir).filter(|e| e.kind == Kind::Dir && e.holds(&now)).and_then(|e| e.listing) {
            self.counts.reused += 1;
            return Some(listing);
        }
        self.counts.listed += 1;
        let mut entries: Vec<(String, bool)> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| {
                let is_dir = match e.file_type() {
                    Ok(t) if t.is_symlink() => e.path().is_dir(),
                    Ok(t) => t.is_dir(),
                    Err(_) => false,
                };
                (e.file_name().to_string_lossy().into_owned(), is_dir)
            })
            .collect();
        entries.sort();
        let listing: Listing = Arc::new(entries);
        if encode_listing(&listing).is_some() {
            self.remember(dir, Entry { kind: Kind::Dir, stat: now, digest: listing_digest(&listing), listing: Some(Arc::clone(&listing)), recorded_ns: started });
        }
        Some(listing)
    }

    /// The digest of `dir`'s listing, as `listing` gives it.
    pub fn listing_digest(&mut self, dir: &Path) -> Option<[u8; 32]> {
        self.listing(dir).map(|l| listing_digest(&l))
    }

    /// The digest of a clean Git blob's bytes, when the store has seen them (`git_alias`, read as the observer opened).
    pub fn alias(&mut self, blob: &[u8]) -> Option<[u8; 32]> {
        self.aliases.get(blob).copied()
    }

    /// Records that a Git blob's bytes have `digest`.
    pub fn record_alias(&mut self, blob: &[u8], digest: [u8; 32]) {
        if self.aliases.insert(blob.to_owned(), digest) != Some(digest) {
            self.new_aliases.push((blob.to_owned(), digest));
        }
    }

    /// The record of the last run under `key`: what it consulted, and its outputs (`corpus::reuse`).
    pub fn run_record(&self, key: &str) -> Option<(String, Vec<u8>)> {
        self.db.as_ref()?.records.get(key).cloned()
    }

    /// Keeps a run's record under `key`, replacing the last: in memory now, in the store behind the run.
    pub fn put_run_record(&mut self, key: &str, consulted: &str, outputs: &[u8]) {
        if let Some(k) = self.db.as_mut() {
            k.records.insert(key.to_owned(), (consulted.to_owned(), outputs.to_owned()));
            let (key, consulted, outputs) = (key.to_owned(), consulted.to_owned(), outputs.to_owned());
            k.writer.send(move |db| {
                let _ = db.execute("INSERT OR REPLACE INTO run_record VALUES (?1, ?2, ?3)", params![key, consulted, outputs]);
            });
        }
    }

    /// Writes what changed since the last flush to the store, in one transaction.
    pub fn flush(&mut self) {
        self.write_dirty();
        self.settle_roots();
        self.flushes += 1;
        self.open = self.open.saturating_sub(1);
    }

    fn write_dirty(&mut self) {
        let Some(k) = self.db.as_ref() else {
            self.dirty.clear();
            self.new_aliases.clear();
            self.new_canon.clear();
            return;
        };
        if self.dirty.is_empty() && self.new_aliases.is_empty() && self.new_canon.is_empty() {
            return;
        }
        self.dirty.sort();
        self.dirty.dedup();
        type Row = (String, i64, Stat, Vec<u8>, Option<Vec<u8>>, i64);
        let rows: Vec<Row> = self
            .dirty
            .iter()
            .filter_map(|path| {
                let e = self.entries.get(path)?;
                Some((path.to_string_lossy().into_owned(), e.kind.code(), e.stat, e.digest.to_vec(), e.listing.as_ref().and_then(|l| encode_listing(l)), e.recorded_ns))
            })
            .collect();
        let aliases = std::mem::take(&mut self.new_aliases);
        let canon: Vec<(String, String)> = std::mem::take(&mut self.new_canon).into_iter().map(|(p, c)| (format!("{CANONICAL}{}", p.display()), c.to_string_lossy().into_owned())).collect();
        self.dirty.clear();
        k.writer.send(move |db| {
            let Ok(tx) = db.transaction() else { return };
            {
                let Ok(mut put) = tx.prepare_cached("INSERT OR REPLACE INTO observed VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)") else { return };
                for (path, kind, s, digest, listing, recorded) in &rows {
                    let _ = put.execute(params![path, kind, s.dev, s.ino, s.size, s.mtime_ns, s.ctime_ns, digest, listing, recorded]);
                }
                let Ok(mut alias) = tx.prepare_cached("INSERT OR REPLACE INTO git_alias VALUES (?1, ?2)") else { return };
                for (blob, d) in &aliases {
                    let _ = alias.execute(params![blob, &d[..]]);
                }
                let Ok(mut put_canon) = tx.prepare_cached("INSERT OR REPLACE INTO run_record VALUES (?1, ?2, x'')") else { return };
                for (key, c) in &canon {
                    let _ = put_canon.execute(params![key, c]);
                }
            }
            let _ = tx.commit();
        });
    }
}

impl Observer {
    /// For each root the journal could not speak for this run: forgets the answers under it the run did not check,
    /// in memory and in the store, and stores the cursor taken as the run began, in one transaction. Every answer
    /// left under the root then held as of that cursor, so the next run may trust the journal from it.
    fn settle_roots(&mut self) {
        type Settled = (PathBuf, Option<[u8; 16]>, String, HashSet<PathBuf>, HashSet<PathBuf>);
        let roots: Vec<Settled> = self
            .roots
            .iter_mut()
            .filter_map(|r| r.pending.take().map(|(v, c)| (r.path.clone(), v, c, std::mem::take(&mut r.checked), std::mem::take(&mut r.resolved))))
            .collect();
        if roots.is_empty() {
            return;
        }
        // What is still to be written was checked (it is this run's).
        self.write_dirty();
        for (root, volume, cursor, checked, resolved) in roots {
            // Everything kept was read as the observer opened: what memory drops here is what the store drops.
            let paths: Vec<String> = self.entries.keys().filter(|k| k.starts_with(&root) && !checked.contains(*k)).map(|k| k.to_string_lossy().into_owned()).collect();
            let keys: Vec<String> = self.canon.keys().filter(|k| k.starts_with(&root) && !resolved.contains(*k)).map(|k| format!("{CANONICAL}{}", k.display())).collect();
            self.entries.retain(|k, _| !k.starts_with(&root) || checked.contains(k));
            self.canon.retain(|k, _| !k.starts_with(&root) || resolved.contains(k));
            let Some(k) = self.db.as_mut() else { continue };
            let at = volume.map(|v| (cursor_key(&root, v), cursor));
            if let Some((key, cursor)) = &at {
                k.meta.insert(key.clone(), rusqlite::types::Value::Text(cursor.clone()));
            }
            k.writer.send(move |db| {
                let Ok(tx) = db.transaction() else { return };
                {
                    let Ok(mut del) = tx.prepare_cached("DELETE FROM observed WHERE path = ?1") else { return };
                    for p in &paths {
                        let _ = del.execute([p]);
                    }
                    let Ok(mut drop_key) = tx.prepare_cached("DELETE FROM run_record WHERE key = ?1") else { return };
                    for key in &keys {
                        let _ = drop_key.execute([key]);
                    }
                    if let Some((key, cursor)) = at {
                        let _ = tx.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
                    }
                }
                let _ = tx.commit();
            });
        }
    }
}

/// What the store keeps of an observer beside its entries, aliases and canonical forms, as it opened: the run records
/// and the observer's own settings (the epoch, each root's journal cursor); and the writer of what changes.
struct Kept {
    records: HashMap<String, (String, Vec<u8>)>,
    meta: HashMap<String, rusqlite::types::Value>,
    writer: Writer,
}

/// Reads everything the store at `path` keeps, at once: its entries, aliases and canonical forms into the maps given,
/// its run records and settings returned. A store not made yet holds nothing.
type Records = HashMap<String, (String, Vec<u8>)>;
type Meta = HashMap<String, rusqlite::types::Value>;
fn read_all(path: &Path, entries: &mut HashMap<PathBuf, Entry>, aliases: &mut HashMap<Vec<u8>, [u8; 32]>, canon: &mut HashMap<PathBuf, PathBuf>) -> (Records, Meta) {
    let (mut records, mut meta) = (HashMap::new(), HashMap::new());
    let Ok(db) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return (records, meta) };
    if let Ok(mut st) = db.prepare("SELECT path, kind, dev, ino, size, mtime_ns, ctime_ns, digest, listing, recorded_ns FROM observed")
        && let Ok(rows) = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, [r.get::<_, i64>(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?], r.get::<_, Option<Vec<u8>>>(7)?, r.get::<_, Option<Vec<u8>>>(8)?, r.get::<_, i64>(9)?))
        })
    {
        for (path, kind, [dev, ino, size, mtime_ns, ctime_ns], digest, listing, recorded_ns) in rows.flatten() {
            let listing = match listing.map(|b| decode_listing(&b)) {
                Some(None) => continue,
                Some(Some(l)) => Some(Arc::new(l)),
                None => None,
            };
            entries.insert(PathBuf::from(path), Entry { kind: Kind::of_code(kind), stat: Stat { dev, ino, size, mtime_ns, ctime_ns }, digest: digest.and_then(|d| d.try_into().ok()).unwrap_or_default(), listing, recorded_ns });
        }
    }
    if let Ok(mut st) = db.prepare("SELECT blob, digest FROM git_alias")
        && let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))
    {
        for (blob, d) in rows.flatten() {
            if let Ok(d) = d.try_into() {
                aliases.insert(blob, d);
            }
        }
    }
    if let Ok(mut st) = db.prepare("SELECT key, consulted, outputs FROM run_record")
        && let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?)))
    {
        for (key, consulted, outputs) in rows.flatten() {
            match key.strip_prefix(CANONICAL) {
                Some(p) => {
                    canon.insert(PathBuf::from(p), PathBuf::from(consulted));
                }
                None => {
                    records.insert(key, (consulted, outputs));
                }
            }
        }
    }
    if let Ok(mut st) = db.prepare("SELECT key, value FROM observer_meta")
        && let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, rusqlite::types::Value>(1)?)))
    {
        meta.extend(rows.flatten());
    }
    (records, meta)
}

/// An observer's writer: a thread of its own with its own connection, behind the run (decision 0104), running each
/// write in the order it was queued. Dropped, it finishes what was queued.
struct Writer {
    jobs: Option<std::sync::mpsc::Sender<Box<dyn FnOnce(&mut Connection) + Send>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    fn open(path: &Path) -> Writer {
        let (jobs, queued) = std::sync::mpsc::channel::<Box<dyn FnOnce(&mut Connection) + Send>>();
        let path = path.to_owned();
        let thread = rerail_accel::exec::service("rerail-observer-writer", None, move || {
                super::hazards::behind_the_run();
                let db = (|| {
                    let db = Connection::open(&path).ok()?;
                    db.busy_timeout(std::time::Duration::from_secs(10)).ok()?;
                    db.execute_batch("PRAGMA journal_mode=WAL;").ok()?;
                    db.execute_batch(SCHEMA).ok()?;
                    Some(db)
                })();
                let Some(mut db) = db else {
                    // No store to write: what is queued is dropped (an ask for `written` too, which then returns).
                    for _ in queued {}
                    return;
                };
                for job in queued {
                    job(&mut db);
                }
            })
            .ok();
        Writer { jobs: Some(jobs), thread }
    }

    fn send(&self, job: impl FnOnce(&mut Connection) + Send + 'static) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Box::new(job));
        }
    }

    /// Waits until everything queued before is written.
    fn written(&self) {
        let (done, wait) = std::sync::mpsc::channel();
        self.send(move |_| {
            let _ = done.send(());
        });
        let _ = wait.recv();
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The prefix of a path's canonical form's record (`run_record`).
const CANONICAL: &str = "canonical:";

/// Whether `path` has no `.` or `..` component: its last name is a name in its directory's listing.
fn plain(path: &Path) -> bool {
    !path.components().any(|c| matches!(c, std::path::Component::CurDir | std::path::Component::ParentDir))
}

/// Where a root's journal cursor is kept (`observer_meta`): `journal:<volume>:<root>`.
fn cursor_key(root: &Path, volume: [u8; 16]) -> String {
    format!("journal:{}:{}", volume.iter().map(|b| format!("{b:02x}")).collect::<String>(), root.display())
}

/// The P0 store beside a content store: `<content store>.files`.
pub fn store_beside(content_store: &Path) -> PathBuf {
    let mut name = content_store.as_os_str().to_owned();
    name.push(".files");
    PathBuf::from(name)
}

static OBSERVERS: OnceLock<Mutex<HashMap<Option<PathBuf>, Observer>>> = OnceLock::new();

/// Waits until every observer's queued writes are in its store: the run's end, beside the content store's writer.
pub fn written() {
    if let Some(all) = OBSERVERS.get() {
        all.lock().unwrap_or_else(|e| e.into_inner()).values().for_each(Observer::written);
    }
}

/// Drops the observer of the P0 store at `store`, its queued writes written first: its store is being deleted (a
/// test's own). Every run's end waits on each observer the process holds (`written`), so one kept past its store's
/// life is a cost to every later run.
pub fn forget(store: &Path) {
    let gone = OBSERVERS.get().and_then(|all| all.lock().unwrap_or_else(|e| e.into_inner()).remove(&Some(store.to_path_buf())));
    if let Some(observer) = gone {
        observer.written();
    }
}

/// Runs `f` with the observer of the P0 store at `store` (`None`: in memory, for the process). One observer per
/// store, for the process.
pub fn with<T>(store: Option<&Path>, f: impl FnOnce(&mut Observer) -> T) -> T {
    let all = OBSERVERS.get_or_init(Default::default);
    let mut all = all.lock().unwrap_or_else(|e| e.into_inner());
    let key = store.map(Path::to_path_buf);
    let observer = all.entry(key.clone()).or_insert_with(|| Observer::open(key.as_deref()));
    f(observer)
}

/// A hex digest, as records keep them.
pub fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the observer asks the file system's journal (decision 0105). On unless `RERAIL_JOURNAL=off`, or turned off
/// by `set_journal`: the engine's tests do (each run of a test waits for its cookie's event, up to `journal`'s `WAIT`, and a test
/// makes many runs), and the journal's own tests here keep it on.
static JOURNAL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn journal_on() -> bool {
    use std::sync::atomic::Ordering;
    match JOURNAL.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let on = !std::env::var("RERAIL_JOURNAL").is_ok_and(|v| v == "off");
            JOURNAL.store(if on { 1 } else { 2 }, Ordering::Relaxed);
            on
        }
    }
}

/// Turns the observer's use of the journal on or off for the process, whatever `RERAIL_JOURNAL` says.
pub fn set_journal(on: bool) {
    JOURNAL.store(if on { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 0105: a run that watches a root does no `stat` of what the journal says is unchanged. A first run
    /// checks every path; the next, with nothing changed, none, and its answers are the same; after an edit, the
    /// edited file is read again and the others are trusted. Each run is a new process's observer.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_watched_root_is_not_statted_where_the_journal_says_nothing_changed() {
        let base = std::env::temp_dir().join(format!("rerail-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        let files: Vec<PathBuf> = (0..5).map(|i| root.join(format!("pkg/m{i}.py"))).collect();
        for f in &files {
            std::fs::write(f, "x = 1\n").unwrap();
            aged(f);
        }
        let store = base.join("store.files");
        let run = |files: &[PathBuf]| {
            let mut o = Observer::open(Some(&store));
            o.begin();
            o.watch(std::slice::from_ref(&root));
            let listing = o.listing(&root.join("pkg")).unwrap();
            let digests: Vec<[u8; 32]> = files.iter().map(|f| o.digest(f).unwrap()).collect();
            o.flush();
            (o.counts, listing, digests)
        };
        let (first, listing, digests) = run(&files);
        assert!(first.stats >= 6 && first.trusted == 0, "the first run checks every path: {first}");
        // The setup's writes may be named once more (their events may come after the first run's cursor).
        std::thread::sleep(std::time::Duration::from_millis(300));
        run(&files);
        let (second, again, same) = run(&files);
        assert_eq!((second.stats, second.trusted), (0, 6), "nothing changed: no stat ({second})");
        assert_eq!((again, same), (listing, digests.clone()));
        std::fs::write(&files[2], "x = 2\n").unwrap();
        aged(&files[2]);
        let (third, _, after) = run(&files);
        assert!(third.forgotten >= 1 && third.hashed == 1, "the edited file is read again: {third}");
        assert_ne!(after[2], digests[2]);
        assert_eq!([after[0], after[1], after[3], after[4]], [digests[0], digests[1], digests[3], digests[4]]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A path's canonical form is resolved once: under a root the journal says is unchanged, the next run answers it
    /// with no `realpath`, and a symlink retargeted under the root is resolved again.
    #[test]
    fn a_canonical_path_is_resolved_again_only_where_the_journal_names_a_change() {
        let base = std::env::temp_dir().join(format!("rerail-canonical-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        for i in 0..3 {
            std::fs::write(root.join(format!("pkg/m{i}.py")), "x = 1\n").unwrap();
        }
        let link = root.join("pkg/alias.py");
        std::os::unix::fs::symlink("m0.py", &link).unwrap();
        let store = base.join("store.files");
        let asked = [root.join("pkg/m1.py"), link.clone(), root.join("pkg/../pkg/m2.py")];
        let run = || {
            let mut o = Observer::open(Some(&store));
            o.begin();
            o.watch(std::slice::from_ref(&root));
            let got: Vec<Option<PathBuf>> = asked.iter().map(|p| o.canonical(p)).collect();
            o.flush();
            (o.counts, got)
        };
        let (first, got) = run();
        assert_eq!(first.resolved, 3, "{first}");
        let real = std::fs::canonicalize(&root).unwrap();
        assert_eq!(got, vec![Some(real.join("pkg/m1.py")), Some(real.join("pkg/m0.py")), Some(real.join("pkg/m2.py"))]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        run();
        let (second, same) = run();
        // The path through `..` is resolved each time: it is not a name its directory's listing holds.
        assert_eq!((second.resolved, second.trusted), (1, 2), "nothing changed: {second}");
        assert_eq!(same, got);
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("m2.py", &link).unwrap();
        let (third, after) = run();
        assert!(third.resolved >= 2, "the retargeted symlink is resolved again: {third}");
        assert_eq!(after[1], Some(real.join("pkg/m2.py")));
        let _ = std::fs::remove_dir_all(&base);
    }

    fn aged(path: &Path) {
        // Old enough to be trusted (past the racy window).
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let f = std::fs::File::options().write(true).open(path).or_else(|_| std::fs::File::open(path)).unwrap();
        f.set_modified(old).unwrap();
    }

    /// A file's digest is reused on its stat, and computed again when it changes; the store keeps it across
    /// observers (processes).
    #[test]
    fn a_files_digest_is_reused_while_its_stat_holds() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("p0.files");
        let file = dir.path().join("a.py");
        std::fs::write(&file, "x = 1\n").unwrap();
        aged(&file);
        // The setup's events delivered before the first run's cursor (decision 0105): a later one names the file, read
        // again (safe, and not what this test is of).
        std::thread::sleep(std::time::Duration::from_millis(300));
        // ctime is not settable: wait out the racy window by recording later than it.
        let mut first = Observer::open(Some(&store));
        first.validate_every = 0;
        let d1 = first.digest(&file).unwrap();
        // Recorded with a time after ctime + RACY_NS, as a later run would.
        for e in first.entries.values_mut() {
            e.recorded_ns += 2 * RACY_NS;
        }
        first.dirty = first.entries.keys().cloned().collect();
        first.flush();
        first.written();
        let mut second = Observer::open(Some(&store));
        second.validate_every = 0;
        assert_eq!(second.digest(&file), Some(d1));
        assert_eq!((second.counts.hashed, second.counts.reused), (0, 1), "unchanged: one stat, no read");
        std::fs::write(&file, "x = 2\n").unwrap();
        // The next run (a run's answers are as its journal found them, decision 0105).
        second.flush();
        let d2 = second.digest(&file).unwrap();
        assert_ne!(d1, d2);
        assert_eq!(second.counts.hashed, 1, "changed: read again");
    }

    /// A file written just before its recording is not trusted: it may have changed in the same tick.
    #[test]
    fn a_racy_file_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.py");
        std::fs::write(&file, "x = 1\n").unwrap();
        let mut o = Observer::open(None);
        o.digest(&file).unwrap();
        o.digest(&file).unwrap();
        assert_eq!(o.counts.hashed, 2, "inside the racy window every observation reads");
    }

    /// A rewrite that keeps size and mtime is seen by ctime; a validation pass sees what a lying stat hides.
    #[test]
    fn validation_catches_bytes_changed_under_a_kept_stat() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.py");
        std::fs::write(&file, "x = 1\n").unwrap();
        let mut o = Observer::open(None);
        o.validate_every = 1;
        let d1 = o.digest(&file).unwrap();
        // Pretend the stat is trusted (recorded long after), then change the bytes behind the recorded stat.
        let e = o.entries.get_mut(&file).unwrap();
        e.recorded_ns += 10 * RACY_NS;
        e.digest = [7; 32];
        let d = o.digest(&file).unwrap();
        assert_eq!(d, d1);
        assert_eq!((o.counts.validated, o.counts.mismatched), (1, 1));
    }

    /// A directory's listing is reused while its stat holds, and listed again when an entry is added.
    #[test]
    fn a_listing_is_reused_until_an_entry_is_added() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("pkg");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("a.py"), "").unwrap();
        std::fs::create_dir(sub.join("inner")).unwrap();
        let mut o = Observer::open(None);
        let l = o.listing(&sub).unwrap();
        assert_eq!(*l, vec![("a.py".to_owned(), false), ("inner".to_owned(), true)]);
        o.entries.get_mut(&sub).unwrap().recorded_ns += 10 * RACY_NS;
        o.listing(&sub).unwrap();
        assert_eq!((o.counts.listed, o.counts.reused), (1, 1));
        // A new entry changes the directory's mtime and ctime.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(sub.join("b.py"), "").unwrap();
        let l = o.listing(&sub).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(o.counts.listed, 2);
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn age_all(dir: &Path) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.file_name() == ".git" {
                continue;
            }
            if e.path().is_dir() {
                age_all(&e.path());
            } else {
                aged(&e.path());
            }
        }
    }

    /// Git's view: the files it sees (ignored ones left out, untracked ones in, deleted ones out), and a clean
    /// file's digest from its blob in another clone, unread.
    #[test]
    fn git_lists_the_worktree_and_a_clone_reads_no_clean_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("pkg")).unwrap();
        git(&repo, &["init", "--initial-branch=main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.py"), "x = 1\n").unwrap();
        std::fs::write(repo.join("pkg/b.rr"), "r(1).\n").unwrap();
        std::fs::write(repo.join("gone.py"), "").unwrap();
        std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
        git(&repo, &["add", "--", "a.py", "pkg/b.rr", "gone.py", ".gitignore"]);
        git(&repo, &["commit", "-m", "one"]);
        std::fs::remove_file(repo.join("gone.py")).unwrap();
        std::fs::write(repo.join("new.py"), "").unwrap();
        std::fs::write(repo.join("noise.log"), "").unwrap();
        let g = crate::git_workspace::discover(&repo).unwrap();
        assert_eq!(g.worktree_files(), vec![".gitignore", "a.py", "new.py", "pkg/b.rr"]);

        // A clone whose index stats are settled (mtimes before the index): its clean files are proven by stat.
        let clone = dir.path().join("clone");
        git(dir.path(), &["clone", "-q", repo.to_str().unwrap(), clone.to_str().unwrap()]);
        age_all(&repo);
        age_all(&clone);
        // The index is written after a later tick than the files' ctimes: its entries are not racy.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        git(&repo, &["update-index", "--ignore-missing", "--refresh"]);
        git(&clone, &["update-index", "--refresh"]);
        for root in [&repo, &clone] {
            assert!(crate::git_workspace::discover(root).unwrap().clean_blob_unread("a.py").is_some(), "{}: a.py proven clean by its stat", root.display());
        }
        let store = dir.path().join("p0.files");
        let mut o = Observer::open(Some(&store));
        o.validate_every = 0;
        o.git(&repo).unwrap();
        let d = o.digest(&repo.join("a.py")).unwrap();
        assert_eq!((o.counts.hashed, o.counts.git), (1, 0), "first sight of the blob: read once, remembered");
        o.flush();
        o.written();
        let mut fresh = Observer::open(Some(&store));
        fresh.validate_every = 0;
        fresh.git(&clone).unwrap();
        assert_eq!(fresh.digest(&clone.join("a.py")), Some(d));
        assert_eq!((fresh.counts.hashed, fresh.counts.git), (0, 1), "the clone's clean a.py: its blob's digest, unread");
        // An edited file is not clean: hashed.
        std::fs::write(clone.join("a.py"), "x = 2\n").unwrap();
        fresh.begin();
        fresh.git(&clone).unwrap();
        assert_ne!(fresh.digest(&clone.join("a.py")), Some(d));
        assert_eq!(fresh.counts.hashed, 1);
    }

    #[test]
    fn listings_round_trip_through_the_store_encoding() {
        let l = vec![("a".to_owned(), true), ("b.py".to_owned(), false)];
        assert_eq!(decode_listing(&encode_listing(&l).unwrap()).unwrap(), l);
        assert_eq!(decode_listing(&encode_listing(&[]).unwrap()).unwrap(), Vec::<(String, bool)>::new());
        assert!(encode_listing(&[("a\nb".to_owned(), false)]).is_none());
    }
}
