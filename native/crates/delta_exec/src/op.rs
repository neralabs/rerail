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
use rerail_delta_twins::{Ctx, Field, Params, Slot, Tile, Twin};
pub use rerail_delta_twins::Meter;

/// The most slots an operation binds: the twins' one definition (`rerail_delta_twins::MAX_SLOTS`).
pub use rerail_delta_twins::MAX_SLOTS;

/// The most instances a wide launch runs (`Runner::run_wide`; `<ukl/delta/op.h>`'s `DELTA_MAX_WIDE`).
pub const MAX_WIDE: usize = 32;

/// One instance of a wide launch, as its device binds it: its slots' block ids, in order, and its params' bytes.
pub struct Instance<'a> {
    pub slots: &'a [u32],
    pub params: &'a [u8],
}

/// A wide launch's instances, as the executor takes them: each one's slots and params.
pub type Wide<'a> = [(&'a [BlockId], &'a [u8])];

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

/// An operation's interface, as UKL declares it (`operations.uglykl`, `<ukl/operation.h>`'s `DECLARE_KERNEL2`): its
/// name, its slots (each a buffer row: its access the slot's mode, its type its block's kind) and its params'
/// structure's name. Read into `interfaces` by the build; every version of the operation runs against it.
pub struct Interface {
    pub name: &'static str,
    pub slots: &'static [SlotSpec],
    pub params: &'static str,
}

/// The table of interfaces: one an operation, read from `operations.uglykl` (`build.rs`).
pub mod interfaces {
    use super::{Interface, Mode, SlotSpec};
    use rerail_blocks::block::kind;
    include!(concat!(env!("OUT_DIR"), "/interfaces.rs"));
}

/// Whether two names are the same, in a constant.
const fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// What an operation is: its interface (its name, slots and params), shape, the axis its items are independent along,
/// and how its written slots' facts are read. Declared once, as a constant, from its UKL interface
/// (`Signature::of`); `Registry::declare` takes it.
pub struct Signature {
    pub name: &'static str,
    pub slots: &'static [SlotSpec],
    pub params: ParamsLayout,
    pub shape: &'static [Phase],
    pub independent: Independence,
    pub facts: FactsFn,
}

impl Signature {
    /// The signature of the operation `interface` declares: its params `params`, whose structure must be the one its
    /// `ARGS` row names, and more than `MAX_SLOTS` slots fail where it is declared (at compile time for a constant).
    pub const fn of(interface: &'static Interface, params: ParamsLayout, shape: &'static [Phase], independent: Independence, facts: FactsFn) -> Signature {
        assert!(same_name(interface.params, params.name), "an operation's params are not the structure its UKL interface names");
        Signature::parts(interface.name, interface.slots, params, shape, independent, facts)
    }

    /// A signature not declared in UKL: a test's own operation, never the engine's.
    #[cfg(any(test, feature = "test-hooks"))]
    pub const fn new(
        name: &'static str,
        slots: &'static [SlotSpec],
        params: ParamsLayout,
        shape: &'static [Phase],
        independent: Independence,
        facts: FactsFn,
    ) -> Signature {
        Signature::parts(name, slots, params, shape, independent, facts)
    }

    const fn parts(
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
    /// A host version (*0686: Operations* §10's host work: `answer_load`, the file layer as it is; `parse`, the rr
    /// parser, host for now): its phases run on the CPU over the slots as a twin's do, written in this crate because
    /// they call what the twins may not (the observer, the store's objects, the parser). The reference until a device
    /// version registers, checked against it.
    Host(Twin),
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
    /// An operation's host version as its reference (`Impl::Host`), running the work as the signature's shape.
    pub fn host(op: OpId, name: &'static str, phases: Twin, shape: &'static [Phase]) -> Version {
        Version { imp: Impl::Host(phases), ..Version::twin(op, name, phases, shape) }
    }

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

    /// The operation `sig` declares: the one declared by that signature.
    pub fn find(&self, sig: &'static Signature) -> Option<OpId> {
        self.ops.iter().position(|s| std::ptr::eq(*s, sig)).map(|i| OpId(i as u32))
    }

    pub fn signature(&self, op: OpId) -> &'static Signature {
        self.ops[op.0 as usize]
    }

