//! The trace's operations (decision 0686; *0686: Operations* §10): `consolidate` and `merge`.
//!
//! **consolidate** (scratch δ runs · write facts · rw control): a run's `(row, diff)` pairs into a layer in its own new
//! block, over the trace's newest layer (`below`, reached through the descriptor table): the pairs sorted in the
//! trace's canonical order, each row's diffs summed, zeros dropped; each row gone given the id of the live row it kills
//! (a batched search of the state below, by its indexes); the layer written as P§12 has it, a
//! `blocks::relation::Relation` continuing the one below, written an item a call and frozen where it lies
//! (`relation::layer_write`: its ranges, its indexes, its seal); its partitions' change and its kills, kept once
//! (`parts`, `gone`); its orders and Bloom filters; its
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
use core::cmp::Ordering;
use rerail_blocks::bistate::ColumnIndex;
use rerail_blocks::block::Block;
use rerail_blocks::bloom::BloomW;
use rerail_blocks::bloom::{self, Bloom};
use rerail_blocks::gens::{BODY_AT, Id, Table, set_word, word, words, words_mut};
use rerail_blocks::layer::OwnRowsIn;
use rerail_blocks::layer::{
    ENC_OPEN, IX_BLOOM, IX_FENCES, IX_ORDER, IX_PERM, IX_PRINTS, LAYER_BODY, Layer, LayerHead, MAX_COLS, MAX_IX, MAX_LAYERS, NONE, OwnRows, Runs, ShapeW, State,
    bloom_key, bloom_words,
};
use rerail_blocks::order::{FENCE, print_vals};
use rerail_blocks::relation::RelationW;
use rerail_blocks::relation::layer_write::{self, Part};
use rerail_blocks::relation::{Relation, row_digest};
use rerail_blocks::scratch::Scratch;

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

/// Slots: the source (δ runs, or the newest layer merged), the new layer, and consolidate's control; each one's
/// scratch is its last (`Op::SCRATCH`).
const SRC: usize = 0;
const OUT: usize = 1;
const CTL: usize = 2;

/// The bytes of the scratch slot a launch over `cap` source items takes: a call's temporaries, one index's or one
/// order's at a time (a key range's bitmap, 2^24 bits at most, and a few words a row), or a working sketch a column
/// (`SKETCH_WORK` each, in a region of its own).
pub fn scratch_bytes(cap: usize) -> usize {
    ((4 << 20) + cap.max(1) * 64 + 64).next_multiple_of(rerail_blocks::block::PAGE)
}

/// A column's working sketch's bytes in the scratch slot (the floor's `SketchW`), at 64 bytes.
const SKETCH_WORK: usize = rerail_blocks::sketch::WORK_BYTES.next_multiple_of(64);
const _: () = assert!(MAX_COLS * SKETCH_WORK + 64 <= 4 << 20, "every column's working sketch in the scratch slot");

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
/// kills): the state they end, each merged layer's header read where it lies, and its first item (`off`, the items'
/// end past the last). One a phase's call, on its stack.
#[allow(clippy::large_enum_variant)]
enum Src<'a> {
    Runs(Runs<'a>),
    Layers { chain: State<'a>, layers: [Option<Layer<'a>>; MAX_LAYERS], off: [usize; MAX_LAYERS + 1], k: usize },
}

impl<'a> Src<'a> {
    fn n(&self) -> usize {
        match self {
            Src::Runs(r) => r.len(),
            Src::Layers { off, k, .. } => off[*k],
        }
    }

    fn shape(&self) -> ShapeW {
        match self {
            Src::Runs(r) => r.head.shape,
            Src::Layers { layers, .. } => layers[0].expect("a merged layer").head.shape,
        }
    }

