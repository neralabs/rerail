//! Block programs on Metal (`block`), both ways the transducer runs on the device, each over a
//! batch of files in one launch (decision 0066):
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
//! Carried operations (`prev`, `toggle`, `escaped`, `fill`) follow `06-block-parallel.md` §2
//! and §5. A lane's carry is combined across its SIMD group by SIMD-group operations, and across a
//! file's groups by decoupled look-back with bounded polls (`13-tape-on-the-gpu.md` §5.3). Groups
//! take their numbers in launch order from persistent workers; a group waits only on earlier groups
//! of its file. A poll that runs out leaves the group unfinished, and the host launches again.
use super::*;
pub(in crate::code_rule_dsl) use crate::code_rule_dsl::transducer::packed::block::BatchFile;
use crate::code_rule_dsl::transducer::packed::block::BlockRun;
use crate::code_rule_dsl::transducer::packed::native_ir::{BlockInst, BlockNative};
use metal::MTLSize;

/// The same programs in the CPU's shape: a thread a segment of blocks, the carry forward by
/// speculation (`metal_block_serial.rs`).
#[path = "metal_block_serial.rs"]
pub(in crate::code_rule_dsl) mod serial;

/// The carries' helpers (`metal_block_carry.metal`), shared by the interpreter and generated MSL.
const CARRY: &str = include_str!("metal_block_carry.metal");
/// The sinks: outputs at their delays, extracts and totals (`metal_block_sinks.metal`), shared
/// likewise.
const SINKS: &str = include_str!("metal_block_sinks.metal");
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
    /// Where the file's counts start (one per counted output: `native_cpu_block::counted`).
    tot_base: u32,
    /// A program in passes (`native_ir::Passes`, the forward carry only): where the file's slot
    /// words start in the output buffer, block `b`'s slot `k` at `slot_base + b * slots + k`.
    slot_base: u32,
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
        BlockInst::Escaped(a) => [10, u32::from(a), carry, 0],
        // Its second operand in `imm`: `b` is the carry's index.
        BlockInst::Fill(a, e) => [12, u32::from(a), carry, u32::from(e)],
        BlockInst::Within(d) => [13, 0, 0, u32::from(d)],
        BlockInst::Carry(_) => unreachable!("refused at `run_batch`: Metal runs no fixed point yet"),
        BlockInst::BackFill(..) | BlockInst::Load(_) => unreachable!("refused at `run_batch`: Metal runs no backward pass yet"),
        // `j` in `a`; below 6 the period's pattern in `b` and `imm`.
        BlockInst::Position(j) => {
            let pattern = (0..64u32).filter(|i| u32::from(j) < 6 && i >> j & 1 == 1).fold(0u64, |a, i| a | 1 << i);
            [16, u32::from(j), pattern as u32, (pattern >> 32) as u32]
        }
        // The period in `a` (`k | r << 8`, never read as a slot: it is clamped), its bits in `b` and `imm`.
        BlockInst::Phase(k, r) => {
            let bits = super::block::period_bits(k);
            [15, u32::from(k) | u32::from(r) << 8, bits as u32, (bits >> 32) as u32]
        }
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

/// Workers of the persistent dispatch: SIMD groups that take the batch's tickets in order. Every
/// worker must be resident: a worker waiting on a ticket held by one that is not polls until its
/// budget runs out and the launch is repeated. On an M5 Max (40 GPU cores) JSON's tier 1 ran
/// fastest at 768 (step 15) and 1,024 (step 14), and fell off a cliff past 1,024 (doc 22).
const WORKERS: usize = 768;

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

