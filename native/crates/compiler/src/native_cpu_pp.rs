//! The preprocessing words on the CPU JIT (decision 0348, stage 4b): Cranelift emits a program's
//! words (`pp_words`, the meaning) as one function `pp_run(ctx)`, a block a word, made from the
//! word and its operand alone (and the program's strings), as tier 2's stream words are
//! (`native_cpu_stream`).
//!
//! The machine's state is `pp_words::Ctx`: the machine's own buffers, each reserved to its bound
//! ahead, so a word writes in place, and a write past a bound refuses the TU (to the lean pass) as
//! the interpreter's does. A call's return is a slot of `Ctx::returns`, and `RETURN` jumps
//! through one jump table of the words.
//!
//! The loops are the words' stated steps, as the verifier requires: a token read (`READ`'s
//! block), a call returned (`Step::Return`), a source or conditional popped (`Step::Pop`).
//! `native_cpu_shape` refuses any other loop, and any call.

use crate::Error;
use crate::native_cpu_shape::Step;
use crate::pp_words::{self, BOL, Bounds, COND_SIZE, COND_SRC, Ctx, Def, EOF, Env, Expanded, File, FileC, HASH, NONE, Refused, Src, Tok};
use crate::program::{ACCEPT, GOTO, IS, PUSH, PUT, READ, RETURN, STOP, Words, split};
use cranelift_codegen::ir::{AbiParam, Block, BlockCall, InstBuilder, JumpTableData, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use std::mem::{offset_of, size_of};

/// Why the compiled words refuse a TU: the index the code returns above bit 32 (`run_native`),
/// each the interpreter's reason.
const REASONS: &[&str] = &["the compiled words refuse", "the program stopped", "past the calls' bound", "a return with no call", "past the output's bound"];
const R_STOPPED: u64 = 1;
const R_CALLS: u64 = 2;
const R_NO_CALL: u64 = 3;
const R_OUTPUT: u64 = 4;

/// A program's words, compiled.
pub struct CompiledPp {
    _module: JITModule,
    entry: unsafe extern "C" fn(*mut Ctx) -> u64,
    /// The function's IR, as text.
    pub clif: String,
    /// Its stated loop heads.
    pub heads: Vec<Step>,
}

impl CompiledPp {
    /// `pp_words::run`'s work, by the compiled words.
    pub fn run(&self, program: &Words, files: &[File], env: &Env, bounds: &Bounds) -> Result<Expanded, Refused> {
        // SAFETY: the code reads and writes only within its `Ctx`: every write is checked
        // against its buffer's room, every read is of a buffer's written part.
        unsafe { pp_words::run_native(program, files, env, bounds, self.entry, REASONS) }
    }
}

/// `Ctx`'s field `$f`'s offset.
macro_rules! at {
    ($f:ident) => {
        offset_of!(Ctx, $f) as i32
    };
}

const TOK: i64 = size_of::<Tok>() as i64;
const SRC: i64 = size_of::<Src>() as i64;
const FILE: i64 = size_of::<FileC>() as i64;
const DEF: i64 = size_of::<Def>() as i64;

fn flags() -> MemFlags {
    MemFlags::trusted()
}

fn ld(f: &mut FunctionBuilder<'_>, ty: types::Type, base: Value, off: i32) -> Value {
    f.ins().load(ty, flags(), base, off)
}

/// A narrow load, widened to 64 bits.
fn ldu(f: &mut FunctionBuilder<'_>, ty: types::Type, base: Value, off: i32) -> Value {
    let x = f.ins().load(ty, flags(), base, off);
    if ty == types::I64 { x } else { f.ins().uextend(types::I64, x) }
}

fn st(f: &mut FunctionBuilder<'_>, ty: types::Type, x: Value, base: Value, off: i32) {
    let x = if f.func.dfg.value_type(x) == ty { x } else { f.ins().ireduce(ty, x) };
    f.ins().store(flags(), x, base, off);
}

/// `base + i * size`.
fn elem(f: &mut FunctionBuilder<'_>, base: Value, i: Value, size: i64) -> Value {
    let o = f.ins().imul_imm(i, size);
    f.ins().iadd(base, o)
}

/// A token's 20 bytes copied.
fn copy_tok(f: &mut FunctionBuilder<'_>, from: Value, from_off: i32, to: Value, to_off: i32) {
    for w in 0..(TOK as i32 / 4) {
        let x = f.ins().load(types::I32, flags(), from, from_off + 4 * w);
        f.ins().store(flags(), x, to, to_off + 4 * w);
    }
}

/// The function's code, a word at a time.
struct Lower<'a, 'b> {
    f: &'a mut FunctionBuilder<'b>,
    ctx: Value,
    words: &'a [u32],
    strings: &'a [Vec<u8>],
    blocks: Vec<Block>,
    /// Returns its one argument, the run's result.
    exit: Block,
    heads: Vec<(Block, Step)>,
}

impl Lower<'_, '_> {
    /// Refuse at word `pc` for `reason`.
    fn refuse(&mut self, pc: usize, reason: u64) {
        let r = self.f.ins().iconst(types::I64, ((pc as u64 + 1) | (reason << 32)) as i64);
        self.f.ins().jump(self.exit, &[r.into()]);
    }

    /// Go on to `then` where `c` holds, else refuse at `pc` for `reason`.
    fn check(&mut self, c: Value, pc: usize, reason: u64) {
        let ok = self.f.create_block();
        let no = self.f.create_block();
        self.f.ins().brif(c, ok, &[], no, &[]);
        self.f.switch_to_block(no);
        self.refuse(pc, reason);
        self.f.switch_to_block(ok);
    }

    /// A test's result: the next word where it holds, the one after where it does not.
    fn test(&mut self, pc: usize, held: Value) {
        let (yes, no) = (self.blocks[pc + 1], self.blocks[pc + 2]);
        self.f.ins().brif(held, yes, &[], no, &[]);
    }

    fn next(&mut self, pc: usize) {
        let b = self.blocks[pc + 1];
        self.f.ins().jump(b, &[]);
    }

    /// `T`'s spelling: its first byte's address and its length.
    fn text(&mut self) -> (Value, Value) {
        let (ctx, f) = (self.ctx, &mut *self.f);
        let buf = ldu(f, types::I32, ctx, at!(t) + offset_of!(Tok, buf) as i32);
        let start = ldu(f, types::I32, ctx, at!(t) + offset_of!(Tok, start) as i32);
        let end = ldu(f, types::I32, ctx, at!(t) + offset_of!(Tok, end) as i32);
        let scratch = f.create_block();
        let file = f.create_block();
        let join = f.create_block();
        f.append_block_param(join, types::I64);
        f.ins().brif(buf, file, &[], scratch, &[]);
        f.switch_to_block(scratch);
        let s = ld(f, types::I64, ctx, at!(scratch));
        f.ins().jump(join, &[s.into()]);
        f.switch_to_block(file);
        let files = ld(f, types::I64, ctx, at!(files));
        let k = f.ins().iadd_imm(buf, -1);
        let fp = elem(f, files, k, FILE);
        let b = ld(f, types::I64, fp, offset_of!(FileC, bytes) as i32);
        f.ins().jump(join, &[b.into()]);
        f.switch_to_block(join);
        let base = f.block_params(join)[0];
        let p = f.ins().iadd(base, start);
        let n = f.ins().isub(end, start);
        (p, n)
    }

    fn word(&mut self, pc: usize) -> Result<(), Error> {
        let (op, mode, operand) = split(self.words[pc]);
        let ctx = self.ctx;
        match op {
            GOTO => {
                let b = self.blocks[usize::from(operand)];
                self.f.ins().jump(b, &[]);
            }
            ACCEPT => {
                let z = self.f.ins().iconst(types::I64, 0);
                self.f.ins().jump(self.exit, &[z.into()]);
            }
            STOP => self.refuse(pc, R_STOPPED),
            PUSH => {
                let n = ld(self.f, types::I64, ctx, at!(nreturns));
                let room = ld(self.f, types::I64, ctx, at!(room_returns));
                let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, n, room);
                self.check(ok, pc, R_CALLS);
                let r = ld(self.f, types::I64, ctx, at!(returns));
                let slot = elem(self.f, r, n, 4);
                let to = self.f.ins().iconst(types::I32, i64::from(operand));
                self.f.ins().store(flags(), to, slot, 0);
                let n1 = self.f.ins().iadd_imm(n, 1);
                st(self.f, types::I64, n1, ctx, at!(nreturns));
                self.next(pc);
            }
            RETURN => {
                // One dispatch block for every `RETURN` (`Step::Return`).
                let n = ld(self.f, types::I64, ctx, at!(nreturns));
                let some = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
                self.check(some, pc, R_NO_CALL);
                let n1 = self.f.ins().iadd_imm(n, -1);
                st(self.f, types::I64, n1, ctx, at!(nreturns));
                let r = ld(self.f, types::I64, ctx, at!(returns));
                let slot = elem(self.f, r, n1, 4);
                let to = self.f.ins().load(types::I32, flags(), slot, 0);
                let dispatch = self.f.create_block();
                self.heads.push((dispatch, Step::Return));
                self.f.ins().jump(dispatch, &[]);
                self.f.switch_to_block(dispatch);
                let bad = self.f.create_block();
                let targets: Vec<BlockCall> = self.blocks.iter().map(|&b| self.f.func.dfg.block_call(b, &[])).collect();
                let default = self.f.func.dfg.block_call(bad, &[]);
                let jt = self.f.create_jump_table(JumpTableData::new(default, &targets));
                self.f.ins().br_table(to, jt);
                self.f.switch_to_block(bad);
                self.refuse(pc, R_NO_CALL);
            }
            READ => self.read(pc),
            IS => {
                let held = match mode {
                    0 => {
                        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
                        self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(operand))
                    }
                    1 => {
                        let s = self.strings[usize::from(operand)].clone();
                        self.spelled(&s)
                    }
                    3 => {
                        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
                        let fl = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, flags) as i32);
                        let ff = ldu(self.f, types::I8, ctx, at!(from_file));
                        let hash = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(HASH));
                        let bol = self.f.ins().band_imm(fl, i64::from(BOL));
                        let bol = self.f.ins().icmp_imm(IntCC::NotEqual, bol, 0);
                        let x = self.f.ins().band(hash, bol);
                        let ff = self.f.ins().ireduce(types::I8, ff);
                        self.f.ins().band(x, ff)
                    }
                    _ => {
                        let x = ldu(self.f, types::I8, ctx, at!(in_if));
                        self.f.ins().icmp_imm(IntCC::NotEqual, x, 0)
                    }
                };
                self.test(pc, held);
            }
            PUT => {
                let n = ld(self.f, types::I64, ctx, at!(nout));
                let room = ld(self.f, types::I64, ctx, at!(room_out));
                let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, n, room);
                self.check(ok, pc, R_OUTPUT);
                let out = ld(self.f, types::I64, ctx, at!(out));
                let p = elem(self.f, out, n, TOK);
                copy_tok(self.f, ctx, at!(t), p, 0);
                let n1 = self.f.ins().iadd_imm(n, 1);
                st(self.f, types::I64, n1, ctx, at!(nout));
                self.next(pc);
            }
            _ => return Err(Error::RefusedShape(format!("the preprocessing word {op:#04x} m={mode} at {pc} is not lowered yet"))),
        }
        Ok(())
    }

    /// Whether `T` is spelled `s`: its length, then its bytes, unrolled (`s` is the program's).
    fn spelled(&mut self, s: &[u8]) -> Value {
        let (p, n) = self.text();
        let no = self.f.create_block();
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        let len_ok = self.f.ins().icmp_imm(IntCC::Equal, n, s.len() as i64);
        let mut go = self.f.create_block();
        self.f.ins().brif(len_ok, go, &[], no, &[]);
        for (i, &c) in s.iter().enumerate() {
            self.f.switch_to_block(go);
            let b = ldu(self.f, types::I8, p, i as i32);
            let eq = self.f.ins().icmp_imm(IntCC::Equal, b, i64::from(c));
            go = self.f.create_block();
            self.f.ins().brif(eq, go, &[], no, &[]);
        }
        self.f.switch_to_block(go);
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[one.into()]);
        self.f.switch_to_block(no);
        let zero = self.f.ins().iconst(types::I8, 0);
        self.f.ins().jump(join, &[zero.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// `READ`: the top source's next token (`Machine::read`). A source at its end is popped
    /// (`Step::Pop`) unless it is read again and again (a stop source, the main file, a file
    /// while arguments are read); a file's conditionals go with it, and its macro is enabled
    /// again.
    fn read(&mut self, pc: usize) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        let f = &mut *self.f;
        let top = f.create_block();
        self.heads.push((top, Step::Pop));
        let eof = f.create_block();
        f.append_block_param(eof, types::I64);
        f.ins().jump(top, &[]);

        // The top source.
        f.switch_to_block(top);
        let n = ld(f, types::I64, ctx, at!(nsources));
        let some = f.create_block();
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().brif(n, some, &[], eof, &[zero.into()]);
        f.switch_to_block(some);
        let srcs = ld(f, types::I64, ctx, at!(sources));
        let i = f.ins().iadd_imm(n, -1);
        let s = elem(f, srcs, i, SRC);
        let pos = ldu(f, types::I32, s, offset_of!(Src, pos) as i32);
        let end = ldu(f, types::I32, s, offset_of!(Src, end) as i32);
        let file = ldu(f, types::I32, s, offset_of!(Src, file) as i32);
        let in_file = f.ins().icmp_imm(IntCC::NotEqual, file, i64::from(NONE));
        let has = f.ins().icmp(IntCC::UnsignedLessThan, pos, end);
        let take = f.create_block();
        let ended = f.create_block();
        f.ins().brif(has, take, &[], ended, &[]);

        // A token: the arena's or the file's.
        f.switch_to_block(take);
        let from_file = f.create_block();
        let from_arena = f.create_block();
        let got = f.create_block();
        f.append_block_param(got, types::I64);
        f.ins().brif(in_file, from_file, &[], from_arena, &[]);
        f.switch_to_block(from_arena);
        let arena = ld(f, types::I64, ctx, at!(arena));
        let p = elem(f, arena, pos, TOK);
        f.ins().jump(got, &[p.into()]);
        f.switch_to_block(from_file);
        let files = ld(f, types::I64, ctx, at!(files));
        let fp = elem(f, files, file, FILE);
        let toks = ld(f, types::I64, fp, offset_of!(FileC, toks) as i32);
        let p = elem(f, toks, pos, TOK);
        f.ins().jump(got, &[p.into()]);
        f.switch_to_block(got);
        let p = f.block_params(got)[0];
        copy_tok(f, p, 0, ctx, at!(t));
        st(f, types::I8, in_file, ctx, at!(from_file));
        // A file's `#` at a line's start: the directive's place.
        let kind = ldu(f, types::I8, p, offset_of!(Tok, kind) as i32);
        let fl = ldu(f, types::I8, p, offset_of!(Tok, flags) as i32);
        let hash = f.ins().icmp_imm(IntCC::Equal, kind, i64::from(HASH));
        let bol = f.ins().band_imm(fl, i64::from(BOL));
        let bol = f.ins().icmp_imm(IntCC::NotEqual, bol, 0);
        let mark = f.ins().band(hash, bol);
        let mark = f.ins().band(mark, in_file);
        let marked = f.create_block();
        let moved = f.create_block();
        f.ins().brif(mark, marked, &[], moved, &[]);
        f.switch_to_block(marked);
        st(f, types::I32, pos, s, offset_of!(Src, hash) as i32);
        f.ins().jump(moved, &[]);
        f.switch_to_block(moved);
        let pos1 = f.ins().iadd_imm(pos, 1);
        st(f, types::I32, pos1, s, offset_of!(Src, pos) as i32);
        f.ins().jump(next, &[]);

        // At its end: read again and again, or popped.
        f.switch_to_block(ended);
        let stop = ldu(f, types::I8, s, offset_of!(Src, stop) as i32);
        let collecting = ld(f, types::I64, ctx, at!(collecting));
        let main = f.ins().icmp_imm(IntCC::Equal, n, 1);
        let held = f.ins().icmp_imm(IntCC::NotEqual, collecting, 0);
        let keep = f.ins().bor(main, held);
        let keep = f.ins().band(keep, in_file);
        let stopped = f.ins().icmp_imm(IntCC::NotEqual, stop, 0);
        let keep = f.ins().bor(keep, stopped);
        let pop = f.create_block();
        let again = f.create_block();
        f.ins().brif(keep, again, &[], pop, &[]);
        f.switch_to_block(again);
        let sflag = f.ins().iconst(types::I64, i64::from(pp_words::STOP));
        let zero = f.ins().iconst(types::I64, 0);
        let fl = f.ins().select(stopped, sflag, zero);
        f.ins().jump(eof, &[fl.into()]);

        f.switch_to_block(pop);
        st(f, types::I64, i, ctx, at!(nsources));
        let conds = f.create_block();
        self.heads.push((conds, Step::Pop));
        let enable = f.create_block();
        f.ins().brif(in_file, conds, &[], enable, &[]);
        // A file's conditionals end with it.
        f.switch_to_block(conds);
        let nc = ld(f, types::I64, ctx, at!(nconds));
        let any = f.create_block();
        f.ins().brif(nc, any, &[], enable, &[]);
        f.switch_to_block(any);
        let cs = ld(f, types::I64, ctx, at!(conds));
        let c1 = f.ins().iadd_imm(nc, -1);
        let c = elem(f, cs, c1, COND_SIZE as i64);
        let src = ldu(f, types::I32, c, COND_SRC as i32);
        let mine = f.ins().icmp(IntCC::Equal, src, i);
        let drop = f.create_block();
        f.ins().brif(mine, drop, &[], enable, &[]);
        f.switch_to_block(drop);
        st(f, types::I64, c1, ctx, at!(nconds));
        f.ins().jump(conds, &[]);
        // Its macro enabled again.
        f.switch_to_block(enable);
        let def = ldu(f, types::I32, s, offset_of!(Src, def) as i32);
        let has_def = f.ins().icmp_imm(IntCC::NotEqual, def, i64::from(NONE));
        let on = f.create_block();
        f.ins().brif(has_def, on, &[], top, &[]);
        f.switch_to_block(on);
        let defs = ld(f, types::I64, ctx, at!(defs));
        let d = elem(f, defs, def, DEF);
        let z = f.ins().iconst(types::I8, 0);
        f.ins().store(flags(), z, d, offset_of!(Def, disabled) as i32);
        f.ins().jump(top, &[]);

        // `T` an end of input: `EOF`, its flags given.
        f.switch_to_block(eof);
        let fl = f.block_params(eof)[0];
        let k = f.ins().iconst(types::I8, i64::from(EOF));
        f.ins().store(flags(), k, ctx, at!(t) + offset_of!(Tok, kind) as i32);
        st(f, types::I8, fl, ctx, at!(t) + offset_of!(Tok, flags) as i32);
        let z = f.ins().iconst(types::I32, 0);
        for off in [offset_of!(Tok, buf), offset_of!(Tok, start), offset_of!(Tok, end), offset_of!(Tok, exp)] {
            f.ins().store(flags(), z, ctx, at!(t) + off as i32);
        }
        let z8 = f.ins().iconst(types::I8, 0);
        f.ins().store(flags(), z8, ctx, at!(from_file));
        f.ins().jump(next, &[]);
    }
}

