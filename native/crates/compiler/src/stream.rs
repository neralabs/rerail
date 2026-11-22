//! Tier 2's instructions (decision 0087): the transducer's words over tier 1's token stream, as
//! simdjson's stage 2 runs over its structural indexes. A program is a state machine: each
//! state takes a token (`TOKEN`), switches on its byte (`SWITCH`), writes nodes, and jumps to the
//! next state (`GOTO`). Its verifier refuses a cycle with no `TOKEN`, so its one loop is over the
//! stream. This file holds the switch tables, the verifier and the interpreter; `native_ir` and
//! Cranelift lower the same words.

use crate::program::*;
use crate::Error;
use std::collections::{HashMap, HashSet};

pub use crate::stream_ir::{ByteSwitch, DEAD, GONE, KEEP, REFUSE, REMEMBERED, Reading, Readings, WORD_KIND};

/// Whether a word is one of tier 2's.
pub fn is_stream(opcode: u8) -> bool {
    (TOKEN..=SETTLE).contains(&opcode)
}

/// The operand rules of tier 2's words, and of the words a tier-2 program may hold besides
/// (`STOP`, `GOTO`).
pub fn validate_word(program: &Words, pc: usize, opcode: u8, mode: u8, operand: u16) -> Result<(), Error> {
    let words = program.words.len();
    let block = program.block_programs.first();
    let extracts = block.map_or(0, |b| b.extracts.len());
    // The totals are one flag (`BlockRun::totals`).
    let totals = block.map_or(0, |b| b.totals.len().min(1));
    let masks = block.map_or(0, |b| b.masks.len());
    let records = block.and_then(|b| b.records.as_ref());
    let ok = match opcode {
        // A token is a record (`records`): its extract is the records' starts.
        // `m=1+k`: mask `k` holds where any mark does (the marks are live only there).
        // `TOKEN_EXTRAS`: the extras taken too (`Words::extras`).
        TOKEN => {
            let m = mode & !TOKEN_EXTRAS;
            (m == 0 || usize::from(m - 1) < masks) && (mode & TOKEN_EXTRAS == 0 || !program.extras.is_empty()) && usize::from(operand) < extracts && records.is_some_and(|r| r.starts == operand)
        }
        // `m=1`: a padded root (`whole padded`), from its first token or comment.
        ACCEPT => mode <= 1 && operand == 0,
        EMPTY => mode == 0 && usize::from(operand) < totals,
        SWITCH => mode == 0 && usize::from(operand) < program.switches.len(),
        // `m=1`: a test, which skips the next word where the token is not the string.
        // A word of at most 64 bytes: the CPU compares 8 at a time, unrolled, within the 64 bytes
        // past the file.
        // The word is the record's (its code), so the program's strings are the records' words.
        WORD_IS => mode <= 1 && program.strings.get(usize::from(operand)).is_some_and(|s| !s.is_empty() && s.len() <= 64) && records.is_some_and(|r| r.words == program.strings),
        // A level's word's kind (`REMEMBERED`) only as a moved or relabelled node's.
        OPEN | NODE_SUPER => mode == 0 && operand != REMEMBERED,
        // `m=1`: an extra's leaf, deferred; `m=2`: an extra's leaf placed now, never the node
        // made last at its level.
        // `m=3`: a token the grammar implies, no node.
        // A keyword's leaf (`WORD_KIND`, `m=0`): the kind of its token's word, which the records
        // name (`Words::word_kinds`).
        LEAF if operand == WORD_KIND => mode == 0 && !program.word_kinds.is_empty() && records.is_some_and(|r| r.words == program.strings),
        LEAF => mode <= 3 && operand != REMEMBERED,
        // Mark `m & MARK_ORDER` (its order), its bit in mask `operand`; `MARK_END`: the file's
        // end has it; `MARK_KEEP`: kept, for the callee its arm calls, which takes it.
        MARK => usize::from(operand) < masks,
        ROW => mode == 0 && usize::from(operand) < masks,
        // `m=1`: the node a wrap moved; `m=2`: the node made last at this level (`relabel`);
        // `m=3` (a field): the node that one, a wrap, holds first. `RETAG m=3`: that node and
        // the nodes inside it, read by reading `operand` (`reread`).
        RETAG => (mode <= 2 && (mode != 0 || operand != REMEMBERED)) || (mode == 3 && usize::from(operand) < program.readings.maps.len()),
        NODE_FIELD => mode <= 4 && operand != 0,
        // `m=1`: the open node's two leaves, one (`fold`).
        MERGE => mode <= 1 && operand != REMEMBERED,
        WRAP => mode == 0 && operand != REMEMBERED,
        // A keyword's word (`WORD_KIND`): its token's word's kind, as a keyword's leaf's.
        REMEMBER if operand == WORD_KIND => mode == 0 && !program.word_kinds.is_empty() && records.is_some_and(|r| r.words == program.strings),
        REMEMBER => mode == 0 && operand != 0 && operand != REMEMBERED,
        FLAG => mode <= 2 && operand < 16,
        COUNT => mode <= 1 && usize::from(operand) < program.counts.len(),
        CHECK => mode == 0 && usize::from(operand) < program.local_checks.len(),
        // `m=2`, `m=3+f`: a mark's `loses`, the second where flag `f` is set.
        // A settle's mask, or a `drops`'s or `loses`'s gap pattern (`Words::gap_patterns`).
        SETTLE if mode == 0 => operand < 32 && program.block_programs.first().and_then(|b| b.records.as_ref()).is_some_and(|r| r.own >> operand & 1 == 1),
        SETTLE => mode < SETTLE_MOVE + 1 + 16 && program.gap_patterns.get(usize::from(operand)).is_some_and(|m| usize::from(operand) == 0 || program.gap_patterns[usize::from(operand) - 1].order != m.order),
        CLOSE => mode <= 1 && operand == 0,
        // `m`: the flag the call runs with (`push_flag`), of 16.
        PUSH => mode <= 32 && usize::from(operand) < words,
        RETURN => mode <= 1 && operand == 0,
        // `m=1`: the token's end is its record's (operand 0).
        SPAN => (mode == 0 && operand != 0) || (mode == 1 && operand == 0),
        _ => return Err(Error::InvalidOpcode(pc)),
    };
    if ok { Ok(()) } else { Err(Error::InvalidOperand(pc)) }
}

