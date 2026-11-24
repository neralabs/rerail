//! What tier 2's stream program is as a whole, apart from any lowering of it (`StreamFacts`): the marks its records hold and
//! tier 2 tests itself, where it is one function (`OneFn`) and its calls nest as its nodes (`stacked`), where each
//! node's key is known at its word (`coded`, `key_after`), where tier 2 makes the records itself (`Fetched`) and how a
//! region's parts go (`part_facts`). A format 16 run passes each node at its word where `direct_keys` says so, with the
//! writer's stack bounded by `open_bound`, for the JIT (`native_cpu_stream`). Also the local checks' word form
//! (`word_form`). Nothing here emits code.
//!
//! `facts` refuses the programs a direct run does not take yet. It reads no code a lowering writes: a part's content
//! is checked on the words' own successors (`word_succ`).

use crate::native_ir::{FlagOp, On, Structure, StructuralOperation as Op};
use crate::program::LocalOp;
use crate::stream_ir::{ByteSwitch, REFUSE, REMEMBERED, WORD_KIND};
use std::collections::{BTreeSet, HashMap};

/// The program's facts as a whole (`facts`), which the words' lowerings read.
pub struct StreamFacts<'a> {
    pub native: &'a Structure,
    /// A mark loses an extra (`SETTLE m=2`): the token taken makes it again.
    pub loses: bool,
    /// The marks the records hold, and the live bits at a token and at the stream's end
    /// (`native_cpu_stream`'s `in_code`, `rest_bits`, `end_bits`).
    pub in_code: usize,
    pub rest_bits: u64,
    pub end_bits: u64,
    /// The gap marks tier 2 tests itself (`records::lazy_marks`): `(mark, output)`.
    pub lazy: Vec<(u8, u16)>,
    /// Each kind's least byte, the stream's end as a kind past them, each word's kind.
    pub kind_reps: Option<Vec<u8>>,
    pub end_kind: Option<u8>,
    pub word_kind_of: Vec<u8>,
    /// The parts' own kinds (`records::Retag::part`): a token of one may be any word.
    pub part_kinds: Vec<usize>,
    pub framed: bool,
    pub context: bool,
    /// Each function's entry place.
    pub entries: Vec<usize>,
    /// Each word's phase (`native_cpu_stream::phases`), and whether a `TOKEN` has both (then a
    /// flag, `need`, says whether it takes a token, and `end` is the token's end): else each
    /// word knows both.
    pub phase: Vec<u8>,
    pub flagged: bool,
    /// The tape's widths and the language's tag (`native_cpu_stream::compile_at`'s).
    pub widths: rerail_tape_format::Widths,
    pub tag: u8,
    /// Where tier 2 makes each token's record from its start (page 40), as simdjson's stage 2
    /// takes a structural index: the rules and the region read as `parse_string`.
    pub fetch: Option<Fetched>,
    /// The supertype classes the tape's codes carry (`native_cpu_stream::supertype_classes`).
    pub classes: Vec<u16>,
    /// The tape sized from the input once (`Bounds::unchecked`): no room check a token.
    pub unchecked: bool,
    /// Where the program is one function, as `walk_document` (`OneFn`): its calls and returns.
    pub one: Option<OneFn>,
    /// Each node's code written as it is made (`coded`), no commit pass after.
    pub coded: bool,
    /// Format 16 (decision 0434): each node passed to the writer's ops at its word (`OPEN`, `LEAF`, `CLOSE`, the
    /// root's at `ACCEPT`), its number by its key, which `coded` knows there; no format 15 image.
    pub sixteen: bool,
    /// The stream's end as a start: past the last token, `TOKEN` gives the file's length as its
    /// start, with no branch (`sentinel`), as simdjson's index ends with one past the last.
    pub sentinel: bool,
}

