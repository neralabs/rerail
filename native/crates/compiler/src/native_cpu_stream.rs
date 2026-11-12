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
    /// The records' other columns (`records`): each token's last byte and its code, zipped
    /// with `starts`.
    lasts: *const u32,
    codes: *const u32,
    totals: *const u64,
    /// The node table, a row a node (`NodeRow`), as the lean parser's tape.
    rows: *mut NodeRow,
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
    /// The node table in preorder, written at `ACCEPT` when the file had a wrap.
    out_rows: *mut NodeRow,
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

/// A node of the table, as the lean parser's tape has one (`rr_syntax::lean::Node`, 24 bytes):
/// its kind and field, its bytes, and (where the lean parser has its parent) the records in its
/// subtree, its own counted; its parent and whether it is an extra, made at `ACCEPT` (`table`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeRow {
    pub kind: u16,
    pub field: u16,
    pub start: u32,
    pub end: u32,
    pub size: u32,
    /// The parent's record; `NO_PARENT` for the root.
    pub parent: u32,
    /// 1 for an extra (a comment).
    pub extra: u8,
    pub pad: [u8; 3],
}

/// A node row's bytes, and where its fields are in it.
const ROW: i64 = 24;
const AT_KIND: i32 = 0;
const AT_FIELD: i32 = 2;
const AT_START: i32 = 4;
const AT_END: i32 = 8;
const AT_SIZE: i32 = 12;
const AT_PARENT: i32 = 16;
const AT_EXTRA: i32 = 20;
const _: () = assert!(
    std::mem::size_of::<NodeRow>() == ROW as usize
        && std::mem::offset_of!(NodeRow, field) == AT_FIELD as usize
        && std::mem::offset_of!(NodeRow, start) == AT_START as usize
        && std::mem::offset_of!(NodeRow, end) == AT_END as usize
        && std::mem::offset_of!(NodeRow, size) == AT_SIZE as usize
        && std::mem::offset_of!(NodeRow, parent) == AT_PARENT as usize
        && std::mem::offset_of!(NodeRow, extra) == AT_EXTRA as usize
);

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

    /// Row `i`'s field, stored (its width: kind and field 16 bits, the extra flag 8).
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
            AT_EXTRA => f.ins().istore8(flags, x, a, self.at),
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
            AT_EXTRA => f.ins().uload8(types::I64, flags, a, self.at),
            _ => f.ins().uload32(flags, a, self.at),
        }
    }
}

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
        Op::Push { target } => vec![usize::from(target), usize::from(target) + 1],
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
    /// With wraps: the last node made at the innermost open node's level, one more than its
    /// record (0: none), and that node's base. Only the innermost level's is ever read, and
    /// each change of level writes the new one's: two fields, not a column by depth.
    LastMade,
    LastBase,
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
    const COUNT: u32 = 25;
}

/// The fields a rule's function holds in registers (`Cut::held`) and passes to its calls, given
/// back with their results (`Cut::pass`): none. The whole state stays in memory, read and
/// written where used, as the lean parser's `&mut self` fields are: it reloads `self.i` and the
/// token where it tests them, and keeps nothing live across a rule. Held and passed, the
/// token's code, start, end and end flag were live through every block and spilled (15,133
/// stack loads over `.rr`'s tier 2, against 161 in the lean parser).
pub const HELD_IN_RULES: [u32; 0] = [];

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

/// Compile a program's tier-2 words.
/// The columns `reorder` reads and writes.
struct Reorder {
    nodes: Value,
    supers: Value,
    wraps: Value,
    /// The node rows: as made, and in preorder.
    rows: Value,
    out: Value,
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
        [self.nodes, self.supers, self.wraps, self.rows, self.out, self.head, self.link, self.place, self.snode, self.counts].into_iter().chain(self.before).collect()
    }

    /// The same, from the parameters `p` of its function.
    fn of(p: &[Value], before: bool) -> Self {
        Reorder { nodes: p[0], supers: p[1], wraps: p[2], rows: p[3], out: p[4], head: p[5], link: p[6], place: p[7], snode: p[8], counts: p[9], before: before.then(|| p[10]) }
    }
}

