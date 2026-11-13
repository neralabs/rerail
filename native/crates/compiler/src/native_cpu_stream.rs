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
use crate::stream::{ByteSwitch, DEAD, GONE, KEEP, Node, REFUSE, REMEMBERED, Refused, WORD_KIND};
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
    /// The records' other columns (`records`): each token's last byte and its code, zipped
    /// with `starts`.
    lasts: *const u32,
    codes: *const u32,
    totals: *const u64,
    /// The working rows, 12 bytes a node (`native_cpu_tape`): key, start, end.
    rows: *mut u8,
    super_node: *mut u32,
    super_kind: *mut u32,
    open: *mut u32,
    returns: *mut u32,
    /// Written at `ACCEPT`: the node count, the supertype count.
    counts: *mut u64,
    /// The bytes from one of a mask's words to the next.
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
    /// Unused: the table is written in place (decision 0136).
    out_rows: *mut u8,
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
    /// With the calls open counted by the machine's stack (`calls_on_stack`): the stack
    /// pointer where the run's first rule is called, the lowest it may go (`STACK_BYTES` below
    /// it), and the log of the smallest rule function's frame, in bytes (so the calls open are
    /// at most the stack's depth shifted down by it).
    stack_base: u64,
    stack_limit: u64,
    frame_shift: u64,
    /// In clusters (`compile_cut`), the run's state (`Mem`): every cluster reaches it through
    /// `ctx`, its first argument.
    state: [u64; Mem::COUNT as usize],
    /// The tape (docs/parser-generators/37): its columns at the compiled widths, the bases, the
    /// image (the header's room, then the code column in place), and the code ids: each key's
    /// (`NO_CODE`: none), and each code's key and first node.
    size_c: *mut u8,
    off_c: *mut u8,
    tail_c: *mut u8,
    base_c: *mut u32,
    image: *mut u8,
    id_of: *mut u16,
    code_kf: *mut u16,
    code_first: *mut u32,
}

const SIZE_C: i32 = std::mem::offset_of!(Ctx, size_c) as i32;
const OFF_C: i32 = std::mem::offset_of!(Ctx, off_c) as i32;
const TAIL_C: i32 = std::mem::offset_of!(Ctx, tail_c) as i32;
const BASE_C: i32 = std::mem::offset_of!(Ctx, base_c) as i32;
const IMAGE: i32 = std::mem::offset_of!(Ctx, image) as i32;
const ID_OF: i32 = std::mem::offset_of!(Ctx, id_of) as i32;
const CODE_KF: i32 = std::mem::offset_of!(Ctx, code_kf) as i32;
const CODE_FIRST: i32 = std::mem::offset_of!(Ctx, code_first) as i32;

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
const CODES: i32 = 40;
const TOTALS: i32 = 48;
const ROWS: i32 = 56;
const SUPER_NODE: i32 = 64;
const SUPER_KIND: i32 = 72;
const OPEN: i32 = 80;
const RETURNS: i32 = 88;
const COUNTS: i32 = 96;
const LASTS_STRIDE: i32 = 104;
const OUTPUTS: i32 = 112;
const FRAME_BASE: i32 = 120;
const FRAME_LAST: i32 = 128;
const FRAME_LAST_BASE: i32 = 136;
const HEAD: i32 = 144;
const LINK: i32 = 152;
const PLACE: i32 = 160;
const OUT_ROWS: i32 = 168;
const OUTPUT_STRIDE: i32 = 176;
const TOTALS_STRIDE: i32 = 184;
const TOTALS_MASK: i32 = 192;
const ROOM_NODES: i32 = 200;
const ROOM_SUPERS: i32 = 208;
const ROOM_PUSHES: i32 = 216;
const STACK_BASE: i32 = 224;
const STACK_LIMIT: i32 = 232;
const FRAME_SHIFT: i32 = 240;
const STATE: i32 = 248;
const _: () = assert!(std::mem::offset_of!(Ctx, out_rows) == OUT_ROWS as usize && std::mem::offset_of!(Ctx, stack_base) == STACK_BASE as usize && std::mem::offset_of!(Ctx, state) == STATE as usize);

// The working rows (`native_cpu_tape`): a node's key, start and end, 12 bytes. The tape's columns
// are apart, at the compiled widths.
use crate::native_cpu_tape::{AT_END, AT_FIELD, AT_KIND, AT_START, ROW};

/// A field of the node rows: the rows' address and the field's place in a row.
#[derive(Clone, Copy)]
struct NodeCol {
    rows: Value,
    at: i32,
}

impl NodeCol {
    fn of(rows: Value, at: i32) -> Self {
        NodeCol { rows, at }
    }

    /// Row `i`'s address: one `madd`, then each field at an offset from it, as the lean
    /// parser's push. Behind a `bitcast` of its own type (no instruction): Cranelift folds an
    /// `iadd` it can see into every load's and store's address (an `add` and a register offset
    /// each).
    fn row(f: &mut FunctionBuilder<'_>, rows: Value, i: Value) -> Value {
        let off = f.ins().imul_imm(i, ROW);
        let a = f.ins().iadd(rows, off);
        f.ins().bitcast(types::I64, MemFlags::new(), a)
    }

    /// Row `i`'s field, stored (its width: kind and field 16 bits).
    fn put(self, f: &mut FunctionBuilder<'_>, i: Value, x: Value) {
        let a = Self::row(f, self.rows, i);
        self.put_row(f, a, x);
    }

    /// The same, at a row's address (`row`): a word's stores to one row share it, as the lean
    /// parser's push computes its row once.
    fn put_row(self, f: &mut FunctionBuilder<'_>, a: Value, x: Value) {
        let flags = MemFlags::trusted();
        match self.at {
            AT_KIND | AT_FIELD => f.ins().istore16(flags, x, a, self.at),
            _ => f.ins().istore32(flags, x, a, self.at),
        };
    }

    /// Row `i`'s field, loaded.
    fn get(self, f: &mut FunctionBuilder<'_>, i: Value) -> Value {
        let a = Self::row(f, self.rows, i);
        self.get_row(f, a)
    }

    /// The same, at a row's address.
    fn get_row(self, f: &mut FunctionBuilder<'_>, a: Value) -> Value {
        let flags = MemFlags::trusted();
        match self.at {
            AT_KIND | AT_FIELD => f.ins().uload16(types::I64, flags, a, self.at),
            _ => f.ins().uload32(flags, a, self.at),
        }
    }
}

/// Returns 0 when the words accept, else one more than the refusing word's place.
/// Tier 2's entry: `ctx`, the run's end.
pub type Entry = unsafe extern "C" fn(*const Ctx) -> u64;

/// Keep each tier 2 function's IR (`CompiledStream::ir`), for an ahead-of-time build of the same
/// code (`clif_rust`).
pub static KEEP_IR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    /// The log of the smallest rule function's frame (`Ctx::frame_shift`).
    frame_shift: u64,
    bounds: Bounds,
    /// The extract `TOKEN` reads, and the output of the mask `SPAN m=1` reads.
    pub starts: Option<usize>,
    pub lasts: Option<usize>,
    /// The marks `MARK` tests: a token can be switched on once per mark and once more.
    marks: u64,
    /// The program has `WRAP`: the tape has the frames, chains and preorder columns.
    pub wraps: bool,
    /// The widths the tape is written at (docs/parser-generators/37), and the language's tag.
    pub widths: rerail_tape_format::Widths,
    tag: u8,
    /// The program has context beside each return (`has_context`): two return slots a call.
    context: bool,
    pub clif: String,
    pub disassembly: String,
    /// The machine code as emitted, each function's after the last: after the branches are
    /// folded, which `disassembly` (Cranelift's code before emission) is not.
    pub code_bytes: Vec<u8>,
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
    /// Each word's machine code's bytes, in its rule's function (not the functions it calls),
    /// for the shape: where the code goes.
    pub word_bytes: Vec<u32>,
    /// Each word's machine code's ranges in `code_bytes` (its place, start, end), for the shape.
    pub word_ranges: Vec<(u32, u32, u32)>,
    /// With `KEEP_IR`: each function's IR as Cranelift was given it (its `FuncId`), and the
    /// entry's id.
    pub ir: Vec<(u32, cranelift_codegen::ir::Function)>,
    pub entry_id: u32,
}

/// Each word's phases (`stream::phases`), over the lowered words: `TAKEN` after a `SPAN`,
/// `UNTAKEN` after a `TOKEN`; a return lands on its caller's `TOKEN` with the token taken
/// (`RETURN m=0`) or past it (`m=1`). The verifier checked them on the program.
pub(crate) fn phases(native: &Structure, pushes: &[u16]) -> Vec<u8> {
    use crate::stream::{TAKEN, UNTAKEN};
    let n = native.structural.len();
    let mut phase = vec![0u8; n];
    let mut work = vec![(0usize, TAKEN)];
    while let Some((pc, ph)) = work.pop() {
        if pc >= n || phase[pc] & ph != 0 {
            continue;
        }
        phase[pc] |= ph;
        let after = match native.structural[pc].operation {
            Op::Span { .. } => TAKEN,
            Op::Token { .. } => UNTAKEN,
            _ => ph,
        };
        match native.structural[pc].operation {
            Op::Return { again } => work.extend(pushes.iter().map(|&p| if again { (usize::from(p) + 1, UNTAKEN) } else { (usize::from(p), TAKEN) })),
            Op::Token { .. } => work.push((pc + 1, after)),
            _ => work.extend(successors(native, pushes, pc).into_iter().map(|q| (q, after))),
        }
    }
    phase
}

/// Each word's successors, as `stream::validate` reads them, `TOKEN`'s cut.
pub(crate) fn successors(native: &Structure, pushes: &[u16], pc: usize) -> Vec<usize> {
    match native.structural[pc].operation {
        Op::Stop | Op::Accept | Op::Token { .. } => vec![],
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => {
            let s = &native.switches[usize::from(table)];
            let mut out: Vec<usize> = s.all_targets().filter(|&t| t != REFUSE).map(usize::from).collect();
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

/// Each word's successors within its rule (`Cut::calls`): a call (`PUSH`) goes on at its
/// return's place and the one after (`RETURN m=1`), its callee called; `RETURN` leaves the
/// rule; `TOKEN` goes on to the next word.
pub fn rule_successors(native: &Structure, pc: usize) -> Vec<usize> {
    match native.structural[pc].operation {
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } => vec![],
        Op::Token { .. } => vec![pc + 1],
        _ => successors(native, &[], pc),
    }
    .into_iter()
    .filter(|&n| n < native.structural.len())
    .collect()
}

/// The word `pc` runs as, past `GOTO`s.
pub fn past_gotos(native: &Structure, mut pc: usize) -> usize {
    for _ in 0..native.structural.len() {
        match native.structural.get(pc).map(|w| w.operation) {
            Some(Op::Goto { target }) => pc = usize::from(target),
            _ => break,
        }
    }
    pc
}

/// A call's callee: the word after its `PUSH`, past `GOTO`s.
pub fn callee(native: &Structure, pc: usize) -> usize {
    past_gotos(native, pc + 1)
}

/// A rule's native return (`Cut::calls`): to its call's place, or (`RETURN m=1`) the one after:
/// 0 or 1, one test each (`cbz`, a compare with 1), as a `Result`'s. Any other value a call gives
/// back is the run's end (accepted, refused, out of room) with its top bit flipped (`ENDED`),
/// passed on; the driver flips it back.
pub const RETURNED: u64 = 0;
/// The top bit, flipped in the run's end values inside the clusters' functions (`RETURNED`).
pub const ENDED: u64 = 1 << 63;

/// Native calls nested at most this deep (`Cut::calls`): past it, the file is refused (to the
/// fallback), as the machine's stack is the calls'. A rule's frame is at most about 350 bytes
/// on `.rr`: some 360 KB at the most.
pub const DEPTH: i64 = 1024;

/// With the calls open counted by the machine's stack (`calls_on_stack`): the most of it a run
/// takes, about `DEPTH` calls of `.rr`'s largest frames. Past it, a rule function refuses the
/// file where it is entered (to the fallback), as a stack check in a prologue does.
pub const STACK_BYTES: i64 = 384 << 10;

/// Whether the machine's stack can count the calls open (`Cut::calls`), as the lean parser's
/// has no count: no `RETURN` runs with no call open, and no `ACCEPT` within a call. Each by the
/// words reached at the same depth (`rule_successors`): from the start, no `RETURN`; from
/// every callee's entry, no `ACCEPT`. So neither needs a check, and the count serves only the
/// depth (a check where each rule function is entered) and the room for the nodes returns
/// write (`TOKEN`'s, bounded by the stack's depth).
pub fn calls_on_stack(native: &Structure) -> bool {
    let n = native.structural.len();
    let reach = |from: Vec<usize>| -> Vec<bool> {
        let mut seen = vec![false; n];
        let mut todo = from;
        while let Some(pc) = todo.pop() {
            if pc >= n || std::mem::replace(&mut seen[pc], true) {
                continue;
            }
            todo.extend(rule_successors(native, pc));
        }
        seen
    };
    let top = reach(vec![0]);
    if (0..n).any(|pc| top[pc] && matches!(native.structural[pc].operation, Op::Return { .. })) {
        return false;
    }
    let callees: Vec<usize> = (0..n).filter(|&pc| matches!(native.structural[pc].operation, Op::Push { .. })).map(|pc| callee(native, pc)).collect();
    let inner = reach(callees);
    !(0..n).any(|pc| inner[pc] && matches!(native.structural[pc].operation, Op::Accept))
}

/// The calls open, at most (`calls_on_stack`): the stack's depth below the run's first call,
/// shifted down by the log of the smallest rule frame (each call open has one such frame, or
/// more).
fn calls_open_by_stack(f: &mut FunctionBuilder<'_>, ctx: Value) -> Value {
    let flags = MemFlags::trusted();
    let sp = f.ins().get_stack_pointer(types::I64);
    let base = f.ins().load(types::I64, flags, ctx, STACK_BASE);
    let shift = f.ins().load(types::I64, flags, ctx, FRAME_SHIFT);
    let depth = f.ins().isub(base, sp);
    f.ins().ushr(depth, shift)
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
            Op::Push { target, .. } => Some(target),
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
    /// The node the last wrap moved.
    Moved,
    /// A shared arm's target, as an index into its body's (`shared_runs`).
    Sel,
    /// A shared arm's words' distance from its body's first place's, for a refusal there.
    Shift,
    /// With wraps: the last node made at the innermost open node's level, one more than its
    /// row (0: none). Only the innermost level's is ever read, and each change of level writes
    /// the new one's: a field, not a column by depth.
    LastMade,
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
    /// The count of tokens (`COUNT`).
    Depth,
    /// The tape: the last block's base; the nodes committed (their codes written) and the
    /// codes; the flags of values that did not fit their widths (`native_cpu_tape::WIDE_OFF`).
    BlockBase,
    Coded,
    Codes,
    Wide,
}

impl Mem {
    const COUNT: u32 = 27;
}

/// `Ctx::state`'s slots (`rust_stream` writes `Ctx` field for field).
pub(crate) const STATE_SLOTS: usize = Mem::COUNT as usize;

/// The fields a rule's function holds in registers (`Cut::held`) and passes to its calls, given
/// back with their results (`Cut::pass`): none. The whole state stays in memory, read and
/// written where used, as the lean parser's `&mut self` fields are: it reloads `self.i` and the
/// token where it tests them, and keeps nothing live across a rule. Held and passed, the
/// token's code, start, end and end flag were live through every block and spilled (15,133
/// stack loads over `.rr`'s tier 2, against 161 in the lean parser).
pub const HELD_IN_RULES: [u32; 0] = [];

/// A compare tree over a byte (or a place), as rustc lowers a `match` (LLVM's switch lowering):
/// `runs` are runs of one target (the first from 0, the last on to `end`, past every value the
/// switch reads: 256 for a byte). The target with the most runs is the default; the tree is over
/// the other runs alone (the cases), split at the middle, each side knowing the range its value
/// is in; a case is one test at its leaf (an equality, a bound, or a range), and a value in no
/// case goes to the default. A switch that expects one kind is one compare, as `expect(kind)`.
/// Cases of at most three targets within 64 values are bit tests, as LLVM's: one shift, then a
/// mask a target.
fn dispatch(f: &mut FunctionBuilder<'_>, x: Value, runs: &[(u32, Block)], end: u64) {
    let ends: Vec<u64> = runs.iter().skip(1).map(|r| u64::from(r.0)).chain([end]).collect();
    let spans: Vec<(u64, u64, Block)> = runs.iter().zip(&ends).enumerate().map(|(i, (r, &e))| (if i == 0 { 0 } else { u64::from(r.0) }, e, r.1)).filter(|&(a, b, _)| a < b).collect();
    // The default: the target with the most runs (the first such).
    let mut count: Vec<(Block, usize)> = Vec::new();
    for &(_, _, b) in &spans {
        match count.iter_mut().find(|(x, _)| *x == b) {
            Some(c) => c.1 += 1,
            None => count.push((b, 1)),
        }
    }
    let default = count.iter().max_by_key(|&&(_, n)| n).map(|&(b, _)| b).expect("a run");
    let cases: Vec<(u64, u64, Block)> = spans.into_iter().filter(|&(_, _, b)| b != default).collect();
    let clusters = clusters(&cases);
    cases_tree(f, x, 0, end, &clusters, default);
}

/// A leaf of `dispatch`'s tree: the values `a..b`, to one target (`Range`), or tested by bits
/// (`Bits`: each target with its mask over `x - a`; a value in none goes to the default).
enum Cluster {
    Range(u64, u64, Block),
    Bits(u64, u64, Vec<(Block, u64)>),
}

impl Cluster {
    fn start(&self) -> u64 {
        match *self {
            Cluster::Range(a, ..) | Cluster::Bits(a, ..) => a,
        }
    }
}

/// `cases` (rising, apart) as clusters: from each case on, the longest run of cases within 64
/// values and of at most three targets, as bit tests where LLVM finds them worth it (one
/// target and 3 compares saved, two and 5, three and 6: a range counting two), else the case.
fn clusters(cases: &[(u64, u64, Block)]) -> Vec<Cluster> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < cases.len() {
        let mut best = None;
        let (mut dests, mut cmps): (Vec<Block>, usize) = (Vec::new(), 0);
        for j in i..cases.len() {
            let (a, b, t) = cases[j];
            if b - cases[i].0 > 64 || b == u64::MAX {
                break;
            }
            if !dests.contains(&t) {
                if dests.len() == 3 {
                    break;
                }
                dests.push(t);
            }
            cmps += if b == a + 1 { 1 } else { 2 };
            let worth = match dests.len() {
                1 => cmps >= 3,
                2 => cmps >= 5,
                _ => cmps >= 6,
            };
            if worth {
                best = Some(j);
            }
        }
        match best {
            Some(j) => {
                let base = cases[i].0;
                let mut masks: Vec<(Block, u64)> = Vec::new();
                for &(a, b, t) in &cases[i..=j] {
                    let m = (a..b).fold(0u64, |m, v| m | 1 << (v - base));
                    match masks.iter_mut().find(|(x, _)| *x == t) {
                        Some(x) => x.1 |= m,
                        None => masks.push((t, m)),
                    }
                }
                // The target with the most values first, as LLVM orders them.
                masks.sort_by_key(|&(_, m)| std::cmp::Reverse(m.count_ones()));
                out.push(Cluster::Bits(base, cases[j].1, masks));
                i = j + 1;
            }
            None => {
                let (a, b, t) = cases[i];
                out.push(Cluster::Range(a, b, t));
                i += 1;
            }
        }
    }
    out
}

/// `dispatch`'s tree over `cases` for a value known to be in `lo..hi`.
fn cases_tree(f: &mut FunctionBuilder<'_>, x: Value, lo: u64, hi: u64, cases: &[Cluster], default: Block) {
    match cases {
        [] => {
            f.ins().jump(default, &[]);
        }
        [Cluster::Range(a, b, to)] => {
            let (a, b, to) = (*a, *b, *to);
            let inside = if a <= lo && b >= hi {
                f.ins().jump(to, &[]);
                return;
            } else if a <= lo {
                f.ins().icmp_imm(IntCC::UnsignedLessThan, x, b as i64)
            } else if b >= hi {
                f.ins().icmp_imm(IntCC::UnsignedGreaterThanOrEqual, x, a as i64)
            } else if b == a + 1 {
                f.ins().icmp_imm(IntCC::Equal, x, a as i64)
            } else {
                let d = f.ins().iadd_imm(x, -(a as i64));
                f.ins().icmp_imm(IntCC::UnsignedLessThan, d, (b - a) as i64)
            };
            f.ins().brif(inside, to, &[], default, &[]);
        }
        [Cluster::Bits(a, b, masks)] => {
            let (a, b) = (*a, *b);
            let d = if a == 0 { x } else { f.ins().iadd_imm(x, -(a as i64)) };
            if a > lo || b < hi {
                let tests = f.create_block();
                let inside = f.ins().icmp_imm(IntCC::UnsignedLessThan, d, (b - a) as i64);
                f.ins().brif(inside, tests, &[], default, &[]);
                f.switch_to_block(tests);
            }
            let one = f.ins().iconst(types::I64, 1);
            let bit = f.ins().ishl(one, d);
            // Every value of the cluster some target's: the last target needs no test.
            let full = (a.max(lo)..b.min(hi)).all(|v| masks.iter().any(|&(_, m)| m >> (v - a) & 1 != 0));
            for (k, &(to, m)) in masks.iter().enumerate() {
                if full && k + 1 == masks.len() {
                    f.ins().jump(to, &[]);
                    return;
                }
                let next = if k + 1 == masks.len() { default } else { f.create_block() };
                let t = f.ins().band_imm(bit, m as i64);
                f.ins().brif(t, to, &[], next, &[]);
                if next != default {
                    f.switch_to_block(next);
                }
            }
        }
        _ => {
            let mid = cases.len() / 2;
            let pivot = cases[mid].start();
            let (low, high) = (f.create_block(), f.create_block());
            let below = f.ins().icmp_imm(IntCC::UnsignedLessThan, x, pivot as i64);
            f.ins().brif(below, low, &[], high, &[]);
            f.switch_to_block(low);
            cases_tree(f, x, lo, pivot, &cases[..mid], default);
            f.switch_to_block(high);
            cases_tree(f, x, pivot, hi, &cases[mid..], default);
        }
    }
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

/// Word `w` of a mask (`w` below its count).
fn mask_word(f: &mut FunctionBuilder<'_>, first: Value, stride: Value, w: Value) -> Value {
    let off = f.ins().imul(w, stride);
    let at = f.ins().iadd(first, off);
    f.ins().load(types::I64, MemFlags::trusted().with_readonly(), at, 0)
}

/// A place past every word's: a switch's kind that it switches on by word, `BY_WORD` and the
/// kind (below `REFUSE`).
const BY_WORD: u16 = 0xfe00;

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


/// A function of tier 2's own, made from the same program and called once per file: `build`
/// makes its body from its `params` words and gives its heads. Only the token loop is inlined;
/// the node table's passes run once, at `ACCEPT`, and would only crowd it. Checked: it calls
/// nothing, and each loop is a stated one.
///
/// `pinned`: its first parameter is `ctx`, in the pinned register (`pinned_ctx`): not an
/// argument, read where it is entered; its callers pass the rest (`call_word`).
fn outline(module: &mut JITModule, name: &str, params: usize, returns: usize, pinned: bool, own: &[cranelift_codegen::ir::UserExternalName], build: impl FnOnce(&mut FunctionBuilder<'_>, &[Value], &mut Vec<(Block, Step)>)) -> Result<(cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
    let mut signature = module.make_signature();
    for _ in 0..params - usize::from(pinned) {
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
        let mut p = f.block_params(entry).to_vec();
        if pinned {
            let ctx = f.ins().get_pinned_reg(types::I64);
            p.insert(0, ctx);
        }
        build(&mut f, &p, &mut heads);
        f.seal_all_blocks();
        f.finalize();
    }
    crate::native_cpu_shape::check(&func, &heads, own).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
    Ok((id, signature, func))
}

/// Where an outlined word reads its token's end (`Lowering::token_end`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum EndAt {
    /// After its `SPAN`: its record's.
    Taken,
    /// Before: its start, or at the stream's end the last record's.
    Untaken,
    /// A program whose `TOKEN` has both phases: the field.
    Field,
}

/// A word made as a function of its own (`outline`), which every place of it calls: the lean
/// parser's helpers (`push`, `bump_as`), called from the grammar's functions. The state is in
/// `ctx` (`Ctx::state`), as the lean parser's in `self`. Its kind or field is an argument.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum WordFn {
    /// `by_word`: a keyword's leaf, its kind its token's word's (`WORD_KIND`).
    Leaf { deferred: bool, extra: bool, end: EndAt, by_word: bool },
    Open,
    Close { this: bool, end: EndAt },
    Wrap,
    /// `RETAG m=1`: the node the last wrap moved.
    RetagMoved { remembered: bool },
    /// `RETAG m=2`, `NODE_FIELD m=2`: the node made last at the level.
    Made { retag: bool, remembered: bool, dead: bool, unset: bool },
    /// `RETAG`: the innermost open node.
    Here,
    /// `MERGE m=1`.
    Fold,
    /// `REMEMBER #WORD_KIND`: the level's word, its token's word's kind.
    RememberWord,
    Merge { end: EndAt },
    /// `TOKEN`, taking a token: 0, or 1 past the stream's end, or `OVERFLOW` out of room.
    /// `extras`: the extras taken too (`TOKEN_EXTRAS`).
    Token { any_mark: Option<usize>, extras: bool },
    /// A token mark's tokens (`Words::token_marks`, its index): 1 where they follow, else 0.
    TokenMark { index: u16 },
    /// A local check over the token's bytes (`Words::local_checks`, its index): 1 where it holds.
    Check { index: u16 },
}

/// What a word's body reads and writes, wherever it is made (`WordFn`): in its rule's function,
/// or in a function of its own that each place of the word calls.
struct WordEnv<'a> {
    get: &'a dyn Fn(&mut FunctionBuilder<'_>, Mem) -> Value,
    set: &'a dyn Fn(&mut FunctionBuilder<'_>, Mem, Value),
    /// A field of `ctx`: a column's address, a count.
    ctx: &'a dyn Fn(&mut FunctionBuilder<'_>, i32) -> Value,
    /// Refuse where the value is set, at the word's place; else go on in a new block.
    guard: &'a dyn Fn(&mut FunctionBuilder<'_>, Value),
}

