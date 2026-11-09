//! Proofs about a block program's values, for the Bitlex compiler: which bytes a value's bits
//! can be at (`first_bytes`), and whether a value is zero everywhere (`never`).
//!
//! The compiler asks them of token rows: the bytes a row's tokens begin with (tier 2 switches on
//! a token's first byte), whether two rows can start at one byte (a pair that can goes to the
//! fallback), and the modes a row's start fixes (`bitlex_compile::row_modes`). Each answer is
//! sound: a byte left out never has a bit, and a value proved zero is zero on every stream.
//!
//! A value is a function of the bytes around a position and of carried values (`toggle`,
//! `escaped`, `fill`, …). The proof is a search over them, as a SAT solver's, under three-valued
//! logic:
//!   - **The free values.** Each byte at a lag from the position (`Byte`), split over the groups
//!     of bytes the classes read at that lag tell apart, and "before the stream", where every
//!     `prev` shifts in 0. Each carried value at a lag (`Op`), true or false.
//!   - **The equations** that tie a carried value to the one a byte before, so `x & ~x` is not
//!     all they know: `escaped(a)` is `prev(a) & ~prev(escaped(a))`, `toggle(x)` is
//!     `prev(toggle(x)) ^ x`, `fill(s, e)` is `s | (prev(fill(s, e)) & ~e)`. A way of the free
//!     values that breaks one is no stream's: the value is zero there by default.
//!   - **The search.** The value under the values chosen so far; when it turns on an unchosen
//!     one, each way of it in turn. Then the equations of the chosen carried values, likewise.
//!     A budget of tries per byte: past it, the byte may have a bit (sound, less sharp).
//! The carried values an equation may choose are those the value reads (its cone, closed under
//! the equations' operands): an equation reaching outside them tells only what its operands do.
//!
//! The position is in the stream (the questions are about tokens): a byte at or before it is the
//! stream's or before it (`within` is which), one after it may be past the end.

use super::block::{BlockOp, ClassTable};
use std::collections::{HashMap, HashSet};

/// A set of bytes, bit `b` for byte `b`.
pub(super) type ByteSet = [u64; 4];

/// A byte before the stream: no byte class has it, every carried value is 0 there.
const BEFORE: u16 = 256;

/// The tries per byte of the position.
const BUDGET: u32 = 1 << 12;

/// A free value: the byte at a lag, or an operation's value at a lag.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Var {
    Byte(u32),
    Op(u16, u32),
}

/// What reads a byte: a class or byte at that lag, or a `Pair`'s byte or byte before.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Reader {
    Eq(u8),
    Class(u16, u8),
}

struct Proof<'a> {
    ops: &'a [BlockOp],
    tables: &'a [ClassTable],
    /// The lag of the position's own byte, and that byte.
    d: u32,
    b: u8,
    bytes: HashMap<u32, u16>,
    vals: HashMap<(u16, u32), bool>,
    /// The carried values that may be chosen.
    cone: HashSet<(u16, u32)>,
    /// Each lag's byte groups: a byte of each.
    groups: HashMap<u32, Vec<u8>>,
    budget: u32,
}

