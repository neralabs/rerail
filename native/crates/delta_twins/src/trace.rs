//! The trace's operations (decision 0686; *0686: Operations* §10): `consolidate` and `merge`.
//!
//! **consolidate** (scratch δ runs · write facts · rw control): a run's `(row, diff)` pairs into a sorted layer in
//! its own new block, over the trace's newest layer (`below`, reached through the descriptor table): the pairs sorted
//! in the trace's canonical order, each row's diffs summed, zeros dropped; each row gone given the id of the live row
//! it kills; the partition index with each partition's digest and counts; the flips; the shape's indexes; a sketch a
//! column. A change that is not a set change is refused into control: a row's diffs summing past ±1, a row gone that
//! is not live below, a row added that already is.
//!
//! **merge** (read facts, older generations by reference · write facts): the newest `count` layers of a trace into
//! one new layer over the layer below them: the same steps, its rows the merged layers' rows with their signs, so an
//! add and its later kill cancel. A roll-up: the merged generations are let go by the host after it.
//!
//! Both are the same phases over a source (`Src`): sort (chunks, then merge passes), segmented sums, emit (count,
//! scan, place), kills (a batched search of the layers below), partitions (heads counted, scanned, filled, their gone
//! rows scanned), flips (sorted), indexes (sorted), sketch (partials, then combined), seal (the header last). Each
//! phase's items are independent; what one phase leaves for the next lies in the new layer's block, past the layer
//! itself: the freeze trims it away.

use crate::sort::{CHUNK, chunks, merge_pairs, more, pairs, sort_chunks};
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::gens::{BODY_AT, Table, set_word, word, words, words_mut};
use rerail_blocks::bloom;
use rerail_blocks::layer::{
    Chain, IX_BLOOM, IX_FENCES, IX_HASH, IX_PRINTS, Layer, LayerOut, MAX_IX, NONE, PartW, Runs, ShapeW, SketchW, bloom_key, bloom_words,
    layer_bytes, layer_end, row_hash,
};
use rerail_blocks::order::{FENCE, print_vals, tag};
use std::cmp::Ordering;

crate::params! {
    /// `consolidate`'s params: the trace's newest layer, which the new one goes over (`NONE` for the first).
    pub struct ConsolidateP {
        pub below: u32,
        pub pad: u32,
    }
}

crate::params! {
    /// `merge`'s params: how many layers, from the read slot's down, are merged, and the layer below them (`NONE`).
    pub struct MergeP {
        pub count: u32,
        pub below: u32,
    }
}

/// Slots: the source (δ runs, or the newest layer merged), the new layer, and consolidate's control.
const SRC: usize = 0;
const OUT: usize = 1;
const CTL: usize = 2;

/// A refusal, in control's first word: none, a row's diffs summing past ±1, a row gone not live below, a row added
/// that already is.
pub const REFUSED_SUM: u64 = 1;
pub const REFUSED_GONE: u64 = 2;
pub const REFUSED_ADDED: u64 = 3;

/// Control's words: the refusal, the new layer's rows, its rows added and gone.
pub const CONTROL_BYTES: usize = 40;

// ---- the source ---------------------------------------------------------------------------------------------------

