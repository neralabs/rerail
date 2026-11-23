//! A relation: its rows, its kills, and, once frozen, everything reads need, in one descriptor.
//!
//! **Freeze** turns rows written in any order (by many threads, or a GPU's claims) into a relation read in a
//! fixed order:
//!
//! - **Set semantics.** It drops duplicate rows, and in a delta it also drops rows its base still holds live.
//! - **The canonical order.** The live rows, sorted by the relation's order key (its declared columns first,
//!   then the rest), moved into that order in place: an own row's id is then its place in the order, so there is
//!   no order array, and no dead bits for its own rows (the duplicates are gone, not marked).
//! - **Hash indexes** on the declared keys. Each key's rows are grouped in canonical order, so a probe's answer
//!   comes in a fixed order too.
//! - **Dead bits and statistics.** The dead bits are a delta's kills of its base's rows (none in a full
//!   relation); the statistics are each column's minimum and maximum.
//!
//! **A delta** is a relation whose ids continue after its base's (`base_rows`). Its kills name base rows or its
//! own. `View` reads a base and its delta as one relation: liveness from both, the merged canonical order, and
//! probes of both indexes merged.

use crate::block::Block;
use crate::refs::Span;
use crate::rows::Rows;
use crate::{key_hash, structure};
use std::cmp::Ordering;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering::*};

pub const FROZEN: u32 = 1;
pub const DELTA: u32 = 2;
/// Its index directory and every index in it were added (`try_reserve_index`), each a chunk of the block's: given back
/// with the relation (`release`), or when forgotten (`forget_indexes`). A built relation's indexes lie with its rows.
pub const ADDED: u32 = 4;
/// Its rows' values change in place (a lattice's fold, decision 0384 step 5), so its indexes are forgotten after each
/// change and built again: each is a whole chunk of its class (`Block::try_alloc_chunk`), which the next index of its
/// size takes back from the free list. A fitted chunk (every other relation's, `try_alloc_chunk_fit`) goes back as
/// smaller pieces that a chunk of its size cannot take.
pub const REWRITTEN: u32 = 8;
/// A continuation of another relation (decision 0384 step 7): its ids below `base_rows` are its base's rows, which
/// lies in the same block at `base_off` from it (`try_continue`); every read of a row by its id goes there for them
/// (`rel_get`), so a reader binds it as one relation of `base_rows + own` rows.
pub const CONTINUES: u32 = 16;
/// Its rows are another relation's of the same block, where they lie (`try_link`): its row directories are that
/// relation's, so its release gives none of them back (that relation's holder does).
pub const ALIASED: u32 = 32;
/// A continuation whose base lies in another block (`try_link`, decision 0353 step 6): `base_off` is the base's
/// descriptor's offset from that block's start, which a reader binds beside its own (`rel_get`'s `far`).
pub const FAR_BASE: u32 = 64;
/// A key's columns at most: the device's probes' (decision 0325 step 5).
pub const MAX_KEY: usize = 8;
/// Indexes and sorted orders a relation can be given after it is made (`try_add_index`, `try_add_order`): its
/// directory's room. 16: a frozen relation's orders (up to one a column) beside the hash indexes probes build.
pub const ADDED_INDEXES: usize = 16;

structure! {
    pub struct Index {
        pub ncols: u32,
        /// 1 when the key is the order key's first columns: its rows are one range of the sorted rows, found by
        /// binary search, and the index holds no buckets or rows.
        pub ordered: u32,
        /// Its key's columns, `ncols` of them. An added index not yet built has `ncols` 0 (`find_index` skips it).
        pub key: [u16; 8],
        /// Buckets less one (a power of two less one).
        pub mask: u64,
        /// Bucket `b`'s rows are `rows[offs[b]..offs[b + 1]]`: every key whose hash falls in `b`, in canonical
        /// order. A reader checks the key (4 bytes a bucket, about one key a bucket; no hash kept). In a sorted order
        /// (`ORDER`), `rows` is every row in the key's order, and `offs` is empty.
        pub offs: Span<u32>,
        pub rows: Span<u32>,
        /// A sorted order's (`ORDER`, `ORDER_ROWS`; decision 0478 step 5b): the leading column's value at every
        /// `FENCE`th row of the order, each row's tag (an order tagged by its lead's hash), each row's key print (an
        /// order probed by whole keys). Empty when the order keeps none.
        pub fences: Span<i64>,
        pub tags: Span<u16>,
        pub prints: Span<u8>,
        /// The bytes of the chunks a sorted order's `rows`, `fences`, `tags` and `prints` keep, given back when it is
        /// forgotten; 0 for a span it does not own.
        pub held: [u64; 4],
        /// A hashed index's first row: it holds rows `from..from + rows.len()` (a recursive head's delta, its rows past
        /// those it held, read in place: decision 0353 step 6), 0 for one of every row.
        pub from: u32,
        /// `DEAD` once given back (`give_back_index`): its room is the block's again, and the next reservation takes
        /// its entry.
        pub state: u32,
    }
}

/// An index entry given back (`Relation::give_back_index`): no reader finds it, and a reservation takes it.
pub const DEAD: u32 = 1;

/// `Index::ordered`: a hashed index (buckets of rows).
pub const HASHED: u32 = 0;
/// `Index::ordered`: a sorted order whose rows are listed in `rows`, sorted by the key (decision 0478 step 5b).
pub const ORDER: u32 = 2;
/// `Index::ordered`: a sorted order of the rows `lo..hi` themselves, already in the key's order (`mask` holds
/// `lo << 32 | hi`), so no list of them is kept.
pub const ORDER_ROWS: u32 = 3;

/// A chunk of a relation's block an order is given (`Relation::try_add_order`): its offset, its items, and the bytes
/// its chunk keeps (0: one it does not own, given back by its owner).
#[derive(Clone, Copy, Debug, Default)]
pub struct Given {
    pub at: usize,
    pub len: usize,
    pub kept: usize,
}

/// An order's rows: listed, a chunk of the block (`ORDER`), or the rows `lo..hi` themselves, already in the key's
/// order (`ORDER_ROWS`).
#[derive(Clone, Copy, Debug)]
pub enum OrderRows {
    Listed(Given),
    Range(u32, u32),
}

/// An order given to a relation's directory (`Relation::try_add_order`): its rows, and its fences, tags and prints,
/// each a chunk of the block written already.
#[derive(Clone, Copy, Debug)]
pub struct GivenOrder {
    pub rows: OrderRows,
    pub fences: Given,
    pub tags: Given,
    pub prints: Given,
}

structure! {
    pub struct ColStats { pub min: i64, pub max: i64 }
}

structure! {
    pub struct Relation {
        pub arity: u32,
        pub flags: AtomicU32,
        /// A delta's base's row count: its own rows' ids start here.
        pub base_rows: u32,
        /// Live rows of its own, once frozen.
        pub live: AtomicU32,
        /// The order key: columns compared first, `nokey` of them, then the rest in order.
        pub okey: [u16; 8],
        pub nokey: u32,
        pub nindex: AtomicU32,
        pub rows: Rows,
        /// Row ids killed: a delta's of its base, or of its own.
        pub killed: Rows,
        pub dead: Span<u64>,
        pub indexes: Span<Index>,
        pub stats: Span<ColStats>,
        /// An open relation's set test (`insert_unique`): slots holding row id + 1.
        pub pending: Span<u32>,
        pub pend_used: AtomicU64,
        /// A continuation's base (`CONTINUES`): its descriptor's offset from this one's, in the same block; 0 for none.
        pub base_off: AtomicI64,
        /// A continuation's jump: an ancestor down its chain in the same block (Myers' skew-binary jumps), its
        /// descriptor's offset from this one's; 0 for none (a relation of its own, or a base in another block). A read
        /// of an id below `jump_rows` goes there at once, so a read down a chain of `n` takes O(log n) steps, and a
        /// chain needs no length bound (decision 0353 step 6: none of it is merged).
        pub jump_off: AtomicI64,
        /// The jump's `base_rows`: an id below them lies at or past the jump.
        pub jump_rows: u32,
        /// Its depth in its block's chain: 0 for a relation that continues none here (a full one, or a far base's link).
        pub depth: u32,
        /// Its row ranges known to be in an order already (decision 0180), written once when it is frozen into an
        /// input (decision 0478 step 5c), in row order; and the bytes of their chunk, given back at its release.
        pub known: Span<Known>,
        pub known_kept: AtomicU64,
        /// A frozen relation's dead rows (bit `id`; decision 0478 step 5d), given to it once and given back at its
        /// release; the bytes of their chunk. Its kills delta (`try_kills_at`) reads them here.
        pub own_dead: Span<u64>,
        pub own_dead_kept: AtomicU64,
    }
}

structure! {
    /// Rows `lo..hi` of a relation are in the order on `cols` (`ncols` of them; decision 0180): a writer wrote them
    /// sorted, so an order built later takes them in place.
    pub struct Known {
        pub lo: u32,
        pub hi: u32,
        pub ncols: u32,
        pub cols: [u16; MAX_KEY],
    }
}

/// An index's room in its relation's block (`Relation::try_reserve_index`): its number, its buckets' and rows'
/// offsets in the block, its mask, and the rows it covers.
#[derive(Clone, Copy, Debug)]
pub struct IndexRoom {
    pub index: usize,
    pub offs: usize,
    pub rows: usize,
    pub mask: u64,
    pub rows_n: u32,
    /// Its key's columns: `ncols` of `key`.
    pub key: [u16; MAX_KEY],
    pub ncols: usize,
    /// Its first row (`Index::from`).
    pub from: u32,
}

/// What a relation is: its columns' widths, and what freeze builds for its readers.
#[derive(Clone, Debug)]
pub struct Spec {
    pub widths: Vec<u32>,
    /// Columns compared first in the canonical order.
    pub order: Vec<u16>,
    /// The keys probes ask by.
    pub indexes: Vec<Vec<u16>>,
    /// The first chunk holds `2^min_log` rows.
    pub min_log: u32,
}

