//! What tier 1's outputs need of a file's bytes, from the program's instructions (`Needs`), and which outputs are made
//! from the bytes alone (`made_from_bytes`): the analysis the CPU's token loop (`native_cpu_bytes`, which lowers it),
//! the program's CPU form (`BlockNative::read_at_tokens`) and tier 2's region reads (`sequential`) share. Nothing here
//! emits code.

use std::collections::HashMap;

use crate::block_native::{BlockInst, BlockNative};
use crate::lane_plan::Plan;

/// Each instruction of `native`: made from the bytes alone, its value a function of the bytes near each position
/// (`native_cpu_bytes`' two forms): a byte-local value on the vectors (`plan`, `Plan::window`), a constant or a
/// prefix, the logic and `prev`s of such values, and `escaped` of one.
pub fn made_from_bytes(native: &BlockNative, plan: &Plan) -> Vec<bool> {
    let mut can = vec![false; native.insts.len()];
    for (x, inst) in native.insts.iter().enumerate() {
        let o = |a: u16| can[usize::from(a)];
        can[x] = plan.lanes[x]
            || match *inst {
                BlockInst::Const(_) | BlockInst::Prefix(..) => true,
                BlockInst::Escaped(a) | BlockInst::Not(a) | BlockInst::Prev(a, _) | BlockInst::Early(a, _) => o(a),
                BlockInst::And(a, b) | BlockInst::Or(a, b) | BlockInst::Xor(a, b) | BlockInst::AndNot(a, b) => o(a) && o(b),
                _ => false,
            };
    }
    can
}

/// The outputs of `native` made from the bytes (`made_from_bytes`), by output.
pub struct MadeFromBytes<'a> {
    native: &'a BlockNative,
    can: Vec<bool>,
}

impl<'a> MadeFromBytes<'a> {
    pub fn of(native: &'a BlockNative) -> Self {
        MadeFromBytes { native, can: made_from_bytes(native, &Plan::window(native)) }
    }

    /// Whether output `o` is made from the bytes (`Bytewise::can`).
    pub fn can(&self, o: u16) -> bool {
        self.native.outputs.get(usize::from(o)).is_some_and(|&v| self.can[usize::from(v)])
    }
}
/// Each value known at every byte of a window where values `zero` are false at every one
/// of them and the carries in of the runs `given` are none: an escape of a run that is none,
/// an `and` with a side none, and the logic of those. A `prev` reads bytes before the window:
/// not known.
pub fn known_if_zero(native: &BlockNative, zero: &[usize], given: &[usize]) -> HashMap<usize, bool> {
    let mut k: HashMap<usize, bool> = HashMap::new();
    for (x, inst) in native.insts.iter().enumerate() {
        let g = |a: u16| k.get(&usize::from(a)).copied();
        let v = if zero.contains(&x) {
            Some(false)
        } else {
            match *inst {
                BlockInst::Const(0) => Some(false),
                BlockInst::Const(u64::MAX) => Some(true),
                BlockInst::Not(a) => g(a).map(|b| !b),
                BlockInst::And(a, b) => match (g(a), g(b)) {
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                },
                BlockInst::AndNot(a, b) => match (g(a), g(b)) {
                    (Some(false), _) | (_, Some(true)) => Some(false),
                    (Some(true), Some(false)) => Some(true),
                    _ => None,
                },
                BlockInst::Or(a, b) => match (g(a), g(b)) {
                    (Some(true), _) | (_, Some(true)) => Some(true),
                    (Some(false), Some(false)) => Some(false),
                    _ => None,
                },
                BlockInst::Xor(a, b) => g(a).zip(g(b)).map(|(p, q)| p != q),
                BlockInst::Escaped(a) if given.contains(&usize::from(a)) && g(a) == Some(false) => Some(false),
                _ => None,
            }
        };
        if let Some(v) = v {
            k.insert(x, v);
        }
    }
    k
}

