//! Tier 2 on the CPU JIT: Cranelift emits a program's words (`native_ir::StructuralOperation`'s
//! tier-2 words) as one function `tempo_stream(ctx)`. Each word is one block of code made from
//! the word and its operand alone, and its tables (a switch's targets, a string's bytes). Nothing in it is fixed for a language or a tape format.
//!
//! The machine's state (`stream::run`, the reference) is in SSA variables: the stream's cursors,
//! the token's start, end and limit, the end of the token before, the node and supertype
//! counts, and the open and return stacks' depths. Its output is the node table, one row per node
//! in preorder (kind, field, start, end, size), and the supertypes, one row each (node, kind).
//!
//! The loops are the stream's, as the verifier requires: every cycle passes a `TOKEN`.
//! `native_cpu_shape` refuses any other loop, and any call.
//!
//! Room is sized from the program, not checked per node: between two `TOKEN`s the words write at
//! most `per_token` nodes, supertypes and returns (the longest path, `Bounds`), so a stream of
//! `n` starts needs `per_token * (n + 2)`.

use super::Error;

use super::native_cpu_shape::Step;
use super::native_ir::{NativeModule, StructuralOperation as Op};
use super::stream::{ByteSwitch, Node, REFUSE, Refused};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, Block, BlockArg, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

/// The function's one argument: where everything is. Each field is 8 bytes.
#[repr(C)]
pub(super) struct Ctx {
    /// The file's first byte. One readable byte before it and 64 after its end.
    bytes: *const u8,
    len: u64,
    /// The extract `TOKEN` names, and its count; two readable slots past the count.
    starts: *const u32,
    nstarts: u64,
    /// The mask `SPAN m=1` names: its first word, and its number of words (one a 64 bytes).
    lasts: *const u64,
    nlasts: u64,
    totals: *const u64,
    kind: *mut u32,
    field: *mut u32,
    start: *mut u32,
    end: *mut u32,
    size: *mut u32,
    super_node: *mut u32,
    super_kind: *mut u32,
    open: *mut u32,
    returns: *mut u32,
    /// Written at `ACCEPT`: the node count, the supertype count.
    counts: *mut u64,
    /// The bytes from one of the mask's words to the next.
    lasts_stride: u64,
    /// Every output's word 0, interleaved as `lasts` is (output `o`'s word `w` at
    /// `o + w * stride` words): the masks `MARK` reads.
    outputs: *const u64,
}

/// The field offsets of `Ctx`.
const BYTES: i32 = 0;
const LEN: i32 = 8;
const STARTS: i32 = 16;
const NSTARTS: i32 = 24;
const LASTS: i32 = 32;
const NLASTS: i32 = 40;
const TOTALS: i32 = 48;
const KIND: i32 = 56;
const FIELD: i32 = 64;
const START: i32 = 72;
const END: i32 = 80;
const SIZE: i32 = 88;
const SUPER_NODE: i32 = 96;
const SUPER_KIND: i32 = 104;
const OPEN: i32 = 112;
const RETURNS: i32 = 120;
const COUNTS: i32 = 128;
const LASTS_STRIDE: i32 = 136;
const OUTPUTS: i32 = 144;

/// Returns 0 when the words accept, else one more than the refusing word's place.
type Entry = unsafe extern "C" fn(*const Ctx) -> u64;

/// The most a program writes between two `TOKEN`s: nodes, supertypes and returns pushed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Bounds {
    pub(super) nodes: u64,
    pub(super) supers: u64,
    pub(super) pushes: u64,
}

/// A compiled tier-2 program.
pub(super) struct CompiledStream {
    _module: JITModule,
    entry: Entry,
    bounds: Bounds,
    /// The extract `TOKEN` reads, and the output of the mask `SPAN m=1` reads.
    starts: Option<usize>,
    lasts: Option<usize>,
    /// The marks `MARK` tests: a token can be switched on once per mark and once more.
    marks: u64,
    pub(super) clif: String,
    pub(super) disassembly: String,
    /// The CLIF blocks of the loops' heads, with their steps (`native_cpu_shape`).
    pub(super) heads: Vec<(String, Step)>,
}

/// Each word's successors, as `stream::validate` reads them, `TOKEN`'s cut.
fn successors(native: &NativeModule, pushes: &[u16], pc: usize) -> Vec<usize> {
    match native.structural[pc].operation {
        Op::Stop | Op::Accept | Op::Token { .. } => vec![],
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => {
            let s = &native.switches[usize::from(table)];
            let mut out: Vec<usize> = s.targets.iter().chain([&s.end]).filter(|&&t| t != REFUSE).map(|&t| usize::from(t)).collect();
            out.sort_unstable();
            out.dedup();
            out
        }
        Op::Return { again } => pushes.iter().map(|&p| usize::from(p) + usize::from(again)).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } => vec![pc + 1, pc + 2],
        _ => vec![pc + 1],
    }
    .into_iter()
    .filter(|&n| n < native.structural.len())
    .collect()
}

