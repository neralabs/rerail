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

use super::Error;
use super::block::{BatchFile, BlockRun, padded_by};
use super::native_cpu_lanes::{Lane, Lanes, Plan, all_ascii};
use super::native_cpu_shape::Step;
use super::native_ir::{BlockInst, BlockNative};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, BlockArg, ConstantData, Endianness, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

/// The batch's bytes, its rows (`Row`), their count, the mask words, the positions, the counts.
type Entry = unsafe extern "C" fn(*const u8, *const Row, u64, *mut u64, *mut u32, *mut u64);

/// A file of a batch, for the function: where its blocks start in the bytes (`FRONT` zero bytes
/// before them), how many there are, its length, the word at which its mask words start, where
/// its first extract's positions start and each extract's room, and where its counts start (one
/// per counted output: `CompiledBlock::counted`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    pub(super) bytes: u64,
    blocks: u64,
    pub(super) len: u64,
    out: u64,
    pub(super) positions: u64,
    pub(super) capacity: u64,
    pub(super) counts: u64,
}

/// A compiled block program.
pub(super) struct CompiledBlock {
    _module: JITModule,
    entry: Entry,
    outputs: usize,
    delay: u8,
    pub(super) extracts: Vec<usize>,
    totals: Vec<usize>,
    /// The outputs with a running count, extracts' and totals' together.
    counted: Vec<usize>,
    /// The full body branches once on an all-ASCII block (`native_cpu_lanes`).
    pub(super) ascii_branch: bool,
    /// The gated regions' branches in the function, each body's counted (`native_cpu_lanes::Region`).
    pub(super) gate_branches: usize,
    /// The function as Cranelift IR, before code generation.
    pub(super) clif: String,
    /// The machine code, disassembled.
    pub(super) disassembly: String,
}

/// The running counts, as output indexes: each extract's, then each total's. A file's counts are
/// laid out in this order, so tier 2 reads its totals from the count after the extracts'.
pub(super) fn counted(native: &BlockNative) -> Vec<usize> {
    native.extracts.iter().chain(&native.totals).copied().collect()
}

impl CompiledBlock {
    /// Every output's words over `bytes`; bits past the end are zero.
    pub(super) fn run(&self, bytes: &[u8]) -> Vec<Vec<u64>> {
        self.extract(bytes).outputs
    }

    /// The outputs, positions and totals over `bytes`.
    pub(super) fn extract(&self, bytes: &[u8]) -> BlockRun {
        self.run_files(&[bytes]).pop().expect("one file")
    }

    /// `extract` over several files in one call of the function (`Batch`).
    pub(super) fn run_files(&self, files: &[&[u8]]) -> Vec<BlockRun> {
        let mut batch = Batch::new(self, files);
        batch.call(self);
        batch.results(self, files)
    }
}

/// A batch of files laid out for one call: their blocks end to end, each after `FRONT` zero
/// bytes; a row per file; and one buffer each for the mask words, positions and counts. The host
/// lays out and reads back; every word in them is the function's.
pub(super) struct Batch {
    pub(super) bytes: Vec<u8>,
    pub(super) rows: Vec<Row>,
    out: Vec<u64>,
    pub(super) positions: Vec<u32>,
    pub(super) counts: Vec<u64>,
}

impl Batch {
    pub(super) fn new(program: &CompiledBlock, files: &[&[u8]]) -> Self {
        let (mut bytes, mut rows) = (Vec::new(), Vec::with_capacity(files.len()));
        let (mut words, mut positions, mut counts) = (0u64, 0u64, 0u64);
        for file in files {
            let blocks = padded_by(file, usize::from(program.delay));
            bytes.resize(bytes.len() + FRONT, 0);
            // An extract has at most one position per byte; the flattening writes up to seven
            // past the last.
            let capacity = file.len() as u64 + 8;
            words += program.outputs as u64;
            let row = Row { bytes: bytes.len() as u64, blocks: blocks.len() as u64, len: file.len() as u64, out: words, positions, capacity, counts };
            bytes.extend(blocks.iter().flatten());
            words += (program.outputs * blocks.len()) as u64;
            positions += program.extracts.len() as u64 * capacity;
            counts += program.counted.len() as u64;
            rows.push(row);
        }
        // 64 zero bytes after the last file: tier 2 reads a token's bytes a block at a time.
        bytes.resize(bytes.len() + 64, 0);
        Self { bytes, rows, out: vec![0u64; words as usize], positions: vec![0u32; positions as usize], counts: vec![0u64; counts as usize] }
    }

