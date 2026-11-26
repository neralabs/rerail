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
//! descriptor table the state (`relation::View::chain`) and the change between two heads (`Bistate`), read where the layers lie.

use crate::dir::{BlockId, Directory, Launch};
use crate::op::{
    Across, Bound, Case, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit,
    Version, run_cpu,
};
use rerail_blocks::bistate::Bistate;
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table, word};
use rerail_blocks::layer::{Layer, NONE, Runs, Shape, ShapeW, init_runs, layer_room, runs_bytes};
use rerail_blocks::relation::View;
use rerail_delta_twins::Params;
use rerail_delta_twins::trace::{CONTROL_BYTES, ConsolidateP, MergeP, due, out_bytes, refused};

const fn ph(name: &'static str, found: Found, unit: Unit, inside: Inside, across: Across, output: Output, bounds: &'static [Bound]) -> Phase {
    Phase { name, found, unit, inside, across, output, bounds }
}

/// The phases `consolidate` and `merge` share (Operations §4), each on the six axes, as their twins run them:
///
/// - sort (14): chunks sorted, then merge passes, rounds bounded at 32 turns;
/// - segmented sums (10), then emit (11): each chunk's counts, their scan (in order, a carry from chunk to chunk), the
///   layer opened (one group), the rows placed;
/// - a batched search of the state below (17), a row out a lane; the kills sorted (14) and placed in the change;
/// - the partitions' change (8): each row out's start flag, the starts numbered (in order), then a partition an item;
/// - the freeze where the layer lies: a column's range a lane, a hashed index (18) a group, the seal (one group);
/// - its orders and Bloom filters (18), an index a group; its statistics (9), a column a lane; the header, last.
static PIPELINE: [Phase; 19] = {
    use Found::*;
    use Output::*;
    use Unit::*;
    const PASSES: &[Bound] = &[Bound::Turns(32)];
    // A kill's search reads a layer a step; the geometric roll-up keeps a trace of n rows at O(log n) layers.
    const KILL_STEPS: &[Bound] = &[Bound::Steps(64)];
    [
        ph("sort", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("passes", Indirect, Threadgroup, Inside::Nothing, Across::Rounds, ByIndex, PASSES),
        ph("segments", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("count", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("scan", Fixed, Threadgroup, Inside::Nothing, Across::Ordered, ByScan, &[]),
        ph("open", Fixed, Threadgroup, Inside::Nothing, Across::Nothing, InPlace, &[]),
        ph("place", Fixed, Threadgroup, Inside::Nothing, Across::Nothing, ByScan, &[]),
        ph("encode", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("perm", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("kills", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, KILL_STEPS),
        ph("pstarts", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("pscan", Indirect, Threadgroup, Inside::Nothing, Across::Ordered, ByScan, &[]),
        ph("parts", Indirect, Partition, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("stats", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("indexes", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("freeze", Fixed, Threadgroup, Inside::Nothing, Across::Nothing, InPlace, &[]),
        ph("orders", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("sketch", Fixed, Lane, Inside::Nothing, Across::Nothing, Sketch, &[]),
        ph("seal", Fixed, Threadgroup, Inside::Nothing, Across::Nothing, InPlace, &[]),
    ]
};

const ALONG: Independence = Independence::Along("rows: a chunk, a pair of runs, a row out, an index");

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

/// A layer's facts by meaning (F1), each a tagged row: its rows (id, values), its kills, its partitions' change, its
/// kills by partition, each index (an order's ids, fences and prints by its key; a Bloom filter's words), each
/// column's statistics, its counts and the layer below; a control block's words. Where the block put each structure
/// is not a fact.
pub fn facts_of(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == 2 {
        return vec![(0..5).map(|i| word::<u64>(bytes, BODY_AT + 8 + 8 * i) as i64).collect()];
    }
    let l = Layer::new(bytes).expect("a committed layer");
    let a = l.head.shape.arity();
    let mut out = Vec::new(); // copy: derived (a check's reading of facts)
    for id in l.own.ids() {
        let mut r = vec![0, i64::from(id)];
        r.extend((0..a).map(|c| l.get(id, c)));
        out.push(r);
    }
    out.extend(l.kills().iter().map(|&k| vec![1, i64::from(k)]));
    for id in l.parts.ids() {
        out.push(vec![2, l.parts.own(id, 0), l.parts.own(id, 1), l.parts.own(id, 2), l.parts.own(id, 3)]);
    }
    for id in l.gone.ids() {
        out.push(vec![3, l.gone.own(id, 0), l.gone.own(id, 1)]);
    }
    let s = &l.head.shape;
    for x in 0..s.nidx as usize {
        if let Some(b) = l.bloom(x) {
            out.extend((0..b.len()).map(|i| vec![4, x as i64, i as i64, b.word(i) as i64]));
            continue;
        }
        let cols: Vec<u16> = s.index_key(x).iter().map(|&c| u16::from(c)).collect();
        let Some(o) = l.rows.find_order(&cols, l.n() as u32) else { continue };
        let ix = l.rows.index_at(o);
        out.extend(ix.rows.as_slice().iter().enumerate().map(|(i, &id)| vec![5, x as i64, i as i64, i64::from(id)]));
        out.extend(ix.fences.as_slice().iter().enumerate().map(|(i, &f)| vec![6, x as i64, i as i64, f]));
        out.extend(ix.prints.as_slice().iter().enumerate().map(|(i, &p)| vec![7, x as i64, i as i64, i64::from(p)]));
    }
    for c in 0..a {
        let r = l.sketch(c).map(|s| s.reading()).unwrap_or_default();
        out.push(vec![8, c as i64, r.rows as i64, r.ndv as i64, r.sq as i64]);
    }
    let h = l.head;
    out.push(vec![9, i64::from(l.start()), i64::from(h.added), i64::from(h.ngone), i64::from(h.nparts), h.digest as i64, l.below().map_or(-1, |(b, _)| i64::from(b))]);
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

/// When a trace's newest layers are merged (O5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RollUp {
    /// Geometric (the settled policy): after each commit, the newest layers `consolidate` found due
    /// (`due_to_merge`), merged into one.
    Geometric,
    /// Never on its own: only by `roll_up` (a test that reads a trace's layers as committed).
    Never,
}

/// A trace on the host: its shape, its newest layer's block id and its roll-up policy.
pub struct Trace {
    shape: ShapeW,
    head: Option<BlockId>,
    roll: RollUp,
}

impl Trace {
    /// An empty trace of `shape`; refused when the shape cannot be stored (P§11.2's grouping, what is not built).
    pub fn new(shape: &Shape) -> Result<Trace, String> {
        Ok(Trace { shape: shape.encode()?, head: None, roll: RollUp::Geometric })
    }

    /// A kept trace (`Placement::KeptFile`) opened from its blocks' folder (`Directory::open_kept`): its head the
    /// layer no other refers to.
    pub fn open_kept(shape: &Shape, dir: &Directory) -> Result<Trace, String> {
        let mut t = Trace::new(shape)?;
        if t.shape.placement != crate::dir::PLACE_KEPT {
            return Err("a trace opened from kept files is kept (Placement::KeptFile)".into());
        }
        match dir.unreferred()[..] {
            [] => {}
            [h] => t.head = Some(h),
            ref many => return Err(format!("a kept folder of one trace has one head, not {many:?}")),
        }
        Ok(t)
    }

    /// The trace with roll-up policy `roll`.
    pub fn with_roll_up(self, roll: RollUp) -> Trace {
        Trace { roll, ..self }
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
    /// which is then frozen and becomes the head; then, by the trace's roll-up policy, the newest layers due to merge
    /// are merged (`roll_up`), and the merged layer is the head. The runs block is let go. Refused, with the head as it was, when
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
        let out = dir.make_placed(self.shape.placement, kind::FACTS, out_bytes(n), layer_room(&self.shape, n))?;
        if let Some(h) = self.head {
            dir.refer(out, h, BODY_AT)?;
        }
        let ctl = dir.make(kind::CONTROL, CONTROL_BYTES);
        let mut params = vec![0u8; ConsolidateP::SIZE];
        ConsolidateP { below: self.head.map_or(NONE, |h| h.0), pad: 0 }.write(&mut params);
        let run = run_cpu(reg, dir, ops.consolidate, &[runs, out, ctl], &params, tiling);
        let (code, due) = {
            let l = dir.read("commit", &[ctl])?;
            (refused(l.block(0)), due(l.block(0)))
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
                4 => "a key is live at two levels (0364: a rewrite kills the old level's row)",
                _ => "an unknown refusal",
            }));
        }
        dir.freeze(out)?;
        if let Some(h) = self.head {
            dir.release(h);
        }
        self.head = Some(out);
        if self.roll == RollUp::Geometric && due >= 2 {
            return self.roll_up(reg, ops, dir, due as u32, tiling);
        }
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
                rows += layer.n() + layer.kills().len();
                id = layer.below().map(|(b, _)| b);
            }
            (rows, id)
        };
        let out = dir.make_placed(self.shape.placement, kind::FACTS, out_bytes(rows), layer_room(&self.shape, rows))?;
        if let Some(b) = below {
            dir.refer(out, BlockId(b), BODY_AT)?;
        }
        let mut params = vec![0u8; MergeP::SIZE];
        MergeP { head: head.0, count, below: below.unwrap_or(NONE), pad: 0 }.write(&mut params);
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
    pub fn state<'t>(&self, table: &'t Table<'t>, head: Option<BlockId>) -> Result<View<'t>, String> {
        View::chain(table, head.map(|h| h.0), self.shape.arity())
    }

    /// The change from the state at `before` to the state at `after` (`before` below `after` in the chain).
    pub fn change<'t>(&self, table: &'t Table<'t>, before: Option<BlockId>, after: Option<BlockId>) -> Result<Bistate<View<'t>>, String> {
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
        let out = dir.make_room(kind::FACTS, out_bytes(rows.len()), layer_room(t.shape(), rows.len()));
        let ctl = dir.make(kind::CONTROL, CONTROL_BYTES);
        let mut params = vec![0u8; ConsolidateP::SIZE];
        ConsolidateP { below: NONE, pad: 0 }.write(&mut params);
        Case { slots: vec![runs, out, ctl], params }
    }
}

/// A launch's case for the version check of `merge`: a trace of `runs` committed in turn (by the reference
/// consolidate, whole), then its newest `count` layers merged over the layer below them.
pub fn merge_case<'r>(shape: &'r Shape, runs: &'r [Vec<(Vec<i64>, i32)>], count: u32) -> impl Fn(&mut Directory) -> Case + 'r {
    move |dir: &mut Directory| {
        let mut reg = Registry::default();
        let ops = TraceOps::declare(&mut reg).expect("the trace's operations");
        // A layer a run: the merge's input as committed, not rolled up.
        let mut t = Trace::new(shape).expect("a storable shape").with_roll_up(RollUp::Never);
        let mut heads = Vec::new();
        for rows in runs {
            let id = t.open(dir, rows.len().max(1));
            {
                let b = dir.block(id).expect("made");
                let pin = rerail_blocks::region::pin_block(&b, "case", true).expect("unused");
                let w = rerail_blocks::layer::RunsW::new(&pin);
                for (r, d) in rows {
                    w.push(r, *d).expect("room");
                }
            }
            heads.push(t.commit(&reg, &ops, dir, id, Tiling::WHOLE).expect("a set change"));
        }
        let head = *heads.last().expect("a layer");
        let below = heads.len().checked_sub(count as usize + 1).map(|i| heads[i]);
        let total: usize = runs[runs.len() - count as usize..].iter().map(Vec::len).sum();
        let out = dir.make_room(kind::FACTS, out_bytes(total), layer_room(t.shape(), total));
        if let Some(b) = below {
            dir.refer(out, b, BODY_AT).expect("frozen");
        }
        let mut params = vec![0u8; MergeP::SIZE];
        MergeP { head: head.0, count, below: below.map_or(NONE, |b| b.0), pad: 0 }.write(&mut params);
        Case { slots: vec![head, out], params }
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
