//! Tier 2 on the CPU JIT: Cranelift emits a program's words (`native_ir::StructuralOperation`'s
//! tier-2 words) as one function `tempo_stream(ctx)`. Each word is one block of code made from
//! the word and its operand alone, and its tables (a switch's targets, a string's bytes). Nothing in it is fixed for a language or a tape format.
//!
//! The machine's state (`stream::run`, the reference) is in SSA variables where every token
//! uses it: the stream's cursors, the token's start, end and limit, the end of the token
//! before, the node count, and the open and return stacks' depths. What few words touch (the
//! supertype count, pending extras, wraps, dropped records, a shared arm's target) is in a
//! stack slot (`Mem`): an SSA variable is carried through every block where paths meet. Its output is the node table, one row per node
//! in preorder (kind, field, start, end, size), and the supertypes, one row each (node, kind).
//!
//! The loops are the stream's, as the verifier requires: every cycle passes a `TOKEN`.
//! `native_cpu_shape` refuses any other loop, and any call.
//!
//! Room is sized from the program, not checked per node: between two `TOKEN`s the words write at
//! most `per_token` nodes, supertypes and returns (the longest path, `Bounds`), so a stream of
//! `n` starts needs `per_token * (n + 2)`.

use crate::Error;

use crate::native_cpu_shape::Step;
use crate::native_ir::{FlagOp, On, Structure, StructuralOperation as Op};
use crate::stream::{ByteSwitch, DEAD, GONE, KEEP, Node, REFUSE, REMEMBERED, Refused};
use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, Block, BlockArg, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

/// The function's one argument: where everything is. Each field is 8 bytes.
#[repr(C)]
pub struct Ctx {
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
    /// Only a program with `WRAP` uses these (docs/parser-generators/24-rr-and-bitlex-plan.md
    /// §2.5). Per open node, by depth: its subtree's base, and the last node made at its level
    /// (one more than its record; 0: none) with that node's base.
    frame_base: *mut u32,
    frame_last: *mut u32,
    frame_last_base: *mut u32,
    /// Per record: the latest wrap on this base, one more than its record (0: none; `u32::MAX`:
    /// the record is a wrap); per wrap, the wrap before it on its base, so; each record's place.
    head: *mut u32,
    link: *mut u32,
    place: *mut u32,
    /// The node table in preorder, written at `ACCEPT` when the file had a wrap.
    out_kind: *mut u32,
    out_field: *mut u32,
    out_start: *mut u32,
    out_end: *mut u32,
    out_size: *mut u32,
    /// The table's last columns, made by its final pass (`table`): each node's parent
    /// (`NO_PARENT` for the root) and whether it is an extra.
    parent: *mut u32,
    extra: *mut u8,
    /// Tier 1's layout: the bytes from one output's words to the next's (8 on the CPU, where a
    /// block's outputs are together; a file's blocks times 8 on Metal, an output's words
    /// together); the bytes from one total to the next, and the mask of a total's bits (64-bit
    /// totals on the CPU, 32-bit on Metal).
    output_stride: u64,
    totals_stride: u64,
    totals_mask: u64,
    /// The tape's room: its node, supertype and return slots (`Tape`'s lengths). A token word
    /// stops the run (`OVERFLOW`) when the path it starts could pass them.
    room_nodes: u64,
    room_supers: u64,
    room_pushes: u64,
    /// In clusters (`compile_cut`), the run's state (`Mem`): every cluster reaches it, and
    /// `ctx`, through the one pinned register.
    state: [u64; Mem::COUNT as usize],
}

/// The run stopped for the tape's room: the host runs it again with the bound's room.
pub const OVERFLOW: u64 = u64::MAX;

/// Runs again on the bound's tape (`CompiledStream::call`), for the profile.
pub static ROOM_RETRIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
const FRAME_BASE: i32 = 152;
const FRAME_LAST: i32 = 160;
const FRAME_LAST_BASE: i32 = 168;
const HEAD: i32 = 176;
const LINK: i32 = 184;
const PLACE: i32 = 192;
const OUT_KIND: i32 = 200;
const OUT_FIELD: i32 = 208;
const OUT_START: i32 = 216;
const OUT_END: i32 = 224;
const OUT_SIZE: i32 = 232;
const PARENT: i32 = 240;
const EXTRA: i32 = 248;
const OUTPUT_STRIDE: i32 = 256;
const TOTALS_STRIDE: i32 = 264;
const TOTALS_MASK: i32 = 272;
const ROOM_NODES: i32 = 280;
const ROOM_SUPERS: i32 = 288;
const ROOM_PUSHES: i32 = 296;
const STATE: i32 = 304;

/// The root's parent in the table's `parent` column.
pub const NO_PARENT: u32 = u32::MAX;
/// `head`'s mark of a wrap's own record.
const IS_WRAP: i64 = u32::MAX as i64;

/// Returns 0 when the words accept, else one more than the refusing word's place.
type Entry = unsafe extern "C" fn(*const Ctx) -> u64;

/// The most a program writes between two `TOKEN`s: nodes, supertypes and returns pushed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub nodes: u64,
    pub supers: u64,
    pub pushes: u64,
    /// Returns chain: a cycle passes a `RETURN` and no `TOKEN`.
    pub chains: bool,
    /// Where returns chain, the most a call popped writes: a token's path writes at most the
    /// bound above and this for each call still open (the bound is per token over a file, the
    /// pops paid by the pushes; a path alone pops the calls open when it starts).
    pub pop_nodes: u64,
    pub pop_supers: u64,
}

/// A compiled tier-2 program.
pub struct CompiledStream {
    _module: JITModule,
    entry: Entry,
    bounds: Bounds,
    /// The extract `TOKEN` reads, and the output of the mask `SPAN m=1` reads.
    pub starts: Option<usize>,
    pub lasts: Option<usize>,
    /// The marks `MARK` tests: a token can be switched on once per mark and once more.
    marks: u64,
    /// The program has `WRAP`: the tape has the frames, chains and preorder columns.
    pub wraps: bool,
    /// The program has context beside each return (`has_context`): two return slots a call.
    context: bool,
    pub clif: String,
    pub disassembly: String,
    /// The CLIF blocks of the loops' heads, with their steps (`native_cpu_shape`): the token
    /// loop's function's.
    pub heads: Vec<(String, Step)>,
    /// Its calls, each of an outlined function made from the same program (`outline`): the
    /// only calls the machine code may have.
    pub own_calls: usize,
    /// Its functions of clusters (`compile_cut`: one, the token loop's alone), and their tail
    /// calls of each other: one where a jump left its cluster.
    pub clusters: usize,
    pub crossings: usize,
    /// Each word's Cranelift instructions as lowered (a shared run's body counted at its first
    /// place), for the profile: times how often each word runs, where tier 2's work goes.
    pub word_insts: Vec<u32>,
}

/// Each word's successors, as `stream::validate` reads them, `TOKEN`'s cut.
pub(crate) fn successors(native: &Structure, pushes: &[u16], pc: usize) -> Vec<usize> {
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
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } => vec![pc + 1, pc + 2],
        _ => vec![pc + 1],
    }
    .into_iter()
    .filter(|&n| n < native.structural.len())
    .collect()
}

/// The program's `Bounds`: the most written between two `TOKEN`s, weighting `OPEN`, `LEAF` and
/// `WRAP` (a node), `NODE_SUPER` and `PUSH`.
///
/// With `TOKEN`'s edges cut, the graph is acyclic unless returns chain (a level ends, and the
/// one it returns to ends on the same token). Then it is the longest path from the entry or a
/// `TOKEN` to the next `TOKEN`, each word's found once. Where returns chain, `RETURN`'s edges are
/// cut too (the verifier's rule: then the graph is acyclic), and the words between two tokens
/// are a segment from the entry or a `TOKEN`, segments from a return's landing to a `RETURN`,
/// one per call popped, and a last segment. A file pops at most the calls it makes, so the
/// chain's segments are counted once per call: the bound is per token, as the tape sizes it.
pub fn bounds(native: &Structure) -> Bounds {
    let n = native.structural.len();
    let pushes = push_targets(native);
    let token = |pc: usize| matches!(native.structural[pc].operation, Op::Token { .. });
    let weight = |pc: usize| match native.structural[pc].operation {
        Op::Open { .. } | Op::Leaf { .. } | Op::Wrap { .. } => Bounds { nodes: 1, ..Bounds::default() },
        Op::NodeSuper { .. } => Bounds { supers: 1, ..Bounds::default() },
        Op::Push { .. } => Bounds { pushes: 1, ..Bounds::default() },
        _ => Bounds::default(),
    };
    let max = |a: Bounds, b: Bounds| Bounds { nodes: a.nodes.max(b.nodes), supers: a.supers.max(b.supers), pushes: a.pushes.max(b.pushes), ..Bounds::default() };
    let add = |a: Bounds, b: Bounds| Bounds { nodes: a.nodes + b.nodes, supers: a.supers + b.supers, pushes: a.pushes + b.pushes, ..Bounds::default() };
    let cut = |pc: usize, cut_return: bool| -> Vec<usize> {
        if cut_return && matches!(native.structural[pc].operation, Op::Return { .. }) {
            return vec![];
        }
        successors(native, &pushes, pc).into_iter().filter(|&s| !token(s)).collect()
    };
    // Whether the graph with `TOKEN`'s edges cut has a cycle: 1 on the path, 2 done.
    let chains = {
        let mut mark = vec![0u8; n];
        let mut cyclic = false;
        for root in 0..n {
            if mark[root] != 0 || cyclic {
                continue;
            }
            let mut stack: Vec<(usize, Vec<usize>)> = vec![(root, cut(root, false))];
            mark[root] = 1;
            while let Some((pc, next)) = stack.last_mut() {
                match next.pop() {
                    Some(q) if mark[q] == 1 => {
                        cyclic = true;
                        break;
                    }
                    Some(q) if mark[q] == 0 => {
                        mark[q] = 1;
                        let s = cut(q, false);
                        stack.push((q, s));
                    }
                    Some(_) => {}
                    None => {
                        mark[*pc] = 2;
                        stack.pop();
                    }
                }
            }
        }
        cyclic
    };
    // Each word's longest path on (`.0`), and its longest on to a `RETURN` (`.1`), the graph
    // acyclic: memoized, depth first, without recursion.
    let longest = |cut_return: bool| -> Vec<(Bounds, Option<Bounds>)> {
        let mut memo: Vec<Option<(Bounds, Option<Bounds>)>> = vec![None; n];
        for root in 0..n {
            let mut stack = vec![(root, false)];
            while let Some((pc, expanded)) = stack.pop() {
                if memo[pc].is_some() {
                    continue;
                }
                let next = cut(pc, cut_return);
                if !expanded {
                    stack.push((pc, true));
                    stack.extend(next.iter().filter(|&&q| memo[q].is_none()).map(|&q| (q, false)));
                    continue;
                }
                let own = weight(pc);
                let is_return = matches!(native.structural[pc].operation, Op::Return { .. });
                let (mut any, mut to_return) = (Bounds::default(), is_return.then(Bounds::default));
                for q in next {
                    let (a, r) = memo[q].expect("a successor first");
                    any = max(any, a);
                    to_return = match (to_return, r) {
                        (Some(x), Some(y)) => Some(max(x, y)),
                        (x, y) => x.or(y),
                    };
                }
                memo[pc] = Some((add(own, any), to_return.map(|r| add(own, r))));
            }
        }
        memo.into_iter().map(|m| m.expect("every word")).collect()
    };
    // From the entry, and from the word after each `TOKEN` (whose own edges are cut).
    let froms: Vec<usize> = (0..n).filter(|&pc| pc == 0 && !token(pc)).chain((0..n).filter(|&pc| token(pc) && pc + 1 < n).map(|pc| pc + 1)).filter(|&pc| !token(pc)).collect();
    let paths = longest(chains);
    let over = |at: &[usize], f: &dyn Fn(&(Bounds, Option<Bounds>)) -> Bounds| at.iter().fold(Bounds::default(), |b, &pc| max(b, f(&paths[pc])));
    let first = over(&froms, &|p| p.0);
    if !chains {
        return first;
    }
    // Where a return lands: a state's `SWITCH` (`m=1`; `m=0` lands on its `TOKEN`).
    let landings: Vec<usize> = pushes.iter().map(|&p| usize::from(p) + 1).filter(|&pc| pc < n && !token(pc)).collect();
    let last = over(&landings, &|p| p.0);
    let chain = over(&landings, &|p| p.1.unwrap_or_default());
    let calls = first.pushes + last.pushes;
    let times = |x: u64, k: u64| x * k;
    Bounds { nodes: first.nodes + last.nodes + times(chain.nodes, calls), supers: first.supers + last.supers + times(chain.supers, calls), pushes: calls, chains: true, pop_nodes: chain.nodes, pop_supers: chain.supers }
}