impl Relation {
    /// Makes an empty relation in `block`: a delta of `base` when given. Returns its descriptor.
    pub fn create<'b>(block: &'b Block, spec: &Spec, base: Option<&Relation>) -> &'b Relation {
        Relation::try_create(block, spec, base).unwrap_or_else(|| panic!("block full: a relation of {} columns", spec.widths.len()))
    }

    /// `create`, or `None` when the block cannot hold the relation's descriptor and directories (what it took of
    /// the block then stays unused).
    pub fn try_create<'b>(block: &'b Block, spec: &Spec, base: Option<&Relation>) -> Option<&'b Relation> {
        // The descriptor and both row sets' column directories, one chunk: given back whole (`release`).
        let at = block.try_alloc_chunk(Relation::descriptor_bytes(spec.widths.len()), 64)?;
        let p: *mut Relation = block.ptr(at);
        let cols: *mut crate::rows::ColDir = block.ptr(at + Relation::DIRS_AT);
        unsafe {
            (*p).arity = spec.widths.len() as u32;
            (*p).flags.store(if base.is_some() { DELTA } else { 0 }, Relaxed);
            (*p).base_rows = base.map_or(0, |b| {
                // A continuation's delta (its kills, which read no rows through it) counts its base's rows too.
                if b.continues() {
                    return b.base_rows + b.rows.len();
                }
                assert_eq!(b.base_rows, 0, "a delta's base is a full relation");
                b.rows.len()
            });
            assert!(spec.order.len() <= 8);
            for (i, &c) in spec.order.iter().enumerate() {
                (*p).okey[i] = c;
            }
            (*p).nokey = spec.order.len() as u32;
            Rows::init_in(&raw mut (*p).rows, cols, &spec.widths, spec.min_log);
            Rows::init_in(&raw mut (*p).killed, cols.add(spec.widths.len()), &[4], 6);
            Some(&*p)
        }
    }

    /// A relation that continues the first `base_rows` rows `base` reads (decision 0384 step 7): its ids below them
    /// are `base`'s, read there, and its own rows come after (a second relation started from rows whose writer
    /// another took, `Rows::shared`). `base` is a full relation, or a continuation itself (a fork of a fork, step 8),
    /// whose own rows it then reads past its base's: ids are read down the chain (`rel_get`). It lies in `base`'s
    /// block, which every reader binding it binds. `None` when the block has no room, or `base` is neither (a delta of
    /// kills), or `base_rows` is not within the rows `base` reads past its own base's.
    pub fn try_continue<'b>(block: &'b Block, spec: &Spec, base: &Relation, base_rows: u32) -> Option<&'b Relation> {
        let full = base.base_rows == 0 && !base.is_delta();
        let from = if base.continues() { base.base_rows } else { 0 };
        if !(full || base.continues()) || base_rows < from || base_rows > base.len_all() || !block.holds(std::ptr::from_ref(base)) {
            return None;
        }
        let rel = Relation::try_create(block, spec, None)?;
        // SAFETY: just made, this caller's alone.
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            (*me).base_rows = base_rows;
        }
        let off = std::ptr::from_ref(base) as i64 - std::ptr::from_ref(rel) as i64;
        rel.base_off.store(off, Release);
        // Its jump (Myers 1983): the base's jump's jump when the base's two jumps are as long, else the base. A base
        // with none (depth 0) jumps to itself.
        let jump = |r: &Relation| -> *const Relation {
            let o = r.jump_off.load(Relaxed);
            if o == 0 { r } else { std::ptr::from_ref(r).cast::<u8>().wrapping_offset(o as isize).cast() }
        };
        // SAFETY: a chain's jumps lie in its block, before it (each made with its base there).
        let (j, depth_of) = (jump(base), |p: *const Relation| unsafe { (*p).depth });
        let jj = unsafe { jump(&*j) };
        let to = if base.depth - depth_of(j) == depth_of(j) - depth_of(jj) && base.depth > 0 { jj } else { std::ptr::from_ref(base) };
        // SAFETY: just made, this caller's alone; `to` is a descriptor of the chain in this block.
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            (*me).depth = base.depth + 1;
            (*me).jump_rows = (*to).base_rows;
        }
        rel.jump_off.store(to as i64 - std::ptr::from_ref(rel) as i64, Release);
        rel.flags.fetch_or(DELTA | CONTINUES, Release);
        Some(rel)
    }

    /// A relation of `block` that reads a relation of several parts as one, nothing copied (decision 0353 step 6):
    /// rows `0..base_rows` its base's, read down the base's chain (`base`: the base and the block it lies in, this
    /// one or another, `FAR_BASE`; none for a first part, `base_rows` 0), then `rows_of`'s first `len` own rows, where
    /// they lie in `block` (`ALIASED`). `None` when `block` cannot hold its descriptor, or `rows_of` is not `block`'s.
    pub fn try_link<'b>(block: &'b Block, spec: &Spec, base: Option<(&Block, &Relation)>, base_rows: u32, rows_of: &Relation, len: u32) -> Option<&'b Relation> {
        if !block.holds(std::ptr::from_ref(rows_of)) || len > rows_of.rows.len() || rows_of.rows.ncols as usize != spec.widths.len() {
            return None;
        }
        let rel = match base {
            None if base_rows == 0 => Relation::try_create(block, spec, None)?,
            None => return None,
            Some((home, b)) if std::ptr::eq(home, block) => Relation::try_continue(block, spec, b, base_rows)?,
            Some((home, b)) => {
                let full = b.base_rows == 0 && !b.is_delta();
                let at = home.offset_of(std::ptr::from_ref(b));
                if !(full || b.continues()) || base_rows > b.len_all() || !home.holds(std::ptr::from_ref(b)) || at == 0 {
                    return None;
                }
                let rel = Relation::try_create(block, spec, None)?;
                // SAFETY: just made, this caller's alone.
                unsafe { (*std::ptr::from_ref(rel).cast_mut()).base_rows = base_rows };
                rel.base_off.store(at as i64, Release);
                rel.flags.fetch_or(DELTA | CONTINUES | FAR_BASE, Release);
                rel
            }
        };
        // Its rows: `rows_of`'s directories, read where they lie; none of its own (its own directories, made with
        // its descriptor, stay unused and go back with it).
        // SAFETY: just made, this caller's alone; `rows_of` is frozen at `len` rows for as long as this lives (its
        // holder keeps it).
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            let (src, dst) = (&rows_of.rows, &mut (*me).rows);
            dst.min_log = src.min_log;
            dst.ncols = src.ncols;
            dst.nchunks.store(src.nchunks.load(Acquire), Relaxed);
            dst.reserved.store(len, Relaxed);
            dst.len.store(len, Release);
            Span::set(&dst.cols, src.cols.as_ptr(), src.cols.len());
        }
        rel.flags.fetch_or(ALIASED | FROZEN, Release);
        Some(rel)
    }

    /// Whether its base lies in another block (`try_link`).
    #[inline]
    pub fn far_base(&self) -> bool {
        self.flags.load(Relaxed) & FAR_BASE != 0
    }

    /// Whether it continues another relation's rows (`try_continue`).
    #[inline]
    pub fn continues(&self) -> bool {
        self.flags.load(Relaxed) & CONTINUES != 0
    }

    /// The relation it continues (`try_continue`).
    pub fn base(&self) -> Option<&Relation> {
        let off = self.base_off.load(Acquire);
        // SAFETY: a continuation's base lies in its block at `off`, kept while it is (its holder's). A base in another
        // block is read through that block (`far_base`).
        (self.continues() && off != 0 && !self.far_base()).then(|| unsafe { &*(std::ptr::from_ref(self).cast::<u8>().offset(off as isize).cast::<Relation>()) })
    }

    /// Rows it reads by id: its own, and a continuation's base's first `base_rows` before them.
    pub fn len_all(&self) -> u32 {
        if self.continues() { self.base_rows + self.rows.len() } else { self.rows.len() }
    }

    /// Row `id`'s value in column `c` (the CLIF's `rel_get`, the kernels' and the JIT's): a continuation's id below
    /// `base_rows` read in its base, down the chain of bases (a fork of a fork) to the one whose own rows hold it;
    /// its own rows from `base_rows` on (a delta's own ids count from its base's too).
    #[inline]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        self.get_far(None, id, c)
    }

    /// `get`, a base in another block (`FAR_BASE`) read in `far`, the block it lies in (`rel_get`'s `far`).
    #[inline]
    pub fn get_far(&self, far: Option<&Block>, id: u32, c: usize) -> i64 {
        let mut rel = self;
        loop {
            let off = rel.base_off.load(Relaxed);
            if id >= rel.base_rows || off == 0 {
                break;
            }
            let jump = rel.jump_off.load(Relaxed);
            rel = if jump != 0 && id < rel.jump_rows {
                // SAFETY: its jump lies in its block `jump` bytes from it, an ancestor its holder keeps.
                unsafe { &*std::ptr::from_ref(rel).cast::<u8>().offset(jump as isize).cast::<Relation>() }
            } else if rel.far_base() {
                let far = far.expect("a relation whose base lies in another block, read with that block");
                // SAFETY: its base's descriptor lies `off` bytes into `far`, kept while it is (its holder's).
                unsafe { &*far.ptr::<Relation>(off as usize) }
            } else {
                // SAFETY: a continuation's base lies in its block `off` bytes from it, kept while it is (its holder's).
                unsafe { &*std::ptr::from_ref(rel).cast::<u8>().offset(off as isize).cast::<Relation>() }
            };
        }
        rel.rows.get(id.wrapping_sub(rel.base_rows), c)
    }

    /// A delta of `base` with no rows of its own whose dead bits are `words` (row `id` dead when bit `id` of them is
    /// set): a relation's kills of the rows it shares, read beside them where they lie (decision 0325: the device
    /// skips a dead row by its bit, `rel_dead`). `None` when the block cannot hold it.
    /// `try_kills`, its dead bits the ones `base` keeps (`own_dead`), read where they lie: nothing copied.
    pub fn try_kills_own<'b>(block: &'b Block, spec: &Spec, base: &Relation) -> Option<&'b Relation> {
        let words = base.own_dead();
        assert!(!words.is_empty(), "a relation's own dead rows");
        let rel = Relation::try_create(block, spec, Some(base))?;
        // SAFETY: `words` lie in `block` (`base`'s), which `base` keeps while its delta lives (the delta's holder keeps
        // `base`).
        unsafe { Span::set(&rel.dead, words.as_ptr(), words.len()) };
        Some(rel)
    }

    /// Its dead bits are a chunk of their own, so a delta replaced when they change goes back whole
    /// (`release_kills`).
    pub fn try_kills<'b>(block: &'b Block, spec: &Spec, base: &Relation, words: &[u64]) -> Option<&'b Relation> {
        let rel = Relation::try_create(block, spec, Some(base))?;
        let Some(at) = block.try_alloc_chunk(Relation::kills_bytes(words.len()), 64) else {
            // SAFETY: just made in `block`; no one has it.
            unsafe { rel.release(block) };
            return None;
        };
        let w: *mut u64 = block.ptr(at);
        // SAFETY: `w` holds `words.len()` words, just allocated in `block`, as `rel` is.
        unsafe {
            std::ptr::copy_nonoverlapping(words.as_ptr(), w, words.len());
            Span::set(&rel.dead, w, words.len());
        }
        Some(rel)
    }

    /// The chunk a kills delta's `n` dead words take (`try_kills`).
    fn kills_bytes(n: usize) -> usize {
        n.max(1) * size_of::<u64>()
    }

    /// Gives a kills delta (`try_kills`) back to the block's free list: its dead bits' chunk, then itself
    /// (`release`). Returns the bytes given to the OS with them.
    ///
    /// # Safety
    /// It is a delta `try_kills` made in `block`, and no one reads it after (its last holder's).
    pub unsafe fn release_kills(&self, block: &Block) -> usize {
        let dead = self.dead.as_slice();
        let given = block.free_chunk(block.offset_of(dead.as_ptr()), Relation::kills_bytes(dead.len()));
        given + unsafe { self.release(block) }
    }

    /// Gives its chunks back to the block's free list (decision 0342), when the block keeps one. Returns the bytes
    /// given to the OS with them (`Block::free_chunk`).
    ///
    /// # Safety
    /// It lies in `block`, and no one reads or writes it after (its last holder's).
    pub unsafe fn release(&self, block: &Block) -> usize {
        // Aliased rows are their own relation's, given back with it.
        let own_rows = if self.flags.load(Acquire) & ALIASED != 0 { 0 } else { unsafe { self.rows.release(block) } };
        let given = own_rows + unsafe { self.killed.release(block) };
        let kept = self.known_kept.load(Acquire) as usize;
        if kept > 0 {
            block.free_chunk_fit(block.offset_of(self.known.as_ptr()), kept);
        }
        let kept = self.own_dead_kept.load(Acquire) as usize;
        if kept > 0 {
            block.free_chunk_fit(block.offset_of(self.own_dead.as_ptr()), kept);
        }
        // Its added indexes, and their directory: chunks of the block's.
        if self.flags.load(Acquire) & ADDED != 0 {
            // SAFETY: the caller's: this is the relation's last use.
            unsafe { self.forget_indexes(block) };
            block.free_chunk_fit_resident(block.offset_of(self.indexes.as_ptr()), block.fit_kept(ADDED_INDEXES * size_of::<Index>()));
        }
        let (at, bytes) = (block.offset_of(std::ptr::from_ref(self)), Relation::descriptor_bytes(self.arity as usize));
        // The last use of `self`: its own memory goes back.
        given + block.free_chunk(at, bytes)
    }

    /// Where the column directories lie in a descriptor's chunk.
    const DIRS_AT: usize = size_of::<Relation>().next_multiple_of(64);

    /// A descriptor's chunk: the relation, and the directories of its `arity` columns and its killed rows' one.
    pub fn descriptor_bytes(arity: usize) -> usize {
        Relation::DIRS_AT + (arity + 1) * size_of::<crate::rows::ColDir>()
    }

    #[inline]
    pub fn is_frozen(&self) -> bool {
        self.flags.load(Acquire) & FROZEN != 0
    }

    #[inline]
    pub fn is_delta(&self) -> bool {
        self.flags.load(Relaxed) & DELTA != 0
    }

    /// Its own rows' id range: `base_rows ..`.
    pub fn ids(&self) -> std::ops::Range<u32> {
        self.base_rows..self.base_rows + self.rows.len()
    }

    #[inline]
    pub fn own(&self, id: u32, c: usize) -> i64 {
        self.rows.get(id - self.base_rows, c)
    }

    /// Whether the dead bits mark row `id` (the CLIF's `rel_dead`, decision 0325): none past the bits.
    #[inline]
    pub fn dead_bit(&self, id: u32) -> bool {
        self.dead.as_slice().get((id >> 6) as usize).is_some_and(|w| w >> (id & 63) & 1 != 0)
    }

    #[inline]
    pub fn index_at(&self, i: usize) -> &Index {
        &self.indexes.as_slice()[i]
    }

    /// One relation's rows for a key of hash `h` on index `i`, before liveness and key equality: its bucket (the
    /// CLIF's `rel_group`, decision 0325). None for an index of no buckets.
    #[inline]
    pub fn group(&self, i: usize, h: u64) -> &[u32] {
        let ix = self.index_at(i);
        let offs = ix.offs.as_slice();
        if offs.is_empty() {
            return &[];
        }
        let b = (h & ix.mask) as usize;
        &ix.rows.as_slice()[offs[b] as usize..offs[b + 1] as usize]
    }

    /// The index whose key is `cols` over its rows `0..rows` (a hashed one: an ordered index holds no buckets), built.
    /// An index of fewer rows (built before the relation grew) is not it.
    pub fn find_index(&self, cols: &[u16], rows: u32) -> Option<usize> {
        self.find_index_from(cols, 0, rows)
    }

    /// `find_index` of its rows `from..from + rows` (a recursive head's delta read in place).
    pub fn find_index_from(&self, cols: &[u16], from: u32, rows: u32) -> Option<usize> {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: an entry's `ncols` is written once, last (`index_built`); read as written, then the index after it.
        let built = |i: usize| unsafe { std::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| ix[i].ordered == 0 && built(i) == cols.len() && ix[i].key[..cols.len()] == *cols && ix[i].from == from && ix[i].rows.len() == rows as usize);
        std::sync::atomic::fence(Acquire);
        found
    }

    /// Its dead rows' bits as given (`adopt_dead`): empty when none were.
    pub fn own_dead(&self) -> &[u64] {
        if self.own_dead_kept.load(Acquire) == 0 {
            return &[];
        }
        self.own_dead.as_slice()
    }

    /// Takes its dead rows' bits, `len` words in the chunk at `at` of `block` (`kept` bytes), written: kept from now on,
    /// given back at its release (decision 0478 step 5d). False when it has some already.
    ///
    /// # Safety
    /// It lies in `block`; the chunk is the caller's to give and nothing writes it after; nothing else writes its
    /// descriptor meanwhile.
    pub unsafe fn adopt_dead(&self, block: &Block, at: usize, len: usize, kept: usize) -> bool {
        if self.own_dead_kept.load(Acquire) != 0 || len == 0 || kept == 0 {
            return false;
        }
        // SAFETY: the caller's.
        unsafe {
            let me = std::ptr::from_ref(self).cast_mut();
            Span::set(&raw mut (*me).own_dead, block.ptr::<u64>(at), len);
        }
        self.own_dead_kept.store(kept as u64, Release);
        true
    }

    /// Its row ranges known to be in an order (`Known`): none until `try_set_known`.
    pub fn known(&self) -> &[Known] {
        if self.known_kept.load(Acquire) == 0 {
            return &[];
        }
        self.known.as_slice()
    }

    /// Writes its known ranges (decision 0478 step 5c), once: `(lo, hi, cols)` each, in row order. False when they
    /// were written already, a key is past `MAX_KEY`, or `block` has no room.
    ///
    /// # Safety
    /// It lies in `block`, and nothing else writes its descriptor meanwhile (its writer's, at freeze).
    pub unsafe fn try_set_known(&self, block: &Block, ranges: &[(u32, u32, &[u16])]) -> bool {
        if ranges.is_empty() {
            return true;
        }
        if self.known_kept.load(Acquire) != 0 || ranges.iter().any(|(_, _, c)| c.len() > MAX_KEY) {
            return false;
        }
        let Some((at, kept)) = block.try_alloc_chunk_fit(ranges.len() * size_of::<Known>(), 8) else { return false };
        let p = block.ptr::<Known>(at);
        // SAFETY: `p` holds `ranges.len()` entries, this relation's alone; the caller's for its descriptor.
        unsafe {
            for (i, &(lo, hi, cols)) in ranges.iter().enumerate() {
                let mut c = [0u16; MAX_KEY];
                c[..cols.len()].copy_from_slice(cols);
                p.add(i).write(Known { lo, hi, ncols: cols.len() as u32, cols: c });
            }
            let me = std::ptr::from_ref(self).cast_mut();
            Span::set(&raw mut (*me).known, p, ranges.len());
        }
        self.known_kept.store(kept as u64, Release);
        true
    }

    /// The sorted order on `cols` over `rows` rows (decision 0478 step 5b), built: its number.
    pub fn find_order(&self, cols: &[u16], rows: u32) -> Option<usize> {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: as `find_index`'s.
        let built = |i: usize| unsafe { std::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| (ix[i].ordered == ORDER || ix[i].ordered == ORDER_ROWS) && built(i) == cols.len() && ix[i].key[..cols.len()] == *cols && Self::order_rows(&ix[i]) == rows as usize);
        std::sync::atomic::fence(Acquire);
        found
    }

    /// The sorted orders built, by number: every one `find_order` can find.
    pub fn orders(&self) -> impl Iterator<Item = usize> + '_ {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: as `find_index`'s.
        let built = move |i: usize| unsafe { std::ptr::read_volatile(&raw const ix[i].ncols) } != 0;
        let found: Vec<usize> = (0..n).filter(|&i| (ix[i].ordered == ORDER || ix[i].ordered == ORDER_ROWS) && built(i)).collect();
        std::sync::atomic::fence(Acquire);
        found.into_iter()
    }

    /// The rows a sorted order holds.
    pub fn order_rows(ix: &Index) -> usize {
        if ix.ordered == ORDER_ROWS {
            ((ix.mask & 0xffff_ffff) - (ix.mask >> 32)) as usize
        } else {
            ix.rows.len()
        }
    }

    /// The number of this relation's next directory entry, its directory made first where it has none
    /// (`ADDED_INDEXES` entries, a chunk of `block`); `None` when the directory is full or `block` has no room.
    ///
    /// # Safety
    /// As `try_reserve_index`'s: this relation's descriptor is the caller's to change.
    unsafe fn next_entry(&self, block: &Block) -> Option<usize> {
        if self.indexes.is_empty() {
            let (at, _) = block.try_alloc_chunk_fit_zeroed(ADDED_INDEXES * size_of::<Index>(), 8)?;
            let me = std::ptr::from_ref(self).cast_mut();
            // SAFETY: the caller's.
            unsafe { Span::set(&raw mut (*me).indexes, block.ptr::<Index>(at), ADDED_INDEXES) };
            self.flags.fetch_or(ADDED, Release);
        }
        let n = (self.nindex.load(Relaxed) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // An entry given back is taken first (`give_back_index`) when its room is the block's already; one that keeps
        // its room (a block of no free list) is left for an index that fits it (`dead_room`), and taken last.
        let dead = |i: usize, room: bool| ix[i].state == DEAD && (ix[i].offs.len() != 0) == room;
        // Its taker marks it live (`state` 0) once its room is had.
        (0..n).find(|&i| dead(i, false)).or((n < ix.len()).then_some(n)).or_else(|| (0..n).find(|&i| dead(i, true)))
    }

    /// A dead entry whose room (kept: its block has no free list) holds `buckets` buckets and `n` rows: its number.
    fn dead_room(&self, buckets: usize, n: usize) -> Option<usize> {
        let ix = self.indexes.as_slice();
        let m = (self.nindex.load(Relaxed) as usize).min(ix.len());
        (0..m).find(|&i| ix[i].state == DEAD && ix[i].offs.len() >= buckets && ix[i].rows.len() >= n.max(1))
    }

    /// Gives back index `room`, reserved and not marked built (`index_built`), or built and no longer read: its buckets
    /// and rows go back to the block's free list, and its entry to the directory, for the next reservation. In a block
    /// of no free list (whose chunks given back are never taken again) the entry keeps them, for the next index that
    /// fits them (`dead_room`). For an index a batch built that another built meanwhile (both found none, each in its
    /// own batch), and one whose builder failed.
    ///
    /// # Safety
    /// `room` is this relation's, in `block`; nothing reads it now or after (its builder is done), and no other thread
    /// adds an index to this relation meanwhile (its owner's lock).
    pub unsafe fn give_back_index(&self, block: &Block, room: &IndexRoom) {
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        // SAFETY: the caller's; `find_index` skips an entry whose `ncols` is 0.
        unsafe {
            std::ptr::write_volatile(&raw mut (*ix).ncols, 0);
            assert!((*ix).ordered == 0 && (*ix).state != DEAD, "a hashed index given back once");
            (*ix).key = [0; MAX_KEY];
            if !block.has_free_list() {
                (*ix).state = DEAD;
                return;
            }
            let (offs, rows) = (4 * (*ix).offs.len(), 4 * (*ix).rows.len().max(1));
            let (o, r) = (block.offset_of((*ix).offs.as_ptr()), block.offset_of((*ix).rows.as_ptr()));
            if self.flags.load(Acquire) & REWRITTEN != 0 {
                block.free_chunk_resident(o, offs);
                block.free_chunk_resident(r, rows);
            } else {
                block.free_chunk_fit_resident(o, block.fit_kept(offs));
                block.free_chunk_fit_resident(r, block.fit_kept(rows));
            }
            for s in [&(*ix).offs, &(*ix).rows] {
                s.off.store(0, Relaxed);
                s.len.store(0, Relaxed);
            }
            (*ix).key = [0; MAX_KEY];
            (*ix).state = DEAD;
        }
    }

    /// Gives this relation a sorted order on `cols` (decision 0478 step 5b): `order`'s rows, and its fences, tags
    /// and prints, each a chunk of `block` written already. The relation keeps them from now on and gives them back
    /// when its indexes are forgotten. Its number; `None` when its directory has no room (the chunks are then still
    /// the caller's).
    ///
    /// # Safety
    /// As `try_reserve_index`'s; each chunk lies in `block` and is the caller's to give, written, and nothing writes
    /// it after.
    pub unsafe fn try_add_order(&self, block: &Block, cols: &[u16], order: GivenOrder) -> Option<usize> {
        assert!(!cols.is_empty() && cols.len() <= MAX_KEY, "a key of {} columns", cols.len());
        let GivenOrder { rows, fences, tags, prints } = order;
        // SAFETY: the caller's: this relation's descriptor is ours to change.
        unsafe {
            let i = self.next_entry(block)?;
            let ix = std::ptr::from_ref(&self.indexes.as_slice()[i]).cast_mut();
            (*ix).ncols = 0;
            (*ix).from = 0;
            (*ix).state = 0;
            let mut key = [0u16; MAX_KEY];
            key[..cols.len()].copy_from_slice(cols);
            (*ix).key = key;
            /// An empty span, as a zeroed directory holds it (its target its own place, never address 0).
            fn clear<T>(dst: *mut Span<T>) {
                // SAFETY: the caller's: `dst` is the entry's.
                unsafe {
                    (*dst).off.store(0, Relaxed);
                    (*dst).len.store(0, Relaxed);
                }
            }
            clear(&raw mut (*ix).offs);
            let ids = match rows {
                OrderRows::Range(lo, hi) => {
                    (*ix).ordered = ORDER_ROWS;
                    (*ix).mask = (u64::from(lo) << 32) | u64::from(hi);
                    clear(&raw mut (*ix).rows);
                    Given::default()
                }
                OrderRows::Listed(ids) => {
                    (*ix).ordered = ORDER;
                    (*ix).mask = 0;
                    Span::set(&raw mut (*ix).rows, block.ptr::<u32>(ids.at), ids.len);
                    ids
                }
            };
            fn set<T>(block: &Block, dst: *mut Span<T>, g: Given) {
                if g.len == 0 {
                    clear(dst);
                    return;
                }
                // SAFETY: the caller's: `dst` is the entry's, `g` a chunk of `block`.
                unsafe { Span::set(dst, block.ptr::<T>(g.at), g.len) }
            }
            set(block, &raw mut (*ix).fences, fences);
            set(block, &raw mut (*ix).tags, tags);
            set(block, &raw mut (*ix).prints, prints);
            (*ix).held = [ids.kept as u64, fences.kept as u64, tags.kept as u64, prints.kept as u64];
            self.nindex.fetch_max(i as u32 + 1, Release);
            std::sync::atomic::fence(Release);
            std::ptr::write_volatile(&raw mut (*ix).ncols, cols.len() as u32);
            Some(i)
        }
    }

    /// Room for an index on `cols` of rows `0..n` in `block`, which holds this relation: its directory's entry, its
    /// `2^log` buckets (`n`'s power of two at least) and its rows, which a builder fills (`build_index`, or the
    /// device's), then marks built (`index_built`). Its number, its buckets' and rows' offsets in the block, and its
    /// mask; `None` when the block or the directory has no room.
    ///
    /// # Safety
    /// No other thread adds an index to this relation meanwhile (its owner's lock), and its rows `0..n` are written.
    pub unsafe fn try_reserve_index(&self, block: &Block, cols: &[u16], n: u32) -> Option<IndexRoom> {
        // SAFETY: the caller's.
        unsafe { self.try_reserve_index_buckets(block, cols, n, n as usize) }
    }

    /// `try_reserve_index` of its rows `from..from + n` (`Index::from`): a recursive head's delta read in place.
    ///
    /// # Safety
    /// As `try_reserve_index`'s, its rows `from..from + n` written.
    pub unsafe fn try_reserve_index_from(&self, block: &Block, cols: &[u16], from: u32, n: u32) -> Option<IndexRoom> {
        // SAFETY: the caller's.
        let mut room = unsafe { self.try_reserve_index_buckets(block, cols, n, n as usize) }?;
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        // SAFETY: the entry is the caller's, not yet built.
        unsafe { (*ix).from = from };
        room.from = from;
        Some(room)
    }

    /// `try_reserve_index` with room for `keys` distinct keys: `keys`' power of two buckets (at most `n`'s), about
    /// one key a bucket. A key of few values (a kind) is a few buckets, not one a row: its index is about 4 bytes a
    /// row and 4 a key.
    ///
    /// # Safety
    /// As `try_reserve_index`'s.
    pub unsafe fn try_reserve_index_buckets(&self, block: &Block, cols: &[u16], n: u32, keys: usize) -> Option<IndexRoom> {
        assert!(!cols.is_empty() && cols.len() <= MAX_KEY, "a key of {} columns", cols.len());
        // SAFETY: the caller's: this relation's descriptor is ours to change.
        unsafe {
            let cap = keys.min(n as usize).next_power_of_two().max(1);
            // Chunks the block's free list takes back when the index is forgotten (`forget_indexes`), zeroed as an
            // allocation is.
            let (ob, rb) = (4 * (cap + 1), 4 * (n as usize).max(1));
            // In a block of no free list, an index given back keeps its room for one that fits it (`give_back_index`).
            let reused = if block.has_free_list() || self.indexes.is_empty() { None } else { self.dead_room(cap + 1, n as usize) };
            let i = match reused {
                Some(i) => i,
                None => self.next_entry(block)?,
            };
            let (offs_at, rows_at) = if let Some(d) = reused {
                let d = &self.indexes.as_slice()[d];
                let (o, r) = (block.offset_of(d.offs.as_ptr()), block.offset_of(d.rows.as_ptr()));
                // Zeroed, as a new chunk is (the builders count into the buckets).
                block.ptr::<u8>(o).write_bytes(0, ob);
                block.ptr::<u8>(r).write_bytes(0, rb);
                (o, r)
            } else if self.flags.load(Acquire) & REWRITTEN != 0 {
                let o = block.try_alloc_chunk(ob, 8)?;
                let Some(r) = block.try_alloc_chunk(rb, 8) else {
                    block.free_chunk(o, ob);
                    return None;
                };
                (o, r)
            } else {
                let (o, _) = block.try_alloc_chunk_fit_zeroed(ob, 8)?;
                let Some((r, _)) = block.try_alloc_chunk_fit_zeroed(rb, 8) else {
                    block.free_chunk_fit(o, block.fit_kept(ob));
                    return None;
                };
                (o, r)
            };
            let (offs, rows): (*mut u32, *mut u32) = (block.ptr(offs_at), block.ptr(rows_at));
            let ix = std::ptr::from_ref(&self.indexes.as_slice()[i]).cast_mut();
            (*ix).ncols = 0;
            (*ix).ordered = 0;
            (*ix).from = 0;
            (*ix).held = [0; 4];
            (*ix).state = 0;
            let mut key = [0u16; MAX_KEY];
            key[..cols.len()].copy_from_slice(cols);
            (*ix).key = key;
            (*ix).mask = (cap - 1) as u64;
            Span::set(&raw mut (*ix).offs, offs, cap + 1);
            Span::set(&raw mut (*ix).rows, rows, n as usize);
            self.nindex.fetch_max(i as u32 + 1, Release);
            Some(IndexRoom { index: i, offs: block.offset_of(offs), rows: block.offset_of(rows), mask: (cap - 1) as u64, rows_n: n, key: (*ix).key, ncols: cols.len(), from: 0 })
        }
    }

    /// Fills index `room` on the host: each row's bucket by its key's hash, the buckets' starts, then the rows in id
    /// order within each bucket, as the device's builder places them.
    ///
    /// # Safety
    /// `room` is this relation's, reserved and not yet built, and nothing reads it meanwhile.
    pub unsafe fn build_index(&self, room: &IndexRoom) {
        let key = &room.key[..room.ncols];
        // A continuation's values are of every row it reads (its base's first); a delta's its own rows'.
        let value = |i: u32, c: u16| if self.continues() { self.get(i, c as usize) } else { self.rows.get(i, c as usize) };
        // SAFETY: the caller's.
        unsafe { self.build_index_hashed(room, &|i| key.iter().fold(crate::SEED, |h, &c| crate::mix(h, value(i, c)))) }
    }

    /// `build_index`, each row's key hash given (`hash(i)` for its row `i`: `SEED` mixed with its key's values, as
    /// `key_hash`): for a relation whose key values are read elsewhere (a view's home of no columns, decision 0478
    /// step 5e).
    ///
    /// # Safety
    /// As `build_index`'s.
    pub unsafe fn build_index_hashed(&self, room: &IndexRoom, hash: &dyn Fn(u32) -> u64) {
        let ix = &self.indexes.as_slice()[room.index];
        let n = room.rows_n as usize;
        // SAFETY: the caller's: the room's spans are ours to write.
        let (offs, rows) = unsafe {
            (std::slice::from_raw_parts_mut(ix.offs.as_slice().as_ptr().cast_mut(), ix.offs.len()),
             std::slice::from_raw_parts_mut(ix.rows.as_slice().as_ptr().cast_mut(), ix.rows.len()))
        };
        offs.fill(0);
        // A continuation's index is of every row it reads (its base's first, ids from 0); a delta's of its own.
        let base = if self.continues() { 0 } else { self.base_rows };
        let bucket: Vec<u32> = (0..n as u32).map(|i| (hash(i) & room.mask) as u32).collect();
        for &b in &bucket {
            offs[b as usize + 1] += 1;
        }
        for b in 0..offs.len() - 1 {
            offs[b + 1] += offs[b];
        }
        let mut at: Vec<u32> = offs[..offs.len() - 1].to_vec();
        for (i, &b) in bucket.iter().enumerate() {
            rows[at[b as usize] as usize] = base + i as u32;
            at[b as usize] += 1;
        }
    }

    /// Index `room` built (by the host or the device): `find_index` finds it from now on.
    ///
    /// # Safety
    /// Its builder has finished.
    pub unsafe fn index_built(&self, room: &IndexRoom) {
        let ix = std::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        std::sync::atomic::fence(Release);
        // SAFETY: the caller's; `ncols` is a u32 written once, after the index (the fence above).
        unsafe { std::ptr::write_volatile(&raw mut (*ix).ncols, room.ncols as u32) };
    }

    /// Marks its rows' values as changed in place (`REWRITTEN`): before any index is added.
    pub fn rewritten(&self) {
        debug_assert!(self.indexes.is_empty(), "marked rewritten after an index was added");
        self.flags.fetch_or(REWRITTEN, Release);
    }

    /// Every index added forgotten, its buckets and rows given back to `block`'s free list (`try_reserve_index`'s
    /// chunks): for a relation whose rows' values change in place (a lattice's fold, decision 0384 step 5), whose
    /// indexes built before no longer hold. The next reader that probes a key builds its index again, in room the
    /// block takes from its free list.
    ///
    /// # Safety
    /// `block` holds this relation, whose indexes were all added (`try_reserve_index`, none of a built relation's);
    /// under its owner's index lock, and no reader probes an index of it meanwhile, or after.
    pub unsafe fn forget_indexes(&self, block: &Block) {
        assert!(self.flags.load(Acquire) & ADDED != 0 || self.indexes.is_empty(), "a built relation's indexes forgotten");
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        for ix in &self.indexes.as_slice()[..n] {
            // SAFETY: the caller's; `find_index` skips an entry whose `ncols` is 0.
            unsafe { std::ptr::write_volatile(&raw mut (*std::ptr::from_ref(ix).cast_mut()).ncols, 0) };
            if ix.state == DEAD {
                // Given back already (`give_back_index`): its room is the block's.
                // SAFETY: the caller's.
                unsafe { (*std::ptr::from_ref(ix).cast_mut()).state = 0 };
                continue;
            }
            if ix.ordered == ORDER || ix.ordered == ORDER_ROWS {
                // A sorted order's chunks, each as it was given (`try_add_order`).
                let spans = [(ix.rows.as_ptr().cast::<u8>(), ix.held[0]), (ix.fences.as_ptr().cast(), ix.held[1]), (ix.tags.as_ptr().cast(), ix.held[2]), (ix.prints.as_ptr().cast(), ix.held[3])];
                for (p, kept) in spans {
                    if kept > 0 {
                        block.free_chunk_fit(block.offset_of(p), kept as usize);
                    }
                }
                continue;
            }
            let (offs, rows) = (4 * ix.offs.len(), 4 * ix.rows.len().max(1));
            let (o, r) = (block.offset_of(ix.offs.as_ptr()), block.offset_of(ix.rows.as_ptr()));
            // Left resident, as an index's room was before it went back (a few bytes a row; the next index takes it).
            if self.flags.load(Acquire) & REWRITTEN != 0 {
                block.free_chunk_resident(o, offs);
                block.free_chunk_resident(r, rows);
            } else {
                block.free_chunk_fit_resident(o, block.fit_kept(offs));
                block.free_chunk_fit_resident(r, block.fit_kept(rows));
            }
        }
        self.nindex.store(0, Release);
    }

    /// `held_bytes` by part: values written, chunk room unwritten, order, index buckets, index rows, dead bits,
    /// descriptors.
    pub fn held_parts(&self) -> [usize; 7] {
        let cap = crate::rows::capacity(self.rows.nchunks.load(Relaxed), self.rows.min_log) as usize;
        let width: usize = (0..self.rows.ncols as usize).map(|c| self.rows.col(c).width as usize).sum();
        let n = self.rows.len() as usize;
        let ix = self.indexes.as_slice();
        [
            n * width,
            (cap - n) * width,
            0,
            ix.iter().map(|i| i.offs.len() * 4).sum(),
            ix.iter().map(|i| i.rows.len() * 4).sum(),
            self.dead.len() * 8,
            size_of::<Relation>() + self.rows.ncols as usize * size_of::<crate::rows::ColDir>() + size_of::<crate::rows::ColDir>(),
        ]
    }

    /// Bytes its block holds for it: the chunks made (written or not), the order, the indexes and the bits.
    pub fn held_bytes(&self) -> usize {
        let cap = crate::rows::capacity(self.rows.nchunks.load(Relaxed), self.rows.min_log) as usize;
        let width: usize = (0..self.rows.ncols as usize).map(|c| self.rows.col(c).width as usize).sum();
        let ix: usize =
            self.indexes.as_slice().iter().map(|i| i.offs.len() * 4 + i.rows.len() * 4).sum();
        cap * width + self.dead.len() * 8 + ix + size_of::<Relation>()
    }

    /// Bytes this relation's values, order, indexes and bits take (what a reader may touch: a continuation's chain's
    /// rows too, decision 0384 step 7).
    pub fn bytes(&self) -> usize {
        let ix: usize =
            self.indexes.as_slice().iter().map(|i| i.offs.len() * 4 + i.rows.len() * 4).sum();
        self.rows.value_bytes() + self.chain_value_bytes() + self.dead.len() * 8 + ix
    }

    /// A continuation's chain's values it reads (0 for one that continues none): what a copy of it holds past its own.
    pub fn chain_value_bytes(&self) -> usize {
        if !self.continues() {
            return 0;
        }
        let width: usize = (0..self.rows.ncols as usize).map(|c| self.rows.col(c).width as usize).sum();
        self.base_rows as usize * width
    }
}