/// A state field of `ctx`, loaded or stored, in a function of its own.
fn field_get(f: &mut FunctionBuilder<'_>, ctx: Value, k: Mem) -> Value {
    f.ins().load(types::I64, MemFlags::trusted(), ctx, STATE + 8 * k as i32)
}

fn field_set(f: &mut FunctionBuilder<'_>, ctx: Value, k: Mem, x: Value) {
    f.ins().store(MemFlags::trusted(), x, ctx, STATE + 8 * k as i32);
}

/// A word's function (`WordFn`): params `ctx` and the word's arguments, its body, then 0 (where
/// it may refuse, 1 at a refusal, which its caller makes at its own place).
fn own_word(f: &mut FunctionBuilder<'_>, p: &[Value], refuses: bool, body: &dyn Fn(&mut FunctionBuilder<'_>, &WordEnv<'_>, &[Value])) {
    let ctx = p[0];
    let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
    let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
    let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
    let guard = |f: &mut FunctionBuilder<'_>, bad: Value| {
        let (ok, no) = (f.create_block(), f.create_block());
        f.ins().brif(bad, no, &[], ok, &[]);
        f.switch_to_block(no);
        f.set_cold_block(no);
        let one = f.ins().iconst(types::I64, 1);
        f.ins().return_(&[one]);
        f.switch_to_block(ok);
    };
    let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
    body(f, &env, &p[1..]);
    if refuses {
        let zero = f.ins().iconst(types::I64, 0);
        f.ins().return_(&[zero]);
    } else {
        f.ins().return_(&[]);
    }
}

/// Whether token mark `m`'s tokens follow at the token (`Words::token_marks`): each element's
/// record, the records' codes from the one before `Next`, the extras (`extra_bit`) skipped; 1 or
/// 0. Each turn of a skip is a record's: a stated `Token` loop.
/// A local check over the token's bytes (`CHECK`), word-parallel: the mask's words from the
/// token's start to its end (a stated loop, `Step::Block`); at a word with bits, the check's
/// value at each of its 64 places at once (`local_word`), and the test one AND. 1 where it holds
/// at a bit, else 0. No loop over bits or bytes. The token's start and end are passed: the
/// state may be the caller's variables.
fn check_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, c: &crate::program::LocalCheck, form: &crate::rust_stream::WordForm, output: usize, start: Value, limit: Value, heads: &mut Vec<(Block, Step)>) -> Value {
    let (outputs_p, stride, lasts_stride) = ((e.ctx)(f, OUTPUTS), (e.ctx)(f, OUTPUT_STRIDE), (e.ctx)(f, LASTS_STRIDE));
    let (bytes, len) = ((e.ctx)(f, BYTES), (e.ctx)(f, LEN));
    let consts = crate::native_cpu_block::Consts::make(f);
    let off = f.ins().imul_imm(stride, output as i64);
    let base = f.ins().iadd(outputs_p, off);
    let w0 = f.ins().ushr_imm(start, 6);
    let l1 = f.ins().iadd_imm(limit, -1);
    let wl = f.ins().ushr_imm(l1, 6);
    let (words, done) = (f.create_block(), f.create_block());
    f.append_block_param(words, types::I64);
    f.append_block_param(done, types::I64);
    f.ins().jump(words, &[BlockArg::from(w0)]);
    // A word of the mask, cut to the token's bytes.
    f.switch_to_block(words);
    heads.push((words, Step::Block));
    let w = f.block_params(words)[0];
    let past = f.ins().icmp(IntCC::UnsignedGreaterThan, w, wl);
    let (body, zero) = (f.create_block(), f.ins().iconst(types::I64, 0));
    f.ins().brif(past, done, &[BlockArg::from(zero)], body, &[]);
    f.switch_to_block(body);
    let word = mask_word(f, base, lasts_stride, w);
    let ones = f.ins().iconst(types::I64, -1);
    let s = f.ins().band_imm(start, 63);
    let lo = f.ins().ishl(ones, s);
    let first = f.ins().icmp(IntCC::Equal, w, w0);
    let lo = f.ins().select(first, lo, ones);
    let t = f.ins().band_imm(l1, 63);
    let t = f.ins().irsub_imm(t, 63);
    let hi = f.ins().ushr(ones, t);
    let last = f.ins().icmp(IntCC::Equal, w, wl);
    let hi = f.ins().select(last, hi, ones);
    let word = f.ins().band(word, lo);
    let a = f.ins().band(word, hi);
    let (look, next) = (f.create_block(), f.create_block());
    f.ins().brif(a, look, &[], next, &[]);
    // The check at the word's places: its value at place `64 w + b + delay`, bit `b`.
    f.switch_to_block(look);
    let mut env = LocalWords { form, bytes, len, w, consts: &consts, values: Default::default(), classes: Default::default(), tables: Default::default() };
    let top = c.ops.len() - 1;
    let (dq, dr) = (i64::from(c.delay) / 64, i64::from(c.delay) % 64);
    let l = local_word(f, &c.ops, &mut env, top, dq);
    let l = if dr == 0 {
        l
    } else {
        let after = local_word(f, &c.ops, &mut env, top, dq + 1);
        let x = f.ins().ushr_imm(l, dr);
        let y = f.ins().ishl_imm(after, 64 - dr);
        f.ins().bor(x, y)
    };
    let hit = f.ins().band(a, l);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().brif(hit, done, &[BlockArg::from(one)], next, &[]);
    f.switch_to_block(next);
    let w1 = f.ins().iadd_imm(w, 1);
    f.ins().jump(words, &[BlockArg::from(w1)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// What `local_word` reads: the check's classes, the file, the mask's word `w`; each op's value
/// at a word (by its distance from `w`), each word's classes and the tables, made once.
struct LocalWords<'a> {
    form: &'a crate::rust_stream::WordForm,
    bytes: Value,
    len: Value,
    w: Value,
    consts: &'a crate::native_cpu_block::Consts,
    values: std::collections::HashMap<(usize, i64), Value>,
    classes: std::collections::HashMap<(usize, i64), [Value; 4]>,
    tables: std::collections::HashMap<usize, (Value, Value)>,
}

/// The bits of word `q` whose places are in `lo..hi`.
fn within_word(f: &mut FunctionBuilder<'_>, q: Value, lo: Value, hi: Value) -> Value {
    let q64 = f.ins().ishl_imm(q, 6);
    let (zero, sixty_four) = (f.ins().iconst(types::I64, 0), f.ins().iconst(types::I64, 64));
    let clamp = |f: &mut FunctionBuilder<'_>, x: Value| {
        let x = f.ins().isub(x, q64);
        let x = f.ins().smax(x, zero);
        f.ins().smin(x, sixty_four)
    };
    let (a, b) = (clamp(f, lo), clamp(f, hi));
    let ones = f.ins().iconst(types::I64, -1);
    let one = f.ins().iconst(types::I64, 1);
    let below = f.ins().ishl(one, b);
    let below = f.ins().iadd_imm(below, -1);
    let full = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, b, 64);
    let below = f.ins().select(full, ones, below);
    let from = f.ins().ishl(ones, a);
    let none = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, a, 64);
    let from = f.ins().select(none, zero, from);
    f.ins().band(below, from)
}

/// Op `k` of a local check at word `w + o`, a bit a place (`program::local_value`): the byte
/// sets by tier 1's classes over the word's four vectors (`native_cpu_lanes`' lookup, then
/// `bitmask`), the rest word operations; a `Prev` reads the word before its operand's too.
/// Straight-line, each (op, word) once.
fn local_word(f: &mut FunctionBuilder<'_>, ops: &[crate::program::LocalOp], e: &mut LocalWords<'_>, k: usize, o: i64) -> Value {
    use crate::program::LocalOp as L;
    if let Some(&v) = e.values.get(&(k, o)) {
        return v;
    }
    let q = f.ins().iadd_imm(e.w, o);
    let v = match ops[k] {
        L::Bytes(_) => match e.form.class_of[&k] {
            (_, 0) => f.ins().iconst(types::I64, 0),
            (ti, bits) => {
                let c = local_classes(f, e, ti, o);
                let mask = {
                    let b = f.ins().iconst(types::I8, i64::from(bits));
                    f.ins().splat(types::I8X16, b)
                };
                let zero16 = {
                    let z = f.ins().iconst(types::I8, 0);
                    f.ins().splat(types::I8X16, z)
                };
                let parts: Vec<Value> = c
                    .iter()
                    .map(|&v| {
                        let x = f.ins().band(v, mask);
                        f.ins().icmp(IntCC::NotEqual, x, zero16)
                    })
                    .collect();
                let m = crate::native_cpu_block::bitmask(f, &parts, e.consts.weights);
                let zero = f.ins().iconst(types::I64, 0);
                let w = within_word(f, q, zero, e.len);
                f.ins().band(m, w)
            }
        },
        L::Const(p) => {
            let x = f.ins().iconst(types::I64, p as i64);
            let zero = f.ins().iconst(types::I64, 0);
            let neg = f.ins().icmp_imm(IntCC::SignedLessThan, q, 0);
            f.ins().select(neg, zero, x)
        }
        L::Within(d) => {
            let lo = f.ins().iconst(types::I64, i64::from(d));
            let hi = f.ins().iadd_imm(e.len, i64::from(d));
            within_word(f, q, lo, hi)
        }
        L::Not(a) => {
            let x = local_word(f, ops, e, usize::from(a), o);
            f.ins().bnot(x)
        }
        L::And(a, b) | L::Or(a, b) | L::Xor(a, b) | L::AndNot(a, b) => {
            let x = local_word(f, ops, e, usize::from(a), o);
            let y = local_word(f, ops, e, usize::from(b), o);
            match ops[k] {
                L::And(..) => f.ins().band(x, y),
                L::Or(..) => f.ins().bor(x, y),
                L::Xor(..) => f.ins().bxor(x, y),
                _ => f.ins().band_not(x, y),
            }
        }
        L::Prev(a, s) => {
            // Its operand `s` places before: this word's bits up, the word before's top bits in;
            // none before the file's start.
            let x = local_word(f, ops, e, usize::from(a), o);
            let y = local_word(f, ops, e, usize::from(a), o - 1);
            let x = f.ins().ishl_imm(x, i64::from(s));
            let y = f.ins().ushr_imm(y, 64 - i64::from(s));
            let zero = f.ins().iconst(types::I64, 0);
            let first = f.ins().icmp_imm(IntCC::SignedLessThan, q, 1);
            let y = f.ins().select(first, zero, y);
            f.ins().bor(x, y)
        }
    };
    e.values.insert((k, o), v);
    v
}

/// Table `ti`'s classes of the 64 bytes of word `w + o` (zeros outside the file): four vectors.
fn local_classes(f: &mut FunctionBuilder<'_>, e: &mut LocalWords<'_>, ti: usize, o: i64) -> [Value; 4] {
    if let Some(&c) = e.classes.get(&(ti, o)) {
        return c;
    }
    let (tlo, thi) = match e.tables.get(&ti) {
        Some(&t) => t,
        None => {
            let (lo, hi) = &e.form.tables[ti];
            let hl = f.func.dfg.constants.insert(cranelift_codegen::ir::ConstantData::from(&lo[..]));
            let hh = f.func.dfg.constants.insert(cranelift_codegen::ir::ConstantData::from(&hi[..]));
            let t = (f.ins().vconst(types::I8X16, hl), f.ins().vconst(types::I8X16, hh));
            e.tables.insert(ti, t);
            t
        }
    };
    // The word's address, where it is inside the file; else the file's first (readable: 64
    // bytes after its end), its classes then zeros.
    let q = f.ins().iadd_imm(e.w, o);
    let q64 = f.ins().ishl_imm(q, 6);
    let after = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, q, 0);
    let before_end = f.ins().icmp(IntCC::SignedLessThan, q64, e.len);
    let inside = f.ins().band(after, before_end);
    let at = f.ins().iadd(e.bytes, q64);
    let at = f.ins().select(inside, at, e.bytes);
    let zero16 = {
        let z = f.ins().iconst(types::I8, 0);
        f.ins().splat(types::I8X16, z)
    };
    let c = [0, 16, 32, 48].map(|off| {
        let v = f.ins().load(types::I8X16, MemFlags::trusted(), at, off);
        let lo = f.ins().band(v, e.consts.nibble);
        let hi = f.ins().ushr_imm(v, 4);
        let l = f.ins().swizzle(tlo, lo);
        let h = f.ins().swizzle(thi, hi);
        let x = f.ins().band(l, h);
        f.ins().select(inside, x, zero16)
    });
    e.classes.insert((ti, o), c);
    c
}

fn token_pattern(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, m: &crate::program::TokenMark, extra_bit: u8, heads: &mut Vec<(Block, Step)>) -> Value {
    let flags = MemFlags::trusted();
    let codes = (e.ctx)(f, CODES);
    let n = (e.ctx)(f, NSTARTS);
    let next = (e.get)(f, Mem::Next);
    let here = f.ins().iadd_imm(next, -1);
    let (no, yes) = (f.create_block(), f.create_block());
    let code_at = |f: &mut FunctionBuilder<'_>, j: Value| {
        let off = f.ins().ishl_imm(j, 2);
        let a = f.ins().iadd(codes, off);
        f.ins().uload32(flags, a, 0)
    };
    let shift = i64::from(crate::records::MARK_SHIFT) + i64::from(extra_bit);
    // The element's test on a code: a kind of its set, or a word of its.
    let test = |f: &mut FunctionBuilder<'_>, c: Value, el: &crate::program::TokenElem| {
        let ok = f.create_block();
        if el.kinds != 0 {
            let kind = f.ins().band_imm(c, 0xff);
            let small = f.ins().icmp_imm(IntCC::UnsignedLessThan, kind, 64);
            let set = f.ins().iconst(types::I64, el.kinds as i64);
            let bit = f.ins().ushr(set, kind);
            let bit = f.ins().band_imm(bit, 1);
            let zero = f.ins().iconst(types::I64, 0);
            let bit = f.ins().select(small, bit, zero);
            let other = f.create_block();
            f.ins().brif(bit, ok, &[], other, &[]);
            f.switch_to_block(other);
        }
        if !el.words.is_empty() {
            let w = f.ins().ushr_imm(c, i64::from(crate::records::KIND_BITS));
            let w = f.ins().band_imm(w, 0xff);
            for &s in &el.words {
                let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(s) + 1);
                let other = f.create_block();
                f.ins().brif(same, ok, &[], other, &[]);
                f.switch_to_block(other);
            }
        }
        f.ins().jump(no, &[]);
        f.switch_to_block(ok);
    };
    // The records after the token, in turn; the one before, for an element at -1.
    let mut at = here;
    for el in &m.elems {
        match el.offset {
            0 => {
                let c = code_at(f, here);
                test(f, c, el);
            }
            -1 => {
                let head = f.create_block();
                f.append_block_param(head, types::I64);
                f.ins().jump(head, &[BlockArg::from(here)]);
                f.switch_to_block(head);
                heads.push((head, Step::Token));
                let j = f.block_params(head)[0];
                let first = f.ins().icmp_imm(IntCC::Equal, j, 0);
                let on = f.create_block();
                f.ins().brif(first, no, &[], on, &[]);
                f.switch_to_block(on);
                let j = f.ins().iadd_imm(j, -1);
                let c = code_at(f, j);
                let x = f.ins().ushr_imm(c, shift);
                let x = f.ins().band_imm(x, 1);
                let found = f.create_block();
                f.ins().brif(x, head, &[BlockArg::from(j)], found, &[]);
                f.switch_to_block(found);
                test(f, c, el);
            }
            _ => {
                let head = f.create_block();
                f.append_block_param(head, types::I64);
                f.ins().jump(head, &[BlockArg::from(at)]);
                f.switch_to_block(head);
                heads.push((head, Step::Token));
                let j = f.block_params(head)[0];
                let j = f.ins().iadd_imm(j, 1);
                let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, n);
                let on = f.create_block();
                f.ins().brif(past, no, &[], on, &[]);
                f.switch_to_block(on);
                let c = code_at(f, j);
                let x = f.ins().ushr_imm(c, shift);
                let x = f.ins().band_imm(x, 1);
                let found = f.create_block();
                f.ins().brif(x, head, &[BlockArg::from(j)], found, &[]);
                f.switch_to_block(found);
                test(f, c, el);
                at = j;
            }
        }
    }
    f.ins().jump(yes, &[]);
    let done = f.create_block();
    f.append_block_param(done, types::I64);
    f.switch_to_block(no);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(done, &[BlockArg::from(zero)]);
    f.switch_to_block(yes);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().jump(done, &[BlockArg::from(one)]);
    f.switch_to_block(done);
    f.block_params(done)[0]
}

/// The token's end, as a word reads it (`EndAt`).
fn token_end_at(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, end_at: EndAt) -> Value {
    match end_at {
        EndAt::Taken => (e.get)(f, Mem::Limit),
        EndAt::Field => (e.get)(f, Mem::End),
        EndAt::Untaken => {
            let (at_end, limit, start) = ((e.get)(f, Mem::AtEnd), (e.get)(f, Mem::Limit), (e.get)(f, Mem::Start));
            f.ins().select(at_end, limit, start)
        }
    }
}

/// A u32 column's slot `i`, loaded or stored.
fn slot_get(f: &mut FunctionBuilder<'_>, c: Value, i: Value) -> Value {
    let off = f.ins().ishl_imm(i, 2);
    let a = f.ins().iadd(c, off);
    f.ins().uload32(MemFlags::trusted(), a, 0)
}

fn slot_put(f: &mut FunctionBuilder<'_>, c: Value, i: Value, x: Value) {
    let off = f.ins().ishl_imm(i, 2);
    let a = f.ins().iadd(c, off);
    f.ins().istore32(MemFlags::trusted(), x, a, 0);
}

/// The kind a word names as its level's word (`REMEMBERED`): refused where the level has none.
fn remembered_kind(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>) -> Value {
    let w = (e.get)(f, Mem::Word);
    let none = f.ins().icmp_imm(IntCC::Equal, w, 0);
    (e.guard)(f, none);
    w
}

/// A keyword's leaf kind (`WORD_KIND`): its token's word's (`Words::word_kinds`), refused where
/// the word has none. As LLVM lowers a `match` that maps a value to a value: runs of strings
/// whose kinds rise with them are one add each (`Kind::DotDecl + (d - W_DECL)`), a compare
/// tree picks the run.
fn word_kind(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, word_kinds: &[(u16, u16)]) -> Value {
    let code = (e.get)(f, Mem::Taken);
    let w = f.ins().ushr_imm(code, i64::from(crate::records::KIND_BITS));
    let w = f.ins().band_imm(w, 0xff);
    // The word's string: the word less one (0, no word, wraps past every string).
    let s = f.ins().iadd_imm(w, -1);
    // Runs: (first string, past the last, kind less string).
    let mut runs: Vec<(u32, u32, i64)> = Vec::new();
    for &(at, k) in word_kinds {
        let delta = i64::from(k) - i64::from(at);
        match runs.last_mut() {
            Some(r) if r.1 == u32::from(at) && r.2 == delta => r.1 += 1,
            _ => runs.push((u32::from(at), u32::from(at) + 1, delta)),
        }
    }
    let join = f.create_block();
    f.append_block_param(join, types::I64);
    let none = f.create_block();
    let mut cases: Vec<(u32, Block)> = Vec::new();
    let mut made = Vec::new();
    let mut at = 0;
    for &(a, b, delta) in &runs {
        if a > at {
            cases.push((at, none));
        }
        let blk = f.create_block();
        cases.push((a, blk));
        made.push((blk, delta));
        at = b;
    }
    cases.push((at, none));
    dispatch(f, s, &cases, u64::MAX);
    for (blk, delta) in made {
        f.switch_to_block(blk);
        let k = f.ins().iadd_imm(s, delta);
        f.ins().jump(join, &[k.into()]);
    }
    f.switch_to_block(none);
    let bad = f.ins().iconst(types::I8, 1);
    (e.guard)(f, bad);
    let zero = f.ins().iconst(types::I64, 0);
    f.ins().jump(join, &[zero.into()]);
    f.switch_to_block(join);
    f.block_params(join)[0]
}

/// What the node words need to write the tape as it stands at the end (`TableFns`): whether
/// rows carry marks (`head`, `rust_stream::marks`: a wrap, `WRAPPED`; a dropped row taken out
/// from among its children, `DROPPED_IN`), the tape's widths, the functions.
#[derive(Clone, Copy)]
struct Table<'a> {
    inner: bool,
    w: rerail_tape_format::Widths,
    fns: &'a TableFns,
}

const WRAPPED: i64 = 1;
const DROPPED_IN: i64 = 2;

