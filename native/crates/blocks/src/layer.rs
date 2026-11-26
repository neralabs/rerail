//! A trace's blocks (decision 0686; *0686: Operations* §7 and §10; *Delta-first engine primitives* §11–§12): the
//! run's δ runs, and the layers `consolidate` and `merge` write from them, each layer its own block.
//!
//! **A trace** is one relation's history: a chain of frozen layer blocks, newest first. The host holds only the newest
//! layer's block id. Two states are read (the user, 2026-11-24): the committed chain (before), and the chain with the
//! run's new layer on it (after). The before state stays readable while the next layer is written: that is a new
//! block.
//!
//! **δ runs** (`Runs`, a scratch block): a run's `(row, diff)` pairs as its operations write them, each writer
//! claiming its place with one atomic add (host threads or a GPU's); past the capacity a claim writes nothing and
//! counts what was needed, and the run is redone at that size (shape 12).
//!
//! **A layer** (P§12: "a layer is one `blocks::relation::Relation`: rows added, kills of rows below, its indexes, its
//! partition digests, its statistics"; P§13: `Relation` "Keep"): in its block, a `Relation` whose own rows are the rows
//! the layer added, in the trace's canonical order, and whose `changed` are its kills of rows below; it continues the
//! layer below by that block's id and its descriptor's offset there (`Relation::try_continue_far`), so its ids follow
//! the ones below. Its indexes are the relation's: the row index and a hashed index per `Hash` (`build_indexes`),
//! sorted orders per `Order` (`Relation::add_order`, `blocks::order`'s search), Bloom filters per `Bloom` (in the
//! block). Its statistics are each column's `blocks::sketch` of the state it ends (`Relation::sketches`). Beside it,
//! two small relations: `parts` (each partition it changed: its digest's move and counts) and `gone` (its kills by
//! partition). Its header names the three and is written last: a layer is read only once committed.
//!
//! **Reads** (`Chain`, P§12's `Version`: `relation::View` generalized from a base and one delta to n layers): a
//! trace's state at a layer, read in place through a launch's descriptor table, as `bistate::Read`; its change since
//! another state, `bistate::Bistate`, read in the change's size.

use crate::bistate::{ColStats, ColumnIndex, Flips, Read, RowId, Since, States, WordIndex};
use crate::bloom::{self, Bloom};
use crate::gens::{BODY_AT, Id, Table, set_word, word};
use crate::relation::{Relation, Spec, layer_sketch, leading, row_digest};
use crate::sketch::Sketch;
use std::cell::Cell;
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
    /// A hashed index of the layer's relation: its rows grouped by the key's hash (`Relation`'s buckets).
    Hash(Vec<u16>),
    /// The rows' ids in the key's order (`blocks::order`'s form, given to the relation: `Relation::add_order`), with
    /// fences and prints when asked.
    Order { key: Vec<u16>, fences: bool, prints: bool },
    /// Packed's permutation (`blocks::packed`): an index of the Packed encoding, refused with it.
    Perm(Vec<u16>),
    /// A Bloom filter of the key (`blocks::bloom`), in the layer's block.
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
                IndexSpec::Order { key, fences, prints } => (IX_ORDER | if *fences { IX_FENCES } else { 0 } | if *prints { IX_PRINTS } else { 0 }, key),
                IndexSpec::Perm(_) => {
                    return Err(format!(
                        "index {i}: Perm is packed's permutation (P§12), an index of the Packed encoding, which a run \
                         block's Open layers are not (encoding Packed comes with the kept stores, after L1): an Order \
                         serves an Open layer"
                    ));
                }
                IndexSpec::Hash(key) => (IX_HASH, key),
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

/// An index's kind, as its shape holds it: a sorted order of the layer's relation (its ids sorted by the key) and
/// beside it the leading value of every `order::FENCE`th (fences) and each one's 8-bit print of its whole key
/// (prints), the search `blocks::order::find` reads; a hashed index of the relation (its buckets); a Bloom filter of
/// the key (`blocks::bloom`'s hash and mask, its words a power of two). One format each, the existing ones.
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

