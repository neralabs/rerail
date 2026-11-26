//! The trace's operations (decision 0686; *0686: Operations* §10): `consolidate` and `merge`.
//!
//! **consolidate** (scratch δ runs · write facts · rw control): a run's `(row, diff)` pairs into a layer in its own new
//! block, over the trace's newest layer (`below`, reached through the descriptor table): the pairs sorted in the
//! trace's canonical order, each row's diffs summed, zeros dropped; each row gone given the id of the live row it kills
//! (a batched search of the state below, by its indexes); the layer written as P§12 has it, a
//! `blocks::relation::Relation` continuing the one below, and frozen where it lies (`relation::freeze_layer`: its
//! order, its indexes, its change); its partitions' change (`parts`, `gone`); its orders and Bloom filters; its
//! statistics, each column's `blocks::sketch` of the state it ends (the one below's, its rows added and its kills
//! taken); the header last. A change that is not a set change is refused into control: a row's diffs summing past ±1,
//! a row gone that is not live below, a row added that already is.
//!
//! **merge** (read facts, older generations by reference · write facts): the newest `count` layers of a trace into
//! one new layer over the layer below them: the same phases, its source the merged layers' own rows (+1) and kills
//! (−1, the killed row's values), so a row added and killed among them cancels, and a row killed below and added again
//! stays the row below. A roll-up: the merged generations are let go by the host after it.
//!
//! Each phase's items are independent; what one phase leaves for the next lies in the new layer's block: the work
//! arrays in its body past the header (the freeze trims them where it can), the relations in its chunks.

use crate::sort::{CHUNK, chunks, merge_pairs, more, pairs, sort_chunks};
use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::block::Block;
use rerail_blocks::bloom::{self, Bloom};
use rerail_blocks::gens::{BODY_AT, Id, Table, set_word, word, words, words_mut};
use rerail_blocks::layer::{
    Chain, IX_BLOOM, IX_FENCES, IX_ORDER, IX_PRINTS, LAYER_BODY, Layer, LayerHead, MAX_IX, NONE, Runs, ShapeW, bloom_key, bloom_words, gone_spec,
    parts_spec, rows_spec, sketch_room,
};
use rerail_blocks::order::{FENCE, print_vals};
use rerail_blocks::relation::{Relation, freeze_layer, layer_sketches, row_digest};
use rerail_blocks::scratch::Scratch;
use rerail_blocks::sketch::Sketch;
use std::cmp::Ordering;
use std::sync::Arc;

crate::params! {
    /// `consolidate`'s params: the trace's newest layer, which the new one goes over (`NONE` for the first).
    pub struct ConsolidateP {
        pub below: u32,
        pub pad: u32,
    }
}