impl Table<'_> {
    /// A kind's change on row `at`: its key's kind (16 bits).
    fn retag(&self, f: &mut FunctionBuilder<'_>, rows: Value, at: Value, kind: Value) {
        let row = NodeCol::row(f, rows, at);
        f.ins().istore16(MemFlags::trusted(), kind, row, AT_KIND);
    }

    /// Node `node`'s offset, at its push with start `start`: from its block's base, which it sets
    /// where it starts a block; where it starts before the base or 0x10000 past it, the block's
    /// base and offsets again (`blocks`). `n`: the nodes, it counted.
    fn push_off(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, node: Value, start: Value, n: Value) {
        let off_c = (e.ctx)(f, OFF_C);
        let low = f.ins().band_imm(node, rerail_tape_format::CHECKPOINT_STRIDE as i64 - 1);
        let base = (e.get)(f, Mem::BlockBase);
        let off = f.ins().isub(start, base);
        let off = f.ins().band_imm(off, 0xffff_ffff);
        let far = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, off, 0xffff);
        let (first, other, near, again, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block());
        f.ins().brif(low, other, &[], first, &[]);
        // The block's first node: its start is the base.
        f.switch_to_block(first);
        let base_c = (e.ctx)(f, BASE_C);
        let b = f.ins().ushr_imm(node, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
        slot_put(f, base_c, b, start);
        (e.set)(f, Mem::BlockBase, start);
        let zero = f.ins().iconst(types::I64, 0);
        crate::native_cpu_tape::put(f, off_c, node, zero, self.w.off);
        f.ins().jump(done, &[]);
        f.switch_to_block(other);
        f.ins().brif(far, again, &[], near, &[]);
        f.switch_to_block(near);
        crate::native_cpu_tape::put(f, off_c, node, off, self.w.off);
        f.ins().jump(done, &[]);
        f.switch_to_block(again);
        f.set_cold_block(again);
        let b = f.ins().ushr_imm(node, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
        self.blocks(f, e, b, n);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Blocks `b` on, of `n` nodes: their bases and offsets again (`native_cpu_tape::blocks_from`),
    /// the last base kept, a wide offset flagged.
    fn blocks(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, b: Value, n: Value) {
        let (rows, base_c, off_c) = ((e.ctx)(f, ROWS), (e.ctx)(f, BASE_C), (e.ctx)(f, OFF_C));
        let (id, sig, _) = &self.fns.blocks;
        let r = call_own(f, *id, sig, &[rows, base_c, off_c, b, n]);
        (e.set)(f, Mem::BlockBase, r[0]);
        let wide = (e.get)(f, Mem::Wide);
        let wide = f.ins().bor(wide, r[1]);
        (e.set)(f, Mem::Wide, wide);
    }

    /// The table cut back to `n` nodes (a fold, a merge, a drop): its last block's base again,
    /// where a block is left part full.
    fn cut_to(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, n: Value) {
        let low = f.ins().band_imm(n, rerail_tape_format::CHECKPOINT_STRIDE as i64 - 1);
        let (part, done) = (f.create_block(), f.create_block());
        f.ins().brif(low, part, &[], done, &[]);
        f.switch_to_block(part);
        let n1 = f.ins().iadd_imm(n, -1);
        let b = f.ins().ushr_imm(n1, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
        self.blocks(f, e, b, n);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Node `at` closed: its size, and its tail from its last descendant (`last`, a row) to
    /// `end`; a tail that does not fit 2 bytes (or wraps: an end before its last descendant's
    /// start) flagged.
    fn close_cells(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, at: Value, size: Value, last: Value, end: Value) {
        let rows = (e.ctx)(f, ROWS);
        let from = if self.w.from_start { at } else { last };
        let s = crate::native_cpu_tape::start(f, rows, from);
        let tail = f.ins().isub(end, s);
        let tail = f.ins().band_imm(tail, 0xffff_ffff);
        let (size_c, tail_c) = ((e.ctx)(f, SIZE_C), (e.ctx)(f, TAIL_C));
        crate::native_cpu_tape::put(f, size_c, at, size, self.w.size);
        crate::native_cpu_tape::put(f, tail_c, at, tail, self.w.tail);
        let mut odd = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, tail, if self.w.tail == 2 { 0xffff } else { 0x7fff_ffff });
        if self.w.from_start {
            let ls = crate::native_cpu_tape::start(f, rows, last);
            let early = f.ins().icmp(IntCC::UnsignedLessThan, end, ls);
            odd = f.ins().bor(odd, early);
        }
        let (flag, done) = (f.create_block(), f.create_block());
        f.ins().brif(odd, flag, &[], done, &[]);
        f.switch_to_block(flag);
        f.set_cold_block(flag);
        let wide = (e.get)(f, Mem::Wide);
        let wide = f.ins().bor_imm(wide, crate::native_cpu_tape::WIDE_TAIL);
        (e.set)(f, Mem::Wide, wide);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }

    /// Nodes `from..to` final: their codes (`native_cpu_tape::commit`).
    fn commit(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, to: Value) {
        let from = (e.get)(f, Mem::Coded);
        let (rows, image, id_of, kf, first) = ((e.ctx)(f, ROWS), (e.ctx)(f, IMAGE), (e.ctx)(f, ID_OF), (e.ctx)(f, CODE_KF), (e.ctx)(f, CODE_FIRST));
        let code = f.ins().iadd_imm(image, rerail_tape_format::HEADER_BYTES as i64);
        let codes = (e.get)(f, Mem::Codes);
        let (id, sig, _) = &self.fns.commit;
        let r = call_own(f, *id, sig, &[rows, code, id_of, kf, first, from, to, codes]);
        (e.set)(f, Mem::Codes, r[0]);
        (e.set)(f, Mem::Coded, to);
    }

    /// An edit of node `node`: where it is committed, the codes from it taken back
    /// (`native_cpu_tape::uncode`), numbered again at the next commit.
    fn uncode(&self, f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, node: Value) {
        let coded = (e.get)(f, Mem::Coded);
        let before = f.ins().icmp(IntCC::UnsignedLessThan, node, coded);
        let (back, done) = (f.create_block(), f.create_block());
        f.ins().brif(before, back, &[], done, &[]);
        f.switch_to_block(back);
        f.set_cold_block(back);
        let (id_of, kf, first) = ((e.ctx)(f, ID_OF), (e.ctx)(f, CODE_KF), (e.ctx)(f, CODE_FIRST));
        let codes = (e.get)(f, Mem::Codes);
        let (id, sig, _) = &self.fns.uncode;
        let r = call_own(f, *id, sig, &[id_of, kf, first, codes, node]);
        (e.set)(f, Mem::Codes, r[0]);
        (e.set)(f, Mem::Coded, node);
        f.ins().jump(done, &[]);
        f.switch_to_block(done);
    }
}

/// `LEAF`: its row (its key, its field above its kind or none; its bytes), its offset, its size
/// (1) and tail, the pending extras, the level's last node.
fn leaf_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, deferred: bool, extra: bool, end_at: EndAt, wrapping: bool) {
    let flags = MemFlags::trusted();
    let zero = f.ins().iconst(types::I64, 0);
    let pending = if deferred {
        // The first pending leaf keeps the end of the token before it.
        let (q, keep, last_end) = ((e.get)(f, Mem::Pending), (e.get)(f, Mem::Keep), (e.get)(f, Mem::LastEnd));
        let keep = f.ins().select(q, keep, last_end);
        (e.set)(f, Mem::Keep, keep);
        f.ins().iadd_imm(q, 1)
    } else {
        zero
    };
    (e.set)(f, Mem::Pending, pending);
    let (nodes, start, end) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Start), token_end_at(f, e, end_at));
    let rows = (e.ctx)(f, ROWS);
    let row = NodeCol::row(f, rows, nodes);
    f.ins().istore32(flags, kind, row, AT_KIND);
    NodeCol::of(rows, AT_START).put_row(f, row, start);
    NodeCol::of(rows, AT_END).put_row(f, row, end);
    let n1 = f.ins().iadd_imm(nodes, 1);
    t.push_off(f, e, nodes, start, n1);
    let one = f.ins().iconst(types::I64, 1);
    t.close_cells(f, e, nodes, one, nodes, end);
    if t.inner {
        let head = (e.ctx)(f, HEAD);
        slot_put(f, head, nodes, zero);
    }
    if wrapping && !deferred && !extra {
        // The last node made at the innermost open node's level.
        (e.set)(f, Mem::LastMade, n1);
    }
    (e.set)(f, Mem::Nodes, n1);
}

/// `OPEN`: its row (its key, its field above its kind or none; its start), its offset; open.
fn open_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, wrapping: bool) {
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::Pending, zero);
    let (nodes, start, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Start), (e.get)(f, Mem::Open));
    let rows = (e.ctx)(f, ROWS);
    let row = NodeCol::row(f, rows, nodes);
    f.ins().istore32(MemFlags::trusted(), kind, row, AT_KIND);
    NodeCol::of(rows, AT_START).put_row(f, row, start);
    let n1 = f.ins().iadd_imm(nodes, 1);
    t.push_off(f, e, nodes, start, n1);
    let open_c = (e.ctx)(f, OPEN);
    slot_put(f, open_c, open, nodes);
    if t.inner {
        let head = (e.ctx)(f, HEAD);
        slot_put(f, head, nodes, zero);
    }
    if wrapping {
        // No node made at its level yet.
        (e.set)(f, Mem::LastMade, zero);
    }
    let open = f.ins().iadd_imm(open, 1);
    (e.set)(f, Mem::Open, open);
    (e.set)(f, Mem::Nodes, n1);
}

/// `CLOSE`: the innermost open node's end (`this`: the token's; else the last taken's, or
/// the one before the pending extras), size and tail; the pending extras its parent's (they
/// follow its subtree); with wraps, it is the last node made at its parent's level. A node
/// closed at the top level: the nodes before it are final, their codes committed.
fn close_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, this: bool, end_at: EndAt, wrapping: bool) {
    let open = (e.get)(f, Mem::Open);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let open = f.ins().iadd_imm(open, -1);
    (e.set)(f, Mem::Open, open);
    let open_c = (e.ctx)(f, OPEN);
    let at = slot_get(f, open_c, open);
    let (nodes, pending) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending));
    let end = if this {
        token_end_at(f, e, end_at)
    } else {
        let (keep, last_end) = ((e.get)(f, Mem::Keep), (e.get)(f, Mem::LastEnd));
        f.ins().select(pending, keep, last_end)
    };
    let rows = (e.ctx)(f, ROWS);
    let at_row = NodeCol::row(f, rows, at);
    NodeCol::of(rows, AT_END).put_row(f, at_row, end);
    if wrapping {
        let at1 = f.ins().iadd_imm(at, 1);
        (e.set)(f, Mem::LastMade, at1);
    }
    let kept = f.ins().isub(nodes, pending);
    let size = f.ins().isub(kept, at);
    let last = f.ins().iadd_imm(kept, -1);
    t.close_cells(f, e, at, size, last, end);
    let top = f.ins().icmp_imm(IntCC::Equal, open, 1);
    let (commit, done) = (f.create_block(), f.create_block());
    f.ins().brif(top, commit, &[], done, &[]);
    f.switch_to_block(commit);
    t.commit(f, e, at);
    f.ins().jump(done, &[]);
    f.switch_to_block(done);
}

/// `WRAP`: a node before the last node made at the innermost open node's level, its field that
/// one's; open around it. That node and the rest of the table (the pending extras) move up a
/// cell: one row and nothing else where it is a leaf no supertype names, else `shift`, then
/// the blocks from it again.
fn wrap_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value) {
    let flags = MemFlags::trusted();
    let zero = f.ins().iconst(types::I64, 0);
    let open = (e.get)(f, Mem::Open);
    let last = (e.get)(f, Mem::LastMade);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, none);
    (e.guard)(f, no_last);
    let r = f.ins().iadd_imm(last, -1);
    t.uncode(f, e, r);
    let (nodes, supers) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Supers));
    let n1 = f.ins().iadd_imm(nodes, 1);
    let rows = (e.ctx)(f, ROWS);
    let head = if t.inner { (e.ctx)(f, HEAD) } else { zero };
    let snode = (e.ctx)(f, SUPER_NODE);
    let (size_c, tail_c) = ((e.ctx)(f, SIZE_C), (e.ctx)(f, TAIL_C));
    // One row: the leaf at the end, no supertype on it.
    let tail = f.ins().isub(nodes, r);
    let one = f.ins().icmp_imm(IntCC::Equal, tail, 1);
    let any = f.ins().icmp_imm(IntCC::NotEqual, supers, 0);
    let s1 = f.ins().iadd_imm(supers, -1);
    let s1 = f.ins().select(any, s1, zero);
    let ls = slot_get(f, snode, s1);
    let named = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, ls, r);
    let named = f.ins().band(any, named);
    let (swap, long, placed) = (f.create_block(), f.create_block(), f.create_block());
    let plain = f.ins().band_not(one, named);
    f.ins().brif(plain, swap, &[], long, &[]);
    f.switch_to_block(swap);
    let (src, dst) = (NodeCol::row(f, rows, r), NodeCol::row(f, rows, nodes));
    let w = row_words(f, src);
    put_row_words(f, dst, w);
    let size = crate::native_cpu_tape::get(f, size_c, r, t.w.size);
    crate::native_cpu_tape::put(f, size_c, nodes, size, t.w.size);
    let tl = crate::native_cpu_tape::get(f, tail_c, r, t.w.tail);
    crate::native_cpu_tape::put(f, tail_c, nodes, tl, t.w.tail);
    let st = f.ins().uload32(flags, dst, AT_START);
    t.push_off(f, e, nodes, st, n1);
    if t.inner {
        let h = slot_get(f, head, r);
        slot_put(f, head, nodes, h);
    }
    f.ins().jump(placed, &[]);
    f.switch_to_block(long);
    f.set_cold_block(long);
    let (id, sig, _) = t.fns.shift.as_ref().expect("a program that wraps has its shift");
    call_own(f, *id, sig, &[rows, head, snode, r, nodes, supers, size_c, tail_c]);
    let b = f.ins().ushr_imm(r, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
    t.blocks(f, e, b, n1);
    f.ins().jump(placed, &[]);
    f.switch_to_block(placed);
    // The wrap at r (its start the wrapped node's: r's offset holds), the wrapped node after it.
    let (to, from) = (NodeCol::row(f, rows, r), NodeCol::row(f, rows, last));
    let fld = f.ins().uload16(types::I64, flags, from, AT_FIELD);
    let kf = f.ins().ishl_imm(fld, 16);
    let kf = f.ins().bor(kf, kind);
    f.ins().istore32(flags, kf, to, AT_KIND);
    f.ins().istore16(flags, zero, from, AT_FIELD);
    if t.inner {
        let mark = f.ins().iconst(types::I64, WRAPPED);
        slot_put(f, head, r, mark);
    }
    let open_c = (e.ctx)(f, OPEN);
    slot_put(f, open_c, open, r);
    let made = f.ins().iadd_imm(r, 2);
    (e.set)(f, Mem::LastMade, made);
    (e.set)(f, Mem::Moved, last);
    (e.set)(f, Mem::Pending, zero);
    let open1 = f.ins().iadd_imm(open, 1);
    (e.set)(f, Mem::Open, open1);
    (e.set)(f, Mem::Nodes, n1);
}

/// A dropped node's rows taken out from row `from` on (`drop`), then the tails of the nodes
/// left there and the blocks from there again.
fn drop_from(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, from: Value) {
    let zero = f.ins().iconst(types::I64, 0);
    t.uncode(f, e, from);
    let rows = (e.ctx)(f, ROWS);
    let (before, to, stack) = ((e.ctx)(f, FRAME_BASE), (e.ctx)(f, LINK), (e.ctx)(f, PLACE));
    let head = if t.inner { (e.ctx)(f, HEAD) } else { zero };
    let (snode, skind) = ((e.ctx)(f, SUPER_NODE), (e.ctx)(f, SUPER_KIND));
    let (size_c, tail_c) = ((e.ctx)(f, SIZE_C), (e.ctx)(f, TAIL_C));
    let (nodes, supers, moved) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Supers), (e.get)(f, Mem::Moved));
    // The innermost open node: a dropped row's parent where no row of the range holds it.
    let open = (e.get)(f, Mem::Open);
    let o1 = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let outer = slot_get(f, open_c, o1);
    let (id, sig, _) = t.fns.drop.as_ref().expect("a program that drops has its drop");
    let r = call_own(f, *id, sig, &[rows, before, to, head, snode, skind, from, nodes, supers, moved, size_c, tail_c, stack, outer]);
    (e.set)(f, Mem::Nodes, r[0]);
    (e.set)(f, Mem::Supers, r[1]);
    (e.set)(f, Mem::Moved, r[2]);
    let wide = (e.get)(f, Mem::Wide);
    let wide = f.ins().bor(wide, r[3]);
    (e.set)(f, Mem::Wide, wide);
    let b = f.ins().ushr_imm(from, rerail_tape_format::CHECKPOINT_STRIDE.trailing_zeros() as i64);
    t.blocks(f, e, b, r[0]);
}

/// `RETAG m=2`, `NODE_FIELD m=2`: the node made last at the innermost open node's level
/// (`relabel`), its frame's last one more than its record (0: none), its kind or field `x`.
fn made_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, x: Value, retag: bool, dead: bool, unset: bool) {
    let open = (e.get)(f, Mem::Open);
    let last = (e.get)(f, Mem::LastMade);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, none);
    (e.guard)(f, no_last);
    let r = f.ins().iadd_imm(last, -1);
    let rows = (e.ctx)(f, ROWS);
    if retag {
        t.retag(f, rows, r, x);
    } else {
        let col = NodeCol::of(rows, AT_FIELD);
        // A call's field (`On::MadeUnset`): the node's own, where it has one.
        let x = if unset {
            let had = col.get(f, r);
            f.ins().select(had, had, x)
        } else {
            x
        };
        col.put(f, r, x);
    }
    if dead {
        // `drop`: the node goes (a token the grammar implies), taken out now; none is made last
        // at this level until the next.
        let zero = f.ins().iconst(types::I64, 0);
        (e.set)(f, Mem::LastMade, zero);
        drop_from(f, e, t, r);
    }
}

/// `RETAG`: the innermost open node's kind.
fn here_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value) {
    let open = (e.get)(f, Mem::Open);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let top = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let at = slot_get(f, open_c, top);
    let rows = (e.ctx)(f, ROWS);
    t.retag(f, rows, at, kind);
}

/// `MERGE m=1`: the innermost open node holds two leaves and nothing else, no extra pending:
/// the first becomes one leaf through the second's end, closed again.
fn fold_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, wrapping: bool) {
    let (nodes, pending, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending), (e.get)(f, Mem::Open));
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let top = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let o = slot_get(f, open_c, top);
    let want = f.ins().iadd_imm(o, 3);
    let count = f.ins().icmp(IntCC::NotEqual, nodes, want);
    let waiting = f.ins().icmp_imm(IntCC::NotEqual, pending, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, count);
    (e.guard)(f, waiting);
    let (a, b) = (f.ins().iadd_imm(o, 1), f.ins().iadd_imm(o, 2));
    let rows = (e.ctx)(f, ROWS);
    let size_c = (e.ctx)(f, SIZE_C);
    let end_c = NodeCol::of(rows, AT_END);
    let (ra, rb) = (NodeCol::row(f, rows, a), NodeCol::row(f, rows, b));
    let (sa, sb) = (crate::native_cpu_tape::get(f, size_c, a, t.w.size), crate::native_cpu_tape::get(f, size_c, b, t.w.size));
    let na = f.ins().icmp_imm(IntCC::NotEqual, sa, 1);
    let nb = f.ins().icmp_imm(IntCC::NotEqual, sb, 1);
    (e.guard)(f, na);
    (e.guard)(f, nb);
    let end = end_c.get_row(f, rb);
    t.retag(f, rows, a, kind);
    end_c.put_row(f, ra, end);
    let one = f.ins().iconst(types::I64, 1);
    t.close_cells(f, e, a, one, a, end);
    (e.set)(f, Mem::Nodes, b);
    t.cut_to(f, e, b);
    if wrapping {
        // The first is the last node made at this level now.
        (e.set)(f, Mem::LastMade, b);
    }
}

/// `MERGE`: the grammar's leaf before the pending extras, a leaf inside the innermost open
/// node; the extras are its text: dropped. The leaf closed again at its new end.
fn merge_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, t: Table<'_>, kind: Value, end_at: EndAt) {
    let zero = f.ins().iconst(types::I64, 0);
    let (nodes, pending, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending), (e.get)(f, Mem::Open));
    let kept = f.ins().isub(nodes, pending);
    let none = f.ins().icmp_imm(IntCC::Equal, kept, 0);
    (e.guard)(f, none);
    let leaf = f.ins().iadd_imm(kept, -1);
    let rows = (e.ctx)(f, ROWS);
    let size_c = (e.ctx)(f, SIZE_C);
    let end_c = NodeCol::of(rows, AT_END);
    let leaf_row = NodeCol::row(f, rows, leaf);
    let size = crate::native_cpu_tape::get(f, size_c, leaf, t.w.size);
    let not_leaf = f.ins().icmp_imm(IntCC::NotEqual, size, 1);
    (e.guard)(f, not_leaf);
    let any = f.ins().icmp_imm(IntCC::NotEqual, open, 0);
    let top = f.ins().iadd_imm(open, -1);
    let top = f.ins().select(any, top, zero);
    let open_c = (e.ctx)(f, OPEN);
    let o = slot_get(f, open_c, top);
    let inside = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, o, leaf);
    let bad = f.ins().band(any, inside);
    (e.guard)(f, bad);
    let end = token_end_at(f, e, end_at);
    t.retag(f, rows, leaf, kind);
    end_c.put_row(f, leaf_row, end);
    let one = f.ins().iconst(types::I64, 1);
    t.close_cells(f, e, leaf, one, leaf, end);
    (e.set)(f, Mem::Nodes, kept);
    (e.set)(f, Mem::Pending, zero);
    // The extras taken back: where there were any, the last block's base again.
    let (cut, done) = (f.create_block(), f.create_block());
    f.ins().brif(pending, cut, &[], done, &[]);
    f.switch_to_block(cut);
    t.cut_to(f, e, kept);
    f.ins().jump(done, &[]);
    f.switch_to_block(done);
}

/// What `TOKEN` reads besides `ctx`: the room each token's path may need, and the marks.
#[derive(Clone, Copy)]
struct TokenShape {
    bound: Bounds,
    in_code: usize,
    rest_bits: i64,
    end_bits: i64,
    /// The stream's end as a token's code (`end_kind`), where it has one.
    end_code: Option<i64>,
    /// The calls open counted by the machine's stack (`calls_on_stack`): no argument for them,
    /// and no room for returns (none is written).
    on_stack: bool,
    /// The extra this `TOKEN` takes (`TOKEN_EXTRAS`): its code's mark bit and its leaf's kind.
    extra: Option<(u8, u16)>,
    /// The program has wraps (`framed`): a leaf's frame slot too.
    wrapping: bool,
}

/// `TOKEN`'s body (`WordFn::Token`), taking a token: params `ctx`, the calls open. The room for
/// the path to the next token, then the next record, or the stream's end (its own kind's code). 0, or 1 where the stream had ended
/// already, or `OVERFLOW` out of room.
fn take_token(f: &mut FunctionBuilder<'_>, p: &[Value], any_mark: Option<usize>, t: TokenShape, table: Table<'_>, heads: &mut Vec<(Block, Step)>) {
    let ctx = p[0];
    let flags = MemFlags::trusted();
    let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, flags, ctx, at);
    let pops = t.bound.pop_nodes > 0 || t.bound.pop_supers > 0;
    let open_calls = if !t.on_stack { p[1] } else if pops { calls_open_by_stack(f, ctx) } else { f.ins().iconst(types::I64, 0) };
    // Each token from here: an extra taken (`TOKEN_EXTRAS`) comes back for the next.
    let top = f.create_block();
    f.ins().jump(top, &[]);
    f.switch_to_block(top);
    heads.push((top, Step::Token));
    let over = f.create_block();
    let mut counts = vec![field_get(f, ctx, Mem::Nodes), field_get(f, ctx, Mem::Supers)];
    let mut rooms = vec![load(f, ROOM_NODES), load(f, ROOM_SUPERS)];
    let mut most = vec![(t.bound.nodes, t.bound.pop_nodes), (t.bound.supers, t.bound.pop_supers)];
    if !t.on_stack {
        counts.push(open_calls);
        rooms.push(load(f, ROOM_PUSHES));
        most.push((t.bound.pushes, 0));
    }
    for (at, (room, (most, per_call))) in counts.into_iter().zip(rooms.into_iter().zip(most)) {
        let at = if per_call > 0 {
            let popped = f.ins().imul_imm(open_calls, per_call as i64);
            f.ins().iadd(at, popped)
        } else {
            at
        };
        let reach = f.ins().iadd_imm(at, most as i64);
        let past = f.ins().icmp(IntCC::UnsignedGreaterThan, reach, room);
        let fits = f.create_block();
        f.ins().brif(past, over, &[], fits, &[]);
        f.switch_to_block(fits);
    }
    let fits = f.current_block().expect("in a block");
    f.switch_to_block(over);
    f.set_cold_block(over);
    let overflow = f.ins().iconst(types::I64, OVERFLOW as i64);
    f.ins().return_(&[overflow]);
    f.switch_to_block(fits);
    let (ended, go) = (f.create_block(), f.create_block());
    let at_end = field_get(f, ctx, Mem::AtEnd);
    f.ins().brif(at_end, ended, &[], go, &[]);
    f.switch_to_block(ended);
    f.set_cold_block(ended);
    let one = f.ins().iconst(types::I64, 1);
    f.ins().return_(&[one]);
    f.switch_to_block(go);
    let (next, end) = (field_get(f, ctx, Mem::Next), field_get(f, ctx, Mem::Limit));
    field_set(f, ctx, Mem::LastEnd, end);
    let zero = f.ins().iconst(types::I64, 0);
    let nstarts = load(f, NSTARTS);
    let is_end = f.ins().icmp(IntCC::Equal, next, nstarts);
    let (last, token) = (f.create_block(), f.create_block());
    f.ins().brif(is_end, last, &[], token, &[]);
    f.switch_to_block(last);
    f.set_cold_block(last);
    let one = f.ins().iconst(types::I64, 1);
    field_set(f, ctx, Mem::AtEnd, one);
    let at_end_live = f.ins().iconst(types::I64, t.end_bits);
    field_set(f, ctx, Mem::Live, at_end_live);
    if let Some(code) = t.end_code {
        let code = f.ins().iconst(types::I64, code);
        field_set(f, ctx, Mem::Taken, code);
    }
    f.ins().return_(&[zero]);
    // The token's record: its first byte, last byte and code, from three columns.
    f.switch_to_block(token);
    let off = f.ins().ishl_imm(next, 2);
    let column = |f: &mut FunctionBuilder<'_>, at: i32| {
        let c = load(f, at);
        let a = f.ins().iadd(c, off);
        f.ins().uload32(flags, a, 0)
    };
    let (s, l, c) = (column(f, STARTS), column(f, LASTS), column(f, CODES));
    let l1 = f.ins().iadd_imm(l, 1);
    let after = f.ins().iadd_imm(next, 1);
    field_set(f, ctx, Mem::Start, s);
    field_set(f, ctx, Mem::Next, after);
    field_set(f, ctx, Mem::Limit, l1);
    field_set(f, ctx, Mem::Taken, c);
    // An extra: its bytes taken and its leaf deferred (`SPAN m=1; LEAF m=1 #K`), then the next
    // token, as the lean parser places a comment where it passes it.
    if let Some((bit, kind)) = t.extra {
        let b = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(bit));
        let b = f.ins().band_imm(b, 1);
        let (extra, on) = (f.create_block(), f.create_block());
        f.ins().brif(b, extra, &[], on, &[]);
        f.switch_to_block(extra);
        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a leaf does not refuse");
        let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
        let k = f.ins().iconst(types::I64, i64::from(kind));
        leaf_word(f, &env, table, k, true, true, EndAt::Taken, t.wrapping);
        f.ins().jump(top, &[]);
        f.switch_to_block(on);
    }
    let live = match any_mark {
        _ if t.in_code > 0 => {
            let marks = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
            if t.rest_bits == 0 { marks } else { f.ins().bor_imm(marks, t.rest_bits) }
        }
        Some(output) => {
            let (outputs_p, output_stride, lasts_stride) = (load(f, OUTPUTS), load(f, OUTPUT_STRIDE), load(f, LASTS_STRIDE));
            let w = f.ins().ushr_imm(s, 6);
            let o = f.ins().imul_imm(output_stride, output as i64);
            let base = f.ins().iadd(outputs_p, o);
            let word = mask_word(f, base, lasts_stride, w);
            let sh = f.ins().band_imm(s, 63);
            let bit = f.ins().ushr(word, sh);
            let bit = f.ins().band_imm(bit, 1);
            f.ins().ineg(bit)
        }
        None => f.ins().iconst(types::I64, -1),
    };
    field_set(f, ctx, Mem::Live, live);
    f.ins().return_(&[zero]);
}