/// The one value a Bloom filter hashes for a key: the value itself for one column, else the key's `row_digest`.
pub fn bloom_key(key: impl ExactSizeIterator<Item = i64>) -> i64 {
    if key.len() == 1 {
        let mut k = key;
        k.next().expect("one value")
    } else {
        row_digest(key) as i64
    }
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

/// A splitmix64 step: a shuffle's and a test's random numbers.
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A layer block's header, at `BODY_AT`: where its relations' descriptors lie in the block, and its counts. Written
/// last (`state`): a layer is read only once committed.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LayerHead {
    /// The body's bytes (`gens::body_len`): the header's; the relations lie in the block's chunks past the body.
    pub len: u64,
    pub magic: u32,
    pub state: u32,
    /// The layer (P§12: "a layer is one `blocks::relation::Relation`"): its own rows are the rows it added, in the
    /// trace's canonical order; its `changed` its kills of rows below; it continues the layer below by block id and
    /// offset (`Relation::try_continue_far`); its indexes and statistics are its own.
    pub rows: u64,
    /// `parts(part, digest, added, gone)`: one row a partition the layer changed, sorted on the partition (P§11.2's
    /// partition digests, "a small relation in the batch"). The digest is the move of the partition's: its rows
    /// added's `row_digest`s less its rows gone's.
    pub parts: u64,
    /// `gone(part, id)`: the layer's kills by partition, ascending within each, one chunk (P§11.2's partition index,
    /// of the kills: `restrict` reads a partition's alone).
    pub gone: u64,
    pub added: u32,
    pub ngone: u32,
    pub nparts: u32,
    pub pad: u32,
    /// The sum of its partitions' digests.
    pub digest: u64,
    /// Each Bloom index's filter (`blocks::bloom`, in the block): its descriptor's offset; 0 for another kind.
    pub blooms: [u64; MAX_IX],
    pub shape: ShapeW,
}

unsafe impl crate::Pod for LayerHead {}

impl LayerHead {
    /// A layer's header as its writer lays it out at `BODY_AT`, before its rows: open, its three relations' offsets.
    pub fn open(shape: ShapeW, rows: usize, parts: usize, gone: usize) -> LayerHead {
        LayerHead {
            len: LAYER_BODY as u64,
            magic: LAYER_MAGIC,
            state: OPEN,
            rows: rows as u64,
            parts: parts as u64,
            gone: gone as u64,
            added: 0,
            ngone: 0,
            nparts: 0,
            pad: 0,
            digest: 0,
            blooms: [0; MAX_IX],
            shape,
        }
    }

    /// The header of a layer being written, at `BODY_AT` of its block's bytes.
    pub fn of_open(bytes: &[u8]) -> LayerHead {
        let h: LayerHead = word(bytes, BODY_AT);
        assert!(h.magic == LAYER_MAGIC && h.state == OPEN, "a layer being written");
        h
    }

    /// Writes the header committed, whole, at `BODY_AT` of its block's bytes: the layer's last write.
    pub fn commit(mut self, bytes: &mut [u8]) {
        self.state = COMMITTED;
        set_word(bytes, BODY_AT, self);
    }
}

/// The body bytes a layer block's header takes.
pub const LAYER_BODY: usize = size_of::<LayerHead>();

/// The bytes a layer block keeps room for past its body, for `n` rows of `shape`: its relations' chunks (each a power
/// of two, rows growing by doubling), their indexes and orders, its Bloom filters and its sketches.
pub fn layer_room(shape: &ShapeW, n: usize) -> usize {
    let a = shape.arity();
    let width: usize = (0..a).map(|c| shape.width(c)).sum();
    (1 << 20) + n * (4 * width + 32 * (MAX_IX + 2) + 4 * 24) + a * (64 << 10)
}

