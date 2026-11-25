//! A packed relation: a frozen relation written again, exactly sized, in the fewest bits.
//!
//! Its rows are its live rows in the order `prepare` was asked for (`Order`), so a row's position is its place in
//! that order: sorted on the spec's one sort column (a relation's, and a kept relation's: decision 0678 step 8), or
//! on every row (the store's own tables, which it merges and probes on leading columns), or as given.
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
use crate::relation::{Spec, View};
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

/// The canonical order's columns: the sort column, then the rest in order (all in order with none).
pub(crate) fn canonical(spec: &Spec, a: usize) -> Vec<usize> {
    let s = spec.sort.map(|c| c as usize);
    s.into_iter().chain((0..a).filter(|&c| Some(c) != s)).collect()
}

/// The order a packed relation records (`Packed::okey`): in `Order::Rows`, the canonical order's first `MAX_KEY`
/// columns; in `Order::Column`, the sort column; none as given, or with no sort column.
fn recorded(spec: &Spec, a: usize, kind: Order) -> Vec<u16> {
    match (kind, spec.sort) {
        (Order::AsGiven, _) | (_, None) => Vec::new(),
        (Order::Column, Some(c)) => vec![c],
        (Order::Rows, Some(_)) => canonical(spec, a).into_iter().take(crate::relation::MAX_KEY).map(|c| c as u16).collect(),
    }
}

/// Whether index `cols` is answered by the order recorded (`lead`): its columns are its first ones, in any order.
fn on_order(lead: &[u16], cols: &[u16]) -> bool {
    cols.len() <= lead.len() && cols.iter().all(|c| lead[..cols.len()].contains(c))
}

/// A permutation index planned: its column, and its keys, starts and positions with their encodings.
struct IxPlan<'s> {
    col: u32,
    parts: [(&'s [i64], Choice); 3],
}

/// Everything chosen before it is placed, so the block is made exactly its size: the columns (each read where its
/// values lie, in canonical order) and their encodings, and the indexes.
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

/// Rows the packer reads where they lie (decision 0353 step 8): `n` rows of `arity` columns, row `r`'s value in
/// column `c`. A kept relation's live rows, an input's granule read through its source, or column-major values.
pub trait RowsIn: Sync {
    fn n(&self) -> usize;
    fn arity(&self) -> usize;
    fn get(&self, r: usize, c: usize) -> i64;
}

/// Where a part's values are read: column-major values (`n` rows, column `c` at `raw[c * n..]`), or rows read where
/// they lie.
#[derive(Clone, Copy)]
enum Src<'s> {
    Cols { raw: &'s [i64], n: usize },
    Rows(&'s dyn RowsIn),
}

impl Src<'_> {
    #[inline]
    fn get(&self, r: usize, c: usize) -> i64 {
        match self {
            Src::Cols { raw, n } => raw[c * n + r],
            Src::Rows(rows) => rows.get(r, c),
        }
    }
}

/// How a part reads its column (`PCol`'s `split` and `rel`): whole, the high or low part of a split value, and each
/// less column 0's same part at the row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Whole,
    Hi,
    Lo,
    RelWhole,
    RelHi,
    RelLo,
}

/// A column's values as a part packs them, read in place (decision 0353 step 8): column `c` of `src`, its rows in
/// `order` (positions in `src`) or as they lie, read as `form` says. Nothing is made of them first.
#[derive(Clone, Copy)]
struct Part<'s> {
    src: Src<'s>,
    order: Option<&'s [u32]>,
    n: usize,
    c: usize,
    form: Form,
}

impl crate::col::Vals for Part<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.n
    }
    #[inline]
    fn at(&self, i: usize) -> i64 {
        let r = self.order.map_or(i, |o| o[i] as usize);
        let x = self.src.get(r, self.c);
        let b = if matches!(self.form, Form::RelWhole | Form::RelHi | Form::RelLo) { self.src.get(r, 0) } else { 0 };
        form_of(self.form, x, b)
    }
}

