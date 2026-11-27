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
//! Under demand (P§7.1), the harness checks D1 as well: demand restricts, never changes. A run under a want gives,
//! on each side, the full output restricted to that side's want, and no row the full output lacks. An operator may
//! produce more at its grain, which is over-demand. Asking for more (every output) changes nothing asked. The inputs
//! restricted to what the operator asks of them give the same wanted output. The instance keeps its want and its
//! requests as traces (`Instance::want`, `Instance::asks`): the demand in and the demand out of Figure 7.
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

use crate::cache::{CacheOps, OpCache};
#[cfg(any(test, feature = "test-hooks"))]
use crate::cache::cache_facts;
use std::marker::PhantomData;
use crate::dir::{BlockId, Directory};
use crate::files::BlobRows;
use crate::op::{Registry, Tiling};
use crate::trace::{RollUp, Trace, TraceOps};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read};
use rerail_blocks::layer::{ColType, Runs, RunsW, Shape};
use rerail_blocks::region::{BlockPin, pin_block};
use rerail_blocks::relation::View;
use rerail_delta_twins::cache as cache_twin;
#[cfg(any(test, feature = "test-hooks"))]
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

impl std::ops::AddAssign for Work {
    fn add_assign(&mut self, w: Work) {
        self.rows_read += w.rows_read;
        self.probes += w.probes;
        self.rows_written += w.rows_written;
        for (name, n) in w.excess {
            match self.excess.iter_mut().find(|e| e.0 == name) {
                Some(e) => e.1 += n,
                None => self.excess.push((name, n)),
            }
        }
    }
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

/// An operator's ports, declared: the traces it reads, the program globals it reads as facts in (B§4), and the
/// traces it writes.
pub struct Ports {
    pub ins: Vec<Port>,
    pub globals: Vec<&'static str>,
    pub outs: Vec<Port>,
}

/// An operator's port set (P§12's `type Ports`): its ports, declared, and typed handles on them (`PortIn`,
/// `PortGlobal`, `PortOut`), so an operator reads and writes its own ports only: a handle of one port set is no
/// index of another's `In`, `At` or `Out`.
pub trait PortSet {
    /// Its input ports', globals' and output ports' counts.
    const INS: usize;
    const GLOBALS: usize;
    const OUTS: usize;
    /// Its ports: their names and shapes, `INS`, `GLOBALS` and `OUTS` of them.
    fn ports() -> Ports;
    /// The input ports it writes demand on (P§7.1's demand out, `Asks`): each is asked by key, its partition's, and
    /// the driver answers only what is asked. An input port not listed is wanted whole.
    const ASKED: &'static [usize] = &[];
}

macro_rules! port_handle {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        pub struct $name<P>(usize, PhantomData<fn() -> P>);

        impl<P> $name<P> {
            pub const fn new(i: usize) -> Self {
                $name(i, PhantomData)
            }
            pub fn index(self) -> usize {
                self.0
            }
        }

        impl<P> Clone for $name<P> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<P> Copy for $name<P> {}
    };
}

port_handle!(
    /// Input port `i` of port set `P`.
    PortIn
);
port_handle!(
    /// Global `i` of port set `P`.
    PortGlobal
);
port_handle!(
    /// Output port `i` of port set `P`.
    PortOut
);

/// Every output port of port set `P`, in order.
pub fn outs<P: PortSet>() -> impl Iterator<Item = PortOut<P>> {
    (0..P::OUTS).map(PortOut::new)
}

/// An operator's cache body (P§12's `type Cache`, `Cache<T>`'s `T`): what it keeps in its cache block, read through
/// its own type (`CacheW::body`). The block and its header are the operator cache's (`cache`); `()` keeps nothing.
pub trait CacheBody {
    type Read<'a>;
    fn read(bytes: &[u8]) -> Self::Read<'_>;
}

impl CacheBody for () {
    type Read<'a> = ();
    fn read(_: &[u8]) -> Self::Read<'_> {}
}

/// A program global at one state (B§4's facts in: "only facts in which are program globals"): a block every level
/// shares, append-only (P§13: the atoms, a trace with no kills), read in place by the operator's operations; its
/// state is the count it holds, its content up to that count fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Global {
    pub block: BlockId,
    pub count: u64,
}

/// A global port between two states (`None`: a cold run).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalIn {
    pub block: BlockId,
    pub before: Option<u64>,
    pub after: u64,
}

impl GlobalIn {
    pub fn is_identity(&self) -> bool {
        self.before == Some(self.after)
    }

    fn side(&self, after: bool) -> Global {
        Global { block: self.block, count: if after { self.after } else { self.before.unwrap_or(0) } }
    }
}

/// What an operator's run reaches: the registry its operations are declared in, the directory of their blocks, the
/// blobs (P§11: with traces, the two kinds of stored thing; a row names a blob by its id), and the trace's and the
/// cache's operations.
pub struct Cx<'c> {
    pub reg: &'c Registry,
    pub dir: &'c mut Directory,
    pub blobs: &'c BlobRows,
    pub traces: &'c TraceOps,
    pub caches: &'c CacheOps,
}

/// A bistate per input port of `P`, read where its layers lie, with the heads it is between (a frozen layer's block
/// id), and each global between two states.
pub struct In<'t, P> {
    ports: Vec<Bistate<View<'t>>>,
    heads: Vec<(Option<BlockId>, Option<BlockId>)>,
    globals: Vec<GlobalIn>,
    of: PhantomData<fn() -> P>,
}

impl<'t, P> In<'t, P> {
    /// Input port `p`'s bistate.
    pub fn port(&self, p: PortIn<P>) -> &Bistate<View<'t>> {
        &self.ports[p.0]
    }

    /// The heads input port `p`'s bistate is between.
    pub fn heads(&self, p: PortIn<P>) -> (Option<BlockId>, Option<BlockId>) {
        self.heads[p.0]
    }

    /// Global `g` between the run's two states.
    pub fn global(&self, g: PortGlobal<P>) -> GlobalIn {
        self.globals[g.0]
    }

    /// Whether every input is the identity, by the layers' counts and the globals' (L4's check).
    pub fn is_identity(&self) -> bool {
        self.ports.iter().all(|b| b.is_identity()) && self.globals.iter().all(GlobalIn::is_identity)
    }
}