/// Whether the zeros before a file of the batch say a `prev`'s bound (`scalar`): its operand
/// `a`, read `reach` bytes before a byte of the file, is false on a zero byte, and `reach` is
/// at most `native_cpu_block::FRONT`, the zeros there.
pub fn zeros_bound(native: &crate::block_native::BlockNative, a: usize, reach: i64) -> bool {
    reach <= crate::block_native::FRONT as i64 && crate::lane_plan::zero_kills(native, a, 0)
}

/// What an output needs of the bytes (`needs`): a bit at `p` only where some byte from
/// `p + lo` to `p + hi` is in `set` (256 bits). An over-approximation, from the program's
/// instructions: a test that can hold only so is skipped where the bytes say it cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
    pub set: [u64; 4],
    pub lo: i32,
    pub hi: i32,
}

impl Needs {
    fn bytes(f: impl Fn(u8) -> bool, at: i32) -> Needs {
        let mut set = [0u64; 4];
        for b in 0..=255u8 {
            if f(b) {
                set[usize::from(b >> 6)] |= 1 << (b & 63);
            }
        }
        Needs { set, lo: at, hi: at }
    }

    fn shift(self, k: i32) -> Needs {
        Needs { lo: self.lo + k, hi: self.hi + k, ..self }
    }

    /// Its cost as a filter: the bytes it lets through times the places it looks at.
    fn width(&self) -> u32 {
        self.set.iter().map(|w| w.count_ones()).sum::<u32>() * (self.hi - self.lo + 1) as u32
    }

    /// Whether byte `b` is in the set.
    pub fn has(&self, b: u8) -> bool {
        self.set[usize::from(b >> 6)] >> (b & 63) & 1 == 1
    }

    /// The union of two needs: a bit of either's.
    pub fn union(self, o: Needs) -> Needs {
        let mut set = self.set;
        for (s, w) in set.iter_mut().zip(o.set) {
            *s |= w;
        }
        Needs { set, lo: self.lo.min(o.lo), hi: self.hi.max(o.hi) }
    }

    /// The set as runs of bytes, `(first, last)` each, in order.
    pub fn ranges(&self) -> Vec<(u8, u8)> {
        let mut v: Vec<(u8, u8)> = Vec::new();
        for b in 0..=255u8 {
            if !self.has(b) {
                continue;
            }
            match v.last_mut() {
                Some(r) if r.1.checked_add(1) == Some(b) => r.1 = b,
                _ => v.push((b, b)),
            }
        }
        v
    }
}

/// The `Or` terms of instruction `x`: its operands' through every `Or`, else itself.
pub fn or_terms(native: &BlockNative, x: usize) -> Vec<usize> {
    fn walk(native: &BlockNative, x: usize, out: &mut Vec<usize>) {
        match native.insts[x] {
            BlockInst::Or(a, b) => {
                walk(native, usize::from(a), out);
                walk(native, usize::from(b), out);
            }
            _ => out.push(x),
        }
    }
    let mut out = Vec::new();
    walk(native, x, &mut out);
    out
}

/// The bytes that can be at offset `at` from where output `o` holds: every byte where its
/// instructions do not say (an over-set), as `needs` at one offset, but at any of them: a
/// conjunction's sets met, a `prev`'s and a delay's offsets followed.
pub fn bytes_at(native: &BlockNative, o: u16, at: i32) -> [u64; 4] {
    let Some(&x) = native.outputs.get(usize::from(o)) else { return [u64::MAX; 4] };
    // The output at `p` is its instruction at `p + delay`.
    inst_bytes_at(native, usize::from(x), at - i32::from(native.output_delays[usize::from(o)]))
}

