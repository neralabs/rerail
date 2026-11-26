//! Shapes inferred for derived relations (decision 0361, "Inferred for derived relations"): what a relation's rules
//! give its rows from the declared and inferred facts of the relations they read, as types are propagated.
//!
//! A relation's facts are its keys (each a mask of its columns: a probe binding them all returns at most one row) and
//! its columns' fanout caps (the rows a probe binding the column returns at most). What a rule gives its head:
//!
//! - **a key** where the head's columns at one atom's key determine every head variable: a projection keeping the
//!   key, a join along a key, a filter. A variable is determined by a constant, by an atom's key whose variables are
//!   all determined (it then determines the atom's every variable), or by an equality with a determined one;
//! - **a fanout** of a head column where it holds an atom's fanout column, and the atom's row determines every head
//!   variable: the head's rows a value are then at most the atom's.
//!
//! Where it stops (0361 step 3): a relation of several rules, or a recursive one, keeps its declared facts alone. A
//! key is not sound across a union (two branches may give one key two rows), and a recursion's facts would need the
//! base case and the step proved.
//!
//! The facts are derived, small and computed once a stratum's run (0353: a derived fact, written where it is computed:
//! into the stratum's plan, beside the declarations' sources that the planning operations read in place).

use crate::program::{Cmp, Kind, Rule, Term};
use std::collections::{BTreeMap, BTreeSet};

/// A relation's facts: its keys, each a mask of its columns, and its columns' fanout caps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub keys: Vec<u64>,
    pub caps: Vec<(usize, f64)>,
}

impl Facts {
    /// Both facts' sets, each fact once.
    pub fn and(&self, o: &Facts) -> Facts {
        let mut f = self.clone();
        for &k in &o.keys {
            if !f.keys.contains(&k) {
                f.keys.push(k);
            }
        }
        for &(c, x) in &o.caps {
            match f.caps.iter_mut().find(|e| e.0 == c) {
                Some(e) => e.1 = e.1.min(x),
                None => f.caps.push((c, x)),
            }
        }
        f
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.caps.is_empty()
    }

    /// The rows a probe with columns `mask` bound returns at most: one where they hold a key, a column's cap where
    /// they hold it; none known, infinite.
    pub fn cap(&self, mask: u64) -> f64 {
        if self.keys.iter().any(|&k| k != 0 && k & !mask == 0) {
            return 1.0;
        }
        self.caps.iter().filter(|&&(c, _)| c < 64 && mask >> c & 1 == 1).map(|&(_, x)| x).fold(f64::INFINITY, f64::min)
    }

    /// Its words: its keys, then its caps.
    pub fn words(&self, w: &mut Vec<i64>) {
        w.push(self.keys.len() as i64);
        w.extend(self.keys.iter().map(|&k| k as i64));
        w.push(self.caps.len() as i64);
        w.extend(self.caps.iter().flat_map(|&(c, x)| [c as i64, x.to_bits() as i64]));
    }

    pub fn of_words(next: &mut impl FnMut() -> i64) -> Facts {
        let keys = (0..next()).map(|_| next() as u64).collect();
        let caps = (0..next()).map(|_| (next() as usize, f64::from_bits(next() as u64))).collect();
        Facts { keys, caps }
    }
}

/// The relations' facts: declared, then inferred.
pub type Shapes = BTreeMap<i64, Facts>;

/// The declarations' relations (0361), each of three columns, its first the relation declared:
/// - `key(rel, k, col)`: column `col` is in key `k` of `rel`;
/// - `fanout(rel, col, class)`: a probe binding `col` returns at most one row (`one`), or `FEW` (`few`);
/// - `tree(rel, p, c)`: `p -> c` is a forest, so each `c` has one `p`: a key `c`;
/// - `order(rel, col, group)`: `col` numbers the rows of each `group` densely from 0: a key `(group, col)`.
pub const DECLS: [&str; 4] = ["key", "fanout", "tree", "order"];