/// A state per input port of `P` and per global: the plain function's inputs.
pub struct At<'t, P> {
    ports: Vec<View<'t>>,
    globals: Vec<Global>,
    of: PhantomData<fn() -> P>,
}

impl<'t, P> At<'t, P> {
    /// Input port `p`'s state.
    pub fn port(&self, p: PortIn<P>) -> &View<'t> {
        &self.ports[p.0]
    }

    /// Global `g` at its state.
    pub fn global(&self, g: PortGlobal<P>) -> Global {
        self.globals[g.0]
    }
}

/// A δ runs block a port (P§12's `LayerW` a port): the run's `(row, ±1)` pushed there, by host writers or by the
/// operations that take the block as a slot; and each port's kept state, the run's before (P§3: "the file's last
/// result is its output trace's before"), read where it lies.
pub struct Out<P> {
    runs: Vec<BlockId>,
    kept: Vec<Option<BlockId>>,
    of: PhantomData<fn() -> P>,
}

/// An operator's asks of its inputs (P§12's `asks`, P§7.1's demand out): a δ runs block per input port it asks of
/// (`PortSet::ASKED`), the change of its demand on that port, `(key, ±1)` (`asks_shape`): a key newly needed +1, one
/// no longer needed −1. The driver keeps them as the instance's requests traces (`Instance::asks`) and answers what
/// they hold.
pub struct Asks<P> {
    runs: Vec<Option<BlockId>>,
    of: PhantomData<fn() -> P>,
}

impl<P> Asks<P> {
    /// No asks: an operator run outside a driver that answers them (the reference's).
    pub fn none() -> Asks<P> {
        Asks { runs: Vec::new(), of: PhantomData }
    }

    /// Whether it asks of no port.
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(Option::is_none)
    }

    /// A writer of its asks of input port `port`, pinned to write while it lives: refused for a port its port set
    /// does not ask of.
    pub fn writer(&self, dir: &Directory, port: PortIn<P>) -> Result<Writer, String> {
        let r = self.runs.get(port.0).copied().flatten().ok_or_else(|| format!("input port {} is not asked of (PortSet::ASKED)", port.0))?;
        let b = dir.block(r).ok_or("an ask's runs, held by nothing")?;
        Ok(Writer { pin: pin_block(&b, "an operator's asks", true)? })
    }
}

/// The want trace's rows (P§7.1's demand in; P§13: "Productions, demand: Trace (wanted relation, key)"): `(port,
/// key)`, output port `port` wanted at `key`, its partition's (a port with no partition is wanted whole by any row of
/// it). Keys are content and symbols, never node ids (P§7.2): a document's is its key from its path's hash, in 32
/// bits as every partition's is.
pub fn want_shape() -> Shape {
    Shape::run(vec![ColType::I32, ColType::I32], vec![1, 0], Some(1))
}

/// A want as its maker names it (P§7.1's demand in: `(port, key)` rows), without gathering it: each of its rows, once,
/// and whether it holds a row. `Instance::set_want` writes its change against the want trace into a runs block.
pub trait WantRows {
    fn each(&self, f: &mut dyn FnMut(usize, i64));
    fn holds(&self, port: usize, key: i64) -> bool;
}

/// Every one of `ports` output ports wanted at `key` (a run's productions: every relation of one document).
#[derive(Clone, Copy, Debug)]
pub struct AtKey {
    pub ports: usize,
    pub key: i64,
}

impl WantRows for AtKey {
    fn each(&self, f: &mut dyn FnMut(usize, i64)) {
        (0..self.ports).for_each(|p| f(p, self.key));
    }

    fn holds(&self, port: usize, key: i64) -> bool {
        port < self.ports && key == self.key
    }
}

/// A test's want, a set of its rows.
#[cfg(any(test, feature = "test-hooks"))]
impl WantRows for BTreeSet<(usize, i64)> {
    fn each(&self, f: &mut dyn FnMut(usize, i64)) {
        self.iter().for_each(|&(p, k)| f(p, k));
    }

    fn holds(&self, port: usize, key: i64) -> bool {
        self.contains(&(port, key))
    }
}

/// An ask's rows on an input port (P§7.1's demand out): `(key)`, the port's partition key it needs.
pub fn asks_shape() -> Shape {
    Shape::run(vec![ColType::I32], vec![0], Some(0))
}

#[cfg(any(test, feature = "test-hooks"))]
/// Rows `rows` of output port `port` (its shape `shape`) restricted to `want` (P§7.1, D1): those whose partition key
/// is wanted on the port; a port with no partition, whole when any key of it is wanted.
pub fn restrict(rows: &BTreeSet<Vec<i64>>, port: usize, shape: &Shape, want: &BTreeSet<(usize, i64)>) -> BTreeSet<Vec<i64>> {
    match shape.partition {
        None if want.iter().any(|w| w.0 == port) => rows.clone(),
        None => BTreeSet::new(),
        Some(c) => rows.iter().filter(|r| want.contains(&(port, r[c as usize]))).cloned().collect(),
    }
}

impl<P> Out<P> {
    /// Output port `port`'s δ runs block.
    pub fn block(&self, port: PortOut<P>) -> BlockId {
        self.runs[port.0]
    }

    /// Output port `port`'s kept state: its trace's head before the run (`None`: ∅, as a cold run's and `run`'s are).
    pub fn kept(&self, port: PortOut<P>) -> Option<BlockId> {
        self.kept[port.0]
    }