impl Proof<'_> {
    fn byte(&self, lag: u32, open: &mut Option<Var>) -> Option<u16> {
        if lag == self.d {
            return Some(u16::from(self.b));
        }
        match self.bytes.get(&lag) {
            Some(&x) => Some(x),
            None => {
                open.get_or_insert(Var::Byte(lag));
                None
            }
        }
    }

    fn member(&self, r: Reader, x: u16) -> bool {
        match r {
            Reader::Eq(c) => x == u16::from(c),
            Reader::Class(table, bits) => x < BEFORE && self.tables[usize::from(table)].member(x as u8, bits),
        }
    }

    /// `at`'s value at `lag + k`, as a `prev` reads it: 0 before the stream.
    fn prev(&self, at: u16, lag: u32, open: &mut Option<Var>) -> Option<bool> {
        let mut mine = None;
        let place = self.byte(lag, &mut mine);
        if place == Some(BEFORE) {
            return Some(false);
        }
        let r = self.eval(at, lag, open);
        match (r, place) {
            (Some(false), _) => Some(false),
            (r, Some(_)) => r,
            (_, None) => {
                if let Some(v) = mine {
                    open.get_or_insert(v);
                }
                None
            }
        }
    }

    fn eval(&self, at: u16, lag: u32, open: &mut Option<Var>) -> Option<bool> {
        match self.ops[usize::from(at)] {
            BlockOp::Eq(c) => self.byte(lag, open).map(|x| self.member(Reader::Eq(c), x)),
            BlockOp::Class { table, bits } => self.byte(lag, open).map(|x| self.member(Reader::Class(table, bits), x)),
            BlockOp::Pair { before, table, bits } => {
                let x = self.byte(lag, open)?;
                if !self.member(Reader::Class(table, bits), x) {
                    return Some(false);
                }
                // The byte before the stream is 0x00 to a pair.
                let y = self.byte(lag + 1, open)?;
                Some(self.member(Reader::Class(before, bits), if y == BEFORE { 0 } else { y }))
            }
            BlockOp::Const(0) => Some(false),
            BlockOp::Const(u64::MAX) => Some(true),
            BlockOp::Not(a) => self.eval(a, lag, open).map(|x| !x),
            BlockOp::And(a, c) | BlockOp::AndNot(a, c) => {
                let neg = matches!(self.ops[usize::from(at)], BlockOp::AndNot(..));
                let x = self.eval(a, lag, open);
                if x == Some(false) {
                    return Some(false);
                }
                match (x, self.eval(c, lag, open).map(|y| y != neg)) {
                    (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                }
            }
            BlockOp::Or(a, c) => {
                let x = self.eval(a, lag, open);
                if x == Some(true) {
                    return Some(true);
                }
                match (x, self.eval(c, lag, open)) {
                    (_, Some(true)) => Some(true),
                    (Some(false), Some(false)) => Some(false),
                    _ => None,
                }
            }
            BlockOp::Xor(a, c) => match (self.eval(a, lag, open), self.eval(c, lag, open)) {
                (Some(x), Some(y)) => Some(x != y),
                _ => None,
            },
            BlockOp::Prev(a, k) => self.prev(a, lag + u32::from(k), open),
            // The stream's bytes: a byte at or before the position is in it unless before it (the
            // position is a token's, in the stream); one after it may be past the end.
            BlockOp::Within(w) if lag + u32::from(w) >= self.d => self.byte(lag + u32::from(w), open).map(|x| x != BEFORE),
            // A carried value, a position's pattern, `within`: free.
            _ => match self.vals.get(&(at, lag)) {
                Some(&x) => Some(x),
                None => {
                    if self.cone.contains(&(at, lag)) {
                        open.get_or_insert(Var::Op(at, lag));
                    }
                    None
                }
            },
        }
    }

    /// The equations of the carried values chosen so far: `Some(false)` when one is broken,
    /// `None` when one turns on an unchosen value (in `open`).
    fn equations(&self, open: &mut Option<Var>) -> Option<bool> {
        let mut all = Some(true);
        for (&(at, lag), &val) in &self.vals {
            let mut mine = None;
            let want = match self.ops[usize::from(at)] {
                BlockOp::Escaped(a) => match self.prev(a, lag + 1, &mut mine) {
                    Some(false) => Some(false),
                    x => match (x, self.prev(at, lag + 1, &mut mine)) {
                        (_, Some(true)) => Some(false),
                        (Some(true), Some(false)) => Some(true),
                        _ => None,
                    },
                },
                BlockOp::Toggle(x) => match (self.prev(at, lag + 1, &mut mine), self.eval(x, lag, &mut mine)) {
                    (Some(p), Some(x)) => Some(p != x),
                    _ => None,
                },
                BlockOp::Fill(s, e) => match self.eval(s, lag, &mut mine) {
                    Some(true) => Some(true),
                    s => match (s, self.prev(at, lag + 1, &mut mine), self.eval(e, lag, &mut mine)) {
                        (_, Some(false), _) | (_, _, Some(true)) => s,
                        (Some(false), Some(true), Some(false)) => Some(true),
                        _ => None,
                    },
                },
                _ => Some(val),
            };
            match want {
                Some(w) if w != val => return Some(false),
                Some(_) => {}
                None => {
                    if let Some(v) = mine {
                        open.get_or_insert(v);
                        all = None;
                    }
                }
            }
        }
        all
    }

    /// Whether the value `v` is zero under every way of the values not chosen yet.
    fn zero(&mut self, v: u16) -> bool {
        let mut open = None;
        let r = self.eval(v, 0, &mut open);
        if r == Some(false) {
            return true;
        }
        let mut need = None;
        let eqs = self.equations(&mut need);
        if eqs == Some(false) {
            // No stream has these values.
            return true;
        }
        let Some(var) = open.or(need) else {
            // The value is set and every equation holds, or the rest are free: a stream may.
            return false;
        };
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        match var {
            Var::Op(at, lag) => [false, true].into_iter().all(|x| {
                self.vals.insert((at, lag), x);
                let z = self.zero(v);
                self.vals.remove(&(at, lag));
                z
            }),
            Var::Byte(lag) => {
                // Before the stream: a byte before the position's own, with none chosen before it
                // in the stream; and it must be when one chosen after it is.
                let before_ok = lag > self.d && self.bytes.iter().all(|(&l, &x)| l < lag || x == BEFORE);
                let after_before = self.bytes.iter().any(|(&l, &x)| l < lag && x == BEFORE);
                let mut ways: Vec<u16> = Vec::new();
                if !after_before {
                    // No class reads it: one byte stands for every one.
                    ways.extend(self.groups.get(&lag).map_or_else(|| vec![0], |g| g.iter().map(|&x| u16::from(x)).collect()));
                }
                if before_ok || after_before {
                    ways.push(BEFORE);
                }
                ways.into_iter().all(|x| {
                    self.bytes.insert(lag, x);
                    let z = self.zero(v);
                    self.bytes.remove(&lag);
                    z
                })
            }
        }
    }
}

