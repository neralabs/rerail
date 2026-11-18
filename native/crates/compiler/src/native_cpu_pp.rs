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
use crate::pp_expr::Value as Val;
use crate::pp_words::{self, DIGITS, EmC, Asked, B_BASE_FILE, B_COUNTER, B_DATE, B_FILE, B_FILE_NAME, B_HAS_EMBED, B_HAS_INCLUDE, B_HAS_INCLUDE_NEXT, B_INCLUDE_LEVEL, B_LINE, B_PRAGMA, B_TIME, B_TIMESTAMP, BUILTINS, Bi, Bounds, COND_ELSE, COND_SIZE, COND_SRC, COND_TAKEN, Ctx, Ex, FeatureC, Inc, MAX_INCLUDE_DEPTH, NamedC, OPS, Want, op_index, BOL, DIGRAPH, Def, DefState, EOF, EXP, Env, Expanded, File, FileC, Frame, HASH, HASHHASH, IDENT, LEVEL, LITERAL, NO_PARAM, NOEXP, NONE, NUMBER, PAIR, PASTE, PASTE_MARK, PLACEMARKER, PUNCT, Refused, SPACE, Slot, Src, Sub, Tok, USE, VA_ARGS, VA_OPT, VA_OPT_MARK};
use crate::program::{ACCEPT, ARG, BUILTIN, COND, EMBED, DEFINE, DEFINED, EXPR, FRAME, GLUE, GOTO, IF, INCLUDE, IS, LINEDIR, PRAGMA, SKIP, KEEP, LINE, LOOKUP, MACRO, PAINT, PEEK, PRE, PUSH, PUT, PpTables, READ, RETURN, SKIPLINE, STOP, STR, SUB, UNDEF, Words, split};
use cranelift_codegen::ir::{AbiParam, Block, BlockArg, BlockCall, InstBuilder, JumpTableData, MemFlags, Value, condcodes::IntCC, types};
use cranelift_codegen::ir::{ExtFuncData, ExternalName, FuncRef, UserExternalName};
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

