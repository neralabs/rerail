//! What the JIT's tier 2 knows of a program before it compiles it (`native_cpu_stream`): whether a format 16 run can
//! pass each node to the writer as it is made, each node word's key bits (`direct_keys`), the most nodes such a run
//! holds open (`open_bound`), whether rows carry marks (`marks`), and a local check's word form (`word_form`).
//!
//! The program's facts (`facts`) are the analysis the deprecated Rust backend (`rust_stream`, decision 0550) made
//! before it wrote a program: kept here, where the JIT reads them, with the Rust it wrote removed (decision 0565).


use crate::native_ir::{FlagOp, On, Structure, StructuralOperation as Op};

use crate::program::{LocalCheck, LocalOp};

use crate::stream_ir::{ByteSwitch, DEAD, REFUSE, REMEMBERED, WORD_KIND};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use std::fmt::Write;

const UNNUMBERED: &str = "// No number in the language for this node: the file refused (`fail::NUMBER`).\nreturn refuse(why::NUMBER);";

/// The program's facts the words read.
struct Facts<'a> {
    native: &'a Structure,
    /// The marks the records hold (`native_cpu_stream`'s `in_code`).
    in_code: usize,
    /// The gap marks tier 2 tests itself (`native_cpu_block::lazy_marks`): `(mark, output)`.
    lazy: Vec<(u8, u16)>,
    /// Each kind's least byte, the stream's end as a kind past them, each word's kind.
    kind_reps: Option<Vec<u8>>,
    end_kind: Option<u8>,
    word_kind_of: Vec<u8>,
    context: bool,
    /// Each function's entry place.
    entries: Vec<usize>,
    /// Each word's phase (`native_cpu_stream::phases`), and whether a `TOKEN` has both (then a
    /// flag, `need`, says whether it takes a token, and `end` is the token's end): else each
    /// word knows both.
    phase: Vec<u8>,
    flagged: bool,
    /// Where tier 2 makes each token's record from its start (page 40), as simdjson's stage 2
    /// takes a structural index: the rules and the region read as `parse_string`.
    fetch: Option<Fetched>,
    /// The supertype classes the tape's codes carry (`native_cpu_stream::supertype_classes`).
    classes: Vec<u16>,
    /// The tape sized from the input once (`Bounds::unchecked`): no room check a token.
    unchecked: bool,
    /// Where the program is one function, as `walk_document` (`OneFn`): its calls and returns.
    one: Option<OneFn>,
    /// Each node's code written as it is made (`coded`), no commit pass after.
    coded: bool,
    /// Format 16 (decision 0434): each node passed to the writer's ops at its word (`OPEN`, `LEAF`, `CLOSE`, the
    /// root's at `ACCEPT`), its number by its key, which `coded` knows there; no format 15 image.
    sixteen: bool,
    /// The stream's end as a start: past the last token, `TOKEN` gives the file's length as its
    /// start, with no branch (`sentinel`), as simdjson's index ends with one past the last.
    sentinel: bool,
    /// A format 16 source's language's numbers by key (`native_cpu_stream::numbers_by_key`): a direct run writes
    /// each node's as a literal at its word. Empty elsewhere.
    numbers: Vec<u32>,
    /// The regions read as a method each (`string_methods`), as the lean parser's `string`: each part's `TOKEN` the
    /// name of the method its loop is, one method a loop of the same code.
    strings: HashMap<usize, String>,
    /// The opener's place is passed to the region's method (`s.string(s.start + 1)`), as the lean parser's
    /// `string(v)`, where every way into each method comes from an opener's arm with the token's start unchanged
    /// (`opener_place_at_call`); else the arm sets `s.part_at`, which the method reads.
    part_arg: bool,
    /// The methods the switches' arms call (`Facts::method`): each its name and its signature and body, one a code.
    methods: std::cell::RefCell<Vec<(String, String)>>,
    /// Where the last token's end is read from (`last_ends`): `None` where a `TOKEN` keeps it (`t.last_end`).
    ends: Option<LastEnds>,
    /// A direct format 16 run whose writer's room is reserved once (`reserved`): its `OPEN` and `LEAF` cannot fail.
    reserved: bool,
    /// The containers written as one leaf where they are empty (`empties`), by their `OPEN`.
    empties: HashMap<usize, EmptyAt>,
    /// The document's containers whose closer must be the stream's last token (`last_closers`), by their `OPEN`.
    closers: HashMap<usize, u8>,
}

/// A container's `OPEN` whose call's first switch closes it on one byte at once (`empties`): where the next token is
/// that byte, the container is one leaf, as the lean parser's `empty!` writes `{}` and `[]`, and the walk goes on at
/// the call's return place (`after`). `table` and `q` are the switch whose arm takes the closer, and that arm's place.
struct EmptyAt {
    after: usize,
    byte: u8,
    table: usize,
    q: usize,
}

/// The last token's end without a copy a token (`last_ends`), as the lean parser passes the `end` it holds: a `TOKEN`
/// leaves it in `rec_end`, which an arm makes again for its own token only where a word reads that. A `CLOSE` of the
/// last token's end reads `rec_end` where no arm made it again on any way in (`rec`), else `last_end`, which each
/// writing arm of the switches (by table) in `copies` keeps first.
struct LastEnds {
    rec: HashSet<usize>,
    copies: HashSet<usize>,
}

impl Facts<'_> {
    /// The method of signature and body `code`, named for `base` (`base`, else `base_N` where another method has that
    /// name): one method a code, however many arms call it, as the lean parser's `primitive` from each place.
    fn method(&self, base: &str, code: String) -> String {
        let mut ms = self.methods.borrow_mut();
        if let Some((name, _)) = ms.iter().find(|(_, c)| *c == code) {
            return name.clone();
        }
        let same = ms.iter().filter(|(n, _)| n == base || n.strip_prefix(base).is_some_and(|r| r.starts_with('_') && r[1..].bytes().all(|b| b.is_ascii_digit()))).count();
        let name = if same == 0 { base.to_string() } else { format!("{base}_{same}") };
        ms.push((name.clone(), code));
        name
    }

    /// The number of the node of key `kf` (kind, field at bit 16) and supertype class bits `classes` in the source's
    /// language, `None` where its table has none (the run refuses there, as the JIT's at `fail::NUMBER`).
    fn number(&self, kf: u32, classes: u32) -> Option<u32> {
        let k = (kf & 0x3ff) | ((kf >> 16) & 0x3f) << rerail_tape_format::KIND_BITS | classes;
        self.numbers.get(k as usize).copied().filter(|&n| n != u32::MAX)
    }
}

impl Facts<'_> {
    /// A node's writer call's `?`, where it can refuse: none where the writer's room is reserved (`reserved`).
    fn may_fail(&self) -> &'static str {
        if self.reserved { "" } else { "?" }
    }

    /// The end's test, of the token `t` in the run `s`.
    fn at_end(&self, t: &str, s: &str) -> String {
        if self.sentinel { format!("({t}.start == {s}.len)") } else { format!("{t}.at_end") }
    }
}