/// A tier-2 program's whole-program rules: only tier 2's words, `STOP` and `GOTO`; switch
/// targets in range; a return lands on a state's `TOKEN` (or the `SWITCH` after it); and no
/// cycle without a `TOKEN` (decision 0087 §5).
pub fn validate(program: &Words) -> Result<(), Error> {
    let words = &program.words;
    for (pc, &w) in words.iter().enumerate() {
        let (opcode, _, _) = split(w);
        if !(is_stream(opcode) || opcode == STOP || opcode == GOTO) {
            return Err(Error::InvalidOpcode(pc));
        }
    }
    let target = |t: u16| t == REFUSE || usize::from(t) < words.len();
    if program.switches.iter().any(|s| s.targets.len() != 256 || !s.all_targets().all(target)) {
        return Err(Error::InvalidTarget(0));
    }
    // A switch's words: on the first byte, with records whose words are the strings; each a
    // string, in order.
    let records = program.block_programs.first().and_then(|b| b.records.as_ref());
    for s in program.switches.iter().filter(|s| !s.words.is_empty()) {
        let in_order = s.words.windows(2).all(|w| w[0].0 < w[1].0);
        let named = s.words.iter().all(|&(w, _)| usize::from(w) < program.strings.len());
        if s.offset != 0 || !in_order || !named || records.is_none_or(|r| r.words != program.strings) {
            return Err(Error::InvalidTarget(0));
        }
    }
    // A switch on the first byte is one on the record's kind (`records`): alike within a kind.
    if let Some(r) = program.block_programs.first().and_then(|b| b.records.as_ref()) {
        for s in program.switches.iter().filter(|s| s.offset == 0) {
            let mut seen: HashMap<u8, u16> = HashMap::new();
            for (b, &t) in s.targets.iter().enumerate() {
                if *seen.entry(r.kinds[b]).or_insert(t) != t {
                    return Err(Error::InvalidTarget(0));
                }
            }
        }
    }
    let returns: Vec<usize> = words
        .iter()
        .filter(|&&w| split(w).0 == PUSH)
        .map(|&w| usize::from(split(w).2))
        .collect();
    // A state: `TOKEN`, a `MARK` and its way (`GOTO` or `STOP`) per mark it tests, `SWITCH`; or
    // a state that only calls (decision 0492): `TOKEN`, then its call's code (its guards, acts,
    // `PUSH`), or a `GOTO` to it, with no mark test.
    // After the `TOKEN`, a call's field on the node it made (`NODE_FIELD m=4`), where both
    // returns land (`RETURN m=0` at the `TOKEN`, `m=1` past it). A mark that loses (`, loses`)
    // has its `SETTLE m=2` right before its `MARK`, and one that refuses (`, refuses`) its
    // `SETTLE m=19` before that. A mark that ends levels where a flag says so (`lose_flag`) has
    // its test gated: `FLAG m=2`, `GOTO` the test, `GOTO` past it.
    let asks = |q: usize| words.get(q).is_some_and(|&w| split(w).0 == SETTLE && split(w).1 >= 2);
    let lose = |q: usize| (q..).take_while(|&i| asks(i)).count();
    let state = |p: usize| {
        let mut q = p + 1;
        if words.get(q).is_some_and(|&w| split(w).0 == NODE_FIELD && split(w).1 == 4) {
            q += 1;
        }
        // A mark test gated on a flag (a mark that ends levels only at its own level's callees):
        // `FLAG m=2`, a `GOTO` to the test, a `GOTO` past it.
        let gate = |q: usize| {
            let at = |i: usize| words.get(q + i).map(|&w| split(w));
            usize::from(matches!(at(0), Some((FLAG, 2, _))) && at(1).is_some_and(|w| w.0 == GOTO && usize::from(w.2) == q + 3) && at(2).is_some_and(|w| w.0 == GOTO)) * 3
        };
        while words.get(q + gate(q) + lose(q + gate(q))).is_some_and(|&w| split(w).0 == MARK) {
            q += gate(q);
            q += lose(q);
            if words.get(q + 1).is_none_or(|&w| !matches!(split(w).0, GOTO | STOP)) {
                return false;
            }
            q += 2;
        }
        split(words[p]).0 == TOKEN && words.get(q).is_some_and(|&w| split(w).0 == SWITCH || q == p + 1 + usize::from(split(words[p + 1]).0 == NODE_FIELD) && matches!(split(w).0, GOTO | PUSH | OPEN | FLAG | REMEMBER | SETTLE))
    };
    for &p in &returns {
        if !state(p) {
            return Err(Error::InvalidTarget(p));
        }
    }
    for (pc, &w) in words.iter().enumerate() {
        if matches!(split(w).0, MARK | ROW) && words.get(pc + 1).is_none_or(|&w| !matches!(split(w).0, GOTO | STOP)) {
            return Err(Error::InvalidTarget(pc));
        }
    }
    // Every cycle passes a `TOKEN` or a `RETURN`: with their edges cut, the graph is acyclic.
    // A `RETURN` pops a call, and from a `PUSH` no `RETURN` is reached before a `TOKEN` or a
    // taken mark (below): a cycle with no `TOKEN` pushes nothing, so it turns at most once per
    // call made, and calls are made only between tokens and taken marks. The returns of a file
    // are at most its calls.
    let step = |pc: usize, cut_return: bool| -> Vec<usize> {
        let (opcode, mode, operand) = split(words[pc]);
        match opcode {
            STOP | ACCEPT | TOKEN => vec![],
            RETURN if cut_return => vec![],
            GOTO => vec![usize::from(operand)],
            SWITCH => {
                let s = &program.switches[usize::from(operand)];
                let mut out: Vec<usize> = s.all_targets().filter(|&t| t != REFUSE).map(usize::from).collect();
                out.sort_unstable();
                out.dedup();
                out
            }
            RETURN => returns.iter().map(|&p| p + usize::from(mode)).collect(),
            WORD_IS if mode == 1 => vec![pc + 1, pc + 2],
            FLAG if mode == 2 => vec![pc + 1, pc + 2],
            MARK | ROW => vec![pc + 1, pc + 2],
            _ => vec![pc + 1],
        }
    };
    let successors = |pc: usize| step(pc, true);
    // From a `PUSH`, no `RETURN` before a `TOKEN`; but a `RETURN m=0` after a `SPAN`: the call
    // took the token, and its return lands on the caller's `TOKEN`, which takes the next. (A
    // call that takes no token itself, a function a rule's, goes to its callee's start, whose arm
    // takes it.) A `RETURN m=1` switches on the token again: a `TOKEN` must come first, or a
    // mark taken: a `MARK` that holds (its next word) and is not kept clears itself and the marks
    // before it, and only `TOKEN` sets them again, so a token's calls that take only marks are
    // at most the marks, 32. A kept mark (`MARK_KEEP`, its callee's to take) is no progress.
    for (pc, &w) in words.iter().enumerate() {
        if split(w).0 != PUSH {
            continue;
        }
        // Each place, seen before or after a `SPAN` (bit 0) and a taken mark (bit 1).
        let mut seen = vec![[false; 4]; words.len()];
        let mut todo = vec![(pc, 0u8)];
        while let Some((q, done)) = todo.pop() {
            let (op_q, mode_q, _) = split(words[q]);
            let done = done | u8::from(op_q == SPAN);
            for n in step(q, false) {
                if n >= words.len() {
                    return Err(Error::InvalidTarget(q));
                }
                // The mark's next word runs where it holds: taken, unless kept.
                let done = done | if op_q == MARK && n == q + 1 && mode_q & MARK_KEEP == 0 { 2 } else { 0 };
                let (op, mode, _) = split(words[n]);
                if op == RETURN {
                    if done & 2 != 0 || (done & 1 != 0 && mode == 0) {
                        continue;
                    }
                    return Err(Error::NonProgressingCycle(n));
                }
                if !std::mem::replace(&mut seen[n][usize::from(done)], true) {
                    todo.push((n, done));
                }
            }
        }
    }
    // 0: unseen, 1: on the path, 2: done.
    let mut mark = vec![0u8; words.len()];
    for root in 0..words.len() {
        if mark[root] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(root, successors(root))];
        mark[root] = 1;
        while let Some((pc, next)) = stack.last_mut() {
            match next.pop() {
                Some(n) if n >= words.len() => return Err(Error::InvalidTarget(*pc)),
                Some(n) if mark[n] == 1 => return Err(Error::NonProgressingCycle(n)),
                Some(n) if mark[n] == 0 => {
                    mark[n] = 1;
                    stack.push((n, successors(n)));
                }
                Some(_) => {}
                None => {
                    mark[*pc] = 2;
                    stack.pop();
                }
            }
        }
    }
    phases(program)?;
    Ok(())
}

/// A word's phase (`phases`): its token taken (`SPAN`) and the next not taken yet (`TOKEN`).
pub const TAKEN: u8 = 1;
/// The token not taken: words switch on it, test its marks, take it.
pub const UNTAKEN: u8 = 2;

/// Each word's phases, as bits (`TAKEN`, `UNTAKEN`): a word runs between a `SPAN` and the next
/// `TOKEN` (the token taken) or between a `TOKEN` and the next `SPAN`. A `SPAN` takes a token
/// for good, so the phase is the program's, not the run's, and no executor keeps it:
///
/// - the program starts with no token, as after one taken;
/// - a call's callee starts in its caller's phase; a return lands on its caller's `TOKEN` with
///   a token taken (`RETURN m=0`), or past it with none (`m=1`);
/// - a word that reads the next token (`SWITCH`, `MARK`, `ROW`, `SPAN`) runs only with it
///   untaken, and every word but `TOKEN` (which then takes none) in one phase only.
///
/// Words of an unreached place have none.
pub fn phases(program: &Words) -> Result<Vec<u8>, Error> {
    let words = &program.words;
    let n = words.len();
    let returns: Vec<usize> = words.iter().filter(|&&w| split(w).0 == PUSH).map(|&w| usize::from(split(w).2)).collect();
    let mut phase = vec![0u8; n];
    let mut work = vec![(0usize, TAKEN)];
    while let Some((pc, ph)) = work.pop() {
        if pc >= n || phase[pc] & ph != 0 {
            continue;
        }
        phase[pc] |= ph;
        let (op, mode, operand) = split(words[pc]);
        let reads = matches!(op, SWITCH | MARK | ROW | SPAN);
        if (reads && ph == TAKEN) || (op != TOKEN && phase[pc] == TAKEN | UNTAKEN) {
            return Err(Error::Phase(pc));
        }
        let after = match op {
            SPAN => TAKEN,
            TOKEN => UNTAKEN,
            _ => ph,
        };
        match op {
            STOP | ACCEPT => {}
            RETURN => {
                if (mode == 0) != (ph == TAKEN) {
                    return Err(Error::Phase(pc));
                }
                for &p in &returns {
                    work.push(if mode == 0 { (p, TAKEN) } else { (p + 1, UNTAKEN) });
                }
            }
            GOTO => work.push((usize::from(operand), after)),
            SWITCH => {
                let s = &program.switches[usize::from(operand)];
                work.extend(s.all_targets().filter(|&t| t != REFUSE).map(|t| (usize::from(t), after)));
            }
            WORD_IS if mode != 0 => work.extend([(pc + 1, after), (pc + 2, after)]),
            FLAG if mode == 2 => work.extend([(pc + 1, after), (pc + 2, after)]),
            MARK | ROW => work.extend([(pc + 1, after), (pc + 2, after)]),
            _ => work.push((pc + 1, after)),
        }
    }
    Ok(phase)
}

