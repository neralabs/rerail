//! Block programs on Metal (`block`), both ways the transducer runs on the device:
//!
//! - **the interpreter** (`metal_block_program.metal`): the set program's shape, one dispatch
//!   running every operation in order;
//! - **generated MSL** from the native IR (`BlockNative`): the operations written out as one
//!   straight-line kernel, with no loop, no switch and no branch.
//!
//! Each lane owns `W` consecutive source bytes and builds its own `W`-bit word of every value
//! (`LaneWidth`: 16, 32 or 64), so a SIMD group's 32 lanes work on 32 words at once, not one bit
//! each. A byte compare is SWAR over the lane's `uint4`s; the logic is word logic.
//!
//! Carried operations (`prev`, `toggle`, `after_odd_run`) follow `06-block-parallel.md` §2 and
//! §5: each is a round, one dispatch. A lane's summary is combined across its SIMD group (one
//! threadgroup of 32 lanes) by SIMD-group operations, and across groups through a state per
//! group. Masks are recomputed in every round, not stored; only the states persist.
//!
//! - **Generated MSL** passes states by decoupled look-back with bounded waits: a group waits a
//!   fixed number of polls for a predecessor's state, then computes that predecessor's aggregate
//!   from its bytes itself. The mask code stays straight-line; only the look-back loops.
//! - **The interpreter** keeps its values in slots, so a group cannot recompute another's; it
//!   scans the groups' aggregates in a dispatch of its own instead (`block_carry_scan`).
use super::*;
use crate::code_rule_dsl::transducer::packed::block::{BlockProgram, mask_tail};
use crate::code_rule_dsl::transducer::packed::native_ir::{BlockInst, BlockNative};
use metal::MTLSize;

/// The carries' helpers (`metal_block_carry.metal`), shared by the interpreter and generated MSL.
const CARRY: &str = include_str!("metal_block_carry.metal");
const SOURCE: &str = concat!(include_str!("metal_block_carry.metal"), include_str!("metal_block_program.metal"));
/// Lanes per threadgroup: one SIMD group.
const GROUP: usize = 32;

/// The bytes each lane owns, and so the width of its mask words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum LaneWidth {
    W16,
    W32,
    W64,
}

impl LaneWidth {
    pub(in crate::code_rule_dsl) const ALL: [LaneWidth; 3] = [LaneWidth::W16, LaneWidth::W32, LaneWidth::W64];

    pub(in crate::code_rule_dsl) fn bytes(self) -> usize {
        match self {
            LaneWidth::W16 => 16,
            LaneWidth::W32 => 32,
            LaneWidth::W64 => 64,
        }
    }

