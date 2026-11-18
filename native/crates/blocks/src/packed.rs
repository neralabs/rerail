//! A packed relation: a frozen relation written again, exactly sized, in the fewest bits.
//!
//! Its rows are its live rows in canonical order, so a row's position is its place in that order:
//!
//! - no order array, no dead bits and no duplicates: they were the in-place form's;
//! - each column an `IntCol`, its encoding chosen by exact size;
//! - a key that is a prefix of the order key needs no index: its rows are one range, found by binary search
//!   on the leading columns;
//! - any other key gets a permutation index: its distinct values (sorted), where each value's positions start,
//!   and the positions, each packed in the bits it needs.
//!
//! It is the form of a block that leaves its run (kept, journaled, or put in a pinned slab for the GPU), and of
//! a roll-up: `pack` reads any view, so a base and its delta pack into one full relation.

use crate::block::Block;
use crate::col::{Choice, IntCol, choose};
use crate::refs::Span;
use crate::relation::{Spec, View, canonical_cols, same_row, sort_canonical};
use crate::scratch::Scratch;
use crate::structure;
use rayon::prelude::*;

/// Values (rows times columns) a relation holds before its columns are read, chosen and packed in parallel: under
/// it, handing work to other threads costs more than the work.
const PAR_MIN: usize = 1 << 16;

/// `f` for each of `0..k`, in parallel when `par`.
fn each<T: Send>(par: bool, k: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    if par { (0..k).into_par_iter().map(f).collect() } else { (0..k).map(f).collect() }
}

structure! {
    /// A permutation index on one column that does not lead the order.
    pub struct PIndex {
        pub col: u32,
        pub pad: u32,
        /// The column's distinct values, sorted.
        pub keys: IntCol,
        /// Where each key's positions start in `perm` (one more entry than keys).
        pub starts: IntCol,
        /// Positions, grouped by key, in canonical order within a key.
        pub perm: IntCol,
    }
}

structure! {
    pub struct Packed {
        pub arity: u32,
        pub n: u32,
        pub nokey: u32,
        pub nindex: u32,
        pub okey: [u16; 8],
        pub cols: Span<IntCol>,
        pub indexes: Span<PIndex>,
        /// Each row's level (decision 0090), in the rows' order; empty when they have none (decision 0402 K1).
        pub levels: IntCol,
    }
}

