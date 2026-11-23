//! What tier 2's stream program is, read from its words (`native_ir::Structure`), apart from any lowering of it: the
//! words a word goes on to (`successors`, `rule_successors`, `past_gotos`, `callee`, `push_targets`), the phases a
//! token's taking is in (`phases`), the bounds a run's buffers need (`bounds`), the calls on the machine's stack
//! (`calls_on_stack`), the records tier 2 makes itself (`fetch_of`) and the regions it splits (`split_parts`), the
//! runs words share (`shared_runs`), and the state a run keeps (`Mem`), which the JIT
//! (`native_cpu_stream`) reads. Nothing here emits code.

#![allow(clippy::too_many_arguments)]

use crate::Error;
use crate::byte_needs::{Seq, sequential};
use crate::native_ir::{FlagOp, Structure, StructuralOperation as Op};
use crate::stream::{REFUSE, WORD_KIND};
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

/// The most nodes a token's path may write for a run's tape to be the bound's from the start
/// (`unchecked`): the lean tape holds 1.5 a token, and the bound's capacity is reserved, not
/// written, so pages a run does not reach are never touched.
pub const UNCHECKED_NODES: u64 = 4;

impl Bounds {
    /// No room check a token: the run's tape is the bound's (`full_tape`) from the start, as
    /// simdjson sizes its tape from the input once. Only where the bound holds for every run (no
    /// returns chain) and is small (`UNCHECKED_NODES`), with `marks` the program's marks.
    pub fn unchecked(&self, marks: u64) -> bool {
        !self.chains && self.pop_nodes == 0 && self.pop_supers == 0 && self.nodes * (1 + marks) <= UNCHECKED_NODES
    }
}

/// The marks `MARK` tests (`CompiledStream::marks`).
pub fn marks_of(words: &[crate::native_ir::StructuralBlock]) -> u64 {
    words.iter().filter_map(|b| if let Op::Mark { order, .. } = b.operation { Some(u64::from(order) + 1) } else { None }).max().unwrap_or(0)
}