/// The `Spec::min_log` that holds `n` rows in one chunk: the first group's quarter (`rows::chunk_of`).
fn one_chunk(n: usize) -> u32 {
    (n.max(1).next_power_of_two().trailing_zeros() + 2).max(crate::rows::MIN_LOG)
}

/// The layer relation's spec: the shape's columns at their widths, sorted on its canonical order's first column (the
/// partition, when it has one; its rows written in the whole order, which the sort keeps), a hashed index for each
/// `Hash` index (P§11.2), and its row index (every column: the next layer finds a row's live copy by it). Room for `n`
/// rows in one chunk.
pub fn rows_spec(shape: &ShapeW, n: usize) -> Spec {
    let a = shape.arity();
    let widths = (0..a).map(|c| col_width(if shape.width(c) == 8 { ColType::I64 } else { ColType::I32 })).collect();
    let mut indexes: Vec<Vec<u16>> = (0..shape.nidx as usize).filter(|&x| shape.index_kind(x) & IX_HASH != 0).map(|x| shape.index_key(x).iter().map(|&c| u16::from(c)).collect()).collect();
    let every: Vec<u16> = (0..a as u16).collect();
    if a <= crate::relation::MAX_KEY && !indexes.contains(&every) {
        indexes.push(every);
    }
    Spec { widths, sort: Some(u16::from(shape.order()[0])), indexes, min_log: one_chunk(n) }
}

/// A column type's width in a relation's spec: a signed 32-bit column's `rows::I32`.
fn col_width(t: ColType) -> u32 {
    match t {
        ColType::I32 => crate::rows::I32,
        ColType::I64 => 8,
    }
}

/// `parts`' spec: the partition (signed 32 bits), its digest's move, rows added, rows gone; sorted on the partition.
pub fn parts_spec(n: usize) -> Spec {
    Spec { widths: vec![crate::rows::I32, 8, 4, 4], sort: Some(0), indexes: Vec::new(), min_log: one_chunk(n) }
}

/// `gone`'s spec: the partition, the id killed; sorted on the partition, in one chunk (a partition's ids are a slice).
pub fn gone_spec(n: usize) -> Spec {
    Spec { widths: vec![crate::rows::I32, 4], sort: Some(0), indexes: Vec::new(), min_log: one_chunk(n) }
}

/// A committed layer, read in place: its block's bytes, its header, and its three relations.
#[derive(Clone, Copy)]
pub struct Layer<'a> {
    pub bytes: &'a [u8],
    pub head: LayerHead,
    pub rows: &'a Relation,
    pub parts: &'a Relation,
    pub gone: &'a Relation,
}

impl<'a> Layer<'a> {
    /// The layer at `BODY_AT` of a block's bytes, committed.
    pub fn new(bytes: &'a [u8]) -> Result<Layer<'a>, String> {
        if bytes.len() < BODY_AT + LAYER_BODY {
            return Err("not a layer block".into());
        }
        let head: LayerHead = word(bytes, BODY_AT);
        if head.magic != LAYER_MAGIC {
            return Err("not a layer block".into());
        }
        if head.state != COMMITTED {
            return Err("a layer read before it was committed".into());
        }
        let rel = |at: u64, what: &str| Relation::at(bytes, at as usize).filter(|r| r.is_frozen()).ok_or_else(|| format!("the layer's {what}, frozen"));
        Ok(Layer { bytes, head, rows: rel(head.rows, "rows")?, parts: rel(head.parts, "parts")?, gone: rel(head.gone, "gone")? })
    }

    /// Its first own row's id: every id the layer below reads.
    pub fn start(&self) -> u32 {
        self.rows.base_rows
    }

    pub fn end(&self) -> u32 {
        self.rows.base_rows + self.rows.rows.len()
    }

    /// Its own rows.
    pub fn n(&self) -> usize {
        self.rows.rows.len() as usize
    }

    /// The layer below: its block id, and its relation's offset there (the reference, `Relation::far`).
    pub fn below(&self) -> Option<(Id, usize)> {
        self.rows.far()
    }

    /// Own row `id`'s value in column `c`.
    #[inline]
    pub fn get(&self, id: u32, c: usize) -> i64 {
        self.rows.own(id, c)
    }

    /// Its kills: rows of the layers below, ascending.
    pub fn kills(&self) -> &'a [u32] {
        self.rows.changed.as_slice()
    }