    /// A merged item's layer, and its place in it: an own row's id, or a kill's place in its kills.
    fn at(&self, i: usize) -> (Layer<'a>, Result<u32, usize>) {
        let Src::Layers { layers, off, k, .. } = self else { unreachable!("a merge's source") };
        let j = off[..=*k].partition_point(|&o| o <= i) - 1;
        let (l, local) = (layers[j].expect("a merged layer"), i - off[j]);
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
    /// Its scratch slot.
    const SCRATCH: usize;
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
    const SCRATCH: usize = 3;
}

impl Op for Merge {
    fn src<'a>(src: &'a [u8], table: &'a Table<'a>, params: &[u8]) -> Src<'a> {
        let p = <MergeP as crate::Params>::read(params);
        assert!(p.count >= 1 && p.count as usize <= MAX_LAYERS, "a merge of one layer or more, and at most a state's MAX_LAYERS");
        let mut layers = [None; MAX_LAYERS];
        let mut off = [0usize; MAX_LAYERS + 1];
        let mut l = Layer::new(src).expect("the newest merged layer, committed");
        let arity = l.head.shape.arity();
        for k in 0..p.count as usize {
            layers[k] = Some(l);
            off[k + 1] = off[k] + l.n() + l.kills().len();
            if k + 1 < p.count as usize {
                let (below, _) = l.below().expect("as many layers as merged");
                l = Layer::new(table.block(below).expect("a merged layer reached by reference")).expect("committed");
            }
        }
        assert_eq!(l.below().map_or(NONE, |(b, _)| b), p.below, "the merged layers lie over the layer below them");
        let chain = State::chain(table, Some(p.head), arity).expect("the merged layers and those below, reached");
        Src::Layers { chain, layers, off, k: p.count as usize }
    }
    fn below(params: &[u8]) -> u32 {
        <MergeP as crate::Params>::read(params).below
    }
    const CONTROL: bool = false;
    const SCRATCH: usize = 2;
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
    abefore: usize,
    st: usize,
    pstart: usize,
    /// An encoded layer's added rows' source items, in the layer's order (`place`; read by `encode`).
    addsrc: usize,
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
/// The scan's carry (rows out, rows added); the partitions numbered so far.
const W_SCAN: usize = W_BLOOM + 8 * MAX_IX;
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
        abefore: next(4 * (cap + 1)),
        st: next(4 * cap),
        pstart: next(4 * (cap + 1)),
        addsrc: next(4 * cap),
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

/// The new layer's block, for the launch, and its header as laid out (`scan`).
fn out_block<'p>(c: &Ctx<'_, 'p, '_>) -> (&'p Block, LayerHead) {
    (c.block(OUT), LayerHead::of_open(c.read(OUT)))
}

fn rel(block: &Block, at: u64) -> &Relation {
    Relation::in_block(block, at as usize).expect("the new layer's relation")
}

/// The state below the new layer, read through the descriptor table.
fn below_chain<'t>(table: &'t Table<'t>, below: u32, arity: usize) -> State<'t> {
    State::chain(table, (below != NONE).then_some(below), arity).expect("the layers below, reached by reference")
}

/// Columns `cols` as a key's column indexes, on the stack: the array, of which the first `cols.len()` are the key's.
fn col_indexes(cols: &[u8]) -> [ColumnIndex; MAX_COLS] {
    assert!(cols.len() <= MAX_COLS, "a key of at most MAX_COLS columns");
    core::array::from_fn(|k| ColumnIndex::new(cols.get(k).map_or(0, |&c| u16::from(c))))
}

