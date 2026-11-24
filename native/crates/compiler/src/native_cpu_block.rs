//! Block programs on the CPU JIT: Cranelift emits a `BlockNative` (the native IR) as one function
//! `rerail_block(bytes, rows, nrows, out, positions, counts)` over a batch of files (decision
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
    /// The zero bytes the code counts on after every file (`BlockNative::pad`): `Batch` lays
    /// them out.
    pub pad: usize,
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
    /// The places its token loop reads a record it wrote back (`FileCols::after_last_start`):
    /// none where the lean lexer keeps the value in a register (its `base`, its cursor).
    pub reads_back: usize,
    /// The run rules its token loop reads in their arm (`RuleMode::Short`'s form there, no
    /// call), as the lean lexer reads a number in its `match` arm.
    pub arms: usize,
    /// The rare arms whose tail knows its first bytes' entry (`rare_entry`), as the lean
    /// lexer's arm knows its kind.
    pub known_entries: usize,
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

    /// The first bytes whose tokens take their codes from the punctuation table
    /// (`TOKEN_PUNCT`), no word looked up.
    pub fn punct_bytes(&self) -> Vec<u8> {
        self.records.as_ref().map_or_else(Vec::new, |t| (0..=255u8).filter(|&b| t.words[t.token / 8 + usize::from(b)] >> TOKEN_PUNCT & 1 == 1).collect())
    }

    /// Whether the punctuation's codes are in their first bytes' entries
    /// (`RecordTables::punct_in_entry`).
    pub fn punct_in_entry(&self) -> bool {
        self.records.as_ref().is_some_and(|t| t.punct_in_entry.is_some())
    }

    /// The record pass's tables' words, as its code reads them.
    pub fn tables_words(&self) -> Option<&[u64]> {
        self.records.as_ref().map(|t| &t.words[..])
    }

    /// The record pass's tables: their address and bytes. Its code holds addresses inside them
    /// as constants (the slots' keys and meta, say).
    pub fn tables_span(&self) -> Option<(i64, i64)> {
        self.records.as_ref().map(|t| (t.words.as_ptr() as i64, 8 * t.words.len() as i64))
    }

    /// Every output's words over `bytes`; bits past the end are zero.
    /// The calls in the passes' IR: each of a function the same lowering made from the same
    /// program (`native_cpu_shape::check`), the only calls the machine code may have. Those in a
    /// block nothing reaches are not counted: code generation drops them.
    pub fn own_calls(&self) -> usize {
        self.passes.iter().map(|p| crate::clif_text::reachable_calls(&p.clif) + p.outlined.iter().map(|o| crate::clif_text::reachable_calls(&o.clif)).sum::<usize>()).sum()
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
            // One copy, as simdjson's padded input: the file, then zeros to its last block's end
            // and the program's padding past its last byte at least (`BlockNative::pad`).
            let at = bytes.len();
            bytes.extend_from_slice(file);
            bytes.resize(at + (blocks * 64).max(file.len() + program.pad), 0);
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
                // A record's field by its layout (`records::RecordLayout`); an extract's
                // positions are a column, or the rows' field of the records.
                let field_of = |e: usize| program.records.as_ref().and_then(|t| [t.starts, t.lasts].iter().position(|&x| usize::from(x) == e));
                let read = |field: usize, n: u64| -> Vec<u32> {
                    let t = program.records.as_ref().expect("records");
                    let cols = [u64::from(t.starts), u64::from(t.lasts), program.extracts.len() as u64];
                    (0..n).map(|k| self.positions[(row.positions + t.layout.index(field, k, cols, row.capacity)) as usize]).collect()
                };
                let positions = (0..program.extracts.len())
                    .map(|e| match field_of(e) {
                        Some(field) => read(field, count(e)),
                        None => {
                            let at = (row.positions + e as u64 * row.capacity) as usize;
                            self.positions[at..at + count(e) as usize].to_vec()
                        }
                    })
                    .collect();
                let totals = if program.totals.is_empty() { Vec::new() } else { vec![u64::from(count(program.extracts.len()) != 0)] };
                let codes = match &program.records {
                    Some(t) => read(crate::records::FIELD_CODE, count(usize::from(t.starts)).min(count(usize::from(t.lasts)))),
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

/// The default padding (`BlockNative::pad`): zero bytes after every file's last byte, at least,
/// wherever a compiled function reads it (`Batch`, a resident source's slab). As simdjson's
/// padded input, a window that reads past the end reads zeros, so an output false on a zero
/// byte has no bits there and needs no cut (`Bytewise::padded`). The widest window: 64 bits a
/// lane times 32 lanes (Metal's), 256 bytes; a CPU window reaches less (`pad_reach`). An
/// application may ask for less, its windows cut on a rare branch.
pub const BACK: usize = 256;


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
    pub(crate) even: Value,
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
                if !(inner.contains(&o) || (plan.lanes[o] && plan.lanes[m]) || inputs.contains(&o)) {
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
    let made = native;
    let native = &cpu;
    let passes = native.passes();
    // The records (`records`): made in the last pass at each file's end where it is a forward
    // one, else in a pass of their own after the passes.
    let mut tables = native.records.as_ref().map(|r| RecordTables::new(r, made)).transpose()?;
    if let Some(t) = tables.as_mut() {
        t.layout = record_layout(native, t);
    }
    let merged = tables.is_some() && matches!(passes.passes.last(), Some(Pass::Forward { .. }));
    let mut ir = Vec::new();
    // Each function's constants folded (`clif_fold`) before either backend is given it, with
    // what the functions defined before it return known of their calls' results (a callee is
    // defined before its callers).
    let sums = std::cell::RefCell::new(crate::clif_fold::Summaries::new());
    let keep = |ir: &mut Vec<(u32, cranelift_codegen::ir::Function)>, id: cranelift_module::FuncId, func: &mut cranelift_codegen::ir::Function| {
        let mut s = sums.borrow_mut();
        crate::clif_fold::fold_with(func, &s);
        let r = crate::clif_fold::returns(func, &s);
        s.insert(id.as_u32(), r);
        if crate::native_cpu_stream::KEEP_IR.load(std::sync::atomic::Ordering::Relaxed) {
            ir.push((id.as_u32(), func.clone()));
        }
    };
    let mut ids = Vec::with_capacity(passes.passes.len());
    let mut disassembly = String::new();
    let mut shapes: Vec<PassShape> = Vec::with_capacity(passes.passes.len());
    for (k, pass) in passes.passes.iter().enumerate() {
        let name = if passes.passes.len() == 1 { "rerail_block".to_owned() } else { format!("rerail_block_pass{k}") };
        let id = module.declare_function(&name, Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
        let mut context: Context = module.make_context();
        context.func.signature = signature.clone();
        context.set_disasm(true);
        let mut hot = Vec::new();
        let mut shape = PassShape { back: false, clif: String::new(), ascii_branch: false, gate_branches: 0, round_branches: 0, extracts: 0, disassembly: String::new(), pressure: Default::default(), outlined: Vec::new(), region_calls: Vec::new(), inlined_regions: 0, reads_back: 0, arms: 0, known_entries: 0 };
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
                (shape.ascii_branch, shape.gate_branches, shape.round_branches, regions, shape.region_calls, shape.inlined_regions, shape.reads_back, shape.arms, shape.known_entries) = forward(&mut context, &mut general_context, (general_id, signature), &mut module, &name, body, stores, passes.slots, &mut hot, native.records.as_ref().zip(tables.as_ref()).filter(|_| merged && k + 1 == passes.passes.len()).map(|(r, t)| (r, t, native.outputs.len())), made)?;
                shape.extracts = body.extracts.len();
                general_context.func.srclocs.clear();
                let clif = general_context.func.display().to_string();
                // The general body where the pass still calls it (none without a block loop).
                if calls_of(&context.func).contains(&general_id.as_u32()) {
                    { keep(&mut ir, general_id, &mut general_context.func); crate::native_cpu_codegen::define_function(&mut module, general_id, &mut general_context) }.map_err(|_| Error::BackendFailure)?;
                    let code = general_context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default();
                    shape.outlined.push(Outlined { clif, disassembly: code });
                }
                module.clear_context(&mut general_context);
                for (id, func) in regions {
                    let mut region_context: Context = module.make_context();
                    region_context.func = func;
                    region_context.set_disasm(true);
                    region_context.func.srclocs.clear();
                    let clif = region_context.func.display().to_string();
                    { keep(&mut ir, id, &mut region_context.func); crate::native_cpu_codegen::define_function(&mut module, id, &mut region_context) }.map_err(|_| Error::BackendFailure)?;
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
        { keep(&mut ir, id, &mut context.func); crate::native_cpu_codegen::define_function(&mut module, id, &mut context) }.map_err(|_| Error::BackendFailure)?;
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
            let id = module.declare_function("rerail_records", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
            let mut context: Context = module.make_context();
            context.func.signature = signature.clone();
            context.set_disasm(true);
            record_pass(&mut context, r, &t, native.extracts.len(), native.outputs.len())?;
            { keep(&mut ir, id, &mut context.func); crate::native_cpu_codegen::define_function(&mut module, id, &mut context) }.map_err(|_| Error::BackendFailure)?;
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
        pad: usize::from(native.pad),
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

/// What a token compares past a word's first eight bytes, by the table's longest word: as
/// little as the words need, as the lean lexer's `word_code` compares the ninth byte of its one
/// nine-byte word alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordRest {
    /// No word past eight bytes: nothing.
    None,
    /// Nine bytes at most: the ninth, in its slot's meta's top byte (`NINTH_SHIFT`), compared
    /// where the token is nine bytes (lean's `(len != 9) | (ninth == b'n')`).
    Ninth,
    /// Sixteen at most: the second eight, in the slot's third word, straight-line.
    Second,
    /// Longer: a hit on a long word calls the tail compare (`tail_fn`).
    Tails,
}

/// A slot's ninth byte in its meta (`WordRest::Ninth`): the top byte, above the code bits.
const NINTH_SHIFT: i64 = 56;

/// The record pass's tables (`records::WordTable`), in one buffer the code reads by address:
/// each byte's kind (256 bytes), the slots' keys, the slots' `len | code << 8`, each long word's
/// tails (seven words). Byte offsets of each part.
/// A record table's meta bit (`RecordTables::new`): the slot's word is past eight bytes.
const LONG_WORD: u64 = 1 << 63;

pub struct RecordTables {
    pub words: Vec<u64>,
    /// The word slots, 16 bytes each: the key (the word's first eight bytes), then its meta.
    /// A token takes both with one pair load, as the lean lexer's `WORDS` entry. Where a word
    /// is past nine bytes and none past 16 (`WordRest::Second`), 32 bytes each: then its bytes
    /// 8 to 16, and a zero.
    pub slots: usize,
    pub tails: usize,
    /// What a token compares past a word's first eight bytes (`WordRest`), by the longest word.
    pub rest: WordRest,
    /// The first bytes whose tokens take no fused rule (`rule_last`), as nibble tables (a bit
    /// where the lookups' `and` is not zero, `set_nibbles`): the lean lexer's `rare`. A
    /// block's rare starts split its starts into runs (`record_tokens`); none where the loop's
    /// starts are not the lean lexer's.
    pub rare: Option<[[u8; 16]; 2]>,
    /// How each rule's tokens end (`TokenEnd`): a rare start's tail is its rule's class's.
    pub ends: Vec<TokenEnd>,
    /// A rare byte's entry is the same with the pair bit or without (`TOKEN[c | pair << 8]`):
    /// a rare start's is loaded by its byte alone.
    pub rare_unpaired: bool,
    pub table: crate::records::WordTable,
    /// The extracts of the first and last bytes.
    pub starts: u16,
    pub lasts: u16,
    /// Where each pair table's rows (a byte a first byte) and sets (four words each) are
    /// (`Records::nexts`).
    pub pairs: Vec<(usize, usize)>,
    /// Where each first byte's entry is, eight bytes each (`TOKEN_*`): its kind, its end rule,
    /// whether a word starts with it, its pair row. A token's one load, as the lean lexer's
    /// `TOKEN`. Where the codes are in the entries (`punct_in_entry`), 512 of them, as the lean
    /// lexer's: by the first byte and (`+ 256`) whether it starts a pair.
    pub token: usize,
    /// Each token test's bit in the entries (`TOKEN_TESTS`), where its first byte says whether
    /// it can hold (`test_filter`): set for the first bytes it can.
    pub filters: Vec<Option<i64>>,
    /// Where the records are (`record_layout`): rows where the token loop writes them all.
    pub layout: crate::records::RecordLayout,
    /// The fused rules the entries' bits name (`TOKEN_FUSED_*`): the pair rule and the run
    /// rule `fused_rules` takes with the main run's word in a register. The loop tests the
    /// bits where its own choice is this one, else it compares the rule.
    pub fused: Option<(usize, usize)>,
    /// Where each first byte's punctuation codes are (`TOKEN_PUNCT`), 8 bytes each: the code
    /// bits of its one-byte token, then of its two-byte one, as the word table gives them.
    pub punct: usize,
    /// Where every punctuation's codes fit the entry (`TOKEN_PUNCT_ONE`), the field's width:
    /// the one-byte token's in the first byte's entry, the two-byte one's in its pair's (`+
    /// 256`). Then its code is the entry's, no load beside it and no choice by the length, as
    /// the lean lexer's `TOKEN` gives a punctuation's kind and length by one load. Where the
    /// pair rule's word is the loop's (`pair_nibbles`), whose bit chooses the entry.
    pub punct_in_entry: Option<u32>,
    /// The loop's starts are the lean lexer's (`lean_starts`), for the fused rules (`fused`):
    /// whether pairs chain. The run rule's fused bit is then on its run bytes alone.
    pub lean: Option<bool>,
}

/// The records' layout the compiler chooses for a program (decision 0231): rows where the
/// token loop writes each token's start, last and code itself and they are the only extracts
/// (`records_made`), so the three columns' room is the rows' and nothing else writes it; else
/// columns, as the extracts write their positions.
pub fn record_layout(native: &crate::block_native::BlockNative, t: &RecordTables) -> crate::records::RecordLayout {
    let made = native.records_made();
    let mine = |e: u16| native.extracts.get(usize::from(e)).is_some_and(|k| made.contains(k));
    let both = native.extracts.len() == 2 && (t.starts, t.lasts) == (0, 1) && mine(t.starts) && mine(t.lasts);
    crate::records::RecordLayout::choose(crate::records::RecordWriter::TokenLoop, both)
}

/// The entries' bits from which the token tests' first-byte filters are (`RecordTables::filters`).
const TOKEN_TESTS: i64 = 17;

/// The first bytes at which token test `c` can hold, where they decide it (`native_cpu_bytes::
/// needs`): a run's mask at the token's first byte, which needs that byte alone. As the lean
/// lexer tests a `-` or a `0` only at those bytes. None where it lets every first byte of the
/// test's rule through (a number's digits): the bit would decide nothing.
pub fn test_filter(made: &crate::block_native::BlockNative, c: &crate::records::TokenCheck) -> Option<crate::native_cpu_bytes::Needs> {
    if c.count || c.least == 0 {
        return None;
    }
    let r = made.records.as_ref()?;
    crate::native_cpu_bytes::needs(made, c.o).filter(|n| n.lo == 0 && n.hi == 0 && (0..=255u8).any(|b| r.ends.get(usize::from(b)) == Some(&c.rule) && !n.has(b)))
}

/// A first byte's entry (`RecordTables::token`): the kind in its low byte, then the end rule
/// (`Records::ends`), then 1 where some word starts with it (else no word is looked up, the
/// lean lexer's `words != 0`), then its row in its pair table (`EndRule::Next`).
const TOKEN_RULE: i64 = 8;
const TOKEN_WORDY: i64 = 16;
const TOKEN_PAIR_ROW: i64 = 24;
/// Past the pair row: 1 where the byte's rule is the fused pair rule, and where it is the fused
/// run rule (`RecordTables::fused`), so the loop takes its common tokens by a bit of the entry
/// it loaded, no compare of the rule, as the lean lexer's `TOKEN` gives a punctuation's
/// length and a word's run is its mask's.
const TOKEN_FUSED_PAIR: i64 = 32;
const TOKEN_FUSED_RUN: i64 = 33;
/// 1 where the byte's tokens are the fused pair rule's and their code bits are by the first
/// byte and the length alone (`RecordTables::punct`): no word looked up, as the lean lexer's
/// `TOKEN` gives a punctuation's kind.
const TOKEN_PUNCT: i64 = 34;

/// A punctuation's code in its entry (`RecordTables::punct_in_entry`), past the kind's bits
/// (none set in a code): from this bit to the entry's last.
const TOKEN_PUNCT_ONE: u32 = 36;

impl RecordTables {
    /// Whether `b` is a rare first byte (`rare`'s nibble tables).
    pub fn is_rare(&self, b: u8) -> bool {
        self.rare.is_some_and(|n| n[0][usize::from(b & 15)] & n[1][usize::from(b >> 4)] != 0)
    }

    /// How first byte `b`'s tokens end (`TokenEnd`), by its entry's rule.
    pub fn end_at(&self, b: u8) -> TokenEnd {
        self.ends.get(self.rule_at(b)).copied().unwrap_or(TokenEnd::Any)
    }

    /// First byte `b`'s rule, its entry's (`TOKEN_RULE`).
    pub fn rule_at(&self, b: u8) -> usize {
        (self.words[self.token / 8 + usize::from(b)] >> TOKEN_RULE & 0xff) as usize
    }

    pub fn new(r: &crate::records::Records, made: &crate::block_native::BlockNative) -> Result<Self, Error> {
        let table = crate::records::WordTable::new(&r.words).map_err(|_| Error::InvalidTarget(0))?;
        let mut words: Vec<u64> = r.kinds.chunks(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 kinds"))).collect();
        let slots = 8 * words.len();
        // Each slot's length, then the code bits its word gives (`len | bits << 8`): the word at
        // `KIND_BITS`, and the marks of that word alone at `MARK_SHIFT` (`Records::marks`), so
        // that a token takes them with the slot's one load.
        let longest = table.meta.iter().map(|&m| m & 0xff).max().unwrap_or(0);
        let rest = match longest {
            0..=8 => WordRest::None,
            9 => WordRest::Ninth,
            10..=16 => WordRest::Second,
            _ => WordRest::Tails,
        };
        let metas: Vec<u64> = table.meta.iter().map(|&m| {
            let word = u64::from(m >> 8);
            let alone = r.marks.iter().enumerate().filter(|(_, x)| x.0 == crate::records::WORD_ALONE && u64::from(x.1) == word && word != 0).fold(0u64, |a, (k, _)| a | 1 << k);
            // A word past eight bytes: its top bit (`LONG_WORD`), whose other words a token
            // compares; where the words are nine bytes at most, the ninth byte there instead.
            let len = m & 0xff;
            let top = match rest {
                WordRest::Ninth => {
                    let code = (m >> 8) as usize;
                    if len == 9 { (table.tails[code - 1][0] & 0xff) << NINTH_SHIFT } else { 0 }
                }
                _ if len > 8 => LONG_WORD,
                _ => 0,
            };
            u64::from(len) | (word << crate::records::KIND_BITS | alone << crate::records::MARK_SHIFT) << 8 | top
        }).collect();
        let second = rest == WordRest::Second;
        for (&k, (&m, &w)) in table.keys.iter().zip(metas.iter().zip(&table.meta)) {
            words.extend([k, m]);
            if second {
                // A slot's second eight bytes: its word's, zeros for a short word or none.
                let code = (w >> 8) as usize;
                words.extend([if code == 0 { 0 } else { table.tails[code - 1][0] }, 0]);
            }
        }
        let tails = 8 * words.len();
        for t in &table.tails {
            words.extend(t);
        }
        let mut pairs = Vec::new();
        for t in &r.nexts {
            let rows = 8 * words.len();
            words.extend(t.row.chunks(8).map(|c| u64::from_le_bytes(c.try_into().expect("8 rows"))));
            let sets = 8 * words.len();
            words.extend(t.sets.iter().flatten());
            pairs.push((rows, sets));
        }
        let token = 8 * words.len();
        // The tests' filters, a bit each past the word flag, as many as fit below the pair row.
        let mut filters = Vec::new();
        let mut bit = TOKEN_TESTS;
        let mut sets = Vec::new();
        for c in &r.checks {
            match test_filter(made, c).filter(|_| bit < TOKEN_PAIR_ROW) {
                Some(n) => {
                    filters.push(Some(bit));
                    sets.push((bit, n, c.rule));
                    bit += 1;
                }
                None => filters.push(None),
            }
        }
        // The fused rules, as the loop chooses them with the main run's word in a register (as
        // `record_tokens` puts it there).
        let first_bytes = |k: usize| (0..256).filter(|&b| r.ends.get(b).is_some_and(|&e| usize::from(e) == k)).count();
        let main = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], crate::records::EndRule::Run(_))).max_by_key(|&k| first_bytes(k));
        let main_run: Vec<u16> = main.into_iter().filter_map(|k| match r.rules[k] {
            crate::records::EndRule::Run(o) => Some(o),
            _ => None,
        }).collect();
        let fused = if r.skip.is_some() { fused_of(r, &filters, Some(made), &main_run) } else { None };
        // The fused pair rule's tokens' code bits, by first byte and length, where one value
        // each is the word table's (the code's own lookup, `WordTable::slot`): every pair of
        // the byte gives the same; no mark of a word read at the token (`Records::marks`).
        let lookup = |text: &[u8]| -> u64 {
            let len = text.len() as u64;
            let key = text.iter().take(8).enumerate().fold(0u64, |a, (i, &b)| a | u64::from(b) << (8 * i));
            let slot = table.slot(key, len);
            match (table.keys.get(slot), metas.get(slot)) {
                (Some(&k), Some(&m)) if k == key && m & 0xff == len => m >> 8,
                _ => 0,
            }
        };
        let read_marks = r.marks.iter().any(|&(o, wc)| o != crate::records::WORD_ALONE && wc != 0);
        let mut punct_codes = [(0u64, 0u64, false); 256];
        if fused.is_some() && !read_marks {
            // A byte's pair rule (`EndRule::Next`): its own rule, or the other rule of its
            // reader (`/` alone, of the comment's reader), whose tokens not the reader's are
            // that rule's.
            let pair_of = |x: u8| -> Option<&crate::records::PairTable> {
                let next = |k: usize| match r.rules.get(k) {
                    Some(&crate::records::EndRule::Next(i)) => r.nexts.get(usize::from(i)),
                    _ => None,
                };
                next(usize::from(x)).or_else(|| match r.rules.get(usize::from(x)) {
                    Some(&crate::records::EndRule::Read(i)) => r.readers.get(usize::from(i)).and_then(|rd| next(usize::from(rd.other))),
                    _ => None,
                })
            };
            for (b, e) in punct_codes.iter_mut().enumerate() {
                let Some(pt) = r.ends.get(b).and_then(|&x| pair_of(x)) else { continue };
                let one = lookup(&[b as u8]);
                let twos: Vec<u64> = (0..=255u8).filter(|&c| pt.holds(b as u8, c)).map(|c| lookup(&[b as u8, c])).collect();
                let two = twos.first().copied().unwrap_or(0);
                if one >> 32 == 0 && two >> 32 == 0 && twos.iter().all(|&x| x == two) {
                    *e = (one, two, true);
                }
            }
        }
        let kb = crate::records::KIND_BITS;
        let width = |x: u64| 64 - (x >> kb).leading_zeros();
        // The codes in the entries are the fused pair rule's (`by_pair`): its bytes' alone.
        let of_fused: Vec<(u64, u64, bool)> = (0..256).filter(|&b| fused.is_some_and(|(kn, _)| r.ends.get(b).is_some_and(|&x| usize::from(x) == kn))).map(|b| punct_codes[b]).collect();
        let low = of_fused.iter().all(|&(one, two, _)| (one | two) & ((1 << kb) - 1) == 0);
        let w = of_fused.iter().fold(0, |a, &(one, two, _)| a.max(width(one)).max(width(two)));
        // The pair's entry by the pair rule's word (`pair_nibbles`), as the loop has it.
        let paired = fused.is_some_and(|(kn, _)| match r.rules[kn] {
            crate::records::EndRule::Next(i) => r.nexts.get(usize::from(i)).is_some_and(|pt| pair_nibbles(r, pt, kn).is_some()),
            _ => false,
        });
        let punct_in_entry = (low && paired && TOKEN_PUNCT_ONE + w <= 64).then_some(w);
        // The lean lexer's starts (`lean_starts`): the run rule's first bytes that are no run
        // byte left to the general tail.
        let lean = match (fused, r.skip) {
            (Some((kn, kr)), Some(skip)) => lean_starts(r, made, skip, kn, kr),
            _ => None,
        };
        let run_word = fused.and_then(|(_, kr)| match r.rules[kr] {
            crate::records::EndRule::Run(o) => Some(o),
            _ => None,
        });
        // Each rule's tokens end plainly (`plain_end`), with the lean lexer's starts.
        // Else, their last byte no pair's first (`word_end`); else any.
        // A rule's first bytes are its own and those of the readers that go on to it
        // (`Reader::other`): its form holds for each of its tokens.
        let mut firsts: Vec<Vec<u8>> = (0..r.rules.len()).map(|k| (0..=255u8).filter(|&b| r.ends.get(usize::from(b)).is_some_and(|&e| usize::from(e) == k)).collect()).collect();
        for _ in 0..r.rules.len() {
            for k in 0..r.rules.len() {
                if let crate::records::EndRule::Read(i) = r.rules[k] {
                    let o = usize::from(r.readers[usize::from(i)].other);
                    if o < r.rules.len() && o != k {
                        let more: Vec<u8> = firsts[k].iter().copied().filter(|b| !firsts[o].contains(b)).collect();
                        firsts[o].extend(more);
                    }
                }
            }
        }
        let ends_of: Vec<TokenEnd> = (0..r.rules.len()).map(|k| {
            let starts = &firsts[k];
            match lean.and(fused) {
                Some((kn, kr)) if plain_end(r, made, k, starts, kn, kr, 0) => TokenEnd::Plain,
                Some((kn, kr)) if word_end(r, made, k, starts, kn, kr) => TokenEnd::Word,
                _ => TokenEnd::Any,
            }
        }).collect();
        let mut entries = [0u64; 512];
        for (at, e) in entries.iter_mut().enumerate() {
            let (b, two) = (at & 255, at >= 256);
            let rule = r.ends.get(b).copied().unwrap_or(crate::records::NO_RULE);
            let wordy = r.words.iter().any(|w| w.first() == Some(&(b as u8)));
            let row = match r.rules.get(usize::from(rule)) {
                Some(&crate::records::EndRule::Next(i)) => r.nexts[usize::from(i)].row.get(b).copied().unwrap_or(0),
                _ => 0,
            };
            // A bit only on its rule's bytes: a test then runs wherever its bit is set, on a path
            // that more than one rule takes (`rule_last`'s fused rules).
            let tests = sets.iter().filter(|(_, n, k)| n.has(b as u8) && *k == rule).fold(0u32, |a, &(k, _, _)| a | 1 << k);
            let (fp, fr) = fused.map_or((false, false), |(kn, kr)| (usize::from(rule) == kn, usize::from(rule) == kr));
            let fr = fr && (lean.is_none() || run_word.is_some_and(|o| run_fused(made, o, b as u8)));
            *e = u64::from(r.kinds[b]) | u64::from(rule) << TOKEN_RULE | u64::from(wordy) << TOKEN_WORDY | u64::from(tests) | u64::from(row) << TOKEN_PAIR_ROW | u64::from(fp) << TOKEN_FUSED_PAIR | u64::from(fr) << TOKEN_FUSED_RUN | u64::from(punct_codes[b].2) << TOKEN_PUNCT;
            if let (Some(_), true, true) = (punct_in_entry, punct_codes[b].2, fp) {
                *e |= (if two { punct_codes[b].1 } else { punct_codes[b].0 } >> kb) << TOKEN_PUNCT_ONE;
            }
        }
        words.extend(entries);
        // The rare first bytes: those of no fused rule (neither entry bit), as the lean
        // lexer's `rare` (its strings', comments', numbers' first bytes).
        let rare = lean.and(fused).and_then(|_| {
            let mut set = [false; 256];
            for (b, x) in set.iter_mut().enumerate() {
                *x = entries[b] & (1 << TOKEN_FUSED_PAIR | 1 << TOKEN_FUSED_RUN) == 0;
            }
            set_nibbles(&set)
        });
        let punct = 8 * words.len();
        words.extend(punct_codes.iter().map(|&(one, two, _)| one | two << 32));
        let rare_unpaired = rare.is_some_and(|n| (0..256).all(|b| n[0][b & 15] & n[1][b >> 4] == 0 || entries[b] == entries[b | 256]));
        Ok(RecordTables { words, slots, tails, rest, rare, ends: ends_of, rare_unpaired, table, starts: r.starts, lasts: r.lasts, pairs, token, filters, layout: crate::records::RecordLayout::Columns, fused, punct, punct_in_entry, lean })
    }
}

/// The bytes `len` of `w` keeps (all eight at eight or more), a mask by `select`: a shift by
/// 64 is a shift by 0 in Cranelift.
fn cut_to(f: &mut FunctionBuilder<'_>, w: Value, len: Value) -> Value {
    // All ones shifted down by `64 - 8 * min(len, 8)`, mod 64 (a shift's amount is): eight
    // bytes, a shift of 0. As lean's `u64::MAX >> (64 - 8 * len)`, a subtraction from 64, not
    // a negation: LLVM makes `-(8 * n) & 63` a `(56 * n) & 56`, a multiply and an `and` more.
    let eight = f.ins().iconst(types::I64, 8);
    let n = f.ins().umin(len, eight);
    let bits = f.ins().ishl_imm(n, 3);
    let sh = f.ins().irsub_imm(bits, 64);
    let all = f.ins().iconst(types::I64, -1);
    let m = f.ins().ushr(all, sh);
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
    /// The file's bytes.
    len: Value,
    /// The record tables (`RecordTables::words`) and the word hash's multiplier: values, not
    /// constants, so a loop over tokens keeps them in registers (Cranelift remakes a constant
    /// at each use).
    tables: Value,
    mult: Value,
    /// The slots' keys and meta (`RecordTables::keys`, `meta`): a slot's two loads, each one
    /// instruction.
    /// The word slots (`RecordTables::slots`).
    slots: Value,
    /// Where the records are (`records::RecordLayout`): rows from `starts`, or a column each.
    layout: crate::records::RecordLayout,
    /// The records the token loop reads back (`after_last_start`), for its shape
    /// (`PassShape::reads_back`).
    reads_back: std::cell::Cell<usize>,
    /// The run rules whose short form is the loop's arm (`rule_last`'s `arm`), for its shape
    /// (`PassShape::arms`).
    arms: std::cell::Cell<usize>,
    /// The rare arms whose entry is a constant (`PassShape::known_entries`).
    known_entries: std::cell::Cell<usize>,
}

/// The record function's parameters (`record_fn`): `FileCols`' fields, in order.
const RECORD_PARAMS: usize = 13;

impl FileCols {
    fn values(&self) -> [Value; RECORD_PARAMS] {
        [self.bytes, self.starts, self.lasts, self.codes, self.ns, self.nl, self.out, self.counts, self.blocks, self.tables, self.mult, self.slots, self.len]
    }

    fn of(p: &[Value], layout: crate::records::RecordLayout) -> Self {
        FileCols { bytes: p[0], starts: p[1], lasts: p[2], codes: p[3], ns: p[4], nl: p[5], out: p[6], counts: p[7], blocks: p[8], tables: p[9], mult: p[10], slots: p[11], len: p[12], layout, reads_back: Default::default(), arms: Default::default(), known_entries: Default::default() }
    }

    /// As `of`, the grammar's tables, their slots and the word table's multiplier constants
    /// of this function (`tables_consts`), as the lean lexer's tables are statics: an address
    /// or a number the code makes again where it needs it, no register held for it across the
    /// token loop, none loaded from the caller's.
    fn of_tables(f: &mut FunctionBuilder<'_>, p: &[Value], t: &RecordTables) -> Self {
        let (tables, mult, slots) = tables_consts(f, t);
        FileCols { tables, mult, slots, ..FileCols::of(p, t.layout) }
    }

    /// Token `k`'s field `field` (`records::FIELD_*`): the address to load or store at, and
    /// its offset. In rows, one base for the three fields (the lean lexer's `Tok`): the
    /// offset says which.
    fn field(&self, f: &mut FunctionBuilder<'_>, field: usize, k: Value) -> (Value, i32) {
        let cur = self.cursor(f, k);
        self.at(f, field, cur)
    }

    /// The token loop's cursor for token `k` (`at`, `step`): in rows the address of its
    /// record, which the loop bumps a record a token, as the lean lexer pushes a `Tok`, so a
    /// field is the cursor and an offset, no index scaled; in columns the index.
    fn cursor(&self, f: &mut FunctionBuilder<'_>, k: Value) -> Value {
        match self.layout {
            crate::records::RecordLayout::Columns => k,
            crate::records::RecordLayout::Rows => {
                let at = f.ins().imul_imm(k, crate::records::RecordLayout::Rows.stride() as i64);
                f.ins().iadd(self.starts, at)
            }
        }
    }

    /// The token a cursor is at (`cursor`'s inverse), where the loop's count leaves it.
    fn index(&self, f: &mut FunctionBuilder<'_>, cur: Value) -> Value {
        match self.layout {
            crate::records::RecordLayout::Columns => cur,
            crate::records::RecordLayout::Rows => {
                let d = f.ins().isub(cur, self.starts);
                f.ins().udiv_imm(d, crate::records::RecordLayout::Rows.stride() as i64)
            }
        }
    }

    /// The cursor at the next token.
    fn step(&self, f: &mut FunctionBuilder<'_>, cur: Value) -> Value {
        match self.layout {
            crate::records::RecordLayout::Columns => f.ins().iadd_imm(cur, 1),
            crate::records::RecordLayout::Rows => f.ins().iadd_imm(cur, crate::records::RecordLayout::Rows.stride() as i64),
        }
    }

    /// One past the first byte of the token before the cursor's (its record, written), 0 at the
    /// first token: where the gap marks' bits not yet taken start (`records::gap_marks`), as the
    /// loop leaves it, read where it is needed and not carried a token.
    fn after_last_start(&self, f: &mut FunctionBuilder<'_>, cur: Value) -> Value {
        self.reads_back.set(self.reads_back.get() + 1);
        let zero = f.ins().iconst(types::I64, 0);
        let first = self.cursor(f, zero);
        let is_first = f.ins().icmp(IntCC::Equal, cur, first);
        let back = match self.layout {
            crate::records::RecordLayout::Columns => f.ins().iadd_imm(cur, -1),
            crate::records::RecordLayout::Rows => f.ins().iadd_imm(cur, -(crate::records::RecordLayout::Rows.stride() as i64)),
        };
        let at = f.ins().select(is_first, cur, back);
        let (a, o) = self.at(f, crate::records::FIELD_START, at);
        let start = f.ins().uload32(MemFlags::trusted(), a, o);
        let one = f.ins().iadd_imm(start, 1);
        f.ins().select(is_first, zero, one)
    }

    /// Field `field` at cursor `cur`: the address and its offset.
    fn at(&self, f: &mut FunctionBuilder<'_>, field: usize, cur: Value) -> (Value, i32) {
        match self.layout {
            crate::records::RecordLayout::Columns => {
                let at = f.ins().ishl_imm(cur, 2);
                let col = [self.starts, self.lasts, self.codes][field];
                (f.ins().iadd(col, at), 0)
            }
            crate::records::RecordLayout::Rows => (cur, 4 * field as i32),
        }
    }

}

/// A token's record at cursor `cur`: its start and last where the loop made them, and its code,
/// at once at its end, as the lean lexer writes its `Tok` (`out.add(len).write(..)`): adjacent
/// stores in rows (`RecordLayout::Rows`), which rustc pairs as it does lean's fields (`stp`).
fn write_record(f: &mut FunctionBuilder<'_>, c: &FileCols, stores: MemFlags, cur: Value, start: Option<Value>, last: Option<Value>, code: Value) {
    for (field, v) in [(crate::records::FIELD_START, start), (crate::records::FIELD_LAST, last), (crate::records::FIELD_CODE, Some(code))] {
        if let Some(v) = v {
            let (a, o) = c.at(f, field, cur);
            f.ins().istore32(stores, v, a, o);
        }
    }
}

/// The signature of a file's record function (`record_fn`).
fn record_signature(module: &JITModule) -> cranelift_codegen::ir::Signature {
    let mut sig = module.make_signature();
    for _ in 0..RECORD_PARAMS {
        sig.params.push(cranelift_codegen::ir::AbiParam::new(types::I64));
    }
    sig
}

/// A file's records as a function of its own (`record_file`), which the last forward pass
/// calls once a file: a register allocation of its own, apart from the block loop's, so its
/// token loop keeps its values in registers.
fn record_fn(func: &mut cranelift_codegen::ir::Function, r: &crate::records::Records, t: &RecordTables, extracts: usize, outputs: usize) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let (entry, done) = (f.create_block(), f.create_block());
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let cols = FileCols::of_tables(&mut f, &p, t);
    let mut heads = vec![];
    record_file(&mut f, r, t, extracts, outputs, &cols, done, &mut heads);
    f.switch_to_block(done);
    f.ins().return_(&[]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// A row's record slot (decision 0177): `FileCols`' values, then the token loop's state.
const REC_SLOT: usize = RECORD_PARAMS + STATE_WORDS;

/// A signature of `n` words.
fn words_signature(module: &JITModule, n: usize) -> cranelift_codegen::ir::Signature {
    let mut sig = module.make_signature();
    for _ in 0..n {
        sig.params.push(cranelift_codegen::ir::AbiParam::new(types::I64));
    }
    sig
}

/// A file's token loop's end (`(slot)`), a function of its own: the block loop runs the token
/// loop after each block, over the blocks whose outputs are final, and calls this at the
/// file's end. The slot holds the file's columns and the loop's state (`REC_SLOT`).
fn end_fn(func: &mut cranelift_codegen::ir::Function, r: &crate::records::Records, extracts: usize, outputs: usize, looped: u32, layout: crate::records::RecordLayout) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let (entry, done) = (f.create_block(), f.create_block());
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let fl = MemFlags::trusted();
    let vals: Vec<Value> = (0..RECORD_PARAMS).map(|k| f.ins().load(types::I64, fl, p[0], 8 * k as i32)).collect();
    let cols = FileCols::of(&vals, layout);
    let state = f.ins().iadd_imm(p[0], 8 * RECORD_PARAMS as i64);
    let mut heads = vec![];
    let n = f.ins().load(types::I64, fl, state, 0);
    let bad = f.ins().load(types::I64, fl, state, 16);
    let zero = f.ins().iconst(types::I64, 0);
    record_end(&mut f, r, extracts, outputs, &cols, bad, n, zero, looped, done, &mut heads);
    f.switch_to_block(done);
    f.ins().return_(&[]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// Output `o`'s window from the bytes (`native_cpu_bytes`), a function of its own: `(bytes,
/// len, p)` to the output's bits for the file's bytes `p .. p + 64`.
fn bytes_fn(func: &mut cranelift_codegen::ir::Function, bw: &crate::native_cpu_bytes::Bytewise<'_>, o: u16, carries: &[cranelift_codegen::ir::UserExternalName]) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let k = Consts::make(&mut f);
    let file = crate::native_cpu_bytes::File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
    let mut heads = vec![];
    let w = bw.word(&mut f, &file, o, p[2], &mut heads);
    f.ins().return_(&[w]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(func, &heads, carries).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// A rule's count tests (`TokenCheck::count`) as a function of its own (`tests_fn`), its key in
/// the loop's calls (`ByteCall`): past the outputs.
const TESTS_CALL: u16 = 0x8000;

/// A long word's tail compare (`tail_fn`), its key in the loop's calls: past the outputs.
const TAIL_CALL: u16 = 0x4000;

/// A word past eight bytes, its other words compared (decision 0177): `(from, len, tail)` to
/// 1 where each of its words past the first is its table row's (`RecordTables::tails`, `tail`
/// the row), else 0. Seven words at most, straight-line; a function of its own, called on a
/// hit's long flag, as the lean parser keeps its long words' compare out of its loop.
fn tail_fn(func: &mut cranelift_codegen::ir::Function, tails: usize) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let (from, len, tail) = (p[0], p[1], p[2]);
    let (fl, ro) = (MemFlags::trusted(), MemFlags::trusted().with_readonly());
    let mut all = f.ins().iconst(types::I8, 1);
    for j in 1..8i64 {
        let past = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, len, 8 * j);
        let chunk = f.ins().load(types::I64, fl, from, (8 * j) as i32);
        let rest = f.ins().iadd_imm(len, -8 * j);
        let chunk = cut_to(&mut f, chunk, rest);
        let want = f.ins().load(types::I64, ro, tail, tails as i32 + 8 * (j as i32 - 1));
        let same = f.ins().icmp(IntCC::Equal, chunk, want);
        let ok = f.ins().bor(past, same);
        all = f.ins().band(all, ok);
    }
    let all = f.ins().uextend(types::I64, all);
    f.ins().return_(&[all]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(func, &[], &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// The count tests of a rule, fused: `(bytes, len, s, l)` to their hits at token `s..=l`, the
/// marks' as `records::check_hits` (`At` and `After` bits), a refusal at bit 63. One loop over
/// the token's words; each word's outputs from one window (`Bytewise::words`), each test's
/// count its popcount. The rule's tests classify the bytes once, not once a test.
fn tests_fn(func: &mut cranelift_codegen::ir::Function, bw: &crate::native_cpu_bytes::Bytewise<'_>, tests: &[crate::records::TokenCheck], carries: &[cranelift_codegen::ir::UserExternalName]) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let (s, l) = (p[2], p[3]);
    let k = Consts::make(&mut f);
    let file = crate::native_cpu_bytes::File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
    let mut heads = vec![];
    let mut outs: Vec<u16> = tests.iter().map(|t| t.o).collect();
    outs.sort_unstable();
    outs.dedup();
    let q0 = f.ins().ushr_imm(s, 6);
    let ql = f.ins().ushr_imm(l, 6);
    let sh = f.ins().band_imm(s, 63);
    let lb = f.ins().band_imm(l, 63);
    let lb = f.ins().irsub_imm(lb, 63);
    let ones = f.ins().iconst(types::I64, -1);
    let zero = f.ins().iconst(types::I64, 0);
    let (head, out) = (f.create_block(), f.create_block());
    f.append_block_param(head, types::I64);
    for _ in &outs {
        f.append_block_param(head, types::I64);
        f.append_block_param(out, types::I64);
    }
    let init: Vec<BlockArg> = std::iter::once(q0).chain(outs.iter().map(|_| zero)).map(BlockArg::from).collect();
    f.ins().jump(head, &init);
    f.switch_to_block(head);
    heads.push((head, Step::Word));
    let hp = f.block_params(head).to_vec();
    let q = hp[0];
    let at = f.ins().ishl_imm(q, 6);
    let ws = bw.words(&mut f, &file, &outs, at, &mut heads);
    let first = f.ins().icmp(IntCC::Equal, q, q0);
    let from = f.ins().ishl(ones, sh);
    let from = f.ins().select(first, from, ones);
    let last = f.ins().icmp(IntCC::Equal, q, ql);
    let to = f.ins().ushr(ones, lb);
    let to = f.ins().select(last, to, ones);
    let m = f.ins().band(from, to);
    let accs: Vec<Value> = ws.iter().zip(&hp[1..]).map(|(&w, &a)| {
        let x = f.ins().band(w, m);
        let pc = f.ins().popcnt(x);
        f.ins().iadd(a, pc)
    }).collect();
    let q1 = f.ins().iadd_imm(q, 1);
    let back: Vec<BlockArg> = std::iter::once(q1).chain(accs.iter().copied()).map(BlockArg::from).collect();
    let done: Vec<BlockArg> = accs.iter().copied().map(BlockArg::from).collect();
    f.ins().brif(last, out, &done, head, &back);
    f.switch_to_block(out);
    let counts = f.block_params(out).to_vec();
    let mut result = zero;
    for t in tests {
        let n = counts[outs.iter().position(|&o| o == t.o).expect("its output")];
        let hit = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, i64::from(t.least));
        let hit = f.ins().uextend(types::I64, hit);
        let at = match t.then {
            crate::records::CheckThen::Escape => 63,
            crate::records::CheckThen::At(k) => i64::from(k),
            crate::records::CheckThen::After(k) => i64::from(crate::records::AFTER_SHIFT) + i64::from(k),
        };
        let bit = f.ins().ishl_imm(hit, at);
        result = f.ins().bor(result, bit);
    }
    f.ins().return_(&[result]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(func, &heads, carries).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// A reader's arm and its rule's count tests, joined (`read_fn`), its key in the loop's calls:
/// `READ_CALL | rule << 3 | arm`, past the outputs.
const READ_CALL: u16 = 0x1000;

/// A rule's function's arguments (`rule_fn`), in this order: the bytes, the token's first
/// byte, the file's length, its blocks, the token's first eight bytes and its first byte's
/// entry. A function takes the first of them it reads to (`rule_reads`), as many as its
/// signature has, as a lexer's number arm reads its bytes from its start alone.
const RULE_ARGS: usize = 6;

/// The arguments of a call to a rule's function of signature `sig`: the first of `all`
/// (`RULE_ARGS`' order) as many as it takes.
fn rule_args(sig: &cranelift_codegen::ir::Signature, all: [Value; RULE_ARGS]) -> Vec<Value> {
    all[..sig.params.len()].to_vec()
}

/// How many of `RULE_ARGS`' a rule's function built with all of them reads once optimized (the
/// code the backends compile and `clif_rust` writes: no value no instruction reads, no block
/// not reached): one past the last it reads. Its signature is then those, and it is built
/// again.
fn rule_reads(func: &cranelift_codegen::ir::Function, isa: &dyn cranelift_codegen::isa::TargetIsa) -> usize {
    let mut ctx = cranelift_codegen::Context::for_function(func.clone());
    if ctx.optimize(isa, &mut cranelift_codegen::control::ControlPlane::default()).is_err() {
        return RULE_ARGS;
    }
    let func = &ctx.func;
    let Some(entry) = func.layout.entry_block() else { return RULE_ARGS };
    let params = func.dfg.block_params(entry).to_vec();
    let mut cfg = cranelift_codegen::flowgraph::ControlFlowGraph::new();
    cfg.compute(func);
    let mut reached = std::collections::HashSet::new();
    let mut stack = vec![entry];
    while let Some(b) = stack.pop() {
        if reached.insert(b) {
            stack.extend(cfg.succ_iter(b));
        }
    }
    let mut read = 0;
    for b in func.layout.blocks().filter(|b| reached.contains(b)) {
        for inst in func.layout.block_insts(b) {
            for v in func.dfg.inst_values(inst) {
                if let Some(i) = params.iter().position(|&p| p == v) {
                    read = read.max(i + 1);
                }
            }
        }
    }
    read
}

/// A rule's function of its own (`rule_fn`): its key in the loop's calls, `RULES_CALL | rule`,
/// past the outputs.
const RULES_CALL: u16 = 0x0800;

/// Which of a token loop's heads a token's rules are for (`record_tokens`): one head for every
/// token; or, as the lean lexer's two loops, the run loop's (the fused rules, and the run
/// rule's token past its block) and the rare starts' (the other rules, no fused one).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Both,
    Fused,
    /// The rare starts', all their rules (`None`); or, as the lean lexer's `match` arms, one
    /// rule's (`RareClass`), each with its own tail.
    Rare(Option<RareClass>),
}

/// A rare start's arm (`Side::Rare`), as the lean lexer's `match` arm writes its own token:
/// one rule (`NO_RULE`: the bytes with none), how its tokens end (`TokenEnd`), and whether its
/// reader tags its token (`Reader::retag`, `tagged`). Its tail knows its first bytes
/// (`rare_bytes`), and their entry where they share one (`rare_entry`).
#[derive(Clone, Copy, PartialEq, Eq)]
struct RareClass {
    end: TokenEnd,
    tagged: bool,
    rule: usize,
}

/// Whether rule `k` can give a last below its token's start, with the loop's starts
/// (`records::walk`): a reader's arm that ends with no closer at `s - 1` (`ReadNone::Bytes(0)`),
/// or a reader's other rule that can. A search with no last gives the first byte; a refusal
/// leaves the loop (`record_tokens`).
fn last_below_start(r: &crate::records::Records, k: usize) -> bool {
    match r.rules.get(k) {
        Some(&crate::records::EndRule::Read(i)) => {
            let rd = &r.readers[usize::from(i)];
            rd.arms.iter().any(|a| matches!(a.none, crate::records::ReadNone::Bytes(0))) || (usize::from(rd.other) != k && last_below_start(r, usize::from(rd.other)))
        }
        Some(crate::records::EndRule::Last(_)) => r.skip.is_none(),
        _ => false,
    }
}

/// The bytes that can be right before a token's first byte, with the loop's starts
/// (`records::walk`), for `Bytewise::before_start`: each `(next, bytes)`, `bytes` where the
/// token's first byte can be in `next`. A gap's byte (`skip`); a token's last, by its rule: a
/// run's holds its run or is its first byte, a pair's is its first byte or a pair's second, a
/// reader's is its closer's byte at `off`. A reader whose closer begins right after its last
/// (`off` -1, a line comment's `\n`): the next byte is the closer's, so its last is right
/// before a token only one of the closer's bytes starts. All bytes where a rule's last is
/// not so known; none without the loop's starts.
pub(crate) fn before_start(r: &crate::records::Records, native: &BlockNative) -> Option<Vec<([u64; 4], [u64; 4])>> {
    use crate::records::{EndRule, ReadNone};
    let skip = r.skip?;
    let all = [u64::MAX; 4];
    // Where output `o` can hold, at offset `at` from its position: by its needs.
    let held = |o: u16, at: i32| crate::native_cpu_bytes::bytes_at(native, o, at);
    let or = |a: [u64; 4], b: [u64; 4]| -> [u64; 4] { std::array::from_fn(|k| a[k] | b[k]) };
    let mut any = held(skip, 0);
    // A byte with no rule is a token alone, where the loop does not refuse it (`rule_last`'s
    // no-rule arm: refused with untaken outputs).
    for b in (0..256usize).filter(|_| r.untaken.is_empty()) {
        if r.ends.get(b).is_none_or(|&e| usize::from(e) >= r.rules.len()) {
            any[b / 64] |= 1 << (b % 64);
        }
    }
    let mut after = Vec::new();
    for k in 0..r.rules.len() {
        let first = first_bytes(r, k);
        let last = match r.rules[k] {
            EndRule::Run(o) => or(held(o, 0), first),
            EndRule::Next(i) => r.nexts.get(usize::from(i)).map_or(all, |t| t.sets.iter().fold(first, |a, &s| or(a, s))),
            EndRule::Pair(_) | EndRule::Last(_) => all,
            EndRule::Read(i) => {
                let mut x = [0u64; 4];
                for a in &r.readers[usize::from(i)].arms {
                    if a.off == -1 {
                        after.push((held(a.closer, 0), all));
                    } else {
                        x = or(x, held(a.closer, i32::from(a.off)));
                    }
                    x = or(x, match a.none {
                        // The opener's first `m` bytes: its `m`th byte where it holds.
                        ReadNone::Bytes(0) => all,
                        ReadNone::Bytes(m) => held(a.opener, i32::from(m) - 1),
                        ReadNone::End | ReadNone::Refuse => [0; 4],
                    });
                }
                x
            }
        };
        any = or(any, last);
    }
    after.push((all, any));
    Some(after)
}

/// `RareClass::rule` of the rare bytes with no rule.
const NO_RULE: usize = usize::MAX;

/// The rare first bytes whose tokens reach arm `x`'s rule: those `rule_last`'s table sends
/// to it, and those a reader with no opener there goes on to it from (`first_bytes`). The
/// arm with no rule: every rare byte, as many reach it.
fn rare_bytes(r: &crate::records::Records, t: &RecordTables, x: RareClass) -> Vec<u8> {
    let reach = (x.rule != NO_RULE).then(|| first_bytes(r, x.rule));
    (0..=255u8).filter(|&b| t.is_rare(b) && reach.is_none_or(|s| s[usize::from(b) / 64] >> (b % 64) & 1 == 1)).collect()
}

/// Arm `x`'s first bytes' entry (`RecordTables::token`) where they share one: the arm's
/// constant, as the lean lexer's arm knows its byte's kind. None for the arm with no rule.
fn rare_entry(r: &crate::records::Records, t: &RecordTables, x: RareClass) -> Option<u64> {
    if x.rule == NO_RULE {
        return None;
    }
    let bytes = rare_bytes(r, t, x);
    let e = t.words[t.token / 8 + usize::from(*bytes.first()?)];
    bytes.iter().all(|&b| t.words[t.token / 8 + usize::from(b)] == e).then_some(e)
}

/// Where `rule_last` builds the rules' blocks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RuleMode {
    /// Every rule's block in the function.
    Inline,
    /// Each rule with a function of its own (`rule_fn`) its call, but the fused two: the loop's
    /// code is its common tokens', as the lean lexer's run loop, and a rare token one call.
    Calls,
    /// Rule `k`'s block alone, and those it goes on to (`Reader::other`), entered at once: the
    /// body of its function.
    Only(usize),
    /// Rule `k`'s block alone, a run's: its token where the run ends in its first 16 bytes, and
    /// else the call of its whole function (`LONG_CALL`), returned at once. A lexer's short
    /// number: no call on its way, so no register saved (`rule_fn`).
    Short(usize),
}

/// A run rule's whole function (`rule_fn`, `RuleMode::Only`), where the loop calls its short
/// one (`RuleMode::Short`): its key in the calls, `LONG_CALL | rule`.
const LONG_CALL: u16 = 0x0400;

/// A rule function's results in two words (`rule_fn`), where every reader's tag fits: the
/// last and the tag, `(last + 1) | (tag + 1) << 32`, and the hits with the refusal as their
/// sign bit, `hits | refuse & 1 << 63`. Two words come back in two registers, as a lexer's
/// function returns its token; four went through memory, a store each in the callee and a
/// load each in the loop. A last is a record's `u32` (or `-1`), a tag a code (a `u32`, `-1`
/// for none), a hit a mark's bit (`records::check_hits`, a `u32`).
fn packs(r: &crate::records::Records) -> bool {
    r.readers.iter().all(|x| x.retag.is_none_or(|(kind, marks)| (u64::from(kind) | u64::from(marks) << crate::records::MARK_SHIFT) < u64::from(u32::MAX)))
}

/// A rule function's four results packed (`packs`).
fn pack_rule(f: &mut FunctionBuilder<'_>, l: Value, refuse: Value, tag: Value, hits: Value) -> [Value; 2] {
    // The last's field is its low 32 bits, said so: a caller knows the tag's field is the
    // rest (`clif_fold`), a constant where every return's tag is.
    let l1 = f.ins().iadd_imm(l, 1);
    let l1 = f.ins().ireduce(types::I32, l1);
    let l1 = f.ins().uextend(types::I64, l1);
    let t1 = f.ins().iadd_imm(tag, 1);
    let t1 = f.ins().ishl_imm(t1, 32);
    let a = f.ins().bor(l1, t1);
    let sign = f.ins().band_imm(refuse, i64::MIN);
    let b = f.ins().bor(hits, sign);
    [a, b]
}

/// A packed rule function's results (`pack_rule`) as its four: the refusal is the second
/// word, as only its sign is read.
fn unpack_rule(f: &mut FunctionBuilder<'_>, a: Value, b: Value) -> [Value; 4] {
    let l1 = f.ins().band_imm(a, 0xffff_ffff);
    let l = f.ins().iadd_imm(l1, -1);
    let t1 = f.ins().ushr_imm(a, 32);
    let tag = f.ins().iadd_imm(t1, -1);
    let hits = f.ins().band_imm(b, 0xffff_ffff);
    [l, b, tag, hits]
}

/// The arms whose reads `read_fn` joins with their rule's count tests: a closer, and its
/// tests' masks, the bytes make.
fn read_fused(bw: &crate::native_cpu_bytes::Bytewise<'_>, a: &crate::records::ReadArm, tests: &[crate::records::TokenCheck]) -> bool {
    !tests.is_empty() && bw.can(a.closer) && tests.iter().all(|t| t.count && bw.can(t.o))
}

/// Whether a reader's arm makes its rule's count tests in its read loop (`read_fn`), as the
/// lean lexer's `string_end` makes its escape test (`u |= esc & eq('u') & below(quote)`) in
/// the scan that finds the quote: each window's test masks with its closer's. The loop reads
/// from the byte after the opener, so no test's mask may hold where the opener does
/// (`native_cpu_bytes::never_both`); and the token's last is the closer's (`off` 0), so the
/// window that holds the closer holds the last too.
fn tests_in_read(bw: &crate::native_cpu_bytes::Bytewise<'_>, a: &crate::records::ReadArm, tests: &[crate::records::TokenCheck]) -> bool {
    a.off == 0 && tests.iter().all(|t| crate::native_cpu_bytes::never_both(bw.native, t.o, a.opener))
}

/// The outputs a reader's loop makes for its rule's count tests (`tests_in_read`), each once,
/// and whether it counts them: a test of two bytes or more needs their number, one of one
/// byte only whether one is there, so its words are ORed, as `string_end`'s `u`.
fn read_test_outs(tests: &[crate::records::TokenCheck]) -> Vec<(u16, bool)> {
    let mut outs: Vec<u16> = tests.iter().map(|t| t.o).collect();
    outs.sort_unstable();
    outs.dedup();
    outs.into_iter().map(|o| (o, tests.iter().any(|t| t.o == o && t.least > 1))).collect()
}

/// A test output's window into its accumulator (`read_test_outs`): ORed, or its bits counted.
fn read_test_acc(f: &mut FunctionBuilder<'_>, counted: bool, acc: Value, w: Value) -> Value {
    if counted {
        let n = f.ins().popcnt(w);
        f.ins().iadd(acc, n)
    } else {
        f.ins().bor(acc, w)
    }
}

/// A test's hit, 0 or 1, into the refusal (`CheckThen::Escape`, all ones) or at its mark's
/// bit: `(refusal, hits)`.
fn hit_into(f: &mut FunctionBuilder<'_>, then: crate::records::CheckThen, hit: Value, refuse: Value, hits: Value) -> (Value, Value) {
    match then {
        crate::records::CheckThen::Escape => {
            let all = f.ins().ineg(hit);
            (f.ins().bor(refuse, all), hits)
        }
        crate::records::CheckThen::At(k) => {
            let b = f.ins().ishl_imm(hit, i64::from(k));
            (refuse, f.ins().bor(hits, b))
        }
        crate::records::CheckThen::After(k) => {
            let b = f.ins().ishl_imm(hit, i64::from(crate::records::AFTER_SHIFT) + i64::from(k));
            (refuse, f.ins().bor(hits, b))
        }
    }
}

/// The count tests' hits from their accumulators (`read_test_acc`), as `tests_fn` gives them:
/// the refusal (`CheckThen::Escape`) as the sign bit, a mark's hit at its bit.
fn read_test_hits(f: &mut FunctionBuilder<'_>, tests: &[crate::records::TokenCheck], outs: &[(u16, bool)], accs: &[Value]) -> Value {
    let mut result = f.ins().iconst(types::I64, 0);
    for t in tests {
        let i = outs.iter().position(|&(o, _)| o == t.o).expect("its output");
        let hit = if outs[i].1 { f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, accs[i], i64::from(t.least)) } else { f.ins().icmp_imm(IntCC::NotEqual, accs[i], 0) };
        let hit = f.ins().uextend(types::I64, hit);
        let at = match t.then {
            crate::records::CheckThen::Escape => 63,
            crate::records::CheckThen::At(k) => i64::from(k),
            crate::records::CheckThen::After(k) => i64::from(crate::records::AFTER_SHIFT) + i64::from(k),
        };
        let bit = f.ins().ishl_imm(hit, at);
        result = f.ins().bor(result, bit);
    }
    result
}

/// A reader's arm, its end and its rule's count tests in one call, as a lexer's string reader
/// finds the end and checks the bytes it reads: `(bytes, len, s)` to its closer's first bit
/// after `s` (`-1`, none in the file) and the tests' hits at `s..=l` (`tests_fn`'s), `l` the
/// closer's at `off`. The closer's words from `s + 1`, 64 bytes apart, an escape's carry
/// passed from each to the next (`Bytewise::words_carried`), none after an opener that is no
/// escape's (`Bytewise::apart`). The tests' masks in the same windows where they can be
/// (`tests_in_read`), as `string_end` makes its escape test as it scans: each window's ORed or
/// counted in, the closer's up to the closer. Else, after the loop: none where the token's
/// bytes have none of those they need (`needs_spans`), else their function (`tests_fn`).
#[allow(clippy::too_many_arguments)]
fn read_fn(func: &mut cranelift_codegen::ir::Function, bw: &crate::native_cpu_bytes::Bytewise<'_>, a: &crate::records::ReadArm, tests: &[crate::records::TokenCheck], counts: &ByteCall, carries: &[cranelift_codegen::ir::UserExternalName]) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let (bytes, len, s) = (p[0], p[1], p[2]);
    let k = Consts::make(&mut f);
    let file = crate::native_cpu_bytes::File { bytes, len, nibble: k.nibble, weights: k.weights, even: k.even };
    let mut heads = vec![];
    // The tests in the loop (`tests_in_read`): their outputs' words with the closer's.
    let inloop = tests_in_read(bw, a, tests);
    let touts = if inloop { read_test_outs(tests) } else { Vec::new() };
    let os: Vec<u16> = std::iter::once(a.closer).chain(touts.iter().map(|&(o, _)| o)).collect();
    let runs = bw.carried_runs(&os);
    let zero = f.ins().iconst(types::I64, 0);
    let p0 = f.ins().iadd_imm(s, 1);
    let init: Vec<Value> = runs.iter().map(|&r| if bw.apart(a.opener, r) { zero } else { bw.carry_at(&mut f, &file, r, p0, &mut heads) }).collect();
    let (head, found, none) = (f.create_block(), f.create_block(), f.create_block());
    f.set_cold_block(none);
    for _ in 0..1 + runs.len() + touts.len() {
        f.append_block_param(head, types::I64);
    }
    let args: Vec<BlockArg> = std::iter::once(p0).chain(init).chain(touts.iter().map(|_| zero)).map(BlockArg::from).collect();
    f.ins().jump(head, &args);
    f.switch_to_block(head);
    heads.push((head, Step::Word));
    let hp = f.block_params(head).to_vec();
    let at = hp[0];
    let cins: Vec<(usize, Value)> = runs.iter().copied().zip(hp[1..].iter().copied()).collect();
    let accs = hp[1 + runs.len()..].to_vec();
    let (ws, couts) = bw.words_carried(&mut f, &file, &os, at, &cins, &mut heads);
    let x = ws[0];
    // No closer in the word: the next, if in the file, with the carries out.
    let next = f.create_block();
    f.ins().brif(x, found, &[], next, &[]);
    f.switch_to_block(next);
    f.set_cold_block(next);
    let at1 = f.ins().iadd_imm(at, 64);
    let inside = f.ins().icmp(IntCC::SignedLessThan, at1, len);
    let more = f.create_block();
    f.ins().brif(inside, more, &[], none, &[]);
    f.switch_to_block(more);
    f.set_cold_block(more);
    // A window with no closer is the token's: its test bits all in.
    let accs1: Vec<Value> = touts.iter().zip(&accs).zip(&ws[1..]).map(|((&(_, n), &acc), &w)| read_test_acc(&mut f, n, acc, w)).collect();
    let back: Vec<BlockArg> = std::iter::once(at1).chain(couts).chain(accs1).map(BlockArg::from).collect();
    f.ins().jump(head, &back);
    f.switch_to_block(none);
    let no = f.ins().iconst(types::I64, -1);
    f.ins().return_(&[no, zero]);
    // The closer at `close`, the last `l` at `off` from it.
    f.switch_to_block(found);
    let tz = f.ins().ctz(x);
    let close = f.ins().iadd(at, tz);
    if inloop {
        // The closer's window up to the closer, its last (`x ^ (x - 1)`), into the tests.
        let x1 = f.ins().iadd_imm(x, -1);
        let upto = f.ins().bxor(x, x1);
        let fin: Vec<Value> = touts.iter().zip(&accs).zip(&ws[1..]).map(|((&(_, n), &acc), &w)| {
            let w = f.ins().band(w, upto);
            read_test_acc(&mut f, n, acc, w)
        }).collect();
        let got = read_test_hits(&mut f, tests, &touts, &fin);
        f.ins().return_(&[close, got]);
        f.seal_all_blocks();
        f.finalize();
        return crate::native_cpu_shape::check(func, &heads, carries).map_err(|r| Error::RefusedShape(format!("{r:?}")));
    }
    let l = f.ins().iadd_imm(close, i64::from(a.off));
    let call = f.create_block();
    if let Some(ns) = counts_need_by_offset(bw.native, tests) {
        // A token none of whose bytes, at each offset, is one the tests need there: no
        // hits, and no test's mask made. One of 64 bytes at most by its vectors, a longer
        // one 64 bytes at a time (`needs_chunks`).
        let (vec, long, quiet) = (f.create_block(), f.create_block(), f.create_block());
        f.set_cold_block(long);
        let n1 = f.ins().isub(l, s);
        let span = f.ins().iadd_imm(n1, 1);
        let fits = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, span, 64);
        f.ins().brif(fits, vec, &[], long, &[]);
        f.switch_to_block(vec);
        let some = needs_spans(&mut f, &ns, bytes, s, span);
        f.ins().brif(some, call, &[], quiet, &[]);
        f.switch_to_block(long);
        let some = needs_chunks(&mut f, &ns, bytes, s, span, &mut heads);
        f.ins().brif(some, call, &[], quiet, &[]);
        f.switch_to_block(quiet);
        f.ins().return_(&[close, zero]);
    } else {
        f.ins().jump(call, &[]);
    }
    // The tests' function, at a token with such bytes.
    f.switch_to_block(call);
    f.set_cold_block(call);
    let (_, id, sig) = counts;
    let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
    let signature = f.import_signature(sig.clone());
    let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
    let got = f.ins().call(callee, &[bytes, len, s, l]);
    let got = f.inst_results(got)[0];
    f.ins().return_(&[close, got]);
    f.seal_all_blocks();
    f.finalize();
    let mut own = carries.to_vec();
    own.push(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
    crate::native_cpu_shape::check(func, &heads, &own).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// Whether rule `k`, and a reader's other rule, read only outputs the bytes make
/// (`Bytewise::can`), their tests too: their reads wait on no block, so the rule can be a
/// function of its own (`rule_fn`).
fn rule_bytewise(r: &crate::records::Records, bw: &crate::native_cpu_bytes::Bytewise<'_>, k: usize) -> bool {
    use crate::records::EndRule;
    let own = |k: usize| {
        (match r.rules[k] {
            EndRule::Run(o) | EndRule::Pair(o) | EndRule::Last(o) => bw.can(o),
            EndRule::Read(i) => r.readers[usize::from(i)].arms.iter().all(|a| bw.can(a.opener) && bw.can(a.closer)),
            _ => true,
        }) && r.checks.iter().all(|c| usize::from(c.rule) != k || bw.can(c.o))
    };
    own(k) && match r.rules[k] {
        EndRule::Read(i) => r.rules.get(usize::from(r.readers[usize::from(i)].other)).is_none() || own(usize::from(r.readers[usize::from(i)].other)),
        _ => true,
    }
}

/// Rule `k` (`rule_last`'s block), a function of its own, as the lean lexer's rare tokens are
/// out of its run loop: `(bytes, len, blocks, s, raw, entry)` to the token's last, its refusal,
/// its tag and its tests' hits (`rule_last`'s). The loop calls it for a token of the rule; its
/// registers are the common tokens'.
#[allow(clippy::too_many_arguments)]
fn rule_fn(func: &mut cranelift_codegen::ir::Function, r: &crate::records::Records, t: &RecordTables, bw: &crate::native_cpu_bytes::Bytewise<'_>, outputs: usize, mode: RuleMode, calls: &[ByteCall], carries: &[cranelift_codegen::ir::UserExternalName]) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    // Its arguments, the first of `rule_args`' as many as its signature has: those past them
    // it never reads (`rule_reads`), here a zero no instruction reads.
    let p = f.block_params(entry).to_vec();
    let none = if p.len() < RULE_ARGS { Some(f.ins().iconst(types::I64, 0)) } else { None };
    let arg = |i: usize| p.get(i).copied().or(none).expect("an argument");
    let (bytes, s, len, blocks, raw, first) = (arg(0), arg(1), arg(2), arg(3), arg(4), arg(5));
    let consts = Consts::make(&mut f);
    let (tables, mult, slots) = tables_consts(&mut f, t);
    // The records' columns: not read by the rules.
    let zero = f.ins().iconst(types::I64, 0);
    let c = FileCols { bytes, starts: zero, lasts: zero, codes: zero, ns: zero, nl: zero, out: zero, counts: zero, blocks, len, tables, mult, slots, layout: t.layout, reads_back: Default::default(), arms: Default::default(), known_entries: Default::default() };
    let rd = Reads { bytes: Some((bw, crate::native_cpu_bytes::File { bytes, len, nibble: consts.nibble, weights: consts.weights, even: consts.even })), outputs, calls };
    let mut heads = vec![];
    let (exits, _) = rule_last(&mut f, r, t, &rd, &c, s, raw, (first, false), None, None, mode, Side::Both, &mut heads);
    let (l, refuse, tag, hits) = exit_values(&mut f, exits[0].1);
    if packs(r) {
        let got = pack_rule(&mut f, l, refuse, tag, hits);
        f.ins().return_(&got);
    } else {
        f.ins().return_(&[l, refuse, tag, hits]);
    }
    f.seal_all_blocks();
    f.finalize();
    let own: Vec<cranelift_codegen::ir::UserExternalName> = calls.iter().map(|x| cranelift_codegen::ir::UserExternalName::new(0, x.1.as_u32())).chain(carries.iter().cloned()).collect();
    crate::native_cpu_shape::check(func, &heads, &own).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// `escaped(a)`'s carry in (`Bytewise::carry_fn`), a function of its own: `(bytes, len, pos)`
/// to the parity of `a`'s run that ends at `pos - 1`, a window at a time.
fn carry_fn(func: &mut cranelift_codegen::ir::Function, bw: &crate::native_cpu_bytes::Bytewise<'_>, a: usize) -> Result<(), Error> {
    let mut builder_context = FunctionBuilderContext::new();
    let mut f = FunctionBuilder::new(func, &mut builder_context);
    let entry = f.create_block();
    f.append_block_params_for_function_params(entry);
    f.switch_to_block(entry);
    let p = f.block_params(entry).to_vec();
    let k = Consts::make(&mut f);
    let file = crate::native_cpu_bytes::File { bytes: p[0], len: p[1], nibble: k.nibble, weights: k.weights, even: k.even };
    let mut heads = vec![];
    let w = bw.carry_fn(&mut f, &file, a, p[2], &mut heads);
    f.ins().return_(&[w]);
    f.seal_all_blocks();
    f.finalize();
    crate::native_cpu_shape::check(func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))
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
    let cols = file_cols(&mut f, r, t, extracts, rr, rows, bytes_all, out_all, positions_all, counts_all);
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
fn file_cols(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, t: &RecordTables, extracts: usize, rr: Value, rows: Value, bytes_all: Value, out_all: Value, positions_all: Value, counts_all: Value) -> FileCols {
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
    let (starts, lasts, codes) = match t.layout {
        crate::records::RecordLayout::Columns => (column(i64::from(r.starts)), column(i64::from(r.lasts)), column(extracts as i64)),
        // Rows from the first column (`RecordLayout::index`): a token's fields 4 bytes apart.
        crate::records::RecordLayout::Rows => {
            let first = column(i64::from(r.starts.min(r.lasts)));
            (first, f.ins().iadd_imm(first, 4), f.ins().iadd_imm(first, 8))
        }
    };
    let ns = f.ins().load(types::I64, fl, counts, 8 * i32::from(r.starts));
    let nl = f.ins().load(types::I64, fl, counts, 8 * i32::from(r.lasts));
    let blocks = f.ins().load(types::I64, ro, at, 8);
    let len = f.ins().load(types::I64, ro, at, 16);
    let (tables, mult, slots) = tables_consts(f, t);
    FileCols { bytes, starts, lasts, codes, ns, nl, out, counts, blocks, tables, mult, slots, len, layout: t.layout, reads_back: Default::default(), arms: Default::default(), known_entries: Default::default() }
}

/// The record pass's tables' address, its word table's multiplier and its slots' address, as
/// constants (`clif_rust` names addresses inside the tables `TABLES`).
fn tables_consts(f: &mut FunctionBuilder<'_>, t: &RecordTables) -> (Value, Value, Value) {
    let tables = f.ins().iconst(types::I64, t.words.as_ptr() as i64);
    let mult = f.ins().iconst(types::I64, t.table.mult as i64);
    let slots = f.ins().iconst(types::I64, t.words.as_ptr() as i64 + t.slots as i64);
    (tables, mult, slots)
}

/// Where the token loop runs a block at a time (decision 0177): the blocks whose outputs are
/// final (`ready`), whether they are all the file's (`last`), and where the loop waits for more.
#[derive(Clone, Copy)]
struct Limit {
    ready: Value,
    last: Value,
    wait: cranelift_codegen::ir::Block,
}

/// The words a search may read: the ready blocks', or the file's.
fn bound(c: &FileCols, l: Option<Limit>) -> Value {
    l.map_or(c.blocks, |l| l.ready)
}

/// A search past the words it may read: at the file's end, on to `end` with `args`; before it,
/// the loop waits for the next block (`Limit::wait`).
fn past(f: &mut FunctionBuilder<'_>, l: Option<Limit>, end: cranelift_codegen::ir::Block, args: &[BlockArg]) {
    match l {
        None => {
            f.ins().jump(end, args);
        }
        Some(l) => {
            f.ins().brif(l.last, end, args, l.wait, &[]);
        }
    }
}

/// A branch on `inside`: to `to`, or past the words (`past`).
fn inside_or_past(f: &mut FunctionBuilder<'_>, inside: Value, to: cranelift_codegen::ir::Block, l: Option<Limit>, end: cranelift_codegen::ir::Block, args: &[BlockArg]) {
    let out = f.create_block();
    f.ins().brif(inside, to, &[], out, &[]);
    f.switch_to_block(out);
    f.set_cold_block(out);
    past(f, l, end, args);
}

/// How the token loop reads tier 1's outputs (decision 0177): from the file's bytes where the
/// output is made from them (`native_cpu_bytes`), else from the block's rows, which wait on the
/// blocks the loop may read (`Limit`).
#[derive(Clone, Copy)]
struct Reads<'r> {
    bytes: Option<(&'r crate::native_cpu_bytes::Bytewise<'r>, crate::native_cpu_bytes::File)>,
    outputs: usize,
    /// Each output's window as a function of its own (`bytes_fn`): called where the loop is not
    /// at its block's refill, which alone is inline (the main loop's).
    calls: &'r [ByteCall],
}

/// An output's window function (`bytes_fn`): the output, its id and its signature.
type ByteCall = (u16, cranelift_module::FuncId, cranelift_codegen::ir::Signature);

impl Reads<'_> {
    fn bytewise(&self, o: u16) -> bool {
        self.bytes.is_some_and(|(b, _)| b.can(o))
    }

    /// Output `o`'s word `q`, from the bytes by its function's call.
    fn word(&self, f: &mut FunctionBuilder<'_>, c: &FileCols, q: Value, o: u16, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        if let (Some(_), Some((_, id, sig))) = (self.bytes.filter(|(b, _)| b.can(o)), self.calls.iter().find(|x| x.0 == o)) {
            let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
            let signature = f.import_signature(sig.clone());
            let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
            let p = f.ins().ishl_imm(q, 6);
            let call = f.ins().call(callee, &[c.bytes, c.len, p]);
            return f.inst_results(call)[0];
        }
        self.word_inline(f, c, q, o, heads)
    }

    /// Outputs `os`' words `q`, inline: from the bytes, one window for all of them (its loads
    /// and classes shared, `Bytewise::words`), as the lean lexer's `Window::at` makes a
    /// block's masks from one load; else each from its row.
    fn words_inline(&self, f: &mut FunctionBuilder<'_>, c: &FileCols, q: Value, os: &[u16], heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Vec<Value> {
        match self.bytes.filter(|(b, _)| os.iter().all(|&o| b.can(o))) {
            Some((b, file)) => {
                let p = f.ins().ishl_imm(q, 6);
                b.words(f, &file, os, p, heads)
            }
            None => os.iter().map(|&o| self.word_inline(f, c, q, o, heads)).collect(),
        }
    }

    /// Output `o`'s word `q`, from the bytes inline (the refill's).
    fn word_inline(&self, f: &mut FunctionBuilder<'_>, c: &FileCols, q: Value, o: u16, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        match self.bytes.filter(|(b, _)| b.can(o)) {
            Some((b, file)) => {
                let p = f.ins().ishl_imm(q, 6);
                b.word(f, &file, o, p, heads)
            }
            None => {
                let off = f.ins().imul_imm(q, 8 * self.outputs as i64);
                let row = f.ins().iadd(c.out, off);
                f.ins().load(types::I64, MemFlags::trusted(), row, 8 * i32::from(o))
            }
        }
    }

    /// Output `o` at byte `s`, 0 or 1.
    fn bit(&self, f: &mut FunctionBuilder<'_>, c: &FileCols, s: Value, o: u16, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        self.bit_first(f, c, s, o, None, heads)
    }

    /// `bit` at a token's first byte `s`, known to be in `first` (`first_bytes`): from the
    /// bytes, its tests of that byte constants (`Bytewise::bit_first`).
    fn bit_first(&self, f: &mut FunctionBuilder<'_>, c: &FileCols, s: Value, o: u16, first: Option<[u64; 4]>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
        match self.bytes.filter(|(b, _)| b.can(o)) {
            Some((b, file)) => b.bit_first(f, &file, o, s, first, heads),
            None => {
                let q = f.ins().ushr_imm(s, 6);
                let x = self.word(f, c, q, o, heads);
                let sh = f.ins().band_imm(s, 63);
                let x = f.ins().ushr(x, sh);
                f.ins().band_imm(x, 1)
            }
        }
    }

    /// The blocks a read of output `o` waits on: none from the bytes.
    fn limit(&self, o: u16, l: Option<Limit>) -> Option<Limit> {
        if self.bytewise(o) { None } else { l }
    }
}

/// A token's last byte from its first, `s`, by its end rule (`records::last_by_rule`, decision
/// 0177): the rule by one load of its first byte, then a block a rule, each straight-line in its
/// first word; a run or a search past it goes on a word at a time, in a cold block. A search
/// that finds no last gives `s - 1`, which the order check refuses.
#[allow(clippy::too_many_arguments)]
fn rule_last(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, t: &RecordTables, rd: &Reads<'_>, c: &FileCols, s: Value, raw: Value, (entry, pure): (Value, bool), limit: Option<Limit>, regs: Option<&BlockRegs>, mode: RuleMode, side: Side, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (Vec<(Side, cranelift_codegen::ir::Block)>, Option<cranelift_codegen::ir::Block>) {
    use crate::records::EndRule;
    // The exits: one; or the rare starts' (`Side::Rare`), one a class of how their tokens end
    // (`TokenEnd`), each rule's arms going to its class's, as the lean lexer's arms each set
    // their `cursor` (the class's tail after it, `record_tokens`). The others' last.
    let forms: Vec<Side> = match side {
        Side::Rare(_) => {
            let mut v: Vec<Side> = (0..r.rules.len()).map(|k| Side::Rare(Some(RareClass { end: t.ends.get(k).copied().unwrap_or(TokenEnd::Any), tagged: tagged(r, k), rule: k }))).collect();
            v.push(Side::Rare(Some(RareClass { end: TokenEnd::Any, tagged: false, rule: NO_RULE })));
            let mut seen = Vec::new();
            v.retain(|x| !seen.contains(x) && {
                seen.push(*x);
                true
            });
            v
        }
        x => vec![x],
    };
    let exits: Vec<(Side, cranelift_codegen::ir::Block, cranelift_codegen::ir::Block)> = forms.iter().map(|&x| {
        // The last, and a refusal (its sign bit): a byte with no rule, where errors by tokens
        // refuse.
        let (ended, done) = (f.create_block(), f.create_block());
        f.append_block_param(ended, types::I64);
        f.append_block_param(ended, types::I64);
        // And a refusal (`read_none`'s: a reader with no closer).
        f.append_block_param(ended, types::I64);
        f.append_block_param(done, types::I64);
        f.append_block_param(done, types::I64);
        f.append_block_param(done, types::I64);
        // And the marks' tests' hits (`records::check_hits`).
        f.append_block_param(done, types::I64);
        (x, ended, done)
    }).collect();
    let exit_of = |k: usize| match side {
        Side::Rare(_) => {
            let rule = if k < r.rules.len() { k } else { NO_RULE };
            let x = exits.iter().find(|x| matches!(x.0, Side::Rare(Some(c)) if c.rule == rule)).expect("the arm's exit");
            (x.1, x.2)
        }
        _ => (exits[0].1, exits[0].2),
    };
    let (_, done) = exit_of(usize::MAX);
    // The tag (`Reader::retag`): a reader's kind and marks for its token, `-1` for none.
    let untag = f.ins().iconst(types::I64, -1);
    let nohits = f.ins().iconst(types::I64, 0);
    // Output `o`'s word `q` (`q` below the file's blocks).
    let word_at = |f: &mut FunctionBuilder<'_>, q: Value, o: u16, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>| rd.word(f, c, q, o, heads);
    let q0 = f.ins().ushr_imm(s, 6);
    let sh = f.ins().band_imm(s, 63);
    let blocks: Vec<cranelift_codegen::ir::Block> = r.rules.iter().map(|_| f.create_block()).collect();
    // The two common rules, if-converted (decision 0177): a pair's (`Next`) and a run's in a
    // register (`Run`), each the rule most first bytes take of its form, where their tests
    // are all by first-byte bits (`TOKEN_TESTS`, each on its rule's bytes alone). Both lasts
    // straight-line and one `select`, no branch between them: a run past its block's word
    // goes to its rule's block. Most tokens are of the two (`.rerail`: punctuation and words).
    let reg_of = |o: u16| regs.and_then(|g| g.runs.iter().find(|&&(k, _)| k == o).map(|&(_, v)| v));
    // The fused rules' exit of their own (`fdone`, as `done`'s): their tokens are never a
    // reader's or an extra, so the loop's tail for them knows it (the lean lexer's run loop).
    let mut fused_exit = None;
    let in_regs: Vec<u16> = regs.map(|g| g.runs.iter().map(|&(k, _)| k).collect()).unwrap_or_default();
    let fused = fused_rules(r, t, rd, &in_regs).filter(|_| !matches!(side, Side::Rare(_)));
    // The run loop's head (`Side::Fused`): a token not of the fused rules is the run rule's
    // past its block (the lean lexer's `word_end`), its block alone.
    let mut only = match mode {
        RuleMode::Only(k) | RuleMode::Short(k) => Some(k),
        _ => None,
    };
    if let Some((kn, kr)) = fused {
        if side == Side::Fused {
            only = Some(kr);
        }
        let (EndRule::Next(pi), EndRule::Run(o)) = (r.rules[kn], r.rules[kr]) else { unreachable!("the forms chosen") };
        let fdone = f.create_block();
        for _ in 0..4 {
            f.append_block_param(fdone, types::I64);
        }
        fused_exit = Some(fdone);
        let (go, table, on) = (f.create_block(), f.create_block(), f.create_block());
        let tests: Vec<(crate::records::TokenCheck, Option<i64>)> = r.checks.iter().zip(&t.filters).filter(|(x, _)| usize::from(x.rule) == kn || usize::from(x.rule) == kr).map(|(x, &b)| (*x, b)).collect();
        // The token's last, made in `on` after the tests' blocks (`tested` is built from an
        // empty block), on to the tests or the exit.
        let finish = |f: &mut FunctionBuilder<'_>, last: &dyn Fn(&mut FunctionBuilder<'_>) -> Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>| {
            if tests.is_empty() {
                let l = last(f);
                f.ins().jump(fdone, &[l.into(), nohits.into(), untag.into(), nohits.into()]);
            } else {
                let (to, _) = tested(f, &tests, rd, c, s, entry, fdone, None, &[], 1, None, None, heads);
                let l = last(f);
                f.ins().jump(to, &[l.into(), untag.into(), nohits.into()]);
            }
        };
        let pair_word = reg_of(PAIR_WORD);
        let v = reg_of(o).expect("a register word");
        if let (Some(pw), Some(_), true) = (pair_word, t.lean, t.fused == Some((kn, kr))) {
            // The lean lexer's starts (`lean_starts`): a run token's first byte is a run byte, a
            // pair's is not, and the pair word is set at a pair's first byte alone. Then the
            // token's length is lean's `words.max(n1)`: the run from its first byte (none at a
            // pair's), and 1 or, at the pair word's bit, 2. One branch, to the run rule's block
            // where the run reaches the block's end (lean's `at + words == 64`), or to the rules'
            // table where the entry names neither rule.
            let either = f.ins().band_imm(entry, 1 << TOKEN_FUSED_PAIR | 1 << TOKEN_FUSED_RUN);
            let y = f.ins().ushr(v, sh);
            let y = f.ins().bnot(y);
            let words = f.ins().ctz(y);
            let to = f.ins().iadd(sh, words);
            // The common way first, the run past the block after it (its code laid out after the
            // loop's, as `word_end`'s byte loop is after lean's run loop).
            let inside = f.ins().icmp_imm(IntCC::UnsignedLessThan, to, 64);
            if side == Side::Fused {
                // The table is the run rule's block alone (`only`).
                let named = f.ins().icmp_imm(IntCC::NotEqual, either, 0);
                let fast = f.ins().band(inside, named);
                f.ins().brif(fast, on, &[], table, &[]);
            } else {
                f.ins().brif(either, go, &[], table, &[]);
                f.switch_to_block(go);
                f.ins().brif(inside, on, &[], blocks[kr], &[]);
            }
            f.switch_to_block(on);
            finish(f, &|f: &mut FunctionBuilder<'_>| {
                let bit = f.ins().ushr(pw, sh);
                let bit = f.ins().band_imm(bit, 1);
                let n1 = f.ins().iadd_imm(bit, 1);
                let len = f.ins().umax(words, n1);
                let end = f.ins().iadd(s, len);
                f.ins().iadd_imm(end, -1)
            }, heads);
        } else {
            // By the entry's bits where they name these rules (`TOKEN_FUSED_*`): a bit test, no
            // compare of the rule, which a compiler would make one switch with the table's.
            let (is_n, is_r, either) = if t.fused == Some((kn, kr)) {
                let bit = |f: &mut FunctionBuilder<'_>, b: i64| {
                    let x = f.ins().band_imm(entry, 1 << b);
                    f.ins().icmp_imm(IntCC::NotEqual, x, 0)
                };
                let (n, r) = (bit(f, TOKEN_FUSED_PAIR), bit(f, TOKEN_FUSED_RUN));
                let x = f.ins().band_imm(entry, 1 << TOKEN_FUSED_PAIR | 1 << TOKEN_FUSED_RUN);
                (n, r, x)
            } else {
                let rule = rule_in(f, entry);
                let n = f.ins().icmp_imm(IntCC::Equal, rule, kn as i64);
                let r = f.ins().icmp_imm(IntCC::Equal, rule, kr as i64);
                (n, r, f.ins().bor(n, r))
            };
            f.ins().brif(either, go, &[], table, &[]);
            f.switch_to_block(go);
            // The pair's second byte: the block's pair word's bit (`PAIR_WORD`), as the lean
            // lexer's; else the table's row for the next byte.
            let l_n = match pair_word {
                Some(w) => {
                    let bit = f.ins().ushr(w, sh);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().iadd(s, bit)
                }
                None => next_last(f, t, c, s, raw, entry, pi),
            };
            // The run from `s + 1` in the block's word, by two shifts (none past the block): whole
            // where it ends inside the block, as the lean lexer's `run`.
            let y = f.ins().ushr(v, sh);
            let y = f.ins().ushr_imm(y, 1);
            let y = f.ins().bnot(y);
            let ones = f.ins().ctz(y);
            let to = f.ins().iadd(sh, ones);
            let past = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, to, 63);
            let l_r = f.ins().iadd(s, ones);
            let slow = f.ins().band(is_r, past);
            f.ins().brif(slow, blocks[kr], &[], on, &[]);
            f.switch_to_block(on);
            finish(f, &|f: &mut FunctionBuilder<'_>| f.ins().select(is_n, l_n, l_r), heads);
        }
        f.switch_to_block(table);
    }
    // The rule by a jump table, as the lean lexer's `match` on the first byte; none, the byte
    // alone (`records::NO_RULE`, past the table).
    // Rule `k` alone: its block, and the rules it goes on to.
    let needed: Vec<usize> = match (only, side) {
        (Some(k), _) => {
            let mut v = vec![k];
            let mut at = 0;
            while at < v.len() {
                if let EndRule::Read(i) = r.rules[v[at]] {
                    let o = usize::from(r.readers[usize::from(i)].other);
                    if o < r.rules.len() && !v.contains(&o) {
                        v.push(o);
                    }
                }
                at += 1;
            }
            v
        }
        _ => (0..r.rules.len()).collect(),
    };
    if let Some(k) = only {
        f.ins().jump(blocks[k], &[]);
    } else {
        let none = f.create_block();
        // A rare start (`Side::Rare`): its rule's block by its first byte, as the lean lexer's
        // `match` on it; else by the rule in its entry (`RecordTables::token`).
        let (index, calls): (Value, Vec<_>) = if let Side::Rare(_) = side {
            let byte = f.ins().band_imm(raw, 0xff);
            let byte = f.ins().ireduce(types::I32, byte);
            // Only a rare byte comes here (`mid`'s rare word): the others none's.
            let calls = (0..256).map(|b| {
                let to = r.ends.get(b).map(|&e| usize::from(e)).filter(|&k| k < r.rules.len() && t.is_rare(b as u8)).map_or(none, |k| blocks[k]);
                f.func.dfg.block_call(to, &[])
            }).collect();
            (byte, calls)
        } else {
            (rule_in(f, entry), blocks.iter().map(|&b| f.func.dfg.block_call(b, &[])).collect())
        };
        let default = f.func.dfg.block_call(none, &[]);
        let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
        f.ins().br_table(index, jt);
        f.switch_to_block(none);
        f.set_cold_block(none);
        let refuse = f.ins().iconst(types::I64, if r.untaken.is_empty() || r.skip.is_none() { 0 } else { -1 });
        f.ins().jump(done, &[s.into(), refuse.into(), untag.into(), nohits.into()]);
    }
    for (k, (&rule, &b)) in r.rules.iter().zip(&blocks).enumerate() {
        if !needed.contains(&k) {
            continue;
        }
        f.switch_to_block(b);
        // A rare arm whose first bytes share one entry (`rare_entry`): that entry, as the lean
        // lexer's arm knows its byte, its tests of it the compiler's.
        let entry = match (side, pure) {
            (Side::Rare(_), true) => match rare_entry(r, t, RareClass { end: TokenEnd::Any, tagged: false, rule: k }) {
                Some(e) => {
                    // The rule's body in a block of its own, empty as its builders expect
                    // (`tested` leaves it); the straight jump is merged (`clif_fold`).
                    let e = f.ins().iconst(types::I64, e as i64);
                    let body = f.create_block();
                    f.ins().jump(body, &[]);
                    f.switch_to_block(body);
                    e
                }
                None => entry,
            },
            _ => entry,
        };
        // The rule's exits: its class's (`exit_of`).
        let (ended, done) = exit_of(k);
        // A run rule with a short form (`RuleMode::Short`, its whole function `LONG_CALL`): that
        // form here, in the arm, as the lean lexer reads a number in its `match` arm; a token
        // past its 16 bytes calls the whole function. A reader keeps its call, as the lean
        // lexer's string calls `string_end`.
        let arm = matches!(mode, RuleMode::Calls) && matches!(rule, EndRule::Run(o) if rd.bytewise(o)) && rd.calls.iter().any(|x| x.0 == LONG_CALL | k as u16);
        // A reader with a short form likewise: its openers' tests and its first window here,
        // as the lean lexer's `b'/'` arm tests the byte after and scans a line comment in place;
        // a read past the window calls the whole function, as that arm calls `find_pair`.
        let arm = arm || matches!(mode, RuleMode::Calls) && matches!(rule, EndRule::Read(_)) && rd.calls.iter().any(|x| x.0 == LONG_CALL | k as u16);
        // `arm`, where a reader's openers name their own (`in_arm`).
        let in_arm = arm;
        // A short form's long way (`RuleMode::Short`): its whole function's results, returned
        // from the short function, or on to the arm's exit (`done`), as a call's.
        let long_out = |f: &mut FunctionBuilder<'_>, got: Vec<Value>| {
            if matches!(mode, RuleMode::Short(_)) {
                f.ins().return_(&got);
            } else {
                let got = if packs(r) { unpack_rule(f, got[0], got[1]).to_vec() } else { got };
                let got: Vec<BlockArg> = got.iter().map(|&v| v.into()).collect();
                f.ins().jump(done, &got);
            }
        };
        // A rule with a function of its own, but the fused two: its call (`rule_fn`).
        let own = rd.calls.iter().find(|x| x.0 == RULES_CALL | k as u16).filter(|_| !arm);
        if arm {
            c.arms.set(c.arms.get() + 1);
        }
        if let (RuleMode::Calls, Some((_, id, sig)), false) = (mode, own, fused.is_some_and(|(kn, kr)| k == kn || k == kr)) {
            f.set_cold_block(b);
            let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
            let signature = f.import_signature(sig.clone());
            let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
            let call = f.ins().call(callee, &rule_args(sig, [c.bytes, s, c.len, c.blocks, raw, entry]));
            let mut got = f.inst_results(call).to_vec();
            if packs(r) {
                got = unpack_rule(f, got[0], got[1]).to_vec();
            }
            let got: Vec<BlockArg> = got.iter().map(|&v| v.into()).collect();
            f.ins().jump(done, &got);
            continue;
        }
        // A rule with token tests (`Records::checks`) ends at a block of its own, which tests
        // the token, as a lexer checks the token it reads.
        let tests: Vec<(crate::records::TokenCheck, Option<i64>)> = r.checks.iter().zip(&t.filters).filter(|(c, _)| usize::from(c.rule) == k).map(|(c, &b)| (*c, b)).collect();
        // A short run's token is 16 bytes at most (`RuleMode::Short`).
        let short = (arm || matches!(mode, RuleMode::Short(_))) && matches!(rule, EndRule::Run(o) if rd.bytewise(o)) && rd.calls.iter().any(|x| x.0 == LONG_CALL | k as u16);
        // (Its longest by the split's form: `term_form`'s, below.)
        let most16 = short.then_some(16);
        // A run read from the bytes (`short_word`) and its count tests of a byte set its first
        // bytes never are (`run_tests`): their bits from one window, the run's, as the lean
        // lexer's number arm finds its `.` among its digits.
        let reg_o = |o: u16| regs.and_then(|g| g.runs.iter().find(|&&(k, _)| k == o));
        // A run of a byte set with a rarer way on (`Bytewise::run_split`): the set's run from
        // the window, the rest asked at its end, as the lean lexer's number arm runs its digits
        // and then asks for `.` and a digit.
        let split = match (rule, rd.bytes) {
            (EndRule::Run(o), Some((b, _))) if b.can(o) && reg_o(o).is_none() => b.run_split(o),
            _ => None,
        };
        // Its count tests known on each of its ways (`split_counts`), where the tests' tail
        // (`tested`'s `rest`) takes them: none counted, as lean's number arm knows a float by
        // the branch it took.
        let fused_counts = rd.calls.iter().any(|x| x.0 == TESTS_CALL | k as u16) && tests.iter().any(|(t, _)| t.count && !t.always());
        let by_way = match (rule, rd.bytes, split) {
            (EndRule::Run(o), Some((b, _)), Some(sp)) if fused_counts => split_counts(r, b, k, &tests, o, sp),
            _ => None,
        };
        let run_outs: Vec<u16> = match (rule, rd.bytes) {
            (EndRule::Run(o), Some((b, _))) if b.can(o) && reg_o(o).is_none() && by_way.is_none() => run_tests(r, b, k, &tests),
            _ => Vec::new(),
        };
        // The window's terms: the run's (its split's byte term, else its output), its tests'.
        let run_terms = |b: &crate::native_cpu_bytes::Bytewise<'_>, o: u16| -> Vec<(usize, i64)> {
            let term = |o: u16| (usize::from(b.native.outputs[usize::from(o)]), i64::from(b.native.output_delays[usize::from(o)]));
            let first = split.map_or_else(|| term(o), |(y, d, _)| (y, d));
            std::iter::once(first).chain(run_outs.iter().map(|&o| term(o))).collect()
        };
        // The split's term by the experiment's form (`run_form`), where its counts are by way:
        // its block's word (`BlockRegs::terms`), or its bytes one at a time.
        let term_form = match (split, by_way, run_form()) {
            (Some((_, _, set)), Some(_), RunForm::Block) if regs.is_some_and(|g| g.terms.iter().any(|t| t.0 == set)) => RunForm::Block,
            (Some((_, _, set)), Some(_), RunForm::Bytes) if set[0] & 1 == 0 => RunForm::Bytes,
            _ => RunForm::Window,
        };
        // A token of the window's 16 bytes at most; of the block's 64 by its word. By its bytes
        // a run of any length, but its tests as the window's: a longer token takes the whole
        // rule's way, cold, as lean's `end - pos < 19 || parse()` (its tests made inline would
        // read the block's words past the token, a call the arm does not make).
        let most = match term_form {
            RunForm::Window | RunForm::Bytes => most16,
            RunForm::Block => most16.map(|_| 64),
        };
        let window: Option<Vec<Value>> = match (rule, rd.bytes) {
            (EndRule::Run(o), Some((b, file))) if !run_outs.is_empty() || split.is_some() && term_form == RunForm::Window => {
                let i = f.ins().iadd_imm(s, 1);
                let w = b.short_terms(f, &file, &run_terms(b, o), i, heads);
                // `tested` from a block of its own, empty (its blocks first).
                let after = f.create_block();
                f.ins().jump(after, &[]);
                f.switch_to_block(after);
                Some(w)
            }
            _ => None,
        };
        let pre: Vec<(u16, Value)> = window.as_ref().map(|w| run_outs.iter().copied().zip(w[1..].iter().copied()).collect()).unwrap_or_default();
        // The window's bits a lane (`short_stride`).
        let pre_stride = match (rule, rd.bytes) {
            (EndRule::Run(o), Some((b, _))) if window.is_some() => b.short_stride_x(&run_terms(b, o).iter().map(|t| t.0).collect::<Vec<_>>()),
            _ => 1,
        };
        // The split's term's lanes, for the tests of the token's bytes (`Bytewise::bit_on_run`):
        // the window's (its 16 lanes all known).
        let lanes = match (split, &window, term_form) {
            (Some((_, _, set)), Some(w), RunForm::Window) => Some(crate::native_cpu_bytes::RunLanes { mask: w[0], stride: pre_stride, set, known: 16 }),
            _ => None,
        };
        // The byte loop's: whether its first `while` ran a turn (the byte after the first in
        // the set), given the tests' tail by each way's end (`tested`'s `way_lane`).
        let way_lane = match (split, by_way, term_form) {
            (Some((_, _, set)), Some(_), RunForm::Bytes) => Some(set),
            _ => None,
        };
        let firsts_k = first_bytes(r, k);
        let (ended, rest) = if tests.is_empty() { (ended, None) } else { tested(f, &tests, rd, c, s, entry, done, most, &pre, pre_stride, lanes.map(|l| (l, firsts_k)), way_lane.map(|w| (w, firsts_k)), heads) };
        debug_assert!(by_way.is_none() || rest.is_some(), "a split's counts by way with no tests' tail");
        // The split's term as the arm runs it, made after `tested`'s blocks (it starts from an
        // empty one).
        let term: Option<TermRun> = match (split, term_form, &window) {
            (Some(_), RunForm::Window, Some(w)) => {
                let cap = f.ins().iconst(types::I64, 16);
                Some(TermRun::Mask { x: w[0], stride: pre_stride, cap })
            }
            (Some((_, _, set)), RunForm::Block, _) => {
                let word = regs.and_then(|g| g.terms.iter().find(|t| t.0 == set)).map(|t| t.1).expect("the term's word");
                let sh = f.ins().band_imm(s, 63);
                let sh1 = f.ins().iadd_imm(sh, 1);
                let x = f.ins().ushr(word, sh1);
                // Its lanes to the block's last byte; a run there takes the long way.
                let cap = f.ins().irsub_imm(sh, 63);
                Some(TermRun::Mask { x, stride: 1, cap })
            }
            (Some(_), RunForm::Bytes, _) => Some(TermRun::Bytes),
            _ => None,
        };
        match rule {
            // `pair "AB" …`: the next byte, if inside the file, one of the pairs' seconds.
            // `pair P`: the first byte's row (its entry's), then the next byte's bit in the
            // row's set, if inside the file (`PairTable`): the next byte from the token's
            // bytes, one load of the set.
            EndRule::Next(i) => {
                let l = next_last(f, t, c, s, raw, entry, i);
                f.ins().jump(ended, &[l.into(), untag.into(), nohits.into()]);
            }
            EndRule::Pair(o) => {
                let x = word_at(f, q0, o, heads);
                let bit = f.ins().ushr(x, sh);
                let bit = f.ins().band_imm(bit, 1);
                let l = f.ins().iadd(s, bit);
                f.ins().jump(ended, &[l.into(), untag.into(), nohits.into()]);
            }
            EndRule::Run(o) => {
                let limit = rd.limit(o, limit);
                // The run from `s + 1`: its first word's ones from there, then whole words.
                let (more, whole) = (f.create_block(), f.create_block());
                f.append_block_param(more, types::I64);
                let i = f.ins().iadd_imm(s, 1);
                let q = f.ins().ushr_imm(i, 6);
                let (first_word, next) = (f.create_block(), f.create_block());
                let isn = f.ins().band_imm(i, 63);
                // The block's word, in a register: but where the run starts in the next block
                // (the token at the block's last byte).
                let reg = regs.and_then(|g| g.runs.iter().find(|&&(k, _)| k == o).map(|&(_, v)| v));
                // A run on past the word in the register: its next bytes by one vector
                // (`Bytewise::short_word`) from the next block's first, as the lean lexer's
                // `word_end` reads a word's few bytes past its window; past those 16, whole
                // words below. Where the output is the bytes'.
                let cont_of = match (reg, rd.bytes.filter(|(b, _)| b.can(o))) {
                    (Some(_), Some((b, file))) => {
                        let cont = f.create_block();
                        f.append_block_param(cont, types::I64);
                        Some((cont, b, file))
                    }
                    _ => None,
                };
                let cont = cont_of.map(|(b, _, _)| b);
                let x = match reg {
                    Some(v) => {
                        let (in_reg, edge) = (f.create_block(), f.create_block());
                        f.append_block_param(first_word, types::I64);
                        f.ins().brif(isn, in_reg, &[], edge, &[]);
                        f.switch_to_block(in_reg);
                        f.ins().jump(first_word, &[v.into()]);
                        f.switch_to_block(edge);
                        f.set_cold_block(edge);
                        if let Some(cont) = cont {
                            // The run from the next block's first byte.
                            f.ins().jump(cont, &[i.into()]);
                            f.switch_to_block(first_word);
                            f.block_params(first_word)[0]
                        } else {
                        let inside = f.ins().icmp(IntCC::UnsignedLessThan, q, bound(c, limit));
                        let load = f.create_block();
                        inside_or_past(f, inside, load, limit, ended, &[s.into(), untag.into(), nohits.into()]);
                        f.switch_to_block(load);
                        f.set_cold_block(load);
                        let x = word_at(f, q, o, heads);
                        f.ins().jump(first_word, &[x.into()]);
                        f.switch_to_block(first_word);
                        f.block_params(first_word)[0]
                        }
                    }
                    None => {
                        // From the bytes: the run's first 16 from `s + 1` by one vector
                        // (`Bytewise::short_word`), as a lexer reads a number's few digits;
                        // the run on past them, the word's way below (cold).
                        if let Some((b, file)) = rd.bytes.filter(|(b, _)| b.can(o)) {
                            let long = f.create_block();
                            match (split, term) {
                                (Some((a, _, set)), Some(term)) => {
                                    // A way's end: its counts known (`by_way`), on to the tests'
                                    // tail with them, else the tests from the start.
                                    // `lane`, the byte loop's first lane (`way_lane`).
                                    let way_end = |f: &mut FunctionBuilder<'_>, way: usize, l: Value, lane: Option<Value>| -> (cranelift_codegen::ir::Block, Vec<BlockArg>) {
                                        match (by_way, rest) {
                                            (Some(ways), Some(tail)) => {
                                                let (no, hits) = ways[way];
                                                let always = always_hits(tests.iter().map(|(t, _)| t));
                                                let refuse = if no || always.0 != 0 { f.ins().iconst(types::I64, -1) } else { nohits };
                                                let hits = f.ins().iconst(types::I64, always.1 | hits);
                                                let mut args = vec![l.into(), untag.into(), refuse.into(), hits.into()];
                                                args.extend(lane.map(BlockArg::from));
                                                (tail, args)
                                            }
                                            _ => (ended, vec![l.into(), untag.into(), nohits.into()]),
                                        }
                                    };
                                    let firsts = first_bytes(r, k);
                                    let before: [u64; 4] = std::array::from_fn(|w| set[w] | firsts[w]);
                                    let before2 = b.split_before_again(o, a, set);
                                    match term {
                                        TermRun::Mask { x, stride, cap } => {
                                            // The byte term's run ended inside the mask's known
                                            // lanes: the rest asked at its end (lean's
                                            // `b[end] == b'.' && digit(b[end + 1])`).
                                            let ones = lanes_set(f, x, stride);
                                            let short = f.ins().icmp(IntCC::UnsignedLessThan, ones, cap);
                                            let l = f.ins().iadd(s, ones);
                                            let (at_end, on, end2) = (f.create_block(), f.create_block(), f.create_block());
                                            f.ins().brif(short, at_end, &[], long, &[]);
                                            f.switch_to_block(at_end);
                                            let e = f.ins().iadd(i, ones);
                                            let more = b.rest_after(f, &file, o, a, e, before, heads);
                                            let (to, args) = way_end(f, 0, l, None);
                                            f.ins().brif(more, on, &[], to, &args);
                                            // On past it: the term's run from the byte after, in
                                            // the same mask (lean's second `while digit`), else
                                            // the whole rule's way.
                                            f.switch_to_block(on);
                                            let n1 = f.ins().iadd_imm(ones, 1);
                                            let inside = f.ins().icmp(IntCC::UnsignedLessThan, n1, cap);
                                            let second = f.create_block();
                                            f.ins().brif(inside, second, &[], long, &[]);
                                            f.switch_to_block(second);
                                            let sh = f.ins().imul_imm(n1, i64::from(stride));
                                            let gone = f.ins().bnot(x);
                                            let gone = f.ins().ushr(gone, sh);
                                            let z = f.ins().ctz(gone);
                                            let ones2 = if stride > 1 { f.ins().ushr_imm(z, i64::from(stride.trailing_zeros())) } else { z };
                                            let total = f.ins().iadd(n1, ones2);
                                            let fits = f.ins().icmp(IntCC::UnsignedLessThan, total, cap);
                                            f.ins().brif(fits, end2, &[], long, &[]);
                                            // Its end: where the rest holds again, the whole
                                            // rule's way.
                                            f.switch_to_block(end2);
                                            let e2 = f.ins().iadd(i, total);
                                            let again = b.rest_after(f, &file, o, a, e2, before2, heads);
                                            let l2 = f.ins().iadd(s, total);
                                            let (to, args) = way_end(f, 1, l2, None);
                                            f.ins().brif(again, long, &[], to, &args);
                                        }
                                        TermRun::Bytes => {
                                            // Lean's `while digit { end += 1 }`, twice: a byte a
                                            // time to the first not in the set (the zeros past
                                            // the file are none of it).
                                            let run_bytes = |f: &mut FunctionBuilder<'_>, from: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>| -> Value {
                                                let (lp, out) = (f.create_block(), f.create_block());
                                                f.append_block_param(lp, types::I64);
                                                f.append_block_param(out, types::I64);
                                                f.ins().jump(lp, &[from.into()]);
                                                f.switch_to_block(lp);
                                                heads.push((lp, Step::Bytes));
                                                let e = f.block_params(lp)[0];
                                                let isin = b.byte_in(f, &file, e, set);
                                                let e1 = f.ins().iadd_imm(e, 1);
                                                f.ins().brif(isin, lp, &[e1.into()], out, &[e.into()]);
                                                f.switch_to_block(out);
                                                f.block_params(out)[0]
                                            };
                                            // A token past `most` bytes: the whole rule's way.
                                            let within = |f: &mut FunctionBuilder<'_>, e: Value| {
                                                if let Some(m) = most {
                                                    let n = f.ins().isub(e, s);
                                                    let fits = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, n, i64::from(m));
                                                    let go = f.create_block();
                                                    f.ins().brif(fits, go, &[], long, &[]);
                                                    f.switch_to_block(go);
                                                }
                                            };
                                            let e = run_bytes(f, i, heads);
                                            // Whether it ran a turn: the byte after the first in
                                            // the set, the tests' lane (`way_lane`).
                                            let lane = way_lane.map(|_| {
                                                let ran = f.ins().icmp(IntCC::NotEqual, e, i);
                                                f.ins().uextend(types::I64, ran)
                                            });
                                            let l = f.ins().iadd_imm(e, -1);
                                            let more = b.rest_after(f, &file, o, a, e, before, heads);
                                            let (on, end1) = (f.create_block(), f.create_block());
                                            f.ins().brif(more, on, &[], end1, &[]);
                                            f.switch_to_block(end1);
                                            within(f, e);
                                            let (to, args) = way_end(f, 0, l, lane);
                                            f.ins().jump(to, &args);
                                            f.switch_to_block(on);
                                            let from = f.ins().iadd_imm(e, 1);
                                            let e2 = run_bytes(f, from, heads);
                                            within(f, e2);
                                            let again = b.rest_after(f, &file, o, a, e2, before2, heads);
                                            let l2 = f.ins().iadd_imm(e2, -1);
                                            let (to, args) = way_end(f, 1, l2, lane);
                                            f.ins().brif(again, long, &[], to, &args);
                                        }
                                    }
                                }
                                _ => {
                                    let terms = run_terms(b, o);
                                    let x = match &window {
                                        Some(w) => w[0],
                                        None => b.short_terms(f, &file, &terms[..1], i, heads)[0],
                                    };
                                    let stride = b.short_stride_x(&terms.iter().map(|t| t.0).collect::<Vec<_>>());
                                    let ones = lanes_set(f, x, stride);
                                    let short = f.ins().icmp_imm(IntCC::UnsignedLessThan, ones, 16);
                                    let l = f.ins().iadd(s, ones);
                                    f.ins().brif(short, ended, &[l.into(), untag.into(), nohits.into()], long, &[]);
                                }
                            }
                            f.switch_to_block(long);
                            f.set_cold_block(long);
                            // The short function's long run: its whole function's, returned.
                            if let (RuleMode::Short(_) | RuleMode::Calls, true, Some((_, id, sig))) = (mode, arm || matches!(mode, RuleMode::Short(_)), rd.calls.iter().find(|x| x.0 == LONG_CALL | k as u16)) {
                                let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
                                let signature = f.import_signature(sig.clone());
                                let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
                                let call = f.ins().call(callee, &rule_args(sig, [c.bytes, s, c.len, c.blocks, raw, entry]));
                                let got = f.inst_results(call).to_vec();
                                if matches!(mode, RuleMode::Short(_)) {
                                    f.ins().return_(&got);
                                } else {
                                    // In the arm: its results on, as a call's (`done`).
                                    let got = if packs(r) { unpack_rule(f, got[0], got[1]).to_vec() } else { got };
                                    let got: Vec<BlockArg> = got.iter().map(|&v| v.into()).collect();
                                    f.ins().jump(done, &got);
                                }
                                continue;
                            }
                        }
                        let inside = f.ins().icmp(IntCC::UnsignedLessThan, q, bound(c, limit));
                        inside_or_past(f, inside, first_word, limit, ended, &[s.into(), untag.into(), nohits.into()]);
                        f.switch_to_block(first_word);
                        word_at(f, q, o, heads)
                    }
                };
                let y = f.ins().ushr(x, isn);
                let y = f.ins().bnot(y);
                let ones = f.ins().ctz(y);
                let left = f.ins().irsub_imm(isn, 64);
                let short = f.ins().icmp(IntCC::UnsignedLessThan, ones, left);
                let l = f.ins().iadd(s, ones);
                let w1 = f.ins().iadd_imm(q, 1);
                f.ins().brif(short, ended, &[l.into(), untag.into(), nohits.into()], next, &[]);
                f.switch_to_block(next);
                f.set_cold_block(next);
                match cont {
                    Some(cont) => {
                        let p = f.ins().ishl_imm(w1, 6);
                        f.ins().jump(cont, &[p.into()]);
                    }
                    None => {
                        f.ins().jump(more, &[w1.into()]);
                    }
                }
                // A whole word of the run: on to the next, or the file's end.
                f.switch_to_block(more);
                f.set_cold_block(more);
                heads.push((more, Step::Word));
                let w = f.block_params(more)[0];
                let inside = f.ins().icmp(IntCC::UnsignedLessThan, w, bound(c, limit));
                let w64 = f.ins().ishl_imm(w, 6);
                let at_end = f.ins().iadd_imm(w64, -1);
                inside_or_past(f, inside, whole, limit, ended, &[at_end.into(), untag.into(), nohits.into()]);
                f.switch_to_block(whole);
                f.set_cold_block(whole);
                let x = word_at(f, w, o, heads);
                let y = f.ins().bnot(x);
                let ones = f.ins().ctz(y);
                let full = f.ins().icmp_imm(IntCC::Equal, ones, 64);
                let l = f.ins().iadd(w64, ones);
                let l = f.ins().iadd_imm(l, -1);
                let w1 = f.ins().iadd_imm(w, 1);
                f.ins().brif(full, more, &[w1.into()], ended, &[l.into(), untag.into(), nohits.into()]);
                if let Some((cont, b, file)) = cont_of {
                    f.switch_to_block(cont);
                    f.set_cold_block(cont);
                    let p = f.block_params(cont)[0];
                    let x = b.short_word(f, &file, o, p, heads);
                    let ones = lanes_set(f, x, b.short_stride(&[o]));
                    let short = f.ins().icmp_imm(IntCC::UnsignedLessThan, ones, 16);
                    let l = f.ins().iadd(p, ones);
                    let l = f.ins().iadd_imm(l, -1);
                    let w = f.ins().ushr_imm(p, 6);
                    f.ins().brif(short, ended, &[l.into(), untag.into(), nohits.into()], more, &[w.into()]);
                }
            }
            EndRule::Read(i) => {
                // A reader (`records::read_last`): the first region whose opener holds at `s`,
                // then its closer's first bit after `s`, in that word, then the words after; a
                // lexer's reader of a string or a comment, from its start.
                let reader = &r.readers[usize::from(i)];
                let tag = match reader.retag {
                    Some((kind, marks)) => f.ins().iconst(types::I64, i64::from(kind) | i64::from(marks) << crate::records::MARK_SHIFT),
                    None => untag,
                };
                // The token's first byte is one the dispatch sends here (`first_bytes`): the
                // openers' tests of it are known, as the lean lexer's `b'/'` arm tests only the
                // byte after.
                let firsts = first_bytes(r, k);
                for (j, a) in reader.arms.iter().enumerate() {
                    let limit = rd.limit(a.closer, limit);
                    let (arm, not) = (f.create_block(), f.create_block());
                    let bit = rd.bit_first(f, c, s, a.opener, Some(firsts), heads);
                    f.ins().brif(bit, arm, &[], not, &[]);
                    f.switch_to_block(arm);
                    // No closer after the opener: what its region says.
                    let none = f.create_block();
                    f.set_cold_block(none);
                    // The end and the count tests in one pass (`read_fn`), as a lexer's string
                    // reader: its last and its hits on to the other tests (`tested`'s rest).
                    let read = rd.calls.iter().find(|x| x.0 == READ_CALL | (k as u16) << 3 | j as u16);
                    // A short reader's (`RuleMode::Short`): the read's first window inline, as
                    // `read_fn`'s, and a token whose closer is past it, or whose bytes the
                    // counts need, its whole function's call, returned. As a lexer reads a short
                    // string with no `\` straight through.
                    let whole = match mode {
                        RuleMode::Short(_) => rd.calls.iter().find(|x| x.0 == LONG_CALL | k as u16),
                        RuleMode::Calls if in_arm => rd.calls.iter().find(|x| x.0 == LONG_CALL | k as u16),
                        _ => None,
                    };
                    let counts: Vec<crate::records::TokenCheck> = tests.iter().map(|(t, _)| *t).filter(|t| t.count && !t.always()).collect();
                    if let (Some(_), Some(rest), Some((_, id, sig)), Some((b, file))) = (read, rest, whole, rd.bytes) {
                        let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
                        let signature = f.import_signature(sig.clone());
                        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
                        let long = f.create_block();
                        f.set_cold_block(long);
                        // The tests' words with the closer's where `read_fn` makes them in its
                        // loop (`tests_in_read`), as `string_end` reads a short string once.
                        let inloop = tests_in_read(b, a, &counts);
                        let touts = if inloop { read_test_outs(&counts) } else { Vec::new() };
                        let os: Vec<u16> = std::iter::once(a.closer).chain(touts.iter().map(|&(o, _)| o)).collect();
                        let runs = b.carried_runs(&os);
                        let zero = f.ins().iconst(types::I64, 0);
                        let p0 = f.ins().iadd_imm(s, 1);
                        let cins: Vec<(usize, Value)> = runs.iter().map(|&r| (r, if b.apart(a.opener, r) { zero } else { b.carry_at(f, &file, r, p0, heads) })).collect();
                        // The window gated on its rare bytes (`words_gated`): where none up to
                        // the closer, the escapes and the tests they make none known, the code
                        // after on each path. It reads the words up to the closer only.
                        b.words_gated(f, &file, &os, p0, &cins, true, heads, &mut |f, ws, _, _| {
                            let found = f.create_block();
                            f.ins().brif(ws[0], found, &[], long, &[]);
                            f.switch_to_block(found);
                            let tz = f.ins().ctz(ws[0]);
                            let close = f.ins().iadd(p0, tz);
                            let l = f.ins().iadd_imm(close, i64::from(a.off));
                            let (refuse0, hits0) = always_hits(counts.iter().chain(tests.iter().map(|(t, _)| t).filter(|t| t.always())));
                            let gate = if inloop { None } else { counts_need_by_offset(b.native, &counts) };
                            if inloop {
                                let x1 = f.ins().iadd_imm(ws[0], -1);
                                let upto = f.ins().bxor(ws[0], x1);
                                let fin: Vec<Value> = touts.iter().zip(&ws[1..]).map(|(&(_, n), &w)| {
                                    let w = f.ins().band(w, upto);
                                    read_test_acc(f, n, zero, w)
                                }).collect();
                                let got = read_test_hits(f, &counts, &touts, &fin);
                                let no = f.ins().sshr_imm(got, 63);
                                let r = if refuse0 == 0 { no } else { f.ins().bor_imm(no, refuse0) };
                                let marks = f.ins().band_imm(got, i64::MAX);
                                let h = if hits0 == 0 { marks } else { f.ins().bor_imm(marks, hits0) };
                                f.ins().jump(rest, &[l.into(), tag.into(), r.into(), h.into()]);
                            } else { match gate {
                                Some(ns) => {
                                    let (vec, quiet) = (f.create_block(), f.create_block());
                                    let n1 = f.ins().isub(l, s);
                                    let span = f.ins().iadd_imm(n1, 1);
                                    let fits = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, span, 64);
                                    f.ins().brif(fits, vec, &[], long, &[]);
                                    f.switch_to_block(vec);
                                    let some = needs_spans(f, &ns, c.bytes, s, span);
                                    f.ins().brif(some, long, &[], quiet, &[]);
                                    f.switch_to_block(quiet);
                                    let (r0, h0) = (f.ins().iconst(types::I64, refuse0), f.ins().iconst(types::I64, hits0));
                                    f.ins().jump(rest, &[l.into(), tag.into(), r0.into(), h0.into()]);
                                }
                                None => {
                                    f.ins().jump(long, &[]);
                                }
                            } }
                        });
                        // No closer in the file: the whole function's, as any other long read.
                        f.switch_to_block(none);
                        f.ins().jump(long, &[]);
                        f.switch_to_block(long);
                        let call = f.ins().call(callee, &rule_args(sig, [c.bytes, s, c.len, c.blocks, raw, entry]));
                        let got = f.inst_results(call).to_vec();
                        long_out(f, got);
                        f.switch_to_block(not);
                        continue;
                    }
                    if let (Some((_, id, sig)), Some(rest)) = (read, rest) {
                        let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
                        let signature = f.import_signature(sig.clone());
                        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
                        let call = f.ins().call(callee, &[c.bytes, c.len, s]);
                        let (close, got) = (f.inst_results(call)[0], f.inst_results(call)[1]);
                        let some = f.create_block();
                        let missing = f.ins().icmp_imm(IntCC::SignedLessThan, close, 0);
                        f.ins().brif(missing, none, &[], some, &[]);
                        f.switch_to_block(some);
                        let l = f.ins().iadd_imm(close, i64::from(a.off));
                        let refuse = f.ins().sshr_imm(got, 63);
                        let marks = f.ins().band_imm(got, i64::MAX);
                        f.ins().jump(rest, &[l.into(), tag.into(), refuse.into(), marks.into()]);
                        f.switch_to_block(none);
                        read_none(f, a.none, c, s, tag, nohits, ended);
                        f.switch_to_block(not);
                        continue;
                    }
                    let (first_word, more, look, found) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(more, types::I64);
                    f.append_block_param(found, types::I64);
                    let from = f.ins().iadd_imm(s, 1);
                    let q = f.ins().ushr_imm(from, 6);
                    let inside = f.ins().icmp(IntCC::UnsignedLessThan, q, bound(c, limit));
                    inside_or_past(f, inside, first_word, limit, none, &[]);
                    f.switch_to_block(first_word);
                    // A short reader's (`RuleMode::Short`): its closer's 64 bytes from the token's
                    // second inline, as a lexer's search from where it is, and a region past them
                    // its whole function's call, returned.
                    let whole = match (mode, rd.bytes.filter(|(b, _)| b.can(a.closer))) {
                        (RuleMode::Short(_), Some(bytes)) => rd.calls.iter().find(|x| x.0 == LONG_CALL | k as u16).map(|w| (w, bytes)),
                        (RuleMode::Calls, Some(bytes)) if in_arm => rd.calls.iter().find(|x| x.0 == LONG_CALL | k as u16).map(|w| (w, bytes)),
                        _ => None,
                    };
                    let (x, base) = match whole {
                        Some((_, (b, file))) => (b.word(f, &file, a.closer, from, heads), from),
                        None => {
                            let x = word_at(f, q, a.closer, heads);
                            let fb = f.ins().band_imm(from, 63);
                            let ones = f.ins().iconst(types::I64, -1);
                            let m = f.ins().ishl(ones, fb);
                            (f.ins().band(x, m), f.ins().ishl_imm(q, 6))
                        }
                    };
                    let whole = whole.map(|(w, _)| w);
                    let tz = f.ins().ctz(x);
                    let at = f.ins().iadd(base, tz);
                    let q1 = f.ins().iadd_imm(q, 1);
                    f.ins().brif(x, found, &[at.into()], more, &[q1.into()]);
                    // A word at a time past the first: a string or a comment over blocks.
                    f.switch_to_block(more);
                    f.set_cold_block(more);
                    if let Some((_, id, sig)) = whole {
                        let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
                        let signature = f.import_signature(sig.clone());
                        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
                        let call = f.ins().call(callee, &rule_args(sig, [c.bytes, s, c.len, c.blocks, raw, entry]));
                        let got = f.inst_results(call).to_vec();
                        long_out(f, got);
                        f.switch_to_block(found);
                        let at = f.block_params(found)[0];
                        let l = f.ins().iadd_imm(at, i64::from(a.off));
                        f.ins().jump(ended, &[l.into(), tag.into(), nohits.into()]);
                        f.switch_to_block(none);
                        read_none(f, a.none, c, s, tag, nohits, ended);
                        f.switch_to_block(not);
                        continue;
                    }
                    heads.push((more, Step::Word));
                    let w = f.block_params(more)[0];
                    let inside = f.ins().icmp(IntCC::UnsignedLessThan, w, bound(c, limit));
                    inside_or_past(f, inside, look, limit, none, &[]);
                    f.switch_to_block(look);
                    f.set_cold_block(look);
                    let x = word_at(f, w, a.closer, heads);
                    let w64 = f.ins().ishl_imm(w, 6);
                    let tz = f.ins().ctz(x);
                    let at = f.ins().iadd(w64, tz);
                    let w1 = f.ins().iadd_imm(w, 1);
                    f.ins().brif(x, found, &[at.into()], more, &[w1.into()]);
                    // The closer's first byte: the last at its offset.
                    f.switch_to_block(found);
                    let at = f.block_params(found)[0];
                    let l = f.ins().iadd_imm(at, i64::from(a.off));
                    f.ins().jump(ended, &[l.into(), tag.into(), nohits.into()]);
                    f.switch_to_block(none);
                    read_none(f, a.none, c, s, tag, nohits, ended);
                    f.switch_to_block(not);
                }
                // No opener: the byte's other rule, or the file refused.
                match blocks.get(usize::from(reader.other)) {
                    Some(&b) => {
                        f.ins().jump(b, &[]);
                    }
                    None => {
                        let refuse = f.ins().iconst(types::I64, -1);
                        f.ins().jump(done, &[s.into(), refuse.into(), untag.into(), nohits.into()]);
                    }
                }
            }
            EndRule::Last(o) => {
                let limit = rd.limit(o, limit);
                // The mask's first bit at or after `s`: in its word, then the words after.
                let (more, look) = (f.create_block(), f.create_block());
                f.append_block_param(more, types::I64);
                let x = word_at(f, q0, o, heads);
                let ones = f.ins().iconst(types::I64, -1);
                let from = f.ins().ishl(ones, sh);
                let x = f.ins().band(x, from);
                let base = f.ins().band_imm(s, !63);
                let tz = f.ins().ctz(x);
                let l = f.ins().iadd(base, tz);
                let w1 = f.ins().iadd_imm(q0, 1);
                let next = f.create_block();
                f.ins().brif(x, ended, &[l.into(), untag.into(), nohits.into()], next, &[]);
                f.switch_to_block(next);
                f.set_cold_block(next);
                f.ins().jump(more, &[w1.into()]);
                f.switch_to_block(more);
                f.set_cold_block(more);
                heads.push((more, Step::Word));
                let w = f.block_params(more)[0];
                let inside = f.ins().icmp(IntCC::UnsignedLessThan, w, bound(c, limit));
                // None found: with the loop's starts (`records::walk`), the first byte alone;
                // else `s - 1`, which the order check refuses.
                let none = if r.skip.is_some() { s } else { f.ins().iadd_imm(s, -1) };
                inside_or_past(f, inside, look, limit, ended, &[none.into(), untag.into(), nohits.into()]);
                f.switch_to_block(look);
                f.set_cold_block(look);
                let x = word_at(f, w, o, heads);
                let w64 = f.ins().ishl_imm(w, 6);
                let tz = f.ins().ctz(x);
                let l = f.ins().iadd(w64, tz);
                let w1 = f.ins().iadd_imm(w, 1);
                f.ins().brif(x, ended, &[l.into(), untag.into(), nohits.into()], more, &[w1.into()]);
            }
        }
    }
    for &(_, ended, done) in &exits {
        f.switch_to_block(ended);
        let (l, tag, no) = (f.block_params(ended)[0], f.block_params(ended)[1], f.block_params(ended)[2]);
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().jump(done, &[l.into(), no.into(), tag.into(), zero.into()]);
    }
    (exits.iter().map(|&(x, _, d)| (x, d)).collect(), fused_exit)
}