/// A node tier 2 wrote, in preorder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: u16,
    pub field: u16,
    /// Supertypes, outermost first.
    pub supers: Vec<u16>,
    pub start: u32,
    /// One past its last byte.
    pub end: u32,
    /// It and every node inside it.
    pub size: u32,
}

/// The engine's tape of `nodes`, in preorder as `run` gives them, over `src`: its bytes
/// (`rerail_tape_format`, decision 0140). A node of an extra's kind (`Readings::extras`) is
/// flagged an extra; `tag` is the language's. The interpreter's tape: the meaning the
/// compiled tiers' bytes are checked against.
pub fn image(nodes: &[Node], extras: &[u16], tag: u8, src: &[u8]) -> Vec<u8> {
    // The supertypes the nodes are in, ascending: a bit each in a node's key, its code's classes.
    let mut classes: Vec<u16> = nodes.iter().flat_map(|n| n.supers.iter().copied()).collect();
    classes.sort_unstable();
    classes.dedup();
    let rows: Vec<(u32, u32, u32, u32)> = nodes
        .iter()
        .map(|n| {
            let flags = if extras.contains(&n.kind) { rerail_tape_format::CODE_EXTRA } else { 0 };
            let member = n.supers.iter().fold(0u32, |m, s| m | 1 << (rerail_tape_format::CLASS_SHIFT as usize + classes.binary_search(s).expect("a class of the nodes'")));
            (rerail_tape_format::key(n.kind, n.field, flags) | member, n.start, n.end, n.size)
        })
        .collect();
    rerail_tape_format::preorder_image_in(tag, &classes, src, &rows)
}

/// Why a file went to tree-sitter: the word that refused it, and what it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub pc: usize,
    pub why: &'static str,
    /// The first byte of the token it refused at, where the refuser knows it (the interpreter).
    pub at: Option<u32>,
}

/// The interpreter of a tier-2 program over tier 1's run of `bytes`: a token is tier 1's
/// record (`records`), its first byte, its last byte and its code; `EMPTY` reads the totals.
/// It executes the words only; the tables it reads are the program's switches, strings and
/// checks.
/// The reader a record's first byte `b`'s rule is, where tier 2 splits its regions (`Split::Tier2`).
pub fn split_reader(r: &crate::records::Records, b: u8) -> Option<&crate::records::Reader> {
    match r.rules.get(usize::from(r.ends[usize::from(b)])) {
        Some(&crate::records::EndRule::Read(i)) => r.readers.get(usize::from(i)).filter(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2),
        _ => None,
    }
}

pub fn run(program: &Words, bytes: &[u8], tier1: &crate::block::BlockRun) -> Result<Vec<Node>, Refused> {
    run_seen(program, bytes, tier1, &mut |_| {})
}

