//! A layer of a trace, read where it lies (decision 0686, P§12; moved from `rerail_blocks::layer`, whose writers lay a
//! layer out, consolidate and merge it): its block's header at `BODY_AT` (`LayerHead`), its three relations (its rows,
//! the partitions it changed, its kills by partition), its own rows in its shape's encoding (`OwnRows`), and every
//! read of one layer: a partition's rows and kills, a key's range by its order, its sorted orders, its packed
//! permutation index or its Bloom filters, and its sketches' readings.

use crate::block::{BODY_AT, Id};
use crate::bloom::{self, Bloom};
use crate::hash::row_digest;
use crate::pod::{set_word, word};
use crate::relation::{Relation, leading};
use crate::runs::{MAX_IX, ShapeW};
use core::cmp::Ordering;

pub const LAYER_MAGIC: u32 = u32::from_le_bytes(*b"LAY0");
/// A layer being written.
pub const OPEN: u32 = 1;
/// A layer committed: its header was written last.
pub const COMMITTED: u32 = 2;

/// An index's kind, as its shape holds it: a sorted order of the layer's relation (its ids sorted by the key) and
/// beside it the leading value of every `order::FENCE`th (fences) and each one's 8-bit print of its whole key
/// (prints), the search `blocks::order::find` reads; a hashed index of the relation (its buckets); a Bloom filter of
/// the key (`blocks::bloom`'s hash and mask, its words a power of two). One format each, the existing ones.
pub const IX_ORDER: u32 = 1;
pub const IX_FENCES: u32 = 2;
pub const IX_PRINTS: u32 = 4;
pub const IX_HASH: u32 = 8;
pub const IX_BLOOM: u32 = 16;
/// Packed's permutation index on one column (`packed::PIndex`), in a packed layer's encoding.
pub const IX_PERM: u32 = 32;

/// A shape's encoding, as its blocks hold it (P§12's `Encoding`).
pub const ENC_OPEN: u32 = 0;
pub const ENC_PACKED: u32 = 1;
pub const ENC_RANGED: u32 = 2;
pub const ENC_DICT: u32 = 3;

/// A shape's retention, as its blocks hold it: run data (two states during a run, one at rest). A cache is not a
/// trace (P§11.3: one state, mutated in place).
pub const RETAIN_RUN: u32 = 0;

/// The one value a Bloom filter hashes for a key: the value itself for one column, else the key's `row_digest`.
pub fn bloom_key(key: impl ExactSizeIterator<Item = i64>) -> i64 {
    if key.len() == 1 {
        let mut k = key;
        k.next().expect("one value")
    } else {
        row_digest(key) as i64
    }
}

/// A layer block's header, at `BODY_AT`: where its relations' descriptors lie in the block, and its counts. Written
/// last (`state`): a layer is read only once committed.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LayerHead {
    /// The body's bytes (`gens::body_len`): the header's; the relations lie in the block's chunks past the body.
    pub len: u64,
    pub magic: u32,
    pub state: u32,
    /// The layer (P§12: "a layer is one `blocks::relation::Relation`"): its own rows are the rows it added, in the
    /// trace's canonical order, its change empty (its kills are `gone`'s); it continues the layer below by block id and
    /// offset (`Relation::try_continue_far`); its indexes and statistics are its own.
    pub rows: u64,
    /// `parts(part, digest, added, gone, first, kills_at)`: one row a partition the layer changed, sorted on the
    /// partition (P§11.2's partition index, "part → its rows, its digest and count"; its digests "a small relation in
    /// the batch"). The digest is the move of the partition's: its rows added's `row_digest`s less its rows gone's.
    /// Its own rows are `first .. first + added` of the layer's (they lie sorted on the partition), its kills
    /// `kills_at .. kills_at + gone` of `gone`'s: `restrict` reads "a range of the block, by the partition's index".
    pub parts: u64,
    /// `gone(part, id)`: the layer's kills by partition, ascending within each, one chunk (P§11.2's partition index,
    /// of the kills: `restrict` reads a partition's alone).
    pub gone: u64,
    pub added: u32,
    pub ngone: u32,
    pub nparts: u32,
    pub pad: u32,
    /// The sum of its partitions' digests.
    pub digest: u64,
    /// Its own rows in its shape's encoding (P§12; not Open): the `packed::Packed` or `ranged::Columns`'s offset in
    /// the block, which its readers read in place (`OwnRows`); 0 when Open, its rows its relation's.
    pub enc: u64,
    /// Each Bloom index's filter (`blocks::bloom`, in the block): its descriptor's offset; 0 for another kind.
    pub blooms: [u64; MAX_IX],
    pub shape: ShapeW,
}

