//! A trace's shape as its blocks hold it, and the δ runs block (moved from `rerail_blocks::layer`).
//!
//! A δ runs block holds a run's `(row, diff)` pairs, column by column, then the diffs: the rows a layer is made from
//! (`consolidate`'s source). A writer claims a place, then writes its cells; the many-writer form, by atomic claims
//! through a write pin, is `rerail_blocks::layer::RunsW`, over this layout.

use crate::block::BODY_AT;
use crate::pod::{Pod, set_word, word};

/// The most columns a trace's relation has.
pub const MAX_COLS: usize = 16;
/// The most indexes a trace's shape asks for, beside its canonical order.
pub const MAX_IX: usize = 4;
/// The most columns an index's key has.
pub const MAX_KEY: usize = 8;
/// No block, no partition, no kill.
pub const NONE: u32 = u32::MAX;

/// A δ runs block's mark.
pub const RUNS_MAGIC: u32 = u32::from_le_bytes(*b"RUN0");

/// One index of a shape, as its blocks hold it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexW {
    pub kind: u32,
    pub ncols: u32,
    pub cols: [u8; MAX_KEY],
}

/// A shape as its blocks hold it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeW {
    pub arity: u32,
    pub partition: u32,
    pub levels: u32,
    pub encoding: u32,
    pub placement: u32,
    pub retention: u32,
    pub nidx: u32,
    pub pad: u32,
    /// 1 where the column is 64 bits wide.
    pub types: [u8; MAX_COLS],
    pub order: [u8; MAX_COLS],
    pub indexes: [IndexW; MAX_IX],
}

// SAFETY: u32s and byte arrays, laid out with no padding.
unsafe impl Pod for ShapeW {}

impl ShapeW {
    pub fn arity(&self) -> usize {
        self.arity as usize
    }

    pub fn width(&self, c: usize) -> usize {
        if self.types[c] == 1 { 8 } else { 4 }
    }

    pub fn partition(&self) -> Option<usize> {
        (self.partition != NONE).then_some(self.partition as usize)
    }

    pub fn order(&self) -> &[u8] {
        &self.order[..self.arity()]
    }

    /// A recursive relation's level column (0364): its last.
    pub fn level_col(&self) -> Option<usize> {
        (self.levels != 0).then(|| self.arity() - 1)
    }

    /// The columns that name a row apart from its level: the canonical order less a level column.
    pub fn key_order(&self) -> &[u8] {
        &self.order()[..self.arity() - usize::from(self.levels != 0)]
    }

    /// Index `i`'s key (`i < nidx`).
    pub fn index_key(&self, i: usize) -> &[u8] {
        &self.indexes[i].cols[..self.indexes[i].ncols as usize]
    }

    /// Index `i`'s kind bits (`IX_*`).
    pub fn index_kind(&self, i: usize) -> u32 {
        self.indexes[i].kind
    }
}

/// `n` rounded up to a multiple of `to`.
pub fn round_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

/// Reads column cell `i` of a column of `width` bytes at `at`.
#[inline]
pub fn cell(bytes: &[u8], at: usize, width: usize, i: usize) -> i64 {
    if width == 8 { word::<i64>(bytes, at + 8 * i) } else { i64::from(word::<i32>(bytes, at + 4 * i)) }
}

/// A δ runs block's header, at `BODY_AT`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RunsHead {
    /// The body's bytes (`gens::body_len`).
    pub len: u64,
    pub magic: u32,
    pub cap: u32,
    /// Claims made: a writer's place is its claim, when below `cap`.
    pub claimed: u32,
    /// Claims past `cap`: the size a rerun needs, beyond it.
    pub needed: u32,
    pub shape: ShapeW,
    pub col_at: [u64; MAX_COLS],
    pub diff_at: u64,
}

// SAFETY: integers and `ShapeW`, laid out with no padding.
unsafe impl Pod for RunsHead {}

/// Where the header's claim count and its count past the capacity lie: the words many writers add to at once.
pub const CLAIMED_AT: usize = BODY_AT + 16;
pub const NEEDED_AT: usize = BODY_AT + 20;
const _: () = assert!(CLAIMED_AT == BODY_AT + core::mem::offset_of!(RunsHead, claimed));
const _: () = assert!(NEEDED_AT == BODY_AT + core::mem::offset_of!(RunsHead, needed));