/// `run`, with `seen` told the place of each word it runs, in order: the words' profile (which
/// states follow which, how often), for laying out the generated code.
pub fn run_seen(program: &Words, bytes: &[u8], tier1: &crate::block::BlockRun, seen: &mut dyn FnMut(usize)) -> Result<Vec<Node>, Refused> {
    let words = &program.words;
    let none: Vec<u32> = Vec::new();
    // A kind as a word names it: a level's word's (`REMEMBERED`), none where it has none.
    let kind_of = |k: u16, word: u16| if k != REMEMBERED { Some(k) } else if word != 0 { Some(word) } else { None };
    // The records' starts, as any `TOKEN` names them (its mode names the marks' mask).
    let starts: &[u32] = words.iter().map(|&w| split(w)).find(|&(o, _, _)| o == TOKEN).map_or(&none, |(_, _, x)| &tier1.positions[usize::from(x)]);
    let records = program.block_programs.first().and_then(|b| b.records.as_ref());
    let lasts: &[u32] = records.map_or(&none, |r| &tier1.positions[usize::from(r.lasts)]);
    // Each kind's least byte: a switch on the first byte is one on the record's kind.
    let kind_reps: Option<Vec<u8>> = records.map(|r| {
        let n = r.kinds.iter().copied().max().map_or(0, |m| usize::from(m) + 1);
        (0..n).map(|k| r.kinds.iter().position(|&x| usize::from(x) == k).unwrap_or(0) as u8).collect()
    });
    let mut pc = 0usize;
    // The token's record: its start, its end (one past its last byte) and its code. `end` is
    // `start` until its bytes are taken (`SPAN`).
    let (mut next, mut start, mut end, mut rec_end, mut code, mut last_end, mut at_end) = (0usize, 0u32, 0u32, 0u32, 0u32, 0u32, false);
    // `need`: the token's bytes were taken (`SPAN`), so `TOKEN` takes the next; an arm of a mark
    // takes none, and the next state switches on the same token. `live`: the marks not yet
    // taken at this token (a level that ends at it leaves the rest to its caller, which tests
    // them again). `pending`: the deferred extras' leaves at the table's end.
    // `keep`: while leaves are pending, the end of the token before them, where a node closed
    // then ends.
    let (mut need, mut live, mut pending, mut keep) = (true, 0u64, 0usize, 0u32);
    // A region read in parts that tier 2 splits (`Split::Tier2`, decision 0323): where a record
    // is a whole region (at its opener, past its byte), its tokens (`records::read_parts`) with
    // their codes (`records::pass`), taken one a `TOKEN` before the next record.
    let late = records.filter(|r| r.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2)).map(|r| (r, crate::records::WordTable::new(&r.words).expect("verified: the words fit a table")));

    let mut parts: std::collections::VecDeque<(u32, u32, u32)> = std::collections::VecDeque::new();
    // Tier 1's records a template left open (decision 0600): a string's, whose parts go on at the
    // next opener of a reader that resumes it (`Reader::resumes`, the template's `}`), past the
    // records of the template's code, as the lean parser reads a string's template.
    let mut strings = 0usize;
    let resumers: Vec<u16> = records.map(|r| r.readers.iter().filter(|rd| rd.resumes).flat_map(|rd| rd.arms.iter().map(|a| a.opener)).collect()).unwrap_or_default();
    let block = program.block_programs.first();
    let mask_bit = |m: u16, at: u32| -> bool {
        block.is_some_and(|b| {
            let words = &tier1.outputs[b.mask_outputs()[usize::from(m)]];
            words.get(at as usize / 64).is_some_and(|w| w >> (at % 64) & 1 == 1)
        })
    };
    // A `drops`'s or `loses`'s question (`Words::gap_patterns` from `o`) at the token, record
    // `next - 1`: none at the end.
    // A gap pattern's question at the token (at the end, past the last record: what `refuses`
    // asks there; `drops` and `loses` need a token, `!at_end`).
    // An extra's bit in its code (`Words::extras`), for a `since`'s grammar token before.
    let gap_extra = program.extras.first().map_or(0, |e| 1u32 << (crate::records::MARK_SHIFT + u32::from(e.0)));
    let gap_at = |o: u16, next: usize, at_end: bool| -> bool {
        let here = if at_end { next } else { next.wrapping_sub(1) };
        (at_end || next > 0) && crate::program::patterns_hold(&program.gap_patterns, usize::from(o), &mut |m| gap_pattern_holds(m, &tier1.codes, starts, bytes, here, gap_extra))
    };
    let gap = |o: u16, next: usize, at_end: bool| -> bool { !at_end && gap_at(o, next, at_end) };
    let mut nodes: Vec<Node> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    // Wraps (docs/parser-generators/24-rr-and-bitlex-plan.md §2.5). Each open node's frame: its
    // subtree's base (the first of its records, by when they were made) and the last node made
    // at its level, as (record, base). A wrap's record goes at the table's end, linked into its
    // base's chain (`head`, `link`); at `ACCEPT` each chain goes before its base, latest
    // (outermost) first. `moved`: the node the last wrap moved.
    let mut bases: Vec<usize> = Vec::new();
    let mut lasts_made: Vec<Option<(usize, usize)>> = Vec::new();
    let (mut head, mut link): (HashMap<usize, usize>, HashMap<usize, usize>) = (HashMap::new(), HashMap::new());
    let mut wraps: HashSet<usize> = HashSet::new();
    let mut moved = 0usize;
    // A reading dropped a node (`GONE`): its record stays, of kind `DEAD`, left out at `ACCEPT`.
    let mut dead = false;
    // Each call's return, with its caller's word (`REMEMBER`), given back where it returns.
    let mut returns: Vec<(u16, u16, u16)> = Vec::new();
    let mut word: u16 = 0;
    // The level's flags (`FLAG`), a bit each: the caller's come back where a call returns.
    let mut flags: u16 = 0;
    // The counts of tokens (`COUNT`).
    let mut counts: Vec<u32> = vec![0; program.counts.len()];
    // An extra a mark lost (`SETTLE m=2`), made again by the next `TOKEN`.
    let mut lost: Option<u16> = None;
    // A refusal at the token being read.
    macro_rules! refuse {
        ($pc:expr, $why:expr) => {
            Err(Refused { pc: $pc, why: $why, at: Some(start) })
        };
    }
    loop {
        seen(pc);
        let (opcode, mode, operand) = split(words[pc]);
        pc += 1;
        match opcode {
            TOKEN if !need => {}
            TOKEN => {
                if at_end {
                    return refuse!(pc - 1, "a token past the stream's end");
                }
                need = false;
                // An extra a mark lost at the token taken (`SETTLE m=2`): after its leaf, of
                // no width at its start.
                if let Some(kind) = lost.take() {
                    if pending == 0 {
                        keep = end;
                    }
                    pending += 1;
                    nodes.push(Node { kind, field: 0, supers: vec![], start, end: start, size: 1 });
                }
                loop {
                    last_end = end;
                    // The record pass checked the records (the totals' flag, `EMPTY`): as many
                    // lasts as starts, each token after the one before.
                    if let Some((s, l, c)) = parts.pop_front() {
                        (start, end, rec_end, code) = (s, s, l + 1, c);
                        break;
                    }
                    let count = starts.len().min(lasts.len());
                    // The string's parts go on at its template's `}`, where it comes before the
                    // next record.
                    let resume = if strings == 0 { None } else { resumers.iter().filter_map(|&o| crate::records::first_bit(&tier1.outputs, o, rec_end as usize)).min().filter(|&p| next == count || (p as u32) < starts[next]) };
                    if let Some(p) = resume {
                        (start, end, rec_end, code) = (p as u32, p as u32, p as u32 + 2, 0);
                    } else {
                        if next == count {
                            at_end = true;
                            break;
                        }
                        (start, end) = (starts[next], starts[next]);
                        rec_end = lasts[next] + 1;
                        code = tier1.codes.get(next).copied().unwrap_or(0);
                        next += 1;
                    }
                    if let Some((r, rd, table)) = late.as_ref().and_then(|(r, t)| Some((*r, split_reader(r, bytes.get(start as usize).copied()?)?, t))) {
                        let opens = rd.arms.first().is_some_and(|a| tier1.outputs.get(usize::from(a.opener)).and_then(|w| w.get(start as usize / 64)).is_some_and(|w| w >> (start % 64) & 1 == 1));
                        if opens && rec_end > start + 1 {
                            let Some(tokens) = crate::records::read_parts(rd, start as usize, &tier1.outputs) else {
                                return refuse!(pc - 1, "a region with no closer");
                            };
                            // Closed at a template: the string's record open over its code.
                            let template = tokens.last().is_some_and(|&(c, _)| rd.arms.iter().any(|a| a.template.is_some_and(|t| crate::records::first_bit(&tier1.outputs, t, c) == Some(c))));
                            if rd.resumes && !template {
                                strings = strings.saturating_sub(1);
                            } else if !rd.resumes && template {
                                strings += 1;
                            }
                            let (s, l): (Vec<u32>, Vec<u32>) = tokens.iter().map(|&(a, b)| (a as u32, b as u32)).unzip();
                            let (codes, _) = crate::records::pass(r, table, bytes, &s, &l, &tier1.outputs);
                            parts.extend(s.iter().zip(&l).zip(&codes).map(|((&a, &b), &c)| (a, b, c)));
                            let (s, l, c) = parts.pop_front().expect("the opener");
                            (start, end, rec_end, code) = (s, s, l + 1, c);
                        }
                    }
                    // An extra (`TOKEN_EXTRAS`): its bytes taken and its leaf deferred, as
                    // `SPAN m=1; LEAF m=1 #K`; then the next token.
                    let extra = if mode & TOKEN_EXTRAS == 0 { None } else { extra_kind(&program.extras, code) };
                    let Some(kind) = extra else { break };
                    end = rec_end;
                    if pending == 0 {
                        keep = last_end;
                    }
                    pending += 1;
                    nodes.push(Node { kind, field: 0, supers: vec![], start, end, size: 1 });
                }
                // The marks live: all, or (`m=1+k`) none where no mark holds and the stream
                // goes on.
                let m = mode & !TOKEN_EXTRAS;
                live = if m == 0 || at_end || mask_bit(u16::from(m - 1), start) { u64::MAX } else { 0 };
                // A token mark that refuses (`TokenMark::refuses`): at a token its bit names,
                // its tokens following, the file is refused.
                let extra_bit = program.extras.first().map(|e| u32::from(e.0));
                let refused = !at_end && program.token_marks.iter().any(|t| t.refuses && code >> (crate::records::MARK_SHIFT + u32::from(t.order)) & 1 == 1 && token_mark_holds(t, &tier1.codes, next, extra_bit));
                if refused {
                    return refuse!(pc - 1, "a token mark that refuses");
                }
            }
            SPAN => {
                // The token's bytes taken: its record's end, or a length (a one-byte token),
                // which must be its record's.
                let e = if mode == 0 { start + u32::from(operand) } else { rec_end };
                if e != rec_end {
                    return refuse!(pc - 1, "a token's length not its record's");
                }
                end = e;
                need = true;
            }
            SWITCH => {
                let s = &program.switches[usize::from(operand)];
                let t = if at_end {
                    s.end
                } else {
                    let at = start as usize + usize::from(s.offset);
                    // A word the switch names: its target. Else zeros past the file, as every
                    // executor reads them.
                    let word = crate::records::word_of(code).checked_sub(1).and_then(|w| s.word(w as u16));
                    // On the first byte with records: on the record's kind, its least byte's
                    // target, as the compiled switch's jump table (a reader moved to a byte of
                    // its own is met there, not at its first byte).
                    let byte = match kind_reps.as_ref() {
                        Some(reps) if s.offset == 0 => reps.get((code & 0xff) as usize).copied().unwrap_or(0),
                        _ => bytes.get(at).copied().unwrap_or(0),
                    };
                    match word {
                        Some(t) if s.offset == 0 => t,
                        _ if at >= rec_end as usize => REFUSE,
                        _ => s.targets[usize::from(byte)],
                    }
                };
                if t == REFUSE {
                    return refuse!(pc - 1, "no state takes the token");
                }
                pc = usize::from(t);
            }
            WORD_IS => {
                // The record's word: its code names the string.
                let same = crate::records::word_of(code) == u32::from(operand) + 1;
                match (mode, same) {
                    (_, true) => {}
                    // A test: the next word is the keyword's way; a token that is not the word skips it.
                    (1, false) => pc += 1,
                    _ => return refuse!(pc - 1, "not the keyword"),
                }
            }
            MARK => {
                let k = u32::from(mode & crate::program::MARK_ORDER);
                // A mark of one word (`Words::mark_words`): the token's record names it too.
                let word_ok = || match program.mark_words.iter().find(|&&(x, _)| u32::from(x) == k) {
                    Some(&(_, s)) => crate::records::word_of(code) == u32::from(s) + 1,
                    None => true,
                };
                // A mark the record has (`Records::marks`): its bit in the code.
                // One the code holds no bit of (`Records::alone`) is a token mark's past it.
                let in_code = (k as usize) < records.map_or(0, |r| r.mark_bits()) && records.is_none_or(|r| r.alone >> k & 1 == 0);
                let token_mark = program.token_marks.iter().any(|m| u32::from(m.order) == k);
                let here = if at_end {
                    mode & crate::program::MARK_END != 0
                } else if in_code {
                    code >> (crate::records::MARK_SHIFT + k) & 1 == 1
                } else if token_mark {
                    // Past the code's (decision 0407): its tokens alone, tested below.
                    true
                } else {
                    mask_bit(operand, start) && word_ok()
                };
                // A token mark (`Words::token_marks`): its tokens follow too.
                // Its entries together (`patterns_hold`): `unless before`s and `before`s.
                let extra_bit = program.extras.first().map(|e| u32::from(e.0));
                let here = here && (at_end || program.token_marks.iter().position(|m| u32::from(m.order) == k).is_none_or(|i| crate::program::patterns_hold(&program.token_marks, i, &mut |m| token_mark_holds(m, &tier1.codes, next, extra_bit))));
                if live >> k & 1 == 1 && here {
                    // Taken: it and the marks before it are done at this token. Kept: its arm
                    // calls the level that takes it.
                    if mode & MARK_KEEP == 0 {
                        live &= !program.taken(k as u8);
                    }
                } else {
                    pc += 1;
                }
            }
            ROW => {
                if !mask_bit(operand, start) {
                    pc += 1;
                }
            }
            SETTLE if mode >= SETTLE_MOVE => {
                // `loses since`: where the question holds at the token, at the mark's level, the
                // first extra pending of no width at the next one's start, in its place.
                let at_level = mode == SETTLE_MOVE || flags >> (mode - SETTLE_MOVE - 1) & 1 == 1;
                if at_level && pending > 1 && gap(operand, next, at_end) {
                    let first = nodes.len() - pending;
                    let s = nodes[first + 1].start;
                    nodes[first].start = s;
                    nodes[first].end = s;
                }
            }
            SETTLE if mode >= SETTLE_REFUSE => {
                // `refuses`: where the question holds at the token, at the mark's level.
                let at_level = mode == SETTLE_REFUSE || flags >> (mode - SETTLE_REFUSE - 1) & 1 == 1;
                if at_level && gap_at(operand, next, at_end) {
                    return refuse!(pc - 1, "a mark's question that refuses");
                }
            }
            SETTLE if mode >= 2 => {
                // `loses`: where the mask holds at the token, the extra pending last is lost (the
                // lean lexer's `asi`): of no width, after the token.
                // Taken out, its kind kept: the `TOKEN` that takes this token makes it again after
                // the token's leaf. One a token: a second test at it finds it taken out.
                let at_level = mode == 2 || flags >> (mode - 3) & 1 == 1;
                if at_level && pending > 0 && lost.is_none() && gap(operand, next, at_end) {
                    lost = nodes.pop().map(|x| x.kind);
                    pending -= 1;
                    // None left pending: the nodes closed next end at the token before.
                    if pending == 0 {
                        last_end = keep;
                    }
                }
            }
            SETTLE if mode == 1 => {
                // `drops`: the extra pending last, where the mask holds at the token. With none
                // left pending, the nodes closed next end at the token, where the semicolon of no
                // width is (the lean lexer's `Asi::Swallow`).
                if pending > 0 && gap(operand, next, at_end) {
                    nodes.pop();
                    pending -= 1;
                    if pending == 0 {
                        last_end = start;
                    }
                }
            }
            SETTLE => {
                // The pending extras are the records before the token's (the one lost last
                // left out): settled up to the first whose own gap has the mark.
                let first = nodes.len() - pending;
                let token = if at_end { next } else { next - 1 };
                let from = token - pending - usize::from(lost.is_some());
                let settled = (0..pending).take_while(|&i| tier1.codes[from + i] >> (crate::records::MARK_SHIFT + u32::from(operand)) & 1 == 0).count();
                // An extra a `loses since` moved (of no width: no other pending is) and the one
                // after it go with those before: the lean lexer's semicolon is past that one.
                let moved = settled + 1 < pending && nodes[first + settled].start == nodes[first + settled].end;
                let settled = if moved { settled + 2 } else { settled };
                if settled > 0 {
                    keep = nodes[first + settled - 1].end;
                    pending -= settled;
                }
            }
            RETAG if mode == 1 => {
                let Some(k) = kind_of(operand, word) else { return refuse!(pc - 1, "a word's kind, and no word") };
                nodes[moved].kind = k;
            }
            RETAG if mode == 3 => {
                // The node made last at this level and the nodes inside it, the records from its
                // base to the pending extras, each base's chain of wraps first (outermost first,
                // as `ACCEPT` places them): read, a kind each.
                let Some(Some((r, base))) = lasts_made.last().copied() else { return refuse!(pc - 1, "a reread with no node before it") };
                let reading = &program.readings.maps[usize::from(operand)];
                let end = nodes.len() - pending;
                let mut i = base;
                let mut went = false;
                while i < end {
                    if wraps.contains(&i) {
                        i += 1;
                        continue;
                    }
                    // At the root's base, its chain from the root: the wraps around it (one open
                    // around it, say) are not read.
                    let mut chain = Vec::new();
                    let mut c = if i == base && r != base { Some(r) } else { head.get(&i).copied() };
                    while let Some(w) = c {
                        chain.push(w);
                        c = link.get(&w).copied();
                    }
                    chain.push(i);
                    let mut next = i + 1;
                    for n in chain {
                        let way = match reading.root {
                            Some(w) if n == r => Some(w),
                            _ => reading.kinds.iter().find(|x| x.0 == nodes[n].kind).map(|x| x.1),
                        };
                        let Some(to) = way else { return refuse!(pc - 1, "a node the reading does not name") };
                        if n != r
                            && let Some(&(_, g)) = reading.fields.iter().find(|x| x.0 == nodes[n].field)
                        {
                            nodes[n].field = g;
                        }
                        match to {
                            // A node's records run from its base, `size` of them.
                            KEEP => {
                                next = i + nodes[n].size as usize;
                                break;
                            }
                            GONE => {
                                went |= n == r;
                                nodes[n].kind = DEAD;
                                dead = true;
                            }
                            k => {
                                let Some(k) = kind_of(k, word) else { return refuse!(pc - 1, "a word's kind, and no word") };
                                nodes[n].kind = k;
                            }
                        }
                    }
                    i = next;
                }
                // The node read went, its children in its place: none is made last at this
                // level until the next.
                if went && let Some(l) = lasts_made.last_mut() {
                    *l = None;
                }
            }
            NODE_FIELD if mode == 3 => {
                // A wrap's frame base is the first record of the node it moved: that node, unless
                // it is a wrap too (then an older wrap is linked before this one).
                let Some(Some((r, base))) = lasts_made.last().copied() else { return refuse!(pc - 1, "a relabel with no node before it") };
                if !wraps.contains(&r) || link.contains_key(&r) {
                    return refuse!(pc - 1, "an inner field on a node that is no wrap of a plain node");
                }
                nodes[base].field = operand;
            }
            // A call's field where it returns: on the node it made, where that has none.
            NODE_FIELD if mode == 4 => {
                let Some(Some((r, _))) = lasts_made.last().copied() else { return refuse!(pc - 1, "a call's field with no node before it") };
                if nodes[r].field == 0 {
                    nodes[r].field = operand;
                }
            }
            RETAG | NODE_FIELD if mode == 2 => {
                let Some(Some((r, _))) = lasts_made.last().copied() else { return refuse!(pc - 1, "a relabel with no node before it") };
                if opcode == RETAG {
                    let Some(k) = kind_of(operand, word) else { return refuse!(pc - 1, "a word's kind, and no word") };
                    nodes[r].kind = k;
                    if k == DEAD {
                        // `drop`: the node goes (a token the grammar implies), left out at
                        // `ACCEPT`; none is made last at this level until the next.
                        dead = true;
                        if let Some(l) = lasts_made.last_mut() {
                            *l = None;
                        }
                    }
                } else {
                    nodes[r].field = operand;
                }
            }
            RETAG => {
                let Some(&at) = open.last() else { return refuse!(pc - 1, "a retag with no open node") };
                nodes[at].kind = operand;
            }
            WRAP => {
                let Some(Some((r, base))) = lasts_made.last().copied() else { return refuse!(pc - 1, "a wrap with no node before it") };
                let w = nodes.len();
                let (field, start) = (nodes[r].field, nodes[r].start);
                nodes[r].field = 0;
                nodes.push(Node { kind: operand, field, supers: vec![], start, end: 0, size: 0 });
                wraps.insert(w);
                if let Some(h) = head.insert(base, w) {
                    link.insert(w, h);
                }
                moved = r;
                pending = 0;
                open.push(w);
                bases.push(base);
                lasts_made.push(Some((r, base)));
            }
            MERGE if mode == 1 => {
                // The innermost open node holds two leaves and nothing else, no extra pending.
                let Some(&o) = open.last() else { return refuse!(pc - 1, "a fold with no open node") };
                if nodes.len() != o + 3 || pending != 0 || nodes[o + 1].size != 1 || nodes[o + 2].size != 1 {
                    return refuse!(pc - 1, "a fold of no two leaves");
                }
                let second = nodes.pop().expect("two leaves");
                nodes[o + 1].kind = operand;
                nodes[o + 1].end = second.end;
                if let Some(l) = lasts_made.last_mut() {
                    *l = Some((o + 1, o + 1));
                }
            }
            MERGE => {
                // The grammar's leaf before the pending extras, which are its text.
                let n = nodes.len();
                let leaf = n.checked_sub(pending + 1);
                if leaf.is_none_or(|l| nodes[l].size != 1 || open.last().is_some_and(|&o| o >= l)) {
                    return refuse!(pc - 1, "a merge with no leaf before it");
                }
                nodes.truncate(n - pending);
                pending = 0;
                let leaf = nodes.last_mut().expect("a leaf");
                leaf.kind = operand;
                leaf.end = end;
            }
            OPEN => {
                pending = 0;
                open.push(nodes.len());
                bases.push(nodes.len());
                lasts_made.push(None);
                nodes.push(Node { kind: operand, field: 0, supers: vec![], start, end: 0, size: 0 });
            }
            CLOSE => {
                let Some(at) = open.pop() else { return refuse!(pc - 1, "a close with no open node") };
                let base = bases.pop().expect("a frame per open node");
                lasts_made.pop();
                if let Some(l) = lasts_made.last_mut() {
                    *l = Some((at, base));
                }
                nodes[at].end = if mode == 1 { end } else if pending > 0 { keep } else { last_end };
                nodes[at].size = (nodes.len() - pending - base) as u32;
            }
            // A token the grammar implies (`implied`): no node; the extras pending stay where
            // they are, as a leaf's would.
            LEAF if mode == 3 => pending = 0,
            LEAF => {
                // A keyword's leaf: its token's word's kind, refused where it names none.
                let operand = if operand != WORD_KIND {
                    operand
                } else {
                    let w = crate::records::word_of(code).checked_sub(1);
                    match w.and_then(|w| program.word_kinds.binary_search_by_key(&(w as u16), |&(s, _)| s).ok()) {
                        Some(i) => program.word_kinds[i].1,
                        None => return refuse!(pc - 1, "a keyword's leaf at a token of no keyword"),
                    }
                };
                if mode == 1 && pending == 0 {
                    keep = last_end;
                }
                pending = if mode == 1 { pending + 1 } else { 0 };
                if mode == 0 {
                    // An extra's (`m=2`) is never the node made last.
                    if let Some(l) = lasts_made.last_mut() {
                        *l = Some((nodes.len(), nodes.len()));
                    }
                }
                nodes.push(Node { kind: operand, field: 0, supers: vec![], start, end, size: 1 });
            }
            NODE_FIELD if mode == 1 => nodes[moved].field = operand,
            NODE_FIELD => nodes.last_mut().expect("a node before its field").field = operand,
            NODE_SUPER => nodes.last_mut().expect("a node before its supertype").supers.push(operand),
            PUSH => {
                // Calls nested at most `DEPTH` deep, as tier 2's runs and simdjson's `DEFAULT_MAX_DEPTH` (0537).
                if returns.len() >= crate::native_cpu_stream::DEPTH as usize {
                    return refuse!(pc - 1, "calls nested past the depth");
                }
                returns.push((operand, word, flags));
                match push_flag_of(mode) {
                    Some((k, true)) => flags |= 1 << k,
                    Some((k, false)) => flags &= !(1 << k),
                    None => {}
                }
            }
            REMEMBER if operand == WORD_KIND => {
                let w = crate::records::word_of(code).checked_sub(1);
                match w.and_then(|w| program.word_kinds.binary_search_by_key(&(w as u16), |&(s, _)| s).ok()) {
                    Some(i) => word = program.word_kinds[i].1,
                    None => return refuse!(pc - 1, "a keyword's word at a token of no keyword"),
                }
            }
            REMEMBER => word = operand,
            COUNT if mode == 0 => {
                counts[usize::from(operand)] += 1;
                if counts[usize::from(operand)] > u32::from(program.counts[usize::from(operand)]) {
                    return refuse!(pc - 1, "a count past its most");
                }
            }
            COUNT => counts[usize::from(operand)] = counts[usize::from(operand)].saturating_sub(1),
            CHECK if program.local_checks[usize::from(operand)].token => {
                // The token's own bytes, as simdjson's stage 2 reads a number (decision 0398).
                let c = &program.local_checks[usize::from(operand)];
                let end = (rec_end as usize).min(bytes.len()).max(start as usize);
                // The tokens before it: the records before this one, ending in turn (decision 0428).
                let before = &lasts[..next.saturating_sub(1).min(lasts.len())];
                let ended = |j: usize| before.binary_search(&(j as u32)).is_ok();
                if crate::program::token_value(c, bytes, start as usize, end, &ended) {
                    return refuse!(pc - 1, "a token's check holds");
                }
            }
            CHECK => {
                // The mask's bits in the token's bytes, a word at a time; at each, the check.
                let c = &program.local_checks[usize::from(operand)];
                let words = block.map(|b| &tier1.outputs[b.mask_outputs()[usize::from(c.mask)]]);
                let (mut j, end) = (start, rec_end);
                while j < end {
                    let w = words.and_then(|ws| ws.get(j as usize / 64)).copied().unwrap_or(0) >> (j % 64);
                    if w == 0 {
                        j = (j / 64 + 1) * 64;
                        continue;
                    }
                    j += w.trailing_zeros();
                    if j < end && local_value(&c.ops, bytes, i64::from(j) + i64::from(c.delay)) {
                        return refuse!(pc - 1, "a local check holds");
                    }
                    j += 1;
                }
            }
            FLAG if mode == 0 => flags |= 1 << operand,
            FLAG if mode == 1 => flags &= !(1 << operand),
            FLAG => {
                if flags & (1 << operand) == 0 {
                    pc += 1;
                }
            }
            RETURN => {
                let Some((p, w, fl)) = returns.pop() else { return refuse!(pc - 1, "a return with no call") };
                (word, flags) = (w, fl);
                pc = usize::from(p) + usize::from(mode);
            }
            ACCEPT => {
                if !at_end || open.len() != 1 || !returns.is_empty() {
                    return refuse!(pc - 1, "the file ends inside a node");
                }
                let at = open[0];
                // A padded root (`m=1`): from its first token or comment (none: the file's end),
                // as tree-sitter's root starts past the file's leading blanks.
                if mode == 1 {
                    nodes[at].start = nodes.iter().enumerate().filter(|&(i, n)| i != at && n.kind != DEAD).map(|(_, n)| n.start).min().unwrap_or(bytes.len() as u32);
                }
                nodes[at].end = bytes.len() as u32;
                nodes[at].size = (nodes.len() - at) as u32;
                if nodes.iter().any(|n| n.kind != DEAD && n.field != 0 && program.readings.provisional.contains(&n.field)) {
                    return refuse!(pc - 1, "a provisional field at the file's end");
                }
                if head.is_empty() && !dead {
                    return Ok(nodes);
                }
                if dead {
                    // Each node's size, less the dropped records in its range: from its base
                    // (a wrap's is its chain's), `size` records.
                    let mut before = vec![0u32; nodes.len() + 1];
                    for (i, n) in nodes.iter().enumerate() {
                        before[i + 1] = before[i] + u32::from(n.kind == DEAD);
                    }
                    let mut first: Vec<usize> = (0..nodes.len()).collect();
                    for (&b, &w) in &head {
                        let mut c = Some(w);
                        while let Some(w) = c {
                            first[w] = b;
                            c = link.get(&w).copied();
                        }
                    }
                    for i in 0..nodes.len() {
                        let f = first[i];
                        let e = (f + nodes[i].size as usize).min(nodes.len());
                        nodes[i].size -= before[e] - before[f];
                    }
                }
                // Each wrap before its base, outermost first; the wraps' own records are placed so.
                let mut order = Vec::with_capacity(nodes.len());
                for i in 0..nodes.len() {
                    if wraps.contains(&i) {
                        continue;
                    }
                    let mut c = head.get(&i).copied();
                    while let Some(w) = c {
                        order.push(w);
                        c = link.get(&w).copied();
                    }
                    order.push(i);
                }
                return Ok(order.into_iter().filter(|&i| nodes[i].kind != DEAD).map(|i| nodes[i].clone()).collect());
            }
            EMPTY => {
                if tier1.totals[usize::from(operand)] != 0 {
                    return refuse!(pc - 1, "tier 1 sends the file to tree-sitter");
                }
            }
            GOTO => pc = usize::from(operand),
            _ => return refuse!(pc - 1, "the stream program refuses here"),
        }
    }
}

