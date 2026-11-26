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

/// A way the chunk can be, from one entry: its state at the chunk's first block (`entry`), at its end, the back
/// fills' carries it assumed into its last block (what it needs of the next chunk) and gave out
/// of its first block (what the chunk before must have assumed), and its outputs over the chunk.
/// Known where every world merged into it agrees.
#[derive(Clone, Debug)]
pub struct Leaf {
    pub entry: TStart,
    pub end: TStart,
    pub back_in: Vec<T>,
    pub back_out: Vec<T>,
    pub outputs: Vec<Vec<T>>,
    /// No refusal over the chunk is unknown and every state component of the end is known (a
    /// stack's kinds aside).
    pub clean: bool,
    /// The end's components that are the entry's, passed through: the kinds of the stack's
    /// levels the chunk never went below. The join takes them from the end before.
    pub pass: Vec<Place>,
    /// Its entry's state's place among those the chunk kept (`Entries::Ranked`; 0: the best).
    pub rank: usize,
}

/// What a chunk's bytes leave of its start.
#[derive(Clone, Debug, Default)]
pub struct Solved {
    /// The block the run started at: `look` blocks before the chunk (fewer at a file's start).
    pub from: usize,
    pub leaves: Vec<Leaf>,
    /// Distinct states at the chunk's first block.
    pub entries: usize,
    /// The most worlds alive after any block.
    pub widest: usize,
    /// Worlds a refusal or a back fill's check refuted.
    pub refuted: usize,
    /// One-block three-valued runs made.
    pub runs: usize,
    /// More worlds than `cap` after a block: some were merged (still sound, less known).
    pub over: bool,
    /// Per entry, the splits it made inside the chunk.
    pub splits: Vec<usize>,
    /// The entries the run to the chunk left (before any split inside the chunk).
    pub entry_states: Vec<TStart>,
    /// Entries that reached the split budget (their leaves keep what is left unknown).
    pub spent: usize,
    /// The entries not solved: their states were not among the best `M` (`Entries::Ranked`).
    pub dropped: Vec<TStart>,
    /// What was still open when an entry reached the budget: refusals, end components, outputs
    /// unknown, and whether a back carry was (ops).
    pub open_at_budget: Vec<(Vec<usize>, Vec<usize>, Vec<usize>, bool)>,
}

/// A component's values: a toggle's two words, a 1-bit carry's two bits, a `prev`'s `2^k` held
/// values.
fn values_of(program: &BlockProgram, c: &Component) -> Vec<u64> {
    match program.ops[c.op] {
        BlockOp::Toggle(_) => vec![0, u64::MAX],
        BlockOp::Prev(_, k) => (0..1u64 << k).collect(),
        _ => vec![0, 1],
    }
}

/// A component's known mask, as `TStart` holds it.
pub fn width(program: &BlockProgram, c: &Component) -> u64 {
    match program.ops[c.op] {
        BlockOp::Prev(_, k) => (1u64 << k) - 1,
        BlockOp::Toggle(_) => u64::MAX,
        _ => 1,
    }
}

fn fixed(program: &BlockProgram, c: &Component, v: u64) -> T {
    T::norm(width(program, c), v)
}

fn known_in(program: &BlockProgram, c: &Component, s: &TStart) -> bool {
    let w = width(program, c);
    s.get(c.at).k & w == w
}

/// The state a start keeps across many blocks, by component (`Shape::long`), and what of it is
/// left unknown until the chunk reads it (`Shape::lazy`).
///
/// - **Long-lived**: a walk's or a set apart's fills (the region open at a place), a stack's
///   levels (decision 0621), the toggles, and the `prev`s tied to them.
/// - **Lazy**: a stack level's kind (the string whose template it is). A chunk never enumerates
///   them: where it pops a level below its own pushes and the kind decides an output, the entry
///   is split on that level's kind only, and the leaf records it. The join checks the record
///   against the chunk before's pushes.
#[derive(Clone, Debug, Default)]
pub struct Shape {
    pub long: Vec<bool>,
    pub lazy: Vec<bool>,
}

/// The order a block splits a world's components in: the long-lived state first (the only one
/// split before the chunk), then the other fills, escapes and the lazy kinds, then `prev`s of at
/// most 4 bits. Wider `prev`s and back fills (guessed,
/// then checked) are never split here.
fn split_rank(program: &BlockProgram, shape: &Shape, i: usize, c: &Component) -> Option<u8> {
    match (c.at, &program.ops[c.op]) {
        (Place::Back(_), _) => None,
        _ if shape.lazy[i] => Some(1),
        _ if shape.long[i] => Some(0),
        // A group's short `prev`: where it leaves the block open, a case before the chunk too
        // (a region's opener at the byte before: the cases rejoin after its closer, which three
        // values cannot see).
        (Place::Group(..), BlockOp::Prev(_, k)) if *k <= 4 => Some(0),
        (_, BlockOp::Fill(..) | BlockOp::Toggle(_) | BlockOp::Escaped(_)) => Some(1),
        (_, BlockOp::Prev(_, k)) if *k <= 4 => Some(2),
        _ => None,
    }
}