    /// Port `port`'s runs with room for `cap` rows: opened again at that room when it has less and nothing is pushed
    /// yet. An operator that knows the room its run needs (its cache's) asks for it before its first write, and the
    /// run does not outgrow its runs and run again.
    pub fn room(&mut self, dir: &mut Directory, port: PortOut<P>, cap: usize) -> Result<(), String> {
        let (port, r) = (port.0, self.runs[port.0]);
        let h = {
            let l = dir.read("room", &[r])?;
            Runs::new(l.block(0)).head
        };
        if h.cap as usize >= cap {
            return Ok(());
        }
        if h.claimed != 0 {
            return Err(format!("port {port}'s room asked after a write"));
        }
        self.runs[port] = crate::trace::open_runs(dir, &h.shape, cap);
        dir.release(r);
        Ok(())
    }

    /// A writer of port `port`, pinned to write while it lives.
    pub fn writer(&self, dir: &Directory, port: PortOut<P>) -> Result<Writer, String> {
        let b = dir.block(self.runs[port.0]).ok_or("an output's runs, held by nothing")?;
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

/// A run's cache (P§12's `CacheW<T>`), its body `C`: read as ∅ unless it holds the state the run starts from; its
/// header marked open before the run's first write.
pub struct CacheW<C> {
    cache: OpCache,
    from: u64,
    begun: bool,
    of: PhantomData<fn() -> C>,
}

impl<C: CacheBody> CacheW<C> {
    /// The cache `cache` for a run from state `from`.
    pub(crate) fn new(cache: OpCache, from: u64) -> CacheW<C> {
        CacheW { cache, from, begun: false, of: PhantomData }
    }

    /// `f` of its body, read where it lies, when it holds the run's before (or this run has begun writing it):
    /// `None` when it reads as ∅.
    pub fn body<R>(&self, cx: &Cx<'_>, f: impl FnOnce(C::Read<'_>) -> R) -> Result<Option<R>, String> {
        let Some(c) = self.read(cx)? else { return Ok(None) };
        let l = cx.dir.read("an operator's cache", &[c])?;
        Ok(Some(f(C::read(l.block(0)))))
    }

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
    /// Which traces it reads and writes, and which globals it reads (P§12's `type Ports`).
    type Ports: PortSet;
    /// What its cache keeps (P§12's `type Cache`).
    type Cache: CacheBody;

    /// L1: equal to `run` on both sides, diffed. P1: its work follows the deltas. `want` is the demand on its outputs
    /// (`None`: all of them); `asks` its demand on its inputs.
    fn delta(&self, cx: &mut Cx<'_>, ins: &In<'_, Self::Ports>, want: Option<&Bistate<View<'_>>>, cache: &mut CacheW<Self::Cache>, out: &mut Out<Self::Ports>, asks: &mut Asks<Self::Ports>) -> Result<Work, String>;

    /// The plain function: the reference path, and the tests' oracle.
    fn run(&self, cx: &mut Cx<'_>, ins: &At<'_, Self::Ports>, out: &mut Out<Self::Ports>) -> Result<(), String>;
}

/// Operator `O`'s ports, declared, checked against its port set's counts.
fn ports_of<O: DeltaOp>() -> Result<Ports, String> {
    let p = O::Ports::ports();
    if (p.ins.len(), p.globals.len(), p.outs.len()) != (O::Ports::INS, O::Ports::GLOBALS, O::Ports::OUTS) {
        return Err("a port set's ports are not its counts".into());
    }
    Ok(p)
}

/// An input port's trace and the heads of the run's two states.
pub type InPort<'a> = (&'a Trace, Option<BlockId>, Option<BlockId>);

/// The digest a cache names: its operator's inputs' states, by content, port by port, then each global's count (an
/// append-only block's content to its count is fixed).
fn inputs_digest(dir: &Directory, ports: &[(&Trace, Option<BlockId>)], globals: &[Global]) -> Result<u64, String> {
    let mut d = 0u64;
    for (i, (t, h)) in ports.iter().enumerate() {
        d = rerail_blocks::layer::mix(d ^ t.digest(dir, *h)?.wrapping_add(i as u64));
    }
    for (i, g) in globals.iter().enumerate() {
        d = rerail_blocks::layer::mix(d ^ g.count.wrapping_add((ports.len() + i) as u64));
    }
    Ok(d)
}

/// The room a port's runs block is first opened with; a run past it runs again at the size it needed.
const FIRST_RUNS: usize = 256;

/// The excess of a run that outgrew its runs: its attempts past the first.
pub const OUTGROWN: &str = "attempts past the first, the runs outgrown";

/// An operator's instance: its output traces and its cache, kept across its runs, and its two demand ports' traces
/// (P§7.1, Figure 7): the want on its outputs (demand in, `want_shape`) and its requests of each input it asks of
/// (demand out, the asks it wrote, `asks_shape`).
pub struct Instance {
    pub outs: Vec<Trace>,
    pub cache: OpCache,
    pub want: Trace,
    pub asks: Vec<Option<Trace>>,
}

/// The want a run is under: its want trace between two heads (`Instance::set_want`), or `None`, every output wanted.
pub type WantIn = Option<(Option<BlockId>, Option<BlockId>)>;

impl Instance {
    pub fn new<O: DeltaOp>(_: &O, dir: &mut Directory) -> Result<Instance, String> {
        let ports = ports_of::<O>()?;
        let outs = ports.outs.iter().map(|p| Trace::new(&p.shape)).collect::<Result<_, _>>()?;
        let asks = (0..ports.ins.len()).map(|p| O::Ports::ASKED.contains(&p).then(|| Trace::new(&asks_shape())).transpose()).collect::<Result<_, _>>()?;
        Ok(Instance { outs, cache: OpCache::new(dir), want: Self::want_trace()?, asks })
    }

    /// The want trace: no roll-up while a run reads it (its before is a layer in its chain); rolled up at the run's end
    /// (`roll_up_want`).
    fn want_trace() -> Result<Trace, String> {
        Ok(Trace::new(&want_shape())?.with_roll_up(RollUp::Never))
    }