/// A tier-2 program's words (decision 0087): `STOP`, `GOTO` and tier 2's own, each checked
/// (`validate_word`), then the program's rules (`validate`).
pub fn validate_tier2(program: &Words) -> Result<(), Error> {
    for (pc, &w) in program.words.iter().enumerate() {
        let (opcode, mode, operand) = split(w);
        match opcode {
            STOP => {
                if mode != 0 || operand != 0 {
                    return Err(Error::InvalidMode(pc));
                }
            }
            GOTO => {
                if mode != 0 || usize::from(operand) >= program.words.len() {
                    return Err(Error::InvalidTarget(pc));
                }
            }
            _ => validate_word(program, pc, opcode, mode, operand)?,
        }
    }
    validate(program)?;
    validate_mark_words(program)?;
    validate_word_kinds(program)?;
    validate_extras(program)?;
    validate_token_marks(program)?;
    validate_gap_patterns(program)?;
    validate_local_checks(program)?;
    validate_rules(program)
}

/// The gap patterns (`Words::gap_patterns`): rising by pattern, of no run. An element of an
/// extra (its code has the extra's bit) holds `with` the blank marks alone (`Records::blanks`):
/// an extra's code holds no other mark of its gap (`Records::extra`).
pub fn validate_gap_patterns(program: &Words) -> Result<(), Error> {
    let records = program.block_programs.first().and_then(|b| b.records.as_ref());
    let blanks = records.map_or(0u32, |r| r.blanks.iter().fold(0, |a, &(m, _)| a | 1 << m));
    let extras = program.extras.iter().fold(0u32, |a, &(b, _)| a | 1 << b);
    for (i, m) in program.gap_patterns.iter().enumerate() {
        let rising = i == 0 || program.gap_patterns[i - 1].order <= m.order;
        let plain = m.elems.iter().all(|e| !e.run && e.nests.is_empty() && e.ending.is_none());
        let extra_marks = m.elems.iter().filter(|e| e.with & extras != 0).all(|e| e.with & !extras & !blanks == 0);
        if !rising || !plain || !extra_marks || m.refuses {
            return Err(Error::InvalidOperand(i));
        }
    }
    Ok(())
}

