//! The operator contract (decision 0686, B3; P§2, P§12 item 7, P§11.3; *0686: Operations* §6.1).
//!
//! Every stage of a run is a **delta operator** (`DeltaOp`): it takes a bistate for every input port, the demand on
//! its outputs and the cache its last run left, and writes its outputs' change and its asks of its inputs, reporting
//! its work (`Work`, P1). Its plain function (`run`) is its meaning: the **reference** (`reference`) runs it on both
//! sides and diffs, and every law is judged against that:
//!
//! - L1, the square: the output's change is the reference's;
//! - L2, a null start: from ∅, the output is ∅ → op(X);
//! - L3, the chain: through a chain of edits, each run's output is a cold run's on that run's inputs;
//! - L4, identity is free: with every input the identity, the output is the identity, the work is the checks alone,
//!   and the cache is returned unchanged;
//! - L5, the cache changes cost, never answers: a run whose cache was dropped gives the same output.
//!
//! The operator's cache is an operator cache (`cache`): a block with P§11.3's header, handed to a run as `CacheW`,
//! whose header is marked open before the run's first write (`cache_open`) and committed with the state the run made
//! (`cache_commit`) by the driver (`Instance`); a run that writes nothing leaves it untouched. A cache read as ∅ (open,
//! or naming another state) gives `None`. The state it names is the digest of its operator's inputs, by content
//! (`Trace::digest`).
//!
//! An operator's outputs are traces it owns (`Instance::outs`), each run's change pushed into a δ runs block a port
//! (`Out`) and committed by `consolidate`. One harness (`harness`) checks every law of any `DeltaOp` over a chain of
//! edits.

use crate::cache::{CacheOps, OpCache, cache_facts};
use crate::dir::{BlockId, Directory};
use crate::op::{Registry, Tiling};
use crate::trace::{RollUp, Trace, TraceOps};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::layer::{Runs, RunsW, Shape};
use rerail_blocks::region::{BlockPin, pin_block};
use rerail_blocks::relation::View;
use rerail_delta_twins::cache as cache_twin;
use std::collections::BTreeSet;

/// P1's counters: the rows an operator's run read, the probes it made and the rows it wrote, and each excess, named
/// (a fallback, a gate violated, levels' returning rows). An identity run's work is the checks alone: none of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub rows_read: u64,
    pub probes: u64,
    pub rows_written: u64,
    pub excess: Vec<(&'static str, u64)>,
}

impl Work {
    /// Whether it did nothing past the checks (L4).
    pub fn is_checks_only(&self) -> bool {
        *self == Work::default()
    }
}

/// One port: its name and the shape of the trace it carries.
pub struct Port {
    pub name: &'static str,
    pub shape: Shape,
}

/// An operator's ports (P§12's `type Ports`): the traces it reads, and those it writes.
pub struct Ports {
    pub ins: Vec<Port>,
    pub outs: Vec<Port>,
}

/// What an operator's run reaches: the registry its operations are declared in, the directory of their blocks, and
/// the trace's and the cache's operations.
pub struct Cx<'c> {
    pub reg: &'c Registry,
    pub dir: &'c mut Directory,
    pub traces: &'c TraceOps,
    pub caches: &'c CacheOps,
}

/// A bistate per input port, read where its layers lie, with the heads it is between (a frozen layer's block id).
pub struct In<'t> {
    pub ports: Vec<Bistate<View<'t>>>,
    pub heads: Vec<(Option<BlockId>, Option<BlockId>)>,
}

impl In<'_> {
    /// Whether every input is the identity, by the layers' counts (L4's check).
    pub fn is_identity(&self) -> bool {
        self.ports.iter().all(|b| b.is_identity())
    }
}

/// A state per input port: the plain function's inputs.
pub struct At<'t> {
    pub ports: Vec<View<'t>>,
    pub heads: Vec<Option<BlockId>>,
}

