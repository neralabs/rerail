//! Block programs on the device (`block`), both ways the transducer runs there, each over a
//! batch of files in one launch (decision 0066), in UKL on the process's device (decision 0440
//! step C):
//!
//! - **the interpreter** (`block_program.uglykl`): one kernel for any programs, reading each
//!   program's steps from a table;
//! - **generated code** from the native IR (`BlockNative`): a function per program, the operations
//!   written out straight-line, and one kernel that calls the file's.
//!
//! Both are compiled after `block_common.uglykl`, for one lane width each. A group is a SIMD group
//! of 32 lanes, its ballots 32-bit words: where the device's SIMD width is not 32 a batch is
//! refused (`ready`), and the CPU's executors run it.
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
#[allow(unused_imports)]
use crate::Error;
use crate::lane;
pub use rerail_compiler::block::BatchFile;
use rerail_compiler::block::BlockRun;
use rerail_compiler::block_native::{BlockInst, BlockNative};
use rerail_device::device::Buffer;

/// The same programs in the CPU's shape: a thread a segment of blocks, the carry forward by
/// speculation (`block_serial.rs`).
#[path = "block_serial.rs"]
pub mod serial;

/// The shared code (`block_common.uglykl`): the file table, the carries, the sinks and the
/// look-backs, for the lane word the source defines in front of it (`head`).
const COMMON: &str = include_str!("block_common.uglykl");
/// The interpreter's source at each lane width (`LaneWidth::ALL`): the shared code, then its
/// kernel.
static INTERPRETER: std::sync::LazyLock<[String; 3]> =
    std::sync::LazyLock::new(|| LaneWidth::ALL.map(|w| format!("{}{}", head(w), include_str!("block_program.uglykl"))));

/// A source's head at lane width `width`: its word (`BW`, `BM`), how a poll reads
/// (`poll_rmw`), and the shared code. Every source starts with it, the interpreter's and each generated one.
pub fn head(width: LaneWidth) -> String {
    format!("#define BW {}\n#define BM {}\n#define BLOCK_POLL_RMW {}\n{COMMON}\n", width.bytes(), width.word(), u32::from(poll_rmw()))
}

/// Whether a poll is a read-modify-write (`poll` in `block_common.uglykl`): on a Metal GPU before
/// Apple's family 9. On the M1 Max (family 7, Q123) a relaxed load in a poll loop kept reading the
/// polling core's copy and never saw the store of a group running beside it: every launch of a
/// batch ran out of polls, whatever their number (5–9 launches at 65,536 polls, as at none, at
/// 768 workers as at 32), though a `volatile` or `coherent(device)` load did no better. A relaxed
/// read-modify-write reads the coherent copy: one launch. Family 9 on (the M3, the M5) the load
/// sees it, and stays a load there. OpenCL's load is a read-modify-write already.
fn poll_rmw() -> bool {
    // A fact of the machine's device, found once (decision 0442: the machine's).
    #[derive(Default)]
    struct PollRmw(std::sync::OnceLock<bool>);
    *rerail_context::machine::<PollRmw>().0.get_or_init(|| {
        lane::device().is_ok_and(|d| d.metal().is_some()) && crate::device().is_ok_and(|d| !d.supports_family(metal::MTLGPUFamily::Apple9))
    })
}

/// The device the batches run on, where they can: a group is a SIMD group of 32 lanes, so a
/// device of another SIMD width refuses them (`UnsupportedBackend`), and the CPU runs them.
pub fn ready() -> Result<&'static rerail_device::device::Device, Error> {
    let device = lane::device()?;
    if device.simd_width() != GROUP as u32 {
        return Err(Error::UnsupportedBackend);
    }
    Ok(device)
}
/// Lanes per threadgroup: one SIMD group.
const GROUP: usize = 32;

/// The bytes each lane owns, and so the width of its mask words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaneWidth {
    W16,
    W32,
    W64,
}

impl LaneWidth {
    pub const ALL: [LaneWidth; 3] = [LaneWidth::W16, LaneWidth::W32, LaneWidth::W64];

    pub fn bytes(self) -> usize {
        match self {
            LaneWidth::W16 => 16,
            LaneWidth::W32 => 32,
            LaneWidth::W64 => 64,
        }
    }

    /// The C type of a lane word (`BM` in the kernels).
    fn word(self) -> &'static str {
        match self {
            LaneWidth::W16 => "uint16_t",
            LaneWidth::W32 => "uint32_t",
            LaneWidth::W64 => "uint64_t",
        }
    }

    /// Its place in `ALL`.
    fn index(self) -> usize {
        match self {
            LaneWidth::W16 => 0,
            LaneWidth::W32 => 1,
            LaneWidth::W64 => 2,
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
        BlockInst::Walk { .. } | BlockInst::Args(..) => unreachable!("the CPU's form only: Metal runs a walk's rounds"),
        BlockInst::BackFill(..) | BlockInst::Load(_) => unreachable!("refused at `run_batch`: Metal runs no backward pass yet"),
        // `a >> k` in the word (decision 0330): `a` has no bits past a file's first `EARLY_BITS`,
        // all in its first lane.
        BlockInst::Early(a, k) => [17, u32::from(a), 0, u32::from(k)],
        // `n` in `a`, the bytes in `b` and `imm` (decision 0335).
        BlockInst::Prefix(n, w) => [18, u32::from(n), w as u32, (w >> 32) as u32],
        // `j` in `a`; below 6 the period's pattern in `b` and `imm`.
        BlockInst::Position(j) => {
            let pattern = (0..64u32).filter(|i| u32::from(j) < 6 && i >> j & 1 == 1).fold(0u64, |a, i| a | 1 << i);
            [16, u32::from(j), pattern as u32, (pattern >> 32) as u32]
        }
        // The period in `a` (`k | r << 8`, never read as a slot: it is clamped), its bits in `b` and `imm`.
        BlockInst::Phase(k, r) => {
            let bits = rerail_compiler::block::period_bits(k);
            [15, u32::from(k) | u32::from(r) << 8, bits as u32, (bits >> 32) as u32]
        }
        BlockInst::Class { .. } | BlockInst::Pair { .. } => unreachable!("the planes form has no class or pair"),
        BlockInst::Cmp { .. } | BlockInst::Bit { .. } => unreachable!("a row program's, never a byte program's"),
    }
}

/// Which Metal executor runs a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Executor {
    Interpreter,
    Generated(ClassStrategy),
}


/// A batch's results: each file's outputs and extract positions, the launches it took, and their
/// GPU seconds.
pub struct BatchRun {
    pub files: Vec<BlockRun>,
    pub launches: usize,
    pub seconds: f64,
}

