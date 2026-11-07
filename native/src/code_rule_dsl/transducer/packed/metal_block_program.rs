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
//! Carried operations (`prev`, `toggle`, `after_odd_run`, `fill`) follow `06-block-parallel.md` §2 and
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
use crate::code_rule_dsl::transducer::packed::block::{BlockProgram, BlockRun, undelay};
use crate::code_rule_dsl::transducer::packed::native_ir::{BlockInst, BlockNative};
use metal::MTLSize;

/// The carries' helpers (`metal_block_carry.metal`), shared by the interpreter and generated MSL.
const CARRY: &str = include_str!("metal_block_carry.metal");
/// The extract pass's kernels (`metal_block_extract.metal`), shared likewise.
const EXTRACT: &str = include_str!("metal_block_extract.metal");
const SOURCE: &str = concat!(
    include_str!("metal_block_carry.metal"),
    include_str!("metal_block_program.metal"),
    include_str!("metal_block_extract.metal")
);
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
        BlockInst::AfterOddRun(_) | BlockInst::Fill(..) => 1,
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
        BlockInst::PlaneBefore(k) => [11, 0, 0, u32::from(k)],
        BlockInst::Prev(a, k) => [8, u32::from(a), round, u32::from(k)],
        BlockInst::Toggle(a) => [9, u32::from(a), round, 0],
        BlockInst::AfterOddRun(a) => [10, u32::from(a), round, 0],
        // Its second operand in `imm`: `b` is the round.
        BlockInst::Fill(a, e) => [12, u32::from(a), round, u32::from(e)],
        // The length is the dispatch's (`BlockArgs::pad`).
        BlockInst::Within => [13, 0, 0, 0],
        BlockInst::Class { .. } | BlockInst::Pair { .. } => unreachable!("the planes form has no class or pair"),
    }
}

/// The interpreter: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn interpret(
    program: &BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    interpret_extract(program, bytes, width).map(|run| run.outputs)
}