/// Whether the end can be a start (`Facts::sentinel`): records made in tier 2, no marks (their
/// `live` at the end is its own), and no `TOKEN` after a switch takes the end (a token taken
/// past the end refuses, where the end as a start would be the end again).
fn sentinel(x: &Facts<'_>) -> bool {
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
struct OneFn {
    /// Each call's place, and the number it stores: calls that every `RETURN` sends to the same
    /// place store one number, as `is_array` is one bit for a scope's two states.
    sites: Vec<(usize, usize)>,
    /// Each `RETURN`'s numbers (those of the calls whose rule holds it), each with its place.
    returns: HashMap<usize, Vec<(usize, usize)>>,
    /// Each call's places after it: where its rule's returns go back to (its target, and the one
    /// after only where a `RETURN` gives `again`).
    after: HashMap<usize, Vec<usize>>,
    /// The calls nested as the containers (`stacked`): a call's number kept in its container's
    /// row, and no `CLOSE` that can find no node open.
    stacked: bool,
    /// Where they nest so: the most nodes a rule holds open at a word, counted as `stacked` counts them (a call's
    /// container its callee's).
    most: i64,
}

/// Whether each node's code can be written as it is made (`Facts::coded`): no word edits a
/// node made (`stacked`'s words; a field only on the node made last), and every `NODE_SUPER` and
/// `NODE_FIELD` is reached only from a node's `OPEN` or `LEAF` by words that make no node
/// (`GOTO`, `SPAN`, another of them), so the node's key is its own and the words after it
/// (`key_after`), known when it is made.
fn coded(x: &Facts<'_>) -> bool {
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
    fn back(x: &Facts<'_>, preds: &[Vec<usize>], ok: &mut Vec<Option<bool>>, q: usize) -> bool {
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
fn key_after(x: &Facts<'_>, pc: usize) -> (u32, u32) {
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
fn stacked(x: &Facts<'_>) -> Option<i64> {
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
    entries.extend(one.sites.iter().map(|&(pc, _)| crate::native_cpu_stream::callee(x.native, pc)));
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
    fn of(native: &Structure, x: &dyn Fn(usize) -> Vec<usize>) -> OneFn {
        let words = &native.structural;
        let sites: Vec<usize> = (0..words.len()).filter(|&pc| matches!(words[pc].operation, Op::Push { .. })).collect();
        let mut returns: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for (k, &pc) in sites.iter().enumerate() {
            let Op::Push { target, .. } = words[pc].operation else { unreachable!() };
            // The rule's words: from its first, its calls not followed (`plain_succ`).
            let mut seen = vec![false; words.len()];
            let mut work = vec![crate::native_cpu_stream::callee(native, pc)];
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

/// The number the document's own calls store, where the calls nest as the nodes (`OneFn::stacked`): the calls in the
/// document's words (from word 0, no call followed), none of them in a rule's, all storing one number that no other
/// call stores. Their return is the one that leaves the depth at 0, as the lean parser's `scope_end` ends the walk
/// where `d == 0`, so it is told by the depth, not by the number.
fn root_number(x: &Facts<'_>) -> Option<usize> {
    let one = x.one.as_ref().filter(|o| o.stacked)?;
    let words = &x.native.structural;
    let reach = |from: usize| {
        let mut seen = vec![false; words.len()];
        let mut work = vec![from];
        while let Some(w) = work.pop() {
            if w < words.len() && !std::mem::replace(&mut seen[w], true) {
                work.extend(plain_succ(x, w));
            }
        }
        seen
    };
    let document = reach(0);
    let mut rules = vec![false; words.len()];
    for &(pc, _) in &one.sites {
        for (w, r) in reach(crate::native_cpu_stream::callee(x.native, pc)).into_iter().enumerate() {
            rules[w] |= r;
        }
    }
    let mut root = None;
    for &(pc, k) in &one.sites {
        if document[pc] && rules[pc] {
            return None;
        }
        if document[pc] {
            if root.is_some_and(|r| r != k) {
                return None;
            }
            root = Some(k);
        }
    }
    let r = root?;
    one.sites.iter().all(|&(pc, k)| document[pc] || k != r).then_some(r)
}

/// The facts of a program whose tier 2 makes the records (`Records::fetched_in_tier2`).
struct Fetched {
    ft: Box<crate::native_cpu_stream::Fetch>,
    /// The region tier 2 splits, read as `parse_string` (`native_cpu_stream::Seq`), and its
    /// part's first byte.
    seq: Option<(crate::native_cpu_stream::Seq, u8)>,
    /// Each word reached with no part pending, and with one (`parts_pending_seen`).
    seen: Vec<[bool; 2]>,
}

impl Fetched {
    /// A `TOKEN` where a part of the region is always pending: the string's next part.
    fn part_at(&self, x: &Facts<'_>, pc: usize) -> bool {
        matches!(x.native.structural[pc].operation, Op::Token { .. }) && self.seen[pc] == [false, true]
    }
}

/// Where every node's key is known at its word (`Facts::coded`) and no leaf is deferred or an extra: each node word's
/// key bits from the words after it (`key_after`: its field, its supertypes' classes), by its place. A format 16 run
/// of such a program passes each node to the writer's ops at its word, with no row (`native_cpu_stream`'s, as
/// `rust_source_16`'s). `None` elsewhere.
pub(crate) fn direct_keys(native: &Structure) -> Option<Vec<(u32, u32)>> {
    let x = facts(native, true).ok()?;
    let plain = native.extras.is_empty() && native.readings.provisional.is_empty() && !native.structural.iter().any(|w| matches!(w.operation, Op::Leaf { deferred: true, .. } | Op::Leaf { extra: true, .. }));
    (x.coded && plain).then(|| (0..native.structural.len()).map(|pc| key_after(&x, pc)).collect())
}

/// The most nodes a direct run of the program holds open (`f16::Writer::reserve_run`'s depth): a rule holds at most
/// `OneFn::most` at a word, a call nests at most `native_cpu_stream::DEPTH` deep and keeps its container open. With
/// the stack reserved so up front, a direct run checks its room once, where it starts, as the lean parser sizes its
/// stacks once (`DEFAULT_MAX_DEPTH`), and no node checks it. `None` where the program is not one a direct run takes.
pub(crate) fn open_bound(native: &Structure) -> Option<u64> {
    open_bound_of(&facts(native, true).ok()?)
}

fn open_bound_of(x: &Facts<'_>) -> Option<u64> {
    let one = x.one.as_ref().filter(|o| o.stacked && x.coded && x.sixteen)?;
    let depth = crate::native_cpu_stream::DEPTH as u64;
    Some((depth + 1) * one.most.max(0) as u64 + depth)
}

/// The program's facts (`Facts`), or why the Rust backend does not take it.
fn facts(native: &Structure, sixteen: bool) -> Result<Facts<'_>, String> {
    if !crate::native_cpu_stream::calls_on_stack(native) {
        return Err("calls are not on the machine's stack (`calls_on_stack`)".into());
    }
    let fetch = fetched(native)?;
    let late = native.block_programs.iter().filter_map(|b| b.records.as_ref()).any(|r| r.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2));
    if late && fetch.as_ref().is_none_or(|f| f.seq.is_none()) {
        return Err("a region tier 2 splits (decision 0323), not read as `parse_string`: not yet ahead of time".into());
    }
    let classes = crate::native_cpu_stream::supertype_classes(native).map_err(|e| format!("supertype classes: {e:?}"))?;
    let records = native.block_programs.first().and_then(|b| b.records.as_ref());
    let starts = native.structural.iter().any(|b| matches!(b.operation, Op::Token { .. }));
    let in_code = records.map_or(0, |r| r.marks.len());
    let lazy: Vec<(u8, u16)> = native.block_programs.first().and_then(|b| b.records.as_ref().map(|r| (b, r))).map_or(Vec::new(), |(b, r)| {
        let bw = crate::native_cpu_bytes::Bytewise::of(b);
        crate::native_cpu_block::lazy_marks(r, &|o| bw.can(o))
    });
    let kind_reps: Option<Vec<u8>> = records.filter(|_| starts).map(|r| {
        let n = r.kinds.iter().copied().max().map_or(0, |m| usize::from(m) + 1);
        (0..n).map(|k| r.kinds.iter().position(|&x| usize::from(x) == k).unwrap_or(0) as u8).collect()
    });
    let end_kind = kind_reps.as_ref().and_then(|r| u8::try_from(r.len()).ok());
    let word_kind_of: Vec<u8> = records.filter(|_| starts).map_or_else(Vec::new, |r| r.words.iter().map(|w| r.kinds[usize::from(w[0])]).collect());
    let words = &native.structural;
    let context = words.iter().any(|b| matches!(b.operation, Op::Remember { .. } | Op::Flag { .. } | Op::Push { flag: Some(_), .. }));
    let mut entries: BTreeSet<usize> = BTreeSet::from([0]);
    for (pc, b) in words.iter().enumerate() {
        if let Op::Push { .. } = b.operation {
            entries.insert(crate::native_cpu_stream::callee(native, pc));
        }
    }
    let phase = crate::native_cpu_stream::phases(native, &crate::native_cpu_stream::push_targets(native));
    let flagged = phase.contains(&(crate::stream::TAKEN | crate::stream::UNTAKEN));
    if fetch.is_some() && (flagged || !native.extras.is_empty()) {
        return Err("records made in tier 2, with a token's taking flagged or extras: not yet ahead of time".into());
    }
    let unchecked = crate::native_cpu_stream::bounds(native).unchecked(crate::native_cpu_stream::marks_of(words));
    let mut facts = Facts { native, in_code, lazy, kind_reps, end_kind, word_kind_of, context, entries: entries.into_iter().collect(), phase, flagged, fetch, classes, unchecked, one: None, sentinel: false, coded: false, sixteen, numbers: Vec::new(), strings: HashMap::new(), part_arg: false, methods: Default::default(), ends: None, reserved: false, empties: HashMap::new(), closers: HashMap::new() };
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
        for (pc, w) in words.iter().enumerate() {
            if matches!(w.operation, Op::Token { .. }) && fx.seen[pc] == [true, true] {
                return Err(format!("word {pc}: a `TOKEN` where a part may be pending or not"));
            }
            if fx.part_at(&facts, pc) {
                part_facts(&facts, fx, pc)?;
            }
        }
    }
    Ok(facts)
}

/// Whether every `CLOSE` closes an open node, by the program's shape: one function with its stack (`OneFn::stacked`),
/// where each close is its open's (the `SURE` of `S::close`). With the room reserved (`reserved`), a close then cannot
/// fail, as the lean parser's `close`.
fn sure_closes(x: &Facts<'_>) -> bool {
    x.reserved && x.one.as_ref().is_some_and(|o| o.stacked)
}

fn mask_output(n: &Structure, mask: u16) -> Result<usize, String> {
    n.block_programs.first().and_then(|b| b.masks.get(usize::from(mask)).copied()).ok_or_else(|| format!("mask {mask}: no output"))
}

fn token_name(any: Option<u8>, extras: bool) -> String {
    format!("token_{}{}", any.map_or("all".to_string(), |k| format!("m{k}")), if extras { "_x" } else { "" })
}

/// A local check's word form: the byte sets' classes, tier 1's form (a class a bit of a byte, by two
/// nibble tables, `lo[b & 15] & hi[b >> 4]`, eight classes a pair of tables), each set's table
/// and class bits.
pub(crate) struct WordForm {
    pub tables: Vec<([u8; 16], [u8; 16])>,
    pub class_of: HashMap<usize, (usize, u8)>,
}

pub(crate) fn word_form(ops: &[LocalOp]) -> Result<WordForm, String> {
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
    Ok(WordForm { tables, class_of })
}

/// A word's successors as its operation names them (`succ` before the part's `TOKEN` and a
/// switch's sending its shared bytes on): its next, its targets, a call's two returns.
fn plain_succ(x: &Facts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    let v = match x.native.structural[pc].operation {
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => x.native.switches[usize::from(table)].all_targets().filter(|&t| t != REFUSE).map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } => vec![pc + 1, pc + 2],
        // In one function (`OneFn`), only the places its rule's returns go back to.
        Op::Push { .. } if x.one.is_some() => x.one.as_ref().and_then(|o| o.after.get(&pc)).cloned().unwrap_or_default(),
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}

/// A word's successors in its function: its next, its targets, a call's two returns.
fn succ(x: &Facts<'_>, pc: usize) -> Vec<usize> {
    let n = x.native.structural.len();
    if let Some(fx) = x.fetch.as_ref().filter(|fx| fx.part_at(x, pc)) {
        // A region read by its method (`string_methods`): on at the closer's place alone.
        if x.strings.contains_key(&pc) {
            return vec![usize::from(past_span(x, part_facts(x, fx, pc).expect("checked").closer, 1))];
        }
        return part_plan(x, fx, pc).refs.into_iter().map(usize::from).filter(|&t| t < n).collect();
    }
    if let Some(one) = &x.one {
        match x.native.structural[pc].operation {
            Op::Push { .. } => return vec![crate::native_cpu_stream::callee(x.native, pc)],
            Op::Return { .. } => return one.returns.get(&pc).map_or(Vec::new(), |r| r.iter().map(|&(_, t)| t).collect()),
            _ => {}
        }
    }
    let v = match x.native.structural[pc].operation {
        // An empty container's leaf (`empties`): on at its call's return place too.
        Op::Open { .. } if x.empties.contains_key(&pc) => vec![if field_after(x, pc).is_some() { pc + 2 } else { pc + 1 }, x.empties[&pc].after],
        // A node's field set by the next word, stored with its kind: on past that word.
        Op::Open { .. } | Op::Leaf { .. } if field_after(x, pc).is_some() => vec![pc + 2],
        Op::Goto { target } => vec![usize::from(target)],
        Op::Switch { table } => switch_plan(x, &x.native.switches[usize::from(table)]).refs.into_iter().map(usize::from).collect(),
        Op::Word { test: true, .. } | Op::Mark { .. } | Op::Row { .. } | Op::Flag { op: FlagOp::Test, .. } => vec![pc + 1, pc + 2],
        Op::Push { target, .. } => vec![usize::from(target), usize::from(target) + 1],
        Op::Return { .. } | Op::Accept | Op::Stop => vec![],
        _ => vec![pc + 1],
    };
    v.into_iter().filter(|&t| t < n).collect()
}

/// Whether rows carry marks (`head`): where a field is set on a wrap's inner node, or a reread
/// may meet a dropped row's place (`native_cpu_stream::TableFns` keeps the same).
pub(crate) fn marks(n: &Structure) -> bool {
    n.structural.iter().any(|b| matches!(b.operation, Op::NodeField { on: On::Inner, .. } | Op::Reread { .. }))
}

/// The field the word after a node's sets on it (`NODE_FIELD` on the node just made), stored
/// with the node's kind: one 32-bit store of both, as the JIT's (`with_field`).
fn field_after(x: &Facts<'_>, pc: usize) -> Option<u16> {
    match x.native.structural.get(pc + 1).map(|w| w.operation) {
        Some(Op::NodeField { field, on: On::Here }) => Some(field),
        _ => None,
    }
}

/// Whether a switch is on the record's kind (`records`): at offset 0, with records.
fn on_kinds(x: &Facts<'_>, sw: &ByteSwitch) -> bool {
    x.kind_reps.is_some() && sw.offset == 0
}

/// A switch's code with its branches as marks (`\u{1}k\u{1}`, its `k`th reference), and the
/// place each reference goes to: a word's successors, one an arm, so a place two arms share is
/// a merge of the graph, written once.
#[derive(Default)]
struct Plan {
    code: String,
    refs: Vec<u16>,
}

impl Plan {
    fn to(&mut self, t: u16) -> String {
        if t == REFUSE {
            return "return refuse(why::BYTE);".into();
        }
        self.refs.push(t);
        format!("\u{1}{}\u{1}", self.refs.len() - 1)
    }

    /// The code, each mark its branch's code.
    fn fill(&self, jump: &dyn Fn(usize) -> Result<String, String>) -> Result<String, String> {
        let mut out = String::with_capacity(self.code.len());
        for (i, part) in self.code.split('\u{1}').enumerate() {
            if i % 2 == 0 {
                out.push_str(part);
            } else {
                out.push_str(&jump(part.parse::<usize>().map_err(|e| e.to_string())?)?);
            }
        }
        Ok(out)
    }
}

/// A `match` over `values` (each value's place), `0..len`: the runs of a place one arm, the
/// place with the most runs the default where every value has one; else a value with no
/// place refuses.
fn arms(p: &mut Plan, scrutinee: &str, values: &[u16]) -> String {
    let mut by: BTreeMap<u16, Vec<(usize, usize)>> = BTreeMap::new();
    let mut i = 0;
    while i < values.len() {
        let mut j = i;
        while j + 1 < values.len() && values[j + 1] == values[i] {
            j += 1;
        }
        by.entry(values[i]).or_default().push((i, j));
        i = j + 1;
    }
    let mut s = format!("match {scrutinee} {{\n");
    let dflt = if by.contains_key(&REFUSE) { None } else { by.iter().max_by_key(|(_, r)| r.len()).map(|(&t, _)| t) };
    for (&t, runs) in &by {
        if t == REFUSE || Some(t) == dflt {
            continue;
        }
        let pats: Vec<String> = runs.iter().map(|&(a, b)| if a == b { format!("{a}") } else { format!("{a}..={b}") }).collect();
        let j = p.to(t);
        writeln!(s, "{} => {{\n{j}\n}}", pats.join(" | ")).unwrap();
    }
    match dflt {
        Some(t) => {
            let j = p.to(t);
            writeln!(s, "_ => {{\n{j}\n}}").unwrap();
        }
        None => s.push_str("_ => return refuse(why::BYTE),\n"),
    }
    s.push_str("}\n");
    s
}

/// A switch's `match`: on the record's kind (a kind whose words the switch names, on its word
/// within its arm), or on a byte of the token.
fn switch_plan(x: &Facts<'_>, sw: &ByteSwitch) -> Plan {
    let mut p = Plan::default();
    // Where tier 2 makes the records, a token not made yet takes the switch on its first byte,
    // as simdjson's `switch (*value)`: each arm makes the token's record (its last byte, its
    // code) for its bytes, then goes on. A made one (a part, the stream's end, one switched on
    // again) takes the switch on its kind.
    let by_byte = match (&x.fetch, on_kinds(x, sw)) {
        (Some(fx), true) => Some(fetched_arms(x, fx, sw, &mut p)),
        _ => None,
    };
    // Reached only from the `TOKEN` before it, which takes a token: the token is made here, or
    // the stream has ended; no switch on its kind, as simdjson switches on a value once.
    if let Some(b) = &by_byte
        && fresh(x, sw)
    {
        // The end as a start refused by the byte switch (its byte past the file, 0, has no
        // arm): no test of the end, as simdjson's switch takes the index past the last.
        let refuses = sw.end == REFUSE || x.native.structural.get(usize::from(sw.end)).is_some_and(|w| matches!(w.operation, Op::Stop));
        if x.sentinel && refuses && x.fetch.as_ref().is_some_and(|fx| zero_refused(x, fx, sw)) {
            p.code = b.clone();
            return p;
        }
        let j = p.to(sw.end);
        // Every token taken: the end by the stream's count, the token before not taken (`counted_end`).
        let counted = x.native.switches.iter().position(|o| std::ptr::eq(o, sw)).is_some_and(|k| counted_end(x, k as u16));
        let end = if counted { "(t.next as usize == s.n)".to_string() } else { x.at_end("t", "s") };
        p.code = format!("if {end} {{\n{j}\n}} else {{\n{b}}}\n");
        return p;
    }
    if let (true, Some(reps)) = (on_kinds(x, sw), &x.kind_reps) {
        let mut t: Vec<u16> = reps.iter().map(|&b| sw.targets[usize::from(b)]).collect();
        if x.end_kind.is_some() {
            t.push(sw.end);
        }
        let head = if x.end_kind.is_none() {
            let j = p.to(sw.end);
            format!("if {} {{\n{j}\n}} else ", x.at_end("t", "s"))
        } else {
            String::new()
        };
        let mut by_word = String::new();
        let mut values = vec![REFUSE; 256];
        for (k, &tk) in t.iter().enumerate() {
            let named: Vec<(u16, u16)> = sw.words.iter().copied().filter(|&(w, _)| k < reps.len() && x.word_kind_of.get(usize::from(w)).is_some_and(|&y| usize::from(y) == k)).collect();
            if named.is_empty() {
                values[k] = tk;
                continue;
            }
            let mut inner = String::from("match t.word() {\n");
            for (w, to_w) in named {
                let j = p.to(to_w);
                writeln!(inner, "{} => {{ {j} }}", u32::from(w) + 1).unwrap();
            }
            let j = p.to(tk);
            writeln!(inner, "_ => {{ {j} }}\n}}").unwrap();
            writeln!(by_word, "{k} => {{ {inner} }}").unwrap();
        }
        let rest = arms(&mut p, "kind", &values);
        let body = if by_word.is_empty() { rest } else { format!("match kind {{\n{by_word}_ => {{ {rest} }}\n}}\n") };
        // The end's kind: the record's, or (the end as a start) the end's own.
        let kind = match x.end_kind {
            Some(k) if x.sentinel => format!("if {} {{ {k} }} else {{ t.code & 0xff }}", x.at_end("t", "s")),
            _ => "t.code & 0xff".into(),
        };
        let by_kind = format!("{head}{{ let kind = {kind};\n{body}}}");
        p.code = match by_byte {
            Some(b) => format!("if t.rec_end == 0 && !{} {{\n{b}}} else {{\n{by_kind}}}\n", x.at_end("t", "s")),
            None => by_kind,
        };
        return p;
    }
    let j = p.to(sw.end);
    let mut out = format!("if {} {{\n{j}\n}} else {{\n", x.at_end("t", "s"));
    if sw.offset == 0 && !sw.words.is_empty() {
        out.push_str("match t.word() {\n");
        for &(w, t) in &sw.words {
            let j = p.to(t);
            writeln!(out, "{} => {{ {j} }}", u32::from(w) + 1).unwrap();
        }
        out.push_str("_ => {}\n}\n");
    }
    writeln!(out, "let at = t.start + {};\nif at >= t.rec_end {{ return refuse(why::SPAN); }}", sw.offset).unwrap();
    out.push_str(&arms(&mut p, "unsafe { *s.bytes.add(at as usize) }", &sw.targets));
    out.push_str("}\n");
    p.code = out;
    p
}

/// A word's code, `jump(k)` its `k`th successor's.
fn word(x: &Facts<'_>, pc: usize, jump: &dyn Fn(usize) -> Result<String, String>) -> Result<String, String> {
    word_with(x, pc, jump, None)
}

/// `word`, the token's code known (`known`) where it is a constant: a keyword's leaf its kind,
/// as simdjson's `case 't'` writes its own tape word.
fn word_with(x: &Facts<'_>, pc: usize, jump: &dyn Fn(usize) -> Result<String, String>, known: Option<u32>) -> Result<String, String> {
    let n = x.native;
    let kind_of = |k: u16| -> String { if k == REMEMBERED { "{ if word == 0 { return refuse(why::WORD) } word as u16 }".into() } else { format!("{k}") } };
    // The token's end, as this word reads it: its phase's (`phases`), or the run's field.
    let taken = x.phase[pc] == crate::stream::TAKEN;
    let end = if x.flagged {
        "t.end"
    } else if taken {
        "t.rec_end"
    } else if x.sentinel {
        "(if t.start == s.len { t.rec_end } else { t.start })"
    } else {
        "(if t.at_end { t.rec_end } else { t.start })"
    };
    if let Some(name) = x.strings.get(&pc) {
        let at = if x.part_arg { "t.start + 1, " } else { "" };
        return Ok(format!("s.{name}({at}t)?;\n{}", jump(0)?));
    }
    if let Some(fx) = x.fetch.as_ref().filter(|fx| fx.part_at(x, pc)) {
        return part_plan(x, fx, pc).fill(jump);
    }
    if let Some(one) = &x.one {
        match n.structural[pc].operation {
            // A call: its site on the stack, then the rule's first word (`object_begin`'s
            // `depth++`).
            Op::Push { .. } => {
                let k = one.sites.iter().find(|&&(p, _)| p == pc).expect("a site").1;
                if one.stacked {
                    // In its container's row, until the `CLOSE` before the return reads it; none for the document's
                    // own calls, whose return the depth tells (`root_number`).
                    let keep = if root_number(x) == Some(k) { String::new() } else { format!("s.keep_ret({k});\n") };
                    return Ok(format!("if s.depth == DEPTH {{ return refuse(why::DEPTH); }}\ns.depth += 1;\n{keep}{}", jump(0)?));
                }
                return Ok(format!("if s.depth == DEPTH {{ return refuse(why::DEPTH); }}\ns.ret[s.depth as usize] = {k};\ns.depth += 1;\n{}", jump(0)?));
            }
            // A return: the site off the stack, then its place (`scope_end`).
            Op::Return { .. } => {
                let arms = one.returns.get(&pc).map_or(&[][..], Vec::as_slice);
                if one.stacked {
                    // As the lean parser's `scope_end`: the document's own call's return where the depth is 0
                    // (`root_number`), then the container's: a test of its number where there are two.
                    let root = root_number(x).and_then(|r| arms.iter().position(|&(k, _)| k == r));
                    let rest: Vec<(usize, usize)> = arms.iter().enumerate().filter(|&(i, _)| Some(i) != root).map(|(i, &(k, _))| (i, k)).collect();
                    let inner = match rest.as_slice() {
                        [] => None,
                        [(i, _)] => Some(jump(*i)?),
                        [(i, k), (j, _)] => Some(format!("if ret == {k} {{\n{}\n}} else {{\n{}\n}}", jump(*i)?, jump(*j)?)),
                        _ => None,
                    };
                    if rest.len() <= 2 && (root.is_some() || !rest.is_empty()) {
                        return Ok(match (root, inner) {
                            (Some(r), None) => format!("s.depth -= 1;\n{}\n", jump(r)?),
                            (Some(r), Some(c)) => format!("s.depth -= 1;\nif s.depth == 0 {{\n{}\n}} else {{\n{c}\n}}\n", jump(r)?),
                            (None, Some(c)) => format!("s.depth -= 1;\n{c}\n"),
                            (None, None) => unreachable!(),
                        });
                    }
                }
                let mut m = String::from(if one.stacked { "s.depth -= 1;\nmatch ret {\n" } else { "s.depth -= 1;\nmatch s.ret[s.depth as usize] {\n" });
                for (i, &(k, _)) in arms.iter().enumerate() {
                    writeln!(m, "{k} => {{ {} }}", jump(i)?).unwrap();
                }
                // Every open call's site is one of these (`OneFn`).
                m.push_str("_ => return refuse(why::RETURN),\n}\n");
                return Ok(m);
            }
            _ => {}
        }
    }
    Ok(match n.structural[pc].operation {
        // Before a switch that takes the end alone (`counted_end`): no token taken, the end the stream's count.
        Op::Token { any: None, extras: false, .. } if taken && matches!(n.structural.get(pc + 1).map(|w| w.operation), Some(Op::Switch { table }) if counted_end(x, table)) => jump(0)?,
        Op::Token { any, extras, .. } if x.flagged || taken => format!("s.{}(t)?;\n{}", token_name(any, extras), jump(0)?),
        // No token to take here: the next word switches on the one untaken.
        Op::Token { .. } => format!("{}{}", if x.unchecked { "" } else { "s.room()?;\n" }, jump(0)?),
        Op::Switch { table } => switch_plan(x, &n.switches[usize::from(table)]).fill(jump)?,
        Op::Span { last: false, len } => format!("if t.start + {len} != t.rec_end {{ return refuse(why::SPAN); }}\n{}{}", if x.flagged { "t.end = t.rec_end;\nt.need = true;\n" } else { "" }, jump(0)?),
        Op::Span { last: true, .. } => format!("{}{}", if x.flagged { "t.end = t.rec_end;\nt.need = true;\n" } else { "" }, jump(0)?),
        Op::Empty { total } => format!("if s.total({total}) != 0 {{ return refuse(why::EMPTY); }}\n{}", jump(0)?),
        Op::Word { string, test: true } => format!("if t.word() == {} {{\n{}\n}} else {{\n{}\n}}", u32::from(string) + 1, jump(0)?, jump(1)?),
        Op::Word { string, test: false } => format!("if t.word() != {} {{ return refuse(why::WORD); }}\n{}", u32::from(string) + 1, jump(0)?),
        Op::Open { kind } => {
            let kf = u32::from(kind) | u32::from(field_after(x, pc).unwrap_or(0)) << 16;
            let code = if x.coded {
                let (field, classes) = key_after(x, pc);
                let kf = if field != 0 { kf & 0xffff | field } else { kf };
                if x.sixteen && x.coded {
                    // The node is the ops' alone: no row, its container's slot on the stack. Its number the
                    // language's, a literal; a key the table lacks refuses the file (the JIT's `fail::NUMBER`).
                    let Some(number) = x.number(kf, classes) else { return Ok(UNNUMBERED.into()) };
                    // The document's container: its closer the stream's last token, checked before the walk, as the lean
                    // parser's `last != '}'` (`last_closers`).
                    let first = match x.closers.get(&pc) {
                        Some(b) => format!("if s.n == 0 || s.byte(unsafe {{ *s.starts.add((s.n - 1) * s.rs) }}) != {b:#04x} {{ return refuse(why::BYTE); }}\n"),
                        None => String::new(),
                    };
                    // Empty, its closer the next token (`empties`): one leaf, the closer taken as its call's switch
                    // takes it (the depth counted, as the call's; its end kept where that arm keeps it), then on at
                    // the call's return place.
                    if let Some(e) = x.empties.get(&pc) {
                        let keep = x.ends.as_ref().is_some_and(|en| en.copies.contains(&e.table)) && end_read_from(x, e.q, true);
                        return Ok(format!(
                            "{first}if s.byte(unsafe {{ *s.starts.add(t.next as usize * s.rs) }}) == {:#04x} {{\nif s.depth == DEPTH {{ return refuse(why::DEPTH); }}\nlet st = t.start;\ns.{}(t)?;\n{}t.rec_end = t.start + 1;\ns.leaf16({number}, st, t.rec_end){};\n{}\n}} else {{\ns.open16({number}, t.start){};\n{}\n}}",
                            e.byte,
                            token_name(None, false),
                            if keep { "t.last_end = t.rec_end;\n" } else { "" },
                            x.may_fail(),
                            jump(1)?,
                            x.may_fail(),
                            jump(0)?
                        ));
                    }
                    return Ok(format!("{first}s.open16({number}, t.start){};\n{}", x.may_fail(), jump(0)?));
                }
                format!("s.code_now({kf}, {classes:#x});\n")
            } else {
                String::new()
            };
            format!("s.open({kf}, t.start);\n{code}{}", jump(0)?)
        }
        Op::Close { this } => {
            let sure = x.one.as_ref().is_some_and(|o| o.stacked);
            // The `CLOSE` before a `RETURN`: the call's number from its container's row first.
            let before = plain_succ(x, pc).iter().any(|&t| matches!(n.structural[t].operation, Op::Return { .. }));
            let take = if sure && before { "ret = s.taken_ret();\n" } else { "" };
            // The last token's end where `last_ends` finds it.
            let last = if x.ends.as_ref().is_some_and(|e| e.rec.contains(&pc)) { "t.rec_end" } else { "t.last_end" };
            let may = if sure_closes(x) && x.sixteen && x.coded { "" } else { "?" };
            format!("{take}s.close::<{this}, {sure}>({}){may};\n{}", if this { end } else { last }, jump(0)?)
        }
        Op::Leaf { kind, deferred, extra } => {
            let f = u32::from(field_after(x, pc).unwrap_or(0)) << 16;
            let constant = known.filter(|_| kind == WORD_KIND).map(|c| n.word_kinds.iter().find(|&&(w, _)| u32::from(w) + 1 == (c >> 8) & 0xff).map(|&(_, k)| u32::from(k)));
            let k = match (kind == WORD_KIND, f) {
                (true, _) if constant == Some(None) => return Ok("return refuse(why::WORD);".into()),
                (true, f) if constant.is_some() => format!("{}", constant.flatten().unwrap_or(0) | f),
                (true, 0) => "s.word_kind(t.word())?".to_string(),
                (true, f) => format!("s.word_kind(t.word())? | {f}"),
                (false, f) => format!("{}", u32::from(kind) | f),
            };
            if x.coded {
                let (field, classes) = key_after(x, pc);
                let kf = if field != 0 { format!("kf & 0xffff | {field}") } else { "kf".into() };
                if x.sixteen && x.coded {
                    if deferred || extra {
                        return Err(format!("word {pc}: a deferred or extra leaf in a format 16 run"));
                    }
                    // The leaf's number the language's, a literal: its kind's, or a keyword's by its token's word, as
                    // the lean lexer's keyword `match` gives its kind.
                    let key = |kind: u32| if field != 0 { (kind | f) & 0xffff | field } else { kind | f };
                    if kind == WORD_KIND && constant.is_none() {
                        let mut m = String::from("let number = match t.word() {\n");
                        for &(w, k) in &n.word_kinds {
                            if let Some(number) = x.number(key(u32::from(k)), classes) {
                                writeln!(m, "{} => {number},", u32::from(w) + 1).unwrap();
                            }
                        }
                        m.push_str("_ => return refuse(why::WORD),\n};\n");
                        return Ok(format!("{m}s.leaf16(number, t.start, {end}){};\n{}", x.may_fail(), jump(0)?));
                    }
                    let kind = if kind == WORD_KIND { constant.flatten().unwrap_or(0) } else { u32::from(kind) };
                    let Some(number) = x.number(key(kind), classes) else { return Ok(UNNUMBERED.into()) };
                    return Ok(format!("s.leaf16({number}, t.start, {end}){};\n{}", x.may_fail(), jump(0)?));
                }
                let now = format!("s.code_now({kf}, {classes:#x});");
                return Ok(format!("let kf = {k};\ns.leaf::<{deferred}, {extra}>(kf, t.start, {end}, t.last_end);\n{now}\n{}", jump(0)?));
            }
            format!("s.leaf::<{deferred}, {extra}>({k}, t.start, {end}, t.last_end);\n{}", jump(0)?)
        }
        Op::Settle { .. } => format!("s.pending = 0;\n{}", jump(0)?),
        Op::SettleBefore { mark } => {
            // The token's record: the pending extras are the records just before it (4784e6db9c, 0451 item 4).
            let token = if x.sentinel { "t.next as usize - 1".to_owned() } else { format!("t.next as usize - usize::from(!{})", x.at_end("t", "s")) };
            format!("s.settle_before({mark}, {token});\n{}", jump(0)?)
        }
        Op::Mark { mask, order, end, keep, word: mark_word, pattern } => {
            let k = u32::from(order);
            let here = if (k as usize) < x.in_code {
                // In the record, folded into `live` at the token (the end's marks too).
                match pattern {
                    Some(i) => format!("(t.at_end || s.pattern_{i}(t.next))"),
                    None if x.lazy.iter().any(|&(m, _)| u32::from(m) == k) => format!("(t.at_end || s.lazy_{k}(t))"),
                    None => "true".into(),
                }
            } else {
                if pattern.is_some() {
                    return Err("a token mark not in the records".into());
                }
                let o = mask_output(n, mask)?;
                let w = mark_word.map_or(String::new(), |s| format!("t.word() == {} && ", u32::from(s) + 1));
                format!("(if t.at_end {{ {end} }} else {{ {w}s.bit({o}, t.start) }})")
            };
            let take = if keep { String::new() } else { format!("t.live &= !((2u64 << {k}) - 1);\n") };
            format!("if t.live >> {k} & 1 != 0 && {here} {{\n{take}{}\n}} else {{\n{}\n}}", jump(0)?, jump(1)?)
        }
        Op::Row { mask } => format!("if s.bit({}, t.start) {{\n{}\n}} else {{\n{}\n}}", mask_output(n, mask)?, jump(0)?, jump(1)?),
        Op::Retag { kind, on: On::Here } => format!("s.here_kind({kind})?;\n{}", jump(0)?),
        Op::Retag { kind, on: On::Moved } => format!("s.moved_kind({});\n{}", kind_of(kind), jump(0)?),
        Op::Retag { kind, on: On::Made } => format!("s.made_kind::<{}>({})?;\n{}", kind == DEAD, kind_of(kind), jump(0)?),
        Op::Reread { reading } => format!("s.reread_{reading}({})?;\n{}", if x.context { "word" } else { "0" }, jump(0)?),
        Op::Wrap { kind } => format!("s.wrap({kind})?;\n{}", jump(0)?),
        Op::Merge { kind, fold: true } => format!("s.fold({kind})?;\n{}", jump(0)?),
        Op::Merge { kind, fold: false } => format!("s.merge({kind}, {end})?;\n{}", jump(0)?),
        // In a format 16 run the node's field and supertypes are in its number, taken at its word (`key_after`).
        Op::NodeField { on: On::Here, .. } | Op::NodeSuper { .. } if x.sixteen && x.coded => jump(0)?,
        Op::NodeField { field, on: On::Here } => format!("unsafe {{ (*s.row(s.nodes - 1)).field = {field} }};\n{}", jump(0)?),
        Op::NodeField { field, on: On::Moved } => format!("unsafe {{ (*s.row(s.moved)).field = {field} }};\n{}", jump(0)?),
        Op::NodeField { field, on: On::Made } => format!("s.made_field::<false>({field})?;\n{}", jump(0)?),
        Op::NodeField { field, on: On::MadeUnset } => format!("s.made_field::<true>({field})?;\n{}", jump(0)?),
        Op::NodeField { field, on: On::Inner } => format!("s.inner_field({field})?;\n{}", jump(0)?),
        Op::NodeSuper { kind } => format!("s.node_super({kind});\n{}", jump(0)?),
        Op::Push { flag, .. } => {
            let callee = crate::native_cpu_stream::callee(n, pc);
            let args = if x.context {
                let f = match flag {
                    Some((k, true)) => format!("flags | {}", 1u32 << k),
                    Some((k, false)) => format!("flags & !{}", 1u32 << k),
                    None => "flags".into(),
                };
                format!(", word, {f}")
            } else {
                String::new()
            };
            format!("if s.depth == DEPTH {{ return refuse(why::DEPTH); }}\ns.depth += 1;\nlet again = r{callee}(s, t{args})?;\ns.depth -= 1;\nif again {{\n{}\n}} else {{\n{}\n}}", jump(1)?, jump(0)?)
        }
        Op::Remember { kind: WORD_KIND } => format!("word = s.word_kind(t.word())?;\n{}", jump(0)?),
        Op::Remember { kind } => format!("word = {kind};\n{}", jump(0)?),
        Op::Flag { flag, op: FlagOp::Set } => format!("flags |= {};\n{}", 1u32 << flag, jump(0)?),
        Op::Flag { flag, op: FlagOp::Clear } => format!("flags &= !{};\n{}", 1u32 << flag, jump(0)?),
        Op::Flag { flag, op: FlagOp::Test } => format!("if flags & {} != 0 {{\n{}\n}} else {{\n{}\n}}", 1u32 << flag, jump(0)?, jump(1)?),
        Op::Count { count, up: true, max } => format!("s.counts[{count}] += 1;\nif s.counts[{count}] > {max} {{ return refuse(why::COUNT); }}\n{}", jump(0)?),
        Op::Count { count, up: false, .. } => format!("s.counts[{count}] = s.counts[{count}].saturating_sub(1);\n{}", jump(0)?),
        Op::Check { check } => format!("s.check_{check}(t)?;\n{}", jump(0)?),
        Op::Return { again } => format!("return Ok({again});"),
        Op::Accept => "s.accept(t)?;\nreturn Ok(false);".into(),
        Op::Stop => "return refuse(why::STOP);".into(),
        Op::Goto { .. } => jump(0)?,
        ref op => return Err(format!("word {pc}: {op:?} is not tier 2's")),
    })
}

/// Where tier 2 makes the records (`Records::fetched_in_tier2`): its facts; refused where a
/// first byte's rule is one this backend does not write (a pair of two bytes, a last-byte mask,
/// a region not read as `parse_string`, a run whose output is not a byte class), or a word
/// whose first byte begins a run is not one simdjson's atom test reads (eight bytes or fewer,
/// each of the run's class).
fn fetched(native: &Structure) -> Result<Option<Fetched>, String> {
    use crate::records::EndRule;
    let Some(ft) = crate::native_cpu_stream::fetch_of(native).map_err(|e| format!("records made in tier 2: {e:?}"))? else { return Ok(None) };
    let split = crate::native_cpu_stream::split_parts(native).map_err(|e| format!("a region tier 2 splits: {e:?}"))?;
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
            Some(r) if crate::native_cpu_stream::one_byte(&ft, r) => {}
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
    let seen = crate::native_cpu_stream::parts_pending_seen(native, &crate::native_cpu_stream::push_targets(native), &ft);
    Ok(Some(Fetched { ft, seq, seen }))
}

/// A token's place by the switch's kind path (`switch_plan`): its kind's (its byte's first,
/// `kind_reps`), or its word's where the switch names it.
fn kind_target(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, code: u32) -> u16 {
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

/// The arms of a switch on a token's first byte, where tier 2 makes the records: bytes alike in
/// rule, code and place one arm, each making the token's record for its bytes:
/// - a token of one byte (`{`, `,`): its code and place known, taken at once (simdjson's
///   `case ','`);
/// - a region's opener (`"`): the token is the opener alone, its parts pending after it;
/// - a run (a number's bytes): each word that begins with its bytes as simdjson's atom test
///   (`str4ncmp(value, "true")`, then the byte after not in the run's class), then the run's end.
fn fetched_arms(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, p: &mut Plan) -> String {
    fetched_arms_with(x, fx, sw, p, &mut Vec::new())
}

/// A group of a byte switch's arms (`fetched_arms`): whether it makes its token's end (`rec_end`), and the places it
/// goes to.
struct ArmGroup {
    writes: bool,
    to: Vec<u16>,
}

impl ArmGroup {
    /// Whether the group keeps the last token's end (`t.last_end`) before it makes its own, where its switch's arms
    /// do (`LastEnds::copies`): where it makes it, and a word after one of its places reads the last end.
    fn keeps(&self, x: &Facts<'_>) -> bool {
        self.writes && self.to.iter().any(|&q| end_read_from(x, usize::from(q), true))
    }
}

/// `fetched_arms`, each group of arms it writes in `groups`.
fn fetched_arms_with(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, p: &mut Plan, groups_out: &mut Vec<ArmGroup>) -> String {
    use crate::records::EndRule;
    // The last token's end kept first where the switch's writing arms come before a reader of it (`last_ends`).
    let keep = x.ends.as_ref().is_some_and(|e| x.native.switches.iter().position(|o| std::ptr::eq(o, sw)).is_some_and(|k| e.copies.contains(&k)));
    let keep = if keep { "t.last_end = t.rec_end;\n" } else { "" };
    let ft = &fx.ft;
    // A switch whose arms are another's and some of its own (an array's first value, which also
    // takes `]`, and its value after a `,`): its own arms, then the other's switch for every other
    // byte, as `walk_document` peeks for `]` before the one value switch (`delegate_of`).
    let delegate = delegate_of(x, fx, sw);
    // (form, code or rule, kind, place): 0 one byte, 1 an opener, 2 a run.
    let mut groups: BTreeMap<(u8, u32, u32, u16), Vec<usize>> = BTreeMap::new();
    for b in 0..256usize {
        let named = sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&(b as u8))));
        let key = match ft.rules.get(usize::from(ft.ends[b])) {
            Some(&EndRule::Run(_)) => (2, u32::from(ft.ends[b]), u32::from(ft.kinds[b]), kind_target(x, fx, sw, u32::from(ft.kinds[b]))),
            Some(&EndRule::Read(_)) => (1, ft.codes1[b], 0, kind_target(x, fx, sw, ft.codes1[b])),
            _ => (0, ft.codes1[b], 0, kind_target(x, fx, sw, ft.codes1[b])),
        };
        if key.3 == REFUSE && !named {
            continue;
        }
        if let Some((_, other)) = delegate
            && byte_sig(x, fx, other, b) == byte_sig(x, fx, sw, b)
        {
            continue;
        }
        groups.entry(key).or_default().push(b);
    }
    let mut s = String::from("match s.byte(t.start) {\n");
    for (&(form, a, kind, t), bytes) in &groups {
        let mut pats: Vec<String> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let mut j = i;
            while j + 1 < bytes.len() && bytes[j + 1] == bytes[j] + 1 {
                j += 1;
            }
            pats.push(if i == j { format!("{:#04x}", bytes[i]) } else { format!("{:#04x}..={:#04x}", bytes[i], bytes[j]) });
            i = j + 1;
        }
        // A one-byte token (punctuation, an opener): its place's test of that (`SPAN`) holds,
        // and is not made again (`past_span`).
        let one = past_span(x, t, 1);
        let before = p.refs.len();
        let writes = form == 2 || end_read(x, one);
        let made = if writes { "t.rec_end = t.start + 1;\n" } else { "" };
        let body = match form {
            // Its end only where a word reads it (`end_read`).
            0 => format!("{made}{}{}", code_set(x, &[one], a), p.to(one)),
            // The opener's place: passed at the method's call where `part_arg` holds, else kept for the part.
            1 => format!("{made}{}{}{}", code_set(x, &[one], a), if x.part_arg { "" } else { "t.part_at = t.start + 1;\n" }, p.to(one)),
            _ => {
                let r = a;
                let (mut tests, mut arms) = (String::new(), String::new());
                let class = ft.run_class[r as usize].as_ref().expect("checked in `fetched`");
                for (w, code) in ft.words.iter().filter(|(w, _)| w.first().is_some_and(|f| bytes.contains(&usize::from(*f))) && w.iter().all(|&y| class[usize::from(y)] == 1)) {
                    let n = w.len();
                    let mask = if n == 8 { u64::MAX } else { (1u64 << (8 * n)) - 1 };
                    let want = w.iter().rev().fold(0u64, |acc, &y| acc << 8 | u64::from(y));
                    let at = match check_at(x, kind_target(x, fx, sw, *code)) {
                        // A check of no delay at the word's place reads the word's bytes alone:
                        // decided here, as simdjson tests an atom by its bytes and nothing else.
                        Some((q, _, c)) if c.delay == 0 => {
                            if crate::program::token_value(c, w, 0, w.len(), &|_| false) {
                                REFUSE
                            } else {
                                q as u16 + 1
                            }
                        }
                        _ => kind_target(x, fx, sw, *code),
                    };
                    // Its leaf written in its arm, its kind a constant (`known_leaf`).
                    let (to, set) = match known_leaf(x, at, *code).ok().flatten() {
                        Some((code_text, _)) if code_text.ends_with(UNNUMBERED) => (code_text, String::new()),
                        Some((code_text, past)) => (format!("{code_text}{}", p.to(past)), code_set(x, &[past], *code)),
                        None => (p.to(at), code_set(x, &[at], *code)),
                    };
                    let k = arms.matches("\n// \"").count() + 1;
                    let name = format!("{:?}", String::from_utf8_lossy(w));
                    writeln!(tests, "// {name}\nif st + {n} <= s.len && s.word8(st) & {mask:#x} == {want:#x} && (st + {n} == s.len || S::CLASS_{r}[usize::from(s.byte(st + {n}))] == 0) {{\nreturn {k};\n}}").unwrap();
                    writeln!(arms, "{k} => {{\n// {name}\nt.rec_end = st + {n};\n{set}{to}\n}}").unwrap();
                }
                // The words, one method that names the one the token is (as simdjson's `is_valid_true_atom` and its
                // kin), its arms here.
                let atom = (!tests.is_empty()).then(|| {
                    x.method("atom", format!("/// Which of its run's words the token at `st` is, one load of its bytes each (as simdjson's atom tests): one more than its place, 0 for none.\n\u{0}(&self, st: u32) -> u32 {{\nlet s = self;\n{tests}0\n}}"))
                });
                let words = |rest: String| match &atom {
                    Some(m) => format!("match s.{m}(st) {{\n{arms}_ => {{\n{rest}\n}}\n}}"),
                    None => rest,
                };
                match check_at(x, t).filter(|_| t != REFUSE) {
                    // The check at its place made in the pass that finds its end, then on past it.
                    Some((q, ci, _)) => {
                        let (sp, sl) = (code_set(x, &[q as u16 + 1], kind), code_set(x, &[t], kind));
                        // The place's words on to past the check, each with one way (`straight_back`): the run, its
                        // end and its check one method, as simdjson's `parse_number`, then on past the check.
                        let check = straight_back(x, usize::from(t), q + 1).filter(|_| sp == sl).and_then(|ws| ws.iter().map(|&w| word(x, w, &|_| Ok(String::new())).ok()).collect::<Option<String>>());
                        match check {
                            Some(check) => {
                                let m = x.method("number", format!("/// The token at `st` read to its end and checked in one pass (`run_{r}_check_{ci}`), as simdjson's `parse_number`; where that pass cannot tell, its end by the run, then the check.\n\u{0}(&mut self, st: u32, t: &mut T) -> R {{\nlet s = self;\nmatch s.run_{r}_check_{ci}(st)? {{\nSome(l) => t.rec_end = l + 1,\nNone => {{\nt.rec_end = s.run_{r}(st) + 1;\n{check}}}\n}}\nOk(())\n}}"));
                                let past = p.to(q as u16 + 1);
                                format!("let st = t.start;\n{}", words(format!("s.{m}(st, t)?;\n{sp}{past}")))
                            }
                            None => {
                                let past = p.to(q as u16 + 1);
                                let long = p.to(t);
                                format!("let st = t.start;\n{}", words(format!("match s.run_{r}_check_{ci}(st)? {{\nSome(l) => {{\nt.rec_end = l + 1;\n{sp}{past}\n}}\nNone => {{\nt.rec_end = s.run_{r}(st) + 1;\n{sl}{long}\n}}\n}}")))
                            }
                        }
                    }
                    // A token no word of it is refuses: its end is not made.
                    None if t == REFUSE => format!("let st = t.start;\n{}", words("return refuse(why::BYTE);".into())),
                    None => format!("let st = t.start;\n{}", words(format!("t.rec_end = s.run_{r}(st) + 1;\n{}{}", code_set(x, &[t], kind), p.to(t)))),
                }
            }
        };
        let group = ArmGroup { writes, to: p.refs[before..].to_vec() };
        // The last token's end kept before this arm makes its own, where a word after reads it.
        let body = if group.keeps(x) { format!("{keep}{body}") } else { body };
        groups_out.push(group);
        writeln!(s, "{} => {{\n{body}\n}}", pats.join(" | ")).unwrap();
    }
    if let Some((at, _)) = delegate {
        groups_out.push(ArmGroup { writes: false, to: vec![at] });
    }
    // No byte of its own: the switch is its default alone, no `match`.
    if groups.is_empty() {
        return match delegate {
            Some((at, _)) => format!("{}\n", p.to(at)),
            None => "return refuse(why::BYTE);\n".into(),
        };
    }
    match delegate {
        Some((at, _)) => {
            let j = p.to(at);
            writeln!(s, "_ => {{\n{j}\n}}\n}}").unwrap();
        }
        None => s.push_str("_ => return refuse(why::BYTE),\n}\n"),
    }
    s
}

/// What a switch does with a token of first byte `b` (`fetched_arms`): its arm's form, code and
/// rule, and each place it can go to (a word's that begins with `b`, then the rest's). None: it
/// refuses it.
fn byte_sig(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch, b: usize) -> Option<(u8, u32, Vec<u16>)> {
    use crate::records::EndRule;
    let ft = &fx.ft;
    let named = sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&(b as u8))));
    let (form, a, t) = match ft.rules.get(usize::from(ft.ends[b])) {
        Some(&EndRule::Run(_)) => (2, u32::from(ft.ends[b]), kind_target(x, fx, sw, u32::from(ft.kinds[b]))),
        Some(&EndRule::Read(_)) => (1, ft.codes1[b], kind_target(x, fx, sw, ft.codes1[b])),
        _ => (0, ft.codes1[b], kind_target(x, fx, sw, ft.codes1[b])),
    };
    if t == REFUSE && !named {
        return None;
    }
    let mut to: Vec<u16> = ft.words.iter().filter(|(w, _)| w.first() == Some(&(b as u8))).map(|(_, code)| kind_target(x, fx, sw, *code)).collect();
    to.push(t);
    if to.iter().all(|&t| t == REFUSE) {
        return None;
    }
    Some((form, a, to))
}