/// `bytes_at` of instruction `x`: the bytes at offset `at` from its own position.
pub fn inst_bytes_at(native: &BlockNative, x: usize, at: i32) -> [u64; 4] {
    fn go(native: &BlockNative, x: usize, d: i32, depth: u32) -> [u64; 4] {
        let all = [u64::MAX; 4];
        if depth > 64 {
            return all;
        }
        let here = |n: Option<Needs>| n.filter(|n| n.lo == d && n.hi == d).map_or(all, |n| n.set);
        let one = |x: usize| needs_at(native, x, &mut HashMap::new());
        match native.insts[x] {
            BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) | BlockInst::Pair { .. } => here(one(x)),
            BlockInst::And(a, b) => {
                let (p, q) = (go(native, usize::from(a), d, depth + 1), go(native, usize::from(b), d, depth + 1));
                std::array::from_fn(|k| p[k] & q[k])
            }
            BlockInst::Or(a, b) => {
                let (p, q) = (go(native, usize::from(a), d, depth + 1), go(native, usize::from(b), d, depth + 1));
                std::array::from_fn(|k| p[k] | q[k])
            }
            BlockInst::AndNot(a, _) => go(native, usize::from(a), d, depth + 1),
            // `a` `k` back: the byte at `d` is `d + k` from where `a` holds.
            BlockInst::Prev(a, k) => go(native, usize::from(a), d + i32::from(k), depth + 1),
            BlockInst::Early(a, k) => go(native, usize::from(a), d - i32::from(k), depth + 1),
            // A bit only after a byte of `a`'s run: `a` held one byte back.
            BlockInst::Escaped(a) if d == -1 => go(native, usize::from(a), 0, depth + 1),
            BlockInst::Const(0) => [0; 4],
            _ => all,
        }
    }
    go(native, x, at, 0)
}

/// What output `o` needs of the bytes (`Needs`), where its instructions say: byte tests, their
/// logic, `prev`s and `escaped` (a bit only after a byte of its run). None where it can hold
/// whatever the bytes near it (a negation, a carried value, a position).
pub fn needs(native: &BlockNative, o: u16) -> Option<Needs> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    let mut memo = HashMap::new();
    // The output at `p` is its instruction at `p + d` (`word`'s `start`).
    needs_at(native, x, &mut memo).map(|n| n.shift(d))
}

pub fn needs_at(native: &BlockNative, x: usize, memo: &mut HashMap<usize, Option<Needs>>) -> Option<Needs> {
    if let Some(&n) = memo.get(&x) {
        return n;
    }
    let t = |table: u16, bits: u8| {
        let c = native.tables[usize::from(table)];
        move |b: u8| c.member(b, bits)
    };
    let n = match native.insts[x] {
        BlockInst::EqByte(c) => Some(Needs::bytes(|b| b == c, 0)),
        BlockInst::Class { table, bits } => Some(Needs::bytes(t(table, bits), 0)),
        BlockInst::Plane(k) => Some(Needs::bytes(|b| b >> k & 1 == 1, 0)),
        BlockInst::PlaneBefore(k) => Some(Needs::bytes(|b| b >> k & 1 == 1, -1)),
        BlockInst::Pair { before, table, bits } => {
            let (a, b) = (Needs::bytes(t(table, bits), 0), Needs::bytes(t(before, bits), -1));
            Some(if b.width() < a.width() { b } else { a })
        }
        BlockInst::Const(0) => Some(Needs { set: [0; 4], lo: 0, hi: 0 }),
        BlockInst::And(a, b) => match (needs_at(native, usize::from(a), memo), needs_at(native, usize::from(b), memo)) {
            (Some(p), Some(q)) => Some(if q.width() < p.width() { q } else { p }),
            (p, q) => p.or(q),
        },
        BlockInst::AndNot(a, _) => needs_at(native, usize::from(a), memo),
        BlockInst::Or(a, b) => match (needs_at(native, usize::from(a), memo), needs_at(native, usize::from(b), memo)) {
            (Some(p), Some(q)) => {
                let mut set = p.set;
                for (s, w) in set.iter_mut().zip(q.set) {
                    *s |= w;
                }
                Some(Needs { set, lo: p.lo.min(q.lo), hi: p.hi.max(q.hi) })
            }
            _ => None,
        },
        BlockInst::Prev(a, k) => needs_at(native, usize::from(a), memo).map(|n| n.shift(-i32::from(k))),
        // A bit at `p` is the operand's at `p + k` (`BlockOp::Early`).
        BlockInst::Early(a, k) => needs_at(native, usize::from(a), memo).map(|n| n.shift(i32::from(k))),
        BlockInst::Escaped(a) => needs_at(native, usize::from(a), memo).map(|n| n.shift(-1)),
        _ => None,
    };
    memo.insert(x, n);
    n
}