/// Item `i`'s values in columns `cols`, on the stack.
fn key_of(src: &Src<'_>, i: usize, cols: &[ColumnIndex]) -> [i64; MAX_COLS] {
    core::array::from_fn(|k| cols.get(k).map_or(0, |c| src.get(i, c.get())))
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
    let seg = words_mut::<i32>(rest, 0, n);
    let mut bad = false;
    for i in t.items() {
        if i > 0 && same(i, i - 1) {
            seg[i] = 0;
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
        seg[i] = sum.clamp(-1, 1);
    }
    if bad && word::<u64>(perms, W_REFUSED) == 0 {
        set_word::<u64>(perms, W_REFUSED, REFUSED_SUM);
    }
}

// ---- the rows out: count, scan, the layer opened, place ------------------------------------------------------------

fn count<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let v = view::<O>(c);
    let n = v.src.n();
    let ch = chunks(n) as usize;
    // The segments lie before the counts (`work`): read the one while the other is written.
    let (cc, ac) = (v.w.ccount, v.w.acount);
    let (head, rest) = v.work.split_at_mut(cc);
    let seg: &[i32] = words(head, v.w.seg, n);
    for k in t.items() {
        let s = &seg[k * CHUNK..((k + 1) * CHUNK).min(n)];
        words_mut::<u64>(rest, 0, ch + 1)[k] = s.iter().filter(|&&x| x != 0).count() as u64;
        words_mut::<u64>(rest, ac - cc, ch + 1)[k] = s.iter().filter(|&&x| x > 0).count() as u64;
    }
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
    let block = c.block(OUT);
    let pin = c.pin_mut(OUT);
    let rp = Part::Rows(shape);
    let rows = layer_write::create(block, rp, n_add, below_layer.map(|(id, l)| (id, l.head.rows as usize, l.rows, l.end()))).expect("room for the layer");
    let parts = layer_write::create(block, Part::Parts, n_out, None).expect("room for the layer's partitions");
    let gone = layer_write::create(block, Part::Gone, n_gone, None).expect("room for the layer's kills by partition");
    assert!(layer_write::open_part(pin, rows, rp, n_add, true), "room for the layer's freeze");
    assert!(layer_write::open_part(pin, parts, Part::Parts, n_out, false), "room for the partitions' freeze");
    assert!(layer_write::open_part(pin, gone, Part::Gone, n_gone, false), "room for the kills' freeze");
    // An Open layer's rows are its relation's; an encoded one's lie in its encoded form (`encode`), beside it.
    let enc = if shape.encoding == ENC_OPEN {
        rows.claim_own(pin, n_add as u32).expect("room for the layer's rows");
        0
    } else {
        let o = shape.order();
        let order: [u16; MAX_COLS] = core::array::from_fn(|k| o.get(k).map_or(0, |&c| u16::from(c)));
        let (perms, np) = perm_cols(&shape);
        layer_write::encode_open(pin, rows, shape.encoding, shape.arity(), n_add, &order[..o.len()], &perms[..np]).expect("room for the layer's encoded rows")
    };
    gone.claim_own(pin, n_gone as u32).expect("room for the layer's kills by partition");
    let at = |r: &Relation| block.offset_of(core::ptr::from_ref(r));
    let mut head = LayerHead::open(shape, at(rows), at(parts), at(gone));
    head.enc = enc as u64;
    set_word(pin.bytes_mut().expect("a write pin"), BODY_AT, head);
}

