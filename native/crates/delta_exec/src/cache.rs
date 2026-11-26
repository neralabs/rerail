//! An operator's cache, on the host: handles only (P§2, L5, P§11.3; *0686: Operations* §6.1, §10).
//!
//! A delta operator holds one or more cache blocks across its runs, each a mutable block of kind `CACHE`, never
//! frozen, with no generations: one state, mutated in place, its header naming that state. They come into its
//! operations as slots: read for an operation that probes the cache, read-write for one that adds to it, so the
//! regions' checks give the one-writer rule (two operations writing one cache block cannot run together). Its layout
//! and its operations' twins are `rerail_delta_twins::cache`:
//!
//! - `cache_open` (rw the cache): a header open, or naming another state than the run's before, is reset to ∅ where it
//!   lies; then the header is marked open;
//! - `order` (read a layer, rw the cache): a layer's row ids in a column order, with fences, added where a plan probes
//!   the layer so and the cache does not hold them; the cache grows within its reservation (`Capacity(Grow)`);
//! - `cache_commit` (rw the cache): the after state's digest written and the mark cleared, last.
//!
//! Dropping a cache is always correct (L5): a new one is ∅, and its operator's answers do not change.

use crate::dir::{BlockId, Directory};
use crate::op::{Across, Bound, Found, Independence, Inside, Mode, OpId, Output, Over, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use rerail_blocks::block::kind;
use rerail_blocks::gens::Table;
use rerail_delta_twins::Params;
use rerail_delta_twins::cache::{self as twin, CACHE_BYTES, CacheP, OrderP};

const fn one(name: &'static str, inside: Inside, output: Output, bounds: &'static [Bound]) -> Phase {
    Phase { name, found: Found::Fixed, unit: Unit::Threadgroup, inside, across: Across::Nothing, output, bounds }
}

static OPEN_PHASE: [Phase; 1] = [one("open", Inside::Nothing, Output::InPlace, &[])];
static COMMIT_PHASE: [Phase; 1] = [one("commit", Inside::Nothing, Output::InPlace, &[])];
/// A sort (14) of a layer's ids, with its runs' fences (15), in a group's memory; appended at the cache's cursor, which
/// grows the cache within its reservation.
static ORDER_PHASE: [Phase; 1] = [one("order", Inside::GroupMemory, Output::WriterState, &[Bound::Capacity(Over::Grow)])];

/// `cache_open`: rw the cache: reset to ∅ unless it holds the run's before, then marked open.
pub static CACHE_OPEN: Signature = Signature::new(
    "cache_open",
    &[SlotSpec { name: "cache", mode: Mode::ReadWrite, kind: kind::CACHE }],
    ParamsLayout::of::<CacheP>(),
    &OPEN_PHASE,
    Independence::Along("one cache"),
    cache_facts,
);

/// `cache_commit`: rw the cache: the after state's digest, then the mark cleared, last.
pub static CACHE_COMMIT: Signature = Signature::new(
    "cache_commit",
    &[SlotSpec { name: "cache", mode: Mode::ReadWrite, kind: kind::CACHE }],
    ParamsLayout::of::<CacheP>(),
    &COMMIT_PHASE,
    Independence::Along("one cache"),
    cache_facts,
);

/// `order`: read a layer · rw the cache.
pub static ORDER: Signature = Signature::new(
    "order",
    &[SlotSpec { name: "layer", mode: Mode::Read, kind: kind::FACTS }, SlotSpec { name: "cache", mode: Mode::ReadWrite, kind: kind::CACHE }],
    ParamsLayout::of::<OrderP>(),
    &ORDER_PHASE,
    Independence::Not("one order a launch, appended at the cache's cursor"),
    cache_facts,
);

/// A cache's facts by meaning: its header's state, open mark and what the last open found; then each order it holds,
/// by its layer and columns, as its ids and fences.
pub fn cache_facts(_: &Table<'_>, _: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    let mut out = vec![vec![-1, twin::state(bytes) as i64, i64::from(twin::is_open(bytes)), i64::from(twin::held(bytes))]];
    out.extend(twin::entries(bytes).map(|e| {
        let o = twin::order(bytes, e.layer, &e.cols).expect("an entry the cache holds");
        let mut r = vec![i64::from(e.layer)];
        r.extend(e.cols.iter().map(|&c| i64::from(c)));
        r.extend(o.ids.iter().map(|&i| i64::from(i)));
        r.extend(o.fences);
        r
    }));
    out
}

/// The cache operations, declared and their twins registered as references.
pub struct CacheOps {
    pub open: OpId,
    pub commit: OpId,
    pub order: OpId,
}

impl CacheOps {
    pub fn declare(reg: &mut Registry) -> Result<CacheOps, String> {
        let open = reg.declare(&CACHE_OPEN)?;
        reg.register_reference(Version::twin(open, "cache_open twin", twin::CACHE_OPEN, &OPEN_PHASE))?;
        let commit = reg.declare(&CACHE_COMMIT)?;
        reg.register_reference(Version::twin(commit, "cache_commit twin", twin::CACHE_COMMIT, &COMMIT_PHASE))?;
        let order = reg.declare(&ORDER)?;
        reg.register_reference(Version::twin(order, "order twin", twin::ORDER, &ORDER_PHASE))?;
        Ok(CacheOps { open, commit, order })
    }
}

/// A cache block's first capacity, and its reservation (address space only: nothing is resident until written).
pub const CACHE_CAPACITY: usize = 64 << 10;
pub const CACHE_RESERVE: usize = 1 << 30;

/// One block of an operator's cache, by its id: made ∅, kept across runs by the operator's instance, dropped at will.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpCache(pub BlockId);

impl OpCache {
    /// A new cache block: ∅ (its header names state 0 and no entries).
    pub fn new(dir: &mut Directory) -> OpCache {
        OpCache(dir.make_capped(kind::CACHE, CACHE_BYTES, CACHE_CAPACITY, CACHE_RESERVE))
    }

    /// A run begins: the header marked open, the cache reset to ∅ unless it holds `from`, the state the run starts
    /// from. True when it held it.
    pub fn open(self, reg: &Registry, ops: &CacheOps, dir: &Directory, from: u64) -> Result<bool, String> {
        let mut p = vec![0u8; CacheP::SIZE];
        CacheP { state: from }.write(&mut p);
        run_cpu(reg, dir, ops.open, &[self.0], &p, Tiling::WHOLE)?;
        Ok(twin::held(dir.read("cache", &[self.0])?.block(0)))
    }

    /// A run ends: `to`, the state it made, in the header, and the mark cleared, last.
    pub fn commit(self, reg: &Registry, ops: &CacheOps, dir: &Directory, to: u64) -> Result<(), String> {
        let mut p = vec![0u8; CacheP::SIZE];
        CacheP { state: to }.write(&mut p);
        run_cpu(reg, dir, ops.commit, &[self.0], &p, Tiling::WHOLE)
    }

    /// Adds layer `layer`'s order by `cols` to the cache, unless it holds it (`order`).
    pub fn order(self, reg: &Registry, ops: &CacheOps, dir: &Directory, layer: BlockId, cols: &[u16]) -> Result<(), String> {
        let mut p = vec![0u8; OrderP::SIZE];
        twin::order_params(layer.0, cols).write(&mut p);
        run_cpu(reg, dir, ops.order, &[layer, self.0], &p, Tiling::WHOLE)
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
