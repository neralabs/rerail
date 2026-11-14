//! Row programs on the CPU JIT (decision 0095 §2): Cranelift emits a row program's native IR
//! (`native_ir::BlockNative` of a `block::BlockProgram` with `rows`) as one function
//! `tempo_rows(columns, params, sets, words, out)`. Its one loop is over the run's words, 64 rows
//! each; a word's body is straight-line:
//!
//! - **`Cmp`**: each of the word's 64 rows loaded from its column, read at the comparison's
//!   width (a shift up and an arithmetic shift down), compared, and its bit set in the word.
//! - **`Bit`**: each row's value split into a word index and a bit; the index tested against the
//!   set's length and clamped by a select, so no row reads past the set, and the bit taken.
//! - **The mask operations** as they are.
//!
//! Every output's words are stored at `out[k * words + w]`. The columns are padded to whole
//! words by the caller (`CompiledRows::run`), so the last word reads no row past them; the host
//! cuts the bits past the batch, as the interpreter does.

use crate::Error;
use crate::block::{CmpOp, RowBatch, Rhs, mask_tail};
use crate::block_native::{BlockInst, BlockNative};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::{Linkage, Module};

/// The columns' pointers, the parameters, the sets as (pointer, words) pairs, the run's words,
/// and the outputs' words.
type Entry = unsafe extern "C" fn(*const *const i64, *const i64, *const [u64; 2], u64, *mut u64);

/// A compiled row program.
pub struct CompiledRows {
    _module: JITModule,
    entry: Entry,
    outputs: usize,
    columns: usize,
    /// The function's Cranelift IR, for the shape's checks.
    pub clif: String,
}

/// One word of zeros, for a set with none of its own.
static EMPTY: [u64; 1] = [0];

impl CompiledRows {
    /// Every output's words over the batch, output `k` at `out[k * words..]`; bits past the
    /// batch are zero. Columns shorter than whole words are copied padded first.
    pub fn run(&self, batch: &RowBatch<'_>, out: &mut Vec<u64>) {
        let words = batch.len.div_ceil(64);
        let padded: Vec<Vec<i64>> = batch.columns[..self.columns]
            .iter()
            .filter(|c| c.len() < 64 * words)
            .map(|c| {
                let mut p = c[..batch.len.min(c.len())].to_vec();
                p.resize(64 * words, 0);
                p
            })
            .collect();
        let mut short = padded.iter();
        let columns: Vec<*const i64> = batch.columns[..self.columns]
            .iter()
            .map(|c| if c.len() < 64 * words { short.next().expect("padded").as_ptr() } else { c.as_ptr() })
            .collect();
        let sets: Vec<[u64; 2]> = batch
            .sets
            .iter()
            .map(|s| if s.is_empty() { [EMPTY.as_ptr() as u64, 0] } else { [s.as_ptr() as u64, s.len() as u64] })
            .collect();
        out.clear();
        out.resize(self.outputs * words, 0);
        // SAFETY: every column holds `64 * words` values, every set its words (a pointer to one
        // word when it has none), and `out` the outputs' words: the function reads and writes
        // only those (`compile`).
        unsafe { (self.entry)(columns.as_ptr(), batch.params.as_ptr(), sets.as_ptr(), words as u64, out.as_mut_ptr()) };
        for k in 0..self.outputs {
            mask_tail(&mut out[k * words..(k + 1) * words], batch.len);
        }
    }
}

/// `v` at `width` bits, sign-extended back to 64 (`block::lane`).
fn lane(f: &mut FunctionBuilder<'_>, v: Value, width: u8) -> Value {
    if width >= 64 {
        return v;
    }
    let up = f.ins().ishl_imm(v, i64::from(64 - width));
    f.ins().sshr_imm(up, i64::from(64 - width))
}

