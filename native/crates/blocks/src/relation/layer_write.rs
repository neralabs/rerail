//! A trace's layer written and frozen in place, an item at a time (decision 0686; *0686: Operations* §4, §10): what the
//! phases of `consolidate` and `merge` (`rerail_delta_twins::trace`) call to lay a layer's `Relation` out in its block,
//! through the write pin of its slot. Each function is one item of a phase, independent of the phase's other items:
//!
//! - `open`: the freeze's directories, laid out once (the phase that opens the layer): its columns' statistics, its
//!   index directory and its sketches' directory (its change stays empty: its kills are its `gone` relation's);
//! - `stat`: one column's range;
//! - `index`: one of its spec's hashed indexes (`build_indexes`' builder, one index a call), after its columns' ranges;
//! - `encode_open`, `encode_col`, `encode_perm`: its own rows in its shape's encoding (P§12: Packed, Dict, Ranged),
//!   laid out once, then a column a call and a permutation index a call, read where the rows lie;
//! - `seal`: the relation frozen (its sort column, its directory's count, its live rows), its rows' every other item
//!   done;
//! - `sketch`: one column's statistics, in its kept form, into its entry of the directory.
//!
//! A layer's rows are written in the trace's canonical order, each once (its writer made them a set change): so a
//! freeze here sorts and deduplicates nothing.
//!
//! **Its claims are exact** (*Flat Views and Lifecycles* L4): an operation's out block keeps no free list and hands
//! out nothing zeroed (`Block::try_claim`): each structure here writes its own state over its exact room (a
//! directory its empty entries, a column's words their clear bits), and a final form settled from a working one is
//! claimed at its need (`Block::try_settle`: a sketch's kept bytes, from its `SketchW`).
//!
//! **The twins' rules hold here** (the twins' crate doc): nothing outlives a call or comes from outside it: no
//! `static`, thread-local, `Once`/`Lazy` cell or lock, no thread's scratch (a call is given its own `Scratch`), no
//! file, environment, process or time. `check_delta_exec_shape.py` checks this module with the twins, and its clippy
//! lints are denied here (blocks' `clippy.toml` lists the twins' paths). Its `unsafe` is blocks' own layout writing:
//! a chunk allocated in the block the pin holds to write, and spans set to it.

#![deny(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]

use super::{ADDED_INDEXES, ColStats, FROZEN, Index, Relation, build_one_index};
use crate::layer::{ENC_DICT, ENC_RANGED, LayerSpec};
use crate::packed::RowsIn;
pub use crate::packed::writer::Short;
use crate::refs::Span;
use crate::region::BlockPin;
use crate::scratch::Scratch;
use std::sync::atomic::Ordering::{Relaxed, Release};

fn check_open(pin: &BlockPin, rel: &Relation) {
    assert!(pin.block().holds(std::ptr::from_ref(rel)) && pin.writes() && !rel.is_frozen(), "an open layer of a block pinned to write");
}

/// One of a layer's three relations, as an operation names it (its spec is blocks', `LayerSpec`, fixed: nothing a
/// phase calls here takes the heap): its rows, of the trace's shape; its partitions' change; its kills by partition.
#[derive(Clone, Copy, Debug)]
pub enum Part {
    Rows(crate::layer::ShapeW),
    Parts,
    Gone,
}

impl Part {
    /// Its spec, for `n` rows.
    fn spec(self, n: usize) -> LayerSpec {
        match self {
            Part::Rows(shape) => LayerSpec::rows(&shape, n),
            Part::Parts => LayerSpec::parts(n),
            Part::Gone => LayerSpec::gone(n),
        }
    }
}

/// The hashed indexes a layer of `shape` has (`index`'s items): each `Hash` index of the shape, and its row index.
pub fn index_count(shape: &crate::layer::ShapeW) -> usize {
    LayerSpec::rows(shape, 0).nkeys()
}

