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
use crate::pp_words::{self, BOL, Bounds, COND_SIZE, COND_SRC, Ctx, Def, EOF, Env, Expanded, File, FileC, Frame, HASH, IDENT, LEVEL, NOEXP, NONE, PAIR, Refused, SPACE, Slot, Src, Tok, USE};
use crate::program::{ACCEPT, ARG, FRAME, GOTO, IS, KEEP, LINE, LOOKUP, MACRO, PAINT, PEEK, PRE, PUSH, PUT, READ, RETURN, SKIPLINE, STOP, Words, split};
use cranelift_codegen::ir::{AbiParam, Block, BlockCall, InstBuilder, JumpTableData, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use std::mem::{offset_of, size_of};

/// `Ctx`'s field `$f`'s offset.
macro_rules! at {
    ($f:ident) => {
        offset_of!(Ctx, $f) as i32
    };
}

/// Why the compiled words refuse a TU: the index the code returns above bit 32 (`run_native`),
/// each the interpreter's reason.
const REASONS: &[&str] = &[
    "the compiled words refuse",
    "the program stopped",
    "past the calls' bound",
    "a return with no call",
    "past the output's bound",
    "a directive with no file",
    "a directive's line not in a file",
    "past the arena's bound",
    "past the macro table's bound",
    "past the frames' bound",
    "an argument with no frame",
    "past the sources' bound",
    "no level open",
    "a level's source not on top",
    "a level with no frame",
    "an expansion with no frame",
    "a word not lowered yet",
];
const R_STOPPED: u64 = 1;
const R_CALLS: u64 = 2;
const R_NO_CALL: u64 = 3;
const R_OUTPUT: u64 = 4;
const R_NO_FILE: u64 = 5;
const R_NOT_FILE: u64 = 6;
const R_ARENA: u64 = 7;
const R_TABLE: u64 = 8;
const R_FRAMES: u64 = 9;
const R_ARG_FRAME: u64 = 10;
const R_SOURCES: u64 = 11;
const R_NO_LEVEL: u64 = 12;
const R_LEVEL_SRC: u64 = 13;
const R_LEVEL_FRAME: u64 = 14;
const R_PRE_FRAME: u64 = 15;
/// `compile_partial`'s refusal at a word it has no code for.
pub const NOT_LOWERED: &str = "a word not lowered yet";
const R_NOT_LOWERED: u64 = 16;

/// One of the machine's buffers in `Ctx`: its start's, length's and room's fields, its
/// element's size, and why a push past its room refuses.
#[derive(Clone, Copy)]
struct V {
    ptr: i32,
    len: i32,
    room: i32,
    size: i64,
    full: u64,
}

macro_rules! v {
    ($p:ident, $n:ident, $r:ident, $size:expr, $full:expr) => {
        V { ptr: at!($p), len: at!($n), room: at!($r), size: $size as i64, full: $full }
    };
}

const SOURCES: V = v!(sources, nsources, room_sources, size_of::<Src>(), R_SOURCES);
const ARENA: V = v!(arena, narena, room_arena, size_of::<Tok>(), R_ARENA);
const KEPT: V = v!(kept, nkept, room_kept, size_of::<Tok>(), R_ARENA);
const FRAMES: V = v!(frames, nframes, room_frames, size_of::<Frame>(), R_FRAMES);
const ARGV: V = v!(argv, nargv, room_argv, PAIR.0, R_ARENA);
const PRES: V = v!(pres, npres, room_pres, PAIR.0, R_ARENA);
const USES: V = v!(uses, nuses, room_uses, USE.0, R_ARENA);
const LEVELS: V = v!(levels, nlevels, room_levels, LEVEL.0, R_SOURCES);

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

const TOK: i64 = size_of::<Tok>() as i64;
const SRC: i64 = size_of::<Src>() as i64;
const FILE: i64 = size_of::<FileC>() as i64;
const DEF: i64 = size_of::<Def>() as i64;
const SLOT: i64 = size_of::<Slot>() as i64;

/// `pp_words::key`'s constants.
const KEY_SEED: i64 = 0x9e37_79b9_7f4a_7c15_u64 as i64;
const KEY_MUL: i64 = 0xff51_afd7_ed55_8ccd_u64 as i64;

/// A load of bytes at any address (a spelling's).
fn bytes_flags() -> MemFlags {
    MemFlags::new().with_notrap()
}

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
    /// A word not lowered refuses the TU when it runs (`NOT_LOWERED`), not the compile.
    partial: bool,
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
        let ctx = self.ctx;
        self.spelling(ctx, at!(t))
    }

    /// The spelling of the token at `tok + off`: scratch's bytes (`buf` 0) or a file's.
    fn spelling(&mut self, tok: Value, off: i32) -> (Value, Value) {
        let (ctx, f) = (self.ctx, &mut *self.f);
        let buf = ldu(f, types::I32, tok, off + offset_of!(Tok, buf) as i32);
        let start = ldu(f, types::I32, tok, off + offset_of!(Tok, start) as i32);
        let end = ldu(f, types::I32, tok, off + offset_of!(Tok, end) as i32);
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

    fn vlen(&mut self, v: V) -> Value {
        ld(self.f, types::I64, self.ctx, v.len)
    }

    fn vset_len(&mut self, v: V, n: Value) {
        st(self.f, types::I64, n, self.ctx, v.len);
    }

    /// Element `i`'s address.
    fn vat(&mut self, v: V, i: Value) -> Value {
        let base = ld(self.f, types::I64, self.ctx, v.ptr);
        elem(self.f, base, i, v.size)
    }

    /// The last element's address, refusing for `why` where there is none.
    fn vtop(&mut self, v: V, pc: usize, why: u64) -> Value {
        let n = self.vlen(v);
        self.check(n, pc, why);
        let i = self.f.ins().iadd_imm(n, -1);
        self.vat(v, i)
    }

    /// Room for one more (refusing past it): the new element's address, the length one more.
    fn vpush(&mut self, v: V, pc: usize) -> Value {
        let n = self.vlen(v);
        let room = ld(self.f, types::I64, self.ctx, v.room);
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, n, room);
        self.check(ok, pc, v.full);
        let n1 = self.f.ins().iadd_imm(n, 1);
        self.vset_len(v, n1);
        self.vat(v, n)
    }

    /// A pair `(a, b)` of `u32`s stored at `p` (`PAIR`'s layout).
    fn put_pair(&mut self, p: Value, a: Value, b: Value) {
        st(self.f, types::I32, a, p, PAIR.1 as i32);
        st(self.f, types::I32, b, p, PAIR.2 as i32);
    }

    fn pair(&mut self, p: Value) -> (Value, Value) {
        (ldu(self.f, types::I32, p, PAIR.1 as i32), ldu(self.f, types::I32, p, PAIR.2 as i32))
    }

    /// `Src::arena(a, b, def, stop)` pushed (refusing past the sources' bound).
    fn push_arena_src(&mut self, pc: usize, a: Value, b: Value, def: u32, stop: bool) {
        let s = self.vpush(SOURCES, pc);
        let f = &mut *self.f;
        let none = f.ins().iconst(types::I32, i64::from(NONE));
        let z = f.ins().iconst(types::I32, 0);
        let d = f.ins().iconst(types::I32, i64::from(def));
        let stop = f.ins().iconst(types::I8, i64::from(stop));
        f.ins().store(flags(), none, s, offset_of!(Src, file) as i32);
        st(f, types::I32, a, s, offset_of!(Src, pos) as i32);
        st(f, types::I32, b, s, offset_of!(Src, end) as i32);
        f.ins().store(flags(), d, s, offset_of!(Src, def) as i32);
        f.ins().store(flags(), stop, s, offset_of!(Src, stop) as i32);
        f.ins().store(flags(), none, s, offset_of!(Src, hash) as i32);
        f.ins().store(flags(), z, s, offset_of!(Src, delta) as i32);
        let p = offset_of!(Src, presumed);
        f.ins().store(flags(), none, s, (p + offset_of!((u32, u32), 0)) as i32);
        f.ins().store(flags(), none, s, (p + offset_of!((u32, u32), 1)) as i32);
    }

    /// The token at `from` (its offset `off`) pushed to the arena; its new length.
    fn push_arena_tok(&mut self, pc: usize, from: Value, off: i32) -> Value {
        let p = self.vpush(ARENA, pc);
        copy_tok(self.f, from, off, p, 0);
        self.vlen(ARENA)
    }

    /// `Machine::append`: `T` to the open argument (a line's first token, with space before it).
    fn append(&mut self, pc: usize) {
        let ctx = self.ctx;
        let p = self.vpush(ARENA, pc);
        copy_tok(self.f, ctx, at!(t), p, 0);
        let fo = offset_of!(Tok, flags) as i32;
        let fl = ldu(self.f, types::I8, p, fo);
        let bol = self.f.ins().band_imm(fl, i64::from(BOL));
        let cleared = self.f.ins().band_imm(fl, !i64::from(BOL));
        let spaced = self.f.ins().bor_imm(cleared, i64::from(SPACE));
        let fl = self.f.ins().select(bol, spaced, fl);
        st(self.f, types::I8, fl, p, fo);
        let n = self.vlen(ARENA);
        let a = self.vtop(ARGV, pc, R_ARG_FRAME);
        st(self.f, types::I32, n, a, PAIR.2 as i32);
    }

    /// `Machine::close_args`: the top frame's arguments read.
    fn close_args(&mut self, fr: Value) {
        let ctx = self.ctx;
        let oo = offset_of!(Frame, open) as i32;
        let open = ldu(self.f, types::I8, fr, oo);
        let on = self.f.create_block();
        let done = self.f.create_block();
        self.f.ins().brif(open, on, &[], done, &[]);
        self.f.switch_to_block(on);
        let z = self.f.ins().iconst(types::I8, 0);
        self.f.ins().store(flags(), z, fr, oo);
        let c = ld(self.f, types::I64, ctx, at!(collecting));
        let c = self.f.ins().iadd_imm(c, -1);
        st(self.f, types::I64, c, ctx, at!(collecting));
        self.f.ins().jump(done, &[]);
        self.f.switch_to_block(done);
    }

    /// `Machine::drop_frame`: the top frame (at `fr`, of `n` frames) and its arguments gone.
    fn drop_frame(&mut self, fr: Value) {
        let n = self.vlen(FRAMES);
        let n1 = self.f.ins().iadd_imm(n, -1);
        self.vset_len(FRAMES, n1);
        let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
        let pres = ldu(self.f, types::I32, fr, offset_of!(Frame, pres) as i32);
        self.vset_len(ARGV, args);
        self.vset_len(PRES, pres);
    }

    /// The definition of the frame at `fr`.
    fn frame_def(&mut self, fr: Value) -> Value {
        let d = ldu(self.f, types::I32, fr, offset_of!(Frame, def) as i32);
        let defs = ld(self.f, types::I64, self.ctx, at!(defs));
        elem(self.f, defs, d, DEF)
    }

    /// `FRAME` (`Machine::frame`): a call of `d` opened.
    fn frame(&mut self, pc: usize) {
        let ctx = self.ctx;
        let nargv = self.vlen(ARGV);
        let npres = self.vlen(PRES);
        let fr = self.vpush(FRAMES, pc);
        let d = ld(self.f, types::I64, ctx, at!(d));
        let defs = ld(self.f, types::I64, ctx, at!(defs));
        let def = elem(self.f, defs, d, DEF);
        let fnlike = ldu(self.f, types::I8, def, offset_of!(Def, fnlike) as i32);
        let params = ldu(self.f, types::I16, def, offset_of!(Def, params) as i32);
        let f = &mut *self.f;
        st(f, types::I32, d, fr, offset_of!(Frame, def) as i32);
        copy_tok(f, ctx, at!(t), fr, offset_of!(Frame, name) as i32);
        copy_tok(f, ctx, at!(t), fr, offset_of!(Frame, end) as i32);
        st(f, types::I32, nargv, fr, offset_of!(Frame, args) as i32);
        st(f, types::I32, npres, fr, offset_of!(Frame, pres) as i32);
        let z = f.ins().iconst(types::I32, 0);
        f.ins().store(flags(), z, fr, offset_of!(Frame, depth) as i32);
        f.ins().store(flags(), z, fr, offset_of!(Frame, next_use) as i32);
        st(f, types::I8, fnlike, fr, offset_of!(Frame, open) as i32);
        let c = ld(f, types::I64, ctx, at!(collecting));
        let c = f.ins().iadd(c, fnlike);
        st(f, types::I64, c, ctx, at!(collecting));
        // Its expansions, none made yet: a parameter a step (`Step::Item`).
        let end = f.ins().iadd(npres, params);
        let room = ld(f, types::I64, ctx, at!(room_pres));
        let ok = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(ok, pc, R_ARENA);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let done = self.f.create_block();
        self.f.ins().jump(head, &[npres.into()]);
        self.f.switch_to_block(head);
        let i = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, end);
        let body = self.f.create_block();
        self.f.ins().brif(more, body, &[], done, &[]);
        self.f.switch_to_block(body);
        let p = self.vat(PRES, i);
        let none = self.f.ins().iconst(types::I64, i64::from(NONE));
        self.put_pair(p, none, none);
        let i1 = self.f.ins().iadd_imm(i, 1);
        self.f.ins().jump(head, &[i1.into()]);
        self.f.switch_to_block(done);
        self.vset_len(PRES, end);
        // A function-like macro's first argument, empty.
        let open = self.f.create_block();
        let next = self.blocks[pc + 1];
        self.f.ins().brif(fnlike, open, &[], next, &[]);
        self.f.switch_to_block(open);
        let a = self.vlen(ARENA);
        let p = self.vpush(ARGV, pc);
        self.put_pair(p, a, a);
        self.f.ins().jump(next, &[]);
    }

    /// `ARG` (`Machine::arg`), its modes.
    fn arg(&mut self, pc: usize, mode: u8) {
        let ctx = self.ctx;
        let fr = self.vtop(FRAMES, pc, R_ARG_FRAME);
        let def = self.frame_def(fr);
        let next = self.blocks[pc + 1];
        let depth_at = offset_of!(Frame, depth) as i32;
        match mode {
            0 | 1 => {
                if mode == 1 {
                    let d = ldu(self.f, types::I32, fr, depth_at);
                    let d = self.f.ins().iadd_imm(d, 1);
                    st(self.f, types::I32, d, fr, depth_at);
                }
                self.append(pc);
                self.f.ins().jump(next, &[]);
            }
            2 => {
                // A `)`: the call's end at depth 0 (held), else an argument's token.
                let d = ldu(self.f, types::I32, fr, depth_at);
                let end = self.f.create_block();
                let inner = self.f.create_block();
                self.f.ins().brif(d, inner, &[], end, &[]);
                self.f.switch_to_block(end);
                copy_tok(self.f, ctx, at!(t), fr, offset_of!(Frame, end) as i32);
                self.close_args(fr);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(inner);
                let d1 = self.f.ins().iadd_imm(d, -1);
                st(self.f, types::I32, d1, fr, depth_at);
                self.append(pc);
                let skip = self.blocks[pc + 2];
                self.f.ins().jump(skip, &[]);
            }
            3 => {
                // A `,`: the next argument, unless nested or in the variadic one.
                let d = ldu(self.f, types::I32, fr, depth_at);
                let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
                let nargv = self.vlen(ARGV);
                let nargs = self.f.ins().isub(nargv, args);
                let variadic = ldu(self.f, types::I8, def, offset_of!(Def, variadic) as i32);
                let params = ldu(self.f, types::I16, def, offset_of!(Def, params) as i32);
                let last = self.f.ins().icmp(IntCC::Equal, nargs, params);
                let va = self.f.ins().icmp_imm(IntCC::NotEqual, variadic, 0);
                let in_va = self.f.ins().band(va, last);
                let top = self.f.ins().icmp_imm(IntCC::Equal, d, 0);
                let not_va = self.f.ins().bxor_imm(in_va, 1);
                let split_here = self.f.ins().band(top, not_va);
                let new = self.f.create_block();
                let tok = self.f.create_block();
                self.f.ins().brif(split_here, new, &[], tok, &[]);
                self.f.switch_to_block(new);
                let a = self.vlen(ARENA);
                let p = self.vpush(ARGV, pc);
                self.put_pair(p, a, a);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(tok);
                self.append(pc);
                self.f.ins().jump(next, &[]);
            }
            4 => {
                // The count of arguments checked (held), else the call is its name alone.
                let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
                let nargv = self.vlen(ARGV);
                let nargs = self.f.ins().isub(nargv, args);
                let variadic = ldu(self.f, types::I8, def, offset_of!(Def, variadic) as i32);
                let params = ldu(self.f, types::I16, def, offset_of!(Def, params) as i32);
                let none = self.f.create_block();
                let some = self.f.create_block();
                let good = self.f.create_block();
                self.f.append_block_param(good, types::I8);
                let bad = self.f.create_block();
                self.f.ins().brif(params, some, &[], none, &[]);
                // No parameters: one argument, empty, and it is dropped.
                self.f.switch_to_block(none);
                let a0 = self.vat(ARGV, args);
                let (a, b) = self.pair(a0);
                let one = self.f.ins().icmp_imm(IntCC::Equal, nargs, 1);
                let empty = self.f.ins().icmp(IntCC::Equal, a, b);
                let ok = self.f.ins().band(one, empty);
                let drop = self.f.ins().iconst(types::I8, 0);
                self.f.ins().brif(ok, good, &[drop.into()], bad, &[]);
                // The variadic arguments may be left out.
                self.f.switch_to_block(some);
                let exact = self.f.ins().icmp(IntCC::Equal, nargs, params);
                let n1 = self.f.ins().iadd_imm(nargs, 1);
                let short = self.f.ins().icmp(IntCC::Equal, n1, params);
                let va = self.f.ins().icmp_imm(IntCC::NotEqual, variadic, 0);
                let left_out = self.f.ins().band(short, va);
                let ok = self.f.ins().bor(exact, left_out);
                self.f.ins().brif(ok, good, &[left_out.into()], bad, &[]);
                self.f.switch_to_block(good);
                let add = self.f.block_params(good)[0];
                let drop_all = self.f.create_block();
                let push = self.f.create_block();
                let done = self.f.create_block();
                let has = self.f.create_block();
                self.f.ins().brif(params, has, &[], drop_all, &[]);
                self.f.switch_to_block(drop_all);
                self.vset_len(ARGV, args);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(has);
                self.f.ins().brif(add, push, &[], done, &[]);
                self.f.switch_to_block(push);
                let a = self.vlen(ARENA);
                let p = self.vpush(ARGV, pc);
                self.put_pair(p, a, a);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(done);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(bad);
                self.drop_frame(fr);
                copy_tok(self.f, fr, offset_of!(Frame, name) as i32, ctx, at!(t));
                let fo = at!(t) + offset_of!(Tok, flags) as i32;
                let fl = ld(self.f, types::I8, ctx, fo);
                let fl = self.f.ins().bor_imm(fl, i64::from(NOEXP));
                self.f.ins().store(flags(), fl, ctx, fo);
                let skip = self.blocks[pc + 2];
                self.f.ins().jump(skip, &[]);
            }
            _ => {
                self.close_args(fr);
                self.drop_frame(fr);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    /// Whether the variadic argument of the frame at `fr` (its definition at `def`) has tokens
    /// once expanded (`Machine::va_present`).
    fn va_present(&mut self, fr: Value, def: Value) -> Value {
        let params = ldu(self.f, types::I16, def, offset_of!(Def, params) as i32);
        let va = self.f.ins().iadd_imm(params, -1);
        let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
        let pres = ldu(self.f, types::I32, fr, offset_of!(Frame, pres) as i32);
        let ai = self.f.ins().iadd(args, va);
        let pi = self.f.ins().iadd(pres, va);
        let ap = self.vat(ARGV, ai);
        let (a, b) = self.pair(ap);
        let pp = self.vat(PRES, pi);
        let (x, y) = self.pair(pp);
        let f = &mut *self.f;
        let has = f.ins().icmp(IntCC::NotEqual, a, b);
        let made = f.ins().icmp_imm(IntCC::NotEqual, x, i64::from(NONE));
        let some = f.ins().icmp(IntCC::NotEqual, x, y);
        let r = f.ins().band(has, made);
        f.ins().band(r, some)
    }

    /// `PRE m=0` (`Machine::pre`), a test: the next argument a use asks for, not expanded yet,
    /// pushed to be expanded (a stop source, a level); the uses walked a step at a time
    /// (`Step::Item`).
    fn pre(&mut self, pc: usize) {
        let ctx = self.ctx;
        let fr = self.vtop(FRAMES, pc, R_PRE_FRAME);
        let def = self.frame_def(fr);
        let pre0 = ldu(self.f, types::I32, def, (offset_of!(Def, pre) + offset_of!((u32, u32), 0)) as i32);
        let pre1 = ldu(self.f, types::I32, def, (offset_of!(Def, pre) + offset_of!((u32, u32), 1)) as i32);
        let nu = offset_of!(Frame, next_use) as i32;
        let u0 = ldu(self.f, types::I32, fr, nu);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        self.f.ins().jump(head, &[u0.into()]);
        self.f.switch_to_block(head);
        let u = self.f.block_params(head)[0];
        let k = self.f.ins().iadd(pre0, u);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, k, pre1);
        let body = self.f.create_block();
        let none = self.f.create_block();
        self.f.ins().brif(more, body, &[], none, &[]);
        self.f.switch_to_block(none);
        st(self.f, types::I32, u, fr, nu);
        let skip = self.blocks[pc + 2];
        self.f.ins().jump(skip, &[]);

        self.f.switch_to_block(body);
        let up = self.vat(USES, k);
        let p = ldu(self.f, types::I16, up, USE.1 as i32);
        let inside = ldu(self.f, types::I8, up, USE.2 as i32);
        let u1 = self.f.ins().iadd_imm(u, 1);
        let pres = ldu(self.f, types::I32, fr, offset_of!(Frame, pres) as i32);
        let pi = self.f.ins().iadd(pres, p);
        let pp = self.vat(PRES, pi);
        let (x, _) = self.pair(pp);
        let made = self.f.ins().icmp_imm(IntCC::NotEqual, x, i64::from(NONE));
        let on = self.f.create_block();
        let check_va = self.f.create_block();
        self.f.ins().brif(made, head, &[u1.into()], check_va, &[]);
        self.f.switch_to_block(check_va);
        let va_test = self.f.create_block();
        self.f.ins().brif(inside, va_test, &[], on, &[]);
        self.f.switch_to_block(va_test);
        let present = self.va_present(fr, def);
        self.f.ins().brif(present, on, &[], head, &[u1.into()]);
        self.f.switch_to_block(on);
        let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
        let ai = self.f.ins().iadd(args, p);
        let ap = self.vat(ARGV, ai);
        let (a, b) = self.pair(ap);
        let empty = self.f.ins().icmp(IntCC::Equal, a, b);
        let fill = self.f.create_block();
        let push = self.f.create_block();
        self.f.ins().brif(empty, fill, &[], push, &[]);
        // An empty argument's expansion is empty.
        self.f.switch_to_block(fill);
        self.put_pair(pp, b, b);
        self.f.ins().jump(head, &[u1.into()]);
        self.f.switch_to_block(push);
        st(self.f, types::I32, u1, fr, nu);
        let ns = self.vlen(SOURCES);
        let nl = self.vlen(LEVELS);
        let room = ld(self.f, types::I64, ctx, at!(room_sources));
        let s_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, ns, room);
        let l_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, nl, room);
        let ok = self.f.ins().band(s_ok, l_ok);
        self.check(ok, pc, R_SOURCES);
        self.push_arena_src(pc, a, b, NONE, true);
        let nk = self.vlen(KEPT);
        let lp = self.vpush(LEVELS, pc);
        st(self.f, types::I16, p, lp, LEVEL.1 as i32);
        st(self.f, types::I32, nk, lp, LEVEL.2 as i32);
        let next = self.blocks[pc + 1];
        self.f.ins().jump(next, &[]);
    }

    /// `PRE m=1` (`Machine::pre_end`): the level closed, its tokens copied from `kept` to the
    /// arena a token a step (`Step::Item`), the argument's expansion their range.
    fn pre_end(&mut self, pc: usize) {
        let lp = self.vtop(LEVELS, pc, R_NO_LEVEL);
        let p = ldu(self.f, types::I16, lp, LEVEL.1 as i32);
        let at = ldu(self.f, types::I32, lp, LEVEL.2 as i32);
        let nl = self.vlen(LEVELS);
        let nl1 = self.f.ins().iadd_imm(nl, -1);
        self.vset_len(LEVELS, nl1);
        let s = self.vtop(SOURCES, pc, R_LEVEL_SRC);
        let stop = ldu(self.f, types::I8, s, offset_of!(Src, stop) as i32);
        self.check(stop, pc, R_LEVEL_SRC);
        let ns = self.vlen(SOURCES);
        let ns1 = self.f.ins().iadd_imm(ns, -1);
        self.vset_len(SOURCES, ns1);
        let a = self.vlen(ARENA);
        let nk = self.vlen(KEPT);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let done = self.f.create_block();
        self.f.ins().jump(head, &[at.into()]);
        self.f.switch_to_block(head);
        let k = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, k, nk);
        let body = self.f.create_block();
        self.f.ins().brif(more, body, &[], done, &[]);
        self.f.switch_to_block(body);
        let kp = self.vat(KEPT, k);
        self.push_arena_tok(pc, kp, 0);
        let k1 = self.f.ins().iadd_imm(k, 1);
        self.f.ins().jump(head, &[k1.into()]);
        self.f.switch_to_block(done);
        self.vset_len(KEPT, at);
        let fr = self.vtop(FRAMES, pc, R_LEVEL_FRAME);
        let pres = ldu(self.f, types::I32, fr, offset_of!(Frame, pres) as i32);
        let pi = self.f.ins().iadd(pres, p);
        let pp = self.vat(PRES, pi);
        let b = self.vlen(ARENA);
        self.put_pair(pp, a, b);
        self.next(pc);
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
            FRAME => self.frame(pc),
            ARG => self.arg(pc, mode),
            PRE if mode == 0 => self.pre(pc),
            PRE => self.pre_end(pc),
            LINE => {
                let (held, _) = self.line(pc);
                self.test(pc, held);
            }
            SKIPLINE => {
                // The line's tokens passed, a token a turn.
                let head = self.f.create_block();
                self.heads.push((head, Step::Token));
                self.f.ins().jump(head, &[]);
                self.f.switch_to_block(head);
                let (held, _) = self.line(pc);
                let next = self.blocks[pc + 1];
                self.f.ins().brif(held, head, &[], next, &[]);
            }
            PEEK => {
                let s = self.strings[usize::from(operand)].clone();
                let held = self.peek(&s);
                self.test(pc, held);
            }
            LOOKUP => {
                let held = self.lookup(pc);
                self.test(pc, held);
            }
            MACRO => {
                let d = ld(self.f, types::I64, ctx, at!(d));
                let defs = ld(self.f, types::I64, ctx, at!(defs));
                let p = elem(self.f, defs, d, DEF);
                let off = match mode {
                    0 => offset_of!(Def, disabled),
                    1 => offset_of!(Def, fnlike),
                    _ => offset_of!(Def, builtin),
                } as i32;
                let x = ldu(self.f, types::I8, p, off);
                let held = self.f.ins().icmp_imm(IntCC::NotEqual, x, 0);
                self.test(pc, held);
            }
            PAINT => {
                let fl = ld(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, flags) as i32);
                let fl = self.f.ins().bor_imm(fl, i64::from(NOEXP));
                self.f.ins().store(flags(), fl, ctx, at!(t) + offset_of!(Tok, flags) as i32);
                self.next(pc);
            }
            KEEP => {
                let n = ld(self.f, types::I64, ctx, at!(nkept));
                let room = ld(self.f, types::I64, ctx, at!(room_kept));
                let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, n, room);
                self.check(ok, pc, R_ARENA);
                let kept = ld(self.f, types::I64, ctx, at!(kept));
                let p = elem(self.f, kept, n, TOK);
                copy_tok(self.f, ctx, at!(t), p, 0);
                let n1 = self.f.ins().iadd_imm(n, 1);
                st(self.f, types::I64, n1, ctx, at!(nkept));
                self.next(pc);
            }
            _ if self.partial => self.refuse(pc, R_NOT_LOWERED),
            _ => return Err(Error::RefusedShape(format!("the preprocessing word {op:#04x} m={mode} at {pc} is not lowered yet"))),
        }
        Ok(())
    }

    /// Whether `T` is spelled `s`: its length, then its bytes, unrolled (`s` is the program's).
    fn spelled(&mut self, s: &[u8]) -> Value {
        let (p, n) = self.text();
        self.spelled_at(p, n, s)
    }

    fn spelled_at(&mut self, p: Value, n: Value, s: &[u8]) -> Value {
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

    /// `Machine::line`: the directive line's next token from the file on top, into `T` (and
    /// `line_last`); the result whether there was one (a token at a line's start ends it).
    fn line(&mut self, pc: usize) -> (Value, ()) {
        let ctx = self.ctx;
        let n = ld(self.f, types::I64, ctx, at!(nsources));
        self.check(n, pc, R_NO_FILE);
        let f = &mut *self.f;
        let srcs = ld(f, types::I64, ctx, at!(sources));
        let i = f.ins().iadd_imm(n, -1);
        let s = elem(f, srcs, i, SRC);
        let file = ldu(f, types::I32, s, offset_of!(Src, file) as i32);
        let in_file = f.ins().icmp_imm(IntCC::NotEqual, file, i64::from(NONE));
        self.check(in_file, pc, R_NOT_FILE);
        let f = &mut *self.f;
        let pos = ldu(f, types::I32, s, offset_of!(Src, pos) as i32);
        let end = ldu(f, types::I32, s, offset_of!(Src, end) as i32);
        let join = f.create_block();
        f.append_block_param(join, types::I8);
        let no = f.ins().iconst(types::I8, 0);
        let has = f.ins().icmp(IntCC::UnsignedLessThan, pos, end);
        let take = f.create_block();
        f.ins().brif(has, take, &[], join, &[no.into()]);
        f.switch_to_block(take);
        let files = ld(f, types::I64, ctx, at!(files));
        let fp = elem(f, files, file, FILE);
        let toks = ld(f, types::I64, fp, offset_of!(FileC, toks) as i32);
        let p = elem(f, toks, pos, TOK);
        let fl = ldu(f, types::I8, p, offset_of!(Tok, flags) as i32);
        let bol = f.ins().band_imm(fl, i64::from(BOL));
        let on = f.create_block();
        f.ins().brif(bol, join, &[no.into()], on, &[]);
        f.switch_to_block(on);
        copy_tok(f, p, 0, ctx, at!(t));
        copy_tok(f, p, 0, ctx, at!(line_last));
        let pos1 = f.ins().iadd_imm(pos, 1);
        st(f, types::I32, pos1, s, offset_of!(Src, pos) as i32);
        let yes = f.ins().iconst(types::I8, 1);
        f.ins().jump(join, &[yes.into()]);
        f.switch_to_block(join);
        (f.block_params(join)[0], ())
    }

    /// `Machine::peek`: whether the next token any source gives is spelled `s`, without reading
    /// it: the sources from the top passed (`Step::Pop`) while they are spent, not past a stop
    /// source's end or a file's.
    fn peek(&mut self, s: &[u8]) -> Value {
        let ctx = self.ctx;
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Pop));
        let n = ld(self.f, types::I64, ctx, at!(nsources));
        self.f.ins().jump(head, &[n.into()]);
        self.f.switch_to_block(head);
        let k = self.f.block_params(head)[0];
        let no = self.f.ins().iconst(types::I8, 0);
        let some = self.f.create_block();
        self.f.ins().brif(k, some, &[], join, &[no.into()]);
        self.f.switch_to_block(some);
        let srcs = ld(self.f, types::I64, ctx, at!(sources));
        let i = self.f.ins().iadd_imm(k, -1);
        let src = elem(self.f, srcs, i, SRC);
        let pos = ldu(self.f, types::I32, src, offset_of!(Src, pos) as i32);
        let end = ldu(self.f, types::I32, src, offset_of!(Src, end) as i32);
        let file = ldu(self.f, types::I32, src, offset_of!(Src, file) as i32);
        let has = self.f.ins().icmp(IntCC::UnsignedLessThan, pos, end);
        let look = self.f.create_block();
        let spent = self.f.create_block();
        self.f.ins().brif(has, look, &[], spent, &[]);
        self.f.switch_to_block(look);
        let p = self.src_tok(file, pos);
        let (q, m) = self.spelling(p, 0);
        let r = self.spelled_at(q, m, s);
        self.f.ins().jump(join, &[r.into()]);
        self.f.switch_to_block(spent);
        let stop = ldu(self.f, types::I8, src, offset_of!(Src, stop) as i32);
        let in_file = self.f.ins().icmp_imm(IntCC::NotEqual, file, i64::from(NONE));
        let stopped = self.f.ins().icmp_imm(IntCC::NotEqual, stop, 0);
        let end_here = self.f.ins().bor(in_file, stopped);
        let no = self.f.ins().iconst(types::I8, 0);
        let down = self.f.create_block();
        self.f.ins().brif(end_here, join, &[no.into()], down, &[]);
        self.f.switch_to_block(down);
        self.f.ins().jump(head, &[i.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// The address of source token `pos` of `file` (`NONE`: the arena's).
    fn src_tok(&mut self, file: Value, pos: Value) -> Value {
        let (ctx, f) = (self.ctx, &mut *self.f);
        let in_file = f.ins().icmp_imm(IntCC::NotEqual, file, i64::from(NONE));
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
        f.block_params(got)[0]
    }

    /// Bytes `p[0..min(rem, 8)]` as a little-endian word, zeros past them: eight at once where
    /// eight are left, else the few left one by one (straight code: at most seven), so no load
    /// passes the spelling's end.
    fn chunk(&mut self, p: Value, rem: Value) -> Value {
        let f = &mut *self.f;
        let join = f.create_block();
        f.append_block_param(join, types::I64);
        let full = f.create_block();
        let tail = f.create_block();
        let eight = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, rem, 8);
        f.ins().brif(eight, full, &[], tail, &[]);
        f.switch_to_block(full);
        let w = f.ins().load(types::I64, bytes_flags(), p, 0);
        f.ins().jump(join, &[w.into()]);
        f.switch_to_block(tail);
        let mut w = f.ins().iconst(types::I64, 0);
        for i in 0..7i64 {
            let more = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, rem, i);
            let take = f.create_block();
            f.ins().brif(more, take, &[], join, &[w.into()]);
            f.switch_to_block(take);
            let b = f.ins().load(types::I8, bytes_flags(), p, i as i32);
            let b = f.ins().uextend(types::I64, b);
            let b = f.ins().ishl_imm(b, 8 * i);
            w = f.ins().bor(w, b);
        }
        f.ins().jump(join, &[w.into()]);
        f.switch_to_block(join);
        f.block_params(join)[0]
    }

    /// `pp_words::key` of the `n` bytes at `p`, eight a step (`Step::Word`).
    fn key(&mut self, p: Value, n: Value) -> Value {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Word));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let h0 = self.f.ins().bxor_imm(n, KEY_SEED);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[h0.into(), z.into()]);
        self.f.switch_to_block(head);
        let (h, i) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[h.into()]);
        self.f.switch_to_block(step);
        let q = self.f.ins().iadd(p, i);
        let rem = self.f.ins().isub(n, i);
        let w = self.chunk(q, rem);
        let x = self.f.ins().bxor(h, w);
        let x = self.f.ins().imul_imm(x, KEY_MUL);
        let x = self.f.ins().rotl_imm(x, 31);
        let i8 = self.f.ins().iadd_imm(i, 8);
        self.f.ins().jump(head, &[x.into(), i8.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }

    /// Whether the `n` bytes at `p` and at `q` are the same, eight a step (`Step::Word`).
    fn same_bytes(&mut self, p: Value, q: Value, n: Value) -> Value {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Word));
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into()]);
        self.f.switch_to_block(head);
        let i = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let step = self.f.create_block();
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().brif(more, step, &[], join, &[yes.into()]);
        self.f.switch_to_block(step);
        let rem = self.f.ins().isub(n, i);
        let a = self.f.ins().iadd(p, i);
        let b = self.f.ins().iadd(q, i);
        let wa = self.chunk(a, rem);
        let wb = self.chunk(b, rem);
        let eq = self.f.ins().icmp(IntCC::Equal, wa, wb);
        let i8 = self.f.ins().iadd_imm(i, 8);
        let no = self.f.ins().iconst(types::I8, 0);
        let on = self.f.create_block();
        self.f.ins().brif(eq, on, &[], join, &[no.into()]);
        self.f.switch_to_block(on);
        self.f.ins().jump(head, &[i8.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// `LOOKUP` (`Machine::lookup`): `T` an identifier not painted, and a defined macro's name:
    /// its key, then the table probed a slot a step (`Step::Probe`), at most every slot once.
    fn lookup(&mut self, pc: usize) -> Value {
        let ctx = self.ctx;
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        let no = self.f.ins().iconst(types::I8, 0);
        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
        let fl = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, flags) as i32);
        let ident = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
        let painted = self.f.ins().band_imm(fl, i64::from(NOEXP));
        let unpainted = self.f.ins().icmp_imm(IntCC::Equal, painted, 0);
        let go = self.f.ins().band(ident, unpainted);
        let probe = self.f.create_block();
        self.f.ins().brif(go, probe, &[], join, &[no.into()]);
        self.f.switch_to_block(probe);
        let (p, n) = self.text();
        let key = self.key(p, n);
        let mask = ld(self.f, types::I64, ctx, at!(table_mask));
        let table = ld(self.f, types::I64, ctx, at!(table));
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Probe));
        let i0 = self.f.ins().band(key, mask);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[i0.into(), z.into()]);
        self.f.switch_to_block(head);
        let (i, tries) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let within = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, tries, mask);
        self.check(within, pc, R_TABLE);
        let slot = elem(self.f, table, i, SLOT);
        let sk = ldu(self.f, types::I8, slot, (offset_of!(Slot, name) + offset_of!(Tok, kind)) as i32);
        let empty = self.f.ins().icmp_imm(IntCC::Equal, sk, i64::from(EOF));
        let used = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(empty, join, &[no.into()], used, &[]);
        self.f.switch_to_block(used);
        let skey = ld(self.f, types::I64, slot, offset_of!(Slot, key) as i32);
        let keyed = self.f.ins().icmp(IntCC::Equal, skey, key);
        let compare = self.f.create_block();
        let on = self.f.create_block();
        self.f.ins().brif(keyed, compare, &[], on, &[]);
        self.f.switch_to_block(compare);
        let (q, m) = self.spelling(slot, offset_of!(Slot, name) as i32);
        let len_eq = self.f.ins().icmp(IntCC::Equal, m, n);
        let bytes = self.f.create_block();
        self.f.ins().brif(len_eq, bytes, &[], on, &[]);
        self.f.switch_to_block(bytes);
        let eq = self.same_bytes(p, q, n);
        let found = self.f.create_block();
        self.f.ins().brif(eq, found, &[], on, &[]);
        self.f.switch_to_block(found);
        let def = ldu(self.f, types::I32, slot, offset_of!(Slot, def) as i32);
        let defined = self.f.ins().icmp_imm(IntCC::NotEqual, def, i64::from(NONE));
        let set = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(defined, set, &[], join, &[no.into()]);
        self.f.switch_to_block(set);
        st(self.f, types::I64, def, ctx, at!(d));
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[yes.into()]);
        self.f.switch_to_block(on);
        let i1 = self.f.ins().iadd_imm(i, 1);
        let i1 = self.f.ins().band(i1, mask);
        let t1 = self.f.ins().iadd_imm(tries, 1);
        self.f.ins().jump(head, &[i1.into(), t1.into()]);
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
    compile_as(program, false)
}

