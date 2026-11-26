//! A trace's layer written and frozen in place, an item at a time (decision 0686; *0686: Operations* §4, §10): what the
//! phases of `consolidate` and `merge` (`rerail_delta_twins::trace`) call to lay a layer's `Relation` out in its block,
//! through the write pin of its slot. Each function is one item of a phase, independent of the phase's other items:
//!
//! - `open`: the freeze's directories, laid out once (the phase that opens the layer): its columns' statistics, its
//!   index directory, its sketches' directory, its change (`changed`, room for its kills);
//! - `stat`: one column's range;
//! - `index`: one of its spec's hashed indexes (`build_indexes`' builder, one index a call), after its columns' ranges;
//! - `kills_mut`: its change's ids, which the kills' sort writes in place;
//! - `encode_open`, `encode_col`, `encode_perm`: its own rows in its shape's encoding (P§12: Packed, Dict, Ranged),
//!   laid out once, then a column a call and a permutation index a call, read where the rows lie;
//! - `seal`: the relation frozen (its sort column, its directory's count, its live rows), its rows' every other item
//!   done;
//! - `sketch`: one column's statistics, in its kept form, into its entry of the directory.
//!
//! A layer's rows are written in the trace's canonical order, each once (its writer made them a set change): so a
//! freeze here sorts and deduplicates nothing.
//!
//! **The twins' rules hold here** (the twins' crate doc): nothing outlives a call or comes from outside it: no
//! `static`, thread-local, `Once`/`Lazy` cell or lock, no thread's scratch (a call is given its own `Scratch`), no
//! file, environment, process or time. `check_delta_exec_shape.py` checks this module with the twins, and its clippy
//! lints are denied here (blocks' `clippy.toml` lists the twins' paths). Its `unsafe` is blocks' own layout writing:
//! a chunk allocated in the block the pin holds to write, and spans set to it.

#![deny(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]

use super::{ADDED_INDEXES, ColStats, FROZEN, Index, Relation, Spec, build_one_index};
use crate::layer::{ENC_DICT, ENC_RANGED};
use crate::packed::RowsIn;
use crate::refs::Span;
use crate::region::BlockPin;
use crate::scratch::Scratch;
use std::sync::atomic::Ordering::{Relaxed, Release};

fn check_open(pin: &BlockPin, rel: &Relation) {
    assert!(pin.block().holds(std::ptr::from_ref(rel)) && pin.writes() && !rel.is_frozen(), "an open layer of a block pinned to write");
}