/// A tagging reader's tag and hits (`records::check_hits`) where an arm reads its token: its
/// `retag`, and its rule's tests' bits where each holds at every such token
/// (`TokenCheck::always`); none where a test is the bytes'.
fn tag_known(r: &crate::records::Records, k: usize) -> Option<(i64, u32)> {
    use crate::records::{CheckThen, EndRule};
    let &EndRule::Read(i) = r.rules.get(k)? else { return None };
    let (kind, marks) = r.readers.get(usize::from(i))?.retag?;
    let mut hits = 0u32;
    for c in r.checks.iter().filter(|c| usize::from(c.rule) == k) {
        if !c.always() {
            return None;
        }
        hits |= match c.then {
            CheckThen::Escape => 0,
            CheckThen::At(m) => 1 << m,
            CheckThen::After(m) => 1 << (crate::records::AFTER_SHIFT + u32::from(m)),
        };
    }
    Some((i64::from(kind) | i64::from(marks) << crate::records::MARK_SHIFT, hits))
}

/// Whether rule `k` is a reader that tags its token (`Reader::retag`).
fn tagged(r: &crate::records::Records, k: usize) -> bool {
    matches!(r.rules.get(k), Some(&crate::records::EndRule::Read(i)) if r.readers.get(usize::from(i)).is_some_and(|x| x.retag.is_some()))
}