/// The rows a layer is made from: a run's δ runs, or the layers merged (newest first, each sorted). One a phase's
/// call, on its stack: the variants' sizes do not matter.
#[allow(clippy::large_enum_variant)]
enum Src<'a> {
    Runs(Runs<'a>),
    // copy: not block data (a handle and a first row a merged layer)
    Layers { layers: Vec<Layer<'a>>, off: Vec<usize> },
}

impl<'a> Src<'a> {
    fn n(&self) -> usize {
        match self {
            Src::Runs(r) => r.len(),
            Src::Layers { off, .. } => off[off.len() - 1],
        }
    }

    fn shape(&self) -> ShapeW {
        match self {
            Src::Runs(r) => r.head.shape,
            Src::Layers { layers, .. } => layers[0].head.shape,
        }
    }

    fn at(&self, i: usize) -> (Layer<'a>, usize) {
        let Src::Layers { layers, off } = self else { unreachable!() };
        let j = off.partition_point(|&o| o <= i) - 1;
        (layers[j], i - off[j])
    }

    fn get(&self, i: usize, c: usize) -> i64 {
        match self {
            Src::Runs(r) => r.get(i, c),
            Src::Layers { .. } => {
                let (l, j) = self.at(i);
                l.get(j, c)
            }
        }
    }

    fn diff(&self, i: usize) -> i32 {
        match self {
            Src::Runs(r) => r.diff(i),
            Src::Layers { .. } => {
                let (l, j) = self.at(i);
                if l.is_plus(j) { 1 } else { -1 }
            }
        }
    }

    /// Rows `i` and `j` in the canonical order.
    fn cmp(&self, i: u32, j: u32) -> Ordering {
        let s = self.shape();
        s.order().iter().map(|&c| self.get(i as usize, c as usize).cmp(&self.get(j as usize, c as usize))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
    }
}

/// Which operation a pipeline runs as: where its source and its params' layer below come from.
trait Op {
    fn src<'a>(src: &'a [u8], table: &Table<'a>, params: &[u8]) -> Src<'a>;
    fn below(params: &[u8]) -> u32;
    const CONTROL: bool;
}

/// `consolidate`.
pub struct Cons;
/// `merge`.
pub struct Merge;

impl Op for Cons {
    fn src<'a>(src: &'a [u8], _: &Table<'a>, _: &[u8]) -> Src<'a> {
        Src::Runs(Runs::new(src))
    }
    fn below(params: &[u8]) -> u32 {
        <ConsolidateP as crate::Params>::read(params).below
    }
    const CONTROL: bool = true;
}

impl Op for Merge {
    fn src<'a>(src: &'a [u8], table: &Table<'a>, params: &[u8]) -> Src<'a> {
        let p = <MergeP as crate::Params>::read(params);
        assert!(p.count >= 1, "a merge of one layer or more");
        let mut layers = Vec::with_capacity(p.count as usize);
        let mut off = vec![0usize];
        let mut l = Layer::new(src).expect("the newest merged layer, committed");
        for k in 0..p.count as usize {
            layers.push(l);
            off.push(off[k] + l.n());
            if k + 1 < p.count as usize {
                let below = l.below().expect("as many layers as merged");
                l = Layer::new(table.block(below).expect("a merged layer reached by reference")).expect("committed");
            }
        }
        assert_eq!(l.below().unwrap_or(NONE), p.below, "the merged layers lie over the layer below them");
        Src::Layers { layers, off }
    }
    fn below(params: &[u8]) -> u32 {
        <MergeP as crate::Params>::read(params).below
    }
    const CONTROL: bool = false;
}

// ---- the work, past the layer in the new block ---------------------------------------------------------------------

/// Where each work array lies, relative to the work's start (`base`, an absolute offset in the block).
#[derive(Clone, Copy)]
struct Work {
    base: usize,
    cap: usize,
    perm_a: usize,
    perm_b: usize,
    seg: usize,
    ccount: usize,
    osign: usize,
    pcount: usize,
    gcount: usize,
    f_a: usize,
    f_b: usize,
    ix_a: usize,
    ix_b: usize,
    sk: usize,
    len: usize,
}

/// Its header's words, at `base`.
const W_WIDTH: usize = 0;
const W_WHICH: usize = 8;
const W_NOUT: usize = 16;
const W_NPARTS: usize = 24;
const W_NGONE: usize = 32;
const W_REFUSED: usize = 40;
const W_FWIDTH: usize = 48;
const W_FWHICH: usize = 56;
const W_IWIDTH: usize = 64;
const W_IWHICH: usize = 72;

fn round64(n: usize) -> usize {
    n.div_ceil(64) * 64
}

fn work(shape: &ShapeW, cap: usize) -> Work {
    let base = round64(BODY_AT + layer_bytes(shape, cap));
    let mut at = 128;
    let mut next = |bytes: usize| {
        let a = at;
        at = round64(at + bytes);
        a
    };
    let ch = chunks(cap) as usize;
    let w = Work {
        base,
        cap,
        perm_a: next(4 * cap),
        perm_b: next(4 * cap),
        seg: next(4 * cap),
        ccount: next(8 * (ch + 1)),
        osign: next(4 * cap),
        pcount: next(8 * (ch + 1)),
        gcount: next(8 * (cap + 1)),
        f_a: next(4 * cap),
        f_b: next(4 * cap),
        ix_a: next(4 * cap * MAX_IX),
        ix_b: next(4 * cap * MAX_IX),
        sk: next(size_of::<SketchW>() * shape.arity() * ch.max(1)),
        len: 0,
    };
    Work { len: at, ..w }
}

/// The body bytes a new layer's block takes, for a source of `cap` rows of `shape`: the layer and its work.
pub fn out_bytes(shape: &ShapeW, cap: usize) -> usize {
    let w = work(shape, cap);
    w.base - BODY_AT + w.len
}

/// The phase's view: the source, the layer region of the new block, its work region, and the work's layout.
struct View<'a> {
    src: Src<'a>,
    layer: &'a mut [u8],
    work: &'a mut [u8],
    w: Work,
    below: u32,
}

fn view<'a, O: Op>(c: &'a mut Ctx<'_, '_, '_>) -> View<'a> {
    let (table, params) = (c.table, c.params);
    let ([s], [out]) = c.views([SRC], [OUT]);
    let src = O::src(s, table, params);
    let w = work(&src.shape(), src.n());
    let (layer, work) = out.split_at_mut(w.base);
    View { src, layer, work, w, below: O::below(params) }
}

impl View<'_> {
    fn word(&self, at: usize) -> u64 {
        word::<u64>(self.work, at)
    }

    fn set(&mut self, at: usize, v: u64) {
        set_word::<u64>(self.work, at, v);
    }

    /// The sorted positions: the array the last pass wrote.
    fn perm(&self) -> &[u32] {
        let at = if self.word(W_WHICH) == 0 { self.w.perm_a } else { self.w.perm_b };
        words(self.work, at, self.src.n())
    }

    fn seg(&self) -> &[i32] {
        words(self.work, self.w.seg, self.src.n())
    }

    fn n_out(&self) -> usize {
        self.word(W_NOUT) as usize
    }
}

