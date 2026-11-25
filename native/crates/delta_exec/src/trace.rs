//! The program trace (decision 0686 B2; *0686: Operations* §7, §10; P§11–§12), on the host: handles only.
//!
//! A trace is a chain of frozen layer blocks, newest first, each referring to the layer below by block id. The host
//! holds the newest layer's id (`head`) and the trace's shape, and no row. A run's writers push `(row, diff)` pairs
//! into a δ runs block (`open`, `rerail_blocks::layer::RunsW`: atomic claims, no lock); `commit` runs `consolidate`,
//! which writes the new layer into a new block over the head, and the new block, frozen, becomes the head. The
//! committed state stays readable throughout: it is other blocks. `roll_up` runs `merge` over the newest layers and
//! lets the merged generations go.
//!
//! Two states are read (the user, 2026-11-24): `read` gives a launch over the layers a state reaches, and through its
//! descriptor table the state (`Chain`) and the change between two heads (`Bistate`), read where the layers lie.

use crate::dir::{BlockId, Directory, Launch};
use crate::op::{
    Across, Bound, Case, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit,
    Version, run_cpu,
};
use rerail_blocks::bistate::Bistate;
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table, word};
use rerail_blocks::layer::{Chain, Layer, NONE, Runs, Shape, ShapeW, init_runs, runs_bytes};
use rerail_delta_twins::Params;
use rerail_delta_twins::trace::{CONTROL_BYTES, ConsolidateP, MergeP, out_bytes, refused};

const fn ph(name: &'static str, found: Found, unit: Unit, inside: Inside, across: Across, output: Output, bounds: &'static [Bound]) -> Phase {
    Phase { name, found, unit, inside, across, output, bounds }
}