/// The rule from a first byte's entry (`RecordTables::token`).
fn rule_in(f: &mut FunctionBuilder<'_>, entry: Value) -> Value {
    let rule = f.ins().ushr_imm(entry, TOKEN_RULE);
    let rule = f.ins().band_imm(rule, 0xff);
    f.ins().ireduce(types::I32, rule)
}

/// An exit of `rule_last`'s: its last, refusal, tag and hits, at its block.
fn exit_values(f: &mut FunctionBuilder<'_>, done: cranelift_codegen::ir::Block) -> (Value, Value, Value, Value) {
    f.switch_to_block(done);
    let p = f.block_params(done);
    (p[0], p[1], p[2], p[3])
}

/// The pair rule's word in the block's registers (`BlockRegs::runs`), past the outputs: bit
/// `i`, byte `i` and the next one a pair (`PairTable::holds`), as the lean lexer's `pair`.
const PAIR_WORD: u16 = 0x2000;

/// A pair table as nibble tables (decision 0177), the lean lexer's `FIRST_LO`, `FIRST_HI`,
/// `SECOND_LO`, `SECOND_HI`: each first byte of rule `rule` with a pair its own bit (eight at
/// most), the first byte's bits the lookups' `and` of its nibbles', the second byte's the bits of
/// the first bytes it ends a pair after. Byte `i` starts a pair where the two `and`ed are not
/// zero; none where the tables do not say exactly `holds` for each first byte of the rule.
pub fn pair_nibbles(r: &crate::records::Records, t: &crate::records::PairTable, rule: usize) -> Option<[[u8; 16]; 4]> {
    let firsts: Vec<u8> = (0..=255u8).filter(|&a| r.ends.get(usize::from(a)).is_some_and(|&e| usize::from(e) == rule) && (0..=255u8).any(|b| t.holds(a, b))).collect();
    if firsts.is_empty() || firsts.len() > 8 {
        return None;
    }
    let mut n = [[0u8; 16]; 4];
    for (k, &a) in firsts.iter().enumerate() {
        n[0][usize::from(a & 15)] |= 1 << k;
        n[1][usize::from(a >> 4)] |= 1 << k;
        for b in (0..=255u8).filter(|&b| t.holds(a, b)) {
            n[2][usize::from(b & 15)] |= 1 << k;
            n[3][usize::from(b >> 4)] |= 1 << k;
        }
    }
    let class = |lo: &[u8; 16], hi: &[u8; 16], x: u8| lo[usize::from(x & 15)] & hi[usize::from(x >> 4)];
    let exact = (0..=255u8).filter(|&a| r.ends.get(usize::from(a)).is_some_and(|&e| usize::from(e) == rule)).all(|a| (0..=255u8).all(|b| (class(&n[0], &n[1], a) & class(&n[2], &n[3], b) != 0) == t.holds(a, b)));
    // The bytes past the file are zeros: a pair ending in one would hold there.
    let padded = firsts.iter().all(|&a| !t.holds(a, 0));
    (exact && padded).then_some(n)
}

