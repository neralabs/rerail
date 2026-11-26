//! A chunk solved from an unknown start: what its own bytes allow its start to be.
//!
//! Nothing is known at a chunk's first block but its bytes, and the bytes before it (the
//! components read in place, `starts::in_place`). Every other component of its start is
//! unknown, the back fills' carries into its last block too. The chunk is run in three values a
//! bit, known 0, known 1 or unknown (`T`):
//!
//! - an unknown carry makes what it reaches unknown, until the bytes make it known again: a fill
//!   at its first end bit, a `prev` after its `k` bits, an escape after its backslash run. That
//!   forgetting is what the bytes say about the start;
//! - a refusal (a total) with a known bit refutes the start the run assumed.
//!
//! A start component is fixed only where it matters: while it is unknown at the chunk's first
//! block and the run leaves a refusal or the chunk's end unknown. Each of its values is run
//! again, three-valued, from the start with it fixed (`solve`'s split). The result is the starts
//! left (`Leaf`): each a partial start, the components it never fixed free, with the end it
//! gives and the outputs it makes. A start every leaf leaves out is one the bytes rule out.

use crate::chunk::{File, Prepared};
use crate::starts::{Component, Place};
use rerail_compiler::block::{self, BlockOp, BlockProgram, MAX_ROUNDS};
use std::ops::Range;

/// A word of three-valued bits: `k` the known bits, `v` their values (zero where unknown).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct T {
    pub k: u64,
    pub v: u64,
}

impl T {
    pub const X: T = T { k: 0, v: 0 };

    pub fn known(v: u64) -> T {
        T { k: u64::MAX, v }
    }

    fn norm(k: u64, v: u64) -> T {
        T { k, v: v & k }
    }

    pub fn full(self) -> bool {
        self.k == u64::MAX
    }

    fn not(self) -> T {
        T::norm(self.k, !self.v)
    }

    fn and(self, o: T) -> T {
        let zero = (self.k & !self.v) | (o.k & !o.v);
        T::norm((self.k & o.k) | zero, self.v & o.v)
    }

    fn or(self, o: T) -> T {
        let one = self.v | o.v;
        T::norm((self.k & o.k) | one, one)
    }

    fn xor(self, o: T) -> T {
        T::norm(self.k & o.k, self.v ^ o.v)
    }

    fn bit(self, i: u32) -> Option<bool> {
        (self.k >> i & 1 == 1).then_some(self.v >> i & 1 == 1)
    }

    /// Where two words agree, known; else unknown.
    pub fn merge(self, o: T) -> T {
        let k = self.k & o.k & !(self.v ^ o.v);
        T::norm(k, self.v)
    }
}

