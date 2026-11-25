//! A trace's blocks (decision 0686; *0686: Operations* §7 and §10; *Delta-first engine primitives* §11–§12): the
//! run's δ runs, and the layers `consolidate` and `merge` write from them, each layer its own block.
//!
//! **A trace** is one relation's history: a chain of frozen layer blocks, newest first, each referring to the layer
//! below it by block id (`gens`). The host holds only the newest layer's id. Two states are read (the user,
//! 2026-11-24): the committed chain (before), and the chain with the run's new layer on it (after). The before state
//! stays readable while the next layer is written: that is a new block.
//!
//! **δ runs** (`Runs`, a scratch block): a run's `(row, diff)` pairs as its operations write them, each writer
//! claiming its place with one atomic add (host threads or a GPU's); past the capacity a claim writes nothing and
//! counts what was needed, and the run is redone at that size (shape 12).
//!
//! **A layer** (`Layer`): the δ, consolidated, in its trace's canonical order, grouped by partition first: sorted
//! columns of rows, a sign a row (`+1` added, `-1` gone), for each gone row the id of the row it kills, those ids
//! ascending (globally, and within each partition), the partition index with each partition's digest and counts, a
//! statistics sketch per column, and the indexes the trace's shape asks for. Its rows' ids continue the layer below's
//! (`start`). Its header's state is written last: a layer is read only once committed.
//!
//! **Reads** (`Chain`): a trace's state at a layer, read in place through a launch's descriptor table, as
//! `bistate::Read`: a row is live when it was added and no layer at or below the state's has killed it. Its change
//! since the layer below is that layer's own: `Bistate` reads it in its size.
//!
//! Every column is fixed-width, 32 bits where the values fit (`ColType`).

use crate::bistate::{ColStats, ColumnIndex, Flips, Read, RowId, Since, States, WordIndex, below};
use crate::gens::{BODY_AT, Id, Table, set_word, word, words, words_mut};
use std::cmp::Ordering;

/// The most columns a trace's relation has.
pub const MAX_COLS: usize = 16;
/// The most indexes a trace's shape asks for, beside its canonical order.
pub const MAX_IX: usize = 4;
/// The most columns an index's key has.
pub const MAX_KEY: usize = 8;
/// No block, no partition, no kill.
pub const NONE: u32 = u32::MAX;

const RUNS_MAGIC: u32 = u32::from_le_bytes(*b"RUN0");
const LAYER_MAGIC: u32 = u32::from_le_bytes(*b"LAY0");
/// A layer being written.
pub const OPEN: u32 = 1;
/// A layer committed: its header was written last.
pub const COMMITTED: u32 = 2;

/// A column's type: its width in the block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColType {
    I32,
    I64,
}

impl ColType {
    pub fn width(self) -> usize {
        match self {
            ColType::I32 => 4,
            ColType::I64 => 8,
        }
    }
}

/// An index a trace's shape asks for (P§11.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IndexSpec {
    Hash(Vec<u16>),
    /// The rows' ids in the key's order (`blocks::order`), with fences and prints when asked.
    Order { key: Vec<u16>, fences: bool, prints: bool },
    /// Packed's permutation: the rows' ids in the key's order.
    Perm(Vec<u16>),
    Bloom(Vec<u16>),
}

/// How a trace's layers are stored (P§12): chosen for cost, never changing what the rows are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    Open,
    Packed,
    Ranged,
    Dict,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    RunBlock,
    KeptFile,
    PinnedSlab,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Retention {
    Run,
    Cache,
    History,
}

/// A trace's shape (P§12).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shape {
    pub columns: Vec<ColType>,
    /// The canonical order: every column once, the partition first.
    pub order: Vec<u16>,
    pub indexes: Vec<IndexSpec>,
    pub partition: Option<u16>,
    /// A recursive relation's level column (0364).
    pub levels: bool,
    pub encoding: Encoding,
    pub placement: Placement,
    pub retention: Retention,
}

impl Shape {
    /// A run's trace of `columns`, sorted in `order`, partitioned by `partition`, stored open in the run's blocks.
    pub fn run(columns: Vec<ColType>, order: Vec<u16>, partition: Option<u16>) -> Shape {
        Shape {
            columns,
            order,
            indexes: Vec::new(),
            partition,
            levels: false,
            encoding: Encoding::Open,
            placement: Placement::RunBlock,
            retention: Retention::Run,
        }
    }