    /// The MSL type of a lane word.
    fn word(self) -> &'static str {
        match self {
            LaneWidth::W16 => "ushort",
            LaneWidth::W32 => "uint",
            LaneWidth::W64 => "ulong",
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlockArgs {
    from: u32,
    to: u32,
    lanes: u32,
    pad: u32,
}

/// A carried instruction's kind, as the carry helpers number it.
fn kind(inst: &BlockInst) -> u32 {
    match inst {
        BlockInst::Toggle(_) => 0,
        BlockInst::AfterOddRun(_) => 1,
        _ => unreachable!("scanned carries only"),
    }
}

/// An instruction of the planes form as the interpreter's step: `[op, a, b, imm]`; a carried
/// instruction's `b` is its round.
fn encoded(inst: &BlockInst, round: u32) -> [u32; 4] {
    match *inst {
        BlockInst::EqByte(byte) => [0, 0, 0, u32::from(byte)],
        BlockInst::Const(c) => [1, 0, c as u32, (c >> 32) as u32],
        BlockInst::Not(a) => [2, u32::from(a), 0, 0],
        BlockInst::And(a, b) => [3, u32::from(a), u32::from(b), 0],
        BlockInst::Or(a, b) => [4, u32::from(a), u32::from(b), 0],
        BlockInst::Xor(a, b) => [5, u32::from(a), u32::from(b), 0],
        BlockInst::AndNot(a, b) => [6, u32::from(a), u32::from(b), 0],
        BlockInst::Plane(k) => [7, 0, 0, u32::from(k)],
        BlockInst::Prev(a, k) => [8, u32::from(a), round, u32::from(k)],
        BlockInst::Toggle(a) => [9, u32::from(a), round, 0],
        BlockInst::AfterOddRun(a) => [10, u32::from(a), round, 0],
        BlockInst::Class { .. } => unreachable!("the planes form has no class"),
    }
}

/// The interpreter: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn interpret(
    program: &BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    // The interpreter runs the planes form: `class` as formulas of bit-planes, as generated MSL.
    let native = BlockNative::lower(program).planes_form();
    let carried: Vec<usize> = (0..native.insts.len()).filter(|&at| native.insts[at].carried()).collect();
    let mut round = 0u32;
    let steps: Vec<[u32; 4]> = native
        .insts
        .iter()
        .map(|inst| {
            let step = encoded(inst, round);
            round += u32::from(inst.carried());
            step
        })
        .collect();
    let w = width.bytes();
    let run = pipeline_named(SOURCE, &format!("block_program_{w}"))?;
    let summary = pipeline_named(SOURCE, &format!("block_summary_{w}"))?;
    let scan = pipeline_named(SOURCE, "block_carry_scan")?;
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let stream = Stream::new(bytes, width, steps.len())?;
    let steps_buffer = MappedBuffer::from_slice(device, &steps)?;
    let states = MappedBuffer::zeroed::<u32>(device, carried.len().max(1) * stream.groups)?;
    let aggregates = MappedBuffer::zeroed::<u32>(device, stream.groups)?;
    let halos = MappedBuffer::zeroed::<u64>(device, carried.len().max(1) * stream.groups)?;
    let args = |from: usize, to: usize| BlockArgs { from: from as u32, to: to as u32, lanes: stream.lanes as u32, pad: 0 };
    let state_bytes = (stream.groups * 4) as u64;
    let mut passes: Vec<Pass<'_>> = Vec::new();
    let lanes_pass = |pipeline, a: BlockArgs| Pass {
        pipeline,
        buffers: vec![(0, &*stream.source.buffer, 0), (1, &*stream.values.buffer, 0), (3, &*steps_buffer.buffer, 0), (4, &*states.buffer, 0), (5, &*aggregates.buffer, 0), (6, &*halos.buffer, 0)],
        args: vec![(2, bytes_of(&a))],
        groups: stream.groups,
        threads: GROUP,
    };
    let mut from = 0;
    for (j, &at) in carried.iter().enumerate() {
        if from < at {
            passes.push(lanes_pass(&run, args(from, at)));
        }
        passes.push(lanes_pass(&summary, args(at, at)));
        // A halo (`prev`) is the group before's last 64 bits, written by the summary pass: no scan.
        if matches!(native.insts[at], BlockInst::Prev(..)) {
            from = at;
            continue;
        }
        let p = [stream.groups as u32, kind(&native.insts[at])];
        passes.push(Pass {
            pipeline: &scan,
            buffers: vec![(0, &*aggregates.buffer, 0), (1, &*states.buffer, j as u64 * state_bytes)],
            args: vec![(2, bytes_of(&p))],
            groups: 1,
            threads: 256,
        });
        from = at;
    }
    passes.push(lanes_pass(&run, args(from, native.insts.len())));
    encode_passes(&passes)?;
    let words = stream.slots(steps.len());
    Ok(native.outputs.iter().map(|&v| finish(&words[usize::from(v)], bytes.len())).collect())
}

/// SWAR byte compare, emitted once into every generated kernel (the interpreter's `eq16`).
const EQ16: &str = "static inline uint eq16(uint4 v, uint c4) {
  const uint4 x = v ^ c4;
  const uint4 z = ~(((x & 0x7f7f7f7fu) + 0x7f7f7f7fu) | x) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}
";

/// How generated MSL computes `class`. `Planes` is the default: on simdjson's five classes over
/// 64 MiB it ran at the speed of five plain byte compares (memory-bound), and `Lookup` about 30%
/// slower at every lane width (`block_tests::class_strategies_gpu_time`).
///
/// - `Lookup`: simdjson's form, byte by byte: two lookups per byte in `constant` tables give each
///   byte's class bits, then a SWAR nonzero test of the selected bits gives the mask.
/// - `Planes`: the lane's bytes transposed into eight bit-planes (plane `k`: bit `k` of every
///   byte), then each class is a boolean formula of the planes: an OR over class bits of a
///   function of the low four planes and one of the high four, each written out by Shannon
///   expansion. Only word operations; no memory but the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum ClassStrategy {
    Lookup,
    Planes,
}

impl ClassStrategy {
    pub(in crate::code_rule_dsl) const ALL: [ClassStrategy; 2] = [ClassStrategy::Lookup, ClassStrategy::Planes];
}

/// SWAR nonzero bytes: 16 bytes that are not zero as 16 bits.
const NZ16: &str = "static inline uint nz16(uint4 x) {
  const uint4 z = (((x & 0x7f7f7f7fu) + 0x7f7f7f7fu) | x) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}
