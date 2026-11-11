//! One string interner for the graph: every name, key, path and content hash is a `Sym` (u32).
//! Strings live back to back in one byte arena; the table holds only ids. Append-only: a string
//! interned by any worktree keeps its id for every other (ids are shared across base and deltas).
//!
//! The store's single interner (C12, docs/final-shape/03-architecture.md §10.6; decision 0049). A
//! process holds one `Atoms` per database: the arena, shared by the store and the planner's symbols.
//! On disk the arena is `atom_segment` rows, each a run of consecutive atoms (their end offsets and
//! their bytes), appended by a flush; SQL reads atoms through the `atoms` virtual table over the
//! arena (`vtab`). Ids from `stored` on are pending: given out in memory and not yet written.
//!
//! The invariant (a crash must never leave an inconsistent store): no committed row names a pending
//! atom. Every write transaction on the database appends the pending atoms first, in the same
//! transaction (`write_pending`, then `committed` after the commit), and a commit hook refuses one
//! that writes the database without having done so. A crash loses only pending atoms, which nothing
//! durable names; reopening rebuilds the arena from the segments.
//!
//! Other processes: pending ids are given out only under the atoms lease, an exclusive file lock
//! beside the database (`<db>-atoms.lock`), which the operating system releases when its process
//! dies. A process that waits for it holds a shared lock on `<db>-atoms.want`; the holder sees that
//! at its next interning and flushes, which gives the lease back.
use super::{StoreError, StoreResult};
use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, ffi};
use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

mod vtab;

pub type Sym = u32;

/// The fixed hash every string key uses: the interner's table, and the dependency keys of
/// probes (`Engine::probe`), which must agree for strings that are not (yet) interned.
pub fn hash_str(s: &str) -> u64 {
    foldhash::quality::FixedState::with_seed(0x5EED_71).hash_one(s)
}

/// A run of consecutive stored atoms held in memory, as a segment row has them (decision 0111 §2).
struct Held {
    /// End offsets in `bytes`, one per atom (start = the previous end).
    ends: Vec<u32>,
    bytes: Vec<u8>,
    hashes: Vec<u64>,
}

impl Held {
    fn str(&self, i: usize) -> &str {
        let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
        // SAFETY: only whole, checked `&str`s are held.
        unsafe { std::str::from_utf8_unchecked(&self.bytes[start..self.ends[i] as usize]) }
    }
}

/// The atoms a process holds: the stored segments it has read (not all of them, decision 0111), and the atoms from
/// `base` on, given out here and not yet written (pending), back to back. Ids below `base` are in the database.
pub struct Interner {
    held: std::collections::BTreeMap<Sym, Held>,
    base: Sym,
    buf: Vec<u8>,
    /// ends[i] = end offset of pending atom `base + i` in `buf`.
    ends: Vec<u32>,
    hashes: Vec<u64>,
    table: hashbrown::HashTable<Sym>,
}

impl Default for Interner {
    fn default() -> Self {
        // Sym 0 is the empty string, the schema's first segment: stored from the start.
        let mut i = Interner { held: Default::default(), base: 0, buf: Vec::new(), ends: Vec::new(), hashes: Vec::new(), table: hashbrown::HashTable::new() };
        i.intern("");
        i.store_upto(1);
        i
    }
}

impl Interner {
    /// The string of `s`, if this process holds it (pending, or of a segment it has read).
    pub fn str(&self, s: Sym) -> Option<&str> {
        if s >= self.base {
            let i = (s - self.base) as usize;
            let &end = self.ends.get(i)?;
            let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
            // SAFETY: only whole `&str`s are appended to `buf`.
            return Some(unsafe { std::str::from_utf8_unchecked(&self.buf[start..end as usize]) });
        }
        let (first, held) = self.held.range(..=s).next_back()?;
        let i = (s - first) as usize;
        (i < held.ends.len()).then(|| held.str(i))
    }

    fn hash_of(held: &std::collections::BTreeMap<Sym, Held>, base: Sym, hashes: &[u64], s: Sym) -> u64 {
        if s >= base {
            return hashes[(s - base) as usize];
        }
        let (first, h) = held.range(..=s).next_back().expect("a held atom");
        h.hashes[(s - first) as usize]
    }

    pub fn get(&self, s: &str) -> Option<Sym> {
        self.get_h(s, hash_str(s))
    }

    pub fn get_h(&self, s: &str, h: u64) -> Option<Sym> {
        self.table.find(h, |&id| Self::hash_of(&self.held, self.base, &self.hashes, id) == h && self.str(id) == Some(s)).copied()
    }

    pub fn intern(&mut self, s: &str) -> Sym {
        let h = hash_str(s);
        if let Some(id) = self.get_h(s, h) {
            return id;
        }
        self.push(s, h)
    }

    fn push(&mut self, s: &str, h: u64) -> Sym {
        let id = self.len() as Sym;
        self.buf.extend_from_slice(s.as_bytes());
        assert!(self.buf.len() <= u32::MAX as usize, "pending atoms over 4 GiB");
        self.ends.push(self.buf.len() as u32);
        self.hashes.push(h);
        let (held, base, hashes) = (&self.held, self.base, &self.hashes);
        self.table.insert_unique(h, id, |&x| Self::hash_of(held, base, hashes, x));
        id
    }

    /// One past the last id: the database's (as far as this process knows) and the pending ones.
    pub fn len(&self) -> usize {
        self.base as usize + self.ends.len()
    }

    /// Where the pending atoms start: ids below it are in the database.
    fn base(&self) -> usize {
        self.base as usize
    }

    /// How many atoms are pending, and their bytes.
    fn pending(&self) -> (usize, usize) {
        (self.ends.len(), self.buf.len())
    }

    /// Heap bytes held (capacities).
    pub fn bytes(&self) -> usize {
        let held: usize = self.held.values().map(|h| h.bytes.capacity() + h.ends.capacity() * 4 + h.hashes.capacity() * 8 + 48).sum();
        held + self.buf.capacity() + self.ends.capacity() * 4 + self.hashes.capacity() * 8 + self.table.capacity() * 5
    }