/// A writer of one open relation: claims rows, writes them, kills rows. `Send`: one per thread or morsel.
pub struct RelW<'b> {
    pub block: &'b Block,
    pub rel: &'b Relation,
    pub lock: &'b Mutex<()>,
}

impl<'b> RelW<'b> {
    /// One row. Returns its id.
    pub fn push(&self, vals: &[i64]) -> u32 {
        debug_assert_eq!(vals.len(), self.rel.arity as usize);
        assert!(!self.rel.is_frozen(), "push into a frozen relation");
        let s = self.rel.rows.claim(self.block, self.lock, 1);
        for (c, &v) in vals.iter().enumerate() {
            unsafe { self.rel.rows.put(s, c, v) };
        }
        s + self.rel.base_rows
    }

    /// Rows, flat: one claim for all of them.
    pub fn extend(&self, flat: &[i64]) {
        let a = self.rel.arity as usize;
        let n = flat.len() / a;
        if n == 0 {
            return;
        }
        let s = self.rel.rows.claim(self.block, self.lock, n as u32);
        for (r, row) in flat.chunks_exact(a).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                unsafe { self.rel.rows.put(s + r as u32, c, v) };
            }
        }
    }

    /// Kills a row by id: a base row (in a delta) or one of its own.
    pub fn kill(&self, id: u32) {
        let s = self.rel.killed.claim(self.block, self.lock, 1);
        unsafe { self.rel.killed.put(s, 0, id as i64) };
    }

    /// Adds `vals` unless this relation already holds it: one writer only (a fixpoint's own relation). Returns
    /// whether it was new. The set test is an open-addressed table in the block, grown by doubling.
    pub fn insert_unique(&self, vals: &[i64]) -> bool {
        let rel = self.rel;
        let n = rel.rows.len() as u64;
        if (n + 1) * 2 > rel.pending.len() as u64 {
            self.grow_pending(((n + 1) * 4).next_power_of_two().max(64) as usize);
        }
        let slots = unsafe { rel.pending.as_mut_slice() };
        let mask = slots.len() - 1;
        let mut s = key_hash(vals) as usize & mask;
        loop {
            let x = slots[s];
            if x == 0 {
                let id = self.push(vals);
                slots[s] = id - rel.base_rows + 1;
                return true;
            }
            let id = x - 1;
            if (0..vals.len()).all(|c| rel.rows.get(id, c) == vals[c]) {
                return false;
            }
            s = (s + 1) & mask;
        }
    }

    fn grow_pending(&self, cap: usize) {
        let rel = self.rel;
        let p: *mut u32 = self.block.alloc_t(cap);
        let slots = unsafe { std::slice::from_raw_parts_mut(p, cap) };
        let mut row = vec![0i64; rel.arity as usize];
        for i in 0..rel.rows.len() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = rel.rows.get(i, c);
            }
            let mut s = key_hash(&row) as usize & (cap - 1);
            while slots[s] != 0 {
                s = (s + 1) & (cap - 1);
            }
            slots[s] = i + 1;
        }
        rel.pend_used.fetch_add((cap * 4) as u64, Relaxed);
        unsafe { Span::set(&rel.pending, p, cap) };
    }
}

