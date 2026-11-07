//! Block programs on the CPU JIT: Cranelift emits a `BlockNative` (the native IR) as one function
//! `tempo_block(bytes, rows, nrows, out)` over a batch of files (decision 0044). Its two loops run
//! over the files and over each file's 64-byte blocks, both sizes; the body is straight-line bit
//! operations on vectors and words, with no branch (`native_cpu_value` defines value programs the
//! same way, one function each). Each file starts with zero carries.
//!
//! The bitmask follows the architecture, as simdjson's does: on aarch64 its `to_bitmask` (each
//! byte weighted by its bit, then three rounds of pairwise adds, `addp`); elsewhere
//! `vhigh_bits` (one `pmovmskb` on x86).
//!
//! A file's words in `out` start at its row's `out`: output `k`'s word for block `i` at
//! `k * blocks + i`; then each extract's popcount over all its blocks, the size of its extract
//! pass.

use super::Error;
use super::block::{BatchFile, BlockRun, extract::positions, mask_tail, padded_by, undelay};
use super::native_ir::{BlockInst, BlockNative};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, ConstantData, Endianness, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

/// The batch's bytes, its rows (`Row`), their count, and the outputs.
type Entry = unsafe extern "C" fn(*const u8, *const Row, u64, *mut u64);

/// A file of a batch, for the function: where its blocks start in the bytes (`FRONT` zero bytes
/// before them), how many there are, its length (for `within`), and the word at which its
/// outputs start (each output's words in turn, then each extract's count).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    bytes: u64,
    blocks: u64,
    len: u64,
    out: u64,
}

/// A compiled block program.
pub(super) struct CompiledBlock {
    _module: JITModule,
    entry: Entry,
    outputs: usize,
    /// The outputs whose positions the extract pass writes.
    extracts: Vec<usize>,
    /// The program's delay (`block::BlockProgram::delay`), and each output's.
    delay: u8,
    output_delays: Vec<u8>,
    /// The function as Cranelift IR, before code generation.
    pub(super) clif: String,
    /// The machine code, disassembled.
    pub(super) disassembly: String,
}

impl CompiledBlock {
    /// Every output's words over `bytes`; bits past the end are zero.
    pub(super) fn run(&self, bytes: &[u8]) -> Vec<Vec<u64>> {
        self.extract(bytes).outputs
    }

    /// The outputs, then the extract pass: each extract's positions, into a vector allocated at
    /// the count the function summed as it wrote the plane.
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
/// bytes; a row per file; and one output buffer for all of them.
pub(super) struct Batch {
    bytes: Vec<u8>,
    rows: Vec<Row>,
    out: Vec<u64>,
}

impl Batch {
    pub(super) fn new(program: &CompiledBlock, files: &[&[u8]]) -> Self {
        let (mut bytes, mut rows, mut words) = (Vec::new(), Vec::with_capacity(files.len()), 0u64);
        for file in files {
            let blocks = padded_by(file, usize::from(program.delay));
            bytes.resize(bytes.len() + FRONT, 0);
            let row = Row { bytes: bytes.len() as u64, blocks: blocks.len() as u64, len: file.len() as u64, out: words };
            bytes.extend(blocks.iter().flatten());
            words += (program.outputs * blocks.len() + program.extracts.len()) as u64;
            rows.push(row);
        }
        Self { bytes, rows, out: vec![0u64; words as usize] }
    }

    pub(super) fn call(&mut self, program: &CompiledBlock) {
        // SAFETY: each row's blocks lie in `bytes` after `FRONT` zero bytes, and its words (each
        // output's per block, then a count per extract) in `out`; the function reads and writes
        // nothing else.
        unsafe { (program.entry)(self.bytes.as_ptr(), self.rows.as_ptr(), self.rows.len() as u64, self.out.as_mut_ptr()) };
    }