";

/// Hacker's Delight's 8x8 bit transpose: byte `i` bit `j` to byte `j` bit `i`.
const TRANSPOSE8: &str = "static inline ulong transpose8(ulong x) {
  ulong t = (x ^ (x >> 7u)) & 0x00AA00AA00AA00AAul;
  x = x ^ t ^ (t << 7u);
  t = (x ^ (x >> 14u)) & 0x0000CCCC0000CCCCul;
  x = x ^ t ^ (t << 14u);
  t = (x ^ (x >> 28u)) & 0x00000000F0F0F0F0ul;
  return x ^ t ^ (t << 28u);
}
";

/// The native IR as one straight-line MSL kernel, `tempo_block`, for lanes of `width`.
pub(in crate::code_rule_dsl) fn generate(native: &BlockNative, width: LaneWidth) -> String {
    generate_with(native, width, ClassStrategy::Planes)
}

/// The lane's eight bit-planes `p0..p7` from its chunks `c0..`: `transpose8` on each 8 bytes,
/// then byte `k` of every group gathered into plane `k` (for 64-byte lanes, an 8x8 byte
/// transpose).
fn planes(width: LaneWidth) -> String {
    let m = width.word();
    let groups = width.bytes() / 8;
    let mut out = String::new();
    for g in 0..groups {
        let (c, half) = (g / 2, if g % 2 == 0 { ("x", "y") } else { ("z", "w") });
        out.push_str(&format!(
            "  ulong t{g} = transpose8(ulong(c{c}.{}) | (ulong(c{c}.{}) << 32u));\n",
            half.0, half.1
        ));
    }
    if groups == 8 {
        for (sh, mask, step) in [(32, 0x0000_0000_FFFF_FFFFu64, 4), (16, 0x0000_FFFF_0000_FFFF, 2), (8, 0x00FF_00FF_00FF_00FF, 1)] {
            for g in (0..8).filter(|g| g & step == 0) {
                let h = g + step;
                out.push_str(&format!(
                    "  {{ const ulong a = t{g}, b = t{h}; t{g} = (a & {mask:#018x}ul) | ((b & {mask:#018x}ul) << {sh}u); \
                     t{h} = ((a >> {sh}u) & {mask:#018x}ul) | (b & ~{mask:#018x}ul); }}\n"
                ));
            }
        }
        for k in 0..8 {
            out.push_str(&format!("  const ulong p{k} = t{k};\n"));
        }
    } else {
        for k in 0..8 {
            let parts: Vec<String> = (0..groups)
                .map(|g| {
                    let byte = format!("((t{g} >> {}u) & 0xfful)", k * 8);
                    if g == 0 { byte } else { format!("({byte} << {}u)", g * 8) }
                })
                .collect();
            out.push_str(&format!("  const {m} p{k} = {m}({});\n", parts.join(" | ")));
        }
    }
    out
}

/// One table's class bytes of a `uint4` by lookups: byte `j`'s `lo[b & 15] & hi[b >> 4]`.
fn lookup_helper(t: usize, table: &super::block::ClassTable) -> String {
    let hex = |xs: &[u8]| xs.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
    let mut out = format!(
        "constant uchar T{t}[32] = {{{}, {}}};\nstatic inline uint4 classes_t{t}(uint4 v) {{\n  uint4 k;\n",
        hex(&table.lo),
        hex(&table.hi)
    );
    for lane in ["x", "y", "z", "w"] {
        let bytes: Vec<String> = (0..4)
            .map(|j| {
                let b = format!("uint(T{t}[(v.{lane} >> {}u) & 15u] & T{t}[16u + ((v.{lane} >> {}u) & 15u)])", 8 * j, 8 * j + 4);
                if j == 0 { b } else { format!("({b} << {}u)", 8 * j) }
            })
            .collect();
        out.push_str(&format!("  k.{lane} = {};\n", bytes.join(" | ")));
    }
    out.push_str("  return k;\n}\n");
    out
}

/// The native IR as MSL for lanes of `width`, `class` by `strategy`: a straight-line summary
/// function and a round kernel (`tempo_round_J`) per carried instruction, and the straight-line
/// kernel `tempo_block` that writes the outputs.
pub(in crate::code_rule_dsl) fn generate_with(native: &BlockNative, width: LaneWidth, strategy: ClassStrategy) -> String {
    generate_polling(native, width, strategy, SPINS)
}