crate::params! {
    /// `merge`'s params: the newest layer (the read slot's block id), how many layers from it down are merged, and the
    /// layer below them (`NONE`).
    pub struct MergeP {
        pub head: u32,
        pub count: u32,
        pub below: u32,
        pub pad: u32,
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

/// The rows a layer is made from: a run's δ runs, or the layers merged (newest first: each one's own rows, then its
/// kills). One a phase's call, on its stack.
#[allow(clippy::large_enum_variant)]
enum Src<'a> {
    Runs(Runs<'a>),
    // copy: not block data (a handle and a first item a merged layer)
    Layers { chain: Chain<'a>, layers: Vec<Layer<'a>>, off: Vec<usize> },
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

    /// A merged item's layer, and its place in it: an own row's id, or a kill's place in its kills.
    fn at(&self, i: usize) -> (Layer<'a>, Result<u32, usize>) {
        let Src::Layers { layers, off, .. } = self else { unreachable!("a merge's source") };
        let j = off.partition_point(|&o| o <= i) - 1;
        let (l, local) = (layers[j], i - off[j]);
        (l, if local < l.n() { Ok(l.start() + local as u32) } else { Err(local - l.n()) })
    }

    fn get(&self, i: usize, c: usize) -> i64 {
        match self {
            Src::Runs(r) => r.get(i, c),
            Src::Layers { chain, .. } => match self.at(i) {
                (l, Ok(id)) => l.get(id, c),
                (l, Err(k)) => chain.get(rerail_blocks::bistate::RowId::new(l.kills()[k]), ColumnIndex::new(c as u16)),
            },
        }
    }

    fn diff(&self, i: usize) -> i32 {
        match self {
            Src::Runs(r) => r.diff(i),
            Src::Layers { .. } => {
                if self.at(i).1.is_ok() {
                    1
                } else {
                    -1
                }
            }
        }
    }

    /// Items `i` and `j` in the canonical order.
    fn cmp(&self, i: u32, j: u32) -> Ordering {
        let s = self.shape();
        s.order().iter().map(|&c| self.get(i as usize, c as usize).cmp(&self.get(j as usize, c as usize))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
    }
}

/// Which operation a pipeline runs as: where its source and its params' layer below come from.
trait Op {
    fn src<'a>(src: &'a [u8], table: &'a Table<'a>, params: &[u8]) -> Src<'a>;
    fn below(params: &[u8]) -> u32;
    const CONTROL: bool;
}

/// `consolidate`.
pub struct Cons;
/// `merge`.
pub struct Merge;

impl Op for Cons {
    fn src<'a>(src: &'a [u8], _: &'a Table<'a>, _: &[u8]) -> Src<'a> {
        Src::Runs(Runs::new(src))
    }
    fn below(params: &[u8]) -> u32 {
        <ConsolidateP as crate::Params>::read(params).below
    }
    const CONTROL: bool = true;
}

impl Op for Merge {
    fn src<'a>(src: &'a [u8], table: &'a Table<'a>, params: &[u8]) -> Src<'a> {
        let p = <MergeP as crate::Params>::read(params);
        assert!(p.count >= 1, "a merge of one layer or more");
        let mut layers = Vec::with_capacity(p.count as usize);
        let mut off = vec![0usize];
        let mut l = Layer::new(src).expect("the newest merged layer, committed");
        let arity = l.head.shape.arity();
        for k in 0..p.count as usize {
            layers.push(l);
            off.push(off[k] + l.n() + l.kills().len());
            if k + 1 < p.count as usize {
                let (below, _) = l.below().expect("as many layers as merged");
                l = Layer::new(table.block(below).expect("a merged layer reached by reference")).expect("committed");
            }
        }
        assert_eq!(l.below().map_or(NONE, |(b, _)| b), p.below, "the merged layers lie over the layer below them");
        let chain = Chain::new(table, Some(p.head), arity).expect("the merged layers and those below, reached");
        Src::Layers { chain, layers, off }
    }
    fn below(params: &[u8]) -> u32 {
        <MergeP as crate::Params>::read(params).below
    }
    const CONTROL: bool = false;
}

// ---- the work, past the header in the new block's body --------------------------------------------------------------

/// Where each work array lies, relative to the work's start (`base`, an absolute offset in the block).
#[derive(Clone, Copy)]
struct Work {
    base: usize,
    perm_a: usize,
    perm_b: usize,
    seg: usize,
    ccount: usize,
    acount: usize,
    net: usize,
    sgn: usize,
    kid: usize,
    ks: usize,
    len: usize,
}

/// Its header's words, at `base`: the sort's width and array, the rows out (net), added and gone, the refusal, and each
/// index's Bloom filter's offset in the block.
const W_WIDTH: usize = 0;
const W_WHICH: usize = 8;
const W_NOUT: usize = 16;
const W_NADD: usize = 24;
const W_NGONE: usize = 32;
const W_REFUSED: usize = 40;
const W_BLOOM: usize = 48;

fn round64(n: usize) -> usize {
    n.div_ceil(64) * 64
}