    /// Partition `p`'s own rows: a range of ids (its rows lie sorted on the partition). Every own row when the
    /// trace has no partition.
    pub fn part_rows(&self, p: i64) -> std::ops::Range<u32> {
        let Some(c) = self.head.shape.partition() else { return self.start()..self.end() };
        let r = leading(self.rows, &[c as u16], &[p]);
        self.start() + r.start..self.start() + r.end
    }

    /// Partition `p`'s row of `parts`: its digest's move, rows added and rows gone.
    pub fn part_entry(&self, p: i64) -> Option<(u64, u32, u32)> {
        let r = leading(self.parts, &[0], &[p]);
        (!r.is_empty()).then(|| (self.parts.own(r.start, 1) as u64, self.parts.own(r.start, 2) as u32, self.parts.own(r.start, 3) as u32))
    }

    /// Partition `p`'s kills, ascending: a slice of `gone`'s id column.
    pub fn part_kills(&self, p: i64) -> &'a [u32] {
        let r = leading(self.gone, &[0], &[p]);
        if r.is_empty() {
            return &[];
        }
        let ids = self.gone.rows.col_chunks_u32(1).next().expect("a layer's kills by partition, in one chunk");
        assert!(ids.len() >= r.end as usize, "a layer's kills by partition, in one chunk");
        &ids[r.start as usize..r.end as usize]
    }

    /// Compares own row `id` with `key` on columns `cols`.
    pub fn cmp_key(&self, id: u32, cols: &[u16], key: &[i64]) -> Ordering {
        cols.iter().zip(key).map(|(&c, &k)| self.get(id, c as usize).cmp(&k)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
    }

    /// The own rows whose `cols` equal `key`, where `cols` is a prefix of the canonical order: a range of ids, found
    /// by binary search (its rows lie in that order). `None` when `cols` is not one.
    pub fn prefix_range(&self, cols: &[u16], key: &[i64]) -> Option<std::ops::Range<u32>> {
        let order = self.head.shape.order();
        if cols.is_empty() || cols.len() > order.len() || order[..cols.len()].iter().zip(cols).any(|(&o, &c)| u16::from(o) != c) {
            return None;
        }
        let (s, n) = (self.start(), self.n());
        let lo = partition_point(n, |i| self.cmp_key(s + i as u32, cols, key) == Ordering::Less);
        let hi = lo + partition_point(n - lo, |i| self.cmp_key(s + (lo + i) as u32, cols, key) != Ordering::Greater);
        Some(s + lo as u32..s + hi as u32)
    }

    /// The own rows whose `cols` equal `key` by a sorted order whose key starts with `cols` (`Relation::add_order`):
    /// its ids, found by `blocks::order::find` (fences, prints, then bisection). `None` without such an order.
    pub fn order_range(&self, cols: &[u16], key: &[i64]) -> Option<&'a [u32]> {
        let rows = self.rows;
        let x = rows.orders().find(|&x| {
            let ix = rows.index_at(x);
            let k = &ix.key[..ix.ncols as usize];
            !cols.is_empty() && cols.len() <= k.len() && k[..cols.len()] == *cols
        })?;
        let ix = rows.index_at(x);
        let ids = ix.rows.as_slice();
        let cmp = |p: usize| self.cmp_key(ids[p], cols, key);
        let whole = cols.len() == ix.ncols as usize;
        let (lo, hi) = crate::order::find(ids.len(), ix.fences.as_slice(), ix.tags.as_slice(), ix.prints.as_slice(), key, whole, cmp);
        Some(&ids[lo..hi])
    }

    /// Index `x`'s Bloom filter, for a Bloom index.
    pub fn bloom(&self, x: usize) -> Option<&'a Bloom> {
        let at = self.head.blooms[x];
        (at != 0).then(|| Bloom::at(self.bytes, at as usize).expect("a layer's Bloom filter"))
    }

    /// Whether a key on `cols` may be among its own rows: false only when a Bloom index on exactly `cols` says no.
    pub fn may_hold(&self, cols: &[u16], key: &[i64]) -> bool {
        let s = &self.head.shape;
        (0..s.nidx as usize).all(|x| {
            let k = s.index_key(x);
            s.index_kind(x) & IX_BLOOM == 0 || k.len() != cols.len() || k.iter().zip(cols).any(|(&a, &b)| u16::from(a) != b) || self.bloom(x).is_none_or(|b| b.may_hold_hashed(bloom::hash(bloom_key(key.iter().copied()))))
        })
    }

    /// Column `c`'s sketch of the state this layer ends (`Relation::sketches`), read back from its kept form.
    pub fn sketch(&self, c: usize) -> Option<Sketch> {
        let b = layer_sketch(self.rows, c)?;
        Sketch::decode(&|i| b[i], 0, b.len(), sketch_room())
    }
}