/// The interpreter with the extract pass: the outputs, then each extract's positions, counted
/// from its slot (`block_extract_count_W`) in the same command buffer.
pub(in crate::code_rule_dsl) fn interpret_extract(
    program: &BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<BlockRun, Error> {
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
    let stream = Stream::new(bytes, width, steps.len(), native.delay)?;
    let steps_buffer = MappedBuffer::from_slice(device, &steps)?;
    let states = MappedBuffer::zeroed::<u32>(device, carried.len().max(1) * stream.groups)?;
    let aggregates = MappedBuffer::zeroed::<u32>(device, stream.groups)?;
    let halos = MappedBuffer::zeroed::<u64>(device, carried.len().max(1) * stream.groups)?;
    // `pad` holds the stream's length, for `within`.
    let args = |from: usize, to: usize| BlockArgs { from: from as u32, to: to as u32, lanes: stream.lanes as u32, pad: bytes.len() as u32 };
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
    let planes: Vec<u64> = native.extracts.iter().map(|&k| (usize::from(native.outputs[k]) * stream.slot_bytes) as u64).collect();
    let extract = Extract::new(SOURCE, &stream, width, bytes.len(), native.extracts.len())?;
    passes.extend(extract.count_passes(&stream, &planes));
    passes.extend(extract.scan_passes());
    encode_passes(&passes)?;
    let positions = extract.scatter(&stream, &planes)?;
    let words = stream.slots(steps.len());
    let outputs = native.outputs.iter().map(|&v| undelay(&words[usize::from(v)], native.delay, bytes.len())).collect();
    Ok(BlockRun { outputs, positions })
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

/// The native IR as MSL for lanes of `width`, `class` by `strategy`: one kernel, `tempo_block`,
/// that computes each instruction once, in order, with a look-back for each carried one, and
/// writes the outputs (`13-tape-on-the-gpu.md` §5.3).
pub(in crate::code_rule_dsl) fn generate_with(native: &BlockNative, width: LaneWidth, strategy: ClassStrategy) -> String {
    generate_polling(native, width, strategy, SPINS)
}

/// `generate_with`, polling a predecessor's state once and then `spins` more times before the
/// group gives up and is run again (`run_source`); 0 tests that path.
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
    if native.insts.iter().any(|i| matches!(i, BlockInst::Class { .. } | BlockInst::Pair { .. })) {
        helpers.push_str(NZ16);
        for (t, table) in native.tables.iter().enumerate() {
            helpers.push_str(&lookup_helper(t, table));
        }
    }
    if native.insts.iter().any(|i| matches!(i, BlockInst::Plane(_) | BlockInst::PlaneBefore(_))) {
        helpers.push_str(TRANSPOSE8);
    }
    let plan = Plan::new(native);
    let extract = if native.extracts.is_empty() { "" } else { EXTRACT };
    let mut out = format!("{CARRY}\n{extract}\n{EQ16}\n{helpers}\n");
    if !plan.scanned.is_empty() {
        out.push_str(&table_helpers(1));
        out.push_str(LOOK_BACK);
    }
    if !plan.halos.is_empty() {
        out.push_str(HALO_EXCHANGE);
    }
    if !plan.windowed.is_empty() {
        out.push_str(&halo_window(native, &plan));
    }
    out.push_str(&format!(
        "kernel void tempo_block(\n\
         \x20   device const uint4* bytes [[buffer(0)]],\n\
         \x20   device {}* out [[buffer(1)]],\n\
         \x20   constant uint& lanes [[buffer(2)]],\n\
         \x20   device atomic_uint* states [[buffer(3)]],\n\
         \x20   device atomic_uint* halos [[buffer(4)]],\n\
         \x20   constant uint& len [[buffer(5)]],\n\
         \x20   device atomic_uint* extract_states [[buffer(6)]],\n\
         \x20   device uint* positions [[buffer(7)]],\n\
         \x20   device atomic_uint* deferred [[buffer(8)]],\n\
         \x20   constant uint& capacity [[buffer(9)]],\n\
         \x20   device uint* counts [[buffer(10)]],\n\
         \x20   device atomic_uint* tickets [[buffer(11)]],\n\
         \x20   device uint* done [[buffer(12)]],\n\
         \x20   uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 const uint groups = lanes / {GROUP}u;\n\
         \x20 // Groups take their numbers in launch order, so a group waits only on groups that\n\
         \x20 // started before it (13-tape-on-the-gpu.md §5.3).\n\
         \x20 uint ticket = 0u;\n\
         \x20 if (l == 0u) ticket = atomic_fetch_add_explicit(&tickets[0], 1u, memory_order_relaxed);\n\
         \x20 const uint g = simd_broadcast(ticket, ushort(0));\n\
         \x20 if (g >= groups || done[g] != 0u) return;\n\
         \x20 const uint gl = g * {GROUP}u + l;\n",
        width.word()
    ));
    out.push_str(&body(native, &plan, width, spins));
    for (k, v) in native.outputs.iter().enumerate() {
        out.push_str(&format!("  out[{k}u * lanes + gl] = v{v};\n"));
    }
    // The extract pass, in the common case here: each extract's positions, from the plane just
    // written, at offsets found by look-back (`extract_write`). Its fallback is a second command
    // buffer, not a recompute, so it polls longer than the carries do (still bounded).
    let extract_spins = spins * EXTRACT_POLLS;
    for (e, &k) in native.extracts.iter().enumerate() {
        out.push_str(&format!(
            "  extract_write(v{}, gl, g, l, len, extract_states + {e}u * groups, positions + {e}u * capacity, capacity, deferred + {e}u, counts + {e}u * groups, {extract_spins}u);\n",
            native.outputs[k]
        ));
    }
    out.push_str("  if (l == 0u) done[g] = 1u;\n}\n");
    out
}

/// Polls of a predecessor's state after the first.
const SPINS: u32 = 64;
/// The extract look-back's polls per predecessor, as a multiple of the carries'.
const EXTRACT_POLLS: u32 = 64;

/// One scanned carry's look-back (`06-block-parallel.md` §5): the lanes' tables (bit `c` is the
/// lane's carry-out for carry-in `c`) scanned in the SIMD group, the group's aggregate published,
/// then the predecessors' states read back to an inclusive one. Each predecessor is polled once
/// and `spins` more times; false when one has published nothing yet.
const LOOK_BACK: &str = "static inline bool look_back(device atomic_uint* state, uint g, uint l, uint t, uint spins, thread uint& cin) {
  uint agg = 0u;
  const uint e = texclusive1(t, l, agg);
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_AGGREGATE | agg, memory_order_relaxed);
  uint prefix = TID1;
  for (uint back = 1u; back <= g; back++) {
    uint st = 0u;
    for (uint spin = 0u; spin <= spins && (st >> 30u) == 0u; spin++) {
      st = simd_broadcast(atomic_load_explicit(&state[g - back], memory_order_relaxed), ushort(0));
    }
    if ((st >> 30u) == 0u) return false;
    prefix = tcompose1(st & CARRY_VALUE, prefix);
    if ((st >> 30u) == 2u) break;
  }
  if (l == 0u) atomic_store_explicit(&state[g], CARRY_INCLUSIVE | tcompose1(prefix, agg), memory_order_relaxed);
  cin = tapply1(e, tapply1(prefix, 0u));
  return true;
}