    /// The shape as its blocks hold it, or why it cannot be stored. What is not built yet is refused by name.
    pub fn encode(&self) -> Result<ShapeW, String> {
        let a = self.columns.len();
        if a == 0 || a > MAX_COLS {
            return Err(format!("a trace's relation has 1 to {MAX_COLS} columns, not {a}"));
        }
        let mut seen = [false; MAX_COLS];
        if self.order.len() != a || self.order.iter().any(|&c| (c as usize) >= a || std::mem::replace(&mut seen[c as usize], true)) {
            return Err(format!("the canonical order {:?} is not every one of the {a} columns once", self.order));
        }
        if let Some(p) = self.partition
            && self.order[0] != p
        {
            return Err(format!("rows are grouped by partition first (P§11.2): the order starts with column {p}"));
        }
        if let Some(p) = self.partition
            && self.columns[p as usize] != ColType::I32
        {
            return Err(format!("the partition column {p} is a module's or a document's id: 32 bits"));
        }
        // What a run block's trace is not, each where the plan puts it.
        if self.levels {
            return Err("a level column (0364) is a recursive stratum's, run by Δfix after L1 (the rebuild plan, \
                        after the target): not in B2"
                .into());
        }
        if self.encoding != Encoding::Open {
            return Err(format!(
                "encoding {:?} is the form of a block that leaves its run (P§13: kept, journaled or a pinned slab); a run \
                 block's layers are Open",
                self.encoding
            ));
        }
        if self.placement != Placement::RunBlock {
            return Err(format!(
                "placement {:?}: kept files and pinned slabs come with the kept stores and the device executor, after L1 \
                 (the rebuild plan); B2's traces are run blocks",
                self.placement
            ));
        }
        match self.retention {
            Retention::Run => {}
            Retention::Cache => {
                return Err("retention Cache is the operator cache's (P§11.3), B3's: one state mutated in place, not a trace".into());
            }
            Retention::History => {
                return Err("retention History keeps named frontiers; a trace has two states (the user, 2026-11-25)".into());
            }
        }
        if self.indexes.len() > MAX_IX {
            return Err(format!("at most {MAX_IX} indexes beside the canonical order"));
        }
        let mut w = ShapeW {
            arity: a as u32,
            partition: self.partition.map_or(NONE, u32::from),
            levels: 0,
            encoding: 0,
            placement: 0,
            retention: 0,
            nidx: self.indexes.len() as u32,
            pad: 0,
            types: [0; MAX_COLS],
            order: [0; MAX_COLS],
            indexes: [IndexW { kind: 0, ncols: 0, cols: [0; MAX_KEY] }; MAX_IX],
        };
        for (i, t) in self.columns.iter().enumerate() {
            w.types[i] = (*t == ColType::I64) as u8;
        }
        for (i, &c) in self.order.iter().enumerate() {
            w.order[i] = c as u8;
        }
        for (i, ix) in self.indexes.iter().enumerate() {
            let (kind, key) = match ix {
                IndexSpec::Order { key, fences, prints } => (IX_ORDER | u32::from(*fences) * IX_FENCES | u32::from(*prints) * IX_PRINTS, key),
                IndexSpec::Perm(key) => (IX_ORDER, key),
                IndexSpec::Hash(key) => (IX_HASH | IX_PRINTS, key),
                IndexSpec::Bloom(key) => (IX_BLOOM, key),
            };
            if key.is_empty() || key.len() > MAX_KEY || key.iter().any(|&c| c as usize >= a) {
                return Err(format!("index {i}'s key {key:?}"));
            }
            w.indexes[i].kind = kind;
            w.indexes[i].ncols = key.len() as u32;
            for (j, &c) in key.iter().enumerate() {
                w.indexes[i].cols[j] = c as u8;
            }
        }
        Ok(w)
    }
}

/// An index's kind, as its blocks hold it: an order (the rows' positions sorted by its key), and beside it the
/// leading value of every `order::FENCE`th position (fences) and each position's 8-bit print of its whole key
/// (prints), the search `blocks::order::find` reads; a hash order (sorted by the lead's 16-bit `order::tag`, then the
/// key: a tag a position, and prints); a Bloom filter of the key (`blocks::bloom`'s hash and mask, its words a power of
/// two). One format each, written once, the existing ones.
pub const IX_ORDER: u32 = 1;
pub const IX_FENCES: u32 = 2;
pub const IX_PRINTS: u32 = 4;
pub const IX_HASH: u32 = 8;
pub const IX_BLOOM: u32 = 16;

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

unsafe impl crate::Pod for ShapeW {}

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

    /// Index `i`'s key (`i < nidx`).
    pub fn index_key(&self, i: usize) -> &[u8] {
        &self.indexes[i].cols[..self.indexes[i].ncols as usize]
    }

    /// Index `i`'s kind bits (`IX_*`).
    pub fn index_kind(&self, i: usize) -> u32 {
        self.indexes[i].kind
    }
}

/// A Bloom filter's words for `n` keys: about ten bits a key, a power of two.
pub fn bloom_words(n: usize) -> usize {
    (n * 10).div_ceil(64).max(1).next_power_of_two()
}

/// The one value a Bloom filter hashes for a key: the value itself for one column, else the key's `row_hash`.
pub fn bloom_key(key: impl ExactSizeIterator<Item = i64>) -> i64 {
    if key.len() == 1 {
        let mut k = key;
        k.next().expect("one value")
    } else {
        row_hash(key) as i64
    }
}

impl ShapeW {
}

fn round_up(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

/// Reads column cell `i` of a column of `width` bytes at `at`.
#[inline]
fn cell(bytes: &[u8], at: usize, width: usize, i: usize) -> i64 {
    if width == 8 { word::<i64>(bytes, at + 8 * i) } else { i64::from(word::<i32>(bytes, at + 4 * i)) }
}

// ---- δ runs -------------------------------------------------------------------------------------------------------

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

unsafe impl crate::Pod for RunsHead {}

const CLAIMED_AT: usize = BODY_AT + 16;
const NEEDED_AT: usize = BODY_AT + 20;

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

/// The run's writers of a δ runs block: each push claims its place with one atomic add, then writes its cells. Many
/// writers share one write pin (host threads, or a GPU's); none takes a lock.
pub struct RunsW<'a> {
    pin: &'a crate::region::BlockPin,
    head: RunsHead,
}