/// What packing wrote: its bytes, and each column's encoding and bytes.
#[derive(Debug)]
pub struct PackStats {
    pub rows: usize,
    pub bytes: usize,
    pub cols: Vec<(&'static str, u32, usize)>,
    pub index_bytes: usize,
}

/// Whether index `cols` is answered by the order: its columns are the order's first ones, in any order.
fn on_order(spec: &Spec, cols: &[u16]) -> bool {
    let lead: Vec<u16> = spec.order.iter().take(cols.len()).copied().collect();
    cols.len() <= spec.order.len() && cols.iter().all(|c| lead.contains(c))
}

/// A permutation index planned: its column, and its keys, starts and positions with their encodings.
struct IxPlan<'s> {
    col: u32,
    parts: [(&'s [i64], Choice); 3],
}

/// Everything chosen before it is placed, so the block is made exactly its size: the columns (column `c`'s `n`
/// values at `cols[c * n..]`, in canonical order) and their encodings, and the indexes.
struct Plan<'s> {
    arity: usize,
    n: usize,
    cols: &'s [i64],
    choices: Vec<Choice>,
    indexes: Vec<IxPlan<'s>>,
    /// The rows' levels, in their order, and their encoding.
    levels: Option<(&'s [i64], Choice)>,
}

impl Plan<'_> {
    fn bytes(&self) -> usize {
        let descr = size_of::<Packed>() + self.arity * size_of::<IntCol>() + self.indexes.len() * size_of::<PIndex>();
        let cols: usize = self.choices.iter().map(|c| c.bytes() + 2 * 64).sum();
        let ix: usize = self.indexes.iter().flat_map(|x| x.parts.iter()).map(|(_, c)| c.bytes() + 2 * 64).sum();
        let levels = self.levels.as_ref().map_or(0, |(_, c)| c.bytes() + 2 * 64);
        descr + cols + ix + levels + 512
    }
}

fn plan<'s>(scratch: &'s Scratch, arity: usize, spec: &Spec, cols: &'s [i64], n: usize) -> Plan<'s> {
    let mut indexes = Vec::new();
    for ix in &spec.indexes {
        if on_order(spec, ix) {
            continue;
        }
        assert_eq!(ix.len(), 1, "a packed index off the order is on one column");
        let c = ix[0] as usize;
        let v = &cols[c * n..(c + 1) * n];
        // Positions grouped by the column's value: a radix sort of the values (less their minimum), stable, so
        // positions stay in canonical order within a value.
        let lo = v.iter().copied().min().unwrap_or(0);
        let hi = v.iter().copied().max().unwrap_or(0);
        let keys: &mut [u64] = scratch.take(n);
        let perm: &mut [u32] = scratch.take(n);
        for (i, (k, p)) in keys.iter_mut().zip(perm.iter_mut()).enumerate() {
            *k = (v[i] as i128 - lo as i128) as u64;
            *p = i as u32;
        }
        let bits = 64 - ((hi as i128 - lo as i128) as u64).leading_zeros();
        crate::sort::radix_pairs(keys, perm, bits, scratch.take(n), scratch.take(n));
        let distinct = if n == 0 { 0 } else { 1 + keys.windows(2).filter(|w| w[0] != w[1]).count() };
        let kv: &mut [i64] = scratch.take(distinct);
        let st: &mut [i64] = scratch.take(distinct + 1);
        let mut d = 0;
        for i in 0..n {
            if i == 0 || keys[i] != keys[i - 1] {
                kv[d] = lo + keys[i] as i64;
                st[d] = i as i64;
                d += 1;
            }
        }
        st[distinct] = n as i64;
        let pm: &mut [i64] = scratch.take(n);
        for (o, &p) in pm.iter_mut().zip(perm.iter()) {
            *o = p as i64;
        }
        let (kv, st, pm) = (&*kv, &*st, &*pm);
        let [ck, cs, cp]: [Choice; 3] = each(n >= PAR_MIN, 3, |j| choose([kv, st, pm][j])).try_into().unwrap();
        indexes.push(IxPlan { col: c as u32, parts: [(kv, ck), (st, cs), (pm, cp)] });
    }
    let choices = each(n * arity >= PAR_MIN, arity, |c| choose(&cols[c * n..(c + 1) * n]));
    Plan { arity, n, cols, choices, indexes, levels: None }
}

/// Lays out the block (descriptors, and every column's words, zero), then packs the columns into it in
/// parallel: each column into its own words.
fn place(plan: &Plan, spec: &Spec, kind: u32) -> (Block, PackStats) {
    let block = crate::alloc::host(kind, plan.bytes());
    let p = place_at(&block, plan, spec);
    block.set_root(p);
    let stats = stats_of(plan, block.used());
    (block, stats)
}

fn stats_of(plan: &Plan, bytes: usize) -> PackStats {
    PackStats {
        rows: plan.n,
        bytes,
        cols: plan.choices.iter().map(|c| (crate::col::TAG_NAMES[c.tag as usize], c.bits, c.bytes())).collect(),
        index_bytes: plan.indexes.iter().flat_map(|x| x.parts.iter()).map(|(_, c)| c.bytes()).sum(),
    }
}

/// A column to pack: its encoding, its values, and its words and runs' words in the block.
type Job<'a> = (&'a Choice, &'a [i64], &'a mut [u64], &'a mut [u64]);

/// Places `plan` in `block` (open): its descriptor first, then its columns' words, all from one point on, so the
/// packed relation is one contiguous range of the block (decision 0402 K4: it moves by `memcpy`).
fn place_at(block: &Block, plan: &Plan, spec: &Spec) -> *mut Packed {
    let p: *mut Packed = block.alloc_t(1);
    let cp: *mut IntCol = block.alloc_t(plan.arity);
    let ip: *mut PIndex = block.alloc_t(plan.indexes.len().max(1));
    let n = plan.n;
    let mut jobs: Vec<Job> = Vec::new();
    unsafe {
        (*p).arity = plan.arity as u32;
        (*p).n = n as u32;
        (*p).nokey = spec.order.len() as u32;
        for (i, &c) in spec.order.iter().enumerate() {
            (*p).okey[i] = c;
        }
        // Columns are placed in schema order, their rows in canonical order.
        for (c, ch) in plan.choices.iter().enumerate() {
            let (d, r) = IntCol::lay_out(cp.add(c), block, ch);
            jobs.push((ch, &plan.cols[c * n..(c + 1) * n], d, r));
        }
        for (x, ixp) in plan.indexes.iter().enumerate() {
            let ix = ip.add(x);
            (*ix).col = ixp.col;
            let [(k, kc), (s, sc), (m, mc)] = &ixp.parts;
            for (at, v, ch) in [(&raw mut (*ix).keys, k, kc), (&raw mut (*ix).starts, s, sc), (&raw mut (*ix).perm, m, mc)] {
                let (d, r) = IntCol::lay_out(at, block, ch);
                jobs.push((ch, v, d, r));
            }
        }
        (*p).nindex = plan.indexes.len() as u32;
        Span::set(&raw mut (*p).cols, cp, plan.arity);
        Span::set(&raw mut (*p).indexes, ip, plan.indexes.len());
        if let Some((v, ch)) = &plan.levels {
            let (d, r) = IntCol::lay_out(&raw mut (*p).levels, block, ch);
            jobs.push((ch, v, d, r));
        }
    }
    let write = |(ch, v, d, r): Job| crate::col::write(ch, v, d, r);
    if n * plan.arity >= PAR_MIN { jobs.into_par_iter().for_each(write) } else { jobs.into_iter().for_each(write) }
    p
}

/// Packs what `view` reads (a full relation, or a base and its delta) into a new block of its exact size. A full
/// frozen relation's rows are in canonical order already, so its columns are read a chunk at a time; a base and
/// its delta are read through their merged order.
pub fn pack(view: &View, spec: &Spec, kind: u32) -> (Block, PackStats) {
    crate::scratch::with(|s| {
        let a = view.arity();
        let (cols, n) = match view.base {
            None if view.top.is_frozen() => {
                let n = view.top.rows.len() as usize;
                let cols: &mut [i64] = s.take(n * a);
                let rows = &view.top.rows;
                let read = |(c, out): (usize, &mut [i64])| rows.read_col(c, n as u32, out);
                if n * a >= PAR_MIN { cols.par_chunks_mut(n.max(1)).enumerate().for_each(read) } else { cols.chunks_mut(n.max(1)).enumerate().for_each(read) }
                (cols, n)
            }
            _ => {
                let n = view.live_len();
                let cols: &mut [i64] = s.take(n * a);
                for (j, id) in view.ordered_iter().enumerate() {
                    for c in 0..a {
                        cols[c * n + j] = view.get(id, c);
                    }
                }
                (cols, n)
            }
        };
        place(&plan(s, a, spec, cols, n), spec, kind)
    })
}

/// Packs an open relation's rows directly, never freezing them in place: each column read once, sorted into
/// canonical order (a radix sort of the packed key, as freeze's), deduplicated, encoded.
pub fn pack_open(rel: &crate::relation::Relation, spec: &Spec, kind: u32) -> (Block, PackStats) {
    assert!(!rel.is_delta(), "a delta is packed through its view, with its base");
    crate::scratch::with(|s| {
        let a = rel.arity as usize;
        let n = rel.rows.len() as usize;
        let raw: &mut [i64] = s.take(n * a);
        let rows = &rel.rows;
        let read = |(c, col): (usize, &mut [i64])| {
            rows.read_col(c, n as u32, col);
            col.iter().fold((i64::MAX, i64::MIN), |(a, b), &v| (a.min(v), b.max(v)))
        };
        let range: Vec<(i64, i64)> = if n * a >= PAR_MIN {
            raw.par_chunks_mut(n.max(1)).enumerate().map(read).collect()
        } else {
            raw.chunks_mut(n.max(1)).enumerate().map(read).collect()
        };
        let (lo, hi): (Vec<i64>, Vec<i64>) = (0..a).map(|c| range.get(c).copied().unwrap_or((i64::MAX, i64::MIN))).unzip();
        let key = canonical_cols(spec, a);
        let ids: &mut [u32] = s.take(n);
        for (i, id) in ids.iter_mut().enumerate() {
            *id = i as u32;
        }
        let keys = sort_canonical(s, raw, n, &key, &lo, &hi, ids);
        let order: &mut [u32] = s.take(n);
        let mut m = 0;
        for p in 0..n {
            if p == 0 || !same_row(raw, n, &key, keys, ids, p) {
                order[m] = ids[p];
                m += 1;
            }
        }
        let cols: &mut [i64] = s.take(m * a);
        let (raw, order) = (&*raw, &order[..m]);
        let gather = |(c, out): (usize, &mut [i64])| {
            for (d, &i) in out.iter_mut().zip(order) {
                *d = raw[c * n + i as usize];
            }
        };
        if m * a >= PAR_MIN {
            cols.par_chunks_mut(m.max(1)).enumerate().for_each(gather)
        } else {
            cols.chunks_mut(m.max(1)).enumerate().for_each(gather)
        }
        place(&plan(s, a, spec, cols, m), spec, kind)
    })
}

/// Rows ready to be placed (decision 0402 K3): in canonical order, without duplicates, each column's encoding chosen.
pub struct Prepared<'s> {
    plan: Plan<'s>,
    order: Vec<u16>,
    indexes: Vec<Vec<u16>>,
    /// Whether the rows came in canonical order and distinct, so were not sorted.
    pub in_order: bool,
}