/// The places `PUSH` names, sorted: where a `RETURN` can go.
pub fn push_targets(native: &Structure) -> Vec<u16> {
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
/// Tier 2's state, each a slot of the function's stack frame: a word loads what it reads and
/// stores what it writes. Not SSA variables, which every block where paths meet carries (as
/// block parameters, moved and spilled at each join): the state in memory is a fifth less code
/// and no slower (docs/parser-generators/27-tier-2-table-spike.md).
#[derive(Clone, Copy)]
enum Mem {
    /// The supertypes written.
    Supers,
    /// The deferred extras' leaves at the table's end.
    Pending,
    /// While leaves are pending, the end of the token before them: a node closed then ends there.
    Keep,
    /// The wraps made, and the node the last one moved.
    Wraps,
    Moved,
    /// The records a reading dropped (`DEAD`).
    Dead,
    /// A shared arm's target, as an index into its body's (`shared_runs`).
    Sel,
    /// A shared arm's words' distance from its body's first place's, for a refusal there.
    Shift,
    /// The level's word (`REMEMBER`) and flags (`FLAG`), kept beside each call's return.
    Word,
    Flags,
    /// The next token's index into the extract.
    Next,
    /// The mask's word `lw`'s bits not yet taken; before the first word, `lw` is -1 (no bits).
    Taken,
    Lw,
    /// The token's bytes, and the next token's start (or the file's end).
    Start,
    End,
    Limit,
    /// The end of the token before.
    LastEnd,
    /// 1 at the stream's end.
    AtEnd,
    /// The nodes made, the nodes open, the calls made and not returned.
    Nodes,
    Open,
    Ret,
    /// 1 once the token's bytes are taken (`SPAN`): `TOKEN` then takes the next.
    Need,
    /// The marks not yet taken or passed at this token, a bit each.
    Live,
}

impl Mem {
    const COUNT: u32 = 23;
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

/// `dispatch` weighted by a profile: a run with half the weight or more is tested first (one
/// range compare), else the runs split at their weighted middle. `ends` are the runs' ends
/// (the next run's first byte; the last's, past every byte).
fn dispatch_weighted(f: &mut FunctionBuilder<'_>, x: Value, runs: &[(u32, u32, Block, u64)]) {
    if runs.len() == 1 {
        f.ins().jump(runs[0].2, &[]);
        return;
    }
    let total: u64 = runs.iter().map(|r| r.3).sum();
    let (top, &(lo, hi, b, w)) = runs.iter().enumerate().max_by_key(|(i, r)| (r.3, std::cmp::Reverse(*i))).expect("runs");
    if total > 0 && 2 * w >= total {
        // In the run: lo <= x < hi, as one unsigned compare of x - lo.
        let rest = f.create_block();
        let inside = if top == 0 {
            f.ins().icmp_imm(IntCC::UnsignedLessThan, x, i64::from(hi))
        } else if top == runs.len() - 1 {
            f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, x, i64::from(lo))
        } else {
            let d = f.ins().iadd_imm(x, -i64::from(lo));
            f.ins().icmp_imm(IntCC::UnsignedLessThan, d, i64::from(hi - lo))
        };
        f.ins().brif(inside, b, &[], rest, &[]);
        f.switch_to_block(rest);
        // Without the run: x is in no byte of it, so its neighbours' ranges may cover it.
        let others: Vec<_> = runs.iter().enumerate().filter(|&(i, _)| i != top).map(|(_, r)| *r).collect();
        dispatch_weighted(f, x, &others);
        return;
    }
    // The split nearest the weighted middle (by count when the profile saw none).
    let weight = |r: &(u32, u32, Block, u64)| if total > 0 { r.3 } else { 1 };
    let all: u64 = runs.iter().map(weight).sum();
    let (mut left, mut mid, mut best) = (0u64, 1usize, u64::MAX);
    for (i, r) in runs.iter().enumerate().take(runs.len() - 1) {
        left += weight(r);
        let gap = left.abs_diff(all - left);
        if gap < best {
            (best, mid) = (gap, i + 1);
        }
    }
    let (low, high) = (f.create_block(), f.create_block());
    let below = f.ins().icmp_imm(IntCC::UnsignedLessThan, x, i64::from(runs[mid].0));
    f.ins().brif(below, low, &[], high, &[]);
    f.switch_to_block(low);
    dispatch_weighted(f, x, &runs[..mid]);
    f.switch_to_block(high);
    dispatch_weighted(f, x, &runs[mid..]);
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

/// `SPAN m=1`'s search, a function of its own (`outline`): the next of the last-byte mask's
/// bits after the last one taken, the bits of word `lw` not yet taken (`taken`), then a word
/// (64 bytes) a turn. Parameters: the mask (its first word, stride, word count), `lw`, `taken`,
/// the token's start and limit. Returns the token's end (the bit's byte, plus one), the bit's
/// word and its bits left, and 1 when it refuses: no bit is left, or the bit is before the
/// token's start or its end past its limit.
fn span_last(f: &mut FunctionBuilder<'_>, p: &[Value], heads: &mut Vec<(Block, Step)>) {
    let (first, stride, words, w, bits, start, limit) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
    let refuse = f.create_block();
    f.append_block_param(refuse, types::I64);
    let (found, scan) = mask_scan(f, Mask { first, stride, words }, w, bits, refuse, 0, true);
    heads.push((scan, Step::Block));
    let (w, bits) = (f.block_params(found)[0], f.block_params(found)[1]);
    let tz = f.ins().ctz(bits);
    let at = f.ins().ishl_imm(w, 6);
    let l = f.ins().iadd(at, tz);
    let less = f.ins().iadd_imm(bits, -1);
    let rest = f.ins().band(bits, less);
    let before = f.ins().icmp(IntCC::UnsignedLessThan, l, start);
    let e = f.ins().iadd_imm(l, 1);
    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, e, limit);
    let bad = f.ins().bor(before, past);
    let bad = f.ins().uextend(types::I64, bad);
    f.ins().return_(&[e, w, rest, bad]);
    f.switch_to_block(refuse);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().return_(&[start, p[3], p[4], one]);
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
/// The columns `reorder` reads and writes.
struct Reorder {
    nodes: Value,
    supers: Value,
    wraps: Value,
    /// Kind, field, start, end, size: as made, and in preorder.
    cols: [Value; 5],
    out: [Value; 5],
    head: Value,
    link: Value,
    place: Value,
    snode: Value,
    /// A program that rereads: a column of `nodes + 1` slots (the frames', free at `ACCEPT`)
    /// for the dropped records before each, a dropped record (`DEAD`) left out.
    before: Option<Value>,
    /// Where the count of records placed goes.
    counts: Value,
}

impl Reorder {
    /// Its values, as an outlined function's parameters (`outline`).
    fn values(&self) -> Vec<Value> {
        [self.nodes, self.supers, self.wraps].into_iter().chain(self.cols).chain(self.out).chain([self.head, self.link, self.place, self.snode, self.counts]).chain(self.before).collect()
    }

    /// The same, from the parameters `p` of its function.
    fn of(p: &[Value], before: bool) -> Self {
        let col = |k: usize| [p[k], p[k + 1], p[k + 2], p[k + 3], p[k + 4]];
        Reorder { nodes: p[0], supers: p[1], wraps: p[2], cols: col(3), out: col(8), head: p[13], link: p[14], place: p[15], snode: p[16], counts: p[17], before: before.then(|| p[18]) }
    }
}

/// A function of tier 2's own, made from the same program and called once per file: `build`
/// makes its body from its `params` words and gives its heads. Only the token loop is inlined;
/// the node table's passes run once, at `ACCEPT`, and would only crowd it. Checked: it calls
/// nothing, and each loop is a stated one.
fn outline(module: &mut JITModule, name: &str, params: usize, returns: usize, build: impl FnOnce(&mut FunctionBuilder<'_>, &[Value], &mut Vec<(Block, Step)>)) -> Result<(cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
    let mut signature = module.make_signature();
    for _ in 0..params {
        signature.params.push(AbiParam::new(types::I64));
    }
    for _ in 0..returns {
        signature.returns.push(AbiParam::new(types::I64));
    }
    let id = module.declare_function(name, Linkage::Local, &signature).map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2's own function {name}: {e:?}");
        }
        Error::BackendFailure
    })?;
    let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), signature.clone());
    let mut fctx = FunctionBuilderContext::new();
    let mut heads = Vec::new();
    {
        let mut f = FunctionBuilder::new(&mut func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        f.switch_to_block(entry);
        let p = f.block_params(entry).to_vec();
        build(&mut f, &p, &mut heads);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::native_cpu_shape::check(&func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    Ok((id, signature, func))
}

/// A call of an outlined function (`outline`), from `f`.
fn call_own(f: &mut FunctionBuilder<'_>, id: cranelift_module::FuncId, signature: &cranelift_codegen::ir::Signature, args: &[Value]) -> Vec<Value> {
    let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
    let signature = f.import_signature(signature.clone());
    let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
    let call = f.ins().call(callee, args);
    f.inst_results(call).to_vec()
}

/// At `ACCEPT` with wraps: the node table in preorder. Each record in the order made, a wrap's
/// own skipped, and before each its base's chain of wraps, latest (outermost) first; then each
/// supertype's node at its place. A program that rereads leaves the dropped records out, each
/// node's size less those in its range (from its base, `size` records), counted first. Loops
/// over the table and one over the supertypes, each step a record (`Step::Node`). Writes the
/// count placed. An outlined function of its own (`outline`).
fn reorder(f: &mut FunctionBuilder<'_>, r: Reorder, heads: &mut Vec<(Block, Step)>) {
    let flags = MemFlags::trusted();
    let at = |f: &mut FunctionBuilder<'_>, c: Value, i: Value| {
        let off = f.ins().ishl_imm(i, 2);
        f.ins().iadd(c, off)
    };
    let get = |f: &mut FunctionBuilder<'_>, c: Value, i: Value| {
        let a = at(f, c, i);
        f.ins().uload32(flags, a, 0)
    };
    let put = |f: &mut FunctionBuilder<'_>, c: Value, i: Value, x: Value| {
        let a = at(f, c, i);
        f.ins().istore32(flags, x, a, 0);
    };
    // Record `from` to place `to`; its range starts at `first`.
    let copy = |f: &mut FunctionBuilder<'_>, from: Value, to: Value, first: Value| {
        for (k, (c, o)) in r.cols.iter().zip(r.out.iter()).enumerate() {
            let mut x = get(f, *c, from);
            if k == 4
                && let Some(b) = r.before
            {
                let e = f.ins().iadd(first, x);
                let (de, df) = (get(f, b, e), get(f, b, first));
                let gone = f.ins().isub(de, df);
                x = f.ins().isub(x, gone);
            }
            put(f, *o, to, x);
        }
        put(f, r.place, from, to);
    };
    // Whether record `n` was dropped (only a program that rereads drops one).
    let dropped = |f: &mut FunctionBuilder<'_>, n: Value| -> Option<Value> {
        r.before.map(|_| {
            let k = get(f, r.cols[0], n);
            f.ins().icmp_imm(IntCC::Equal, k, i64::from(DEAD))
        })
    };
    let (scan, chain, supers, done, plain) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    for (b, n) in [(scan, 2), (chain, 3), (supers, 1)] {
        for _ in 0..n {
            f.append_block_param(b, types::I64);
        }
    }
    heads.extend([(scan, Step::Node), (chain, Step::Node), (supers, Step::Node)]);
    let zero = f.ins().iconst(types::I64, 0);
    let start = f.create_block();
    f.ins().brif(r.wraps, start, &[], done, &[]);
    f.switch_to_block(start);
    if let Some(b) = r.before {
        // count(i, d): the dropped records before record i.
        let count = f.create_block();
        f.append_block_param(count, types::I64);
        f.append_block_param(count, types::I64);
        heads.push((count, Step::Node));
        f.ins().jump(count, &[BlockArg::from(zero), BlockArg::from(zero)]);
        f.switch_to_block(count);
        let (i, d) = (f.block_params(count)[0], f.block_params(count)[1]);
        put(f, b, i, d);
        let step = f.create_block();
        let end = f.ins().icmp(IntCC::Equal, i, r.nodes);
        f.ins().brif(end, scan, &[BlockArg::from(zero), BlockArg::from(zero)], step, &[]);
        f.switch_to_block(step);
        let k = get(f, r.cols[0], i);
        let is = f.ins().icmp_imm(IntCC::Equal, k, i64::from(DEAD));
        let is = f.ins().uextend(types::I64, is);
        let (i1, d1) = (f.ins().iadd_imm(i, 1), f.ins().iadd(d, is));
        f.ins().jump(count, &[BlockArg::from(i1), BlockArg::from(d1)]);
    } else {
        f.ins().jump(scan, &[BlockArg::from(zero), BlockArg::from(zero)]);
    }

    // scan(i, j): record i, the next place j.
    f.switch_to_block(scan);
    let (i, j) = (f.block_params(scan)[0], f.block_params(scan)[1]);
    let body = f.create_block();
    let end = f.ins().icmp(IntCC::Equal, i, r.nodes);
    let ended = f.create_block();
    f.ins().brif(end, ended, &[], body, &[]);
    f.switch_to_block(ended);
    f.ins().store(flags, j, r.counts, 0);
    f.ins().jump(supers, &[BlockArg::from(zero)]);
    f.switch_to_block(body);
    let h = get(f, r.head, i);
    let is_wrap = f.ins().icmp_imm(IntCC::Equal, h, IS_WRAP);
    f.ins().brif(is_wrap, plain, &[], chain, &[BlockArg::from(h), BlockArg::from(i), BlockArg::from(j)]);
    f.switch_to_block(plain);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().jump(scan, &[BlockArg::from(i1), BlockArg::from(j)]);

    // chain(c, i, j): the wraps before record i, from c (one more than a wrap's record; 0: i).
    f.switch_to_block(chain);
    let (c, i, j) = (f.block_params(chain)[0], f.block_params(chain)[1], f.block_params(chain)[2]);
    let (wrap, own) = (f.create_block(), f.create_block());
    f.ins().brif(c, wrap, &[], own, &[]);
    f.switch_to_block(wrap);
    let w = f.ins().iadd_imm(c, -1);
    let c2 = get(f, r.link, w);
    let j1 = match dropped(f, w) {
        Some(gone) => {
            let (put_it, after) = (f.create_block(), f.create_block());
            f.append_block_param(after, types::I64);
            f.ins().brif(gone, after, &[BlockArg::from(j)], put_it, &[]);
            f.switch_to_block(put_it);
            copy(f, w, j, i);
            let j1 = f.ins().iadd_imm(j, 1);
            f.ins().jump(after, &[BlockArg::from(j1)]);
            f.switch_to_block(after);
            f.block_params(after)[0]
        }
        None => {
            copy(f, w, j, i);
            f.ins().iadd_imm(j, 1)
        }
    };
    f.ins().jump(chain, &[BlockArg::from(c2), BlockArg::from(i), BlockArg::from(j1)]);
    f.switch_to_block(own);
    let i1 = f.ins().iadd_imm(i, 1);
    let j1 = match dropped(f, i) {
        Some(gone) => {
            let (put_it, after) = (f.create_block(), f.create_block());
            f.append_block_param(after, types::I64);
            f.ins().brif(gone, after, &[BlockArg::from(j)], put_it, &[]);
            f.switch_to_block(put_it);
            copy(f, i, j, i);
            let j1 = f.ins().iadd_imm(j, 1);
            f.ins().jump(after, &[BlockArg::from(j1)]);
            f.switch_to_block(after);
            f.block_params(after)[0]
        }
        None => {
            copy(f, i, j, i);
            f.ins().iadd_imm(j, 1)
        }
    };
    f.ins().jump(scan, &[BlockArg::from(i1), BlockArg::from(j1)]);

    // supers(s): supertype s's node, at its place.
    f.switch_to_block(supers);
    let s = f.block_params(supers)[0];
    let step = f.create_block();
    let end = f.ins().icmp(IntCC::Equal, s, r.supers);
    f.ins().brif(end, done, &[], step, &[]);
    f.switch_to_block(step);
    let n = get(f, r.snode, s);
    let p = get(f, r.place, n);
    put(f, r.snode, s, p);
    let s1 = f.ins().iadd_imm(s, 1);
    f.ins().jump(supers, &[BlockArg::from(s1)]);

    f.switch_to_block(done);
    f.ins().return_(&[]);
}