impl<'a> RunsW<'a> {
    /// The writers of the runs block `pin` holds, pinned to write.
    pub fn new(pin: &'a crate::region::BlockPin) -> RunsW<'a> {
        assert!(pin.writes(), "a runs block is written through a write pin");
        let head: RunsHead = word(pin.bytes(), BODY_AT);
        assert_eq!(head.magic, RUNS_MAGIC, "a runs block");
        RunsW { pin, head }
    }

    /// Adds `row` with `diff`: its place, or `None` past the capacity (counted as needed).
    pub fn push(&self, row: &[i64], diff: i32) -> Option<u32> {
        use std::sync::atomic::{AtomicI64, AtomicU32, Ordering::Relaxed};
        let s = &self.head.shape;
        assert_eq!(row.len(), s.arity(), "a row of the trace's arity");
        let i = self.pin.atomics::<AtomicU32>(CLAIMED_AT, 1).expect("a write pin")[0].fetch_add(1, Relaxed);
        if i >= self.head.cap {
            self.pin.atomics::<AtomicU32>(NEEDED_AT, 1).expect("a write pin")[0].fetch_add(1, Relaxed);
            return None;
        }
        for (c, &v) in row.iter().enumerate() {
            let at = self.head.col_at[c] as usize;
            if s.width(c) == 8 {
                self.pin.atomics::<AtomicI64>(at + 8 * i as usize, 1).expect("a write pin")[0].store(v, Relaxed);
            } else {
                let v = i32::try_from(v).unwrap_or_else(|_| panic!("column {c} holds 32 bits: {v}"));
                self.pin.atomics::<AtomicU32>(at + 4 * i as usize, 1).expect("a write pin")[0].store(v as u32, Relaxed);
            }
        }
        self.pin.atomics::<AtomicU32>(self.head.diff_at as usize + 4 * i as usize, 1).expect("a write pin")[0].store(diff as u32, Relaxed);
        Some(i)
    }
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

// ---- layers -------------------------------------------------------------------------------------------------------

/// One partition of a layer: its rows `lo..hi`, its gone rows' kill ids `flo..fhi` of the per-partition flips, its
/// counts and its digest (the sum over its rows of each row's hash times its sign).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PartW {
    /// The partition's value: a module's or a document's id, 32 bits.
    pub key: i32,
    pub lo: u32,
    pub hi: u32,
    pub flo: u32,
    pub fhi: u32,
    pub added: u32,
    pub gone: u32,
    pub pad: u32,
    pub digest: u64,
}

unsafe impl crate::Pod for PartW {}

/// The number of AMS counters a column's sketch keeps.
pub const AMS: usize = 8;

/// A column's statistics sketch in one layer: bounds of its added rows, and linear counters (adding two layers' is
/// the sketch of both).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchW {
    pub min: i64,
    pub max: i64,
    /// The sum of the rows' signs.
    pub count: i64,
    /// AMS counters: the sum of sign × ±1 by a hash bit of the value.
    pub ams: [i64; AMS],
}

unsafe impl crate::Pod for SketchW {}

impl SketchW {
    pub const EMPTY: SketchW = SketchW { min: i64::MAX, max: i64::MIN, count: 0, ams: [0; AMS] };

    /// Adds one row's value with its sign.
    pub fn add(&mut self, v: i64, diff: i32) {
        if diff > 0 {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.count += i64::from(diff);
        let h = mix(v as u64);
        for (k, a) in self.ams.iter_mut().enumerate() {
            *a += if h >> k & 1 == 1 { i64::from(diff) } else { -i64::from(diff) };
        }
    }

    /// Two sketches combined: the sketch of both layers' rows.
    pub fn combine(&self, o: &SketchW) -> SketchW {
        let mut ams = self.ams;
        for (a, b) in ams.iter_mut().zip(o.ams) {
            *a += b;
        }
        SketchW { min: self.min.min(o.min), max: self.max.max(o.max), count: self.count + o.count, ams }
    }
}

/// SplitMix64's finalizer.
#[inline]
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A row's hash, for partition digests: the same row the same hash in every layer.
pub fn row_hash(row: impl Iterator<Item = i64>) -> u64 {
    row.fold(0x9e37_79b9_7f4a_7c15u64, |h, v| mix(h ^ mix(v as u64)))
}

/// A layer block's header, at `BODY_AT`: written last (`state`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LayerHead {
    pub len: u64,
    pub magic: u32,
    pub state: u32,
    /// The layer below's block id (`NONE`: the first).
    pub below: u32,
    /// Its first row's id: the layer below's `end`.
    pub start: u32,
    pub n: u32,
    pub added: u32,
    pub gone: u32,
    pub nparts: u32,
    pub digest: u64,
    pub shape: ShapeW,
    pub col_at: [u64; MAX_COLS],
    /// The signs, a bit a row (set: added).
    pub plus_at: u64,
    /// Each row's kill: the id a gone row kills, `NONE` for an added row.
    pub kill_at: u64,
    /// The kill ids, ascending: the layer's flips.
    pub flips_at: u64,
    /// The kill ids by partition, ascending within each (`PartW::flo..fhi`).
    pub pflips_at: u64,
    pub parts_at: u64,
    pub sketch_at: u64,
    /// Each index's row positions in its key's order (a Bloom filter's: its words).
    pub idx_at: [u64; MAX_IX],
    /// Each index's fences (`i64`, a `FENCE`th position's lead) or tags (`u16` a position), where it keeps them.
    pub aux_at: [u64; MAX_IX],
    /// Each index's prints (`u8` a position), where it keeps them.
    pub prints_at: [u64; MAX_IX],
}