/// What `freeze` did, for measurement.
#[derive(Default, Debug, Clone, Copy)]
pub struct FreezeStats {
    pub rows: u64,
    pub live: u64,
    pub dropped: u64,
    pub bytes: u64,
}

/// Freezes an open relation: set semantics, the canonical order, indexes, dead bits and statistics (see the
/// module's comment). `base` is the delta's base. After this, the relation only changes by a new delta. Its
/// temporaries come from this thread's scratch (`scratch::with`).
pub fn freeze(block: &Block, rel: &Relation, spec: &Spec, base: Option<&Relation>) -> FreezeStats {
    crate::scratch::with(|s| freeze_in(block, rel, spec, base, s))
}

/// Bits a value range needs.
fn bits_for(range: u64) -> u32 {
    64 - range.leading_zeros()
}

/// Sorts `ids` (rows of `cols`: column `c`'s `n` values at `cols[c * n..]`, ranging over `lo[c]..=hi[c]`) into the
/// canonical order of `key`. When the key packs into 64 bits (each column less its minimum, in the bits its range
/// needs, the first highest), the packed keys are radix sorted with the ids and returned alongside them: equal
/// keys are equal rows. Else the ids are sorted by comparing the rows.
pub(crate) fn sort_canonical<'s>(
    scratch: &'s crate::scratch::Scratch,
    cols: &[i64],
    n: usize,
    key: &[usize],
    lo: &[i64],
    hi: &[i64],
    ids: &mut [u32],
) -> Option<&'s [u64]> {
    let m = ids.len();
    let kbits: Vec<u32> = key.iter().map(|&c| if n == 0 { 0 } else { bits_for((hi[c] as i128 - lo[c] as i128) as u64) }).collect();
    let packed_bits: u32 = kbits.iter().sum();
    let val = |c: usize, i: usize| cols[c * n + i];
    if packed_bits > 64 {
        ids.sort_unstable_by(|&x, &y| {
            for &c in key {
                let o = val(c, x as usize).cmp(&val(c, y as usize));
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });
        return None;
    }
    let keys: &mut [u64] = scratch.take(m);
    for (k, &i) in keys.iter_mut().zip(ids.iter()) {
        let mut x = 0u64;
        for (j, &c) in key.iter().enumerate() {
            // A column of one value takes no bits; `checked_shl` keeps a 64-bit shift of an empty key defined.
            x = x.checked_shl(kbits[j]).unwrap_or(0) | (val(c, i as usize) as i128 - lo[c] as i128) as u64;
        }
        *k = x;
    }
    let (tk, ti) = (scratch.take::<u64>(m), scratch.take::<u32>(m));
    crate::sort::radix_pairs(keys, ids, packed_bits, tk, ti);
    Some(keys)
}

