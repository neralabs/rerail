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

use crate::bitlex_grown::FormulaBlock;
use crate::block::{BlockOp, ClassTable};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// A set of bytes, bit `b` for byte `b`.
pub type ByteSet = [u64; 4];

/// A byte before the stream: no byte class has it, every carried value is 0 there.
const BEFORE: u16 = 256;

/// The tries per byte of the position.
const BUDGET: u32 = 1 << 12;

/// The evaluations a step of the search is (decision 0663): about a solver's step's time.
const EVALS_A_STEP: u64 = 32;

/// The evaluations the search takes of a question a solve is asked after (decision 0663): past
/// them it stops, not proved, and the solve answers. Of Kotlin's, the search proved none the
/// solver did not; it proved the cheap ones faster, and spent most of its time on those it then
/// failed, at 2^21 to 2^23 evaluations each.
const EVALS_BEFORE_SOLVE: u64 = 1 << 16;

/// The provers' work left to a compile, shared by every prover it makes (decision 0646): a
/// count of steps (32 of a search's evaluations, a solver's conflict, a window's variable), so a compile's
/// answers are the same on any machine and under any load, sized to about a second of proving.
/// Spent, a prover stops with its sound answer (not proved), as past its own limits.
///
/// A question takes at most a 16th of it (decision 0650, [`Proofs::ask`]): the steps below its
/// floor are not its. One hard question (an induction of 100,000 conflicts a solve)
/// took all of Kotlin's, and every question after it was answered not proved.
#[derive(Clone, Debug)]
pub struct Budget(std::sync::Arc<(std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64, u64, u64)>);

impl Budget {
    pub fn new(steps: u64) -> Self {
        Budget(std::sync::Arc::new((std::sync::atomic::AtomicU64::new(steps), std::sync::atomic::AtomicU64::new(0), steps / 16, steps)))
    }

    /// The steps the compile has taken: its budget less those left.
    pub fn spent(&self) -> u64 {
        self.0.3.saturating_sub(self.total())
    }

    /// A budget of its own for one question of a batch (decision 0654): a question's share of
    /// what is left, as `question` gives it, fixed when the batch begins, so the questions of a
    /// batch answered on several threads are answered as on one. The caller spends what each
    /// took ([`Budget::spend`] of its [`Budget::spent`]) when the batch is done.
    pub fn child(&self) -> Budget {
        Budget::new(self.left().min(self.0.2))
    }

    /// No limit: a question asked apart from a compile (the provers' own tests).
    pub fn unlimited() -> Self {
        Budget::new(u64::MAX)
    }

    fn total(&self) -> u64 {
        self.0.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn floor(&self) -> u64 {
        self.0.1.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The steps left to the question being asked (to the compile, outside one).
    pub fn left(&self) -> u64 {
        self.total().saturating_sub(self.floor())
    }

    /// `n` steps taken: whether any were left.
    pub fn spend(&self, n: u64) -> bool {
        let left = self.left();
        self.0.0.store(self.total().saturating_sub(n.min(left)), std::sync::atomic::Ordering::Relaxed);
        left > 0
    }

    /// Run `f` as one question: within a 16th of the compile's budget (and within the floor of
    /// the question it is asked in, if any). Unlimited, unlimited.
    fn question<T>(&self, f: impl FnOnce() -> T) -> T {
        let before = self.floor();
        let floor = before.max(self.total().saturating_sub(self.0.2));
        self.0.1.store(floor, std::sync::atomic::Ordering::Relaxed);
        let out = f();
        self.0.1.store(before, std::sync::atomic::Ordering::Relaxed);
        out
    }
}

impl Default for Budget {
    fn default() -> Self {
        Budget::unlimited()
    }
}

/// The steps a compile's provers take in all (`Budget`): about a second's, on an M-series core
/// in a debug build.
pub const PROVING_STEPS: u64 = 1 << 20;

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
    bytes: Bytes,
    vals: HashMap<(u16, u32), bool>,
    /// The carried values that may be chosen.
    cone: HashSet<(u16, u32)>,
    /// Each lag's byte groups: a byte of each.
    groups: HashMap<u32, Vec<u8>>,
    budget: u32,
    /// The position's own byte: the classes its readers tell apart, a byte of each and its
    /// members.
    own: Vec<(u8, ByteSet)>,
    /// Each `(op, lag)`'s value under the values chosen now, and the first unchosen one it
    /// turned on: a program shares its values, and a walk as a tree reads each again for every
    /// reader. Cleared whenever a value is chosen (`zero`).
    memo: std::cell::RefCell<Memo>,
    work: Budget,
    /// The evaluations made (`eval_op`), those spent as steps, and the most it may make.
    evals: std::cell::Cell<u64>,
    charged: u64,
    cap: u64,
}

impl<'a> Proof<'a> {
    fn of(ops: &'a [BlockOp], tables: &'a [ClassTable], v: u16, d: u8, work: &Budget) -> Proof<'a> {
        let (cone, readers, seen) = cone_walked(ops, v);
        let groups = readers.iter().map(|(&lag, r)| (lag, groups(tables, r))).collect();
        let own = classes(tables, readers.get(&u32::from(d)));
        let memo = std::cell::RefCell::new(Memo::of(ops, &seen));
        Proof { ops, tables, d: u32::from(d), b: 0, bytes: Bytes::default(), vals: HashMap::default(), cone, groups, budget: 0, own, memo, work: work.clone(), evals: std::cell::Cell::new(0), charged: 0, cap: u64::MAX }
    }

    /// Whether the value is zero with the position's byte `b`.
    fn zero_at(&mut self, v: u16, b: u8) -> bool {
        self.b = b;
        self.budget = BUDGET;
        self.zero(v)
    }

    fn byte(&self, lag: u32, open: &mut Option<Var>) -> Option<u16> {
        if lag == self.d {
            return Some(u16::from(self.b));
        }
        match self.bytes.get(lag) {
            Some(x) => Some(x),
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
        if let Some((r, first)) = self.memo.borrow().get(at, lag) {
            if let Some(v) = first {
                open.get_or_insert(v);
            }
            return r;
        }
        let mut first = None;
        let r = self.eval_op(at, lag, &mut first);
        self.memo.borrow_mut().set(at, lag, (r, first));
        if let Some(v) = first {
            open.get_or_insert(v);
        }
        r
    }

    fn eval_op(&self, at: u16, lag: u32, open: &mut Option<Var>) -> Option<bool> {
        self.evals.set(self.evals.get() + 1);
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
        // Past the compile's budget, or its own evaluations: not proved. Its steps are its
        // evaluations (decision 0663): a split of a large cone costs as much more as it reads.
        let e = self.evals.get();
        if e > self.cap {
            return false;
        }
        let steps = (e - self.charged) / EVALS_A_STEP;
        self.charged += steps * EVALS_A_STEP;
        if !self.work.spend(steps) {
            return false;
        }
        self.memo.get_mut().clear();
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
                let before_ok = lag > self.d && self.bytes.iter().all(|(l, x)| l < lag || x == BEFORE);
                let after_before = self.bytes.iter().any(|(l, x)| l < lag && x == BEFORE);
                let mut ways: Vec<u16> = Vec::new();
                if !after_before {
                    // No class reads it: one byte stands for every one.
                    ways.extend(self.groups.get(&lag).map_or_else(|| vec![0], |g| g.iter().map(|&x| u16::from(x)).collect()));
                }
                if before_ok || after_before {
                    ways.push(BEFORE);
                }
                ways.into_iter().all(|x| {
                    self.bytes.set(lag, x);
                    let z = self.zero(v);
                    self.bytes.unset(lag);
                    z
                })
            }
        }
    }
}

/// The bytes a proof has chosen, by lag: `UNSET` where none is.
#[derive(Default)]
struct Bytes(Vec<u16>);

const UNSET: u16 = u16::MAX;

impl Bytes {
    fn get(&self, lag: u32) -> Option<u16> {
        self.0.get(lag as usize).copied().filter(|&x| x != UNSET)
    }

    fn set(&mut self, lag: u32, x: u16) {
        let i = lag as usize;
        if i >= self.0.len() {
            self.0.resize(i + 1, UNSET);
        }
        self.0[i] = x;
    }

    fn unset(&mut self, lag: u32) {
        self.0[lag as usize] = UNSET;
    }

    /// Each lag chosen and its byte.
    fn iter(&self) -> impl Iterator<Item = (u32, u16)> + '_ {
        self.0.iter().enumerate().filter(|&(_, &x)| x != UNSET).map(|(l, &x)| (l as u32, x))
    }
}

/// A memo entry: a value, and the first unchosen value it turned on.
type Memoed = (Option<bool>, Option<Var>);

/// `Proof`'s memo (decision 0663): a slot for each operation and lag of the cone's walk (each
/// operation its run of lags), valid while its stamp is the memo's, so clearing it is one
/// increment; any other is kept in `spill`.
struct Memo {
    stamp: u32,
    /// Each operation's first slot, first lag and lags (0: none).
    at: Vec<(u32, u32, u32)>,
    slots: Vec<(u32, Memoed)>,
    spill: HashMap<(u16, u32), Memoed>,
}

