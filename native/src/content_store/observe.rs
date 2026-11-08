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
//! **Git.** Inside a Git work tree, a tracked file that is clean against HEAD and whose stat equals its index entry's
//! holds its committed blob's bytes. Its digest is the blob's, when the store has seen those bytes before
//! (`git_alias`): a fresh clone or another worktree of the same repository is then not hashed.

use crate::content_store::RACY_NS;
use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One trusted file in this many is hashed again each process (`RERAIL_VALIDATE_EVERY`).
pub(crate) const VALIDATE_EVERY: u64 = 64;

/// What is at a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
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
pub(crate) struct Stat {
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
pub(crate) type Listing = Arc<Vec<(String, bool)>>;

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
pub(crate) struct Counts {
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
    /// Trusted files hashed again to check the stat (`VALIDATE_EVERY`).
    pub validated: u64,
    /// Of those, files whose bytes had changed under the same stat.
    pub mismatched: u64,
    /// Digests taken from a clean Git blob's alias, unread.
    pub git: u64,
}

impl Counts {
    pub(crate) fn since(&self, before: &Counts) -> Counts {
        Counts {
            stats: self.stats - before.stats,
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
            "{} stats, {} answers reused, {} files hashed ({} KB), {} directories listed, {} checked again ({} changed under the same stat), {} from Git blobs",
            self.stats,
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
pub(crate) struct Observer {
    db: Option<Connection>,
    entries: HashMap<PathBuf, Entry>,
    /// Paths whose entry changed since the last `flush`.
    dirty: Vec<PathBuf>,
    /// Git blob → sha256 of its bytes, seen this process (and in `git_alias`).
    aliases: HashMap<Vec<u8>, [u8; 32]>,
    new_aliases: Vec<(Vec<u8>, [u8; 32])>,
    epoch: u64,
    validate_every: u64,
    pub(crate) counts: Counts,
    /// The working directory, for relative paths: an entry is kept by its absolute path.
    cwd: PathBuf,
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
CREATE TABLE IF NOT EXISTS observer_meta(key TEXT PRIMARY KEY, value INTEGER) WITHOUT ROWID;";

impl Observer {
    /// An observer over the store at `path` (`<path>` is the P0 store itself), or over none.
    fn open(path: Option<&Path>) -> Observer {
        let db = path.and_then(|p| {
            let db = Connection::open(p).ok()?;
            db.busy_timeout(std::time::Duration::from_secs(10)).ok()?;
            db.execute_batch("PRAGMA journal_mode=WAL;").ok()?;
            db.execute_batch(SCHEMA).ok()?;
            Some(db)
        });
        // Each process checks another slice of the trusted files.
        let epoch = db
            .as_ref()
            .and_then(|db| {
                db.execute("INSERT INTO observer_meta VALUES ('epoch', 1) ON CONFLICT(key) DO UPDATE SET value = value + 1", []).ok()?;
                db.query_row("SELECT value FROM observer_meta WHERE key='epoch'", [], |r| r.get::<_, i64>(0)).ok()
            })
            .unwrap_or(0) as u64;
        let validate_every = std::env::var("RERAIL_VALIDATE_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(VALIDATE_EVERY);
        Observer { db, entries: HashMap::new(), dirty: Vec::new(), aliases: HashMap::new(), new_aliases: Vec::new(), epoch, validate_every, counts: Counts::default(), cwd: std::env::current_dir().unwrap_or_default() }
    }

    /// `path` made absolute, as entries are kept.
    fn abs(&self, path: &Path) -> PathBuf {
        if path.is_absolute() { path.to_owned() } else { self.cwd.join(path) }
    }

    /// The entry kept for `path`, in memory or in the store.
    fn known(&mut self, path: &Path) -> Option<Entry> {
        if let Some(e) = self.entries.get(path) {
            return Some(e.clone());
        }
        let db = self.db.as_ref()?;
        let row = db
            .prepare_cached("SELECT kind, dev, ino, size, mtime_ns, ctime_ns, digest, listing, recorded_ns FROM observed WHERE path=?1")
            .ok()?
            .query_row([path.to_string_lossy()], |r| {
                Ok((r.get::<_, i64>(0)?, [r.get::<_, i64>(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?], r.get::<_, Option<Vec<u8>>>(6)?, r.get::<_, Option<Vec<u8>>>(7)?, r.get::<_, i64>(8)?))
            })
            .optional()
            .ok()??;
        let (kind, [dev, ino, size, mtime_ns, ctime_ns], digest, listing, recorded_ns) = row;
        let entry = Entry {
            kind: Kind::of_code(kind),
            stat: Stat { dev, ino, size, mtime_ns, ctime_ns },
            digest: digest.and_then(|d| d.try_into().ok()).unwrap_or_default(),
            listing: match listing {
                Some(bytes) => Some(Arc::new(decode_listing(&bytes)?)),
                None => None,
            },
            recorded_ns,
        };
        self.entries.insert(path.to_owned(), entry.clone());
        Some(entry)
    }

    /// Loads every kept entry under `dir` at once: one range read, for a walk.
    pub(crate) fn preload(&mut self, dir: &Path) {
        let dir = &self.abs(dir);
        let Some(db) = self.db.as_ref() else { return };
        let lo = dir.to_string_lossy().into_owned();
        let hi = format!("{lo}\u{10ffff}");
        let Ok(mut st) = db.prepare_cached("SELECT path, kind, dev, ino, size, mtime_ns, ctime_ns, digest, listing, recorded_ns FROM observed WHERE path >= ?1 AND path < ?2") else { return };
        let Ok(rows) = st.query_map(params![lo, hi], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, [r.get::<_, i64>(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?], r.get::<_, Option<Vec<u8>>>(7)?, r.get::<_, Option<Vec<u8>>>(8)?, r.get::<_, i64>(9)?))
        }) else {
            return;
        };
        for (path, kind, [dev, ino, size, mtime_ns, ctime_ns], digest, listing, recorded_ns) in rows.flatten() {
            let listing = match listing.map(|b| decode_listing(&b)) {
                Some(None) => continue,
                Some(Some(l)) => Some(Arc::new(l)),
                None => None,
            };
            self.entries.entry(PathBuf::from(path)).or_insert(Entry {
                kind: Kind::of_code(kind),
                stat: Stat { dev, ino, size, mtime_ns, ctime_ns },
                digest: digest.and_then(|d| d.try_into().ok()).unwrap_or_default(),
                listing,
                recorded_ns,
            });
        }
    }

    fn stat(&mut self, path: &Path) -> Option<(Kind, Stat)> {
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
        self.entries.insert(path.to_owned(), entry);
        self.dirty.push(path.to_owned());
    }

    /// What is at `path` now (following symlinks): one `stat`.
    pub(crate) fn kind(&mut self, path: &Path) -> Kind {
        self.stat(path).map_or(Kind::Missing, |(k, _)| k)
    }

    /// Whether this trusted file is one this process checks again.
    fn checks(&self, path: &Path) -> bool {
        self.validate_every != 0 && (fnv(path.as_os_str().as_encoded_bytes()).wrapping_add(self.epoch)) % self.validate_every == 0
    }

    /// The sha256 of the file at `path` (`None`: not a readable file), read only when its stat changed.
    pub(crate) fn digest(&mut self, path: &Path) -> Option<[u8; 32]> {
        let path = &self.abs(path);
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
        self.hash(path, now, started).map(|(_, d)| d)
    }

    /// The bytes of the file at `path` and their sha256 (`None`: not a readable file). Always reads.
    pub(crate) fn read(&mut self, path: &Path) -> Option<(Vec<u8>, [u8; 32])> {
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
    pub(crate) fn listing(&mut self, dir: &Path) -> Option<Listing> {
        let dir = &self.abs(dir);
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
    pub(crate) fn listing_digest(&mut self, dir: &Path) -> Option<[u8; 32]> {
        self.listing(dir).map(|l| listing_digest(&l))
    }

    /// The digest of a clean Git blob's bytes, when the store has seen them (`git_alias`).
    pub(crate) fn alias(&mut self, blob: &[u8]) -> Option<[u8; 32]> {
        if let Some(d) = self.aliases.get(blob) {
            return Some(*d);
        }
        let d: [u8; 32] = self
            .db
            .as_ref()?
            .prepare_cached("SELECT digest FROM git_alias WHERE blob=?1")
            .ok()?
            .query_row([blob], |r| r.get::<_, Vec<u8>>(0))
            .optional()
            .ok()??
            .try_into()
            .ok()?;
        self.aliases.insert(blob.to_owned(), d);
        Some(d)
    }

    /// Records that a Git blob's bytes have `digest`.
    pub(crate) fn record_alias(&mut self, blob: &[u8], digest: [u8; 32]) {
        if self.aliases.insert(blob.to_owned(), digest) != Some(digest) {
            self.new_aliases.push((blob.to_owned(), digest));
        }
    }

    /// A clean Git file's digest, unread: its stat is its index entry's (the caller checked), and `blob` its blob.
    pub(crate) fn git_digest(&mut self, path: &Path, blob: &[u8]) -> Option<[u8; 32]> {
        let d = self.alias(blob)?;
        self.counts.git += 1;
        let _ = path;
        Some(d)
    }

    /// Writes what changed since the last flush to the store, in one transaction.
    pub(crate) fn flush(&mut self) {
        let Some(db) = self.db.as_mut() else {
            self.dirty.clear();
            self.new_aliases.clear();
            return;
        };
        if self.dirty.is_empty() && self.new_aliases.is_empty() {
            return;
        }
        let Ok(tx) = db.transaction() else { return };
        {
            let Ok(mut put) = tx.prepare_cached("INSERT OR REPLACE INTO observed VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)") else { return };
            self.dirty.sort();
            self.dirty.dedup();
            for path in &self.dirty {
                let Some(e) = self.entries.get(path) else { continue };
                let listing = e.listing.as_ref().and_then(|l| encode_listing(l));
                let s = e.stat;
                let _ = put.execute(params![path.to_string_lossy(), e.kind.code(), s.dev, s.ino, s.size, s.mtime_ns, s.ctime_ns, &e.digest[..], listing, e.recorded_ns]);
            }
            let Ok(mut alias) = tx.prepare_cached("INSERT OR REPLACE INTO git_alias VALUES (?1, ?2)") else { return };
            for (blob, d) in &self.new_aliases {
                let _ = alias.execute(params![blob, &d[..]]);
            }
        }
        if tx.commit().is_ok() {
            self.dirty.clear();
            self.new_aliases.clear();
        }
    }
}

/// The P0 store beside a content store: `<content store>.files`.
pub(crate) fn store_beside(content_store: &Path) -> PathBuf {
    let mut name = content_store.as_os_str().to_owned();
    name.push(".files");
    PathBuf::from(name)
}

/// Runs `f` with the observer of the P0 store at `store` (`None`: in memory, for the process). One observer per
/// store, for the process.
pub(crate) fn with<T>(store: Option<&Path>, f: impl FnOnce(&mut Observer) -> T) -> T {
    static OBSERVERS: OnceLock<Mutex<HashMap<Option<PathBuf>, Observer>>> = OnceLock::new();
    let all = OBSERVERS.get_or_init(Default::default);
    let mut all = all.lock().unwrap_or_else(|e| e.into_inner());
    let key = store.map(Path::to_path_buf);
    let observer = all.entry(key.clone()).or_insert_with(|| Observer::open(key.as_deref()));
    f(observer)
}

/// A hex digest, as records keep them.
pub(crate) fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut second = Observer::open(Some(&store));
        second.validate_every = 0;
        assert_eq!(second.digest(&file), Some(d1));
        assert_eq!((second.counts.hashed, second.counts.reused), (0, 1), "unchanged: one stat, no read");
        std::fs::write(&file, "x = 2\n").unwrap();
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

    #[test]
    fn listings_round_trip_through_the_store_encoding() {
        let l = vec![("a".to_owned(), true), ("b.py".to_owned(), false)];
        assert_eq!(decode_listing(&encode_listing(&l).unwrap()).unwrap(), l);
        assert_eq!(decode_listing(&encode_listing(&[]).unwrap()).unwrap(), Vec::<(String, bool)>::new());
        assert!(encode_listing(&[("a\nb".to_owned(), false)]).is_none());
    }
}