/// Whether the row at sorted place `p` equals the one before it (`sort_canonical`'s keys, or the rows).
#[inline]
pub(crate) fn same_row(cols: &[i64], n: usize, key: &[usize], keys: Option<&[u64]>, ids: &[u32], p: usize) -> bool {
    match keys {
        Some(k) => k[p] == k[p - 1],
        None => key.iter().all(|&c| cols[c * n + ids[p] as usize] == cols[c * n + ids[p - 1] as usize]),
    }
}

/// The canonical order's columns: the order key's, then the rest.
pub(crate) fn canonical_cols(spec: &Spec, a: usize) -> Vec<usize> {
    spec.order.iter().map(|&c| c as usize).chain((0..a).filter(|c| !spec.order.contains(&(*c as u16)))).collect()
}

/// `freeze`, its temporaries from `scratch`. How, for `n` rows of `a` columns:
///
/// - each column is read once, a chunk at a time, into `n` values;
/// - the order key is packed into a `u64` (each column less its minimum, in the bits its range needs) and radix
///   sorted with the row ids, when it fits 64 bits; else the ids are sorted by comparing the rows;
/// - a duplicate is a row whose packed key equals the one before it;
/// - each column is written back in order, a chunk at a time;
/// - an index's buckets are filled by one counting pass over the rows in order, its bucket count from the
///   number of distinct keys (a bitmap of the key's range, when that is small).
pub fn freeze_in(block: &Block, rel: &Relation, spec: &Spec, base: Option<&Relation>, scratch: &crate::scratch::Scratch) -> FreezeStats {
    assert!(!rel.is_frozen());
    let a = rel.arity as usize;
    // Its rows are moved by value: a column under key runs is written in its runs' order by its writer, never sorted.
    assert!((0..a).all(|c| !rel.rows.col(c).has_runs()), "a relation of key runs frozen");
    let n = rel.rows.len() as usize;
    let before = block.used();
    let br = rel.base_rows as usize;

    // Dead bits: its kills, over the base's rows and its own.
    let total = br + n;
    let dead: &mut [u64] = scratch.zeroed(total.div_ceil(64).max(1));
    let nkilled = rel.killed.len();
    let mut kills_base = false;
    for k in 0..nkilled {
        let id = rel.killed.get(k, 0) as usize;
        dead[id >> 6] |= 1 << (id & 63);
        kills_base |= id < br;
    }
    let is_dead = |dead: &[u64], id: usize| dead[id >> 6] >> (id & 63) & 1 == 1;

    // Each column read once, and its range over the rows.
    let cols: &mut [i64] = scratch.take(n * a);
    let mut lo = vec![i64::MAX; a];
    let mut hi = vec![i64::MIN; a];
    for c in 0..a {
        let col = &mut cols[c * n..(c + 1) * n];
        rel.rows.read_col(c, n as u32, col);
        for &v in col.iter() {
            lo[c] = lo[c].min(v);
            hi[c] = hi[c].max(v);
        }
    }
    let val = |c: usize, i: usize| cols[c * n + i];

    // The canonical order: the order key's columns, then the rest.
    let key = canonical_cols(spec, a);
    let ids: &mut [u32] = scratch.take(n);
    let mut m = 0;
    for i in 0..n {
        if nkilled == 0 || !is_dead(dead, br + i) {
            ids[m] = i as u32;
            m += 1;
        }
    }
    let ids = &mut ids[..m];
    let keys = sort_canonical(scratch, cols, n, &key, &lo, &hi, ids);
    let same = |p: usize| same_row(cols, n, &key, keys, ids, p);

    // Set semantics: a row once, and in a delta, not a row its base still holds live.
    let base_live = |i: usize| -> bool {
        let Some(b) = base else { return false };
        let (mut l, mut h) = (0u32, b.rows.len());
        while l < h {
            let mid = (l + h) / 2;
            let mut o = Ordering::Equal;
            for &c in &key {
                o = b.rows.get(mid, c).cmp(&val(c, i));
                if o != Ordering::Equal {
                    break;
                }
            }
            match o {
                Ordering::Less => l = mid + 1,
                Ordering::Greater => h = mid,
                Ordering::Equal => return !b.dead_bit(mid) && !is_dead(dead, mid as usize),
            }
        }
        false
    };
    // Decided against the kills alone: a duplicate dropped here is one of its own rows, never a base row.
    let order: &mut [u32] = scratch.take(m);
    let mut live = 0;
    for (p, &id) in ids.iter().enumerate().take(m) {
        let i = id as usize;
        if (p > 0 && same(p)) || base_live(i) {
            continue;
        }
        order[live] = i as u32;
        live += 1;
    }
    let order = &order[..live];
    let dropped = (m - live) as u64;

    // Sorted in place: each column written back in canonical order, so an own row's id is its place in that
    // order. No order array, and no dead bits for its own rows (the dropped ones are gone). The statistics are
    // each column's range over the live rows.
    let moved: &mut [i64] = scratch.take(live);
    let sp: *mut ColStats = block.alloc_t(a);
    for c in 0..a {
        let (mut l, mut h) = (i64::MAX, i64::MIN);
        for (d, &i) in moved.iter_mut().zip(order) {
            let v = val(c, i as usize);
            *d = v;
            l = l.min(v);
            h = h.max(v);
        }
        rel.rows.write_col(c, moved);
        unsafe { *sp.add(c) = ColStats { min: l, max: h } };
    }
    rel.rows.len.store(live as u32, Release);

    // Indexes: each key's rows grouped, in canonical order within a group.
    let ixp: *mut crate::relation::Index = block.alloc_t(spec.indexes.len().max(1));
    for (x, icols) in spec.indexes.iter().enumerate() {
        assert!(icols.len() <= MAX_KEY);
        let lead = &spec.order[..icols.len().min(spec.order.len())];
        let ix = unsafe { &mut *ixp.add(x) };
        ix.ncols = icols.len() as u32;
        for (j, &c) in icols.iter().enumerate() {
            ix.key[j] = c;
        }
        if icols.len() <= spec.order.len() && icols.iter().all(|c| lead.contains(c)) {
            ix.ordered = 1;
            continue;
        }
        let kv = |j: usize| -> [i64; MAX_KEY] {
            let mut k = [0i64; MAX_KEY];
            for (t, &c) in icols.iter().enumerate() {
                k[t] = val(c as usize, order[j] as usize);
            }
            k
        };
        let nk = icols.len();
        // Distinct keys: a bitmap over the key's packed range when it is at most 2^24, else the hashes sorted.
        let ibits: Vec<u32> = icols.iter().map(|&c| bits_for((hi[c as usize] as i128 - lo[c as usize] as i128) as u64)).collect();
        let span: u32 = ibits.iter().sum();
        let distinct = if live == 0 {
            0
        } else if span <= 24 {
            let words: &mut [u64] = scratch.zeroed((1usize << span).div_ceil(64));
            let mut d = 0usize;
            for j in 0..live {
                let k = kv(j);
                let mut p = 0u64;
                for (t, &c) in icols.iter().enumerate() {
                    p = (p << ibits[t]) | (k[t] - lo[c as usize]) as u64;
                }
                let (w, b) = ((p >> 6) as usize, p & 63);
                d += (words[w] >> b & 1 == 0) as usize;
                words[w] |= 1 << b;
            }
            d
        } else {
            let hs: &mut [u64] = scratch.take(live);
            for (j, h) in hs.iter_mut().enumerate() {
                *h = key_hash(&kv(j)[..nk]);
            }
            hs.sort_unstable();
            1 + hs.windows(2).filter(|w| w[0] != w[1]).count()
        };
        let cap = distinct.next_power_of_two().max(1);
        let mask = (cap - 1) as u64;
        // One counting pass: each row's bucket, the buckets' sizes, then the rows placed in order.
        let bucket: &mut [u32] = scratch.take(live);
        let bp: *mut u32 = block.alloc_t(cap + 1);
        let rp: *mut u32 = block.alloc_t(live.max(1));
        let offs = unsafe { std::slice::from_raw_parts_mut(bp, cap + 1) };
        for (j, b) in bucket.iter_mut().enumerate() {
            *b = (key_hash(&kv(j)[..nk]) & mask) as u32;
            offs[*b as usize + 1] += 1;
        }
        for b in 0..cap {
            offs[b + 1] += offs[b];
        }
        let at: &mut [u32] = scratch.take(cap);
        at.copy_from_slice(&offs[..cap]);
        let rows_out = unsafe { std::slice::from_raw_parts_mut(rp, live.max(1)) };
        for (j, &b) in bucket.iter().enumerate() {
            rows_out[at[b as usize] as usize] = (br + j) as u32;
            at[b as usize] += 1;
        }
        ix.mask = mask;
        unsafe {
            Span::set(&raw mut ix.offs, bp, cap + 1);
            Span::set(&raw mut ix.rows, rp, live);
        }
    }

    // Dead bits: only the base's rows a delta killed.
    unsafe {
        if kills_base {
            let base_words = br.div_ceil(64);
            // The last word's bits past the base are own ids, which moved: none of them is dead.
            let tail = br % 64;
            if tail != 0 {
                dead[base_words - 1] &= (1u64 << tail) - 1;
            }
            let words: *mut u64 = block.alloc_t(base_words);
            std::ptr::copy_nonoverlapping(dead.as_ptr(), words, base_words);
            Span::set(&rel.dead, words, base_words);
        }
        Span::set(&rel.indexes, ixp, spec.indexes.len());
        Span::set(&rel.stats, sp, a);
    }
    rel.nindex.store(spec.indexes.len() as u32, Relaxed);
    rel.live.store(live as u32, Relaxed);
    rel.flags.fetch_or(FROZEN, Release);
    FreezeStats { rows: n as u64, live: live as u64, dropped, bytes: (block.used() - before) as u64 }
}

