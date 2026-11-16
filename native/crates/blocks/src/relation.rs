//! A relation: its rows, its kills, and, once frozen, everything reads need, in one descriptor.
//!
//! **Freeze** turns rows written in any order (by many threads, or a GPU's claims) into a relation read in a
//! fixed order:
//!
//! - **Set semantics.** It drops duplicate rows, and in a delta it also drops rows its base still holds live.
//! - **The canonical order.** The live rows, sorted by the relation's order key (its declared columns first,
//!   then the rest). This is a permutation; rows are not moved.
//! - **Hash indexes** on the declared keys. Each key's rows are grouped in canonical order, so a probe's answer
//!   comes in a fixed order too.
//! - **Dead bits and statistics.** The dead bits are the kills, plus the duplicates freeze dropped; the
//!   statistics are each column's minimum and maximum.
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
pub const MAX_KEY: usize = 4;

structure! {
    /// A hash bucket: a key's hash and its rows' place in the index's `rows` (len 0: empty).
    pub struct Bucket { pub hash: u64, pub start: u32, pub len: u32 }
}

structure! {
    pub struct Index {
        pub ncols: u32,
        pub pad: u32,
        pub key: [u16; 4],
        pub mask: u64,
        pub buckets: Span<Bucket>,
        /// Row ids, grouped by key and in canonical order within a key.
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
        /// Its own live rows in canonical order.
        pub order: Span<u32>,
        pub indexes: Span<Index>,
        pub stats: Span<ColStats>,
        /// An open relation's set test (`insert_unique`): slots holding row id + 1.
        pub pending: Span<u32>,
        pub pend_used: AtomicU64,
    }
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
        let p: *mut Relation = block.alloc_t(1);
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
            Rows::init(&raw mut (*p).rows, block, &spec.widths, spec.min_log);
            Rows::init(&raw mut (*p).killed, block, &[4], 6);
            &*p
        }
    }

    pub fn is_frozen(&self) -> bool {
        self.flags.load(Acquire) & FROZEN != 0
    }

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

    #[inline]
    pub fn dead_bit(&self, id: u32) -> bool {
        let w = (id >> 6) as usize;
        let d = self.dead.as_slice();
        w < d.len() && (d[w] >> (id & 63)) & 1 == 1
    }

    pub fn index(&self, i: usize) -> &Index {
        &self.indexes.as_slice()[i]
    }

    /// One relation's rows for `key` on index `i`, before liveness and key equality: a hash group.
    pub fn group(&self, i: usize, h: u64) -> &[u32] {
        let ix = self.index(i);
        let b = ix.buckets.as_slice();
        if b.is_empty() {
            return &[];
        }
        let mut s = (h & ix.mask) as usize;
        for _ in 0..b.len() {
            let x = &b[s];
            if x.len == 0 {
                return &[];
            }
            if x.hash == h {
                return &ix.rows.as_slice()[x.start as usize..(x.start + x.len) as usize];
            }
            s = (s + 1) & ix.mask as usize;
        }
        &[]
    }

    /// Bytes this relation's values, order, indexes and bits take (what a reader may touch).
    pub fn bytes(&self) -> usize {
        let ix: usize =
            self.indexes.as_slice().iter().map(|i| i.buckets.len() * size_of::<Bucket>() + i.rows.len() * 4).sum();
        self.rows.value_bytes() + self.order.len() * 4 + self.dead.len() * 8 + ix
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
/// module's comment). `base` is the delta's base. After this, the relation only changes by a new delta.
pub fn freeze(block: &Block, rel: &Relation, spec: &Spec, base: Option<&Relation>) -> FreezeStats {
    assert!(!rel.is_frozen());
    let a = rel.arity as usize;
    let n = rel.rows.len() as usize;
    let before = block.used();

    // Dead bits: its kills, over the base's rows and its own.
    let total = rel.base_rows as usize + n;
    let words: *mut u64 = block.alloc_t(total.div_ceil(64).max(1));
    let dead = unsafe { std::slice::from_raw_parts_mut(words, total.div_ceil(64).max(1)) };
    for k in 0..rel.killed.len() {
        let id = rel.killed.get(k, 0) as usize;
        dead[id >> 6] |= 1 << (id & 63);
    }

    // The canonical order of its own rows, read once into a sort key.
    let key: Vec<usize> = spec.order.iter().map(|&c| c as usize).chain((0..a).filter(|c| !spec.order.contains(&(*c as u16)))).collect();
    let mut vals = vec![0i64; n * a];
    for i in 0..n {
        for (j, &c) in key.iter().enumerate() {
            vals[i * a + j] = rel.rows.get(i as u32, c);
        }
    }
    let row = |i: usize| &vals[i * a..i * a + a];
    let mut ids: Vec<u32> = (0..n as u32).filter(|&i| dead[(rel.base_rows + i) as usize >> 6] >> ((rel.base_rows + i) & 63) & 1 == 0).collect();
    ids.sort_unstable_by(|&x, &y| row(x as usize).cmp(row(y as usize)));

    // Set semantics: a row once, and in a delta, not a row its base still holds live.
    let mut order = Vec::with_capacity(ids.len());
    let mut dropped = 0u64;
    let base_live = |r: &[i64]| -> bool {
        let Some(b) = base else { return false };
        let bo = b.order.as_slice();
        let found = bo.binary_search_by(|&bid| {
            let mut o = Ordering::Equal;
            for (j, &c) in key.iter().enumerate() {
                o = b.rows.get(bid, c).cmp(&r[j]);
                if o != Ordering::Equal {
                    break;
                }
            }
            o
        });
        match found {
            Ok(p) => {
                let bid = bo[p];
                !b.dead_bit(bid) && dead[bid as usize >> 6] >> (bid & 63) & 1 == 0
            }
            Err(_) => false,
        }
    };
    // Decided against the kills alone: a duplicate dropped here is one of its own rows, never a base row.
    let drops: Vec<bool> = ids
        .iter()
        .enumerate()
        .map(|(p, &i)| (p > 0 && row(ids[p - 1] as usize) == row(i as usize)) || base_live(row(i as usize)))
        .collect();
    for (&i, drop) in ids.iter().zip(drops) {
        if drop {
            let id = (rel.base_rows + i) as usize;
            dead[id >> 6] |= 1 << (id & 63);
            dropped += 1;
        } else {
            order.push(rel.base_rows + i);
        }
    }
    let op: *mut u32 = block.alloc_t(order.len().max(1));
    unsafe { std::ptr::copy_nonoverlapping(order.as_ptr(), op, order.len()) };

    // Indexes: each key's rows grouped, in canonical order within a group.
    let ixp: *mut crate::relation::Index = block.alloc_t(spec.indexes.len().max(1));
    for (x, cols) in spec.indexes.iter().enumerate() {
        assert!(cols.len() <= MAX_KEY);
        let mut keyed: Vec<(u64, u32)> = order
            .iter()
            .map(|&id| {
                let k: Vec<i64> = cols.iter().map(|&c| rel.own(id, c as usize)).collect();
                (key_hash(&k), id)
            })
            .collect();
        keyed.sort_by_key(|&(h, _)| h); // stable: canonical order within a key
        let groups = keyed.chunk_by(|x, y| x.0 == y.0).count();
        let cap = (groups * 2).next_power_of_two().max(2);
        let bp: *mut Bucket = block.alloc_t(cap);
        let rp: *mut u32 = block.alloc_t(keyed.len().max(1));
        let buckets = unsafe { std::slice::from_raw_parts_mut(bp, cap) };
        let mut start = 0usize;
        for g in keyed.chunk_by(|x, y| x.0 == y.0) {
            let h = g[0].0;
            let mut s = (h as usize) & (cap - 1);
            while buckets[s].len != 0 {
                s = (s + 1) & (cap - 1);
            }
            buckets[s] = Bucket { hash: h, start: start as u32, len: g.len() as u32 };
            for (j, &(_, id)) in g.iter().enumerate() {
                unsafe { *rp.add(start + j) = id };
            }
            start += g.len();
        }
        unsafe {
            let ix = ixp.add(x);
            (*ix).ncols = cols.len() as u32;
            for (j, &c) in cols.iter().enumerate() {
                (*ix).key[j] = c;
            }
            (*ix).mask = (cap - 1) as u64;
            Span::set(&raw mut (*ix).buckets, bp, cap);
            Span::set(&raw mut (*ix).rows, rp, keyed.len());
        }
    }

    // Statistics: each column's range over its live rows.
    let sp: *mut ColStats = block.alloc_t(a);
    for c in 0..a {
        let (mut lo, mut hi) = (i64::MAX, i64::MIN);
        for &id in &order {
            let v = rel.own(id, c);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        unsafe { *sp.add(c) = ColStats { min: lo, max: hi } };
    }

    unsafe {
        Span::set(&rel.dead, words, total.div_ceil(64).max(1));
        Span::set(&rel.order, op, order.len());
        Span::set(&rel.indexes, ixp, spec.indexes.len());
        Span::set(&rel.stats, sp, a);
    }
    rel.nindex.store(spec.indexes.len() as u32, Relaxed);
    rel.live.store(order.len() as u32, Relaxed);
    rel.flags.fetch_or(FROZEN, Release);
    FreezeStats { rows: n as u64, live: order.len() as u64, dropped, bytes: (block.used() - before) as u64 }
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

    pub fn arity(&self) -> usize {
        self.top.arity as usize
    }

    /// Every id, live or not: `0 .. total`.
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
        assert!(self.top.is_frozen(), "an ordered read needs a frozen relation");
        let own = self.top.order.as_slice();
        let Some(b) = self.base else { return own.to_vec() };
        let based: Vec<u32> = b.order.as_slice().iter().copied().filter(|&id| self.live(id)).collect();
        merge(self, &based, own)
    }

    pub fn live_len(&self) -> usize {
        let own = self.top.order.len();
        match self.base {
            None => own,
            Some(b) => own + b.order.as_slice().iter().filter(|&&id| self.live(id)).count(),
        }
    }

    /// The live rows whose index-`i` columns equal `key`, in canonical order.
    pub fn probe(&self, i: usize, key: &[i64]) -> Vec<u32> {
        let h = key_hash(key);
        let cols: Vec<usize> = {
            let ix = self.top.index(i);
            ix.key[..ix.ncols as usize].iter().map(|&c| c as usize).collect()
        };
        let matches = |id: u32| self.live(id) && cols.iter().zip(key).all(|(&c, &k)| self.get(id, c) == k);
        let own: Vec<u32> = self.top.group(i, h).iter().copied().filter(|&id| matches(id)).collect();
        let Some(b) = self.base else { return own };
        let based: Vec<u32> = b.group(i, h).iter().copied().filter(|&id| matches(id)).collect();
        merge(self, &based, &own)
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

fn merge(v: &View, a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if v.cmp(a[i], b[j]) != Ordering::Greater {
            out.push(a[i]);
            i += 1;
        } else {
            out.push(b[j]);
            j += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}