impl Memo {
    fn of(ops: &[BlockOp], seen: &HashSet<(u16, u32)>) -> Memo {
        let n = seen.iter().map(|&(at, _)| usize::from(at) + 1).max().unwrap_or(0);
        let mut range = vec![(u32::MAX, 0u32); n];
        for &(at, lag) in seen {
            let r = &mut range[usize::from(at)];
            // A carried value is read a byte back too (its equation).
            let top = if carried_op(&ops[usize::from(at)]) { lag + 1 } else { lag };
            *r = (r.0.min(lag), r.1.max(top));
        }
        let mut at = Vec::with_capacity(n);
        let mut total = 0u32;
        for (lo, hi) in range {
            if lo == u32::MAX {
                at.push((0, 0, 0));
            } else {
                at.push((total, lo, hi - lo + 1));
                total += hi - lo + 1;
            }
        }
        Memo { stamp: 1, at, slots: vec![(0, (None, None)); total as usize], spill: HashMap::default() }
    }

    fn slot(&self, at: u16, lag: u32) -> Option<usize> {
        let &(base, lo, w) = self.at.get(usize::from(at))?;
        let k = lag.wrapping_sub(lo);
        (k < w).then_some((base + k) as usize)
    }

    fn get(&self, at: u16, lag: u32) -> Option<Memoed> {
        match self.slot(at, lag) {
            Some(i) => {
                let (stamp, m) = self.slots[i];
                (stamp == self.stamp).then_some(m)
            }
            None => self.spill.get(&(at, lag)).copied(),
        }
    }

    fn set(&mut self, at: u16, lag: u32, m: Memoed) {
        match self.slot(at, lag) {
            Some(i) => self.slots[i] = (self.stamp, m),
            None => {
                self.spill.insert((at, lag), m);
            }
        }
    }

    fn clear(&mut self) {
        self.stamp += 1;
        if !self.spill.is_empty() {
            self.spill.clear();
        }
    }
}

/// Whether an operation is a carried value (`Escaped`, `Toggle`, `Fill`), read a byte back by its equation.
fn carried_op(op: &BlockOp) -> bool {
    matches!(op, BlockOp::Escaped(_) | BlockOp::Toggle(_) | BlockOp::Fill(..))
}

/// The carried values `v` reads (`cone`), each once, in order.
pub fn carried_in(ops: &[BlockOp], v: u16) -> Vec<u16> {
    let mut out: Vec<u16> = cone(ops, v).0.into_iter().map(|(at, _)| at).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The carried values `v` reads, closed under their equations' operands, and the readers of
/// each lag's byte.
fn cone(ops: &[BlockOp], v: u16) -> (HashSet<(u16, u32)>, HashMap<u32, HashSet<Reader>>) {
    let (cone, readers, _) = cone_walked(ops, v);
    (cone, readers)
}

/// `cone`, and every operation and lag its walk reached.
fn cone_walked(ops: &[BlockOp], v: u16) -> (HashSet<(u16, u32)>, HashMap<u32, HashSet<Reader>>, HashSet<(u16, u32)>) {
    let mut cone = HashSet::default();
    let mut readers: HashMap<u32, HashSet<Reader>> = HashMap::default();
    let mut seen = HashSet::default();
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
            // A `back_fill` too: it reads bytes after, which the window does not hold; and an
            // `early`, a file's first bytes read ahead, and the prefix's positions (`proof_ops`
            // makes them zero).
            BlockOp::Early(..) | BlockOp::Prefix(..) | BlockOp::Within(_) | BlockOp::Phase(..) | BlockOp::Position(_) | BlockOp::Carry(_) | BlockOp::BackFill(..) | BlockOp::Cmp { .. } | BlockOp::Bit { .. } => {
                cone.insert((at, lag));
            }
        }
    }
    (cone, readers, seen)
}

/// A byte of each group the readers tell apart.
/// The bytes `readers` tell apart (none: one class of every byte), a byte of each class (its
/// least) and its members.
fn classes(tables: &[ClassTable], readers: Option<&HashSet<Reader>>) -> Vec<(u8, ByteSet)> {
    let readers: Vec<Reader> = readers.map(|r| r.iter().copied().collect()).unwrap_or_default();
    let sig = |x: u8| -> Vec<bool> {
        readers
            .iter()
            .map(|&r| match r {
                Reader::Eq(c) => x == c,
                Reader::Class(t, bits) => tables[usize::from(t)].member(x, bits),
            })
            .collect()
    };
    let mut out: Vec<(u8, ByteSet)> = Vec::new();
    let mut at: HashMap<Vec<bool>, usize> = HashMap::default();
    for x in 0..=255u8 {
        let k = *at.entry(sig(x)).or_insert_with(|| {
            out.push((x, [0; 4]));
            out.len() - 1
        });
        out[k].1[usize::from(x / 64)] |= 1 << (x % 64);
    }
    out
}

fn groups(tables: &[ClassTable], readers: &HashSet<Reader>) -> Vec<u8> {
    let of = group_of(tables, Some(readers));
    (0..=255u8).filter(|&x| of[usize::from(x)] == x).collect()
}

/// Each byte's group: the first byte the readers do not tell from it. With no readers (`None`),
/// one group: no test reads the byte.
fn group_of(tables: &[ClassTable], readers: Option<&HashSet<Reader>>) -> [u8; 256] {
    let mut of = [0u8; 256];
    let Some(readers) = readers else {
        return of;
    };
    // One class of every byte, split by each reader in turn: two bytes stay in one class where
    // every reader takes both or neither. Classes numbered in the order of their first bytes.
    let mut class = [0u16; 256];
    let mut classes = 1usize;
    let mut renumber = vec![u16::MAX; 512];
    for &r in readers {
        renumber[..2 * classes].fill(u16::MAX);
        let mut next = 0u16;
        for x in 0..=255u8 {
            let member = match r {
                Reader::Eq(c) => x == c,
                Reader::Class(t, bits) => tables[usize::from(t)].member(x, bits),
            };
            let k = 2 * usize::from(class[usize::from(x)]) + usize::from(member);
            if renumber[k] == u16::MAX {
                renumber[k] = next;
                next += 1;
            }
            class[usize::from(x)] = renumber[k];
        }
        classes = usize::from(next);
    }
    // A class's first byte: its number is the classes before it, so the first bytes come in order.
    let mut first = [0u8; 256];
    let mut seen = 0usize;
    for x in 0..=255u8 {
        let c = usize::from(class[usize::from(x)]);
        if c == seen {
            first[c] = x;
            seen += 1;
        }
        of[usize::from(x)] = first[c];
    }
    of
}