/// Three-valued logic of single bits.
fn t_or(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

fn t_and(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn t_not(a: Option<bool>) -> Option<bool> {
    a.map(|x| !x)
}

fn set(t: &mut T, i: u32, b: Option<bool>) {
    if let Some(b) = b {
        t.k |= 1 << i;
        if b {
            t.v |= 1 << i;
        }
    }
}

/// `fill(s, e)` from carry `c`, bit by bit: `f[i] = s[i] | (f[i - 1] & !e[i])`.
fn fill_t(s: T, e: T, c: Option<bool>) -> (T, Option<bool>) {
    let mut out = T::X;
    let mut f = c;
    for i in 0..64 {
        f = t_or(s.bit(i), t_and(f, t_not(e.bit(i))));
        set(&mut out, i, f);
    }
    (out, f)
}

/// `back_fill(s, e)` from carry `c` into bit 63: `b[i] = s[i] | (b[i + 1] & !e[i])`.
fn back_fill_t(s: T, e: T, c: Option<bool>) -> (T, Option<bool>) {
    let mut out = T::X;
    let mut b = c;
    for i in (0..64).rev() {
        b = t_or(s.bit(i), t_and(b, t_not(e.bit(i))));
        set(&mut out, i, b);
    }
    (out, b)
}

/// A 1-bit carried word function over a known input, from an unknown carry: both carries run,
/// the results merged.
fn both(c: Option<bool>, f: impl Fn(u64) -> (u64, u64), one: u64) -> (T, Option<bool>) {
    let run = |cin: u64| {
        let (w, o) = f(cin);
        (T::known(w), Some(o != 0))
    };
    match c {
        Some(b) => run(if b { one } else { 0 }),
        None => {
            let ((w0, c0), (w1, c1)) = (run(0), run(one));
            (w0.merge(w1), if c0 == c1 { c0 } else { None })
        }
    }
}

/// `toggle`: the prefix XOR of a known word from a known carry; an unknown input bit or carry
/// leaves everything after it unknown.
fn toggle_t(x: T, c: Option<bool>) -> (T, Option<bool>) {
    if x.full() {
        return both(c, |cin| block::carry::toggle(x.v, cin), u64::MAX);
    }
    let mut out = T::X;
    let mut p = c;
    for i in 0..64 {
        p = match (p, x.bit(i)) {
            (Some(a), Some(b)) => Some(a ^ b),
            _ => None,
        };
        set(&mut out, i, p);
    }
    (out, p)
}

/// `escaped` over a known run word, from a carry; an unknown run leaves it unknown.
fn escaped_t(x: T, c: Option<bool>) -> (T, Option<bool>) {
    if x.full() { both(c, |cin| block::carry::escaped(x.v, cin), 1) } else { (T::X, None) }
}

/// `prev`: the word up `k` bits, the `k` bits held before below.
fn prev_t(x: T, held: T, k: u8) -> (T, T) {
    if k == 0 {
        return (x, T::X);
    }
    let lo = (1u64 << k) - 1;
    let out = T::norm((x.k << k) | (held.k & lo), (x.v << k) | (held.v & lo));
    let next = T::norm(x.k >> (64 - u32::from(k)), x.v >> (64 - u32::from(k)));
    (out, next)
}

/// A carry held as a 1-bit value (a toggle's spread word, a fill's or escape's bit).
fn held_bit(t: T) -> Option<bool> {
    t.bit(0)
}

fn bit_held(b: Option<bool>, toggle: bool) -> T {
    match b {
        None => T::X,
        Some(false) => T::known(0),
        Some(true) => T::known(if toggle { u64::MAX } else { 1 }),
    }
}

/// A start in three values, as `chunk::Start` holds one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TStart {
    pub ops: Vec<T>,
    pub groups: Vec<Vec<T>>,
    pub backs: Vec<T>,
}

impl TStart {
    pub fn get(&self, at: Place) -> T {
        match at {
            Place::Op(i) => self.ops[i],
            Place::Group(g, k) => self.groups[g][k],
            Place::Back(i) => self.backs[i],
        }
    }

    pub fn set(&mut self, at: Place, t: T) {
        match at {
            Place::Op(i) => self.ops[i] = t,
            Place::Group(g, k) => self.groups[g][k] = t,
            Place::Back(i) => self.backs[i] = t,
        }
    }
}

impl Prepared<'_> {
    /// Every carry unknown.
    pub fn unknown(&self) -> TStart {
        TStart { ops: vec![T::X; self.carried.len()], groups: self.groups.iter().map(|g| vec![T::X; g.members.len()]).collect(), backs: vec![T::X; self.backs.len()] }
    }

    /// `run` in three values: the values over `range` from `start`, and the end.
    pub fn run_t(&self, file: &File, range: Range<usize>, start: &TStart) -> (Vec<Vec<T>>, TStart) {
        let m = range.len();
        let (values, end, _) = self.run_t_at(file, range, start, m);
        (values, end)
    }

    /// `run_t`, and the forward state on the way into the range's block `mark` (its back fills'
    /// carries left unknown).
    pub fn run_t_at(&self, file: &File, range: Range<usize>, start: &TStart, mark: usize) -> (Vec<Vec<T>>, TStart, TStart) {
        let program = self.program;
        let m = range.len();
        let mut values: Vec<Vec<T>> = vec![Vec::new(); program.ops.len()];
        let mut end = start.clone();
        let mut entry = start.clone();
        entry.backs.iter_mut().for_each(|b| *b = T::X);
        let at = |i: usize| range.start + i;
        for x in self.order_ops() {
            let x = match x {
                Err(g) => {
                    self.rounds_t(g, m, &start.groups[g], &mut values, &mut end.groups[g], mark, &mut entry.groups[g]);
                    continue;
                }
                Ok(x) => x,
            };
            let v = |a: u16| &values[usize::from(a)];
            let two = |a: u16, b: u16, f: fn(T, T) -> T| -> Vec<T> { v(a).iter().zip(v(b)).map(|(&p, &q)| f(p, q)).collect() };
            let s = self.slot_of(x);
            let known = |f: &dyn Fn(usize) -> u64| -> Vec<T> { (0..m).map(|i| T::known(f(i))).collect() };
            let words: Vec<T> = match program.ops[x] {
                BlockOp::Within(d) => known(&|i| block::within_at(at(i), file.len, d)),
                BlockOp::Prefix(n, w) => known(&|i| block::prefix_at(&file.blocks[at(i)], at(i), file.len, n, w)),
                BlockOp::Phase(k, r) => known(&|i| block::phase(64 * at(i), k, r)),
                BlockOp::Position(j) => known(&|i| block::position(at(i), j)),
                BlockOp::Eq(byte) => known(&|i| block::eq(&file.blocks[at(i)], byte)),
                BlockOp::Class { table, bits } => {
                    let table = &program.tables[usize::from(table)];
                    known(&|i| block::class(&file.blocks[at(i)], table, bits))
                }
                BlockOp::Pair { before, table, bits } => {
                    let (a, b) = (&program.tables[usize::from(before)], &program.tables[usize::from(table)]);
                    known(&|i| block::pair(file.before_of(at(i)), &file.blocks[at(i)], a, b, bits))
                }
                BlockOp::Const(c) => vec![T::known(c); m],
                BlockOp::Not(a) => v(a).iter().map(|p| p.not()).collect(),
                BlockOp::And(a, b) => two(a, b, T::and),
                BlockOp::Or(a, b) => two(a, b, T::or),
                BlockOp::Xor(a, b) => two(a, b, T::xor),
                BlockOp::AndNot(a, b) => two(a, b, |p, q| p.and(q.not())),
                BlockOp::Early(a, k) => v(a).iter().map(|p| T::norm((p.k >> k) | !(u64::MAX >> k), p.v >> k)).collect(),
                BlockOp::Prev(a, k) => {
                    let mut held = start.ops[s];
                    let out = v(a).iter().enumerate().map(|(i, &p)| {
                        if i == mark {
                            entry.ops[s] = held;
                        }
                        let (o, h) = prev_t(p, held, k);
                        held = h;
                        o
                    }).collect();
                    end.ops[s] = held;
                    out
                }
                BlockOp::Toggle(_) | BlockOp::Escaped(_) | BlockOp::Fill(..) => {
                    let toggle = matches!(program.ops[x], BlockOp::Toggle(_));
                    let (a, b) = match program.ops[x] {
                        BlockOp::Fill(a, b) => (a, Some(b)),
                        BlockOp::Toggle(a) | BlockOp::Escaped(a) => (a, None),
                        _ => unreachable!(),
                    };
                    let mut c = held_bit(start.ops[s]);
                    let out = (0..m).map(|i| {
                        if i == mark {
                            entry.ops[s] = bit_held(c, toggle);
                        }
                        let p = v(a)[i];
                        let (o, n) = match program.ops[x] {
                            BlockOp::Toggle(_) => toggle_t(p, c),
                            BlockOp::Escaped(_) => escaped_t(p, c),
                            _ => fill_t(p, v(b.expect("a fill's end"))[i], c),
                        };
                        c = n;
                        o
                    }).collect();
                    end.ops[s] = bit_held(c, toggle);
                    out
                }
                BlockOp::BackFill(a, b) => {
                    let (p, q) = (v(a), v(b));
                    let mut out = vec![T::X; m];
                    let mut c = held_bit(start.backs[s]);
                    for i in (0..m).rev() {
                        (out[i], c) = back_fill_t(p[i], q[i], c);
                    }
                    end.backs[s] = bit_held(c, false);
                    out
                }
                BlockOp::Carry(_) => unreachable!("a carry is its group's"),
                BlockOp::Cmp { .. } | BlockOp::Bit { .. } => unreachable!("refused by `new`"),
            };
            values[x] = words;
        }
        (values, end, entry)
    }

    /// Group `g`'s rounds in three values: from each block's carries' initial words, the members
    /// again until nothing changes, or `MAX_ROUNDS` rounds.
    #[allow(clippy::too_many_arguments)]
    fn rounds_t(&self, g: usize, m: usize, start: &[T], values: &mut [Vec<T>], end: &mut [T], mark: usize, entry: &mut [T]) {
        let program = self.program;
        let group = &self.groups[g];
        let mut local = vec![T::X; program.ops.len()];
        // Each member's carry in, as the word its operation takes (a `prev`'s previous word).
        let mut state: Vec<T> = group.members.iter().zip(start).map(|(&x, &h)| match program.ops[x] {
            BlockOp::Prev(_, k) if k > 0 => T::norm(h.k << (64 - u32::from(k)), h.v << (64 - u32::from(k))),
            _ => h,
        }).collect();
        let mut outs = vec![T::X; group.members.len()];
        for &x in group.members.iter() {
            values[x] = vec![T::X; m];
        }
        let carry_of = |c: usize| group.carries.iter().position(|p| p.0 == c).expect("a carry of the group");
        let held_of = |state: &[T], out: &mut [T]| {
            for ((e, &x), &w) in out.iter_mut().zip(group.members.iter()).zip(state) {
                *e = match program.ops[x] {
                    BlockOp::Prev(_, k) if k > 0 => T::norm(w.k >> (64 - u32::from(k)), w.v >> (64 - u32::from(k))),
                    _ => w,
                };
            }
        };
        for i in 0..m {
            if i == mark {
                held_of(&state, entry);
            }
            let mut cur: Vec<T> = group.carries.iter().map(|&(c, _)| match program.ops[c] {
                BlockOp::Carry(init) => values[usize::from(init)][i],
                _ => unreachable!("verified: a carry"),
            }).collect();
            for _ in 0..MAX_ROUNDS {
                for (k, &x) in group.members.iter().enumerate() {
                    let get = |o: u16| if group.members.binary_search(&usize::from(o)).is_ok() { local[usize::from(o)] } else { values[usize::from(o)][i] };
                    let (w, c) = match program.ops[x] {
                        BlockOp::Carry(_) => (cur[carry_of(x)], T::X),
                        BlockOp::Not(a) => (get(a).not(), T::X),
                        BlockOp::And(a, b) => (get(a).and(get(b)), T::X),
                        BlockOp::Or(a, b) => (get(a).or(get(b)), T::X),
                        BlockOp::Xor(a, b) => (get(a).xor(get(b)), T::X),
                        BlockOp::AndNot(a, b) => (get(a).and(get(b).not()), T::X),
                        BlockOp::Early(a, k) => {
                            let p = get(a);
                            (T::norm((p.k >> k) | !(u64::MAX >> k), p.v >> k), T::X)
                        }
                        // A member `prev`'s carry is its operand's whole word.
                        BlockOp::Prev(a, d) => {
                            let p = get(a);
                            let (o, _) = prev_t(p, T::norm(state[k].k >> (64 - u32::from(d.max(1))), state[k].v >> (64 - u32::from(d.max(1)))), d);
                            (o, p)
                        }
                        BlockOp::Toggle(a) => { let (o, n) = toggle_t(get(a), held_bit(state[k])); (o, bit_held(n, true)) }
                        BlockOp::Escaped(a) => { let (o, n) = escaped_t(get(a), held_bit(state[k])); (o, bit_held(n, false)) }
                        BlockOp::Fill(a, b) => { let (o, n) = fill_t(get(a), get(b), held_bit(state[k])); (o, bit_held(n, false)) }
                        op => unreachable!("a value that reads nothing is no member: {op:?}"),
                    };
                    local[x] = w;
                    outs[k] = c;
                }
                let next: Vec<T> = group.carries.iter().map(|&(_, x)| local[x]).collect();
                if next == cur {
                    break;
                }
                cur = next;
            }
            // Settled or not, the last round stands: the concrete rounds settle by round
            // `MAX_ROUNDS - 1` (`rounds_group` asserts it), and round i in three values holds
            // concrete round i, so the last round's members and carries out hold the fixed point.
            for &x in group.members.iter() {
                values[x][i] = local[x];
            }
            state.copy_from_slice(&outs);
        }
        held_of(&state, end);
        if mark >= m {
            held_of(&state, entry);
        }
    }
}