    /// The `i`-th operation declared, by its place (`None` past the last).
    pub fn signature_at(&self, i: u32) -> Option<&'static Signature> {
        self.ops.get(i as usize).copied()
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
    /// axes. A kernel reports a slot's room (`OnDevice::run`); a bound on steps, a stack's depth or a sample is metered
    /// for twins only (a kernel's report of it is not built), so a UKL version declaring one is refused.
    fn fits_shape(sig: &Signature, v: &Version) -> Result<(), String> {
        Self::fits_bounds(sig, v.shape)?;
        if let Some(p) = v.shape.iter().find(|p| p.bounds.iter().any(|b| !matches!(b, Bound::Turns(_) | Bound::Capacity(_)))) {
            return Err(format!("{}: {}: phase {} has a bound its items report, metered for twins only", sig.name, v.name, p.name));
        }
        Ok(())
    }

    /// The version `device` runs `op` by: its default UKL version for the device's processors, else none.
    fn device_version(&self, op: OpId, device: &dyn Kernels) -> Option<&Version> {
        self.versions.iter().find(|v| v.op == op && v.status == Status::Default && matches!(v.imp, Impl::Ukl { .. }) && v.devices.has(device.devices()))
    }

    /// An operation's reference: its twin (or, for host work, its host version), registered first and once.
    pub fn register_reference(&mut self, v: Version) -> Result<(), String> {
        let sig = *self.ops.get(v.op.0 as usize).ok_or("a version of an operation not declared")?;
        let (Impl::Twin(twin) | Impl::Host(twin)) = v.imp else {
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

    pub(crate) fn reference(&self, op: OpId) -> Option<&Version> {
        self.versions.iter().find(|v| v.op == op && v.status == Status::Reference)
    }

    /// The version the CPU executor runs: a default twin when one is registered, else the reference.
    fn cpu_version(&self, op: OpId) -> Option<&Version> {
        let twin = |status| self.versions.iter().find(|v| v.op == op && v.status == status && matches!(v.imp, Impl::Twin(_) | Impl::Host(_)));
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
            Impl::Twin(twin) | Impl::Host(twin) => Self::fits(sig, candidate, twin)?,
            Impl::Ukl { .. } => Self::fits_shape(sig, candidate)?,
            Impl::Compiled(_) => return Err(format!("{}: {} is compiled ahead, which is not built", sig.name, candidate.name)),
        }
        let run = |v: &Version, tiling: Tiling| -> Result<Vec<Vec<Vec<i64>>>, String> {
            let mut dir = Directory::default();
            let c = case(&mut dir);
            execute(sig, v, &dir, &[(&c.slots, &c.params)], tiling, device)?;
            written_facts(sig, &dir, &c.slots)
        };
        let want = run(reference, Tiling::WHOLE)?;
        for &t in tilings {
            let got = run(candidate, t)?;
            if got != want {
                return Err(format!("{}: {} at {t:?} wrote other facts than the reference", sig.name, candidate.name));
            }
        }
        // Wide: the case made twice, run as one launch of two instances; each writes what the reference wrote when
        // the same two are made the same way (a fact may name a block) and each run alone.
        let two = |wide: bool, v: &Version| -> Result<Vec<Vec<Vec<Vec<i64>>>>, String> {
            let mut dir = Directory::default();
            let (a, b) = (case(&mut dir), case(&mut dir));
            if wide {
                execute(sig, v, &dir, &[(&a.slots, &a.params), (&b.slots, &b.params)], Tiling::WHOLE, device)?;
            } else {
                for c in [&a, &b] {
                    execute(sig, v, &dir, &[(&c.slots, &c.params)], Tiling::WHOLE, device)?;
                }
            }
            [a, b].iter().map(|c| written_facts(sig, &dir, &c.slots)).collect()
        };
        let (want, got) = (two(false, reference)?, two(true, candidate)?);
        if let Some(k) = (0..2).find(|&k| got[k] != want[k]) {
            return Err(format!("{}: {} wrote other facts than the reference as instance {k} of a wide launch", sig.name, candidate.name));
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

/// The operations runner (0686, the runner's step 3): the one registry, every operation declared from its UKL interface
/// and its twin registered (`Runner::new`). Nothing runs an operation but the runner: a caller asks it to run one by
/// its signature (`run`), and it takes the operation's implementation from the registry (today the twin, on the
/// CPU; a device's checked UKL version by `run_on`), pins its slots, writes its descriptor table and runs it.
pub struct Runner {
    reg: Registry,
    /// The device it runs on (`Runner::on`), each operation's checked UKL version there; none: the CPU, the twins.
    device: Option<Box<dyn Kernels>>,
    /// A mixed runner's seed (`Runner::mixed`), then each launch's, and its launches on the CPU and on the device.
    mix: Option<Mix>,
    /// Its launches so far (`run`, and each `MAX_WIDE` of a `run_wide`), wherever each ran.
    launched: std::sync::atomic::AtomicU64,
}

/// Each launch of a mixed runner on the CPU or its device, by a seed (`Runner::mixed`).
struct Mix {
    seed: std::sync::atomic::AtomicU64,
    on: [std::sync::atomic::AtomicU64; 2],
}

impl Runner {
    /// The engine's runner: each module's operations declared and their twins registered as references, here alone.
    pub fn new() -> Result<Runner, String> {
        let mut reg = Registry::default();
        crate::trace::register(&mut reg)?;
        crate::cache::register(&mut reg)?;
        crate::answer::register(&mut reg)?;
        crate::intern::register(&mut reg)?;
        crate::k0::register(&mut reg)?;
        crate::drop::register(&mut reg)?;
        crate::semi::register(&mut reg)?;
        Ok(Runner { reg, device: None, mix: None, launched: 0.into() })
    }

    /// A runner on `device`: every operation's UKL version checked there against its twin and registered
    /// (`crate::ukl::register_on`), so `run` runs it there; an operation with none (a host version's) runs its twin.
    pub fn on(device: Box<dyn Kernels>) -> Result<Runner, String> {
        let mut r = Runner::new()?;
        crate::ukl::register_on(&mut r.reg, &*device)?;
        r.device = Some(device);
        Ok(r)
    }

    /// A runner of a test's own registry (its own operations and versions).
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn of(reg: Registry) -> Runner {
        Runner { reg, device: None, mix: None, launched: 0.into() }
    }

    /// A runner on `device` (`Runner::on`) that runs each launch there or on the CPU, picked by `seed`: both read and
    /// write the same blocks where they lie, so a run's result is the same whichever runs each launch.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn mixed(device: Box<dyn Kernels>, seed: u64) -> Result<Runner, String> {
        let mut r = Runner::on(device)?;
        r.mix = Some(Mix { seed: seed.into(), on: [0.into(), 0.into()] });
        Ok(r)
    }

    /// Its launches so far, wherever each ran: what a test counts a run's launches by (L4: an identity run's are its
    /// checks alone).
    pub fn launches(&self) -> u64 {
        self.launched.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A mixed runner's launches so far, on the CPU and on its device.
    pub fn mixed_launches(&self) -> Option<[u64; 2]> {
        use std::sync::atomic::Ordering::Relaxed;
        self.mix.as_ref().map(|m| [m.on[0].load(Relaxed), m.on[1].load(Relaxed)])
    }

    /// Its registry: what is declared and registered (to check a version against the reference, to list versions).
    pub fn registry(&self) -> &Registry {
        &self.reg
    }

    /// A version other than the reference, checked against it (`Registry::check`), registered in the runner's
    /// registry: a default one is what the runner runs from then on.
    pub fn register(&mut self, v: Version, checked: Checked) -> Result<(), String> {
        self.reg.register(v, checked)
    }

    /// The operation `sig` declares, by its place in the registry (a version of it names it); an operation never
    /// declared is an error.
    pub fn op(&self, sig: &'static Signature) -> Result<OpId, String> {
        self.reg.find(sig).ok_or_else(|| format!("{}: not declared in the runner's registry", sig.name))
    }

    /// Runs the operation `sig` declares on the CPU (`run_cpu`): on the blocks `slots` name, with `params`' bytes,
    /// its phases cut by `tiling`.
    pub fn run(&self, dir: &Directory, sig: &'static Signature, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
        self.launched.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let device = self.device.as_ref().filter(|_| {
            let Some(m) = &self.mix else { return true };
            use std::sync::atomic::Ordering::Relaxed;
            let s = rerail_blocks::layer::mix(m.seed.fetch_add(1, Relaxed));
            m.on[(s & 1) as usize].fetch_add(1, Relaxed);
            s & 1 == 1
        });
        match device {
            Some(d) => run_on(&self.reg, dir, self.op(sig)?, &**d, slots, params, tiling),
            None => run_cpu(&self.reg, dir, self.op(sig)?, slots, params, tiling),
        }
    }

    /// Runs the operation `sig` declares as one launch over the instances `wide` (each its slots and params, no block
    /// in two), on the device when the runner has one (`Runner::run`): every phase over all their items at once, each
    /// item its instance's. Past `MAX_WIDE` instances, a launch each `MAX_WIDE`.
    pub fn run_wide(&self, dir: &Directory, sig: &'static Signature, wide: &Wide<'_>, tiling: Tiling) -> Result<(), String> {
        let op = self.op(sig)?;
        for part in wide.chunks(MAX_WIDE) {
            self.launched.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let device = self.device.as_ref().filter(|_| {
                let Some(m) = &self.mix else { return true };
                use std::sync::atomic::Ordering::Relaxed;
                let s = rerail_blocks::layer::mix(m.seed.fetch_add(1, Relaxed));
                m.on[(s & 1) as usize].fetch_add(1, Relaxed);
                s & 1 == 1
            });
            let sig = self.reg.signature(op);
            match device.and_then(|d| self.reg.device_version(op, &**d).map(|v| (d, v))) {
                Some((d, v)) => execute(sig, v, dir, part, tiling, Some(&**d))?,
                None => execute(sig, self.reg.cpu_version(op).ok_or_else(|| format!("{}: no twin registered", sig.name))?, dir, part, tiling, None)?,
            }
        }
        Ok(())
    }

    /// Runs the operation `sig` declares on `device` (`run_on`): its checked UKL version there, else its twin.
    pub fn run_on(&self, dir: &Directory, sig: &'static Signature, device: &dyn Kernels, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
        run_on(&self.reg, dir, self.op(sig)?, device, slots, params, tiling)
    }
}

/// Runs `op` on the CPU: its default twin, else its reference, on the blocks `slots` name, with `params`' bytes, its
/// phases cut by `tiling`. The runner's (`Runner::run`): nothing else calls it but the registry's own tests.
///
/// Every slot is pinned first, by its mode, naming the operation, and every block a slot refers to is pinned to read;
/// the CPU's descriptor table of those is written (`Directory::launch`). If any cannot be (a second writer, a writer
/// beside a reader, a write of a frozen block, the wrong kind of block), the operation does not start, every pin
/// taken so far is let go, and the `Err` names the operation, the slot and what holds its block.
pub(crate) fn run_cpu(reg: &Registry, dir: &Directory, op: OpId, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
    let sig = reg.signature(op);
    let v = reg.cpu_version(op).ok_or_else(|| format!("{}: no twin registered", sig.name))?;
    execute(sig, v, dir, &[(slots, params)], tiling, None)
}

/// Runs `op` on `device`: its default UKL version for the device's processors (a version is registered only once
/// checked against the twin), else its twin on the CPU (`run_cpu`). Its slots pinned and its descriptor table made as
/// `run_cpu`'s, then handed to the device (E1, E2) until its work completes. The runner's (`Runner::run_on`).
pub(crate) fn run_on(reg: &Registry, dir: &Directory, op: OpId, device: &dyn Kernels, slots: &[BlockId], params: &[u8], tiling: Tiling) -> Result<(), String> {
    let sig = reg.signature(op);
    match reg.device_version(op, device) {
        Some(v) => execute(sig, v, dir, &[(slots, params)], tiling, Some(device)),
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
    /// launch pins (each instance's slots first, in order), that table's block handed to `fence` (E1); the kernels'
    /// other arguments are each instance of `wide`'s slots (their block ids, in order) and params. Each slot's block is
    /// bound up to its instance's `rooms` capacity at least (its kernels claim past its used bytes within it). The
    /// launch's own pins are handed to `fence` after.
    fn bind(&self, launch: &Launch, fence: &Arc<Fence>, wide: &[Instance<'_>], rooms: &[Meter]) -> Result<Box<dyn OnDevice + '_>, String>;
}

/// A launch bound on a device (`Kernels::bind`).
pub trait OnDevice {
    /// Runs the phases `phases` of `source`'s kernels named `{entry}_{phase}` as one command, and waits for it: each
    /// phase's count kernel (`_items`), then its kernel over that many items (a thread an item, or a threadgroup an
    /// item for a `Threadgroup` phase), each turn of a phase that runs again followed by its `_again` kernel, up to its
    /// bound on turns; each phase over every instance's items at once (a wide launch). An instance's slots' capacities
    /// are its `rooms` meter's `capacity` (how far each block's allocations may reach) and their fronts `fronts`, which
    /// its claims move; an item that found one too small wrote nothing past it and recorded what it needs, into
    /// `needs`, and no phase of its instance after it did work (0686 D6: the executor grows the blocks and runs that
    /// instance again). The turns each phase ran, the most of any instance's.
    fn program(&mut self, source: &'static str, entry: &str, phases: &[DevicePhase], rooms: &mut [Meter]) -> Result<Vec<u32>, String>;
}

/// A phase as a device queues it (`OnDevice::program`).
#[derive(Clone, Copy, Debug)]
pub struct DevicePhase {
    pub name: &'static str,
    /// An item a threadgroup's (`Unit::Threadgroup`), else a lane's.
    pub group: bool,
    /// Its turns at most: 1, or a round phase's bound.
    pub turns: u32,
    /// Whether it runs again (`Across::Rounds`): its `_again` kernel after each turn.
    pub rounds: bool,
}

impl DevicePhase {
    /// A version's phase as its device runs it.
    pub fn of(p: &Phase) -> DevicePhase {
        let rounds = p.across == Across::Rounds;
        DevicePhase { name: p.name, group: p.unit == Unit::Threadgroup, turns: if rounds { p.turns() } else { 1 }, rounds }
    }
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
    /// A twin's, not a host version's: its phases are watched for the heap (in tests, `heap_watch`).
    watched: bool,
    name: &'static str,
}

/// `f`, its heap allocations failing the run when `on` (the crate's tests' `heap_watch`); elsewhere `f`.
#[cfg(test)]
use crate::heap_watch::during as heap_watched;
#[cfg(not(test))]
fn heap_watched<R>(_on: bool, _what: &str, f: impl FnOnce() -> R) -> R {
    f()
}

impl Runs for TwinRuns<'_, '_, '_> {
    fn items(&mut self, p: usize) -> Result<u32, String> {
        let (twin, ctx) = (self.twin, &self.ctx);
        Ok(heap_watched(self.watched, &format!("{} items {p}", self.name), || (twin[p].items)(ctx)))
    }
    fn run(&mut self, p: usize, t: Tile) -> Result<(), String> {
        let (twin, ctx) = (self.twin, &mut self.ctx);
        heap_watched(self.watched, &format!("{} run {p}", self.name), || (twin[p].run)(ctx, t));
        Ok(())
    }
    fn again(&mut self, p: usize) -> Result<bool, String> {
        let (twin, ctx) = (self.twin, &mut self.ctx);
        Ok(heap_watched(self.watched, &format!("{} again {p}", self.name), || twin[p].again.is_some_and(|again| again(ctx))))
    }
    fn meter(&mut self) -> Option<&mut Meter> {
        Some(&mut *self.ctx.meter)
    }
}

/// A UKL version's kernels on a bound launch.
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

fn execute(sig: &Signature, v: &Version, dir: &Directory, wide: &Wide<'_>, tiling: Tiling, device: Option<&dyn Kernels>) -> Result<(), String> {
    // A launch on the CPU, logged as the device's are (`RERAIL_KERNEL_TIMES`), so a run's launches are counted whole.
    if device.is_none() && std::env::var_os("RERAIL_KERNEL_TIMES").is_some() {
        eprintln!("delta-launch {} on the CPU ({} wide)", sig.name, wide.len());
    }
    for &(slots, params) in wide {
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
    }
    if wide.len() > MAX_WIDE {
        return Err(format!("{}: a launch of {} instances, past {MAX_WIDE}", sig.name, wide.len()));
    }
    // Items that are not independent run whole and in order, whatever tiling is asked: only an operation that says
    // the axis its items are independent along is cut into tiles and run in any order.
    let tiling = match sig.independent {
        Independence::Along(_) => tiling,
        Independence::Not(_) => Tiling::WHOLE,
    };
    // Its launches count in the open evaluation, or in one of its own (0601).
    let _evaluation = dir.evaluation(crate::dir::LAUNCH_BUDGET);
    // The instances still to run: each runs again, alone among those that ran, while it needs room.
    let mut pending: Vec<usize> = (0..wide.len()).collect(); // copy: not block data (the instances left)
    let mut reran = 0;
    loop {
        let run: Vec<(&[BlockId], &[u8])> = pending.iter().map(|&j| wide[j]).collect(); // copy: not block data (the instances left)
        let modes: Vec<(BlockId, bool)> = run.iter().flat_map(|(slots, _)| slots.iter().zip(sig.slots).map(|(&id, s)| (id, s.mode.writes()))).collect(); // copy: not block data (the slots' modes)
        let launch = dir.launch(sig.name, &modes)?;
        let drove: Vec<Drove> = match v.imp {
            Impl::Twin(twin) | Impl::Host(twin) => {
                let mut launch = launch;
                let ns = sig.slots.len();
                let (pins, table) = launch.parts();
                // The validation mode: every state the twin reads through the table counts its reads.
                let table = if dir.validating() { table.counting(dir.state_reads()) } else { table };
                let held: Vec<std::sync::Arc<rerail_blocks::block::Block>> = pins.iter().map(|p| std::sync::Arc::clone(p.block())).collect();
                let blocks: Vec<&rerail_blocks::block::Block> = held.iter().map(|b| &**b).collect();
                let mut out = Vec::with_capacity(run.len());
                // Each instance in turn, its own slots' pins and meter, over the launch's one table.
                for (k, (pins, &(slots, params))) in pins.chunks_mut(ns.max(1)).zip(&run).enumerate() {
                    let mut meter = Meter::new(slots.iter().map(|&id| dir.capacity(id)));
                    let mut slot_views: Vec<Slot<'_>> = pins.iter_mut().map(|p| if p.writes() { Slot::Write(p) } else { Slot::Read(p) }).collect();
                    let ctx = Ctx { slots: &mut slot_views, blocks: &blocks[k * ns..(k + 1) * ns], table: &table, params, meter: &mut meter };
                    out.push(drive(sig, v, tiling, dir, slots, &mut TwinRuns { ctx, twin, watched: matches!(v.imp, Impl::Twin(_)), name: sig.name })?);
                }
                drop(launch);
                out
            }
            // Bound to the device and handed to its fence, the launch is the device's: it ends with the work.
            Impl::Ukl { source, entry } => on_device(sig, v, dir, &run, device, launch, source, entry)?,
            Impl::Compiled(_) => return Err(format!("{}: {} is compiled ahead, which is not built", sig.name, v.name)),
        };
        let mut next = Vec::new();
        for (&j, d) in pending.iter().zip(drove) {
            if let Drove::Rerun(needs) = d {
                // Run again from the first phase, each slot that needed room grown to what it needs: a twin's once
                // (past that, the need was not the exact size, and the launch fails); a device's command up to 32
                // times (its items stop at the first slot found too small, so a later one's need shows on a later
                // run).
                let most = if matches!(v.imp, Impl::Ukl { .. }) { 32 } else { 1 };
                if reran >= most {
                    return Err(format!("{}: run again at the capacity its slots needed, and they needed more: {needs:?}", sig.name));
                }
                let slots = wide[j].0;
                for (s, need) in needs {
                    dir.grow(slots[s], need).map_err(|e| format!("{}: slot {} ({}): {e}", sig.name, s, sig.slots[s].name))?;
                }
                next.push(j);
            }
        }
        if next.is_empty() {
            return Ok(());
        }
        pending = next;
        reran += 1;
    }
}

/// A UKL version's launch on `device` over the instances `wide`: bound there, handed to its fence, its phases driven:
/// for each instance, done, or to run again at the capacities its slots need (`execute` grows them).
#[allow(clippy::too_many_arguments)]
fn on_device(sig: &Signature, v: &Version, dir: &Directory, wide: &Wide<'_>, device: Option<&dyn Kernels>, launch: Launch, source: &'static str, entry: &'static str) -> Result<Vec<Drove>, String> {
    let d = device.ok_or_else(|| format!("{}: {} is a UKL version, and no device was given to run it on", sig.name, v.name))?;
    if !v.devices.has(d.devices()) {
        return Err(format!("{}: {} runs on {:?}, not on the device's {:?}", sig.name, v.name, v.devices, d.devices()));
    }
    let ids: Vec<Vec<u32>> = wide.iter().map(|(slots, _)| slots.iter().map(|s| s.0).collect()).collect(); // copy: not block data (the slots' ids, an argument)
    let insts: Vec<Instance<'_>> = ids.iter().zip(wide).map(|(ids, &(_, params))| Instance { slots: ids, params }).collect(); // copy: not block data (the launch's arguments)
    let fence = Fence::new();
    // Every use the launch hands the fence ends with it, whatever the work did.
    let done = |r: Result<Vec<Drove>, String>| {
        fence.mark_committed();
        fence.complete();
        r
    };
    // Each instance's slots' capacities, and their fronts: where its kernels' claims start.
    let mut rooms: Vec<Meter> = wide
        .iter()
        .map(|(slots, _)| {
            let mut room = Meter::new(slots.iter().map(|&id| dir.capacity(id)));
            for (f, &id) in room.fronts.iter_mut().zip(*slots) {
                *f = dir.block(id).map_or(0, |b| b.used());
            }
            room
        })
        .collect(); // copy: not block data (the instances' meters)
    let bound = match d.bind(&launch, &fence, &insts, &rooms) {
        Ok(b) => b,
        Err(e) => return done(Err(e)),
    };
    if let Err(b) = launch.to_fence(&fence, sig.name, d.domain()) {
        drop(bound);
        return done(Err(b.1));
    }
    let mut bound = bound;
    let r = done(program(sig, v, dir, &mut *bound, source, entry, &mut rooms));
    // What its kernels claimed is the blocks' from now on.
    for (&(slots, _), room) in wide.iter().zip(&rooms) {
        for ((spec, &id), &front) in sig.slots.iter().zip(slots).zip(&room.fronts) {
            if let (true, Some(b)) = (spec.mode.writes(), dir.block(id)) {
                b.claimed_to(front);
            }
        }
    }
    // A device domain of its own: what the kernels wrote comes back to the host's bytes (`Region::sync_host`).
    if d.domain() != 0 {
        for &(slots, _) in wide {
            for (spec, &id) in sig.slots.iter().zip(slots) {
                if let (true, Some(b)) = (spec.mode.writes(), dir.block(id)) {
                    rerail_blocks::region::whole(&b).region().sync_host();
                }
            }
        }
    }
    r
}

/// A UKL version's phases run on its device as one command (`OnDevice::program`), each phase's turns charged: done,
/// or, where a slot was found too small, to run again at the capacities its slots need (`execute` grows them and binds
/// the launch again: a phase that bounds its capacity by growing or by running again; else refused).
fn program(sig: &Signature, v: &Version, dir: &Directory, bound: &mut dyn OnDevice, source: &'static str, entry: &'static str, rooms: &mut [Meter]) -> Result<Vec<Drove>, String> {
    let phases: Vec<DevicePhase> = v.shape.iter().map(DevicePhase::of).collect(); // copy: not block data (the phases' launch settings)
    let turns = bound.program(source, entry, &phases, rooms)?;
    for (p, &n) in v.shape.iter().zip(&turns) {
        for _ in 0..n.max(1) {
            dir.charge(sig.name, p.name)?;
        }
    }
    let regrow = v.shape.iter().any(|p| matches!(p.capacity(), Some(Over::Grow | Over::Rerun)));
    let mut drove = Vec::with_capacity(rooms.len());
    for room in rooms.iter_mut() {
        if let Some(f) = room.fault.take() {
            return Err(format!("{}: {f}", sig.name));
        }
        if room.no_needs() {
            drove.push(Drove::Done);
            continue;
        }
        let needs: Vec<(usize, usize)> = room.take_needs().iter().enumerate().filter(|n| *n.1 > 0).map(|(s, &n)| (s, n)).collect(); // copy: not block data (the host's list of the slots to grow)
        if !regrow {
            let (s, need) = needs[0];
            return Err(format!("{}: slot {s} ({}) needs a capacity of {need} bytes, and has {}: refused", sig.name, sig.slots[s].name, room.capacity[s]));
        }
        drove.push(Drove::Rerun(needs));
    }
    Ok(drove)
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
            let read0 = dir.state_reads().load(std::sync::atomic::Ordering::Relaxed);
            for k2 in order {
                let lo = k2 * t;
                r.run(k, Tile { lo: lo as u32, hi: (lo + t).min(n) as u32 })?;
                if let Some(f) = r.meter().and_then(|m| m.fault.take()) {
                    return Err(format!("{}: phase {}: {f}", sig.name, phase.name));
                }
            }
            if dir.validating() {
                let reads = dir.state_reads().load(std::sync::atomic::Ordering::Relaxed) - read0;
                dir.account(sig.name, phase.name, turn == 1 && grown == 0, n as u64, reads);
            }
            if let Some(m) = r.meter()
                && !m.no_needs()
            {
                let taken = m.take_needs();
                let needs: Vec<(usize, usize)> = taken.iter().enumerate().filter(|n| *n.1 > 0).map(|(s, &n)| (s, n)).collect(); // copy: not block data (the host's list of the slots to grow)
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