    /// The pending atoms from `from` on as one segment: their end offsets (u32, little-endian, relative to the
    /// segment's first byte), their bytes, and each one's (hash, id).
    fn segment(&self, from: usize) -> (Vec<u8>, Vec<u8>, Vec<(u64, Sym)>) {
        let i = from - self.base();
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        let ends = self.ends[i..].iter().flat_map(|&end| (end - start).to_le_bytes()).collect();
        let keys = self.hashes[i..].iter().enumerate().map(|(k, &h)| (h, (from + k) as Sym)).collect();
        (ends, self.buf[start as usize..].to_vec(), keys)
    }

    /// The pending atoms below `upto` are stored now: held as a segment of their own.
    fn store_upto(&mut self, upto: usize) {
        let n = upto.saturating_sub(self.base()).min(self.ends.len());
        if n == 0 {
            return;
        }
        let cut = self.ends[n - 1] as usize;
        let bytes = self.buf[..cut].to_vec();
        self.buf.drain(..cut);
        let ends: Vec<u32> = self.ends.drain(..n).collect();
        for end in &mut self.ends {
            *end -= cut as u32;
        }
        let hashes: Vec<u64> = self.hashes.drain(..n).collect();
        self.held.insert(self.base, Held { ends, bytes, hashes });
        self.base += n as Sym;
    }

    /// The database's ids end at `end`, past what this process knew, and none is pending here: new ids continue
    /// from there (the atoms between are read when asked for).
    fn advance(&mut self, end: usize) -> StoreResult<()> {
        if end <= self.base() {
            return Ok(());
        }
        if !self.ends.is_empty() {
            return Err(StoreError::Corrupt("another process wrote atoms while this one had atoms pending".to_owned()));
        }
        self.base = end as Sym;
        Ok(())
    }

    /// Whether this process holds stored atom `s`'s string.
    fn holds(&self, s: Sym) -> bool {
        s >= self.base || self.held.range(..=s).next_back().is_some_and(|(first, h)| ((s - first) as usize) < h.ends.len())
    }

    /// Holds a stored segment read from the database: atoms `first..`. Those already held are skipped; a string
    /// held twice is refused as corrupt.
    fn add_segment(&mut self, first: Sym, ends: &[u8], bytes: &[u8]) -> StoreResult<()> {
        let corrupt = |what: &str| StoreError::Corrupt(format!("atom segment {first}: {what}"));
        if ends.len() % 4 != 0 {
            return Err(corrupt("its end offsets are not u32s"));
        }
        let n = ends.len() / 4;
        if first as usize + n > self.base() {
            return Err(corrupt("it reaches past the atoms the database holds"));
        }
        let mut start = 0usize;
        // The atoms not held yet, as runs of consecutive ids.
        let mut run: Option<(Sym, Held)> = None;
        let mut runs = Vec::new();
        for (k, end) in ends.chunks_exact(4).enumerate() {
            let end = u32::from_le_bytes(end.try_into().expect("4 bytes")) as usize;
            let s = bytes.get(start..end).ok_or_else(|| corrupt("an end offset is out of order or past its bytes"))?;
            let s = std::str::from_utf8(s).map_err(|_| corrupt("a string is not UTF-8"))?;
            let id = first + k as Sym;
            start = end;
            if self.holds(id) {
                runs.extend(run.take());
                continue;
            }
            let (_, held) = run.get_or_insert_with(|| (id, Held { ends: Vec::new(), bytes: Vec::new(), hashes: Vec::new() }));
            held.bytes.extend_from_slice(s.as_bytes());
            held.ends.push(held.bytes.len() as u32);
            held.hashes.push(hash_str(s));
        }
        if start != bytes.len() {
            return Err(corrupt("bytes follow its last string"));
        }
        runs.extend(run);
        for (at, held) in runs {
            for (k, &h) in held.hashes.iter().enumerate() {
                if self.get_h(held.str(k), h).is_some() {
                    return Err(corrupt("a string is stored twice"));
                }
            }
            let n = held.hashes.len();
            self.held.insert(at, held);
            for k in 0..n {
                let id = at + k as Sym;
                let h = self.held[&at].hashes[k];
                let (held, base, hashes) = (&self.held, self.base, &self.hashes);
                self.table.insert_unique(h, id, |&x| Self::hash_of(held, base, hashes, x));
            }
        }
        Ok(())
    }
}

/// More pending atoms than this, more pending bytes, or atoms pending this long, make a flush due
/// (decision 0049 §4.2): what a crash can lose.
const DUE_ATOMS: usize = 256 * 1024;
const DUE_BYTES: usize = 16 << 20;
const DUE_AFTER: Duration = Duration::from_secs(30);
/// How long a process waits for another's atoms lease before giving up (`StoreError::Busy`).
const LEASE_WAIT: Duration = Duration::from_secs(60);
/// How often a holder looks for a process waiting for its lease.
const WANT_CHECK: Duration = Duration::from_millis(50);

struct State {
    /// The atoms held: what this process read of the database's, and its pending ones (`Interner::base` on).
    atoms: Interner,
    /// When the first pending atom was given out.
    since: Option<Instant>,
    /// Every stored atom below this is held (`read_all`): a scan reads only the segments from it on.
    all_held: usize,
}

/// A process's atoms of one database (decision 0049): the arena, its watermark and the lease.
pub struct Atoms {
    /// The database file; `None` for an in-memory database, whose atoms are never written.
    path: Option<PathBuf>,
    /// The store's identity (`store_identity`) when these are a store's own atoms: a store created again at the
    /// path has another, and atoms of its own (`for_store`).
    identity: Option<i64>,
    state: RwLock<State>,
    /// Held (the lock file, exclusively locked) while this process may give out pending ids.
    /// Lock order: `lease`, then `reader`, then `state`.
    lease: Mutex<Option<File>>,
    leased: AtomicBool,
    /// When this holder last looked for a waiter, and what it saw.
    wanted: Mutex<(Option<Instant>, bool)>,
    /// A connection of this process's own, for reading the database's segments and key (decision 0111).
    /// Lock order: `keyed`, then `reader`.
    reader: Mutex<Option<Connection>>,
    /// Whether the database's key from hash to id covers its segments (decision 0111 §1): found once, true.
    keyed: Mutex<bool>,
    /// Per connection (by handle): whether its open transaction appended the pending atoms.
    flushed: Mutex<HashMap<usize, Arc<AtomicBool>>>,
    /// Every id, by string, for range scans of `atoms.value` (`vtab`); extended when read.
    sorted: Mutex<Vec<Sym>>,
    /// Segments this process appended, their atoms and their bytes (the profile's `atoms` line).
    written: Mutex<(u64, u64, u64)>,
}