/// A start left: the partial starts (each component fixed, or free) that give one end, that
/// end, and their outputs over the chunk (known where every one of them agrees).
#[derive(Clone, Debug)]
pub struct Leaf {
    /// Partial starts at the lookback's first block (`Solved::from`).
    pub starts: Vec<Vec<Option<u64>>>,
    /// The state at the chunk's first block they give (known where every one agrees).
    pub entry: TStart,
    pub end: TStart,
    pub outputs: Vec<Vec<T>>,
    /// No refusal is unknown and every state component of the end is known.
    pub clean: bool,
}

/// What a chunk's bytes leave of its start.
#[derive(Clone, Debug, Default)]
pub struct Solved {
    /// The block the run started at: `look` blocks before the chunk (fewer at a file's start).
    pub from: usize,
    pub leaves: Vec<Leaf>,
    /// Partial starts a refusal refuted.
    pub refuted: usize,
    /// Three-valued runs made.
    pub runs: usize,
    /// More partial starts were wanted than `cap`: the solve stopped splitting.
    pub over: bool,
}

/// A component's values to split on: a toggle's two words, a 1-bit carry's two bits, a `prev`'s
/// `2^k` held values.
fn values_of(program: &BlockProgram, c: &Component) -> Vec<u64> {
    match program.ops[c.op] {
        BlockOp::Toggle(_) => vec![0, u64::MAX],
        BlockOp::Prev(_, k) => (0..1u64 << k).collect(),
        _ => vec![0, 1],
    }
}