// SAFETY: its fields are plain words, `#[repr(C)]`.
unsafe impl crate::Pod for LayerHead {}

impl LayerHead {
    /// A layer's header as its writer lays it out at `BODY_AT`, before its rows: open, its three relations' offsets.
    pub fn open(shape: ShapeW, rows: usize, parts: usize, gone: usize) -> LayerHead {
        LayerHead {
            len: LAYER_BODY as u64,
            magic: LAYER_MAGIC,
            state: OPEN,
            rows: rows as u64,
            parts: parts as u64,
            gone: gone as u64,
            added: 0,
            ngone: 0,
            nparts: 0,
            pad: 0,
            digest: 0,
            enc: 0,
            blooms: [0; MAX_IX],
            shape,
        }
    }

    /// The header of a layer being written, at `BODY_AT` of its block's bytes.
    pub fn of_open(bytes: &[u8]) -> LayerHead {
        let h: LayerHead = word(bytes, BODY_AT);
        assert!(h.magic == LAYER_MAGIC && h.state == OPEN, "a layer being written");
        h
    }

    /// Writes the header committed, whole, at `BODY_AT` of its block's bytes: the layer's last write.
    pub fn commit(mut self, bytes: &mut [u8]) {
        self.state = COMMITTED;
        set_word(bytes, BODY_AT, self);
    }
}

/// The body bytes a layer block's header takes.
pub const LAYER_BODY: usize = size_of::<LayerHead>();

/// A layer's own rows, read where they lie, in its shape's encoding (P§12): its relation's rows (Open), or a
/// `packed::Packed` or `ranged::Columns` in its block, its rows in the layer's order. Ids from `start`.
#[derive(Clone, Copy)]
pub enum OwnRows<'a> {
    Open(&'a Relation),
    Packed { p: &'a crate::packed::Packed, start: u32 },
    Ranged { cols: &'a crate::ranged::Columns, start: u32 },
}

impl<'a> OwnRows<'a> {
    /// A layer's own rows: its header's encoded form at `head.enc` of `bytes`, else its relation's.
    pub fn of(bytes: &'a [u8], head: &LayerHead, rows: &'a Relation) -> OwnRows<'a> {
        let start = rows.base_rows;
        let at = head.enc as usize;
        if at == 0 || head.shape.encoding == ENC_OPEN {
            return OwnRows::Open(rows);
        }
        assert!(at.is_multiple_of(8) && at + 64 <= bytes.len(), "a layer's encoded rows lie in its block");
        // SAFETY: its writer laid the encoded form out at `at` of the block (`layer_write::encode_open`), frozen since;
        // its spans are self-relative, read in place.
        unsafe {
            match head.shape.encoding {
                ENC_RANGED => OwnRows::Ranged { cols: &*bytes.as_ptr().add(at).cast::<crate::ranged::Columns>(), start },
                _ => OwnRows::Packed { p: &*bytes.as_ptr().add(at).cast::<crate::packed::Packed>(), start },
            }
        }
    }

    pub fn start(&self) -> u32 {
        match *self {
            OwnRows::Open(r) => r.base_rows,
            OwnRows::Packed { start, .. } | OwnRows::Ranged { start, .. } => start,
        }
    }

    /// Its rows' count.
    pub fn n(&self) -> u32 {
        match *self {
            OwnRows::Open(r) => r.rows.len(),
            OwnRows::Packed { p, .. } => p.len() as u32,
            OwnRows::Ranged { cols, .. } => cols.len() as u32,
        }
    }

    pub fn ids(&self) -> core::ops::Range<u32> {
        self.start()..self.start() + self.n()
    }

    /// Own row `id`'s value in column `c`.
    #[inline]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        match *self {
            OwnRows::Open(r) => r.own(id, c),
            OwnRows::Packed { p, start } => p.get((id - start) as usize, c),
            OwnRows::Ranged { cols, start } => cols.get((id - start) as usize, c),
        }
    }

    /// Its packed form, when it has one (`Perm` indexes lie there).
    pub fn packed(&self) -> Option<&'a crate::packed::Packed> {
        match *self {
            OwnRows::Packed { p, .. } => Some(p),
            _ => None,
        }
    }
}

