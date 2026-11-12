//! A small CDCL SAT solver, for the Bitlex compiler's proofs over a window of bytes
//! (`bitlex_prove::never_unrolled`).
//!
//! Clauses are lists of literals: a variable `v` (from 1) is `v` true or `-v` false. The search
//! is the usual one: two watched literals a clause, unit propagation, a conflict's first unique
//! implication point learned as a clause, a jump back to its second-highest level. Decisions take
//! the most active variable (activity bumped in each learned clause). A conflict budget bounds the
//! work: past it, the answer is "unknown", which a proof reads as "not proved" (sound).

/// A literal: a variable from 1, negative for its negation.
pub type Lit = i32;

/// `solve`'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Sat,
    Unsat,
    Unknown,
}

fn var(l: Lit) -> usize {
    l.unsigned_abs() as usize
}

/// A literal's watch slot: `2v` for `v`, `2v + 1` for `-v`.
fn slot(l: Lit) -> usize {
    2 * var(l) + usize::from(l < 0)
}

pub struct Solver {
    clauses: Vec<Vec<Lit>>,
    watches: Vec<Vec<usize>>,
    /// Each variable's value: 0 unset, 1 true, -1 false.
    value: Vec<i8>,
    level: Vec<u32>,
    reason: Vec<Option<usize>>,
    trail: Vec<Lit>,
    /// Where each decision level starts in the trail.
    limits: Vec<usize>,
    head: usize,
    activity: Vec<f64>,
    bump: f64,
    /// The clauses given empty, or two units against each other: unsatisfiable at once.
    broken: bool,
}

impl Solver {
    pub fn new(vars: usize) -> Self {
        Solver { clauses: Vec::new(), watches: vec![Vec::new(); 2 * vars + 2], value: vec![0; vars + 1], level: vec![0; vars + 1], reason: vec![None; vars + 1], trail: Vec::new(), limits: Vec::new(), head: 0, activity: vec![0.0; vars + 1], bump: 1.0, broken: false }
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
        let mut c: Vec<Lit> = lits.to_vec();
        c.sort_unstable();
        c.dedup();
        if c.windows(2).any(|w| w[0] == -w[1]) || c.iter().any(|&l| -l == c[0] && l != c[0]) {
            return;
        }
        if c.iter().any(|a| c.contains(&-a)) {
            return;
        }
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
                if self.clauses[ci][0] == falsified {
                    self.clauses[ci].swap(0, 1);
                }
                let other = self.clauses[ci][0];
                if self.lit_value(other) == 1 {
                    i += 1;
                    continue;
                }
                // A new literal to watch: not false.
                let mut moved = false;
                for k in 2..self.clauses[ci].len() {
                    let x = self.clauses[ci][k];
                    if self.lit_value(x) != -1 {
                        self.clauses[ci].swap(1, k);
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
        let mut seen = vec![false; self.value.len()];
        let mut learned: Vec<Lit> = vec![0];
        let mut count = 0;
        let mut at = self.trail.len();
        let mut clause = conflict;
        let mut p: Option<Lit> = None;
        loop {
            let lits: Vec<Lit> = self.clauses[clause].clone();
            for &q in &lits {
                if Some(q) == p {
                    continue;
                }
                let v = var(q);
                if seen[v] || self.level[v] == 0 {
                    continue;
                }
                seen[v] = true;
                self.activity[v] += self.bump;
                if self.level[v] == here {
                    count += 1;
                } else {
                    learned.push(q);
                }
            }
            // The latest seen literal on the trail.
            loop {
                at -= 1;
                if seen[var(self.trail[at])] {
                    break;
                }
            }
            let l = self.trail[at];
            seen[var(l)] = false;
            count -= 1;
            if count == 0 {
                learned[0] = -l;
                break;
            }
            p = Some(l);
            clause = self.reason[var(l)].expect("an implied literal");
        }
        self.bump *= 1.05;
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
        for &l in &self.trail[start..] {
            self.value[var(l)] = 0;
            self.reason[var(l)] = None;
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
                let (learned, back) = self.analyze(c);
                self.back_to(back);
                if learned.len() == 1 {
                    self.assign(learned[0], None);
                } else {
                    let i = self.clauses.len();
                    self.watches[slot(-learned[0])].push(i);
                    self.watches[slot(-learned[1])].push(i);
                    let first = learned[0];
                    self.clauses.push(learned);
                    self.assign(first, Some(i));
                }
                continue;
            }
            // A decision: the most active unset variable, false first.
            let next = (1..self.value.len()).filter(|&v| self.value[v] == 0).max_by(|&a, &b| self.activity[a].total_cmp(&self.activity[b]));
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