/// Per value, the fill or toggle component whose carry is its last bit: the fill or toggle
/// itself, or a group's carry whose next is one (its value is the next's).
fn carried_by(program: &BlockProgram, comps: &[Component]) -> std::collections::HashMap<usize, usize> {
    let mut out: std::collections::HashMap<usize, usize> = comps.iter().enumerate().filter(|(_, c)| matches!(program.ops[c.op], BlockOp::Fill(..) | BlockOp::Toggle(_))).map(|(i, c)| (c.op, i)).collect();
    for group in &program.rounds {
        for &(c, x) in group {
            if let Some(&i) = out.get(&usize::from(x)) {
                out.insert(usize::from(c), i);
            }
        }
    }
    out
}

/// The ties between components: each `prev(x, k)` whose `x` is a fill or a toggle (or a group's
/// carry of one) holds, as its top bit, `x`'s last bit at the block before, which is the fill's
/// carry. `(prev, fill)` component pairs.
pub fn ties(program: &BlockProgram, comps: &[Component]) -> Vec<(usize, usize)> {
    let by = carried_by(program, comps);
    comps.iter().enumerate().filter_map(|(i, c)| match program.ops[c.op] {
        BlockOp::Prev(a, k) if k > 0 && !c.in_place => Some((i, *by.get(&usize::from(a))?)),
        _ => None,
    }).collect()
}

/// Sets of fills no two of which are open at one place: each walk's kinds, each set apart, each
/// stack level's kinds. Per set, each fill's component and the `prev`s of it.
#[derive(Clone, Debug)]
pub struct Walks {
    /// Per set, per kind: the fill's component.
    pub fills: Vec<Vec<usize>>,
    /// Per set, per kind: `(component, k)` of each `prev(fill, k)`.
    pub prevs: Vec<Vec<Vec<(usize, u8)>>>,
}

/// A stack's levels by component (`block::Nest`): per level, outermost first, its fill, and its
/// kinds' fills.
#[derive(Clone, Debug)]
pub struct Stack {
    pub levels: Vec<usize>,
    pub kinds: Vec<Vec<usize>>,
    /// The fill a kind's own stack is within.
    pub within: Option<usize>,
}

/// The sets of region fills, by component: each walk's kinds, each set apart.
pub fn regions(program: &BlockProgram, comps: &[Component]) -> Vec<Vec<usize>> {
    let by = carried_by(program, comps);
    let mut sets: Vec<Vec<usize>> = Vec::new();
    for w in &program.walks {
        let fills: Option<Vec<usize>> = w.kinds.iter().map(|k| by.get(&usize::from(k.fill)).copied()).collect();
        sets.extend(fills);
    }
    for set in &program.apart {
        sets.push(set.iter().filter_map(|&v| by.get(&usize::from(v)).copied()).collect());
    }
    sets.retain(|s| s.len() > 1);
    sets
}

pub fn walks(program: &BlockProgram, comps: &[Component]) -> Walks {
    let by = carried_by(program, comps);
    let mut sets = regions(program, comps);
    for st in stacks(program, comps) {
        sets.extend((0..st.levels.len()).map(|j| st.kinds.iter().map(|k| k[j]).collect::<Vec<usize>>()));
    }
    sets.retain(|s| s.len() > 1);
    sets.dedup();
    let prevs = sets.iter().map(|fills| fills.iter().map(|&f| comps.iter().enumerate().filter_map(|(i, c)| match program.ops[c.op] {
        BlockOp::Prev(a, k) if k > 0 && by.get(&usize::from(a)) == Some(&f) => Some((i, k)),
        _ => None,
    }).collect()).collect()).collect();
    Walks { fills: sets, prevs }
}

/// Fills within a run: `fill(s, e)` whose bits are within a value `R`'s at every place of
/// every stream, so its carry is 1 only where `R`'s last byte was: what `prev(R, 1)` holds. `R`
/// is a value whose runs a closer of the fill ends (`e`, or a branch it ORs, is `prev(R, 1)`
/// less a part of `R`): the fill opens in a run and closes at the byte after it. Proved by the
/// compiler's prover (`f & ~R` never, at the program's delay: bytes after the position may be
/// past the stream's end, which is sound; by induction, `Induction::never_inductive`). `(fill,
/// prev)` component pairs; proved once a
/// program (`Prepared::bounds`).
pub fn bounds(p: &Prepared<'_>, comps: &[Component]) -> Vec<(usize, usize)> {
    let program = p.program;
    let ops = &program.ops;
    let by_op: std::collections::HashMap<usize, usize> = comps.iter().enumerate().map(|(i, c)| (c.op, i)).collect();
    let proved = p.bounds.get_or_init(|| {
        let mut out = Vec::new();
        for (f, op) in ops.iter().enumerate() {
            let BlockOp::Fill(_, e) = *op else { continue };
            let mut todo = vec![(e, 3u8)];
            let mut rs = Vec::new();
            while let Some((e, d)) = todo.pop() {
                match ops[usize::from(e)] {
                    BlockOp::Or(a, b) if d > 0 => todo.extend([(a, d - 1), (b, d - 1)]),
                    BlockOp::AndNot(q, _) => {
                        if let BlockOp::Prev(r, 1) = ops[usize::from(q)] {
                            rs.push((r, q));
                        }
                    }
                    _ => {}
                }
            }
            for (r, q) in rs {
                let mut with = ops.clone();
                let (Ok(fv), Ok(at)) = (u16::try_from(f), u16::try_from(with.len())) else { continue };
                with.push(BlockOp::AndNot(fv, r));
                // By induction on the positions, `f & ~R` itself the hypothesis: at the first
                // place it would hold, the fill was within `R` the byte before.
                let work = rerail_bitlex::bitlex_prove::Budget::new(2_000_000);
                let mut prover = rerail_bitlex::bitlex_prove::Induction::new(&program.rounds, 8, 100_000).within(&work);
                if prover.never_inductive(&with, &program.tables, at, program.delay) {
                    out.push((f, usize::from(q)));
                    break;
                }
            }
        }
        out
    });
    proved.iter().filter_map(|&(f, q)| {
        let held = by_op.get(&q).copied().or_else(|| comps.iter().position(|c| ops[c.op] == ops[q]))?;
        Some((*by_op.get(&f)?, held))
    }).collect()
}

