//! Operations over blocks (decision 0686, B1; *0686: Operations* §2–3, §9).
//!
//! An operation is a meaningful quantum of work over at most 30 whole blocks: its **signature** names each slot's
//! mode (read, write, read-write, scratch: UKL's `ROBUFFER`, `WOBUFFER`, `RWBUFFER`) and the kind of block it takes,
//! its params' layout (declared once, by `rerail_delta_twins::params!`, and given to UKL from there) and its work
//! shape as phases (Operations §4). What it writes is a function of its params and the bytes of its slots, and of
//! nothing else.
//!
//! Each operation's shape is a list of phases, each placed on the six axes of Operations §4 (how work is found, an
//! item's unit, what is shared inside it, how units relate, where output goes, its bounds): there is no enum of
//! shapes. The signature also says the axis its items are independent along, and how its written slots' facts are
//! read by meaning.
//!
//! Each operation has registered **versions** (`Version`, Operations §3, E3), chosen by their metadata. Its Rust
//! twin (`rerail_delta_twins`: its phases, each a function of its pinned blocks, its descriptor table and its params)
//! is its reference: registered first (`register_reference`), and the one every other version is checked against. A
//! version is registered only after `Registry::check` has run it against the reference and found the same facts.
//!
//! **Starting** an operation pins every slot from its signature through the blocks' own regions (0491's checks,
//! `rerail_blocks::region::pin_block`), naming the operation, and read-pins every block a slot refers to (a referenced
//! generation, `dir`): a second writer, a writer beside a reader, or a write of a frozen block refuses the start, and
//! nothing stays pinned (E1). The CPU's descriptor table of the reached blocks is written for the launch (E2). The
//! CPU executor (`run_cpu`) then runs the twin's phases on the pinned bytes, in place, each phase cut into tiles of
//! its items, run in any order (`Tiling`). The registry and the executor are host Rust, handles only; the computation
//! is the twin's alone.

use crate::dir::{BlockId, Directory};
use rerail_blocks::gens::Table;
use rerail_delta_twins::{Ctx, Field, Params, Slot, Tile, Twin};

/// The most slots an operation binds: Metal's argument table has 31 entries, one of them the params' (D1).
pub const MAX_SLOTS: usize = 30;

/// How an operation uses a slot's block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Read,
    Write,
    ReadWrite,
    /// Written and read inside the operation; nothing in it is kept after.
    Scratch,
}

impl Mode {
    /// The UKL argument row of a slot with this mode.
    pub fn ukl(self) -> &'static str {
        match self {
            Mode::Read => "ROBUFFER",
            Mode::Write => "WOBUFFER",
            Mode::ReadWrite | Mode::Scratch => "RWBUFFER",
        }
    }

    pub fn writes(self) -> bool {
        self != Mode::Read
    }
}

/// One slot of a signature: its name, its mode and the kind of block it takes (`rerail_blocks::block::kind`).
#[derive(Clone, Copy, Debug)]
pub struct SlotSpec {
    pub name: &'static str,
    pub mode: Mode,
    pub kind: u32,
}

// ---- Work shapes (Operations §4): no enum of shapes; a shape is a list of phases, each placed on six axes ---------

/// How a phase's work is found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Found {
    /// A fixed count.
    Fixed,
    /// A count made on the device by an earlier phase (an indirect size).
    Indirect,
    JobTable,
    /// Spawned by an item.
    Spawned,
    /// Pushed onto a queue.
    Queue,
    /// Taken from a cursor.
    Cursor,
    /// Requested by a reader.
    Requested,
}

/// An item's unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Lane,
    SimdGroup,
    /// A threadgroup: a tile.
    Threadgroup,
    SimdGroups,
    /// A whole partition: a file, a unit, a piece, a component.
    Partition,
}

/// What is shared inside a unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inside {
    Nothing,
    /// SIMD-group operations: ballot, shuffle, scan.
    SimdOps,
    /// Group memory and barriers.
    GroupMemory,
    MatrixFragments,
    /// A thread's stacks.
    Stacks,
}