/// The items of a phase, from the block's words: `f` of the view.
fn items_of<O: Op>(c: &Ctx<'_, '_, '_>, f: impl Fn(&Src<'_>, &[u8], &Work) -> usize) -> u32 {
    let src = O::src(c.read(SRC), c.table, c.params);
    let w = work(&src.shape(), src.n());
    f(&src, &c.read(OUT)[w.base..], &w) as u32
}

// ---- sort -----------------------------------------------------------------------------------------------------------

fn sort_first<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n = v.src.n();
    set_word::<u64>(v.work, W_WIDTH, CHUNK as u64);
    set_word::<u64>(v.work, W_WHICH, 0);
    let a = words_mut::<u32>(v.work, v.w.perm_a, n);
    for k in t.items() {
        for (i, p) in a.iter_mut().enumerate().take(((k + 1) * CHUNK).min(n)).skip(k * CHUNK) {
            *p = i as u32;
        }
    }
    sort_chunks(a, t, |x, y| v.src.cmp(x, y));
}

fn pass_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |s, wb, _| {
        let width = word::<u64>(wb, W_WIDTH) as usize;
        if more(s.n(), width) { pairs(s.n(), width) as usize } else { 0 }
    })
}

fn pass<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let (n, width, which) = (v.src.n(), v.word(W_WIDTH) as usize, v.word(W_WHICH));
    let (a, b) = v.work[v.w.perm_a..].split_at_mut(v.w.perm_b - v.w.perm_a);
    let (a, b) = (words_mut::<u32>(a, 0, n), words_mut::<u32>(b, 0, n));
    let cmp = |x: u32, y: u32| v.src.cmp(x, y);
    if which == 0 { merge_pairs(a, b, t, width, cmp) } else { merge_pairs(b, a, t, width, cmp) }
}

/// After a pass: the next width, and the array holding the runs flipped; whether another pass is due.
fn next_pass(work: &mut [u8], n: usize, width_at: usize, which_at: usize) -> bool {
    let width = word::<u64>(work, width_at) as usize;
    if !more(n, width) {
        return false;
    }
    set_word::<u64>(work, width_at, 2 * width as u64);
    set_word::<u64>(work, which_at, 1 - word::<u64>(work, which_at));
    more(n, 2 * width)
}

fn pass_next<O: Op>(c: &mut Ctx<'_, '_, '_>) -> bool {
    let v = view::<O>(c);
    let n = v.src.n();
    next_pass(v.work, n, W_WIDTH, W_WHICH)
}

// ---- segmented sums -----------------------------------------------------------------------------------------------

fn segments<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n = v.src.n();
    let which = v.word(W_WHICH);
    let (perms, rest) = v.work.split_at_mut(v.w.seg);
    let perm: &[u32] = words(perms, if which == 0 { v.w.perm_a } else { v.w.perm_b }, n);
    let src = &v.src;
    let same = |i: usize, j: usize| src.cmp(perm[i], perm[j]) == Ordering::Equal;
    let mut sums = Vec::with_capacity(t.items().len()); // copy: not block data (this tile's sums)
    let mut bad = false;
    for i in t.items() {
        if i > 0 && same(i, i - 1) {
            sums.push(0);
            continue;
        }
        let mut sum = 0i32;
        let mut j = i;
        while j < n && (j == i || same(j, i)) {
            sum += src.diff(perm[j] as usize);
            j += 1;
        }
        bad |= !(-1..=1).contains(&sum);
        // A sum of zero is dropped; past ±1, refused.
        sums.push(sum.clamp(-1, 1));
    }
    words_mut::<i32>(rest, 0, n)[t.items()].copy_from_slice(&sums);
    if bad && word::<u64>(perms, W_REFUSED) == 0 {
        set_word::<u64>(perms, W_REFUSED, REFUSED_SUM);
    }
}

// ---- emit: count, scan, place -------------------------------------------------------------------------------------

fn count<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n = v.src.n();
    let counts: Vec<u64> = t.items().map(|k| v.seg()[k * CHUNK..((k + 1) * CHUNK).min(n)].iter().filter(|&&s| s != 0).count() as u64).collect(); // copy: not block data (this tile's counts)
    let ch = chunks(n) as usize;
    words_mut::<u64>(v.work, v.w.ccount, ch + 1)[t.items()].copy_from_slice(&counts);
}

