//! The prover's SAT solver on the circuit (decision 0670): a window is gates, not clauses, kept as flat arrays a
//! UKL kernel can read as the CPU does, and the solver propagates each gate by its kind.
//!
//! A gate is a kind, an output literal and its arguments' literals, end to end (`Gates`):
//! - `And`: the output holds exactly when every argument does (none: it holds). An `or`, an equation (one argument)
//!   and a reader (its groups' `or`) are `And`s of negated literals.
//! - `Xor`: the output is its two arguments' `xor`.
//! - `One`: exactly one argument holds (a lag's byte: one of its groups, or before the stream).
//!
//! Each kind's rules are unit propagation of its clauses (`Gates::cnf`), so propagation alone decides what the
//! clauses' does. The search is `bitlex_sat`'s: plain and learned clauses watched by two literals, a conflict's first
//! unique implication point learned, a jump back to its second-highest level, decisions by activity (`Order`).
//!
//! A variable's gates are a list made once a window (`Fanout`); the gates a solve adds are linked from each
//! variable's head, set again each solve. Gates never move, so a solve loads only its values and its own clauses.
//! A reason is a gate and an argument (`why`): its clause is made when a conflict's analysis reads it, from the
//! values set before the literal it implied.

use crate::bitlex_sat::{Answer, Clauses, Lit, Order, normal};

/// A gate's kind (`Gates`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    And,
    Xor,
    One,
}

/// Gates end to end: each a kind, an output (none for `One`) and its arguments, to its `ends` entry in `args`.
#[derive(Clone, Debug, Default)]
pub struct Gates {
    kind: Vec<Kind>,
    out: Vec<Lit>,
    ends: Vec<u32>,
    args: Vec<Lit>,
}

impl Gates {
    pub fn len(&self) -> usize {
        self.kind.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_empty()
    }

    fn push(&mut self, kind: Kind, out: Lit, args: &[Lit]) {
        self.kind.push(kind);
        self.out.push(out);
        self.args.extend_from_slice(args);
        self.ends.push(self.args.len() as u32);
    }

    /// `out` holds exactly when each of `args` does.
    pub fn and(&mut self, out: Lit, args: &[Lit]) {
        self.push(Kind::And, out, args);
    }

    /// `out` is `a ^ b`.
    pub fn xor(&mut self, out: Lit, a: Lit, b: Lit) {
        self.push(Kind::Xor, out, &[a, b]);
    }

    /// Exactly one of `args` holds.
    pub fn one(&mut self, args: &[Lit]) {
        self.push(Kind::One, 0, args);
    }

    /// The first `n` gates kept.
    pub fn truncate(&mut self, n: usize) {
        if n >= self.len() {
            return;
        }
        self.kind.truncate(n);
        self.out.truncate(n);
        self.ends.truncate(n);
        self.args.truncate(self.ends.last().map_or(0, |&e| e as usize));
    }

    pub fn kind(&self, g: usize) -> Kind {
        self.kind[g]
    }

    pub fn out(&self, g: usize) -> Lit {
        self.out[g]
    }

    pub fn args(&self, g: usize) -> &[Lit] {
        let start = if g == 0 { 0 } else { self.ends[g - 1] as usize };
        &self.args[start..self.ends[g] as usize]
    }

    /// Each gate's clauses, whose unit propagation is the gate's own (`bitlex_sat`'s input; 0654's kernel's).
    pub fn cnf(&self, into: &mut Clauses) {
        for g in 0..self.len() {
            let (o, a) = (self.out(g), self.args(g));
            match self.kind(g) {
                Kind::And => {
                    for &x in a {
                        into.push(&[-o, x]);
                    }
                    let mut c = vec![o];
                    c.extend(a.iter().map(|&x| -x));
                    into.push(&c);
                }
                Kind::Xor => {
                    let (x, y) = (a[0], a[1]);
                    for c in [[-o, x, y], [-o, -x, -y], [o, -x, y], [o, x, -y]] {
                        into.push(&c);
                    }
                }
                Kind::One => {
                    into.push(a);
                    for (i, &x) in a.iter().enumerate() {
                        for &y in &a[i + 1..] {
                            into.push(&[-x, -y]);
                        }
                    }
                }
            }
        }
    }
}

/// Each variable's gates (`starts[v]` to `starts[v + 1]` in `gates`, each once), and the gates that can imply with
/// nothing set (an `And` of no arguments, a `One` of one, an `Xor` of one variable twice): made once a window, for
/// its first `base` gates.
#[derive(Clone, Debug, Default)]
pub struct Fanout {
    starts: Vec<u32>,
    gates: Vec<u32>,
    seeds: Vec<u32>,
}