/// The program's facts (`StreamFacts`), or why a direct run does not take it yet: the reasons the JIT's direct
/// format 16 run (`native_cpu_stream`) keeps.
pub fn facts(native: &Structure, widths: rerail_tape_format::Widths, tag: u8, sixteen: bool) -> Result<StreamFacts<'_>, String> {
    if !crate::stream_analysis::calls_on_stack(native) {
        return Err("calls are not on the machine's stack (`calls_on_stack`)".into());
    }
    let fetch = fetched(native)?;
    let late = native.block_programs.iter().filter_map(|b| b.records.as_ref()).any(|r| r.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2));
    if late && fetch.as_ref().is_none_or(|f| f.seq.is_none()) {
        return Err("a region tier 2 splits (decision 0323), not read as `parse_string`: not yet ahead of time".into());
    }
    let classes = crate::stream_analysis::supertype_classes(native).map_err(|e| format!("supertype classes: {e:?}"))?;
    let records = native.block_programs.first().and_then(|b| b.records.as_ref());
    let starts = native.structural.iter().any(|b| matches!(b.operation, Op::Token { .. }));
    let in_code = records.map_or(0, |r| r.mark_bits());
    let lazy: Vec<(u8, u16)> = native.block_programs.first().and_then(|b| b.records.as_ref().map(|r| (b, r))).map_or(Vec::new(), |(b, r)| {
        let bw = crate::byte_needs::MadeFromBytes::of(b);
        crate::records::lazy_marks(r, &|o| bw.can(o))
    });
    let lazy_bits: u64 = lazy.iter().fold(0, |a, &(k, _)| a | 1 << k);
    // The code's token marks with no bit (`Records::alone`): live at every token, as past it.
    let rest_bits: u64 = (if in_code >= 64 { 0 } else { u64::MAX << in_code }) | lazy_bits | records.map_or(0, |r| u64::from(r.alone));
    let mut end_bits = rest_bits;
    for b in &native.structural {
        if let Op::Mark { order, end: true, .. } = b.operation
            && usize::from(order) < in_code
        {
            end_bits |= 1 << order;
        }
    }
    let kind_reps: Option<Vec<u8>> = records.filter(|_| starts).map(|r| {
        let n = r.kinds.iter().copied().max().map_or(0, |m| usize::from(m) + 1);
        (0..n).map(|k| r.kinds.iter().position(|&x| usize::from(x) == k).unwrap_or(0) as u8).collect()
    });
    let end_kind = kind_reps.as_ref().and_then(|r| u8::try_from(r.len()).ok());
    let word_kind_of: Vec<u8> = records.filter(|_| starts).map_or_else(Vec::new, |r| r.words.iter().map(|w| r.kinds[usize::from(w[0])]).collect());
    let part_kinds: Vec<usize> = records.map_or(Vec::new(), |r| r.retags.iter().filter(|t| t.part).map(|t| usize::from(t.kind)).collect());
    let words = &native.structural;
    let framed = words.iter().any(|b| matches!(b.operation, Op::Wrap { .. } | Op::Reread { .. } | Op::Retag { on: On::Made, .. } | Op::NodeField { on: On::Made | On::MadeUnset | On::Inner, .. }));
    let context = words.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. } | Op::Push { flag: Some(_), .. }));
    let mut entries: BTreeSet<usize> = BTreeSet::from([0]);
    for (pc, b) in words.iter().enumerate() {
        if let Op::Push { .. } = b.operation {
            entries.insert(crate::stream_analysis::callee(native, pc));
        }
    }
    let phase = crate::stream_analysis::phases(native, &crate::stream_analysis::push_targets(native));
    let flagged = phase.contains(&(crate::stream::TAKEN | crate::stream::UNTAKEN));
    let loses = native.structural.iter().any(|b| matches!(b.operation, Op::LoseLast { .. }));
    if fetch.is_some() && (flagged || !native.extras.is_empty()) {
        return Err("records made in tier 2, with a token's taking flagged or extras: not yet ahead of time".into());
    }
    let unchecked = crate::stream_analysis::bounds(native).unchecked(crate::stream_analysis::marks_of(words));
    let mut facts = StreamFacts { native, loses, in_code, rest_bits, end_bits, lazy, kind_reps, end_kind, word_kind_of, part_kinds, framed, context, entries: entries.into_iter().collect(), phase, flagged, widths, tag, fetch, classes, unchecked, one: None, sentinel: false, coded: false, sixteen,  };
    facts.sentinel = sentinel(&facts);
    // Where tier 2 makes the records, the program is one function, as `walk_document`.
    if facts.fetch.is_some() && !facts.context {
        let one = OneFn::of(native, &|w| plain_succ(&facts, w));
        facts.one = Some(one);
        facts.entries = vec![0];
        let st = stacked(&facts);
        if let Some(one) = &mut facts.one {
            one.stacked = st.is_some();
            one.most = st.unwrap_or(0);
        }
        facts.coded = coded(&facts);
    }
    if sixteen && facts.coded && !native.readings.provisional.is_empty() {
        return Err("format 16 ahead of time: provisional fields, which `ACCEPT` reads from the rows".into());
    }
    if let Some(fx) = &facts.fetch {
        for (pc, word) in words.iter().enumerate() {
            if matches!(word.operation, Op::Token { .. }) && fx.seen[pc] == [true, true] {
                return Err(format!("word {pc}: a `TOKEN` where a part may be pending or not"));
            }
            if fx.part_at(&facts, pc) {
                part_facts(&facts, fx, pc, &|q| word_succ(&facts, q))?;
            }
        }
    }
    Ok(facts)
}