/// The table's final pass, once it is in preorder: each node's parent and its extra flag, into
/// the `parent` and `extra` columns, from the `kind` and `size` columns of `count` records.
/// The open nodes are a stack of records in `stack` (the open stack's room, free at `ACCEPT`):
/// before record `i`, each open node whose range ends at or before `i` is popped, and the top is
/// `i`'s parent. One loop over the records and one over the pops, each step a record
/// (`Step::Node`): a record is pushed once and popped at most once. Returns 0.
fn table(f: &mut FunctionBuilder<'_>, cols: [Value; 2], count: Value, stack: Value, out: [Value; 2], extras: &[u16], heads: &mut Vec<(Block, Step)>) {
    let flags = MemFlags::trusted();
    let [kind_c, size_c] = cols;
    let [parent_c, extra_c] = out;
    let at = |f: &mut FunctionBuilder<'_>, c: Value, i: Value, shift: i64| {
        let off = f.ins().ishl_imm(i, shift);
        f.ins().iadd(c, off)
    };
    let (next, pop, popped, place, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
    // next(i, d): record i, d open records on the stack.
    for b in [next, pop] {
        f.append_block_param(b, types::I64);
        f.append_block_param(b, types::I64);
    }
    heads.extend([(next, Step::Node), (pop, Step::Node)]);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(next, &[BlockArg::from(zero), BlockArg::from(zero)]);

    f.switch_to_block(next);
    let (i, d) = (f.block_params(next)[0], f.block_params(next)[1]);
    let end = f.ins().icmp(IntCC::Equal, i, count);
    let body = f.create_block();
    f.ins().brif(end, done, &[], body, &[]);
    f.switch_to_block(body);
    f.ins().jump(pop, &[BlockArg::from(i), BlockArg::from(d)]);

    // pop(i, d): the top's range ends at or before i: popped.
    f.switch_to_block(pop);
    let (i, d) = (f.block_params(pop)[0], f.block_params(pop)[1]);
    let has = f.create_block();
    f.ins().brif(d, has, &[], place, &[]);
    f.switch_to_block(has);
    let d1 = f.ins().iadd_imm(d, -1);
    let a = at(f, stack, d1, 2);
    let top = f.ins().uload32(flags, a, 0);
    let a = at(f, size_c, top, 2);
    let size = f.ins().uload32(flags, a, 0);
    let ends = f.ins().iadd(top, size);
    let closed = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, ends, i);
    f.ins().brif(closed, popped, &[], place, &[]);
    f.switch_to_block(popped);
    f.ins().jump(pop, &[BlockArg::from(i), BlockArg::from(d1)]);

    // place: the parent is the top (none on an empty stack); i is pushed.
    f.switch_to_block(place);
    let none = f.ins().iconst(types::I64, i64::from(NO_PARENT));
    let d1 = f.ins().iadd_imm(d, -1);
    let a = at(f, stack, d1, 2);
    let has = f.ins().icmp_imm(IntCC::NotEqual, d, 0);
    // An empty stack reads no slot: the address is the stack's own.
    let a = f.ins().select(has, a, stack);
    let top = f.ins().uload32(flags, a, 0);
    let parent = f.ins().select(has, top, none);
    let a = at(f, parent_c, i, 2);
    f.ins().istore32(flags, parent, a, 0);
    let a = at(f, kind_c, i, 2);
    let k = f.ins().uload32(flags, a, 0);
    let mut extra = f.ins().iconst(types::I8, 0);
    for &e in extras {
        let is = f.ins().icmp_imm(IntCC::Equal, k, i64::from(e));
        extra = f.ins().bor(extra, is);
    }
    let a = at(f, extra_c, i, 0);
    f.ins().store(flags, extra, a, 0);
    let a = at(f, stack, d, 2);
    f.ins().istore32(flags, i, a, 0);
    let (i1, d2) = (f.ins().iadd_imm(i, 1), f.ins().iadd_imm(d, 1));
    f.ins().jump(next, &[BlockArg::from(i1), BlockArg::from(d2)]);

    f.switch_to_block(done);
    f.ins().return_(&[zero]);
}

/// A body of shared arms (`shared_runs`): its words, from `start` (its first place) to the
/// `GOTO` that ends it, and the places its places go on to.
pub struct SharedBody {
    pub start: usize,
    pub end: usize,
    pub targets: Vec<u16>,
}

/// Runs of words lowered once for every place that has them (`shared_runs`).
pub struct SharedRuns {
    pub bodies: Vec<SharedBody>,
    /// Each place a run starts at: its body, and its target's index in the body's `targets`.
    pub at: std::collections::HashMap<usize, (usize, usize)>,
    /// Each body's last word: its `GOTO`, at the body's first place.
    pub ends: std::collections::HashMap<usize, usize>,
    /// The words of a run that is not its body's first place: never lowered.
    pub skipped: Vec<bool>,
}

/// A word that only goes on to the next, or refuses: an arm's span, keyword check and acts.
fn plain(op: &Op) -> bool {
    matches!(
        op,
        Op::Span { .. } | Op::Word { test: false, .. } | Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::NodeField { .. } | Op::NodeSuper { .. } | Op::Retag { .. } | Op::Wrap { .. } | Op::Merge { .. } | Op::Push { .. } | Op::Remember { .. } | Op::Flag { op: FlagOp::Set | FlagOp::Clear, .. }
    )
}

/// Arms that differ only in where they go on are one copy of code. A run is the plain words
/// (`plain`) before a `GOTO`, none of them but the first entered from elsewhere; runs with the
/// same words that go on to a `TOKEN` are one body, lowered at its first place. Each place enters the body with the
/// index of its `GOTO`'s target, and the body's end jumps by it (a jump table): an indirect
/// jump an arm, as simdjson's stage 2 takes one a token, for code the size of the distinct
/// arms instead of every arm's.
pub fn shared_runs(native: &Structure, pushes: &[u16]) -> SharedRuns {
    let words = &native.structural;
    let n = words.len();
    // Entered other than by falling from a plain word before it.
    let mut entered = vec![false; n + 1];
    entered[0] = true;
    for pc in 0..n {
        let mut next = successors(native, pushes, pc);
        if matches!(words[pc].operation, Op::Token { .. }) {
            next.push(pc + 1);
        }
        for t in next {
            if !(t == pc + 1 && plain(&words[pc].operation)) {
                entered[t] = true;
            }
        }
    }
    // Each run: its first place, its `GOTO`, the target, and its words.
    let mut occ: Vec<(usize, usize, u16, Vec<Op>)> = Vec::new();
    for g in 0..n {
        let Op::Goto { target } = words[g].operation else { continue };
        if entered[g] {
            continue;
        }
        let mut s = g;
        while s > 0 && plain(&words[s - 1].operation) && !entered[s] {
            s -= 1;
        }
        if s < g {
            occ.push((s, g, target, words[s..g].iter().map(|w| w.operation).collect()));
        }
    }
    // Runs alone: each its own body. Sharing makes a body's jump reach every place's target on
    // the graph (not at run time: a place's index picks its own), and the shape check reads the
    // graph: a cycle that passes no `TOKEN` there is refused. So runs share, and where a cycle
    // appears that a shared jump closes, the run whose jump goes where its own does not is
    // alone; again, until none is left.
    let chains = bounds(native).chains;
    let mut alone = vec![false; occ.len()];
    let goto_of: std::collections::HashMap<usize, usize> = occ.iter().enumerate().map(|(i, o)| (o.1, i)).collect();
    loop {
        let mut keys: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut body_of = vec![0usize; occ.len()];
        let mut targets: Vec<Vec<u16>> = Vec::new();
        for (i, (_, _, t, ops)) in occ.iter().enumerate() {
            let key = if alone[i] { format!("{ops:?} to {t}") } else { format!("{ops:?}") };
            let b = *keys.entry(key).or_insert_with(|| {
                targets.push(Vec::new());
                targets.len() - 1
            });
            if !targets[b].contains(t) {
                targets[b].push(*t);
            }
            body_of[i] = b;
        }
        // The graph: a word's ways on (`TOKEN` and, where returns chain, `RETURN` cut: the
        // shape check's loop heads), a run's `GOTO` to each of its body's targets.
        let next = |pc: usize| -> Vec<usize> {
            if let Some(&i) = goto_of.get(&pc) {
                return targets[body_of[i]].iter().map(|&t| usize::from(t)).filter(|&t| t < n).collect();
            }
            if chains && matches!(words[pc].operation, Op::Return { .. }) {
                return Vec::new();
            }
            successors(native, pushes, pc)
        };
        // A depth-first search for a cycle: 1 on the path, 2 done.
        let mut mark = vec![0u8; n];
        let mut found: Option<Vec<usize>> = None;
        'search: for root in 0..n {
            if mark[root] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, Vec<usize>)> = vec![(root, next(root))];
            mark[root] = 1;
            while let Some((pc, rest)) = stack.last_mut() {
                let pc = *pc;
                match rest.pop() {
                    Some(q) if mark[q] == 1 => {
                        let from = stack.iter().position(|(w, _)| *w == q).unwrap_or(0);
                        let mut cycle: Vec<usize> = stack[from..].iter().map(|(w, _)| *w).collect();
                        cycle.push(q);
                        found = Some(cycle);
                        break 'search;
                    }
                    Some(q) if mark[q] == 0 => {
                        mark[q] = 1;
                        let nq = next(q);
                        stack.push((q, nq));
                    }
                    Some(_) => {}
                    None => {
                        mark[pc] = 2;
                        stack.pop();
                    }
                }
            }
        }
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() && found.is_none() {
            let words_in: usize = occ.iter().map(|o| o.1 - o.0 + 1).sum();
            let kept: usize = (0..targets.len()).map(|b| occ.iter().enumerate().find(|(i, _)| body_of[*i] == b).map_or(0, |(_, o)| o.1 - o.0 + 1)).sum();
            eprintln!("shared runs: {} runs, {} alone, {} bodies; {words_in} words in runs, {kept} lowered", occ.len(), alone.iter().filter(|a| **a).count(), targets.len());
        }
        let Some(cycle) = found else {
            let mut out = SharedRuns { bodies: Vec::new(), at: std::collections::HashMap::new(), ends: std::collections::HashMap::new(), skipped: vec![false; n] };
            let mut first: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for (i, (s, g, t, _)) in occ.iter().enumerate() {
                let b = body_of[i];
                let body = *first.entry(b).or_insert_with(|| {
                    out.bodies.push(SharedBody { start: *s, end: *g, targets: targets[b].clone() });
                    out.ends.insert(*g, out.bodies.len() - 1);
                    out.bodies.len() - 1
                });
                let ti = out.bodies[body].targets.iter().position(|x| x == t).expect("a target of its body");
                out.at.insert(*s, (body, ti));
                if out.bodies[body].start != *s {
                    for k in s + 1..=*g {
                        out.skipped[k] = true;
                    }
                }
            }
            return out;
        };
        // The runs on the cycle whose jump goes on where their own `GOTO` does not: alone.
        let mut split = false;
        for w in cycle.windows(2) {
            if let Some(&i) = goto_of.get(&w[0])
                && usize::from(occ[i].2) != w[1]
            {
                alone[i] = true;
                split = true;
            }
        }
        assert!(split, "a cycle on the words' own graph: the shape check refuses the program");
    }
}

pub fn compile(native: &Structure) -> Result<CompiledStream, Error> {
    let speed = crate::native_cpu_codegen::OPTIMIZE_TIER2.load(std::sync::atomic::Ordering::Relaxed);
    compile_cut(native, &crate::native_cpu_clusters::Cut { speed, ..crate::native_cpu_clusters::Cut::one(native.structural.len()) })
}

/// A cluster's function, lowered: its loop heads, the places it is entered at (each with its
/// block), its tail calls' blocks (each with the place it calls), and their number.
struct Lowered {
    func: cranelift_codegen::ir::Function,
    heads: Vec<(Block, Step)>,
    targets: Vec<(usize, Block)>,
    exits: std::collections::HashMap<Block, usize>,
    tails: usize,
}