/// The two common rules `rule_last` if-converts (decision 0177): the pair rule (`Next`) and the
/// run rule whose word is in the block's registers (`runs`), each the rule most first bytes take
/// of its form, where their tests are all by first-byte bits; where no token of the two can be
/// an extra (its mark needs a first byte neither rule has).
fn fused_rules(r: &crate::records::Records, t: &RecordTables, rd: &Reads<'_>, runs: &[u16]) -> Option<(usize, usize)> {
    fused_of(r, &t.filters, rd.bytes.map(|(b, _)| b.native), runs)
}

/// `fused_rules` from the tests' filters and the program whose bytes make the loop's words
/// (none where they are not the bytes').
fn fused_of(r: &crate::records::Records, filters: &[Option<i64>], native: Option<&crate::block_native::BlockNative>, runs: &[u16]) -> Option<(usize, usize)> {
    use crate::records::EndRule;
    let first_bytes = |k: usize| (0..256).filter(|&b| r.ends.get(b).is_some_and(|&e| usize::from(e) == k)).count();
    let filtered = |k: usize| r.checks.iter().zip(filters).all(|(x, b)| usize::from(x.rule) != k || b.is_some());
    let kn = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], EndRule::Next(_)) && filtered(k) && first_bytes(k) > 0).max_by_key(|&k| first_bytes(k));
    let kr = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], EndRule::Run(o) if runs.contains(&o)) && filtered(k) && first_bytes(k) > 0).max_by_key(|&k| first_bytes(k));
    let no_extra = |ks: [usize; 2]| match r.extra {
        None => true,
        Some(e) => native.zip(r.marks.get(usize::from(e))).and_then(|(b, &(o, _))| crate::native_cpu_bytes::needs(b, o)).is_some_and(|n| {
            n.lo == 0 && n.hi == 0 && (0..=255u8).all(|x| !n.has(x) || ks.iter().all(|&k| r.ends.get(usize::from(x)).is_none_or(|&e| usize::from(e) != k)))
        }),
    };
    kn.zip(kr).filter(|&(a, b)| no_extra([a, b]))
}

/// Whether the loop's starts can be the lean lexer's (decision 0177): every byte not the skip's,
/// but a run's bytes after its first (`word & word << 1`) and a pair's second byte. Then a token
/// of the fused rules (`fused_rules`, the pair rule `kn`, the run rule `kr`) covers no start
/// after its own, and the loop clears its bit alone. It holds where the words are the bytes'
/// alone and: no first byte of `kn`, nor a second of its pairs, is a run byte (a pair ends no
/// run, starts none); a pair's second byte and a run byte are no skip's and no zero (the bytes
/// past the file), so a start put back after another rule's token (`record_tokens`) is one.
/// A first byte of `kr` that is no run byte (`#` of `#include`: the run from the byte after
/// it starts there) is then not fused (`run_fused`): its token takes the rules' general tail.
///
/// Some(whether a pair's second byte can start one, `=>=`), as the lean lexer's
/// `left_to_right`; none where the starts cannot be the lean lexer's.
pub fn lean_starts(r: &crate::records::Records, native: &crate::block_native::BlockNative, skip: u16, kn: usize, kr: usize) -> Option<bool> {
    use crate::records::EndRule;
    let (EndRule::Next(pi), EndRule::Run(o)) = (r.rules[kn], r.rules[kr]) else { return None };
    let pt = r.nexts.get(usize::from(pi))?;
    let alone = |o: u16| crate::native_cpu_bytes::needs(native, o).filter(|n| n.lo == 0 && n.hi == 0);
    let (word, space) = (alone(o)?, alone(skip)?);
    let of = |x: u8, k: usize| r.ends.get(usize::from(x)).is_some_and(|&e| usize::from(e) == k);
    let pairs_apart = (0..=255u8).filter(|&a| of(a, kn)).all(|a| !word.has(a) && (0..=255u8).filter(|&b| pt.holds(a, b)).all(|b| !word.has(b) && !space.has(b) && b != 0));
    let words_start = !word.has(0) && (0..=255u8).all(|x| !word.has(x) || !space.has(x));
    let starts_pair = |a: u8| of(a, kn) && (0..=255u8).any(|b| pt.holds(a, b));
    let chained = (0..=255u8).filter(|&a| starts_pair(a)).any(|a| (0..=255u8).any(|b| pt.holds(a, b) && starts_pair(b)));
    (pairs_apart && words_start).then_some(chained)
}

/// Whether first byte `b` of the fused run rule (word `o`) takes the fused rules' path where
/// the starts are the lean lexer's (`lean_starts`): it is a run byte, its token the run from it.
fn run_fused(native: &crate::block_native::BlockNative, o: u16, b: u8) -> bool {
    crate::native_cpu_bytes::needs(native, o).is_some_and(|n| n.has(b))
}

/// Whether output `o` is never set at a first byte of rules `ks`: it needs the byte alone
/// (`native_cpu_bytes::needs`) and none of the rules' first bytes has it.
fn never_at(r: &crate::records::Records, native: Option<&crate::block_native::BlockNative>, o: u16, ks: [usize; 2]) -> bool {
    native.and_then(|b| crate::native_cpu_bytes::needs(b, o)).is_some_and(|n| {
        n.lo == 0 && n.hi == 0 && (0..=255u8).all(|x| !n.has(x) || ks.iter().all(|&k| r.ends.get(usize::from(x)).is_none_or(|&e| usize::from(e) != k)))
    })
}

/// The pair rule's word for the block at byte `p` (`PAIR_WORD`), as the lean lexer's window
/// makes its `pair`: each 16 bytes and the 16 from the byte after, their nibbles' lookups
/// `and`ed, a bit where not zero. The 16 from the byte after are an `ext` of two loaded
/// (`vextq_u8::<1>(x, v[k + 1])`), one load past the block, not a load each at `+1`. The
/// file's zero bytes after it end no pair.
fn pair_word(f: &mut FunctionBuilder<'_>, file: &crate::native_cpu_bytes::File, n: &[[u8; 16]; 4], p: Value) -> Value {
    use cranelift_codegen::ir::ConstantData;
    let at = f.ins().iadd(file.bytes, p);
    let tables: Vec<Value> = n.iter().map(|t| {
        let k = f.func.dfg.constants.insert(ConstantData::from(&t[..]));
        f.ins().vconst(types::I8X16, k)
    }).collect();
    let zero = f.ins().iconst(types::I8, 0);
    let zero = f.ins().splat(types::I8X16, zero);
    let class = |f: &mut FunctionBuilder<'_>, x: Value, lo: Value, hi: Value| {
        let xl = f.ins().band(x, file.nibble);
        let xh = f.ins().ushr_imm(x, 4);
        let l = f.ins().swizzle(lo, xl);
        let h = f.ins().swizzle(hi, xh);
        f.ins().band(l, h)
    };
    let v: Vec<Value> = (0..5).map(|k| f.ins().load(types::I8X16, MemFlags::trusted(), at, 16 * k)).collect();
    let next: Vec<u8> = (1..17).collect();
    let next = f.func.dfg.immediates.push(ConstantData::from(&next[..]));
    let parts: Vec<Value> = (0..4).map(|k| {
        let x = v[k];
        let d = f.ins().shuffle(v[k], v[k + 1], next);
        let a = class(f, x, tables[0], tables[1]);
        let b = class(f, d, tables[2], tables[3]);
        let both = f.ins().band(a, b);
        f.ins().icmp(IntCC::Equal, both, zero)
    }).collect();
    let none = bitmask(f, &parts, file.weights);
    f.ins().bnot(none)
}

/// How a rule's tokens end, for the starts after them (`lean_starts` leaves out a run byte
/// after a run byte, and a pair's second): as the lean lexer's arms, each what it read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenEnd {
    /// No byte after it left out (`plain_end`): the starts past it.
    Plain,
    /// Its last byte no pair's first (`word_end`): the byte after it put back where it is a
    /// run byte, as the lean lexer's number arm reads the word after it.
    Word,
    /// Either put back, and a run of pairs across its end read again.
    Any,
}

/// The bytes a token of rule `k` may end with (its first bytes `starts`), by its ends'
/// outputs (`native_cpu_bytes::output_byte_at`); `None` where they do not say. A reader's
/// closers at their `off`, its opener's `M` bytes where none closes (none for a token to the
/// file's end: no byte after it), and the rule it goes on to; a last's output; a run's first
/// byte and the bytes it can end with (`run_last_bytes`); a pair's first byte and its seconds.
fn last_bytes(r: &crate::records::Records, made: &crate::block_native::BlockNative, k: usize, starts: &[u8], depth: u32) -> Option<[u64; 4]> {
    use crate::records::{EndRule, ReadNone};
    let or = |a: [u64; 4], b: [u64; 4]| [a[0] | b[0], a[1] | b[1], a[2] | b[2], a[3] | b[3]];
    let mut first = [0u64; 4];
    for &b in starts {
        first[usize::from(b >> 6)] |= 1 << (b & 63);
    }
    match *r.rules.get(k)? {
        EndRule::Read(i) => {
            let rd = r.readers.get(usize::from(i))?;
            let mut set = [0u64; 4];
            for a in &rd.arms {
                set = or(set, crate::native_cpu_bytes::output_byte_at(made, a.closer, i32::from(a.off))?);
                if let ReadNone::Bytes(m) = a.none {
                    set = or(set, crate::native_cpu_bytes::output_byte_at(made, a.opener, i32::from(m.checked_sub(1)?))?);
                }
            }
            let other = usize::from(rd.other);
            if other < r.rules.len() && other != k {
                set = or(set, if depth < 4 { last_bytes(r, made, other, starts, depth + 1)? } else { return None });
            }
            Some(set)
        }
        EndRule::Last(out) => crate::native_cpu_bytes::output_byte_at(made, out, 0),
        EndRule::Run(o) => Some(or(first, crate::native_cpu_bytes::run_last_bytes(made, o)?)),
        EndRule::Next(p) => {
            let t = r.nexts.get(usize::from(p))?;
            let mut set = first;
            for &x in starts {
                for y in 0..=255u8 {
                    if t.holds(x, y) {
                        set[usize::from(y >> 6)] |= 1 << (y & 63);
                    }
                }
            }
            Some(set)
        }
        _ => None,
    }
}

/// Whether no token of rule `k` ends with a pair's first byte (the pair rule `kn`, the run rule
/// `kr`, as `plain_end`): the only start left out after it is a run byte after a run byte, so
/// the byte after it is put back where it is a run byte (`TokenEnd::Word`). After a last byte
/// that is no run byte, a run byte is a start already: its bit again changes nothing.
fn word_end(r: &crate::records::Records, made: &crate::block_native::BlockNative, k: usize, starts: &[u8], kn: usize, kr: usize) -> bool {
    use crate::records::EndRule;
    let (EndRule::Next(pi), EndRule::Run(_)) = (r.rules[kn], r.rules[kr]) else { return false };
    let Some(pt) = r.nexts.get(usize::from(pi)) else { return false };
    let of_kn = |x: u8| r.ends.get(usize::from(x)).is_some_and(|&e| usize::from(e) == kn);
    let Some(last) = last_bytes(r, made, k, starts, 0) else { return false };
    (0..=255u8).all(|b| last[usize::from(b >> 6)] >> (b & 63) & 1 == 0 || !(of_kn(b) && (0..=255u8).any(|y| pt.holds(b, y))))
}

/// Whether a token of rule `k` ends where the loop's starts (`lean_starts`, the pair rule `kn`,
/// the run rule `kr`) leave out no byte after it: its last byte no run byte and no pair's
/// first, or the byte after it no run byte and no pair's second. Then no start is put back
/// after it. By the bytes its ends' outputs test (`native_cpu_bytes::output_byte_at`): a reader's
/// closer at its `off` (and the one after), its opener's `M` bytes where none closes; a
/// reader it goes on to (`Reader::other`) the same. False where they do not say.
fn plain_end(r: &crate::records::Records, made: &crate::block_native::BlockNative, k: usize, starts: &[u8], kn: usize, kr: usize, depth: u32) -> bool {
    use crate::records::{EndRule, ReadNone};
    let (EndRule::Next(pi), EndRule::Run(o)) = (r.rules[kn], r.rules[kr]) else { return false };
    let Some(word) = crate::native_cpu_bytes::needs(made, o).filter(|n| n.lo == 0 && n.hi == 0) else { return false };
    let Some(pt) = r.nexts.get(usize::from(pi)) else { return false };
    let of_kn = |x: u8| r.ends.get(usize::from(x)).is_some_and(|&e| usize::from(e) == kn);
    let firsts: Vec<bool> = (0..=255u8).map(|x| of_kn(x) && (0..=255u8).any(|y| pt.holds(x, y))).collect();
    let seconds: Vec<bool> = (0..=255u8).map(|y| (0..=255u8).any(|x| of_kn(x) && pt.holds(x, y))).collect();
    let none_of = |set: Option<[u64; 4]>, risky: &dyn Fn(u8) -> bool| set.is_some_and(|s| (0..=255u8).all(|b| s[usize::from(b >> 6)] >> (b & 63) & 1 == 0 || !risky(b)));
    let last_risky = |b: u8| word.has(b) || firsts[usize::from(b)];
    let after_risky = |b: u8| word.has(b) || seconds[usize::from(b)];
    let safe = |out: u16, at: i32| none_of(crate::native_cpu_bytes::output_byte_at(made, out, at), &last_risky) || none_of(crate::native_cpu_bytes::output_byte_at(made, out, at + 1), &after_risky);
    match r.rules.get(k) {
        Some(&EndRule::Read(i)) => {
            let Some(rd) = r.readers.get(usize::from(i)) else { return false };
            let arms = rd.arms.iter().all(|a| {
                let closed = safe(a.closer, i32::from(a.off));
                let open = match a.none {
                    ReadNone::Refuse | ReadNone::End => true,
                    ReadNone::Bytes(m) => m > 0 && safe(a.opener, i32::from(m) - 1),
                };
                closed && open
            });
            let other = usize::from(rd.other);
            arms && (other >= r.rules.len() || other == k || (depth < 4 && plain_end(r, made, other, starts, kn, kr, depth + 1)))
        }
        Some(&EndRule::Last(out)) => safe(out, 0),
        // A pair rule's token ends at its first byte, where no second follows it (any byte
        // the pair row has not), or at the second, whose next byte is any.
        Some(&EndRule::Next(p)) => r.nexts.get(usize::from(p)).is_some_and(|t| starts.iter().all(|&x| {
            let alone = !last_risky(x) || (0..=255u8).all(|y| t.holds(x, y) || !after_risky(y));
            alone && (0..=255u8).all(|y| !t.holds(x, y) || !last_risky(y))
        })),
        _ => false,
    }
}