/// The body bytes a runs block of `cap` rows of `shape` takes.
pub fn runs_bytes(shape: &ShapeW, cap: usize) -> usize {
    runs_layout(shape, cap).2
}

fn runs_layout(shape: &ShapeW, cap: usize) -> ([u64; MAX_COLS], u64, usize) {
    let mut at = round_up(BODY_AT + size_of::<RunsHead>(), 64);
    let mut col_at = [0u64; MAX_COLS];
    for (c, slot) in col_at.iter_mut().enumerate().take(shape.arity()) {
        *slot = at as u64;
        at = round_up(at + shape.width(c) * cap, 64);
    }
    let diff_at = at as u64;
    at = round_up(at + 4 * cap, 64);
    (col_at, diff_at, at - BODY_AT)
}

/// Lays an empty runs block out in a write pin's bytes: `cap` rows of `shape`.
pub fn init_runs(bytes: &mut [u8], shape: &ShapeW, cap: usize) {
    let (col_at, diff_at, len) = runs_layout(shape, cap);
    let h = RunsHead { len: len as u64, magic: RUNS_MAGIC, cap: cap as u32, claimed: 0, needed: 0, shape: *shape, col_at, diff_at };
    set_word(bytes, BODY_AT, h);
}

/// Claims `n` places at once in the runs block `bytes` (an emit's scan, its one writer): the first, or `None` when
/// they do not all fit, the claims past the capacity counted as needed (the driver runs again at the size needed).
pub fn runs_claim_in(bytes: &mut [u8], n: u32) -> Option<u32> {
    let h: RunsHead = word(bytes, BODY_AT);
    assert_eq!(h.magic, RUNS_MAGIC, "a runs block");
    let (old, new) = (h.claimed, h.claimed + n);
    set_word::<u32>(bytes, CLAIMED_AT, new);
    if new > h.cap {
        set_word::<u32>(bytes, NEEDED_AT, h.needed + new - old.max(h.cap));
        return None;
    }
    Some(old)
}

/// Writes `row` with `diff` at place `i` of the runs block `bytes`, a place claimed (`runs_claim_in`): an emit's place.
pub fn runs_set_in(bytes: &mut [u8], i: u32, row: &[i64], diff: i32) {
    let h: RunsHead = word(bytes, BODY_AT);
    let s = &h.shape;
    assert!(i < h.cap && row.len() == s.arity(), "a claimed place, a row of the trace's arity");
    for (c, &v) in row.iter().enumerate() {
        let at = h.col_at[c] as usize;
        if s.width(c) == 8 {
            set_word::<i64>(bytes, at + 8 * i as usize, v);
        } else {
            let v = i32::try_from(v).unwrap_or_else(|_| panic!("column {c} holds 32 bits: {v}"));
            set_word::<i32>(bytes, at + 4 * i as usize, v);
        }
    }
    set_word::<i32>(bytes, h.diff_at as usize + 4 * i as usize, diff);
}

/// A δ runs block, read in place.
#[derive(Clone, Copy)]
pub struct Runs<'a> {
    bytes: &'a [u8],
    pub head: RunsHead,
}

impl<'a> Runs<'a> {
    pub fn new(bytes: &'a [u8]) -> Runs<'a> {
        let head: RunsHead = word(bytes, BODY_AT);
        assert_eq!(head.magic, RUNS_MAGIC, "a runs block");
        Runs { bytes, head }
    }

    /// Its rows: the claims within its capacity.
    pub fn len(&self) -> usize {
        self.head.claimed.min(self.head.cap) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The claims past its capacity.
    pub fn needed(&self) -> usize {
        self.head.needed as usize
    }

    pub fn get(&self, i: usize, c: usize) -> i64 {
        cell(self.bytes, self.head.col_at[c] as usize, self.head.shape.width(c), i)
    }

    pub fn diff(&self, i: usize) -> i32 {
        word::<i32>(self.bytes, self.head.diff_at as usize + 4 * i)
    }
}