/// The program's `Bounds`: the longest path from the entry or a `TOKEN` to the next `TOKEN`,
/// weighting `OPEN` and `LEAF` (a node), `NODE_SUPER` and `PUSH`. The graph with `TOKEN`'s edges
/// cut is acyclic (the verifier's rule), so each word's longest path is found once.
pub(super) fn bounds(native: &NativeModule) -> Bounds {
    let n = native.structural.len();
    let pushes = push_targets(native);
    let mut memo: Vec<Option<Bounds>> = vec![None; n];
    fn longest(native: &NativeModule, pushes: &[u16], memo: &mut Vec<Option<Bounds>>, pc: usize) -> Bounds {
        if let Some(b) = memo[pc] {
            return b;
        }
        let mut best = Bounds::default();
        for s in successors(native, pushes, pc) {
            if matches!(native.structural[s].operation, Op::Token { .. }) {
                continue;
            }
            let b = longest(native, pushes, memo, s);
            best = Bounds { nodes: best.nodes.max(b.nodes), supers: best.supers.max(b.supers), pushes: best.pushes.max(b.pushes) };
        }
        let own = match native.structural[pc].operation {
            Op::Open { .. } | Op::Leaf { .. } => Bounds { nodes: 1, ..Bounds::default() },
            Op::NodeSuper { .. } => Bounds { supers: 1, ..Bounds::default() },
            Op::Push { .. } => Bounds { pushes: 1, ..Bounds::default() },
            _ => Bounds::default(),
        };
        let b = Bounds { nodes: best.nodes + own.nodes, supers: best.supers + own.supers, pushes: best.pushes + own.pushes };
        memo[pc] = Some(b);
        b
    }
    let mut out = Bounds::default();
    // From the entry, and from the word after each `TOKEN` (whose own edges are cut).
    let token = |pc: usize| matches!(native.structural[pc].operation, Op::Token { .. });
    let froms: Vec<usize> = (0..n).filter(|&pc| pc == 0 && !token(pc)).chain((0..n).filter(|&pc| token(pc) && pc + 1 < n).map(|pc| pc + 1)).collect();
    for pc in froms {
        if token(pc) {
            continue;
        }
        let b = longest(native, &pushes, &mut memo, pc);
        out = Bounds { nodes: out.nodes.max(b.nodes), supers: out.supers.max(b.supers), pushes: out.pushes.max(b.pushes) };
    }
    out
}