/// Each word's phases (`stream::phases`), over the lowered words: `TAKEN` after a `SPAN`,
/// `UNTAKEN` after a `TOKEN`; a return lands on its caller's `TOKEN` with the token taken
/// (`RETURN m=0`) or past it (`m=1`). The verifier checked them on the program.
pub fn phases(native: &Structure, pushes: &[u16]) -> Vec<u8> {
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

/// Which tokens' words the words from `from` on read, before the next `TOKEN` (its code's word
/// bits): `None`, every token's (`WORD`, a mark, a keyword's leaf or `REMEMBER`, `WORD_KIND`);
/// else the kinds whose words a switch names (`word_kinds`: a word's kind is its first
/// byte's). A `RETURN` goes on at every call's place. A token of another kind's word is dead,
/// its code its kind alone, as a compiler drops a value no use reads.
pub fn words_read(native: &Structure, pushes: &[u16], word_kinds: &[u8], from: usize) -> Option<Vec<u8>> {
    let n = native.structural.len();
    if from >= n {
        return None;
    }
    let mut kinds: Vec<u8> = Vec::new();
    let mut seen = vec![false; n];
    let mut work = vec![from];
    while let Some(q) = work.pop() {
        if q >= n || std::mem::replace(&mut seen[q], true) {
            continue;
        }
        match native.structural[q].operation {
            Op::Token { .. } => continue,
            Op::Switch { table } => {
                for &(w, _) in &native.switches[usize::from(table)].words {
                    kinds.push(*word_kinds.get(usize::from(w))?);
                }
            }
            Op::Word { .. } | Op::Mark { .. } | Op::Leaf { kind: WORD_KIND, .. } | Op::Remember { kind: WORD_KIND } => return None,
            _ => {}
        }
        work.extend(successors(native, pushes, q));
    }
    kinds.sort_unstable();
    kinds.dedup();
    Some(kinds)
}

/// The `TOKEN`s where a part of a region read as `parse_string` reads (`Fetch::seq_bytes`, its
/// openers' bytes) may be pending: only they hold its scan (`split_next`), as simdjson's
/// `parse_string` loop is only where a string is read. Over (place, a part pending), from the
/// first word with none: a switch's edge of an opener's byte makes one pending (its record is
/// the opener alone); under one, the token is a part or the closer, and the closer's kind (its
/// bytes') ends it, every other kind keeps it. A switch with an offset keeps what it had.
pub fn parts_pending(native: &Structure, pushes: &[u16], ft: &Fetch) -> Vec<bool> {
    let seen = parts_pending_seen(native, pushes, ft);
    (0..native.structural.len()).map(|pc| matches!(native.structural[pc].operation, Op::Token { .. }) && seen[pc][1]).collect()
}

/// As `parts_pending`, each word's two states: reached with no part pending, and with one.
pub fn parts_pending_seen(native: &Structure, pushes: &[u16], ft: &Fetch) -> Vec<[bool; 2]> {
    let n = native.structural.len();
    let opener = |b: usize| ft.seq_bytes[b / 64] >> (b % 64) & 1 == 1;
    let closing: Vec<bool> = (0..256).map(|b| (0..256).any(|x| opener(x) && ft.kinds[x] == ft.kinds[b])).collect();
    let mut seen = vec![[false; 2]; n];
    let mut work = vec![(0usize, false)];
    while let Some((pc, p)) = work.pop() {
        if pc >= n || std::mem::replace(&mut seen[pc][usize::from(p)], true) {
            continue;
        }
        match native.structural[pc].operation {
            Op::Token { .. } => work.push((pc + 1, p)),
            Op::Switch { table } => {
                let sw = &native.switches[usize::from(table)];
                if sw.offset != 0 {
                    work.extend(successors(native, pushes, pc).into_iter().map(|t| (t, p)));
                    continue;
                }
                for (b, &t) in sw.targets.iter().enumerate() {
                    if t != REFUSE {
                        work.push((usize::from(t), if p { !closing[b] } else { opener(b) }));
                    }
                }
                work.extend(sw.words.iter().filter(|&&(_, t)| t != REFUSE).map(|&(_, t)| (usize::from(t), p)));
                if sw.end != REFUSE {
                    work.push((usize::from(sw.end), false));
                }
            }
            _ => work.extend(successors(native, pushes, pc).into_iter().map(|t| (t, p))),
        }
    }
    seen
}

/// Each word's successors, as `stream::validate` reads them, `TOKEN`'s cut.
pub fn successors(native: &Structure, pushes: &[u16], pc: usize) -> Vec<usize> {
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

/// With the calls open counted by the machine's stack (`calls_on_stack`): the most of it a run
/// takes, about `DEPTH` calls of `.rerail`'s largest frames. Past it, a rule function refuses the
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
pub enum Mem {
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
    /// A region tier 2 splits (`Split::Tier2`, decision 0323): the byte its next part starts at
    /// (0: none), and its closer's last byte.
    PartAt,
    PartEnd,
    /// A split region's closer's first byte, and its reader (`SplitParts::readers`).
    PartClose,
    PartRd,
    /// An extra a mark lost (`SETTLE m=2`): its kind and field, plus one (0: none), made again
    /// by the next `TOKEN` that takes a token, of no width at the token taken.
    Lost,
    /// The strings whose template's code is being read (decision 0600): their records left open
    /// at a template's `$`, their parts going on at the next opener of a reader that resumes
    /// (`SplitParts::resumers`).
    Strings,
}

impl Mem {
    pub const COUNT: u32 = 32;
    /// Every field, in order: what a statement's mark keeps (decision 0650).
    pub const ALL: [Mem; Mem::COUNT as usize] = [
        Mem::Supers, Mem::Pending, Mem::Keep, Mem::Moved, Mem::Sel, Mem::Shift, Mem::LastMade, Mem::Word, Mem::Flags, Mem::Next, Mem::Taken, Mem::Lw, Mem::Start, Mem::End, Mem::Limit, Mem::LastEnd,
        Mem::Nodes, Mem::Open, Mem::Ret, Mem::Need, Mem::Live, Mem::Depth, Mem::BlockBase, Mem::Coded, Mem::Codes, Mem::Wide, Mem::PartAt, Mem::PartEnd, Mem::PartClose, Mem::PartRd, Mem::Lost, Mem::Strings,
    ];
}

const _: () = {
    let mut k = 0;
    while k < Mem::COUNT as usize {
        assert!(Mem::ALL[k] as usize == k);
        k += 1;
    }
};

/// The fields a rule's function holds in registers (`Cut::held`) and passes to its calls, given
/// back with their results (`Cut::pass`): none. The whole state stays in memory, read and
/// written where used, as the lean parser's `&mut self` fields are: it reloads `self.i` and the
/// token where it tests them, and keeps nothing live across a rule. Held and passed, the
/// token's code, start, end and end flag were live through every block and spilled (15,133
/// stack loads over `.rerail`'s tier 2, against 161 in the lean parser).
pub const HELD_IN_RULES: [u32; 0] = [];

/// The fields tier 2 holds as SSA values where it is one function (`Cut::one`): those every
/// token reads and writes (the next token's index, its start, limit and code, the end of the
/// token before, the nodes made and open, the tape block's base every leaf's start is coded
/// from, a split region's cursor), as simdjson's stage 2
/// keeps `next_structural` and `src` in registers. The loop's cold calls keep the caller's
/// general registers (`preserve_most`), and the values from far away are made again where used
/// (`remat_far`), so these stay in registers. The rest stays in the frame, read where used.
pub const HELD_IN_ONE: [u32; 9] = [Mem::Next as u32, Mem::Start as u32, Mem::Limit as u32, Mem::Taken as u32, Mem::LastEnd as u32, Mem::Nodes as u32, Mem::Open as u32, Mem::BlockBase as u32, Mem::PartAt as u32];

/// The token's last bytes at which a token's check can hold (decision 0428), where its test is
/// at its last byte (`token_ops_past`: `Last` read `delay` back from the test, in a conjunct).
/// A byte set read there in a conjunct of every way to 1 bounds it, as simdjson's stage 2
/// switches on an escape's byte before its `\u` work. `None`: no bound (any byte, or the test
/// not at the last byte).
pub fn token_gate(c: &crate::program::LocalCheck) -> Option<[u64; 4]> {
    use crate::program::LocalOp as L;
    const ALL: [u64; 4] = [!0; 4];
    // Op `k` read `t` bytes back from the test: the bytes at the last that it needs, and
    // whether it needs the last byte there.
    fn need(ops: &[L], k: usize, t: u32, d: u32, steps: &mut u32) -> ([u64; 4], bool) {
        if *steps == 0 || t > 64 {
            return (ALL, false);
        }
        *steps -= 1;
        match ops[k] {
            L::Bytes(s) if t == d => (s, false),
            L::Last => (ALL, t == d),
            L::And(a, b) => {
                let ((x, p), (y, q)) = (need(ops, usize::from(a), t, d, steps), need(ops, usize::from(b), t, d, steps));
                ([x[0] & y[0], x[1] & y[1], x[2] & y[2], x[3] & y[3]], p || q)
            }
            L::Or(a, b) => {
                let ((x, p), (y, q)) = (need(ops, usize::from(a), t, d, steps), need(ops, usize::from(b), t, d, steps));
                ([x[0] | y[0], x[1] | y[1], x[2] | y[2], x[3] | y[3]], p && q)
            }
            L::AndNot(a, _) => need(ops, usize::from(a), t, d, steps),
            L::Prev(a, sh) => need(ops, usize::from(a), t + u32::from(sh), d, steps),
            _ => (ALL, false),
        }
    }
    let (set, at_last) = need(&c.ops, c.ops.len() - 1, 0, u32::from(c.delay), &mut 4096);
    (at_last && set != ALL).then_some(set)
}

/// A reader whose regions tier 2 splits: its opener and closer (outputs), its parts (each its start, its
/// bytes or its last's output, and its own kind; the shortest first), its content's own kind,
/// and its closers' own kinds by their first byte.
pub struct SplitReader {
    pub opener: u16,
    pub closer: u16,
    pub parts: Vec<(u16, u8, Option<u16>, Option<u8>)>,
    pub content: Option<u8>,
    pub closers: Vec<(u8, u8)>,
    /// Its opener's token has the reader's tag (`Reader::retag`): the record's code, no word.
    pub tagged: bool,
    /// Its closer at a template (`ReadArm::template`, decision 0600), and its closer's last
    /// (`ReadArm::last`, else its offset), where it has one: its closer piece ends there, not at
    /// the record's last, which is the string's.
    pub template: Option<u16>,
    pub last: Option<u16>,
    pub off: i8,
    /// Its regions resume a string's (`Reader::resumes`): entered at the template's `}`, with
    /// no record, its opener's code its tag's (`code`).
    pub resumes: bool,
    pub code: Option<u32>,
}

/// A region read in parts that tier 2's `TOKEN` splits (`Split::Tier2`, decision 0323): its
/// readers, and the record pass's tables for the parts' codes (`native_cpu_block::plain_code`),
/// which the code reads by address.
pub struct SplitParts {
    /// Each reader whose regions tier 2 splits, as the lean parser's string arm reads its
    /// string's pieces (decision 0509): its pieces need no marks.
    pub readers: Vec<SplitReader>,
    pub tables: crate::record_tables::RecordTables,
    /// Where the region reads as simdjson's `parse_string` (`sequential`): its closer's byte
    /// test and its part's (instructions of tier 1's block, read from the bytes in tier 2).
    pub seq: Option<Seq>,
}

/// The region tier 2 splits (`SplitParts`), where a reader says so; refused where its parts'
/// codes need what `plain_code` does not give (marks, retags, words past sixteen bytes) or the
/// program has more than one such reader.
pub fn split_parts(native: &Structure) -> Result<Option<Box<SplitParts>>, Error> {
    let Some(block) = native.block_programs.first() else { return Ok(None) };
    let Some(r) = &block.records else { return Ok(None) };
    let late: Vec<&crate::records::Reader> = r.readers.iter().filter(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2).collect();
    let Some(&rd) = late.first() else { return Ok(None) };
    // A piece's code is its kind and word: no mark holds in the region (`Reader::quiet`), or
    // the program has none.
    let marked = !r.marks.is_empty() || r.retags.iter().any(|t| !t.part) || r.gaps != 0;
    if late.iter().any(|rd| rd.arms.len() != 1 || marked && !rd.quiet) {
        return Err(Error::UnsupportedFeature("a region tier 2 splits, of two regions or with marks"));
    }
    let readers: Vec<SplitReader> = late
        .iter()
        .map(|rd| {
            let a = &rd.arms[0];
            let mut parts: Vec<(u16, u8, Option<u16>, Option<u8>)> = rd.parts.iter().map(|p| (p.start, p.bytes, p.last, p.kind)).collect();
            parts.sort_by_key(|&(o, n, _, _)| (n, o));
            let code = a.retag.or(rd.retag).map(|(kind, marks)| u32::from(kind) | marks << crate::records::MARK_SHIFT);
            SplitReader { opener: a.opener, closer: a.closer, parts, content: rd.content, closers: rd.closers.clone(), tagged: a.retag.or(rd.retag).is_some(), template: a.template, last: a.last, off: a.off, resumes: rd.resumes, code }
        })
        .collect();
    // simdjson's `parse_string`: one reader, its parts of bytes, no kinds of their own.
    let simple = late.len() == 1 && rd.parts.iter().all(|p| p.last.is_none() && p.kind.is_none()) && rd.arms[0].last.is_none() && rd.content.is_none() && rd.closers.is_empty() && !marked;
    let tables = crate::record_tables::RecordTables::new(r, block)?;
    if tables.rest == crate::record_tables::WordRest::Tails {
        return Err(Error::UnsupportedFeature("a region tier 2 splits, with words past sixteen bytes"));
    }
    let mut parts: Vec<(u16, u8)> = rd.parts.iter().map(|p| (p.start, p.bytes)).collect();
    parts.sort_by_key(|&(o, n)| (n, o));
    let a = &rd.arms[0];
    // Sequential only where tier 2 makes the records: the region's record is then its opener.
    let mut seq = if r.fetched_in_tier2() && simple { sequential(block, a, &parts) } else { None };
    if let Some(sq) = seq.as_mut() {
        let set = crate::byte_needs::exact_at(block, sq.e).map(|n| n.set).unwrap_or_default();
        let ones: Vec<u8> = (0..=255u8).filter(|&b| set[usize::from(b) / 64] >> (b % 64) & 1 == 1).collect();
        if let (&[e0], Ok(wt)) = (&ones[..], crate::records::WordTable::new(&r.words)) {
            let mut t = Box::new([0u32; 256]);
            for (b, c) in t.iter_mut().enumerate() {
                *c = crate::records::pass(r, &wt, &[e0, b as u8], &[0], &[1], &[]).0.first().copied().unwrap_or(0);
            }
            sq.codes2 = Some(t);
        }
    }
    Ok(Some(Box::new(SplitParts { readers, tables, seq })))
}

/// Each token's record made by `TOKEN` from its start (`Records::fetched_in_tier2`, page 40), as
/// simdjson's stage 2 takes a structural index: its first byte's end rule (`ends`, a byte each,
/// its address read by the code), the rules, the readers, and the record pass's tables for its
/// code (`native_cpu_block::plain_code`).
pub struct Fetch {
    pub ends: Box<[u8; 256]>,
    pub rules: Vec<crate::records::EndRule>,
    pub readers: Vec<crate::records::Reader>,
    pub tables: crate::record_tables::RecordTables,
    /// Each pair table (`EndRule::Next`): its rows by first byte (256 bytes), then its sets by
    /// row (4 words each, row 0 none), as `PairTable::holds` reads them.
    pub nexts: Vec<(Box<[u8; 256]>, Vec<[u64; 4]>)>,
    /// By first byte: its kind, whether a word begins with it, and the code of the token of that
    /// byte alone (`records::pass`'s), for a `SWITCH` edge's record (page 40).
    pub kinds: [u8; 256],
    pub worded: [bool; 256],
    pub codes1: Box<[u32; 256]>,
    /// The words (`Records::words`), each with its code as a token of its bytes alone.
    pub words: Vec<(Vec<u8>, u32)>,
    /// By rule: where it is a run (`EndRule::Run`) whose output is a byte class (its bit at a
    /// byte the byte's being in a set), that set as a table of 256 bytes (1: in it), as
    /// simdjson's `structural_or_whitespace` table.
    pub run_class: Vec<Option<Box<[u8; 256]>>>,
    /// By reader: whether it reads as simdjson's `parse_string` (`sequential`): its token's
    /// record is its opener alone, its parts split from there.
    pub seq: Vec<bool>,
    /// The first bytes whose rule is a sequential reader's, a bit each.
    pub seq_bytes: [u64; 4],
}

/// The records tier 2 makes (`Fetch`), where the program says so; refused where a token's code
/// or last needs what `plain_code` and the rules below do not give (marks, retags, gap marks,
/// token tests, pair tables, a reader of more than one region, words past sixteen bytes).
pub fn fetch_of(native: &Structure) -> Result<Option<Box<Fetch>>, Error> {
    let Some(block) = native.block_programs.first() else { return Ok(None) };
    let Some(r) = block.records.as_ref().filter(|r| r.fetched_in_tier2()) else { return Ok(None) };
    if !r.marks.is_empty() || !r.retags.is_empty() || r.gaps != 0 || !r.checks.is_empty() || r.readers.iter().any(|rd| rd.arms.len() != 1) {
        return Err(Error::UnsupportedFeature("records made in tier 2, with marks, retags, token tests or a reader of two regions"));
    }
    let tables = crate::record_tables::RecordTables::new(r, block)?;
    if tables.rest == crate::record_tables::WordRest::Tails {
        return Err(Error::UnsupportedFeature("records made in tier 2, with words past sixteen bytes"));
    }
    let mut ends = Box::new([crate::records::NO_RULE; 256]);
    for (b, &k) in r.ends.iter().enumerate().take(256) {
        ends[b] = k;
    }
    let nexts = r
        .nexts
        .iter()
        .map(|t| {
            let mut row = Box::new([0u8; 256]);
            for (b, &x) in t.row.iter().enumerate().take(256) {
                row[b] = x;
            }
            let most = row.iter().copied().max().unwrap_or(0) as usize;
            let sets = (0..=most).map(|k| t.sets.get(k).copied().unwrap_or([0; 4])).collect();
            (row, sets)
        })
        .collect();
    let mut kinds = [0u8; 256];
    let mut worded = [false; 256];
    let mut codes1 = Box::new([0u32; 256]);
    let wt = crate::records::WordTable::new(&r.words).map_err(|_| Error::UnsupportedFeature("records made in tier 2: the words fit no table"))?;
    for b in 0..256usize {
        kinds[b] = r.kinds.get(b).copied().unwrap_or(0);
        worded[b] = r.words.iter().any(|w| w.first() == Some(&(b as u8)));
        codes1[b] = crate::records::pass(r, &wt, &[b as u8], &[0], &[0], &[]).0.first().copied().unwrap_or(0);
    }
    let words = r.words.iter().map(|w| (w.clone(), crate::records::pass(r, &wt, w, &[0], &[w.len().saturating_sub(1) as u32], &[]).0.first().copied().unwrap_or(0))).collect();
    let run_class = r
        .rules
        .iter()
        .map(|rule| match *rule {
            crate::records::EndRule::Run(o) => crate::byte_needs::exact(block, o).filter(|n| n.lo == 0 && n.hi == 0).map(|n| {
                let mut t = Box::new([0u8; 256]);
                for (b, x) in t.iter_mut().enumerate() {
                    *x = u8::from(n.set[b / 64] >> (b % 64) & 1 == 1);
                }
                t
            }),
            _ => None,
        })
        .collect();
    let seq: Vec<bool> = r
        .readers
        .iter()
        .map(|rd| {
            let mut parts: Vec<(u16, u8)> = rd.parts.iter().map(|p| (p.start, p.bytes)).collect();
            parts.sort_by_key(|&(o, n)| (n, o));
            rd.split == crate::records::Split::Tier2 && rd.arms.len() == 1 && sequential(block, &rd.arms[0], &parts).is_some()
        })
        .collect();
    let mut seq_bytes = [0u64; 4];
    for (x, &k) in ends.iter().enumerate() {
        if matches!(r.rules.get(usize::from(k)), Some(crate::records::EndRule::Read(i)) if seq[usize::from(*i)]) {
            seq_bytes[x / 64] |= 1 << (x % 64);
        }
    }
    Ok(Some(Box::new(Fetch { ends, rules: r.rules.clone(), readers: r.readers.clone(), tables, nexts, kinds, worded, codes1, words, run_class, seq, seq_bytes })))
}

/// Whether rule `rule` ends every token at its first byte: a pair table with no pairs
/// (`pair empty`).
pub fn one_byte(ft: &Fetch, rule: &crate::records::EndRule) -> bool {
    matches!(*rule, crate::records::EndRule::Next(i) if ft.nexts[usize::from(i)].1.iter().all(|s| s.iter().all(|&w| w == 0)))
}

/// The supertypes a program's nodes are put in (`NODE_SUPER`'s kinds), ascending: each a bit of
/// a code's key (`native_cpu_tape::commit`). Past `MAX_CLASSES`, refused.
pub fn supertype_classes(native: &Structure) -> Result<Vec<u16>, Error> {
    let mut c: Vec<u16> = native.structural.iter().filter_map(|b| if let Op::NodeSuper { kind } = b.operation { Some(kind) } else { None }).collect();
    c.sort_unstable();
    c.dedup();
    if c.len() > MAX_CLASSES {
        return Err(Error::UnsupportedBackend);
    }
    Ok(c)
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
pub fn plain(op: &Op) -> bool {
    matches!(
        op,
        Op::Span { .. } | Op::Word { test: false, .. } | Op::Open { .. } | Op::Close { .. } | Op::Leaf { .. } | Op::Settle { .. } | Op::NodeField { .. } | Op::NodeSuper { .. } | Op::Retag { .. } | Op::Wrap { .. } | Op::Merge { .. } | Op::Push { .. } | Op::Remember { .. } | Op::Flag { op: FlagOp::Set | FlagOp::Clear, .. } | Op::Count { .. } | Op::Check { .. } | Op::SettleBefore { .. }
    )
}

/// Arms that differ only in where they go on are one copy of code. A run is the plain words
/// (`plain`) before a `GOTO`, none of them but the first entered from elsewhere; runs with the
/// same words that go on to a `TOKEN` are one body, lowered at its first place. Each place enters the body with the
/// index of its `GOTO`'s target, and the body's end jumps by it (a jump table): an indirect
/// jump an arm, as simdjson's stage 2 takes one a token, for code the size of the distinct
/// arms instead of every arm's.
/// `alone_to`: the places a run going on to stays a body of its own, its `GOTO` a direct jump
/// (a string loop's head, `parts_pending`: the loop's turn a part, as `parse_string`'s).
/// `keys`: a direct format 16 run's key bits by place (`Table::direct`), which a word's body takes from its place: runs
/// share only where theirs are the same too.
pub fn shared_runs(native: &Structure, pushes: &[u16], alone_to: Option<&[bool]>, keys: Option<&[(u32, u32)]>) -> SharedRuns {
    let words = &native.structural;
    let n = words.len();
    // Entered other than by falling from a plain word before it.
    let mut entered = vec![false; n + 1];
    entered[0] = true;
    for (pc, word) in words.iter().enumerate() {
        let mut next = successors(native, pushes, pc);
        if matches!(word.operation, Op::Token { .. }) {
            next.push(pc + 1);
        }
        for t in next {
            if !(t == pc + 1 && plain(&word.operation)) {
                entered[t] = true;
            }
        }
    }
    // Each run: its first place, its `GOTO`, the target, and its words.
    let mut occ: Vec<(usize, usize, u16, Vec<Op>, Option<Vec<(u32, u32)>>)> = Vec::new();
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
            occ.push((s, g, target, words[s..g].iter().map(|w| w.operation).collect(), keys.map(|k| k[s..g].to_vec())));
        }
    }
    // Runs alone: each its own body. Sharing makes a body's jump reach every place's target on
    // the graph (not at run time: a place's index picks its own), and the shape check reads the
    // graph: a cycle that passes no `TOKEN` there is refused. So runs share, and where a cycle
    // appears that a shared jump closes, the run whose jump goes where its own does not is
    // alone; again, until none is left.
    let chains = bounds(native).chains;
    let mut alone: Vec<bool> = occ.iter().map(|o| alone_to.is_some_and(|a| a.get(usize::from(o.2)).copied().unwrap_or(false))).collect();
    let goto_of: std::collections::HashMap<usize, usize> = occ.iter().enumerate().map(|(i, o)| (o.1, i)).collect();
    loop {
        let mut keys: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut body_of = vec![0usize; occ.len()];
        let mut targets: Vec<Vec<u16>> = Vec::new();
        for (i, (_, _, t, ops, bits)) in occ.iter().enumerate() {
            let key = if alone[i] { format!("{ops:?} {bits:?} to {t}") } else { format!("{ops:?} {bits:?}") };
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
            for (i, (s, g, t, _, _)) in occ.iter().enumerate() {
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

/// `lang`'s numbers by a program's keys, `u32::MAX` where the table has none: each entry of its table that a key names
/// (no alias, a kind and field within a key's bits, its flags the extra's where its kind is one of `extras`, its
/// classes among `classes`, the program's supertype classes), at the key (`kind | field << KIND_BITS | class bits <<
/// 16`). The JIT's run reads them at run time (`Ctx::numbers`).
pub fn numbers_by_key(lang: &rerail_tape_format::f16::Lang, classes: &[u16], extras: &[u16]) -> Vec<u32> {
    use rerail_tape_format::{CODE_EXTRA, KIND_BITS};
    let mut by = vec![u32::MAX; id_slots(classes.len())];
    for e in lang.table().entries() {
        let Some(n) = lang.number(e) else { continue };
        let extra = if extras.contains(&e.kind) { CODE_EXTRA } else { 0 };
        if e.alias.is_some() || e.kind >= 1 << KIND_BITS || e.field >= 1 << 6 || e.flags != extra {
            continue;
        }
        let bits: Option<u32> = e.classes.iter().map(|c| classes.iter().position(|x| x == c).map(|j| 1u32 << j)).sum();
        let Some(bits) = bits else { continue };
        let slot = u32::from(e.kind) | u32::from(e.field) << KIND_BITS | bits << 16;
        by[slot as usize] = n;
    }
    by
}

/// The id table's slots: a key's kind (10 bits) and field (6), then a bit for each supertype
/// class the program names (`classes`, at most `MAX_CLASSES`).
pub const fn id_slots(classes: usize) -> usize {
    1 << (16 + classes)
}
/// The most supertype classes a program's codes carry (`NODE_SUPER`'s kinds).
pub const MAX_CLASSES: usize = 4;

/// No code for a key yet (`id_of`).
pub const NO_CODE: i64 = 0xffff;

/// The flags of a value that did not fit its width (`Mem::Wide`): an offset, a tail (or an end
/// before the last descendant's start, from the last descendant).
pub const WIDE_OFF: i64 = 1;
pub const WIDE_TAIL: i64 = 2;
