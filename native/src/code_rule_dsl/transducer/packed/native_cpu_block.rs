//! Block programs on the CPU JIT: Cranelift emits a `BlockNative` (the native IR) as one function
//! `tempo_block(bytes, blocks, out)`. Its only loop runs over the 64-byte blocks, a size; the body
//! is straight-line bit operations on vectors and words, with no branch (`native_cpu_value`
//! defines value programs the same way, one function each).
//!
//! The bitmask follows the architecture, as simdjson's does: on aarch64 its `to_bitmask` (each
//! byte weighted by its bit, then three rounds of pairwise adds, `addp`); elsewhere
//! `vhigh_bits` (one `pmovmskb` on x86).
//!
//! `out` holds each output's words in turn: output `k`'s word for block `i` at `k * blocks + i`.

use super::Error;
use super::block::{mask_tail, padded};
use super::native_ir::{BlockInst, BlockNative};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, ConstantData, Endianness, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

type Entry = unsafe extern "C" fn(*const u8, u64, *mut u64);

/// A compiled block program.
pub(super) struct CompiledBlock {
    _module: JITModule,
    entry: Entry,
    outputs: usize,
    /// The function as Cranelift IR, before code generation.
    pub(super) clif: String,
    /// The machine code, disassembled.
    pub(super) disassembly: String,
}

impl CompiledBlock {
    /// Every output's words over `bytes`; bits past the end are zero.
    pub(super) fn run(&self, bytes: &[u8]) -> Vec<Vec<u64>> {
        let blocks = padded(bytes);
        let flat: Vec<u8> = blocks.iter().flatten().copied().collect();
        let mut out = vec![0u64; self.outputs * blocks.len()];
        // SAFETY: the function reads `blocks.len()` whole blocks of `flat` and writes
        // `outputs * blocks.len()` words of `out`, both allocated to those sizes.
        unsafe { (self.entry)(flat.as_ptr(), blocks.len() as u64, out.as_mut_ptr()) };
        let n = blocks.len();
        (0..self.outputs)
            .map(|k| {
                let mut words = out[k * n..(k + 1) * n].to_vec();
                mask_tail(&mut words, bytes.len());
                words
            })
            .collect()
    }
}

impl CompiledBlock {
    /// The function's CPU time over `bytes` (thread user + system time), best of `runs`, the
    /// input already in blocks: the cost of the compiled code alone.
    #[cfg(test)]
    pub(super) fn cpu_seconds(&self, bytes: &[u8], runs: usize) -> f64 {
        let blocks = padded(bytes);
        let flat: Vec<u8> = blocks.iter().flatten().copied().collect();
        let mut out = vec![0u64; self.outputs * blocks.len()];
        let now = || {
            let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            // SAFETY: `t` is a valid timespec to write.
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
            t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
        };
        (0..runs)
            .map(|_| {
                let start = now();
                // SAFETY: as in `run`.
                unsafe { (self.entry)(flat.as_ptr(), blocks.len() as u64, out.as_mut_ptr()) };
                now() - start
            })
            .fold(f64::MAX, f64::min)
    }
}