fn work(cap: usize) -> Work {
    let base = round64(BODY_AT + LAYER_BODY);
    let mut at = round64(W_BLOOM + 8 * MAX_IX);
    let mut next = |bytes: usize| {
        let a = at;
        at = round64(at + bytes);
        a
    };
    let ch = chunks(cap) as usize;
    let w = Work {
        base,
        perm_a: next(4 * cap),
        perm_b: next(4 * cap),
        seg: next(4 * cap),
        ccount: next(8 * (ch + 1)),
        acount: next(8 * (ch + 1)),
        net: next(4 * cap),
        sgn: next(4 * cap),
        kid: next(4 * cap),
        ks: next(4 * cap),
        len: 0,
    };
    Work { len: at, ..w }
}

/// The body bytes a new layer's block takes, for a source of `cap` items: its header and the work.
pub fn out_bytes(cap: usize) -> usize {
    let w = work(cap);
    w.base - BODY_AT + w.len
}

/// The phase's view: the source, the work region of the new block, its layout, and the layer below's block id.
struct View<'a> {
    src: Src<'a>,
    work: &'a mut [u8],
    w: Work,
    below: u32,
}

fn view<'a, O: Op>(c: &'a mut Ctx<'_, '_, '_>) -> View<'a> {
    let (table, params) = (c.table, c.params);
    let ([s], [out]) = c.views([SRC], [OUT]);
    let src = O::src(s, table, params);
    let w = work(src.n());
    View { src, work: &mut out[w.base..], w, below: O::below(params) }
}