/// A δ runs block a port (P§12's `LayerW` a port): the run's `(row, ±1)` pushed there, by host writers or by the
/// operations that take the block as a slot.
pub struct Out {
    runs: Vec<BlockId>,
}

impl Out {
    /// No ports: an operator that asks nothing of its inputs.
    pub fn none() -> Out {
        Out { runs: Vec::new() }
    }

    /// Port `port`'s δ runs block.
    pub fn block(&self, port: usize) -> BlockId {
        self.runs[port]
    }

    /// A writer of port `port`, pinned to write while it lives.
    pub fn writer(&self, dir: &Directory, port: usize) -> Result<Writer, String> {
        let b = dir.block(self.runs[port]).ok_or("an output's runs, held by nothing")?;
        Ok(Writer { pin: pin_block(&b, "an operator's output", true)? })
    }
}

/// One output port's writer: pushes `(row, ±1)` into its runs block.
pub struct Writer {
    pin: BlockPin,
}

impl Writer {
    /// Pushes `row` with `diff`: false past the block's room (counted; the driver runs the operator again at the
    /// size it needed).
    pub fn push(&self, row: &[i64], diff: i32) -> bool {
        RunsW::new(&self.pin).push(row, diff).is_some()
    }
}

/// A run's cache (P§12's `CacheW`): read as ∅ unless it holds the state the run starts from; its header marked open
/// before the run's first write.
pub struct CacheW {
    cache: OpCache,
    from: u64,
    begun: bool,
}

impl CacheW {
    /// The cache block to read, when it holds the run's before (or this run has begun writing it): `None` when it
    /// reads as ∅, open or naming another state.
    pub fn read(&self, cx: &Cx<'_>) -> Result<Option<BlockId>, String> {
        if self.begun {
            return Ok(Some(self.cache.0));
        }
        let holds = cache_twin::holds(cx.dir.read("cache", &[self.cache.0])?.block(0), self.from);
        Ok(holds.then_some(self.cache.0))
    }

    /// The cache block to write: on the first write, its header marked open (`cache_open`), reset to ∅ when it did
    /// not hold the run's before.
    pub fn write(&mut self, cx: &mut Cx<'_>) -> Result<BlockId, String> {
        if !self.begun {
            self.cache.open(cx.reg, cx.caches, cx.dir, self.from)?;
            self.begun = true;
        }
        Ok(self.cache.0)
    }

    /// Whether the run wrote it.
    pub fn begun(&self) -> bool {
        self.begun
    }
}

/// A delta operator (P§12 item 7).
pub trait DeltaOp {
    /// The traces it reads, and those it writes.
    fn ports(&self) -> Ports;

    /// L1: equal to `run` on both sides, diffed. P1: its work follows the deltas. `want` is the demand on its outputs
    /// (`None`: all of them); `asks` its demand on its inputs, a runs block an input port (`Out::none()` when it asks
    /// nothing).
    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW, out: &mut Out, asks: &mut Out) -> Result<Work, String>;

    /// The plain function: the reference path, and the tests' oracle.
    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_>, out: &mut Out) -> Result<(), String>;
}

/// An input port's trace and the heads of the run's two states.
pub type InPort<'a> = (&'a Trace, Option<BlockId>, Option<BlockId>);

/// The digest a cache names: its operator's inputs' states, by content, port by port.
fn inputs_digest(dir: &Directory, ports: &[(&Trace, Option<BlockId>)]) -> Result<u64, String> {
    let mut d = 0u64;
    for (i, (t, h)) in ports.iter().enumerate() {
        d = rerail_blocks::layer::mix(d ^ t.digest(dir, *h)?.wrapping_add(i as u64));
    }
    Ok(d)
}

/// The room a port's runs block is first opened with; a run past it runs again at the size it needed.
const FIRST_RUNS: usize = 256;

/// An operator's instance: its output traces and its cache, kept across its runs.
pub struct Instance {
    pub outs: Vec<Trace>,
    pub cache: OpCache,
}