/// Where output `o` is its bytes exactly (`Needs` with `lo == hi`): a bit at `p` where, and only
/// where, the byte at `p + lo` is in `set`. Byte tests and their unions at one offset; a count
/// of it over a token's bytes is then the count of its bytes in the set, one vector's compares.
pub fn exact(native: &BlockNative, o: u16) -> Option<Needs> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    exact_at(native, x).map(|n| n.shift(d))
}

pub fn exact_at(native: &BlockNative, x: usize) -> Option<Needs> {
    match native.insts[x] {
        BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) => needs_at(native, x, &mut HashMap::new()),
        BlockInst::Or(a, b) => {
            let (p, q) = (exact_at(native, usize::from(a))?, exact_at(native, usize::from(b))?);
            (p.lo == q.lo).then(|| p.union(q))
        }
        _ => None,
    }
}

/// The bytes the byte at `p + off` can be where output `o`'s bit at `p` is set: a set every
/// such byte is in (a test at that offset; an `And`'s both, an `Or`'s either, a `Prev` or an
/// `escaped` shifted). None where its instructions do not say (any byte).
pub fn output_byte_at(native: &BlockNative, o: u16, off: i32) -> Option<[u64; 4]> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    // The output at `p` is its instruction at `p + d` (`needs`).
    inst_byte_at(native, x, off - d, 0)
}

/// Whether output `o` never holds where output `p` does, at the same place: each of `o`'s
/// `Or` terms either needs a byte there that `p` never has (`inst_byte_at`), or needs a value
/// (an `And`'s operand, an `AndNot`'s first) that `p` excludes (an `AndNot`'s second). A
/// string's bad-escape test, `escaped(\) & !…`, so never holds at its opener, `" & !escaped(\)`.
pub fn never_both(native: &BlockNative, o: u16, p: u16) -> bool {
    let (Some(&x), Some(&y)) = (native.outputs.get(usize::from(o)), native.outputs.get(usize::from(p))) else { return false };
    let d = native.output_delays[usize::from(o)];
    if d != native.output_delays[usize::from(p)] {
        return false;
    }
    fn needs(native: &BlockNative, x: usize, out: &mut Vec<usize>) {
        out.push(x);
        match native.insts[x] {
            BlockInst::And(a, b) => {
                needs(native, usize::from(a), out);
                needs(native, usize::from(b), out);
            }
            BlockInst::AndNot(a, _) => needs(native, usize::from(a), out),
            _ => {}
        }
    }
    fn excludes(native: &BlockNative, x: usize, out: &mut Vec<usize>) {
        match native.insts[x] {
            BlockInst::And(a, b) => {
                excludes(native, usize::from(a), out);
                excludes(native, usize::from(b), out);
            }
            BlockInst::AndNot(a, b) => {
                excludes(native, usize::from(a), out);
                out.push(usize::from(b));
            }
            _ => {}
        }
    }
    let mut not = Vec::new();
    excludes(native, usize::from(y), &mut not);
    let at = -i32::from(d);
    let py = inst_byte_at(native, usize::from(y), at, 0);
    let mut terms = Vec::new();
    let mut todo = vec![usize::from(x)];
    while let Some(t) = todo.pop() {
        match native.insts[t] {
            BlockInst::Or(a, b) => todo.extend([usize::from(a), usize::from(b)]),
            _ => terms.push(t),
        }
    }
    terms.iter().all(|&t| {
        let bytes = py.zip(inst_byte_at(native, t, at, 0)).is_some_and(|(q, r)| (0..4).all(|k| q[k] & r[k] == 0));
        let mut need = Vec::new();
        needs(native, t, &mut need);
        bytes || need.iter().any(|n| not.contains(n))
    })
}