/// Each row out's source item and sign, and how many rows added come before it; its added rows written: each one's
/// place is the count of rows added before it.
fn place<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let open = head.shape.encoding == ENC_OPEN;
    let ch = {
        let v = view::<O>(c);
        let n = v.src.n();
        let ch = chunks(n) as usize;
        let perm_at = if v.word(W_WHICH) == 0 { v.w.perm_a } else { v.w.perm_b };
        for k in t.items() {
            let (mut o, mut a) = (word::<u64>(v.work, v.w.ccount + 8 * k) as usize, word::<u64>(v.work, v.w.acount + 8 * k) as usize);
            for i in k * CHUNK..((k + 1) * CHUNK).min(n) {
                let s = word::<i32>(v.work, v.w.seg + 4 * i);
                if s == 0 {
                    continue;
                }
                let item = word::<u32>(v.work, perm_at + 4 * i);
                set_word::<u32>(v.work, v.w.net + 4 * o, item);
                set_word::<i32>(v.work, v.w.sgn + 4 * o, if s > 0 { 1 } else { -1 });
                set_word::<u32>(v.work, v.w.abefore + 4 * o, a as u32);
                if s > 0 && !open {
                    // Encoded: each added row's source item, in the layer's order; its values are written by `encode`.
                    set_word::<u32>(v.work, v.w.addsrc + 4 * a, item);
                }
                o += 1;
                a += usize::from(s > 0);
            }
        }
        ch
    };
    if !open {
        return;
    }
    // Open: each added row's values, at its place, from the chunk's rows out just placed.
    let rows = rel(block, head.rows);
    let (table, params) = (c.table, c.params);
    let (sb, pin) = c.read_and_pin(SRC, OUT);
    let src = O::src(sb, table, params);
    let w = work(src.n());
    let a = head.shape.arity();
    for k in t.items() {
        let wk = &pin.bytes()[w.base..];
        let (lo, hi) = (word::<u64>(wk, w.ccount + 8 * k) as usize, word::<u64>(wk, w.ccount + 8 * (k + 1).min(ch)) as usize);
        for o in lo..hi {
            let wk = &pin.bytes()[w.base..];
            if word::<i32>(wk, w.sgn + 4 * o) < 0 {
                continue;
            }
            let (at, i) = (word::<u32>(wk, w.abefore + 4 * o), word::<u32>(wk, w.net + 4 * o) as usize);
            for col in 0..a {
                rows.put_own(pin, at, col, src.get(i, col));
            }
        }
    }
}

// ---- encode: an encoded layer's rows, a column an item ------------------------------------------------------------

/// The columns of the shape's permutation indexes (`Perm`), in its index order: the array, of which the first so many
/// are theirs.
fn perm_cols(shape: &ShapeW) -> ([u16; MAX_IX], usize) {
    let (mut cols, mut n) = ([0u16; MAX_IX], 0);
    for x in (0..shape.nidx as usize).filter(|&x| shape.index_kind(x) & IX_PERM != 0) {
        cols[n] = u16::from(shape.index_key(x)[0]);
        n += 1;
    }
    (cols, n)
}

/// An encoded layer's added rows, read where they lie: row `r` is source item `addsrc[r]` (`place`'s).
struct Added<'a> {
    src: &'a Src<'a>,
    addsrc: &'a [u32],
}

impl rerail_blocks::packed::RowsIn for Added<'_> {
    fn n(&self) -> usize {
        self.addsrc.len()
    }
    fn arity(&self) -> usize {
        self.src.shape().arity()
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        self.src.get(self.addsrc[r] as usize, c)
    }
}

fn encode_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    let shape = O::src(c.read(SRC), c.table, c.params).shape();
    if refused_so_far::<O>(c) || shape.encoding == ENC_OPEN { 0 } else { shape.arity() as u32 }
}

fn perm_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    let shape = O::src(c.read(SRC), c.table, c.params).shape();
    if refused_so_far::<O>(c) || shape.encoding == ENC_OPEN { 0 } else { perm_cols(&shape).1 as u32 }
}

/// An encoded layer's rows, a column an item (`layer_write::encode_col`), read from the source where they lie.
fn encode<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    encoded::<O>(c, t, false);
}

/// An encoded layer's permutation indexes, an index an item (`layer_write::encode_perm`).
fn perm<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    encoded::<O>(c, t, true);
}