fn var(l: Lit) -> usize {
    l.unsigned_abs() as usize
}

/// The variables a gate reads and writes.
fn vars_of(gates: &Gates, g: usize) -> impl Iterator<Item = usize> + '_ {
    let o = gates.out(g);
    (o != 0).then_some(var(o)).into_iter().chain(gates.args(g).iter().map(|&x| var(x)))
}

/// Whether a gate can imply with nothing set.
fn seed(gates: &Gates, g: usize) -> bool {
    match gates.kind(g) {
        Kind::And => gates.args(g).is_empty(),
        Kind::One => gates.args(g).len() <= 1,
        Kind::Xor => var(gates.args(g)[0]) == var(gates.args(g)[1]),
    }
}

impl Fanout {
    /// The fanout of `gates`' first `base` gates over `vars` variables.
    pub fn of(gates: &Gates, base: usize, vars: usize) -> Fanout {
        let mut count = vec![0u32; vars + 2];
        let mut last = vec![u32::MAX; vars + 1];
        for g in 0..base {
            for v in vars_of(gates, g) {
                if last[v] != g as u32 {
                    last[v] = g as u32;
                    count[v + 1] += 1;
                }
            }
        }
        for v in 0..=vars {
            count[v + 1] += count[v];
        }
        let starts = count.clone();
        let mut fill = count;
        let mut out = vec![0u32; starts[vars + 1] as usize];
        last.fill(u32::MAX);
        for g in 0..base {
            for v in vars_of(gates, g) {
                if last[v] != g as u32 {
                    last[v] = g as u32;
                    out[fill[v] as usize] = g as u32;
                    fill[v] += 1;
                }
            }
        }
        let seeds = (0..base).filter(|&g| seed(gates, g)).map(|g| g as u32).collect();
        Fanout { starts, gates: out, seeds }
    }

    fn of_var(&self, v: usize) -> &[u32] {
        if v + 1 >= self.starts.len() {
            return &[];
        }
        &self.gates[self.starts[v] as usize..self.starts[v + 1] as usize]
    }
}

/// No reason (a decision, or a unit given), and no link.
const NONE: u32 = u32::MAX;
/// A reason that is a gate: its number with this bit (else a clause's number).
const GATE: u32 = 1 << 31;
/// An `Xor` of one variable twice: its output implied by nothing (`why`).
const FIXED: u32 = 3;

/// A literal's watch slot: `2v` for `v`, `2v + 1` for `-v`.
fn slot(l: Lit) -> usize {
    2 * var(l) + usize::from(l < 0)
}

/// The solver, its memory kept from solve to solve.
#[derive(Default)]
pub struct Solver {
    /// The plain clauses given and the learned ones.
    clauses: Clauses,
    scratch: Vec<Lit>,
    watches: Vec<Vec<u32>>,
    /// Each variable's value: 0 unset, 1 true, -1 false.
    value: Vec<i8>,
    level: Vec<u32>,
    /// A clause's number, a gate's with `GATE`, or `NONE`; and the gate's argument it names.
    reason: Vec<u32>,
    why: Vec<u32>,
    trail: Vec<Lit>,
    limits: Vec<usize>,
    head: usize,
    order: Order,
    seen: Vec<bool>,
    /// The solve's own gates by variable: a list from each variable's head through `next`.
    head_of: Vec<u32>,
    next: Vec<u32>,
    gate_of: Vec<u32>,
    /// The conflict's clause, all its literals false; a reason's clause as analysis reads it.
    conflict: Vec<Lit>,
    explained: Vec<Lit>,
    broken: bool,
    /// The conflicts its solves met, in all (a caller's work budget reads it).
    pub conflicts: u64,
}

impl Solver {
    /// No clauses, of `vars` variables.
    pub fn reset(&mut self, vars: usize) {
        self.clauses.truncate(0);
        for w in &mut self.watches {
            w.clear();
        }
        self.watches.resize_with(2 * vars + 2, Vec::new);
        self.value.clear();
        self.value.resize(vars + 1, 0);
        self.level.clear();
        self.level.resize(vars + 1, 0);
        self.reason.clear();
        self.reason.resize(vars + 1, NONE);
        self.why.clear();
        self.why.resize(vars + 1, 0);
        self.seen.clear();
        self.seen.resize(vars + 1, false);
        self.head_of.clear();
        self.head_of.resize(vars + 1, NONE);
        self.next.clear();
        self.gate_of.clear();
        self.trail.clear();
        self.limits.clear();
        (self.head, self.broken) = (0, false);
        self.order.reset(vars);
    }