/// Workers of the persistent dispatch: SIMD groups that take the batch's tickets in order. Every
/// worker must be resident: a worker waiting on a ticket held by one that is not polls until its
/// budget runs out and the launch is repeated. On an M5 Max (40 GPU cores) JSON's tier 1 ran
/// fastest at 768 (step 15) and 1,024 (step 14), and fell off a cliff past 1,024 (doc 22).
const WORKERS: usize = 768;

/// `run_batch`, split while the device refuses it for capacity (lessons ledger #26: a batch over
/// a budget is split, never refused). Halves run one after another, in the files' order; a file
/// alone that does not fit is the error. `launches` and `seconds` are the halves' sums.
pub fn run_split(
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

/// Whether generated UKL's pipeline for `programs` at `width` (classes by lookups) is compiled; when it is not, its
/// compile starts on a thread of its own and this is false at once: the caller runs the batch on the interpreter's
/// pipeline (compiled once for every program) meanwhile, never waiting on the compiler (`kernels::ready_behind_function`).
pub fn generated_ready(programs: &[&BlockNative], width: LaneWidth) -> bool {
    ready().is_ok() && rerail_accel::gpu::kernels::ready_behind_function(&generate_forms(programs, width), "tempo_batch", None).is_some()
}

/// The rows of a batch's slabs (`MAX_SLABS`), in every kernel that reads a file's bytes.
const SLAB_BUFFERS: [&str; crate::slabs::MAX_SLABS] = ["s0", "s1", "s2", "s3"];

/// Launches repeated because a look-back ran out of polls: the run's count (decision 0442).
#[derive(Default)]
pub struct RepeatedLaunches(pub std::sync::atomic::AtomicUsize);

/// What `program` needs that generated UKL lacks (decision 0202's `metal-form` list), if any: a
/// `run_batch` of it is refused. A fixed point (`rounds`) is the CPU's for now: each lane would run
/// its rounds in its own frame, within `block::MAX_ROUNDS`, to the same values. A `back_fill`
/// likewise: a backward pass over the groups' summaries, not generated yet. An `early` (decision
/// 0330) and a prefix (0335) run: the first is a shift in the file's first word, the second its
/// first bytes compared there.
pub fn missing_form(program: &BlockNative) -> Option<&'static str> {
    if !program.groups.is_empty() {
        Some("a fixed point (rounds)")
    } else if program.insts.iter().any(|i| matches!(i, BlockInst::BackFill(..) | BlockInst::Load(_))) {
        Some("a back fill or a load")
    } else {
        None
    }
}