/// Layer part `part` made in `block`, for `n` rows: its rows continuing the layer below (`below`, as
/// `Relation::try_continue_far` takes it), the others a relation of their own. `None` when the block has no room.
pub fn create<'b>(block: &'b crate::block::Block, part: Part, n: usize, below: Option<(u32, usize, &Relation, u32)>) -> Option<&'b Relation> {
    let spec = part.spec(n);
    match part {
        Part::Rows(_) => super::try_continue_far_cols(block, spec.cols(), below),
        Part::Parts | Part::Gone => super::try_create_cols(block, spec.cols(), None),
    }
}

/// `open` of layer part `part` of `n` rows.
pub fn open_part(pin: &mut BlockPin, rel: &Relation, part: Part, n: usize, sketched: bool) -> bool {
    open(pin, rel, &part.spec(n), sketched)
}

/// `index` `x` of layer part `part` of `n` rows.
pub fn index_part(pin: &mut BlockPin, rel: &Relation, part: Part, n: usize, x: usize, scratch: &Scratch, rows: &dyn RowsIn) {
    index(pin, rel, &part.spec(n), x, scratch, rows);
}

/// `seal` of layer part `part`, its `n` own rows live.
pub fn seal_part(pin: &mut BlockPin, rel: &Relation, part: Part, n: u32) {
    seal(pin, rel, &part.spec(n as usize), n);
}

/// Column `c`'s statistics of the state a layer ends, into its entry (`sketch`): the sketch of the layer below's
/// state (`below`'s kept one; none for a trace's first layer, or one that is no sketch of this format), each value of
/// `added` added and of `removed` taken, worked in `work` (`sketch::WORK_BYTES` of the operation's scratch, the floor's
/// `SketchW`: no heap) and encoded into its chunk of the block. False when the block has no room.
pub fn sketch_change(pin: &mut BlockPin, rel: &Relation, c: usize, below: Option<&crate::layer::Layer<'_>>, added: &mut dyn Iterator<Item = i64>, removed: &mut dyn Iterator<Item = i64>, work: &mut [u8]) -> bool {
    use rerail_delta_core::sketch::SketchW;
    let mut s = SketchW::over(work).expect("a sketch's working bytes, aligned");
    if let Some(kept) = below.and_then(|l| layer_sketch(l.rows, c)) {
        s.decode(kept);
    }
    for v in added {
        s.add(v);
    }
    for v in removed {
        s.remove(v);
    }
    settle_sketch(pin, rel, c, &s)
}

/// Lays out the freeze's directories of an open layer `rel` of `spec`: a range a column (`stat`), the index directory
/// (its hashed indexes, and room for the orders added after it is frozen), and a sketch a column when `sketched`
/// (`sketch`). False when the block has no room. A layer records no change of its own: its kills are its `gone` relation's.
pub fn open(pin: &mut BlockPin, rel: &Relation, spec: &LayerSpec, sketched: bool) -> bool {
    check_open(pin, rel);
    let block = pin.block();
    let a = rel.arity as usize;
    let room = ADDED_INDEXES.max(spec.nkeys());
    // Each directory's entries start empty, written so here (an index's entry of no columns, a sketch's empty span);
    // a column's range is written below.
    let (Some(sp), Some(ixp)) = (block.try_claim_t::<ColStats>(a.max(1)), block.try_claim_clear_t::<Index>(room)) else { return false };
    let sk = if sketched {
        let Some(p) = block.try_claim_clear_t::<Span<u8>>(a.max(1)) else { return false };
        Some(p)
    } else {
        None
    };
    // SAFETY: each claim just made in `rel`'s block, which the pin holds to write; a column's range starts empty.
    unsafe {
        for c in 0..a {
            *sp.add(c) = ColStats { min: i64::MAX, max: i64::MIN };
        }
        Span::set(&rel.stats, sp, a);
        Span::set(&rel.indexes, ixp, room);
        if let Some(p) = sk {
            Span::set(&rel.sketches, p.cast::<u8>(), a * size_of::<Span<u8>>());
        }
    }
    true
}