/// Tier 2 lowered in the clusters of `cut` (`native_cpu_clusters`): one function a cluster.
/// One cluster is the one function `tempo_stream(ctx)`, its state in its stack frame. With
/// more, `tempo_stream` holds the state and calls the cluster of the first word; a cluster's
/// function takes `ctx` and the index of the place it enters at, `ctx` (and the state in it)
/// in the pinned register (x21: no cluster allocates it, so none spills it), and a jump
/// to a word of another cluster is a tail call of that cluster (`return_call`, the `tail`
/// calling convention): no stack grows, and every function returns the run's result to
/// `tempo_stream`.
pub fn compile_cut(native: &Structure, cut: &crate::native_cpu_clusters::Cut) -> Result<CompiledStream, Error> {
    if cut.of.len() != native.structural.len() || cut.count == 0 || (native.structural.is_empty() && cut.count > 1) {
        return Err(Error::UnsupportedBackend);
    }
    let words = &native.structural;
    let pushes = push_targets(native);
    let named = |op: fn(&Op) -> Option<u16>| words.iter().find_map(|b| op(&b.operation)).map(usize::from);
    let starts = named(|o| if let Op::Token { extract } = *o { Some(extract) } else { None });
    // The mask `SPAN m=1` names, as its output's index.
    let lasts = named(|o| if let Op::Span { last: true, len } = *o { Some(len) } else { None }).and_then(|m| native.block_programs.first().and_then(|b| b.masks.get(m).copied()));

    let clustered = cut.count > 1;
    // In clusters, the state's address is in the pinned register (x21 on aarch64): never
    // allocated, so never spilled, as the one function's stack frame is never moved.
    // `Cut::speed`: Cranelift's optimizer (`opt_level = speed`: value numbering, dead code,
    // redundant loads, invariants out of loops); by default it is off (`none`).
    let mut flags: Vec<(&str, &str)> = Vec::new();
    if clustered {
        flags.push(("enable_pinned_reg", "true"));
    }
    if cut.speed {
        flags.push(("opt_level", "speed"));
    }
    let mut builder = if cut.unsigned {
        // `Cut::unsigned`: return addresses not signed (Cranelift signs them on every macOS
        // target; an arm64 process, not arm64e, does not need it). The JIT's own flags, as
        // `JITBuilder::with_flags` sets them.
        use cranelift_codegen::settings::Configurable;
        let mut shared = cranelift_codegen::settings::builder();
        for (k, v) in flags.iter().copied().chain([("use_colocated_libcalls", "false"), ("is_pic", "false")]) {
            shared.set(k, v).map_err(|_| Error::BackendFailure)?;
        }
        let isa = cranelift_codegen::isa::lookup_by_name("aarch64-apple-darwin")
            .map_err(|_| Error::BackendFailure)?
            .finish(cranelift_codegen::settings::Flags::new(shared))
            .map_err(|_| Error::BackendFailure)?;
        JITBuilder::with_isa(isa, default_libcall_names())
    } else {
        JITBuilder::with_flags(&flags, default_libcall_names()).map_err(|_| Error::BackendFailure)?
    };
    if cut.near {
        // `Cut::near`: the module's code in one arena, well within a direct branch's 128 MB.
        builder.memory_provider(Box::new(cranelift_jit::ArenaMemoryProvider::new_with_size(64 << 20).map_err(|_| Error::BackendFailure)?));
    }
    let mut module = JITModule::new(builder);
    let pointer = module.target_config().pointer_type();
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(pointer));
    signature.returns.push(AbiParam::new(types::I64));
    let id = module.declare_function("tempo_stream", Linkage::Local, &signature).map_err(|_| Error::BackendFailure)?;
    // A cluster's function: (ctx, the entry's index), the `tail` convention.
    let csig = if clustered {
        let mut c = module.make_signature();
        c.call_conv = cranelift_codegen::isa::CallConv::Tail;
        c.params.extend([AbiParam::new(pointer), AbiParam::new(types::I64)]);
        // `Cut::pass`: those fields as arguments, in registers (the tail convention's x4 to x7).
        c.params.extend(cut.pass.iter().map(|_| AbiParam::new(types::I64)));
        c.returns.push(AbiParam::new(types::I64));
        c
    } else {
        signature.clone()
    };
    let cids: Vec<cranelift_module::FuncId> = if clustered {
        (0..cut.count).map(|c| module.declare_function(&format!("tempo_stream_c{c}"), Linkage::Local, &csig).map_err(|_| Error::BackendFailure)).collect::<Result<_, _>>()?
    } else {
        vec![id]
    };
    let mut fctx = FunctionBuilderContext::new();
    // The node table's passes, outlined (`outline`): made where first called, by any cluster.
    let mut reorder_fn = None;
    let mut table_fn: Option<(cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)> = None;
    let mut span_fn = None;
    let mut lowered: Vec<Lowered> = Vec::new();
    let mut word_insts: Vec<u32> = vec![0; words.len()];
    let phase = std::time::Instant::now();
    let times = std::env::var_os("RERAIL_STREAM_TIMES").is_some();
    let shared_all = shared_runs(native, &pushes);
    if times {
        eprintln!("tier 2: shared runs {:.2?}", phase.elapsed());
    }
    // Where each word can go on to: its successors, a token's next word, a shared body's end's
    // targets.
    let n_words = words.len();
    let goes_to = |pc: usize| -> Vec<usize> {
        let mut r = successors(native, &pushes, pc);
        if matches!(words[pc].operation, Op::Token { .. }) {
            r.push(pc + 1);
        }
        if let Some(&b) = shared_all.ends.get(&pc) {
            r.extend(shared_all.bodies[b].targets.iter().map(|&t| usize::from(t)));
        }
        // A shared run's place goes on into its body, not to its own next word (never lowered).
        r.retain(|&t| t < n_words && !shared_all.skipped[t]);
        r
    };
    // A place's cluster: a word's, or (past the words) a shared body's first place's.
    let cluster_of = |g: usize| (if g < n_words { cut.of[g] } else { cut.of[shared_all.bodies[g - n_words].start] }) as usize;
    // Each cluster's entries, before any is lowered: the first word, and every place of it
    // another cluster's word can go on to. A call names its entry by its index here.
    let mut incoming: Vec<std::collections::BTreeSet<usize>> = vec![std::collections::BTreeSet::new(); cut.count as usize];
    if clustered {
        incoming[cut.of[0] as usize].insert(0);
        for pc in (0..n_words).filter(|&pc| !shared_all.skipped[pc]) {
            for t in goes_to(pc) {
                if cut.of[t] != cut.of[pc] {
                    incoming[cut.of[t] as usize].insert(t);
                }
            }
            if let Some(&(b, _)) = shared_all.at.get(&pc) {
                if cut.of[shared_all.bodies[b].start] != cut.of[pc] {
                    incoming[cluster_of(n_words + b)].insert(n_words + b);
                }
            }
        }
    }
    let index: Vec<std::collections::HashMap<usize, usize>> = incoming.iter().map(|s| s.iter().enumerate().map(|(i, &g)| (g, i)).collect()).collect();
    for c in 0..cut.count {
        // The places this cluster's function names: its own words and every place they can go
        // on to. A block each, and no more: Cranelift's passes size their tables by the blocks.
        let mut named = vec![false; words.len()];
        for pc in (0..words.len()).filter(|&pc| cut.of[pc] == c && !shared_all.skipped[pc]) {
            named[pc] = true;
            for t in successors(native, &pushes, pc).into_iter().chain([pc + 1, pc + 2]) {
                if t < words.len() {
                    named[t] = true;
                }
            }
            if let Some(&b) = shared_all.ends.get(&pc) {
                for &t in &shared_all.bodies[b].targets {
                    if usize::from(t) < words.len() {
                        named[usize::from(t)] = true;
                    }
                }
            }
        }
        if let Some(first) = named.first_mut() {
            *first |= cut.of[0] == c;
        }
        let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, cids[c as usize].as_u32()), csig.clone());
        let mut heads: Vec<(Block, Step)> = Vec::new();
        let foreign = std::cell::RefCell::new(std::collections::BTreeSet::new());
        let mut exits: std::collections::HashMap<Block, usize> = std::collections::HashMap::new();
        let mut targets: Vec<(usize, Block)> = Vec::new();
        let mut tails = 0usize;
        {
        let mut f = FunctionBuilder::new(&mut func, &mut fctx);
        let entry = f.create_block();
        f.append_block_params_for_function_params(entry);
        let refuse = f.create_block();
        f.append_block_param(refuse, types::I64);
        let wb: Vec<Option<Block>> = named.iter().map(|&n| n.then(|| f.create_block())).collect();
        // A place past the words refuses, as the interpreter's `STOP`.
        let off_end = f.create_block();
        // A word's block: its own, in its cluster; else the block that calls its cluster (made
        // after the clusters are lowered, where the jump is kept).
        let goto = |pc: usize| -> Block {
            match wb.get(pc) {
                None => off_end,
                Some(&Some(b)) => {
                    if cut.of[pc] != c {
                        foreign.borrow_mut().insert(pc);
                    }
                    b
                }
                Some(None) => unreachable!("place {pc}: not among the places cluster {c} names"),
            }
        };
        let word = |t: u16| -> Block { if t == REFUSE { off_end } else { goto(usize::from(t)) } };
        // Where returns chain, each `RETURN` is a loop's head: a turn pops a call.
        let bound = bounds(native);
        let chains = bound.chains;
        // Programs without `WRAP` or a `relabel` pay nothing for them: no frames, no chains, no
        // reorder (which runs only once a wrap was made).
        let wrapping = framed(words);

        f.switch_to_block(entry);
        let ctx = f.block_params(entry)[0];
        // `can_move`: a pure value to the optimizer, removed where unused (`Cut::speed`).
        let ro = MemFlags::trusted().with_readonly().with_can_move();
        let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, ro, ctx, at);
        let bytes = load(&mut f, BYTES);
        let len = load(&mut f, LEN);
        let starts_p = load(&mut f, STARTS);
        let nstarts = load(&mut f, NSTARTS);
        let lasts_p = load(&mut f, LASTS);
        let nlasts = load(&mut f, NLASTS);
        let lasts_stride = load(&mut f, LASTS_STRIDE);
        let totals_p = load(&mut f, TOTALS);
        let (output_stride, totals_stride, totals_mask) = (load(&mut f, OUTPUT_STRIDE), load(&mut f, TOTALS_STRIDE), load(&mut f, TOTALS_MASK));
        let rooms = [load(&mut f, ROOM_NODES), load(&mut f, ROOM_SUPERS), load(&mut f, ROOM_PUSHES)];
        let outputs_p = load(&mut f, OUTPUTS);
        let cols: Vec<Value> = [KIND, FIELD, START, END, SIZE, SUPER_NODE, SUPER_KIND, OPEN, RETURNS, COUNTS].iter().map(|&at| load(&mut f, at)).collect();
        let (kind_c, field_c, start_c, end_c, size_c, snode_c, skind_c, open_c, ret_c, counts_c) =
            (cols[0], cols[1], cols[2], cols[3], cols[4], cols[5], cols[6], cols[7], cols[8], cols[9]);
        let (parent_c, extra_c) = (load(&mut f, PARENT), load(&mut f, EXTRA));
        // The table's final pass, shared by every `ACCEPT`: (kind column, size column, records).
        let fin = f.create_block();
        for _ in 0..3 {
            f.append_block_param(fin, types::I64);
        }
        let mut fin_used = false;
        let wcols: Vec<Value> = if wrapping {
            [FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE, OUT_KIND, OUT_FIELD, OUT_START, OUT_END, OUT_SIZE].iter().map(|&at| load(&mut f, at)).collect()
        } else {
            Vec::new()
        };
        let _ = &wcols;
        let (fbase_c, flast_c, flbase_c, head_c, link_c, place_c) = (0, 1, 2, 3, 4, 5);
        let zero = f.ins().iconst(types::I64, 0);
        // The state few words touch, in memory (`Mem`): not carried through every block where
        // paths meet, as an SSA variable is.
        // In clusters, the state is `tempo_stream`'s, at the address in the pinned register.
        let slot = (!clustered).then(|| f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * Mem::COUNT, 3)));
        // The register read once a block: a move each, and the value dies with its block.
        let pinned_at: std::cell::Cell<Option<(Block, Value)>> = std::cell::Cell::new(None);
        let pinned = |f: &mut FunctionBuilder<'_>| -> Value {
            let here = f.current_block().expect("in a block");
            match pinned_at.get() {
                Some((b, v)) if b == here => v,
                _ => {
                    let v = f.ins().get_pinned_reg(types::I64);
                    pinned_at.set(Some((here, v)));
                    v
                }
            }
        };
        // The state's fields the cut holds in registers (`Cut::held`): SSA variables within the
        // function, loaded where it is entered and stored where it calls another cluster.
        let vars: Vec<Option<cranelift_frontend::Variable>> = (0..Mem::COUNT).map(|k| cut.held.contains(&k).then(|| f.declare_var(types::I64))).collect();
        let mget = |f: &mut FunctionBuilder<'_>, k: Mem| match (vars[k as usize], slot) {
            (Some(v), _) => f.use_var(v),
            (None, Some(s)) => f.ins().stack_load(types::I64, s, 8 * k as i32),
            (None, None) => {
                let p = pinned(f);
                f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32)
            }
        };
        let mset = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| match (vars[k as usize], slot) {
            (Some(v), _) => f.def_var(v, x),
            (None, Some(s)) => {
                f.ins().stack_store(x, s, 8 * k as i32);
            }
            (None, None) => {
                let p = pinned(f);
                f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
            }
        };
        let one = f.ins().iconst(types::I64, 1);
        let before_first = f.ins().iconst(types::I64, -1);
        let pick = f.create_block();
        if clustered {
            for (k, v) in vars.iter().enumerate() {
                if let Some(v) = *v {
                    let x = match cut.pass.iter().position(|&q| q as usize == k) {
                        Some(j) => f.block_params(entry)[2 + j],
                        None => {
                            let p = pinned(&mut f);
                            f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32)
                        }
                    };
                    f.def_var(v, x);
                }
            }
            f.ins().jump(pick, &[]);
        } else {
            for k in 0..Mem::COUNT {
                f.ins().stack_store(zero, slot.expect("a slot"), 8 * k as i32);
                if let Some(v) = vars[k as usize] {
                    f.def_var(v, zero);
                }
            }
            mset(&mut f, Mem::Need, one);
            mset(&mut f, Mem::Lw, before_first);
            match wb.first().copied().flatten() {
                Some(b) => f.ins().jump(b, &[]),
                None => f.ins().jump(off_end, &[]),
            };
        }

        let flags = MemFlags::trusted();
        // Refuse at `pc` when `bad`; else go on in a new block.
        let guard = |f: &mut FunctionBuilder<'_>, bad: Value, pc: usize| {
            let ok = f.create_block();
            let at = f.ins().iconst(types::I64, pc as i64 + 1);
            f.ins().brif(bad, refuse, &[BlockArg::from(at)], ok, &[]);
            f.switch_to_block(ok);
        };
        // A kind as a word names it: its level's word's (`REMEMBERED`, `REMEMBER`), refused where
        // the level has none; else the kind.
        let two_slots = has_context(words);
        let kind_of = |f: &mut FunctionBuilder<'_>, kind: u16, pc: usize| {
            if kind == REMEMBERED {
                let w = mget(f, Mem::Word);
                let none = f.ins().icmp_imm(IntCC::Equal, w, 0);
                guard(f, none, pc);
                w
            } else {
                f.ins().iconst(types::I64, i64::from(kind))
            }
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

        // Shared arms (`shared_runs`): each run's code is made once, at its first place; every
        // place enters it with the run's next place (an index into its targets) and the shift
        // of its words from the first place's, for a refusal's word.
        let shared = &shared_all;
        let entries: Vec<Block> = shared.bodies.iter().map(|_| f.create_block()).collect();
        // A body's entry: its own, where its first place is in this cluster; else the call of its
        // cluster.
        let body_entry = |b: usize| -> Block {
            if cut.of[shared.bodies[b].start] != c {
                foreign.borrow_mut().insert(words.len() + b);
            }
            entries[b]
        };
        let mut marks_at: Vec<(usize, usize)> = Vec::new();
        for (pc, w) in words.iter().enumerate() {
            if shared.skipped[pc] || cut.of[pc] != c {
                continue;
            }
            // Each word's instructions, for the profile (`CompiledStream::word_insts`).
            marks_at.push((pc, f.func.dfg.num_insts()));
            if let Some(&(body, ti)) = shared.at.get(&pc) {
                let first = shared.bodies[body].start;
                f.switch_to_block(wb[pc].expect("its own place"));
                let i = f.ins().iconst(types::I64, ti as i64);
                mset(&mut f, Mem::Sel, i);
                let sh = f.ins().iconst(types::I64, pc as i64 - first as i64);
                mset(&mut f, Mem::Shift, sh);
                f.ins().jump(body_entry(body), &[]);
                if pc != first {
                    continue;
                }
                f.switch_to_block(entries[body]);
            } else {
                f.switch_to_block(wb[pc].expect("its own place"));
            }
            if let Some(&body) = shared.ends.get(&pc) {
                // The run's end: its place's next word, by the index the place gave.
                let zero = f.ins().iconst(types::I64, 0);
                mset(&mut f, Mem::Shift, zero);
                let targets = &shared.bodies[body].targets;
                if let [t] = targets.as_slice() {
                    f.ins().jump(word(*t), &[]);
                } else {
                    let i = mget(&mut f, Mem::Sel);
                    let i = f.ins().ireduce(types::I32, i);
                    let calls: Vec<_> = targets.iter().map(|&t| f.func.dfg.block_call(word(t), &[])).collect();
                    let default = f.func.dfg.block_call(off_end, &[]);
                    let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
                    f.ins().br_table(i, jt);
                }
                continue;
            }
            // `Cut::reload`: the `ctx` fields this word reads, loaded here, so none lives past
            // its word (its registers are the word's and the state's).
            let (bytes, len, starts_p, nstarts, lasts_p, nlasts, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p) =
                (bytes, len, starts_p, nstarts, lasts_p, nlasts, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p);
            let (kind_c, field_c, start_c, end_c, size_c, snode_c, skind_c, open_c, ret_c, counts_c) = (kind_c, field_c, start_c, end_c, size_c, snode_c, skind_c, open_c, ret_c, counts_c);
            let wcols_here: Vec<Value>;
            let (bytes, len, starts_p, nstarts, lasts_p, nlasts, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p, kind_c, field_c, start_c, end_c, size_c, snode_c, skind_c, open_c, ret_c, counts_c, wc) = {
                let reads = ctx_reads(&w.operation, wrapping);
                let all = reads.is_empty() && cut.reload && cold_op(&w.operation);
                let get = |f: &mut FunctionBuilder<'_>, at: i32, old: Value| -> Value {
                    if cut.reload && (all || reads.contains(&at)) {
                        // Not read-only: Cranelift would merge it with the entry's load (one
                        // value again, live everywhere).
                        let b = if clustered { pinned(f) } else { ctx };
                        f.ins().load(types::I64, MemFlags::trusted(), b, at)
                    } else {
                        old
                    }
                };
                let v = (
                    get(&mut f, BYTES, bytes), get(&mut f, LEN, len), get(&mut f, STARTS, starts_p), get(&mut f, NSTARTS, nstarts), get(&mut f, LASTS, lasts_p), get(&mut f, NLASTS, nlasts),
                    get(&mut f, LASTS_STRIDE, lasts_stride), get(&mut f, TOTALS, totals_p), get(&mut f, OUTPUT_STRIDE, output_stride), get(&mut f, TOTALS_STRIDE, totals_stride), get(&mut f, TOTALS_MASK, totals_mask),
                    [get(&mut f, ROOM_NODES, rooms[0]), get(&mut f, ROOM_SUPERS, rooms[1]), get(&mut f, ROOM_PUSHES, rooms[2])], get(&mut f, OUTPUTS, outputs_p),
                    get(&mut f, KIND, kind_c), get(&mut f, FIELD, field_c), get(&mut f, START, start_c), get(&mut f, END, end_c), get(&mut f, SIZE, size_c), get(&mut f, SUPER_NODE, snode_c), get(&mut f, SUPER_KIND, skind_c),
                    get(&mut f, OPEN, open_c), get(&mut f, RETURNS, ret_c), get(&mut f, COUNTS, counts_c),
                );
                wcols_here = if wrapping {
                    [FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE, OUT_KIND, OUT_FIELD, OUT_START, OUT_END, OUT_SIZE].iter().enumerate().map(|(i, &at)| get(&mut f, at, wcols[i])).collect()
                } else {
                    Vec::new()
                };
                let wc = |i: usize| wcols_here[i];
                (v.0, v.1, v.2, v.3, v.4, v.5, v.6, v.7, v.8, v.9, v.10, v.11, v.12, v.13, v.14, v.15, v.16, v.17, v.18, v.19, v.20, v.21, v.22, wc)
            };
            let fall = goto(pc + 1);
            match w.operation {
                Op::Token { .. } => {
                    heads.push((wb[pc].expect("its own place"), Step::Token));
                    // The path from here to the next token word writes at most `bound`'s nodes,
                    // supertypes and returns: past the tape's room, the run stops for a bigger
                    // tape. Three compares a token, not one a write.
                    let mut over = f.ins().iconst(types::I8, 0);
                    let open_calls = mget(&mut f, Mem::Ret);
                    let counts = [mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Supers), mget(&mut f, Mem::Ret)];
                    for (at, (room, (most, per_call))) in counts.into_iter().zip(rooms.into_iter().zip([(bound.nodes, bound.pop_nodes), (bound.supers, bound.pop_supers), (bound.pushes, 0)])) {
                        let at = if per_call > 0 {
                            let popped = f.ins().imul_imm(open_calls, per_call as i64);
                            f.ins().iadd(at, popped)
                        } else {
                            at
                        };
                        let reach = f.ins().iadd_imm(at, most as i64);
                        let past = f.ins().icmp(IntCC::UnsignedGreaterThan, reach, room);
                        over = f.ins().bor(over, past);
                    }
                    let fits = f.create_block();
                    let overflow = f.ins().iconst(types::I64, OVERFLOW as i64);
                    f.ins().brif(over, refuse, &[BlockArg::from(overflow)], fits, &[]);
                    f.switch_to_block(fits);
                    // A token whose bytes are not taken yet (after a mark's arm) is kept.
                    let take = f.create_block();
                    let need = mget(&mut f, Mem::Need);
                    f.ins().brif(need, take, &[], fall, &[]);
                    f.switch_to_block(take);
                    mset(&mut f, Mem::Need, zero);
                    let all = f.ins().iconst(types::I64, -1);
                    mset(&mut f, Mem::Live, all);
                    let at_end = mget(&mut f, Mem::AtEnd);
                    guard(&mut f, at_end, pc);
                    let (next, end) = (mget(&mut f, Mem::Next), mget(&mut f, Mem::End));
                    mset(&mut f, Mem::LastEnd, end);
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
                    let (start, end, limit) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::End), mget(&mut f, Mem::Limit));
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
                    mset(&mut f, Mem::Start, start);
                    mset(&mut f, Mem::End, end);
                    mset(&mut f, Mem::Next, after);
                    mset(&mut f, Mem::Limit, limit);
                    mset(&mut f, Mem::AtEnd, at_end);
                    f.ins().jump(fall, &[]);
                }
                Op::Switch { table } => {
                    let s = &native.switches[usize::from(table)];
                    let at_end = mget(&mut f, Mem::AtEnd);
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
                    let (start, limit) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::Limit));
                    let at = f.ins().iadd_imm(start, i64::from(s.offset));
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, limit);
                    guard(&mut f, past, pc);
                    let addr = f.ins().iadd(bytes, at);
                    let b = f.ins().uload8(types::I64, flags, addr, 0);
                    // A refusing target refuses at this word.
                    let no = f.create_block();
                    match cut.pick {
                        crate::native_cpu_clusters::Pick::Tree => {
                            let t = runs(s, &|t| if t == REFUSE { no } else { word(t) });
                            dispatch(&mut f, b, &t);
                        }
                        crate::native_cpu_clusters::Pick::Weighted => {
                            // Each run's weight: its target's share of this word's profile, split
                            // among the target's runs.
                            let mut first: Vec<(u32, u16)> = Vec::new();
                            for (x, &t) in s.targets.iter().enumerate() {
                                if first.last().is_none_or(|&(_, l)| l != t) {
                                    first.push((x as u32, t));
                                }
                            }
                            let per: std::collections::HashMap<u16, u64> = first.iter().fold(Default::default(), |mut m, &(_, t)| {
                                *m.entry(t).or_default() += 1;
                                m
                            });
                            let t: Vec<(u32, u32, Block, u64)> = first
                                .iter()
                                .enumerate()
                                .map(|(i, &(lo, t))| {
                                    let hi = first.get(i + 1).map_or(s.targets.len() as u32, |r| r.0);
                                    let w = if t == REFUSE { 0 } else { cut.pick_traffic.get(&(pc as u32, u32::from(t))).copied().unwrap_or(0) / per[&t] };
                                    (lo, hi, if t == REFUSE { no } else { word(t) }, w)
                                })
                                .collect();
                            dispatch_weighted(&mut f, b, &t);
                        }
                        crate::native_cpu_clusters::Pick::Table => {
                            // One indirect branch on the byte.
                            let i = f.ins().ireduce(types::I32, b);
                            let calls: Vec<_> = s.targets.iter().map(|&t| f.func.dfg.block_call(if t == REFUSE { no } else { word(t) }, &[])).collect();
                            let default = f.func.dfg.block_call(no, &[]);
                            let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
                            f.ins().br_table(i, jt);
                        }
                    }
                    f.switch_to_block(no);
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                }
                Op::Span { last: false, len: n } => {
                    let start = mget(&mut f, Mem::Start);
                    let e = f.ins().iadd_imm(start, i64::from(n));
                    let limit = mget(&mut f, Mem::Limit);
                    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, e, limit);
                    guard(&mut f, past, pc);
                    mset(&mut f, Mem::End, e);
                    let one = f.ins().iconst(types::I64, 1);
                    mset(&mut f, Mem::Need, one);
                    f.ins().jump(fall, &[]);
                }
                Op::Span { last: true, .. } => {
                    // Found once per token: a token spanned already (its end past its start: a
                    // level that ended at it left it to its caller) keeps its end.
                    let (start, end) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::End));
                    let known = f.ins().icmp(IntCC::UnsignedGreaterThan, end, start);
                    let find = f.create_block();
                    f.ins().brif(known, fall, &[], find, &[]);
                    f.switch_to_block(find);
                    // The next of the mask's bits after the last one taken: the bits of word
                    // `lw` not yet taken (`taken`), then a word (64 bytes) at a time.
                    let (w, bits, start, limit) = (mget(&mut f, Mem::Lw), mget(&mut f, Mem::Taken), mget(&mut f, Mem::Start), mget(&mut f, Mem::Limit));
                    if cut.inline_span {
                        // `Cut::inline_span`: a bit left in word `lw` is the token's end, found
                        // here (straight-line); only a search past it calls the helper.
                        let here = f.create_block();
                        let search = f.create_block();
                        f.ins().brif(bits, here, &[], search, &[]);
                        f.switch_to_block(here);
                        let tz = f.ins().ctz(bits);
                        let at = f.ins().ishl_imm(w, 6);
                        let l = f.ins().iadd(at, tz);
                        let less = f.ins().iadd_imm(bits, -1);
                        let rest = f.ins().band(bits, less);
                        let before = f.ins().icmp(IntCC::UnsignedLessThan, l, start);
                        let e = f.ins().iadd_imm(l, 1);
                        let past = f.ins().icmp(IntCC::UnsignedGreaterThan, e, limit);
                        let bad = f.ins().bor(before, past);
                        let bad = f.ins().uextend(types::I64, bad);
                        mset(&mut f, Mem::End, e);
                        mset(&mut f, Mem::Taken, rest);
                        guard(&mut f, bad, pc);
                        let one = f.ins().iconst(types::I64, 1);
                        mset(&mut f, Mem::Need, one);
                        f.ins().jump(fall, &[]);
                        f.set_cold_block(search);
                        f.switch_to_block(search);
                    }
                    if span_fn.is_none() {
                        span_fn = Some(outline(&mut module, "tempo_stream_last", 7, 4, |f, p, heads| span_last(f, p, heads))?);
                    }
                    let (id, sig, _) = span_fn.as_ref().expect("made");
                    let r = call_own(&mut f, *id, sig, &[lasts_p, lasts_stride, nlasts, w, bits, start, limit]);
                    mset(&mut f, Mem::End, r[0]);
                    mset(&mut f, Mem::Lw, r[1]);
                    mset(&mut f, Mem::Taken, r[2]);
                    guard(&mut f, r[3], pc);
                    let one = f.ins().iconst(types::I64, 1);
                    mset(&mut f, Mem::Need, one);
                    f.ins().jump(fall, &[]);
                }
                Op::Word { string, test } => {
                    // The token's bytes against the string: its length, then an 8-byte load per
                    // 8 bytes of the string (64 readable bytes past the file, the token starting
                    // inside it), the last masked to its length. Straight-line: no loop.
                    let text = &native.strings[usize::from(string)];
                    let (start, end) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::End));
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
                        let skip = goto(pc + 2);
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
                    let skip = goto(pc + 2);
                    let (at_end, live) = (mget(&mut f, Mem::AtEnd), mget(&mut f, Mem::Live));
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
                    let start = mget(&mut f, Mem::Start);
                    let w = f.ins().ushr_imm(start, 6);
                    let off = f.ins().imul_imm(output_stride, output as i64);
                    let base = f.ins().iadd(outputs_p, off);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let bit = f.ins().ushr(word, s);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().brif(bit, taken, &[], skip, &[]);
                    f.switch_to_block(taken);
                    // It and the marks before it are done at this token.
                    let live = mget(&mut f, Mem::Live);
                    let rest = f.ins().band_imm(live, !((2i64 << order) - 1));
                    mset(&mut f, Mem::Live, rest);
                    f.ins().jump(fall, &[]);
                }
                Op::Row { mask } => {
                    // The row's start bit at the token's start: one load.
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    let skip = goto(pc + 2);
                    let start = mget(&mut f, Mem::Start);
                    let w = f.ins().ushr_imm(start, 6);
                    let off = f.ins().imul_imm(output_stride, output as i64);
                    let base = f.ins().iadd(outputs_p, off);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let bit = f.ins().ushr(word, s);
                    let bit = f.ins().band_imm(bit, 1);
                    f.ins().brif(bit, fall, &[], skip, &[]);
                }
                Op::Open { kind } => {
                    mset(&mut f, Mem::Pending, zero);
                    let (nodes, start, open) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Start), mget(&mut f, Mem::Open));
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, kind_c, nodes, k);
                    put(&mut f, field_c, nodes, zero);
                    put(&mut f, start_c, nodes, start);
                    put(&mut f, open_c, open, nodes);
                    if wrapping {
                        // Its frame, at depth + 1 (slot 0 is the root's parent's): its own base,
                        // no node made yet; and its record is no base of a wrap yet.
                        let slot1 = f.ins().iadd_imm(open, 1);
                        put(&mut f, wc(fbase_c), slot1, nodes);
                        put(&mut f, wc(flast_c), slot1, zero);
                        put(&mut f, wc(head_c), nodes, zero);
                    }
                    let open = f.ins().iadd_imm(open, 1);
                    let nodes = f.ins().iadd_imm(nodes, 1);
                    mset(&mut f, Mem::Open, open);
                    mset(&mut f, Mem::Nodes, nodes);
                    f.ins().jump(fall, &[]);
                }
                Op::Close { this } => {
                    let open = mget(&mut f, Mem::Open);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    guard(&mut f, none, pc);
                    let open = f.ins().iadd_imm(open, -1);
                    mset(&mut f, Mem::Open, open);
                    let at = {
                        let a = slot(&mut f, open_c, open);
                        f.ins().uload32(flags, a, 0)
                    };
                    let (nodes, pending) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending));
                    let e = if this {
                        mget(&mut f, Mem::End)
                    } else {
                        let (keep, last_end) = (mget(&mut f, Mem::Keep), mget(&mut f, Mem::LastEnd));
                        f.ins().select(pending, keep, last_end)
                    };
                    put(&mut f, end_c, at, e);
                    let base = if wrapping {
                        // Its subtree starts at its base (a wrap's is the moved node's); it is the
                        // last node made at its parent's level.
                        let slot1 = f.ins().iadd_imm(open, 1);
                        let base = {
                            let a = slot(&mut f, wc(fbase_c), slot1);
                            f.ins().uload32(flags, a, 0)
                        };
                        let at1 = f.ins().iadd_imm(at, 1);
                        put(&mut f, wc(flast_c), open, at1);
                        put(&mut f, wc(flbase_c), open, base);
                        base
                    } else {
                        at
                    };
                    let size = f.ins().isub(nodes, base);
                    let size = f.ins().isub(size, pending);
                    put(&mut f, size_c, at, size);
                    f.ins().jump(fall, &[]);
                }
                Op::Leaf { kind, deferred, extra } => {
                    let pending = if deferred {
                        // The first pending leaf keeps the end of the token before it.
                        let (p, keep, last_end) = (mget(&mut f, Mem::Pending), mget(&mut f, Mem::Keep), mget(&mut f, Mem::LastEnd));
                        let keep = f.ins().select(p, keep, last_end);
                        mset(&mut f, Mem::Keep, keep);
                        f.ins().iadd_imm(p, 1)
                    } else {
                        zero
                    };
                    mset(&mut f, Mem::Pending, pending);
                    let (nodes, start, end) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Start), mget(&mut f, Mem::End));
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    let one = f.ins().iconst(types::I64, 1);
                    put(&mut f, kind_c, nodes, k);
                    put(&mut f, field_c, nodes, zero);
                    put(&mut f, start_c, nodes, start);
                    put(&mut f, end_c, nodes, end);
                    put(&mut f, size_c, nodes, one);
                    if wrapping {
                        put(&mut f, wc(head_c), nodes, zero);
                        if !deferred && !extra {
                            // The last node made at the innermost open node's level.
                            let open = mget(&mut f, Mem::Open);
                            let n1 = f.ins().iadd_imm(nodes, 1);
                            put(&mut f, wc(flast_c), open, n1);
                            put(&mut f, wc(flbase_c), open, nodes);
                        }
                    }
                    let nodes = f.ins().iadd_imm(nodes, 1);
                    mset(&mut f, Mem::Nodes, nodes);
                    f.ins().jump(fall, &[]);
                }
                Op::Remember { kind } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    mset(&mut f, Mem::Word, k);
                    f.ins().jump(fall, &[]);
                }
                Op::Flag { flag, op } => {
                    let fl = mget(&mut f, Mem::Flags);
                    let bit = 1i64 << flag;
                    match op {
                        FlagOp::Set => {
                            let fl = f.ins().bor_imm(fl, bit);
                            mset(&mut f, Mem::Flags, fl);
                            f.ins().jump(fall, &[]);
                        }
                        FlagOp::Clear => {
                            let fl = f.ins().band_imm(fl, !bit);
                            mset(&mut f, Mem::Flags, fl);
                            f.ins().jump(fall, &[]);
                        }
                        // The next word where the flag is set; else the one after.
                        FlagOp::Test => {
                            let skip = goto(pc + 2);
                            let set = f.ins().band_imm(fl, bit);
                            f.ins().brif(set, fall, &[], skip, &[]);
                        }
                    }
                }
                Op::Retag { kind, on: On::Moved } => {
                    let m = mget(&mut f, Mem::Moved);
                    let k = kind_of(&mut f, kind, pc);
                    put(&mut f, kind_c, m, k);
                    f.ins().jump(fall, &[]);
                }
                Op::Wrap { kind } if wrapping => {
                    // Around the last node made at the innermost open node's level: its record
                    // at the table's end, linked into the moved node's base's chain; open, its
                    // frame's base that base, the moved node the last made at its level.
                    let open = mget(&mut f, Mem::Open);
                    let (last, base) = {
                        let a = slot(&mut f, wc(flast_c), open);
                        let b = slot(&mut f, wc(flbase_c), open);
                        (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, b, 0))
                    };
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    let bad = f.ins().bor(none, no_last);
                    guard(&mut f, bad, pc);
                    let r = f.ins().iadd_imm(last, -1);
                    let (nodes, wraps) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Wraps));
                    let (fld, st) = {
                        let a = slot(&mut f, field_c, r);
                        let b = slot(&mut f, start_c, r);
                        (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, b, 0))
                    };
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, kind_c, nodes, k);
                    put(&mut f, field_c, nodes, fld);
                    put(&mut f, start_c, nodes, st);
                    put(&mut f, field_c, r, zero);
                    let h = {
                        let a = slot(&mut f, wc(head_c), base);
                        f.ins().uload32(flags, a, 0)
                    };
                    put(&mut f, wc(link_c), nodes, h);
                    let n1 = f.ins().iadd_imm(nodes, 1);
                    put(&mut f, wc(head_c), base, n1);
                    let mark = f.ins().iconst(types::I64, IS_WRAP);
                    put(&mut f, wc(head_c), nodes, mark);
                    put(&mut f, open_c, open, nodes);
                    let slot1 = f.ins().iadd_imm(open, 1);
                    put(&mut f, wc(fbase_c), slot1, base);
                    put(&mut f, wc(flast_c), slot1, last);
                    put(&mut f, wc(flbase_c), slot1, base);
                    mset(&mut f, Mem::Moved, r);
                    mset(&mut f, Mem::Pending, zero);
                    mset(&mut f, Mem::Open, slot1);
                    mset(&mut f, Mem::Nodes, n1);
                    let wraps = f.ins().iadd_imm(wraps, 1);
                    mset(&mut f, Mem::Wraps, wraps);
                    f.ins().jump(fall, &[]);
                }
                Op::NodeField { field, on: On::Inner } => {
                    // The node the node made last, a wrap, holds first: that frame's last base.
                    let open = mget(&mut f, Mem::Open);
                    let (last, base) = {
                        let a = slot(&mut f, wc(flast_c), open);
                        let b = slot(&mut f, wc(flbase_c), open);
                        (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, b, 0))
                    };
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    let bad = f.ins().bor(none, no_last);
                    guard(&mut f, bad, pc);
                    let r = f.ins().iadd_imm(last, -1);
                    let mark = {
                        let a = slot(&mut f, wc(head_c), r);
                        f.ins().uload32(flags, a, 0)
                    };
                    let not_wrap = f.ins().icmp_imm(IntCC::NotEqual, mark, IS_WRAP);
                    guard(&mut f, not_wrap, pc);
                    // The node it moved is the record at its base unless it is a wrap too, which
                    // an older wrap linked before this one shows.
                    let older = {
                        let a = slot(&mut f, wc(link_c), r);
                        f.ins().uload32(flags, a, 0)
                    };
                    let nested = f.ins().icmp_imm(IntCC::NotEqual, older, 0);
                    guard(&mut f, nested, pc);
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    put(&mut f, field_c, base, x);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind, on: On::Made } | Op::NodeField { field: kind, on: On::Made } => {
                    // The node made last at the innermost open node's level (`relabel`): its
                    // frame's last, one more than its record (0: none).
                    let open = mget(&mut f, Mem::Open);
                    let last = {
                        let a = slot(&mut f, wc(flast_c), open);
                        f.ins().uload32(flags, a, 0)
                    };
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    let bad = f.ins().bor(none, no_last);
                    guard(&mut f, bad, pc);
                    let r = f.ins().iadd_imm(last, -1);
                    let x = if matches!(w.operation, Op::Retag { .. }) { kind_of(&mut f, kind, pc) } else { f.ins().iconst(types::I64, i64::from(kind)) };
                    let col = if matches!(w.operation, Op::Retag { .. }) { kind_c } else { field_c };
                    put(&mut f, col, r, x);
                    f.ins().jump(fall, &[]);
                }
                Op::Reread { reading } if wrapping => {
                    // The node made last at the innermost open node's level and the nodes inside
                    // it: the records from its base to the pending extras, each base's chain of
                    // wraps first (outermost first, as the reorder places them). A loop over the
                    // records and one over a chain, each step a record (`Step::Node`); a kind
                    // each, by a run of compares.
                    let r = native.readings.maps.get(usize::from(reading)).ok_or(Error::UnsupportedBackend)?;
                    let open = mget(&mut f, Mem::Open);
                    let (last, base) = {
                        let a = slot(&mut f, wc(flast_c), open);
                        let b = slot(&mut f, wc(flbase_c), open);
                        (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, b, 0))
                    };
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    let bad = f.ins().bor(none, no_last);
                    guard(&mut f, bad, pc);
                    let root = f.ins().iadd_imm(last, -1);
                    let (nodes, pending) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending));
                    let end = f.ins().isub(nodes, pending);
                    let (scan, chain) = (f.create_block(), f.create_block());
                    f.append_block_param(scan, types::I64);
                    f.append_block_param(chain, types::I64);
                    f.append_block_param(chain, types::I64);
                    heads.push((scan, Step::Node));
                    heads.push((chain, Step::Node));
                    // The root's base first, its chain from the root (`chain(root + 1, base)`; a
                    // plain root is its base): the wraps around the root are not read.
                    let wrapped = f.ins().icmp(IntCC::NotEqual, root, base);
                    let c0 = f.ins().select(wrapped, last, zero);
                    f.ins().jump(chain, &[BlockArg::from(c0), BlockArg::from(base)]);
                    // scan(i): record i, a wrap's skipped (its chain's base reads it).
                    f.switch_to_block(scan);
                    let i = f.block_params(scan)[0];
                    let (body, plain) = (f.create_block(), f.create_block());
                    let done = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, end);
                    f.ins().brif(done, fall, &[], body, &[]);
                    f.switch_to_block(body);
                    let h = {
                        let a = slot(&mut f, wc(head_c), i);
                        f.ins().uload32(flags, a, 0)
                    };
                    let is_wrap = f.ins().icmp_imm(IntCC::Equal, h, IS_WRAP);
                    f.ins().brif(is_wrap, plain, &[], chain, &[BlockArg::from(h), BlockArg::from(i)]);
                    f.switch_to_block(plain);
                    let i1 = f.ins().iadd_imm(i, 1);
                    f.ins().jump(scan, &[BlockArg::from(i1)]);
                    // chain(c, i): the wrap `c - 1` on base i, else (c = 0) record i itself.
                    f.switch_to_block(chain);
                    let (c, i) = (f.block_params(chain)[0], f.block_params(chain)[1]);
                    let cm1 = f.ins().iadd_imm(c, -1);
                    let n = f.ins().select(c, cm1, i);
                    let k = {
                        let a = slot(&mut f, kind_c, n);
                        f.ins().uload32(flags, a, 0)
                    };
                    // Its field, unless it is the node read: the reading's, by a run of selects.
                    let fld = {
                        let a = slot(&mut f, field_c, n);
                        f.ins().uload32(flags, a, 0)
                    };
                    let mut to = fld;
                    for &(a, b) in &r.fields {
                        let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(a));
                        let g = f.ins().iconst(types::I64, i64::from(b));
                        to = f.ins().select(is, g, to);
                    }
                    let is_root = f.ins().icmp(IntCC::Equal, n, root);
                    let to = f.ins().select(is_root, fld, to);
                    put(&mut f, field_c, n, to);
                    // The chain's next: an older wrap, or record i; after record i, the next.
                    let next = f.create_block();
                    let i1 = f.ins().iadd_imm(i, 1);
                    let older = {
                        let at = f.ins().select(c, cm1, zero);
                        let a = slot(&mut f, wc(link_c), at);
                        f.ins().uload32(flags, a, 0)
                    };
                    // Its way: the root's, for the node read, when the reading gives one; else
                    // its kind's, a compare each, the last refusing.
                    let ways: Vec<(Option<u16>, u16)> = r.root.map(|w| (None, w)).into_iter().chain(r.kinds.iter().map(|&(kk, w)| (Some(kk), w))).collect();
                    for (kk, way) in ways {
                        let (hit, miss) = (f.create_block(), f.create_block());
                        let is = match kk {
                            Some(kk) => f.ins().icmp_imm(IntCC::Equal, k, i64::from(kk)),
                            None => is_root,
                        };
                        f.ins().brif(is, hit, &[], miss, &[]);
                        f.switch_to_block(hit);
                        match way {
                            KEEP => {
                                let size = {
                                    let a = slot(&mut f, size_c, n);
                                    f.ins().uload32(flags, a, 0)
                                };
                                let past = f.ins().iadd(i, size);
                                f.ins().jump(scan, &[BlockArg::from(past)]);
                            }
                            GONE => {
                                // The node read going: none made last at this level until the next.
                                let kept = f.ins().select(is_root, zero, last);
                                let a = slot(&mut f, wc(flast_c), open);
                                f.ins().istore32(flags, kept, a, 0);
                                let d = f.ins().iconst(types::I64, i64::from(DEAD));
                                put(&mut f, kind_c, n, d);
                                let dead = mget(&mut f, Mem::Dead);
                                let dead = f.ins().iadd_imm(dead, 1);
                                mset(&mut f, Mem::Dead, dead);
                                f.ins().jump(next, &[]);
                            }
                            m => {
                                let m = kind_of(&mut f, m, pc);
                                put(&mut f, kind_c, n, m);
                                f.ins().jump(next, &[]);
                            }
                        }
                        f.switch_to_block(miss);
                    }
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                    f.switch_to_block(next);
                    let (more, own) = (f.create_block(), f.create_block());
                    f.ins().brif(c, more, &[], own, &[]);
                    f.switch_to_block(more);
                    f.ins().jump(chain, &[BlockArg::from(older), BlockArg::from(i)]);
                    f.switch_to_block(own);
                    f.ins().jump(scan, &[BlockArg::from(i1)]);
                }
                Op::Retag { kind, on: On::Here } => {
                    // The innermost open node's kind.
                    let open = mget(&mut f, Mem::Open);
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
                Op::Merge { kind, fold: true } => {
                    // The innermost open node holds two leaves and nothing else, no extra
                    // pending: the first becomes one leaf through the second's end.
                    let (nodes, pending, open) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending), mget(&mut f, Mem::Open));
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    guard(&mut f, none, pc);
                    let top = f.ins().iadd_imm(open, -1);
                    let o = {
                        let a = slot(&mut f, open_c, top);
                        f.ins().uload32(flags, a, 0)
                    };
                    let want = f.ins().iadd_imm(o, 3);
                    let count = f.ins().icmp(IntCC::NotEqual, nodes, want);
                    let waiting = f.ins().icmp_imm(IntCC::NotEqual, pending, 0);
                    let bad = f.ins().bor(count, waiting);
                    guard(&mut f, bad, pc);
                    let (a, b) = (f.ins().iadd_imm(o, 1), f.ins().iadd_imm(o, 2));
                    let (sa, sb) = {
                        let x = slot(&mut f, size_c, a);
                        let y = slot(&mut f, size_c, b);
                        (f.ins().uload32(flags, x, 0), f.ins().uload32(flags, y, 0))
                    };
                    let na = f.ins().icmp_imm(IntCC::NotEqual, sa, 1);
                    let nb = f.ins().icmp_imm(IntCC::NotEqual, sb, 1);
                    let bad = f.ins().bor(na, nb);
                    guard(&mut f, bad, pc);
                    let end = {
                        let x = slot(&mut f, end_c, b);
                        f.ins().uload32(flags, x, 0)
                    };
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, kind_c, a, k);
                    put(&mut f, end_c, a, end);
                    mset(&mut f, Mem::Nodes, b);
                    if wrapping {
                        // The first is the last node made at this level now.
                        put(&mut f, wc(flast_c), open, b);
                        put(&mut f, wc(flbase_c), open, a);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Merge { kind, fold: false } => {
                    // The grammar's leaf before the pending extras: a leaf, inside the innermost
                    // open node. The extras are its text: dropped.
                    let (nodes, pending, open) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending), mget(&mut f, Mem::Open));
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
                    let end = mget(&mut f, Mem::End);
                    put(&mut f, kind_c, leaf, k);
                    put(&mut f, end_c, leaf, end);
                    mset(&mut f, Mem::Nodes, kept);
                    mset(&mut f, Mem::Pending, zero);
                    f.ins().jump(fall, &[]);
                }
                Op::NodeField { field, on } => {
                    let last = if on == On::Moved {
                        mget(&mut f, Mem::Moved)
                    } else {
                        let nodes = mget(&mut f, Mem::Nodes);
                        f.ins().iadd_imm(nodes, -1)
                    };
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    put(&mut f, field_c, last, x);
                    f.ins().jump(fall, &[]);
                }
                Op::NodeSuper { kind } => {
                    let (nodes, supers) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Supers));
                    let last = f.ins().iadd_imm(nodes, -1);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    put(&mut f, snode_c, supers, last);
                    put(&mut f, skind_c, supers, k);
                    let supers = f.ins().iadd_imm(supers, 1);
                    mset(&mut f, Mem::Supers, supers);
                    f.ins().jump(fall, &[]);
                }
                Op::Push { target } => {
                    let ret = mget(&mut f, Mem::Ret);
                    let t = f.ins().iconst(types::I64, i64::from(target));
                    // With context (`REMEMBER`, `FLAG`), two slots a call: its return, then the
                    // caller's word and flags (`context`).
                    if two_slots {
                        let at = f.ins().ishl_imm(ret, 1);
                        put(&mut f, ret_c, at, t);
                        let (w, fl) = (mget(&mut f, Mem::Word), mget(&mut f, Mem::Flags));
                        let fl = f.ins().ishl_imm(fl, 16);
                        let wf = f.ins().bor(w, fl);
                        let at = f.ins().iadd_imm(at, 1);
                        put(&mut f, ret_c, at, wf);
                    } else {
                        put(&mut f, ret_c, ret, t);
                    }
                    let ret = f.ins().iadd_imm(ret, 1);
                    mset(&mut f, Mem::Ret, ret);
                    f.ins().jump(fall, &[]);
                }
                Op::Return { again } => {
                    if chains {
                        heads.push((wb[pc].expect("its own place"), Step::Return));
                    }
                    let ret = mget(&mut f, Mem::Ret);
                    let none = f.ins().icmp_imm(IntCC::Equal, ret, 0);
                    guard(&mut f, none, pc);
                    let ret = f.ins().iadd_imm(ret, -1);
                    mset(&mut f, Mem::Ret, ret);
                    let p = if two_slots {
                        // The caller's word and flags back, from beside its return.
                        let at = f.ins().ishl_imm(ret, 1);
                        let a = slot(&mut f, ret_c, at);
                        let wf = f.ins().uload32(flags, a, 4);
                        let w = f.ins().band_imm(wf, 0xffff);
                        let fl = f.ins().ushr_imm(wf, 16);
                        mset(&mut f, Mem::Word, w);
                        mset(&mut f, Mem::Flags, fl);
                        f.ins().uload32(flags, a, 0)
                    } else {
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
                    let (w, bits) = (mget(&mut f, Mem::Lw), mget(&mut f, Mem::Taken));
                    let (_, scan) = mask_scan(&mut f, Mask { first: lasts_p, stride: lasts_stride, words: nlasts }, w, bits, refuse, pc, false);
                    heads.push((scan, Step::Block));
                    let (at_end, open, ret) = (mget(&mut f, Mem::AtEnd), mget(&mut f, Mem::Open), mget(&mut f, Mem::Ret));
                    let not_end = f.ins().icmp_imm(IntCC::Equal, at_end, 0);
                    let not_one = f.ins().icmp_imm(IntCC::NotEqual, open, 1);
                    let returns = f.ins().icmp_imm(IntCC::NotEqual, ret, 0);
                    let bad = f.ins().bor(not_end, not_one);
                    let bad = f.ins().bor(bad, returns);
                    guard(&mut f, bad, pc);
                    let root = f.ins().uload32(flags, open_c, 0);
                    put(&mut f, end_c, root, len);
                    let nodes = mget(&mut f, Mem::Nodes);
                    let size = f.ins().isub(nodes, root);
                    put(&mut f, size_c, root, size);
                    if !native.readings.provisional.is_empty() {
                        // No node left holds a provisional field (a dropped record aside): a loop
                        // over the table, a step a record.
                        let (check, step, after) = (f.create_block(), f.create_block(), f.create_block());
                        f.append_block_param(check, types::I64);
                        heads.push((check, Step::Node));
                        f.ins().jump(check, &[BlockArg::from(zero)]);
                        f.switch_to_block(check);
                        let i = f.block_params(check)[0];
                        let done = f.ins().icmp(IntCC::Equal, i, nodes);
                        f.ins().brif(done, after, &[], step, &[]);
                        f.switch_to_block(step);
                        let (fld, k) = {
                            let a = slot(&mut f, field_c, i);
                            let b = slot(&mut f, kind_c, i);
                            (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, b, 0))
                        };
                        let mut hit = f.ins().iconst(types::I8, 0);
                        for &p in &native.readings.provisional {
                            let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(p));
                            hit = f.ins().bor(hit, is);
                        }
                        let live = f.ins().icmp_imm(IntCC::NotEqual, k, i64::from(DEAD));
                        let bad = f.ins().band(hit, live);
                        guard(&mut f, bad, pc);
                        let i1 = f.ins().iadd_imm(i, 1);
                        f.ins().jump(check, &[BlockArg::from(i1)]);
                        f.switch_to_block(after);
                    }
                    f.ins().store(flags, nodes, counts_c, 0);
                    let supers = mget(&mut f, Mem::Supers);
                    f.ins().store(flags, supers, counts_c, 8);
                    if wrapping {
                        // A dropped record needs the table in preorder too: without it.
                        let (wraps, dead) = (mget(&mut f, Mem::Wraps), mget(&mut f, Mem::Dead));
                        let wraps = f.ins().iadd(wraps, dead);
                        f.ins().store(flags, wraps, counts_c, 16);
                        let rereads = words.iter().any(|b| matches!(b.operation, Op::Reread { .. }));
                        let r = Reorder { nodes, supers, wraps, cols: [kind_c, field_c, start_c, end_c, size_c], out: [wc(6), wc(7), wc(8), wc(9), wc(10)], head: wc(head_c), link: wc(link_c), place: wc(place_c), snode: snode_c, before: rereads.then(|| wc(fbase_c)), counts: counts_c };
                        let args = r.values();
                        if reorder_fn.is_none() {
                            let before = r.before.is_some();
                            reorder_fn = Some(outline(&mut module, "tempo_stream_reorder", args.len(), 0, |f, p, heads| reorder(f, Reorder::of(p, before), heads))?);
                        }
                        let (id, sig, _) = reorder_fn.as_ref().expect("made");
                        call_own(&mut f, *id, sig, &args);
                        // Without a wrap made, the table is in preorder as made.
                        let placed = f.ins().load(types::I64, flags, counts_c, 0);
                        let kind = f.ins().select(wraps, r.out[0], r.cols[0]);
                        let size = f.ins().select(wraps, r.out[4], r.cols[4]);
                        f.ins().jump(fin, &[BlockArg::from(kind), BlockArg::from(size), BlockArg::from(placed)]);
                    } else {
                        f.ins().jump(fin, &[BlockArg::from(kind_c), BlockArg::from(size_c), BlockArg::from(nodes)]);
                    }
                    fin_used = true;
                }
                Op::Empty { total } => {
                    // A total of tier 1's layout: 8 readable bytes at each (`Ctx::totals_mask`).
                    let off = f.ins().imul_imm(totals_stride, i64::from(total));
                    let at = f.ins().iadd(totals_p, off);
                    let t = f.ins().load(types::I64, flags, at, 0);
                    let t = f.ins().band(t, totals_mask);
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
        marks_at.push((usize::MAX, f.func.dfg.num_insts()));
        for w in marks_at.windows(2) {
            word_insts[w[0].0] = (w[1].1 - w[0].1) as u32;
        }
        if clustered {
            // Each place of another cluster a word here goes to: the held fields stored, then
            // the tail call of its cluster with the place's index there.
            use cranelift_codegen::ir::{ExtFuncData, ExternalName, UserExternalName};
            let mut refs: std::collections::HashMap<usize, cranelift_codegen::ir::FuncRef> = std::collections::HashMap::new();
            let named_foreign: Vec<usize> = foreign.borrow().iter().copied().collect();
            for g in named_foreign {
                let d = cluster_of(g);
                // Named only by a word's fall it never takes (no word goes on to it).
                let Some(&i) = index[d].get(&g) else { continue };
                let b = if g < n_words { wb[g].expect("a place named") } else { entries[g - n_words] };
                f.switch_to_block(b);
                for (k, v) in vars.iter().enumerate() {
                    if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                        let x = f.use_var(v);
                        let p = pinned(&mut f);
                        f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                    }
                }
                let passed: Vec<Value> = cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))).collect();
                let fref = *refs.entry(d).or_insert_with(|| {
                    let name = f.func.declare_imported_user_function(UserExternalName::new(0, cids[d].as_u32()));
                    let signature = f.import_signature(csig.clone());
                    f.import_function(ExtFuncData { name: ExternalName::user(name), signature, colocated: true })
                });
                let iv = f.ins().iconst(types::I64, i as i64);
                let args: Vec<Value> = [ctx, iv].into_iter().chain(passed).collect();
                f.ins().return_call(fref, &args);
                exits.insert(b, g);
                tails += 1;
            }
            // The entry's place, by its index among this cluster's entries.
            f.switch_to_block(pick);
            for &g in &incoming[c as usize] {
                targets.push((g, if g < n_words { wb[g].expect("an own place") } else { entries[g - n_words] }));
            }
            let at = f.block_params(entry)[1];
            if targets.len() == 1 {
                // One entry: no choice to make.
                f.ins().jump(targets[0].1, &[]);
            } else if !cut.pick_traffic.is_empty() {
                // Each entry weighted by the profile's steps into it from other clusters, the
                // hottest tested first (`dispatch_weighted`), not one indirect branch.
                let into: Vec<u64> = targets
                    .iter()
                    .map(|&(g, _)| {
                        let pc = if g < n_words { g } else { shared_all.bodies[g - n_words].start };
                        cut.pick_traffic.iter().filter(|&(&(a, b), _)| b as usize == pc && (a as usize) < n_words && cut.of[a as usize] != c).map(|(_, &w)| w).sum()
                    })
                    .collect();
                let runs: Vec<(u32, u32, Block, u64)> = targets.iter().enumerate().map(|(i, &(_, b))| (i as u32, i as u32 + 1, b, into[i])).collect();
                dispatch_weighted(&mut f, at, &runs);
            } else {
                let i = f.ins().ireduce(types::I32, at);
                let default = f.func.dfg.block_call(off_end, &[]);
                let calls: Vec<_> = targets.iter().map(|&(_, b)| f.func.dfg.block_call(b, &[])).collect();
                let jt = f.create_jump_table(cranelift_codegen::ir::JumpTableData::new(default, &calls));
                f.ins().br_table(i, jt);
            }
        }
        if fin_used {
            f.switch_to_block(fin);
            let p = f.block_params(fin).to_vec();
            let extras = &native.readings.extras;
            if table_fn.is_none() {
                table_fn = Some(outline(&mut module, "tempo_stream_table", 6, 1, |f, q, heads| table(f, [q[0], q[1]], q[2], q[3], [q[4], q[5]], extras, heads))?);
            }
            let made = table_fn.as_ref().expect("made");
            let (open_c, parent_c, extra_c) = if cut.reload {
                let b = if clustered { pinned(&mut f) } else { ctx };
                (f.ins().load(types::I64, MemFlags::trusted(), b, OPEN), f.ins().load(types::I64, MemFlags::trusted(), b, PARENT), f.ins().load(types::I64, MemFlags::trusted(), b, EXTRA))
            } else {
                (open_c, parent_c, extra_c)
            };
            let r = call_own(&mut f, made.0, &made.1, &[p[0], p[1], p[2], open_c, parent_c, extra_c]);
            f.ins().return_(&[r[0]]);
        }
        f.switch_to_block(off_end);
        let past = f.ins().iconst(types::I64, words.len() as i64 + 1);
        f.ins().jump(refuse, &[BlockArg::from(past)]);
        f.switch_to_block(refuse);
        let pc = f.block_params(refuse)[0];
        // Inside a shared run, its words are the first place's: the shift makes them this one's.
        let sh = mget(&mut f, Mem::Shift);
        let pc = f.ins().iadd(pc, sh);
        f.ins().return_(&[pc]);
        f.seal_all_blocks();
        f.finalize();
        }
        lowered.push(Lowered { func, heads, targets, exits, tails });
    }
    if times {
        eprintln!("tier 2: lowered {:.2?}", phase.elapsed());
    }
    let own: Vec<_> = reorder_fn.into_iter().chain(table_fn).chain(span_fn).collect();
    let names: Vec<_> = own.iter().map(|(id, _, _)| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect();
    let crossings: usize = lowered.iter().map(|l| l.tails).sum();
    if clustered {
        check_across(&lowered)?;
    }
    let cluster_names: Vec<_> = cids.iter().map(|id| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect();
    for l in &lowered {
        crate::native_cpu_shape::check_tails(&l.func, &l.heads, &names, if clustered { &cluster_names } else { &[] }).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    }
    if times {
        eprintln!("tier 2: joined and checked {:.2?}", phase.elapsed());
    }
    // `tempo_stream` itself, in clusters: the state, then the first word's cluster.
    let driver = if clustered {
        let mut func = cranelift_codegen::ir::Function::with_name_signature(cranelift_codegen::ir::UserFuncName::user(0, id.as_u32()), signature.clone());
        {
            let mut f = FunctionBuilder::new(&mut func, &mut fctx);
            let entry = f.create_block();
            f.append_block_params_for_function_params(entry);
            f.switch_to_block(entry);
            let ctx = f.block_params(entry)[0];
            // The state, in `ctx` (`Ctx::state`).
            let zero = f.ins().iconst(types::I64, 0);
            for k in 0..Mem::COUNT {
                f.ins().store(MemFlags::trusted(), zero, ctx, STATE + 8 * k as i32);
            }
            let one = f.ins().iconst(types::I64, 1);
            f.ins().store(MemFlags::trusted(), one, ctx, STATE + 8 * Mem::Need as i32);
            let before_first = f.ins().iconst(types::I64, -1);
            f.ins().store(MemFlags::trusted(), before_first, ctx, STATE + 8 * Mem::Lw as i32);
            // `ctx` in the pinned register for the clusters, the caller's value kept (x21 is
            // callee-saved, and Cranelift saves no pinned register).
            let kept = f.ins().get_pinned_reg(types::I64);
            f.ins().set_pinned_reg(ctx);
            // The first word is its cluster's first entry (`join_clusters`).
            let first = f.ins().iconst(types::I64, 0);
            // The passed fields' first values, as the state's in `ctx`.
            let passed: Vec<Value> = cut.pass.iter().map(|&k| if k == Mem::Need as u32 { one } else if k == Mem::Lw as u32 { before_first } else { zero }).collect();
            let args: Vec<Value> = [ctx, first].into_iter().chain(passed).collect();
            let r = call_own(&mut f, cids[cut.of[0] as usize], &csig, &args);
            f.ins().set_pinned_reg(kept);
            f.ins().return_(&[r[0]]);
            f.seal_all_blocks();
            f.finalize();
        }
        crate::native_cpu_shape::check(&func, &[], &cluster_names).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
        Some(func)
    } else {
        None
    };
    // The token loop's function first (a driver, in clusters), then the clusters', then the
    // outlined functions: compiled at once (`define_functions`), their IR and machine code
    // appended in that order.
    let mut heads: Vec<(String, Step)> = Vec::new();
    let mut items: Vec<(cranelift_module::FuncId, Context)> = Vec::new();
    let context_of = |module: &JITModule, func: cranelift_codegen::ir::Function| {
        let mut c: Context = module.make_context();
        c.func = func;
        c.set_disasm(true);
        c
    };
    if let Some(func) = driver {
        items.push((id, context_of(&module, func)));
    }
    for (c, l) in lowered.into_iter().enumerate() {
        heads.extend(l.heads.iter().map(|(b, s)| (if clustered { format!("c{c}:{b}") } else { b.to_string() }, *s)));
        items.push((cids[c], context_of(&module, l.func)));
    }
    for (fid, _, func) in own {
        items.push((fid, context_of(&module, func)));
    }
    let mut clif = String::new();
    for (_, c) in &items {
        clif.push_str(&c.func.display().to_string());
    }
    let defined = crate::native_cpu_codegen::define_functions(&mut module, items, cut.near).map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2: {e:?}");
        }
        Error::BackendFailure
    })?;
    if times {
        eprintln!("tier 2: compiled {:.2?}", phase.elapsed());
    }
    let mut disassembly = String::new();
    for c in &defined {
        disassembly.push_str(&c.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default());
    }
    let calls = clif.lines().filter(|l| l.trim().starts_with("call ") || l.contains(" = call ")).count();
    module.finalize_definitions().map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2's module: {e:?}");
        }
        Error::BackendFailure
    })?;
    let code = module.get_finalized_function(id);
    // SAFETY: `tempo_stream` was declared and defined with this signature.
    let entry = unsafe { std::mem::transmute::<*const u8, Entry>(code) };
    let own_calls = calls;
    let marks = words.iter().filter_map(|b| if let Op::Mark { order, .. } = b.operation { Some(u64::from(order) + 1) } else { None }).max().unwrap_or(0);
    let wraps = framed(words);
    let two_slots = has_context(words);
    Ok(CompiledStream { _module: module, entry, bounds: bounds(native), starts, lasts, marks, wraps, context: two_slots, clif, disassembly, heads, own_calls, clusters: cut.count as usize, crossings, word_insts })
}

