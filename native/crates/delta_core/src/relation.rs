//! A relation's format and its reads (moved from `rerail_blocks::relation`, whose writers make, continue, freeze and
//! index relations): its rows, its kills, its indexes and orders, in one descriptor in its block.

use crate::flat::RelSpan;
use crate::rows::Rows;
use crate::structure;
use core::sync::atomic::{AtomicI64, AtomicU16, AtomicU32, AtomicU64, Ordering::*};

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
        /// Bucket `b`'s rows are `rows[offs[b]..offs[b + 1]]`: every key whose hash falls in `b`, in id
        /// order. A reader checks the key (4 bytes a bucket, about one key a bucket; no hash kept). In a sorted order
        /// (`ORDER`), `rows` is every row in the key's order, and `offs` is empty.
        pub offs: RelSpan<u32>,
        pub rows: RelSpan<u32>,
        /// A hashed index's: each place's mark (decision 0671), as many as `rows`, after them in their chunk: its
        /// key's tag (`mark_tag`) and `MARK_SAME`. Empty in a sorted order.
        pub marks: RelSpan<u32>,
        /// A sorted order's (`ORDER`, `ORDER_ROWS`; decision 0478 step 5b): the leading column's value at every
        /// `FENCE`th row of the order, each row's tag (an order tagged by its lead's hash), each row's key print (an
        /// order probed by whole keys). Empty when the order keeps none.
        pub fences: RelSpan<i64>,
        pub tags: RelSpan<u16>,
        pub prints: RelSpan<u8>,
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

/// `Index::marks`' bit 0 (decision 0671): the place's key is its bucket's previous place of its tag's, that place
/// among the `MARK_REACH` before it. Clear where there is none or it holds another key. A walk that met that place
/// has its verdict; one that did not (its first place of the tag, wherever in the group the walk starts) compares.
pub const MARK_SAME: u32 = 1;
/// The places before one a builder looks back over for its tag (decision 0671).
pub const MARK_REACH: usize = 8;

/// A key's tag, from its hash (`key_hash`): its high 32 bits but bit 0, which is `MARK_SAME`'s. A bucket is the
/// hash's low bits, so the two are independent.
#[inline]
pub fn mark_tag(h: u64) -> u32 {
    (h >> 32) as u32 & !MARK_SAME
}

/// A hashed index's rows' chunk for `n` rows: the rows, then their marks.
#[inline]
pub fn index_rows_bytes(n: usize) -> usize {
    8 * n.max(1)
}

