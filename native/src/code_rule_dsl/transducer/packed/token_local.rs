//! Which of tier 1's values are token-level checks: the facts the compiler needs to move a
//! value out of the per-block program, into tier 2's per-token one (doc 25).
//!
//! For each operation of a block program, three facts:
//!
//! - **Local**: its bit at a byte is a function of a bounded window. Its leaves are byte
//!   tests (and other values of the position alone: constants, `within`, phases) and carried
//!   values (`toggle`, `escaped`, `fill`, `back_fill`, a fixed point's members), each read at
//!   fixed offsets. A carried value is never local: it is a leaf of the values that read it.
//! - **Anchored**: its bits can only be set where an anchor's are (a token kind's first bytes,
//!   say), at a fixed shift. `a & b` is anchored where either side is, `a | b` and `a ^ b`
//!   where both are, `prev(a, k)` where `a` is, `k` later.
//! - **Readers**: the outputs it reaches, and the carried values that read it through local
//!   values only. A carried reader needs the value at every byte.
//!
//! All offsets are in the operation's own frame: bit `q` of its word. A value's name has a
//! delay `d` (`bitlex_compile::Named`): its bit at byte `p` is bit `p + d`. The facts are
//! claims, and `check` tests them against the interpreter: an anchored value's bits fall on
//! its anchor's, and a local value at each bit, made from its leaves alone (`eval`), equals
//! the interpreter's word.

use std::collections::{BTreeSet, HashMap};

use super::block::{BlockOp, BlockProgram};

/// One operation's facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Fact {
    /// `None` for a carried value.
    pub local: Option<Local>,
    /// Each `(anchor, shift)`: wherever this value's bit `q` is set, anchor `anchor`'s bit
    /// `q - shift` is (`anchors` as given to `facts`).
    pub anchors: BTreeSet<(usize, i32)>,
    /// The outputs (indexes of `BlockProgram::outputs`) it reaches, through any values.
    pub outputs: BTreeSet<usize>,
    /// The carried values that read it through local values only.
    pub carried_readers: BTreeSet<u16>,
}

/// A local value's window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Local {
    /// Each `(leaf, offset)`: the leaf's bit `q + offset` is read for this value's bit `q`.
    pub leaves: BTreeSet<(u16, i32)>,
    /// The local operations from the leaves to this value, itself included.
    pub cone: BTreeSet<u16>,
}

/// Whether operation `x` is carried: its bit depends on bytes any distance away. A fixed
/// point's members are: their words are the settled rounds'.
pub(super) fn carried(program: &BlockProgram, members: &[bool], x: usize) -> bool {
    members[x] || matches!(program.ops[x], BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..) | BlockOp::BackFill(..) | BlockOp::Carry(_))
}

/// Whether operation `x` is a leaf of position alone: a byte test, a constant, or a value of
/// the position (`within`, a phase, a plane of the position).
pub(super) fn positional(op: &BlockOp) -> bool {
    matches!(op, BlockOp::Eq(_) | BlockOp::Class { .. } | BlockOp::Pair { .. } | BlockOp::Const(_) | BlockOp::Within(_) | BlockOp::Phase(..) | BlockOp::Position(_))
}

/// Each fixed point's members (`BlockProgram::rounds`).
pub(super) fn members(program: &BlockProgram) -> Vec<bool> {
    let mut m = vec![false; program.ops.len()];
    for g in super::block::groups(&program.ops, &program.rounds).expect("a verified program") {
        for x in g.members {
            m[x] = true;
        }
    }
    m
}