/// Where a sketch read back from a layer keeps its arrays while it is read: the heap's (a reader's own, for one read).
pub fn sketch_room() -> std::sync::Arc<dyn crate::maps::Room> {
    std::sync::Arc::new(rerail_vector::HeapRoom)
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

/// A trace's state at a layer (P§12's `Version`: `blocks::relation::View` generalized to n layers), read in place
/// through a launch's descriptor table: `bistate::Read`. A row is live when it is an own row of a layer at or below the
/// state's and no layer above that one, up to the state's, kills it. With a partition key (`restrict`), only that
/// partition's rows: each layer's range of it, and its kills of it. Its reads can be counted (`counted`): every own
/// row and every liveness word they touch.
#[derive(Clone, Copy)]
pub struct Chain<'a> {
    table: &'a Table<'a>,
    head: Option<Id>,
    arity: usize,
    part: Option<i64>,
    reads: Option<&'a Cell<u64>>,
}

impl<'a> Chain<'a> {
    /// The state at layer `head` (`None`: the empty state, of `arity` columns). Each layer is checked once: reached,
    /// committed, of `arity` columns, and where the layer above refers to it.
    pub fn new(table: &'a Table<'a>, head: Option<Id>, arity: usize) -> Result<Chain<'a>, String> {
        let c = Chain { table, head, arity, part: None, reads: None };
        let mut at = head.map(|h| (h, None));
        while let Some((id, off)) = at {
            let l = c.layer(id)?;
            if off.is_some_and(|o| o != l.head.rows as usize) {
                return Err(format!("layer #{id}'s relation is not where the layer above refers"));
            }
            if l.head.shape.arity() != arity || l.rows.arity as usize != arity {
                return Err(format!("layer #{id} has {} columns, not {arity}", l.head.shape.arity()));
            }
            // Its reference to the layer below (O§2: block id and offset, the layer's header) is its relation's.
            if let Some((b, _)) = l.below()
                && !crate::gens::refs(l.bytes).contains(&crate::gens::Ref { id: b, at: BODY_AT as u32 })
            {
                return Err(format!("layer #{id} continues #{b} and does not refer to it"));
            }
            at = l.below().map(|(b, o)| (b, Some(o)));
        }
        Ok(c)
    }

    /// The same state, its reads counted into `reads`.
    pub fn counted(self, reads: &'a Cell<u64>) -> Chain<'a> {
        Chain { reads: Some(reads), ..self }
    }

    fn touch(&self, n: usize) {
        if let Some(r) = self.reads {
            r.set(r.get() + n as u64);
        }
    }

    fn layer(&self, id: Id) -> Result<Layer<'a>, String> {
        let b = self.table.block(id).ok_or_else(|| format!("layer #{id} is not reachable from the launch"))?;
        Layer::new(b).map_err(|e| format!("block #{id}: {e}"))
    }

    fn at(&self, id: Id) -> Layer<'a> {
        self.layer(id).expect("a chain's layers were checked when it was made")
    }

    /// Its layers, newest first, with their block ids.
    pub fn layers(&self) -> impl Iterator<Item = (Id, Layer<'a>)> + '_ {
        let mut id = self.head;
        std::iter::from_fn(move || {
            let i = id?;
            let l = self.at(i);
            id = l.below().map(|(b, _)| b);
            Some((i, l))
        })
    }

    pub fn head(&self) -> Option<Id> {
        self.head
    }

    pub fn part(&self) -> Option<i64> {
        self.part
    }

    /// The layers above the state `before` ends (all of them for `None`), newest first: `None` when `before` is not
    /// one of this state's.
    fn above(&self, before: Option<&Chain<'a>>) -> Option<Vec<Layer<'a>>> {
        let stop = before.and_then(|b| b.head);
        // copy: not block data (a handle a layer, as many as lie above)
        let mut out = Vec::new();
        for (id, l) in self.layers() {
            if Some(id) == stop {
                return Some(out);
            }
            out.push(l);
        }
        stop.is_none().then_some(out)
    }

    /// The layer holding id `id`.
    fn locate(&self, id: u32) -> Option<Layer<'a>> {
        self.layers().map(|(_, l)| l).find(|l| l.start() <= id && id < l.end())
    }

    /// Partition `p`'s own rows of layer `l`: all of them without a restriction.
    fn span(&self, l: &Layer<'a>) -> std::ops::Range<u32> {
        match self.part {
            None => l.start()..l.end(),
            Some(p) => l.part_rows(p),
        }
    }

    /// Layer `l`'s kills of this state's rows: its partition's alone when restricted.
    fn kills_of(&self, l: &Layer<'a>) -> &'a [u32] {
        match self.part {
            None => l.kills(),
            Some(p) => l.part_kills(p),
        }
    }

    /// Whether row `id`, of layer `l`, is killed by a layer above `l`, up to this state's.
    fn killed_above(&self, l: &Layer<'a>, id: u32) -> bool {
        self.layers().map(|(_, k)| k).take_while(|k| !std::ptr::eq(k.rows, l.rows)).any(|k| k.kills().binary_search(&id).is_ok())
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
        self.touch(1);
        let (first, end) = (w.first().get(), self.end().get());
        let last = first.saturating_add(64).min(end);
        if first >= last {
            return 0;
        }
        // Each layer's own rows in the word (its partition's, when restricted), less every kill of one of them by a
        // layer above it: a layer kills only rows below its own, so a kill names a live row of the state it was made
        // over, and a row killed is never live again (a row added again is a new row). The kills are taken after
        // every layer's rows are in.
        let (mut live, mut killed) = (0u64, 0u64);
        for (_, l) in self.layers() {
            if l.end() <= first {
                break;
            }
            let s = self.span(&l);
            let (lo, hi) = (s.start.max(first), s.end.min(last));
            if lo < hi {
                let bits = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << (lo - first) };
                live |= bits;
            }
            let k = l.kills();
            let from = k.partition_point(|&x| x < first);
            for &x in k[from..].iter().take_while(|&&x| x < last) {
                killed |= 1u64 << (x - first);
            }
        }
        live & !killed
    }

    fn get(&self, id: RowId, c: ColumnIndex) -> i64 {
        let l = self.locate(id.get()).unwrap_or_else(|| panic!("row {id} is not in the state"));
        l.get(id.get(), c.get())
    }

    fn column(&self, c: ColumnIndex) -> Self::Column<'_> {
        crate::bistate::Cells::new(self, c)
    }

    fn since(&self) -> Option<Since<'_>> {
        let l = self.at(self.head?);
        Some(Since { end: RowId::new(l.start()), flips: self.kills_of(&l) })
    }

    fn flips_since(&self, before: Option<&Self>) -> Option<Flips<'_>> {
        let mut f = Flips::default();
        for l in self.above(before)? {
            f.push(self.kills_of(&l));
        }
        Some(f)
    }

    fn new_ranges(&self, before: Option<&Self>) -> Option<Vec<(RowId, RowId)>> {
        let mut r: Vec<(RowId, RowId)> = self.above(before)?.iter().map(|l| self.span(l)).filter(|s| !s.is_empty()).map(|s| (RowId::new(s.start), RowId::new(s.end))).collect(); // copy: not block data (a range a layer)
        r.reverse();
        Some(r)
    }

    fn exact_counts_since(&self, before: Option<&Self>) -> Option<(usize, usize)> {
        let above = self.above(before)?;
        let [l] = above.as_slice() else { return None };
        match self.part {
            None => Some((l.head.ngone as usize, l.head.added as usize)),
            Some(p) => Some(l.part_entry(p).map_or((0, 0), |(_, added, gone)| (gone as usize, added as usize))),
        }
    }

    fn scan(&self) -> impl Iterator<Item = RowId> + '_ {
        Scan::new(self)
    }

    fn matching<'s>(&'s self, cols: &'s [ColumnIndex], key: &'s [i64]) -> impl Iterator<Item = RowId> + 's {
        let cs: [u16; MAX_COLS] = std::array::from_fn(|i| cols.get(i).map_or(0, |c| c.get() as u16));
        let n = cols.len();
        let k: [i64; MAX_COLS] = std::array::from_fn(|i| key.get(i).copied().unwrap_or(0));
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
        // A partition's are read from its rows; the whole state's from its layers: the bounds of every row added
        // (an estimate: rows since gone included), and the state's sketch, which its last layer keeps.
        if self.part.is_some() || self.head.is_none() {
            return crate::bistate::stats_of(self, c);
        }
        let (mut min, mut max) = (i64::MAX, i64::MIN);
        for (_, l) in self.layers() {
            if let Some(s) = l.rows.stats.as_slice().get(c.get()).filter(|_| l.n() > 0) {
                min = min.min(s.min);
                max = max.max(s.max);
            }
        }
        let reading = self.at(self.head.expect("a state")).sketch(c.get()).map(|s| s.reading()).unwrap_or_default();
        ColStats { min, max, rows: reading.rows, ndv: reading.ndv, sq: reading.sq }
    }
}

