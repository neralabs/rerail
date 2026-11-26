//! The atoms (decision 0009): the interner that numbers every text the engine holds as a symbol, its blocks in the
//! store's files and their chain (`stored`), the map between this process's ids and the store's after a fork (`map`),
//! the `atoms` virtual table (`vtab`), and the guests' keys' registry each arena keeps (`guest_keys`). Below the
//! store (crate split step 10b-3): the crates that intern and read atoms need no store. The store's calls over them
//! (`ContentStore::intern_atoms`, its writer's flush) and the kept facts installed beside them on its connections
//! (`content_store::attach_atoms`) are the store's.

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
use rerail_store_base::content_store::Cached as _;
use rerail_store_base::content_store::{StoreError, StoreResult};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, ffi};
use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

pub mod map;
pub mod stored;
pub mod guest_keys;
pub use stored::{atoms_dir, copy_blocks, freeze_blocks};
pub use stored::clone_file;
pub mod vtab;
pub use map::{ProcessIds, StoreIds, map_columns, mapped};

pub use rerail_store_base::content_store::Sym;

/// The values `Atoms::intern_all` looks up under one read lock (decision 0315): a writer waits at most this many
/// lookups for it.
const READ_RUN: usize = 256;

/// The fixed hash every string key uses: the interner's table, and the dependency keys of
/// probes (`Engine::probe`), which must agree for strings that are not (yet) interned.
pub fn hash_str(s: &str) -> u64 {
    foldhash::quality::FixedState::with_seed(0x005E_ED71).hash_one(s)
}

/// The atoms a process holds: the store's, read where they lie in its mapped blocks (decision 0401, `Stored`), and
/// the atoms from `base` on, given out here and not yet written (pending), back to back on its heap. Ids below `base`
/// are the store's; atom 0, the empty string, is every store's and no block's.
pub struct Interner {
    stored: stored::Stored,
    base: Sym,
    /// The pending atoms (`base` on), in atoms blocks of the stored chain's format in anonymous memory, each twice
    /// the last (decision 0478 step 2a): every string, its end, its hash and the table from hash to id inside its
    /// block, where any reader reads it. A block may begin below `base`: its atoms stored since are read in the
    /// store's chain, and a block wholly stored is let go.
    pending: Vec<rerail_blocks::atoms::AtomsW>,
}

/// The first pending block's room: atoms, and string bytes. Each next has twice the last's (or a long string's).
pub fn pending_first_cap() -> u32 {
    if small_blocks() { 64 } else { 1 << 12 }
}
fn pending_first_bytes() -> u64 {
    64 * u64::from(pending_first_cap())
}

/// Whether blocks start small (the store's tests, `set_small_blocks`), so that a test's store grows chains and rolls
/// them up.
pub fn small_blocks() -> bool {
    #[cfg(feature = "test-hooks")]
    {
        SMALL_BLOCKS.load(Ordering::Relaxed)
    }
    #[cfg(not(feature = "test-hooks"))]
    {
        false
    }
}

#[cfg(feature = "test-hooks")]
static SMALL_BLOCKS: AtomicBool = AtomicBool::new(false);

/// The test hook: blocks start small in this process, as the store's own tests have them (they did under the
/// store's `cfg(test)` before the atoms were a crate). Set as the test binary loads, before any atom.
#[cfg(feature = "test-hooks")]
pub fn set_small_blocks() {
    SMALL_BLOCKS.store(true, Ordering::Relaxed);
}

impl Default for Interner {
    fn default() -> Self {
        Interner::over(None)
    }
}

impl Interner {
    /// The atoms of the store at `database` (none read yet), or of an in-memory database's.
    fn over(database: Option<&Path>) -> Self {
        Interner { stored: stored::Stored::new(database), base: 1, pending: Vec::new() }
    }