    fn results(&self, program: &CompiledBlock, files: &[&[u8]]) -> Vec<BlockRun> {
        self.rows
            .iter()
            .zip(files)
            .map(|(row, bytes)| {
                let (n, base) = (row.blocks as usize, row.out as usize);
                let out = &self.out[base..base + program.outputs * n + program.extracts.len()];
                // The function counts whole blocks; the bits past the end (the padding's) come off.
                let past = |words: &[u64]| match (words.last(), bytes.len() % 64) {
                    (Some(&w), r) if r != 0 => (w >> r).count_ones() as usize,
                    _ => 0,
                };
                let positions = program
                    .extracts
                    .iter()
                    .enumerate()
                    .map(|(e, &k)| {
                        let words = &out[k * n..(k + 1) * n];
                        let count = out[program.outputs * n + e] as usize - past(words);
                        let mut words = words.to_vec();
                        mask_tail(&mut words, bytes.len());
                        positions(&words, count)
                    })
                    .collect();
                let outputs = (0..program.outputs).map(|k| undelay(&out[k * n..(k + 1) * n], program.output_delays[k], bytes.len())).collect();
                BlockRun { outputs, positions }
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
        let now = || {
            let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            // SAFETY: `t` is a valid timespec to write.
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
            t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
        };
        (0..runs)
            .map(|_| {
                let start = now();
                batch.call(self);
                now() - start
            })
            .fold(f64::MAX, f64::min)
    }
}

/// Zero bytes before the first block: `pair` and `plane_before` load each block one byte back,
/// and the byte before the stream is 0x00.
const FRONT: usize = 16;


/// Compile a block program's native IR.
pub(super) fn compile(native: &BlockNative) -> Result<CompiledBlock, Error> {
    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(pointer));
    signature.params.push(AbiParam::new(types::I64));
    signature.params.push(AbiParam::new(pointer));
    let id = module
        .declare_function("tempo_block", Linkage::Local, &signature)
        .map_err(|_| Error::BackendFailure)?;
    let mut context: Context = module.make_context();
    context.func.signature = signature;
    context.set_disasm(true);
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        // entry -> row (one per file) -> body (one per block) -> exit (the file's counts) ->
        // row or done.
        let entry = f.create_block();
        let row = f.create_block();
        let body = f.create_block();
        let exit = f.create_block();
        let done = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.append_block_param(row, types::I64);
        f.append_block_param(body, types::I64);
        // One loop-carried word per carried instruction: `prev`'s previous word, `toggle`'s
        // parity (all ones or zero), `after_odd_run`'s and `fill`'s borrow (0 or 1).
        let carried: Vec<usize> = (0..native.insts.len()).filter(|&at| native.insts[at].carried()).collect();
        for _ in &carried {
            f.append_block_param(body, types::I64);
        }
        // One loop-carried popcount per extract, the mask pass's count for the extract pass;
        // stored after the outputs' words at the exit.
        for _ in &native.extracts {
            f.append_block_param(body, types::I64);
            f.append_block_param(exit, types::I64);
        }

        // The entry: constants every block uses, and each output's first word.
        f.switch_to_block(entry);
        let params = f.block_params(entry).to_vec();
        let (arena, rows, nrows, out_all) = (params[0], params[1], params[2], params[3]);
        // Constants, made once outside the loop: each compare's splat, each `const`'s word, and
        // the bitmask's weights.
        let constants: Vec<Option<Value>> = native
            .insts
            .iter()
            .map(|inst| match *inst {
                BlockInst::EqByte(byte) => {
                    let c = f.ins().iconst(types::I8, i64::from(byte));
                    Some(f.ins().splat(types::I8X16, c))
                }
                BlockInst::Const(c) => Some(f.ins().iconst(types::I64, c as i64)),
                BlockInst::Class { bits, .. } => {
                    let c = f.ins().iconst(types::I8, i64::from(bits));
                    Some(f.ins().splat(types::I8X16, c))
                }
                BlockInst::Plane(k) | BlockInst::PlaneBefore(k) => {
                    let c = f.ins().iconst(types::I8, 1i64 << k);
                    Some(f.ins().splat(types::I8X16, c))
                }
                BlockInst::Pair { bits, .. } => {
                    let c = f.ins().iconst(types::I8, i64::from(bits));
                    Some(f.ins().splat(types::I8X16, c))
                }
                _ => None,
            })
            .collect();
        // Each table's two halves as vector constants, the low-nibble mask, and zero.
        let tables: Vec<(Value, Value)> = native
            .tables
            .iter()
            .map(|t| {
                let lo = f.func.dfg.constants.insert(ConstantData::from(&t.lo[..]));
                let hi = f.func.dfg.constants.insert(ConstantData::from(&t.hi[..]));
                (f.ins().vconst(types::I8X16, lo), f.ins().vconst(types::I8X16, hi))
            })
            .collect();
        let nibble = {
            let c = f.ins().iconst(types::I8, 0x0f);
            f.ins().splat(types::I8X16, c)
        };
        let zero16 = {
            let c = f.ins().iconst(types::I8, 0);
            f.ins().splat(types::I8X16, c)
        };
        let weights = {
            const WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
            let handle = f.func.dfg.constants.insert(ConstantData::from(&WEIGHTS[..]));
            f.ins().vconst(types::I8X16, handle)
        };
        let zero = f.ins().iconst(types::I64, 0);
        let even = f.ins().iconst(types::I64, super::block::carry::EVEN as i64);
        let no_rows = f.ins().icmp_imm(IntCC::Equal, nrows, 0);
        f.ins().brif(no_rows, done, &[], row, &[zero.into()]);

        // A row: the file's blocks, length and outputs.
        f.switch_to_block(row);
        let r = f.block_params(row)[0];
        let at = f.ins().imul_imm(r, std::mem::size_of::<Row>() as i64);
        let at = f.ins().iadd(rows, at);
        let flags = MemFlags::trusted().with_readonly();
        let field = |f: &mut FunctionBuilder<'_>, k: i32| f.ins().load(types::I64, flags, at, 8 * k);
        let (start, blocks, len, first_word) = (field(&mut f, 0), field(&mut f, 1), field(&mut f, 2), field(&mut f, 3));
        let bytes = f.ins().iadd(arena, start);
        let out = {
            let from = f.ins().ishl_imm(first_word, 3);
            f.ins().iadd(out_all, from)
        };
        let output_bases: Vec<Value> = (0..native.outputs.len())
            .map(|k| {
                let first = f.ins().imul_imm(blocks, k as i64);
                let bytes_from = f.ins().ishl_imm(first, 3);
                f.ins().iadd(out, bytes_from)
            })
            .collect();
        let none = f.ins().icmp_imm(IntCC::Equal, blocks, 0);
        let first: Vec<cranelift_codegen::ir::BlockArg> =
            std::iter::repeat_n(zero.into(), carried.len() + native.extracts.len() + 1).collect();
        let no_counts: Vec<cranelift_codegen::ir::BlockArg> = std::iter::repeat_n(zero.into(), native.extracts.len()).collect();
        f.ins().brif(none, exit, &no_counts, body, &first);

        // The body: one block, straight-line.
        f.switch_to_block(body);
        let i = f.block_params(body)[0];
        let carries_in: Vec<Value> = f.block_params(body)[1..1 + carried.len()].to_vec();
        let counts_in: Vec<Value> = f.block_params(body)[1 + carried.len()..].to_vec();
        let mut carries_out: Vec<Value> = Vec::with_capacity(carried.len());
        let offset = f.ins().ishl_imm(i, 6);
        let base = f.ins().iadd(bytes, offset);
        let loads = MemFlags::new().with_notrap();
        let mut values: Vec<Value> = Vec::with_capacity(native.insts.len());
        // The block's 64 bytes, loaded once for every compare and class.
        let reads = native.insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::Pair { .. }));
        let chunks: Vec<Value> = if reads {
            (0..4).map(|part| f.ins().load(types::I8X16, loads, base, part * 16)).collect()
        } else {
            vec![]
        };
        // The block's bytes one back (each byte's byte before), loaded once for every `pair`:
        // unaligned loads at -1, the first block's reading `FRONT`'s zeros.
        let back = native.insts.iter().any(|i| matches!(i, BlockInst::Pair { .. } | BlockInst::PlaneBefore(_)));
        let befores: Vec<Value> = if back {
            (0..4).map(|part| f.ins().load(types::I8X16, loads, base, part * 16 - 1)).collect()
        } else {
            vec![]
        };
        // Each class's nibbles, split once.
        let classy = native.insts.iter().any(|i| matches!(i, BlockInst::Class { .. } | BlockInst::Pair { .. }));
        let (low, high): (Vec<Value>, Vec<Value>) = if classy {
            chunks.iter().map(|&c| (f.ins().band(c, nibble), f.ins().ushr_imm(c, 4))).unzip()
        } else {
            (vec![], vec![])
        };
        let (low_before, high_before): (Vec<Value>, Vec<Value>) =
            befores.iter().map(|&c| (f.ins().band(c, nibble), f.ins().ushr_imm(c, 4))).unzip();
        // Each table's class bytes, looked up once per block for every class taken from it (and
        // at the bytes before, for every `pair`).
        let mut classes: Vec<Option<Vec<Value>>> = vec![None; native.tables.len()];
        let mut classes_before: Vec<Option<Vec<Value>>> = vec![None; native.tables.len()];
        let lookup = |f: &mut FunctionBuilder<'_>, cache: &mut Vec<Option<Vec<Value>>>, t: usize, low: &[Value], high: &[Value]| {
            if cache[t].is_none() {
                let (lo, hi) = tables[t];
                cache[t] = Some(
                    (0..4)
                        .map(|part| {
                            let l = f.ins().swizzle(lo, low[part]);
                            let h = f.ins().swizzle(hi, high[part]);
                            f.ins().band(l, h)
                        })
                        .collect(),
                );
            }
            cache[t].clone().expect("looked up")
        };
        for (at, inst) in native.insts.iter().enumerate() {
            let v = |x: u16| values[usize::from(x)];
            let value = match *inst {
                // The bits below the stream's end: t = clamp(len - 64 i, 0, 64) ones, by two
                // shifts so that t = 64 needs no shift by 64 (`block::within`). At a delay `d`,
                // the bits below `len + d` and not below `d` (`block::within_at`).
                BlockInst::Within(d) => {
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
                        below(&mut f, len)
                    } else {
                        let end = f.ins().iadd_imm(len, i64::from(d));
                        let upper = below(&mut f, end);
                        let start = f.ins().iconst(types::I64, i64::from(d));
                        let lower = below(&mut f, start);
                        f.ins().band_not(upper, lower)
                    }
                }
                BlockInst::EqByte(_) => {
                    let splat = constants[at].expect("a splat per compare");
                    let parts: Vec<Value> = chunks.iter().map(|&bytes| f.ins().icmp(IntCC::Equal, bytes, splat)).collect();
                    bitmask(&mut f, &parts, weights)
                }
                BlockInst::Class { table, .. } => {
                    let sel = constants[at].expect("a splat per class");
                    let bytes = lookup(&mut f, &mut classes, usize::from(table), &low, &high);
                    let parts: Vec<Value> = bytes
                        .iter()
                        .map(|&both| {
                            let picked = f.ins().band(both, sel);
                            f.ins().icmp(IntCC::NotEqual, picked, zero16)
                        })
                        .collect();
                    bitmask(&mut f, &parts, weights)
                }
                BlockInst::Pair { before, table, .. } => {
                    let sel = constants[at].expect("a splat per pair");
                    let back = lookup(&mut f, &mut classes_before, usize::from(before), &low_before, &high_before);
                    let own = lookup(&mut f, &mut classes, usize::from(table), &low, &high);
                    let parts: Vec<Value> = back
                        .iter()
                        .zip(&own)
                        .map(|(&p, &b)| {
                            let both = f.ins().band(p, b);
                            let picked = f.ins().band(both, sel);
                            f.ins().icmp(IntCC::NotEqual, picked, zero16)
                        })
                        .collect();
                    bitmask(&mut f, &parts, weights)
                }
                BlockInst::Plane(_) | BlockInst::PlaneBefore(_) => {
                    let bit = constants[at].expect("a splat per plane");
                    let source = if matches!(inst, BlockInst::Plane(_)) { &chunks } else { &befores };
                    let parts: Vec<Value> = source
                        .iter()
                        .map(|&bytes| {
                            let picked = f.ins().band(bytes, bit);
                            f.ins().icmp(IntCC::NotEqual, picked, zero16)
                        })
                        .collect();
                    bitmask(&mut f, &parts, weights)
                }
                BlockInst::Const(_) => constants[at].expect("a word per const"),
                BlockInst::Not(a) => f.ins().bnot(v(a)),
                BlockInst::And(a, b) => f.ins().band(v(a), v(b)),
                BlockInst::Or(a, b) => f.ins().bor(v(a), v(b)),
                BlockInst::Xor(a, b) => f.ins().bxor(v(a), v(b)),
                BlockInst::AndNot(a, b) => f.ins().band_not(v(a), v(b)),
                BlockInst::Prev(a, k) => {
                    let previous = carries_in[carries_out.len()];
                    let up = f.ins().ishl_imm(v(a), i64::from(k));
                    let halo = f.ins().ushr_imm(previous, 64 - i64::from(k));
                    carries_out.push(v(a));
                    f.ins().bor(up, halo)
                }
                BlockInst::Toggle(a) => {
                    // prefix XOR by six shift-XOR rounds (Cranelift has no carry-less multiply),
                    // then the parity carried in.
                    let parity = carries_in[carries_out.len()];
                    let mut x = v(a);
                    for k in [1, 2, 4, 8, 16, 32] {
                        let up = f.ins().ishl_imm(x, k);
                        x = f.ins().bxor(x, up);
                    }
                    let y = f.ins().bxor(x, parity);
                    carries_out.push(f.ins().sshr_imm(y, 63));
                    y
                }
                BlockInst::AfterOddRun(a) => {
                    // simdjson's find_escaped_branchless (`block::carry::after_odd_run`).
                    let cin = carries_in[carries_out.len()];
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
                    carries_out.push(f.ins().uextend(types::I64, overflow));
                    f.ins().band_not(escaped, run)
                }
                BlockInst::Fill(s, e) => {
                    // `block::carry::fill`: the carry into each bit of `s + (s | !e) + cin` is
                    // the fill coming into that byte.
                    let cin = carries_in[carries_out.len()];
                    let (start, end) = (v(s), v(e));
                    let b = f.ins().bor_not(start, end);
                    let sum = f.ins().iadd(start, b);
                    let sum = f.ins().iadd(sum, cin);
                    let into = f.ins().bxor(sum, start);
                    let into = f.ins().bxor(into, b);
                    let open = f.ins().band_not(into, end);
                    let y = f.ins().bor(start, open);
                    carries_out.push(f.ins().ushr_imm(y, 63));
                    y
                }
            };
            values.push(value);
        }
        for (k, &v) in native.outputs.iter().enumerate() {
            let at = f.ins().ishl_imm(i, 3);
            let address = f.ins().iadd(output_bases[k], at);
            f.ins().store(MemFlags::new().with_notrap(), values[usize::from(v)], address, 0);
        }
        let counts_out: Vec<Value> = native
            .extracts
            .iter()
            .zip(&counts_in)
            .map(|(&k, &c)| {
                let n = f.ins().popcnt(values[usize::from(native.outputs[k])]);
                f.ins().iadd(c, n)
            })
            .collect();
        let next = f.ins().iadd_imm(i, 1);
        let more = f.ins().icmp(IntCC::UnsignedLessThan, next, blocks);
        let again: Vec<cranelift_codegen::ir::BlockArg> = std::iter::once(next.into())
            .chain(carries_out.iter().map(|&c| c.into()))
            .chain(counts_out.iter().map(|&c| c.into()))
            .collect();
        let counted: Vec<cranelift_codegen::ir::BlockArg> = counts_out.iter().map(|&c| c.into()).collect();
        f.ins().brif(more, body, &again, exit, &counted);

        f.switch_to_block(exit);
        let totals = f.block_params(exit).to_vec();
        if !totals.is_empty() {
            let words = f.ins().imul_imm(blocks, native.outputs.len() as i64);
            let from = f.ins().ishl_imm(words, 3);
            let after = f.ins().iadd(out, from);
            for (e, &t) in totals.iter().enumerate() {
                f.ins().store(MemFlags::new().with_notrap(), t, after, (e * 8) as i32);
            }
        }
        let next_row = f.ins().iadd_imm(r, 1);
        let more_rows = f.ins().icmp(IntCC::UnsignedLessThan, next_row, nrows);
        f.ins().brif(more_rows, row, &[next_row.into()], done, &[]);

        f.switch_to_block(done);
        f.ins().return_(&[]);
        f.seal_all_blocks();
        f.finalize();
    }
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
    Ok(CompiledBlock { _module: module, entry, outputs: native.outputs.len(), extracts: native.extracts.clone(), delay: native.delay, output_delays: native.output_delays.clone(), clif, disassembly })
}

/// Four 16-byte compare results (each byte 0 or 0xff) as one 64-bit mask.
fn bitmask(f: &mut FunctionBuilder<'_>, parts: &[Value], weights: Value) -> Value {
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