/// A relation as readers see it: a full relation alone, or a base and its delta read as one. Ids `..base_rows`
/// are the base's, the rest the delta's. A probe and the ordered scan come in the canonical order.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub base: Option<&'a Relation>,
    pub top: &'a Relation,
}

impl<'a> View<'a> {
    pub fn full(r: &'a Relation) -> View<'a> {
        View { base: None, top: r }
    }

    #[inline]
    pub fn arity(&self) -> usize {
        self.top.arity as usize
    }

    /// Every id, live or not: `0 .. total`.
    #[inline]
    pub fn total(&self) -> u32 {
        self.top.base_rows + self.top.rows.len()
    }

    #[inline]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        if id < self.top.base_rows { self.base.expect("a delta's base").rows.get(id, c) } else { self.top.own(id, c) }
    }

    #[inline]
    pub fn live(&self, id: u32) -> bool {
        if self.top.dead_bit(id) {
            return false;
        }
        if id < self.top.base_rows {
            return !self.base.expect("a delta's base").dead_bit(id);
        }
        // An open relation's own rows are live; a frozen one's dead bits cover its dropped duplicates.
        true
    }

    pub fn row(&self, id: u32) -> Vec<i64> {
        (0..self.arity()).map(|c| self.get(id, c)).collect()
    }

    fn cmp(&self, x: u32, y: u32) -> Ordering {
        let ok = &self.top.okey[..self.top.nokey as usize];
        for &c in ok {
            let o = self.get(x, c as usize).cmp(&self.get(y, c as usize));
            if o != Ordering::Equal {
                return o;
            }
        }
        for c in 0..self.arity() {
            if ok.contains(&(c as u16)) {
                continue;
            }
            let o = self.get(x, c).cmp(&self.get(y, c));
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    }

    /// Live ids in the canonical order: the base's (less what the delta killed) merged with the delta's.
    pub fn ordered(&self) -> Vec<u32> {
        self.ordered_iter().collect()
    }

    /// `ordered`, read in place: no allocation.
    pub fn ordered_iter(&self) -> impl Iterator<Item = u32> + '_ {
        assert!(self.top.is_frozen(), "an ordered read needs a frozen relation");
        let own = self.top.base_rows..self.top.base_rows + self.top.rows.len();
        let based = 0..self.base.map_or(0, |b| b.rows.len());
        Merge::new(self, based.filter(|&id| self.live(id)), own)
    }

    pub fn live_len(&self) -> usize {
        let own = self.top.rows.len() as usize;
        match self.base {
            None => own,
            Some(b) => own + (0..b.rows.len()).filter(|&id| self.live(id)).count(),
        }
    }

    /// The live rows whose index-`i` columns equal `key`, in canonical order.
    pub fn probe(&self, i: usize, key: &[i64]) -> Vec<u32> {
        self.probe_iter(i, key).collect()
    }

    /// `probe`, read in place: both groups filtered and merged as they are walked, no allocation.
    pub fn probe_iter<'k>(&'k self, i: usize, key: &'k [i64]) -> Probe<'k> {
        let ix = self.top.index_at(i);
        let cols = &ix.key[..ix.ncols as usize];
        let (own, based, exact) = if ix.ordered == 1 {
            let own = leading(self.top, cols, key);
            let based = self.base.map_or(0..0, |b| leading(b, cols, key));
            (Ids::Range(self.top.base_rows + own.start..self.top.base_rows + own.end), Ids::Range(based), true)
        } else {
            let h = key_hash(key);
            let based: &[u32] = self.base.map_or(&[], |b| b.group(i, h));
            (Ids::Slice(self.top.group(i, h).iter()), Ids::Slice(based.iter()), false)
        };
        let own0 = self.top.rows.reader(cols[0] as usize);
        let base0 = self.base.map_or(own0, |b| b.rows.reader(cols[0] as usize));
        Probe { v: *self, key, cols, own0, base0, exact, own, based, ha: None, hb: None, primed: false }
    }

