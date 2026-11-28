//! Toy operators, each a delta operator over small traces, and the flawed copies built to break one law each: the
//! harness's and the pipeline driver's subjects (B3), on the CPU's tests and the device's (`rerail_delta_device`).

use crate::cache::OpCache;
use crate::contract::*;
use crate::dir::{BlockId, Directory};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::{ColType, Shape};
use rerail_blocks::relation::View;
use std::collections::BTreeSet;

/// What a toy gets wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flaw {
    /// Nothing: the good toy.
    None,
    /// Drops the rows gone from its input: breaks the square (L1), not a null start.
    Kills,
    /// Writes a row of its own on a cold run: breaks the null start (L2).
    Cold,
    /// Reads its whole input when nothing changed: breaks "identity is free" (L4), its answers right.
    Busy,
    /// Drops the rows gone when its cache reads as ∅: its answer hangs on its cache (L5).
    Cache,
    /// Under a want, writes a row of its own on a cold run, where nothing is wanted: past its want, a row the full
    /// output lacks (D1).
    Past,
}

/// `out(b, a) :- in(a, b), a % 3 != 0`, its added rows read in the order its cache keeps of its input's newest layer.
pub struct Toy(pub Flaw);

pub fn keep(a: i64) -> bool {
    a % 3 != 0
}

pub fn in_shape() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], None)
}

pub fn out_shape() -> Shape {
    Shape::run(vec![ColType::I64, ColType::I32], vec![0, 1], None)
}

pub fn cell(s: &impl Read, id: RowId, c: u16) -> i64 {
    s.get(id, ColumnIndex::new(c))
}

pub fn live_ids(s: &impl Read) -> impl Iterator<Item = RowId> + '_ {
    (0..s.end().get()).map(RowId::new).filter(|&id| s.live(id))
}

/// The toys' port set: one input, one output.
pub struct ToyPorts;

pub const IN: PortIn<ToyPorts> = PortIn::new(0);
pub const OUT: PortOut<ToyPorts> = PortOut::new(0);

impl PortSet for ToyPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "in", shape: in_shape() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: out_shape() }] }
    }
}

impl DeltaOp for Toy {
    type Ports = ToyPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, ToyPorts>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<crate::cache::Orders>, out: &mut Out<ToyPorts>, _: &mut Asks<ToyPorts>) -> Result<Work, String> {
        let x = ins.port(IN);
        let mut work = Work::default();
        if ins.is_identity() {
            if self.0 == Flaw::Busy {
                work.rows_read = live_ids(x.after()).count() as u64;
            }
            return Ok(work);
        }
        let held = cache.read(cx)?.is_some();
        let w = out.writer(cx.dir, OUT)?;
        let skip_gone = self.0 == Flaw::Kills || (self.0 == Flaw::Cache && !held && x.before().is_some());
        if let Some(b) = x.before().filter(|_| !skip_gone) {
            for id in x.gone() {
                work.rows_read += 1;
                let a = cell(b, id, 0);
                if keep(a) {
                    w.push(&[cell(b, id, 1), a], -1);
                    work.rows_written += 1;
                }
            }
        }
        if self.0 == Flaw::Cold && x.before().is_none() {
            w.push(&[-1, -1], 1);
        }
        if self.0 == Flaw::Past && want.is_some() && x.before().is_none() {
            w.push(&[-2, -2], 1);
        }
        // The added rows, each layer's between the two states read in its order by the second column, from the
        // cache: made once a layer, and kept while the cache holds its state.
        let added = x.added().count();
        let mut layers = Vec::new();
        {
            let (before, after) = ins.heads(IN);
            let heads: Vec<BlockId> = after.into_iter().collect();
            let l = cx.dir.read("the toy's layers", &heads)?;
            let table = l.table();
            let mut at = after.map(|h| h.0);
            while let Some(id) = at.filter(|&id| Some(BlockId(id)) != before) {
                layers.push(BlockId(id));
                at = rerail_blocks::layer::Layer::new(table.block(id).ok_or("a layer below")?)?.below().map(|(b, _)| b);
            }
        }
        if layers.is_empty() {
            return Ok(work);
        }
        let c = cache.write(cx)?;
        let a = x.after();
        let mut seen = 0;
        for layer in layers {
            OpCache(c).order(cx.ops, cx.dir, layer, &[1])?;
            let read = cache.body(cx, |orders| -> Result<(), String> {
                let order = orders.order(layer, &[1]).ok_or("the order just added")?;
                for &id in order.ids {
                    let id = RowId::new(id);
                    if !Read::live(a, id) || x.before().is_some_and(|b| Read::live(b, id)) {
                        continue;
                    }
                    seen += 1;
                    work.probes += 1;
                    if keep(cell(a, id, 0)) {
                        w.push(&[cell(a, id, 1), cell(a, id, 0)], 1);
                        work.rows_written += 1;
                    }
                }
                Ok(())
            })?;
            read.ok_or("the cache this run writes, read as ∅")??;
        }
        if seen != added {
            return Err(format!("the newest layer's order holds {seen} of the {added} rows added"));
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, ToyPorts>, out: &mut Out<ToyPorts>) -> Result<(), String> {
        let x = ins.port(IN);
        let w = out.writer(cx.dir, OUT)?;
        for id in live_ids(x) {
            let a = cell(x, id, 0);
            if keep(a) {
                w.push(&[cell(x, id, 1), a], 1);
            }
        }
        Ok(())
    }
}