/// Run `files` in one launch, each under `programs[file.program]` (decision 0066). `spins` is the
/// polls of a predecessor after the first; 0 tests the repeated launch.
pub fn run_batch(
    programs: &[&BlockNative],
    files: &[BatchFile<'_>],
    width: LaneWidth,
    executor: Executor,
    spins: u32,
) -> Result<BatchRun, Error> {
    if let Some(reason) = programs.iter().find_map(|p| missing_form(p)) {
        rerail_accel::forms::missing("metal-form", "block program", "block program", reason, files.len() as u64);
        return Err(Error::UnsupportedBackend);
    }
    ready()?;
    let word = width.bytes() / 8;
    let span = width.bytes() * GROUP;
    // The form each executor runs: `class` by lookups only in generated UKL's `Lookup`.
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
    let sources = crate::slabs::admit_sources(&bytes)?;
    let (slabs, slab_of) = crate::slabs::source_slabs(&sources).ok_or(Error::BackendCapacity)?;
    if files.is_empty() {
        return Ok(BatchRun { files: Vec::new(), launches: 0, seconds: 0.0 });
    }
    // The file table, and each base after the files before.
    let (mut group0, mut out_base, mut state_base, mut halo_base, mut ext_base, mut pos_base, mut tot_base) = (0, 0, 0, 0, 0, 0, 0);
    let counted: Vec<Vec<usize>> = forms.iter().map(rerail_compiler::native_cpu_block::counted).collect();
    let mut rows = Vec::with_capacity(files.len());
    let mut group_file: Vec<u32> = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let n = forms.get(file.program).ok_or(Error::BackendFailure)?;
        let len = file.bytes.len();
        // Generated UKL's groups each start with `warm_lanes` lanes whose sinks write nothing.
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
        // The interpreter's look-back states, a word per carried instruction; generated UKL's
        // exchange records (`record_words`).
        state_base += records[file.program] * groups;
        halo_base += carried[file.program] * groups * 3;
        ext_base += counted[file.program].len() * groups;
        pos_base += n.extracts.len() * len.max(1);
        tot_base += counted[file.program].len();
    }
    let total = group0;
    let table = crate::pool::upload(&rows)?;
    // Generated UKL takes the groups round robin over the files (`ticket_order`): a (file, group)
    // per ticket. The interpreter takes them file by file.
    let group_file = match executor {
        Executor::Interpreter => crate::pool::upload(&group_file)?,
        Executor::Generated(_) => crate::pool::upload(&ticket_order(&rows.iter().map(|r| r.groups as usize).collect::<Vec<_>>()))?,
    };
    let out = crate::pool::zeroed::<u8>(out_base.max(1) * word)?;
    let states = crate::pool::zeroed::<u32>(state_base.max(1))?;
    let halos = crate::pool::zeroed::<u32>(halo_base.max(1))?;
    let ext_states = crate::pool::zeroed::<u32>(ext_base.max(1))?;
    let counts = crate::pool::zeroed::<u32>(tot_base.max(1))?;
    let positions = crate::pool::zeroed::<u32>(pos_base.max(1))?;
    let tickets = crate::pool::zeroed::<u32>(2)?;
    let done = crate::pool::zeroed::<u32>(total)?;
    let workers = workers(total);
    let most = forms.iter().map(|n| n.insts.len()).max().unwrap_or(0);
    let params = [fit(total)?, spins, fit(most)?, 0];
    let mut buffers: Vec<(&'static str, Buffer)> = vec![
        ("out", out.device_buffer().clone()),
        ("files", table.device_buffer().clone()),
        ("group_file", group_file.device_buffer().clone()),
        ("states", states.device_buffer().clone()),
        ("halos", halos.device_buffer().clone()),
        ("ext_states", ext_states.device_buffer().clone()),
        ("positions", positions.device_buffer().clone()),
        ("counts", counts.device_buffer().clone()),
        ("tickets", tickets.device_buffer().clone()),
        ("done", done.device_buffer().clone()),
    ];
    // The slabs, each bound as it is (no copy); unused bindings repeat the first.
    for (k, name) in SLAB_BUFFERS.iter().enumerate() {
        buffers.push((name, slabs.get(k).unwrap_or(&slabs[0]).device_buffer().clone()));
    }
    let interpreter;
    let (name, kernel) = match executor {
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
                    exts: fit(rerail_compiler::native_cpu_block::counted(n).len())?,
                    pad: [0; 2],
                });
            }
            steps.push([0; 4]);
            outs.push(0);
            interpreter = [
                crate::pool::upload(&table)?,
                crate::pool::upload(&steps)?,
                crate::pool::upload(&outs)?,
                // Each worker's values: a slot of 32 lane words per step (decision 0034).
                crate::pool::zeroed::<u8>((workers * most * GROUP * word).max(1))?,
            ];
            for (name, block) in ["programs", "steps", "outs", "scratch"].into_iter().zip(&interpreter) {
                buffers.push((name, block.device_buffer().clone()));
            }
            ("block_batch", lane::kernel_fixed(&INTERPRETER[width.index()], "block_batch", None))
        }
        Executor::Generated(_) => {
            let refs: Vec<&BlockNative> = forms.iter().collect();
            ("tempo_batch", lane::kernel(&generate_forms(&refs, width), "tempo_batch", None))
        }
    };
    let pass = Pass { name, kernel, buffers, args: vec![("p", params)], groups: workers, threads: GROUP };
    // Each launch finishes at least the first unfinished group, whose predecessors are all done.
    let (mut seconds, mut launches) = (0.0, 0);
    loop {
        if launches == total {
            return Err(Error::BackendFailure);
        }
        if launches > 0 {
            rerail_context::with_local::<RepeatedLaunches, _>(|r| r.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
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
        let totals = if n.totals.is_empty() { Vec::new() } else { vec![u64::from(count(n.extracts.len()) != 0)] };
        let _ = groups;
        // This executor has no record pass: a program with records runs on the serial one.
        results.push(BlockRun { outputs, positions, totals, codes: Vec::new() });
    }
    Ok(BatchRun { files: results, launches, seconds })
}

/// The interpreter: every output's words over `bytes`, as 64-bit words of the stream.
pub fn interpret(
    program: &rerail_compiler::block::BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    interpret_extract(program, bytes, width).map(|run| run.outputs)
}

/// The interpreter with the extract pass: a batch of one file.
pub fn interpret_extract(
    program: &rerail_compiler::block::BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<BlockRun, Error> {
    let native = BlockNative::lower(program);
    let mut run = run_batch(&[&native], &[BatchFile { bytes, program: 0 }], width, Executor::Interpreter, SPINS)?;
    Ok(run.files.remove(0))
}

/// How generated UKL computes `class`. `Planes` is the default: on simdjson's five classes over
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
pub enum ClassStrategy {
    Lookup,
    Planes,
}

impl ClassStrategy {
    pub const ALL: [ClassStrategy; 2] = [ClassStrategy::Lookup, ClassStrategy::Planes];
}

/// The native IR as generated UKL for lanes of `width`, `class` by bit-planes.
pub fn generate(native: &BlockNative, width: LaneWidth) -> String {
    generate_with(native, width, ClassStrategy::Planes)
}

/// The lane's eight bit-planes `p0..p7` from its chunks `c0..`: `transpose8` on each 8 bytes,
/// then byte `k` of every group gathered into plane `k` (for 64-byte lanes, an 8x8 byte
/// transpose).
fn planes(width: LaneWidth) -> String {
    let groups = width.bytes() / 8;
    let mut out = String::new();
    for g in 0..groups {
        let (c, half) = (g / 2, if g % 2 == 0 { ("x", "y") } else { ("z", "w") });
        out.push_str(&format!(
            "  uint64_t t{g} = transpose8(U64(c{c}.{}) | (U64(c{c}.{}) << 32u));\n",
            half.0, half.1
        ));
    }
    if groups == 8 {
        for (sh, mask, step) in [(32, 0x0000_0000_FFFF_FFFFu64, 4), (16, 0x0000_FFFF_0000_FFFF, 2), (8, 0x00FF_00FF_00FF_00FF, 1)] {
            for g in (0..8).filter(|g| g & step == 0) {
                let h = g + step;
                out.push_str(&format!(
                    "  {{ const uint64_t a = t{g}, b = t{h}; t{g} = (a & {mask:#018x}ul) | ((b & {mask:#018x}ul) << {sh}u); \
                     t{h} = ((a >> {sh}u) & {mask:#018x}ul) | (b & ~{mask:#018x}ul); }}\n"
                ));
            }
        }
        for k in 0..8 {
            out.push_str(&format!("  const uint64_t p{k} = t{k};\n"));
        }
    } else {
        for k in 0..8 {
            let parts: Vec<String> = (0..groups)
                .map(|g| {
                    let byte = format!("((t{g} >> {}u) & 0xfful)", k * 8);
                    if g == 0 { byte } else { format!("({byte} << {}u)", g * 8) }
                })
                .collect();
            out.push_str(&format!("  const BM p{k} = BMC({});\n", parts.join(" | ")));
        }
    }
    out
}

/// One table's class bytes of a `B4` by lookups: byte `j`'s `lo[b & 15] & hi[b >> 4]`.
fn lookup_helper(p: usize, t: usize, table: &rerail_compiler::block::ClassTable) -> String {
    let hex = |xs: &[u8]| xs.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
    let mut out = format!(
        "UKL_TABLE uint8_t T{p}_{t}[32] = {{{}, {}}};\nUKL_INLINE B4 classes_p{p}_t{t}(B4 v) {{\n  B4 k;\n",
        hex(&table.lo),
        hex(&table.hi)
    );
    for lane in ["x", "y", "z", "w"] {
        let bytes: Vec<String> = (0..4)
            .map(|j| {
                let b = format!("U32(T{p}_{t}[(v.{lane} >> {}u) & 15u] & T{p}_{t}[16u + ((v.{lane} >> {}u) & 15u)])", 8 * j, 8 * j + 4);
                if j == 0 { b } else { format!("({b} << {}u)", 8 * j) }
            })
            .collect();
        out.push_str(&format!("  k.{lane} = {};\n", bytes.join(" | ")));
    }
    out.push_str("  return k;\n}\n");
    out
}

/// The native IR as UKL for lanes of `width`, `class` by `strategy`: a batch's kernel for one
/// program.
pub fn generate_with(native: &BlockNative, width: LaneWidth, strategy: ClassStrategy) -> String {
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
    let mut out = head(width);
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
                "      case {p}u: ok = program_{p}(bytes, f, fg1, l, p.spins, out, states, halos, ext_states, positions, counts __UKL_ARGS); break;\n"
            )
        })
        .collect();
    out.push_str(&format!(
        "// A launch's parameters: the batch's groups, the polls after the first.\n\
         typedef struct BatchP {{ uint32_t groups, spins, most, pad; }} BatchP;\n\
         UKL_REFLECT_STRUCT(BatchP, groups, spins, most, pad)\n\
         // Persistent workers (decision 0027): each takes group numbers in launch order and runs its\n\
         // file's program over the group. A threadgroup is a worker: one group of 32 lanes.\n\
         DEFINE_KERNEL2(block, tempo_batch,\n\
         \x20   ROBUFFER, uint32_t, s0, \"[slab]\", RWBUFFER, BM, out, \"[outputs]\", ROBUFFER, FileRow, files, \"[files]\",\n\
         \x20   ROBUFFER, uint32_t, group_file, \"[2 * groups]\", RWBUFFER, uint32_t, states, \"[states]\",\n\
         \x20   RWBUFFER, uint32_t, halos, \"[halos]\", RWBUFFER, uint32_t, ext_states, \"[ext_states]\",\n\
         \x20   RWBUFFER, uint32_t, positions, \"[positions]\", RWBUFFER, uint32_t, counts, \"[counts]\", RWBUFFER, uint32_t, tickets, \"[2]\",\n\
         \x20   RWBUFFER, uint32_t, done, \"[groups]\", ROBUFFER, uint32_t, s1, \"[slab]\",\n\
         \x20   ROBUFFER, uint32_t, s2, \"[slab]\", ROBUFFER, uint32_t, s3, \"[slab]\", ARGS, BatchP, p, , LANE, uint32_t, l, )\n\
         {{\n\
         \x20 for (uint32_t round = 0u; round < p.groups; round++) {{\n\
         \x20   // Lane 0 takes the ticket and reads whether its group is done (`block_batch`: lanes need not run\n\
         \x20   // in lockstep).\n\
         \x20   uint32_t ticket = 0u;\n\
         \x20   if (l == 0u) {{\n\
         \x20     ticket = ukl_atomic_add(&tickets[0], 1u);\n\
         \x20     if (ticket < p.groups && done[ticket] != 0u) ticket |= 0x80000000u;\n\
         \x20   }}\n\
         \x20   const uint32_t taken = ukl_sg_shuffle(ticket, 0u __UKL_ARGS);\n\
         \x20   const uint32_t t = taken & 0x7fffffffu;\n\
         \x20   if (t >= p.groups) return;\n\
         \x20   if ((taken >> 31) != 0u) continue;\n\
         \x20   // Its (file, group) pair (`ticket_order`).\n\
         \x20   const uint32_t fg0 = group_file[2u * t], fg1 = group_file[2u * t + 1u];\n\
         \x20   const FileRow f = files[fg0];\n\
         \x20   // The slab the file's bytes are in (decision 0067): the same in every lane.\n\
         \x20   Bytes bytes = f.slab == 0u ? s0 : f.slab == 1u ? s1 : f.slab == 2u ? s2 : s3;\n\
         \x20   bool ok = false;\n\
         \x20   // The same program in every lane: the group holds one file.\n\
         \x20   switch (f.program) {{\n{cases}\
         \x20     default: break;\n\
         \x20   }}\n\
         \x20   if (l == 0u && ok) done[t] = 1u;\n\
         \x20   if (l == 0u && !ok) ukl_atomic_add(&tickets[1], 1u);\n\
         \x20 }}\n\
         }}\n"
    ));
    out
}

