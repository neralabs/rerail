//! One string interner for the graph: every name, key, path and content hash is a `Sym` (u32).
//! Strings live back to back in one byte arena; the table holds ids and hash tags. Append-only: a string
//! interned by any worktree keeps its id for every other (ids are shared across base and deltas).
//!
//! The store's single interner (C12, docs/final-shape/03-architecture.md §10.6; decision 0049). A
//! process holds one `Atoms` per database: the arena, shared by the store and the planner's symbols.
//! On disk the arena is a chain of atoms blocks beside the store (decision 0401, `stored`), mapped by every process
//! and read where they lie; a flush appends to the open one. SQL reads atoms through the `atoms` virtual table over
//! the arena (`vtab`). Ids from `base` on are pending: given out in memory and not yet written.
//!
//! The invariant (a crash must never leave an inconsistent store): no committed row names a pending
//! atom. Every write transaction on the database appends the pending atoms first, in the same
//! transaction (`write_pending`, then `committed` after the commit), and a commit hook refuses one
//! that writes the database without having done so. A crash loses only pending atoms, which nothing
//! durable names; reopening maps the blocks up to the end the last commit reached.
//!
//! Other processes: pending ids are given out only under the atoms lease, an exclusive file lock
//! beside the database (`<db>-atoms.lock`), which the operating system releases when its process
//! dies. A process that waits for it holds a shared lock on `<db>-atoms.want`; the holder sees that
//! at its next interning and flushes, which gives the lease back.
#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::{StoreError, StoreResult};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, ffi};
use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

mod map;
pub(crate) mod stored;
pub use stored::{atoms_dir, copy_blocks, freeze_blocks};
mod vtab;
pub use map::{ProcessIds, StoreIds, map_columns, mapped};

pub type Sym = u32;

/// The values `Atoms::intern_all` looks up under one read lock (decision 0315): a writer waits at most this many
/// lookups for it.
const READ_RUN: usize = 256;

/// The fixed hash every string key uses: the interner's table, and the dependency keys of
/// probes (`Engine::probe`), which must agree for strings that are not (yet) interned.
pub fn hash_str(s: &str) -> u64 {
    foldhash::quality::FixedState::with_seed(0x5EED_71).hash_one(s)
}

/// The atoms a process holds: the store's, read where they lie in its mapped blocks (decision 0401, `Stored`), and
/// the atoms from `base` on, given out here and not yet written (pending), back to back on its heap. Ids below `base`
/// are the store's; atom 0, the empty string, is every store's and no block's.
pub struct Interner {
    stored: stored::Stored,
    base: Sym,
    buf: Vec<u8>,
    /// ends[i] = end offset of pending atom `base + i` in `buf`.
    ends: Vec<u32>,
    hashes: Vec<u64>,
    /// Each pending atom's id with its hash's high half (`tag`): a probe compares tags and reads no atom's hash.
    table: hashbrown::HashTable<(u32, Sym)>,
}

impl Default for Interner {
    fn default() -> Self {
        Interner::over(None)
    }
}

impl Interner {
    /// The atoms of the store at `database` (none read yet), or of an in-memory database's.
    fn over(database: Option<&Path>) -> Self {
        Interner { stored: stored::Stored::new(database), base: 1, buf: Vec::new(), ends: Vec::new(), hashes: Vec::new(), table: hashbrown::HashTable::new() }
    }

    /// The string of `s`, if this process holds it: the store's (mapped), or pending.
    pub fn str(&self, s: Sym) -> Option<&str> {
        if s >= self.base {
            let i = (s - self.base) as usize;
            let &end = self.ends.get(i)?;
            let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
            // SAFETY: only whole `&str`s are appended to `buf`.
            return Some(unsafe { std::str::from_utf8_unchecked(&self.buf[start..end as usize]) });
        }
        if s == 0 {
            return Some("");
        }
        self.stored.str(s)
    }

    /// The hash of `s`'s string (`hash_str`), if this process holds it: kept with each atom, so no string is read.
    fn hash_of(&self, s: Sym) -> Option<u64> {
        if s >= self.base {
            return self.hashes.get((s - self.base) as usize).copied();
        }
        if s == 0 {
            return Some(hash_str(""));
        }
        self.stored.hash(s)
    }

    /// A hash's tag, and the table's hash of a tag: the table places entries by what it holds, so it rehashes
    /// without reading an atom.
    fn tag(h: u64) -> u32 {
        (h >> 32) as u32
    }

    fn placed(tag: u32) -> u64 {
        ((tag as u64) << 32) | tag.rotate_left(16) as u64
    }

    pub fn get(&self, s: &str) -> Option<Sym> {
        self.get_h(s, hash_str(s))
    }

    /// The id of `s` (its hash `h`): pending here, or the store's below `base` (found in its blocks' tables).
    pub fn get_h(&self, s: &str, h: u64) -> Option<Sym> {
        if s.is_empty() {
            return Some(0);
        }
        let tag = Self::tag(h);
        if let Some(&(_, id)) = self.table.find(Self::placed(tag), |&(t, id)| t == tag && self.str(id) == Some(s)) {
            return Some(id);
        }
        self.stored.find(s, h).filter(|&id| id < self.base)
    }

    pub fn intern(&mut self, s: &str) -> Sym {
        self.intern_h(s, hash_str(s))
    }