/// The facts of every operation of `program`, with `anchors` (operations) as the anchors.
pub(super) fn facts(program: &BlockProgram, anchors: &[u16]) -> Vec<Fact> {
    let n = program.ops.len();
    let members = members(program);
    let mut out: Vec<Fact> = vec![Fact::default(); n];
    // Forward: local windows and anchors.
    for x in 0..n {
        let op = program.ops[x];
        let is_carried = carried(program, &members, x);
        let leaf = |y: u16, o: i32| -> Local {
            let y = usize::from(y);
            match &out[y].local {
                Some(l) if !carried(program, &members, y) && !positional(&program.ops[y]) => {
                    Local { leaves: l.leaves.iter().map(|&(z, k)| (z, k + o)).collect(), cone: l.cone.clone() }
                }
                _ => Local { leaves: [(y as u16, o)].into(), cone: BTreeSet::new() },
            }
        };
        let local = if is_carried {
            None
        } else if positional(&op) {
            Some(Local { leaves: [(x as u16, 0)].into(), cone: BTreeSet::new() })
        } else {
            let mut l = Local::default();
            let shift = if let BlockOp::Prev(_, k) = op { -i32::from(k) } else { 0 };
            for y in op.operands() {
                let part = leaf(y, shift);
                l.leaves.extend(part.leaves);
                l.cone.extend(part.cone);
            }
            l.cone.insert(x as u16);
            Some(l)
        };
        let a = |y: u16| &out[usize::from(y)].anchors;
        let mut anchored: BTreeSet<(usize, i32)> = match op {
            BlockOp::And(p, q) => a(p).union(a(q)).copied().collect(),
            BlockOp::AndNot(p, _) => a(p).clone(),
            BlockOp::Or(p, q) | BlockOp::Xor(p, q) => a(p).intersection(a(q)).copied().collect(),
            BlockOp::Prev(p, k) => a(p).iter().map(|&(i, s)| (i, s + i32::from(k))).collect(),
            _ => BTreeSet::new(),
        };
        for (i, _) in anchors.iter().enumerate().filter(|&(_, &v)| usize::from(v) == x) {
            anchored.insert((i, 0));
        }
        out[x].local = local;
        out[x].anchors = anchored;
    }
    // Backward: readers.
    for (k, &v) in program.outputs.iter().enumerate() {
        out[usize::from(v)].outputs.insert(k);
    }
    for x in (0..n).rev() {
        let reads_carried = carried(program, &members, x);
        let (outputs, readers) = (out[x].outputs.clone(), out[x].carried_readers.clone());
        for y in program.ops[x].operands() {
            let y = usize::from(y);
            out[y].outputs.extend(outputs.iter().copied());
            if reads_carried {
                out[y].carried_readers.insert(x as u16);
            } else {
                out[y].carried_readers.extend(readers.iter().copied());
            }
        }
    }
    out
}

/// Local value `x`'s bit `q`, made from its leaves alone: `leaf(y, q)` is leaf `y`'s bit `q`.
/// A `prev` reads zero before the stream's first byte, as every executor's does. Reading a
/// leaf that `fact`'s window does not hold is an error: the window is wrong.
pub(super) fn eval(program: &BlockProgram, fact: &Local, x: u16, q: i64, leaf: &dyn Fn(u16, i64) -> bool) -> Result<bool, String> {
    fn go(program: &BlockProgram, fact: &Local, x: u16, q: i64, base: i64, leaf: &dyn Fn(u16, i64) -> bool, memo: &mut HashMap<(u16, i64), bool>) -> Result<bool, String> {
        if let Some(&b) = memo.get(&(x, q)) {
            return Ok(b);
        }
        let b = if !fact.cone.contains(&x) {
            let offset = i32::try_from(q - base).map_err(|_| "an offset out of range".to_owned())?;
            if !fact.leaves.contains(&(x, offset)) {
                return Err(format!("%{x} at offset {offset} is read, and is not a leaf of the window"));
            }
            leaf(x, q)
        } else {
            let mut g = |y: u16, at: i64| go(program, fact, y, at, base, leaf, memo);
            match program.ops[usize::from(x)] {
                BlockOp::Not(a) => !g(a, q)?,
                BlockOp::And(a, b) => g(a, q)? & g(b, q)?,
                BlockOp::Or(a, b) => g(a, q)? | g(b, q)?,
                BlockOp::Xor(a, b) => g(a, q)? ^ g(b, q)?,
                BlockOp::AndNot(a, b) => g(a, q)? & !g(b, q)?,
                BlockOp::Prev(a, k) => q >= i64::from(k) && g(a, q - i64::from(k))?,
                other => return Err(format!("%{x}: {other:?} in a local cone")),
            }
        };
        memo.insert((x, q), b);
        Ok(b)
    }
    go(program, fact, x, q, q, leaf, &mut HashMap::new())
}

/// What `check` found: the claims tested, and each one that failed.
#[derive(Debug, Default)]
pub(super) struct Checked {
    pub anchored: usize,
    pub local: usize,
    pub bits: usize,
    pub wrong: Vec<String>,
}