unsafe impl crate::Pod for LayerHead {}

/// Where a layer's columns go: its header with every offset, and the body bytes it takes, for `n` rows.
pub fn layer_layout(shape: &ShapeW, n: usize) -> LayerHead {
    let mut at = round_up(BODY_AT + size_of::<LayerHead>(), 64);
    let mut next = |bytes: usize| {
        let a = at;
        at = round_up(at + bytes, 64);
        a as u64
    };
    let mut col_at = [0u64; MAX_COLS];
    for (c, slot) in col_at.iter_mut().enumerate().take(shape.arity()) {
        *slot = next(shape.width(c) * n);
    }
    let plus_at = next(8 * n.div_ceil(64));
    let kill_at = next(4 * n);
    let flips_at = next(4 * n);
    let pflips_at = next(4 * n);
    let parts_at = next(size_of::<PartW>() * n);
    let sketch_at = next(size_of::<SketchW>() * shape.arity());
    let (mut idx_at, mut aux_at, mut prints_at) = ([0u64; MAX_IX], [0u64; MAX_IX], [0u64; MAX_IX]);
    for x in 0..shape.nidx as usize {
        let k = shape.index_kind(x);
        if k & IX_BLOOM != 0 {
            idx_at[x] = next(8 * bloom_words(n));
            continue;
        }
        idx_at[x] = next(4 * n);
        if k & IX_FENCES != 0 {
            aux_at[x] = next(8 * n.div_ceil(crate::order::FENCE));
        }
        if k & IX_HASH != 0 {
            aux_at[x] = next(2 * n);
        }
        if k & IX_PRINTS != 0 {
            prints_at[x] = next(n);
        }
    }
    LayerHead {
        len: (at - BODY_AT) as u64,
        magic: LAYER_MAGIC,
        state: OPEN,
        below: NONE,
        start: 0,
        n: n as u32,
        added: 0,
        gone: 0,
        nparts: 0,
        digest: 0,
        shape: *shape,
        col_at,
        plus_at,
        kill_at,
        flips_at,
        pflips_at,
        parts_at,
        sketch_at,
        idx_at,
        aux_at,
        prints_at,
    }
}

/// The body bytes a layer of `n` rows of `shape` takes.
pub fn layer_bytes(shape: &ShapeW, n: usize) -> usize {
    layer_layout(shape, n).len as usize
}

/// A layer's id end: its start, then its rows, rounded to a word so that the next layer's ids start a word.
pub fn layer_end(start: u32, n: u32) -> u32 {
    start + round_up(n as usize, 64) as u32
}

/// The writer's view of a layer being written: its header (offsets) and its bytes. Used by the operations that write
/// layers (`consolidate`, `merge`), in their write slot.
pub struct LayerOut<'a> {
    pub bytes: &'a mut [u8],
    pub head: LayerHead,
}