/// A set of bytes as nibble tables: byte `x` is in it where `lo[x & 15] & hi[x >> 4]` is not
/// zero. Each distinct set of low nibbles a high nibble has is a bit (eight at most); none
/// past eight.
pub fn set_nibbles(set: &[bool; 256]) -> Option<[[u8; 16]; 2]> {
    let mut groups: Vec<u16> = Vec::new();
    let mut n = [[0u8; 16]; 2];
    for h in 0..16 {
        let lows = (0..16).filter(|&l| set[h << 4 | l]).fold(0u16, |a, l| a | 1 << l);
        if lows == 0 {
            continue;
        }
        let g = match groups.iter().position(|&x| x == lows) {
            Some(g) => g,
            None => {
                groups.push(lows);
                groups.len() - 1
            }
        };
        if g >= 8 {
            return None;
        }
        n[1][h] = 1 << g;
    }
    for (g, &lows) in groups.iter().enumerate() {
        for (l, nl) in n[0].iter_mut().enumerate().take(16) {
            if lows >> l & 1 == 1 {
                *nl |= 1 << g;
            }
        }
    }
    Some(n)
}

/// The block at byte `p`'s word of a byte set (`set_nibbles`): each 16 bytes' nibbles looked
/// up and `and`ed, a bit where not zero, as the lean lexer's window makes its `rare`.
fn set_word(f: &mut FunctionBuilder<'_>, file: &crate::native_cpu_bytes::File, n: &[[u8; 16]; 2], p: Value) -> Value {
    use cranelift_codegen::ir::ConstantData;
    let at = f.ins().iadd(file.bytes, p);
    let tables: Vec<Value> = n.iter().map(|t| {
        let k = f.func.dfg.constants.insert(ConstantData::from(&t[..]));
        f.ins().vconst(types::I8X16, k)
    }).collect();
    let zero = f.ins().iconst(types::I8, 0);
    let zero = f.ins().splat(types::I8X16, zero);
    let parts: Vec<Value> = (0..4).map(|k| {
        let x = f.ins().load(types::I8X16, MemFlags::trusted(), at, 16 * k);
        let xl = f.ins().band(x, file.nibble);
        let xh = f.ins().ushr_imm(x, 4);
        let l = f.ins().swizzle(tables[0], xl);
        let h = f.ins().swizzle(tables[1], xh);
        let both = f.ins().band(l, h);
        f.ins().icmp(IntCC::Equal, both, zero)
    }).collect();
    let none = bitmask(f, &parts, file.weights);
    f.ins().bnot(none)
}

/// A pair's last (`EndRule::Next`, table `i`): the first byte's row (its entry's), then the
/// next byte's bit in the row's set, if inside the file (`PairTable`): the next byte from the
/// token's bytes, one load of the set.
#[allow(clippy::too_many_arguments)]
fn next_last(f: &mut FunctionBuilder<'_>, t: &RecordTables, c: &FileCols, s: Value, raw: Value, entry: Value, i: u16) -> Value {
    let ro = MemFlags::trusted().with_readonly();
    let (_, sets) = t.pairs[usize::from(i)];
    let i1 = f.ins().iadd_imm(s, 1);
    let inside = f.ins().icmp(IntCC::UnsignedLessThan, i1, c.len);
    let b = f.ins().ushr_imm(raw, 8);
    let b = f.ins().band_imm(b, 0xff);
    let row = f.ins().ushr_imm(entry, TOKEN_PAIR_ROW);
    let row = f.ins().band_imm(row, 0xff);
    let w = f.ins().ishl_imm(row, 2);
    let hi = f.ins().ushr_imm(b, 6);
    let w = f.ins().iadd(w, hi);
    let w = f.ins().ishl_imm(w, 3);
    let wa = f.ins().iadd(c.tables, w);
    let set = f.ins().load(types::I64, ro, wa, sets as i32);
    let bit = f.ins().ushr(set, b);
    let bit = f.ins().band_imm(bit, 1);
    let inside = f.ins().uextend(types::I64, inside);
    let bit = f.ins().band(bit, inside);
    f.ins().iadd(s, bit)
}

/// A reader's arm with no closer after its opener (`ReadNone`): its last on to `ended`, its
/// tests', and where refused, the refusal with it.
fn read_none(f: &mut FunctionBuilder<'_>, none: crate::records::ReadNone, c: &FileCols, s: Value, tag: Value, nohits: Value, ended: cranelift_codegen::ir::Block) {
    use crate::records::ReadNone;
    match none {
        // The opener held: the token is the reader's, its first byte alone, tagged and tested,
        // as `records::pass` tags it by its opener's bit (`Records::retags`) and tests it
        // (`records::walk`); and the file refused.
        ReadNone::Refuse => {
            let refuse = f.ins().iconst(types::I64, -1);
            f.ins().jump(ended, &[s.into(), tag.into(), refuse.into()]);
        }
        ReadNone::End => {
            let l = f.ins().iadd_imm(c.len, -1);
            f.ins().jump(ended, &[l.into(), tag.into(), nohits.into()]);
        }
        ReadNone::Bytes(m) => {
            let l = f.ins().iadd_imm(s, i64::from(m) - 1);
            f.ins().jump(ended, &[l.into(), tag.into(), nohits.into()]);
        }
    }
}

/// What a rule's count tests need of the bytes together (`native_cpu_bytes::needs`), where one
/// vector's compares can say a short token has none of them (`needs_some`).
fn counts_need(native: &crate::block_native::BlockNative, tests: &[crate::records::TokenCheck]) -> Option<crate::native_cpu_bytes::Needs> {
    let n = tests.iter().map(|t| crate::native_cpu_bytes::needs(native, t.o)).try_fold(None, |a: Option<crate::native_cpu_bytes::Needs>, n| n.map(|n| Some(a.map_or(n, |a| a.union(n)))))??;
    (n.lo >= -8 && n.hi <= 8 && (1..=4).contains(&n.ranges().len())).then_some(n)
}

/// What a rule's count tests need of the bytes, offset by offset (`native_cpu_bytes::
/// needs_by_offset`), each offset's set a `Needs` at that offset alone: a token has none of
/// them where, at no offset `d`, a byte `s + d ..= l + d` is in its set. Two offsets at most,
/// each within 8 bytes and four runs of bytes, so the check is a few vectors.
fn counts_need_by_offset(native: &crate::block_native::BlockNative, tests: &[crate::records::TokenCheck]) -> Option<Vec<crate::native_cpu_bytes::Needs>> {
    let mut all: std::collections::BTreeMap<i32, [u64; 4]> = std::collections::BTreeMap::new();
    for t in tests {
        for (d, set) in crate::native_cpu_bytes::needs_by_offset(native, t.o)? {
            let e = all.entry(d).or_insert([0; 4]);
            for (w, v) in e.iter_mut().zip(set) {
                *w |= v;
            }
        }
    }
    let v: Vec<crate::native_cpu_bytes::Needs> = all.into_iter().map(|(d, set)| crate::native_cpu_bytes::Needs { set, lo: d, hi: d }).collect();
    (!v.is_empty() && v.len() <= 2 && v.iter().all(|n| (-8..=8).contains(&n.lo) && (1..=4).contains(&n.ranges().len()))).then_some(v)
}

/// Whether the bytes `s + n.lo` on, `span` of them (16 at most), have one of `n`'s
/// (`native_cpu_bytes::Needs`): one vector's compares, nonzero where some byte is in the set.
fn needs_some(f: &mut FunctionBuilder<'_>, n: &crate::native_cpu_bytes::Needs, bytes: Value, s: Value, span: Value) -> Value {
    let any = needs_lanes(f, n, bytes, s, span);
    let wide = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little), any);
    let w0 = f.ins().extractlane(wide, 0);
    let w1 = f.ins().extractlane(wide, 1);
    f.ins().bor(w0, w1)
}

/// How many of the bytes `s + n.lo` on, `span` of them (16 at most), are in `n`'s set
/// (`native_cpu_bytes::exact`): the lanes' ones added, two words' bytes summed by a multiply.
fn needs_count(f: &mut FunctionBuilder<'_>, n: &crate::native_cpu_bytes::Needs, bytes: Value, s: Value, span: Value) -> Value {
    let any = needs_lanes(f, n, bytes, s, span);
    let one = f.ins().iconst(types::I8, 1);
    let one = f.ins().splat(types::I8X16, one);
    let ones = f.ins().band(any, one);
    let wide = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little), ones);
    let w0 = f.ins().extractlane(wide, 0);
    let w1 = f.ins().extractlane(wide, 1);
    let w = f.ins().iadd(w0, w1);
    let w = f.ins().imul_imm(w, 0x0101_0101_0101_0101);
    f.ins().ushr_imm(w, 56)
}

/// The lanes of the bytes `s + n.lo` on, `span` of them (16 at most), in `n`'s set: all ones
/// each, the others and those past the span zero.
fn needs_lanes(f: &mut FunctionBuilder<'_>, n: &crate::native_cpu_bytes::Needs, bytes: Value, s: Value, span: Value) -> Value {
    let at = f.ins().iadd(bytes, s);
    let v = f.ins().load(types::I8X16, MemFlags::trusted(), at, n.lo);
    let mut any: Option<Value> = None;
    for (a, b) in n.ranges() {
        let m = if a == b {
            let k = f.ins().iconst(types::I8, i64::from(a));
            let k = f.ins().splat(types::I8X16, k);
            f.ins().icmp(IntCC::Equal, v, k)
        } else {
            let lo = f.ins().iconst(types::I8, i64::from(a));
            let lo = f.ins().splat(types::I8X16, lo);
            let w = f.ins().iconst(types::I8, i64::from(b - a) + 1);
            let w = f.ins().splat(types::I8X16, w);
            let d = if a == 0 { v } else { f.ins().isub(v, lo) };
            if u16::from(b - a) + 1 == 256 { f.ins().icmp(IntCC::Equal, v, v) } else { f.ins().icmp(IntCC::UnsignedLessThan, d, w) }
        };
        any = Some(match any {
            Some(x) => f.ins().bor(x, m),
            None => m,
        });
    }
    let any = any.expect("a range");
    // The lanes of the span alone, by their index: one compare, not each half's shifts.
    let span8 = f.ins().ireduce(types::I8, span);
    let span16 = f.ins().splat(types::I8X16, span8);
    let index: Vec<u8> = (0..16).collect();
    let index = f.func.dfg.constants.insert(index.into());
    let index = f.ins().vconst(types::I8X16, index);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, span16);
    f.ins().band_not(any, past)
}

/// Whether the bytes `s + n.lo` on, `span` of them (64 at most), have one of `n`'s
/// (`native_cpu_bytes::Needs`), for each of `ns`: nonzero where some byte is. Only the 16-byte
/// vectors the span reaches, one, two or four, each a predictable branch on the span, as a
/// lexer reads a short string's bytes and no more; each vector's lanes past the span cut by
/// their index.
fn needs_spans(f: &mut FunctionBuilder<'_>, ns: &[crate::native_cpu_bytes::Needs], bytes: Value, s: Value, span: Value) -> Value {
    let at = f.ins().iadd(bytes, s);
    let span8 = f.ins().ireduce(types::I8, span);
    let span16 = f.ins().splat(types::I8X16, span8);
    // Vector `c` of every need, the lanes in their sets and inside the span.
    let part = |f: &mut FunctionBuilder<'_>, c: i32| -> Value {
        let index: Vec<u8> = (0..16).map(|i| (16 * c + i) as u8).collect();
        let index = f.func.dfg.constants.insert(index.into());
        let index = f.ins().vconst(types::I8X16, index);
        let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, span16);
        let mut all: Option<Value> = None;
        for n in ns {
            let v = f.ins().load(types::I8X16, MemFlags::trusted(), at, n.lo + 16 * c);
            for (a, b) in n.ranges() {
                let m = if a == b {
                    let k = f.ins().iconst(types::I8, i64::from(a));
                    let k = f.ins().splat(types::I8X16, k);
                    f.ins().icmp(IntCC::Equal, v, k)
                } else {
                    let lo = f.ins().iconst(types::I8, i64::from(a));
                    let lo = f.ins().splat(types::I8X16, lo);
                    let w = f.ins().iconst(types::I8, i64::from(b - a) + 1);
                    let w = f.ins().splat(types::I8X16, w);
                    let d = if a == 0 { v } else { f.ins().isub(v, lo) };
                    if u16::from(b - a) + 1 == 256 { f.ins().icmp(IntCC::Equal, v, v) } else { f.ins().icmp(IntCC::UnsignedLessThan, d, w) }
                };
                all = Some(match all {
                    Some(x) => f.ins().bor(x, m),
                    None => m,
                });
            }
        }
        let all = all.expect("a range");
        f.ins().band_not(all, past)
    };
    let (two, four, join) = (f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(join, types::I8X16);
    let a0 = part(f, 0);
    let more = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, span, 16);
    f.ins().brif(more, two, &[], join, &[a0.into()]);
    f.switch_to_block(two);
    let p1 = part(f, 1);
    let a1 = f.ins().bor(a0, p1);
    let more = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, span, 32);
    f.ins().brif(more, four, &[], join, &[a1.into()]);
    f.switch_to_block(four);
    let p2 = part(f, 2);
    let p3 = part(f, 3);
    let a2 = f.ins().bor(a1, p2);
    let a3 = f.ins().bor(a2, p3);
    f.ins().jump(join, &[a3.into()]);
    f.switch_to_block(join);
    let all = f.block_params(join)[0];
    let wide = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little), all);
    let w0 = f.ins().extractlane(wide, 0);
    let w1 = f.ins().extractlane(wide, 1);
    f.ins().bor(w0, w1)
}

/// `needs_spans` over a token of any length: 64 bytes at a time while more than 64 are left,
/// each chunk's four vectors, and the rest as `needs_spans`'s; nonzero at the first chunk
/// with a byte of the sets. A lexer's string read past its first window that saw no `\`
/// and no control byte: the tests' function is never called for it (`read_fn`).
fn needs_chunks(f: &mut FunctionBuilder<'_>, ns: &[crate::native_cpu_bytes::Needs], bytes: Value, s: Value, span: Value, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
    let (head, full, last, join) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(head, types::I64);
    f.append_block_param(head, types::I64);
    f.append_block_param(join, types::I64);
    f.ins().jump(head, &[s.into(), span.into()]);
    f.switch_to_block(head);
    heads.push((head, Step::Word));
    let (p, rest) = (f.block_params(head)[0], f.block_params(head)[1]);
    let big = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, rest, 64);
    f.ins().brif(big, full, &[], last, &[]);
    f.switch_to_block(full);
    let all = f.ins().iconst(types::I64, 64);
    let a = needs_spans(f, ns, bytes, p, all);
    let p1 = f.ins().iadd_imm(p, 64);
    let r1 = f.ins().iadd_imm(rest, -64);
    f.ins().brif(a, join, &[a.into()], head, &[p1.into(), r1.into()]);
    f.switch_to_block(last);
    let a = needs_spans(f, ns, bytes, p, rest);
    f.ins().jump(join, &[a.into()]);
    f.switch_to_block(join);
    f.block_params(join)[0]
}

/// The bytes a token of rule `k` can start with: those whose end rule is `k` (`Records::ends`,
/// the dispatch's, by the first byte alone), and those of a reader whose `other` it is, which
/// goes on to it with the same first byte.
fn first_bytes(r: &crate::records::Records, k: usize) -> [u64; 4] {
    let mut set = [0u64; 4];
    for b in 0..256usize {
        let mut at = r.ends.get(b).map(|&e| usize::from(e));
        let mut seen = 0;
        while let Some(j) = at.filter(|&j| j < r.rules.len()) {
            if j == k {
                set[b / 64] |= 1 << (b % 64);
                break;
            }
            seen += 1;
            at = match r.rules[j] {
                crate::records::EndRule::Read(i) if seen <= r.rules.len() => Some(usize::from(r.readers[usize::from(i)].other)),
                _ => None,
            };
        }
    }
    set
}

/// A run rule `k`'s count tests whose bits its window from the token's second byte gives
/// (`tested`'s `pre`): each output a byte set at its own byte (`native_cpu_bytes::exact`, no
/// delay), read from the bytes, that none of the rule's first bytes is in, so the token's first
/// byte has no bit.
fn run_tests(r: &crate::records::Records, b: &crate::native_cpu_bytes::Bytewise<'_>, k: usize, tests: &[(crate::records::TokenCheck, Option<i64>)]) -> Vec<u16> {
    let firsts: Vec<u8> = (0..256).filter(|&x| r.ends.get(x).is_some_and(|&e| usize::from(e) == k)).map(|x| x as u8).collect();
    let mut v: Vec<u16> = tests.iter().filter(|(t, _)| t.count && !t.always() && b.can(t.o)).filter(|(t, _)| {
        crate::native_cpu_bytes::exact(b.native, t.o).is_some_and(|n| n.lo == 0 && n.hi == 0 && !firsts.iter().any(|&x| n.has(x)))
    }).map(|(t, _)| t.o).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// How a split run's byte term runs (`Bytewise::run_split`) on the CPU, by `RERAIL_RUN_FORM`:
/// `bytes`, a byte at a time, as lean's `while digit { end += 1 }` (a `Step::Bytes` loop, the
/// default: 0177, "A number's run is lean's loop"); `window`, a 16-byte vector from the
/// token's second byte; `block`, the term's word made once a block with the loop's
/// (`BlockRegs::terms`), the form for a target whose lanes must not diverge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunForm {
    Window,
    Block,
    Bytes,
}

pub fn run_form() -> RunForm {
    static FORM: std::sync::OnceLock<RunForm> = std::sync::OnceLock::new();
    *FORM.get_or_init(|| match std::env::var("RERAIL_RUN_FORM").as_deref() {
        Ok("block") => RunForm::Block,
        Ok("window") => RunForm::Window,
        _ => RunForm::Bytes,
    })
}

/// A split run's term as the arm runs it (`run_form`): a mask of its lanes from the token's
/// second byte, `stride` bits a lane, `cap` lanes of it known; or its bytes one at a time.
#[derive(Clone, Copy)]
enum TermRun {
    Mask { x: Value, stride: u32, cap: Value },
    Bytes,
}

/// A split run's count tests (`Bytewise::run_split`, its byte term's `set`) on each of its
/// two inline ways: the term's run alone, and the term's, the rest's one byte, the term's
/// again. Where each count's bytes (`native_cpu_bytes::exact`, at the token's bytes) are none
/// of the term's and the token's first's, and the rest's byte is all or none of them, its
/// count is 0 on the first way and 1 or 0 on the second: `(refused, hits)` each, as lean's
/// number arm knows a float by its branch. None where a count is not so known.
fn split_counts(r: &crate::records::Records, b: &crate::native_cpu_bytes::Bytewise<'_>, k: usize, tests: &[(crate::records::TokenCheck, Option<i64>)], o: u16, split: (usize, i64, [u64; 4])) -> Option<[(bool, i64); 2]> {
    let (a, _, set) = split;
    let firsts = first_bytes(r, k);
    let rest = b.rest_bytes(o, a, 0);
    let meets = |x: &[u64; 4], y: &[u64; 4]| (0..4).any(|w| x[w] & y[w] != 0);
    let within = |x: &[u64; 4], y: &[u64; 4]| (0..4).all(|w| x[w] & !y[w] == 0);
    let mut ways = [(false, 0i64); 2];
    for (t, _) in tests.iter().filter(|(t, _)| t.count && !t.always()) {
        let n = crate::native_cpu_bytes::exact(b.native, t.o).filter(|n| n.lo == 0 && n.hi == 0)?;
        if meets(&n.set, &set) || meets(&n.set, &firsts) {
            return None;
        }
        let second = if within(&rest, &n.set) {
            1
        } else if !meets(&rest, &n.set) {
            0
        } else {
            return None;
        };
        for (way, count) in [(0usize, 0u32), (1, second)] {
            if count >= t.least {
                match t.then {
                    crate::records::CheckThen::Escape => ways[way].0 = true,
                    crate::records::CheckThen::At(m) => ways[way].1 |= 1i64 << m,
                    crate::records::CheckThen::After(m) => ways[way].1 |= 1i64 << (crate::records::AFTER_SHIFT + u32::from(m)),
                }
            }
        }
    }
    Some(ways)
}

/// The refusal and the hits of the tests that hold at every token (`TokenCheck::always`).
fn always_hits<'t>(tests: impl Iterator<Item = &'t crate::records::TokenCheck>) -> (i64, i64) {
    tests.filter(|t| t.always()).fold((0i64, 0i64), |(r, h), t| match t.then {
        crate::records::CheckThen::Escape => (-1, h),
        crate::records::CheckThen::At(k) => (r, h | 1i64 << k),
        crate::records::CheckThen::After(k) => (r, h | 1i64 << (crate::records::AFTER_SHIFT + u32::from(k))),
    })
}

/// A rule's end block that tests its token `s..=l` (`l` its parameter) by its token tests
/// (`records::check_holds`), then on to `done` with the last and the refusal (a sign bit). A
/// count is the mask's bits over the token's words, a word at a time; a run, the mask's first
/// `least` bits from `s` compared at once, from one word or two.
///
/// `pre`: outputs' bits from `s + 1` on, 16 of them, from the run's own window (`run_tests`):
/// a count of one of them over a token of 16 bytes at most is its bits below the token's
/// length, as the lean lexer asks of a number's `.` in its digits' pass. The output is never
/// set at the token's first byte.
#[allow(clippy::too_many_arguments)]
fn tested(f: &mut FunctionBuilder<'_>, tests: &[(crate::records::TokenCheck, Option<i64>)], rd: &Reads<'_>, c: &FileCols, s: Value, entry: Value, done: cranelift_codegen::ir::Block, most: Option<u32>, pre: &[(u16, Value)], pre_stride: u32, lanes: Option<(crate::native_cpu_bytes::RunLanes, [u64; 4])>, way_lane: Option<([u64; 4], [u64; 4])>, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> (cranelift_codegen::ir::Block, Option<cranelift_codegen::ir::Block>) {
    let here = f.current_block().expect("a rule's block");
    let mut lanes = lanes;
    let ended = f.create_block();
    f.append_block_param(ended, types::I64);
    f.append_block_param(ended, types::I64);
    f.append_block_param(ended, types::I64);
    f.switch_to_block(ended);
    let (mut l, mut tag, no) = (f.block_params(ended)[0], f.block_params(ended)[1], f.block_params(ended)[2]);
    let word_at = |f: &mut FunctionBuilder<'_>, q: Value, o: u16, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>| rd.word(f, c, q, o, heads);
    let mut q0 = f.ins().ushr_imm(s, 6);
    let mut sh = f.ins().band_imm(s, 63);
    let mut ql = f.ins().ushr_imm(l, 6);
    let len = f.ins().isub(l, s);
    let mut len = f.ins().iadd_imm(len, 1);
    let mut ones = f.ins().iconst(types::I64, -1);
    let mut rest = None;
    // The tests that hold at every token (`TokenCheck::always`): their hits, made nowhere.
    let always = always_hits(tests.iter().map(|(t, _)| t));
    // A refusal in (`read_none`), and the always tests'.
    let mut refuse = if always.0 == 0 { no } else { f.ins().iconst(types::I64, always.0) };
    let mut hits = f.ins().iconst(types::I64, always.1);
    // The count tests, fused into one call (`tests_fn`) where the loop has it.
    let rule = tests.first().map_or(0, |(t, _)| t.rule);
    // A test of more bytes than the token has (`most`, its longest) never holds: none made.
    let never = |t: &crate::records::TokenCheck| most.is_some_and(|m| t.least > m);
    let tests: Vec<(crate::records::TokenCheck, Option<i64>)> = tests.iter().filter(|(t, _)| !never(t)).copied().collect();
    let tests = &tests[..];
    let fused = rd.calls.iter().find(|x| x.0 == TESTS_CALL | u16::from(rule)).cloned().filter(|_| tests.iter().any(|(t, _)| t.count && !t.always()));
    if let Some((_, id, sig)) = &fused {
        let least = tests.iter().filter(|(t, _)| t.count && !t.always()).map(|(t, _)| t.least).min().unwrap_or(0);
        let (on, join) = (f.create_block(), f.create_block());
        f.append_block_param(join, types::I64);
        f.append_block_param(join, types::I64);
        let long = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, len, i64::from(least));
        f.ins().brif(long, on, &[], join, &[refuse.into(), hits.into()]);
        f.switch_to_block(on);
        // What the tests need of the bytes, together (`native_cpu_bytes::needs`): a token whose
        // bytes (and the few its masks look past) fit one vector has none of them where no byte
        // there is in the set, a compare a run of bytes, and the call is skipped.
        let counts: Vec<crate::records::TokenCheck> = tests.iter().filter(|(t, _)| t.count && !t.always()).map(|(t, _)| *t).collect();
        // Where each count's mask is its bytes exactly (`native_cpu_bytes::exact`), a token of
        // 16 bytes at most has its counts from one vector each, as a lexer counts a float's `.`
        // as it reads it, and the call is made for a longer one alone.
        let exacts: Option<Vec<crate::native_cpu_bytes::Needs>> = rd.bytes.and_then(|(b, _)| counts.iter().map(|t| crate::native_cpu_bytes::exact(b.native, t.o)).collect());
        // Every token in one vector (`most`): no call.
        let mut called = true;
        if let Some(ex) = exacts.filter(|v| v.iter().all(|n| (-8..=8).contains(&n.lo) && (1..=4).contains(&n.ranges().len()))) {
            let (vec, call) = (f.create_block(), f.create_block());
            if most.is_some_and(|m| m <= 16) {
                f.ins().jump(vec, &[]);
                called = false;
            } else {
                let fits = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, len, 16);
                f.ins().brif(fits, vec, &[], call, &[]);
            }
            f.switch_to_block(vec);
            let (mut r, mut h) = (refuse, hits);
            // Each set's bytes in the token, by what its tests ask, as the lean lexer asks
            // whether a number has a `.`: at least one, any lane (`needs_some`'s); at least
            // two, their bits with more than one set (`m & (m - 1)`); more, their count.
            let mut made: Vec<(crate::native_cpu_bytes::Needs, Value)> = Vec::new();
            let weights = rd.bytes.map(|(_, file)| file.weights);
            // The token's bytes after its first in `pre`'s bits: `len - 1` of them.
            let below = (!pre.is_empty()).then(|| {
                let n = f.ins().iadd_imm(len, -1);
                // The token's bytes after its first: `most - 1` at most.
                lanes_below(f, n, pre_stride, most.map_or(16, |m| m.saturating_sub(1)).min(16))
            });
            for (t, n) in counts.iter().zip(&ex) {
                let most = counts.iter().zip(&ex).filter(|(_, m)| *m == n).map(|(t, _)| t.least).max().unwrap_or(0);
                let k = match made.iter().find(|(m, _)| m == n) {
                    Some(&(_, k)) => k,
                    None => {
                        let given = pre.iter().find(|&&(o, _)| o == t.o).map(|&(_, m)| m);
                        let k = match (given, below, most, weights) {
                            // The run's window's bits: a mask where the hit asks a mask, else
                            // its count.
                            (Some(m), Some(b), _, _) => {
                                let k = f.ins().band(m, b);
                                // A bit a lane where the count asks more than one.
                                let k = if most > 1 && pre_stride > 1 { f.ins().band_imm(k, lane_bits(pre_stride)) } else { k };
                                if most <= 1 || (most == 2 && weights.is_some()) { k } else { f.ins().popcnt(k) }
                            }
                            (_, _, 0..=1, _) => needs_some(f, n, c.bytes, s, len),
                            (_, _, 2, Some(w)) => {
                                let lanes = needs_lanes(f, n, c.bytes, s, len);
                                let m = short_mask(f, lanes, w);
                                if SHORT_STRIDE > 1 { f.ins().band_imm(m, lane_bits(SHORT_STRIDE)) } else { m }
                            }
                            _ => needs_count(f, n, c.bytes, s, len),
                        };
                        made.push((*n, k));
                        k
                    }
                };
                let hit = match (most, weights, t.least) {
                    (0..=2, Some(_), 1) | (0..=1, _, 1) => f.ins().icmp_imm(IntCC::NotEqual, k, 0),
                    (2, Some(_), 2) => {
                        let k1 = f.ins().iadd_imm(k, -1);
                        let two = f.ins().band(k, k1);
                        f.ins().icmp_imm(IntCC::NotEqual, two, 0)
                    }
                    _ => f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, k, i64::from(t.least)),
                };
                let hit = f.ins().uextend(types::I64, hit);
                (r, h) = hit_into(f, t.then, hit, r, h);
            }
            f.ins().jump(join, &[r.into(), h.into()]);
            if called {
                f.switch_to_block(call);
            }
        } else if let Some(n) = rd.bytes.and_then(|(b, _)| counts_need(b.native, &counts)) {
            let (vec, call) = (f.create_block(), f.create_block());
            let span = f.ins().iadd_imm(len, i64::from(n.hi - n.lo));
            let fits = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, span, 16);
            f.ins().brif(fits, vec, &[], call, &[]);
            f.switch_to_block(vec);
            let some = needs_some(f, &n, c.bytes, s, span);
            f.ins().brif(some, call, &[], join, &[refuse.into(), hits.into()]);
            f.switch_to_block(call);
        }
        if called {
            let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
            let signature = f.import_signature(sig.clone());
            let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
            let call = f.ins().call(callee, &[c.bytes, c.len, s, l]);
            let got = f.inst_results(call)[0];
            let no = f.ins().sshr_imm(got, 63);
            let refuse2 = f.ins().bor(refuse, no);
            let marks = f.ins().band_imm(got, i64::MAX);
            let hits2 = f.ins().bor(hits, marks);
            f.ins().jump(join, &[refuse2.into(), hits2.into()]);
        }
        f.switch_to_block(join);
        refuse = f.block_params(join)[0];
        hits = f.block_params(join)[1];
        // The other tests from here (`rest`), where a reader's fused read (`read_fn`) gives
        // the counts' hits with its last: `(l, tag, refusal, hits)`.
        // And a split's byte loop's first lane (`way_lane`), given by each way's end: from
        // here, its byte read.
        let way_lane = way_lane.zip(rd.bytes);
        let after = f.create_block();
        for _ in 0..4 + usize::from(way_lane.is_some()) {
            f.append_block_param(after, types::I64);
        }
        let mut args: Vec<BlockArg> = vec![l.into(), tag.into(), refuse.into(), hits.into()];
        if let Some(((set, _), (b, file))) = way_lane {
            let i = f.ins().iadd_imm(s, 1);
            args.push(b.byte_in(f, &file, i, set).into());
        }
        f.ins().jump(after, &args);
        f.switch_to_block(after);
        let ps = f.block_params(after).to_vec();
        (l, tag, refuse, hits) = (ps[0], ps[1], ps[2], ps[3]);
        if let Some(((set, firsts), _)) = way_lane {
            lanes = Some((crate::native_cpu_bytes::RunLanes { mask: ps[4], stride: 1, set, known: 1 }, firsts));
        }
        q0 = f.ins().ushr_imm(s, 6);
        sh = f.ins().band_imm(s, 63);
        ql = f.ins().ushr_imm(l, 6);
        let n = f.ins().isub(l, s);
        len = f.ins().iadd_imm(n, 1);
        ones = f.ins().iconst(types::I64, -1);
        rest = Some(after);
    }
    for &(ref t, filter) in tests {
        if t.always() || t.count && fused.is_some() {
            continue;
        }
        let next = f.create_block();
        f.append_block_param(next, types::I64);
        // A test's hit (0 or 1) into the refusal (`CheckThen::Escape`) or its mark's bit.
        let into = |f: &mut FunctionBuilder<'_>, hit: Value, refuse: Value, hits: Value| -> Value {
            let (r, h) = hit_into(f, t.then, hit, refuse, hits);
            if t.then == crate::records::CheckThen::Escape { r } else { h }
        };
        let escape = t.then == crate::records::CheckThen::Escape;
        let kept = if escape { refuse } else { hits };
        // A test of the token's first byte and a split run's lanes (`Bytewise::bit_on_run`):
        // its bit inline, no byte read past the first, as lean's `b[pos] == b'0'` in its arm.
        if let (Some((rl, firsts)), Some((b, file)), false, 1) = (lanes, rd.bytes, t.count, t.least)
            && b.can(t.o)
            && b.on_run_only(t.o, rl.set, rl.known)
        {
            let bit = b.bit_on_run(f, &file, t.o, s, Some(firsts), rl, heads);
            let hit = into(f, bit, refuse, hits);
            f.ins().jump(next, &[hit.into()]);
            f.switch_to_block(next);
            if escape {
                refuse = f.block_params(next)[0];
            } else {
                hits = f.block_params(next)[0];
            }
            continue;
        }
        // What the compiler proves first, each a branch past the test (`next`, the value kept):
        // the first byte one at which it can hold (`TOKEN_TESTS`, the entry's bit); a count of
        // `n` in a token of `n` bytes at least.
        if let Some(b) = filter {
            let on = f.create_block();
            let may = f.ins().band_imm(entry, 1i64 << b);
            f.ins().brif(may, on, &[], next, &[kept.into()]);
            f.switch_to_block(on);
            f.set_cold_block(on);
        }
        if t.count && t.least > 1 {
            let on = f.create_block();
            let long = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, len, i64::from(t.least));
            f.ins().brif(long, on, &[], next, &[kept.into()]);
            f.switch_to_block(on);
        }
        if t.count {
            // The mask's bits over the token's words: the first from `s`, the last to `l`.
            let (head, body) = (f.create_block(), f.create_block());
            f.append_block_param(head, types::I64);
            f.append_block_param(head, types::I64);
            heads.push((head, Step::Word));
            let zero = f.ins().iconst(types::I64, 0);
            f.ins().jump(head, &[q0.into(), zero.into()]);
            f.switch_to_block(head);
            let (q, acc) = (f.block_params(head)[0], f.block_params(head)[1]);
            let x = word_at(f, q, t.o, heads);
            let first = f.ins().icmp(IntCC::Equal, q, q0);
            let from = f.ins().ishl(ones, sh);
            let from = f.ins().select(first, from, ones);
            let x = f.ins().band(x, from);
            let last = f.ins().icmp(IntCC::Equal, q, ql);
            let lb = f.ins().band_imm(l, 63);
            let lb = f.ins().irsub_imm(lb, 63);
            let to = f.ins().ushr(ones, lb);
            let to = f.ins().select(last, to, ones);
            let x = f.ins().band(x, to);
            let pc = f.ins().popcnt(x);
            let acc = f.ins().iadd(acc, pc);
            let q1 = f.ins().iadd_imm(q, 1);
            f.ins().brif(last, body, &[], head, &[q1.into(), acc.into()]);
            f.switch_to_block(body);
            let hit = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, acc, i64::from(t.least));
            let hit = f.ins().uextend(types::I64, hit);
            let hit = into(f, hit, refuse, hits);
            f.ins().jump(next, &[hit.into()]);
        } else {
            // The token `least` long at least, and the mask at each of its first `least`.
            let (test, two, one) = (f.create_block(), f.create_block(), f.create_block());
            f.append_block_param(one, types::I64);
            let long = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, len, i64::from(t.least));
            f.ins().brif(long, test, &[], next, &[kept.into()]);
            f.switch_to_block(test);
            // Read at the token's bytes (`native_cpu_bytes`): its first `least`, straight-line.
            if rd.bytewise(t.o) && t.least <= 8 {
                let mut all = f.ins().iconst(types::I64, 1);
                for j in 0..t.least {
                    let at = f.ins().iadd_imm(s, i64::from(j));
                    let b = rd.bit(f, c, at, t.o, heads);
                    all = f.ins().band(all, b);
                }
                let hit = into(f, all, refuse, hits);
                f.ins().jump(next, &[hit.into()]);
                // The mask's form's blocks, unreached.
                let _ = (two, one);
                f.switch_to_block(next);
                if escape {
                    refuse = f.block_params(next)[0];
                } else {
                    hits = f.block_params(next)[0];
                }
                continue;
            }
            let x = word_at(f, q0, t.o, heads);
            let lo = f.ins().ushr(x, sh);
            let end = f.ins().iadd_imm(sh, i64::from(t.least));
            let past = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, end, 64);
            f.ins().brif(past, two, &[], one, &[lo.into()]);
            // Past the word: its next word's low bits after `lo`'s (in the token, so in the file).
            f.switch_to_block(two);
            f.set_cold_block(two);
            let q1 = f.ins().iadd_imm(q0, 1);
            let y = word_at(f, q1, t.o, heads);
            let back = f.ins().irsub_imm(sh, 64);
            let hi = f.ins().ishl(y, back);
            let win = f.ins().bor(lo, hi);
            f.ins().jump(one, &[win.into()]);
            f.switch_to_block(one);
            let win = f.block_params(one)[0];
            let need = (!0u64 >> (64 - t.least)) as i64;
            let m = f.ins().band_imm(win, need);
            let hit = f.ins().icmp_imm(IntCC::Equal, m, need);
            let hit = f.ins().uextend(types::I64, hit);
            let hit = into(f, hit, refuse, hits);
            f.ins().jump(next, &[hit.into()]);
        }
        f.switch_to_block(next);
        if escape {
            refuse = f.block_params(next)[0];
        } else {
            hits = f.block_params(next)[0];
        }
    }
    f.ins().jump(done, &[l.into(), refuse.into(), tag.into(), hits.into()]);
    f.switch_to_block(here);
    (ended, rest)
}