/// Whether `cols` (`n` rows, column-major) are strictly increasing in `key`'s order: in canonical order, distinct.
fn in_canonical_order(cols: &[i64], n: usize, key: &[usize]) -> bool {
    (1..n).all(|i| {
        for &c in key {
            let (x, y) = (cols[c * n + i - 1], cols[c * n + i]);
            if x != y {
                return x < y;
            }
        }
        false
    })
}

/// `n` rows of `a` columns (`raw`, column-major), with each row's level when they have one, prepared to be placed:
/// one pass checks whether they are in canonical order and distinct already (an input's granule usually is, and a
/// relation that declares its output sorted, K7, always); else they are sorted as `pack_open` sorts and their
/// duplicates dropped, the levels following their rows. Then each column's encoding is chosen.
pub fn prepare<'s>(s: &'s Scratch, raw: &'s [i64], n: usize, a: usize, spec: &Spec, levels: Option<&[u32]>) -> Prepared<'s> {
    assert_eq!(raw.len(), n * a);
    assert!(levels.is_none_or(|l| l.len() == n), "a level a row");
    let key = canonical_cols(spec, a);
    let in_order = in_canonical_order(raw, n, &key);
    let (cols, lv, m): (&'s [i64], Option<&'s [i64]>, usize) = if in_order {
        let lv = levels.map(|l| {
            let out: &mut [i64] = s.take(n);
            for (o, &x) in out.iter_mut().zip(l) {
                *o = x as i64;
            }
            &*out
        });
        (raw, lv, n)
    } else {
        let range = |c: usize| raw[c * n..(c + 1) * n].iter().fold((i64::MAX, i64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let (lo, hi): (Vec<i64>, Vec<i64>) = (0..a).map(range).unzip();
        let ids: &mut [u32] = s.take(n);
        for (i, id) in ids.iter_mut().enumerate() {
            *id = i as u32;
        }
        let keys = sort_canonical(s, raw, n, &key, &lo, &hi, ids);
        let order: &mut [u32] = s.take(n);
        let mut m = 0;
        for p in 0..n {
            if p == 0 || !same_row(raw, n, &key, keys, ids, p) {
                order[m] = ids[p];
                m += 1;
            }
        }
        let order = &order[..m];
        let cols: &mut [i64] = s.take(m * a);
        for (c, out) in cols.chunks_mut(m.max(1)).enumerate().take(a) {
            for (d, &i) in out.iter_mut().zip(order) {
                *d = raw[c * n + i as usize];
            }
        }
        let lv = levels.map(|l| {
            let out: &mut [i64] = s.take(m);
            for (o, &i) in out.iter_mut().zip(order) {
                *o = l[i as usize] as i64;
            }
            &*out
        });
        (&*cols, lv, m)
    };
    let mut plan = plan(s, a, spec, cols, m);
    plan.levels = lv.map(|v| (v, choose(v)));
    Prepared { plan, order: spec.order.clone(), indexes: spec.indexes.clone(), in_order }
}

impl Prepared<'_> {
    /// Rows after duplicates were dropped.
    pub fn len(&self) -> usize {
        self.plan.n
    }

    pub fn is_empty(&self) -> bool {
        self.plan.n == 0
    }

    /// The most bytes placing it takes.
    pub fn bytes(&self) -> usize {
        self.plan.bytes()
    }

    /// Places it in `block` (open), one contiguous range: the packed relation, and what it wrote.
    pub fn place_in<'b>(&self, block: &'b Block) -> (&'b Packed, PackStats) {
        let spec = Spec { widths: vec![8; self.plan.arity], order: self.order.clone(), indexes: self.indexes.clone(), min_log: 0 };
        let before = block.used();
        let p = place_at(block, &self.plan, &spec);
        // SAFETY: placed in `block`, which lives as long as the reference.
        (unsafe { &*p }, stats_of(&self.plan, block.used() - before))
    }
}