/// The bytes a value's set bits can be at, at delay `d` (bit `i` is byte `i - d`): each byte for
/// which the value is not proved zero. Sound: a byte left out never has a bit.
///
/// The position's byte is read only by its readers (`cone`), so the bytes they cannot tell
/// apart have one answer: one proof a class (`Proof::own`).
pub fn first_bytes(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> ByteSet {
    first_bytes_in(&Budget::unlimited(), ops, tables, v, d)
}

/// [`first_bytes`] within `work`.
pub fn first_bytes_in(work: &Budget, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> ByteSet {
    let mut p = Proof::of(ops, tables, v, d, work);
    let mut out = [0u64; 4];
    for (b, members) in std::mem::take(&mut p.own) {
        if !p.zero_at(v, b) {
            for i in 0..4 {
                out[i] |= members[i];
            }
        }
    }
    out
}

/// Two values (an operation and its delay) proved never to hold at one byte.
pub type Apart = ((u16, u8), (u16, u8));

/// Whether `v` is zero by its shape alone (decision 0650): of its conjuncts (through `And`,
/// `AndNot` and `Prev`), one is a disjunct (through `Or` and `Prev`) of a negated one at the same
/// lag, as a region's content row is its body less every part's bytes and a part's row is one of
/// those parts. A conjunct `prev(x, k)` holding says `x` holds `k` bytes back, in the stream, so
/// each is an operation at a lag. Or two of its conjuncts are a pair `apart` at one byte (two
/// regions' fills). No proof, no window.
pub fn never_by_shape(ops: &[BlockOp], apart: &[Apart], v: u16) -> bool {
    let (mut pos, mut neg, mut todo) = (HashSet::default(), Vec::new(), vec![(v, 0u32)]);
    while let Some((x, l)) = todo.pop() {
        // Every conjunct holds, a whole part's start as well as its own conjuncts.
        if !matches!(ops[usize::from(x)], BlockOp::Const(_)) && !pos.insert((x, l)) {
            continue;
        }
        match ops[usize::from(x)] {
            BlockOp::And(a, b) => todo.extend([(a, l), (b, l)]),
            BlockOp::AndNot(a, b) => {
                todo.push((a, l));
                neg.push((b, l));
            }
            BlockOp::Not(a) => neg.push((a, l)),
            BlockOp::Prev(a, k) => todo.push((a, l + u32::from(k))),
            // A constant is no byte's: it says nothing of a lag before the stream.
            _ => {}
        }
    }
    // A value at delay `d` read at lag `l` is the byte `l + d` back from the question's value.
    for &((a, da), (b, db)) in apart {
        let byte = |x: u16, d: u8| pos.iter().filter(move |&&(y, _)| y == x).map(move |&(_, l)| l + u32::from(d));
        if byte(a, da).any(|p| byte(b, db).any(|q| q == p)) {
            return true;
        }
    }
    let mut leaves = neg;
    while let Some((x, l)) = leaves.pop() {
        if pos.contains(&(x, l)) {
            return true;
        }
        match ops[usize::from(x)] {
            BlockOp::Or(a, b) => leaves.extend([(a, l), (b, l)]),
            BlockOp::Prev(a, k) => leaves.push((a, l + u32::from(k))),
            _ => {}
        }
    }
    false
}

/// Whether a value is zero at every byte of every stream, proved: `first_bytes` empty, stopped
/// at the first class of bytes not proved (most questions fail, and fail early).
pub fn never(ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
    never_in(&Budget::unlimited(), ops, tables, v, d)
}

/// [`never`] within `work`.
pub fn never_in(work: &Budget, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
    let mut p = Proof::of(ops, tables, v, d, work);
    std::mem::take(&mut p.own).into_iter().all(|(b, _)| p.zero_at(v, b))
}

/// [`never_in`] of a question a solve is asked after where it is not proved: within
/// `EVALS_BEFORE_SOLVE` evaluations (decision 0663).
pub fn never_in_before_solve(work: &Budget, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
    let mut p = Proof::of(ops, tables, v, d, work);
    p.cap = EVALS_BEFORE_SOLVE;
    std::mem::take(&mut p.own).into_iter().all(|(b, _)| p.zero_at(v, b))
}

/// Whether a value is zero at every byte of every stream, proved by a SAT solver over a window
/// of bytes (`Unrolled`), as `Induction::never`.
pub fn never_unrolled(ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>], v: u16, d: u8, horizon: u32, conflicts: u32) -> bool {
    Induction::new(rounds, horizon, conflicts).never(ops, tables, v, d)
}

/// A lemma: values that never hold together at a position, each an operation at a lag from it
/// (read as a `prev` reads it) and its value; `STREAM` for "the byte is in the stream".
pub type Lemma = Vec<(u16, u32, bool)>;

/// A lemma's value "the byte at this lag is in the stream" (`Lemma`).
const STREAM: u16 = u16::MAX;

/// Proofs over a window of bytes, by a SAT solver, with the lemmas they prove on the way.
///
/// The window is each operation at each lag the value reads it at, as a circuit (`bitlex_csat`, decision 0670).
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
/// after, to the question's end (a question's answer depends on it alone, so a later one starts
/// with none: decision 0414); the value is tried again. Not proved, or past the levels or rounds: not proved (sound,
/// a lemma is a proof of its own).
pub struct Induction {
    nexts: HashMap<u16, u16>,
    horizon: u32,
    conflicts: u32,
    lemmas: Vec<Lemma>,
    /// The solves left to the question being asked (`SOLVES`).
    solves: u32,
    /// The streams the bounded solves found, which refute a question before its proof.
    witnesses: Witnesses,
    /// The fixed points, as the program has them (`Witnesses` runs them).
    rounds: Vec<Vec<(u16, u16)>>,
    /// The windows made for the question being asked, by value, delay and the operations the
    /// question added up to the value (`question`: the first of them), and whether the root's
    /// hypothesis is made at every lag (`whole_hypothesis`).
    windows: HashMap<(u16, u8, bool, u64), Vec<usize>>,
    /// Those windows, each with the operations it was made of (`windows` finds it by their
    /// hash, with no copy of them).
    window_list: Vec<(Vec<BlockOp>, Window)>,
    /// The block its windows are built in (`bitlex_grown`): its forks', so a batch of their solves is in one block
    /// a kernel binds once (decision 0670, step 3). Made at its first window.
    block: std::sync::OnceLock<FormulaBlock>,
    question: usize,
    /// The solver, reset for each solve (its memory kept).
    solver: Solver,
    /// The lemmas a caller proved and gave (`add_lemma`, `never_inductive`, decision 0412), kept
    /// across its questions, unlike a question's own (`lemmas`). A prover's answers depend on
    /// them, so a prover given any is not asked through the keyed answers (`Proofs`, decision
    /// 0414), which are keyed by a question's content alone: `gap_ends`' prover is its own.
    given: Vec<Lemma>,
    /// The root's hypothesis is made at every lag of the window (`never_inductive`): a value
    /// the window reads at the position only is then assumed at the lags before it too.
    whole_hypothesis: bool,
    /// The compile's work left (`Budget`).
    work: Budget,
    /// A question asked in rounds (`never_round`, decision 0654): each solve's outcome so far, by
    /// its place in the question's solves, replayed as the question is asked again.
    replay: Vec<Replayed>,
    /// The place of the next solve in the question being asked.
    at: usize,
    /// Asked in rounds: the first solve past `replay` stops the question with its formula.
    capture: bool,
    captured: Option<Captured>,
    /// The solve the last round stopped at had a hypothesis, whose core it tells (`decided`).
    captured_core: bool,
    /// The core of the solve that proved the last `prove` (`Outcome::Proved`).
    core: Option<Vec<bool>>,
}

/// The solve a round stopped at (decision 0654), as the device reads it (decision 0670, step 3): its window, whose
/// gates and clauses hold the solve's own past the window's until the next round attaches it (`Window::kept`), its
/// variables, and its own gates' fanout, in the formula block.
struct Captured {
    window: usize,
    vars: usize,
    own: Option<Fanout>,
}

/// A solve's outcome in a question asked in rounds (decision 0654).
#[derive(Clone)]
enum Replayed {
    Known(Outcome),
    /// Its formula met no conflict on the GPU: solved on the CPU when the question reaches it
    /// again (its variables already spent).
    Cpu,
}

/// A round of a question (`Induction::never_round`).
pub enum Round {
    Answered(bool),
    /// Stopped at a solve: its formula is `Induction::formula`'s.
    Formula,
}

/// The version of what the provers answer, in every question's key (`Proofs`, decision 0414):
/// bumped with any change that can make one answer differently.
pub const PROOFS_VERSION: u32 = 5;

/// An answer the provers made: whether a value is zero everywhere, or the bytes it can be at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Never(bool),
    Bytes(ByteSet),
}

/// A question's key as it is kept: the first 16 bytes of the SHA-256 of its text (`question`). The
/// text is the question's whole cone, about 4.7 KB of Kotlin's 2,974 (14 MB); its digest names it
/// as well, in 16 bytes. Two questions share a digest with odds of about 2^-128 a pair: a wrong
/// answer that way is past any run's reach, as for content stores' keys.
pub type Key = [u8; 16];

/// `text`'s [`Key`].
pub fn key_of(text: &[u8]) -> Key {
    use sha2::Digest as _;
    let d = sha2::Sha256::digest(text);
    let mut k = [0u8; 16];
    k.copy_from_slice(&d[..16]);
    k
}

/// Answers by question's key (`question`, as [`Key`]).
pub type Known = HashMap<Key, Answer>;

/// The provers' answers by question (decision 0414): `known`, given to a compile, answers a
/// question without its proof; `made`, each question the compile answered otherwise, for the
/// caller to keep (as the engine keeps images, 0264); `used`, each known answer the compile took,
/// so its answers whole (`used` and `made`) can be shipped from a compile that proved few. A
/// question is its key (`question`): the answer depends on nothing else.
#[derive(Clone, Debug, Default)]
pub struct Proofs {
    pub known: Known,
    pub made: Vec<(Key, Answer)>,
    pub used: Vec<(Key, Answer)>,
    /// The compile's provers' work left (`Budget`).
    pub work: Budget,
}

impl Proofs {
    /// The answer to the question `text` (`question`): a known one, else `f`'s, kept in `made`.
    pub fn ask(&mut self, text: Vec<u8>, f: impl FnOnce() -> Answer) -> Answer {
        let key = key_of(&text);
        if let Some(&a) = self.known.get(&key) {
            self.used.push((key, a));
            return a;
        }
        // `RERAIL_PROVING_TRACE`: each question proved, its work and time, on stderr.
        let trace = std::env::var_os("RERAIL_PROVING_TRACE").is_some();
        let (before, t) = (self.work.left(), std::time::Instant::now());
        let a = self.work.question(f);
        if trace {
            let head = text.iter().position(|&b| b == b'\n').map_or(&text[..text.len().min(80)], |i| &text[..i.min(80)]);
            eprintln!("ask {:>10} steps {:>9.1} ms {}", before - self.work.left(), t.elapsed().as_secs_f64() * 1e3, String::from_utf8_lossy(head));
        }
        // Asked again in this compile (another pair's scopes, say): this answer, not a proof again.
        self.known.insert(key, a);
        self.made.push((key, a));
        a
    }

    /// `ask` for a question answered yes or no.
    pub fn never(&mut self, key: Vec<u8>, f: impl FnOnce() -> bool) -> bool {
        // Past the budget: not proved, with no proof begun.
        let out = self.work.left() == 0;
        matches!(self.ask(key, || Answer::Never(!out && f())), Answer::Never(true))
    }

    /// `ask` for a question answered by bytes.
    pub fn bytes(&mut self, key: Vec<u8>, f: impl FnOnce() -> ByteSet) -> ByteSet {
        // Past the budget: every byte, with no proof begun.
        let out = self.work.left() == 0;
        match self.ask(key, || Answer::Bytes(if out { [!0; 4] } else { f() })) {
            Answer::Bytes(b) => b,
            Answer::Never(_) => [!0; 4],
        }
    }
}