/// A file's records (`records::pass`): per token, its code from its first and last bytes, stored
/// in the codes' column; where the streams disagree, the totals' flag set (the count after the
/// extracts'). Then on to `then`. A token's loop (`Step::Token`); its work is straight-line, but
/// for a word past eight bytes, whose other words are compared in a block of their own.
#[allow(clippy::too_many_arguments)]
fn record_file(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, t: &RecordTables, extracts: usize, outputs: usize, c: &FileCols, then: cranelift_codegen::ir::Block, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    record_tokens(f, r, t, extracts, outputs, c, None, None, then, heads);
}

/// The token loop's state between its parts (`RecPart`): the next token's index, the end of
/// the token before, the refusal so far, the marks the next grammar token has from the tokens
/// before (`records::gap_marks`' `pending`), the byte from which the gap marks' bits are not
/// yet in `pending` (`from`), and the word of the grammar token before (`unless after`); six.
const STATE_WORDS: usize = 6;

/// In the marks so far (`pending`): the cursor's block has a gap mark's bit (`loop_marks`'
/// lift), set by the refill. A token whose `pending` and hits are zero then takes no merge of
/// the marks, and its gap test reads no word (`record_tokens`). Past every mark's bit.
const GAP_FLAG: i64 = 1 << 62;

/// In the marks so far (`pending`): the cursor's block is read again from the cursor, as a run
/// of pairs goes on across the token's end (`record_tokens`' `chained`), set by the token and
/// cleared by the refill. Without it, a refill whose cursor's block the token before started in
/// reads the next block: that one's starts are spent. Past every mark's bit.
const REREAD_FLAG: i64 = i64::MIN;