    /// `intern` of `s`, whose hash is `h`.
    pub fn intern_h(&mut self, s: &str, h: u64) -> Sym {
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
        let tag = Self::tag(h);
        self.table.insert_unique(Self::placed(tag), (tag, id), |&(t, _)| Self::placed(t));
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

    /// Heap bytes held (capacities): the pending atoms' alone. The store's are mapped (`mapped`).
    pub fn bytes(&self) -> usize {
        self.buf.capacity() + self.ends.capacity() * 4 + self.hashes.capacity() * 8 + self.table.capacity() * 9
    }

    /// Bytes of the store's blocks mapped.
    pub fn mapped(&self) -> usize {
        self.stored.mapped()
    }

    /// Appends the pending atoms from `from` to `upto` to the store's open block, in `connection`'s write transaction
    /// (`Stored::append`): read from the pending buffer as they are, not copied.
    fn append_pending(&mut self, connection: &Connection, from: usize, upto: usize) -> StoreResult<Sym> {
        let Interner { stored, base, buf, ends, hashes, .. } = self;
        let base = *base as usize;
        let run = (from..upto).map(|id| {
            let i = id - base;
            let start = if i == 0 { 0 } else { ends[i - 1] as usize };
            // SAFETY: only whole `&str`s are appended to `buf`.
            (unsafe { std::str::from_utf8_unchecked(&buf[start..ends[i] as usize]) }, hashes[i])
        });
        stored.append(connection, from as Sym, run)
    }

    /// The pending atoms below `upto` are stored now (their commit reached it): read where they lie in the blocks,
    /// and their heap copies dropped.
    fn store_upto(&mut self, upto: usize) -> StoreResult<()> {
        let n = upto.saturating_sub(self.base()).min(self.ends.len());
        if n == 0 {
            return Ok(());
        }
        self.stored.view_to(upto as Sym)?;
        let cut = self.ends[n - 1] as usize;
        // The pending rest (if any) is copied into a buffer of its own: a drain would keep the buffer's capacity.
        self.buf = self.buf[cut..].to_vec();
        self.ends.drain(..n);
        for end in &mut self.ends {
            *end -= cut as u32;
        }
        self.hashes.drain(..n);
        self.ends.shrink_to_fit();
        self.hashes.shrink_to_fit();
        self.base += n as Sym;
        // The table holds the pending atoms alone.
        let base = self.base;
        self.table.retain(|&mut (_, id)| id >= base);
        self.table.shrink_to_fit(|&(t, _)| Self::placed(t));
        Ok(())
    }

    /// The database's ids end at `end` (its blocks `rows`), past what this process knew, and none is pending here: new
    /// ids continue from there, and the atoms between are read where they lie.
    fn advance(&mut self, end: usize, rows: &[stored::Row]) -> StoreResult<()> {
        if end <= self.base() {
            return Ok(());
        }
        if !self.ends.is_empty() {
            return Err(StoreError::Corrupt("another process wrote atoms while this one had atoms pending".to_owned()));
        }
        self.stored.learn(rows);
        self.stored.view_to(end as Sym)?;
        self.base = end as Sym;
        Ok(())
    }

    /// Forked (decision 0170): the store's blocks read to `rows`' end, past this process's `base`, which stays (its
    /// ids past it are its own). `store_str` and `store_find` read them.
    fn view_store(&mut self, rows: &[stored::Row]) -> StoreResult<()> {
        self.stored.view_rows(rows)
    }

    /// The store's string of its id `id`, past the fork too (`view_store`).
    fn store_str(&self, id: Sym) -> Option<&str> {
        if id == 0 { Some("") } else { self.stored.str(id) }
    }

    /// The store's id of `s`, past the fork too.
    fn store_find(&self, s: &str, h: u64) -> Option<Sym> {
        if s.is_empty() { Some(0) } else { self.stored.find(s, h) }
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

pub(super) struct State {
    /// The atoms: the store's, mapped, and this process's pending ones (`Interner::base` on).
    atoms: Interner,
    /// When the first pending atom was given out.
    since: Option<Instant>,
    /// After a fork (decision 0170): this process's ids and the store's past it.
    map: Option<map::Map>,
}

/// A process's atoms of one database (decision 0049): the arena, its watermark and the lease.
pub struct Atoms {
    /// The database file; `None` for an in-memory database, whose atoms are never written.
    path: Option<PathBuf>,
    /// The store's identity (`store_identity`) when these are a store's own atoms: a store created again at the
    /// path has another, and atoms of its own (`for_store`).
    identity: Option<i64>,
    state: rerail_accel::sharded::Sharded<State>,
    /// The store's guest keys as this process holds them (decision 0391), shared as the atoms are.
    guest_keys: Arc<super::guest_keys::GuestKeys>,
    /// Held (the lock file, exclusively locked) while this process may give out pending ids.
    /// Lock order: `lease`, then `reader`, then `state`.
    lease: Mutex<Option<File>>,
    leased: AtomicBool,
    /// When this holder last looked for a waiter, and what it saw.
    wanted: Mutex<(Option<Instant>, bool)>,
    /// A connection of this process's own, for reading where the database's atoms end (`atom_blocks`).
    reader: Mutex<Option<Connection>>,
    /// Per connection (by handle): whether its open transaction appended the pending atoms.
    flushed: Mutex<HashMap<usize, Arc<AtomicBool>>>,
    /// Every id, by string, for range scans of `atoms.value` (`vtab`); extended when read.
    sorted: Mutex<Vec<Sym>>,
    /// Appends this process made, their atoms and their bytes (the profile's `atoms` line).
    written: Mutex<(u64, u64, u64)>,
    /// Runs under way in this process (`ready_for_run` to `run_over`): while one is, and its view of the store is
    /// complete (`complete`), ids are given out without the lease (provisional, decision 0160).
    runs: std::sync::atomic::AtomicUsize,
    /// Where this process's last append ended (`write_pending`): the database's end there is its own, not another
    /// process's.
    wrote: std::sync::atomic::AtomicUsize,
    /// `RERAIL_TEST_FORK_AFTER`'s thresholds (`test_fork`), read once when the table is made; none unless it is set.
    fork_after: Vec<usize>,
    /// Another process stored atoms while this one's were provisional (decision 0160): this process's ids are not the
    /// store's past `base`, and are mapped to them at the store's boundary (decision 0170, `map`).
    forked: AtomicBool,
    /// This process reads the store's atoms to its end, and the lease has been held since that was seen: a string
    /// not found is not interned, and nothing is read to know it (decision 0104).
    complete: AtomicBool,
    /// A flush is queued to the store's writer and has not run yet (`queue_atoms_flush`).
    pub(super) flush_queued: AtomicBool,
}

/// The identity read of each database file, by its path and (device, inode, creation time): `Atoms::identity`. A file
/// made again at a path may have the inode the deleted one had; it was not made at the same time.
static IDENTITIES: LazyLock<Mutex<HashMap<String, ((u64, u64, u128), Option<i64>)>>> = LazyLock::new(Default::default);

/// The database at `path` was made a store (`schema::initialize`): the identity read of it before (none) is gone.
pub(crate) fn forget_identity(path: &str) {
    IDENTITIES.lock().remove(path);
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
///
/// Reservations nest: a component this thread runs while its own waits (a job the wait runs, such as another
/// component's, while a join's morsels finish) reserves its atoms as its own, and the waiting one's reservation
/// goes on when it ends.
pub fn reserving(arena: &Arc<Atoms>) -> Reservation {
    Reservation(RESERVING.with_borrow_mut(|r| r.replace((Arc::as_ptr(arena) as usize, Vec::new()))))
}

/// A thread's reservation in progress (`reserving`): `end` gives what it made. It holds the reservation it was
/// made inside, which goes on when it ends.
pub struct Reservation(Option<(usize, Vec<(usize, Vec<Sym>)>)>);

impl Reservation {
    pub fn end(mut self) -> Reserved {
        let outer = self.0.take();
        let made = RESERVING.with_borrow_mut(|r| std::mem::replace(r, outer)).map(|r| r.1).unwrap_or_default();
        std::mem::forget(self);
        Reserved(made)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let outer = self.0.take();
        drop(Reserved(RESERVING.with_borrow_mut(|r| std::mem::replace(r, outer)).map(|r| r.1).unwrap_or_default()));
    }
}

/// The atoms a component reserved: published (counted as pending) when dropped, as the component retires.
#[derive(Default)]
pub struct Reserved(Vec<(usize, Vec<Sym>)>);

impl Reserved {
    /// Takes in what a later reservation of the same component made (its members resumed after their wave).
    pub fn extend(&mut self, mut more: Reserved) {
        self.0.append(&mut more.0);
    }

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

/// Decision 0160's trial (`RERAIL_ATOMS_PROVISIONAL=1`): ids are provisional and the lease is never taken. Off, the
/// lease is taken as before (decision 0049).
fn lease_free() -> bool {
    static FREE: LazyLock<bool> = LazyLock::new(|| std::env::var("RERAIL_ATOMS_PROVISIONAL").is_ok_and(|v| v == "1"));
    *FREE
}

/// The error of a write refused after a fork (decisions 0160, 0170): its rows may name this process's ids, not the
/// store's. A write behind the run that meets it is dropped, as a cache's miss.
pub const DIVERGED_TEXT: &str = "this process's atoms forked from the store's and this write's are not mapped (decision 0170)";

static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Weak<Atoms>>>> = LazyLock::new(Default::default);
/// The atoms of in-memory databases: one set for the process, as the planner's symbols had.
static MEMORY: LazyLock<Arc<Atoms>> = LazyLock::new(|| Arc::new(Atoms::new(None, None)));

/// A path's registry key: its canonical form, resolved once per path (a `realpath` per stratum was 2.5% of a corpus
/// run's main thread). A path that does not resolve yet is not remembered.
pub(super) fn key(path: &Path) -> PathBuf {
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

use stored::database_end;

impl Atoms {
    fn new(path: Option<PathBuf>, identity: Option<i64>) -> Self {
        let atoms = Interner::over(path.as_deref());
        Atoms {
            path,
            identity,
            // Atom 0, the empty string, is every store's.
            state: rerail_accel::sharded::Sharded::new(State { atoms, since: None, map: None }),
            guest_keys: Arc::default(),
            lease: Mutex::new(None),
            leased: AtomicBool::new(false),
            wanted: Mutex::new((None, false)),
            reader: Mutex::new(None),
            flushed: Mutex::new(HashMap::new()),
            sorted: Mutex::new(Vec::new()),
            written: Mutex::new((0, 0, 0)),
            runs: std::sync::atomic::AtomicUsize::new(0),
            wrote: std::sync::atomic::AtomicUsize::new(0),
            fork_after: std::env::var("RERAIL_TEST_FORK_AFTER").map(|v| v.split(',').filter_map(|k| k.trim().parse().ok()).collect()).unwrap_or_default(),
            forked: AtomicBool::new(false),
            complete: AtomicBool::new(false),
            flush_queued: AtomicBool::new(false),
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
    /// The store's guest keys as this process holds them (decision 0391).
    pub(crate) fn guest_keys(&self) -> Arc<super::guest_keys::GuestKeys> {
        Arc::clone(&self.guest_keys)
    }

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

    /// The identity of `connection`'s main database if it is a store (`store_identity`): read once a file (decision
    /// 0104: a connection a run opens in its waves reads nothing), and again only for another file at the path (a
    /// store deleted and made again, which has another identity).
    fn identity(connection: &Connection) -> rusqlite::Result<Option<i64>> {
        #[cfg(unix)]
        let file = connection.path().filter(|p| !p.is_empty()).and_then(|p| {
            use std::os::unix::fs::MetadataExt as _;
            let m = std::fs::metadata(p).ok()?;
            let made = m.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            Some((p.to_owned(), (m.dev(), m.ino(), made)))
        });
        #[cfg(not(unix))]
        let file: Option<(String, (u64, u64, u128))> = None;
        if let Some((path, at)) = &file
            && let Some((held, id)) = IDENTITIES.lock().get(path)
            && held == at
        {
            return Ok(*id);
        }
        let id = Self::identity_read(connection)?;
        if let Some((path, at)) = file {
            IDENTITIES.lock().insert(path, (at, id));
        }
        Ok(id)
    }

    fn identity_read(connection: &Connection) -> rusqlite::Result<Option<i64>> {
        let store: bool = connection.query_row_cached(
            "SELECT EXISTS (SELECT 1 FROM main.sqlite_schema WHERE type = 'table' AND name = 'store_identity')",
            [],
            |row| row.get(0),
        )?;
        if !store {
            return Ok(None);
        }
        connection.query_row_cached("SELECT id FROM main.store_identity WHERE singleton = 1", [], |row| row.get(0)).optional()
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

    /// Shared scratch databases borrow the arena; only its owner stores atoms.
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
                // This hook replaces the one every connection is given for the warm check (`RERAIL_WARM`).
                // SAFETY: called by SQLite on this connection's thread, inside its commit hook.
                unsafe { super::hazards::warm_commit(handle as *mut ffi::sqlite3) };
                let flushed = hook.swap(false, Ordering::AcqRel);
                if flushed || !held.pending() {
                    return false;
                }
                // SAFETY: called by SQLite on this connection's thread, under its mutex.
                let state = unsafe { ffi::sqlite3_txn_state(handle as *mut ffi::sqlite3, c"main".as_ptr()) };
                if state == ffi::SQLITE_TXN_WRITE {
                    if held.forked() {
                        return true;
                    }
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

    /// Whether ids are given out provisionally (decision 0160): a run is under way and every stored atom it may meet is
    /// held, so a string not in memory is new to it, and its id is checked against the store's when it is written.
    /// Every stored atom is held (`complete`), or this process maps its ids to the store's (decision 0170): forked, or
    /// over a store it does not hold whole, holding the atoms its stored rows name as they are read (`hold_named`).
    fn provisional(&self) -> bool {
        lease_free() && self.runs.load(Ordering::Acquire) > 0 && (self.complete.load(Ordering::Acquire) || self.forked())
    }

    /// Forks on this process's reader (outside a write): another process stored atoms past this one's pending ones.
    fn fork_reading(&self) -> StoreResult<()> {
        self.with_reader(|connection| {
            let mut state = self.state.write();
            self.fork_now(connection, &mut state)
        })?;
        Ok(())
    }

    /// Appends this process made so far: (appends, atoms, bytes), and the atoms pending now.
    pub fn writes(&self) -> ((u64, u64, u64), usize) {
        let pending = self.state.read().atoms.pending().0;
        (*self.written.lock(), pending)
    }

    /// Heap bytes the atoms held take.
    pub fn bytes(&self) -> usize {
        self.state.read().atoms.bytes()
    }

    /// Whether atoms are pending (given out, not yet in the database).
    /// Pending atoms that a flush must store: none once forked (decision 0170). A forked process's write maps the
    /// atoms its rows name as it writes them (`to_store`), and an atom no row names needs no store id. A flush queued
    /// for them found the store's string by its key and wrote nothing, or stored a string nothing names.
    pub fn flush_wanted(&self) -> bool {
        !self.forked() && self.pending()
    }

    pub fn pending(&self) -> bool {
        if self.path.is_none() {
            return false;
        }
        let state = self.state.read();
        if let Some(n) = self.pending_forked(&state) {
            return n > 0;
        }
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

    /// The atom `intern` would give `value` without making one: in memory, or found where interning finds a stored
    /// atom (outside a run's provisional ids, decision 0160, and before a fork, 0170). Inside a run, or forked,
    /// interning reads nothing and a string not held gets a new id, which no row holds: so `None` there, and no read.
    pub fn existing(&self, value: &str) -> Option<Sym> {
        if let Some(atom) = self.state.read().atoms.get(value) {
            return Some(atom);
        }
        // Where `lookup` would read nothing and find nothing (`misses_are_final`), it is not asked: its locks are a
        // join's cost per text it misses.
        if self.misses_are_final() {
            return None;
        }
        self.lookup(value).ok().flatten()
    }

    /// Each of `ids` (atoms) replaced by its content key: its string's hash (`hash_str`), the same in every process
    /// whichever id it gave the string (decision 0212). A digest of rows that hold atoms reads them so: by id, a
    /// process that numbered a string otherwise (a fork, decision 0170) digested the same rows otherwise, and every
    /// frame it kept was stale to the next. Under one read of the atoms, each hash read where it lies. A value that is
    /// no atom here (an in-memory store's test rows) keys as itself.
    pub fn content_keys(&self, ids: &mut [i64]) {
        let state = self.state.read();
        for v in ids.iter_mut() {
            if let Some(h) = u32::try_from(*v).ok().and_then(|s| state.atoms.hash_of(s)) {
                *v = h as i64;
            }
        }
    }

    /// Whether a text not held in memory has no atom `existing` would find (it reads nothing then): inside a run's
    /// provisional ids, forked, or every stored atom held while this process holds the lease. Atomic loads only, no
    /// lock: a walk in morsels asks it for each text it misses.
    pub fn misses_are_final(&self) -> bool {
        self.provisional() || self.forked() || (self.complete.load(Ordering::Acquire) && self.leased.load(Ordering::Acquire))
    }

    /// `f` over the atoms held in memory, under one read of the arena: a join step that reads several texts of a
    /// row (a split's whole and each part) reads them under it, not one read each.
    pub fn in_memory<R>(&self, f: impl FnOnce(&Interner) -> R) -> R {
        f(&self.state.read().atoms)
    }

    /// `a . "." . b` as a join's `@cat` makes it, under one read of the arena (not one per text and one for the
    /// lookup: on a walk in morsels, each read is one more write of the lock's word that every thread shares).
    /// `Some(Ok(Some(atom)))`: the text's atom; `Some(Ok(None))`: none held in memory, the text left in `buf`;
    /// `Some(Err(()))`: `a`'s text is empty, so the text is `b`'s; `None`: a text not in memory (read as before).
    pub fn cat_held(&self, a: Sym, b: Sym, buf: &mut String) -> Option<Result<Option<Sym>, ()>> {
        let state = self.state.read();
        let x = state.atoms.str(a)?;
        if x.is_empty() {
            return Some(Err(()));
        }
        let y = state.atoms.str(b)?;
        buf.clear();
        buf.push_str(x);
        buf.push('.');
        buf.push_str(y);
        Some(Ok(state.atoms.get(buf)))
    }

    /// `f` of atom `atom`'s string, if the database or this process holds it, read where it lies.
    pub fn with_str<R>(&self, atom: Sym, f: impl FnOnce(&str) -> R) -> Option<R> {
        self.state.read().atoms.str(atom).map(f)
    }

    /// The atom of `value`, if the database or this process holds it.
    pub fn lookup(&self, value: &str) -> StoreResult<Option<Sym>> {
        if let Some(atom) = self.state.read().atoms.get(value) {
            return Ok(Some(atom));
        }
        // Every stored atom is held: a string not in memory is not stored, as far as this run is concerned.
        if self.provisional() {
            return Ok(None);
        }
        // Forked: another process's atom of it is found by its key and held here (decision 0170).
        if self.forked() {
            self.find_forked(&[value])?;
            return Ok(self.state.read().atoms.get(value));
        }
        // Without the lease another process may have stored it since: the store's end is read, and its blocks mapped
        // to it.
        if !self.leased.load(Ordering::Acquire) {
            self.catch_up()?;
        }
        Ok(self.state.read().atoms.get(value))
    }

    /// The atom of each of `values` (`lookup`), those not in memory found in one read.
    pub fn lookup_all(&self, values: &[&str]) -> StoreResult<Vec<Option<Sym>>> {
        let missing: Vec<&str> = {
            let state = self.state.read();
            values.iter().copied().filter(|v| state.atoms.get(v).is_none()).collect()
        };
        if !missing.is_empty() && !self.provisional() {
            if self.forked() {
                self.find_forked(&missing)?;
            } else if !self.leased.load(Ordering::Acquire) {
                self.catch_up()?;
            }
        }
        let state = self.state.read();
        Ok(values.iter().map(|v| state.atoms.get(v)).collect())
    }

    /// The string of `atom`, if the database or this process holds it.
    pub fn value(&self, atom: Sym) -> StoreResult<Option<String>> {
        if (atom as usize) >= self.len() {
            self.catch_up()?;
        }
        Ok(self.state.read().atoms.str(atom).map(str::to_owned))
    }

    /// Maps the store's atoms to its end (an arena made after the store was written reads none of them yet).
    pub fn read_stored(&self) -> StoreResult<()> {
        self.catch_up()
    }

    /// Closes this process's connection for reading where the store's atoms end (reopened when next needed): the
    /// store's last handle closed, and the arena, which the planner's symbols keep, must not hold its file open.
    pub(crate) fn release_reader(&self) {
        self.reader.lock().take();
        self.state.write().atoms.stored.release_writer();
    }

    /// The atom of each of `values`, giving out new ones (pending) under the lease. Decision 0315: each value is
    /// hashed once, outside any lock; those held are found under the read lock, and only the rest are looked for
    /// again and given ids under the write lock (`give`), so a batch of mostly known values stops the arena's readers
    /// for its new ones only.
    pub fn intern_all(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
        let hashes: Vec<u64> = values.iter().map(|value| hash_str(value)).collect();
        // Looked up `READ_RUN` at a time, each run under a read lock of its own: a writer waits for every reader
        // (`Sharded::write`), and while it waits every reader of a shard it holds waits too; a parser's batch read
        // under one lock was the arena's longest hold (decision 0315).
        let mut found: Vec<Option<Sym>> = Vec::with_capacity(values.len());
        for (values, hashes) in values.chunks(READ_RUN).zip(hashes.chunks(READ_RUN)) {
            let state = self.state.read();
            found.extend(values.iter().zip(hashes).map(|(value, &h)| state.atoms.get_h(value, h)));
        }
        if found.iter().all(Option::is_some) {
            return Ok(found.into_iter().flatten().collect());
        }
        // Inside a run, with every stored atom held: new ids are provisional, given out without the lease and checked
        // against the store's as they are written (decision 0160). Nothing is read. Forked, the store's are found as
        // they are written (decision 0170).
        if self.provisional() || self.forked() {
            return Ok(self.give(values, &hashes, found));
        }
        // Outside the waves: what this process does not find may have been stored since, found once the store's
        // blocks are read to its end (`give` looks again), and the rest is provisional too. No lease (decision 0160).
        if lease_free() {
            self.catch_up_free()?;
            return Ok(self.give(values, &hashes, found));
        }
        // The lease is held while ids are given out, so a flush on another thread cannot give it
        // back between the check and the new ids. Taking it reads the store's end.
        let mut lease = self.lease.lock();
        self.acquire(&mut lease)?;
        let ids = self.give(values, &hashes, found);
        drop(lease);
        Ok(ids)
    }

    /// The ids of `values` (hashed: `hashes`), those not `found` looked for again and new ones pending from this
    /// process's end, under the write lock: the only work under it is theirs.
    fn give(&self, values: &[&str], hashes: &[u64], found: Vec<Option<Sym>>) -> Vec<Sym> {
        let missing: Vec<usize> = found.iter().enumerate().filter(|(_, f)| f.is_none()).map(|(i, _)| i).collect();
        let mut given = Vec::with_capacity(missing.len());
        let mut state = self.state.write();
        let before = state.atoms.len() as Sym;
        for &i in &missing {
            given.push(state.atoms.intern_h(values[i], hashes[i]));
        }
        if state.since.is_none() && state.atoms.pending().0 > 0 {
            state.since = Some(Instant::now());
        }
        let new = state.atoms.len() as Sym - before;
        drop(state);
        let mut ids = found;
        for (&i, id) in missing.iter().zip(given) {
            ids[i] = Some(id);
        }
        let ids: Vec<Sym> = ids.into_iter().map(|id| id.expect("every value given an id")).collect();
        if new > 0 {
            self.test_fork(new as usize);
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
        ids
    }

    /// A test of decision 0170 (`RERAIL_TEST_FORK_AFTER=k1,k2,…`): once this process has given out `k` new ids, a
    /// connection of its own, as another process would, appends 20 strings no rule uses at the store's end (in a write
    /// transaction, through the blocks, `stored::append_as_another`), as a concurrent process giving out provisional ids
    /// does (0160). This process forks at its next write if ids are pending then, at a point the thresholds choose.
    /// Each threshold fires once. Off unless the variable is set. With `RERAIL_TEST_FORK_FROM=<store>`, that store's
    /// atoms instead: real strings, which this process may give ids of its own later (those the store holds then are
    /// left out: found, never stored twice).
    fn test_fork(&self, new: usize) {
        static GIVEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if self.fork_after.is_empty() {
            return;
        }
        let Some(path) = self.path.clone() else { return };
        let was = GIVEN.fetch_add(new, Ordering::AcqRel);
        for (i, &k) in self.fork_after.iter().enumerate() {
            if was < k && was + new >= k {
                let path = path.clone();
                let append = move || {
                    let appended = (|| -> StoreResult<(usize, Sym)> {
                        let strings: Vec<String> = match std::env::var("RERAIL_TEST_FORK_FROM") {
                            Ok(donor) => stored::every_stored(&Connection::open(donor)?)?,
                            Err(_) => (0..20).map(|j| format!("rerail-test-fork-{}-{i}-{j}", std::process::id())).collect(),
                        };
                        let c = Connection::open(&path)?;
                        c.busy_timeout(LEASE_WAIT)?;
                        stored::append_as_another(&c, &strings)
                    })();
                    match appended {
                        Ok((n, end)) => eprintln!("rerail: test fork after {k} ids: {n} strings appended, the store ends at {end}"),
                        Err(e) => eprintln!("rerail: test fork after {k} ids: {e}"),
                    }
                };
                // Waited for, so the ids just given are still pending when the store moves; on a thread of its own, as
                // another process's write (and never the writer's batch thread, whose transaction it waits for).
                let _ = rerail_accel::exec::service("rerail-test-fork", None, move || {
                    super::hazards::behind_the_run();
                    append()
                })
                .map(|h| h.join());
            }
        }
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
        self.read_end()?;
        // Its blocks read to the store's end (`read_end`), and no other process gives out ids while the lease is
        // held: a string not found is not stored, and lookups read nothing (decision 0104).
        self.complete.store(true, Ordering::Release);
        Ok(())
    }

    /// Without the lease (decision 0160): learns where the database's ids end, outside the waves.
    fn catch_up_free(&self) -> StoreResult<()> {
        if self.provisional() {
            return Ok(());
        }
        self.read_end()
    }

    /// Learns where the database's ids end, unless this process holds the lease (then it knows).
    fn catch_up(&self) -> StoreResult<()> {
        if self.leased.load(Ordering::Acquire) || self.provisional() {
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

    /// The store's blocks as its rows name them now, read on this process's own connection; `None` when the database
    /// is not on disk (yet).
    fn read_rows(&self) -> StoreResult<Option<Vec<stored::Row>>> {
        let path = self.path.clone();
        self.with_reader(|connection| {
            stored::rows(connection).map_err(|error| StoreError::Corrupt(format!("{}: reading atom blocks: {error}", path.as_deref().unwrap_or(Path::new("")).display())))
        })
    }

    /// This process's end moved to the store's (`rows`), its blocks read to it. A block the rows named may be gone
    /// by the time it is mapped (a roll-up committed since, and the next write removed what it replaced): the rows are
    /// read again, and the blocks they name now mapped.
    fn advance_reading(&self, mut rows: Vec<stored::Row>) -> StoreResult<()> {
        for again in (0..3).rev() {
            match self.state.write().atoms.advance(stored::end_of(&rows) as usize, &rows) {
                Err(_) if again > 0 => {}
                done => return done,
            }
            let Some(now) = self.read_rows()? else { return Ok(()) };
            rows = now;
        }
        Ok(())
    }

    /// Where the database's ids end: new ids continue from there, its blocks read to it where they lie.
    fn read_end(&self) -> StoreResult<()> {
        let Some(rows) = self.read_rows()? else { return Ok(()) };
        let end = stored::end_of(&rows) as usize;
        if self.forked() {
            return Ok(());
        }
        // Provisional ids pending here (decision 0160) and another process's stored past them: forked (0170).
        let (base, pending) = { let s = self.state.read(); (s.atoms.base(), s.atoms.pending().0) };
        if end > base && pending > 0 && end != self.wrote.load(Ordering::Acquire) {
            return self.fork_reading();
        }
        self.advance_reading(rows)
    }

    /// Maps every stored atom: for a scan or a range of the `atoms` table. The store's end is read (without the
    /// lease), and its blocks read to it.
    pub(super) fn read_all(&self) -> StoreResult<()> {
        if self.path.is_none() {
            return Ok(());
        }
        self.catch_up()
    }

    /// In `connection`'s open transaction, if it writes the main database: appends the pending
    /// atoms to the store's blocks and marks the transaction as having done so (the commit hook). Returns
    /// the watermark to pass to `committed` after the commit.
    pub fn write_pending(&self, connection: &Connection) -> StoreResult<usize> {
        if self.path.is_none() {
            return Ok(0);
        }
        if !self.owns(connection) {
            return Err(StoreError::Invalid("atoms must be stored on the arena's owning database".to_owned()));
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
        if self.forked() {
            return self.write_forked(connection);
        }
        // Where the database's atoms end now (another connection of this process may have committed a flush whose
        // `committed` has not run yet).
        let end = database_end(connection)?;
        let (end, upto) = {
            let state = self.state.read();
            let (len, base) = (state.atoms.len(), state.atoms.base());
            if end < base {
                return Err(StoreError::Corrupt(format!("the database's atoms end at {end}, before the {} this process wrote", base)));
            }
            // Past this process's end: its own append, committed on another connection (`wrote`), or another
            // process's. Another's with none pending here moves this process's end; with some pending, the ids given
            // out here are another's strings in the store (decision 0160): this process forks (0170). This
            // transaction's rows may name the ids given out here, so it is refused, as a cache's miss; a writer's batch
            // forks before its rows are made (`begin_write`).
            if end > base && end != self.wrote.load(Ordering::Acquire) {
                if len > base {
                    drop(state);
                    let mut state = self.state.write();
                    self.fork_now(connection, &mut state)?;
                    return Err(StoreError::Invalid(DIVERGED_TEXT.to_owned()));
                }
                drop(state);
                self.advance_holding(connection, base, end)?;
                return Ok(end);
            }
            (end, len)
        };
        // Appended to the open block (decision 0401 A3), on the disk before this transaction commits; the atoms are the
        // store's once it has.
        if upto > end {
            let bytes = state_bytes(&self.state.read().atoms, end, upto);
            self.state.write().atoms.append_pending(connection, end, upto)?;
            // Counted when written; a transaction that then rolls back still counts.
            let mut written = self.written.lock();
            *written = (written.0 + 1, written.1 + (upto - end) as u64, written.2 + bytes as u64);
            // Where this process's own appends end: one committed on another connection is not another process's.
            self.wrote.store(upto, Ordering::Release);
        }
        Ok(upto.max(end))
    }

    /// Another process stored atoms `base..end` and none is pending here: this process's end moves there, its blocks
    /// read to it where they lie (on the writer's connection, in its transaction), so a run's lookups still read nothing
    /// more (decision 0160).
    fn advance_holding(&self, connection: &Connection, base: usize, end: usize) -> StoreResult<()> {
        let mut state = self.state.write();
        self.advance_holding_in(connection, &mut state, base, end)
    }

    /// `advance_holding`, under `state`'s lock held.
    fn advance_holding_in(&self, connection: &Connection, state: &mut State, base: usize, end: usize) -> StoreResult<()> {
        if state.map.is_some() || state.atoms.pending().0 > 0 || state.atoms.base() != base {
            return Ok(());
        }
        state.atoms.advance(end, &stored::rows(connection)?)
    }

    /// After a commit that held `write_pending`'s append: atoms below `upto` are stored, read where they lie. The
    /// lease is given back once none is pending.
    pub fn committed(&self, upto: usize) {
        if self.path.is_none() {
            return;
        }
        let mut lease = self.lease.lock();
        let mut state = self.state.write();
        if state.map.is_some() {
            self.committed_forked(&mut state);
            return;
        }
        if let Err(error) = state.atoms.store_upto(upto) {
            // The commit holds them; a block that cannot be mapped again is a broken store.
            panic!("rerail: atoms: the atoms just committed cannot be read: {error}");
        }
        if state.atoms.pending().0 == 0 {
            state.since = None;
            // A run keeps the lease to its end (`ready_for_run`), unless another process waits for it.
            if (self.runs.load(Ordering::Acquire) == 0 || self.wanted()) && lease.take().is_some() {
                self.leased.store(false, Ordering::Release);
                self.complete.store(false, Ordering::Release);
            }
        }
    }

    /// Before a run's first wave (decision 0104): takes the lease, kept to the run's end (`run_over`), and reads where
    /// the database's ids end, once, its blocks mapped to it: the run's lookups and ids then read nothing from the
    /// store (decision 0401 A4: every stored atom is read where it lies, from the open). Returns whether they do not.
    pub fn ready_for_run(&self) -> StoreResult<bool> {
        if self.path.is_none() {
            return Ok(true);
        }
        // The outermost run reads where the database's ids end, once, before its first wave; no lease is taken
        // (decision 0160: the run's new ids are provisional). A nested run is inside it and reads nothing.
        if !lease_free() {
            let mut lease = self.lease.lock();
            self.acquire(&mut lease)?;
            self.runs.fetch_add(1, Ordering::AcqRel);
        } else if self.runs.fetch_add(1, Ordering::AcqRel) == 0 && !self.leased.load(Ordering::Acquire) && !super::hazards::inside_waves() {
            let _lease = self.lease.lock();
            if let Some(rows) = self.read_rows()? {
                let end = stored::end_of(&rows) as usize;
                let (base, pending) = { let s = self.state.read(); (s.atoms.base(), s.atoms.pending().0) };
                if end > base && end != self.wrote.load(Ordering::Acquire) && pending > 0 && !self.forked() {
                    self.fork_reading()?;
                } else if pending == 0 && !self.forked() {
                    self.advance_reading(rows)?;
                }
            }
        }
        // Inside another run's waves (decision 0160's trial) nothing is read: what is mapped stays as it was.
        if lease_free() && super::hazards::inside_waves() {
            return Ok(self.complete.load(Ordering::Acquire));
        }
        self.complete.store(true, Ordering::Release);
        Ok(true)
    }

    /// Whether the atoms a stored row names are checked as the row is read (`hold_named`): this process maps its ids
    /// to the store's, or gives ids out without the lease. Without it, a row another process stored past this one's end
    /// names ids this one may have given to other strings: reading the row finds the fork (decision 0170) before its
    /// symbols are decoded, in a wave.
    pub fn holds_by_reading(&self) -> bool {
        self.path.is_some() && (self.forked() || !self.leased.load(Ordering::Acquire))
    }

    /// The stored atoms `ids` (as the store numbers them: a stored row's symbols), as the rows are read before a run's
    /// waves: those past this process's end are another process's, stored since; the store's end is read first (its
    /// blocks mapped to it), which forks where ids are pending here. Forked, the store's atoms past the fork are held
    /// under this process's ids. So the waves find each such atom, and a string not there is not one of them.
    pub fn hold_named(&self, ids: &[i64]) -> StoreResult<()> {
        if self.path.is_none() || ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<Sym> = ids.iter().filter_map(|&v| Sym::try_from(v).ok()).collect();
        if self.forked() {
            let missing: Vec<Sym> = {
                let state = self.state.read();
                let map = state.map.as_ref().expect("forked");
                ids.iter().copied().filter(|&v| v >= map.fork && map.local_of(v).is_none()).collect()
            };
            return if missing.is_empty() { Ok(()) } else { self.hold_forked(&missing) };
        }
        let base = self.state.read().atoms.base();
        if ids.iter().any(|&v| v as usize >= base) {
            self.read_end()?;
            if self.forked() {
                return self.hold_named(&ids.iter().map(|&v| i64::from(v)).collect::<Vec<_>>());
            }
        }
        Ok(())
    }

    /// A run's end: the lease is given back with its next commit, as before the run; at once when nothing is pending.
    /// The run's last commit can come before its end (Q42): `committed` kept the lease then, the run still going, and
    /// no commit came after to give it back, so the process held the store's lock file after its last run.
    pub fn run_over(&self) {
        let _ = self.runs.try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
        if self.path.is_none() || self.runs.load(Ordering::Acquire) != 0 {
            return;
        }
        let mut lease = self.lease.lock();
        let state = self.state.read();
        if state.map.is_none() && state.atoms.pending().0 == 0 && lease.take().is_some() {
            self.leased.store(false, Ordering::Release);
            self.complete.store(false, Ordering::Release);
        }
    }

    /// Whether a flush is due: enough pending (count, bytes or age), or another process waiting. Never inside a run
    /// (`ready_for_run`): the store's writer flushes then (`queue_atoms_flush`), and no wave writes (decision 0104).
    pub fn due(&self) -> bool {
        self.runs.load(Ordering::Acquire) == 0 && self.due_now()
    }

    pub(super) fn due_now(&self) -> bool {
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
            // a flush becomes due there, but scratch deliberately has no atom_blocks.
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
        // A flush writes atoms alone: forked or forking, they are mapped (decision 0170).
        let written = self.begin_write(connection).and_then(|()| mapped(|| self.write_pending(connection))).and_then(|upto| {
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

/// The bytes of pending atoms `from..upto`.
fn state_bytes(atoms: &Interner, from: usize, upto: usize) -> usize {
    let (i, j) = (from - atoms.base(), upto - atoms.base());
    let start = if i == 0 { 0 } else { atoms.ends[i - 1] as usize };
    (if j == 0 { 0 } else { atoms.ends[j - 1] as usize }) - start
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

/// `temp.atoms(atom, value)` on `connection`: the atoms its database's blocks hold, read on its rows, and
/// nothing pending in any process. For checks of what a store holds on disk (after a crash, in a
/// capture), on a connection without the `atoms` virtual table; the temp table hides it otherwise.
pub fn materialize_stored(connection: &Connection) -> StoreResult<()> {
    let atoms = stored::every_stored(connection)?;
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
        if !self.flush_wanted() || !path.exists() {
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
        // A store with a writer has it append them (decision 0104: only its writers write); one without, now.
        if self.write_back.is_some() {
            // Due, and with atoms a flush would write (not only those a component reserves, `Atoms::pending`).
            if self.atoms.due_now() && self.atoms.flush_wanted() && !self.atoms.flush_queued.swap(true, Ordering::AcqRel) {
                self.queue_atoms_flush();
            }
        } else if self.atoms.due() && self.atoms.flush_wanted() {
            self.flush_atoms()?;
        }
        Ok(ids)
    }

    /// The pending atoms, appended by the store's writer behind the run (decision 0104).
    pub fn queue_atoms_flush(&self) {
        self.write_back_syms(crate::content_store::write_back::Syms::None, "the pending atoms", 0, Vec::new(), crate::content_store::write_back::Work::atoms(true));
    }

    /// The pending atoms, appended by the store's writer behind the run's other writes, as `what`.
    pub fn queue_atoms(&self, what: &'static str) {
        self.write_back_syms(crate::content_store::write_back::Syms::None, what, 0, Vec::new(), crate::content_store::write_back::Work::atoms(false));
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
        let atoms = std::sync::Arc::clone(&self.atoms);
        match mapped(|| self.immediate_transaction_no_wait(|t| atoms.begin_write(t))) {
            Ok(()) | Err(StoreError::Busy) => Ok(()),
            Err(StoreError::Database(error))
                if matches!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// `values` (this process's symbols) as the store numbers them, in this handle's open write transaction (the
    /// writer's batch): unchanged unless forked (decision 0170).
    pub fn symbols_to_store<'v>(&self, ids: ProcessIds<'v>) -> StoreResult<StoreIds<'v>> {
        if !self.atoms.forked() {
            return Ok(StoreIds(ids.0));
        }
        let connection = self.connection.lock().map_err(StoreError::Invalid)?;
        self.atoms.to_store(&connection, ids)
    }

    /// The atom of `value`, if the store holds it.
    pub fn atom(&self, value: &str) -> StoreResult<Option<Sym>> {
        self.atoms.lookup(value)
    }

    /// The atoms' writes so far: (appends, atoms, bytes), and the atoms pending now.
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