/// The switch `sw` sends the bytes it shares with another to (`fetched_arms`): another switch on a
/// token's first byte in the same function, which does with each byte what `sw` does, or refuses
/// it, and refuses some `sw` takes: its place and table. The smallest such, so a chain of them
/// ends.
fn delegate_of<'a>(x: &'a Facts<'_>, fx: &Fetched, sw: &ByteSwitch) -> Option<(u16, &'a ByteSwitch)> {
    let words = &x.native.structural;
    let mine = |o: &ByteSwitch| -> Option<usize> {
        let (mut own, mut shared) = (0, 0);
        for b in 0..256usize {
            match (byte_sig(x, fx, sw, b), byte_sig(x, fx, o, b)) {
                (a, c) if a == c => shared += usize::from(a.is_some()),
                (Some(_), None) => own += 1,
                _ => return None,
            }
        }
        (own > 0 && shared > 0).then_some(shared)
    };
    let here = words.iter().position(|w| matches!(w.operation, Op::Switch { table } if std::ptr::eq(&x.native.switches[usize::from(table)], sw)))?;
    let mut best: Option<(usize, u16, &'a ByteSwitch)> = None;
    for (pc, w) in words.iter().enumerate() {
        let Op::Switch { table } = w.operation else { continue };
        let o = &x.native.switches[usize::from(table)];
        if pc == here || !on_kinds(x, o) || o.end != sw.end || !reaches(x, here, pc) {
            continue;
        }
        if let Some(shared) = mine(o)
            && best.as_ref().is_none_or(|&(s, _, _)| shared > s)
        {
            best = Some((shared, pc as u16, o));
        }
    }
    best.map(|(_, pc, o)| (pc, o))
}

/// Whether switch `sw` is reached only from the `TOKEN` before it, where that takes a token: its
/// token not made yet, or the stream's end.
fn fresh(x: &Facts<'_>, sw: &ByteSwitch) -> bool {
    let words = &x.native.structural;
    let Some(here) = words.iter().position(|w| matches!(w.operation, Op::Switch { table } if std::ptr::eq(&x.native.switches[usize::from(table)], sw))) else { return false };
    if here == 0 || !matches!(words[here - 1].operation, Op::Token { .. }) || x.phase[here - 1] != crate::stream::TAKEN || x.fetch.as_ref().is_some_and(|fx| fx.part_at(x, here - 1)) {
        return false;
    }
    (0..words.len()).all(|pc| pc == here - 1 || !plain_succ(x, pc).contains(&here))
}

/// A keyword's way from `at` to its leaf (`LEAF` of the word's kind), each word on it one with
/// one successor that reads no code (an `OPEN`, a supertype, a field, a `GOTO`): their code, the
/// leaf's kind the constant of `code`'s word, and the place after the leaf. `None` where the way
/// is not such.
fn known_leaf(x: &Facts<'_>, at: u16, code: u32) -> Result<Option<(String, u16)>, String> {
    let words = &x.native.structural;
    let (mut q, mut out) = (usize::from(at), String::new());
    for _ in 0..16 {
        let Some(w) = words.get(q) else { return Ok(None) };
        let next = succ(x, q);
        let &[t] = next.as_slice() else { return Ok(None) };
        match w.operation {
            Op::Leaf { kind, .. } if kind == WORD_KIND => {
                out.push_str(&word_with(x, q, &|_| Ok(String::new()), Some(code))?);
                return Ok(Some((out, t as u16)));
            }
            Op::Open { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } => {
                out.push_str(&word(x, q, &|_| Ok(String::new()))?);
                q = t;
            }
            _ => return Ok(None),
        }
    }
    Ok(None)
}

/// Whether the end a switch's arm makes (`rec_end`) is read from word `t` on, in a direct format 16 run: as the lean
/// parser makes a node's end where it writes it, an arm makes its token's end only where a word reads it. Each way is
/// walked twice over: the end itself, and the end a `TOKEN` passes on as the last token's (`last_end`), until a word
/// makes it again. A word not known to leave both alone reads them; outside a direct run, every end is read.
fn end_read(x: &Facts<'_>, t: u16) -> bool {
    end_read_from(x, usize::from(t), false)
}

/// `end_read` from word `t`, of the last token's end where `last` (a string's copy of it: `part_plan`).
fn end_read_from(x: &Facts<'_>, t: usize, last: bool) -> bool {
    if !(x.sixteen && x.coded) || x.flagged {
        return true;
    }
    let words = &x.native.structural;
    // (word, the last token's end, not this one's)
    let mut seen: HashSet<(usize, bool)> = HashSet::new();
    let mut work = vec![(t, last)];
    while let Some((pc, last)) = work.pop() {
        if pc >= words.len() || !seen.insert((pc, last)) {
            continue;
        }
        // A string copies this end first, as the last token's (`s.last_end = s.rec_end;`), then makes both again
        // at its parts (a content's end, the closer's): this end on, as the last one, at the closer's place, where
        // no part came between. The last one in it is made again.
        if let Some(fx) = x.fetch.as_ref().filter(|fx| fx.part_at(x, pc)) {
            if !last {
                let Ok(pf) = part_facts(x, fx, pc) else { return true };
                work.push((usize::from(past_span(x, pf.closer, 1)), true));
            }
            continue;
        }
        // The words after, as they run: a call's callee, a return's places, else each word's own (not `succ`, which
        // writes each switch's arms, and so asks this again).
        let next: Vec<usize> = match (words[pc].operation, &x.one) {
            (Op::Push { .. }, Some(_)) => vec![crate::native_cpu_stream::callee(x.native, pc)],
            (Op::Return { .. }, Some(one)) => one.returns.get(&pc).map_or(Vec::new(), |r| r.iter().map(|&(_, t)| t).collect()),
            _ => plain_succ(x, pc),
        };
        let on = |work: &mut Vec<(usize, bool)>, l: bool| work.extend(next.iter().map(|&q| (q, l)));
        match words[pc].operation {
            // The end passed on as the last token's; this one's kept until an arm makes it again.
            Op::Token { .. } if !last => {
                on(&mut work, false);
                on(&mut work, true);
            }
            Op::Token { .. } => {}
            Op::Switch { table } => {
                let sw = &x.native.switches[usize::from(table)];
                if !(on_kinds(x, sw) && fresh(x, sw)) {
                    return true;
                }
                if last {
                    on(&mut work, true);
                } else if sw.end != REFUSE {
                    // Every arm makes its token's end; at the stream's end, none does.
                    work.push((usize::from(sw.end), false));
                }
            }
            // A close of its own token's end reads this one's; any other, the last one's.
            Op::Close { this } => {
                if this != last {
                    return true;
                }
                on(&mut work, last);
            }
            Op::Leaf { deferred, .. } => {
                if !last || deferred {
                    return true;
                }
                on(&mut work, last);
            }
            // A last-byte `SPAN` reads nothing here (`flagged` is out); a length's reads the end.
            Op::Check { .. } | Op::Span { last: false, .. } if !last => return true,
            Op::Check { .. } | Op::Span { .. } | Op::Open { .. } | Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Count { .. } | Op::Push { .. } | Op::Return { .. } | Op::Empty { .. } => on(&mut work, last),
            Op::Accept | Op::Stop => {}
            _ => return true,
        }
    }
    false
}

/// `s.code = code;`, where a word reached from `to` reads it (`code_read`); nothing where none
/// does, as simdjson's switch makes no record a token.
fn code_set(x: &Facts<'_>, to: &[u16], code: u32) -> String {
    if to.iter().any(|&t| reads_code(x, t)) { format!("t.code = {code:#x};\n") } else { String::new() }
}

/// Whether the byte past the file (0, padding) is refused by `sw`'s switch on the first byte, and
/// by the switch it gives its other bytes to (`delegate_of`).
/// Whether switch `table` takes the stream's end alone and is tested by the stream's count, as the lean parser tests
/// `next == index + count` once, after the walk: a byte switch of the records tier 2 makes, in a sentinel run, with
/// an end arm and no byte arm of its own or the other switch's (`fetched_arms`), each word on it right after a `TOKEN`
/// that takes, where the last token's end needs no copy at a token (`last_ends`). There the `TOKEN` takes nothing, and the switch tests `t.next == n`, every token taken: the token
/// it would take is past the count (its end, 0543) or one no arm takes.
fn counted_end(x: &Facts<'_>, table: u16) -> bool {
    // Where the last token's end is kept without a copy at each token (`last_ends`): a take left out changes no end.
    let Some(fx) = x.fetch.as_ref().filter(|_| x.sentinel && !x.flagged && x.ends.is_some()) else { return false };
    let sw = &x.native.switches[usize::from(table)];
    if sw.end == REFUSE || !(on_kinds(x, sw) && fresh(x, sw)) {
        return false;
    }
    let mut groups = Vec::new();
    fetched_arms_with(x, fx, sw, &mut Plan::default(), &mut groups);
    let words = &x.native.structural;
    groups.is_empty()
        && words.iter().enumerate().filter(|(_, w)| matches!(w.operation, Op::Switch { table: t } if t == table)).all(|(pc, _)| {
            pc > 0 && matches!(words[pc - 1].operation, Op::Token { any: None, extras: false, .. }) && x.phase[pc - 1] == crate::stream::TAKEN && !fx.part_at(x, pc - 1)
        })
}

fn zero_refused(x: &Facts<'_>, fx: &Fetched, sw: &ByteSwitch) -> bool {
    let ft = &fx.ft;
    let named = sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first() == Some(&0)));
    let code = match ft.rules.get(usize::from(ft.ends[0])) {
        Some(&crate::records::EndRule::Run(_)) => u32::from(ft.kinds[0]),
        _ => ft.codes1[0],
    };
    if named || kind_target(x, fx, sw, code) != REFUSE {
        return false;
    }
    delegate_of(x, fx, sw).is_none_or(|(_, other)| zero_refused(x, fx, other))
}