    pub(super) fn call(&mut self, program: &CompiledBlock) {
        // SAFETY: each row's blocks lie in `bytes` after `FRONT` zero bytes; its mask words (a
        // spare block, then each block's) in `out`; each extract's `capacity` positions in
        // `positions`; its counts in `counts`. The function reads and writes nothing else.
        unsafe {
            (program.entry)(
                self.bytes.as_ptr(),
                self.rows.as_ptr(),
                self.rows.len() as u64,
                self.out.as_mut_ptr(),
                self.positions.as_mut_ptr(),
                self.counts.as_mut_ptr(),
            )
        };
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
                let totals = (0..program.totals.len()).map(|t| count(program.extracts.len() + t)).collect();
                BlockRun { outputs, positions, totals }
            })
            .collect()
    }
}

/// Cranelift over a batch (decision 0044): each file's run under its program, in the files'
/// order; one call of each program's function over all of that program's files.
pub(super) fn run_batch(programs: &[&CompiledBlock], files: &[BatchFile<'_>]) -> Vec<BlockRun> {
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
    #[cfg(test)]
    pub(super) fn cpu_seconds(&self, bytes: &[u8], runs: usize) -> f64 {
        self.batch_cpu_seconds(&[bytes], runs)
    }

    /// `cpu_seconds` over a batch of files, in one call.
    #[cfg(test)]
    pub(super) fn batch_cpu_seconds(&self, files: &[&[u8]], runs: usize) -> f64 {
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

/// The thread's CPU time (user + system), in seconds.
#[cfg(test)]
pub(super) fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `t` is a valid timespec to write.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// Zero bytes before the first block: `pair` and `plane_before` load each block one byte back,
/// and the byte before the stream is 0x00.
pub(super) const FRONT: usize = 16;


/// A field of the carried state: bits `pos..pos + width` of loop-carried word `word`. A `top`
/// field keeps the top `width` bits of instruction `at`'s word (for its `prev`s, and a `toggle`'s
/// or `fill`'s own carry, its top bit); a `flag` field an `escaped`'s borrow.
#[derive(Clone, Copy, Debug)]
pub(super) struct Field {
    at: usize,
    width: u8,
    word: usize,
    pos: u8,
}

/// The carried state's layout (decision 0048): its fields, packed first-fit into words.
#[derive(Debug)]
pub(super) struct State {
    fields: Vec<Field>,
    /// The values on the byte vectors (`native_cpu_lanes`).
    pub(super) plan: Plan,
    /// Each gated region's carried fields: per word, the bits its values carry.
    pub(super) region_fields: Vec<Vec<(usize, u64)>>,
    /// Each instruction's top field and flag field, if it has one.
    top: Vec<Option<usize>>,
    flag: Vec<Option<usize>>,
    pub(super) words: usize,
}

impl State {
    pub(super) fn of(native: &BlockNative) -> Self {
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
        let mut wanted: Vec<(usize, u8, bool)> = Vec::new();
        for (at, inst) in native.insts.iter().enumerate() {
            match inst {
                BlockInst::Toggle(_) | BlockInst::Fill(..) => wanted.push((at, back[at].max(1), false)),
                BlockInst::Escaped(_) => {
                    wanted.push((at, 1, true));
                    if back[at] > 0 {
                        wanted.push((at, back[at], false));
                    }
                }
                _ if back[at] > 0 => wanted.push((at, back[at], false)),
                _ => {}
            }
        }
        let mut used: Vec<u8> = Vec::new();
        let (mut fields, mut top, mut flag) = (Vec::new(), vec![None; n], vec![None; n]);
        for (at, width, is_flag) in wanted {
            let word = match used.iter().position(|&u| u + width <= 64) {
                Some(w) => w,
                None => {
                    used.push(0);
                    used.len() - 1
                }
            };
            fields.push(Field { at, width, word, pos: used[word] });
            used[word] += width;
            if is_flag { flag[at] = Some(fields.len() - 1) } else { top[at] = Some(fields.len() - 1) }
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
        State { fields, plan, region_fields, top, flag, words: used.len() }
    }
}

/// The constants every body uses, made once in the function's entry: the nibble mask, the
/// bitmask's weights, and simdjson's even bits.
pub(super) struct Consts {
    nibble: Value,
    weights: Value,
    even: Value,
}

impl Consts {
    pub(super) fn make(f: &mut FunctionBuilder<'_>) -> Self {
        let even = f.ins().iconst(types::I64, super::block::carry::EVEN as i64);
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
pub(super) struct At<'a> {
    pub(super) i: Value,
    pub(super) len: Value,
    pub(super) base: Value,
    pub(super) packed_in: &'a [Value],
    pub(super) full: bool,
    /// The ASCII body: the block and the 16 bytes before it are all below 0x80.
    pub(super) ascii: bool,
    /// The block's byte chunks, when already loaded (for the ASCII test).
    pub(super) bytes: Option<[Option<Value>; 5]>,
}

/// One block's body: every instruction as straight-line bit operations. Gives each output's
/// value (the value itself, at its delay, not yet cut to the stream) and the carried state out.
///
/// In a full block the plan's gated regions (`native_cpu_lanes::Region`) each run behind one
/// branch: when the gate has no bit and the region's carried bits came in zero, its values are
/// zero and none of it is computed. The builder is left in the block after the last region.
pub(super) fn body(f: &mut FunctionBuilder<'_>, native: &BlockNative, state: &State, c: &Consts, at: At<'_>) -> (Vec<Value>, Vec<Value>, usize) {
    let plan = &state.plan;
    let n = native.insts.len();
    let lanes = Lanes::new(&mut *f, native, plan, at.base, at.i, !at.full, at.ascii, at.bytes, c.nibble);
    let mut e = Emit {
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
    };
    (e.live, e.masked) = live(native, state, at.full);
    let regions = if at.full { plan.regions.len() } else { 0 };
    let in_region = |x: usize| if regions > 0 { plan.region_of[x] } else { None };
    let mut done = vec![false; n];
    let mut emitted = vec![false; regions];
    let mut gates = 0;
    loop {
        // Every value outside the regions whose operands are made, in order; then the first
        // region whose operands outside it are made.
        let mut progress = true;
        while progress {
            progress = false;
            for x in 0..n {
                if !done[x] && in_region(x).is_none_or(|r| emitted[r]) && native.insts[x].operands().iter().all(|&o| done[o]) {
                    e.emit(&mut *f, x);
                    done[x] = true;
                    progress = true;
                }
            }
        }
        let ready = (0..regions).find(|&r| {
            let reg = &plan.regions[r];
            !emitted[r] && done[reg.gate] && reg.members.iter().all(|&m| native.insts[m].operands().iter().all(|&o| done[o] || plan.region_of[o] == Some(r)))
        });
        match ready {
            Some(r) => {
                e.region(&mut *f, r);
                gates += 1;
                for &m in &plan.regions[r].members {
                    done[m] = true;
                }
                emitted[r] = true;
            }
            None if done.iter().all(|&d| d) => break,
            None => {
                // A region waiting on another's values after it: dissolved, its values computed
                // in place without its branch. Regions are convex (`native_cpu_lanes::regions`),
                // so this is a region whose gate is made in a later one.
                let r = (0..regions).find(|&r| !emitted[r]).expect("a region left");
                emitted[r] = true;
            }
        }
    }
    let (outputs, packed) = e.finish(&mut *f);
    (outputs, packed, gates)
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
    for x in (0..n).rev() {
        live[x] |= state.top[x].is_some() || state.flag[x].is_some();
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
        let native = self.native;
        let state = self.state;
        let plan = &state.plan;
        let inst = native.insts[at];
        if !self.live[at] {
            self.set(at, Sym::Known(0));
            return;
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
        for x in inst.operands() {
            plain(&mut *f, &self.syms, &mut self.values, x);
        }
        let v = |x: u16| self.values[usize::from(x)].expect("made plain");
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
                let bits = f.ins().iconst(types::I64, super::block::period_bits(k) as i64);
                f.ins().ishl(bits, first)
            }
            BlockInst::Within(d) => within_word(&mut *f, i, len, d),
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
        let gate = plain(&mut *f, &self.syms, &mut self.values, reg.gate);
        let mut any = gate;
        for &(w, m) in &state.region_fields[r] {
            let bits = f.ins().band_imm(self.packed_in[w], m as i64);
            any = f.ins().bor(any, bits);
        }
        let (active, skip, after) = (f.create_block(), f.create_block(), f.create_block());
        for _ in 0..outside.len() + words.len() {
            f.append_block_param(after, types::I64);
        }
        f.ins().brif(any, active, &[], skip, &[]);
        let before = self.clone();
        // The region computed. Its carried bits are made plain, not gathered as constants: the
        // two ways must agree.
        f.switch_to_block(active);
        let consts = std::mem::replace(&mut self.consts, vec![0; state.words]);
        for &m in &reg.members {
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

    fn finish(mut self, f: &mut FunctionBuilder<'_>) -> (Vec<Value>, Vec<Value>) {
        let outputs = self.native.outputs.iter().map(|&v| plain(&mut *f, &self.syms, &mut self.values, usize::from(v))).collect();
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
pub(super) fn within_word(f: &mut FunctionBuilder<'_>, i: Value, len: Value, d: u8) -> Value {
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
pub(super) fn flatten(f: &mut FunctionBuilder<'_>, m: Value, at: Value, ptr: Value) -> (Value, cranelift_codegen::ir::Block) {
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

/// Compile a block program's native IR.
pub(super) fn compile(native: &BlockNative) -> Result<CompiledBlock, Error> {
    // Not optimised: the mid-end's code motion would hoist every vector constant out of the loop,
    // more than the registers hold, and spill them (decision 0047). The body is already
    // straight-line and deduplicated.
    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    for _ in 0..2 {
        signature.params.push(AbiParam::new(pointer));
    }
    signature.params.push(AbiParam::new(types::I64));
    for _ in 0..3 {
        signature.params.push(AbiParam::new(pointer));
    }
    let id = module
        .declare_function("tempo_block", Linkage::Local, &signature)
        .map_err(|_| Error::BackendFailure)?;
    let mut context: Context = module.make_context();
    context.func.signature = signature;
    context.set_disasm(true);
    let mut builder_context = FunctionBuilderContext::new();
    let counted = counted(native);
    // The outputs with a delay: each carries its word shifted down, for the store of the next.
    let delayed: Vec<usize> = (0..native.outputs.len()).filter(|&k| native.output_delays[k] > 0).collect();
    let shape;
    let ascii_branch;
    let mut gate_branches = 0;
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        // entry -> row (one per file) -> head -> full or general (one per block) -> head ...
        // -> finish -> exit -> row or done. A full block lies inside the file and is not the
        // first: every `within` is all ones, so its body (the hot loop, block2) has no
        // end-of-file arithmetic. The first block and the last ones take the general body.
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
        // The carried state, packed (decision 0048): each carry holds only its bits, in a field
        // of a few loop-carried words, never a word per bit.
        let state = State::of(native);
        ascii_branch = state.plan.ascii_branch;
        let carried = state.words;
        // Each loop block's parameters: the block index, the byte and output pointers (so that
        // every load and store is `[pointer, #offset]`), the state words, the running counts,
        // and the delayed outputs' held words.
        let params = 3 + carried + counted.len() + delayed.len();
        for b in [body_block, general, head, finish] {
            for _ in 0..params {
                f.append_block_param(b, types::I64);
            }
        }

        f.switch_to_block(entry);
        let entry_params = f.block_params(entry).to_vec();
        let (arena, rows, nrows, out_all, pos_all, counts_all) =
            (entry_params[0], entry_params[1], entry_params[2], entry_params[3], entry_params[4], entry_params[5]);
        let zero = f.ins().iconst(types::I64, 0);
        // The constants every block uses, made once. (Others are made where used, decision 0047.)
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
        let first: Vec<BlockArg> = [zero.into(), bytes.into(), out.into()]
            .into_iter()
            .chain(std::iter::repeat_n(zero.into(), params - 3))
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

        let stride = 8 * native.outputs.len() as i64;
        let stores = MemFlags::new().with_notrap();
        // The loops' progress points: the files, the blocks, and each flattening's set bits.
        let mut heads = vec![(row, Step::File), (head, Step::Block), (body_block, Step::Block)];
        for (b, full) in [(body_block, true), (general, false)] {
            f.switch_to_block(b);
            let p = f.block_params(b).to_vec();
            let (i, base, dst) = (p[0], p[1], p[2]);
            let packed_in = &p[3..3 + carried];
            let counts_in = &p[3 + carried..3 + carried + counted.len()];
            let held_in = &p[3 + carried + counted.len()..];
            // The full body's one branch: an all-ASCII block runs a body with the values known on
            // ASCII bytes folded (`native_cpu_lanes`), the others the whole body. Both meet before
            // the sinks.
            let (values, packed_out) = if full && state.plan.ascii_branch {
                let (ascii, bytes) = all_ascii(&mut f, native, &state.plan, base);
                let (fast, slow, merge) = (f.create_block(), f.create_block(), f.create_block());
                let n = native.outputs.len();
                for _ in 0..n + carried {
                    f.append_block_param(merge, types::I64);
                }
                f.ins().brif(ascii, fast, &[], slow, &[]);
                for (blk, ascii) in [(fast, true), (slow, false)] {
                    f.switch_to_block(blk);
                    let (v, packed, gates) = body(&mut f, native, &state, &consts, At { i, len, base, packed_in, full, ascii, bytes: Some(bytes) });
                    gate_branches += gates;
                    let args: Vec<BlockArg> = v.into_iter().chain(packed).map(BlockArg::from).collect();
                    f.ins().jump(merge, &args);
                }
                f.switch_to_block(merge);
                let m = f.block_params(merge).to_vec();
                (m[..n].to_vec(), m[n..].to_vec())
            } else {
                let (v, packed, gates) = body(&mut f, native, &state, &consts, At { i, len, base, packed_in, full, ascii: false, bytes: None });
                gate_branches += gates;
                (v, packed)
            };
            // Each output cut to the stream outside full blocks: the bits `d <= 64 i + j < len + d`.
            let words: Vec<Value> = values
                .iter()
                .enumerate()
                .map(|(k, &v)| if full { v } else {
                    let inside = within_word(&mut f, i, len, native.output_delays[k]);
                    f.ins().band(v, inside)
                })
                .collect();
            // The mask stores: a delayed output's word before this one, from the held word.
            let mut held_out = Vec::with_capacity(delayed.len());
            for k in 0..native.outputs.len() {
                let d = native.output_delays[k];
                if d == 0 {
                    f.ins().store(stores, words[k], dst, 8 * k as i32);
                    continue;
                }
                let h = delayed.iter().position(|&x| x == k).expect("a delayed output");
                let up = f.ins().ishl_imm(words[k], 64 - i64::from(d));
                let before = f.ins().bor(held_in[h], up);
                f.ins().store(stores, before, dst, 8 * k as i32 - stride as i32);
                held_out.push(f.ins().ushr_imm(words[k], i64::from(d)));
            }
            // The running counts, and each extract's positions at its count.
            let mut counts_out = Vec::with_capacity(counted.len());
            for (c, &k) in counted.iter().enumerate() {
                let n = counts_in[c];
                let cnt = match (c < native.extracts.len()).then_some(c) {
                    Some(e) => {
                        let start = f.ins().ishl_imm(i, 6);
                        let at = f.ins().iadd_imm(start, -i64::from(native.output_delays[k]));
                        let offset = f.ins().ishl_imm(n, 2);
                        let ptr = f.ins().iadd(extract_base[e], offset);
                        let (cnt, more) = flatten(&mut f, words[k], at, ptr);
                        heads.push((more, Step::Bits));
                        cnt
                    }
                    // A total only tests for a bit (`BlockProgram::totals`): its words OR-ed.
                    None => {
                        counts_out.push(f.ins().bor(n, words[k]));
                        continue;
                    }
                };
                counts_out.push(f.ins().iadd(n, cnt));
            }
            let next = f.ins().iadd_imm(i, 1);
            let (next_base, next_dst) = (f.ins().iadd_imm(base, 64), f.ins().iadd_imm(dst, stride));
            let again: Vec<BlockArg> = [next, next_base, next_dst]
                .into_iter()
                .chain(packed_out)
                .chain(counts_out)
                .chain(held_out)
                .map(BlockArg::from)
                .collect();
            if full {
                // The hot loop's back edge: to itself while the next block is full.
                let more = f.ins().icmp(IntCC::UnsignedLessThan, next, full_end);
                f.ins().brif(more, body_block, &again, head, &again);
            } else {
                f.ins().jump(head, &again);
            }
        }

        // The row's end: each delayed output's last word, and the counts.
        f.switch_to_block(finish);
        let p = f.block_params(finish).to_vec();
        let dst = p[2];
        for (h, &k) in delayed.iter().enumerate() {
            f.ins().store(stores, p[3 + carried + counted.len() + h], dst, 8 * k as i32 - stride as i32);
        }
        for c in 0..counted.len() {
            let mut v = p[3 + carried + c];
            if c >= native.extracts.len() {
                let any = f.ins().icmp_imm(IntCC::NotEqual, v, 0);
                v = f.ins().uextend(types::I64, any);
            }
            f.ins().store(stores, v, counts_at, 8 * c as i32);
        }
        f.ins().jump(exit, &[]);

        f.switch_to_block(exit);
        let next_row = f.ins().iadd_imm(r, 1);
        let more_rows = f.ins().icmp(IntCC::UnsignedLessThan, next_row, nrows);
        f.ins().brif(more_rows, row, &[next_row.into()], done, &[]);

        f.switch_to_block(done);
        f.ins().return_(&[]);
        f.seal_all_blocks();
        f.finalize();
        shape = super::native_cpu_shape::check(&context.func, &heads);
    }
    shape.map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    let clif = context.func.display().to_string();
    super::native_cpu_codegen::define_function(&mut module, id, &mut context).map_err(|_| Error::BackendFailure)?;
    let disassembly = context
        .compiled_code()
        .and_then(|code| code.vcode.clone())
        .unwrap_or_default();
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_block` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };
    Ok(CompiledBlock {
        _module: module,
        entry,
        outputs: native.outputs.len(),
        delay: native.delay,
        extracts: native.extracts.clone(),
        totals: native.totals.clone(),
        counted,
        ascii_branch,
        gate_branches,
        clif,
        disassembly,
    })
}

/// A value of the bit logic: a known word, or a value that is the word itself or its complement.
#[derive(Clone, Copy)]
pub(super) enum Sym {
    Known(u64),
    Value(Value, bool),
}

impl Sym {
    fn not(self) -> Sym {
        match self {
            Sym::Known(c) => Sym::Known(!c),
            Sym::Value(v, neg) => Sym::Value(v, !neg),
        }
    }

    /// `a & b`, one instruction at most: a complemented side is a `bic`; both complemented,
    /// the complement of an `or`.
    fn and(f: &mut FunctionBuilder<'_>, a: Sym, b: Sym) -> Sym {
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
        }
    }

    fn xor(f: &mut FunctionBuilder<'_>, a: Sym, b: Sym) -> Sym {
        match (a, b) {
            (Sym::Known(x), Sym::Known(y)) => Sym::Known(x ^ y),
            (Sym::Known(0), s) | (s, Sym::Known(0)) => s,
            (Sym::Known(u64::MAX), s) | (s, Sym::Known(u64::MAX)) => s.not(),
            (Sym::Known(c), Sym::Value(v, neg)) | (Sym::Value(v, neg), Sym::Known(c)) => {
                let k = f.ins().iconst(types::I64, c as i64);
                Sym::Value(f.ins().bxor(v, k), neg)
            }
            (Sym::Value(x, nx), Sym::Value(y, ny)) => Sym::Value(f.ins().bxor(x, y), nx != ny),
        }
    }
}

/// Value `x` plain (not complemented), made once.
fn plain(f: &mut FunctionBuilder<'_>, syms: &[Sym], values: &mut [Option<Value>], x: usize) -> Value {
    if let Some(v) = values[x] {
        return v;
    }
    let v = match syms[x] {
        Sym::Known(c) => f.ins().iconst(types::I64, c as i64),
        Sym::Value(v, false) => v,
        Sym::Value(v, true) => f.ins().bnot(v),
    };
    values[x] = Some(v);
    v
}

/// Four 16-byte compare results (each byte 0 or 0xff) as one 64-bit mask.
pub(super) fn bitmask(f: &mut FunctionBuilder<'_>, parts: &[Value], weights: Value) -> Value {
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