/// The gap marks tier 2 on the CPU tests itself, where a state asks (decision 0177): each
/// `(mark, output)`, a gap mark made from the bytes (no `else escape`, not the extras'). The
/// token loop makes none of their gaps; tier 2 reads the mask from the grammar token before to
/// the token, as the lean parser asks whether a token is on a line only where a statement asks
/// it (`on_line`), and reads a JSON document's bytes only where it takes one. A mark with token
/// tests too (`lazy_coded`) keeps them in the code, as a lexer marks what it reads: tier 2
/// takes its bit there first. Metal and the interpreter set them in the codes: the compiler
/// chooses by the target, the GPU's tier 1 making every mark at once.
pub fn lazy_marks(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> Vec<(u8, u16)> {
    // A mark with tests and an `unless after` stays the loop's: the word before holds its
    // tests' hits too.
    let coded = lazy_coded(r);
    let held = |k: u8| coded >> k & 1 == 1 && r.unless.iter().any(|&(m, _)| m == k);
    loop_marks(r, bytewise).0.into_iter().filter(|&(k, _)| r.tested_only >> k & 1 == 0 && r.unplaced >> k & 1 == 0 && r.extra != Some(k) && !held(k)).collect()
}

/// The marks of `lazy_marks` with token tests too: their tests' hits are in the code (the
/// token loop's), their gaps tier 2's.
pub fn lazy_coded(r: &crate::records::Records) -> u32 {
    r.checks.iter().fold(0, |a, c| match c.then {
        crate::records::CheckThen::At(k) | crate::records::CheckThen::After(k) => a | 1 << k,
        crate::records::CheckThen::Escape => a,
    })
}

/// The marks the token loop sets itself (decision 0177), their masks made from the bytes
/// (`Reads`): each `(mark, output)`. The gap marks' (`lift`), from the bytes between grammar
/// tokens, as `records::gap_marks`; the others', at a token's first byte. A mark a reader gives
/// (`Reader::retag`) or of tests alone is not.
pub(crate) fn loop_marks(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> (Vec<(u8, u16)>, Vec<(u8, u16)>) {
    if r.skip.is_none() {
        return (Vec::new(), Vec::new());
    }
    let retagged: u32 = r.readers.iter().filter_map(|rd| rd.retag).fold(0, |a, (_, m)| a | m);
    let mut lift = Vec::new();
    let mut at = Vec::new();
    for (k, &(o, w)) in r.marks.iter().enumerate() {
        if o == crate::records::WORD_ALONE || w != 0 || retagged >> k & 1 == 1 || r.tested_only >> k & 1 == 1 || !bytewise(o) {
            continue;
        }
        if r.gaps >> k & 1 == 1 { lift.push((k as u8, o)) } else { at.push((k as u8, o)) }
    }
    (lift, at)
}

/// The marks of `marks` with a bit in the file's bytes `a ..= b` (none where `a > b`), as bits
/// `1 << mark`. In the block in registers (`regs`, its first byte `base`) when `a` is in it, `b`
/// too; else a word at a time from the bytes, in cold blocks.
#[allow(clippy::too_many_arguments)]
fn marks_in(f: &mut FunctionBuilder<'_>, rd: &Reads<'_>, c: &FileCols, marks: &[(u8, u16)], a: Value, b: Value, regs: Option<(&BlockRegs, Value)>, inside_known: bool, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) -> Value {
    let join = f.create_block();
    f.append_block_param(join, types::I64);
    let zero = f.ins().iconst(types::I64, 0);
    let ones = f.ins().iconst(types::I64, -1);
    let slow = f.create_block();
    if let Some((g, base)) = regs {
        let fast = f.create_block();
        // `a` in the block where the refill says so (`inside_known`): no compare.
        if inside_known {
            f.ins().jump(fast, &[]);
        } else {
            let inside = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, a, base);
            f.ins().brif(inside, fast, &[], slow, &[]);
        }
        f.switch_to_block(fast);
        // The marks are sparse: a block whose mark words are all zero has none of their bits,
        // one branch on the registers (a form feed's mark: almost never set).
        let words: Vec<Value> = marks.iter().map(|&(_, o)| g.runs.iter().find(|&&(k, _)| k == o).map(|&(_, v)| v).expect("a mark's word in the registers")).collect();
        let some = words[1..].iter().fold(words[0], |x, &w| f.ins().bor(x, w));
        let set = f.create_block();
        f.ins().brif(some, set, &[], join, &[zero.into()]);
        f.switch_to_block(set);
        f.set_cold_block(set);
        let lo = f.ins().isub(a, base);
        let hi = f.ins().isub(b, base);
        let hi = f.ins().irsub_imm(hi, 63);
        let from = f.ins().ishl(ones, lo);
        let to = f.ins().ushr(ones, hi);
        let range = f.ins().band(from, to);
        let mut acc = zero;
        for &(m, o) in marks {
            let w = g.runs.iter().find(|&&(k, _)| k == o).map(|&(_, v)| v).expect("a mark's word in the registers");
            let x = f.ins().band(w, range);
            let any = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
            let any = f.ins().uextend(types::I64, any);
            let bit = f.ins().ishl_imm(any, i64::from(m));
            acc = f.ins().bor(acc, bit);
        }
        f.ins().jump(join, &[acc.into()]);
        if inside_known {
            f.switch_to_block(join);
            return f.block_params(join)[0];
        }
    } else {
        f.ins().jump(slow, &[]);
    }
    f.switch_to_block(slow);
    f.set_cold_block(slow);
    let (head, go) = (f.create_block(), f.create_block());
    f.append_block_param(head, types::I64);
    f.append_block_param(head, types::I64);
    let empty = f.ins().icmp(IntCC::SignedGreaterThan, a, b);
    let qa = f.ins().ushr_imm(a, 6);
    let qb = f.ins().ushr_imm(b, 6);
    f.ins().brif(empty, join, &[zero.into()], go, &[]);
    f.switch_to_block(go);
    f.set_cold_block(go);
    f.ins().jump(head, &[qa.into(), zero.into()]);
    f.switch_to_block(head);
    f.set_cold_block(head);
    heads.push((head, Step::Word));
    let (w, acc0) = (f.block_params(head)[0], f.block_params(head)[1]);
    let first = f.ins().icmp(IntCC::Equal, w, qa);
    let last = f.ins().icmp(IntCC::Equal, w, qb);
    let sa = f.ins().band_imm(a, 63);
    let from = f.ins().ishl(ones, sa);
    let from = f.ins().select(first, from, ones);
    let sb = f.ins().band_imm(b, 63);
    let sb = f.ins().irsub_imm(sb, 63);
    let to = f.ins().ushr(ones, sb);
    let to = f.ins().select(last, to, ones);
    let range = f.ins().band(from, to);
    let mut acc = acc0;
    for &(m, o) in marks {
        let x = rd.word(f, c, w, o, heads);
        let x = f.ins().band(x, range);
        let any = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
        let any = f.ins().uextend(types::I64, any);
        let bit = f.ins().ishl_imm(any, i64::from(m));
        acc = f.ins().bor(acc, bit);
    }
    let w1 = f.ins().iadd_imm(w, 1);
    let on = f.create_block();
    f.ins().brif(last, join, &[acc.into()], on, &[]);
    f.switch_to_block(on);
    f.set_cold_block(on);
    f.ins().jump(head, &[w1.into(), acc.into()]);
    f.switch_to_block(join);
    f.block_params(join)[0]
}

/// The cursor's block in registers (decision 0177), as the lean lexer's window: the starts
/// left (`cand`, its bits from the cursor), and each run rule's word (output, word).
#[derive(Clone)]
struct BlockRegs {
    cand: Value,
    runs: Vec<(u16, Value)>,
    /// The block's first byte's position: the next start's base, the block's, as lean's
    /// `base + q`, not the token's end's (which chained each start to the token before).
    base: Value,
    /// Split runs' term words, by their byte sets (`RunForm::Block`).
    terms: Vec<([u64; 4], Value)>,
}


/// A part of a file's token loop (decision 0177): the tokens whose words are in the `ready`
/// blocks, from the state `init`; it goes on to `then` with the state it stops at.
#[derive(Clone, Copy)]
struct RecPart {
    ready: Value,
    init: [Value; STATE_WORDS],
}

/// A file's token loop: the whole file's (`part` none), then its end (`record_end`); or a
/// part of it, which waits where a token reads past the ready blocks and stores its state.
#[allow(clippy::too_many_arguments)]
fn record_tokens(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, t: &RecordTables, extracts: usize, outputs: usize, c: &FileCols, part: Option<RecPart>, bw: Option<(&crate::native_cpu_bytes::Bytewise<'_>, &Consts, &[ByteCall])>, then: cranelift_codegen::ir::Block, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let rd = Reads { bytes: bw.map(|(b, k, _)| (b, crate::native_cpu_bytes::File { bytes: c.bytes, len: c.len, nibble: k.nibble, weights: k.weights, even: k.even })), outputs, calls: bw.map_or(&[][..], |x| x.2) };
    let (head, token, finish) = (f.create_block(), f.create_block(), f.create_block());
    // The loop's values: the token, the end of the token before, the refusal so far, and the
    // marks' tests' bits for the next grammar token (`CheckThen`).
    for _ in 0..STATE_WORDS {
        f.append_block_param(head, types::I64);
    }
    // At `finish`, the refusal, the tokens' count, the marks' bits left and where the gap
    // marks' bits not in them start.
    for _ in 0..4 {
        f.append_block_param(finish, types::I64);
    }
    // The tokens by the loop (stage 2a): each start found at the head.
    let walked = r.skip;
    if walked.is_some() {
        f.append_block_param(token, types::I64);
    }
    // The block's starts and runs, in registers (`BlockRegs`), as the lean lexer's: the head
    // has the starts left in the cursor's block, and the run rules' words there.
    // The marks the loop sets (`loop_marks`), their words in the registers too.
    let (mut lift, at_marks) = if rd.bytes.is_some() { loop_marks(r, &|o| rd.bytewise(o)) } else { (Vec::new(), Vec::new()) };
    // Tier 2's own (`lazy_marks`): the loop sets none of them.
    if rd.bytes.is_some() {
        let lazy = lazy_marks(r, &|o| rd.bytewise(o));
        lift.retain(|m| !lazy.contains(m));
    }
    let runs: Vec<u16> = if walked.is_some() {
        // One run word in a register a block: the run rule most first bytes take (a word's),
        // as the lean lexer's window has its `word` mask alone. A rarer run (a number's) reads
        // its word at its token, by a call: no window a block for a token in hundreds.
        let first_bytes = |k: usize| (0..256).filter(|&b| r.ends.get(b).is_some_and(|&e| usize::from(e) == k)).count();
        let main = (0..r.rules.len()).filter(|&k| matches!(r.rules[k], crate::records::EndRule::Run(_))).max_by_key(|&k| first_bytes(k));
        let mut v: Vec<u16> = main.into_iter().filter_map(|k| match r.rules[k] {
            crate::records::EndRule::Run(o) => Some(o),
            _ => None,
        }).collect();
        v.extend(lift.iter().chain(&at_marks).map(|&(_, o)| o));
        v.sort_unstable();
        v.dedup();
        v
    } else {
        Vec::new()
    };
    // The pair rule's word a block (`PAIR_WORD`, last of the registers), where the loop fuses
    // it (`fused_rules`) and its table is nibbles' (`pair_nibbles`), as the lean lexer's window.
    let pairs: Option<[[u8; 16]; 4]> = rd.bytes.and(fused_rules(r, t, &rd, &runs)).and_then(|(kn, _)| match r.rules[kn] {
        crate::records::EndRule::Next(i) => r.nexts.get(usize::from(i)).and_then(|pt| pair_nibbles(r, pt, kn)),
        _ => None,
    });
    let fused_ks = if rd.bytes.is_some() { fused_rules(r, t, &rd, &runs) } else { None };
    let runs: Vec<u16> = runs.into_iter().chain(pairs.map(|_| PAIR_WORD)).collect();
    let outs: Vec<u16> = runs.iter().copied().filter(|&o| o != PAIR_WORD).collect();
    // The lean lexer's starts (`lean_starts`): the run word's and the pair word's places in the
    // registers, and whether pairs chain.
    let lean: Option<(usize, usize, bool)> = match (walked, fused_ks, pairs, rd.bytes) {
        (Some(_), Some((kn, kr)), Some(_), Some(_)) if t.fused == Some((kn, kr)) => t.lean.and_then(|chained| {
            let crate::records::EndRule::Run(wo) = r.rules[kr] else { return None };
            Some((runs.iter().position(|&k| k == wo)?, runs.iter().position(|&k| k == PAIR_WORD)?, chained))
        }),
        _ => None,
    };
    if walked.is_some() {
        for _ in 0..1 + runs.len() {
            f.append_block_param(head, types::I64);
        }
    }
    let ro = MemFlags::trusted().with_readonly();
    let fl = MemFlags::trusted();
    let stores = MemFlags::new().with_notrap();
    let zero = f.ins().iconst(types::I64, 0);
    let tables = c.tables;
    // The file's last block and its bytes' bits (`valid`), once a file, as the lean lexer's
    // `valid`: the refill cuts its starts by them, no token compares its start with the length.
    let file_end = walked.map(|_| {
        let ones = f.ins().iconst(types::I64, -1);
        let l1 = f.ins().iadd_imm(c.len, -1);
        let last_q = f.ins().ushr_imm(l1, 6);
        let tail = f.ins().band_imm(c.len, 63);
        let cut = f.ins().irsub_imm(tail, 64);
        let cut = f.ins().band_imm(cut, 63);
        let last_valid = f.ins().ushr(ones, cut);
        let l63 = f.ins().iadd_imm(c.len, 63);
        let file_blocks = f.ins().ushr_imm(l63, 6);
        let blocks = f.ins().umin(c.blocks, file_blocks);
        (last_q, last_valid, blocks)
    });
    // With end rules (decision 0177) the lasts are this pass's: as many as the starts.
    let ruled = !r.rules.is_empty();
    let (differ, n) = if walked.is_some() {
        (zero, zero)
    } else if ruled {
        (zero, c.ns)
    } else {
        let differ = f.ins().icmp(IntCC::NotEqual, c.ns, c.nl);
        (f.ins().uextend(types::I64, differ), f.ins().umin(c.ns, c.nl))
    };
    // The refusal so far: the differences that must not be negative, ORed (their sign bit).
    // A part starts from the state the one before stored.
    let wait = part.map(|_| f.create_block());
    let limit = match (part, wait) {
        (Some(p), Some(wait)) => {
            let k0 = c.cursor(f, p.init[0]);
            let init: Vec<BlockArg> = std::iter::once(k0).chain(p.init[1..].iter().copied()).map(BlockArg::from).chain((0..if walked.is_some() { 1 + runs.len() } else { 0 }).map(|_| BlockArg::from(zero))).collect();
            let last = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, p.ready, c.blocks);
            f.ins().jump(head, &init);
            Some(Limit { ready: p.ready, last, wait })
        }
        _ => {
            let k0 = c.cursor(f, zero);
            let init: Vec<BlockArg> = std::iter::once(k0).chain((1..STATE_WORDS + if walked.is_some() { 1 + runs.len() } else { 0 }).map(|_| zero)).map(BlockArg::from).collect();
            f.ins().jump(head, &init);
            None
        }
    };
    heads.push((head, Step::Token));

    f.switch_to_block(head);
    let mut h = f.block_params(head).to_vec();
    // The head's state, which a wait stores for the next part (below), and the bits the refill
    // took from the words the head carries (`carried`), which the next part's head has not.
    let head_h = h.clone();
    let mut carried_bits: Option<Value> = None;
    let mut regs = None;
    // The marks' bits so far and where the gap marks' bits not in them start, at the token.
    let (mut pending_in, mut from_in) = (h[3], h[4]);
    // Whether the refill folds the gap marks' bits of the blocks it passes into the bits so far
    // (decision 0177), so `from` is in the token's block: its test in the registers, as the
    // lean lexer's window has its masks. Where the bytes make the words, waiting on no block.
    let mut folded = false;
    // Whether a refill reads the block after a spent one (`REREAD_FLAG`): with the lean
    // lexer's starts, the gap marks folded or none in the loop, in one part. Then the state's
    // `from` (`STATE_WORDS`), which no token changes there, is the block after the one the
    // refill read (`spent_q`), as the lean lexer's `base`.
    let mut skips = false;
    // The inner loop's header (`from_word`), where the loop has one.
    let mut inner: Option<cranelift_codegen::ir::Block> = None;
    // The loop over a block's runs (`mid`, its header) and the starts after the run (`rest`),
    // where the rare starts split them, as the lean lexer's.
    let mut runs_loop: Option<(cranelift_codegen::ir::Block, Value, cranelift_codegen::ir::Block)> = None;
    // The rare starts' head, where the runs loop has one: its token block, the state, the
    // block's registers, as the run loop's (`Side::Rare`).
    let mut rare_part: Option<(cranelift_codegen::ir::Block, Vec<Value>, BlockRegs)> = None;
    match walked {
        Some(o) => {
            // The next start, the first of the block's left (`cand`); none left, the cursor's
            // block loaded (`refill`): its starts from the cursor, its runs.
            let refill_limit = if rd.bytewise(o) && outs.iter().all(|&k| rd.bytewise(k)) { None } else { limit };
            folded = !lift.is_empty() && refill_limit.is_none();
            skips = (folded || lift.is_empty()) && refill_limit.is_none() && lean.is_some();
            // Two loops, as the lean lexer's: over blocks (`head`, its refill), and over a
            // block's starts (`from_word`). The block's words are the refill's, which the
            // inner loop's every token reads and none changes; a token that leaves starts
            // goes on in the inner loop, else back to the refill.
            let (refill, from_word) = (f.create_block(), f.create_block());
            for _ in 0..2 + STATE_WORDS {
                f.append_block_param(from_word, types::I64);
            }
            f.ins().jump(refill, &[]);
            // A block's words: from the cursor's, a word at a time past those with no start.
            // Once a block, not a token: the token's way is the loop's hot one, as the lean
            // lexer's inner run loop inside its loop over windows (`set_cold_block`, the
            // weights rustc's allocator spills by).
            f.switch_to_block(refill);
            f.set_cold_block(refill);
            let more = f.create_block();
            f.append_block_param(more, types::I64);
            f.append_block_param(more, types::I64);
            f.append_block_param(more, types::I64);
            let qc = f.ins().ushr_imm(h[1], 6);
            let sh = f.ins().band_imm(h[1], 63);
            let ones = f.ins().iconst(types::I64, -1);
            let mc = f.ins().ishl(ones, sh);
            // The cursor's block read already: the token before started in it, so its starts
            // are spent (a token past it moves the cursor on). The next block then, whole, as
            // the lean lexer's next window; not this one again from the cursor. But where the
            // token before asks it (`REREAD_FLAG`: pairs on across its end), this one from
            // the cursor. Where the loop does not skip (`skips`), it reads it again. Its block
            // is the one the refill read last (the state's, `spent_q`): the lean lexer's
            // `base`, in a register, no record read back.
            let h3 = f.ins().band_imm(h[3], !REREAD_FLAG);
            // By a branch, not a `select`: on the common way the next block is the spent one's
            // next (`h[4]`, in a register), so its loads wait on no token. A `select` of the
            // cursor's block made every refill's loads wait for the block's last token's end,
            // the token chain and the refill in series, where the lean lexer's next window is
            // `base + 64` whatever its tokens did (0177, "The next block waits on no token").
            let (q0, m0) = if skips {
                let read = f.ins().iadd_imm(qc, 1);
                let spent = f.ins().icmp(IntCC::Equal, h[4], read);
                let reread = f.ins().icmp_imm(IntCC::SignedLessThan, h[3], 0);
                let reread = f.ins().bnot(reread);
                let here = f.ins().band(spent, reread);
                let (next, cursor, chosen) = (f.create_block(), f.create_block(), f.create_block());
                f.append_block_param(chosen, types::I64);
                f.append_block_param(chosen, types::I64);
                f.ins().brif(here, next, &[], cursor, &[]);
                f.switch_to_block(next);
                f.ins().jump(chosen, &[h[4].into(), ones.into()]);
                f.switch_to_block(cursor);
                f.set_cold_block(cursor);
                f.ins().jump(chosen, &[qc.into(), mc.into()]);
                f.switch_to_block(chosen);
                (f.block_params(chosen)[0], f.block_params(chosen)[1])
            } else {
                (qc, mc)
            };
            // The gap marks' bits before the cursor's block (a token across blocks): by the
            // bytes, a word at a time, cold. Then from the cursor's block, each passed one's.
            let acc0 = if folded {
                let fr = c.after_last_start(f, h[0]);
                // The last start's block is the one the token loop left, its gap marks' words
                // at the head (`carried`): its bits past the start from them, as the lean
                // lexer's window keeps its masks. Only blocks a token crossed, by the bytes.
                let q064 = f.ins().ishl_imm(q0, 6);
                let before = f.ins().iadd_imm(q064, -1);
                let lo = f.ins().band_imm(fr, 63);
                let inside = f.ins().icmp_imm(IntCC::NotEqual, lo, 0);
                let ahead = f.ins().icmp(IntCC::UnsignedLessThan, fr, q064);
                let inside = f.ins().band(inside, ahead);
                let above = f.ins().ishl(ones, lo);
                let mut a = zero;
                for &(mk, mo) in &lift {
                    let w = h[STATE_WORDS + 1 + runs.iter().position(|&k| k == mo).expect("a mark's word at the head")];
                    let x = f.ins().band(w, above);
                    let any = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
                    let any = f.ins().band(any, inside);
                    let any = f.ins().uextend(types::I64, any);
                    let bit = f.ins().ishl_imm(any, i64::from(mk));
                    a = f.ins().bor(a, bit);
                }
                carried_bits = Some(a);
                let next_q = f.ins().bor_imm(fr, 63);
                let next_q = f.ins().iadd_imm(next_q, 1);
                let rest = f.ins().select(inside, next_q, fr);
                let crossed = marks_in(f, &rd, c, &lift, rest, before, None, false, heads);
                f.ins().bor(a, crossed)
            } else {
                zero
            };
            f.ins().jump(more, &[q0.into(), m0.into(), acc0.into()]);
            f.switch_to_block(more);
            heads.push((more, Step::Word));
            let (q, m, acc) = (f.block_params(more)[0], f.block_params(more)[1], f.block_params(more)[2]);
            // Where the gap marks' bits not in `acc` start: in block `q` or before it.
            let (pend_q, from_q) = if folded {
                let q64 = f.ins().ishl_imm(q, 6);
                let fr = c.after_last_start(f, h[0]);
                (f.ins().bor(h3, acc), f.ins().umax(fr, q64))
            } else {
                (h3, h[4])
            };
            // From the bytes, the block's words wait on no block (`Reads`).
            // From the bytes, no block past the file's last.
            let to = match (refill_limit, file_end) {
                (None, Some((_, _, blocks))) => blocks,
                _ => bound(c, refill_limit),
            };
            let inside = f.ins().icmp(IntCC::UnsignedLessThan, q, to);
            let (look, next) = (f.create_block(), f.create_block());
            inside_or_past(f, inside, look, refill_limit, finish, &[h[2].into(), h[0].into(), pend_q.into(), if folded { h[4] } else { from_q }.into()]);
            f.switch_to_block(look);
            // The skip and the run words from one window of the block's bytes.
            let all: Vec<u16> = std::iter::once(o).chain(outs.iter().copied()).collect();
            // The block's rare starts' set (`RecordTables::rare`), as the lean lexer's `rare`:
            // tested in the same window as the skip and run words, its classes packed with
            // theirs (`class_pack`), a skipped byte's answer free (no start is one).
            let rare_need = match (t.rare, rd.bytes) {
                (Some(n), Some((b, _))) if lean.is_some() => {
                    let set: crate::class_pack::ByteSet = std::array::from_fn(|k| (0..64).filter(|&i| {
                        let x = 64 * k + i;
                        n[0][x & 15] & n[1][x >> 4] != 0
                    }).fold(0u64, |a, i| a | 1 << i));
                    let skip = usize::from(b.native.outputs[usize::from(o)]);
                    let care = match crate::native_cpu_lanes::byte_set(b.native, skip) {
                        Some(sk) if b.native.output_delays[usize::from(o)] == 0 => std::array::from_fn(|k| !sk[k]),
                        _ => [u64::MAX; 4],
                    };
                    Some(crate::class_pack::Need { set, care })
                }
                _ => None,
            };
            // Split runs' term sets (`RunForm::Block`): their words in the same window.
            let term_sets: Vec<[u64; 4]> = match (run_form(), rd.bytes) {
                (RunForm::Block, Some((b, _))) => {
                    let mut v: Vec<[u64; 4]> = r.rules.iter().filter_map(|rule| match rule {
                        crate::records::EndRule::Run(o) if b.can(*o) => b.run_split(*o).map(|x| x.2),
                        _ => None,
                    }).collect();
                    v.sort_unstable();
                    v.dedup();
                    v
                }
                _ => Vec::new(),
            };
            let (got, rare_got, term_words) = match (rare_need, rd.bytes.filter(|(b, _)| all.iter().all(|&x| b.can(x)))) {
                (Some(need), Some((b, file))) => {
                    let p = f.ins().ishl_imm(q, 6);
                    let as_set = |set: &[u64; 4]| -> crate::class_pack::ByteSet { *set };
                    let extras: Vec<crate::class_pack::Need> = std::iter::once(need).chain(term_sets.iter().map(|set| crate::class_pack::Need { set: as_set(set), care: [u64::MAX; 4] })).collect();
                    let (w, more) = b.words_with(f, &file, &all, &extras, p, heads);
                    let tw: Vec<([u64; 4], Value)> = term_sets.iter().copied().zip(more[1..].iter().copied()).collect();
                    (w, Some(more[0]), tw)
                }
                _ => (rd.words_inline(f, c, q, &all, heads), None, Vec::new()),
            };
            let (x, mut words) = (got[0], got[1..].to_vec());
            if let (Some(n), Some((_, file))) = (&pairs, rd.bytes) {
                let p = f.ins().ishl_imm(q, 6);
                words.push(pair_word(f, &file, n, p));
            }
            let x = f.ins().bnot(x);
            let x = f.ins().band(x, m);
            let q64 = f.ins().ishl_imm(q, 6);
            // None past the file's last byte, once a block, as the lean lexer's `valid`: no
            // token compares its start with the file's length.
            let valid = match (refill_limit, file_end) {
                (None, Some((last_q, last_valid, _))) => {
                    let last = f.ins().icmp(IntCC::Equal, q, last_q);
                    f.ins().select(last, last_valid, ones)
                }
                _ => {
                    // The words may go on past the file's block (none of its bytes in them).
                    let rem = f.ins().isub(c.len, q64);
                    let rem = f.ins().smax(rem, zero);
                    let one = f.ins().iconst(types::I64, 1);
                    let below = f.ins().ishl(one, rem);
                    let below = f.ins().iadd_imm(below, -1);
                    let whole = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, rem, 64);
                    f.ins().select(whole, ones, below)
                }
            };
            let mut x = f.ins().band(x, valid);
            // The lean lexer's starts: but a run's bytes after its first (from the cursor), and
            // but a pair's second byte, pairs read left to right where they chain.
            if let Some((wi, pi, chained)) = lean {
                let w = words[wi];
                let wm = f.ins().band(w, m);
                let wl = f.ins().ishl_imm(wm, 1);
                let inner = f.ins().band(w, wl);
                x = f.ins().band_not(x, inner);
                let mut p = f.ins().band(words[pi], x);
                if chained {
                    // Each run of pairs' first bytes keeps its bits an even way from its first:
                    // a run from an even place its even bits, from an odd place its odd ones.
                    // Only where two are next to each other (`=>=`), as the lean lexer reads
                    // them left to right only then: a branch, the rare case's work in it.
                    let pl = f.ins().ishl_imm(p, 1);
                    let adjacent = f.ins().band(p, pl);
                    let (chain, join) = (f.create_block(), f.create_block());
                    f.append_block_param(join, types::I64);
                    f.ins().brif(adjacent, chain, &[], join, &[p.into()]);
                    f.switch_to_block(chain);
                    f.set_cold_block(chain);
                    let edges = f.ins().band_not(p, pl);
                    let even = f.ins().iconst(types::I64, 0x5555_5555_5555_5555);
                    let from_even = f.ins().band(edges, even);
                    let sum = f.ins().iadd(p, from_even);
                    let runs_even = f.ins().band_not(p, sum);
                    let keep_e = f.ins().band(runs_even, even);
                    let odd_runs = f.ins().band_not(p, runs_even);
                    let keep_o = f.ins().band_not(odd_runs, even);
                    let kept = f.ins().bor(keep_e, keep_o);
                    f.ins().jump(join, &[kept.into()]);
                    f.switch_to_block(join);
                    p = f.block_params(join)[0];
                }
                let seconds = f.ins().ishl_imm(p, 1);
                x = f.ins().band_not(x, seconds);
            }
            // The block's rare starts' word (`RecordTables::rare`), as the lean lexer's `rare`.
            let rare_w = match (rare_got, t.rare, rd.bytes) {
                (Some(w), _, _) => Some(w),
                (None, Some(n), Some((_, file))) if lean.is_some() => Some(set_word(f, &file, &n, q64)),
                _ => None,
            };
            let tz = f.ins().ctz(x);
            let s1 = f.ins().iadd(q64, tz);
            // Whether the block has a gap mark's bit (`GAP_FLAG`), in the marks so far.
            let pend_q = if folded {
                let mut any = zero;
                for &(_, mo) in &lift {
                    any = f.ins().bor(any, words[runs.iter().position(|&k| k == mo).expect("a mark's word in the registers")]);
                }
                let has = f.ins().icmp_imm(IntCC::NotEqual, any, 0);
                let has = f.ins().uextend(types::I64, has);
                let flag = f.ins().ishl_imm(has, 62);
                let kept = f.ins().band_imm(pend_q, !GAP_FLAG);
                f.ins().bor(kept, flag)
            } else {
                pend_q
            };
            // The carried `from` goes on as it came where the loop reads it from the records
            // (`after_last_start`): a value no token changes.
            // Where the refill skips, the block after the one it read (`spent_q`).
            let spent_q = if skips { f.ins().iadd_imm(q, 1) } else { h[4] };
            let state = [h[0], h[1], h[2], if folded { pend_q } else { h[3] }, spent_q, h[5]];
            let mid = rare_w.map(|_| {
                let mid = f.create_block();
                for _ in 0..1 + STATE_WORDS {
                    f.append_block_param(mid, types::I64);
                }
                mid
            });
            // A rare start's head: its own, as the lean lexer's `match` on a rare token's first
            // byte, apart from the run loop's.
            let rare_word = mid.map(|_| {
                let b = f.create_block();
                for _ in 0..2 + STATE_WORDS {
                    f.append_block_param(b, types::I64);
                }
                b
            });
            match mid {
                Some(mid) => {
                    let args: Vec<BlockArg> = std::iter::once(&x).chain(&state).map(|&v| v.into()).collect();
                    f.ins().brif(x, mid, &args, next, &[]);
                }
                None => {
                    let args: Vec<BlockArg> = [s1, x].iter().chain(&state).map(|&v| v.into()).collect();
                    f.ins().brif(x, from_word, &args, next, &[]);
                }
            }
            f.switch_to_block(next);
            let q1 = f.ins().iadd_imm(q, 1);
            // A block with no start: its gap marks' bits from `from_q`, into `acc`.
            let acc1 = if folded {
                let lo = f.ins().band_imm(from_q, 63);
                let range = f.ins().ishl(ones, lo);
                let mut a = acc;
                for &(mk, mo) in &lift {
                    let w = words[runs.iter().position(|&k| k == mo).expect("a mark's word in the registers")];
                    let x = f.ins().band(w, range);
                    let any = f.ins().icmp_imm(IntCC::NotEqual, x, 0);
                    let any = f.ins().uextend(types::I64, any);
                    let bit = f.ins().ishl_imm(any, i64::from(mk));
                    a = f.ins().bor(a, bit);
                }
                a
            } else {
                acc
            };
            f.ins().jump(more, &[q1.into(), ones.into(), acc1.into()]);
            // The block's runs, as the lean lexer's loop inside its window's: the starts before
            // the first rare one (`run`), in the inner loop, which then has only the fused
            // rules' tokens and no rule's call; else the rare start, a token of the rules'
            // general tail, which comes back here; none left, the refill.
            if let (Some(mid), Some(rw)) = (mid, rare_w) {
                f.switch_to_block(mid);
                heads.push((mid, Step::Token));
                let mp = f.block_params(mid).to_vec();
                let (cand, st) = (mp[0], &mp[1..]);
                let rare = f.ins().band(cand, rw);
                let below = f.ins().iadd_imm(rare, -1);
                let below = f.ins().band_not(below, rare);
                let run = f.ins().band(cand, below);
                let rest = f.ins().bxor(cand, run);
                // The run's end (`run_done`, with the starts after the run and the state): the
                // rare start found here, as the lean lexer's `if rare == 0 { break }` after its
                // run; none, the refill. `mid` comes to it with no run.
                let (go_run, run_done, go_rare) = (f.create_block(), f.create_block(), f.create_block());
                for _ in 0..1 + STATE_WORDS {
                    f.append_block_param(run_done, types::I64);
                }
                let now: Vec<BlockArg> = mp.iter().map(|&v| v.into()).collect();
                f.ins().brif(run, go_run, &[], run_done, &now);
                f.switch_to_block(go_run);
                let tz = f.ins().ctz(run);
                let s0 = f.ins().iadd(q64, tz);
                let args: Vec<BlockArg> = [s0, run].iter().chain(st).map(|&v| v.into()).collect();
                f.ins().jump(from_word, &args);
                f.switch_to_block(run_done);
                let dp = f.block_params(run_done).to_vec();
                let (left, st) = (dp[0], &dp[1..]);
                // The block's gap marks' words to the refill (`carried`), the others none.
                let carried: Vec<Value> = runs.iter().zip(&words).map(|(&k, &w)| if folded && lift.iter().any(|&(_, mo)| mo == k) { w } else { zero }).collect();
                let refill_args: Vec<BlockArg> = st.iter().copied().chain(std::iter::once(zero)).chain(carried).map(BlockArg::from).collect();
                f.ins().brif(rare, go_rare, &[], head, &refill_args);
                f.switch_to_block(go_rare);
                let tz = f.ins().ctz(rare);
                let s0 = f.ins().iadd(q64, tz);
                let args: Vec<BlockArg> = [s0, left].iter().chain(st).map(|&v| v.into()).collect();
                f.ins().jump(rare_word.expect("with mid"), &args);
                runs_loop = Some((mid, rest, run_done));
                // The rare head: the state and the block's words as the run loop's, its token
                // block its own.
                let rw = rare_word.expect("with mid");
                f.switch_to_block(rw);
                let rp = f.block_params(rw).to_vec();
                let token_r = f.create_block();
                f.append_block_param(token_r, types::I64);
                let hr: Vec<Value> = rp[2..].iter().copied().chain(std::iter::once(rp[1])).chain(words.iter().copied()).collect();
                f.ins().jump(token_r, &[rp[0].into()]);
                rare_part = Some((token_r, hr, BlockRegs { cand: rp[1], runs: runs.iter().copied().zip(words.iter().copied()).collect(), base: q64, terms: term_words.clone() }));
            }
            // The start: a token, or past the file.
            f.switch_to_block(from_word);
            heads.push((from_word, Step::Token));
            inner = Some(from_word);
            let fw = f.block_params(from_word).to_vec();
            // Every start is inside the file (the refill's `valid`).
            let s = fw[0];
            regs = Some(BlockRegs { cand: fw[1], runs: runs.iter().copied().zip(words.iter().copied()).collect(), base: q64, terms: term_words.clone() });
            // The token's state, the inner loop's: what the code after reads as the head's.
            h = fw[2..].iter().copied().chain(std::iter::once(fw[1])).chain(words.iter().copied()).collect();
            (pending_in, from_in) = (h[3], h[4]);
            f.ins().jump(token, &[s.into()]);
        }
        None => {
            let end = c.cursor(f, n);
            let more = f.ins().icmp(IntCC::UnsignedLessThan, h[0], end);
            f.ins().brif(more, token, &[], finish, &[h[2].into(), end.into(), h[3].into(), h[4].into()]);
        }
    }

    // Waiting: the state as the head has it, for the next part, with the bits the refill took
    // from the carried words (the next part's head carries none).
    if let Some(wait) = wait {
        f.switch_to_block(wait);
        f.set_cold_block(wait);
        let k = c.index(f, head_h[0]);
        let mut st: Vec<Value> = head_h[1..STATE_WORDS].to_vec();
        if let Some(a) = carried_bits {
            st[2] = f.ins().bor(st[2], a);
        }
        let args: Vec<BlockArg> = std::iter::once(k).chain(st).map(BlockArg::from).collect();
        f.ins().jump(then, &args);
    }

    // The token's body, once a head: the run loop's and the rare starts', where they are two
    // (`Side`), as the lean lexer's run loop and its `match` on a rare token's first byte.
    let mut parts = vec![(token, h.clone(), regs.clone(), pending_in, from_in, if rare_part.is_some() { Side::Fused } else { Side::Both })];
    if let Some((tr, hr, gr)) = rare_part.take() {
        let (p_in, f_in) = (hr[3], hr[4]);
        parts.push((tr, hr, Some(gr), p_in, f_in, Side::Rare(None)));
    }
    for (token, h, regs, pending_in, from_in, side) in parts {
    // A token: its first and last bytes, its kind and its word's slot.
    f.switch_to_block(token);
    let (k, after, bad, pending) = (h[0], h[1], h[2], pending_in);
    let (sa, so) = c.at(f, crate::records::FIELD_START, k);
    let (la, lo_off) = c.at(f, crate::records::FIELD_LAST, k);
    // The loop's start, written with the token's last and code at its end (`write_record`),
    // as the lean lexer writes its `Tok` once; else the extract's, read.
    let (s, start_kept) = if walked.is_some() {
        (f.block_params(token)[0], true)
    } else {
        (f.ins().uload32(fl, sa, so), false)
    };
    // The token's first eight bytes and its first byte's entry (`RecordTables::token`): its
    // kind, rule, word flag and pair row, one load, as the lean lexer's `TOKEN`.
    let from = f.ins().iadd(c.bytes, s);
    let raw = f.ins().load(types::I64, fl, from, 0);
    let first = f.ins().band_imm(raw, 0xff);
    // With the codes in the entries (`punct_in_entry`), the pair's entry where the byte starts
    // one: the pair word's bit, as the lean lexer's `TOKEN[c | pair << 8]`.
    let pair_reg = regs.as_ref().and_then(|g| g.runs.iter().find(|&&(k, _)| k == PAIR_WORD)).map(|&(_, v)| v);
    let by_pair = t.punct_in_entry.and(pair_reg).filter(|_| !(matches!(side, Side::Rare(_)) && t.rare_unpaired));
    let first = match by_pair {
        Some(p) => {
            let sh = f.ins().band_imm(s, 63);
            let bit = f.ins().ushr(p, sh);
            let bit = f.ins().band_imm(bit, 1);
            let bit = f.ins().ishl_imm(bit, 8);
            f.ins().bor(first, bit)
        }
        None => first,
    };
    let ea = f.ins().ishl_imm(first, 3);
    let ea = f.ins().iadd(tables, ea);
    let entry = f.ins().load(types::I64, ro, ea, t.token as i32);
    // A rare start's rules, as the lean lexer's `match` arms: by the entry's bit, those whose
    // tokens end plainly (`TOKEN_PLAIN_END`) and the others, each class its rules' table and
    // its own tail, so that a plain token's has no byte put back.
    // The rules' exits (`rule_last`): every rule's, and the fused rules' of their own, whose
    // tail is made again knowing their tokens are no reader's and no extra (the lean lexer's
    // run loop, apart from its rare tokens). A rare start's, one a class of how its tokens end
    // (`TokenEnd`), each with its own tail, as the lean lexer's `match` arms: a plain token's
    // puts no byte back.
    let (exits, fused_exit) = if ruled {
        let mode = if rd.calls.iter().any(|x| x.0 & !0xff == RULES_CALL || x.0 & !0xff == LONG_CALL) { RuleMode::Calls } else { RuleMode::Inline };
        let (exits, fx) = rule_last(f, r, t, &rd, c, s, raw, (entry, by_pair.is_none()), limit, regs.as_ref(), mode, side, heads);
        (exits.into_iter().map(|(x, d)| (x, Some(d))).collect(), fx)
    } else {
        (vec![(side, None)], None)
    };
    let classes = exits.len();
    for (at, (side, done)) in exits.into_iter().enumerate() {
    let general = done.map(|d| exit_values(f, d));
    // A rare arm whose first bytes share one entry (`rare_entry`): that entry, a constant its
    // tail's tests fold, as the lean lexer's arm knows its kind and whether it is a word.
    let entry = match side {
        Side::Rare(Some(x)) if by_pair.is_none() => match rare_entry(r, t, x) {
            Some(e) => {
                c.known_entries.set(c.known_entries.get() + 1);
                f.ins().iconst(types::I64, e as i64)
            }
            None => entry,
        },
        _ => entry,
    };
    let fused_exit = fused_exit.filter(|_| at + 1 == classes);
    for common in std::iter::once(false).chain(fused_exit.map(|_| true)) {
    let (l, refused, retag, hits) = if common {
        let fb = fused_exit.expect("the fused exit");
        f.switch_to_block(fb);
        let p = f.block_params(fb).to_vec();
        (p[0], Some(p[1]), None, p[3])
    } else {
        match general {
            Some((l, refuse, tag, h)) => (l, Some(refuse), Some(tag), h),
            None => (f.ins().uload32(fl, la, lo_off), None, None, zero),
        }
    };
    // The rules' last, written with the code (`write_record`).
    let last_kept = ruled.then_some(l);
    let (long, coded) = (f.create_block(), f.create_block());
    for _ in 0..STATE_WORDS + 2 {
        f.append_block_param(coded, types::I64);
    }
    // Out of order (a last before its start, a start inside the token before): a difference
    // below zero. The columns are 32 bits wide, so a difference's sign is its 64th bit.
    // With the loop's starts (`records::walk`) neither can be: a start is past the token
    // before (the starts below it cleared), and every rule's last is at its start at least.
    // Then only a rule's own refusal is left.
    let wrong = if walked.is_some() {
        refused.unwrap_or(zero)
    } else {
        let before = f.ins().isub(l, s);
        let inside = f.ins().isub(s, after);
        let wrong = f.ins().bor(before, inside);
        match refused {
            Some(x) => f.ins().bor(wrong, x),
            None => wrong,
        }
    };
    // A refusal is rare: a branch out of the loop, to its end (`finish`) with the refusal set,
    // which sets the file's flag (`record_end`), as the lean lexer writes its `BAD` token and
    // leaves its loop (`break 'read`). The file goes to the fallback: nothing after it is read.
    let (refuse, on) = (f.create_block(), f.create_block());
    let no = f.ins().icmp_imm(IntCC::SignedLessThan, wrong, 0);
    f.ins().brif(no, refuse, &[], on, &[]);
    f.switch_to_block(refuse);
    f.set_cold_block(refuse);
    let all = f.ins().iconst(types::I64, -1);
    f.ins().jump(finish, &[all.into(), k.into(), pending.into(), from_in.into()]);
    f.switch_to_block(on);
    // The loop goes on past the token's first byte at least, whatever its rule found: a
    // search that found no last gives `s - 1`, below zero at `s = 0`, so the max is signed.
    // A fused rule's last is at its start at least (`rule_last`): no max.
    // A rare class whose rule never gives a last below its start (`last_below_start`): no
    // max, as the lean lexer's arms go on from the token's end.
    let floored = matches!(side, Side::Rare(Some(x)) if x.rule < r.rules.len() && !last_below_start(r, x.rule));
    let next_after = if walked.is_some() && (common || floored) {
        f.ins().iadd_imm(l, 1)
    } else if walked.is_some() {
        let past = f.ins().smax(l, s);
        f.ins().iadd_imm(past, 1)
    } else {
        f.ins().iadd_imm(l, 1)
    };
    let len = f.ins().isub(next_after, s);
    let kind = f.ins().band_imm(entry, 0xff);
    // A reader's token's kind and marks (`Reader::retag`), where it gives them.
    // None in a rare class of no tagging reader's rules (`Side::Rare`).
    let retag = retag.filter(|_| r.readers.iter().any(|rd| rd.retag.is_some()) && !matches!(side, Side::Rare(Some(RareClass { tagged: false, .. }))));
    // A punctuation of the fused pair rule (`TOKEN_PUNCT`): its code bits by its first byte
    // and its length, a load beside the entry's, no word looked up, as the lean lexer's.
    if common {
        let (punct, rest) = (f.create_block(), f.create_block());
        let is_p = f.ins().band_imm(entry, 1 << TOKEN_PUNCT);
        f.ins().brif(is_p, punct, &[], rest, &[]);
        f.switch_to_block(punct);
        let bits = if by_pair.is_some() {
            // The entry is the token's own: its pair's where it is two bytes.
            let x = f.ins().ushr_imm(entry, i64::from(TOKEN_PUNCT_ONE));
            f.ins().ishl_imm(x, i64::from(crate::records::KIND_BITS))
        } else {
            let two = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, len, 1);
            let pc = f.ins().load(types::I64, ro, ea, t.punct as i32);
            let hi = f.ins().ushr_imm(pc, 32);
            let lo = f.ins().band_imm(pc, 0xffff_ffff);
            f.ins().select(two, hi, lo)
        };
        let code = f.ins().bor(kind, bits);
        f.ins().jump(coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), code.into(), hits.into()]);
        f.switch_to_block(rest);
    }
    // A token a reader says (`Reader::retag`, a comment): its tag is its code, its kind and
    // marks, and it is no word (`records::pass`), as the lean lexer's comment branch.
    let mut known_tail: Option<(cranelift_codegen::ir::Block, (i64, i64))> = None;
    if let Some(tag) = retag {
        let rest = f.create_block();
        let has = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, tag, 0);
        // A rare class whose tagging rules give one tag, and tests that hold at each of their
        // tokens (`tag_known`): a token tagged is one of theirs, its tag and hits the
        // compiler's, as the lean lexer's comment arm knows what it read. Its tail its own
        // (`known_tail`), as that arm writes its own token: the constants reach the marks'
        // merge and the code's store, not a join with the class's other tokens.
        let fixed = match side {
            Side::Rare(Some(RareClass { tagged: true, rule, .. })) if rule < r.rules.len() => tag_known(r, rule).map(|(tc, hc)| (tc, i64::from(hc))),
            _ => None,
        };
        let target = match fixed {
            Some(v) => {
                let own = f.create_block();
                for _ in 0..STATE_WORDS + 2 {
                    f.append_block_param(own, types::I64);
                }
                known_tail = Some((own, v));
                own
            }
            None => coded,
        };
        f.ins().brif(has, target, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), tag.into(), hits.into()], rest, &[]);
        f.switch_to_block(rest);
    }
    // A rare class none of whose first bytes starts a word (`TOKEN_WORDY`, by the bytes the
    // rare table sends to its rules): no word to look up, its code the kind, as the lean
    // lexer's `/` arm writes its punctuation.
    let class_bytes: Vec<u8> = match side {
        Side::Rare(Some(x)) => rare_bytes(r, t, x),
        _ => Vec::new(),
    };
    let entry_of = |b: u8| t.words[t.token / 8 + usize::from(b)];
    let no_words = !class_bytes.is_empty() && class_bytes.iter().all(|&b| entry_of(b) >> TOKEN_WORDY & 1 == 0);
    // A rare class whose tokens not a reader's are the fused pair rule's (`TOKEN_PUNCT`: `/`
    // alone, the comment reader's other): their code bits by the first byte and the length,
    // no word looked up, as the lean lexer's `/` arm writes its punctuation. A tagged class's
    // alone, where a token not tagged is its reader's other rule's.
    // A class of a pair rule (`EndRule::Next`, the `/` alone a reader goes on to) is that too:
    // its tokens are its pairs', as the lean lexer's `b'/' => tok!(SLASH)`.
    let pair_class = matches!(side, Side::Rare(Some(RareClass { rule, .. })) if matches!(r.rules.get(rule), Some(crate::records::EndRule::Next(_))));
    let punct = (matches!(side, Side::Rare(Some(RareClass { tagged: true, .. }))) || pair_class) && !class_bytes.is_empty() && class_bytes.iter().all(|&b| entry_of(b) >> TOKEN_PUNCT & 1 == 1);
    if punct {
        let two = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, len, 1);
        let pc = f.ins().load(types::I64, ro, ea, t.punct as i32);
        let hi = f.ins().ushr_imm(pc, 32);
        let lo = f.ins().band_imm(pc, 0xffff_ffff);
        let bits = f.ins().select(two, hi, lo);
        let code = f.ins().bor(kind, bits);
        f.ins().jump(coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), code.into(), hits.into()]);
    } else if no_words {
        f.ins().jump(coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), kind.into(), hits.into()]);
    } else {
    // A first byte no word starts: no word to look up, its code the kind.
    let looked = f.create_block();
    let wordy = f.ins().band_imm(entry, 1 << TOKEN_WORDY);
    f.ins().brif(wordy, looked, &[], coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), kind.into(), hits.into()]);
    f.switch_to_block(looked);
    let key = cut_to(f, raw, len);
    let h1 = f.ins().bxor(key, len);
    let h1 = f.ins().imul(h1, c.mult);
    let slot = f.ins().ushr_imm(h1, i64::from(64 - t.table.bits));
    // The slot's key and meta, side by side: one pair load.
    let off = f.ins().ishl_imm(slot, if t.rest == WordRest::Second { 5 } else { 4 });
    let sa = f.ins().iadd(c.slots, off);
    let kk = f.ins().load(types::I64, ro, sa, 0);
    let meta = f.ins().load(types::I64, ro, sa, 8);
    let same_key = f.ins().icmp(IntCC::Equal, kk, key);
    // The slot's length and ninth byte are bytes of its meta, each loaded alone (lean's
    // `ldrb`s): a byte load is as soon as the meta's, a field cut from it an instruction later.
    let mlen = f.ins().uload8(types::I64, ro, sa, 8);
    let same_len = f.ins().icmp(IntCC::Equal, mlen, len);
    let mut hit = f.ins().band(same_key, same_len);
    if t.rest == WordRest::Ninth {
        // Its ninth byte, where it has nine, as the slot's (lean's `(len != 9) | (ninth ==
        // b'n')`): read past the token where it is shorter, in the padding at the file's end.
        let ninth = f.ins().uload8(types::I64, fl, from, 8);
        let want = f.ins().uload8(types::I64, ro, sa, 8 + (NINTH_SHIFT / 8) as i32);
        let same = f.ins().icmp(IntCC::Equal, ninth, want);
        let other = f.ins().icmp_imm(IntCC::NotEqual, len, 9);
        let ok = f.ins().bor(same, other);
        hit = f.ins().band(hit, ok);
    }
    if t.rest == WordRest::Second {
        // Its second eight bytes, as the slot's: read where it has them (else its first again,
        // in bounds as `raw`), none for a token of eight bytes at most. Straight-line, as the
        // lean lexer's ninth byte.
        let long = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, len, 8);
        let past = f.ins().iadd_imm(from, 8);
        let at = f.ins().select(long, past, from);
        let chunk = f.ins().load(types::I64, fl, at, 0);
        let rest = f.ins().iadd_imm(len, -8);
        let chunk = cut_to(f, chunk, rest);
        let chunk = f.ins().select(long, chunk, zero);
        let want = f.ins().load(types::I64, ro, sa, 16);
        let same = f.ins().icmp(IntCC::Equal, chunk, want);
        hit = f.ins().band(hit, same);
    }
    // The word's code bits (its word and the marks of it alone, `RecordTables`), on a hit; not
    // the ninth byte above them.
    let w = f.ins().ushr_imm(meta, 8);
    let w = if t.rest == WordRest::Ninth { f.ins().band_imm(w, (1i64 << (NINTH_SHIFT - 8)) - 1) } else { w };
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
    // A hit on a word past eight bytes: the meta's top bit, in the bits taken (`LONG_WORD`).
    if t.rest != WordRest::Tails {
        f.ins().jump(coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), code.into(), hits.into()]);
    } else {
        let check = f.ins().ushr_imm(bits, 55);
        f.ins().brif(check, long, &[], coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), code.into(), hits.into()]);
    }

    if t.rest == WordRest::Tails {
        // A word past eight bytes: its other words, compared (seven at most, unrolled).
        f.switch_to_block(long);
        f.set_cold_block(long);
        let word = word.unwrap_or_else(|| word_of(&mut *f));
        let wi = f.ins().iadd_imm(word, -1);
        let toff = f.ins().imul_imm(wi, 56);
        let tail = f.ins().iadd(tables, toff);
        // Out of the loop where it has the call (`tail_fn`); else straight-line here.
        let all = match rd.calls.iter().find(|x| x.0 == TAIL_CALL) {
            Some((_, id, sig)) => {
                let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
                let signature = f.import_signature(sig.clone());
                let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
                let call = f.ins().call(callee, &[from, len, tail]);
                f.inst_results(call)[0]
            }
            None => {
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
                all
            }
        };
        let code2 = f.ins().select(all, code, no_word);
        f.ins().jump(coded, &[k.into(), next_after.into(), bad.into(), pending.into(), from_in.into(), h[5].into(), code2.into(), hits.into()]);
    }
    }

    // The token's tail: the class's (`coded`), and a known tag's own (`known_tail`), its code
    // and hits the compiler's.
    for (tail, fixed) in std::iter::once((coded, None)).chain(known_tail.map(|(b, v)| (b, Some(v)))) {
    f.switch_to_block(tail);
    let mut cv = f.block_params(tail).to_vec();
    if let Some((tc, hc)) = fixed {
        cv[STATE_WORDS] = f.ins().iconst(types::I64, tc);
        cv[STATE_WORDS + 1] = f.ins().iconst(types::I64, hc);
    }
    let (mut code, mut pending) = (cv[STATE_WORDS], cv[3]);
    let (mut from, mut word_before) = (cv[4], cv[5]);
    let base = f.ins().band_imm(s, !63);
    let in_regs = regs.as_ref().map(|g| (g, base));
    // The marks at a token's first byte the loop sets (`loop_marks`): its word's bit there. On
    // the fused rules' path, none whose first bytes they never have (an extra's, `fused_rules`).
    for &(m, o) in at_marks.iter().filter(|&&(_, o)| !common || !fused_ks.is_some_and(|(kn, kr)| never_at(r, rd.bytes.map(|(b, _)| b.native), o, [kn, kr]))) {
        let w = regs.as_ref().and_then(|g| g.runs.iter().find(|&&(k, _)| k == o)).map(|&(_, v)| v).expect("a mark's word in the registers");
        let sh = f.ins().band_imm(s, 63);
        let bit = f.ins().ushr(w, sh);
        let bit = f.ins().band_imm(bit, 1);
        let placed = f.ins().ishl_imm(bit, i64::from(crate::records::MARK_SHIFT) + i64::from(m));
        code = f.ins().bor(code, placed);
    }
    // The marks' tests' bits and the gap marks' (`records::gap_marks`): a grammar token has its
    // own `At` bits, those the tokens before left, and the gap marks' bits from the byte after
    // the grammar token before's first to its own, but where the word before says none
    // (`unless after`); an extra's token leaves all of its own to the next.
    let tests = r.checks.iter().any(|c| c.then != crate::records::CheckThen::Escape);
    if tests || !lift.is_empty() {
        let h = cv[STATE_WORDS + 1];
        // `unless after`: of the gap marks the loop sets alone. The word before is only kept
        // where one is.
        let unless: Vec<(u8, u16)> = r.unless.iter().copied().filter(|&(k, _)| lift.iter().any(|&(m, _)| m == k)).collect();
        let gapped = !lift.is_empty();
        let kept_word = gapped && !unless.is_empty();
        let extra = r.extra.filter(|_| !common);
        // The word before, as this token leaves it: its word, but an extra's leaves it.
        let next_word = |f: &mut FunctionBuilder<'_>, code: Value, word_before: Value| {
            let word = f.ins().ushr_imm(code, i64::from(crate::records::KIND_BITS));
            let word = f.ins().band_imm(word, (1i64 << (crate::records::MARK_SHIFT - crate::records::KIND_BITS)) - 1);
            match extra {
                Some(e) => {
                    let is_extra = f.ins().ushr_imm(code, i64::from(crate::records::MARK_SHIFT) + i64::from(e));
                    let is_extra = f.ins().band_imm(is_extra, 1);
                    f.ins().select(is_extra, word_before, word)
                }
                None => word,
            }
        };
        // Most tokens have no mark from the tokens before, no test's hit and no gap mark's
        // bit in their block (the refill folds a block's bits past the last grammar token,
        // and every passed block's, into `pending`): the bits are all zero, the code is the
        // token's, and the state goes on untouched, as the lean lexer's run loop has no
        // marks. Else the merge below, cold.
        let quiet = regs.is_some() && (!gapped || folded);
        let join = if quiet {
            let any = f.ins().bor(pending, h);
            let (merge, join) = (f.create_block(), f.create_block());
            for _ in 0..4 {
                f.append_block_param(join, types::I64);
            }
            let wb = if kept_word { next_word(&mut *f, code, word_before) } else { word_before };
            f.ins().brif(any, merge, &[], join, &[code.into(), pending.into(), from.into(), wb.into()]);
            f.switch_to_block(merge);
            f.set_cold_block(merge);
            Some(join)
        } else {
            None
        };
        let at_bits = f.ins().band_imm(h, (1i64 << crate::records::AFTER_SHIFT) - 1);
        let after_bits = f.ins().ushr_imm(h, i64::from(crate::records::AFTER_SHIFT));
        // The block's flag (`GAP_FLAG`) stays for its tokens after this one; no mark's bit.
        let flag = if folded { f.ins().band_imm(pending, GAP_FLAG) } else { zero };
        let pending_marks = if folded { f.ins().band_imm(pending, !GAP_FLAG) } else { pending };
        let after_bits_kept = if folded { f.ins().bor(after_bits, flag) } else { after_bits };
        let mut mine = f.ins().bor(pending_marks, at_bits);
        if gapped {
            // Folded, the bits not yet taken start past the token before's first byte, in the
            // token's block (the refill took those before it).
            let from_eff = if folded {
                let fr = c.after_last_start(f, cv[0]);
                f.ins().umax(fr, base)
            } else {
                from
            };
            let bits = marks_in(f, &rd, c, &lift, from_eff, s, in_regs, folded, heads);
            mine = f.ins().bor(mine, bits);
        }
        let held = if kept_word {
            let mut none = zero;
            for &(k, w) in &unless {
                let same = f.ins().icmp_imm(IntCC::Equal, word_before, i64::from(w));
                let same = f.ins().uextend(types::I64, same);
                let bit = f.ins().ishl_imm(same, i64::from(k));
                none = f.ins().bor(none, bit);
            }
            f.ins().band_not(mine, none)
        } else {
            mine
        };
        let placed = f.ins().ishl_imm(held, i64::from(crate::records::MARK_SHIFT));
        let marked = f.ins().bor(code, placed);
        if kept_word {
            word_before = next_word(&mut *f, code, word_before);
        }
        match extra {
            Some(e) => {
                let is_extra = f.ins().ushr_imm(code, i64::from(crate::records::MARK_SHIFT) + i64::from(e));
                let is_extra = f.ins().band_imm(is_extra, 1);
                let left = f.ins().bor(mine, after_bits_kept);
                code = f.ins().select(is_extra, code, marked);
                pending = f.ins().select(is_extra, left, after_bits_kept);
            }
            None => {
                code = marked;
                pending = after_bits_kept;
            }
        }
        if gapped && !folded {
            from = f.ins().iadd_imm(s, 1);
        }
        if let Some(join) = join {
            f.ins().jump(join, &[code.into(), pending.into(), from.into(), word_before.into()]);
            f.switch_to_block(join);
            let p = f.block_params(join).to_vec();
            (code, pending, from, word_before) = (p[0], p[1], p[2], p[3]);
        }
    }
    write_record(f, c, stores, cv[0], start_kept.then_some(s), last_kept, code);
    let k1 = c.step(f, cv[0]);
    let mut back: Vec<BlockArg> = vec![k1.into(), cv[1].into(), cv[2].into(), pending.into(), from.into(), word_before.into()];
    // The block's starts after the token, while it ends in the block; else none, for a refill.
    // The marks so far flagged where the refill reads this block again (`REREAD_FLAG`).
    let mut reread: Option<Value> = None;
    if let Some(g) = &regs {
        let left = if let (true, Some(_)) = (common, lean) {
            // A fused rule's token covers no start after its own (`lean_starts`): its bit
            // cleared alone, as the lean lexer's `run &= run - 1`. One past its block leaves
            // none, for a refill.
            let less = f.ins().iadd_imm(g.cand, -1);
            f.ins().band(g.cand, less)
        } else {
            let ones = f.ins().iconst(types::I64, -1);
            let sh = f.ins().band_imm(cv[1], 63);
            let m = f.ins().ishl(ones, sh);
            // With the runs (`runs_loop`), the starts left are the run's and those after it; a
            // rare start's are those after the run already (`run_done`'s).
            let cand = match runs_loop {
                Some((_, rest, _)) if !matches!(side, Side::Rare(_)) => f.ins().bor(g.cand, rest),
                _ => g.cand,
            };
            let mut left = f.ins().band(cand, m);
            // A rare token's class (`Side::Rare`, `TokenEnd`): one that ends plainly, the starts
            // past it, as the lean lexer's after its string's arm; else the bytes put back, as
            // its number's arm reads the word after it.
            let form = match side {
                Side::Rare(Some(x)) => x.end,
                _ => TokenEnd::Any,
            };
            if let (Some((wi, _)), TokenEnd::Word) = (lean.map(|(wi, pi, _)| (wi, pi)), form) {
                // Its last byte a run byte (`word_end`): the byte after it put back where it is
                // one, as the lean lexer's number arm reads the word after it.
                let w = g.runs[wi].1;
                let one = f.ins().iconst(types::I64, 1);
                let bit = f.ins().ishl(one, sh);
                let back = f.ins().band(w, bit);
                left = f.ins().bor(left, back);
            }
            if let (Some((wi, pi, chained)), TokenEnd::Any) = (lean, form) {
                // Another rule's token may end where the starts left out the byte after it (a
                // run's byte after a run's, a pair's second): the byte put back, a start.
                let (w, p) = (g.runs[wi].1, g.runs[pi].1);
                let wl = f.ins().ishl_imm(w, 1);
                let inner = f.ins().band(w, wl);
                let pl = f.ins().ishl_imm(p, 1);
                let left_out = f.ins().bor(inner, pl);
                let one = f.ins().iconst(types::I64, 1);
                let bit = f.ins().ishl(one, sh);
                let back = f.ins().band(left_out, bit);
                let put = f.ins().bor(left, back);
                left = if chained {
                    // A run of pairs across the token's end: their order is read again from the
                    // byte after it, by a refill of this block (`REREAD_FLAG`).
                    let across = f.ins().band(pl, bit);
                    if skips {
                        let flagged = f.ins().bor_imm(pending, REREAD_FLAG);
                        reread = Some(f.ins().select(across, flagged, pending));
                    }
                    f.ins().select(across, zero, put)
                } else {
                    put
                };
            }
            let qa = f.ins().ushr_imm(cv[1], 6);
            let qs = f.ins().ushr_imm(s, 6);
            let same = f.ins().icmp(IntCC::Equal, qa, qs);
            f.ins().select(same, left, zero)
        };
        if let Some(p) = reread {
            back[3] = p.into();
        }
        if let (Some(inner), Some((mid, rest, run_done))) = (inner, runs_loop) {
            // A fused token: the run's next start, in the inner loop, as the lean lexer's
            // `while run != 0`; the run done, the starts after it (`mid`). Another rule's token:
            // the starts left, by `mid`, out of the inner loop.
            let after: Vec<BlockArg> = std::iter::once(BlockArg::from(if common && lean.is_some() { rest } else { left })).chain(back.iter().cloned()).collect();
            if common && lean.is_some() {
                // The block's base (`BlockRegs::base`): starts are left only where the cursor is
                // in the token's block.
                let base = g.base;
                let tz = f.ins().ctz(left);
                let s0 = f.ins().iadd(base, tz);
                let on: Vec<BlockArg> = [s0, left].iter().copied().map(BlockArg::from).chain(back.iter().cloned()).collect();
                f.ins().brif(left, inner, &on, run_done, &after);
            } else {
                f.ins().jump(mid, &after);
            }
        } else if let Some(inner) = inner {
            // Starts left in the block: the next, in the inner loop, as the lean lexer's
            // `while run != 0`; none, the refill (the head, its words unread). From the block's
            // base (`BlockRegs::base`), not the token's end.
            let base = g.base;
            let tz = f.ins().ctz(left);
            let s0 = f.ins().iadd(base, tz);
            let on: Vec<BlockArg> = [s0, left].iter().copied().map(BlockArg::from).chain(back.iter().cloned()).collect();
            back.push(zero.into());
            // The block's gap marks' words to the refill (`carried`), the others none.
            back.extend(g.runs.iter().map(|&(k, w)| BlockArg::from(if folded && lift.iter().any(|&(_, mo)| mo == k) { w } else { zero })));
            f.ins().brif(left, inner, &on, head, &back);
        } else {
            back.push(left.into());
            back.extend(g.runs.iter().map(|&(_, v)| BlockArg::from(v)));
            f.ins().jump(head, &back);
        }
    } else {
        f.ins().jump(head, &back);
    }
    }
    }
    }
    }

    // The file's end: a refusal sets the totals' flag, the count after the extracts'. A bit
    // left of an `else escape` mark (`Records::unplaced`) has no grammar token: it refuses.
    f.switch_to_block(finish);
    let bad = f.block_params(finish)[0];
    let n = c.index(f, f.block_params(finish)[1]);
    let unplaced_lift: Vec<(u8, u16)> = lift.iter().copied().filter(|&(m, _)| r.unplaced >> m & 1 == 1).collect();
    let bad = if r.unplaced != 0 && (r.checks.iter().any(|c| c.then != crate::records::CheckThen::Escape) || !unplaced_lift.is_empty()) {
        let mut rest = f.block_params(finish)[2];
        if !unplaced_lift.is_empty() {
            // The gap marks' bits after the last grammar token's first byte.
            let from = if folded { c.after_last_start(f, f.block_params(finish)[1]) } else { f.block_params(finish)[3] };
            let end = f.ins().iadd_imm(c.len, -1);
            let bits = marks_in(f, &rd, c, &unplaced_lift, from, end, None, false, heads);
            rest = f.ins().bor(rest, bits);
        }
        let left = f.ins().band_imm(rest, i64::from(r.unplaced));
        let left = f.ins().icmp_imm(IntCC::NotEqual, left, 0);
        let left = f.ins().sextend(types::I64, left);
        let left = f.ins().ineg(left);
        f.ins().bor(bad, left)
    } else {
        bad
    };
    // A part at the file's end: its count and refusal stored, for the end.
    if part.is_some() {
        // The loop is past the file's last byte: a part after this one finds no token.
        f.ins().jump(then, &[n.into(), c.len.into(), bad.into(), zero.into(), c.len.into(), zero.into()]);
        return;
    }
    record_end(f, r, extracts, outputs, c, bad, n, differ, 0, then, heads);
}

/// A file's records' end (`record_tokens`): a refusal sets the totals' flag, the count after the
/// extracts'; the loop's counts stored; then the marks of any word (`late_marks`).
#[allow(clippy::too_many_arguments)]
fn record_end(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, extracts: usize, outputs: usize, c: &FileCols, bad: Value, n: Value, differ: Value, looped: u32, then: cranelift_codegen::ir::Block, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    let fl = MemFlags::trusted();
    let stores = MemFlags::new().with_notrap();
    let (walked, ruled) = (r.skip, !r.rules.is_empty());
    let bad = f.ins().ushr_imm(bad, 63);
    let bad = f.ins().bor(bad, differ);
    let flag = f.ins().load(types::I64, fl, c.counts, 8 * extracts as i32);
    let flag = f.ins().bor(flag, bad);
    f.ins().store(stores, flag, c.counts, 8 * extracts as i32);
    if walked.is_some() {
        f.ins().store(stores, n, c.counts, 8 * i32::from(r.starts));
    }
    if ruled {
        f.ins().store(stores, n, c.counts, 8 * i32::from(r.lasts));
    }
    // The gap marks (decision 0177) after the others: they read the extra's mark.
    if r.gaps == 0 {
        late_marks(f, r, extracts, outputs, c, n, false, looped, then, heads);
    } else {
        let gaps = f.create_block();
        late_marks(f, r, extracts, outputs, c, n, false, looped, gaps, heads);
        f.switch_to_block(gaps);
        late_marks(f, r, extracts, outputs, c, n, true, looped, then, heads);
    }
}