impl<'a> LayerOut<'a> {
    /// The layer at `BODY_AT` of `bytes`, as last written.
    pub fn new(bytes: &'a mut [u8]) -> LayerOut<'a> {
        let head: LayerHead = word(bytes, BODY_AT);
        assert_eq!(head.magic, LAYER_MAGIC, "a layer block");
        LayerOut { bytes, head }
    }

    /// Lays out an open layer of `n` rows of `shape`, starting at id `start`, over `below`.
    pub fn init(bytes: &'a mut [u8], shape: &ShapeW, n: usize, start: u32, below: u32) -> LayerOut<'a> {
        let mut head = layer_layout(shape, n);
        head.start = start;
        head.below = below;
        assert!(BODY_AT + head.len as usize <= bytes.len(), "a layer of {n} rows needs {} body bytes", head.len);
        set_word(bytes, BODY_AT, head);
        LayerOut { bytes, head }
    }

    pub fn set(&mut self, i: usize, c: usize, v: i64) {
        let at = self.head.col_at[c] as usize;
        if self.head.shape.width(c) == 8 {
            set_word::<i64>(self.bytes, at + 8 * i, v);
        } else {
            let v = i32::try_from(v).unwrap_or_else(|_| panic!("column {c} holds 32 bits: {v}"));
            set_word::<i32>(self.bytes, at + 4 * i, v);
        }
    }

    pub fn get(&self, i: usize, c: usize) -> i64 {
        cell(self.bytes, self.head.col_at[c] as usize, self.head.shape.width(c), i)
    }

    /// Whether row `i`'s sign, as written, is plus.
    pub fn is_plus(&self, i: usize) -> bool {
        word::<u64>(self.bytes, self.head.plus_at as usize + 8 * (i / 64)) >> (i % 64) & 1 == 1
    }

    /// Row `i`'s kill id, as written.
    pub fn kill(&self, i: usize) -> u32 {
        word::<u32>(self.bytes, self.head.kill_at as usize + 4 * i)
    }

    /// The sign words (a bit a row).
    pub fn plus(&mut self) -> &mut [u64] {
        words_mut(self.bytes, self.head.plus_at as usize, (self.head.n as usize).div_ceil(64))
    }

    pub fn kills(&mut self) -> &mut [u32] {
        words_mut(self.bytes, self.head.kill_at as usize, self.head.n as usize)
    }

    pub fn flips(&mut self) -> &mut [u32] {
        words_mut(self.bytes, self.head.flips_at as usize, self.head.n as usize)
    }

    pub fn pflips(&mut self) -> &mut [u32] {
        words_mut(self.bytes, self.head.pflips_at as usize, self.head.n as usize)
    }

    pub fn parts(&mut self) -> &mut [PartW] {
        words_mut(self.bytes, self.head.parts_at as usize, self.head.n as usize)
    }

    pub fn sketches(&mut self) -> &mut [SketchW] {
        words_mut(self.bytes, self.head.sketch_at as usize, self.head.shape.arity())
    }

    pub fn index(&mut self, i: usize) -> &mut [u32] {
        words_mut(self.bytes, self.head.idx_at[i] as usize, self.head.n as usize)
    }

    pub fn fences(&mut self, i: usize) -> &mut [i64] {
        words_mut(self.bytes, self.head.aux_at[i] as usize, (self.head.n as usize).div_ceil(crate::order::FENCE))
    }

    pub fn tags(&mut self, i: usize) -> &mut [u16] {
        words_mut(self.bytes, self.head.aux_at[i] as usize, self.head.n as usize)
    }

    pub fn prints(&mut self, i: usize) -> &mut [u8] {
        words_mut(self.bytes, self.head.prints_at[i] as usize, self.head.n as usize)
    }

    pub fn bloom(&mut self, i: usize) -> &mut [u64] {
        words_mut(self.bytes, self.head.idx_at[i] as usize, bloom_words(self.head.n as usize))
    }

    /// Writes the header with its counts and its state, committed: last.
    pub fn commit(&mut self, added: u32, gone: u32, nparts: u32, digest: u64) {
        let mut h = self.head;
        h.added = added;
        h.gone = gone;
        h.nparts = nparts;
        h.digest = digest;
        h.state = OPEN;
        set_word(self.bytes, BODY_AT, h);
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
        h.state = COMMITTED;
        set_word(self.bytes, BODY_AT, h);
        self.head = h;
    }
}

/// A committed layer, read in place.
#[derive(Clone, Copy)]
pub struct Layer<'a> {
    bytes: &'a [u8],
    pub head: LayerHead,
}

impl<'a> Layer<'a> {
    /// The layer in `bytes`: `Err` when they hold none, or one not committed.
    pub fn new(bytes: &'a [u8]) -> Result<Layer<'a>, String> {
        let head: LayerHead = word(bytes, BODY_AT);
        if head.magic != LAYER_MAGIC {
            return Err("not a layer block".into());
        }
        if head.state != COMMITTED {
            return Err("a layer not committed".into());
        }
        Ok(Layer { bytes, head })
    }

    pub fn n(&self) -> usize {
        self.head.n as usize
    }

    pub fn start(&self) -> u32 {
        self.head.start
    }

    /// The id past its rows' word: the next layer's start.
    pub fn end(&self) -> u32 {
        layer_end(self.head.start, self.head.n)
    }

    pub fn below(&self) -> Option<Id> {
        (self.head.below != NONE).then_some(self.head.below)
    }

    pub fn get(&self, i: usize, c: usize) -> i64 {
        cell(self.bytes, self.head.col_at[c] as usize, self.head.shape.width(c), i)
    }

    pub fn plus(&self) -> &'a [u64] {
        words(self.bytes, self.head.plus_at as usize, self.n().div_ceil(64))
    }

    /// Whether row `i` (local) is added (`+1`), not gone.
    pub fn is_plus(&self, i: usize) -> bool {
        self.plus()[i / 64] >> (i % 64) & 1 == 1
    }

    pub fn kills(&self) -> &'a [u32] {
        words(self.bytes, self.head.kill_at as usize, self.n())
    }

