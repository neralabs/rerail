//! Block programs on the CPU JIT: Cranelift emits a `BlockNative` (the native IR) as one function
//! `tempo_block(bytes, rows, nrows, out, positions, counts)` over a batch of files (decision
//! 0044). Its loops run over the files and over each file's 64-byte blocks. A block's body is
//! straight-line bit operations (`body`). Each file starts with zero carries.
//!
//! The sinks are lowered in the loop, from the IR alone (`native_ir::BlockNative`):
//!
//! - **An output** at delay `d`. The stream's word `i - 1` is `(w[i-1] >> d) | (w[i] << (64 - d))`,
//!   so each block stores the word before it from a carried `w >> d`, and the row's end stores the
//!   last. At delay 0 a block stores its own word. Outside full blocks a word is first cut to the
//!   stream (`within` at the delay).
//! - **An extract.** The word's set bits as positions `64 i + bit - d`, at the running count, by
//!   simdjson's flattening: eight written whatever the count, then eight at a time while more are
//!   left. That loop is over set bits, eight at a time, never over bytes.
//! - **A total.** The running count, stored at the row's end. An extract's count is stored too.
//!
//! A file's mask words start at its row's `out`: block `i`'s words together, output `k`'s at
//! `i * outputs + k`. One spare block before them takes block 0's store of the word before it.
//!
//! The bitmask follows the architecture, as simdjson's does: on aarch64 its `to_bitmask` (each
//! byte weighted by its bit, then three rounds of pairwise adds, `addp`); elsewhere
//! `vhigh_bits` (one `pmovmskb` on x86).

use crate::Error;
use crate::block::{BatchFile, BlockRun};
use crate::native_cpu_lanes::{Lane, Lanes, Plan, all_ascii};
use crate::native_cpu_shape::Step;
use crate::block_native::{BlockInst, BlockNative, Pass};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, BlockArg, ConstantData, Endianness, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{Linkage, Module};

/// The batch's bytes, its rows (`Row`), their count, the mask words, the positions, the counts,
/// and the slots the passes store for each other (`native_ir::Passes`).
pub type Entry = unsafe extern "C" fn(*const u8, *const Row, u64, *mut u64, *mut u32, *mut u64, *mut u64);

/// A file of a batch, for the function: where its blocks start in the bytes (`FRONT` zero bytes
/// before them), how many there are, its length, the word at which its mask words start, where
/// its first extract's positions start and each extract's room, and where its counts start (one
/// per counted output: `CompiledBlock::counted`), and the word at which its slots start (block
/// `i`'s slot `k` at `i * slots + k`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub bytes: u64,
    pub blocks: u64,
    pub len: u64,
    pub out: u64,
    pub positions: u64,
    pub capacity: u64,
    pub counts: u64,
    pub scratch: u64,
}

/// A compiled block program.
pub struct CompiledBlock {
    _module: JITModule,
    /// One function per pass (`native_ir::Passes`), called in order over the batch: a program
    /// without `back_fill` is one.
    entries: Vec<Entry>,
    /// The words each block stores for the passes after.
    slots: usize,
    pub outputs: usize,
    delay: u8,
    pub extracts: Vec<usize>,
    totals: Vec<usize>,
    /// The outputs with a running count, extracts' and totals' together.
    counted: Vec<usize>,
    /// The full body branches once on an all-ASCII block (`native_cpu_lanes`).
    pub ascii_branch: bool,
    /// The gated regions' branches in the function, each body's counted (`native_cpu_lanes::Region`).
    pub gate_branches: usize,
    /// The fixed points' loops in the full blocks' bodies (`block::Group`), as the hot loop
    /// has them: one branch each, back to the round's head or on. The last block's body has its
    /// own.
    pub round_branches: usize,
    /// The last pass's function as Cranelift IR, before code generation; the counts above are
    /// its too.
    pub clif: String,
    /// Each pass's function, for the shape's checks (`native_ir::Passes`).
    pub passes: Vec<PassShape>,
    /// The machine code, disassembled.
    pub disassembly: String,
    /// The record pass's tables (`records`), which its code reads by address; its function is
    /// the last entry. A file's codes are the column after its extracts'.
    pub records: Option<RecordTables>,
    /// With `native_cpu_stream::KEEP_IR`: each function's IR as Cranelift was given it (its
    /// `FuncId`), and the entries' ids in order, for an ahead-of-time build (`clif_rust`).
    pub ir: Vec<(u32, cranelift_codegen::ir::Function)>,
    pub entry_ids: Vec<u32>,
}

/// A pass's function, as its checks read it: backward or forward, its Cranelift IR, and (a
/// forward pass) its ASCII test, gated regions' branches, fixed points' loops and extracts.
#[derive(Clone, Debug)]
pub struct PassShape {
    pub back: bool,
    pub clif: String,
    pub ascii_branch: bool,
    pub gate_branches: usize,
    pub round_branches: usize,
    pub extracts: usize,
    /// Its machine code, disassembled.
    pub disassembly: String,
    /// What is live where in its Cranelift IR, by the whole program's values
    /// (`native_cpu_pressure`): for the profile.
    pub pressure: crate::native_cpu_pressure::Pressure,
    /// The functions it calls, made from the same program: only the hot loop is inlined
    /// (`general`).
    pub outlined: Vec<Outlined>,
    /// What is live across each region call it keeps, integer and vector (`worth_calling`).
    pub region_calls: Vec<(usize, usize)>,
    /// The regions inlined after all: a call would have cost the hot loop.
    pub inlined_regions: usize,
}

/// A function a pass calls, made from the same program: its Cranelift IR and machine code.
#[derive(Clone, Debug)]
pub struct Outlined {
    pub clif: String,
    pub disassembly: String,
}

/// The running counts, as output indexes: each extract's, then one for all the totals (the first
/// total's index stands for them: `BlockProgram::totals`, one flag). A file's counts are laid out
/// in this order, so tier 2 reads its totals' flag from the count after the extracts'.
pub fn counted(native: &BlockNative) -> Vec<usize> {
    native.extracts.iter().chain(native.totals.first()).copied().collect()
}

impl CompiledBlock {
    /// Runs another build of the same code from now on: one entry a pass, as `entries`.
    ///
    /// # Safety
    /// `entries` must be this program's code (`clif_rust` of `ir`), in `entry_ids`' order.
    pub unsafe fn set_entries(&mut self, entries: Vec<Entry>) {
        assert_eq!(entries.len(), self.entries.len(), "one entry a pass");
        self.entries = entries;
    }

    /// The passes' count, the record pass's included (`Batch::call_pass`).
    pub fn pass_count(&self) -> usize {
        self.entries.len()
    }

    /// The record pass's tables' address, which its code holds as a constant.
    pub fn tables_address(&self) -> Option<i64> {
        self.records.as_ref().map(|t| t.words.as_ptr() as i64)
    }

    /// Every output's words over `bytes`; bits past the end are zero.
    /// The calls in the passes' IR: each of a function the same lowering made from the same
    /// program (`native_cpu_shape::check`), the only calls the machine code may have.
    pub fn own_calls(&self) -> usize {
        let call = |l: &str| {
            let t = l.trim();
            t.starts_with("call ") || t.contains(" = call ")
        };
        self.passes.iter().map(|p| p.clif.lines().filter(|l| call(l)).count() + p.outlined.iter().map(|o| o.clif.lines().filter(|l| call(l)).count()).sum::<usize>()).sum()
    }

    /// The passes' own machine code, the outlined functions' left out: where the hot loops are.
    pub fn loops_disassembly(&self) -> String {
        self.passes.iter().map(|p| p.disassembly.as_str()).collect()
    }

    pub fn run(&self, bytes: &[u8]) -> Vec<Vec<u64>> {
        self.extract(bytes).outputs
    }

    /// The outputs, positions and totals over `bytes`.
    pub fn extract(&self, bytes: &[u8]) -> BlockRun {
        self.run_files(&[bytes]).pop().expect("one file")
    }

    /// `extract` over several files in one call of the function (`Batch`).
    pub fn run_files(&self, files: &[&[u8]]) -> Vec<BlockRun> {
        let mut batch = Batch::new(self, files);
        batch.call(self);
        batch.results(self, files)
    }
}

/// A batch of files laid out for one call: their blocks end to end, each after `FRONT` zero
/// bytes; a row per file; and one buffer each for the mask words, positions and counts. The host
/// lays out and reads back; every word in them is the function's.
pub struct Batch {
    pub bytes: Vec<u8>,
    pub rows: Vec<Row>,
    pub out: Vec<u64>,
    pub positions: Vec<u32>,
    pub counts: Vec<u64>,
    pub scratch: Vec<u64>,
}

impl Batch {
    pub fn new(program: &CompiledBlock, files: &[&[u8]]) -> Self {
        let (mut bytes, mut rows) = (Vec::new(), Vec::with_capacity(files.len()));
        let (mut words, mut positions, mut counts, mut scratch) = (0u64, 0u64, 0u64, 0u64);
        for file in files {
            // The file's blocks and `delay` bytes past its end, zero beyond it.
            let blocks = (file.len() + usize::from(program.delay)).div_ceil(64);
            bytes.resize(bytes.len() + FRONT, 0);
            // An extract has at most one position per byte; the flattening writes up to seven
            // past the last.
            let capacity = file.len() as u64 + 8;
            words += program.outputs as u64;
            let row = Row { bytes: bytes.len() as u64, blocks: blocks as u64, len: file.len() as u64, out: words, positions, capacity, counts, scratch };
            scratch += (program.slots * blocks) as u64;
            // One copy, as simdjson's padded input: the file, then zeros to its last block's end.
            let at = bytes.len();
            bytes.extend_from_slice(file);
            bytes.resize(at + blocks * 64, 0);
            words += (program.outputs * blocks) as u64;
            // The extracts' positions, and with records a code a token after them.
            positions += (program.extracts.len() + usize::from(program.records.is_some())) as u64 * capacity;
            counts += program.counted.len() as u64;
            rows.push(row);
        }
        // 64 zero bytes after the last file: tier 2 reads a token's bytes a block at a time.
        bytes.resize(bytes.len() + 64, 0);
        Self {
            bytes,
            rows,
            out: vec![0u64; words as usize],
            positions: vec![0u32; positions as usize],
            counts: vec![0u64; counts as usize],
            scratch: vec![0u64; scratch as usize],
        }
    }

    pub fn call(&mut self, program: &CompiledBlock) {
        for entry in &program.entries {
            self.run(*entry);
        }
    }

    /// The program's pass `k` alone (the record pass last): for the profile, the passes before
    /// it run first.
    pub fn call_pass(&mut self, program: &CompiledBlock, k: usize) {
        self.run(program.entries[k]);
    }

    fn run(&mut self, entry: Entry) {
        // SAFETY: each row's blocks lie in `bytes` after `FRONT` zero bytes; its mask words (a
        // spare block, then each block's) in `out`; each extract's `capacity` positions in
        // `positions`; its counts in `counts`; its slots in `scratch`. The functions read and
        // write nothing else.
        {
            unsafe {
                entry(
                    self.bytes.as_ptr(),
                    self.rows.as_ptr(),
                    self.rows.len() as u64,
                    self.out.as_mut_ptr(),
                    self.positions.as_mut_ptr(),
                    self.counts.as_mut_ptr(),
                    self.scratch.as_mut_ptr(),
                )
            };
        }
    }

    fn results(&self, program: &CompiledBlock, files: &[&[u8]]) -> Vec<BlockRun> {
        self.rows
            .iter()
            .zip(files)
            .map(|(row, bytes)| {
                let (base, n) = (row.out as usize, bytes.len().div_ceil(64));
                let outputs = (0..program.outputs).map(|k| (0..n).map(|i| self.out[base + i * program.outputs + k]).collect()).collect();
                let count = |c: usize| self.counts[row.counts as usize + c];
                let positions = (0..program.extracts.len())
                    .map(|e| {
                        let at = (row.positions + e as u64 * row.capacity) as usize;
                        self.positions[at..at + count(e) as usize].to_vec()
                    })
                    .collect();
                let totals = if program.totals.is_empty() { Vec::new() } else { vec![u64::from(count(program.extracts.len()) != 0)] };
                let codes = match &program.records {
                    Some(t) => {
                        let n = count(usize::from(t.starts)).min(count(usize::from(t.lasts))) as usize;
                        let at = (row.positions + program.extracts.len() as u64 * row.capacity) as usize;
                        self.positions[at..at + n].to_vec()
                    }
                    None => Vec::new(),
                };
                BlockRun { outputs, positions, totals, codes }
            })
            .collect()
    }
}