/// The order components are split in: the long-lived carries first (a region's fill, a scan's
/// toggle), then the back fills (the next chunk's to answer), then the short-lived ones the
/// lookback should have made known (an escape, a `prev` of at most 4 bits). Wider `prev`s are
/// never split.
fn split_rank(program: &BlockProgram, c: &Component) -> Option<u8> {
    match (c.at, &program.ops[c.op]) {
        (Place::Back(_), _) => Some(1),
        (_, BlockOp::Fill(..) | BlockOp::Toggle(_)) => Some(0),
        (_, BlockOp::Escaped(_)) => Some(2),
        (_, BlockOp::Prev(_, k)) if *k <= 4 => Some(3),
        _ => None,
    }
}

/// The ties between components: each `prev(x, k)` whose `x` is a fill or a toggle holds, as its
/// top bit, `x`'s last bit at the block before, which is `x`'s carry. `(prev, x)` component pairs.
pub fn ties(program: &BlockProgram, comps: &[Component]) -> Vec<(usize, usize)> {
    let by_op: std::collections::HashMap<usize, usize> = comps.iter().enumerate().map(|(i, c)| (c.op, i)).collect();
    comps.iter().enumerate().filter_map(|(i, c)| match program.ops[c.op] {
        BlockOp::Prev(a, k) if k > 0 && !c.in_place => {
            let j = *by_op.get(&usize::from(a))?;
            matches!(program.ops[comps[j].op], BlockOp::Fill(..) | BlockOp::Toggle(_)).then_some((i, j))
        }
        _ => None,
    }).collect()
}