/// The loops across clusters, checked as a function's are: from each place a cluster is
/// entered at, the tail calls it reaches before any loop head; over all clusters, no cycle of
/// those (`native_cpu_shape::acyclic`).
fn check_across(lowered: &[Lowered]) -> Result<(), Error> {
    let mut edges: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for l in lowered {
        let blocks: Vec<Block> = l.targets.iter().map(|&(_, b)| b).collect();
        for (&(g, _), reach) in l.targets.iter().zip(crate::native_cpu_shape::exits(&l.func, &l.heads, &blocks, &l.exits)) {
            edges.insert(g, reach);
        }
    }
    crate::native_cpu_shape::acyclic(&edges).map_err(|r| Error::RefusedShape(format!("{r:?}")))
}

/// The `ctx` fields a word of the hot path reads (`Cut::reload`); none for the words that read
/// many and are cold (`cold_op`: they load every field).
fn ctx_reads(op: &Op, wrapping: bool) -> Vec<i32> {
    let mut r: Vec<i32> = match op {
        Op::Token { .. } => vec![STARTS, NSTARTS, LEN, ROOM_NODES, ROOM_SUPERS, ROOM_PUSHES],
        Op::Switch { .. } | Op::Word { .. } => vec![BYTES],
        Op::Span { last: true, .. } => vec![LASTS, LASTS_STRIDE, NLASTS],
        Op::Mark { .. } | Op::Row { .. } => vec![OUTPUTS, OUTPUT_STRIDE, LASTS_STRIDE],
        Op::Open { .. } => vec![KIND, FIELD, START, OPEN],
        Op::Close { .. } => vec![OPEN, END, SIZE],
        Op::Leaf { .. } => vec![KIND, FIELD, START, END, SIZE],
        Op::Retag { on: On::Moved, .. } => vec![KIND],
        Op::Retag { on: On::Here, .. } => vec![OPEN, KIND],
        Op::Merge { .. } => vec![OPEN, SIZE, END, KIND],
        Op::NodeField { on: On::Moved | On::Here, .. } => vec![FIELD],
        Op::NodeSuper { .. } => vec![SUPER_NODE, SUPER_KIND],
        Op::Push { .. } | Op::Return { .. } => vec![RETURNS],
        Op::Empty { .. } => vec![TOTALS, TOTALS_STRIDE, TOTALS_MASK],
        _ => vec![],
    };
    if wrapping && matches!(op, Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::Merge { .. }) {
        r.extend([FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE]);
    }
    r
}