/// How units relate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Across {
    Nothing,
    /// An associative combine.
    Combine,
    /// Phases in order.
    Ordered,
    DependencyCount,
    /// Rounds to a fixpoint.
    Rounds,
    /// Speculation and a check.
    Speculation,
    /// Requests memoized.
    Memoized,
}

/// Where output goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    ByIndex,
    /// By a scan of counts.
    ByScan,
    /// By an atomic claim.
    Claim,
    /// In place: CAS, OR, max.
    InPlace,
    WriterState,
    Sketch,
}

/// What a phase past its output capacity does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    Refuse,
    Grow,
    Rerun,
}

/// A bound on a phase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    /// Steps an item.
    Steps(u32),
    /// Turns of a phase that runs again (a queue's, a fixpoint's rounds, a merge's passes): past it, a hard failure.
    Turns(u32),
    /// The launch budget (0540, 0601).
    LaunchBudget,
    Capacity(Over),
    StackDepth(u32),
    /// A sample's size.
    Sample(u32),
}

/// One phase of an operation's work, placed on the six axes (Operations §4), with its bounds.
#[derive(Clone, Copy, Debug)]
pub struct Phase {
    pub name: &'static str,
    pub found: Found,
    pub unit: Unit,
    pub inside: Inside,
    pub across: Across,
    pub output: Output,
    pub bounds: &'static [Bound],
}

impl Phase {
    /// A phase of lanes, each an item, sharing nothing: placed on the other axes by the caller.
    pub const fn lanes(name: &'static str, found: Found, across: Across, output: Output, bounds: &'static [Bound]) -> Phase {
        Phase { name, found, unit: Unit::Lane, inside: Inside::Nothing, across, output, bounds }
    }

    /// The most turns it runs: its `Turns` bound (one turn without one).
    pub fn turns(&self) -> u32 {
        self.bounds.iter().find_map(|b| if let Bound::Turns(n) = b { Some(*n) } else { None }).unwrap_or(1)
    }
}

/// The axis an operation's items are independent along, so the executor may batch, cut into morsels, hand off and
/// split it (Operations §4, family G), or why they are not: an operation that is `Not` runs each phase whole and in
/// order. A claim of independence is checked: `Registry::check` runs a version at tilings in shuffled order against
/// the reference run whole, so items that are not independent write other facts there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Independence {
    Along(&'static str),
    Not(&'static str),
}

/// A params struct's layout, from its one declaration (`params!`).
#[derive(Clone, Copy)]
pub struct ParamsLayout {
    pub name: &'static str,
    pub fields: &'static [Field],
    pub size: usize,
}

impl ParamsLayout {
    pub const fn of<P: Params>() -> ParamsLayout {
        ParamsLayout { name: P::NAME, fields: P::FIELDS, size: P::SIZE }
    }

    /// Its UKL declaration: a `typedef`, then its size and each field's offset asserted against the Rust layout.
    pub fn ukl(&self) -> String {
        let mut s = format!("typedef struct {} {{\n", self.name);
        for f in self.fields {
            s += &format!("    {} {};\n", f.c, f.name);
        }
        s += &format!("}} {};\n", self.name);
        s += &format!("UKL_STATIC_ASSERT(sizeof({0}) == {1}, \"{0}: size\");\n", self.name, self.size);
        for f in self.fields {
            s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF({0}, {1}) == {2}, \"{0}.{1}\");\n", self.name, f.name, f.at);
        }
        s
    }
}

