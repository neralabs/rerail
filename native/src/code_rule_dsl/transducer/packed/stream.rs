//! Tier 2's instructions (decision 0087): the transducer's words over tier 1's token stream, as
//! simdjson's stage 2 runs over its structural indexes. A program is a state machine: each
//! state takes a token (`TOKEN`), switches on its byte (`SWITCH`), writes nodes, and jumps to the
//! next state (`GOTO`). Its verifier refuses a cycle with no `TOKEN`, so its one loop is over the
//! stream. This file holds the switch tables, the verifier and the interpreter; `native_ir` and
//! Cranelift lower the same words.

use super::*;

/// A target that refuses: the file goes to tree-sitter.
pub(in crate::code_rule_dsl) const REFUSE: u16 = u16::MAX;

/// `SWITCH`'s table: a target per value of the token's byte at `offset`, and one for the end
/// of the stream. `REFUSE` refuses.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct ByteSwitch {
    pub(super) offset: u8,
    pub(super) targets: Vec<u16>,
    pub(super) end: u16,
}

impl ByteSwitch {
    pub(super) fn refusing(offset: u8) -> Self {
        Self { offset, targets: vec![REFUSE; 256], end: REFUSE }
    }
}

/// Whether a word is one of tier 2's.
pub(super) fn is_stream(opcode: u8) -> bool {
    (TOKEN..=ROW).contains(&opcode)
}

/// The operand rules of tier 2's words, and of the words a tier-2 program may hold besides
/// (`STOP`, `GOTO`).
pub(super) fn validate_word(program: &Program, pc: usize, opcode: u8, mode: u8, operand: u16) -> Result<(), Error> {
    let words = program.words.len();
    let block = program.block_programs.first();
    let extracts = block.map_or(0, |b| b.extracts.len());
    let totals = block.map_or(0, |b| b.totals.len());
    let masks = block.map_or(0, |b| b.masks.len());
    let ok = match opcode {
        TOKEN => mode == 0 && usize::from(operand) < extracts,
        ACCEPT => mode == 0 && operand == 0,
        EMPTY => mode == 0 && usize::from(operand) < totals,
        SWITCH => mode == 0 && usize::from(operand) < program.switches.len(),
        // `m=1`: a test, which skips the next word where the token is not the string.
        WORD_IS => mode <= 1 && program.strings.get(usize::from(operand)).is_some_and(|s| !s.is_empty() && s.len() <= 8),
        OPEN | NODE_SUPER => mode == 0,
        // `m=1`: an extra's leaf, deferred.
        LEAF => mode <= 1,
        // Mark `m % 32` (its order), its bit in mask `operand`; `m & 32`: the file's end has it.
        MARK => mode < 64 && usize::from(operand) < masks,
        ROW => mode == 0 && usize::from(operand) < masks,
        NODE_FIELD => mode == 0 && operand != 0,
        CLOSE => mode <= 1 && operand == 0,
        PUSH => mode == 0 && usize::from(operand) < words,
        RETURN => mode <= 1 && operand == 0,
        SPAN => (mode == 0 && operand != 0) || (mode == 1 && usize::from(operand) < masks),
        _ => return Err(Error::InvalidOpcode(pc)),
    };
    if ok { Ok(()) } else { Err(Error::InvalidOperand(pc)) }
}

