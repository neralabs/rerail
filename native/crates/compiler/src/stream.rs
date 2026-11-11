//! Tier 2's instructions (decision 0087): the transducer's words over tier 1's token stream, as
//! simdjson's stage 2 runs over its structural indexes. A program is a state machine: each
//! state takes a token (`TOKEN`), switches on its byte (`SWITCH`), writes nodes, and jumps to the
//! next state (`GOTO`). Its verifier refuses a cycle with no `TOKEN`, so its one loop is over the
//! stream. This file holds the switch tables, the verifier and the interpreter; `native_ir` and
//! Cranelift lower the same words.

use crate::program::*;
use crate::Error;
use std::collections::{HashMap, HashSet};

pub use crate::stream_ir::{ByteSwitch, DEAD, GONE, KEEP, REFUSE, REMEMBERED, Reading, Readings};

/// Whether a word is one of tier 2's.
pub fn is_stream(opcode: u8) -> bool {
    (TOKEN..=FLAG).contains(&opcode)
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
        TOKEN => mode == 0 && usize::from(operand) < extracts && records.is_some_and(|r| r.starts == operand),
        ACCEPT => mode == 0 && operand == 0,
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
        LEAF => mode <= 3 && operand != REMEMBERED,
        // Mark `m % 32` (its order), its bit in mask `operand`; `m & 32`: the file's end has it.
        MARK => mode < 64 && usize::from(operand) < masks,
        ROW => mode == 0 && usize::from(operand) < masks,
        // `m=1`: the node a wrap moved; `m=2`: the node made last at this level (`relabel`);
        // `m=3` (a field): the node that one, a wrap, holds first. `RETAG m=3`: that node and
        // the nodes inside it, read by reading `operand` (`reread`).
        RETAG => (mode <= 2 && (mode != 0 || operand != REMEMBERED)) || (mode == 3 && usize::from(operand) < program.readings.maps.len()),
        NODE_FIELD => mode <= 3 && operand != 0,
        // `m=1`: the open node's two leaves, one (`fold`).
        MERGE => mode <= 1 && operand != REMEMBERED,
        WRAP => mode == 0 && operand != REMEMBERED,
        REMEMBER => mode == 0 && operand != 0 && operand != REMEMBERED,
        FLAG => mode <= 2 && operand < 16,
        CLOSE => mode <= 1 && operand == 0,
        PUSH => mode == 0 && usize::from(operand) < words,
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
    if program.switches.iter().any(|s| s.targets.len() != 256 || !s.targets.iter().all(|&t| target(t)) || !target(s.end)) {
        return Err(Error::InvalidTarget(0));
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
    // A state: `TOKEN`, a `MARK` and its way (`GOTO` or `STOP`) per mark it tests, `SWITCH`.
    let state = |p: usize| {
        let mut q = p + 1;
        while words.get(q).is_some_and(|&w| split(w).0 == MARK) {
            if words.get(q + 1).is_none_or(|&w| !matches!(split(w).0, GOTO | STOP)) {
                return false;
            }
            q += 2;
        }
        split(words[p]).0 == TOKEN && words.get(q).is_some_and(|&w| split(w).0 == SWITCH)
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
    // A `RETURN` pops a call, and from a `PUSH` no `RETURN` is reached before a `TOKEN` (below):
    // a cycle with no `TOKEN` pushes nothing, so it turns at most once per call made, and calls
    // are made only between tokens. The returns of a file are at most its calls.
    let step = |pc: usize, cut_return: bool| -> Vec<usize> {
        let (opcode, mode, operand) = split(words[pc]);
        match opcode {
            STOP | ACCEPT | TOKEN => vec![],
            RETURN if cut_return => vec![],
            GOTO => vec![usize::from(operand)],
            SWITCH => {
                let s = &program.switches[usize::from(operand)];
                let mut out: Vec<usize> = s.targets.iter().chain([&s.end]).filter(|&&t| t != REFUSE).map(|&t| usize::from(t)).collect();
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
    // From a `PUSH`, no `RETURN` before a `TOKEN`.
    for (pc, &w) in words.iter().enumerate() {
        if split(w).0 != PUSH {
            continue;
        }
        let mut seen = vec![false; words.len()];
        let mut todo = vec![pc];
        while let Some(q) = todo.pop() {
            for n in step(q, false) {
                if n >= words.len() {
                    return Err(Error::InvalidTarget(q));
                }
                if split(words[n]).0 == RETURN {
                    return Err(Error::NonProgressingCycle(n));
                }
                if !std::mem::replace(&mut seen[n], true) {
                    todo.push(n);
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
    Ok(())
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

/// Why a file went to tree-sitter: the word that refused it, and what it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub pc: usize,
    pub why: &'static str,
}

/// The interpreter of a tier-2 program over tier 1's run of `bytes`: a token is tier 1's
/// record (`records`), its first byte, its last byte and its code; `EMPTY` reads the totals.
/// It executes the words only; the tables it reads are the program's switches, strings and
/// checks.
pub fn run(program: &Words, bytes: &[u8], tier1: &crate::block::BlockRun) -> Result<Vec<Node>, Refused> {
    run_seen(program, bytes, tier1, &mut |_| {})
}

/// `run`, with `seen` told the place of each word it runs, in order: the words' profile (which
/// states follow which, how often), for laying out the generated code.
pub fn run_seen(program: &Words, bytes: &[u8], tier1: &crate::block::BlockRun, seen: &mut dyn FnMut(usize)) -> Result<Vec<Node>, Refused> {
    let words = &program.words;
    let stream = |op: u8, m: u8| words.iter().map(|&w| split(w)).find(|&(o, mm, _)| o == op && mm == m).map(|(_, _, x)| usize::from(x));
    let none: Vec<u32> = Vec::new();
    // A kind as a word names it: a level's word's (`REMEMBERED`), none where it has none.
    let kind_of = |k: u16, word: u16| if k != REMEMBERED { Some(k) } else if word != 0 { Some(word) } else { None };
    let starts: &[u32] = stream(TOKEN, 0).map_or(&none, |e| &tier1.positions[e]);
    let records = program.block_programs.first().and_then(|b| b.records.as_ref());
    let lasts: &[u32] = records.map_or(&none, |r| &tier1.positions[usize::from(r.lasts)]);
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
    let (mut need, mut live, mut pending, mut keep) = (true, 0u32, 0usize, 0u32);
    let block = program.block_programs.first();
    let mask_bit = |m: u16, at: u32| -> bool {
        block.is_some_and(|b| {
            let words = &tier1.outputs[b.mask_outputs()[usize::from(m)]];
            words.get(at as usize / 64).is_some_and(|w| w >> (at % 64) & 1 == 1)
        })
    };
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
    let refuse = |pc: usize, why: &'static str| Err(Refused { pc, why });
    loop {
        seen(pc);
        let (opcode, mode, operand) = split(words[pc]);
        pc += 1;
        match opcode {
            TOKEN if !need => {}
            TOKEN => {
                if at_end {
                    return refuse(pc - 1, "a token past the stream's end");
                }
                need = false;
                live = u32::MAX;
                last_end = end;
                // The record pass checked the records (the totals' flag, `EMPTY`): as many lasts
                // as starts, each token after the one before.
                if next == starts.len().min(lasts.len()) {
                    at_end = true;
                } else {
                    (start, end) = (starts[next], starts[next]);
                    rec_end = lasts[next] + 1;
                    code = tier1.codes.get(next).copied().unwrap_or(0);
                    next += 1;
                }
            }
            SPAN => {
                // The token's bytes taken: its record's end, or a length (a one-byte token),
                // which must be its record's.
                let e = if mode == 0 { start + u32::from(operand) } else { rec_end };
                if e != rec_end {
                    return refuse(pc - 1, "a token's length not its record's");
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
                    // Zeros past the file, as every executor reads them.
                    if at >= rec_end as usize { REFUSE } else { s.targets[usize::from(bytes.get(at).copied().unwrap_or(0))] }
                };
                if t == REFUSE {
                    return refuse(pc - 1, "no state takes the token");
                }
                pc = usize::from(t);
            }
            WORD_IS => {
                // The record's word: its code names the string.
                let same = code >> crate::records::KIND_BITS == u32::from(operand) + 1;
                match (mode, same) {
                    (_, true) => {}
                    // A test: the next word is the keyword's way; a token that is not the word skips it.
                    (1, false) => pc += 1,
                    _ => return refuse(pc - 1, "not the keyword"),
                }
            }
            MARK => {
                let k = u32::from(mode % 32);
                let here = if at_end { mode & 32 != 0 } else { mask_bit(operand, start) };
                if live >> k & 1 == 1 && here {
                    // Taken: it and the marks before it are done at this token.
                    live &= !(u32::MAX >> (31 - k));
                } else {
                    pc += 1;
                }
            }
            ROW => {
                if !mask_bit(operand, start) {
                    pc += 1;
                }
            }
            RETAG if mode == 1 => {
                let Some(k) = kind_of(operand, word) else { return refuse(pc - 1, "a word's kind, and no word") };
                nodes[moved].kind = k;
            }
            RETAG if mode == 3 => {
                // The node made last at this level and the nodes inside it, the records from its
                // base to the pending extras, each base's chain of wraps first (outermost first,
                // as `ACCEPT` places them): read, a kind each.
                let Some(Some((r, base))) = lasts_made.last().copied() else { return refuse(pc - 1, "a reread with no node before it") };
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
                        let Some(to) = way else { return refuse(pc - 1, "a node the reading does not name") };
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
                                let Some(k) = kind_of(k, word) else { return refuse(pc - 1, "a word's kind, and no word") };
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
                let Some(Some((r, base))) = lasts_made.last().copied() else { return refuse(pc - 1, "a relabel with no node before it") };
                if !wraps.contains(&r) || link.contains_key(&r) {
                    return refuse(pc - 1, "an inner field on a node that is no wrap of a plain node");
                }
                nodes[base].field = operand;
            }
            RETAG | NODE_FIELD if mode == 2 => {
                let Some(Some((r, _))) = lasts_made.last().copied() else { return refuse(pc - 1, "a relabel with no node before it") };
                if opcode == RETAG {
                    let Some(k) = kind_of(operand, word) else { return refuse(pc - 1, "a word's kind, and no word") };
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
                let Some(&at) = open.last() else { return refuse(pc - 1, "a retag with no open node") };
                nodes[at].kind = operand;
            }
            WRAP => {
                let Some(Some((r, base))) = lasts_made.last().copied() else { return refuse(pc - 1, "a wrap with no node before it") };
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
                let Some(&o) = open.last() else { return refuse(pc - 1, "a fold with no open node") };
                if nodes.len() != o + 3 || pending != 0 || nodes[o + 1].size != 1 || nodes[o + 2].size != 1 {
                    return refuse(pc - 1, "a fold of no two leaves");
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
                    return refuse(pc - 1, "a merge with no leaf before it");
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
                let Some(at) = open.pop() else { return refuse(pc - 1, "a close with no open node") };
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
            PUSH => returns.push((operand, word, flags)),
            REMEMBER => word = operand,
            FLAG if mode == 0 => flags |= 1 << operand,
            FLAG if mode == 1 => flags &= !(1 << operand),
            FLAG => {
                if flags & (1 << operand) == 0 {
                    pc += 1;
                }
            }
            RETURN => {
                let Some((p, w, fl)) = returns.pop() else { return refuse(pc - 1, "a return with no call") };
                (word, flags) = (w, fl);
                pc = usize::from(p) + usize::from(mode);
            }
            ACCEPT => {
                if !at_end || open.len() != 1 || !returns.is_empty() {
                    return refuse(pc - 1, "the file ends inside a node");
                }
                let at = open[0];
                nodes[at].end = bytes.len() as u32;
                nodes[at].size = (nodes.len() - at) as u32;
                if nodes.iter().any(|n| n.kind != DEAD && n.field != 0 && program.readings.provisional.contains(&n.field)) {
                    return refuse(pc - 1, "a provisional field at the file's end");
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
                    return refuse(pc - 1, "tier 1 sends the file to tree-sitter");
                }
            }
            GOTO => pc = usize::from(operand),
            _ => return refuse(pc - 1, "the stream program refuses here"),
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
    validate(program)
}