/// Compile `program`'s words (validated first).
pub fn compile(program: &Words) -> Result<CompiledPp, Error> {
    pp_words::validate(program)?;
    let builder = JITBuilder::with_flags(&[("opt_level", "speed")], default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("pp_run", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
    let mut context = module.make_context();
    context.func.signature = signature;
    let mut fctx = FunctionBuilderContext::new();
    let heads;
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let blocks: Vec<Block> = program.words.iter().map(|_| f.create_block()).collect();
        let exit = f.create_block();
        f.append_block_param(exit, types::I64);
        f.switch_to_block(entry);
        let ctx = f.block_params(entry)[0];
        f.ins().jump(blocks[0], &[]);
        let mut l = Lower { f: &mut f, ctx, words: &program.words, strings: &program.strings, blocks, exit, heads: Vec::new() };
        for pc in 0..program.words.len() {
            let b = l.blocks[pc];
            l.f.switch_to_block(b);
            // A token read is a stated step: a cycle through `READ`'s block.
            if split(l.words[pc]).0 == READ {
                l.heads.push((b, Step::Token));
            }
            l.word(pc)?;
        }
        l.f.switch_to_block(exit);
        let r = l.f.block_params(exit)[0];
        l.f.ins().return_(&[r]);
        heads = std::mem::take(&mut l.heads);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::native_cpu_shape::check(&context.func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    let clif = context.func.display().to_string();
    module.define_function(id, &mut context).map_err(|e| Error::RefusedShape(format!("pp_run: {e:?}")))?;
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(id);
    // SAFETY: `pp_run` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, unsafe extern "C" fn(*mut Ctx) -> u64>(code) };
    Ok(CompiledPp { _module: module, entry, clif, heads: heads.iter().map(|h| h.1).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pp_words::{BOL, IDENT, PUNCT, SPACE};

    /// A file of tokens over `text`: each a word, its kind and flags.
    fn file<'a>(text: &'a [u8], toks: &'a [Tok]) -> File<'a> {
        File { path: b"main.c", bytes: text, lines: &[], toks, chains: &[], includes: &[], named: &[] }
    }

    fn tok(kind: u8, flags: u8, start: u32, end: u32) -> Tok {
        Tok { kind, flags, buf: 1, start, end, exp: 0 }
    }

    fn program(words: &str) -> Words {
        crate::rir_asm::assemble(&format!("program - ir=7 grammar=none\n  string 0 \"b\"\n{words}")).unwrap()
    }

    /// The JIT gives what the interpreter gives, for a program and a file.
    fn same(words: &str) {
        let p = program(words);
        let text = b"a b\n# c b";
        let toks = [tok(IDENT, BOL, 0, 1), tok(IDENT, SPACE, 2, 3), tok(HASH, BOL, 4, 5), tok(IDENT, SPACE, 6, 7), tok(IDENT, SPACE, 8, 9), tok(PUNCT, SPACE, 8, 9)];
        let files = [file(text, &toks)];
        let bounds = Bounds::of(&files);
        let env = Env::DEFAULT;
        let want = pp_words::run(&p, &files, &env, &bounds);
        let c = compile(&p).unwrap();
        let got = c.run(&p, &files, &env, &bounds);
        match (want, got) {
            (Ok(w), Ok(g)) => assert_eq!(w.toks, g.toks),
            (Err(w), Err(g)) => assert_eq!((w.pc, w.why), (g.pc, g.why)),
            (w, g) => panic!("interpreter {:?}, JIT {:?}", w.map(|e| e.toks), g.map(|e| e.toks)),
        }
    }

    #[test]
    fn a_copy_loop_is_the_interpreters() {
        same("  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 PUT 0\n  0004 GOTO @0000\n");
    }

    #[test]
    fn a_test_by_spelling_and_a_call_are_the_interpreters() {
        // Each `b` put twice (a call that puts `T`), others once; `#` at a line's start dropped.
        same(concat!(
            "  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 IS m=3 0\n  0004 GOTO @0000\n",
            "  0005 IS m=1 s0\n  0006 GOTO @0009\n  0007 PUT 0\n  0008 GOTO @0000\n",
            "  0009 PUSH @0011\n  0010 GOTO @0013\n  0011 PUT 0\n  0012 GOTO @0000\n",
            "  0013 PUT 0\n  0014 RETURN 0\n",
        ));
    }

    #[test]
    fn the_shape_holds() {
        let c = compile(&program("  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 PUT 0\n  0004 GOTO @0000\n")).unwrap();
        assert!(!c.clif.contains(" call "), "no calls out");
        assert!(c.heads.contains(&Step::Token) && c.heads.contains(&Step::Pop));
    }
}