/// Cranelift over a batch (decision 0066): each file's run under its program, in the files'
/// order; one call of each program's function over all of that program's files.
pub fn run_batch(programs: &[&CompiledBlock], files: &[BatchFile<'_>]) -> Vec<BlockRun> {
    let mut out: Vec<Option<BlockRun>> = vec![None; files.len()];
    for (p, program) in programs.iter().enumerate() {
        let mine: Vec<usize> = (0..files.len()).filter(|&i| files[i].program == p).collect();
        let bytes: Vec<&[u8]> = mine.iter().map(|&i| files[i].bytes).collect();
        for (i, run) in mine.into_iter().zip(program.run_files(&bytes)) {
            out[i] = Some(run);
        }
    }
    out.into_iter().map(|run| run.expect("every file names a program of the batch")).collect()
}

impl CompiledBlock {
    /// The function's CPU time over `bytes` (thread user + system time), best of `runs`, the
    /// input already in blocks: the cost of the compiled code alone.
    pub fn cpu_seconds(&self, bytes: &[u8], runs: usize) -> f64 {
        self.batch_cpu_seconds(&[bytes], runs)
    }

    /// `cpu_seconds` over a batch of files, in one call.
    pub fn batch_cpu_seconds(&self, files: &[&[u8]], runs: usize) -> f64 {
        let mut batch = Batch::new(self, files);
        (0..runs)
            .map(|_| {
                let start = thread_cpu();
                batch.call(self);
                thread_cpu() - start
            })
            .fold(f64::MAX, f64::min)
    }
}

/// The thread's CPU time (user + system), in seconds (0 where there is no thread CPU clock: Windows).
pub fn thread_cpu() -> f64 {
    #[cfg(unix)]
    {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `t` is a valid timespec to write.
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

/// Zero bytes before the first block: `pair` and `plane_before` load each block one byte back,
/// and the byte before the stream is 0x00.
pub const FRONT: usize = 16;


/// A field of the carried state: bits `pos..pos + width` of loop-carried word `word`. A `top`
/// field keeps the top `width` bits of instruction `at`'s word (for its `prev`s, and a `toggle`'s
/// or `fill`'s own carry, its top bit); a `flag` field an `escaped`'s borrow.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    at: usize,
    width: u8,
    word: usize,
    pos: u8,
}

/// The carried state's layout (decision 0070): its fields, packed first-fit into words.
#[derive(Debug)]
pub struct State {
    fields: Vec<Field>,
    /// The values on the byte vectors (`native_cpu_lanes`).
    pub plan: Plan,
    /// Each gated region's carried fields: per word, the bits its values carry.
    pub region_fields: Vec<Vec<(usize, u64)>>,
    /// Each instruction's top field and flag field, if it has one.
    top: Vec<Option<usize>>,
    flag: Vec<Option<usize>>,
    /// Each walk's state field (`WalkState`), by walk: kept on its hidden value.
    walk: Vec<Option<usize>>,
    pub words: usize,
}

/// A walk's state from one block to the next (`build_walk`): bits 0..4 the kind of region open
/// at the block's end, plus one (0: none); bits 4..6 the hidden bytes it hides past its end (a
/// closer's bytes, an empty literal's); bit `8 + k` that kind `k`'s closer has no byte left in
/// the file (its `else` is taken from there on, with no scan).
pub struct WalkState;

impl WalkState {
    pub const SKIP: u8 = 4;
    pub const NONE: u8 = 8;
    /// The field's width for a walk of `kinds` kinds.
    pub fn width(kinds: usize) -> u8 {
        Self::NONE + kinds as u8
    }
}

impl State {
    /// The carried fields, as (instruction, bits, word): what each loop-carried word holds.
    pub fn fields(&self) -> Vec<(usize, u8, usize)> {
        self.fields.iter().map(|f| (f.at, f.width, f.word)).collect()
    }

    pub fn of(native: &BlockNative) -> Self {
        let n = native.insts.len();
        let plan = Plan::of(native);
        let mut back = vec![0u8; n];
        for (at, inst) in native.insts.iter().enumerate() {
            // A `prev` on the byte vectors reads the bytes before the block, not a carried word.
            if let BlockInst::Prev(a, k) = *inst
                && plan.carries(at)
            {
                back[usize::from(a)] = back[usize::from(a)].max(k);
            }
        }
        // (instruction, width, what): 0 its top bits, 1 an escape's flag, 2 + w walk `w`'s state.
        let mut wanted: Vec<(usize, u8, usize)> = Vec::new();
        for (at, inst) in native.insts.iter().enumerate() {
            match inst {
                BlockInst::Toggle(_) | BlockInst::Fill(..) => wanted.push((at, back[at].max(1), 0)),
                BlockInst::Escaped(_) => {
                    wanted.push((at, 1, 1));
                    if back[at] > 0 {
                        wanted.push((at, back[at], 0));
                    }
                }
                _ if back[at] > 0 => wanted.push((at, back[at], 0)),
                _ => {}
            }
        }
        for (w, walk) in native.walks.iter().enumerate().filter(|(_, w)| !w.kinds.is_empty()) {
            wanted.push((usize::from(walk.hidden), WalkState::width(walk.kinds.len()), 2 + w));
        }
        let mut used: Vec<u8> = Vec::new();
        let (mut fields, mut top, mut flag, mut walk) = (Vec::new(), vec![None; n], vec![None; n], vec![None; native.walks.len()]);
        for (at, width, what) in wanted {
            let word = match used.iter().position(|&u| u + width <= 64) {
                Some(w) => w,
                None => {
                    used.push(0);
                    used.len() - 1
                }
            };
            fields.push(Field { at, width, word, pos: used[word] });
            used[word] += width;
            match what {
                0 => top[at] = Some(fields.len() - 1),
                1 => flag[at] = Some(fields.len() - 1),
                w => walk[w - 2] = Some(fields.len() - 1),
            }
        }
        let mut region_fields: Vec<Vec<(usize, u64)>> = vec![Vec::new(); plan.regions.len()];
        for field in &fields {
            // A value zero without the gate, or the gate itself: a `prev` of it reads the last
            // block's gate.
            for r in (0..plan.regions.len()).filter(|&r| plan.regions[r].gate == field.at || plan.regions[r].zero.contains(&field.at)) {
                let bits = if field.width == 64 { u64::MAX } else { ((1u64 << field.width) - 1) << field.pos };
                match region_fields[r].iter_mut().find(|x| x.0 == field.word) {
                    Some(x) => x.1 |= bits,
                    None => region_fields[r].push((field.word, bits)),
                }
            }
        }
        State { fields, plan, region_fields, top, flag, walk, words: used.len() }
    }
}

/// The constants every body uses, made once in the function's entry: the nibble mask, the
/// bitmask's weights, and simdjson's even bits.
pub struct Consts {
    pub(crate) nibble: Value,
    pub(crate) weights: Value,
    even: Value,
}

impl Consts {
    pub fn make(f: &mut FunctionBuilder<'_>) -> Self {
        let even = f.ins().iconst(types::I64, crate::block::carry::EVEN as i64);
        let nibble = {
            let c = f.ins().iconst(types::I8, 0x0f);
            f.ins().splat(types::I8X16, c)
        };
        let weights = {
            const WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
            let handle = f.func.dfg.constants.insert(ConstantData::from(&WEIGHTS[..]));
            f.ins().vconst(types::I8X16, handle)
        };
        Consts { nibble, weights, even }
    }
}

/// Where a body runs: its block's index in the stream, the stream's length, the block's first
/// byte, the carried state in, whether the block is full (inside the stream and not the first:
/// every `within` all ones).
pub struct At<'a> {
    pub i: Value,
    pub len: Value,
    pub base: Value,
    pub packed_in: &'a [Value],
    pub full: bool,
    /// The ASCII body: the block and the 16 bytes before it are all below 0x80.
    pub ascii: bool,
    /// The block's byte chunks, when already loaded (for the ASCII test).
    pub bytes: Option<[Option<Value>; 5]>,
    /// The block's row of slot words, and the slots per block (`native_ir::Passes`).
    pub scratch: Value,
    pub slots: u16,
    /// Before the last pass: each output's slot, stored where the body makes it.
    pub slot_of: &'a [u16],
    /// The last pass's mask stores made where the body makes their values (`Sinks`).
    pub sinks: Option<Sinks<'a>>,
    /// The gated regions' functions, each called from its branch (`Emit::region`).
    pub calls: Option<&'a RegionCalls>,
    /// The walks' functions (`native_cpu_walk`), each called once a block.
    pub walks: Option<&'a crate::native_cpu_walk::WalkCalls>,
}

/// The gated regions' functions of a pass: each region is computed by a function of its own,
/// made from the same program, that its branch calls. Only the hot loop is inlined: a region
/// runs in few blocks, and its code inline would sit between the loop's. A function is built
/// where its region is first lowered; a region in both the ASCII body and the other is one.
pub struct RegionCalls {
    ids: Vec<cranelift_module::FuncId>,
    signature: cranelift_codegen::ir::Signature,
    built: std::cell::RefCell<Vec<Option<cranelift_codegen::ir::Function>>>,
    /// The regions inlined after all: a call of theirs would cost the hot loop more than it
    /// saves (`worth_calling`).
    inline: std::cell::RefCell<Vec<bool>>,
}

/// A region function's signature: its words' address (in, then out), the block's first byte,
/// its row of slots, its index, the stream's length.
fn region_signature(module: &JITModule) -> cranelift_codegen::ir::Signature {
    let mut signature = module.make_signature();
    for _ in 0..5 {
        signature.params.push(AbiParam::new(types::I64));
    }
    signature
}

/// The last pass's outputs a body stores where it makes them: each output not counted (an
/// extract's or a total's word is read after the body), cut to the stream outside full
/// blocks, at `dst`; a delayed one by `delayed_store`. The body gives back in its place
/// nothing (a zero).
#[derive(Clone, Copy)]
pub struct Sinks<'a> {
    pub dst: Value,
    pub stride: i64,
    pub early: &'a [bool],
}

/// Output `k`'s word at delay `d > 0`, stored without holding a word across the loop. The
/// stream's word `i - 1` is `(w[i-1] >> d) | (w[i] << (64 - d))`. Block `i - 1` stored `w[i-1]`
/// as it is, in its own place. Block `i` reads it back from there (stored one iteration ago),
/// stores the stream's word `i - 1` over it, and stores `w[i]` in its own place. The row's
/// end shifts the last (`forward`). A held word would be live through the whole loop, one per
/// delayed output: a register each, or a spill.
fn delayed_store(f: &mut FunctionBuilder<'_>, dst: Value, stride: i64, k: usize, d: u8, word: Value) {
    let before = 8 * k as i32 - stride as i32;
    let last = f.ins().load(types::I64, MemFlags::trusted(), dst, before);
    let down = f.ins().ushr_imm(last, i64::from(d));
    let up = f.ins().ishl_imm(word, 64 - i64::from(d));
    let joined = f.ins().bor(down, up);
    f.ins().store(stores_flags(), joined, dst, before);
    f.ins().store(stores_flags(), word, dst, 8 * k as i32);
}

/// One block's body: every instruction as straight-line bit operations. Gives each output's
/// value (the value itself, at its delay, not yet cut to the stream) and the carried state out.
///
/// In a full block the plan's gated regions (`native_cpu_lanes::Region`) each run behind one
/// branch: when the gate has no bit and the region's carried bits came in zero, its values are
/// zero and none of it is computed. The builder is left in the block after the last region.
/// A fixed point's group (`block::Group`) is one loop: its round's head takes the carries and a
/// count, its members are made in it, and it goes on once no carry changed (at most
/// `block::MAX_ROUNDS` times). The loop's heads are given back for the shape's check.
pub fn body(f: &mut FunctionBuilder<'_>, native: &BlockNative, state: &State, c: &Consts, at: At<'_>) -> (Vec<Value>, Vec<Value>, usize, Vec<cranelift_codegen::ir::Block>) {
    let plan = &state.plan;
    let n = native.insts.len();
    let lanes = Lanes::new(&mut *f, native, plan, at.base, at.i, at.len, !at.full, at.ascii, at.bytes, c.nibble);
    // The block's row of slots, a loop parameter: each `Load` is one load at its slot's offset.
    let row = at.scratch;
    let mut e = Emit {
        sinks: at.sinks,
        sunk: vec![None; native.outputs.len()],
        native,
        state,
        full: at.full,
        i: at.i,
        len: at.len,
        packed_in: at.packed_in,
        weights: c.weights,
        even: c.even,
        live: Vec::new(),
        masked: Vec::new(),
        syms: vec![Sym::Known(0); n],
        values: vec![None; n],
        packed_out: vec![None; state.words],
        consts: vec![0; state.words],
        lanes,
        rounds: Vec::new(),
        row,
        calls: at.calls,
        walks: at.walks,
        walked: vec![false; native.walks.len()],
    };
    (e.live, e.masked) = live(native, state, at.full);
    let mut grouped: Vec<Option<usize>> = vec![None; n];
    for (g, group) in native.groups.iter().enumerate() {
        for &m in &group.members {
            grouped[m] = Some(g);
        }
    }
    let regions = if at.full { plan.regions.len() } else { 0 };
    // Each slot word to store (before the last pass), by the value that is it.
    let mut store_at: Vec<Vec<u16>> = vec![Vec::new(); n];
    let mut sink_at: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, &v) in native.outputs.iter().enumerate() {
        if let Some(&slot) = at.slot_of.get(k) {
            store_at[usize::from(v)].push(slot);
        }
    }
    // The last pass's outputs to store where they are made.
    if let Some(sk) = at.sinks {
        for (k, &v) in native.outputs.iter().enumerate() {
            if sk.early[k] {
                sink_at[usize::from(v)].push(k);
            }
        }
    }
    let gates = demand_order(&mut e, &mut *f, &grouped, &store_at, &sink_at, regions);
    let rounds = std::mem::take(&mut e.rounds);
    let (outputs, packed) = e.finish(&mut *f);
    (outputs, packed, gates, rounds)
}

/// A body's values in the order they are needed: each just before the first value that reads
/// it, a fixed point's loop just before its first member is read, a gated region (the first
/// `regions` of the plan's, in a full block) just before the first value that reads what it
/// gives, from the values no other reads (the outputs, the carried ones), first to last. In the
/// program's order a value made early and read late lives through all between, and spills.
/// Each slot word is stored where it is made. A value on the vectors reads another's bytes, made
/// on demand, not its mask: it waits on no value on the vectors (as `native_cpu_lanes::regions`
/// plans). Gives the regions' branches.
fn demand_order(e: &mut Emit<'_, '_>, f: &mut FunctionBuilder<'_>, grouped: &[Option<usize>], store_at: &[Vec<u16>], sink_at: &[Vec<usize>], regions: usize) -> usize {
    let native = e.native;
    let plan = &e.state.plan;
    let n = native.insts.len();
    let needs = |o: usize, x: usize| !(plan.lanes[o] && plan.lanes[x]);
    // Read by a value that makes it first: one on the vectors read only on the vectors (an output
    // made a mask, say) is a root of its own.
    let mut read = vec![false; n];
    for (x, inst) in native.insts.iter().enumerate() {
        if e.live[x] {
            for o in inst.operands().into_iter().filter(|&o| needs(o, x)) {
                read[o] = true;
            }
        }
    }
    #[derive(Clone, Copy)]
    enum Node {
        Value(usize),
        Group(usize),
        Region(usize),
    }
    #[derive(Clone, Copy, PartialEq)]
    enum R {
        Waiting,
        Visiting,
        Made,
        /// Needed by a value it waits on (two regions each reading the other's): its values made
        /// in place, without its branch.
        Dissolved,
    }
    let mut region = vec![R::Waiting; regions];
    let region_of = |x: usize, region: &[R]| plan.region_of[x].filter(|&r| r < regions && region[r] != R::Dissolved);
    let mut done = vec![false; n];
    let mut ran = vec![false; native.groups.len()];
    let mut gates = 0;
    let mut stack: Vec<(Node, bool)> = Vec::new();
    // Each walk's call first, before the body's other values are live: what is live across a
    // call stays in a callee-saved register, or spills.
    let walks_first: Vec<usize> = native.walks.iter().filter(|w| !w.kinds.is_empty() && e.live[usize::from(w.hidden)]).map(|w| usize::from(w.hidden)).collect();
    for root in walks_first.iter().copied().chain(0..n) {
        if done[root] || (e.live[root] && read[root] && !walks_first.contains(&root)) {
            continue;
        }
        stack.push((Node::Value(root), false));
        while let Some((node, ready)) = stack.pop() {
            match node {
                Node::Value(x) if done[x] => {}
                Node::Value(x) => match (grouped[x], region_of(x, &region)) {
                    (Some(g), _) => stack.push((Node::Group(g), false)),
                    (None, Some(r)) => stack.push((Node::Region(r), false)),
                    (None, None) if ready => {
                        e.emit(&mut *f, x);
                        done[x] = true;
                        e.store(&mut *f, x, &store_at[x]);
                        e.sink(&mut *f, x, &sink_at[x]);
                    }
                    (None, None) => {
                        stack.push((node, true));
                        if e.live[x] {
                            for o in native.insts[x].operands().into_iter().rev().filter(|&o| needs(o, x)) {
                                stack.push((Node::Value(o), false));
                            }
                        }
                    }
                },
                Node::Group(g) if ran[g] => {}
                Node::Group(g) if ready => {
                    e.rounds(&mut *f, g);
                    ran[g] = true;
                    for &m in &native.groups[g].members {
                        done[m] = true;
                        e.store(&mut *f, m, &store_at[m]);
                        e.sink(&mut *f, m, &sink_at[m]);
                    }
                }
                Node::Group(g) => {
                    stack.push((node, true));
                    for &m in native.groups[g].members.iter().rev() {
                        for o in native.insts[m].operands().into_iter().rev() {
                            if grouped[o] != Some(g) {
                                stack.push((Node::Value(o), false));
                            }
                        }
                    }
                }
                Node::Region(r) => match region[r] {
                    R::Made => {}
                    R::Dissolved => {
                        // Its values, each as any other.
                        for &m in plan.regions[r].members.iter().rev() {
                            stack.push((Node::Value(m), false));
                        }
                    }
                    R::Visiting if ready => {
                        e.region(&mut *f, r);
                        gates += 1 + usize::from(plan.regions[r].pre.is_some());
                        region[r] = R::Made;
                        for &m in &plan.regions[r].members {
                            done[m] = true;
                            e.store(&mut *f, m, &store_at[m]);
                            e.sink(&mut *f, m, &sink_at[m]);
                        }
                    }
                    R::Visiting => {
                        // Needed while its operands are made: by one of them.
                        region[r] = R::Dissolved;
                        stack.push((node, false));
                    }
                    R::Waiting => {
                        region[r] = R::Visiting;
                        stack.push((node, true));
                        let reg = &plan.regions[r];
                        let mut need: Vec<usize> = vec![reg.pre.unwrap_or(reg.gate)];
                        for &m in &reg.members {
                            for o in native.insts[m].operands() {
                                if plan.region_of[o] != Some(r) && needs(o, m) {
                                    need.push(o);
                                }
                            }
                        }
                        for &o in need.iter().rev() {
                            stack.push((Node::Value(o), false));
                        }
                    }
                },
            }
        }
    }
    gates
}