    /// A digest of the live rows in canonical order: the same for the same set, whatever wrote it, in any order.
    pub fn digest(&self) -> u64 {
        let mut h = crate::SEED;
        let mut n = 0u64;
        for id in self.ordered() {
            for c in 0..self.arity() {
                h = crate::mix(h, self.get(id, c));
            }
            n += 1;
        }
        crate::mix(h, n as i64)
    }
}

/// A probe's answer, in canonical order: a frozen relation's own rows (always live: its dead bits name only its
/// base's rows), and when it is a delta, its base's live rows, merged. A leading key's ranges are exact; a bucket's
/// rows are checked against the key.
pub struct Probe<'k> {
    v: View<'k>,
    /// The key, and its first column resolved once: the delta's (or the full relation's) own, and its base's. A
    /// key of more columns checks the rest by `cols` (`rest_match`).
    key: &'k [i64],
    cols: &'k [u16],
    own0: crate::rows::ColRead,
    base0: crate::rows::ColRead,
    exact: bool,
    own: Ids<'k>,
    based: Ids<'k>,
    ha: Option<u32>,
    hb: Option<u32>,
    primed: bool,
}

/// A key's columns past its first, checked on `rows` at `row`.
#[cold]
#[inline(never)]
fn rest_match(rows: &Rows, cols: &[u16], key: &[i64], row: u32) -> bool {
    cols.iter().zip(key).skip(1).all(|(&c, &k)| rows.get(row, c as usize) == k)
}

impl Probe<'_> {
    #[inline(always)]
    fn next_own(&mut self) -> Option<u32> {
        let br = self.v.top.base_rows;
        loop {
            let id = self.own.next()?;
            if self.exact {
                return Some(id);
            }
            let row = id - br;
            if self.own0.get(row) == self.key[0] && (self.cols.len() == 1 || rest_match(&self.v.top.rows, self.cols, self.key, row)) {
                return Some(id);
            }
        }
    }

    #[inline]
    fn next_based(&mut self) -> Option<u32> {
        let b = self.v.base?;
        loop {
            let id = self.based.next()?;
            if self.v.top.dead_bit(id) {
                continue;
            }
            if self.exact || (self.base0.get(id) == self.key[0] && (self.cols.len() == 1 || rest_match(&b.rows, self.cols, self.key, id))) {
                return Some(id);
            }
        }
    }
}

impl Iterator for Probe<'_> {
    type Item = u32;

    #[inline(always)]
    fn next(&mut self) -> Option<u32> {
        if self.v.base.is_none() {
            return self.next_own();
        }
        if !self.primed {
            self.primed = true;
            self.ha = self.next_based();
            self.hb = self.next_own();
        }
        let take_a = match (self.ha, self.hb) {
            (Some(x), Some(y)) => self.v.cmp(x, y) != Ordering::Greater,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return None,
        };
        if take_a {
            let x = self.ha;
            self.ha = self.next_based();
            x
        } else {
            let y = self.hb;
            self.hb = self.next_own();
            y
        }
    }
}

/// Ids from a hash bucket, or a range of sorted rows.
enum Ids<'a> {
    Slice(std::slice::Iter<'a, u32>),
    Range(std::ops::Range<u32>),
}

impl Iterator for Ids<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        match self {
            Ids::Slice(s) => s.next().copied(),
            Ids::Range(r) => r.next(),
        }
    }
}

/// The positions of `rel`'s own rows (sorted in place) whose columns `cols`, the order key's first ones, equal
/// `key`: one range, by binary search on each leading column in turn.
fn leading(rel: &Relation, cols: &[u16], key: &[i64]) -> std::ops::Range<u32> {
    let (mut lo, mut hi) = (0u32, rel.rows.len());
    for &oc in &rel.okey[..cols.len()] {
        let k = key[cols.iter().position(|&c| c == oc).expect("a leading column")];
        let a = rel.rows.lower_bound(oc as usize, lo, hi, k);
        hi = rel.rows.upper_from(oc as usize, a, hi, k);
        lo = a;
    }
    lo..hi
}

/// Two id streams, each in canonical order, merged into one.
struct Merge<'v, A: Iterator<Item = u32>, B: Iterator<Item = u32>> {
    v: &'v View<'v>,
    a: std::iter::Peekable<A>,
    b: std::iter::Peekable<B>,
}

impl<'v, A: Iterator<Item = u32>, B: Iterator<Item = u32>> Merge<'v, A, B> {
    fn new(v: &'v View<'v>, a: A, b: B) -> Self {
        Merge { v, a: a.peekable(), b: b.peekable() }
    }
}

impl<A: Iterator<Item = u32>, B: Iterator<Item = u32>> Iterator for Merge<'_, A, B> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        match (self.a.peek(), self.b.peek()) {
            (Some(&x), Some(&y)) => {
                if self.v.cmp(x, y) != Ordering::Greater { self.a.next() } else { self.b.next() }
            }
            (Some(_), None) => self.a.next(),
            _ => self.b.next(),
        }
    }
}


#[cfg(test)]
mod tests {
    /// A relation released in a free-listed block (decision 0342) gives its chunks back: the next relation of its
    /// shape takes them, so the block grows by the descriptors alone, and reads its own rows.
    #[test]
    fn a_released_relations_chunks_are_taken_by_the_next() {
        let block = Block::anon(kind::FACTS, 64 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8, 8], order: Vec::new(), indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let fill = |r: &Relation, base: i64| {
            assert!(r.rows.try_reserve(&block, 50_000));
            unsafe {
                r.rows.claim_reserved(50_000);
                for i in 0..50_000u32 {
                    for c in 0..3 {
                        r.rows.put(i, c, base + i as i64 * 3 + c as i64);
                    }
                }
            }
        };
        let a = Relation::try_create(&block, &spec, None).unwrap();
        fill(a, 0);
        let after_one = block.used();
        unsafe { a.release(&block) };
        assert!(block.free_bytes() >= 50_000 * 24, "its chunks are free: {}", block.free_bytes());
        let b = Relation::try_create(&block, &spec, None).unwrap();
        fill(b, 1_000_000);
        assert_eq!(block.used(), after_one, "the next took its chunks and its descriptor: nothing new from the front");
        assert_eq!(b.rows.get(49_999, 2), 1_000_000 + 49_999 * 3 + 2);
    }

    use super::*;
    use crate::block::kind;
    use std::collections::BTreeSet;