/// The phases `consolidate` and `merge` share (Operations §4): sort (14), segmented scan (10), emit (11), a batched
/// search (17), histogram (8) for the partition index, sorts for the flips and indexes, the indexes' fences, tags and
/// prints, Bloom words (18), the sketch (9), the seal.
static PIPELINE: [Phase; 25] = {
    use Found::*;
    use Output::*;
    use Unit::*;
    const PASSES: &[Bound] = &[Bound::Turns(32)];
    [
        ph("sort", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("passes", Indirect, Threadgroup, Inside::Nothing, Across::Ordered, ByIndex, PASSES),
        ph("segments", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("count", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("scan", Fixed, Threadgroup, Inside::GroupMemory, Across::Combine, ByScan, &[]),
        ph("place", Indirect, Threadgroup, Inside::Nothing, Across::Nothing, ByScan, &[]),
        ph("signs", Indirect, Lane, Inside::SimdOps, Across::Nothing, ByIndex, &[]),
        ph("kills", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("pcount", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("pscan", Fixed, Threadgroup, Inside::GroupMemory, Across::Combine, ByScan, &[]),
        ph("pindex", Indirect, Threadgroup, Inside::Nothing, Across::Nothing, ByScan, &[]),
        ph("pfill", Indirect, Partition, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("gscan", Fixed, Threadgroup, Inside::GroupMemory, Across::Combine, ByScan, &[]),
        ph("pflips", Indirect, Partition, Inside::Nothing, Across::Nothing, ByScan, &[]),
        ph("fsort", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("fpasses", Indirect, Threadgroup, Inside::Nothing, Across::Ordered, ByIndex, PASSES),
        ph("fsettle", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("isort", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("ipasses", Indirect, Threadgroup, Inside::Nothing, Across::Ordered, ByIndex, PASSES),
        ph("isettle", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("iaux", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("bloom", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("sketch", Indirect, Threadgroup, Inside::GroupMemory, Across::Combine, Sketch, &[]),
        ph("sketches", Fixed, Lane, Inside::Nothing, Across::Combine, Sketch, &[]),
        ph("seal", Fixed, Lane, Inside::Nothing, Across::Ordered, InPlace, &[]),
    ]
};

const ALONG: Independence = Independence::Along("rows: a chunk, a pair of runs, a partition, a column");

/// `consolidate`: scratch δ runs · write facts · rw control.
pub static CONSOLIDATE: Signature = Signature::new(
    "consolidate",
    &[
        SlotSpec { name: "δ runs", mode: Mode::Scratch, kind: kind::SCRATCH },
        SlotSpec { name: "facts", mode: Mode::Write, kind: kind::FACTS },
        SlotSpec { name: "control", mode: Mode::ReadWrite, kind: kind::CONTROL },
    ],
    ParamsLayout::of::<ConsolidateP>(),
    &PIPELINE,
    ALONG,
    facts_of,
);

/// `merge`: read facts (the newest layer merged; older ones by reference) · write facts.
pub static MERGE: Signature = Signature::new(
    "merge",
    &[SlotSpec { name: "newest", mode: Mode::Read, kind: kind::FACTS }, SlotSpec { name: "facts", mode: Mode::Write, kind: kind::FACTS }],
    ParamsLayout::of::<MergeP>(),
    &PIPELINE,
    ALONG,
    facts_of,
);

/// A layer's facts by meaning (F1): its rows with their signs and kills, its partition index, flips, sketches, indexes
/// and counts, each a tagged row; a control block's words.
pub fn facts_of(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == 2 {
        return vec![(0..4).map(|i| word::<u64>(bytes, BODY_AT + 8 + 8 * i) as i64).collect()];
    }
    let l = Layer::new(bytes).expect("a committed layer");
    let a = l.head.shape.arity();
    let mut out = Vec::new(); // copy: derived (a check's reading of facts)
    for i in 0..l.n() {
        let mut r = vec![0, i as i64];
        r.extend((0..a).map(|c| l.get(i, c)));
        r.extend([i64::from(l.is_plus(i)), i64::from(l.kills()[i])]);
        out.push(r);
    }
    for p in l.parts() {
        out.push(vec![1, p.key.into(), p.lo.into(), p.hi.into(), p.flo.into(), p.fhi.into(), p.added.into(), p.gone.into(), p.digest as i64]);
    }
    out.extend(l.flips().iter().enumerate().map(|(i, &f)| vec![2, i as i64, f.into()]));
    out.extend(l.pflips().iter().enumerate().map(|(i, &f)| vec![3, i as i64, f.into()]));
    for (c, s) in l.sketches().iter().enumerate() {
        let mut r = vec![4, c as i64, s.min, s.max, s.count];
        r.extend(s.ams);
        out.push(r);
    }
    for x in 0..l.head.shape.nidx as usize {
        if l.head.shape.index_kind(x) & rerail_blocks::layer::IX_BLOOM != 0 {
            out.extend(l.bloom(x).iter().enumerate().map(|(i, &w)| vec![9, x as i64, i as i64, w as i64]));
            continue;
        }
        out.extend(l.index(x).iter().enumerate().map(|(i, &p)| vec![5, x as i64, i as i64, p.into()]));
        out.extend(l.fences(x).iter().enumerate().map(|(i, &f)| vec![7, x as i64, i as i64, f]));
        out.extend(l.tags(x).iter().enumerate().map(|(i, &t)| vec![8, x as i64, i as i64, t.into()]));
        out.extend(l.prints(x).iter().enumerate().map(|(i, &p)| vec![10, x as i64, i as i64, p.into()]));
    }
    let h = l.head;
    out.push(vec![6, h.start.into(), h.n.into(), h.added.into(), h.gone.into(), h.nparts.into(), h.digest as i64, h.below.into()]);
    out
}

/// The trace operations, declared and their twins registered as references.
pub struct TraceOps {
    pub consolidate: OpId,
    pub merge: OpId,
}

impl TraceOps {
    pub fn declare(reg: &mut Registry) -> Result<TraceOps, String> {
        let consolidate = reg.declare(&CONSOLIDATE)?;
        reg.register_reference(Version::twin(consolidate, "consolidate twin", rerail_delta_twins::trace::CONSOLIDATE, &PIPELINE))?;
        let merge = reg.declare(&MERGE)?;
        reg.register_reference(Version::twin(merge, "merge twin", rerail_delta_twins::trace::MERGE, &PIPELINE))?;
        Ok(TraceOps { consolidate, merge })
    }
}

/// A trace on the host: its shape, and its newest layer's block id.
pub struct Trace {
    shape: ShapeW,
    head: Option<BlockId>,
}

impl Trace {
    /// An empty trace of `shape`; refused when the shape cannot be stored (P§11.2's grouping, what is not built).
    pub fn new(shape: &Shape) -> Result<Trace, String> {
        Ok(Trace { shape: shape.encode()?, head: None })
    }

    pub fn head(&self) -> Option<BlockId> {
        self.head
    }

    pub fn shape(&self) -> &ShapeW {
        &self.shape
    }

    /// A δ runs block for a run's writers: room for `cap` pairs.
    pub fn open(&self, dir: &mut Directory, cap: usize) -> BlockId {
        let id = dir.make(kind::SCRATCH, runs_bytes(&self.shape, cap));
        let b = dir.block(id).expect("just made");
        let mut w = rerail_blocks::region::pin_block(&b, "open", true).expect("a new block is unused");
        init_runs(w.bytes_mut().expect("a write pin"), &self.shape, cap);
        id
    }

    /// Commits the run's δ runs as the trace's new layer: `consolidate` writes it into a new block over the head,
    /// which is then frozen and becomes the head. The runs block is let go. Refused, with the head as it was, when
    /// the runs overflowed (rerun at the size it names) or `consolidate` refused the change.
    pub fn commit(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, runs: BlockId, tiling: Tiling) -> Result<BlockId, String> {
        let (n, needed) = {
            let l = dir.read("commit", &[runs])?;
            let r = Runs::new(l.block(0));
            (r.len(), r.needed())
        };
        if needed > 0 {
            dir.release(runs);
            return Err(format!("the run's δ outgrew its runs block by {needed} pairs: rerun at {}", n + needed));
        }
        let out = dir.make(kind::FACTS, out_bytes(&self.shape, n));
        if let Some(h) = self.head {
            dir.refer(out, h)?;
        }
        let ctl = dir.make(kind::CONTROL, CONTROL_BYTES);
        let mut params = vec![0u8; ConsolidateP::SIZE];
        ConsolidateP { below: self.head.map_or(NONE, |h| h.0), pad: 0 }.write(&mut params);
        let run = run_cpu(reg, dir, ops.consolidate, &[runs, out, ctl], &params, tiling);
        let code = {
            let l = dir.read("commit", &[ctl])?;
            refused(l.block(0))
        };
        dir.release(runs);
        dir.release(ctl);
        if let Err(e) = run {
            dir.release(out);
            return Err(e);
        }
        if code != 0 {
            dir.release(out);
            return Err(format!("consolidate refused the run's change: {}", match code {
                1 => "a row's diffs sum past ±1",
                2 => "a row gone is not live in the trace",
                3 => "a row added is already live in the trace",
                _ => "an unknown refusal",
            }));
        }
        dir.freeze(out)?;
        if let Some(h) = self.head {
            dir.release(h);
        }
        self.head = Some(out);
        Ok(out)
    }

    /// Rolls the newest `count` layers up into one (`merge`), over the layer below them; the merged generations are let
    /// go, and live on only while a reader holds them.
    pub fn roll_up(&mut self, reg: &Registry, ops: &TraceOps, dir: &mut Directory, count: u32, tiling: Tiling) -> Result<BlockId, String> {
        let head = self.head.ok_or("a roll-up of an empty trace")?;
        let (rows, below) = {
            let l = dir.read("roll-up", &[head])?;
            let table = l.table();
            let (mut rows, mut id) = (0usize, Some(head.0));
            for _ in 0..count {
                let layer = Layer::new(table.block(id.ok_or("fewer layers than merged")?).expect("reached"))?;
                rows += layer.n();
                id = layer.below();
            }
            (rows, id)
        };
        let out = dir.make(kind::FACTS, out_bytes(&self.shape, rows));
        if let Some(b) = below {
            dir.refer(out, BlockId(b))?;
        }
        let mut params = vec![0u8; MergeP::SIZE];
        MergeP { count, below: below.unwrap_or(NONE) }.write(&mut params);
        if let Err(e) = run_cpu(reg, dir, ops.merge, &[head, out], &params, tiling) {
            dir.release(out);
            return Err(e);
        }
        dir.freeze(out)?;
        dir.release(head);
        self.head = Some(out);
        Ok(out)
    }

    /// A launch reading the trace's layers from `heads` down: each pinned to read, with the descriptor table of them.
    pub fn read(&self, dir: &Directory, heads: &[Option<BlockId>]) -> Result<Launch, String> {
        dir.read("read", &heads.iter().flatten().copied().collect::<Vec<_>>())
    }

    /// The state at `head`, read through a launch's table.
    pub fn state<'t>(&self, table: &'t Table<'t>, head: Option<BlockId>) -> Result<Chain<'t>, String> {
        Chain::new(table, head.map(|h| h.0), self.shape.arity())
    }

    /// The change from the state at `before` to the state at `after` (`before` below `after` in the chain).
    pub fn change<'t>(&self, table: &'t Table<'t>, before: Option<BlockId>, after: Option<BlockId>) -> Result<Bistate<Chain<'t>>, String> {
        let a = self.state(table, after)?;
        Ok(match before {
            None => Bistate::cold(a),
            Some(b) => Bistate::new(self.state(table, Some(b))?, a),
        })
    }
}

/// A launch's case for the version check: a fresh trace's first commit of `rows`.
pub fn consolidate_case<'r>(shape: &'r Shape, rows: &'r [(Vec<i64>, i32)]) -> impl Fn(&mut Directory) -> Case + 'r {
    move |dir: &mut Directory| {
        let t = Trace::new(shape).expect("a storable shape");
        let runs = t.open(dir, rows.len());
        {
            let b = dir.block(runs).expect("made");
            let pin = rerail_blocks::region::pin_block(&b, "head", true).expect("unused");
            let w = rerail_blocks::layer::RunsW::new(&pin);
            for (r, d) in rows {
                w.push(r, *d).expect("room");
            }
        }
        let out = dir.make(kind::FACTS, out_bytes(t.shape(), rows.len()));
        let ctl = dir.make(kind::CONTROL, CONTROL_BYTES);
        let mut params = vec![0u8; ConsolidateP::SIZE];
        ConsolidateP { below: NONE, pad: 0 }.write(&mut params);
        Case { slots: vec![runs, out, ctl], params }
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