pub(super) fn compile(native: &BlockNative, columns: usize) -> Result<CompiledRows, Error> {
    let builder = crate::native_cpu_codegen::tier1_builder().map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    for _ in 0..3 {
        signature.params.push(AbiParam::new(pointer));
    }
    signature.params.push(AbiParam::new(types::I64));
    signature.params.push(AbiParam::new(pointer));
    let id = module.declare_function("tempo_rows", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
    let mut context: Context = module.make_context();
    context.func.signature = signature;
    let mut builder_context = FunctionBuilderContext::new();
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut builder_context);
        let (entry, head, body, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
        f.append_block_params_for_function_params(entry);
        f.append_block_param(head, types::I64);
        f.switch_to_block(entry);
        let p = f.block_params(entry).to_vec();
        let (cols, params, sets, words, out) = (p[0], p[1], p[2], p[3], p[4]);
        let read = MemFlags::trusted().with_readonly();
        // Each column's pointer and each set's, loaded once.
        let col_ptr: Vec<Value> = (0..columns).map(|c| f.ins().load(pointer, read, cols, 8 * c as i32)).collect();
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().jump(head, &[zero.into()]);
        // The loop over words: a bounded count, `w < words`.
        f.switch_to_block(head);
        let w = f.block_params(head)[0];
        let more = f.ins().icmp(IntCC::UnsignedLessThan, w, words);
        f.ins().brif(more, body, &[], done, &[]);
        f.switch_to_block(body);
        let row0 = f.ins().ishl_imm(w, 9); // 64 rows of 8 bytes a word
        let mut values: Vec<Value> = Vec::with_capacity(native.insts.len());
        for inst in &native.insts {
            let value = match *inst {
                BlockInst::Cmp { col, op, rhs, width } => {
                    let a_base = f.ins().iadd(col_ptr[usize::from(col)], row0);
                    let (b_base, b_param) = match rhs {
                        Rhs::Col(c) => (Some(f.ins().iadd(col_ptr[usize::from(c)], row0)), None),
                        Rhs::Param(k) => {
                            let v = f.ins().load(types::I64, read, params, 8 * i32::from(k));
                            (None, Some(lane(&mut f, v, width)))
                        }
                    };
                    let cc = match op {
                        CmpOp::Eq => IntCC::Equal,
                        CmpOp::Ne => IntCC::NotEqual,
                        CmpOp::Lt => IntCC::SignedLessThan,
                        CmpOp::Le => IntCC::SignedLessThanOrEqual,
                    };
                    let mut m = f.ins().iconst(types::I64, 0);
                    for j in 0..64 {
                        let x = f.ins().load(types::I64, read, a_base, 8 * j);
                        let x = lane(&mut f, x, width);
                        let y = match (b_base, b_param) {
                            (Some(b), _) => {
                                let y = f.ins().load(types::I64, read, b, 8 * j);
                                lane(&mut f, y, width)
                            }
                            (None, Some(y)) => y,
                            _ => unreachable!("a column or a parameter"),
                        };
                        let bit = f.ins().icmp(cc, x, y);
                        let bit = f.ins().uextend(types::I64, bit);
                        let bit = f.ins().ishl_imm(bit, i64::from(j));
                        m = f.ins().bor(m, bit);
                    }
                    m
                }
                BlockInst::Bit { set, col } => {
                    let base = f.ins().iadd(col_ptr[usize::from(col)], row0);
                    let s = f.ins().load(pointer, read, sets, 16 * i32::from(set));
                    let len = f.ins().load(types::I64, read, sets, 16 * i32::from(set) + 8);
                    let mut m = f.ins().iconst(types::I64, 0);
                    for j in 0..64 {
                        let x = f.ins().load(types::I64, read, base, 8 * j);
                        let at = f.ins().ushr_imm(x, 6);
                        // A negative value is a huge word index: out, as any past the set.
                        let inside = f.ins().icmp(IntCC::UnsignedLessThan, at, len);
                        let safe = f.ins().select(inside, at, zero);
                        let off = f.ins().ishl_imm(safe, 3);
                        let addr = f.ins().iadd(s, off);
                        let word = f.ins().load(types::I64, read, addr, 0);
                        let k = f.ins().band_imm(x, 63);
                        let bit = f.ins().ushr(word, k);
                        let bit = f.ins().band_imm(bit, 1);
                        let inside = f.ins().uextend(types::I64, inside);
                        let bit = f.ins().band(bit, inside);
                        let bit = f.ins().ishl_imm(bit, i64::from(j));
                        m = f.ins().bor(m, bit);
                    }
                    m
                }
                BlockInst::Const(c) => f.ins().iconst(types::I64, c as i64),
                BlockInst::Not(a) => f.ins().bnot(values[usize::from(a)]),
                BlockInst::And(a, b) => f.ins().band(values[usize::from(a)], values[usize::from(b)]),
                BlockInst::Or(a, b) => f.ins().bor(values[usize::from(a)], values[usize::from(b)]),
                BlockInst::Xor(a, b) => f.ins().bxor(values[usize::from(a)], values[usize::from(b)]),
                BlockInst::AndNot(a, b) => f.ins().band_not(values[usize::from(a)], values[usize::from(b)]),
                ref other => unreachable!("verified: a row program has no {other:?}"),
            };
            values.push(value);
        }
        let at = f.ins().ishl_imm(w, 3);
        let stride = f.ins().ishl_imm(words, 3);
        let mut place = f.ins().iadd(out, at);
        for &o in &native.outputs {
            f.ins().store(MemFlags::trusted(), values[usize::from(o)], place, 0);
            place = f.ins().iadd(place, stride);
        }
        let next = f.ins().iadd_imm(w, 1);
        f.ins().jump(head, &[next.into()]);
        f.switch_to_block(done);
        f.ins().return_(&[]);
        f.seal_all_blocks();
        f.finalize();
    }
    let clif = context.func.display().to_string();
    crate::native_cpu_codegen::define_function(&mut module, id, &mut context).map_err(|_| Error::BackendFailure)?;
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    // SAFETY: the function was declared and defined with `Entry`'s signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(module.get_finalized_function(id)) };
    Ok(CompiledRows { _module: module, entry, outputs: native.outputs.len(), columns, clif })
}