impl Instance {
    pub fn new<O: DeltaOp>(op: &O, dir: &mut Directory) -> Result<Instance, String> {
        let outs = op.ports().outs.iter().map(|p| Trace::new(&p.shape)).collect::<Result<_, _>>()?;
        Ok(Instance { outs, cache: OpCache::new(dir) })
    }

    /// Its outputs' heads.
    pub fn heads(&self) -> Vec<Option<BlockId>> {
        self.outs.iter().map(Trace::head).collect()
    }

    /// Drops its cache: a new one, ∅ (L5: always correct).
    pub fn drop_cache(&mut self, dir: &mut Directory) {
        let old = self.cache;
        self.cache = OpCache::new(dir);
        dir.release(old.0);
    }

    /// One run of `op` over `ins`, each input port's trace between two heads: its change pushed into a runs block a
    /// port, then committed (an empty one is not: the output stays where it was), and the cache committed with the
    /// state the run made when the run wrote it. A run whose outputs outgrew their runs runs again at the size they
    /// needed, from a cache read as ∅ (its first attempt's writes may be half done).
    pub fn step<O: DeltaOp>(&mut self, op: &O, cx: &mut Cx<'_>, ins: &[InPort<'_>]) -> Result<Work, String> {
        let befores: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.1)).collect();
        let afters: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.2)).collect();
        let (from, to) = (inputs_digest(cx.dir, &befores)?, inputs_digest(cx.dir, &afters)?);
        let mut cap = vec![FIRST_RUNS; self.outs.len()];
        loop {
            let runs: Vec<BlockId> = self.outs.iter().zip(&cap).map(|(t, &c)| t.open(cx.dir, c)).collect();
            let mut out = Out { runs };
            // After an attempt that outgrew its runs, a cache it wrote is open: it reads as ∅, and its first write
            // resets it.
            let mut cache = CacheW { cache: self.cache, from, begun: false };
            let work = {
                let heads: Vec<BlockId> = ins.iter().flat_map(|p| [p.1, p.2]).flatten().collect();
                let l = cx.dir.read("an operator's inputs", &heads)?;
                let table = l.table();
                let mut ports = Vec::with_capacity(ins.len());
                for (t, b, a) in ins {
                    let after = t.state(&table, *a)?;
                    ports.push(match b {
                        None => Bistate::cold(after),
                        Some(_) => Bistate::new(t.state(&table, *b)?, after),
                    });
                }
                let input = In { ports, heads: ins.iter().map(|p| (p.1, p.2)).collect() };
                op.delta(cx, &input, None, &mut cache, &mut out, &mut Out::none())
            };
            let work = match work {
                Ok(w) => w,
                Err(e) => {
                    out.runs.iter().for_each(|&r| cx.dir.release(r));
                    return Err(e);
                }
            };
            let mut over = false;
            for (k, &r) in out.runs.iter().enumerate() {
                let l = cx.dir.read("an operator's output", &[r])?;
                let rr = Runs::new(l.block(0));
                if rr.needed() > 0 {
                    cap[k] = rr.len() + rr.needed();
                    over = true;
                }
            }
            if over {
                out.runs.iter().for_each(|&r| cx.dir.release(r));
                continue;
            }
            for (t, &r) in self.outs.iter_mut().zip(&out.runs) {
                let n = {
                    let l = cx.dir.read("an operator's output", &[r])?;
                    Runs::new(l.block(0)).len()
                };
                if n == 0 {
                    cx.dir.release(r);
                } else {
                    t.commit(cx.reg, cx.traces, cx.dir, r, Tiling::WHOLE)?;
                }
            }
            if cache.begun() {
                self.cache.commit(cx.reg, cx.caches, cx.dir, to)?;
            }
            return Ok(work);
        }
    }
}

