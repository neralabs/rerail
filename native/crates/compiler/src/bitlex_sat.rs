//! A small CDCL SAT solver over clauses: the reference the circuit's solver (`bitlex_csat`, decision 0670) is
//! tested against. The prover's windows are solved on the circuit.
//!
//! Clauses are lists of literals: a variable `v` (from 1) is `v` true or `-v` false. The search
//! is the usual one: two watched literals a clause, unit propagation, a conflict's first unique
//! implication point learned as a clause, a jump back to its second-highest level. Decisions take
//! the most active variable (activity bumped in each learned clause; of equal ones, the highest),
//! from a heap ordered by activity: a decision costs the log of the variables, not their number.
//! A conflict budget bounds the work: past it, the answer is "unknown", which a proof reads as
//! "not proved" (sound).

use crate::bitlex_grown::{FormulaBlock, Grown};

/// A literal: a variable from 1, negative for its negation.
pub type Lit = i32;

/// `solve`'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Sat,
    Unsat,
    Unknown,
}

/// Clauses end to end, in one arena: each a run of `lits`, ending at its `ends` entry. A
/// window's thousands of clauses are two allocations, not one each.
#[derive(Clone, Debug, Default)]
pub struct Clauses {
    lits: Grown<Lit>,
    ends: Grown<u32>,
}

impl Clauses {
    /// None yet, built in `block`, where a kernel reads them (`bitlex_grown`).
    pub fn in_block(block: &FormulaBlock) -> Clauses {
        Clauses { lits: Grown::in_block(block), ends: Grown::in_block(block) }
    }

    /// Its literals and ends: their block and byte offsets there (`None` on the heap).
    pub fn placed(&self) -> Option<(&FormulaBlock, usize, usize)> {
        let (b, lits) = self.lits.placed()?;
        Some((b, lits, self.ends.placed()?.1))
    }

    pub fn push(&mut self, c: &[Lit]) {
        self.lits.extend_from_slice(c);
        self.ends.push(self.lits.len() as u32);
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    fn range(&self, i: usize) -> std::ops::Range<usize> {
        (if i == 0 { 0 } else { self.ends[i - 1] as usize })..self.ends[i] as usize
    }

    pub fn get(&self, i: usize) -> &[Lit] {
        &self.lits[self.range(i)]
    }

    pub(crate) fn get_mut(&mut self, i: usize) -> &mut [Lit] {
        let r = self.range(i);
        &mut self.lits[r]
    }

    pub fn iter(&self) -> impl Iterator<Item = &[Lit]> {
        (0..self.len()).map(|i| self.get(i))
    }

    /// Every clause's literals, end to end.
    pub fn lits(&self) -> &[Lit] {
        &self.lits
    }

    /// Where each clause ends in `lits`.
    pub fn ends(&self) -> &[u32] {
        &self.ends
    }

    /// The first `n` clauses kept, the rest dropped.
    pub fn truncate(&mut self, n: usize) {
        if n < self.ends.len() {
            self.ends.truncate(n);
            self.lits.truncate(self.ends.last().map_or(0, |&e| e as usize));
        }
    }
}

/// `lits` sorted, without repeats, into `c`: its length, or none where it is a tautology (a
/// literal and its negation), as `Solver::add` takes a clause. A short clause (a window's are
/// two or three literals) sorted in place, a longer one by `sort_unstable`: the same order.
pub(crate) fn normal(lits: &[Lit], c: &mut Vec<Lit>) -> Option<usize> {
    c.clear();
    c.extend_from_slice(lits);
    if c.len() <= 8 {
        for i in 1..c.len() {
            let x = c[i];
            let mut j = i;
            while j > 0 && c[j - 1] > x {
                c[j] = c[j - 1];
                j -= 1;
            }
            c[j] = x;
        }
    } else {
        c.sort_unstable();
    }
    c.dedup();
    // Sorted: a literal's negation, if there, by a search (a scan, for a short one).
    let taut = if c.len() <= 8 { c.iter().any(|&a| c.contains(&-a)) } else { c.iter().any(|&a| c.binary_search(&-a).is_ok()) };
    (!taut).then_some(c.len())
}

/// Not in the heap (`Solver::at`).
const NONE: usize = usize::MAX;

fn var(l: Lit) -> usize {
    l.unsigned_abs() as usize
}

/// A literal's watch slot: `2v` for `v`, `2v + 1` for `-v`.
fn slot(l: Lit) -> usize {
    2 * var(l) + usize::from(l < 0)
}

/// The variables in the order decisions take them (`Solver`'s and `bitlex_csat`'s): the unset ones (and some set
/// ones, dropped when they come up) in a max-heap by activity, then by number; `at` each variable's place in it
/// (`NONE`: not in it). Activity is bumped in each learned clause, and the bump grows each conflict.
#[derive(Default)]
pub(crate) struct Order {
    activity: Vec<f64>,
    bump: f64,
    heap: Vec<usize>,
    at: Vec<usize>,
}

impl Order {
    /// `vars` variables, every activity 0: the heap's order is the numbers', highest first, a valid
    /// heap as it stands.
    pub(crate) fn reset(&mut self, vars: usize) {
        self.at.clear();
        self.at.resize(vars + 1, NONE);
        self.activity.clear();
        self.activity.resize(vars + 1, 0.0);
        self.bump = 1.0;
        self.heap.clear();
        self.heap.extend((1..=vars).rev());
        for (i, &v) in self.heap.iter().enumerate() {
            self.at[v] = i;
        }
    }