/// The marks of any word (`Records::marks` with no word): rare bits, so set after the tokens,
/// not tested at each. Over the file's rows, the marks' words ORed; at each bit, its token by a
/// binary search over the starts from the token before; where a token starts there, each
/// mark's bit ORed into its code. Then on to `then`.
///
/// The gap marks (`lift`, `Records::gaps`): each bit to the first grammar token that starts at
/// it or after (an extra's token, by its mark, passed over), as `records::gap_marks`; a bit
/// with none, of a mark with `else escape` (`Records::unplaced`), sets the totals' flag.
#[allow(clippy::too_many_arguments)]
fn late_marks(f: &mut FunctionBuilder<'_>, r: &crate::records::Records, extracts: usize, outputs: usize, c: &FileCols, n: Value, lift: bool, looped: u32, then: cranelift_codegen::ir::Block, heads: &mut Vec<(cranelift_codegen::ir::Block, Step)>) {
    // The marks a reader gives (`Reader::retag`) are its: no mask is read for them.
    let retagged: u32 = r.readers.iter().filter_map(|rd| rd.retag).fold(0, |a, (_, m)| a | m);
    let late: Vec<(usize, u16)> = r.marks.iter().enumerate().filter(|&(k, m)| m.0 != crate::records::WORD_ALONE && m.1 == 0 && (r.gaps >> k & 1 == 1) == lift && retagged >> k & 1 == 0 && r.tested_only >> k & 1 == 0 && looped >> k & 1 == 0).map(|(k, m)| (k, m.0)).collect();
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
    // `set`'s: the word of the grammar token before (`Records::unless`; 0 for none).
    f.append_block_param(set, types::I64);
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
    let (ma, mo) = c.field(f, crate::records::FIELD_START, mid);
    let sv = f.ins().uload32(fl, ma, mo);
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
    let (sa, so) = c.field(f, crate::records::FIELD_START, k);
    let sv = f.ins().uload32(fl, sa, so);
    let qa: Vec<BlockArg> = q.iter().map(|&v| v.into()).collect();
    if lift {
        // On past the extras' tokens: the first grammar token from it.
        let (over, test) = (f.create_block(), f.create_block());
        for _ in 0..4 {
            f.append_block_param(over, types::I64);
        }
        heads.push((over, Step::Token));
        let _ = (sv, place, k);
        f.ins().jump(over, &qa);
        f.switch_to_block(over);
        let q = f.block_params(over).to_vec();
        let qa: Vec<BlockArg> = q.iter().map(|&v| v.into()).collect();
        let inside = f.ins().icmp(IntCC::UnsignedLessThan, q[3], n);
        let refusing: Vec<u16> = late.iter().filter(|&&(m, _)| r.unplaced >> m & 1 == 1).map(|&(_, o)| o).collect();
        if refusing.is_empty() {
            f.ins().brif(inside, test, &[], next, &qa);
        } else {
            // No grammar token at or after the bit: its `else escape` marks' bits there refuse.
            let unplaced = f.create_block();
            f.ins().brif(inside, test, &[], unplaced, &[]);
            f.switch_to_block(unplaced);
            let off = f.ins().imul_imm(q[0], 8 * outputs as i64);
            let at = f.ins().iadd(c.out, off);
            let b = f.ins().ctz(q[2]);
            let mut any = None;
            for o in refusing {
                let w = f.ins().load(types::I64, fl, at, 8 * i32::from(o));
                any = Some(match any {
                    None => w,
                    Some(a) => f.ins().bor(a, w),
                });
            }
            let x = f.ins().ushr(any.expect("a mark"), b);
            let x = f.ins().band_imm(x, 1);
            let flag = f.ins().load(types::I64, fl, c.counts, 8 * extracts as i32);
            let flag = f.ins().bor(flag, x);
            f.ins().store(stores, flag, c.counts, 8 * extracts as i32);
            f.ins().jump(next, &qa);
        }
        f.switch_to_block(test);
        let (ca, co) = c.field(f, crate::records::FIELD_CODE, q[3]);
        let code = f.ins().uload32(fl, ca, co);
        let e = i64::from(crate::records::MARK_SHIFT) + i64::from(r.extra.unwrap_or(0));
        let is_extra = f.ins().ushr_imm(code, e);
        let is_extra = f.ins().band_imm(is_extra, i64::from(r.extra.is_some()));
        let k1 = f.ins().iadd_imm(q[3], 1);
        if r.unless.is_empty() {
            let mut sa = qa.clone();
            sa.push(zero.into());
            f.ins().brif(is_extra, over, &[q[0].into(), q[1].into(), q[2].into(), k1.into()], set, &sa);
        } else {
            // Back over the extras' tokens before it to the grammar token before, for its word.
            let (back, prior) = (f.create_block(), f.create_block());
            for _ in 0..5 {
                f.append_block_param(back, types::I64);
                f.append_block_param(prior, types::I64);
            }
            heads.push((back, Step::Token));
            let mut ba = qa.clone();
            ba.push(q[3].into());
            f.ins().brif(is_extra, over, &[q[0].into(), q[1].into(), q[2].into(), k1.into()], back, &ba);
            // (block, cursor, bits, token, j): the grammar token before is the last before `j`.
            f.switch_to_block(back);
            let b = f.block_params(back).to_vec();
            let ba: Vec<BlockArg> = b.iter().map(|&v| v.into()).collect();
            let mut none = ba[..4].to_vec();
            none.push(zero.into());
            f.ins().brif(b[4], prior, &ba, set, &none);
            f.switch_to_block(prior);
            let b = f.block_params(prior).to_vec();
            let j1 = f.ins().iadd_imm(b[4], -1);
            let (ca, co) = c.field(f, crate::records::FIELD_CODE, j1);
            let code = f.ins().uload32(fl, ca, co);
            let is_extra = f.ins().ushr_imm(code, e);
            let is_extra = f.ins().band_imm(is_extra, i64::from(r.extra.is_some()));
            let word = f.ins().ushr_imm(code, i64::from(crate::records::KIND_BITS));
            let word = f.ins().band_imm(word, 0xff);
            let mut again: Vec<BlockArg> = b[..4].iter().map(|&v| v.into()).collect();
            again.push(j1.into());
            let mut done: Vec<BlockArg> = b[..4].iter().map(|&v| v.into()).collect();
            done.push(word.into());
            f.ins().brif(is_extra, back, &again, set, &done);
        }
    } else {
        let at_it = f.ins().icmp(IntCC::Equal, sv, place);
        let hit = f.ins().band(inside, at_it);
        let mut sa = qa.clone();
        sa.push(zero.into());
        f.ins().brif(hit, set, &sa, next, &qa);
    }

    f.switch_to_block(set);
    let q = f.block_params(set).to_vec();
    let off = f.ins().imul_imm(q[0], 8 * outputs as i64);
    let at = f.ins().iadd(c.out, off);
    let b = f.ins().ctz(q[2]);
    let (ca, co) = c.field(f, crate::records::FIELD_CODE, q[3]);
    let mut code = f.ins().uload32(fl, ca, co);
    for &(m, o) in &late {
        let w = f.ins().load(types::I64, fl, at, 8 * i32::from(o));
        let x = f.ins().ushr(w, b);
        let mut x = f.ins().band_imm(x, 1);
        // `unless after`: none after a grammar token of these words.
        for &(_, word) in r.unless.iter().filter(|&&(u, _)| usize::from(u) == m) {
            let other = f.ins().icmp_imm(IntCC::NotEqual, q[4], i64::from(word));
            let other = f.ins().uextend(types::I64, other);
            x = f.ins().band(x, other);
        }
        let x = f.ins().ishl_imm(x, i64::from(crate::records::MARK_SHIFT) + m as i64);
        code = f.ins().bor(code, x);
    }
    f.ins().istore32(stores, code, ca, co);
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
    /// The outputs whose rows are stored: not those the record pass makes (decision 0177,
    /// `BlockNative::records_made`), which no one reads from the rows on the CPU.
    stored: &'a [bool],
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
    let sunk: Vec<bool> = early.iter().zip(lp.stored).map(|(&e, &s)| e && s).collect();
    let sinks = slot_of.is_empty().then_some(Sinks { dst, stride: lp.stride, early: &sunk });
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
    for k in (0..native.outputs.len()).filter(|&k| slot_of.is_empty() && !early[k] && lp.stored[k]) {
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
        // The lasts by the end rules, the starts by the loop (decision 0177): the record pass
        // writes them.
        if c < native.extracts.len() && native.records_made().contains(&k) {
            counts_out.push(n);
            continue;
        }
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
fn forward(context: &mut Context, general_context: &mut Context, outlined: (cranelift_module::FuncId, cranelift_codegen::ir::Signature), module: &mut JITModule, name: &str, native: &BlockNative, slot_of: &[u16], slots: u16, hot: &mut Vec<cranelift_codegen::ir::Block>, records: Option<(&crate::records::Records, &RecordTables, usize)>, made: &BlockNative) -> Result<(bool, usize, usize, Vec<(cranelift_module::FuncId, cranelift_codegen::ir::Function)>, Vec<(usize, usize)>, usize, usize, usize, usize), Error> {
    // The outputs the token loop makes from the bytes (`native_cpu_bytes`), from the program
    // before its CPU form.
    let mut bytewise = crate::native_cpu_bytes::Bytewise::of(made);
    bytewise.before_start = made.records.as_ref().and_then(|r| before_start(r, made));
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
    // The records the token loop reads back (`PassShape::reads_back`).
    let mut reads_back = 0;
    let (mut arms, mut known_entries) = (0, 0);
    // On the CPU, no row of an output the record pass makes (`records_made`).
    let unmade = if records.is_some() && made.outputs.len() == native.outputs.len() { made.records_made() } else { Vec::new() };
    let stored: Vec<bool> = (0..native.outputs.len()).map(|k| !unmade.contains(&k)).collect();
    let lp = Loop { native, state: &state, counted: &counted, early: &early, stored: &stored, stride: 8 * native.outputs.len() as i64, slot_of, slots, params, walks: Some(&walks) };
    general(general_context, &lp)?;
    let own = cranelift_codegen::ir::UserExternalName::new(0, outlined.0.as_u32());
    // Each gated region's function, declared here and built where the body lowers it.
    let signature = region_signature(module);
    let ids = (0..state.plan.regions.len())
        .map(|r| module.declare_function(&format!("{name}_region{r}"), Linkage::Local, &signature).map_err(|_| Error::BackendFailure))
        .collect::<Result<Vec<_>, _>>()?;
    let n_regions = state.plan.regions.len();
    // The file's records: a function of its own (`record_fn`), called once a file; or, where
    // the loop finds the starts (decision 0177), a part after each block and the file's end.
    let part_calls = match records {
        Some((rec, rt, outputs)) if rec.skip.is_some() => {
            let sig = words_signature(module, 1);
            let id = module.declare_function(&format!("{name}_records_end"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
            // The marks the token loop sets itself (`loop_marks`): the end sets the others.
            let (lift, at) = loop_marks(rec, &|o| bytewise.can(o));
            let looped = lift.iter().chain(&at).fold(0u32, |a, &(m, _)| a | 1 << m);
            end_fn(&mut func, rec, native.extracts.len(), outputs, looped, rt.layout)?;
            vec![(id, sig, func)]
        }
        _ => Vec::new(),
    };
    // Each `escaped`'s carry past a window's first byte, a function of its own (`carry_fn`):
    // cold, out of the loops that read it.
    let mut carry_calls = Vec::new();
    if records.is_some() && !part_calls.is_empty() {
        let plain = crate::native_cpu_bytes::Bytewise::of(made);
        for a in plain.escaped_runs() {
            let mut sig = words_signature(module, 3);
            sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
            let id = module.declare_function(&format!("{name}_carry{a}"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
            carry_fn(&mut func, &plain, a)?;
            carry_calls.push((id, func));
            bytewise.carries.push((a, id, sig));
        }
    }
    let carry_names: Vec<cranelift_codegen::ir::UserExternalName> = carry_calls.iter().map(|(id, _)| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect();
    // Each output the token loop makes from the bytes but the skip, a window function of its
    // own (`bytes_fn`); the skip's is the refill's, inline.
    let mut byte_calls = Vec::new();
    if let Some((rec, _, _)) = records.filter(|_| !part_calls.is_empty()) {
        for o in made.read_at_tokens().into_iter().map(|o| o as u16).filter(|&o| Some(o) != rec.skip) {
            let mut sig = words_signature(module, 3);
            sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
            let id = module.declare_function(&format!("{name}_bytes{o}"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
            bytes_fn(&mut func, &bytewise, o, &carry_names)?;
            byte_calls.push((o, id, sig, func));
        }
        // Each rule's count tests, fused (`tests_fn`), where the bytes make all their masks.
        for (k, _) in rec.rules.iter().enumerate() {
            let tests: Vec<crate::records::TokenCheck> = rec.checks.iter().filter(|c| usize::from(c.rule) == k && c.count && !c.always()).copied().collect();
            if tests.is_empty() || !tests.iter().all(|c| bytewise.can(c.o)) {
                continue;
            }
            let mut sig = words_signature(module, 4);
            sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
            let id = module.declare_function(&format!("{name}_tests{k}"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
            tests_fn(&mut func, &bytewise, &tests, &carry_names)?;
            byte_calls.push((TESTS_CALL | k as u16, id, sig, func));
        }
        // Each reader's arm with its rule's count tests, fused (`read_fn`): one pass over a
        // string's bytes for its end and its tests.
        for (k, rule) in rec.rules.iter().enumerate() {
            let crate::records::EndRule::Read(i) = *rule else { continue };
            let tests: Vec<crate::records::TokenCheck> = rec.checks.iter().filter(|c| usize::from(c.rule) == k && c.count && !c.always()).copied().collect();
            for (j, a) in rec.readers[usize::from(i)].arms.iter().enumerate() {
                let Some(counts) = byte_calls.iter().find(|x| x.0 == TESTS_CALL | k as u16).map(|x| (x.0, x.1, x.2.clone())) else { continue };
                if !read_fused(&bytewise, a, &tests) || j >= 8 {
                    continue;
                }
                let mut sig = words_signature(module, 3);
                sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
                sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
                let id = module.declare_function(&format!("{name}_read{k}_{j}"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
                let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
                read_fn(&mut func, &bytewise, a, &tests, &counts, &carry_names)?;
                byte_calls.push((READ_CALL | (k as u16) << 3 | j as u16, id, sig, func));
            }
        }
        // The long words' tail compare (`tail_fn`), out of the loop; none where each slot has
        // what its word has past eight bytes (`RecordTables::rest`).
        if let Some((_, t, _)) = records.filter(|(_, t, _)| t.rest == WordRest::Tails) {
            let mut sig = words_signature(module, 3);
            sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
            let id = module.declare_function(&format!("{name}_tail"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
            tail_fn(&mut func, t.tails)?;
            byte_calls.push((TAIL_CALL, id, sig, func));
        }
        // Each rule a function of its own (`rule_fn`), where its reads are the bytes': the
        // loop calls those of its rare tokens (`RuleMode::Calls`).
        let ids: Vec<ByteCall> = byte_calls.iter().map(|(o, id, sig, _)| (*o, *id, sig.clone())).collect();
        let t = records.map(|(_, t, _)| t).expect("records");
        // A pair rule with no tests (`EndRule::Next`: a row load, the next byte's bit) is
        // straight-line: read in its arm, no function, as the lean lexer reads a pair from its
        // `TOKEN` table in the arm. A function is for a rule that reads on past a window.
        let straight = |k: usize| matches!(rec.rules[k], crate::records::EndRule::Next(_)) && rec.checks.iter().all(|c| usize::from(c.rule) != k);
        for k in (0..rec.rules.len()).filter(|&k| k < 0xff && rule_bytewise(rec, &bytewise, k) && !straight(k)) {
            // Its signature, the arguments it reads (`rule_reads`): built with all of them, then
            // again with those.
            let empty = module.make_signature();
            let sig_of = |n: usize| {
                let mut sig = empty.clone();
                for _ in 0..n {
                    sig.params.push(cranelift_codegen::ir::AbiParam::new(types::I64));
                }
                for _ in 0..if packs(rec) { 2 } else { 4 } {
                    sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
                }
                sig
            };
            let built = |sig: &cranelift_codegen::ir::Signature, mode: RuleMode, bw: &crate::native_cpu_bytes::Bytewise<'_>, calls: &[ByteCall], isa: &dyn cranelift_codegen::isa::TargetIsa| -> Result<cranelift_codegen::ir::Function, Error> {
                let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, u32::MAX), sig.clone());
                rule_fn(&mut func, rec, t, bw, made.outputs.len(), mode, calls, &carry_names)?;
                let n = rule_reads(&func, isa);
                if n == sig.params.len() {
                    return Ok(func);
                }
                let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, u32::MAX), sig_of(n));
                rule_fn(&mut func, rec, t, bw, made.outputs.len(), mode, calls, &carry_names)?;
                Ok(func)
            };
            let mut func = built(&sig_of(RULE_ARGS), RuleMode::Only(k), &bytewise, &ids, module.isa())?;
            let sig = func.signature.clone();
            let id = module.declare_function(&format!("{name}_rule{k}"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            func.name = cranelift_codegen::ir::UserFuncName::user(0, id.as_u32());
            // A run's or a reader's short form (`RuleMode::Short`) is the loop's arm (`rule_last`'s
            // `arm`), no function of its own, where it calls nothing but its whole one: the loop
            // calls the whole one alone, past the short form, as the lean lexer's arm calls
            // `find_pair`. Built here with that call only to see so.
            let mut arm = false;
            if matches!(rec.rules[k], crate::records::EndRule::Run(_) | crate::records::EndRule::Read(_)) {
                let mut with = ids.clone();
                with.push((LONG_CALL | k as u16, id, sig.clone()));
                let mut short_bw = bytewise.clone();
                short_bw.bail = Some((id, sig.clone()));
                let sfunc = built(&sig_of(RULE_ARGS), RuleMode::Short(k), &short_bw, &with, module.isa())?;
                let made = calls_of(&sfunc);
                arm = !made.is_empty() && made.iter().all(|&x| x == id.as_u32());
            }
            byte_calls.push((if arm { LONG_CALL } else { RULES_CALL } | k as u16, id, sig, func));
        }
    }
    let byte_ids: Vec<ByteCall> = byte_calls.iter().map(|(o, id, sig, _)| (*o, *id, sig.clone())).collect();
    let record_call = match records {
        Some(_) if !part_calls.is_empty() => None,
        Some((rec, t, outputs)) => {
            let sig = record_signature(module);
            let id = module.declare_function(&format!("{name}_records"), Linkage::Local, &sig).map_err(|_| Error::BackendFailure)?;
            let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), sig.clone());
            record_fn(&mut func, rec, t, native.extracts.len(), outputs)?;
            Some((id, sig, func))
        }
        None => None,
    };
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
        // The token loop after each block (decision 0177), in this loop as the lean lexer's:
        // `tokens` runs it over the blocks whose outputs are final, `after` goes on to the next
        // block, `row_end` calls the file's end. Its state goes round the block loop.
        let tokened = records.is_some() && !part_calls.is_empty();
        // The last pass's body makes nothing but constants, and stores none of them.
        let bodiless = tokened && bodiless_body(native, slot_of, true);
        let (tokens, after, row_end) = (f.create_block(), f.create_block(), f.create_block());
        let states = if tokened { STATE_WORDS } else { 0 };
        f.append_block_params_for_function_params(entry);
        f.append_block_param(row, types::I64);
        // The carried state, packed (decision 0070): each carry holds only its bits, in a field
        // of a few loop-carried words, never a word per bit. Each loop block's parameters
        // (`Loop::params`): every load and store is `[pointer, #offset]`. Then the token loop's.
        for b in [body_block, general, head, finish, tokens] {
            for _ in 0..params + states {
                f.append_block_param(b, types::I64);
            }
        }
        f.append_block_param(tokens, types::I64);
        for b in [after, row_end] {
            for _ in 0..states {
                f.append_block_param(b, types::I64);
            }
        }

        f.switch_to_block(entry);
        let entry_params = f.block_params(entry).to_vec();
        // The pass's arguments and the row's index, in a slot: read where a row's work needs
        // them, before and after its blocks. Held through the block loop, which reads none of
        // them, they took its registers (or were spilled and reloaded in it).
        let outer = f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * 8, 3));
        for (k, &v) in entry_params.iter().enumerate() {
            f.ins().stack_store(v, outer, 8 * k as i32);
        }
        let reload = |f: &mut FunctionBuilder<'_>, k: i32| f.ins().stack_load(types::I64, outer, 8 * k);
        const ROW_INDEX: i32 = 7;
        let nrows = entry_params[2];
        let zero = f.ins().iconst(types::I64, 0);
        // The constants every block uses, made once. (Others are made where used, decision 0069.)
        let consts = Consts::make(&mut f);
        let no_rows = f.ins().icmp_imm(IntCC::Equal, nrows, 0);
        f.ins().brif(no_rows, done, &[], row, &[zero.into()]);

        // A row: the file's blocks, length, outputs, positions and counts.
        f.switch_to_block(row);
        let r = f.block_params(row)[0];
        f.ins().stack_store(r, outer, 8 * ROW_INDEX);
        let (arena, rows, out_all, pos_all, scratch_all) = (reload(&mut f, 0), reload(&mut f, 1), reload(&mut f, 3), reload(&mut f, 4), reload(&mut f, 6));
        let at = f.ins().imul_imm(r, std::mem::size_of::<Row>() as i64);
        let at = f.ins().iadd(rows, at);
        let flags = MemFlags::trusted().with_readonly();
        let field = |f: &mut FunctionBuilder<'_>, k: i32| f.ins().load(types::I64, flags, at, 8 * k);
        let (start, blocks, len, first_word) = (field(&mut f, 0), field(&mut f, 1), field(&mut f, 2), field(&mut f, 3));
        let (first_position, capacity) = (field(&mut f, 4), field(&mut f, 5));
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
            .chain(std::iter::repeat_n(zero.into(), states))
            .collect();
        // The row's record slot (`REC_SLOT`): its columns, and the token loop's state at zero.
        let rec_slot = (!part_calls.is_empty()).then(|| f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * REC_SLOT as u32, 3)));
        let callee_of = |f: &mut FunctionBuilder<'_>, k: usize| {
            let (id, sig, _) = &part_calls[k];
            let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
            let signature = f.import_signature(sig.clone());
            f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true })
        };
        let parts = rec_slot.map(|slot| (slot, callee_of(&mut f, 0)));
        if let (Some((slot, _)), Some((rec, t, _))) = (parts, records) {
            let counts_all = reload(&mut f, 5);
            let cols = file_cols(&mut f, rec, t, native.extracts.len(), r, rows, arena, out_all, pos_all, counts_all);
            for (k, v) in cols.values().into_iter().enumerate() {
                f.ins().stack_store(v, slot, 8 * k as i32);
            }
            let zero = f.ins().iconst(types::I64, 0);
            for k in 0..STATE_WORDS {
                f.ins().stack_store(zero, slot, 8 * (RECORD_PARAMS + k) as i32);
            }
        }
        // After a block (`next` the block after it), its parameters and the token loop's: the
        // blocks before it are final (an output's delayed bits go into the row before).
        let to_tokens = |f: &mut FunctionBuilder<'_>, block: &[BlockArg], st: &[Value]| -> Vec<BlockArg> {
            let next = match block[0] {
                BlockArg::Value(v) => v,
                _ => unreachable!("a value"),
            };
            let ready = f.ins().iadd_imm(next, -1);
            block.iter().copied().chain(st.iter().map(|&v| v.into())).chain([ready.into()]).collect()
        };
        // Blocks 1 .. len / 64 are full.
        let full_end = f.ins().ushr_imm(len, 6);
        // A body that makes nothing (every output the token loop's, from the bytes): no block
        // loop. The file goes to its end at once, and the token loop runs over it whole, as the
        // lean lexer's one loop.
        if bodiless {
            f.ins().jump(finish, &first);
        } else {
            f.ins().brif(none, finish, &first, head, &first);
        }

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
        let st = p[params..].to_vec();
        let again;
        (again, gate_branches, round_branches) = step(&mut f, &lp, &consts, &p[..params], len, true, scratch, Some(&calls), &mut heads);
        let again: Vec<BlockArg> = again.into_iter().map(BlockArg::from).collect();
        // The hot loop's back edge: to itself while the next block is full.
        hot.push(body_block);
        hot.extend((made_from..f.func.dfg.num_blocks()).map(|k| cranelift_codegen::ir::Block::from_u32(k as u32)));
        let next = match again[0] {
            BlockArg::Value(v) => v,
            _ => unreachable!("a value"),
        };
        if tokened {
            let args = to_tokens(&mut f, &again, &st);
            f.ins().jump(tokens, &args);
        } else {
            let more = f.ins().icmp(IntCC::UnsignedLessThan, next, full_end);
            f.ins().brif(more, body_block, &again, head, &again);
        }

        // The other blocks: the general body's call, the parameters through a stack slot.
        f.switch_to_block(general);
        let p = f.block_params(general).to_vec();
        let st = p[params..].to_vec();
        let slot = f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * params as u32, 3));
        for (k, &v) in p[..params].iter().enumerate() {
            f.ins().stack_store(v, slot, 8 * k as i32);
        }
        let pointer = f.func.dfg.value_type(arena);
        let address = f.ins().stack_addr(pointer, slot, 0);
        let name = f.func.declare_imported_user_function(own.clone());
        let signature = f.import_signature(outlined.1.clone());
        let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
        f.ins().call(callee, &[address, len]);
        let back: Vec<BlockArg> = (0..params).map(|k| BlockArg::from(f.ins().stack_load(types::I64, slot, 8 * k as i32))).collect();
        if tokened {
            let args = to_tokens(&mut f, &back, &st);
            f.ins().jump(tokens, &args);
        } else {
            f.ins().jump(head, &back);
        }

        // The token loop over the final blocks, from the file's columns in the record slot;
        // then the next block, or at the file's end (every block final) its end.
        if let (Some((slot, end)), Some((rec, t, outputs))) = (parts, records) {
            f.switch_to_block(tokens);
            let tp = f.block_params(tokens).to_vec();
            let vals: Vec<Value> = (0..RECORD_PARAMS).map(|k| f.ins().stack_load(types::I64, slot, 8 * k as i32)).collect();
            let cols = FileCols::of_tables(&mut f, &vals, t);
            let ready = tp[params + STATE_WORDS];
            let init: [Value; STATE_WORDS] = std::array::from_fn(|k| tp[params + k]);
            record_tokens(&mut f, rec, t, native.extracts.len(), outputs, &cols, Some(RecPart { ready, init }), Some((&bytewise, &consts, &byte_ids)), after, &mut heads);
            reads_back += cols.reads_back.get();
            arms += cols.arms.get();
            known_entries += cols.known_entries.get();
            f.switch_to_block(after);
            let st = f.block_params(after).to_vec();
            let st_args: Vec<BlockArg> = st.iter().map(|&v| v.into()).collect();
            if bodiless {
                // Every block final at once: the one part is the file's.
                f.ins().jump(row_end, &st_args);
            } else {
                let last = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, ready, cols.blocks);
                let go = f.create_block();
                f.ins().brif(last, row_end, &st_args, go, &[]);
                f.switch_to_block(go);
                let args: Vec<BlockArg> = tp[..params].iter().chain(&st).map(|&v| v.into()).collect();
                let more = f.ins().icmp(IntCC::UnsignedLessThan, tp[0], full_end);
                f.ins().brif(more, body_block, &args, head, &args);
            }

            // The file's end: the loop's count and refusal for the end's function.
            f.switch_to_block(row_end);
            let st = f.block_params(row_end).to_vec();
            f.ins().stack_store(st[0], slot, 8 * RECORD_PARAMS as i32);
            f.ins().stack_store(st[2], slot, 8 * (RECORD_PARAMS + 2) as i32);
            let pointer = f.func.dfg.value_type(st[0]);
            let address = f.ins().stack_addr(pointer, slot, 0);
            f.ins().call(end, &[address]);
            f.ins().jump(exit, &[]);
        }

        // The row's end: each delayed output's last word, and the counts.
        let stores = MemFlags::new().with_notrap();
        f.switch_to_block(finish);
        let p = f.block_params(finish).to_vec();
        let dst = p[2];
        for &k in delayed.iter().filter(|&&k| slot_of.is_empty() && stored[k]) {
            // The last block's word, as it stored it: shifted down, the stream's last.
            let at = 8 * k as i32 - lp.stride as i32;
            let last = f.ins().load(types::I64, MemFlags::trusted(), dst, at);
            let down = f.ins().ushr_imm(last, i64::from(native.output_delays[k]));
            f.ins().store(stores, down, dst, at);
        }
        // The row's places again, from the slot: none is held through its blocks.
        let r = reload(&mut f, ROW_INDEX);
        let (rows, pos_all, counts_all) = (reload(&mut f, 1), reload(&mut f, 4), reload(&mut f, 5));
        let at = f.ins().imul_imm(r, std::mem::size_of::<Row>() as i64);
        let at = f.ins().iadd(rows, at);
        let flags = MemFlags::trusted().with_readonly();
        let (first_position, capacity, first_count) = (f.ins().load(types::I64, flags, at, 32), f.ins().load(types::I64, flags, at, 40), f.ins().load(types::I64, flags, at, 48));
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
        // The token loop's last part, every block final, and its end.
        if tokened {
            let blocks = f.ins().load(types::I64, MemFlags::trusted().with_readonly(), at, 8);
            let args: Vec<BlockArg> = p.iter().map(|&v| v.into()).chain([blocks.into()]).collect();
            f.ins().jump(tokens, &args);
        }
        if let (Some((rec, t, _)), Some((id, sig, _))) = (records, &record_call) {
            let (arena, out_all) = (reload(&mut f, 0), reload(&mut f, 3));
            let cols = file_cols(&mut f, rec, t, native.extracts.len(), r, rows, arena, out_all, pos_all, counts_all);
            let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
            let signature = f.import_signature(sig.clone());
            let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
            f.ins().call(callee, &cols.values());
        }
        if !tokened {
            f.ins().jump(exit, &[]);
        }

        f.switch_to_block(exit);
        let (r, nrows) = (reload(&mut f, ROW_INDEX), reload(&mut f, 2));
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
        owns.extend(record_call.iter().chain(&part_calls).map(|(id, _, _)| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())));
        owns.extend(byte_ids.iter().map(|(_, id, _)| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())));
        owns.extend(carry_names.iter().cloned());
        shape = crate::native_cpu_shape::check(&context.func, &heads, &owns);
    }
    if round == n_regions || !worth_calling(&context.func, &calls) {
        break;
    }
    }
    shape.map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    // A body that makes nothing has no block loop: its blocks, unreachable, are cut, and the
    // hot loop is the token loop's.
    if bodiless_body(native, slot_of, records.is_some()) {
        context.compute_cfg();
        context.compute_domtree();
        context.eliminate_unreachable_code(module.isa()).map_err(|_| Error::BackendFailure)?;
        hot.clear();
    }
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
    // The records' function, checked where it is built (`record_fn`).
    if let Some((id, _, func)) = record_call {
        regions.push((id, func));
    }
    for (id, _, func) in part_calls {
        regions.push((id, func));
    }
    // The window and carry functions some function calls; the rest are not built (an output
    // the loop reads inline, a carry no window needs).
    let mut extra: Vec<(cranelift_module::FuncId, cranelift_codegen::ir::Function)> = byte_calls.into_iter().map(|(_, id, _, func)| (id, func)).chain(carry_calls).collect();
    let names = calls_of;
    let mut called: std::collections::HashSet<u32> = names(&context.func).into_iter().chain(regions.iter().flat_map(|(_, func)| names(func))).collect();
    loop {
        let more: Vec<u32> = extra.iter().filter(|(id, _)| called.contains(&id.as_u32())).flat_map(|(_, func)| names(func)).filter(|n| !called.contains(n)).collect();
        if more.is_empty() {
            break;
        }
        called.extend(more);
    }
    extra.retain(|(id, _)| called.contains(&id.as_u32()));
    regions.extend(extra);
    // The walks' functions, checked where they are built (`native_cpu_walk::build_walk`).
    for (w, built) in walks.built.into_inner().into_iter().enumerate() {
        if let Some(func) = built {
            regions.push((walks.ids[w], func));
        }
    }
    Ok((ascii_branch, gate_branches, round_branches, regions, kept, inlined, reads_back, arms, known_entries))
}

/// The last pass's body makes nothing but constants and stores none (`forward`): every output
/// is the token loop's, from the bytes, so the pass has no block loop.
fn bodiless_body(native: &BlockNative, slot_of: &[u16], records: bool) -> bool {
    records && slot_of.is_empty() && native.walks.is_empty() && native.insts.iter().all(|i| matches!(i, BlockInst::Const(_)))
}

/// The functions `func` calls (their user names' indexes, a `FuncId` each), by its calls left
/// in its layout in blocks its entry reaches: a name declared but no longer called is not one,
/// nor a call in a block no branch reaches (compiled out).
pub fn calls_of(func: &cranelift_codegen::ir::Function) -> Vec<u32> {
    let mut v = Vec::new();
    let mut cfg = cranelift_codegen::flowgraph::ControlFlowGraph::new();
    cfg.compute(func);
    let mut reached = std::collections::HashSet::new();
    let mut stack: Vec<cranelift_codegen::ir::Block> = func.layout.entry_block().into_iter().collect();
    while let Some(b) = stack.pop() {
        if reached.insert(b) {
            stack.extend(cfg.succ_iter(b));
        }
    }
    for b in func.layout.blocks().filter(|b| reached.contains(b)) {
        for inst in func.layout.block_insts(b) {
            if let cranelift_codegen::ir::InstructionData::Call { func_ref, .. } = func.dfg.insts[inst]
                && let cranelift_codegen::ir::ExternalName::User(r) = func.dfg.ext_funcs[func_ref].name {
                    v.push(func.params.user_named_funcs()[r].index);
                }
        }
    }
    v
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

/// The bits a short window's mask has a lane (`nibbles`): four on NEON, whose narrowing shift
/// (`shrn`) makes a 16-lane compare a 64-bit mask in one instruction, as simdjson's arm64
/// `to_bitmask` for 16 bytes; one elsewhere (a movemask). The compiler's choice by the target.
pub const SHORT_STRIDE: u32 = if cfg!(target_arch = "aarch64") { 4 } else { 1 };

/// One bit a lane of a mask of `SHORT_STRIDE` bits a lane: its lowest each.
pub const fn lane_bits(stride: u32) -> i64 {
    match stride {
        4 => 0x1111_1111_1111_1111,
        _ => -1,
    }
}

/// A 16-byte compare result (each byte 0 or 0xff) as a mask of `SHORT_STRIDE` bits a lane:
/// on NEON its 16-bit halves shifted right by four and narrowed (`shrn #4`), lane `i` the
/// nibble at bit `4 i`; `clif_rust` writes it `vshrn_n_u16`. Else the 16 bits of `bitmask`.
pub fn short_mask(f: &mut FunctionBuilder<'_>, part: Value, weights: Value) -> Value {
    if SHORT_STRIDE != 4 {
        return bitmask(f, &[part], weights);
    }
    let le = MemFlags::new().with_endianness(Endianness::Little);
    let h = f.ins().bitcast(types::I16X8, le, part);
    let s = f.ins().ushr_imm(h, 4);
    let ff = f.ins().iconst(types::I16, 0xff);
    let ff = f.ins().splat(types::I16X8, ff);
    // At most 0xff a lane: the narrowing's saturation never clips.
    let k = f.ins().band(s, ff);
    let n = f.ins().uunarrow(k, k);
    let lanes = f.ins().bitcast(types::I64X2, le, n);
    f.ins().extractlane(lanes, 0)
}

/// The mask of a short window's first `n` lanes (`n` at most `most`, at most 16), `stride`
/// bits a lane: `n * stride` low bits, all 64 at 16 lanes of four.
pub fn lanes_below(f: &mut FunctionBuilder<'_>, n: Value, stride: u32, most: u32) -> Value {
    let one = f.ins().iconst(types::I64, 1);
    let bits = if stride > 1 { f.ins().ishl_imm(n, i64::from(stride.trailing_zeros())) } else { n };
    if stride * most < 64 {
        let b = f.ins().ishl(one, bits);
        return f.ins().iadd_imm(b, -1);
    }
    let b = f.ins().ishl(one, bits);
    let b = f.ins().iadd_imm(b, -1);
    let all = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, bits, 64);
    let ones = f.ins().iconst(types::I64, -1);
    f.ins().select(all, ones, b)
}

/// A short window's lanes from its first to its first unset one: `ctz(!m)` in lanes.
pub fn lanes_set(f: &mut FunctionBuilder<'_>, m: Value, stride: u32) -> Value {
    let y = f.ins().bnot(m);
    let z = f.ins().ctz(y);
    if stride > 1 { f.ins().ushr_imm(z, i64::from(stride.trailing_zeros())) } else { z }
}

/// Four 16-byte compare results (each byte 0 or 0xff) as one 64-bit mask.
pub fn bitmask(f: &mut FunctionBuilder<'_>, parts: &[Value], weights: Value) -> Value {
    if cfg!(target_arch = "aarch64") && parts.len() == 1 {
        // One vector's 16 bits: weight, three pairwise adds, its low two bytes.
        let w = f.ins().band(parts[0], weights);
        let s = f.ins().iadd_pairwise(w, w);
        let s = f.ins().iadd_pairwise(s, s);
        let s = f.ins().iadd_pairwise(s, s);
        let lanes = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(Endianness::Little), s);
        let x = f.ins().extractlane(lanes, 0);
        f.ins().band_imm(x, 0xffff)
    } else if cfg!(target_arch = "aarch64") {
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