/// A word's successors in its function, as its operation names them: its next, its targets, a call's two returns (in
/// one function, `OneFn`: a call's rule, a return's places); past a node's field set by the next word.
pub fn word_succ(x: &StreamFacts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    if let Some(one) = &x.one {
        match x.native.structural[pc].operation {
            Op::Push { .. } => return vec![crate::stream_analysis::callee(x.native, pc)],
            Op::Return { .. } => return one.returns.get(&pc).map_or(Vec::new(), |r| r.iter().map(|&(_, t)| t).collect()),
            _ => {}
        }
    }
    let v = match x.native.structural[pc].operation {
        // A node's field set by the next word, stored with its kind: on past that word.
        Op::Open { .. } | Op::Leaf { .. } if field_after(x, pc).is_some() => vec![pc + 2],
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => x.native.switches[usize::from(table)].all_targets().filter(|&t| t != REFUSE).map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } | Op::MadeIs { .. } => vec![pc + 1, pc + 2],
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}
/// Whether the end can be a start (`StreamFacts::sentinel`): records made in tier 2, no marks (their
/// `live` at the end is its own), and no `TOKEN` after a switch takes the end (a token taken
/// past the end refuses, where the end as a start would be the end again).
pub fn sentinel(x: &StreamFacts<'_>) -> bool {
    let words = &x.native.structural;
    if x.fetch.is_none() || x.flagged || words.iter().any(|w| matches!(w.operation, Op::Mark { .. })) {
        return false;
    }
    let mut seen = vec![false; words.len()];
    let mut work: Vec<usize> = x.native.switches.iter().map(|sw| usize::from(sw.end)).filter(|&t| t < words.len()).collect();
    while let Some(pc) = work.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        if matches!(words[pc].operation, Op::Token { .. }) {
            return false;
        }
        work.extend(plain_succ(x, pc));
    }
    true
}

/// A program written as one function, as simdjson's `walk_document`: a call (`PUSH`) stores its
/// site's number on a stack of its own (`S::ret`, as `is_array[depth]`) and goes on at the rule's
/// first word; a `RETURN` takes the number off and goes on at that site's place, as `scope_end`
/// goes back to the state of the scope it returns to.
pub struct OneFn {
    /// Each call's place, and the number it stores: calls that every `RETURN` sends to the same
    /// place store one number, as `is_array` is one bit for a scope's two states.
    pub sites: Vec<(usize, usize)>,
    /// Each `RETURN`'s numbers (those of the calls whose rule holds it), each with its place.
    pub returns: HashMap<usize, Vec<(usize, usize)>>,
    /// Each call's places after it: where its rule's returns go back to (its target, and the one
    /// after only where a `RETURN` gives `again`).
    pub after: HashMap<usize, Vec<usize>>,
    /// The calls nested as the containers (`stacked`): a call's number kept in its container's
    /// row, and no `CLOSE` that can find no node open.
    pub stacked: bool,
    /// Where they nest so: the most nodes a rule holds open at a word, counted as `stacked` counts them (a call's
    /// container its callee's).
    pub most: i64,
}

/// Whether each node's code can be written as it is made (`StreamFacts::coded`): no word edits a
/// node made (`stacked`'s words; a field only on the node made last), and every `NODE_SUPER` and
/// `NODE_FIELD` is reached only from a node's `OPEN` or `LEAF` by words that make no node
/// (`GOTO`, `SPAN`, another of them), so the node's key is its own and the words after it
/// (`key_after`), known when it is made.
pub fn coded(x: &StreamFacts<'_>) -> bool {
    let words = &x.native.structural;
    if !x.one.as_ref().is_some_and(|o| o.stacked) || words.iter().any(|w| matches!(w.operation, Op::NodeField { on, .. } if on != On::Here)) {
        return false;
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for t in plain_succ(x, pc) {
            preds[t].push(pc);
        }
    }
    // A straight word's way back reaches a node's word on every path.
    let mut ok: Vec<Option<bool>> = vec![None; words.len()];
    fn back(x: &StreamFacts<'_>, preds: &[Vec<usize>], ok: &mut Vec<Option<bool>>, q: usize) -> bool {
        if let Some(v) = ok[q] {
            return v;
        }
        ok[q] = Some(false);
        let words = &x.native.structural;
        let v = !preds[q].is_empty()
            && preds[q].iter().all(|&p| match words[p].operation {
                Op::Open { .. } | Op::Leaf { .. } => plain_succ(x, p).len() == 1,
                Op::Goto { .. } | Op::Span { .. } | Op::NodeSuper { .. } | Op::NodeField { on: On::Here, .. } => plain_succ(x, p).len() == 1 && back(x, preds, ok, p),
                _ => false,
            });
        ok[q] = Some(v);
        v
    }
    (0..words.len()).filter(|&q| matches!(words[q].operation, Op::NodeSuper { .. } | Op::NodeField { .. })).all(|q| back(x, &preds, &mut ok, q))
}