/// The places `PUSH` names, sorted: where a `RETURN` can go.
fn push_targets(native: &NativeModule) -> Vec<u16> {
    let mut out: Vec<u16> = native
        .structural
        .iter()
        .filter_map(|b| match b.operation {
            Op::Push { target } => Some(target),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The machine's state, as variables.
struct Vars {
    next: Variable,
    /// The mask's word `lw`'s bits not yet taken; before the first word, `lw` is -1 (no bits).
    taken: Variable,
    lw: Variable,
    start: Variable,
    end: Variable,
    limit: Variable,
    last_end: Variable,
    at_end: Variable,
    nodes: Variable,
    supers: Variable,
    open: Variable,
    ret: Variable,
    /// 1 once the token's bytes are taken (`SPAN`): `TOKEN` then takes the next.
    need: Variable,
    /// The marks not yet taken or passed at this token, a bit each.
    live: Variable,
    /// The deferred extras' leaves at the table's end.
    pending: Variable,
    /// While leaves are pending, the end of the token before them: a node closed then ends there.
    keep: Variable,
}

/// A compare tree over a byte (or a place): runs of one target, split at the middle run.
fn dispatch(f: &mut FunctionBuilder<'_>, x: Value, runs: &[(u32, Block)]) {
    if runs.len() == 1 {
        f.ins().jump(runs[0].1, &[]);
        return;
    }
    let mid = runs.len() / 2;
    let (low, high) = (f.create_block(), f.create_block());
    let below = f.ins().icmp_imm(IntCC::UnsignedLessThan, x, i64::from(runs[mid].0));
    f.ins().brif(below, low, &[], high, &[]);
    f.switch_to_block(low);
    dispatch(f, x, &runs[..mid]);
    f.switch_to_block(high);
    dispatch(f, x, &runs[mid..]);
}

/// A mask tier 1 wrote (`block::BlockProgram::masks`): its first word, the bytes from one word
/// to the next, and its number of words.
#[derive(Clone, Copy)]
struct Mask {
    first: Value,
    stride: Value,
    words: Value,
}

/// Word `w` of a mask (`w` below its count).
fn mask_word(f: &mut FunctionBuilder<'_>, first: Value, stride: Value, w: Value) -> Value {
    let off = f.ins().imul(w, stride);
    let at = f.ins().iadd(first, off);
    f.ins().load(types::I64, MemFlags::trusted().with_readonly(), at, 0)
}

/// The first set bit of `m` from `bits`, word `w`'s bits not yet passed, on: a loop of a word
/// (64 bytes) a turn, its head given second. With `want`, a bit is found (the builder is left in
/// a block with parameters the bit's word and its bits) and the mask's end refuses at `pc`;
/// without, a bit refuses and the builder is left after the mask's end.
fn mask_scan(f: &mut FunctionBuilder<'_>, m: Mask, w: Value, bits: Value, refuse: Block, pc: usize, want: bool) -> (Block, Block) {
    let (scan, next, found, end) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    for b in [scan, found] {
        f.append_block_param(b, types::I64);
        f.append_block_param(b, types::I64);
    }
    let at = f.ins().iconst(types::I64, pc as i64 + 1);
    f.ins().jump(scan, &[w.into(), bits.into()]);
    f.switch_to_block(scan);
    let (w, bits) = (f.block_params(scan)[0], f.block_params(scan)[1]);
    let none = f.ins().icmp_imm(IntCC::Equal, bits, 0);
    f.ins().brif(none, next, &[], found, &[w.into(), bits.into()]);
    f.switch_to_block(next);
    let w1 = f.ins().iadd_imm(w, 1);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, w1, m.words);
    let more = f.create_block();
    f.ins().brif(past, end, &[], more, &[]);
    f.switch_to_block(more);
    let word = mask_word(f, m.first, m.stride, w1);
    f.ins().jump(scan, &[w1.into(), word.into()]);
    if want {
        f.switch_to_block(end);
        f.ins().jump(refuse, &[at.into()]);
        f.switch_to_block(found);
        (found, scan)
    } else {
        f.switch_to_block(found);
        f.ins().jump(refuse, &[at.into()]);
        f.switch_to_block(end);
        (end, scan)
    }
}

/// A switch's runs: each maximal range of bytes with one target, as (first byte, block).
fn runs(s: &ByteSwitch, word: &dyn Fn(u16) -> Block) -> Vec<(u32, Block)> {
    let mut out: Vec<(u32, u16)> = Vec::new();
    for (b, &t) in s.targets.iter().enumerate() {
        if out.last().is_none_or(|&(_, last)| last != t) {
            out.push((b as u32, t));
        }
    }
    out.into_iter().map(|(b, t)| (b, word(t))).collect()
}

/// Compile a program's tier-2 words.
pub(super) fn compile(native: &NativeModule) -> Result<CompiledStream, Error> {
    let words = &native.structural;
    let pushes = push_targets(native);
    let named = |op: fn(&Op) -> Option<u16>| words.iter().find_map(|b| op(&b.operation)).map(usize::from);
    let starts = named(|o| if let Op::Token { extract } = *o { Some(extract) } else { None });
    // The mask `SPAN m=1` names, as its output's index.
    let lasts = named(|o| if let Op::Span { last: true, len } = *o { Some(len) } else { None }).and_then(|m| native.block_programs.first().and_then(|b| b.masks.get(m).copied()));

    let builder = JITBuilder::new(default_libcall_names()).map_err(|_| Error::BackendFailure)?;
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("tempo_stream", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
    let mut context: Context = module.make_context();
    context.func.signature = signature;
    context.set_disasm(true);
    let mut fctx = FunctionBuilderContext::new();
    let mut heads: Vec<(Block, Step)> = Vec::new();
    let shape;
    {
        let mut f = FunctionBuilder::new(&mut context.func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let refuse = f.create_block();
        f.append_block_param(refuse, types::I64);
        let wb: Vec<Block> = (0..words.len()).map(|_| f.create_block()).collect();
        // A place past the words refuses, as the interpreter's `STOP`.
        let off_end = f.create_block();
        let word = |t: u16| -> Block { if t == REFUSE { off_end } else { wb.get(usize::from(t)).copied().unwrap_or(off_end) } };
        let v = Vars {
            next: f.declare_var(types::I64),
            taken: f.declare_var(types::I64),
            lw: f.declare_var(types::I64),
            start: f.declare_var(types::I64),
            end: f.declare_var(types::I64),
            limit: f.declare_var(types::I64),
            last_end: f.declare_var(types::I64),
            at_end: f.declare_var(types::I64),
            nodes: f.declare_var(types::I64),
            supers: f.declare_var(types::I64),
            open: f.declare_var(types::I64),
            ret: f.declare_var(types::I64),
            need: f.declare_var(types::I64),
            live: f.declare_var(types::I64),
            pending: f.declare_var(types::I64),
            keep: f.declare_var(types::I64),
        };

        f.switch_to_block(entry);
        let ctx = f.block_params(entry)[0];
        let ro = MemFlags::trusted().with_readonly();
        let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, ro, ctx, at);
        let bytes = load(&mut f, BYTES);
        let len = load(&mut f, LEN);
        let starts_p = load(&mut f, STARTS);
        let nstarts = load(&mut f, NSTARTS);
        let lasts_p = load(&mut f, LASTS);
        let nlasts = load(&mut f, NLASTS);
        let lasts_stride = load(&mut f, LASTS_STRIDE);
        let totals_p = load(&mut f, TOTALS);
        let outputs_p = load(&mut f, OUTPUTS);
        let cols: Vec<Value> = [KIND, FIELD, START, END, SIZE, SUPER_NODE, SUPER_KIND, OPEN, RETURNS, COUNTS].iter().map(|&at| load(&mut f, at)).collect();
        let (kind_c, field_c, start_c, end_c, size_c, snode_c, skind_c, open_c, ret_c, counts_c) =
            (cols[0], cols[1], cols[2], cols[3], cols[4], cols[5], cols[6], cols[7], cols[8], cols[9]);
        let zero = f.ins().iconst(types::I64, 0);
        for var in [v.next, v.taken, v.start, v.end, v.limit, v.last_end, v.at_end, v.nodes, v.supers, v.open, v.ret, v.live, v.pending, v.keep] {
            f.def_var(var, zero);
        }
        let one = f.ins().iconst(types::I64, 1);
        f.def_var(v.need, one);
        let before_first = f.ins().iconst(types::I64, -1);
        f.def_var(v.lw, before_first);
        match wb.first() {
            Some(&b) => f.ins().jump(b, &[]),
            None => f.ins().jump(off_end, &[]),
        };

        let flags = MemFlags::trusted();
        // Refuse at `pc` when `bad`; else go on in a new block.
        let guard = |f: &mut FunctionBuilder<'_>, bad: Value, pc: usize| {
            let ok = f.create_block();
            let at = f.ins().iconst(types::I64, pc as i64 + 1);
            f.ins().brif(bad, refuse, &[BlockArg::from(at)], ok, &[]);
            f.switch_to_block(ok);
        };
        // Column `c`'s slot `i` (u32s).
        let slot = |f: &mut FunctionBuilder<'_>, c: Value, i: Value| {
            let off = f.ins().ishl_imm(i, 2);
            f.ins().iadd(c, off)
        };
        let put = |f: &mut FunctionBuilder<'_>, c: Value, i: Value, x: Value| {
            let at = slot(f, c, i);
            f.ins().istore32(flags, x, at, 0);
        };

        for (pc, w) in words.iter().enumerate() {
            f.switch_to_block(wb[pc]);
            let fall = wb.get(pc + 1).copied().unwrap_or(off_end);
            match w.operation {
                Op::Token { .. } => {
                    heads.push((wb[pc], Step::Token));
                    // A token whose bytes are not taken yet (after a mark's arm) is kept.
                    let take = f.create_block();
                    let need = f.use_var(v.need);
                    f.ins().brif(need, take, &[], fall, &[]);
                    f.switch_to_block(take);
                    f.def_var(v.need, zero);
                    let all = f.ins().iconst(types::I64, -1);
                    f.def_var(v.live, all);
                    let at_end = f.use_var(v.at_end);
                    guard(&mut f, at_end, pc);
                    let (next, end) = (f.use_var(v.next), f.use_var(v.end));
                    f.def_var(v.last_end, end);
                    let is_end = f.ins().icmp(IntCC::Equal, next, nstarts);
                    // Two slots past the count are readable: `s` and the next start are loaded
                    // whether or not the stream has them, and chosen with `select`.
                    let s = {
                        let at = slot(&mut f, starts_p, next);
                        f.ins().uload32(flags, at, 0)
                    };
                    let inside = f.ins().icmp(IntCC::UnsignedLessThan, s, end);
                    let not_end = f.ins().bxor_imm(is_end, 1);
                    let bad = f.ins().band(inside, not_end);
                    guard(&mut f, bad, pc);
                    let (start, end, limit) = (f.use_var(v.start), f.use_var(v.end), f.use_var(v.limit));
                    let start = f.ins().select(is_end, start, s);
                    let end = f.ins().select(is_end, end, s);
                    let step = f.ins().uextend(types::I64, not_end);
                    let after = f.ins().iadd(next, step);
                    let following = {
                        let at = slot(&mut f, starts_p, after);
                        f.ins().uload32(flags, at, 0)
                    };
                    let has = f.ins().icmp(IntCC::UnsignedLessThan, after, nstarts);
                    let fresh = f.ins().select(has, following, len);
                    let limit = f.ins().select(is_end, limit, fresh);
                    let at_end = f.ins().uextend(types::I64, is_end);
                    f.def_var(v.start, start);
                    f.def_var(v.end, end);
                    f.def_var(v.next, after);
                    f.def_var(v.limit, limit);
                    f.def_var(v.at_end, at_end);
                    f.ins().jump(fall, &[]);
                }
                Op::Switch { table } => {
                    let s = &native.switches[usize::from(table)];
                    f.def_var(v.live, zero);
                    let at_end = f.use_var(v.at_end);
                    let (ended, taking) = (f.create_block(), f.create_block());
                    f.ins().brif(at_end, ended, &[], taking, &[]);
                    f.switch_to_block(ended);
                    if s.end == REFUSE {
                        let at = f.ins().iconst(types::I64, pc as i64 + 1);
                        f.ins().jump(refuse, &[BlockArg::from(at)]);
                    } else {
                        f.ins().jump(word(s.end), &[]);
                    }
                    f.switch_to_block(taking);
                    let (start, limit) = (f.use_var(v.start), f.use_var(v.limit));
                    let at = f.ins().iadd_imm(start, i64::from(s.offset));
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, limit);
                    guard(&mut f, past, pc);
                    let addr = f.ins().iadd(bytes, at);
                    let b = f.ins().uload8(types::I64, flags, addr, 0);
                    // A refusing target refuses at this word.
                    let no = f.create_block();
                    let t = runs(s, &|t| if t == REFUSE { no } else { word(t) });
                    dispatch(&mut f, b, &t);
                    f.switch_to_block(no);
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                }
                Op::Span { last: false, len: n } => {
                    let start = f.use_var(v.start);
                    let e = f.ins().iadd_imm(start, i64::from(n));
                    let limit = f.use_var(v.limit);
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, e, limit);
                    guard(&mut f, past, pc);
                    f.def_var(v.end, e);
                    let one = f.ins().iconst(types::I64, 1);
                    f.def_var(v.need, one);
                    f.ins().jump(fall, &[]);
                }
                Op::Span { last: true, .. } => {
                    // The next of the mask's bits after the last one taken: the bits of word
                    // `lw` not yet taken (`taken`), then a word (64 bytes) at a time.
                    let (w, bits) = (f.use_var(v.lw), f.use_var(v.taken));
                    let (found, scan) = mask_scan(&mut f, Mask { first: lasts_p, stride: lasts_stride, words: nlasts }, w, bits, refuse, pc, true);
                    heads.push((scan, Step::Block));
                    let (w, bits) = (f.block_params(found)[0], f.block_params(found)[1]);
                    let tz = f.ins().ctz(bits);
                    let at = f.ins().ishl_imm(w, 6);
                    let l = f.ins().iadd(at, tz);
                    let less = f.ins().iadd_imm(bits, -1);
                    let rest = f.ins().band(bits, less);
                    f.def_var(v.taken, rest);
                    f.def_var(v.lw, w);
                    let start = f.use_var(v.start);
                    let before = f.ins().icmp(IntCC::UnsignedLessThan, l, start);
                    guard(&mut f, before, pc);
                    let e = f.ins().iadd_imm(l, 1);
                    let limit = f.use_var(v.limit);
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, e, limit);
                    guard(&mut f, past, pc);
                    f.def_var(v.end, e);
                    let one = f.ins().iconst(types::I64, 1);
                    f.def_var(v.need, one);
                    f.ins().jump(fall, &[]);
                }
                Op::Word { string, test } => {
                    // The token's bytes against the string: its length, then an 8-byte load per
                    // 8 bytes of the string (64 readable bytes past the file, the token starting
                    // inside it), the last masked to its length. Straight-line: no loop.
                    let text = &native.strings[usize::from(string)];
                    let (start, end) = (f.use_var(v.start), f.use_var(v.end));
                    let n = f.ins().isub(end, start);
                    let mut same = f.ins().icmp_imm(IntCC::Equal, n, text.len() as i64);
                    let addr = f.ins().iadd(bytes, start);
                    for (i, chunk) in text.chunks(8).enumerate() {
                        let got = f.ins().load(types::I64, flags, addr, 8 * i as i32);
                        let mask = if chunk.len() == 8 { u64::MAX } else { (1u64 << (8 * chunk.len())) - 1 };
                        let got = f.ins().band_imm(got, mask as i64);
                        let mut want = [0u8; 8];
                        want[..chunk.len()].copy_from_slice(chunk);
                        let eq = f.ins().icmp_imm(IntCC::Equal, got, u64::from_le_bytes(want) as i64);
                        same = f.ins().band(same, eq);
                    }
                    if test {
                        let skip = wb.get(pc + 2).copied().unwrap_or(off_end);
                        f.ins().brif(same, fall, &[], skip, &[]);
                    } else {
                        let differs = f.ins().bxor_imm(same, 1);
                        guard(&mut f, differs, pc);
                        f.ins().jump(fall, &[]);
                    }
                }
                Op::Mark { mask, order, end } => {
                    // The mark's bit at the token's start, if the mark is live: one load. At the
                    // stream's end, the mark is there when the end has it.
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    let skip = wb.get(pc + 2).copied().unwrap_or(off_end);
                    let (at_end, live) = (f.use_var(v.at_end), f.use_var(v.live));
                    let lit = f.ins().ushr_imm(live, i64::from(order));
                    let lit = f.ins().band_imm(lit, 1);
                    let (look, ended) = (f.create_block(), f.create_block());
                    let taken = f.create_block();
                    let mid = f.create_block();
                    f.ins().brif(lit, mid, &[], skip, &[]);
                    f.switch_to_block(mid);
                    f.ins().brif(at_end, ended, &[], look, &[]);
                    f.switch_to_block(ended);
                    if end {
                        f.ins().jump(taken, &[]);
                    } else {
                        f.ins().jump(skip, &[]);
                    }
                    f.switch_to_block(look);
                    let start = f.use_var(v.start);
                    let w = f.ins().ushr_imm(start, 6);
                    let base = f.ins().iadd_imm(outputs_p, 8 * output as i64);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let bit = f.ins().ushr(word, s);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().brif(bit, taken, &[], skip, &[]);
                    f.switch_to_block(taken);
                    // It and the marks before it are done at this token.
                    let live = f.use_var(v.live);
                    let rest = f.ins().band_imm(live, !((2i64 << order) - 1));
                    f.def_var(v.live, rest);
                    f.ins().jump(fall, &[]);
                }
                Op::Row { mask } => {
                    // The row's start bit at the token's start: one load.
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    let skip = wb.get(pc + 2).copied().unwrap_or(off_end);
                    let start = f.use_var(v.start);
                    let w = f.ins().ushr_imm(start, 6);
                    let base = f.ins().iadd_imm(outputs_p, 8 * output as i64);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let bit = f.ins().ushr(word, s);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().brif(bit, fall, &[], skip, &[]);
                }
                Op::Open { kind } => {
                    f.def_var(v.pending, zero);
                    let (nodes, start, open) = (f.use_var(v.nodes), f.use_var(v.start), f.use_var(v.open));
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, kind_c, nodes, k);
                    put(&mut f, field_c, nodes, zero);
                    put(&mut f, start_c, nodes, start);
                    put(&mut f, open_c, open, nodes);
                    let open = f.ins().iadd_imm(open, 1);
                    let nodes = f.ins().iadd_imm(nodes, 1);
                    f.def_var(v.open, open);
                    f.def_var(v.nodes, nodes);
                    f.ins().jump(fall, &[]);
                }
                Op::Close { this } => {
                    let open = f.use_var(v.open);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    guard(&mut f, none, pc);
                    let open = f.ins().iadd_imm(open, -1);
                    f.def_var(v.open, open);
                    let at = {
                        let a = slot(&mut f, open_c, open);
                        f.ins().uload32(flags, a, 0)
                    };
                    let (nodes, pending) = (f.use_var(v.nodes), f.use_var(v.pending));
                    let e = if this {
                        f.use_var(v.end)
                    } else {
                        let (keep, last_end) = (f.use_var(v.keep), f.use_var(v.last_end));
                        f.ins().select(pending, keep, last_end)
                    };
                    put(&mut f, end_c, at, e);
                    let size = f.ins().isub(nodes, at);
                    let size = f.ins().isub(size, pending);
                    put(&mut f, size_c, at, size);
                    f.ins().jump(fall, &[]);
                }
                Op::Leaf { kind, deferred } => {
                    let pending = if deferred {
                        // The first pending leaf keeps the end of the token before it.
                        let (p, keep, last_end) = (f.use_var(v.pending), f.use_var(v.keep), f.use_var(v.last_end));
                        let keep = f.ins().select(p, keep, last_end);
                        f.def_var(v.keep, keep);
                        f.ins().iadd_imm(p, 1)
                    } else {
                        zero
                    };
                    f.def_var(v.pending, pending);
                    let (nodes, start, end) = (f.use_var(v.nodes), f.use_var(v.start), f.use_var(v.end));
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    let one = f.ins().iconst(types::I64, 1);
                    put(&mut f, kind_c, nodes, k);
                    put(&mut f, field_c, nodes, zero);
                    put(&mut f, start_c, nodes, start);
                    put(&mut f, end_c, nodes, end);
                    put(&mut f, size_c, nodes, one);
                    let nodes = f.ins().iadd_imm(nodes, 1);
                    f.def_var(v.nodes, nodes);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind } => {
                    // The innermost open node's kind.
                    let open = f.use_var(v.open);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    guard(&mut f, none, pc);
                    let top = f.ins().iadd_imm(open, -1);
                    let at = {
                        let a = slot(&mut f, open_c, top);
                        f.ins().uload32(flags, a, 0)
                    };
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, kind_c, at, k);
                    f.ins().jump(fall, &[]);
                }
                Op::Merge { kind } => {
                    // The grammar's leaf before the pending extras: a leaf, inside the innermost
                    // open node. The extras are its text: dropped.
                    let (nodes, pending, open) = (f.use_var(v.nodes), f.use_var(v.pending), f.use_var(v.open));
                    let kept = f.ins().isub(nodes, pending);
                    let none = f.ins().icmp_imm(IntCC::Equal, kept, 0);
                    guard(&mut f, none, pc);
                    let leaf = f.ins().iadd_imm(kept, -1);
                    let size = {
                        let a = slot(&mut f, size_c, leaf);
                        f.ins().uload32(flags, a, 0)
                    };
                    let not_leaf = f.ins().icmp_imm(IntCC::NotEqual, size, 1);
                    guard(&mut f, not_leaf, pc);
                    let any = f.ins().icmp_imm(IntCC::NotEqual, open, 0);
                    let top = f.ins().iadd_imm(open, -1);
                    let top = f.ins().select(any, top, zero);
                    let o = {
                        let a = slot(&mut f, open_c, top);
                        f.ins().uload32(flags, a, 0)
                    };
                    let inside = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, o, leaf);
                    let bad = f.ins().band(any, inside);
                    guard(&mut f, bad, pc);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    let end = f.use_var(v.end);
                    put(&mut f, kind_c, leaf, k);
                    put(&mut f, end_c, leaf, end);
                    f.def_var(v.nodes, kept);
                    f.def_var(v.pending, zero);
                    f.ins().jump(fall, &[]);
                }
                Op::NodeField { field } => {
                    let nodes = f.use_var(v.nodes);
                    let last = f.ins().iadd_imm(nodes, -1);
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    put(&mut f, field_c, last, x);
                    f.ins().jump(fall, &[]);
                }
                Op::NodeSuper { kind } => {
                    let (nodes, supers) = (f.use_var(v.nodes), f.use_var(v.supers));
                    let last = f.ins().iadd_imm(nodes, -1);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, snode_c, supers, last);
                    put(&mut f, skind_c, supers, k);
                    let supers = f.ins().iadd_imm(supers, 1);
                    f.def_var(v.supers, supers);
                    f.ins().jump(fall, &[]);
                }
                Op::Push { target } => {
                    let ret = f.use_var(v.ret);
                    let t = f.ins().iconst(types::I64, i64::from(target));
                    put(&mut f, ret_c, ret, t);
                    let ret = f.ins().iadd_imm(ret, 1);
                    f.def_var(v.ret, ret);
                    f.ins().jump(fall, &[]);
                }
                Op::Return { again } => {
                    let ret = f.use_var(v.ret);
                    let none = f.ins().icmp_imm(IntCC::Equal, ret, 0);
                    guard(&mut f, none, pc);
                    let ret = f.ins().iadd_imm(ret, -1);
                    f.def_var(v.ret, ret);
                    let p = {
                        let a = slot(&mut f, ret_c, ret);
                        f.ins().uload32(flags, a, 0)
                    };
                    // A compare tree over the places `PUSH` names.
                    let t: Vec<(u32, Block)> = pushes.iter().map(|&p| (u32::from(p), word(p + u16::from(again)))).collect();
                    if t.is_empty() {
                        f.ins().jump(off_end, &[]);
                    } else {
                        dispatch(&mut f, p, &t);
                    }
                }
                Op::Accept => {
                    // No bit of the mask is left after the last one taken.
                    let (w, bits) = (f.use_var(v.lw), f.use_var(v.taken));
                    let (_, scan) = mask_scan(&mut f, Mask { first: lasts_p, stride: lasts_stride, words: nlasts }, w, bits, refuse, pc, false);
                    heads.push((scan, Step::Block));
                    let (at_end, open, ret) = (f.use_var(v.at_end), f.use_var(v.open), f.use_var(v.ret));
                    let not_end = f.ins().icmp_imm(IntCC::Equal, at_end, 0);
                    let not_one = f.ins().icmp_imm(IntCC::NotEqual, open, 1);
                    let returns = f.ins().icmp_imm(IntCC::NotEqual, ret, 0);
                    let bad = f.ins().bor(not_end, not_one);
                    let bad = f.ins().bor(bad, returns);
                    guard(&mut f, bad, pc);
                    let root = f.ins().uload32(flags, open_c, 0);
                    put(&mut f, end_c, root, len);
                    let nodes = f.use_var(v.nodes);
                    let size = f.ins().isub(nodes, root);
                    put(&mut f, size_c, root, size);
                    f.ins().store(flags, nodes, counts_c, 0);
                    let supers = f.use_var(v.supers);
                    f.ins().store(flags, supers, counts_c, 8);
                    f.ins().return_(&[zero]);
                }
                Op::Empty { total } => {
                    let t = f.ins().load(types::I64, flags, totals_p, 8 * i32::from(total));
                    let some = f.ins().icmp_imm(IntCC::NotEqual, t, 0);
                    guard(&mut f, some, pc);
                    f.ins().jump(fall, &[]);
                }
                Op::Goto { target } => {
                    f.ins().jump(word(target), &[]);
                }
                Op::Stop => {
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                }
                _ => return Err(Error::UnsupportedBackend),
            }
        }
        f.switch_to_block(off_end);
        let past = f.ins().iconst(types::I64, words.len() as i64 + 1);
        f.ins().jump(refuse, &[BlockArg::from(past)]);
        f.switch_to_block(refuse);
        let pc = f.block_params(refuse)[0];
        f.ins().return_(&[pc]);
        f.seal_all_blocks();
        f.finalize();
        shape = super::native_cpu_shape::check(&context.func, &heads);
    }
    shape.map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    let clif = context.func.display().to_string();
    super::native_cpu_codegen::define_function(&mut module, id, &mut context).map_err(|_| Error::BackendFailure)?;
    let disassembly = context.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default();
    module.clear_context(&mut context);
    module.finalize_definitions().map_err(|_| Error::BackendFailure)?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_stream` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };
    let heads = heads.into_iter().map(|(b, s)| (b.to_string(), s)).collect();
    let marks = words.iter().filter_map(|b| if let Op::Mark { order, .. } = b.operation { Some(u64::from(order) + 1) } else { None }).max().unwrap_or(0);
    Ok(CompiledStream { _module: module, entry, bounds: bounds(native), starts, lasts, marks, clif, disassembly, heads })
}

/// A mask's words for `CompiledStream::call`: word `i` is `words[i * stride]`, `count` of them.
pub(super) struct MaskWords<'a> {
    pub(super) words: &'a [u64],
    pub(super) stride: usize,
    pub(super) count: usize,
}

/// The buffers one call writes, sized from the stream and the program's `Bounds`.
pub(super) struct Tape {
    pub(super) kind: Vec<u32>,
    pub(super) field: Vec<u32>,
    pub(super) start: Vec<u32>,
    pub(super) end: Vec<u32>,
    pub(super) size: Vec<u32>,
    pub(super) super_node: Vec<u32>,
    pub(super) super_kind: Vec<u32>,
    open: Vec<u32>,
    returns: Vec<u32>,
    pub(super) counts: [u64; 2],
}

impl CompiledStream {
    /// Room for a stream of `n` starts.
    pub(super) fn tape(&self, n: usize) -> Tape {
        // A token is switched on once, and once more after each mark's arm.
        let paths = (n as u64 + 2) * (1 + self.marks);
        let nodes = (self.bounds.nodes * paths + 1) as usize;
        let supers = (self.bounds.supers * paths + 1) as usize;
        let pushes = (self.bounds.pushes * paths + 1) as usize;
        Tape {
            kind: vec![0; nodes],
            field: vec![0; nodes],
            start: vec![0; nodes],
            end: vec![0; nodes],
            size: vec![0; nodes],
            super_node: vec![0; supers],
            super_kind: vec![0; supers],
            open: vec![0; nodes],
            returns: vec![0; pushes],
            counts: [0; 2],
        }
    }

    /// One call over a file. `padded` is the file with one byte before it and 64 zero bytes after
    /// (`len` its length); `starts` has two readable slots past its count; `lasts` holds the
    /// mask's words, a word every `stride`.
    ///
    /// # Safety
    /// The slices must be as stated, and `tape` from `self.tape(nstarts)`.
    pub(super) unsafe fn call(&self, padded: *const u8, len: usize, starts: (&[u32], usize), lasts: MaskWords<'_>, outputs: *const u64, totals: &[u64], tape: &mut Tape) -> u64 {
        assert!(lasts.count == 0 || (lasts.count - 1) * lasts.stride < lasts.words.len(), "the mask's words lie in its slice");
        let ctx = Ctx {
            bytes: padded,
            len: len as u64,
            starts: starts.0.as_ptr(),
            nstarts: starts.1 as u64,
            lasts: lasts.words.as_ptr(),
            nlasts: lasts.count as u64,
            totals: totals.as_ptr(),
            kind: tape.kind.as_mut_ptr(),
            field: tape.field.as_mut_ptr(),
            start: tape.start.as_mut_ptr(),
            end: tape.end.as_mut_ptr(),
            size: tape.size.as_mut_ptr(),
            super_node: tape.super_node.as_mut_ptr(),
            super_kind: tape.super_kind.as_mut_ptr(),
            open: tape.open.as_mut_ptr(),
            returns: tape.returns.as_mut_ptr(),
            counts: tape.counts.as_mut_ptr(),
            lasts_stride: 8 * lasts.stride as u64,
            outputs,
        };
        // SAFETY: the caller's contract; the function writes only the tape's buffers.
        unsafe { (self.entry)(&ctx) }
    }

    /// The words over tier 1's run of `bytes`, as the interpreter's `stream::run` gives them.
    pub(super) fn run(&self, bytes: &[u8], tier1: &super::block::BlockRun) -> Result<Vec<Node>, Refused> {
        let mut padded = vec![0u8; bytes.len() + 65];
        padded[1..=bytes.len()].copy_from_slice(bytes);
        let stream = |e: Option<usize>| -> (Vec<u32>, usize) {
            let mut v = e.map_or_else(Vec::new, |e| tier1.positions[e].clone());
            let n = v.len();
            v.extend([0, 0]);
            (v, n)
        };
        let starts = stream(self.starts);
        // The outputs interleaved, as a batch holds them: output `o`'s word `w` at `o + w * n`.
        let n = tier1.outputs.len().max(1);
        let count = tier1.outputs.iter().map(Vec::len).max().unwrap_or(0);
        let mut all = vec![0u64; n * count.max(1)];
        for (o, words) in tier1.outputs.iter().enumerate() {
            for (w, &x) in words.iter().enumerate() {
                all[o + w * n] = x;
            }
        }
        let (words, count) = match self.lasts {
            Some(m) => (&all[m..], tier1.outputs[m].len()),
            None => (&all[..0], 0),
        };
        let mut tape = self.tape(starts.1);
        let totals = if tier1.totals.is_empty() { vec![0] } else { tier1.totals.clone() };
        // SAFETY: `padded` has the byte before and 64 after; each stream two slots past its count;
        // the tape is sized for the stream; every output has `count` words in `all`.
        let r = unsafe { self.call(padded.as_ptr().wrapping_add(1), bytes.len(), (&starts.0, starts.1), MaskWords { words, stride: n, count }, all.as_ptr(), &totals, &mut tape) };
        if r != 0 {
            return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse" });
        }
        Ok(tape.nodes())
    }
}

impl Tape {
    /// The node table as `stream::Node`s.
    pub(super) fn nodes(&self) -> Vec<Node> {
        let n = self.counts[0] as usize;
        let mut out: Vec<Node> = (0..n)
            .map(|i| Node { kind: self.kind[i] as u16, field: self.field[i] as u16, supers: Vec::new(), start: self.start[i], end: self.end[i], size: self.size[i] })
            .collect();
        for s in 0..self.counts[1] as usize {
            out[self.super_node[s] as usize].supers.push(self.super_kind[s] as u16);
        }
        out
    }
}

/// A language's whole compiled path: tier 1's function, then tier 2's over the buffers tier 1
/// wrote. Between and around the two calls the host only lays out the file (padded, as simdjson
/// requires) and reads the node table back.
pub(super) struct Parser {
    pub(super) tier1: super::native_cpu_block::CompiledBlock,
    pub(super) tier2: CompiledStream,
}

impl Parser {
    pub(super) fn new(native: &NativeModule) -> Result<Self, Error> {
        let block = native.block_programs.first().ok_or(Error::UnsupportedBackend)?;
        Ok(Parser { tier1: super::native_cpu_block::compile(block)?, tier2: compile(native)? })
    }

    /// The nodes of `bytes`, or the word that refused it (the file goes to tree-sitter).
    pub(super) fn parse(&self, bytes: &[u8]) -> Result<Vec<Node>, Refused> {
        let mut batch = super::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let mut tape = self.tape(&batch);
        self.words(&batch, &mut tape)?;
        Ok(tape.nodes())
    }

    /// Tier 1's stream `e` in `batch` (its first file), as tier 1 wrote it: its positions (room
    /// for two past its count) and count.
    fn stream<'b>(&self, batch: &'b super::native_cpu_block::Batch, e: Option<usize>) -> (&'b [u32], usize) {
        let row = batch.rows[0];
        match e {
            Some(e) => {
                let at = (row.positions + e as u64 * row.capacity) as usize;
                (&batch.positions[at..at + row.capacity as usize], batch.counts[row.counts as usize + e] as usize)
            }
            None => (&batch.positions[..0], 0),
        }
    }

    /// A tape for the stream tier 1 wrote in `batch`.
    pub(super) fn tape(&self, batch: &super::native_cpu_block::Batch) -> Tape {
        self.tier2.tape(self.stream(batch, self.tier2.starts).1)
    }

    /// Tier 2's function over tier 1's buffers in `batch` (one file), into `tape`; or the word
    /// that refused the file.
    pub(super) fn words(&self, batch: &super::native_cpu_block::Batch, tape: &mut Tape) -> Result<(), Refused> {
        let row = batch.rows[0];
        let extracts = self.tier1.extracts.len();
        let starts = self.stream(batch, self.tier2.starts);
        let lasts = match self.tier2.lasts {
            Some(m) => {
                MaskWords { words: &batch.out[row.out as usize + m..], stride: self.tier1.outputs, count: row.blocks as usize }
            }
            None => MaskWords { words: &[], stride: 1, count: 0 },
        };
        let none = [0u64];
        let totals = match batch.counts.get(row.counts as usize + extracts..) {
            Some(t) if !t.is_empty() => t,
            _ => &none[..],
        };
        debug_assert!(starts.0.len() >= starts.1 + 2 || self.tier2.starts.is_none());
        // SAFETY: the file's bytes in the batch have `FRONT` zero bytes before them and 64 after
        // its blocks; each stream has `capacity` slots, at least two past its count; the tape is
        // sized for the stream.
        let outputs = batch.out[row.out as usize..].as_ptr();
        let r = unsafe { self.tier2.call(batch.bytes.as_ptr().wrapping_add(row.bytes as usize), row.len as usize, starts, lasts, outputs, totals, tape) };
        if r != 0 {
            return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse" });
        }
        Ok(())
    }
}