fn encoded<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile, perms: bool) {
    let (block, head) = out_block(c);
    let rows = rel(block, head.rows);
    let (table, params) = (c.table, c.params);
    let (n_add, addsrc_at) = {
        let v = view::<O>(c);
        (v.word(W_NADD) as usize, v.w.base + v.w.addsrc)
    };
    // The added rows' source items, in the block's work (`place` wrote them; this phase only reads them).
    let addsrc: &[u32] = words(block.bytes(addsrc_at, addsrc_at + 4 * n_add.max(1)), 0, n_add.max(1));
    // The scratch slot's room is its capacity: a Dict column whose distinct values do not fit asks for more
    // (`Ctx::reach`), and the phase runs again over the grown slot, the columns written before left as they are.
    let room = c.capacity(O::SCRATCH);
    for x in t.items() {
        let (sb, pin, scratch) = c.split(SRC, OUT, O::SCRATCH);
        let src = O::src(sb, table, params);
        let added = Added { src: &src, addsrc: &addsrc[..n_add] };
        let lent = room.min(scratch.len());
        let scratch = &mut scratch[..lent];
        let wrote = Scratch::lend(scratch, |s| {
            if perms {
                if layer_write::encode_perm(pin, rows, head.enc as usize, x, &added, s) { Ok(()) } else { Err(None) }
            } else {
                layer_write::encode_col(pin, rows, head.shape.encoding, head.enc as usize, x, &added, s).map_err(|e| match e {
                    layer_write::Short::Block => None,
                    layer_write::Short::Work(f) => Some(s.slot_need(&f)),
                })
            }
        });
        match wrote {
            Ok(()) => {}
            Err(Some(need)) => {
                c.reach(O::SCRATCH, need);
            }
            Err(None) => panic!("room for the layer's encoded rows"),
        }
    }
}

// ---- kills: a batched search of the state below, then sorted ------------------------------------------------------

fn n_out<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    work_word::<O>(c, W_NOUT) as u32
}


fn kills<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    let table = c.table;
    // A row's search reads the state below a layer a step: its bound fails a trace whose roll-up let it grow deep.
    let depth = {
        let arity = O::src(c.read(SRC), table, c.params).shape().arity();
        below_chain(table, O::below(c.params), arity).layers().count() as u32
    };
    if !t.items().is_empty() && !c.steps(depth) {
        return;
    }
    let v = view::<O>(c);
    let shape = v.src.shape();
    let chain = below_chain(table, v.below, shape.arity());
    let order = col_indexes(shape.order());
    let cols = &order[..shape.order().len()];
    let mut refusal = 0;
    for o in t.items() {
        let i = word::<u32>(v.work, v.w.net + 4 * o) as usize;
        let sign = word::<i32>(v.work, v.w.sgn + 4 * o);
        let key = key_of(&v.src, i, cols);
        let live = chain.matching(cols, &key[..cols.len()]).next();
        let id = match (sign < 0, live) {
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
        if sign > 0 && O::CONTROL && shape.level_col().is_some() && !one_level::<O>(&v, &chain, o) {
            refusal = REFUSED_LEVELS;
        }
        set_word::<u32>(v.work, v.w.kid + 4 * o, id);
    }
    if refusal != 0 && v.word(W_REFUSED) == 0 {
        set_word::<u64>(v.work, W_REFUSED, refusal);
    }
}

/// Whether row out `o`, added, leaves its key live at one level (0364): no other row of the run adds the key, and
/// the key's live row below, if any, is one this run kills. The run's rows of a key lie together in the rows out (the
/// level is last in the canonical order), so they are `o`'s neighbours; the rows below, a probe of the key's prefix.
fn one_level<O: Op>(v: &View<'_>, chain: &State<'_>, o: usize) -> bool {
    let shape = v.src.shape();
    let ko = col_indexes(shape.key_order());
    let keys = &ko[..shape.key_order().len()];
    let level = shape.level_col().expect("levels");
    let net = |p: usize| word::<u32>(v.work, v.w.net + 4 * p) as usize;
    let sgn = |p: usize| word::<i32>(v.work, v.w.sgn + 4 * p);
    let key = key_of(&v.src, net(o), keys);
    // The run's rows of the key: the range of rows out around `o` whose key is its.
    let (mut lo, mut hi) = (o, o + 1);
    while lo > 0 && key_of(&v.src, net(lo - 1), keys) == key {
        lo -= 1;
    }
    while hi < v.n_out() && key_of(&v.src, net(hi), keys) == key {
        hi += 1;
    }
    if (lo..hi).any(|q| q != o && sgn(q) > 0) {
        return false;
    }
    chain.matching(keys, &key[..keys.len()]).all(|id| {
        let lv = chain.get(id.get(), level);
        (lo..hi).any(|q| sgn(q) < 0 && v.src.get(net(q), level) == lv)
    })
}

