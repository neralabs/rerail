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
    IX_BLOOM, IX_FENCES, IX_ORDER, IX_PRINTS, LAYER_BODY, Layer, LayerHead, MAX_IX, NONE, Runs, ShapeW, bloom_key, bloom_words, gone_spec,
    parts_spec, rows_spec, sketch_room,
};
use rerail_blocks::order::{FENCE, print_vals};
use rerail_blocks::relation::{self, Relation, layer_write, row_digest};
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
/// A recursive relation's key live at two levels after the change (0364: a rewrite kills the old level's row).
pub const REFUSED_LEVELS: u64 = 4;

/// Control's words: the refusal, the new layer's rows, its rows added and gone, and the newest layers due to merge.
pub const CONTROL_BYTES: usize = 48;

// ---- the source ---------------------------------------------------------------------------------------------------

/// The rows a layer is made from: a run's δ runs, or the layers merged (newest first: each one's own rows, then its
/// kills). One a phase's call, on its stack.
#[allow(clippy::large_enum_variant)]
enum Src<'a> {
    Runs(Runs<'a>),
    // copy: not block data (a handle and a first item a merged layer)
    Layers { chain: relation::View<'a>, layers: Vec<Layer<'a>>, off: Vec<usize> },
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
                (l, Err(k)) => chain.get(l.kills()[k], c),
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
        let chain = relation::View::chain(table, Some(p.head), arity).expect("the merged layers and those below, reached");
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
    ks2: usize,
    abefore: usize,
    st: usize,
    pstart: usize,
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
/// The kills' sort's width and array; the scan's carry (rows out, rows added); the partitions numbered so far.
const W_KWIDTH: usize = W_BLOOM + 8 * MAX_IX;
const W_KWHICH: usize = W_KWIDTH + 8;
const W_SCAN: usize = W_KWHICH + 8;
const W_NPART: usize = W_SCAN + 16;
const W_END: usize = W_NPART + 8;

fn round64(n: usize) -> usize {
    n.div_ceil(64) * 64
}

fn work(cap: usize) -> Work {
    let base = round64(BODY_AT + LAYER_BODY);
    let mut at = round64(W_END);
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
        ks2: next(4 * cap),
        abefore: next(4 * (cap + 1)),
        st: next(4 * cap),
        pstart: next(4 * (cap + 1)),
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
fn below_chain<'t>(table: &'t Table<'t>, below: u32, arity: usize) -> relation::View<'t> {
    relation::View::chain(table, (below != NONE).then_some(below), arity).expect("the layers below, reached by reference")
}

/// A call's own scratch for an index of `n` rows: a bitmap of a key's range (2^24 bits at most) and a word a row.
fn scratch_for(n: usize) -> Scratch {
    Scratch::anon(((4 << 20) + n * 64).next_multiple_of(rerail_blocks::block::PAGE))
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

// ---- the rows out: count, scan, the layer opened, place ------------------------------------------------------------

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

/// The counts' exclusive prefix sums, a chunk an item, in order (each item adds its chunk's counts to the carry the
/// item before it left); the last item, past the last chunk, writes the totals: rows out, added and gone.
fn scan<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let ch = chunks(v.src.n()) as usize;
    for k in t.items() {
        let carry = [v.word(W_SCAN), v.word(W_SCAN + 8)];
        if k == ch {
            words_mut::<u64>(v.work, v.w.ccount, ch + 1)[ch] = carry[0];
            words_mut::<u64>(v.work, v.w.acount, ch + 1)[ch] = carry[1];
            set_word::<u64>(v.work, W_NOUT, carry[0]);
            set_word::<u64>(v.work, W_NADD, carry[1]);
            set_word::<u64>(v.work, W_NGONE, carry[0] - carry[1]);
            words_mut::<u32>(v.work, v.w.abefore, carry[0] as usize + 1)[carry[0] as usize] = carry[1] as u32;
            continue;
        }
        let mut next = carry;
        for (j, at) in [v.w.ccount, v.w.acount].into_iter().enumerate() {
            let x = &mut words_mut::<u64>(v.work, at, ch + 1)[k];
            next[j] += *x;
            *x = carry[j];
        }
        set_word::<u64>(v.work, W_SCAN, next[0]);
        set_word::<u64>(v.work, W_SCAN + 8, next[1]);
    }
}

fn scan_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    src_chunks::<O>(c) + 1
}

/// The layer opened in its block (one item): its three relations made (the layer continuing the one below), the
/// freeze's directories laid out (`layer_write::open`), its added rows and its kills by partition claimed, its header
/// laid out, open.
fn open<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let table = c.table;
    let (n_out, n_add, shape, below) = {
        let v = view::<O>(c);
        (v.n_out(), v.word(W_NADD) as usize, v.src.shape(), v.below)
    };
    let n_gone = n_out - n_add;
    let below_layer = (below != NONE).then(|| (below, Layer::new(table.block(below).expect("the layer below, reached by reference")).expect("committed")));
    let pin = c.pin_mut(OUT);
    let block = Arc::clone(pin.block());
    let rs = rows_spec(&shape, n_add);
    let rows = Relation::try_continue_far(&block, &rs, below_layer.map(|(id, l)| (id, l.head.rows as usize, l.rows))).expect("room for the layer");
    let parts = Relation::try_create(&block, &parts_spec(n_out), None).expect("room for the layer's partitions");
    let gone = Relation::try_create(&block, &gone_spec(n_gone), None).expect("room for the layer's kills by partition");
    assert!(layer_write::open(pin, rows, &rs, n_gone, true), "room for the layer's freeze");
    assert!(layer_write::open(pin, parts, &parts_spec(n_out), 0, false), "room for the partitions' freeze");
    assert!(layer_write::open(pin, gone, &gone_spec(n_gone), 0, false), "room for the kills' freeze");
    rows.claim_own(pin, n_add as u32).expect("room for the layer's rows");
    gone.claim_own(pin, n_gone as u32).expect("room for the layer's kills by partition");
    let at = |r: &Relation| block.offset_of(std::ptr::from_ref(r));
    let head = LayerHead::open(shape, at(rows), at(parts), at(gone));
    set_word(pin.bytes_mut().expect("a write pin"), BODY_AT, head);
}