/// Generated UKL's order of the groups of files of `groups` groups each: round robin, the first
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

/// Polls of a predecessor's state after the first: bounded, so a launch whose predecessors never
/// publish ends and is repeated. 64 polls ran out on full batches: the launch was repeated while
/// the predecessor was only slow.
pub const SPINS: u32 = 65536;
/// The extract look-back's polls per predecessor, as a multiple of the carries'.
#[allow(dead_code)]
const EXTRACT_POLLS: u32 = 64;


/// How generated UKL treats each carried instruction.
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

/// Generated UKL's kernel for `native` at `width`, as the device compiled it: its exchanges (each
/// one's halos and scans), its record, the lines of UKL, and the pipeline's limits. Metal reports
/// no register count; a pipeline whose threads use many registers allows fewer than 1,024
/// threads a threadgroup.
pub fn kernel_report(native: &BlockNative, width: LaneWidth) -> Result<String, Error> {
    let form = native.planes_form();
    let plan = Plan::new(&form).at(&form, width);
    let exchanges: Vec<String> = plan.exchanges().iter().map(|(h, s)| format!("{}h+{}s", h.len(), s.len())).collect();
    let source = generate_forms(&[&form], width);
    let device = ready()?;
    let kernel = lane::kernel(&source, "tempo_batch", None);
    // Metal's pipeline reports its limits; elsewhere the device's.
    let (threads, simd, memory) = match kernel.metal() {
        Some(m) => {
            let p = &m.compute_pipeline_state;
            (p.max_total_threads_per_threadgroup(), p.thread_execution_width(), p.static_threadgroup_memory_length())
        }
        None => (u64::from(device.max_threadgroup()), u64::from(device.simd_width()), 0),
    };
    Ok(format!(
        "{} instructions; {} local prevs ({} warm-up lanes a group), {} halos, {} scans; exchanges [{}] + counts {}, totals {}; record {} words a group; UKL {} lines; kernel: {} threads a threadgroup at most, SIMD width {}, {} bytes of threadgroup memory; launched as {} threadgroups of {GROUP}",
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
        threads,
        simd,
        memory,
        workers(usize::MAX),
    ))
}