/// The stacks by component.
pub fn stacks(program: &BlockProgram, comps: &[Component]) -> Vec<Stack> {
    let by = carried_by(program, comps);
    program.nests.iter().filter_map(|n| {
        let levels: Option<Vec<usize>> = n.levels.iter().map(|&v| by.get(&usize::from(v)).copied()).collect();
        let kinds: Option<Vec<Vec<usize>>> = n.kinds.iter().map(|k| k.iter().map(|&v| by.get(&usize::from(v)).copied()).collect()).collect();
        Some(Stack { levels: levels?, kinds: kinds?, within: n.within.and_then(|w| by.get(&usize::from(w)).copied()) })
    }).collect()
}

/// The components' shape: long-lived and lazy (`Shape`).
pub fn shape(program: &BlockProgram, comps: &[Component]) -> Shape {
    let mut long = vec![false; comps.len()];
    let mut lazy = vec![false; comps.len()];
    let walks = walks(program, comps);
    for &f in walks.fills.iter().flatten() {
        long[f] = true;
    }
    for st in stacks(program, comps) {
        for &l in &st.levels {
            long[l] = true;
        }
        for &k in st.kinds.iter().flatten() {
            lazy[k] = true;
            long[k] = false;
        }
    }
    for (i, c) in comps.iter().enumerate() {
        if matches!(program.ops[c.op], BlockOp::Toggle(_)) && !c.in_place {
            long[i] = true;
        }
    }
    for (i, j) in ties(program, comps) {
        long[i] |= long[j];
        lazy[i] |= lazy[j];
        long[i] &= !lazy[i];
    }
    Shape { long, lazy }
}

/// A walk's kinds open at the last places before a block, from a start: per place back from the
/// last (0: the last byte, the fills' carries; j: what each `prev(fill, k > j)` holds), the kinds
/// known open. False when two are, which no walk makes; or when a stack's level is open with
/// the one below it closed, or a level's kind with the level closed.
fn exclusive(walks: &Walks, stacks: &[Stack], start: &TStart, comps: &[Component]) -> bool {
    for (fills, prevs) in walks.fills.iter().zip(&walks.prevs) {
        for j in 0..8u8 {
            let mut open = 0;
            for (kind, &f) in fills.iter().enumerate() {
                let one = if j == 0 {
                    let t = start.get(comps[f].at);
                    t.k & t.v & 1 == 1
                } else {
                    prevs[kind].iter().any(|&(c, k)| k > j && {
                        let t = start.get(comps[c].at);
                        let bit = 1u64 << (k - 1 - j);
                        t.k & t.v & bit != 0
                    })
                };
                open += usize::from(one);
            }
            if open > 1 {
                return false;
            }
        }
    }
    let bit = |c: usize| start.get(comps[c].at).bit(0);
    for st in stacks {
        if st.within.is_some_and(|w| bit(w) == Some(false)) && bit(st.levels[0]) == Some(true) {
            return false;
        }
        for (j, &l) in st.levels.iter().enumerate() {
            let below_closed = j > 0 && bit(st.levels[j - 1]) == Some(false);
            if bit(l) == Some(true) && below_closed {
                return false;
            }
            if bit(l) == Some(false) && st.kinds.iter().any(|k| bit(k[j]) == Some(true)) {
                return false;
            }
        }
    }
    true
}

/// The ties applied to a start: a known side gives the other. `None` when the two disagree.
fn tied(program: &BlockProgram, comps: &[Component], ties: &[(usize, usize)], mut start: TStart) -> Option<TStart> {
    for &(i, j) in ties {
        let BlockOp::Prev(_, k) = program.ops[comps[i].op] else { unreachable!() };
        let top = 1u64 << (k - 1);
        let held = start.get(comps[i].at);
        let carry = start.get(comps[j].at);
        match (carry.k & 1 == 1, held.k & top != 0) {
            (true, true) if (carry.v & 1 == 1) != (held.v & top != 0) => return None,
            (true, false) => start.set(comps[i].at, T::norm(held.k | top, held.v | if carry.v & 1 == 1 { top } else { 0 })),
            (false, true) => {
                let b = u64::from(held.v & top != 0);
                start.set(comps[j].at, fixed(program, &comps[j], if matches!(program.ops[comps[j].op], BlockOp::Toggle(_)) { b.wrapping_neg() } else { b }));
            }
            _ => {}
        }
    }
    Some(start)
}

fn merge_all(a: &[T], b: &[T]) -> Vec<T> {
    a.iter().zip(b).map(|(p, q)| p.merge(*q)).collect()
}

/// Two starts merged: known where both agree.
fn merge_start(a: &TStart, b: &TStart) -> TStart {
    TStart { ops: merge_all(&a.ops, &b.ops), groups: a.groups.iter().zip(&b.groups).map(|(x, y)| merge_all(x, y)).collect(), backs: merge_all(&a.backs, &b.backs) }
}

