//! Block programs on Metal (`block`), both ways the transducer runs on the device, each over a
//! batch of files in one launch (decision 0044):
//!
//! - **the interpreter** (`metal_block_program.metal`): one kernel for any programs, reading each
//!   program's steps from a table;
//! - **generated MSL** from the native IR (`BlockNative`): a function per program, the operations
//!   written out straight-line, and one kernel that calls the file's.
//!
//! Each lane owns `W` consecutive bytes of a file and builds its own `W`-bit word of every value
//! (`LaneWidth`: 16, 32 or 64), so a SIMD group's 32 lanes work on 32 words at once. A group of
//! lanes holds one file, so it runs one program. Files are read in place through a file table
//! (`FileRow`), as zero outside them.
//!
//! Carried operations (`prev`, `toggle`, `after_odd_run`, `fill`) follow `06-block-parallel.md` §2
//! and §5. A lane's carry is combined across its SIMD group by SIMD-group operations, and across a
//! file's groups by decoupled look-back with bounded polls (`13-tape-on-the-gpu.md` §5.3). Groups
//! take their numbers in launch order from persistent workers; a group waits only on earlier groups
//! of its file. A poll that runs out leaves the group unfinished, and the host launches again.
use super::*;
pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::block::BatchFile;
use crate::code_rule_dsl::transducer::packed::block::{BlockRun, undelay};
use crate::code_rule_dsl::transducer::packed::native_ir::{BlockInst, BlockNative};
use metal::MTLSize;

/// The carries' helpers (`metal_block_carry.metal`), shared by the interpreter and generated MSL.
const CARRY: &str = include_str!("metal_block_carry.metal");
/// The extract pass (`metal_block_extract.metal`), shared likewise.
const EXTRACT: &str = include_str!("metal_block_extract.metal");
/// The interpreter's source: the shared helpers, then its kernels.
static INTERPRETER: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("{CARRY}\n{}\n{}", batch_helpers(), include_str!("metal_block_program.metal"))
});
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

/// A file of a batch, as the kernels read it (`FileRow` in `batch_helpers`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileRow {
    src16: u32,
    len: u32,
    program: u32,
    group0: u32,
    groups: u32,
    out_base: u32,
    state_base: u32,
    halo_base: u32,
    ext_base: u32,
    pos_base: u32,
    /// The batch's slab binding the file's bytes are in (`SLAB_BUFFERS`).
    slab: u32,
    pad: u32,
}

/// A program of a batch, as the interpreter reads it (`ProgramRow` in the interpreter).
#[repr(C)]
#[derive(Clone, Copy)]
struct ProgramRow {
    step0: u32,
    steps: u32,
    out0: u32,
    outs: u32,
    ext0: u32,
    exts: u32,
    pad: [u32; 2],
}

/// An instruction of the planes form as the interpreter's step: `[op, a, b, imm]`; a carried
/// instruction's `b` is its index among the program's carried instructions.
fn encoded(inst: &BlockInst, carry: u32) -> [u32; 4] {
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
        BlockInst::Prev(a, k) => [8, u32::from(a), carry, u32::from(k)],
        BlockInst::Toggle(a) => [9, u32::from(a), carry, 0],
        BlockInst::AfterOddRun(a) => [10, u32::from(a), carry, 0],
        // Its second operand in `imm`: `b` is the carry's index.
        BlockInst::Fill(a, e) => [12, u32::from(a), carry, u32::from(e)],
        BlockInst::Within(d) => [13, 0, 0, u32::from(d)],
        BlockInst::Class { .. } | BlockInst::Pair { .. } => unreachable!("the planes form has no class or pair"),
    }
}

/// Which Metal executor runs a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Executor {
    Interpreter,
    Generated(ClassStrategy),
}


/// A batch's results: each file's outputs and extract positions, the launches it took, and their
/// GPU seconds.
pub(in crate::code_rule_dsl) struct BatchRun {
    pub(in crate::code_rule_dsl) files: Vec<BlockRun>,
    pub(in crate::code_rule_dsl) launches: usize,
    pub(in crate::code_rule_dsl) seconds: f64,
}

/// Workers of the persistent dispatch: SIMD groups that take the batch's groups in order.
const WORKERS: usize = 256;