impl Packed {
    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    pub fn get(&self, pos: usize, c: usize) -> i64 {
        self.cols.as_slice()[c].get(pos)
    }

    #[inline]
    pub fn col(&self, c: usize) -> &IntCol {
        &self.cols.as_slice()[c]
    }

    /// Its rows' levels, in their order; `None` when they have none.
    pub fn levels(&self) -> Option<&IntCol> {
        (self.n > 0 && self.levels.len() == self.len()).then_some(&self.levels)
    }

    /// The positions whose columns `cols` equal `key`, in canonical order: a range when the key leads the order,
    /// else through the column's permutation index.
    pub fn probe(&self, cols: &[u16], key: &[i64]) -> Probe<'_> {
        let ok = &self.okey[..self.nokey as usize];
        if cols.len() <= ok.len() && cols.iter().all(|c| ok[..cols.len()].contains(c)) {
            let (mut lo, mut hi) = (0usize, self.len());
            for &oc in &ok[..cols.len()] {
                let k = key[cols.iter().position(|&c| c == oc).unwrap()];
                (lo, hi) = self.col(oc as usize).dec().equal_range(lo, hi, k);
            }
            return Probe::Range(lo..hi);
        }
        assert_eq!(cols.len(), 1);
        let ix = self.indexes.as_slice().iter().find(|ix| ix.col == cols[0] as u32).expect("an index on the column");
        let keys = ix.keys.dec();
        let k = keys.lower_bound(0, keys.len(), key[0]);
        if k >= keys.len() || keys.get(k) != key[0] {
            return Probe::Range(0..0);
        }
        let starts = ix.starts.dec();
        Probe::Perm(ix.perm.dec(), starts.get(k) as usize..starts.get(k + 1) as usize)
    }

    /// A digest of the rows in order: the same as the view's it was packed from.
    pub fn digest(&self) -> u64 {
        let mut h = crate::SEED;
        for p in 0..self.len() {
            for c in 0..self.arity as usize {
                h = crate::mix(h, self.get(p, c));
            }
        }
        crate::mix(h, self.len() as i64)
    }
}