/// The facts a written slot holds, read by meaning (F1): rows compared as a set, so two versions agree when they
/// wrote the same facts, whatever their order or ids. Given the slot's index, its bytes and the launch's table.
pub type FactsFn = fn(&Table<'_>, usize, &[u8]) -> Vec<Vec<i64>>;

/// What an operation is: its slots, params, shape, the axis its items are independent along, and how its written
/// slots' facts are read. Declared once, as a constant; `Registry::declare` takes it.
pub struct Signature {
    pub name: &'static str,
    pub slots: &'static [SlotSpec],
    pub params: ParamsLayout,
    pub shape: &'static [Phase],
    pub independent: Independence,
    pub facts: FactsFn,
}

impl Signature {
    /// A signature; more than `MAX_SLOTS` slots fails where it is declared (at compile time for a constant).
    pub const fn new(
        name: &'static str,
        slots: &'static [SlotSpec],
        params: ParamsLayout,
        shape: &'static [Phase],
        independent: Independence,
        facts: FactsFn,
    ) -> Signature {
        assert!(slots.len() <= MAX_SLOTS, "an operation binds at most 30 slots");
        Signature { name, slots, params, shape, independent, facts }
    }
}

/// An operation, by its place in the registry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpId(u32);

/// How a version runs.
#[derive(Clone, Copy)]
pub enum Impl {
    /// The Rust twin (`rerail_delta_twins`): the reference every other version is checked against.
    Twin(Twin),
    Ukl { source: &'static str, entry: &'static str },
    /// Compiled ahead and kept per machine (0395), found by its key.
    Compiled(ArtifactKey),
}

/// A compiled artifact's key: the digest of its source, the library's fingerprint and its device.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArtifactKey(pub [u8; 32]);

/// The processors a version runs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Devices(pub u32);

impl Devices {
    pub const CPU: Devices = Devices(1);
    pub const METAL: Devices = Devices(2);
    pub const CUDA: Devices = Devices(4);
    pub const HIP: Devices = Devices(8);

    pub fn has(self, d: Devices) -> bool {
        self.0 & d.0 == d.0
    }
}

/// What a version needs of its processor: a SIMD width, matrix units, SVE2, device enqueue, coherent memory, ….
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Features(pub u64);

/// The most a version takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    pub items: u64,
    pub rows: u64,
    pub bytes: u64,
}

impl Limits {
    pub const NONE: Limits = Limits { items: u64::MAX, rows: u64::MAX, bytes: u64::MAX };
}

/// What the planner prices a version by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CostKey(pub &'static str);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Reference,
    Candidate,
    Default,
}

/// One registered version of an operation, with what the planner and the executor choose it by (Operations §3).
#[derive(Clone, Copy)]
pub struct Version {
    pub op: OpId,
    pub name: &'static str,
    pub imp: Impl,
    pub devices: Devices,
    pub needs: Features,
    /// The shape it runs the work as: its phases on the six axes.
    pub shape: &'static [Phase],
    pub limits: Limits,
    pub cost: CostKey,
    pub status: Status,
}

impl Version {
    /// An operation's twin as its reference version, running the work as the signature's shape.
    pub fn twin(op: OpId, name: &'static str, twin: Twin, shape: &'static [Phase]) -> Version {
        Version {
            op,
            name,
            imp: Impl::Twin(twin),
            devices: Devices::CPU,
            needs: Features::default(),
            shape,
            limits: Limits::NONE,
            cost: CostKey(name),
            status: Status::Reference,
        }
    }
}

/// How the executor cuts a launch's phases: items a tile (0: one tile of every item), and the order the tiles run in
/// (a seed shuffles them; 0 runs them in order). An operation's result is correct at every tiling and order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tiling {
    pub tile: u32,
    pub seed: u64,
}

impl Tiling {
    pub const WHOLE: Tiling = Tiling { tile: 0, seed: 0 };
}

/// A launch to run: the blocks of its slots, in its signature's order, and its params' bytes.
pub struct Case {
    pub slots: Vec<BlockId>,
    pub params: Vec<u8>,
}

/// A version's check against its operation's reference: made only by `Registry::check`, and the only way a version
/// other than the reference is registered.
pub struct Checked {
    op: OpId,
    name: &'static str,
}

/// The operations and their versions.
#[derive(Default)]
pub struct Registry {
    ops: Vec<&'static Signature>,
    versions: Vec<Version>,
}