/// What a column's job reads: a part of a column, or values derived from them (an index's, the levels), made once.
#[derive(Clone, Copy)]
enum JobVals<'s> {
    Part(Part<'s>),
    Made(&'s [i64]),
}

impl crate::col::Vals for JobVals<'_> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            JobVals::Part(p) => p.n,
            JobVals::Made(v) => v.len(),
        }
    }
    #[inline]
    fn at(&self, i: usize) -> i64 {
        match self {
            JobVals::Part(p) => crate::col::Vals::at(p, i),
            JobVals::Made(v) => v[i],
        }
    }
}

/// A column planned: its form (`PCol`'s `split` and `rel`) and its parts with their encodings (one, or two when split).
struct ColPlan<'s> {
    split: u32,
    rel: bool,
    parts: Vec<(Part<'s>, Choice)>,
    /// Its greatest value, `None` with no rows: found as its form is chosen, so a kept entry records its atoms'
    /// greatest without reading them again (`Prepared::col_max`).
    max: Option<i64>,
}

impl ColPlan<'_> {
    /// What it takes in a block: each part's words, and the alignment a part's placement may cost.
    fn cost(parts: &[(Part<'_>, Choice)]) -> usize {
        parts.iter().map(|(_, c)| c.bytes() + 2 * 64).sum()
    }
}

/// Column `c`'s value read as `form`, from its value `x` and column 0's `b` at the row.
#[inline]
fn form_of(form: Form, x: i64, b: i64) -> i64 {
    let (s, m) = (ID_SPLIT, (1i64 << ID_SPLIT) - 1);
    match form {
        Form::Whole => x,
        Form::Hi => x >> s,
        Form::Lo => x & m,
        Form::RelWhole => x.wrapping_sub(b),
        Form::RelHi => (x >> s).wrapping_sub(b >> s),
        Form::RelLo => (x & m) - (b & m),
    }
}

/// Column `c`'s form and encodings, the smallest of: whole; split at `ID_SPLIT`; and, where `rel` may be (`Rel`),
/// each of those less column 0's. Only a column whose values span past `2^ID_SPLIT` is tried split or relative:
/// below it its high part is constant. Every form's statistics come from one pass over the rows, each row's value
/// and column 0's read once where they lie (decision 0353 step 8; `col::Stats` is `choose`'s).
fn plan_col<'s>(src: Src<'s>, order: Option<&'s [u32]>, n: usize, c: usize, may_rel: Rel) -> ColPlan<'s> {
    use crate::col::{Stats, Vals};
    let rel = may_rel != Rel::Never && c > 0;
    let forms: &[Form] = if rel { &[Form::Whole, Form::Hi, Form::Lo, Form::RelWhole, Form::RelHi, Form::RelLo] } else { &[Form::Whole, Form::Hi, Form::Lo] };
    let x = Part { src, order, n, c, form: Form::Whole };
    let b = Part { src, order, n, c: 0, form: Form::Whole };
    let read = |i: usize| (x.at(i), if rel { b.at(i) } else { 0 });
    let ends: Vec<(i64, i64)> = if n == 0 {
        vec![(0, 0); forms.len()]
    } else {
        let ((x0, b0), (x1, b1)) = (read(0), read(n - 1));
        forms.iter().map(|&f| (form_of(f, x0, b0), form_of(f, x1, b1))).collect()
    };
    // Each piece: every form's statistics, and whether a difference from column 0 wraps.
    let pieces = crate::col::over_pieces(n, |r| {
        let before = (r.start > 0).then(|| read(r.start - 1));
        let mut st: Vec<Stats> = forms.iter().zip(&ends).map(|(&f, &(first, last))| Stats::piece(first, last, n, before.map(|(x, b)| form_of(f, x, b)))).collect();
        let mut wraps = false;
        for i in r {
            let (x, b) = read(i);
            for (k, &f) in forms.iter().enumerate() {
                st[k].push(i, form_of(f, x, b));
            }
            wraps |= rel && x.checked_sub(b).is_none();
        }
        (st, wraps)
    });
    let mut joined: Option<Vec<Stats>> = None;
    let mut wraps = false;
    for (st, w) in pieces {
        wraps |= w;
        joined = Some(match joined {
            None => st,
            Some(j) => j.into_iter().zip(st).map(|(a, b)| a.join(b)).collect(),
        });
    }
    let stats = joined.unwrap_or_default();
    let part = |k: usize| -> (Part<'s>, Choice) { (Part { src, order, n, c, form: forms[k] }, stats.get(k).map_or_else(|| choose(&x), |s| s.choice(n))) };
    let (lo, hi) = stats.first().map_or((0, 0), |s| s.range());
    let max = (n > 0).then_some(hi);
    let whole = ColPlan { split: 0, rel: false, parts: vec![part(0)], max };
    if n < 2 || (hi as i128 - lo as i128) < 1 << ID_SPLIT {
        return whole;
    }
    let s = ID_SPLIT;
    let mut best = whole;
    let mut take = |cand: ColPlan<'s>| {
        if ColPlan::cost(&cand.parts) < ColPlan::cost(&best.parts) {
            best = cand;
        }
    };
    take(ColPlan { split: s, rel: false, parts: vec![part(1), part(2)], max });
    if rel {
        // A searched column keeps its order less a constant only where no difference wraps; the split parts' never
        // do (each part is under 2^46).
        if may_rel == Rel::Any || !wraps {
            take(ColPlan { split: 0, rel: true, parts: vec![part(3)], max });
        }
        take(ColPlan { split: s, rel: true, parts: vec![part(4), part(5)], max });
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

fn plan<'s>(scratch: &'s Scratch, arity: usize, spec: &Spec, order_kind: Order, src: Src<'s>, order: Option<&'s [u32]>, n: usize) -> Plan<'s> {
    use crate::col::Vals;
    let mut indexes = Vec::new();
    let lead = recorded(spec, arity, order_kind);
    for ix in &spec.indexes {
        if on_order(&lead, ix) {
            continue;
        }
        assert_eq!(ix.len(), 1, "a packed index off the order is on one column");
        let c = ix[0] as usize;
        let v = Part { src, order, n, c, form: Form::Whole };
        // Positions grouped by the column's value: a radix sort of the values (less their minimum), stable, so
        // positions stay in canonical order within a value.
        let lo = (0..n).map(|i| v.at(i)).min().unwrap_or(0);
        let hi = (0..n).map(|i| v.at(i)).max().unwrap_or(0);
        let keys: &mut [u64] = scratch.take(n);
        let perm: &mut [u32] = scratch.take(n);
        for (i, (k, p)) in keys.iter_mut().zip(perm.iter_mut()).enumerate() {
            *k = (v.at(i) as i128 - lo as i128) as u64;
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
        _ if !lead.contains(&(c as u16)) => Rel::Any,
        _ if lead.first() == Some(&0) => Rel::Searched,
        _ => Rel::Never,
    };
    let choices = each(n * arity >= PAR_MIN, arity, |c| plan_col(src, order, n, c, rel(c)));
    Plan { arity, n, choices, indexes, levels: None }
}

/// Lays out the block (descriptors, and every column's words, zero), then packs the columns into it in
/// parallel: each column into its own words.
fn place(plan: &Plan, okey: &[u16], kind: u32) -> (Block, PackStats) {
    let block = crate::alloc::host(kind, plan.bytes());
    let p = place_at(&block, plan, okey);
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
type Job<'a> = (&'a Choice, JobVals<'a>, &'a mut [u64], &'a mut [u64]);

/// Places `plan` in `block` (open): its descriptor first, then its columns' words, all from one point on, so the
/// packed relation is one contiguous range of the block (decision 0456 K4: it moves by `memcpy`).
fn place_at(block: &Block, plan: &Plan, okey: &[u16]) -> *mut Packed {
    let p: *mut Packed = block.alloc_t(1);
    let cp: *mut PCol = block.alloc_t(plan.arity);
    let ip: *mut PIndex = block.alloc_t(plan.indexes.len().max(1));
    let n = plan.n;
    let mut jobs: Vec<Job> = Vec::new();
    unsafe {
        (*p).arity = plan.arity as u32;
        (*p).n = n as u32;
        (*p).nokey = okey.len() as u32;
        for (i, &c) in okey.iter().enumerate() {
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
                jobs.push((ch, JobVals::Part(*v), d, r));
            }
        }
        for (x, ixp) in plan.indexes.iter().enumerate() {
            let ix = ip.add(x);
            (*ix).col = ixp.col;
            let [(k, kc), (s, sc), (m, mc)] = &ixp.parts;
            for (at, v, ch) in [(&raw mut (*ix).keys, k, kc), (&raw mut (*ix).starts, s, sc), (&raw mut (*ix).perm, m, mc)] {
                let (d, r) = IntCol::lay_out(at, block, ch);
                jobs.push((ch, JobVals::Made(v), d, r));
            }
        }
        (*p).nindex = plan.indexes.len() as u32;
        Span::set(&raw mut (*p).cols, cp, plan.arity);
        Span::set(&raw mut (*p).indexes, ip, plan.indexes.len());
        if let Some((v, ch)) = &plan.levels {
            let (d, r) = IntCol::lay_out(&raw mut (*p).levels, block, ch);
            jobs.push((ch, JobVals::Made(v), d, r));
        }
    }
    let write = |(ch, v, d, r): Job| crate::col::write(ch, &v, d, r);
    if n * plan.arity >= PAR_MIN { rerail_context::on_cores(|| jobs.into_par_iter().for_each(write)) } else { jobs.into_iter().for_each(write) }
    p
}

/// Packs what `view` reads (a full relation, or a base and its delta) into a new block of its exact size. A full
/// frozen relation's columns are read a chunk at a time; a base and its delta through their merged order, which is
/// the sort column's already, so nothing is sorted (`Order::Column`'s check).
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
        let prepared = prepare_src(s, Src::Cols { raw: cols, n }, n, a, spec, None, Order::Column);
        place(&prepared.plan, &prepared.order, kind)
    })
}