/// A port's rows: its trace's state at `head`, read where it lies.
pub fn rows_at(dir: &Directory, t: &Trace, head: Option<BlockId>) -> Result<BTreeSet<Vec<i64>>, String> {
    let Some(h) = head else { return Ok(BTreeSet::new()) };
    let l = t.read(dir, &[Some(h)])?;
    let table = l.table();
    let s = t.state(&table, head)?;
    let live = (0..Read::end(&s).get() as u32).map(RowId::new).filter(|&id| Read::live(&s, id));
    Ok(live.map(|id| ColumnIndex::all(Read::arity(&s)).map(|c| Read::get(&s, id, c)).collect()).collect()) // copy: derived (the oracle's rows)
}

/// A port's change, by its rows (the oracle's form).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub added: BTreeSet<Vec<i64>>,
    pub gone: BTreeSet<Vec<i64>>,
}

/// The plain function's output on the inputs' states `heads`: `run` into fresh output traces, committed, read.
pub fn apply<O: DeltaOp>(op: &O, cx: &mut Cx<'_>, ins: &[(&Trace, Option<BlockId>)]) -> Result<Vec<BTreeSet<Vec<i64>>>, String> {
    let ports = op.ports();
    let mut outs: Vec<Trace> = ports.outs.iter().map(|p| Trace::new(&p.shape).map(|t| t.with_roll_up(RollUp::Never))).collect::<Result<_, _>>()?;
    let mut cap = vec![FIRST_RUNS; outs.len()];
    loop {
        let runs: Vec<BlockId> = outs.iter().zip(&cap).map(|(t, &c)| t.open(cx.dir, c)).collect();
        let mut out = Out { runs };
        {
            let heads: Vec<BlockId> = ins.iter().filter_map(|p| p.1).collect();
            let l = cx.dir.read("the reference's inputs", &heads)?;
            let table = l.table();
            let views = ins.iter().map(|(t, h)| t.state(&table, *h)).collect::<Result<Vec<_>, _>>()?;
            op.run(cx, &At { ports: views, heads: ins.iter().map(|p| p.1).collect() }, &mut out)?;
        }
        let mut over = false;
        for (k, &r) in out.runs.iter().enumerate() {
            let l = cx.dir.read("the reference's output", &[r])?;
            let rr = Runs::new(l.block(0));
            if rr.needed() > 0 {
                cap[k] = rr.len() + rr.needed();
                over = true;
            }
        }
        if over {
            out.runs.iter().for_each(|&r| cx.dir.release(r));
            continue;
        }
        let mut rows = Vec::with_capacity(outs.len());
        for (t, &r) in outs.iter_mut().zip(&out.runs) {
            let head = t.commit(cx.reg, cx.traces, cx.dir, r, Tiling::WHOLE)?;
            rows.push(rows_at(cx.dir, t, Some(head))?);
        }
        return Ok(rows);
    }
}

/// The generic reference (P§2, Figure 4): the plain function run on the inputs' before side and on their after side,
/// and the two diffed, port by port.
pub fn reference<O: DeltaOp>(op: &O, cx: &mut Cx<'_>, ins: &[InPort<'_>]) -> Result<Vec<Change>, String> {
    let befores: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.1)).collect();
    let afters: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.2)).collect();
    let (b, a) = (apply(op, cx, &befores)?, apply(op, cx, &afters)?);
    Ok(b.iter().zip(&a).map(|(b, a)| Change { added: a.difference(b).cloned().collect(), gone: b.difference(a).cloned().collect() }).collect())
}

/// A law the harness found broken: which, at which step of the chain, and how.
#[derive(Debug)]
pub struct Broken {
    pub law: &'static str,
    pub step: usize,
    pub what: String,
}

/// One edit chain: for each step, each input port's `(row, ±1)` pairs, a set change of the port's state (an empty one
/// leaves the port's state where it was: the identity).
pub type Chain = Vec<Vec<Vec<(Vec<i64>, i32)>>>;