    pub fn flips(&self) -> &'a [u32] {
        words(self.bytes, self.head.flips_at as usize, self.head.gone as usize)
    }

    pub fn pflips(&self) -> &'a [u32] {
        words(self.bytes, self.head.pflips_at as usize, self.head.gone as usize)
    }

    pub fn parts(&self) -> &'a [PartW] {
        words(self.bytes, self.head.parts_at as usize, self.head.nparts as usize)
    }

    pub fn sketches(&self) -> &'a [SketchW] {
        words(self.bytes, self.head.sketch_at as usize, self.head.shape.arity())
    }

    pub fn index(&self, i: usize) -> &'a [u32] {
        words(self.bytes, self.head.idx_at[i] as usize, self.n())
    }

    /// Index `i`'s fences, when it keeps them.
    pub fn fences(&self, i: usize) -> &'a [i64] {
        if self.head.shape.index_kind(i) & IX_FENCES == 0 {
            return &[];
        }
        words(self.bytes, self.head.aux_at[i] as usize, self.n().div_ceil(crate::order::FENCE))
    }

    /// Index `i`'s tags, when it is a hash order.
    pub fn tags(&self, i: usize) -> &'a [u16] {
        if self.head.shape.index_kind(i) & IX_HASH == 0 {
            return &[];
        }
        words(self.bytes, self.head.aux_at[i] as usize, self.n())
    }

    /// Index `i`'s prints, when it keeps them.
    pub fn prints(&self, i: usize) -> &'a [u8] {
        if self.head.shape.index_kind(i) & IX_PRINTS == 0 {
            return &[];
        }
        words(self.bytes, self.head.prints_at[i] as usize, self.n())
    }

    /// Index `i`'s Bloom words.
    pub fn bloom(&self, i: usize) -> &'a [u64] {
        words(self.bytes, self.head.idx_at[i] as usize, bloom_words(self.n()))
    }

    /// Whether the layer may hold a row whose `cols` are `key`: false only when a Bloom filter on exactly those columns
    /// says it holds none.
    pub fn may_hold(&self, cols: &[u8], key: &[i64]) -> bool {
        let s = &self.head.shape;
        (0..s.nidx as usize).filter(|&x| s.index_kind(x) & IX_BLOOM != 0 && s.index_key(x) == cols).all(|x| {
            let (w, m) = crate::bloom::place(crate::bloom::hash(bloom_key(key.iter().copied())), bloom_words(self.n()));
            self.bloom(x)[w] & m == m
        })
    }

    /// Partition `p`'s entry.
    pub fn part(&self, p: i64) -> Option<&'a PartW> {
        let parts = self.parts();
        let p = i32::try_from(p).ok()?;
        parts.binary_search_by_key(&p, |e| e.key).ok().map(|i| &parts[i])
    }

    /// Compares local row `i` with `key` on columns `cols`.
    pub fn cmp_key(&self, i: usize, cols: &[u8], key: &[i64]) -> Ordering {
        for (&c, &k) in cols.iter().zip(key) {
            match self.get(i, c as usize).cmp(&k) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }

    /// The local rows whose `cols` equal `key`, where `cols` is a prefix of the canonical order: a range, found by
    /// binary search. `None` when `cols` is not one.
    pub fn prefix_range(&self, cols: &[u8], key: &[i64]) -> Option<std::ops::Range<usize>> {
        let order = self.head.shape.order();
        if cols.len() > order.len() || order[..cols.len()] != *cols {
            return None;
        }
        let n = self.n();
        let lo = partition_point(n, |i| self.cmp_key(i, cols, key) == Ordering::Less);
        let hi = partition_point(n, |i| self.cmp_key(i, cols, key) != Ordering::Greater);
        Some(lo..hi)
    }

    /// The local rows whose `cols` equal `key` by index `x` (its key starting with `cols`): positions in its order.
    pub fn index_range(&self, x: usize, cols: &[u8], key: &[i64]) -> Option<&'a [u32]> {
        let s = &self.head.shape;
        if x >= s.nidx as usize {
            return None;
        }
        let ikey = s.index_key(x);
        if cols.len() > ikey.len() || ikey[..cols.len()] != *cols {
            return None;
        }
        let kind = s.index_kind(x);
        if kind & IX_BLOOM != 0 {
            return None;
        }
        let perm = self.index(x);
        let n = perm.len();
        let (tags, fences) = (self.tags(x), self.fences(x));
        let cmp = |p: usize| self.cmp_key(perm[p] as usize, cols, key);
        let (lo, hi) = if !tags.is_empty() || !fences.is_empty() {
            // The search written once (`blocks::order::find`): the tags' or fences' span, then prints or bisection.
            // In a hash order, positions are sorted by the lead's tag and then by the key.
            let whole = cols.len() == ikey.len();
            crate::order::find(n, fences, tags, self.prints(x), key, whole, cmp)
        } else {
            let lo = partition_point(n, |p| cmp(p) == Ordering::Less);
            (lo, lo + partition_point(n - lo, |p| cmp(lo + p) != Ordering::Greater))
        };
        Some(&perm[lo..hi])
    }
}

/// The first `i` in `0..n` for which `pred` is false (`pred` true then false).
fn partition_point(n: usize, pred: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if pred(mid) { lo = mid + 1 } else { hi = mid }
    }
    lo
}

/// A trace's state at a layer, read in place through a launch's descriptor table: `bistate::Read`. With a partition
/// key (`restrict`), only that partition's rows: each layer's range of it, from its partition index.
#[derive(Clone, Copy)]
pub struct Chain<'a> {
    table: &'a Table<'a>,
    head: Option<Id>,
    arity: usize,
    part: Option<i64>,
}

