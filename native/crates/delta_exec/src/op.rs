//! Operations over blocks (decision 0686, B1; *0686: Operations* §2–3, §9).
//!
//! An operation is a meaningful quantum of work over at most 30 whole blocks: its **signature** names each slot's
//! mode (read, write, read-write, scratch: UKL's `ROBUFFER`, `WOBUFFER`, `RWBUFFER`) and the kind of block it takes,
//! its params' layout (declared once, by `rerail_delta_twins::params!`, and given to UKL from there) and its work
//! shape as phases (Operations §4). What it writes is a function of its params and the bytes of its slots, and of
//! nothing else.
//!
//! Each operation has registered **versions** (`Version`, Operations §3, E3), chosen by their metadata. Its Rust
//! twin (`rerail_delta_twins`: a function of its pinned blocks and params alone, no state outliving a call) is its
//! reference: registered first, and the one every other version is checked against.
//!
//! **Starting** an operation pins every slot from its signature through the blocks' own regions (0491's checks,
//! `rerail_blocks::region::pin_block`), naming the operation: a second writer, a writer beside a reader, or a write of
//! a frozen block refuses the start, and nothing stays pinned (E1). The CPU executor (`run_cpu`) then runs the twin
//! on the pinned bytes, in place. The registry and the executor are host Rust, which may keep track of the operation
//! graph; the computation is the twin's alone.

use rerail_blocks::Block;
use rerail_blocks::region::{BlockPin, pin_block};
use rerail_delta_twins::{Field, Params, Slot, TwinFn};
use std::sync::Arc;

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

/// A work shape of Operations §4, by its number there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shape(pub u8);

impl Shape {
    pub const MAP: Shape = Shape(1);
    pub const UNEVEN_SEGMENTS: Shape = Shape(3);
    pub const REDUCE: Shape = Shape(7);
    pub const HISTOGRAM: Shape = Shape(8);
    pub const SCAN: Shape = Shape(10);
    pub const EMIT: Shape = Shape(11);
    pub const SORT: Shape = Shape(14);
    pub const RUNS: Shape = Shape(15);
    pub const MERGE: Shape = Shape(16);
    pub const HASH_BUILD: Shape = Shape(18);
    pub const CODEC: Shape = Shape(22);
    pub const WORK_QUEUE: Shape = Shape(32);
}

/// One phase of an operation's work, with its shape.
#[derive(Clone, Copy, Debug)]
pub struct Phase {
    pub name: &'static str,
    pub shape: Shape,
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

/// What an operation is: its slots, params and shape. Declared once, as a constant; `Registry::declare` takes it.
pub struct Signature {
    pub name: &'static str,
    pub slots: &'static [SlotSpec],
    pub params: ParamsLayout,
    pub shape: &'static [Phase],
}

impl Signature {
    /// A signature; more than `MAX_SLOTS` slots fails where it is declared (at compile time for a constant).
    pub const fn new(
        name: &'static str,
        slots: &'static [SlotSpec],
        params: ParamsLayout,
        shape: &'static [Phase],
    ) -> Signature {
        assert!(slots.len() <= MAX_SLOTS, "an operation binds at most 30 slots");
        Signature { name, slots, params, shape }
    }
}

/// An operation, by its place in the registry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpId(u32);

/// How a version runs.
#[derive(Clone, Copy)]
pub enum Impl {
    /// The Rust twin (`rerail_delta_twins`): the reference every other version is checked against.
    Twin(TwinFn),
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
    pub shape: &'static [Shape],
    pub limits: Limits,
    pub cost: CostKey,
    pub status: Status,
}

impl Version {
    /// An operation's twin as its reference version.
    pub fn twin(op: OpId, name: &'static str, f: TwinFn, shape: &'static [Shape]) -> Version {
        Version {
            op,
            name,
            imp: Impl::Twin(f),
            devices: Devices::CPU,
            needs: Features::default(),
            shape,
            limits: Limits::NONE,
            cost: CostKey(name),
            status: Status::Reference,
        }
    }
}

/// The operations and their versions.
#[derive(Default)]
pub struct Registry {
    ops: Vec<&'static Signature>,
    versions: Vec<Version>,
}

impl Registry {
    /// An operation, by its signature. Refused past `MAX_SLOTS` slots, or under a name already declared.
    pub fn declare(&mut self, sig: &'static Signature) -> Result<OpId, String> {
        if sig.slots.len() > MAX_SLOTS {
            return Err(format!("{}: {} slots; an operation binds at most {MAX_SLOTS}", sig.name, sig.slots.len()));
        }
        if self.ops.iter().any(|s| s.name == sig.name) {
            return Err(format!("{}: declared twice", sig.name));
        }
        self.ops.push(sig);
        Ok(OpId(self.ops.len() as u32 - 1))
    }

