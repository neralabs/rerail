//! The prover's SAT solver on the circuit (decision 0670): a window is gates, not clauses, kept as flat arrays in
//! a block, and the solver propagates each gate by its kind.
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

use crate::bitlex_grown::{FormulaBlock, Grown};
use crate::bitlex_sat::{Answer, Clauses, Lit, Order, normal};

/// A gate's kind (`Gates`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    And = 0,
    Xor = 1,
    One = 2,
}

/// Gates end to end: each a kind, an output (none for `One`) and its arguments, to its `ends` entry in `args`.
#[derive(Clone, Debug, Default)]
pub struct Gates {
    /// A gate a record: its kind and number of arguments (`kind | n << 2`), its output, and its arguments where it
    /// has at most two, else where they begin in `args` (decision 0670: one load a gate).
    rec: Grown<[Lit; 4]>,
    args: Grown<Lit>,
}


/// The kinds in a record's first word.
const KINDS: [Kind; 3] = [Kind::And, Kind::Xor, Kind::One];

impl Gates {
    /// None yet, built in `block` (`bitlex_grown`).
    pub fn in_block(block: &FormulaBlock) -> Gates {
        Gates { rec: Grown::in_block(block), args: Grown::in_block(block) }
    }

    /// Its records and wide arguments: their block and byte offsets there (`None` on the heap).
    pub fn placed(&self) -> Option<(&FormulaBlock, usize, usize)> {
        let (b, rec) = self.rec.placed()?;
        Some((b, rec, self.args.placed()?.1))
    }

    pub fn len(&self) -> usize {
        self.rec.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rec.is_empty()
    }

    fn push(&mut self, kind: Kind, out: Lit, args: &[Lit]) {
        let head = kind as Lit | (args.len() as Lit) << 2;
        self.rec.push(match *args {
            [] => [head, out, 0, 0],
            [a] => [head, out, a, 0],
            [a, b] => [head, out, a, b],
            _ => {
                let start = self.args.len() as Lit;
                self.args.extend_from_slice(args);
                [head, out, start, 0]
            }
        });
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
        self.rec.truncate(n);
        let last = self.rec.iter().rev().find(|r| r[0] >> 2 > 2);
        let end = last.map_or(0, |r| (r[2] + (r[0] >> 2)) as usize);
        self.args.truncate(end);
    }

    pub fn kind(&self, g: usize) -> Kind {
        KINDS[(self.rec[g][0] & 3) as usize]
    }

    pub fn out(&self, g: usize) -> Lit {
        self.rec[g][1]
    }

    /// Whether the gates from `base` on each have an output of their own.
    pub fn out_distinct(&self, base: usize) -> bool {
        let mut outs: Vec<usize> = (base..self.len()).map(|g| var(self.out(g))).collect();
        outs.sort_unstable();
        outs.windows(2).all(|w| w[0] != w[1])
    }

    pub fn args(&self, g: usize) -> &[Lit] {
        let r = &self.rec[g];
        let n = (r[0] >> 2) as usize;
        if n <= 2 { &r[2..2 + n] } else { &self.args[r[2] as usize..r[2] as usize + n] }
    }

    /// The formula as text, to solve or profile it apart from a compile (`RERAIL_CSAT_DUMP`, decision 0670): `v VARS
    /// BASE`, then a line a gate (`a OUT ARGS…`, `x OUT A B`, `o ARGS…`) and a line a plain clause (`c LITS…`).
    pub fn dump(&self, vars: usize, base: usize, clauses: &Clauses) -> String {
        use std::fmt::Write as _;
        let mut t = format!("v {vars} {base}\n");
        for g in 0..self.len() {
            let k = match self.kind(g) {
                Kind::And => "a",
                Kind::Xor => "x",
                Kind::One => "o",
            };
            let _ = write!(t, "{k}");
            if self.kind(g) != Kind::One {
                let _ = write!(t, " {}", self.out(g));
            }
            for a in self.args(g) {
                let _ = write!(t, " {a}");
            }
            t.push('\n');
        }
        for c in clauses.iter() {
            t.push('c');
            for l in c {
                let _ = write!(t, " {l}");
            }
            t.push('\n');
        }
        t
    }

    /// `dump`'s text read back: the gates, the variables, the window's gates and the plain clauses.
    pub fn undump(text: &str) -> (Gates, usize, usize, Clauses) {
        Gates::undump_into(text, Gates::default(), Clauses::default())
    }

