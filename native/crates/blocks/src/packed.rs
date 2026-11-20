//! A packed relation: a frozen relation written again, exactly sized, in the fewest bits.
//!
//! Its rows are its live rows in canonical order, so a row's position is its place in that order:
//!
//! - no order array, no dead bits and no duplicates: they were the in-place form's;
//! - each column a `PCol`: one `IntCol`, or a node id's two parts (split at `ID_SPLIT`), each its own `IntCol`, and
//!   each part optionally less column 0's same part at the row; the form and the encodings chosen by exact size;
//! - a key that is a prefix of the order key needs no index: its rows are one range, found by binary search
//!   on the leading columns;
//! - any other key gets a permutation index: its distinct values (sorted), where each value's positions start,
//!   and the positions, each packed in the bits it needs.
//!
//! It is the form of a block that leaves its run (kept, journaled, or put in a pinned slab for the GPU), and of
//! a roll-up: `pack` reads any view, so a base and its delta pack into one full relation.

use crate::block::Block;
use crate::col::{Choice, Dec, IntCol, choose};
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
    if par { rerail_context::on_cores(|| (0..k).into_par_iter().map(f).collect()) } else { (0..k).map(f).collect() }
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

/// The bit a node id is split at (decision 0456 K2): an id is `document << 32 | slot << 18 | index` (decisions 0083
/// §3, 0094 §2), so above it is the id's granule, below it its index there. The engine's `granules::INDEX_BITS`,
/// which asserts they are equal.
pub const ID_SPLIT: u32 = 18;

structure! {
    /// A column of a packed relation (decision 0456 K2). `split` 0: its values whole, in `hi`. Else split at bit
    /// `split`: `hi` holds each value's `v >> split` and `lo` its `v & (2^split - 1)`, so a sorted id column is runs
    /// of granules and small indexes where a group of 128 ids would span documents. `rel` 1: each part is less
    /// column 0's same part at the row (an id near the row's first id, as a child is near its parent); column 0 is
    /// never relative, nor a column of an order column 0 does not lead (`Rel`).
    pub struct PCol {
        pub split: u32,
        pub rel: u32,
        pub hi: IntCol,
        pub lo: IntCol,
    }
}

structure! {
    pub struct Packed {
        pub arity: u32,
        pub n: u32,
        pub nokey: u32,
        pub nindex: u32,
        pub okey: [u16; 8],
        pub cols: Span<PCol>,
        pub indexes: Span<PIndex>,
        /// Each row's level (decision 0090), in the rows' order; empty when they have none (decision 0456 K1).
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
    choices: Vec<ColPlan<'s>>,
    indexes: Vec<IxPlan<'s>>,
    /// The rows' levels, in their order, and their encoding.
    levels: Option<(&'s [i64], Choice)>,
}

impl Plan<'_> {
    fn bytes(&self) -> usize {
        let descr = size_of::<Packed>() + self.arity * size_of::<PCol>() + self.indexes.len() * size_of::<PIndex>();
        let cols: usize = self.choices.iter().flat_map(|c| c.parts.iter()).map(|(_, c)| c.bytes() + 2 * 64).sum();
        let ix: usize = self.indexes.iter().flat_map(|x| x.parts.iter()).map(|(_, c)| c.bytes() + 2 * 64).sum();
        let levels = self.levels.as_ref().map_or(0, |(_, c)| c.bytes() + 2 * 64);
        descr + cols + ix + levels + 512
    }
}

/// A column's values as a part holds them: the column's own, or made from them.
enum Vals<'s> {
    Of(&'s [i64]),
    Made(Vec<i64>),
}

impl std::ops::Deref for Vals<'_> {
    type Target = [i64];
    fn deref(&self) -> &[i64] {
        match self {
            Vals::Of(v) => v,
            Vals::Made(v) => v,
        }
    }
}

/// A column planned: its form (`PCol`'s `split` and `rel`) and its parts' values and encodings (one, or two when split).
struct ColPlan<'s> {
    split: u32,
    rel: bool,
    parts: Vec<(Vals<'s>, Choice)>,
}