impl View<'_> {
    fn word(&self, at: usize) -> u64 {
        word::<u64>(self.work, at)
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

/// The items of a phase, from the work's words: `f` of the source's size and the work.
fn items_of<O: Op>(c: &Ctx<'_, '_, '_>, f: impl Fn(usize, &[u8]) -> usize) -> u32 {
    let src = O::src(c.read(SRC), c.table, c.params);
    let w = work(src.n());
    f(src.n(), &c.read(OUT)[w.base..]) as u32
}

/// The work's word at `at`, read beside the source.
fn work_word<O: Op>(c: &Ctx<'_, '_, '_>, at: usize) -> u64 {
    items_of::<O>(c, |_, wb| word::<u64>(wb, at) as usize) as u64
}

fn refused_so_far<O: Op>(c: &Ctx<'_, '_, '_>) -> bool {
    work_word::<O>(c, W_REFUSED) != 0
}

/// The new layer's block, and its header as laid out (`scan`).
fn out_block(c: &Ctx<'_, '_, '_>) -> (Arc<Block>, LayerHead) {
    let pin = c.slots[OUT].pin();
    (Arc::clone(pin.block()), LayerHead::of_open(pin.bytes()))
}

fn rel(block: &Block, at: u64) -> &Relation {
    Relation::in_block(block, at as usize).expect("the new layer's relation")
}

/// The state below the new layer, read through the descriptor table.
fn below_chain<'t>(table: &'t Table<'t>, below: u32, arity: usize) -> Chain<'t> {
    Chain::new(table, (below != NONE).then_some(below), arity).expect("the layers below, reached by reference")
}

fn scratch_for(n: usize, a: usize) -> Scratch {
    Scratch::anon(((64 << 20) + n * (a + 8) * 64).next_multiple_of(rerail_blocks::block::PAGE))
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
    items_of::<O>(c, |n, wb| {
        let width = word::<u64>(wb, W_WIDTH) as usize;
        if more(n, width) { pairs(n, width) as usize } else { 0 }
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

fn pass_next<O: Op>(c: &mut Ctx<'_, '_, '_>) -> bool {
    let v = view::<O>(c);
    let n = v.src.n();
    let width = v.word(W_WIDTH) as usize;
    if !more(n, width) {
        return false;
    }
    set_word::<u64>(v.work, W_WIDTH, 2 * width as u64);
    set_word::<u64>(v.work, W_WHICH, 1 - v.word(W_WHICH));
    more(n, 2 * width)
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

// ---- the rows out: count, scan (the layer opened), place -------------------------------------------------------------

fn count<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n = v.src.n();
    let ch = chunks(n) as usize;
    // copy: not block data (this tile's counts: rows out, rows added)
    let counts: Vec<(u64, u64)> = t
        .items()
        .map(|k| {
            let s = &v.seg()[k * CHUNK..((k + 1) * CHUNK).min(n)];
            (s.iter().filter(|&&x| x != 0).count() as u64, s.iter().filter(|&&x| x > 0).count() as u64)
        })
        .collect();
    let (cc, ac) = (v.w.ccount, v.w.acount);
    words_mut::<u64>(v.work, cc, ch + 1)[t.items()].copy_from_slice(&counts.iter().map(|x| x.0).collect::<Vec<_>>());
    words_mut::<u64>(v.work, ac, ch + 1)[t.items()].copy_from_slice(&counts.iter().map(|x| x.1).collect::<Vec<_>>());
}

/// The counts' prefix sums, and the layer opened in its block: its three relations made (the layer continuing the one
/// below), its added rows claimed, its header laid out, open.
fn scan<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let (n_out, n_add, shape, below) = {
        let v = view::<O>(c);
        let ch = chunks(v.src.n()) as usize;
        let mut sums = [0u64; 2];
        for (k, at) in [v.w.ccount, v.w.acount].into_iter().enumerate() {
            for x in words_mut::<u64>(v.work, at, ch + 1).iter_mut() {
                let y = *x;
                *x = sums[k];
                sums[k] += y;
            }
        }
        set_word::<u64>(v.work, W_NOUT, sums[0]);
        set_word::<u64>(v.work, W_NADD, sums[1]);
        set_word::<u64>(v.work, W_NGONE, sums[0] - sums[1]);
        (sums[0] as usize, sums[1] as usize, v.src.shape(), v.below)
    };
    let below_layer = (below != NONE).then(|| (below, Layer::new(table.block(below).expect("the layer below, reached by reference")).expect("committed")));
    let pin = c.pin_mut(OUT);
    let block = Arc::clone(pin.block());
    let rows = Relation::try_continue_far(&block, &rows_spec(&shape, n_add), below_layer.map(|(id, l)| (id, l.head.rows as usize, l.rows))).expect("room for the layer");
    let parts = Relation::try_create(&block, &parts_spec(n_out), None).expect("room for the layer's partitions");
    let gone = Relation::try_create(&block, &gone_spec(n_out - n_add), None).expect("room for the layer's kills by partition");
    rows.claim_own(pin, n_add as u32).expect("room for the layer's rows");
    let at = |r: &Relation| block.offset_of(std::ptr::from_ref(r));
    let head = LayerHead::open(shape, at(rows), at(parts), at(gone));
    set_word(pin.bytes_mut().expect("a write pin"), BODY_AT, head);
}

/// Each row out's source item, and its added rows written: each one's place is the count of rows added before it.
fn place<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    // copy: not block data (this tile's placements: row out, its source item, and its place among the rows added)
    let mut placed = Vec::new();
    {
        let v = view::<O>(c);
        let n = v.src.n();
        let ch = chunks(n) as usize;
        let (perm, seg) = (v.perm(), v.seg());
        let (oc, ac): (&[u64], &[u64]) = (words(v.work, v.w.ccount, ch + 1), words(v.work, v.w.acount, ch + 1));
        for k in t.items() {
            let (mut o, mut a) = (oc[k] as usize, ac[k] as usize);
            for i in k * CHUNK..((k + 1) * CHUNK).min(n) {
                if seg[i] != 0 {
                    placed.push((o, perm[i], (seg[i] > 0).then_some(a)));
                    o += 1;
                    a += usize::from(seg[i] > 0);
                }
            }
        }
        let n_out = v.n_out();
        let (net_at, sgn_at) = (v.w.net, v.w.sgn);
        let net = words_mut::<u32>(v.work, net_at, n_out.max(1));
        for &(o, src, _) in &placed {
            net[o] = src;
        }
        let sgn = words_mut::<i32>(v.work, sgn_at, n_out.max(1));
        for &(o, _, add) in &placed {
            sgn[o] = if add.is_some() { 1 } else { -1 };
        }
    }
    let (block, head) = out_block(c);
    let rows = rel(&block, head.rows);
    let (table, params) = (c.table, c.params);
    let (sb, pin) = c.read_and_pin(SRC, OUT);
    let src = O::src(sb, table, params);
    let a = head.shape.arity();
    for &(_, i, at) in &placed {
        if let Some(at) = at {
            for col in 0..a {
                rows.put_own(pin, at as u32, col, src.get(i as usize, col));
            }
        }
    }
}

// ---- kills: a batched search of the state below ---------------------------------------------------------------------

fn n_out<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    work_word::<O>(c, W_NOUT) as u32
}

fn kills<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let table = c.table;
    let v = view::<O>(c);
    let shape = v.src.shape();
    let chain = below_chain(table, v.below, shape.arity());
    let cols: Vec<ColumnIndex> = shape.order().iter().map(|&c| ColumnIndex::new(u16::from(c))).collect(); // copy: not block data (the probe's columns)
    let net: &[u32] = words(v.work, v.w.net, v.n_out().max(1));
    let sgn: &[i32] = words(v.work, v.w.sgn, v.n_out().max(1));
    let mut found = Vec::with_capacity(t.items().len()); // copy: not block data (this tile's kill ids)
    let mut refusal = 0;
    for o in t.items() {
        let i = net[o] as usize;
        let sign = sgn[o];
        let key: Vec<i64> = cols.iter().map(|c| v.src.get(i, c.get())).collect(); // copy: not block data (a probe's key)
        let live = chain.matching(&cols, &key).next();
        found.push(match (sign < 0, live) {
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
        });
    }
    let kid = v.w.kid;
    words_mut::<u32>(v.work, kid, v.n_out().max(1))[t.items()].copy_from_slice(&found);
    if refusal != 0 && v.word(W_REFUSED) == 0 {
        set_word::<u64>(v.work, W_REFUSED, refusal);
    }
}