    /// Its want's layers since `since` rolled up (a run's end: its before is no longer read).
    pub fn roll_up_want(&mut self, cx: &mut Cx<'_>, since: Option<BlockId>) -> Result<(), String> {
        self.want.roll_up_run(cx.reg, cx.traces, cx.dir, since, Tiling::WHOLE).map(drop)
    }

    /// The instance with its traces' layers placed at `place` (`dir::PLACE_KEPT`: they last across processes), at a
    /// kept run's heads (`[]`, `None`: none). Its cache is in memory: a new process's is ∅ (L5).
    pub fn resumed(mut self, place: u32, outs: &[Option<BlockId>], want: Option<BlockId>, asks: &[Option<BlockId>]) -> Instance {
        self.outs = std::mem::take(&mut self.outs).into_iter().enumerate().map(|(k, t)| t.resumed(place, outs.get(k).copied().flatten())).collect();
        self.want = std::mem::replace(&mut self.want, Self::want_trace().expect("the want's shape")).resumed(place, want);
        self.asks = std::mem::take(&mut self.asks).into_iter().enumerate().map(|(p, t)| t.map(|t| t.resumed(place, asks.get(p).copied().flatten()))).collect();
        self
    }

    /// Its outputs' heads.
    pub fn heads(&self) -> Vec<Option<BlockId>> {
        self.outs.iter().map(Trace::head).collect()
    }

    /// Its requests traces' heads, a port each (`None` for a port it does not ask of, and for one asked nothing yet).
    pub fn ask_heads(&self) -> Vec<Option<BlockId>> {
        self.asks.iter().map(|t| t.as_ref().and_then(Trace::head)).collect()
    }

    /// Its requests trace of input port `p` (`None`: it asks nothing of it): the keys it asks now, `(key)`, read where
    /// its state lies.
    pub fn asks_of(&self, p: usize) -> Option<&Trace> {
        self.asks.get(p).and_then(Option::as_ref)
    }

    /// Its want moved to `rows` (`(port, key)`), a layer of their change on its want trace, written into its runs as
    /// it is found: the heads the run's want is between (the identity when they are the same rows).
    pub fn set_want(&mut self, cx: &mut Cx<'_>, rows: &dyn WantRows) -> Result<(Option<BlockId>, Option<BlockId>), String> {
        let before = self.want.head();
        // The want's change, read where its state lies: its rows `rows` lacks, gone; those of `rows` it lacks, added
        // (by its canonical order, key first).
        let row = |s: &View<'_>, id| (Read::get(s, id, ColumnIndex::new(0)) as usize, Read::get(s, id, ColumnIndex::new(1)));
        let held = |s: &View<'_>, p: usize, k: i64| Read::matching(s, &[ColumnIndex::new(1), ColumnIndex::new(0)], &[k, p as i64]).next().is_some();
        let change = |s: &View<'_>, f: &mut dyn FnMut(usize, i64, i32)| {
            for id in s.live_ids() {
                let (p, k) = row(s, id);
                if !rows.holds(p, k) {
                    f(p, k, -1);
                }
            }
            rows.each(&mut |p, k| {
                if !held(s, p, k) {
                    f(p, k, 1);
                }
            });
        };
        let n = self.want.reading(cx.dir, before, |s| {
            let mut n = 0;
            change(s, &mut |_, _, _| n += 1);
            n
        })?;
        if n == 0 {
            return Ok((before, before));
        }
        let after = push_with(cx.reg, cx.traces, cx.dir, &mut self.want, n, |dir, t, w| {
            t.reading(dir, before, |s| {
                let mut room = true;
                change(s, &mut |p, k, diff| room &= w.push(&[p as i64, k], diff).is_some());
                if room { Ok(()) } else { Err("room for the want's change".to_string()) }
            })?
        })?;
        Ok((before, Some(after)))
    }

    /// Empties its outputs, want and requests (a cold run's start).
    pub fn clear(&mut self, dir: &mut Directory) {
        self.outs.iter_mut().for_each(|t| t.clear(dir));
        self.want.clear(dir);
        self.asks.iter_mut().flatten().for_each(|t| t.clear(dir));
    }

    /// Drops its cache: a new one, ∅ (L5: always correct).
    pub fn drop_cache(&mut self, dir: &mut Directory) {
        let old = self.cache;
        self.cache = OpCache::new(dir);
        dir.release(old.0);
    }