/// The local checks (`Words::local_checks`): each a mask tier 2 reads, its ops' operands
/// earlier ones, a `Prev` of 1 to 63, at least one op. A token's check (`LocalCheck::token`)
/// reads no mask, and a delay under 64; only it has `fill`, `start` and `last`.
pub fn validate_local_checks(program: &Words) -> Result<(), Error> {
    let masks = program.block_programs.first().map_or(0, |b| b.masks.len());
    for (i, c) in program.local_checks.iter().enumerate() {
        let earlier = c.ops.iter().enumerate().all(|(k, op)| {
            let k = k as u16;
            let ok = match *op {
                LocalOp::Prev(a, s) => a < k && (1..=63).contains(&s),
                _ => op.operands().iter().all(|&a| a < k),
            };
            ok && (c.token || !op.token_only())
        });
        // A token's check that reads the tokens before it (`LocalOp::Ended`) reads the records,
        // which a region tier 2 splits (decision 0323) does not hold apart (decision 0428).
        let split = program.block_programs.first().and_then(|b| b.records.as_ref()).is_some_and(|r| r.readers.iter().any(|rd| !rd.parts.is_empty() && rd.split == crate::records::Split::Tier2));
        let ended = c.ops.contains(&LocalOp::Ended);
        if c.ops.is_empty() || !earlier || (!c.token && usize::from(c.mask) >= masks) || (c.token && (usize::from(c.delay) + usize::from(c.back) >= 64 || (ended && split))) || (!c.token && c.back != 0) {
            return Err(Error::InvalidOperand(i));
        }
    }
    Ok(())
}