/// The kills, ascending: the layer's change.
fn ksort<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let v = view::<O>(c);
    if v.word(W_REFUSED) != 0 {
        return;
    }
    let n_out = v.n_out();
    let kid: &[u32] = words(v.work, v.w.kid, n_out.max(1));
    let mut ks: Vec<u32> = kid[..n_out].iter().copied().filter(|&k| k != NONE).collect(); // copy: not block data (the kills, sorted here)
    ks.sort_unstable();
    let at = v.w.ks;
    words_mut::<u32>(v.work, at, ks.len().max(1))[..ks.len()].copy_from_slice(&ks);
}

// ---- the layer frozen where it lies -----------------------------------------------------------------------------------

/// The layer's freeze (`relation::freeze_layer`): its order, its indexes, its change.
fn freeze<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let v = view::<O>(c);
    let ng = v.word(W_NGONE) as usize;
    let ks: &[u32] = words(v.work, v.w.ks, ng.max(1));
    let rows = rel(&block, head.rows);
    let n = rows.rows.len() as usize;
    let s = scratch_for(n, head.shape.arity());
    freeze_layer(&block, rows, &rows_spec(&head.shape, n), &ks[..ng], &s);
}

/// The layer's partitions' change (P§11.2's partition index): `parts(part, digest, added, gone)` and
/// `gone(part, id)`, each sorted on the partition, written and frozen.
fn parts<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let (rows, pr, gr) = (rel(&block, head.rows), rel(&block, head.parts), rel(&block, head.gone));
    let a = head.shape.arity();
    let part_col = head.shape.partition();
    let table = c.table;
    let below = O::below(c.params);
    let chain = below_chain(table, below, a);
    let part_of_own = |id: u32| part_col.map_or(0, |p| rows.own(id, p));
    let part_of_below = |id: u32| part_col.map_or(0, |p| chain.get(rerail_blocks::bistate::RowId::new(id), ColumnIndex::new(p as u16)));
    let digest_own = |id: u32| row_digest((0..a).map(|col| rows.own(id, col)));
    let digest_below = |id: u32| row_digest((0..a).map(|col| chain.get(rerail_blocks::bistate::RowId::new(id), ColumnIndex::new(col as u16))));
    // By partition: the rows lie sorted on it; the kills are gathered by it, ascending within each.
    let mut entries: std::collections::BTreeMap<i64, (u64, u32, u32)> = std::collections::BTreeMap::new(); // copy: derived (the partitions' change, written below)
    let mut gone: Vec<(i64, u32)> = Vec::new(); // copy: derived (the kills by partition, written below)
    for id in rows.ids() {
        let e = entries.entry(part_of_own(id)).or_default();
        e.0 = e.0.wrapping_add(digest_own(id));
        e.1 += 1;
    }
    for &id in rows.changed.as_slice() {
        let p = part_of_below(id);
        let e = entries.entry(p).or_default();
        e.0 = e.0.wrapping_sub(digest_below(id));
        e.2 += 1;
        gone.push((p, id));
    }
    gone.sort_unstable();
    let pin = c.pin_mut(OUT);
    let at = pr.claim_own(pin, entries.len() as u32).expect("room for the partitions");
    for (k, (&p, &(d, added, g))) in entries.iter().enumerate() {
        for (col, v) in [p, d as i64, i64::from(added), i64::from(g)].into_iter().enumerate() {
            pr.put_own(pin, at + k as u32, col, v);
        }
    }
    let at = gr.claim_own(pin, gone.len() as u32).expect("room for the kills by partition");
    for (k, &(p, id)) in gone.iter().enumerate() {
        gr.put_own(pin, at + k as u32, 0, p);
        gr.put_own(pin, at + k as u32, 1, i64::from(id));
    }
    let s = scratch_for(entries.len() + gone.len(), 4);
    freeze_layer(&block, pr, &parts_spec(entries.len()), &[], &s);
    freeze_layer(&block, gr, &gone_spec(gone.len()), &[], &s);
}