    /// One run of `op` over `ins`, each input port's trace between two heads, under `want` (its want trace between two
    /// heads; `None`: every output wanted): its outputs' change pushed into a runs block a port and its asks' change a
    /// runs block an asked port, each then committed (an empty one is not: the trace stays where it was), and the
    /// cache committed with the state the run made when the run wrote it. A run whose runs outgrew their room runs
    /// again at the size they needed, from a cache read as ∅ (its first attempt's writes may be half done).
    pub fn step<O: DeltaOp>(&mut self, op: &O, cx: &mut Cx<'_>, ins: &[InPort<'_>], globals: &[GlobalIn], want: WantIn) -> Result<Work, String> {
        let befores: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.1)).collect();
        let afters: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.2)).collect();
        let (gb, ga): (Vec<Global>, Vec<Global>) = (globals.iter().map(|g| g.side(false)).collect(), globals.iter().map(|g| g.side(true)).collect());
        let (mut from, mut to) = (inputs_digest(cx.dir, &befores, &gb)?, inputs_digest(cx.dir, &afters, &ga)?);
        // The state a cache names is its inputs' and its want's: a cache made under one want reads as ∅ under another.
        if let Some((wb, wa)) = want {
            from = rerail_blocks::layer::mix(from ^ self.want.digest(cx.dir, wb)?.rotate_left(17));
            to = rerail_blocks::layer::mix(to ^ self.want.digest(cx.dir, wa)?.rotate_left(17));
        }
        let mut cap = vec![FIRST_RUNS; self.outs.len()];
        let mut ask_cap = vec![FIRST_RUNS; self.asks.len()];
        let mut again = 0;
        loop {
            let runs: Vec<BlockId> = self.outs.iter().zip(&cap).map(|(t, &c)| t.open(cx.dir, c)).collect();
            let mut out = Out::<O::Ports> { runs, kept: self.heads(), of: PhantomData };
            let mut asks = Asks::<O::Ports> { runs: self.asks.iter().zip(&ask_cap).map(|(t, &c)| t.as_ref().map(|t| t.open(cx.dir, c))).collect(), of: PhantomData };
            let release = |dir: &mut Directory, out: &Out<O::Ports>, asks: &Asks<O::Ports>| out.runs.iter().chain(asks.runs.iter().flatten()).for_each(|&r| dir.release(r));
            // After an attempt that outgrew its runs, a cache it wrote is open: it reads as ∅, and its first write
            // resets it.
            let mut cache = CacheW::<O::Cache>::new(self.cache, from);
            let work = {
                let wanted = want.map_or(Vec::new(), |(b, a)| [b, a].into_iter().flatten().collect());
                let heads: Vec<BlockId> = ins.iter().flat_map(|p| [p.1, p.2]).flatten().chain(wanted).collect();
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
                let w = match want {
                    None => None,
                    Some((None, a)) => Some(Bistate::cold(self.want.state(&table, a)?)),
                    Some((b, a)) => Some(Bistate::new(self.want.state(&table, b)?, self.want.state(&table, a)?)),
                };
                let input = In { ports, heads: ins.iter().map(|p| (p.1, p.2)).collect(), globals: globals.to_vec(), of: PhantomData };
                op.delta(cx, &input, w.as_ref(), &mut cache, &mut out, &mut asks)
            };
            let work = match work {
                Ok(w) => w,
                Err(e) => {
                    release(cx.dir, &out, &asks);
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
            for (p, r) in asks.runs.iter().enumerate() {
                let Some(r) = *r else { continue };
                let l = cx.dir.read("an operator's asks", &[r])?;
                let rr = Runs::new(l.block(0));
                if rr.needed() > 0 {
                    ask_cap[p] = rr.len() + rr.needed();
                    over = true;
                }
            }
            if over {
                release(cx.dir, &out, &asks);
                again += 1;
                continue;
            }
            let commits = self.outs.iter_mut().zip(out.runs.iter().copied().map(Some)).chain(self.asks.iter_mut().flatten().zip(asks.runs.iter().copied().filter(Option::is_some)));
            for (t, r) in commits {
                let r = r.expect("an asked port's runs");
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
            let mut work = work;
            if again > 0 {
                work.excess.push((OUTGROWN, again));
            }
            return Ok(work);
        }
    }
}

/// The oracle's rows of a port (F1, by meaning): its trace's state at `head`, read where it lies, gathered as a set to
/// compare with the plain function's. Tests and the law harness only: no product build has it.
#[cfg(any(test, feature = "test-hooks"))]
pub fn oracle_rows(dir: &Directory, t: &Trace, head: Option<BlockId>) -> Result<BTreeSet<Vec<i64>>, String> {
    // copy: derived (the oracle's rows)
    t.reading(dir, head, |s| s.live_ids().map(|id| ColumnIndex::all(Read::arity(s)).map(|c| Read::get(s, id, c)).collect()).collect())
}

#[cfg(any(test, feature = "test-hooks"))]
/// A port's change, by its rows (the oracle's form).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub added: BTreeSet<Vec<i64>>,
    pub gone: BTreeSet<Vec<i64>>,
}

#[cfg(any(test, feature = "test-hooks"))]
/// The plain function's output on the inputs' states `heads`: `run` into fresh output traces, committed, read.
pub fn apply<O: DeltaOp>(op: &O, cx: &mut Cx<'_>, ins: &[(&Trace, Option<BlockId>)], globals: &[Global]) -> Result<Vec<BTreeSet<Vec<i64>>>, String> {
    let ports = ports_of::<O>()?;
    let mut outs: Vec<Trace> = ports.outs.iter().map(|p| Trace::new(&p.shape).map(|t| t.with_roll_up(RollUp::Never))).collect::<Result<_, _>>()?;
    let mut cap = vec![FIRST_RUNS; outs.len()];
    loop {
        let runs: Vec<BlockId> = outs.iter().zip(&cap).map(|(t, &c)| t.open(cx.dir, c)).collect();
        let mut out = Out::<O::Ports> { runs, kept: vec![None; outs.len()], of: PhantomData };
        {
            let heads: Vec<BlockId> = ins.iter().filter_map(|p| p.1).collect();
            let l = cx.dir.read("the reference's inputs", &heads)?;
            let table = l.table();
            let views = ins.iter().map(|(t, h)| t.state(&table, *h)).collect::<Result<Vec<_>, _>>()?;
            op.run(cx, &At { ports: views, globals: globals.to_vec(), of: PhantomData }, &mut out)?;
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
            rows.push(oracle_rows(cx.dir, t, Some(head))?);
        }
        return Ok(rows);
    }
}

#[cfg(any(test, feature = "test-hooks"))]
/// The generic reference (P§2, Figure 4): the plain function run on the inputs' before side and on their after side,
/// and the two diffed, port by port.
pub fn reference<O: DeltaOp>(op: &O, cx: &mut Cx<'_>, ins: &[InPort<'_>], globals: &[GlobalIn]) -> Result<Vec<Change>, String> {
    let befores: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.1)).collect();
    let afters: Vec<(&Trace, Option<BlockId>)> = ins.iter().map(|p| (p.0, p.2)).collect();
    let (gb, ga): (Vec<Global>, Vec<Global>) = (globals.iter().map(|g| g.side(false)).collect(), globals.iter().map(|g| g.side(true)).collect());
    let (b, a) = (apply(op, cx, &befores, &gb)?, apply(op, cx, &afters, &ga)?);
    Ok(b.iter().zip(&a).map(|(b, a)| Change { added: a.difference(b).cloned().collect(), gone: b.difference(a).cloned().collect() }).collect())
}