/// The values a body must make: the outputs, the carried ones (a known one's carry is a constant)
/// and what they read. In a full block a value known in every full block reads nothing. Second,
/// the byte-local values made a mask: those a live value reads as one (`native_cpu_lanes`).
fn live(native: &BlockNative, state: &State, full: bool) -> (Vec<bool>, Vec<bool>) {
    let n = native.insts.len();
    let known = |x: usize| full && !state.plan.lanes[x] && state.plan.full[x].is_some();
    let mut live = vec![false; n];
    for &v in &native.outputs {
        live[usize::from(v)] = true;
    }
    let mut masked = vec![false; n];
    // A fixed point's members are all made: each carry's next is read by the next round.
    for group in &native.groups {
        for &m in &group.members {
            live[m] = true;
        }
    }
    for x in (0..n).rev() {
        live[x] |= state.top[x].is_some() || state.flag[x].is_some() || state.walk.iter().flatten().any(|&k| state.fields[k].at == x);
        masked[x] |= state.plan.mask[x] && (live[x] && (native.outputs.contains(&(x as u16)) || state.top[x].is_some()));
        if live[x] && !known(x) {
            for o in native.insts[x].operands() {
                live[o] = true;
                masked[o] |= state.plan.mask[o] && !state.plan.lanes[x];
            }
        }
    }
    (live, masked)
}

/// The body's emitter: each value's symbol, the carried state being packed, the byte vectors.
#[derive(Clone)]
struct Emit<'a, 'b> {
    sinks: Option<Sinks<'b>>,
    /// Each output's value given back where the body stored it (`Sinks`).
    sunk: Vec<Option<Value>>,
    native: &'a BlockNative,
    state: &'a State,
    full: bool,
    i: Value,
    len: Value,
    packed_in: &'b [Value],
    weights: Value,
    even: Value,
    /// The values something reads: in a full block, one whose readers all fold is not made.
    live: Vec<bool>,
    masked: Vec<bool>,
    /// Each instruction's value, symbolic for the bit logic: a constant folds, and a class
    /// stays complemented (as its compare makes it) until a use needs it plain, so that an
    /// `and` with it is one `bic` and a `not` of it is free.
    syms: Vec<Sym>,
    values: Vec<Option<Value>>,
    /// The state carried out, word by word: each field ORed in as soon as its value is made,
    /// so that no value stays live to the back edge for its carry. Known bits are gathered
    /// apart, one constant per word.
    packed_out: Vec<Option<Value>>,
    consts: Vec<u64>,
    /// The byte-local values, on the block's byte vectors (`native_cpu_lanes`): made a mask only
    /// where something else reads them.
    lanes: Lanes<'a>,
    /// The heads of the fixed points' loops made so far.
    rounds: Vec<cranelift_codegen::ir::Block>,
    /// The block's row of slots (`body`).
    row: Value,
    /// The regions' functions (`RegionCalls`); none: each region inline.
    calls: Option<&'b RegionCalls>,
    /// The walks' functions, and which walks this body has called.
    walks: Option<&'b crate::native_cpu_walk::WalkCalls>,
    walked: Vec<bool>,
}

impl Emit<'_, '_> {
    fn put(&mut self, f: &mut FunctionBuilder<'_>, field: Field, bits: Value) {
        let placed = if field.pos == 0 { bits } else { f.ins().ishl_imm(bits, i64::from(field.pos)) };
        self.packed_out[field.word] = Some(match self.packed_out[field.word] {
            None => placed,
            Some(acc) => f.ins().bor(acc, placed),
        });
    }

    /// Bits `from..from + n` of the state carried in, as the low bits of a word.
    fn read(&self, f: &mut FunctionBuilder<'_>, field: &Field, from: u8, n: u8) -> Value {
        let shift = field.pos + from;
        let w = self.packed_in[field.word];
        let x = if shift == 0 { w } else { f.ins().ushr_imm(w, i64::from(shift)) };
        if field.pos + from + n == 64 { x } else { f.ins().band_imm(x, ((1u64 << n) - 1) as i64) }
    }

    /// Value `at`'s top bits into its field, if it has one.
    fn carry(&mut self, f: &mut FunctionBuilder<'_>, at: usize) {
        let Some(k) = self.state.top[at] else { return };
        let field = self.state.fields[k];
        if let Sym::Known(c) = self.syms[at] {
            self.consts[field.word] |= (c >> (64 - u32::from(field.width))) << field.pos;
            return;
        }
        let value = plain(&mut *f, &self.syms, &mut self.values, at);
        let top = f.ins().ushr_imm(value, 64 - i64::from(field.width));
        self.put(&mut *f, field, top);
    }

    fn set(&mut self, at: usize, sym: Sym) {
        self.syms[at] = sym;
        self.values[at] = match sym {
            Sym::Value(v, false) => Some(v),
            _ => None,
        };
    }

    fn emit(&mut self, f: &mut FunctionBuilder<'_>, at: usize) {
        // Each instruction made for value `at` is marked with it (`native_cpu_pressure`).
        f.set_srcloc(cranelift_codegen::ir::SourceLoc::new(at as u32 + 1));
        let native = self.native;
        let state = self.state;
        let plan = &state.plan;
        let inst = native.insts[at];
        if !self.live[at] {
            self.set(at, Sym::Known(0));
            return;
        }
        // A walk's values are made together, by its call; what it reads, by the call too.
        match inst {
            BlockInst::Walk { walk, .. } => {
                if !self.walked[usize::from(walk)] {
                    self.walk(&mut *f, usize::from(walk));
                }
                return;
            }
            BlockInst::Args(..) => {
                self.set(at, Sym::Known(0));
                return;
            }
            _ => {}
        }
        if plan.lanes[at] {
            let sym = if self.masked[at] {
                let parts: Vec<Lane> = (1..5).map(|c| self.lanes.get(&mut *f, at, c)).collect();
                self.lanes.mask(&mut *f, &parts, self.weights)
            } else {
                // Read on the vectors only.
                Sym::Known(0)
            };
            self.set(at, sym);
            self.carry(&mut *f, at);
            return;
        }
        // In a full block, a value of constants and `within` is known, a `prev` of one included.
        if self.full
            && let Some(c) = plan.full[at]
        {
            self.set(at, Sym::Known(c));
            self.carry(&mut *f, at);
            return;
        }
        let logic = |x: u16| self.syms[usize::from(x)];
        let folded = match inst {
            BlockInst::Const(c) => Some(Sym::Known(c)),
            BlockInst::Within(_) if self.full => Some(Sym::Known(u64::MAX)),
            BlockInst::Not(a) => Some(logic(a).not()),
            BlockInst::And(a, b) => Some(Sym::and(&mut *f, logic(a), logic(b))),
            BlockInst::AndNot(a, b) => Some(Sym::and(&mut *f, logic(a), logic(b).not())),
            BlockInst::Or(a, b) => Some(Sym::and(&mut *f, logic(a).not(), logic(b).not()).not()),
            BlockInst::Xor(a, b) => Some(Sym::xor(&mut *f, logic(a), logic(b))),
            _ => None,
        };
        if let Some(sym) = folded {
            self.set(at, sym);
            // A carried value is made plain where it is made.
            self.carry(&mut *f, at);
            return;
        }
        let operands = inst.operands();
        let given: Vec<Value> = operands.iter().map(|&x| plain(&mut *f, &self.syms, &mut self.values, x)).collect();
        let v = |x: u16| given[operands.iter().position(|&o| o == usize::from(x)).expect("an operand")];
        let (i, len, even) = (self.i, self.len, self.even);
        let value = match inst {
            // The bits below the stream's end: t = clamp(len - 64 i, 0, 64) ones, by two
            // shifts so that t = 64 needs no shift by 64 (`block::within`). At a delay `d`,
            // the bits below `len + d` and not below `d` (`block::within_at`).
            // The period's bits shifted up to the block's first byte of the residue:
            // `(r + k - (64 i) % k) % k`, two remainders by a constant.
            BlockInst::Phase(k, r) => {
                let at = f.ins().ishl_imm(i, 6);
                let m = f.ins().urem_imm(at, i64::from(k));
                let up = f.ins().iconst(types::I64, i64::from(r) + i64::from(k));
                let up = f.ins().isub(up, m);
                let first = f.ins().urem_imm(up, i64::from(k));
                let bits = f.ins().iconst(types::I64, crate::block::period_bits(k) as i64);
                f.ins().ishl(bits, first)
            }
            // Bit `j` of each byte's position: below 6 the word's pattern; from 6 on, bit
            // `j - 6` of the block's index, spread to the word (`block::position`).
            BlockInst::Position(j) if j < 6 => f.ins().iconst(types::I64, crate::block::position(0, j) as i64),
            BlockInst::Position(j) => {
                let bit = f.ins().ushr_imm(i, i64::from(j - 6));
                let bit = f.ins().band_imm(bit, 1);
                f.ins().ineg(bit)
            }
            BlockInst::Within(d) => within_word(&mut *f, i, len, d),
            // A slot an earlier pass stored: the block's word, as that pass made it.
            BlockInst::Load(k) => f.ins().load(types::I64, MemFlags::trusted().with_readonly(), self.row, 8 * i32::from(k)),
            BlockInst::BackFill(..) => unreachable!("a pass of its own (`native_ir::passes`)"),
            BlockInst::Carry(_) => unreachable!("a carry is its round's head's parameter"),
            BlockInst::Walk { .. } | BlockInst::Args(..) => unreachable!("made by its walk's call"),
            BlockInst::Cmp { .. } | BlockInst::Bit { .. } => unreachable!("a row program's (`native_cpu_rows`)"),
            BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Pair { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) => unreachable!("on the vectors"),
            BlockInst::Const(_) | BlockInst::Not(_) | BlockInst::And(..) | BlockInst::Or(..) | BlockInst::Xor(..) | BlockInst::AndNot(..) => unreachable!("folded"),
            BlockInst::Prev(a, k) => {
                // The top `k` of the bits the field keeps of `a`'s previous word.
                let field = state.fields[state.top[usize::from(a)].expect("a looked-back value has a field")];
                let up = f.ins().ishl_imm(v(a), i64::from(k));
                let halo = self.read(&mut *f, &field, field.width - k, k);
                f.ins().bor(up, halo)
            }
            BlockInst::Toggle(a) => {
                // prefix XOR by six shift-XOR rounds (Cranelift has no carry-less multiply),
                // then the parity carried in: the top bit of the previous word, all ones or zero.
                let field = state.fields[state.top[at].expect("a toggle has a field")];
                let bit = field.pos + field.width - 1;
                let parity = f.ins().ishl_imm(self.packed_in[field.word], i64::from(63 - bit));
                let parity = f.ins().sshr_imm(parity, 63);
                let mut x = v(a);
                for k in [1, 2, 4, 8, 16, 32] {
                    let up = f.ins().ishl_imm(x, k);
                    x = f.ins().bxor(x, up);
                }
                f.ins().bxor(x, parity)
            }
            BlockInst::Escaped(a) => {
                // simdjson's find_escaped_branchless (`block::carry::escaped`).
                let flag = state.fields[state.flag[at].expect("an escaped has a field")];
                let cin = self.read(&mut *f, &flag, 0, 1);
                let run = v(a);
                let b = f.ins().band_not(run, cin);
                let up = f.ins().ishl_imm(b, 1);
                let follows = f.ins().bor(up, cin);
                let odd = f.ins().band_not(b, even);
                let odd_starts = f.ins().band_not(odd, follows);
                let (sequences, overflow) = f.ins().uadd_overflow(odd_starts, b);
                let invert = f.ins().ishl_imm(sequences, 1);
                let flipped = f.ins().bxor(even, invert);
                let escaped = f.ins().band(flipped, follows);
                let borrow = f.ins().uextend(types::I64, overflow);
                self.put(&mut *f, flag, borrow);
                escaped
            }
            BlockInst::Fill(s, e) => {
                // `block::carry::fill`: the carry into each bit of `s + (s | !e) + cin` is
                // the fill coming into that byte. The borrow carried in: the previous word's
                // top bit.
                let field = state.fields[state.top[at].expect("a fill has a field")];
                let cin = self.read(&mut *f, &field, field.width - 1, 1);
                let (start, end) = (v(s), v(e));
                let b = f.ins().bor_not(start, end);
                let sum = f.ins().iadd(start, b);
                let sum = f.ins().iadd(sum, cin);
                let into = f.ins().bxor(sum, start);
                let into = f.ins().bxor(into, b);
                let open = f.ins().band_not(into, end);
                f.ins().bor(start, open)
            }
        };
        self.set(at, Sym::Value(value, false));
        self.carry(&mut *f, at);
    }