/// A function of tier 2's own, made from the same program and called once per file: `build`
/// makes its body from its `params` words and gives its heads. Only the token loop is inlined;
/// the node table's passes run once, at `ACCEPT`, and would only crowd it. Checked: it calls
/// nothing, and each loop is a stated one.
///
/// `pinned`: its first parameter is `ctx`, in the pinned register (`pinned_ctx`): not an
/// argument, read where it is entered; its callers pass the rest (`call_word`).
fn outline(module: &mut JITModule, name: &str, params: usize, returns: usize, pinned: bool, build: impl FnOnce(&mut FunctionBuilder<'_>, &[Value], &mut Vec<(Block, Step)>)) -> Result<(cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
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
    crate::native_cpu_shape::check(&func, &heads, &[]).map_err(|r| Error::RefusedShape(format!("{r:?}")))?;
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
    Leaf { deferred: bool, extra: bool, end: EndAt },
    Open,
    Close { this: bool, end: EndAt },
    Wrap,
    /// `RETAG m=1`: the node the last wrap moved.
    RetagMoved { remembered: bool },
    /// `RETAG m=2`, `NODE_FIELD m=2`: the node made last at the level.
    Made { retag: bool, remembered: bool, dead: bool },
    /// `RETAG`: the innermost open node.
    Here,
    /// `MERGE m=1`.
    Fold,
    Merge { end: EndAt },
    /// `TOKEN`, taking a token: 0, or 1 past the stream's end, or `OVERFLOW` out of room.
    Token { any_mark: Option<usize> },
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

/// `LEAF`: its row (its kind, with its field above it or none; its bytes; one record), the
/// pending extras, the level's last node.
fn leaf_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, kind: Value, deferred: bool, extra: bool, end_at: EndAt, wrapping: bool) {
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
    let one = f.ins().iconst(types::I64, 1);
    let rows = (e.ctx)(f, ROWS);
    let row = NodeCol::row(f, rows, nodes);
    f.ins().istore32(flags, kind, row, AT_KIND);
    NodeCol::of(rows, AT_START).put_row(f, row, start);
    NodeCol::of(rows, AT_END).put_row(f, row, end);
    NodeCol::of(rows, AT_SIZE).put_row(f, row, one);
    if wrapping {
        let head = (e.ctx)(f, HEAD);
        slot_put(f, head, nodes, zero);
        if !deferred && !extra {
            // The last node made at the innermost open node's level.
            let n1 = f.ins().iadd_imm(nodes, 1);
            (e.set)(f, Mem::LastMade, n1);
            (e.set)(f, Mem::LastBase, nodes);
        }
    }
    let nodes = f.ins().iadd_imm(nodes, 1);
    (e.set)(f, Mem::Nodes, nodes);
}

/// `OPEN`: its row (its kind, with its field above it or none, and its start), open; with wraps,
/// its frame.
fn open_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, kind: Value, wrapping: bool) {
    let zero = f.ins().iconst(types::I64, 0);
    (e.set)(f, Mem::Pending, zero);
    let (nodes, start, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Start), (e.get)(f, Mem::Open));
    let rows = (e.ctx)(f, ROWS);
    let row = NodeCol::row(f, rows, nodes);
    f.ins().istore32(MemFlags::trusted(), kind, row, AT_KIND);
    NodeCol::of(rows, AT_START).put_row(f, row, start);
    let open_c = (e.ctx)(f, OPEN);
    slot_put(f, open_c, open, nodes);
    if wrapping {
        // Its frame, at depth + 1 (slot 0 is the root's parent's): its own base, no node made
        // yet; and its record is no base of a wrap yet.
        let slot1 = f.ins().iadd_imm(open, 1);
        let fbase = (e.ctx)(f, FRAME_BASE);
        slot_put(f, fbase, slot1, nodes);
        (e.set)(f, Mem::LastMade, zero);
        let head = (e.ctx)(f, HEAD);
        slot_put(f, head, nodes, zero);
    }
    let open = f.ins().iadd_imm(open, 1);
    let nodes = f.ins().iadd_imm(nodes, 1);
    (e.set)(f, Mem::Open, open);
    (e.set)(f, Mem::Nodes, nodes);
}

/// `CLOSE`: the innermost open node's end (`this`: the token's; else the last taken's, or
/// the one before the pending extras) and size; with wraps, the last node made at its parent's
/// level.
fn close_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, this: bool, end_at: EndAt, wrapping: bool) {
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
    let base = if wrapping {
        // Its subtree starts at its base (a wrap's is the moved node's); it is the last node
        // made at its parent's level.
        let slot1 = f.ins().iadd_imm(open, 1);
        let fbase = (e.ctx)(f, FRAME_BASE);
        let base = slot_get(f, fbase, slot1);
        let at1 = f.ins().iadd_imm(at, 1);
        (e.set)(f, Mem::LastMade, at1);
        (e.set)(f, Mem::LastBase, base);
        base
    } else {
        at
    };
    let size = f.ins().isub(nodes, base);
    let size = f.ins().isub(size, pending);
    NodeCol::of(rows, AT_SIZE).put_row(f, at_row, size);
}