/// Each row out's source item and sign, and how many rows added come before it; its added rows written: each one's
/// place is the count of rows added before it.
fn place<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    // copy: not block data (this tile's placements: row out, its source item, the rows added before it, and whether it
    // is added)
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
                    placed.push((o, perm[i], a, seg[i] > 0));
                    o += 1;
                    a += usize::from(seg[i] > 0);
                }
            }
        }
        let n_out = v.n_out();
        let (net_at, sgn_at, ab_at) = (v.w.net, v.w.sgn, v.w.abefore);
        let net = words_mut::<u32>(v.work, net_at, n_out.max(1));
        for &(o, src, _, _) in &placed {
            net[o] = src;
        }
        let sgn = words_mut::<i32>(v.work, sgn_at, n_out.max(1));
        for &(o, _, _, add) in &placed {
            sgn[o] = if add { 1 } else { -1 };
        }
        let ab = words_mut::<u32>(v.work, ab_at, n_out + 1);
        for &(o, _, a, _) in &placed {
            ab[o] = a as u32;
        }
    }
    let (block, head) = out_block(c);
    let rows = rel(&block, head.rows);
    let (table, params) = (c.table, c.params);
    let (sb, pin) = c.read_and_pin(SRC, OUT);
    let src = O::src(sb, table, params);
    let a = head.shape.arity();
    for &(_, i, at, add) in &placed {
        if add {
            for col in 0..a {
                rows.put_own(pin, at as u32, col, src.get(i as usize, col));
            }
        }
    }
}

// ---- kills: a batched search of the state below, then sorted ------------------------------------------------------

fn n_out<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    work_word::<O>(c, W_NOUT) as u32
}

fn n_gone<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    work_word::<O>(c, W_NGONE) as u32
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
        if sign > 0 && O::CONTROL && shape.level_col().is_some() && !one_level::<O>(&v, &chain, o) {
            refusal = REFUSED_LEVELS;
        }
    }
    let kid = v.w.kid;
    words_mut::<u32>(v.work, kid, v.n_out().max(1))[t.items()].copy_from_slice(&found);
    if refusal != 0 && v.word(W_REFUSED) == 0 {
        set_word::<u64>(v.work, W_REFUSED, refusal);
    }
}