/// `generate_with`, polling a predecessor's state `spins` times before computing it (0: always
/// compute it, which tests the fallback).
pub(in crate::code_rule_dsl) fn generate_polling(native: &BlockNative, width: LaneWidth, strategy: ClassStrategy, spins: u32) -> String {
    let planes_form;
    let native = match strategy {
        ClassStrategy::Lookup => native,
        ClassStrategy::Planes => {
            planes_form = native.planes_form();
            &planes_form
        }
    };
    let mut helpers = String::new();
    if native.insts.iter().any(|i| matches!(i, BlockInst::Class { .. })) {
        helpers.push_str(NZ16);
        for (t, table) in native.tables.iter().enumerate() {
            helpers.push_str(&lookup_helper(t, table));
        }
    }
    if native.insts.iter().any(|i| matches!(i, BlockInst::Plane(_))) {
        helpers.push_str(TRANSPOSE8);
    }
    let mut out = format!("{CARRY}\n{EQ16}\n{helpers}\n");
    let plan = Plan::new(native);
    if !plan.windowed.is_empty() {
        out.push_str(&halo_window(native, &plan));
    }
    for (j, &at) in plan.rounds.iter().enumerate() {
        let inst = native.insts[at];
        let ret = if matches!(inst, BlockInst::Prev(..)) { "ulong" } else { "uint" };
        out.push_str(&format!(
            "// Round {j}: the lane's summary of %{at}.\n\
             static inline {ret} summary_{j}(device const uint4* bytes, device atomic_uint* states, device const ulong* halos, uint groups, uint g, uint l) {{\n\
             \x20 const uint gl = g * {GROUP}u + l;\n"
        ));
        out.push_str(&lane_values(native, &plan, width, at));
        out.push_str(&format!("  return {};\n}}\n\n", summary_of(&inst)));
        out.push_str(&round_kernel(j, &inst, spins));
    }
    out.push_str(
        "kernel void tempo_block(\n\
         \x20   device const uint4* bytes [[buffer(0)]],\n",
    );
    out.push_str(&format!(
        "    device {}* out [[buffer(1)]],\n\
         \x20   constant uint& lanes [[buffer(2)]],\n\
         \x20   device atomic_uint* states [[buffer(3)]],\n\
         \x20   device const ulong* halos [[buffer(4)]],\n\
         \x20   uint gl [[thread_position_in_grid]],\n\
         \x20   uint g [[threadgroup_position_in_grid]],\n\
         \x20   uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint groups = lanes / {GROUP}u;\n",
        width.word()
    ));
    out.push_str(&lane_values(native, &plan, width, native.insts.len()));
    for (k, v) in native.outputs.iter().enumerate() {
        out.push_str(&format!("  out[{k}u * lanes + gl] = v{v};\n"));
    }
    out.push_str("}\n");
    out
}

/// A carried instruction's summary of its operand, the carry helpers' function for its kind.
fn summary_of(inst: &BlockInst) -> String {
    match *inst {
        BlockInst::Prev(a, _) => format!("halo_last(v{a})"),
        BlockInst::Toggle(a) => format!("summary_toggle(v{a})"),
        BlockInst::AfterOddRun(a) => format!("summary_after_odd_run(v{a})"),
        _ => unreachable!("not carried"),
    }
}

fn kind_name(inst: &BlockInst) -> &'static str {
    match inst {
        BlockInst::Toggle(_) => "parity",
        BlockInst::AfterOddRun(_) => "borrow",
        BlockInst::Prev(..) => "halo",
        _ => unreachable!("not carried"),
    }
}