/// The words that read many `ctx` fields and run rarely: each loads them all (`Cut::reload`).
fn cold_op(op: &Op) -> bool {
    matches!(op, Op::Accept | Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made | On::Inner, .. } | Op::NodeField { on: On::Made | On::Inner, .. })
}

/// Whether the program has context beside each call's return (a level's word, its flags): two
/// return slots a call.
fn has_context(words: &[crate::native_ir::StructuralBlock]) -> bool {
    words.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. }))
}

/// Whether the program keeps a frame per open node: it wraps, or it relabels the node made last
/// at a level (`On::Made`).
fn framed(words: &[crate::native_ir::StructuralBlock]) -> bool {
    words.iter().any(|b| matches!(b.operation, Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made, .. } | Op::NodeField { on: On::Made | On::Inner, .. }))
}

/// Tier 1's buffers as tier 2 reads them, where tier 1 wrote them (the CPU's batch, or the
/// device's buffers in shared memory): the starts (two readable slots past `nstarts`); the
/// mask's words, word `i` at `lasts + i * lasts_stride` bytes, `nlasts` of them; output `o`'s
/// words from `outputs + o * output_stride` bytes, a word every `lasts_stride`; total `t` the
/// bits `totals_mask` of the 8 readable bytes at `totals + t * totals_stride`.
#[derive(Clone, Copy)]
pub struct Tier1View {
    pub starts: *const u32,
    pub nstarts: usize,
    pub lasts: *const u64,
    pub nlasts: usize,
    pub lasts_stride: usize,
    pub outputs: *const u64,
    pub output_stride: usize,
    pub totals: *const u8,
    pub totals_stride: usize,
    pub totals_mask: u64,
}