/// `TOKEN`'s function (`WordFn::Token`), made where first called.
fn token_fn<'a>(module: &mut JITModule, fns: &'a mut std::collections::HashMap<WordFn, (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)>, any_mark: Option<usize>, shape: TokenShape, table: Table<'_>, pinned: bool) -> Result<&'a (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
    let key = WordFn::Token { any_mark, extras: shape.extra.is_some() };
    if !fns.contains_key(&key) {
        let name = format!("tempo_word_{}", fns.len());
        let made = outline(module, &name, if shape.on_stack { 1 } else { 2 }, 1, pinned, &table.fns.names(), |g, q, heads| take_token(g, q, any_mark, shape, table, heads))?;
        fns.insert(key, made);
    }
    Ok(&fns[&key])
}

/// A call of a word's function (`outline`): `ctx`, then its arguments; with `ctx` pinned, the
/// arguments alone.
fn call_word(f: &mut FunctionBuilder<'_>, id: cranelift_module::FuncId, signature: &cranelift_codegen::ir::Signature, pinned: bool, ctx: Value, args: &[Value]) -> Vec<Value> {
    let all: Vec<Value> = (!pinned).then_some(ctx).into_iter().chain(args.iter().copied()).collect();
    call_own(f, id, signature, &all)
}

/// A call of an outlined function (`outline`), from `f`.
fn call_own(f: &mut FunctionBuilder<'_>, id: cranelift_module::FuncId, signature: &cranelift_codegen::ir::Signature, args: &[Value]) -> Vec<Value> {
    let name = f.func.declare_imported_user_function(cranelift_codegen::ir::UserExternalName::new(0, id.as_u32()));
    let signature = f.import_signature(signature.clone());
    let callee = f.import_function(cranelift_codegen::ir::ExtFuncData { name: cranelift_codegen::ir::ExternalName::user(name), signature, colocated: true });
    let call = f.ins().call(callee, args);
    f.inst_results(call).to_vec()
}

/// A function of tier 2's own (`outline`): its id, signature and body.
type OwnFn = (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function);

/// The functions the node words call to keep the table as it stands at the end, as the lean
/// parser keeps its tape (`rust_stream::node_words` has the same in Rust): each rare, each a
/// loop over the table's tail. `shift`: a wrap's (where it wraps more than a leaf, or a node a
/// supertype names); `unpend`: a close the pending extras outlive; `drop`: a dropped node's.
struct TableFns {
    shift: Option<OwnFn>,
    drop: Option<OwnFn>,
    /// The tape's (`native_cpu_tape`): a block's base and offsets again, a commit of code ids,
    /// codes taken back, the image's end.
    blocks: OwnFn,
    commit: OwnFn,
    uncode: OwnFn,
    finish: OwnFn,
}

impl TableFns {
    fn make(module: &mut JITModule, words: &[crate::native_ir::StructuralBlock], x: crate::native_cpu_tape::Image<'_>) -> Result<Self, Error> {
        let w = x.widths;
        let inner = words.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. } | Op::Reread { .. }));
        let wraps = words.iter().any(|b| matches!(b.operation, Op::Wrap { .. }));
        let drops = words.iter().any(|b| matches!(b.operation, Op::Reread { .. } | Op::Retag { kind: DEAD, on: On::Made }));
        let shift = if wraps { Some(outline(module, "tempo_stream_shift", 8, 0, false, &[], |f, p, heads| shift_rows(f, p, inner, w, heads))?) } else { None };
        let drop = if drops { Some(outline(module, "tempo_stream_drop", 14, 4, false, &[], |f, p, heads| drop_rows(f, p, inner, w, heads))?) } else { None };
        let blocks = outline(module, "tempo_tape_blocks", 5, 2, false, &[], |f, p, heads| crate::native_cpu_tape::blocks_from(f, p, w.off, heads))?;
        let commit = outline(module, "tempo_tape_commit", 8, 1, false, &[], crate::native_cpu_tape::commit)?;
        let uncode = outline(module, "tempo_tape_uncode", 5, 1, false, &[], crate::native_cpu_tape::uncode)?;
        let finish = outline(module, "tempo_tape_finish", 10, 1, false, &[], |f, p, heads| crate::native_cpu_tape::finish(f, p, x, heads))?;
        Ok(TableFns { shift, drop, blocks, commit, uncode, finish })
    }

    /// The functions' names: the node words' functions call them.
    fn names(&self) -> Vec<cranelift_codegen::ir::UserExternalName> {
        self.shift.iter().chain(&self.drop).chain([&self.blocks, &self.commit, &self.uncode, &self.finish]).map(|f| cranelift_codegen::ir::UserExternalName::new(0, f.0.as_u32())).collect()
    }

    fn all(self) -> impl Iterator<Item = OwnFn> {
        self.shift.into_iter().chain(self.drop).chain([self.blocks, self.commit, self.uncode, self.finish])
    }
}

/// A row's 12 bytes (its key and start, its end), loaded or stored.
fn row_words(f: &mut FunctionBuilder<'_>, row: Value) -> [Value; 2] {
    [f.ins().load(types::I64, MemFlags::trusted(), row, 0), f.ins().uload32(MemFlags::trusted(), row, 8)]
}

fn put_row_words(f: &mut FunctionBuilder<'_>, row: Value, w: [Value; 2]) {
    f.ins().store(MemFlags::trusted(), w[0], row, 0);
    f.ins().istore32(MemFlags::trusted(), w[1], row, 8);
}

/// `(rows, head, snode, r, n, supers, size, tail)`: a wrap at row `r` of a table of `n`: rows
/// `r..n` up a row, latest first, with their size and tail cells; `head` (which rows are wraps)
/// moves with them. The supertypes' nodes from `r` on, the last ones, one more. A step a row
/// (`Step::Node`). The caller sets the blocks from `r` again.
fn shift_rows(f: &mut FunctionBuilder<'_>, p: &[Value], inner: bool, w: rerail_tape_format::Widths, heads: &mut Vec<(Block, Step)>) {
    let (rows, head, snode, r, n, supers, size_c, tail_c) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
    let (step, body, sups, done) = (f.create_block(), f.create_block(), f.create_block(), f.create_block());
    f.append_block_param(step, types::I64);
    f.append_block_param(sups, types::I64);
    heads.extend([(step, Step::Node), (sups, Step::Node)]);
    f.ins().jump(step, &[BlockArg::from(n)]);
    // step(i): row i - 1 to row i.
    f.switch_to_block(step);
    let i = f.block_params(step)[0];
    let at_r = f.ins().icmp(IntCC::Equal, i, r);
    f.ins().brif(at_r, sups, &[BlockArg::from(supers)], body, &[]);
    f.switch_to_block(body);
    let from = f.ins().iadd_imm(i, -1);
    let (src, dst) = (NodeCol::row(f, rows, from), NodeCol::row(f, rows, i));
    let x = row_words(f, src);
    put_row_words(f, dst, x);
    for (col, width) in [(size_c, w.size), (tail_c, w.tail)] {
        let v = crate::native_cpu_tape::get(f, col, from, width);
        crate::native_cpu_tape::put(f, col, i, v, width);
    }
    if inner {
        let h = slot_get(f, head, from);
        slot_put(f, head, i, h);
    }
    f.ins().jump(step, &[BlockArg::from(from)]);
    // sups(k): the supertype k - 1, while its node is from r on.
    f.switch_to_block(sups);
    let k = f.block_params(sups)[0];
    let (look, bump) = (f.create_block(), f.create_block());
    f.ins().brif(k, look, &[], done, &[]);
    f.switch_to_block(look);
    let k1 = f.ins().iadd_imm(k, -1);
    let s = slot_get(f, snode, k1);
    let moved = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, r);
    f.ins().brif(moved, bump, &[], done, &[]);
    f.switch_to_block(bump);
    let s1 = f.ins().iadd_imm(s, 1);
    slot_put(f, snode, k1, s1);
    f.ins().jump(sups, &[BlockArg::from(k1)]);
    f.switch_to_block(done);
    f.ins().return_(&[]);
}

/// `(rows, before, to, head, snode, skind, from, n, supers, moved, size, tail, stack, outer)`: the
/// dropped rows (`DEAD`) of rows `from..n` taken out, the rest moved down. First each row's
/// count of dropped before it (`before`); then each kept row to its place, its size less the
/// dropped in its range (`to`: a kept row's place, a dropped one's parent's). A dropped row's
/// parent is the innermost of the rows before it whose range holds it: a stack of them
/// (`stack`, each its row and its range's end), or else `outer`, the innermost open node; it is
/// marked (`DROPPED_IN`). Then each kept
/// row's tail again, from its last descendant now; then the supertypes of the tail's nodes on
/// to their rows, a dropped node's gone. Gives back the rows, the supertypes, the node the last
/// wrap moved at its place, and `WIDE_TAIL` where a tail does not fit. A step a row.
fn drop_rows(f: &mut FunctionBuilder<'_>, p: &[Value], inner: bool, w: rerail_tape_format::Widths, heads: &mut Vec<(Block, Step)>) {
    let flags = MemFlags::trusted();
    let (rows, before, to, head, snode, skind, from, n, supers, moved, size_c, tail_c, stack, outer) = (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], p[9], p[10], p[11], p[12], p[13]);
    let zero = f.ins().iconst(types::I64, 0);
    let (count, cbody, place, pbody, pop, popped, kept_row, tails, tbody, back, sback, sstep, sbody, sput, done) = (
        f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(),
        f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(), f.create_block(),
    );
    // count(i, d); place(i, depth); pop(depth); tails(j, wide); back(k); sstep(s, kept); done(kept, wide).
    for (x, k) in [(count, 2), (place, 2), (pop, 1), (popped, 1), (tails, 2), (back, 2), (sback, 1), (sstep, 3), (sbody, 1), (sput, 1), (done, 2)] {
        for _ in 0..k {
            f.append_block_param(x, types::I64);
        }
    }
    heads.extend([(count, Step::Node), (place, Step::Node), (pop, Step::Node), (tails, Step::Node), (back, Step::Node), (sstep, Step::Node)]);
    f.ins().jump(count, &[BlockArg::from(from), BlockArg::from(zero)]);
    f.switch_to_block(count);
    let (i, d) = (f.block_params(count)[0], f.block_params(count)[1]);
    slot_put(f, before, i, d);
    let end = f.ins().icmp(IntCC::Equal, i, n);
    f.ins().brif(end, place, &[BlockArg::from(from), BlockArg::from(zero)], cbody, &[]);
    f.switch_to_block(cbody);
    let k = NodeCol::of(rows, AT_KIND).get(f, i);
    let dead = f.ins().icmp_imm(IntCC::Equal, k, i64::from(DEAD));
    let dead = f.ins().uextend(types::I64, dead);
    let d1 = f.ins().iadd(d, dead);
    let i1 = f.ins().iadd_imm(i, 1);
    f.ins().jump(count, &[BlockArg::from(i1), BlockArg::from(d1)]);
    // place(i, depth): row i; the stack holds `depth` rows whose ranges were open before it.
    f.switch_to_block(place);
    let (i, depth) = (f.block_params(place)[0], f.block_params(place)[1]);
    let end = f.ins().icmp(IntCC::Equal, i, n);
    let wide0 = zero;
    f.ins().brif(end, tails, &[BlockArg::from(from), BlockArg::from(wide0)], pbody, &[]);
    f.switch_to_block(pbody);
    f.ins().jump(pop, &[BlockArg::from(depth)]);
    // pop(depth): the rows whose ranges end at or before i, off the stack.
    f.switch_to_block(pop);
    let depth = f.block_params(pop)[0];
    let (look, off) = (f.create_block(), f.create_block());
    f.ins().brif(depth, look, &[], popped, &[BlockArg::from(depth)]);
    f.switch_to_block(look);
    let d1 = f.ins().iadd_imm(depth, -1);
    let e_at = f.ins().ishl_imm(d1, 1);
    let e_at = f.ins().iadd_imm(e_at, 1);
    let top_end = slot_get(f, stack, e_at);
    let ended = f.ins().icmp(IntCC::UnsignedLessThanOrEqual, top_end, i);
    f.ins().brif(ended, off, &[], popped, &[BlockArg::from(depth)]);
    f.switch_to_block(off);
    f.ins().jump(pop, &[BlockArg::from(d1)]);
    f.switch_to_block(popped);
    let depth = f.block_params(popped)[0];
    // The parent: the stack's top (its place, `to`), or else the innermost open node (`outer`,
    // before `from`).
    let has = f.ins().icmp_imm(IntCC::NotEqual, depth, 0);
    let d1 = f.ins().iadd_imm(depth, -1);
    let d1 = f.ins().select(has, d1, zero);
    let j_at = f.ins().ishl_imm(d1, 1);
    let pj = slot_get(f, stack, j_at);
    let pj = f.ins().select(has, pj, from);
    let mapped = slot_get(f, to, pj);
    let pr = f.ins().select(has, mapped, outer);
    let size = crate::native_cpu_tape::get(f, size_c, i, w.size);
    let e = f.ins().iadd(i, size);
    let e = f.ins().umin(e, n);
    // Row i on the stack, its range to `e`.
    let at = f.ins().ishl_imm(depth, 1);
    slot_put(f, stack, at, i);
    let at1 = f.ins().iadd_imm(at, 1);
    slot_put(f, stack, at1, e);
    let depth1 = f.ins().iadd_imm(depth, 1);
    let k = NodeCol::of(rows, AT_KIND).get(f, i);
    let dead = f.ins().icmp_imm(IntCC::Equal, k, i64::from(DEAD));
    let (gone, i1) = (f.create_block(), f.ins().iadd_imm(i, 1));
    f.ins().brif(dead, gone, &[], kept_row, &[]);
    f.switch_to_block(gone);
    slot_put(f, to, i, pr);
    if inner {
        let m = slot_get(f, head, pr);
        let m = f.ins().bor_imm(m, DROPPED_IN);
        slot_put(f, head, pr, m);
    }
    f.ins().jump(place, &[BlockArg::from(i1), BlockArg::from(depth1)]);
    f.switch_to_block(kept_row);
    let bi = slot_get(f, before, i);
    let j = f.ins().isub(i, bi);
    slot_put(f, to, i, j);
    let be = slot_get(f, before, e);
    let lost = f.ins().isub(be, bi);
    let size = f.ins().isub(size, lost);
    let (src, dst) = (NodeCol::row(f, rows, i), NodeCol::row(f, rows, j));
    let x = row_words(f, src);
    put_row_words(f, dst, x);
    crate::native_cpu_tape::put(f, size_c, j, size, w.size);
    if inner {
        let h = slot_get(f, head, i);
        slot_put(f, head, j, h);
    }
    f.ins().jump(place, &[BlockArg::from(i1), BlockArg::from(depth1)]);
    // tails(j, wide): each kept row's tail, from its last descendant now.
    f.switch_to_block(tails);
    let (j, wide) = (f.block_params(tails)[0], f.block_params(tails)[1]);
    let dn = slot_get(f, before, n);
    let kept_n = f.ins().isub(n, dn);
    let end = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, kept_n);
    f.ins().brif(end, back, &[BlockArg::from(supers), BlockArg::from(wide)], tbody, &[]);
    f.switch_to_block(tbody);
    let size = crate::native_cpu_tape::get(f, size_c, j, w.size);
    let last = f.ins().iadd(j, size);
    let last = f.ins().iadd_imm(last, -1);
    let base = if w.from_start { j } else { last };
    let s = crate::native_cpu_tape::start(f, rows, base);
    let row_j = NodeCol::row(f, rows, j);
    let end_j = f.ins().uload32(flags, row_j, AT_END);
    let tl = f.ins().isub(end_j, s);
    let tl = f.ins().band_imm(tl, 0xffff_ffff);
    crate::native_cpu_tape::put(f, tail_c, j, tl, w.tail);
    let odd = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, tl, if w.tail == 2 { 0xffff } else { 0x7fff_ffff });
    let odd = f.ins().uextend(types::I64, odd);
    let odd = f.ins().imul_imm(odd, crate::native_cpu_tape::WIDE_TAIL);
    let wide = f.ins().bor(wide, odd);
    let j1 = f.ins().iadd_imm(j, 1);
    f.ins().jump(tails, &[BlockArg::from(j1), BlockArg::from(wide)]);
    // back(k, wide): the first supertype of a node from `from` on.
    f.switch_to_block(back);
    let (k, wide) = (f.block_params(back)[0], f.block_params(back)[1]);
    let look = f.create_block();
    f.ins().brif(k, look, &[], sback, &[BlockArg::from(k)]);
    f.switch_to_block(look);
    let k1 = f.ins().iadd_imm(k, -1);
    let s = slot_get(f, snode, k1);
    let tail = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, from);
    f.ins().brif(tail, back, &[BlockArg::from(k1), BlockArg::from(wide)], sback, &[BlockArg::from(k)]);
    f.switch_to_block(sback);
    let k = f.block_params(sback)[0];
    f.ins().jump(sstep, &[BlockArg::from(k), BlockArg::from(k), BlockArg::from(wide)]);
    // sstep(s, kept, wide).
    f.switch_to_block(sstep);
    let (s, kept, wide) = (f.block_params(sstep)[0], f.block_params(sstep)[1], f.block_params(sstep)[2]);
    let end = f.ins().icmp(IntCC::Equal, s, supers);
    f.ins().brif(end, done, &[BlockArg::from(kept), BlockArg::from(wide)], sbody, &[BlockArg::from(wide)]);
    f.switch_to_block(sbody);
    let wide = f.block_params(sbody)[0];
    let at = slot_get(f, snode, s);
    let a1 = f.ins().iadd_imm(at, 1);
    let (b0, b1) = (slot_get(f, before, at), slot_get(f, before, a1));
    let s1 = f.ins().iadd_imm(s, 1);
    let lost = f.ins().icmp(IntCC::NotEqual, b0, b1);
    f.ins().brif(lost, sstep, &[BlockArg::from(s1), BlockArg::from(kept), BlockArg::from(wide)], sput, &[BlockArg::from(wide)]);
    f.switch_to_block(sput);
    let wide = f.block_params(sput)[0];
    let place_at = slot_get(f, to, at);
    slot_put(f, snode, kept, place_at);
    let sk = slot_get(f, skind, s);
    slot_put(f, skind, kept, sk);
    let kept1 = f.ins().iadd_imm(kept, 1);
    f.ins().jump(sstep, &[BlockArg::from(s1), BlockArg::from(kept1), BlockArg::from(wide)]);
    f.switch_to_block(done);
    let (kept, wide) = (f.block_params(done)[0], f.block_params(done)[1]);
    let dn = slot_get(f, before, n);
    let nodes = f.ins().isub(n, dn);
    let lo = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, moved, from);
    let hi = f.ins().icmp(IntCC::UnsignedLessThan, moved, n);
    let inside = f.ins().band(lo, hi);
    let safe = f.ins().select(inside, moved, from);
    let m = slot_get(f, to, safe);
    let moved = f.ins().select(inside, m, moved);
    f.ins().return_(&[nodes, kept, moved, wide]);
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
        Op::Span { .. } | Op::Word { test: false, .. } | Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::Settle { .. } | Op::NodeField { .. } | Op::NodeSuper { .. } | Op::Retag { .. } | Op::Wrap { .. } | Op::Merge { .. } | Op::Push { .. } | Op::Remember { .. } | Op::Flag { op: FlagOp::Set | FlagOp::Clear, .. } | Op::Count { .. } | Op::Check { .. }
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
/// function takes `ctx` (and the state in it) and the index of the place it enters at, and a jump
/// to a word of another cluster is a tail call of that cluster (`return_call`, the `tail`
/// calling convention): no stack grows, and every function returns the run's result to
/// `tempo_stream`.
pub fn compile_cut(native: &Structure, cut: &crate::native_cpu_clusters::Cut) -> Result<CompiledStream, Error> {
    compile_at(native, cut, rerail_tape_format::Narrow::WIDTHS, native.tag)
}