/// A partial start in three values, the ties applied. `None` when a fixed `prev` disagrees with
/// the carry it is tied to.
fn start_of(p: &Prepared<'_>, comps: &[Component], ties: &[(usize, usize)], base: &TStart, fixed: &[Option<u64>]) -> Option<TStart> {
    let program = p.program;
    let mut start = base.clone();
    for (c, f) in comps.iter().zip(fixed) {
        if let Some(v) = *f {
            start.set(c.at, match program.ops[c.op] {
                BlockOp::Prev(_, k) => T::norm((1u64 << k) - 1, v),
                BlockOp::Toggle(_) => T::known(v),
                _ => T::norm(1, v),
            });
        }
    }
    for &(i, j) in ties {
        let BlockOp::Prev(_, k) = program.ops[comps[i].op] else { unreachable!() };
        let top = 1u64 << (k - 1);
        let mut held = start.get(comps[i].at);
        let carry = start.get(comps[j].at);
        // The carry's bit 0 (a toggle's spread word has it too).
        match (carry.k & 1 == 1, held.k & top != 0) {
            (true, true) if (carry.v & 1 == 1) != (held.v & top != 0) => return None,
            (true, false) => {
                held = T::norm(held.k | top, held.v | if carry.v & 1 == 1 { top } else { 0 });
                start.set(comps[i].at, held);
            }
            (false, true) => {
                let b = held.v & top != 0;
                start.set(comps[j].at, match program.ops[comps[j].op] {
                    BlockOp::Toggle(_) => T::known(if b { u64::MAX } else { 0 }),
                    _ => T::norm(1, u64::from(b)),
                });
            }
            _ => {}
        }
    }
    Some(start)
}