/// The declared facts of the declarations' rows `rows` (in `DECLS`' order), the classes `one` and `few`.
pub fn declared(rows: &[BTreeSet<Vec<i64>>; 4], one: Option<i64>, few: Option<i64>) -> Shapes {
    let [keys, fanouts, trees, orders] = rows;
    let mut s: Shapes = BTreeMap::new();
    let mut by_key: BTreeMap<(i64, i64), u64> = BTreeMap::new();
    for r in keys.iter().filter(|r| r.len() >= 3 && (0..64).contains(&r[2])) {
        *by_key.entry((r[0], r[1])).or_default() |= 1 << r[2];
    }
    for ((rel, _), m) in by_key {
        s.entry(rel).or_default().keys.push(m);
    }
    for r in fanouts.iter().filter(|r| r.len() >= 3 && (0..64).contains(&r[1])) {
        let cap = if Some(r[2]) == one { 1.0 } else if Some(r[2]) == few { crate::body::FEW } else { continue };
        let f = s.entry(r[0]).or_default();
        *f = f.and(&Facts { keys: Vec::new(), caps: vec![(r[1] as usize, cap)] });
    }
    let col = |c: i64| (0..64).contains(&c).then(|| 1u64 << c);
    let implied = trees.iter().filter_map(|r| Some((r[0], col(r[2])?))).chain(orders.iter().filter_map(|r| Some((r[0], col(r[1])? | col(r[2])?))));
    for (rel, k) in implied {
        let f = s.entry(rel).or_default();
        *f = f.and(&Facts { keys: vec![k], caps: Vec::new() });
    }
    s
}

/// The variables `start` determines in `rule`'s body, by the facts `of` gives each relation it reads.
fn closure(rule: &Rule, start: BTreeSet<i64>, of: &impl Fn(i64) -> Option<Facts>) -> BTreeSet<i64> {
    let mut d = start;
    let fixed = |t: &Term, d: &BTreeSet<i64>| match t {
        Term::Var(v) => d.contains(v),
        Term::Const(_) => true,
        Term::Wild => false,
    };
    loop {
        let n = d.len();
        for s in &rule.steps {
            match s.kind {
                Kind::Atom => {
                    let Some(f) = of(s.rel) else { continue };
                    let mask = s.args.iter().enumerate().filter(|(i, t)| *i < 64 && fixed(t, &d)).fold(0u64, |m, (i, _)| m | 1 << i);
                    if f.keys.iter().any(|&k| k != 0 && k & !mask == 0) {
                        d.extend(s.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(*v) } else { None }));
                    }
                }
                Kind::Cmp(Cmp::Eq) if s.args.len() == 2 && (fixed(&s.args[0], &d) || fixed(&s.args[1], &d)) => {
                    d.extend(s.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(*v) } else { None }));
                }
                _ => {}
            }
        }
        if d.len() == n {
            return d;
        }
    }
}

/// Whether the variables `d` determine every one of `rule`'s head's.
fn determines(rule: &Rule, d: &BTreeSet<i64>) -> bool {
    rule.args.iter().all(|t| !matches!(t, Term::Var(v) if !d.contains(v)) && !matches!(t, Term::Wild))
}

/// The facts relation `head`'s rules `rules` give it (none: several rules, or a recursive one), its body's relations'
/// facts `of`.
pub fn infer(head: i64, rules: &[&Rule], recursive: bool, of: &impl Fn(i64) -> Option<Facts>) -> Facts {
    let mut out = Facts::default();
    let [rule] = rules else { return out };
    if recursive || rule.head != head || rule.args.len() > 64 {
        return out;
    }
    let at = |v: i64| rule.args.iter().position(|t| *t == Term::Var(v));
    for s in rule.steps.iter().filter(|s| s.kind == Kind::Atom) {
        let Some(f) = of(s.rel) else { continue };
        // A key: an atom's key, every one of its columns a head variable.
        for &k in &f.keys {
            let vars: Option<Vec<i64>> = (0..s.args.len().min(64)).filter(|i| k >> i & 1 == 1).map(|i| if let Term::Var(v) = s.args[i] { Some(v) } else { None }).collect();
            let Some(vars) = vars else { continue };
            let Some(cols) = vars.iter().map(|&v| at(v)).collect::<Option<Vec<usize>>>() else { continue };
            let mask = cols.iter().fold(0u64, |m, &c| m | 1 << c);
            if mask != 0 && !out.keys.contains(&mask) && determines(rule, &closure(rule, vars.into_iter().collect(), of)) {
                out.keys.push(mask);
            }
        }
        // A fanout: the atom's row determines the head's.
        let all: BTreeSet<i64> = s.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(*v) } else { None }).collect();
        if !f.caps.is_empty() && determines(rule, &closure(rule, all, of)) {
            for &(c, x) in &f.caps {
                if let Some(Term::Var(v)) = s.args.get(c)
                    && let Some(hc) = at(*v)
                {
                    out = out.and(&Facts { keys: Vec::new(), caps: vec![(hc, x)] });
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "shapes_tests.rs"]
mod tests;