// ---- the partitions' change: a partition an item ------------------------------------------------------------------

/// Row out `o`'s partition: its source item's value in the partition column (0 without one), its item read from the
/// work (`net`, at `net_at` of `work`).
fn part_of(src: &Src<'_>, work: &[u8], net_at: usize, part: Option<usize>, o: usize) -> i64 {
    part.map_or(0, |p| src.get(word::<u32>(work, net_at + 4 * o) as usize, p))
}

/// Whether each row out starts a partition (the rows out lie in the canonical order, the partition first).
fn pstarts<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let v = view::<O>(c);
    let part = v.src.shape().partition();
    let net = v.w.net;
    for o in t.items() {
        let start = o == 0 || part_of(&v.src, v.work, net, part, o) != part_of(&v.src, v.work, net, part, o - 1);
        set_word::<u32>(v.work, v.w.st + 4 * o, u32::from(start));
    }
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
        // The flags lie before the starts (`work`): read the one while the other is written.
        let (flags, rest) = v.work.split_at_mut(v.w.pstart);
        let st: &[u32] = words(flags, v.w.st, n.max(1));
        let ps = words_mut::<u32>(rest, 0, n + 1);
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
        rel(block, head.parts).claim_own(c.pin_mut(OUT), np as u32).expect("room for the partitions");
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
    let (block, head) = out_block(c);
    let (pr, gr) = (rel(block, head.parts), rel(block, head.gone));
    let (table, params) = (c.table, c.params);
    let (sb, pin) = c.read_and_pin(SRC, OUT);
    let src = O::src(sb, table, params);
    let w = work(src.n());
    let shape = src.shape();
    let a = shape.arity();
    for p in t.items() {
        let entry = {
            let wk = &pin.bytes()[w.base..];
            let n = word::<u64>(wk, W_NOUT) as usize;
            let (net, sgn): (&[u32], &[i32]) = (words(wk, w.net, n.max(1)), words(wk, w.sgn, n.max(1)));
            let (ab, ps): (&[u32], &[u32]) = (words(wk, w.abefore, n + 1), words(wk, w.pstart, n + 1));
            let (s, e) = (ps[p] as usize, ps[p + 1] as usize);
            let first = ab[s];
            let added = ab[e] - first;
            let gone = (e - s) as u32 - added;
            let mut digest = 0u64;
            for o in s..e {
                let d = row_digest((0..a).map(|col| src.get(net[o] as usize, col)));
                digest = if sgn[o] > 0 { digest.wrapping_add(d) } else { digest.wrapping_sub(d) };
            }
            let part = part_of(&src, wk, w.net, shape.partition(), s);
            (s, e, [part, digest as i64, i64::from(added), i64::from(gone), i64::from(first), (s as u32 - first).into()])
        };
        let (s, e, row) = entry;
        // The partition's kills, ascending, where its rows out's kill ids lie (an added row's is `NONE`, sorted last):
        // `kid` is read by no later phase.
        let n = word::<u64>(&pin.bytes()[w.base..], W_NOUT) as usize;
        words_mut::<u32>(&mut pin.bytes_mut().expect("a write pin")[w.base..], w.kid, n.max(1))[s..e].sort_unstable();
        for (col, v) in row.into_iter().enumerate() {
            pr.put_own(pin, p as u32, col, v);
        }
        for j in 0..row[3] as u32 {
            let id = word::<u32>(&pin.bytes()[w.base..], w.kid + 4 * (s + j as usize));
            gr.put_own(pin, row[5] as u32 + j, 0, row[0]);
            gr.put_own(pin, row[5] as u32 + j, 1, i64::from(id));
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
            (rel(block, head.rows), x)
        } else if x < a + PARTS_COLS {
            (rel(block, head.parts), x - a)
        } else {
            (rel(block, head.gone), x - a - PARTS_COLS)
        };
        let own = if x < a { OwnRows::in_block(block, &head, r) } else { OwnRows::Open(r) };
        layer_write::stat(pin, r, col, &own);
    }
}