    fn lit_value(&self, l: Lit) -> i8 {
        let v = self.value[var(l)];
        if l < 0 { -v } else { v }
    }

    fn assign(&mut self, l: Lit, reason: u32, why: u32) {
        let v = var(l);
        self.value[v] = if l > 0 { 1 } else { -1 };
        self.level[v] = self.limits.len() as u32;
        self.reason[v] = reason;
        self.why[v] = why;
        self.trail.push(l);
    }

    /// A plain clause, before the search: duplicates dropped, a tautology ignored.
    pub fn add(&mut self, lits: &[Lit]) {
        let mut c = std::mem::take(&mut self.scratch);
        if let Some(n) = normal(lits, &mut c) {
            match n {
                0 => self.broken = true,
                1 => match self.lit_value(c[0]) {
                    1 => {}
                    -1 => self.broken = true,
                    _ => self.assign(c[0], NONE, 0),
                },
                _ => {
                    let i = self.clauses.len() as u32;
                    self.watches[slot(-c[0])].push(i);
                    self.watches[slot(-c[1])].push(i);
                    self.clauses.push(&c[..n]);
                }
            }
        }
        self.scratch = c;
    }

    /// A variable's value after `Sat`.
    pub fn holds(&self, v: usize) -> bool {
        self.value[v] == 1
    }

    /// The conflict's clause: these literals, all false.
    fn conflict_of(&mut self, lits: impl IntoIterator<Item = Lit>) -> bool {
        self.conflict.clear();
        self.conflict.extend(lits);
        true
    }

    /// Gate `g` checked against the values: what it implies is assigned; a conflict is `true`, its clause in
    /// `conflict`.
    fn check(&mut self, gates: &Gates, g: usize) -> bool {
        let r = GATE | g as u32;
        let (o, args) = (gates.out(g), gates.args(g));
        match gates.kind(g) {
            Kind::And => {
                let vo = self.lit_value(o);
                let (mut unset, mut unset_at) = (0, 0);
                for (i, &a) in args.iter().enumerate() {
                    match self.lit_value(a) {
                        -1 => {
                            match vo {
                                1 => return self.conflict_of([-o, a]),
                                0 => self.assign(-o, r, i as u32),
                                _ => {}
                            }
                            return false;
                        }
                        // Counted by literal: a literal twice is one, as in its clause.
                        0 if unset == 0 => (unset, unset_at) = (1, i),
                        0 if a != args[unset_at] => unset += 1,
                        _ => {}
                    }
                }
                match vo {
                    1 => {
                        for (i, &a) in args.iter().enumerate() {
                            match self.lit_value(a) {
                                0 => self.assign(a, r, i as u32),
                                -1 => return self.conflict_of([-o, a]),
                                _ => {}
                            }
                        }
                    }
                    0 if unset == 0 => self.assign(o, r, 0),
                    -1 if unset == 0 => return self.conflict_of(std::iter::once(o).chain(args.iter().map(|&a| -a))),
                    -1 if unset == 1 => self.assign(-args[unset_at], r, unset_at as u32),
                    _ => {}
                }
                false
            }
            Kind::Xor => {
                // Of one variable twice: the output is fixed (`FIXED`), as its clauses' propagation finds.
                if var(args[0]) == var(args[1]) {
                    let want = if args[0] == args[1] { -o } else { o };
                    match self.lit_value(want) {
                        0 => self.assign(want, r, FIXED),
                        -1 => return self.conflict_of([want]),
                        _ => {}
                    }
                    return false;
                }
                let p = [o, args[0], args[1]];
                let vals = p.map(|x| self.lit_value(x));
                let (mut unset, mut k) = (0, 0);
                for (j, &x) in vals.iter().enumerate() {
                    if x == 0 {
                        (unset, k) = (unset + 1, j);
                    }
                }
                let t = |j: usize| vals[j] == 1;
                match unset {
                    0 if t(0) != (t(1) ^ t(2)) => return self.conflict_of(p.map(|x| if self.lit_value(x) == 1 { -x } else { x })),
                    1 => {
                        let holds = match k {
                            0 => t(1) ^ t(2),
                            1 => t(0) ^ t(2),
                            _ => t(0) ^ t(1),
                        };
                        self.assign(if holds { p[k] } else { -p[k] }, r, k as u32);
                    }
                    _ => {}
                }
                false
            }
            Kind::One => {
                let (mut held, mut unset, mut unset_at): (Option<usize>, u32, usize) = (None, 0, 0);
                for (i, &a) in args.iter().enumerate() {
                    match self.lit_value(a) {
                        1 => match held {
                            Some(h) => return self.conflict_of([-args[h], -a]),
                            None => held = Some(i),
                        },
                        0 => (unset, unset_at) = (unset + 1, i),
                        _ => {}
                    }
                }
                match held {
                    Some(h) => {
                        for (i, &a) in args.iter().enumerate() {
                            match self.lit_value(a) {
                                0 => self.assign(-a, r, h as u32),
                                1 if i != h => return self.conflict_of([-args[h], -a]),
                                _ => {}
                            }
                        }
                    }
                    None if unset == 0 => return self.conflict_of(args.iter().copied()),
                    None if unset == 1 => self.assign(args[unset_at], r, unset_at as u32),
                    None => {}
                }
                false
            }
        }
    }