/// A probe of a chain: each layer's matches in turn, newest first, by the layer's indexes: a range of the canonical
/// order when the key is a prefix of it, a sorted order (`Order`), a hashed index (`Hash`, or the row index), else
/// its rows; a Bloom index skips a layer that cannot hold the key.
pub struct ProbeIter<'a> {
    chain: Chain<'a>,
    cs: [u16; MAX_COLS],
    k: [i64; MAX_COLS],
    n: usize,
    inner: Option<(Layer<'a>, Candidates<'a>)>,
    layer: Option<Id>,
}

/// A layer's candidates for a probe: a range of ids, or a list of them.
enum Candidates<'a> {
    Range(std::ops::Range<u32>),
    Listed(std::slice::Iter<'a, u32>),
}

impl Iterator for Candidates<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        match self {
            Candidates::Range(r) => r.next(),
            Candidates::Listed(s) => s.next().copied(),
        }
    }
}

impl Iterator for ProbeIter<'_> {
    type Item = RowId;

    fn next(&mut self) -> Option<RowId> {
        loop {
            if let Some((l, ids)) = &mut self.inner {
                let (cs, ks) = (&self.cs[..self.n], &self.k[..self.n]);
                let span = self.chain.span(l);
                for id in ids.by_ref() {
                    self.chain.touch(1);
                    if span.contains(&id) && l.cmp_key(id, cs, ks) == Ordering::Equal && !self.chain.killed_above(l, id) {
                        return Some(RowId::new(id));
                    }
                }
                self.inner = None;
            }
            let l = self.chain.at(self.layer?);
            self.layer = l.below().map(|(b, _)| b);
            let (cs, ks) = (&self.cs[..self.n], &self.k[..self.n]);
            if !l.may_hold(cs, ks) {
                continue;
            }
            let ids = if let Some(r) = l.prefix_range(cs, ks) {
                Candidates::Range(r)
            } else if let Some(p) = l.order_range(cs, ks) {
                Candidates::Listed(p.iter())
            } else if let Some(x) = l.rows.find_index(cs, l.rows.rows.len()) {
                Candidates::Listed(l.rows.group(x, crate::key_hash(ks)).iter())
            } else {
                Candidates::Range(self.chain.span(&l))
            };
            self.inner = Some((l, ids));
        }
    }
}