/// The buffers one call writes, sized from the stream and the program's `Bounds`.
pub struct Tape {
    pub kind: Vec<u32>,
    pub field: Vec<u32>,
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub size: Vec<u32>,
    pub super_node: Vec<u32>,
    pub super_kind: Vec<u32>,
    open: Vec<u32>,
    returns: Vec<u32>,
    /// The node count, the supertype count, the wraps (then the table is in `out`).
    pub counts: [u64; 3],
    /// A program with `WRAP`'s: frames, chains, places, and the table in preorder (kind, field,
    /// start, end, size). Empty otherwise.
    frames: [Vec<u32>; 3],
    chains: [Vec<u32>; 3],
    out: [Vec<u32>; 5],
    /// The final pass's columns (`table`): each node's parent and extra flag, in preorder.
    pub parent: Vec<u32>,
    pub extra: Vec<u8>,
}

impl CompiledStream {
    /// Room for a stream of `n` starts.
    pub fn tape(&self, n: usize) -> Tape {
        // Room for what files write (on the `.rr` corpus, a node a token and fewer supertypes
        // and open calls), and a token's path past it; a run that could pass it stops at a
        // token word (`OVERFLOW`) and `call` runs it again on `full_tape`.
        let b = &self.bounds;
        let tokens = n as u64 + 2;
        self.sized((2 * tokens + b.nodes + 1) as usize, (tokens + b.supers + 1) as usize, (tokens / 4 + b.pushes + 64) as usize)
    }

