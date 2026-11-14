//! Proofs about a block program's values, for the Bitlex compiler: which bytes a value's bits
//! can be at (`first_bytes`), and whether a value is zero everywhere (`never`; with the fixed
//! points' carries and lemmas proved by induction, `Induction`).
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
//!
//! The carried values an equation may choose are those the value reads (its cone, closed under
//! the equations' operands): an equation reaching outside them tells only what its operands do.
//!
//! The position is in the stream (the questions are about tokens): a byte at or before it is the
//! stream's or before it (`within` is which), one after it may be past the end.

use crate::block::{BlockOp, ClassTable};
use std::collections::{HashMap, HashSet};

/// A set of bytes, bit `b` for byte `b`.
pub type ByteSet = [u64; 4];

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
            // A `back_fill` too: it reads bytes after, which the window does not hold.
            BlockOp::Within(_) | BlockOp::Phase(..) | BlockOp::Position(_) | BlockOp::Carry(_) | BlockOp::BackFill(..) | BlockOp::Cmp { .. } | BlockOp::Bit { .. } => {
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
pub fn first_bytes(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> ByteSet {
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
pub fn never(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
    first_bytes(ops, tables, v, d) == [0; 4]
}

/// Whether a value is zero at every byte of every stream, proved by a SAT solver over a window
/// of bytes (`Unrolled`), with no lemmas kept (`Induction` keeps them).
pub fn never_unrolled(ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>], v: u16, d: u8, horizon: u32, conflicts: u32) -> bool {
    Induction::new(rounds, horizon, conflicts).never(ops, tables, v, d)
}

/// A lemma: values that never hold together at a position, each an operation at a lag from it
/// (read as a `prev` reads it) and its value; `STREAM` for "the byte is in the stream".
type Lemma = Vec<(u16, u32, bool)>;

/// A lemma's value "the byte at this lag is in the stream" (`Lemma`).
const STREAM: u16 = u16::MAX;

/// Proofs over a window of bytes, by a SAT solver, with the lemmas they prove on the way.
///
/// The window is each operation at each lag the value reads it at, as clauses (`bitlex_sat`).
/// The free values are as `never`'s: each lag's byte (one of the groups of bytes its readers
/// tell apart, or before the stream from some lag on), and what the window does not hold. Each
/// carried value (`toggle`, `escaped`, `fill`, a fixed point's carry) has its equation at every
/// lag up to `horizon` lags past the farthest the value reads without them; past it, it is
/// free. Free is sound but loose: a value the window's edge sets may be one no stream has (a
/// region's carry on at a byte that no region holds), and a run of bytes that change nothing
/// carries it to the position, however long the window.
///
/// So the edge is constrained by lemmas, as IC3's frames are: when the solver finds a way, its
/// edge (the carried values and bytes past the horizon) is a set of values to prove never hold
/// together at any position, by the same proof one level down. Proved, it is made smaller (each
/// value dropped while the rest still never hold), kept, and holds at every lag of every window
/// after; the value is tried again. Not proved, or past the levels or rounds: not proved (sound,
/// a lemma is a proof of its own).
pub struct Induction {
    nexts: HashMap<u16, u16>,
    horizon: u32,
    conflicts: u32,
    lemmas: Vec<Lemma>,
    /// The solves left to the question being asked (`SOLVES`).
    solves: u32,
}

/// The solves a question may take, lemmas included: past them, not proved.
const SOLVES: u32 = 64;

/// The levels of lemmas under a proof, and the rounds of each.
const LEVELS: u32 = 3;
const ROUNDS: u32 = 24;

enum Outcome {
    Proved,
    /// A way: its edge (the values past the horizon, in the stream), and the group of the
    /// position's byte (every byte when nothing reads it).
    Way(Lemma, ByteSet),
    Unknown,
}

impl Induction {
    pub fn new(rounds: &[Vec<(u16, u16)>], horizon: u32, conflicts: u32) -> Self {
        Induction { nexts: rounds.iter().flatten().copied().collect(), horizon, conflicts, lemmas: Vec::new(), solves: 0 }
    }

    /// Whether `v` is zero at every byte of every stream, at delay `d`.
    pub fn never(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
        let mut ops = ops.to_vec();
        self.solves = SOLVES;
        self.prove(&mut ops, tables, v, d, LEVELS, None, &[0; 4])
    }

    /// `hypothesis`: `v` is this lemma's values all holding, assumed not to hold at every lag
    /// past the position's: by induction on the stream's positions, the first where they hold
    /// has none before it (sound).
    /// The bytes `v`'s bits can be at, at delay `d`, as `first_bytes`: a real stream's byte at
    /// the position (a stream that begins within the window, `solve`'s `bounded`), its group,
    /// and again with those left out, until none is left; then proved with the lemmas. Not
    /// proved: every byte (sound).
    pub fn first_bytes(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> ByteSet {
        let mut ops = ops.to_vec();
        let mut found = [0u64; 4];
        for _ in 0..256 {
            match self.solve(&ops, tables, v, d, None, true, &found) {
                Outcome::Way(_, group) if group != [0; 4] && (0..4).any(|i| group[i] & !found[i] != 0) => {
                    for i in 0..4 {
                        found[i] |= group[i];
                    }
                }
                Outcome::Proved => {
                    self.solves = SOLVES;
                    return if self.prove(&mut ops, tables, v, d, LEVELS, None, &found) { found } else { [!0; 4] };
                }
                _ => return [!0; 4],
            }
        }
        [!0; 4]
    }

    /// `hypothesis`: `v` is this lemma's values all holding, assumed not to hold at every lag
    /// past the position's: by induction on the stream's positions, the first where they hold
    /// has none before it (sound).
    /// `exclude`: bytes the position's is not (`first_bytes`).
    #[allow(clippy::too_many_arguments)]
    fn prove(&mut self, ops: &mut Vec<BlockOp>, tables: &[ClassTable], v: u16, d: u8, level: u32, hypothesis: Option<&Lemma>, exclude: &ByteSet) -> bool {
        // First a stream that begins within the window, where every carried value has its
        // equation: a way there is a stream's, and no lemma can rule it out.
        if self.solves == 0 {
            return false;
        }
        self.solves -= 1;
        if matches!(self.solve(ops, tables, v, d, hypothesis, true, exclude), Outcome::Way(..) | Outcome::Unknown) {
            return false;
        }
        for _ in 0..ROUNDS {
            if self.solves == 0 {
                return false;
            }
            self.solves -= 1;
            let edge = match self.solve(ops, tables, v, d, hypothesis, false, exclude) {
                Outcome::Proved => return true,
                Outcome::Unknown => return false,
                Outcome::Way(edge, _) => edge,
            };
            if level == 0 || edge.is_empty() {
                return false;
            }
            let base = ops.len();
            let mut lemma = edge;
            let c = cube(ops, &lemma);
            if !self.prove(ops, tables, c, 0, level - 1, Some(&lemma), &[0; 4]) {
                ops.truncate(base);
                return false;
            }
            // Smaller: each value dropped while the rest are still proved never to hold.
            let mut i = 0;
            while i < lemma.len() && lemma.len() > 1 {
                let mut fewer = lemma.clone();
                fewer.remove(i);
                let c = cube(ops, &fewer);
                if self.prove(ops, tables, c, 0, level - 1, Some(&fewer), &[0; 4]) {
                    lemma = fewer;
                } else {
                    i += 1;
                }
            }
            ops.truncate(base);
            self.lemmas.push(lemma);
        }
        false
    }

    /// `bounded`: the stream begins within the window (before it past the horizon).
    #[allow(clippy::too_many_arguments)]
    fn solve(&self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8, hypothesis: Option<&Lemma>, bounded: bool, exclude: &ByteSet) -> Outcome {
        let nexts = &self.nexts;
        let (_, plain) = cone(ops, v);
        let reach = plain.keys().copied().max().unwrap_or(0).max(u32::from(d)) + self.horizon;
        let (readers, far) = unrolled_readers(ops, nexts, v, reach);
        let mut u = Unrolled { ops, tables, nexts, d: u32::from(d), reach, clauses: Vec::new(), vars: 1, memo: HashMap::new(), before: HashMap::new(), past: HashMap::new(), groups: HashMap::new(), readers: HashMap::new() };
        // A literal that is true.
        u.clauses.push(vec![1]);
        // Before the stream: from some lag past the position's on.
        for lag in u.d + 1..=far {
            let b = u.fresh();
            u.before.insert(lag, b);
            if let Some(&p) = u.before.get(&(lag - 1)) {
                u.clauses.push(vec![-p, b]);
            }
        }
        // Past its end: from some lag short of the position's down (the stream ends once).
        for lag in (0..u.d).rev() {
            let x = u.fresh();
            u.past.insert(lag, x);
            if let Some(&p) = u.past.get(&(lag + 1)) {
                u.clauses.push(vec![-p, x]);
            }
        }
        let mut lags: Vec<u32> = readers.keys().copied().collect();
        lags.sort_unstable();
        for lag in lags {
            let gs = groups(tables, &readers[&lag]);
            let vs: Vec<Lit> = gs.iter().map(|_| u.fresh()).collect();
            let b = u.before.get(&lag).copied();
            // Exactly one group, or none before the stream.
            u.clauses.push(vs.iter().copied().chain(b).collect());
            for (i, &x) in vs.iter().enumerate() {
                if let Some(b) = b {
                    u.clauses.push(vec![-b, -x]);
                }
                for &y in &vs[i + 1..] {
                    u.clauses.push(vec![-x, -y]);
                }
            }
            // Past the end the bytes are 0x00 (`block::padded`): the group of 0x00 is the one
            // `groups` gives first.
            if let Some(&p) = u.past.get(&lag) {
                u.clauses.push(vec![-p, vs[0]]);
            }
            u.groups.insert(lag, gs.into_iter().zip(vs).collect());
        }
        let root = u.op(v, 0);
        u.clauses.push(vec![root]);
        if bounded
            && let Some(&b) = u.before.get(&(reach + 1)) {
                u.clauses.push(vec![b]);
            }
        // The position's byte is none of `exclude`'s: each group of them left out (a group is
        // one way of its readers, so all its bytes are or none).
        let has = |set: &ByteSet, b: u8| set[usize::from(b / 64)] >> (b % 64) & 1 == 1;
        let pos = u32::from(d);
        let position: Vec<(u8, Lit)> = u.groups.get(&pos).cloned().unwrap_or_default();
        if *exclude != [0; 4] {
            if position.is_empty() {
                return Outcome::Unknown;
            }
            for &(b, l) in &position {
                if has(exclude, b) {
                    u.clauses.push(vec![-l]);
                }
            }
        }
        // The lemmas, at each lag of the window whose values they read are in it.
        let mut held: Vec<(u16, u32)> = u.memo.keys().copied().collect();
        held.sort_unstable();
        let lemmas = self.lemmas.iter().map(|l| (l, 0)).chain(hypothesis.map(|l| (l, 1)));
        for (lemma, from) in lemmas {
            let Some(&(a0, l0, _)) = lemma.iter().find(|&&(at, _, _)| at != STREAM) else { continue };
            for &(_, lag) in held.iter().filter(|&&(at, _)| at == a0) {
                let Some(base) = lag.checked_sub(l0) else { continue };
                if base < from {
                    continue;
                }
                if lemma.iter().any(|&(at, l, _)| if at == STREAM { base + l > far } else { !u.memo.contains_key(&(at, base + l)) }) {
                    continue;
                }
                // Not all of them, where the position is in the stream.
                let mut c = vec![-u.inside(base)];
                for &(at, l, val) in lemma {
                    let x = if at == STREAM { u.inside(base + l) } else if l == 0 { u.memo[&(at, base)] } else { u.prev(at, base + l) };
                    c.push(if val { -x } else { x });
                }
                u.clauses.push(c);
            }
        }
        let mut s = Solver::new(u.vars as usize);
        for c in &u.clauses {
            s.add(c);
        }
        match s.solve(self.conflicts) {
            Answer::Unsat => Outcome::Proved,
            Answer::Unknown => Outcome::Unknown,
            Answer::Sat => {
                let holds = |l: Lit| if l == 1 { true } else if l == -1 { false } else { s.holds(l.unsigned_abs() as usize) == (l > 0) };
                let inside = |lag: u32| holds(u.inside(lag));
                let edge: Vec<(u16, u32)> = held.iter().copied().filter(|&(at, lag)| lag > reach && inside(lag) && (carried(ops, nexts, at) || matches!(ops[usize::from(at)], BlockOp::Eq(_) | BlockOp::Class { .. }))).collect();
                // The position's byte's group: the bytes its readers can not tell from it.
                let group = match position.iter().find(|&&(_, l)| holds(l)) {
                    None => [!0; 4],
                    Some(&(b, _)) => {
                        let readers: Vec<Reader> = readers[&pos].iter().copied().collect();
                        let sig = |x: u8| readers.iter().map(|&r| match r {
                            Reader::Eq(c) => x == c,
                            Reader::Class(t, bits) => tables[usize::from(t)].member(x, bits),
                        }).collect::<Vec<bool>>();
                        let want = sig(b);
                        let mut g = [0u64; 4];
                        for x in 0..=255u8 {
                            if sig(x) == want {
                                g[usize::from(x / 64)] |= 1 << (x % 64);
                            }
                        }
                        g
                    }
                };
                if edge.is_empty() {
                    return Outcome::Way(Vec::new(), group);
                }
                // And where the stream begins, past the horizon.
                let m = reach + 1;
                let mut way: Lemma = edge.into_iter().map(|(at, lag)| (at, lag - m, holds(u.memo[&(at, lag)]))).collect();
                way.extend((m..=far).map(|lag| (STREAM, lag - m, inside(lag))));
                Outcome::Way(way, group)
            }
        }
    }
}

/// An operation that is a lemma's values all holding: their `and`, each at its lag by a `prev`,
/// pushed on `ops`.
fn cube(ops: &mut Vec<BlockOp>, lemma: &Lemma) -> u16 {
    let push = |ops: &mut Vec<BlockOp>, op: BlockOp| {
        ops.push(op);
        (ops.len() - 1) as u16
    };
    let mut acc = push(ops, BlockOp::Const(u64::MAX));
    for &(at, lag, val) in lemma {
        let mut x = if at == STREAM { push(ops, BlockOp::Within(0)) } else { at };
        let mut l = lag;
        while l > 0 {
            let k = l.min(255);
            x = push(ops, BlockOp::Prev(x, k as u8));
            l -= k;
        }
        acc = push(ops, if val { BlockOp::And(acc, x) } else { BlockOp::AndNot(acc, x) });
    }
    acc
}

use crate::bitlex_sat::{Answer, Lit, Solver};

/// Whether `at` is a carried value with an equation (`never_unrolled`).
fn carried(ops: &[BlockOp], nexts: &HashMap<u16, u16>, at: u16) -> bool {
    match ops[usize::from(at)] {
        BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..) => true,
        BlockOp::Carry(_) => nexts.contains_key(&at),
        _ => false,
    }
}

/// The readers of each lag's byte, as `Unrolled::op` reads them.
/// And the farthest lag any operation is read at (a `within` at its byte's).
fn unrolled_readers(ops: &[BlockOp], nexts: &HashMap<u16, u16>, v: u16, reach: u32) -> (HashMap<u32, HashSet<Reader>>, u32) {
    let mut far = 0;
    let mut readers: HashMap<u32, HashSet<Reader>> = HashMap::new();
    let mut seen = HashSet::new();
    let mut work = vec![(v, 0u32)];
    while let Some((at, lag)) = work.pop() {
        if !seen.insert((at, lag)) {
            continue;
        }
        far = far.max(lag + 1);
        if let BlockOp::Within(w) = ops[usize::from(at)] {
            far = far.max(lag + u32::from(w));
        }
        if carried(ops, nexts, at) {
            if lag <= reach {
                match ops[usize::from(at)] {
                    BlockOp::Toggle(t) => work.extend([(at, lag + 1), (t, lag)]),
                    BlockOp::Escaped(a) => work.extend([(a, lag + 1), (at, lag + 1)]),
                    BlockOp::Fill(s, e) => work.extend([(s, lag), (e, lag), (at, lag + 1)]),
                    _ => work.push((nexts[&at], lag)),
                }
            }
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
            BlockOp::Not(a) => work.push((a, lag)),
            BlockOp::And(a, b) | BlockOp::Or(a, b) | BlockOp::Xor(a, b) | BlockOp::AndNot(a, b) => work.extend([(a, lag), (b, lag)]),
            BlockOp::Prev(a, k) => work.push((a, lag + u32::from(k))),
            _ => {}
        }
    }
    (readers, far)
}

struct Unrolled<'a> {
    ops: &'a [BlockOp],
    tables: &'a [ClassTable],
    nexts: &'a HashMap<u16, u16>,
    d: u32,
    reach: u32,
    clauses: Vec<Vec<Lit>>,
    vars: i32,
    memo: HashMap<(u16, u32), Lit>,
    /// Each lag past the position's: "the byte is before the stream".
    before: HashMap<u32, Lit>,
    /// Each lag short of the position's: "the byte is past the stream's end".
    past: HashMap<u32, Lit>,
    /// Each read lag's groups: a byte of each, and its literal.
    groups: HashMap<u32, Vec<(u8, Lit)>>,
    readers: HashMap<(Reader, u32, bool), Lit>,
}

impl Unrolled<'_> {
    fn fresh(&mut self) -> Lit {
        self.vars += 1;
        self.vars
    }

    fn and(&mut self, a: Lit, b: Lit) -> Lit {
        let x = self.fresh();
        self.clauses.extend([vec![-x, a], vec![-x, b], vec![x, -a, -b]]);
        x
    }

    fn or(&mut self, a: Lit, b: Lit) -> Lit {
        -self.and(-a, -b)
    }

    fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        let x = self.fresh();
        self.clauses.extend([vec![-x, a, b], vec![-x, -a, -b], vec![x, -a, b], vec![x, a, -b]]);
        x
    }

    /// `x` is `a` exactly: `x`'s equation.
    fn equal(&mut self, x: Lit, a: Lit) {
        self.clauses.extend([vec![-x, a], vec![x, -a]]);
    }

    /// Whether the byte at `lag` is in the stream: neither before it nor past its end.
    fn inside(&self, lag: u32) -> Lit {
        self.before.get(&lag).or(self.past.get(&lag)).map_or(1, |&b| -b)
    }

    /// A reader's value at a lag; `zero_before`: a `Pair`'s byte before, 0x00 before the stream.
    fn read(&mut self, r: Reader, lag: u32, zero_before: bool) -> Lit {
        if let Some(&x) = self.readers.get(&(r, lag, zero_before)) {
            return x;
        }
        let member = |t: &[ClassTable], x: u8| match r {
            Reader::Eq(c) => x == c,
            Reader::Class(table, bits) => t[usize::from(table)].member(x, bits),
        };
        let yes: Vec<Lit> = self.groups[&lag].iter().filter(|&&(b, _)| member(self.tables, b)).map(|&(_, l)| l).collect();
        let mut yes = yes;
        if zero_before && member(self.tables, 0)
            && let Some(&b) = self.before.get(&lag) {
                yes.push(b);
            }
        let x = self.fresh();
        let mut c = vec![-x];
        c.extend(&yes);
        self.clauses.push(c);
        for &y in &yes {
            self.clauses.push(vec![x, -y]);
        }
        self.readers.insert((r, lag, zero_before), x);
        x
    }

    /// `a`'s value at `lag`, as a `prev` reads it: 0 before the stream.
    /// (Past the stream's end a `prev` reads what the executor computed there: no 0.)
    fn prev(&mut self, a: u16, lag: u32) -> Lit {
        let x = self.op(a, lag);
        match self.before.get(&lag) {
            Some(&b) => self.and(-b, x),
            None => x,
        }
    }

    fn op(&mut self, at: u16, lag: u32) -> Lit {
        if let Some(&x) = self.memo.get(&(at, lag)) {
            return x;
        }
        if carried(self.ops, self.nexts, at) {
            // A variable first: its equation reads it a byte earlier.
            let x = self.fresh();
            self.memo.insert((at, lag), x);
            if lag <= self.reach {
                let want = match self.ops[usize::from(at)] {
                    BlockOp::Toggle(t) => {
                        let p = self.prev(at, lag + 1);
                        let t = self.op(t, lag);
                        self.xor(p, t)
                    }
                    BlockOp::Escaped(a) => {
                        let p = self.prev(a, lag + 1);
                        let q = self.prev(at, lag + 1);
                        self.and(p, -q)
                    }
                    BlockOp::Fill(s, e) => {
                        let s = self.op(s, lag);
                        let p = self.prev(at, lag + 1);
                        let e = self.op(e, lag);
                        let held = self.and(p, -e);
                        self.or(s, held)
                    }
                    _ => {
                        let n = self.nexts[&at];
                        self.op(n, lag)
                    }
                };
                self.equal(x, want);
            }
            return x;
        }
        let x = match self.ops[usize::from(at)] {
            BlockOp::Eq(c) => self.read(Reader::Eq(c), lag, false),
            BlockOp::Class { table, bits } => self.read(Reader::Class(table, bits), lag, false),
            BlockOp::Pair { before, table, bits } => {
                let x = self.read(Reader::Class(table, bits), lag, false);
                let y = self.read(Reader::Class(before, bits), lag + 1, true);
                self.and(x, y)
            }
            BlockOp::Const(0) => -1,
            BlockOp::Const(u64::MAX) => 1,
            BlockOp::Not(a) => -self.op(a, lag),
            BlockOp::And(a, b) => {
                let (a, b) = (self.op(a, lag), self.op(b, lag));
                self.and(a, b)
            }
            BlockOp::AndNot(a, b) => {
                let (a, b) = (self.op(a, lag), self.op(b, lag));
                self.and(a, -b)
            }
            BlockOp::Or(a, b) => {
                let (a, b) = (self.op(a, lag), self.op(b, lag));
                self.or(a, b)
            }
            BlockOp::Xor(a, b) => {
                let (a, b) = (self.op(a, lag), self.op(b, lag));
                self.xor(a, b)
            }
            BlockOp::Prev(a, k) => self.prev(a, lag + u32::from(k)),
            BlockOp::Within(w) => self.inside(lag + u32::from(w)),
            // Free: a fixed point's carry without its rounds, a position's pattern, a constant
            // word.
            _ => self.fresh(),
        };
        self.memo.insert((at, lag), x);
        x
    }
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

    /// The unrolled proof knows what the search does, and the carry's equation.
    #[test]
    fn unrolled_proofs() {
        let ops = vec![
            BlockOp::Eq(b'"'),
            BlockOp::Eq(b'\\'),
            BlockOp::Escaped(1),
            BlockOp::Toggle(0),
            BlockOp::AndNot(3, 3),
            BlockOp::Prev(1, 1),
            BlockOp::AndNot(2, 5),
            BlockOp::And(0, 2),
        ];
        let n = |v| never_unrolled(&ops, &[], &[], v, 0, 4, 1000);
        assert!(n(4));
        assert!(!n(3));
        assert!(n(6), "escaped(a) is after an a");
        assert!(!n(7));
        let ops = vec![BlockOp::Eq(b'a'), BlockOp::Const(0), BlockOp::Carry(1), BlockOp::Prev(0, 1), BlockOp::AndNot(2, 3), BlockOp::AndNot(3, 2)];
        let rounds = vec![vec![(2, 3)]];
        assert!(never_unrolled(&ops, &[], &rounds, 4, 0, 4, 1000));
        assert!(never_unrolled(&ops, &[], &rounds, 5, 0, 4, 1000));
        assert!(!never_unrolled(&ops, &[], &[], 4, 0, 4, 1000));
        // A toggle over a run of bytes none of which flips it: the same on both sides of it.
        // t = toggle('"'); x = t & ~prev(t, 3) & ~prev('"') & ~prev('"', 2) & ~'"': never.
        let ops = vec![
            BlockOp::Eq(b'"'),     // 0
            BlockOp::Toggle(0),    // 1
            BlockOp::Prev(1, 3),   // 2
            BlockOp::Prev(0, 1),   // 3
            BlockOp::Prev(0, 2),   // 4
            BlockOp::Or(3, 4),     // 5
            BlockOp::Or(5, 0),     // 6
            BlockOp::Or(6, 2),     // 7
            BlockOp::AndNot(1, 7), // 8
        ];
        assert!(never_unrolled(&ops, &[], &[], 8, 0, 4, 1000));
        assert!(!never_unrolled(&ops, &[], &[], 1, 0, 4, 1000));
    }

    /// Two fills of the same bytes are equal however long a run carries them: the window's edge
    /// may set them apart, and the lemma that they never are is proved by induction.
    #[test]
    fn induction_over_a_run() {
        let ops = vec![BlockOp::Eq(b'a'), BlockOp::Eq(b'z'), BlockOp::Fill(0, 1), BlockOp::Fill(0, 1), BlockOp::Xor(2, 3)];
        assert!(never_unrolled(&ops, &[], &[], 4, 0, 4, 1000));
        assert!(!never_unrolled(&ops, &[], &[], 2, 0, 4, 1000));
    }

    /// The stream ends once, and past its end the bytes are 0x00.
    #[test]
    fn the_end_of_the_stream() {
        let ops = vec![BlockOp::Within(0), BlockOp::Within(1), BlockOp::AndNot(0, 1), BlockOp::AndNot(1, 0), BlockOp::Eq(b'a'), BlockOp::AndNot(4, 0)];
        // A byte in the stream after one past its end: never.
        assert!(never_unrolled(&ops, &[], &[], 2, 2, 4, 1000));
        assert!(!never_unrolled(&ops, &[], &[], 3, 2, 4, 1000));
        // An `a` past the end: never.
        assert!(never_unrolled(&ops, &[], &[], 5, 1, 4, 1000));
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