fn scan<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let mut v = view::<O>(c);
    let n = v.src.n();
    let ch = chunks(n) as usize;
    let cc = words_mut::<u64>(v.work, v.w.ccount, ch + 1);
    let mut sum = 0;
    for x in cc.iter_mut() {
        let k = *x;
        *x = sum;
        sum += k;
    }
    v.set(W_NOUT, sum);
    let below = v.below;
    let start = if below == NONE {
        0
    } else {
        let l = Layer::new(table.block(below).expect("the layer below, reached by reference")).expect("committed");
        layer_end(l.start(), l.head.n)
    };
    let shape = v.src.shape();
    LayerOut::init(v.layer, &shape, sum as usize, start, below);
}

fn place<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n = v.src.n();
    let ch = chunks(n) as usize;
    let perm = v.perm();
    let seg = v.seg();
    let offsets: &[u64] = words(v.work, v.w.ccount, ch + 1);
    let mut rows = Vec::new(); // copy: not block data (this tile's placements: out index, source row, sign)
    for k in t.items() {
        let mut o = offsets[k] as usize;
        for i in k * CHUNK..((k + 1) * CHUNK).min(n) {
            if seg[i] != 0 {
                rows.push((o, perm[i], seg[i]));
                o += 1;
            }
        }
    }
    let n_out = v.n_out();
    let mut out = LayerOut::new(v.layer);
    let a = out.head.shape.arity();
    for &(o, src, _) in &rows {
        for col in 0..a {
            out.set(o, col, v.src.get(src as usize, col));
        }
    }
    let signs = words_mut::<i32>(v.work, v.w.osign, n_out.max(1));
    for &(o, _, s) in &rows {
        signs[o] = s;
    }
}

fn signs<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let s: &[i32] = words(v.work, v.w.osign, n_out.max(1));
    let mut words_out = Vec::new(); // copy: not block data (this tile's sign words)
    for w in t.items() {
        let mut word = 0u64;
        for b in 0..64 {
            let o = 64 * w + b;
            if o < n_out && s[o] > 0 {
                word |= 1 << b;
            }
        }
        words_out.push(word);
    }
    let mut out = LayerOut::new(v.layer);
    out.plus()[t.items()].copy_from_slice(&words_out);
}

// ---- kills: a batched search of the layers below ------------------------------------------------------------------

fn kills<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let table = c.table;
    let View { layer, work, w, below, .. } = view::<O>(c);
    let n_out = word::<u64>(work, W_NOUT) as usize;
    let mut out = LayerOut::new(layer);
    let shape = out.head.shape;
    let below = (below != NONE).then_some(below);
    let chain = Chain::new(table, below, shape.arity()).expect("the layers below, reached by reference");
    let cols: Vec<ColumnIndex> = shape.order().iter().map(|&c| ColumnIndex::new(u16::from(c))).collect(); // copy: not block data (the probe's columns)
    let signs: &[i32] = words(work, w.osign, n_out.max(1));
    let mut refusal = 0;
    for o in t.items() {
        let key: Vec<i64> = cols.iter().map(|c| out.get(o, c.get())).collect(); // copy: not block data (a probe's key)
        let live = chain.matching(&cols, &key).next();
        out.kills()[o] = match (signs[o] < 0, live) {
            (true, Some(id)) => id.get(),
            (true, None) => {
                refusal = REFUSED_GONE;
                NONE
            }
            (false, Some(_)) => {
                refusal = REFUSED_ADDED;
                NONE
            }
            (false, None) => NONE,
        };
    }
    if refusal != 0 && word::<u64>(work, W_REFUSED) == 0 {
        set_word::<u64>(work, W_REFUSED, refusal);
    }
}

// ---- partitions ---------------------------------------------------------------------------------------------------

/// Whether out row `o` starts a partition: the first row, or (partitioned) a new partition value.
fn part_head(out: &LayerOut<'_>, o: usize) -> bool {
    o == 0 || out.head.shape.partition().is_some_and(|p| out.get(o, p) != out.get(o - 1, p))
}

fn part_key(out: &LayerOut<'_>, o: usize) -> i64 {
    out.head.shape.partition().map_or(0, |p| out.get(o, p))
}

fn pcount<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let out = LayerOut::new(v.layer);
    let counts: Vec<u64> = t.items().map(|k| (k * CHUNK..((k + 1) * CHUNK).min(n_out)).filter(|&o| part_head(&out, o)).count() as u64).collect(); // copy: not block data (this tile's counts)
    words_mut::<u64>(v.work, v.w.pcount, chunks(v.w.cap) as usize + 1)[t.items()].copy_from_slice(&counts);
}

fn pscan<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let mut v = view::<O>(c);
    let n_out = v.n_out();
    let pc = words_mut::<u64>(v.work, v.w.pcount, chunks(v.w.cap) as usize + 1);
    let mut sum = 0;
    for x in pc.iter_mut().take(chunks(n_out) as usize + 1) {
        let k = *x;
        *x = sum;
        sum += k;
    }
    v.set(W_NPARTS, sum);
}