/// The marks of an index's places (decision 0671): `tag(t)` of place `t`'s key, `same(t, u)` whether places `t` and
/// `u` hold one key. Each place's tag, and `MARK_SAME` where its bucket's previous place of the tag is among the
/// `MARK_REACH` before it and holds its key. The device's builder writes them as these are.
pub fn fill_marks(offs: &[u32], marks: &mut [u32], tag: &dyn Fn(usize) -> u32, same: &dyn Fn(usize, usize) -> bool) {
    for (t, m) in marks.iter_mut().enumerate() {
        *m = tag(t);
    }
    for b in 0..offs.len().saturating_sub(1) {
        let (lo, hi) = (offs[b] as usize, offs[b + 1] as usize);
        for t in lo..hi {
            let prev = (t.saturating_sub(MARK_REACH).max(lo)..t).rev().find(|&u| marks[u] & !MARK_SAME == marks[t] & !MARK_SAME);
            marks[t] |= u32::from(prev.is_some_and(|u| same(u, t)));
        }
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
/// `Index::ordered`: a node-keyed index of one column (`try_add_node_bits`, decision 0694): `rows` its groups'
/// directory, `fences` their bits, `mask` the rows it holds. No hashed index's or order's finder finds it.
pub const NODE_BITS: u32 = 4;
/// `Index::ordered`: a membership filter on `key` (`try_add_filter`, decision 0706 E2): `fences` its words, in
/// `bloom`'s format (decision 0488, the reads written once in CLIF), `mask` the rows it holds. A key it answers "no" for is in no row; one it answers
/// "maybe" for is probed. No hashed index's or order's finder finds it.
pub const FILTER: u32 = 5;
/// A membership filter's bits a row (`try_add_filter`): a power of two of `bloom`'s words at least this many bits a
/// row, so about one absent key in 400 passes (`bloom::K` bits a key).
pub const FILTER_BITS: usize = 16;

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
        /// The column its rows lie sorted on, `okey[0]`, when `nokey` is 1; none when it is 0 (decision 0678 step 8):
        /// set at create, and at freeze to the column its rows came in order on (`sort_col`). The other entries are
        /// unused, kept for the descriptor's layout.
        pub okey: [AtomicU16; 8],
        pub nokey: AtomicU32,
        pub nindex: AtomicU32,
        pub rows: Rows,
        /// Row ids killed: a delta's of its base, or of its own.
        pub killed: Rows,
        pub dead: RelSpan<u64>,
        pub indexes: RelSpan<Index>,
        pub stats: RelSpan<ColStats>,
        /// An open relation's set test (`insert_unique`): slots holding row id + 1.
        pub pending: RelSpan<u32>,
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
        pub known: RelSpan<Known>,
        pub known_kept: AtomicU64,
        /// A frozen relation's dead rows (bit `id`; decision 0478 step 5d), given to it once and given back at its
        /// release; the bytes of their chunk. Its kills delta (`try_kills_at`) reads them here.
        pub own_dead: RelSpan<u64>,
        pub own_dead_kept: AtomicU64,
        /// A frozen delta's change (decision 0678 step 4): the base's ids whose liveness it changes (its kills of
        /// live base rows), ascending, each once. A change is read from these and its own rows alone, never from the
        /// base's liveness. Empty for a full relation, an open one, and a delta that kills nothing.
        ///
        /// A trace's layer (`layer_write`, decision 0686 B2) keeps its kills here alone: rows of the layers below it
        /// that it kills, ascending, each live below. It keeps no dead bits (`dead` empty): a row is live in a state
        /// when no layer above its own, up to the state's, names it here.
        pub changed: RelSpan<u32>,
        /// A far base's block (`FAR_BASE`, `try_continue_far`): its block id, which a reader resolves through its
        /// launch's descriptor table, `base_off` being the base's descriptor's offset in it (a reference by block id
        /// and offset, decision 0686 B1); `NO_BLOCK` for none.
        pub base_block: u32,
        pub pad_far: u32,
        /// A trace's layer's statistics (P§11.2, O§10 "stats: each layer's sketch"): a directory of a `RelSpan<u8>` a
        /// column, each that column's sketch in its kept form (what its `SketchW` settles), of the state the layer
        /// ends (the state below's sketch with the layer's rows added and its kills taken), each settled by an item of
        /// its own (`layer_write::sketch_change`). Empty for any other.
        pub sketches: RelSpan<u8>,
    }
}

/// `Relation::base_block` of a relation whose base is in no other block.
pub const NO_BLOCK: u32 = u32::MAX;

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
    /// Its marks' offset in the block (decision 0671): after its rows, as many.
    pub marks: usize,
    pub mask: u64,
    pub rows_n: u32,
    /// Its key's columns: `ncols` of `key`.
    pub key: [u16; MAX_KEY],
    pub ncols: usize,
    /// Its first row (`Index::from`).
    pub from: u32,
}

impl Relation {
    /// Whether its base lies in another block (`try_link`).
    #[inline]
    pub fn far_base(&self) -> bool {
        self.flags.load(Relaxed) & FAR_BASE != 0
    }

    /// The relation whose descriptor lies at `at` of a block's bytes `bytes` (a pinned block's, read in place):
    /// `None` when it does not fit there or is misaligned. Its spans are relative to itself, so it reads the same
    /// wherever the block is mapped.
    pub fn at(bytes: &[u8], at: usize) -> Option<&Relation> {
        let end = at.checked_add(size_of::<Relation>())?;
        if end > bytes.len() || at == 0 || !(bytes.as_ptr() as usize + at).is_multiple_of(align_of::<Relation>()) {
            return None;
        }
        // SAFETY: in bounds and aligned; a descriptor is plain words, read as atomics where it changes.
        Some(unsafe { &*bytes.as_ptr().add(at).cast::<Relation>() })
    }