    /// The next conflict's bump.
    pub(crate) fn decay(&mut self) {
        self.bump *= 1.05;
    }

    /// The most active unset variable (`value` 0), taken out of the heap.
    pub(crate) fn next(&mut self, value: &[i8]) -> Option<usize> {
        loop {
            match self.pop() {
                Some(v) if value[v] != 0 => continue,
                other => return other,
            }
        }
    }

    /// Whether `a` comes before `b` in the heap: more active, or as active and higher.
    fn before(&self, a: usize, b: usize) -> bool {
        match self.activity[a].total_cmp(&self.activity[b]) {
            std::cmp::Ordering::Equal => a > b,
            o => o == std::cmp::Ordering::Greater,
        }
    }

    fn up(&mut self, mut i: usize) {
        let v = self.heap[i];
        while i > 0 {
            let p = (i - 1) / 2;
            if !self.before(v, self.heap[p]) {
                break;
            }
            self.heap[i] = self.heap[p];
            self.at[self.heap[i]] = i;
            i = p;
        }
        self.heap[i] = v;
        self.at[v] = i;
    }

    fn down(&mut self, mut i: usize) {
        let v = self.heap[i];
        let n = self.heap.len();
        loop {
            let mut c = 2 * i + 1;
            if c >= n {
                break;
            }
            if c + 1 < n && self.before(self.heap[c + 1], self.heap[c]) {
                c += 1;
            }
            if !self.before(self.heap[c], v) {
                break;
            }
            self.heap[i] = self.heap[c];
            self.at[self.heap[i]] = i;
            i = c;
        }
        self.heap[i] = v;
        self.at[v] = i;
    }

    pub(crate) fn insert(&mut self, v: usize) {
        if self.at[v] == NONE {
            self.heap.push(v);
            let i = self.heap.len() - 1;
            self.at[v] = i;
            self.up(i);
        }
    }

    fn pop(&mut self) -> Option<usize> {
        let top = *self.heap.first()?;
        let last = self.heap.pop().expect("one");
        self.at[top] = NONE;
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.at[last] = 0;
            self.down(0);
        }
        Some(top)
    }

    /// A variable's activity raised; past 1e100, every one scaled down alike (the order kept).
    pub(crate) fn bump(&mut self, v: usize) {
        self.activity[v] += self.bump;
        if self.activity[v] > 1e100 {
            for a in &mut self.activity {
                *a *= 1e-100;
            }
            self.bump *= 1e-100;
        }
        if self.at[v] != NONE {
            self.up(self.at[v]);
        }
    }
}

pub struct Solver {
    clauses: Clauses,
    /// `add`'s clause, sorted (kept to reuse its allocation).
    scratch: Vec<Lit>,
    watches: Vec<Vec<usize>>,
    /// Each variable's value: 0 unset, 1 true, -1 false.
    value: Vec<i8>,
    level: Vec<u32>,
    reason: Vec<Option<usize>>,
    trail: Vec<Lit>,
    /// Where each decision level starts in the trail.
    limits: Vec<usize>,
    head: usize,
    order: Order,
    /// `analyze`'s marks, all false between conflicts.
    seen: Vec<bool>,
    /// The clauses given empty, or two units against each other: unsatisfiable at once.
    broken: bool,
    /// The conflicts its solves met, in all (a caller's work budget reads it).
    pub conflicts: u64,
}