impl Registry {
    /// An operation, by its signature. Refused past `MAX_SLOTS` slots, under a name already declared, or with a phase
    /// that names no bound on its turns.
    pub fn declare(&mut self, sig: &'static Signature) -> Result<OpId, String> {
        if sig.slots.len() > MAX_SLOTS {
            return Err(format!("{}: {} slots; an operation binds at most {MAX_SLOTS}", sig.name, sig.slots.len()));
        }
        if self.ops.iter().any(|s| s.name == sig.name) {
            return Err(format!("{}: declared twice", sig.name));
        }
        if sig.shape.is_empty() {
            return Err(format!("{}: an operation's shape has at least one phase", sig.name));
        }
        self.ops.push(sig);
        Ok(OpId(self.ops.len() as u32 - 1))
    }

    pub fn signature(&self, op: OpId) -> &'static Signature {
        self.ops[op.0 as usize]
    }

    /// Whether a twin's phases are its version's shape's (the shape it runs the work as, `Version::shape`): the same
    /// names, in order; a phase that runs again has a bound on its turns.
    fn fits(sig: &Signature, v: &Version, twin: Twin) -> Result<(), String> {
        let names = |p: &mut dyn Iterator<Item = &'static str>| p.collect::<Vec<_>>().join(", ");
        if v.shape.len() != twin.len() || v.shape.iter().zip(twin).any(|(p, t)| p.name != t.name) {
            return Err(format!(
                "{}: {}: the twin's phases ({}) are not its shape's ({})",
                sig.name,
                v.name,
                names(&mut twin.iter().map(|t| t.name)),
                names(&mut v.shape.iter().map(|p| p.name))
            ));
        }
        if let Some((p, _)) = v.shape.iter().zip(twin).find(|(p, t)| t.again.is_some() && p.turns() < 2) {
            return Err(format!("{}: phase {} runs again, and its shape bounds no turns", sig.name, p.name));
        }
        // A phase runs again exactly when its units relate by rounds (Operations §4: "rounds to a fixpoint").
        if let Some((p, t)) = v.shape.iter().zip(twin).find(|(p, t)| t.again.is_some() != (p.across == Across::Rounds)) {
            return Err(format!(
                "{}: phase {} {}",
                sig.name,
                p.name,
                if t.again.is_some() { "runs again, and its shape's units do not relate by rounds" } else { "relates its units by rounds, and its twin has no finisher" }
            ));
        }
        Ok(())
    }

    /// An operation's reference: its twin, registered first and once.
    pub fn register_reference(&mut self, v: Version) -> Result<(), String> {
        let sig = *self.ops.get(v.op.0 as usize).ok_or("a version of an operation not declared")?;
        let Impl::Twin(twin) = v.imp else {
            return Err(format!("{}: {} is a reference but not the twin", sig.name, v.name));
        };
        if v.status != Status::Reference {
            return Err(format!("{}: {} is registered as a reference with status {:?}", sig.name, v.name, v.status));
        }
        if !self.versions(v.op).is_empty() {
            return Err(format!("{}: a second reference ({})", sig.name, v.name));
        }
        Self::fits(sig, &v, twin)?;
        self.versions.push(v);
        Ok(())
    }

    /// A version other than the reference, once a test has run it against the reference and found the same facts
    /// (`check`): `checked` names it.
    pub fn register(&mut self, v: Version, checked: Checked) -> Result<(), String> {
        let sig = *self.ops.get(v.op.0 as usize).ok_or("a version of an operation not declared")?;
        if checked.op != v.op || checked.name != v.name {
            return Err(format!("{}: {} was not the version checked ({})", sig.name, v.name, checked.name));
        }
        if v.status == Status::Reference {
            return Err(format!("{}: a second reference ({})", sig.name, v.name));
        }
        if self.versions(v.op).iter().any(|w| w.name == v.name) {
            return Err(format!("{}: version {} registered twice", sig.name, v.name));
        }
        self.versions.push(v);
        Ok(())
    }

    pub fn versions(&self, op: OpId) -> Vec<&Version> {
        self.versions.iter().filter(|v| v.op == op).collect()
    }

    fn reference(&self, op: OpId) -> Option<&Version> {
        self.versions.iter().find(|v| v.op == op && v.status == Status::Reference)
    }

    /// The version the CPU executor runs: a default twin when one is registered, else the reference.
    fn cpu_version(&self, op: OpId) -> Option<&Version> {
        let twin = |status| self.versions.iter().find(|v| v.op == op && v.status == status && matches!(v.imp, Impl::Twin(_)));
        twin(Status::Default).or_else(|| twin(Status::Reference))
    }

    /// Runs `candidate` and the reference on the launches `case` makes (each in a directory of its own), at each of
    /// `tilings`, and compares the facts every written slot holds after, by meaning (F1). The `Checked` it gives is
    /// what `register` takes. A version the CPU cannot run (UKL, compiled) cannot be checked here yet.
    pub fn check(&self, candidate: &Version, case: &dyn Fn(&mut Directory) -> Case, tilings: &[Tiling]) -> Result<Checked, String> {
        let sig = *self.ops.get(candidate.op.0 as usize).ok_or("a version of an operation not declared")?;
        let reference = self.reference(candidate.op).ok_or_else(|| format!("{}: no reference to check against", sig.name))?;
        let Impl::Twin(twin) = candidate.imp else {
            return Err(format!("{}: {} is not a twin, and the CPU runs only twins", sig.name, candidate.name));
        };
        Self::fits(sig, candidate, twin)?;
        let run = |v: &Version, tiling: Tiling| -> Result<Vec<Vec<Vec<i64>>>, String> {
            let mut dir = Directory::default();
            let c = case(&mut dir);
            execute(sig, v, &dir, &c.slots, &c.params, tiling)?;
            written_facts(sig, &dir, &c.slots)
        };
        let want = run(reference, Tiling::WHOLE)?;
        for &t in tilings {
            let got = run(candidate, t)?;
            if got != want {
                return Err(format!("{}: {} at {t:?} wrote other facts than the reference", sig.name, candidate.name));
            }
        }
        Ok(Checked { op: candidate.op, name: candidate.name })
    }
}