impl ColPlan<'_> {
    /// What it takes in a block: each part's words, and the alignment a part's placement may cost.
    fn cost(parts: &[(Vals<'_>, Choice)]) -> usize {
        parts.iter().map(|(_, c)| c.bytes() + 2 * 64).sum()
    }
}

/// Column `c`'s form and encodings, the smallest of: whole; split at `ID_SPLIT`; and, where `rel` may be (`Rel`),
/// each of those less column 0's. Only a column whose values span past `2^ID_SPLIT` is tried split or relative:
/// below it its high part is constant.
fn plan_col<'s>(cols: &'s [i64], n: usize, c: usize, may_rel: Rel) -> ColPlan<'s> {
    let v = &cols[c * n..(c + 1) * n];
    let whole = ColPlan { split: 0, rel: false, parts: vec![(Vals::Of(v), choose(v))] };
    let (lo, hi) = v.iter().fold((i64::MAX, i64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    if n < 2 || (hi as i128 - lo as i128) < 1 << ID_SPLIT {
        return whole;
    }
    let (s, m) = (ID_SPLIT, (1i64 << ID_SPLIT) - 1);
    let part = |f: &dyn Fn(usize) -> i64| -> (Vals<'s>, Choice) {
        let made: Vec<i64> = (0..n).map(f).collect();
        let ch = choose(&made);
        (Vals::Made(made), ch)
    };
    let mut best = whole;
    let mut take = |cand: ColPlan<'s>| {
        if ColPlan::cost(&cand.parts) < ColPlan::cost(&best.parts) {
            best = cand;
        }
    };
    take(ColPlan { split: s, rel: false, parts: vec![part(&|i| v[i] >> s), part(&|i| v[i] & m)] });
    if may_rel != Rel::Never && c > 0 {
        let b = &cols[..n];
        // A searched column keeps its order less a constant only where no difference wraps; the split parts' never
        // do (each part is under 2^46).
        if may_rel == Rel::Any || (0..n).all(|i| v[i].checked_sub(b[i]).is_some()) {
            take(ColPlan { split: 0, rel: true, parts: vec![part(&|i| v[i].wrapping_sub(b[i]))] });
        }
        take(ColPlan { split: s, rel: true, parts: vec![part(&|i| (v[i] >> s).wrapping_sub(b[i] >> s)), part(&|i| (v[i] & m) - (b[i] & m))] });
    }
    best
}

/// Whether a column may be relative to column 0: `Any` off the order key; `Searched` in it after column 0, which
/// leads it (a probe searches it only within a range of one column-0 value, `Packed::probe`, where less column 0's
/// value is less a constant); `Never` in an order column 0 does not lead (searched where column 0 varies).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rel {
    Any,
    Searched,
    Never,
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
    let rel = |c: usize| match () {
        _ if !spec.order.contains(&(c as u16)) => Rel::Any,
        _ if spec.order.first() == Some(&0) => Rel::Searched,
        _ => Rel::Never,
    };
    let choices = each(n * arity >= PAR_MIN, arity, |c| plan_col(cols, n, c, rel(c)));
    Plan { arity, n, choices, indexes, levels: None }
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
        cols: plan
            .choices
            .iter()
            .map(|c| {
                let tag = if c.parts.len() > 1 { "split" } else { crate::col::TAG_NAMES[c.parts[0].1.tag as usize] };
                (tag, c.parts[0].1.bits, c.parts.iter().map(|(_, ch)| ch.bytes()).sum())
            })
            .collect(),
        index_bytes: plan.indexes.iter().flat_map(|x| x.parts.iter()).map(|(_, c)| c.bytes()).sum(),
    }
}

/// A column to pack: its encoding, its values, and its words and runs' words in the block.
type Job<'a> = (&'a Choice, &'a [i64], &'a mut [u64], &'a mut [u64]);