/// A committed layer, read in place: its block's bytes, its header, its three relations, and its own rows where they
/// lie (`OwnRows`).
#[derive(Clone, Copy)]
pub struct Layer<'a> {
    pub bytes: &'a [u8],
    pub head: LayerHead,
    pub rows: &'a Relation,
    pub parts: &'a Relation,
    pub gone: &'a Relation,
    pub own: OwnRows<'a>,
}

impl<'a> Layer<'a> {
    /// The layer at `BODY_AT` of a block's bytes, committed.
    pub fn new(bytes: &'a [u8]) -> Result<Layer<'a>, &'static str> {
        if bytes.len() < BODY_AT + LAYER_BODY {
            return Err("not a layer block");
        }
        let head: LayerHead = word(bytes, BODY_AT);
        if head.magic != LAYER_MAGIC {
            return Err("not a layer block");
        }
        if head.state != COMMITTED {
            return Err("a layer read before it was committed");
        }
        let rel = |at: u64, what: &'static str| Relation::at(bytes, at as usize).filter(|r| r.is_frozen()).ok_or(what);
        let rows = rel(head.rows, "the layer's rows, frozen")?;
        let parts = rel(head.parts, "the layer's parts, frozen")?;
        let gone = rel(head.gone, "the layer's gone, frozen")?;
        Ok(Layer { bytes, head, rows, parts, gone, own: OwnRows::of(bytes, &head, rows) })
    }

    /// Its first own row's id: every id the layer below reads.
    pub fn start(&self) -> u32 {
        self.rows.base_rows
    }

    pub fn end(&self) -> u32 {
        self.start() + self.own.n()
    }

    /// Its own rows.
    pub fn n(&self) -> usize {
        self.own.n() as usize
    }

    /// The layer below: its block id, and its relation's offset there (the reference, `Relation::far`).
    pub fn below(&self) -> Option<(Id, usize)> {
        self.rows.far()
    }