/// The token marks (`Words::token_marks`): rising by mark, each a mark in the records' codes
/// (its bit gates it), with extras taken in the token step (they are skipped by their bit);
/// its elements rising by offset, the mark's own (0) among them, the words the program's.
pub fn validate_token_marks(program: &Words) -> Result<(), Error> {
    // A token mark past the code's (decision 0407) is a bit of `live`, which holds 64; one that
    // refuses is tested once a token by its code bit, so in the code.
    let marks = 64;
    let bits = program.block_programs.first().and_then(|b| b.records.as_ref()).map_or(0, |r| r.mark_bits());
    for (i, m) in program.token_marks.iter().enumerate() {
        if m.refuses && usize::from(m.order) >= bits {
            return Err(Error::InvalidOperand(i));
        }
        // Rising by order; a mark's entries together (decision 0416), one where it refuses (its
        // `TOKEN` test is the one entry's), and none an `unless` alone of a refusing mark.
        let rising = i == 0 || program.token_marks[i - 1].order < m.order || (program.token_marks[i - 1].order == m.order && !m.refuses && !program.token_marks[i - 1].refuses);
        let rising = rising && !(m.refuses && m.unless);
        // One run at most, after the mark's token, not last, of no `end`.
        let runs = m.elems.iter().filter(|e| e.run).count();
        let last = m.elems.last().is_some_and(|e| e.run);
        let rising = rising && runs <= 1 && !last && m.elems.iter().all(|e| !e.run || (e.offset > 0 && !e.end));
        let offsets = m.elems.windows(2).all(|w| w[0].offset < w[1].offset) && m.elems.iter().any(|e| e.offset == 0) && m.elems.first().is_some_and(|e| e.offset >= -1);
        // A `not` or an `end` only after the mark's token, but for the token before's `not`,
        // which has an `end` (no token before holds), and the mark's own token's `not`, a gap
        // mark's `unless before` (decision 0416: its mask gives the token's bytes); an `end`
        // only last after the mark's.
        let after = m.elems.iter().all(|e| (!e.not && !e.end) || e.offset > 0 || (e.offset == 0 && !e.end) || (e.offset == -1 && e.not && e.end)) && m.elems.iter().filter(|e| e.offset >= 0).rev().skip(1).all(|e| !e.end);
        let offsets = offsets && after;
        // A run's depths only on a run: each side of kinds and words, its counter below
        // `NESTS`, each counter with an opener and a closer (Q339).
        let nests = m.elems.iter().all(|e| {
            let counters = |open: bool| e.nests.iter().filter(|n| n.open == open).fold(0u8, |a, n| a | 1 << n.counter);
            (e.run || (e.nests.is_empty() && e.ending.is_none()))
                && e.ending.as_ref().is_none_or(|x| !x.not && !x.end && !x.run && x.nests.is_empty() && x.ending.is_none())
                && e.nests.iter().all(|n| usize::from(n.counter) < crate::program::NESTS && !n.test.not && !n.test.end && !n.test.run && n.test.nests.is_empty())
                && counters(true) == counters(false)
        });
        let words = m.elems.iter().flat_map(|e| e.words.iter().chain(e.nests.iter().flat_map(|n| &n.test.words))).all(|&s| usize::from(s) < program.strings.len());
        let words = words && nests;
        // A grammar with no extras has nothing to skip between the mark's tokens.
        if !rising || !offsets || !words || usize::from(m.order) >= marks {
            return Err(Error::InvalidOperand(i));
        }
    }
    Ok(())
}

/// The kind of the extra a record's `code` is (`Words::extras`), or `None` for a grammar token:
/// an extra has the first entry's bit; its kind is the last later entry whose bit it has, or the
/// first entry's.
pub fn extra_kind(extras: &[(u8, u16)], code: u32) -> Option<u16> {
    let has = |b: u8| code >> (crate::records::MARK_SHIFT + u32::from(b)) & 1 == 1;
    let &(first, kind) = extras.first()?;
    if !has(first) {
        return None;
    }
    Some(extras[1..].iter().rev().find(|&&(b, _)| has(b)).map_or(kind, |&(_, k)| k))
}

