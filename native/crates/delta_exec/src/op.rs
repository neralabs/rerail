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
//! is the version's alone.
//!
//! A **UKL version** (`Impl::Ukl`) runs on a device the crate above gives (`Kernels`: this crate compiles and launches
//! nothing, B§7), by the same driver and the same shape: the launch is pinned as a twin's is, its device descriptor
//! table made (E2), both handed to one fence (E1), and each phase's kernels launched by tiles in any order; the fence
//! completes when the last has. So `Registry::check_on` checks it against the twin (Operations, revision 4: "UKL
//! versions after the twins, checked against them on ukl_vec"), and `run_on` runs a device's default version.

use crate::dir::{BlockId, Directory, Launch};
use rerail_blocks::ledger::Fence;
use std::sync::Arc;
use rerail_blocks::gens::Table;
use rerail_delta_twins::{Ctx, Field, Meter, Params, Slot, Tile, Twin};

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

/// A bound on a phase, each held by the executor (`drive`): an item past it fails the launch, or, for capacity, the
/// launch refuses, grows the slot and runs the phase again, or runs again from its first phase.
///
/// The launch budget (0540, 0601) is not a phase's bound: every launch counts in one ledger (`Directory::charge`), each
/// phase's turn one, and one past the evaluation's budget fails.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    /// Steps an item (`Ctx::steps`).
    Steps(u32),
    /// Turns of a phase that runs again (a queue's, a fixpoint's rounds, a merge's passes): past it, a hard failure.
    Turns(u32),
    /// What an item that finds a written slot too small does (`Ctx::room`): refuse, grow the slot's capacity within its
    /// reservation and run the phase again, or run the launch again from its first phase at the capacity it needs.
    Capacity(Over),
    /// A stack's depth (`Ctx::depth`): a phase whose units hold stacks (`Inside::Stacks`) bounds them.
    StackDepth(u32),
    /// A sample's size (`Ctx::sample`).
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

    /// Its bound on an item's steps.
    pub fn steps(&self) -> Option<u32> {
        self.bounds.iter().find_map(|b| if let Bound::Steps(n) = b { Some(*n) } else { None })
    }

    /// Its bound on a stack's depth.
    pub fn depth(&self) -> Option<u32> {
        self.bounds.iter().find_map(|b| if let Bound::StackDepth(n) = b { Some(*n) } else { None })
    }

    /// Its sample's size.
    pub fn sample(&self) -> Option<u32> {
        self.bounds.iter().find_map(|b| if let Bound::Sample(n) = b { Some(*n) } else { None })
    }

    /// What it does past a written slot's capacity.
    pub fn capacity(&self) -> Option<Over> {
        self.bounds.iter().find_map(|b| if let Bound::Capacity(o) = b { Some(*o) } else { None })
    }

    /// Whether its bounds fit its axes (Operations §4): each kind of bound once; a unit holding stacks bounds their
    /// depth, and only such a unit does; output by an atomic claim bounds its capacity (claim and rerun, shape 12); a
    /// phase by rounds bounds its turns.
    fn fits_axes(&self) -> Result<(), String> {
        let kind = |b: &Bound| std::mem::discriminant(b);
        if let Some((i, _)) = self.bounds.iter().enumerate().find(|(i, b)| self.bounds[..*i].iter().any(|a| kind(a) == kind(b))) {
            return Err(format!("phase {}: its bound {:?} is given twice", self.name, self.bounds[i]));
        }
        if (self.inside == Inside::Stacks) != self.depth().is_some() {
            return Err(format!(
                "phase {}: {}",
                self.name,
                if self.inside == Inside::Stacks { "its units hold stacks, and it bounds no stack depth" } else { "bounds a stack's depth, and its units hold no stacks" }
            ));
        }
        if self.output == Output::Claim && self.capacity().is_none() {
            return Err(format!("phase {}: its output is claimed, and it bounds no capacity", self.name));
        }
        if self.across == Across::Rounds && self.turns() < 2 {
            return Err(format!("phase {}: relates its units by rounds, and its shape bounds no turns", self.name));
        }
        Ok(())
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
    /// UKL `source`, its kernels named from `entry` (`Kernels`): phase `p`'s `<entry>_<p>`, a thread an item; its item
    /// count's `<entry>_<p>_items`, and, for a phase by rounds, whether it runs again, `<entry>_<p>_again`.
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
    pub const OPENCL: Devices = Devices(16);

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
        Self::fits_bounds(sig, sig.shape)?;
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
        Self::fits_bounds(sig, v.shape)?;
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

    /// Whether a shape's bounds fit its axes (`Phase::fits_axes`), and a launch run again from its first phase (a
    /// capacity's `Rerun`) writes no slot it reads: what the first run wrote there would be read by the second.
    fn fits_bounds(sig: &Signature, shape: &[Phase]) -> Result<(), String> {
        for p in shape {
            p.fits_axes().map_err(|e| format!("{}: {e}", sig.name))?;
        }
        if let Some(p) = shape.iter().find(|p| p.capacity() == Some(Over::Rerun))
            && let Some(s) = sig.slots.iter().find(|s| s.mode == Mode::ReadWrite)
        {
            return Err(format!("{}: phase {} runs the launch again past a capacity, and slot {} is read and written", sig.name, p.name, s.name));
        }
        Ok(())
    }

    /// A UKL version's shape: it declares no twin's phases, so its own shape is what it runs by, its bounds fitting its
    /// axes. A bound an item reports (steps, a stack's depth, a sample, a slot's room) is metered for twins only: a
    /// kernel's report of it is not built, so a UKL version declaring one is refused.
    fn fits_shape(sig: &Signature, v: &Version) -> Result<(), String> {
        Self::fits_bounds(sig, v.shape)?;
        if let Some(p) = v.shape.iter().find(|p| p.bounds.iter().any(|b| !matches!(b, Bound::Turns(_)))) {
            return Err(format!("{}: {}: phase {} has a bound its items report, metered for twins only", sig.name, v.name, p.name));
        }
        Ok(())
    }

    /// The version `device` runs `op` by: its default UKL version for the device's processors, else none.
    fn device_version(&self, op: OpId, device: &dyn Kernels) -> Option<&Version> {
        self.versions.iter().find(|v| v.op == op && v.status == Status::Default && matches!(v.imp, Impl::Ukl { .. }) && v.devices.has(device.devices()))
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
    /// what `register` takes. A UKL version runs on a device (`check_on`); a compiled one is not built.
    pub fn check(&self, candidate: &Version, case: &dyn Fn(&mut Directory) -> Case, tilings: &[Tiling]) -> Result<Checked, String> {
        self.check_with(candidate, None, case, tilings)
    }

    /// `check`, a UKL version run on `device` (the reference on the CPU, as ever).
    pub fn check_on(&self, candidate: &Version, device: &dyn Kernels, case: &dyn Fn(&mut Directory) -> Case, tilings: &[Tiling]) -> Result<Checked, String> {
        self.check_with(candidate, Some(device), case, tilings)
    }

    fn check_with(&self, candidate: &Version, device: Option<&dyn Kernels>, case: &dyn Fn(&mut Directory) -> Case, tilings: &[Tiling]) -> Result<Checked, String> {
        let sig = *self.ops.get(candidate.op.0 as usize).ok_or("a version of an operation not declared")?;
        let reference = self.reference(candidate.op).ok_or_else(|| format!("{}: no reference to check against", sig.name))?;
        match candidate.imp {
            Impl::Twin(twin) => Self::fits(sig, candidate, twin)?,
            Impl::Ukl { .. } => Self::fits_shape(sig, candidate)?,
            Impl::Compiled(_) => return Err(format!("{}: {} is compiled ahead, which is not built", sig.name, candidate.name)),
        }
        let run = |v: &Version, tiling: Tiling| -> Result<Vec<Vec<Vec<i64>>>, String> {
            let mut dir = Directory::default();
            let c = case(&mut dir);
            execute(sig, v, &dir, &c.slots, &c.params, tiling, device)?;
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
    execute(sig, v, dir, slots, params, tiling, None)
}

/// Runs `op` on `device`: its default UKL version for the device's processors (a version is registered only once
/// checked against the twin), else its twin on the CPU (`run_cpu`). Its slots pinned and its descriptor table made as
/// `run_cpu`'s, then handed to the device (E1, E2) until its work completes.
pub fn run_on(reg: &Registry, dir: &Directory, op: OpId, device: &dyn Kernels, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
    let sig = reg.signature(op);
    match reg.device_version(op, device) {
        Some(v) => execute(sig, v, dir, slots, params, tiling, Some(device)),
        None => run_cpu(reg, dir, op, slots, params, tiling),
    }
}

/// A device a UKL version's kernels run on (E3), given by the crate above that holds one (its compiler, its queue):
/// this crate makes the launch and drives its phases, and launches nothing itself (B§7).
pub trait Kernels {
    /// The processors it is: a version runs on it when its `devices` has them.
    fn devices(&self) -> Devices;
    /// The memory domain its kernels read (0: the host's, each block read in place; 1 and up, a discrete GPU's, each
    /// block's replica there, `Directory::replicate`).
    fn domain(&self) -> u32;
    /// Binds `launch` on the device: its descriptor table there (E2, `gens::device_table`), over every block the
    /// launch pins, that table's block handed to `fence` (E1); the kernels' other arguments are `slots` (the slots'
    /// block ids, in order) and `params`. The launch's own pins are handed to `fence` after.
    fn bind(&self, launch: &Launch, fence: &Arc<Fence>, slots: &[u32], params: &[u8]) -> Result<Box<dyn OnDevice + '_>, String>;
}

/// A launch bound on a device (`Kernels::bind`).
pub trait OnDevice {
    /// Runs kernel `name` of `source` on items `lo..hi`, a thread an item (item `lo + i` for thread `i`), and waits
    /// for it: the word it wrote to its result (an item count, or whether its phase runs again; 0 otherwise).
    fn run(&mut self, source: &'static str, name: &str, lo: u32, hi: u32) -> Result<u64, String>;
}

/// What runs a version's phases: its twin's calls, or a device's kernels.
trait Runs {
    fn items(&mut self, p: usize) -> Result<u32, String>;
    fn run(&mut self, p: usize, t: Tile) -> Result<(), String>;
    /// Whether phase `p` runs again (a phase by rounds; asked after each turn).
    fn again(&mut self, p: usize) -> Result<bool, String>;
    /// The meter of its items' bounded work, where they report it (a twin's; a kernel's report is not built).
    fn meter(&mut self) -> Option<&mut Meter>;
}

struct TwinRuns<'s, 'p, 't> {
    ctx: Ctx<'s, 'p, 't>,
    twin: Twin,
}

impl Runs for TwinRuns<'_, '_, '_> {
    fn items(&mut self, p: usize) -> Result<u32, String> {
        Ok((self.twin[p].items)(&self.ctx))
    }
    fn run(&mut self, p: usize, t: Tile) -> Result<(), String> {
        (self.twin[p].run)(&mut self.ctx, t);
        Ok(())
    }
    fn again(&mut self, p: usize) -> Result<bool, String> {
        Ok(self.twin[p].again.is_some_and(|again| again(&mut self.ctx)))
    }
    fn meter(&mut self) -> Option<&mut Meter> {
        Some(&mut *self.ctx.meter)
    }
}

/// A UKL version's kernels on a bound launch.
struct UklRuns<'b> {
    bound: Box<dyn OnDevice + 'b>,
    source: &'static str,
    entry: &'static str,
    shape: &'static [Phase],
}

impl Runs for UklRuns<'_> {
    fn items(&mut self, p: usize) -> Result<u32, String> {
        let n = self.bound.run(self.source, &format!("{}_{}_items", self.entry, self.shape[p].name), 0, 1)?;
        u32::try_from(n).map_err(|_| format!("phase {}: {n} items", self.shape[p].name))
    }
    fn run(&mut self, p: usize, t: Tile) -> Result<(), String> {
        if t.lo < t.hi {
            self.bound.run(self.source, &format!("{}_{}", self.entry, self.shape[p].name), t.lo, t.hi)?;
        }
        Ok(())
    }
    fn again(&mut self, p: usize) -> Result<bool, String> {
        if !matches!(self.shape[p].across, Across::Rounds) {
            return Ok(false);
        }
        Ok(self.bound.run(self.source, &format!("{}_{}_again", self.entry, self.shape[p].name), 0, 1)? != 0)
    }
    fn meter(&mut self) -> Option<&mut Meter> {
        None
    }
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

fn execute(sig: &Signature, v: &Version, dir: &Directory, slots: &[BlockId], params: &[u8], tiling: Tiling, device: Option<&dyn Kernels>) -> Result<(), String> {
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
    // Its launches count in the open evaluation, or in one of its own (0601).
    let _evaluation = dir.evaluation(crate::dir::LAUNCH_BUDGET);
    let modes: Vec<(BlockId, bool)> = slots.iter().zip(sig.slots).map(|(&id, s)| (id, s.mode.writes())).collect();
    let mut reran = false;
    loop {
        let mut launch = dir.launch(sig.name, &modes)?;
        let drove = match v.imp {
            Impl::Twin(twin) => {
                let mut meter = Meter { capacity: slots.iter().map(|&id| dir.capacity(id)).collect(), ..Meter::default() };
                let (pins, table) = launch.parts();
                let mut slot_views: Vec<Slot<'_>> = pins.iter_mut().map(|p| if p.writes() { Slot::Write(p) } else { Slot::Read(p) }).collect();
                let ctx = Ctx { slots: &mut slot_views, table: &table, params, meter: &mut meter };
                drive(sig, v, tiling, dir, slots, &mut TwinRuns { ctx, twin })?
            }
            Impl::Ukl { source, entry } => return on_device(sig, v, dir, slots, params, tiling, device, launch, source, entry),
            Impl::Compiled(_) => return Err(format!("{}: {} is compiled ahead, which is not built", sig.name, v.name)),
        };
        let needs = match drove {
            Drove::Done => return Ok(()),
            Drove::Rerun(needs) => needs,
        };
        // Run again from the first phase, once, each slot that needed room grown to what it needs: past that, the need
        // was not the exact size, and the launch fails.
        drop(launch);
        if reran {
            return Err(format!("{}: run again at the capacity its slots needed, and they needed more: {needs:?}", sig.name));
        }
        for (s, need) in needs {
            dir.grow(slots[s], need).map_err(|e| format!("{}: slot {} ({}): {e}", sig.name, s, sig.slots[s].name))?;
        }
        reran = true;
    }
}

/// A UKL version's launch on `device`: bound there, handed to its fence, its phases driven.
#[allow(clippy::too_many_arguments)]
fn on_device(
    sig: &Signature,
    v: &Version,
    dir: &Directory,
    slots: &[BlockId],
    params: &[u8],
    tiling: Tiling,
    device: Option<&dyn Kernels>,
    launch: Launch,
    source: &'static str,
    entry: &'static str,
) -> Result<(), String> {
    let d = device.ok_or_else(|| format!("{}: {} is a UKL version, and no device was given to run it on", sig.name, v.name))?;
    if !v.devices.has(d.devices()) {
        return Err(format!("{}: {} runs on {:?}, not on the device's {:?}", sig.name, v.name, v.devices, d.devices()));
    }
    let ids: Vec<u32> = slots.iter().map(|s| s.0).collect(); // copy: not block data (the slots' ids, an argument)
    let fence = Fence::new();
    // Every use the launch hands the fence ends with it, whatever the work did.
    let done = |r: Result<(), String>| {
        fence.mark_committed();
        fence.complete();
        r
    };
    let bound = match d.bind(&launch, &fence, &ids, params) {
        Ok(b) => b,
        Err(e) => return done(Err(e)),
    };
    if let Err(b) = launch.to_fence(&fence, sig.name, d.domain()) {
        drop(bound);
        return done(Err(b.1));
    }
    let r = done(drive(sig, v, tiling, dir, slots, &mut UklRuns { bound, source, entry, shape: v.shape }).and_then(|d| match d {
        Drove::Done => Ok(()),
        Drove::Rerun(_) => Err(format!("{}: a kernel's room is not metered", sig.name)),
    }));
    // A device domain of its own: what the kernels wrote comes back to the host's bytes (`Region::sync_host`).
    if d.domain() != 0 {
        for (spec, &id) in sig.slots.iter().zip(slots) {
            if let (true, Some(b)) = (spec.mode.writes(), dir.block(id)) {
                rerail_blocks::region::whole(&b).region().sync_host();
            }
        }
    }
    r
}

/// How a launch's phases ended: every one ran, or one asked for the launch to run again at the capacities it needs.
enum Drove {
    Done,
    Rerun(Vec<(usize, usize)>),
}

/// Runs a version's phases by its shape, axis by axis:
///
/// - how work is found: a phase of a fixed count has the count it had when the launch started (no earlier phase
///   made it); an indirect one's is made by the phases before it;
/// - across units: tiles whose units share nothing, combine associatively or make a round run in any order; units
///   in order (a carry from one to the next), or any other relation, run in order;
/// - its bounds: each turn is a launch, charged to the evaluation (0601); an item past its bound on steps, a stack's
///   depth or a sample fails the launch, as does a report of one its phase does not bound; a slot found too small is
///   refused, grown within its reservation and the phase run again, or the launch run again, as its capacity bound
///   says; a phase that runs again stops at its bound on turns, past which the launch fails.
fn drive(sig: &Signature, v: &Version, tiling: Tiling, dir: &Directory, slots: &[BlockId], r: &mut dyn Runs) -> Result<Drove, String> {
    let mut fixed = Vec::with_capacity(v.shape.len());
    for (k, p) in v.shape.iter().enumerate() {
        fixed.push(if p.found == Found::Fixed { Some(r.items(k)?) } else { None });
    }
    for (k, (phase, fixed)) in v.shape.iter().zip(fixed).enumerate() {
        if let Some(m) = r.meter() {
            m.steps = phase.steps();
            m.depth = phase.depth();
            m.sample = phase.sample();
            m.bounded_room = phase.capacity().is_some();
        }
        let (mut turn, mut grown) = (0, 0);
        loop {
            turn += 1;
            dir.charge(sig.name, phase.name)?;
            let n = r.items(k)? as usize;
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
            for k2 in order {
                let lo = k2 * t;
                r.run(k, Tile { lo: lo as u32, hi: (lo + t).min(n) as u32 })?;
                if let Some(f) = r.meter().and_then(|m| m.fault.take()) {
                    return Err(format!("{}: phase {}: {f}", sig.name, phase.name));
                }
            }
            if let Some(m) = r.meter()
                && !m.needs.is_empty()
            {
                let needs = std::mem::take(&mut m.needs);
                let (s, need) = needs[0];
                match phase.capacity() {
                    Some(Over::Grow) => {
                        grown += 1;
                        if grown > 32 {
                            return Err(format!("{}: phase {}: grown 32 times, and its slots need more: {needs:?}", sig.name, phase.name));
                        }
                        for (s, need) in needs {
                            m.capacity[s] = dir.grow(slots[s], need).map_err(|e| format!("{}: phase {}: slot {s} ({}): {e}", sig.name, phase.name, sig.slots[s].name))?;
                        }
                        // The same turn again, over the grown slots: what its items wrote is found, not written twice.
                        turn -= 1;
                        continue;
                    }
                    Some(Over::Rerun) => return Ok(Drove::Rerun(needs)),
                    Some(Over::Refuse) | None => {
                        return Err(format!(
                            "{}: phase {}: slot {s} ({}) needs a capacity of {need} bytes, and has {}: refused",
                            sig.name, phase.name, sig.slots[s].name, m.capacity[s]
                        ));
                    }
                }
            }
            if r.again(k)? {
                if turn >= phase.turns() {
                    return Err(format!("{}: phase {} ran past its bound of {} turns", sig.name, phase.name, phase.turns()));
                }
            } else {
                break;
            }
        }
    }
    Ok(Drove::Done)
}

#[cfg(test)]
#[path = "op_tests.rs"]
mod tests;