pub fn row(w: u64) -> Vec<i64> {
    vec![(w % 40) as i64, ((w >> 8) % 1000) as i64]
}

/// `out(p, v) :- in(p, v)` per partition `p`, and three flawed copies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// Nothing: the good toy.
    None,
    /// Writes a row of partition 99 at each warm step, a partition its input's change never names (L9, checked in the
    /// validation mode as it runs).
    Other,
    /// Its plain function also gives each row's partition the count of every partition's rows: it reads across
    /// partitions, though its ports' types allow it (L9 by meaning, `by_partition`).
    Leak,
    /// Reads every live row of its input on each warm step, its answer right: its work grows with the partitions an
    /// edit leaves alone (L7's work).
    Scan,
}

pub struct PerPart(pub Part);

pub struct PerPartPorts;

pub fn parted() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I32], vec![0, 1], Some(0))
}

impl PortSet for PerPartPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;
    const PER_PARTITION: bool = true;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "in", shape: parted() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: parted() }] }
    }
}

impl DeltaOp for PerPart {
    type Ports = PerPartPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, PerPartPorts>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<PerPartPorts>, _: &mut Asks<PerPartPorts>) -> Result<Work, String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        if let Some(b) = x.before() {
            for id in x.gone() {
                w.push(&[cell(b, id, 0), cell(b, id, 1)], -1);
            }
        }
        for id in x.added() {
            w.push(&[cell(x.after(), id, 0), cell(x.after(), id, 1)], 1);
        }
        if self.0 == Part::Other && x.before().is_some() {
            w.push(&[99, Read::end(x.after()).get() as i64], 1);
        }
        if self.0 == Part::Scan && x.before().is_some() {
            std::hint::black_box(live_ids(x.after()).map(|id| cell(x.after(), id, 1)).sum::<i64>());
        }
        Ok(Work::default())
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, PerPartPorts>, out: &mut Out<PerPartPorts>) -> Result<(), String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        let n = live_ids(x).count() as i64;
        let mut parts = BTreeSet::new();
        for id in live_ids(x) {
            w.push(&[cell(x, id, 0), cell(x, id, 1)], 1);
            if self.0 == Part::Leak && parts.insert(cell(x, id, 0)) {
                w.push(&[cell(x, id, 0), -n], 1);
            }
        }
        Ok(())
    }
}

pub fn part_row(w: u64) -> Vec<i64> {
    vec![(w % 5) as i64, ((w >> 8) % 1000) as i64]
}

/// `out(a, b) :- in(b, a), b % 2 == 0`: Toy's output read back into Toy's input shape.
pub struct Back;

pub struct BackPorts;

impl PortSet for BackPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "in", shape: out_shape() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: in_shape() }] }
    }
}

impl DeltaOp for Back {
    type Ports = BackPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, BackPorts>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<BackPorts>, _: &mut Asks<BackPorts>) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        if let Some(b) = x.before() {
            for id in x.gone() {
                work.rows_read += 1;
                if cell(b, id, 0) % 2 == 0 {
                    w.push(&[cell(b, id, 1), cell(b, id, 0)], -1);
                }
            }
        }
        for id in x.added() {
            work.rows_read += 1;
            if cell(x.after(), id, 0) % 2 == 0 {
                w.push(&[cell(x.after(), id, 1), cell(x.after(), id, 0)], 1);
            }
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, BackPorts>, out: &mut Out<BackPorts>) -> Result<(), String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        for id in live_ids(x).filter(|&id| cell(x, id, 0) % 2 == 0) {
            w.push(&[cell(x, id, 1), cell(x, id, 0)], 1);
        }
        Ok(())
    }
}