/// The bytes [`encode`] starts with: the format (keys as digests, [`Key`]) and the provers' version.
fn header() -> Vec<u8> {
    let mut out = b"RRANSWER".to_vec();
    out.extend(PROOFS_VERSION.to_le_bytes());
    out
}

/// Answers as bytes, for a store (decision 0414): `RRANSWER`, the provers' version (u32), then each
/// answer: its key (16 bytes), a tag (0 no, 1 yes, 2 bytes) and, for bytes, the four words. Sorted
/// by key, so the same answers are the same bytes. About 17 bytes an answer: Kotlin's 2,974 in 52 KB.
pub fn encode(answers: &[(Key, Answer)]) -> Vec<u8> {
    let mut sorted: Vec<&(Key, Answer)> = answers.iter().collect();
    sorted.sort_by_key(|a| a.0);
    sorted.dedup_by(|a, b| a.0 == b.0);
    let mut out = header();
    for (key, answer) in sorted {
        out.extend(key);
        match answer {
            Answer::Never(b) => out.push(u8::from(*b)),
            Answer::Bytes(set) => {
                out.push(2);
                set.iter().for_each(|w| out.extend(w.to_le_bytes()));
            }
        }
    }
    out
}

/// `encode`'s answers again: none when the bytes are not of this format and version (their keys would
/// not be asked), and those before the first malformed one.
pub fn decode(bytes: &[u8]) -> Known {
    let mut known = Known::default();
    let head = header();
    let Some(mut rest) = bytes.strip_prefix(&head[..]) else { return known };
    let mut take = |n: usize| -> Option<&[u8]> {
        let (a, b) = (rest.get(..n)?, rest.get(n..)?);
        rest = b;
        Some(a)
    };
    let mut one = || -> Option<(Key, Answer)> {
        let key: Key = take(16)?.try_into().ok()?;
        let answer = match take(1)?[0] {
            0 => Answer::Never(false),
            1 => Answer::Never(true),
            2 => {
                let mut set = [0u64; 4];
                for w in &mut set {
                    *w = u64::from_le_bytes(take(8)?.try_into().ok()?);
                }
                Answer::Bytes(set)
            }
            _ => return None,
        };
        Some((key, answer))
    };
    while let Some((key, answer)) = one() {
        known.insert(key, answer);
    }
    known
}

/// A question's key (decision 0414): what the prover reads of it and nothing else. The value's
/// cone (the operations it reads, through the fixed points' nexts), numbered in the order a walk
/// from the value meets them, each class by its table's bytes; the delay; and `asker`, the prover
/// and its settings. The same question in another grammar, or at other numbers, has this key.
pub fn question(ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>], v: u16, d: u8, asker: &str) -> Vec<u8> {
    question_given(ops, tables, rounds, v, d, asker, &[])
}

/// `question`, asked of a prover given `given` (`Induction::given`): the lemmas whose values all lie in the cone, as
/// the cone numbers them, are in the key too, as the answer may use them (a walk's kinds apart, decision 0484, which
/// the equations do not show). A lemma with a value outside the cone is never applied (the window is the cone's).
pub fn question_given(ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>], v: u16, d: u8, asker: &str, given: &[Lemma]) -> Vec<u8> {
    use std::fmt::Write;
    let nexts: HashMap<u16, u16> = rounds.iter().flatten().copied().collect();
    let mut number: HashMap<u16, u16> = HashMap::default();
    let mut order: Vec<u16> = Vec::new();
    let mut work = vec![v];
    while let Some(at) = work.pop() {
        if number.contains_key(&at) {
            continue;
        }
        number.insert(at, order.len() as u16);
        order.push(at);
        let op = ops[usize::from(at)];
        if let (BlockOp::Carry(_), Some(&n)) = (op, nexts.get(&at)) {
            work.push(n);
        }
        work.extend(op.operands().into_iter().rev());
    }
    let mut out = format!("v{PROOFS_VERSION} {asker} d{d}");
    let table = |t: u16| tables.get(usize::from(t)).map(|t| (t.lo, t.hi));
    for &at in &order {
        let op = ops[usize::from(at)].renumbered(|x| number[&x]);
        let _ = match op {
            BlockOp::Class { table: t, bits } => write!(out, ";class {bits} {:?}", table(t)),
            BlockOp::Pair { before, table: t, bits } => write!(out, ";pair {bits} {:?} {:?}", table(before), table(t)),
            BlockOp::Carry(_) => write!(out, ";{op:?} {:?}", nexts.get(&at).map(|n| number[n])),
            _ => write!(out, ";{op:?}"),
        };
    }
    let mut lemmas: Vec<Vec<(u16, u32, bool)>> = given
        .iter()
        .filter(|l| l.iter().all(|(at, _, _)| number.contains_key(at)))
        .map(|l| {
            let mut l: Vec<(u16, u32, bool)> = l.iter().map(|&(at, lag, b)| (number[&at], lag, b)).collect();
            l.sort_unstable();
            l
        })
        .collect();
    lemmas.sort_unstable();
    lemmas.dedup();
    for l in &lemmas {
        let _ = write!(out, ";lemma {l:?}");
    }
    out.into_bytes()
}

/// The streams a compile's solves found, shared by its questions (decision 0654): a question one
/// of them refutes is answered with no prover. A batch's questions are asked by forks, each with
/// streams of its own (`Induction::found`); the batch's caller keeps them here for the next. At
/// most `STREAMS`.
#[derive(Default)]
pub struct Streams(Witnesses);

/// The most streams a compile keeps (`Streams`).
const STREAMS: usize = 4096;

impl Streams {
    pub fn add(&mut self, files: impl IntoIterator<Item = Vec<u8>>) {
        for f in files {
            if self.0.files.len() < STREAMS && !self.0.files.contains(&f) {
                self.0.files.push(f);
                self.0.stale = true;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.0.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.files.is_empty()
    }

    /// Whether `v` at delay `d` holds inside one of the streams.
    pub fn refutes(&mut self, ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>], v: u16, d: u8) -> bool {
        self.0.refutes(ops, tables, rounds, v, d)
    }
}

/// The most witnesses kept (`Witnesses`): each is evaluated for every question asked.
const WITNESSES: usize = 256;

/// Real streams, each one a bounded solve found (a way where the stream begins within the
/// window, so every carried value has its equation: `Induction::solve`), with every operation's
/// words on them as the interpreter makes them (`BlockProgram::rounds_values`, the walks' groups
/// as rounds, as the proofs read them). A value with a bit inside one of them is not zero on
/// every stream: the question is refuted by evaluating it, with no proof. Most questions a
/// grammar's compile asks fail (a row's start fixes no mode, two rows can start at a byte), and a
/// failing proof is the dear one: the SAT solves until a way, at every level. The values of
/// operations the asker appends after the witnesses' were made (an `and` of two values, its
/// question) are made from theirs, word by word.
#[derive(Default)]
struct Witnesses {
    files: Vec<Vec<u8>>,
    /// The operations `values` are of, and their words over the files end to end.
    ops: Vec<BlockOp>,
    values: Vec<Vec<u64>>,
    /// Each file's blocks in `values`, and whether a block is its file's first.
    spans: Vec<std::ops::Range<usize>>,
    first: Vec<bool>,
    /// A file added since the values were made.
    stale: bool,
    /// The values that read a carry no rounds group names (`remake`): refuting nothing.
    tainted: Vec<bool>,
}

impl Witnesses {
    fn add(&mut self, file: Vec<u8>) {
        if self.files.len() < WITNESSES && !self.files.contains(&file) {
            self.files.push(file);
            self.stale = true;
        }
    }

    /// Whether `v` (at delay `d`) has a bit inside one of the files: at a position `p` that is
    /// byte `p - d`'s, `d <= p < len + d`.
    fn refutes(&mut self, ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>], v: u16, d: u8) -> bool {
        if self.files.is_empty() || !self.sync(ops, tables, rounds) {
            return false;
        }
        if self.tainted.get(usize::from(v)).copied().unwrap_or(true) {
            return false;
        }
        let words = &self.values[usize::from(v)];
        self.files.iter().zip(&self.spans).any(|(f, span)| {
            let (lo, hi) = (usize::from(d), f.len() + usize::from(d));
            (lo..hi).step_by(64).any(|p| {
                let (w, b) = (span.start + p / 64, p % 64);
                // The bits from `p` to the word's end or `hi`.
                let n = (64 - b).min(hi - p);
                let m = if n == 64 { !0 } else { ((1u64 << n) - 1) << b };
                words.get(w).is_some_and(|x| x & m != 0)
            })
        })
    }