/// Column `c`'s range over the layer's own rows `rows` (its relation's, or its encoded form's: `layer::OwnRows`), into
/// its statistics (`open`'s).
pub fn stat(pin: &mut BlockPin, rel: &Relation, c: usize, rows: &dyn RowsIn) {
    check_open(pin, rel);
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    for i in 0..rows.n() {
        let v = rows.get(i, c);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    // SAFETY: the open relation's statistics, laid out by `open`; this call is column `c`'s one writer.
    unsafe { rel.stats.as_mut_slice()[c] = ColStats { min: lo, max: hi } };
}

/// Hashed index `x` of `spec`, over the layer's own rows `rows` (in id order, as they lie), into its directory entry
/// (`open`'s), its temporaries from `scratch`: after every column's range (`stat`). Its columns' bounds and numbers
/// on the stack, at most `MAX_COLS`.
pub fn index(pin: &mut BlockPin, rel: &Relation, spec: &LayerSpec, x: usize, scratch: &Scratch, rows: &dyn RowsIn) {
    use rerail_delta_core::runs::MAX_COLS;
    check_open(pin, rel);
    let a = rel.arity as usize;
    assert!(a <= MAX_COLS, "a layer of at most MAX_COLS columns");
    let n = rows.n();
    let stats = rel.stats.as_slice();
    let lo: [i64; MAX_COLS] = core::array::from_fn(|c| stats.get(c).map_or(0, |s| s.min));
    let hi: [i64; MAX_COLS] = core::array::from_fn(|c| stats.get(c).map_or(0, |s| s.max));
    let every: [u16; MAX_COLS] = core::array::from_fn(|c| c as u16);
    let order: &mut [u32] = scratch.take(n);
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u32;
    }
    let val = |c: usize, i: usize| rows.get(i, c);
    let sort = spec.sort().map(usize::from);
    // SAFETY: entry `x` of the open relation's directory (`open`'s); this call is its one writer.
    let ix = unsafe { &mut rel.indexes.as_mut_slice()[x] };
    build_one_index(pin.block(), ix, spec.key(x), &every[..a], sort, &val, order, &lo[..a], &hi[..a], rel.base_rows as usize, scratch);
}

/// Lays out the layer's own rows in `encoding` (not Open: `layer::ENC_*`) in `rel`'s block, for `n` rows of `arity`
/// columns lying in canonical order `order`, with a permutation index on each column of `perms` (Packed and Dict
/// only): a `packed::Packed`, or a `ranged::Columns`, empty until each column and index is written. Its offset in the
/// block (the layer's header's `enc`), or `None` when the block has no room.
pub fn encode_open(pin: &mut BlockPin, rel: &Relation, encoding: u32, arity: usize, n: usize, order: &[u16], perms: &[u16]) -> Option<usize> {
    check_open(pin, rel);
    let block = pin.block();
    let p = if encoding == ENC_RANGED {
        assert!(perms.is_empty(), "a permutation is a packed layer's");
        crate::ranged::writer::lay_out(block, arity, n)?.cast::<u8>()
    } else {
        crate::packed::writer::lay_out(block, arity, n, order, perms)?.cast::<u8>()
    };
    Some(block.offset_of(p))
}