/// Run `files` in one launch, each under `programs[file.program]` (decision 0066). `spins` is the
/// polls of a predecessor after the first; 0 tests the repeated launch.
pub(in crate::code_rule_dsl) fn run_batch(
    programs: &[&BlockNative],
    files: &[BatchFile<'_>],
    width: LaneWidth,
    executor: Executor,
    spins: u32,
) -> Result<BatchRun, Error> {
    // A fixed point (`rounds`) is the CPU's for now: each lane would run its rounds in its own
    // frame, within `block::MAX_ROUNDS`, to the same values.
    // A `back_fill` likewise: a backward pass over the groups' summaries, not generated yet.
    if programs.iter().any(|p| !p.groups.is_empty() || p.insts.iter().any(|i| matches!(i, BlockInst::BackFill(..) | BlockInst::Load(_)))) {
        return Err(Error::UnsupportedBackend);
    }
    METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let word = width.bytes() / 8;
    let span = width.bytes() * GROUP;
    // The form each executor runs: `class` by lookups only in generated MSL's `Lookup`.
    let forms: Vec<BlockNative> = programs
        .iter()
        .map(|p| if executor == Executor::Generated(ClassStrategy::Lookup) { (*p).clone() } else { p.planes_form() })
        .collect();
    let carried: Vec<usize> = forms.iter().map(|n| n.insts.iter().filter(|i| i.carried()).count()).collect();
    let records: Vec<usize> = match executor {
        Executor::Interpreter => carried.clone(),
        Executor::Generated(_) => forms.iter().map(record_words).collect(),
    };
    let fit = |n: usize| u32::try_from(n).map_err(|_| Error::BackendCapacity);
    // The files' bytes, resident in slabs (decision 0067): read in place, at most `MAX_SLABS`
    // slabs a batch; past that the caller splits the batch.
    let bytes: Vec<&[u8]> = files.iter().map(|f| f.bytes).collect();
    let sources = crate::code_rule_dsl::residency::slabs::admit_sources(&bytes)?;
    let (slabs, slab_of) = crate::code_rule_dsl::residency::slabs::source_slabs(&sources).ok_or(Error::BackendCapacity)?;
    if files.is_empty() {
        return Ok(BatchRun { files: Vec::new(), launches: 0, seconds: 0.0 });
    }
    // The file table, and each base after the files before.
    let (mut group0, mut out_base, mut state_base, mut halo_base, mut ext_base, mut pos_base, mut tot_base) = (0, 0, 0, 0, 0, 0, 0);
    let counted: Vec<Vec<usize>> = forms.iter().map(super::native_cpu_block::counted).collect();
    let mut rows = Vec::with_capacity(files.len());
    let mut group_file: Vec<u32> = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let n = forms.get(file.program).ok_or(Error::BackendFailure)?;
        let len = file.bytes.len();
        // Generated MSL's groups each start with `warm_lanes` lanes whose sinks write nothing.
        let span = match executor {
            Executor::Interpreter => span,
            Executor::Generated(_) => width.bytes() * (GROUP - warm_lanes(n, width)),
        };
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
            tot_base: fit(tot_base)?,
            slot_base: 0,
        });
        group_file.extend(std::iter::repeat_n(fit(i)?, groups));
        group0 += groups;
        out_base += n.outputs.len() * groups * GROUP;
        // The interpreter's look-back states, a word per carried instruction; generated MSL's
        // exchange records (`record_words`).
        state_base += records[file.program] * groups;
        halo_base += carried[file.program] * groups * 3;
        ext_base += counted[file.program].len() * groups;
        pos_base += n.extracts.len() * len.max(1);
        tot_base += counted[file.program].len();
    }
    let total = group0;
    let table = super::blocks::upload(&rows)?;
    // Generated MSL takes the groups round robin over the files (`ticket_order`): a (file, group)
    // per ticket. The interpreter takes them file by file.
    let group_file = match executor {
        Executor::Interpreter => super::blocks::upload(&group_file)?,
        Executor::Generated(_) => super::blocks::upload(&ticket_order(&rows.iter().map(|r| r.groups as usize).collect::<Vec<_>>()))?,
    };
    let out = super::blocks::zeroed::<u8>(out_base.max(1) * word)?;
    let states = super::blocks::zeroed::<u32>(state_base.max(1))?;
    let halos = super::blocks::zeroed::<u32>(halo_base.max(1))?;
    let ext_states = super::blocks::zeroed::<u32>(ext_base.max(1))?;
    let counts = super::blocks::zeroed::<u32>(tot_base.max(1))?;
    let positions = super::blocks::zeroed::<u32>(pos_base.max(1))?;
    let tickets = super::blocks::zeroed::<u32>(2)?;
    let done = super::blocks::zeroed::<u32>(total)?;
    let workers = workers(total);
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
            for (pi, n) in forms.iter().enumerate() {
                let mut carry = 0u32;
                let step0 = fit(steps.len())?;
                for inst in &n.insts {
                    steps.push(encoded(inst, carry));
                    carry += u32::from(inst.carried());
                }
                let out0 = fit(outs.len())?;
                // Each output: its value, and its delay in the top byte.
                outs.extend(n.outputs.iter().enumerate().map(|(k, &v)| u32::from(v) | u32::from(n.output_delays[k]) << 24));
                let ext0 = fit(outs.len())?;
                // Each counted output: its index, its extract (0xff: a total only) and its delay.
                for (c, &k) in counted[pi].iter().enumerate() {
                    let e = if c < n.extracts.len() { c as u32 } else { 0xff };
                    outs.push(fit(k)? | e << 16 | u32::from(n.output_delays[k]) << 24);
                }
                table.push(ProgramRow {
                    step0,
                    steps: fit(n.insts.len())?,
                    out0,
                    outs: fit(n.outputs.len())?,
                    ext0,
                    exts: fit(super::native_cpu_block::counted(n).len())?,
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
    let counts = counts.read::<u32>(tot_base);
    let positions = positions.read::<u32>(pos_base);
    let mut results = Vec::with_capacity(files.len());
    for row in &rows {
        let n = &forms[row.program as usize];
        let (len, groups) = (row.len as usize, row.groups as usize);
        let lanes = groups * GROUP;
        // The kernel wrote the stream's words, at their delays and cut to the stream: read back.
        let words = len.div_ceil(64);
        let outputs: Vec<Vec<u64>> = (0..n.outputs.len())
            .map(|k| {
                let at = (row.out_base as usize + k * lanes) * word;
                lanes_bytes[at..at + lanes * word].chunks(8).take(words).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
            })
            .collect();
        let capacity = len.max(1);
        let count = |c: usize| counts[row.tot_base as usize + c];
        let positions = (0..n.extracts.len())
            .map(|e| {
                let from = row.pos_base as usize + e * capacity;
                positions[from..from + (count(e) as usize).min(capacity)].to_vec()
            })
            .collect();
        let totals = (0..n.totals.len()).map(|t| u64::from(count(n.extracts.len() + t))).collect();
        let _ = groups;
        results.push(BlockRun { outputs, positions, totals });
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
/// the batch's groups in order and call their file's program (decision 0066).
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
        let plan = Plan::new(native).at(native, width);
        out.push_str(&program_function(p, native, &plan, width));
    }
    let cases: String = (0..programs.len())
        .map(|p| {
            format!(
                "      case {p}u: ok = program_{p}(bytes, f, fg.y, l, p.y, out, states, halos, ext_states, positions, counts); break;\n"
            )
        })
        .collect();
    out.push_str(&format!(
        "// Persistent workers (decision 0027): each takes group numbers in launch order and runs its\n\
         // file's program over the group. `p`: the batch's groups, the polls after the first.\n\
         kernel void tempo_batch(\n\
         \x20   device const uint4* s0 [[buffer(0)]], device {m}* out [[buffer(1)]], device const FileRow* files [[buffer(2)]],\n\
         \x20   device const uint2* group_of [[buffer(3)]], device atomic_uint* states [[buffer(4)]],\n\
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
         \x20   const uint2 fg = group_of[t];\n\
         \x20   const FileRow f = files[fg.x];\n\
         \x20   // The slab the file's bytes are in (decision 0067): the same in every lane.\n\
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

/// Generated MSL's order of the groups of files of `groups` groups each: round robin, the first
/// group of every file, then the second of each that has one, and so on, as (file, group) words.
/// A group waits on the one before it in its file, which took its ticket a round earlier. Given
/// in file order, the groups in flight would be mostly a file's next ones, each waiting on the
/// one before.
fn ticket_order(groups: &[usize]) -> Vec<[u32; 2]> {
    let most = groups.iter().copied().max().unwrap_or(0);
    let mut order = Vec::with_capacity(groups.iter().sum());
    for g in 0..most {
        for (file, &n) in groups.iter().enumerate() {
            if g < n {
                order.push([file as u32, g as u32]);
            }
        }
    }
    order
}

/// The helpers both executors share: the file table's row, reading a file's bytes, and the
/// look-backs.
fn batch_helpers() -> String {
    format!("{FILE_ROW}\n{}{LOOK_BACK}{HALO_EXCHANGE}{EXCHANGE_WORD}{SINKS}\n", table_helpers(1))
}

/// A file of a batch, and its bytes read in place in its slab: 16-byte chunks from `src16`, zero
/// at and past its length (a neighbour's bytes never read as its own).
const FILE_ROW: &str = "// A file of the batch (decision 0066). Lane words are indexed in the file: output k of lane gl at
// out_base + k * groups * 32 + gl; carry j of group g at state_base + j * groups + g (a halo's
// three words from halo_base + (j * groups + g) * 3); counted output c's state of group g at
// ext_base + c * groups + g, and its total at tot_base + c; extract e's positions from
// pos_base + e * max(len, 1); in passes, block b's slot k at slot_base + b * SLOTS + k.
struct FileRow { uint src16, len, program, group0, groups, out_base, state_base, halo_base, ext_base, pos_base, slab, tot_base, slot_base; };
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

/// Polls of a predecessor's state after the first: bounded, so a launch whose predecessors never
/// publish ends and is repeated. 64 polls ran out on full batches: the launch was repeated while
/// the predecessor was only slow.
pub(in crate::code_rule_dsl) const SPINS: u32 = 65536;
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

/// One word of an exchange (`Plan::exchanges`), by the lane that owns it; a group's record has
/// `stride` words, and `rec` is this word's place in group 0's. Kind 0: a halo word, published
/// with a flag and the group before's read once it is; kind 3 the same, published only (its value
/// is known). Kind 1: a scan's table, and kind 2: a
/// count, each published as an aggregate and read back to an inclusive prefix by `look_back`'s
/// protocol. `got`: the halo word, or the prefix before the group. False when a predecessor has
/// published nothing after `spins` polls; a lane past the exchange's words (`owns` false) has
/// nothing to do. Every lane of a group polls its own word at once, so an
/// exchange of many values waits about as long as one.
///
/// A table that gives the same carry for either carry in has one state: a group whose own table
/// is constant publishes it as its inclusive prefix at once, and a look-back stops at the first
/// constant prefix, since nothing before it changes the carry.
const EXCHANGE_WORD: &str = "static inline bool exchange_word(device atomic_uint* rec, uint g, uint stride, uint kind, uint mine, uint spins, bool owns, thread uint& got) {
  got = 0u;
  if (!owns) return true;
  if (kind == 3u) {
    atomic_store_explicit(&rec[g * stride], HALO_FLAG | mine, memory_order_relaxed);
    return true;
  }
  if (kind == 0u) {
    atomic_store_explicit(&rec[g * stride], HALO_FLAG | mine, memory_order_relaxed);
    if (g == 0u) return true;
    uint x = 0u;
    for (uint spin = 0u; spin <= spins && (x & HALO_FLAG) == 0u; spin++) {
      x = atomic_load_explicit(&rec[(g - 1u) * stride], memory_order_relaxed);
    }
    got = x & 0x3ffffffful;
    return (x & HALO_FLAG) != 0u;
  }
  const bool single = kind == 1u && tapply1(mine, 0u) == tapply1(mine, 1u);
  atomic_store_explicit(&rec[g * stride], select(CARRY_AGGREGATE, CARRY_INCLUSIVE, single) | mine, memory_order_relaxed);
  uint prefix = select(0u, TID1, kind == 1u);
  for (uint back = 1u; back <= g; back++) {
    uint st = 0u;
    for (uint spin = 0u; spin <= spins && (st >> 30u) == 0u; spin++) {
      st = atomic_load_explicit(&rec[(g - back) * stride], memory_order_relaxed);
    }
    if ((st >> 30u) == 0u) return false;
    prefix = select(prefix + (st & CARRY_VALUE), tcompose1(st & CARRY_VALUE, prefix), kind == 1u);
    if ((st >> 30u) == 2u || (kind == 1u && tapply1(prefix, 0u) == tapply1(prefix, 1u))) break;
  }
  atomic_store_explicit(&rec[g * stride], CARRY_INCLUSIVE | select(prefix + mine, tcompose1(prefix, mine), kind == 1u), memory_order_relaxed);
  got = prefix;
  return true;
}

";

/// How generated MSL treats each carried instruction.
struct Plan {
    /// The local `prev`s: of a value of the bytes alone (local `prev`s of such values included);
    /// their halo comes from the lanes before, by shuffles, the group's warm-up lanes included
    /// (`warm_lanes`).
    windowed: Vec<usize>,
    /// The scanned carries (`toggle`, `escaped`, `fill`), in order: a state per group each.
    scanned: Vec<usize>,
    /// The other `prev`s, in order: a published halo per group each.
    halos: Vec<usize>,
    /// The warm-up lanes of each group at the lane width (`warm_lanes`); 0 until `at`.
    warm: usize,
    /// Each local value's reach: the bytes before it that it reads, through its local `prev`s.
    reach: Vec<usize>,
    /// Each instruction's level: the exchanges between groups it waits on. A carried
    /// instruction (not windowed) of level `L + 1` is resolved in exchange `L`, with every other
    /// one whose operands are of level `L` at most.
    level: Vec<usize>,
}

impl Plan {
    fn new(native: &BlockNative) -> Self {
        let n = native.insts.len();
        let mut local = vec![true; n];
        let mut plan = Plan { windowed: Vec::new(), scanned: Vec::new(), halos: Vec::new(), warm: 0, reach: vec![0; n], level: vec![0; n] };
        for at in 0..n {
            let ops = operands(&native.insts[at]);
            let ops_local = ops.iter().all(|&a| local[a]);
            let reach = ops.iter().map(|&a| plan.reach[a]).max().unwrap_or(0);
            match native.insts[at] {
                // A `prev` of a local value is local however deep: the warm-up lanes cover its reach.
                BlockInst::Prev(_, k) if ops_local => {
                    plan.windowed.push(at);
                    plan.reach[at] = reach + usize::from(k);
                }
                BlockInst::Prev(..) => {
                    local[at] = false;
                    plan.halos.push(at);
                }
                ref inst if inst.carried() => {
                    local[at] = false;
                    plan.scanned.push(at);
                }
                _ => {
                    local[at] = ops_local;
                    plan.reach[at] = reach;
                }
            }
            let below = ops.iter().map(|&a| plan.level[a]).max().unwrap_or(0);
            let exchanged = native.insts[at].carried() && !plan.windowed.contains(&at);
            plan.level[at] = below + usize::from(exchanged);
        }
        plan
    }

    /// The plan at lane width `width`: with its warm-up lanes.
    fn at(mut self, native: &BlockNative, width: LaneWidth) -> Self {
        let _ = native;
        self.warm = self.windowed.iter().map(|&at| self.reach[at].div_ceil(width.bytes())).max().unwrap_or(0);
        self
    }

    /// The exchanges, in order: each one's halos and scans (instructions it resolves). Every
    /// level above 0 has one: a level is only reached through an exchange.
    fn exchanges(&self) -> Vec<(Vec<usize>, Vec<usize>)> {
        let most = self.level.iter().copied().max().unwrap_or(0);
        (1..=most)
            .map(|e| {
                let at = |list: &[usize]| list.iter().copied().filter(|&x| self.level[x] == e).collect::<Vec<_>>();
                (at(&self.halos), at(&self.scanned))
            })
            .collect()
    }

    /// Words per group of the exchanges' records: each exchange's halo words (`halo_words`), one
    /// per scan, and one per counted output for the counts' exchange.
    fn record_words(&self, native: &BlockNative) -> usize {
        // The extracts' counts; a total is a flag, set without an exchange (`totals_or`).
        let halos: usize = self.exchanges().iter().map(|(h, _)| halo_words(native, h).len()).sum();
        halos + self.scanned.len() + native.extracts.len()
    }
}

/// Generated MSL's kernel for `native` at `width`, as the device compiled it: its exchanges (each
/// one's halos and scans), its record, the lines of MSL, and the pipeline's limits. Metal reports
/// no register count; a pipeline whose threads use many registers allows fewer than 1,024
/// threads a threadgroup.
pub(in crate::code_rule_dsl) fn kernel_report(native: &BlockNative, width: LaneWidth) -> Result<String, Error> {
    let form = native.planes_form();
    let plan = Plan::new(&form).at(&form, width);
    let exchanges: Vec<String> = plan.exchanges().iter().map(|(h, s)| format!("{}h+{}s", h.len(), s.len())).collect();
    let source = generate_forms(&[&form], width);
    let pipeline = pipeline_named(&source, "tempo_batch")?;
    Ok(format!(
        "{} instructions; {} local prevs ({} warm-up lanes a group), {} halos, {} scans; exchanges [{}] + counts {}, totals {}; record {} words a group; MSL {} lines; pipeline: {} threads a threadgroup at most, SIMD width {}, {} bytes of threadgroup memory; launched as {} threadgroups of {GROUP}",
        form.insts.len(),
        plan.windowed.len(),
        plan.warm,
        plan.halos.len(),
        plan.scanned.len(),
        exchanges.join(", "),
        form.extracts.len(),
        form.totals.len(),
        plan.record_words(&form),
        source.lines().count(),
        pipeline.max_total_threads_per_threadgroup(),
        pipeline.thread_execution_width(),
        pipeline.static_threadgroup_memory_length(),
        workers(usize::MAX),
    ))
}

/// The lanes each group of generated MSL spends warming up: enough lanes of `width` bytes to
/// hold the farthest local `prev`'s look-back, whose bytes the shuffles of the group's first
/// useful lane read. A group's useful lanes start after them, so that groups overlap by these
/// lanes and no lane recomputes the bytes before it.
fn warm_lanes(native: &BlockNative, width: LaneWidth) -> usize {
    Plan::new(native).at(native, width).warm
}

/// The persistent workers of a launch over `groups` groups: `RERAIL_WORKERS`, or `WORKERS`.
fn workers(groups: usize) -> usize {
    groups.min(std::env::var("RERAIL_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or(WORKERS))
}

/// The words per group generated MSL keeps of a program's exchanges (`Plan::exchanges`).
pub(in crate::code_rule_dsl) fn record_words(native: &BlockNative) -> usize {
    Plan::new(native).record_words(native)
}

/// A piece of a halo in an exchange word: bits `lo..lo + len` of halo `at`'s newest k bits, at
/// bit `first` of the word.
#[derive(Clone, Copy)]
struct Piece {
    at: usize,
    first: u32,
    lo: u32,
    len: u32,
}

/// An exchange's halos packed in words: a `prev` by k reads only the newest k bits of the group
/// before, so each halo takes k bits of 30-bit words (the flag is above them), in order, split
/// where a word fills. The same in every body: a group reads the words of the group before by
/// place, whatever body that group ran.
fn halo_words(native: &BlockNative, halos: &[usize]) -> Vec<Vec<Piece>> {
    let mut words: Vec<Vec<Piece>> = Vec::new();
    let mut used = 30u32;
    for &at in halos {
        let BlockInst::Prev(_, k) = native.insts[at] else { unreachable!("a halo is a prev") };
        let mut lo = 0u32;
        while lo < u32::from(k) {
            if used == 30 {
                words.push(Vec::new());
                used = 0;
            }
            let len = (u32::from(k) - lo).min(30 - used);
            words.last_mut().expect("a word").push(Piece { at, first: used, lo, len });
            used += len;
            lo += len;
        }
    }
    words
}

fn operands(inst: &BlockInst) -> Vec<usize> {
    inst.operands()
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
    let w = width.bytes();
    let mut out = format!(
        "static inline bool program_{p}(device const uint4* bytes, FileRow f, uint g, uint l, uint spins, device {m}* out,\n\
         \x20                              device atomic_uint* states, device atomic_uint* halos, device atomic_uint* ext_states,\n\
         \x20                              device uint* positions, device uint* counts) {{\n\
         \x20 // The group's first {h} lanes warm up (`warm_lanes`): they make the bytes before its\n\
         \x20 // first useful lane, for the shuffles of local `prev`s, and their sinks write nothing.\n\
         \x20 const int glw = int(g * {useful}u + l) - {h};\n\
         \x20 const uint gl = uint(max(glw, 0));\n\
         \x20 const bool keep = l >= {h}u;\n\
         \x20 const bool before_file = glw < 0;\n\
         \x20 const uint lanes = f.groups * {GROUP}u;\n\
         \x20 const uint len = f.len;\n",
        h = plan.warm,
        useful = GROUP - plan.warm,
    );
    // A full group's body (`full_form`): the same in every lane of the group. Then the general one.
    let full = full_form(native);
    let span = w * (GROUP - plan.warm);
    // In a full group, a group of ASCII bytes takes a third body (`ascii_form`): each lane's bytes
    // and the 16 before them below 0x80, the warm-up lanes' included, so every byte a byte-local
    // value reads.
    let ascii = ascii_form(native);
    let chunks = w / 16;
    let loads: Vec<String> = (0..=chunks).map(|c| format!("load16(bytes, f, gl * {chunks}u + {c}u - 1u)")).collect();
    out.push_str(&format!(
        "  if (g > 0u && (g + 1u) * {span}u <= len) {{\n    const uint4 high = {};\n    const bool ascii = ((high.x | high.y | high.z | high.w) & 0x80808080u) == 0u;\n    if (simd_all(ascii)) {{\n{}    return true;\n    }}\n{}  return true;\n  }}\n",
        loads.join(" | "),
        levels(p, native, &ascii, plan, width),
        levels(p, native, &full, plan, width)
    ));
    out.push_str(&levels(p, native, native, plan, width));
    out.push_str("  return true;\n}\n\n");
    out
}

/// An exchange of `words` (MSL expressions, the same in every lane) at word `offset` of the
/// group's record of `record` words: the first `halo_words` are halo words, the rest scan
/// tables. Gives each word's result as an expression read by every lane.
fn exchange(out: &mut String, name: &str, words: &[String], halo_words: usize, offset: usize, record: usize) -> Vec<String> {
    exchange_kinds(out, name, words, |j| if j < halo_words { "0u".to_owned() } else { "1u".to_owned() }, offset, record)
}

/// `exchange` with each word's kind (`exchange_word`): lane `j` of a pass owns word `32 * pass +
/// j`. A group that a predecessor keeps waiting returns unfinished.
fn exchange_kinds(out: &mut String, name: &str, words: &[String], kind: impl Fn(usize) -> String, offset: usize, record: usize) -> Vec<String> {
    let mut got = Vec::with_capacity(words.len());
    for (pass, chunk) in words.chunks(GROUP).enumerate() {
        let base = pass * GROUP;
        let mut mine = "0u".to_owned();
        let mut kinds = "0u".to_owned();
        for (j, w) in chunk.iter().enumerate() {
            mine = format!("select({mine}, {w}, l == {j}u)");
            kinds = format!("select({kinds}, {}, l == {j}u)", kind(base + j));
        }
        out.push_str(&format!(
            "  uint {name}_{pass} = 0u;\n  const bool {name}_{pass}_waits = !exchange_word(states + f.state_base + {}u + l, g, {record}u, {kinds}, {mine}, spins, l < {}u, {name}_{pass});\n  if (simd_any({name}_{pass}_waits)) return false;\n",
            offset + base,
            chunk.len(),
        ));
        for j in 0..chunk.len() {
            got.push(format!("simd_broadcast({name}_{pass}, ushort({j}))"));
        }
    }
    got
}

/// A program's levels and sinks (`program_function`), its values from `values`: the program
/// itself, or its full groups' form (`full_form`), whose exchanges are the same words.
fn levels(p: usize, native: &BlockNative, values: &BlockNative, plan: &Plan, width: LaneWidth) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let w = width.bytes();
    let mut out = String::new();
    // Level by level (`Plan::level`): the instructions of a level, then the exchange that resolves
    // the carried ones of the next; last, the counts' exchange.
    let record = plan.record_words(native);
    let exchanges = plan.exchanges();
    let mut offset = 0usize;
    for level in 0..=exchanges.len() {
        let chosen: Vec<usize> = (0..native.insts.len()).filter(|&at| plan.level[at] == level && (!values.insts[at].carried() || plan.windowed.contains(&at))).collect();
        // A local `prev` takes the bytes before its lane from the lanes before it, by shuffles:
        // the group's warm-up lanes make those before its first useful lane, and a lane before
        // the file gives zeros.
        out.push_str(&emit(p, values, width, &chosen, &format!("gl * {chunks}u"), &format!("gl * {w}u"), |at, inst, out| match *inst {
            BlockInst::Prev(a, k) => out.push_str(&format!("  const {m} v{at} = apply_prev(v{a}, {k}u, halo_before(select(v{a}, {m}(0), before_file), l, 0ul));\n")),
            _ => unreachable!("only windowed prevs are emitted in a level"),
        }));
        let Some((halos, scans)) = exchanges.get(level) else { break };
        // Each value's words, the same in every lane: the halos' newest bits, packed
        // (`halo_words`), then a scan's aggregate table each.
        let packed = halo_words(native, halos);
        let mut words: Vec<String> = Vec::new();
        for &at in halos {
            let BlockInst::Prev(a, k) = native.insts[at] else { unreachable!() };
            out.push_str(&format!("  const ulong last{at} = halo_last(v{a}) >> {}u;\n", 64 - u32::from(k)));
        }
        for word in &packed {
            let parts: Vec<String> = word.iter().map(|p| format!("(uint((last{} >> {}u) & {}ul) << {}u)", p.at, p.lo, (1u64 << p.len) - 1, p.first)).collect();
            words.push(parts.join(" | "));
        }
        let halo_count = packed.len();
        for &at in scans {
            // The lane's table: bit c its carry out for carry in c.
            match native.insts[at] {
                BlockInst::Toggle(a) => out.push_str(&format!("  const uint s{at} = summary_toggle(v{a});\n  const uint tt{at} = s{at} | ((1u ^ s{at}) << 1u);\n")),
                BlockInst::Escaped(a) => out.push_str(&format!(
                    "  uint o{at}_0 = 0u, o{at}_1 = 0u;\n  const {m} f{at}_0 = escaped(v{a}, {m}(0), o{at}_0), f{at}_1 = escaped(v{a}, {m}(1), o{at}_1);\n  const uint tt{at} = (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u);\n"
                )),
                BlockInst::Fill(a, e) => out.push_str(&format!(
                    "  uint o{at}_0 = 0u, o{at}_1 = 0u;\n  const {m} f{at}_0 = fill_word(v{a}, v{e}, {m}(0), o{at}_0), f{at}_1 = fill_word(v{a}, v{e}, {m}(1), o{at}_1);\n  const uint tt{at} = (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u);\n"
                )),
                _ => unreachable!("a scanned instruction"),
            }
            // A warm-up lane's bytes are the group before's: its table is the identity.
            out.push_str(&format!("  uint agg{at} = 0u;\n  const uint e{at} = texclusive1(select(2u, tt{at}, keep), l, agg{at});\n"));
            words.push(format!("agg{at}"));
        }
        // A halo word whose halos are all known in this body is published, for a general group
        // after, and not waited on.
        let known = |j: usize| j < halo_count && packed[j].iter().all(|p| !matches!(values.insts[p.at], BlockInst::Prev(..)));
        let got = exchange_kinds(&mut out, &format!("x{level}"), &words, |j| if known(j) { "3u" } else if j < halo_count { "0u" } else { "1u" }.to_owned(), offset, record);
        for &at in halos {
            let BlockInst::Prev(a, k) = native.insts[at] else { unreachable!() };
            if !matches!(values.insts[at], BlockInst::Prev(..)) {
                // Its value: a constant, emitted with the next level.
                continue;
            }
            // The group before's newest k bits, from their pieces, at the top of the halo.
            let pieces: Vec<String> = packed
                .iter()
                .enumerate()
                .flat_map(|(j, word)| word.iter().filter(|p| p.at == at).map(move |p| (j, *p)))
                .map(|(j, p)| format!("(ulong(({} >> {}u) & {}u) << {}u)", got[j], p.first, (1u64 << p.len) - 1, p.lo))
                .collect();
            out.push_str(&format!(
                "  const ulong h{at} = ({}) << {}u;\n  const {m} v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l - {}u, h{at}));\n",
                pieces.join(" | "),
                64 - u32::from(k),
                plan.warm
            ));
        }
        for (s_, &at) in scans.iter().enumerate() {
            let cin = format!("tapply1(e{at}, tapply1({}, 0u))", got[halo_count + s_]);
            match native.insts[at] {
                BlockInst::Toggle(a) => out.push_str(&format!("  const {m} v{at} = apply_toggle(v{a}, {cin});\n")),
                _ => out.push_str(&format!("  const {m} v{at} = select(f{at}_0, f{at}_1, {cin} != 0u);\n")),
            }
        }
        offset += words.len();
    }
    // The sinks (`metal_block_sinks.metal`): each output at its delay, cut to the stream; then
    // the counted outputs' counts, exchanged together, their positions and their totals.
    for (k, v) in native.outputs.iter().enumerate() {
        match native.output_delays[k] {
            // A warm-up lane's word goes to the output's last word, which no stream reaches.
            0 => out.push_str(&format!("  out[f.out_base + {k}u * lanes + select(lanes - 1u, gl, keep)] = stream_cut(v{v}, gl, len, 0u);\n")),
            d => out.push_str(&format!("  write_output(out, f.out_base + {k}u * lanes, gl, len, {d}u, select({m}(0), v{v}, keep));\n")),
        }
    }
    // A total is a flag (`BlockProgram::totals`): a group with a bit sets it, in any order and
    // again on a relaunch, so it waits on no other group.
    let extracts = native.extracts.len();
    for (t, &k) in native.totals.iter().enumerate() {
        out.push_str(&format!("  totals_or(select({m}(0), stream_cut(v{}, gl, len, {}u), keep), l, counts + f.tot_base + {}u);\n", native.outputs[k], native.output_delays[k], extracts + t));
    }
    if extracts > 0 {
        let mut words = Vec::new();
        for (c, &k) in native.extracts.iter().enumerate() {
            out.push_str(&format!(
                "  const {m} cw{c} = select({m}(0), stream_cut(v{}, gl, len, {}u), keep);\n  const uint cn{c} = uint(popcount(lane_bits(cw{c})));\n  const uint ca{c} = simd_sum(cn{c});\n",
                native.outputs[k], native.output_delays[k]
            ));
            words.push(format!("ca{c}"));
        }
        let got = exchange_kinds(&mut out, "xc", &words, |_| "2u".to_owned(), offset, record);
        for (c, &k) in native.extracts.iter().enumerate() {
            out.push_str(&format!(
                "  count_finish(cw{c}, cn{c}, ca{c}, {}, gl, g, l, f.groups, {}u, positions + f.pos_base + {c}u * max(len, 1u), max(len, 1u), true, counts + f.tot_base + {c}u);\n",
                got[c],
                native.output_delays[k],
            ));
        }
    }
    out
}

/// A program's form in a full group: one past the file's first and inside the file, so that
/// every `within` is all ones and the values the CPU's known bits find all zeros or all ones in
/// a full block (`native_cpu_lanes::Plan::full`) are constants. Only those two: a lane word is
/// narrower than a block, and a constant of both kinds is the same at every width. A carry
/// (`toggle`, `escaped`, `fill`) is never folded: its look-back is the same in every body.
fn full_form(native: &BlockNative) -> BlockNative {
    let plan = super::native_cpu_lanes::Plan::of(native);
    let mut form = native.clone();
    let members = group_members(native);
    for (at, inst) in form.insts.iter_mut().enumerate() {
        if members[at] {
            continue;
        }
        let folded = match (*inst, plan.full[at]) {
            (BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..), _) => None,
            (BlockInst::Within(_), _) => Some(u64::MAX),
            (_, Some(c)) if c == 0 || c == u64::MAX => Some(c),
            _ => None,
        };
        if let Some(c) = folded {
            *inst = BlockInst::Const(c);
        }
    }
    form
}

/// Whether each instruction is a fixed point's member: its value is the loop's, never folded.
fn group_members(native: &BlockNative) -> Vec<bool> {
    let mut members = vec![false; native.insts.len()];
    for g in &native.groups {
        for &m in &g.members {
            members[m] = true;
        }
    }
    members
}

/// A program's form in a full group of ASCII bytes: `full_form`, with each byte-local value known
/// on ASCII bytes (`native_cpu_lanes::Plan::ascii`) a constant: a UTF-8 check's, say.
fn ascii_form(native: &BlockNative) -> BlockNative {
    let plan = super::native_cpu_lanes::Plan::of(native);
    let mut form = full_form(native);
    let members = group_members(native);
    for (at, inst) in form.insts.iter_mut().enumerate() {
        if inst.carried() || !plan.lanes[at] || members[at] {
            continue;
        }
        if let Some(one) = plan.ascii[at] {
            *inst = BlockInst::Const(if one { u64::MAX } else { 0 });
        }
    }
    form
}

/// The instructions in `chosen`, as straight-line statements for a word of `width` whose first
/// byte is chunk `base` (16-byte chunks) and stream byte `offset` (for `const`'s phase).
/// `carried` writes each carried instruction's statements. An empty `base`: the word's chunks
/// `c0..` and the byte before them, `before`, are loaded already (the serial loop loads a block
/// ahead).
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
    let loaded = base.is_empty();
    if !loaded && (back || insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_)))) {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 c{c} = load16(bytes, f, {base} + {c}u);\n"));
        }
    }
    // The byte before the word, read from the source (no carry); and, for `pair` by lookups,
    // the word's bytes one back.
    if back && !loaded {
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
    // Each instruction's statements, into `out`.
    let mut one = |at: usize, out: &mut String| {
        let inst = &native.insts[at];
        if inst.carried() {
            carried(at, inst, out);
            return;
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
            // The period's bits shifted up to the word's first byte of the residue.
            BlockInst::Phase(k, r) => {
                let bits = super::block::period_bits(k);
                format!("{m}({bits:#018x}ul << (({r}u + {k}u - (({offset}) % {k}u)) % {k}u))")
            }
            BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..) => unreachable!(),
            // Bit j of each byte's place in its file: below 6 a pattern of period 2^(j+1), from
            // the word's first byte (a lane's bytes start at a multiple of its width); from 6 on
            // the same in all of a word's bytes, at most 64 of them.
            BlockInst::Position(j) if j < 6 => {
                let pattern = (0..64u32).filter(|i| i >> j & 1 == 1).fold(0u64, |a, i| a | 1 << i);
                format!("{m}({pattern:#018x}ul >> ulong(({offset}) & 63u))")
            }
            BlockInst::Position(j) => format!("{m}(0) - {m}((uint({offset}) >> {j}u) & 1u)"),
            BlockInst::Carry(_) => unreachable!("a carry is its group's loop's (`emit`)"),
            // A slot word of a pass before (`native_ir::Passes`): the forward carry's, a block a
            // word, in the output buffer `out`.
            BlockInst::Load(k) => format!("out[f.slot_base + ({offset}) / 64u * SLOTS{p} + {k}u]"),
            BlockInst::BackFill(..) => unreachable!("a backward pass's (`metal_block_serial::back_kernels`)"),
        };
        out.push_str(&format!("  const {m} v{at} = {rhs};\n"));
    };
    // The order: `chosen`'s, each fixed point (`rounds until settled`) one step where its members
    // can all run (`schedule`).
    for step in schedule(native, chosen) {
        match step {
            Step::One(at) => one(at, &mut out),
            Step::Group(g) => group_loop(native, g, m, &mut |at, out| one(at, out), &mut out),
        }
    }
    let _ = bits;
    out
}

