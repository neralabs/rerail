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
//! the layer added, in the trace's canonical order (its relation records no change: its kills of rows below are
//! `gone`'s); it continues the layer below by that block's id and its descriptor's offset there
//! (`Relation::try_continue_far`), so its ids follow the ones below. Its indexes are the relation's: the row index and a hashed index per `Hash` (`build_indexes`),
//! sorted orders per `Order` (`Relation::add_order`, `blocks::order`'s search), Bloom filters per `Bloom` (in the
//! block). Its statistics are each column's `blocks::sketch` of the state it ends (`Relation::sketches`). Beside it,
//! two small relations: `parts` (each partition it changed: its digest's move and counts) and `gone` (its kills by
//! partition). Its header names the three and is written last: a layer is read only once committed.
//!
//! **Reads** (`relation::View::chain`, P§12's `Version`: `relation::View` generalized from a base and one delta to n
//! layers, one definition): a trace's state at a layer, read in place through a launch's descriptor table, as
//! `bistate::Read`; its change since another state, `bistate::Bistate`, read in the change's size.

use crate::bistate::{ColStats, ColumnIndex, Flips, Since, WordIndex};
// A trace's state and its reads in place: the floor's (`rerail_delta_core::trace`).
pub use rerail_delta_core::trace::{ChainError, ChainFlips, MAX_BELOW, MAX_LAYERS, ProbeIter, Ranges, Scan, State};
use crate::gens::{BODY_AT, Id, Table, word};
use crate::relation::{Relation, View};