impl<'a> Chain<'a> {
    /// The state at layer `head` (`None`: the empty state, of `arity` columns).
    pub fn new(table: &'a Table<'a>, head: Option<Id>, arity: usize) -> Result<Chain<'a>, String> {
        let c = Chain { table, head, arity, part: None };
        let mut id = head;
        while let Some(i) = id {
            let l = c.layer(i)?;
            if l.head.shape.arity() != arity {
                return Err(format!("layer #{i} has {} columns, not {arity}", l.head.shape.arity()));
            }
            id = l.below();
        }
        Ok(c)
    }

    fn layer(&self, id: Id) -> Result<Layer<'a>, String> {
        let b = self.table.block(id).ok_or_else(|| format!("layer #{id} is not reachable from the launch"))?;
        Layer::new(b).map_err(|e| format!("block #{id}: {e}"))
    }

    fn at(&self, id: Id) -> Layer<'a> {
        self.layer(id).expect("a chain's layers were checked when it was made")
    }

    /// Its layers, newest first.
    pub fn layers(&self) -> impl Iterator<Item = Layer<'a>> + '_ {
        let mut id = self.head;
        std::iter::from_fn(move || {
            let l = self.at(id?);
            id = l.below();
            Some(l)
        })
    }

    pub fn head(&self) -> Option<Id> {
        self.head
    }

    pub fn part(&self) -> Option<i64> {
        self.part
    }

    /// The layer holding id `id`, and its local row.
    fn locate(&self, id: u32) -> Option<(Layer<'a>, usize)> {
        self.layers().find(|l| l.start() <= id).filter(|l| id < l.start() + l.head.n).map(|l| (l, (id - l.start()) as usize))
    }

    /// Partition `p`'s rows in layer `l`, local: all of them without a restriction.
    fn span(&self, l: &Layer<'a>) -> std::ops::Range<usize> {
        match self.part {
            None => 0..l.n(),
            Some(p) => l.part(p).map_or(0..0, |e| e.lo as usize..e.hi as usize),
        }
    }

    /// Whether row `id` was killed by a layer at or below this state's head and above `l`.
    fn killed_above(&self, l: &Layer<'a>, id: u32) -> bool {
        self.layers().take_while(|k| k.start() > l.start()).any(|k| k.flips().binary_search(&id).is_ok())
    }
}

impl Read for Chain<'_> {
    type Column<'s>
        = crate::bistate::Cells<'s, Self>
    where
        Self: 's;

    fn arity(&self) -> usize {
        self.arity
    }

    fn end(&self) -> RowId {
        RowId::new(self.head.map_or(0, |h| self.at(h).end()))
    }

    fn live_word(&self, w: WordIndex) -> u64 {
        let first = w.first().get();
        let Some(l) = self.layers().find(|l| l.start() <= first) else { return 0 };
        if first >= l.end() {
            return 0;
        }
        let local = (first - l.start()) as usize;
        let span = self.span(&l);
        let mut live = l.plus()[local / 64] & below(RowId::new(l.head.n), WordIndex::new((local / 64) as u32));
        // Only the partition's rows, when restricted.
        let lo = span.start.saturating_sub(local).min(64);
        let hi = span.end.saturating_sub(local).min(64);
        let mask = if hi <= lo { 0 } else { (if hi == 64 { u64::MAX } else { (1u64 << hi) - 1 }) & !((1u64 << lo) - 1) };
        live &= mask;
        if live == 0 {
            return 0;
        }
        for k in self.layers().take_while(|k| k.start() > l.start()) {
            let f = k.flips();
            let from = f.partition_point(|&x| x < first);
            for &x in f[from..].iter().take_while(|&&x| x < first + 64) {
                live &= !(1u64 << (x - first));
            }
        }
        live
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        let (l, i) = self.locate(id.get()).unwrap_or_else(|| panic!("row {id} is not in the state"));
        l.get(i, c.get())
    }

    fn column(&self, c: ColumnIndex) -> Self::Column<'_> {
        crate::bistate::Cells::new(self, c)
    }

    fn since(&self) -> Option<Since<'_>> {
        let l = self.at(self.head?);
        let flips = match self.part {
            None => l.flips(),
            Some(p) => l.part(p).map_or(&[][..], |e| &l.pflips()[e.flo as usize..e.fhi as usize]),
        };
        Some(Since { end: RowId::new(l.start()), flips })
    }

    fn flips_from(&self, end: RowId) -> Option<Flips<'_>> {
        let mut f = Flips::default();
        for l in self.layers() {
            if l.start() < end.get() {
                return None;
            }
            let run = match self.part {
                None => l.flips(),
                Some(p) => l.part(p).map_or(&[][..], |e| &l.pflips()[e.flo as usize..e.fhi as usize]),
            };
            f.push(run);
            if l.start() == end.get() {
                return Some(f);
            }
        }
        (end.get() == 0).then_some(f)
    }

    fn exact_counts_from(&self, end: RowId) -> Option<(usize, usize)> {
        let l = self.at(self.head?);
        if l.start() != end.get() {
            return None;
        }
        match self.part {
            None => Some((l.head.gone as usize, l.head.added as usize)),
            Some(p) => Some(l.part(p).map_or((0, 0), |e| (e.gone as usize, e.added as usize))),
        }
    }

    fn scan(&self) -> impl Iterator<Item = RowId> + '_ {
        Scan::new(self)
    }

    fn matching<'s>(&'s self, cols: &'s [ColumnIndex], key: &'s [i64]) -> impl Iterator<Item = RowId> + 's {
        let cs: [u8; MAX_COLS] = std::array::from_fn(|i| cols.get(i).map_or(0, |c| c.get() as u8));
        let n = cols.len();
        let k: [i64; MAX_COLS] = std::array::from_fn(|i| key.get(i).copied().unwrap_or(0));
        // The probe's key lives in the iterator; the chain's reads borrow `self`.
        ProbeIter { chain: *self, cs, k, n, inner: None, layer: self.head }
    }

    fn part(&self, p: i64) -> impl Iterator<Item = RowId> + '_ {
        Scan::new(&States::restrict(self, p))
    }

    fn partition(&self) -> Option<ColumnIndex> {
        let l = self.at(self.head?);
        l.head.shape.partition().map(|c| ColumnIndex::new(c as u16))
    }

    fn stats(&self, c: ColumnIndex) -> ColStats {
        let s = self.layers().fold(SketchW::EMPTY, |s, l| s.combine(&l.sketches()[c.get()]));
        ColStats { min: s.min, max: s.max, count: s.count, ams: s.ams }
    }
}