#[cfg(any(test, feature = "test-hooks"))]
/// A law the harness found broken: which, at which step of the chain, and how.
#[derive(Debug)]
pub struct Broken {
    pub law: &'static str,
    pub step: usize,
    pub what: String,
}

#[cfg(any(test, feature = "test-hooks"))]
/// One edit chain: for each step, each input port's `(row, ±1)` pairs, a set change of the port's state (an empty one
/// leaves the port's state where it was: the identity).
pub type Chain = Vec<Vec<Vec<(Vec<i64>, i32)>>>;

#[cfg(any(test, feature = "test-hooks"))]
/// Checks every law of `op` over `chain` (P§12's "one test harness for every operator"): each step run by a threaded
/// instance (its cache carried), by an instance whose cache is dropped at step `drop_at`, and cold, by a fresh
/// instance from ∅; each against the reference. In order, a step's checks are L2 (the cold run is ∅ → op(X)), L1 (the
/// threaded run's change is the reference's), L4 (with every input the identity: the output too, the work the checks
/// alone, the cache unchanged), L5 (the run whose cache was dropped gives the threaded run's output) and L3 (the
/// threaded run's output is the cold run's). The first that fails is returned.
pub fn harness<O: DeltaOp>(op: &O, chain: &Chain, drop_at: usize) -> Result<(), Broken> {
    harness_under(op, chain, drop_at, None)
}

#[cfg(any(test, feature = "test-hooks"))]
/// `harness` under `wants` (a want a state of the chain, the first the state before it): D1 checked at each step
/// (`harness_on`).
pub fn harness_under<O: DeltaOp>(op: &O, chain: &Chain, drop_at: usize, wants: Option<&Wants>) -> Result<(), Broken> {
    let setup = |e: String| Broken { law: "setup", step: 0, what: e };
    let mut bench = Bench::new().map_err(setup)?;
    let (mut ins, heads) = inputs::<O>(&mut bench, chain).map_err(setup)?;
    let globals = vec![Vec::new(); heads.len()];
    harness_on(op, &mut bench, &mut ins, &heads, &globals, drop_at, wants)
}

/// What operators run in (a run's, a harness's): a registry with the trace's and the cache's operations (and any
/// its operators' own), a directory and the blobs.
pub struct Bench {
    pub reg: Registry,
    pub dir: Directory,
    pub blobs: BlobRows,
    pub traces: TraceOps,
    pub caches: CacheOps,
    /// Δanswer's operations (`answer_load`, `parse`).
    pub answers: crate::answer::AnswerOps,
}

impl Bench {
    /// A bench in the run's memory: nothing lasts past it.
    pub fn new() -> Result<Bench, String> {
        let mut reg = Registry::default();
        let traces = TraceOps::declare(&mut reg)?;
        let caches = CacheOps::declare(&mut reg)?;
        let answers = crate::answer::AnswerOps::declare(&mut reg)?;
        Ok(Bench { reg, dir: Directory::default(), blobs: BlobRows::new(&traces), traces, caches, answers })
    }

    /// A bench over the kept blocks in folder `kept` (a store's, `files::Store::kept`): each mapped where it lies, at
    /// its id (`Directory::open_kept`), and what lasts kept there from now on; the last run's pins (`boot::Pins`) and
    /// their block, when one was kept, and the blobs' rows at the head they name, less those of blocks a process does
    /// not keep (a slab's: its bytes the store's objects hold, loaded again when asked).
    pub fn open(kept: &std::path::Path) -> Result<(Bench, Option<(BlockId, crate::boot::Pins)>), String> {
        let mut bench = Bench::new()?;
        bench.dir = if rerail_blocks::kept::trace_folder::blocks(kept).map_err(|e| format!("{}: {e}", kept.display()))?.is_empty() {
            Directory::default().with_kept(kept)
        } else {
            Directory::open_kept(kept, rerail_blocks::block::kind::FACTS)?
        };
        let pins = crate::boot::Pins::open(&bench.dir)?;
        bench.blobs = BlobRows::new(&bench.traces).resumed(bench.dir.lasting(), pins.as_ref().and_then(|p| p.1.blobs));
        // What the newest pins reach, and the blocks their blobs lie in (the symbols blocks, the pure answers kept
        // by key): every other kept block is a state no run reads again (an older run's, a process stopped before
        // letting it go), released, its file removed.
        if let Some((id, _)) = &pins {
            let mut keep = bench.dir.reach(&[*id]);
            // The blocks the blobs' rows name, read where the rows lie.
            bench.blobs.rows_trace().reading(&bench.dir, bench.blobs.head(), |s| keep.extend(s.live_ids().map(|r| BlockId(Read::get(s, r, ColumnIndex::new(4)) as u32))))?;
            for b in bench.dir.owned() {
                if !keep.contains(&b) {
                    bench.dir.release(b);
                }
            }
        }
        let Bench { reg, dir, blobs, .. } = &mut bench;
        blobs.forget_dead(reg, dir)?;
        Ok((bench, pins))
    }