/// Places `plan` in `block` (open): its descriptor first, then its columns' words, all from one point on, so the
/// packed relation is one contiguous range of the block (decision 0456 K4: it moves by `memcpy`).
fn place_at(block: &Block, plan: &Plan, spec: &Spec) -> *mut Packed {
    let p: *mut Packed = block.alloc_t(1);
    let cp: *mut PCol = block.alloc_t(plan.arity);
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
        for (c, cpl) in plan.choices.iter().enumerate() {
            let pc = cp.add(c);
            (*pc).split = cpl.split;
            (*pc).rel = u32::from(cpl.rel);
            for (k, (v, ch)) in cpl.parts.iter().enumerate() {
                let at = if k == 0 { &raw mut (*pc).hi } else { &raw mut (*pc).lo };
                let (d, r) = IntCol::lay_out(at, block, ch);
                jobs.push((ch, &v[..], d, r));
            }
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
    if n * plan.arity >= PAR_MIN { rerail_context::on_cores(|| jobs.into_par_iter().for_each(write)) } else { jobs.into_iter().for_each(write) }
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
                if n * a >= PAR_MIN { rerail_context::on_cores(|| cols.par_chunks_mut(n.max(1)).enumerate().for_each(read)) } else { cols.chunks_mut(n.max(1)).enumerate().for_each(read) }
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
        let rows = &rel.rows;
        let raw: &mut [i64] = s.take(n * a);
        let read = |(c, col): (usize, &mut [i64])| {
            rows.read_col(c, n as u32, col);
            col.iter().fold((i64::MAX, i64::MIN), |(a, b), &v| (a.min(v), b.max(v)))
        };
        let range: Vec<(i64, i64)> = if n * a >= PAR_MIN {
            rerail_context::on_cores(|| raw.par_chunks_mut(n.max(1)).enumerate().map(read).collect())
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
            rerail_context::on_cores(|| cols.par_chunks_mut(m.max(1)).enumerate().for_each(gather))
        } else {
            cols.chunks_mut(m.max(1)).enumerate().for_each(gather)
        }
        place(&plan(s, a, spec, cols, m), spec, kind)
    })
}

/// Rows ready to be placed (decision 0456 K3): in canonical order, without duplicates, each column's encoding chosen.
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

/// The order `prepare` lays rows out in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Canonical: the spec's order columns, then the rest; duplicates dropped.
    Canonical,
    /// As given, every row.
    AsGiven,
}

/// `n` rows of `a` columns (`raw`, column-major), with each row's level when they have one, prepared to be placed:
/// one pass checks whether they are in canonical order and distinct already (an input's granule usually is, and a
/// relation that declares its output sorted, K7, always); else they are sorted as `pack_open` sorts and their
/// duplicates dropped, the levels following their rows. Then each column's encoding is chosen.
///
/// `Order::AsGiven`: the rows are kept in the order given, none dropped, and the packed rows record no order (an
/// input's granule: its rows by owner, its readers' ranges by position, decision 0117 step 2).
pub fn prepare<'s>(s: &'s Scratch, raw: &'s [i64], n: usize, a: usize, spec: &Spec, levels: Option<&[u32]>, given: Order) -> Prepared<'s> {
    assert_eq!(raw.len(), n * a);
    assert!(levels.is_none_or(|l| l.len() == n), "a level a row");
    let key = canonical_cols(spec, a);
    let as_given = given == Order::AsGiven;
    let in_order = as_given || in_canonical_order(raw, n, &key);
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
    let order = if as_given { Vec::new() } else { spec.order.clone() };
    Prepared { plan, order, indexes: spec.indexes.clone(), in_order: in_order && !as_given }
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
        self.col(c).get(pos)
    }

    /// Column `c`, as it reads: its parts, and column 0 where it is relative to it.
    #[inline]
    pub fn col(&self, c: usize) -> Col<'_> {
        let cols = self.cols.as_slice();
        Col { col: &cols[c], base: (cols[c].rel != 0).then(|| &cols[0]) }
    }

    /// Its rows' levels, in their order; `None` when they have none.
    pub fn levels(&self) -> Option<&IntCol> {
        (self.n > 0 && self.levels.len() == self.len()).then_some(&self.levels)
    }

    /// Whether column `c` leads its canonical order: its rows equal to a value are one range (`col(c).equal_range`).
    pub fn leads(&self, c: usize) -> bool {
        self.nokey > 0 && self.okey[0] as usize == c
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

/// A column of a `Packed`, as it reads (`Packed::col`): its values whole or from their two parts, and column 0's
/// added where it is relative to it.
#[derive(Clone, Copy)]
pub struct Col<'a> {
    col: &'a PCol,
    base: Option<&'a PCol>,
}