/// `WRAP`: around the last node made at the innermost open node's level, its record at the
/// table's end, linked into the moved node's base's chain; open, its frame's base that base, the
/// moved node the last made at its level.
fn wrap_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, kind: Value) {
    let zero = f.ins().iconst(types::I64, 0);
    let open = (e.get)(f, Mem::Open);
    let (last, base) = ((e.get)(f, Mem::LastMade), (e.get)(f, Mem::LastBase));
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, none);
    (e.guard)(f, no_last);
    let r = f.ins().iadd_imm(last, -1);
    let (nodes, wraps) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Wraps));
    let rows = (e.ctx)(f, ROWS);
    let (kind_c, field_c, start_c) = (NodeCol::of(rows, AT_KIND), NodeCol::of(rows, AT_FIELD), NodeCol::of(rows, AT_START));
    let rr = NodeCol::row(f, rows, r);
    let (fld, st) = (field_c.get_row(f, rr), start_c.get_row(f, rr));
    let row = NodeCol::row(f, rows, nodes);
    kind_c.put_row(f, row, kind);
    field_c.put_row(f, row, fld);
    start_c.put_row(f, row, st);
    let rr = NodeCol::row(f, rows, r);
    field_c.put_row(f, rr, zero);
    let (head, link) = ((e.ctx)(f, HEAD), (e.ctx)(f, LINK));
    let h = slot_get(f, head, base);
    slot_put(f, link, nodes, h);
    let n1 = f.ins().iadd_imm(nodes, 1);
    slot_put(f, head, base, n1);
    let mark = f.ins().iconst(types::I64, IS_WRAP);
    slot_put(f, head, nodes, mark);
    let open_c = (e.ctx)(f, OPEN);
    slot_put(f, open_c, open, nodes);
    let slot1 = f.ins().iadd_imm(open, 1);
    let fbase = (e.ctx)(f, FRAME_BASE);
    slot_put(f, fbase, slot1, base);
    (e.set)(f, Mem::LastMade, last);
    (e.set)(f, Mem::LastBase, base);
    (e.set)(f, Mem::Moved, r);
    (e.set)(f, Mem::Pending, zero);
    (e.set)(f, Mem::Open, slot1);
    (e.set)(f, Mem::Nodes, n1);
    let wraps = f.ins().iadd_imm(wraps, 1);
    (e.set)(f, Mem::Wraps, wraps);
}

/// `RETAG m=2`, `NODE_FIELD m=2`: the node made last at the innermost open node's level
/// (`relabel`), its frame's last one more than its record (0: none), its kind or field `x`.
fn made_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, x: Value, retag: bool, dead: bool) {
    let open = (e.get)(f, Mem::Open);
    let last = (e.get)(f, Mem::LastMade);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
    // A branch each, not the tests joined.
    (e.guard)(f, none);
    (e.guard)(f, no_last);
    let r = f.ins().iadd_imm(last, -1);
    let rows = (e.ctx)(f, ROWS);
    NodeCol::of(rows, if retag { AT_KIND } else { AT_FIELD }).put(f, r, x);
    if dead {
        // `drop`: the node goes (a token the grammar implies), left out at `ACCEPT`; none is
        // made last at this level until the next.
        let d = (e.get)(f, Mem::Dead);
        let d = f.ins().iadd_imm(d, 1);
        (e.set)(f, Mem::Dead, d);
        let zero = f.ins().iconst(types::I64, 0);
        (e.set)(f, Mem::LastMade, zero);
    }
}

/// `RETAG`: the innermost open node's kind.
fn here_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, kind: Value) {
    let open = (e.get)(f, Mem::Open);
    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
    (e.guard)(f, none);
    let top = f.ins().iadd_imm(open, -1);
    let open_c = (e.ctx)(f, OPEN);
    let at = slot_get(f, open_c, top);
    let rows = (e.ctx)(f, ROWS);
    NodeCol::of(rows, AT_KIND).put(f, at, kind);
}