    /// The block id and descriptor offset of a far base (`try_continue_far`).
    pub fn far(&self) -> Option<(u32, usize)> {
        (self.far_base() && self.base_block != NO_BLOCK).then(|| (self.base_block, self.base_off.load(Acquire) as usize))
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
        (self.continues() && off != 0 && !self.far_base()).then(|| unsafe { &*(core::ptr::from_ref(self).cast::<u8>().offset(off as isize).cast::<Relation>()) })
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
        self.get_at(None, id, c)
    }

    /// `get`, a far base's descriptor read in the block whose first byte is at `far` (`RelationW::get_far`).
    pub fn get_at(&self, far: Option<*const u8>, id: u32, c: usize) -> i64 {
        let mut rel = self;
        loop {
            let off = rel.base_off.load(Relaxed);
            if id >= rel.base_rows || off == 0 {
                break;
            }
            let jump = rel.jump_off.load(Relaxed);
            rel = if jump != 0 && id < rel.jump_rows {
                // SAFETY: its jump lies in its block `jump` bytes from it, an ancestor its holder keeps.
                unsafe { &*core::ptr::from_ref(rel).cast::<u8>().offset(jump as isize).cast::<Relation>() }
            } else if rel.far_base() {
                let far = far.expect("a relation whose base lies in another block, read with that block");
                // SAFETY: its base's descriptor lies `off` bytes into `far`, kept while it is (its holder's).
                unsafe { &*far.add(off as usize).cast::<Relation>() }
            } else {
                // SAFETY: a continuation's base lies in its block `off` bytes from it, kept while it is (its holder's).
                unsafe { &*core::ptr::from_ref(rel).cast::<u8>().offset(off as isize).cast::<Relation>() }
            };
        }
        rel.rows.get(id.wrapping_sub(rel.base_rows), c)
    }

    /// The chunk a kills delta's `n` dead words take (`try_kills`).
    pub fn kills_bytes(n: usize) -> usize {
        n.max(1) * size_of::<u64>()
    }

    /// Where the column directories lie in a descriptor's chunk.
    pub const DIRS_AT: usize = size_of::<Relation>().next_multiple_of(64);

    /// A descriptor's chunk: the relation, and the directories of its `arity` columns and its killed rows' one.
    pub fn descriptor_bytes(arity: usize) -> usize {
        Relation::DIRS_AT + (arity + 1) * size_of::<crate::rows::ColDir>()
    }

    /// The column its rows lie sorted on, when they lie sorted on one.
    #[inline]
    pub fn sort_col(&self) -> Option<usize> {
        (self.nokey.load(Relaxed) > 0).then(|| self.okey[0].load(Relaxed) as usize)
    }