fn pindex<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let View { layer, work, w, .. } = view::<O>(c);
    let n_out = word::<u64>(work, W_NOUT) as usize;
    let offsets: &[u64] = words(work, w.pcount, chunks(w.cap) as usize + 1);
    let mut out = LayerOut::new(layer);
    for k in t.items() {
        let mut e = offsets[k] as usize;
        for o in k * CHUNK..((k + 1) * CHUNK).min(n_out) {
            if part_head(&out, o) {
                let key = part_key(&out, o);
                let key = i32::try_from(key).expect("a partition column holds 32 bits");
                out.parts()[e] = PartW { key, lo: o as u32, ..PartW::default() };
                e += 1;
            }
        }
    }
}

fn nparts<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |_, wb, _| word::<u64>(wb, W_NPARTS) as usize)
}

fn pfill<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let (n_out, np) = (v.n_out(), v.word(W_NPARTS) as usize);
    let mut out = LayerOut::new(v.layer);
    let mut gone = Vec::new(); // copy: not block data (this tile's gone counts)
    for e in t.items() {
        let lo = out.parts()[e].lo as usize;
        let hi = if e + 1 < np { out.parts()[e + 1].lo as usize } else { n_out };
        // The partition's digest: each row's hash (the layer format's, `row_hash`) times its sign, summed.
        let a = out.head.shape.arity();
        let d = (lo..hi).fold(0u64, |d, o| {
            let h = row_hash((0..a).map(|c| out.get(o, c)));
            if out.is_plus(o) { d.wrapping_add(h) } else { d.wrapping_sub(h) }
        });
        let added = (lo..hi).filter(|&o| out.is_plus(o)).count() as u32;
        let p = &mut out.parts()[e];
        p.hi = hi as u32;
        p.added = added;
        p.gone = (hi - lo) as u32 - added;
        p.digest = d;
        gone.push(u64::from(p.gone));
    }
    words_mut::<u64>(v.work, v.w.gcount, v.w.cap + 1)[t.items()].copy_from_slice(&gone);
}

fn gscan<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let mut v = view::<O>(c);
    let np = v.word(W_NPARTS) as usize;
    let g = words_mut::<u64>(v.work, v.w.gcount, v.w.cap + 1);
    let mut sum = 0;
    for x in g.iter_mut().take(np + 1) {
        let k = *x;
        *x = sum;
        sum += k;
    }
    v.set(W_NGONE, sum);
}

/// Each partition's gone rows' kill ids, ascending, at its place in the per-partition flips: a partition an item.
fn pflips<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let View { layer, work, w, .. } = view::<O>(c);
    let offsets: &[u64] = words(work, w.gcount, w.cap + 1);
    let mut out = LayerOut::new(layer);
    for e in t.items() {
        let PartW { lo, hi, .. } = out.parts()[e];
        let mut ids: Vec<u32> = (lo as usize..hi as usize).filter(|&o| !out.is_plus(o)).map(|o| out.kill(o)).collect(); // copy: derived (the partition's flips, sorted into the layer)
        ids.sort_unstable();
        let flo = offsets[e] as usize;
        out.pflips()[flo..flo + ids.len()].copy_from_slice(&ids);
        let p = &mut out.parts()[e];
        p.flo = flo as u32;
        p.fhi = (flo + ids.len()) as u32;
    }
}

// ---- flips: the kill ids ascending ---------------------------------------------------------------------------------

fn ngone<O: Op>(c: &Ctx<'_, '_, '_>) -> usize {
    items_of::<O>(c, |_, wb, _| word::<u64>(wb, W_NGONE) as usize) as usize
}

fn fsort<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let View { layer, work, w, .. } = view::<O>(c);
    let g = word::<u64>(work, W_NGONE) as usize;
    set_word::<u64>(work, W_FWIDTH, CHUNK as u64);
    set_word::<u64>(work, W_FWHICH, 0);
    let mut out = LayerOut::new(layer);
    let src = out.pflips();
    let a = words_mut::<u32>(work, w.f_a, g.max(1));
    for k in t.items() {
        let r = k * CHUNK..((k + 1) * CHUNK).min(g);
        a[r.clone()].copy_from_slice(&src[r]); // copy: not block data (the flips' sort's work, from the partitions' flips)
    }
    sort_chunks(&mut a[..g], t, |x, y| x.cmp(&y));
}

fn fpass_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |_, wb, _| {
        let (g, width) = (word::<u64>(wb, W_NGONE) as usize, word::<u64>(wb, W_FWIDTH) as usize);
        if more(g, width) { pairs(g, width) as usize } else { 0 }
    })
}