/// Packs an open relation's rows directly, never freezing them in place: each column read once, sorted on the sort
/// column and deduplicated (`Order::Column`), encoded.
pub fn pack_open(rel: &crate::relation::Relation, spec: &Spec, kind: u32) -> (Block, PackStats) {
    assert!(!rel.is_delta(), "a delta is packed through its view, with its base");
    crate::scratch::with(|s| {
        let a = rel.arity as usize;
        let n = rel.rows.len() as usize;
        let rows = &rel.rows;
        let raw: &mut [i64] = s.take(n * a);
        let read = |(c, col): (usize, &mut [i64])| rows.read_col(c, n as u32, col);
        if n * a >= PAR_MIN {
            rerail_context::on_cores(|| raw.par_chunks_mut(n.max(1)).enumerate().for_each(read))
        } else {
            raw.chunks_mut(n.max(1)).enumerate().for_each(read)
        }
        let raw = &*raw;
        let prepared = prepare_src(s, Src::Cols { raw, n }, n, a, spec, None, Order::Column);
        place(&prepared.plan, &prepared.order, kind)
    })
}

/// Rows ready to be placed (decision 0456 K3): in canonical order, without duplicates, each column's encoding chosen.
pub struct Prepared<'s> {
    plan: Plan<'s>,
    order: Vec<u16>,
    /// Whether the rows came in canonical order and distinct, so were not sorted.
    pub in_order: bool,
}