/// Compile a block program's native IR.
pub(super) fn compile(native: &BlockNative) -> Result<CompiledBlock, Error> {
    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
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
        let entry = f.create_block();
        let body = f.create_block();
        let exit = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.append_block_param(body, types::I64);
        // One loop-carried word per carried instruction: `prev`'s previous word, `toggle`'s
        // parity (all ones or zero), `after_odd_run`'s borrow (0 or 1).
        let carried: Vec<usize> = (0..native.insts.len()).filter(|&at| native.insts[at].carried()).collect();
        for _ in &carried {
            f.append_block_param(body, types::I64);
        }

        // The entry: constants every block uses, and each output's first word.
        f.switch_to_block(entry);
        let params = f.block_params(entry).to_vec();
        let (bytes, blocks, out) = (params[0], params[1], params[2]);
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
                BlockInst::Plane(k) => {
                    let c = f.ins().iconst(types::I8, 1i64 << k);
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
        let output_bases: Vec<Value> = (0..native.outputs.len())
            .map(|k| {
                let first = f.ins().imul_imm(blocks, k as i64);
                let bytes_from = f.ins().ishl_imm(first, 3);
                f.ins().iadd(out, bytes_from)
            })
            .collect();
        let zero = f.ins().iconst(types::I64, 0);
        let even = f.ins().iconst(types::I64, super::block::carry::EVEN as i64);
        let none = f.ins().icmp_imm(IntCC::Equal, blocks, 0);
        let first: Vec<cranelift_codegen::ir::BlockArg> = std::iter::repeat_n(zero.into(), carried.len() + 1).collect();
        f.ins().brif(none, exit, &[], body, &first);

        // The body: one block, straight-line.
        f.switch_to_block(body);
        let i = f.block_params(body)[0];
        let carries_in: Vec<Value> = f.block_params(body)[1..].to_vec();
        let mut carries_out: Vec<Value> = Vec::with_capacity(carried.len());
        let offset = f.ins().ishl_imm(i, 6);
        let base = f.ins().iadd(bytes, offset);
        let loads = MemFlags::new().with_notrap();
        let mut values: Vec<Value> = Vec::with_capacity(native.insts.len());
        // The block's 64 bytes, loaded once for every compare and class.
        let reads = native.insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_)));
        let chunks: Vec<Value> = if reads {
            (0..4).map(|part| f.ins().load(types::I8X16, loads, base, part * 16)).collect()
        } else {
            vec![]
        };
        // Each class's nibbles, split once.
        let (low, high): (Vec<Value>, Vec<Value>) = if native.insts.iter().any(|i| matches!(i, BlockInst::Class { .. })) {
            chunks.iter().map(|&c| (f.ins().band(c, nibble), f.ins().ushr_imm(c, 4))).unzip()
        } else {
            (vec![], vec![])
        };
        // Each table's class bytes, looked up once per block for every class taken from it.
        let mut classes: Vec<Option<Vec<Value>>> = vec![None; native.tables.len()];
        for (at, inst) in native.insts.iter().enumerate() {
            let v = |x: u16| values[usize::from(x)];
            let value = match *inst {
                BlockInst::EqByte(_) => {
                    let splat = constants[at].expect("a splat per compare");
                    let parts: Vec<Value> = chunks.iter().map(|&bytes| f.ins().icmp(IntCC::Equal, bytes, splat)).collect();
                    bitmask(&mut f, &parts, weights)
                }
                BlockInst::Class { table, .. } => {
                    let sel = constants[at].expect("a splat per class");
                    let t = usize::from(table);
                    if classes[t].is_none() {
                        let (lo, hi) = tables[t];
                        classes[t] = Some(
                            (0..4)
                                .map(|part| {
                                    let l = f.ins().swizzle(lo, low[part]);
                                    let h = f.ins().swizzle(hi, high[part]);
                                    f.ins().band(l, h)
                                })
                                .collect(),
                        );
                    }
                    let bytes = classes[t].clone().expect("looked up");
                    let parts: Vec<Value> = bytes
                        .iter()
                        .map(|&both| {
                            let picked = f.ins().band(both, sel);
                            f.ins().icmp(IntCC::NotEqual, picked, zero16)
                        })
                        .collect();
                    bitmask(&mut f, &parts, weights)
                }
                BlockInst::Plane(_) => {
                    let bit = constants[at].expect("a splat per plane");
                    let parts: Vec<Value> = chunks
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
            };
            values.push(value);
        }
        for (k, &v) in native.outputs.iter().enumerate() {
            let at = f.ins().ishl_imm(i, 3);
            let address = f.ins().iadd(output_bases[k], at);
            f.ins().store(MemFlags::new().with_notrap(), values[usize::from(v)], address, 0);
        }
        let next = f.ins().iadd_imm(i, 1);
        let more = f.ins().icmp(IntCC::UnsignedLessThan, next, blocks);
        let again: Vec<cranelift_codegen::ir::BlockArg> =
            std::iter::once(next.into()).chain(carries_out.iter().map(|&c| c.into())).collect();
        f.ins().brif(more, body, &again, exit, &[]);

        f.switch_to_block(exit);
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
    Ok(CompiledBlock { _module: module, entry, outputs: native.outputs.len(), clif, disassembly })
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