/// What crosses a group's edge in generated UKL (no device): each exchange's scans and halo bits,
/// the edge's bits in all, and the most edge bits any carried value depends on (its cone): a
/// group's summary as a function of its carry-in state has 2^cone candidates at that value.
pub fn boundary_shape(native: &BlockNative) -> String {
    let form = native.planes_form();
    let plan = Plan::new(&form);
    let bits = |at: usize| match form.insts[at] {
        BlockInst::Prev(_, k) => usize::from(k),
        _ => 1,
    };
    let mut cone: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); form.insts.len()];
    for at in 0..form.insts.len() {
        let mut c: std::collections::BTreeSet<usize> = operands(&form.insts[at]).iter().flat_map(|&a| cone[a].iter().copied()).collect();
        if form.insts[at].carried() && !plan.windowed.contains(&at) {
            c.insert(at);
        }
        cone[at] = c;
    }
    let edge: usize = plan.scanned.iter().chain(&plan.halos).map(|&at| bits(at)).sum();
    let most = cone.iter().map(|c| c.iter().map(|&at| bits(at)).sum::<usize>()).max().unwrap_or(0);
    let levels: Vec<String> = plan
        .exchanges()
        .iter()
        .map(|(h, s)| format!("{}s+{}h({}b)", s.len(), h.len(), h.iter().map(|&at| bits(at)).sum::<usize>()))
        .collect();
    // A launch per level: a value used in a later launch than its own is saved across the cut
    // (the counts' launch, after the last level, uses the extracts' values).
    let last = plan.exchanges().len() + usize::from(!form.extracts.is_empty());
    let mut saved = std::collections::BTreeSet::new();
    for at in 0..form.insts.len() {
        for a in operands(&form.insts[at]) {
            if plan.level[at] > plan.level[a] {
                saved.insert(a);
            }
        }
    }
    for &k in &form.extracts {
        let v = usize::from(form.outputs[k]);
        if plan.level[v] < last {
            saved.insert(v);
        }
    }
    format!(
        "{} insts; exchanges [{}]; extracts {}; edge {} bits; widest cone {} bits; saved across cuts {}",
        form.insts.len(),
        levels.join(", "),
        form.extracts.len(),
        edge,
        most,
        saved.len()
    )
}

/// The lanes each group of generated UKL spends warming up: enough lanes of `width` bytes to
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

/// The words per group generated UKL keeps of a program's exchanges (`Plan::exchanges`).
pub fn record_words(native: &BlockNative) -> usize {
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

/// Tables of `m`-bit carry vectors: apply, compose (`a` then `b`), the identity, and the group's
/// exclusive scan (Hillis-Steele over shuffles, five steps) with the group's total. The 1-bit
/// tables are `block_common.uglykl`'s (`tapply1`, ...); this writes the others.
fn table_helpers(m: usize) -> String {
    assert!(m > 1, "the 1-bit tables are the common code's");
    let mask = (1u32 << m) - 1;
    let n = 1usize << m;
    let compose: Vec<String> = (0..n).map(|c| format!("(tapply{m}(b, tapply{m}(a, {c}u)) << {}u)", c * m)).collect();
    let identity: u32 = (0..n as u32).map(|c| c << (c as usize * m)).sum();
    let steps: String = [1, 2, 4, 8, 16].iter().map(|d| format!("\x20 u = ukl_sg_shuffle_up(x, {d}u __UKL_ARGS); x = SEL(x, tcompose{m}(u, x), l >= {d}u);\n")).collect();
    format!(
        "UKL_INLINE uint32_t tapply{m}(uint32_t t, uint32_t c) {{ return (t >> (c * {m}u)) & {mask}u; }}\n\
         UKL_INLINE uint32_t tcompose{m}(uint32_t a, uint32_t b) {{ return {}; }}\n\
         #define TID{m} {identity}u\n\
         UKL_INLINE uint32_t texclusive{m}(uint32_t t, uint32_t l, UKL_PRIVATE uint32_t* total __UKL_PARAMS) {{\n\
         \x20 uint32_t x = t, u;\n{steps}\
         \x20 *total = ukl_sg_shuffle(x, GROUP_LANES - 1u __UKL_ARGS);\n\
         \x20 u = ukl_sg_shuffle_up(x, 1u __UKL_ARGS);\n\
         \x20 return SEL(u, TID{m}, l == 0u);\n}}\n\n",
        compose.join(" | ")
    )
}

/// Program `p` as a function over group `g` (lane `l`) of file `f`: every instruction in order,
/// each once, then the outputs and the extracts. A look-back or a halo that is not published yet
/// returns false: the group is unfinished.
fn program_function(p: usize, native: &BlockNative, plan: &Plan, width: LaneWidth) -> String {
    let w = width.bytes();
    let mut out = format!(
        "UKL_INLINE bool program_{p}(Bytes bytes, FileRow f, uint32_t g, uint32_t l, uint32_t spins, UKL_GLOBAL BM* out,\n\
         \x20                              UKL_GLOBAL uint32_t* states, UKL_GLOBAL uint32_t* halos, UKL_GLOBAL uint32_t* ext_states,\n\
         \x20                              UKL_GLOBAL uint32_t* positions, UKL_GLOBAL uint32_t* counts __UKL_PARAMS) {{\n\
         \x20 // The group's first {h} lanes warm up (`warm_lanes`): they make the bytes before its\n\
         \x20 // first useful lane, for the shuffles of local `prev`s, and their sinks write nothing.\n\
         \x20 const int glw = I32(g * {useful}u + l) - {h};\n\
         \x20 const uint32_t gl = U32(ukl_max(glw, 0));\n\
         \x20 const bool keep = l >= {h}u;\n\
         \x20 const bool before_file = glw < 0;\n\
         \x20 const uint32_t lanes = f.groups * {GROUP}u;\n\
         \x20 const uint32_t len = f.len;\n",
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
    let loads = (1..=chunks).fold(format!("load16(bytes, f, gl * {chunks}u - 1u)"), |a, c| format!("b4_or({a}, load16(bytes, f, gl * {chunks}u + {c}u - 1u))"));
    out.push_str(&format!(
        "  if (g > 0u && (g + 1u) * {span}u <= len) {{\n    const B4 high = {};\n    const bool ascii = (b4_any(high) & 0x80808080u) == 0u;\n    if (sg_all(ascii __UKL_ARGS)) {{\n{}    return true;\n    }}\n{}  return true;\n  }}\n",
        loads,
        levels(p, native, &ascii, plan, width),
        levels(p, native, &full, plan, width)
    ));
    out.push_str(&levels(p, native, native, plan, width));
    out.push_str("  return true;\n}\n\n");
    out
}

/// An exchange of `words` (UKL expressions, the same in every lane) at word `offset` of the
/// group's record of `record` words: the first `halo_words` are halo words, the rest scan
/// tables. Gives each word's result as an expression read by every lane.
#[allow(dead_code)]
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
            mine = format!("SEL({mine}, {w}, l == {j}u)");
            kinds = format!("SEL({kinds}, {}, l == {j}u)", kind(base + j));
        }
        out.push_str(&format!(
            "  uint32_t {name}_{pass} = 0u;\n  const bool {name}_{pass}_waits = !exchange_word(states + f.state_base + {}u + l, g, {record}u, {kinds}, {mine}, spins, l < {}u, &{name}_{pass});\n  if (sg_any({name}_{pass}_waits __UKL_ARGS)) return false;\n",
            offset + base,
            chunk.len(),
        ));
        for j in 0..chunk.len() {
            got.push(format!("sg_bcast({name}_{pass}, {j}u __UKL_ARGS)"));
        }
    }
    got
}