    /// The values made for `ops`: the appended operations' from the others', else all again.
    /// False where they can not be made (no stream to make them over).
    fn sync(&mut self, ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>]) -> bool {
        let n = self.ops.len();
        if !self.stale && ops.len() >= n && ops[..n] == self.ops[..] {
            for (i, op) in ops.iter().enumerate().skip(n) {
                let Some(words) = self.combine(*op) else {
                    return self.remake(ops, tables, rounds);
                };
                debug_assert_eq!(self.values.len(), i);
                self.values.push(words);
                self.tainted.push(op.operands().into_iter().any(|o| self.tainted[usize::from(o)]));
                self.ops.push(*op);
            }
            return true;
        }
        self.remake(ops, tables, rounds)
    }

    /// An operation of values made already, word by word: `None` for one that reads the bytes
    /// or carries (made by the interpreter, `remake`).
    fn combine(&self, op: BlockOp) -> Option<Vec<u64>> {
        let at = |x: u16| &self.values[usize::from(x)];
        let two = |a: u16, b: u16, f: fn(u64, u64) -> u64| at(a).iter().zip(at(b)).map(|(&x, &y)| f(x, y)).collect();
        Some(match op {
            BlockOp::Const(c) => vec![c; self.first.len()],
            BlockOp::Not(a) => at(a).iter().map(|&x| !x).collect(),
            BlockOp::And(a, b) => two(a, b, |x, y| x & y),
            BlockOp::Or(a, b) => two(a, b, |x, y| x | y),
            BlockOp::Xor(a, b) => two(a, b, |x, y| x ^ y),
            BlockOp::AndNot(a, b) => two(a, b, |x, y| x & !y),
            BlockOp::Prev(a, k) if (1..64).contains(&k) => {
                let x = at(a);
                (0..x.len()).map(|i| crate::block::carry::prev(x[i], if self.first[i] { 0 } else { x[i - 1] }, k)).collect()
            }
            _ => return None,
        })
    }

    /// A carry no rounds group names is an earlier try's of a `rounds` statement, whose delays
    /// did not settle (`Lowering`'s rounds: each try makes its carries anew, and the program
    /// keeps the operations): made zero here, and the values that read it tainted, refuting
    /// nothing. The values the program reads are the last try's.
    fn remake(&mut self, ops: &[BlockOp], tables: &[ClassTable], rounds: &[Vec<(u16, u16)>]) -> bool {
        let named: HashSet<u16> = rounds.iter().flatten().map(|p| p.0).collect();
        let mut tainted = vec![false; ops.len()];
        let mut made = ops.to_vec();
        for (i, op) in ops.iter().enumerate() {
            if matches!(op, BlockOp::Carry(_)) && !named.contains(&(i as u16)) {
                tainted[i] = true;
                made[i] = BlockOp::Const(0);
            } else {
                tainted[i] = op.operands().into_iter().any(|o| tainted[usize::from(o)]);
            }
        }
        let program = crate::block::BlockProgram { tables: tables.to_vec(), ops: made.clone(), rounds: rounds.to_vec(), delay: 63, ..Default::default() };
        if crate::block::groups(&made, rounds).is_err() {
            return false;
        }
        self.tainted = tainted;
        let files: Vec<&[u8]> = self.files.iter().map(|f| f.as_slice()).collect();
        let (values, spans) = program.rounds_values(&files);
        self.first = vec![false; values.first().map_or(0, |v| v.len())];
        for s in &spans {
            if let Some(f) = self.first.get_mut(s.start) {
                *f = true;
            }
        }
        (self.values, self.spans, self.ops, self.stale) = (values, spans, ops.to_vec(), false);
        true
    }
}

/// The solves a question may take, lemmas included: past them, not proved.
const SOLVES: u32 = 64;

/// The levels of lemmas under a proof, and the rounds of each.
const LEVELS: u32 = 3;
const ROUNDS: u32 = 24;

#[derive(Clone)]
enum Outcome {
    /// No way; with a hypothesis, each of its values whether the contradiction read it at the position (its core,
    /// `Solver::used`; decision 0670), where the solver could tell.
    Proved(Option<Vec<bool>>),
    /// A way: its edge (the values past the horizon, in the stream), and the group of the
    /// position's byte (every byte when nothing reads it).
    Way(Lemma, ByteSet),
    Unknown,
}

impl Induction {
    pub fn new(rounds: &[Vec<(u16, u16)>], horizon: u32, conflicts: u32) -> Self {
        Induction { nexts: rounds.iter().flatten().copied().collect(), horizon, conflicts, lemmas: Vec::new(), solves: 0, witnesses: Witnesses::default(), rounds: rounds.to_vec(), windows: HashMap::default(), window_list: Vec::new(), block: std::sync::OnceLock::new(), question: 0, solver: Solver::default(), given: Vec::new(), whole_hypothesis: false, work: Budget::unlimited(), replay: Vec::new(), at: 0, capture: false, captured: None, captured_core: false, core: None }
    }

    /// This prover within the compile's budget `work`.
    pub fn within(mut self, work: &Budget) -> Self {
        self.work = work.clone();
        self
    }

    /// A prover of these settings and given lemmas, with its own windows, solver and streams, and
    /// `work` for its budget: for a question of a batch answered on another thread (decision
    /// 0654). Its answers are this one's: the streams it has not seen only refute questions a
    /// solve would refute.
    /// The last question's solves, its solver's conflicts in all, its lemmas and its windows
    /// (`RERAIL_PROVING_TRACE`).
    pub fn stats(&self) -> (u32, u64, usize, usize) {
        (SOLVES.saturating_sub(self.solves), self.solver.conflicts, self.lemmas.len(), self.window_list.len())
    }

    pub fn fork(&self, work: &Budget) -> Induction {
        let mut p = Induction::new(&self.rounds, self.horizon, self.conflicts).within(work);
        p.given = self.given.clone();
        p.block = self.block().clone().into();
        p
    }

    /// The block its windows are built in.
    pub fn block(&self) -> &FormulaBlock {
        self.block.get_or_init(FormulaBlock::new)
    }

    /// `never` in rounds (decision 0654): the question asked again from its start, each solve
    /// reached before answered from `replay`, until it is answered or reaches a new solve, whose
    /// formula it returns; `decided` says how that solve came out. The solves happen in the
    /// order `never`'s do, with the same outcomes, so the answer is `never`'s. Not `stop`: the
    /// new solves are made on the CPU, to the answer.
    pub fn never_round(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8, stop: bool) -> Round {
        if self.replay.is_empty() && self.refutes(ops, tables, v, d) {
            return Round::Answered(false);
        }
        let mut ops = ops.to_vec();
        self.ask(&ops);
        self.solves = SOLVES;
        (self.at, self.capture, self.captured) = (0, stop, None);
        let proved = self.prove(&mut ops, tables, v, d, LEVELS, None, &[0; 4]);
        self.capture = false;
        match self.captured {
            Some(_) => Round::Formula,
            None => Round::Answered(proved),
        }
    }