// The shape's words, its bounds and the δ runs block: the floor's one definition (`rerail_delta_core::runs`).
pub use rerail_delta_core::runs::{
    IndexW, MAX_COLS, MAX_IX, MAX_KEY, NONE, Runs, RunsHead, ShapeW, init_runs, runs_bytes, runs_claim_in, runs_set_in,
};
// A layer's format and its reads: the floor's one definition (`rerail_delta_core::layer`).
pub use rerail_delta_core::layer::{
    COMMITTED, ENC_DICT, ENC_OPEN, ENC_PACKED, ENC_RANGED, IX_BLOOM, IX_FENCES, IX_HASH, IX_ORDER, IX_PERM, IX_PRINTS, LAYER_BODY, LAYER_MAGIC, Layer,
    LayerHead, OPEN, OwnRows, RETAIN_RUN, bloom_key, partition_point,
};
use rerail_delta_core::runs::{CLAIMED_AT, NEEDED_AT, RUNS_MAGIC};
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
        // A recursive relation's level (0364): "a level per row, as a column of its relations", its last, 32 bits, and
        // last in the canonical order, so a key's rows lie together and its probe is a prefix. A rewrite is a kill and
        // an add (0364 §4); consolidate refuses a key live at two levels.
        if self.levels {
            let l = a - 1;
            if a < 2 || self.columns[l] != ColType::I32 || self.order[l] as usize != l || self.partition == Some(l as u16) {
                return Err(format!(
                    "a level column (0364) is the relation's last, 32 bits, last in the canonical order, and not its \
                     partition: column {l} of {:?}, order {:?}",
                    self.columns, self.order
                ));
            }
        }
        // Its layers' own rows: open (the relation's rows chunks), or written once in the fewest bits, read in place:
        // `packed::Packed` (a column an `IntCol`, the smallest of `col`'s), `Dict` (the same, `DICT` taken where
        // smallest: `col::choose_for_reads`), or `ranged::Columns` (a column cut in ranges, each its own encoding).
        let encoding = match self.encoding {
            Encoding::Open => ENC_OPEN,
            Encoding::Packed => ENC_PACKED,
            Encoding::Ranged => ENC_RANGED,
            Encoding::Dict => ENC_DICT,
        };
        // Where its layers' blocks lie: the run's memory, a kept file (`blocks::kept`'s place: the traces'
        // directory, mapped where it lies), or a pinned slab (the device's, read in place).
        let placement = match self.placement {
            Placement::RunBlock => 0,
            Placement::KeptFile => 1,
            Placement::PinnedSlab => 2,
        };
        let retention = match self.retention {
            Retention::Run => RETAIN_RUN,
            Retention::Cache => {
                return Err("a cache is not a trace's layers: it is one state, blocks mutated in place, never frozen \
                            (P§11.3): an operator's cache (delta_exec::cache)"
                    .into());
            }
            Retention::History => {
                return Err("retention History keeps named frontiers, and a trace holds two states: \"no Frontier, at, \
                            between, fork or pins\" (the user's answer, 2026-11-25, in the rebuild plan)"
                    .into());
            }
        };
        if self.indexes.len() > MAX_IX {
            return Err(format!("at most {MAX_IX} indexes beside the canonical order"));
        }
        let mut w = ShapeW {
            arity: a as u32,
            partition: self.partition.map_or(NONE, u32::from),
            levels: u32::from(self.levels),
            encoding,
            placement,
            retention,
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
                IndexSpec::Perm(key) => {
                    // Packed's permutation (`packed::PIndex`): an index of a packed layer, on one column its order
                    // does not lead.
                    if !matches!(self.encoding, Encoding::Packed | Encoding::Dict) {
                        return Err(format!("index {i}: Perm is packed's permutation (P§12), an index of a Packed or Dict layer"));
                    }
                    if key.len() != 1 || self.order.first() == key.first() {
                        return Err(format!("index {i}: a permutation is on one column the order does not lead, not {key:?}"));
                    }
                    (IX_PERM, key)
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

/// A Bloom filter's words for `n` keys: about ten bits a key, a power of two.
pub fn bloom_words(n: usize) -> usize {
    (n * 10).div_ceil(64).max(1).next_power_of_two()
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

// ---- layers -------------------------------------------------------------------------------------------------------

/// A splitmix64 step: a shuffle's and a test's random numbers.
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

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

/// A layer relation's spec, fixed (no heap: the phases that write a layer hold it on their stack, `layer_write`): its
/// columns' widths, its sort column, its hashed indexes' keys, and the chunk size that holds its rows.
#[derive(Clone, Copy, Debug)]
pub struct LayerSpec {
    widths: [u32; MAX_COLS],
    arity: usize,
    sort: Option<u16>,
    keys: [[u16; MAX_KEY]; MAX_IX + 1],
    key_len: [usize; MAX_IX + 1],
    nkeys: usize,
    min_log: u32,
}

impl LayerSpec {
    fn of(widths: &[u32], sort: Option<u16>, n: usize) -> LayerSpec {
        let mut w = [0; MAX_COLS];
        w[..widths.len()].copy_from_slice(widths);
        LayerSpec { widths: w, arity: widths.len(), sort, keys: [[0; MAX_KEY]; MAX_IX + 1], key_len: [0; MAX_IX + 1], nkeys: 0, min_log: one_chunk(n) }
    }

    fn push_key(&mut self, key: impl Iterator<Item = u16>) {
        let k = self.nkeys;
        for (j, c) in key.enumerate() {
            self.keys[k][j] = c;
            self.key_len[k] = j + 1;
        }
        self.nkeys += 1;
    }

    /// The layer's rows: the shape's columns at their widths, sorted on its canonical order's first column (the
    /// partition, when it has one; its rows written in the whole order, which the sort keeps), a hashed index for each
    /// `Hash` index (P§11.2), and its row index (every column: the next layer finds a row's live copy by it). Room for
    /// `n` rows in one chunk.
    pub fn rows(shape: &ShapeW, n: usize) -> LayerSpec {
        let a = shape.arity();
        let widths: [u32; MAX_COLS] = core::array::from_fn(|c| if c < a { col_width(if shape.width(c) == 8 { ColType::I64 } else { ColType::I32 }) } else { 0 });
        let mut s = LayerSpec::of(&widths[..a], Some(u16::from(shape.order()[0])), n);
        for x in (0..shape.nidx as usize).filter(|&x| shape.index_kind(x) & IX_HASH != 0) {
            s.push_key(shape.index_key(x).iter().map(|&c| u16::from(c)));
        }
        let every = |k: &[u16]| k.len() == a && k.iter().enumerate().all(|(j, &c)| c as usize == j);
        if a <= MAX_KEY && !(0..s.nkeys).any(|k| every(s.key(k))) {
            s.push_key(0..a as u16);
        }
        s
    }

    /// `parts`: the partition (signed 32 bits), its digest's move, rows added, rows gone, its first own row's place
    /// among the layer's, its first kill's place in `gone`; sorted on the partition.
    pub fn parts(n: usize) -> LayerSpec {
        LayerSpec::of(&[crate::rows::I32, 8, 4, 4, 4, 4], Some(0), n)
    }

    /// `gone`: the partition, the id killed; sorted on the partition, in one chunk (a partition's ids are a slice).
    pub fn gone(n: usize) -> LayerSpec {
        LayerSpec::of(&[crate::rows::I32, 4], Some(0), n)
    }

    /// Its columns, as a relation is made with them.
    pub fn cols(&self) -> crate::relation::Cols<'_> {
        crate::relation::Cols { widths: &self.widths[..self.arity], sort: self.sort, min_log: self.min_log }
    }

    pub fn sort(&self) -> Option<u16> {
        self.sort
    }

    /// Its hashed indexes' count, and index `x`'s key.
    pub fn nkeys(&self) -> usize {
        self.nkeys
    }

    pub fn key(&self, x: usize) -> &[u16] {
        &self.keys[x][..self.key_len[x]]
    }
}

/// A column type's width in a relation's spec: a signed 32-bit column's `rows::I32`.
fn col_width(t: ColType) -> u32 {
    match t {
        ColType::I32 => crate::rows::I32,
        ColType::I64 => 8,
    }
}

/// A layer being written's own rows in its block: blocks' own, over the floor's `OwnRows`.
pub trait OwnRowsIn<'a> {
    fn in_block(block: &'a crate::block::Block, head: &LayerHead, rows: &'a Relation) -> OwnRows<'a>;
}

impl<'a> OwnRowsIn<'a> for OwnRows<'a> {
    /// A layer being written's own rows, in its block (its writer's phases after the rows are written or encoded).
    fn in_block(block: &'a crate::block::Block, head: &LayerHead, rows: &'a Relation) -> OwnRows<'a> {
        let start = rows.base_rows;
        let at = head.enc as usize;
        if at == 0 || head.shape.encoding == ENC_OPEN {
            return OwnRows::Open(rows);
        }
        assert!(at.is_multiple_of(8) && block.holds(block.ptr::<u8>(at)), "a layer's encoded rows lie in its block");
        // SAFETY: laid out at `at` of the block by `layer_write::encode_open`; read after its columns are written.
        unsafe {
            match head.shape.encoding {
                ENC_RANGED => OwnRows::Ranged { cols: &*block.ptr::<crate::ranged::Columns>(at), start },
                _ => OwnRows::Packed { p: &*block.ptr::<crate::packed::Packed>(at), start },
            }
        }
    }
}