    pub fn signature(&self, op: OpId) -> &'static Signature {
        self.ops[op.0 as usize]
    }

    /// A version. An operation's first is its reference, and only a twin is a reference; a version's name is its
    /// operation's once.
    pub fn register(&mut self, v: Version) -> Result<(), String> {
        let sig = self.ops.get(v.op.0 as usize).ok_or("a version of an operation not declared")?;
        let mine = self.versions(v.op);
        if mine.iter().any(|w| w.name == v.name) {
            return Err(format!("{}: version {} registered twice", sig.name, v.name));
        }
        let twin = matches!(v.imp, Impl::Twin(_));
        match (mine.iter().any(|w| w.status == Status::Reference), v.status == Status::Reference) {
            (false, false) => Err(format!("{}: {} before the operation's reference (its twin)", sig.name, v.name)),
            (true, true) => Err(format!("{}: a second reference ({})", sig.name, v.name)),
            (_, true) if !twin => Err(format!("{}: {} is a reference but not the twin", sig.name, v.name)),
            _ => {
                self.versions.push(v);
                Ok(())
            }
        }
    }

    pub fn versions(&self, op: OpId) -> Vec<&Version> {
        self.versions.iter().filter(|v| v.op == op).collect()
    }

    /// The twin the CPU executor runs: the default when a twin is, else the reference.
    fn cpu_twin(&self, op: OpId) -> Option<TwinFn> {
        let twin = |status| {
            self.versions.iter().find_map(|v| match v.imp {
                Impl::Twin(f) if v.op == op && v.status == status => Some(f),
                _ => None,
            })
        };
        twin(Status::Default).or_else(|| twin(Status::Reference))
    }
}