/// Round `j`'s kernel: the group's aggregate published, the look-back for its exclusive prefix,
/// then its inclusive prefix published. A halo (`prev`) needs no look-back: the round writes the
/// group's last 64 bits, which only later rounds read.
fn round_kernel(j: usize, inst: &BlockInst, spins: u32) -> String {
    let kind = kind_name(inst);
    let mut out = format!(
        "kernel void tempo_round_{j}(device const uint4* bytes [[buffer(0)]], constant uint& lanes [[buffer(2)]],\n\
         \x20                          device atomic_uint* states [[buffer(3)]], device ulong* halos [[buffer(4)]],\n\
         \x20                          uint g [[threadgroup_position_in_grid]], uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint groups = lanes / {GROUP}u;\n"
    );
    if kind == "halo" {
        out.push_str(&format!(
            "  const ulong h = summary_{j}(bytes, states, halos, groups, g, l);\n\
             \x20 if (l == 0u) halos[{j}u * groups + g] = h;\n}}\n\n"
        ));
        return out;
    }
    out.push_str(&format!(
        "  device atomic_uint* state = states + {j}u * groups;\n\
         \x20 const uint agg = reduce_{kind}(summary_{j}(bytes, states, halos, groups, g, l));\n"
    ));
    out.push_str(&format!(
        "  if (l == 0u) atomic_store_explicit(&state[g], CARRY_AGGREGATE | agg, memory_order_relaxed);\n\
         \x20 // Look-back, bounded: at most {spins} polls per predecessor, then its aggregate recomputed here.\n\
         \x20 uint prefix = carry_identity({k}u);\n\
         \x20 for (uint back = 1u; back <= g; back++) {{\n\
         \x20   const uint p = g - back;\n\
         \x20   uint st = 0u;\n\
         \x20   for (uint spin = 0u; spin < {spins}u && (st >> 30u) == 0u; spin++) {{\n\
         \x20     st = simd_broadcast(atomic_load_explicit(&state[p], memory_order_relaxed), 0u);\n\
         \x20   }}\n\
         \x20   if ((st >> 30u) == 0u) {{\n\
         \x20     st = CARRY_AGGREGATE | reduce_{kind}(summary_{j}(bytes, states, halos, groups, p, l));\n\
         \x20   }}\n\
         \x20   prefix = combine_{kind}(st & CARRY_VALUE, prefix);\n\
         \x20   if ((st >> 30u) == 2u) break;\n\
         \x20 }}\n\
         \x20 if (l == 0u) atomic_store_explicit(&state[g], CARRY_INCLUSIVE | combine_{kind}(prefix, agg), memory_order_relaxed);\n}}\n\n",
        k = kind_index(inst)
    ));
    out
}

fn kind_index(inst: &BlockInst) -> u32 {
    kind(inst)
}

/// Polls of a predecessor's state before computing it here.
const SPINS: u32 = 64;

/// How generated MSL treats each instruction: whether its value depends on no scanned carry
/// (`local`: a function of the bytes alone), and the round of each instruction that needs one.
struct Plan {
    local: Vec<bool>,
    /// Instructions that are rounds, in order: carried ones, except a `prev` of a local value
    /// whose operand has no `prev` in it (its halo is recomputed from the 64 bytes before).
    rounds: Vec<usize>,
    /// The local `prev`s, by operand: their halo comes from `halo_window`.
    windowed: Vec<usize>,
}

impl Plan {
    fn new(native: &BlockNative) -> Self {
        let n = native.insts.len();
        let operands = |inst: &BlockInst| -> Vec<u16> {
            match *inst {
                BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Toggle(a) | BlockInst::AfterOddRun(a) => vec![a],
                BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => vec![a, b],
                _ => vec![],
            }
        };
        let mut local = vec![true; n];
        let mut has_prev = vec![false; n];
        let mut rounds = Vec::new();
        let mut windowed = Vec::new();
        for at in 0..n {
            let ops = operands(&native.insts[at]);
            let ops_local = ops.iter().all(|&a| local[usize::from(a)]);
            let ops_prev = ops.iter().any(|&a| has_prev[usize::from(a)]);
            match native.insts[at] {
                BlockInst::Prev(a, _) if ops_local && !has_prev[usize::from(a)] => {
                    windowed.push(at);
                    local[at] = true;
                    has_prev[at] = true;
                }
                ref inst if inst.carried() => {
                    rounds.push(at);
                    local[at] = false;
                    has_prev[at] = ops_prev || matches!(inst, BlockInst::Prev(..));
                }
                _ => {
                    local[at] = ops_local;
                    has_prev[at] = ops_prev;
                }
            }
        }
        Plan { local, rounds, windowed }
    }

    fn round_of(&self, at: usize) -> Option<usize> {
        self.rounds.iter().position(|&r| r == at)
    }
}

