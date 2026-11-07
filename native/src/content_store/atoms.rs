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

pub struct Interner {
    buf: Vec<u8>,
    /// ends[i] = end offset of string i in `buf` (start = ends[i - 1]).
    ends: Vec<u32>,
    hashes: Vec<u64>,
    table: hashbrown::HashTable<Sym>,
}

impl Default for Interner {
    fn default() -> Self {
        let mut i = Interner { buf: Vec::new(), ends: Vec::new(), hashes: Vec::new(), table: hashbrown::HashTable::new() };
        // Sym 0 is the empty string.
        i.intern("");
        i
    }
}

impl Interner {
    pub fn str(&self, s: Sym) -> &str {
        let s = s as usize;
        // SAFETY: only whole `&str`s are appended to `buf`.
        unsafe { std::str::from_utf8_unchecked(&self.buf[self.offset(s)..self.ends[s] as usize]) }
    }

    pub fn get(&self, s: &str) -> Option<Sym> {
        self.get_h(s, hash_str(s))
    }

    pub fn get_h(&self, s: &str, h: u64) -> Option<Sym> {
        self.table.find(h, |&id| self.hashes[id as usize] == h && self.str(id) == s).copied()
    }

    pub fn intern(&mut self, s: &str) -> Sym {
        let h = hash_str(s);
        if let Some(id) = self.get_h(s, h) {
            return id;
        }
        self.push(s, h)
    }