/// The key bits node word `pc`'s key gets from the words after it that make no node (`coded`):
/// each supertype's class (`native_cpu_tape::commit`'s), and a field set there (in `kf`'s
/// place, above its kind). Of a node with its field in the next word (`field_after`), that one
/// is its own already.
pub fn key_after(x: &StreamFacts<'_>, pc: usize) -> (u32, u32) {
    let words = &x.native.structural;
    let (mut bits, mut field) = (0u32, 0u32);
    let mut q = pc;
    for _ in 0..words.len() {
        let &[t] = plain_succ(x, q).as_slice() else { break };
        match words[t].operation {
            Op::NodeSuper { kind } => {
                if let Some(j) = x.classes.iter().position(|&c| c == kind) {
                    bits |= 1 << (16 + j);
                }
            }
            Op::NodeField { field: f, on: On::Here } => field = u32::from(f) << 16,
            Op::Goto { .. } | Op::Span { .. } => {}
            _ => break,
        }
        q = t;
    }
    (field, bits)
}

/// Whether the calls nest as the nodes (`OneFn::stacked`): each `PUSH` right after its
/// container's `OPEN` (a supertype or field word between), each `RETURN` right after the `CLOSE`
/// of that container, and each rule's opens and closes counted alike on every way, a call the
/// close of the container before it: 0 at a `RETURN`'s `CLOSE`, and at least 1 at every other
/// `CLOSE` and every `PUSH`. Then that `CLOSE` closes the node the call's `OPEN` made, the
/// container's row holds the call's number until it (its end is not read before), and no
/// `CLOSE` finds no node open. Only words whose nodes this counts: no wrap, fold or drop. Where they do: the most
/// nodes a rule holds open at a word (`OneFn::most`, the stack's bound's).
pub fn stacked(x: &StreamFacts<'_>) -> Option<i64> {
    let words = &x.native.structural;
    let one = x.one.as_ref()?;
    let mut most = 0;
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
    for pc in 0..words.len() {
        for t in plain_succ(x, pc) {
            preds[t].push(pc);
        }
    }
    let only_from = |pc: usize, q: usize| preds[pc].as_slice() == [q];
    for &(pc, _) in &one.sites {
        let mut q = pc;
        loop {
            if q == 0 || !only_from(q, q - 1) {
                return None;
            }
            q -= 1;
            match words[q].operation {
                Op::Open { .. } => break,
                Op::NodeSuper { .. } | Op::NodeField { .. } => {}
                _ => return None,
            }
        }
    }
    for &r in one.returns.keys() {
        if r == 0 || !only_from(r, r - 1) || !matches!(words[r - 1].operation, Op::Close { .. }) {
            return None;
        }
    }
    // Each rule from its first word (the document's from word 0): a count a word, the same on
    // every way.
    let mut entries: Vec<usize> = vec![0];
    entries.extend(one.sites.iter().map(|&(pc, _)| crate::stream_analysis::callee(x.native, pc)));
    entries.sort_unstable();
    entries.dedup();
    for e in entries {
        let mut at: HashMap<usize, i64> = HashMap::from([(e, 0)]);
        let mut work = vec![e];
        while let Some(pc) = work.pop() {
            let c = at[&pc];
            most = most.max(c);
            let after = match words[pc].operation {
                Op::Open { .. } => c + 1,
                Op::Close { .. } => {
                    let last = plain_succ(x, pc).iter().any(|&t| matches!(words[t].operation, Op::Return { .. }));
                    if (last && c != 0 && e != 0) || (!last && c < 1) {
                        return None;
                    }
                    c - 1
                }
                Op::Push { .. } if c < 1 => return None,
                Op::Push { .. } => c - 1,
                Op::Token { .. } | Op::Switch { .. } | Op::Leaf { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Span { .. } | Op::Check { .. } | Op::Goto { .. } | Op::Return { .. } | Op::Accept | Op::Stop | Op::Empty { .. } => c,
                _ => return None,
            };
            most = most.max(after);
            for t in plain_succ(x, pc) {
                match at.get(&t) {
                    Some(&v) if v != after => return None,
                    Some(_) => {}
                    None => {
                        at.insert(t, after);
                        work.push(t);
                    }
                }
            }
        }
    }
    Some(most)
}

impl OneFn {
    pub fn of(native: &Structure, x: &dyn Fn(usize) -> Vec<usize>) -> OneFn {
        let words = &native.structural;
        let sites: Vec<usize> = (0..words.len()).filter(|&pc| matches!(words[pc].operation, Op::Push { .. })).collect();
        let mut returns: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for (k, &pc) in sites.iter().enumerate() {
            let Op::Push { target, .. } = words[pc].operation else { unreachable!() };
            // The rule's words: from its first, its calls not followed (`plain_succ`).
            let mut seen = vec![false; words.len()];
            let mut work = vec![crate::stream_analysis::callee(native, pc)];
            while let Some(w) = work.pop() {
                if std::mem::replace(&mut seen[w], true) {
                    continue;
                }
                if let Op::Return { again } = words[w].operation {
                    returns.entry(w).or_default().push((k, usize::from(target) + usize::from(again)));
                }
                work.extend(x(w));
            }
        }
        let mut after: HashMap<usize, Vec<usize>> = HashMap::new();
        for arms in returns.values() {
            for &(k, t) in arms {
                let v = after.entry(sites[k]).or_default();
                if !v.contains(&t) {
                    v.push(t);
                }
            }
        }
        // A call's number: the places every return sends it to, numbered as they first come.
        let mut rets: Vec<usize> = returns.keys().copied().collect();
        rets.sort_unstable();
        let place = |k: usize| -> Vec<Option<usize>> { rets.iter().map(|r| returns[r].iter().find(|&&(s, _)| s == k).map(|&(_, t)| t)).collect() };
        let mut kinds: Vec<Vec<Option<usize>>> = Vec::new();
        let number: Vec<usize> = (0..sites.len())
            .map(|k| {
                let p = place(k);
                kinds.iter().position(|q| *q == p).unwrap_or_else(|| {
                    kinds.push(p);
                    kinds.len() - 1
                })
            })
            .collect();
        for arms in returns.values_mut() {
            for a in arms.iter_mut() {
                a.0 = number[a.0];
            }
            arms.sort_unstable();
            arms.dedup();
        }
        OneFn { sites: sites.into_iter().zip(number).collect(), returns, after, stacked: false, most: 0 }
    }
}

/// The facts of a program whose tier 2 makes the records (`Records::fetched_in_tier2`).
pub struct Fetched {
    pub ft: Box<crate::stream_analysis::Fetch>,
    /// The region tier 2 splits, read as `parse_string` (`native_cpu_stream::Seq`), and its
    /// part's first byte.
    pub seq: Option<(crate::byte_needs::Seq, u8)>,
    /// Each word reached with no part pending, and with one (`parts_pending_seen`).
    pub seen: Vec<[bool; 2]>,
}

impl Fetched {
    /// A `TOKEN` where a part of the region is always pending: the string's next part.
    pub fn part_at(&self, x: &StreamFacts<'_>, pc: usize) -> bool {
        matches!(x.native.structural[pc].operation, Op::Token { .. }) && self.seen[pc] == [false, true]
    }
}

/// Where every node's key is known at its word (`StreamFacts::coded`) and no leaf is deferred or an extra: each node word's
/// key bits from the words after it (`key_after`: its field, its supertypes' classes), by its place. A format 16 run
/// of such a program passes each node to the writer's ops at its word, with no row (`native_cpu_stream`'s, as
/// `rust_source_16`'s). `None` elsewhere.
pub fn direct_keys(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Option<Vec<(u32, u32)>> {
    let x = facts(native, widths, tag, true).ok()?;
    let plain = native.extras.is_empty() && native.readings.provisional.is_empty() && !native.structural.iter().any(|w| matches!(w.operation, Op::Leaf { deferred: true, .. } | Op::Leaf { extra: true, .. }));
    (x.coded && plain).then(|| (0..native.structural.len()).map(|pc| key_after(&x, pc)).collect())
}

/// The most nodes a direct run of the program holds open (`f16::Writer::reserve_run`'s depth): a rule holds at most
/// `OneFn::most` at a word, a call nests at most `native_cpu_stream::DEPTH` deep and keeps its container open. With
/// the stack reserved so up front, a direct run checks its room once, where it starts, as the lean parser sizes its
/// stacks once (`DEFAULT_MAX_DEPTH`), and no node checks it. `None` where the program is not one a direct run takes.
pub fn open_bound(native: &Structure, widths: rerail_tape_format::Widths, tag: u8) -> Option<u64> {
    open_bound_of(&facts(native, widths, tag, true).ok()?)
}

pub fn open_bound_of(x: &StreamFacts<'_>) -> Option<u64> {
    let one = x.one.as_ref().filter(|o| o.stacked && x.coded && x.sixteen)?;
    let depth = crate::stream::DEPTH as u64;
    Some((depth + 1) * one.most.max(0) as u64 + depth)
}

/// A local check's word form (`program::local_value` over the 64 places of a word, a bit
/// each): the values each op's needs, `(op, words back)`, a `Prev` reading the word before its
/// operand's too; and the byte sets' classes, tier 1's form (a class a bit of a byte, by two
/// nibble tables, `lo[b & 15] & hi[b >> 4]`, eight classes a pair of tables), each set's table
/// and class bits.
pub struct WordForm {
    pub need: BTreeSet<(usize, usize)>,
    pub tables: Vec<([u8; 16], [u8; 16])>,
    pub class_of: HashMap<usize, (usize, u8)>,
}

pub fn word_form(ops: &[LocalOp]) -> Result<WordForm, String> {
    let n = ops.len();
    let mut need: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut todo = vec![(n - 1, 0usize)];
    while let Some((k, t)) = todo.pop() {
        if !need.insert((k, t)) {
            continue;
        }
        match ops[k] {
            LocalOp::Not(a) => todo.push((usize::from(a), t)),
            LocalOp::And(a, b) | LocalOp::Or(a, b) | LocalOp::Xor(a, b) | LocalOp::AndNot(a, b) => todo.extend([(usize::from(a), t), (usize::from(b), t)]),
            LocalOp::Prev(a, _) => todo.extend([(usize::from(a), t), (usize::from(a), t + 1)]),
            _ => {}
        }
    }
    let mut tables: Vec<([u8; 16], [u8; 16])> = Vec::new();
    let mut class_of: HashMap<usize, (usize, u8)> = HashMap::new();
    let mut classes = 0usize;
    for (k, op) in ops.iter().enumerate() {
        let LocalOp::Bytes(set) = *op else { continue };
        let member = |b: usize| set[b / 64] >> (b % 64) & 1 == 1;
        // Each high nibble's low nibbles; each distinct nonempty set of them a class.
        let mut pats: Vec<u16> = Vec::new();
        let mut of_hi = [0u16; 16];
        for (h, slot) in of_hi.iter_mut().enumerate() {
            *slot = (0..16).filter(|&l| member(h * 16 + l)).fold(0u16, |m, l| m | 1 << l);
            if *slot != 0 && !pats.contains(slot) {
                pats.push(*slot);
            }
        }
        if pats.len() > 8 {
            return Err(format!("a byte set of {} classes", pats.len()));
        }
        if classes % 8 + pats.len() > 8 {
            classes = classes.div_ceil(8) * 8;
        }
        let mut bits = 0u8;
        for (pi, &pat) in pats.iter().enumerate() {
            let at = classes + pi;
            while tables.len() <= at / 8 {
                tables.push(([0; 16], [0; 16]));
            }
            let (lo, hi) = &mut tables[at / 8];
            let bit = 1u8 << (at % 8);
            for (l, cell) in lo.iter_mut().enumerate() {
                if pat >> l & 1 == 1 {
                    *cell |= bit;
                }
            }
            for h in 0..16 {
                if of_hi[h] == pat {
                    hi[h] |= bit;
                }
            }
            bits |= bit;
        }
        class_of.insert(k, (classes / 8, bits));
        classes += pats.len();
    }
    Ok(WordForm { need, tables, class_of })
}

/// A word's successors as its operation names them (`succ` before the part's `TOKEN` and a
/// switch's sending its shared bytes on): its next, its targets, a call's two returns.
pub fn plain_succ(x: &StreamFacts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    let v = match x.native.structural[pc].operation {
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => x.native.switches[usize::from(table)].all_targets().filter(|&t| t != REFUSE).map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } | Op::MadeIs { .. } => vec![pc + 1, pc + 2],
        // In one function (`OneFn`), only the places its rule's returns go back to.
        Op::Push { .. } if x.one.is_some() => x.one.as_ref().and_then(|o| o.after.get(&pc)).cloned().unwrap_or_default(),
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}

/// Whether rows carry marks (`head`): where a field is set on a wrap's inner node, or a reread
/// may meet a dropped row's place (`native_cpu_stream::TableFns` keeps the same).
pub fn marks(n: &Structure) -> bool {
    n.structural.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. } | Op::Reread { .. }))
}