/// Column `c` of the encoded rows at `at` (`encode_open`'s), from the layer's rows `rows` where they lie, its
/// temporaries (a Dict column's distinct values) from `scratch`. `Short::Block` when the block has no room;
/// `Short::Work` when `scratch`'s arena ran out, nothing written (the phase runs again over a larger scratch, and
/// the columns written before are left as they are).
pub fn encode_col(pin: &mut BlockPin, rel: &Relation, encoding: u32, at: usize, c: usize, rows: &dyn RowsIn, scratch: &Scratch) -> Result<(), Short> {
    check_open(pin, rel);
    let block = pin.block();
    // SAFETY: laid out at `at` by `encode_open` in the block the pin holds to write; column `c` is this call's alone.
    unsafe {
        if encoding == ENC_RANGED {
            // Only a Dict column takes work room, so a ranged layer's phase never runs again.
            if crate::ranged::writer::write_col(block, &*block.ptr::<crate::ranged::Columns>(at), c, rows) { Ok(()) } else { Err(Short::Block) }
        } else {
            crate::packed::writer::write_col(block, &*block.ptr::<crate::packed::Packed>(at), c, rows, encoding == ENC_DICT, scratch)
        }
    }
}

/// Permutation index `x` of the packed rows at `at` (`encode_open`'s), from `rows`, its temporaries from `scratch`.
/// False when the block has no room.
pub fn encode_perm(pin: &mut BlockPin, rel: &Relation, at: usize, x: usize, rows: &dyn RowsIn, scratch: &Scratch) -> bool {
    check_open(pin, rel);
    let block = pin.block();
    // SAFETY: as `encode_col`'s, for index `x`.
    unsafe { crate::packed::writer::write_perm(block, &*block.ptr::<crate::packed::Packed>(at), x, rows, scratch) }
}

/// Freezes the layer: sorted on its spec's sort column (its rows were written in the trace's canonical order), its
/// directory holding its hashed indexes (each built, `index`), its `n` own rows live (its relation's, or its encoded
/// form's). After every item of its other phases, and before its orders (`Relation::add_order`) and sketches
/// (`sketch`).
pub fn seal(pin: &mut BlockPin, rel: &Relation, spec: &LayerSpec, n: u32) {
    check_open(pin, rel);
    assert!(rel.changed.as_slice().is_empty(), "a layer's kills are kept once, in its gone relation: its rows record no change");
    rel.set_sort_col(spec.sort().map(usize::from));
    rel.nindex.store(spec.nkeys() as u32, Relaxed);
    assert!(rel.rows.is_empty() || rel.rows.len() == n, "a layer's rows are its relation's or its encoded form's");
    rel.live.store(n, Relaxed);
    rel.flags.fetch_or(FROZEN, Release);
}

/// Column `c`'s sketch `s` (its working form, in the phase's scratch), settled as its kept bytes into an exact claim
/// of the block, its span set in its entry of the layer's sketches' directory (`open`'s, `sketched`). False when the
/// block has no room.
fn settle_sketch(pin: &mut BlockPin, rel: &Relation, c: usize, s: &rerail_delta_core::sketch::SketchW<'_>) -> bool {
    let block = pin.block();
    assert!(block.holds(std::ptr::from_ref(rel)) && pin.writes(), "a layer of a block pinned to write");
    let dir = sketches(rel);
    assert!(c < dir.len() && dir[c].is_empty(), "column {c}'s sketch, written once");
    let Some(kept) = block.try_settle(s) else { return false };
    // SAFETY: the kept bytes just settled in the block the pin holds to write; entry `c` of the directory is this
    // call's alone.
    unsafe { Span::set(&dir[c], kept.as_ptr().cast_mut(), kept.len()) };
    true
}

/// The layer's sketches' directory: a `Span<u8>` a column, empty when it keeps none.
fn sketches(rel: &Relation) -> &[Span<u8>] {
    let b = rel.sketches.as_slice();
    if b.is_empty() {
        return &[];
    }
    // SAFETY: `open` laid the directory out as `Span<u8>`s, aligned to 8, in the relation's block.
    unsafe { std::slice::from_raw_parts(b.as_ptr().cast::<Span<u8>>(), b.len() / size_of::<Span<u8>>()) }
}

// Column `c`'s kept sketch in a layer's sketches' directory: the floor's read (`rerail_delta_core::sketch::kept`).
pub use rerail_delta_core::sketch::kept as layer_sketch;