/// A layer's rows as a packer reads them (`packed::RowsIn`): row `r` is own row `start + r`.
impl crate::packed::RowsIn for OwnRows<'_> {
    fn n(&self) -> usize {
        OwnRows::n(self) as usize
    }
    fn arity(&self) -> usize {
        match *self {
            OwnRows::Open(r) => r.arity as usize,
            OwnRows::Packed { p, .. } => p.arity as usize,
            OwnRows::Ranged { cols, .. } => cols.arity as usize,
        }
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        OwnRows::get(self, self.start() + r as u32, c)
    }
}

/// A trace's state (`relation::View::chain`; P§12's `Version`, "its View generalized from a base and one delta to n
/// layers"): the floor's `trace::State`, which reads its chain of layers in place through the launch's descriptor
/// table, held by the view; its reads are the floor's.
impl<'a> View<'a> {
    /// The state at layer `head` (`None`: the empty state, of `arity` columns), its layers reached through `table`,
    /// each checked once (`trace::State::chain`).
    pub fn chain(table: &'a Table<'a>, head: Option<Id>, arity: usize) -> Result<View<'a>, String> {
        let s = State::chain(table, head, arity).map_err(|e| e.to_string())?;
        let top = head.map(|h| s.layer(h).rows);
        Ok(View { base: None, top, arity: arity as u32, chain: Some(s) })
    }

    /// Its state, a trace's.
    pub fn state(&self) -> State<'a> {
        self.chain.expect("a trace's state")
    }

    /// Layer `id`, checked when the state was made.
    pub fn layer(&self, id: Id) -> Layer<'a> {
        self.state().layer(id)
    }

    /// Its layers, newest first, with their block ids.
    pub fn layers(&self) -> impl Iterator<Item = (Id, Layer<'a>)> + use<'a> {
        self.state().layers()
    }

    pub(crate) fn chain_end(&self) -> u32 {
        self.state().end()
    }

    pub(crate) fn chain_get(&self, id: u32, c: usize) -> i64 {
        self.state().get(id, c)
    }

    pub(crate) fn chain_live(&self, id: u32) -> bool {
        self.state().live(id)
    }

    pub(crate) fn chain_live_word(&self, w: WordIndex) -> u64 {
        self.state().live_word(w)
    }

    pub(crate) fn chain_since(&self) -> Option<Since<'a>> {
        self.state().since()
    }

    pub(crate) fn chain_flips_since(&self, before: Option<&View<'a>>) -> Option<Flips<'a>> {
        let b = before.and_then(|b| b.chain);
        self.state().flips_since(b.as_ref()).map(Flips::chain)
    }

    pub(crate) fn chain_new_ranges(&self, before: Option<&View<'a>>) -> Option<Ranges> {
        let b = before.and_then(|b| b.chain);
        self.state().new_ranges(b.as_ref())
    }

    pub(crate) fn chain_exact_counts_since(&self, before: Option<&View<'a>>) -> Option<(usize, usize)> {
        let b = before.and_then(|b| b.chain);
        self.state().exact_counts_since(b.as_ref())
    }

    pub(crate) fn chain_partition(&self) -> Option<ColumnIndex> {
        self.state().partition()
    }

    /// The state's statistics, from its layers; a partition's (or the empty state's) from its rows.
    pub(crate) fn chain_stats(&self, c: ColumnIndex) -> ColStats {
        self.state().stats(c).unwrap_or_else(|| crate::bistate::stats_of(self, c))
    }

    pub(crate) fn chain_matching(&self, cols: &[ColumnIndex], key: &[i64]) -> ProbeIter<'a> {
        self.state().matching(cols, key)
    }

    pub(crate) fn chain_scan(&self) -> Scan<'a> {
        self.state().scan()
    }

    pub(crate) fn chain_changed_parts_since(&self, before: Option<&View<'a>>, out: &mut dyn FnMut(i64)) -> bool {
        let b = before.and_then(|b| b.chain);
        self.state().changed_parts_since(b.as_ref(), out)
    }
}