/// `compile_cut`, the tape written at `widths` (constants of the code: docs/parser-generators/37)
/// with the language's tag `tag`.
pub fn compile_at(native: &Structure, cut: &crate::native_cpu_clusters::Cut, widths: rerail_tape_format::Widths, tag: u8) -> Result<CompiledStream, Error> {
    let widths_tag = (widths, tag);
    if cut.of.len() != native.structural.len() || cut.count == 0 || (native.structural.is_empty() && cut.count > 1) {
        return Err(Error::UnsupportedBackend);
    }
    let words = &native.structural;
    let pushes = push_targets(native);
    // Each word's phase (`stream::phases`): where every `TOKEN` knows whether it takes a token,
    // no flag says so, and a word after a `SPAN` reads its token's end from its record.
    let phase_of = phases(native, &pushes);
    let flagged = phase_of.iter().any(|&p| p == crate::stream::TAKEN | crate::stream::UNTAKEN);
    let taken = |pc: usize| !flagged && phase_of[pc] == crate::stream::TAKEN;
    let end_at = |pc: usize| if flagged { EndAt::Field } else if taken(pc) { EndAt::Taken } else { EndAt::Untaken };
    let named = |op: fn(&Op) -> Option<u16>| words.iter().find_map(|b| op(&b.operation)).map(usize::from);
    let starts = named(|o| if let Op::Token { extract, .. } = *o { Some(extract) } else { None });
    // The mask `SPAN m=1` names, as its output's index.
    // The records' last bytes, an extract (`records`).
    let lasts = starts.and(native.block_programs.first().and_then(|b| b.records.as_ref())).map(|r| usize::from(r.lasts));
    // Each kind's least byte (`records`): a switch on the first byte is a jump table on the kind.
    let kind_reps: Option<Vec<u8>> = starts.and(native.block_programs.first().and_then(|b| b.records.as_ref())).map(|r| {
        let n = r.kinds.iter().copied().max().map_or(0, |m| usize::from(m) + 1);
        (0..n).map(|k| r.kinds.iter().position(|&x| usize::from(x) == k).unwrap_or(0) as u8).collect()
    });
    // Each word's kind: its first byte's.
    // The stream's end as a kind of its own, past the records' (no word, no mark): a switch on
    // kinds takes it to its end's target, as the lean parser's end token, with no test of its
    // own. None where the kinds fill a byte.
    let end_kind: Option<u8> = kind_reps.as_ref().and_then(|r| u8::try_from(r.len()).ok());
    let word_kinds: Vec<u8> = starts.and(native.block_programs.first().and_then(|b| b.records.as_ref())).map_or_else(Vec::new, |r| r.words.iter().map(|w| r.kinds[usize::from(w[0])]).collect());

    let clustered = cut.count > 1 || cut.calls;
    // `Cut::speed`: Cranelift's optimizer (`opt_level = speed`: value numbering, dead code,
    // redundant loads, invariants out of loops); by default it is off (`none`).
    let mut flags: Vec<(&str, &str)> = Vec::new();
    if cut.speed {
        flags.push(("opt_level", "speed"));
    }
    // With a function a grammar rule, `ctx` is in a register of its own through the run
    // (Cranelift's pinned register, x21): `tempo_stream` sets it (and gives the host's back),
    // each function reads it once where it is entered, into one value with no use fixed to an
    // argument's register (a call passes no `ctx`), which the allocator keeps in one
    // callee-saved register through the function, as rustc keeps the lean parser's `self`.
    let pinned_ctx = clustered && cut.calls && !native.rules.is_empty();
    if pinned_ctx {
        flags.push(("enable_pinned_reg", "true"));
    }
    // The passed fields come back with a function's result: more results than registers go
    // through a return area.
    if clustered && !cut.pass.is_empty() {
        flags.push(("enable_multi_ret_implicit_sret", "true"));
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
    // The calls open (`Mem::Ret`) are a field of the state in `ctx`, read and written where a
    // call is made and returns, and read by `TOKEN`'s room check, as the lean parser's
    // `self` fields are: no value of them lives across a call. Passed as an argument, one value
    // through a whole function, given to each call and to the code of other rules it goes
    // into, was a value regalloc2 split twice at most and then cut into a piece a use, each
    // reloaded from the stack (1,148 loads over `.rr`'s tier 2, against 17 so).
    // A cluster's function: (ctx, the entry's index), the `tail` convention.
    let csig = if clustered {
        let mut c = module.make_signature();
        c.call_conv = cranelift_codegen::isa::CallConv::Tail;
        // `ctx` in the pinned register (`pinned_ctx`), else the first argument.
        if !pinned_ctx {
            c.params.push(AbiParam::new(pointer));
        }
        c.params.push(AbiParam::new(types::I64));
        // `Cut::pass`: those fields as arguments, in registers, and given back with the result,
        // as a call's arguments and results: a call stores none of them.
        c.params.extend(cut.pass.iter().map(|_| AbiParam::new(types::I64)));
        c.returns.push(AbiParam::new(types::I64));
        c.returns.extend(cut.pass.iter().map(|_| AbiParam::new(types::I64)));
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
    // The node words' rare loops, outlined (`TableFns`).
    let image = crate::native_cpu_tape::Image { tag: widths_tag.1, widths: widths_tag.0, extras: &native.readings.extras };
    let table_fns = TableFns::make(&mut module, &native.structural, image)?;
    let table = Table { inner: crate::rust_stream::marks(native), w: widths_tag.0, fns: &table_fns };
    // The words made as functions of their own (`WordFn`), with a function a grammar rule.
    let outlined = !native.rules.is_empty() && cut.calls;
    // The calls open counted by the machine's stack (`calls_on_stack`), with `ctx` pinned.
    let on_stack = pinned_ctx && outlined && !flagged && calls_on_stack(native);
    // The callee takes its caller's token (`RETURN m=0`), as the lean parser's callee consumes
    // it and its caller peeks: where every call's place to return to is one `TOKEN`, alike
    // (the same mask, and taking a token there), a `RETURN m=0` takes it and returns as `m=1`
    // does, to the caller's switch. So every call returns one way and is tested once (`cbnz`).
    let landing_token: Option<(Option<u8>, bool)> = {
        let pushes = push_targets(native);
        let mut anys = pushes.iter().map(|&p| match words.get(usize::from(p)).map(|b| b.operation) {
            Some(Op::Token { any, extras, .. }) if phase_of[usize::from(p)] == crate::stream::TAKEN => Some((any, extras)),
            _ => None,
        });
        let first = anys.next().flatten();
        first.filter(|&a| on_stack && anys.all(|x| x == Some(a)))
    };
    let callee_takes = landing_token.is_some();
    // With a function a grammar rule, the ways a call returns (bit 0: `RETURN m=0`, bit 1:
    // `m=1`), by the place it enters: the `RETURN`s reached from there at the same depth
    // (`rule_successors`), in its function and in the code of earlier rules it goes into (which
    // it calls, passing their result on). A call of a place with one way tests once (`cbnz`), as
    // a `Result`'s: a `RETURN m=1` gives back 0 where every place reaching it has that way alone.
    // By place, not function: a rule's function holds code other rules go into, with ways of
    // its own.
    let rule_of = |pc: usize| native.rules.partition_point(|&s| usize::from(s) <= pc).saturating_sub(1);
    let by_rule = outlined && (0..words.len()).all(|pc| cut.of[pc] as usize == rule_of(pc));
    let n_structural = words.len();
    let entered: Vec<usize> = {
        let mut e: Vec<usize> = (0..n_structural).filter(|&pc| matches!(words[pc].operation, Op::Push { .. })).map(|pc| callee(native, pc)).collect();
        e.sort_unstable();
        e.dedup();
        e
    };
    // The `RETURN`s each entered place reaches.
    let returns_from: Vec<Vec<usize>> = entered
        .iter()
        .map(|&e| {
            let mut seen = vec![false; n_structural];
            let (mut todo, mut out) = (vec![e], Vec::new());
            while let Some(pc) = todo.pop() {
                if pc >= n_structural || std::mem::replace(&mut seen[pc], true) {
                    continue;
                }
                if matches!(words[pc].operation, Op::Return { .. }) {
                    out.push(pc);
                }
                todo.extend(rule_successors(native, pc));
            }
            out
        })
        .collect();
    let again_of = |r: usize| matches!(words[r].operation, Op::Return { again: true });
    let mut one: Vec<Option<u16>> = returns_from
        .iter()
        .map(|rs| {
            let ways = rs.iter().fold(0u8, |w, &r| w | 1 << u8::from(again_of(r)));
            if !by_rule { None } else { match ways { 1 => Some(0), 2 => Some(1), _ => None } }
        })
        .collect();
    // Each `RETURN m=1`'s places (the entered places reaching it).
    let mut users: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for (i, rs) in returns_from.iter().enumerate() {
        for &r in rs.iter().filter(|&&r| again_of(r)) {
            users.entry(r).or_default().push(i);
        }
    }
    // A `RETURN m=1` a place with both ways reaches gives back 1: then every place reaching it
    // tests for 1. To a fixed point.
    loop {
        let mut changed = false;
        for us in users.values() {
            if us.iter().any(|&u| one[u].is_none()) {
                for &u in us {
                    if one[u] == Some(1) {
                        one[u] = None;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // A `RETURN m=1` gives back 0 where every place reaching it reads 0 so.
    let again_as_one: Vec<bool> = (0..n_structural).map(|r| users.get(&r).is_some_and(|us| us.iter().all(|&u| one[u] == Some(1)))).collect();
    // The one way a call of place `e` returns, where it has one.
    let one_way = |e: usize| -> Option<u16> { if callee_takes { Some(1) } else { entered.binary_search(&e).ok().and_then(|i| one[i]) } };
    let mut word_fns: std::collections::HashMap<WordFn, (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)> = std::collections::HashMap::new();
    let mut lowered: Vec<Lowered> = Vec::new();
    let mut word_insts: Vec<u32> = vec![0; words.len()];
    let phase = std::time::Instant::now();
    let times = std::env::var_os("RERAIL_STREAM_TIMES").is_some();
    // With native calls, a function a rule: no arms shared across them.
    let shared_all = if cut.calls { SharedRuns { bodies: Vec::new(), at: Default::default(), ends: Default::default(), skipped: vec![false; words.len()] } } else { shared_runs(native, &pushes) };
    if times {
        eprintln!("tier 2: shared runs {:.2?}", phase.elapsed());
    }
    // Where each word can go on to: its successors, a token's next word, a shared body's end's
    // targets.
    let n_words = words.len();
    let goes_to = |pc: usize| -> Vec<usize> {
        let mut r = if cut.calls { rule_successors(native, pc) } else { successors(native, &pushes, pc) };
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
        // A call's callee: an entry of its function, wherever it is called from.
        for pc in (0..n_words).filter(|&pc| cut.calls && matches!(words[pc].operation, Op::Push { .. })) {
            let e = callee(native, pc);
            if e < n_words {
                incoming[cut.of[e] as usize].insert(e);
            }
        }
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
    // Each cluster's level may change its word or flags (`REMEMBER`, `FLAG` set or clear): in
    // its own words, or in a cluster it goes on to (a tail call); its own calls give them back.
    // A call keeps the caller's only where its callee's may change (`Op::Push`).
    let mut changes_context: Vec<bool> = vec![false; cut.count as usize];
    for pc in (0..n_words).filter(|&pc| matches!(words[pc].operation, Op::Remember { .. } | Op::Flag { op: FlagOp::Set | FlagOp::Clear, .. })) {
        changes_context[cut.of[pc] as usize] = true;
    }
    // With a function a grammar rule and `ctx` pinned (every exit through the function's one
    // `exit`), the word and flags are callee-saved, as registers are: a function whose own
    // words change them keeps the values it was entered with and gives them back where it
    // leaves, as the lean parser's are its callees' locals. A call keeps nothing; code of
    // another rule a function goes into keeps its own.
    let own_changes = changes_context.clone();
    let callee_keeps = pinned_ctx && has_context(words);
    loop {
        let mut more = false;
        for pc in (0..n_words).filter(|&pc| !shared_all.skipped[pc]) {
            let c = cut.of[pc] as usize;
            if changes_context[c] {
                continue;
            }
            if goes_to(pc).into_iter().any(|t| cut.of[t] as usize != c && changes_context[cut.of[t] as usize]) {
                changes_context[c] = true;
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    for c in 0..cut.count {
        // The places this cluster's function names: its own words and every place they can go
        // on to. A block each, and no more: Cranelift's passes size their tables by the blocks.
        let mut named = vec![false; words.len()];
        for pc in (0..words.len()).filter(|&pc| cut.of[pc] == c && !shared_all.skipped[pc]) {
            named[pc] = true;
            let next = if cut.calls { rule_successors(native, pc) } else { successors(native, &pushes, pc) };
            for t in next.into_iter().chain([pc + 1, pc + 2]) {
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
        // after the clusters are lowered, where the jump is kept). A block that only jumps costs
        // nothing: emission folds it (`CompiledStream::code_bytes`). Resolving the jumps here
        // instead made the code larger and slower (594 KB from 590 KB, 500M instructions from
        // 484M over the `.rr` corpus).
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
        // `ctx`: read from its register where the function is entered, else its first argument.
        let ctx = if pinned_ctx { f.ins().get_pinned_reg(types::I64) } else { f.block_params(entry)[0] };
        let first_param = usize::from(!pinned_ctx);
        // `can_move`: a pure value to the optimizer, removed where unused (`Cut::speed`).
        let ro = MemFlags::trusted().with_readonly().with_can_move();
        let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, ro, ctx, at);
        let bytes = load(&mut f, BYTES);
        let len = load(&mut f, LEN);
        let starts_p = load(&mut f, STARTS);
        let nstarts = load(&mut f, NSTARTS);
        let lasts_p = load(&mut f, LASTS);
        let codes_p = load(&mut f, CODES);
        let lasts_stride = load(&mut f, LASTS_STRIDE);
        let totals_p = load(&mut f, TOTALS);
        let (output_stride, totals_stride, totals_mask) = (load(&mut f, OUTPUT_STRIDE), load(&mut f, TOTALS_STRIDE), load(&mut f, TOTALS_MASK));
        let rooms = [load(&mut f, ROOM_NODES), load(&mut f, ROOM_SUPERS), load(&mut f, ROOM_PUSHES)];
        let outputs_p = load(&mut f, OUTPUTS);
        let cols: Vec<Value> = [ROWS, SUPER_NODE, SUPER_KIND, OPEN, RETURNS, COUNTS].iter().map(|&at| load(&mut f, at)).collect();
        let (rows_v, snode_c, skind_c, open_c, ret_c, counts_c) = (cols[0], cols[1], cols[2], cols[3], cols[4], cols[5]);
        // The table's final pass, shared by every `ACCEPT`: (its rows, records).
        let fin = f.create_block();
        for _ in 0..2 {
            f.append_block_param(fin, types::I64);
        }
        let mut fin_used = false;
        let wcols: Vec<Value> = if wrapping {
            [FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE, OUT_ROWS].iter().map(|&at| load(&mut f, at)).collect()
        } else {
            Vec::new()
        };
        let _ = &wcols;
        let (fbase_c, head_c, link_c) = (0, 3, 4);
        let zero = f.ins().iconst(types::I64, 0);
        // The state few words touch, in memory (`Mem`): not carried through every block where
        // paths meet, as an SSA variable is.
        // In clusters, the state is `tempo_stream`'s, in `ctx`.
        let slot = (!clustered).then(|| f.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(cranelift_codegen::ir::StackSlotKind::ExplicitSlot, 8 * Mem::COUNT, 3)));
        // The state's address: `ctx`, every cluster's first argument, as the lean parser's
        // `&mut self` (one register the allocator keeps, no move a block).
        let pinned = |_: &mut FunctionBuilder<'_>| -> Value { ctx };
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
        // The token's kind and word, read from its code in memory as the lean parser reads a
        // token's (`ldrb` of the kind; the word's bytes after it): no shift or mask a test, which
        // Cranelift would make again in every block that tests (`band` with a constant is
        // rematerialized).
        let in_memory = vars[Mem::Taken as usize].is_none() && slot.is_none();
        let token_kind = |f: &mut FunctionBuilder<'_>| -> Value {
            if in_memory {
                f.ins().uload8(types::I64, MemFlags::trusted(), ctx, STATE + 8 * Mem::Taken as i32)
            } else {
                let code = mget(f, Mem::Taken);
                f.ins().band_imm(code, 0xff)
            }
        };
        let token_word = |f: &mut FunctionBuilder<'_>| -> Value {
            if in_memory {
                // The code's second byte (`records::word_of`), as the lean parser loads a
                // token's word.
                const _: () = assert!(crate::records::KIND_BITS == 8);
                f.ins().uload8(types::I64, MemFlags::trusted(), ctx, STATE + 8 * Mem::Taken as i32 + 1)
            } else {
                let code = mget(f, Mem::Taken);
                let w = f.ins().ushr_imm(code, i64::from(crate::records::KIND_BITS));
                f.ins().band_imm(w, 0xff)
            }
        };
        // The marks the records hold (`Records::marks`): mark `k` below `in_code` is bit `k` of
        // the code's top half. At a token, `Live` is those bits (each mark that holds there),
        // the others all set (tested by their masks); at the stream's end, the bits of the
        // marks the end has. A mark the record holds is then one bit test.
        let in_code = native.block_programs.first().and_then(|b| b.records.as_ref()).map_or(0, |r| r.marks.len());
        let mut end_bits: i64 = if in_code >= 64 { 0 } else { -1i64 << in_code };
        for b in words {
            if let Op::Mark { order, end: true, .. } = b.operation
                && usize::from(order) < in_code
            {
                end_bits |= 1 << order;
            }
        }
        let rest_bits: i64 = if in_code >= 64 { 0 } else { -1i64 << in_code };
        let one = f.ins().iconst(types::I64, 1);
        let before_first = f.ins().iconst(types::I64, -1);
        let pick = f.create_block();
        // A rule's function's results: the value, then the passed fields (`Cut::pass`).
        // `end`: `v` is the run's end, its top bit flipped here (`ENDED`).
        let results = |f: &mut FunctionBuilder<'_>, v: Value, end: bool| -> Vec<Value> {
            let v = if clustered && end { f.ins().bxor_imm(v, ENDED as i64) } else { v };
            let mut r = vec![v];
            if clustered {
                r.extend(cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))));
            }
            r
        };
        // The function's one exit, as the lean parser's functions have one epilogue: every
        // return (a rule's, a call's run end passed on, a refusal, the table's end) jumps here
        // with its results. A return each would be an epilogue each (its callee-saved
        // registers restored).
        let exit = f.create_block();
        // The word and flags this function gives back where it leaves (`callee_keeps`).
        let mut entry_kept: Option<(Value, Value)> = None;
        for _ in 0..1 + if clustered { cut.pass.len() } else { 0 } {
            f.append_block_param(exit, types::I64);
        }
        let leave = |f: &mut FunctionBuilder<'_>, r: &[Value]| {
            let args: Vec<BlockArg> = r.iter().map(|&v| BlockArg::from(v)).collect();
            f.ins().jump(exit, &args);
        };
        if clustered {
            for (k, v) in vars.iter().enumerate() {
                if let Some(v) = *v {
                    let x = match cut.pass.iter().position(|&q| q as usize == k) {
                        Some(j) => f.block_params(entry)[first_param + 1 + j],
                        None => {
                            let p = pinned(&mut f);
                            f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32)
                        }
                    };
                    f.def_var(v, x);
                }
            }
            // The word and flags the function was entered with, where its own words change them
            // (`callee_keeps`).
            if callee_keeps && own_changes[c as usize] {
                entry_kept = Some((mget(&mut f, Mem::Word), mget(&mut f, Mem::Flags)));
            }
            // The calls open on the machine's stack (`on_stack`): its depth checked once, where
            // the function is entered, as a stack check in a prologue (one load, one compare).
            if on_stack {
                let sp = f.ins().get_stack_pointer(types::I64);
                let limit = f.ins().load(types::I64, MemFlags::trusted(), ctx, STACK_LIMIT);
                let deep = f.ins().icmp(IntCC::UnsignedLessThan, sp, limit);
                let (ok, no) = (f.create_block(), f.create_block());
                f.ins().brif(deep, no, &[], ok, &[]);
                f.switch_to_block(no);
                f.set_cold_block(no);
                let at = f.ins().iconst(types::I64, 1);
                f.ins().jump(refuse, &[BlockArg::from(at)]);
                f.switch_to_block(ok);
            }
            f.ins().jump(pick, &[]);
        } else {
            for k in 0..Mem::COUNT {
                f.ins().stack_store(zero, slot.expect("a slot"), 8 * k as i32);
                if let Some(v) = vars[k as usize] {
                    f.def_var(v, zero);
                }
            }
            if flagged {
                mset(&mut f, Mem::Need, one);
            }
            mset(&mut f, Mem::Lw, before_first);
            match wb.first().copied().flatten() {
                Some(b) => f.ins().jump(b, &[]),
                None => f.ins().jump(off_end, &[]),
            };
        }

        let flags = MemFlags::trusted();
        // The other functions this one calls or tail-calls, each imported once.
        let mut call_refs: std::collections::HashMap<usize, cranelift_codegen::ir::FuncRef> = std::collections::HashMap::new();
        // Refuse at `pc` when `bad`; else go on in a new block.
        // The refusal's place is set in a cold block of its own, not on the hot path.
        let guard = |f: &mut FunctionBuilder<'_>, bad: Value, pc: usize| {
            let (ok, no) = (f.create_block(), f.create_block());
            f.ins().brif(bad, no, &[], ok, &[]);
            f.switch_to_block(no);
            f.set_cold_block(no);
            let at = f.ins().iconst(types::I64, pc as i64 + 1);
            f.ins().jump(refuse, &[BlockArg::from(at)]);
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
            // The word's place as its instructions' source location: its machine code's bytes
            // (`CompiledStream::word_bytes`).
            f.set_srcloc(cranelift_codegen::ir::SourceLoc::new(pc as u32 + 1));
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
            let (bytes, len, starts_p, nstarts, lasts_p, codes_p, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p) =
                (bytes, len, starts_p, nstarts, lasts_p, codes_p, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p);
            let (rows_v, snode_c, skind_c, open_c, ret_c, counts_c) = (rows_v, snode_c, skind_c, open_c, ret_c, counts_c);
            let wcols_here: Vec<Value>;
            let (bytes, len, starts_p, nstarts, lasts_p, codes_p, lasts_stride, totals_p, output_stride, totals_stride, totals_mask, rooms, outputs_p, rows_here, snode_c, skind_c, open_c, ret_c, counts_c, wc) = {
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
                    get(&mut f, BYTES, bytes), get(&mut f, LEN, len), get(&mut f, STARTS, starts_p), get(&mut f, NSTARTS, nstarts), get(&mut f, LASTS, lasts_p), get(&mut f, CODES, codes_p),
                    get(&mut f, LASTS_STRIDE, lasts_stride), get(&mut f, TOTALS, totals_p), get(&mut f, OUTPUT_STRIDE, output_stride), get(&mut f, TOTALS_STRIDE, totals_stride), get(&mut f, TOTALS_MASK, totals_mask),
                    [get(&mut f, ROOM_NODES, rooms[0]), get(&mut f, ROOM_SUPERS, rooms[1]), get(&mut f, ROOM_PUSHES, rooms[2])], get(&mut f, OUTPUTS, outputs_p),
                    get(&mut f, ROWS, rows_v), get(&mut f, SUPER_NODE, snode_c), get(&mut f, SUPER_KIND, skind_c),
                    get(&mut f, OPEN, open_c), get(&mut f, RETURNS, ret_c), get(&mut f, COUNTS, counts_c),
                );
                wcols_here = if wrapping {
                    [FRAME_BASE, FRAME_LAST, FRAME_LAST_BASE, HEAD, LINK, PLACE, OUT_ROWS].iter().enumerate().map(|(i, &at)| get(&mut f, at, wcols[i])).collect()
                } else {
                    Vec::new()
                };
                let wc = |i: usize| wcols_here[i];
                (v.0, v.1, v.2, v.3, v.4, v.5, v.6, v.7, v.8, v.9, v.10, v.11, v.12, v.13, v.14, v.15, v.16, v.17, v.18, wc)
            };
            // The node rows' fields.
            let (kind_c, field_c, end_c) = (NodeCol::of(rows_here, AT_KIND), NodeCol::of(rows_here, AT_FIELD), NodeCol::of(rows_here, AT_END));
            let fall = goto(pc + 1);
            // What a word's body reads and writes here (`WordEnv`): the state's fields, `ctx`'s
            // columns as this word has them, its refusal at its place.
            let env_get = |f: &mut FunctionBuilder<'_>, k: Mem| mget(f, k);
            let env_set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| mset(f, k, x);
            let env_ctx = |f: &mut FunctionBuilder<'_>, at: i32| -> Value {
                match at {
                    ROWS => rows_here,
                    OPEN => open_c,
                    FRAME_BASE if wrapping => wc(fbase_c),
                    HEAD if wrapping => wc(head_c),
                    LINK if wrapping => wc(link_c),
                    _ => {
                        let c = if clustered { pinned(f) } else { ctx };
                        f.ins().load(types::I64, MemFlags::trusted(), c, at)
                    }
                }
            };
            let env_guard = |f: &mut FunctionBuilder<'_>, bad: Value| guard(f, bad, pc);
            let env = WordEnv { get: &env_get, set: &env_set, ctx: &env_ctx, guard: &env_guard };
            // A word's body: made here, or with a function a grammar rule, a function of its own
            // (`WordFn`) every place calls with its arguments, which gives back whether it refuses.
            macro_rules! word {
                ($key:expr, $args:expr, $refuses:expr, $body:expr) => {
                    word!($key, $args, $refuses, $body, fall)
                };
                ($key:expr, $args:expr, $refuses:expr, $body:expr, $fall:expr) => {{
                    let body = $body;
                    if outlined {
                        let key = $key;
                        let args: &[Value] = $args;
                        if !word_fns.contains_key(&key) {
                            let name = format!("tempo_word_{}", word_fns.len());
                            let made = outline(&mut module, &name, 1 + args.len(), usize::from($refuses), pinned_ctx, &table.fns.names(), |g, q, _| own_word(g, q, $refuses, &body))?;
                            word_fns.insert(key, made);
                        }
                        let (id, sig, _) = &word_fns[&key];
                        let c = pinned(&mut f);
                        let r = call_word(&mut f, *id, sig, pinned_ctx, c, args);
                        if $refuses {
                            guard(&mut f, r[0], pc);
                        }
                    } else {
                        body(&mut f, &env, $args);
                    }
                    f.ins().jump($fall, &[]);
                }};
            }
            // A node's field set by the next word (`NODE_FIELD` on the node just made), stored
            // with its kind (one 32-bit store of both), the field word passed: as a call's
            // argument, with a function a grammar rule.
            let with_field = |kind: u16| -> (i64, Block) {
                match words.get(pc + 1).map(|w| w.operation) {
                    Some(Op::NodeField { field, on: On::Here }) if outlined => (i64::from(kind) | i64::from(field) << 16, goto(pc + 2)),
                    _ => (i64::from(kind), fall),
                }
            };
            match w.operation {
                Op::Token { any, extras, .. } => {
                    // One kind of extra (`.rr`'s comments) for now.
                    let extra: Option<(u8, u16)> = match (extras, native.extras.as_slice()) {
                        (false, _) => None,
                        (true, &[one]) => Some(one),
                        (true, _) => return Err(Error::UnsupportedBackend),
                    };
                    // Any mark's mask (`TOKEN m=1+k`): its output.
                    let any_mark: Option<usize> = match any {
                        Some(k) => Some(native.block_programs.first().and_then(|b| b.masks.get(usize::from(k)).copied()).ok_or(Error::UnsupportedBackend)?),
                        None => None,
                    };
                    heads.push((wb[pc].expect("its own place"), Step::Token));
                    if outlined && !flagged {
                        if phase_of[pc] != crate::stream::TAKEN {
                            // No token to take: the state switches on the one it has.
                            f.ins().jump(fall, &[]);
                            continue;
                        }
                        let shape = TokenShape { bound, in_code, rest_bits, end_bits, end_code: end_kind.map(i64::from), on_stack, extra, wrapping };
                        let (id, sig, _) = token_fn(&mut module, &mut word_fns, any_mark, shape, table, pinned_ctx)?;
                        let c = pinned(&mut f);
                        let open_calls: Vec<Value> = if on_stack { vec![] } else { vec![mget(&mut f, Mem::Ret)] };
                        let r = call_word(&mut f, *id, sig, pinned_ctx, c, &open_calls)[0];
                        // A refusal: past the stream's end at this word, or out of room.
                        let no = f.create_block();
                        f.ins().brif(r, no, &[], fall, &[]);
                        f.switch_to_block(no);
                        f.set_cold_block(no);
                        let past = f.ins().icmp_imm(IntCC::Equal, r, 1);
                        let here = f.ins().iconst(types::I64, pc as i64 + 1);
                        let at = f.ins().select(past, here, r);
                        f.ins().jump(refuse, &[BlockArg::from(at)]);
                        continue;
                    }
                    // The path from here to the next token word writes at most `bound`'s nodes,
                    // supertypes and returns: past the tape's room, the run stops for a bigger
                    // tape. Three compares a token, not one a write.
                    let open_calls = mget(&mut f, Mem::Ret);
                    // Out of room: one cold block, which sets the run's end.
                    let over = f.create_block();
                    let counts = [mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Supers), mget(&mut f, Mem::Ret)];
                    for (at, (room, (most, per_call))) in counts.into_iter().zip(rooms.into_iter().zip([(bound.nodes, bound.pop_nodes), (bound.supers, bound.pop_supers), (bound.pushes, 0)])) {
                        let at = if per_call > 0 {
                            let popped = f.ins().imul_imm(open_calls, per_call as i64);
                            f.ins().iadd(at, popped)
                        } else {
                            at
                        };
                        let reach = f.ins().iadd_imm(at, most as i64);
                        // A branch each, not the compares joined (`cset`, `orr`, `cbnz`).
                        let past = f.ins().icmp(IntCC::UnsignedGreaterThan, reach, room);
                        let fits = f.create_block();
                        f.ins().brif(past, over, &[], fits, &[]);
                        f.switch_to_block(fits);
                    }
                    let fits = f.current_block().expect("in a block");
                    f.switch_to_block(over);
                    f.set_cold_block(over);
                    let overflow = f.ins().iconst(types::I64, OVERFLOW as i64);
                    f.ins().jump(refuse, &[BlockArg::from(overflow)]);
                    f.switch_to_block(fits);
                    // A token whose bytes are not taken yet (after a mark's arm) is kept: by the
                    // word's phase, or where a `TOKEN` has both, by a flag.
                    if flagged {
                        let take = f.create_block();
                        let need = mget(&mut f, Mem::Need);
                        f.ins().brif(need, take, &[], fall, &[]);
                        f.switch_to_block(take);
                        mset(&mut f, Mem::Need, zero);
                    } else if phase_of[pc] != crate::stream::TAKEN {
                        f.ins().jump(fall, &[]);
                        let dead = f.create_block();
                        f.switch_to_block(dead);
                    }
                    let at_end = mget(&mut f, Mem::AtEnd);
                    guard(&mut f, at_end, pc);
                    let next = mget(&mut f, Mem::Next);
                    let end = if flagged { mget(&mut f, Mem::End) } else { mget(&mut f, Mem::Limit) };
                    mset(&mut f, Mem::LastEnd, end);
                    // The stream's end: once a file, a cold block (the lean parser's index past
                    // its tokens). The token's fields stay; every mark is live (a mark the end
                    // has is there).
                    let all = f.ins().iconst(types::I64, -1);
                    let is_end = f.ins().icmp(IntCC::Equal, next, nstarts);
                    let (last, token) = (f.create_block(), f.create_block());
                    f.ins().brif(is_end, last, &[], token, &[]);
                    f.switch_to_block(last);
                    f.set_cold_block(last);
                    let one = f.ins().iconst(types::I64, 1);
                    mset(&mut f, Mem::AtEnd, one);
                    let at_end_live = f.ins().iconst(types::I64, end_bits);
                    mset(&mut f, Mem::Live, at_end_live);
                    if let Some(k) = end_kind {
                        let code = f.ins().iconst(types::I64, i64::from(k));
                        mset(&mut f, Mem::Taken, code);
                    }
                    f.ins().jump(fall, &[]);
                    // The token's record (`records`): its first byte, last byte and code, from
                    // three columns; no select, no compare but the end's. The record pass checked
                    // the columns (the totals' flag).
                    f.switch_to_block(token);
                    let load_at = |f: &mut FunctionBuilder<'_>, c: Value| {
                        let at = slot(f, c, next);
                        f.ins().uload32(flags, at, 0)
                    };
                    let (s, l, c) = (load_at(&mut f, starts_p), load_at(&mut f, lasts_p), load_at(&mut f, codes_p));
                    let l1 = f.ins().iadd_imm(l, 1);
                    let after = f.ins().iadd_imm(next, 1);
                    mset(&mut f, Mem::Start, s);
                    if flagged {
                        mset(&mut f, Mem::End, s);
                    }
                    mset(&mut f, Mem::Next, after);
                    mset(&mut f, Mem::Limit, l1);
                    mset(&mut f, Mem::Taken, c);
                    // An extra (`TOKEN_EXTRAS`): its bytes taken and its leaf deferred, then this
                    // word again for the next token.
                    if let Some((bit, kind)) = extra {
                        let b = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT) + i64::from(bit));
                        let b = f.ins().band_imm(b, 1);
                        let (xb, on) = (f.create_block(), f.create_block());
                        f.ins().brif(b, xb, &[], on, &[]);
                        f.switch_to_block(xb);
                        if flagged {
                            mset(&mut f, Mem::End, l1);
                            let one = f.ins().iconst(types::I64, 1);
                            mset(&mut f, Mem::Need, one);
                        }
                        let k = f.ins().iconst(types::I64, i64::from(kind));
                        let again = wb[pc].expect("its own place");
                        word!(WordFn::Leaf { deferred: true, extra: true, end: EndAt::Taken, by_word: false }, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| leaf_word(g, e, table, q[0], true, true, EndAt::Taken, wrapping), again);
                        f.switch_to_block(on);
                    }
                    // The marks live at this token: every one; with any mark's mask
                    // (`TOKEN m=1+k`), none where no mark holds, so a state's marks fail at their
                    // first test, with no load.
                    let live = match any_mark {
                        _ if in_code > 0 => {
                            let marks = f.ins().ushr_imm(c, i64::from(crate::records::MARK_SHIFT));
                            if rest_bits == 0 { marks } else { f.ins().bor_imm(marks, rest_bits) }
                        }
                        Some(output) => {
                            let w = f.ins().ushr_imm(s, 6);
                            let off = f.ins().imul_imm(output_stride, output as i64);
                            let base = f.ins().iadd(outputs_p, off);
                            let word = mask_word(&mut f, base, lasts_stride, w);
                            let sh = f.ins().band_imm(s, 63);
                            let bit = f.ins().ushr(word, sh);
                            let bit = f.ins().band_imm(bit, 1);
                            f.ins().ineg(bit)
                        }
                        None => all,
                    };
                    mset(&mut f, Mem::Live, live);
                    f.ins().jump(fall, &[]);
                }
                Op::Switch { table } => {
                    let s = &native.switches[usize::from(table)];
                    // On kinds, the end is a kind (`end_kind`): its target in the tree.
                    let end_in_tree = end_kind.filter(|_| kind_reps.is_some() && s.offset == 0);
                    if end_in_tree.is_none() {
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
                    }
                    // On the first byte, with records: the record's kind (`records`), the
                    // switch's targets by kind (its least byte's), a compare tree as on bytes.
                    // A switch that names words: a kind whose words it names goes to a block of
                    // its own (a place past every word's, `BY_WORD` and the kind), which takes
                    // a token that is no word to the kind's target, else switches on the word.
                    // A word's kind is its first byte's, so a token of the kind is one of its
                    // words or none. Tokens that are no keyword pay one test, as a parser's
                    // keywords are its identifiers' case.
                    let mut by_word: Vec<(usize, u16)> = Vec::new();
                    let by_kind = kind_reps.as_ref().filter(|_| s.offset == 0).map(|reps| {
                        let mut t = vec![REFUSE; 256];
                        for (k, &b) in reps.iter().enumerate() {
                            t[k] = s.targets[usize::from(b)];
                        }
                        if let Some(e) = end_in_tree {
                            t[usize::from(e)] = s.end;
                        }
                        for k in 0..reps.len() {
                            if s.words.iter().any(|&(w, _)| word_kinds.get(usize::from(w)).is_some_and(|&x| usize::from(x) == k)) {
                                by_word.push((k, t[k]));
                                t[k] = BY_WORD + k as u16;
                            }
                        }
                        ByteSwitch { targets: t, ..s.clone() }
                    });
                    let s = by_kind.as_ref().unwrap_or(s);
                    let kind_blocks: Vec<Block> = by_word.iter().map(|_| f.create_block()).collect();
                    let plain_of = |t: u16| by_word.iter().find(|&&(k, _)| BY_WORD + k as u16 == t).map(|&(_, p)| p);
                    let word = |t: u16| -> Block {
                        match by_word.iter().position(|&(k, _)| t != REFUSE && BY_WORD + k as u16 == t) {
                            Some(i) => kind_blocks[i],
                            None => word(t),
                        }
                    };
                    let b = if by_kind.is_some() {
                        token_kind(&mut f)
                    } else {
                        let (start, limit) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::Limit));
                        let at = f.ins().iadd_imm(start, i64::from(s.offset));
                        let past = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, at, limit);
                        guard(&mut f, past, pc);
                        let addr = f.ins().iadd(bytes, at);
                        f.ins().uload8(types::I64, flags, addr, 0)
                    };
                    // A refusing target refuses at this word.
                    let no = f.create_block();
                    match cut.pick {
                        crate::native_cpu_clusters::Pick::Tree => {
                            let t = runs(s, &|t| if t == REFUSE { no } else { word(t) });
                            dispatch(&mut f, b, &t, 256);
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
                                    // A kind's block by word: its target's traffic as the kind's.
                                    let traffic = plain_of(t).unwrap_or(t);
                                    let w = if t == REFUSE { 0 } else { cut.pick_traffic.get(&(pc as u32, u32::from(traffic))).copied().unwrap_or(0) / per[&t] };
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
                    // Each kind's block by word: its words' index plus one (0: no word), a tree
                    // over them; a word the switch does not name, and no word, the kind's target.
                    for (&(k, plain), &blk) in by_word.iter().zip(&kind_blocks) {
                        f.switch_to_block(blk);
                        let w = token_word(&mut f);
                        let mut t = vec![plain; word_kinds.len() + 1];
                        for &(x, to) in &s.words {
                            if word_kinds.get(usize::from(x)).is_some_and(|&y| usize::from(y) == k) {
                                t[usize::from(x) + 1] = to;
                            }
                        }
                        let sw = ByteSwitch { targets: t, ..ByteSwitch::refusing(0) };
                        let r = runs(&sw, &|t| if t == REFUSE { no } else { word(t) });
                        dispatch(&mut f, w, &r, 256);
                    }
                    f.switch_to_block(no);
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                }
                Op::Span { last: false, len: n } => {
                    // A length, which must be the record's.
                    let start = mget(&mut f, Mem::Start);
                    let e = f.ins().iadd_imm(start, i64::from(n));
                    let rec_end = mget(&mut f, Mem::Limit);
                    let past = f.ins().icmp(IntCC::NotEqual, e, rec_end);
                    guard(&mut f, past, pc);
                    // Taken: the words after read the record's end (`taken`).
                    if flagged {
                        mset(&mut f, Mem::End, e);
                        let one = f.ins().iconst(types::I64, 1);
                        mset(&mut f, Mem::Need, one);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Span { last: true, .. } => {
                    // The record's end: the words after read it there (`taken`).
                    if flagged {
                        let rec_end = mget(&mut f, Mem::Limit);
                        mset(&mut f, Mem::End, rec_end);
                        let one = f.ins().iconst(types::I64, 1);
                        mset(&mut f, Mem::Need, one);
                    }
                    f.ins().jump(fall, &[]);
                }
                Op::Word { string, test } => {
                    // The record's word: its code names the string (`records`), one compare.
                    let w = token_word(&mut f);
                    if test {
                        let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(string) + 1);
                        let skip = goto(pc + 2);
                        f.ins().brif(same, fall, &[], skip, &[]);
                    } else {
                        // One compare and a branch on it, as the lean parser's (no `bnot` of a
                        // compare, which Cranelift makes a `cset`, an `eor` and a `uxtb`).
                        let differs = f.ins().icmp_imm(IntCC::NotEqual, w, i64::from(string) + 1);
                        guard(&mut f, differs, pc);
                        f.ins().jump(fall, &[]);
                    }
                }
                Op::Mark { mask, order, end, keep, word: mark_word, pattern } => {
                    // The mark's bit at the token's start, if the mark is live: one load. At the
                    // stream's end, the mark is there when the end has it.
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    let skip = goto(pc + 2);
                    if usize::from(order) < in_code {
                        // In the record: its bit in `Live`, one test. Taken, it and the marks
                        // before it are done at this token.
                        let live = mget(&mut f, Mem::Live);
                        let lit = f.ins().band_imm(live, 1i64 << order);
                        let taken = f.create_block();
                        f.ins().brif(lit, taken, &[], skip, &[]);
                        f.switch_to_block(taken);
                        // A token mark (decision 0129): its tokens follow, from the records.
                        if let Some(i) = pattern {
                            let m = native.token_marks[usize::from(i)].clone();
                            let bit = native.extras.first().ok_or(Error::UnsupportedBackend)?.0;
                            let r = if outlined {
                                let key = WordFn::TokenMark { index: i };
                                if !word_fns.contains_key(&key) {
                                    let name = format!("tempo_word_{}", word_fns.len());
                                    let made = outline(&mut module, &name, 1, 1, pinned_ctx, &[], |g, q, hs| {
                                        let ctx = q[0];
                                        let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
                                        let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
                                        let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
                                        let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a token mark does not refuse");
                                        let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
                                        let v = token_pattern(g, &env, &m, bit, hs);
                                        g.ins().return_(&[v]);
                                    })?;
                                    word_fns.insert(key, made);
                                }
                                let (id, sig, _) = &word_fns[&key];
                                let c = pinned(&mut f);
                                call_word(&mut f, *id, sig, pinned_ctx, c, &[])[0]
                            } else {
                                token_pattern(&mut f, &env, &m, bit, &mut heads)
                            };
                            let follows = f.create_block();
                            f.ins().brif(r, follows, &[], skip, &[]);
                            f.switch_to_block(follows);
                        }
                        // Kept: the callee its arm calls takes it.
                        if !keep {
                            let rest = f.ins().band_imm(live, !((2i64 << order) - 1));
                            mset(&mut f, Mem::Live, rest);
                        }
                        f.ins().jump(fall, &[]);
                        continue;
                    }
                    // A token mark is in the record (`validate_token_marks`).
                    if pattern.is_some() {
                        return Err(Error::UnsupportedBackend);
                    }
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
                    // A mark of one word: the record's word first, a compare on a register.
                    if let Some(s) = mark_word {
                        let w = token_word(&mut f);
                        let same = f.ins().icmp_imm(IntCC::Equal, w, i64::from(s) + 1);
                        let bits = f.create_block();
                        f.ins().brif(same, bits, &[], skip, &[]);
                        f.switch_to_block(bits);
                    }
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
                    // It and the marks before it are done at this token; kept, the callee its arm
                    // calls takes it.
                    if !keep {
                        let live = mget(&mut f, Mem::Live);
                        let rest = f.ins().band_imm(live, !((2i64 << order) - 1));
                        mset(&mut f, Mem::Live, rest);
                    }
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
                    let (code, next) = with_field(kind);
                    let k = f.ins().iconst(types::I64, code);
                    word!(WordFn::Open, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| open_word(g, e, table, q[0], wrapping), next);
                }
                Op::Close { this } => {
                    let end = end_at(pc);
                    word!(WordFn::Close { this, end }, &[], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| close_word(g, e, table, this, end, wrapping));
                }
                Op::Leaf { kind, deferred, extra } => {
                    let end = end_at(pc);
                    let by_word = kind == WORD_KIND;
                    let (code, next) = with_field(kind);
                    // A keyword's leaf: the field above, the kind from the token's word.
                    let code = if by_word { code & !0xffff } else { code };
                    let k = f.ins().iconst(types::I64, code);
                    let word_kinds = &native.word_kinds;
                    word!(WordFn::Leaf { deferred, extra, end, by_word }, &[k], by_word, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if by_word {
                            let kind = word_kind(g, e, word_kinds);
                            g.ins().bor(q[0], kind)
                        } else {
                            q[0]
                        };
                        leaf_word(g, e, table, k, deferred, extra, end, wrapping)
                    }, next);
                }
                Op::Settle { .. } => {
                    // A token the grammar implies: no node; the extras pending stay where they are.
                    mset(&mut f, Mem::Pending, zero);
                    f.ins().jump(fall, &[]);
                }
                // A keyword's: its token's word's kind (`WORD_KIND`), in a word of its own.
                Op::Remember { kind: WORD_KIND } => {
                    let word_kinds = &native.word_kinds;
                    word!(WordFn::RememberWord, &[], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| {
                        let k = word_kind(g, e, word_kinds);
                        (e.set)(g, Mem::Word, k);
                    });
                }
                Op::Remember { kind } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    mset(&mut f, Mem::Word, k);
                    f.ins().jump(fall, &[]);
                }
                Op::Check { check } => {
                    let c = native.local_checks.get(usize::from(check)).cloned().ok_or(Error::UnsupportedBackend)?;
                    let form = crate::rust_stream::word_form(&c.ops).map_err(|_| Error::UnsupportedBackend)?;
                    let output = native.block_programs.first().and_then(|b| b.masks.get(usize::from(c.mask)).copied()).ok_or(Error::UnsupportedBackend)?;
                    // Most tokens: no bit of the mask from the start on, in the start's word, and
                    // the token ends in it. One load.
                    let (start, limit) = (mget(&mut f, Mem::Start), mget(&mut f, Mem::Limit));
                    let w = f.ins().ushr_imm(start, 6);
                    let off = f.ins().imul_imm(output_stride, output as i64);
                    let base = f.ins().iadd(outputs_p, off);
                    let word = mask_word(&mut f, base, lasts_stride, w);
                    let s = f.ins().band_imm(start, 63);
                    let word = f.ins().ushr(word, s);
                    let l1 = f.ins().iadd_imm(limit, -1);
                    let wl = f.ins().ushr_imm(l1, 6);
                    let other = f.ins().icmp(IntCC::NotEqual, wl, w);
                    let any = f.ins().icmp_imm(IntCC::NotEqual, word, 0);
                    let go = f.ins().bor(any, other);
                    let slow = f.create_block();
                    f.ins().brif(go, slow, &[], fall, &[]);
                    f.switch_to_block(slow);
                    // Always a function of its own, one a check: rare code, out of the arm's.
                    let r = {
                        let key = WordFn::Check { index: check };
                        if !word_fns.contains_key(&key) {
                            let name = format!("tempo_word_{}", word_fns.len());
                            let made = outline(&mut module, &name, 3, 1, pinned_ctx, &[], |g, q, hs| {
                                let ctx = q[0];
                                let get = |f: &mut FunctionBuilder<'_>, k: Mem| field_get(f, ctx, k);
                                let set = |f: &mut FunctionBuilder<'_>, k: Mem, x: Value| field_set(f, ctx, k, x);
                                let at = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, MemFlags::trusted(), ctx, at);
                                let guard = |_: &mut FunctionBuilder<'_>, _: Value| unreachable!("a check returns whether it holds");
                                let env = WordEnv { get: &get, set: &set, ctx: &at, guard: &guard };
                                let v = check_word(g, &env, &c, &form, output, q[1], q[2], hs);
                                g.ins().return_(&[v]);
                            })?;
                            word_fns.insert(key, made);
                        }
                        let (id, sig, _) = &word_fns[&key];
                        let cv = pinned(&mut f);
                        call_word(&mut f, *id, sig, pinned_ctx, cv, &[start, limit])[0]
                    };
                    guard(&mut f, r, pc);
                    f.ins().jump(fall, &[]);
                }
                Op::Count { count, up, max } => {
                    // One count (a file's depth) for now.
                    if count != 0 {
                        return Err(Error::UnsupportedBackend);
                    }
                    let d = mget(&mut f, Mem::Depth);
                    if up {
                        let d = f.ins().iadd_imm(d, 1);
                        mset(&mut f, Mem::Depth, d);
                        let past = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, d, i64::from(max));
                        guard(&mut f, past, pc);
                    } else {
                        let above = f.ins().icmp_imm(IntCC::NotEqual, d, 0);
                        let one = f.ins().uextend(types::I64, above);
                        let d = f.ins().isub(d, one);
                        mset(&mut f, Mem::Depth, d);
                    }
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
                    let remembered = kind == REMEMBERED;
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::RetagMoved { remembered }, &[k], remembered, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if remembered { remembered_kind(g, e) } else { q[0] };
                        let m = (e.get)(g, Mem::Moved);
                        let rows = (e.ctx)(g, ROWS);
                        table.retag(g, rows, m, k);
                    });
                }
                Op::Wrap { kind } if wrapping => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Wrap, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| wrap_word(g, e, table, q[0]));
                }
                Op::NodeField { field, on: On::Inner } => {
                    // The node the node made last, a wrap, holds first: the row after it, unless
                    // that is a wrap too (`head`).
                    let open = mget(&mut f, Mem::Open);
                    let last = mget(&mut f, Mem::LastMade);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    // A branch each, not the tests joined.
                    guard(&mut f, none, pc);
                    guard(&mut f, no_last, pc);
                    let r = f.ins().iadd_imm(last, -1);
                    let (mark, inner_mark) = {
                        let a = slot(&mut f, wc(head_c), r);
                        (f.ins().uload32(flags, a, 0), f.ins().uload32(flags, a, 4))
                    };
                    let mark = f.ins().band_imm(mark, WRAPPED);
                    let not_wrap = f.ins().icmp_imm(IntCC::Equal, mark, 0);
                    guard(&mut f, not_wrap, pc);
                    let inner_mark = f.ins().band_imm(inner_mark, WRAPPED);
                    let nested = f.ins().icmp_imm(IntCC::NotEqual, inner_mark, 0);
                    guard(&mut f, nested, pc);
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    field_c.put(&mut f, last, x);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind, on: On::Made } | Op::NodeField { field: kind, on: On::Made | On::MadeUnset } => {
                    let retag = matches!(w.operation, Op::Retag { .. });
                    let unset = matches!(w.operation, Op::NodeField { on: On::MadeUnset, .. });
                    let remembered = retag && kind == REMEMBERED;
                    let dead = retag && kind == DEAD;
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Made { retag, remembered, dead, unset }, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if remembered { remembered_kind(g, e) } else { q[0] };
                        made_word(g, e, table, k, retag, dead, unset)
                    });
                }
                Op::Reread { reading } if wrapping => {
                    // The node made last at the innermost open node's level and the nodes inside
                    // it: its rows, in preorder, to the pending extras. A loop over the rows, a
                    // step a row (`Step::Node`); a kind each, by a run of compares. A dropped row
                    // is taken out after (`drop`), where any was.
                    let r = native.readings.maps.get(usize::from(reading)).ok_or(Error::UnsupportedBackend)?;
                    let open = mget(&mut f, Mem::Open);
                    let last = mget(&mut f, Mem::LastMade);
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    // A branch each, not the tests joined.
                    guard(&mut f, none, pc);
                    guard(&mut f, no_last, pc);
                    let root = f.ins().iadd_imm(last, -1);
                    let (nodes, pending) = (mget(&mut f, Mem::Nodes), mget(&mut f, Mem::Pending));
                    let end = f.ins().isub(nodes, pending);
                    let (scan, body, after) = (f.create_block(), f.create_block(), f.create_block());
                    f.append_block_param(scan, types::I64);
                    f.append_block_param(scan, types::I64);
                    f.append_block_param(after, types::I64);
                    heads.push((scan, Step::Node));
                    f.ins().jump(scan, &[BlockArg::from(root), BlockArg::from(zero)]);
                    // scan(i, gone): row i; whether a row was dropped.
                    f.switch_to_block(scan);
                    let (i, gone) = (f.block_params(scan)[0], f.block_params(scan)[1]);
                    let done = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, end);
                    f.ins().brif(done, after, &[BlockArg::from(gone)], body, &[]);
                    f.switch_to_block(body);
                    let nrow = NodeCol::row(&mut f, rows_here, i);
                    let k = kind_c.get_row(&mut f, nrow);
                    // Its field, unless it is the node read: the reading's, by a run of selects.
                    let fld = field_c.get_row(&mut f, nrow);
                    let mut to = fld;
                    for &(a, b) in &r.fields {
                        let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(a));
                        let g = f.ins().iconst(types::I64, i64::from(b));
                        to = f.ins().select(is, g, to);
                    }
                    let is_root = f.ins().icmp(IntCC::Equal, i, root);
                    let to = f.ins().select(is_root, fld, to);
                    field_c.put_row(&mut f, nrow, to);
                    let i1 = f.ins().iadd_imm(i, 1);
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
                        if way != KEEP {
                            // Reading on into its children, where a dropped row was: the
                            // interpreter meets that row and refuses.
                            let m = {
                                let a = slot(&mut f, wc(head_c), i);
                                f.ins().uload32(flags, a, 0)
                            };
                            let m = f.ins().band_imm(m, DROPPED_IN);
                            guard(&mut f, m, pc);
                        }
                        match way {
                            KEEP => {
                                let sc = env_ctx(&mut f, SIZE_C);
                                let size = crate::native_cpu_tape::get(&mut f, sc, i, table.w.size);
                                let past = f.ins().iadd(i, size);
                                f.ins().jump(scan, &[BlockArg::from(past), BlockArg::from(gone)]);
                            }
                            GONE => {
                                // The node read going: none made last at this level until the next.
                                let kept = f.ins().select(is_root, zero, last);
                                mset(&mut f, Mem::LastMade, kept);
                                let d = f.ins().iconst(types::I64, i64::from(DEAD));
                                kind_c.put_row(&mut f, nrow, d);
                                let one = f.ins().iconst(types::I64, 1);
                                f.ins().jump(scan, &[BlockArg::from(i1), BlockArg::from(one)]);
                            }
                            m => {
                                let m = kind_of(&mut f, m, pc);
                                table.retag(&mut f, rows_here, i, m);
                                f.ins().jump(scan, &[BlockArg::from(i1), BlockArg::from(gone)]);
                            }
                        }
                        f.switch_to_block(miss);
                    }
                    let at = f.ins().iconst(types::I64, pc as i64 + 1);
                    f.ins().jump(refuse, &[BlockArg::from(at)]);
                    f.switch_to_block(after);
                    let gone = f.block_params(after)[0];
                    let (dropping, kept) = (f.create_block(), f.create_block());
                    f.ins().brif(gone, dropping, &[], kept, &[]);
                    f.switch_to_block(dropping);
                    f.set_cold_block(dropping);
                    drop_from(&mut f, &env, table, root);
                    f.ins().jump(fall, &[]);
                    f.switch_to_block(kept);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind, on: On::Here } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Here, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| here_word(g, e, table, q[0]));
                }
                Op::Merge { kind, fold: true } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Fold, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| fold_word(g, e, table, q[0], wrapping));
                }
                Op::Merge { kind, fold: false } => {
                    let end = end_at(pc);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Merge { end }, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| merge_word(g, e, table, q[0], end));
                }
                Op::NodeField { field, on } => {
                    let last = if on == On::Moved {
                        mget(&mut f, Mem::Moved)
                    } else {
                        let nodes = mget(&mut f, Mem::Nodes);
                        f.ins().iadd_imm(nodes, -1)
                    };
                    let x = f.ins().iconst(types::I64, i64::from(field));
                    field_c.put(&mut f, last, x);
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
                Op::Push { target, flag } if cut.calls => {
                    // A native call of the callee's function (`Cut::calls`): the calls open are
                    // counted (`ACCEPT` and `RETURN` check them), the held state stored for the
                    // callee and loaded again after, the level's word and flags kept in this
                    // frame. Back at the return's place, or the one after; any other value is
                    // the run's end, passed on. The call is a loop's head: back at the place after
                    // the return's (`RETURN m=1`), a rule switches on the same token again, and
                    // each turn calls a rule, which takes a token before it returns.
                    heads.push((wb[pc].expect("its own place"), Step::Return));
                    // Counted by the machine's stack (`on_stack`), as the lean parser's: nothing
                    // here, the depth checked where the callee is entered.
                    if !on_stack {
                        let ret = mget(&mut f, Mem::Ret);
                        let deep = f.ins().icmp_imm(IntCC::SignedGreaterThanOrEqual, ret, DEPTH);
                        guard(&mut f, deep, pc);
                        let ret = f.ins().iadd_imm(ret, 1);
                        mset(&mut f, Mem::Ret, ret);
                    }
                    let kept = (two_slots && !callee_keeps && changes_context[cut.of[callee(native, pc)] as usize]).then(|| (mget(&mut f, Mem::Word), mget(&mut f, Mem::Flags)));
                    // The flag the call runs with, as a parameter: the caller's flags given back
                    // after it (`let f = self.flags; ..; self.call(); self.flags = f`).
                    let flag_back = flag.map(|(k, set)| {
                        let fl = mget(&mut f, Mem::Flags);
                        let changed = if set { f.ins().bor_imm(fl, 1i64 << k) } else { f.ins().band_imm(fl, !(1i64 << k)) };
                        mset(&mut f, Mem::Flags, changed);
                        fl
                    });
                    for (k, v) in vars.iter().enumerate() {
                        if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                            let x = f.use_var(v);
                            let p = pinned(&mut f);
                            f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                        }
                    }
                    let e = callee(native, pc);
                    let d = cut.of[e] as usize;
                    let i = index[d][&e];
                    let fref = *call_refs.entry(d).or_insert_with(|| {
                        use cranelift_codegen::ir::{ExtFuncData, ExternalName, UserExternalName};
                        let name = f.func.declare_imported_user_function(UserExternalName::new(0, cids[d].as_u32()));
                        let signature = f.import_signature(csig.clone());
                        f.import_function(ExtFuncData { name: ExternalName::user(name), signature, colocated: true })
                    });
                    let iv = f.ins().iconst(types::I64, i as i64);
                    let passed: Vec<Value> = cut.pass.iter().map(|&k| f.use_var(vars[k as usize].expect("a passed field is held"))).collect();
                    let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([iv]).chain(passed).collect();
                    let call = f.ins().call(fref, &args);
                    let back = f.inst_results(call).to_vec();
                    let v = back[0];
                    for (j, &k) in cut.pass.iter().enumerate() {
                        f.def_var(vars[k as usize].expect("a passed field is held"), back[1 + j]);
                    }
                    for (k, var) in vars.iter().enumerate() {
                        if let Some(var) = var.filter(|_| !cut.pass.contains(&(k as u32))) {
                            let p = pinned(&mut f);
                            let x = f.ins().load(types::I64, MemFlags::trusted(), p, STATE + 8 * k as i32);
                            f.def_var(var, x);
                        }
                    }
                    if let Some((w, fl)) = kept {
                        mset(&mut f, Mem::Word, w);
                        mset(&mut f, Mem::Flags, fl);
                    }
                    if let Some(fl) = flag_back {
                        mset(&mut f, Mem::Flags, fl);
                    }
                    let (other, not_next) = (f.create_block(), f.create_block());
                    if let Some(way) = one_way(e) {
                        // Zero: back at the call's place, or the one after, as the callee always
                        // returns; else the run's end, passed on.
                        f.ins().brif(v, other, &[], word(target + way), &[]);
                        f.switch_to_block(not_next);
                        f.ins().jump(other, &[]);
                        f.switch_to_block(other);
                        f.set_cold_block(other);
                        let r = results(&mut f, v, false);
                        leave(&mut f, &r);
                        continue;
                    }
                    // Zero: back at the call's place; else one, the place after.
                    f.ins().brif(v, not_next, &[], word(target), &[]);
                    f.switch_to_block(not_next);
                    let again = f.ins().icmp_imm(IntCC::Equal, v, RETURNED as i64 + 1);
                    f.ins().brif(again, word(target + 1), &[], other, &[]);
                    f.switch_to_block(other);
                    f.set_cold_block(other);
                    let r = results(&mut f, v, false);
                    leave(&mut f, &r);
                }
                Op::Return { again } if cut.calls => {
                    // With the calls open on the stack, a call is always open here
                    // (`calls_on_stack`).
                    if !on_stack {
                        let ret = mget(&mut f, Mem::Ret);
                        let none = f.ins().icmp_imm(IntCC::Equal, ret, 0);
                        guard(&mut f, none, pc);
                        let ret = f.ins().iadd_imm(ret, -1);
                        mset(&mut f, Mem::Ret, ret);
                    }
                    for (k, v) in vars.iter().enumerate() {
                        if let Some(v) = v.filter(|_| !cut.pass.contains(&(k as u32))) {
                            let x = f.use_var(v);
                            let p = pinned(&mut f);
                            f.ins().store(MemFlags::trusted(), x, p, STATE + 8 * k as i32);
                        }
                    }
                    // The caller's token, taken here (`callee_takes`): a refusal is this word's.
                    if let (Some((any, extras)), false) = (landing_token, again) {
                        let extra: Option<(u8, u16)> = if extras { Some(*native.extras.first().ok_or(Error::UnsupportedBackend)?) } else { None };
                        let any_mark: Option<usize> = match any {
                            Some(k) => Some(native.block_programs.first().and_then(|b| b.masks.get(usize::from(k)).copied()).ok_or(Error::UnsupportedBackend)?),
                            None => None,
                        };
                        let shape = TokenShape { bound, in_code, rest_bits, end_bits, end_code: end_kind.map(i64::from), on_stack, extra, wrapping };
                        let (id, sig, _) = token_fn(&mut module, &mut word_fns, any_mark, shape, table, pinned_ctx)?;
                        let c = pinned(&mut f);
                        let r = call_word(&mut f, *id, sig, pinned_ctx, c, &[])[0];
                        let (no, taken) = (f.create_block(), f.create_block());
                        f.ins().brif(r, no, &[], taken, &[]);
                        f.switch_to_block(no);
                        f.set_cold_block(no);
                        let past = f.ins().icmp_imm(IntCC::Equal, r, 1);
                        let here = f.ins().iconst(types::I64, pc as i64 + 1);
                        let at = f.ins().select(past, here, r);
                        f.ins().jump(refuse, &[BlockArg::from(at)]);
                        f.switch_to_block(taken);
                    }
                    // A function that returns one way gives back 0 for it (`one_way`); with the
                    // caller's token taken here, every return (`callee_takes`).
                    let way = if callee_takes || (again && by_rule && again_as_one[pc]) { 0 } else { i64::from(again) };
                    let v = f.ins().iconst(types::I64, RETURNED as i64 + way);
                    let r = results(&mut f, v, false);
                    leave(&mut f, &r);
                }
                Op::Push { target, flag } => {
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
                    // The flag the call runs with: the caller's saved beside its return above.
                    if let Some((k, set)) = flag {
                        let fl = mget(&mut f, Mem::Flags);
                        let changed = if set { f.ins().bor_imm(fl, 1i64 << k) } else { f.ins().band_imm(fl, !(1i64 << k)) };
                        mset(&mut f, Mem::Flags, changed);
                    }
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
                        dispatch(&mut f, p, &t, u64::MAX);
                    }
                }
                Op::Accept => {
                    let (at_end, open) = (mget(&mut f, Mem::AtEnd), mget(&mut f, Mem::Open));
                    let not_end = f.ins().icmp_imm(IntCC::Equal, at_end, 0);
                    let not_one = f.ins().icmp_imm(IntCC::NotEqual, open, 1);
                    // A branch each, not the tests joined.
                    guard(&mut f, not_end, pc);
                    guard(&mut f, not_one, pc);
                    // Within no call (`calls_on_stack`), or checked.
                    if !on_stack {
                        let ret = mget(&mut f, Mem::Ret);
                        let returns = f.ins().icmp_imm(IntCC::NotEqual, ret, 0);
                        guard(&mut f, returns, pc);
                    }
                    let root = f.ins().uload32(flags, open_c, 0);
                    let root_row = NodeCol::row(&mut f, rows_here, root);
                    end_c.put_row(&mut f, root_row, len);
                    let nodes = mget(&mut f, Mem::Nodes);
                    let size = f.ins().isub(nodes, root);
                    let last = f.ins().iadd_imm(nodes, -1);
                    table.close_cells(&mut f, &env, root, size, last, len);
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
                            let row = NodeCol::row(&mut f, rows_here, i);
                            (field_c.get_row(&mut f, row), kind_c.get_row(&mut f, row))
                        };
                        let mut hit = f.ins().iconst(types::I8, 0);
                        for &p in &native.readings.provisional {
                            let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(p));
                            hit = f.ins().bor(hit, is);
                        }
                        let _ = k;
                        guard(&mut f, hit, pc);
                        let i1 = f.ins().iadd_imm(i, 1);
                        f.ins().jump(check, &[BlockArg::from(i1)]);
                        f.switch_to_block(after);
                    }
                    // A value past its width (an offset, a tail, the node count): refused at
                    // these widths (docs/parser-generators/37: parsed again at wider ones).
                    let wide = mget(&mut f, Mem::Wide);
                    guard(&mut f, wide, pc);
                    if table.w.size == 2 {
                        let many = f.ins().icmp_imm(IntCC::UnsignedGreaterThan, nodes, 0xffff);
                        guard(&mut f, many, pc);
                    }
                    // The codes of the nodes not yet committed, then the image's end.
                    table.commit(&mut f, &env, nodes);
                    let image_p = env_ctx(&mut f, IMAGE);
                    let cols = [SIZE_C, BASE_C, OFF_C, TAIL_C, CODE_KF, ID_OF].map(|at| env_ctx(&mut f, at));
                    let codes = mget(&mut f, Mem::Codes);
                    let (id, sig, _) = &table.fns.finish;
                    let total = call_own(&mut f, *id, sig, &[image_p, cols[0], cols[1], cols[2], cols[3], cols[4], cols[5], nodes, codes, len])[0];
                    f.ins().store(flags, nodes, counts_c, 0);
                    let supers = mget(&mut f, Mem::Supers);
                    f.ins().store(flags, supers, counts_c, 8);
                    // The image's length.
                    f.ins().store(flags, total, counts_c, 16);
                    f.ins().jump(fin, &[BlockArg::from(rows_here), BlockArg::from(nodes)]);
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
            // the tail call of its cluster with the place's index there. With a function a
            // grammar rule (`pinned_ctx`), the code of an earlier rule a word goes into
            // (`merge_words`): a call of that rule's function, whose result is this one's (the
            // lean parser's `return self.helper()`), to its one exit. A rule goes into earlier
            // rules alone, so these calls nest no deeper than the rules are many. One call for
            // each place gone to, its index made there, however many words go there: an index
            // passed in from many places would be one constant, folded, live through the whole
            // function. A tail call each was an epilogue each (785 restores, against 170).
            use cranelift_codegen::ir::{ExtFuncData, ExternalName, UserExternalName};
            let refs = &mut call_refs;
            let named_foreign: Vec<usize> = foreign.borrow().iter().copied().collect();
            let mut into: std::collections::BTreeMap<(usize, usize), Block> = std::collections::BTreeMap::new();
            for g in named_foreign {
                let d = cluster_of(g);
                // Named only by a word's fall it never takes (no word goes on to it).
                let Some(&i) = index[d].get(&g) else { continue };
                let b = if g < n_words { wb[g].expect("a place named") } else { entries[g - n_words] };
                f.switch_to_block(b);
                if pinned_ctx {
                    let to = *into.entry((d, i)).or_insert_with(|| f.create_block());
                    f.ins().jump(to, &[]);
                    exits.insert(b, g);
                    continue;
                }
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
                let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([iv]).chain(passed).collect();
                f.ins().return_call(fref, &args);
                exits.insert(b, g);
                tails += 1;
            }
            for ((d, i), to) in into {
                f.switch_to_block(to);
                let iv = f.ins().iconst(types::I64, i as i64);
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
                let args: Vec<Value> = std::iter::once(iv).chain(passed).collect();
                let call = f.ins().call(fref, &args);
                let v = f.inst_results(call)[0];
                let r = results(&mut f, v, false);
                leave(&mut f, &r);
            }
            // The entry's place, by its index among this cluster's entries.
            f.switch_to_block(pick);
            for &g in &incoming[c as usize] {
                targets.push((g, if g < n_words { wb[g].expect("an own place") } else { entries[g - n_words] }));
            }
            let at = f.block_params(entry)[first_param];
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
            let _ = p;
            let r = [zero];
            let r = results(&mut f, r[0], true);
            leave(&mut f, &r);
        }
        // Refusals are cold: placed after the hot code, branched to forward.
        f.switch_to_block(off_end);
        f.set_cold_block(off_end);
        let past = f.ins().iconst(types::I64, words.len() as i64 + 1);
        f.ins().jump(refuse, &[BlockArg::from(past)]);
        f.switch_to_block(refuse);
        f.set_cold_block(refuse);
        let pc = f.block_params(refuse)[0];
        // Inside a shared run, its words are the first place's: the shift makes them this one's.
        let sh = mget(&mut f, Mem::Shift);
        let pc = f.ins().iadd(pc, sh);
        let r = results(&mut f, pc, true);
        leave(&mut f, &r);
        f.switch_to_block(exit);
        let r = f.block_params(exit).to_vec();
        if let Some((w, fl)) = entry_kept {
            mset(&mut f, Mem::Word, w);
            mset(&mut f, Mem::Flags, fl);
        }
        f.ins().return_(&r);
        f.seal_all_blocks();
        f.finalize();
        }
        if !pinned_ctx {
            state_in_registers(&mut func);
        }
        drop_unused_loads(&mut func);
        lowered.push(Lowered { func, heads, targets, exits, tails });
    }
    if times {
        eprintln!("tier 2: lowered {:.2?}", phase.elapsed());
    }
    let mut word_fns: Vec<_> = word_fns.into_values().collect();
    word_fns.sort_by_key(|w| w.0.as_u32());
    let own: Vec<_> = table_fns.all().chain(word_fns).collect();
    let names: Vec<_> = own.iter().map(|(id, _, _)| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect();
    let crossings: usize = lowered.iter().map(|l| l.tails).sum();
    if clustered {
        check_across(&lowered)?;
    }
    let cluster_names: Vec<_> = cids.iter().map(|id| cranelift_codegen::ir::UserExternalName::new(0, id.as_u32())).collect();
    for l in &lowered {
        // With native calls, a rule's function calls the others as well as tail-calling them.
        let callable: Vec<_> = names.iter().chain(if cut.calls { &cluster_names[..] } else { &[] }).cloned().collect();
        crate::native_cpu_shape::check_tails(&l.func, &l.heads, &callable, if clustered { &cluster_names } else { &[] }).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
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
            // The first word is its cluster's first entry (`join_clusters`).
            let first = f.ins().iconst(types::I64, 0);
            // The passed fields' first values, as the state's in `ctx`.
            let passed: Vec<Value> = cut.pass.iter().map(|&k| if k == Mem::Need as u32 { one } else if k == Mem::Lw as u32 { before_first } else { zero }).collect();
            // With `ctx` pinned: the host's x21 kept, `ctx` there for the run, the host's back
            // after (Cranelift saves no pinned register).
            let host = pinned_ctx.then(|| {
                let host = f.ins().get_pinned_reg(types::I64);
                f.ins().set_pinned_reg(ctx);
                host
            });
            // The calls open on the machine's stack (`on_stack`): its base and limit, here.
            if on_stack {
                let sp = f.ins().get_stack_pointer(types::I64);
                f.ins().store(MemFlags::trusted(), sp, ctx, STACK_BASE);
                let limit = f.ins().iadd_imm(sp, -STACK_BYTES);
                f.ins().store(MemFlags::trusted(), limit, ctx, STACK_LIMIT);
            }
            let args: Vec<Value> = (!pinned_ctx).then_some(ctx).into_iter().chain([first]).chain(passed).collect();
            let r = call_own(&mut f, cids[cut.of[0] as usize], &csig, &args);
            if let Some(host) = host {
                f.ins().set_pinned_reg(host);
            }
            // The passed fields' last values, in the state, as the host reads it.
            for (j, &k) in cut.pass.iter().enumerate() {
                f.ins().store(MemFlags::trusted(), r[1 + j], ctx, STATE + 8 * k as i32);
            }
            let end = f.ins().bxor_imm(r[0], ENDED as i64);
            f.ins().return_(&[end]);
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
    let has_driver = driver.is_some();
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
    let ir: Vec<(u32, cranelift_codegen::ir::Function)> = if KEEP_IR.load(std::sync::atomic::Ordering::Relaxed) { items.iter().map(|(fid, c)| (fid.as_u32(), c.func.clone())).collect() } else { Vec::new() };
    let defined = crate::native_cpu_codegen::define_functions(&mut module, items, cut.near).map_err(|e| {
        if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
            eprintln!("tier 2: {e:?}");
        }
        Error::BackendFailure
    })?;
    if times {
        eprintln!("tier 2: compiled {:.2?}", phase.elapsed());
    }
    // The smallest rule function's frame (its saved registers, slots, and the frame pointer and
    // return address): each call open has one, or more (`calls_open_by_stack`).
    let frame_shift = defined
        .iter()
        .skip(usize::from(has_driver))
        .take(cut.count as usize)
        .filter_map(|c| c.compiled_code().map(|code| code.frame_size + 16))
        .min()
        .map_or(4, |least| u64::from(least.max(16).ilog2()));
    let mut disassembly = String::new();
    let mut code_bytes = Vec::new();
    let mut word_bytes = vec![0u32; words.len()];
    let mut word_ranges = Vec::new();
    for c in &defined {
        if let Some(code) = c.compiled_code() {
            for l in code.buffer.get_srclocs_sorted() {
                if let Some(pc) = (l.loc.bits() as usize).checked_sub(1).filter(|&pc| pc < words.len()) {
                    word_bytes[pc] += l.end - l.start;
                    let at = code_bytes.len() as u32;
                    word_ranges.push((pc as u32, at + l.start, at + l.end));
                }
            }
        }
        disassembly.push_str(&c.compiled_code().and_then(|code| code.vcode.clone()).unwrap_or_default());
        code_bytes.extend_from_slice(c.compiled_code().map_or(&[][..], |code| code.code_buffer()));
    }
    // A call's line, its source location (`@` and the word's place) before it or not.
    let calls = clif.lines().filter(|l| l.trim().starts_with("call ") || l.contains(" call ")).count();
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
    Ok(CompiledStream { _module: module, entry, frame_shift, bounds: bounds(native), starts, lasts, marks, wraps, widths, tag, context: two_slots, clif, disassembly, code_bytes, heads, own_calls, clusters: cut.count as usize, crossings, word_insts, word_bytes, word_ranges, ir, entry_id: id.as_u32() })
}