/// `MERGE m=1`: the innermost open node holds two leaves and nothing else, no extra pending:
/// the first becomes one leaf through the second's end.
fn fold_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, kind: Value, wrapping: bool) {
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
    let (kind_c, end_c, size_c) = (NodeCol::of(rows, AT_KIND), NodeCol::of(rows, AT_END), NodeCol::of(rows, AT_SIZE));
    let (ra, rb) = (NodeCol::row(f, rows, a), NodeCol::row(f, rows, b));
    let (sa, sb) = (size_c.get_row(f, ra), size_c.get_row(f, rb));
    let na = f.ins().icmp_imm(IntCC::NotEqual, sa, 1);
    let nb = f.ins().icmp_imm(IntCC::NotEqual, sb, 1);
    (e.guard)(f, na);
    (e.guard)(f, nb);
    let end = end_c.get_row(f, rb);
    kind_c.put_row(f, ra, kind);
    end_c.put_row(f, ra, end);
    (e.set)(f, Mem::Nodes, b);
    if wrapping {
        // The first is the last node made at this level now.
        (e.set)(f, Mem::LastMade, b);
        (e.set)(f, Mem::LastBase, a);
    }
}

/// `MERGE`: the grammar's leaf before the pending extras, a leaf inside the innermost open
/// node; the extras are its text: dropped.
fn merge_word(f: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, kind: Value, end_at: EndAt) {
    let zero = f.ins().iconst(types::I64, 0);
    let (nodes, pending, open) = ((e.get)(f, Mem::Nodes), (e.get)(f, Mem::Pending), (e.get)(f, Mem::Open));
    let kept = f.ins().isub(nodes, pending);
    let none = f.ins().icmp_imm(IntCC::Equal, kept, 0);
    (e.guard)(f, none);
    let leaf = f.ins().iadd_imm(kept, -1);
    let rows = (e.ctx)(f, ROWS);
    let (kind_c, end_c, size_c) = (NodeCol::of(rows, AT_KIND), NodeCol::of(rows, AT_END), NodeCol::of(rows, AT_SIZE));
    let leaf_row = NodeCol::row(f, rows, leaf);
    let size = size_c.get_row(f, leaf_row);
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
    kind_c.put_row(f, leaf_row, kind);
    end_c.put_row(f, leaf_row, end);
    (e.set)(f, Mem::Nodes, kept);
    (e.set)(f, Mem::Pending, zero);
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
}