/// Whether `src`'s `n` rows are strictly increasing in `key`'s order: in canonical order, distinct.
fn in_canonical_order(src: Src<'_>, n: usize, key: &[usize]) -> bool {
    (1..n).all(|i| {
        for &c in key {
            let (x, y) = (src.get(i - 1, c), src.get(i, c));
            if x != y {
                return x < y;
            }
        }
        false
    })
}

/// The order `prepare` lays rows out in. None sorts rows already in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Sorted on the spec's sort column, stably (none: as given); duplicates dropped by hashing the rows. The order
    /// recorded is that column. A relation's (decision 0678 step 8).
    Column,
    /// Sorted on every column (the spec's sort column, then the rest); duplicates dropped. The order recorded is its
    /// first `MAX_KEY` columns: a table the store merges with others on leading columns (`sorted_put`).
    Rows,
    /// As given, every row; no order recorded.
    AsGiven,
}

/// `n` rows of `a` columns (`raw`, column-major), with each row's level when they have one, prepared to be placed:
/// `prepare_rows` over them.
pub fn prepare<'s>(s: &'s Scratch, raw: &'s [i64], n: usize, a: usize, spec: &Spec, levels: Option<&[u32]>, given: Order) -> Prepared<'s> {
    assert_eq!(raw.len(), n * a);
    prepare_src(s, Src::Cols { raw, n }, n, a, spec, levels, given)
}