/// Chunk `range` of `file` solved from an unknown start: the run starts `look` blocks before the
/// chunk (the bytes before it, read in place), every carry unknown there but the components read
/// in place, and splits at most `cap` partial starts. Partial starts that give one end are one
/// leaf.
pub fn solve(p: &Prepared<'_>, comps: &[Component], file: &File, range: Range<usize>, look: usize, cap: usize) -> Solved {
    let from = range.start.saturating_sub(look);
    let program = p.program;
    let totals: Vec<usize> = program.totals.iter().map(|&t| usize::from(t)).collect();
    let outputs: Vec<usize> = program.outputs.iter().map(|&o| usize::from(o)).collect();
    let ties = ties(program, comps);
    let tied: Vec<bool> = (0..comps.len()).map(|i| ties.iter().any(|t| t.0 == i)).collect();
    // The components read in place, from the bytes before the chunk.
    let mut base = p.unknown();
    {
        let mut s = p.zero();
        crate::starts::in_place(p, comps, file, from, &mut s);
        for c in comps.iter().filter(|c| c.in_place) {
            base.set(c.at, T::known(c.at.get(&s)));
        }
    }
    let mut out = Solved { from, ..Solved::default() };
    let mut made = 1usize;
    let mut work: Vec<Vec<Option<u64>>> = vec![vec![None; comps.len()]];
    while let Some(fixed) = work.pop() {
        let Some(start) = start_of(p, comps, &ties, &base, &fixed) else {
            out.refuted += 1;
            continue;
        };
        // One run over the lookback and the chunk (a back fill's carry comes in at the chunk's
        // last block), its forward state recorded on the way into the chunk.
        let look = range.start - from;
        let (mut values, end, entry) = p.run_t_at(file, from..range.end, &start, look);
        let before: Vec<Vec<T>> = values.iter_mut().map(|v| v.drain(..look.min(v.len())).collect()).collect();
        out.runs += 1;
        let refused = |vs: &Vec<Vec<T>>| totals.iter().any(|&t| vs[t].iter().any(|w| w.v != 0));
        if refused(&before) || refused(&values) {
            out.refuted += 1;
            continue;
        }
        let open_refusal = totals.iter().any(|&t| values[t].iter().chain(&before[t]).any(|w| !w.full()));
        let end_open = comps.iter().any(|c| !c.in_place && {
            let t = end.get(c.at);
            match program.ops[c.op] {
                BlockOp::Prev(_, k) => t.k & ((1u64 << k) - 1) != (1u64 << k) - 1,
                BlockOp::Toggle(_) => !t.full(),
                _ => t.k & 1 == 0,
            }
        });
        // The component to fix: still free and untied, unknown somewhere in the chunk, first by
        // rank, then in the program's order.
        let split = if open_refusal || end_open {
            comps.iter().enumerate()
                .filter(|(i, c)| !c.in_place && !tied[*i] && fixed[*i].is_none() && start.get(c.at) == T::X && values[c.op].iter().chain(&before[c.op]).any(|w| !w.full()))
                .filter_map(|(i, c)| split_rank(program, c).map(|r| (r, c.op, i)))
                .min()
                .map(|(.., i)| i)
        } else {
            None
        };
        match split {
            Some(i) if made + values_of(program, &comps[i]).len() - 1 <= cap => {
                let vs = values_of(program, &comps[i]);
                made += vs.len() - 1;
                for v in vs {
                    let mut f = fixed.clone();
                    f[i] = Some(v);
                    work.push(f);
                }
            }
            s => {
                out.over |= s.is_some();
                let outs: Vec<Vec<T>> = outputs.iter().map(|&o| values[o].clone()).collect();
                let clean = !open_refusal && !end_open;
                match out.leaves.iter_mut().find(|l| l.end == end) {
                    Some(l) => {
                        l.starts.push(fixed);
                        l.clean &= clean;
                        l.entry = merge_start(&l.entry, &entry);
                        for (a, b) in l.outputs.iter_mut().zip(&outs) {
                            for (x, y) in a.iter_mut().zip(b) {
                                *x = x.merge(*y);
                            }
                        }
                    }
                    None => out.leaves.push(Leaf { starts: vec![fixed], entry, end, outputs: outs, clean }),
                }
            }
        }
    }
    out
}

/// Two starts merged: known where both agree.
fn merge_start(a: &TStart, b: &TStart) -> TStart {
    let m = |x: &[T], y: &[T]| -> Vec<T> { x.iter().zip(y).map(|(p, q)| p.merge(*q)).collect() };
    TStart { ops: m(&a.ops, &b.ops), groups: a.groups.iter().zip(&b.groups).map(|(x, y)| m(x, y)).collect(), backs: m(&a.backs, &b.backs) }
}