/// The field the word after a node's sets on it (`NODE_FIELD` on the node just made), stored
/// with the node's kind: one 32-bit store of both, as the JIT's (`with_field`).
pub fn field_after(x: &StreamFacts<'_>, pc: usize) -> Option<u16> {
    match x.native.structural.get(pc + 1).map(|w| w.operation) {
        Some(Op::NodeField { field, on: On::Here }) => Some(field),
        _ => None,
    }
}

/// Where tier 2 makes the records (`Records::fetched_in_tier2`): its facts; refused where a
/// first byte's rule is one this backend does not write (a pair of two bytes, a last-byte mask,
/// a region not read as `parse_string`, a run whose output is not a byte class), or a word
/// whose first byte begins a run is not one simdjson's atom test reads (eight bytes or fewer,
/// each of the run's class).
pub fn fetched(native: &Structure) -> Result<Option<Fetched>, String> {
    use crate::records::EndRule;
    let Some(ft) = crate::stream_analysis::fetch_of(native).map_err(|e| format!("records made in tier 2: {e:?}"))? else { return Ok(None) };
    let split = crate::stream_analysis::split_parts(native).map_err(|e| format!("a region tier 2 splits: {e:?}"))?;
    let seq = match split.and_then(|sp| sp.seq.clone()) {
        Some(q) => {
            let es: Vec<u8> = (0..=255u8).filter(|&b| q.stops[usize::from(b) / 64] >> (b % 64) & 1 == 1 && q.xs[usize::from(b) / 64] >> (b % 64) & 1 == 0).collect();
            let &[e0] = es.as_slice() else { return Err("a region read as `parse_string` whose part begins with more than one byte".into()) };
            if q.codes2.is_none() {
                return Err("a region read as `parse_string` with no part codes".into());
            }
            Some((q, e0))
        }
        None => None,
    };
    for b in 0..256usize {
        match ft.rules.get(usize::from(ft.ends[b])) {
            None => {}
            Some(r) if crate::stream_analysis::one_byte(&ft, r) => {}
            Some(&EndRule::Read(i)) if ft.seq[usize::from(i)] && seq.is_some() => {}
            Some(&EndRule::Run(_)) => {
                let class = ft.run_class[usize::from(ft.ends[b])].as_ref().ok_or_else(|| format!("byte {b:#04x}: a run whose output is not a byte class"))?;
                // A word with a byte out of the run's class is never the run's token.
                for (w, _) in ft.words.iter().filter(|(w, _)| w.first() == Some(&(b as u8)) && w.iter().all(|&y| class[usize::from(y)] == 1)) {
                    if w.len() > 8 {
                        return Err(format!("byte {b:#04x}: a word ({w:?}) that is not an atom of its run"));
                    }
                }
            }
            Some(r) => return Err(format!("byte {b:#04x}: records made in tier 2 by {r:?}: not yet ahead of time")),
        }
    }
    let seen = crate::stream_analysis::parts_pending_seen(native, &crate::stream_analysis::push_targets(native), &ft);
    Ok(Some(Fetched { ft, seq, seen }))
}