/// Whether row out `o`, added, leaves its key live at one level (0364): no other row of the run adds the key, and
/// the key's live row below, if any, is one this run kills. The run's rows of a key lie together in the rows out (the
/// level is last in the canonical order), so they are `o`'s neighbours; the rows below, a probe of the key's prefix.
fn one_level<O: Op>(v: &View<'_>, chain: &relation::View<'_>, o: usize) -> bool {
    let shape = v.src.shape();
    let keys: Vec<ColumnIndex> = shape.key_order().iter().map(|&c| ColumnIndex::new(u16::from(c))).collect(); // copy: not block data (the key's columns)
    let level = shape.level_col().expect("levels");
    let net: &[u32] = words(v.work, v.w.net, v.n_out().max(1));
    let sgn: &[i32] = words(v.work, v.w.sgn, v.n_out().max(1));
    let key_of = |p: usize| -> Vec<i64> { keys.iter().map(|c| v.src.get(net[p] as usize, c.get())).collect() }; // copy: not block data (a key)
    let key = key_of(o);
    let mut same = Vec::new(); // copy: not block data (the run's rows of the key)
    let mut p = o;
    while p > 0 && key_of(p - 1) == key {
        p -= 1;
    }
    while p < v.n_out() && key_of(p) == key {
        same.push(p);
        p += 1;
    }
    if same.iter().any(|&q| q != o && sgn[q] > 0) {
        return false;
    }
    chain.matching(&keys, &key).all(|id| {
        let lv = chain.get(id.get(), level);
        same.iter().any(|&q| sgn[q] < 0 && v.src.get(net[q] as usize, level) == lv)
    })
}

/// The kills' sort, first phase: each chunk of the rows out's kill ids (none for a row added: `NONE`, sorted last)
/// into `ks`, sorted by itself.
fn ksort<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let v = view::<O>(c);
    let n = v.n_out();
    set_word::<u64>(v.work, W_KWIDTH, CHUNK as u64);
    set_word::<u64>(v.work, W_KWHICH, 0);
    let (kid, ks) = (v.w.kid, v.w.ks);
    let (lo, rest) = v.work.split_at_mut(ks);
    let src: &[u32] = words(lo, kid, n.max(1));
    let dst = words_mut::<u32>(rest, 0, n.max(1));
    for k in t.items() {
        let r = k * CHUNK..((k + 1) * CHUNK).min(n);
        dst[r.clone()].copy_from_slice(&src[r]);
    }
    sort_chunks(&mut dst[..n], t, |x, y| x.cmp(&y));
}

fn kpass_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    if refused_so_far::<O>(c) {
        return 0;
    }
    let n = n_out::<O>(c) as usize;
    let width = work_word::<O>(c, W_KWIDTH) as usize;
    if more(n, width) { pairs(n, width) } else { 0 }
}

/// A merge pass of the kills' sort: pairs of runs, between `ks` and `ks2`.
fn kpass<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let (n, width, which) = (v.n_out(), v.word(W_KWIDTH) as usize, v.word(W_KWHICH));
    let (a, b) = v.work[v.w.ks..].split_at_mut(v.w.ks2 - v.w.ks);
    let (a, b) = (words_mut::<u32>(a, 0, n), words_mut::<u32>(b, 0, n));
    let cmp = |x: u32, y: u32| x.cmp(&y);
    if which == 0 { merge_pairs(a, b, t, width, cmp) } else { merge_pairs(b, a, t, width, cmp) }
}

fn kpass_next<O: Op>(c: &mut Ctx<'_, '_, '_>) -> bool {
    let v = view::<O>(c);
    let n = v.n_out();
    let width = v.word(W_KWIDTH) as usize;
    if !more(n, width) {
        return false;
    }
    set_word::<u64>(v.work, W_KWIDTH, 2 * width as u64);
    set_word::<u64>(v.work, W_KWHICH, 1 - v.word(W_KWHICH));
    more(n, 2 * width)
}