/// What an instruction does to `ctx`'s fields (`state_in_registers`).
#[derive(Clone, Copy, PartialEq)]
enum Access {
    Load(usize),
    Store(usize, Value),
    /// A narrow load (the record's byte), or a field not on a word boundary.
    Reads(u64),
    /// A narrow store: reads (not all of it written) and changes it.
    Clobbers(usize),
    /// A call, a return, `ctx` elsewhere: every field.
    All,
    None,
}

/// The fields' constants (`state_in_registers`): forward, to a fixed point, each field at each
/// block's start is a constant every predecessor ends with, or not known. A load of a known
/// field is its constant; a branch on a constant is a jump. Whether anything changed.
fn constant_fields(func: &mut cranelift_codegen::ir::Function, access: &dyn Fn(&cranelift_codegen::ir::Function, cranelift_codegen::ir::Inst) -> Access) -> bool {
    use cranelift_codegen::cursor::{Cursor, FuncCursor};
    use cranelift_codegen::ir::{InstBuilder, InstructionData, Opcode};
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        /// No path seen yet.
        Top,
        Is(i64),
        Unknown,
    }
    let meet = |a: C, b: C| match (a, b) {
        (C::Top, x) | (x, C::Top) => x,
        (C::Is(x), C::Is(y)) if x == y => C::Is(x),
        _ => C::Unknown,
    };
    let constant = |func: &cranelift_codegen::ir::Function, v: Value| -> Option<i64> {
        let v = func.dfg.resolve_aliases(v);
        match func.dfg.value_def(v) {
            cranelift_codegen::ir::ValueDef::Result(i, _) => match func.dfg.insts[i] {
                InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => Some(imm.bits()),
                _ => None,
            },
            _ => None,
        }
    };
    let n = (STATE / 8 + Mem::COUNT as i32) as usize;
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let dom = cranelift_codegen::dominator_tree::DominatorTree::with_function(func, &cfg);
    let order: Vec<Block> = dom.cfg_rpo().copied().collect();
    let Some(entry) = func.layout.entry_block() else { return false };
    let transfer = |func: &cranelift_codegen::ir::Function, b: Block, mut at: Vec<C>| -> Vec<C> {
        for inst in func.layout.block_insts(b) {
            match access(func, inst) {
                Access::Store(k, v) => at[k] = constant(func, v).map_or(C::Unknown, C::Is),
                Access::Clobbers(k) => at[k] = C::Unknown,
                Access::All => at.iter_mut().for_each(|x| *x = C::Unknown),
                _ => {}
            }
        }
        at
    };
    let mut start: std::collections::HashMap<Block, Vec<C>> = order.iter().map(|&b| (b, vec![C::Top; n])).collect();
    start.insert(entry, vec![C::Unknown; n]);
    let mut end: std::collections::HashMap<Block, Vec<C>> = std::collections::HashMap::new();
    loop {
        let mut changed = false;
        for &b in &order {
            let at = if b == entry {
                vec![C::Unknown; n]
            } else {
                let mut at = vec![C::Top; n];
                for p in cfg.pred_iter(b) {
                    if let Some(e) = end.get(&p.block) {
                        for k in 0..n {
                            at[k] = meet(at[k], e[k]);
                        }
                    }
                }
                at
            };
            if start.get(&b) != Some(&at) || !end.contains_key(&b) {
                let e = transfer(func, b, at.clone());
                start.insert(b, at);
                if end.get(&b) != Some(&e) {
                    end.insert(b, e);
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // The loads of known fields, then the branches on constants.
    let mut any = false;
    for &b in &order {
        let mut at = start[&b].clone();
        let insts: Vec<_> = func.layout.block_insts(b).collect();
        for inst in insts {
            let a = access(func, inst);
            if let Access::Load(k) = a
                && let C::Is(c) = at[k]
            {
                let r = func.dfg.inst_results(inst)[0];
                let mut cur = FuncCursor::new(func).at_inst(inst);
                let x = cur.ins().iconst(types::I64, c);
                func.dfg.detach_inst_results(inst);
                func.dfg.change_to_alias(r, x);
                func.layout.remove_inst(inst);
                any = true;
                continue;
            }
            // A store of the constant the field holds: nothing changes.
            if let Access::Store(k, v) = a
                && let Some(c) = constant(func, v)
                && at[k] == C::Is(c)
            {
                func.layout.remove_inst(inst);
                any = true;
                continue;
            }
            match a {
                Access::Store(k, v) => at[k] = constant(func, v).map_or(C::Unknown, C::Is),
                Access::Clobbers(k) => at[k] = C::Unknown,
                Access::All => at.iter_mut().for_each(|x| *x = C::Unknown),
                _ => {}
            }
        }
        if let Some(last) = func.layout.last_inst(b)
            && let InstructionData::Brif { arg, blocks: [then, other], .. } = func.dfg.insts[last]
            && let Some(c) = constant(func, arg)
        {
            let taken = if c != 0 { then } else { other };
            let target = taken.block(&func.dfg.value_lists);
            let args: Vec<cranelift_codegen::ir::BlockArg> = taken.args(&func.dfg.value_lists).collect();
            func.layout.remove_inst(last);
            let mut cur = FuncCursor::new(func).at_bottom(b);
            cur.ins().jump(target, &args);
            any = true;
        }
    }
    any
}

/// `ctx`'s fields as the lean parser's locals: within a function, a field's value goes from
/// the word that stores it to the word that reads it in a register, not through memory.
///
/// - **Forwarded:** along a chain of blocks each of one predecessor (a word falling into the
///   next), a field's load after its store or an earlier load is that value. A call ends what
///   is known (the callee may write any field).
/// - **Dead stores:** a store every path writes again before any read is removed (a backward
///   pass over the fields, a bit each). A call or a return reads every field (the callee and
///   the host read the state).
///
/// The fields are `ctx`'s, at fixed offsets from its address (the function's first argument);
/// the tape's buffers are elsewhere. A function where `ctx` goes anywhere but a load's or a
/// store's address, or a call, is left as it is.
fn state_in_registers(func: &mut cranelift_codegen::ir::Function) {
    use cranelift_codegen::ir::{InstructionData, Opcode};
    let Some(entry) = func.layout.entry_block() else { return };
    let ctx = func.dfg.block_params(entry)[0];
    // Field `k` at offset `8k`: a full load or store of it, or another access of it.
    const FIELDS: i32 = STATE / 8 + Mem::COUNT as i32;
    const _: () = assert!(FIELDS <= 64);
    let field_of = |off: i32| -> Option<usize> { (0..8 * FIELDS).contains(&off).then_some((off / 8) as usize) };
    let access = |func: &cranelift_codegen::ir::Function, inst: cranelift_codegen::ir::Inst| -> Access {
        let data = &func.dfg.insts[inst];
        let op = data.opcode();
        if op.is_call() || op.is_return() {
            return Access::All;
        }
        match *data {
            InstructionData::Load { opcode, arg, offset, .. } if func.dfg.resolve_aliases(arg) == ctx => {
                let off = i32::from(offset);
                let Some(k) = field_of(off) else { return Access::All };
                let full = opcode == Opcode::Load && off % 8 == 0 && func.dfg.value_type(func.dfg.inst_results(inst)[0]) == types::I64;
                if full { Access::Load(k) } else { Access::Reads(1 << k) }
            }
            InstructionData::Store { opcode, args, offset, .. } if func.dfg.resolve_aliases(args[1]) == ctx => {
                let off = i32::from(offset);
                let Some(k) = field_of(off) else { return Access::All };
                if func.dfg.resolve_aliases(args[0]) == ctx {
                    return Access::All;
                }
                let full = opcode == Opcode::Store && off % 8 == 0 && func.dfg.value_type(args[0]) == types::I64;
                if full { Access::Store(k, args[0]) } else { Access::Clobbers(k) }
            }
            _ => {
                if func.dfg.inst_values(inst).any(|v| func.dfg.resolve_aliases(v) == ctx) {
                    Access::All
                } else {
                    Access::None
                }
            }
        }
    };
    let blocks: Vec<Block> = func.layout.blocks().collect();
    // `ctx` as a branch's argument: another name for it, which this does not follow.
    for &b in &blocks {
        for inst in func.layout.block_insts(b) {
            if func.dfg.insts[inst].opcode().is_branch() && func.dfg.inst_values(inst).any(|v| func.dfg.resolve_aliases(v) == ctx) {
                return;
            }
        }
    }
    // Constants: a field every path into a word gives one constant (the transducer's `need`
    // after a `SPAN`, `pending` after a node) is that constant there, and a branch on it is a
    // jump. Folded branches make more chains of one predecessor: again, a few times.
    for _ in 0..4 {
        if !constant_fields(func, &access) {
            break;
        }
    }
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let dom = cranelift_codegen::dominator_tree::DominatorTree::with_function(func, &cfg);
    let order: Vec<Block> = dom.cfg_rpo().copied().collect();

    // Forwarded, in reverse postorder: a block of one predecessor starts from what its
    // predecessor knew at its end.
    let mut known_at_end: std::collections::HashMap<Block, Vec<Option<Value>>> = std::collections::HashMap::new();
    for &b in &order {
        let preds: Vec<Block> = cfg.pred_iter(b).map(|p| p.block).collect();
        let mut known: Vec<Option<Value>> = match preds.as_slice() {
            [p] if *p != b => known_at_end.get(p).cloned().unwrap_or_else(|| vec![None; FIELDS as usize]),
            _ => vec![None; FIELDS as usize],
        };
        let insts: Vec<_> = func.layout.block_insts(b).collect();
        for inst in insts {
            match access(func, inst) {
                Access::Load(k) => {
                    let r = func.dfg.inst_results(inst)[0];
                    match known[k] {
                        Some(v) => {
                            func.dfg.detach_inst_results(inst);
                            func.dfg.change_to_alias(r, v);
                            func.layout.remove_inst(inst);
                        }
                        None => known[k] = Some(r),
                    }
                }
                // A store of the value the field holds (loaded or stored before): none.
                Access::Store(k, v) if known[k].is_some_and(|x| func.dfg.resolve_aliases(x) == func.dfg.resolve_aliases(v)) => func.layout.remove_inst(inst),
                Access::Store(k, v) => known[k] = Some(v),
                Access::Clobbers(k) => known[k] = None,
                Access::All => known.iter_mut().for_each(|x| *x = None),
                Access::Reads(_) | Access::None => {}
            }
        }
        known_at_end.insert(b, known);
    }

    // Dead stores: the fields live (read before written again) at each block's start, to a
    // fixed point, then each block backward.
    let all = if FIELDS == 64 { u64::MAX } else { (1u64 << FIELDS) - 1 };
    let step = |a: Access, live: u64| -> u64 {
        match a {
            Access::Load(k) => live | 1 << k,
            Access::Store(k, _) => live & !(1 << k),
            Access::Reads(m) => live | m,
            Access::Clobbers(k) => live | 1 << k,
            Access::All => all,
            Access::None => live,
        }
    };
    let succs = |func: &cranelift_codegen::ir::Function, b: Block| -> Vec<Block> {
        let Some(last) = func.layout.last_inst(b) else { return Vec::new() };
        func.dfg.insts[last].branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables).iter().map(|c| c.block(&func.dfg.value_lists)).collect()
    };
    let accesses: std::collections::HashMap<Block, Vec<(cranelift_codegen::ir::Inst, Access)>> =
        blocks.iter().map(|&b| (b, func.layout.block_insts(b).map(|i| (i, access(func, i))).collect())).collect();
    let next: std::collections::HashMap<Block, Vec<Block>> = blocks.iter().map(|&b| (b, succs(func, b))).collect();
    let mut live_in: std::collections::HashMap<Block, u64> = blocks.iter().map(|&b| (b, 0)).collect();
    loop {
        let mut changed = false;
        for &b in order.iter().rev() {
            let mut live = next[&b].iter().fold(0, |m, s| m | live_in[s]);
            for &(_, a) in accesses[&b].iter().rev() {
                live = step(a, live);
            }
            if live != live_in[&b] {
                live_in.insert(b, live);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for &b in &order {
        let mut live = next[&b].iter().fold(0, |m, s| m | live_in[s]);
        for &(inst, a) in accesses[&b].iter().rev() {
            if let Access::Store(k, _) = a
                && live & 1 << k == 0
            {
                func.layout.remove_inst(inst);
                continue;
            }
            live = step(a, live);
        }
    }
}

/// Every load that can not trap and whose value nothing reads, removed, until none is left. A
/// word loads the `ctx` fields it may read where it starts (`Cut::reload`); those it does not
/// read on a path are dead, but Cranelift keeps a load that is not read-only (it orders it with
/// the stores). A load that can not trap has no effect but its value: removing an unread one
/// changes nothing, as the lean parser's code loads a field only where it is used.
fn drop_unused_loads(func: &mut cranelift_codegen::ir::Function) {
    loop {
        let mut used = std::collections::HashSet::new();
        let blocks: Vec<Block> = func.layout.blocks().collect();
        for &b in &blocks {
            for inst in func.layout.block_insts(b) {
                for v in func.dfg.inst_values(inst) {
                    used.insert(func.dfg.resolve_aliases(v));
                }
            }
        }
        let mut dead = Vec::new();
        for &b in &blocks {
            for inst in func.layout.block_insts(b) {
                let data = &func.dfg.insts[inst];
                let op = data.opcode();
                let quiet = data.memflags().is_some_and(|m| m.trap_code().is_none());
                if op.can_load() && !op.can_store() && !op.is_call() && quiet && func.dfg.inst_results(inst).iter().all(|r| !used.contains(r)) {
                    dead.push(inst);
                }
            }
        }
        if dead.is_empty() {
            return;
        }
        for inst in dead {
            func.layout.remove_inst(inst);
        }
    }
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
        // With any mark's mask, `TOKEN` reads it (an output).
        Op::Token { .. } => vec![STARTS, NSTARTS, LASTS, CODES, ROOM_NODES, ROOM_SUPERS, ROOM_PUSHES, OUTPUTS, OUTPUT_STRIDE, LASTS_STRIDE],
        Op::Switch { .. } => vec![BYTES],
        Op::Mark { .. } | Op::Row { .. } => vec![OUTPUTS, OUTPUT_STRIDE, LASTS_STRIDE],
        Op::Open { .. } => vec![ROWS, OPEN],
        Op::Close { .. } => vec![OPEN, ROWS],
        Op::Leaf { .. } => vec![ROWS],
        Op::Retag { on: On::Moved, .. } => vec![ROWS],
        Op::Retag { on: On::Here, .. } => vec![OPEN, ROWS],
        Op::Merge { .. } => vec![OPEN, ROWS],
        Op::NodeField { on: On::Moved | On::Here, .. } => vec![ROWS],
        Op::NodeSuper { .. } => vec![SUPER_NODE, SUPER_KIND],
        Op::Push { .. } | Op::Return { .. } => vec![RETURNS],
        Op::Empty { .. } => vec![TOTALS, TOTALS_STRIDE, TOTALS_MASK],
        _ => vec![],
    };
    if wrapping && matches!(op, Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::Merge { .. }) {
        r.extend([FRAME_BASE, HEAD, LINK, PLACE]);
    }
    r
}

/// The words that read many `ctx` fields and run rarely: each loads them all (`Cut::reload`).
fn cold_op(op: &Op) -> bool {
    matches!(op, Op::Accept | Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made | On::Inner, .. } | Op::NodeField { on: On::Made | On::MadeUnset | On::Inner, .. })
}

/// Whether the program has context beside each call's return (a level's word, its flags): two
/// return slots a call.
fn has_context(words: &[crate::native_ir::StructuralBlock]) -> bool {
    words.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. } | Op::Push { flag: Some(_), .. }))
}