    pub fn set_sort_col(&self, c: Option<usize>) {
        self.okey[0].store(c.unwrap_or(0) as u16, Relaxed);
        self.nokey.store(u32::from(c.is_some()), Relaxed);
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
    pub fn ids(&self) -> core::ops::Range<u32> {
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

    /// The membership filter on `cols` over its rows `0..rows` (`try_add_filter`), built: its number.
    pub fn find_filter(&self, cols: &[u16], rows: u32) -> Option<usize> {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: as `find_index`'s.
        let built = |i: usize| unsafe { core::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| ix[i].ordered == FILTER && built(i) == cols.len() && ix[i].key[..cols.len()] == *cols && ix[i].mask == u64::from(rows));
        core::sync::atomic::fence(Acquire);
        found
    }

    /// Whether filter `i` (`find_filter`) may hold the key `bloom::key` places at `h`: `false` is certain (no row has
    /// it), `true` is to be probed.
    #[inline]
    pub fn filter_may_hold(&self, i: usize, h: u64) -> bool {
        let f = self.index_at(i).fences.as_slice();
        let (w, mask) = crate::bloom::place(h, f.len());
        f[w] as u64 & mask == mask
    }

    /// The node-keyed index on `col` over its rows `0..rows` (`try_add_node_bits`), built: its number.
    pub fn find_node_bits(&self, col: u16, rows: u32) -> Option<usize> {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: as `find_index`'s.
        let built = |i: usize| unsafe { core::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| ix[i].ordered == NODE_BITS && built(i) == 1 && ix[i].key[0] == col && ix[i].mask == u64::from(rows));
        core::sync::atomic::fence(Acquire);
        found
    }

    /// Whether node-keyed index `i` (`find_node_bits`) holds value `v`: its document found (`hint` the last one, tried
    /// first, then the one after it), and then its slot's entry and bit, or a search of its lows.
    #[inline]
    pub fn node_bits_has(&self, i: usize, v: i64, hint: &mut u32) -> bool {
        use crate::packed::ID_SPLIT;
        let ix = self.index_at(i);
        let dir = ix.rows.as_slice();
        let (hi, low) = ((v >> 32) as u32, v as u32);
        let docs = dir[0] as usize;
        let at = |k: usize| 1 + 4 * k;
        let h = *hint as usize;
        let k = if h < docs && dir[at(h)] == hi {
            h
        } else if h + 1 < docs && dir[at(h + 1)] == hi {
            h + 1
        } else {
            let (mut lo, mut top) = (0usize, docs);
            loop {
                if lo >= top {
                    return false;
                }
                let mid = (lo + top) / 2;
                match dir[at(mid)].cmp(&hi) {
                    core::cmp::Ordering::Less => lo = mid + 1,
                    core::cmp::Ordering::Greater => top = mid,
                    core::cmp::Ordering::Equal => break mid,
                }
            }
        };
        *hint = k as u32;
        let (first, second, start) = (dir[at(k) + 1], dir[at(k) + 2], dir[at(k) + 3] as usize);
        if first == u32::MAX {
            return dir[start..start + second as usize].binary_search(&low).is_ok();
        }
        let s = (low >> ID_SPLIT).wrapping_sub(first);
        if s >= second {
            return false;
        }
        let e = start + 2 * s as usize;
        let (off, fw) = (dir[e], dir[e + 1]);
        let x = low & ((1 << ID_SPLIT) - 1);
        let word = (x / 64).wrapping_sub(fw >> 16);
        word < fw & 0xffff && ix.fences.as_slice()[(off + word) as usize] >> (x % 64) & 1 != 0
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
        let built = |i: usize| unsafe { core::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| ix[i].ordered == 0 && built(i) == cols.len() && ix[i].key[..cols.len()] == *cols && ix[i].from == from && ix[i].rows.len() == rows as usize);
        core::sync::atomic::fence(Acquire);
        found
    }

    /// Its dead rows' bits as given (`adopt_dead`): empty when none were.
    pub fn own_dead(&self) -> &[u64] {
        if self.own_dead_kept.load(Acquire) == 0 {
            return &[];
        }
        self.own_dead.as_slice()
    }

    /// Its row ranges known to be in an order (`Known`): none until `try_set_known`.
    pub fn known(&self) -> &[Known] {
        if self.known_kept.load(Acquire) == 0 {
            return &[];
        }
        self.known.as_slice()
    }

    /// The sorted order on `cols` over `rows` rows (decision 0478 step 5b), built: its number.
    pub fn find_order(&self, cols: &[u16], rows: u32) -> Option<usize> {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: as `find_index`'s.
        let built = |i: usize| unsafe { core::ptr::read_volatile(&raw const ix[i].ncols) } as usize;
        let found = (0..n).find(|&i| (ix[i].ordered == ORDER || ix[i].ordered == ORDER_ROWS) && built(i) == cols.len() && ix[i].key[..cols.len()] == *cols && Self::order_rows(&ix[i]) == rows as usize);
        core::sync::atomic::fence(Acquire);
        found
    }

    /// The sorted orders built, by number: every one `find_order` can find.
    pub fn orders(&self) -> impl Iterator<Item = usize> + '_ {
        let n = (self.nindex.load(Acquire) as usize).min(self.indexes.len());
        let ix = self.indexes.as_slice();
        // SAFETY: as `find_index`'s.
        let built = move |i: usize| unsafe { core::ptr::read_volatile(&raw const ix[i].ncols) } != 0;
        // Each order's index is read after its flag (the fence), as it is yielded: no list of them gathered.
        (0..n).filter(move |&i| {
            let found = (ix[i].ordered == ORDER || ix[i].ordered == ORDER_ROWS) && built(i);
            core::sync::atomic::fence(Acquire);
            found
        })
    }

    /// The rows a sorted order holds.
    pub fn order_rows(ix: &Index) -> usize {
        if ix.ordered == ORDER_ROWS {
            ((ix.mask & 0xffff_ffff) - (ix.mask >> 32)) as usize
        } else {
            ix.rows.len()
        }
    }

    /// A dead entry whose room (kept: its block has no free list) holds `buckets` buckets and `n` rows: its number.
    pub fn dead_room(&self, buckets: usize, n: usize) -> Option<usize> {
        let ix = self.indexes.as_slice();
        let m = (self.nindex.load(Relaxed) as usize).min(ix.len());
        (0..m).find(|&i| ix[i].state == DEAD && ix[i].offs.len() >= buckets && ix[i].rows.len() >= n.max(1))
    }

    /// Index `room` built (by the host or the device): `find_index` finds it from now on.
    ///
    /// # Safety
    /// Its builder has finished.
    pub unsafe fn index_built(&self, room: &IndexRoom) {
        let ix = core::ptr::from_ref(&self.indexes.as_slice()[room.index]).cast_mut();
        core::sync::atomic::fence(Release);
        // SAFETY: the caller's; `ncols` is a u32 written once, after the index (the fence above).
        unsafe { core::ptr::write_volatile(&raw mut (*ix).ncols, room.ncols as u32) };
    }

    /// Marks its rows' values as changed in place (`REWRITTEN`): before any index is added.
    pub fn rewritten(&self) {
        debug_assert!(self.indexes.is_empty(), "marked rewritten after an index was added");
        self.flags.fetch_or(REWRITTEN, Release);
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
            ix.iter().map(|i| i.rows.len() * 4 + i.marks.len() * 4).sum(),
            self.dead.len() * 8,
            size_of::<Relation>() + self.rows.ncols as usize * size_of::<crate::rows::ColDir>() + size_of::<crate::rows::ColDir>(),
        ]
    }

