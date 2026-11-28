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
use crate::op::{Across, Bound, Found, Independence, Inside, Output, Over, ParamsLayout, Phase, Registry, Runner, Signature, Tiling, Unit, Version};
#[cfg(any(test, feature = "test-hooks"))]
use crate::op::Case;
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::MAX_COLS;
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table, word};
use rerail_blocks::layer::{Layer, NONE, Runs, Shape, ShapeW, init_runs, layer_room, runs_bytes};
use rerail_blocks::relation::View;
use rerail_delta_twins::Params;
use rerail_delta_twins::trace::{CONTROL_BYTES, ConsolidateP, MergeP, due, out_bytes, refused, scratch_bytes};

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
    // A Dict column's distinct values past the scratch slot's capacity: the slot grown, the phase run again.
    const GROWS: &[Bound] = &[Bound::Capacity(Over::Grow)];
    // A kill's search reads a layer a step; the geometric roll-up keeps a trace of n rows at O(log n) layers.
    // A commit's kill search reads at most the floor's `MAX_BELOW` layers below it (a trace's state's bound).
    const KILL_STEPS: &[Bound] = &[Bound::Steps(rerail_blocks::layer::MAX_BELOW as u32)];
    [
        ph("sort", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("passes", Indirect, Threadgroup, Inside::Nothing, Across::Rounds, ByIndex, PASSES),
        ph("segments", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("count", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("scan", Fixed, Threadgroup, Inside::Nothing, Across::Ordered, ByScan, &[]),
        ph("open", Fixed, Threadgroup, Inside::Nothing, Across::Nothing, InPlace, &[]),
        ph("place", Fixed, Threadgroup, Inside::Nothing, Across::Nothing, ByScan, &[]),
        ph("encode", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, GROWS),
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

/// The UKL version's shape (`ukl/trace.uglykl`), the twins' phases across their items, those that read only what
/// the phase before last wrote run together (a phase over the most of their items): the source sorted (each chunk's
/// items ranked, a lane an item, then merge passes of 16 runs, rounds bounded at 6 turns: 2^30 items), each run's
/// diffs summed, the rows out counted (a chunk a group); scanned, the layer opened and its indexes' entries laid out
/// (one group: the opening one thread, its room claimed from the block's front); the rows placed (a chunk a group);
/// each row out's kill and partition start, each index element's hash (a lane each); the digests summed (a chunk a
/// group), the partitions numbered and the digests' chunks scanned (one group); each partition's change, each row out
/// and index element ranked in its chunk (a lane each), the kills' and the indexes' merge passes (rounds, a lane an
/// element of either); the kills, the indexes' rows and marks and their buckets' starts placed (a lane each); each
/// column's range (a chunk of rows a group, then a column a lane, with each index place's mark); each column's sketch
/// (a group, claimed so too); the freeze and the header last.
pub(crate) static TRACE_UKL: [Phase; 16] = {
    use Found::*;
    use Output::*;
    use Unit::*;
    const CLAIMS: &[Bound] = &[Bound::Capacity(Over::Grow)];
    const PASSES: &[Bound] = &[Bound::Turns(6)];
    [
        ph("sort", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("passes", Indirect, Lane, Inside::Nothing, Across::Rounds, ByIndex, PASSES),
        ph("segments", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("count", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("open", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByScan, CLAIMS),
        ph("place", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, ByScan, &[]),
        ph("rowsout", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("digests", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByScan, &[]),
        ph("pscan", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByScan, CLAIMS),
        ph("ranks", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("xpasses", Indirect, Lane, Inside::Nothing, Across::Rounds, ByIndex, PASSES),
        ph("places", Indirect, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("stats", Indirect, Threadgroup, Inside::GroupMemory, Across::Nothing, ByIndex, &[]),
        ph("ranges", Fixed, Lane, Inside::Nothing, Across::Nothing, ByIndex, &[]),
        ph("sketch", Fixed, Threadgroup, Inside::GroupMemory, Across::Nothing, Sketch, CLAIMS),
        ph("seal", Fixed, Lane, Inside::Nothing, Across::Nothing, InPlace, &[]),
    ]
};

/// A launch's scratch slot over `cap` source items: `scratch_bytes(cap)` of capacity, in a reservation four times
/// that, so a phase whose work runs out (a Dict column's distinct values, `encode`) grows it and runs again.
pub(crate) fn scratch_slot(dir: &mut Directory, cap: usize) -> BlockId {
    let bytes = scratch_bytes(cap);
    dir.make_capped(kind::SCRATCH, 0, BODY_AT + bytes, 4 * (BODY_AT + bytes))
}

const ALONG: Independence = Independence::Along("rows: a chunk, a pair of runs, a row out, an index");

/// `consolidate`: scratch δ runs · write facts · rw control · scratch (a call's temporaries: an index's, an order's).
pub static CONSOLIDATE: Signature = Signature::of(
    &crate::op::interfaces::CONSOLIDATE,
    ParamsLayout::of::<ConsolidateP>(),
    &PIPELINE,
    ALONG,
    facts_of,
);

/// `merge`: read facts (the newest layer merged; older ones by reference) · write facts · scratch.
pub static MERGE: Signature = Signature::of(
    &crate::op::interfaces::MERGE,
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
        let r = l.reading(c).unwrap_or_default();
        out.push(vec![8, c as i64, r.rows as i64, r.ndv as i64, r.sq as i64]);
    }
    let h = l.head;
    out.push(vec![9, i64::from(l.start()), i64::from(h.added), i64::from(h.ngone), i64::from(h.nparts), h.digest as i64, l.below().map_or(-1, |(b, _)| i64::from(b))]);
    out
}

/// Why `consolidate` refused a run's change, by its code.
fn refusal_of(code: u64) -> &'static str {
    match code {
        1 => "a row's diffs sum past ±1",
        2 => "a row gone is not live in the trace",
        3 => "a row added is already live in the trace",
        4 => "a key is live at two levels (0364: a rewrite kills the old level's row)",
        _ => "an unknown refusal",
    }
}

/// Whether the runs block `runs` holds a pair.
pub fn holds_rows(dir: &Directory, runs: BlockId) -> Result<bool, String> {
    Ok(!Runs::new(dir.read("a runs block", &[runs])?.block(0)).is_empty())
}

/// A δ runs block of rows of `shape`, with room for `cap` of them, empty.
pub fn open_runs(dir: &mut Directory, shape: &ShapeW, cap: usize) -> BlockId {
    let id = dir.make(kind::SCRATCH, runs_bytes(shape, cap));
    let b = dir.block(id).expect("just made");
    let mut w = rerail_blocks::region::pin_block(&b, "open", true).expect("a new block is unused");
    init_runs(w.bytes_mut().expect("a write pin"), shape, cap);
    id
}

/// The module's operations declared, each from its UKL interface, and their twins registered as references: the
/// runner's registry (`Runner::new`), the one place they are registered.
pub(crate) fn register(reg: &mut Registry) -> Result<(), String> {
    let consolidate = reg.declare(&CONSOLIDATE)?;
    reg.register_reference(Version::twin(consolidate, "consolidate twin", rerail_delta_twins::trace::CONSOLIDATE, &PIPELINE))?;
    let merge = reg.declare(&MERGE)?;
    reg.register_reference(Version::twin(merge, "merge twin", rerail_delta_twins::trace::MERGE, &PIPELINE))?;
    Ok(())
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

/// A trace on the host: its shape, its newest layer's block id, its roll-up policy, and the rows a run has written
/// and not yet committed (`write`): its open runs, read after the head by `now`, committed with the run's other traces
/// in one launch (`commit_wide`).
pub struct Trace {
    shape: ShapeW,
    head: Option<BlockId>,
    roll: RollUp,
    written: Option<BlockId>,
}

/// The room a trace's open runs first have (`Trace::write`); past it, what is open is committed and the next runs
/// have twice the room.
const FIRST_WRITTEN: usize = 64;

impl Trace {
    /// An empty trace of `shape`; refused when the shape cannot be stored (P§11.2's grouping, what is not built).
    pub fn new(shape: &Shape) -> Result<Trace, String> {
        Ok(Trace { shape: shape.encode()?, head: None, roll: RollUp::Geometric, written: None })
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

    /// The trace with its layers placed at `place` (`dir::PLACE_KEPT`: kept files, which last across processes),
    /// resumed at the committed state `head` a kept run left (`None`: empty): a constructor, as `open_kept` is.
    pub fn resumed(mut self, place: u32, head: Option<BlockId>) -> Trace {
        self.shape.placement = place;
        Trace { head, ..self }
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
        open_runs(dir, &self.shape, cap)
    }

    /// Commits the run's δ runs as the trace's new layer: `consolidate` writes it into a new block over the head,
    /// which is then frozen and becomes the head; then, by the trace's roll-up policy, the newest layers due to merge
    /// are merged (`roll_up`), and the merged layer is the head. The runs block is let go. Refused, with the head as it was, when
    /// the runs overflowed (rerun at the size it names) or `consolidate` refused the change.
    pub fn commit(&mut self, ops: &Runner, dir: &mut Directory, runs: BlockId, tiling: Tiling) -> Result<BlockId, String> {
        Trace::commit_wide(ops, dir, &mut [(self, runs)], tiling)?;
        Ok(self.head.expect("committed"))
    }

    /// Commits each trace's runs as its new layer (`commit`), every one's `consolidate` in one launch (`run_wide`), the
    /// traces' layers independent; then each one's roll-up. Refused, every head as it was, when a runs block
    /// overflowed or `consolidate` refused a change; every runs block is let go.
    pub fn commit_wide(ops: &Runner, dir: &mut Directory, items: &mut [(&mut Trace, BlockId)], tiling: Tiling) -> Result<(), String> {
        let release_runs = |dir: &mut Directory, items: &[(&mut Trace, BlockId)]| items.iter().for_each(|(_, r)| dir.release(*r));
        let mut sizes = Vec::with_capacity(items.len()); // copy: not block data (each runs block's pairs)
        for (_, runs) in items.iter() {
            let (n, needed) = {
                let l = dir.read("commit", &[*runs])?;
                let r = Runs::new(l.block(0));
                (r.len(), r.needed())
            };
            if needed > 0 {
                release_runs(dir, items);
                return Err(format!("the run's δ outgrew its runs block by {needed} pairs: rerun at {}", n + needed));
            }
            sizes.push(n);
        }
        // Each trace's launch: its runs, the new layer's block over its head, control, and the scratch slot.
        let mut slots: Vec<[BlockId; 4]> = Vec::with_capacity(items.len()); // copy: not block data (the launches' slots)
        let mut params: Vec<[u8; ConsolidateP::SIZE]> = Vec::with_capacity(items.len()); // copy: not block data (the launches' params)
        let made = (|| {
            for ((t, runs), &n) in items.iter().zip(&sizes) {
                let out = dir.make_placed(t.shape.placement, kind::FACTS, out_bytes(n), layer_room(&t.shape, n))?;
                slots.push([*runs, out, dir.make(kind::CONTROL, CONTROL_BYTES), scratch_slot(dir, n)]);
                if let Some(h) = t.head {
                    dir.refer(out, h, BODY_AT)?;
                }
                let mut p = [0u8; ConsolidateP::SIZE];
                ConsolidateP { below: t.head.map_or(NONE, |h| h.0), pad: 0 }.write(&mut p);
                params.push(p);
            }
            Ok::<_, String>(())
        })();
        let run = made.and_then(|()| {
            let wide: Vec<(&[BlockId], &[u8])> = slots.iter().zip(&params).map(|(s, p)| (&s[..], &p[..])).collect(); // copy: not block data (the launch's instances)
            ops.run_wide(dir, &CONSOLIDATE, &wide, tiling)
        });
        let mut codes = Vec::with_capacity(slots.len()); // copy: not block data (each launch's refusal and roll-up)
        for s in &slots {
            if run.is_ok() {
                let l = dir.read("commit", &[s[2]])?;
                codes.push((refused(l.block(0)), due(l.block(0))));
            }
            for &b in &[s[0], s[2], s[3]] {
                dir.release(b);
            }
        }
        // A runs block whose launch was never made.
        for (_, r) in items.iter().skip(slots.len()) {
            dir.release(*r);
        }
        let refusal = codes.iter().find(|c| c.0 != 0).map(|c| c.0);
        if let Err(e) = run.and_then(|()| refusal.map_or(Ok(()), |code| Err(format!("consolidate refused the run's change: {}", refusal_of(code))))) {
            slots.iter().for_each(|s| dir.release(s[1]));
            return Err(e);
        }
        for ((t, _), (s, &(_, due))) in items.iter_mut().zip(slots.iter().zip(&codes)) {
            let out = s[1];
            dir.freeze(out)?;
            if let Some(h) = t.head {
                dir.release(h);
            }
            t.head = Some(out);
            if t.roll == RollUp::Geometric && due >= 2 {
                t.roll_up(ops, dir, due as u32, tiling)?;
            }
        }
        Ok(())
    }

    /// Writes `pairs` into the trace's open runs, as the run makes them (`now` reads them; `commit_open` commits them).
    /// Past their room, what is open is committed, and the next pairs go into runs of twice the room.
    pub fn write<R: AsRef<[i64]>>(&mut self, ops: &Runner, dir: &mut Directory, pairs: &[(R, i32)]) -> Result<(), String> {
        let r = match self.written {
            Some(r) => {
                let (len, cap) = {
                    let l = dir.read("a trace's open runs", &[r])?;
                    let runs = Runs::new(l.block(0));
                    (runs.len(), runs.head.cap as usize)
                };
                if len + pairs.len() <= cap {
                    r
                } else {
                    self.written = None;
                    self.commit(ops, dir, r, Tiling::WHOLE)?;
                    let r = self.open(dir, (2 * cap).max(pairs.len()));
                    self.written = Some(r);
                    r
                }
            }
            None => {
                let r = self.open(dir, FIRST_WRITTEN.max(pairs.len()));
                self.written = Some(r);
                r
            }
        };
        let b = dir.block(r).ok_or("a trace's open runs, held by nothing")?;
        let pin = rerail_blocks::region::pin_block(&b, "a trace's open runs", true)?;
        let w = rerail_blocks::layer::RunsW::new(&pin);
        for (row, diff) in pairs {
            w.push(row.as_ref(), *diff).ok_or("a trace's open runs, full")?;
        }
        Ok(())
    }

    /// Its open runs, taken (`None`: nothing written since the last commit): the caller commits them
    /// (`commit_wide`) or lets them go.
    pub fn take_written(&mut self) -> Option<BlockId> {
        self.written.take()
    }

    /// The sum of the diffs its open runs hold: how many rows the run's writes add, net.
    pub fn written_diff(&self, dir: &Directory) -> Result<i64, String> {
        let Some(r) = self.written else { return Ok(0) };
        let l = dir.read("a trace's open runs", &[r])?;
        let runs = Runs::new(l.block(0));
        Ok((0..runs.len()).map(|i| i64::from(runs.diff(i))).sum())
    }

    /// Whether it holds rows written and not committed.
    pub fn is_written(&self) -> bool {
        self.written.is_some()
    }

    /// Each trace's open runs that hold rows committed, in one launch (`commit_wide`); empty ones let go.
    pub fn commit_open(ops: &Runner, dir: &mut Directory, traces: &mut [&mut Trace], tiling: Tiling) -> Result<(), String> {
        let mut items = Vec::with_capacity(traces.len()); // copy: not block data (the traces to commit)
        for t in traces.iter_mut() {
            if let Some(r) = t.take_written() {
                if holds_rows(dir, r)? {
                    items.push((&mut **t, r));
                } else {
                    dir.release(r);
                }
            }
        }
        if items.is_empty() {
            return Ok(());
        }
        Trace::commit_wide(ops, dir, &mut items, tiling)
    }

    /// The first row whose leading columns are `key` in the trace as the run has it now: at its head, then each pair
    /// its open runs hold, in order (a row gone, a row come), read where they lie.
    pub fn now(&self, dir: &Directory, key: &[i64]) -> Result<Option<Row>, String> {
        let mut row = self.probe(dir, self.head, key)?;
        if let Some(r) = self.written {
            let l = dir.read("a trace's open runs", &[r])?;
            let runs = Runs::new(l.block(0));
            let a = self.shape.arity();
            for i in (0..runs.len()).filter(|&i| key.iter().enumerate().all(|(c, &k)| runs.get(i, c) == k)) {
                let x: Row = (0..a).map(|c| runs.get(i, c)).collect();
                if runs.diff(i) > 0 {
                    row = Some(x);
                } else if row == Some(x) {
                    row = None;
                }
            }
        }
        Ok(row)
    }

    /// Rolls the newest `count` layers up into one (`merge`), over the layer below them; the merged generations are let
    /// go, and live on only while a reader holds them.
    pub fn roll_up(&mut self, ops: &Runner, dir: &mut Directory, count: u32, tiling: Tiling) -> Result<BlockId, String> {
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
        let scratch = scratch_slot(dir, rows);
        let mut params = [0u8; MergeP::SIZE];
        MergeP { head: head.0, count, below: below.unwrap_or(NONE), pad: 0 }.write(&mut params);
        let run = ops.run(dir, &MERGE, &[head, out, scratch], &params, tiling);
        dir.release(scratch);
        if let Err(e) = run {
            dir.release(out);
            return Err(e);
        }
        dir.freeze(out)?;
        dir.release(head);
        self.head = Some(out);
        Ok(out)
    }

    /// A run's end, for a trace a run reads as a chain (`RollUp::Never` while it runs: the run's before, `since`, a
    /// layer of it until its delta is read): the layers committed since `since` rolled up into one, then the geometric
    /// policy (`due_to_merge`) over the layers below, as a commit under `RollUp::Geometric` does. The head.
    pub fn roll_up_run(&mut self, ops: &Runner, dir: &mut Directory, since: Option<BlockId>, tiling: Tiling) -> Result<Option<BlockId>, String> {
        let Some(head) = self.head else { return Ok(None) };
        let count = {
            let l = dir.read("a run's roll-up", &[head])?;
            let table = l.table();
            let (mut k, mut id) = (0u32, Some(head.0));
            while let Some(b) = id
                && Some(BlockId(b)) != since
            {
                k += 1;
                id = Layer::new(table.block(b).expect("reached"))?.below().map(|(b, _)| b);
            }
            k
        };
        if count >= 2 {
            self.roll_up(ops, dir, count, tiling)?;
        }
        let head = self.head.expect("rolled up");
        let due = {
            let l = dir.read("a run's roll-up", &[head])?;
            let table = l.table();
            let layer = Layer::new(table.block(head.0).expect("reached"))?;
            let size = layer.n() as u64 + layer.kills().len() as u64;
            rerail_delta_twins::trace::due_to_merge(&table, layer.below().map_or(NONE, |(b, _)| b), size)
        };
        if due >= 2 {
            self.roll_up(ops, dir, due as u32, tiling)?;
        }
        Ok(self.head)
    }

    /// The trace emptied: its head let go (a cold run's outputs start from ∅, B§8). A trace has two states, the
    /// committed one and the run's open layer on it: this drops the committed one.
    pub fn clear(&mut self, dir: &mut Directory) {
        if let Some(h) = self.head.take() {
            dir.release(h);
        }
    }

    /// The digest of the state at `head`: the sum of its layers' digests (each its added rows' digests less its
    /// kills'), so a set has one digest whatever its layers. `None`, the empty state: 0.
    pub fn digest(&self, dir: &Directory, head: Option<BlockId>) -> Result<u64, String> {
        let Some(h) = head else { return Ok(0) };
        let l = dir.read("digest", &[h])?;
        let table = l.table();
        let (mut d, mut id) = (0u64, Some(h.0));
        while let Some(i) = id {
            let layer = Layer::new(table.block(i).ok_or("a layer below, reached by reference")?)?;
            d = d.wrapping_add(layer.head.digest);
            id = layer.below().map(|(b, _)| b);
        }
        Ok(d)
    }

    /// A launch reading the trace's layers from `heads` down: each pinned to read, with the descriptor table of them.
    pub fn read(&self, dir: &Directory, heads: &[Option<BlockId>]) -> Result<Launch, String> {
        dir.read("read", &heads.iter().flatten().copied().collect::<Vec<_>>())
    }

    /// The state at `head`, read where its layers lie, given to `f`: a launch over them held while `f` reads.
    pub fn reading<R>(&self, dir: &Directory, head: Option<BlockId>, f: impl FnOnce(&View<'_>) -> R) -> Result<R, String> {
        let l = self.read(dir, &[head])?;
        let table = l.table();
        Ok(f(&self.state(&table, head)?))
    }

    /// The first row of the state at `head` whose leading columns are `key` (by its layers' indexes), its cells read
    /// where they lie.
    pub fn probe(&self, dir: &Directory, head: Option<BlockId>, key: &[i64]) -> Result<Option<Row>, String> {
        if head.is_none() {
            return Ok(None);
        }
        let cols: [ColumnIndex; MAX_COLS] = std::array::from_fn(|c| ColumnIndex::new(c as u16));
        self.reading(dir, head, |s| Read::matching(s, &cols[..key.len()], key).next().map(|id| Row::of(s, id)))
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

/// One row's cells, read where they lie into a value of at most `MAX_COLS` cells (as a load into registers is): a key
/// to probe by, a row to compare with or to push, never a structure a reader reads instead of the block.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Row {
    n: u8,
    cells: [i64; MAX_COLS],
}

impl Row {
    /// The row of `cells`.
    pub fn new(cells: &[i64]) -> Row {
        cells.iter().copied().collect()
    }

    /// Row `id` of state `s`.
    pub fn of(s: &impl Read, id: RowId) -> Row {
        (0..Read::arity(s)).map(|c| Read::get(s, id, ColumnIndex::new(c as u16))).collect()
    }
}

impl FromIterator<i64> for Row {
    fn from_iter<I: IntoIterator<Item = i64>>(it: I) -> Row {
        let mut r = Row { n: 0, cells: [0; MAX_COLS] };
        for v in it {
            assert!((r.n as usize) < MAX_COLS, "a row of more than {MAX_COLS} cells");
            r.cells[r.n as usize] = v;
            r.n += 1;
        }
        r
    }
}

impl std::ops::Deref for Row {
    type Target = [i64];

    fn deref(&self) -> &[i64] {
        &self.cells[..self.n as usize]
    }
}

impl AsRef<[i64]> for Row {
    fn as_ref(&self) -> &[i64] {
        self
    }
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[cfg(any(test, feature = "test-hooks"))]
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
        let scratch = scratch_slot(dir, rows.len());
        let mut params = vec![0u8; ConsolidateP::SIZE];
        ConsolidateP { below: NONE, pad: 0 }.write(&mut params);
        Case { slots: vec![runs, out, ctl, scratch], params }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
/// A launch's case for the version check of `merge`: a trace of `runs` committed in turn (by the reference
/// consolidate, whole), then its newest `count` layers merged over the layer below them.
pub fn merge_case<'r>(shape: &'r Shape, runs: &'r [Vec<(Vec<i64>, i32)>], count: u32) -> impl Fn(&mut Directory) -> Case + 'r {
    move |dir: &mut Directory| {
        let ops = crate::op::Runner::new().expect("the engine's operations");
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
            heads.push(t.commit(&ops, dir, id, Tiling::WHOLE).expect("a set change"));
        }
        let head = *heads.last().expect("a layer");
        let below = heads.len().checked_sub(count as usize + 1).map(|i| heads[i]);
        let total: usize = runs[runs.len() - count as usize..].iter().map(Vec::len).sum();
        let out = dir.make_room(kind::FACTS, out_bytes(total), layer_room(t.shape(), total));
        if let Some(b) = below {
            dir.refer(out, b, BODY_AT).expect("frozen");
        }
        let scratch = scratch_slot(dir, total);
        let mut params = vec![0u8; MergeP::SIZE];
        MergeP { head: head.0, count, below: below.map_or(NONE, |b| b.0), pad: 0 }.write(&mut params);
        Case { slots: vec![head, out, scratch], params }
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
pub(crate) mod tests;