    fn push(&mut self, s: &str, h: u64) -> Sym {
        let id = self.ends.len() as Sym;
        self.buf.extend_from_slice(s.as_bytes());
        assert!(self.buf.len() <= u32::MAX as usize, "interner arena over 4 GiB");
        self.ends.push(self.buf.len() as u32);
        self.hashes.push(h);
        let hashes = &self.hashes;
        self.table.insert_unique(h, id, |&x| hashes[x as usize]);
        id
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// The arena offset string `i` starts at (the arena's length for `i == len`).
    fn offset(&self, i: usize) -> usize {
        if i == 0 { 0 } else { self.ends[i - 1] as usize }
    }

    /// Heap bytes held (capacities).
    pub fn bytes(&self) -> usize {
        self.buf.capacity() + self.ends.capacity() * 4 + self.hashes.capacity() * 8 + self.table.capacity() * 5
    }

    /// Strings `from..` as one segment: their end offsets (u32, little-endian, relative to the
    /// segment's first byte) and their bytes.
    fn segment(&self, from: usize) -> (Vec<u8>, Vec<u8>) {
        let start = self.offset(from);
        let ends = self.ends[from..].iter().flat_map(|&end| (end - start as u32).to_le_bytes()).collect();
        (ends, self.buf[start..].to_vec())
    }

    /// Appends a segment's strings, refusing one that is malformed or repeats a string.
    fn extend(&mut self, ends: &[u8], bytes: &[u8]) -> StoreResult<()> {
        let corrupt = |what: &str| StoreError::Corrupt(format!("atom segment: {what}"));
        if ends.len() % 4 != 0 {
            return Err(corrupt("its end offsets are not u32s"));
        }
        let mut start = 0usize;
        for end in ends.chunks_exact(4) {
            let end = u32::from_le_bytes(end.try_into().expect("4 bytes")) as usize;
            let s = bytes.get(start..end).ok_or_else(|| corrupt("an end offset is out of order or past its bytes"))?;
            let s = std::str::from_utf8(s).map_err(|_| corrupt("a string is not UTF-8"))?;
            let h = hash_str(s);
            if self.get_h(s, h).is_some() {
                return Err(corrupt("a string is stored twice"));
            }
            self.push(s, h);
            start = end;
        }
        if start != bytes.len() {
            return Err(corrupt("bytes follow its last string"));
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
    atoms: Interner,
    /// Atoms below this are in the database's segments.
    stored: usize,
    /// When the first pending atom was given out.
    since: Option<Instant>,
}

/// A process's atoms of one database (decision 0049): the arena, its watermark and the lease.
pub(crate) struct Atoms {
    /// The database file; `None` for an in-memory database, whose atoms are never written.
    path: Option<PathBuf>,
    state: RwLock<State>,
    /// Held (the lock file, exclusively locked) while this process may give out pending ids.
    /// Lock order: `lease`, then `reader`, then `state`.
    lease: Mutex<Option<File>>,
    leased: AtomicBool,
    /// When this holder last looked for a waiter, and what it saw.
    wanted: Mutex<(Option<Instant>, bool)>,
    /// A connection of this process's own, for reading other processes' segments.
    reader: Mutex<Option<Connection>>,
    /// Per connection (by handle): whether its open transaction appended the pending atoms.
    flushed: Mutex<HashMap<usize, Arc<AtomicBool>>>,
    /// Every id, by string, for range scans of `atoms.value` (`vtab`); extended when read.
    sorted: Mutex<Vec<Sym>>,
    /// Segments this process appended, their atoms and their bytes (the profile's `atoms` line).
    written: Mutex<(u64, u64, u64)>,
}

static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Weak<Atoms>>>> = LazyLock::new(Default::default);
/// The atoms of in-memory databases: one set for the process, as the planner's symbols had.
static MEMORY: LazyLock<Arc<Atoms>> = LazyLock::new(|| Arc::new(Atoms::new(None)));

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

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

impl Atoms {
    fn new(path: Option<PathBuf>) -> Self {
        Atoms {
            path,
            // Atom 0, the empty string, is the schema's first segment.
            state: RwLock::new(State { atoms: Interner::default(), stored: 1, since: None }),
            lease: Mutex::new(None),
            leased: AtomicBool::new(false),
            wanted: Mutex::new((None, false)),
            reader: Mutex::new(None),
            flushed: Mutex::new(HashMap::new()),
            sorted: Mutex::new(Vec::new()),
            written: Mutex::new((0, 0, 0)),
        }
    }

    /// This process's atoms of the database at `path`.
    pub(crate) fn for_path(path: &Path) -> Arc<Atoms> {
        let key = key(path);
        let mut registry = REGISTRY.lock();
        if let Some(atoms) = registry.get(&key).and_then(Weak::upgrade) {
            return atoms;
        }
        let atoms = Arc::new(Atoms::new(Some(key.clone())));
        registry.insert(key, Arc::downgrade(&atoms));
        atoms
    }

    /// Atoms for a database just created at `path`, replacing any this process held for a file
    /// that was there before.
    pub(crate) fn fresh(path: &Path) -> Arc<Atoms> {
        let key = key(path);
        let atoms = Arc::new(Atoms::new(Some(key.clone())));
        REGISTRY.lock().insert(key, Arc::downgrade(&atoms));
        atoms
    }

    /// Makes `path`'s database read and write these atoms in this process: a run's working database
    /// (its SQL's scratch) whose symbols are the store's (decision 0052).
    pub(crate) fn share(self: &Arc<Self>, path: &Path) {
        REGISTRY.lock().insert(key(path), Arc::downgrade(self));
    }

    /// This process's atoms of `connection`'s main database.
    pub(crate) fn for_connection(connection: &Connection) -> Arc<Atoms> {
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
    /// append.
    pub(crate) fn install(connection: &Connection) -> rusqlite::Result<Arc<Atoms>> {
        let atoms = Atoms::for_connection(connection);
        vtab::install(connection, &atoms)?;
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
                    return true;
                }
                false
            }))?;
            connection.rollback_hook(Some(move || flushed.store(false, Ordering::Relaxed)))?;
        }
        Ok(atoms)
    }

    pub(crate) fn len(&self) -> usize {
        self.state.read().atoms.len()
    }

    /// Segments this process appended so far: (segments, atoms, bytes), and the atoms pending now.
    pub(crate) fn writes(&self) -> ((u64, u64, u64), usize) {
        let state = self.state.read();
        (*self.written.lock(), state.atoms.len() - state.stored.min(state.atoms.len()))
    }

    /// Heap bytes the arena holds.
    pub(crate) fn bytes(&self) -> usize {
        self.state.read().atoms.bytes()
    }

    /// Whether atoms are pending (given out, not yet in the database).
    pub(crate) fn pending(&self) -> bool {
        self.path.is_some() && {
            let state = self.state.read();
            state.atoms.len() > state.stored
        }
    }

    /// The atom of `value` in this process's arena, without reading the database.
    pub(crate) fn get(&self, value: &str) -> Option<Sym> {
        self.state.read().atoms.get(value)
    }

    /// `f` of atom `atom`'s string, if the arena holds it.
    pub(crate) fn with_str<R>(&self, atom: Sym, f: impl FnOnce(&str) -> R) -> Option<R> {
        let state = self.state.read();
        ((atom as usize) < state.atoms.len()).then(|| f(state.atoms.str(atom)))
    }

    /// The atom of `value`, if the database or this process holds it.
    pub(crate) fn lookup(&self, value: &str) -> StoreResult<Option<Sym>> {
        if let Some(atom) = self.get(value) {
            return Ok(Some(atom));
        }
        self.catch_up()?;
        Ok(self.get(value))
    }

    /// The string of `atom`, if the database or this process holds it.
    pub(crate) fn value(&self, atom: Sym) -> StoreResult<Option<String>> {
        if (atom as usize) >= self.len() {
            self.catch_up()?;
        }
        Ok(self.with_str(atom, str::to_owned))
    }

    /// The atom of each of `values`, giving out new ones (pending) under the lease.
    pub(crate) fn intern_all(&self, values: &[&str]) -> StoreResult<Vec<Sym>> {
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
        let mut state = self.state.write();
        let ids = values.iter().map(|value| state.atoms.intern(value)).collect();
        if state.since.is_none() && state.atoms.len() > state.stored {
            state.since = Some(Instant::now());
        }
        Ok(ids)
    }

    /// The atom of `value`, given out (pending) under the lease if it is new.
    pub(crate) fn intern(&self, value: &str) -> StoreResult<Sym> {
        if let Some(atom) = self.get(value) {
            return Ok(atom);
        }
        Ok(self.intern_all(&[value])?[0])
    }

    /// Takes the lease if this process does not hold it, then reads what other processes wrote.
    fn acquire(&self, lease: &mut Option<File>) -> StoreResult<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if lease.is_some() {
            return Ok(());
        }
        let open = |suffix: &str| {
            std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(sidecar(path, suffix))
        };
        let file = open("-atoms.lock")?;
        match file.try_lock() {
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
                    match file.try_lock() {
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
        // No other process gives out ids now: read what they wrote before this one gives any out.
        self.read_segments()
    }

    /// Reads segments other processes wrote, unless this process holds the lease (then there are
    /// none it lacks).
    fn catch_up(&self) -> StoreResult<()> {
        if self.leased.load(Ordering::Acquire) {
            return Ok(());
        }
        // Under the lease's mutex: no thread of this process gives out ids while it reads.
        let lease = self.lease.lock();
        if lease.is_some() {
            return Ok(());
        }
        self.read_segments()
    }

    /// Appends the database's segments from `stored` on, which must be everything this process
    /// holds (none pending).
    fn read_segments(&self) -> StoreResult<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let mut reader = self.reader.lock();
        if reader.is_none() {
            if !path.exists() {
                return Ok(());
            }
            let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            connection.busy_timeout(Duration::from_secs(5))?;
            *reader = Some(connection);
        }
        let connection = reader.as_ref().expect("opened above");
        let from = self.state.read().stored;
        let mut statement = connection.prepare_cached("SELECT first, ends, bytes FROM atom_segment WHERE first >= ?1 ORDER BY first")
            .map_err(|error| StoreError::Corrupt(format!("{}: reading atom segments: {error}", path.display())))?;
        let segments = statement
            .query_map([from as i64], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        if segments.is_empty() {
            return Ok(());
        }
        let mut state = self.state.write();
        if state.atoms.len() != state.stored {
            return Err(StoreError::Corrupt("another process wrote atoms while this one had atoms pending".to_owned()));
        }
        for (first, ends, bytes) in segments {
            if first as usize != state.atoms.len() {
                return Err(StoreError::Corrupt(format!("atom segments are not contiguous at {first}")));
            }
            state.atoms.extend(&ends, &bytes)?;
            state.stored = state.atoms.len();
        }
        Ok(())
    }

    /// In `connection`'s open transaction, if it writes the main database: appends the pending
    /// atoms as one segment and marks the transaction as having done so (the commit hook). Returns
    /// the watermark to pass to `committed` after the commit.
    pub(crate) fn write_pending(&self, connection: &Connection) -> StoreResult<usize> {
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
        let (from, ends, bytes, upto) = {
            let state = self.state.read();
            let (end, len) = (end as usize, state.atoms.len());
            if end < state.stored {
                return Err(StoreError::Corrupt(format!("the database's atoms end at {end}, before the {} this process wrote", state.stored)));
            }
            if end >= len {
                return Ok(end);
            }
            let (ends, bytes) = state.atoms.segment(end);
            (end, ends, bytes, len)
        };
        let size = (ends.len() + bytes.len()) as u64;
        connection
            .prepare_cached("INSERT INTO atom_segment(first, ends, bytes) VALUES (?1, ?2, ?3)")?
            .execute(rusqlite::params![from as i64, ends, bytes])?;
        // Counted when written; a transaction that then rolls back still counts.
        let mut written = self.written.lock();
        *written = (written.0 + 1, written.1 + (upto - from) as u64, written.2 + size);
        Ok(upto)
    }

    /// After a commit that held `write_pending`'s segment: atoms below `upto` are stored. The
    /// lease is given back once none is pending.
    pub(crate) fn committed(&self, upto: usize) {
        if self.path.is_none() {
            return;
        }
        let mut lease = self.lease.lock();
        let mut state = self.state.write();
        state.stored = state.stored.max(upto.min(state.atoms.len()));
        if state.stored == state.atoms.len() {
            state.since = None;
            if lease.take().is_some() {
                self.leased.store(false, Ordering::Release);
            }
        }
    }

    /// Whether a flush is due: enough pending (count, bytes or age), or another process waiting.
    pub(crate) fn due(&self) -> bool {
        if self.path.is_none() {
            return false;
        }
        {
            let state = self.state.read();
            let pending = state.atoms.len() - state.stored;
            if pending == 0 {
                return false;
            }
            if pending >= DUE_ATOMS
                || state.atoms.buf.len() - state.atoms.offset(state.stored) >= DUE_BYTES
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
            .open(sidecar(path, "-atoms.want"))
            .is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
        *wanted = (Some(Instant::now()), seen);
        seen
    }

    /// Appends the pending atoms in a transaction of their own on `connection`, unless it is inside
    /// a transaction (its commit will) or another writer holds the database now (a later one will).
    pub(crate) fn flush(&self, connection: &Connection) -> StoreResult<()> {
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
    pub(crate) fn flush_if_due(&self, connection: &Connection) -> StoreResult<()> {
        if self.due() { self.flush(connection) } else { Ok(()) }
    }
}

/// Commits `connection`'s open transaction (`BEGIN` … ), appending the pending atoms first if it
/// writes the main database: for transactions written as SQL rather than through the store.
pub(crate) fn commit(connection: &Connection) -> StoreResult<()> {
    let atoms = Atoms::for_connection(connection);
    let upto = atoms.write_pending(connection)?;
    connection.execute_batch("COMMIT")?;
    atoms.committed(upto);
    Ok(())
}

/// `temp.atoms(atom, value)` on `connection`: the atoms its database's segments hold, decoded, and
/// nothing pending in any process. For checks of what a store holds on disk (after a crash, in a
/// capture), on a connection without the `atoms` virtual table; the temp table hides it otherwise.
pub(crate) fn materialize_stored(connection: &Connection) -> StoreResult<()> {
    let mut atoms = Interner::default();
    {
        let mut statement = connection.prepare("SELECT first, ends, bytes FROM main.atom_segment WHERE first > 0 ORDER BY first")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, i64>(0)? as usize != atoms.len() {
                return Err(StoreError::Corrupt("atom segments are not contiguous".to_owned()));
            }
            atoms.extend(&row.get::<_, Vec<u8>>(1)?, &row.get::<_, Vec<u8>>(2)?)?;
        }
    }
    connection.execute_batch("DROP TABLE IF EXISTS temp.atoms; CREATE TEMP TABLE atoms(atom INTEGER PRIMARY KEY, value TEXT NOT NULL);")?;
    let mut insert = connection.prepare("INSERT INTO temp.atoms(atom, value) VALUES (?1, ?2)")?;
    for atom in 0..atoms.len() as Sym {
        insert.execute(rusqlite::params![i64::from(atom), atoms.str(atom)])?;
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
    pub(crate) fn share_atoms_with(&self, path: &Path) {
        self.atoms.share(path);
    }

    pub(crate) fn flush_atoms(&self) -> StoreResult<()> {
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
    pub(crate) fn atom_writes(&self) -> ((u64, u64, u64), usize) {
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
    pub(crate) fn commit(&self, transaction: rusqlite::Transaction<'_>) -> StoreResult<()> {
        let upto = self.atoms.write_pending(&transaction)?;
        transaction.commit()?;
        self.atoms.committed(upto);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