/// Whether word `to` is reached from word `from` in the same function (`succ`, the plain
/// successors: no switch's to another).
fn reaches(x: &Facts<'_>, from: usize, to: usize) -> bool {
    let mut seen = vec![false; x.native.structural.len()];
    let mut work = vec![from];
    while let Some(pc) = work.pop() {
        if pc == to {
            return true;
        }
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        work.extend(plain_succ(x, pc));
    }
    false
}

/// What a part's `TOKEN` (`Fetched::part_at`) and the switch after it go to: the closer's
/// place, the content's, the part's by its word (else its kind's); the closer's code.
struct PartFacts {
    closer: u16,
    content: u16,
    words: Vec<(u16, u16)>,
    part: u16,
    closer_code: u32,
}

/// Whether the token's code is read from word `t` on, before a word that makes the next
/// token's or this one's again: a `TOKEN`, a part (which writes its own) and a switch on the
/// first byte (`fresh`) end the walk; a switch with a kind path reads it. Of the other words,
/// those known to read no code are passed (each successor, a call's returns); any other reads it.
fn reads_code(x: &Facts<'_>, t: u16) -> bool {
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
        work.extend(succ(x, pc));
    }
    false
}

fn part_facts(x: &Facts<'_>, fx: &Fetched, pc: usize) -> Result<PartFacts, String> {
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
    if sw.words.iter().any(|&(y, _)| ft.words.get(usize::from(y)).is_some_and(|(w, _)| w.first().is_some_and(|&f| !in_set(&q.stops, usize::from(f))))) || reads_code(x, content) {
        return Err(format!("word {pc}: a content part whose code is read"));
    }
    let codes2 = q.codes2.as_ref().expect("checked in `fetched`");
    let kinds: BTreeSet<u32> = codes2.iter().map(|c| c & 0xff).collect();
    let &[kind] = kinds.iter().copied().collect::<Vec<_>>().as_slice() else { return Err(format!("word {pc}: a part of more than one kind")) };
    let part = kind_target(x, fx, sw, kind);
    let _ = e0;
    Ok(PartFacts { closer, content, words: sw.words.clone(), part, closer_code })
}