/// Lays out the freeze's directories of an open layer `rel` of `spec`: a range a column (`stat`), the index directory
/// (`spec.indexes`, and room for the orders added after it is frozen), a sketch a column when `sketched` (`sketch`),
/// and its change, room for `kills` ids (`kills_mut`). False when the block has no room.
pub fn open(pin: &mut BlockPin, rel: &Relation, spec: &Spec, kills: usize, sketched: bool) -> bool {
    check_open(pin, rel);
    let block = pin.block();
    let a = rel.arity as usize;
    let room = ADDED_INDEXES.max(spec.indexes.len());
    let (Some(sp), Some(ixp)) = (block.try_alloc_t::<ColStats>(a.max(1)), block.try_alloc_t::<Index>(room)) else { return false };
    let sk = if sketched {
        let Some(p) = block.try_alloc_t::<Span<u8>>(a.max(1)) else { return false };
        Some(p)
    } else {
        None
    };
    let cp = if kills > 0 {
        let Some(p) = block.try_alloc_t::<u32>(kills) else { return false };
        Some(p)
    } else {
        None
    };
    // SAFETY: each chunk just allocated in `rel`'s block, which the pin holds to write; a column's range starts empty.
    unsafe {
        for c in 0..a {
            *sp.add(c) = ColStats { min: i64::MAX, max: i64::MIN };
        }
        Span::set(&rel.stats, sp, a);
        Span::set(&rel.indexes, ixp, room);
        if let Some(p) = sk {
            Span::set(&rel.sketches, p.cast::<u8>(), a * size_of::<Span<u8>>());
        }
        if let Some(p) = cp {
            Span::set(&rel.changed, p, kills);
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

/// Index `x` of `spec.indexes`, over the layer's own rows `rows` (in id order, as they lie), into its directory entry
/// (`open`'s), its temporaries from `scratch`: after every column's range (`stat`).
pub fn index(pin: &mut BlockPin, rel: &Relation, spec: &Spec, x: usize, scratch: &Scratch, rows: &dyn RowsIn) {
    check_open(pin, rel);
    let a = rel.arity as usize;
    let n = rows.n();
    let stats = rel.stats.as_slice();
    let lo: Vec<i64> = stats.iter().map(|s| s.min).collect(); // copy: not block data (the key columns' bounds)
    let hi: Vec<i64> = stats.iter().map(|s| s.max).collect(); // copy: not block data (the key columns' bounds)
    let order: &mut [u32] = scratch.take(n);
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u32;
    }
    let every: Vec<u16> = (0..a as u16).collect(); // copy: not block data (the columns' numbers)
    let val = |c: usize, i: usize| rows.get(i, c);
    let sort = spec.sort.map(usize::from);
    // SAFETY: entry `x` of the open relation's directory (`open`'s); this call is its one writer.
    let ix = unsafe { &mut rel.indexes.as_mut_slice()[x] };
    build_one_index(pin.block(), ix, &spec.indexes[x], &every, sort, &val, order, &lo, &hi, rel.base_rows as usize, scratch);
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

/// Column `c` of the encoded rows at `at` (`encode_open`'s), from the layer's rows `rows` where they lie. False when
/// the block has no room.
pub fn encode_col(pin: &mut BlockPin, rel: &Relation, encoding: u32, at: usize, c: usize, rows: &dyn RowsIn) -> bool {
    check_open(pin, rel);
    let block = pin.block();
    // SAFETY: laid out at `at` by `encode_open` in the block the pin holds to write; column `c` is this call's alone.
    unsafe {
        if encoding == ENC_RANGED {
            crate::ranged::writer::write_col(block, &*block.ptr::<crate::ranged::Columns>(at), c, rows)
        } else {
            crate::packed::writer::write_col(block, &*block.ptr::<crate::packed::Packed>(at), c, rows, encoding == ENC_DICT)
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

/// The layer's change: room for its kills (`open`'s), written in place by the kills' sort, ascending, each once.
#[allow(clippy::mut_from_ref)]
pub fn kills_mut<'p>(pin: &'p mut BlockPin, rel: &Relation) -> &'p mut [u32] {
    check_open(pin, rel);
    // SAFETY: the open relation's change, laid out by `open` in the block `pin` holds to write; borrowed for as long as
    // the pin is.
    unsafe { std::mem::transmute::<&mut [u32], &'p mut [u32]>(rel.changed.as_mut_slice()) }
}

/// Freezes the layer: sorted on `spec.sort` (its rows were written in the trace's canonical order), its directory
/// holding `spec.indexes` (each built, `index`), its `n` own rows live (its relation's, or its encoded form's). After
/// every item of its other phases, and before its orders (`Relation::add_order`) and sketches (`sketch`).
pub fn seal(pin: &mut BlockPin, rel: &Relation, spec: &Spec, n: u32) {
    check_open(pin, rel);
    let ch = rel.changed.as_slice();
    assert!(ch.windows(2).all(|w| w[0] < w[1]) && ch.last().is_none_or(|&k| k < rel.base_rows), "a layer's kills: rows below it, ascending, once");
    rel.set_sort_col(spec.sort.map(usize::from));
    rel.nindex.store(spec.indexes.len() as u32, Relaxed);
    assert!(rel.rows.is_empty() || rel.rows.len() == n, "a layer's rows are its relation's or its encoded form's");
    rel.live.store(n, Relaxed);
    rel.flags.fetch_or(FROZEN, Release);
}

/// Column `c`'s statistics, in its kept form (`Sketch::encode`), into its entry of the layer's sketches' directory
/// (`open`'s, `sketched`): a chunk of the block of its own. False when the block has no room.
pub fn sketch(pin: &mut BlockPin, rel: &Relation, c: usize, kept: &[u8]) -> bool {
    let block = pin.block();
    assert!(block.holds(std::ptr::from_ref(rel)) && pin.writes(), "a layer of a block pinned to write");
    let dir = sketches(rel);
    assert!(c < dir.len() && dir[c].is_empty(), "column {c}'s sketch, written once");
    let Some(p) = block.try_alloc_t::<u8>(kept.len().max(1)) else { return false };
    // SAFETY: `p` holds `kept.len()` bytes, just allocated in the block the pin holds to write; entry `c` of the
    // directory is this call's alone.
    unsafe {
        // copy: derived: a column's statistics, computed by its writer, written once where the layer keeps them
        std::ptr::copy_nonoverlapping(kept.as_ptr(), p, kept.len());
        Span::set(&dir[c], p, kept.len());
    }
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

/// Column `c`'s statistics in a layer's sketches' directory, its kept form's bytes; `None` when it keeps none.
pub fn layer_sketch(rel: &Relation, c: usize) -> Option<&[u8]> {
    sketches(rel).get(c).filter(|s| !s.is_empty()).map(Span::as_slice)
}