    /// Own row `id`'s value in column `c`.
    #[inline]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        self.own.get(id, c)
    }

    /// Its kills, kept once: `gone`'s id column, by partition (`gone(part, id)` is sorted on both), so each
    /// partition's kills are ascending, and all of them when the trace has no partition. Its rows' relation records no
    /// change of its own.
    pub fn kills(&self) -> &'a [u32] {
        if self.gone.rows.is_empty() {
            return &[];
        }
        self.gone.rows.col_chunks_u32(1).next().expect("a layer's kills by partition, in one chunk")
    }

    /// Its kills as ascending runs, one a partition it changed (`parts`' `kills_at`, `gone`).
    pub fn kill_runs(&self) -> impl Iterator<Item = &'a [u32]> + '_ {
        let base = self.parts.base_rows;
        (0..self.parts.rows.len()).map(move |e| self.part_kills_at(base + e)).filter(|r| !r.is_empty())
    }

    /// Own row `id`'s partition: its value in the partition column (0 when the trace has none).
    pub fn part_of(&self, id: u32) -> i64 {
        self.head.shape.partition().map_or(0, |c| self.get(id, c))
    }

    /// Its kills among `ids` of rows whose partitions lie in `parts`, ascending within each partition: by the
    /// partition index, each partition's kills a search (the rows of a word, `View::chain_live_word`).
    pub fn kills_within(&self, parts: core::ops::RangeInclusive<i64>, ids: core::ops::Range<u32>, mut f: impl FnMut(u32)) {
        let (base, n) = (self.parts.base_rows, self.parts.rows.len());
        let mut e = partition_point(n as usize, |i| self.parts.own(base + i as u32, 0) < *parts.start()) as u32;
        while e < n && self.parts.own(base + e, 0) <= *parts.end() {
            let ks = self.part_kills_at(base + e);
            let from = ks.partition_point(|&x| x < ids.start);
            for &x in ks[from..].iter().take_while(|&&x| x < ids.end) {
                f(x);
            }
            e += 1;
        }
    }

    /// Partition `p`'s row of `parts`, the partition index: found by a search of the index's entries (one a
    /// partition the layer changed), not of the layer's rows.
    pub fn part_at(&self, p: i64) -> Option<u32> {
        let r = leading(self.parts, &[0], &[p]);
        (!r.is_empty()).then_some(self.parts.base_rows + r.start)
    }

    /// Partition `p`'s own rows: a range of ids, read from the partition index. Every own row when the trace has no
    /// partition.
    pub fn part_rows(&self, p: i64) -> core::ops::Range<u32> {
        if self.head.shape.partition().is_none() {
            return self.start()..self.end();
        }
        let Some(e) = self.part_at(p) else { return self.start()..self.start() };
        let (first, added) = (self.parts.own(e, 4) as u32, self.parts.own(e, 2) as u32);
        self.start() + first..self.start() + first + added
    }

    /// Partition `p`'s row of `parts`: its digest's move, rows added and rows gone.
    pub fn part_entry(&self, p: i64) -> Option<(u64, u32, u32)> {
        let e = self.part_at(p)?;
        Some((self.parts.own(e, 1) as u64, self.parts.own(e, 2) as u32, self.parts.own(e, 3) as u32))
    }

    /// Partition `p`'s kills, ascending: a slice of `gone`'s id column, by the partition index.
    pub fn part_kills(&self, p: i64) -> &'a [u32] {
        match self.part_at(p) {
            Some(e) => self.part_kills_at(e),
            None => &[],
        }
    }

    /// The kills of the partition at entry `e` of `parts`.
    pub fn part_kills_at(&self, e: u32) -> &'a [u32] {
        let (at, n) = (self.parts.own(e, 5) as usize, self.parts.own(e, 3) as usize);
        if n == 0 {
            return &[];
        }
        let ids = self.gone.rows.col_chunks_u32(1).next().expect("a layer's kills by partition, in one chunk");
        assert!(ids.len() >= at + n, "a layer's kills by partition, in one chunk");
        &ids[at..at + n]
    }

    /// Compares own row `id` with `key` on columns `cols`.
    pub fn cmp_key(&self, id: u32, cols: &[u16], key: &[i64]) -> Ordering {
        cols.iter().zip(key).map(|(&c, &k)| self.get(id, c as usize).cmp(&k)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
    }

    /// The own rows whose `cols` equal `key`, where `cols` is a prefix of the canonical order: a range of ids, found
    /// by binary search (its rows lie in that order). `None` when `cols` is not one.
    pub fn prefix_range(&self, cols: &[u16], key: &[i64]) -> Option<core::ops::Range<u32>> {
        self.prefix_range_counted(cols, key, self.start()..self.end(), &|| ())
    }

    /// `prefix_range` among the own ids `within` (a partition's, when restricted), calling `read` at each row it
    /// compares (a counted read's search steps).
    pub fn prefix_range_counted(&self, cols: &[u16], key: &[i64], within: core::ops::Range<u32>, read: &dyn Fn()) -> Option<core::ops::Range<u32>> {
        let order = self.head.shape.order();
        if cols.is_empty() || cols.len() > order.len() || order[..cols.len()].iter().zip(cols).any(|(&o, &c)| u16::from(o) != c) {
            return None;
        }
        assert!(self.start() <= within.start && within.end <= self.end(), "a search among the layer's own rows");
        let (s, n) = (within.start, (within.end - within.start) as usize);
        let cmp = |id: u32| {
            read();
            self.cmp_key(id, cols, key)
        };
        let lo = partition_point(n, |i| cmp(s + i as u32) == Ordering::Less);
        let hi = lo + partition_point(n - lo, |i| cmp(s + (lo + i) as u32) != Ordering::Greater);
        Some(s + lo as u32..s + hi as u32)
    }

    /// The own rows whose `cols` equal `key` by a sorted order whose key starts with `cols` (`Relation::add_order`):
    /// its ids, found by `order::find` (fences, prints, then bisection). `None` without such an order.
    pub fn order_range(&self, cols: &[u16], key: &[i64]) -> Option<&'a [u32]> {
        let rows = self.rows;
        let x = rows.orders().find(|&x| {
            let ix = rows.index_at(x);
            let k = &ix.key[..ix.ncols as usize];
            !cols.is_empty() && cols.len() <= k.len() && k[..cols.len()] == *cols
        })?;
        let ix = rows.index_at(x);
        let ids = ix.rows.as_slice();
        // The order prints its whole key (`ix.ncols` columns): a key of them all is found by the prints.
        let cmp = |p: usize, m: usize| self.cmp_key(ids[p], &cols[..m], &key[..m]);
        let (lo, hi) = crate::order::find(ids.len(), ix.fences.as_slice(), ix.tags.as_slice(), ix.prints.as_slice(), ix.ncols as usize, key, cmp);
        Some(&ids[lo..hi])
    }

    /// The own rows whose column `cols[0]` equals `key[0]` by a packed layer's permutation index on it (`Perm`): its
    /// positions' range in the index's `perm`. `None` without one.
    pub fn perm_range(&self, cols: &[u16], key: &[i64]) -> Option<(crate::col::Dec<'a>, core::ops::Range<usize>)> {
        let p = self.own.packed()?;
        if cols.len() != 1 {
            return None;
        }
        let ix = p.indexes.as_slice().iter().find(|ix| ix.col == u32::from(cols[0]))?;
        let keys = ix.keys.dec();
        let k = keys.lower_bound(0, keys.len(), key[0]);
        if k >= keys.len() || keys.get(k) != key[0] {
            return Some((ix.perm.dec(), 0..0));
        }
        let starts = ix.starts.dec();
        Some((ix.perm.dec(), starts.get(k) as usize..starts.get(k + 1) as usize))
    }

    /// Index `x`'s Bloom filter, for a Bloom index.
    pub fn bloom(&self, x: usize) -> Option<&'a Bloom> {
        let at = self.head.blooms[x];
        (at != 0).then(|| Bloom::at(self.bytes, at as usize).expect("a layer's Bloom filter"))
    }

    /// Whether a key on `cols` may be among its own rows: false only when a Bloom index on exactly `cols` says no.
    pub fn may_hold(&self, cols: &[u16], key: &[i64]) -> bool {
        let s = &self.head.shape;
        (0..s.nidx as usize).all(|x| {
            let k = s.index_key(x);
            s.index_kind(x) & IX_BLOOM == 0 || k.len() != cols.len() || k.iter().zip(cols).any(|(&a, &b)| u16::from(a) != b) || self.bloom(x).is_none_or(|b| b.may_hold_hashed(bloom::hash(bloom_key(key.iter().copied()))))
        })
    }

    /// Column `c`'s sketch's reading, of the state this layer ends: taken from its kept form's bytes in place
    /// (`sketch::reading`); `None` when it keeps none.
    pub fn reading(&self, c: usize) -> Option<crate::sketch::Reading> {
        crate::sketch::reading(crate::sketch::kept(self.rows, c)?)
    }
}

/// The first `i` in `0..n` for which `pred` is false (`pred` true then false).
pub fn partition_point(n: usize, pred: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if pred(mid) { lo = mid + 1 } else { hi = mid }
    }
    lo
}