/// `rows`, with each row's level when they have one, prepared to be placed, read where they lie (decision 0353
/// step 8: nothing is read into columns first): one pass checks whether they are in canonical order and distinct
/// already (an input's granule usually is, and a relation that declares its output sorted, K7, always); else their
/// positions are sorted as `pack_open` sorts its rows, by the packed key or by comparing rows, and their duplicates
/// dropped, the levels following their rows. Then each column's encoding is chosen, read in that order.
///
/// `Order::AsGiven`: the rows are kept in the order given, none dropped, and the packed rows record no order (an
/// input's granule: its rows by owner, its readers' ranges by position, decision 0117 step 2).
pub fn prepare_rows<'s>(s: &'s Scratch, rows: &'s dyn RowsIn, spec: &Spec, levels: Option<&[u32]>, given: Order) -> Prepared<'s> {
    prepare_src(s, Src::Rows(rows), rows.n(), rows.arity(), spec, levels, given)
}

fn prepare_src<'s>(s: &'s Scratch, src: Src<'s>, n: usize, a: usize, spec: &Spec, levels: Option<&[u32]>, given: Order) -> Prepared<'s> {
    assert!(levels.is_none_or(|l| l.len() == n), "a level a row");
    if given == Order::Column {
        return prepare_column(s, src, n, a, spec, levels);
    }
    let key = canonical(spec, a);
    let as_given = given == Order::AsGiven;
    let in_order = as_given || in_canonical_order(src, n, &key);
    let (order, lv, m): (Option<&'s [u32]>, Option<&'s [i64]>, usize) = if in_order {
        let lv = levels.map(|l| {
            let out: &mut [i64] = s.take(n);
            for (o, &x) in out.iter_mut().zip(l) {
                *o = x as i64;
            }
            &*out
        });
        (None, lv, n)
    } else {
        let range = |c: usize| (0..n).fold((i64::MAX, i64::MIN), |(a, b), r| {
            let v = src.get(r, c);
            (a.min(v), b.max(v))
        });
        let (lo, hi): (Vec<i64>, Vec<i64>) = (0..a).map(range).unzip();
        let ids: &mut [u32] = s.take(n);
        for (i, id) in ids.iter_mut().enumerate() {
            *id = i as u32;
        }
        let keys = sort_rows(s, src, n, &key, &lo, &hi, ids);
        let order: &mut [u32] = s.take(n);
        let mut m = 0;
        for p in 0..n {
            let same = p > 0 && match keys {
                Some(k) => k[p] == k[p - 1],
                None => key.iter().all(|&c| src.get(ids[p] as usize, c) == src.get(ids[p - 1] as usize, c)),
            };
            if !same {
                order[m] = ids[p];
                m += 1;
            }
        }
        let order = &order[..m];
        let lv = levels.map(|l| {
            let out: &mut [i64] = s.take(m);
            for (o, &i) in out.iter_mut().zip(order) {
                *o = l[i as usize] as i64;
            }
            &*out
        });
        (Some(order), lv, m)
    };
    let mut plan = plan(s, a, spec, given, src, order, m);
    plan.levels = lv.map(|v| (v, choose(v)));
    let order = recorded(spec, a, given);
    Prepared { plan, order, in_order: in_order && !as_given }
}