/// The sorted kills into the layer's change, a chunk an item: its first `ngone` (the rest are `NONE`).
fn kplace<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (lo, hi) = (t.lo as usize * CHUNK, t.hi as usize * CHUNK);
    let tile: Vec<u32> = {
        let v = view::<O>(c);
        let ng = v.word(W_NGONE) as usize;
        let at = if v.word(W_KWHICH) == 0 { v.w.ks } else { v.w.ks2 };
        let sorted: &[u32] = words(v.work, at, ng.max(1));
        sorted[lo.min(ng)..hi.min(ng)].to_vec() // copy: not block data (this tile's kills, between two places in one block)
    };
    let (block, head) = out_block(c);
    let rows = rel(&block, head.rows);
    let ks = layer_write::kills_mut(c.pin_mut(OUT), rows);
    ks[lo..lo + tile.len()].copy_from_slice(&tile);
}

fn kplace_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    chunks(n_gone::<O>(c) as usize)
}

// ---- the partitions' change: a partition an item ------------------------------------------------------------------

/// Row out `o`'s partition: its source item's value in the partition column (0 without one).
fn part_of(src: &Src<'_>, net: &[u32], part: Option<usize>, o: usize) -> i64 {
    part.map_or(0, |p| src.get(net[o] as usize, p))
}

/// Whether each row out starts a partition (the rows out lie in the canonical order, the partition first).
fn pstarts<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let v = view::<O>(c);
    let n = v.n_out();
    let part = v.src.shape().partition();
    let net: &[u32] = words(v.work, v.w.net, n.max(1));
    // copy: not block data (this tile's flags)
    let flags: Vec<u32> = t.items().map(|o| u32::from(o == 0 || part_of(&v.src, net, part, o) != part_of(&v.src, net, part, o - 1))).collect();
    words_mut::<u32>(v.work, v.w.st, n.max(1))[t.items()].copy_from_slice(&flags);
}

/// The partitions' starts, a chunk of rows out an item, in order (each item numbers its chunk's starts after the carry
/// the item before it left); the last item writes the end and claims a row of `parts` a partition.
fn pscan<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let last = {
        let v = view::<O>(c);
        let n = v.n_out();
        let ch = chunks(n) as usize;
        let mut np = v.word(W_NPART) as usize;
        let st: Vec<u32> = words::<u32>(v.work, v.w.st, n.max(1)).to_vec(); // copy: not block data (the flags, read beside the starts)
        let ps = words_mut::<u32>(v.work, v.w.pstart, n + 1);
        for k in t.items() {
            for (o, &s) in st.iter().enumerate().take(((k + 1) * CHUNK).min(n)).skip(k * CHUNK) {
                if s != 0 {
                    ps[np] = o as u32;
                    np += 1;
                }
            }
        }
        if t.items().contains(&(ch - 1)) {
            ps[np] = n as u32;
        }
        set_word::<u64>(v.work, W_NPART, np as u64);
        t.items().contains(&(ch - 1)).then_some(np)
    };
    if let Some(np) = last {
        let (block, head) = out_block(c);
        rel(&block, head.parts).claim_own(c.pin_mut(OUT), np as u32).expect("room for the partitions");
    }
}

fn pscan_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    if refused_so_far::<O>(c) { 0 } else { chunks(n_out::<O>(c) as usize) }
}

fn parts_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    if refused_so_far::<O>(c) { 0 } else { work_word::<O>(c, W_NPART) as u32 }
}