/// `compile`, where a word with no code yet refuses the TU when it runs (`NOT_LOWERED`): the
/// lowering's tests, word group by word group.
pub fn compile_partial(program: &Words) -> Result<CompiledPp, Error> {
    compile_as(program, true)
}

fn compile_as(program: &Words, partial: bool) -> Result<CompiledPp, Error> {
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
        let mut l = Lower { f: &mut f, ctx, words: &program.words, strings: &program.strings, blocks, exit, heads: Vec::new(), partial };
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
        let text = b"a b\n# c b";
        let toks = [tok(IDENT, BOL, 0, 1), tok(IDENT, SPACE, 2, 3), tok(HASH, BOL, 4, 5), tok(IDENT, SPACE, 6, 7), tok(IDENT, SPACE, 8, 9), tok(PUNCT, SPACE, 8, 9)];
        same_on(words, text, &toks);
    }

    fn same_on(words: &str, text: &[u8], toks: &[Tok]) {
        let p = program(words);
        let files = [file(text, toks)];
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
    fn a_lookup_is_the_interpreters() {
        // Built-ins (the table's names) painted, others put as they are; long and short names.
        let text = b"__LINE__ x __FILE__ __has_cpp_attribute __LINE__x __COUNTER__";
        let toks = [tok(IDENT, BOL, 0, 8), tok(IDENT, SPACE, 9, 10), tok(IDENT, SPACE, 11, 19), tok(IDENT, SPACE, 20, 39), tok(IDENT, SPACE, 40, 49), tok(IDENT, SPACE, 50, 61)];
        same_on(
            concat!(
                "  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 LOOKUP 0\n  0004 GOTO @0007\n",
                "  0005 PUT 0\n  0006 GOTO @0000\n  0007 MACRO m=2 0\n  0008 PAINT 0\n  0009 PUT 0\n  0010 GOTO @0000\n",
            ),
            text,
            &toks,
        );
    }

    #[test]
    fn a_directive_line_is_the_interpreters() {
        // A token before `b` painted (PEEK); a directive's first token put, the rest passed.
        same(concat!(
            "  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 IS m=3 0\n  0004 GOTO @0009\n",
            "  0005 PEEK m=1 s0\n  0006 PAINT 0\n  0007 PUT 0\n  0008 GOTO @0000\n",
            "  0009 LINE 0\n  0010 GOTO @0012\n  0011 GOTO @0000\n  0012 PUT 0\n  0013 SKIPLINE 0\n  0014 KEEP 0\n  0015 GOTO @0000\n",
        ));
    }

    #[test]
    fn the_shape_holds() {
        let c = compile(&program("  0000 READ 0\n  0001 IS m=0 8\n  0002 ACCEPT 0\n  0003 PUT 0\n  0004 GOTO @0000\n")).unwrap();
        assert!(!c.clif.contains(" call "), "no calls out");
        assert!(c.heads.contains(&Step::Token) && c.heads.contains(&Step::Pop));
    }
}