/// A token's place by the switch's kind path (`switch_plan`): its kind's (its byte's first,
/// `kind_reps`), or its word's where the switch names it.
pub fn kind_target(x: &StreamFacts<'_>, fx: &Fetched, sw: &ByteSwitch, code: u32) -> u16 {
    let w = (code >> 8) & 0xff;
    if w > 0
        && let Some(&(_, t)) = sw.words.iter().find(|&&(y, _)| u32::from(y) + 1 == w)
    {
        return t;
    }
    let kind = (code & 0xff) as usize;
    let rep = x.kind_reps.as_ref().and_then(|r| r.get(kind)).map_or_else(|| fx.ft.kinds.iter().position(|&k| usize::from(k) == kind).unwrap_or(0), |&b| usize::from(b));
    sw.targets[rep]
}

/// Whether switch `sw` is reached only from the `TOKEN` before it, where that takes a token: its
/// token not made yet, or the stream's end.
pub fn fresh(x: &StreamFacts<'_>, sw: &ByteSwitch) -> bool {
    let words = &x.native.structural;
    let Some(here) = words.iter().position(|w| matches!(w.operation, Op::Switch { table } if std::ptr::eq(&x.native.switches[usize::from(table)], sw))) else { return false };
    if here == 0 || !matches!(words[here - 1].operation, Op::Token { .. }) || x.phase[here - 1] != crate::stream::TAKEN || x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, here - 1)) {
        return false;
    }
    (0..words.len()).all(|pc| pc == here - 1 || !plain_succ(x, pc).contains(&here))
}