    /// Walk `w`'s call (`native_cpu_walk`): its state and inputs stored to a slot, its function
    /// called, its values and state read back. Every value it makes is set, and carried.
    fn walk(&mut self, f: &mut FunctionBuilder<'_>, w: usize) {
        let native = self.native;
        let walk = &native.walks[w];
        let calls = self.walks.expect("a walk's function");
        let field = self.state.fields[self.state.walk[w].expect("a walk's state field")];
        let made = walk.made();
        let n = crate::native_cpu_walk::layout(walk.kinds.len(), made.len());
        let slot = f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * n as u32, 3));
        let state_in = self.read(&mut *f, &field, 0, field.width);
        f.ins().stack_store(state_in, slot, 0);
        for (k, kind) in walk.kinds.iter().enumerate() {
            let o = plain(&mut *f, &self.syms, &mut self.values, usize::from(kind.opener));
            f.ins().stack_store(o, slot, 8 * (1 + 2 * k as i32));
            let c = plain(&mut *f, &self.syms, &mut self.values, usize::from(kind.closer));
            f.ins().stack_store(c, slot, 8 * (2 + 2 * k as i32));
        }
        if calls.built.borrow()[w].is_none() {
            let func = crate::native_cpu_walk::build_walk(native, w, calls).expect("a walk the program verified");
            calls.built.borrow_mut()[w] = Some(func);
        }
        let address = f.ins().stack_addr(types::I64, slot, 0);
        let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, calls.ids[w].as_u32()));
        let signature = f.import_signature(calls.signature.clone());
        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
        let base = self.lanes.base();
        f.ins().call(callee, &[address, base, self.i, self.len]);
        let state_out = f.ins().stack_load(types::I64, slot, 0);
        self.put(&mut *f, field, state_out);
        self.walked[w] = true;
        for (j, &v) in made.iter().enumerate() {
            self.set(usize::from(v), Sym::Slot(slot, 8 * (1 + 2 * walk.kinds.len() + j) as i32, false));
            self.carry(&mut *f, usize::from(v));
        }
    }

    /// Region `r` behind its branch: computed where its gate has a bit or its carried bits came
    /// in nonzero; zero elsewhere. The values read after it, and the carried words it writes,
    /// meet in the block after.
    fn region(&mut self, f: &mut FunctionBuilder<'_>, r: usize) {
        let state = self.state;
        let reg = &state.plan.regions[r];
        // Read after the region: by a value outside it, or as an output.
        let outside: Vec<usize> = reg
            .members
            .iter()
            .copied()
            .filter(|&m| self.native.outputs.contains(&(m as u16)) || (0..self.native.insts.len()).any(|u| state.plan.region_of[u] != Some(r) && self.native.insts[u].operands().contains(&m)))
            .collect();
        let words: Vec<usize> = state.region_fields[r].iter().map(|&(w, _)| w).collect();
        // The region's carried bits that came in, ORed.
        let mut carried: Option<Value> = None;
        for &(w, m) in &state.region_fields[r] {
            let bits = f.ins().band_imm(self.packed_in[w], m as i64);
            carried = Some(match carried {
                Some(c) => f.ins().bor(c, bits),
                None => bits,
            });
        }
        let (active, skip, after) = (f.create_block(), f.create_block(), f.create_block());
        for _ in 0..outside.len() + words.len() {
            f.append_block_param(after, types::I64);
        }
        let test = |e: &mut Self, f: &mut FunctionBuilder<'_>, x: usize| {
            let v = plain(&mut *f, &e.syms, &mut e.values, x);
            carried.map_or(v, |c| f.ins().bor(v, c))
        };
        let first = test(self, &mut *f, reg.pre.unwrap_or(reg.gate));
        let before = self.clone();
        let consts = std::mem::replace(&mut self.consts, vec![0; state.words]);
        // With a test before the gate's: the gate's class and the gate made after it.
        let early: Vec<usize> = match reg.pre {
            Some(_) => {
                let check = f.create_block();
                f.ins().brif(first, check, &[], skip, &[]);
                f.switch_to_block(check);
                let class = reg.members.iter().copied().find(|&m| m != reg.gate && self.native.insts[reg.gate].operands().contains(&m)).expect("the gate's class");
                self.emit(&mut *f, class);
                self.emit(&mut *f, reg.gate);
                let any = test(self, &mut *f, reg.gate);
                f.ins().brif(any, active, &[], skip, &[]);
                vec![class, reg.gate]
            }
            None => {
                f.ins().brif(first, active, &[], skip, &[]);
                Vec::new()
            }
        };
        // The region computed. Its carried bits are made plain, not gathered as constants: the
        // two ways must agree.
        f.switch_to_block(active);
        if let Some(calls) = self.calls.filter(|c| !c.inline.borrow()[r]) {
            let args = self.call_region(&mut *f, r, &early, &outside, &words, calls);
            self.consts = consts;
            f.ins().jump(after, &args);
        } else {
        for &m in reg.members.iter().filter(|m| !early.contains(m)) {
            self.emit(&mut *f, m);
        }
        let region_consts = std::mem::replace(&mut self.consts, consts);
        let mut args: Vec<BlockArg> = outside.iter().map(|&m| BlockArg::from(plain(&mut *f, &self.syms, &mut self.values, m))).collect();
        for &w in &words {
            let mut x = self.packed_out[w];
            if region_consts[w] != 0 {
                let k = f.ins().iconst(types::I64, region_consts[w] as i64);
                x = Some(match x {
                    Some(v) => f.ins().bor(v, k),
                    None => k,
                });
            }
            args.push(BlockArg::from(x.unwrap_or_else(|| f.ins().iconst(types::I64, 0))));
        }
        f.ins().jump(after, &args);
        }
        // Skipped: the values zero, the carried words as they were.
        f.switch_to_block(skip);
        let zero = f.ins().iconst(types::I64, 0);
        let mut args: Vec<BlockArg> = outside.iter().map(|_| BlockArg::from(zero)).collect();
        for &w in &words {
            args.push(BlockArg::from(before.packed_out[w].unwrap_or(zero)));
        }
        f.ins().jump(after, &args);
        f.switch_to_block(after);
        let params = f.block_params(after).to_vec();
        // Back to the state before the region: what it computed inside does not reach here.
        let (syms, values, lanes) = (before.syms, before.values, before.lanes);
        self.syms = syms;
        self.values = values;
        self.lanes = lanes;
        for (k, &m) in outside.iter().enumerate() {
            self.set(m, Sym::Value(params[k], false));
        }
        for (k, &w) in words.iter().enumerate() {
            self.packed_out[w] = Some(params[outside.len() + k]);
        }
        for &w in (0..state.words).collect::<Vec<_>>().iter().filter(|w| !words.contains(w)) {
            self.packed_out[w] = before.packed_out[w];
        }
    }

    /// Region `r`'s call, in its active block: the words in (the state carried in, the region's
    /// carried words so far, the values it reads from outside) through a stack slot, the call,
    /// and the words out (the values read after it but those made before the call, its carried
    /// words). Gives the block after's arguments. The function is built on the first call.
    #[allow(clippy::too_many_arguments)]
    fn call_region(&mut self, f: &mut FunctionBuilder<'_>, r: usize, early: &[usize], outside: &[usize], words: &[usize], calls: &RegionCalls) -> Vec<BlockArg> {
        let (state, native) = (self.state, self.native);
        let plan = &state.plan;
        let reg = &plan.regions[r];
        let inner: Vec<usize> = reg.members.iter().copied().filter(|m| !early.contains(m)).collect();
        // What the region reads from outside: on the vectors, a value on the vectors is made
        // again from the bytes (`native_cpu_lanes`); any other is passed.
        let mut inputs: Vec<usize> = Vec::new();
        for &m in &inner {
            for o in native.insts[m].operands() {
                if !inner.contains(&o) && !(plan.lanes[o] && plan.lanes[m]) && !inputs.contains(&o) {
                    inputs.push(o);
                }
            }
        }
        let out: Vec<usize> = outside.iter().copied().filter(|m| !early.contains(m)).collect();
        let n_in = state.words + words.len() + inputs.len();
        let n = n_in + out.len() + words.len();
        let slot = f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * n.max(1) as u32, 3));
        let mut k = 0;
        let mut put = |f: &mut FunctionBuilder<'_>, v: Value| {
            f.ins().stack_store(v, slot, 8 * k);
            k += 1;
        };
        for w in 0..state.words {
            put(&mut *f, self.packed_in[w]);
        }
        // The carried words so far: what came before, and the gate's own if made before the call
        // (its constant bits as well).
        for &w in words {
            let k = self.consts[w];
            let v = match self.packed_out[w] {
                Some(v) if k != 0 => {
                    let k = f.ins().iconst(types::I64, k as i64);
                    f.ins().bor(v, k)
                }
                Some(v) => v,
                None => f.ins().iconst(types::I64, k as i64),
            };
            put(&mut *f, v);
        }
        for &o in &inputs {
            let v = plain(&mut *f, &self.syms, &mut self.values, o);
            put(&mut *f, v);
        }
        if calls.built.borrow()[r].is_none() {
            let func = self.build_region(r, &inner, &inputs, &out, words, calls);
            calls.built.borrow_mut()[r] = Some(func);
        }
        let address = f.ins().stack_addr(types::I64, slot, 0);
        let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, calls.ids[r].as_u32()));
        let signature = f.import_signature(calls.signature.clone());
        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
        let base = self.lanes.base();
        f.ins().call(callee, &[address, base, self.row, self.i, self.len]);
        let mut args = Vec::with_capacity(outside.len() + words.len());
        let mut from = n_in;
        for &m in outside {
            let v = if early.contains(&m) {
                plain(&mut *f, &self.syms, &mut self.values, m)
            } else {
                let v = f.ins().stack_load(types::I64, slot, 8 * from as i32);
                from += 1;
                v
            };
            args.push(BlockArg::from(v));
        }
        for _ in words {
            args.push(BlockArg::from(f.ins().stack_load(types::I64, slot, 8 * from as i32)));
            from += 1;
        }
        args
    }

    /// Region `r`'s function (`call_region`'s layout): its values `inner` made as the body
    /// makes them, in a full block, from its inputs and the block's bytes.
    fn build_region(&self, r: usize, inner: &[usize], inputs: &[usize], out: &[usize], words: &[usize], calls: &RegionCalls) -> cranelift_codegen::ir::Function {
        let (state, native) = (self.state, self.native);
        let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, calls.ids[r].as_u32()), calls.signature.clone());
        let mut builder_context = FunctionBuilderContext::new();
        {
            let mut f = FunctionBuilder::new(&mut func, &mut builder_context);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            let p = f.block_params(entry).to_vec();
            let (io, base, row, i, len) = (p[0], p[1], p[2], p[3], p[4]);
            let c = Consts::make(&mut f);
            let flags = MemFlags::trusted();
            let mut k = 0;
            let mut get = |f: &mut FunctionBuilder<'_>| {
                let v = f.ins().load(types::I64, flags, io, 8 * k);
                k += 1;
                v
            };
            let packed_in: Vec<Value> = (0..state.words).map(|_| get(&mut f)).collect();
            let so_far: Vec<Value> = words.iter().map(|_| get(&mut f)).collect();
            let given: Vec<Value> = inputs.iter().map(|_| get(&mut f)).collect();
            let lanes = Lanes::new(&mut f, native, &state.plan, base, i, len, false, false, None, c.nibble);
            let n = native.insts.len();
            let mut e = Emit {
                sinks: None,
                sunk: vec![None; native.outputs.len()],
                native,
                state,
                full: true,
                i,
                len,
                packed_in: &packed_in,
                weights: c.weights,
                even: c.even,
                live: self.live.clone(),
                masked: self.masked.clone(),
                syms: vec![Sym::Known(0); n],
                values: vec![None; n],
                packed_out: vec![None; state.words],
                consts: vec![0; state.words],
                lanes,
                rounds: Vec::new(),
                row,
                calls: None,
                walks: None,
                walked: vec![false; native.walks.len()],
            };
            for (k, &w) in words.iter().enumerate() {
                e.packed_out[w] = Some(so_far[k]);
            }
            for (k, &o) in inputs.iter().enumerate() {
                e.set(o, Sym::Value(given[k], false));
            }
            for &m in inner {
                e.emit(&mut f, m);
            }
            let mut at = (state.words + words.len() + inputs.len()) as i32;
            for &m in out {
                let v = plain(&mut f, &e.syms, &mut e.values, m);
                f.ins().store(flags, v, io, 8 * at);
                at += 1;
            }
            for &w in words {
                let mut x = e.packed_out[w].expect("its word so far");
                if e.consts[w] != 0 {
                    let k = f.ins().iconst(types::I64, e.consts[w] as i64);
                    x = f.ins().bor(x, k);
                }
                f.ins().store(flags, x, io, 8 * at);
                at += 1;
            }
            f.ins().return_(&[]);
            f.seal_all_blocks();
            f.finalize();
        }
        func
    }

    /// Group `g`'s loop. The head takes each carry and the rounds so far; the members are made
    /// in it, each carried one from the block's carry in, so every round starts from the same
    /// state; it goes back with the nexts while one differs from its carry and the rounds are
    /// under `block::MAX_ROUNDS`, else on, where the last round's values (and the carried fields
    /// it wrote) are the block's: the head dominates the block after.
    fn rounds(&mut self, f: &mut FunctionBuilder<'_>, g: usize) {
        let group = &self.native.groups[g];
        let inits: Vec<Value> = group
            .carries
            .iter()
            .map(|&(c, _)| match self.native.insts[c] {
                BlockInst::Carry(init) => plain(&mut *f, &self.syms, &mut self.values, usize::from(init)),
                _ => unreachable!("a carry"),
            })
            .collect();
        let (head, after) = (f.create_block(), f.create_block());
        for _ in 0..=inits.len() {
            f.append_block_param(head, types::I64);
        }
        let zero = f.ins().iconst(types::I64, 0);
        let args: Vec<BlockArg> = inits.iter().copied().chain([zero]).map(BlockArg::from).collect();
        f.ins().jump(head, &args);
        f.switch_to_block(head);
        self.rounds.push(head);
        let params = f.block_params(head).to_vec();
        for (k, &(c, _)) in group.carries.iter().enumerate() {
            self.set(c, Sym::Value(params[k], false));
            // Its top bits, for a `prev` of it in the next block.
            self.carry(&mut *f, c);
        }
        for &m in &group.members {
            if !matches!(self.native.insts[m], BlockInst::Carry(_)) {
                self.emit(&mut *f, m);
            }
        }
        let nexts: Vec<Value> = group.carries.iter().map(|&(_, x)| plain(&mut *f, &self.syms, &mut self.values, x)).collect();
        let mut changed: Option<Value> = None;
        for (k, &x) in nexts.iter().enumerate() {
            let d = f.ins().bxor(x, params[k]);
            changed = Some(match changed {
                Some(c) => f.ins().bor(c, d),
                None => d,
            });
        }
        let round = f.ins().iadd_imm(params[inits.len()], 1);
        let more = f.ins().icmp_imm(IntCC::UnsignedLessThan, round, crate::block::MAX_ROUNDS as i64);
        let changed = f.ins().icmp_imm(IntCC::NotEqual, changed.expect("a carry"), 0);
        let again = f.ins().band(changed, more);
        let back: Vec<BlockArg> = nexts.iter().copied().chain([round]).map(BlockArg::from).collect();
        f.ins().brif(again, head, &back, after, &[]);
        f.switch_to_block(after);
    }

    /// Value `x`'s word to each of `slots` of the block's row.
    fn store(&mut self, f: &mut FunctionBuilder<'_>, x: usize, slots: &[u16]) {
        f.set_srcloc(cranelift_codegen::ir::SourceLoc::new(x as u32 + 1));
        for &slot in slots {
            let v = plain(&mut *f, &self.syms, &mut self.values, x);
            f.ins().store(stores_flags(), v, self.row, 8 * i32::from(slot));
        }
    }

    /// Value `x`'s mask stores, as outputs `ks` of the last pass (`Sinks`).
    fn sink(&mut self, f: &mut FunctionBuilder<'_>, x: usize, ks: &[usize]) {
        f.set_srcloc(cranelift_codegen::ir::SourceLoc::new(x as u32 + 1));
        let Some(sk) = self.sinks else { return };
        for &k in ks {
            let v = plain(&mut *f, &self.syms, &mut self.values, x);
            let d = self.native.output_delays[k];
            let word = if self.full {
                v
            } else {
                let inside = within_word(&mut *f, self.i, self.len, d);
                f.ins().band(v, inside)
            };
            if d == 0 {
                f.ins().store(stores_flags(), word, sk.dst, 8 * k as i32);
            } else {
                delayed_store(&mut *f, sk.dst, sk.stride, k, d, word);
            }
            self.sunk[k] = Some(f.ins().iconst(types::I64, 0));
        }
    }

    fn finish(mut self, f: &mut FunctionBuilder<'_>) -> (Vec<Value>, Vec<Value>) {
        f.set_srcloc(cranelift_codegen::ir::SourceLoc::default());
        let outputs = (0..self.native.outputs.len())
            .map(|k| match self.sunk[k] {
                Some(v) => v,
                None => plain(&mut *f, &self.syms, &mut self.values, usize::from(self.native.outputs[k])),
            })
            .collect();
        let packed = (0..self.state.words)
            .map(|w| match (self.packed_out[w], self.consts[w]) {
                (Some(v), 0) => v,
                (Some(v), k) => {
                    let k = f.ins().iconst(types::I64, k as i64);
                    f.ins().bor(v, k)
                }
                (None, k) => f.ins().iconst(types::I64, k as i64),
            })
            .collect();
        (outputs, packed)
    }
}