/// A probe of a chain: each layer's matches in turn.
pub struct ProbeIter<'a> {
    chain: Chain<'a>,
    cs: [u8; MAX_COLS],
    k: [i64; MAX_COLS],
    n: usize,
    inner: Option<(Layer<'a>, std::ops::Range<usize>, Option<&'a [u32]>)>,
    layer: Option<Id>,
}

impl Iterator for ProbeIter<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        loop {
            if let Some((l, r, perm)) = &mut self.inner {
                let (cs, ks) = (&self.cs[..self.n], &self.k[..self.n]);
                let span = self.chain.span(l);
                while let Some(j) = r.next() {
                    let i = match perm {
                        Some(p) => p[j] as usize,
                        None => j,
                    };
                    if !span.contains(&i) || !l.is_plus(i) || (perm.is_none() && l.cmp_key(i, cs, ks) != Ordering::Equal) {
                        continue;
                    }
                    let id = l.start() + i as u32;
                    if !self.chain.killed_above(l, id) {
                        return Some(RowId::new(id));
                    }
                }
                self.inner = None;
            }
            let l = self.chain.at(self.layer?);
            self.layer = l.below();
            let (cs, ks) = (&self.cs[..self.n], &self.k[..self.n]);
            if !l.may_hold(cs, ks) {
                continue;
            }
            self.inner = Some(if let Some(r) = l.prefix_range(cs, ks) {
                (l, r, None)
            } else if let Some(p) = (0..l.head.shape.nidx as usize).find_map(|x| l.index_range(x, cs, ks)) {
                (l, 0..p.len(), Some(p))
            } else {
                (l, self.chain.span(&l), None)
            });
        }
    }
}

/// A chain's live rows in canonical order: a merge of its layers' runs (each layer is sorted), as many as it has.
pub struct Scan<'a> {
    chain: Chain<'a>,
    // copy: not block data (a cursor a layer: its handle and its place)
    heads: Vec<(Layer<'a>, u32, u32)>,
}

impl<'a> Scan<'a> {
    fn new(chain: &Chain<'a>) -> Scan<'a> {
        let heads = chain
            .layers()
            .map(|l| {
                let s = chain.span(&l);
                (l, s.start as u32, s.end as u32)
            })
            .collect();
        Scan { chain: *chain, heads }
    }
}

impl Iterator for Scan<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        loop {
            let mut best: Option<usize> = None;
            for k in 0..self.heads.len() {
                let (lk, ik, hi) = self.heads[k];
                if ik >= hi {
                    continue;
                }
                best = match best {
                    Some(b) => {
                        let (lb, ib, _) = self.heads[b];
                        let o = lb.head.shape.order().iter().map(|&c| lk.get(ik as usize, c as usize).cmp(&lb.get(ib as usize, c as usize))).find(|o| o.is_ne());
                        if o == Some(Ordering::Less) { Some(k) } else { Some(b) }
                    }
                    None => Some(k),
                };
            }
            let k = best?;
            let (l, i, _) = self.heads[k];
            self.heads[k].1 += 1;
            let id = l.start() + i;
            if l.is_plus(i as usize) && !self.chain.killed_above(&l, id) {
                return Some(RowId::new(id));
            }
        }
    }
}

impl<'a> States for Chain<'a> {
    fn empty(&self) -> Self {
        Chain { table: self.table, head: None, arity: self.arity, part: self.part }
    }

    fn same(&self, other: &Self) -> bool {
        self.head == other.head && self.part == other.part && self.arity == other.arity
    }

    fn restrict(&self, p: i64) -> Self {
        assert!(self.part.is_none_or(|q| q == p), "a restriction of another partition's");
        Chain { part: Some(p), ..*self }
    }

    fn changed_parts_from(&self, end: RowId, out: &mut dyn FnMut(i64)) -> bool {
        // The layers above `end`: their partition lists merged by key, digests summed.
        let mut runs: Vec<&[PartW]> = Vec::new(); // copy: not block data (a handle a layer's partition index)
        for l in self.layers() {
            if l.start() < end.get() {
                return false;
            }
            runs.push(l.parts());
            if l.start() == end.get() {
                break;
            }
        }
        let n = runs.len();
        let mut at = vec![0usize; n]; // copy: not block data (a cursor a layer)
        loop {
            let key = (0..n).filter_map(|k| runs[k].get(at[k]).map(|e| e.key)).min();
            let Some(key) = key else { return true };
            let key: i32 = key;
            let (mut digest, mut moved) = (0u64, 0u64);
            for k in 0..n {
                if let Some(e) = runs[k].get(at[k]).filter(|e| e.key == key) {
                    digest = digest.wrapping_add(e.digest);
                    moved += u64::from(e.added + e.gone);
                    at[k] += 1;
                }
            }
            // One layer's partition changed when it holds a row of it; across layers, when their digests do not
            // cancel (an add and its undo sum to zero).
            if (n == 1 && moved > 0) || digest != 0 {
                if self.part.is_none_or(|p| p == i64::from(key)) {
                    out(i64::from(key));
                }
            }
        }
    }
}