/// A field of `#embed`'s state, from the context.
macro_rules! em {
    ($f:ident) => {
        (offset_of!(Ctx, em) + offset_of!(EmC, $f)) as i32
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
    "past the parameters' bound",
    "past the definitions' bound",
    "a __VA_OPT__ inside a __VA_OPT__",
    "past the scratch bytes' bound",
    "a substitution with no frame",
    "past the expansions' bound",
    "an argument substituted before its expansion",
    "a raw string's prefix pasted",
    "past the conditionals' bound",
    "a line's source not on top",
    "past the expression's bound",
    "no value",
    "no operator",
    "no precedence",
    "past the wanted files' bound",
    "past push_macro's bound",
    "past the unanswered features' bound",
    "a _Pragma's text the token grammar refuses",
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
const R_PARAMS: u64 = 16;
const R_DEFS: u64 = 17;
const R_VA_OPT: u64 = 18;
const R_SCRATCH: u64 = 19;
const R_SUB_FRAME: u64 = 20;
const R_EXPS: u64 = 21;
const R_EARLY: u64 = 22;
const R_RAW: u64 = 23;
const R_CONDS: u64 = 24;
const R_LINE_SRC: u64 = 25;
const R_EX: u64 = 26;
const R_NO_VALUE: u64 = 27;
const R_NO_OP: u64 = 28;
const R_NO_PREC: u64 = 29;
const R_WANTED: u64 = 30;
const R_PUSHED: u64 = 31;
const R_ASKED: u64 = 32;
const R_PRAGMA: u64 = 33;

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
const DEFS: V = v!(defs, ndefs, room_defs, size_of::<Def>(), R_DEFS);
const PARAMS: V = v!(params, nparams, room_params, size_of::<Tok>(), R_ARENA);
const MARKS: V = v!(marks, nmarks, room_marks, 4, R_ARENA);
const EXPS: V = v!(exps, nexps, room_exps, EXP.0, R_EXPS);
const CONDS: V = v!(conds, nconds, room_conds, COND_SIZE, R_CONDS);
const VALS: V = v!(vals, nvals, room_vals, size_of::<Val>(), R_EX);
const OPSV: V = v!(ops, nops, room_ops, 1, R_EX);
const MINS: V = v!(mins, nmins, room_mins, 1, R_EX);
const LIVE: V = v!(live, nlive, room_live, 1, R_EX);
const WANTED: V = v!(wanted, nwanted, room_wanted, size_of::<Want>(), R_WANTED);
const NAMED: i64 = size_of::<NamedC>() as i64;
const PUSHED: V = v!(pushed, npushed, room_pushed, PAIR.0, R_PUSHED);
const ASKED: V = v!(asked, nasked, room_asked, size_of::<Asked>(), R_ASKED);
const V_BITS: i32 = offset_of!(Val, bits) as i32;
const V_UNS: i32 = offset_of!(Val, unsigned) as i32;

/// `Ex`'s field `$f`'s offset.
macro_rules! ex {
    ($f:ident) => {
        offset_of!(Ex, $f) as i32
    };
}

/// No operator (`expr_op`'s `None`); an operator of no expression is `OPS.len()`.
const NO_OP: i64 = 0xff;

/// `Sub`'s field `$f`'s offset.
macro_rules! sb {
    ($f:ident) => {
        offset_of!(Sub, $f) as i32
    };
    ($f:ident . $g:tt) => {
        (offset_of!(Sub, $f) + offset_of!((u32, u32), $g)) as i32
    };
}

/// A token's fields' offsets.
const T_KIND: i32 = offset_of!(Tok, kind) as i32;
const T_FLAGS: i32 = offset_of!(Tok, flags) as i32;

/// Eight bytes' words: each byte 1, each byte 0x7f, each byte's high bit.
const ONES: i64 = 0x0101_0101_0101_0101;
const LOW7: i64 = 0x7f7f_7f7f_7f7f_7f7f;
const HIGH: i64 = 0x8080_8080_8080_8080_u64 as i64;

/// `DefState`'s field `$f`'s offset, and its definition's (`d`) field `$g`'s.
macro_rules! ds {
    ($f:ident) => {
        offset_of!(DefState, $f) as i32
    };
    (d . $g:ident) => {
        (offset_of!(DefState, d) + offset_of!(Def, $g)) as i32
    };
}

/// A program's words, compiled.
pub struct CompiledPp {
    _module: JITModule,
    entry: unsafe extern "C" fn(*mut Ctx) -> u64,
    /// The function's IR, as text.
    pub clif: String,
    /// Its stated loop heads.
    pub heads: Vec<Step>,
    /// Its machine code's bytes, each function's: the run's (`pp_run`) first.
    pub code_bytes: Vec<usize>,
    /// Each word's CLIF instructions in the run's function.
    pub word_insts: Vec<usize>,
    /// Each function's IR as Cranelift was given it, by its id, where asked for (`rust_source`).
    pub ir: Vec<(u32, cranelift_codegen::ir::Function)>,
    /// The run's function's id.
    pub run_id: u32,
    /// Each function's machine code as Cranelift prints it, the run's first, where asked for
    /// (`disassemble`).
    pub disasm: Vec<String>,
    /// Each function's machine code's bytes, the run's first, where asked for.
    pub code: Vec<Vec<u8>>,
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

/// A narrow load, widened to 64 bits: one extending load (`uload8` and the rest), which the
/// backend emits as one instruction, where a load and an `uextend` cost a move more.
fn ldu(f: &mut FunctionBuilder<'_>, ty: types::Type, base: Value, off: i32) -> Value {
    match ty {
        types::I8 => f.ins().uload8(types::I64, flags(), base, off),
        types::I16 => f.ins().uload16(types::I64, flags(), base, off),
        types::I32 => f.ins().uload32(flags(), base, off),
        _ => f.ins().load(ty, flags(), base, off),
    }
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
    pp: &'a PpTables,
    /// Each word's rank among the return points (`PUSH`'s operands), `NONE` for none: what the
    /// return stack holds, so a `RETURN`'s table has a target a return point.
    ranks: &'a [u32],
    /// The return points, by rank.
    rets: &'a [usize],
}

impl Lower<'_, '_> {
    /// Return point `v` (a rank) pushed on the return stack (refusing past its bound).
    fn push_return(&mut self, pc: usize, v: Value) {
        let ctx = self.ctx;
        let n = ld(self.f, types::I64, ctx, at!(nreturns));
        let room = ld(self.f, types::I64, ctx, at!(room_returns));
        let end = self.f.ins().iadd_imm(n, 1);
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(ok, pc, R_CALLS);
        let r = ld(self.f, types::I64, ctx, at!(returns));
        let slot = elem(self.f, r, n, 4);
        self.f.ins().store(flags(), v, slot, 0);
        st(self.f, types::I64, end, ctx, at!(nreturns));
    }

    /// To `cases`' block whose key `x` is (the keys rising, small), else to `default`: a jump
    /// table. Cranelift's code has it as a tree of compares
    /// (`native_cpu_codegen::tables_as_compares`, its jump tables' barrier); `clif_rust`'s keeps
    /// the table. Ends the current block.
    fn switch(&mut self, x: Value, cases: &[(u64, Block)], default: Block) {
        debug_assert!(cases.windows(2).all(|w| w[0].0 < w[1].0));
        let n = cases.last().map_or(0, |c| c.0 as usize + 1);
        let mut entries = vec![default; n];
        for &(k, b) in cases {
            entries[k as usize] = b;
        }
        let calls: Vec<BlockCall> = entries.iter().map(|&b| self.f.func.dfg.block_call(b, &[])).collect();
        let dflt = self.f.func.dfg.block_call(default, &[]);
        let jt = self.f.create_jump_table(JumpTableData::new(dflt, &calls));
        let x = match self.f.func.dfg.value_type(x).bits() {
            32 => x,
            b if b > 32 => self.f.ins().ireduce(types::I32, x),
            _ => self.f.ins().uextend(types::I32, x),
        };
        self.f.ins().br_table(x, jt);
    }

    /// A return to the return point of rank `to`, its block `targets[to]`: through a dispatch
    /// block of the `RETURN`'s own (`Step::Return`), a tree of compares over the ranks
    /// (`switch`).
    fn dispatch(&mut self, pc: usize, to: Value, targets: &[Block]) {
        let dispatch = self.f.create_block();
        self.heads.push((dispatch, Step::Return));
        self.f.ins().jump(dispatch, &[]);
        self.f.switch_to_block(dispatch);
        let bad = self.f.create_block();
        let cases: Vec<(u64, Block)> = targets.iter().enumerate().map(|(k, &b)| (k as u64, b)).collect();
        self.switch(to, &cases, bad);
        self.f.switch_to_block(bad);
        self.refuse(pc, R_NO_CALL);
    }

    /// An outlined word called (`compile`): its result 0 to `next`, 1 to `skip`, else the
    /// refusal; the held branch through a step of its own where the word steps where it holds.
    fn call_word(&mut self, fref: FuncRef, op: u8, mode: u8, next: Block, skip: Block) {
        let ctx = self.ctx;
        let r = self.f.ins().call(fref, &[ctx]);
        let r = self.f.inst_results(r)[0];
        let step = pp_words::steps_where_held(op, mode).then(|| self.f.create_block());
        let held = step.unwrap_or(next);
        let other = self.f.create_block();
        let zero = self.f.ins().icmp_imm(IntCC::Equal, r, 0);
        self.f.ins().brif(zero, held, &[], other, &[]);
        self.f.switch_to_block(other);
        let one = self.f.ins().icmp_imm(IntCC::Equal, r, 1);
        let refused = self.f.create_block();
        self.f.ins().brif(one, skip, &[], refused, &[]);
        self.f.switch_to_block(refused);
        self.f.ins().jump(self.exit, &[r.into()]);
        if let Some(step) = step {
            self.heads.push((step, if op == DEFINE && mode == 7 { Step::Item } else { Step::Token }));
            self.f.switch_to_block(step);
            self.f.ins().jump(next, &[]);
        }
    }

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
    fn push_arena_src(&mut self, pc: usize, a: Value, b: Value, def: Value, stop: bool) {
        let s = self.vpush(SOURCES, pc);
        let f = &mut *self.f;
        let none = f.ins().iconst(types::I32, i64::from(NONE));
        let z = f.ins().iconst(types::I32, 0);
        let stop = f.ins().iconst(types::I8, i64::from(stop));
        f.ins().store(flags(), none, s, offset_of!(Src, file) as i32);
        st(f, types::I32, a, s, offset_of!(Src, pos) as i32);
        st(f, types::I32, b, s, offset_of!(Src, end) as i32);
        st(f, types::I32, def, s, offset_of!(Src, def) as i32);
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
        let none = self.f.ins().iconst(types::I32, i64::from(NONE));
        self.push_arena_src(pc, a, b, none, true);
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

    /// `Machine::skip_line`: the directive line's tokens passed (`Step::Token`); then `next`.
    fn skip_line_then(&mut self, pc: usize, next: Block) {
        let head = self.f.create_block();
        self.heads.push((head, Step::Token));
        self.f.ins().jump(head, &[]);
        self.f.switch_to_block(head);
        let (held, _) = self.line(pc);
        self.f.ins().brif(held, head, &[], next, &[]);
    }

    /// Whether the token at `tok + off` is spelled `s`.
    fn tok_spelled(&mut self, tok: Value, off: i32, s: &[u8]) -> Value {
        let (p, n) = self.spelling(tok, off);
        self.spelled_at(p, n, s)
    }

    /// `Machine::scratch_tok(kind, s, 0)`: `s` written to scratch, into the token at `to + off`.
    fn scratch_tok(&mut self, pc: usize, kind: u8, s: &[u8], to: Value, off: i32) {
        let ctx = self.ctx;
        let n = ld(self.f, types::I64, ctx, at!(nscratch));
        let room = ld(self.f, types::I64, ctx, at!(room_scratch));
        let end = self.f.ins().iadd_imm(n, s.len() as i64);
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(ok, pc, R_SCRATCH);
        let base = ld(self.f, types::I64, ctx, at!(scratch));
        let p = self.f.ins().iadd(base, n);
        for (i, &c) in s.iter().enumerate() {
            let c = self.f.ins().iconst(types::I8, i64::from(c));
            self.f.ins().store(bytes_flags(), c, p, i as i32);
        }
        st(self.f, types::I64, end, ctx, at!(nscratch));
        let k = self.f.ins().iconst(types::I8, i64::from(kind));
        let z8 = self.f.ins().iconst(types::I8, 0);
        let z = self.f.ins().iconst(types::I32, 0);
        self.f.ins().store(flags(), k, to, off + offset_of!(Tok, kind) as i32);
        self.f.ins().store(flags(), z8, to, off + offset_of!(Tok, flags) as i32);
        self.f.ins().store(flags(), z, to, off + offset_of!(Tok, buf) as i32);
        st(self.f, types::I32, n, to, off + offset_of!(Tok, start) as i32);
        st(self.f, types::I32, end, to, off + offset_of!(Tok, end) as i32);
        self.f.ins().store(flags(), z, to, off + offset_of!(Tok, exp) as i32);
    }

    /// The token at `to + off` an end of input (`eof(0)`: none).
    fn set_eof(&mut self, to: Value, off: i32) {
        let k = self.f.ins().iconst(types::I8, i64::from(EOF));
        let z8 = self.f.ins().iconst(types::I8, 0);
        let z = self.f.ins().iconst(types::I32, 0);
        self.f.ins().store(flags(), k, to, off + offset_of!(Tok, kind) as i32);
        self.f.ins().store(flags(), z8, to, off + offset_of!(Tok, flags) as i32);
        for o in [offset_of!(Tok, buf), offset_of!(Tok, start), offset_of!(Tok, end), offset_of!(Tok, exp)] {
            self.f.ins().store(flags(), z, to, off + o as i32);
        }
    }

    fn def_state(&mut self) -> Value {
        ld(self.f, types::I64, self.ctx, at!(def_state))
    }

    /// Body token `k`'s place in its file's tape, for the definition at `d + off` (a `Def`).
    fn body_tok(&mut self, d: Value, off: i32, k: Value) -> Value {
        let file = ldu(self.f, types::I32, d, off + offset_of!(Def, file) as i32);
        let first = ldu(self.f, types::I32, d, off + offset_of!(Def, first) as i32);
        let files = ld(self.f, types::I64, self.ctx, at!(files));
        let fp = elem(self.f, files, file, FILE);
        let toks = ld(self.f, types::I64, fp, offset_of!(FileC, toks) as i32);
        let i = self.f.ins().iadd(first, k);
        elem(self.f, toks, i, TOK)
    }

    /// Body token `k`'s mark (`Machine::mark`'s, and its paste flag).
    fn body_mark(&mut self, d: Value, off: i32, k: Value) -> Value {
        let marks = ldu(self.f, types::I32, d, off + offset_of!(Def, marks) as i32);
        let i = self.f.ins().iadd(marks, k);
        let p = self.vat(MARKS, i);
        ldu(self.f, types::I32, p, 0)
    }

    /// `Machine::body_tok(def, k).flags & PASTE != 0`: its mark's paste flag.
    fn body_pasted(&mut self, d: Value, off: i32, k: Value) -> Value {
        let m = self.body_mark(d, off, k);
        let x = self.f.ins().band_imm(m, i64::from(PASTE_MARK));
        let t = self.body_tok(d, off, k);
        let fl = ldu(self.f, types::I8, t, T_FLAGS);
        let y = self.f.ins().band_imm(fl, i64::from(PASTE));
        let z = self.f.ins().bor(x, y);
        self.f.ins().icmp_imm(IntCC::NotEqual, z, 0)
    }

    /// `Machine::mark_body`: the token at `tok + off` the next of the body being read, marked.
    fn mark_body(&mut self, pc: usize, tok: Value, off: i32) {
        let ds = self.def_state();
        let end = ldu(self.f, types::I32, ds, ds!(d.end));
        let first = ldu(self.f, types::I32, ds, ds!(d.first));
        let k = self.f.ins().isub(end, first);
        let kind = ldu(self.f, types::I8, tok, off + offset_of!(Tok, kind) as i32);
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        let none = self.f.ins().iconst(types::I64, i64::from(NO_PARAM));
        let ident = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(IDENT));
        let named = self.f.create_block();
        self.f.ins().brif(ident, named, &[], join, &[none.into()]);
        self.f.switch_to_block(named);
        let variadic = ldu(self.f, types::I8, ds, ds!(d.variadic));
        let va_opt = self.f.create_block();
        let params = self.f.create_block();
        self.f.ins().brif(variadic, va_opt, &[], params, &[]);
        self.f.switch_to_block(va_opt);
        let is_va_opt = self.tok_spelled(tok, off, VA_OPT);
        let mark = self.f.ins().iconst(types::I64, i64::from(VA_OPT_MARK));
        self.f.ins().brif(is_va_opt, join, &[mark.into()], params, &[]);
        // A parameter's name: the parameters walked (`Step::Item`).
        self.f.switch_to_block(params);
        let fnlike = ldu(self.f, types::I8, ds, ds!(d.fnlike));
        let walk = self.f.create_block();
        let none = self.f.ins().iconst(types::I64, i64::from(NO_PARAM));
        self.f.ins().brif(fnlike, walk, &[], join, &[none.into()]);
        self.f.switch_to_block(walk);
        let (p, n) = self.spelling(tok, off);
        let p0 = ldu(self.f, types::I32, ds, ds!(p0));
        let np = self.vlen(PARAMS);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        self.f.ins().jump(head, &[p0.into()]);
        self.f.switch_to_block(head);
        let j = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, j, np);
        let look = self.f.create_block();
        let none = self.f.ins().iconst(types::I64, i64::from(NO_PARAM));
        self.f.ins().brif(more, look, &[], join, &[none.into()]);
        self.f.switch_to_block(look);
        let pp = self.vat(PARAMS, j);
        let (q, m) = self.spelling(pp, 0);
        let len_eq = self.f.ins().icmp(IntCC::Equal, m, n);
        let bytes = self.f.create_block();
        let on = self.f.create_block();
        self.f.ins().brif(len_eq, bytes, &[], on, &[]);
        self.f.switch_to_block(bytes);
        let eq = self.same_bytes(p, q, n);
        let idx = self.f.ins().isub(j, p0);
        self.f.ins().brif(eq, join, &[idx.into()], on, &[]);
        self.f.switch_to_block(on);
        let j1 = self.f.ins().iadd_imm(j, 1);
        self.f.ins().jump(head, &[j1.into()]);
        self.f.switch_to_block(join);
        let m = self.f.block_params(join)[0];
        // `##` but first: a paste.
        let hh = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(HASHHASH));
        let later = self.f.ins().icmp_imm(IntCC::NotEqual, k, 0);
        let paste = self.f.ins().band(hh, later);
        let pm = self.f.ins().bor_imm(m, i64::from(PASTE_MARK));
        let m = self.f.ins().select(paste, pm, m);
        let mp = self.vpush(MARKS, pc);
        st(self.f, types::I32, m, mp, 0);
        let ds = self.def_state();
        let end1 = self.f.ins().iadd_imm(end, 1);
        st(self.f, types::I32, end1, ds, ds!(d.end));
    }

    /// `__VA_OPT__` at body token `i` (before `hi`): its content's range and the token after it
    /// (`Machine::va_opt_content`), its `(…)` walked a token a step (`Step::Item`).
    fn va_opt_content(&mut self, d: Value, off: i32, i: Value, hi: Value) -> (Value, Value, Value) {
        let join = self.f.create_block();
        for _ in 0..3 {
            self.f.append_block_param(join, types::I64);
        }
        let i1 = self.f.ins().iadd_imm(i, 1);
        let i2 = self.f.ins().iadd_imm(i, 2);
        let within = self.f.ins().icmp(IntCC::UnsignedLessThan, i1, hi);
        let look = self.f.create_block();
        let bare = self.f.create_block();
        self.f.ins().brif(within, look, &[], bare, &[]);
        self.f.switch_to_block(look);
        let t = self.body_tok(d, off, i1);
        let open = self.tok_spelled(t, 0, b"(");
        let walk = self.f.create_block();
        self.f.ins().brif(open, walk, &[], bare, &[]);
        self.f.switch_to_block(bare);
        self.f.ins().jump(join, &[i1.into(), i1.into(), i1.into()]);
        self.f.switch_to_block(walk);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[i2.into(), z.into()]);
        self.f.switch_to_block(head);
        let (j, depth) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, j, hi);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], join, &[i2.into(), hi.into(), hi.into()]);
        self.f.switch_to_block(step);
        let t = self.body_tok(d, off, j);
        let j1 = self.f.ins().iadd_imm(j, 1);
        let opens = self.tok_spelled(t, 0, b"(");
        let deeper = self.f.ins().iadd_imm(depth, 1);
        let not_open = self.f.create_block();
        self.f.ins().brif(opens, head, &[j1.into(), deeper.into()], not_open, &[]);
        self.f.switch_to_block(not_open);
        let closes = self.tok_spelled(t, 0, b")");
        let close = self.f.create_block();
        self.f.ins().brif(closes, close, &[], head, &[j1.into(), depth.into()]);
        self.f.switch_to_block(close);
        let shallower = self.f.ins().iadd_imm(depth, -1);
        let inner = self.f.create_block();
        self.f.ins().brif(depth, inner, &[], join, &[i2.into(), j.into(), j1.into()]);
        self.f.switch_to_block(inner);
        self.f.ins().jump(head, &[j1.into(), shallower.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1], r[2])
    }

    /// `DEFINE` (`Machine::define`), its modes.
    fn define(&mut self, pc: usize, mode: u8) {
        let ctx = self.ctx;
        let (next, skip) = (self.blocks[pc + 1], self.blocks.get(pc + 2).copied().unwrap_or(self.blocks[pc + 1]));
        match mode {
            0 => {
                // Its name, else the line passed.
                let (held, _) = self.line(pc);
                let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
                let ident = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
                let named = self.f.ins().band(held, ident);
                let start = self.f.create_block();
                let pass = self.f.create_block();
                self.f.ins().brif(named, start, &[], pass, &[]);
                self.f.switch_to_block(pass);
                self.skip_line_then(pc, skip);
                self.f.switch_to_block(start);
                let ds = self.def_state();
                // `DefState::default()`, then the name, the parameters' start, `want`.
                let z8 = self.f.ins().iconst(types::I8, 0);
                let z = self.f.ins().iconst(types::I32, 0);
                let none = self.f.ins().iconst(types::I32, i64::from(NONE));
                copy_tok(self.f, ctx, at!(t), ds, ds!(name));
                let np = self.vlen(PARAMS);
                st(self.f, types::I32, np, ds, ds!(p0));
                for o in [ds!(fnlike), ds!(variadic), ds!(closed)] {
                    self.f.ins().store(flags(), z8, ds, o);
                }
                let one = self.f.ins().iconst(types::I8, 1);
                self.f.ins().store(flags(), one, ds, ds!(want));
                self.set_eof(ds, ds!(first));
                for w in 0..(size_of::<Def>() as i32 / 4) {
                    self.f.ins().store(flags(), z, ds, ds!(d) + 4 * w);
                }
                for o in [ds!(lo), ds!(i), ds!(hi)] {
                    self.f.ins().store(flags(), z, ds, o);
                }
                let c = offset_of!(DefState, content) as i32;
                self.f.ins().store(flags(), none, ds, c + offset_of!((u32, u32), 0) as i32);
                self.f.ins().store(flags(), none, ds, c + offset_of!((u32, u32), 1) as i32);
                self.f.ins().jump(next, &[]);
            }
            1 => {
                // `(` right after the name: function-like.
                let (held, _) = self.line(pc);
                let join = self.f.create_block();
                self.f.append_block_param(join, types::I8);
                let look = self.f.create_block();
                let no = self.f.ins().iconst(types::I8, 0);
                self.f.ins().brif(held, look, &[], join, &[no.into()]);
                self.f.switch_to_block(look);
                let paren = self.tok_spelled(ctx, at!(t), b"(");
                let fl = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, flags) as i32);
                let sp = self.f.ins().band_imm(fl, i64::from(SPACE));
                let tight = self.f.ins().icmp_imm(IntCC::Equal, sp, 0);
                let r = self.f.ins().band(paren, tight);
                self.f.ins().jump(join, &[r.into()]);
                self.f.switch_to_block(join);
                let fnlike = self.f.block_params(join)[0];
                let yes = self.f.create_block();
                let no = self.f.create_block();
                self.f.ins().brif(fnlike, yes, &[], no, &[]);
                self.f.switch_to_block(yes);
                let ds = self.def_state();
                let one = self.f.ins().iconst(types::I8, 1);
                self.f.ins().store(flags(), one, ds, ds!(fnlike));
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(no);
                let ds = self.def_state();
                let got = self.f.create_block();
                let none = self.f.create_block();
                self.f.ins().brif(held, got, &[], none, &[]);
                self.f.switch_to_block(got);
                copy_tok(self.f, ctx, at!(t), ds, ds!(first));
                self.f.ins().jump(skip, &[]);
                self.f.switch_to_block(none);
                self.set_eof(ds, ds!(first));
                self.f.ins().jump(skip, &[]);
            }
            2 => self.define_param(pc),
            3 => {
                let ds = self.def_state();
                let closed = ldu(self.f, types::I8, ds, ds!(closed));
                let read = self.f.create_block();
                self.f.ins().brif(closed, read, &[], skip, &[]);
                self.f.switch_to_block(read);
                let (held, _) = self.line(pc);
                let ds = self.def_state();
                let got = self.f.create_block();
                let none = self.f.create_block();
                self.f.ins().brif(held, got, &[], none, &[]);
                self.f.switch_to_block(got);
                copy_tok(self.f, ctx, at!(t), ds, ds!(first));
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(none);
                self.set_eof(ds, ds!(first));
                self.f.ins().jump(next, &[]);
            }
            4 => {
                // The body: a range of the file's tape from `first`.
                let top = self.vtop(SOURCES, pc, R_NO_FILE);
                let pos = ldu(self.f, types::I32, top, offset_of!(Src, pos) as i32);
                let file = ldu(self.f, types::I32, top, offset_of!(Src, file) as i32);
                let ds = self.def_state();
                let fk = ldu(self.f, types::I8, ds, ds!(first) + offset_of!(Tok, kind) as i32);
                let has_first = self.f.ins().icmp_imm(IntCC::NotEqual, fk, i64::from(EOF));
                let back = self.f.ins().iadd_imm(pos, -1);
                let first_at = self.f.ins().select(has_first, back, pos);
                let np = self.vlen(PARAMS);
                let p0 = ldu(self.f, types::I32, ds, ds!(p0));
                let params = self.f.ins().isub(np, p0);
                let ok = self.f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, params, 0xfff0);
                self.check(ok, pc, R_PARAMS);
                let ds = self.def_state();
                let fnlike = ldu(self.f, types::I8, ds, ds!(fnlike));
                let variadic = ldu(self.f, types::I8, ds, ds!(variadic));
                let nm = self.vlen(MARKS);
                let z = self.f.ins().iconst(types::I32, 0);
                let z8 = self.f.ins().iconst(types::I8, 0);
                st(self.f, types::I8, fnlike, ds, ds!(d.fnlike));
                st(self.f, types::I8, variadic, ds, ds!(d.variadic));
                st(self.f, types::I16, params, ds, ds!(d.params));
                st(self.f, types::I32, file, ds, ds!(d.file));
                st(self.f, types::I32, first_at, ds, ds!(d.first));
                st(self.f, types::I32, first_at, ds, ds!(d.end));
                st(self.f, types::I32, nm, ds, ds!(d.marks));
                self.f.ins().store(flags(), z, ds, ds!(d.pre) + offset_of!((u32, u32), 0) as i32);
                self.f.ins().store(flags(), z, ds, ds!(d.pre) + offset_of!((u32, u32), 1) as i32);
                self.f.ins().store(flags(), z8, ds, ds!(d.disabled));
                self.f.ins().store(flags(), z8, ds, ds!(d.builtin));
                let mark = self.f.create_block();
                self.f.ins().brif(has_first, mark, &[], next, &[]);
                self.f.switch_to_block(mark);
                let ds = self.def_state();
                self.mark_body(pc, ds, ds!(first));
                self.f.ins().jump(next, &[]);
            }
            5 => {
                let (held, _) = self.line(pc);
                let mark = self.f.create_block();
                let end = self.f.create_block();
                self.f.ins().brif(held, mark, &[], end, &[]);
                self.f.switch_to_block(mark);
                self.mark_body(pc, ctx, at!(t));
                self.f.ins().jump(next, &[]);
                // A `##` last is no paste operator.
                self.f.switch_to_block(end);
                let ds = self.def_state();
                let de = ldu(self.f, types::I32, ds, ds!(d.end));
                let df = ldu(self.f, types::I32, ds, ds!(d.first));
                let some = self.f.ins().icmp(IntCC::UnsignedGreaterThan, de, df);
                let clear = self.f.create_block();
                self.f.ins().brif(some, clear, &[], skip, &[]);
                self.f.switch_to_block(clear);
                let n = self.f.ins().isub(de, df);
                let last = self.f.ins().iadd_imm(n, -1);
                let ds = self.def_state();
                let mk = ldu(self.f, types::I32, ds, ds!(d.marks));
                let k = self.f.ins().iadd(mk, last);
                let mp = self.vat(MARKS, k);
                let m = ldu(self.f, types::I32, mp, 0);
                let m = self.f.ins().band_imm(m, !i64::from(PASTE_MARK));
                st(self.f, types::I32, m, mp, 0);
                self.f.ins().jump(skip, &[]);
            }
            6 => {
                let u0 = self.vlen(USES);
                let ds = self.def_state();
                st(self.f, types::I32, u0, ds, ds!(d.pre) + offset_of!((u32, u32), 0) as i32);
                st(self.f, types::I32, u0, ds, ds!(d.pre) + offset_of!((u32, u32), 1) as i32);
                let fnlike = ldu(self.f, types::I8, ds, ds!(d.fnlike));
                let de = ldu(self.f, types::I32, ds, ds!(d.end));
                let df = ldu(self.f, types::I32, ds, ds!(d.first));
                let n = self.f.ins().isub(de, df);
                let z = self.f.ins().iconst(types::I64, 0);
                let hi = self.f.ins().select(fnlike, n, z);
                let none = self.f.ins().iconst(types::I32, i64::from(NONE));
                let z32 = self.f.ins().iconst(types::I32, 0);
                self.f.ins().store(flags(), z32, ds, ds!(lo));
                self.f.ins().store(flags(), z32, ds, ds!(i));
                st(self.f, types::I32, hi, ds, ds!(hi));
                let c = offset_of!(DefState, content) as i32;
                self.f.ins().store(flags(), none, ds, c + offset_of!((u32, u32), 0) as i32);
                self.f.ins().store(flags(), none, ds, c + offset_of!((u32, u32), 1) as i32);
                self.f.ins().jump(next, &[]);
            }
            7 => self.walk_use(pc),
            _ => {
                // The definition entered: the table's slot of its name made or changed.
                let ds = self.def_state();
                let nu = self.vlen(USES);
                st(self.f, types::I32, nu, ds, ds!(d.pre) + offset_of!((u32, u32), 1) as i32);
                let p0 = ldu(self.f, types::I32, ds, ds!(p0));
                self.vset_len(PARAMS, p0);
                let dp = self.vpush(DEFS, pc);
                let ds = self.def_state();
                for w in 0..(size_of::<Def>() as i32 / 4) {
                    let x = self.f.ins().load(types::I32, flags(), ds, ds!(d) + 4 * w);
                    self.f.ins().store(flags(), x, dp, 4 * w);
                }
                let nd = self.vlen(DEFS);
                let k = self.f.ins().iadd_imm(nd, -1);
                let (p, n) = self.spelling(ds, ds!(name));
                let (slot, found) = self.probe(pc, p, n);
                let new = self.f.create_block();
                let set = self.f.create_block();
                self.f.ins().brif(found, set, &[], new, &[]);
                self.f.switch_to_block(new);
                let key = self.key(p, n);
                st(self.f, types::I64, key, slot, offset_of!(Slot, key) as i32);
                let ds = self.def_state();
                copy_tok(self.f, ds, ds!(name), slot, offset_of!(Slot, name) as i32);
                self.f.ins().jump(set, &[]);
                self.f.switch_to_block(set);
                st(self.f, types::I32, k, slot, offset_of!(Slot, def) as i32);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    /// `DEFINE m=2`, a test: a parameter list's token read (held while the list goes on); its
    /// `)` closes it, a malformed one or the line's end leaves no definition.
    fn define_param(&mut self, pc: usize) {
        let ctx = self.ctx;
        let (next, skip) = (self.blocks[pc + 1], self.blocks[pc + 2]);
        let ds = self.def_state();
        let p0 = ldu(self.f, types::I32, ds, ds!(p0));
        let want = ldu(self.f, types::I8, ds, ds!(want));
        let variadic = ldu(self.f, types::I8, ds, ds!(variadic));
        let (held, _) = self.line(pc);
        let got = self.f.create_block();
        let unterminated = self.f.create_block();
        self.f.ins().brif(held, got, &[], unterminated, &[]);
        self.f.switch_to_block(unterminated);
        self.vset_len(PARAMS, p0);
        self.f.ins().jump(skip, &[]);
        self.f.switch_to_block(got);
        let np = self.vlen(PARAMS);
        let n = self.f.ins().isub(np, p0);
        let want_b = self.f.ins().icmp_imm(IntCC::NotEqual, want, 0);
        let va_b = self.f.ins().icmp_imm(IntCC::NotEqual, variadic, 0);
        let n0 = self.f.ins().icmp_imm(IntCC::Equal, n, 0);
        let close = self.tok_spelled(ctx, at!(t), b")");
        let not_want = self.f.ins().bxor_imm(want_b, 1);
        let a = self.f.ins().bor(not_want, n0);
        let wv = self.f.ins().band(want_b, va_b);
        let not_wv = self.f.ins().bxor_imm(wv, 1);
        let closes = self.f.ins().band(close, a);
        let closes = self.f.ins().band(closes, not_wv);
        let closed = self.f.create_block();
        let other = self.f.create_block();
        self.f.ins().brif(closes, closed, &[], other, &[]);
        self.f.switch_to_block(closed);
        let ds = self.def_state();
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, ds, ds!(closed));
        self.f.ins().jump(skip, &[]);
        self.f.switch_to_block(other);
        let malformed = self.f.create_block();
        let not_va = self.f.create_block();
        self.f.ins().brif(va_b, malformed, &[], not_va, &[]);
        self.f.switch_to_block(not_va);
        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
        let ident = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
        let want_name = self.f.ins().band(want_b, ident);
        let name = self.f.create_block();
        let not_name = self.f.create_block();
        self.f.ins().brif(want_name, name, &[], not_name, &[]);
        // A parameter's name.
        self.f.switch_to_block(name);
        let pp = self.vpush(PARAMS, pc);
        copy_tok(self.f, ctx, at!(t), pp, 0);
        let ds = self.def_state();
        let z = self.f.ins().iconst(types::I8, 0);
        self.f.ins().store(flags(), z, ds, ds!(want));
        self.f.ins().jump(next, &[]);
        self.f.switch_to_block(not_name);
        let dots = self.tok_spelled(ctx, at!(t), b"...");
        let comma = self.tok_spelled(ctx, at!(t), b",");
        let wd = self.f.ins().band(want_b, dots);
        let va = self.f.create_block();
        let not_va2 = self.f.create_block();
        self.f.ins().brif(wd, va, &[], not_va2, &[]);
        // `...`: `__VA_ARGS__`, its spelling made.
        self.f.switch_to_block(va);
        let pp = self.vpush(PARAMS, pc);
        self.scratch_tok(pc, IDENT, VA_ARGS, pp, 0);
        let ds = self.def_state();
        let one = self.f.ins().iconst(types::I8, 1);
        let z = self.f.ins().iconst(types::I8, 0);
        self.f.ins().store(flags(), one, ds, ds!(variadic));
        self.f.ins().store(flags(), z, ds, ds!(want));
        self.f.ins().jump(next, &[]);
        self.f.switch_to_block(not_va2);
        let not_want = self.f.ins().bxor_imm(want_b, 1);
        let some = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
        let gnu = self.f.ins().band(not_want, dots);
        let gnu = self.f.ins().band(gnu, some);
        let gnu_b = self.f.create_block();
        let not_gnu = self.f.create_block();
        self.f.ins().brif(gnu, gnu_b, &[], not_gnu, &[]);
        // GNU's `name...`.
        self.f.switch_to_block(gnu_b);
        let ds = self.def_state();
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, ds, ds!(variadic));
        self.f.ins().jump(next, &[]);
        self.f.switch_to_block(not_gnu);
        let sep = self.f.ins().band(not_want, comma);
        let sep_b = self.f.create_block();
        self.f.ins().brif(sep, sep_b, &[], malformed, &[]);
        self.f.switch_to_block(sep_b);
        let ds = self.def_state();
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, ds, ds!(want));
        self.f.ins().jump(next, &[]);
        // Malformed: no definition, the line passed.
        self.f.switch_to_block(malformed);
        self.vset_len(PARAMS, p0);
        self.skip_line_then(pc, skip);
    }

    /// `DEFINE m=7` (`Machine::walk_use`), a test: one step of the walk of the definition's
    /// argument uses.
    fn walk_use(&mut self, pc: usize) {
        let (next, skip) = (self.blocks[pc + 1], self.blocks[pc + 2]);
        let ds = self.def_state();
        let d = ds!(d);
        let lo = ldu(self.f, types::I32, ds, ds!(lo));
        let i = ldu(self.f, types::I32, ds, ds!(i));
        let hi = ldu(self.f, types::I32, ds, ds!(hi));
        let c = offset_of!(DefState, content) as i32;
        let c0 = ldu(self.f, types::I32, ds, c + offset_of!((u32, u32), 0) as i32);
        let c1 = ldu(self.f, types::I32, ds, c + offset_of!((u32, u32), 1) as i32);
        let inside = self.f.ins().icmp_imm(IntCC::NotEqual, c0, i64::from(NONE));
        let at_end = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, hi);
        let ended = self.f.create_block();
        let step = self.f.create_block();
        self.f.ins().brif(at_end, ended, &[], step, &[]);
        // A content's end: the walk after it; else the walk's end.
        self.f.switch_to_block(ended);
        let resume = self.f.create_block();
        self.f.ins().brif(inside, resume, &[], skip, &[]);
        self.f.switch_to_block(resume);
        let z = self.f.ins().iconst(types::I32, 0);
        let none = self.f.ins().iconst(types::I32, i64::from(NONE));
        self.f.ins().store(flags(), z, ds, ds!(lo));
        st(self.f, types::I32, c0, ds, ds!(i));
        st(self.f, types::I32, c1, ds, ds!(hi));
        self.f.ins().store(flags(), none, ds, c + offset_of!((u32, u32), 0) as i32);
        self.f.ins().store(flags(), none, ds, c + offset_of!((u32, u32), 1) as i32);
        self.f.ins().jump(next, &[]);

        self.f.switch_to_block(step);
        let variadic = ldu(self.f, types::I8, ds, ds!(d.variadic));
        let variadic = self.f.ins().icmp_imm(IntCC::NotEqual, variadic, 0);
        let params = ldu(self.f, types::I16, ds, ds!(d.params));
        let bt = self.body_tok(ds, d, i);
        let kind = ldu(self.f, types::I8, bt, offset_of!(Tok, kind) as i32);
        let is_hash = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(HASH));
        let i1 = self.f.ins().iadd_imm(i, 1);
        let i2 = self.f.ins().iadd_imm(i, 2);
        let next_in = self.f.ins().icmp(IntCC::UnsignedLessThan, i1, hi);
        // `#` and a parameter: stringized, no use; two tokens passed.
        let hash_b = self.f.create_block();
        let opens_at = self.f.create_block();
        self.f.append_block_param(opens_at, types::I64);
        let not_opt = self.f.create_block();
        let h_next = self.f.ins().band(is_hash, next_in);
        let check_self = self.f.create_block();
        self.f.ins().brif(h_next, hash_b, &[], check_self, &[]);
        self.f.switch_to_block(hash_b);
        let m1 = self.body_mark(ds, d, i1);
        let m1 = self.f.ins().band_imm(m1, 0xffff);
        let is_param = self.f.ins().icmp_imm(IntCC::UnsignedLessThan, m1, i64::from(VA_OPT_MARK));
        let passed = self.f.create_block();
        let maybe_opt = self.f.create_block();
        self.f.ins().brif(is_param, passed, &[], maybe_opt, &[]);
        self.f.switch_to_block(passed);
        st(self.f, types::I32, i2, ds, ds!(i));
        self.f.ins().jump(next, &[]);
        self.f.switch_to_block(maybe_opt);
        let opt1 = self.f.ins().icmp_imm(IntCC::Equal, m1, i64::from(VA_OPT_MARK));
        let opt1 = self.f.ins().band(opt1, variadic);
        self.f.ins().brif(opt1, opens_at, &[i1.into()], check_self, &[]);
        self.f.switch_to_block(check_self);
        let m0 = self.body_mark(ds, d, i);
        let m0p = self.f.ins().band_imm(m0, 0xffff);
        let opt0 = self.f.ins().icmp_imm(IntCC::Equal, m0p, i64::from(VA_OPT_MARK));
        let opt0 = self.f.ins().band(opt0, variadic);
        self.f.ins().brif(opt0, opens_at, &[i.into()], not_opt, &[]);
        // `__VA_OPT__`: the variadic argument's use, its content walked in place.
        self.f.switch_to_block(opens_at);
        let k = self.f.block_params(opens_at)[0];
        let flat = self.f.ins().bxor_imm(inside, 1);
        self.check(flat, pc, R_VA_OPT);
        let up = self.vpush(USES, pc);
        let va = self.f.ins().iadd_imm(params, -1);
        let fls = self.f.ins().iconst(types::I8, 0);
        st(self.f, types::I16, va, up, USE.1 as i32);
        self.f.ins().store(flags(), fls, up, USE.2 as i32);
        let (clo, chi, end) = self.va_opt_content(ds, d, k, hi);
        let dv = self.def_state();
        st(self.f, types::I32, clo, dv, ds!(lo));
        st(self.f, types::I32, clo, dv, ds!(i));
        st(self.f, types::I32, chi, dv, ds!(hi));
        st(self.f, types::I32, end, dv, c + offset_of!((u32, u32), 0) as i32);
        st(self.f, types::I32, hi, dv, c + offset_of!((u32, u32), 1) as i32);
        self.f.ins().jump(next, &[]);
        // A parameter not beside a `##`: its argument's use.
        self.f.switch_to_block(not_opt);
        let is_param = self.f.ins().icmp_imm(IntCC::UnsignedLessThan, m0p, i64::from(VA_OPT_MARK));
        let param = self.f.create_block();
        let done = self.f.create_block();
        self.f.ins().brif(is_param, param, &[], done, &[]);
        self.f.switch_to_block(param);
        let has_before = self.f.ins().icmp(IntCC::UnsignedGreaterThan, i, lo);
        let bb = self.f.create_block();
        let ab = self.f.create_block();
        self.f.append_block_param(ab, types::I8);
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(has_before, bb, &[], ab, &[no.into()]);
        self.f.switch_to_block(bb);
        let im = self.f.ins().iadd_imm(i, -1);
        let before = self.body_pasted(ds, d, im);
        self.f.ins().jump(ab, &[before.into()]);
        self.f.switch_to_block(ab);
        let before = self.f.block_params(ab)[0];
        let after_b = self.f.create_block();
        let both = self.f.create_block();
        self.f.append_block_param(both, types::I8);
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(next_in, after_b, &[], both, &[no.into()]);
        self.f.switch_to_block(after_b);
        let after = self.body_pasted(ds, d, i1);
        self.f.ins().jump(both, &[after.into()]);
        self.f.switch_to_block(both);
        let after = self.f.block_params(both)[0];
        let pasted = self.f.ins().bor(before, after);
        let record = self.f.create_block();
        self.f.ins().brif(pasted, done, &[], record, &[]);
        self.f.switch_to_block(record);
        let up = self.vpush(USES, pc);
        st(self.f, types::I16, m0p, up, USE.1 as i32);
        st(self.f, types::I8, inside, up, USE.2 as i32);
        self.f.ins().jump(done, &[]);
        self.f.switch_to_block(done);
        let ds = self.def_state();
        st(self.f, types::I32, i1, ds, ds!(i));
        self.f.ins().jump(next, &[]);
    }

    /// `UNDEF` (`Machine::undef`): the name's slot undefined; the line passed.
    fn undef(&mut self, pc: usize) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        let (held, _) = self.line(pc);
        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
        let ident = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
        let named = self.f.ins().band(held, ident);
        let find = self.f.create_block();
        let pass = self.f.create_block();
        self.f.ins().brif(named, find, &[], pass, &[]);
        self.f.switch_to_block(find);
        let (p, n) = self.text();
        let (slot, found) = self.probe(pc, p, n);
        let clear = self.f.create_block();
        self.f.ins().brif(found, clear, &[], pass, &[]);
        self.f.switch_to_block(clear);
        let none = self.f.ins().iconst(types::I32, i64::from(NONE));
        self.f.ins().store(flags(), none, slot, offset_of!(Slot, def) as i32);
        self.f.ins().jump(pass, &[]);
        self.f.switch_to_block(pass);
        self.skip_line_then(pc, next);
    }

    /// `DEFINED` (`Machine::defined`), a test: `T` is `defined` (not painted); its operand read
    /// (`X` or `( X )`), and `T` the number `1` or `0`.
    fn defined(&mut self, pc: usize) {
        let ctx = self.ctx;
        let (next, skip) = (self.blocks[pc + 1], self.blocks[pc + 2]);
        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
        let fl = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, flags) as i32);
        let ident = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
        let painted = self.f.ins().band_imm(fl, i64::from(NOEXP));
        let unpainted = self.f.ins().icmp_imm(IntCC::Equal, painted, 0);
        let go = self.f.ins().band(ident, unpainted);
        let spell = self.f.create_block();
        self.f.ins().brif(go, spell, &[], skip, &[]);
        self.f.switch_to_block(spell);
        let is = self.spelled(b"defined");
        let op = self.f.create_block();
        self.f.ins().brif(is, op, &[], skip, &[]);
        self.f.switch_to_block(op);
        // Its flags, for the number.
        let keep = self.f.ins().band_imm(fl, i64::from(BOL | SPACE));
        let r1 = self.f.create_block();
        self.read_then(r1);
        self.f.switch_to_block(r1);
        let paren = self.spelled(b"(");
        let r2 = self.f.create_block();
        let name = self.f.create_block();
        self.f.ins().brif(paren, r2, &[], name, &[]);
        self.f.switch_to_block(r2);
        self.read_then(name);
        self.f.switch_to_block(name);
        let k = ldu(self.f, types::I8, ctx, at!(t) + offset_of!(Tok, kind) as i32);
        let ident = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
        let v = self.f.create_block();
        self.f.append_block_param(v, types::I8);
        let find = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(ident, find, &[], v, &[no.into()]);
        self.f.switch_to_block(find);
        let (p, n) = self.text();
        let (slot, found) = self.probe(pc, p, n);
        let check = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(found, check, &[], v, &[no.into()]);
        self.f.switch_to_block(check);
        let def = ldu(self.f, types::I32, slot, offset_of!(Slot, def) as i32);
        let is_def = self.f.ins().icmp_imm(IntCC::NotEqual, def, i64::from(NONE));
        self.f.ins().jump(v, &[is_def.into()]);
        self.f.switch_to_block(v);
        let val = self.f.block_params(v)[0];
        let close = self.f.create_block();
        let number = self.f.create_block();
        self.f.ins().brif(paren, close, &[], number, &[]);
        self.f.switch_to_block(close);
        self.read_then(number);
        self.f.switch_to_block(number);
        let one = self.f.create_block();
        let zero = self.f.create_block();
        let done = self.f.create_block();
        self.f.ins().brif(val, one, &[], zero, &[]);
        for (b, s) in [(one, b"1"), (zero, b"0")] {
            self.f.switch_to_block(b);
            self.scratch_tok(pc, NUMBER, s, ctx, at!(t));
            self.f.ins().jump(done, &[]);
        }
        self.f.switch_to_block(done);
        st(self.f, types::I8, keep, ctx, at!(t) + offset_of!(Tok, flags) as i32);
        self.f.ins().jump(next, &[]);
    }

    fn sub_state(&mut self) -> Value {
        ld(self.f, types::I64, self.ctx, at!(sub))
    }

    /// A token's six fields stored at `to + off`.
    #[allow(clippy::too_many_arguments)]
    fn put_tok(&mut self, to: Value, off: i32, kind: Value, fl: Value, buf: Value, start: Value, end: Value, exp: Value) {
        st(self.f, types::I8, kind, to, off + T_KIND);
        st(self.f, types::I8, fl, to, off + T_FLAGS);
        st(self.f, types::I32, buf, to, off + offset_of!(Tok, buf) as i32);
        st(self.f, types::I32, start, to, off + offset_of!(Tok, start) as i32);
        st(self.f, types::I32, end, to, off + offset_of!(Tok, end) as i32);
        st(self.f, types::I32, exp, to, off + offset_of!(Tok, exp) as i32);
    }

    /// `flags = (flags & !clear) | (from & keep)` on the token at `to + off`.
    fn set_flags(&mut self, to: Value, off: i32, clear: u8, from: Value, keep: u8) {
        let fl = ldu(self.f, types::I8, to, off + T_FLAGS);
        let a = self.f.ins().band_imm(fl, !i64::from(clear));
        let b = self.f.ins().band_imm(from, i64::from(keep));
        let x = self.f.ins().bor(a, b);
        st(self.f, types::I8, x, to, off + T_FLAGS);
    }

    /// The token at `from + off` pushed to the arena, its flags then `PLACEMARKER` alone where
    /// `placemarker`, its expansion `exp` where given.
    fn push_tok_as(&mut self, pc: usize, from: Value, off: i32, placemarker: bool, exp: Option<Value>) {
        let p = self.vpush(ARENA, pc);
        copy_tok(self.f, from, off, p, 0);
        if placemarker {
            let pm = self.f.ins().iconst(types::I8, i64::from(PLACEMARKER));
            self.f.ins().store(flags(), pm, p, T_FLAGS);
        }
        if let Some(e) = exp {
            st(self.f, types::I32, e, p, offset_of!(Tok, exp) as i32);
        }
    }

    /// The constant bytes `b` appended to scratch (`Machine::scratch_push`).
    fn scratch_push(&mut self, pc: usize, b: &[u8]) {
        let ctx = self.ctx;
        let n = ld(self.f, types::I64, ctx, at!(nscratch));
        let room = ld(self.f, types::I64, ctx, at!(room_scratch));
        let end = self.f.ins().iadd_imm(n, b.len() as i64);
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(ok, pc, R_SCRATCH);
        let base = ld(self.f, types::I64, ctx, at!(scratch));
        let p = self.f.ins().iadd(base, n);
        // Eight bytes a store, then the rest one by one.
        let words = b.len() / 8;
        for (i, w) in b.chunks_exact(8).enumerate() {
            let c = self.f.ins().iconst(types::I64, i64::from_le_bytes(w.try_into().unwrap()));
            self.f.ins().store(bytes_flags(), c, p, (i * 8) as i32);
        }
        for (i, &c) in b[words * 8..].iter().enumerate() {
            let c = self.f.ins().iconst(types::I8, i64::from(c));
            self.f.ins().store(bytes_flags(), c, p, (words * 8 + i) as i32);
        }
        st(self.f, types::I64, end, ctx, at!(nscratch));
    }

    /// `w`'s first `k` bytes (`k` at least 1) stored at `p`, at a byte buffer's end: one store of
    /// all eight, the bytes past `k` past the buffer's length, in the eight bytes of capacity a
    /// byte buffer has past its room (`pp_words::run_native`), where the next bytes go.
    fn store_bytes(&mut self, p: Value, w: Value, _k: Value) {
        self.f.ins().store(bytes_flags(), w, p, 0);
    }

    /// The high bit of each of a word's bytes that is zero (exact: no borrow between bytes).
    fn zero_bytes(&mut self, x: Value) -> Value {
        let f = &mut *self.f;
        let a = f.ins().band_imm(x, LOW7);
        let a = f.ins().iadd_imm(a, LOW7);
        let a = f.ins().bor(a, x);
        let a = f.ins().bor_imm(a, LOW7);
        f.ins().bnot(a)
    }

    /// The high bit of each byte equal to `c`.
    fn bytes_eq(&mut self, w: Value, c: u8) -> Value {
        let x = self.f.ins().bxor_imm(w, ONES.wrapping_mul(i64::from(c)));
        self.zero_bytes(x)
    }

    /// The high bit of each byte `c` with `lo < c < hi` (`hi` at most 128; exact, bytes of
    /// 128 and more clear).
    fn bytes_between(&mut self, w: Value, lo: u8, hi: u8) -> Value {
        let f = &mut *self.f;
        let low = f.ins().band_imm(w, LOW7);
        let top = f.ins().iconst(types::I64, ONES.wrapping_mul(127 + i64::from(hi)));
        let a = f.ins().isub(top, low);
        let nw = f.ins().bnot(w);
        let a = f.ins().band(a, nw);
        let b = f.ins().iadd_imm(low, ONES.wrapping_mul(127 - i64::from(lo)));
        let a = f.ins().band(a, b);
        f.ins().band_imm(a, HIGH)
    }

    /// The high bits of a word's first `k` bytes (`k` from 1).
    fn valid_bytes(&mut self, k: Value) -> Value {
        let f = &mut *self.f;
        let bits = f.ins().ishl_imm(k, 3);
        let one = f.ins().iconst(types::I64, 1);
        let m = f.ins().ishl(one, bits);
        let m = f.ins().iadd_imm(m, -1);
        let all = f.ins().iconst(types::I64, -1);
        let eight = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, k, 8);
        let m = f.ins().select(eight, all, m);
        f.ins().band_imm(m, HIGH)
    }

    /// Whether every one of the `n` bytes at `p` is in the class `class` makes the high bits of
    /// (a word's bytes), eight a step (`Step::Word`).
    fn all_bytes(&mut self, p: Value, n: Value, class: fn(&mut Self, Value) -> Value) -> Value {
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
        let q = self.f.ins().iadd(p, i);
        let w = self.chunk(q, rem);
        let m = class(self, w);
        let valid = self.valid_bytes(rem);
        let m = self.f.ins().band(m, valid);
        let ok = self.f.ins().icmp(IntCC::Equal, m, valid);
        let i8 = self.f.ins().iadd_imm(i, 8);
        let on = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(ok, on, &[], join, &[no.into()]);
        self.f.switch_to_block(on);
        self.f.ins().jump(head, &[i8.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// `pp_words::word_byte`'s bytes: letters, digits, `_` and `$`.
    fn word_class(&mut self, w: Value) -> Value {
        let a = self.bytes_between(w, b'0' - 1, b'9' + 1);
        let b = self.bytes_between(w, b'A' - 1, b'Z' + 1);
        let c = self.bytes_between(w, b'a' - 1, b'z' + 1);
        let d = self.bytes_eq(w, b'_');
        let e = self.bytes_eq(w, b'$');
        let f = &mut *self.f;
        let x = f.ins().bor(a, b);
        let x = f.ins().bor(x, c);
        let x = f.ins().bor(x, d);
        f.ins().bor(x, e)
    }

    fn dot_class(&mut self, w: Value) -> Value {
        self.bytes_eq(w, b'.')
    }

    /// The `n` bytes at `p` appended to scratch (`Machine::copy_spell`), `"` and `\\` escaped
    /// where `escape`: eight bytes a step, or the bytes before an escaped one and it
    /// (`Step::Word`); the worst case checked against the bound first.
    fn scratch_append(&mut self, pc: usize, p: Value, n: Value, escape: bool) {
        self.append_bytes(pc, (at!(scratch), at!(nscratch), at!(room_scratch)), R_SCRATCH, p, n, escape);
    }

    /// `scratch_append` to the byte buffer whose start's, length's and room's fields are `buf`.
    fn append_bytes(&mut self, pc: usize, (bp, bn, br): (i32, i32, i32), why: u64, p: Value, n: Value, escape: bool) {
        let ctx = self.ctx;
        let n0 = ld(self.f, types::I64, ctx, bn);
        let room = ld(self.f, types::I64, ctx, br);
        let worst = if escape { self.f.ins().ishl_imm(n, 1) } else { n };
        let end = self.f.ins().iadd(n0, worst);
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(ok, pc, why);
        let base = ld(self.f, types::I64, ctx, bp);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Word));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into(), n0.into()]);
        self.f.switch_to_block(head);
        let (at, d) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, at, n);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[d.into()]);
        self.f.switch_to_block(step);
        let rem = self.f.ins().isub(n, at);
        let src = self.f.ins().iadd(p, at);
        let w = self.chunk(src, rem);
        let eight = self.f.ins().iconst(types::I64, 8);
        let k = self.f.ins().umin(rem, eight);
        let dst = self.f.ins().iadd(base, d);
        let plain = self.f.create_block();
        if escape {
            let q = self.bytes_eq(w, b'"');
            let b = self.bytes_eq(w, b'\\');
            let m = self.f.ins().bor(q, b);
            let valid = self.valid_bytes(k);
            let m = self.f.ins().band(m, valid);
            let esc = self.f.create_block();
            self.f.ins().brif(m, esc, &[], plain, &[]);
            self.f.switch_to_block(esc);
            let tz = self.f.ins().ctz(m);
            let j = self.f.ins().ushr_imm(tz, 3);
            let some = self.f.create_block();
            let put = self.f.create_block();
            self.f.ins().brif(j, some, &[], put, &[]);
            self.f.switch_to_block(some);
            self.store_bytes(dst, w, j);
            self.f.ins().jump(put, &[]);
            self.f.switch_to_block(put);
            let sh = self.f.ins().ishl_imm(j, 3);
            let c = self.f.ins().ushr(w, sh);
            let c = self.f.ins().ireduce(types::I8, c);
            let at_c = self.f.ins().iadd(dst, j);
            let bs = self.f.ins().iconst(types::I8, i64::from(b'\\'));
            self.f.ins().store(bytes_flags(), bs, at_c, 0);
            self.f.ins().store(bytes_flags(), c, at_c, 1);
            let j1 = self.f.ins().iadd_imm(j, 1);
            let at1 = self.f.ins().iadd(at, j1);
            let j2 = self.f.ins().iadd_imm(j, 2);
            let d1 = self.f.ins().iadd(d, j2);
            self.f.ins().jump(head, &[at1.into(), d1.into()]);
        } else {
            self.f.ins().jump(plain, &[]);
        }
        self.f.switch_to_block(plain);
        self.store_bytes(dst, w, k);
        let at1 = self.f.ins().iadd(at, k);
        let d1 = self.f.ins().iadd(d, k);
        self.f.ins().jump(head, &[at1.into(), d1.into()]);
        self.f.switch_to_block(done);
        let d = self.f.block_params(done)[0];
        st(self.f, types::I64, d, ctx, bn);
    }

    /// The token at `tok + off`'s spelling appended to scratch (`Machine::scratch_copy`).
    fn scratch_copy(&mut self, pc: usize, tok: Value, off: i32, escape: bool) {
        let (p, n) = self.spelling(tok, off);
        self.scratch_append(pc, p, n, escape);
    }

    /// `Machine::retain_from`: the arena's placemarkers from `at` on dropped, a token a step
    /// (`Step::Item`).
    fn retain_from(&mut self, at: Value) {
        let n = self.vlen(ARENA);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        self.f.ins().jump(head, &[at.into(), at.into()]);
        self.f.switch_to_block(head);
        let (r, w) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, r, n);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[w.into()]);
        self.f.switch_to_block(step);
        let rp = self.vat(ARENA, r);
        let fl = ldu(self.f, types::I8, rp, T_FLAGS);
        let pm = self.f.ins().band_imm(fl, i64::from(PLACEMARKER));
        let r1 = self.f.ins().iadd_imm(r, 1);
        let keep = self.f.create_block();
        self.f.ins().brif(pm, head, &[r1.into(), w.into()], keep, &[]);
        self.f.switch_to_block(keep);
        let wp = self.vat(ARENA, w);
        copy_tok(self.f, rp, 0, wp, 0);
        let w1 = self.f.ins().iadd_imm(w, 1);
        self.f.ins().jump(head, &[r1.into(), w1.into()]);
        self.f.switch_to_block(done);
        let w = self.f.block_params(done)[0];
        self.vset_len(ARENA, w);
    }

    /// `Machine::body_tok`: body token `k` of the definition at `def` into `to + off`: the
    /// first without space before it, a `##` its paste mark.
    fn load_body_tok(&mut self, def: Value, k: Value, to: Value, off: i32) {
        let t = self.body_tok(def, 0, k);
        copy_tok(self.f, t, 0, to, off);
        let m = self.body_mark(def, 0, k);
        let fl = ldu(self.f, types::I8, to, off + T_FLAGS);
        let first = self.f.ins().icmp_imm(IntCC::Equal, k, 0);
        let cleared = self.f.ins().band_imm(fl, !i64::from(SPACE));
        let fl = self.f.ins().select(first, cleared, fl);
        let pm = self.f.ins().band_imm(m, i64::from(PASTE_MARK));
        let pasted = self.f.ins().bor_imm(fl, i64::from(PASTE));
        let fl = self.f.ins().select(pm, pasted, fl);
        st(self.f, types::I8, fl, to, off + T_FLAGS);
    }

    /// Body token `k`'s parameter (its mark less the paste flag).
    fn body_param(&mut self, def: Value, k: Value) -> Value {
        let m = self.body_mark(def, 0, k);
        self.f.ins().band_imm(m, 0xffff)
    }

    /// `SUB` (`Machine::sub`), its modes.
    fn sub(&mut self, pc: usize, mode: u8) {
        let next = self.blocks[pc + 1];
        let skip = self.blocks.get(pc + 2).copied().unwrap_or(next);
        let fr = self.vtop(FRAMES, pc, R_SUB_FRAME);
        let def = self.frame_def(fr);
        let sv = self.sub_state();
        let i = ldu(self.f, types::I32, sv, sb!(i));
        let hi = ldu(self.f, types::I32, sv, sb!(hi));
        let at = ldu(self.f, types::I32, sv, sb!(at));
        let out = ldu(self.f, types::I32, sv, sb!(out));
        let bt = sb!(bt);
        match mode {
            0 => {
                let e = self.vpush(EXPS, pc);
                copy_tok(self.f, fr, offset_of!(Frame, name) as i32, e, EXP.1 as i32);
                copy_tok(self.f, fr, offset_of!(Frame, end) as i32, e, EXP.2 as i32);
                let z = self.f.ins().iconst(types::I32, 0);
                for w in 0..(size_of::<Sub>() as i32 / 4) {
                    self.f.ins().store(flags(), z, sv, 4 * w);
                }
                let id = self.vlen(EXPS);
                let de = ldu(self.f, types::I32, def, offset_of!(Def, end) as i32);
                let df = ldu(self.f, types::I32, def, offset_of!(Def, first) as i32);
                let n = self.f.ins().isub(de, df);
                let a = self.vlen(ARENA);
                st(self.f, types::I32, id, sv, sb!(id));
                st(self.f, types::I32, n, sv, sb!(hi));
                st(self.f, types::I32, a, sv, sb!(base));
                st(self.f, types::I32, a, sv, sb!(out));
                self.f.ins().jump(next, &[]);
            }
            1 => {
                let end = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, hi);
                let take = self.f.create_block();
                self.f.ins().brif(end, skip, &[], take, &[]);
                self.f.switch_to_block(take);
                self.load_body_tok(def, i, sv, bt);
                st(self.f, types::I32, i, sv, sb!(at));
                let i1 = self.f.ins().iadd_imm(i, 1);
                st(self.f, types::I32, i1, sv, sb!(i));
                self.f.ins().jump(next, &[]);
            }
            2 => {
                // `#` and a parameter: the argument's raw range to stringize.
                let fnlike = ldu(self.f, types::I8, def, offset_of!(Def, fnlike) as i32);
                let k = ldu(self.f, types::I8, sv, bt + T_KIND);
                let hash = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(HASH));
                let a1 = self.f.ins().iadd_imm(at, 1);
                let within = self.f.ins().icmp(IntCC::UnsignedLessThan, a1, hi);
                let fl = self.f.ins().icmp_imm(IntCC::NotEqual, fnlike, 0);
                let go = self.f.ins().band(fl, hash);
                let go = self.f.ins().band(go, within);
                let look = self.f.create_block();
                self.f.ins().brif(go, look, &[], skip, &[]);
                self.f.switch_to_block(look);
                let p = self.body_param(def, a1);
                let is = self.f.ins().icmp_imm(IntCC::UnsignedLessThan, p, i64::from(VA_OPT_MARK));
                let yes = self.f.create_block();
                self.f.ins().brif(is, yes, &[], skip, &[]);
                self.f.switch_to_block(yes);
                let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
                let ai = self.f.ins().iadd(args, p);
                let ap = self.vat(ARGV, ai);
                let (a, b) = self.pair(ap);
                let a2 = self.f.ins().iadd_imm(at, 2);
                st(self.f, types::I32, a2, sv, sb!(i));
                st(self.f, types::I32, a, sv, sb!(s.0));
                st(self.f, types::I32, b, sv, sb!(s.1));
                copy_tok(self.f, sv, bt, sv, sb!(like));
                self.f.ins().jump(next, &[]);
            }
            3 | 4 => self.sub_va_opt(pc, mode, fr, def),
            5 => self.substitute_one(pc, fr, def),
            6 => {
                let cs = ldu(self.f, types::I8, sv, sb!(closed_str));
                let yes = self.f.create_block();
                self.f.ins().brif(cs, yes, &[], skip, &[]);
                self.f.switch_to_block(yes);
                let ca = ldu(self.f, types::I32, sv, sb!(closed_at));
                let n = self.vlen(ARENA);
                for o in [sb!(ps), sb!(r), sb!(w)] {
                    st(self.f, types::I32, ca, sv, o);
                }
                st(self.f, types::I32, n, sv, sb!(end));
                self.f.ins().jump(next, &[]);
            }
            7 => {
                // A `__VA_OPT__`'s content closed: the walk after it.
                let open = ldu(self.f, types::I8, sv, sb!(va_open));
                let yes = self.f.create_block();
                self.f.ins().brif(open, yes, &[], skip, &[]);
                self.f.switch_to_block(yes);
                let vs = ldu(self.f, types::I8, sv, sb!(va_str));
                let n = self.vlen(ARENA);
                let inside = self.f.ins().icmp(IntCC::UnsignedLessThan, out, n);
                let not_str = self.f.ins().icmp_imm(IntCC::Equal, vs, 0);
                let fix = self.f.ins().band(inside, not_str);
                let fixb = self.f.create_block();
                let close = self.f.create_block();
                self.f.ins().brif(fix, fixb, &[], close, &[]);
                self.f.switch_to_block(fixb);
                let xp = self.vat(ARENA, out);
                let vf = ldu(self.f, types::I8, sv, sb!(va_bt) + T_FLAGS);
                self.set_flags(xp, 0, SPACE, vf, SPACE);
                self.f.ins().jump(close, &[]);
                self.f.switch_to_block(close);
                for (from, to) in [(sb!(va_lo), sb!(lo)), (sb!(va_i), sb!(i)), (sb!(va_hi), sb!(hi)), (sb!(va_out), sb!(out))] {
                    let x = self.f.ins().load(types::I32, flags(), sv, from);
                    self.f.ins().store(flags(), x, sv, to);
                }
                let z = self.f.ins().iconst(types::I8, 0);
                self.f.ins().store(flags(), z, sv, sb!(va_open));
                st(self.f, types::I32, out, sv, sb!(closed_at));
                st(self.f, types::I8, vs, sv, sb!(closed_str));
                copy_tok(self.f, sv, sb!(va_bt), sv, sb!(closed_bt));
                self.f.ins().jump(next, &[]);
            }
            8 => {
                let ca = ldu(self.f, types::I32, sv, sb!(closed_at));
                self.retain_from(ca);
                let n = self.vlen(ARENA);
                st(self.f, types::I32, ca, sv, sb!(s.0));
                st(self.f, types::I32, n, sv, sb!(s.1));
                copy_tok(self.f, sv, sb!(closed_bt), sv, sb!(like));
                self.f.ins().jump(next, &[]);
            }
            9 => {
                let base = ldu(self.f, types::I32, sv, sb!(base));
                let n = self.vlen(ARENA);
                for o in [sb!(ps), sb!(r), sb!(w)] {
                    st(self.f, types::I32, base, sv, o);
                }
                st(self.f, types::I32, n, sv, sb!(end));
                self.f.ins().jump(next, &[]);
            }
            _ => {
                // The output a source to rescan, its macro disabled.
                let base = ldu(self.f, types::I32, sv, sb!(base));
                self.retain_from(base);
                let n = self.vlen(ARENA);
                let nf = ldu(self.f, types::I8, fr, offset_of!(Frame, name) as i32 + T_FLAGS);
                let d = ldu(self.f, types::I32, fr, offset_of!(Frame, def) as i32);
                let some = self.f.ins().icmp(IntCC::UnsignedLessThan, base, n);
                let fixb = self.f.create_block();
                let on = self.f.create_block();
                self.f.ins().brif(some, fixb, &[], on, &[]);
                self.f.switch_to_block(fixb);
                let xp = self.vat(ARENA, base);
                self.set_flags(xp, 0, BOL | SPACE, nf, BOL | SPACE);
                self.f.ins().jump(on, &[]);
                self.f.switch_to_block(on);
                self.drop_frame(fr);
                let one = self.f.ins().iconst(types::I8, 1);
                self.f.ins().store(flags(), one, def, offset_of!(Def, disabled) as i32);
                self.push_arena_src(pc, base, n, d, false);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    /// `SUB m=3` (`#__VA_OPT__(…)`) and `m=4` (`__VA_OPT__(…)`), tests: the content walked in
    /// place of the body, or (not present, not stringized) a placemarker.
    fn sub_va_opt(&mut self, pc: usize, mode: u8, fr: Value, def: Value) {
        let (next, skip) = (self.blocks[pc + 1], self.blocks[pc + 2]);
        let sv = self.sub_state();
        let lo = ldu(self.f, types::I32, sv, sb!(lo));
        let hi = ldu(self.f, types::I32, sv, sb!(hi));
        let at = ldu(self.f, types::I32, sv, sb!(at));
        let out = ldu(self.f, types::I32, sv, sb!(out));
        let k = if mode == 3 { self.f.ins().iadd_imm(at, 1) } else { at };
        let variadic = ldu(self.f, types::I8, def, offset_of!(Def, variadic) as i32);
        let va = self.f.ins().icmp_imm(IntCC::NotEqual, variadic, 0);
        let within = self.f.ins().icmp(IntCC::UnsignedLessThan, k, hi);
        let go = self.f.ins().band(va, within);
        let go = if mode == 3 {
            let fnlike = ldu(self.f, types::I8, def, offset_of!(Def, fnlike) as i32);
            let fl = self.f.ins().icmp_imm(IntCC::NotEqual, fnlike, 0);
            let kind = ldu(self.f, types::I8, sv, sb!(bt) + T_KIND);
            let hash = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(HASH));
            let x = self.f.ins().band(fl, hash);
            self.f.ins().band(go, x)
        } else {
            go
        };
        let look = self.f.create_block();
        self.f.ins().brif(go, look, &[], skip, &[]);
        self.f.switch_to_block(look);
        let m = self.body_param(def, k);
        let is = self.f.ins().icmp_imm(IntCC::Equal, m, i64::from(VA_OPT_MARK));
        let yes = self.f.create_block();
        self.f.ins().brif(is, yes, &[], skip, &[]);
        self.f.switch_to_block(yes);
        let open = ldu(self.f, types::I8, sv, sb!(va_open));
        let flat = self.f.ins().icmp_imm(IntCC::Equal, open, 0);
        self.check(flat, pc, R_VA_OPT);
        let (clo, chi, end) = self.va_opt_content(def, 0, k, hi);
        let present = self.va_present(fr, def);
        let sv = self.sub_state();
        if mode == 4 {
            let absent = self.f.create_block();
            let walk = self.f.create_block();
            self.f.ins().brif(present, walk, &[], absent, &[]);
            self.f.switch_to_block(absent);
            self.push_tok_as(pc, sv, sb!(bt), true, None);
            st(self.f, types::I32, end, sv, sb!(i));
            self.f.ins().jump(next, &[]);
            self.f.switch_to_block(walk);
        }
        let a = self.vlen(ARENA);
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, sv, sb!(va_open));
        st(self.f, types::I32, lo, sv, sb!(va_lo));
        st(self.f, types::I32, end, sv, sb!(va_i));
        st(self.f, types::I32, hi, sv, sb!(va_hi));
        st(self.f, types::I32, out, sv, sb!(va_out));
        let vs = self.f.ins().iconst(types::I8, i64::from(mode == 3));
        self.f.ins().store(flags(), vs, sv, sb!(va_str));
        copy_tok(self.f, sv, sb!(bt), sv, sb!(va_bt));
        // Not present (stringized): its content empty.
        let nlo = self.f.ins().select(present, clo, chi);
        st(self.f, types::I32, nlo, sv, sb!(lo));
        st(self.f, types::I32, nlo, sv, sb!(i));
        st(self.f, types::I32, chi, sv, sb!(hi));
        st(self.f, types::I32, a, sv, sb!(out));
        self.f.ins().jump(next, &[]);
    }

    /// `SUB m=5` (`Machine::substitute_one`): `B` substituted, a parameter's argument (its
    /// tokens copied a step each, `Step::Item`) or `B` itself.
    fn substitute_one(&mut self, pc: usize, fr: Value, def: Value) {
        let next = self.blocks[pc + 1];
        let sv = self.sub_state();
        let lo = ldu(self.f, types::I32, sv, sb!(lo));
        let hi = ldu(self.f, types::I32, sv, sb!(hi));
        let i = ldu(self.f, types::I32, sv, sb!(at));
        let out = ldu(self.f, types::I32, sv, sb!(out));
        let id = ldu(self.f, types::I32, sv, sb!(id));
        let p = self.body_param(def, i);
        let is_param = self.f.ins().icmp_imm(IntCC::UnsignedLessThan, p, i64::from(VA_OPT_MARK));
        let param = self.f.create_block();
        let itself = self.f.create_block();
        self.f.ins().brif(is_param, param, &[], itself, &[]);
        self.f.switch_to_block(itself);
        self.push_tok_as(pc, sv, sb!(bt), false, Some(id));
        self.f.ins().jump(next, &[]);

        self.f.switch_to_block(param);
        let variadic = ldu(self.f, types::I8, def, offset_of!(Def, variadic) as i32);
        let params = ldu(self.f, types::I16, def, offset_of!(Def, params) as i32);
        let last = self.f.ins().iadd_imm(params, -1);
        let is_last = self.f.ins().icmp(IntCC::Equal, p, last);
        let vb = self.f.ins().icmp_imm(IntCC::NotEqual, variadic, 0);
        let is_va = self.f.ins().band(vb, is_last);
        // A `##` before it, after it.
        let bj = self.f.create_block();
        self.f.append_block_param(bj, types::I8);
        let has_before = self.f.ins().icmp(IntCC::UnsignedGreaterThan, i, lo);
        let bl = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(has_before, bl, &[], bj, &[no.into()]);
        self.f.switch_to_block(bl);
        let im = self.f.ins().iadd_imm(i, -1);
        let x = self.body_pasted(def, 0, im);
        self.f.ins().jump(bj, &[x.into()]);
        self.f.switch_to_block(bj);
        let before = self.f.block_params(bj)[0];
        let aj = self.f.create_block();
        self.f.append_block_param(aj, types::I8);
        let i1 = self.f.ins().iadd_imm(i, 1);
        let has_after = self.f.ins().icmp(IntCC::UnsignedLessThan, i1, hi);
        let al = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(has_after, al, &[], aj, &[no.into()]);
        self.f.switch_to_block(al);
        let x = self.body_pasted(def, 0, i1);
        self.f.ins().jump(aj, &[x.into()]);
        self.f.switch_to_block(aj);
        let after = self.f.block_params(aj)[0];
        let args = ldu(self.f, types::I32, fr, offset_of!(Frame, args) as i32);
        let ai = self.f.ins().iadd(args, p);
        let ap = self.vat(ARGV, ai);
        let (ra, rb) = self.pair(ap);
        // GNU's `, ## __VA_ARGS__`: the arena's last two a `,` and the `##`.
        let n = self.vlen(ARENA);
        let cj = self.f.create_block();
        self.f.append_block_param(cj, types::I8);
        let o2 = self.f.ins().iadd_imm(out, 2);
        let room = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, n, o2);
        let bv = self.f.ins().band(before, is_va);
        let go = self.f.ins().band(room, bv);
        let cl = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(go, cl, &[], cj, &[no.into()]);
        self.f.switch_to_block(cl);
        let n2 = self.f.ins().iadd_imm(n, -2);
        let n1 = self.f.ins().iadd_imm(n, -1);
        let t2 = self.vat(ARENA, n2);
        let t1 = self.vat(ARENA, n1);
        let comma = self.tok_spelled(t2, 0, b",");
        let k2 = ldu(self.f, types::I8, t2, T_KIND);
        let punct = self.f.ins().icmp_imm(IntCC::Equal, k2, i64::from(PUNCT));
        let f1 = ldu(self.f, types::I8, t1, T_FLAGS);
        let pf = self.f.ins().band_imm(f1, i64::from(PASTE));
        let pasted = self.f.ins().icmp_imm(IntCC::NotEqual, pf, 0);
        let c = self.f.ins().band(comma, punct);
        let c = self.f.ins().band(c, pasted);
        self.f.ins().jump(cj, &[c.into()]);
        self.f.switch_to_block(cj);
        let comma_paste = self.f.block_params(cj)[0];
        let empty = self.f.ins().icmp(IntCC::Equal, ra, rb);
        let drop2 = self.f.ins().band(comma_paste, empty);
        let gone = self.f.create_block();
        let kept = self.f.create_block();
        self.f.ins().brif(drop2, gone, &[], kept, &[]);
        // ...with no variadic arguments the comma goes...
        self.f.switch_to_block(gone);
        let n2 = self.f.ins().iadd_imm(n, -2);
        self.vset_len(ARENA, n2);
        self.f.ins().jump(next, &[]);
        // ...and with some, the `##` is no paste.
        self.f.switch_to_block(kept);
        let n1 = self.f.ins().iadd_imm(n, -1);
        let nn = self.f.ins().select(comma_paste, n1, n);
        self.vset_len(ARENA, nn);
        let raw_b = self.f.ins().bor(before, after);
        let range = self.f.create_block();
        self.f.append_block_param(range, types::I64);
        self.f.append_block_param(range, types::I64);
        let expanded = self.f.create_block();
        self.f.ins().brif(raw_b, range, &[ra.into(), rb.into()], expanded, &[]);
        self.f.switch_to_block(expanded);
        let pres = ldu(self.f, types::I32, fr, offset_of!(Frame, pres) as i32);
        let pi = self.f.ins().iadd(pres, p);
        let pp = self.vat(PRES, pi);
        let (x, y) = self.pair(pp);
        let made = self.f.ins().icmp_imm(IntCC::NotEqual, x, i64::from(NONE));
        self.check(made, pc, R_EARLY);
        self.f.ins().jump(range, &[x.into(), y.into()]);
        self.f.switch_to_block(range);
        let (a, b) = (self.f.block_params(range)[0], self.f.block_params(range)[1]);
        let none = self.f.ins().icmp(IntCC::Equal, a, b);
        let pm = self.f.create_block();
        let copy = self.f.create_block();
        self.f.ins().brif(none, pm, &[], copy, &[]);
        self.f.switch_to_block(pm);
        let sv = self.sub_state();
        self.push_tok_as(pc, sv, sb!(bt), true, None);
        self.f.ins().jump(next, &[]);
        self.f.switch_to_block(copy);
        let k = self.vlen(ARENA);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let done = self.f.create_block();
        self.f.ins().jump(head, &[a.into()]);
        self.f.switch_to_block(head);
        let x = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, x, b);
        let body = self.f.create_block();
        self.f.ins().brif(more, body, &[], done, &[]);
        self.f.switch_to_block(body);
        let xp = self.vat(ARENA, x);
        self.push_tok_as(pc, xp, 0, false, None);
        let x1 = self.f.ins().iadd_imm(x, 1);
        self.f.ins().jump(head, &[x1.into()]);
        self.f.switch_to_block(done);
        let kp = self.vat(ARENA, k);
        let sv = self.sub_state();
        let bf = ldu(self.f, types::I8, sv, sb!(bt) + T_FLAGS);
        self.set_flags(kp, 0, BOL | SPACE, bf, SPACE);
        self.f.ins().jump(next, &[]);
    }

    /// `STR` (`Machine::str_word`), its modes: a stringized literal begun, an argument's token
    /// added (a test), the literal closed.
    fn str_word(&mut self, pc: usize, mode: u8) {
        let next = self.blocks[pc + 1];
        let sv = self.sub_state();
        match mode {
            0 => {
                let s0 = ldu(self.f, types::I32, sv, sb!(s.0));
                st(self.f, types::I32, s0, sv, sb!(s_first));
                let n = ld(self.f, types::I64, self.ctx, at!(nscratch));
                st(self.f, types::I32, n, sv, sb!(sb));
                self.scratch_push(pc, b"\"");
                self.f.ins().jump(next, &[]);
            }
            1 => {
                let skip = self.blocks[pc + 2];
                let s0 = ldu(self.f, types::I32, sv, sb!(s.0));
                let s1 = ldu(self.f, types::I32, sv, sb!(s.1));
                let end = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s0, s1);
                let take = self.f.create_block();
                self.f.ins().brif(end, skip, &[], take, &[]);
                self.f.switch_to_block(take);
                let t = self.vat(ARENA, s0);
                let s01 = self.f.ins().iadd_imm(s0, 1);
                st(self.f, types::I32, s01, sv, sb!(s.0));
                let fl = ldu(self.f, types::I8, t, T_FLAGS);
                let pm = self.f.ins().band_imm(fl, i64::from(PLACEMARKER));
                let real = self.f.create_block();
                self.f.ins().brif(pm, next, &[], real, &[]);
                self.f.switch_to_block(real);
                let first = ldu(self.f, types::I32, sv, sb!(s_first));
                let sbv = ldu(self.f, types::I32, sv, sb!(sb));
                let n = ld(self.f, types::I64, self.ctx, at!(nscratch));
                let later = self.f.ins().icmp(IntCC::UnsignedGreaterThan, s0, first);
                let sp = self.f.ins().band_imm(fl, i64::from(SPACE | BOL));
                let spaced = self.f.ins().icmp_imm(IntCC::NotEqual, sp, 0);
                let sb1 = self.f.ins().iadd_imm(sbv, 1);
                let begun = self.f.ins().icmp(IntCC::UnsignedGreaterThan, n, sb1);
                let gap = self.f.ins().band(later, spaced);
                let gap = self.f.ins().band(gap, begun);
                let space = self.f.create_block();
                let copy = self.f.create_block();
                self.f.ins().brif(gap, space, &[], copy, &[]);
                self.f.switch_to_block(space);
                self.scratch_push(pc, b" ");
                self.f.ins().jump(copy, &[]);
                self.f.switch_to_block(copy);
                let kind = ldu(self.f, types::I8, t, T_KIND);
                let lit = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(LITERAL));
                let esc = self.f.create_block();
                let plain = self.f.create_block();
                self.f.ins().brif(lit, esc, &[], plain, &[]);
                self.f.switch_to_block(esc);
                self.scratch_copy(pc, t, 0, true);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(plain);
                self.scratch_copy(pc, t, 0, false);
                self.f.ins().jump(next, &[]);
            }
            _ => {
                self.scratch_push(pc, b"\"");
                let sv = self.sub_state();
                if mode == 3 {
                    let first = ldu(self.f, types::I32, sv, sb!(s_first));
                    self.vset_len(ARENA, first);
                }
                let lf = ldu(self.f, types::I8, sv, sb!(like) + T_FLAGS);
                let fl = self.f.ins().band_imm(lf, i64::from(BOL | SPACE));
                let start = ldu(self.f, types::I32, sv, sb!(sb));
                let id = ldu(self.f, types::I32, sv, sb!(id));
                let end = ld(self.f, types::I64, self.ctx, at!(nscratch));
                let p = self.vpush(ARENA, pc);
                let k = self.f.ins().iconst(types::I8, i64::from(LITERAL));
                let z = self.f.ins().iconst(types::I32, 0);
                self.put_tok(p, 0, k, fl, z, start, end, id);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    /// `GLUE` (`Machine::paste_step`), a test: one step of the pastes of `arena[r..end]`,
    /// written from `w`; at the end the arena cut to `w`.
    fn glue(&mut self, pc: usize) {
        let (next, skip) = (self.blocks[pc + 1], self.blocks[pc + 2]);
        let sv = self.sub_state();
        let r = ldu(self.f, types::I32, sv, sb!(r));
        let w = ldu(self.f, types::I32, sv, sb!(w));
        let ps = ldu(self.f, types::I32, sv, sb!(ps));
        let end = ldu(self.f, types::I32, sv, sb!(end));
        let id = ldu(self.f, types::I32, sv, sb!(id));
        let done = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, r, end);
        let fin = self.f.create_block();
        let step = self.f.create_block();
        self.f.ins().brif(done, fin, &[], step, &[]);
        self.f.switch_to_block(fin);
        self.vset_len(ARENA, w);
        self.f.ins().jump(skip, &[]);
        self.f.switch_to_block(step);
        let t = self.vat(ARENA, r);
        let fl = ldu(self.f, types::I8, t, T_FLAGS);
        let pf = self.f.ins().band_imm(fl, i64::from(PASTE));
        let pasted = self.f.ins().icmp_imm(IntCC::NotEqual, pf, 0);
        let after = self.f.ins().icmp(IntCC::UnsignedGreaterThan, w, ps);
        let r1 = self.f.ins().iadd_imm(r, 1);
        let r2 = self.f.ins().iadd_imm(r, 2);
        let w1 = self.f.ins().iadd_imm(w, 1);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, r1, end);
        let go = self.f.ins().band(pasted, after);
        let go = self.f.ins().band(go, more);
        let paste = self.f.create_block();
        let plain = self.f.create_block();
        self.f.ins().brif(go, paste, &[], plain, &[]);
        self.f.switch_to_block(plain);
        let wp = self.vat(ARENA, w);
        copy_tok(self.f, t, 0, wp, 0);
        st(self.f, types::I32, w1, sv, sb!(w));
        st(self.f, types::I32, r1, sv, sb!(r));
        self.f.ins().jump(next, &[]);

        self.f.switch_to_block(paste);
        let wm = self.f.ins().iadd_imm(w, -1);
        let lp = self.vat(ARENA, wm);
        let rp = self.vat(ARENA, r1);
        let wp = self.vat(ARENA, w);
        st(self.f, types::I32, r2, sv, sb!(r));
        let lf = ldu(self.f, types::I8, lp, T_FLAGS);
        let rf = ldu(self.f, types::I8, rp, T_FLAGS);
        // A placemarker on the right: the left alone; on the left: the right, the left's spacing.
        let rpm = self.f.ins().band_imm(rf, i64::from(PLACEMARKER));
        let both = self.f.create_block();
        self.f.ins().brif(rpm, next, &[], both, &[]);
        self.f.switch_to_block(both);
        let lpm = self.f.ins().band_imm(lf, i64::from(PLACEMARKER));
        let left_pm = self.f.create_block();
        let spell = self.f.create_block();
        self.f.ins().brif(lpm, left_pm, &[], spell, &[]);
        self.f.switch_to_block(left_pm);
        copy_tok(self.f, rp, 0, lp, 0);
        let a = self.f.ins().band_imm(rf, !i64::from(BOL | SPACE));
        let b = self.f.ins().band_imm(lf, i64::from(BOL | SPACE));
        let x = self.f.ins().bor(a, b);
        st(self.f, types::I8, x, lp, T_FLAGS);
        self.f.ins().jump(next, &[]);
        // The two spelled together in scratch.
        self.f.switch_to_block(spell);
        let s0 = ld(self.f, types::I64, self.ctx, at!(nscratch));
        self.scratch_copy(pc, lp, 0, false);
        let m = ld(self.f, types::I64, self.ctx, at!(nscratch));
        self.scratch_copy(pc, rp, 0, false);
        let e = ld(self.f, types::I64, self.ctx, at!(nscratch));
        let base = ld(self.f, types::I64, self.ctx, at!(scratch));
        let lb = self.f.ins().iadd(base, s0);
        let ln = self.f.ins().isub(m, s0);
        let rb = self.f.ins().iadd(base, m);
        let rn = self.f.ins().isub(e, m);
        let tn = self.f.ins().isub(e, s0);
        let lk = ldu(self.f, types::I8, lp, T_KIND);
        let rk = ldu(self.f, types::I8, rp, T_KIND);
        let (has, kind) = self.pasted_kind(pc, lk, rk, (lb, ln), (rb, rn), tn);
        let one = self.f.create_block();
        let two = self.f.create_block();
        self.f.ins().brif(has, one, &[], two, &[]);
        // One token: in the left's place.
        self.f.switch_to_block(one);
        let fl = self.f.ins().band_imm(lf, i64::from(BOL | SPACE));
        let z = self.f.ins().iconst(types::I32, 0);
        self.put_tok(lp, 0, kind, fl, z, s0, e, id);
        self.f.ins().jump(next, &[]);
        // None: the right written after the left.
        self.f.switch_to_block(two);
        st(self.f, types::I64, s0, self.ctx, at!(nscratch));
        copy_tok(self.f, rp, 0, wp, 0);
        let sv = self.sub_state();
        st(self.f, types::I32, w1, sv, sb!(w));
        self.f.ins().jump(next, &[]);
    }

    /// `Machine::pasted_kind`: whether `l`'s and `r`'s bytes (`lb`, `rb`, and the two together,
    /// `tn` bytes from `lb`) spell one token, and its kind.
    fn pasted_kind(&mut self, pc: usize, lk: Value, rk: Value, (lp, ln): (Value, Value), (rp, rn): (Value, Value), tn: Value) -> (Value, Value) {
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        let none = |l: &mut Self| {
            let z8 = l.f.ins().iconst(types::I8, 0);
            let z = l.f.ins().iconst(types::I64, 0);
            l.f.ins().jump(join, &[z8.into(), z.into()]);
        };
        let kind = |l: &mut Self, k: u8| {
            let y = l.f.ins().iconst(types::I8, 1);
            let k = l.f.ins().iconst(types::I64, i64::from(k));
            l.f.ins().jump(join, &[y.into(), k.into()]);
        };
        let when = |l: &mut Self, c: Value, k: u8| {
            let yes = l.f.create_block();
            let no = l.f.create_block();
            l.f.ins().brif(c, yes, &[], no, &[]);
            l.f.switch_to_block(yes);
            kind(l, k);
            l.f.switch_to_block(no);
            none(l);
        };
        let is = |l: &mut Self, x: Value, k: u8| l.f.ins().icmp_imm(IntCC::Equal, x, i64::from(k));
        let branch = |l: &mut Self, c: Value| {
            let yes = l.f.create_block();
            let no = l.f.create_block();
            l.f.ins().brif(c, yes, &[], no, &[]);
            l.f.switch_to_block(yes);
            no
        };
        // (IDENT, …)
        let li = is(self, lk, IDENT);
        let not_ident = branch(self, li);
        let ri = is(self, rk, IDENT);
        let r_not_ident = branch(self, ri);
        kind(self, IDENT);
        self.f.switch_to_block(r_not_ident);
        let rnum = is(self, rk, NUMBER);
        let r_not_num = branch(self, rnum);
        let all = self.all_bytes(rp, rn, Self::word_class);
        when(self, all, IDENT);
        self.f.switch_to_block(r_not_num);
        let rlit = is(self, rk, LITERAL);
        let r_not_lit = branch(self, rlit);
        let some = self.f.ins().icmp_imm(IntCC::NotEqual, rn, 0);
        let r_empty = branch(self, some);
        let c0 = ldu(self.f, types::I8, rp, 0);
        let dq = self.f.ins().icmp_imm(IntCC::Equal, c0, i64::from(b'"'));
        let sq = self.f.ins().icmp_imm(IntCC::Equal, c0, i64::from(b'\''));
        let quoted = self.f.ins().bor(dq, sq);
        let not_quoted = branch(self, quoted);
        let mut prefix = self.f.ins().iconst(types::I8, 0);
        for p in [&b"L"[..], b"u", b"U", b"u8"] {
            let e = self.spelled_at(lp, ln, p);
            prefix = self.f.ins().bor(prefix, e);
        }
        let not_prefix = branch(self, prefix);
        kind(self, LITERAL);
        self.f.switch_to_block(not_prefix);
        let mut raw = self.f.ins().iconst(types::I8, 0);
        for p in [&b"R"[..], b"LR", b"uR", b"UR", b"u8R"] {
            let e = self.spelled_at(lp, ln, p);
            raw = self.f.ins().bor(raw, e);
        }
        let not_raw = branch(self, raw);
        self.refuse(pc, R_RAW);
        self.f.switch_to_block(not_raw);
        none(self);
        for b in [not_quoted, r_empty, r_not_lit] {
            self.f.switch_to_block(b);
            none(self);
        }
        // (NUMBER, …)
        self.f.switch_to_block(not_ident);
        let lnum = is(self, lk, NUMBER);
        let not_number = branch(self, lnum);
        let a = is(self, rk, IDENT);
        let b = is(self, rk, NUMBER);
        let ab = self.f.ins().bor(a, b);
        let not_word = branch(self, ab);
        kind(self, NUMBER);
        self.f.switch_to_block(not_word);
        let rpunct = is(self, rk, PUNCT);
        let not_punct = branch(self, rpunct);
        let dots = self.all_bytes(rp, rn, Self::dot_class);
        let not_dots = branch(self, dots);
        kind(self, NUMBER);
        self.f.switch_to_block(not_dots);
        let plus = self.spelled_at(rp, rn, b"+");
        let minus = self.spelled_at(rp, rn, b"-");
        let sign = self.f.ins().bor(plus, minus);
        let has_l = self.f.ins().icmp_imm(IntCC::NotEqual, ln, 0);
        let sign = self.f.ins().band(sign, has_l);
        let no_sign = branch(self, sign);
        let lastp = self.f.ins().iadd(lp, ln);
        let c = self.f.ins().load(types::I8, bytes_flags(), lastp, -1);
        let mut e = self.f.ins().iconst(types::I8, 0);
        for x in [b'e', b'E', b'p', b'P'] {
            let q = self.f.ins().icmp_imm(IntCC::Equal, c, i64::from(x));
            e = self.f.ins().bor(e, q);
        }
        when(self, e, NUMBER);
        self.f.switch_to_block(no_sign);
        none(self);
        self.f.switch_to_block(not_punct);
        none(self);
        // (PUNCT, NUMBER)
        self.f.switch_to_block(not_number);
        let lp_ = is(self, lk, PUNCT);
        let rn_ = is(self, rk, NUMBER);
        let pn = self.f.ins().band(lp_, rn_);
        let not_pn = branch(self, pn);
        let dot = self.spelled_at(lp, ln, b".");
        let some = self.f.ins().icmp_imm(IntCC::NotEqual, rn, 0);
        let go = self.f.ins().band(dot, some);
        let no_dot = branch(self, go);
        let c0 = ldu(self.f, types::I8, rp, 0);
        let dig = self.f.ins().iadd_imm(c0, -i64::from(b'0'));
        let digit = self.f.ins().icmp_imm(IntCC::UnsignedLessThan, dig, 10);
        when(self, digit, NUMBER);
        self.f.switch_to_block(no_dot);
        none(self);
        // (LITERAL, IDENT): a suffix, where the dialect has them.
        self.f.switch_to_block(not_pn);
        let ll = is(self, lk, LITERAL);
        let ri = is(self, rk, IDENT);
        let lr = self.f.ins().band(ll, ri);
        let not_lr = branch(self, lr);
        if self.pp.suffix {
            // C++'s: where the run is C++'s.
            let cpp = self.is_cpp();
            let (yes, no) = (self.f.create_block(), self.f.create_block());
            self.f.ins().brif(cpp, yes, &[], no, &[]);
            self.f.switch_to_block(yes);
            kind(self, LITERAL);
            self.f.switch_to_block(no);
            none(self);
        } else {
            none(self);
        }
        // Two punctuators: the dialect's punctuators, compared one by one (straight code).
        self.f.switch_to_block(not_lr);
        let op_like = |l: &mut Self, k: Value| {
            let mut x = l.f.ins().iconst(types::I8, 0);
            for c in [PUNCT, DIGRAPH, HASH, HASHHASH] {
                let q = l.f.ins().icmp_imm(IntCC::Equal, k, i64::from(c));
                x = l.f.ins().bor(x, q);
            }
            x
        };
        let a = op_like(self, lk);
        let b = op_like(self, rk);
        let ops = self.f.ins().band(a, b);
        let not_ops = branch(self, ops);
        // Then C++'s own, where the run is C++'s.
        let strings = |l: &Self, ops: &[u16]| -> Vec<Vec<u8>> { ops.iter().map(|&i| l.strings[usize::from(i)].clone()).collect() };
        let (ops, cpp_ops) = (strings(self, &self.pp.ops), strings(self, &self.pp.cpp_ops));
        for (k, list) in [ops, cpp_ops].into_iter().enumerate() {
            if k == 1 {
                if list.is_empty() {
                    break;
                }
                let cpp = self.is_cpp();
                let (yes, no) = (self.f.create_block(), self.f.create_block());
                self.f.ins().brif(cpp, yes, &[], no, &[]);
                self.f.switch_to_block(no);
                none(self);
                self.f.switch_to_block(yes);
            }
            for op in list {
                let eq = self.spelled_at(lp, tn, &op);
                let k = match &op[..] {
                    b"#" | b"%:" => HASH,
                    b"##" | b"%:%:" => HASHHASH,
                    b"<:" | b":>" | b"<%" | b"%>" => DIGRAPH,
                    _ => PUNCT,
                };
                let not_this = branch(self, eq);
                kind(self, k);
                self.f.switch_to_block(not_this);
            }
        }
        none(self);
        self.f.switch_to_block(not_ops);
        none(self);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    /// Whether the run is C++'s (`Env::cpp`), as an `i8` 0 or 1.
    fn is_cpp(&mut self) -> Value {
        let c = ld(self.f, types::I64, self.ctx, at!(cpp));
        self.f.ins().icmp_imm(IntCC::NotEqual, c, 0)
    }

    /// `Machine::file_src`: the top source (a file's): its address.
    fn file_src(&mut self, pc: usize) -> Value {
        let s = self.vtop(SOURCES, pc, R_NO_FILE);
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let in_file = self.f.ins().icmp_imm(IntCC::NotEqual, file, i64::from(NONE));
        self.check(in_file, pc, R_NOT_FILE);
        s
    }

    /// `SKIP` (`Machine::skip`): the file on top read on from the next `#` of its directive's
    /// chain (the chains searched by halves, `Step::Halve`), else from its end.
    fn skip(&mut self, pc: usize) {
        let s = self.file_src(pc);
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let hash = ldu(self.f, types::I32, s, offset_of!(Src, hash) as i32);
        let end = ldu(self.f, types::I32, s, offset_of!(Src, end) as i32);
        let files = ld(self.f, types::I64, self.ctx, at!(files));
        let fp = elem(self.f, files, file, FILE);
        let chains = ld(self.f, types::I64, fp, offset_of!(FileC, chains) as i32);
        let n = ld(self.f, types::I64, fp, offset_of!(FileC, nchains) as i32);
        let (found, next) = self.pair_lookup(chains, n, hash);
        let later = self.f.ins().icmp(IntCC::UnsignedGreaterThan, next, hash);
        let ok = self.f.ins().band(found, later);
        let pos = self.f.ins().select(ok, next, end);
        st(self.f, types::I32, pos, s, offset_of!(Src, pos) as i32);
        self.next(pc);
    }

    /// The `(key, value)` pairs (`PAIR`'s layout, `n` of them at `p`, rising by key) searched for
    /// `key` (`binary_search_by_key`), by halves (`Step::Halve`): whether it is there, and its
    /// value (0 where not).
    fn pair_lookup(&mut self, p: Value, n: Value, key: Value) -> (Value, Value) {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Halve));
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into(), n.into()]);
        self.f.switch_to_block(head);
        let (lo, hi) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
        let step = self.f.create_block();
        let found = self.f.create_block();
        self.f.ins().brif(more, step, &[], found, &[]);
        self.f.switch_to_block(step);
        let span = self.f.ins().isub(hi, lo);
        let half = self.f.ins().ushr_imm(span, 1);
        let mid = self.f.ins().iadd(lo, half);
        let c = elem(self.f, p, mid, PAIR.0 as i64);
        let k = ldu(self.f, types::I32, c, PAIR.1 as i32);
        let below = self.f.ins().icmp(IntCC::UnsignedLessThan, k, key);
        let m1 = self.f.ins().iadd_imm(mid, 1);
        let lo1 = self.f.ins().select(below, m1, lo);
        let hi1 = self.f.ins().select(below, hi, mid);
        self.f.ins().jump(head, &[lo1.into(), hi1.into()]);
        self.f.switch_to_block(found);
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        let within = self.f.ins().icmp(IntCC::UnsignedLessThan, lo, n);
        let look = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(within, look, &[], join, &[no.into(), z.into()]);
        self.f.switch_to_block(look);
        let c = elem(self.f, p, lo, PAIR.0 as i64);
        let k = ldu(self.f, types::I32, c, PAIR.1 as i32);
        let v = ldu(self.f, types::I32, c, PAIR.2 as i32);
        let is = self.f.ins().icmp(IntCC::Equal, k, key);
        self.f.ins().jump(join, &[is.into(), v.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    /// The file `file`'s `FileC`.
    fn file_c(&mut self, file: Value) -> Value {
        let files = ld(self.f, types::I64, self.ctx, at!(files));
        elem(self.f, files, file, FILE)
    }

    /// `Machine::header_name` of the tokens `v[lo..hi]` (`kept` or the arena): whether they make
    /// a header name, its bytes (`"…"`'s inside, or `<…>`'s spelled in scratch, a token a step,
    /// `Step::Item`), and whether it is `<…>`.
    fn header_name(&mut self, pc: usize, v: V, lo: Value, hi: Value) -> (Value, Value, Value, Value) {
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I8);
        let z = self.f.ins().iconst(types::I64, 0);
        let f8 = self.f.ins().iconst(types::I8, 0);
        let some = self.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
        let look = self.f.create_block();
        self.f.ins().brif(some, look, &[], join, &[f8.into(), z.into(), z.into(), f8.into()]);
        self.f.switch_to_block(look);
        let first = self.vat(v, lo);
        let kind = ldu(self.f, types::I8, first, T_KIND);
        let (p, n) = self.spelling(first, 0);
        let lit = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(LITERAL));
        let two = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, 2);
        let go = self.f.ins().band(lit, two);
        let quoted = self.f.create_block();
        let not_quoted = self.f.create_block();
        self.f.ins().brif(go, quoted, &[], not_quoted, &[]);
        self.f.switch_to_block(quoted);
        let c0 = ldu(self.f, types::I8, p, 0);
        let dq = self.f.ins().icmp_imm(IntCC::Equal, c0, i64::from(b'"'));
        let inner = self.f.create_block();
        self.f.ins().brif(dq, inner, &[], not_quoted, &[]);
        self.f.switch_to_block(inner);
        let p1 = self.f.ins().iadd_imm(p, 1);
        let n2 = self.f.ins().iadd_imm(n, -2);
        let yes = self.f.ins().iconst(types::I8, 1);
        let f8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().jump(join, &[yes.into(), p1.into(), n2.into(), f8.into()]);
        self.f.switch_to_block(not_quoted);
        let lt = self.spelled_at(p, n, b"<");
        let angle = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        let f8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(lt, angle, &[], join, &[f8.into(), z.into(), z.into(), f8.into()]);
        self.f.switch_to_block(angle);
        let start = ld(self.f, types::I64, self.ctx, at!(nscratch));
        let lo1 = self.f.ins().iadd_imm(lo, 1);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        self.f.ins().jump(head, &[lo1.into()]);
        self.f.switch_to_block(head);
        let k = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, k, hi);
        let step = self.f.create_block();
        let unclosed = self.f.create_block();
        self.f.ins().brif(more, step, &[], unclosed, &[]);
        self.f.switch_to_block(step);
        let t = self.vat(v, k);
        let tk = ldu(self.f, types::I8, t, T_KIND);
        let (tp, tn) = self.spelling(t, 0);
        let gt = self.spelled_at(tp, tn, b">");
        let punct = self.f.ins().icmp_imm(IntCC::Equal, tk, i64::from(PUNCT));
        let close = self.f.ins().band(gt, punct);
        let closed = self.f.create_block();
        let part = self.f.create_block();
        self.f.ins().brif(close, closed, &[], part, &[]);
        self.f.switch_to_block(closed);
        let base = ld(self.f, types::I64, self.ctx, at!(scratch));
        let sp = self.f.ins().iadd(base, start);
        let end = ld(self.f, types::I64, self.ctx, at!(nscratch));
        let sn = self.f.ins().isub(end, start);
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[yes.into(), sp.into(), sn.into(), yes.into()]);
        self.f.switch_to_block(part);
        let later = self.f.ins().icmp(IntCC::UnsignedGreaterThan, k, lo1);
        let fl = ldu(self.f, types::I8, t, T_FLAGS);
        let sp = self.f.ins().band_imm(fl, i64::from(SPACE));
        let spaced = self.f.ins().icmp_imm(IntCC::NotEqual, sp, 0);
        let gap = self.f.ins().band(later, spaced);
        let space = self.f.create_block();
        let copy = self.f.create_block();
        self.f.ins().brif(gap, space, &[], copy, &[]);
        self.f.switch_to_block(space);
        self.scratch_push(pc, b" ");
        self.f.ins().jump(copy, &[]);
        self.f.switch_to_block(copy);
        let t = self.vat(v, k);
        self.scratch_copy(pc, t, 0, false);
        let k1 = self.f.ins().iadd_imm(k, 1);
        self.f.ins().jump(head, &[k1.into()]);
        self.f.switch_to_block(unclosed);
        st(self.f, types::I64, start, self.ctx, at!(nscratch));
        let z = self.f.ins().iconst(types::I64, 0);
        let f8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().jump(join, &[f8.into(), z.into(), z.into(), f8.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1], r[2], r[3])
    }

    /// The file `file`'s named target `(p, n)`, `<…>` where `angle`, `#include_next` where
    /// `next`, an `#embed`'s where `embed`: whether it has one, and its file; an entry a step
    /// (`Step::Item`).
    #[allow(clippy::too_many_arguments)]
    fn find_named(&mut self, file: Value, p: Value, n: Value, angle: Value, next: Value, embed: Value) -> (Value, Value) {
        let no = self.f.ins().iconst(types::I8, 0);
        self.find_named_as(file, p, n, angle, next, embed, no)
    }

    /// `find_named`, or a `_Pragma`'s text's file (`pragma`).
    #[allow(clippy::too_many_arguments)]
    fn find_named_as(&mut self, file: Value, p: Value, n: Value, angle: Value, next: Value, embed: Value, pragma: Value) -> (Value, Value) {
        let fc = self.file_c(file);
        let named = ld(self.f, types::I64, fc, offset_of!(FileC, named) as i32);
        let count = ld(self.f, types::I64, fc, offset_of!(FileC, nnamed) as i32);
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into()]);
        self.f.switch_to_block(head);
        let i = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, count);
        let step = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(more, step, &[], join, &[no.into(), z.into()]);
        self.f.switch_to_block(step);
        let e = elem(self.f, named, i, NAMED);
        let len = ld(self.f, types::I64, e, offset_of!(NamedC, len) as i32);
        let mut same = self.f.ins().icmp(IntCC::Equal, len, n);
        for (o, want) in [(offset_of!(NamedC, angle), angle), (offset_of!(NamedC, next), next), (offset_of!(NamedC, embed), embed), (offset_of!(NamedC, pragma), pragma)] {
            let x = ldu(self.f, types::I8, e, o as i32);
            let w = self.f.ins().uextend(types::I64, want);
            let eq = self.f.ins().icmp(IntCC::Equal, x, w);
            same = self.f.ins().band(same, eq);
        }
        let i1 = self.f.ins().iadd_imm(i, 1);
        let bytes = self.f.create_block();
        let on = self.f.create_block();
        self.f.ins().brif(same, bytes, &[], on, &[]);
        self.f.switch_to_block(bytes);
        let q = ld(self.f, types::I64, e, offset_of!(NamedC, name) as i32);
        let eq = self.same_bytes(p, q, n);
        let hit = self.f.create_block();
        self.f.ins().brif(eq, hit, &[], on, &[]);
        self.f.switch_to_block(hit);
        let to = ldu(self.f, types::I32, e, offset_of!(NamedC, to) as i32);
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[yes.into(), to.into()]);
        self.f.switch_to_block(on);
        self.f.ins().jump(head, &[i1.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    /// `Machine::want`: a file wanted and not found, recorded; its name's bytes copied.
    #[allow(clippy::too_many_arguments)]
    fn want(&mut self, pc: usize, file: Value, p: Value, n: Value, angle: Value, next: Value, embed: Value) {
        let no = self.f.ins().iconst(types::I8, 0);
        self.want_as(pc, file, p, n, angle, next, embed, no);
    }

    /// `want`, or a `_Pragma`'s text wanted as a file (`pragma`).
    #[allow(clippy::too_many_arguments)]
    fn want_as(&mut self, pc: usize, file: Value, p: Value, n: Value, angle: Value, next: Value, embed: Value, pragma: Value) {
        let ctx = self.ctx;
        let nw = self.vlen(WANTED);
        let room = ld(self.f, types::I64, ctx, at!(room_wanted));
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, nw, room);
        let lo = ld(self.f, types::I64, ctx, at!(nwanted_bytes));
        let broom = ld(self.f, types::I64, ctx, at!(room_wanted_bytes));
        let end = self.f.ins().iadd(lo, n);
        let fits = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, broom);
        let ok = self.f.ins().band(ok, fits);
        self.check(ok, pc, R_WANTED);
        self.append_bytes(pc, (at!(wanted_bytes), at!(nwanted_bytes), at!(room_wanted_bytes)), R_WANTED, p, n, false);
        let w = self.vpush(WANTED, pc);
        st(self.f, types::I32, file, w, offset_of!(Want, file) as i32);
        st(self.f, types::I32, lo, w, offset_of!(Want, lo) as i32);
        st(self.f, types::I32, end, w, offset_of!(Want, hi) as i32);
        st(self.f, types::I8, angle, w, offset_of!(Want, angle) as i32);
        st(self.f, types::I8, next, w, offset_of!(Want, next) as i32);
        st(self.f, types::I8, embed, w, offset_of!(Want, embed) as i32);
        st(self.f, types::I8, pragma, w, offset_of!(Want, pragma) as i32);
    }

    /// `Src::file(f, end)` pushed (refusing past the sources' bound).
    fn push_file_src(&mut self, pc: usize, file: Value, end: Value) {
        let s = self.vpush(SOURCES, pc);
        let f = &mut *self.f;
        let none = f.ins().iconst(types::I32, i64::from(NONE));
        let z = f.ins().iconst(types::I32, 0);
        let z8 = f.ins().iconst(types::I8, 0);
        st(f, types::I32, file, s, offset_of!(Src, file) as i32);
        f.ins().store(flags(), z, s, offset_of!(Src, pos) as i32);
        st(f, types::I32, end, s, offset_of!(Src, end) as i32);
        f.ins().store(flags(), none, s, offset_of!(Src, def) as i32);
        f.ins().store(flags(), z8, s, offset_of!(Src, stop) as i32);
        f.ins().store(flags(), none, s, offset_of!(Src, hash) as i32);
        f.ins().store(flags(), z, s, offset_of!(Src, delta) as i32);
        let p = offset_of!(Src, presumed);
        f.ins().store(flags(), none, s, (p + offset_of!((u32, u32), 0)) as i32);
        f.ins().store(flags(), none, s, (p + offset_of!((u32, u32), 1)) as i32);
    }

    /// The count of sources that are files (`#include`'s depth), a source a step
    /// (`Step::Item`).
    fn file_sources(&mut self) -> Value {
        let n = self.vlen(SOURCES);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into(), z.into()]);
        self.f.switch_to_block(head);
        let (i, c) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[c.into()]);
        self.f.switch_to_block(step);
        let s = self.vat(SOURCES, i);
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let is = self.f.ins().icmp_imm(IntCC::NotEqual, file, i64::from(NONE));
        let x = self.f.ins().uextend(types::I64, is);
        let c1 = self.f.ins().iadd(c, x);
        let i1 = self.f.ins().iadd_imm(i, 1);
        self.f.ins().jump(head, &[i1.into(), c1.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }

    fn inc_state(&mut self) -> Value {
        ld(self.f, types::I64, self.ctx, at!(inc))
    }

    /// `Machine::destringize` of the `n` bytes at `p`: whether they are a string literal (not a
    /// raw one), and its text unescaped (`\"` and `\\`) into scratch, its range there. The run
    /// before each backslash copied whole (`Step::Word`).
    fn destringize(&mut self, pc: usize, p: Value, n: Value) -> (Value, Value, Value) {
        let ctx = self.ctx;
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        let f8 = self.f.ins().iconst(types::I8, 0);
        let none: [BlockArg; 3] = [f8.into(), z.into(), z.into()];
        let (found, q) = self.find_first(p, n, Self::quote_class);
        let look = self.f.create_block();
        self.f.ins().brif(found, look, &[], join, &none);
        self.f.switch_to_block(look);
        let qp = self.f.ins().iadd(p, q);
        let qc = ldu(self.f, types::I8, qp, 0);
        let dq = self.f.ins().icmp_imm(IntCC::Equal, qc, i64::from(b'"'));
        let q2 = self.f.ins().iadd_imm(q, 2);
        let long = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, q2, n);
        let ok = self.f.ins().band(dq, long);
        let prefix = self.f.create_block();
        self.f.ins().brif(ok, prefix, &[], join, &none);
        self.f.switch_to_block(prefix);
        let (raw, _) = self.find_first(p, q, |l, w| l.bytes_eq(w, b'R'));
        let go = self.f.create_block();
        self.f.ins().brif(raw, join, &none, go, &[]);
        self.f.switch_to_block(go);
        let lo = self.f.ins().iadd_imm(q, 1);
        let hi = self.f.ins().iadd_imm(n, -1);
        let len = self.f.ins().isub(hi, lo);
        let start = ld(self.f, types::I64, ctx, at!(nscratch));
        let room = ld(self.f, types::I64, ctx, at!(room_scratch));
        let end = self.f.ins().iadd(start, len);
        let fits = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(fits, pc, R_SCRATCH);
        let src = self.f.ins().iadd(p, lo);
        let base = ld(self.f, types::I64, ctx, at!(scratch));
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Word));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into(), start.into()]);
        self.f.switch_to_block(head);
        let (at, d) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, at, len);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[d.into()]);
        self.f.switch_to_block(step);
        let rem = self.f.ins().isub(len, at);
        let sp = self.f.ins().iadd(src, at);
        let w = self.chunk(sp, rem);
        let eight = self.f.ins().iconst(types::I64, 8);
        let k = self.f.ins().umin(rem, eight);
        let dst = self.f.ins().iadd(base, d);
        let m = self.bytes_eq(w, b'\\');
        let valid = self.valid_bytes(k);
        let m = self.f.ins().band(m, valid);
        let esc = self.f.create_block();
        let plain = self.f.create_block();
        self.f.ins().brif(m, esc, &[], plain, &[]);
        self.f.switch_to_block(plain);
        self.store_bytes(dst, w, k);
        let at1 = self.f.ins().iadd(at, k);
        let d1 = self.f.ins().iadd(d, k);
        self.f.ins().jump(head, &[at1.into(), d1.into()]);
        // The bytes before the backslash, then it: `\"` and `\\` the byte after, else itself.
        self.f.switch_to_block(esc);
        let tz = self.f.ins().ctz(m);
        let j = self.f.ins().ushr_imm(tz, 3);
        let some = self.f.create_block();
        let after = self.f.create_block();
        self.f.ins().brif(j, some, &[], after, &[]);
        self.f.switch_to_block(some);
        self.store_bytes(dst, w, j);
        self.f.ins().jump(after, &[]);
        self.f.switch_to_block(after);
        let bj = self.f.ins().iadd(at, j);
        let nj = self.f.ins().iadd_imm(bj, 1);
        let has_next = self.f.ins().icmp(IntCC::UnsignedLessThan, nj, len);
        let nb = self.f.create_block();
        self.f.append_block_param(nb, types::I64);
        let read = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().brif(has_next, read, &[], nb, &[z.into()]);
        self.f.switch_to_block(read);
        let np = self.f.ins().iadd(src, nj);
        let c = ldu(self.f, types::I8, np, 0);
        self.f.ins().jump(nb, &[c.into()]);
        self.f.switch_to_block(nb);
        let c = self.f.block_params(nb)[0];
        let q = self.f.ins().icmp_imm(IntCC::Equal, c, i64::from(b'"'));
        let b = self.f.ins().icmp_imm(IntCC::Equal, c, i64::from(b'\\'));
        let pair = self.f.ins().bor(q, b);
        let pair = self.f.ins().band(pair, has_next);
        let bs = self.f.ins().iconst(types::I64, i64::from(b'\\'));
        let out = self.f.ins().select(pair, c, bs);
        let at_c = self.f.ins().iadd(dst, j);
        st(self.f, types::I8, out, at_c, 0);
        let one = self.f.ins().iconst(types::I64, 1);
        let two = self.f.ins().iconst(types::I64, 2);
        let adv = self.f.ins().select(pair, two, one);
        let at1 = self.f.ins().iadd(bj, adv);
        let dj = self.f.ins().iadd(d, j);
        let d1 = self.f.ins().iadd_imm(dj, 1);
        self.f.ins().jump(head, &[at1.into(), d1.into()]);
        self.f.switch_to_block(done);
        let d = self.f.block_params(done)[0];
        st(self.f, types::I64, d, ctx, at!(nscratch));
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[yes.into(), start.into(), d.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1], r[2])
    }

    /// `Machine::pragma_do`: `#pragma once` (the file read once), `push_macro("X")` (X's
    /// definition saved, its slot made where there is none), `pop_macro("X")` (the last saved
    /// restored: the saved searched from the top, then the rest moved down, an entry a step,
    /// `Step::Item`). `word` is the pragma's word's bytes, `lit` (where `has_lit`) its string's.
    #[allow(clippy::too_many_arguments)]
    fn pragma_do(&mut self, pc: usize, file: Value, (wp, wn): (Value, Value), has_lit: Value, (lp, ln): (Value, Value), next: Block) {
        let ctx = self.ctx;
        let once = self.spelled_at(wp, wn, b"once");
        let set_once = self.f.create_block();
        let other = self.f.create_block();
        self.f.ins().brif(once, set_once, &[], other, &[]);
        self.f.switch_to_block(set_once);
        let o = ld(self.f, types::I64, ctx, at!(once));
        let op = self.f.ins().iadd(o, file);
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, op, 0);
        self.f.ins().jump(next, &[]);
        self.f.switch_to_block(other);
        let push = self.spelled_at(wp, wn, b"push_macro");
        let pop = self.spelled_at(wp, wn, b"pop_macro");
        let either = self.f.ins().bor(push, pop);
        let go = self.f.ins().band(either, has_lit);
        let named = self.f.create_block();
        self.f.ins().brif(go, named, &[], next, &[]);
        self.f.switch_to_block(named);
        let (ok, n0, n1) = self.destringize(pc, lp, ln);
        let find = self.f.create_block();
        self.f.ins().brif(ok, find, &[], next, &[]);
        self.f.switch_to_block(find);
        let base = ld(self.f, types::I64, ctx, at!(scratch));
        let np = self.f.ins().iadd(base, n0);
        let nn = self.f.ins().isub(n1, n0);
        let (slot, found) = self.probe(pc, np, nn);
        let have = self.f.create_block();
        let absent = self.f.create_block();
        self.f.ins().brif(found, have, &[], absent, &[]);
        // Not in the table: push_macro makes its slot (undefined); pop_macro does nothing.
        self.f.switch_to_block(absent);
        let make = self.f.create_block();
        self.f.ins().brif(push, make, &[], next, &[]);
        self.f.switch_to_block(make);
        let key = self.key(np, nn);
        st(self.f, types::I64, key, slot, offset_of!(Slot, key) as i32);
        let k = self.f.ins().iconst(types::I8, i64::from(IDENT));
        let z8 = self.f.ins().iconst(types::I8, 0);
        let z = self.f.ins().iconst(types::I32, 0);
        let nm = offset_of!(Slot, name) as i32;
        self.put_tok(slot, nm, k, z8, z, n0, n1, z);
        let none = self.f.ins().iconst(types::I32, i64::from(NONE));
        self.f.ins().store(flags(), none, slot, offset_of!(Slot, def) as i32);
        self.f.ins().jump(have, &[]);
        self.f.switch_to_block(have);
        let table = ld(self.f, types::I64, ctx, at!(table));
        let off = self.f.ins().isub(slot, table);
        let si = self.f.ins().udiv_imm(off, SLOT);
        let saving = self.f.create_block();
        let restoring = self.f.create_block();
        self.f.ins().brif(push, saving, &[], restoring, &[]);
        self.f.switch_to_block(saving);
        let def = ldu(self.f, types::I32, slot, offset_of!(Slot, def) as i32);
        let e = self.vpush(PUSHED, pc);
        self.put_pair(e, si, def);
        self.f.ins().jump(next, &[]);
        // pop_macro: the last saved of this slot.
        self.f.switch_to_block(restoring);
        let n = self.vlen(PUSHED);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        self.f.ins().jump(head, &[n.into()]);
        self.f.switch_to_block(head);
        let i = self.f.block_params(head)[0];
        let step = self.f.create_block();
        self.f.ins().brif(i, step, &[], next, &[]);
        self.f.switch_to_block(step);
        let k = self.f.ins().iadd_imm(i, -1);
        let e = self.vat(PUSHED, k);
        let (es, ed) = self.pair(e);
        let mine = self.f.ins().icmp(IntCC::Equal, es, si);
        let hit = self.f.create_block();
        self.f.ins().brif(mine, hit, &[], head, &[k.into()]);
        self.f.switch_to_block(hit);
        st(self.f, types::I32, ed, slot, offset_of!(Slot, def) as i32);
        // `Vec::remove(k)`: the rest moved down.
        let mv = self.f.create_block();
        self.f.append_block_param(mv, types::I64);
        self.heads.push((mv, Step::Item));
        let k1 = self.f.ins().iadd_imm(k, 1);
        self.f.ins().jump(mv, &[k1.into()]);
        self.f.switch_to_block(mv);
        let j = self.f.block_params(mv)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, j, n);
        let shift = self.f.create_block();
        let cut = self.f.create_block();
        self.f.ins().brif(more, shift, &[], cut, &[]);
        self.f.switch_to_block(shift);
        let from = self.vat(PUSHED, j);
        let (a, b) = self.pair(from);
        let jm = self.f.ins().iadd_imm(j, -1);
        let to = self.vat(PUSHED, jm);
        self.put_pair(to, a, b);
        let j1 = self.f.ins().iadd_imm(j, 1);
        self.f.ins().jump(mv, &[j1.into()]);
        self.f.switch_to_block(cut);
        let n1 = self.f.ins().iadd_imm(n, -1);
        self.vset_len(PUSHED, n1);
        self.f.ins().jump(next, &[]);
    }

    /// `PRAGMA` (`Machine::pragma`): the line read (`Step::Token`), its first three tokens
    /// kept: the pragma's word, and `( "…" )`'s string.
    fn pragma(&mut self, pc: usize) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        let s = self.file_src(pc);
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let pt = at!(pragma_toks);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Token));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into()]);
        self.f.switch_to_block(head);
        let n = self.f.block_params(head)[0];
        let (held, _) = self.line(pc);
        let got = self.f.create_block();
        self.f.ins().brif(held, got, &[], done, &[n.into()]);
        self.f.switch_to_block(got);
        let n1 = self.f.ins().iadd_imm(n, 1);
        let room = self.f.ins().icmp_imm(IntCC::UnsignedLessThan, n, 3);
        let keep = self.f.create_block();
        self.f.ins().brif(room, keep, &[], head, &[n1.into()]);
        self.f.switch_to_block(keep);
        let base = self.f.ins().iadd_imm(ctx, i64::from(pt));
        let slot = elem(self.f, base, n, TOK);
        copy_tok(self.f, ctx, at!(t), slot, 0);
        self.f.ins().jump(head, &[n1.into()]);
        self.f.switch_to_block(done);
        let n = self.f.block_params(done)[0];
        self.pragma_of(pc, file, n, next);
    }

    /// `Machine::pragma_of`: the pragma whose first tokens (at most three) are `pragma_toks`
    /// and whose count is `n`, carried out for `file`; then `next`.
    fn pragma_of(&mut self, pc: usize, file: Value, n: Value, next: Block) {
        let ctx = self.ctx;
        let pt = at!(pragma_toks);
        let t0 = pt;
        let t1 = pt + TOK as i32;
        let t2 = pt + 2 * TOK as i32;
        // The word: the first token, an identifier (else none).
        let has = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
        let k0 = ldu(self.f, types::I8, ctx, t0 + T_KIND);
        let id = self.f.ins().icmp_imm(IntCC::Equal, k0, i64::from(IDENT));
        let word = self.f.ins().band(has, id);
        let (p0, n0) = self.spelling(ctx, t0);
        let z = self.f.ins().iconst(types::I64, 0);
        let wn = self.f.ins().select(word, n0, z);
        // The string: `(` then a literal.
        let three = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, n, 2);
        let lj = self.f.create_block();
        self.f.append_block_param(lj, types::I8);
        let look = self.f.create_block();
        let f8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(three, look, &[], lj, &[f8.into()]);
        self.f.switch_to_block(look);
        let paren = self.tok_spelled(ctx, t1, b"(");
        let k2 = ldu(self.f, types::I8, ctx, t2 + T_KIND);
        let lit = self.f.ins().icmp_imm(IntCC::Equal, k2, i64::from(LITERAL));
        let x = self.f.ins().band(paren, lit);
        self.f.ins().jump(lj, &[x.into()]);
        self.f.switch_to_block(lj);
        let has_lit = self.f.block_params(lj)[0];
        let (lp, ln) = self.spelling(ctx, t2);
        self.pragma_do(pc, file, (p0, wn), has_lit, (lp, ln), next);
    }

    /// The count of the `n` `u32`s at `p` (rising) below `key` (`partition_point`), by halves
    /// (`Step::Halve`).
    fn count_below(&mut self, p: Value, n: Value, key: Value) -> Value {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Halve));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into(), n.into()]);
        self.f.switch_to_block(head);
        let (lo, hi) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, lo, hi);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[lo.into()]);
        self.f.switch_to_block(step);
        let span = self.f.ins().isub(hi, lo);
        let half = self.f.ins().ushr_imm(span, 1);
        let mid = self.f.ins().iadd(lo, half);
        let e = elem(self.f, p, mid, 4);
        let k = ldu(self.f, types::I32, e, 0);
        let below = self.f.ins().icmp(IntCC::UnsignedLessThan, k, key);
        let m1 = self.f.ins().iadd_imm(mid, 1);
        let lo1 = self.f.ins().select(below, m1, lo);
        let hi1 = self.f.ins().select(below, hi, mid);
        self.f.ins().jump(head, &[lo1.into(), hi1.into()]);
        self.f.switch_to_block(done);
        self.f.block_params(done)[0]
    }

    /// The top source whose `test(file)` holds, searched from the top (`Step::Item`): whether
    /// there is one, and its address.
    fn last_source(&mut self, test: &dyn Fn(&mut Self, Value) -> Value) -> (Value, Value) {
        let n = self.vlen(SOURCES);
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        self.f.ins().jump(head, &[n.into()]);
        self.f.switch_to_block(head);
        let i = self.f.block_params(head)[0];
        let step = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(i, step, &[], join, &[no.into(), z.into()]);
        self.f.switch_to_block(step);
        let k = self.f.ins().iadd_imm(i, -1);
        let s = self.vat(SOURCES, k);
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let hit = test(self, file);
        let yes = self.f.ins().iconst(types::I8, 1);
        let on = self.f.create_block();
        self.f.ins().brif(hit, join, &[yes.into(), s.into()], on, &[]);
        self.f.switch_to_block(on);
        self.f.ins().jump(head, &[k.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    /// `Machine::cur_file`: the innermost file read, 0 where none is.
    fn cur_file(&mut self) -> Value {
        let (some, s) = self.last_source(&|l, f| l.f.ins().icmp_imm(IntCC::NotEqual, f, i64::from(NONE)));
        let j = self.f.create_block();
        self.f.append_block_param(j, types::I64);
        let yes = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().brif(some, yes, &[], j, &[z.into()]);
        self.f.switch_to_block(yes);
        let f = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        self.f.ins().jump(j, &[f.into()]);
        self.f.switch_to_block(j);
        self.f.block_params(j)[0]
    }

    /// `Machine::number_tok`: `v`'s decimal digits (and sign) in scratch, a number token with
    /// `like`'s spacing and expansion, into `T`. Straight code: the digits' count by compares,
    /// then up to 20 digits.
    fn number_tok(&mut self, pc: usize, v: Value) {
        let ctx = self.ctx;
        let neg = self.f.ins().icmp_imm(IntCC::SignedLessThan, v, 0);
        let nv = self.f.ins().ineg(v);
        let u = self.f.ins().select(neg, nv, v);
        let mut len = self.f.ins().iconst(types::I64, 1);
        let mut p10: u64 = 10;
        for _ in 1..20 {
            let ge = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, u, p10 as i64);
            let x = self.f.ins().uextend(types::I64, ge);
            len = self.f.ins().iadd(len, x);
            p10 = p10.wrapping_mul(10);
        }
        let sign = self.f.ins().uextend(types::I64, neg);
        let total = self.f.ins().iadd(len, sign);
        let start = ld(self.f, types::I64, ctx, at!(nscratch));
        let room = ld(self.f, types::I64, ctx, at!(room_scratch));
        let end = self.f.ins().iadd(start, total);
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, room);
        self.check(ok, pc, R_SCRATCH);
        let base = ld(self.f, types::I64, ctx, at!(scratch));
        let p = self.f.ins().iadd(base, start);
        let minus = self.f.ins().iconst(types::I8, i64::from(b'-'));
        let put_sign = self.f.create_block();
        let digits = self.f.create_block();
        self.f.ins().brif(neg, put_sign, &[], digits, &[]);
        self.f.switch_to_block(put_sign);
        self.f.ins().store(bytes_flags(), minus, p, 0);
        self.f.ins().jump(digits, &[]);
        self.f.switch_to_block(digits);
        // The last digit at `end - 1`, the one before it before.
        let last = self.f.ins().iadd(base, end);
        let mut x = u;
        let done = self.f.create_block();
        for i in 0..20i64 {
            let more = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, len, i);
            let put = self.f.create_block();
            self.f.ins().brif(more, put, &[], done, &[]);
            self.f.switch_to_block(put);
            let q = self.f.ins().udiv_imm(x, 10);
            let q10 = self.f.ins().imul_imm(q, 10);
            let d = self.f.ins().isub(x, q10);
            let c = self.f.ins().iadd_imm(d, i64::from(b'0'));
            let c = self.f.ins().ireduce(types::I8, c);
            self.f.ins().store(bytes_flags(), c, last, -1 - i as i32);
            x = q;
        }
        self.f.ins().jump(done, &[]);
        self.f.switch_to_block(done);
        st(self.f, types::I64, end, ctx, at!(nscratch));
        self.like_tok(NUMBER, start, end);
    }

    /// `T` a token of `kind` spelled `scratch[start..end]`, with `like`'s spacing and expansion.
    fn like_tok(&mut self, kind: u8, start: Value, end: Value) {
        let ctx = self.ctx;
        let lf = ldu(self.f, types::I8, ctx, at!(like) + T_FLAGS);
        let fl = self.f.ins().band_imm(lf, i64::from(BOL | SPACE));
        let exp = ldu(self.f, types::I32, ctx, at!(like) + offset_of!(Tok, exp) as i32);
        let k = self.f.ins().iconst(types::I8, i64::from(kind));
        let z = self.f.ins().iconst(types::I32, 0);
        self.put_tok(ctx, at!(t), k, fl, z, start, end, exp);
    }

    /// `Machine::string_tok`: `(p, n)` as a string literal (`"` and `\\` escaped) into `T`.
    fn string_tok(&mut self, pc: usize, p: Value, n: Value) {
        let start = ld(self.f, types::I64, self.ctx, at!(nscratch));
        self.scratch_push(pc, b"\"");
        self.scratch_append(pc, p, n, true);
        self.scratch_push(pc, b"\"");
        let end = ld(self.f, types::I64, self.ctx, at!(nscratch));
        self.like_tok(LITERAL, start, end);
    }

    /// `Machine::line_of` of the token at `tok`: its physical line, moved by its file's `#line`.
    fn line_of(&mut self, tok: Value) -> Value {
        let ctx = self.ctx;
        let buf = ldu(self.f, types::I32, tok, offset_of!(Tok, buf) as i32);
        let start = ldu(self.f, types::I32, tok, offset_of!(Tok, start) as i32);
        let nf = ld(self.f, types::I64, ctx, at!(nfiles));
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        let some = self.f.ins().icmp_imm(IntCC::NotEqual, buf, 0);
        let within = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, buf, nf);
        let ok = self.f.ins().band(some, within);
        let go = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().brif(ok, go, &[], join, &[z.into()]);
        self.f.switch_to_block(go);
        let f = self.f.ins().iadd_imm(buf, -1);
        let fc = self.file_c(f);
        let lines = ld(self.f, types::I64, fc, offset_of!(FileC, lines) as i32);
        let nl = ld(self.f, types::I64, fc, offset_of!(FileC, nlines) as i32);
        let below = self.count_below(lines, nl, start);
        let phys = self.f.ins().iadd_imm(below, 1);
        let (has, s) = self.last_source(&|l, file| l.f.ins().icmp(IntCC::Equal, file, f));
        let dj = self.f.create_block();
        self.f.append_block_param(dj, types::I64);
        let dy = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().brif(has, dy, &[], dj, &[z.into()]);
        self.f.switch_to_block(dy);
        let d = ld(self.f, types::I32, s, offset_of!(Src, delta) as i32);
        let d = self.f.ins().sextend(types::I64, d);
        self.f.ins().jump(dj, &[d.into()]);
        self.f.switch_to_block(dj);
        let d = self.f.block_params(dj)[0];
        let line = self.f.ins().iadd(phys, d);
        let line = self.f.ins().band_imm(line, 0xffff_ffff);
        self.f.ins().jump(join, &[line.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// `Machine::presumed_path`: the innermost file's path, a `#line`'s where it set one.
    fn presumed_path(&mut self) -> (Value, Value) {
        let (some, s) = self.last_source(&|l, f| l.f.ins().icmp_imm(IntCC::NotEqual, f, i64::from(NONE)));
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I64);
        let yes = self.f.create_block();
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().brif(some, yes, &[], join, &[z.into(), z.into()]);
        self.f.switch_to_block(yes);
        let p = offset_of!(Src, presumed);
        let p0 = ldu(self.f, types::I32, s, (p + offset_of!((u32, u32), 0)) as i32);
        let p1 = ldu(self.f, types::I32, s, (p + offset_of!((u32, u32), 1)) as i32);
        let set = self.f.ins().icmp_imm(IntCC::NotEqual, p0, i64::from(NONE));
        let mine = self.f.create_block();
        let path = self.f.create_block();
        self.f.ins().brif(set, mine, &[], path, &[]);
        self.f.switch_to_block(mine);
        let base = ld(self.f, types::I64, self.ctx, at!(scratch));
        let a = self.f.ins().iadd(base, p0);
        let n = self.f.ins().isub(p1, p0);
        self.f.ins().jump(join, &[a.into(), n.into()]);
        self.f.switch_to_block(path);
        let f = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let fc = self.file_c(f);
        let a = ld(self.f, types::I64, fc, offset_of!(FileC, path) as i32);
        let n = ld(self.f, types::I64, fc, offset_of!(FileC, npath) as i32);
        self.f.ins().jump(join, &[a.into(), n.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    /// The bytes after the last `/` of `(p, n)` (all of them where there is none); the windows
    /// searched eight bytes a step (`Step::Word`).
    fn after_last_slash(&mut self, p: Value, n: Value) -> (Value, Value) {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Word));
        let done = self.f.create_block();
        self.f.append_block_param(done, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into(), z.into()]);
        self.f.switch_to_block(head);
        let (i, from) = (self.f.block_params(head)[0], self.f.block_params(head)[1]);
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[from.into()]);
        self.f.switch_to_block(step);
        let rem = self.f.ins().isub(n, i);
        let q = self.f.ins().iadd(p, i);
        let w = self.chunk(q, rem);
        let m = self.bytes_eq(w, b'/');
        let valid = self.valid_bytes(rem);
        let m = self.f.ins().band(m, valid);
        let lz = self.f.ins().clz(m);
        let top = self.f.ins().iconst(types::I64, 63);
        let bit = self.f.ins().isub(top, lz);
        let j = self.f.ins().ushr_imm(bit, 3);
        let at = self.f.ins().iadd(i, j);
        let at1 = self.f.ins().iadd_imm(at, 1);
        let from1 = self.f.ins().select(m, at1, from);
        let i8 = self.f.ins().iadd_imm(i, 8);
        self.f.ins().jump(head, &[i8.into(), from1.into()]);
        self.f.switch_to_block(done);
        let from = self.f.block_params(done)[0];
        let a = self.f.ins().iadd(p, from);
        let k = self.f.ins().isub(n, from);
        (a, k)
    }

    /// `Machine::builtin_value`: built-in `b`'s value (its call `like`, its operand
    /// `kept[lo..hi]`) into `T`.
    fn builtin_value(&mut self, pc: usize, b: Value, lo: Value, hi: Value) {
        let ctx = self.ctx;
        let done = self.f.create_block();
        let mut cases = Vec::new();
        for c in [B_LINE, B_FILE, B_FILE_NAME, B_BASE_FILE, B_COUNTER, B_INCLUDE_LEVEL, B_DATE, B_TIME, B_TIMESTAMP, B_HAS_INCLUDE, B_HAS_INCLUDE_NEXT, B_HAS_EMBED] {
            let blk = self.f.create_block();
            cases.push((c, blk));
        }
        let feature = self.f.create_block();
        let mut keyed: Vec<(u64, Block)> = cases.iter().map(|&(c, blk)| (u64::from(c), blk)).collect();
        keyed.sort_unstable_by_key(|c| c.0);
        self.switch(b, &keyed, feature);
        for (c, blk) in cases {
            self.f.switch_to_block(blk);
            match c {
                B_LINE => {
                    // The outermost call's end: the expansions' ends followed (`Step::Item`).
                    let head = self.f.create_block();
                    self.f.append_block_param(head, types::I64);
                    self.heads.push((head, Step::Item));
                    let at = self.f.ins().iadd_imm(ctx, i64::from(at!(like)));
                    self.f.ins().jump(head, &[at.into()]);
                    self.f.switch_to_block(head);
                    let r = self.f.block_params(head)[0];
                    let exp = ldu(self.f, types::I32, r, offset_of!(Tok, exp) as i32);
                    let up = self.f.create_block();
                    let found = self.f.create_block();
                    self.f.ins().brif(exp, up, &[], found, &[]);
                    self.f.switch_to_block(up);
                    let k = self.f.ins().iadd_imm(exp, -1);
                    let e = self.vat(EXPS, k);
                    let e1 = self.f.ins().iadd_imm(e, EXP.2 as i64);
                    self.f.ins().jump(head, &[e1.into()]);
                    self.f.switch_to_block(found);
                    let line = self.line_of(r);
                    self.number_tok(pc, line);
                }
                B_FILE | B_FILE_NAME | B_BASE_FILE => {
                    let (p, n) = if c == B_BASE_FILE {
                        let z = self.f.ins().iconst(types::I64, 0);
                        let nf = ld(self.f, types::I64, ctx, at!(nfiles));
                        let j = self.f.create_block();
                        self.f.append_block_param(j, types::I64);
                        self.f.append_block_param(j, types::I64);
                        let some = self.f.create_block();
                        self.f.ins().brif(nf, some, &[], j, &[z.into(), z.into()]);
                        self.f.switch_to_block(some);
                        let fc = self.file_c(z);
                        let a = ld(self.f, types::I64, fc, offset_of!(FileC, path) as i32);
                        let n = ld(self.f, types::I64, fc, offset_of!(FileC, npath) as i32);
                        self.f.ins().jump(j, &[a.into(), n.into()]);
                        self.f.switch_to_block(j);
                        (self.f.block_params(j)[0], self.f.block_params(j)[1])
                    } else {
                        self.presumed_path()
                    };
                    let (p, n) = if c == B_FILE_NAME { self.after_last_slash(p, n) } else { (p, n) };
                    self.string_tok(pc, p, n);
                }
                B_COUNTER => {
                    let n = ld(self.f, types::I64, ctx, at!(counter));
                    let n1 = self.f.ins().iadd_imm(n, 1);
                    let n1 = self.f.ins().band_imm(n1, 0xffff_ffff);
                    st(self.f, types::I64, n1, ctx, at!(counter));
                    let v = self.f.ins().iadd_imm(n1, -1);
                    self.number_tok(pc, v);
                }
                B_INCLUDE_LEVEL => {
                    let n = self.file_sources();
                    let v = self.f.ins().iadd_imm(n, -1);
                    self.number_tok(pc, v);
                }
                B_DATE | B_TIME | B_TIMESTAMP => {
                    let (fp, fn_) = match c {
                        B_DATE => (at!(date), at!(ndate)),
                        B_TIME => (at!(time), at!(ntime)),
                        _ => (at!(timestamp), at!(ntimestamp)),
                    };
                    let p = ld(self.f, types::I64, ctx, fp);
                    let n = ld(self.f, types::I64, ctx, fn_);
                    self.string_tok(pc, p, n);
                }
                _ => {
                    // `__has_include`, `__has_include_next`, `__has_embed`: 0 not found, 1 found,
                    // 2 found and empty (`#embed`).
                    let file = self.cur_file();
                    let s0 = ld(self.f, types::I64, ctx, at!(nscratch));
                    let (some, p, n, angle) = self.header_name(pc, KEPT, lo, hi);
                    let v = self.f.create_block();
                    self.f.append_block_param(v, types::I64);
                    let look = self.f.create_block();
                    let z = self.f.ins().iconst(types::I64, 0);
                    self.f.ins().brif(some, look, &[], v, &[z.into()]);
                    self.f.switch_to_block(look);
                    let nx = if c == B_HAS_INCLUDE_NEXT { self.f.ins().icmp_imm(IntCC::NotEqual, file, 0) } else { self.f.ins().iconst(types::I8, 0) };
                    let em = self.f.ins().iconst(types::I8, i64::from(c == B_HAS_EMBED));
                    let (found, to) = self.find_named(file, p, n, angle, nx, em);
                    let hit = self.f.create_block();
                    let miss = self.f.create_block();
                    self.f.ins().brif(found, hit, &[], miss, &[]);
                    self.f.switch_to_block(miss);
                    self.want(pc, file, p, n, angle, nx, em);
                    let z = self.f.ins().iconst(types::I64, 0);
                    self.f.ins().jump(v, &[z.into()]);
                    self.f.switch_to_block(hit);
                    let none = self.f.ins().icmp_imm(IntCC::Equal, to, i64::from(NONE));
                    let there = self.f.create_block();
                    let z = self.f.ins().iconst(types::I64, 0);
                    self.f.ins().brif(none, v, &[z.into()], there, &[]);
                    self.f.switch_to_block(there);
                    let one = self.f.ins().iconst(types::I64, 1);
                    if c == B_HAS_EMBED {
                        let fc = self.file_c(to);
                        let nb = ld(self.f, types::I64, fc, offset_of!(FileC, nbytes) as i32);
                        let empty = self.f.ins().icmp_imm(IntCC::Equal, nb, 0);
                        let two = self.f.ins().iconst(types::I64, 2);
                        let x = self.f.ins().select(empty, two, one);
                        self.f.ins().jump(v, &[x.into()]);
                    } else {
                        self.f.ins().jump(v, &[one.into()]);
                    }
                    self.f.switch_to_block(v);
                    let x = self.f.block_params(v)[0];
                    st(self.f, types::I64, s0, ctx, at!(nscratch));
                    self.number_tok(pc, x);
                }
            }
            self.f.ins().jump(done, &[]);
        }
        // A feature: the operand's spellings run together, looked up among the answers.
        self.f.switch_to_block(feature);
        let s0 = ld(self.f, types::I64, ctx, at!(nscratch));
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let spelled = self.f.create_block();
        self.f.ins().jump(head, &[lo.into()]);
        self.f.switch_to_block(head);
        let k = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, k, hi);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], spelled, &[]);
        self.f.switch_to_block(step);
        let t = self.vat(KEPT, k);
        self.scratch_copy(pc, t, 0, false);
        let k1 = self.f.ins().iadd_imm(k, 1);
        self.f.ins().jump(head, &[k1.into()]);
        self.f.switch_to_block(spelled);
        let e = ld(self.f, types::I64, ctx, at!(nscratch));
        let base = ld(self.f, types::I64, ctx, at!(scratch));
        let p = self.f.ins().iadd(base, s0);
        let n = self.f.ins().isub(e, s0);
        let feats = ld(self.f, types::I64, ctx, at!(features));
        let nfe = ld(self.f, types::I64, ctx, at!(nfeatures));
        let v = self.f.create_block();
        self.f.append_block_param(v, types::I64);
        let fh = self.f.create_block();
        self.f.append_block_param(fh, types::I64);
        self.heads.push((fh, Step::Item));
        let z = self.f.ins().iconst(types::I64, 0);
        let unanswered = self.f.create_block();
        self.f.ins().jump(fh, &[z.into()]);
        self.f.switch_to_block(fh);
        let i = self.f.block_params(fh)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, nfe);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], unanswered, &[]);
        self.f.switch_to_block(step);
        let fe = elem(self.f, feats, i, size_of::<FeatureC>() as i64);
        let fb = ldu(self.f, types::I32, fe, offset_of!(FeatureC, b) as i32);
        let fl = ld(self.f, types::I64, fe, offset_of!(FeatureC, len) as i32);
        let sb = self.f.ins().icmp(IntCC::Equal, fb, b);
        let sl = self.f.ins().icmp(IntCC::Equal, fl, n);
        let same = self.f.ins().band(sb, sl);
        let i1 = self.f.ins().iadd_imm(i, 1);
        let bytes = self.f.create_block();
        let on = self.f.create_block();
        self.f.ins().brif(same, bytes, &[], on, &[]);
        self.f.switch_to_block(bytes);
        let fp = ld(self.f, types::I64, fe, offset_of!(FeatureC, name) as i32);
        let eq = self.same_bytes(p, fp, n);
        let hit = self.f.create_block();
        self.f.ins().brif(eq, hit, &[], on, &[]);
        self.f.switch_to_block(hit);
        let fv = ld(self.f, types::I64, fe, offset_of!(FeatureC, value) as i32);
        self.f.ins().jump(v, &[fv.into()]);
        self.f.switch_to_block(on);
        self.f.ins().jump(fh, &[i1.into()]);
        // Not answered: recorded; `__is_identifier`'s 1, any other's 0.
        self.f.switch_to_block(unanswered);
        let na = self.vlen(ASKED);
        let room = ld(self.f, types::I64, ctx, at!(room_asked));
        let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, na, room);
        let lo_b = ld(self.f, types::I64, ctx, at!(nasked_bytes));
        let broom = ld(self.f, types::I64, ctx, at!(room_asked_bytes));
        let end = self.f.ins().iadd(lo_b, n);
        let fits = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, broom);
        let ok = self.f.ins().band(ok, fits);
        self.check(ok, pc, R_ASKED);
        self.append_bytes(pc, (at!(asked_bytes), at!(nasked_bytes), at!(room_asked_bytes)), R_ASKED, p, n, false);
        let a = self.vpush(ASKED, pc);
        st(self.f, types::I32, b, a, offset_of!(Asked, b) as i32);
        st(self.f, types::I32, lo_b, a, offset_of!(Asked, lo) as i32);
        st(self.f, types::I32, end, a, offset_of!(Asked, hi) as i32);
        let is_id = BUILTINS.iter().position(|x| x.0 == "__is_identifier").unwrap() as i64;
        let ii = self.f.ins().icmp_imm(IntCC::Equal, b, is_id);
        let x = self.f.ins().uextend(types::I64, ii);
        self.f.ins().jump(v, &[x.into()]);
        self.f.switch_to_block(v);
        let x = self.f.block_params(v)[0];
        st(self.f, types::I64, s0, ctx, at!(nscratch));
        self.number_tok(pc, x);
        self.f.ins().jump(done, &[]);
        self.f.switch_to_block(done);
    }

    /// A level opened (`(u16::MAX, kept's length)`).
    fn push_level(&mut self, pc: usize) {
        let nk = self.vlen(KEPT);
        let lp = self.vpush(LEVELS, pc);
        let p = self.f.ins().iconst(types::I16, i64::from(u16::MAX));
        self.f.ins().store(flags(), p, lp, LEVEL.1 as i32);
        st(self.f, types::I32, nk, lp, LEVEL.2 as i32);
    }

    /// `Machine::push_kept`: `kept[lo..hi]` to the arena, a token a step (`Step::Item`).
    fn push_kept(&mut self, pc: usize, lo: Value, hi: Value) {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Item));
        let done = self.f.create_block();
        self.f.ins().jump(head, &[lo.into()]);
        self.f.switch_to_block(head);
        let k = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, k, hi);
        let step = self.f.create_block();
        self.f.ins().brif(more, step, &[], done, &[]);
        self.f.switch_to_block(step);
        let t = self.vat(KEPT, k);
        self.push_arena_tok(pc, t, 0);
        let k1 = self.f.ins().iadd_imm(k, 1);
        self.f.ins().jump(head, &[k1.into()]);
        self.f.switch_to_block(done);
    }

    fn bi_state(&mut self) -> Value {
        ld(self.f, types::I64, self.ctx, at!(bi))
    }

    /// `BUILTIN` (`Machine::builtin`), its modes.
    fn builtin(&mut self, pc: usize, mode: u8) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        let skip = self.blocks.get(pc + 2).copied().unwrap_or(next);
        let header = |l: &mut Self, b: Value| {
            let x = l.f.ins().icmp_imm(IntCC::Equal, b, i64::from(B_HAS_INCLUDE));
            let y = l.f.ins().icmp_imm(IntCC::Equal, b, i64::from(B_HAS_INCLUDE_NEXT));
            let z = l.f.ins().icmp_imm(IntCC::Equal, b, i64::from(B_HAS_EMBED));
            let a = l.f.ins().bor(x, y);
            l.f.ins().bor(a, z)
        };
        match mode {
            0 | 1 => {
                let d = ld(self.f, types::I64, ctx, at!(d));
                let defs = ld(self.f, types::I64, ctx, at!(defs));
                let def = elem(self.f, defs, d, DEF);
                let bb = ldu(self.f, types::I8, def, offset_of!(Def, builtin) as i32);
                let b = self.f.ins().iadd_imm(bb, -1);
                let b = self.f.ins().band_imm(b, 0xff);
                if mode == 0 {
                    copy_tok(self.f, ctx, at!(t), ctx, at!(like));
                    let z = self.f.ins().iconst(types::I64, 0);
                    self.builtin_value(pc, b, z, z);
                    self.f.ins().jump(next, &[]);
                } else {
                    let h = header(self, b);
                    let in_if = ldu(self.f, types::I8, ctx, at!(in_if));
                    let out = self.f.ins().icmp_imm(IntCC::Equal, in_if, 0);
                    let no = self.f.ins().band(h, out);
                    let go = self.f.create_block();
                    self.f.ins().brif(no, skip, &[], go, &[]);
                    self.f.switch_to_block(go);
                    let bi = self.bi_state();
                    st(self.f, types::I32, b, bi, offset_of!(Bi, b) as i32);
                    copy_tok(self.f, ctx, at!(t), bi, offset_of!(Bi, name) as i32);
                    let z = self.f.ins().iconst(types::I32, 0);
                    self.f.ins().store(flags(), z, bi, offset_of!(Bi, depth) as i32);
                    self.push_level(pc);
                    let c = ld(self.f, types::I64, ctx, at!(collecting));
                    let c = self.f.ins().iadd_imm(c, 1);
                    st(self.f, types::I64, c, ctx, at!(collecting));
                    self.f.ins().jump(next, &[]);
                }
            }
            2 => {
                // The operand's tokens to its `)`, a parenthesis' depth kept.
                let k = ldu(self.f, types::I8, ctx, at!(t) + T_KIND);
                let punct = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(PUNCT));
                let close = self.spelled(b")");
                let close = self.f.ins().band(close, punct);
                let open = self.spelled(b"(");
                let open = self.f.ins().band(open, punct);
                let bi = self.bi_state();
                let depth = ldu(self.f, types::I32, bi, offset_of!(Bi, depth) as i32);
                let top = self.f.ins().icmp_imm(IntCC::Equal, depth, 0);
                let end = self.f.ins().band(close, top);
                let keep = self.f.create_block();
                self.f.ins().brif(end, next, &[], keep, &[]);
                self.f.switch_to_block(keep);
                let up = self.f.ins().iadd_imm(depth, 1);
                let down = self.f.ins().iadd_imm(depth, -1);
                let d = self.f.ins().select(open, up, depth);
                let d = self.f.ins().select(close, down, d);
                st(self.f, types::I32, d, bi, offset_of!(Bi, depth) as i32);
                let p = self.vpush(KEPT, pc);
                copy_tok(self.f, ctx, at!(t), p, 0);
                self.f.ins().jump(skip, &[]);
            }
            3 => {
                let c = ld(self.f, types::I64, ctx, at!(collecting));
                let c = self.f.ins().iadd_imm(c, -1);
                st(self.f, types::I64, c, ctx, at!(collecting));
                let lp = self.vtop(LEVELS, pc, R_NO_LEVEL);
                let at = ldu(self.f, types::I32, lp, LEVEL.2 as i32);
                let nl = self.vlen(LEVELS);
                let nl1 = self.f.ins().iadd_imm(nl, -1);
                self.vset_len(LEVELS, nl1);
                let hi = self.vlen(KEPT);
                let bi = self.bi_state();
                let b = ldu(self.f, types::I32, bi, offset_of!(Bi, b) as i32);
                let h = header(self, b);
                // Whether the operand starts a header name already: `"…"` or `<`.
                let sj = self.f.create_block();
                self.f.append_block_param(sj, types::I8);
                let some = self.f.ins().icmp(IntCC::UnsignedLessThan, at, hi);
                let look = self.f.create_block();
                let f8 = self.f.ins().iconst(types::I8, 0);
                self.f.ins().brif(some, look, &[], sj, &[f8.into()]);
                self.f.switch_to_block(look);
                let t = self.vat(KEPT, at);
                let k = ldu(self.f, types::I8, t, T_KIND);
                let (p, n) = self.spelling(t, 0);
                let lit = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(LITERAL));
                let ne = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
                let lj = self.f.create_block();
                self.f.append_block_param(lj, types::I8);
                let lq = self.f.create_block();
                let litne = self.f.ins().band(lit, ne);
                let f8 = self.f.ins().iconst(types::I8, 0);
                self.f.ins().brif(litne, lq, &[], lj, &[f8.into()]);
                self.f.switch_to_block(lq);
                let c0 = ldu(self.f, types::I8, p, 0);
                let dq = self.f.ins().icmp_imm(IntCC::Equal, c0, i64::from(b'"'));
                self.f.ins().jump(lj, &[dq.into()]);
                self.f.switch_to_block(lj);
                let quoted = self.f.block_params(lj)[0];
                let punct = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(PUNCT));
                let lt = self.spelled_at(p, n, b"<");
                let lt = self.f.ins().band(punct, lt);
                let x = self.f.ins().bor(quoted, lt);
                self.f.ins().jump(sj, &[x.into()]);
                self.f.switch_to_block(sj);
                let starts = self.f.block_params(sj)[0];
                let pragma = self.f.ins().icmp_imm(IntCC::Equal, b, i64::from(B_PRAGMA));
                let ns = self.f.ins().bxor_imm(starts, 1);
                let hn = self.f.ins().band(h, ns);
                let expand = self.f.ins().bor(pragma, hn);
                let exp_b = self.f.create_block();
                let value = self.f.create_block();
                self.f.ins().brif(expand, exp_b, &[], value, &[]);
                // The operand expanded first: a stop source of its tokens, and a level.
                self.f.switch_to_block(exp_b);
                let a = self.vlen(ARENA);
                self.push_kept(pc, at, hi);
                self.vset_len(KEPT, at);
                let ns = self.vlen(SOURCES);
                let nl = self.vlen(LEVELS);
                let room = ld(self.f, types::I64, ctx, at!(room_sources));
                let s_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, ns, room);
                let l_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, nl, room);
                let ok = self.f.ins().band(s_ok, l_ok);
                self.check(ok, pc, R_SOURCES);
                let e = self.vlen(ARENA);
                let none = self.f.ins().iconst(types::I32, i64::from(NONE));
                self.push_arena_src(pc, a, e, none, true);
                self.push_level(pc);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(value);
                let bi = self.bi_state();
                copy_tok(self.f, bi, offset_of!(Bi, name) as i32, ctx, at!(like));
                self.builtin_value(pc, b, at, hi);
                self.vset_len(KEPT, at);
                self.f.ins().jump(skip, &[]);
            }
            _ => {
                let (lo, hi) = self.line_end(pc);
                let bi = self.bi_state();
                let b = ldu(self.f, types::I32, bi, offset_of!(Bi, b) as i32);
                let pragma = self.f.ins().icmp_imm(IntCC::Equal, b, i64::from(B_PRAGMA));
                let value = self.f.create_block();
                let prag = self.f.create_block();
                self.f.ins().brif(pragma, prag, &[], value, &[]);
                self.f.switch_to_block(value);
                copy_tok(self.f, bi, offset_of!(Bi, name) as i32, ctx, at!(like));
                self.builtin_value(pc, b, lo, hi);
                self.vset_len(KEPT, lo);
                self.f.ins().jump(next, &[]);
                // `_Pragma("…")`: one string literal, its text the pragma's.
                self.f.switch_to_block(prag);
                let n = self.f.ins().isub(hi, lo);
                let one = self.f.ins().icmp_imm(IntCC::Equal, n, 1);
                let look = self.f.create_block();
                self.vset_len(KEPT, lo);
                self.f.ins().brif(one, look, &[], skip, &[]);
                self.f.switch_to_block(look);
                let t = self.vat(KEPT, lo);
                let k = ldu(self.f, types::I8, t, T_KIND);
                let lit = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(LITERAL));
                let go = self.f.create_block();
                self.f.ins().brif(lit, go, &[], skip, &[]);
                self.f.switch_to_block(go);
                let (p, n) = self.spelling(t, 0);
                let (ok, t0, t1) = self.destringize(pc, p, n);
                let text = self.f.create_block();
                self.f.ins().brif(ok, text, &[], skip, &[]);
                // Its text is a file of its own, lexed by the token grammar (the host's): the
                // file on top's named one, its first three tokens the pragma's; else wanted.
                self.f.switch_to_block(text);
                let base = ld(self.f, types::I64, ctx, at!(scratch));
                let tp = self.f.ins().iadd(base, t0);
                let tn = self.f.ins().isub(t1, t0);
                let file = self.cur_file();
                let (no, yes) = (self.f.ins().iconst(types::I8, 0), self.f.ins().iconst(types::I8, 1));
                let (found, to) = self.find_named_as(file, tp, tn, no, no, no, yes);
                let (hit, miss) = (self.f.create_block(), self.f.create_block());
                self.f.ins().brif(found, hit, &[], miss, &[]);
                self.f.switch_to_block(miss);
                self.want_as(pc, file, tp, tn, no, no, no, yes);
                self.f.ins().jump(skip, &[]);
                self.f.switch_to_block(hit);
                let lexed = self.f.ins().icmp_imm(IntCC::NotEqual, to, i64::from(NONE));
                self.check(lexed, pc, R_PRAGMA);
                let fc = self.file_c(to);
                let toks = ld(self.f, types::I64, fc, offset_of!(FileC, toks) as i32);
                let ntoks = ld(self.f, types::I64, fc, offset_of!(FileC, ntoks) as i32);
                // Its first tokens, at most three (straight code), where `#pragma`'s are kept.
                for k in 0..3i64 {
                    let have = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, ntoks, k);
                    let (put, on) = (self.f.create_block(), self.f.create_block());
                    self.f.ins().brif(have, put, &[], on, &[]);
                    self.f.switch_to_block(put);
                    let src = self.f.ins().iadd_imm(toks, k * TOK);
                    copy_tok(self.f, src, 0, ctx, at!(pragma_toks) + (k * TOK) as i32);
                    self.f.ins().jump(on, &[]);
                    self.f.switch_to_block(on);
                }
                self.pragma_of(pc, file, ntoks, skip);
            }
        }
    }

    /// `LINEDIR` (`Machine::linedir`): `m=0` a line marker's number kept; `m=1` the expanded
    /// line read: its number the next line's, its string the file's presumed name.
    fn linedir(&mut self, pc: usize, mode: u8) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        if mode == 0 {
            copy_tok(self.f, ctx, at!(t), ctx, at!(marker));
            let one = self.f.ins().iconst(types::I8, 1);
            self.f.ins().store(flags(), one, ctx, at!(has_marker));
            self.f.ins().jump(next, &[]);
            return;
        }
        let last_end = ldu(self.f, types::I32, ctx, at!(line_last) + offset_of!(Tok, end) as i32);
        let (lo, hi) = self.line_end(pc);
        let has_marker = ldu(self.f, types::I8, ctx, at!(has_marker));
        let z8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().store(flags(), z8, ctx, at!(has_marker));
        // The line's tokens, after the marker's where there is one: the first and the second.
        let mk = self.f.ins().icmp_imm(IntCC::NotEqual, has_marker, 0);
        let kept = ld(self.f, types::I64, ctx, at!(kept));
        let n = self.f.ins().isub(hi, lo);
        let lo_p = elem(self.f, kept, lo, TOK);
        let marker = self.f.ins().iadd_imm(ctx, i64::from(at!(marker)));
        // first: the marker, else kept[lo] (where n > 0); second: kept[lo] (marker, n > 0), else
        // kept[lo + 1] (n > 1).
        let lo1_p = self.f.ins().iadd_imm(lo_p, TOK);
        let first = self.f.ins().select(mk, marker, lo_p);
        let one = self.f.ins().iconst(types::I8, 1);
        let n1 = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, n, 0);
        let n2 = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, n, 1);
        let has_first = self.f.ins().select(mk, one, n1);
        let second = self.f.ins().select(mk, lo_p, lo1_p);
        let has_second = self.f.ins().select(mk, n1, n2);
        // Copied aside before kept is cut (the copies' tokens are read after).
        let pt = at!(pragma_toks);
        copy_tok(self.f, first, 0, ctx, pt);
        copy_tok(self.f, second, 0, ctx, pt + TOK as i32);
        self.vset_len(KEPT, lo);
        let s = self.file_src(pc);
        let fk = ldu(self.f, types::I8, ctx, pt + T_KIND);
        let num = self.f.ins().icmp_imm(IntCC::Equal, fk, i64::from(NUMBER));
        let go = self.f.ins().band(has_first, num);
        let parse = self.f.create_block();
        self.f.ins().brif(go, parse, &[], next, &[]);
        self.f.switch_to_block(parse);
        let (p, n) = self.spelling(ctx, pt);
        let (ok, v, _) = self.number(p, n);
        let set = self.f.create_block();
        self.f.ins().brif(ok, set, &[], next, &[]);
        self.f.switch_to_block(set);
        // The directive's last line: the line after it is line `v`.
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        let fc = self.file_c(file);
        let lines = ld(self.f, types::I64, fc, offset_of!(FileC, lines) as i32);
        let nl = ld(self.f, types::I64, fc, offset_of!(FileC, nlines) as i32);
        let below = self.count_below(lines, nl, last_end);
        let phys = self.f.ins().iadd_imm(below, 2);
        let delta = self.f.ins().isub(v, phys);
        st(self.f, types::I32, delta, s, offset_of!(Src, delta) as i32);
        let sk = ldu(self.f, types::I8, ctx, pt + TOK as i32 + T_KIND);
        let lit = self.f.ins().icmp_imm(IntCC::Equal, sk, i64::from(LITERAL));
        let go = self.f.ins().band(has_second, lit);
        let name = self.f.create_block();
        self.f.ins().brif(go, name, &[], next, &[]);
        self.f.switch_to_block(name);
        let (p, n) = self.spelling(ctx, pt + TOK as i32);
        let (ok, p0, p1) = self.destringize(pc, p, n);
        let put = self.f.create_block();
        self.f.ins().brif(ok, put, &[], next, &[]);
        self.f.switch_to_block(put);
        let pr = offset_of!(Src, presumed);
        st(self.f, types::I32, p0, s, (pr + offset_of!((u32, u32), 0)) as i32);
        st(self.f, types::I32, p1, s, (pr + offset_of!((u32, u32), 1)) as i32);
        self.f.ins().jump(next, &[]);
    }

    /// `Machine::push_expand`: `arena[lo..hi]` pushed to be expanded, a stop source and a level.
    fn push_expand(&mut self, pc: usize, lo: Value, hi: Value) {
        let ctx = self.ctx;
        let ns = self.vlen(SOURCES);
        let nl = self.vlen(LEVELS);
        let room = ld(self.f, types::I64, ctx, at!(room_sources));
        let s_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, ns, room);
        let l_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, nl, room);
        let ok = self.f.ins().band(s_ok, l_ok);
        self.check(ok, pc, R_SOURCES);
        let none = self.f.ins().iconst(types::I32, i64::from(NONE));
        self.push_arena_src(pc, lo, hi, none, true);
        self.push_level(pc);
    }

    fn em_ld(&mut self, off: i32) -> Value {
        ld(self.f, types::I64, self.ctx, off)
    }

    fn em_st(&mut self, v: Value, off: i32) {
        st(self.f, types::I64, v, self.ctx, off);
    }

    /// `EMBED` (`Machine::embed`), its modes; its state `Ctx::em`.
    fn embed(&mut self, pc: usize, mode: u8) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        let t8 = |l: &mut Self, b: bool| l.f.ins().iconst(types::I8, i64::from(b));
        match mode {
            0 => {
                let s = self.file_src(pc);
                let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
                let hash = ldu(self.f, types::I32, s, offset_of!(Src, hash) as i32);
                // The line's tokens to the arena: the name's, then the parameters'.
                let a = self.vlen(ARENA);
                let head = self.f.create_block();
                self.heads.push((head, Step::Token));
                let read = self.f.create_block();
                self.f.ins().jump(head, &[]);
                self.f.switch_to_block(head);
                let (held, _) = self.line(pc);
                let push = self.f.create_block();
                self.f.ins().brif(held, push, &[], read, &[]);
                self.f.switch_to_block(push);
                self.push_arena_tok(pc, ctx, at!(t));
                self.f.ins().jump(head, &[]);
                self.f.switch_to_block(read);
                let end = self.vlen(ARENA);
                // The name ends with its `"…"` or `>`: `k` past it.
                let some = self.f.ins().icmp(IntCC::UnsignedLessThan, a, end);
                let aj = self.f.create_block();
                self.f.append_block_param(aj, types::I8);
                self.f.append_block_param(aj, types::I8);
                let look = self.f.create_block();
                let f8 = t8(self, false);
                self.f.ins().brif(some, look, &[], aj, &[f8.into(), f8.into()]);
                self.f.switch_to_block(look);
                let t0 = self.vat(ARENA, a);
                let k0 = ldu(self.f, types::I8, t0, T_KIND);
                let punct = self.f.ins().icmp_imm(IntCC::Equal, k0, i64::from(PUNCT));
                let lt = self.tok_spelled(t0, 0, b"<");
                let angle = self.f.ins().band(punct, lt);
                let lit = self.f.ins().icmp_imm(IntCC::Equal, k0, i64::from(LITERAL));
                let (p, n) = self.spelling(t0, 0);
                let lj = self.f.create_block();
                self.f.append_block_param(lj, types::I8);
                let lq = self.f.create_block();
                let ne = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
                let go = self.f.ins().band(lit, ne);
                let f8 = t8(self, false);
                self.f.ins().brif(go, lq, &[], lj, &[f8.into()]);
                self.f.switch_to_block(lq);
                let c0 = ldu(self.f, types::I8, p, 0);
                let dq = self.f.ins().icmp_imm(IntCC::Equal, c0, i64::from(b'"'));
                self.f.ins().jump(lj, &[dq.into()]);
                self.f.switch_to_block(lj);
                let quoted = self.f.block_params(lj)[0];
                self.f.ins().jump(aj, &[angle.into(), quoted.into()]);
                self.f.switch_to_block(aj);
                let (angle, quoted) = (self.f.block_params(aj)[0], self.f.block_params(aj)[1]);
                // `k`: past the first `>` where it is `<…>` (else the end), else past the first.
                let kj = self.f.create_block();
                self.f.append_block_param(kj, types::I64);
                let find = self.f.create_block();
                let first = self.f.create_block();
                self.f.ins().brif(angle, find, &[], first, &[]);
                self.f.switch_to_block(first);
                let a1 = self.f.ins().iadd_imm(a, 1);
                let k = self.f.ins().umin(a1, end);
                self.f.ins().jump(kj, &[k.into()]);
                self.f.switch_to_block(find);
                let fh = self.f.create_block();
                self.f.append_block_param(fh, types::I64);
                self.heads.push((fh, Step::Item));
                self.f.ins().jump(fh, &[a.into()]);
                self.f.switch_to_block(fh);
                let j = self.f.block_params(fh)[0];
                let more = self.f.ins().icmp(IntCC::UnsignedLessThan, j, end);
                let step = self.f.create_block();
                self.f.ins().brif(more, step, &[], kj, &[end.into()]);
                self.f.switch_to_block(step);
                let t = self.vat(ARENA, j);
                let tk = ldu(self.f, types::I8, t, T_KIND);
                let pu = self.f.ins().icmp_imm(IntCC::Equal, tk, i64::from(PUNCT));
                let gt = self.tok_spelled(t, 0, b">");
                let hit = self.f.ins().band(pu, gt);
                let j1 = self.f.ins().iadd_imm(j, 1);
                let on = self.f.create_block();
                self.f.ins().brif(hit, kj, &[j1.into()], on, &[]);
                self.f.switch_to_block(on);
                self.f.ins().jump(fh, &[j1.into()]);
                self.f.switch_to_block(kj);
                let k = self.f.block_params(kj)[0];
                self.em_st(k, em!(rest_lo));
                self.em_st(end, em!(rest_hi));
                let z = self.f.ins().iconst(types::I64, 0);
                self.em_st(z, em!(has_name));
                // The file mode (a) found.
                let fc = self.file_c(file);
                let incs = ld(self.f, types::I64, fc, offset_of!(FileC, includes) as i32);
                let ni = ld(self.f, types::I64, fc, offset_of!(FileC, nincludes) as i32);
                let (found, to) = self.pair_lookup(incs, ni, hash);
                let found = self.f.ins().uextend(types::I64, found);
                self.em_st(found, em!(has_to));
                self.em_st(to, em!(to));
                let spelled = self.f.ins().band(some, quoted);
                let spelled = self.f.ins().bor(angle, spelled);
                let empty = self.f.ins().icmp(IntCC::Equal, a, k);
                let named = self.f.ins().bor(spelled, empty);
                let yes = self.f.create_block();
                let expand = self.f.create_block();
                self.f.ins().brif(named, yes, &[], expand, &[]);
                self.f.switch_to_block(yes);
                let (some, p, n, angle) = self.header_name(pc, ARENA, a, k);
                self.set_name(some, p, n, angle);
                // Malformed: nothing, its parameters not expanded (the lean pass's order).
                let lo = self.f.ins().select(some, k, end);
                self.push_expand(pc, lo, end);
                let skip = self.blocks[pc + 2];
                self.f.ins().jump(skip, &[]);
                self.f.switch_to_block(expand);
                self.push_expand(pc, a, k);
                self.f.ins().jump(next, &[]);
            }
            1 => {
                let (lo, hi) = self.line_end(pc);
                let (some, p, n, angle) = self.header_name(pc, KEPT, lo, hi);
                self.set_name(some, p, n, angle);
                self.vset_len(KEPT, lo);
                let r0 = self.em_ld(em!(rest_lo));
                let r1 = self.em_ld(em!(rest_hi));
                let z = self.f.ins().iconst(types::I64, 0);
                let r0 = self.f.ins().select(some, r0, z);
                let r1 = self.f.ins().select(some, r1, z);
                self.push_expand(pc, r0, r1);
                self.f.ins().jump(next, &[]);
            }
            2 => {
                let (at, hi) = self.line_end(pc);
                let has = self.em_ld(em!(has_name));
                let i = self.f.ins().select(has, at, hi);
                let z = self.f.ins().iconst(types::I64, 0);
                for o in [em!(arg0), em!(arg1), em!(depth), em!(prefix0), em!(prefix1), em!(suffix0), em!(suffix1), em!(if_empty0), em!(if_empty1), em!(has_limit), em!(limit), em!(file), em!(k), em!(n), em!(a)] {
                    self.em_st(z, o);
                }
                let zt = self.f.ins().iconst(types::I32, 0);
                for o in (0..TOK as i32).step_by(4) {
                    self.f.ins().store(flags(), zt, ctx, em!(p) + o);
                }
                self.em_st(at, em!(lo));
                self.em_st(i, em!(i));
                self.em_st(hi, em!(hi));
                self.f.ins().jump(next, &[]);
            }
            3 => {
                let i = self.em_ld(em!(i));
                let hi = self.em_ld(em!(hi));
                let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, hi);
                let go = self.f.create_block();
                self.f.ins().brif(more, go, &[], self.blocks[pc + 2], &[]);
                self.f.switch_to_block(go);
                let t = self.vat(KEPT, i);
                copy_tok(self.f, t, 0, ctx, em!(p));
                let i1 = self.f.ins().iadd_imm(i, 1);
                let two = self.f.ins().icmp(IntCC::UnsignedLessThan, i1, hi);
                let pj = self.f.create_block();
                self.f.append_block_param(pj, types::I8);
                let look = self.f.create_block();
                let f8 = t8(self, false);
                self.f.ins().brif(two, look, &[], pj, &[f8.into()]);
                self.f.switch_to_block(look);
                let t1 = self.vat(KEPT, i1);
                let lp = self.tok_spelled(t1, 0, b"(");
                self.f.ins().jump(pj, &[lp.into()]);
                self.f.switch_to_block(pj);
                let paren = self.f.block_params(pj)[0];
                let px = self.f.ins().uextend(types::I64, paren);
                let ni = self.f.ins().iadd(i1, px);
                self.em_st(ni, em!(i));
                self.em_st(ni, em!(arg0));
                let max = self.f.ins().iconst(types::I64, i64::from(u32::MAX));
                let a1 = self.f.ins().select(paren, max, ni);
                self.em_st(a1, em!(arg1));
                let z = self.f.ins().iconst(types::I64, 0);
                self.em_st(z, em!(depth));
                self.f.ins().jump(next, &[]);
            }
            4 => {
                let no = self.blocks[pc + 2];
                let a1 = self.em_ld(em!(arg1));
                let open = self.f.ins().icmp_imm(IntCC::Equal, a1, i64::from(u32::MAX));
                let go = self.f.create_block();
                self.f.ins().brif(open, go, &[], no, &[]);
                self.f.switch_to_block(go);
                let i = self.em_ld(em!(i));
                let hi = self.em_ld(em!(hi));
                let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, hi);
                let tok = self.f.create_block();
                let unclosed = self.f.create_block();
                self.f.ins().brif(more, tok, &[], unclosed, &[]);
                // Unclosed: the argument runs to the end.
                self.f.switch_to_block(unclosed);
                self.em_st(hi, em!(arg1));
                let h1 = self.f.ins().iadd_imm(hi, 1);
                self.em_st(h1, em!(i));
                self.f.ins().jump(no, &[]);
                self.f.switch_to_block(tok);
                let t = self.vat(KEPT, i);
                let lp = self.tok_spelled(t, 0, b"(");
                let rp = self.tok_spelled(t, 0, b")");
                let depth = self.em_ld(em!(depth));
                let top = self.f.ins().icmp_imm(IntCC::Equal, depth, 0);
                let closes = self.f.ins().band(rp, top);
                let i1 = self.f.ins().iadd_imm(i, 1);
                self.em_st(i1, em!(i));
                let closed = self.f.create_block();
                let on = self.f.create_block();
                self.f.ins().brif(closes, closed, &[], on, &[]);
                self.f.switch_to_block(closed);
                self.em_st(i, em!(arg1));
                self.f.ins().jump(no, &[]);
                self.f.switch_to_block(on);
                let up = self.f.ins().iadd_imm(depth, 1);
                let down = self.f.ins().iadd_imm(depth, -1);
                let d = self.f.ins().select(lp, up, depth);
                let d = self.f.ins().select(rp, down, d);
                self.em_st(d, em!(depth));
                self.f.ins().jump(next, &[]);
            }
            5 => {
                let no = self.blocks[pc + 2];
                let k = ldu(self.f, types::I8, ctx, em!(p) + T_KIND);
                let id = self.f.ins().icmp_imm(IntCC::Equal, k, i64::from(IDENT));
                let go = self.f.create_block();
                self.f.ins().brif(id, go, &[], no, &[]);
                self.f.switch_to_block(go);
                let (p, n) = self.spelling(ctx, em!(p));
                let a0 = self.em_ld(em!(arg0));
                let a1 = self.em_ld(em!(arg1));
                let limit = self.spelled_at(p, n, b"limit");
                let lim = self.f.create_block();
                let rest = self.f.create_block();
                self.f.ins().brif(limit, lim, &[], rest, &[]);
                self.f.switch_to_block(lim);
                let e = self.ex_state();
                st(self.f, types::I32, a0, e, ex!(lo));
                st(self.f, types::I32, a0, e, ex!(at));
                st(self.f, types::I32, a1, e, ex!(hi));
                let z8 = self.f.ins().iconst(types::I8, 0);
                self.f.ins().store(flags(), z8, e, ex!(err));
                let z = self.f.ins().iconst(types::I64, 0);
                for v in [VALS, OPSV, MINS, LIVE] {
                    self.vset_len(v, z);
                }
                let lv = self.vpush(LIVE, pc);
                let one = self.f.ins().iconst(types::I8, 1);
                self.f.ins().store(flags(), one, lv, 0);
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(rest);
                for (w, o0, o1) in [(&b"prefix"[..], em!(prefix0), em!(prefix1)), (b"suffix", em!(suffix0), em!(suffix1)), (b"if_empty", em!(if_empty0), em!(if_empty1))] {
                    let is = self.spelled_at(p, n, w);
                    let set = self.f.create_block();
                    let on = self.f.create_block();
                    self.f.ins().brif(is, set, &[], on, &[]);
                    self.f.switch_to_block(set);
                    self.em_st(a0, o0);
                    self.em_st(a1, o1);
                    self.f.ins().jump(no, &[]);
                    self.f.switch_to_block(on);
                }
                self.f.ins().jump(no, &[]);
            }
            6 => {
                let e = self.ex_state();
                let err = ldu(self.f, types::I8, e, ex!(err));
                let at = ldu(self.f, types::I32, e, ex!(at));
                let hi = ldu(self.f, types::I32, e, ex!(hi));
                let ok = self.f.ins().icmp_imm(IntCC::Equal, err, 0);
                let end = self.f.ins().icmp(IntCC::Equal, at, hi);
                let ok = self.f.ins().band(ok, end);
                let okx = self.f.ins().uextend(types::I64, ok);
                self.em_st(okx, em!(has_limit));
                let n = self.vlen(VALS);
                let vj = self.f.create_block();
                self.f.append_block_param(vj, types::I64);
                let top = self.f.create_block();
                let z = self.f.ins().iconst(types::I64, 0);
                self.f.ins().brif(n, top, &[], vj, &[z.into()]);
                self.f.switch_to_block(top);
                let n1 = self.f.ins().iadd_imm(n, -1);
                let v = self.vat(VALS, n1);
                let bits = ld(self.f, types::I64, v, offset_of!(crate::pp_expr::Value, bits) as i32);
                self.f.ins().jump(vj, &[bits.into()]);
                self.f.switch_to_block(vj);
                let bits = self.f.block_params(vj)[0];
                self.em_st(bits, em!(limit));
                self.f.ins().jump(next, &[]);
            }
            7 => {
                let no = self.blocks[pc + 2];
                let a = self.vlen(ARENA);
                self.em_st(a, em!(a));
                let has = self.em_ld(em!(has_name));
                let z = self.f.ins().iconst(types::I64, 0);
                self.em_st(z, em!(has_name));
                let go = self.f.create_block();
                self.f.ins().brif(has, go, &[], no, &[]);
                self.f.switch_to_block(go);
                let file = self.cur_file();
                let has_to = self.em_ld(em!(has_to));
                self.em_st(z, em!(has_to));
                let tj = self.f.create_block();
                self.f.append_block_param(tj, types::I64);
                let known = self.f.create_block();
                let look = self.f.create_block();
                self.f.ins().brif(has_to, known, &[], look, &[]);
                self.f.switch_to_block(known);
                let to = self.em_ld(em!(to));
                self.f.ins().jump(tj, &[to.into()]);
                self.f.switch_to_block(look);
                let p = self.em_ld(em!(name));
                let n = self.em_ld(em!(nname));
                let angle = self.em_ld(em!(angle));
                let angle = self.f.ins().ireduce(types::I8, angle);
                let nx = t8(self, false);
                let embed = t8(self, true);
                let (found, to) = self.find_named(file, p, n, angle, nx, embed);
                let hit = self.f.create_block();
                let miss = self.f.create_block();
                self.f.ins().brif(found, hit, &[], miss, &[]);
                self.f.switch_to_block(hit);
                self.f.ins().jump(tj, &[to.into()]);
                self.f.switch_to_block(miss);
                let nx = t8(self, false);
                let embed = t8(self, true);
                self.want(pc, file, p, n, angle, nx, embed);
                let none = self.f.ins().iconst(types::I64, i64::from(NONE));
                self.f.ins().jump(tj, &[none.into()]);
                self.f.switch_to_block(tj);
                let to = self.f.block_params(tj)[0];
                let none = self.f.ins().icmp_imm(IntCC::Equal, to, i64::from(NONE));
                let read = self.f.create_block();
                self.f.ins().brif(none, no, &[], read, &[]);
                self.f.switch_to_block(read);
                let fc = self.file_c(to);
                let len = ld(self.f, types::I64, fc, offset_of!(FileC, nbytes) as i32);
                let hl = self.em_ld(em!(has_limit));
                let lim = self.em_ld(em!(limit));
                let cut = self.f.ins().umin(lim, len);
                let n = self.f.ins().select(hl, cut, len);
                self.em_st(to, em!(file));
                self.em_st(z, em!(k));
                self.em_st(n, em!(n));
                let empty = self.f.ins().icmp_imm(IntCC::Equal, n, 0);
                let ie0 = self.em_ld(em!(if_empty0));
                let p0 = self.em_ld(em!(prefix0));
                let lo = self.f.ins().select(empty, ie0, p0);
                let ie1 = self.em_ld(em!(if_empty1));
                let p1 = self.em_ld(em!(prefix1));
                let hi = self.f.ins().select(empty, ie1, p1);
                self.push_kept(pc, lo, hi);
                let some = self.f.create_block();
                self.f.ins().brif(empty, no, &[], some, &[]);
                // `0` … `255`, then `,`, in scratch once.
                self.f.switch_to_block(some);
                let hd = self.em_ld(em!(has_digits));
                let put = self.f.create_block();
                self.f.ins().brif(hd, next, &[], put, &[]);
                self.f.switch_to_block(put);
                let base = ld(self.f, types::I64, ctx, at!(nscratch));
                self.scratch_push(pc, &DIGITS);
                self.em_st(base, em!(digits));
                let one = self.f.ins().iconst(types::I64, 1);
                self.em_st(one, em!(has_digits));
                self.f.ins().jump(next, &[]);
            }
            8 => {
                let k = self.em_ld(em!(k));
                let n = self.em_ld(em!(n));
                let more = self.f.ins().icmp(IntCC::UnsignedLessThan, k, n);
                let go = self.f.create_block();
                self.f.ins().brif(more, go, &[], self.blocks[pc + 2], &[]);
                self.f.switch_to_block(go);
                let base = self.em_ld(em!(digits));
                let comma = self.f.create_block();
                let byte = self.f.create_block();
                self.f.ins().brif(k, comma, &[], byte, &[]);
                self.f.switch_to_block(comma);
                let c = self.f.ins().iadd_imm(base, 658);
                let c1 = self.f.ins().iadd_imm(c, 1);
                let t = self.vpush(ARENA, pc);
                let kd = self.f.ins().iconst(types::I8, i64::from(PUNCT));
                let z8 = t8(self, false);
                let z = self.f.ins().iconst(types::I32, 0);
                self.put_tok(t, 0, kd, z8, z, c, c1, z);
                self.f.ins().jump(byte, &[]);
                self.f.switch_to_block(byte);
                let file = self.em_ld(em!(file));
                let fc = self.file_c(file);
                let bytes = ld(self.f, types::I64, fc, offset_of!(FileC, bytes) as i32);
                let bp = self.f.ins().iadd(bytes, k);
                let d = ldu(self.f, types::I8, bp, 0);
                // `d`'s digits: at `d` (one), `10 + 2(d - 10)` (two), `190 + 3(d - 100)` (three).
                let two = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, d, 10);
                let three = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, d, 100);
                let o2 = self.f.ins().imul_imm(d, 2);
                let o2 = self.f.ins().iadd_imm(o2, -10);
                let o3 = self.f.ins().imul_imm(d, 3);
                let o3 = self.f.ins().iadd_imm(o3, -110);
                let off = self.f.ins().select(two, o2, d);
                let off = self.f.ins().select(three, o3, off);
                let one = self.f.ins().iconst(types::I64, 1);
                let l2 = self.f.ins().iconst(types::I64, 2);
                let l3 = self.f.ins().iconst(types::I64, 3);
                let len = self.f.ins().select(two, l2, one);
                let len = self.f.ins().select(three, l3, len);
                let start = self.f.ins().iadd(base, off);
                let end = self.f.ins().iadd(start, len);
                let t = self.vpush(ARENA, pc);
                let kd = self.f.ins().iconst(types::I8, i64::from(NUMBER));
                let z8 = t8(self, false);
                let z = self.f.ins().iconst(types::I32, 0);
                self.put_tok(t, 0, kd, z8, z, start, end, z);
                let k1 = self.f.ins().iadd_imm(k, 1);
                self.em_st(k1, em!(k));
                self.f.ins().jump(next, &[]);
            }
            9 => {
                let lo = self.em_ld(em!(suffix0));
                let hi = self.em_ld(em!(suffix1));
                self.push_kept(pc, lo, hi);
                self.f.ins().jump(next, &[]);
            }
            _ => {
                let lo = self.em_ld(em!(lo));
                self.vset_len(KEPT, lo);
                let a = self.em_ld(em!(a));
                let end = self.vlen(ARENA);
                let some = self.f.ins().icmp(IntCC::UnsignedGreaterThan, end, a);
                let go = self.f.create_block();
                self.f.ins().brif(some, go, &[], next, &[]);
                self.f.switch_to_block(go);
                let t = self.vat(ARENA, a);
                let fl = ldu(self.f, types::I8, t, T_FLAGS);
                let fl = self.f.ins().bor_imm(fl, i64::from(BOL));
                st(self.f, types::I8, fl, t, T_FLAGS);
                let ns = self.vlen(SOURCES);
                let room = ld(self.f, types::I64, ctx, at!(room_sources));
                let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, ns, room);
                self.check(ok, pc, R_SOURCES);
                let none = self.f.ins().iconst(types::I32, i64::from(NONE));
                self.push_arena_src(pc, a, end, none, false);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    /// `#embed`'s name kept (where `some`).
    fn set_name(&mut self, some: Value, p: Value, n: Value, angle: Value) {
        let s = self.f.ins().uextend(types::I64, some);
        self.em_st(s, em!(has_name));
        self.em_st(p, em!(name));
        self.em_st(n, em!(nname));
        let a = self.f.ins().uextend(types::I64, angle);
        self.em_st(a, em!(angle));
    }

    /// `INCLUDE` (`Machine::include`), its modes.
    fn include(&mut self, pc: usize, mode: u8) {
        let ctx = self.ctx;
        let next = self.blocks[pc + 1];
        let line = if mode == 1 { Some(self.line_end(pc)) } else { None };
        let s = self.file_src(pc);
        let file = ldu(self.f, types::I32, s, offset_of!(Src, file) as i32);
        match mode {
            0 => {
                // How: `#include_next`, `#import`, else `#include`; the file mode (a) resolved.
                let skip = self.blocks[pc + 2];
                let a = self.spelled(b"include_next");
                let b = self.spelled(b"import");
                let one = self.f.ins().iconst(types::I32, 1);
                let two = self.f.ins().iconst(types::I32, 2);
                let z = self.f.ins().iconst(types::I32, 0);
                let how = self.f.ins().select(b, two, z);
                let how = self.f.ins().select(a, one, how);
                let inc = self.inc_state();
                self.f.ins().store(flags(), how, inc, offset_of!(Inc, how) as i32);
                let hash = ldu(self.f, types::I32, s, offset_of!(Src, hash) as i32);
                let fc = self.file_c(file);
                let incs = ld(self.f, types::I64, fc, offset_of!(FileC, includes) as i32);
                let n = ld(self.f, types::I64, fc, offset_of!(FileC, nincludes) as i32);
                let (found, to) = self.pair_lookup(incs, n, hash);
                let yes = self.f.create_block();
                self.f.ins().brif(found, yes, &[], skip, &[]);
                self.f.switch_to_block(yes);
                let inc = self.inc_state();
                st(self.f, types::I32, to, inc, offset_of!(Inc, to) as i32);
                self.skip_line_then(pc, next);
            }
            1 => {
                // The expanded line's header name, looked up among the file's named targets.
                let skip = self.blocks[pc + 2];
                let (lo, hi) = line.unwrap();
                let s0 = ld(self.f, types::I64, ctx, at!(nscratch));
                let (some, p, n, angle) = self.header_name(pc, KEPT, lo, hi);
                self.vset_len(KEPT, lo);
                let named = self.f.create_block();
                self.f.ins().brif(some, named, &[], skip, &[]);
                self.f.switch_to_block(named);
                let inc = self.inc_state();
                let how = ldu(self.f, types::I32, inc, offset_of!(Inc, how) as i32);
                let is_next = self.f.ins().icmp_imm(IntCC::Equal, how, 1);
                let not_main = self.f.ins().icmp_imm(IntCC::NotEqual, file, 0);
                let nx = self.f.ins().band(is_next, not_main);
                let f8 = self.f.ins().iconst(types::I8, 0);
                let (found, to) = self.find_named(file, p, n, angle, nx, f8);
                let hit = self.f.create_block();
                let miss = self.f.create_block();
                self.f.ins().brif(found, hit, &[], miss, &[]);
                self.f.switch_to_block(hit);
                let inc = self.inc_state();
                st(self.f, types::I32, to, inc, offset_of!(Inc, to) as i32);
                st(self.f, types::I64, s0, ctx, at!(nscratch));
                self.f.ins().jump(next, &[]);
                self.f.switch_to_block(miss);
                let f8 = self.f.ins().iconst(types::I8, 0);
                self.want(pc, file, p, n, angle, nx, f8);
                st(self.f, types::I64, s0, ctx, at!(nscratch));
                self.f.ins().jump(skip, &[]);
            }
            _ => {
                // The file read: not read once already, within the include depth.
                let inc = self.inc_state();
                let f = ldu(self.f, types::I32, inc, offset_of!(Inc, to) as i32);
                let how = ldu(self.f, types::I32, inc, offset_of!(Inc, how) as i32);
                let some = self.f.ins().icmp_imm(IntCC::NotEqual, f, i64::from(NONE));
                let look = self.f.create_block();
                self.f.ins().brif(some, look, &[], next, &[]);
                self.f.switch_to_block(look);
                let once = ld(self.f, types::I64, ctx, at!(once));
                let op = self.f.ins().iadd(once, f);
                let done = ldu(self.f, types::I8, op, 0);
                let fresh = self.f.create_block();
                self.f.ins().brif(done, next, &[], fresh, &[]);
                self.f.switch_to_block(fresh);
                let depth = self.file_sources();
                let deep = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, depth, MAX_INCLUDE_DEPTH as i64);
                let read = self.f.create_block();
                self.f.ins().brif(deep, next, &[], read, &[]);
                self.f.switch_to_block(read);
                let import = self.f.ins().icmp_imm(IntCC::Equal, how, 2);
                let old = ldu(self.f, types::I8, op, 0);
                let one = self.f.ins().iconst(types::I64, 1);
                let x = self.f.ins().select(import, one, old);
                st(self.f, types::I8, x, op, 0);
                let fc = self.file_c(f);
                let end = ld(self.f, types::I64, fc, offset_of!(FileC, ntoks) as i32);
                self.push_file_src(pc, f, end);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    fn cond(&mut self, pc: usize, mode: u8) {
        self.file_src(pc);
        let ns = self.vlen(SOURCES);
        let i = self.f.ins().iadd_imm(ns, -1);
        // The top conditional, where it is this file's.
        let n = self.vlen(CONDS);
        let tj = self.f.create_block();
        self.f.append_block_param(tj, types::I8);
        self.f.append_block_param(tj, types::I64);
        let look = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().brif(n, look, &[], tj, &[no.into(), z.into()]);
        self.f.switch_to_block(look);
        let n1 = self.f.ins().iadd_imm(n, -1);
        let c = self.vat(CONDS, n1);
        let src = ldu(self.f, types::I32, c, COND_SRC as i32);
        let mine = self.f.ins().icmp(IntCC::Equal, src, i);
        self.f.ins().jump(tj, &[mine.into(), c.into()]);
        self.f.switch_to_block(tj);
        let (top, c) = (self.f.block_params(tj)[0], self.f.block_params(tj)[1]);
        match mode {
            0 | 1 => {
                let p = self.vpush(CONDS, pc);
                st(self.f, types::I32, i, p, COND_SRC as i32);
                let t = self.f.ins().iconst(types::I8, i64::from(mode == 1));
                let f = self.f.ins().iconst(types::I8, 0);
                self.f.ins().store(flags(), t, p, COND_TAKEN as i32);
                self.f.ins().store(flags(), f, p, COND_ELSE as i32);
                self.next(pc);
            }
            2 => self.test(pc, top),
            3 => {
                let j = self.f.create_block();
                self.f.append_block_param(j, types::I8);
                let some = self.f.create_block();
                let yes = self.f.ins().iconst(types::I8, 1);
                self.f.ins().brif(top, some, &[], j, &[yes.into()]);
                self.f.switch_to_block(some);
                let a = ldu(self.f, types::I8, c, COND_TAKEN as i32);
                let b = ldu(self.f, types::I8, c, COND_ELSE as i32);
                let x = self.f.ins().bor(a, b);
                let x = self.f.ins().icmp_imm(IntCC::NotEqual, x, 0);
                self.f.ins().jump(j, &[x.into()]);
                self.f.switch_to_block(j);
                let held = self.f.block_params(j)[0];
                self.test(pc, held);
            }
            4 | 5 => {
                let set = self.f.create_block();
                let next = self.blocks[pc + 1];
                self.f.ins().brif(top, set, &[], next, &[]);
                self.f.switch_to_block(set);
                let one = self.f.ins().iconst(types::I8, 1);
                self.f.ins().store(flags(), one, c, if mode == 4 { COND_TAKEN } else { COND_ELSE } as i32);
                self.f.ins().jump(next, &[]);
            }
            _ => {
                let pop = self.f.create_block();
                let next = self.blocks[pc + 1];
                self.f.ins().brif(top, pop, &[], next, &[]);
                self.f.switch_to_block(pop);
                let n1 = self.f.ins().iadd_imm(n, -1);
                self.vset_len(CONDS, n1);
                self.f.ins().jump(next, &[]);
            }
        }
    }

    /// `IF m=0, 2` (`Machine::if_open`): the directive's line to the arena (a token a step,
    /// `Step::Token`), pushed as a stop source to expand, a level open; `in_if` set for `#if`.
    fn if_open(&mut self, pc: usize, in_if: bool) {
        let ctx = self.ctx;
        let a = self.vlen(ARENA);
        let head = self.f.create_block();
        self.heads.push((head, Step::Token));
        let done = self.f.create_block();
        self.f.ins().jump(head, &[]);
        self.f.switch_to_block(head);
        let (held, _) = self.line(pc);
        let push = self.f.create_block();
        self.f.ins().brif(held, push, &[], done, &[]);
        self.f.switch_to_block(push);
        self.push_arena_tok(pc, ctx, at!(t));
        self.f.ins().jump(head, &[]);
        self.f.switch_to_block(done);
        let ns = self.vlen(SOURCES);
        let nl = self.vlen(LEVELS);
        let room = ld(self.f, types::I64, ctx, at!(room_sources));
        let s_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, ns, room);
        let l_ok = self.f.ins().icmp(IntCC::UnsignedLessThan, nl, room);
        let ok = self.f.ins().band(s_ok, l_ok);
        self.check(ok, pc, R_SOURCES);
        let b = self.vlen(ARENA);
        let none = self.f.ins().iconst(types::I32, i64::from(NONE));
        self.push_arena_src(pc, a, b, none, true);
        let nk = self.vlen(KEPT);
        let lp = self.vpush(LEVELS, pc);
        let p = self.f.ins().iconst(types::I16, i64::from(u16::MAX));
        self.f.ins().store(flags(), p, lp, LEVEL.1 as i32);
        st(self.f, types::I32, nk, lp, LEVEL.2 as i32);
        let v = self.f.ins().iconst(types::I8, i64::from(in_if));
        self.f.ins().store(flags(), v, ctx, at!(in_if));
        self.next(pc);
    }

    /// `Machine::line_end`: the expanded line's level closed (its source, a stop source, off
    /// the top): its tokens' range in `kept`.
    fn line_end(&mut self, pc: usize) -> (Value, Value) {
        let lp = self.vtop(LEVELS, pc, R_NO_LEVEL);
        let at = ldu(self.f, types::I32, lp, LEVEL.2 as i32);
        let nl = self.vlen(LEVELS);
        let nl1 = self.f.ins().iadd_imm(nl, -1);
        self.vset_len(LEVELS, nl1);
        let s = self.vtop(SOURCES, pc, R_LINE_SRC);
        let stop = ldu(self.f, types::I8, s, offset_of!(Src, stop) as i32);
        let ns = self.vlen(SOURCES);
        let ns1 = self.f.ins().iadd_imm(ns, -1);
        self.vset_len(SOURCES, ns1);
        self.check(stop, pc, R_LINE_SRC);
        let z = self.f.ins().iconst(types::I8, 0);
        self.f.ins().store(flags(), z, self.ctx, at!(in_if));
        let hi = self.vlen(KEPT);
        (at, hi)
    }

    fn ex_state(&mut self) -> Value {
        ld(self.f, types::I64, self.ctx, at!(ex))
    }

    /// `IF m=1` (`Machine::expr_begin`): the expression `kept[at..hi]`, its stacks empty, its
    /// operands evaluated.
    fn expr_begin(&mut self, pc: usize) {
        let (at, hi) = self.line_end(pc);
        let e = self.ex_state();
        st(self.f, types::I32, at, e, ex!(lo));
        st(self.f, types::I32, at, e, ex!(at));
        st(self.f, types::I32, hi, e, ex!(hi));
        let z8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().store(flags(), z8, e, ex!(err));
        let z = self.f.ins().iconst(types::I64, 0);
        for v in [VALS, OPSV, MINS, LIVE] {
            self.vset_len(v, z);
        }
        let p = self.vpush(LIVE, pc);
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, p, 0);
        self.next(pc);
    }

    /// `Machine::expr_op` of the token at `tok + off`: its `OPS` index (an alternative token's
    /// operator's in C++; `OPS.len()` an operator of no expression), else `NO_OP`. Straight code:
    /// the spelling compared with each operator.
    fn expr_op(&mut self, tok: Value, off: i32) -> Value {
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        let kind = ldu(self.f, types::I8, tok, off + T_KIND);
        let (p, n) = self.spelling(tok, off);
        let punct = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(PUNCT));
        let ops = self.f.create_block();
        let other = self.f.create_block();
        self.f.ins().brif(punct, ops, &[], other, &[]);
        self.f.switch_to_block(ops);
        for (i, op) in OPS.iter().enumerate() {
            let eq = self.spelled_at(p, n, op);
            let k = self.f.ins().iconst(types::I64, i as i64);
            let on = self.f.create_block();
            self.f.ins().brif(eq, join, &[k.into()], on, &[]);
            self.f.switch_to_block(on);
        }
        let k = self.f.ins().iconst(types::I64, OPS.len() as i64);
        self.f.ins().jump(join, &[k.into()]);
        self.f.switch_to_block(other);
        if self.pp.cpp {
            // C++'s alternative tokens, where the run is C++'s.
            let ident = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(IDENT));
            let cpp = self.is_cpp();
            let ident = self.f.ins().band(ident, cpp);
            let alt = self.f.create_block();
            let none = self.f.ins().iconst(types::I64, NO_OP);
            self.f.ins().brif(ident, alt, &[], join, &[none.into()]);
            self.f.switch_to_block(alt);
            for (name, op) in crate::pp_expr::ALT {
                let eq = self.spelled_at(p, n, name);
                let k = self.f.ins().iconst(types::I64, i64::from(op_index(op)));
                let on = self.f.create_block();
                self.f.ins().brif(eq, join, &[k.into()], on, &[]);
                self.f.switch_to_block(on);
            }
        }
        let none = self.f.ins().iconst(types::I64, NO_OP);
        self.f.ins().jump(join, &[none.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// `Machine::expr_peek`: the next token's operator, `NO_OP` at the end or after an error.
    fn expr_peek(&mut self) -> Value {
        let e = self.ex_state();
        let err = ldu(self.f, types::I8, e, ex!(err));
        let at = ldu(self.f, types::I32, e, ex!(at));
        let hi = ldu(self.f, types::I32, e, ex!(hi));
        let end = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, hi);
        let errb = self.f.ins().icmp_imm(IntCC::NotEqual, err, 0);
        let stop = self.f.ins().bor(end, errb);
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        let look = self.f.create_block();
        let none = self.f.ins().iconst(types::I64, NO_OP);
        self.f.ins().brif(stop, join, &[none.into()], look, &[]);
        self.f.switch_to_block(look);
        let t = self.vat(KEPT, at);
        let op = self.expr_op(t, 0);
        self.f.ins().jump(join, &[op.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// The next token taken (`ex.at` one on).
    fn expr_take(&mut self) {
        let e = self.ex_state();
        let at = ldu(self.f, types::I32, e, ex!(at));
        let a1 = self.f.ins().iadd_imm(at, 1);
        st(self.f, types::I32, a1, e, ex!(at));
    }

    fn expr_err(&mut self) {
        let e = self.ex_state();
        let one = self.f.ins().iconst(types::I8, 1);
        self.f.ins().store(flags(), one, e, ex!(err));
    }

    /// A value pushed (`Machine::expr_push`).
    fn val_push(&mut self, pc: usize, bits: Value, uns: Value) {
        let p = self.vpush(VALS, pc);
        st(self.f, types::I64, bits, p, V_BITS);
        st(self.f, types::I8, uns, p, V_UNS);
    }

    /// The top value's address (refusing where there is none).
    fn val_top(&mut self, pc: usize) -> Value {
        self.vtop(VALS, pc, R_NO_VALUE)
    }

    /// The top value popped: its bits and unsigned flag.
    fn val_pop(&mut self, pc: usize) -> (Value, Value) {
        let p = self.val_top(pc);
        let bits = ld(self.f, types::I64, p, V_BITS);
        let uns = ld(self.f, types::I8, p, V_UNS);
        let n = self.vlen(VALS);
        let n1 = self.f.ins().iadd_imm(n, -1);
        self.vset_len(VALS, n1);
        (bits, uns)
    }

    /// `Machine::live`: the top of `live`, false where it is empty.
    fn live_top(&mut self) -> Value {
        let n = self.vlen(LIVE);
        let j = self.f.create_block();
        self.f.append_block_param(j, types::I8);
        let some = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(n, some, &[], j, &[no.into()]);
        self.f.switch_to_block(some);
        let n1 = self.f.ins().iadd_imm(n, -1);
        let p = self.vat(LIVE, n1);
        let x = ld(self.f, types::I8, p, 0);
        let x = self.f.ins().icmp_imm(IntCC::NotEqual, x, 0);
        self.f.ins().jump(j, &[x.into()]);
        self.f.switch_to_block(j);
        self.f.block_params(j)[0]
    }

    /// `live`'s top popped, where it has one (`Vec::pop`).
    fn live_pop(&mut self) {
        let n = self.vlen(LIVE);
        let one = self.f.ins().iconst(types::I64, 1);
        let m = self.f.ins().umin(n, one);
        let n1 = self.f.ins().isub(n, m);
        self.vset_len(LIVE, n1);
    }

    fn live_push(&mut self, pc: usize, x: Value) {
        let p = self.vpush(LIVE, pc);
        st(self.f, types::I8, x, p, 0);
    }

    /// A byte stack's top popped (refusing for `why` where it is empty).
    fn byte_pop(&mut self, pc: usize, v: V, why: u64) -> Value {
        let p = self.vtop(v, pc, why);
        let x = ldu(self.f, types::I8, p, 0);
        let n = self.vlen(v);
        let n1 = self.f.ins().iadd_imm(n, -1);
        self.vset_len(v, n1);
        x
    }

    fn byte_push(&mut self, pc: usize, v: V, x: Value) {
        let p = self.vpush(v, pc);
        st(self.f, types::I8, x, p, 0);
    }

    /// `pp_expr::binary` of an `OPS` index: its precedence, 0 where it is no binary operator.
    fn precedence(&mut self, op: Value) -> Value {
        let mut table: u64 = 0;
        for (i, o) in OPS.iter().enumerate().take(16) {
            table |= u64::from(crate::pp_expr::binary(o).unwrap_or(0)) << (4 * i);
        }
        let f = &mut *self.f;
        let t = f.ins().iconst(types::I64, table as i64);
        let sh = f.ins().ishl_imm(op, 2);
        let x = f.ins().ushr(t, sh);
        let low = f.ins().band_imm(x, 15);
        let small = f.ins().icmp_imm(IntCC::UnsignedLessThan, op, 16);
        let mut high = f.ins().iconst(types::I64, 0);
        for (i, o) in OPS.iter().enumerate().skip(16) {
            if let Some(p) = crate::pp_expr::binary(o) {
                let is = f.ins().icmp_imm(IntCC::Equal, op, i as i64);
                let pv = f.ins().iconst(types::I64, i64::from(p));
                high = f.ins().select(is, pv, high);
            }
        }
        f.ins().select(small, low, high)
    }

    /// `pp_expr::apply` (and `&&`, `||`): `l op r`, its result's bits and unsigned flag; a
    /// division by zero evaluated sets the error, its value 0.
    #[allow(clippy::too_many_arguments)]
    fn apply(&mut self, pc: usize, op: Value, (a, lu): (Value, Value), (b, ru): (Value, Value), live: Value) -> (Value, Value) {
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I8);
        let u = self.f.ins().bor(lu, ru);
        let ub = self.f.ins().icmp_imm(IntCC::NotEqual, u, 0);
        let lub = self.f.ins().icmp_imm(IntCC::NotEqual, lu, 0);
        let bad = self.f.create_block();
        let names: [&[u8]; 18] = [b"||", b"&&", b"|", b"^", b"&", b"==", b"!=", b"<", b">", b"<=", b">=", b"<<", b">>", b"+", b"-", b"*", b"/", b"%"];
        let blocks: Vec<Block> = names.iter().map(|_| self.f.create_block()).collect();
        let mut keyed: Vec<(u64, Block)> = names.iter().zip(&blocks).map(|(n, &b)| (u64::from(op_index(n)), b)).collect();
        keyed.sort_unstable_by_key(|c| c.0);
        self.switch(op, &keyed, bad);
        self.f.switch_to_block(bad);
        self.refuse(pc, R_NO_OP);
        for (name, blk) in names.iter().zip(blocks) {
            self.f.switch_to_block(blk);
            let f = &mut *self.f;
            let truth = |f: &mut FunctionBuilder<'_>, c: Value| -> (Value, Value) {
                let x = f.ins().uextend(types::I64, c);
                (x, f.ins().iconst(types::I8, 0))
            };
            let (bits, uns) = match *name {
                b"||" | b"&&" => {
                    let x = f.ins().icmp_imm(IntCC::NotEqual, a, 0);
                    let y = f.ins().icmp_imm(IntCC::NotEqual, b, 0);
                    let c = if *name == b"&&" { f.ins().band(x, y) } else { f.ins().bor(x, y) };
                    truth(f, c)
                }
                b"|" => (f.ins().bor(a, b), u),
                b"^" => (f.ins().bxor(a, b), u),
                b"&" => (f.ins().band(a, b), u),
                b"*" => (f.ins().imul(a, b), u),
                b"+" => (f.ins().iadd(a, b), u),
                b"-" => (f.ins().isub(a, b), u),
                b"==" => {
                    let c = f.ins().icmp(IntCC::Equal, a, b);
                    truth(f, c)
                }
                b"!=" => {
                    let c = f.ins().icmp(IntCC::NotEqual, a, b);
                    truth(f, c)
                }
                b"<" | b">" | b"<=" | b">=" => {
                    let (uc, sc) = match *name {
                        b"<" => (IntCC::UnsignedLessThan, IntCC::SignedLessThan),
                        b">" => (IntCC::UnsignedGreaterThan, IntCC::SignedGreaterThan),
                        b"<=" => (IntCC::UnsignedLessThanOrEqual, IntCC::SignedLessThanOrEqual),
                        _ => (IntCC::UnsignedGreaterThanOrEqual, IntCC::SignedGreaterThanOrEqual),
                    };
                    let x = f.ins().icmp(uc, a, b);
                    let y = f.ins().icmp(sc, a, b);
                    let c = f.ins().select(ub, x, y);
                    truth(f, c)
                }
                b"<<" => {
                    let big = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, b, 64);
                    let x = f.ins().ishl(a, b);
                    let z = f.ins().iconst(types::I64, 0);
                    (f.ins().select(big, z, x), lu)
                }
                b">>" => {
                    let big = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, b, 64);
                    let sh = f.ins().band_imm(b, 63);
                    let ux = f.ins().ushr(a, sh);
                    let z = f.ins().iconst(types::I64, 0);
                    let ux = f.ins().select(big, z, ux);
                    let s63 = f.ins().iconst(types::I64, 63);
                    let sh = f.ins().select(big, s63, sh);
                    let sx = f.ins().sshr(a, sh);
                    (f.ins().select(lub, ux, sx), lu)
                }
                _ => {
                    // `/` and `%`: by zero, an error where evaluated; signed `MIN / -1` wraps.
                    let div = *name == b"/";
                    let zero = f.create_block();
                    let nz = f.create_block();
                    f.ins().brif(b, nz, &[], zero, &[]);
                    f.switch_to_block(zero);
                    let z = f.ins().iconst(types::I64, 0);
                    let f8 = f.ins().iconst(types::I8, 0);
                    let uns = f.ins().select(live, f8, u);
                    let err = f.create_block();
                    let quiet = f.create_block();
                    f.ins().brif(live, err, &[], quiet, &[]);
                    f.switch_to_block(err);
                    let e = ld(f, types::I64, self.ctx, at!(ex));
                    let one = f.ins().iconst(types::I8, 1);
                    f.ins().store(flags(), one, e, ex!(err));
                    f.ins().jump(join, &[z.into(), uns.into()]);
                    f.switch_to_block(quiet);
                    f.ins().jump(join, &[z.into(), uns.into()]);
                    f.switch_to_block(nz);
                    let unsigned = f.create_block();
                    let signed = f.create_block();
                    f.ins().brif(ub, unsigned, &[], signed, &[]);
                    f.switch_to_block(unsigned);
                    let x = if div { f.ins().udiv(a, b) } else { f.ins().urem(a, b) };
                    f.ins().jump(join, &[x.into(), u.into()]);
                    f.switch_to_block(signed);
                    let m1 = f.ins().icmp_imm(IntCC::Equal, b, -1);
                    let wrap = f.create_block();
                    let plain = f.create_block();
                    f.ins().brif(m1, wrap, &[], plain, &[]);
                    f.switch_to_block(wrap);
                    let x = if div { f.ins().ineg(a) } else { f.ins().iconst(types::I64, 0) };
                    f.ins().jump(join, &[x.into(), u.into()]);
                    f.switch_to_block(plain);
                    let x = if div { f.ins().sdiv(a, b) } else { f.ins().srem(a, b) };
                    f.ins().jump(join, &[x.into(), u.into()]);
                    continue;
                }
            };
            f.ins().jump(join, &[bits.into(), uns.into()]);
        }
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    /// The `n` bytes at `p` folded through `lane` a byte at a time (`init` the fold's values),
    /// eight lanes a step: straight code over each 8-byte window (`Step::Word`), a lane past
    /// the end leaving the fold as it was.
    fn lanes(&mut self, p: Value, n: Value, init: &[Value], lane: &mut dyn FnMut(&mut Self, Value, &[Value]) -> Vec<Value>) -> Vec<Value> {
        let tys: Vec<types::Type> = init.iter().map(|&v| self.f.func.dfg.value_type(v)).collect();
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        let done = self.f.create_block();
        for &t in &tys {
            self.f.append_block_param(head, t);
            self.f.append_block_param(done, t);
        }
        self.heads.push((head, Step::Word));
        let z = self.f.ins().iconst(types::I64, 0);
        let args: Vec<BlockArg> = std::iter::once(z).chain(init.iter().copied()).map(BlockArg::from).collect();
        self.f.ins().jump(head, &args);
        self.f.switch_to_block(head);
        let params = self.f.block_params(head).to_vec();
        let (i, st0) = (params[0], params[1..].to_vec());
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let step = self.f.create_block();
        let out: Vec<BlockArg> = st0.iter().copied().map(BlockArg::from).collect();
        self.f.ins().brif(more, step, &[], done, &out);
        self.f.switch_to_block(step);
        let rem = self.f.ins().isub(n, i);
        let q = self.f.ins().iadd(p, i);
        let w = self.chunk(q, rem);
        let mut st = st0;
        for j in 0..8i64 {
            let c = self.f.ins().ushr_imm(w, 8 * j);
            let c = self.f.ins().band_imm(c, 0xff);
            let valid = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThan, rem, j);
            let new = lane(self, c, &st);
            st = st.iter().zip(new).map(|(&old, new)| self.f.ins().select(valid, new, old)).collect();
        }
        let i8 = self.f.ins().iadd_imm(i, 8);
        let args: Vec<BlockArg> = std::iter::once(i8).chain(st).map(BlockArg::from).collect();
        self.f.ins().jump(head, &args);
        self.f.switch_to_block(done);
        self.f.block_params(done).to_vec()
    }

    /// The first of the `n` bytes at `p` whose high bit `class` sets: whether there is one, and
    /// its index; eight a step (`Step::Word`).
    fn find_first(&mut self, p: Value, n: Value, class: fn(&mut Self, Value) -> Value) -> (Value, Value) {
        let head = self.f.create_block();
        self.f.append_block_param(head, types::I64);
        self.heads.push((head, Step::Word));
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(head, &[z.into()]);
        self.f.switch_to_block(head);
        let i = self.f.block_params(head)[0];
        let more = self.f.ins().icmp(IntCC::UnsignedLessThan, i, n);
        let step = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(more, step, &[], join, &[no.into(), z.into()]);
        self.f.switch_to_block(step);
        let rem = self.f.ins().isub(n, i);
        let q = self.f.ins().iadd(p, i);
        let w = self.chunk(q, rem);
        let m = class(self, w);
        let valid = self.valid_bytes(rem);
        let m = self.f.ins().band(m, valid);
        let hit = self.f.create_block();
        let on = self.f.create_block();
        self.f.ins().brif(m, hit, &[], on, &[]);
        self.f.switch_to_block(on);
        let i8 = self.f.ins().iadd_imm(i, 8);
        self.f.ins().jump(head, &[i8.into()]);
        self.f.switch_to_block(hit);
        let tz = self.f.ins().ctz(m);
        let j = self.f.ins().ushr_imm(tz, 3);
        let k = self.f.ins().iadd(i, j);
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[yes.into(), k.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join);
        (r[0], r[1])
    }

    fn quote_class(&mut self, w: Value) -> Value {
        let a = self.bytes_eq(w, b'"');
        let b = self.bytes_eq(w, b'\'');
        self.f.ins().bor(a, b)
    }

    /// `pp_expr::number` of the `n` bytes at `p`: whether it is an integer constant, its bits and
    /// unsigned flag. A fold a byte a lane (`lanes`): its phase (0 the first byte, 1 after a
    /// leading `0`, 2 the digits, 3 the suffix), radix, value, overflow, digits, and the suffix
    /// lowered and packed; digit separators pass.
    fn number(&mut self, p: Value, n: Value) -> (Value, Value, Value) {
        let k = |l: &mut Self, x: i64| l.f.ins().iconst(types::I64, x);
        let init = [k(self, 0), k(self, 10), k(self, 0), k(self, 0), k(self, 0), k(self, 0), k(self, 0)];
        let st = self.lanes(p, n, &init, &mut |l, c, st| {
            let (phase, radix, v, of, ndig, sfx, slen) = (st[0], st[1], st[2], st[3], st[4], st[5], st[6]);
            let f = &mut *l.f;
            let one = f.ins().iconst(types::I64, 1);
            let lc = f.ins().bor_imm(c, 0x20);
            let up = f.ins().iadd_imm(c, -i64::from(b'A'));
            let is_up = f.ins().icmp_imm(IntCC::UnsignedLessThan, up, 26);
            let lower = f.ins().select(is_up, lc, c);
            // The first byte: a `0` (octal so far), else a decimal's first digit.
            let p0 = f.ins().icmp_imm(IntCC::Equal, phase, 0);
            let is0 = f.ins().icmp_imm(IntCC::Equal, c, i64::from(b'0'));
            let c1 = f.ins().iconst(types::I64, 1);
            let c2 = f.ins().iconst(types::I64, 2);
            let c8 = f.ins().iconst(types::I64, 8);
            let c10 = f.ins().iconst(types::I64, 10);
            let ph0 = f.ins().select(is0, c1, c2);
            let phase = f.ins().select(p0, ph0, phase);
            let r0 = f.ins().select(is0, c8, c10);
            let radix = f.ins().select(p0, r0, radix);
            let cons = f.ins().band(p0, is0);
            // After a leading `0`: `x` or `b`, else octal's digits.
            let p1 = f.ins().icmp_imm(IntCC::Equal, phase, 1);
            let ncons = f.ins().bxor_imm(cons, 1);
            let p1 = f.ins().band(p1, ncons);
            let x = f.ins().icmp_imm(IntCC::Equal, lc, i64::from(b'x'));
            let b = f.ins().icmp_imm(IntCC::Equal, lc, i64::from(b'b'));
            let c16 = f.ins().iconst(types::I64, 16);
            let rb = f.ins().select(b, c2, c8);
            let rx = f.ins().select(x, c16, rb);
            let radix = f.ins().select(p1, rx, radix);
            let phase = f.ins().select(p1, c2, phase);
            let xb = f.ins().bor(x, b);
            let took = f.ins().band(p1, xb);
            let cons = f.ins().bor(cons, took);
            // A digit of the radix.
            let p2 = f.ins().icmp_imm(IntCC::Equal, phase, 2);
            let ncons = f.ins().bxor_imm(cons, 1);
            let p2 = f.ins().band(p2, ncons);
            let dd = f.ins().iadd_imm(c, -i64::from(b'0'));
            let is_dec = f.ins().icmp_imm(IntCC::UnsignedLessThan, dd, 10);
            let hx = f.ins().iadd_imm(lc, -i64::from(b'a'));
            let is_hex = f.ins().icmp_imm(IntCC::UnsignedLessThan, hx, 6);
            let hv = f.ins().iadd_imm(hx, 10);
            let big = f.ins().iconst(types::I64, 99);
            let d = f.ins().select(is_hex, hv, big);
            let d = f.ins().select(is_dec, dd, d);
            let isd = f.ins().icmp(IntCC::UnsignedLessThan, d, radix);
            let take = f.ins().band(p2, isd);
            let hi = f.ins().umulhi(v, radix);
            let m = f.ins().imul(v, radix);
            let sum = f.ins().iadd(m, d);
            let carry = f.ins().icmp(IntCC::UnsignedLessThan, sum, m);
            let wide = f.ins().icmp_imm(IntCC::NotEqual, hi, 0);
            let over = f.ins().bor(wide, carry);
            let over = f.ins().band(take, over);
            let over = f.ins().uextend(types::I64, over);
            let of = f.ins().bor(of, over);
            let v = f.ins().select(take, sum, v);
            let t64 = f.ins().uextend(types::I64, take);
            let ndig = f.ins().iadd(ndig, t64);
            let nd = f.ins().bxor_imm(isd, 1);
            let ends = f.ins().band(p2, nd);
            let c3 = f.ins().iconst(types::I64, 3);
            let phase = f.ins().select(ends, c3, phase);
            let cons = f.ins().bor(cons, take);
            // The suffix's bytes, lowered.
            let p3 = f.ins().icmp_imm(IntCC::Equal, phase, 3);
            let ncons = f.ins().bxor_imm(cons, 1);
            let p3 = f.ins().band(p3, ncons);
            let c7 = f.ins().iconst(types::I64, 7);
            let at = f.ins().umin(slen, c7);
            let sh = f.ins().ishl_imm(at, 3);
            let byte = f.ins().ishl(lower, sh);
            let s1 = f.ins().bor(sfx, byte);
            let sfx = f.ins().select(p3, s1, sfx);
            let l1 = f.ins().iadd(slen, one);
            let slen = f.ins().select(p3, l1, slen);
            // A separator: nothing.
            let sep = f.ins().icmp_imm(IntCC::Equal, c, i64::from(b'\''));
            [phase, radix, v, of, ndig, sfx, slen].iter().zip(st).map(|(&new, &old)| f.ins().select(sep, old, new)).collect()
        });
        let (radix, v, of, ndig, sfx, slen) = (st[1], st[2], st[3], st[4], st[5], st[6]);
        let f = &mut *self.f;
        let pack = |s: &[u8]| s.iter().enumerate().fold(0i64, |a, (i, &c)| a | (i64::from(c) << (8 * i)));
        let mut signed = f.ins().iconst(types::I8, 0);
        let mut unsigned = f.ins().iconst(types::I8, 0);
        for (list, out) in [(&[&b""[..], b"l", b"ll", b"z", b"wb"][..], &mut signed), (&[&b"u"[..], b"ul", b"lu", b"ull", b"llu", b"uz", b"zu", b"uwb", b"wbu"][..], &mut unsigned)] {
            for x in list {
                let a = f.ins().icmp_imm(IntCC::Equal, sfx, pack(x));
                let b = f.ins().icmp_imm(IntCC::Equal, slen, x.len() as i64);
                let e = f.ins().band(a, b);
                *out = f.ins().bor(*out, e);
            }
        }
        let suffix_ok = f.ins().bor(signed, unsigned);
        let no_of = f.ins().icmp_imm(IntCC::Equal, of, 0);
        let r10 = f.ins().icmp_imm(IntCC::Equal, radix, 10);
        let r8 = f.ins().icmp_imm(IntCC::Equal, radix, 8);
        let free = f.ins().bor(r10, r8);
        let some = f.ins().icmp_imm(IntCC::NotEqual, ndig, 0);
        let digits_ok = f.ins().bor(free, some);
        let ok = f.ins().band(suffix_ok, no_of);
        let ok = f.ins().band(ok, digits_ok);
        // A decimal past `intmax_t` is unsigned.
        let past = f.ins().icmp_imm(IntCC::SignedLessThan, v, 0);
        let past = f.ins().band(past, r10);
        let uns = f.ins().bor(unsigned, past);
        (ok, v, uns)
    }

    /// `pp_expr::char_value` of the `n` bytes at `p`, whose first quote (at `q`) is `'`:
    /// whether it is a character constant, and its value. A fold over its body a byte a lane
    /// (`lanes`): the escape's state (0 none, 1 after `\`, 2 octal, 3 hex, 4 `\u`/`\U`, 5 a
    /// UTF-8 character's continuation), its value so far, its count and width or lead byte, and
    /// the characters' first, count and GCC's packing.
    fn char_value(&mut self, p: Value, n: Value, q: Value) -> (Value, Value) {
        // The prefix: the bytes before the quote.
        let (lp, ln) = (p, q);
        let wide = self.f.ins().icmp_imm(IntCC::NotEqual, q, 0);
        let b0 = self.f.ins().iadd_imm(q, 1);
        let b1 = self.f.ins().iadd_imm(n, -1);
        let has_body = self.f.ins().icmp(IntCC::UnsignedLessThanOrEqual, b0, b1);
        let nonempty = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
        let has_body = self.f.ins().band(has_body, nonempty);
        let start = self.f.ins().iadd(p, b0);
        let z = self.f.ins().iconst(types::I64, 0);
        let len = self.f.ins().isub(b1, b0);
        let len = self.f.ins().select(has_body, len, z);
        let k = |l: &mut Self, x: i64| l.f.ins().iconst(types::I64, x);
        // state, acc, aux, lead, first, count, fold, bad
        let init = [k(self, 0), k(self, 0), k(self, 0), k(self, 0), k(self, 0), k(self, 0), k(self, 0), k(self, 0)];
        fn emit(f: &mut FunctionBuilder<'_>, cond: Value, val: Value, first: Value, count: Value, fold: Value) -> (Value, Value, Value) {
            let none = f.ins().icmp_imm(IntCC::Equal, count, 0);
            let set = f.ins().band(cond, none);
            let first = f.ins().select(set, val, first);
            let c = f.ins().uextend(types::I64, cond);
            let count = f.ins().iadd(count, c);
            let sh = f.ins().ishl_imm(fold, 8);
            let lo = f.ins().band_imm(val, 0xff);
            let x = f.ins().bor(sh, lo);
            let x = f.ins().band_imm(x, 0xffff_ffff);
            let fold = f.ins().select(cond, x, fold);
            (first, count, fold)
        }
        let st = self.lanes(start, len, &init, &mut |l, c, st| {
            let f = &mut *l.f;
            let (state, acc, aux, lead, first, count, fold, bad) = (st[0], st[1], st[2], st[3], st[4], st[5], st[6], st[7]);
            let k = |f: &mut FunctionBuilder<'_>, x: i64| f.ins().iconst(types::I64, x);
            let is = |f: &mut FunctionBuilder<'_>, x: Value, y: i64| f.ins().icmp_imm(IntCC::Equal, x, y);
            let lc = f.ins().bor_imm(c, 0x20);
            let dd = f.ins().iadd_imm(c, -i64::from(b'0'));
            let is_dec = f.ins().icmp_imm(IntCC::UnsignedLessThan, dd, 10);
            let is_oct = f.ins().icmp_imm(IntCC::UnsignedLessThan, dd, 8);
            let hx = f.ins().iadd_imm(lc, -i64::from(b'a'));
            let is_hx = f.ins().icmp_imm(IntCC::UnsignedLessThan, hx, 6);
            let hv = f.ins().iadd_imm(hx, 10);
            let hexd = f.ins().select(is_dec, dd, hv);
            let is_hex = f.ins().bor(is_dec, is_hx);
            let zero = k(f, 0);
            let mut bad = bad;
            let mut st_ = state;
            let mut acc = acc;
            let mut aux = aux;
            let mut lead = lead;
            let (mut first, mut count, mut fold) = (first, count, fold);
            let mut cons = f.ins().iconst(types::I8, 0);
            // 2: octal, up to two more digits; else it ends and the byte is read anew.
            let s2 = is(f, st_, 2);
            let more = f.ins().icmp_imm(IntCC::UnsignedLessThan, aux, 2);
            let go = f.ins().band(is_oct, more);
            let take = f.ins().band(s2, go);
            let a8 = f.ins().ishl_imm(acc, 3);
            let a8 = f.ins().iadd(a8, dd);
            acc = f.ins().select(take, a8, acc);
            let aux1 = f.ins().iadd_imm(aux, 1);
            aux = f.ins().select(take, aux1, aux);
            cons = f.ins().bor(cons, take);
            let nt = f.ins().bxor_imm(go, 1);
            let end2 = f.ins().band(s2, nt);
            (first, count, fold) = emit(f, end2, acc, first, count, fold);
            st_ = f.ins().select(end2, zero, st_);
            // 3: hex digits, any number; at least one.
            let s3 = is(f, st_, 3);
            let take = f.ins().band(s3, is_hex);
            let a16 = f.ins().ishl_imm(acc, 4);
            let a16 = f.ins().iadd(a16, hexd);
            acc = f.ins().select(take, a16, acc);
            let aux1 = f.ins().iadd_imm(aux, 1);
            aux = f.ins().select(take, aux1, aux);
            cons = f.ins().bor(cons, take);
            let nh = f.ins().bxor_imm(is_hex, 1);
            let end3 = f.ins().band(s3, nh);
            let none = is(f, aux, 0);
            let empty = f.ins().band(end3, none);
            let e64 = f.ins().uextend(types::I64, empty);
            bad = f.ins().bor(bad, e64);
            (first, count, fold) = emit(f, end3, acc, first, count, fold);
            st_ = f.ins().select(end3, zero, st_);
            // 4: `\u`'s or `\U`'s hex digits (`from_str_radix`: a `+` may lead).
            let s4 = is(f, st_, 4);
            let at0 = f.ins().icmp(IntCC::Equal, aux, lead);
            let plus = is(f, c, i64::from(b'+'));
            let lead_plus = f.ins().band(at0, plus);
            let ok = f.ins().bor(is_hex, lead_plus);
            let nok = f.ins().bxor_imm(ok, 1);
            let wrong = f.ins().band(s4, nok);
            let w64 = f.ins().uextend(types::I64, wrong);
            bad = f.ins().bor(bad, w64);
            let a16 = f.ins().ishl_imm(acc, 4);
            let a16 = f.ins().iadd(a16, hexd);
            let a16 = f.ins().select(lead_plus, acc, a16);
            acc = f.ins().select(s4, a16, acc);
            let aux1 = f.ins().iadd_imm(aux, -1);
            aux = f.ins().select(s4, aux1, aux);
            let last = is(f, aux, 0);
            let end4 = f.ins().band(s4, last);
            (first, count, fold) = emit(f, end4, acc, first, count, fold);
            st_ = f.ins().select(end4, zero, st_);
            cons = f.ins().bor(cons, s4);
            // 5: a UTF-8 character's continuation bytes (`from_utf8`'s ranges).
            let s5 = is(f, st_, 5);
            let need3 = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, lead, 0xf0);
            let need2 = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, lead, 0xe0);
            let two = k(f, 2);
            let one = k(f, 1);
            let three = k(f, 3);
            let need = f.ins().select(need2, two, one);
            let need = f.ins().select(need3, three, need);
            let second = f.ins().icmp(IntCC::Equal, aux, need);
            let (mut lo, mut hi) = (k(f, 0x80), k(f, 0xbf));
            for (b, l, h) in [(0xe0, 0xa0, 0xbf), (0xed, 0x80, 0x9f), (0xf0, 0x90, 0xbf), (0xf4, 0x80, 0x8f)] {
                let x = is(f, lead, b);
                let x = f.ins().band(x, second);
                let lv = k(f, l);
                let hv = k(f, h);
                lo = f.ins().select(x, lv, lo);
                hi = f.ins().select(x, hv, hi);
            }
            let ge = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, c, lo);
            let le = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, c, hi);
            let inr = f.ins().band(ge, le);
            let nin = f.ins().bxor_imm(inr, 1);
            let wrong = f.ins().band(s5, nin);
            let w64 = f.ins().uextend(types::I64, wrong);
            bad = f.ins().bor(bad, w64);
            let a6 = f.ins().ishl_imm(acc, 6);
            let low6 = f.ins().band_imm(c, 0x3f);
            let a6 = f.ins().bor(a6, low6);
            acc = f.ins().select(s5, a6, acc);
            let aux1 = f.ins().iadd_imm(aux, -1);
            aux = f.ins().select(s5, aux1, aux);
            let last = is(f, aux, 0);
            let end5 = f.ins().band(s5, last);
            (first, count, fold) = emit(f, end5, acc, first, count, fold);
            st_ = f.ins().select(end5, zero, st_);
            cons = f.ins().bor(cons, s5);
            // 1: the escape's letter.
            let s1 = is(f, st_, 1);
            let ncons = f.ins().bxor_imm(cons, 1);
            let s1 = f.ins().band(s1, ncons);
            // A letter's character, else the byte itself (`\'`, `\\`, `\?`, …).
            let mut sv = c;
            for (ch, v) in [(b'n', 10), (b't', 9), (b'r', 13), (b'a', 7), (b'b', 8), (b'f', 12), (b'v', 11), (b'e', 27), (b'E', 27)] {
                let x = is(f, c, i64::from(ch));
                let vv = k(f, v);
                sv = f.ins().select(x, vv, sv);
            }
            let x = is(f, c, i64::from(b'x'));
            let u = is(f, c, i64::from(b'u'));
            let uu = is(f, c, i64::from(b'U'));
            let uni = f.ins().bor(u, uu);
            let other = f.ins().bor(is_oct, x);
            let other = f.ins().bor(other, uni);
            let plain = f.ins().bxor_imm(other, 1);
            let e1 = f.ins().band(s1, plain);
            (first, count, fold) = emit(f, e1, sv, first, count, fold);
            let o1 = f.ins().band(s1, is_oct);
            let x1 = f.ins().band(s1, x);
            let u1 = f.ins().band(s1, uni);
            let k2 = k(f, 2);
            let k3 = k(f, 3);
            let k4 = k(f, 4);
            let k8 = k(f, 8);
            st_ = f.ins().select(e1, zero, st_);
            st_ = f.ins().select(o1, k2, st_);
            st_ = f.ins().select(x1, k3, st_);
            st_ = f.ins().select(u1, k4, st_);
            acc = f.ins().select(o1, dd, acc);
            let starts = f.ins().bor(x1, u1);
            acc = f.ins().select(starts, zero, acc);
            let width = f.ins().select(uu, k8, k4);
            aux = f.ins().select(o1, zero, aux);
            aux = f.ins().select(x1, zero, aux);
            aux = f.ins().select(u1, width, aux);
            lead = f.ins().select(u1, width, lead);
            cons = f.ins().bor(cons, s1);
            // 0: a character, a `\`, or (wide) a UTF-8 character's lead byte.
            let s0 = is(f, st_, 0);
            let ncons = f.ins().bxor_imm(cons, 1);
            let s0 = f.ins().band(s0, ncons);
            let bs = is(f, c, i64::from(b'\\'));
            let hi_b = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, c, 0x80);
            let utf = f.ins().band(hi_b, wide);
            let nbs = f.ins().bxor_imm(bs, 1);
            let nutf = f.ins().bxor_imm(utf, 1);
            let ch = f.ins().band(s0, nbs);
            let ch = f.ins().band(ch, nutf);
            (first, count, fold) = emit(f, ch, c, first, count, fold);
            let b1 = f.ins().band(s0, bs);
            let one_ = k(f, 1);
            st_ = f.ins().select(b1, one_, st_);
            let u0 = f.ins().band(s0, utf);
            let in2 = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, c, 0xc2);
            let le2 = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, c, 0xdf);
            let r2 = f.ins().band(in2, le2);
            let ge3 = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, c, 0xe0);
            let le3 = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, c, 0xef);
            let r3 = f.ins().band(ge3, le3);
            let ge4 = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, c, 0xf0);
            let le4 = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, c, 0xf4);
            let r4 = f.ins().band(ge4, le4);
            let lead_ok = f.ins().bor(r2, r3);
            let lead_ok = f.ins().bor(lead_ok, r4);
            let nl = f.ins().bxor_imm(lead_ok, 1);
            let wrong = f.ins().band(u0, nl);
            let w64 = f.ins().uextend(types::I64, wrong);
            bad = f.ins().bor(bad, w64);
            let n_ = f.ins().select(r3, two, one);
            let n_ = f.ins().select(r4, three, n_);
            let m2 = f.ins().band_imm(c, 0x1f);
            let m3 = f.ins().band_imm(c, 0x0f);
            let m4 = f.ins().band_imm(c, 0x07);
            let a = f.ins().select(r3, m3, m2);
            let a = f.ins().select(r4, m4, a);
            let five = k(f, 5);
            st_ = f.ins().select(u0, five, st_);
            acc = f.ins().select(u0, a, acc);
            aux = f.ins().select(u0, n_, aux);
            lead = f.ins().select(u0, c, lead);
            vec![st_, acc, aux, lead, first, count, fold, bad]
        });
        let (state, acc, aux, first, count, fold, bad) = (st[0], st[1], st[2], st[4], st[5], st[6], st[7]);
        let f = &mut *self.f;
        // The body's end: an octal or hex escape ends with it; `\` or a part left is none.
        let s2 = f.ins().icmp_imm(IntCC::Equal, state, 2);
        let s3 = f.ins().icmp_imm(IntCC::Equal, state, 3);
        let ends = f.ins().bor(s2, s3);
        let (first, count, fold) = emit(f, ends, acc, first, count, fold);
        let aux0 = f.ins().icmp_imm(IntCC::Equal, aux, 0);
        let hex_empty = f.ins().band(s3, aux0);
        let s1 = f.ins().icmp_imm(IntCC::Equal, state, 1);
        let s4 = f.ins().icmp_imm(IntCC::Equal, state, 4);
        let s5 = f.ins().icmp_imm(IntCC::Equal, state, 5);
        let left = f.ins().bor(s1, s4);
        let left = f.ins().bor(left, s5);
        let left = f.ins().bor(left, hex_empty);
        let badb = f.ins().icmp_imm(IntCC::NotEqual, bad, 0);
        let fail = f.ins().bor(left, badb);
        let some = f.ins().icmp_imm(IntCC::NotEqual, count, 0);
        let nf = f.ins().bxor_imm(fail, 1);
        let ok = f.ins().band(some, nf);
        let ok = f.ins().band(ok, has_body);
        // Its value by prefix: a plain one's one character a signed `char`, several packed as
        // `int`; `u8`, `u`, `L` (signed 32 bits), `U`.
        let one = f.ins().icmp_imm(IntCC::Equal, count, 1);
        let c8 = f.ins().ireduce(types::I8, first);
        let c8 = f.ins().sextend(types::I64, c8);
        let c32 = f.ins().ireduce(types::I32, fold);
        let c32 = f.ins().sextend(types::I64, c32);
        let plain = f.ins().select(one, c8, c32);
        let u8v = f.ins().band_imm(first, 0xff);
        let u16v = f.ins().band_imm(first, 0xffff);
        let l32 = f.ins().ireduce(types::I32, first);
        let l32 = f.ins().sextend(types::I64, l32);
        let u32v = f.ins().band_imm(first, 0xffff_ffff);
        let mut v = plain;
        // No prefix, or one of the four.
        let mut known = f.ins().icmp_imm(IntCC::Equal, ln, 0);
        let pres: [(&[u8], Value); 4] = [(b"u8", u8v), (b"u", u16v), (b"L", l32), (b"U", u32v)];
        for (pre, val) in pres {
            let e = self.spelled_at(lp, ln, pre);
            v = self.f.ins().select(e, val, v);
            known = self.f.ins().bor(known, e);
        }
        let ok = self.f.ins().band(ok, known);
        (ok, v)
    }

    /// `EXPR m=5`: `T`'s value as a primary (`pp_expr::primary`): a number, a character
    /// constant, an identifier (0, C++'s `true` 1); any other an error and a 0.
    fn expr_primary(&mut self, pc: usize) {
        let e = self.ex_state();
        let kind = ldu(self.f, types::I8, e, ex!(t) + T_KIND);
        let (p, n) = self.spelling(e, ex!(t));
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I8);
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I8);
        let z = self.f.ins().iconst(types::I64, 0);
        let f8 = self.f.ins().iconst(types::I8, 0);
        let num = self.f.create_block();
        let not_num = self.f.create_block();
        let is_num = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(NUMBER));
        self.f.ins().brif(is_num, num, &[], not_num, &[]);
        self.f.switch_to_block(num);
        let (ok, v, u) = self.number(p, n);
        self.f.ins().jump(join, &[ok.into(), v.into(), u.into()]);
        self.f.switch_to_block(not_num);
        let lit = self.f.create_block();
        let not_lit = self.f.create_block();
        let is_lit = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(LITERAL));
        self.f.ins().brif(is_lit, lit, &[], not_lit, &[]);
        self.f.switch_to_block(lit);
        let (found, q) = self.find_first(p, n, Self::quote_class);
        let chr = self.f.create_block();
        self.f.ins().brif(found, chr, &[], join, &[f8.into(), z.into(), f8.into()]);
        self.f.switch_to_block(chr);
        let qp = self.f.ins().iadd(p, q);
        let qc = ldu(self.f, types::I8, qp, 0);
        let single = self.f.ins().icmp_imm(IntCC::Equal, qc, i64::from(b'\''));
        let cv = self.f.create_block();
        self.f.ins().brif(single, cv, &[], join, &[f8.into(), z.into(), f8.into()]);
        self.f.switch_to_block(cv);
        let (ok, v) = self.char_value(p, n, q);
        let f8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().jump(join, &[ok.into(), v.into(), f8.into()]);
        self.f.switch_to_block(not_lit);
        let ident = self.f.create_block();
        let other = self.f.create_block();
        let is_id = self.f.ins().icmp_imm(IntCC::Equal, kind, i64::from(IDENT));
        self.f.ins().brif(is_id, ident, &[], other, &[]);
        self.f.switch_to_block(ident);
        let f8 = self.f.ins().iconst(types::I8, 0);
        // C's: an identifier is 0.
        let c_ident = self.f.create_block();
        if self.pp.cpp {
            // C++'s, where the run is C++'s: an alternative token is an operator, no primary;
            // `true` is 1.
            let cpp = self.is_cpp();
            let cpp_ident = self.f.create_block();
            self.f.ins().brif(cpp, cpp_ident, &[], c_ident, &[]);
            self.f.switch_to_block(cpp_ident);
            let mut alt = self.f.ins().iconst(types::I8, 0);
            for (name, _) in crate::pp_expr::ALT {
                let e = self.spelled_at(p, n, name);
                alt = self.f.ins().bor(alt, e);
            }
            let t = self.spelled_at(p, n, b"true");
            let tv = self.f.ins().uextend(types::I64, t);
            let ok = self.f.ins().bxor_imm(alt, 1);
            self.f.ins().jump(join, &[ok.into(), tv.into(), f8.into()]);
        } else {
            self.f.ins().jump(c_ident, &[]);
        }
        self.f.switch_to_block(c_ident);
        let f8 = self.f.ins().iconst(types::I8, 0);
        let one = self.f.ins().iconst(types::I8, 1);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(join, &[one.into(), z.into(), f8.into()]);
        self.f.switch_to_block(other);
        let f8 = self.f.ins().iconst(types::I8, 0);
        let z = self.f.ins().iconst(types::I64, 0);
        self.f.ins().jump(join, &[f8.into(), z.into(), f8.into()]);
        self.f.switch_to_block(join);
        let r = self.f.block_params(join).to_vec();
        let (ok, v, u) = (r[0], r[1], r[2]);
        let bad = self.f.create_block();
        let push = self.f.create_block();
        self.f.append_block_param(push, types::I64);
        self.f.append_block_param(push, types::I8);
        self.f.ins().brif(ok, push, &[v.into(), u.into()], bad, &[]);
        self.f.switch_to_block(bad);
        self.expr_err();
        let z = self.f.ins().iconst(types::I64, 0);
        let f8 = self.f.ins().iconst(types::I8, 0);
        self.f.ins().jump(push, &[z.into(), f8.into()]);
        self.f.switch_to_block(push);
        let (v, u) = (self.f.block_params(push)[0], self.f.block_params(push)[1]);
        self.val_push(pc, v, u);
        self.next(pc);
    }

    /// `EXPR` (`Machine::expr`), its modes but the primary's (`m=5`, `expr_primary`).
    fn expr(&mut self, pc: usize, mode: u8) {
        let next = self.blocks[pc + 1];
        let skip = self.blocks.get(pc + 2).copied().unwrap_or(next);
        match mode {
            0 => {
                // The next token, a step; at the end (or after an error) an error and a 0.
                let e = self.ex_state();
                let err = ldu(self.f, types::I8, e, ex!(err));
                let at = ldu(self.f, types::I32, e, ex!(at));
                let hi = ldu(self.f, types::I32, e, ex!(hi));
                let end = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, hi);
                let errb = self.f.ins().icmp_imm(IntCC::NotEqual, err, 0);
                let stop = self.f.ins().bor(end, errb);
                let fail = self.f.create_block();
                let take = self.f.create_block();
                self.f.ins().brif(stop, fail, &[], take, &[]);
                self.f.switch_to_block(fail);
                self.expr_err();
                let z = self.f.ins().iconst(types::I64, 0);
                let z8 = self.f.ins().iconst(types::I8, 0);
                self.val_push(pc, z, z8);
                self.f.ins().jump(skip, &[]);
                self.f.switch_to_block(take);
                let t = self.vat(KEPT, at);
                let e = self.ex_state();
                copy_tok(self.f, t, 0, e, ex!(t));
                let a1 = self.f.ins().iadd_imm(at, 1);
                st(self.f, types::I32, a1, e, ex!(at));
                self.f.ins().jump(next, &[]);
            }
            1 => {
                let e = self.ex_state();
                let op = self.expr_op(e, ex!(t));
                let mut is = self.f.ins().iconst(types::I8, 0);
                for o in [&b"+"[..], b"-", b"~", b"!"] {
                    let x = self.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(o)));
                    is = self.f.ins().bor(is, x);
                }
                let push = self.f.create_block();
                self.f.ins().brif(is, push, &[], skip, &[]);
                self.f.switch_to_block(push);
                self.byte_push(pc, OPSV, op);
                self.f.ins().jump(next, &[]);
            }
            2 => {
                let op = self.byte_pop(pc, OPSV, R_NO_OP);
                let p = self.val_top(pc);
                let bits = ld(self.f, types::I64, p, V_BITS);
                let neg = self.f.ins().ineg(bits);
                let not = self.f.ins().bnot(bits);
                let z = self.f.ins().icmp_imm(IntCC::Equal, bits, 0);
                let lnot = self.f.ins().uextend(types::I64, z);
                let is = |l: &mut Self, o: &[u8]| l.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(o)));
                let m = is(self, b"-");
                let t = is(self, b"~");
                let x = is(self, b"!");
                let r = self.f.ins().select(m, neg, bits);
                let r = self.f.ins().select(t, not, r);
                let r = self.f.ins().select(x, lnot, r);
                st(self.f, types::I64, r, p, V_BITS);
                let u = ld(self.f, types::I8, p, V_UNS);
                let f8 = self.f.ins().iconst(types::I8, 0);
                let u = self.f.ins().select(x, f8, u);
                st(self.f, types::I8, u, p, V_UNS);
                self.f.ins().jump(next, &[]);
            }
            3 => {
                let e = self.ex_state();
                let op = self.expr_op(e, ex!(t));
                let x = self.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(b"(")));
                self.test(pc, x);
            }
            4 | 12 => {
                if mode == 12 {
                    self.live_pop();
                }
                let want = if mode == 4 { b")" } else { b":" };
                let op = self.expr_peek();
                let x = self.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(want)));
                let take = self.f.create_block();
                let err = self.f.create_block();
                let done = self.f.create_block();
                self.f.ins().brif(x, take, &[], err, &[]);
                self.f.switch_to_block(take);
                self.expr_take();
                self.f.ins().jump(done, &[]);
                self.f.switch_to_block(err);
                self.expr_err();
                self.f.ins().jump(done, &[]);
                self.f.switch_to_block(done);
                if mode == 12 {
                    // The `:` operand evaluated where the condition is false.
                    let n = self.vlen(VALS);
                    let ok = self.f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, n, 2);
                    self.check(ok, pc, R_NO_VALUE);
                    let i = self.f.ins().iadd_imm(n, -2);
                    let p = self.vat(VALS, i);
                    let c = ld(self.f, types::I64, p, V_BITS);
                    let f = self.f.ins().icmp_imm(IntCC::Equal, c, 0);
                    let l = self.live_top();
                    let live = self.f.ins().band(l, f);
                    self.live_push(pc, live);
                }
                self.f.ins().jump(next, &[]);
            }
            6 | 11 => {
                let want = if mode == 6 { b"," } else { b"?" };
                let op = self.expr_peek();
                let x = self.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(want)));
                let yes = self.f.create_block();
                self.f.ins().brif(x, yes, &[], skip, &[]);
                self.f.switch_to_block(yes);
                if mode == 6 {
                    self.expr_take();
                    self.val_pop(pc);
                } else {
                    let n = self.vlen(LIVE);
                    let room = ld(self.f, types::I64, self.ctx, at!(room_live));
                    let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, n, room);
                    self.check(ok, pc, R_EX);
                    self.expr_take();
                    let p = self.val_top(pc);
                    let c = ld(self.f, types::I64, p, V_BITS);
                    let t = self.f.ins().icmp_imm(IntCC::NotEqual, c, 0);
                    let l = self.live_top();
                    let live = self.f.ins().band(l, t);
                    self.live_push(pc, live);
                }
                self.f.ins().jump(next, &[]);
            }
            7 => {
                let one = self.f.ins().iconst(types::I8, 1);
                self.byte_push(pc, MINS, one);
                self.f.ins().jump(next, &[]);
            }
            8 => {
                // A binary operator of at least the least precedence: taken, its right operand's
                // least precedence one more.
                let mp = self.vtop(MINS, pc, R_NO_PREC);
                let min = ldu(self.f, types::I8, mp, 0);
                let op = self.expr_peek();
                let some = self.f.ins().icmp_imm(IntCC::NotEqual, op, NO_OP);
                let look = self.f.create_block();
                self.f.ins().brif(some, look, &[], skip, &[]);
                self.f.switch_to_block(look);
                let p = self.precedence(op);
                let is = self.f.ins().icmp_imm(IntCC::NotEqual, p, 0);
                let enough = self.f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, p, min);
                let go = self.f.ins().band(is, enough);
                let take = self.f.create_block();
                self.f.ins().brif(go, take, &[], skip, &[]);
                self.f.switch_to_block(take);
                for v in [MINS, OPSV, LIVE] {
                    let n = self.vlen(v);
                    let room = ld(self.f, types::I64, self.ctx, v.room);
                    let ok = self.f.ins().icmp(IntCC::UnsignedLessThan, n, room);
                    self.check(ok, pc, R_EX);
                }
                self.expr_take();
                let lp = self.val_top(pc);
                let l = ld(self.f, types::I64, lp, V_BITS);
                let lt = self.f.ins().icmp_imm(IntCC::NotEqual, l, 0);
                let lf = self.f.ins().icmp_imm(IntCC::Equal, l, 0);
                let and = self.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(b"&&")));
                let or = self.f.ins().icmp_imm(IntCC::Equal, op, i64::from(op_index(b"||")));
                let one = self.f.ins().iconst(types::I8, 1);
                let x = self.f.ins().select(and, lt, one);
                let x = self.f.ins().select(or, lf, x);
                let lv = self.live_top();
                let live = self.f.ins().band(lv, x);
                self.byte_push(pc, OPSV, op);
                let p1 = self.f.ins().iadd_imm(p, 1);
                self.byte_push(pc, MINS, p1);
                self.live_push(pc, live);
                self.f.ins().jump(next, &[]);
            }
            9 => {
                let op = self.byte_pop(pc, OPSV, R_NO_OP);
                self.live_pop();
                let live = self.live_top();
                let r = self.val_pop(pc);
                let l = self.val_pop(pc);
                let (bits, uns) = self.apply(pc, op, l, r, live);
                self.val_push(pc, bits, uns);
                self.f.ins().jump(next, &[]);
            }
            10 => {
                self.byte_pop(pc, MINS, R_NO_PREC);
                self.f.ins().jump(next, &[]);
            }
            13 => {
                self.live_pop();
                let (b, bu) = self.val_pop(pc);
                let (a, au) = self.val_pop(pc);
                let (c, _) = self.val_pop(pc);
                let t = self.f.ins().icmp_imm(IntCC::NotEqual, c, 0);
                let v = self.f.ins().select(t, a, b);
                let u = self.f.ins().bor(au, bu);
                self.val_push(pc, v, u);
                self.f.ins().jump(next, &[]);
            }
            _ => {
                // The expression's end: true where it read every token and its value is not 0.
                let e = self.ex_state();
                let err = ldu(self.f, types::I8, e, ex!(err));
                let at = ldu(self.f, types::I32, e, ex!(at));
                let hi = ldu(self.f, types::I32, e, ex!(hi));
                let lo = ldu(self.f, types::I32, e, ex!(lo));
                let all = self.f.ins().icmp(IntCC::Equal, at, hi);
                let fine = self.f.ins().icmp_imm(IntCC::Equal, err, 0);
                let ok = self.f.ins().band(all, fine);
                let n = self.vlen(VALS);
                let j = self.f.create_block();
                self.f.append_block_param(j, types::I8);
                let look = self.f.create_block();
                let no = self.f.ins().iconst(types::I8, 0);
                self.f.ins().brif(n, look, &[], j, &[no.into()]);
                self.f.switch_to_block(look);
                let n1 = self.f.ins().iadd_imm(n, -1);
                let p = self.vat(VALS, n1);
                let v = ld(self.f, types::I64, p, V_BITS);
                let t = self.f.ins().icmp_imm(IntCC::NotEqual, v, 0);
                self.f.ins().jump(j, &[t.into()]);
                self.f.switch_to_block(j);
                let t = self.f.block_params(j)[0];
                let held = self.f.ins().band(ok, t);
                self.vset_len(KEPT, lo);
                self.test(pc, held);
            }
        }
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
                let r = self.f.ins().iconst(types::I32, i64::from(self.ranks[usize::from(operand)]));
                self.push_return(pc, r);
                self.next(pc);
            }
            RETURN => {
                let n = ld(self.f, types::I64, ctx, at!(nreturns));
                let some = self.f.ins().icmp_imm(IntCC::NotEqual, n, 0);
                self.check(some, pc, R_NO_CALL);
                let n1 = self.f.ins().iadd_imm(n, -1);
                st(self.f, types::I64, n1, ctx, at!(nreturns));
                let r = ld(self.f, types::I64, ctx, at!(returns));
                let slot = elem(self.f, r, n1, 4);
                let to = self.f.ins().load(types::I32, flags(), slot, 0);
                let targets: Vec<Block> = self.rets.iter().map(|&p| self.blocks[p]).collect();
                self.dispatch(pc, to, &targets);
            }
            READ => {
                let next = self.blocks[pc + 1];
                self.read_then(next);
            }
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
            DEFINE => self.define(pc, mode),
            SUB => self.sub(pc, mode),
            SKIP => self.skip(pc),
            INCLUDE => self.include(pc, mode),
            PRAGMA => self.pragma(pc),
            BUILTIN => self.builtin(pc, mode),
            LINEDIR => self.linedir(pc, mode),
            EMBED => self.embed(pc, mode),
            COND => self.cond(pc, mode),
            IF if mode != 1 => self.if_open(pc, mode == 0),
            IF => self.expr_begin(pc),
            EXPR if mode == 5 => self.expr_primary(pc),
            EXPR => self.expr(pc, mode),
            STR => self.str_word(pc, mode),
            GLUE => self.glue(pc),
            UNDEF => self.undef(pc),
            DEFINED => self.defined(pc),
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

    /// Bytes `p[0..min(rem, 8)]` as a little-endian word, zeros past them: one load of eight,
    /// masked to the `rem` left, where the eight are within `p`'s page (so the load cannot fault,
    /// though it may read past the spelling); else, the few left one by one (straight code: at
    /// most seven), as at a page's last seven bytes. A token's length decides only the mask, not
    /// a branch.
    fn chunk(&mut self, p: Value, rem: Value) -> Value {
        let f = &mut *self.f;
        let join = f.create_block();
        f.append_block_param(join, types::I64);
        let fast = f.create_block();
        let tail = f.create_block();
        let eight = f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, rem, 8);
        let at = f.ins().band_imm(p, 4095);
        let in_page = f.ins().icmp_imm(IntCC::UnsignedLessThanOrEqual, at, 4096 - 8);
        let safe = f.ins().bor(eight, in_page);
        f.ins().brif(safe, fast, &[], tail, &[]);
        f.switch_to_block(fast);
        let w = f.ins().load(types::I64, bytes_flags(), p, 0);
        let bits = f.ins().ishl_imm(rem, 3);
        let one = f.ins().iconst(types::I64, 1);
        let m = f.ins().ishl(one, bits);
        let m = f.ins().iadd_imm(m, -1);
        let all = f.ins().iconst(types::I64, -1);
        let m = f.ins().select(eight, all, m);
        let w = f.ins().band(w, m);
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
        let (slot, found) = self.probe(pc, p, n);
        let defined = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(found, defined, &[], join, &[no.into()]);
        self.f.switch_to_block(defined);
        let def = ldu(self.f, types::I32, slot, offset_of!(Slot, def) as i32);
        let is_def = self.f.ins().icmp_imm(IntCC::NotEqual, def, i64::from(NONE));
        let set = self.f.create_block();
        let no = self.f.ins().iconst(types::I8, 0);
        self.f.ins().brif(is_def, set, &[], join, &[no.into()]);
        self.f.switch_to_block(set);
        st(self.f, types::I64, def, ctx, at!(d));
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().jump(join, &[yes.into()]);
        self.f.switch_to_block(join);
        self.f.block_params(join)[0]
    }

    /// `Machine::find` of the `n` bytes at `p`: the slot's address, and whether it holds the name
    /// (else it is the empty slot where the name would go); the key, then a slot a step
    /// (`Step::Probe`), at most every slot once.
    fn probe(&mut self, pc: usize, p: Value, n: Value) -> (Value, Value) {
        let ctx = self.ctx;
        let join = self.f.create_block();
        self.f.append_block_param(join, types::I64);
        self.f.append_block_param(join, types::I8);
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
        self.f.ins().brif(empty, join, &[slot.into(), no.into()], used, &[]);
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
        let yes = self.f.ins().iconst(types::I8, 1);
        self.f.ins().brif(eq, join, &[slot.into(), yes.into()], on, &[]);
        self.f.switch_to_block(on);
        let i1 = self.f.ins().iadd_imm(i, 1);
        let i1 = self.f.ins().band(i1, mask);
        let t1 = self.f.ins().iadd_imm(tries, 1);
        self.f.ins().jump(head, &[i1.into(), t1.into()]);
        self.f.switch_to_block(join);
        let params = self.f.block_params(join);
        (params[0], params[1])
    }

    /// `READ`: the top source's next token (`Machine::read`). A source at its end is popped
    /// (`Step::Pop`) unless it is read again and again (a stop source, the main file, a file
    /// while arguments are read); a file's conditionals go with it, and its macro is enabled
    /// again.
    fn read_then(&mut self, next: Block) {
        let ctx = self.ctx;
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

/// A word whose code is over this many CLIF instructions is a function of its own, called
/// (`compile`): the run's function keeps the words a token passes through, not the rare large
/// ones (each of `BUILTIN`'s modes made the whole of `builtin_value` again).
pub const OUTLINE: usize = 512;

/// Each word's rank among the return points, and the return points by rank.
fn return_points(words: &[u32]) -> (Vec<u32>, Vec<usize>) {
    let mut ranks = vec![NONE; words.len()];
    let mut rets = Vec::new();
    for &w in words {
        let (op, _, operand) = split(w);
        if op == PUSH && ranks[usize::from(operand)] == NONE {
            ranks[usize::from(operand)] = rets.len() as u32;
            rets.push(usize::from(operand));
        }
    }
    (ranks, rets)
}

/// Each branch on a combination of tests made a chain of branches on the tests themselves:
/// `brif(bor(a, b), T, F)` is `brif(a, T, m); m: brif(b, T, F)`, `band` the dual, a `not`
/// (`bxor_imm x, 1`) swaps the targets, and a widened test is the test. The backend fuses a
/// compare into the branch that tests it, but materializes a combination (`cset`, `orr`, `uxtb`,
/// then `cbnz`): Q36's moves and selects. Only where every operand is a test (0 or 1), so
/// `band`'s split means what it did. The operands are computed before the first branch, as
/// before: nothing moves past a branch.
fn branch_on_tests(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::cursor::{Cursor, FuncCursor};
    use cranelift_codegen::ir::{InstructionData, Opcode, ValueDef};
    fn def(func: &cranelift_codegen::ir::Function, v: Value) -> Option<(Opcode, Vec<Value>, Option<i64>)> {
        let v = func.dfg.resolve_aliases(v);
        let ValueDef::Result(inst, 0) = func.dfg.value_def(v) else { return None };
        let data = &func.dfg.insts[inst];
        let imm = match data {
            InstructionData::BinaryImm64 { imm, .. } => Some(imm.bits()),
            _ => None,
        };
        Some((data.opcode(), func.dfg.inst_args(inst).iter().map(|&a| func.dfg.resolve_aliases(a)).collect(), imm))
    }
    fn is_test(func: &cranelift_codegen::ir::Function, v: Value, depth: u32) -> bool {
        depth < 8
            && match def(func, v) {
                Some((Opcode::Icmp | Opcode::IcmpImm, _, _)) => true,
                Some((Opcode::Band | Opcode::Bor | Opcode::Bxor, args, _)) => args.iter().all(|&a| is_test(func, a, depth + 1)),
                Some((Opcode::BxorImm, args, Some(1))) | Some((Opcode::Uextend, args, _)) => is_test(func, args[0], depth + 1),
                _ => false,
            }
    }
    let mut work: Vec<Block> = func.layout.blocks().collect();
    while let Some(block) = work.pop() {
        let Some(inst) = func.layout.last_inst(block) else { continue };
        let InstructionData::Brif { arg, blocks, .. } = func.dfg.insts[inst] else { continue };
        let pool = &func.dfg.value_lists;
        let call = |b: &BlockCall| (b.block(pool), b.args(pool).collect::<Vec<BlockArg>>());
        let ((t, targs), (e, eargs)) = (call(&blocks[0]), call(&blocks[1]));
        let Some((op, args, imm)) = def(func, arg) else { continue };
        enum Then {
            Or(Value, Value),
            And(Value, Value),
            Swap(Value),
            Same(Value),
        }
        let then = match (op, imm) {
            (Opcode::Bor, _) if is_test(func, arg, 0) => Then::Or(args[0], args[1]),
            (Opcode::Band, _) if is_test(func, arg, 0) => Then::And(args[0], args[1]),
            (Opcode::BxorImm, Some(1)) if is_test(func, args[0], 0) => Then::Swap(args[0]),
            (Opcode::Uextend, _) if is_test(func, args[0], 0) => Then::Same(args[0]),
            _ => continue,
        };
        let mut pos = FuncCursor::new(func).at_inst(inst);
        pos.remove_inst();
        match then {
            Then::Or(a, b) | Then::And(a, b) => {
                let m = pos.func.dfg.make_block();
                pos.func.layout.insert_block_after(m, block);
                let mut pos = FuncCursor::new(pos.func).at_bottom(block);
                if matches!(then, Then::Or(..)) {
                    pos.ins().brif(a, t, &targs, m, &[]);
                } else {
                    pos.ins().brif(a, m, &[], e, &eargs);
                }
                let mut pos = FuncCursor::new(pos.func).at_bottom(m);
                pos.ins().brif(b, t, &targs, e, &eargs);
                work.extend([block, m]);
            }
            Then::Swap(x) => {
                let mut pos = FuncCursor::new(pos.func).at_bottom(block);
                pos.ins().brif(x, e, &eargs, t, &targs);
                work.push(block);
            }
            Then::Same(x) => {
                let mut pos = FuncCursor::new(pos.func).at_bottom(block);
                pos.ins().brif(x, t, &targs, e, &eargs);
                work.push(block);
            }
        }
    }
}

/// Compile `program`'s words (validated first).
pub fn compile(program: &Words) -> Result<CompiledPp, Error> {
    compile_kept(program, false)
}

/// The compiled words' IR as Rust (`clif_rust`): the same functions, compiled by
/// rustc and LLVM ahead of time. Its entry is `pp_run`, run by `run_aot`; it ends with its
/// fingerprint, `SOURCE`, so a build runs it only where the words translate to it again.
pub fn rust_source(program: &Words) -> Result<String, String> {
    let c = compile_kept(program, true).map_err(|e| format!("{e:?}"))?;
    let one = crate::clif_rust::translate(&c.ir, &[(c.run_id, "pp_run")], &[])?;
    let h = crate::clif_rust::fingerprint(&one);
    Ok(format!("{one}\n/// This source's fingerprint (`clif_rust::fingerprint` of what comes before).\npub const SOURCE: u64 = {h:#x};\n"))
}

/// The interface an ahead-of-time build shares with the harness that runs it (`run_aot`): the
/// sizes of the context and of the records it points at, and the refusal reasons. A build
/// records it, and the host runs a build only where it is the compiler's now, so a build left
/// stale by a change of the compiler is never handed a context of another layout.
pub const ABI: u64 = {
    use std::mem::size_of;
    let parts = [
        size_of::<Ctx>(), size_of::<Tok>(), size_of::<Inc>(), size_of::<Want>(), size_of::<EmC>(), size_of::<Bi>(), size_of::<Asked>(),
        size_of::<FeatureC>(), size_of::<NamedC>(), size_of::<Src>(), size_of::<Def>(), size_of::<Slot>(), size_of::<Frame>(),
        size_of::<Ex>(), size_of::<DefState>(), size_of::<Sub>(), size_of::<FileC>(), REASONS.len(),
    ];
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut i = 0;
    while i < parts.len() {
        h = (h ^ parts[i] as u64).wrapping_mul(0x100_0000_01b3);
        i += 1;
    }
    h
};

/// `pp_words::run`'s work, by an ahead-of-time build of the words (`rust_source`'s `pp_run`).
///
/// # Safety
/// `entry` must be `rust_source`'s `pp_run` for `program`.
pub unsafe fn run_aot(entry: unsafe extern "C" fn(u64) -> u64, program: &Words, files: &[File], env: &Env, bounds: &Bounds) -> Result<Expanded, Refused> {
    // SAFETY: the same ABI: one pointer-sized argument, a 64-bit result.
    let code = unsafe { std::mem::transmute::<unsafe extern "C" fn(u64) -> u64, unsafe extern "C" fn(*mut Ctx) -> u64>(entry) };
    // SAFETY: as `CompiledPp::run`'s; the caller vouches the code is the words'.
    unsafe { pp_words::run_native(program, files, env, bounds, code, REASONS) }
}

/// The compiled words, their IR and machine code kept (`ir`, `disasm`): to read
/// Cranelift's code beside rustc's of the same IR (`rust_source`).
pub fn disassemble(program: &Words) -> Result<CompiledPp, Error> {
    compile_kept(program, true)
}

fn compile_kept(program: &Words, keep: bool) -> Result<CompiledPp, Error> {
    let mut ir = Vec::new();
    let mut disasm = Vec::new();
    let mut code_kept = Vec::new();
    pp_words::validate(program)?;
    let mut builder = JITBuilder::with_flags(&[("opt_level", "speed")], default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    // One arena for the module's code, so a call of an outlined word is a direct branch (its
    // `colocated`): JIT mappings apart can be past a branch's reach.
    builder.memory_provider(Box::new(cranelift_jit::ArenaMemoryProvider::new_with_size(64 << 20).map_err(|_| Error::BackendFailure)?));
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("pp_run", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
    let (ranks, rets) = return_points(&program.words);
    let mut fctx = FunctionBuilderContext::new();
    let mut clif = String::new();
    let mut all_heads = Vec::new();
    // The outlined words, a function each: 0 its next word, 1 the one after, else the refusal.
    let mut outlined = Vec::new();
    let mut sizes = Vec::new();
    // Each word lowered alone, and kept so where it is large: the words that go elsewhere than their next two stay in the run's function.
    for pc in (0..program.words.len()).filter(|&pc| !matches!(split(program.words[pc]).0, GOTO | PUSH | RETURN | ACCEPT | STOP)) {
        let mut context = module.make_context();
        context.func.signature = signature.clone();
        let heads;
        {
            let mut f = FunctionBuilder::new(&mut context.func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            let (held, skip, exit, body) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
            f.append_block_param(exit, types::I64);
            f.switch_to_block(entry);
            let ctx = f.block_params(entry)[0];
            f.ins().jump(body, &[]);
            let mut blocks = vec![skip; program.words.len()];
            blocks[pc + 1] = held;
            let mut l = Lower { f: &mut f, ctx, words: &program.words, strings: &program.strings, blocks, exit, heads: Vec::new(), pp: &program.pp, ranks: &ranks, rets: &rets };
            l.f.switch_to_block(body);
            l.word(pc)?;
            for (b, r) in [(held, 0), (skip, 1)] {
                l.f.switch_to_block(b);
                let v = l.f.ins().iconst(types::I64, r);
                l.f.ins().return_(&[v]);
            }
            l.f.switch_to_block(exit);
            let r = l.f.block_params(exit)[0];
            l.f.ins().return_(&[r]);
            heads = std::mem::take(&mut l.heads);
            f.seal_all_blocks();
            f.finalize();
        }
        if context.func.dfg.num_insts() <= OUTLINE {
            module.clear_context(&mut context);
            continue;
        }
        let fid = module.declare_function(&format!("pp_word_{pc}"), Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
        branch_on_tests(&mut context.func);
        crate::native_cpu_shape::check(&context.func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("word {pc}: {r:?}")))?;
        clif.push_str(&context.func.display().to_string());
        all_heads.extend(heads.iter().map(|h| h.1));
        if keep {
            ir.push((fid.as_u32(), context.func.clone()));
            context.set_disasm(true);
        }
        crate::native_cpu_codegen::tables_as_compares(&mut context.func);
        module.define_function(fid, &mut context).map_err(|e| Error::RefusedShape(format!("word {pc}: {e:?}")))?;
        if keep {
            disasm.push(context.compiled_code().and_then(|c| c.vcode.clone()).unwrap_or_default());
            code_kept.push(context.compiled_code().map_or(Vec::new(), |c| c.code_buffer().to_vec()));
        }
        sizes.push(context.compiled_code().map_or(0, |c| c.code_info().total_size as usize));
        module.clear_context(&mut context);
        outlined.push((pc, fid));
    }
    let mut word_insts = vec![0; program.words.len()];
    let own: Vec<UserExternalName> = outlined.iter().map(|&(_, fid)| UserExternalName::new(0, fid.as_u32())).collect();
    let mut context = module.make_context();
    context.func.signature = signature.clone();
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
        let mut calls = vec![None; program.words.len()];
        let sig = f.import_signature(signature.clone());
        for (&(pc, _), name) in outlined.iter().zip(&own) {
            let name = f.func.declare_imported_user_function(name.clone());
            calls[pc] = Some(f.import_function(ExtFuncData { name: ExternalName::user(name), signature: sig, colocated: true }));
        }
        f.ins().jump(blocks[0], &[]);
        let mut l = Lower { f: &mut f, ctx, words: &program.words, strings: &program.strings, blocks, exit, heads: Vec::new(), pp: &program.pp, ranks: &ranks, rets: &rets };
        for pc in 0..program.words.len() {
            let before = l.f.func.dfg.num_insts();
            let b = l.blocks[pc];
            l.f.switch_to_block(b);
            // A token read is a stated step: a cycle through `READ`'s block.
            let (op, mode, _) = split(l.words[pc]);
            if op == READ {
                l.heads.push((b, Step::Token));
            }
            if let Some(fref) = calls[pc] {
                let next = l.blocks[pc + 1];
                let skip = if pp_words::is_test(op, mode) { l.blocks[pc + 2] } else { next };
                l.call_word(fref, op, mode, next, skip);
            } else if pp_words::steps_where_held(op, mode) {
                // The held branch is the step: through a named block of its own.
                let next = l.blocks[pc + 1];
                let step = l.f.create_block();
                l.heads.push((step, if op == DEFINE && mode == 7 { Step::Item } else { Step::Token }));
                l.blocks[pc + 1] = step;
                let r = l.word(pc);
                l.blocks[pc + 1] = next;
                r?;
                l.f.switch_to_block(step);
                l.f.ins().jump(next, &[]);
            } else {
                l.word(pc)?;
            }
            word_insts[pc] = l.f.func.dfg.num_insts() - before;
        }
        l.f.switch_to_block(exit);
        let r = l.f.block_params(exit)[0];
        l.f.ins().return_(&[r]);
        heads = std::mem::take(&mut l.heads);
        f.seal_all_blocks();
        f.finalize();
    }
    // A debug aid: the verifier's errors, annotated on the function, written to this file.
    if let Ok(path) = std::env::var("RERAIL_PP_DEBUG_CLIF") {
        if let Err(e) = cranelift_codegen::verify_function(&context.func, module.isa()) {
            let _ = std::fs::write(path, cranelift_codegen::print_errors::pretty_verifier_error(&context.func, None, e));
        }
    }
    branch_on_tests(&mut context.func);
    crate::native_cpu_shape::check(&context.func, &heads, &own).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    clif.push_str(&context.func.display().to_string());
    all_heads.extend(heads.iter().map(|h| h.1));
    if keep {
        ir.push((id.as_u32(), context.func.clone()));
        context.set_disasm(true);
    }
    crate::native_cpu_codegen::tables_as_compares(&mut context.func);
    module.define_function(id, &mut context).map_err(|e| Error::RefusedShape(format!("pp_run: {e:?}")))?;
    if keep {
        disasm.insert(0, context.compiled_code().and_then(|c| c.vcode.clone()).unwrap_or_default());
        code_kept.insert(0, context.compiled_code().map_or(Vec::new(), |c| c.code_buffer().to_vec()));
    }
    sizes.insert(0, context.compiled_code().map_or(0, |c| c.code_info().total_size as usize));
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(id);
    // SAFETY: `pp_run` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, unsafe extern "C" fn(*mut Ctx) -> u64>(code) };
    Ok(CompiledPp { _module: module, entry, clif, heads: all_heads, code_bytes: sizes, word_insts, ir, run_id: id.as_u32(), disasm, code: code_kept })
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