pub enum Probe<'a> {
    Range(std::ops::Range<usize>),
    Perm(crate::col::Dec<'a>, std::ops::Range<usize>),
}

impl Iterator for Probe<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            Probe::Range(r) => r.next(),
            Probe::Perm(p, r) => r.next().map(|i| p.get(i) as usize),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;

    fn spec(a: usize) -> Spec {
        Spec { widths: vec![8; a], order: vec![0], indexes: Vec::new(), min_log: 0 }
    }

    /// Rows (row-major) as columns.
    fn columns(rows: &[[i64; 2]]) -> Vec<i64> {
        (0..2).flat_map(|c| rows.iter().map(move |r| r[c])).collect()
    }

    fn read(p: &Packed) -> Vec<[i64; 2]> {
        (0..p.len()).map(|i| [p.get(i, 0), p.get(i, 1)]).collect()
    }

    /// K-R2 (decision 0402): rows already in canonical order and distinct are packed without a sort, and read as the
    /// same rows given out of order are after theirs.
    #[test]
    fn a_sorted_input_is_not_sorted_again() {
        let sorted: Vec<[i64; 2]> = (0..5000).map(|i| [(i / 50 + 3) << 32 | i, i % 7]).collect();
        let mut shuffled = sorted.clone();
        shuffled.reverse();
        shuffled.extend_from_slice(&sorted[..100]);
        let block = Block::anon(kind::KEPT, 16 << 20);
        let (a, b) = crate::scratch::with(|s| {
            let ca = s.take::<i64>(10_000);
            ca.copy_from_slice(&columns(&sorted));
            let pa = prepare(s, ca, 5000, 2, &spec(2), None);
            let cb = s.take::<i64>(10_200);
            cb.copy_from_slice(&columns(&shuffled));
            let pb = prepare(s, cb, 5100, 2, &spec(2), None);
            assert!(pa.in_order && !pb.in_order);
            assert_eq!((pa.len(), pb.len()), (5000, 5000), "duplicates dropped");
            (read(pa.place_in(&block).0), read(pb.place_in(&block).0))
        });
        assert_eq!(a, sorted);
        assert_eq!(b, sorted);
    }

    /// K-R3: each row's level is read at its packed position, the rows sorted or not.
    #[test]
    fn levels_follow_their_rows() {
        let rows: Vec<[i64; 2]> = (0..3000).map(|i| [(i * 7919) % 3001, i]).collect();
        let levels: Vec<u32> = (0..3000).map(|i| (i % 13) as u32).collect();
        let block = Block::anon(kind::KEPT, 16 << 20);
        crate::scratch::with(|s| {
            let c = s.take::<i64>(6000);
            c.copy_from_slice(&columns(&rows));
            let p = prepare(s, c, 3000, 2, &spec(2), Some(&levels[..]));
            assert!(!p.in_order);
            let (packed, _) = p.place_in(&block);
            let lv = packed.levels().expect("levels kept");
            for i in 0..packed.len() {
                let orig = packed.get(i, 1) as usize;
                assert_eq!(lv.get(i), levels[orig] as i64, "row {i}");
            }
            assert!(packed.get(0, 0) <= packed.get(1, 0));
        });
        crate::scratch::with(|s| {
            let c = s.take::<i64>(6000);
            c.copy_from_slice(&columns(&rows));
            assert!(prepare(s, c, 3000, 2, &spec(2), None).place_in(&block).0.levels().is_none());
        });
    }
}