/// One block's run from a state, for one guess of the back fills' carries into it.
struct Step {
    /// The state run (the guess in its `backs`).
    from: TStart,
    values: Vec<Vec<T>>,
    /// The state after the block, its `backs` the carries out of the block's first bit.
    end: TStart,
    open_refusal: bool,
    /// A long-lived component of `end` unknown.
    end_open: bool,
    /// A back fill's carry out of the block's first bit unknown: the guess for it, made by the
    /// block before, goes unchecked.
    back_open: bool,
}

struct Ctx<'a, 'p> {
    /// Per op, the start components it reads, at any depth (through carried operations too).
    cones: Vec<Vec<bool>>,
    p: &'a Prepared<'p>,
    comps: &'a [Component],
    file: &'a File,
    totals: Vec<usize>,
    outputs: Vec<usize>,
    ties: Vec<(usize, usize)>,
    walks: Walks,
    walk_of: std::collections::HashMap<usize, usize>,
    stacks: Vec<Stack>,
    shape: Shape,
    /// Per component, a stack kind's fill and the `prev`s tied to it (empty for any other).
    kind_comps: Vec<Vec<usize>>,
    /// Fills within a run, each with the `prev` that bounds its carry (`bounds`).
    bounds: Vec<(usize, usize)>,    /// Per value, the fill or toggle component whose carry is its last bit (`carried_by`).
    carried: std::collections::HashMap<usize, usize>,
    /// The `prev(x, 1)` components not read in place: `(component, x)`.
    lasts: Vec<(usize, usize)>,
}

impl Ctx<'_, '_> {
    fn program(&self) -> &BlockProgram {
        self.p.program
    }

    /// No two kinds of a set open at one place, and every stack's levels nested.
    fn consistent(&self, s: &TStart) -> bool {
        exclusive(&self.walks, &self.stacks, s, self.comps)
    }

    /// What a start's known closed levels say: a level closed (or the fill a stack is within)
    /// closes every level above it and its kinds, which are then known 0; the ties applied.
    /// `None` when the start is not consistent.
    fn settle(&self, mut s: TStart) -> Option<TStart> {
        let program = self.program();
        let comps = self.comps;
        let closed = |s: &TStart, c: usize| { let t = s.get(comps[c].at); t.k & 1 == 1 && t.v & 1 == 0 };
        for st in &self.stacks {
            let from = if st.within.is_some_and(|w| closed(&s, w)) { Some(0) } else { st.levels.iter().position(|&l| closed(&s, l)) };
            if let Some(j0) = from {
                for j in j0..st.levels.len() {
                    for &c in std::iter::once(&st.levels[j]).chain(st.kinds.iter().map(|k| &k[j])) {
                        let t = s.get(comps[c].at);
                        if t.k & 1 == 1 && t.v & 1 == 1 {
                            return None;
                        }
                        s.set(comps[c].at, fixed(program, &comps[c], 0));
                    }
                }
            }
        }
        // A fill within a run is open only where the run's last byte was.
        for &(f, r) in &self.bounds {
            let (ft, rt) = (s.get(comps[f].at), s.get(comps[r].at));
            match (ft.k & 1 == 1, ft.v & 1 == 1, rt.k & 1 == 1, rt.v & 1 == 1) {
                (true, true, true, false) => return None,
                (false, _, true, false) => s.set(comps[f].at, fixed(program, &comps[f], 0)),
                (true, true, false, _) => s.set(comps[r].at, T::norm(rt.k | 1, rt.v | 1)),
                _ => {}
            }
        }
        let mut s = tied(program, comps, &self.ties, s)?;
        // A `prev(x, 1)` holds `x`'s last bit: where `x` combines values whose last bits the
        // start holds (a fill's or toggle's carry), that bit.
        for &(i, x) in &self.lasts {
            let t = s.get(comps[i].at);
            if t.k & 1 == 0 {
                if let Some(b) = self.last_bit(&s, x, 6) {
                    s.set(comps[i].at, T::norm(t.k | 1, t.v | u64::from(b)));
                }
            }
        }
        Some(s).filter(|x| self.consistent(x))
    }