    pub fn cx(&mut self) -> Cx<'_> {
        Cx { reg: &self.reg, dir: &mut self.dir, blobs: &self.blobs, traces: &self.traces, caches: &self.caches }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
/// The input traces of a chain of row changes, a trace a port, every state kept (a layer a step, none rolled up), and
/// each step's heads.
#[allow(clippy::type_complexity)]
fn inputs<O: DeltaOp>(bench: &mut Bench, chain: &Chain) -> Result<(Vec<Trace>, Vec<Vec<Option<BlockId>>>), String> {
    let Bench { reg, dir, traces, .. } = &mut *bench;
    let mut ins: Vec<Trace> = ports_of::<O>()?.ins.iter().map(|p| Trace::new(&p.shape).map(|t| t.with_roll_up(RollUp::Never))).collect::<Result<_, _>>()?;
    let mut heads: Vec<Vec<Option<BlockId>>> = vec![vec![None; ins.len()]];
    for step in chain {
        let mut now = heads.last().cloned().expect("a state");
        for (k, pairs) in step.iter().enumerate() {
            if pairs.is_empty() {
                continue;
            }
            now[k] = Some(push_layer(reg, traces, dir, &mut ins[k], pairs)?);
        }
        heads.push(now);
    }
    Ok((ins, heads))
}

/// Trace `t`'s next layer, of the `(row, ±1)` pairs `pairs`: its new head.
pub fn push_layer<R: AsRef<[i64]>>(reg: &Registry, traces: &TraceOps, dir: &mut Directory, t: &mut Trace, pairs: &[(R, i32)]) -> Result<BlockId, String> {
    push_with(reg, traces, dir, t, pairs.len(), |_, _, w| {
        for (r, d) in pairs {
            w.push(r.as_ref(), *d).ok_or("room for the pairs")?;
        }
        Ok(())
    })
}

/// A layer of at most `n` pairs on `t`, committed: `fill` pushes them into its runs block, reading what it needs where
/// it lies (a state of `t` itself among them: its layers are other blocks), so no pair is gathered first. The runs
/// block is let go when `fill` fails.
pub fn push_with(reg: &Registry, traces: &TraceOps, dir: &mut Directory, t: &mut Trace, n: usize, fill: impl FnOnce(&Directory, &Trace, &RunsW<'_>) -> Result<(), String>) -> Result<BlockId, String> {
    let id = t.open(dir, n.max(1));
    let filled = {
        let b = dir.block(id).expect("made");
        let pin = pin_block(&b, "a layer's pairs", true)?;
        let w = RunsW::new(&pin);
        fill(dir, t, &w)
    };
    if let Err(e) = filled {
        dir.release(id);
        return Err(e);
    }
    t.commit(reg, traces, dir, id, Tiling::WHOLE)
}

#[cfg(any(test, feature = "test-hooks"))]
/// A chain's wants: the want at each state of the chain (`(port, key)`), the first the state before it (its inputs ∅, so
/// its output is ∅ under any want).
pub type Wants = [BTreeSet<(usize, i64)>];

#[cfg(any(test, feature = "test-hooks"))]
/// `harness`'s laws over input traces `ins` made already, at `heads`, a step's heads each (the first, every port's
/// state before the chain), and its globals' states at each step, `globals`: the traces and the globals an
/// operator's world read writes, as L1's tape port and its atoms are (B6). Under `wants` (a want a state), every
/// instance runs under them, each law is judged on the outputs restricted to the want of its side, and P§7.1's D1 is
/// checked at each step:
///
/// - demand restricts, never changes: the output restricted to the want is the full output restricted, and holds no
///   row the full output lacks (an operator may produce more, over-demand at its grain);
/// - over-demand is sound: an instance under no want (every output) gives the same rows where the want asks;
/// - the asks suffice: the plain function on the inputs restricted to what the operator asks of each asked port gives
///   the same wanted output.
pub fn harness_on<O: DeltaOp>(op: &O, bench: &mut Bench, ins: &mut [Trace], heads: &[Vec<Option<BlockId>>], globals: &[Vec<Global>], drop_at: usize, wants: Option<&Wants>) -> Result<(), Broken> {
    let fail = |law, step, what: String| Broken { law, step, what };
    let setup = |e: String| fail("setup", 0, e);
    let ports = ports_of::<O>().map_err(|e| fail("setup", 0, e))?;
    let ins: &[Trace] = ins;
    let mut cx = bench.cx();
    let at = |k: usize| -> Vec<(&Trace, Option<BlockId>)> { ins.iter().zip(&heads[k]).map(|(t, &h)| (t, h)).collect() };
    let between = |a: Option<usize>, b: usize| -> Vec<GlobalIn> {
        globals[b].iter().enumerate().map(|(i, g)| GlobalIn { block: g.block, before: a.map(|a| globals[a][i].count), after: g.count }).collect()
    };
    // Each output's rows at state `k`, restricted to its want (all of them under no want).
    let narrow = |rows: &[BTreeSet<Vec<i64>>], k: usize| -> Vec<BTreeSet<Vec<i64>>> {
        match wants {
            None => rows.to_vec(),
            Some(w) => rows.iter().enumerate().map(|(p, r)| restrict(r, p, &ports.outs[p].shape, &w[k])).collect(),
        }
    };
    let outs_of = |cx: &Cx<'_>, inst: &Instance| -> Result<Vec<BTreeSet<Vec<i64>>>, String> { inst.outs.iter().map(|t| oracle_rows(cx.dir, t, t.head())).collect() };
    let mut full = vec![apply(op, &mut cx, &at(0), &globals[0]).map_err(setup)?];
    let mut threaded = Instance::new(op, cx.dir).map_err(setup)?;
    let mut dropped = Instance::new(op, cx.dir).map_err(setup)?;
    let mut over = Instance::new(op, cx.dir).map_err(setup)?;
    // Each instance's want moved to state `k`'s: the heads its run is between.
    let move_want = |cx: &mut Cx<'_>, inst: &mut Instance, k: usize| -> Result<WantIn, String> { wants.map(|w| inst.set_want(cx, &w[k])).transpose() };
    // The chain's instances start as any instance does, under no want; the chain's first state is ∅ (its inputs'), so
    // the first step's want is ∅ → the second state's.
    for k in 1..heads.len() {
        let step: Vec<InPort<'_>> = ins.iter().enumerate().map(|(p, t)| (t, heads[k - 1][p], heads[k][p])).collect();
        let cold: Vec<InPort<'_>> = ins.iter().enumerate().map(|(p, t)| (t, None, heads[k][p])).collect();
        let (gstep, gcold) = (between(Some(k - 1), k), between(None, k));
        full.push(apply(op, &mut cx, &at(k), &globals[k]).map_err(|e| fail("reference", k, e))?);
        // L2: a null start, from ∅ and an empty cache, is ∅ → op(X).
        let mut fresh = Instance::new(op, cx.dir).map_err(setup)?;
        let w = move_want(&mut cx, &mut fresh, k).map_err(|e| fail("L2", k, e))?;
        fresh.step(op, &mut cx, &cold, &gcold, w).map_err(|e| fail("L2", k, e))?;
        let cold_rows = outs_of(&cx, &fresh).map_err(|e| fail("L2", k, e))?;
        if narrow(&cold_rows, k) != narrow(&full[k], k) {
            return Err(fail("L2", k, format!("from ∅, the output is not op(X): {cold_rows:?}, want {:?}", narrow(&full[k], k))));
        }
        // L1: the threaded run's change is the reference's (each side restricted to its want).
        let before = threaded.heads();
        let rows_before = outs_of(&cx, &threaded).map_err(|e| fail("L1", k, e))?;
        let cache_before = {
            let l = cx.dir.read("the harness", &[threaded.cache.0]).map_err(setup)?;
            cache_facts(&l.table(), 0, l.block(0))
        };
        let w = move_want(&mut cx, &mut threaded, k).map_err(|e| fail("L1", k, e))?;
        let work = threaded.step(op, &mut cx, &step, &gstep, w).map_err(|e| fail("L1", k, e))?;
        let rows_after = outs_of(&cx, &threaded).map_err(|e| fail("L1", k, e))?;
        let reference: Vec<Change> = if wants.is_none() {
            reference(op, &mut cx, &step, &gstep).map_err(|e| fail("reference", k, e))?
        } else {
            let (b, a) = (narrow(&full[k - 1], k - 1), narrow(&full[k], k));
            b.iter().zip(&a).map(|(b, a)| Change { added: a.difference(b).cloned().collect(), gone: b.difference(a).cloned().collect() }).collect()
        };
        let (nb, na) = (narrow(&rows_before, k - 1), narrow(&rows_after, k));
        for p in 0..threaded.outs.len() {
            let got = Change { added: na[p].difference(&nb[p]).cloned().collect(), gone: nb[p].difference(&na[p]).cloned().collect() };
            if got != reference[p] {
                return Err(fail("L1", k, format!("port {}: the change is {got:?}; the reference's is {:?}", ports.outs[p].name, reference[p])));
            }
        }
        // D1: demand restricts, never changes.
        if wants.is_some() {
            for p in 0..threaded.outs.len() {
                if let Some(r) = rows_after[p].difference(&full[k][p]).next() {
                    return Err(fail("D1", k, format!("port {}: row {r:?} is in no full output", ports.outs[p].name)));
                }
            }
            over.step(op, &mut cx, &step, &gstep, None).map_err(|e| fail("D1", k, e))?;
            if narrow(&outs_of(&cx, &over).map_err(|e| fail("D1", k, e))?, k) != na {
                return Err(fail("D1", k, "over-demand (every output) changed what the want asks".into()));
            }
            // The asks suffice: each asked port restricted to the keys asked of it.
            let mut narrowed: Vec<Trace> = Vec::new();
            let mut nheads: Vec<Option<BlockId>> = Vec::new();
            for (p, t) in ins.iter().enumerate() {
                if threaded.asks[p].is_none() {
                    continue;
                }
                let at = threaded.asks[p].as_ref().expect("a port asked of");
                let asked: BTreeSet<i64> = oracle_rows(cx.dir, at, at.head()).map_err(|e| fail("D1", k, e))?.into_iter().map(|r| r[0]).collect();
                let c = ports.ins[p].shape.partition.map_or(0, |c| c as usize);
                let rows: Vec<(Vec<i64>, i32)> = oracle_rows(cx.dir, t, heads[k][p]).map_err(|e| fail("D1", k, e))?.into_iter().filter(|r| asked.contains(&r[c])).map(|r| (r, 1)).collect();
                let mut n = Trace::new(&ports.ins[p].shape).map_err(|e| fail("D1", k, e))?.with_roll_up(RollUp::Never);
                nheads.push(if rows.is_empty() { None } else { Some(push_layer(cx.reg, cx.traces, cx.dir, &mut n, &rows).map_err(|e| fail("D1", k, e))?) });
                narrowed.push(n);
            }
            let mut j = 0;
            let restricted: Vec<(&Trace, Option<BlockId>)> = ins
                .iter()
                .enumerate()
                .map(|(p, t)| {
                    if threaded.asks[p].is_none() {
                        return (t, heads[k][p]);
                    }
                    j += 1;
                    (&narrowed[j - 1], nheads[j - 1])
                })
                .collect();
            let on_asks = apply(op, &mut cx, &restricted, &globals[k]).map_err(|e| fail("D1", k, e))?;
            if narrow(&on_asks, k) != narrow(&full[k], k) {
                return Err(fail("D1", k, format!("on the inputs it asked ({:?}), the wanted output differs", threaded.ask_heads())));
            }
        }
        // L4: identity is free.
        let want_same = wants.is_none_or(|w| w[k] == w[k - 1]);
        if want_same && (0..ins.len()).all(|p| heads[k][p] == heads[k - 1][p]) && gstep.iter().all(GlobalIn::is_identity) {
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
        let w = move_want(&mut cx, &mut dropped, k).map_err(|e| fail("L5", k, e))?;
        dropped.step(op, &mut cx, &step, &gstep, w).map_err(|e| fail("L5", k, e))?;
        let rows_dropped = outs_of(&cx, &dropped).map_err(|e| fail("L5", k, e))?;
        if narrow(&rows_dropped, k) != na {
            return Err(fail("L5", k, format!("with its cache dropped at step {drop_at}, the output differs")));
        }
        // L3: the chain's output is a cold run's.
        let nc = narrow(&cold_rows, k);
        for p in 0..threaded.outs.len() {
            if na[p] != nc[p] {
                return Err(fail("L3", k, format!("port {}: the chain's output drifted from a cold run's", ports.outs[p].name)));
            }
        }
    }
    Ok(())
}

#[cfg(any(test, feature = "test-hooks"))]
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