/// The shape's orders and Bloom filters, an index an item (a `Hash` index is the relation's, built at its freeze).
fn orders<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let rows = rel(&block, head.rows);
    let shape = head.shape;
    let mut blooms = Vec::new(); // copy: not block data (this tile's filters' offsets)
    for x in t.items() {
        let (kind, key) = (shape.index_kind(x), shape.index_key(x));
        let cols: Vec<u16> = key.iter().map(|&c| u16::from(c)).collect(); // copy: not block data (the key's columns)
        let vals = |id: u32| cols.iter().map(move |&c| rows.own(id, c as usize));
        if kind & IX_BLOOM != 0 {
            let at = Bloom::try_create(&block, bloom_words(rows.rows.len() as usize)).expect("room for a Bloom filter");
            let b = Bloom::in_block(&block, at).expect("just made");
            for id in rows.ids() {
                b.add_hashed(bloom::hash(bloom_key(vals(id))));
            }
            blooms.push((x, at));
        } else if kind & IX_ORDER != 0 {
            // Its ids in the key's order, ties by id; the leading value at every `FENCE`th; each one's print.
            let mut ids: Vec<u32> = rows.ids().collect(); // copy: derived (the order, written into the layer below)
            ids.sort_by(|&p, &q| vals(p).cmp(vals(q)).then(p.cmp(&q)));
            let fences: Vec<i64> = if kind & IX_FENCES != 0 { ids.iter().step_by(FENCE).map(|&id| rows.own(id, cols[0] as usize)).collect() } else { Vec::new() }; // copy: derived (its fences)
            let prints: Vec<u8> = if kind & IX_PRINTS != 0 { ids.iter().map(|&id| print_vals(vals(id))).collect() } else { Vec::new() }; // copy: derived (its prints)
            rows.add_order(c.pin_mut(OUT), &cols, &ids, &fences, &prints).expect("room for an order");
        }
    }
    let v = view::<O>(c);
    for (x, at) in blooms {
        set_word::<u64>(v.work, W_BLOOM + 8 * x, at as u64);
    }
}

fn nidx<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).shape().nidx
}

