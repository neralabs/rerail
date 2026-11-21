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

#[allow(unused_imports)]
use crate::git_workspace::snapshot as source;
use crate::content_store::connection::Cached as _;
use crate::content_store::RACY_NS;
use rusqlite::{Connection, params};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod listing;

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
        self.stat == *now && !self.racy()
    }

    /// Whether it was recorded inside the racy window: the path changed less than [`RACY_NS`] before.
    fn racy(&self) -> bool {
        self.stat.mtime_ns >= self.recorded_ns - RACY_NS || self.stat.ctime_ns >= self.recorded_ns - RACY_NS
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
    /// Directories listed to check a small root's answers (decision 0400), and the answers they named as changed.
    pub checked_dirs: u64,
    pub checked_named: u64,
    /// Roots the journal answered later than their wait (`journal::late`), checked by `stat` instead, or whose replay's
    /// history did not hold the run's marker in its settle window (`Since::Behind`), checked by listings: in both the
    /// journal was behind the host's file events (Q223).
    pub late: u64,
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
            checked_dirs: self.checked_dirs - before.checked_dirs,
            checked_named: self.checked_named - before.checked_named,
            late: self.late - before.late,
        }
    }
}

impl std::fmt::Display for Counts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} stats, {} paths resolved, {} trusted by the journal ({} forgotten as changed), {} answers reused, {} files hashed ({} KB), {} directories listed, {} checked again ({} changed under the same stat), {} from Git blobs, {} directories listed to check a root ({} answers named), {} roots the journal answered late",
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
            self.git,
            self.checked_dirs,
            self.checked_named,
            self.late
        )
    }
}

/// The observer of one store (or of none: `RERAIL_TAPE_CACHE=off`, answers kept for the process only).
pub struct Observer {
    /// What the store held as the observer opened, and its writer (`Kept`): `None` without a store.
    db: Option<Kept>,
    /// Sorted by path, so what is under a path is one range (`forget`).
    entries: BTreeMap<PathBuf, Entry>,
    /// Paths whose entry changed since the last `flush`.
    dirty: Vec<PathBuf>,
    /// Paths recorded inside the racy window (`Entry::holds`) since the outermost run's last end: read again then,
    /// once the window has passed (`settle_racy`).
    racy: Vec<PathBuf>,
    /// Git blob → sha256 of its bytes, seen this process (and in `git_alias`).
    aliases: HashMap<Vec<u8>, [u8; 32]>,
    /// Each path's canonical form (`canonical`), kept as `canonical:<path>` records (`run_record`), and those new
    /// since the last `flush`.
    canon: BTreeMap<PathBuf, PathBuf>,
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
    /// Each root's volume as the system named it (`journal::volume`), asked once a root the observer watches: a
    /// cursor is of its volume. With the roots, the observer's own state.
    volumes: Vec<(PathBuf, Option<[u8; 16]>)>,
    /// Where the journal's cookies are written: beside the store.
    cookies: Option<PathBuf>,
    /// rerail's own files this observer knows of (decision 0284): its content store (and so every file named after
    /// it), and the directories runs wrote their outputs into (`own_output`, kept as `own:<dir>` in its store). With
    /// the process's (`own`).
    own: Vec<PathBuf>,
    /// Output directories named this run under no root the store has a cursor for yet (`own_output`).
    unreported: Vec<PathBuf>,
    /// How long after its recording an answer is checked by `stat` though the journal names nothing there
    /// ([`JOURNAL_SETTLE_NS`]); a test that asks for trust at once sets it to 0.
    settle_ns: i64,
    /// The least the journal is waited for ([`JOURNAL_WAIT_LEAST_NS`]); a test that asks for the journal's answer
    /// whatever it costs sets it high.
    wait_least_ns: u64,
    /// The observer's own journal of the roots checked by their listings (decision 0400): each path named, with the
    /// generation of the check that named it (`observed_changes`, read a part at a time with the answers), the last
    /// generation, and the oldest a record may ask from (collection drops older rows).
    changes: BTreeMap<PathBuf, u64>,
    generation: u64,
    floor: u64,
    /// The most answers a root checked by its listings may hold (`list_max`; a test of the journal sets 0).
    list_max: usize,
    /// How long a larger root's replay may be started again for its marker before the root is checked by its
    /// listings instead (`journal::since`): none in a run; a test of the journal sets a few seconds.
    journal_settle: std::time::Duration,
}

/// What checking one kept answer by its `stat` costs, about (a warm `lstat` on APFS, with the compare): the journal is
/// waited for no longer than the answers under a root would take so (Q221).
pub const STAT_NS: u64 = 4_000;
/// The least the journal is waited for: a cookie on an idle machine came in 10 ms (t48), under which a tree's `stat`s
/// are cheaper anyway.
pub const JOURNAL_WAIT_LEAST_NS: u64 = 2_000_000;
/// How long after an answer's recording the journal's silence is not enough to trust it (Q189). FSEvents' history
/// merges a path's events within a window: a change made just after a run read the path (the run's cursor taken
/// between the two) was replayed under the earlier event's id, before the cursor, and the next run, told nothing,
/// trusted the old answer: a missed edit (the bench's full shape, 2026-11-18, on the M3, 1 to 2 steps a run). An
/// answer recorded within the window is checked by its `stat` instead: the same stat keeps it (no read, no write),
/// another is read again. 3.5 s apart were merged; the window's length is not documented, so it is wide.
pub const JOURNAL_SETTLE_NS: i64 = 120_000_000_000;

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
    /// Checked by its directories' listings (decision 0400), not asked of the journal: its cursor is the observer's
    /// own journal's (`L<generation>`).
    listed: bool,
}

/// Whether the answer `e` still holds for what is at its path now (`None`: nothing), by a listing (decision 0400):
/// the same kind and stat, a directory's size aside (a listing gives none; its entries changing change its times), and
/// recorded safely after the path last changed (`Entry::holds`).
fn listed_holds(e: &Entry, now: Option<(Kind, Stat)>) -> bool {
    match now {
        None => e.kind == Kind::Missing,
        Some((kind, stat)) => e.kind == kind && e.holds(&if kind == Kind::File { stat } else { Stat { size: e.stat.size, ..stat } }),
    }
}