/// A chain's live rows in canonical order: a merge of its layers' runs (each layer lies in the order), as many as it
/// has.
pub struct Scan<'a> {
    chain: Chain<'a>,
    // copy: not block data (a cursor a layer: its handle and its place)
    heads: Vec<(Layer<'a>, u32, u32)>,
}

impl<'a> Scan<'a> {
    fn new(chain: &Chain<'a>) -> Scan<'a> {
        let heads = chain
            .layers()
            .map(|(_, l)| {
                let s = chain.span(&l);
                (l, s.start, s.end)
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
                        let o = lb.head.shape.order().iter().map(|&c| lk.get(ik, c as usize).cmp(&lb.get(ib, c as usize))).find(|o| o.is_ne());
                        if o == Some(Ordering::Less) { Some(k) } else { Some(b) }
                    }
                    None => Some(k),
                };
            }
            let k = best?;
            let (l, id, _) = self.heads[k];
            self.heads[k].1 += 1;
            self.chain.touch(1);
            if !self.chain.killed_above(&l, id) {
                return Some(RowId::new(id));
            }
        }
    }
}

impl<'a> States for Chain<'a> {
    fn empty(&self) -> Self {
        Chain { head: None, ..*self }
    }

    fn same(&self, other: &Self) -> bool {
        self.head == other.head && self.part == other.part && self.arity == other.arity
    }