    /// Decision 0384 step 7: a continuation reads its ids below `base_rows` in its base (in the same block) and its
    /// own after, by the read written once (`rel_get`); its index is of every id; a kills delta of it marks any; its
    /// base's rows written past `base_rows` (the base's own writer going on) are not its.
    #[test]
    fn a_continuation_reads_its_bases_rows_where_they_lie() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8], order: Vec::new(), indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let put = |r: &Relation, from: u32, n: u32, tag: i64| {
            assert!(r.rows.try_reserve(&block, u64::from(r.rows.len() + n)));
            let at = r.rows.len();
            unsafe {
                r.rows.claim_reserved(n);
                for i in 0..n {
                    r.rows.put(at + i, 0, i64::from((from + i) % 10));
                    r.rows.put(at + i, 1, tag + i64::from(from + i));
                }
            }
        };
        let base = Relation::try_create(&block, &spec, None).unwrap();
        put(base, 0, 100, 1000);
        let fork = Relation::try_continue(&block, &spec, base, 100).expect("room in the base's block");
        assert!(fork.continues() && std::ptr::eq(fork.base().unwrap(), base));
        put(fork, 100, 50, 2000);
        // The base's writer goes on past the fork's start: rows the fork does not read.
        put(base, 100, 30, 9000);
        assert_eq!(fork.len_all(), 150);
        for id in 0..150 {
            assert_eq!(fork.get(id, 1), if id < 100 { 1000 } else { 2000 } + i64::from(id), "row {id}");
        }
        // An index of every id it reads.
        let n = fork.len_all();
        let room = unsafe { fork.try_reserve_index(&block, &[0], n) }.expect("room");
        unsafe {
            fork.build_index(&room);
            fork.index_built(&room);
        }
        let i = fork.find_index(&[0], n).expect("built");
        let key = |v: i64| [v].iter().fold(crate::SEED, |h, &x| crate::mix(h, x));
        let mut sevens: Vec<u32> = fork.group(i, key(7)).iter().copied().filter(|&id| fork.get(id, 0) == 7).collect();
        sevens.sort_unstable();
        assert_eq!(sevens, (0..150).filter(|id| id % 10 == 7).collect::<Vec<u32>>(), "its base's rows and its own");
        // Kills of any of its ids.
        let kills = Relation::try_kills(&block, &spec, fork, &[1 << 3, 0, 1 << 4]).expect("a delta");
        assert!(kills.dead_bit(3) && kills.dead_bit(132) && !kills.dead_bit(4));
        // A continuation of a continuation (a fork of a fork, step 8): ids read down the chain. Below its base's base
        // rows, or past what its base reads, it is refused; a kills delta is no base.
        assert!(Relation::try_continue(&block, &spec, fork, 10).is_none());
        assert!(Relation::try_continue(&block, &spec, fork, 151).is_none());
        assert!(Relation::try_continue(&block, &spec, kills, 0).is_none());
        let fork2 = Relation::try_continue(&block, &spec, fork, 120).expect("a continuation of a continuation");
        put(fork2, 120, 40, 3000);
        // The first fork's writer goes on past 120: rows the second does not read.
        put(fork, 150, 5, 8000);
        assert_eq!(fork2.len_all(), 160);
        for id in 0..160 {
            let want = if id < 100 { 1000 } else if id < 120 { 2000 } else { 3000 } + i64::from(id);
            assert_eq!(fork2.get(id, 1), want, "row {id} down the chain");
        }
        let n = fork2.len_all();
        let room = unsafe { fork2.try_reserve_index(&block, &[0], n) }.expect("room");
        unsafe {
            fork2.build_index(&room);
            fork2.index_built(&room);
        }
        let i = fork2.find_index(&[0], n).expect("built");
        let mut threes: Vec<u32> = fork2.group(i, key(3)).iter().copied().filter(|&id| fork2.get(id, 0) == 3).collect();
        threes.sort_unstable();
        assert_eq!(threes, (0..160).filter(|id| id % 10 == 3).collect::<Vec<u32>>(), "every id down the chain");
        // Kept, a continuation is packed with every row it reads (decision 0456 K3: `kept_facts::columns`, tested
        // there as `a_continuation_is_kept_with_every_row_it_reads`).
    }

    /// Decision 0384 step 5: an added index's buckets and rows are chunks of a free-listed block, which forgetting it
    /// (`forget_indexes`, a relation whose rows changed in place) gives back, and the next index takes again: a
    /// relation probed after every change of its rows grows its block by nothing. Released, the relation gives back
    /// its indexes and their directory too.
    #[test]
    fn a_forgotten_index_gives_its_room_back() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let spec = Spec { widths: vec![8, 8], order: Vec::new(), indexes: Vec::new(), min_log: crate::rows::MIN_LOG };
        let rel = Relation::try_create(&block, &spec, None).unwrap();
        rel.rewritten();
        let n = 20_000u32;
        assert!(rel.rows.try_reserve(&block, u64::from(n)));
        unsafe {
            rel.rows.claim_reserved(n);
            for i in 0..n {
                rel.rows.put(i, 0, i64::from(i % 97));
                rel.rows.put(i, 1, i64::from(i));
            }
        }
        let index = |r: &Relation| unsafe {
            let room = r.try_reserve_index(&block, &[0], n).expect("room");
            r.build_index(&room);
            r.index_built(&room);
        };
        index(rel);
        assert!(rel.find_index(&[0], n).is_some());
        let used = block.used();
        let free = block.free_bytes();
        unsafe { rel.forget_indexes(&block) };
        assert!(rel.find_index(&[0], n).is_none(), "forgotten");
        assert!(block.free_bytes() >= free + 4 * n as usize, "its buckets and rows back on the free list: {} from {free}", block.free_bytes());
        for _ in 0..20 {
            index(rel);
            unsafe { rel.forget_indexes(&block) };
        }
        index(rel);
        assert_eq!(block.used(), used, "each index again in the room the last gave back");
        let i = rel.find_index(&[0], n).expect("built again");
        let group = rel.group(i, [5i64].iter().fold(crate::SEED, |h, &v| crate::mix(h, v)));
        assert_eq!(group.iter().filter(|&&r| rel.rows.get(r, 0) == 5).count(), (0..n).filter(|i| i % 97 == 5).count(), "a key's rows");
        let free = block.free_bytes();
        unsafe { rel.release(&block) };
        assert!(block.free_bytes() >= free + 4 * n as usize + 16 * n as usize, "rows, index and directory given back");
    }

    fn rows_of(v: &View) -> Vec<Vec<i64>> {
        v.ordered_iter().map(|id| v.row(id)).collect()
    }

    /// An index added after the relation was made (`try_reserve_index`, `build_index`): found by its key once built
    /// (not before), and each key's group, read by the read written once (`group`), holds exactly that key's rows
    /// in id order, for a key of one column and one of eight. A full directory refuses.
    #[test]
    fn an_added_index_groups_each_keys_rows_in_id_order() {
        let block = Block::anon(kind::FACTS, 16 << 20);
        let lock = Mutex::new(());
        let spec = Spec { widths: vec![8; 9], order: Vec::new(), indexes: Vec::new(), min_log: 4 };
        let rel = Relation::create(&block, &spec, None);
        let rows: Vec<[i64; 9]> = (0..2_000i64).map(|i| std::array::from_fn(|c| if c == 0 { i % 37 } else { (i * 7 + c as i64) % 5 - 2 })).collect();
        RelW { block: &block, rel, lock: &lock }.extend(&rows.iter().flatten().copied().collect::<Vec<_>>());
        let n = rel.rows.len();
        for cols in [vec![0u16], (1..9).collect::<Vec<u16>>()] {
            assert_eq!(rel.find_index(&cols, n), None);
            // SAFETY: one thread; the rows are written.
            let room = unsafe { rel.try_reserve_index(&block, &cols, n) }.expect("room");
            assert_eq!(rel.find_index(&cols, n), None, "not found before it is built");
            unsafe {
                rel.build_index(&room);
                rel.index_built(&room);
            }
            let i = rel.find_index(&cols, n).expect("found once built");
            for probe in [0usize, 1, 5, 36, 1999] {
                let key: Vec<i64> = cols.iter().map(|&c| rows[probe][c as usize]).collect();
                let want: Vec<u32> = (0..n).filter(|&r| cols.iter().zip(&key).all(|(&c, &v)| rows[r as usize][c as usize] == v)).collect();
                let got: Vec<u32> = rel.group(i, crate::key_hash(&key)).iter().copied().filter(|&r| cols.iter().zip(&key).all(|(&c, &v)| rel.rows.get(r, c as usize) == v)).collect();
                assert_eq!(got, want, "key {key:?} on {cols:?}");
            }
        }
        for c in 2..ADDED_INDEXES as u16 {
            assert!(unsafe { rel.try_reserve_index(&block, &[c], n) }.is_some());
        }
        assert!(unsafe { rel.try_reserve_index(&block, &[0, 1], n) }.is_none(), "the directory is full");
    }

    /// An index of rows `from..` (a delta read in place) is found by its first row as well as its count, never for
    /// the relation's first rows; an index given back (`give_back_index`) is found no more, its room and its entry
    /// taken by the next reservation, and the directory's count does not shrink.
    #[test]
    fn an_index_is_of_its_rows_from_and_one_given_back_is_taken_again() {
        let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
        let lock = Mutex::new(());
        let spec = Spec { widths: vec![8; 2], order: Vec::new(), indexes: Vec::new(), min_log: 4 };
        let rel = Relation::create(&block, &spec, None);
        let rows: Vec<i64> = (0..2_000i64).flat_map(|i| [i % 37, i]).collect();
        RelW { block: &block, rel, lock: &lock }.extend(&rows);
        // SAFETY: one thread; the rows are written. Built as the device builds them: rows `from + t`.
        let full = unsafe { rel.try_reserve_index(&block, &[0], 1_000) }.expect("room");
        unsafe { rel.index_built(&full) };
        let delta = unsafe { rel.try_reserve_index_from(&block, &[0], 1_000, 1_000) }.expect("room");
        unsafe { rel.index_built(&delta) };
        assert_eq!(rel.find_index(&[0], 1_000), Some(full.index), "the first rows' index");
        assert_eq!(rel.find_index_from(&[0], 1_000, 1_000), Some(delta.index), "the delta's, not the first rows'");
        assert_eq!(rel.find_index_from(&[0], 500, 1_000), None);
        let (count, used) = (rel.nindex.load(Relaxed), block.used());
        // SAFETY: one thread; no reader holds it.
        unsafe { rel.give_back_index(&block, &full) };
        assert_eq!(rel.find_index(&[0], 1_000), None, "given back");
        assert_eq!(rel.find_index_from(&[0], 1_000, 1_000), Some(delta.index), "the other kept");
        let again = unsafe { rel.try_reserve_index(&block, &[1], 1_000) }.expect("room");
        assert_eq!(again.index, full.index, "the entry given back is taken");
        assert_eq!(block.used(), used, "and its room");
        assert_eq!(rel.nindex.load(Relaxed), count, "the directory's count kept");
        unsafe { rel.index_built(&again) };
        assert_eq!(rel.find_index(&[1], 1_000), Some(again.index));
        // Forgotten, every entry, the dead one too, goes; none is given back twice.
        unsafe { rel.give_back_index(&block, &again) };
        unsafe { rel.forget_indexes(&block) };
        assert_eq!(rel.find_index_from(&[0], 1_000, 1_000), None);

        // A block of no free list (a relation alone in its block): the entry keeps its room, and an index that fits
        // takes it, the block's bytes as they were; one that does not fit takes new room.
        let block = Block::anon(kind::FACTS, 16 << 20);
        let rel = Relation::create(&block, &spec, None);
        RelW { block: &block, rel, lock: &lock }.extend(&rows);
        let twice = unsafe { rel.try_reserve_index(&block, &[0, 1], 1_000) }.expect("room");
        unsafe { rel.index_built(&twice) };
        let used = block.used();
        unsafe { rel.give_back_index(&block, &twice) };
        assert_eq!(rel.find_index(&[0, 1], 1_000), None, "given back");
        let other = unsafe { rel.try_reserve_index(&block, &[0], 900) }.expect("room");
        assert_eq!((other.index, other.offs, other.rows), (twice.index, twice.offs, twice.rows), "its room taken again");
        assert_eq!(block.used(), used);
        unsafe {
            rel.build_index(&other);
            rel.index_built(&other);
        }
        let i = rel.find_index(&[0], 900).expect("built");
        let got: Vec<u32> = rel.group(i, crate::key_hash(&[5])).iter().copied().filter(|&r| rel.rows.get(r, 0) == 5).collect();
        assert_eq!(got, (0..900).filter(|r| r % 37 == 5).collect::<Vec<u32>>(), "read right from the room taken again");
        let bigger = unsafe { rel.try_reserve_index(&block, &[1], 2_000) }.expect("room");
        assert_ne!(bigger.index, twice.index);
        assert!(block.used() > used);
    }

    /// Freeze keeps a set in canonical order, a delta's set being its base's less its kills plus its own rows; the
    /// probes answer as a scan does. Both sorts: the packed key, and a key too wide to pack.
    #[test]
    fn freeze_and_probes_agree_with_a_set() {
        for wide in [false, true] {
            let spec = Spec { widths: vec![8, 4, 4], order: vec![1, 0], indexes: vec![vec![1], vec![2]], min_log: 4 };
            let big = if wide { 1i64 << 50 } else { 1 };
            let mut x = 7u64;
            let mut next = |m: u64| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % m) as i64
            };
            let rows: Vec<[i64; 3]> = (0..3_000).map(|_| [next(40) * big, next(30), next(500)]).collect();
            let block = Block::anon(kind::FACTS, 64 << 20);
            let lock = Mutex::new(());
            let rel = Relation::create(&block, &spec, None);
            RelW { block: &block, rel, lock: &lock }.extend(&rows.iter().flatten().copied().collect::<Vec<_>>());
            freeze(&block, rel, &spec, None);
            let set: BTreeSet<(i64, i64, i64)> = rows.iter().map(|r| (r[1], r[0], r[2])).collect();
            let v = View::full(rel);
            let want: Vec<Vec<i64>> = set.iter().map(|&(b, a, c)| vec![a, b, c]).collect();
            assert_eq!(rows_of(&v), want, "wide {wide}: the set, in canonical order");

            // A delta: kills of base rows and of its own, repeats of base rows, new rows.
            let dblock = Block::anon(kind::FACTS, 16 << 20);
            let d = Relation::create(&dblock, &spec, Some(rel));
            let w = RelW { block: &dblock, rel: d, lock: &lock };
            let mut now = set.clone();
            for id in (0..rel.rows.len()).step_by(97) {
                w.kill(id);
                let r = v.row(id);
                now.remove(&(r[1], r[0], r[2]));
            }
            for id in (5..rel.rows.len()).step_by(211) {
                let r = v.row(id);
                w.push(&r);
                now.insert((r[1], r[0], r[2]));
            }
            for i in 0..300 {
                let r = [next(50) * big, 30 + next(5), i];
                let own = w.push(&r);
                if i % 7 == 0 {
                    w.kill(own);
                } else {
                    now.insert((r[1], r[0], r[2]));
                }
            }
            freeze(&dblock, d, &spec, Some(rel));
            let dv = View { base: Some(rel), top: d };
            let want: Vec<Vec<i64>> = now.iter().map(|&(b, a, c)| vec![a, b, c]).collect();
            assert_eq!(rows_of(&dv), want, "wide {wide}: the delta's set");
            assert_eq!(dv.live_len(), now.len());
            for (ix, col) in [(0usize, 1usize), (1, 2)] {
                for k in [0i64, 3, 17, 31, 33, 250, 499, 1_000] {
                    let got: Vec<Vec<i64>> = dv.probe_iter(ix, &[k]).map(|id| dv.row(id)).collect();
                    let scan: Vec<Vec<i64>> = want.iter().filter(|r| r[col] == k).cloned().collect();
                    assert_eq!(got, scan, "wide {wide}: probe of column {col} for {k}");
                }
            }
        }
    }
}