/// Whether the program keeps a frame per open node: it wraps, or it relabels the node made last
/// at a level (`On::Made`).
fn framed(words: &[crate::native_ir::StructuralBlock]) -> bool {
    words.iter().any(|b| matches!(b.operation, Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made, .. } | Op::NodeField { on: On::Made | On::MadeUnset | On::Inner, .. }))
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
    /// The records' last bytes and codes, zipped with `starts` (`records`).
    pub lasts: *const u32,
    pub codes: *const u32,
    pub lasts_stride: usize,
    pub outputs: *const u64,
    pub output_stride: usize,
    pub totals: *const u8,
    pub totals_stride: usize,
    pub totals_mask: u64,
}

/// The buffers one call writes, sized from the stream and the program's `Bounds`.
pub struct Tape {
    /// The working rows, 12 bytes a node (key, start, end): never part of the tape.
    rows: Vec<u8>,
    /// The tape's columns at the compiled widths (`SLACK` bytes past each), the bases, and the
    /// image: the header's room, the code column in place, then room for the columns moved
    /// after it and the code table (`native_cpu_tape::finish`).
    size: Vec<u8>,
    off: Vec<u8>,
    tail: Vec<u8>,
    base: Vec<u32>,
    pub image: Vec<u8>,
    /// The code ids: each key's (the thread's table, cleared by the image's end), each code's key
    /// and first node.
    id_of: Vec<u16>,
    code_kf: Vec<u16>,
    code_first: Vec<u32>,
    pub super_node: Vec<u32>,
    pub super_kind: Vec<u32>,
    open: Vec<u32>,
    returns: Vec<u32>,
    /// The node count, the supertype count, the image's length.
    pub counts: [u64; 3],
    /// A program with `WRAP`'s: frames, chains, places (a drop's stack: two slots a node).
    frames: [Vec<u32>; 3],
    chains: [Vec<u32>; 3],
}