/// `prepare_src` in `Order::Column`: the rows sorted on the spec's one column unless they lie in its order already,
/// and distinct by a table of their hashes (`relation::distinct_sorted`), never sorted on every column.
fn prepare_column<'s>(s: &'s Scratch, src: Src<'s>, n: usize, a: usize, spec: &Spec, levels: Option<&[u32]>) -> Prepared<'s> {
    let sort = spec.sort.map(|c| c as usize).filter(|&c| c < a);
    let range = sort.map(|c| {
        let (lo, hi) = (0..n).fold((i64::MAX, i64::MIN), |(l, h), r| {
            let v = src.get(r, c);
            (l.min(v), h.max(v))
        });
        (c, lo, hi.max(lo))
    });
    let ids: &mut [u32] = s.take(n);
    for (i, id) in ids.iter_mut().enumerate() {
        *id = i as u32;
    }
    let m = crate::relation::distinct_sorted(s, |c, i| src.get(i, c), a, range, ids);
    let ids = &ids[..m];
    let in_order = m == n && ids.iter().enumerate().all(|(i, &id)| id as usize == i);
    let order = (!in_order).then_some(ids);
    let lv = levels.map(|l| {
        let out: &mut [i64] = s.take(m);
        for (p, o) in out.iter_mut().enumerate() {
            *o = l[order.map_or(p, |ids| ids[p] as usize)] as i64;
        }
        &*out
    });
    let mut plan = plan(s, a, spec, Order::Column, src, order, m);
    plan.levels = lv.map(|v| (v, choose(v)));
    let order = recorded(spec, a, Order::Column);
    Prepared { plan, order, in_order }
}