    /// The formula of the solve the last round stopped at (`Round::Formula`), as the device's solver reads it, with
    /// a budget of `conflicts`.
    #[cfg(not(target_family = "wasm"))]
    pub fn formula(&self, conflicts: u32) -> Option<crate::bitlex_csat_gpu::Formula<'_>> {
        let c = self.captured.as_ref()?;
        let w = &self.window_list[c.window].1;
        let gates = &w.parts.gates;
        Some(crate::bitlex_csat_gpu::Formula { vars: c.vars, gates, base: w.base_gates, fan: &w.fan, own: c.own.as_ref(), clauses: &w.parts.clauses, conflicts })
    }

    /// The outcome of the solve the last round stopped at: unsatisfiable by propagation alone
    /// (`conflict`, as the CPU's solver finds it with no conflict spent), else the CPU's to solve.
    pub fn decided(&mut self, conflict: bool) {
        // A solve that tells its core is the CPU's: the device's conflict does not say what it read (decision 0670).
        self.replay.push(if conflict && !self.captured_core { Replayed::Known(Outcome::Proved(None)) } else { Replayed::Cpu });
    }

    /// The streams this prover's solves found (`Streams`).
    pub fn found(&self) -> &[Vec<u8>] {
        &self.witnesses.files
    }

    /// This prover and its settings, asking `what`: a part of each question's key (`question`).
    pub fn asker(&self, what: &str) -> String {
        format!("induction {what} h{} c{} s{SOLVES} l{LEVELS} r{ROUNDS}", self.horizon, self.conflicts)
    }

    /// Whether `v` (at delay `d`) has a bit in a stream a solve found before (`Witnesses`): then
    /// it is not zero everywhere, and no proof is needed. A question a caller would send to
    /// another prover first (`never`) can ask this before it.
    pub fn refutes(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
        self.witnesses.refutes(ops, tables, &self.rounds, v, d)
    }

    /// Whether `v` is zero at every byte of every stream, at delay `d`.
    pub fn never(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
        if self.refutes(ops, tables, v, d) {
            return false;
        }
        let mut ops = ops.to_vec();
        self.ask(&ops);
        self.solves = SOLVES;
        self.prove(&mut ops, tables, v, d, LEVELS, None, &[0; 4])
    }

    /// `never`, by induction on the stream's positions with `v` itself as the hypothesis: the
    /// first position where `v` holds has none before it, so the step may assume `v` at every
    /// lag past the position's (sound: the base, a stream that begins within the window, is
    /// `prove`'s first solve). A value whose own past rules it out (a sticky flag, a region
    /// that cannot begin inside another) is proved so where `never` finds no lemma.
    pub fn never_inductive(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
        let mut ops = ops.to_vec();
        self.ask(&ops);
        self.solves = SOLVES;
        let hypothesis: Lemma = vec![(v, 0, true)];
        self.whole_hypothesis = true;
        let proved = self.prove(&mut ops, tables, v, d, LEVELS, Some(&hypothesis), &[0; 4]);
        self.whole_hypothesis = false;
        // Proved, it is a lemma of the proofs after (their operations are these and more): `v`
        // holds at no position.
        if proved {
            self.given.push(hypothesis);
        }
        proved
    }

    /// Whether a stream that begins within the window has `v` at delay `d` (`prove`'s first solve): a stream's, so
    /// no lemma rules it out and `never` and `never_inductive` both fail on `v`.
    pub fn refuted(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8) -> bool {
        matches!(self.solve(ops, tables, v, d, None, true, &[0; 4]), Outcome::Way(..))
    }

    /// A lemma proved by the caller: these values (each an operation, a lag from the position
    /// as a `prev` reads it, and its value) never hold together at a position in the stream.
    /// Over carried values, it constrains a window's edge, where they are free. The caller
    /// proves it first (`never_inductive` of their conjunction).
    pub fn add_lemma(&mut self, lemma: Vec<(u16, u32, bool)>) {
        self.given.push(lemma);
    }

    /// The lemmas given so far (`add_lemma`, `never_inductive`): what a keyed answer may depend on beside its
    /// question (`question_given`).
    pub fn given(&self) -> &[Lemma] {
        &self.given
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
        self.ask(&ops);
        let mut found = [0u64; 4];
        for _ in 0..256 {
            match self.solve(&ops, tables, v, d, None, true, &found) {
                Outcome::Way(_, group) if group != [0; 4] && (0..4).any(|i| group[i] & !found[i] != 0) => {
                    for i in 0..4 {
                        found[i] |= group[i];
                    }
                }
                Outcome::Proved(_) => {
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
                Outcome::Proved(core) => {
                    self.core = core;
                    return true;
                }
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
            // Smaller: to the values its proof's contradiction read at the position (its core, decision 0670),
            // proved again, while that drops any; then each value dropped while the rest are still proved never to
            // hold, and each drop's core taken too, while solves are left. An edge is every carried value past the
            // horizon, and most are not why it never holds: one at a time, a lemma of 254 values spent the
            // question's solves.
            self.by_core(ops, tables, level, &mut lemma, &mut 0);
            let mut i = 0;
            while i < lemma.len() && lemma.len() > 1 && self.solves > 0 {
                let mut fewer = lemma.clone();
                fewer.remove(i);
                let c = cube(ops, &fewer);
                if self.prove(ops, tables, c, 0, level - 1, Some(&fewer), &[0; 4]) {
                    lemma = fewer;
                    self.by_core(ops, tables, level, &mut lemma, &mut i);
                } else {
                    i += 1;
                }
            }
            ops.truncate(base);
            self.lemmas.push(lemma);
        }
        false
    }

    /// `lemma`, proved by the last `prove`, cut to that proof's core (`Outcome::Proved`) and proved again, while
    /// the core leaves any value out (decision 0670). `tried`: the values before it were tried already; it is
    /// moved past those of them kept.
    fn by_core(&mut self, ops: &mut Vec<BlockOp>, tables: &[ClassTable], level: u32, lemma: &mut Lemma, tried: &mut usize) {
        while let Some(core) = self.core.take() {
            let kept = core.iter().filter(|&&k| k).count();
            if kept == 0 || kept == lemma.len() || self.solves == 0 {
                return;
            }
            let smaller: Lemma = lemma.iter().zip(&core).filter(|&(_, &k)| k).map(|(&e, _)| e).collect();
            let c = cube(ops, &smaller);
            if !self.prove(ops, tables, c, 0, level - 1, Some(&smaller), &[0; 4]) {
                return;
            }
            *tried = core[..(*tried).min(core.len())].iter().filter(|&&k| k).count();
            *lemma = smaller;
        }
    }

    /// A new question, of `ops`: the windows of the last are dropped (its operations may not
    /// be these).
    fn ask(&mut self, ops: &[BlockOp]) {
        // A question's lemmas are its own: its answer depends on it alone (decision 0414).
        self.lemmas.clear();
        // Asked again in rounds (`never_round`): its windows are the same.
        if self.replay.is_empty() {
            self.windows.clear();
            self.window_list.clear();
        }
        self.question = ops.len();
    }

    /// `bounded`: the stream begins within the window (before it past the horizon).
    /// In rounds (`never_round`): a solve made before is its outcome; past a stopped one, none
    /// is made.
    #[allow(clippy::too_many_arguments)]
    fn solve(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8, hypothesis: Option<&Lemma>, bounded: bool, exclude: &ByteSet) -> Outcome {
        if self.captured.is_some() {
            return Outcome::Unknown;
        }
        let i = self.at;
        self.at += 1;
        if let Some(Replayed::Known(o)) = self.replay.get(i) {
            return o.clone();
        }
        let o = self.solve_now(ops, tables, v, d, hypothesis, bounded, exclude, i);
        if self.capture && i < self.replay.len() {
            self.replay[i] = Replayed::Known(o.clone());
        }
        o
    }

    #[allow(clippy::too_many_arguments)]
    fn solve_now(&mut self, ops: &[BlockOp], tables: &[ClassTable], v: u16, d: u8, hypothesis: Option<&Lemma>, bounded: bool, exclude: &ByteSet, i: usize) -> Outcome {
        let nexts = &self.nexts;
        // The window as the operations make it, before the solve's own clauses (the bounded
        // start, the bytes left out, the lemmas): the same for every solve of a question's value
        // (`Window`).
        let end = ops.len().min(usize::from(v) + 1);
        // The root's hypothesis at every lag (`whole_hypothesis`), where it is the root's own.
        let whole = hypothesis.filter(|h| self.whole_hypothesis && h.iter().any(|&(at, _, _)| at == v));
        let made_of = &ops[self.question.min(end)..end];
        let key = (v, d, whole.is_some(), {
            use std::hash::{BuildHasher as _, Hash as _, Hasher as _};
            let mut h = rustc_hash::FxBuildHasher.build_hasher();
            made_of.hash(&mut h);
            h.finish()
        });
        let found = self.windows.get(&key).and_then(|js| js.iter().copied().find(|&j| self.window_list[j].0 == made_of));
        let j = match found {
            Some(j) => j,
            None => {
            let window = {
                let (_, plain) = cone(ops, v);
                let reach = plain.keys().copied().max().unwrap_or(0).max(u32::from(d)) + self.horizon;
                // The root at its lag, and the hypothesis' values at every lag of the window where they are made there
                // (`whole`): their readers too, so each byte `op` reads has its groups; one walk from every root.
                let mut roots = vec![(v, 0u32)];
                for lag in 1..=reach {
                    roots.extend(whole.into_iter().flatten().filter(|&&(at, _, _)| at != STREAM).map(|&(at, l, _)| (at, lag + l)));
                }
                let (readers, far) = unrolled_readers(ops, nexts, &roots, reach);
                let mut u = Unrolled { ops, tables, nexts, d: u32::from(d), reach, gates: Gates::in_block(self.block()), clauses: Clauses::in_block(self.block()), vars: 1, memo: HashMap::default(), before: HashMap::default(), past: HashMap::default(), groups: HashMap::default(), readers: HashMap::default(), memo_added: Vec::new(), readers_added: Vec::new() };
                // A literal that is true.
                u.clauses.push(&[1]);
                // Before the stream: from some lag past the position's on.
                for lag in u.d + 1..=far {
                    let b = u.fresh();
                    u.before.insert(lag, b);
                    if let Some(&p) = u.before.get(&(lag - 1)) {
                        u.clauses.push(&[-p, b]);
                    }
                }
                // Past its end: from some lag short of the position's down (the stream ends once).
                for lag in (0..u.d).rev() {
                    let x = u.fresh();
                    u.past.insert(lag, x);
                    if let Some(&p) = u.past.get(&(lag + 1)) {
                        u.clauses.push(&[-p, x]);
                    }
                }
                let mut lags: Vec<u32> = readers.keys().copied().collect();
                lags.sort_unstable();
                for lag in lags {
                    let gs = groups(tables, &readers[&lag]);
                    let vs: Vec<Lit> = gs.iter().map(|_| u.fresh()).collect();
                    let b = u.before.get(&lag).copied();
                    // Exactly one group, or none before the stream.
                    u.gates.one(&vs.iter().copied().chain(b).collect::<Vec<_>>());
                    // Past the end the bytes are 0x00 (`block::padded`): the group of 0x00 is the one
                    // `groups` gives first.
                    if let Some(&p) = u.past.get(&lag) {
                        u.clauses.push(&[-p, vs[0]]);
                    }
                    u.groups.insert(lag, gs.into_iter().zip(vs).collect());
                }
                let root = u.op(v, 0);
                u.clauses.push(&[root]);
                let at_position: Vec<Reader> = readers.get(&u32::from(d)).map(|r| r.iter().copied().collect()).unwrap_or_default();
                u.detach(far, at_position)
            };
            self.window_list.push((made_of.to_vec(), window));
            let j = self.window_list.len() - 1;
            self.windows.entry(key).or_default().push(j);
            j
            }
        };
        let window = &mut self.window_list[j].1;
        let mut u = window.attach(ops, tables, nexts);
        let mut keep = false;
        let out = 'solve: {
        let Window { far, reach, at_position, held, fan, base_gates, .. } = &*window;
        let (far, reach, base_gates) = (*far, *reach, *base_gates);
        if bounded && let Some(&b) = u.before.get(&(reach + 1)) {
            u.clauses.push(&[b]);
        }
        // The position's byte is none of `exclude`'s: each group of them left out (a group is
        // one way of its readers, so all its bytes are or none).
        let has = |set: &ByteSet, b: u8| set[usize::from(b / 64)] >> (b % 64) & 1 == 1;
        let pos = u32::from(d);
        let position: Vec<(u8, Lit)> = u.groups.get(&pos).cloned().unwrap_or_default();
        if *exclude != [0; 4] {
            if position.is_empty() {
                break 'solve Outcome::Unknown;
            }
            for &(b, l) in &position {
                if has(exclude, b) {
                    u.clauses.push(&[-l]);
                }
            }
        }
        // The root's hypothesis at every lag of the window (`whole_hypothesis`), its values made
        // there: each is the operations' own equations, so this adds no assumption.
        if let Some(h) = whole {
            for lag in 1..=reach {
                for &(at, l, _) in h.iter().filter(|&&(at, _, _)| at != STREAM) {
                    u.op(at, lag + l);
                }
            }
        }
        // The lemmas, at each lag of the window whose values they read are in it.
        // The window's values and those the hypothesis made: sorted, so a value's lags are a run.
        let held: std::borrow::Cow<'_, [(u16, u32)]> = if u.memo_added.is_empty() {
            std::borrow::Cow::Borrowed(held.as_slice())
        } else {
            let mut all: Vec<(u16, u32)> = held.iter().copied().chain(u.memo_added.iter().copied()).collect();
            all.sort_unstable();
            std::borrow::Cow::Owned(all)
        };
        let lemmas = self.lemmas.iter().chain(&self.given).map(|l| (l, 0)).chain(hypothesis.map(|l| (l, 1)));
        for (lemma, from) in lemmas {
            let Some(&(a0, l0, _)) = lemma.iter().find(|&&(at, _, _)| at != STREAM) else { continue };
            let from_a0 = held.partition_point(|&(at, _)| at < a0);
            for &(_, lag) in held[from_a0..].iter().take_while(|&&(at, _)| at == a0) {
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
                u.clauses.push(&c);
            }
        }
        // Past the compile's budget: not proved. A window's variables and the solve's conflicts
        // are its steps.
        let spent = matches!(self.replay.get(i), Some(Replayed::Cpu));
        if !spent && !self.work.spend(u.vars.unsigned_abs().into()) {
            break 'solve Outcome::Unknown;
        }
        // In rounds, a new solve: its formula, for the batch (`never_round`), where it was built: the window keeps
        // the solve's own gates and clauses until it is attached again.
        if self.capture && i == self.replay.len() {
            let vars = u.vars.unsigned_abs() as usize;
            self.captured = Some(Captured { window: j, vars, own: (base_gates < u.gates.len()).then(|| Fanout::of_range(&u.gates, base_gates..u.gates.len(), vars)) });
            self.captured_core = hypothesis.is_some();
            keep = true;
            break 'solve Outcome::Unknown;
        }
        let s = &mut self.solver;
        // The window's gates as they stand (their fanout made once), its plain clauses and the solve's own.
        s.reset(u.vars as usize);
        for c in u.clauses.iter() {
            s.add(c);
        }
        let before = s.conflicts;
        let t_dump = std::time::Instant::now();
        let answer = s.solve(&u.gates, fan, base_gates, self.conflicts.min(u32::try_from(self.work.left()).unwrap_or(u32::MAX)));
        // With `RERAIL_CSAT_DUMP` (a directory): each solve of over 100 ms, its formula as `Gates::dump` writes
        // it, to solve or profile it alone (decision 0670).
        if let Some(dir) = std::env::var_os("RERAIL_CSAT_DUMP") && t_dump.elapsed().as_secs_f64() > 0.1 {
            let text = u.gates.dump(u.vars as usize, base_gates, &u.clauses);
            let name = format!("{:016x}.csat", { use std::hash::{BuildHasher as _}; rustc_hash::FxBuildHasher.hash_one(&text) });
            let _ = std::fs::write(std::path::Path::new(&dir).join(name), text);
        }
        self.work.spend(s.conflicts - before);
        let answer = match answer {
            // With a hypothesis, its values the contradiction read at the position: the rest may go (`prove`).
            Sat::Unsat => Outcome::Proved(hypothesis.and_then(|h| {
                let used = s.used(&u.gates)?;
                let at_position = |&(at, l, _): &(u16, u32, bool)| {
                    at == STREAM || u.memo.get(&(at, u32::from(d) + l)).is_none_or(|&x| x.unsigned_abs() <= 1 || used[x.unsigned_abs() as usize])
                };
                Some(h.iter().map(at_position).collect())
            })),
            Sat::Unknown => Outcome::Unknown,
            Sat::Sat => {
                let holds = |l: Lit| if l == 1 { true } else if l == -1 { false } else { s.holds(l.unsigned_abs() as usize) == (l > 0) };
                let inside = |lag: u32| holds(u.inside(lag));
                // A stream that begins within the window is a real one: kept (`Witnesses`).
                if bounded && let Some(file) = u.stream(&holds) {
                    self.witnesses.add(file);
                }
                let edge: Vec<(u16, u32)> = held.iter().copied().filter(|&(at, lag)| lag > reach && inside(lag) && (carried(ops, nexts, at) || matches!(ops[usize::from(at)], BlockOp::Eq(_) | BlockOp::Class { .. }))).collect();
                // The position's byte's group: the bytes its readers can not tell from it.
                let group = match position.iter().find(|&&(_, l)| holds(l)) {
                    None => [!0; 4],
                    Some(&(b, _)) => {
                        let readers = at_position;
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
                    break 'solve Outcome::Way(Vec::new(), group);
                }
                // And where the stream begins, past the horizon.
                let m = reach + 1;
                let mut way: Lemma = edge.into_iter().map(|(at, lag)| (at, lag - m, holds(u.memo[&(at, lag)]))).collect();
                way.extend((m..=far).map(|lag| (STREAM, lag - m, inside(lag))));
                Outcome::Way(way, group)
            }
        };
        answer
        };
        window.give_back(u, keep);
        out
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

use crate::bitlex_csat::{Fanout, Gates, Solver};
use crate::bitlex_sat::{Answer as Sat, Clauses, Lit};

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
fn unrolled_readers(ops: &[BlockOp], nexts: &HashMap<u16, u16>, roots: &[(u16, u32)], reach: u32) -> (HashMap<u32, HashSet<Reader>>, u32) {
    let mut far = 0;
    let mut readers: HashMap<u32, HashSet<Reader>> = HashMap::default();
    let mut seen = HashSet::default();
    let mut work = roots.to_vec();
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
    /// The window's gates (decision 0670) and its plain clauses: the stream's chains, the units, a solve's own.
    gates: Gates,
    clauses: Clauses,
    vars: i32,
    memo: HashMap<(u16, u32), Lit>,
    /// Each lag past the position's: "the byte is before the stream".
    before: HashMap<u32, Lit>,
    /// Each lag short of the position's: "the byte is past the stream's end".
    past: HashMap<u32, Lit>,
    /// Each read lag's groups: a byte of each, and its literal.
    groups: HashMap<u32, Vec<(u8, Lit)>>,
    readers: HashMap<(Reader, u32, bool), Lit>,
    /// The `memo` and `readers` keys made since the window was lent (`Window::give_back`).
    memo_added: Vec<(u16, u32)>,
    readers_added: Vec<(Reader, u32, bool)>,
}

/// A window's gates, clauses and literals, apart from the operations they were made of (`Unrolled`
/// again, by `attach`): a question's solves differ only in the gates and clauses they add to it.
/// A solve borrows its parts (`Window::attach`) and gives them back with what it added undone
/// (`Window::give_back`): no copy of them a solve.
struct Window {
    reach: u32,
    far: u32,
    /// The readers of the position's byte, in the order its group is told by.
    at_position: Vec<Reader>,
    /// The values the window's operations make, at their lags, sorted.
    held: Vec<(u16, u32)>,
    /// Each variable's gates among the window's own (`Fanout`): made once, read by every solve.
    fan: Fanout,
    /// Its own gates, clauses and variables: past them, a solve's.
    base_gates: usize,
    base: usize,
    base_vars: i32,
    parts: Parts,
    /// Its gates and clauses hold a captured solve's own past its own (`give_back`).
    kept: bool,
}

/// A window's gates, clauses and literals.
struct Parts {
    d: u32,
    gates: Gates,
    clauses: Clauses,
    vars: i32,
    memo: HashMap<(u16, u32), Lit>,
    before: HashMap<u32, Lit>,
    past: HashMap<u32, Lit>,
    groups: HashMap<u32, Vec<(u8, Lit)>>,
    readers: HashMap<(Reader, u32, bool), Lit>,
}

impl Window {
    /// The window as an `Unrolled`, for one solve: its parts lent, given back by `give_back`.
    fn attach<'a>(&mut self, ops: &'a [BlockOp], tables: &'a [ClassTable], nexts: &'a HashMap<u16, u16>) -> Unrolled<'a> {
        let p = &mut self.parts;
        if std::mem::take(&mut self.kept) {
            p.gates.truncate(self.base_gates);
            p.clauses.truncate(self.base);
        }
        Unrolled {
            ops,
            tables,
            nexts,
            d: p.d,
            reach: self.reach,
            gates: std::mem::take(&mut p.gates),
            clauses: std::mem::take(&mut p.clauses),
            vars: p.vars,
            memo: std::mem::take(&mut p.memo),
            before: std::mem::take(&mut p.before),
            past: std::mem::take(&mut p.past),
            groups: std::mem::take(&mut p.groups),
            readers: std::mem::take(&mut p.readers),
            memo_added: Vec::new(),
            readers_added: Vec::new(),
        }
    }

    /// A solve's `Unrolled` back: its gates, clauses, variables and literals undone, the window's own. `keep`: its
    /// gates and clauses kept until the window is attached again, for the device to read (`Captured`).
    fn give_back(&mut self, u: Unrolled<'_>, keep: bool) {
        let Unrolled { mut gates, mut clauses, mut memo, before, past, groups, mut readers, memo_added, readers_added, .. } = u;
        if keep {
            self.kept = true;
        } else {
            gates.truncate(self.base_gates);
            clauses.truncate(self.base);
        }
        for k in memo_added {
            memo.remove(&k);
        }
        for k in readers_added {
            readers.remove(&k);
        }
        let p = &mut self.parts;
        (p.gates, p.clauses, p.vars, p.memo, p.before, p.past, p.groups, p.readers) = (gates, clauses, self.base_vars, memo, before, past, groups, readers);
    }
}