impl Solver {
    pub fn new(vars: usize) -> Self {
        let mut s = Solver {
            clauses: Clauses::default(),
            scratch: Vec::new(),
            watches: Vec::new(),
            value: Vec::new(),
            level: Vec::new(),
            reason: Vec::new(),
            trail: Vec::new(),
            limits: Vec::new(),
            head: 0,
            order: Order::default(),
            seen: Vec::new(),
            broken: false,
            conflicts: 0,
        };
        s.reset(vars);
        s
    }

    /// No clauses, of `vars` variables: as `new`, in the memory this one has (a prover asks
    /// thousands of formulas, each of tens of thousands of clauses).
    pub fn reset(&mut self, vars: usize) {
        self.clauses.lits.clear();
        self.clauses.ends.clear();
        for w in &mut self.watches {
            w.clear();
        }
        self.watches.resize_with(2 * vars + 2, Vec::new);
        self.value.clear();
        self.value.resize(vars + 1, 0);
        self.level.clear();
        self.level.resize(vars + 1, 0);
        self.reason.clear();
        self.reason.resize(vars + 1, None);
        self.seen.clear();
        self.seen.resize(vars + 1, false);
        self.trail.clear();
        self.limits.clear();
        (self.head, self.broken) = (0, false);
        self.order.reset(vars);
    }

    fn lit_value(&self, l: Lit) -> i8 {
        let v = self.value[var(l)];
        if l < 0 { -v } else { v }
    }

    fn assign(&mut self, l: Lit, reason: Option<usize>) {
        let v = var(l);
        self.value[v] = if l > 0 { 1 } else { -1 };
        self.level[v] = self.limits.len() as u32;
        self.reason[v] = reason;
        self.trail.push(l);
    }

    /// A clause, before the search: duplicates dropped, a tautology ignored.
    pub fn add(&mut self, lits: &[Lit]) {
        let mut c = std::mem::take(&mut self.scratch);
        // A tautology (a literal and its negation) is ignored.
        if let Some(n) = normal(lits, &mut c) {
            self.place(&c[..n]);
        }
        self.scratch = c;
    }

    /// A clause sorted, without repeats or a tautology: watched, or a unit assigned.
    fn place(&mut self, c: &[Lit]) {
        match c.len() {
            0 => self.broken = true,
            1 => match self.lit_value(c[0]) {
                1 => {}
                -1 => self.broken = true,
                _ => self.assign(c[0], None),
            },
            _ => {
                let i = self.clauses.len();
                self.watches[slot(-c[0])].push(i);
                self.watches[slot(-c[1])].push(i);
                self.clauses.push(c);
            }
        }
    }

    /// Each of `clauses`, as `add` takes it.
    pub fn add_all(&mut self, clauses: &Clauses) {
        for c in clauses.iter() {
            self.add(c);
        }
    }

    /// Unit propagation from the trail's head: a conflicting clause, if any.
    fn propagate(&mut self) -> Option<usize> {
        while self.head < self.trail.len() {
            let l = self.trail[self.head];
            self.head += 1;
            // The clauses watching `-l`... stored at `slot(l)`: those whose watched literal `l`
            // made false.
            let mut ws = std::mem::take(&mut self.watches[slot(l)]);
            let mut i = 0;
            while i < ws.len() {
                let ci = ws[i];
                let falsified = -l;
                // The other watch first.
                if self.clauses.get(ci)[0] == falsified {
                    self.clauses.get_mut(ci).swap(0, 1);
                }
                let other = self.clauses.get(ci)[0];
                if self.lit_value(other) == 1 {
                    i += 1;
                    continue;
                }
                // A new literal to watch: not false.
                let mut moved = false;
                for k in 2..self.clauses.get(ci).len() {
                    let x = self.clauses.get(ci)[k];
                    if self.lit_value(x) != -1 {
                        self.clauses.get_mut(ci).swap(1, k);
                        self.watches[slot(-x)].push(ci);
                        ws.swap_remove(i);
                        moved = true;
                        break;
                    }
                }
                if moved {
                    continue;
                }
                if self.lit_value(other) == -1 {
                    self.watches[slot(l)] = ws;
                    return Some(ci);
                }
                self.assign(other, Some(ci));
                i += 1;
            }
            self.watches[slot(l)] = ws;
        }
        None
    }