fn fpass<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let (g, width, which) = (v.word(W_NGONE) as usize, v.word(W_FWIDTH) as usize, v.word(W_FWHICH));
    let (a, b) = v.work[v.w.f_a..].split_at_mut(v.w.f_b - v.w.f_a);
    let (a, b) = (words_mut::<u32>(a, 0, g), words_mut::<u32>(b, 0, g));
    if which == 0 { merge_pairs(a, b, t, width, |x, y| x.cmp(&y)) } else { merge_pairs(b, a, t, width, |x, y| x.cmp(&y)) }
}

fn fpass_next<O: Op>(c: &mut Ctx<'_, '_, '_>) -> bool {
    let v = view::<O>(c);
    let g = v.word(W_NGONE) as usize;
    next_pass(v.work, g, W_FWIDTH, W_FWHICH)
}

fn fsettle<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let g = v.word(W_NGONE) as usize;
    let at = if v.word(W_FWHICH) == 0 { v.w.f_a } else { v.w.f_b };
    let sorted: &[u32] = words(v.work, at, g.max(1));
    let mut out = LayerOut::new(v.layer);
    for k in t.items() {
        let r = k * CHUNK..((k + 1) * CHUNK).min(g);
        out.flips()[r.clone()].copy_from_slice(&sorted[r]); // copy: derived (the layer's flips, from the sort's work)
    }
}

// ---- indexes: each the out rows' positions in its key's order ------------------------------------------------------

fn nidx(shape: ShapeW) -> usize {
    shape.nidx as usize
}

/// Index `x`'s order of out rows `p` and `q`: by its key (an order); by the lead's tag, then the key (a hash order);
/// by the Bloom word the key falls in (a filter's work); position last.
fn icmp(out: &LayerOut<'_>, x: usize, p: u32, q: u32) -> Ordering {
    let shape = &out.head.shape;
    let key = shape.index_key(x);
    let kind = shape.index_kind(x);
    let by_key = || key.iter().map(|&c| out.get(p as usize, c as usize).cmp(&out.get(q as usize, c as usize))).find(|o| o.is_ne());
    let first = if kind & IX_BLOOM != 0 {
        bloom_word(out, x, p as usize).cmp(&bloom_word(out, x, q as usize)).then_with(|| by_key().unwrap_or(Ordering::Equal))
    } else if kind & IX_HASH != 0 {
        let lead = |r: u32| out.get(r as usize, key[0] as usize);
        tag(lead(p)).cmp(&tag(lead(q))).then_with(|| by_key().unwrap_or(Ordering::Equal))
    } else {
        by_key().unwrap_or(Ordering::Equal)
    };
    first.then(p.cmp(&q))
}

/// The Bloom word index `x`'s key of out row `o` falls in, and that word's bits for it.
fn bloom_place(out: &LayerOut<'_>, x: usize, o: usize) -> (usize, u64) {
    let key = out.head.shape.index_key(x);
    let h = bloom::hash(bloom_key(key.iter().map(|&c| out.get(o, c as usize))));
    bloom::place(h, bloom_words(out.head.n as usize))
}

fn bloom_word(out: &LayerOut<'_>, x: usize, o: usize) -> usize {
    bloom_place(out, x, o).0
}

fn isort_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |s, wb, _| nidx(s.shape()) * chunks(word::<u64>(wb, W_NOUT) as usize) as usize)
}

fn isort<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let ch = chunks(n_out) as usize;
    set_word::<u64>(v.work, W_IWIDTH, CHUNK as u64);
    set_word::<u64>(v.work, W_IWHICH, 0);
    let out = LayerOut::new(v.layer);
    for item in t.items() {
        let (x, k) = (item / ch, item % ch);
        let a = words_mut::<u32>(v.work, v.w.ix_a + 4 * v.w.cap * x, n_out);
        let r = k * CHUNK..((k + 1) * CHUNK).min(n_out);
        for i in r.clone() {
            a[i] = i as u32;
        }
        a[r].sort_by(|&p, &q| icmp(&out, x, p, q));
    }
}

fn ipass_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |s, wb, _| {
        let (n, width) = (word::<u64>(wb, W_NOUT) as usize, word::<u64>(wb, W_IWIDTH) as usize);
        if more(n, width) { nidx(s.shape()) * pairs(n, width) as usize } else { 0 }
    })
}

fn ipass<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let (n_out, width, which) = (v.n_out(), v.word(W_IWIDTH) as usize, v.word(W_IWHICH));
    let pp = pairs(n_out, width) as usize;
    let out = LayerOut::new(v.layer);
    let cap = v.w.cap;
    let (a_all, b_all) = v.work[v.w.ix_a..].split_at_mut(v.w.ix_b - v.w.ix_a);
    for item in t.items() {
        let (x, p) = (item / pp, item % pp);
        let a = words_mut::<u32>(a_all, 4 * cap * x, n_out);
        let b = words_mut::<u32>(b_all, 4 * cap * x, n_out);
        let one = Tile { lo: p as u32, hi: p as u32 + 1 };
        let cmp = |p: u32, q: u32| icmp(&out, x, p, q);
        if which == 0 { merge_pairs(a, b, one, width, cmp) } else { merge_pairs(b, a, one, width, cmp) }
    }
}