    /// The clause that implied `v`'s value, into `explained`: its literal first, every other false before it.
    fn explain(&mut self, gates: &Gates, v: usize) {
        let l = if self.value[v] == 1 { v as Lit } else { -(v as Lit) };
        let (r, w) = (self.reason[v], self.why[v] as usize);
        let mut e = std::mem::take(&mut self.explained);
        e.clear();
        if r & GATE == 0 {
            e.extend_from_slice(self.clauses.get(r as usize));
        } else {
            let g = (r & !GATE) as usize;
            let (o, args) = (gates.out(g), gates.args(g));
            match gates.kind(g) {
                Kind::And => {
                    if o != 0 && var(o) == v {
                        if l == o {
                            e.push(o);
                            e.extend(args.iter().map(|&a| -a));
                        } else {
                            e.extend([-o, args[w]]);
                        }
                    } else if l == args[w] {
                        e.extend([l, -o]);
                    } else {
                        e.extend([l, o]);
                        e.extend(args.iter().enumerate().filter(|&(k, _)| k != w).map(|(_, &a)| -a));
                    }
                }
                Kind::Xor if w == FIXED as usize => e.push(l),
                Kind::Xor => {
                    let p = [o, args[0], args[1]];
                    e.push(l);
                    e.extend((0..3).filter(|&k| k != w).map(|k| if self.lit_value(p[k]) == 1 { -p[k] } else { p[k] }));
                }
                Kind::One => {
                    if l == args[w] {
                        e.push(l);
                        e.extend(args.iter().enumerate().filter(|&(k, _)| k != w).map(|(_, &a)| a));
                    } else {
                        e.extend([l, -args[w]]);
                    }
                }
            }
        }
        self.explained = e;
    }