fn index_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    layer_write::index_count(&O::src(c.read(SRC), c.table, c.params).shape()) as u32
}

/// The layer's hashed indexes, an index an item (`layer_write::index`: its row index, and each `Hash` of its shape).
fn indexes<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let rows = rel(block, head.rows);
    let own = OwnRows::in_block(block, &head, rows);
    let n = own.n() as usize;
    for x in t.items() {
        let (_, pin, scratch) = c.split(SRC, OUT, O::SCRATCH);
        Scratch::lend(scratch, |s| layer_write::index_part(pin, rows, Part::Rows(head.shape), n, x, s, &own));
    }
}

/// The freeze (one item, every item of the phases before it done): the rows, `parts` and `gone` sealed
/// (`layer_write::seal`).
fn freeze<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let (rows, pr, gr) = (rel(block, head.rows), rel(block, head.parts), rel(block, head.gone));
    let pin = c.pin_mut(OUT);
    let n = OwnRows::in_block(block, &head, rows).n();
    layer_write::seal_part(pin, rows, Part::Rows(head.shape), n);
    layer_write::seal_part(pin, pr, Part::Parts, pr.rows.len());
    layer_write::seal_part(pin, gr, Part::Gone, gr.rows.len());
}

/// The shape's orders and Bloom filters, an index an item (a `Hash` index is the relation's, built at its freeze).
fn orders<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let rows = rel(block, head.rows);
    let own = OwnRows::in_block(block, &head, rows);
    let n = own.n() as usize;
    let shape = head.shape;
    for x in t.items() {
        let (kind, key) = (shape.index_kind(x), shape.index_key(x));
        let cs: [u16; MAX_COLS] = core::array::from_fn(|k| key.get(k).map_or(0, |&c| u16::from(c)));
        let cols = &cs[..key.len()];
        let vals = |id: u32| cols.iter().map(move |&c| own.get(id, c as usize));
        if kind & IX_BLOOM != 0 {
            let at = Bloom::try_create(block, bloom_words(n)).expect("room for a Bloom filter");
            let b = Bloom::in_block(block, at).expect("just made");
            for id in own.ids() {
                b.add_hashed(bloom::hash(bloom_key(vals(id))));
            }
            let v = view::<O>(c);
            set_word::<u64>(v.work, W_BLOOM + 8 * x, at as u64);
        } else if kind & IX_ORDER != 0 {
            // Its ids in the key's order, ties by id (a total order); the leading value at every `FENCE`th; each one's
            // print: in the scratch slot, until the order is added to the layer.
            let (_, pin, scratch) = c.split(SRC, OUT, O::SCRATCH);
            Scratch::lend(scratch, |s| {
                let ids: &mut [u32] = s.take(n);
                for (k, id) in own.ids().enumerate() {
                    ids[k] = id;
                }
                ids.sort_unstable_by(|&p, &q| vals(p).cmp(vals(q)).then(p.cmp(&q)));
                let fences: &mut [i64] = s.take(if kind & IX_FENCES != 0 { n.div_ceil(FENCE) } else { 0 });
                for (f, &id) in fences.iter_mut().zip(ids.iter().step_by(FENCE)) {
                    *f = own.get(id, cols[0] as usize);
                }
                let prints: &mut [u8] = s.take(if kind & IX_PRINTS != 0 { n } else { 0 });
                for (p, &id) in prints.iter_mut().zip(ids.iter()) {
                    *p = print_vals(vals(id));
                }
                rows.add_order(pin, cols, ids, fences, prints).expect("room for an order");
            });
        }
    }
}