fn ipass_next<O: Op>(c: &mut Ctx<'_, '_, '_>) -> bool {
    let v = view::<O>(c);
    let n = v.n_out();
    next_pass(v.work, n, W_IWIDTH, W_IWHICH)
}

fn isettle<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let ch = chunks(n_out) as usize;
    let base = if v.word(W_IWHICH) == 0 { v.w.ix_a } else { v.w.ix_b };
    let cap = v.w.cap;
    let mut out = LayerOut::new(v.layer);
    for item in t.items() {
        let (x, k) = (item / ch, item % ch);
        let sorted: &[u32] = words(v.work, base + 4 * cap * x, n_out);
        if out.head.shape.index_kind(x) & IX_BLOOM != 0 {
            continue;
        }
        let r = k * CHUNK..((k + 1) * CHUNK).min(n_out);
        out.perm(x)[r.clone()].copy_from_slice(&sorted[r]); // copy: derived (the layer's index, from the sort's work)
    }
}

/// Each order's fences (a `FENCE`th position's lead), a hash order's tags, and the prints (a position's whole key):
/// a chunk of positions of an index an item, read from the order just settled.
fn iaux<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let ch = chunks(n_out) as usize;
    let mut out = LayerOut::new(v.layer);
    let shape = out.head.shape;
    for item in t.items() {
        let (x, k) = (item / ch, item % ch);
        let (kind, key) = (shape.index_kind(x), shape.index_key(x));
        if kind & IX_BLOOM != 0 {
            continue;
        }
        for p in k * CHUNK..((k + 1) * CHUNK).min(n_out) {
            let row = out.perm(x)[p] as usize;
            let lead = out.get(row, key[0] as usize);
            if kind & IX_FENCES != 0 && p % FENCE == 0 {
                out.fences(x)[p / FENCE] = lead;
            }
            if kind & IX_HASH != 0 {
                out.tags(x)[p] = tag(lead);
            }
            if kind & IX_PRINTS != 0 {
                let print = print_vals(key.iter().map(|&c| out.get(row, c as usize)));
                out.prints(x)[p] = print;
            }
        }
    }
}

fn bloom_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |s, wb, _| nidx(s.shape()) * bloom_words(word::<u64>(wb, W_NOUT) as usize))
}

/// A Bloom filter's words, a word an item: the keys that fall in word `w` lie together in the filter's sorted work
/// (sorted by word), found by bisection, and OR-ed. No two items write one word.
fn bloom_fill<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let nw = bloom_words(n_out);
    let base = if v.word(W_IWHICH) == 0 { v.w.ix_a } else { v.w.ix_b };
    let cap = v.w.cap;
    let mut out = LayerOut::new(v.layer);
    for item in t.items() {
        let (x, w) = (item / nw, item % nw);
        if out.head.shape.index_kind(x) & IX_BLOOM == 0 {
            continue;
        }
        let sorted: &[u32] = words(v.work, base + 4 * cap * x, n_out);
        let lo = sorted.partition_point(|&o| bloom_word(&out, x, o as usize) < w);
        let hi = sorted.partition_point(|&o| bloom_word(&out, x, o as usize) <= w);
        let bits = sorted[lo..hi].iter().fold(0u64, |m, &o| m | bloom_place(&out, x, o as usize).1);
        out.bloom(x)[w] = bits;
    }
}

// ---- sketch: partials a chunk, then combined a column ---------------------------------------------------------------

fn sketch<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let out = LayerOut::new(v.layer);
    let a = out.head.shape.arity();
    let ch = chunks(v.w.cap).max(1) as usize;
    let sk = words_mut::<SketchW>(v.work, v.w.sk, a * ch);
    for k in t.items() {
        for col in 0..a {
            let mut s = SketchW::EMPTY;
            for o in k * CHUNK..((k + 1) * CHUNK).min(n_out) {
                let plus = out.is_plus(o);
                s.add(out.get(o, col), if plus { 1 } else { -1 });
            }
            sk[k * a + col] = s;
        }
    }
}

fn sketches<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n_out = v.n_out();
    let mut out = LayerOut::new(v.layer);
    let a = out.head.shape.arity();
    let ch = chunks(v.w.cap).max(1) as usize;
    let sk: &[SketchW] = words(v.work, v.w.sk, a * ch);
    for col in t.items() {
        out.sketches()[col] = (0..chunks(n_out) as usize).fold(SketchW::EMPTY, |s, k| s.combine(&sk[k * a + col]));
    }
}

// ---- seal: the header, last ----------------------------------------------------------------------------------------