/// A part's `TOKEN` and the switch after it, as `parse_string`'s loop: the next stop from the
/// pending place, then one branch to the closer's place, the content's (the bytes before the
/// stop) or the part's (an escape: its code by its second byte, its place by its word).
fn part_plan(x: &Facts<'_>, fx: &Fetched, pc: usize) -> Plan {
    let pf = part_facts(x, fx, pc).expect("checked in `rust_source_at`");
    let (q, _) = fx.seq.as_ref().expect("checked");
    let mut p = Plan::default();
    let _ = q;
    // A content part whose words go straight back to this one: they are written here, and the
    // stop found is taken on at once, not found again, as `copy_and_find` handles its stop.
    let room = if x.unchecked { "" } else { "s.room()?;\n" };
    // The last token's end, kept at each part, only where a word after the closer reads it (`end_read_from`).
    let keep = if end_read_from(x, usize::from(past_span(x, pf.closer, 1)), true) { "t.last_end = t.rec_end;\n" } else { "" };
    let (content, then) = match straight_to(x, pf.content, pc) {
        Some(code) => (format!("{code}{room}{keep}"), "}\n"),
        None => (p.to(pf.content), "} else "),
    };
    // The closer is one byte: its place's test of that (`SPAN`) holds, and is not made again.
    let closer = p.to(past_span(x, pf.closer, 1));
    // Each escape's word to its place, past the checks its bytes cannot hold at (as simdjson's
    // `\u` work only after a `u`), the words of one place a range, as `escape_map`'s one load.
    let mut to: Vec<u16> = vec![pf.part; 256];
    for &(w, t) in &pf.words {
        let bytes = fx.ft.words.get(usize::from(w)).map(|(b, _)| b.as_slice()).unwrap_or(&[]);
        if let Some(slot) = to.get_mut(usize::from(w) + 1) {
            *slot = past_void_checks(x, past_span(x, t, 2), bytes);
        }
    }
    to[0] = pf.part;
    let by_word = arms(&mut p, "t.word()", &to);
    // The next part's place: the method's argument (`part_arg`), else the field the opener's arm set.
    let (take, place, done) = if x.part_arg { ("", "at = ", "") } else { ("let at = t.part_at;\n", "t.part_at = ", "t.part_at = 0;\n") };
    p.code = format!(
        "{}{keep}{take}let (i, closer) = s.part_stop(at)?;
if i != at {{
// The bytes before the stop: content (its code read by no word after).
t.start = at;
t.rec_end = i;
{place}i;
{content}
{then}if closer {{
// The closer: the region's last token.
t.start = i;
t.rec_end = i + 1;
{}{done}{closer}
}} else {{
// A part of two bytes (an escape): its code by its second byte.
if i + 1 >= s.len {{ return refuse(why::STRING); }}
t.start = i;
t.rec_end = i + 2;
t.code = S::CODES2[usize::from(s.byte(i + 1))];
{place}i + 2;
{by_word}}}
",
        if x.unchecked { "" } else { "s.room()?;\n" },
        code_set(x, &[past_span(x, pf.closer, 1)], pf.closer_code)
    );
    p
}

/// The words from `from` on to word `to`, where each has one successor and is a node word (`LEAF` of a fixed kind, a
/// supertype, a field), a `GOTO`, a last byte's `SPAN` or a `CHECK`: a part's way back to its `TOKEN`. `None` where
/// the way is other.
fn straight_back(x: &Facts<'_>, from: usize, to: usize) -> Option<Vec<usize>> {
    let words = &x.native.structural;
    let (mut q, mut out) = (from, Vec::new());
    for _ in 0..16 {
        if q == to {
            return Some(out);
        }
        let &[t] = succ(x, q).as_slice() else { return None };
        match words.get(q)?.operation {
            Op::Leaf { kind, .. } if kind != WORD_KIND && kind != REMEMBERED => {}
            Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Span { last: true, .. } | Op::Check { .. } => {}
            _ => return None,
        }
        out.push(q);
        q = t;
    }
    None
}

/// The code of the words from `from` to word `to`, where each has one successor and is a node
/// word (`LEAF` of a fixed kind, a supertype, a field), a `GOTO` or a last byte's `SPAN`; `None`
/// where the way is other.
fn straight_to(x: &Facts<'_>, from: u16, to: usize) -> Option<String> {
    let words = &x.native.structural;
    let (mut q, mut out) = (usize::from(from), String::new());
    for _ in 0..16 {
        if q == to {
            return Some(out);
        }
        let &[t] = succ(x, q).as_slice() else { return None };
        match words.get(q)?.operation {
            Op::Leaf { kind, .. } if kind != WORD_KIND && kind != REMEMBERED => {}
            Op::NodeSuper { .. } | Op::NodeField { .. } | Op::Goto { .. } | Op::Span { last: true, .. } => {}
            _ => return None,
        }
        out.push_str(&word(x, q, &|_| Ok(String::new())).ok()?);
        q = t;
    }
    None
}

/// Place `t` past its `SPAN` of `len` bytes, where the token is known to be that long.
fn past_span(x: &Facts<'_>, t: u16, len: u16) -> u16 {
    match x.native.structural.get(usize::from(t)).map(|w| w.operation) {
        Some(Op::Span { last: false, len: l }) if l == len && !x.flagged => t + 1,
        _ => t,
    }
}

/// Place `t` past the `SPAN`s and the mask checks at it that a token of bytes `bytes` cannot
/// hold at: each check's mask holds only at bytes none of them is (`may_bytes`), so it has no
/// bit in the token and the check holds. As simdjson does the `\u` work only after a `u`.
fn past_void_checks(x: &Facts<'_>, t: u16, bytes: &[u8]) -> u16 {
    let Some(block) = x.native.block_programs.first() else { return t };
    let (words, mut q) = (&x.native.structural, usize::from(t));
    let mut past = t;
    if bytes.is_empty() {
        return t;
    }
    loop {
        match words.get(q).map(|w| w.operation) {
            Some(Op::Span { last: true, .. }) if !x.flagged => q += 1,
            Some(Op::Check { check }) => {
                let c = &x.native.local_checks[usize::from(check)];
                let Some(&o) = block.masks.get(usize::from(c.mask)).filter(|_| !c.token && c.back == 0) else { return past };
                // The mask's bit at a place is its value's `delay` places on: the byte there is
                // `delay` back from the value's.
                let may = may_bytes(block, usize::from(block.outputs[o]), u32::from(block.output_delays[o]));
                if bytes.iter().any(|&b| may[usize::from(b) / 64] >> (b % 64) & 1 == 1) {
                    return past;
                }
                q += 1;
                past = q as u16;
            }
            _ => return past,
        }
    }
}

/// The bytes, `back` places before where value `x` of a block holds, that can be there: a
/// superset. A value of its own byte (`native_cpu_lanes::byte_set`) at its own place; a `prev` by
/// `k` the same `k` places further back; an `and` only where both can, an `or` where either; any
/// other value at any byte.
fn may_bytes(b: &crate::block_native::BlockNative, x: usize, back: u32) -> [u64; 4] {
    use crate::block_native::BlockInst as I;
    const ALL: [u64; 4] = [!0; 4];
    if let Some(set) = crate::native_cpu_lanes::byte_set(b, x) {
        return if back == 0 || set == [0; 4] { set } else { ALL };
    }
    match b.insts[x] {
        I::And(p, q) => {
            let (p, q) = (may_bytes(b, usize::from(p), back), may_bytes(b, usize::from(q), back));
            std::array::from_fn(|k| p[k] & q[k])
        }
        I::Or(p, q) => {
            let (p, q) = (may_bytes(b, usize::from(p), back), may_bytes(b, usize::from(q), back));
            std::array::from_fn(|k| p[k] | q[k])
        }
        I::AndNot(p, _) => may_bytes(b, usize::from(p), back),
        I::Prev(a, k) if back >= u32::from(k) => may_bytes(b, usize::from(a), back - u32::from(k)),
        I::Const(0) => [0; 4],
        _ => ALL,
    }
}

/// A token's check that this backend writes (`program::token_value`): none back, no `ended`.
fn token_check_ok(c: &LocalCheck) -> bool {
    c.token && c.back == 0 && !c.ops.iter().any(|o| matches!(o, LocalOp::Ended)) && c.delay < 64
}

/// The check at place `t`, where its words are a token's `SPAN`s then a `CHECK` over its bytes
/// alone (`token_check_ok`) of a delay under 15: the check's place, its index, the check.
fn check_at<'a>(x: &'a Facts<'a>, t: u16) -> Option<(usize, usize, &'a LocalCheck)> {
    let words = &x.native.structural;
    let mut q = usize::from(t);
    while matches!(words.get(q).map(|w| w.operation), Some(Op::Span { last: true, .. })) {
        q += 1;
    }
    let Some(Op::Check { check }) = words.get(q).map(|w| w.operation) else { return None };
    let c = &x.native.local_checks[usize::from(check)];
    (token_check_ok(c) && c.delay < 15 && q + 1 < words.len()).then_some((q, usize::from(check), c))
}