/// `within` at delay `d` for block `i` of a stream of `len` bytes (`block::within_at`): the bits
/// `d <= 64 i + j < len + d`, each bound by two shifts so that 64 bits need no shift by 64.
pub fn within_word(f: &mut FunctionBuilder<'_>, i: Value, len: Value, d: u8) -> Value {
    let below = |f: &mut FunctionBuilder<'_>, end: Value| {
        let start = f.ins().ishl_imm(i, 6);
        let left = f.ins().isub(end, start);
        let zero = f.ins().iconst(types::I64, 0);
        let full = f.ins().iconst(types::I64, 64);
        let left = f.ins().smin(left, full);
        let t = f.ins().smax(left, zero);
        let half = f.ins().ushr_imm(t, 1);
        let rest = f.ins().isub(t, half);
        let one = f.ins().iconst(types::I64, 1);
        let up = f.ins().ishl(one, half);
        let up = f.ins().ishl(up, rest);
        f.ins().iadd_imm(up, -1)
    };
    if d == 0 {
        return below(f, len);
    }
    let end = f.ins().iadd_imm(len, i64::from(d));
    let upper = below(f, end);
    let start = f.ins().iconst(types::I64, i64::from(d));
    let lower = below(f, start);
    f.ins().band_not(upper, lower)
}

/// simdjson's flattening (`bit_indexer::write`): `m`'s set bits as positions `at + bit`, stored as
/// `u32`s from `ptr`. Eight are written whatever the count (the ones past it are slack the next
/// write covers), then eight at a time while more are left. Gives the count, and the loop's block
/// (a `Bits` progress point, `native_cpu_shape`). The builder is left in the block after.
pub fn flatten(f: &mut FunctionBuilder<'_>, m: Value, at: Value, ptr: Value) -> (Value, cranelift_codegen::ir::Block) {
    let flags = MemFlags::new().with_notrap();
    let cnt = f.ins().popcnt(m);
    let eight = |f: &mut FunctionBuilder<'_>, mut w: Value, ptr: Value| {
        for j in 0..8 {
            let tz = f.ins().ctz(w);
            let p = f.ins().iadd(at, tz);
            f.ins().istore32(flags, p, ptr, 4 * j);
            let less = f.ins().iadd_imm(w, -1);
            w = f.ins().band(w, less);
        }
        w
    };
    let w = eight(f, m, ptr);
    let more = f.create_block();
    let after = f.create_block();
    f.append_block_param(more, types::I64);
    f.append_block_param(more, types::I64);
    let over = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, cnt, 8);
    let eight_written = f.ins().iconst(types::I64, 8);
    f.ins().brif(over, more, &[w.into(), eight_written.into()], after, &[]);
    f.switch_to_block(more);
    let (w, written) = (f.block_params(more)[0], f.block_params(more)[1]);
    let offset = f.ins().ishl_imm(written, 2);
    let to = f.ins().iadd(ptr, offset);
    let w = eight(f, w, to);
    let written = f.ins().iadd_imm(written, 8);
    let again = f.ins().icmp(IntCC::UnsignedLessThan, written, cnt);
    f.ins().brif(again, more, &[w.into(), written.into()], after, &[]);
    f.switch_to_block(after);
    (cnt, more)
}