    /// Propagation from the trail's head, of the plain clauses and the gates: a conflict, its clause in `conflict`.
    fn propagate(&mut self, gates: &Gates, fan: &Fanout) -> bool {
        while self.head < self.trail.len() {
            let l = self.trail[self.head];
            self.head += 1;
            // The clauses watching `-l`, stored at `slot(l)`: those whose watched literal `l` made false.
            let mut ws = std::mem::take(&mut self.watches[slot(l)]);
            let mut i = 0;
            while i < ws.len() {
                let ci = ws[i] as usize;
                let falsified = -l;
                if self.clauses.get(ci)[0] == falsified {
                    self.clauses.get_mut(ci).swap(0, 1);
                }
                let other = self.clauses.get(ci)[0];
                if self.lit_value(other) == 1 {
                    i += 1;
                    continue;
                }
                let mut moved = false;
                for k in 2..self.clauses.get(ci).len() {
                    let x = self.clauses.get(ci)[k];
                    if self.lit_value(x) != -1 {
                        self.clauses.get_mut(ci).swap(1, k);
                        self.watches[slot(-x)].push(ci as u32);
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
                    let c = self.clauses.get(ci).to_vec();
                    return self.conflict_of(c);
                }
                self.assign(other, ci as u32, 0);
                i += 1;
            }
            self.watches[slot(l)] = ws;
            // The variable's gates: the window's, then the solve's.
            let v = var(l);
            for &g in fan.of_var(v) {
                if self.check(gates, g as usize) {
                    return true;
                }
            }
            let mut at = self.head_of[v];
            while at != NONE {
                let g = self.gate_of[at as usize] as usize;
                at = self.next[at as usize];
                if self.check(gates, g) {
                    return true;
                }
            }
        }
        false
    }

    /// The conflict's first unique implication point: the learned clause (its asserting literal first) and the
    /// level to go back to.
    fn analyze(&mut self, gates: &Gates) -> (Vec<Lit>, u32) {
        let here = self.limits.len() as u32;
        let mut learned: Vec<Lit> = vec![0];
        let mut count = 0;
        let mut at = self.trail.len();
        let mut clause = std::mem::take(&mut self.conflict);
        let mut p: Option<Lit> = None;
        loop {
            for &q in &clause {
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
            self.explain(gates, var(l));
            std::mem::swap(&mut clause, &mut self.explained);
        }
        self.conflict = clause;
        for &q in &learned[1..] {
            self.seen[var(q)] = false;
        }
        self.order.decay();
        let back = learned[1..].iter().map(|&q| self.level[var(q)]).max().unwrap_or(0);
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
            self.reason[v] = NONE;
            self.order.insert(v);
        }
        self.trail.truncate(start);
        self.limits.truncate(level as usize);
        self.head = start;
    }

    /// Whether the plain clauses added and `gates` can all hold, within `conflicts` conflicts. `fan` is the fanout
    /// of `gates`' first `base` (`Fanout::of`); the gates after them are the solve's own.
    pub fn solve(&mut self, gates: &Gates, fan: &Fanout, base: usize, conflicts: u32) -> Answer {
        if self.broken {
            return Answer::Unsat;
        }
        // The solve's own gates, linked from their variables' heads.
        for g in base..gates.len() {
            let mut last = usize::MAX;
            for v in vars_of(gates, g) {
                if v == last {
                    continue;
                }
                last = v;
                self.gate_of.push(g as u32);
                self.next.push(self.head_of[v]);
                self.head_of[v] = (self.next.len() - 1) as u32;
            }
        }
        // The gates that imply with nothing set.
        for &g in &fan.seeds {
            if self.check(gates, g as usize) {
                return Answer::Unsat;
            }
        }
        for g in base..gates.len() {
            if seed(gates, g) && self.check(gates, g) {
                return Answer::Unsat;
            }
        }
        let mut left = conflicts;
        loop {
            if self.propagate(gates, fan) {
                if self.limits.is_empty() {
                    return Answer::Unsat;
                }
                if left == 0 {
                    return Answer::Unknown;
                }
                left -= 1;
                self.conflicts += 1;
                let (learned, back) = self.analyze(gates);
                self.back_to(back);
                if learned.len() == 1 {
                    self.assign(learned[0], NONE, 0);
                } else {
                    let i = self.clauses.len() as u32;
                    self.watches[slot(-learned[0])].push(i);
                    self.watches[slot(-learned[1])].push(i);
                    let first = learned[0];
                    self.clauses.push(&learned);
                    self.assign(first, i, 0);
                }
                continue;
            }
            // A decision: the most active unset variable, false first.
            let Some(v) = self.order.next(&self.value) else { return Answer::Sat };
            self.limits.push(self.trail.len());
            self.assign(-(v as Lit), NONE, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitlex_sat;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Whether a full assignment (`holds`) satisfies the gates and the clauses.
    fn satisfies(gates: &Gates, clauses: &Clauses, holds: &dyn Fn(Lit) -> bool) -> bool {
        let gates_hold = (0..gates.len()).all(|g| {
            let a = gates.args(g);
            match gates.kind(g) {
                Kind::And => holds(gates.out(g)) == a.iter().all(|&x| holds(x)),
                Kind::Xor => holds(gates.out(g)) == (holds(a[0]) ^ holds(a[1])),
                Kind::One => a.iter().filter(|&&x| holds(x)).count() == 1,
            }
        });
        gates_hold && clauses.iter().all(|c| c.iter().any(|&x| holds(x)))
    }

    /// A random circuit as a window is: inputs in one-hot groups, gates over earlier variables (some arguments
    /// repeated or negated), a few plain clauses, a root asserted. Some gates are the solve's own (past `base`).
    fn random(rng: &mut Rng) -> (usize, Gates, usize, Clauses) {
        let mut gates = Gates::default();
        let mut clauses = Clauses::default();
        let mut vars = 1usize;
        clauses.push(&[1]);
        let lit = |rng: &mut Rng, vars: usize| ((rng.below(vars as u64) + 1) as Lit) * if rng.below(2) == 0 { 1 } else { -1 };
        for _ in 0..1 + rng.below(4) {
            let n = 1 + rng.below(5) as usize;
            let vs: Vec<Lit> = (0..n).map(|k| (vars + 1 + k) as Lit).collect();
            vars += n;
            gates.one(&vs);
        }
        let n = 5 + rng.below(40) as usize;
        let base = n * 3 / 4;
        for _ in 0..n {
            let x = (vars + 1) as Lit;
            match rng.below(3) {
                0 => {
                    let a: Vec<Lit> = (0..rng.below(4)).map(|_| lit(rng, vars)).collect();
                    gates.and(if rng.below(2) == 0 { x } else { -x }, &a);
                }
                1 => {
                    let a = lit(rng, vars);
                    let b = match rng.below(8) {
                        0 => a,
                        1 => -a,
                        _ => lit(rng, vars),
                    };
                    gates.xor(x, a, b);
                }
                _ => {
                    let a = lit(rng, vars);
                    gates.and(x, &[a]);
                }
            }
            vars += 1;
        }
        for _ in 0..rng.below(4) {
            let c: Vec<Lit> = (0..1 + rng.below(3)).map(|_| lit(rng, vars)).collect();
            clauses.push(&c);
        }
        clauses.push(&[lit(rng, vars)]);
        (vars, gates, base, clauses)
    }

    /// Random circuits: the circuit's solver and `bitlex_sat` over the circuit's clauses agree on every one, each
    /// model satisfies the gates and clauses, and a conflict at level 0 is the clauses' propagation's.
    #[test]
    fn the_circuits_answers_are_the_clauses() {
        let mut rng = Rng(0x9e3779b97f4a7c15);
        let mut s = Solver::default();
        let (mut sat, mut unsat) = (0, 0);
        for i in 0..4000 {
            let (vars, gates, base, clauses) = random(&mut rng);
            let fan = Fanout::of(&gates, base, vars);
            s.reset(vars);
            for c in clauses.iter() {
                s.add(c);
            }
            let before = s.conflicts;
            let got = s.solve(&gates, &fan, base, u32::MAX);
            let mut all = clauses.clone();
            gates.cnf(&mut all);
            let mut r = bitlex_sat::Solver::new(vars);
            r.add_all(&all);
            let want = r.solve(u32::MAX);
            assert_eq!(got, want, "formula {i}");
            match got {
                Answer::Sat => {
                    sat += 1;
                    let holds = |l: Lit| s.holds(var(l)) == (l > 0);
                    assert!(satisfies(&gates, &clauses, &holds), "formula {i}: not a model");
                }
                Answer::Unsat => {
                    unsat += 1;
                    // Level 0: the clauses' propagation finds it with no conflict, and so does the circuit's.
                    let mut p = bitlex_sat::Solver::new(vars);
                    p.add_all(&all);
                    if p.solve(0) == Answer::Unsat {
                        assert_eq!(s.conflicts, before, "formula {i}: the clauses' propagation decides it, the gates' did not");
                    }
                }
                Answer::Unknown => unreachable!(),
            }
        }
        assert!(sat > 500 && unsat > 500, "{sat} satisfiable, {unsat} not");
    }

    /// Hard formulas need conflicts, and their learned clauses reason through the gates: pigeons in holes, as
    /// `One`s per pigeon and `And`s saying two share a hole.
    #[test]
    fn pigeons() {
        for n in 2..7usize {
            let holes = n - 1;
            let p = |i: usize, h: usize| (2 + i * holes + h) as Lit;
            let mut vars = 1 + n * holes;
            let mut gates = Gates::default();
            let mut clauses = Clauses::default();
            clauses.push(&[1]);
            for i in 0..n {
                let row: Vec<Lit> = (0..holes).map(|h| p(i, h)).collect();
                gates.one(&row);
            }
            for h in 0..holes {
                for i in 0..n {
                    for j in i + 1..n {
                        vars += 1;
                        gates.and(vars as Lit, &[p(i, h), p(j, h)]);
                        clauses.push(&[-(vars as Lit)]);
                    }
                }
            }
            let fan = Fanout::of(&gates, gates.len(), vars);
            let mut s = Solver::default();
            s.reset(vars);
            for c in clauses.iter() {
                s.add(c);
            }
            assert_eq!(s.solve(&gates, &fan, gates.len(), u32::MAX), Answer::Unsat, "{n} pigeons");
        }
    }
}