    fn restrict(&self, p: i64) -> Self {
        assert!(self.part.is_none_or(|q| q == p), "a restriction of another partition's");
        Chain { part: Some(p), ..*self }
    }

    fn changed_parts_since(&self, before: Option<&Self>, out: &mut dyn FnMut(i64)) -> bool {
        // The layers above `before`: their `parts` merged on the partition, digests summed.
        let Some(above) = self.above(before) else { return false };
        let n = above.len();
        let mut at: Vec<u32> = vec![0; n]; // copy: not block data (a cursor a layer)
        loop {
            let key = (0..n).filter(|&k| at[k] < above[k].parts.rows.len()).map(|k| above[k].parts.own(at[k], 0)).min();
            let Some(key) = key else { return true };
            let (mut digest, mut moved) = (0u64, 0u64);
            for k in 0..n {
                let p = above[k].parts;
                if at[k] < p.rows.len() && p.own(at[k], 0) == key {
                    digest = digest.wrapping_add(p.own(at[k], 1) as u64);
                    moved += (p.own(at[k], 2) + p.own(at[k], 3)) as u64;
                    at[k] += 1;
                }
            }
            // One layer's partition changed when it holds a row of it; across layers, when their digests do not
            // cancel (an add and its undo sum to zero).
            if ((n == 1 && moved > 0) || digest != 0) && self.part.is_none_or(|p| p == key) {
                out(key);
            }
        }
    }
}