/// A program's levels and sinks (`program_function`), its values from `values`: the program
/// itself, or its full groups' form (`full_form`), whose exchanges are the same words.
fn levels(p: usize, native: &BlockNative, values: &BlockNative, plan: &Plan, width: LaneWidth) -> String {
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
        out.push_str(&emit(p, values, width, &chosen, Word { base: &format!("gl * {chunks}u"), offset: &format!("gl * {w}u"), lanes: true }, |at, inst, out| match *inst {
            BlockInst::Prev(a, k) => out.push_str(&format!("  const BM v{at} = apply_prev(v{a}, {k}u, halo_before(SEL(v{a}, BMC(0), before_file), l, 0ul __UKL_ARGS));\n")),
            _ => unreachable!("only windowed prevs are emitted in a level"),
        }));
        let Some((halos, scans)) = exchanges.get(level) else { break };
        // Each value's words, the same in every lane: the halos' newest bits, packed
        // (`halo_words`), then a scan's aggregate table each.
        let packed = halo_words(native, halos);
        let mut words: Vec<String> = Vec::new();
        for &at in halos {
            let BlockInst::Prev(a, k) = native.insts[at] else { unreachable!() };
            out.push_str(&format!("  const uint64_t last{at} = halo_last(v{a} __UKL_ARGS) >> {}u;\n", 64 - u32::from(k)));
        }
        for word in &packed {
            let parts: Vec<String> = word.iter().map(|p| format!("(U32((last{} >> {}u) & {}ul) << {}u)", p.at, p.lo, (1u64 << p.len) - 1, p.first)).collect();
            words.push(parts.join(" | "));
        }
        let halo_count = packed.len();
        for &at in scans {
            // The lane's table: bit c its carry out for carry in c.
            match native.insts[at] {
                BlockInst::Toggle(a) => out.push_str(&format!("  const uint32_t s{at} = summary_toggle(v{a});\n  const uint32_t tt{at} = s{at} | ((1u ^ s{at}) << 1u);\n")),
                BlockInst::Escaped(a) => out.push_str(&format!(
                    "  uint32_t o{at}_0 = 0u, o{at}_1 = 0u;\n  const BM f{at}_0 = escaped(v{a}, BMC(0), &o{at}_0), f{at}_1 = escaped(v{a}, BMC(1), &o{at}_1);\n  const uint32_t tt{at} = (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u);\n"
                )),
                BlockInst::Fill(a, e) => out.push_str(&format!(
                    "  uint32_t o{at}_0 = 0u, o{at}_1 = 0u;\n  const BM f{at}_0 = fill_word(v{a}, v{e}, BMC(0), &o{at}_0), f{at}_1 = fill_word(v{a}, v{e}, BMC(1), &o{at}_1);\n  const uint32_t tt{at} = (o{at}_0 & 1u) | ((o{at}_1 & 1u) << 1u);\n"
                )),
                _ => unreachable!("a scanned instruction"),
            }
            // A warm-up lane's bytes are the group before's: its table is the identity.
            out.push_str(&format!("  uint32_t agg{at} = 0u;\n  const uint32_t e{at} = texclusive1(SEL(2u, tt{at}, keep), l, &agg{at} __UKL_ARGS);\n"));
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
                .map(|(j, p)| format!("(U64(({} >> {}u) & {}u) << {}u)", got[j], p.first, (1u64 << p.len) - 1, p.lo))
                .collect();
            out.push_str(&format!(
                "  const uint64_t h{at} = ({}) << {}u;\n  const BM v{at} = apply_prev(v{a}, {k}u, halo_before(v{a}, l - {}u, h{at} __UKL_ARGS));\n",
                pieces.join(" | "),
                64 - u32::from(k),
                plan.warm
            ));
        }
        for (s_, &at) in scans.iter().enumerate() {
            let cin = format!("tapply1(e{at}, tapply1({}, 0u))", got[halo_count + s_]);
            match native.insts[at] {
                BlockInst::Toggle(a) => out.push_str(&format!("  const BM v{at} = apply_toggle(v{a}, {cin});\n")),
                _ => out.push_str(&format!("  const BM v{at} = SEL(f{at}_0, f{at}_1, {cin} != 0u);\n")),
            }
        }
        offset += words.len();
    }
    // The sinks (`block_common.uglykl`): each output at its delay, cut to the stream; then
    // the counted outputs' counts, exchanged together, their positions and their totals.
    for (k, v) in native.outputs.iter().enumerate() {
        match native.output_delays[k] {
            // A warm-up lane's word goes to the output's last word, which no stream reaches.
            0 => out.push_str(&format!("  out[f.out_base + {k}u * lanes + SEL(lanes - 1u, gl, keep)] = stream_cut(v{v}, gl, len, 0u);\n")),
            d => out.push_str(&format!("  write_output(out, f.out_base + {k}u * lanes, gl, len, {d}u, SEL(BMC(0), v{v}, keep));\n")),
        }
    }
    // A total is a flag (`BlockProgram::totals`): a group with a bit sets it, in any order and
    // again on a relaunch, so it waits on no other group.
    let extracts = native.extracts.len();
    // All of them into one flag, the count after the extracts' (`native_cpu_block::counted`).
    for &k in &native.totals {
        out.push_str(&format!("  totals_or(SEL(BMC(0), stream_cut(v{}, gl, len, {}u), keep), l, counts + f.tot_base + {extracts}u __UKL_ARGS);\n", native.outputs[k], native.output_delays[k]));
    }
    if extracts > 0 {
        let mut words = Vec::new();
        for (c, &k) in native.extracts.iter().enumerate() {
            out.push_str(&format!(
                "  const BM cw{c} = SEL(BMC(0), stream_cut(v{}, gl, len, {}u), keep);\n  const uint32_t cn{c} = POPCOUNT_BM(lane_bits(cw{c}));\n  const uint32_t ca{c} = ukl_sg_sum(cn{c} __UKL_ARGS);\n",
                native.outputs[k], native.output_delays[k]
            ));
            words.push(format!("ca{c}"));
        }
        let got = exchange_kinds(&mut out, "xc", &words, |_| "2u".to_owned(), offset, record);
        for (c, &k) in native.extracts.iter().enumerate() {
            out.push_str(&format!(
                "  count_finish(cw{c}, cn{c}, ca{c}, {}, gl, g, l, f.groups, {}u, positions + f.pos_base + {c}u * ukl_max(len, 1u), ukl_max(len, 1u), true, counts + f.tot_base + {c}u __UKL_ARGS);\n",
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
    let plan = rerail_compiler::native_cpu_lanes::Plan::of(native);
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
    let plan = rerail_compiler::native_cpu_lanes::Plan::of(native);
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

/// Where `emit`'s word is: its first byte's chunk `base` (16-byte chunks) and stream byte `offset`
/// (for `const`'s phase), and whose it is. An empty `base`: the word's chunks `c0..` and the byte
/// before them, `before`, are loaded already (the serial loop loads a block ahead). `lanes`: the
/// SIMD group's lanes hold one stream's words, so a fixed point's rounds go on while any lane's
/// change (a vote); else each thread runs its own stream, and its own rounds.
pub(crate) struct Word<'a> {
    pub base: &'a str,
    pub offset: &'a str,
    pub lanes: bool,
}

/// The instructions in `chosen`, as straight-line statements for a word of `width` at `word`.
/// `carried` writes each carried instruction's statements.
fn emit(p: usize, native: &BlockNative, width: LaneWidth, chosen: &[usize], word: Word<'_>, mut carried: impl FnMut(usize, &BlockInst, &mut String)) -> String {
    let Word { base, offset, lanes } = word;
    let chunks = width.bytes() / 16;
    let bits = width.bytes();
    let insts: Vec<&BlockInst> = chosen.iter().map(|&at| &native.insts[at]).collect();
    let mut out = String::new();
    let back = insts.iter().any(|i| matches!(i, BlockInst::Pair { .. } | BlockInst::PlaneBefore(_)));
    let loaded = base.is_empty();
    if !loaded && (back || insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::Prefix(..)))) {
        for c in 0..chunks {
            out.push_str(&format!("  const B4 c{c} = load16(bytes, f, {base} + {c}u);\n"));
        }
    }
    // The byte before the word, read from the source (no carry); and, for `pair` by lookups,
    // the word's bytes one back.
    if back && !loaded {
        out.push_str(&format!("  const uint32_t before = file_byte_before(bytes, f, {base});\n"));
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
            out.push_str(&format!("  const B4 k{t}_{c} = classes_p{p}_t{t}(c{c});\n"));
        }
    }
    let used_before: std::collections::BTreeSet<u16> =
        insts.iter().filter_map(|i| if let BlockInst::Pair { before, .. } = **i { Some(before) } else { None }).collect();
    if !used_before.is_empty() {
        for c in 0..chunks {
            let prior = if c == 0 { "before".to_owned() } else { format!("(c{}.w >> 24u)", c - 1) };
            out.push_str(&format!("  const B4 b{c} = bytes_before(c{c}, {prior});\n"));
        }
    }
    for t in used_before {
        for c in 0..chunks {
            out.push_str(&format!("  const B4 kb{t}_{c} = classes_p{p}_t{t}(b{c});\n"));
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
            BlockInst::Cmp { .. } | BlockInst::Bit { .. } => unreachable!("a row program's, never a byte program's"),
            BlockInst::Class { table, bits: sel } => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("BMC(nz16(b4_ands(k{table}_{c}, {s4:#010x}u)))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::Plane(k) => format!("p{k}"),
            BlockInst::PlaneBefore(k) => format!("BMC((BMC(p{k}) << 1u) | BMC((before >> {k}u) & 1u))"),
            BlockInst::Pair { before, table, bits: sel } => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("BMC(nz16(b4_ands(b4_and(kb{before}_{c}, k{table}_{c}), {s4:#010x}u)))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::EqByte(byte) => {
                let c4 = u32::from(byte) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("BMC(eq16(c{c}, {c4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            // The pattern's bits from the word's first byte on: a right rotation by its phase.
            BlockInst::Const(c) => format!("BMC(rotr64({c:#018x}ul, U64(({offset}) & 63u)))"),
            BlockInst::Not(a) => format!("BMC(~v{a})"),
            BlockInst::And(a, b) => format!("v{a} & v{b}"),
            BlockInst::Or(a, b) => format!("v{a} | v{b}"),
            BlockInst::Xor(a, b) => format!("v{a} ^ v{b}"),
            BlockInst::AndNot(a, b) => format!("BMC(v{a} & ~v{b})"),
            // The bits of the word's bytes below the stream's end.
            BlockInst::Within(0) => format!("within_lane(len, {offset})"),
            BlockInst::Within(d) => format!("BMC(within_lane(len + {d}u, {offset}) & ~within_lane({d}u, {offset}))"),
            // The period's bits shifted up to the word's first byte of the residue.
            BlockInst::Phase(k, r) => {
                let bits = rerail_compiler::block::period_bits(k);
                format!("BMC({bits:#018x}ul << (({r}u + {k}u - (({offset}) % {k}u)) % {k}u))")
            }
            BlockInst::Prev(..) | BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..) => unreachable!(),
            // Bit j of each byte's place in its file: below 6 a pattern of period 2^(j+1), from
            // the word's first byte (a lane's bytes start at a multiple of its width); from 6 on
            // the same in all of a word's bytes, at most 64 of them.
            BlockInst::Position(j) if j < 6 => {
                let pattern = (0..64u32).filter(|i| i >> j & 1 == 1).fold(0u64, |a, i| a | 1 << i);
                format!("BMC({pattern:#018x}ul >> U64(({offset}) & 63u))")
            }
            BlockInst::Position(j) => format!("BMC(0) - BMC((U32({offset}) >> {j}u) & 1u)"),
            BlockInst::Carry(_) => unreachable!("a carry is its group's loop's (`emit`)"),
            // `a >> k` in the word (decision 0330): `a` has no bits past a file's first
            // `EARLY_BITS`, which lie in its first word at every width.
            BlockInst::Early(a, k) => format!("BMC(v{a} >> {k}u)"),
            // The low `n` bits of the file's first word where its first `n` bytes are `w`'s
            // (decision 0335): the word's first chunk, compared once.
            BlockInst::Prefix(n, w) => {
                let cut = if n >= 8 { u64::MAX } else { (1u64 << (8 * n)) - 1 };
                format!("(({offset}) == 0u && len >= {n}u && ((U64(c0.x) | (U64(c0.y) << 32u)) & {cut:#x}ul) == {w:#x}ul) ? BMC({:#x}ul) : BMC(0)", (1u64 << n) - 1)
            }
            BlockInst::Walk { .. } | BlockInst::Args(..) => unreachable!("the CPU's form only: Metal runs a walk's rounds"),
            // A slot word of a pass before (`native_ir::Passes`): the forward carry's, a block a
            // word, in the output buffer `out`.
            BlockInst::Load(k) => format!("out[f.slot_base + ({offset}) / 64u * SLOTS{p} + {k}u]"),
            BlockInst::BackFill(..) => unreachable!("a backward pass's (`metal_block_serial::back_kernels`)"),
        };
        out.push_str(&format!("  const BM v{at} = {rhs};\n"));
    };
    // The order: `chosen`'s, each fixed point (`rounds until settled`) one step where its members
    // can all run (`schedule`).
    for step in schedule(native, chosen) {
        match step {
            Step::One(at) => one(at, &mut out),
            Step::Group(g) => group_loop(native, g, lanes, &mut |at, out| one(at, out), &mut out),
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
    if let Some(&at) = chosen.iter().find(|&&at| !done[at]) {
        let why: Vec<String> = native.insts[at].operands().iter().filter(|&&o| !done[o]).map(|&o| format!("%{o} {:?} (group {:?}, chosen {})", native.insts[o], group_of[o], wanted.contains(&o))).collect();
        let blocked: Vec<String> = native
            .groups
            .iter()
            .enumerate()
            .filter(|&(g, _)| !ran[g])
            .flat_map(|(g, group)| group.members.iter().flat_map(move |&m| native.insts[m].operands().into_iter().map(move |o| (g, m, o))).collect::<Vec<_>>())
            .filter(|&(g, _, o)| !done[o] && group_of[o] != Some(g))
            .map(|(g, m, o)| format!("group {g}: %BM {:?} reads %{o} {:?} (group {:?}, chosen {})", native.insts[m], native.insts[o], group_of[o], wanted.contains(&o)))
            .collect();
        panic!("%{at} {:?} (group {:?}) is never made: waits on {why:?}; {blocked:?}", native.insts[at], group_of[at]);
    }
    out
}

/// Group `g`'s loop (`rounds until settled`): its members made again from its carries until no
/// carry changes in any thread of the SIMD group (`lanes`; else in the thread), at most `MAX_ROUNDS` rounds a block of the
/// group. Every member is a variable before the loop, so the words after it read the last
/// round's; the carried members' outs too (`o`, `co`), read by the block's tail. A thread that
/// has settled makes the same values again: a fixed point.
fn group_loop(native: &BlockNative, g: usize, lanes: bool, one: &mut dyn FnMut(usize, &mut String), out: &mut String) {
    let group = &native.groups[g];
    for &x in &group.members {
        out.push_str(&format!("  BM v{x} = BMC(0);\n"));
        if matches!(native.insts[x], BlockInst::Toggle(_) | BlockInst::Escaped(_) | BlockInst::Fill(..)) {
            out.push_str(&format!("  uint32_t o{x} = 0u, co{x} = 0u;\n"));
        }
    }
    for (k, &(c, _)) in group.carries.iter().enumerate() {
        let BlockInst::Carry(init) = native.insts[c] else { unreachable!("a carry") };
        out.push_str(&format!("  BM gc{g}_{k} = v{init};\n"));
    }
    let bound = rerail_compiler::block::MAX_ROUNDS * GROUP;
    out.push_str(&format!("  for (uint32_t r{g} = 0u; r{g} < {bound}u; r{g}++) {{\n"));
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
            .replace(&format!("const BM v{x} = "), &format!("v{x} = "))
            .replace(&format!("const uint64_t v{x} = "), &format!("v{x} = "))
            .replace(&format!("uint32_t o{x} = 0u;"), &format!("o{x} = 0u;"))
            .replace(&format!("const uint32_t co{x} = "), &format!("co{x} = "));
        for line in text.lines() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    let changed: Vec<String> = group.carries.iter().enumerate().map(|(k, &(_, x))| format!("v{x} != gc{g}_{k}")).collect();
    out.push_str(&format!("    const bool more{g} = {};\n", changed.join(" || ")));
    for (k, &(_, x)) in group.carries.iter().enumerate() {
        out.push_str(&format!("    gc{g}_{k} = v{x};\n"));
    }
    // A vote is reached by every lane or none: in the serial form, whose threads run segments of
    // their own, it is the thread's own.
    let any = if lanes { format!("sg_any(more{g} __UKL_ARGS)") } else { format!("more{g}") };
    out.push_str(&format!("    if (!{any}) break;\n  }}\n"));
}

/// Generated UKL run: every output's words over `bytes`, as 64-bit words of the stream.
pub fn run_generated(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    run_generated_with(native, bytes, width, ClassStrategy::Planes).map(|(words, _)| words)
}

/// Generated UKL run with `class` by `strategy`: the outputs, and the GPU seconds of its
/// launches.
pub fn run_generated_with(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(Vec<Vec<u64>>, f64), Error> {
    run_generated_extract(native, bytes, width, strategy).map(|(run, seconds)| (run.outputs, seconds))
}

/// Generated UKL with the extract pass, a batch of one file: the outputs and each extract's
/// positions, and the GPU seconds of its launches.
pub fn run_generated_extract(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(BlockRun, f64), Error> {
    let mut run = run_batch(&[native], &[BatchFile { bytes, program: 0 }], width, Executor::Generated(strategy), SPINS)?;
    Ok((run.files.remove(0), run.seconds))
}

/// One launch: its kernel, its buffers and parameters (four words) by row name (a kernel binds
/// those of its rows it declares, as Metal's kernels took the buffer indices they declared), and
/// the grid as threadgroups of `threads`.
struct Pass {
    name: &'static str,
    kernel: lane::Kernel,
    buffers: Vec<(&'static str, Buffer)>,
    args: Vec<(&'static str, [u32; 4])>,
    groups: usize,
    threads: usize,
}

/// The passes in order, in one command; returns its device seconds.
fn encode_passes(passes: &[Pass]) -> Result<f64, Error> {
    let _scope = crate::launch::LaunchScope::enter();
    let mut command = lane::Command::new("Rerail block program");
    for pass in passes {
        let mut launch = command.launch(&pass.kernel, pass.name);
        for (name, buffer) in &pass.buffers {
            launch = launch.buffer_if(name, buffer);
        }
        for (name, value) in &pass.args {
            launch = launch.value_if(name, value);
        }
        launch.groups([pass.groups as u64, 1, 1], [pass.threads as u64, 1, 1]);
    }
    let times = command.run().inspect_err(|e| eprintln!("block program: {e:?}"))?;
    Ok(times.device)
}