/// The instructions in `chosen`, as straight-line statements for a word of `width` whose first
/// byte is chunk `base` (16-byte chunks) and stream byte `offset` (for `const`'s phase).
/// `carried` writes each carried instruction's statements.
fn emit(
    native: &BlockNative,
    width: LaneWidth,
    chosen: &[usize],
    base: &str,
    offset: &str,
    mut carried: impl FnMut(usize, &BlockInst, &mut String),
) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let bits = width.bytes();
    let insts: Vec<&BlockInst> = chosen.iter().map(|&at| &native.insts[at]).collect();
    let mut out = String::new();
    if insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_))) {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 c{c} = bytes[{base} + {c}u];\n"));
        }
    }
    let used: std::collections::BTreeSet<u16> =
        insts.iter().filter_map(|i| if let BlockInst::Class { table, .. } = **i { Some(table) } else { None }).collect();
    for t in used {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 k{t}_{c} = classes_t{t}(c{c});\n"));
        }
    }
    if insts.iter().any(|i| matches!(i, BlockInst::Plane(_))) {
        out.push_str(&planes(width));
    }
    for &at in chosen {
        let inst = &native.insts[at];
        if inst.carried() {
            carried(at, inst, &mut out);
            continue;
        }
        let rhs = match *inst {
            BlockInst::Class { table, bits: sel } => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("{m}(nz16(k{table}_{c} & {s4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::Plane(k) => format!("p{k}"),
            BlockInst::EqByte(byte) => {
                let c4 = u32::from(byte) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("{m}(eq16(c{c}, {c4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            // The pattern's bits from the word's first byte on: a right rotation by its phase.
            BlockInst::Const(c) => format!("{m}(rotate({c:#018x}ul, ulong((64u - (({offset}) & 63u)) & 63u)))"),
            BlockInst::Not(a) => format!("{m}(~v{a})"),
            BlockInst::And(a, b) => format!("v{a} & v{b}"),
            BlockInst::Or(a, b) => format!("v{a} | v{b}"),
            BlockInst::Xor(a, b) => format!("v{a} ^ v{b}"),
            BlockInst::AndNot(a, b) => format!("{m}(v{a} & ~v{b})"),
            BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::AfterOddRun(_) => unreachable!(),
        };
        out.push_str(&format!("  const {m} v{at} = {rhs};\n"));
    }
    let _ = bits;
    out
}

/// The instructions `at` reads, transitively, with `at`, in order.
fn cone(native: &BlockNative, roots: &[usize]) -> Vec<usize> {
    let mut keep = vec![false; native.insts.len()];
    let mut stack: Vec<usize> = roots.to_vec();
    while let Some(at) = stack.pop() {
        if std::mem::replace(&mut keep[at], true) {
            continue;
        }
        match native.insts[at] {
            BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Toggle(a) | BlockInst::AfterOddRun(a) => stack.push(usize::from(a)),
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => {
                stack.push(usize::from(a));
                stack.push(usize::from(b));
            }
            _ => {}
        }
    }
    (0..native.insts.len()).filter(|&at| keep[at]).collect()
}

/// `halo_window`: the operands of the local `prev`s over the 64 bytes from stream byte `base`
/// (a multiple of 16; before the stream, zero), as 64-bit words. A lane calls it once, with
/// `base` 64 bytes before its first byte: the words are those operands' halos.
fn halo_window(native: &BlockNative, plan: &Plan) -> String {
    let operands: Vec<usize> = plan
        .windowed
        .iter()
        .map(|&at| if let BlockInst::Prev(a, _) = native.insts[at] { usize::from(a) } else { unreachable!() })
        .collect();
    let mut wanted = operands.clone();
    wanted.sort_unstable();
    wanted.dedup();
    let params: String = wanted.iter().map(|a| format!(", thread ulong& w{a}")).collect();
    let mut out = format!(
        "// The 64 bytes from `base` (zero before the stream): the local prevs' operands.\n\
         static inline void halo_window(device const uint4* bytes, int base{params}) {{\n\
         \x20 const uint b = uint(max(base, 0));\n"
    );
    out.push_str(&emit(native, LaneWidth::W64, &cone(native, &wanted), "b / 16u", "b", |_, _, _| unreachable!("local")));
    for a in &wanted {
        // A window that starts before the stream: its bits from the stream's start on.
        out.push_str(&format!(
            "  w{a} = select(select(ulong(v{a}) << uint(-base), ulong(v{a}), base >= 0), 0ul, base <= -64);\n"
        ));
    }
    out.push_str("}\n\n");
    out
}

/// The straight-line statements defining `v0..v{upto}` for global lane `gl` of group `g`, lane
/// `l`: local `prev`s from `halo_window`, rounds from their states and halos.
fn lane_values(native: &BlockNative, plan: &Plan, width: LaneWidth, upto: usize) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let w = width.bytes();
    let chosen: Vec<usize> = (0..upto).collect();
    let mut out = String::new();
    let windowed: Vec<usize> = plan.windowed.iter().copied().filter(|&at| at < upto).collect();
    if !windowed.is_empty() {
        let mut operands: Vec<usize> = plan
            .windowed
            .iter()
            .map(|&at| if let BlockInst::Prev(a, _) = native.insts[at] { usize::from(a) } else { unreachable!() })
            .collect();
        operands.sort_unstable();
        operands.dedup();
        for a in &operands {
            out.push_str(&format!("  ulong w{a} = 0ul;\n"));
        }
        let args: String = operands.iter().map(|a| format!(", w{a}")).collect();
        out.push_str(&format!("  halo_window(bytes, int(gl * {w}u) - 64{args});\n"));
    }
    out.push_str(&emit(native, width, &chosen, &format!("gl * {chunks}u"), &format!("gl * {w}u"), |at, inst, out| {
        if let BlockInst::Prev(a, k) = *inst {
            match plan.round_of(at) {
                None => out.push_str(&format!("  const {m} v{at} = apply_prev(v{a}, {k}u, w{a});\n")),
                Some(j) => out.push_str(&format!(
                    "  const ulong h{j} = halo_before(v{a}, l, halo_of_group_before(halos + {j}u * groups, g));\n\
                     \x20 const {m} v{at} = apply_prev(v{a}, {k}u, h{j});\n"
                )),
            }
            return;
        }
        let j = plan.round_of(at).expect("a round");
        let (kind, apply) = match *inst {
            BlockInst::Toggle(a) => ("parity", format!("apply_toggle(v{a}, i{j})")),
            BlockInst::AfterOddRun(a) => ("borrow", format!("apply_after_odd_run(v{a}, i{j})")),
            _ => unreachable!(),
        };
        out.push_str(&format!(
            "  const uint e{j} = carry_before(states + {j}u * groups, {}u, g);\n\
             \x20 const uint s{j} = {};\n\
             \x20 const uint i{j} = lane_in_{kind}(s{j}, e{j}, l);\n\
             \x20 const {m} v{at} = {apply};\n",
            kind_index(inst),
            summary_of(inst)
        ));
    }));
    out
}

/// Generated MSL run: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn run_generated(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    run_generated_with(native, bytes, width, ClassStrategy::Planes).map(|(words, _)| words)
}