thread_local! {
    /// The atoms this thread made while reserving (`reserving`), by arena.
    static RESERVING: std::cell::RefCell<Option<(usize, Vec<(usize, Vec<Sym>)>)>> = const { std::cell::RefCell::new(None) };
}

/// Atoms reserved by components not yet retired, by arena (its address): pending, but no row a commit writes holds
/// them yet (decision 0114 §4).
static RESERVED: LazyLock<parking_lot::Mutex<HashMap<usize, std::collections::BTreeSet<Sym>>>> = LazyLock::new(Default::default);

/// Atoms made ahead of the run's order, by a component of strata (decision 0114 §4: the atoms' publication model).
///
/// - **Allocation.** Ids come from the shared arena, on any thread, as any atom's: unique, and never reused.
/// - **Visibility.** An atom's string is readable by every thread as soon as it is made; rows that hold it are
///   visible to other strata only when its component retires (the component's relations are published then).
/// - **Publication.** Until then the atom is reserved: pending, and written by any flush, but not counted by
///   the commit check of decision 0049, since no row a committing transaction writes can hold it yet. At retire
///   (`Reserved`'s drop) it is an ordinary pending atom, which the next commit writing rows must append first.
///
/// So ids follow the order atoms were first made in, which under parallel strata is not the serial run's: an id
/// names a string, and nothing reads its order.
///
/// Only atoms made in `arena` (the component's scope's) are reserved: another run's work this thread helps with
/// while its component waits makes its atoms pending as usual.
pub fn reserving(arena: &Arc<Atoms>) -> Reservation {
    RESERVING.with_borrow_mut(|r| {
        assert!(r.is_none(), "one reservation per thread at a time");
        *r = Some((Arc::as_ptr(arena) as usize, Vec::new()));
    });
    Reservation(())
}

/// A thread's reservation in progress (`reserving`): `end` gives what it made.
pub struct Reservation(());

impl Reservation {
    pub fn end(self) -> Reserved {
        let made = RESERVING.with_borrow_mut(Option::take).map(|r| r.1).unwrap_or_default();
        std::mem::forget(self);
        Reserved(made)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        drop(Reserved(RESERVING.with_borrow_mut(Option::take).map(|r| r.1).unwrap_or_default()));
    }
}

/// The atoms a component reserved: published (counted as pending) when dropped, as the component retires.
#[derive(Default)]
pub struct Reserved(Vec<(usize, Vec<Sym>)>);

impl Reserved {
    pub fn len(&self) -> usize {
        self.0.iter().map(|(_, ids)| ids.len()).sum()
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        let mut all = RESERVED.lock();
        for (at, ids) in self.0.drain(..) {
            if let Some(set) = all.get_mut(&at) {
                ids.iter().for_each(|id| { set.remove(id); });
                if set.is_empty() {
                    all.remove(&at);
                }
            }
        }
    }
}

static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Weak<Atoms>>>> = LazyLock::new(Default::default);
/// The atoms of in-memory databases: one set for the process, as the planner's symbols had.
static MEMORY: LazyLock<Arc<Atoms>> = LazyLock::new(|| Arc::new(Atoms::new(None, None)));

/// A path's registry key: its canonical form, resolved once per path (a `realpath` per stratum was 2.5% of a corpus
/// run's main thread). A path that does not resolve yet is not remembered.
fn key(path: &Path) -> PathBuf {
    static CANONICAL: LazyLock<Mutex<HashMap<PathBuf, PathBuf>>> = LazyLock::new(Default::default);
    if let Some(key) = CANONICAL.lock().get(path) {
        return key.clone();
    }
    match std::fs::canonicalize(path) {
        Ok(key) => {
            CANONICAL.lock().insert(path.to_path_buf(), key.clone());
            key
        }
        Err(_) => path.to_path_buf(),
    }
}

/// The file's exclusive lock, without waiting. WASI has no file locks; the browser build is the only process on
/// its in-memory filesystem, so the lock is always free there.
fn try_lock(file: &File) -> Result<(), TryLockError> {
    #[cfg(target_family = "wasm")]
    {
        let _ = file;
        Ok(())
    }
    #[cfg(not(target_family = "wasm"))]
    {
        file.try_lock()
    }
}

/// `path` with `suffix` appended to its file name (the arena's lock and journal files).
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The key from hash to id (decision 0111 §1): a row per stored atom, and how far it reaches.
const KEY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS atom_key (h INTEGER NOT NULL, id INTEGER NOT NULL, PRIMARY KEY (h, id)) WITHOUT ROWID, STRICT;
CREATE TABLE IF NOT EXISTS atom_key_end (one INTEGER PRIMARY KEY CHECK (one = 0), upto INTEGER NOT NULL) STRICT;";

/// Where the database's atom ids end (its segments' last first plus its count).
fn database_end(connection: &Connection) -> rusqlite::Result<usize> {
    let end: Option<i64> = connection
        .prepare_cached("SELECT first + length(ends) / 4 FROM atom_segment ORDER BY first DESC LIMIT 1")?
        .query_row([], |row| row.get(0))
        .optional()?;
    Ok(end.unwrap_or(0) as usize)
}

/// How far the database's key reaches: `None` when it has none.
fn key_end(connection: &Connection) -> Option<usize> {
    let mut statement = connection.prepare_cached("SELECT upto FROM atom_key_end").ok()?;
    statement.query_row([], |row| row.get::<_, i64>(0)).optional().ok().flatten().map(|upto| upto as usize)
}