/// `out(0, a, b) :- l(a, b)`, `out(1, a, b) :- r(a, b)`: two inputs of Toy's input shape, tagged into one.
pub struct Union2;

pub struct Union2Ports;

impl PortSet for Union2Ports {
    const INS: usize = 2;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;

    fn ports() -> Ports {
        let out = Shape::run(vec![ColType::I32, ColType::I32, ColType::I64], vec![0, 1, 2], None);
        Ports { ins: vec![Port { name: "l", shape: in_shape() }, Port { name: "r", shape: in_shape() }], globals: Vec::new(), outs: vec![Port { name: "out", shape: out }] }
    }
}

impl DeltaOp for Union2 {
    type Ports = Union2Ports;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, Union2Ports>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<Union2Ports>, _: &mut Asks<Union2Ports>) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let w = out.writer(cx.dir, PortOut::new(0))?;
        for tag in 0..2 {
            let x = ins.port(PortIn::new(tag));
            if let Some(b) = x.before() {
                for id in x.gone() {
                    work.rows_read += 1;
                    w.push(&[tag as i64, cell(b, id, 0), cell(b, id, 1)], -1);
                }
            }
            for id in x.added() {
                work.rows_read += 1;
                w.push(&[tag as i64, cell(x.after(), id, 0), cell(x.after(), id, 1)], 1);
            }
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, Union2Ports>, out: &mut Out<Union2Ports>) -> Result<(), String> {
        let w = out.writer(cx.dir, PortOut::new(0))?;
        for tag in 0..2 {
            let x = ins.port(PortIn::new(tag));
            for id in live_ids(x) {
                w.push(&[tag as i64, cell(x, id, 0), cell(x, id, 1)], 1);
            }
        }
        Ok(())
    }
}

/// An edit chain of Toy's input whose every fourth step (from the third) changes only rows Toy drops (`a % 3 == 0`),
/// so the stages after Toy see the identity, and every fourth (from the fourth) is the identity itself.
pub fn piped_chain(seed: u64, steps: usize) -> Chain {
    let mut z = seed;
    let mut next = move || {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rerail_blocks::layer::mix(z)
    };
    let mut live: BTreeSet<Vec<i64>> = BTreeSet::new();
    let mut chain = Vec::with_capacity(steps);
    for s in 0..steps {
        let mut step: Vec<(Vec<i64>, i32)> = Vec::new();
        let dropped_only = s % 4 == 2;
        if s % 4 != 3 {
            let gone: Vec<Vec<i64>> = live.iter().filter(|r| (!dropped_only || r[0] % 3 == 0) && next().is_multiple_of(3)).cloned().collect();
            for r in gone {
                live.remove(&r);
                step.push((r, -1));
            }
            for _ in 0..(next() % 20 + 4) {
                let mut r = row(next());
                if dropped_only {
                    r[0] = r[0] / 3 * 3;
                }
                if !live.contains(&r) && !step.iter().any(|(x, _)| *x == r) {
                    live.insert(r.clone());
                    step.push((r, 1));
                }
            }
        }
        chain.push(vec![step]);
    }
    chain
}

/// Runs the pipeline of `stages` wired by `wires` over a piped chain of one input, on `bench`.
pub fn piped(bench: &mut Bench, seed: u64, make: &dyn Fn(&mut Directory) -> Vec<Box<dyn Stage + 'static>>, wires: &[Vec<Wire>]) -> Result<Piped, Broken> {
    let (ins, heads) = inputs_of(bench, &[in_shape()], &piped_chain(seed, 12)).map_err(|e| Broken { law: "setup", step: 0, what: e })?;
    let mut stages = make(&mut bench.dir);
    let (globals, gwires) = (vec![Vec::new(); heads.len()], vec![Vec::new(); stages.len()]);
    pipeline_laws(bench, &ins, &heads, &globals, &mut stages, wires, &gwires)
}

pub static TOY: Toy = Toy(Flaw::None);
pub static BUSY: Toy = Toy(Flaw::Busy);
pub static KILLS: Toy = Toy(Flaw::Kills);