    /// Bytes its block holds for it: the chunks made (written or not), the order, the indexes and the bits.
    pub fn held_bytes(&self) -> usize {
        let cap = crate::rows::capacity(self.rows.nchunks.load(Relaxed), self.rows.min_log) as usize;
        let width: usize = (0..self.rows.ncols as usize).map(|c| self.rows.col(c).width as usize).sum();
        let ix: usize =
            self.indexes.as_slice().iter().map(|i| i.offs.len() * 4 + i.rows.len() * 4 + i.marks.len() * 4).sum();
        cap * width + self.dead.len() * 8 + ix + size_of::<Relation>()
    }

    /// Bytes this relation's values, order, indexes and bits take (what a reader may touch: a continuation's chain's
    /// rows too, decision 0384 step 7).
    pub fn bytes(&self) -> usize {
        let ix: usize =
            self.indexes.as_slice().iter().map(|i| i.offs.len() * 4 + i.rows.len() * 4 + i.marks.len() * 4).sum();
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

/// The positions of `rel`'s own rows (sorted in place) whose columns `cols`, the order key's first ones, equal
/// `key`: one range, by binary search on each leading column in turn.
pub fn leading(rel: &Relation, cols: &[u16], key: &[i64]) -> core::ops::Range<u32> {
    let (mut lo, mut hi) = (0u32, rel.rows.len());
    for oc in rel.sort_col().into_iter().take(cols.len()).map(|c| c as u16) {
        let k = key[cols.iter().position(|&c| c == oc).expect("a leading column")];
        let a = rel.rows.lower_bound(oc as usize, lo, hi, k);
        hi = rel.rows.upper_from(oc as usize, a, hi, k);
        lo = a;
    }
    lo..hi
}
