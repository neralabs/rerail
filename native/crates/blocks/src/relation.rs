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
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::*};

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
/// A key's columns at most: the device's probes' (decision 0325 step 5).
pub const MAX_KEY: usize = 8;
/// Indexes a relation can be given after it is made (`try_add_index`): its directory's room.
pub const ADDED_INDEXES: usize = 8;

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
        /// order. A reader checks the key (4 bytes a bucket, about one key a bucket; no hash kept).
        pub offs: Span<u32>,
        pub rows: Span<u32>,
    }
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

    /// A delta of `base` with no rows of its own whose dead bits are `words` (row `id` dead when bit `id` of them is
    /// set): a relation's kills of the rows it shares, read beside them where they lie (decision 0325: the device
    /// skips a dead row by its bit, `rel_dead`). `None` when the block cannot hold it.
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

    /// A copy of this full relation in open `block`: its descriptor made again, then its rows' and kills' chunks,
    /// dead bits, indexes and statistics copied whole by `memcpy` (no row decoded or re-sorted, no index rebuilt),
    /// each reference set to its copy. A reader of the copy reads what a reader of this reads (0401 F2, F6). It is
    /// frozen or not (a run's part, its rows in any order, with the indexes the run added); nothing writes it
    /// meanwhile, and an open relation's set test (`pending`) is not copied. `None` when `block` has no room.
    pub fn try_copy_into<'b>(&self, block: &'b Block) -> Option<&'b Relation> {
        assert!(!self.is_delta(), "only a full relation is copied");
        let spec = Spec {
            widths: (0..self.arity as usize).map(|c| self.rows.col(c).width).collect(),
            order: self.okey[..self.nokey as usize].to_vec(),
            indexes: Vec::new(),
            min_log: self.rows.min_log,
        };
        let rel = Relation::try_create(block, &spec, None)?;
        if !rel.rows.try_copy_from(block, &self.rows) || !rel.killed.try_copy_from(block, &self.killed) {
            return None;
        }
        /// `src`'s items in `block`, `dst` set to them.
        fn copy_span<T>(block: &Block, dst: &Span<T>, src: &Span<T>) -> Option<()> {
            if src.is_empty() {
                return Some(());
            }
            let p: *mut T = block.try_alloc_t(src.len())?;
            // SAFETY: `p` holds `src.len()` items, just allocated in open `block`, where `dst` lies too; the items are
            // plain data (`Pod`), copied as bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), p, src.len());
                Span::set(dst, p, src.len());
            }
            Some(())
        }
        copy_span(block, &rel.dead, &self.dead)?;
        copy_span(block, &rel.stats, &self.stats)?;
        if !self.indexes.is_empty() {
            let n = self.indexes.len();
            let dir: *mut Index = block.try_alloc_t(n)?;
            // SAFETY: `dir` holds `n` zeroed entries in open `block`, as `rel` is; each index's spans are copied
            // after its fixed fields.
            unsafe {
                Span::set(&rel.indexes, dir, n);
                for (i, ix) in self.indexes.as_slice().iter().enumerate() {
                    let to = &*dir.add(i);
                    let w = dir.add(i);
                    (*w).ncols = std::ptr::read_volatile(&raw const ix.ncols);
                    (*w).ordered = ix.ordered;
                    (*w).key = ix.key;
                    (*w).mask = ix.mask;
                    copy_span(block, &to.offs, &ix.offs)?;
                    copy_span(block, &to.rows, &ix.rows)?;
                }
            }
        }
        // SAFETY: `rel` is this block's and not yet given to anyone: its counters are ours to set.
        unsafe {
            let me = std::ptr::from_ref(rel).cast_mut();
            (*me).base_rows = 0;
        }
        rel.live.store(self.live.load(Acquire), Relaxed);
        rel.nindex.store(self.nindex.load(Acquire), Relaxed);
        rel.flags.store(self.flags.load(Acquire), Release);
        Some(rel)
    }

    /// Gives its chunks back to the block's free list (decision 0342), when the block keeps one. Returns the bytes
    /// given to the OS with them (`Block::free_chunk`).
    ///
    /// # Safety
    /// It lies in `block`, and no one reads or writes it after (its last holder's).
    pub unsafe fn release(&self, block: &Block) -> usize {
        let given = unsafe { self.rows.release(block) + self.killed.release(block) };
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

    /// Whether the dead bits mark row `id`: the read written once (`rel_dead`, decision 0325).
    #[inline]
    pub fn dead_bit(&self, id: u32) -> bool {
        // SAFETY: a live descriptor; the read checks `id` against the bits' length.
        unsafe { crate::generated_reads::rel_dead(std::ptr::from_ref(self).cast(), id) != 0 }
    }

    #[inline]
    pub fn index(&self, i: usize) -> &Index {
        &self.indexes.as_slice()[i]
    }

    /// One relation's rows for a key of hash `h` on index `i`, before liveness and key equality: its bucket, found
    /// by the read written once (`rel_group`, decision 0325).
    #[inline]
    pub fn group(&self, i: usize, h: u64) -> &[u32] {
        // SAFETY: a live descriptor; index `i` is one of its indexes.
        let g = unsafe { crate::generated_reads::rel_group(std::ptr::from_ref(self).cast(), i as u32, h) };
        let (start, len) = ((g >> 32) as usize, (g & 0xffff_ffff) as usize);
        if len == 0 {
            return &[];
        }
        &self.index(i).rows.as_slice()[start..start + len]
    }

    /// The index whose key is `cols` over its rows `0..rows` (a hashed one: an ordered index holds no buckets), built.
    /// An index of fewer rows (built before the relation grew) is not it.
    pub fn find_index(&self, cols: &[u16], rows: u32) -> Option<usize> {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: an entry's `ncols` is written once, last (`index_built`); read as written, then the index after it.
        let built = |i: usize| unsafe { std::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| ix[i].ordered == 0 && built(i) == cols.len() && ix[i].key[..cols.len()] == *cols && ix[i].rows.len() == rows as usize);
        std::sync::atomic::fence(Acquire);
        found
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
            let me = std::ptr::from_ref(self).cast_mut();
            if self.indexes.len() == 0 {
                let (at, _) = block.try_alloc_chunk_fit_zeroed(ADDED_INDEXES * size_of::<Index>(), 8)?;
                Span::set(&raw mut (*me).indexes, block.ptr::<Index>(at), ADDED_INDEXES);
                self.flags.fetch_or(ADDED, Release);
            }
            let i = self.nindex.load(Relaxed) as usize;
            if i >= self.indexes.len() {
                return None;
            }
            let cap = keys.min(n as usize).next_power_of_two().max(1);
            // Chunks the block's free list takes back when the index is forgotten (`forget_indexes`), zeroed as an
            // allocation is.
            let (ob, rb) = (4 * (cap + 1), 4 * (n as usize).max(1));
            let (offs_at, rows_at) = if self.flags.load(Acquire) & REWRITTEN != 0 {
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
            let mut key = [0u16; MAX_KEY];
            key[..cols.len()].copy_from_slice(cols);
            (*ix).key = key;
            (*ix).mask = (cap - 1) as u64;
            Span::set(&raw mut (*ix).offs, offs, cap + 1);
            Span::set(&raw mut (*ix).rows, rows, n as usize);
            self.nindex.store(i as u32 + 1, Release);
            Some(IndexRoom { index: i, offs: block.offset_of(offs), rows: block.offset_of(rows), mask: (cap - 1) as u64, rows_n: n, key: (*ix).key, ncols: cols.len() })
        }
    }

    /// Fills index `room` on the host: each row's bucket by its key's hash, the buckets' starts, then the rows in id
    /// order within each bucket, as the device's builder places them.
    ///
    /// # Safety
    /// `room` is this relation's, reserved and not yet built, and nothing reads it meanwhile.
    pub unsafe fn build_index(&self, room: &IndexRoom) {
        let ix = &self.indexes.as_slice()[room.index];
        let n = room.rows_n as usize;
        // SAFETY: the caller's: the room's spans are ours to write.
        let (offs, rows) = unsafe {
            (std::slice::from_raw_parts_mut(ix.offs.as_slice().as_ptr().cast_mut(), ix.offs.len()),
             std::slice::from_raw_parts_mut(ix.rows.as_slice().as_ptr().cast_mut(), ix.rows.len()))
        };
        offs.fill(0);
        let base = self.base_rows;
        let key = &room.key[..room.ncols];
        let bucket: Vec<u32> = (0..n as u32).map(|i| (key.iter().fold(crate::SEED, |h, &c| crate::mix(h, self.rows.get(i, c as usize))) & room.mask) as u32).collect();
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

    /// Bytes this relation's values, order, indexes and bits take (what a reader may touch).
    pub fn bytes(&self) -> usize {
        let ix: usize =
            self.indexes.as_slice().iter().map(|i| i.offs.len() * 4 + i.rows.len() * 4).sum();
        self.rows.value_bytes() + self.dead.len() * 8 + ix
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
    for p in 0..m {
        let i = ids[p] as usize;
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
        let ix = self.top.index(i);
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