pub fn staged<O: DeltaOp>(op: &'static O, dir: &mut Directory) -> Box<dyn Stage + 'static> {
    Box::new(Staged::new(op, dir).unwrap())
}


/// `heads(m, h) :- k_rule(m, _, h)`: each module's rules' heads, L1's first output projected past its rule ids (the
/// local column), per module. A stage after L1 in a pipeline of the engine's own operator (`laws::l1_pipeline`). A
/// head is retracted when its module's last rule of it goes, added when its first comes.
pub struct Heads;

pub struct HeadsPorts;

impl PortSet for HeadsPorts {
    const INS: usize = 1;
    const GLOBALS: usize = 0;
    const OUTS: usize = 1;
    const PER_PARTITION: bool = true;

    fn ports() -> Ports {
        Ports { ins: vec![Port { name: "k_rule", shape: crate::kform::shape(0) }], globals: Vec::new(), outs: vec![Port { name: "heads", shape: parted() }] }
    }
}

/// The first live row of `s` whose module and head are `(m, h)`.
fn first_of(s: &impl Read, m: i64, h: i64) -> Option<RowId> {
    Read::matching(s, &[ColumnIndex::new(0), ColumnIndex::new(2)], &[m, h]).next()
}

impl DeltaOp for Heads {
    type Ports = HeadsPorts;
    type Cache = crate::cache::Orders;

    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, HeadsPorts>, _: Option<&Bistate<View<'_>>>, _: &mut CacheW<crate::cache::Orders>, out: &mut Out<HeadsPorts>, _: &mut Asks<HeadsPorts>) -> Result<Work, String> {
        let mut work = Work::default();
        if ins.is_identity() {
            return Ok(work);
        }
        let x = ins.port(PortIn::new(0));
        let a = x.after();
        let w = out.writer(cx.dir, PortOut::new(0))?;
        if let Some(b) = x.before() {
            for id in x.gone() {
                work.rows_read += 1;
                let (m, h) = (cell(b, id, 0), cell(b, id, 2));
                // Once a head: at its first row before, when no row after holds it.
                if first_of(b, m, h) == Some(id) && first_of(a, m, h).is_none() {
                    w.push(&[m, h], -1);
                }
            }
        }
        for id in x.added() {
            work.rows_read += 1;
            let (m, h) = (cell(a, id, 0), cell(a, id, 2));
            if first_of(a, m, h) == Some(id) && x.before().is_none_or(|b| first_of(b, m, h).is_none()) {
                w.push(&[m, h], 1);
            }
        }
        Ok(work)
    }

    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, HeadsPorts>, out: &mut Out<HeadsPorts>) -> Result<(), String> {
        let x = ins.port(PortIn::new(0));
        let w = out.writer(cx.dir, PortOut::new(0))?;
        let mut seen = BTreeSet::new(); // copy: derived (the heads given, the plain function's)
        for id in live_ids(x) {
            let (m, h) = (cell(x, id, 0), cell(x, id, 2));
            if seen.insert((m, h)) {
                w.push(&[m, h], 1);
            }
        }
        Ok(())
    }
}

/// L9 as it runs (`Instance::changed_within`), on `bench` in the validation mode: the per-partition toy (`flawed`, the
/// one that writes a row of partition 99) stepped over a cold layer, then an edit of partition 2. The warm step's
/// refusal, or none.
pub fn l9_as_it_runs(bench: &mut Bench, flawed: bool) -> Result<(), String> {
    use crate::op::Tiling;
    use crate::trace::{RollUp, Trace};
    bench.dir.validate(true);
    let mut t = Trace::new(&parted())?.with_roll_up(RollUp::Never);
    let op = PerPart(if flawed { Part::Other } else { Part::None });
    let mut inst = Instance::new(&op, &mut bench.dir)?;
    t.write(&bench.ops, &mut bench.dir, &[(vec![1, 10], 1), (vec![2, 20], 1), (vec![3, 30], 1)])?;
    Trace::commit_open(&bench.ops, &mut bench.dir, &mut [&mut t], Tiling::WHOLE)?;
    let h1 = t.head();
    inst.step(&op, &mut bench.cx(), &[(&t, None, h1)], &[], None)?;
    t.write(&bench.ops, &mut bench.dir, &[(vec![2, 20], -1), (vec![2, 21], 1)])?;
    Trace::commit_open(&bench.ops, &mut bench.dir, &mut [&mut t], Tiling::WHOLE)?;
    let ran = inst.step(&op, &mut bench.cx(), &[(&t, h1, t.head())], &[], None);
    bench.dir.validate(false);
    ran.map(drop)
}
