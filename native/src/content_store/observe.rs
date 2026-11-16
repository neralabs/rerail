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
use crate::content_store::connection::Cached as _;
use crate::content_store::RACY_NS;
use rusqlite::{Connection, params};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// Sorted by path, so what is under a path is one range (`forget`).
    entries: BTreeMap<PathBuf, Entry>,
    /// Paths whose entry changed since the last `flush`.
    dirty: Vec<PathBuf>,
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
        let cookies = path.map(cookie_dir).filter(|d| std::fs::create_dir_all(d).is_ok());
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
        Observer { db, entries, dirty: Vec::new(), aliases, new_aliases: Vec::new(), canon, new_canon: Vec::new(), epoch, validate_every, counts: Counts::default(), cwd: std::env::current_dir().unwrap_or_default(), git: Vec::new(), roots: Vec::new(), flushes: 0, open: 0, stored_roots, volumes, cookies, own, unreported: Vec::new() }
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
        self.own.iter().any(|o| owned_by(o, path)) || OWN.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|o| owned_by(o, path))
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
        let unit = match self.stored_roots.iter().filter(|r| under(&path, r)).min_by_key(|r| r.as_os_str().len()) {
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
        if !self.roots.iter().any(|r| r.trusted && r.asked == self.flushes && under(&path, &r.path)) {
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
        // Every root it is under: each settles keeping only what was checked under it (nested roots, a first run's).
        for root in self.roots.iter_mut().filter(|r| !r.trusted && under(&path, &r.path)) {
            root.checked.insert(path.to_owned());
        }
        static SHOW: OnceLock<bool> = OnceLock::new();
        if *SHOW.get_or_init(|| std::env::var_os("RERAIL_OBSERVED_PATHS").is_some()) {
            let under = self.roots.iter().find(|r| under(&path, &r.path)).map(|r| if r.trusted { "trusted root, no answer" } else { "untrusted root" }).unwrap_or("no root");
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
        for root in self.roots.iter_mut().filter(|r| !r.trusted && under(&path, &r.path)) {
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
        self.entries.insert(path.to_owned(), entry);
        self.dirty.push(path.to_owned());
    }

    /// What is at `path` now (following symlinks): one `stat`, or none where the journal says nothing changed
    /// (decision 0105): the answer kept, or for a path its directory's kept listing does not hold, missing.
    pub fn kind(&mut self, path: &Path) -> Kind {
        let path = &self.abs(path);
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
        if !self.roots.iter().any(|r| r.trusted && r.asked == self.flushes && under(&dir, &r.path)) || self.known(path).is_some() {
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
                let c = std::fs::canonicalize(&path).ok()?;
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
        if self.is_own(&path) {
            // rerail's own file (decision 0284): answered here, and never recorded (`canonical_resolved` is not reached).
            return Ok(std::fs::canonicalize(&path).ok());
        }
        self.ensure(&path);
        self.watch_above(&path);
        let asked = self.flushes;
        let trusted = |p: &Path, roots: &[Root]| roots.iter().any(|r| r.trusted && r.asked == asked && (under(&p, &r.path) || under(&p, &r.real)));
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
        self.validate_every != 0 && (fnv(path.as_os_str().as_encoded_bytes()).wrapping_add(self.epoch)) % self.validate_every == 0
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
        let read = |known, blob| DigestStep::Read(DigestRead { path: path.clone(), stat: now, started, known, blob });
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
        self.record_hash(&read.path, read.stat, read.started, digest);
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
                    // News of rerail's own files is no news (decision 0284): each run writes them.
                    let paths: Vec<PathBuf> = paths.into_iter().filter(|p| !self.is_own(p)).collect();
                    self.forget(&root, &paths, volume, &next);
                    self.roots.push(Root { real: std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: Some(next), path: root, asked: self.flushes, trusted: true, pending: None, checked: Default::default(), resolved: Default::default() });
                }
                super::journal::Since::Lost { cursor, .. } => {
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
                    self.roots.push(Root { real: std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone()), cursor: pending.as_ref().map(|(_, c)| c.clone()), path: root, asked: self.flushes, trusted: false, pending, checked, resolved });
                }
            }
        }
    }

    /// The root `root` is asked as: the widest stored root above it on the same volume, else itself. A journal's
    /// cursor is the volume's (an FSEvents id), so the outer root's answers for what is under it too: an inner root
    /// asked on its own started a stream of its own, whose cookie could miss its wait (2 s, then lost), or had no
    /// cursor of its own (lost at once), and each lost root was settled and written again every run.
    fn outermost(&mut self, root: &Path) -> PathBuf {
        let mut above: Vec<PathBuf> = self.stored_roots.iter().filter(|r| under(&root, r) && r.as_path() != root).cloned().collect();
        above.sort_by_key(|r| r.as_os_str().len());
        let here = self.volume(root);
        above.into_iter().find(|r| here.is_some() && self.volume(r) == here).unwrap_or_else(|| root.to_path_buf())
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
            let covered = roots.iter().zip(cursors).enumerate().any(|(j, (r, c))| c == cursor && under(&root, r) && (r != root || j < i));
            if covered {
                continue;
            }
            match super::journal::since(std::slice::from_ref(root), Some(cursor), &cookies) {
                super::journal::Since::Changed { paths, cursor } => {
                    named.extend(paths.into_iter().filter(|p| !self.is_own(p)));
                    now[i] = Some(cursor);
                }
                super::journal::Since::Lost { .. } => return None,
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
        if self.db.is_none() || self.roots.iter().any(|r| r.asked == self.flushes && under(&path, &r.path)) {
            return;
        }
        let root = match self.stored_roots.iter().filter(|r| under(&path, r)).min_by_key(|r| r.as_os_str().len()) {
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
        // rerail's own files are not in a listing (decision 0284): what they hold changes with every run.
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
        let listing: Listing = Arc::new(entries);
        if encode_listing(&listing).is_some() && !own {
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

    /// Keeps a run's record under `key`, replacing the last: in memory now, in the store behind the run.
    pub fn put_run_record(&mut self, key: &str, consulted: &[u8], outputs: &[u8]) {
        if let Some(k) = self.db.as_mut() {
            k.records.insert(key.to_owned(), (consulted.to_owned(), outputs.to_owned()));
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

/// The prefix of a directory runs wrote their outputs into, as `observer_meta` keeps it (`own_output`).
const OWN_OUTPUT: &str = "own:";

/// rerail's own files the process names (decision 0284, `own`): the cost statistics' file.
static OWN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Names `path` as one of rerail's own files for every observer of the process (decision 0284): it, what is under
/// it, and every file named after it beside it (`<path>-wal`, `<path>.upkeep`) are outside the observer.
pub fn own(path: &Path) {
    let mut own = OWN.lock().unwrap_or_else(|e| e.into_inner());
    for p in spellings(path) {
        if !own.contains(&p) {
            own.push(p);
        }
    }
}

/// `path` absolute, and with its directory's canonical form (the journal names paths by their real names).
fn spellings(path: &Path) -> Vec<PathBuf> {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let real = abs.parent().zip(abs.file_name()).and_then(|(dir, name)| Some(std::fs::canonicalize(dir).ok()?.join(name)));
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
    let resolved: Vec<Vec<(usize, Option<PathBuf>)>> = dirs
        .par_iter()
        .map(|(dir, files)| {
            let canonical = std::fs::canonicalize(dir).ok();
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
                        _ => std::fs::canonicalize(paths[i]).ok(),
                    };
                    (i, found)
                })
                .collect()
        })
        .collect();
    let mut out: Vec<Option<PathBuf>> = vec![None; paths.len()];
    for (i, c) in resolved.into_iter().flatten() {
        out[i] = c;
    }
    for i in alone {
        out[i] = std::fs::canonicalize(paths[i]).ok();
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
}

/// Collects the observer's store at `path` (`store_beside`), with the content store's collection (decision 0053),
/// behind the run: nothing ever left it before, and every process reads all of it as it opens. Dropped:
///
/// - a root's cursor where the root's name has a `.` or `..`, or is gone, or lies under another root kept; of two
///   names of one directory (`/var/…`, `/private/var/…`), all but the one with the newest cursor;
/// - an answer, a canonical form or a Git file list under no root kept: every path the observer reads is under a
///   root (`watch_above`), so one under none is of a name no run uses;
/// - a whole-run cutoff's record (`reuse::CUTOFF`) that names no plan `live` says is live (a run of another build,
///   whose manifests the content store no longer keeps), and one from before records named their plans.
///
/// Then the WAL is folded into the database and truncated. Other records (`constants:`, `cone-rounds:`) stay.
pub fn collect(path: &Path, live: impl Fn(&[u8; 32]) -> bool) -> rusqlite::Result<Swept> {
    let mut swept = Swept::default();
    if !path.exists() {
        return Ok(swept);
    }
    let mut db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(10))?;
    // The roots, by volume: (key, root, its directory's real path, cursor).
    let mut roots: Vec<(String, String, PathBuf, Option<PathBuf>, Option<u64>)> = {
        let mut st = db.prepare("SELECT key, value FROM observer_meta WHERE key LIKE 'journal:%'")?;
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, rusqlite::types::Value>(1)?)))?
            .flatten()
            .filter_map(|(key, value)| {
                let mut parts = key.splitn(3, ':');
                let (_, volume, root) = (parts.next()?, parts.next()?.to_owned(), PathBuf::from(parts.next()?));
                let real = plain(&root).then(|| std::fs::canonicalize(&root).ok()).flatten();
                let cursor = text_cursor(&value).and_then(|c| c.parse().ok());
                Some((key, volume, root, real, cursor))
            })
            .collect()
    };
    // Newest cursor first: of two names of one directory, the first is kept.
    roots.sort_by(|a, b| b.4.cmp(&a.4));
    let mut kept: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for (key, volume, root, real, _) in &roots {
        match real {
            Some(real) if !kept.iter().any(|(v, _, r)| v == volume && r == real) => kept.push((volume.clone(), root.clone(), real.clone())),
            _ => dropped.push(key.clone()),
        }
    }
    // A root under another kept is asked as part of it (`watch_above` takes the widest).
    let under: Vec<usize> = (0..kept.len()).filter(|&i| kept.iter().enumerate().any(|(j, (v, _, r))| j != i && *v == kept[i].0 && kept[i].2.starts_with(r) && kept[i].2 != *r)).collect();
    for &i in under.iter().rev() {
        let (volume, root, _) = kept.remove(i);
        dropped.push(cursor_key_text(&volume, &root));
    }
    let held = |path: &Path| plain(path) && kept.iter().any(|(_, root, _)| path.starts_with(root));
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
                    return !lists.iter().flat_map(|l| l.chunks_exact(3)).filter(|c| c[0] == "plan" && c[1].is_empty()).filter_map(|c| unhex(&c[2])).any(|plan| live(&plan));
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
    let tx = db.transaction()?;
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
    stat: Stat,
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
        std::fs::read(&self.path).ok()
    }

    /// `bytes`' length and digest, as `hash` gives a file's.
    pub fn hashed(bytes: &[u8]) -> (u64, [u8; 32]) {
        (bytes.len() as u64, Sha256::digest(bytes).into())
    }

    /// The file's stat as it was before the read: the bytes are its content while the file's stat is this.
    pub fn stat(&self) -> &Stat {
        &self.stat
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let root = std::fs::canonicalize(base.join("root")).unwrap();
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
            let mut o = Observer::open(Some(&store));
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
        let (_, names, kept, _) = run(true, 1);
        assert_eq!(names, ["pkg"], "a listing leaves rerail's own files out");
        assert_eq!(kept, 0, "no answer is kept for an own file");
        // The setup's writes may be named once more.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (_, _, _, marked) = run(false, 2);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let before = cursor();
        let (third, names, kept, _) = run(false, 3);
        assert_eq!((third.forgotten, third.stats), (0, 0), "the journal's news of own files forgets nothing: {third}");
        assert_eq!((names.as_slice(), kept), (["pkg".to_owned()].as_slice(), 0));
        assert_eq!(cursor(), before, "and moves no stored cursor: nothing written");
        // A record made in the second run holds: what changed since under its root is rerail's own.
        let mut o = Observer::open(Some(&store));
        o.begin();
        assert_eq!(o.changed_since("rec", std::slice::from_ref(&root), &marked), Some(Vec::new()));
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
        let real = std::fs::canonicalize(&base).unwrap();
        let (root, elsewhere) = (real.join("root"), real.join("scratch/out"));
        let store = real.join("store.files");
        Observer::open(Some(&store)).written();
        Connection::open(&store).unwrap().execute("INSERT INTO observer_meta VALUES (?1, '1')", [cursor_key_text("aa", &root)]).unwrap();
        let recorded = || -> Vec<String> {
            let db = Connection::open(&store).unwrap();
            let mut keys: Vec<String> = db.prepare(&format!("SELECT key FROM observer_meta WHERE key LIKE '{OWN_OUTPUT}%'")).unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect();
            keys.sort();
            keys
        };
        let mut o = Observer::open(Some(&store));
        o.own_output(&root.join("out"));
        o.own_output(&root.join("out/phase-sources"));
        o.own_output(&elsewhere);
        o.written();
        assert_eq!(recorded(), [format!("{OWN_OUTPUT}{}", root.join("out").display())]);
        assert!(o.is_own(&root.join("out/phase-sources/x.csv")) && o.is_own(&elsewhere.join("x.csv")), "all three are own for the process");
        drop(o);
        let mut o = Observer::open(Some(&store));
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
        let root = std::fs::canonicalize({
            std::fs::create_dir_all(base.join("root/lib")).unwrap();
            base.join("root")
        })
        .unwrap();
        let file = root.join("lib/a.rr");
        std::fs::write(&file, "x.\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let run = |watch: &Path| {
            let mut o = Observer::open(Some(&store));
            o.begin();
            o.watch(&[watch.to_path_buf()]);
            let asked: Vec<(PathBuf, bool)> = o.roots.iter().map(|r| (r.path.clone(), r.trusted)).collect();
            o.digest(&file).unwrap();
            o.flush();
            o.written();
            asked
        };
        run(&root);
        std::thread::sleep(std::time::Duration::from_millis(300));
        run(&root);
        assert_eq!(run(&root.join("lib")), [(root.clone(), true)], "asked as the root above it, and trusted");
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
        let mut o = Observer::open(Some(&store));
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
        let root = std::fs::canonicalize({
            std::fs::create_dir_all(base.join("root/lib")).unwrap();
            base.join("root")
        })
        .unwrap();
        let file = root.join("lib/a.rr");
        std::fs::write(&file, "x.\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let other = root.join("b.rr");
        std::fs::write(&other, "y.\n").unwrap();
        aged(&other);
        let mut o = Observer::open(Some(&store));
        o.begin();
        // Asked under the inner root before the outer one is watched, and under both after.
        o.watch(&[root.join("lib")]);
        assert!(o.canonical(&file).is_some());
        o.watch(std::slice::from_ref(&root));
        o.digest(&file).unwrap();
        o.digest(&other).unwrap();
        assert_eq!(o.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(), [root.clone()], "the outer root took the inner one in");
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
        let root = std::fs::canonicalize({
            std::fs::create_dir_all(base.join("root/lib")).unwrap();
            base.join("root")
        })
        .unwrap();
        let file = root.join("lib/a.rr");
        std::fs::write(&file, "x.\n").unwrap();
        aged(&file);
        let store = base.join("store.files");
        let mut o = Observer::open(Some(&store));
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
        let mut o = Observer::open(Some(&store));
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
        let mut o = Observer::open(Some(&store));
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
        let mut o = Observer::open(Some(&store));
        let record = |cursors: &[&str]| encode_lists(&[cursors, &["/r", "/r/.git"], &["/r/.git/HEAD"]]);
        o.put_run_record("git-files:/r", &record(&["1", "1"]), b"2\na.py");
        o.refresh_record("git-files:/r", &["9".into(), "9".into()]);
        o.written();
        let want = (record(&["9", "9"]), b"2\na.py".to_vec());
        assert_eq!(o.run_record("git-files:/r"), Some(want.clone()));
        drop(o);
        assert_eq!(Observer::open(Some(&store)).run_record("git-files:/r"), Some(want));
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
        let mut first = Observer::open(Some(&store));
        for p in &paths {
            first.entries.insert(p.clone(), entry(1));
            first.dirty.push(p.clone());
        }
        first.flush();
        first.written();
        drop(first);
        let mut o = Observer::open(Some(&store));
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
        let one_by_one: Vec<Option<PathBuf>> = asked.iter().map(|p| std::fs::canonicalize(p).ok()).collect();
        assert_eq!(realpaths(&refs), one_by_one, "as `realpath` gives each: symlinks, a missing file, another case, `..`");
        let _ = std::fs::remove_dir_all(&given);
    }

    #[test]
    fn collection_drops_what_no_run_asks_any_more() {
        let given = std::env::temp_dir().join(format!("rerail-observer-collect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&given);
        std::fs::create_dir_all(given.join("root/inner")).unwrap();
        let real = std::fs::canonicalize(&given).unwrap();
        let store = real.join("store.files");
        Observer::open(Some(&store)).written();
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
        cutoff("cutoff:kept", &["rule", "x.rr", "ab", "plan", "", &hex(&dead), "plan", "", &hex(&live)]);
        cutoff("cutoff:dead", &["plan", "", &hex(&dead)]);
        cutoff(&"ab".repeat(32), &["rule", "x.rr", "ab"]);
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
        assert_eq!(keys("SELECT key FROM observer_meta WHERE key LIKE 'journal:%'"), [cursor_key_text("aa", &root)], "one root: inner is under it, gone is gone, dotted and the alias are other names");
        let mut answers = keys("SELECT path FROM observed");
        answers.sort();
        assert_eq!(answers, [s(&fresh.join("d.py")), s(&root.join("a.py"))]);
        let mut records = keys("SELECT key FROM run_record");
        records.sort();
        assert_eq!(records, [format!("{CANONICAL}{}", s(&fresh)), format!("{CANONICAL}{}", s(&root.join("a.py"))), "cone-rounds:live".to_owned(), "constants:live".to_owned(), "cutoff:kept".to_owned()]);
        let n = usize::from(alias.is_some());
        assert_eq!(keys(&format!("SELECT key FROM observer_meta WHERE key LIKE '{OWN_OUTPUT}%'")), [format!("{OWN_OUTPUT}{}", s(&inner))]);
        assert_eq!(swept, Swept { cursors: 3 + n, answers: 2 + n, records: 6, outputs: 1 });
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
