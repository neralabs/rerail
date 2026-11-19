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
}

impl Plan<'_> {
    fn bytes(&self) -> usize {
        let descr = size_of::<Packed>() + self.arity * size_of::<IntCol>() + self.indexes.len() * size_of::<PIndex>();
        let cols: usize = self.choices.iter().map(|c| c.bytes() + 2 * 64).sum();
        let ix: usize = self.indexes.iter().flat_map(|x| x.parts.iter()).map(|(_, c)| c.bytes() + 2 * 64).sum();
        descr + cols + ix + 512
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
    Plan { arity, n, cols, choices, indexes }
}

/// Lays out the block (descriptors, and every column's words, zero), then packs the columns into it in
/// parallel: each column into its own words.
fn place(plan: &Plan, spec: &Spec, kind: u32) -> (Block, PackStats) {
    let block = crate::alloc::host(kind, plan.bytes());
    let p: *mut Packed = block.alloc_t(1);
    let cp: *mut IntCol = block.alloc_t(plan.arity);
    let ip: *mut PIndex = block.alloc_t(plan.indexes.len().max(1));
    let n = plan.n;
    type Job<'a> = (&'a Choice, &'a [i64], &'a mut [u64], &'a mut [u64]);
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
            let (d, r) = IntCol::lay_out(cp.add(c), &block, ch);
            jobs.push((ch, &plan.cols[c * n..(c + 1) * n], d, r));
        }
        for (x, ixp) in plan.indexes.iter().enumerate() {
            let ix = ip.add(x);
            (*ix).col = ixp.col;
            let [(k, kc), (s, sc), (m, mc)] = &ixp.parts;
            for (at, v, ch) in [(&raw mut (*ix).keys, k, kc), (&raw mut (*ix).starts, s, sc), (&raw mut (*ix).perm, m, mc)] {
                let (d, r) = IntCol::lay_out(at, &block, ch);
                jobs.push((ch, v, d, r));
            }
        }
        (*p).nindex = plan.indexes.len() as u32;
        Span::set(&raw mut (*p).cols, cp, plan.arity);
        Span::set(&raw mut (*p).indexes, ip, plan.indexes.len());
    }
    let write = |(ch, v, d, r): (&Choice, &[i64], &mut [u64], &mut [u64])| crate::col::write(ch, v, d, r);
    if n * plan.arity >= PAR_MIN { jobs.into_par_iter().for_each(write) } else { jobs.into_iter().for_each(write) }
    block.set_root(p);
    let stats = PackStats {
        rows: n,
        bytes: block.used(),
        cols: plan.choices.iter().map(|c| (["const", "pack", "runs", "line"][c.tag as usize], c.bits, c.bytes())).collect(),
        index_bytes: plan.indexes.iter().flat_map(|x| x.parts.iter()).map(|(_, c)| c.bytes()).sum(),
    };
    (block, stats)
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