/// Compile a block program's native IR: one function per pass (`native_ir::Passes`), in one
/// module.
pub fn compile(native: &BlockNative) -> Result<CompiledBlock, Error> {
    // Not optimised: the mid-end's code motion would hoist every vector constant out of the loop,
    // more than the registers hold, and spill them (decision 0069). The body is already
    // straight-line and deduplicated.
    let builder = crate::native_cpu_codegen::tier1_builder().map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    for _ in 0..2 {
        signature.params.push(AbiParam::new(pointer));
    }
    signature.params.push(AbiParam::new(types::I64));
    for _ in 0..4 {
        signature.params.push(AbiParam::new(pointer));
    }
    // The CPU's form: the walks made by walking, not by rounds (decision 0139).
    let cpu = native.cpu_form();
    let native = &cpu;
    let passes = native.passes();
    // The records (`records`): made in the last pass at each file's end where it is a forward
    // one, else in a pass of their own after the passes.
    let tables = native.records.as_ref().map(RecordTables::new).transpose()?;
    let merged = tables.is_some() && matches!(passes.passes.last(), Some(Pass::Forward { .. }));
    let mut ir = Vec::new();
    let keep = |ir: &mut Vec<(u32, cranelift_codegen::ir::Function)>, id: cranelift_module::FuncId, func: &cranelift_codegen::ir::Function| {
        if crate::native_cpu_stream::KEEP_IR.load(std::sync::atomic::Ordering::Relaxed) {
            ir.push((id.as_u32(), func.clone()));
        }
    };
    let mut ids = Vec::with_capacity(passes.passes.len());
    let mut disassembly = String::new();
    let mut shapes: Vec<PassShape> = Vec::with_capacity(passes.passes.len());
    for (k, pass) in passes.passes.iter().enumerate() {
        let name = if passes.passes.len() == 1 { "tempo_block".to_owned() } else { format!("tempo_block_pass{k}") };
        let id = module.declare_function(&name, Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
        let mut context: Context = module.make_context();
        context.func.signature = signature.clone();
        context.set_disasm(true);
        let mut hot = Vec::new();
        let mut shape = PassShape { back: false, clif: String::new(), ascii_branch: false, gate_branches: 0, round_branches: 0, extracts: 0, disassembly: String::new(), pressure: Default::default(), outlined: Vec::new(), region_calls: Vec::new(), inlined_regions: 0 };
        match pass {
            Pass::Forward { body, stores } => {
                // The general body, outlined (`general`): declared, built with the pass, defined
                // after it.
                let signature = general_signature(&module);
                let general_id = module.declare_function(&format!("{name}_general"), Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
                let mut general_context: Context = module.make_context();
                general_context.func.signature = signature.clone();
                general_context.set_disasm(true);
                let regions;
                (shape.ascii_branch, shape.gate_branches, shape.round_branches, regions, shape.region_calls, shape.inlined_regions) = forward(&mut context, &mut general_context, (general_id, signature), &mut module, &name, body, stores, passes.slots, &mut hot, native.records.as_ref().zip(tables.as_ref()).filter(|_| merged && k + 1 == passes.passes.len()).map(|(r, t)| (r, t, native.outputs.len())))?;
                shape.extracts = body.extracts.len();
                general_context.func.srclocs.clear();
                let clif = general_context.func.display().to_string();
                { keep(&mut ir, general_id, &general_context.func); crate::native_cpu_codegen::define_function(&mut module, general_id, &mut general_context) }.map_err(|_| Error::BackendFailure)?;
                let code = general_context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default();
                shape.outlined.push(Outlined { clif, disassembly: code });
                module.clear_context(&mut general_context);
                for (id, func) in regions {
                    let mut region_context: Context = module.make_context();
                    region_context.func = func;
                    region_context.set_disasm(true);
                    region_context.func.srclocs.clear();
                    let clif = region_context.func.display().to_string();
                    { keep(&mut ir, id, &region_context.func); crate::native_cpu_codegen::define_function(&mut module, id, &mut region_context) }.map_err(|_| Error::BackendFailure)?;
                    let code = region_context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default();
                    shape.outlined.push(Outlined { clif, disassembly: code });
                    module.clear_context(&mut region_context);
                }
            }
            Pass::Back(fills) => {
                backward(&mut context, fills, passes.slots)?;
                shape.back = true;
            }
        }
        // The marks are the profile's only (`native_cpu_pressure`): read, then cleared, so the IR
        // as text is the program's alone.
        shape.pressure = crate::native_cpu_pressure::measure(&context.func, &passes.origins[k], &hot);
        context.func.srclocs.clear();
        shape.clif = context.func.display().to_string();
        { keep(&mut ir, id, &context.func); crate::native_cpu_codegen::define_function(&mut module, id, &mut context) }.map_err(|_| Error::BackendFailure)?;
        shape.disassembly = context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default();
        disassembly.push_str(&shape.disassembly);
        for o in &shape.outlined {
            disassembly.push_str(&o.disassembly);
        }
        shapes.push(shape);
        module.clear_context(&mut context);
        ids.push(id);
    }
    // The record pass, after the passes (`records`).
    let records = match (&native.records, tables) {
        (Some(_), Some(t)) if merged => Some(t),
        (Some(r), Some(t)) => {
            let id = module.declare_function("tempo_records", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
            let mut context: Context = module.make_context();
            context.func.signature = signature.clone();
            context.set_disasm(true);
            record_pass(&mut context, r, &t, native.extracts.len(), native.outputs.len())?;
            { keep(&mut ir, id, &context.func); crate::native_cpu_codegen::define_function(&mut module, id, &mut context) }.map_err(|_| Error::BackendFailure)?;
            disassembly.push_str(&context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default());
            module.clear_context(&mut context);
            ids.push(id);
            Some(t)
        }
        _ => None,
    };
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    // SAFETY: each function was declared and defined with `Entry`'s signature.
    let entry_ids = ids.iter().map(|id| id.as_u32()).collect();
    let entries = ids.iter().map(|&id| unsafe { std::mem::transmute::<*const u8, Entry>(module.get_finalized_function(id)) }).collect();
    let last = shapes.last().expect("a forward pass at least").clone();
    Ok(CompiledBlock {
        _module: module,
        entries,
        slots: usize::from(passes.slots),
        outputs: native.outputs.len(),
        delay: native.delay,
        extracts: native.extracts.clone(),
        totals: native.totals.clone(),
        counted: counted(native),
        ascii_branch: last.ascii_branch,
        gate_branches: last.gate_branches,
        round_branches: last.round_branches,
        clif: last.clif,
        passes: shapes,
        disassembly,
        records,
        ir,
        entry_ids,
    })
}

/// The record pass's tables (`records::WordTable`), in one buffer the code reads by address:
/// each byte's kind (256 bytes), the slots' keys, the slots' `len | code << 8`, each long word's
/// tails (seven words). Byte offsets of each part.
pub struct RecordTables {
    pub words: Vec<u64>,
    pub keys: usize,
    pub meta: usize,
    pub tails: usize,
    pub table: crate::records::WordTable,
    /// The extracts of the first and last bytes.
    pub starts: u16,
    pub lasts: u16,
}

impl RecordTables {
    pub fn new(r: &crate::records::Records) -> Result<Self, Error> {
        let table = crate::records::WordTable::new(&r.words).map_err(|_| Error::InvalidTarget(0))?;
        let mut words: Vec<u64> = r.kinds.chunks(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 kinds"))).collect();
        let keys = 8 * words.len();
        words.extend(&table.keys);
        // Each slot's length, then the code bits its word gives (`len | bits << 8`): the word at
        // `KIND_BITS`, and the marks of that word alone at `MARK_SHIFT` (`Records::marks`), so
        // that a token takes them with the slot's one load.
        let meta = 8 * words.len();
        words.extend(table.meta.iter().map(|&m| {
            let word = u64::from(m >> 8);
            let alone = r.marks.iter().enumerate().filter(|(_, x)| x.0 == crate::records::WORD_ALONE && u64::from(x.1) == word && word != 0).fold(0u64, |a, (k, _)| a | 1 << k);
            u64::from(m & 0xff) | (word << crate::records::KIND_BITS | alone << crate::records::MARK_SHIFT) << 8
        }));
        let tails = 8 * words.len();
        for t in &table.tails {
            words.extend(t);
        }
        Ok(RecordTables { words, keys, meta, tails, table, starts: r.starts, lasts: r.lasts })
    }
}

/// The bytes `len` of `w` keeps (all eight at eight or more), a mask by `select`: a shift by
/// 64 is a shift by 0 in Cranelift.
fn cut_to(f: &mut FunctionBuilder<'_>, w: Value, len: Value) -> Value {
    let bits = f.ins().ishl_imm(len, 3);
    let one = f.ins().iconst(types::I64, 1);
    let low = f.ins().ishl(one, bits);
    let low = f.ins().iadd_imm(low, -1);
    let all = f.ins().iconst(types::I64, -1);
    let whole = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, len, 8);
    let m = f.ins().select(whole, all, low);
    f.ins().band(w, m)
}

/// A file's columns, for its records (`record_file`): its bytes, the extracts' first and last
/// bytes and the codes' column, the two extracts' counts, its output words (interleaved: output
/// `o`'s word `w` at `o + w * outputs`) and its counts.
struct FileCols {
    bytes: Value,
    starts: Value,
    lasts: Value,
    codes: Value,
    ns: Value,
    nl: Value,
    out: Value,
    counts: Value,
    /// The file's blocks: its rows of output words.
    blocks: Value,
}

/// The record pass on its own (`records`), a function of the passes' signature, where the last
/// pass is not a forward one: per file, its records (`record_file`). Its loops are a file's and a
/// token's.
fn record_pass(context: &mut Context, r: &crate::records::Records, t: &RecordTables, extracts: usize, outputs: usize) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let (entry, row, exit, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_params_for_function_params(entry);
    f.append_block_param(row, types::I64);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let (bytes_all, rows, nrows, out_all, positions_all, counts_all) = (p[0], p[1], p[2], p[3], p[4], p[5]);
    let zero = f.ins().iconst(types::I64, 0);
    let no_rows = f.ins().icmp_imm(IntCC::Equal, nrows, 0);
    f.ins().brif(no_rows, done, &[], row, &[zero.into()]);
    f.switch_to_block(row);
    let rr = f.block_params(row)[0];
    let cols = file_cols(&mut f, r, extracts, rr, rows, bytes_all, out_all, positions_all, counts_all);
    let mut heads = vec![(row, Step::File)];
    record_file(&mut f, r, t, extracts, outputs, &cols, exit, &mut heads);
    f.switch_to_block(exit);
    let next_row = f.ins().iadd_imm(rr, 1);
    let more_rows = f.ins().icmp(IntCC::UnsignedLessThan, next_row, nrows);
    f.ins().brif(more_rows, row, &[next_row.into()], done, &[]);
    f.switch_to_block(done);
    f.ins().return_(&[]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(&context.func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// File `rr`'s columns (`FileCols`) from its row, the counts as the passes stored them.
#[allow(clippy::too_many_arguments)]
fn file_cols(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, extracts: usize, rr: Value, rows: Value, bytes_all: Value, out_all: Value, positions_all: Value, counts_all: Value) -> FileCols {
    let ro = MemFlags::trusted().with_readonly();
    let fl = MemFlags::trusted();
    let at = f.ins().imul_imm(rr, std::mem::size_of::<Row>() as i64);
    let at = f.ins().iadd(rows, at);
    let b = f.ins().load(types::I64, ro, at, 0);
    let bytes = f.ins().iadd(bytes_all, b);
    let pos = f.ins().load(types::I64, ro, at, 32);
    let cap = f.ins().load(types::I64, ro, at, 40);
    let counts = f.ins().load(types::I64, ro, at, 48);
    let counts = f.ins().ishl_imm(counts, 3);
    let counts = f.ins().iadd(counts_all, counts);
    let out = f.ins().load(types::I64, ro, at, 24);
    let out = f.ins().ishl_imm(out, 3);
    let out = f.ins().iadd(out_all, out);
    let mut column = |k: i64| {
        let off = f.ins().imul_imm(cap, k);
        let off = f.ins().iadd(pos, off);
        let off = f.ins().ishl_imm(off, 2);
        f.ins().iadd(positions_all, off)
    };
    let (starts, lasts, codes) = (column(i64::from(r.starts)), column(i64::from(r.lasts)), column(extracts as i64));
    let ns = f.ins().load(types::I64, fl, counts, 8 * i32::from(r.starts));
    let nl = f.ins().load(types::I64, fl, counts, 8 * i32::from(r.lasts));
    let blocks = f.ins().load(types::I64, ro, at, 8);
    FileCols { bytes, starts, lasts, codes, ns, nl, out, counts, blocks }
}

/// A file's records (`records::pass`): per token, its code from its first and last bytes, stored
/// in the codes' column; where the streams disagree, the totals' flag set (the count after the
/// extracts'). Then on to `then`. A token's loop (`Step::Token`); its work is straight-line, but
/// for a word past eight bytes, whose other words are compared in a block of their own.
#[allow(clippy::too_many_arguments)]
fn record_file(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, t: &RecordTables, extracts: usize, outputs: usize, c: &FileCols, then: cranelift_codegen::ir::Block, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let (head, token, long, coded, finish) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    // The loop's values: the token, the end of the token before, the refusal so far.
    for _ in 0..3 {
        f.append_block_param(head, types::I64);
        f.append_block_param(coded, types::I64);
    }
    // At `coded`, the token's code too.
    f.append_block_param(coded, types::I64);
    f.append_block_param(finish, types::I64);
    let ro = MemFlags::trusted().with_readonly();
    let fl = MemFlags::trusted();
    let stores = MemFlags::new().with_notrap();
    let zero = f.ins().iconst(types::I64, 0);
    let tables = f.ins().iconst(types::I64, t.words.as_ptr() as i64);
    let differ = f.ins().icmp(IntCC::NotEqual, c.ns, c.nl);
    let differ = f.ins().uextend(types::I64, differ);
    let n = f.ins().umin(c.ns, c.nl);
    f.ins().jump(head, &[zero.into(), zero.into(), differ.into()]);
    heads.push((head, Step::Token));

    f.switch_to_block(head);
    let h = f.block_params(head).to_vec();
    let more = f.ins().icmp(IntCC::UnsignedLessThan, h[0], n);
    f.ins().brif(more, token, &[], finish, &[h[2].into()]);

    // A token: its first and last bytes, its kind and its word's slot.
    f.switch_to_block(token);
    let (k, after, bad) = (h[0], h[1], h[2]);
    let at = f.ins().ishl_imm(k, 2);
    let sa = f.ins().iadd(c.starts, at);
    let la = f.ins().iadd(c.lasts, at);
    let s = f.ins().uload32(fl, sa, 0);
    let l = f.ins().uload32(fl, la, 0);
    let before = f.ins().icmp(IntCC::UnsignedLessThan, l, s);
    let inside = f.ins().icmp(IntCC::UnsignedLessThan, s, after);
    let wrong = f.ins().bor(before, inside);
    let wrong = f.ins().uextend(types::I64, wrong);
    let bad = f.ins().bor(bad, wrong);
    let next_after = f.ins().iadd_imm(l, 1);
    let len = f.ins().isub(next_after, s);
    let from = f.ins().iadd(c.bytes, s);
    let raw = f.ins().load(types::I64, fl, from, 0);
    let first = f.ins().band_imm(raw, 0xff);
    let ka = f.ins().iadd(tables, first);
    let kind = f.ins().uload8(types::I64, ro, ka, 0);
    let key = cut_to(f, raw, len);
    let h1 = f.ins().bxor(key, len);
    let h1 = f.ins().imul_imm(h1, t.table.mult as i64);
    let slot = f.ins().ushr_imm(h1, i64::from(64 - t.table.bits));
    let off = f.ins().ishl_imm(slot, 3);
    let at = f.ins().iadd(tables, off);
    let kk = f.ins().load(types::I64, ro, at, t.keys as i32);
    let meta = f.ins().load(types::I64, ro, at, t.meta as i32);
    let same_key = f.ins().icmp(IntCC::Equal, kk, key);
    let mlen = f.ins().band_imm(meta, 0xff);
    let same_len = f.ins().icmp(IntCC::Equal, mlen, len);
    let hit = f.ins().band(same_key, same_len);
    // The word's code bits (its word and the marks of it alone, `RecordTables`), on a hit.
    let w = f.ins().ushr_imm(meta, 8);
    let bits = f.ins().select(hit, w, zero);
    let mut code = f.ins().bor(kind, bits);
    // The word itself, where a mark or a long word's tail needs it.
    let word_of = |f: &mut FunctionBuilder<'_>| {
        let x = f.ins().ushr_imm(bits, i64::from(crate::records::KIND_BITS));
        f.ins().band_imm(x, 0xff)
    };
    let word = if r.marks.iter().any(|m| m.0 != crate::records::WORD_ALONE && m.1 != 0) { Some(word_of(&mut *f)) } else { None };
    // The kind and the marks that need no word: the code where a long word's tail differs.
    let mut no_word = kind;
    // The marks at its first byte (`Records::marks`): each its mask's word at the start, the
    // bit, and (a mark of one word) the word compared. Straight-line: a load a mark.
    if r.marks.iter().any(|&(o, wc)| o != crate::records::WORD_ALONE && wc != 0) {
        let row_w = f.ins().ushr_imm(s, 6);
        let row_off = f.ins().imul_imm(row_w, 8 * outputs as i64);
        let row_at = f.ins().iadd(c.out, row_off);
        let sh = f.ins().band_imm(s, 63);
        for (k, &(o, wc)) in r.marks.iter().enumerate() {
            // A mark of its word alone: in the word's code bits already. A mark of any word: set
            // after the tokens, from its bits (`late_marks`).
            if o == crate::records::WORD_ALONE || wc == 0 {
                continue;
            }
            let x = f.ins().load(types::I64, fl, row_at, 8 * i32::from(o));
            let bit = f.ins().ushr(x, sh);
            let mut bit = f.ins().band_imm(bit, 1);
            if wc != 0 {
                let same = f.ins().icmp_imm(IntCC::Equal, word.expect("made for a mark of a word"), i64::from(wc));
                let same = f.ins().uextend(types::I64, same);
                bit = f.ins().band(bit, same);
            }
            let placed = f.ins().ishl_imm(bit, i64::from(crate::records::MARK_SHIFT) + k as i64);
            code = f.ins().bor(code, placed);
            if wc == 0 {
                no_word = f.ins().bor(no_word, placed);
            }
        }
    }
    let longer = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, len, 8);
    let check = f.ins().band(hit, longer);
    f.ins().brif(check, long, &[], coded, &[k.into(), next_after.into(), bad.into(), code.into()]);

    // A word past eight bytes: its other words, compared (seven at most, unrolled).
    f.switch_to_block(long);
    f.set_cold_block(long);
    let word = word.unwrap_or_else(|| word_of(&mut *f));
    let wi = f.ins().iadd_imm(word, -1);
    let toff = f.ins().imul_imm(wi, 56);
    let tail = f.ins().iadd(tables, toff);
    let mut all = f.ins().iconst(types::I8, 1);
    for j in 1..8i64 {
        let past = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, len, 8 * j);
        let chunk = f.ins().load(types::I64, fl, from, (8 * j) as i32);
        let rest = f.ins().iadd_imm(len, -8 * j);
        let chunk = cut_to(f, chunk, rest);
        let want = f.ins().load(types::I64, ro, tail, t.tails as i32 + 8 * (j as i32 - 1));
        let same = f.ins().icmp(IntCC::Equal, chunk, want);
        let ok = f.ins().bor(past, same);
        all = f.ins().band(all, ok);
    }
    let code2 = f.ins().select(all, code, no_word);
    f.ins().jump(coded, &[k.into(), next_after.into(), bad.into(), code2.into()]);

    f.switch_to_block(coded);
    let cv = f.block_params(coded).to_vec();
    let at = f.ins().ishl_imm(cv[0], 2);
    let ca = f.ins().iadd(c.codes, at);
    f.ins().istore32(stores, cv[3], ca, 0);
    let k1 = f.ins().iadd_imm(cv[0], 1);
    f.ins().jump(head, &[k1.into(), cv[1].into(), cv[2].into()]);

    // The file's end: a refusal sets the totals' flag, the count after the extracts'.
    f.switch_to_block(finish);
    let bad = f.block_params(finish)[0];
    let flag = f.ins().load(types::I64, fl, c.counts, 8 * extracts as i32);
    let flag = f.ins().bor(flag, bad);
    f.ins().store(stores, flag, c.counts, 8 * extracts as i32);
    late_marks(f, r, outputs, c, n, then, heads);
}

/// The marks of any word (`Records::marks` with no word): rare bits, so set after the tokens,
/// not tested at each. Over the file's rows, the marks' words ORed; at each bit, its token by a
/// binary search over the starts from the token before; where a token starts there, each
/// mark's bit ORed into its code. Then on to `then`.
fn late_marks(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, outputs: usize, c: &FileCols, n: Value, then: cranelift_codegen::ir::Block, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let late: Vec<(usize, u16)> = r.marks.iter().enumerate().filter(|(_, m)| m.0 != crate::records::WORD_ALONE && m.1 == 0).map(|(k, m)| (k, m.0)).collect();
    if late.is_empty() {
        f.ins().jump(then, &[]);
        return;
    }
    let fl = MemFlags::trusted();
    let stores = MemFlags::new().with_notrap();
    let [rows, row, bits, bit, search, probe, found, set, next] = [(); 9].map(|_| f.create_block());
    // (block, cursor)
    f.append_block_param(rows, types::I64);
    f.append_block_param(rows, types::I64);
    // (block, cursor, bits)
    for b in [bits, bit] {
        for _ in 0..3 {
            f.append_block_param(b, types::I64);
        }
    }
    // (block, cursor, bits, low, high)
    for _ in 0..5 {
        f.append_block_param(search, types::I64);
        f.append_block_param(probe, types::I64);
    }
    // (block, cursor, bits, token)
    for b in [found, set, next] {
        for _ in 0..4 {
            f.append_block_param(b, types::I64);
        }
    }
    heads.push((rows, Step::Block));
    heads.push((bits, Step::Bits));
    heads.push((search, Step::Token));
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(rows, &[zero.into(), zero.into()]);

    f.switch_to_block(rows);
    let (j, cur) = (f.block_params(rows)[0], f.block_params(rows)[1]);
    let more = f.ins().icmp(IntCC::UnsignedLessThan, j, c.blocks);
    f.ins().brif(more, row, &[], then, &[]);

    f.switch_to_block(row);
    let off = f.ins().imul_imm(j, 8 * outputs as i64);
    let at = f.ins().iadd(c.out, off);
    let mut any = None;
    for &(_, o) in &late {
        let w = f.ins().load(types::I64, fl, at, 8 * i32::from(o));
        any = Some(match any {
            None => w,
            Some(a) => f.ins().bor(a, w),
        });
    }
    let any = any.expect("a mark");
    let j1 = f.ins().iadd_imm(j, 1);
    f.ins().brif(any, bits, &[j.into(), cur.into(), any.into()], rows, &[j1.into(), cur.into()]);

    f.switch_to_block(bits);
    let p = f.block_params(bits).to_vec();
    let j1 = f.ins().iadd_imm(p[0], 1);
    f.ins().brif(p[2], bit, &[p[0].into(), p[1].into(), p[2].into()], rows, &[j1.into(), p[1].into()]);

    // The bit's place, and the first token from the cursor on that starts at it or after.
    f.switch_to_block(bit);
    let p = f.block_params(bit).to_vec();
    f.ins().jump(search, &[p[0].into(), p[1].into(), p[2].into(), p[1].into(), n.into()]);

    f.switch_to_block(search);
    let q = f.block_params(search).to_vec();
    let open = f.ins().icmp(IntCC::UnsignedLessThan, q[3], q[4]);
    let qa: Vec<BlockArg> = q.iter().map(|&v| v.into()).collect();
    f.ins().brif(open, probe, &qa, found, &[q[0].into(), q[1].into(), q[2].into(), q[3].into()]);

    f.switch_to_block(probe);
    let q = f.block_params(probe).to_vec();
    let place = {
        let b = f.ins().ctz(q[2]);
        let base = f.ins().ishl_imm(q[0], 6);
        f.ins().iadd(base, b)
    };
    let sum = f.ins().iadd(q[3], q[4]);
    let mid = f.ins().ushr_imm(sum, 1);
    let ma = f.ins().ishl_imm(mid, 2);
    let ma = f.ins().iadd(c.starts, ma);
    let sv = f.ins().uload32(fl, ma, 0);
    let below = f.ins().icmp(IntCC::UnsignedLessThan, sv, place);
    let mid1 = f.ins().iadd_imm(mid, 1);
    let lo = f.ins().select(below, mid1, q[3]);
    let hi = f.ins().select(below, q[4], mid);
    f.ins().jump(search, &[q[0].into(), q[1].into(), q[2].into(), lo.into(), hi.into()]);

    // A token starting at the bit's place: the marks' bits there, into its code.
    f.switch_to_block(found);
    let q = f.block_params(found).to_vec();
    let b = f.ins().ctz(q[2]);
    let base = f.ins().ishl_imm(q[0], 6);
    let place = f.ins().iadd(base, b);
    let inside = f.ins().icmp(IntCC::UnsignedLessThan, q[3], n);
    let k = f.ins().select(inside, q[3], zero);
    let ka = f.ins().ishl_imm(k, 2);
    let sa = f.ins().iadd(c.starts, ka);
    let sv = f.ins().uload32(fl, sa, 0);
    let at_it = f.ins().icmp(IntCC::Equal, sv, place);
    let hit = f.ins().band(inside, at_it);
    let qa: Vec<BlockArg> = q.iter().map(|&v| v.into()).collect();
    f.ins().brif(hit, set, &qa, next, &qa);

    f.switch_to_block(set);
    let q = f.block_params(set).to_vec();
    let off = f.ins().imul_imm(q[0], 8 * outputs as i64);
    let at = f.ins().iadd(c.out, off);
    let b = f.ins().ctz(q[2]);
    let ka = f.ins().ishl_imm(q[3], 2);
    let ca = f.ins().iadd(c.codes, ka);
    let mut code = f.ins().uload32(fl, ca, 0);
    for &(m, o) in &late {
        let w = f.ins().load(types::I64, fl, at, 8 * i32::from(o));
        let x = f.ins().ushr(w, b);
        let x = f.ins().band_imm(x, 1);
        let x = f.ins().ishl_imm(x, i64::from(crate::records::MARK_SHIFT) + m as i64);
        code = f.ins().bor(code, x);
    }
    f.ins().istore32(stores, code, ca, 0);
    f.ins().jump(next, &[q[0].into(), q[1].into(), q[2].into(), q[3].into()]);

    // The bit done: the next, from this token on.
    f.switch_to_block(next);
    let q = f.block_params(next).to_vec();
    let less = f.ins().iadd_imm(q[2], -1);
    let rest = f.ins().band(q[2], less);
    f.ins().jump(bits, &[q[0].into(), q[3].into(), rest.into()]);
}

/// A backward pass (`native_ir::Pass::Back`): per file, its blocks last to first, each
/// `back_fill`'s word from its operands' slots and the carry from the block after
/// (`block::carry::back_fill`: `fill` on the bits reversed), stored to its slot. The loops are a
/// file's and a block's, each a progress point; the body is straight-line.
fn backward(context: &mut Context, fills: &[(u16, u16, u16)], slots: u16) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let (entry, row, head, exit, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_params_for_function_params(entry);
    f.append_block_param(row, types::I64);
    // The head: the blocks left (block `left - 1` next), the file's first slot word, its row,
    // each fill's carry.
    for _ in 0..3 + fills.len() {
        f.append_block_param(head, types::I64);
    }
    f.append_block_param(exit, types::I64);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let (rows, nrows, scratch_all) = (p[1], p[2], p[6]);
    let zero = f.ins().iconst(types::I64, 0);
    let no_rows = f.ins().icmp_imm(IntCC::Equal, nrows, 0);
    f.ins().brif(no_rows, done, &[], row, &[zero.into()]);

    f.switch_to_block(row);
    let r = f.block_params(row)[0];
    let at = f.ins().imul_imm(r, std::mem::size_of::<Row>() as i64);
    let at = f.ins().iadd(rows, at);
    let flags = MemFlags::trusted().with_readonly();
    let blocks = f.ins().load(types::I64, flags, at, 8);
    let first = f.ins().load(types::I64, flags, at, 56);
    let first = f.ins().ishl_imm(first, 3);
    let scratch = f.ins().iadd(scratch_all, first);
    let zero = f.ins().iconst(types::I64, 0);
    let none = f.ins().icmp_imm(IntCC::Equal, blocks, 0);
    let start: Vec<BlockArg> = [blocks, scratch, r].into_iter().chain(std::iter::repeat_n(zero, fills.len())).map(BlockArg::from).collect();
    f.ins().brif(none, exit, &[r.into()], head, &start);

    f.switch_to_block(head);
    let h = f.block_params(head).to_vec();
    let (left, scratch, r) = (h[0], h[1], h[2]);
    let i = f.ins().iadd_imm(left, -1);
    let offset = f.ins().imul_imm(i, 8 * i64::from(slots));
    let words = f.ins().iadd(scratch, offset);
    let stores = MemFlags::new().with_notrap();
    let mut carries = Vec::with_capacity(fills.len());
    for (k, &(s, e, o)) in fills.iter().enumerate() {
        let sw = f.ins().load(types::I64, MemFlags::trusted(), words, 8 * i32::from(s));
        let ew = f.ins().load(types::I64, MemFlags::trusted(), words, 8 * i32::from(e));
        let (start, end) = (f.ins().bitrev(sw), f.ins().bitrev(ew));
        // `block::carry::fill` on the reversed words, the carry in the block after's bit 0.
        let b = f.ins().bor_not(start, end);
        let sum = f.ins().iadd(start, b);
        let sum = f.ins().iadd(sum, h[3 + k]);
        let into = f.ins().bxor(sum, start);
        let into = f.ins().bxor(into, b);
        let open = f.ins().band_not(into, end);
        let filled = f.ins().bor(start, open);
        let word = f.ins().bitrev(filled);
        f.ins().store(stores, word, words, 8 * i32::from(o));
        carries.push(f.ins().band_imm(word, 1));
    }
    let more = f.ins().icmp_imm(IntCC::NotEqual, i, 0);
    let again: Vec<BlockArg> = [i, scratch, r].into_iter().chain(carries).map(BlockArg::from).collect();
    f.ins().brif(more, head, &again, exit, &[r.into()]);

    f.switch_to_block(exit);
    let r = f.block_params(exit)[0];
    let next_row = f.ins().iadd_imm(r, 1);
    let more_rows = f.ins().icmp(IntCC::UnsignedLessThan, next_row, nrows);
    f.ins().brif(more_rows, row, &[next_row.into()], done, &[]);

    f.switch_to_block(done);
    f.ins().return_(&[]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(&context.func, &[(row, Step::File), (head, Step::Block)], &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// What a block's step needs that is the same for every block of a forward pass.
struct Loop<'a> {
    native: &'a BlockNative,
    state: &'a State,
    counted: &'a [usize],
    /// The outputs the body stores where it makes them (`Sinks`): all but the counted.
    early: &'a [bool],
    stride: i64,
    slot_of: &'a [u16],
    slots: u16,
    /// The loop blocks' parameters: the block index, the byte and output pointers, the state
    /// words, the running counts, the row of slots if any. No delayed output's word is held
    /// (`delayed_store`).
    params: usize,
    /// The walks' functions, called by the full body and the general one alike.
    walks: Option<&'a crate::native_cpu_walk::WalkCalls>,
}

impl Loop<'_> {
    fn carried(&self) -> usize {
        self.state.words
    }
    fn rowed(&self) -> bool {
        self.slots > 0
    }
}

/// One block's step, from the loop's parameters `p`: the body, the cut to the stream outside
/// full blocks, the mask stores, the running counts and the flattenings. Gives the next block's
/// parameters, and the body's gate and round branches; the loops' heads go to `heads`.
#[allow(clippy::too_many_arguments)]
fn step(f: &mut FunctionBuilder<'_>, lp: &Loop<'_>, consts: &Consts, p: &[Value], len: Value, full: bool, scratch: Value, calls: Option<&RegionCalls>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<Value>, usize, usize) {
    let (native, state) = (lp.native, lp.state);
    let (carried, counted, early) = (lp.carried(), lp.counted, lp.early);
    let stores = MemFlags::new().with_notrap();
    let (mut gate_branches, mut round_branches) = (0, 0);
    let (i, base, dst) = (p[0], p[1], p[2]);
    let packed_in = &p[3..3 + carried];
    let counts_in = &p[3 + carried..3 + carried + counted.len()];
    // The block's row of slots.
    let scratch = if lp.rowed() { p[lp.params - 1] } else { scratch };
    let (slot_of, slots) = (lp.slot_of, lp.slots);
    let sinks = slot_of.is_empty().then_some(Sinks { dst, stride: lp.stride, early });
    // The full body's one branch: an all-ASCII block runs a body with the values known on
    // ASCII bytes folded (`native_cpu_lanes`), the others the whole body. Both meet before
    // the sinks.
    let (values, packed_out) = if full && state.plan.ascii_branch {
        let (ascii, bytes) = all_ascii(&mut *f, native, &state.plan, base);
        let (fast, slow, merge) = (f.create_block(), f.create_block(), f.create_block());
        let n = native.outputs.len();
        for _ in 0..n + carried {
            f.append_block_param(merge, types::I64);
        }
        f.ins().brif(ascii, fast, &[], slow, &[]);
        for (blk, ascii) in [(fast, true), (slow, false)] {
            f.switch_to_block(blk);
            let (v, packed, gates, rounds) = body(&mut *f, native, state, consts, At { i, len, base, packed_in, full, ascii, bytes: Some(bytes), scratch, slots, slot_of, sinks, calls, walks: lp.walks });
            gate_branches += gates;
            round_branches += rounds.len();
            heads.extend(rounds.into_iter().map(|b| (b, Step::Round)));
            let args: Vec<BlockArg> = v.into_iter().chain(packed).map(BlockArg::from).collect();
            f.ins().jump(merge, &args);
        }
        f.switch_to_block(merge);
        let m = f.block_params(merge).to_vec();
        (m[..n].to_vec(), m[n..].to_vec())
    } else {
        let (v, packed, gates, rounds) = body(&mut *f, native, state, consts, At { i, len, base, packed_in, full, ascii: false, bytes: None, scratch, slots, slot_of, sinks, calls, walks: lp.walks });
        gate_branches += gates;
        round_branches += rounds.len();
        heads.extend(rounds.into_iter().map(|b| (b, Step::Round)));
        (v, packed)
    };
    // Before the last pass each output's word is in its slot: the body stored it.
    // Each output cut to the stream outside full blocks: the bits `d <= 64 i + j < len + d`.
    let words: Vec<Value> = values
        .iter()
        .enumerate()
        .map(|(k, &v)| if full || (slot_of.is_empty() && early[k]) { v } else {
            let inside = within_word(&mut *f, i, len, native.output_delays[k]);
            f.ins().band(v, inside)
        })
        .collect();
    // The mask stores (those the body did not make): a delayed output's by `delayed_store`.
    for k in (0..native.outputs.len()).filter(|&k| slot_of.is_empty() && !early[k]) {
        match native.output_delays[k] {
            0 => {
                f.ins().store(stores, words[k], dst, 8 * k as i32);
            }
            d => delayed_store(&mut *f, dst, lp.stride, k, d, words[k]),
        }
    }
    // The running counts, and each extract's positions at its count.
    let mut counts_out = Vec::with_capacity(counted.len());
    for (c, &k) in counted.iter().enumerate() {
        let n = counts_in[c];
        if c < native.extracts.len() {
            // `n` is where the next position goes.
            let start = f.ins().ishl_imm(i, 6);
            let at = f.ins().iadd_imm(start, -i64::from(native.output_delays[k]));
            let (cnt, more) = flatten(&mut *f, words[k], at, n);
            heads.push((more, Step::Bits));
            let bytes = f.ins().ishl_imm(cnt, 2);
            counts_out.push(f.ins().iadd(n, bytes));
        } else {
            // The totals only test for a bit (`BlockProgram::totals`): every total's words OR-ed
            // into one running word.
            let _ = k;
            let any = native.totals.iter().fold(n, |acc, &t| f.ins().bor(acc, words[t]));
            counts_out.push(any);
        }
    }
    let next = f.ins().iadd_imm(i, 1);
    let (next_base, next_dst) = (f.ins().iadd_imm(base, 64), f.ins().iadd_imm(dst, lp.stride));
    let next_row = lp.rowed().then(|| f.ins().iadd_imm(scratch, 8 * i64::from(slots)));
    let again: Vec<Value> = [next, next_base, next_dst].into_iter().chain(packed_out).chain(counts_out).chain(next_row).collect();
    (again, gate_branches, round_branches)
}

/// The general body as a function of its own, called by the pass for the blocks that are not
/// full (the first and the last): `(p, len)`, the loop's parameters at `p`, read there and
/// written back for the next block. Only the hot loop is inlined: the general body runs a
/// block or two per file, and its code would only crowd the loop's.
fn general(context: &mut Context, lp: &Loop<'_>) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let (at, len) = (f.block_params(entry)[0], f.block_params(entry)[1]);
    let consts = Consts::make(&mut f);
    let flags = MemFlags::trusted();
    let p: Vec<Value> = (0..lp.params).map(|k| f.ins().load(types::I64, flags, at, 8 * k as i32)).collect();
    let zero = f.ins().iconst(types::I64, 0);
    let mut heads = Vec::new();
    let (again, _, _) = step(&mut f, lp, &consts, &p, len, false, zero, None, &mut heads);
    for (k, &v) in again.iter().enumerate() {
        f.ins().store(flags, v, at, 8 * k as i32);
    }
    f.ins().return_(&[]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(&context.func, &heads, &walk_names(lp.walks)).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// The outlined general body's signature: the loop's parameters' address, the stream's length.
fn general_signature(module: &JITModule) -> cranelift_codegen::ir::Signature {
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(module.target_config().pointer_type()));
    signature.params.push(AbiParam::new(types::I64));
    signature
}

/// A forward pass (`native_ir::Pass::Forward`): the body over each file's blocks, first to last.
/// Before the last pass, `slot_of` gives each output's slot: the block's word is stored there as
/// the body made it. The full blocks' body is the hot loop, inlined; the others call `outlined`,
/// the general body (`general`), built into `general_context`. Gives whether the full body
/// branches on ASCII, the gated regions' branches and the fixed points' loops.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn forward(context: &mut Context, general_context: &mut Context, outlined: (cranelift_module::FuncId, cranelift_codegen::ir::Signature), module: &mut JITModule, name: &str, native: &BlockNative, slot_of: &[u16], slots: u16, hot: &mut Vec<cranelift_codegen::ir::Block>, records: Option<(&crate::records::Records, &RecordTables, usize)>) -> Result<(bool, usize, usize, Vec<(cranelift_module::FuncId, cranelift_codegen::ir::Function)>, Vec<(usize, usize)>, usize), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let counted = counted(native);
    let delayed: Vec<usize> = (0..native.outputs.len()).filter(|&k| native.output_delays[k] > 0).collect();
    // Every total is read at the step's end, into the totals' one running word.
    let early: Vec<bool> = (0..native.outputs.len()).map(|k| !counted.contains(&k) && !native.totals.contains(&k)).collect();
    let state = State::of(native);
    let carried = state.words;
    let rowed = slots > 0;
    let params = 3 + carried + counted.len() + usize::from(rowed);
    // Each walk's function, declared here and built where a body first calls it.
    let walks = crate::native_cpu_walk::WalkCalls::declare(module, name, native)?;
    let lp = Loop { native, state: &state, counted: &counted, early: &early, stride: 8 * native.outputs.len() as i64, slot_of, slots, params, walks: Some(&walks) };
    general(general_context, &lp)?;
    let own = cranelift_codegen::ir::UserExternalName::new(0, outlined.0.as_u32());
    // Each gated region's function, declared here and built where the body lowers it.
    let signature = region_signature(module);
    let ids = (0..state.plan.regions.len())
        .map(|r| module.declare_function(&format!("{name}_region{r}"), Linkage::Local, &signature).map_err(|_| Error::BackendFailure))
        .collect::<Result<Vec<_>, _>>()?;
    let n_regions = state.plan.regions.len();
    let calls = RegionCalls { ids, signature, built: std::cell::RefCell::new(vec![None; n_regions]), inline: std::cell::RefCell::new(vec![false; n_regions]) };
    let mut shape = Ok(());
    let ascii_branch = state.plan.ascii_branch;
    let (mut gate_branches, mut round_branches) = (0, 0);
    let (fname, fsig) = (context.func.name.clone(), context.func.signature.clone());
    // Built until no region's call costs the hot loop (`worth_calling`): each round inlines
    // the regions whose calls would.
    for round in 0..=n_regions {
    context.func = cranelift_codegen::ir::Function::with_name_signature(fname.clone(), fsig.clone());
    hot.clear();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        // entry -> row (one per file) -> head -> full or general (one per block) -> head ...
        // -> finish -> exit -> row or done. A full block lies inside the file and is not the
        // first: every `within` is all ones, so its body (the hot loop, block2) has no
        // end-of-file arithmetic. The first block and the last ones call the general body.
        let entry = f.create_block();
        let row = f.create_block();
        let body_block = f.create_block();
        let general = f.create_block();
        let head = f.create_block();
        let finish = f.create_block();
        let exit = f.create_block();
        let done = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.append_block_param(row, types::I64);
        // The carried state, packed (decision 0070): each carry holds only its bits, in a field
        // of a few loop-carried words, never a word per bit. Each loop block's parameters
        // (`Loop::params`): every load and store is `[pointer, #offset]`.
        for b in [body_block, general, head, finish] {
            for _ in 0..params {
                f.append_block_param(b, types::I64);
            }
        }

        f.switch_to_block(entry);
        let entry_params = f.block_params(entry).to_vec();
        let (arena, rows, nrows, out_all, pos_all, counts_all, scratch_all) =
            (entry_params[0], entry_params[1], entry_params[2], entry_params[3], entry_params[4], entry_params[5], entry_params[6]);
        let zero = f.ins().iconst(types::I64, 0);
        // The constants every block uses, made once. (Others are made where used, decision 0069.)
        let consts = Consts::make(&mut f);
        let no_rows = f.ins().icmp_imm(IntCC::Equal, nrows, 0);
        f.ins().brif(no_rows, done, &[], row, &[zero.into()]);

        // A row: the file's blocks, length, outputs, positions and counts.
        f.switch_to_block(row);
        let r = f.block_params(row)[0];
        let at = f.ins().imul_imm(r, std::mem::size_of::<Row>() as i64);
        let at = f.ins().iadd(rows, at);
        let flags = MemFlags::trusted().with_readonly();
        let field = |f: &mut FunctionBuilder<'_>, k: i32| f.ins().load(types::I64, flags, at, 8 * k);
        let (start, blocks, len, first_word) = (field(&mut f, 0), field(&mut f, 1), field(&mut f, 2), field(&mut f, 3));
        let (first_position, capacity, first_count) = (field(&mut f, 4), field(&mut f, 5), field(&mut f, 6));
        let scratch = {
            let first = field(&mut f, 7);
            let from = f.ins().ishl_imm(first, 3);
            f.ins().iadd(scratch_all, from)
        };
        let bytes = f.ins().iadd(arena, start);
        let out = {
            let from = f.ins().ishl_imm(first_word, 3);
            f.ins().iadd(out_all, from)
        };
        // Each extract's first position, and the row's first count.
        let extract_base: Vec<Value> = (0..native.extracts.len())
            .map(|e| {
                let room = f.ins().imul_imm(capacity, e as i64);
                let index = f.ins().iadd(first_position, room);
                let offset = f.ins().ishl_imm(index, 2);
                f.ins().iadd(pos_all, offset)
            })
            .collect();
        let counts_at = {
            let offset = f.ins().ishl_imm(first_count, 3);
            f.ins().iadd(counts_all, offset)
        };
        let none = f.ins().icmp_imm(IntCC::Equal, blocks, 0);
        // The first block's carries and counts are zero, made here: the entry's zero, live across
        // every row, would be spilled and reloaded once per carried word.
        let zero = f.ins().iconst(types::I64, 0);
        // An extract's running count is carried as where its next position goes: every store of
        // the flattening is then `[pointer, #offset]`, with no index to add.
        let first: Vec<BlockArg> = [zero.into(), bytes.into(), out.into()]
            .into_iter()
            .chain(std::iter::repeat_n(zero.into(), carried))
            .chain((0..counted.len()).map(|c| extract_base.get(c).copied().unwrap_or(zero).into()))
            .chain(rowed.then_some(scratch.into()))
            .collect();
        // Blocks 1 .. len / 64 are full.
        let full_end = f.ins().ushr_imm(len, 6);
        f.ins().brif(none, finish, &first, head, &first);

        // The head: the end, or the next block's body.
        f.switch_to_block(head);
        let hp = f.block_params(head).to_vec();
        let args: Vec<BlockArg> = hp.iter().map(|&v| v.into()).collect();
        let ended = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, hp[0], blocks);
        let next_block = f.create_block();
        f.ins().brif(ended, finish, &args, next_block, &[]);
        f.switch_to_block(next_block);
        let past_first = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, hp[0], 1);
        let inside = f.ins().icmp(IntCC::UnsignedLessThan, hp[0], full_end);
        let is_full = f.ins().band(past_first, inside);
        f.ins().brif(is_full, body_block, &args, general, &args);

        // The loops' progress points: the files, the blocks, and each flattening's set bits.
        let mut heads = vec![(row, Step::File), (head, Step::Block), (body_block, Step::Block)];
        // The hot loop's blocks, for the profile (`native_cpu_pressure`): the full body's head
        // and every block made for it.
        let made_from = f.func.dfg.num_blocks();
        f.switch_to_block(body_block);
        let p = f.block_params(body_block).to_vec();
        let again;
        (again, gate_branches, round_branches) = step(&mut f, &lp, &consts, &p, len, true, scratch, Some(&calls), &mut heads);
        let again: Vec<BlockArg> = again.into_iter().map(BlockArg::from).collect();
        // The hot loop's back edge: to itself while the next block is full.
        hot.push(body_block);
        hot.extend((made_from..f.func.dfg.num_blocks()).map(|k| cranelift_codegen::ir::Block::from_u32(k as u32)));
        let next = match again[0] {
            BlockArg::Value(v) => v,
            _ => unreachable!("a value"),
        };
        let more = f.ins().icmp(IntCC::UnsignedLessThan, next, full_end);
        f.ins().brif(more, body_block, &again, head, &again);

        // The other blocks: the general body's call, the parameters through a stack slot.
        f.switch_to_block(general);
        let p = f.block_params(general).to_vec();
        let slot = f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * params as u32, 3));
        for (k, &v) in p.iter().enumerate() {
            f.ins().stack_store(v, slot, 8 * k as i32);
        }
        let pointer = f.func.dfg.value_type(arena);
        let address = f.ins().stack_addr(pointer, slot, 0);
        let name = f.func.declare_imported_user_function(own.clone());
        let signature = f.import_signature(outlined.1.clone());
        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
        f.ins().call(callee, &[address, len]);
        let back: Vec<BlockArg> = (0..params).map(|k| BlockArg::from(f.ins().stack_load(types::I64, slot, 8 * k as i32))).collect();
        f.ins().jump(head, &back);

        // The row's end: each delayed output's last word, and the counts.
        let stores = MemFlags::new().with_notrap();
        f.switch_to_block(finish);
        let p = f.block_params(finish).to_vec();
        let dst = p[2];
        for &k in delayed.iter().filter(|_| slot_of.is_empty()) {
            // The last block's word, as it stored it: shifted down, the stream's last.
            let at = 8 * k as i32 - lp.stride as i32;
            let last = f.ins().load(types::I64, MemFlags::trusted(), dst, at);
            let down = f.ins().ushr_imm(last, i64::from(native.output_delays[k]));
            f.ins().store(stores, down, dst, at);
        }
        for c in 0..counted.len() {
            let mut v = p[3 + carried + c];
            if let Some(&base) = extract_base.get(c) {
                let written = f.ins().isub(v, base);
                v = f.ins().ushr_imm(written, 2);
            } else {
                let any = f.ins().icmp_imm(IntCC::NotEqual, v, 0);
                v = f.ins().uextend(types::I64, any);
            }
            f.ins().store(stores, v, counts_at, 8 * c as i32);
        }
        // The file's records, in the same pass (the last forward one): its positions and mask
        // words just made, still in the cache.
        match records {
            Some((rec, t, outputs)) => {
                let cols = file_cols(&mut f, rec, native.extracts.len(), r, rows, arena, out_all, pos_all, counts_all);
                record_file(&mut f, rec, t, native.extracts.len(), outputs, &cols, exit, &mut heads);
            }
            None => {
                f.ins().jump(exit, &[]);
            }
        }

        f.switch_to_block(exit);
        let next_row = f.ins().iadd_imm(r, 1);
        let more_rows = f.ins().icmp(IntCC::UnsignedLessThan, next_row, nrows);
        f.ins().brif(more_rows, row, &[next_row.into()], done, &[]);

        f.switch_to_block(done);
        f.ins().return_(&[]);
        f.seal_all_blocks();
        f.finalize();
        let mut owns = vec![own.clone()];
        owns.extend(calls.ids.iter().map(|id| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())));
        owns.extend(walk_names(Some(&walks)));
        shape = crate::native_cpu_shape::check(&context.func, &heads, &owns);
    }
    if round == n_regions || !worth_calling(&context.func, &calls) {
        break;
    }
    }
    shape.map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    // The regions' functions, each checked: it calls nothing and has no loop.
    let mut regions = Vec::new();
    let kept: Vec<(usize, usize)> = crate::native_cpu_pressure::across_calls(&context.func)
        .into_iter()
        .filter(|&(inst, _, _)| region_called(&context.func, inst, &calls).is_some())
        .map(|(_, ints, vecs)| (ints, vecs))
        .collect();
    let inline = calls.inline.into_inner();
    let inlined = inline.iter().filter(|&&x| x).count();
    for (r, built) in calls.built.into_inner().into_iter().enumerate() {
        if let Some(func) = built.filter(|_| !inline[r]) {
            crate::native_cpu_shape::check(&func, &[], &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
            regions.push((calls.ids[r], func));
        }
    }
    // The walks' functions, checked where they are built (`native_cpu_walk::build_walk`).
    for (w, built) in walks.built.into_inner().into_iter().enumerate() {
        if let Some(func) = built {
            regions.push((walks.ids[w], func));
        }
    }
    Ok((ascii_branch, gate_branches, round_branches, regions, kept, inlined))
}