impl<'a> Col<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        self.col.hi.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        if self.col.split == 0 && self.base.is_none() { self.col.hi.get(i) } else { self.dec().get(i) }
    }

    /// The column resolved for reads.
    #[inline]
    pub fn dec(&self) -> ColDec<'a> {
        let parts = |c: &'a PCol| (c.hi.dec(), (c.split > 0).then(|| c.lo.dec()), c.split);
        ColDec { own: parts(self.col), base: self.base.map(parts) }
    }

    /// Every value in order.
    pub fn for_each(&self, mut f: impl FnMut(i64)) {
        if self.col.split == 0 && self.base.is_none() {
            self.col.hi.for_each(f)
        } else {
            let d = self.dec();
            (0..self.len()).for_each(|i| f(d.get(i)))
        }
    }

    /// Its parts' bytes.
    pub fn bytes(&self) -> usize {
        self.col.hi.bytes() + if self.col.split > 0 { self.col.lo.bytes() } else { 0 }
    }

    /// Its form: the encoding's name when whole, else `split`; `rel` after it when relative.
    pub fn form(&self) -> String {
        let f = if self.col.split > 0 { "split" } else { self.col.hi.tag_name() };
        if self.base.is_some() { format!("{f} rel") } else { f.to_owned() }
    }
}

/// A part pair resolved for reads: the high (or whole) part, the low when split, and the split.
type Parts<'a> = (Dec<'a>, Option<Dec<'a>>, u32);

/// A `Col` resolved for reads: its parts' and column 0's loaded once.
#[derive(Clone, Copy)]
pub struct ColDec<'a> {
    own: Parts<'a>,
    base: Option<Parts<'a>>,
}