fn nidx<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).shape().nidx
}

/// The layer's statistics, a column an item: its sketch of the state the layer ends, the state below's with its rows
/// added and its kills taken (O§10: "a delta's sketch is added to and taken from"), in its kept form, into its entry
/// (`layer_write::sketch_change`).
fn sketch<O: Op>(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    if refused_so_far::<O>(c) {
        return;
    }
    let (block, head) = out_block(c);
    let rows = rel(block, head.rows);
    let own = OwnRows::in_block(block, &head, rows);
    let a = head.shape.arity();
    let below = O::below(c.params);
    let table = c.table;
    let chain = below_chain(table, below, a);
    let below_layer = (below != NONE).then(|| Layer::new(table.block(below).expect("reached")).expect("committed"));
    let gone = gone_ids(block, &head);
    for col in t.items() {
        let (mut added, mut removed) = (own.ids().map(|id| own.get(id, col)), gone.iter().map(|&id| chain.get(id, col)));
        // Each column's working sketch in a region of the scratch slot of its own (items independent of each other).
        let (_, pin, scratch) = c.split(SRC, OUT, O::SCRATCH);
        let at = scratch.as_ptr().align_offset(8) + col * SKETCH_WORK;
        let work = &mut scratch[at..at + SKETCH_WORK];
        assert!(layer_write::sketch_change(pin, rows, col, below_layer.as_ref(), &mut added, &mut removed, work), "room for the layer's statistics");
    }
}

/// The layer's kills, kept once: its `gone` relation's id column (by partition, ascending within each).
fn gone_ids<'b>(block: &'b Block, head: &LayerHead) -> &'b [u32] {
    let g = rel(block, head.gone);
    if g.rows.is_empty() { &[] } else { g.rows.col_chunks_u32(1).next().expect("a layer's kills, in one chunk") }
}

fn arity_items<O: Op>(c: &Ctx<'_, '_, '_>) -> u32 {
    O::src(c.read(SRC), c.table, c.params).shape().arity() as u32
}

// ---- seal: the header, last ----------------------------------------------------------------------------------------

fn seal<O: Op>(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let (refused, n_out, blooms) = {
        let v = view::<O>(c);
        let blooms: [u64; MAX_IX] = core::array::from_fn(|x| v.word(W_BLOOM + 8 * x));
        (v.word(W_REFUSED), v.n_out() as u64, blooms)
    };
    let (mut added, mut gone) = (0u64, 0u64);
    if refused == 0 {
        let (block, mut head) = out_block(c);
        let (rows, pr) = (rel(block, head.rows), rel(block, head.parts));
        head.added = OwnRows::in_block(block, &head, rows).n();
        head.ngone = rel(block, head.gone).rows.len();
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
        let due = if refused == 0 { due_to_merge(c.table, O::below(c.params), added + gone) } else { 0 };
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
pub fn due_to_merge(table: &Table<'_>, below: u32, size: u64) -> u64 {
    let (mut acc, mut k, mut id) = (size, 1u64, below);
    while id != NONE {
        let l = Layer::new(table.block(id).expect("a layer below, reached by reference")).expect("committed");
        let n = u64::from(l.head.added) + u64::from(l.head.ngone);
        if 2 * acc <= n {
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
            TwinPhase { name: "encode", items: encode_items::<$o>, run: encode::<$o>, again: None },
            TwinPhase { name: "perm", items: perm_items::<$o>, run: perm::<$o>, again: None },
            TwinPhase { name: "kills", items: n_out::<$o>, run: kills::<$o>, again: None },
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