/// The walks' functions' names: the functions a pass may call besides its regions'.
fn walk_names(walks: Option<&crate::native_cpu_walk::WalkCalls>) -> Vec<cranelift_codegen::ir::UserExternalName> {
    walks.map_or(Vec::new(), |w| w.ids.iter().map(|id| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect())
}

/// The callee-saved integer registers on aarch64 (x19..x28): the values a call can keep.
const CALLEE_SAVED: usize = 10;

/// Whether each region's call in `func` is worth it: what is live across it stays in registers
/// (`native_cpu_pressure::across_calls`). A vector live across a call is stored and reloaded,
/// on the path that skips the region too: then the region is inlined (`RegionCalls::inline`),
/// and the pass is built again. Gives whether any region was newly inlined.
fn worth_calling(func: &cranelift_codegen::ir::Function, calls: &RegionCalls) -> bool {
    let mut changed = false;
    for (inst, ints, vecs) in crate::native_cpu_pressure::across_calls(func) {
        let Some(r) = region_called(func, inst, calls) else { continue };
        if (vecs > 0 || ints > CALLEE_SAVED) && !calls.inline.borrow()[r] {
            calls.inline.borrow_mut()[r] = true;
            changed = true;
        }
    }
    changed
}

/// The region `inst` calls, if it is a region's call.
fn region_called(func: &cranelift_codegen::ir::Function, inst: cranelift_codegen::ir::Inst, calls: &RegionCalls) -> Option<usize> {
    let cranelift_codegen::ir::InstructionData::Call { func_ref, .. } = func.dfg.insts[inst] else { return None };
    let cranelift_codegen::ir::ExternalName::User(name) = func.dfg.ext_funcs[func_ref].name else { return None };
    let index = func.params.user_named_funcs()[name].index;
    calls.ids.iter().position(|id| id.as_u32() == index)
}

/// The flags of a pass's stores: no trap (the host laid out every word written).
fn stores_flags() -> MemFlags {
    MemFlags::new().with_notrap()
}

/// A value of the bit logic: a known word, or a value that is the word itself or its complement.
/// `Slot`: a word in a stack slot (a walk's result, `Emit::walk`), or its complement, loaded at
/// each use and never held: a value held from the call to its last reader would cost a
/// register through the loop, or a spill.
#[derive(Clone, Copy)]
pub enum Sym {
    Known(u64),
    Value(Value, bool),
    Slot(cranelift_codegen::ir::StackSlot, i32, bool),
}

impl Sym {
    fn not(self) -> Sym {
        match self {
            Sym::Known(c) => Sym::Known(!c),
            Sym::Value(v, neg) => Sym::Value(v, !neg),
            Sym::Slot(s, at, neg) => Sym::Slot(s, at, !neg),
        }
    }

    /// A slot's word loaded here; any other as it is.
    fn loaded(self, f: &mut FunctionBuilder<'_>) -> Sym {
        match self {
            Sym::Slot(s, at, neg) => Sym::Value(f.ins().stack_load(types::I64, s, at), neg),
            other => other,
        }
    }

    /// `a & b`, one instruction at most: a complemented side is a `bic`; both complemented,
    /// the complement of an `or`.
    fn and(f: &mut FunctionBuilder<'_>, a: Sym, b: Sym) -> Sym {
        if let (Sym::Known(0), _) | (_, Sym::Known(0)) = (a, b) {
            return Sym::Known(0);
        }
        let (a, b) = (a.loaded(&mut *f), b.loaded(&mut *f));
        match (a, b) {
            (Sym::Known(x), Sym::Known(y)) => Sym::Known(x & y),
            (Sym::Known(0), _) | (_, Sym::Known(0)) => Sym::Known(0),
            (Sym::Known(u64::MAX), s) | (s, Sym::Known(u64::MAX)) => s,
            (Sym::Known(c), Sym::Value(v, neg)) | (Sym::Value(v, neg), Sym::Known(c)) => {
                let x = if neg { f.ins().bnot(v) } else { v };
                Sym::Value(f.ins().band_imm(x, c as i64), false)
            }
            (Sym::Value(x, false), Sym::Value(y, false)) => Sym::Value(f.ins().band(x, y), false),
            (Sym::Value(x, false), Sym::Value(y, true)) | (Sym::Value(y, true), Sym::Value(x, false)) => Sym::Value(f.ins().band_not(x, y), false),
            (Sym::Value(x, true), Sym::Value(y, true)) => Sym::Value(f.ins().bor(x, y), true),
            (Sym::Slot(..), _) | (_, Sym::Slot(..)) => unreachable!("loaded"),
        }
    }

    fn xor(f: &mut FunctionBuilder<'_>, a: Sym, b: Sym) -> Sym {
        let (a, b) = (a.loaded(&mut *f), b.loaded(&mut *f));
        match (a, b) {
            (Sym::Known(x), Sym::Known(y)) => Sym::Known(x ^ y),
            (Sym::Known(0), s) | (s, Sym::Known(0)) => s,
            (Sym::Known(u64::MAX), s) | (s, Sym::Known(u64::MAX)) => s.not(),
            (Sym::Known(c), Sym::Value(v, neg)) | (Sym::Value(v, neg), Sym::Known(c)) => {
                let k = f.ins().iconst(types::I64, c as i64);
                Sym::Value(f.ins().bxor(v, k), neg)
            }
            (Sym::Value(x, nx), Sym::Value(y, ny)) => Sym::Value(f.ins().bxor(x, y), nx != ny),
            (Sym::Slot(..), _) | (_, Sym::Slot(..)) => unreachable!("loaded"),
        }
    }
}

/// Value `x` plain (not complemented), made once; a slot's word loaded at each use, not kept.
fn plain(f: &mut FunctionBuilder<'_>, syms: &[Sym], values: &mut [Option<Value>], x: usize) -> Value {
    if let Some(v) = values[x] {
        return v;
    }
    let v = match syms[x] {
        Sym::Known(c) => f.ins().iconst(types::I64, c as i64),
        Sym::Value(v, false) => v,
        Sym::Value(v, true) => f.ins().bnot(v),
        Sym::Slot(s, at, neg) => {
            let w = f.ins().stack_load(types::I64, s, at);
            return if neg { f.ins().bnot(w) } else { w };
        }
    };
    values[x] = Some(v);
    v
}

/// Four 16-byte compare results (each byte 0 or 0xff) as one 64-bit mask.
pub fn bitmask(f: &mut FunctionBuilder<'_>, parts: &[Value], weights: Value) -> Value {
    if cfg!(target_arch = "aarch64") {
        // simdjson's arm64 `to_bitmask`: weight, then pairwise adds: 4 `and`, 4 `addp`, 1 `umov`.
        let w: Vec<Value> = parts.iter().map(|&p| f.ins().band(p, weights)).collect();
        let s0 = f.ins().iadd_pairwise(w[0], w[1]);
        let s1 = f.ins().iadd_pairwise(w[2], w[3]);
        let s = f.ins().iadd_pairwise(s0, s1);
        let s = f.ins().iadd_pairwise(s, s);
        let lanes = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(Endianness::Little), s);
        f.ins().extractlane(lanes, 0)
    } else {
        let mut word: Option<Value> = None;
        for (part, &m) in parts.iter().enumerate() {
            let bits = f.ins().vhigh_bits(types::I16, m);
            let wide = f.ins().uextend(types::I64, bits);
            let placed = if part == 0 { wide } else { f.ins().ishl_imm(wide, part as i64 * 16) };
            word = Some(match word {
                None => placed,
                Some(w) => f.ins().bor(w, placed),
            });
        }
        word.expect("four parts")
    }
}