impl ColDec<'_> {
    #[inline]
    pub fn len(&self) -> usize {
        self.own.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn whole((hi, lo, s): &Parts<'_>, i: usize) -> i64 {
        match lo {
            None => hi.get(i),
            Some(lo) => hi.get(i) << s | lo.get(i),
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        let Some(base) = &self.base else { return Self::whole(&self.own, i) };
        let b = Self::whole(base, i);
        let (hi, lo, s) = &self.own;
        match lo {
            None => hi.get(i).wrapping_add(b),
            Some(lo) => hi.get(i).wrapping_add(b >> s) << s | (lo.get(i) + (b & ((1i64 << s) - 1))),
        }
    }

    /// `key` as the column holds it over `lo..hi`: its high (or whole) and low parts, less column 0's when
    /// relative. A relative column is searched only where column 0 is one value (it follows column 0 in the order,
    /// `Rel::Searched`), so its values there are the column's less a constant, in the column's order. `Err(at)`:
    /// no value there can be `key`, which would go at `at`.
    #[inline]
    fn held(&self, lo: usize, hi: usize, key: i64) -> Result<(i64, i64), usize> {
        let (_, low, s) = &self.own;
        let m = (1i64 << s) - 1;
        let Some(base) = &self.base else { return Ok(if low.is_some() { (key >> s, key & m) } else { (key, 0) }) };
        if lo >= hi {
            return Err(lo);
        }
        let b = Self::whole(base, lo);
        debug_assert_eq!(Self::whole(base, hi - 1), b, "a relative column searched where column 0 varies");
        match low {
            Some(_) => Ok(((key >> s) - (b >> s), (key & m) - (b & m))),
            // Every value less `b` fits (`plan_col`), so a key whose difference does not lies past them all.
            None => key.checked_sub(b).map(|d| (d, 0)).ok_or(if key > b { hi } else { lo }),
        }
    }

    /// `lo..hi`'s positions whose value is `key`, the column sorted over that range: its high part's range, then
    /// the low part's within it.
    pub fn equal_range(&self, lo: usize, hi: usize, key: i64) -> (usize, usize) {
        let (kh, kl) = match self.held(lo, hi, key) {
            Ok(k) => k,
            Err(at) => return (at, at),
        };
        match &self.own {
            (h, None, _) => h.equal_range(lo, hi, kh),
            (h, Some(l), _) => {
                let (a, b) = h.equal_range(lo, hi, kh);
                l.equal_range(a, b, kl)
            }
        }
    }

    /// `equal_range` over the whole column, searched from `at` where every position before it is below `key`: a
    /// gallop forward from `at` (`at`, `at + 1`, `at + 2`, `at + 4`, …) to past `key`'s positions, then a search of
    /// that stretch. Keys probed in the column's order, each from the last's start, read it forward once (a merge
    /// join's inner side). Any `at` is correct: one past `key`'s positions, or past the end, takes the whole search.
    pub fn equal_range_from(&self, at: usize, key: i64) -> (usize, usize) {
        let n = self.len();
        if at > n || (at > 0 && self.get(at - 1) >= key) {
            return self.equal_range(0, n, key);
        }
        let (mut end, mut step) = (at, 1usize);
        while end < n && self.get(end) <= key {
            end = at + step;
            step <<= 1;
        }
        self.equal_range(at, end.min(n), key)
    }

    /// The first position in `lo..hi` whose value is not less than `key`, the column sorted over that range.
    pub fn lower_bound(&self, lo: usize, hi: usize, key: i64) -> usize {
        let (kh, kl) = match self.held(lo, hi, key) {
            Ok(k) => k,
            Err(at) => return at,
        };
        match &self.own {
            (h, None, _) => h.lower_bound(lo, hi, kh),
            (h, Some(l), _) => {
                let (a, b) = h.equal_range(lo, hi, kh);
                if a == b { a } else { l.lower_bound(a, b, kl) }
            }
        }
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

    /// K-R2 (decision 0456): rows already in canonical order and distinct are packed without a sort, and read as the
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
            let pa = prepare(s, ca, 5000, 2, &spec(2), None, Order::Canonical);
            let cb = s.take::<i64>(10_200);
            cb.copy_from_slice(&columns(&shuffled));
            let pb = prepare(s, cb, 5100, 2, &spec(2), None, Order::Canonical);
            assert!(pa.in_order && !pb.in_order);
            assert_eq!((pa.len(), pb.len()), (5000, 5000), "duplicates dropped");
            (read(pa.place_in(&block).0), read(pb.place_in(&block).0))
        });
        assert_eq!(a, sorted);
        assert_eq!(b, sorted);
    }

    /// K-R12 (decision 0456 K2): node ids (`document << 32 | slot << 18 | index`, documents numbered as digests are)
    /// are split at `ID_SPLIT`, a child's id relative to its parent's in column 0; every value reads back, by `get`,
    /// `for_each` and the order's probe, in fewer bytes than whole columns take.
    #[test]
    fn node_ids_split_at_their_granule() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut rows: Vec<[i64; 2]> = Vec::new();
        for _ in 0..300 {
            let doc = (next() >> 36) as i64 & 0x0fff_ffff;
            for slot in 0..(next() % 6) as i64 {
                for index in 0..(10 + next() % 80) as i64 {
                    let id = doc << 32 | slot << 18 | index * 3;
                    rows.push([id, id + 1 + (next() % 4) as i64]);
                }
            }
        }
        rows.sort();
        rows.dedup();
        let n = rows.len();
        let block = Block::anon(kind::KEPT, 64 << 20);
        crate::scratch::with(|s| {
            let c = s.take::<i64>(2 * n);
            // copy: load (the test's rows into the scratch the pack reads)
            c.copy_from_slice(&columns(&rows));
            let p = prepare(s, c, n, 2, &spec(2), None, Order::Canonical);
            assert!(p.in_order);
            let (packed, stats) = p.place_in(&block);
            assert_eq!(read(packed), rows, "every value, by get");
            let (c0, c1) = (packed.col(0), packed.col(1));
            // The first column split; the child's, a few past its parent's, smallest whole and relative.
            assert_eq!((c0.form().as_str(), c1.form().as_str()), ("split", "pack rel"), "the forms chosen");
            let mut got = Vec::new();
            c1.for_each(|v| got.push(v));
            assert_eq!(got, rows.iter().map(|r| r[1]).collect::<Vec<_>>(), "every value, by for_each");
            for (i, r) in rows.iter().enumerate().step_by(7) {
                let at: Vec<usize> = packed.probe(&[0], &[r[0]]).collect();
                assert_eq!(at, vec![i], "the order's probe of row {i}");
                assert_eq!(c0.dec().lower_bound(0, n, r[0] + 1), i + 1, "a bound between ids");
            }
            assert_eq!(packed.probe(&[0], &[rows[0][0] - 1]).count(), 0, "an id before the first");
            // In an order on both columns, the second relative and searched within its first column's range.
            let both = Spec { order: vec![0, 1], ..spec(2) };
            let c = s.take::<i64>(2 * n);
            // copy: load (the test's rows into the scratch the pack reads)
            c.copy_from_slice(&columns(&rows));
            let held = Block::anon(kind::KEPT, 64 << 20);
            let (pb, _) = prepare(s, c, n, 2, &both, None, Order::Canonical).place_in(&held);
            assert_eq!(pb.col(1).form(), "pack rel", "an order column after column 0, relative");
            assert_eq!(read(pb), rows, "every value, by get");
            for (i, r) in rows.iter().enumerate().step_by(5) {
                assert_eq!(pb.probe(&[0, 1], &[r[0], r[1]]).collect::<Vec<_>>(), vec![i], "the probe of row {i}");
                assert_eq!(pb.probe(&[1, 0], &[r[1], r[0]]).collect::<Vec<_>>(), vec![i], "its columns either way");
                for miss in [r[1] - 1, r[1] + (1 << 20), i64::MIN, i64::MAX] {
                    assert_eq!(pb.probe(&[0, 1], &[r[0], miss]).count(), 0, "row {i}'s first column with {miss}");
                }
                let (lo, hi) = pb.col(0).dec().equal_range(0, n, r[0]);
                let d1 = pb.col(1).dec();
                assert_eq!(d1.lower_bound(lo, hi, r[1]), i, "a bound at row {i}");
                assert_eq!(d1.lower_bound(lo, hi, i64::MIN), lo, "a bound before them all");
                assert_eq!(d1.lower_bound(lo, hi, i64::MAX), hi, "a bound past them all");
            }
            // From a cursor (a merge join's inner side): keys in order, out of order, repeated, and absent, from any
            // start, answer as the whole search does.
            let d = c0.dec();
            let mut keys: Vec<i64> = rows.iter().step_by(3).map(|r| r[0]).collect();
            keys.extend([rows[5][0], rows[5][0], rows[0][0] - 1, rows[n - 1][0] + 1, rows[n / 2][0] + 1, rows[1][0]]);
            let mut at = 0usize;
            for (j, &key) in keys.iter().enumerate() {
                let want = d.equal_range(0, n, key);
                assert_eq!(d.equal_range_from(at, key), want, "key {j} from {at}");
                assert_eq!(d.equal_range_from(n + 5, key), want, "key {j} from past the end");
                at = want.0;
            }
            let whole: usize = [0, 1].iter().map(|&k| choose(&rows.iter().map(|r| r[k]).collect::<Vec<_>>()).bytes()).sum();
            let split: usize = stats.cols.iter().map(|c| c.2).sum();
            assert!(split * 2 < whole, "split {split} bytes against whole {whole}");
        });
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
            let p = prepare(s, c, 3000, 2, &spec(2), Some(&levels[..]), Order::Canonical);
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
            assert!(prepare(s, c, 3000, 2, &spec(2), None, Order::Canonical).place_in(&block).0.levels().is_none());
        });
    }
}