/// The bytes a run of output `o`'s bits (`EndRule::Run`) can end with: `output_byte_at`'s at
/// the run's bytes, but an `Or`'s term whose byte after is surely one of the run's (a test of
/// that byte alone, another term) ends none, as `dot & next(digit)` in `digit | …`.
pub fn run_last_bytes(native: &BlockNative, o: u16) -> Option<[u64; 4]> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    let mut terms = Vec::new();
    let mut todo = vec![x];
    while let Some(t) = todo.pop() {
        match native.insts[t] {
            BlockInst::Or(a, b) => todo.extend([usize::from(a), usize::from(b)]),
            _ => terms.push(t),
        }
    }
    // The bytes whose bit is surely set: a term that tests its own byte alone (a byte's test,
    // shifted by `Prev`s to the output's place).
    fn alone(native: &BlockNative, t: usize) -> bool {
        match native.insts[t] {
            BlockInst::EqByte(_) | BlockInst::Class { .. } => true,
            BlockInst::Prev(a, _) => alone(native, usize::from(a)),
            _ => false,
        }
    }
    let mut sure = [0u64; 4];
    for &t in &terms {
        if alone(native, t)
            && let Some(n) = needs_at(native, t, &mut HashMap::new()).filter(|n| n.lo == -d && n.hi == -d) {
                sure = std::array::from_fn(|k| sure[k] | n.set[k]);
            }
    }
    let mut set = [0u64; 4];
    for &t in &terms {
        let after = inst_byte_at(native, t, 1 - d, 0);
        if after.is_some_and(|a| (0..4).all(|k| a[k] & !sure[k] == 0)) {
            continue;
        }
        let at = inst_byte_at(native, t, -d, 0)?;
        set = std::array::from_fn(|k| set[k] | at[k]);
    }
    Some(set)
}

pub fn inst_byte_at(native: &BlockNative, x: usize, off: i32, depth: u32) -> Option<[u64; 4]> {
    if depth > 64 {
        return None;
    }
    let and = |p: [u64; 4], q: [u64; 4]| std::array::from_fn(|k| p[k] & q[k]);
    match native.insts[x] {
        // Both of a pair's bytes: the byte at `p` in its table's set, the one before in its
        // before's (`needs` keeps the narrower alone).
        BlockInst::Pair { before, table, bits } => {
            let set = |table: u16| {
                let c = native.tables[usize::from(table)];
                Needs::bytes(|b| c.member(b, bits), 0).set
            };
            match off {
                0 => Some(set(table)),
                -1 => Some(set(before)),
                _ => None,
            }
        }
        BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) => {
            let n = needs_at(native, x, &mut HashMap::new())?;
            (n.lo == off && n.hi == off).then_some(n.set)
        }
        BlockInst::And(a, b) => match (inst_byte_at(native, usize::from(a), off, depth + 1), inst_byte_at(native, usize::from(b), off, depth + 1)) {
            (Some(p), Some(q)) => Some(and(p, q)),
            (p, q) => p.or(q),
        },
        BlockInst::AndNot(a, _) => inst_byte_at(native, usize::from(a), off, depth + 1),
        BlockInst::Or(a, b) => {
            let (p, q) = (inst_byte_at(native, usize::from(a), off, depth + 1)?, inst_byte_at(native, usize::from(b), off, depth + 1)?);
            Some(std::array::from_fn(|k| p[k] | q[k]))
        }
        // A bit at `p` is the operand's at `p - k`: its byte at `p + off` is the operand's at
        // `off + k`.
        BlockInst::Prev(a, k) => inst_byte_at(native, usize::from(a), off + i32::from(k), depth + 1),
        BlockInst::Early(a, k) => inst_byte_at(native, usize::from(a), off - i32::from(k), depth + 1),
        BlockInst::Escaped(a) => inst_byte_at(native, usize::from(a), off + 1, depth + 1),
        _ => None,
    }
}

/// What an output needs of the bytes, offset by offset (`needs`'s, unmerged): a bit at `p`
/// only where, for some `(d, set)`, the byte at `p + d` is in `set`. Where `needs` merges two
/// tests at two offsets into one set over both (a control byte or a `\` before, so a newline
/// before a string's quote), this keeps each set at its own offset.
pub fn needs_by_offset(native: &BlockNative, o: u16) -> Option<Vec<(i32, [u64; 4])>> {
    let x = usize::from(*native.outputs.get(usize::from(o))?);
    let d = i32::from(native.output_delays[usize::from(o)]);
    let mut memo = HashMap::new();
    by_offset(native, x, &mut memo).map(|m| m.into_iter().map(|(k, s)| (k + d, s)).collect())
}