/// Partition `p`'s change (P§11.2's partition index), an item each: `parts(part, digest, added, gone, first,
/// kills_at)`, and its kills, ascending, at `kills_at` of `gone(part, id)`. Its rows out are a range (`pscan`'s); its
/// own rows are `first .. first + added` of the layer's, its kills `kills_at .. kills_at + gone` of `gone`.
fn parts<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    // copy: not block data (this tile's partitions' entries and kills, written below)
    let mut entries: Vec<(u32, [i64; 6], Vec<u32>)> = Vec::new();
    {
        let v = view::<O>(c);
        let n = v.n_out();
        let shape = v.src.shape();
        let a = shape.arity();
        let (net, sgn, kid): (&[u32], &[i32], &[u32]) = (words(v.work, v.w.net, n.max(1)), words(v.work, v.w.sgn, n.max(1)), words(v.work, v.w.kid, n.max(1)));
        let ab: &[u32] = words(v.work, v.w.abefore, n + 1);
        let ps: &[u32] = words(v.work, v.w.pstart, n + 1);
        for p in t.items() {
            let (s, e) = (ps[p] as usize, ps[p + 1] as usize);
            let first = ab[s];
            let added = ab[e] - first;
            let gone = (e - s) as u32 - added;
            let mut digest = 0u64;
            let mut ids = Vec::with_capacity(gone as usize); // copy: derived (the partition's kills, written below)
            for o in s..e {
                let d = row_digest((0..a).map(|col| v.src.get(net[o] as usize, col)));
                if sgn[o] > 0 {
                    digest = digest.wrapping_add(d);
                } else {
                    digest = digest.wrapping_sub(d);
                    ids.push(kid[o]);
                }
            }
            ids.sort_unstable();
            let part = part_of(&v.src, net, shape.partition(), s);
            entries.push((p as u32, [part, digest as i64, i64::from(added), i64::from(gone), i64::from(first), (s as u32 - first).into()], ids));
        }
    }
    let (block, head) = out_block(c);
    let (pr, gr) = (rel(&block, head.parts), rel(&block, head.gone));
    let pin = c.pin_mut(OUT);
    for (p, e, ids) in entries {
        for (col, v) in e.into_iter().enumerate() {
            pr.put_own(pin, p, col, v);
        }
        for (j, id) in ids.into_iter().enumerate() {
            gr.put_own(pin, e[5] as u32 + j as u32, 0, e[0]);
            gr.put_own(pin, e[5] as u32 + j as u32, 1, i64::from(id));
        }
    }
}

// ---- the layer frozen where it lies: ranges, indexes, the freeze ---------------------------------------------------

/// The columns whose ranges `stats` takes: the layer's rows', then `parts`' six, then `gone`'s two.
const PARTS_COLS: usize = 6;
const GONE_COLS: usize = 2;

fn stats_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    (O::src(c.read(SRC), c.table, c.params).shape().arity() + PARTS_COLS + GONE_COLS) as u32
}

/// A column's range, an item each, over the rows, `parts` or `gone` (`layer_write::stat`).
fn stats<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let a = head.shape.arity();
    let pin = c.pin_mut(OUT);
    for x in t.items() {
        let (r, col) = if x < a {
            (rel(&block, head.rows), x)
        } else if x < a + PARTS_COLS {
            (rel(&block, head.parts), x - a)
        } else {
            (rel(&block, head.gone), x - a - PARTS_COLS)
        };
        layer_write::stat(pin, r, col);
    }
}

fn index_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    rows_spec(&O::src(c.read(SRC), c.table, c.params).shape(), 0).indexes.len() as u32
}

/// The layer's hashed indexes, an index an item (`layer_write::index`: its row index, and each `Hash` of its shape).
fn indexes<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let rows = rel(&block, head.rows);
    let spec = rows_spec(&head.shape, rows.rows.len() as usize);
    let s = scratch_for(rows.rows.len() as usize);
    let pin = c.pin_mut(OUT);
    for x in t.items() {
        layer_write::index(pin, rows, &spec, x, &s);
    }
}

/// The freeze (one item, every item of the phases before it done): the rows, `parts` and `gone` sealed
/// (`layer_write::seal`).
fn freeze<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let (rows, pr, gr) = (rel(&block, head.rows), rel(&block, head.parts), rel(&block, head.gone));
    let pin = c.pin_mut(OUT);
    layer_write::seal(pin, rows, &rows_spec(&head.shape, rows.rows.len() as usize));
    layer_write::seal(pin, pr, &parts_spec(pr.rows.len() as usize));
    layer_write::seal(pin, gr, &gone_spec(gr.rows.len() as usize));
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