/// The facts each written slot of a launch holds, each slot's rows sorted (F1: compared by meaning).
pub fn written_facts(sig: &Signature, dir: &Directory, slots: &[BlockId]) -> Result<Vec<Vec<Vec<i64>>>, String> {
    let mut out = Vec::new();
    for (i, (spec, &id)) in sig.slots.iter().zip(slots).enumerate() {
        if !matches!(spec.mode, Mode::Write | Mode::ReadWrite) {
            continue;
        }
        let mut l = dir.launch("facts", &[(id, false)])?;
        let (pins, table) = l.parts();
        let mut rows = (sig.facts)(&table, i, pins[0].bytes());
        rows.sort();
        out.push(rows);
    }
    Ok(out)
}

/// Runs `op` on the CPU: its default twin, else its reference, on the blocks `slots` name, with `params`' bytes, its
/// phases cut by `tiling`.
///
/// Every slot is pinned first, by its mode, naming the operation, and every block a slot refers to is pinned to read;
/// the CPU's descriptor table of those is written (`Directory::launch`). If any cannot be (a second writer, a writer
/// beside a reader, a write of a frozen block, the wrong kind of block), the operation does not start, every pin
/// taken so far is let go, and the `Err` names the operation, the slot and what holds its block.
pub fn run_cpu(reg: &Registry, dir: &Directory, op: OpId, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
    let sig = reg.signature(op);
    let v = reg.cpu_version(op).ok_or_else(|| format!("{}: no twin registered", sig.name))?;
    execute(sig, v, dir, slots, params, tiling)
}