";

/// A carried `prev`'s halo: the group's last 64 bits of the operand, published as three words of
/// 30, 30 and 4 bits, each with a flag (no fence: every word says whether it is written), and the
/// group before's read back. False when the group before has not published it yet.
const HALO_EXCHANGE: &str = "constant uint HALO_FLAG = 1u << 31;
static inline bool halo_exchange(device atomic_uint* halos, uint g, uint l, ulong last, uint spins, thread ulong& before) {
  if (l == 0u) {
    atomic_store_explicit(&halos[g * 3u], HALO_FLAG | uint(last & 0x3ffffffful), memory_order_relaxed);
    atomic_store_explicit(&halos[g * 3u + 1u], HALO_FLAG | uint((last >> 30u) & 0x3ffffffful), memory_order_relaxed);
    atomic_store_explicit(&halos[g * 3u + 2u], HALO_FLAG | uint(last >> 60u), memory_order_relaxed);
  }
  before = 0ul;
  if (g == 0u) return true;
  for (uint w = 0u; w < 3u; w++) {
    uint x = 0u;
    for (uint spin = 0u; spin <= spins && (x & HALO_FLAG) == 0u; spin++) {
      x = simd_broadcast(atomic_load_explicit(&halos[(g - 1u) * 3u + w], memory_order_relaxed), ushort(0));
    }
    if ((x & HALO_FLAG) == 0u) return false;
    before |= ulong(x & 0x3ffffffful) << (30u * w);
  }
  return true;
}

";

/// How generated MSL treats each carried instruction.
struct Plan {
    /// The local `prev`s: of a value of the bytes alone with no `prev` in it; their halo is
    /// recomputed over the 64 bytes before the lane (`halo_window`).
    windowed: Vec<usize>,
    /// The scanned carries (`toggle`, `after_odd_run`, `fill`), in order: a state per group each.
    scanned: Vec<usize>,
    /// The other `prev`s, in order: a published halo per group each.
    halos: Vec<usize>,
}

/// Where a carried instruction is resolved.
enum Placement {
    Scanned(usize),
    Halo(usize),
    Windowed,
}

