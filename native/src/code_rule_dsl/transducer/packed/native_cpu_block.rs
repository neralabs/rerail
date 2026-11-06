//! Block programs on the CPU JIT: Cranelift emits a `BlockNative` (the native IR) as one function
//! `tempo_block(bytes, blocks, out)`. Its only loop runs over the 64-byte blocks, a size; the body
//! is straight-line bit operations on vectors and words, with no branch (`native_cpu_value`
//! defines value programs the same way, one function each).
//!
//! `out` holds each output's words in turn: output `k`'s word for block `i` at `k * blocks + i`.

use super::Error;
use super::block::{mask_tail, padded};
use super::native_ir::{BlockInst, BlockNative};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
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

        // The entry: constants every block uses, and each output's first word.
        f.switch_to_block(entry);
        let params = f.block_params(entry).to_vec();
        let (bytes, blocks, out) = (params[0], params[1], params[2]);
        let splats: Vec<Option<Value>> = native
            .insts
            .iter()
            .map(|inst| match *inst {
                BlockInst::EqByte(byte) => {
                    let c = f.ins().iconst(types::I8, i64::from(byte));
                    Some(f.ins().splat(types::I8X16, c))
                }
            })
            .collect();
        let output_bases: Vec<Value> = (0..native.outputs.len())
            .map(|k| {
                let first = f.ins().imul_imm(blocks, k as i64);
                let bytes_from = f.ins().ishl_imm(first, 3);
                f.ins().iadd(out, bytes_from)
            })
            .collect();
        let zero = f.ins().iconst(types::I64, 0);
        let none = f.ins().icmp_imm(IntCC::Equal, blocks, 0);
        f.ins().brif(none, exit, &[], body, &[zero.into()]);

        // The body: one block, straight-line.
        f.switch_to_block(body);
        let i = f.block_params(body)[0];
        let offset = f.ins().ishl_imm(i, 6);
        let base = f.ins().iadd(bytes, offset);
        let loads = MemFlags::new().with_notrap();
        let mut values: Vec<Value> = Vec::with_capacity(native.insts.len());
        for (at, inst) in native.insts.iter().enumerate() {
            let value = match *inst {
                BlockInst::EqByte(_) => {
                    let splat = splats[at].expect("a splat per compare");
                    // Four 16-byte compares, each a 16-bit mask, joined into one word.
                    let mut word: Option<Value> = None;
                    for part in 0..4 {
                        let v = f.ins().load(types::I8X16, loads, base, part * 16);
                        let m = f.ins().icmp(IntCC::Equal, v, splat);
                        let bits = f.ins().vhigh_bits(types::I16, m);
                        let wide = f.ins().uextend(types::I64, bits);
                        let placed = if part == 0 { wide } else { f.ins().ishl_imm(wide, i64::from(part * 16)) };
                        word = Some(match word {
                            None => placed,
                            Some(w) => f.ins().bor(w, placed),
                        });
                    }
                    word.expect("four parts")
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
        f.ins().brif(more, body, &[next.into()], exit, &[]);

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