/// Item `i` of a shuffle of `0..n` by `seed` (Fisher–Yates, splitmix64): the order tiles run in.
fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut z = seed;
    for i in (1..n).rev() {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let j = (rerail_blocks::layer::mix(z) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

fn execute(sig: &Signature, v: &Version, dir: &Directory, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
    let Impl::Twin(twin) = v.imp else {
        return Err(format!("{}: {} is not a twin; the CPU executor runs twins", sig.name, v.name));
    };
    if slots.len() != sig.slots.len() {
        return Err(format!("{}: {} blocks for {} slots", sig.name, slots.len(), sig.slots.len()));
    }
    if params.len() != sig.params.size {
        return Err(format!("{}: {} bytes of params for {} ({})", sig.name, params.len(), sig.params.name, sig.params.size));
    }
    for (i, (spec, &id)) in sig.slots.iter().zip(slots).enumerate() {
        let b = dir.block(id).ok_or_else(|| format!("{} could not start: slot {i} ({}): {id} is held by nothing", sig.name, spec.name))?;
        let kind = b.header().kind;
        if kind != spec.kind {
            return Err(format!("{} could not start: slot {i} ({}) takes a block of kind {}, given one of kind {kind}", sig.name, spec.name, spec.kind));
        }
    }
    // Items that are not independent run whole and in order, whatever tiling is asked: only an operation that says
    // the axis its items are independent along is cut into tiles and run in any order.
    let tiling = match sig.independent {
        Independence::Along(_) => tiling,
        Independence::Not(_) => Tiling::WHOLE,
    };
    let modes: Vec<(BlockId, bool)> = slots.iter().zip(sig.slots).map(|(&id, s)| (id, s.mode.writes())).collect();
    let mut launch = dir.launch(sig.name, &modes)?;
    let (pins, table) = launch.parts();
    let mut slot_views: Vec<Slot<'_>> = pins.iter_mut().map(|p| if p.writes() { Slot::Write(p) } else { Slot::Read(p) }).collect();
    let mut ctx = Ctx { slots: &mut slot_views, table: &table, params };
    // The version's shape is what it runs by (`Registry::fits` checked its twin against it), axis by axis:
    //
    // - how work is found: a phase of a fixed count has the count it had when the launch started (no earlier phase
    //   made it); an indirect one's is made by the phases before it;
    // - across units: tiles whose units share nothing, combine associatively or make a round run in any order; units
    //   in order (a carry from one to the next), or any other relation, run in order;
    // - its bounds: a phase that runs again stops at its bound on turns, past which the launch fails.
    let fixed: Vec<Option<u32>> = v.shape.iter().zip(twin).map(|(p, tp)| (p.found == Found::Fixed).then(|| (tp.items)(&ctx))).collect();
    for ((phase, tp), fixed) in v.shape.iter().zip(twin).zip(fixed) {
        let mut turn = 0;
        loop {
            turn += 1;
            let n = (tp.items)(&ctx) as usize;
            if let Some(f) = fixed
                && f as usize != n
            {
                return Err(format!(
                    "{}: phase {} has a fixed count of items, {f} when the launch started, and {n} when it ran: an earlier phase made it (an indirect count)",
                    sig.name, phase.name
                ));
            }
            let t = if tiling.tile == 0 { n.max(1) } else { tiling.tile as usize };
            let tiles = n.div_ceil(t);
            let any_order = matches!(phase.across, Across::Nothing | Across::Combine | Across::Rounds);
            let order = if tiling.seed == 0 || !any_order { (0..tiles).collect() } else { shuffled(tiles, tiling.seed ^ turn as u64) };
            for k in order {
                let lo = k * t;
                (tp.run)(&mut ctx, Tile { lo: lo as u32, hi: (lo + t).min(n) as u32 });
            }
            match tp.again {
                Some(again) if again(&mut ctx) => {
                    if turn >= phase.turns() {
                        return Err(format!("{}: phase {} ran past its bound of {} turns", sig.name, phase.name, phase.turns()));
                    }
                }
                _ => break,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "op_tests.rs"]
mod tests;