    /// A tape with the program's bound's room for `n` starts: no run passes it.
    fn full_tape(&self, n: usize) -> Tape {
        // A token is switched on once, and once more after each mark's arm.
        let paths = (n as u64 + 2) * (1 + self.marks);
        self.sized((self.bounds.nodes * paths + 1) as usize, (self.bounds.supers * paths + 1) as usize, (self.bounds.pushes * paths + 1) as usize)
    }

    fn sized(&self, nodes: usize, supers: usize, pushes: usize) -> Tape {
        Tape {
            kind: vec![0; nodes],
            field: vec![0; nodes],
            start: vec![0; nodes],
            end: vec![0; nodes],
            size: vec![0; nodes],
            super_node: vec![0; supers],
            super_kind: vec![0; supers],
            open: vec![0; nodes],
            returns: vec![0; if self.context { 2 * pushes } else { pushes }],
            counts: [0; 3],
            frames: std::array::from_fn(|_| vec![0; if self.wraps { nodes + 1 } else { 0 }]),
            chains: std::array::from_fn(|_| vec![0; if self.wraps { nodes } else { 0 }]),
            out: std::array::from_fn(|_| vec![0; if self.wraps { nodes } else { 0 }]),
            parent: vec![0; nodes],
            extra: vec![0; nodes],
        }
    }

    /// One call over a file. `bytes` is the file (`len` its length), with 64 readable bytes
    /// after it; `tier1` is tier 1's buffers for it.
    ///
    /// # Safety
    /// The pointers must be as stated, and `tape` from `self.tape(tier1.nstarts)`.
    pub unsafe fn call(&self, bytes: *const u8, len: usize, tier1: Tier1View, tape: &mut Tape) -> u64 {
        // SAFETY: the caller's contract.
        let r = unsafe { self.call_once(bytes, len, tier1, tape) };
        if r != OVERFLOW {
            return r;
        }
        ROOM_RETRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *tape = self.full_tape(tier1.nstarts);
        // SAFETY: as above, on the bound's tape.
        let r = unsafe { self.call_once(bytes, len, tier1, tape) };
        if r == OVERFLOW { self.overflow_pc() } else { r }
    }

    /// A run that passed even the bound's room (a conservative check where returns chain):
    /// refused at the program's end, the file goes to the fallback.
    fn overflow_pc(&self) -> u64 {
        u64::MAX - 1
    }

    /// `call` on the tape as given.
    ///
    /// # Safety
    /// As `call`.
    unsafe fn call_once(&self, bytes: *const u8, len: usize, tier1: Tier1View, tape: &mut Tape) -> u64 {
        let mut ctx = Ctx {
            bytes,
            len: len as u64,
            starts: tier1.starts,
            nstarts: tier1.nstarts as u64,
            lasts: tier1.lasts,
            nlasts: tier1.nlasts as u64,
            totals: tier1.totals.cast(),
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
            lasts_stride: tier1.lasts_stride as u64,
            outputs: tier1.outputs,
            frame_base: tape.frames[0].as_mut_ptr(),
            frame_last: tape.frames[1].as_mut_ptr(),
            frame_last_base: tape.frames[2].as_mut_ptr(),
            head: tape.chains[0].as_mut_ptr(),
            link: tape.chains[1].as_mut_ptr(),
            place: tape.chains[2].as_mut_ptr(),
            out_kind: tape.out[0].as_mut_ptr(),
            out_field: tape.out[1].as_mut_ptr(),
            out_start: tape.out[2].as_mut_ptr(),
            out_end: tape.out[3].as_mut_ptr(),
            out_size: tape.out[4].as_mut_ptr(),
            parent: tape.parent.as_mut_ptr(),
            extra: tape.extra.as_mut_ptr(),
            output_stride: tier1.output_stride as u64,
            totals_stride: tier1.totals_stride as u64,
            totals_mask: tier1.totals_mask,
            room_nodes: tape.kind.len() as u64,
            room_supers: tape.super_node.len() as u64,
            room_pushes: (tape.returns.len() / if self.context { 2 } else { 1 }) as u64,
            state: [0; Mem::COUNT as usize],
        };
        // SAFETY: the caller's contract; the function writes only the tape's buffers and the
        // state in `ctx`.
        unsafe { (self.entry)(&raw mut ctx) }
    }

    /// The words over tier 1's run of `bytes`, as the interpreter's `stream::run` gives them.
    pub fn run(&self, bytes: &[u8], tier1: &crate::block::BlockRun) -> Result<Vec<Node>, Refused> {
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
        let (lasts, nlasts) = match self.lasts {
            Some(m) => (all[m..].as_ptr(), tier1.outputs[m].len()),
            None => (all.as_ptr(), 0),
        };
        let mut tape = self.tape(starts.1);
        let totals = if tier1.totals.is_empty() { vec![0] } else { tier1.totals.clone() };
        // SAFETY: `padded` has the byte before and 64 after; each stream two slots past its count;
        // the tape is sized for the stream; every output has `count` words in `all`.
        let view = Tier1View {
            starts: starts.0.as_ptr(),
            nstarts: starts.1,
            lasts,
            nlasts,
            lasts_stride: 8 * n,
            outputs: all.as_ptr(),
            output_stride: 8,
            totals: totals.as_ptr().cast(),
            totals_stride: 8,
            totals_mask: u64::MAX,
        };
        let r = unsafe { self.call(padded.as_ptr().wrapping_add(1), bytes.len(), view, &mut tape) };
        if r != 0 {
            return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse" });
        }
        Ok(tape.nodes())
    }
}

/// The node table as tier 2 wrote it, in preorder: a column per field, a record each. Taken
/// from the tape's buffers as they are (`Tape::columns`), not copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Columns {
    pub kind: Vec<u32>,
    pub field: Vec<u32>,
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub size: Vec<u32>,
    /// The parent's record; `NO_PARENT` for the root.
    pub parent: Vec<u32>,
    /// 1 for an extra (a comment).
    pub extra: Vec<u8>,
}

impl Columns {
    pub fn len(&self) -> usize {
        self.kind.len()
    }
}

impl Tape {
    /// The node table's columns: the tape's own buffers, cut to the records written.
    pub fn columns(mut self) -> Columns {
        let n = self.counts[0] as usize;
        let take = |v: &mut Vec<u32>| {
            let mut v = std::mem::take(v);
            v.truncate(n);
            v
        };
        let [kind, field, start, end, size] = if self.counts[2] > 0 {
            let [a, b, c, d, e] = &mut self.out;
            [take(a), take(b), take(c), take(d), take(e)]
        } else {
            [take(&mut self.kind), take(&mut self.field), take(&mut self.start), take(&mut self.end), take(&mut self.size)]
        };
        let (parent, mut extra) = (take(&mut self.parent), std::mem::take(&mut self.extra));
        extra.truncate(n);
        Columns { kind, field, start, end, size, parent, extra }
    }

    /// The node table as `stream::Node`s.
    pub fn nodes(&self) -> Vec<Node> {
        let n = self.counts[0] as usize;
        let [kind, field, start, end, size] = if self.counts[2] > 0 { [&self.out[0], &self.out[1], &self.out[2], &self.out[3], &self.out[4]] } else { [&self.kind, &self.field, &self.start, &self.end, &self.size] };
        let mut out: Vec<Node> =
            (0..n).map(|i| Node { kind: kind[i] as u16, field: field[i] as u16, supers: Vec::new(), start: start[i], end: end[i], size: size[i] }).collect();
        for s in 0..self.counts[1] as usize {
            out[self.super_node[s] as usize].supers.push(self.super_kind[s] as u16);
        }
        out
    }
}

/// A language's whole compiled path: tier 1's function, then tier 2's over the buffers tier 1
/// wrote. Between and around the two calls the host only lays out the file (padded, as simdjson
/// requires) and reads the node table back.
pub struct Parser {
    pub tier1: crate::native_cpu_block::CompiledBlock,
    pub tier2: CompiledStream,
}

impl Parser {
    pub fn new(native: &Structure) -> Result<Self, Error> {
        let block = native.block_programs.first().ok_or(Error::UnsupportedBackend)?;
        Ok(Parser { tier1: crate::native_cpu_block::compile(block)?, tier2: compile(native)? })
    }

    /// `new`, with tier 2 in the clusters of `cut` (`compile_cut`).
    pub fn with_cut(native: &Structure, cut: &crate::native_cpu_clusters::Cut) -> Result<Self, Error> {
        let block = native.block_programs.first().ok_or(Error::UnsupportedBackend)?;
        Ok(Parser { tier1: crate::native_cpu_block::compile(block)?, tier2: compile_cut(native, cut)? })
    }

    /// The node table of `bytes` as tier 2 wrote it, or the word that refused it (the file goes
    /// to tree-sitter).
    pub fn columns(&self, bytes: &[u8]) -> Result<Columns, Refused> {
        let mut batch = crate::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let mut tape = self.tape(&batch);
        self.words(&batch, &mut tape)?;
        Ok(tape.columns())
    }


    /// The nodes of `bytes` as `stream::Node`s, for the tests' comparisons with the interpreter.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn parse(&self, bytes: &[u8]) -> Result<Vec<Node>, Refused> {
        let mut batch = crate::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let mut tape = self.tape(&batch);
        self.words(&batch, &mut tape)?;
        Ok(tape.nodes())
    }

    /// Tier 1's stream `e` in `batch` (its first file), as tier 1 wrote it: its positions (room
    /// for two past its count) and count.
    fn stream<'b>(&self, batch: &'b crate::native_cpu_block::Batch, e: Option<usize>) -> (&'b [u32], usize) {
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
    pub fn tape(&self, batch: &crate::native_cpu_block::Batch) -> Tape {
        self.tier2.tape(self.stream(batch, self.tier2.starts).1)
    }

    /// Tier 2's function over tier 1's buffers in `batch` (one file), into `tape`; or the word
    /// that refused the file.
    pub fn words(&self, batch: &crate::native_cpu_block::Batch, tape: &mut Tape) -> Result<(), Refused> {
        let row = batch.rows[0];
        let extracts = self.tier1.extracts.len();
        let starts = self.stream(batch, self.tier2.starts);
        let outputs = batch.out[row.out as usize..].as_ptr();
        let (lasts, nlasts) = match self.tier2.lasts {
            Some(m) => (batch.out[row.out as usize + m..].as_ptr(), row.blocks as usize),
            None => (outputs, 0),
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
        let view = Tier1View {
            starts: starts.0.as_ptr(),
            nstarts: starts.1,
            lasts,
            nlasts,
            lasts_stride: 8 * self.tier1.outputs,
            outputs,
            output_stride: 8,
            totals: totals.as_ptr().cast(),
            totals_stride: 8,
            totals_mask: u64::MAX,
        };
        let r = unsafe { self.tier2.call(batch.bytes.as_ptr().wrapping_add(row.bytes as usize), row.len as usize, view, tape) };
        if r != 0 {
            return Err(Refused { pc: r as usize - 1, why: "the compiled words refuse" });
        }
        Ok(())
    }
}