/// The layer's statistics: each column's sketch of the state it ends, the state below's with its rows added and its
/// kills taken (O§10: "a delta's sketch is added to and taken from"), in its kept form.
fn sketch<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let rows = rel(&block, head.rows);
    let a = head.shape.arity();
    let below = O::below(c.params);
    let table = c.table;
    let chain = below_chain(table, below, a);
    let below_layer = (below != NONE).then(|| Layer::new(table.block(below).expect("reached")).expect("committed"));
    let mut kept = Vec::with_capacity(a); // copy: derived (each column's sketch, in its kept form)
    for col in 0..a {
        let mut s = below_layer.and_then(|l| l.sketch(col)).unwrap_or_else(|| Sketch::new_in(sketch_room()));
        for id in rows.ids() {
            s.add(rows.own(id, col));
        }
        for &id in rows.changed.as_slice() {
            s.remove(chain.get(rerail_blocks::bistate::RowId::new(id), ColumnIndex::new(col as u16)));
        }
        kept.push(s.encode());
    }
    let all = layer_sketches(kept.iter().map(Vec::as_slice));
    assert!(rows.set_sketches(c.pin_mut(OUT), &all), "room for the layer's statistics");
}

// ---- seal: the header, last ----------------------------------------------------------------------------------------

fn seal<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let (refused, n_out, blooms) = {
        let v = view::<O>(c);
        let blooms: [u64; MAX_IX] = std::array::from_fn(|x| v.word(W_BLOOM + 8 * x));
        (v.word(W_REFUSED), v.n_out() as u64, blooms)
    };
    let (mut added, mut gone) = (0u64, 0u64);
    if refused == 0 {
        let (block, mut head) = out_block(c);
        let (rows, pr) = (rel(&block, head.rows), rel(&block, head.parts));
        head.added = rows.rows.len();
        head.ngone = rows.changed.len() as u32;
        head.nparts = pr.rows.len();
        head.digest = pr.ids().fold(0u64, |d, id| d.wrapping_add(pr.own(id, 1) as u64));
        head.blooms = blooms;
        (added, gone) = (u64::from(head.added), u64::from(head.ngone));
        head.commit(c.pin_mut(OUT).bytes_mut().expect("a write pin"));
    }
    if O::CONTROL {
        let ([], [ctl]) = c.views([], [CTL]);
        set_word::<u64>(ctl, BODY_AT, CONTROL_BYTES as u64);
        set_word::<u64>(ctl, BODY_AT + 8, refused);
        set_word::<u64>(ctl, BODY_AT + 16, n_out);
        set_word::<u64>(ctl, BODY_AT + 24, added);
        set_word::<u64>(ctl, BODY_AT + 32, gone);
    } else {
        assert_eq!(refused, 0, "a merge of committed layers refuses nothing");
    }
}

/// Control's refusal, after `consolidate`.
pub fn refused(control: &[u8]) -> u64 {
    word::<u64>(control, BODY_AT + 8)
}

/// The new layer's block id, as its params name the one below (for the host's reference, `Directory::refer`).
pub fn below_of(params: &[u8], merge: bool) -> Option<Id> {
    let b = if merge { <MergeP as crate::Params>::read(params).below } else { <ConsolidateP as crate::Params>::read(params).below };
    (b != NONE).then_some(b)
}

// ---- the twins -----------------------------------------------------------------------------------------------------

fn src_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).n() as u32
}

fn src_chunks<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    chunks(src_items::<O>(c) as usize)
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
            TwinPhase { name: "kills", items: n_out::<$o>, run: kills::<$o>, again: None },
            TwinPhase { name: "ksort", items: |_| 1, run: ksort::<$o>, again: None },
            TwinPhase { name: "freeze", items: |_| 1, run: freeze::<$o>, again: None },
            TwinPhase { name: "parts", items: |_| 1, run: parts::<$o>, again: None },
            TwinPhase { name: "orders", items: nidx::<$o>, run: orders::<$o>, again: None },
            TwinPhase { name: "sketch", items: |_| 1, run: sketch::<$o>, again: None },
            TwinPhase { name: "seal", items: |_| 1, run: seal::<$o>, again: None },
        ];
    };
}

pipeline!(CONSOLIDATE, Cons);
pipeline!(MERGE, Merge);