    /// The pending block that holds `s`, if one does.
    fn block_of(&self, s: Sym) -> Option<&rerail_blocks::atoms::AtomsW> {
        use rerail_blocks::atoms::AtomRead;
        self.pending.iter().rev().find(|b| b.ids().contains(&s))
    }

    /// The pending blocks' memory, each its first byte and its reservation's end.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn pending_regions(&self) -> Vec<(usize, usize)> {
        self.pending.iter().map(rerail_blocks::atoms::AtomsW::region).collect()
    }

    /// Every atom this process holds as runs where their blocks hold them, with the blocks (decision 0478 step 2b):
    /// the store's chain below `base`, then the pending blocks. A directory of them (`AtomsDir`) is read by the atoms
    /// reads written once in CLIF (`atoms_at`, `atoms_len`), on the host and the device alike. The runs hold the ids
    /// held now: a reader that keeps the blocks reads them while atoms are interned and stored after.
    pub fn runs(&self) -> Vec<(rerail_blocks::atoms::Run, Arc<rerail_blocks::block::Block>)> {
        let mut runs = self.stored.runs(self.base);
        runs.extend(self.pending.iter().filter_map(|w| Some((w.run_from(self.base), Arc::clone(w.block()))).filter(|(r, _)| r.first < r.end)));
        runs
    }

    /// `runs` as a directory for the host.
    pub fn dir(&self) -> rerail_blocks::atoms::AtomsDir {
        rerail_blocks::atoms::AtomsDir::host(&self.runs().into_iter().map(|(r, _)| r).collect::<Vec<_>>())
    }

    /// The string of `s`, if this process holds it: the store's (mapped), or pending.
    pub fn str(&self, s: Sym) -> Option<&str> {
        if s >= self.base {
            use rerail_blocks::atoms::AtomRead;
            return self.block_of(s)?.str(s);
        }
        if s == 0 {
            return Some("");
        }
        self.stored.str(s)
    }

    /// The hash of `s`'s string (`hash_str`), if this process holds it: kept with each atom, so no string is read.
    fn hash_of(&self, s: Sym) -> Option<u64> {
        if s >= self.base {
            use rerail_blocks::atoms::AtomRead;
            return self.block_of(s)?.hash(s);
        }
        if s == 0 {
            return Some(hash_str(""));
        }
        self.stored.hash(s)
    }

    pub fn get(&self, s: &str) -> Option<Sym> {
        self.get_h(s, hash_str(s))
    }

    /// The id of `s` (its hash `h`): pending here, or the store's below `base` (found in its blocks' tables).
    pub fn get_h(&self, s: &str, h: u64) -> Option<Sym> {
        if s.is_empty() {
            return Some(0);
        }
        use rerail_blocks::atoms::AtomRead;
        let base = self.base;
        if let Some(id) = self.pending.iter().find_map(|b| b.find(s, h).filter(|&id| id >= base)) {
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
        if let Some(got) = self.pending.last_mut().filter(|b| b.end() == id).and_then(|b| b.append(s, h)) {
            return got;
        }
        // The next block, twice the last (and room for this string).
        let (cap, bytes) = self.pending.last().map_or((pending_first_cap(), pending_first_bytes()), |b| (b.cap().saturating_mul(2), b.bytes_cap() * 2));
        let mut b = rerail_blocks::atoms::AtomsW::anon(id, cap, bytes.max(2 * s.len() as u64));
        let got = b.append(s, h).expect("room for its first atom");
        self.pending.push(b);
        got
    }

    /// One past the last id: the database's (as far as this process knows) and the pending ones.
    pub fn len(&self) -> usize {
        self.pending.last().map_or(self.base, |b| b.end().max(self.base)) as usize
    }

    /// No ids, in the database or pending.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where the pending atoms start: ids below it are in the database.
    fn base(&self) -> usize {
        self.base as usize
    }

    /// How many atoms are pending, and their bytes.
    fn pending(&self) -> (usize, usize) {
        let bytes: u64 = self.pending.iter().map(|b| b.bytes_of(self.base, Sym::MAX)).sum();
        (self.len() - self.base(), bytes as usize)
    }

    /// Bytes the pending atoms' blocks hold (their pages taken). The store's are mapped (`mapped`).
    pub fn bytes(&self) -> usize {
        self.pending.iter().map(rerail_blocks::atoms::AtomsW::used).sum()
    }

    /// Bytes of the store's blocks mapped.
    pub fn mapped(&self) -> usize {
        self.stored.mapped()
    }

    /// Appends the pending atoms from `from` to `upto` to the store's open block, in `connection`'s write transaction
    /// (`Stored::append`): read from the pending buffer as they are, not copied.
    fn append_pending(&mut self, connection: &Connection, from: usize, upto: usize) -> StoreResult<Sym> {
        use rerail_blocks::atoms::AtomRead;
        let Interner { stored, pending, .. } = self;
        let of = |id: Sym| pending.iter().rev().find(|b| b.ids().contains(&id)).expect("a pending atom's block");
        let run = (from..upto).map(|id| {
            let b = of(id as Sym);
            (b.str(id as Sym).expect("a pending atom"), b.hash(id as Sym).expect("a pending atom's hash"))
        });
        stored.append(connection, from as Sym, run)
    }

    /// The pending atoms below `upto` are stored now (their commit reached it): read where they lie in the blocks,
    /// and their heap copies dropped.
    fn store_upto(&mut self, upto: usize) -> StoreResult<()> {
        let n = upto.saturating_sub(self.base()).min(self.len() - self.base());
        if n == 0 {
            return Ok(());
        }
        self.stored.view_to(upto as Sym)?;
        self.base += n as Sym;
        // A block wholly stored is let go: its atoms are read in the store's chain. A block with atoms still pending
        // stays as it is, nothing copied.
        let base = self.base;
        self.pending.retain(|b| b.end() > base);
        Ok(())
    }

    /// The database's ids end at `end` (its blocks `rows`), past what this process knew, and none is pending here: new
    /// ids continue from there, and the atoms between are read where they lie.
    fn advance(&mut self, end: usize, rows: &[stored::Row]) -> StoreResult<()> {
        if end <= self.base() {
            return Ok(());
        }
        if self.len() > self.base() {
            return Err(StoreError::Corrupt("another process wrote atoms while this one had atoms pending".to_owned()));
        }
        self.pending.clear();
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

pub struct State {
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
    guest_keys: Arc<crate::guest_keys::GuestKeys>,
    /// Its atoms reserved by components not yet retired (`reserving`).
    reserved: ReservedSet,
    /// What these atoms keep of their texts (decision 0478 steps 2d, 2e): verdicts and texts, shared as the atoms are.
    kept: std::sync::OnceLock<Kept>,
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
    pub flush_queued: AtomicBool,
}

/// The identity read of each database file, by its path and (device, inode, creation time): `Atoms::identity`. A file
/// made again at a path may have the inode the deleted one had; it was not made at the same time.
#[derive(Default)]
struct Identities(Mutex<HashMap<String, (FileIdentity, Option<i64>)>>);

/// A database file's (device, inode, creation time).
type FileIdentity = (u64, u64, u128);

/// The database at `path` was made a store (`schema::initialize`): the identity read of it before (none) is gone.
pub fn forget_identity(path: &str) {
    rerail_prof::code_rule_dsl::logic::profile::shared::<Identities>().0.lock().remove(path);
}

thread_local! {
    /// The atoms this thread made while reserving (`reserving`), by arena.
    static RESERVING: std::cell::RefCell<Option<Reserving>> = const { std::cell::RefCell::new(None) };
}

/// An arena's atoms reserved by components not yet retired (`Atoms::reserved`): pending, but no row a commit writes
/// holds them yet (decision 0114 §4). The arena's own (decision 0442), held by each reservation of its atoms.
type ReservedSet = Arc<parking_lot::Mutex<std::collections::BTreeSet<Sym>>>;

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
/// What a store's atoms keep of their texts (decision 0478 steps 2d and 2e): the one rolling cache of
/// `rerail_clif::view::cache`, in a block of its own, read and written through the reads written once in CLIF by the
/// CPU and the kernels alike. It holds the text tests' verdicts (`verdict_get`, `verdict_put`: an entry 64 symbols of
/// one test) and the text functions' texts (`text_find`, `text_open`: an entry a text, by its functions and its
/// symbol, at most `TEXT_MOST` bytes). Three generations of `KEPT_ENTRIES` entries and `KEPT_ARENA` bytes bound it
/// (LRU: an entry read is copied into the head).
pub struct Kept {
    block: rerail_blocks::block::Block,
    at: usize,
}

/// A generation's entries (verdicts' and texts'), and its arena's bytes: its pages taken as entries are written.
pub const KEPT_ENTRIES: u32 = 1 << 16;
pub const KEPT_ARENA: usize = 16 << 20;
/// The longest text kept (decision 0478 step 2e): a longer one is made each time it is read.
pub const TEXT_MOST: usize = 4096;
// The entry layout the kernels read a kept text by (`metal_join`'s `row_text`: its bytes at 12, its payload at 16),
// and the most they keep.
const _: () = assert!(rerail_clif::view::cache::LEN == 12 && rerail_clif::view::cache::PAYLOAD == 16 && TEXT_MOST == 4096);

/// A verdict (`rerail_clif::view::cache`'s, the reads'): none yet, the symbol fails, it passes.
pub use rerail_clif::view::cache::{FAILS, PASSES, UNKNOWN};

// SAFETY: the cache's words are read and written by its reads' atomics alone.
unsafe impl Send for Kept {}
unsafe impl Sync for Kept {}

impl Kept {
    fn new() -> Kept {
        use rerail_clif::view::cache::{Spec, PAYLOAD};
        let spec = Spec::arena(3, KEPT_ENTRIES, KEPT_ARENA, PAYLOAD as usize + TEXT_MOST);
        let block = rerail_blocks::block::Block::anon(rerail_blocks::block::kind::SCRATCH, spec.bytes() + 2 * 4096);
        let at = block.alloc(spec.bytes(), 256);
        // SAFETY: the cache's bytes, in the block, which no reader has yet: zero, as a fresh mapping gives them.
        spec.init_zeroed(unsafe { std::slice::from_raw_parts_mut(block.ptr::<u8>(at), spec.bytes()) }).expect("a cache of its size");
        Kept { block, at }
    }

    /// The cache's first byte on the host (what its reads take), and its offset in its block (a kernel binds the
    /// block there).
    pub fn cache(&self) -> *const u8 {
        self.block.ptr::<u8>(self.at).cast_const()
    }
    pub fn at(&self) -> usize {
        self.at
    }
    pub fn block(&self) -> &rerail_blocks::block::Block {
        &self.block
    }

    /// Test `test`'s verdict for `sym`: `PASSES`, `FAILS` or `UNKNOWN`.
    pub fn get(&self, test: u32, sym: u32) -> u32 {
        // SAFETY: a cache `new` made, in the block it keeps.
        unsafe { rerail_clif::view::cache::verdict_get(self.cache(), test, sym) }
    }

    /// The text functions `fns` (`text::automata::text_functions`) made of `sym`, if kept.
    pub fn text(&self, fns: u32, sym: u32) -> Option<String> {
        use rerail_clif::view::cache::{LEN, PAYLOAD};
        // SAFETY: as `get`'s; an entry found is complete, and its bytes are read before this returns.
        unsafe {
            let r = rerail_clif::view::cache::text_find(self.cache(), fns, sym);
            if r & 3 != 0 {
                return None;
            }
            let e = (r & !3) as *const u8;
            let len = e.add(LEN as usize).cast::<u32>().read() as usize - PAYLOAD as usize;
            std::str::from_utf8(std::slice::from_raw_parts(e.add(PAYLOAD as usize), len)).ok().map(str::to_owned)
        }
    }

    /// Keeps `text`, functions `fns` of `sym`; whether it was (none past `TEXT_MOST`, none with no room, none another
    /// thread writes).
    pub fn keep_text(&self, fns: u32, sym: u32, text: &str) -> bool {
        use rerail_clif::view::cache::PAYLOAD;
        if text.len() > TEXT_MOST {
            return false;
        }
        // SAFETY: as `get`'s; an entry taken is this thread's to write, `text.len()` bytes at its payload.
        unsafe {
            let r = rerail_clif::view::cache::text_open(self.cache(), fns, sym, text.len() as u32);
            if r & 3 != rerail_clif::view::cache::TAKEN as u64 {
                return false;
            }
            let e = (r & !3) as *mut u8;
            std::ptr::copy_nonoverlapping(text.as_ptr(), e.add(PAYLOAD as usize), text.len());
            rerail_clif::view::cache::done(self.cache(), r);
        }
        true
    }

    /// The cache's header word at `at` (`view::cache`'s counts and modes), which its reads keep with atomics.
    fn header(&self, at: i32) -> &std::sync::atomic::AtomicU32 {
        // SAFETY: a u32 of the cache's header, in the block `self` keeps, aligned as the header is.
        unsafe { &*self.cache().add(at as usize).cast::<std::sync::atomic::AtomicU32>() }
    }

    /// Keeps the cache's statistics too (hits, waits, duplicates), for a test or a tuning run.
    pub fn count_statistics(&self) {
        self.header(rerail_clif::view::cache::STATS).store(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// The cache's counts, named: lookups with no room, misses, entries promoted, generations turned, and (with
    /// `count_statistics`) hits, waits and duplicates.
    pub fn counts(&self) -> String {
        use rerail_clif::view::cache as c;
        [("failed", c::FAILED), ("misses", c::MISSES), ("promoted", c::PROMOTED), ("rotated", c::ROTATED), ("hits", c::HITS), ("waited", c::WAITED), ("duplicates", c::DUPLICATES)]
            .map(|(name, at)| format!("{name} {}", self.header(at).load(std::sync::atomic::Ordering::Relaxed)))
            .join(", ")
    }

    /// Keeps `test`'s verdict for `sym` (`PASSES` or `FAILS`); whether the cache had room.
    pub fn put(&self, test: u32, sym: u32, passes: bool) -> bool {
        // SAFETY: as `get`'s.
        unsafe { rerail_clif::view::cache::verdict_put(self.cache(), test, sym, if passes { PASSES } else { FAILS }) }
    }
}

pub fn reserving(arena: &Arc<Atoms>) -> Reservation {
    Reservation(RESERVING.with_borrow_mut(|r| r.replace((Arc::as_ptr(arena) as usize, Vec::new()))))
}

/// A thread's reservation in progress (`reserving`): `end` gives what it made. It holds the reservation it was
/// made inside, which goes on when it ends.
pub struct Reservation(Option<Reserving>);

/// A reservation under way: its arena (by address), and each reserved set with the atoms made in it.
type Reserving = (usize, Vec<(ReservedSet, Vec<Sym>)>);

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
pub struct Reserved(Vec<(ReservedSet, Vec<Sym>)>);

impl Reserved {
    /// Takes in what a later reservation of the same component made (its members resumed after their wave).
    pub fn extend(&mut self, mut more: Reserved) {
        self.0.append(&mut more.0);
    }

    pub fn len(&self) -> usize {
        self.0.iter().map(|(_, ids)| ids.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|(_, ids)| ids.is_empty())
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        for (set, ids) in self.0.drain(..) {
            let mut set = set.lock();
            ids.iter().for_each(|id| { set.remove(id); });
        }
    }
}

/// Decision 0160's trial (`RERAIL_ATOMS_PROVISIONAL=1`): ids are provisional and the lease is never taken. Off, the
/// lease is taken as before (decision 0049).
fn lease_free() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_ATOMS_PROVISIONAL").is_ok_and(|v| v == "1"))
}

/// The error of a write refused after a fork (decisions 0160, 0170): its rows may name this process's ids, not the
/// store's. A write behind the run that meets it is dropped, as a cache's miss.
pub const DIVERGED_TEXT: &str = "this process's atoms forked from the store's and this write's are not mapped (decision 0170)";

#[derive(Default)]
struct AtomsByFile(Mutex<HashMap<PathBuf, Weak<Atoms>>>);
/// The atoms of in-memory databases: one set for the shared object (decision 0442), as the planner's symbols had:
/// another object's in-memory databases have their own, and read none of these texts.
struct MemoryAtoms(Arc<Atoms>);

impl Default for MemoryAtoms {
    fn default() -> Self {
        MemoryAtoms(Arc::new(Atoms::new(None, None)))
    }
}

fn memory() -> Arc<Atoms> {
    rerail_prof::code_rule_dsl::logic::profile::with_shared::<MemoryAtoms, _>(|m| Arc::clone(&m.0))
}

/// The in-memory connections that borrow a store's atoms (`share_memory`), by handle: the shared object's
/// (decision 0442), as the connections are.
#[derive(Default)]
struct MemoryArenas(Mutex<HashMap<usize, Weak<Atoms>>>);

fn arenas() -> Arc<MemoryArenas> {
    rerail_prof::code_rule_dsl::logic::profile::shared::<MemoryArenas>()
}

struct MemoryArena(usize, Arc<Atoms>);
impl Drop for MemoryArena {
    fn drop(&mut self) {
        arenas().0.lock().remove(&self.0);
    }
}

/// A path's registry key: its canonical form, resolved once per path (a `realpath` per stratum was 2.5% of a corpus
/// run's main thread). A path that does not resolve yet is not remembered. Where the thread holds no object (`Atoms`'
/// drop on the pool thread whose job let the store's last holder go, its run gone), resolved here and not remembered.
pub fn key(path: &Path) -> PathBuf {
    #[derive(Default)]
    struct CanonicalPaths(Mutex<HashMap<PathBuf, PathBuf>>);
    if rerail_prof::code_rule_dsl::logic::profile::starter_object().is_none() {
        return dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    }
    let held_ = rerail_prof::code_rule_dsl::logic::profile::shared::<CanonicalPaths>();
    if let Some(key) = held_.0.lock().get(path) {
        return key.clone();
    }
    match dunce::canonicalize(path) {
        Ok(key) => {
            rerail_prof::code_rule_dsl::logic::profile::shared::<CanonicalPaths>().0.lock().insert(path.to_path_buf(), key.clone());
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
pub fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

use stored::database_end;

impl Atoms {
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self::new(None, None))
    }

    fn new(path: Option<PathBuf>, identity: Option<i64>) -> Self {
        let atoms = Interner::over(path.as_deref());
        Atoms {
            path,
            identity,
            // Atom 0, the empty string, is every store's.
            state: rerail_accel::sharded::Sharded::new(State { atoms, since: None, map: None }),
            guest_keys: Arc::default(),
            reserved: ReservedSet::default(),
            kept: std::sync::OnceLock::new(),
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
        let held_ = rerail_prof::code_rule_dsl::logic::profile::shared::<AtomsByFile>();
        let mut registry = held_.0.lock();
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
    /// Every atom held now as runs where their blocks hold them, with the blocks (`Interner::runs`).
    pub fn runs(&self) -> Vec<(rerail_blocks::atoms::Run, Arc<rerail_blocks::block::Block>)> {
        self.state.read().atoms.runs()
    }

    /// What these atoms keep of their texts (verdicts, texts), made when a test or a text function first asks.
    pub fn kept(&self) -> &Kept {
        self.kept.get_or_init(Kept::new)
    }

    pub fn guest_keys(&self) -> Arc<crate::guest_keys::GuestKeys> {
        Arc::clone(&self.guest_keys)
    }

    pub fn for_database(path: &Path, identity: Option<i64>) -> Arc<Atoms> {
        let key = key(path);
        let held_ = rerail_prof::code_rule_dsl::logic::profile::shared::<AtomsByFile>();
        let mut registry = held_.0.lock();
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
        // Windows: the volume's serial and the file's index, as GetFileInformationByHandle gives them. Its birth
        // time alone would not tell the files apart: NTFS's tunnelling gives a file made again under a name, within
        // seconds, the deleted one's.
        #[cfg(windows)]
        let file = connection.path().filter(|p| !p.is_empty()).and_then(|p| {
            use std::os::windows::io::AsRawHandle as _;
            use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
            let opened = std::fs::File::open(p).ok()?;
            // SAFETY: plain data, written whole by the call below.
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: the handle stays open for the call.
            if unsafe { GetFileInformationByHandle(opened.as_raw_handle(), &mut info) } == 0 {
                return None;
            }
            let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
            let made = (u128::from(info.ftCreationTime.dwHighDateTime) << 32) | u128::from(info.ftCreationTime.dwLowDateTime);
            Some((p.to_owned(), (u64::from(info.dwVolumeSerialNumber), index, made)))
        });
        #[cfg(not(any(unix, windows)))]
        let file: Option<(String, (u64, u64, u128))> = None;
        if let Some((path, at)) = &file
            && let Some((held, id)) = rerail_prof::code_rule_dsl::logic::profile::shared::<Identities>().0.lock().get(path)
            && held == at
        {
            return Ok(*id);
        }
        let id = Self::identity_read(connection)?;
        if let Some((path, at)) = file {
            rerail_prof::code_rule_dsl::logic::profile::shared::<Identities>().0.lock().insert(path, (at, id));
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
        rerail_prof::code_rule_dsl::logic::profile::shared::<AtomsByFile>().0.lock().insert(key(path), Arc::downgrade(self));
    }

    /// This process's atoms of `connection`'s main database.
    pub fn for_connection(connection: &Connection) -> Arc<Atoms> {
        match connection.path().filter(|path| !path.is_empty()) {
            Some(path) => Atoms::for_path(Path::new(path)),
            None => arenas().0.lock().get(&(unsafe { connection.handle() } as usize))
                .and_then(std::sync::Weak::upgrade).unwrap_or_else(memory),
        }
    }

    /// Borrow the store's atoms for the life of an in-memory SQL connection.
    pub fn share_memory(connection: &Connection, atoms: Arc<Self>) -> rusqlite::Result<()> {
        let lease = MemoryArena(unsafe { connection.handle() } as usize, atoms);
        arenas().0.lock().insert(lease.0, Arc::downgrade(&lease.1));
        // SQLite drops the closure when this connection closes.
        connection.create_scalar_function(c"rr_memory_arena", 0, rusqlite::functions::FunctionFlags::SQLITE_UTF8, move |_| {
            let _lease = &lease;
            Ok(0)
        })
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
                unsafe { rerail_store_base::content_store::hazards::warm_commit(handle as *mut ffi::sqlite3) };
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
            None => Atoms::for_connection(connection),
        };
        vtab::install(connection, &atoms)?;
        Ok(atoms)
    }

    pub fn len(&self) -> usize {
        self.state.read().atoms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.read().atoms.is_empty()
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
        let reserved = self.reserved.lock().range(base..).count();
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
    /// provisional ids, forked, or the store's blocks mapped to their end while this process holds the lease. Atomic loads only, no
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
    pub fn release_reader(&self) {
        self.reader.lock().take();
        self.state.write().atoms.stored.release_writer();
    }

    /// The atom of each of `values`, giving out new ones (pending) under the lease. Decision 0315: each value is
    /// hashed once, outside any lock; those held are found under the read lock, and only the rest are looked for
    /// again and given ids under the write lock (`give`), so a batch of mostly known values stops the arena's readers
    /// for its new ones only.
    pub fn intern_all(&self, values: &[&str]) -> StoreResult<rerail_blocks::vector::Vector<Sym>> {
        let hashes: rerail_blocks::vector::Vector<u64> = values.iter().map(|value| hash_str(value)).collect();
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
        // Inside a run, with the store's blocks mapped to its end: new ids are provisional, given out without the lease and checked
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
    fn give(&self, values: &[&str], hashes: &[u64], found: Vec<Option<Sym>>) -> rerail_blocks::vector::Vector<Sym> {
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
        let ids: rerail_blocks::vector::Vector<Sym> = ids.into_iter().map(|id| id.expect("every value given an id")).collect();
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
                    self.reserved.lock().extend(new.iter().copied());
                    r.push((Arc::clone(&self.reserved), new));
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
                    rerail_store_base::content_store::hazards::behind_the_run();
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
    pub fn read_all(&self) -> StoreResult<()> {
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
        } else if self.runs.fetch_add(1, Ordering::AcqRel) == 0 && !self.leased.load(Ordering::Acquire) && !rerail_store_base::content_store::hazards::inside_waves() {
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
        if lease_free() && rerail_store_base::content_store::hazards::inside_waves() {
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

    /// Whether stored rows whose symbols are at most `max` (as the store numbers them) read as this process numbers
    /// them, held as `hold_named` holds them: a kept block's rows, read where they lie (decision 0401 F3), which no
    /// reader maps. Not past a fork (decision 0170), where the store's ids are another numbering's.
    pub fn reads_in_place(&self, max: i64) -> StoreResult<bool> {
        self.hold_named(&[max])?;
        if !self.forked() {
            return Ok(true);
        }
        let state = self.state.read();
        Ok(state.map.as_ref().is_some_and(|m| Sym::try_from(max).is_ok_and(|v| v < m.fork)))
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

    pub fn due_now(&self) -> bool {
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
    atoms.pending.iter().map(|b| b.bytes_of(from as Sym, upto as Sym)).sum::<u64>() as usize
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

/// The atoms as a batch interns and reads them (`content_store::AtomTable`, the store's `atoms_handle`).
impl rerail_store_base::content_store::AtomTable for Atoms {
    fn intern(&self, value: &str) -> StoreResult<Sym> {
        Atoms::intern(self, value)
    }
    fn intern_all(&self, values: &[&str]) -> StoreResult<rerail_blocks::vector::Vector<Sym>> {
        Atoms::intern_all(self, values)
    }
    fn value(&self, atom: Sym) -> StoreResult<Option<String>> {
        Atoms::value(self, atom)
    }
}

/// What the store's tests read inside the atoms (the test hooks; they were the store's own module's tests).
#[cfg(feature = "test-hooks")]
impl Atoms {
    /// `f` of the interner as it is now.
    pub fn with_interner<T>(&self, f: impl FnOnce(&Interner) -> T) -> T {
        f(&self.state.read().atoms)
    }
    /// Whether this process holds the store's lease.
    pub fn leased_now(&self) -> bool {
        self.leased.load(Ordering::Acquire)
    }
    /// Whether the arena holds every atom its store has.
    pub fn complete_now(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }
    /// The atoms of the store at `path` (`for_path`).
    pub fn of_path(path: &Path) -> Arc<Atoms> {
        Self::for_path(path)
    }
}

#[cfg(feature = "test-hooks")]
impl Interner {
    /// The pending blocks it holds.
    pub fn pending_blocks(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests;