impl<'a> Unrolled<'a> {
    fn detach(self, far: u32, at_position: Vec<Reader>) -> Window {
        let Unrolled { d, reach, gates, clauses, vars, memo, before, past, groups, readers, .. } = self;
        let mut held: Vec<(u16, u32)> = memo.keys().copied().collect();
        held.sort_unstable();
        let fan = Fanout::of(&gates, gates.len(), vars.unsigned_abs() as usize);
        Window { reach, far, at_position, held, fan, base_gates: gates.len(), base: clauses.len(), base_vars: vars, parts: Parts { d, gates, clauses, vars, memo, before, past, groups, readers }, kept: false }
    }
}

impl Unrolled<'_> {
    fn fresh(&mut self) -> Lit {
        self.vars += 1;
        self.vars
    }

    fn and(&mut self, a: Lit, b: Lit) -> Lit {
        let x = self.fresh();
        self.gates.and(x, &[a, b]);
        x
    }

    fn or(&mut self, a: Lit, b: Lit) -> Lit {
        -self.and(-a, -b)
    }

    fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        let x = self.fresh();
        self.gates.xor(x, a, b);
        x
    }

    /// `x` is `a` exactly: `x`'s equation.
    fn equal(&mut self, x: Lit, a: Lit) {
        self.gates.and(x, &[a]);
    }

    /// Whether the byte at `lag` is in the stream: neither before it nor past its end.
    /// The stream of a solved window, as bytes, where it begins within the window: from the
    /// first lag in it (`before` not holding) to its last (`past` not holding), each byte its
    /// group's (a byte the readers can not tell from the solver's), a space where nothing reads it.
    fn stream(&self, holds: &dyn Fn(Lit) -> bool) -> Option<Vec<u8>> {
        let far = self.before.keys().copied().max()?;
        // The first lag before the stream, from the position's on: the stream's first byte is the
        // lag after it.
        let begin = (self.d + 1..=far).find(|lag| holds(self.before[lag]))? - 1;
        let end = (0..self.d).rev().find(|lag| holds(self.past[lag])).map_or(0, |lag| lag + 1);
        Some(
            (end..=begin)
                .rev()
                .map(|lag| self.groups.get(&lag).and_then(|g| g.iter().find(|&&(_, l)| holds(l)).map(|&(b, _)| b)).unwrap_or(b' '))
                .collect(),
        )
    }

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
        if zero_before
            && member(self.tables, 0)
            && let Some(&b) = self.before.get(&lag)
        {
            yes.push(b);
        }
        // `x` is the `or` of `yes`: not `x` is none of them.
        let x = self.fresh();
        let none: Vec<Lit> = yes.iter().map(|&y| -y).collect();
        self.gates.and(-x, &none);
        self.readers.insert((r, lag, zero_before), x);
        self.readers_added.push((r, lag, zero_before));
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
            self.memo_added.push((at, lag));
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
        self.memo_added.push((at, lag));
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A question's key is its content: the same cone at other numbers is the same key, and any
    /// change to it another (decision 0414). And answers keep through `encode`.
    #[test]
    fn questions_are_keyed_by_content() {
        let tables = vec![ClassTable { lo: [1; 16], hi: [1; 16] }];
        let a = [BlockOp::Eq(b'x'), BlockOp::Class { table: 0, bits: 1 }, BlockOp::And(0, 1)];
        // The same, after an operation the value does not read.
        let b = [BlockOp::Const(7), BlockOp::Class { table: 0, bits: 1 }, BlockOp::Eq(b'x'), BlockOp::And(2, 1)];
        let c = [BlockOp::Eq(b'y'), BlockOp::Class { table: 0, bits: 1 }, BlockOp::And(0, 1)];
        let key = |ops: &[BlockOp], v: u16, d: u8| question(ops, &tables, &[], v, d, "test");
        assert_eq!(key(&a, 2, 0), key(&b, 3, 0));
        assert_ne!(key(&a, 2, 0), key(&c, 2, 0));
        assert_ne!(key(&a, 2, 0), key(&a, 2, 1));
        let other = vec![ClassTable { lo: [2; 16], hi: [1; 16] }];
        assert_ne!(key(&a, 2, 0), question(&a, &other, &[], 2, 0, "test"));
        // Kept by digest: 17 bytes a yes or no, 49 a byte set, after the header.
        let made = vec![(key_of(&key(&a, 2, 0)), Answer::Never(true)), (key_of(&key(&c, 2, 0)), Answer::Bytes([1, 2, 3, u64::MAX]))];
        let bytes = encode(&made);
        assert_eq!(bytes.len(), 12 + 17 + 49);
        let known = decode(&bytes);
        assert_eq!(known.len(), 2);
        assert!(made.iter().all(|(k, a)| known.get(k) == Some(a)));
        // Another format or version: no answers.
        assert!(decode(b"RRPROOFS\0\0\0\0").is_empty());
        let mut other_version = bytes.clone();
        other_version[8] ^= 1;
        assert!(decode(&other_version).is_empty());
        // `ask` keys by the digest of the question it is given.
        let mut p = Proofs { known, made: Vec::new(), used: Vec::new(), work: Budget::unlimited() };
        assert_eq!(p.ask(key(&a, 2, 0), || Answer::Never(false)), Answer::Never(true));
        assert!(p.made.is_empty(), "a known answer proved again");
    }

    /// A value never set by its shape: a part's start under a body less the parts (through
    /// `prev`, at its lag), and two fills proved apart; not where the lags differ (decision 0650).
    #[test]
    fn values_never_by_their_shape() {
        // 0 `x`, 1 `y`, 2 `x | y` (the parts), 3 `prev(x)`, 4 a body, 5 the body less the parts
        // (a content row), 6 `prev` of it.
        let mut ops = vec![BlockOp::Eq(b'x'), BlockOp::Eq(b'y'), BlockOp::Or(0, 1), BlockOp::Prev(0, 1), BlockOp::Eq(b'b'), BlockOp::AndNot(4, 2), BlockOp::Prev(5, 1)];
        let at = |ops: &mut Vec<BlockOp>, op: BlockOp| {
            ops.push(op);
            (ops.len() - 1) as u16
        };
        // The content row at a part's byte: never.
        let v = at(&mut ops, BlockOp::And(5, 1));
        assert!(never_by_shape(&ops, &[], v));
        // A byte later, through `prev` on both sides: never.
        let v = at(&mut ops, BlockOp::And(6, 3));
        assert!(never_by_shape(&ops, &[], v));
        // At a byte the parts are not read at: not by shape.
        let v = at(&mut ops, BlockOp::And(5, 3));
        assert!(!never_by_shape(&ops, &[], v));
        // Two values proved apart: `x` a byte back at delay 0 and `y` here at delay 1 are one
        // byte; at delay 0 both, two.
        let v = at(&mut ops, BlockOp::And(3, 1));
        assert!(never_by_shape(&ops, &[((0, 0), (1, 1))], v));
        assert!(!never_by_shape(&ops, &[((0, 0), (1, 0))], v));
    }

    /// A question takes a 16th of the budget at most: past it, it is not proved, and the next
    /// question has the rest (decision 0650).
    #[test]
    fn a_question_takes_its_share() {
        let work = Budget::new(1600);
        let mut p = Proofs { known: HashMap::default(), made: Vec::new(), used: Vec::new(), work: work.clone() };
        let spent = |w: &Budget| {
            while w.spend(1) {}
            Answer::Never(false)
        };
        p.ask(b"one".to_vec(), || spent(&work));
        assert_eq!(work.left(), 1500, "a question spent past its share");
        p.ask(b"two".to_vec(), || spent(&work));
        assert_eq!(work.left(), 1400);
        assert_eq!(Budget::unlimited().left(), u64::MAX);
    }

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