/// `TOKEN`'s body (`WordFn::Token`), taking a token: params `ctx`, the calls open. The room for
/// the path to the next token, then the next record, or the stream's end (its own kind's code). 0, or 1 where the stream had ended
/// already, or `OVERFLOW` out of room.
fn take_token(f: &mut FunctionBuilder<'_>, p: &[Value], any_mark: Option<usize>, t: TokenShape) {
    let ctx = p[0];
    let flags = MemFlags::trusted();
    let load = |f: &mut FunctionBuilder<'_>, at: i32| f.ins().load(types::I64, flags, ctx, at);
    let pops = t.bound.pop_nodes > 0 || t.bound.pop_supers > 0;
    let open_calls = if !t.on_stack { p[1] } else if pops { calls_open_by_stack(f, ctx) } else { f.ins().iconst(types::I64, 0) };
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
fn token_fn<'a>(module: &mut JITModule, fns: &'a mut std::collections::HashMap<WordFn, (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)>, any_mark: Option<usize>, shape: TokenShape, pinned: bool) -> Result<&'a (cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function), Error> {
    let key = WordFn::Token { any_mark };
    if !fns.contains_key(&key) {
        let name = format!("tempo_word_{}", fns.len());
        let made = outline(module, &name, if shape.on_stack { 1 } else { 2 }, 1, pinned, |g, q, _| take_token(g, q, any_mark, shape))?;
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
    // Record `from` to place `to`, its row's first 16 bytes (kind to size) as two words; its
    // range starts at `first`.
    let copy = |f: &mut FunctionBuilder<'_>, from: Value, to: Value, first: Value| {
        let (a, b) = (NodeCol::row(f, r.rows, from), NodeCol::row(f, r.out, to));
        let (x, y) = (f.ins().load(types::I64, flags, a, 0), f.ins().load(types::I64, flags, a, 8));
        f.ins().store(flags, x, b, 0);
        f.ins().store(flags, y, b, 8);
        if let Some(before) = r.before {
            let size = NodeCol::of(r.rows, AT_SIZE).get(f, from);
            let e = f.ins().iadd(first, size);
            let (de, df) = (get(f, before, e), get(f, before, first));
            let gone = f.ins().isub(de, df);
            let x = f.ins().isub(size, gone);
            NodeCol::of(r.out, AT_SIZE).put(f, to, x);
        }
        put(f, r.place, from, to);
    };
    // Whether record `n` was dropped (only a program that rereads drops one).
    let dropped = |f: &mut FunctionBuilder<'_>, n: Value| -> Option<Value> {
        r.before.map(|_| {
            let k = NodeCol::of(r.rows, AT_KIND).get(f, n);
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
        let k = NodeCol::of(r.rows, AT_KIND).get(f, i);
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

/// The table's final pass, once it is in preorder: each node's parent and its extra flag, in
/// its row, from its kind and size, over `count` rows.
/// The open nodes are a stack of records in `stack` (the open stack's room, free at `ACCEPT`):
/// before record `i`, each open node whose range ends at or before `i` is popped, and the top is
/// `i`'s parent. One loop over the records and one over the pops, each step a record
/// (`Step::Node`): a record is pushed once and popped at most once. Returns 0.
fn table(f: &mut FunctionBuilder<'_>, rows: Value, count: Value, stack: Value, extras: &[u16], heads: &mut Vec<(Block, Step)>) {
    let flags = MemFlags::trusted();
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
    let size = NodeCol::of(rows, AT_SIZE).get(f, top);
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
    let row = NodeCol::row(f, rows, i);
    f.ins().istore32(flags, parent, row, AT_PARENT);
    let k = f.ins().uload16(types::I64, flags, row, AT_KIND);
    let mut extra = f.ins().iconst(types::I8, 0);
    for &e in extras {
        let is = f.ins().icmp_imm(IntCC::Equal, k, i64::from(e));
        extra = f.ins().bor(extra, is);
    }
    // With the row's last three bytes: every byte of a row is written.
    let extra = f.ins().uextend(types::I32, extra);
    f.ins().store(flags, extra, row, AT_EXTRA);
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
        Op::Span { .. } | Op::Word { test: false, .. } | Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::Settle { .. } | Op::NodeField { .. } | Op::NodeSuper { .. } | Op::Retag { .. } | Op::Wrap { .. } | Op::Merge { .. } | Op::Push { .. } | Op::Remember { .. } | Op::Flag { op: FlagOp::Set | FlagOp::Clear, .. }
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
    // The node table's passes, outlined (`outline`): made where first called, by any cluster.
    let mut reorder_fn = None;
    // The words made as functions of their own (`WordFn`), with a function a grammar rule.
    let outlined = !native.rules.is_empty() && cut.calls;
    // The calls open counted by the machine's stack (`calls_on_stack`), with `ctx` pinned.
    let on_stack = pinned_ctx && outlined && !flagged && calls_on_stack(native);
    // The callee takes its caller's token (`RETURN m=0`), as the lean parser's callee consumes
    // it and its caller peeks: where every call's place to return to is one `TOKEN`, alike
    // (the same mask, and taking a token there), a `RETURN m=0` takes it and returns as `m=1`
    // does, to the caller's switch. So every call returns one way and is tested once (`cbnz`).
    let landing_token: Option<Option<u8>> = {
        let pushes = push_targets(native);
        let mut anys = pushes.iter().map(|&p| match words.get(usize::from(p)).map(|b| b.operation) {
            Some(Op::Token { any, .. }) if phase_of[usize::from(p)] == crate::stream::TAKEN => Some(any),
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
    let mut table_fn: Option<(cranelift_module::FuncId, cranelift_codegen::ir::Signature, cranelift_codegen::ir::Function)> = None;
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
        let (fbase_c, head_c, link_c, place_c) = (0, 3, 4, 5);
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
            let (kind_c, field_c, _start_c, end_c, size_c) =
                (NodeCol::of(rows_here, AT_KIND), NodeCol::of(rows_here, AT_FIELD), NodeCol::of(rows_here, AT_START), NodeCol::of(rows_here, AT_END), NodeCol::of(rows_here, AT_SIZE));
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
                            let made = outline(&mut module, &name, 1 + args.len(), usize::from($refuses), pinned_ctx, |g, q, _| own_word(g, q, $refuses, &body))?;
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
                Op::Token { any, .. } => {
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
                        let shape = TokenShape { bound, in_code, rest_bits, end_bits, end_code: end_kind.map(i64::from), on_stack };
                        let (id, sig, _) = token_fn(&mut module, &mut word_fns, any_mark, shape, pinned_ctx)?;
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
                        dispatch(&mut f, w, &r);
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
                Op::Mark { mask, order, end, word: mark_word } => {
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
                        let rest = f.ins().band_imm(live, !((2i64 << order) - 1));
                        mset(&mut f, Mem::Live, rest);
                        f.ins().jump(fall, &[]);
                        continue;
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
                    let (code, next) = with_field(kind);
                    let k = f.ins().iconst(types::I64, code);
                    word!(WordFn::Open, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| open_word(g, e, q[0], wrapping), next);
                }
                Op::Close { this } => {
                    let end = end_at(pc);
                    word!(WordFn::Close { this, end }, &[], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, _: &[Value]| close_word(g, e, this, end, wrapping));
                }
                Op::Leaf { kind, deferred, extra } => {
                    let end = end_at(pc);
                    let (code, next) = with_field(kind);
                    let k = f.ins().iconst(types::I64, code);
                    word!(WordFn::Leaf { deferred, extra, end }, &[k], false, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| leaf_word(g, e, q[0], deferred, extra, end, wrapping), next);
                }
                Op::Settle { .. } => {
                    // A token the grammar implies: no node; the extras pending stay where they are.
                    mset(&mut f, Mem::Pending, zero);
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
                    let remembered = kind == REMEMBERED;
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::RetagMoved { remembered }, &[k], remembered, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if remembered { remembered_kind(g, e) } else { q[0] };
                        let m = (e.get)(g, Mem::Moved);
                        let rows = (e.ctx)(g, ROWS);
                        NodeCol::of(rows, AT_KIND).put(g, m, k);
                    });
                }
                Op::Wrap { kind } if wrapping => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Wrap, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| wrap_word(g, e, q[0]));
                }
                Op::NodeField { field, on: On::Inner } => {
                    // The node the node made last, a wrap, holds first: that frame's last base.
                    let open = mget(&mut f, Mem::Open);
                    let (last, base) = (mget(&mut f, Mem::LastMade), mget(&mut f, Mem::LastBase));
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    // A branch each, not the tests joined.
                    guard(&mut f, none, pc);
                    guard(&mut f, no_last, pc);
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
                    field_c.put(&mut f, base, x);
                    f.ins().jump(fall, &[]);
                }
                Op::Retag { kind, on: On::Made } | Op::NodeField { field: kind, on: On::Made } => {
                    let retag = matches!(w.operation, Op::Retag { .. });
                    let remembered = retag && kind == REMEMBERED;
                    let dead = retag && kind == DEAD;
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Made { retag, remembered, dead }, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| {
                        let k = if remembered { remembered_kind(g, e) } else { q[0] };
                        made_word(g, e, k, retag, dead)
                    });
                }
                Op::Reread { reading } if wrapping => {
                    // The node made last at the innermost open node's level and the nodes inside
                    // it: the records from its base to the pending extras, each base's chain of
                    // wraps first (outermost first, as the reorder places them). A loop over the
                    // records and one over a chain, each step a record (`Step::Node`); a kind
                    // each, by a run of compares.
                    let r = native.readings.maps.get(usize::from(reading)).ok_or(Error::UnsupportedBackend)?;
                    let open = mget(&mut f, Mem::Open);
                    let (last, base) = (mget(&mut f, Mem::LastMade), mget(&mut f, Mem::LastBase));
                    let none = f.ins().icmp_imm(IntCC::Equal, open, 0);
                    let no_last = f.ins().icmp_imm(IntCC::Equal, last, 0);
                    // A branch each, not the tests joined.
                    guard(&mut f, none, pc);
                    guard(&mut f, no_last, pc);
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
                    let nrow = NodeCol::row(&mut f, rows_here, n);
                    let k = kind_c.get_row(&mut f, nrow);
                    // Its field, unless it is the node read: the reading's, by a run of selects.
                    let fld = field_c.get_row(&mut f, nrow);
                    let mut to = fld;
                    for &(a, b) in &r.fields {
                        let is = f.ins().icmp_imm(IntCC::Equal, fld, i64::from(a));
                        let g = f.ins().iconst(types::I64, i64::from(b));
                        to = f.ins().select(is, g, to);
                    }
                    let is_root = f.ins().icmp(IntCC::Equal, n, root);
                    let to = f.ins().select(is_root, fld, to);
                    field_c.put_row(&mut f, nrow, to);
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
                                    size_c.get_row(&mut f, nrow)
                                };
                                let past = f.ins().iadd(i, size);
                                f.ins().jump(scan, &[BlockArg::from(past)]);
                            }
                            GONE => {
                                // The node read going: none made last at this level until the next.
                                let kept = f.ins().select(is_root, zero, last);
                                mset(&mut f, Mem::LastMade, kept);
                                let d = f.ins().iconst(types::I64, i64::from(DEAD));
                                kind_c.put_row(&mut f, nrow, d);
                                let dead = mget(&mut f, Mem::Dead);
                                let dead = f.ins().iadd_imm(dead, 1);
                                mset(&mut f, Mem::Dead, dead);
                                f.ins().jump(next, &[]);
                            }
                            m => {
                                let m = kind_of(&mut f, m, pc);
                                kind_c.put_row(&mut f, nrow, m);
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
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Here, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| here_word(g, e, q[0]));
                }
                Op::Merge { kind, fold: true } => {
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Fold, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| fold_word(g, e, q[0], wrapping));
                }
                Op::Merge { kind, fold: false } => {
                    let end = end_at(pc);
                    let k = f.ins().iconst(types::I64, i64::from(kind));
                    word!(WordFn::Merge { end }, &[k], true, |g: &mut FunctionBuilder<'_>, e: &WordEnv<'_>, q: &[Value]| merge_word(g, e, q[0], end));
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
                Op::Push { target } if cut.calls => {
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
                    if let (Some(any), false) = (landing_token, again) {
                        let any_mark: Option<usize> = match any {
                            Some(k) => Some(native.block_programs.first().and_then(|b| b.masks.get(usize::from(k)).copied()).ok_or(Error::UnsupportedBackend)?),
                            None => None,
                        };
                        let shape = TokenShape { bound, in_code, rest_bits, end_bits, end_code: end_kind.map(i64::from), on_stack };
                        let (id, sig, _) = token_fn(&mut module, &mut word_fns, any_mark, shape, pinned_ctx)?;
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
                    size_c.put_row(&mut f, root_row, size);
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
                        // A program that drops records: it rereads, or it drops a node (`drop`).
                        let rereads = words.iter().any(|b| matches!(b.operation, Op::Reread { .. } | Op::Retag { kind: DEAD, on: On::Made }));
                        let r = Reorder { nodes, supers, wraps, rows: rows_here, out: wc(6), head: wc(head_c), link: wc(link_c), place: wc(place_c), snode: snode_c, before: rereads.then(|| wc(fbase_c)), counts: counts_c };
                        let args = r.values();
                        if reorder_fn.is_none() {
                            let before = r.before.is_some();
                            reorder_fn = Some(outline(&mut module, "tempo_stream_reorder", args.len(), 0, false, |f, p, heads| reorder(f, Reorder::of(p, before), heads))?);
                        }
                        let (id, sig, _) = reorder_fn.as_ref().expect("made");
                        call_own(&mut f, *id, sig, &args);
                        // Without a wrap made, the table is in preorder as made.
                        let placed = f.ins().load(types::I64, flags, counts_c, 0);
                        let table_rows = f.ins().select(wraps, r.out, r.rows);
                        f.ins().jump(fin, &[BlockArg::from(table_rows), BlockArg::from(placed)]);
                    } else {
                        f.ins().jump(fin, &[BlockArg::from(rows_here), BlockArg::from(nodes)]);
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
            let extras = &native.readings.extras;
            if table_fn.is_none() {
                table_fn = Some(outline(&mut module, "tempo_stream_table", 3, 1, false, |f, q, heads| table(f, q[0], q[1], q[2], extras, heads))?);
            }
            let made = table_fn.as_ref().expect("made");
            let open_c = if cut.reload {
                let b = if clustered { pinned(&mut f) } else { ctx };
                f.ins().load(types::I64, MemFlags::trusted(), b, OPEN)
            } else {
                open_c
            };
            let r = call_own(&mut f, made.0, &made.1, &[p[0], p[1], open_c]);
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
    let own: Vec<_> = reorder_fn.into_iter().chain(table_fn).chain(word_fns).collect();
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
    Ok(CompiledStream { _module: module, entry, frame_shift, bounds: bounds(native), starts, lasts, marks, wraps, context: two_slots, clif, disassembly, code_bytes, heads, own_calls, clusters: cut.count as usize, crossings, word_insts, word_bytes, word_ranges })
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
    /// The node table, a row a node.
    pub rows: Vec<NodeRow>,
    pub super_node: Vec<u32>,
    pub super_kind: Vec<u32>,
    open: Vec<u32>,
    returns: Vec<u32>,
    /// The node count, the supertype count, the wraps (then the table is in `out`).
    pub counts: [u64; 3],
    /// A program with `WRAP`'s: frames, chains, places, and the table in preorder. Empty
    /// otherwise.
    frames: [Vec<u32>; 3],
    chains: [Vec<u32>; 3],
    out: Vec<NodeRow>,
}

impl CompiledStream {
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
        let poison = std::env::var_os("RERAIL_TAPE_POISON").is_some();
        let room = |n: usize| -> Vec<u32> { if poison { vec![u32::MAX; n] } else { Vec::with_capacity(n) } };
        let rows = |n: usize| -> Vec<NodeRow> {
            if poison {
                // SAFETY: a row is plain bytes; all ones is one.
                vec![unsafe { std::mem::transmute::<[u8; ROW as usize], NodeRow>([u8::MAX; ROW as usize]) }; n]
            } else {
                Vec::with_capacity(n)
            }
        };
        let wraps = |n: usize| if self.wraps { n } else { 0 };
        let mut t = Tape {
            rows: rows(nodes),
            super_node: room(supers),
            super_kind: room(supers),
            open: room(nodes),
            returns: room(if self.context { 2 * pushes } else { pushes }),
            counts: [0; 3],
            frames: std::array::from_fn(|_| room(wraps(nodes + 1))),
            chains: std::array::from_fn(|_| room(wraps(nodes))),
            out: rows(wraps(nodes)),
        };
        t.clear();
        t
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
            // SAFETY: the code wrote the records and supertypes it counted.
            unsafe { tape.written(r == 0) };
            return r;
        }
        ROOM_RETRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *tape = self.full_tape(tier1.nstarts);
        // SAFETY: as above, on the bound's tape.
        let r = unsafe { self.call_once(bytes, len, tier1, tape) };
        unsafe { tape.written(r == 0) };
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
            out_rows: tape.out.as_mut_ptr(),
            output_stride: tier1.output_stride as u64,
            totals_stride: tier1.totals_stride as u64,
            totals_mask: tier1.totals_mask,
            room_nodes: tape.rows.capacity() as u64,
            room_supers: tape.super_node.capacity() as u64,
            room_pushes: (tape.returns.capacity() / if self.context { 2 } else { 1 }) as u64,
            stack_base: 0,
            stack_limit: 0,
            frame_shift: self.frame_shift,
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

/// The node table as tier 2 wrote it, in preorder: a row a node (`NodeRow`), the tape's own
/// buffer (`Tape::columns`), not copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Columns {
    pub rows: Vec<NodeRow>,
}

impl Columns {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl Tape {
    /// The columns' lengths: none, before a call (the code writes through their room).
    fn clear(&mut self) {
        for v in [&mut self.super_node, &mut self.super_kind, &mut self.open, &mut self.returns] {
            v.clear();
        }
        for v in self.frames.iter_mut().chain(&mut self.chains) {
            v.clear();
        }
        self.rows.clear();
        self.out.clear();
    }

    /// The columns' lengths after a call: where it accepted, the records and supertypes it
    /// counted (`counts`), the table's columns where it wrote them; else none.
    ///
    /// # Safety
    /// The call wrote, where it accepted, each record's columns, its parent and extra flag, and
    /// (with a wrap) the table in preorder, `counts[0]` of them, and `counts[1]` supertypes.
    unsafe fn written(&mut self, accepted: bool) {
        self.clear();
        if !accepted {
            return;
        }
        let (n, s) = (self.counts[0] as usize, self.counts[1] as usize);
        let wrapped = self.counts[2] > 0;
        // SAFETY: the caller's contract; each is within its room (the code checks it).
        unsafe {
            for v in [&mut self.super_node, &mut self.super_kind] {
                v.set_len(s);
            }
            if wrapped {
                self.out.set_len(n);
            } else {
                self.rows.set_len(n);
            }
        }
    }

    /// The node table's rows, in preorder.
    pub fn table(&self) -> &[NodeRow] {
        let n = self.counts[0] as usize;
        let rows = if self.counts[2] > 0 { &self.out } else { &self.rows };
        &rows[..n.min(rows.len())]
    }

    /// The node table: the tape's own rows, cut to the records written.
    pub fn columns(mut self) -> Columns {
        let n = self.counts[0] as usize;
        let mut rows = if self.counts[2] > 0 { std::mem::take(&mut self.out) } else { std::mem::take(&mut self.rows) };
        rows.truncate(n);
        Columns { rows }
    }

    /// The node table as `stream::Node`s.
    pub fn nodes(&self) -> Vec<Node> {
        let mut out: Vec<Node> = self.table().iter().map(|r| Node { kind: r.kind, field: r.field, supers: Vec::new(), start: r.start, end: r.end, size: r.size }).collect();
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
