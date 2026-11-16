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
use crate::col::{Encoded, IntCol, encode};
use crate::refs::Span;
use crate::relation::{Spec, View};
use crate::structure;

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

/// The order key's columns, then the rest: the column order of the canonical sort.
fn order_cols(spec: &Spec, arity: usize) -> Vec<usize> {
    let mut c: Vec<usize> = spec.order.iter().map(|&c| c as usize).collect();
    c.extend((0..arity).filter(|x| !spec.order.contains(&(*x as u16))));
    c
}

/// Whether index `cols` is answered by the order: its columns are the order's first ones, in any order.
fn on_order(spec: &Spec, cols: &[u16]) -> bool {
    let lead: Vec<u16> = spec.order.iter().take(cols.len()).copied().collect();
    cols.len() <= spec.order.len() && cols.iter().all(|c| lead.contains(c))
}

/// Everything packed, before it is placed: so the block can be made exactly its size.
struct Plan {
    arity: usize,
    n: usize,
    cols: Vec<Encoded>,
    indexes: Vec<(u32, Encoded, Encoded, Encoded)>,
}

impl Plan {
    fn bytes(&self) -> usize {
        let descr = size_of::<Packed>() + self.arity * size_of::<IntCol>() + self.indexes.len() * size_of::<PIndex>();
        let cols: usize = self.cols.iter().map(|e| e.bytes() + 64).sum();
        let ix: usize = self.indexes.iter().map(|(_, a, b, c)| a.bytes() + b.bytes() + c.bytes() + 3 * 64).sum();
        descr + cols + ix + 256
    }
}

fn plan(arity: usize, spec: &Spec, cols: Vec<Vec<i64>>) -> Plan {
    let n = cols.first().map_or(0, Vec::len);
    let mut indexes = Vec::new();
    for ix in &spec.indexes {
        if on_order(spec, ix) {
            continue;
        }
        assert_eq!(ix.len(), 1, "a packed index off the order is on one column");
        let c = ix[0] as usize;
        let mut perm: Vec<u32> = (0..n as u32).collect();
        perm.sort_by_key(|&p| cols[c][p as usize]); // stable: canonical order within a key
        let mut keys = Vec::new();
        let mut starts = Vec::new();
        for (i, &p) in perm.iter().enumerate() {
            let v = cols[c][p as usize];
            if keys.last() != Some(&v) {
                keys.push(v);
                starts.push(i as i64);
            }
        }
        starts.push(n as i64);
        let perm: Vec<i64> = perm.into_iter().map(i64::from).collect();
        indexes.push((c as u32, encode(&keys), encode(&starts), encode(&perm)));
    }
    let cols = cols.iter().map(|v| encode(v)).collect();
    Plan { arity, n, cols, indexes }
}

fn place(plan: &Plan, spec: &Spec, kind: u32) -> (Block, PackStats) {
    let block = Block::anon(kind, plan.bytes());
    let p: *mut Packed = block.alloc_t(1);
    let cp: *mut IntCol = block.alloc_t(plan.arity);
    let ip: *mut PIndex = block.alloc_t(plan.indexes.len().max(1));
    unsafe {
        (*p).arity = plan.arity as u32;
        (*p).n = plan.n as u32;
        (*p).nokey = spec.order.len() as u32;
        for (i, &c) in spec.order.iter().enumerate() {
            (*p).okey[i] = c;
        }
        // Columns are placed in schema order; the canonical order sorted them by `order_cols`.
        for (c, e) in plan.cols.iter().enumerate() {
            IntCol::place(cp.add(c), &block, e);
        }
        for (x, (col, k, s, pm)) in plan.indexes.iter().enumerate() {
            let ix = ip.add(x);
            (*ix).col = *col;
            IntCol::place(&raw mut (*ix).keys, &block, k);
            IntCol::place(&raw mut (*ix).starts, &block, s);
            IntCol::place(&raw mut (*ix).perm, &block, pm);
        }
        (*p).nindex = plan.indexes.len() as u32;
        Span::set(&raw mut (*p).cols, cp, plan.arity);
        Span::set(&raw mut (*p).indexes, ip, plan.indexes.len());
    }
    block.set_root(p);
    let stats = PackStats {
        rows: plan.n,
        bytes: block.used(),
        cols: plan.cols.iter().map(|e| (["const", "pack", "runs", "line"][e.tag as usize], e.bits, e.bytes())).collect(),
        index_bytes: plan.indexes.iter().map(|(_, a, b, c)| a.bytes() + b.bytes() + c.bytes()).sum(),
    };
    (block, stats)
}

/// Packs what `view` reads (a full relation, or a base and its delta) into a new block of its exact size.
pub fn pack(view: &View, spec: &Spec, kind: u32) -> (Block, PackStats) {
    let a = view.arity();
    let mut cols: Vec<Vec<i64>> = (0..a).map(|_| Vec::with_capacity(view.live_len())).collect();
    for id in view.ordered_iter() {
        for (c, col) in cols.iter_mut().enumerate() {
            col.push(view.get(id, c));
        }
    }
    place(&plan(a, spec, cols), spec, kind)
}

/// Packs an open relation's rows directly, never freezing them in place: sorted, deduplicated, encoded.
pub fn pack_open(rel: &crate::relation::Relation, spec: &Spec, kind: u32) -> (Block, PackStats) {
    assert!(!rel.is_delta(), "a delta is packed through its view, with its base");
    let a = rel.arity as usize;
    let n = rel.rows.len() as usize;
    let key = order_cols(spec, a);
    let mut vals = vec![0i64; n * a];
    for (j, &c) in key.iter().enumerate() {
        for i in 0..n {
            vals[i * a + j] = rel.rows.get(i as u32, c);
        }
    }
    let row = |i: usize| &vals[i * a..i * a + a];
    let mut ids: Vec<u32> = (0..n as u32).collect();
    ids.sort_unstable_by(|&x, &y| row(x as usize).cmp(row(y as usize)));
    ids.dedup_by(|x, y| row(*x as usize) == row(*y as usize));
    let mut cols: Vec<Vec<i64>> = (0..a).map(|_| Vec::with_capacity(ids.len())).collect();
    for &i in &ids {
        for (j, &c) in key.iter().enumerate() {
            cols[c].push(vals[i as usize * a + j]);
        }
    }
    drop(vals);
    place(&plan(a, spec, cols), spec, kind)
}

impl Packed {
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
                let col = self.col(oc as usize);
                let a = col.lower_bound(lo, hi, k);
                let b = if k == i64::MAX { hi } else { col.lower_bound(a, hi, k + 1) };
                (lo, hi) = (a, b);
            }
            return Probe::Range(lo..hi);
        }
        assert_eq!(cols.len(), 1);
        let ix = self.indexes.as_slice().iter().find(|ix| ix.col == cols[0] as u32).expect("an index on the column");
        let k = ix.keys.lower_bound(0, ix.keys.len(), key[0]);
        if k >= ix.keys.len() || ix.keys.get(k) != key[0] {
            return Probe::Range(0..0);
        }
        Probe::Perm(&ix.perm, ix.starts.get(k) as usize..ix.starts.get(k + 1) as usize)
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
    Perm(&'a IntCol, std::ops::Range<usize>),
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