fn seal<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let refused;
    let (n_out, added, gone);
    {
        let v = view::<O>(c);
        refused = v.word(W_REFUSED);
        let np = v.word(W_NPARTS) as usize;
        let mut out = LayerOut::new(v.layer);
        n_out = out.head.n;
        added = out.parts()[..np].iter().map(|p| p.added).sum::<u32>();
        gone = out.parts()[..np].iter().map(|p| p.gone).sum::<u32>();
        let digest = out.parts()[..np].iter().fold(0u64, |d, p| d.wrapping_add(p.digest));
        out.commit(added, gone, np as u32, digest);
    }
    if O::CONTROL {
        let ([], [ctl]) = c.views([], [CTL]);
        set_word::<u64>(ctl, BODY_AT, CONTROL_BYTES as u64);
        set_word::<u64>(ctl, BODY_AT + 8, refused);
        set_word::<u64>(ctl, BODY_AT + 16, u64::from(n_out));
        set_word::<u64>(ctl, BODY_AT + 24, u64::from(added));
        set_word::<u64>(ctl, BODY_AT + 32, u64::from(gone));
    } else {
        assert_eq!(refused, 0, "a merge of committed layers refuses nothing");
    }
}

/// Control's refusal, after `consolidate`.
pub fn refused(control: &[u8]) -> u64 {
    word::<u64>(control, BODY_AT + 8)
}

// ---- the twins -----------------------------------------------------------------------------------------------------

fn src_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).n() as u32
}

fn src_chunks<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    chunks(src_items::<O>(c) as usize)
}

fn out_chunks<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |_, wb, _| chunks(word::<u64>(wb, W_NOUT) as usize) as usize)
}

fn out_words<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |_, wb, _| (word::<u64>(wb, W_NOUT) as usize).div_ceil(64))
}

fn out_rows<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    items_of::<O>(c, |_, wb, _| word::<u64>(wb, W_NOUT) as usize)
}

fn arity<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).shape().arity() as u32
}

fn gone_chunks<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    chunks(ngone::<O>(c))
}

fn isettle_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    isort_items::<O>(c)
}

macro_rules! pipeline {
    ($name:ident, $o:ty) => {
        pub const $name: &[TwinPhase] = &[
            TwinPhase { name: "sort", items: src_chunks::<$o>, run: sort_first::<$o>, again: None },
            TwinPhase { name: "passes", items: pass_items::<$o>, run: pass::<$o>, again: Some(pass_next::<$o>) },
            TwinPhase { name: "segments", items: src_items::<$o>, run: segments::<$o>, again: None },
            TwinPhase { name: "count", items: src_chunks::<$o>, run: count::<$o>, again: None },
            TwinPhase { name: "scan", items: |_| 1, run: scan::<$o>, again: None },
            TwinPhase { name: "place", items: src_chunks::<$o>, run: place::<$o>, again: None },
            TwinPhase { name: "signs", items: out_words::<$o>, run: signs::<$o>, again: None },
            TwinPhase { name: "kills", items: out_rows::<$o>, run: kills::<$o>, again: None },
            TwinPhase { name: "pcount", items: out_chunks::<$o>, run: pcount::<$o>, again: None },
            TwinPhase { name: "pscan", items: |_| 1, run: pscan::<$o>, again: None },
            TwinPhase { name: "pindex", items: out_chunks::<$o>, run: pindex::<$o>, again: None },
            TwinPhase { name: "pfill", items: nparts::<$o>, run: pfill::<$o>, again: None },
            TwinPhase { name: "gscan", items: |_| 1, run: gscan::<$o>, again: None },
            TwinPhase { name: "pflips", items: nparts::<$o>, run: pflips::<$o>, again: None },
            TwinPhase { name: "fsort", items: gone_chunks::<$o>, run: fsort::<$o>, again: None },
            TwinPhase { name: "fpasses", items: fpass_items::<$o>, run: fpass::<$o>, again: Some(fpass_next::<$o>) },
            TwinPhase { name: "fsettle", items: gone_chunks::<$o>, run: fsettle::<$o>, again: None },
            TwinPhase { name: "isort", items: isort_items::<$o>, run: isort::<$o>, again: None },
            TwinPhase { name: "ipasses", items: ipass_items::<$o>, run: ipass::<$o>, again: Some(ipass_next::<$o>) },
            TwinPhase { name: "isettle", items: isettle_items::<$o>, run: isettle::<$o>, again: None },
            TwinPhase { name: "iaux", items: isettle_items::<$o>, run: iaux::<$o>, again: None },
            TwinPhase { name: "bloom", items: bloom_items::<$o>, run: bloom_fill::<$o>, again: None },
            TwinPhase { name: "sketch", items: out_chunks::<$o>, run: sketch::<$o>, again: None },
            TwinPhase { name: "sketches", items: arity::<$o>, run: sketches::<$o>, again: None },
            TwinPhase { name: "seal", items: |_| 1, run: seal::<$o>, again: None },
        ];
    };
}

pipeline!(CONSOLIDATE, Cons);
pipeline!(MERGE, Merge);