pub type Offsets = std::collections::BTreeMap<i32, [u64; 4]>;

pub fn by_offset(native: &BlockNative, x: usize, memo: &mut HashMap<usize, Option<Offsets>>) -> Option<Offsets> {
    if let Some(n) = memo.get(&x) {
        return n.clone();
    }
    let one = |n: Needs| -> Offsets { (n.lo..=n.hi).map(|k| (k, n.set)).collect() };
    let width = |m: &Offsets| m.values().map(|s| s.iter().map(|w| w.count_ones()).sum::<u32>()).sum::<u32>();
    let shift = |m: Offsets, k: i32| -> Offsets { m.into_iter().map(|(d, s)| (d + k, s)).collect() };
    let n = match native.insts[x] {
        BlockInst::EqByte(_) | BlockInst::Class { .. } | BlockInst::Plane(_) | BlockInst::PlaneBefore(_) | BlockInst::Pair { .. } | BlockInst::Const(0) => needs_at(native, x, &mut HashMap::new()).map(one),
        BlockInst::And(a, b) => match (by_offset(native, usize::from(a), memo), by_offset(native, usize::from(b), memo)) {
            (Some(p), Some(q)) => Some(if width(&q) < width(&p) { q } else { p }),
            (p, q) => p.or(q),
        },
        BlockInst::AndNot(a, _) => by_offset(native, usize::from(a), memo),
        BlockInst::Or(a, b) => match (by_offset(native, usize::from(a), memo), by_offset(native, usize::from(b), memo)) {
            (Some(mut p), Some(q)) => {
                for (d, s) in q {
                    let e = p.entry(d).or_insert([0; 4]);
                    for (w, v) in e.iter_mut().zip(s) {
                        *w |= v;
                    }
                }
                Some(p)
            }
            _ => None,
        },
        BlockInst::Prev(a, k) => by_offset(native, usize::from(a), memo).map(|m| shift(m, -i32::from(k))),
        BlockInst::Early(a, k) => by_offset(native, usize::from(a), memo).map(|m| shift(m, i32::from(k))),
        BlockInst::Escaped(a) => by_offset(native, usize::from(a), memo).map(|m| shift(m, -1)),
        _ => None,
    };
    memo.insert(x, n.clone());
    n
}

/// The bytes past a file's end a window may read, kept on the safe side: its start up to 63
/// and the program's delay past the end (a block of the file, or a token's bytes), its 64
/// bytes, an output's delay, and a 16-byte load's over-read. A program's padding (`pad`) of
/// this many lets a window go uncut (`Bytewise::padded`).
pub fn pad_reach(native: &BlockNative) -> usize {
    // An `early` reads its operand's bytes as much later, as a delay does.
    let early = native.insts.iter().map(|i| if let BlockInst::Early(_, k) = *i { usize::from(k) } else { 0 }).max().unwrap_or(0);
    63 + 2 * usize::from(native.delay) + early + 64 + 16
}

/// Whether a set a body tests (`class_pack::Need`) is false on the zero byte, its answer there
/// cared for: no bits on the padding past the file's end (`Bytewise::padded`).
pub fn zero_free(n: &crate::class_pack::Need) -> bool {
    n.set[0] & 1 == 0 && n.care[0] & 1 != 0
}

/// A region read in parts whose next token, from a clean place, is decided by the first byte of
/// two classes (`sequential`): `x`, the closer's, and `e`, the part's first.
#[derive(Clone)]
pub struct Seq {
    pub x: usize,
    pub e: usize,
    /// The bytes of `X` (the closer's), and of `X` or `E` (where the scan stops): a closer's
    /// first byte is in the first, a content token's in neither.
    pub xs: [u64; 4],
    pub stops: [u64; 4],
    /// Where `E` is one byte: a part's code by its second byte (`records::pass` of the two
    /// bytes), as simdjson's `escape_map[escape_char]`.
    pub codes2: Option<Box<[u32; 256]>>,
}