impl Plan {
    fn new(native: &BlockNative) -> Self {
        let n = native.insts.len();
        let mut local = vec![true; n];
        let mut has_prev = vec![false; n];
        let mut plan = Plan { windowed: Vec::new(), scanned: Vec::new(), halos: Vec::new() };
        for at in 0..n {
            let ops = operands(&native.insts[at]);
            let ops_local = ops.iter().all(|&a| local[a]);
            let ops_prev = ops.iter().any(|&a| has_prev[a]);
            match native.insts[at] {
                BlockInst::Prev(a, _) if ops_local && !has_prev[usize::from(a)] => {
                    plan.windowed.push(at);
                    has_prev[at] = true;
                }
                BlockInst::Prev(..) => {
                    local[at] = false;
                    has_prev[at] = true;
                    plan.halos.push(at);
                }
                ref inst if inst.carried() => {
                    local[at] = false;
                    has_prev[at] = ops_prev;
                    plan.scanned.push(at);
                }
                _ => {
                    local[at] = ops_local;
                    has_prev[at] = ops_prev;
                }
            }
        }
        plan
    }

    fn placement(&self, at: usize) -> Placement {
        if self.windowed.contains(&at) {
            Placement::Windowed
        } else if let Some(j) = self.scanned.iter().position(|&x| x == at) {
            Placement::Scanned(j)
        } else if let Some(j) = self.halos.iter().position(|&x| x == at) {
            Placement::Halo(j)
        } else {
            unreachable!("not carried")
        }
    }
}

fn operands(inst: &BlockInst) -> Vec<usize> {
    match *inst {
        BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Toggle(a) | BlockInst::AfterOddRun(a) => vec![usize::from(a)],
        BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) | BlockInst::Fill(a, b) => {
            vec![usize::from(a), usize::from(b)]
        }
        _ => vec![],
    }
}

/// Tables of `m`-bit carry vectors: apply, compose (`a` then `b`), the identity, and the SIMD
/// group's exclusive scan (Hillis-Steele over shuffles, five steps) with the group's total.
fn table_helpers(m: usize) -> String {
    let mask = (1u32 << m) - 1;
    let n = 1usize << m;
    let compose: Vec<String> = (0..n).map(|c| format!("(tapply{m}(b, tapply{m}(a, {c}u)) << {}u)", c * m)).collect();
    let identity: u32 = (0..n as u32).map(|c| c << (c as usize * m)).sum();
    let steps: String = [1, 2, 4, 8, 16]
        .iter()
        .map(|d| format!("  x = select(x, tcompose{m}(simd_shuffle_up(x, ushort({d})), x), l >= {d}u);\n"))
        .collect();
    format!(
        "static inline uint tapply{m}(uint t, uint c) {{ return (t >> (c * {m}u)) & {mask}u; }}\n\
         static inline uint tcompose{m}(uint a, uint b) {{ return {}; }}\n\
         constant uint TID{m} = {identity}u;\n\
         static inline uint texclusive{m}(uint t, uint l, thread uint& total) {{\n\
         \x20 uint x = t;\n{steps}\
         \x20 total = simd_broadcast(x, ushort(31));\n\
         \x20 return select(simd_shuffle_up(x, ushort(1)), TID{m}, l == 0u);\n}}\n\n",
        compose.join(" | ")
    )
}