/// `run_batch`, split while the device refuses it for capacity (lessons ledger #26: a batch over
/// a budget is split, never refused). Halves run one after another, in the files' order; a file
/// alone that does not fit is the error. `launches` and `seconds` are the halves' sums.
pub(in crate::code_rule_dsl) fn run_split(
    programs: &[&BlockNative],
    files: &[BatchFile<'_>],
    width: LaneWidth,
    executor: Executor,
    spins: u32,
) -> Result<BatchRun, Error> {
    match run_batch(programs, files, width, executor, spins) {
        Err(Error::BackendCapacity) if files.len() > 1 => {
            let (a, b) = files.split_at(files.len() / 2);
            let mut first = run_split(programs, a, width, executor, spins)?;
            let second = run_split(programs, b, width, executor, spins)?;
            first.files.extend(second.files);
            first.launches += second.launches;
            first.seconds += second.seconds;
            Ok(first)
        }
        other => other,
    }
}

/// The bindings of a batch's slabs (`MAX_SLABS`): 0, then past the interpreter's 12–15.
const SLAB_BUFFERS: [u64; crate::code_rule_dsl::residency::slabs::MAX_SLABS] = [0, 16, 17, 18];

/// Launches repeated because a look-back ran out of polls.
pub(in crate::code_rule_dsl) static REPEATED_LAUNCHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Run `files` in one launch, each under `programs[file.program]` (decision 0044). `spins` is the
/// polls of a predecessor after the first; 0 tests the repeated launch.
pub(in crate::code_rule_dsl) fn run_batch(
    programs: &[&BlockNative],
    files: &[BatchFile<'_>],
    width: LaneWidth,
    executor: Executor,
    spins: u32,
) -> Result<BatchRun, Error> {
    METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let word = width.bytes() / 8;
    let span = width.bytes() * GROUP;
    // The form each executor runs: `class` by lookups only in generated MSL's `Lookup`.
    let forms: Vec<BlockNative> = programs
        .iter()
        .map(|p| if executor == Executor::Generated(ClassStrategy::Lookup) { (*p).clone() } else { p.planes_form() })
        .collect();
    let carried: Vec<usize> = forms.iter().map(|n| n.insts.iter().filter(|i| i.carried()).count()).collect();
    let fit = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
    // The files' bytes, resident in slabs (decision 0045): read in place, at most `MAX_SLABS`
    // slabs a batch; past that the caller splits the batch.
    let bytes: Vec<&[u8]> = files.iter().map(|f| f.bytes).collect();
    let sources = crate::code_rule_dsl::residency::slabs::admit_sources(&bytes)?;
    let (slabs, slab_of) = crate::code_rule_dsl::residency::slabs::source_slabs(&sources).ok_or(Error::BackendCapacity)?;
    if files.is_empty() {
        return Ok(BatchRun { files: Vec::new(), launches: 0, seconds: 0.0 });
    }
    // The file table, and each base after the files before.
    let (mut group0, mut out_base, mut state_base, mut halo_base, mut ext_base, mut pos_base) = (0, 0, 0, 0, 0, 0);
    let mut rows = Vec::with_capacity(files.len());
    let mut group_file: Vec<u32> = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let n = forms.get(file.program).ok_or(Error::BackendFailure)?;
        let len = file.bytes.len();
        let groups = (len + usize::from(n.delay)).div_ceil(span).max(1);
        rows.push(FileRow {
            src16: fit(sources[i].offset / 16)?,
            len: fit(len)?,
            program: fit(file.program)?,
            group0: fit(group0)?,
            groups: fit(groups)?,
            out_base: fit(out_base)?,
            state_base: fit(state_base)?,
            halo_base: fit(halo_base)?,
            ext_base: fit(ext_base)?,
            pos_base: fit(pos_base)?,
            slab: slab_of[i],
            pad: 0,
        });
        group_file.extend(std::iter::repeat_n(fit(i)?, groups));
        group0 += groups;
        out_base += n.outputs.len() * groups * GROUP;
        state_base += carried[file.program] * groups;
        halo_base += carried[file.program] * groups * 3;
        ext_base += n.extracts.len() * groups;
        pos_base += n.extracts.len() * len.max(1);
    }
    let total = group0;
    let table = super::blocks::upload(&rows)?;
    let group_file = super::blocks::upload(&group_file)?;
    let out = super::blocks::zeroed::<u8>(out_base.max(1) * word)?;
    let states = super::blocks::zeroed::<u32>(state_base.max(1))?;
    let halos = super::blocks::zeroed::<u32>(halo_base.max(1))?;
    let ext_states = super::blocks::zeroed::<u32>(ext_base.max(1))?;
    let counts = super::blocks::zeroed::<u32>(ext_base.max(1))?;
    let positions = super::blocks::zeroed::<u32>(pos_base.max(1))?;
    let tickets = super::blocks::zeroed::<u32>(2)?;
    let done = super::blocks::zeroed::<u32>(total)?;
    let workers = total.min(WORKERS);
    let most = forms.iter().map(|n| n.insts.len()).max().unwrap_or(0);
    let params = [fit(total)?, spins, fit(most)?, 0];
    let mut buffers: Vec<(u64, &metal::BufferRef, u64)> = vec![
        (1, out.buffer(), 0),
        (2, table.buffer(), 0),
        (3, group_file.buffer(), 0),
        (4, states.buffer(), 0),
        (5, halos.buffer(), 0),
        (6, ext_states.buffer(), 0),
        (7, positions.buffer(), 0),
        (8, counts.buffer(), 0),
        (9, tickets.buffer(), 0),
        (10, done.buffer(), 0),
    ];
    // The slabs, each bound as it is (no copy); unused bindings repeat the first.
    for (k, &at) in SLAB_BUFFERS.iter().enumerate() {
        buffers.push((at, &slabs.get(k).unwrap_or(&slabs[0]).buffer.buffer, 0));
    }
    let interpreter;
    let pipeline = match executor {
        Executor::Interpreter => {
            let (mut steps, mut table, mut outs) = (Vec::new(), Vec::new(), Vec::new());
            for n in &forms {
                let mut carry = 0u32;
                let step0 = fit(steps.len())?;
                for inst in &n.insts {
                    steps.push(encoded(inst, carry));
                    carry += u32::from(inst.carried());
                }
                let out0 = fit(outs.len())?;
                outs.extend(n.outputs.iter().map(|&v| u32::from(v)));
                let ext0 = fit(outs.len())?;
                for &k in &n.extracts {
                    outs.push(fit(k)?);
                }
                table.push(ProgramRow {
                    step0,
                    steps: fit(n.insts.len())?,
                    out0,
                    outs: fit(n.outputs.len())?,
                    ext0,
                    exts: fit(n.extracts.len())?,
                    pad: [0; 2],
                });
            }
            steps.push([0; 4]);
            outs.push(0);
            interpreter = [
                super::blocks::upload(&table)?,
                super::blocks::upload(&steps)?,
                super::blocks::upload(&outs)?,
                // Each worker's values: a slot of 32 lane words per step (decision 0034).
                super::blocks::zeroed::<u8>((workers * most * GROUP * word).max(1))?,
            ];
            for (k, block) in interpreter.iter().enumerate() {
                buffers.push((12 + k as u64, block.buffer(), 0));
            }
            pipeline_named(&INTERPRETER, &format!("block_batch_{}", width.bytes()))?
        }
        Executor::Generated(_) => {
            let refs: Vec<&BlockNative> = forms.iter().collect();
            let source = generate_forms(&refs, width);
            pipeline_named(&source, "tempo_batch").inspect_err(|e| eprintln!("block program: pipeline tempo_batch: {e:?}"))?
        }
    };
    let pass = Pass { pipeline: &pipeline, buffers, args: vec![(11, bytes_of(&params))], groups: workers, threads: GROUP };
    // Each launch finishes at least the first unfinished group, whose predecessors are all done.
    let (mut seconds, mut launches) = (0.0, 0);
    loop {
        if launches == total {
            return Err(Error::BackendFailure);
        }
        if launches > 0 {
            REPEATED_LAUNCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // SAFETY: two words, written between launches, when no kernel runs.
            unsafe { std::ptr::write_bytes(tickets.host().cast::<u32>(), 0, 2) };
        }
        seconds += encode_passes(std::slice::from_ref(&pass))?;
        launches += 1;
        if tickets.read::<u32>(2)[1] == 0 {
            break;
        }
    }
    let lanes_bytes = out.read::<u8>(out_base * word);
    let counts = counts.read::<u32>(ext_base);
    let positions = positions.read::<u32>(pos_base);
    let mut results = Vec::with_capacity(files.len());
    for row in &rows {
        let n = &forms[row.program as usize];
        let (len, groups) = (row.len as usize, row.groups as usize);
        let lanes = groups * GROUP;
        let words = (len + usize::from(n.delay)).div_ceil(64);
        let outputs = (0..n.outputs.len())
            .map(|k| {
                let at = (row.out_base as usize + k * lanes) * word;
                let w: Vec<u64> = lanes_bytes[at..at + lanes * word].chunks(8).take(words).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
                undelay(&w, n.output_delays[k], len)
            })
            .collect();
        let capacity = len.max(1);
        let positions = (0..n.extracts.len())
            .map(|e| {
                let at = row.ext_base as usize + e * groups;
                let count: usize = counts[at..at + groups].iter().map(|&c| c as usize).sum();
                let from = row.pos_base as usize + e * capacity;
                positions[from..from + count.min(capacity)].to_vec()
            })
            .collect();
        results.push(BlockRun { outputs, positions });
    }
    Ok(BatchRun { files: results, launches, seconds })
}

/// The interpreter: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn interpret(
    program: &super::block::BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    interpret_extract(program, bytes, width).map(|run| run.outputs)
}