/// Test `facts`' claims on `files` against the interpreter: each operation's every anchor
/// (its set bits all on the anchor's, shifted), and each local operation's window: made from
/// its leaves' words alone (`eval`'s rule, a word at a time), it equals the interpreter's.
pub(super) fn check(program: &BlockProgram, anchors: &[u16], facts: &[Fact], files: &[&[u8]]) -> Checked {
    let (values, spans) = program.all_values(files);
    let mut c = Checked::default();
    for (x, fact) in facts.iter().enumerate() {
        for &(i, shift) in &fact.anchors {
            c.anchored += 1;
            for span in &spans {
                let own = &values[x][span.clone()];
                let anchor = shifted(&values[usize::from(anchors[i])][span.clone()], -i64::from(shift));
                if let Some(w) = (0..own.len()).find(|&w| own[w] & !anchor[w] != 0) {
                    let b = (own[w] & !anchor[w]).trailing_zeros() as usize;
                    c.wrong.push(format!("%{x} anchored at %{} shifted {shift}: bit {} set, the anchor's is not", anchors[i], 64 * w + b));
                    break;
                }
            }
        }
        let Some(local) = &fact.local else { continue };
        if positional(&program.ops[x]) {
            continue;
        }
        c.local += 1;
        for span in &spans {
            c.bits += 64 * span.len();
            let leaf = |y: u16| &values[usize::from(y)][span.clone()];
            match stream(program, local, x as u16, 0, &leaf, &mut HashMap::new()) {
                Ok(made) => {
                    let own = &values[x][span.clone()];
                    if let Some(w) = (0..own.len()).find(|&w| own[w] != made[w]) {
                        let b = (own[w] ^ made[w]).trailing_zeros() as usize;
                        c.wrong.push(format!("%{x} at bit {}: from its leaves {}, the interpreter's {}", 64 * w + b, made[w] >> b & 1, own[w] >> b & 1));
                        break;
                    }
                }
                Err(e) => {
                    c.wrong.push(format!("%{x}: {e}"));
                    break;
                }
            }
        }
    }
    c
}

/// `words` shifted: bit `q` of the result is bit `q + o` of `words`, zero outside them.
fn shifted(words: &[u64], o: i64) -> Vec<u64> {
    let n = words.len() as i64;
    let (w, b) = (o.div_euclid(64), o.rem_euclid(64));
    let at = |j: i64| if j < 0 || j >= n { 0 } else { words[j as usize] };
    (0..n).map(|k| if b == 0 { at(k + w) } else { (at(k + w) >> b) | (at(k + w + 1) << (64 - b)) }).collect()
}

/// Local value `x`'s words, bit `q` its bit `q + off`, made from its leaves' words: `eval` a
/// word at a time. A leaf read at an offset its window does not hold is an error.
fn stream<'w>(program: &BlockProgram, fact: &Local, x: u16, off: i64, leaf: &dyn Fn(u16) -> &'w [u64], memo: &mut HashMap<(u16, i64), Vec<u64>>) -> Result<Vec<u64>, String> {
    if let Some(s) = memo.get(&(x, off)) {
        return Ok(s.clone());
    }
    let s = if !fact.cone.contains(&x) {
        let offset = i32::try_from(off).map_err(|_| "an offset out of range".to_owned())?;
        if !fact.leaves.contains(&(x, offset)) {
            return Err(format!("%{x} at offset {offset} is read, and is not a leaf of the window"));
        }
        shifted(leaf(x), off)
    } else {
        let mut g = |y: u16, o: i64| stream(program, fact, y, o, leaf, memo);
        let two = |a: Vec<u64>, b: Vec<u64>, f: fn(u64, u64) -> u64| -> Vec<u64> { a.iter().zip(&b).map(|(&p, &q)| f(p, q)).collect() };
        match program.ops[usize::from(x)] {
            BlockOp::Not(a) => g(a, off)?.iter().map(|w| !w).collect(),
            BlockOp::And(a, b) => two(g(a, off)?, g(b, off)?, |p, q| p & q),
            BlockOp::Or(a, b) => two(g(a, off)?, g(b, off)?, |p, q| p | q),
            BlockOp::Xor(a, b) => two(g(a, off)?, g(b, off)?, |p, q| p ^ q),
            BlockOp::AndNot(a, b) => two(g(a, off)?, g(b, off)?, |p, q| p & !q),
            BlockOp::Prev(a, k) => {
                // Zero where the byte read is before the stream: bit q reads q + off - k.
                let mut s = g(a, off - i64::from(k))?;
                let below = i64::from(k) - off;
                for (w, word) in s.iter_mut().enumerate() {
                    let first = 64 * w as i64;
                    if below > first {
                        let n = (below - first).min(64);
                        *word &= if n == 64 { 0 } else { !0u64 << n };
                    }
                }
                s
            }
            other => return Err(format!("%{x}: {other:?} in a local cone")),
        }
    };
    memo.insert((x, off), s.clone());
    Ok(s)
}