/// The carried values `v` reads, closed under their equations' operands, and the readers of
/// each lag's byte.
fn cone(ops: &[BlockOp], v: u16) -> (HashSet<(u16, u32)>, HashMap<u32, HashSet<Reader>>) {
    let mut cone = HashSet::new();
    let mut readers: HashMap<u32, HashSet<Reader>> = HashMap::new();
    let mut seen = HashSet::new();
    let mut work = vec![(v, 0u32)];
    while let Some((at, lag)) = work.pop() {
        if !seen.insert((at, lag)) {
            continue;
        }
        match ops[usize::from(at)] {
            BlockOp::Eq(c) => {
                readers.entry(lag).or_default().insert(Reader::Eq(c));
            }
            BlockOp::Class { table, bits } => {
                readers.entry(lag).or_default().insert(Reader::Class(table, bits));
            }
            BlockOp::Pair { before, table, bits } => {
                readers.entry(lag).or_default().insert(Reader::Class(table, bits));
                readers.entry(lag + 1).or_default().insert(Reader::Class(before, bits));
            }
            BlockOp::Const(_) => {}
            BlockOp::Not(a) => work.push((a, lag)),
            BlockOp::And(a, b) | BlockOp::Or(a, b) | BlockOp::Xor(a, b) | BlockOp::AndNot(a, b) => work.extend([(a, lag), (b, lag)]),
            BlockOp::Prev(a, k) => work.push((a, lag + u32::from(k))),
            BlockOp::Escaped(a) => {
                cone.insert((at, lag));
                work.push((a, lag + 1));
            }
            BlockOp::Toggle(x) => {
                cone.insert((at, lag));
                work.push((x, lag));
            }
            BlockOp::Fill(s, e) => {
                cone.insert((at, lag));
                work.extend([(s, lag), (e, lag)]);
            }
            // A fixed point's carry: free, as a position's pattern is (sound: it may be anything).
            BlockOp::Within(_) | BlockOp::Phase(..) | BlockOp::Carry(_) => {
                cone.insert((at, lag));
            }
        }
    }
    (cone, readers)
}