/// The layer's statistics, a column an item: its sketch of the state the layer ends, the state below's with its rows
/// added and its kills taken (O§10: "a delta's sketch is added to and taken from"), in its kept form, into its entry
/// (`layer_write::sketch`).
fn sketch<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
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
    let pin = c.pin_mut(OUT);
    for col in t.items() {
        let mut s = below_layer.and_then(|l| l.sketch(col)).unwrap_or_else(|| Sketch::new_in(sketch_room()));
        for id in rows.ids() {
            s.add(rows.own(id, col));
        }
        for &id in rows.changed.as_slice() {
            s.remove(chain.get(id, col));
        }
        assert!(layer_write::sketch(pin, rows, col, &s.encode()), "room for the layer's statistics");
    }
}

fn arity_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).shape().arity() as u32
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
        let cache = O::src(c.read(SRC), c.table, c.params).shape().is_cache();
        let due = if refused == 0 { due_to_merge(c.table, O::below(c.params), added + gone, cache) } else { 0 };
        let ([], [ctl]) = c.views([], [CTL]);
        set_word::<u64>(ctl, BODY_AT + 40, due);
    } else {
        assert_eq!(refused, 0, "a merge of committed layers refuses nothing");
    }
}

/// The geometric roll-up (O5, the rebuild plan: "merge a layer into the one below once it reaches a share of it",
/// at every layer; the share a half): how many of the newest layers, the new one (`size` rows and kills) first, are
/// due to merge into one. From the newest down, a layer joins while what is merged so far is more than half its size;
/// so after the merge every layer is at most half the one below it, and a trace of `n` rows has O(log n) layers. 0 or
/// 1: none due. Read from the layers' headers, through the descriptor table.
///
/// A cache's trace (P§11.3, one state) folds every layer into one: all are due.
pub fn due_to_merge(table: &Table<'_>, below: u32, size: u64, cache: bool) -> u64 {
    let (mut acc, mut k, mut id) = (size, 1u64, below);
    while id != NONE {
        let l = Layer::new(table.block(id).expect("a layer below, reached by reference")).expect("committed");
        let n = u64::from(l.head.added) + u64::from(l.head.ngone);
        if !cache && 2 * acc <= n {
            break;
        }
        acc += n;
        k += 1;
        id = l.below().map_or(NONE, |(b, _)| b);
    }
    k
}

/// The newest layers due to merge, in control after `consolidate` (`due_to_merge`).
pub fn due(control: &[u8]) -> u64 {
    word::<u64>(control, BODY_AT + 40)
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
            TwinPhase { name: "scan", items: scan_items::<$o>, run: scan::<$o>, again: None },
            TwinPhase { name: "open", items: |_| 1, run: open::<$o>, again: None },
            TwinPhase { name: "place", items: src_chunks::<$o>, run: place::<$o>, again: None },
            TwinPhase { name: "kills", items: n_out::<$o>, run: kills::<$o>, again: None },
            TwinPhase { name: "ksort", items: |c| chunks(n_out::<$o>(c) as usize), run: ksort::<$o>, again: None },
            TwinPhase { name: "kpasses", items: kpass_items::<$o>, run: kpass::<$o>, again: Some(kpass_next::<$o>) },
            TwinPhase { name: "kplace", items: kplace_items::<$o>, run: kplace::<$o>, again: None },
            TwinPhase { name: "pstarts", items: n_out::<$o>, run: pstarts::<$o>, again: None },
            TwinPhase { name: "pscan", items: pscan_items::<$o>, run: pscan::<$o>, again: None },
            TwinPhase { name: "parts", items: parts_items::<$o>, run: parts::<$o>, again: None },
            TwinPhase { name: "stats", items: stats_items::<$o>, run: stats::<$o>, again: None },
            TwinPhase { name: "indexes", items: index_items::<$o>, run: indexes::<$o>, again: None },
            TwinPhase { name: "freeze", items: |_| 1, run: freeze::<$o>, again: None },
            TwinPhase { name: "orders", items: nidx::<$o>, run: orders::<$o>, again: None },
            TwinPhase { name: "sketch", items: arity_items::<$o>, run: sketch::<$o>, again: None },
            TwinPhase { name: "seal", items: |_| 1, run: seal::<$o>, again: None },
        ];
    };
}

pipeline!(CONSOLIDATE, Cons);
pipeline!(MERGE, Merge);