thread_local! {
    /// The code ids by key, kept between a thread's tapes: each image's end clears what it set.
    static IDS: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Drop for Tape {
    /// The id table back to the thread. A run that stopped before its end clears what it set.
    fn drop(&mut self) {
        let mut ids = std::mem::take(&mut self.id_of);
        if self.counts[2] == 0 {
            ids.fill(crate::native_cpu_tape::NO_CODE as u16);
        }
        IDS.with(|t| *t.borrow_mut() = ids);
    }
}

impl CompiledStream {
    /// The widths the tape is written at, and the language's tag.
    pub fn widths_tag(&self) -> (rerail_tape_format::Widths, u8) {
        (self.widths, self.tag)
    }

    /// Room for a stream of `n` starts: from the token count, as the lean parser sizes its
    /// tape (half again as many nodes as tokens; on the `.rr` corpus, 0.76 a token), and a
    /// token's path past it. A run that could pass it stops at a token word (`OVERFLOW`) and
    /// `call` runs it again on `full_tape`.
    pub fn tape(&self, n: usize) -> Tape {
        let b = &self.bounds;
        let tokens = n as u64 + 2;
        self.sized((tokens + tokens / 2 + b.nodes + 1) as usize, (tokens / 2 + b.supers + 1) as usize, (tokens / 4 + b.pushes + 64) as usize)
    }

    /// A tape with the program's bound's room for `n` starts: no run passes it.
    fn full_tape(&self, n: usize) -> Tape {
        // A token is switched on once, and once more after each mark's arm.
        let paths = (n as u64 + 2) * (1 + self.marks);
        self.sized((self.bounds.nodes * paths + 1) as usize, (self.bounds.supers * paths + 1) as usize, (self.bounds.pushes * paths + 1) as usize)
    }

    /// The buffers, room only: not zeroed. The code writes every slot before it reads it (a
    /// record's columns and its chain's head where it is made, a frame where it opens, the
    /// table's passes over the records made); the columns' lengths are set to what it wrote
    /// (`Tape::written`). `RERAIL_TAPE_POISON=1` fills them with ones instead, to show a slot
    /// read before it is written.
    fn sized(&self, nodes: usize, supers: usize, pushes: usize) -> Tape {
        use crate::native_cpu_tape::SLACK;
        let poison = std::env::var_os("RERAIL_TAPE_POISON").is_some();
        let room = |n: usize| -> Vec<u32> { if poison { vec![u32::MAX; n] } else { Vec::with_capacity(n) } };
        let bytes = |n: usize| -> Vec<u8> { if poison { vec![u8::MAX; n] } else { Vec::with_capacity(n) } };
        let wraps = |n: usize| if self.wraps { n } else { 0 };
        let w = self.widths;
        let pad = rerail_tape_format::padded;
        let blocks = nodes.div_ceil(rerail_tape_format::CHECKPOINT_STRIDE) + 1;
        let image = rerail_tape_format::HEADER_BYTES + pad(2 * nodes) + pad(usize::from(w.size) * nodes) + pad(4 * blocks) + pad(usize::from(w.off) * nodes) + pad(usize::from(w.tail) * nodes) + 6 * nodes + 2 * SLACK;
        let id_of = IDS.with(|t| {
            let mut ids = std::mem::take(&mut *t.borrow_mut());
            ids.resize(crate::native_cpu_tape::ID_SLOTS, crate::native_cpu_tape::NO_CODE as u16);
            ids
        });
        let mut t = Tape {
            rows: bytes(ROW as usize * nodes + SLACK),
            size: bytes(usize::from(w.size) * nodes + SLACK),
            off: bytes(usize::from(w.off) * nodes + SLACK),
            tail: bytes(usize::from(w.tail) * nodes + SLACK),
            base: room(blocks + SLACK),
            image: bytes(image),
            id_of,
            code_kf: Vec::with_capacity(nodes + 1),
            code_first: room(nodes + 1),
            super_node: room(supers),
            super_kind: room(supers),
            open: room(nodes),
            returns: room(if self.context { 2 * pushes } else { pushes }),
            counts: [0; 3],
            frames: std::array::from_fn(|_| room(wraps(nodes + 1))),
            chains: std::array::from_fn(|k| room(wraps(if k == 2 { 2 * nodes + 2 } else { nodes }))),
        };
        t.clear();
        t
    }

    /// Runs another build of the same code from now on: `entry` for the JIT's, and the log of
    /// its smallest rule function's frame (`Ctx::frame_shift`; 4, a frame pointer and a return
    /// address, holds for any).
    ///
    /// # Safety
    /// `entry` must be this program's code (`clif_rust` of `ir`), reading and writing `Ctx` as the
    /// JIT's does.
    pub unsafe fn set_entry(&mut self, entry: Entry, frame_shift: u64) {
        self.entry = entry;
        self.frame_shift = frame_shift;
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
            // SAFETY: the code wrote the image and supertypes it counted.
            unsafe { tape.written(r == 0, std::slice::from_raw_parts(bytes, len)) };
            return r;
        }
        ROOM_RETRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *tape = self.full_tape(tier1.nstarts);
        // SAFETY: as above, on the bound's tape.
        let r = unsafe { self.call_once(bytes, len, tier1, tape) };
        unsafe { tape.written(r == 0, std::slice::from_raw_parts(bytes, len)) };
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
            codes: tier1.codes,
            totals: tier1.totals.cast(),
            rows: tape.rows.as_mut_ptr(),
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
            out_rows: std::ptr::null_mut(),
            output_stride: tier1.output_stride as u64,
            totals_stride: tier1.totals_stride as u64,
            totals_mask: tier1.totals_mask,
            room_nodes: (tape.code_first.capacity() - 1) as u64,
            room_supers: tape.super_node.capacity() as u64,
            room_pushes: (tape.returns.capacity() / if self.context { 2 } else { 1 }) as u64,
            stack_base: 0,
            stack_limit: 0,
            frame_shift: self.frame_shift,
            state: [0; Mem::COUNT as usize],
            size_c: tape.size.as_mut_ptr(),
            off_c: tape.off.as_mut_ptr(),
            tail_c: tape.tail.as_mut_ptr(),
            base_c: tape.base.as_mut_ptr(),
            image: tape.image.as_mut_ptr(),
            id_of: tape.id_of.as_mut_ptr(),
            code_kf: tape.code_kf.as_mut_ptr(),
            code_first: tape.code_first.as_mut_ptr(),
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
        let lasts = stream(self.lasts);
        let mut codes = tier1.codes.clone();
        codes.extend([0, 0]);
        let mut tape = self.tape(starts.1);
        let totals = if tier1.totals.is_empty() { vec![0] } else { tier1.totals.clone() };
        // SAFETY: `padded` has the byte before and 64 after; each stream two slots past its count;
        // the tape is sized for the stream; every output has `count` words in `all`.
        let view = Tier1View {
            starts: starts.0.as_ptr(),
            nstarts: starts.1.min(lasts.1),
            lasts: lasts.0.as_ptr(),
            codes: codes.as_ptr(),
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

impl Tape {
    /// The columns' lengths: none, before a call (the code writes through their room).
    fn clear(&mut self) {
        for v in [&mut self.super_node, &mut self.super_kind, &mut self.open, &mut self.returns, &mut self.base, &mut self.code_first] {
            v.clear();
        }
        for v in self.frames.iter_mut().chain(&mut self.chains) {
            v.clear();
        }
        for v in [&mut self.rows, &mut self.size, &mut self.off, &mut self.tail, &mut self.image] {
            v.clear();
        }
        self.code_kf.clear();
    }

    /// The lengths after a call: where it accepted, the image's (`counts[2]`), its digest of
    /// `src` written (the header's SHA-256: docs/parser-generators/37), and the supertypes;
    /// else none.
    ///
    /// # Safety
    /// The call wrote, where it accepted, the image's `counts[2]` bytes and `counts[1]`
    /// supertypes.
    unsafe fn written(&mut self, accepted: bool, src: &[u8]) {
        let (len, s) = (self.counts[2] as usize, self.counts[1] as usize);
        self.clear();
        if !accepted {
            self.counts[2] = 0;
            return;
        }
        // SAFETY: the caller's contract; each is within its room (the code checks it).
        unsafe {
            for v in [&mut self.super_node, &mut self.super_kind] {
                v.set_len(s);
            }
            self.image.set_len(len);
        }
        self.image[24..56].copy_from_slice(&rerail_tape_format::digest(src));
    }

    /// The nodes the tape has room for.
    pub fn room_nodes(&self) -> usize {
        self.code_first.capacity() - 1
    }

    /// The engine's tape: the image's bytes, as tier 2 wrote them (no copy).
    pub fn image(mut self) -> Vec<u8> {
        std::mem::take(&mut self.image)
    }

    /// The tape's nodes as `stream::Node`s (the tests' comparisons with the interpreter), read
    /// back from the image: each node's code's kind and field, its start (its block's base and
    /// its offset), its size, and its end (its tail from where tails are measured).
    pub fn nodes(&self) -> Vec<Node> {
        let b = &self.image;
        let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"));
        let (n, codes) = (u64_at(56) as usize, u64_at(64) as usize);
        let (ws, wo, wt) = (usize::from(b[9]), usize::from(b[10]), usize::from(b[11] & 0x7f));
        let from_start = b[11] & rerail_tape_format::TAIL_FROM_START != 0;
        let pad = rerail_tape_format::padded;
        let code_at = rerail_tape_format::HEADER_BYTES;
        let size_at = pad(code_at + 2 * n);
        let base_at = pad(size_at + ws * n);
        let off_at = pad(base_at + 4 * n.div_ceil(rerail_tape_format::CHECKPOINT_STRIDE));
        let tail_at = pad(off_at + wo * n);
        let table_at = pad(tail_at + wt * n);
        let cell = |at: usize, w: usize, i: usize| -> u32 {
            let c = &b[at + w * i..at + w * i + w];
            if w == 2 { u32::from(u16::from_le_bytes([c[0], c[1]])) } else { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) }
        };
        let kfs: Vec<u16> = (0..codes).map(|c| u16::from_le_bytes([b[table_at + 6 * c], b[table_at + 6 * c + 1]])).collect();
        let starts: Vec<u32> = (0..n).map(|i| cell(base_at, 4, i / rerail_tape_format::CHECKPOINT_STRIDE) + cell(off_at, wo, i)).collect();
        let mut out: Vec<Node> = (0..n)
            .map(|i| {
                let kf = kfs[cell(code_at, 2, i) as usize];
                let size = cell(size_at, ws, i);
                let from = if from_start { starts[i] } else { starts[i + size as usize - 1] };
                Node { kind: kf & 0x3ff, field: kf >> rerail_tape_format::KIND_BITS, supers: Vec::new(), start: starts[i], end: from.wrapping_add(cell(tail_at, wt, i)), size }
            })
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

    /// The engine's tape of `bytes` as tier 2 wrote it (its bytes), or the word that refused
    /// it (the file goes to the fallback).
    pub fn image(&self, bytes: &[u8]) -> Result<Vec<u8>, Refused> {
        let mut batch = crate::native_cpu_block::Batch::new(&self.tier1, &[bytes]);
        batch.call(&self.tier1);
        let mut tape = self.tape(&batch);
        self.words(&batch, &mut tape)?;
        Ok(tape.image())
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
        // The records' other columns: the last bytes, and the codes after the extracts'.
        let lasts = self.stream(batch, self.tier2.lasts);
        let codes = batch.positions[(row.positions + extracts as u64 * row.capacity) as usize..].as_ptr();
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
            nstarts: starts.1.min(lasts.1),
            lasts: lasts.0.as_ptr(),
            codes,
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