/// Generated MSL run with `class` by `strategy`: the outputs, and the GPU seconds of all the
/// rounds' dispatches.
pub(in crate::code_rule_dsl) fn run_generated_with(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(Vec<Vec<u64>>, f64), Error> {
    run_source(native, &generate_with(native, width, strategy), bytes, width)
}

/// Run generated `source` for `native`.
pub(in crate::code_rule_dsl) fn run_source(
    native: &BlockNative,
    source: &str,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<(Vec<Vec<u64>>, f64), Error> {
    // The planes form has the same plan: `class` is local in both, and carried instructions
    // are kept in order.
    let rounds = Plan::new(native).rounds.len();
    let pipelines: Vec<metal::ComputePipelineState> = (0..rounds)
        .map(|j| format!("tempo_round_{j}"))
        .chain(std::iter::once("tempo_block".to_owned()))
        .map(|name| {
            pipeline_named(source, &name).inspect_err(|e| eprintln!("block program: pipeline {name}: {e:?}"))
        })
        .collect::<Result<_, _>>()?;
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let stream = Stream::new(bytes, width, native.outputs.len())?;
    let states = MappedBuffer::zeroed::<u32>(device, rounds.max(1) * stream.groups)?;
    let halos = MappedBuffer::zeroed::<u64>(device, rounds.max(1) * stream.groups)?;
    let lanes = stream.lanes as u32;
    let passes: Vec<Pass<'_>> = pipelines
        .iter()
        .map(|pipeline| Pass {
            pipeline,
            buffers: vec![(0, &*stream.source.buffer, 0), (1, &*stream.values.buffer, 0), (3, &*states.buffer, 0), (4, &*halos.buffer, 0)],
            args: vec![(2, bytes_of(&lanes))],
            groups: stream.groups,
            threads: GROUP,
        })
        .collect();
    let seconds = encode_passes(&passes)?;
    let words = stream.slots(native.outputs.len());
    Ok((words.iter().map(|w| finish(w, bytes.len())).collect(), seconds))
}

/// The source padded to whole SIMD groups of lanes, and a buffer of `slots` slots of lane words.
struct Stream {
    source: MappedBuffer,
    values: MappedBuffer,
    lanes: usize,
    groups: usize,
    slot_bytes: usize,
    words: usize,
}

impl Stream {
    fn new(bytes: &[u8], width: LaneWidth, slots: usize) -> Result<Self, Error> {
        let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let span = width.bytes() * GROUP;
        let padded_len = bytes.len().div_ceil(span).max(1) * span;
        let mut flat = bytes.to_vec();
        flat.resize(padded_len, 0);
        let lanes = padded_len / width.bytes();
        let slot_bytes = padded_len / 8;
        Ok(Self {
            source: MappedBuffer::from_slice(device, &flat)?,
            values: MappedBuffer::zeroed::<u8>(device, slots.max(1) * slot_bytes)?,
            lanes,
            groups: lanes / GROUP,
            slot_bytes,
            words: bytes.len().div_ceil(64),
        })
    }

    /// Each slot as 64-bit words of the stream (lane words are little-endian and in lane order,
    /// so they are the stream's words).
    fn slots(&self, slots: usize) -> Vec<Vec<u64>> {
        let base = self.values.buffer.contents().cast::<u8>();
        (0..slots)
            .map(|slot| {
                // SAFETY: the buffer holds `slots * slot_bytes` bytes, written by the kernels.
                let raw = unsafe { std::slice::from_raw_parts(base.add(slot * self.slot_bytes), self.slot_bytes) };
                raw.chunks(8).take(self.words).map(|w| u64::from_le_bytes(w.try_into().unwrap())).collect()
            })
            .collect()
    }
}

/// One compute dispatch: buffers `(index, buffer, offset)`, small arguments by value, and the
/// grid as threadgroups of `threads`.
struct Pass<'a> {
    pipeline: &'a metal::ComputePipelineState,
    buffers: Vec<(u64, &'a metal::BufferRef, u64)>,
    args: Vec<(u64, Vec<u8>)>,
    groups: usize,
    threads: usize,
}

/// A failed command buffer's error, described (`error.localizedDescription`).
fn command_error(command: &metal::CommandBufferRef) -> String {
    unsafe extern "C" {
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }
    type Send = unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> *const std::ffi::c_void;
    // SAFETY: `error`, `localizedDescription` and `UTF8String` take no arguments and return
    // objects (or a C string); each result is checked for null before use.
    unsafe {
        let send: Send = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let error = send(metal::foreign_types::ForeignTypeRef::as_ptr(command).cast(), sel_registerName(c"error".as_ptr()));
        if error.is_null() {
            return "no error".into();
        }
        let text = send(error, sel_registerName(c"localizedDescription".as_ptr()));
        let utf8 = send(text, sel_registerName(c"UTF8String".as_ptr())).cast::<std::ffi::c_char>();
        if utf8.is_null() { "?".into() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

fn bytes_of<T: Copy>(value: &T) -> Vec<u8> {
    // SAFETY: `T` is a plain `repr(C)` value or an integer array; its bytes are read only.
    unsafe { std::slice::from_raw_parts(std::ptr::from_ref(value).cast::<u8>(), std::mem::size_of::<T>()) }.to_vec()
}

/// The passes in order, one encoder each, in one command buffer; returns its GPU seconds.
fn encode_passes(passes: &[Pass<'_>]) -> Result<f64, Error> {
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    // One queue for every block program run: a queue per run exhausts the GPU's memory when many
    // run at once ("Insufficient Memory").
    static QUEUE: std::sync::OnceLock<std::sync::Mutex<metal::CommandQueue>> = std::sync::OnceLock::new();
    let queue = QUEUE
        .get_or_init(|| std::sync::Mutex::new(device.new_command_queue()))
        .lock()
        .map_err(|_| Error::BackendFailure)?
        .clone();
    let command = queue.new_command_buffer();
    command.set_label("Rerail block program");
    for pass in passes {
        let encoder = command.new_compute_command_encoder();
        encoder.set_label("block_program");
        encoder.set_compute_pipeline_state(pass.pipeline);
        for &(index, buffer, offset) in &pass.buffers {
            encoder.set_buffer(index, Some(buffer), offset);
        }
        for (index, data) in &pass.args {
            encoder.set_bytes(*index, data.len() as u64, data.as_ptr().cast());
        }
        encoder.checked_dispatch_thread_groups(
            command,
            MTLSize::new(pass.groups as u64, 1, 1),
            MTLSize::new(pass.threads as u64, 1, 1),
        );
        encoder.end_encoding();
    }
    commit_checked(command)?;
    wait_checked(command).inspect_err(|_| eprintln!("block program: command buffer {:?}: {}", command.status(), command_error(command)))?;
    Ok(super::metal_backend::launch::gpu_seconds(command))
}

fn finish(words: &[u64], len: usize) -> Vec<u64> {
    let mut words = words.to_vec();
    mask_tail(&mut words, len);
    words
}