/// The interpreter with the extract pass: a batch of one file.
pub(in crate::code_rule_dsl) fn interpret_extract(
    program: &super::block::BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<BlockRun, Error> {
    let native = BlockNative::lower(program);
    let mut run = run_batch(&[&native], &[BatchFile { bytes, program: 0 }], width, Executor::Interpreter, SPINS)?;
    Ok(run.files.remove(0))
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

/// The native IR as generated MSL for lanes of `width`, `class` by bit-planes.
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
fn lookup_helper(p: usize, t: usize, table: &super::block::ClassTable) -> String {
    let hex = |xs: &[u8]| xs.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
    let mut out = format!(
        "constant uchar T{p}_{t}[32] = {{{}, {}}};\nstatic inline uint4 classes_p{p}_t{t}(uint4 v) {{\n  uint4 k;\n",
        hex(&table.lo),
        hex(&table.hi)
    );
    for lane in ["x", "y", "z", "w"] {
        let bytes: Vec<String> = (0..4)
            .map(|j| {
                let b = format!("uint(T{p}_{t}[(v.{lane} >> {}u) & 15u] & T{p}_{t}[16u + ((v.{lane} >> {}u) & 15u)])", 8 * j, 8 * j + 4);
                if j == 0 { b } else { format!("({b} << {}u)", 8 * j) }
            })
            .collect();
        out.push_str(&format!("  k.{lane} = {};\n", bytes.join(" | ")));
    }
    out.push_str("  return k;\n}\n");
    out
}

/// The native IR as MSL for lanes of `width`, `class` by `strategy`: a batch's kernel for one
/// program.
pub(in crate::code_rule_dsl) fn generate_with(native: &BlockNative, width: LaneWidth, strategy: ClassStrategy) -> String {
    let form = match strategy {
        ClassStrategy::Lookup => native.clone(),
        ClassStrategy::Planes => native.planes_form(),
    };
    generate_forms(&[&form], width)
}

/// A group of programs (each already in the form it runs) as one kernel, `tempo_batch`: a function
/// per program, `program_P`, that computes each instruction once, in order, with a look-back for
/// each carried one, and writes the file's outputs and extracts; and persistent workers that take
/// the batch's groups in order and call their file's program (decision 0044).
fn generate_forms(programs: &[&BlockNative], width: LaneWidth) -> String {
    let m = width.word();
    let mut helpers = String::new();
    if programs.iter().any(|n| n.insts.iter().any(|i| matches!(i, BlockInst::Class { .. } | BlockInst::Pair { .. }))) {
        helpers.push_str(NZ16);
    }
    if programs.iter().any(|n| n.insts.iter().any(|i| matches!(i, BlockInst::Plane(_) | BlockInst::PlaneBefore(_)))) {
        helpers.push_str(TRANSPOSE8);
    }
    let mut out = format!("{CARRY}\n{}\n{EQ16}\n{helpers}\n", batch_helpers());
    for (p, native) in programs.iter().enumerate() {
        if native.insts.iter().any(|i| matches!(i, BlockInst::Class { .. } | BlockInst::Pair { .. })) {
            for (t, table) in native.tables.iter().enumerate() {
                out.push_str(&lookup_helper(p, t, table));
            }
        }
        let plan = Plan::new(native);
        if !plan.windowed.is_empty() {
            out.push_str(&halo_window(p, native, &plan));
        }
        out.push_str(&program_function(p, native, &plan, width));
    }
    let cases: String = (0..programs.len())
        .map(|p| {
            format!(
                "      case {p}u: ok = program_{p}(bytes, f, t - f.group0, l, p.y, out, states, halos, ext_states, positions, counts); break;\n"
            )
        })
        .collect();
    out.push_str(&format!(
        "// Persistent workers (decision 0027): each takes group numbers in launch order and runs its\n\
         // file's program over the group. `p`: the batch's groups, the polls after the first.\n\
         kernel void tempo_batch(\n\
         \x20   device const uint4* s0 [[buffer(0)]], device {m}* out [[buffer(1)]], device const FileRow* files [[buffer(2)]],\n\
         \x20   device const uint* group_file [[buffer(3)]], device atomic_uint* states [[buffer(4)]],\n\
         \x20   device atomic_uint* halos [[buffer(5)]], device atomic_uint* ext_states [[buffer(6)]],\n\
         \x20   device uint* positions [[buffer(7)]], device uint* counts [[buffer(8)]], device atomic_uint* tickets [[buffer(9)]],\n\
         \x20   device uint* done [[buffer(10)]], constant uint4& p [[buffer(11)]], device const uint4* s1 [[buffer(16)]],\n\
         \x20   device const uint4* s2 [[buffer(17)]], device const uint4* s3 [[buffer(18)]], uint l [[thread_index_in_simdgroup]]) {{\n\
         \x20 for (uint round = 0u; round < p.x; round++) {{\n\
         \x20   uint ticket = 0u;\n\
         \x20   if (l == 0u) ticket = atomic_fetch_add_explicit(&tickets[0], 1u, memory_order_relaxed);\n\
         \x20   const uint t = simd_broadcast(ticket, ushort(0));\n\
         \x20   if (t >= p.x) return;\n\
         \x20   if (done[t] != 0u) continue;\n\
         \x20   const FileRow f = files[group_file[t]];\n\
         \x20   // The slab the file's bytes are in (decision 0045): the same in every lane.\n\
         \x20   device const uint4* bytes = f.slab == 0u ? s0 : f.slab == 1u ? s1 : f.slab == 2u ? s2 : s3;\n\
         \x20   bool ok = false;\n\
         \x20   // The same program in every lane: the group holds one file.\n\
         \x20   switch (f.program) {{\n{cases}\
         \x20     default: break;\n\
         \x20   }}\n\
         \x20   if (l == 0u && ok) done[t] = 1u;\n\
         \x20   if (l == 0u && !ok) atomic_fetch_add_explicit(&tickets[1], 1u, memory_order_relaxed);\n\
         \x20 }}\n\
         }}\n"
    ));
    out
}

/// The helpers both executors share: the file table's row, reading a file's bytes, and the
/// look-backs.
fn batch_helpers() -> String {
    format!("{FILE_ROW}\n{}{LOOK_BACK}{HALO_EXCHANGE}{EXTRACT}\n", table_helpers(1))
}

/// A file of a batch, and its bytes read in place in its slab: 16-byte chunks from `src16`, zero
/// at and past its length (a neighbour's bytes never read as its own).
const FILE_ROW: &str = "// A file of the batch (decision 0044). Lane words are indexed in the file: output k of lane gl at
// out_base + k * groups * 32 + gl; carry j of group g at state_base + j * groups + g (a halo's
// three words from halo_base + (j * groups + g) * 3); extract e's state and count of group g at
// ext_base + e * groups + g, its positions from pos_base + e * max(len, 1).
struct FileRow { uint src16, len, program, group0, groups, out_base, state_base, halo_base, ext_base, pos_base, slab, pad; };
static inline uint4 load16(device const uint4* bytes, FileRow f, uint c) {
  const uint chunks = max((f.len + 15u) / 16u, 1u);
  const uint4 v = bytes[f.src16 + min(c, chunks - 1u)];
  const int have = clamp(int(f.len) - int(c * 16u), 0, 16);
  const uint4 keep = uint4(clamp(int4(have) - int4(0, 4, 8, 12), 0, 4));
  const uint4 low = (uint4(1u) << ((keep * 8u) & 31u)) - 1u;
  return v & select(low, uint4(0xffffffffu), keep == uint4(4u));
}
// The byte before chunk c of a file (0x00 before its first byte).
static inline uint file_byte_before(device const uint4* bytes, FileRow f, uint c) {
  return select(0u, load16(bytes, f, max(c, 1u) - 1u).w >> 24u, c > 0u);
}
";

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

/// Program `p` as a function over group `g` (lane `l`) of file `f`: every instruction in order,
/// each once, then the outputs and the extracts. A look-back or a halo that is not published yet
/// returns false: the group is unfinished.
fn program_function(p: usize, native: &BlockNative, plan: &Plan, width: LaneWidth) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let w = width.bytes();
    let mut out = format!(
        "static inline bool program_{p}(device const uint4* bytes, FileRow f, uint g, uint l, uint spins, device {m}* out,\n\
         \x20                              device atomic_uint* states, device atomic_uint* halos, device atomic_uint* ext_states,\n\
         \x20                              device uint* positions, device uint* counts) {{\n\
         \x20 const uint gl = g * {GROUP}u + l;\n\
         \x20 const uint lanes = f.groups * {GROUP}u;\n\
         \x20 const uint len = f.len;\n"
    );
    if !plan.windowed.is_empty() {
        let mut ops: Vec<usize> = plan.windowed.iter().flat_map(|&at| operands(&native.insts[at])).collect();
        ops.sort_unstable();
        ops.dedup();
        for a in &ops {
            out.push_str(&format!("  ulong w{a} = 0ul;\n"));
        }
        let wargs: String = ops.iter().map(|a| format!(", w{a}")).collect();
        out.push_str(&format!("  halo_window_{p}(bytes, f, int(gl * {w}u) - 64{wargs});\n"));
    }
    let all: Vec<usize> = (0..native.insts.len()).collect();
    out.push_str(&emit(p, native, width, &all, &format!("gl * {chunks}u"), &format!("gl * {w}u"), |at, inst, out| {
        // A scanned carry: the lane's results for carry-ins 0 and 1, its table, the look-back.
        let scanned = |out: &mut String, j: usize, f: &str| {
            out.push_str(&format!(
                "  uint o{at}_0 = 0u, o{at}_1 = 0u, cin{at} = 0u;\n\
                 \x20 const {m} f{at}_0 = {}, f{at}_1 = {};\n\
                 \x20 if (!look_back(states + f.state_base + {j}u * f.groups, g, l, (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u), spins, cin{at})) return false;\n\
                 \x20 const {m} v{at} = select(f{at}_0, f{at}_1, cin{at} != 0u);\n",
                f.replace("CIN", &format!("{m}(0)")).replace("OUT", &format!("o{at}_0")),
                f.replace("CIN", &format!("{m}(1)")).replace("OUT", &format!("o{at}_1")),
            ));
        };
        match (plan.placement(at), *inst) {
            (Placement::Windowed, BlockInst::Prev(a, k)) => out.push_str(&format!("  const {m} v{at} = apply_prev(v{a}, {k}u, w{a});\n")),
            (Placement::Halo(j), BlockInst::Prev(a, k)) => out.push_str(&format!(
                "  ulong h{at} = 0ul;\n\
                 \x20 if (!halo_exchange(halos + f.halo_base + {j}u * f.groups * 3u, g, l, halo_last(v{a}), spins, h{at})) return false;\n\
                 \x20 const {m} v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l, h{at}));\n"
            )),
            (Placement::Scanned(j), BlockInst::Toggle(a)) => out.push_str(&format!(
                "  const uint s{at} = summary_toggle(v{a});\n\
                 \x20 uint cin{at} = 0u;\n\
                 \x20 if (!look_back(states + f.state_base + {j}u * f.groups, g, l, s{at} | ((1u ^ s{at}) << 1u), spins, cin{at})) return false;\n\
                 \x20 const {m} v{at} = apply_toggle(v{a}, cin{at});\n"
            )),
            (Placement::Scanned(j), BlockInst::AfterOddRun(a)) => scanned(out, j, &format!("after_odd_run(v{a}, CIN, OUT)")),
            (Placement::Scanned(j), BlockInst::Fill(a, e)) => scanned(out, j, &format!("fill_word(v{a}, v{e}, CIN, OUT)")),
            _ => unreachable!("placement"),
        }
    }));
    for (k, v) in native.outputs.iter().enumerate() {
        out.push_str(&format!("  out[f.out_base + {k}u * lanes + gl] = v{v};\n"));
    }
    // The extract pass: each extract's positions, at offsets found by look-back
    // (`extract_write`). It polls longer than the carries do (still bounded).
    for (e, &k) in native.extracts.iter().enumerate() {
        out.push_str(&format!(
            "  if (!extract_write(v{}, gl, g, l, len, ext_states + f.ext_base + {e}u * f.groups, positions + f.pos_base + {e}u * max(len, 1u), max(len, 1u), counts + f.ext_base + {e}u * f.groups, spins * {EXTRACT_POLLS}u)) return false;\n",
            native.outputs[k]
        ));
    }
    out.push_str("  return true;\n}\n\n");
    out
}

/// The instructions in `chosen`, as straight-line statements for a word of `width` whose first
/// byte is chunk `base` (16-byte chunks) and stream byte `offset` (for `const`'s phase).
/// `carried` writes each carried instruction's statements.
fn emit(
    p: usize,
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
            out.push_str(&format!("  const uint4 c{c} = load16(bytes, f, {base} + {c}u);\n"));
        }
    }
    // The byte before the word, read from the source (no carry); and, for `pair` by lookups,
    // the word's bytes one back.
    if back {
        out.push_str(&format!("  const uint before = file_byte_before(bytes, f, {base});\n"));
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
            out.push_str(&format!("  const uint4 k{t}_{c} = classes_p{p}_t{t}(c{c});\n"));
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
            out.push_str(&format!("  const uint4 kb{t}_{c} = classes_p{p}_t{t}(b{c});\n"));
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
            BlockInst::Within(0) => format!("within_lane<{m}>(len, {offset})"),
            BlockInst::Within(d) => format!("{m}(within_lane<{m}>(len + {d}u, {offset}) & ~within_lane<{m}>({d}u, {offset}))"),
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

/// `halo_window_P`: program `p`'s operands of local `prev`s over the 64 bytes from file byte `base`
/// (a multiple of 16; before the file, zero), as 64-bit words. A lane calls it once, with `base`
/// 64 bytes before its first byte: the words are those operands' halos.
fn halo_window(p: usize, native: &BlockNative, plan: &Plan) -> String {
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
        "// The 64 bytes from `base` (zero before the file): the local prevs' operands.\n\
         static inline void halo_window_{p}(device const uint4* bytes, FileRow f, int base{params}) {{\n\
         \x20 const uint b = uint(max(base, 0));\n\
         \x20 const uint len = f.len;\n"
    );
    out.push_str(&emit(p, native, LaneWidth::W64, &cone(native, &wanted), "b / 16u", "b", |_, _, _| unreachable!("local")));
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
    run_generated_extract(native, bytes, width, strategy).map(|(run, seconds)| (run.outputs, seconds))
}

/// Generated MSL with the extract pass, a batch of one file: the outputs and each extract's
/// positions, and the GPU seconds of its launches.
pub(in crate::code_rule_dsl) fn run_generated_extract(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(BlockRun, f64), Error> {
    let mut run = run_batch(&[native], &[BatchFile { bytes, program: 0 }], width, Executor::Generated(strategy), SPINS)?;
    Ok((run.files.remove(0), run.seconds))
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
    // The one kept queue of device solves (decision 0039).
    let queue = super::blocks::queue(device)?;
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

