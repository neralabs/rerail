//! The bounds of the values a component's join rules write into each of its relations' columns, known before any row
//! is written: a relation's column whose values all fit 32 bits unsigned is held 4 bytes a row (`Rows::narrowed`).
//!
//! A head column's values come from what binds its slot in the rule: a column of an input relation (its bounds,
//! `InRel::col_range`, a view's from its source), a column of the component's own relation (that relation's bounds,
//! to a fixed point, as a recursive head's rows come from its own), or a constant (its value). A value computed (a
//! `cat`, a split, a function's, a scalar's, a document's) is unbounded. The device packs its heads' sort keys by the
//! same bounds (`device::head_bounds`).

use super::{Key, Src, Step};
use super::{Body, Gate, JoinRule, ProbeRule, Rel, Term};

/// No bound: every value.
const ALL: [i64; 2] = [i64::MIN, i64::MAX];
/// No value yet: rows of no rule.
const NONE: [i64; 2] = [i64::MAX, i64::MIN];

fn join(a: [i64; 2], b: [i64; 2]) -> [i64; 2] {
    [a[0].min(b[0]), a[1].max(b[1])]
}

/// What a slot's value is, where the rule says: a column of an input relation or of the component's own, or a constant.
#[derive(Clone, Copy)]
enum From<'a> {
    In(&'a super::InRel, usize),
    Own(usize, usize),
    Const(i64),
}

/// Each slot of `j`: what bound it (`None`: a value computed, or no step of the rule's binds it as an atom or a `let`).
fn slots<'a>(j: &'a JoinRule) -> Vec<Option<From<'a>>> {
    let mut from: Vec<Option<From<'a>>> = (0..j.nslots).map(|_| None).collect();
    let set = |s: usize, f: Option<From<'a>>, from: &mut Vec<Option<From<'a>>>| {
        if s >= from.len() {
            from.resize_with(s + 1, || None);
        }
        from[s] = f;
    };
    for st in j.steps.iter() {
        match st {
            // An atom's binds are the slots first bound there (a slot bound before is one of its keys).
            Step::Atom { src, neg: false, binds, .. } => {
                for &(c, s) in binds {
                    let f = match src {
                        Src::In(rel, _) => From::In(j.bound(rel), c),
                        Src::Delta(r) | Src::Full(r) | Src::FullMulti(r, _) => From::Own(*r, c),
                    };
                    set(s, Some(f), &mut from);
                }
            }
            Step::Let { a, b } => {
                let f = match *b {
                    Key::Const(v) => Some(From::Const(v)),
                    Key::Slot(x) => from.get(x).copied().flatten(),
                };
                set(*a, f, &mut from);
            }
            _ => {}
        }
    }
    from
}

/// Each relation's columns' bounds over the rows the component's rules can write, or `None` where a writer of it is no
/// join rule (a SQL or unplanned probe, an edge rule, a gate, a request), it folds (a lattice), or it holds rows
/// already (its tail continues them).
pub(super) fn written(rels: &[Rel], probes: &[ProbeRule], adds: &[(usize, JoinRule)], edges: &[usize], gates: &[Gate], others: &[usize]) -> Vec<Option<Vec<[i64; 2]>>> {
    let mut b: Vec<Option<Vec<[i64; 2]>>> = rels.iter().map(|r| (r.n == 0 && r.lat.is_none() && r.from.is_none()).then(|| vec![NONE; r.k])).collect();
    let no = |r: usize, b: &mut Vec<Option<Vec<[i64; 2]>>>| {
        if let Some(x) = b.get_mut(r) {
            *x = None;
        }
    };
    for p in probes {
        if !matches!(p.body, Body::Join(_)) {
            no(p.head, &mut b);
        }
    }
    for &r in edges.iter().chain(others) {
        no(r, &mut b);
    }
    for g in gates {
        no(g.done, &mut b);
    }
    // Each join rule: its head, and per head column what bounds it.
    let joins: Vec<(usize, &JoinRule)> = probes.iter().filter_map(|p| match &p.body { Body::Join(j) => Some((p.head, &**j)), _ => None })
        .chain(probes.iter().filter_map(|p| p.whole.as_ref().map(|j| (p.head, j))))
        .chain(adds.iter().map(|(h, j)| (*h, j)))
        .collect();
    let heads: Vec<(usize, Vec<Option<From<'_>>>)> = joins.iter().map(|&(h, j)| {
        let from = slots(j);
        let cols = j.out.iter().map(|t| match *t {
            Term::Slot(s) => from.get(s).copied().flatten(),
            Term::Const(v) => Some(From::Const(v)),
            Term::Cat(..) => None,
        }).collect();
        (h, cols)
    }).collect();
    loop {
        let mut changed = false;
        for (h, cols) in &heads {
            if b[*h].is_none() {
                continue;
            }
            for (c, f) in cols.iter().enumerate() {
                let v = match f {
                    Some(From::Const(v)) => [*v, *v],
                    Some(From::In(r, col)) => r.col_range(*col).unwrap_or(ALL),
                    Some(From::Own(r, col)) => b.get(*r).and_then(|x| x.as_ref()).and_then(|x| x.get(*col).copied()).unwrap_or(ALL),
                    None => ALL,
                };
                let cur = &mut b[*h].as_mut().expect("bounded")[..];
                let Some(x) = cur.get_mut(c) else { continue };
                let n = join(*x, v);
                if n != *x {
                    *x = n;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    if crate::code_rule_dsl::logic::profile::on() {
        // Why a head column is not held 4 bytes a row: what bound it, by rule.
        for (h, cols) in &heads {
            for f in cols {
                let why = match (b[*h].as_ref(), f) {
                    (None, _) => "its relation: rows at open, a lattice, or a writer that is no join rule",
                    (_, None) => "a value computed",
                    (_, Some(From::Const(v))) if (0..=i64::from(u32::MAX)).contains(v) => "a constant within 32 bits",
                    (_, Some(From::Const(_))) => "a constant past 32 bits",
                    (_, Some(From::Own(..))) => "a column of the component's own",
                    (_, Some(From::In(r, c))) => match r.col_range(*c) {
                        None => "an input column with no bounds",
                        Some(x) if x[0] > x[1] || (x[0] >= 0 && x[1] <= i64::from(u32::MAX)) => "an input column within 32 bits",
                        Some(x) if x[0] < 0 => "an input column with negative values",
                        Some(x) if x[1] - x[0] <= i64::from(u32::MAX) => "an input column past 32 bits, its span within",
                        Some(_) => "an input column past 32 bits, its span too",
                    },
                };
                crate::code_rule_dsl::logic::profile::decision("head columns' bounds, by what bound them (rule by rule)", || why.to_owned());
            }
        }
    }
    b
}

/// The columns (a bit a column, of the first 64) whose bounds fit 32 bits unsigned: held 4 bytes a row.
pub(super) fn narrow_mask(bounds: &[[i64; 2]]) -> u64 {
    bounds.iter().take(64).enumerate().filter(|(_, b)| b[0] > b[1] || (b[0] >= 0 && b[1] <= i64::from(u32::MAX))).fold(0, |m, (c, _)| m | 1 << c)
}