/// Sorts `ids` (positions of `src`'s rows, column `c` ranging over `lo[c]..=hi[c]`) into the canonical order of
/// `key`, reading the rows where they lie: the packed keys radix sorted
/// with the ids and returned alongside them when the key packs into 64 bits, else the ids sorted by comparing rows.
fn sort_rows<'s>(scratch: &'s Scratch, src: Src<'_>, n: usize, key: &[usize], lo: &[i64], hi: &[i64], ids: &mut [u32]) -> Option<&'s [u64]> {
    let m = ids.len();
    let bits_for = |range: u64| 64 - range.leading_zeros();
    let kbits: Vec<u32> = key.iter().map(|&c| if n == 0 { 0 } else { bits_for((hi[c] as i128 - lo[c] as i128) as u64) }).collect();
    let packed_bits: u32 = kbits.iter().sum();
    if packed_bits > 64 {
        ids.sort_unstable_by(|&x, &y| {
            for &c in key {
                let o = src.get(x as usize, c).cmp(&src.get(y as usize, c));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            std::cmp::Ordering::Equal
        });
        return None;
    }
    let keys: &mut [u64] = scratch.take(m);
    for (k, &i) in keys.iter_mut().zip(ids.iter()) {
        let mut x = 0u64;
        for (j, &c) in key.iter().enumerate() {
            // A column of one value takes no bits; `checked_shl` keeps a 64-bit shift of an empty key defined.
            x = x.checked_shl(kbits[j]).unwrap_or(0) | (src.get(i as usize, c) as i128 - lo[c] as i128) as u64;
        }
        *k = x;
    }
    let (tk, ti) = (scratch.take::<u64>(m), scratch.take::<u32>(m));
    crate::sort::radix_pairs(keys, ids, packed_bits, tk, ti);
    Some(keys)
}

impl Prepared<'_> {
    /// Rows after duplicates were dropped.
    pub fn len(&self) -> usize {
        self.plan.n
    }

    pub fn is_empty(&self) -> bool {
        self.plan.n == 0
    }

    /// Its columns.
    pub fn arity(&self) -> usize {
        self.plan.arity
    }

    /// Column `c`'s greatest value, `None` with no rows: what choosing its encoding found.
    pub fn col_max(&self, c: usize) -> Option<i64> {
        self.plan.choices[c].max
    }

    /// The most bytes placing it takes.
    pub fn bytes(&self) -> usize {
        self.plan.bytes()
    }

    /// Places it in `block` (open), one contiguous range: the packed relation, and what it wrote.
    pub fn place_in<'b>(&self, block: &'b Block) -> (&'b Packed, PackStats) {
        let before = block.used();
        let p = place_at(block, &self.plan, &self.order);
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

    /// The positions whose columns `cols` equal `key`, in order: a range of the order's leading columns the key holds
    /// (each of the key's other columns checked within it: a relation's entry records its one sort column), else
    /// through the column's permutation index.
    pub fn probe(&self, cols: &[u16], key: &[i64]) -> Probe<'_> {
        let ok = &self.okey[..self.nokey as usize];
        let lead = ok.iter().take_while(|c| cols.contains(c)).count();
        if lead > 0 {
            let (mut lo, mut hi) = (0usize, self.len());
            for &oc in &ok[..lead] {
                let k = key[cols.iter().position(|&c| c == oc).unwrap()];
                (lo, hi) = self.col(oc as usize).dec().equal_range(lo, hi, k);
            }
            let mut rest = [(0u16, 0i64); crate::relation::MAX_KEY];
            let mut m = 0;
            for (&c, &k) in cols.iter().zip(key) {
                if !ok[..lead].contains(&c) {
                    rest[m] = (c, k);
                    m += 1;
                }
            }
            if m == 0 {
                return Probe::Range(lo..hi);
            }
            return Probe::Checked(self, lo..hi, rest, m);
        }
        assert_eq!(cols.len(), 1, "a key of several columns leads with the order's first");
        let ix = self.indexes.as_slice().iter().find(|ix| ix.col == cols[0] as u32).expect("an index on the column");
        let keys = ix.keys.dec();
        let k = keys.lower_bound(0, keys.len(), key[0]);
        if k >= keys.len() || keys.get(k) != key[0] {
            return Probe::Range(0..0);
        }
        let starts = ix.starts.dec();
        Probe::Perm(ix.perm.dec(), starts.get(k) as usize..starts.get(k + 1) as usize)
    }

    /// A digest of the rows: the same as the view's it was packed from (`View::digest`: a sum of each row's hash).
    pub fn digest(&self) -> u64 {
        let mut sum = 0u64;
        for p in 0..self.len() {
            sum = sum.wrapping_add(crate::relation::row_digest((0..self.arity as usize).map(|c| self.get(p, c))));
        }
        crate::mix(sum, self.len() as i64)
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
            let (d, n, mut buf) = (self.dec(), self.len(), [0i64; FILL]);
            for at in (0..n).step_by(FILL) {
                let m = (n - at).min(FILL);
                d.fill(at, &mut buf[..m]);
                buf[..m].iter().for_each(|&v| f(v));
            }
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

/// The values `ColDec::fill` decodes at once: a chunk of rows read column by column, then row by row.
pub const FILL: usize = 256;

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

    /// `whole`'s values `at..at + out.len()`, `low` the low part's scratch.
    fn whole_fill((hi, lo, s): &Parts<'_>, at: usize, out: &mut [i64], low: &mut [i64]) {
        hi.fill(at, out);
        if let Some(lo) = lo {
            lo.fill(at, low);
            out.iter_mut().zip(low.iter()).for_each(|(o, l)| *o = *o << s | l);
        }
    }

    /// Values `at..at + out.len()` into `out`, in order, as `get` gives them: each part decoded as it goes
    /// (`Dec::fill`), at most `FILL` at once.
    pub fn fill(&self, at: usize, out: &mut [i64]) {
        let n = out.len();
        assert!(n <= FILL, "at most FILL values at once");
        let mut low = [0i64; FILL];
        let Some(base) = &self.base else { return Self::whole_fill(&self.own, at, out, &mut low[..n]) };
        let mut b = [0i64; FILL];
        Self::whole_fill(base, at, &mut b[..n], &mut low[..n]);
        let (hi, lo, s) = &self.own;
        hi.fill(at, out);
        match lo {
            None => out.iter_mut().zip(&b[..n]).for_each(|(o, b)| *o = o.wrapping_add(*b)),
            Some(lo) => {
                lo.fill(at, &mut low[..n]);
                let m = (1i64 << s) - 1;
                for ((o, b), l) in out.iter_mut().zip(&b[..n]).zip(&low[..n]) {
                    *o = o.wrapping_add(b >> s) << s | (l + (b & m));
                }
            }
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
    /// A range of the order's leading columns, each position's other key columns checked (the first `usize`).
    Checked(&'a Packed, std::ops::Range<usize>, [(u16, i64); crate::relation::MAX_KEY], usize),
}

impl Iterator for Probe<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            Probe::Range(r) => r.next(),
            Probe::Perm(p, r) => r.next().map(|i| p.get(i) as usize),
            Probe::Checked(p, r, rest, m) => r.find(|&i| rest[..*m].iter().all(|&(c, k)| p.get(i, c as usize) == k)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;

    fn spec(a: usize) -> Spec {
        Spec { widths: vec![8; a], sort: Some(0), indexes: Vec::new(), min_log: 0 }
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
            let pa = prepare(s, ca, 5000, 2, &spec(2), None, Order::Column);
            let cb = s.take::<i64>(10_200);
            cb.copy_from_slice(&columns(&shuffled));
            let pb = prepare(s, cb, 5100, 2, &spec(2), None, Order::Column);
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
                    let id = (doc << 32) | (slot << 18) | (index * 3);
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
            let p = prepare(s, c, n, 2, &spec(2), None, Order::Column);
            assert!(p.in_order);
            let (packed, stats) = p.place_in(&block);
            assert_eq!(read(packed), rows, "every value, by get");
            let (c0, c1) = (packed.col(0), packed.col(1));
            // The first column split; the child's, a few past its parent's, smallest whole and relative.
            assert_eq!((c0.form().as_str(), c1.form().as_str()), ("split", "pack rel"), "the forms chosen");
            let mut got = Vec::new();
            c1.for_each(|v| got.push(v));
            assert_eq!(got, rows.iter().map(|r| r[1]).collect::<Vec<_>>(), "every value, by for_each");
            // A chunk at once from any place, split and relative (`ColDec::fill`), as `get` reads it.
            for c in [c0, c1] {
                let (d, mut buf) = (c.dec(), [0i64; FILL]);
                for at in (0..n).step_by(97) {
                    let m = (n - at).min(FILL);
                    d.fill(at, &mut buf[..m]);
                    assert_eq!(buf[..m].to_vec(), (at..at + m).map(|i| d.get(i)).collect::<Vec<_>>(), "{} from {at}", c.form());
                }
            }
            for (i, r) in rows.iter().enumerate().step_by(7) {
                let at: Vec<usize> = packed.probe(&[0], &[r[0]]).collect();
                assert_eq!(at, vec![i], "the order's probe of row {i}");
                assert_eq!(c0.dec().lower_bound(0, n, r[0] + 1), i + 1, "a bound between ids");
            }
            assert_eq!(packed.probe(&[0], &[rows[0][0] - 1]).count(), 0, "an id before the first");
            // In an order on both columns, the second relative and searched within its first column's range.
            let both = Spec { sort: Some(0), ..spec(2) };
            let c = s.take::<i64>(2 * n);
            // copy: load (the test's rows into the scratch the pack reads)
            c.copy_from_slice(&columns(&rows));
            let held = Block::anon(kind::KEPT, 64 << 20);
            let (pb, _) = prepare(s, c, n, 2, &both, None, Order::Column).place_in(&held);
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
            let p = prepare(s, c, 3000, 2, &spec(2), Some(&levels[..]), Order::Column);
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
            assert!(prepare(s, c, 3000, 2, &spec(2), None, Order::Column).place_in(&block).0.levels().is_none());
        });
    }
}