/// A byte of each group the readers tell apart.
fn groups(tables: &[ClassTable], readers: &HashSet<Reader>) -> Vec<u8> {
    let readers: Vec<Reader> = readers.iter().copied().collect();
    let sig = |x: u8| -> Vec<bool> {
        readers
            .iter()
            .map(|&r| match r {
                Reader::Eq(c) => x == c,
                Reader::Class(t, bits) => tables[usize::from(t)].member(x, bits),
            })
            .collect()
    };
    let mut seen: HashSet<Vec<bool>> = HashSet::new();
    (0..=255u8).filter(|&x| seen.insert(sig(x))).collect()
}

/// The bytes a value's set bits can be at, at delay `d` (bit `i` is byte `i - d`): each byte for
/// which the value is not proved zero. Sound: a byte left out never has a bit.
pub(super) fn first_bytes(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> ByteSet {
    let (cone, readers) = cone(ops, v);
    let groups = readers.iter().map(|(&lag, r)| (lag, groups(tables, r))).collect();
    let mut p = Proof { ops, tables, d: u32::from(d), b: 0, bytes: HashMap::new(), vals: HashMap::new(), cone, groups, budget: 0 };
    let mut out = [0u64; 4];
    for b in 0..=255u8 {
        p.b = b;
        p.budget = BUDGET;
        if !p.zero(v) {
            out[usize::from(b / 64)] |= 1 << (b % 64);
        }
    }
    out
}

/// Whether a value is zero at every byte of every stream, proved.
pub(super) fn never(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
    first_bytes(ops, tables, v, d) == [0; 4]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(set: &[u8]) -> (ClassTable, BlockOp) {
        let mut t = ClassTable { lo: [0; 16], hi: [0; 16] };
        for &b in set {
            t.lo[usize::from(b & 15)] |= 1;
            t.hi[usize::from(b >> 4)] |= 1;
        }
        (t, BlockOp::Class { table: 0, bits: 1 })
    }

    /// `x & ~x` is zero; `x` is not; an escaped byte has the escape before it.
    #[test]
    fn proofs_of_small_values() {
        let ops = vec![
            BlockOp::Eq(b'"'),     // 0 quote
            BlockOp::Eq(b'\\'),    // 1 backslash
            BlockOp::Escaped(1),   // 2
            BlockOp::Toggle(0),    // 3
            BlockOp::AndNot(3, 3), // 4 inside & ~inside
            BlockOp::Prev(1, 1),   // 5 the byte before is a backslash
            BlockOp::AndNot(2, 5), // 6 escaped, not after a backslash
            BlockOp::And(0, 2),    // 7 an escaped quote
        ];
        assert!(never(&ops, &[], 4, 0));
        assert!(!never(&ops, &[], 3, 0));
        assert!(never(&ops, &[], 6, 0), "escaped(a) is after an a");
        assert_eq!(first_bytes(&ops, &[], 7, 0), { let mut s = [0; 4]; s[0] |= 1 << b'"'; s });
    }

    /// `within` is the stream's bytes: an escaped byte's escape is one of them, so it is never
    /// the stream's first byte.
    #[test]
    fn within_is_the_stream() {
        let ops = vec![BlockOp::Eq(b'\\'), BlockOp::Escaped(0), BlockOp::Within(0), BlockOp::Prev(2, 1), BlockOp::AndNot(1, 3), BlockOp::AndNot(2, 3)];
        assert!(never(&ops, &[], 4, 0), "an escaped byte has a byte before it");
        assert!(!never(&ops, &[], 5, 0), "the stream's first byte has none");
    }

    /// A byte class with 0x00 is not set before the stream (a `prev` of it is 0 there): its
    /// complement's `prev` is not proved zero where the class's is.
    #[test]
    fn before_the_stream() {
        let (t, c) = class(&[0]);
        let ops = vec![c, BlockOp::Prev(0, 1), BlockOp::Not(0), BlockOp::Prev(2, 1), BlockOp::Or(1, 3), BlockOp::Not(4)];
        // prev(c) | prev(~c) is 0 at the stream's first byte: its complement is not zero.
        assert!(!never(&ops, &[t], 5, 0));
    }
}