/// What a part's `TOKEN` (`Fetched::part_at`) and the switch after it go to: the closer's
/// place, the content's, the part's by its word (else its kind's); the closer's code.
pub struct PartFacts {
    pub closer: u16,
    pub content: u16,
    pub words: Vec<(u16, u16)>,
    pub part: u16,
    pub closer_code: u32,
}

/// Whether the token's code is read from word `t` on (each word's successors by `succ`: the words' own,
/// `word_succ`, or a lowering's, which are those wherever this walk reads them), before a word that makes the next
/// token's or this one's again: a `TOKEN`, a part (which writes its own) and a switch on the
/// first byte (`fresh`) end the walk; a switch with a kind path reads it. Of the other words,
/// those known to read no code are passed (each successor, a call's returns); any other reads it.
pub fn reads_code(x: &StreamFacts<'_>, t: u16, succ: &dyn Fn(usize) -> Vec<usize>) -> bool {
    let words = &x.native.structural;
    let mut seen = vec![false; words.len()];
    let mut work = vec![usize::from(t)];
    while let Some(pc) = work.pop() {
        if pc >= words.len() || std::mem::replace(&mut seen[pc], true) || x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, pc)) {
            continue;
        }
        match words[pc].operation {
            Op::Token { .. } | Op::Stop => continue,
            Op::Switch { table } if fresh(x, &x.native.switches[usize::from(table)]) => continue,
            Op::Goto { .. } | Op::Span { .. } | Op::Open { .. } | Op::Close { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Check { .. } | Op::Count { .. } => {}
            Op::Leaf { kind, .. } if kind != WORD_KIND && kind != REMEMBERED => {}
            Op::Push { .. } | Op::Return { .. } if x.one.is_some() => {}
            _ => return true,
        }
        work.extend(succ(pc));
    }
    false
}