/// Keys the segments the database's key does not reach, in one transaction: once for an older store, or after a
/// capture copied segments in. False when the database cannot be written now (read-only, or another writer past
/// the busy timeout): lookups then read every segment, and the next process tries again.
fn key_segments(connection: &Connection) -> StoreResult<bool> {
    let end = database_end(connection)?;
    if key_end(connection).is_some_and(|upto| upto >= end) {
        return Ok(true);
    }
    if connection.execute_batch("BEGIN IMMEDIATE").is_err() {
        return Ok(false);
    }
    let keyed = (|| -> StoreResult<()> {
        connection.execute_batch(KEY_SCHEMA)?;
        let from = key_end(connection).unwrap_or(0);
        let mut keys: Vec<(i64, i64)> = Vec::new();
        {
            let mut statement = connection.prepare("SELECT first, ends, bytes FROM atom_segment WHERE first >= ?1 ORDER BY first")?;
            let mut rows = statement.query([from as i64])?;
            while let Some(row) = rows.next()? {
                let (first, ends, bytes): (i64, Vec<u8>, Vec<u8>) = (row.get(0)?, row.get(1)?, row.get(2)?);
                let mut start = 0usize;
                for (k, end) in ends.chunks_exact(4).enumerate() {
                    let end = u32::from_le_bytes(end.try_into().expect("4 bytes")) as usize;
                    let s = bytes.get(start..end).and_then(|s| std::str::from_utf8(s).ok()).ok_or_else(|| StoreError::Corrupt(format!("atom segment {first}: a string is not UTF-8 or out of its bytes")))?;
                    keys.push((hash_str(s) as i64, first + k as i64));
                    start = end;
                }
            }
        }
        keys.sort_unstable();
        let mut insert = connection.prepare("INSERT OR IGNORE INTO atom_key(h, id) VALUES (?1, ?2)")?;
        for (h, id) in keys {
            insert.execute([h, id])?;
        }
        connection.execute("INSERT OR REPLACE INTO atom_key_end(one, upto) VALUES (0, ?1)", [database_end(connection)? as i64])?;
        Ok(())
    })();
    match keyed {
        Ok(()) => {
            connection.execute_batch("COMMIT")?;
            Ok(true)
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

impl Atoms {
    fn new(path: Option<PathBuf>, identity: Option<i64>) -> Self {
        Atoms {
            path,
            identity,
            // Atom 0, the empty string, is the schema's first segment.
            state: RwLock::new(State { atoms: Interner::default(), since: None, all_held: 1 }),
            lease: Mutex::new(None),
            leased: AtomicBool::new(false),
            wanted: Mutex::new((None, false)),
            reader: Mutex::new(None),
            keyed: Mutex::new(false),
            flushed: Mutex::new(HashMap::new()),
            sorted: Mutex::new(Vec::new()),
            written: Mutex::new((0, 0, 0)),
        }
    }

    /// This process's atoms of the database at `path`, as it holds them (`install` checks they are the database's).
    fn for_path(path: &Path) -> Arc<Atoms> {
        let key = key(path);
        let mut registry = REGISTRY.lock();
        if let Some(atoms) = registry.get(&key).and_then(Weak::upgrade) {
            return atoms;
        }
        let atoms = Arc::new(Atoms::new(Some(key.clone()), None));
        registry.insert(key, Arc::downgrade(&atoms));
        atoms
    }

    /// This process's atoms of the database at `path`, whose identity is `identity` (`store_identity`; `None`: not a
    /// store): those it holds for the path, unless they are of a store that was there before (deleted and created
    /// again, by this process or another: it has another identity). A store has atoms of its own; another database
    /// (a run's scratch) may borrow a store's (`share`).
    pub(crate) fn for_database(path: &Path, identity: Option<i64>) -> Arc<Atoms> {
        let key = key(path);
        let mut registry = REGISTRY.lock();
        if let Some(atoms) = registry.get(&key).and_then(Weak::upgrade) {
            let held = if atoms.path.as_ref() == Some(&key) { atoms.identity == identity } else { identity.is_none() };
            if held {
                return atoms;
            }
        }
        let atoms = Arc::new(Atoms::new(Some(key.clone()), identity));
        registry.insert(key, Arc::downgrade(&atoms));
        atoms
    }

    /// The identity of `connection`'s main database if it is a store (`store_identity`).
    fn identity(connection: &Connection) -> rusqlite::Result<Option<i64>> {
        let store: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM main.sqlite_schema WHERE type = 'table' AND name = 'store_identity')",
            [],
            |row| row.get(0),
        )?;
        if !store {
            return Ok(None);
        }
        connection.query_row("SELECT id FROM main.store_identity WHERE singleton = 1", [], |row| row.get(0)).optional()
    }

    /// Makes `path`'s database read and write these atoms in this process: a run's working database
    /// (its SQL's scratch) whose symbols are the store's (decision 0052).
    pub fn share(self: &Arc<Self>, path: &Path) {
        REGISTRY.lock().insert(key(path), Arc::downgrade(self));
    }

    /// This process's atoms of `connection`'s main database.
    pub fn for_connection(connection: &Connection) -> Arc<Atoms> {
        match connection.path().filter(|path| !path.is_empty()) {
            Some(path) => Atoms::for_path(Path::new(path)),
            None => Arc::clone(&MEMORY),
        }
    }

    /// Shared scratch databases borrow the arena; only its owner stores atom segments.
    fn owns(&self, connection: &Connection) -> bool {
        match (&self.path, connection.path()) {
            (Some(owner), Some(path)) => *owner == key(Path::new(path)),
            _ => false,
        }
    }

    /// Registers the `atoms` virtual table on `connection` and, for a database on disk, the commit
    /// hook that refuses a commit writing it while atoms are pending that the transaction did not
    /// append. Called where the connection is opened, which owns it: SQLite's hooks are set through
    /// `&mut Connection`. Code handed the connection later registers the table alone (`attach`).
    pub fn install(connection: &mut Connection) -> rusqlite::Result<Arc<Atoms>> {
        let atoms = Atoms::attach(connection)?;
        if atoms.owns(connection) {
            // SAFETY: the handle is only read by the hooks, which run on this connection.
            let handle = unsafe { connection.handle() } as usize;
            let flushed = Arc::clone(atoms.flushed.lock().entry(handle).or_default());
            flushed.store(false, Ordering::Relaxed);
            let (hook, held) = (Arc::clone(&flushed), Arc::clone(&atoms));
            connection.commit_hook(Some(move || {
                let flushed = hook.swap(false, Ordering::AcqRel);
                if flushed || !held.pending() {
                    return false;
                }
                // SAFETY: called by SQLite on this connection's thread, under its mutex.
                let state = unsafe { ffi::sqlite3_txn_state(handle as *mut ffi::sqlite3, c"main".as_ptr()) };
                if state == ffi::SQLITE_TXN_WRITE {
                    eprintln!("rerail: atoms: a commit that writes the store without its pending atoms was refused (decision 0049)");
                    if std::env::var_os("RERAIL_ATOMS_BACKTRACE").is_some() {
                        eprintln!("{}", std::backtrace::Backtrace::force_capture());
                    }
                    return true;
                }
                false
            }))?;
            connection.rollback_hook(Some(move || flushed.store(false, Ordering::Relaxed)))?;
        }
        Ok(atoms)
    }

    /// Registers the `atoms` virtual table on `connection`, whose opener registered the commit hook
    /// (`install`).
    pub fn attach(connection: &Connection) -> rusqlite::Result<Arc<Atoms>> {
        let atoms = match connection.path().filter(|path| !path.is_empty()) {
            Some(path) => Atoms::for_database(Path::new(path), Atoms::identity(connection)?),
            None => Arc::clone(&MEMORY),
        };
        vtab::install(connection, &atoms)?;
        Ok(atoms)
    }

    pub fn len(&self) -> usize {
        self.state.read().atoms.len()
    }

    /// Segments this process appended so far: (segments, atoms, bytes), and the atoms pending now.
    pub fn writes(&self) -> ((u64, u64, u64), usize) {
        let pending = self.state.read().atoms.pending().0;
        (*self.written.lock(), pending)
    }

    /// Heap bytes the atoms held take.
    pub fn bytes(&self) -> usize {
        self.state.read().atoms.bytes()
    }

    /// Whether atoms are pending (given out, not yet in the database).
    pub fn pending(&self) -> bool {
        if self.path.is_none() {
            return false;
        }
        let state = self.state.read();
        let (n, _) = state.atoms.pending();
        if n == 0 {
            return false;
        }
        // Atoms a component reserves (`reserving`) are no row's that a commit may write yet: not counted.
        let base = state.atoms.base() as Sym;
        let reserved = RESERVED.lock().get(&(self as *const Atoms as usize)).map_or(0, |r| r.range(base..).count());
        n > reserved
    }

    /// The atom of `value`, if the database or this process holds it. A miss in memory asks the database (decision
    /// 0111 §2): it no longer means "not interned".
    pub fn get(&self, value: &str) -> Option<Sym> {
        self.lookup(value).ok().flatten()
    }

    /// `f` of atom `atom`'s string, if the database or this process holds it: a stored atom not held yet is read
    /// with its segment.
    pub fn with_str<R>(&self, atom: Sym, f: impl FnOnce(&str) -> R) -> Option<R> {
        {
            let state = self.state.read();
            if let Some(value) = state.atoms.str(atom) {
                return Some(f(value));
            }
            if atom as usize >= state.atoms.base() {
                return None;
            }
        }
        self.read_atoms(&[atom]).ok()?;
        self.state.read().atoms.str(atom).map(f)
    }

    /// The atom of `value`, if the database or this process holds it.
    pub fn lookup(&self, value: &str) -> StoreResult<Option<Sym>> {
        if let Some(atom) = self.state.read().atoms.get(value) {
            return Ok(Some(atom));
        }
        if !self.leased.load(Ordering::Acquire) {
            self.catch_up()?;
        }
        self.find(&[value])?;
        Ok(self.state.read().atoms.get(value))
    }

    /// The string of `atom`, if the database or this process holds it.
    pub fn value(&self, atom: Sym) -> StoreResult<Option<String>> {
        if (atom as usize) >= self.len() {
            self.catch_up()?;
        }
        self.read_atoms(&[atom])?;
        Ok(self.state.read().atoms.str(atom).map(str::to_owned))
    }

    /// Reads into the arena the atoms the database holds that it does not (an arena made after the store was
    /// written holds none of them).
    pub fn read_stored(&self) -> StoreResult<()> {
        self.catch_up()
    }

    /// Closes this process's connection for reading other processes' segments (reopened when next needed): the
    /// store's last handle closed, and the arena, which the planner's symbols keep, must not hold its file open.
    pub(crate) fn release_reader(&self) {
        self.reader.lock().take();
    }

    /// The atom of each of `values`, giving out new ones (pending) under the lease.
    pub fn intern_all(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
        {
            let state = self.state.read();
            if let Some(found) = values.iter().map(|value| state.atoms.get(value)).collect::<Option<Vec<_>>>() {
                return Ok(found);
            }
        }
        // The lease is held while ids are given out, so a flush on another thread cannot give it
        // back between the check and the new ids.
        let mut lease = self.lease.lock();
        self.acquire(&mut lease)?;
        // What this process does not hold may be stored: found in the database first (decision 0111).
        let missing: Vec<&str> = {
            let state = self.state.read();
            values.iter().copied().filter(|value| state.atoms.get(value).is_none()).collect()
        };
        if !missing.is_empty() {
            self.find(&missing)?;
        }
        let mut state = self.state.write();
        let before = state.atoms.len() as Sym;
        let ids: Vec<Sym> = values.iter().map(|value| state.atoms.intern(value)).collect();
        if state.since.is_none() && state.atoms.pending().0 > 0 {
            state.since = Some(Instant::now());
        }
        // Made while this thread reserves (`reserving`): the ids are its component's until it retires.
        RESERVING.with_borrow_mut(|r| {
            if let Some((arena, r)) = r.as_mut()
                && *arena == self as *const Atoms as usize
            {
                let new: Vec<Sym> = ids.iter().copied().filter(|&id| id >= before).collect();
                if !new.is_empty() {
                    RESERVED.lock().entry(self as *const Atoms as usize).or_default().extend(new.iter().copied());
                    r.push((self as *const Atoms as usize, new));
                }
            }
        });
        Ok(ids)
    }

    /// The atom of `value`, given out (pending) under the lease if it is new.
    pub fn intern(&self, value: &str) -> StoreResult<Sym> {
        if let Some(atom) = self.state.read().atoms.get(value) {
            return Ok(atom);
        }
        Ok(self.intern_all(&[value])?[0])
    }

    /// Takes the lease if this process does not hold it, then learns where the database's ids end.
    fn acquire(&self, lease: &mut Option<File>) -> StoreResult<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if lease.is_some() {
            return Ok(());
        }
        let open = |suffix: &str| {
            std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(suffixed(path, suffix))
        };
        let file = open("-atoms.lock")?;
        match try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::Error(error)) => return Err(error.into()),
            Err(TryLockError::WouldBlock) => {
                // Asks the holder to flush and give the lease back.
                let want = open("-atoms.want")?;
                want.lock_shared()?;
                let deadline = Instant::now() + LEASE_WAIT;
                let mut pause = Duration::from_millis(1);
                loop {
                    std::thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(20));
                    match try_lock(&file) {
                        Ok(()) => break,
                        Err(TryLockError::Error(error)) => return Err(error.into()),
                        Err(TryLockError::WouldBlock) if Instant::now() < deadline => {}
                        Err(TryLockError::WouldBlock) => return Err(StoreError::Busy),
                    }
                }
            }
        }
        *lease = Some(file);
        self.leased.store(true, Ordering::Release);
        // No other process gives out ids now: new ones continue from where the database's end.
        self.read_end()
    }

    /// Learns where the database's ids end, unless this process holds the lease (then it knows).
    fn catch_up(&self) -> StoreResult<()> {
        if self.leased.load(Ordering::Acquire) {
            return Ok(());
        }
        // Under the lease's mutex: no thread of this process gives out ids while it reads.
        let lease = self.lease.lock();
        if lease.is_some() {
            return Ok(());
        }
        self.read_end()
    }

    /// `f` on this process's own connection to the database, opened when first needed; `None` when the database
    /// is not on disk (yet).
    fn with_reader<R>(&self, f: impl FnOnce(&Connection) -> StoreResult<R>) -> StoreResult<Option<R>> {
        let Some(path) = &self.path else { return Ok(None) };
        let mut reader = self.reader.lock();
        if reader.is_none() {
            if !path.exists() {
                return Ok(None);
            }
            let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            connection.busy_timeout(Duration::from_secs(5))?;
            rusqlite::vtab::array::load_module(&connection)?;
            *reader = Some(connection);
        }
        f(reader.as_ref().expect("opened above")).map(Some)
    }

    /// Where the database's ids end: new ids continue from there (decision 0111 §2: the atoms between are read
    /// when asked for).
    fn read_end(&self) -> StoreResult<()> {
        let path = self.path.clone();
        let Some(end) = self.with_reader(|connection| {
            database_end(connection).map_err(|error| StoreError::Corrupt(format!("{}: reading atom segments: {error}", path.as_deref().unwrap_or(Path::new("")).display())))
        })?
        else {
            return Ok(());
        };
        self.state.write().atoms.advance(end)
    }

    /// Reads into this process the segments of the stored atoms among `values` (decision 0111 §1): by the database's
    /// key from hash to id, or, where the database has none it can be given (a read-only copy), every segment.
    fn find(&self, values: &[&str]) -> StoreResult<()> {
        if self.path.is_none() || values.is_empty() {
            return Ok(());
        }
        if !self.keyed()? {
            return self.read_all();
        }
        let hashes: Vec<rusqlite::types::Value> = values.iter().map(|value| rusqlite::types::Value::Integer(hash_str(value) as i64)).collect();
        let ids = self.with_reader(|connection| {
            let mut statement = connection.prepare_cached("SELECT id FROM atom_key WHERE h IN rarray(?1)")?;
            Ok(statement.query_map([std::rc::Rc::new(hashes)], |row| row.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?)
        })?;
        let ids: Vec<Sym> = ids.unwrap_or_default().into_iter().filter_map(|id| Sym::try_from(id).ok()).collect();
        // An id past this process's end is another process's, written since: its end is read first.
        let base = self.state.read().atoms.base();
        if ids.iter().any(|&id| id as usize >= base) && !self.leased.load(Ordering::Acquire) {
            self.catch_up()?;
        }
        self.read_atoms(&ids)
    }

    /// Whether the database's key covers its segments: keyed now if it does not (an older store, or segments a
    /// capture copied in), in one transaction on this process's connection. A database that cannot be written
    /// is not keyed, and lookups read every segment (`read_all`).
    fn keyed(&self) -> StoreResult<bool> {
        let mut keyed = self.keyed.lock();
        if *keyed {
            return Ok(true);
        }
        *keyed = self.with_reader(key_segments)?.unwrap_or(false);
        Ok(*keyed)
    }

    /// Holds the segments of stored atoms `ids` that this process does not hold yet: one read a segment.
    fn read_atoms(&self, ids: &[Sym]) -> StoreResult<()> {
        let wanted: Vec<Sym> = {
            let state = self.state.read();
            let mut wanted: Vec<Sym> = ids.iter().copied().filter(|&id| (id as usize) < state.atoms.base() && !state.atoms.holds(id)).collect();
            wanted.sort_unstable();
            wanted.dedup();
            wanted
        };
        if wanted.is_empty() {
            return Ok(());
        }
        let segments = self.with_reader(|connection| {
            let mut statement = connection.prepare_cached("SELECT first, ends, bytes FROM atom_segment WHERE first <= ?1 ORDER BY first DESC LIMIT 1")?;
            let mut out: Vec<(i64, Vec<u8>, Vec<u8>)> = Vec::new();
            for &id in &wanted {
                if out.last().is_some_and(|(first, ends, _)| i64::from(id) < first + (ends.len() / 4) as i64) {
                    continue;
                }
                if let Some(segment) = statement.query_row([i64::from(id)], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()? {
                    out.push(segment);
                }
            }
            Ok(out)
        })?;
        let mut state = self.state.write();
        for (first, ends, bytes) in segments.unwrap_or_default() {
            state.atoms.add_segment(first as Sym, &ends, &bytes)?;
        }
        Ok(())
    }

    /// Holds every stored atom: for a scan or a range of the `atoms` table, and for lookups in a database with no
    /// key. Only the segments from the last such read on are read again.
    pub(super) fn read_all(&self) -> StoreResult<()> {
        if self.path.is_none() {
            return Ok(());
        }
        self.catch_up()?;
        self.with_reader(|connection| {
            let from = self.state.read().all_held;
            let mut statement = connection.prepare_cached(
                "SELECT first, ends, bytes FROM atom_segment WHERE first >= coalesce((SELECT max(first) FROM atom_segment WHERE first <= ?1), 0) ORDER BY first",
            )?;
            let mut rows = statement.query([from as i64])?;
            let mut state = self.state.write();
            while let Some(row) = rows.next()? {
                let first: i64 = row.get(0)?;
                if first as usize >= state.atoms.base() {
                    break;
                }
                state.atoms.add_segment(first as Sym, &row.get::<_, Vec<u8>>(1)?, &row.get::<_, Vec<u8>>(2)?)?;
            }
            state.all_held = state.atoms.base();
            Ok(())
        })?;
        Ok(())
    }

    /// In `connection`'s open transaction, if it writes the main database: appends the pending
    /// atoms as one segment and marks the transaction as having done so (the commit hook). Returns
    /// the watermark to pass to `committed` after the commit.
    pub fn write_pending(&self, connection: &Connection) -> StoreResult<usize> {
        if self.path.is_none() {
            return Ok(0);
        }
        if !self.owns(connection) {
            return Err(StoreError::Invalid("atom segments must be written on the arena's owning database".to_owned()));
        }
        // SAFETY: a read of the connection's transaction state on its own thread.
        let state = unsafe { ffi::sqlite3_txn_state(connection.handle(), c"main".as_ptr()) };
        if state != ffi::SQLITE_TXN_WRITE {
            return Ok(0);
        }
        // SAFETY: the handle is only used as this connection's key.
        let handle = unsafe { connection.handle() } as usize;
        if let Some(flushed) = self.flushed.lock().get(&handle) {
            flushed.store(true, Ordering::Release);
        }
        // Where the database's segments end now (another connection of this process may have
        // committed a flush whose `committed` has not run yet).
        let end: i64 = connection
            .prepare_cached("SELECT first + length(ends) / 4 FROM atom_segment ORDER BY first DESC LIMIT 1")?
            .query_row([], |row| row.get(0))
            .optional()?
            .unwrap_or(0);
        let (from, ends, bytes, mut keys, upto) = {
            let state = self.state.read();
            let (end, len) = (end as usize, state.atoms.len());
            if end < state.atoms.base() {
                return Err(StoreError::Corrupt(format!("the database's atoms end at {end}, before the {} this process wrote", state.atoms.base())));
            }
            if end >= len {
                return Ok(end);
            }
            let (ends, bytes, keys) = state.atoms.segment(end);
            (end, ends, bytes, keys, len)
        };
        let size = (ends.len() + bytes.len()) as u64;
        connection
            .prepare_cached("INSERT INTO atom_segment(first, ends, bytes) VALUES (?1, ?2, ?3)")?
            .execute(rusqlite::params![from as i64, ends, bytes])?;
        // Their keys, in the same transaction, sorted so that the flush writes neighbouring leaves together
        // (decision 0111 §1). A key that does not reach this segment's first (an older store) is extended later,
        // by `key_segments`.
        if key_end(connection) == Some(from) {
            keys.sort_unstable();
            let mut insert = connection.prepare_cached("INSERT OR IGNORE INTO atom_key(h, id) VALUES (?1, ?2)")?;
            for (h, id) in keys {
                insert.execute([h as i64, i64::from(id)])?;
            }
            connection.prepare_cached("UPDATE atom_key_end SET upto = ?1")?.execute([upto as i64])?;
        }
        // Counted when written; a transaction that then rolls back still counts.
        let mut written = self.written.lock();
        *written = (written.0 + 1, written.1 + (upto - from) as u64, written.2 + size);
        Ok(upto)
    }

    /// After a commit that held `write_pending`'s segment: atoms below `upto` are stored. The
    /// lease is given back once none is pending.
    pub fn committed(&self, upto: usize) {
        if self.path.is_none() {
            return;
        }
        let mut lease = self.lease.lock();
        let mut state = self.state.write();
        state.atoms.store_upto(upto);
        if state.atoms.pending().0 == 0 {
            state.since = None;
            if lease.take().is_some() {
                self.leased.store(false, Ordering::Release);
            }
        }
    }

    /// Whether a flush is due: enough pending (count, bytes or age), or another process waiting.
    pub fn due(&self) -> bool {
        if self.path.is_none() {
            return false;
        }
        {
            let state = self.state.read();
            let (pending, bytes) = state.atoms.pending();
            if pending == 0 {
                return false;
            }
            if pending >= DUE_ATOMS
                || bytes >= DUE_BYTES
                || state.since.is_some_and(|since| since.elapsed() >= DUE_AFTER)
            {
                return true;
            }
        }
        self.wanted()
    }

    /// Whether another process waits for this one's lease (looked at most every `WANT_CHECK`).
    fn wanted(&self) -> bool {
        let Some(path) = &self.path else { return false };
        let mut wanted = self.wanted.lock();
        if wanted.0.is_some_and(|at| at.elapsed() < WANT_CHECK) {
            return wanted.1;
        }
        let seen = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(suffixed(path, "-atoms.want"))
            .is_ok_and(|file| matches!(try_lock(&file), Err(TryLockError::WouldBlock)));
        *wanted = (Some(Instant::now()), seen);
        seen
    }

    /// Appends the pending atoms in a transaction of their own on `connection`, unless it is inside
    /// a transaction (its commit will) or another writer holds the database now (a later one will).
    pub fn flush(&self, connection: &Connection) -> StoreResult<()> {
        if !self.pending() {
            return Ok(());
        }
        if !self.owns(connection) {
            // The caller can be the corpus runner's scratch connection. Under contention
            // a flush becomes due there, but scratch deliberately has no atom_segment.
            // Do not hold the reader mutex: committed() acquires the lease mutex first.
            let owner = Connection::open_with_flags(self.path.as_ref().expect("pending disk atoms"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            return self.flush(&owner);
        }
        if !connection.is_autocommit() {
            return Ok(());
        }
        if let Err(error) = connection.execute_batch("BEGIN IMMEDIATE") {
            return match error.sqlite_error_code() {
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => Ok(()),
                _ => Err(error.into()),
            };
        }
        let written = self.write_pending(connection).and_then(|upto| {
            connection.execute_batch("COMMIT")?;
            Ok(upto)
        });
        match written {
            Ok(upto) => {
                self.committed(upto);
                Ok(())
            }
            Err(error) => {
                if !connection.is_autocommit() {
                    let _ = connection.execute_batch("ROLLBACK");
                }
                Err(error)
            }
        }
    }

    /// `flush` if one is due.
    pub fn flush_if_due(&self, connection: &Connection) -> StoreResult<()> {
        if self.due() { self.flush(connection) } else { Ok(()) }
    }
}

/// Commits `connection`'s open transaction (`BEGIN` … ), appending the pending atoms first if it
/// writes the main database: for transactions written as SQL rather than through the store.
pub fn commit(connection: &Connection) -> StoreResult<()> {
    let atoms = Atoms::for_connection(connection);
    let upto = atoms.write_pending(connection)?;
    connection.execute_batch("COMMIT")?;
    atoms.committed(upto);
    Ok(())
}

/// `temp.atoms(atom, value)` on `connection`: the atoms its database's segments hold, decoded, and
/// nothing pending in any process. For checks of what a store holds on disk (after a crash, in a
/// capture), on a connection without the `atoms` virtual table; the temp table hides it otherwise.
pub fn materialize_stored(connection: &Connection) -> StoreResult<()> {
    let mut atoms: Vec<String> = vec![String::new()];
    {
        let mut statement = connection.prepare("SELECT first, ends, bytes FROM main.atom_segment WHERE first > 0 ORDER BY first")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, i64>(0)? as usize != atoms.len() {
                return Err(StoreError::Corrupt("atom segments are not contiguous".to_owned()));
            }
            let (ends, bytes): (Vec<u8>, Vec<u8>) = (row.get(1)?, row.get(2)?);
            let mut start = 0usize;
            for end in ends.chunks_exact(4) {
                let end = u32::from_le_bytes(end.try_into().expect("4 bytes")) as usize;
                let s = bytes.get(start..end).and_then(|s| std::str::from_utf8(s).ok()).ok_or_else(|| StoreError::Corrupt("atom segment: a string is not UTF-8 or out of its bytes".to_owned()))?;
                atoms.push(s.to_owned());
                start = end;
            }
        }
    }
    connection.execute_batch("DROP TABLE IF EXISTS temp.atoms; CREATE TEMP TABLE atoms(atom INTEGER PRIMARY KEY, value TEXT NOT NULL);")?;
    let mut insert = connection.prepare("INSERT INTO temp.atoms(atom, value) VALUES (?1, ?2)")?;
    for (atom, value) in atoms.iter().enumerate() {
        insert.execute(rusqlite::params![atom as i64, value])?;
    }
    Ok(())
}

impl Drop for Atoms {
    /// The last holder of a database's atoms appends what is pending (a store or the planner's
    /// symbols closing). Atoms never written are lost, as in a crash; nothing durable names them.
    fn drop(&mut self) {
        let Some(path) = &self.path else { return };
        if !self.pending() || !path.exists() {
            return;
        }
        let flushed = Connection::open(path).map_err(StoreError::from).and_then(|connection| {
            connection.busy_timeout(LEASE_WAIT)?;
            self.flush(&connection)
        });
        if let Err(error) = flushed {
            eprintln!("rerail: atoms: writing the pending atoms of {} failed: {error}", path.display());
        }
    }
}

impl super::ContentStore {
    /// The atom of each of `values`, giving out new ones. They are written with the next write
    /// transaction on the store, or in one of their own when a flush is due (decision 0049 §4).
    /// This process's atoms of the store, for a reader that interns away from the store handle (a lazily
    /// decoded source). Pending atoms are appended by the store's next flush.
    pub fn atoms_handle(&self) -> std::sync::Arc<Atoms> {
        std::sync::Arc::clone(&self.atoms)
    }

    pub fn intern_atoms(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
        let ids = self.atoms.intern_all(values)?;
        if self.atoms.due() {
            self.flush_atoms()?;
        }
        Ok(ids)
    }

    /// Appends the pending atoms now, unless another writer holds the store (a later write will).
    /// The database at `path` reads and writes this store's atoms in this process (`Atoms::share`): a
    /// connection there resolves symbols as the store does. Call before opening it.
    pub fn share_atoms_with(&self, path: &Path) {
        self.atoms.share(path);
    }

    pub fn flush_atoms(&self) -> StoreResult<()> {
        if !self.atoms.pending() {
            return Ok(());
        }
        match self.immediate_transaction_no_wait(|_| Ok(())) {
            Ok(()) | Err(StoreError::Busy) => Ok(()),
            Err(StoreError::Database(error))
                if matches!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// The atom of `value`, if the store holds it.
    pub fn atom(&self, value: &str) -> StoreResult<Option<Sym>> {
        self.atoms.lookup(value)
    }

    /// The atoms' writes so far: (segments, atoms, bytes) appended, and the atoms pending now.
    pub fn atom_writes(&self) -> ((u64, u64, u64), usize) {
        self.atoms.writes()
    }

    /// Heap bytes the arena holds (counted against the fact pool, K1).
    pub fn atom_bytes(&self) -> usize {
        self.atoms.bytes()
    }

    /// The string of `atom`, if the store holds it.
    pub fn atom_value(&self, atom: Sym) -> StoreResult<Option<String>> {
        self.atoms.value(atom)
    }

    /// Commits `transaction`, appending the pending atoms first if it writes the store.
    /// Inside the writer's batch (`super::connection::begin`) it releases its savepoint only: the batch's commit
    /// appends the atoms.
    pub(super) fn commit(&self, transaction: super::connection::Tx<'_>) -> StoreResult<()> {
        let super::connection::Tx::Top(transaction) = transaction else {
            transaction.commit()?;
            return Ok(());
        };
        let upto = self.atoms.write_pending(&transaction)?;
        transaction.commit()?;
        self.atoms.committed(upto);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