/// A step of `emit`: an instruction, or a fixed point's group (`BlockNative::groups`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    One(usize),
    Group(usize),
}

/// `chosen` in an order its values can be made in: each instruction once its operands are, the
/// first ready first; a group once every operand of its members outside it is, its members
/// together. As Cranelift places a group (`native_cpu_block`).
fn schedule(native: &BlockNative, chosen: &[usize]) -> Vec<Step> {
    let mut group_of = vec![None; native.insts.len()];
    for (g, group) in native.groups.iter().enumerate() {
        for &m in &group.members {
            group_of[m] = Some(g);
        }
    }
    if chosen.iter().all(|&at| group_of[at].is_none()) {
        return chosen.iter().map(|&at| Step::One(at)).collect();
    }
    let wanted: std::collections::BTreeSet<usize> = chosen.iter().copied().collect();
    // A value not chosen is made already (an earlier level's), or not at all.
    let mut done: Vec<bool> = (0..native.insts.len()).map(|at| !wanted.contains(&at)).collect();
    let mut ran = vec![false; native.groups.len()];
    let mut out = Vec::with_capacity(chosen.len());
    loop {
        let mut progress = false;
        for &at in chosen {
            if !done[at] && group_of[at].is_none() && native.insts[at].operands().iter().all(|&o| done[o]) {
                out.push(Step::One(at));
                done[at] = true;
                progress = true;
            }
        }
        for (g, group) in native.groups.iter().enumerate() {
            if !ran[g] && group.members.iter().all(|&m| wanted.contains(&m)) && group.members.iter().all(|&m| native.insts[m].operands().iter().all(|&o| done[o] || group_of[o] == Some(g))) {
                out.push(Step::Group(g));
                for &m in &group.members {
                    done[m] = true;
                }
                ran[g] = true;
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    assert!(chosen.iter().all(|&at| done[at]), "every chosen value is made: a group's members are chosen together");
    out
}

/// Group `g`'s loop (`rounds until settled`): its members made again from its carries until no
/// carry changes in any thread of the SIMD group, at most `MAX_ROUNDS` rounds a block of the
/// group. Every member is a variable before the loop, so the words after it read the last
/// round's; the carried members' outs too (`o`, `co`), read by the block's tail. A thread that
/// has settled makes the same values again: a fixed point.
fn group_loop(native: &BlockNative, g: usize, m: &str, one: &mut dyn FnMut(usize, &mut String), out: &mut String) {
    let group = &native.groups[g];
    for &x in &group.members {
        out.push_str(&format!("  {m} v{x} = {m}(0);\n"));
        if matches!(native.insts[x], BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..)) {
            out.push_str(&format!("  uint o{x} = 0u, co{x} = 0u;\n"));
        }
    }
    for (k, &(c, _)) in group.carries.iter().enumerate() {
        let BlockInst::Carry(init) = native.insts[c] else { unreachable!("a carry") };
        out.push_str(&format!("  {m} gc{g}_{k} = v{init};\n"));
    }
    let bound = super::block::MAX_ROUNDS * GROUP;
    out.push_str(&format!("  for (uint r{g} = 0u; r{g} < {bound}u; r{g}++) {{\n"));
    for (k, &(c, _)) in group.carries.iter().enumerate() {
        out.push_str(&format!("    v{c} = gc{g}_{k};\n"));
    }
    for &x in &group.members {
        if matches!(native.insts[x], BlockInst::Carry(_)) {
            continue;
        }
        let mut text = String::new();
        one(x, &mut text);
        // The member's own declarations become assignments to the variables above.
        let text = text
            .replace(&format!("const {m} v{x} = "), &format!("v{x} = "))
            .replace(&format!("const ulong v{x} = "), &format!("v{x} = "))
            .replace(&format!("uint o{x} = 0u;"), &format!("o{x} = 0u;"))
            .replace(&format!("const uint co{x} = "), &format!("co{x} = "));
        for line in text.lines() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    let changed: Vec<String> = group.carries.iter().enumerate().map(|(k, &(_, x))| format!("v{x} != gc{g}_{k}")).collect();
    out.push_str(&format!("    const bool more{g} = {};\n", changed.join(" || ")));
    for (k, &(_, x)) in group.carries.iter().enumerate() {
        out.push_str(&format!("    gc{g}_{k} = v{x};\n"));
    }
    out.push_str(&format!("    if (!simd_any(more{g})) break;\n  }}\n"));
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