pub fn part_facts(x: &StreamFacts<'_>, fx: &Fetched, pc: usize, succ: &dyn Fn(usize) -> Vec<usize>) -> Result<PartFacts, String> {
    let (q, e0) = fx.seq.as_ref().ok_or_else(|| format!("word {pc}: a part pending with no region read as `parse_string`"))?;
    let Some(Op::Switch { table }) = x.native.structural.get(pc + 1).map(|w| w.operation) else {
        return Err(format!("word {pc}: a part's `TOKEN` not before a switch"));
    };
    if !matches!(x.native.structural[pc].operation, Op::Token { any: None, extras: false, .. }) || x.phase[pc] != crate::stream::TAKEN {
        return Err(format!("word {pc}: a part's `TOKEN` that tests a mark or takes no token"));
    }
    let sw = &x.native.switches[usize::from(table)];
    let ft = &fx.ft;
    let in_set = |set: &[u64; 4], b: usize| set[b / 64] >> (b % 64) & 1 == 1;
    let one = |ts: BTreeSet<u16>, what: &str| -> Result<u16, String> {
        match ts.into_iter().collect::<Vec<_>>().as_slice() {
            &[t] => Ok(t),
            ts => Err(format!("word {pc}: a part's {what} goes to {ts:?}")),
        }
    };
    let closer = one((0..256).filter(|&b| in_set(&q.xs, b)).map(|b| kind_target(x, fx, sw, ft.codes1[b])).collect(), "closer")?;
    let codes: BTreeSet<u32> = (0..256).filter(|&b| in_set(&q.xs, b)).map(|b| ft.codes1[b]).collect();
    let closer_code = match codes.into_iter().collect::<Vec<_>>().as_slice() {
        &[c] => c,
        _ => return Err(format!("word {pc}: a closer of more than one code")),
    };
    // The content: its place by its first byte's kind; no word the switch names begins with it,
    // and no word after reads its code (which is then not made).
    let content = one((0..256).filter(|&b| !in_set(&q.stops, b)).map(|b| kind_target(x, fx, sw, u32::from(ft.kinds[b]))).collect(), "content")?;
    if sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first().is_some_and(|&f| !in_set(&q.stops, usize::from(f))))) || reads_code(x, content, succ) {
        return Err(format!("word {pc}: a content part whose code is read"));
    }
    let codes2 = q.codes2.as_ref().expect("checked in `fetched`");
    let kinds: BTreeSet<u32> = codes2.iter().map(|c| c & 0xff).collect();
    let &[kind] = kinds.iter().copied().collect::<Vec<_>>().as_slice() else { return Err(format!("word {pc}: a part of more than one kind")) };
    let part = kind_target(x, fx, sw, kind);
    let _ = e0;
    Ok(PartFacts { closer, content, words: sw.words.clone(), part, closer_code })
}