/// Every instruction of lane `gl` (group `g`, lane `l`) in order, each once. A carry's look-back
/// or a halo that is not published yet counts the group as unfinished and leaves it.
fn body(native: &BlockNative, plan: &Plan, width: LaneWidth, spins: u32) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let w = width.bytes();
    let mut out = String::new();
    if !plan.windowed.is_empty() {
        let mut ops: Vec<usize> = plan.windowed.iter().flat_map(|&at| operands(&native.insts[at])).collect();
        ops.sort_unstable();
        ops.dedup();
        for a in &ops {
            out.push_str(&format!("  ulong w{a} = 0ul;\n"));
        }
        let wargs: String = ops.iter().map(|a| format!(", w{a}")).collect();
        out.push_str(&format!("  halo_window(bytes, len, int(gl * {w}u) - 64{wargs});\n"));
    }
    let leave = "{ if (l == 0u) atomic_fetch_add_explicit(&tickets[1], 1u, memory_order_relaxed); return; }";
    let all: Vec<usize> = (0..native.insts.len()).collect();
    out.push_str(&emit(native, width, &all, &format!("gl * {chunks}u"), &format!("gl * {w}u"), |at, inst, out| {
        // A scanned carry: the lane's results for carry-ins 0 and 1, its table, the look-back.
        let scanned = |out: &mut String, j: usize, f: &str| {
            out.push_str(&format!(
                "  uint o{at}_0 = 0u, o{at}_1 = 0u, cin{at} = 0u;\n\
                 \x20 const {m} f{at}_0 = {}, f{at}_1 = {};\n\
                 \x20 if (!look_back(states + {j}u * groups, g, l, (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u), {spins}u, cin{at})) {leave}\n\
                 \x20 const {m} v{at} = select(f{at}_0, f{at}_1, cin{at} != 0u);\n",
                f.replace("CIN", &format!("{m}(0)")).replace("OUT", &format!("o{at}_0")),
                f.replace("CIN", &format!("{m}(1)")).replace("OUT", &format!("o{at}_1")),
            ));
        };
        match (plan.placement(at), *inst) {
            (Placement::Windowed, BlockInst::Prev(a, k)) => out.push_str(&format!("  const {m} v{at} = apply_prev(v{a}, {k}u, w{a});\n")),
            (Placement::Halo(j), BlockInst::Prev(a, k)) => out.push_str(&format!(
                "  ulong h{at} = 0ul;\n\
                 \x20 if (!halo_exchange(halos + {j}u * groups * 3u, g, l, halo_last(v{a}), {spins}u, h{at})) {leave}\n\
                 \x20 const {m} v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, h{at}));\n"
            )),
            (Placement::Scanned(j), BlockInst::Toggle(a)) => out.push_str(&format!(
                "  const uint s{at} = summary_toggle(v{a});\n\
                 \x20 uint cin{at} = 0u;\n\
                 \x20 if (!look_back(states + {j}u * groups, g, l, s{at} | ((1u ^ s{at}) << 1u), {spins}u, cin{at})) {leave}\n\
                 \x20 const {m} v{at} = apply_toggle(v{a}, cin{at});\n"
            )),
            (Placement::Scanned(j), BlockInst::AfterOddRun(a)) => scanned(out, j, &format!("after_odd_run(v{a}, CIN, OUT)")),
            (Placement::Scanned(j), BlockInst::Fill(a, e)) => scanned(out, j, &format!("fill_word(v{a}, v{e}, CIN, OUT)")),
            _ => unreachable!("placement"),
        }
    }));
    out
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
    let back = insts.iter().any(|i| matches!(i, BlockInst::Pair { .. } | BlockInst::PlaneBefore(_)));
    if back || insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_))) {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 c{c} = bytes[{base} + {c}u];\n"));
        }
    }
    // The byte before the word, read from the source (no carry); and, for `pair` by lookups,
    // the word's bytes one back.
    if back {
        out.push_str(&format!("  const uint before = byte_before(bytes, {base});\n"));
    }
    let used: std::collections::BTreeSet<u16> = insts
        .iter()
        .filter_map(|i| match **i {
            BlockInst::Class { table, .. } | BlockInst::Pair { table, .. } => Some(table),
            _ => None,
        })
        .collect();
    for t in used {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 k{t}_{c} = classes_t{t}(c{c});\n"));
        }
    }
    let used_before: std::collections::BTreeSet<u16> =
        insts.iter().filter_map(|i| if let BlockInst::Pair { before, .. } = **i { Some(before) } else { None }).collect();
    if !used_before.is_empty() {
        for c in 0..chunks {
            let prior = if c == 0 { "before".to_owned() } else { format!("(c{}.w >> 24u)", c - 1) };
            out.push_str(&format!("  const uint4 b{c} = bytes_before(c{c}, {prior});\n"));
        }
    }
    for t in used_before {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 kb{t}_{c} = classes_t{t}(b{c});\n"));
        }
    }
    if insts.iter().any(|i| matches!(i, BlockInst::Plane(_) | BlockInst::PlaneBefore(_))) {
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
            BlockInst::PlaneBefore(k) => format!("{m}(({m}(p{k}) << 1u) | {m}((before >> {k}u) & 1u))"),
            BlockInst::Pair { before, table, bits: sel } => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("{m}(nz16(kb{before}_{c} & k{table}_{c} & {s4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
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
            // The bits of the word's bytes below the stream's end.
            BlockInst::Within => format!("within_lane<{m}>(len, {offset})"),
            BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::AfterOddRun(_) | BlockInst::Fill(..) => unreachable!(),
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
            BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) | BlockInst::Fill(a, b) => {
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
         static inline void halo_window(device const uint4* bytes, uint len, int base{params}) {{\n\
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

/// Generated MSL run: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn run_generated(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    run_generated_with(native, bytes, width, ClassStrategy::Planes).map(|(words, _)| words)
}

/// Generated MSL run with `class` by `strategy`: the outputs, and the GPU seconds of its
/// launches.
pub(in crate::code_rule_dsl) fn run_generated_with(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(Vec<Vec<u64>>, f64), Error> {
    run_source(native, &generate_with(native, width, strategy), bytes, width).map(|(run, seconds)| (run.outputs, seconds))
}

/// Generated MSL with the extract pass: the outputs and each extract's positions, and the GPU
/// seconds of the mask passes with the count's scan (the scatter is a second command buffer,
/// after the host has allocated the positions at their total).
pub(in crate::code_rule_dsl) fn run_generated_extract(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(BlockRun, f64), Error> {
    run_source(native, &generate_with(native, width, strategy), bytes, width)
}

/// Run generated `source` for `native`: one launch in the common case. A group whose look-back
/// ran out of polls leaves unfinished, and the launch is repeated for those groups: each repeat
/// finishes at least the first unfinished group, whose predecessors are all done.
pub(in crate::code_rule_dsl) fn run_source(
    native: &BlockNative,
    source: &str,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<(BlockRun, f64), Error> {
    // The planes form has the same plan: `class` is local in both, and carried instructions
    // are kept in order.
    let plan = Plan::new(native);
    let pipeline = pipeline_named(source, "tempo_block").inspect_err(|e| eprintln!("block program: pipeline tempo_block: {e:?}"))?;
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let stream = Stream::new(bytes, width, native.outputs.len(), native.delay)?;
    let states = MappedBuffer::zeroed::<u32>(device, plan.scanned.len().max(1) * stream.groups)?;
    let halos = MappedBuffer::zeroed::<u32>(device, plan.halos.len().max(1) * stream.groups * 3)?;
    let tickets = MappedBuffer::zeroed::<u32>(device, 2)?;
    let done = MappedBuffer::zeroed::<u32>(device, stream.groups)?;
    let lanes = stream.lanes as u32;
    let len = bytes.len() as u32;
    // `tempo_block` writes each extract's positions itself (`extract_write`), into buffers of
    // an upper bound: a plane has at most one position per byte.
    let extracts = native.extracts.len();
    let capacity = bytes.len().max(1);
    let extract = Extract::new(source, &stream, width, bytes.len(), extracts)?;
    let extract_states = MappedBuffer::zeroed::<u32>(device, extracts.max(1) * stream.groups)?;
    let written = MappedBuffer::zeroed::<u32>(device, extracts.max(1) * capacity)?;
    let deferred = MappedBuffer::zeroed::<u32>(device, extracts.max(1))?;
    let capacity_arg = capacity as u32;
    let pass = Pass {
        pipeline: &pipeline,
        buffers: vec![
            (0, &*stream.source.buffer, 0),
            (1, &*stream.values.buffer, 0),
            (3, &*states.buffer, 0),
            (4, &*halos.buffer, 0),
            (6, &*extract_states.buffer, 0),
            (7, &*written.buffer, 0),
            (8, &*deferred.buffer, 0),
            (10, &*extract.counts.buffer, 0),
            (11, &*tickets.buffer, 0),
            (12, &*done.buffer, 0),
        ],
        args: vec![(2, bytes_of(&lanes)), (5, bytes_of(&len)), (9, bytes_of(&capacity_arg))],
        groups: stream.groups,
        threads: GROUP,
    };
    let read = |buffer: &MappedBuffer, at: usize| -> u32 {
        // SAFETY: every index read is inside the buffer's allocation (sizes above).
        unsafe { *buffer.buffer.contents().cast::<u32>().add(at) }
    };
    let mut seconds = 0.0;
    let mut finished = false;
    for launch in 0..stream.groups {
        if launch > 0 {
            REPEATED_LAUNCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // SAFETY: two words, written between launches, when no kernel runs.
            unsafe { std::ptr::write_bytes(tickets.buffer.contents().cast::<u32>(), 0, 2) };
        }
        seconds += encode_passes(std::slice::from_ref(&pass))?;
        if read(&tickets, 1) == 0 {
            finished = true;
            break;
        }
    }
    if !finished {
        return Err(Error::BackendFailure);
    }
    let complete = (0..extracts).all(|e| read(&deferred, e) == 0);
    let positions = if complete {
        (0..extracts)
            .map(|e| {
                let total: usize = (0..stream.groups).map(|g| read(&extract.counts, e * stream.groups + g) as usize).sum();
                (0..total).map(|i| read(&written, e * capacity + i)).collect()
            })
            .collect()
    } else {
        // The fallback, when a look-back ran out of polls: the scan of the counts, then the scatter.
        EXTRACT_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        encode_passes(&extract.scan_passes())?;
        let planes: Vec<u64> = native.extracts.iter().map(|&k| (k * stream.slot_bytes) as u64).collect();
        extract.scatter(&stream, &planes)?
    };
    let words = stream.slots(native.outputs.len());
    let outputs = words.iter().map(|w| undelay(w, native.delay, bytes.len())).collect();
    Ok((BlockRun { outputs, positions }, seconds))
}

/// Launches of generated MSL repeated because a look-back ran out of polls.
pub(in crate::code_rule_dsl) static REPEATED_LAUNCHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Runs of generated MSL whose extract pass needed the fallback (a second command buffer).
pub(in crate::code_rule_dsl) static EXTRACT_FALLBACKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The extract pass's buffers (`metal_block_extract.metal`): each extract's chunk counts (a
/// chunk is a group's span), and its chunk offsets with the total after them.
struct Extract {
    counts: MappedBuffer,
    offsets: MappedBuffer,
    count: Option<metal::ComputePipelineState>,
    scan: Option<metal::ComputePipelineState>,
    scatter: Option<metal::ComputePipelineState>,
    extracts: usize,
    chunks: usize,
    len: u32,
}

impl Extract {
    fn new(source: &str, stream: &Stream, width: LaneWidth, len: usize, extracts: usize) -> Result<Self, Error> {
        let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let chunks = stream.groups;
        let w = width.bytes();
        let pipeline = |name: String| if extracts > 0 { pipeline_named(source, &name).map(Some) } else { Ok(None) };
        Ok(Self {
            counts: MappedBuffer::zeroed::<u32>(device, extracts.max(1) * chunks)?,
            offsets: MappedBuffer::zeroed::<u32>(device, extracts.max(1) * (chunks + 1))?,
            count: pipeline(format!("block_extract_count_{w}"))?,
            scan: pipeline("block_extract_scan".into())?,
            scatter: pipeline(format!("block_extract_scatter_{w}"))?,
            extracts,
            chunks,
            len: len as u32,
        })
    }

    fn counts_at(&self, e: usize) -> u64 {
        (e * self.chunks * 4) as u64
    }

    fn offsets_at(&self, e: usize) -> u64 {
        (e * (self.chunks + 1) * 4) as u64
    }

    /// The interpreter's counts: each plane (a byte offset into the values) counted per chunk.
    fn count_passes<'a>(&'a self, stream: &'a Stream, planes: &[u64]) -> Vec<Pass<'a>> {
        let Some(count) = &self.count else { return vec![] };
        planes
            .iter()
            .enumerate()
            .map(|(e, &plane)| Pass {
                pipeline: count,
                buffers: vec![(0, &*stream.values.buffer, plane), (1, &*self.counts.buffer, self.counts_at(e))],
                args: vec![(2, bytes_of(&self.len))],
                groups: stream.groups,
                threads: GROUP,
            })
            .collect()
    }

    /// Each extract's scan of its counts: one SIMD group.
    fn scan_passes(&self) -> Vec<Pass<'_>> {
        let Some(scan) = &self.scan else { return vec![] };
        let chunks = self.chunks as u32;
        (0..self.extracts)
            .map(|e| Pass {
                pipeline: scan,
                buffers: vec![(0, &*self.counts.buffer, self.counts_at(e)), (1, &*self.offsets.buffer, self.offsets_at(e))],
                args: vec![(2, bytes_of(&chunks))],
                groups: 1,
                threads: GROUP,
            })
            .collect()
    }

    /// After the counts and scans have run: each extract's positions, in buffers allocated at
    /// the scanned totals and written by one scatter dispatch each.
    fn scatter(&self, stream: &Stream, planes: &[u64]) -> Result<Vec<Vec<u32>>, Error> {
        let Some(scatter) = &self.scatter else { return Ok(vec![]) };
        let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let offsets = self.offsets.buffer.contents().cast::<u32>();
        // SAFETY: the buffer holds `extracts * (chunks + 1)` offsets, written by the scans.
        let totals: Vec<usize> =
            (0..self.extracts).map(|e| unsafe { *offsets.add(e * (self.chunks + 1) + self.chunks) } as usize).collect();
        let buffers: Vec<MappedBuffer> =
            totals.iter().map(|&n| MappedBuffer::zeroed::<u32>(device, n.max(1))).collect::<Result<_, _>>()?;
        let passes: Vec<Pass<'_>> = planes
            .iter()
            .enumerate()
            .map(|(e, &plane)| Pass {
                pipeline: scatter,
                buffers: vec![
                    (0, &*stream.values.buffer, plane),
                    (1, &*self.offsets.buffer, self.offsets_at(e)),
                    (3, &*buffers[e].buffer, 0),
                ],
                args: vec![(2, bytes_of(&self.len))],
                groups: stream.groups,
                threads: GROUP,
            })
            .collect();
        encode_passes(&passes)?;
        Ok(buffers
            .iter()
            .zip(&totals)
            .map(|(b, &n)| {
                // SAFETY: the buffer holds at least `n` positions, written by the scatter.
                unsafe { std::slice::from_raw_parts(b.buffer.contents().cast::<u32>(), n) }.to_vec()
            })
            .collect())
    }
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
    /// The stream and `delay` bytes past it (`block::BlockProgram::delay`), in groups' spans.
    fn new(bytes: &[u8], width: LaneWidth, slots: usize, delay: u8) -> Result<Self, Error> {
        let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let span = width.bytes() * GROUP;
        let run = bytes.len() + usize::from(delay);
        let padded_len = run.div_ceil(span).max(1) * span;
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
            words: run.div_ceil(64),
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
    if let Err(refused) = commit_checked(command) {
        // The budget refused a dispatch: the command buffer still holds a place in the shared
        // queue until it completes, and a queue holds 64. Commit what was encoded (the results
        // are not read) so the place is freed.
        command.commit();
        command.wait_until_completed();
        return Err(refused);
    }
    wait_checked(command).inspect_err(|_| eprintln!("block program: command buffer {:?}: {}", command.status(), command_error(command)))?;
    Ok(super::metal_backend::launch::gpu_seconds(command))
}