/// What is at `path`, by one `stat` that follows a link (`None`: nothing there).
fn kind_and_stat(path: &Path) -> Option<(Kind, Stat)> {
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
CREATE TABLE IF NOT EXISTS run_record(key TEXT PRIMARY KEY, consulted TEXT NOT NULL, outputs BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS observed_changes(path TEXT PRIMARY KEY, gen INTEGER NOT NULL) WITHOUT ROWID;";

impl Observer {
    /// An observer over the store at `path` (`<path>` is the P0 store itself), or over none. What is read here is
    /// the roots' cursors and the Git aliases; the answers and canonical forms are read a root at a time, as a run
    /// first watches or reads under it (`ensure`), and a run's record by its key as it is asked (`run_record`). The
    /// store is asked only outside the waves (decision 0104). What changes is written behind the run by the
    /// observer's writer, and never read back: the observer's memory is ahead of its file.
    fn open(path: Option<&Path>) -> Observer {
        let (entries, mut aliases, canon) = (BTreeMap::new(), HashMap::new(), BTreeMap::new());
        let db = path.map(|p| {
            let (reader, meta) = read_open(p, &mut aliases);
            let stored = meta.iter().filter(|(k, _)| k.starts_with("journal:")).filter_map(|(k, v)| Some((k.clone(), text_cursor(v)?))).collect();
            Kept { records: HashMap::new(), meta, stored, writer: Writer::open(p), reader, loaded: Vec::new(), deleted: false, asked: HashSet::new() }
        });
        // Each process checks another slice of the trusted files, by the second it opened in: consecutive processes
        // take consecutive slices, as a stored counter's would, and no process writes to say which it took (a warm run
        // writes nothing, `RERAIL_WARM`). What is lost is the counter's guarantee that 64 processes in a row cover every
        // file once: two in the same second check the same slice, and coverage holds over seconds rather than runs.
        let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let validate_every = std::env::var("RERAIL_VALIDATE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(VALIDATE_EVERY);
        // The journal's cookies in a directory of their own: a stream watches it with its roots, and is told every
        // event in it. Beside the store, it was told every write of the store, its outputs and logs (2,596 events in
        // one warm batch's first ask, all of them replayed and dropped).
        let cookies = path.map(cookie_dir).filter(|d| super::prepare_cache_directory(d).is_ok());
        let stored_roots: Vec<PathBuf> = db
            .as_ref()
            .map(|k| k.meta.keys().filter(|k| k.starts_with("journal:")).filter_map(|k| k.splitn(3, ':').nth(2)).map(PathBuf::from).collect())
            .unwrap_or_default();
        // The journal's streams for the roots the last runs watched, started now: their replays run while the process
        // starts, before a run asks (decision 0105).
        let mut volumes: Vec<(PathBuf, Option<[u8; 16]>)> = Vec::new();
        if journal_on()
            && let (Some(k), Some(dir)) = (db.as_ref(), cookies.as_deref())
        {
            let roots: Vec<(PathBuf, String)> = stored_roots
                .iter()
                .filter_map(|r| {
                    let v = super::journal::volume(r);
                    volumes.push((r.clone(), v));
                    let v = v?;
                    match k.meta.get(&cursor_key(r, v))? {
                        rusqlite::types::Value::Text(t) => Some((r.clone(), t.clone())),
                        rusqlite::types::Value::Integer(i) => Some((r.clone(), i.to_string())),
                        _ => None,
                    }
                })
                .collect();
            super::journal::prepare(&roots, dir);
        }
        // The content store this observer's store is named after (`store_beside`), and the outputs runs recorded.
        let mut own: Vec<PathBuf> = path.map(|p| p.to_string_lossy().strip_suffix(".files").map_or_else(|| p.to_path_buf(), PathBuf::from)).into_iter().flat_map(|p| spellings(&p)).collect();
        if let Some(k) = db.as_ref() {
            own.extend(k.meta.keys().filter_map(|k| k.strip_prefix(OWN_OUTPUT)).flat_map(|d| spellings(Path::new(d))));
        }
        // The observer's own journal's last generation and the oldest a record may ask from (decision 0400).
        let meta_int = |key: &str| db.as_ref().and_then(|k| k.meta.get(key)).and_then(|v| match v {
            rusqlite::types::Value::Integer(i) => u64::try_from(*i).ok(),
            _ => None,
        });
        let (generation, floor) = (meta_int(LISTING_GEN).unwrap_or(0), meta_int(LISTING_FLOOR).unwrap_or(0));
        Observer { db, entries, dirty: Vec::new(), racy: Vec::new(), aliases, new_aliases: Vec::new(), canon, new_canon: Vec::new(), epoch, validate_every, counts: Counts::default(), cwd: std::env::current_dir().unwrap_or_default(), git: Vec::new(), roots: Vec::new(), flushes: 0, open: 0, stored_roots, volumes, cookies, own, unreported: Vec::new(), changes: BTreeMap::new(), generation, floor, list_max: list_max(), journal_settle: std::time::Duration::ZERO, settle_ns: JOURNAL_SETTLE_NS, wait_least_ns: journal_wait_least() }
    }

    /// Records `dir` as a directory a run writes its outputs into: rerail's own (decision 0284), for this observer
    /// and, in its store, for the processes after where their journal reports from (one write, the first time a
    /// directory under a watched root is named).
    pub fn own_output(&mut self, dir: &Path) {
        let dir = self.abs(dir);
        // Under an own directory already (a phase's outputs inside its run's): nothing new.
        if self.is_own(&dir) {
            return;
        }
        let spelled = spellings(&dir);
        // Kept for later processes only where their journal reports from: under a root the store has a cursor for.
        // Elsewhere (phase 0's scratch outputs in the temporary directory) it is own for this process alone.
        self.own.extend(spelled);
        // Its root may be watched later in the run: then it is kept at the run's end (`flush`).
        if !self.reported(&dir) {
            self.unreported.push(dir);
            return;
        }
        self.record_own(dir);
    }

    /// Whether a later run's journal reports changes under `dir`: it is under a root the store has a cursor for.
    fn reported(&self, dir: &Path) -> bool {
        spellings(dir).iter().any(|d| self.stored_roots.iter().any(|r| d.starts_with(r)))
    }

    /// Keeps `dir` as an output directory in the store, once (`own_output`).
    fn record_own(&mut self, dir: PathBuf) {
        let Some(k) = self.db.as_mut() else { return };
        let key = format!("{OWN_OUTPUT}{}", dir.display());
        if k.meta.contains_key(&key) {
            return;
        }
        k.meta.insert(key.clone(), rusqlite::types::Value::Integer(1));
        k.writer.send(move |db| {
            let _ = db.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, 1)", params![key]);
        });
    }

    /// Whether `path` is one of rerail's own files (decision 0284): this observer's or the process's.
    fn is_own(&self, path: &Path) -> bool {
        self.own.iter().any(|o| owned_by(o, path)) || rerail_prof::code_rule_dsl::logic::profile::machine::<OwnFiles>().0.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|o| owned_by(o, path))
    }

    /// The kind and stat of an own file, asked of the system each time: no answer is kept (decision 0284).
    fn own_stat(path: &Path) -> Option<(Kind, Stat)> {
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

    /// The entry kept for `path` (its root's answers read from the store first, `ensure`).
    fn known(&mut self, path: &Path) -> Option<Entry> {
        self.ensure(path);
        self.entries.get(path).cloned()
    }

    /// Reads what the store keeps under `dir` now, before a walk of it reads it a path at a time (`ensure`).
    pub fn preload(&mut self, dir: &Path) {
        if source::contains(dir) { return; }
        let dir = self.abs(dir);
        self.ensure(&dir);
    }

    /// Reads the answers and canonical forms the store keeps for the part of the tree `path` is in, once a process:
    /// under the widest root the store has a cursor for above it, else under its directory. Not inside the waves,
    /// where no store is asked (decision 0104): there `path` is unknown, and read again.
    fn ensure(&mut self, path: &Path) {
        let Some(k) = self.db.as_ref() else { return };
        if k.loaded.iter().any(|u| path.starts_with(u)) {
            return;
        }
        let unit = match self.stored_roots.iter().filter(|r| under(path, r)).min_by_key(|r| r.as_os_str().len()) {
            Some(r) => r.clone(),
            None => path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(path).to_path_buf(),
        };
        self.load_under(&unit);
    }

    /// Reads the answers and canonical forms the store keeps at and under `unit` (`ensure`). Memory is ahead of the
    /// store, so what memory holds is kept as it is; and a part is read before anything under it is forgotten
    /// (`watch` reads its root first), so nothing forgotten is read back before the writer deletes it.
    fn load_under(&mut self, unit: &Path) {
        let Some(k) = self.db.as_mut() else { return };
        if k.loaded.iter().any(|u| unit.starts_with(u)) {
            return;
        }
        if !super::hazards::may_read_store() {
            super::hazards::warm_fault("the observer's store not asked inside the waves", || format!("{}: its answers unknown, read again", unit.display()));
            return;
        }
        if std::mem::take(&mut k.deleted) {
            k.writer.written();
        }
        let Some(reader) = k.reader.as_ref() else { return };
        let at = unit.to_string_lossy();
        // What is under `at`: `at/…`, or `/…` under the root itself (not `//…`).
        let under = if at.ends_with('/') { at.to_string() } else { format!("{at}/") };
        let (from, to) = (under.clone(), format!("{under}\u{10ffff}"));
        if let Ok(mut st) = reader.prepare_cached("SELECT path, kind, dev, ino, size, mtime_ns, ctime_ns, digest, listing, recorded_ns FROM observed WHERE path = ?1 OR (path >= ?2 AND path < ?3)")
            && let Ok(rows) = st.query_map(params![at, from, to], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, [r.get::<_, i64>(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?], r.get::<_, Option<Vec<u8>>>(7)?, r.get::<_, Option<Vec<u8>>>(8)?, r.get::<_, i64>(9)?))
            })
        {
            for (path, kind, [dev, ino, size, mtime_ns, ctime_ns], digest, listing, recorded_ns) in rows.flatten() {
                let listing = match listing.map(|b| decode_listing(&b)) {
                    Some(None) => continue,
                    Some(Some(l)) => Some(Arc::new(l)),
                    None => None,
                };
                self.entries.entry(PathBuf::from(path)).or_insert(Entry { kind: Kind::of_code(kind), stat: Stat { dev, ino, size, mtime_ns, ctime_ns }, digest: digest.and_then(|d| d.try_into().ok()).unwrap_or_default(), listing, recorded_ns });
            }
        }
        // The observer's own journal's rows for the part (decision 0400).
        if let Ok(mut st) = reader.prepare_cached("SELECT path, gen FROM observed_changes WHERE path = ?1 OR (path >= ?2 AND path < ?3)")
            && let Ok(rows) = st.query_map(params![at, from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        {
            for (path, generation) in rows.flatten() {
                self.changes.entry(PathBuf::from(path)).or_insert(generation as u64);
            }
        }
        let key = format!("{CANONICAL}{at}");
        let (from, to) = (format!("{CANONICAL}{under}"), format!("{CANONICAL}{under}\u{10ffff}"));
        if let Ok(mut st) = reader.prepare_cached("SELECT key, consulted FROM run_record WHERE key = ?1 OR (key >= ?2 AND key < ?3)")
            && let Ok(rows) = st.query_map(params![key, from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            for (key, consulted) in rows.flatten() {
                if let Some(p) = key.strip_prefix(CANONICAL) {
                    self.canon.entry(PathBuf::from(p)).or_insert_with(|| PathBuf::from(consulted));
                }
            }
        }
        if std::env::var_os("RERAIL_OBSERVE_TRACE").is_some() {
            eprintln!("rerail: observer read {} ({} answers, {} forms held)", unit.display(), self.entries.len(), self.canon.len());
        }
        // A part read now holds those under it read before.
        k.loaded.retain(|u| !u.starts_with(unit));
        k.loaded.push(unit.to_path_buf());
    }

    /// The answer kept for `path` when the journal says nothing changed there since it was recorded (decision 0105):
    /// used as it is. A write after the answer was recorded, even in the same tick, came after the cursor the
    /// recording run took, so the journal names it: neither the racy rule nor a stat check is needed.
    fn trusted(&mut self, path: &Path) -> Option<Entry> {
        self.watch_above(path);
        let listed = self.trusting_root(path)?;
        let e = self.known(path)?;
        // Recorded within the journal's settle window: its silence may hide a merged event (Q189). Not under a root
        // its listings checked (decision 0400): those compared each answer with the directory as it is now.
        if !listed && self.settling(&e) {
            return None;
        }
        self.counts.trusted += 1;
        Some(e)
    }

    /// The root asked this run that trusts the answers under `path`: whether its listings checked it (decision 0400),
    /// not the journal. `None` where none trusts them.
    fn trusting_root(&self, path: &Path) -> Option<bool> {
        let mut found = None;
        for r in self.roots.iter().filter(|r| r.trusted && r.asked == self.flushes && under(path, &r.path)) {
            // A root the journal answered for is the stricter: its silence is what the settle window doubts.
            found = Some(found.unwrap_or(true) && r.listed);
        }
        found
    }

    /// Whether `e` was recorded within the journal's settle window ([`JOURNAL_SETTLE_NS`]).
    fn settling(&self, e: &Entry) -> bool {
        now_ns().saturating_sub(e.recorded_ns) < self.settle_ns
    }

    fn stat(&mut self, path: &Path) -> Option<(Kind, Stat)> {
        if let Some(e) = self.trusted(path) {
            return Some((e.kind, e.stat));
        }
        if self.missing(path) {
            return None;
        }
        // Every root it is under: each settles keeping only what was checked under it (nested roots, a first run's).
        for root in self.roots.iter_mut().filter(|r| !r.trusted && under(path, &r.path)) {
            root.checked.insert(path.to_owned());
        }
        if rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_OBSERVED_PATHS").is_some()) {
            let under = self.roots.iter().find(|r| under(path, &r.path)).map(|r| if r.trusted { "trusted root, no answer" } else { "untrusted root" }).unwrap_or("no root");
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
        if self.is_own(path) {
            return;
        }
        // Every root it is under: each settles keeping only what was checked under it (nested roots, a first run's).
        for root in self.roots.iter_mut().filter(|r| !r.trusted && under(path, &r.path)) {
            root.checked.insert(path.to_owned());
        }
        // The same answer, already trusted, is kept as it is: recording it again writes the store for nothing (a clean
        // Git file named by its blob was recorded on every run, `RERAIL_WARM`).
        if let Some(e) = self.entries.get(path)
            && e.kind == entry.kind
            && e.digest == entry.digest
            && e.holds(&entry.stat)
            && e.listing.as_deref().map(|l| l.as_slice()) == entry.listing.as_deref().map(|l| l.as_slice())
        {
            return;
        }
        if entry.racy() {
            self.racy.push(path.to_owned());
        }
        self.entries.insert(path.to_owned(), entry);
        self.dirty.push(path.to_owned());
    }

    /// What is at `path` now (following symlinks): one `stat`, or none where the journal says nothing changed
    /// (decision 0105): the answer kept, or for a path its directory's kept listing does not hold, missing.
    pub fn kind(&mut self, path: &Path) -> Kind {
        let path = &self.abs(path);
        if source::contains(path) {
            return if source::is_file(path) { Kind::File } else if source::is_dir(path) { Kind::Dir } else { Kind::Missing };
        }
        if self.is_own(path) {
            return Self::own_stat(path).map_or(Kind::Missing, |(k, _)| k);
        }
        self.ensure(path);
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
        let Some(listed) = self.trusting_root(dir) else { return false };
        if self.known(path).is_some() {
            return false;
        }
        // A listing recorded within the journal's settle window is not trusted for an absence (Q189): `stat` says. Not
        // under a root its listings checked (decision 0400): there the listing was compared with the directory now.
        if !listed && self.known(dir).is_some_and(|e| self.settling(&e)) {
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
        match self.canonical_kept(path) {
            Ok(answer) => answer,
            Err(path) => {
                let c = dunce::canonicalize(&path).ok()?;
                Some(self.canonical_resolved(path, c))
            }
        }
    }

    /// Each of `paths`' canonical forms ([`Observer::canonical`]), the `realpath`s needed run at once on the pool: on
    /// a cold store the journal vouches for no path, and one `realpath` after another was the main thread's for 29,885
    /// files (notes/t40). `observe` reaches the observer (the process's, under its lock): the `realpath`s run with it
    /// released, since a thread waiting on the pool may run another job, and that job may observe.
    pub fn canonical_all(observe: impl Fn(&mut dyn FnMut(&mut Observer)), paths: &[PathBuf]) -> Vec<Option<PathBuf>> {
        let mut out: Vec<Option<PathBuf>> = vec![None; paths.len()];
        let mut todo: Vec<(usize, PathBuf)> = Vec::new();
        observe(&mut |o| {
            for (i, p) in paths.iter().enumerate() {
                match o.canonical_kept(p) {
                    Ok(answer) => out[i] = answer,
                    Err(path) => todo.push((i, path)),
                }
            }
        });
        if todo.is_empty() {
            return out;
        }
        let got = realpaths(&todo.iter().map(|(_, p)| p.as_path()).collect::<Vec<_>>());
        observe(&mut |o| {
            for ((i, path), c) in std::mem::take(&mut todo).into_iter().zip(&got) {
                out[i] = c.clone().map(|c| o.canonical_resolved(path, c));
            }
        });
        out
    }

    /// `path`'s canonical form where the observer can answer for it (`Ok`), else its absolute form, to resolve (`Err`).
    fn canonical_kept(&mut self, path: &Path) -> std::result::Result<Option<PathBuf>, PathBuf> {
        let path = self.abs(path);
        if source::contains(&path) { return Ok(source::canonicalize(&path).ok()); }
        if self.is_own(&path) {
            // rerail's own file (decision 0284): answered here, and never recorded (`canonical_resolved` is not reached).
            return Ok(dunce::canonicalize(&path).ok());
        }
        self.ensure(&path);
        self.watch_above(&path);
        let asked = self.flushes;
        let trusted = |p: &Path, roots: &[Root]| roots.iter().any(|r| r.trusted && r.asked == asked && (under(p, &r.path) || under(p, &r.real)));
        if plain(&path) && trusted(&path, &self.roots) {
            // An answer through a symlink is trusted only where its target is watched too.
            if let Some(c) = self.kept_canonical(&path).filter(|c| trusted(c, &self.roots)) {
                self.counts.trusted += 1;
                return Ok(Some(c));
            }
            if self.missing(&path) {
                return Ok(None);
            }
        }
        for root in self.roots.iter_mut().filter(|r| !r.trusted && under(&path, &r.path)) {
            root.resolved.insert(path.clone());
        }
        self.counts.resolved += 1;
        Err(path)
    }

    /// Records `path`'s canonical form `c`, resolved: kept for the next run.
    fn canonical_resolved(&mut self, path: PathBuf, c: PathBuf) -> PathBuf {
        if self.canon.get(&path) != Some(&c) {
            super::hazards::warm_fault("the observer resolved a canonical form it did not hold", || {
                format!("{} -> {} (held {:?}; {} forms held, store {})", path.display(), c.display(), self.canon.get(&path), self.canon.len(), self.db.is_some())
            });
            self.canon.insert(path.clone(), c.clone());
            self.new_canon.push((path, c.clone()));
        }
        c
    }

    /// The canonical form kept for `path` (its root's read from the store first, `ensure`).
    fn kept_canonical(&mut self, path: &Path) -> Option<PathBuf> {
        self.ensure(path);
        self.canon.get(path).cloned()
    }

    /// Whether this trusted file is one this process checks again.
    fn checks(&self, path: &Path) -> bool {
        self.validate_every != 0 && (fnv(path.as_os_str().as_encoded_bytes()).wrapping_add(self.epoch)).is_multiple_of(self.validate_every)
    }

    /// The sha256 of the file at `path` (`None`: not a readable file), read only when its stat changed.
    pub fn digest(&mut self, path: &Path) -> Option<[u8; 32]> {
        match self.digest_step(path) {
            DigestStep::Answered(d) => d,
            DigestStep::Read(read) => {
                let hashed = read.hash();
                self.digest_read(read, hashed)
            }
        }
    }

    /// `digest` up to its read: the answer where none is needed, else the file to read and hash (`DigestRead::hash`,
    /// which needs no observer, so many files are read at once, outside its lock) and then record (`digest_read`).
    pub fn digest_step(&mut self, path: &Path) -> DigestStep {
        let path = &self.abs(path);
        if source::contains(path) {
            let Some(blob) = source::blob(path) else { return DigestStep::Answered(None) };
            let key = blob.key_bytes();
            if let Some(digest) = self.alias(&key) { return DigestStep::Answered(Some(digest)); }
            return DigestStep::Read(DigestRead { path: path.clone(), stat: None, started: 0, known: None, blob: Some(key) });
        }
        if self.is_own(path) {
            return DigestStep::Answered(self.read(path).map(|(_, d)| d));
        }
        self.ensure(path);
        if let Some(e) = self.trusted(path).filter(|e| e.kind == Kind::File) {
            self.counts.reused += 1;
            return DigestStep::Answered(Some(e.digest));
        }
        let started = now_ns();
        let Some((kind, now)) = self.stat(path) else { return DigestStep::Answered(None) };
        if kind != Kind::File {
            return DigestStep::Answered(None);
        }
        let read = |known, blob| DigestStep::Read(DigestRead { path: path.clone(), stat: Some(now), started, known, blob });
        if let Some(e) = self.known(path).filter(|e| e.kind == Kind::File && e.holds(&now)) {
            if !self.checks(path) {
                self.counts.reused += 1;
                return DigestStep::Answered(Some(e.digest));
            }
            self.counts.validated += 1;
            return read(Some(e.digest), None);
        }
        // A clean Git file whose stat is its index entry's holds its blob: the digest of bytes seen before under
        // that blob, unread; or read once, and the blob remembered.
        // Git's work tree root is canonical: so is the path compared with it (one `realpath`, in Git runs only).
        let canonical = if self.git.is_empty() { None } else { dunce::canonicalize(path).ok() };
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
                return DigestStep::Answered(Some(d));
            }
            return read(None, Some(key));
        }
        read(None, None)
    }

    /// Records a file read for `digest` (`digest_step`): its digest, `None` when it could not be read.
    pub fn digest_read(&mut self, read: DigestRead, hashed: Option<(u64, [u8; 32])>) -> Option<[u8; 32]> {
        let (len, digest) = hashed?;
        self.counts.hashed += 1;
        self.counts.bytes += len;
        if let Some(stat) = read.stat {
            self.record_hash(&read.path, stat, read.started, digest);
        }
        if read.known.is_some_and(|k| k != digest) {
            self.counts.mismatched += 1;
        }
        if let Some(key) = read.blob {
            self.record_alias(&key, digest);
        }
        Some(digest)
    }

    /// Starts a run: the Git work trees are discovered again, as they may have changed since, and each root is
    /// asked of the journal again (`watch`).
    pub fn begin(&mut self) {
        self.git.clear();
        // An outermost run: the journal is synchronised once for it, and the runs inside it share that.
        if self.open == 0 {
            super::journal::next_epoch();
        }
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
            if source::contains(root) { continue; }
            let root = self.outermost(&self.abs(root));
            if self.roots.iter().any(|r| r.asked == self.flushes && under(&root, &r.path)) {
                continue;
            }
            // Asked for an earlier run: asked again (its pending cursor, if any, was settled at that run's end).
            self.roots.retain(|r| r.path != root);
            // Its part of the store read first: what the journal names under it is then forgotten from memory, and
            // none of it read back after (`load_under`).
            self.ensure(&root);
            if !self.stored_roots.contains(&root) {
                self.stored_roots.push(root.clone());
            }
            let volume = self.volume(&root);
            // A cursor is of the volume it was taken on: kept under the volume's id with the root. It is the journal's
            // own text (an FSEvents id, a Watchman clock); one kept as a number is an id.
            let kept = |v: [u8; 16]| match self.db.as_ref()?.meta.get(&cursor_key(&root, v))? {
                rusqlite::types::Value::Text(t) => Some(t.clone()),
                rusqlite::types::Value::Integer(i) => Some(i.to_string()),
                _ => None,
            };
            // A small root is checked by its directories' listings (decision 0400): what FSEvents replays to a new
            // process is not all that changed before it (t46), and listing each directory that holds an answer costs
            // less than waiting for the replay. Its cursor is kept under the volume's id, or under none where the
            // system names no volume (`check_listed`), and read back so.
            if self.listable(&root) {
                let cursor = kept(volume.unwrap_or([0; 16]));
                self.check_listed(root, volume, cursor.as_deref());
                continue;
            }
            let cursor = volume.and_then(kept);
            // The journal is waited for no longer than checking every answer kept under the root by its `stat`
            // would take (Q221: planning costs no more than running, 0122); slower, it is checked so.
            let kept = self.entries.range::<Path, _>((std::ops::Bound::Included(root.as_path()), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(&root)).count();
            let wait = std::time::Duration::from_nanos((kept as u64).saturating_mul(STAT_NS).max(self.wait_least_ns));
            match super::journal::since_within(std::slice::from_ref(&root), cursor.as_deref(), &cookies, self.journal_settle, Some(wait)) {
                // The replay may not hold every change made before this run (its marker was not in its history):
                // the root is checked by its listings, keeping the journal's cursor (decision 0400, step 2).
                super::journal::Since::Behind { cursor: next } => {
                    self.counts.late += 1;
                    self.check_behind(root, volume, &next)
                }
                super::journal::Since::Changed { paths, cursor: next } => {
                    // News of rerail's own files is no news (decision 0284): each run writes them.
                    let paths: Vec<PathBuf> = paths.into_iter().filter(|p| !self.is_own(p)).collect();
                    self.forget(&root, &paths, volume, &next);
                    self.roots.push(Root { real: dunce::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: Some(next), path: root, asked: self.flushes, trusted: true, pending: None, checked: Default::default(), resolved: Default::default(), listed: false });
                }
                super::journal::Since::Lost { cursor, why } => {
                    if super::journal::late(why) {
                        self.counts.late += 1;
                    }
                    let mut pending = cursor.map(|c| (volume, c));
                    // The roots inside it this process asked already hold their answers as of their cursors: this
                    // run's that the journal could not speak for either (settled at its end), and earlier runs'
                    // (phase 0's, before the corpus run's). What is under them is this root's too, and its cursor the
                    // earliest of theirs (the first asked: a cursor taken earlier reports more), so those answers hold
                    // as of it. Settled apart, the outer root forgot what was answered before it was asked, and the
                    // inner one what was checked as the outer one's: the run after a cold one recorded it all again.
                    // This run's untrusted ones are taken in, settled as this root.
                    let mut inner: Vec<(PathBuf, String, bool)> = Vec::new();
                    for r in &self.roots {
                        if r.path.starts_with(&root) && r.path != root && let Some(c) = r.cursor.clone() {
                            inner.push((r.path.clone(), c, r.asked == self.flushes && !r.trusted));
                        }
                    }
                    let inner: Vec<(PathBuf, String, bool)> = inner.into_iter().filter(|(p, _, _)| self.volume(p) == volume).collect();
                    if let (Some((_, c)), Some((_, first, _))) = (pending.as_mut(), inner.first()) {
                        *c = first.clone();
                    }
                    let (mut checked, mut resolved) = (HashSet::new(), HashSet::new());
                    for (path, _, this_run) in &inner {
                        if *this_run {
                            continue;
                        }
                        checked.extend(self.entries.range::<Path, _>((std::ops::Bound::Included(path.as_path()), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(path)).map(|(k, _)| k.clone()));
                        resolved.extend(self.canon.range::<Path, _>((std::ops::Bound::Included(path.as_path()), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(path)).map(|(k, _)| k.clone()));
                    }
                    let taken: Vec<&PathBuf> = inner.iter().filter(|(_, _, this_run)| *this_run).map(|(p, _, _)| p).collect();
                    let (absorbed, rest): (Vec<Root>, Vec<Root>) = std::mem::take(&mut self.roots).into_iter().partition(|r| r.asked == self.flushes && !r.trusted && taken.contains(&&r.path));
                    self.roots = rest;
                    for r in absorbed {
                        checked.extend(r.checked);
                        resolved.extend(r.resolved);
                    }
                    self.roots.push(Root { real: dunce::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: pending.as_ref().map(|(_, c)| c.clone()), path: root, asked: self.flushes, trusted: false, pending, checked, resolved, listed: false });
                }
            }
        }
    }

    /// Whether `root` is checked by its directories' listings (decision 0400): where this system lists entries with
    /// their stats, and the root holds at most `list_max` answers (its part of the store read first, `watch`).
    fn listable(&self, root: &Path) -> bool {
        let max = self.list_max;
        listing::SUPPORTED && max > 0 && self.entries.range::<Path, _>((std::ops::Bound::Included(root), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(root)).take(max + 1).count() <= max
    }

    /// Checks the answers kept under `root` against its directories' listings, forgets those that no longer hold,
    /// and trusts the rest for this run (decision 0400). What it names goes in the observer's own journal under the
    /// next generation; the root's cursor is that journal's. Naming nothing writes nothing, but a root's first
    /// cursor of that journal (a warm run writes none).
    fn check_listed(&mut self, root: PathBuf, volume: Option<[u8; 16]>, stored: Option<&str>) {
        let (exact, whole, stale) = self.listing_named(&root);
        let (exact, whole): (Vec<PathBuf>, Vec<PathBuf>) = (exact.into_iter().filter(|p| !self.is_own(p)).collect(), whole.into_iter().filter(|p| !self.is_own(p)).collect());
        self.counts.checked_named += (exact.len() + whole.len()) as u64;
        // Kept under the volume's id where the system names one, as a journal's cursor is; else under none.
        let volume = volume.unwrap_or([0; 16]);
        if exact.is_empty() && whole.is_empty() && stale.is_empty() {
            let next = format!("L{}", self.generation);
            if stored.and_then(listing_cursor).is_none() {
                self.store_listing_cursor(&root, volume, &next);
            }
            self.roots.push(Root { real: dunce::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: Some(next), path: root, asked: self.flushes, trusted: true, pending: None, checked: Default::default(), resolved: Default::default(), listed: true });
            return;
        }
        self.generation += 1;
        let next = format!("L{}", self.generation);
        self.forget_named(&root, &exact, &whole, &stale, volume, &next);
        self.roots.push(Root { real: dunce::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: Some(next), path: root, asked: self.flushes, trusted: true, pending: None, checked: Default::default(), resolved: Default::default(), listed: true });
    }

    /// A larger root whose replay was `Behind` (decision 0400, step 2): checked by its listings as a small root is,
    /// what they name forgotten and logged as `check_listed` does, the root's cursor the journal's (`next`, every
    /// change after it told to the next replay). Naming nothing writes nothing: the stored cursor stays, and the next
    /// replay from it names again what this run found holding, a path looked at again, never one missed.
    fn check_behind(&mut self, root: PathBuf, volume: Option<[u8; 16]>, next: &str) {
        let (exact, whole, stale) = self.listing_named(&root);
        let (exact, whole): (Vec<PathBuf>, Vec<PathBuf>) = (exact.into_iter().filter(|p| !self.is_own(p)).collect(), whole.into_iter().filter(|p| !self.is_own(p)).collect());
        self.counts.checked_named += (exact.len() + whole.len()) as u64;
        if !(exact.is_empty() && whole.is_empty() && stale.is_empty()) {
            self.generation += 1;
            self.forget_named(&root, &exact, &whole, &stale, volume.unwrap_or([0; 16]), next);
        }
        self.roots.push(Root { real: dunce::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: Some(next.to_owned()), path: root, asked: self.flushes, trusted: true, pending: None, checked: Default::default(), resolved: Default::default(), listed: false });
    }

    /// The answers under `root` that no longer hold, by its directories' listings: those to forget alone (`exact`),
    /// and the symbolic links that changed, forgotten with what is under them and their canonical forms (`whole`).
    /// Each directory holding an answer is listed once; the root itself, and each link, is `stat`ed on its own.
    ///
    /// A directory still a directory whose stat changed has a changed listing, not changed files: as a file event
    /// would, its entries added or gone are named (`exact`, the parent rule catches its listing's readers), and its
    /// own answer is forgotten unnamed (`stale`). Named, it covered every answer under it, and a file made in a
    /// directory asked again for every unit read there (`a_header_edit_asks_again_only_for_the_units_that_read_it`).
    fn listing_named(&mut self, root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>) {
        let kept: Vec<(PathBuf, Entry)> = self.entries.range::<Path, _>((std::ops::Bound::Included(root), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(root)).map(|(k, e)| (k.clone(), e.clone())).collect();
        let (mut exact, mut whole, mut stale) = (Vec::new(), Vec::new(), Vec::new());
        let mut by_dir: BTreeMap<PathBuf, Vec<(PathBuf, Entry)>> = BTreeMap::new();
        for (path, e) in kept {
            match path.parent().filter(|_| path != root) {
                Some(dir) => by_dir.entry(dir.to_path_buf()).or_default().push((path, e)),
                None => {
                    self.counts.stats += 1;
                    let now = kind_and_stat(&path);
                    if !listed_holds(&e, now) {
                        self.dir_named(path, &e, now, &mut exact, &mut stale);
                    }
                }
            }
        }
        for (dir, kids) in by_dir {
            self.counts.checked_dirs += 1;
            let listed = match listing::entries(&dir) {
                Ok(list) => list.into_iter().collect::<HashMap<_, _>>(),
                // Nothing can be under a directory that is gone or is not one: an answer that it is missing holds.
                Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
                    exact.extend(kids.into_iter().filter(|(_, e)| e.kind != Kind::Missing).map(|(p, _)| p));
                    continue;
                }
                Err(_) => {
                    exact.extend(kids.into_iter().map(|(p, _)| p));
                    continue;
                }
            };
            for (path, e) in kids {
                let Some(name) = path.file_name() else {
                    exact.push(path);
                    continue;
                };
                match listed.get(name) {
                    None if e.kind == Kind::Missing => {}
                    None => exact.push(path),
                    Some(Some(now)) => {
                        if !listed_holds(&e, Some(*now)) {
                            self.dir_named(path, &e, Some(*now), &mut exact, &mut stale);
                        }
                    }
                    // A symbolic link (or an entry the listing gave no stat for): its answer is of what it names.
                    Some(None) => {
                        self.counts.stats += 1;
                        if !listed_holds(&e, kind_and_stat(&path)) {
                            whole.push(path);
                        }
                    }
                }
            }
        }
        // A file gone is named as its own answer and as its directory's entry: once.
        exact.sort();
        exact.dedup();
        (exact, whole, stale)
    }

    /// What `listing_named` names of `path`, whose answer `e` no longer holds (`now`): a directory still one, its
    /// entries added or gone, its answer `stale`; a file of the same stat recorded inside the racy window, nothing
    /// where its bytes are as they were (recorded again, outside it); anything else, itself. The journal names a
    /// file only where it changed: a header written just before a run was named again by the next run's listings, and
    /// asked again for the unit that read it.
    fn dir_named(&mut self, path: PathBuf, e: &Entry, now: Option<(Kind, Stat)>, exact: &mut Vec<PathBuf>, stale: &mut Vec<PathBuf>) {
        if let Some((Kind::File, stat)) = now
            && e.kind == Kind::File
            && e.stat == stat
        {
            let started = now_ns();
            match std::fs::read(&path) {
                Ok(bytes) if <[u8; 32]>::from(Sha256::digest(&bytes)) == e.digest => {
                    self.counts.validated += 1;
                    self.remember(&path, Entry { kind: Kind::File, stat, digest: e.digest, listing: None, recorded_ns: started });
                }
                _ => exact.push(path),
            }
            return;
        }
        if !(e.kind == Kind::Dir && matches!(now, Some((Kind::Dir, _)))) {
            exact.push(path);
            return;
        }
        // Never listed: its answer was its kind alone, which holds.
        if let Some(was) = &e.listing {
            let Some(listing) = self.read_listing(&path, false) else {
                exact.push(path);
                return;
            };
            let was: BTreeSet<&str> = was.iter().map(|(n, _)| n.as_str()).collect();
            let is: BTreeSet<&str> = listing.iter().map(|(n, _)| n.as_str()).collect();
            exact.extend(was.symmetric_difference(&is).map(|n| path.join(n)));
        }
        stale.push(path);
    }

    /// Stores `cursor` as `root`'s, the observer's own journal's (`check_listed`).
    fn store_listing_cursor(&mut self, root: &Path, volume: [u8; 16], cursor: &str) {
        let Some(k) = self.db.as_mut() else { return };
        let key = cursor_key(root, volume);
        k.meta.insert(key.clone(), rusqlite::types::Value::Text(cursor.to_owned()));
        k.stored.insert(key.clone(), cursor.to_owned());
        let cursor = cursor.to_owned();
        k.writer.send(move |db| {
            let _ = db.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
        });
    }

    /// Forgets the answers a root's listings named (`listing_named`): each of `exact` alone, each of `whole` with
    /// what is under it and its canonical forms; records each in the observer's own journal under the current
    /// generation, with the root's cursor and the generation, in one transaction.
    /// `stale`: forgotten as `exact` is, but not named (`listing_named`).
    fn forget_named(&mut self, root: &Path, exact: &[PathBuf], whole: &[PathBuf], stale: &[PathBuf], volume: [u8; 16], cursor: &str) {
        let generation = self.generation;
        let mut forgotten = 0u64;
        for p in exact.iter().chain(stale) {
            forgotten += u64::from(self.entries.remove(p.as_path()).is_some());
            self.canon.remove(p.as_path());
        }
        for g in whole {
            let keys: Vec<PathBuf> = self.entries.range::<Path, _>((std::ops::Bound::Included(g.as_path()), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(g)).map(|(k, _)| k.clone()).collect();
            forgotten += keys.iter().filter(|k| self.entries.remove(k.as_path()).is_some()).count() as u64;
            let keys: Vec<PathBuf> = self.canon.range::<Path, _>((std::ops::Bound::Included(g.as_path()), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(g)).map(|(k, _)| k.clone()).collect();
            for k in keys {
                self.canon.remove(&k);
            }
        }
        if !self.new_canon.is_empty() {
            let named: BTreeSet<&Path> = exact.iter().chain(whole).map(PathBuf::as_path).collect();
            self.new_canon.retain(|(k, _)| !named.contains(k.as_path()) && !k.ancestors().any(|a| whole.iter().any(|w| w == a)));
        }
        // What the journal records: each path named (a directory whose listing changed, as its entries: `dir_named`).
        let logged: Vec<PathBuf> = exact.iter().chain(whole).cloned().collect();
        for p in &logged {
            self.changes.insert(p.clone(), generation);
        }
        self.counts.forgotten += forgotten;
        let Some(k) = self.db.as_mut() else { return };
        let key = cursor_key(root, volume);
        k.meta.insert(key.clone(), rusqlite::types::Value::Text(cursor.to_owned()));
        k.meta.insert(LISTING_GEN.to_owned(), rusqlite::types::Value::Integer(generation as i64));
        k.stored.insert(key.clone(), cursor.to_owned());
        super::hazards::warm_fault("the observer forgot answers its listings named changed", || {
            format!("{}: {} paths named changed ({}), {forgotten} answers forgotten", root.display(), exact.len() + whole.len(), exact.iter().chain(whole).take(3).map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
        });
        k.deleted = true;
        let (exact, whole, stale, cursor) = (exact.to_vec(), whole.to_vec(), stale.to_vec(), cursor.to_owned());
        k.writer.send(move |db| {
            let Ok(tx) = db.transaction() else { return };
            {
                let Ok(mut one) = tx.prepare_cached("DELETE FROM observed WHERE path = ?1") else { return };
                let Ok(mut canon_one) = tx.prepare_cached("DELETE FROM run_record WHERE key = ?1") else { return };
                for p in exact.iter().chain(&stale) {
                    let _ = one.execute([p.to_string_lossy()]);
                    let _ = canon_one.execute([format!("{CANONICAL}{}", p.display())]);
                }
                let Ok(mut del) = tx.prepare_cached("DELETE FROM observed WHERE path = ?1 OR (path >= ?2 AND path < ?3)") else { return };
                let Ok(mut canon) = tx.prepare_cached("DELETE FROM run_record WHERE key = ?1 OR (key >= ?2 AND key < ?3)") else { return };
                for g in &whole {
                    let at = g.to_string_lossy();
                    let _ = del.execute(params![at, format!("{at}/"), format!("{at}/\u{10ffff}")]);
                    let at = format!("{CANONICAL}{}", g.display());
                    let _ = canon.execute(params![at, format!("{at}/"), format!("{at}/\u{10ffff}")]);
                }
                let Ok(mut log) = tx.prepare_cached("INSERT OR REPLACE INTO observed_changes VALUES (?1, ?2)") else { return };
                for p in &logged {
                    let _ = log.execute(params![p.to_string_lossy(), generation as i64]);
                }
                let _ = tx.execute_cached("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
                let _ = tx.execute_cached("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![LISTING_GEN, generation as i64]);
            }
            let _ = tx.commit();
        });
    }

    /// The root `root` is asked as: the widest stored root above it on the same volume, else itself. A journal's
    /// cursor is the volume's (an FSEvents id), so the outer root's answers for what is under it too: an inner root
    /// asked on its own started a stream of its own, whose cookie could miss its wait (2 s, then lost), or had no
    /// cursor of its own (lost at once), and each lost root was settled and written again every run. Where the system
    /// names no volume (Linux: the roots are checked by their listings, whose cursor is the observer's own journal's),
    /// the roots are of one, as `check_listed` keys their cursors and `collect` keeps them: collection drops an inner
    /// root's cursor as answered by the root above, so the inner root must be asked as that root.
    fn outermost(&mut self, root: &Path) -> PathBuf {
        let mut above: Vec<PathBuf> = self.stored_roots.iter().filter(|r| under(root, r) && r.as_path() != root).cloned().collect();
        above.sort_by_key(|r| r.as_os_str().len());
        let here = self.volume(root);
        // With no journal there is no volume's id (Linux, no Watchman): the same device holds both. Folded by the
        // id alone, a root under another stored one was never folded there, and kept a listing cursor of its own.
        let dev = |p: &Path| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt as _;
                std::fs::metadata(p).ok().map(|m| m.dev())
            }
            #[cfg(not(unix))]
            {
                let _ = p;
                None::<u64>
            }
        };
        let here_dev = if here.is_none() { dev(root) } else { None };
        above.into_iter().find(|r| match here {
            Some(_) => self.volume(r) == here,
            None => here_dev.is_some() && self.volume(r).is_none() && dev(r) == here_dev,
        }).unwrap_or_else(|| root.to_path_buf())
    }

    /// `root`'s volume, asked once a process.
    fn volume(&mut self, root: &Path) -> Option<[u8; 16]> {
        if let Some((_, v)) = self.volumes.iter().find(|(r, _)| r == root) {
            return *v;
        }
        let v = super::journal::volume(root);
        self.volumes.push((root.to_path_buf(), v));
        v
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
                self.roots.iter().find(|r| r.asked == self.flushes && under(&root, &r.path))?.cursor.clone()
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
    ///
    /// `record` is the run record (`run_record`) whose first list (`encode_lists`) the cursors are. Where the journal names nothing
    /// under any of its roots and a cursor is `CURSOR_AGE` behind, the record holds as of the journal's cursors now:
    /// its first list is stored again with them (a timestamp, `profile::WARM_WRITES_ALLOWED`), so its next check does
    /// not replay from far back (4 million events back, a check timed out and the record was made again).
    pub fn changed_since(&mut self, record: &str, roots: &[PathBuf], cursors: &[String]) -> Option<Vec<PathBuf>> {
        let cookies = self.cookies.clone().filter(|_| self.db.is_some())?;
        if roots.len() != cursors.len() {
            return None;
        }
        let mut named = Vec::new();
        let mut now: Vec<Option<String>> = vec![None; roots.len()];
        for (i, (root, cursor)) in roots.iter().zip(cursors).enumerate() {
            // A root under another of `roots` with the same cursor (one ask gave both, `mark`: a corpus and its
            // `.git`, a `.git` named as the repository's own and its common one) is named in the other's answer.
            let covered = roots.iter().zip(cursors).enumerate().any(|(j, (r, c))| c == cursor && under(root, r) && (r != root || j < i));
            if covered {
                continue;
            }
            // A root checked by its listings: what its checks named after the record's generation (decision 0400),
            // the root checked for this run first. A record from before the oldest kept generation is not answered.
            if let Some(g) = listing_cursor(cursor) {
                self.watch(std::slice::from_ref(root));
                let at = self.abs(root);
                let r = self.roots.iter().find(|r| r.asked == self.flushes && r.listed && under(&at, &r.path))?;
                if g < self.floor {
                    return None;
                }
                now[i] = r.cursor.clone();
                named.extend(self.changes.range::<Path, _>((std::ops::Bound::Included(at.as_path()), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(&at)).filter(|(_, n)| **n > g).map(|(k, _)| k.clone()));
                continue;
            }
            match super::journal::since(std::slice::from_ref(root), Some(cursor), &cookies, self.journal_settle) {
                super::journal::Since::Changed { paths, cursor } => {
                    named.extend(paths.into_iter().filter(|p| !self.is_own(p)));
                    now[i] = Some(cursor);
                }
                // What the replay names may not be all since the record: it is not answered.
                super::journal::Since::Lost { why, .. } => {
                    if super::journal::late(why) {
                        self.counts.late += 1;
                    }
                    return None;
                }
                super::journal::Since::Behind { .. } => {
                    self.counts.late += 1;
                    return None;
                }
            }
        }
        // A covered root's cursor moves with the one that covers it.
        for i in 0..roots.len() {
            if now[i].is_none() {
                now[i] = (0..roots.len()).find(|&j| now[j].is_some() && cursors[j] == cursors[i] && roots[i].starts_with(&roots[j])).and_then(|j| now[j].clone());
            }
        }
        let aged = cursors.iter().zip(&now).any(|(c, n)| c.parse::<u64>().ok().zip(n.as_deref().and_then(|n| n.parse::<u64>().ok())).is_some_and(|(c, n)| n.saturating_sub(c) >= CURSOR_AGE));
        if named.is_empty() && aged && now.iter().all(Option::is_some) {
            self.refresh_record(record, &now.into_iter().flatten().collect::<Vec<_>>());
        }
        // A path named under one name of a root is named under its other (the root as given and its canonical form:
        // `/var/…` and `/private/var/…` on macOS). An answer is kept under the name it was asked by, and a caller
        // matches what it read, in either name, against these: one only, and a file changed under the other was
        // taken as unchanged.
        let forms: Vec<(PathBuf, PathBuf)> = roots
            .iter()
            .filter_map(|r| {
                let at = self.abs(r);
                let real = self.roots.iter().find(|x| x.path == at).map(|x| x.real.clone()).or_else(|| dunce::canonicalize(&at).ok())?;
                (real != at).then_some((at, real))
            })
            .collect();
        let other: Vec<PathBuf> = named
            .iter()
            .flat_map(|n| forms.iter().filter_map(move |(at, real)| n.strip_prefix(real).map(|rest| at.join(rest)).or_else(|_| n.strip_prefix(at).map(|rest| real.join(rest))).ok()))
            .collect();
        for p in other {
            if !named.contains(&p) {
                named.push(p);
            }
        }
        Some(named)
    }

    /// Stores `record` again with `cursors` as its first list (`changed_since`), the rest as it was.
    fn refresh_record(&mut self, record: &str, cursors: &[String]) {
        let Some(k) = self.db.as_mut() else { return };
        let Some((consulted, outputs)) = k.records.get(record).cloned() else { return };
        // The record's first list is its cursors (`encode_lists`); the rest stays as it was.
        let Some(mut lists) = decode_lists(&consulted).filter(|l| !l.is_empty()) else { return };
        lists[0] = cursors.to_vec();
        let consulted = encode_lists(&lists.iter().map(Vec::as_slice).collect::<Vec<_>>());
        k.records.insert(record.to_owned(), (consulted.clone(), outputs.clone()));
        let key = record.to_owned();
        k.writer.send(move |db| {
            // A write a warm run may make (`profile::WARM_WRITES_ALLOWED`).
            let _scope = super::hazards::warm_scope();
            super::hazards::warm_job("the journal's aged cursor");
            let _ = db.execute("INSERT OR REPLACE INTO run_record VALUES (?1, ?2, ?3)", params![key, consulted, outputs]);
        });
    }

    /// Watches the root above `path` when no root this run watches holds it: the stored root above it (one the
    /// store has a cursor for), else its directory, which a later run then trusts. So every path read is under a
    /// root, and asked of the journal once a run.
    fn watch_above(&mut self, path: &Path) {
        if self.db.is_none() || self.roots.iter().any(|r| r.asked == self.flushes && under(path, &r.path)) {
            return;
        }
        let root = match self.stored_roots.iter().filter(|r| under(path, r)).min_by_key(|r| r.as_os_str().len()) {
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
        // The maps are sorted by path, so what is under a path is the range from it: a few lookups a path named,
        // not a match of every path named against every answer kept (971 paths named in a churning root, 8 -> 53 G
        // instructions a warm batch).
        let gone: Vec<PathBuf> = paths.to_vec();
        let listings: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        fn under<V>(map: &mut BTreeMap<PathBuf, V>, g: &Path) -> usize {
            let keys: Vec<PathBuf> = map.range::<Path, _>((std::ops::Bound::Included(g), std::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(g)).map(|(k, _)| k.clone()).collect();
            keys.iter().filter(|k| map.remove(k.as_path()).is_some()).count()
        }
        let (mut forgotten, mut canon_forgotten) = (0u64, 0usize);
        for g in &gone {
            forgotten += under(&mut self.entries, g) as u64;
            canon_forgotten += under(&mut self.canon, g);
        }
        for l in &listings {
            forgotten += u64::from(self.entries.remove(l.as_path()).is_some());
        }
        if !self.new_canon.is_empty() {
            let named: BTreeSet<&Path> = gone.iter().map(PathBuf::as_path).collect();
            self.new_canon.retain(|(k, _)| !k.ancestors().any(|a| named.contains(a)));
        }
        if let Some(k) = self.db.as_mut() {
            if let Some(v) = volume {
                k.meta.insert(cursor_key(root, v), rusqlite::types::Value::Text(cursor.to_owned()));
            }
            // Nothing named, nothing written: the store's cursor stays where it was (memory's moves on, for this
            // process's later asks), and the next process replays from there, a superset of what changed since: no
            // change is missed. A warm run then writes no cursor (`RERAIL_WARM`), where it wrote one for every root it
            // asked, every run, until it is `CURSOR_AGE` behind (a replay from 2M events back cost nothing measurable, from
            // 7M back 0.6-2 s for the first ask). A root the journal names paths
            // under keeps its cursor moving: a replay from a kept cursor names every path changed since, and matching
            // them against every answer kept grew each run (971 paths named in the work tree's root, 8 -> 53 G
            // instructions a warm batch, when the cursor stayed whenever nothing was forgotten).
            // News that forgets nothing is as none (another program's files under the root, a lock file): what is
            // kept holds as of the stored cursor still, and a warm run wrote the moved cursor for it. Only where the
            // root's part of the store is read (`watch` reads it first): memory holding nothing named then means the
            // store holds nothing either. Until the replay names `CURSOR_PATHS` paths: it names them all again each run.
            // No reader: the store held nothing as the observer opened, and what it wrote since is memory's too.
            let loaded = k.reader.is_none() || k.loaded.iter().any(|u| root.starts_with(u));
            if forgotten == 0 && canon_forgotten == 0 && (paths.is_empty() || loaded) {
                // Unless the store's cursor has aged (`CURSOR_AGE`): only an FSEvents id has an age.
                let Some(key) = volume.map(|v| cursor_key(root, v)) else { return };
                let aged = k.stored.get(&key).and_then(|s| s.parse::<u64>().ok()).zip(cursor.parse::<u64>().ok()).is_some_and(|(s, now)| now.saturating_sub(s) >= CURSOR_AGE);
                if !aged && paths.len() < CURSOR_PATHS {
                    return;
                }
                k.stored.insert(key.clone(), cursor.to_owned());
                let cursor = cursor.to_owned();
                k.writer.send(move |db| {
                    // A write a warm run may make (`profile::WARM_WRITES_ALLOWED`).
                    let _scope = super::hazards::warm_scope();
                    super::hazards::warm_job("the journal's aged cursor");
                    let _ = db.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
                });
                return;
            }
            let (gone, listings, at) = (gone.clone(), listings.clone(), volume.map(|v| (cursor_key(root, v), cursor.to_owned())));
            if let Some((key, cursor)) = &at {
                k.stored.insert(key.clone(), cursor.clone());
            }
            super::hazards::warm_fault("the observer forgot answers the journal named changed", || {
                format!("{}: {} paths named changed ({}), {forgotten} answers forgotten", root.display(), paths.len(), paths.iter().take(3).map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
            });
            k.deleted = true;
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
                        let _ = tx.execute_cached("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
                    }
                }
                let _ = tx.commit();
            });
        }
        self.counts.forgotten += forgotten;
    }

    /// The Git work tree holding `root`, discovered once per run (`None`: not in one, or Git cannot read it).
    pub fn git(&mut self, root: &Path) -> Option<Arc<crate::git_workspace::GitWorkspace>> {
        let root = dunce::canonicalize(self.abs(root)).ok()?;
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
        if source::contains(path) {
            let bytes = source::read(path).ok()?;
            let digest = Sha256::digest(&bytes).into();
            self.counts.hashed += 1;
            self.counts.bytes += bytes.len() as u64;
            if let Some(blob) = source::blob(path) { self.record_alias(&blob.key_bytes(), digest); }
            return Some((bytes, digest));
        }
        if self.is_own(path) {
            let bytes = std::fs::read(path).ok().filter(|_| std::fs::metadata(path).is_ok_and(|m| m.is_file()))?;
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            return Some((bytes, digest));
        }
        self.ensure(path);
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
        self.record_hash(path, stat, started, digest);
        Some((bytes, digest))
    }

    /// Keeps a file's digest as read with `stat`, where it is not kept so already.
    fn record_hash(&mut self, path: &Path, stat: Stat, started: i64, digest: [u8; 32]) {
        let changed = self.entries.get(path).is_none_or(|e| e.digest != digest || e.stat != stat || e.kind != Kind::File);
        if changed || self.entries.get(path).is_some_and(|e| !e.holds(&stat)) {
            self.remember(path, Entry { kind: Kind::File, stat, digest, listing: None, recorded_ns: started });
        }
    }

    /// The entries of the directory at `dir` (`None`: not a readable directory), listed only when its stat changed.
    /// An entry is a directory when what it names is one, symlinks followed.
    pub fn listing(&mut self, dir: &Path) -> Option<Listing> {
        let dir = &self.abs(dir);
        if source::contains(dir) { return source::listing(dir).ok().map(Arc::new); }
        let own = self.is_own(dir);
        if !own {
            self.ensure(dir);
        }
        // An own directory is listed each time, and nothing above it watched for it (`trusted` would).
        if let Some(listing) = (!own).then(|| self.trusted(dir)).flatten().filter(|e| e.kind == Kind::Dir).and_then(|e| e.listing) {
            self.counts.reused += 1;
            return Some(listing);
        }
        let started = now_ns();
        let (kind, now) = if own { Self::own_stat(dir)? } else { self.stat(dir)? };
        if kind != Kind::Dir {
            return None;
        }
        if let Some(listing) = self.known(dir).filter(|e| e.kind == Kind::Dir && e.holds(&now) && !own).and_then(|e| e.listing) {
            self.counts.reused += 1;
            return Some(listing);
        }
        self.counts.listed += 1;
        let listing = self.read_listing(dir, own)?;
        if encode_listing(&listing).is_some() && !own {
            self.remember(dir, Entry { kind: Kind::Dir, stat: now, digest: listing_digest(&listing), listing: Some(Arc::clone(&listing)), recorded_ns: started });
        }
        Some(listing)
    }

    /// `dir`'s entries as read now. rerail's own files are not in a listing (decision 0284): what they hold changes
    /// with every run.
    fn read_listing(&self, dir: &Path, own: bool) -> Option<Listing> {
        let mut entries: Vec<(String, bool)> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| own || !self.is_own(&e.path()))
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
        Some(Arc::new(entries))
    }

    /// The answers recorded inside the racy window, read again where the window has passed since their path last
    /// changed (by `now`): the same stat and the same bytes (or listing) are then trusted, recorded by the run that
    /// read them, behind it. Without this the next run, a warm one, reads each again and writes it (Q45: rules
    /// copied in just before a cold run). An answer whose path changed, or still inside the window, is left as it
    /// was: one still inside it waits for a later end of the process's runs (the rules a process reads first are
    /// flushed seconds before its corpus run ends), one that changed is the next run's to read. Sound by the racy
    /// rule itself: the read starts past the window, so a write after it changes the stat.
    fn settle_racy(&mut self, now: i64) {
        let mut waiting = Vec::new();
        for path in std::mem::take(&mut self.racy) {
            let Some(e) = self.entries.get(&path).filter(|e| e.racy()).cloned() else { continue };
            if e.stat.mtime_ns.max(e.stat.ctime_ns) >= now - RACY_NS {
                waiting.push(path);
                continue;
            }
            self.settle_one(&path, e, now);
        }
        self.racy.extend(waiting);
    }

    /// Whether `e`, the racy answer at `path`, is still what is there: the same stat, and the same bytes (or listing)
    /// read now. Recorded again by `now` where the window has passed (`settle_racy`); inside it, left racy (a later
    /// write in the same tick keeps the stat), but what a reader read is what is there now. Sound as `settle_racy`
    /// is: a write after this read changes the stat.
    fn settle_one(&mut self, path: &Path, e: Entry, now: i64) -> bool {
        let Some((kind, stat)) = Self::own_stat(path) else { return false };
        if kind != e.kind || stat != e.stat {
            return false;
        }
        let (digest, listing) = match kind {
            Kind::File => {
                let Ok(bytes) = std::fs::read(path) else { return false };
                self.counts.hashed += 1;
                self.counts.bytes += bytes.len() as u64;
                (Sha256::digest(&bytes).into(), None)
            }
            Kind::Dir => {
                let Some(l) = self.read_listing(path, false) else { return false };
                self.counts.listed += 1;
                (listing_digest(&l), Some(l))
            }
            _ => return false,
        };
        if digest == e.digest && Self::own_stat(path).is_some_and(|(_, s)| s == e.stat) {
            if e.stat.mtime_ns.max(e.stat.ctime_ns) < now - RACY_NS {
                self.remember(path, Entry { listing: listing.or(e.listing), recorded_ns: now, ..e });
            }
            return true;
        }
        false
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
    pub fn run_record(&mut self, key: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        let k = self.db.as_mut()?;
        if let Some(r) = k.records.get(key) {
            return Some(r.clone());
        }
        // Asked of the store once a process, outside the waves (decision 0104); inside, a record not asked before is
        // absent, and its run made again.
        if k.asked.contains(key) || !super::hazards::may_read_store() {
            return None;
        }
        k.asked.insert(key.to_owned());
        // A run's record holds bytes (`encode_lists`); a canonical form's, a text, is not asked here.
        let found: Option<(Vec<u8>, Vec<u8>)> = k.reader.as_ref()?.prepare_cached("SELECT consulted, outputs FROM run_record WHERE key = ?1").ok()?.query_row([key], |r| Ok((bytes_of(r.get(0)?), r.get(1)?))).ok().and_then(|(c, o)| Some((c?, o)));
        if let Some(r) = &found {
            k.records.insert(key.to_owned(), r.clone());
        }
        found
    }

    /// One dependency list per shape in a family of immutable records named
    /// `family:<shape hash>:<result key>`. Each seek skips the other values of that shape.
    pub fn run_record_samples(&self, family: &str) -> Vec<Vec<u8>> {
        if !super::hazards::may_read_store() { return Vec::new(); }
        let Some(db) = self.db.as_ref().and_then(|k| k.reader.as_ref()) else { return Vec::new() };
        let Ok(mut seek) = db.prepare_cached("SELECT key,consulted FROM run_record WHERE key >= ?1 AND key < ?2 ORDER BY key LIMIT 1")
            else { return Vec::new() };
        let mut lower = format!("{family}:");
        let upper = format!("{family};");
        let mut lists = Vec::new();
        while let Ok((key, list)) = seek.query_row((&lower, &upper), |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))) {
            let Some((group, _)) = key.rsplit_once(':') else { break };
            lower = format!("{group};");
            lists.push(list);
        }
        lists
    }

    /// Read an immutable result even if another process published it after an earlier miss.
    pub fn stored_run_record(&self, key: &str) -> Option<(Vec<u8>, Vec<u8>)> {
        if !super::hazards::may_read_store() { return None; }
        self.db.as_ref()?.reader.as_ref()?.prepare_cached("SELECT consulted,outputs FROM run_record WHERE key=?1").ok()?
            .query_row([key], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    }

    /// Keeps a run's record under `key`, replacing the last: in memory now, in the store behind the run.
    pub fn put_run_record(&mut self, key: &str, consulted: &[u8], outputs: &[u8]) {
        if let Some(k) = self.db.as_mut() {
            k.records.insert(key.to_owned(), (consulted.to_owned(), outputs.to_owned()));
        }
        self.store_run_record(key, consulted, outputs);
    }

    /// Queue a durable result. The store controls the lifetime of versioned results.
    pub fn store_run_record(&self, key: &str, consulted: &[u8], outputs: &[u8]) {
        if let Some(k) = self.db.as_ref() {
            let (key, consulted, outputs) = (key.to_owned(), consulted.to_owned(), outputs.to_owned());
            k.writer.send(move |db| {
                let _ = db.execute_cached("INSERT OR REPLACE INTO run_record VALUES (?1, ?2, ?3)", params![key, consulted, outputs]);
            });
        }
    }

    /// Ends a run: writes what changed since the last flush to the store, in one transaction. The outermost run's
    /// end also settles the roots the journal could not speak for, and ends the roots' answers: the next run asks
    /// again. A run inside a run (phase 0's, a batch's policies) leaves the outer run's roots asked: counting its end
    /// asked each of them again after every inner run.
    pub fn flush(&mut self) {
        if self.open <= 1 {
            self.settle_racy(now_ns());
        }
        self.write_dirty();
        self.open = self.open.saturating_sub(1);
        if self.open == 0 {
            self.settle_roots();
            // The run's output directories under a root it watched: kept for later processes (decision 0284).
            for dir in std::mem::take(&mut self.unreported) {
                if self.reported(&dir) {
                    self.record_own(dir);
                }
            }
            self.flushes += 1;
            // What this run wrote folded into the database (the writer's connection lives as long as the process, so
            // its close never does it, and the WAL stayed under SQLite's 1000 pages: 2.9 MB of history every process
            // read through). Passive: it waits for no reader; it commits nothing.
            if let Some(k) = &self.db {
                k.writer.send(|db| {
                    let _ = db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
                });
            }
        }
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
        super::hazards::warm_fault("the observer recorded answers", || {
            format!("{} paths (first {}), {} git aliases, {} canonical forms ({})", rows.len(), rows.first().map_or("-", |r| r.0.as_str()), aliases.len(), canon.len(), canon.iter().map(|(p, c)| format!("{p} -> {c}")).collect::<Vec<_>>().join(", "))
        });
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
                // With the time it was recorded: collection keeps it while young, though no stored root holds it yet.
                let Ok(mut put_canon) = tx.prepare_cached("INSERT OR REPLACE INTO run_record VALUES (?1, ?2, ?3)") else { return };
                let at = now_ns().to_le_bytes().to_vec();
                for (key, c) in &canon {
                    let _ = put_canon.execute(params![key, c, at]);
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
            super::hazards::warm_fault("the observer settled a root the journal could not speak for", || {
                format!("{}: {} answers and {} canonical forms forgotten, cursor {}", root.display(), paths.len(), keys.len(), if at.is_some() { "stored" } else { "none (no volume)" })
            });
            if let Some((key, cursor)) = &at {
                k.meta.insert(key.clone(), rusqlite::types::Value::Text(cursor.clone()));
                k.stored.insert(key.clone(), cursor.clone());
            }
            k.deleted = true;
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
                        let _ = tx.execute_cached("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![key, cursor]);
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
    /// The runs' records asked so far (`run_record`) or kept this process, and the keys asked of the store.
    records: HashMap<String, (Vec<u8>, Vec<u8>)>,
    asked: HashSet<String>,
    /// The store, read only, for what is read as it is first needed (`ensure`, `run_record`).
    reader: Option<Connection>,
    /// The parts of the tree whose answers and canonical forms have been read (`load_under`), and whether a delete
    /// was sent to the writer since the last read: a part read then waits for it, or would read back what was
    /// forgotten.
    loaded: Vec<PathBuf>,
    deleted: bool,
    meta: HashMap<String, rusqlite::types::Value>,
    /// Each root's cursor as the store holds it (`meta` holds memory's, which moves on ahead of it): how far behind
    /// the journal the store's is (`CURSOR_AGE`).
    stored: HashMap<String, String>,
    writer: Writer,
}

/// A cursor as the journal's text (an FSEvents id, a Watchman clock); one kept as a number is an id.
fn text_cursor(v: &rusqlite::types::Value) -> Option<String> {
    match v {
        rusqlite::types::Value::Text(t) => Some(t.clone()),
        rusqlite::types::Value::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

/// How many events a stored cursor may fall behind while the journal names nothing under its root: it is then
/// stored again (a timestamp, `profile::WARM_WRITES_ALLOWED`), at most once in this many events. A replay from a
/// cursor 2 million events old cost nothing measurable, from one 7 million old 0.6 to 2 s for its first ask.
const CURSOR_AGE: u64 = 1 << 20;

/// The most answers a root may hold and still be checked by its directories' listings rather than the journal
/// (decision 0400, `RERAIL_LIST_MAX`).
const LIST_MAX: usize = 20_000;

/// How many of the observer's own journal's generations collection keeps (decision 0400): a record from before
/// them is not answered (`changed_since` gives `None`), and is made again.
const LIST_KEEP: u64 = 1 << 16;

/// The observer's own journal's last generation and the oldest it answers from, in `observer_meta`.
const LISTING_GEN: &str = "listing:generation";
const LISTING_FLOOR: &str = "listing:floor";

/// A cursor of the observer's own journal: `L` and its generation.
fn listing_cursor(text: &str) -> Option<u64> {
    text.strip_prefix('L')?.parse().ok()
}

/// `LIST_MAX`, or `RERAIL_LIST_MAX`.
fn list_max() -> usize {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_LIST_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(LIST_MAX))
}

/// How many paths the journal may name under a root, none of them an answer kept, while its stored cursor stays: a
/// replay from the stored cursor names them all again each run, so past this many it is stored again (a timestamp,
/// `profile::WARM_WRITES_ALLOWED`).
const CURSOR_PATHS: usize = 1024;

/// How long collection keeps an answer or canonical form no stored root holds (`collect`): longer than a run lasts, so
/// the run that recorded it has settled its roots first.
const UNHELD_GRACE_NS: i64 = 3600 * 1_000_000_000;

/// The store at `path` opened to read (`None`: not made yet), with what is read as the observer opens: its roots'
/// cursors and settings, returned, and the Git aliases (blob to sha256), into `aliases`. The rest is read as it is
/// first needed (`Observer::ensure`, `run_record`).
type Meta = HashMap<String, rusqlite::types::Value>;
fn read_open(path: &Path, aliases: &mut HashMap<Vec<u8>, [u8; 32]>) -> (Option<Connection>, Meta) {
    let mut meta = HashMap::new();
    let Ok(db) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return (None, meta) };
    if let Ok(mut st) = db.prepare("SELECT blob, digest FROM git_alias")
        && let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)))
    {
        for (blob, d) in rows.flatten() {
            if let Ok(d) = d.try_into() {
                aliases.insert(blob, d);
            }
        }
    }
    if let Ok(mut st) = db.prepare("SELECT key, value FROM observer_meta")
        && let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, rusqlite::types::Value>(1)?)))
    {
        meta.extend(rows.flatten());
    }
    (Some(db), meta)
}

/// A write the writer runs on its connection.
type WriterJob = Box<dyn FnOnce(&mut Connection) + Send>;

/// An observer's writer: a thread of its own with its own connection, behind the run (decision 0104), running each
/// write in the order it was queued. Dropped, it finishes what was queued.
struct Writer {
    jobs: Option<std::sync::mpsc::Sender<WriterJob>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    fn open(path: &Path) -> Writer {
        let (jobs, queued) = std::sync::mpsc::channel::<Box<dyn FnOnce(&mut Connection) + Send>>();
        let path = path.to_owned();
        let thread = rerail_accel::exec::service("rerail-observer-writer", None, move || {
                super::hazards::behind_the_run();
                let db = (|| {
                    // The answers name the user's files, their stats and contents' digests: private to the user, as
                    // the store is (docs/privacy.md). Made so before SQLite creates it, whose side files take its mode.
                    super::private_files::prepare(&path).ok()?;
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

/// The prefix of a directory runs wrote their outputs into, as `observer_meta` keeps it (`own_output`).
const OWN_OUTPUT: &str = "own:";

/// rerail's own files the machine names (decision 0284, `own`): the cost statistics' file, the machine's (decision
/// 0442).
#[derive(Default)]
struct OwnFiles(Mutex<Vec<PathBuf>>);

/// Names `path` as one of rerail's own files for every observer on the machine (decision 0284): it, what is under
/// it, and every file named after it beside it (`<path>-wal`, `<path>.upkeep`) are outside the observer.
pub fn own(path: &Path) {
    let held = rerail_prof::code_rule_dsl::logic::profile::machine::<OwnFiles>();
    let mut own = held.0.lock().unwrap_or_else(|e| e.into_inner());
    for p in spellings(path) {
        if !own.contains(&p) {
            own.push(p);
        }
    }
}

/// `path` absolute, and with its directory's canonical form (the journal names paths by their real names).
fn spellings(path: &Path) -> Vec<PathBuf> {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let real = abs.parent().zip(abs.file_name()).and_then(|(dir, name)| Some(dunce::canonicalize(dir).ok()?.join(name)));
    std::iter::once(abs.clone()).chain(real.filter(|r| *r != abs)).collect()
}

/// Whether `path` is `own`'s (decision 0284): `own` or under it, or a file named after it beside it (`<own>-wal`,
/// `<own>.files`, `<own>.files.cookies/…`) or under one.
fn owned_by(own: &Path, path: &Path) -> bool {
    if path.starts_with(own) {
        return true;
    }
    let (Some(dir), Some(name)) = (own.parent(), own.file_name()) else { return false };
    let Some(first) = path.strip_prefix(dir).ok().and_then(|rest| rest.components().next()) else { return false };
    let (first, name) = (first.as_os_str().as_encoded_bytes(), name.as_encoded_bytes());
    first.len() > name.len() && first.starts_with(name) && matches!(first[name.len()], b'.' | b'-')
}

/// Whether `path` is `root` or under it, as `Path::starts_with` answers, by its bytes: a root's prefix ending at a
/// separator. `Path::starts_with` parses both into components at each call, and the observer asks it of each root for
/// each path several times (on PostHog its scans were most of the main thread's own time in phase 1's loads, notes/t40).
/// Paths whose components would normalize away bytes (`//`, `/.`, a trailing `/`) are asked of `Path::starts_with`.
fn under(path: impl AsRef<Path>, root: impl AsRef<Path>) -> bool {
    let (path, root) = (path.as_ref(), root.as_ref());
    let (p, r) = (path.as_os_str().as_encoded_bytes(), root.as_os_str().as_encoded_bytes());
    let odd = |b: &[u8]| b.windows(2).any(|w| w == b"//" || w == b"/.") || (b.len() > 1 && b.ends_with(b"/"));
    if odd(p) || odd(r) || r.is_empty() || p.first() == Some(&b'.') {
        return path.starts_with(root);
    }
    p.starts_with(r) && (p.len() == r.len() || p[r.len()] == b'/' || r == b"/")
}

/// Whether `path` has no `.` or `..` component: its last name is a name in its directory's listing.
/// Each of `paths`' `realpath`, a directory at a time (decision 0300 §2): a directory resolved once, and a file in it
/// is its canonical form and the file's name where its listing holds that name as given (the case on disk: macOS's
/// `realpath` gives that) and the entry is no symlink. Any other path is resolved on its own. One `realpath` a file
/// was 8 thread-seconds of `getattrlist` over PostHog's 29,885 files on a cold store, all at once in the kernel.
pub fn realpaths(paths: &[&Path]) -> Vec<Option<PathBuf>> {
    use rayon::prelude::*;
    let mut by_dir: HashMap<&Path, Vec<usize>> = HashMap::new();
    let mut alone: Vec<usize> = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        match (p.is_absolute() && plain(p)).then(|| p.parent().zip(p.file_name())).flatten() {
            Some((dir, _)) => by_dir.entry(dir).or_default().push(i),
            None => alone.push(i),
        }
    }
    let dirs: Vec<(&Path, Vec<usize>)> = by_dir.into_iter().collect();
    let resolved: Vec<Vec<(usize, Option<PathBuf>)>> = rerail_prof::code_rule_dsl::logic::profile::on_cores(|| {
        dirs
            .par_iter()
            .map(|(dir, files)| {
                let canonical = dunce::canonicalize(dir).ok();
                // The names as on disk, and which are symlinks (the listing's entry type: no stat).
                let names: HashMap<std::ffi::OsString, bool> = match (&canonical, std::fs::read_dir(dir)) {
                    (Some(_), Ok(entries)) => entries.flatten().map(|e| (e.file_name(), e.file_type().is_ok_and(|t| t.is_symlink()))).collect(),
                    _ => HashMap::new(),
                };
                files
                    .iter()
                    .map(|&i| {
                        let name = paths[i].file_name().expect("a file name");
                        let found = match (&canonical, names.get(name)) {
                            (Some(c), Some(false)) => Some(c.join(name)),
                            _ => dunce::canonicalize(paths[i]).ok(),
                        };
                        (i, found)
                    })
                    .collect()
            })
            .collect()
    });
    let mut out: Vec<Option<PathBuf>> = vec![None; paths.len()];
    for (i, c) in resolved.into_iter().flatten() {
        out[i] = c;
    }
    for i in alone {
        out[i] = dunce::canonicalize(paths[i]).ok();
    }
    out
}

fn plain(path: &Path) -> bool {
    !path.components().any(|c| matches!(c, std::path::Component::CurDir | std::path::Component::ParentDir))
}

/// Where a root's journal cursor is kept (`observer_meta`): `journal:<volume>:<root>`.
fn cursor_key(root: &Path, volume: [u8; 16]) -> String {
    format!("journal:{}:{}", volume.iter().map(|b| format!("{b:02x}")).collect::<String>(), root.display())
}

/// Where the observer of the store at `path` writes the journal's cookies: `<store>.cookies`, holding nothing else.
fn cookie_dir(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".cookies");
    PathBuf::from(name)
}

/// What `collect` dropped from an observer's store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub cursors: usize,
    pub answers: usize,
    pub records: usize,
    /// Output directories recorded as rerail's own that are gone (decision 0284).
    pub outputs: usize,
    /// Rows of the observer's own journal under no root kept, or older than the generations it keeps (decision 0400).
    pub changes: usize,
}

/// A journal root as `collect` reads it: its key, its volume, the root, its directory's real path, and its cursor (its
/// kind, 0 an FSEvents id or 1 a listing generation, and its number).
type JournalRoot = (String, String, PathBuf, Option<PathBuf>, Option<(u8, u64)>);
/// A journal root kept: its volume, the root, its directory's real path, and its cursor.
type KeptRoot = (String, PathBuf, PathBuf, Option<(u8, u64)>);

/// Collects the observer's store at `path` (`store_beside`), with the content store's collection (decision 0053),
/// behind the run: nothing ever left it before, and every process reads all of it as it opens. Dropped:
///
/// - a root's cursor where the root's name has a `.` or `..`, or is gone, or lies under another root kept; of two
///   names of one directory (`/var/…`, `/private/var/…`), all but the one with the newest cursor;
/// - an answer, a canonical form or a Git file list under no root kept: every path the observer reads is under a
///   root (`watch_above`), so one under none is of a name no run uses;
/// - a revision constant after the last manifest that owns its derivation goes away;
/// - other planned records (including constants, cone rounds and cutoffs) once none of their plans is live.
/// - a row of the observer's own journal (decision 0400) under no root kept, or `LIST_KEEP` generations old: the
///   floor moves up with it, and a record from before it is not answered (`changed_since`).
///
/// Then the WAL is folded into the database and truncated.
pub fn collect(path: &Path, live: impl Fn(&[u8; 32]) -> bool) -> rusqlite::Result<Swept> {
    let mut swept = Swept::default();
    if !path.exists() {
        return Ok(swept);
    }
    let mut db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(10))?;
    // The roots, by volume: (key, root, its directory's real path, cursor: its kind, 0 an FSEvents id, 1 a listing
    // generation, and its number).
    let mut roots: Vec<JournalRoot> = {
        let mut st = db.prepare("SELECT key, value FROM observer_meta WHERE key LIKE 'journal:%'")?;
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, rusqlite::types::Value>(1)?)))?
            .flatten()
            .filter_map(|(key, value)| {
                let mut parts = key.splitn(3, ':');
                let (_, volume, root) = (parts.next()?, parts.next()?.to_owned(), PathBuf::from(parts.next()?));
                let real = plain(&root).then(|| dunce::canonicalize(&root).ok()).flatten();
                let cursor = text_cursor(&value).and_then(|c| c.parse().ok().map(|n| (0, n)).or_else(|| listing_cursor(&c).map(|g| (1, g))));
                Some((key, volume, root, real, cursor))
            })
            .collect()
    };
    // Newest cursor first: of two names of one directory, one whose cursor is older than the other's is no longer
    // asked, and is dropped. Two asked by one run (a root as given and through a symlink, as `/var` and
    // `/private/var` on macOS, its files read by their canonical paths too, `Observer::canonical`) have the same cursor
    // and are both kept: dropped, the next run that asks it stores its cursor again, a write in a warm run (Q243).
    roots.sort_by_key(|r| std::cmp::Reverse(r.4));
    let mut kept: Vec<KeptRoot> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for (key, volume, root, real, cursor) in &roots {
        let older = |c: &Option<(u8, u64)>| matches!((c, cursor), (Some((k, n)), Some((kc, nc))) if k == kc && nc < n);
        match real {
            Some(real) if !kept.iter().any(|(v, _, r, c)| v == volume && r == real && older(c)) => kept.push((volume.clone(), root.clone(), real.clone(), *cursor)),
            _ => dropped.push(key.clone()),
        }
    }
    // A root under another kept is asked as part of it (`watch_above` takes the widest).
    let under: Vec<usize> = (0..kept.len()).filter(|&i| kept.iter().enumerate().any(|(j, (v, _, r, _))| j != i && *v == kept[i].0 && kept[i].2.starts_with(r) && kept[i].2 != *r)).collect();
    for &i in under.iter().rev() {
        let (volume, root, _, _) = kept.remove(i);
        dropped.push(cursor_key_text(&volume, &root));
    }
    let held = |path: &Path| plain(path) && kept.iter().any(|(_, root, _, _)| path.starts_with(root));
    // What a run recorded lately is kept though no root holds it: a root a run first asks has no cursor stored until
    // that run's end (`settle_roots`), and collection may run before it, in the same process or another.
    let young = |recorded_ns: i64| now_ns().saturating_sub(recorded_ns) < UNHELD_GRACE_NS;
    let answers: Vec<String> = {
        let mut st = db.prepare("SELECT path, recorded_ns FROM observed")?;
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.flatten().filter(|(p, at)| !held(Path::new(p)) && !young(*at)).map(|(p, _)| p).collect()
    };
    let records: Vec<String> = {
        let mut st = db.prepare("SELECT key, consulted, outputs FROM run_record")?;
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, bytes_of(r.get(1)?).unwrap_or_default(), bytes_of(r.get(2)?).unwrap_or_default())))?
            .flatten()
            .filter(|(key, consulted, outputs)| {
                // A canonical form's record holds the time it was recorded (`write_dirty`).
                if let Some(path) = key.strip_prefix(CANONICAL) {
                    let at = outputs.as_slice().try_into().map_or(0, i64::from_le_bytes);
                    return !held(Path::new(path)) && !young(at);
                }
                if let Some(path) = key.strip_prefix("git-files:") {
                    return !held(Path::new(path));
                }
                // A record of what a run kept goes with its plans (decision 0252): a whole-run cutoff's consulted list
                // (`reuse::encode_consulted`), the constants' too, and the cone's rounds' third list: kind, path and
                // digest by turns, a plan's path empty and its digest the plan's in hex. One that names none is from
                // before they did.
                if key.starts_with(CUTOFF) || PLANNED.iter().any(|p| key.starts_with(p)) {
                    let lists = decode_lists(consulted).unwrap_or_default();
                    // A revision's constants are kept by their owner's plan (`constants::keep_revision`).
                    if key.starts_with("constants:") && let Some(owner) = lists.iter().flat_map(|l| l.as_chunks::<3>().0).find(|c| c[0] == "constants-owner") {
                        return !unhex(&owner[2]).is_some_and(|plan| live(&plan));
                    }
                    return !lists.iter().flat_map(|l| l.as_chunks::<3>().0).filter(|c| c[0] == "plan" && c[1].is_empty()).filter_map(|c| unhex(&c[2])).any(|plan| live(&plan));
                }
                // A cutoff's record from before they were prefixed: its key alone, a digest.
                key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .map(|(key, _, _)| key)
            .collect()
    };
    if std::env::var_os("RERAIL_OBSERVE_TRACE").is_some() {
        eprintln!(
            "rerail: observer collect: roots {:?}; kept {:?}; cursors dropped {dropped:?}; {} answers dropped (first {:?}); {} records dropped (first {:?})",
            roots.iter().map(|r| (&r.2, r.3.is_some(), r.4)).collect::<Vec<_>>(),
            kept.iter().map(|k| &k.1).collect::<Vec<_>>(),
            answers.len(),
            answers.iter().take(3).collect::<Vec<_>>(),
            records.len(),
            records.iter().take(3).collect::<Vec<_>>()
        );
    }
    // An output directory recorded as rerail's own (`own_output`) that is gone: nothing there to leave out.
    let outputs: Vec<String> = {
        let mut st = db.prepare(&format!("SELECT key FROM observer_meta WHERE key LIKE '{OWN_OUTPUT}%'"))?;
        st.query_map([], |r| r.get::<_, String>(0))?.flatten().filter(|k| k.strip_prefix(OWN_OUTPUT).is_some_and(|d| !Path::new(d).exists())).collect()
    };
    // The observer's own journal: its rows under no root kept, and those older than the generations it keeps.
    let generation: i64 = db.query_row("SELECT value FROM observer_meta WHERE key = ?1", [LISTING_GEN], |r| r.get(0)).unwrap_or(0);
    let floor = generation.saturating_sub(LIST_KEEP as i64);
    let changes: Vec<String> = match db.prepare("SELECT path, gen FROM observed_changes") {
        Ok(mut st) => st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.flatten().filter(|(p, g)| *g < floor || !held(Path::new(p))).map(|(p, _)| p).collect(),
        Err(_) => Vec::new(),
    };
    let tx = db.transaction()?;
    for path in &changes {
        swept.changes += tx.execute("DELETE FROM observed_changes WHERE path = ?1", [path])?;
    }
    if floor > 0 {
        tx.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![LISTING_FLOOR, floor])?;
    }
    for key in &dropped {
        swept.cursors += tx.execute("DELETE FROM observer_meta WHERE key = ?1", [key])?;
    }
    for key in &outputs {
        swept.outputs += tx.execute("DELETE FROM observer_meta WHERE key = ?1", [key])?;
    }
    for path in &answers {
        swept.answers += tx.execute("DELETE FROM observed WHERE path = ?1", [path])?;
    }
    for key in &records {
        swept.records += tx.execute("DELETE FROM run_record WHERE key = ?1", [key])?;
    }
    tx.commit()?;
    // The WAL folded in and emptied: a process that opens the store reads the database, not its history.
    db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(swept)
}

/// Lists of texts as bytes, for a record the store keeps (a run's consulted list, the cone's rounds, the Git files):
/// the lists' count, then each list's count and each text's length and bytes (little-endian `u32`s). Read back by
/// `decode_lists`, never as text.
pub fn encode_lists<S: AsRef<str>>(lists: &[&[S]]) -> Vec<u8> {
    let mut out = (lists.len() as u32).to_le_bytes().to_vec();
    for list in lists {
        out.extend((list.len() as u32).to_le_bytes());
        for t in *list {
            let t = t.as_ref();
            out.extend((t.len() as u32).to_le_bytes());
            out.extend(t.as_bytes());
        }
    }
    out
}

/// The lists `encode_lists` made, or none where the bytes are not some (a record of another form, or cut).
pub fn decode_lists(bytes: &[u8]) -> Option<Vec<Vec<String>>> {
    // The `u32` at `*at`, past it.
    fn word(bytes: &[u8], at: &mut usize) -> Option<usize> {
        let w = u32::from_le_bytes(bytes.get(*at..*at + 4)?.try_into().ok()?) as usize;
        *at += 4;
        Some(w)
    }
    let mut at = 0;
    let n = word(bytes, &mut at)?;
    let mut lists = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        let k = word(bytes, &mut at)?;
        let mut list = Vec::with_capacity(k.min(1 << 16));
        for _ in 0..k {
            let len = word(bytes, &mut at)?;
            list.push(std::str::from_utf8(bytes.get(at..at + len)?).ok()?.to_owned());
            at += len;
        }
        lists.push(list);
    }
    (at == bytes.len()).then_some(lists)
}

/// A record's consulted column as its bytes: a run's are bytes (`encode_lists`), a canonical form's a text.
fn bytes_of(v: rusqlite::types::Value) -> Option<Vec<u8>> {
    match v {
        rusqlite::types::Value::Text(t) => Some(t.into_bytes()),
        rusqlite::types::Value::Blob(b) => Some(b),
        _ => None,
    }
}

/// The prefix of a whole-run cutoff's record (`reuse::CUTOFF`, in the engine).
const CUTOFF: &str = "cutoff:";

/// The prefixes of the other records that name the plans they are kept for (`plan` entries, as a cutoff's): the run's
/// constants (`corpus::constants`) and the cone's rounds (`corpus`, decision 0145).
const PLANNED: &[&str] = &["constants:", "cone-rounds:"];

/// `cursor_key`, of a volume given as its hex.
fn cursor_key_text(volume: &str, root: &Path) -> String {
    format!("journal:{volume}:{}", root.display())
}

/// A 32-byte digest from its hex (`hex`).
pub fn unhex(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in bytes.chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The P0 store beside a content store: `<content store>.files`.
pub fn store_beside(content_store: &Path) -> PathBuf {
    let mut name = content_store.as_os_str().to_owned();
    name.push(".files");
    PathBuf::from(name)
}

/// `Observer::digest_step`'s answer: the digest (`None`: not a readable file), or a file to read first.
pub enum DigestStep {
    Answered(Option<[u8; 32]>),
    Read(DigestRead),
}

/// A file `Observer::digest` reads: hashed with no observer (`hash`), then recorded (`Observer::digest_read`).
pub struct DigestRead {
    path: PathBuf,
    /// Git objects have no filesystem stat.
    stat: Option<Stat>,
    started: i64,
    /// The digest kept for it, which the read checks.
    known: Option<[u8; 32]>,
    /// Its clean Git blob's key, remembered with the digest.
    blob: Option<[u8; 40]>,
}

impl DigestRead {
    /// The file's length and bytes' digest; `None` when it cannot be read.
    pub fn hash(&self) -> Option<(u64, [u8; 32])> {
        self.read().map(|bytes| Self::hashed(&bytes))
    }

    /// The file's bytes, for a caller that keeps them (a run's one read of its sources, decision 0300).
    pub fn read(&self) -> Option<Vec<u8>> {
        source::read(&self.path).ok()
    }

    /// `bytes`' length and digest, as `hash` gives a file's.
    pub fn hashed(bytes: &[u8]) -> (u64, [u8; 32]) {
        (bytes.len() as u64, Sha256::digest(bytes).into())
    }

    /// The file's stat as it was before the read: the bytes are its content while the file's stat is this.
    pub fn stat(&self) -> Option<&Stat> {
        self.stat.as_ref()
    }

    /// Git placements with this key can share an immutable read.
    pub fn blob_key(&self) -> Option<[u8; 40]> {
        self.blob
    }
}

/// The shared object's observers, one per store (decision 0442).
#[derive(Default)]
struct Observers(Mutex<HashMap<Option<PathBuf>, Observer>>);

/// Waits until every observer's queued writes are in its store: the run's end, beside the content store's writer.
pub fn written() {
    if let Some(all) = rerail_prof::code_rule_dsl::logic::profile::shared_object().and_then(|shared| shared.get_if_any::<Observers>()) {
        all.0.lock().unwrap_or_else(|e| e.into_inner()).values().for_each(Observer::written);
    }
}

/// Drops the observer of the P0 store at `store`, its queued writes written first: its store is being deleted (a
/// test's own). Every run's end waits on each observer the process holds (`written`), so one kept past its store's
/// life is a cost to every later run.
pub fn forget(store: &Path) {
    let gone = rerail_prof::code_rule_dsl::logic::profile::shared::<Observers>().0.lock().unwrap_or_else(|e| e.into_inner()).remove(&Some(store.to_path_buf()));
    if let Some(observer) = gone {
        observer.written();
    }
}

/// Runs `f` with the observer of the P0 store at `store` (`None`: in memory, for the shared object). One observer per
/// store, for the shared object.
pub fn with<T>(store: Option<&Path>, f: impl FnOnce(&mut Observer) -> T) -> T {
    let all = rerail_prof::code_rule_dsl::logic::profile::shared::<Observers>();
    let mut all = all.0.lock().unwrap_or_else(|e| e.into_inner());
    let key = store.map(Path::to_path_buf);
    let observer = all.entry(key.clone()).or_insert_with(|| Observer::open(key.as_deref()));
    f(observer)
}

/// A hex digest, as records keep them.
pub fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the observer asks the file system's journal (decision 0105): on unless `RERAIL_JOURNAL=off` (a run's
/// setting). A test build may turn it off or on for its process (`set_journal`): the engine's tests do (each run of a
/// test waits for its cookie's event, up to `journal`'s `WAIT`, and a test makes many runs), and the journal's own
/// tests here keep it on.
fn journal_on() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    match JOURNAL.load(std::sync::atomic::Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    rerail_prof::code_rule_dsl::logic::profile::setting(|| !std::env::var("RERAIL_JOURNAL").is_ok_and(|v| v == "off"))
}

/// A test build's choice of the journal (`set_journal`): 0 none (the setting's), 1 on, 2 off.
#[cfg(any(test, feature = "test-hooks"))]
static JOURNAL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Turns the observer's use of the journal on or off for a test build's process, whatever `RERAIL_JOURNAL` says.
#[cfg(any(test, feature = "test-hooks"))]
pub fn set_journal(on: bool) {
    JOURNAL.store(if on { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
}

/// The least the observers made after this wait for the journal ([`JOURNAL_WAIT_LEAST_NS`] until set): a test that
/// asks for the journal's answer whatever it costs raises it (Q221).
/// On the shared object (decision 0442): set by its owner, read by the observers its runs make.
struct JournalWaitLeast(std::sync::atomic::AtomicU64);
impl Default for JournalWaitLeast {
    fn default() -> Self {
        JournalWaitLeast(std::sync::atomic::AtomicU64::new(JOURNAL_WAIT_LEAST_NS))
    }
}

fn journal_wait_least() -> u64 {
    rerail_prof::code_rule_dsl::logic::profile::shared::<JournalWaitLeast>().0.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_journal_wait_least(wait: std::time::Duration) {
    rerail_prof::code_rule_dsl::logic::profile::shared::<JournalWaitLeast>().0.store(wait.as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long a test's journal may take to answer (`journal_observer`).
    const JOURNAL_TEST_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

    /// An observer that asks the journal for every root (decision 0105), as these tests were written for: no root
    /// is checked by its listings (decision 0400), whose tests open `Observer::open`.
    fn journal_observer(store: &Path) -> Observer {
        let mut o = Observer::open(Some(store));
        o.list_max = 0;
        // The journal's answer, however late: on a loaded host `fseventsd` was seen to deliver a stream's cookie, or
        // to flush the run's marker into its history, more than 5 s late (Q223), and the root was then `Lost`, checked
        // by its `stat`s. What the tests check is the journal's answer.
        o.journal_settle = JOURNAL_TEST_BUDGET;
        // Trust at once: these runs follow each other within the settle window, as no test can wait it out; and the
        // journal's answer whatever it costs: the cookie waited for as long as the replay may settle (a busy host's
        // `fseventsd` takes seconds, Q221).
        o.settle_ns = 0;
        o.wait_least_ns = 30_000_000_000;
        o
    }

    /// Whether the journal answered every root of the test's runs so far (`runs`, each run's counts; `Counts::late`).
    /// On a loaded host it may not (Q223): `fseventsd`, one thread at a full core, delivers the machine's file events
    /// in order and logs them as late, and a cookie or a marker waited behind them 20 s and more, past
    /// `JOURNAL_TEST_BUDGET`. The root was then checked by `stat` or its listings: the run's answers still hold and
    /// are checked; what the journal would have said is not, and the test says why. A late run reaches the next: its
    /// stream answers from its sync on, and the next run's question from the stored cursor before it is a replay of
    /// the history, which misses a change merged into a record older than the cursor (decision 0400), as a new
    /// process's would; a run covers that by its settle window, which these runs turn off (`journal_observer`).
    #[cfg(target_os = "macos")]
    fn journal_answered(runs: &[&Counts], what: &str) -> bool {
        let late: u64 = runs.iter().map(|c| c.late).sum();
        if late == 0 {
            return true;
        }
        eprintln!("{what}: the journal was behind for {late} roots of this test's runs (fseventsd behind the host's file events, Q223): checked by stat or listings, so the journal's own counts are not checked here ({})", runs.last().map(|c| c.to_string()).unwrap_or_default());
        false
    }

    /// `under` answers as `Path::starts_with` for every pair of a set of paths, odd spellings among them.
    #[test]
    fn under_answers_as_path_starts_with() {
        let paths = ["/", "/a", "/a/", "/a/b", "/ab", "/a/b/c", "/a//b", "/a/./b", "/a/.b", "a", "a/b", "ab", "./a", "", "/b/a"];
        for p in paths {
            for r in paths {
                assert_eq!(under(Path::new(p), Path::new(r)), Path::new(p).starts_with(r), "{p:?} under {r:?}");
            }
        }
    }

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
        let run = |files: &[PathBuf], settle: i64| {
            let mut o = journal_observer(&store);
            o.settle_ns = settle;
            o.begin();
            o.watch(std::slice::from_ref(&root));
            let listing = o.listing(&root.join("pkg")).unwrap();
            let digests: Vec<[u8; 32]> = files.iter().map(|f| o.digest(f).unwrap()).collect();
            o.flush();
            (o.counts, listing, digests)
        };
        let (first, listing, digests) = run(&files, 0);
        assert!(first.stats >= 6 && first.trusted == 0, "the first run checks every path: {first}");
        // The setup's writes may be named once more (their events may come after the first run's cursor).
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (middle, _, _) = run(&files, 0);
        let (second, again, same) = run(&files, 0);
        assert_eq!((again, same), (listing, digests.clone()));
        if journal_answered(&[&first, &middle, &second], "nothing changed") {
            assert_eq!((second.stats, second.trusted), (0, 6), "nothing changed: no stat ({second})");
        }
        std::fs::write(&files[2], "x = 2\n").unwrap();
        aged(&files[2]);
        let (third, _, mut after) = run(&files, 0);
        if journal_answered(&[&first, &middle, &second, &third], "an edit") {
            assert!(third.forgotten >= 1 && third.hashed == 1, "the edited file is read again: {third}");
        } else {
            // As a run asks, its settle window on (`journal_answered`): the edit is read whatever the journal said.
            after = run(&files, JOURNAL_SETTLE_NS).2;
        }
        assert_ne!(after[2], digests[2], "the edit is read");
        assert_eq!([after[0], after[1], after[3], after[4]], [digests[0], digests[1], digests[3], digests[4]]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Q219: a root watched through `/./` (`…/q179/./work/r3/corpus-dl`, its files read by that spelling) is told an
    /// edit under it, as a root spelled plainly is: the edited file is read again, the others trusted.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_root_watched_through_a_dot_reads_an_edit_under_it() {
        let base = std::env::temp_dir().join(format!("rerail-watch-dot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("work/root/pkg")).unwrap();
        let root = base.join(".").join("work/root");
        assert!(root.to_str().unwrap().contains("/./"), "{root:?}");
        let files: Vec<PathBuf> = (0..3).map(|i| root.join(format!("pkg/m{i}.py"))).collect();
        for f in &files {
            std::fs::write(f, "x = 1\n").unwrap();
            aged(f);
        }
        let store = base.join("store.files");
        let run = || {
            let mut o = journal_observer(&store);
            o.begin();
            o.watch(std::slice::from_ref(&root));
            let digests: Vec<[u8; 32]> = files.iter().map(|f| o.digest(f).unwrap()).collect();
            o.flush();
            (o.counts, digests)
        };
        let (_, digests) = run();
        std::thread::sleep(std::time::Duration::from_millis(300));
        run();
        let (second, same) = run();
        assert_eq!(same, digests);
        std::fs::write(&files[1], "x = 2\n").unwrap();
        aged(&files[1]);
        let (third, after) = run();
        assert_ne!(after[1], digests[1], "the edit under the dotted root is read ({second}, then {third})");
        assert_eq!([after[0], after[2]], [digests[0], digests[2]]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Q189: FSEvents' history can merge a change made just after a run read a path into the path's earlier event,
    /// replayed under an id before the run's cursor: the next run is told nothing. An answer recorded within the
    /// settle window is then checked by `stat`, and the edit read; with no window (as before) it was missed. The merge
    /// is made here by moving the root's stored cursor past the edit's event, which is what the replay amounts to.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Q293: since 0400 a replay answers by arrival, so moving the cursor past the edit no longer hides it (the stream names it); the first leg's miss cannot happen"]
        fn an_answer_recorded_within_the_settle_window_is_checked_though_the_journal_names_nothing() {
        let base = std::env::temp_dir().join(format!("rerail-settle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("m.py");
        std::fs::write(&file, "x = 1\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let run = |settle: i64| {
            let mut o = Observer::open(Some(&store));
            // The journal's path (Q189): a root this small is otherwise checked by its listings (decision 0400).
            o.list_max = 0;
            o.journal_settle = JOURNAL_TEST_BUDGET;
            o.settle_ns = settle;
            o.wait_least_ns = 30_000_000_000;
            o.begin();
            o.watch(std::slice::from_ref(&root));
            let digest = o.digest(&file).unwrap();
            o.flush();
            o.written();
            (o.counts, digest)
        };
        // Recorded, then asked again so the root holds a cursor and the answer is kept.
        let (recorded, first) = run(0);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (asked, _) = run(0);
        // The edit, then the cursor moved past its event: the journal names nothing since.
        let edit_and_hide = |text: &str| {
            let before = super::super::journal::current_id();
            std::fs::write(&file, text).unwrap();
            aged(&file);
            let volume = super::super::journal::volume(&root).expect("a volume");
            // An id past the edit's events, which a loaded host may give later than the edit (Q223): a live stream's
            // sync, so a question from it is told what came after that sync alone, as a merged replay tells it.
            let Some(id) = super::super::journal::id_past(&root, before, &cookie_dir(&store)) else {
                eprintln!("the journal gave no id past the edit (later than its wait: fseventsd behind, Q223): the edit cannot be hidden");
                return false;
            };
            let db = Connection::open(&store).unwrap();
            db.execute("INSERT OR REPLACE INTO observer_meta VALUES (?1, ?2)", params![cursor_key(&root, volume), id.to_string()]).unwrap();
            // As the next process: its replay starts at the stored cursor. This process's stream saw the edit come,
            // and answers by when events came (decision 0400), so it would name the edit whatever the cursor.
            super::super::journal::forget_streams();
            true
        };
        if !edit_and_hide("x = 22\n") {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let (missed, stale) = run(0);
        // The miss shows only where the journal names nothing after the hidden cursor: its replay may name the edit
        // anyway (its event, merged into the path's earlier one, told after the sync's cookie: Q189), and the run
        // then reads it, which is the safe side. The window's check, below, holds either way.
        if !journal_answered(&[&recorded, &asked, &missed], "the journal's silence") {
        } else if missed.forgotten > 0 {
            eprintln!("the journal named the edit after the hidden cursor (its event told after the sync's cookie): no miss to show here ({missed})");
        } else {
            assert_eq!((stale, missed.hashed), (first, 0), "with no window the journal's silence is trusted: the edit is missed ({missed})");
        }
        if !edit_and_hide("x = 333\n") {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let (checked, fresh) = run(JOURNAL_SETTLE_NS);
        assert!(checked.stats >= 1 && checked.hashed == 1, "within the window the answer is checked and the edit read: {checked}");
        assert_ne!(fresh, first);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Decision 0284: rerail's own files are outside the observer. Under a watched root: the content store and every
    /// file named after it (its WAL, the observer's store `<store>.files` and its cookies, the lock and upkeep files),
    /// and a directory a run wrote its outputs into (recorded by the run that did, `own_output`, for later processes).
    /// A listing leaves them out, no answer is kept for them, and the journal's news of them forgets nothing and moves
    /// no stored cursor: a run after the writes of rerail's own is as warm as one after none.
    #[cfg(target_os = "macos")]
    #[test]
    fn rerails_own_files_are_outside_the_observer() {
        let base = std::env::temp_dir().join(format!("rerail-own-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("root/pkg")).unwrap();
        let root = dunce::canonicalize(base.join("root")).unwrap();
        let source = root.join("pkg/m.py");
        std::fs::write(&source, "x = 1\n").unwrap();
        aged(&source);
        let content = root.join("store.sqlite");
        let out = root.join("out");
        // What a run writes of its own: the store and the files beside it, and its outputs.
        let own_writes = |n: u32| {
            std::fs::write(&content, format!("store {n}")).unwrap();
            std::fs::write(root.join("store.sqlite-wal"), format!("wal {n}")).unwrap();
            std::fs::write(root.join("store.sqlite.upkeep"), format!("frames\t{n}\n")).unwrap();
            std::fs::create_dir_all(&out).unwrap();
            std::fs::write(out.join("rr_diagnostic.csv"), format!("{n}\n")).unwrap();
        };
        own_writes(0);
        let store = store_beside(&content);
        let cursor = || -> Vec<String> {
            let db = Connection::open(&store).unwrap();
            db.prepare("SELECT value FROM observer_meta WHERE key LIKE 'journal:%'").unwrap().query_map([], |r| r.get::<_, rusqlite::types::Value>(0)).unwrap().flatten().filter_map(|v| text_cursor(&v)).collect()
        };
        let run = |first: bool, n: u32| {
            let mut o = journal_observer(&store);
            o.begin();
            if first {
                o.own_output(&out);
            }
            let marked = o.mark(std::slice::from_ref(&root)).unwrap_or_default();
            let names: Vec<String> = o.listing(&root).unwrap().iter().map(|(n, _)| n.clone()).collect();
            o.digest(&source).unwrap();
            assert!(o.digest(&content).is_some(), "an own file is still answered");
            let kept = [&content, &out.join("rr_diagnostic.csv")].iter().filter(|p| o.entries.contains_key(p.as_path())).count();
            own_writes(n);
            o.flush();
            o.written();
            (o.counts, names, kept, marked)
        };
        let (one, names, kept, _) = run(true, 1);
        assert_eq!(names, ["pkg"], "a listing leaves rerail's own files out");
        assert_eq!(kept, 0, "no answer is kept for an own file");
        // The setup's writes may be named once more.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (two, _, _, marked) = run(false, 2);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let before = cursor();
        let (third, names, kept, _) = run(false, 3);
        assert_eq!((names.as_slice(), kept), (["pkg".to_owned()].as_slice(), 0));
        if journal_answered(&[&one, &two, &third], "own files") {
            assert_eq!((third.forgotten, third.stats), (0, 0), "the journal's news of own files forgets nothing: {third}");
            assert_eq!(cursor(), before, "and moves no stored cursor: nothing written");
        }
        // A record made in the second run holds: what changed since under its root is rerail's own.
        let mut o = journal_observer(&store);
        o.begin();
        let since = o.changed_since("rec", std::slice::from_ref(&root), &marked);
        if journal_answered(&[&one, &two, &third, &o.counts], "a record's changes") {
            assert_eq!(since, Some(Vec::new()));
        }
        o.flush();
        o.written();
        drop(o);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Decision 0284: an output directory is recorded once, and only where a later run's journal reports from: under a
    /// root the store has a cursor for. One under a directory already recorded, or outside every such root (phase 0's
    /// scratch outputs in the temporary directory), is rerail's own for the process and written nowhere.
    #[test]
    fn an_output_directory_is_recorded_once_and_only_under_a_watched_root() {
        let base = std::env::temp_dir().join(format!("rerail-own-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("root")).unwrap();
        let real = dunce::canonicalize(&base).unwrap();
        let (root, elsewhere) = (real.join("root"), real.join("scratch/out"));
        let store = real.join("store.files");
        journal_observer(&store).written();
        Connection::open(&store).unwrap().execute("INSERT INTO observer_meta VALUES (?1, '1')", [cursor_key_text("aa", &root)]).unwrap();
        let recorded = || -> Vec<String> {
            let db = Connection::open(&store).unwrap();
            let mut keys: Vec<String> = db.prepare(&format!("SELECT key FROM observer_meta WHERE key LIKE '{OWN_OUTPUT}%'")).unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
            keys.sort();
            keys
        };
        let mut o = journal_observer(&store);
        o.own_output(&root.join("out"));
        o.own_output(&root.join("out/phase-sources"));
        o.own_output(&elsewhere);
        o.written();
        assert_eq!(recorded(), [format!("{OWN_OUTPUT}{}", root.join("out").display())]);
        assert!(o.is_own(&root.join("out/phase-sources/x.csv")) && o.is_own(&elsewhere.join("x.csv")), "all three are own for the process");
        drop(o);
        let mut o = journal_observer(&store);
        o.own_output(&root.join("out"));
        o.written();
        assert_eq!(recorded().len(), 1, "the next process knows it: nothing written again");
        drop(o);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An inner root is asked as the stored root above it (`outermost`): answered from that root's cursor, trusted,
    /// with no cursor or settling of its own.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_inner_root_is_answered_by_the_root_above_it() {
        let base = std::env::temp_dir().join(format!("rerail-inner-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = dunce::canonicalize({
            std::fs::create_dir_all(base.join("root/lib")).unwrap();
            base.join("root")
        })
        .unwrap();
        let file = root.join("lib/a.rerail");
        std::fs::write(&file, "x.\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let run = |watch: &Path| {
            let mut o = journal_observer(&store);
            o.begin();
            o.watch(&[watch.to_path_buf()]);
            let asked: Vec<(PathBuf, bool)> = o.roots.iter().map(|r| (r.path.clone(), r.trusted)).collect();
            o.digest(&file).unwrap();
            o.flush();
            o.written();
            (asked, o.counts)
        };
        let (_, one) = run(&root);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (_, two) = run(&root);
        let (asked, inner) = run(&root.join("lib"));
        if journal_answered(&[&one, &two, &inner], "the inner root") {
            assert_eq!(asked, [(root.clone(), true)], "asked as the root above it, and trusted");
        }
        let db = Connection::open(&store).unwrap();
        let cursors: Vec<String> = db.prepare("SELECT key FROM observer_meta WHERE key LIKE 'journal:%'").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert!(cursors.iter().all(|k| !k.ends_with("/lib")), "no cursor of its own: {cursors:?}");
        drop(db);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Asking about rerail's own files watches nothing: no root above them is asked of the journal. Phase 0's
    /// scratch corpus, listed, had watched the whole temporary directory, whose news the next run stored.
    #[cfg(target_os = "macos")]
    #[test]
    fn rerails_own_files_watch_nothing() {
        let base = std::env::temp_dir().join(format!("rerail-own-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let scratch = base.join("scratch-root");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("a.py"), "x = 1\n").unwrap();
        let store = base.join("store.files");
        let mut o = journal_observer(&store);
        o.own.push(base.join("scratch"));
        o.begin();
        assert!(o.listing(&scratch).is_some());
        assert_eq!(o.kind(&scratch.join("a.py")), Kind::File);
        assert!(o.digest(&scratch.join("a.py")).is_some() && o.canonical(&scratch).is_some());
        assert!(o.roots.is_empty(), "no root asked: {:?}", o.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>());
        o.flush();
        drop(o);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A first run that watches an inner root before the root around it keeps what it checked under both: each root
    /// settles keeping what was checked under it, so the inner one forgot everything checked as the outer one's, and
    /// the run after a cold one recorded it all again (`RERAIL_WARM`).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_first_run_keeps_what_it_checked_under_nested_roots() {
        let base = std::env::temp_dir().join(format!("rerail-nested-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = dunce::canonicalize({
            std::fs::create_dir_all(base.join("root/lib")).unwrap();
            base.join("root")
        })
        .unwrap();
        let file = root.join("lib/a.rerail");
        std::fs::write(&file, "x.\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let other = root.join("b.rerail");
        std::fs::write(&other, "y.\n").unwrap();
        aged(&other);
        let mut o = journal_observer(&store);
        o.begin();
        // Asked under the inner root before the outer one is watched, and under both after.
        o.watch(&[root.join("lib")]);
        assert!(o.canonical(&file).is_some());
        o.watch(std::slice::from_ref(&root));
        o.digest(&file).unwrap();
        o.digest(&other).unwrap();
        assert_eq!(o.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(), std::slice::from_ref(&root), "the outer root took the inner one in");
        o.flush();
        o.written();
        drop(o);
        let db = Connection::open(&store).unwrap();
        let kept = |p: &Path| -> i64 { db.query_row("SELECT count(*) FROM observed WHERE path = ?1", [p.to_string_lossy()], |r| r.get(0)).unwrap() };
        let canon: i64 = db.query_row("SELECT count(*) FROM run_record WHERE key = ?1", [format!("{CANONICAL}{}", file.display())], |r| r.get(0)).unwrap();
        assert_eq!((kept(&file), kept(&other), canon), (1, 1, 1), "the files' answers and the canonical form are kept");
        let cursors: Vec<String> = db.prepare("SELECT key FROM observer_meta WHERE key LIKE 'journal:%'").unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert!(cursors.iter().all(|k| !k.ends_with("/lib")), "one cursor, the outer root's: {cursors:?}");
        drop(db);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The same across runs of one process: an inner root settled at an earlier run's end (phase 0's, before the
    /// corpus run) keeps its answers when a later run first asks the root around it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_first_process_keeps_what_an_earlier_run_checked_under_an_inner_root() {
        let base = std::env::temp_dir().join(format!("rerail-nested-runs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = dunce::canonicalize({
            std::fs::create_dir_all(base.join("root/lib")).unwrap();
            base.join("root")
        })
        .unwrap();
        let file = root.join("lib/a.rerail");
        std::fs::write(&file, "x.\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let mut o = journal_observer(&store);
        o.begin();
        o.watch(&[root.join("lib")]);
        assert!(o.canonical(&file).is_some());
        o.digest(&file).unwrap();
        o.flush();
        o.begin();
        o.watch(std::slice::from_ref(&root));
        o.flush();
        o.written();
        drop(o);
        let db = Connection::open(&store).unwrap();
        let kept: i64 = db.query_row("SELECT count(*) FROM observed WHERE path = ?1", [file.to_string_lossy()], |r| r.get(0)).unwrap();
        let canon: i64 = db.query_row("SELECT count(*) FROM run_record WHERE key = ?1", [format!("{CANONICAL}{}", file.display())], |r| r.get(0)).unwrap();
        assert_eq!((kept, canon), (1, 1), "the earlier run's answer and canonical form are kept");
        drop(db);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A path's canonical form is resolved once: under a root the journal says is unchanged, the next run answers it
    /// with no `realpath`, and a symlink retargeted under the root is resolved again. The journal's (FSEvents), as
    /// the other journal tests: with none (Linux, no Watchman), nothing is trusted by it.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_canonical_path_is_resolved_again_only_where_the_journal_names_a_change() {
        use std::os::unix::fs::symlink as symlink_file;
        let base = std::env::temp_dir().join(format!("rerail-canonical-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        for i in 0..3 {
            std::fs::write(root.join(format!("pkg/m{i}.py")), "x = 1\n").unwrap();
        }
        let link = root.join("pkg/alias.py");
        symlink_file("m0.py", &link).unwrap();
        let store = base.join("store.files");
        let asked = [root.join("pkg/m1.py"), link.clone(), root.join("pkg/../pkg/m2.py")];
        let run = |settle: i64| {
            let mut o = journal_observer(&store);
            o.settle_ns = settle;
            o.begin();
            o.watch(std::slice::from_ref(&root));
            let got: Vec<Option<PathBuf>> = asked.iter().map(|p| o.canonical(p)).collect();
            o.flush();
            (o.counts, got)
        };
        let (first, got) = run(0);
        assert_eq!(first.resolved, 3, "{first}");
        let real = dunce::canonicalize(&root).unwrap();
        assert_eq!(got, vec![Some(real.join("pkg/m1.py")), Some(real.join("pkg/m0.py")), Some(real.join("pkg/m2.py"))]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (middle, _) = run(0);
        let (second, same) = run(0);
        assert_eq!(same, got);
        // The path through `..` is resolved each time: it is not a name its directory's listing holds.
        if journal_answered(&[&first, &middle, &second], "nothing changed") {
            assert_eq!((second.resolved, second.trusted), (1, 2), "nothing changed: {second}");
        }
        std::fs::remove_file(&link).unwrap();
        symlink_file("m2.py", &link).unwrap();
        let (third, mut after) = run(0);
        if journal_answered(&[&first, &middle, &second, &third], "a retargeted symlink") {
            assert!(third.resolved >= 2, "the retargeted symlink is resolved again: {third}");
        } else {
            // As a run asks, its settle window on (`journal_answered`): the link is resolved again whatever the
            // journal said.
            after = run(JOURNAL_SETTLE_NS).1;
        }
        assert_eq!(after[1], Some(real.join("pkg/m2.py")), "the retargeted symlink's target");
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
        let mut first = journal_observer(&store);
        first.validate_every = 0;
        let d1 = first.digest(&file).unwrap();
        // Recorded with a time after ctime + RACY_NS, as a later run would.
        for e in first.entries.values_mut() {
            e.recorded_ns += 2 * RACY_NS;
        }
        first.dirty = first.entries.keys().cloned().collect();
        first.flush();
        first.written();
        let mut second = journal_observer(&store);
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

    /// An answer recorded inside the racy window is read again at the run's end once the window has passed, and
    /// trusted then: the next run (a warm one) neither reads nor records it (Q45). One whose bytes changed is not.
    #[test]
    fn a_racy_answer_is_settled_at_the_runs_end() {
        let dir = tempfile::tempdir().unwrap();
        let (file, other) = (dir.path().join("a.py"), dir.path().join("b.py"));
        std::fs::write(&file, "x = 1\n").unwrap();
        std::fs::write(&other, "y = 1\n").unwrap();
        let mut o = Observer::open(None);
        o.validate_every = 0;
        let d = o.digest(&file).unwrap();
        o.digest(&other).unwrap();
        o.listing(dir.path()).unwrap();
        assert!(o.entries.values().all(Entry::racy), "written just now: recorded inside the window");
        // Still inside the window: left as it was, and kept for a later end.
        o.settle_racy(now_ns());
        assert!(o.entries[&file].racy());
        assert_eq!(o.racy.len(), 3, "each waits for a later end: {:?}", o.racy);
        // `b.py`'s recorded digest no longer its bytes' (as if written in the recording's tick, its stat kept).
        o.entries.get_mut(&other).unwrap().digest = [7; 32];
        let hashed = o.counts.hashed;
        // Past the window (as a run's end some seconds later is).
        o.settle_racy(now_ns() + 3 * RACY_NS);
        assert_eq!(o.counts.hashed, hashed + 2, "each racy file read again once");
        assert!(!o.entries[&file].racy() && !o.entries[dir.path()].racy(), "the same bytes and listing: trusted");
        assert!(o.entries[&other].racy(), "bytes that differ from the answer: left for the next run to read");
        assert!(o.racy.is_empty());
        assert_eq!(o.entries[&file].digest, d);
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
        // The machine's git settings (a signing key, hooks, a default branch) are not the test's.
        let out = std::process::Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
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
        std::fs::write(repo.join("pkg/b.rerail"), "r(1).\n").unwrap();
        std::fs::write(repo.join("gone.py"), "").unwrap();
        std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
        git(&repo, &["add", "--", "a.py", "pkg/b.rerail", "gone.py", ".gitignore"]);
        git(&repo, &["commit", "-m", "one"]);
        std::fs::remove_file(repo.join("gone.py")).unwrap();
        std::fs::write(repo.join("new.py"), "").unwrap();
        std::fs::write(repo.join("noise.log"), "").unwrap();
        let g = crate::git_workspace::discover(&repo).unwrap();
        assert_eq!(g.worktree_files(), vec![".gitignore", "a.py", "new.py", "pkg/b.rerail"]);

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
        let mut o = journal_observer(&store);
        o.validate_every = 0;
        o.git(&repo).unwrap();
        let d = o.digest(&repo.join("a.py")).unwrap();
        assert_eq!((o.counts.hashed, o.counts.git), (1, 0), "first sight of the blob: read once, remembered");
        o.flush();
        o.written();
        let mut fresh = journal_observer(&store);
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

    /// `forget` takes a path named and what is under it, and its directory's own answer: a range of the sorted
    /// answers, which leaves a sibling whose name only starts the same (`b.txt` beside `b`).
    #[test]
    fn forget_takes_what_is_under_a_path_and_no_sibling() {
        let mut o = Observer::open(None);
        let stat = Stat::default();
        let keep = ["/r", "/r/a", "/r/a.txt", "/r/a0/x", "/r/b/c", "/q/a/x"];
        let forgotten = ["/r/a/x", "/r/a/y/z", "/r/b"];
        for p in keep.iter().chain(&forgotten) {
            o.entries.insert(PathBuf::from(p), Entry { kind: Kind::File, stat, digest: [0; 32], listing: None, recorded_ns: 0 });
            o.canon.insert(PathBuf::from(p), PathBuf::from(p));
        }
        // `/r/a` named changed: it and what is under it go, and its directory `/r`'s answer; `/r/b` its own.
        o.forget(Path::new("/r"), &[PathBuf::from("/r/a"), PathBuf::from("/r/b/c")], None, "");
        let left: Vec<&str> = o.entries.keys().map(|k| k.to_str().unwrap()).collect();
        assert_eq!(left, ["/q/a/x", "/r/a.txt", "/r/a0/x"]);
        let canon: Vec<&str> = o.canon.keys().map(|k| k.to_str().unwrap()).collect();
        assert_eq!(canon, ["/q/a/x", "/r", "/r/a.txt", "/r/a0/x", "/r/b"], "a canonical form goes only under a path named");
        assert_eq!(o.counts.forgotten, 6);
    }

    /// A root the journal names nothing under keeps its stored cursor, until the cursor is `CURSOR_AGE` events
    /// behind: it is then stored again, once.
    #[test]
    fn an_aged_cursor_is_stored_again() {
        let base = std::env::temp_dir().join(format!("rerail-aged-cursor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let store = base.join("store.files");
        let (root, volume) = (Path::new("/r"), [7u8; 16]);
        let key = cursor_key(root, volume);
        let stored = || -> Option<String> {
            let db = Connection::open(&store).ok()?;
            db.query_row("SELECT value FROM observer_meta WHERE key = ?1", [&key], |r| r.get::<_, rusqlite::types::Value>(0)).ok().as_ref().and_then(text_cursor)
        };
        let mut o = journal_observer(&store);
        o.db.as_mut().unwrap().stored.insert(key.clone(), "1000".into());
        o.forget(root, &[], Some(volume), &(1000 + CURSOR_AGE - 1).to_string());
        o.written();
        assert_eq!(stored(), None, "not yet aged: nothing written");
        o.forget(root, &[], Some(volume), &(1000 + CURSOR_AGE).to_string());
        o.written();
        assert_eq!(stored(), Some((1000 + CURSOR_AGE).to_string()));
        o.forget(root, &[], Some(volume), &(1000 + CURSOR_AGE + 5).to_string());
        o.written();
        assert_eq!(stored(), Some((1000 + CURSOR_AGE).to_string()), "stored again once, then aged from there");
        drop(o);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The observer's database names the user's files, their stats and digests: private to the user, as the store
    /// is (docs/privacy.md). It was made with the umask's mode (`0644`), readable by every user of the machine.
    #[cfg(unix)]
    #[test]
    fn the_observers_database_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("rerail-observer-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let store = base.join("store.files");
        let o = journal_observer(&store);
        o.written();
        let mode = |p: &Path| std::fs::metadata(p).map(|m| m.permissions().mode() & 0o777).ok();
        assert_eq!(mode(&store), Some(0o600));
        let mut wal = store.as_os_str().to_owned();
        wal.push("-wal");
        assert!(matches!(mode(Path::new(&wal)), None | Some(0o600)), "its WAL takes its mode");
        assert!(matches!(mode(&cookie_dir(&store)), None | Some(0o700)));
        drop(o);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// News that forgets nothing (another program's file under a watched root) keeps the stored cursor too, until it
    /// ages or its replay names `CURSOR_PATHS` paths: a warm run wrote the moved cursor whenever anything changed
    /// under a root, though nothing it kept had (`RERAIL_WARM`). News that forgets an answer stores it at once.
    #[test]
    fn news_that_forgets_nothing_keeps_the_stored_cursor() {
        let base = std::env::temp_dir().join(format!("rerail-unrelated-news-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let store = base.join("store.files");
        let (root, volume) = (Path::new("/r"), [7u8; 16]);
        let key = cursor_key(root, volume);
        let stored = || -> Option<String> {
            let db = Connection::open(&store).ok()?;
            db.query_row("SELECT value FROM observer_meta WHERE key = ?1", [&key], |r| r.get::<_, rusqlite::types::Value>(0)).ok().as_ref().and_then(text_cursor)
        };
        let mut o = journal_observer(&store);
        o.db.as_mut().unwrap().stored.insert(key.clone(), "1000".into());
        // The root's part of the store read first, as `watch` reads it.
        o.ensure(root);
        o.entries.insert(PathBuf::from("/r/kept.py"), Entry { kind: Kind::File, stat: Stat::default(), digest: [0; 32], listing: None, recorded_ns: 0 });
        o.forget(root, &[PathBuf::from("/r/other/x.lock")], Some(volume), "1010");
        o.written();
        assert_eq!(stored(), None, "nothing forgotten: nothing written");
        let many: Vec<PathBuf> = (0..CURSOR_PATHS).map(|i| PathBuf::from(format!("/r/other/{i}"))).collect();
        o.forget(root, &many, Some(volume), "1020");
        o.written();
        assert_eq!(stored(), Some("1020".to_owned()), "a replay of that many paths: stored again");
        o.forget(root, &[PathBuf::from("/r/kept.py")], Some(volume), "1030");
        o.written();
        assert_eq!(stored(), Some("1030".to_owned()), "an answer forgotten: stored with the delete");
        drop(o);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A record refreshed for its aged cursors keeps everything but its first list, in memory and in the store.
    #[test]
    fn a_refreshed_record_moves_only_its_cursors() {
        let base = std::env::temp_dir().join(format!("rerail-refreshed-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let store = base.join("store.files");
        let mut o = journal_observer(&store);
        let record = |cursors: &[&str]| encode_lists(&[cursors, &["/r", "/r/.git"], &["/r/.git/HEAD"]]);
        o.put_run_record("git-files:/r", &record(&["1", "1"]), b"2\na.py");
        o.refresh_record("git-files:/r", &["9".into(), "9".into()]);
        o.written();
        let want = (record(&["9", "9"]), b"2\na.py".to_vec());
        assert_eq!(o.run_record("git-files:/r"), Some(want.clone()));
        drop(o);
        assert_eq!(journal_observer(&store).run_record("git-files:/r"), Some(want));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The store is read a part of the tree at a time, as it is first asked (`ensure`): an answer under one directory
    /// reads that directory's and no other's; what memory holds wins over the store; and what is forgotten before its
    /// part is read is not read back.
    #[test]
    fn the_store_is_read_a_part_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("p0.files");
        let entry = |size| Entry { kind: Kind::File, stat: Stat { size, ..Stat::default() }, digest: [0; 32], listing: None, recorded_ns: 0 };
        let paths = ["/x/a/f", "/x/a/g", "/y/b/f", "/y/b/g"].map(PathBuf::from);
        let mut first = journal_observer(&store);
        for p in &paths {
            first.entries.insert(p.clone(), entry(1));
            first.dirty.push(p.clone());
        }
        first.flush();
        first.written();
        drop(first);
        let mut o = journal_observer(&store);
        assert!(o.entries.is_empty(), "nothing read at open");
        assert_eq!(o.known(&paths[0]).map(|e| e.stat.size), Some(1));
        let held: Vec<&Path> = o.entries.keys().map(PathBuf::as_path).collect();
        assert_eq!(held, [paths[0].as_path(), paths[1].as_path()], "only /x/a read");
        // Memory ahead of the store: a part read later keeps what memory holds.
        o.entries.insert(paths[3].clone(), entry(9));
        // Forgotten before /y/b is read: the store's row is deleted first, not read back.
        o.forget(Path::new("/y"), &[paths[2].clone()], None, "");
        assert!(o.known(&paths[2]).is_none(), "forgotten, not read back");
        assert_eq!(o.known(&paths[3]).map(|e| e.stat.size), Some(9), "memory wins");
    }

    /// `collect` drops what no run asks any more, and keeps the rest.
    #[test]
    fn realpaths_a_directory_at_a_time_are_each_files_own() {
        let given = std::env::temp_dir().join(format!("rerail-realpaths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&given);
        std::fs::create_dir_all(given.join("d/e")).unwrap();
        std::fs::write(given.join("d/a.py"), "").unwrap();
        std::fs::write(given.join("d/e/b.py"), "").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(given.join("d/e/b.py"), given.join("d/link.py")).unwrap();
            std::os::unix::fs::symlink(given.join("d/e"), given.join("d/dirlink")).unwrap();
        }
        let asked: Vec<PathBuf> = ["d/a.py", "d/e/b.py", "d/link.py", "d/dirlink/b.py", "d/missing.py", "d/A.py", "d/e/../a.py"].iter().map(|p| given.join(p)).collect();
        let refs: Vec<&Path> = asked.iter().map(PathBuf::as_path).collect();
        let one_by_one: Vec<Option<PathBuf>> = asked.iter().map(|p| dunce::canonicalize(p).ok()).collect();
        assert_eq!(realpaths(&refs), one_by_one, "as `realpath` gives each: symlinks, a missing file, another case, `..`");
        let _ = std::fs::remove_dir_all(&given);
    }

    #[test]
    fn collection_drops_what_no_run_asks_any_more() {
        let given = std::env::temp_dir().join(format!("rerail-observer-collect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&given);
        std::fs::create_dir_all(given.join("root/inner")).unwrap();
        let real = dunce::canonicalize(&given).unwrap();
        let store = real.join("store.files");
        journal_observer(&store).written();
        let (root, inner, gone) = (real.join("root"), real.join("root/inner"), real.join("gone"));
        let dotted = real.join("root/../root");
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let db = Connection::open(&store).unwrap();
        let meta = |root: &Path, cursor: &str| db.execute("INSERT INTO observer_meta VALUES (?1, ?2)", params![cursor_key_text("aa", root), cursor]).unwrap();
        meta(&root, "100");
        meta(&inner, "50");
        meta(&gone, "10");
        meta(&dotted, "5");
        // Another name of the same directory, with an older cursor (`/var/…` beside `/private/var/…` on macOS).
        let alias = (given != real).then(|| given.join("root"));
        if let Some(a) = &alias {
            meta(a, "20");
        }
        // Two names of one directory one run asked, with the same listing cursor, are both kept; a third name with an
        // older generation is dropped.
        std::fs::create_dir_all(real.join("listed")).unwrap();
        let (same, older) = (real.join("same"), real.join("older"));
        #[cfg(unix)]
        for link in [&same, &older] {
            std::os::unix::fs::symlink(real.join("listed"), link).unwrap();
        }
        meta(&real.join("listed"), "L3");
        meta(&same, "L3");
        meta(&older, "L1");
        let answer = |p: &Path| db.execute("INSERT INTO observed VALUES (?1, 1, 0, 0, 0, 0, 0, x'', NULL, 0)", [s(p)]).unwrap();
        answer(&root.join("a.py"));
        answer(&real.join("elsewhere/b.py"));
        answer(&gone.join("c.py"));
        if let Some(a) = &alias {
            answer(&a.join("a.py"));
        }
        // A root a run first asked has no cursor stored until the run's end: what the run recorded under it meanwhile
        // is young, and kept (it was collected as the run ended, and the next run recorded it all again).
        let fresh = real.join("fresh");
        db.execute("INSERT INTO observed VALUES (?1, 1, 0, 0, 0, 0, 0, x'', NULL, ?2)", params![s(&fresh.join("d.py")), now_ns()]).unwrap();
        db.execute("INSERT INTO run_record VALUES (?1, ?2, ?3)", params![format!("{CANONICAL}{}", s(&fresh)), s(&fresh), now_ns().to_le_bytes().to_vec()]).unwrap();
        let (live, dead) = ([1u8; 32], [2u8; 32]);
        let record = |key: &str, consulted: &str| db.execute("INSERT INTO run_record VALUES (?1, ?2, x'')", params![key, consulted]).unwrap();
        // A cutoff's consulted list as the engine keeps it (`reuse::encode_consulted`): kind, path, digest by turns.
        let cutoff = |key: &str, flat: &[&str]| db.execute("INSERT INTO run_record VALUES (?1, ?2, x'')", params![key, encode_lists(&[flat])]).unwrap();
        record(&format!("{CANONICAL}{}", s(&root.join("a.py"))), "");
        record(&format!("{CANONICAL}{}", s(&real.join("elsewhere"))), "");
        cutoff("cutoff:kept", &["rule", "x.rerail", "ab", "plan", "", &hex(&dead), "plan", "", &hex(&live)]);
        cutoff("cutoff:dead", &["plan", "", &hex(&dead)]);
        cutoff(&"ab".repeat(32), &["rule", "x.rerail", "ab"]);
        // The constants' and the cone's rounds' records go with their plans too; one naming none is from before.
        cutoff("constants:k", &[]);
        cutoff("constants:live", &["doc", "a.py", "ab", "plan", "", &hex(&live)]);
        cutoff("constants:dead", &["doc", "a.py", "ab", "plan", "", &hex(&dead)]);
        let rounds = |key: &str, plan: &[u8; 32]| db.execute("INSERT INTO run_record VALUES (?1, ?2, ?3)", params![key, encode_lists(&[&["1", "2"][..], &["/r", "/s"], &["plan", "", &hex(plan)]]), 7u64.to_le_bytes().to_vec()]).unwrap();
        rounds("cone-rounds:live", &live);
        rounds("cone-rounds:dead", &dead);
        // An output directory still there is kept; one removed goes (decision 0284).
        db.execute("INSERT INTO observer_meta VALUES (?1, 1)", [format!("{OWN_OUTPUT}{}", s(&inner))]).unwrap();
        db.execute("INSERT INTO observer_meta VALUES (?1, 1)", [format!("{OWN_OUTPUT}{}", s(&gone))]).unwrap();
        drop(db);
        let swept = collect(&store, |plan| *plan == live).unwrap();
        let db = Connection::open(&store).unwrap();
        let keys = |sql: &str| -> Vec<String> { db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect() };
        let mut roots = keys("SELECT key FROM observer_meta WHERE key LIKE 'journal:%'");
        roots.sort();
        let mut want = vec![cursor_key_text("aa", &real.join("listed")), cursor_key_text("aa", &root), cursor_key_text("aa", &same)];
        want.sort();
        assert_eq!(roots, want, "root: inner is under it, gone is gone, dotted and the alias are other names with older cursors; listed and same: one directory's two names of one run");
        let mut answers = keys("SELECT path FROM observed");
        answers.sort();
        assert_eq!(answers, [s(&fresh.join("d.py")), s(&root.join("a.py"))]);
        let mut records = keys("SELECT key FROM run_record");
        records.sort();
        assert_eq!(records, [format!("{CANONICAL}{}", s(&fresh)), format!("{CANONICAL}{}", s(&root.join("a.py"))), "cone-rounds:live".to_owned(), "constants:live".to_owned(), "cutoff:kept".to_owned()]);
        let n = usize::from(alias.is_some());
        assert_eq!(keys(&format!("SELECT key FROM observer_meta WHERE key LIKE '{OWN_OUTPUT}%'")), [format!("{OWN_OUTPUT}{}", s(&inner))]);
        assert_eq!(swept, Swept { cursors: 4 + n, answers: 2 + n, records: 6, outputs: 1, changes: 0 });
        assert_eq!(std::fs::metadata(real.join("store.files-wal")).map_or(0, |m| m.len()), 0, "the WAL folded in");
        drop(db);
        let _ = std::fs::remove_dir_all(&given);
    }

    #[test]
    fn listings_round_trip_through_the_store_encoding() {
        let l = vec![("a".to_owned(), true), ("b.py".to_owned(), false)];
        assert_eq!(decode_listing(&encode_listing(&l).unwrap()).unwrap(), l);
        assert_eq!(decode_listing(&encode_listing(&[]).unwrap()).unwrap(), Vec::<(String, bool)>::new());
        assert!(encode_listing(&[("a\nb".to_owned(), false)]).is_none());
    }

    /// What `changed_since` names for `paths`: each under both names of its root, as given and canonical, where they
    /// differ (a temporary directory under macOS's `/var`), sorted.
    fn named(paths: &[&PathBuf]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = paths.iter().flat_map(|p| spellings(p)).collect();
        out.sort();
        out.dedup();
        out
    }

    /// A tree for the listing tests (decision 0400): `root/pkg/m0.py` … `m4.py`, each past the racy window.
    fn listing_tree(name: &str) -> (PathBuf, PathBuf, Vec<PathBuf>) {
        let base = std::env::temp_dir().join(format!("rerail-listing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("root");
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        let files: Vec<PathBuf> = (0..5).map(|i| root.join(format!("pkg/m{i}.py"))).collect();
        for f in &files {
            std::fs::write(f, "x = 1\n").unwrap();
            aged(f);
        }
        aged_dir(&root.join("pkg"));
        // Setting a time moves the ctime to now: past the racy window of that, the first run's answers hold.
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        (base, root, files)
    }

    /// A directory's times set back past the racy window, as `aged` does a file's mtime (its ctime is the setting's).
    fn aged_dir(dir: &Path) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::open(dir).unwrap().set_modified(old).unwrap();
    }

    /// One run of a new process's observer over `root`: its listing of `pkg`, each file's digest, its counts.
    fn listing_run(store: &Path, root: &Path, files: &[PathBuf]) -> (Counts, Option<Listing>, Vec<Option<[u8; 32]>>) {
        let mut o = Observer::open(Some(store));
        o.begin();
        o.watch(&[root.to_path_buf()]);
        let listing = o.listing(&root.join("pkg"));
        let digests = files.iter().map(|f| o.digest(f)).collect();
        o.flush();
        o.written();
        (o.counts, listing, digests)
    }

    /// R1: a second run over an unchanged small root makes no `stat` of a file (one of the root itself, whose answer
    /// no listing of its own holds): it lists the two directories holding answers and trusts every answer.
    #[cfg(unix)]
    #[test]
    fn a_small_root_is_checked_by_listings_not_stats() {
        let (base, root, files) = listing_tree("unchanged");
        let store = base.join("store.files");
        let (first, listing, digests) = listing_run(&store, &root, &files);
        assert!(first.stats >= 6 && first.trusted == 0, "the first run checks every path: {first}");
        // Past the racy window of the first run's own recording (the directory's answer too).
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (second, again, same) = listing_run(&store, &root, &files);
        assert_eq!((second.stats, second.trusted, second.checked_dirs, second.checked_named, second.hashed), (1, 6, 2, 0, 0), "nothing changed: no stat ({second})");
        assert_eq!((again, same), (listing, digests));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// R2: an edit is named and only it, also one that keeps the size and mtime (its ctime moves); a removed file is
    /// named, and a new one, as entries of their directory (0433): not the directory, whose naming would cover its
    /// siblings' answers too, and not the siblings.
    #[cfg(unix)]
    #[test]
    fn a_listing_names_an_edited_file_and_no_other() {
        let (base, root, files) = listing_tree("edited");
        let store = base.join("store.files");
        listing_run(&store, &root, &files);
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (_, listing, digests) = listing_run(&store, &root, &files);
        // The same size and mtime, other bytes: only the ctime says so.
        let mtime = std::fs::metadata(&files[2]).unwrap().modified().unwrap();
        std::fs::write(&files[2], "x = 2\n").unwrap();
        std::fs::File::options().write(true).open(&files[2]).unwrap().set_modified(mtime).unwrap();
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (third, same_listing, after) = listing_run(&store, &root, &files);
        assert_eq!((third.checked_named, third.hashed, third.stats), (1, 1, 2), "the edited file alone is named and read again: {third}");
        assert_ne!(after[2], digests[2]);
        assert_eq!([after[0], after[1], after[3], after[4]], [digests[0], digests[1], digests[3], digests[4]]);
        assert_eq!(same_listing, listing);
        std::fs::remove_file(&files[4]).unwrap();
        std::fs::write(root.join("pkg/new.py"), "y = 1\n").unwrap();
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (fourth, listing, after) = listing_run(&store, &root, &files);
        // The removed file and the new one; the others hold and are not read.
        assert_eq!((fourth.checked_named, fourth.hashed), (2, 0), "{fourth}");
        assert_eq!(fourth.forgotten, 2, "the removed file's answer and the directory's own: {fourth}");
        assert_eq!(after[4], None);
        let names: Vec<String> = listing.unwrap().iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, ["m0.py", "m1.py", "m2.py", "m3.py", "new.py"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An answer recorded inside the racy window (a file written just before a run) is checked by its bytes at the
    /// next run's listings: named only where they changed (0433). The journal names only what changed; the listings
    /// named every racy answer, and a header written before a run asked again for the unit that read it.
    #[cfg(unix)]
    #[test]
    fn a_racy_answer_with_its_bytes_unchanged_is_not_named() {
        let (base, root, files) = listing_tree("racy");
        let store = base.join("store.files");
        listing_run(&store, &root, &files);
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        listing_run(&store, &root, &files);
        // Edited, then recorded at once by a run: inside the racy window.
        std::fs::write(&files[1], "x = 22\n").unwrap();
        let (_, _, edited) = listing_run(&store, &root, &files);
        let (next, _, again) = listing_run(&store, &root, &files);
        assert_eq!((next.checked_named, next.validated), (0, 1), "the racy answer read again, its bytes as they were: {next}");
        assert_eq!(again, edited);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// R3: an answer that a path is missing holds while its directory is gone; when a directory is removed, each
    /// answer under it is named, and none is read back.
    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_listed_names_what_it_held() {
        let (base, root, mut files) = listing_tree("gone");
        files.push(root.join("absent/x.py"));
        let store = base.join("store.files");
        listing_run(&store, &root, &files);
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (second, _, digests) = listing_run(&store, &root, &files);
        assert_eq!((second.checked_named, second.stats), (0, 1), "a missing path under a missing directory holds: {second}");
        assert_eq!(digests[5], None);
        std::fs::remove_dir_all(root.join("pkg")).unwrap();
        let (third, listing, after) = listing_run(&store, &root, &files);
        assert!(third.checked_named >= 6, "the directory and each file under it: {third}");
        assert_eq!((listing, after.iter().flatten().count()), (None, 0));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// R4: an answer recorded inside the racy window is named by the listing: its file may have changed in the tick
    /// it was recorded in. The listing's own stat of the file, with the answer recorded just inside the window and
    /// just past it (a run timed against the window would depend on the host's load).
    #[cfg(unix)]
    #[test]
    fn a_racy_answer_is_named_by_the_listing() {
        let (base, root, files) = listing_tree("racy-named");
        std::fs::write(&files[0], "x = 3\n").unwrap();
        let name = files[0].file_name().unwrap().to_owned();
        let listed = listing::entries(&root.join("pkg")).unwrap().into_iter().find(|(n, _)| *n == name).and_then(|(_, l)| l);
        let Some((Kind::File, stat)) = listed else { panic!("the file is listed with its stat: {listed:?}") };
        let changed = stat.mtime_ns.max(stat.ctime_ns);
        let answer = |recorded_ns: i64| Entry { kind: Kind::File, stat, digest: [0; 32], listing: None, recorded_ns };
        assert!(!listed_holds(&answer(changed + RACY_NS / 2), listed), "recorded inside the window: named");
        assert!(listed_holds(&answer(changed + RACY_NS + 1), listed), "recorded past it: holds");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// R5: a listing's stat of an entry is `std::fs::metadata`'s: a file's (a compressed one too, on macOS), a
    /// directory's but for its size; a symbolic link is left to be `stat`ed on its own.
    #[cfg(unix)]
    #[test]
    fn the_bulk_listing_gives_what_stat_gives() {
        let base = std::env::temp_dir().join(format!("rerail-listing-stat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("dir/sub")).unwrap();
        std::fs::write(base.join("dir/a.py"), "a = 1\n").unwrap();
        std::fs::write(base.join("dir/empty"), "").unwrap();
        std::os::unix::fs::symlink("a.py", base.join("dir/link")).unwrap();
        let big = base.join("big.txt");
        std::fs::write(&big, "abcdefgh".repeat(16 * 1024)).unwrap();
        // A file the file system keeps compressed (APFS): its size is its bytes', not its blocks'.
        #[cfg(target_os = "macos")]
        {
            let made = std::process::Command::new("ditto").arg("--hfsCompression").arg(&big).arg(base.join("dir/compressed.txt")).status();
            assert!(made.is_ok_and(|s| s.success()), "ditto made the compressed copy");
        }
        let listed = listing::entries(&base.join("dir")).unwrap();
        let mut names: Vec<String> = listed.iter().map(|(n, _)| n.to_string_lossy().into_owned()).collect();
        names.sort();
        let mut want = vec!["a.py", "empty", "link", "sub"];
        if cfg!(target_os = "macos") {
            want.insert(1, "compressed.txt");
        }
        assert_eq!(names, want);
        for (name, got) in &listed {
            let path = base.join("dir").join(name);
            let name = name.to_string_lossy();
            let m = std::fs::symlink_metadata(&path).unwrap();
            if m.file_type().is_symlink() {
                assert_eq!(*got, None, "{name}: a link is stat'ed alone");
                continue;
            }
            let (kind, stat) = got.unwrap_or_else(|| panic!("{name}: no stat from the listing"));
            let want = Stat::of(&m);
            if m.is_dir() {
                assert_eq!((kind, stat), (Kind::Dir, Stat { size: 0, ..want }), "{name}");
            } else {
                assert_eq!((kind, stat), (Kind::File, want), "{name}");
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// R6: a run record made at a root's cursor of the observer's own journal is told of what changed after it, over
    /// later runs, and of nothing before it.
    #[cfg(unix)]
    #[test]
    fn a_run_record_is_answered_from_the_observers_own_journal() {
        let (base, root, files) = listing_tree("record");
        let store = base.join("store.files");
        let run = |edit: Option<&Path>, asked: &[String]| -> (String, Vec<Option<Vec<PathBuf>>>) {
            if let Some(f) = edit {
                std::fs::write(f, "x = 9\n").unwrap();
                aged(f);
                // Past the racy window of the edit's ctime: the answer this run records holds for the next.
                std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
            }
            let mut o = Observer::open(Some(&store));
            o.begin();
            let cursor = o.mark(std::slice::from_ref(&root)).unwrap().remove(0);
            for f in &files {
                o.digest(f);
            }
            let told = asked.iter().map(|c| o.changed_since("test-record", std::slice::from_ref(&root), std::slice::from_ref(c)).map(|mut v| { v.sort(); v })).collect();
            o.flush();
            o.written();
            (cursor, told)
        };
        let (_, _) = run(None, &[]);
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (c1, _) = run(None, &[]);
        assert!(listing_cursor(&c1).is_some(), "the root's cursor is the observer's own: {c1}");
        let (c2, told) = run(Some(&files[1]), std::slice::from_ref(&c1));
        assert_eq!(told, [Some(named(&[&files[1]]))]);
        let (_, told) = run(Some(&files[3]), &[c1.clone(), c2.clone()]);
        assert_eq!(told, [Some(named(&[&files[1], &files[3]])), Some(named(&[&files[3]]))]);
        let (c4, told) = run(None, &[c2]);
        assert_eq!(told, [Some(named(&[&files[3]]))], "nothing changed since: the same answer");
        let (_, told) = run(None, &[c4]);
        assert_eq!(told, [Some(vec![])]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// R8: the observer's journal tells a reader what changed, as FSEvents' replay did: a file added to a listed
    /// directory is named, not the directory (whose answer, its listing, is forgotten all the same; a reader takes a
    /// path named with all under it, as the cone's `still` does); and a file written just before one run and not since
    /// is not named to the next while its answer is still inside the racy window, its bytes being the answer's.
    #[cfg(unix)]
    #[test]
    fn the_journal_names_an_added_entry_and_not_a_racy_answer_that_holds() {
        let (base, root, files) = listing_tree("told");
        let store = base.join("store.files");
        let pkg = root.join("pkg");
        let run = |asked: &[String]| -> (String, Vec<Option<Vec<PathBuf>>>, Counts) {
            let mut o = Observer::open(Some(&store));
            o.begin();
            let cursor = o.mark(std::slice::from_ref(&root)).unwrap().remove(0);
            o.listing(&pkg);
            for f in &files {
                o.digest(f);
            }
            let told = asked.iter().map(|c| o.changed_since("test-told", std::slice::from_ref(&root), std::slice::from_ref(c)).map(|mut v| { v.sort(); v })).collect();
            o.flush();
            o.written();
            (cursor, told, o.counts)
        };
        run(&[]);
        std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        let (c1, _, _) = run(&[]);
        std::fs::write(pkg.join("new.py"), "y = 1\n").unwrap();
        let (c2, told, counts) = run(std::slice::from_ref(&c1));
        assert_eq!(told, [Some(named(&[&pkg.join("new.py")]))], "the entry added, not its directory");
        assert_eq!(counts.checked_named, 1, "the directory's answer is forgotten: {counts}");
        // Written now: the next run records its answer inside the window; the one after reads it again and names
        // nothing, its bytes unchanged.
        std::fs::write(&files[1], "x = 7\n").unwrap();
        let (c3, told, _) = run(std::slice::from_ref(&c2));
        assert_eq!(told, [Some(named(&[&files[1]]))]);
        let (_, told, counts) = run(std::slice::from_ref(&c3));
        assert_eq!(told, [Some(vec![])], "the racy answer's bytes are still the file's: {counts}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
