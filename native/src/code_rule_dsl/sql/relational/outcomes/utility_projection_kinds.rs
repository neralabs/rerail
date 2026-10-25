//! Compile-time kind strata for potential boundary demands. These facts never
//! substitute for native entry cost or completed memo events.
use super::super::ir::Set;
use super::utility_projection_lower::{Expr, Id, Program};
use std::collections::BTreeSet;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
fn kinds(set: &Set, out: &mut BTreeSet<u16>) {
    match set {
        Set::Kinds(v) => out.extend(v),
        Set::Union(v) | Set::Intersect(v) => {
            for s in v {
                kinds(s, out)
            }
        }
        Set::Complement(s) => kinds(s, out),
        _ => {}
    }
}
fn atom(set: &Set, kind: Option<u16>) -> u8 {
    match set {
        Set::AllNodes => TRUE,
        Set::Empty => FALSE,
        Set::Kinds(v) => {
            if kind.is_some_and(|k| v.contains(&k)) {
                TRUE
            } else {
                FALSE
            }
        }
        Set::Complement(s) => flip(atom(s, kind)),
        Set::Union(v) => v.iter().fold(FALSE, |a, s| or(a, atom(s, kind))),
        Set::Intersect(v) => v.iter().fold(TRUE, |a, s| and(a, atom(s, kind))),
        _ => FALSE | TRUE,
    }
}
fn flip(a: u8) -> u8 {
    ((a & FALSE) << 1) | ((a & TRUE) >> 1)
}
fn and(a: u8, b: u8) -> u8 {
    ((a | b) & FALSE) | ((a & b) & TRUE)
}
fn or(a: u8, b: u8) -> u8 {
    ((a & b) & FALSE) | ((a | b) & TRUE)
}
fn truth(p: &Program, id: Id, kind: Option<u16>) -> u8 {
    match &p.expressions[id] {
        Expr::Atom(s) => atom(s, kind),
        Expr::Not(c) => flip(truth(p, *c, kind)),
        Expr::All(v) => v.iter().fold(TRUE, |a, c| and(a, truth(p, *c, kind))),
        Expr::Any(v) => v.iter().fold(FALSE, |a, c| or(a, truth(p, *c, kind))),
        Expr::Call(f) => truth(p, p.functions[*f].body, kind),
        Expr::Search { filter, .. } if filter.as_ref().is_some_and(|f| f.kinds.is_empty()) => FALSE,
        Expr::Search { .. } | Expr::Recursive(_) => FALSE | TRUE,
    }
}
fn reach(p: &Program, id: Id, kind: Option<u16>, active: bool, out: &mut Vec<bool>) {
    match &p.expressions[id] {
        Expr::Atom(_) => {}
        Expr::Call(_) | Expr::Search { .. } | Expr::Recursive(_) => out.push(active),
        Expr::Not(c) => reach(p, *c, kind, active, out),
        Expr::All(v) | Expr::Any(v) => {
            let all = matches!(p.expressions[id], Expr::All(_));
            let mut live = active;
            for c in v {
                reach(p, *c, kind, live, out);
                live &= truth(p, *c, kind) & if all { TRUE } else { FALSE } != 0;
            }
        }
    }
}
/// One SQL metadata guard for each boundary occurrence in render order. Unknown
/// regex/text facts conservatively retain both outcomes; only proof removes work.
pub(super) fn boundary_guards(p: &Program, entry: Id, node: &str) -> Vec<String> {
    let mut palette = BTreeSet::new();
    for e in &p.expressions {
        if let Expr::Atom(s) = e {
            kinds(s, &mut palette);
        }
    }
    let palette = palette.into_iter().collect::<Vec<_>>();
    let mut classes = palette.iter().copied().map(Some).collect::<Vec<_>>();
    classes.push(None);
    let possible = classes
        .iter()
        .map(|k| {
            let mut v = vec![];
            reach(p, entry, *k, true, &mut v);
            v
        })
        .collect::<Vec<_>>();
    let count = possible.first().map_or(0, Vec::len);
    (0..count)
        .map(|site| {
            let accepted = palette
                .iter()
                .enumerate()
                .filter(|(i, _)| possible[*i][site])
                .map(|(_, k)| k.to_string())
                .collect::<Vec<_>>();
            let other = possible.last().unwrap()[site];
            if other {
                let excluded = palette
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !possible[*i][site])
                    .map(|(_, k)| k.to_string())
                    .collect::<Vec<_>>();
                if excluded.is_empty() {
                    "1".into()
                } else {
                    format!("{node}.kind NOT IN ({})", excluded.join(","))
                }
            } else if accepted.is_empty() {
                "0".into()
            } else {
                format!("{node}.kind IN ({})", accepted.join(","))
            }
        })
        .collect()
}