/// Whether token mark `m` holds at the record before `next` (`Words::token_marks`): the element
/// at -1 at the record before, the others in turn from the token on, each at the next record
/// that is no extra (`extra_bit`, the code's bit; none for a grammar without extras); a run
/// (decision 0416) any number of records it takes, then the elements after it from any of them
/// on.
pub fn token_mark_holds(m: &TokenMark, codes: &[u32], next: usize, extra_bit: Option<u32>) -> bool {
    let is_extra = |c: u32| extra_bit.is_some_and(|b| c >> (crate::records::MARK_SHIFT + b) & 1 == 1);
    let Some(here) = next.checked_sub(1) else { return false };
    // The record after `j` that is no extra; `None` past the last.
    let after = |j: usize| (j + 1..codes.len()).find(|&k| !is_extra(codes[k]));
    for e in m.elems.iter().filter(|e| e.offset < 0) {
        let at = (0..here).rev().find(|&j| !is_extra(codes[j]));
        if !e.holds(at.map(|j| codes[j])) {
            return false;
        }
    }
    let rest: Vec<&crate::program::TokenElem> = m.elems.iter().filter(|e| e.offset >= 0).collect();
    token_follows(&rest, Some(here), None, codes, &after)
}

/// Whether gap pattern `m`'s entry holds at record `here`, the token's (`Words::gap_patterns`):
/// each element at its offset from it among the records as the lexer wrote them, extras
/// included; one past either end, an element with `end` holds there (a `~` before the first).
/// The element's `starting` tests the record's first byte, as the lean lexer's `asi` reads it.
pub fn gap_pattern_holds(m: &TokenMark, codes: &[u32], starts: &[u32], bytes: &[u8], here: usize, extra_bit: u32) -> bool {
    // `since`: from the grammar token before (none before the file's first: index -1), each
    // before the token.
    let g = || (0..here.min(codes.len())).rev().find(|&i| codes[i] & extra_bit == 0).map_or(0, |g| g + 1);
    m.elems.iter().all(|e| match if e.since { (g() + e.offset as usize).checked_sub(1).filter(|&j| j < here) } else { here.checked_add_signed(isize::from(e.offset)).filter(|&j| j < codes.len()) } {
        None if e.since && g() + e.offset as usize > here => false,
        None => e.holds(None),
        Some(j) => e.holds(Some(codes[j])) && starts.get(j).and_then(|&s| bytes.get(s as usize)).is_some_and(|&b| e.first_holds(b)),
    })
}

/// Whether `rest` holds from record `at` on (`None`: past the last), each element at the next
/// record `after` gives (`token_mark_holds`); `last` the record the element before took.
fn token_follows(rest: &[&crate::program::TokenElem], at: Option<usize>, last: Option<usize>, codes: &[u32], after: &dyn Fn(usize) -> Option<usize>) -> bool {
    let Some((e, tail)) = rest.split_first() else { return true };
    if e.run {
        // Its depths (Q339): the rest only where each is 0; a closer at 0 ends the run.
        // Its `ending` (Q342): the rest only where its last record is one the test takes.
        let mut depth = [0u32; crate::program::NESTS];
        let (mut at, mut last) = (at, last);
        loop {
            let ends = e.ending.as_ref().is_none_or(|x| last.is_some_and(|l| x.takes(codes[l])));
            if depth == [0; crate::program::NESTS] && ends && token_follows(tail, at, last, codes, after) {
                return true;
            }
            match at {
                Some(j) if e.holds(Some(codes[j])) => match crate::program::nest_step(&e.nests, depth, codes[j]) {
                    Some(d) => {
                        depth = d;
                        last = Some(j);
                        at = after(j);
                    }
                    None => return false,
                },
                _ => return false,
            }
        }
    }
    match at {
        // The file's end: an `end` element, the last.
        None => e.holds(None),
        Some(j) => e.holds(Some(codes[j])) && token_follows(tail, after(j), Some(j), codes, after),
    }
}

/// The extras a `TOKEN` takes (`Words::extras`): rising by bit, each a mark the records' codes
/// hold.
pub fn validate_extras(program: &Words) -> Result<(), Error> {
    let marks = program.block_programs.first().and_then(|b| b.records.as_ref()).map_or(0, |r| r.marks.len());
    for (i, &(b, _)) in program.extras.iter().enumerate() {
        let rising = i == 0 || program.extras[i - 1].0 < b;
        if !rising || usize::from(b) >= marks {
            return Err(Error::InvalidOperand(i));
        }
    }
    Ok(())
}

/// The marks' words (`Words::mark_words`): rising by mark, each a mark a `MARK` tests (or a
/// token mark that refuses, which `TOKEN` tests) and a string of the records' words.
pub fn validate_mark_words(program: &Words) -> Result<(), Error> {
    let records = program.block_programs.first().and_then(|b| b.records.as_ref());
    let tested: Vec<u8> = program.words.iter().map(|&w| split(w)).filter(|w| w.0 == MARK).map(|w| w.1 & crate::program::MARK_ORDER).collect();
    for (i, &(k, s)) in program.mark_words.iter().enumerate() {
        let rising = i == 0 || program.mark_words[i - 1].0 < k;
        let refusing = program.token_marks.iter().any(|m| m.refuses && m.order == k);
        let ok = rising && k <= crate::program::MARK_ORDER && (tested.contains(&k) || refusing) && usize::from(s) < program.strings.len() && records.is_some_and(|r| r.words == program.strings);
        if !ok {
            return Err(Error::InvalidOperand(i));
        }
    }
    Ok(())
}

/// The keywords' leaf kinds (`Words::word_kinds`): rising by string, each a string of the
/// program's (the records' words, `LEAF`'s check) and a kind that names no level's word.
pub fn validate_word_kinds(program: &Words) -> Result<(), Error> {
    for (i, &(s, k)) in program.word_kinds.iter().enumerate() {
        let rising = i == 0 || program.word_kinds[i - 1].0 < s;
        if !rising || usize::from(s) >= program.strings.len() || k == WORD_KIND || k == REMEMBERED {
            return Err(Error::InvalidOperand(i));
        }
    }
    Ok(())
}

/// Tier 2's rules (`Words::rules`), when it has them: from place 0, rising, within the words;
/// every word goes on by position within its own rule (its next word, a test's two ways), and
/// a `PUSH` returns into it. A call's jump goes to its callee, any rule; any other jump (a
/// `GOTO`, a switch's targets) stays within its rule. So a rule is a function, entered by calls
/// alone, as the lean parser's methods are.
pub fn validate_rules(program: &Words) -> Result<(), Error> {
    let (rules, words) = (&program.rules, &program.words);
    if rules.is_empty() {
        return Ok(());
    }
    let n = words.len();
    if rules[0] != 0 || rules.windows(2).any(|w| w[0] >= w[1]) || rules.last().is_some_and(|&r| usize::from(r) >= n) {
        return Err(Error::InvalidTarget(0));
    }
    let rule_of = |pc: usize| rules.partition_point(|&s| usize::from(s) <= pc) - 1;
    let target = |t: u16| usize::from(t);
    for pc in 0..n {
        let (op, mode, operand) = split(words[pc]);
        let after_push = pc > 0 && split(words[pc - 1]).0 == PUSH;
        // A jump that is not a call's: within its rule.
        let jumps: Vec<usize> = match op {
            GOTO if !after_push => vec![target(operand)],
            SWITCH => {
                let s = &program.switches[usize::from(operand)];
                s.all_targets().filter(|&t| t != REFUSE).map(target).collect()
            }
            _ => vec![],
        };
        if jumps.iter().any(|&t| t < n && rule_of(t) != rule_of(pc)) {
            return Err(Error::InvalidTarget(pc));
        }
        let next: Vec<usize> = match op {
            STOP | ACCEPT | RETURN | GOTO | SWITCH => vec![],
            PUSH => {
                if split(*words.get(pc + 1).ok_or(Error::InvalidTarget(pc))?).0 != GOTO {
                    return Err(Error::InvalidTarget(pc));
                }
                vec![target(operand), target(operand) + 1, pc + 1]
            }
            TOKEN => vec![pc + 1],
            WORD_IS if mode != 0 => vec![pc + 1, pc + 2],
            FLAG if mode == 2 => vec![pc + 1, pc + 2],
            MARK | ROW => vec![pc + 1, pc + 2],
            _ => vec![pc + 1],
        };
        if next.iter().any(|&t| t < n && rule_of(t) != rule_of(pc)) {
            if std::env::var_os("RERAIL_DEBUG_STREAM").is_some() {
                eprintln!("rules: {pc} (op {op} m={mode} {operand}, rule {}) goes to {next:?} (rules {:?}); the word before: {:?}", rule_of(pc), next.iter().map(|&t| rule_of(t)).collect::<Vec<_>>(), pc.checked_sub(1).map(|p| split(words[p])));
            }
            return Err(Error::InvalidTarget(pc));
        }
    }
    Ok(())
}