/// Checks every law of `op` over `chain` (P§12's "one test harness for every operator"): each step run by a threaded
/// instance (its cache carried), by an instance whose cache is dropped at step `drop_at`, and cold, by a fresh
/// instance from ∅; each against the reference. In order, a step's checks are L2 (the cold run is ∅ → op(X)), L1 (the
/// threaded run's change is the reference's), L4 (with every input the identity: the output too, the work the checks
/// alone, the cache unchanged), L5 (the run whose cache was dropped gives the threaded run's output) and L3 (the
/// threaded run's output is the cold run's). The first that fails is returned.
pub fn harness<O: DeltaOp>(op: &O, chain: &Chain, drop_at: usize) -> Result<(), Broken> {
    let fail = |law, step, what: String| Broken { law, step, what };
    let setup = |e: String| fail("setup", 0, e);
    let mut reg = Registry::default();
    let traces = TraceOps::declare(&mut reg).map_err(setup)?;
    let caches = CacheOps::declare(&mut reg).map_err(setup)?;
    let mut dir = Directory::default();
    let ports = op.ports();
    // The inputs: a trace a port, every state kept (a layer a step, none rolled up).
    let mut ins: Vec<Trace> = ports.ins.iter().map(|p| Trace::new(&p.shape).map(|t| t.with_roll_up(RollUp::Never))).collect::<Result<_, _>>().map_err(setup)?;
    let mut heads: Vec<Vec<Option<BlockId>>> = vec![vec![None; ins.len()]];
    for step in chain {
        let mut now = heads.last().cloned().expect("a state");
        for (k, pairs) in step.iter().enumerate() {
            if pairs.is_empty() {
                continue;
            }
            let id = ins[k].open(&mut dir, pairs.len());
            {
                let b = dir.block(id).expect("made");
                let pin = pin_block(&b, "the chain", true).map_err(setup)?;
                let w = RunsW::new(&pin);
                for (r, d) in pairs {
                    w.push(r, *d).ok_or_else(|| setup("room for the chain's pairs".into()))?;
                }
            }
            now[k] = Some(ins[k].commit(&reg, &traces, &mut dir, id, Tiling::WHOLE).map_err(setup)?);
        }
        heads.push(now);
    }
    let mut cx = Cx { reg: &reg, dir: &mut dir, traces: &traces, caches: &caches };
    let at = |k: usize| -> Vec<(&Trace, Option<BlockId>)> { ins.iter().zip(&heads[k]).map(|(t, &h)| (t, h)).collect() };
    let mut want = vec![apply(op, &mut cx, &at(0)).map_err(setup)?];
    let mut threaded = Instance::new(op, cx.dir).map_err(setup)?;
    let mut dropped = Instance::new(op, cx.dir).map_err(setup)?;
    for k in 1..heads.len() {
        let step: Vec<InPort<'_>> = ins.iter().enumerate().map(|(p, t)| (t, heads[k - 1][p], heads[k][p])).collect();
        let cold: Vec<InPort<'_>> = ins.iter().enumerate().map(|(p, t)| (t, None, heads[k][p])).collect();
        want.push(apply(op, &mut cx, &at(k)).map_err(|e| fail("reference", k, e))?);
        // L2: a null start, from ∅ and an empty cache, is ∅ → op(X).
        let mut fresh = Instance::new(op, cx.dir).map_err(setup)?;
        fresh.step(op, &mut cx, &cold).map_err(|e| fail("L2", k, e))?;
        let cold_rows: Vec<_> = fresh.outs.iter().map(|t| rows_at(cx.dir, t, t.head())).collect::<Result<_, _>>().map_err(|e| fail("L2", k, e))?;
        if cold_rows != want[k] {
            return Err(fail("L2", k, format!("from ∅, the output is not op(X): {cold_rows:?}, want {:?}", want[k])));
        }
        // L1: the threaded run's change is the reference's.
        let before = threaded.heads();
        let rows_before: Vec<_> = threaded.outs.iter().map(|t| rows_at(cx.dir, t, t.head())).collect::<Result<_, _>>().map_err(|e| fail("L1", k, e))?;
        let cache_before = {
            let l = cx.dir.read("the harness", &[threaded.cache.0]).map_err(setup)?;
            cache_facts(&l.table(), 0, l.block(0))
        };
        let work = threaded.step(op, &mut cx, &step).map_err(|e| fail("L1", k, e))?;
        let reference = reference(op, &mut cx, &step).map_err(|e| fail("reference", k, e))?;
        for (p, t) in threaded.outs.iter().enumerate() {
            let after = rows_at(cx.dir, t, t.head()).map_err(|e| fail("L1", k, e))?;
            let got = Change { added: after.difference(&rows_before[p]).cloned().collect(), gone: rows_before[p].difference(&after).cloned().collect() };
            if got != reference[p] {
                return Err(fail("L1", k, format!("port {}: the change is {got:?}; the reference's is {:?}", ports.outs[p].name, reference[p])));
            }
        }
        // L4: identity is free.
        if (0..ins.len()).all(|p| heads[k][p] == heads[k - 1][p]) {
            if !work.is_checks_only() {
                return Err(fail("L4", k, format!("every input the identity, and its work is {work:?}")));
            }
            if threaded.heads() != before {
                return Err(fail("L4", k, "every input the identity, and its output moved".into()));
            }
            let cache_after = {
                let l = cx.dir.read("the harness", &[threaded.cache.0]).map_err(setup)?;
                cache_facts(&l.table(), 0, l.block(0))
            };
            if cache_after != cache_before {
                return Err(fail("L4", k, "every input the identity, and its cache changed".into()));
            }
        }
        // L5: a run whose cache was dropped gives the same output.
        if k == drop_at {
            dropped.drop_cache(cx.dir);
        }
        dropped.step(op, &mut cx, &step).map_err(|e| fail("L5", k, e))?;
        for p in 0..threaded.outs.len() {
            let (a, b) = (rows_at(cx.dir, &threaded.outs[p], threaded.outs[p].head()), rows_at(cx.dir, &dropped.outs[p], dropped.outs[p].head()));
            let (a, b) = (a.map_err(|e| fail("L5", k, e))?, b.map_err(|e| fail("L5", k, e))?);
            if a != b {
                return Err(fail("L5", k, format!("port {}: with its cache dropped at step {drop_at}, the output differs", ports.outs[p].name)));
            }
        }
        // L3: the chain's output is a cold run's.
        for p in 0..threaded.outs.len() {
            let a = rows_at(cx.dir, &threaded.outs[p], threaded.outs[p].head()).map_err(|e| fail("L3", k, e))?;
            if a != cold_rows[p] {
                return Err(fail("L3", k, format!("port {}: the chain's output drifted from a cold run's", ports.outs[p].name)));
            }
        }
    }
    Ok(())
}

/// A random edit chain of `steps` over `ports` input ports, each a set change of its port's state: some live rows
/// gone, some new rows added (`row` makes one from a random word), and every fourth step the identity.
pub fn random_chain(ports: usize, steps: usize, seed: u64, row: &dyn Fn(u64) -> Vec<i64>) -> Chain {
    let mut z = seed;
    let mut next = move || {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rerail_blocks::layer::mix(z)
    };
    let mut live: Vec<BTreeSet<Vec<i64>>> = vec![BTreeSet::new(); ports];
    let mut chain = Vec::with_capacity(steps);
    for s in 0..steps {
        let mut step = vec![Vec::new(); ports];
        if s % 4 != 3 {
            for (p, set) in live.iter_mut().enumerate() {
                let gone: Vec<Vec<i64>> = set.iter().filter(|_| next().is_multiple_of(3)).cloned().collect();
                for r in gone {
                    set.remove(&r);
                    step[p].push((r, -1));
                }
                for _ in 0..(next() % 40 + 1) {
                    let r = row(next());
                    if !set.contains(&r) && !step[p].iter().any(|(x, _)| *x == r) {
                        set.insert(r.clone());
                        step[p].push((r, 1));
                    }
                }
            }
        }
        chain.push(step);
    }
    chain
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