    /// `undump`, the formula built in `gates` and `clauses` (empty: in a block, or on the heap).
    pub fn undump_into(text: &str, mut g: Gates, mut c: Clauses) -> (Gates, usize, usize, Clauses) {
        let (mut vars, mut base) = (0, 0);
        for line in text.lines() {
            let mut w = line.split(' ');
            let k = w.next().unwrap_or("");
            let n: Vec<Lit> = w.map(|x| x.parse().expect("a literal")).collect();
            match k {
                "v" => (vars, base) = (n[0] as usize, n[1] as usize),
                "a" => g.and(n[0], &n[1..]),
                "x" => g.xor(n[0], n[1], n[2]),
                "o" => g.one(&n),
                "c" => c.push(&n),
                _ => {}
            }
        }
        (g, vars, base, c)
    }

    /// Each gate's clauses, whose unit propagation is the gate's own (`bitlex_sat`'s input, the tests').
    pub fn cnf(&self, into: &mut Clauses) {
        let mut cs = Vec::new();
        for g in 0..self.len() {
            cs.clear();
            self.clauses_of(g, &mut cs);
            for c in &cs {
                into.push(c);
            }
        }
    }

    /// Gate `g`'s clauses (its CNF, as `cnf` gives them), each to `into`.
    pub fn clauses_of(&self, g: usize, into: &mut Vec<Vec<Lit>>) {
        let (o, a) = (self.out(g), self.args(g));
        match self.kind(g) {
            Kind::And => {
                for &x in a {
                    into.push(vec![-o, x]);
                }
                let mut c = vec![o];
                c.extend(a.iter().map(|&x| -x));
                into.push(c);
            }
            Kind::Xor => {
                let (x, y) = (a[0], a[1]);
                if var(x) == var(y) {
                    // Of one variable twice: the output fixed (`x ^ x` false, `x ^ -x` true), the one clause its
                    // four amount to.
                    into.push(vec![if x == y { -o } else { o }]);
                    return;
                }
                for c in [[-o, x, y], [-o, -x, -y], [o, -x, y], [o, x, -y]] {
                    into.push(c.to_vec());
                }
            }
            Kind::One => {
                into.push(a.to_vec());
                for (i, &x) in a.iter().enumerate() {
                    for &y in &a[i + 1..] {
                        into.push(vec![-x, -y]);
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
    /// The variables it covers.
    vars: usize,
    /// The least variable it lists; the `i`th past it has its gates from `starts[i]` to `starts[i + 1]`.
    lo: usize,
    starts: Vec<u32>,
    gates: Vec<u32>,
    /// A window's gates that imply with nothing set (`seed`): the CPU's `Solver` checks them first.
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
        let mut f = Fanout::of_range(gates, 0..base, vars);
        f.seeds = (0..base).filter(|&g| seed(gates, g)).map(|g| g as u32).collect();
        f
    }

    /// The fanout of the gates `range` over `vars` variables, a variable's gates in order: over the variables from
    /// the least its gates touch to the most (a solve's own, past a window's, are few of the window's).
    pub fn of_range(gates: &Gates, range: std::ops::Range<usize>, vars: usize) -> Fanout {
        let (mut lo, mut hi) = (usize::MAX, 0);
        for g in range.clone() {
            for v in vars_of(gates, g) {
                (lo, hi) = (lo.min(v), hi.max(v));
            }
        }
        if lo > hi {
            return Fanout { vars, lo: 0, starts: Vec::new(), gates: Vec::new(), seeds: Vec::new() };
        }
        let n = hi - lo + 1;
        let mut count = vec![0u32; n + 1];
        let mut last = vec![u32::MAX; n];
        for g in range.clone() {
            for v in vars_of(gates, g) {
                if last[v - lo] != g as u32 {
                    last[v - lo] = g as u32;
                    count[v - lo + 1] += 1;
                }
            }
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let mut fill = count.clone();
        let mut out = vec![0u32; count[n] as usize];
        last.fill(u32::MAX);
        for g in range.clone() {
            for v in vars_of(gates, g) {
                if last[v - lo] != g as u32 {
                    last[v - lo] = g as u32;
                    out[fill[v - lo] as usize] = g as u32;
                    fill[v - lo] += 1;
                }
            }
        }
        Fanout { vars, lo, starts: count, gates: out, seeds: Vec::new() }
    }

    /// The variables it covers.
    pub fn vars(&self) -> usize {
        self.vars
    }

    pub(crate) fn of_var(&self, v: usize) -> &[u32] {
        match v.checked_sub(self.lo) {
            Some(i) if i + 1 < self.starts.len() => &self.gates[self.starts[i] as usize..self.starts[i + 1] as usize],
            _ => &[],
        }
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

/// A window's solver at level 0 (`Solver::start`): what each of its solves resumes from.
#[derive(Clone, Default)]
pub struct Start {
    /// The values set, in order, each with its reason.
    set: Vec<(Lit, u32, u32)>,
    broken: bool,
    refuted: bool,
    conflict: Vec<Lit>,
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
    /// The variables at level 0 a conflict's analysis left out of its clause: what the clauses it learned rest on
    /// (`used`).
    rested: Vec<usize>,
    /// The last solve's contradiction (`used`): its conflict met at level 0, its clause in `conflict`.
    refuted: bool,
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
        (self.head, self.broken, self.refuted) = (0, false, false);
        self.rested.clear();
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

    /// After `Unsat`, the variables its contradiction rests on: those its conflict at level 0 reads, those the
    /// clauses it learned rest on, and back through each one's reason (a unit given, a decision's, stops it). `None`
    /// where the clauses given were contradictory as they were added. A caller may drop an assumption no variable
    /// of it is among, and must prove what is left again: the set is what the derivation read, not a proof.
    pub fn used(&mut self, gates: &Gates) -> Option<Vec<bool>> {
        if !self.refuted {
            return None;
        }
        let mut used = vec![false; self.value.len()];
        let mut work: Vec<usize> = self.conflict.iter().map(|&l| var(l)).chain(self.rested.iter().copied()).collect();
        while let Some(v) = work.pop() {
            if std::mem::replace(&mut used[v], true) || self.reason[v] == NONE || self.value[v] == 0 {
                continue;
            }
            self.explain(gates, v);
            work.extend(self.explained.iter().map(|&l| var(l)));
        }
        Some(used)
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
                if self.level[v] == 0 {
                    self.rested.push(v);
                    continue;
                }
                if self.seen[v] {
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
        self.link(gates, base);
        // The gates that imply with nothing set.
        for &g in &fan.seeds {
            if self.check(gates, g as usize) {
                self.refuted = true;
                return Answer::Unsat;
            }
        }
        for g in base..gates.len() {
            if seed(gates, g) && self.check(gates, g) {
                self.refuted = true;
                return Answer::Unsat;
            }
        }
        self.search(gates, fan, conflicts, None)
    }

    /// The solve's own gates (`base` on), linked from their variables' heads.
    fn link(&mut self, gates: &Gates, base: usize) {
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
    }

    /// A window's solver at level 0 (decision 0670): its plain clauses added, its gates' seeds checked, all
    /// propagated; made once, each solve of the window resumes from it (`resume`). It keeps the values set and their
    /// reasons alone: the clauses are the window's, read again as a solve resumes.
    pub fn start<'c>(&mut self, gates: &Gates, fan: &Fanout, vars: usize, clauses: impl Iterator<Item = &'c [Lit]>) -> Start {
        self.reset(vars);
        for c in clauses {
            self.add(c);
        }
        if !self.broken && (fan.seeds.iter().any(|&g| self.check(gates, g as usize)) || self.propagate(gates, fan)) {
            (self.broken, self.refuted) = (true, true);
        }
        Start {
            set: self.trail.iter().map(|&l| (l, self.reason[var(l)], self.why[var(l)])).collect(),
            broken: self.broken,
            refuted: self.refuted,
            conflict: self.conflict.clone(),
        }
    }

    /// A solve of a window from its `Start`, of `vars` variables (the window's, a solve's own past them): the
    /// window's plain clauses (those `start` read) numbered as there, each watched by two literals not false, or by
    /// its one true literal first (a reason's, the literal it implied).
    pub fn resume<'c>(&mut self, start: &Start, vars: usize, clauses: impl Iterator<Item = &'c [Lit]>) {
        self.reset(vars);
        for &(l, r, w) in &start.set {
            self.assign(l, r, w);
        }
        self.head = self.trail.len();
        (self.broken, self.refuted) = (start.broken, start.refuted);
        self.conflict.clone_from(&start.conflict);
        // Numbered as `start` did, broken or not: `used` reads their reasons.
        let mut c = std::mem::take(&mut self.scratch);
        for lits in clauses {
            if let Some(n) = normal(lits, &mut c)
                && n >= 2
            {
                c.truncate(n);
                let rank = |l: Lit| match self.lit_value(l) {
                    1 => 0,
                    0 => 1,
                    _ => 2,
                };
                c.sort_by_key(|&l| rank(l));
                let i = self.clauses.len() as u32;
                self.watches[slot(-c[0])].push(i);
                self.watches[slot(-c[1])].push(i);
                self.clauses.push(&c);
            }
        }
        self.scratch = c;
    }

    /// A clause added after `resume`, at level 0 among the values there: satisfied, it is left out; else watched by
    /// two literals not false, or, with one, it implies it (the clause its reason); with none, the solve is refuted.
    pub fn add_resumed(&mut self, lits: &[Lit]) {
        let mut c = std::mem::take(&mut self.scratch);
        if let Some(n) = normal(lits, &mut c) {
            c.truncate(n);
            if !c.iter().any(|&l| self.lit_value(l) == 1) {
                let mut k = 0;
                for i in 0..c.len() {
                    if self.lit_value(c[i]) != -1 {
                        c.swap(i, k);
                        k += 1;
                    }
                }
                match (c.len(), k) {
                    (0, _) | (1, 0) => self.broken = true,
                    (1, _) => self.assign(c[0], NONE, 0),
                    (_, 0) => {
                        self.conflict.clone_from(&c);
                        (self.broken, self.refuted) = (true, true);
                    }
                    (_, k) => {
                        let i = self.clauses.len() as u32;
                        self.watches[slot(-c[0])].push(i);
                        self.watches[slot(-c[1])].push(i);
                        self.clauses.push(&c);
                        if k == 1 {
                            self.assign(c[0], i, 0);
                        }
                    }
                }
            }
        }
        self.scratch = c;
    }

    /// The trail's length and the clauses' count: where a solve's own learning begins (`learned_without`).
    pub fn marks(&self) -> (usize, usize) {
        (self.trail.len(), self.clauses.len())
    }

    /// Whether `gates` and the clauses added hold with `assume` (decision 0670): a resumed window's solve, its own
    /// clauses each with `-assume`. `assume` is decided first, at level 1: a conflict there, with no decision past
    /// it, is unsatisfiable with no conflict spent, as one at level 0 is.
    pub fn solve_assuming(&mut self, gates: &Gates, fan: &Fanout, base: usize, assume: Lit, conflicts: u32) -> Answer {
        if self.broken {
            return Answer::Unsat;
        }
        self.link(gates, base);
        // Each of its own gates once: the values they read at level 0 were set before they were linked.
        for g in base..gates.len() {
            if self.check(gates, g) {
                self.refuted = true;
                return Answer::Unsat;
            }
        }
        self.search(gates, fan, conflicts, Some(assume))
    }

    /// After `solve_assuming`, what it learned that holds without `assume`, over the variables up to `vars`: each
    /// learned clause of at most `longest` literals without `-assume`, and each value learned at level 0, from
    /// `marks` on. Each rests on the clauses before `marks`, `gates` and the solve's own clauses (each with
    /// `-assume`): one without `-assume` used none of the last. The solve's own gates (`base` on) must each define a
    /// variable past `vars` and past its arguments: then any values of the first `vars` the rest allow, they allow
    /// too, and what was learned over those holds without them. Where they do not, nothing is taken.
    pub fn learned_without(&self, gates: &Gates, base: usize, assume: Lit, (trail, clauses): (usize, usize), vars: usize, longest: usize, mut take: impl FnMut(&[Lit])) {
        let defines = |g: usize| {
            let o = var(gates.out(g));
            gates.kind(g) != Kind::One && o > vars && gates.args(g).iter().all(|&a| var(a) < o)
        };
        if !(base..gates.len()).all(defines) || !gates.out_distinct(base) {
            return;
        }
        let within = |c: &[Lit]| c.iter().all(|&l| var(l) <= vars) && !c.contains(&-assume);
        for i in clauses..self.clauses.len() {
            let c = self.clauses.get(i);
            if c.len() <= longest && within(c) {
                take(c);
            }
        }
        let zero = self.limits.first().copied().unwrap_or(self.trail.len());
        for &l in self.trail.get(trail..zero).unwrap_or(&[]) {
            if self.reason[var(l)] == NONE && within(&[l]) {
                take(&[l]);
            }
        }
    }

    /// The search from the values as they stand: `assume`, if any, the first decision.
    fn search(&mut self, gates: &Gates, fan: &Fanout, conflicts: u32, assume: Option<Lit>) -> Answer {
        let mut left = conflicts;
        loop {
            if self.propagate(gates, fan) {
                if self.limits.is_empty() || (assume.is_some() && self.limits.len() == 1) {
                    self.refuted = true;
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
            // The assumption first: false at level 0, the solve is refuted by what implied it.
            if let Some(a) = assume
                && self.limits.is_empty()
            {
                match self.lit_value(a) {
                    0 => {
                        self.limits.push(self.trail.len());
                        self.assign(a, NONE, 0);
                        continue;
                    }
                    -1 => {
                        self.explain(gates, var(a));
                        self.conflict.clone_from(&self.explained);
                        self.refuted = true;
                        return Answer::Unsat;
                    }
                    _ => {}
                }
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

    /// An unsatisfiable formula stays so with every unit clause dropped whose variable its contradiction does not
    /// rest on (`Solver::used`): random circuits with a few units more, as a lemma's values assumed.
    #[test]
    fn a_contradiction_rests_on_what_it_used() {
        let mut rng = Rng(0x2545f4914f6cdd1d);
        let mut s = Solver::default();
        let (mut checked, mut dropped) = (0, 0);
        for i in 0..4000 {
            let (vars, gates, base, mut clauses) = random(&mut rng);
            for _ in 0..rng.below(8) {
                let l = ((rng.below(vars as u64) + 1) as Lit) * if rng.below(2) == 0 { 1 } else { -1 };
                clauses.push(&[l]);
            }
            let fan = Fanout::of(&gates, base, vars);
            s.reset(vars);
            for c in clauses.iter() {
                s.add(c);
            }
            if s.solve(&gates, &fan, base, u32::MAX) != Answer::Unsat {
                continue;
            }
            let Some(used) = s.used(&gates) else { continue };
            let mut kept = Clauses::default();
            for c in clauses.iter() {
                if c.len() == 1 && !used[var(c[0])] {
                    dropped += 1;
                } else {
                    kept.push(c);
                }
            }
            gates.cnf(&mut kept);
            let mut r = bitlex_sat::Solver::new(vars);
            r.add_all(&kept);
            assert_eq!(r.solve(u32::MAX), Answer::Unsat, "formula {i}: a unit it did not use was needed");
            checked += 1;
        }
        assert!(checked > 500 && dropped > 500, "{checked} checked, {dropped} units dropped");
    }

    /// A window's solves resumed from its `Start` (decision 0670): random circuits, the clauses over the window's
    /// gates its own, the rest and some units each solve's own, under an assumption. Each answer is `bitlex_sat`'s over
    /// all of them, a model satisfies them, a unit `used` leaves out is not needed, and what the solves learned
    /// for the window (`learned_without`, carried to its next solves) holds with the window's alone.
    #[test]
    fn a_resumed_window_answers_as_a_solve() {
        let mut rng = Rng(0x6a09e667f3bcc908);
        let mut s = Solver::default();
        let (mut sat, mut unsat, mut carried, mut dropped) = (0, 0, 0, 0);
        for i in 0..1500 {
            let (vars, gates, base, clauses) = random(&mut rng);
            let wvars = (0..base).filter(|&g| gates.kind(g) != Kind::One).map(|g| var(gates.out(g))).chain((0..base).flat_map(|g| gates.args(g).iter().map(|&a| var(a)))).max().unwrap_or(1).max(1);
            let (mut window, mut rest) = (Clauses::default(), Vec::new());
            for c in clauses.iter() {
                if c.iter().all(|&l| var(l) <= wvars) { window.push(c) } else { rest.push(c.to_vec()) }
            }
            let mut wg = gates.clone();
            wg.truncate(base);
            let mut wcnf = window.clone();
            wg.cnf(&mut wcnf);
            let fan = Fanout::of(&gates, base, wvars);
            let start = s.start(&gates, &fan, wvars, window.iter());
            let mut learned = Clauses::default();
            let sel = (vars + 1) as Lit;
            for k in 0..4 {
                let mut own: Vec<Vec<Lit>> = rest.clone();
                for _ in 0..rng.below(6) {
                    own.push(vec![((rng.below(vars as u64) + 1) as Lit) * if rng.below(2) == 0 { 1 } else { -1 }]);
                }
                s.resume(&start, vars + 1, window.iter());
                for c in learned.iter() {
                    s.add_resumed(c);
                }
                for c in &own {
                    let mut g = c.clone();
                    g.push(-sel);
                    s.add_resumed(&g);
                }
                let marks = s.marks();
                let got = s.solve_assuming(&gates, &fan, base, sel, u32::MAX);
                let mut all = window.clone();
                for c in &own {
                    all.push(c);
                }
                gates.cnf(&mut all);
                let mut r = bitlex_sat::Solver::new(vars);
                r.add_all(&all);
                assert_eq!(got, r.solve(u32::MAX), "formula {i}, solve {k}");
                match got {
                    Answer::Sat => {
                        sat += 1;
                        let holds = |l: Lit| s.holds(var(l)) == (l > 0);
                        assert!(satisfies(&gates, &all, &holds), "formula {i}, solve {k}: not a model");
                    }
                    Answer::Unsat => {
                        unsat += 1;
                        if let Some(used) = s.used(&gates) {
                            let mut kept = window.clone();
                            for c in &own {
                                if c.len() == 1 && !used[var(c[0])] {
                                    dropped += 1;
                                } else {
                                    kept.push(c);
                                }
                            }
                            gates.cnf(&mut kept);
                            let mut r = bitlex_sat::Solver::new(vars);
                            r.add_all(&kept);
                            assert_eq!(r.solve(u32::MAX), Answer::Unsat, "formula {i}, solve {k}: a unit it did not use was needed");
                        }
                    }
                    Answer::Unknown => unreachable!(),
                }
                s.learned_without(&gates, base, sel, marks, wvars, 64, |c| {
                    // Implied by the window's: with its negation added, they are unsatisfiable.
                    let mut w = wcnf.clone();
                    for &l in c {
                        w.push(&[-l]);
                    }
                    let mut r = bitlex_sat::Solver::new(vars);
                    r.add_all(&w);
                    assert_eq!(r.solve(u32::MAX), Answer::Unsat, "formula {i}, solve {k}: {c:?} learned, not the window's");
                    learned.push(c);
                    carried += 1;
                });
            }
        }
        assert!(sat > 300 && unsat > 300 && dropped > 100 && carried > 50, "{sat} satisfiable, {unsat} not, {dropped} units dropped, {carried} carried");
    }

    /// The formulas in `RERAIL_CSAT_DUMPS` (a directory of `Gates::dump`s, `RERAIL_CSAT_DUMP`), each solved
    /// `RERAIL_CSAT_TIMES` times: its answer, conflicts and time (decision 0670; a tool, so ignored).
    #[test]
    #[ignore]
    fn the_dumped_formulas() {
        let Some(dir) = std::env::var_os("RERAIL_CSAT_DUMPS") else { return };
        let times: u32 = std::env::var("RERAIL_CSAT_TIMES").ok().and_then(|t| t.parse().ok()).unwrap_or(1);
        let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the dumps").map(|e| e.expect("a dump").path()).collect();
        paths.sort();
        let mut s = Solver::default();
        // Each formula on the heap and in a block (as the prover's windows are), in turns.
        let block = crate::bitlex_grown::FormulaBlock::new();
        for p in paths {
            let text = std::fs::read_to_string(&p).expect("a dump");
            let made = [Gates::undump(&text), Gates::undump_into(&text, Gates::in_block(&block), Clauses::in_block(&block))];
            let fans: Vec<Fanout> = made.iter().map(|(g, vars, base, _)| Fanout::of(g, *base, *vars)).collect();
            let mut ms = [0.0f64; 2];
            let (mut answer, mut conflicts) = (Answer::Unknown, 0);
            for _ in 0..times {
                for (k, ((gates, vars, base, clauses), fan)) in made.iter().zip(&fans).enumerate() {
                    let t = std::time::Instant::now();
                    let c0 = s.conflicts;
                    s.reset(*vars);
                    for c in clauses.iter() {
                        s.add(c);
                    }
                    answer = s.solve(gates, fan, *base, 100_000);
                    ms[k] += t.elapsed().as_secs_f64() * 1e3 / f64::from(times);
                    conflicts = s.conflicts - c0;
                }
            }
            let (gates, vars, _, clauses) = &made[0];
            eprintln!("{}: {vars} vars, {} gates, {} clauses: {answer:?}, {conflicts} conflicts, {:.1} ms a solve on the heap, {:.1} ms in a block", p.display(), gates.len(), clauses.len(), ms[0], ms[1]);
        }
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