    /// Value `x`'s last bit at the block before a start, where the start holds it: a fill's or
    /// toggle's carry, or a combination of them (`depth` deep).
    fn last_bit(&self, s: &TStart, x: usize, depth: u8) -> Option<bool> {
        if let Some(&c) = self.carried.get(&x) {
            let t = s.get(self.comps[c].at);
            return (t.k & 1 == 1).then_some(t.v & 1 == 1);
        }
        if depth == 0 {
            return None;
        }
        let bit = |v: u16| self.last_bit(s, usize::from(v), depth - 1);
        match self.program().ops[x] {
            BlockOp::Or(a, b) => match (bit(a), bit(b)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            BlockOp::And(a, b) => match (bit(a), bit(b)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            BlockOp::AndNot(a, b) => match (bit(a), bit(b)) {
                (Some(false), _) | (_, Some(true)) => Some(false),
                (Some(true), Some(false)) => Some(true),
                _ => None,
            },
            BlockOp::Not(a) => bit(a).map(|b| !b),
            _ => None,
        }
    }

    /// Block `b` from state `s`, each guess of the back fills' carries into it; `pending` is what
    /// the block before assumed of this block's carry out. The runs not refuted.
    fn block(&self, s: &TStart, pending: &[T], b: usize, out: &mut Solved) -> Vec<Step> {
        let program = self.program();
        let mut guesses = vec![s.clone()];
        for i in 0..s.backs.len() {
            guesses = guesses.into_iter().flat_map(|g| [0u64, 1].map(|v| { let mut t = g.clone(); t.backs[i] = T::norm(1, v); t })).collect();
        }
        let mut steps = Vec::new();
        for g in guesses {
            let Some(g) = self.settle(g) else {
                out.refuted += 1;
                continue;
            };
            let (values, end) = self.p.run_t(self.file, b..b + 1, &g);
            out.runs += 1;
            let wrong_back = end.backs.iter().zip(pending).any(|(x, q)| x.k & q.k & 1 == 1 && (x.v ^ q.v) & 1 == 1);
            if wrong_back || self.totals.iter().any(|&t| values[t][0].v != 0) {
                out.refuted += 1;
                continue;
            }
            let open_refusal = self.totals.iter().any(|&t| !values[t][0].full());
            // A stack level's kind left unknown is the entry's, or decides nothing yet: the join
            // takes it from the chunk before (`Leaf::pass`).
            let end_open = self.comps.iter().enumerate().any(|(i, c)| !c.in_place && !self.shape.lazy[i] && !matches!(c.at, Place::Back(_)) && !known_in(program, c, &end));
            let back_open = end.backs.iter().any(|x| x.k & 1 == 0);
            steps.push(Step { from: g, values, end, open_refusal, end_open, back_open });
        }
        steps
    }

    /// A component of `s` to split, by rank: unknown in `s`, in the cone of a value `t` leaves
    /// unknown (a refusal, an output, a component of its end), and (preferred) unknown in the
    /// state the block ran from.
    /// Before the chunk (`outputs` false) only the long-lived state (rank 0) is split, by the
    /// cone: what the lookback block overwrites is left to it. Inside the chunk the direct causes
    /// come first.
    fn pick_for(&self, s: &TStart, t: &Step, outputs: bool) -> Option<usize> {
        let program = self.program();
        let mut want = vec![false; self.comps.len()];
        let mut mark = |o: usize| for (w, &c) in want.iter_mut().zip(&self.cones[o]) { *w |= c; };
        for &o in &self.totals {
            if !t.values[o][0].full() {
                mark(o);
            }
        }
        if outputs {
            for &o in &self.outputs {
                if !t.values[o][0].full() {
                    mark(o);
                }
            }
        }
        for (_, c) in self.comps.iter().enumerate().filter(|(i, c)| !c.in_place && !self.shape.lazy[*i] && !matches!(c.at, Place::Back(_)) && !known_in(program, c, &t.end)) {
            mark(c.op);
        }
        if t.back_open {
            for &x in &self.p.backs {
                mark(x);
            }
        }
        // The direct causes first: the values left unknown here, their ancestors that are unknown
        // with every operand known (a carried operation's own carry, then) — a component whose
        // operation is one of them.
        let mut targets: Vec<usize> = self.totals.iter().copied().filter(|&o| !t.values[o][0].full()).collect();
        if outputs {
            targets.extend(self.outputs.iter().copied().filter(|&o| !t.values[o][0].full()));
        }
        targets.extend(self.comps.iter().enumerate().filter(|(i, c)| !c.in_place && !self.shape.lazy[*i] && !matches!(c.at, Place::Back(_)) && !known_in(program, c, &t.end)).map(|(_, c)| c.op));
        if t.back_open {
            targets.extend(self.p.backs.iter().copied());
        }
        let mut anc = vec![false; program.ops.len()];
        while let Some(x) = targets.pop() {
            if !std::mem::replace(&mut anc[x], true) {
                targets.extend(program.ops[x].operands().into_iter().map(usize::from));
            }
        }
        let source = |x: usize| anc[x] && !t.values[x][0].full() && program.ops[x].operands().iter().all(|&o| t.values[usize::from(o)][0].full());
        let best = |direct: bool| self.comps.iter().enumerate()
            .filter(|(i, c)| want[*i] && !c.in_place && !known_in(program, c, s) && (!direct || source(c.op)))
            .filter_map(|(i, c)| split_rank(program, &self.shape, i, c).filter(|&r| outputs || r == 0).map(|r| (r, known_in(program, c, &t.from), c.op, i)))
            .min()
            .map(|(.., i)| i);
        if outputs { best(true).or_else(|| best(false)) } else { best(false) }
    }

    /// `s` split on component `i`: a walk's fill one world with no kind open and one a kind that
    /// may be (at most one is); else each value its known bits allow.
    fn split(&self, s: &TStart, i: usize) -> Vec<TStart> {
        let program = self.program();
        let comps = self.comps;
        // A `prev` of a set's fill splits as the fill does: the set one-hot.
        let i = self.ties.iter().find(|t| t.0 == i && self.walk_of.contains_key(&t.1) && !known_in(program, &comps[t.1], s)).map_or(i, |t| t.1);
        if let Some(&wk) = self.walk_of.get(&i) {
            let fills = &self.walks.fills[wk];
            let free: Vec<usize> = fills.iter().copied().filter(|&f| !known_in(program, &comps[f], s)).collect();
            let any_open = fills.iter().any(|&f| { let t = s.get(comps[f].at); t.k & t.v & 1 == 1 });
            let mut none = s.clone();
            for &f in &free {
                none.set(comps[f].at, fixed(program, &comps[f], 0));
            }
            let mut out = vec![none.clone()];
            if !any_open {
                for &f in &free {
                    let mut t = none.clone();
                    t.set(comps[f].at, fixed(program, &comps[f], 1));
                    out.push(t);
                }
            }
            return out;
        }
        let have = s.get(comps[i].at);
        values_of(program, &comps[i]).into_iter().filter(|v| v & have.k == have.v).map(|v| {
            let mut t = s.clone();
            t.set(comps[i].at, fixed(program, &comps[i], v));
            t
        }).collect()
    }
}

/// A world inside the chunk: a state into a block, what it assumed of the block before, and what
/// it has made.
#[derive(Clone)]
struct World {
    state: TStart,
    pending: Vec<T>,
    back_out: Vec<T>,
    outputs: Vec<Vec<T>>,
    clean: bool,
    /// Per stack, per level: open, and known so, at every place so far.
    held: Vec<Vec<bool>>,
}

/// Chunk `range` of `file` solved from an unknown start, in two phases.
///
/// - **To the chunk's entry.** From `look` blocks before the chunk, every carry unknown but the
///   components read in place (at the file's start every carry is zero, which is known), block
///   by block with a set of worlds: where a block leaves its end or a refusal unknown, the world
///   is split on its state's component that causes it (the region walk's kind, the long-lived
///   carries first). Worlds in one state merge. What is left at the chunk's first block are its
///   entries.
/// - **Each entry through the chunk.** Where a block leaves something unknown, the entry is
///   split instead (an output left unknown counts), and the variants run again from the chunk's
///   first block: each leaf is then a function of its entry, which the join needs.
///
/// A refusal with a known bit refutes a world; a back fill's carry into each block is guessed
/// and checked against the next block's run.
pub fn solve(p: &Prepared<'_>, comps: &[Component], file: &File, range: Range<usize>, look: usize, cap: usize) -> Solved {
    solve_to(p, comps, file, range, look, cap, true, Entries::All)
}

/// The run to the chunk's entry only (`solve`'s first phase): `Solved::entry_states`.
pub fn entries(p: &Prepared<'_>, comps: &[Component], file: &File, range: Range<usize>, look: usize, cap: usize) -> Solved {
    solve_to(p, comps, file, range, look, cap, false, Entries::All)
}

/// Which entries a chunk solves through.
pub enum Entries<'r> {
    /// Every entry the run to the chunk leaves.
    All,
    /// The entries of the best `M` states by the priors (`rank`); the rest are `Solved::dropped`.
    Ranked(&'r crate::rank::Priors, usize),
    /// These entries, with no run to the chunk: a chunk run again from its now known start.
    Given(Vec<TStart>),
}

/// `solve`, through the entries `how` says.
pub fn solve_with(p: &Prepared<'_>, comps: &[Component], file: &File, range: Range<usize>, look: usize, cap: usize, how: Entries<'_>) -> Solved {
    solve_to(p, comps, file, range, look, cap, true, how)
}

#[allow(clippy::too_many_arguments)]
fn solve_to(p: &Prepared<'_>, comps: &[Component], file: &File, range: Range<usize>, look: usize, cap: usize, through: bool, how: Entries<'_>) -> Solved {
    let program = p.program;
    let walks = walks(program, comps);
    let walk_of = walks.fills.iter().enumerate().flat_map(|(w, f)| f.iter().map(move |&c| (c, w))).collect();
    let cx = Ctx {
        cones: cones(program, comps),
        p,
        comps,
        file,
        totals: program.totals.iter().map(|&t| usize::from(t)).collect(),
        outputs: program.outputs.iter().map(|&o| usize::from(o)).collect(),
        ties: ties(program, comps),
        walks,
        walk_of,
        stacks: stacks(program, comps),
        shape: shape(program, comps),
        bounds: bounds(p, comps),
        carried: carried_by(program, comps),
        lasts: comps.iter().enumerate().filter_map(|(i, c)| match program.ops[c.op] {
            BlockOp::Prev(x, 1) if !c.in_place => Some((i, usize::from(x))),
            _ => None,
        }).collect(),
        kind_comps: {
            let mut k: Vec<Vec<usize>> = vec![Vec::new(); comps.len()];
            let ties = ties(program, comps);
            for st in stacks(program, comps) {
                for &f in st.kinds.iter().flatten() {
                    k[f].push(f);
                    k[f].extend(ties.iter().filter(|t| t.1 == f).map(|t| t.0));
                }
            }
            k
        },
    };
    let from = range.start.saturating_sub(look);
    let base = if from == 0 {
        let z = p.zero();
        TStart { ops: z.ops.iter().map(|&v| T::known(v)).collect(), groups: z.groups.iter().map(|g| g.iter().map(|&v| T::known(v)).collect()).collect(), backs: z.backs.iter().map(|_| T::X).collect() }
    } else {
        let mut base = p.unknown();
        let mut s = p.zero();
        crate::starts::in_place(p, comps, file, from, &mut s);
        for c in comps.iter().filter(|c| c.in_place) {
            base.set(c.at, T::known(c.at.get(&s)));
        }
        base
    };
    let nb = p.backs.len();
    let mut out = Solved { from, ..Solved::default() };
    let trace = std::env::var_os("RERAIL_FM_TRACE").is_some();
    let t0 = std::time::Instant::now();
    // To the entry.
    // Given entries start at the chunk, with no run to it.
    let (mut worlds, lookback): (Vec<(TStart, Vec<T>)>, Range<usize>) = match &how {
        Entries::Given(es) => (es.iter().map(|e| (e.clone(), vec![T::X; nb])).collect(), range.start..range.start),
        _ => (vec![(base, vec![T::X; nb])], from..range.start),
    };
    for b in lookback {
        let mut next: Vec<(TStart, Vec<T>)> = Vec::new();
        for (state, pending) in worlds {
            let mut queue = vec![state];
            let mut made = 0usize;
            while let Some(s) = queue.pop() {
                let steps = cx.block(&s, &pending, b, &mut out);
                // Only long-lived components split here, so the tree is bounded by them.
                if made < 4096 && steps.iter().any(|t| t.open_refusal || t.end_open) {
                    let t = steps.iter().find(|t| t.open_refusal || t.end_open).expect("an open step");
                    if let Some(i) = cx.pick_for(&s, t, false) {
                        queue.extend(cx.split(&s, i));
                        made += 1;
                        continue;
                    }
                }
                for t in steps {
                    let mut st = t.end;
                    st.backs.iter_mut().for_each(|x| *x = T::X);
                    // The state that lives across many blocks concrete at the edge (`Shape::long`:
                    // the region open, a stack's depth, the toggles), each one unknown split into
                    // its values (a set's kinds one-hot, a stack's levels nested), the ties
                    // applied. A stack's kinds stay unknown (`Shape::lazy`), and what the next
                    // block's bytes decide is left to them.
                    let mut edge = vec![st];
                    loop {
                        let mut more = false;
                        let mut split_all = Vec::new();
                        for e in edge {
                            match comps.iter().enumerate().position(|(i, c)| cx.shape.long[i] && !known_in(program, c, &e)) {
                                Some(i) => {
                                    more = true;
                                    split_all.extend(cx.split(&e, i).into_iter().filter_map(|x| cx.settle(x)));
                                }
                                None => split_all.extend(cx.settle(e)),
                            }
                        }
                        edge = split_all;
                        if !more {
                            break;
                        }
                    }
                    for st in edge {
                        let w = (st, t.from.backs.clone());
                        if !next.contains(&w) {
                            next.push(w);
                        }
                    }
                }
            }
        }
        if next.len() > cap {
            out.over = true;
            let per = next.len().div_ceil(cap);
            next = next.chunks(per).map(|c| c.iter().skip(1).fold(c[0].clone(), |a, b| (merge_start(&a.0, &b.0), merge_all(&a.1, &b.1)))).collect();
        }
        out.widest = out.widest.max(next.len());
        if trace {
            eprintln!("   trace: lookback block {b}: {} worlds, {} runs, {} refuted, {:.0} ms", next.len(), out.runs, out.refuted, t0.elapsed().as_secs_f64() * 1e3);
        }
        worlds = next;
    }
    out.entries = worlds.len();
    out.entry_states = worlds.iter().map(|w| w.0.clone()).collect();
    if !through {
        return out;
    }
    // The entries of the best `M` states, each its state's place.
    let mut ranks = vec![0usize; worlds.len()];
    if let Entries::Ranked(priors, m) = &how {
        let keyed: Vec<(Vec<u8>, f64)> = worlds.iter().map(|w| (priors.keys.of(comps, &w.0), priors.score(p, comps, file, range.clone(), &w.0))).collect();
        let mut best: Vec<(Vec<u8>, f64)> = Vec::new();
        for (k, sc) in &keyed {
            match best.iter_mut().find(|b| &b.0 == k) {
                Some(b) => b.1 = b.1.max(*sc),
                None => best.push((k.clone(), *sc)),
            }
        }
        best.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut kept = Vec::new();
        let mut kept_ranks = Vec::new();
        for (w, (k, _)) in worlds.into_iter().zip(&keyed) {
            match best.iter().position(|b| &b.0 == k).filter(|&r| r < *m) {
                Some(r) => {
                    kept.push(w);
                    kept_ranks.push(r);
                }
                None => out.dropped.push(w.0),
            }
        }
        worlds = kept;
        ranks = kept_ranks;
    }
    // Each entry through the chunk.
    for ((entry, pending0), rank) in worlds.into_iter().zip(ranks) {
        let mut variants = vec![entry];
        let mut splits = 0usize;
        out.splits.push(0);
        while let Some(v) = variants.pop() {
            let held0: Vec<Vec<bool>> = cx.stacks.iter().map(|st| vec![true; st.levels.len()]).collect();
            let mut ws = vec![World { state: v.clone(), pending: pending0.clone(), back_out: vec![T::X; nb], outputs: vec![Vec::new(); cx.outputs.len()], clean: true, held: held0 }];
            let mut need: Option<usize> = None;
            for b in range.clone() {
                let mut next: Vec<World> = Vec::new();
                for w in &ws {
                    for t in cx.block(&w.state, &w.pending, b, &mut out) {
                        // Inside the chunk an output unknown is open too: the chunk is decided
                        // only where its outputs are known.
                        let open_out = cx.outputs.iter().any(|&o| !t.values[o][0].full());
                        if need.is_none() && splits < 256 && (t.open_refusal || t.end_open || open_out || t.back_open) {
                            need = cx.pick_for(&v, &t, true);
                        }
                        if splits >= 256 && out.open_at_budget.len() < 4 && (t.open_refusal || t.end_open || open_out || t.back_open) {
                            // The unknown output's sources: ancestors unknown whose operands are known.
                            let mut anc = vec![false; program.ops.len()];
                            let mut todo: Vec<usize> = cx.outputs.iter().copied().filter(|&o| !t.values[o][0].full()).collect();
                            while let Some(x) = todo.pop() {
                                if !std::mem::replace(&mut anc[x], true) {
                                    todo.extend(program.ops[x].operands().into_iter().map(usize::from));
                                }
                            }
                            let sources: Vec<usize> = (0..program.ops.len()).filter(|&x| anc[x] && !t.values[x][0].full() && program.ops[x].operands().iter().all(|&o| t.values[usize::from(o)][0].full())).collect();
                            out.open_at_budget.push((
                                sources,
                                comps.iter().filter(|c| !c.in_place && !matches!(c.at, Place::Back(_)) && !known_in(program, c, &t.end)).map(|c| c.op).collect(),
                                cx.outputs.iter().copied().filter(|&o| !t.values[o][0].full()).collect(),
                                t.back_open,
                            ));
                        }
                        let mut n = w.clone();
                        if b == range.start {
                            n.back_out = t.end.backs.clone();
                        }
                        for (j, &o) in cx.outputs.iter().enumerate() {
                            n.outputs[j].push(t.values[o][0]);
                        }
                        n.clean &= !t.open_refusal;
                        for (st, held) in cx.stacks.iter().zip(n.held.iter_mut()) {
                            for (h, &l) in held.iter_mut().zip(&st.levels) {
                                let x = t.values[comps[l].op][0];
                                *h &= x.full() && x.v == u64::MAX;
                            }
                        }
                        n.pending = t.from.backs.clone();
                        n.state = t.end;
                        n.state.backs.iter_mut().for_each(|x| *x = T::X);
                        match next.iter_mut().find(|m| m.state == n.state && m.pending == n.pending) {
                            Some(m) => {
                                m.back_out = merge_all(&m.back_out, &n.back_out);
                                for (a, b) in m.outputs.iter_mut().zip(&n.outputs) {
                                    *a = merge_all(a, b);
                                }
                                m.clean &= n.clean;
                                for (a, b) in m.held.iter_mut().zip(&n.held) {
                                    for (x, y) in a.iter_mut().zip(b) {
                                        *x &= *y;
                                    }
                                }
                            }
                            None => next.push(n),
                        }
                    }
                }
                ws = next;
                if need.is_some() {
                    break;
                }
            }
            if need.is_none() && splits >= 256 {
                out.spent += 1;
            }
            if trace {
                if let Some(i) = need {
                    eprintln!("   trace: entry split {splits} on component {i} #{} {}, {} runs, {:.0} ms", comps[i].op, program.ops[comps[i].op].text(), out.runs, t0.elapsed().as_secs_f64() * 1e3);
                }
            }
            if let Some(i) = need {
                splits += 1;
                *out.splits.last_mut().expect("this entry's") = splits;
                variants.extend(cx.split(&v, i));
                continue;
            }
            out.widest = out.widest.max(ws.len());
            let mut e = v.clone();
            e.backs.iter_mut().for_each(|x| *x = T::X);
            for w in ws {
                let end_known = comps.iter().enumerate().all(|(i, c)| c.in_place || cx.shape.lazy[i] || matches!(c.at, Place::Back(_)) || known_in(program, c, &w.state));
                // The kinds of the levels the chunk never went below are the entry's.
                let mut pass = Vec::new();
                for (st, held) in cx.stacks.iter().zip(&w.held) {
                    for (j, _) in st.levels.iter().enumerate().take_while(|&(j, &l)| held[j] && v.get(comps[l].at).bit(0) == Some(true)) {
                        for k in &st.kinds {
                            pass.extend(cx.kind_comps[k[j]].iter().map(|&c| comps[c].at));
                        }
                    }
                }
                let leaf = Leaf { entry: e.clone(), end: w.state, back_in: w.pending, back_out: w.back_out, outputs: w.outputs, clean: w.clean && end_known, pass, rank };
                match out.leaves.iter_mut().find(|l| l.end == leaf.end && l.back_in == leaf.back_in && l.entry == leaf.entry && l.pass == leaf.pass) {
                    Some(l) => {
                        l.back_out = merge_all(&l.back_out, &leaf.back_out);
                        for (a, b) in l.outputs.iter_mut().zip(&leaf.outputs) {
                            *a = merge_all(a, b);
                        }
                        l.clean &= leaf.clean;
                    }
                    None => out.leaves.push(leaf),
                }
            }
        }
    }
    out
}

/// Per op, the start components in its cone: the components whose operation it reads, at any
/// depth, through every operation (a carried one reads its operands as any does).
pub fn cones(program: &BlockProgram, comps: &[Component]) -> Vec<Vec<bool>> {
    let n = program.ops.len();
    let mut of_op = vec![Vec::new(); n];
    for (i, c) in comps.iter().enumerate() {
        of_op[c.op].push(i);
    }
    // Ops in a group read each other: iterate to a fixed point.
    let mut cone: Vec<Vec<bool>> = vec![vec![false; comps.len()]; n];
    loop {
        let mut changed = false;
        for x in 0..n {
            let mut c = cone[x].clone();
            for &i in &of_op[x] {
                c[i] = true;
            }
            for o in program.ops[x].operands() {
                for (a, &b) in c.iter_mut().zip(&cone[usize::from(o)]) {
                    *a |= b;
                }
            }
            if c != cone[x] {
                cone[x] = c;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    cone
}