/// Runs `op` on the CPU: its twin on `blocks`, one a slot in its signature's order, with `params`' bytes.
///
/// Every slot is pinned first, by its mode, naming the operation. If any cannot be (a second writer, a writer beside
/// a reader, a write of a frozen block, the wrong kind of block), the operation does not start, every pin taken so far
/// is let go, and the `Err` names the operation, the slot and what holds its block.
pub fn run_cpu(reg: &Registry, op: OpId, blocks: &[&Arc<Block>], params: &[u8]) -> Result<(), String> {
    let sig = reg.signature(op);
    let twin = reg.cpu_twin(op).ok_or_else(|| format!("{}: no twin registered", sig.name))?;
    if blocks.len() != sig.slots.len() {
        return Err(format!("{}: {} blocks for {} slots", sig.name, blocks.len(), sig.slots.len()));
    }
    if params.len() != sig.params.size {
        return Err(format!("{}: {} bytes of params for {} ({})", sig.name, params.len(), sig.params.name, sig.params.size));
    }
    let mut pins: Vec<BlockPin> = Vec::with_capacity(blocks.len());
    for (i, (b, spec)) in blocks.iter().zip(sig.slots).enumerate() {
        let kind = b.header().kind;
        if kind != spec.kind {
            return Err(format!("{} could not start: slot {i} ({}) takes a block of kind {}, given one of kind {kind}", sig.name, spec.name, spec.kind));
        }
        let pin = pin_block(b, sig.name, spec.mode.writes())
            .map_err(|e| format!("{} could not start: slot {i} ({}): {e}", sig.name, spec.name))?;
        pins.push(pin);
    }
    let mut slots: Vec<Slot<'_>> = pins.iter_mut().map(|p| if p.writes() { Slot::Write(p) } else { Slot::Read(p) }).collect();
    twin(&mut slots, params);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_blocks::block::kind;
    use rerail_blocks::region::{pin_block, whole};
    use rerail_delta_twins::{Params, Scalar};

    rerail_delta_twins::params! {
        pub struct SumP {
            /// Where the values start in slot 0's block, and where the sum goes in slot 1's.
            pub at: u64,
            pub n: u32,
            pub pad: u32,
        }
    }

    /// A test twin: the sum of `n` u64s at `at` in slot 0, written at `at` in slot 1.
    fn sum(slots: &mut [Slot<'_>], params: &[u8]) {
        let p = SumP::read(params);
        let at = p.at as usize;
        let src = slots[0].bytes();
        let total: u64 = (0..p.n as usize).map(|i| u64::read(&src[at + 8 * i..])).sum();
        total.write(&mut slots[1].bytes_mut().unwrap()[at..]);
    }

    static SUM: Signature = Signature::new(
        "sum",
        &[
            SlotSpec { name: "values", mode: Mode::Read, kind: kind::FACTS },
            SlotSpec { name: "total", mode: Mode::Write, kind: kind::FACTS },
        ],
        ParamsLayout::of::<SumP>(),
        &[Phase { name: "reduce", shape: Shape::REDUCE }],
    );

    fn reg() -> (Registry, OpId) {
        let mut r = Registry::default();
        let op = r.declare(&SUM).unwrap();
        r.register(Version::twin(op, "sum twin", sum, &[Shape::REDUCE])).unwrap();
        (r, op)
    }

    fn facts() -> Arc<Block> {
        Arc::new(Block::anon(kind::FACTS, 1 << 20))
    }

    fn params() -> Vec<u8> {
        let mut b = vec![0u8; SumP::SIZE];
        SumP { at: 4096, n: 4, pad: 0 }.write(&mut b);
        b
    }

    fn filled() -> Arc<Block> {
        let b = facts();
        let mut w = pin_block(&b, "fill", true).unwrap();
        for i in 0..4u64 {
            (i + 1).write(&mut w.bytes_mut().unwrap()[4096 + 8 * i as usize..]);
        }
        b
    }

    fn total(b: &Arc<Block>) -> u64 {
        u64::read(&pin_block(b, "look", false).unwrap().bytes()[4096..])
    }

    /// A twin reads only its slots and params: the same blocks give the same facts, run after run; a run's write
    /// moves its output block's version.
    #[test]
    fn an_operation_runs_its_twin_on_its_blocks_in_place() {
        let (r, op) = reg();
        let (v, out) = (filled(), facts());
        run_cpu(&r, op, &[&v, &out], &params()).unwrap();
        assert_eq!(total(&out), 10);
        // The block's own region lives while a handle holds it (0491): held here across the run.
        let held = whole(&out);
        let ver = held.version();
        run_cpu(&r, op, &[&v, &out], &params()).unwrap();
        assert_eq!(total(&out), 10, "the same blocks and params, the same facts");
        assert_eq!(held.version(), ver + 1);
    }

    /// E1: a second writer, a writer beside a reader, and a write of a frozen block each refuse the start, naming
    /// the operation and what holds the block, and leave nothing pinned.
    #[test]
    fn a_conflict_refuses_the_start_and_leaves_nothing_pinned() {
        let (r, op) = reg();
        let (v, out) = (filled(), facts());

        let other = pin_block(&out, "consolidate", true).unwrap();
        let e = run_cpu(&r, op, &[&v, &out], &params()).unwrap_err();
        assert!(e.starts_with("sum could not start: slot 1 (total)") && e.contains("a CPU write by consolidate"), "{e}");
        // Slot 0's read pin was taken, then let go: a writer of it starts.
        drop(pin_block(&v, "intern", true).expect("slot 0 was left unpinned"));
        drop(other);

        let reader = pin_block(&out, "step", false).unwrap();
        let e = run_cpu(&r, op, &[&v, &out], &params()).unwrap_err();
        assert!(e.contains("a CPU write by sum while a CPU read by step"), "{e}");
        drop(reader);

        let e = run_cpu(&r, op, &[&out, &out], &params()).unwrap_err();
        assert!(e.contains("slot 1 (total)") && e.contains("a CPU read by sum"), "one operation does not read and write one block: {e}");

        let frozen = Block::anon(kind::FACTS, 1 << 20).freeze();
        let e = run_cpu(&r, op, &[&v, frozen.arc()], &params()).unwrap_err();
        assert!(e.contains("a CPU write by sum of a frozen block"), "{e}");

        run_cpu(&r, op, &[&v, &out], &params()).expect("nothing is left pinned by the refused starts");
        assert_eq!(total(&out), 10);
    }

    /// A slot takes the kind of block its signature names; the params are the declared size.
    #[test]
    fn a_slot_takes_its_kind_of_block_and_the_params_their_size() {
        let (r, op) = reg();
        let atoms = Arc::new(Block::anon(kind::ATOMS, 1 << 20));
        let e = run_cpu(&r, op, &[&filled(), &atoms], &params()).unwrap_err();
        assert!(e.contains("slot 1 (total) takes a block of kind"), "{e}");
        let e = run_cpu(&r, op, &[&filled(), &facts()], &[0u8; 3]).unwrap_err();
        assert!(e.contains("3 bytes of params for SumP (16)"), "{e}");
    }

    /// D1: a 31st slot is refused where the operation is declared.
    #[test]
    fn a_thirty_first_slot_is_refused() {
        let slots: &'static [SlotSpec] =
            Box::leak(vec![SlotSpec { name: "s", mode: Mode::Read, kind: kind::FACTS }; MAX_SLOTS + 1].into_boxed_slice());
        let sig: &'static Signature = Box::leak(Box::new(Signature {
            name: "wide",
            slots,
            params: ParamsLayout::of::<rerail_delta_twins::NoParams>(),
            shape: &[],
        }));
        let e = Registry::default().declare(sig).unwrap_err();
        assert!(e.contains("31 slots; an operation binds at most 30"), "{e}");
    }

    /// E3: the twin is registered first, as the one reference; any other version comes after it.
    #[test]
    fn the_twin_is_the_reference_and_comes_first() {
        let mut r = Registry::default();
        let op = r.declare(&SUM).unwrap();
        let ukl = Version { imp: Impl::Ukl { source: "", entry: "sum" }, status: Status::Candidate, devices: Devices::METAL, ..Version::twin(op, "sum ukl", sum, &[Shape::REDUCE]) };
        assert!(r.register(ukl).unwrap_err().contains("before the operation's reference"));
        assert!(r.register(Version { status: Status::Reference, ..ukl }).unwrap_err().contains("not the twin"));
        r.register(Version::twin(op, "sum twin", sum, &[Shape::REDUCE])).unwrap();
        assert!(r.register(Version::twin(op, "sum twin 2", sum, &[Shape::REDUCE])).unwrap_err().contains("a second reference"));
        r.register(ukl).unwrap();
        assert_eq!(r.versions(op).len(), 2);
        assert!(r.declare(&SUM).unwrap_err().contains("declared twice"));
    }

    /// Params are declared once in Rust; UKL's declaration comes from that one.
    #[test]
    fn params_give_ukl_their_layout() {
        let u = ParamsLayout::of::<SumP>().ukl();
        assert!(u.contains("typedef struct SumP {\n    uint64_t at;\n    uint32_t n;\n    uint32_t pad;\n} SumP;"), "{u}");
        assert!(u.contains("UKL_OFFSETOF(SumP, n) == 8"), "{u}");
        assert_eq!(Mode::Read.ukl(), "ROBUFFER");
        assert_eq!(Mode::Scratch.ukl(), "RWBUFFER");
    }
}