/// A tier-2 program's whole-program rules: only tier 2's words, `STOP` and `GOTO`; switch
/// targets in range; a return lands on a state's `TOKEN` (or the `SWITCH` after it); and no
/// cycle without a `TOKEN` (decision 0087 §5).
pub(super) fn validate(program: &Program) -> Result<(), Error> {
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
    // One stream of starts and one of last bytes: every `TOKEN` names one extract, and every
    // `SPAN m=1` one mask.
    let named = |op: u8, m: u8| -> Vec<u16> {
        let mut v: Vec<u16> = words.iter().map(|&w| split(w)).filter(|&(o, mm, _)| o == op && mm == m).map(|(_, _, x)| x).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (starts, lasts) = (named(TOKEN, 0), named(SPAN, 1));
    if starts.len() > 1 || lasts.len() > 1 {
        return Err(Error::InvalidOperand(0));
    }
    // Every cycle passes a `TOKEN`: with `TOKEN`'s edges cut, the graph is acyclic.
    let successors = |pc: usize| -> Vec<usize> {
        let (opcode, mode, operand) = split(words[pc]);
        match opcode {
            STOP | ACCEPT | TOKEN => vec![],
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
            MARK | ROW => vec![pc + 1, pc + 2],
            _ => vec![pc + 1],
        }
    };
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
pub(in crate::code_rule_dsl) struct Node {
    pub(super) kind: u16,
    pub(super) field: u16,
    /// Supertypes, outermost first.
    pub(super) supers: Vec<u16>,
    pub(super) start: u32,
    /// One past its last byte.
    pub(super) end: u32,
    /// It and every node inside it.
    pub(super) size: u32,
}

/// Why a file went to tree-sitter: the word that refused it, and what it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Refused {
    pub(super) pc: usize,
    pub(super) why: &'static str,
}

/// The first set bit of `words` at `from` or after, a word at a time.
fn next_bit(words: &[u64], from: usize) -> Option<u32> {
    let (mut w, shift) = (from / 64, from % 64);
    let mut bits = words.get(w)? & (u64::MAX << shift);
    while bits == 0 {
        w += 1;
        bits = *words.get(w)?;
    }
    Some((64 * w) as u32 + bits.trailing_zeros())
}

/// The interpreter of a tier-2 program over tier 1's run of `bytes`: the extract `TOKEN` names
/// holds every token's first byte, the mask `SPAN m=1` names the last byte of each token whose
/// arm reads it (taken in order: the next set bit after the last one taken), and `EMPTY` reads
/// the totals. It executes the words only; the tables it reads
/// are the program's switches, strings and checks.
pub(in crate::code_rule_dsl) fn run(program: &Program, bytes: &[u8], tier1: &super::block::BlockRun) -> Result<Vec<Node>, Refused> {
    let words = &program.words;
    let stream = |op: u8, m: u8| words.iter().map(|&w| split(w)).find(|&(o, mm, _)| o == op && mm == m).map(|(_, _, x)| usize::from(x));
    let none: Vec<u32> = Vec::new();
    let starts: &[u32] = stream(TOKEN, 0).map_or(&none, |e| &tier1.positions[e]);
    let no_words: Vec<u64> = Vec::new();
    let lasts: &[u64] = match (stream(SPAN, 1), program.block_programs.first()) {
        (Some(m), Some(block)) => &tier1.outputs[block.mask_outputs()[m]],
        _ => &no_words,
    };
    let mut pc = 0usize;
    let (mut next, mut taken, mut start, mut end, mut limit, mut last_end, mut at_end) = (0usize, 0usize, 0u32, 0u32, 0u32, 0u32, false);
    // `need`: the token's bytes were taken (`SPAN`), so `TOKEN` takes the next; an arm of a mark
    // takes none, and the next state switches on the same token. `live`: the marks not yet
    // taken or passed at this token. `pending`: the deferred extras' leaves at the table's end.
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
    let mut returns: Vec<u16> = Vec::new();
    let refuse = |pc: usize, why: &'static str| Err(Refused { pc, why });
    loop {
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
                if next == starts.len() {
                    at_end = true;
                } else {
                    let s = starts[next];
                    if s < end {
                        return refuse(pc - 1, "a token inside the one before");
                    }
                    (start, end) = (s, s);
                    next += 1;
                    limit = starts.get(next).map_or(bytes.len() as u32, |&n| n);
                }
            }
            SPAN => {
                // The token's end: a length (a one-byte token, a keyword), or its last byte, the
                // next of the stream's last bytes.
                let e = if mode == 0 {
                    start + u32::from(operand)
                } else {
                    let Some(l) = next_bit(lasts, taken) else { return refuse(pc - 1, "a token with no last byte") };
                    taken = l as usize + 1;
                    if l < start {
                        return refuse(pc - 1, "a last byte before its token");
                    }
                    l + 1
                };
                if e > limit {
                    return refuse(pc - 1, "a token past the next one");
                }
                end = e;
                need = true;
            }
            SWITCH => {
                live = 0;
                let s = &program.switches[usize::from(operand)];
                let t = if at_end {
                    s.end
                } else {
                    let at = start as usize + usize::from(s.offset);
                    if at >= limit as usize { REFUSE } else { s.targets[usize::from(bytes[at])] }
                };
                if t == REFUSE {
                    return refuse(pc - 1, "no state takes the token");
                }
                pc = usize::from(t);
            }
            WORD_IS => {
                let same = bytes[start as usize..end as usize] == program.strings[usize::from(operand)][..];
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
            OPEN => {
                pending = 0;
                open.push(nodes.len());
                nodes.push(Node { kind: operand, field: 0, supers: vec![], start, end: 0, size: 0 });
            }
            CLOSE => {
                let Some(at) = open.pop() else { return refuse(pc - 1, "a close with no open node") };
                nodes[at].end = if mode == 1 { end } else if pending > 0 { keep } else { last_end };
                nodes[at].size = (nodes.len() - pending - at) as u32;
            }
            LEAF => {
                if mode == 1 && pending == 0 {
                    keep = last_end;
                }
                pending = if mode == 1 { pending + 1 } else { 0 };
                nodes.push(Node { kind: operand, field: 0, supers: vec![], start, end, size: 1 });
            }
            NODE_FIELD => nodes.last_mut().expect("a node before its field").field = operand,
            NODE_SUPER => nodes.last_mut().expect("a node before its supertype").supers.push(operand),
            PUSH => returns.push(operand),
            RETURN => {
                let Some(p) = returns.pop() else { return refuse(pc - 1, "a return with no call") };
                pc = usize::from(p) + usize::from(mode);
            }
            ACCEPT => {
                if !at_end || open.len() != 1 || !returns.is_empty() || next_bit(lasts, taken).is_some() {
                    return refuse(pc - 1, "the file ends inside a node");
                }
                let at = open[0];
                nodes[at].end = bytes.len() as u32;
                nodes[at].size = (nodes.len() - at) as u32;
                return Ok(nodes);
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

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