/// Whether a region read in parts needs no escape carry in tier 2, as simdjson's `parse_string`
/// needs none: its closer `X &~ escaped(E)`, its one part `(E & R) &~ escaped(E)` of two bytes
/// (`X`, `E` disjoint byte classes), its opener the closer's output, `R` the region's inside
/// (`toggle(T) &~ (T &~ prev(toggle(T), 1))`, `T` the closer and the file's own bytes), no
/// closer at the file's end refused. Then, from a clean place (after the opener, whose byte is
/// not `E`, or after a part), no `E` lies before the first byte of `X` or `E`, so it is not
/// escaped: an `X` there is the closer, an `E` a part, the bytes before it content. A part's
/// two bytes end a run of `E` of even length (its first not escaped), so the place after it is
/// clean again. `R` holds at every byte between the opener and the closer, as `T` holds at
/// neither's inside.
pub fn sequential(block: &crate::block_native::BlockNative, a: &crate::records::ReadArm, parts: &[(u16, u8)]) -> Option<Seq> {
    use crate::block_native::BlockInst as I;
    let class = |x: usize| exact_at(block, x).filter(|n| n.lo == 0 && n.hi == 0).map(|n| n.set);
    let at = |o: u16| usize::from(block.outputs[usize::from(o)]);
    if a.opener != a.closer || a.off != 0 || a.none != crate::records::ReadNone::Refuse || block.output_delays[usize::from(a.closer)] != 0 {
        return None;
    }
    let &[(part, 2)] = parts else { return None };
    if block.output_delays[usize::from(part)] != 0 {
        return None;
    }
    let c = at(a.closer);
    let I::AndNot(x, esc) = block.insts[c] else { return None };
    let I::Escaped(e) = block.insts[usize::from(esc)] else { return None };
    let (x, e) = (usize::from(x), usize::from(e));
    let (xs, es) = (class(x)?, class(e)?);
    if xs.iter().zip(es).any(|(p, q)| p & q != 0) {
        return None;
    }
    // The part: `(E & R) &~ escaped(E)`, `R` the region's inside.
    let I::AndNot(pe, pesc) = block.insts[at(part)] else { return None };
    let I::Escaped(e2) = block.insts[usize::from(pesc)] else { return None };
    if class(usize::from(e2))? != es {
        return None;
    }
    let I::And(u, v) = block.insts[usize::from(pe)] else { return None };
    let (u, v) = (usize::from(u), usize::from(v));
    let r = match (class(u), class(v)) {
        (Some(s), _) if s == es => v,
        (_, Some(s)) if s == es => u,
        _ => return None,
    };
    // `R = toggle(T) &~ (T &~ prev(toggle(T), 1))`, `T` the closer and a mask that holds at
    // every byte of the stream (`file_only`): `Within(0)`, an `&` of such, or one less a test
    // that never holds in the stream (the prefix's bytes, before it; a zero).
    fn file_only(b: &crate::block_native::BlockNative, x: usize) -> bool {
        match b.insts[x] {
            I::Within(0) => true,
            I::And(a, c) => file_only(b, usize::from(a)) && file_only(b, usize::from(c)),
            I::AndNot(a, c) => file_only(b, usize::from(a)) && matches!(b.insts[usize::from(c)], I::Prefix(..) | I::Const(0)),
            _ => false,
        }
    }
    let I::AndNot(tg, opener) = block.insts[r] else { return None };
    let I::Toggle(t) = block.insts[usize::from(tg)] else { return None };
    let t = usize::from(t);
    let closer_and_file = t == c || matches!(block.insts[t], I::And(p, q) if (usize::from(p) == c && file_only(block, usize::from(q))) || (usize::from(q) == c && file_only(block, usize::from(p))));
    let I::AndNot(t2, pt) = block.insts[usize::from(opener)] else { return None };
    let I::Prev(tg2, 1) = block.insts[usize::from(pt)] else { return None };
    let I::Toggle(t3) = block.insts[usize::from(tg2)] else { return None };
    (closer_and_file && usize::from(t2) == t && usize::from(t3) == t).then_some(Seq { x, e, xs, stops: std::array::from_fn(|k| xs[k] | es[k]), codes2: None })
}