    /// The conflict's first unique implication point: the learned clause (its asserting literal
    /// first) and the level to go back to.
    fn analyze(&mut self, conflict: usize) -> (Vec<Lit>, u32) {
        let here = self.limits.len() as u32;
        let mut learned: Vec<Lit> = vec![0];
        let mut count = 0;
        let mut at = self.trail.len();
        let mut clause = conflict;
        let mut p: Option<Lit> = None;
        loop {
            for k in 0..self.clauses.get(clause).len() {
                let q = self.clauses.get(clause)[k];
                if Some(q) == p {
                    continue;
                }
                let v = var(q);
                if self.seen[v] || self.level[v] == 0 {
                    continue;
                }
                self.seen[v] = true;
                self.order.bump(v);
                if self.level[v] == here {
                    count += 1;
                } else {
                    learned.push(q);
                }
            }
            // The latest seen literal on the trail.
            loop {
                at -= 1;
                if self.seen[var(self.trail[at])] {
                    break;
                }
            }
            let l = self.trail[at];
            self.seen[var(l)] = false;
            count -= 1;
            if count == 0 {
                learned[0] = -l;
                break;
            }
            p = Some(l);
            clause = self.reason[var(l)].expect("an implied literal");
        }
        for &q in &learned[1..] {
            self.seen[var(q)] = false;
        }
        self.order.decay();
        let back = learned[1..].iter().map(|&q| self.level[var(q)]).max().unwrap_or(0);
        // The literal of that level second, so it is watched.
        if learned.len() > 1 {
            let k = (1..learned.len()).max_by_key(|&k| self.level[var(learned[k])]).expect("one");
            learned.swap(1, k);
        }
        (learned, back)
    }

    fn back_to(&mut self, level: u32) {
        let Some(&start) = self.limits.get(level as usize) else { return };
        for i in start..self.trail.len() {
            let v = var(self.trail[i]);
            self.value[v] = 0;
            self.reason[v] = None;
            self.order.insert(v);
        }
        self.trail.truncate(start);
        self.limits.truncate(level as usize);
        self.head = start;
    }

    /// A variable's value after `Sat`.
    pub fn holds(&self, v: usize) -> bool {
        self.value[v] == 1
    }

    /// Whether the clauses can all hold, within `conflicts` conflicts.
    pub fn solve(&mut self, conflicts: u32) -> Answer {
        if self.broken {
            return Answer::Unsat;
        }
        let mut left = conflicts;
        loop {
            if let Some(c) = self.propagate() {
                if self.limits.is_empty() {
                    return Answer::Unsat;
                }
                if left == 0 {
                    return Answer::Unknown;
                }
                left -= 1;
                self.conflicts += 1;
                let (learned, back) = self.analyze(c);
                self.back_to(back);
                if learned.len() == 1 {
                    self.assign(learned[0], None);
                } else {
                    let i = self.clauses.len();
                    self.watches[slot(-learned[0])].push(i);
                    self.watches[slot(-learned[1])].push(i);
                    let first = learned[0];
                    self.clauses.push(&learned);
                    self.assign(first, Some(i));
                }
                continue;
            }
            // A decision: the most active unset variable, false first.
            let next = self.order.next(&self.value);
            let Some(v) = next else { return Answer::Sat };
            self.limits.push(self.trail.len());
            self.assign(-(v as Lit), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_formulas() {
        // (a | b) & (~a | b) & (a | ~b): a and b.
        let mut s = Solver::new(2);
        for c in [[1, 2], [-1, 2], [1, -2]] {
            s.add(&c);
        }
        assert_eq!(s.solve(100), Answer::Sat);
        // ... and (~a | ~b): none.
        let mut s = Solver::new(2);
        for c in [[1, 2], [-1, 2], [1, -2], [-1, -2]] {
            s.add(&c);
        }
        assert_eq!(s.solve(100), Answer::Unsat);
    }

    /// The pigeonhole formula, 4 pigeons in 3 holes: unsatisfiable, and needs learning.
    #[test]
    fn pigeons() {
        let (p, h) = (4, 3);
        let x = |i: usize, j: usize| (i * h + j + 1) as Lit;
        let mut s = Solver::new(p * h);
        for i in 0..p {
            s.add(&(0..h).map(|j| x(i, j)).collect::<Vec<_>>());
        }
        for j in 0..h {
            for a in 0..p {
                for b in a + 1..p {
                    s.add(&[-x(a, j), -x(b, j)]);
                }
            }
        }
        assert_eq!(s.solve(10_000), Answer::Unsat);
    }
}
