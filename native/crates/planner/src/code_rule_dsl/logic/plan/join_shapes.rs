//! The shapes the planner recognizes in the rules (the design "Registering an executor for a join shape",
//! 2026-11-26): structures of rules, recursive ones included, that a specialized executor runs as a whole.
//!
//! A shape here finds its matches over the program's strata; each match is a set of relations whose strata become one
//! component, which setfix then runs by its executor (`setfix::forms`). The list is `SHAPES`, tried in order; a
//! relation belongs to one match at most, and an earlier entry's match wins. Each shape has a name: its switch,
//! `RERAIL_SHAPE_<NAME>=off` (upper-cased, `_` for spaces; `RERAIL_SHAPES=off` turns all off), and the profile use it.
//! A shape reads the rules' structure only, never a relation's name.
use super::body;
use std::collections::BTreeSet;

/// What a shape is shown: the placed rules, the strata in dependency order and what each relation reads.
pub(crate) struct ShapeView<'a> {
    pub rules: &'a body::Placed,
    pub strata: &'a super::super::front::Components,
    pub deps: &'a super::super::front::Edges,
    /// Relations an earlier shape's match took.
    pub claimed: &'a BTreeSet<String>,
}

/// A match of a shape: the relations whose strata become one component, the first's stratum first.
pub(crate) struct Match {
    pub shape: &'static str,
    pub takes: Vec<String>,
}

/// A shape of rules the planner groups into one component for its executor.
pub(crate) trait Shape: Sync {
    fn name(&self) -> &'static str;
    /// Its matches among the strata no earlier shape claimed.
    fn find(&self, v: &ShapeView<'_>) -> Vec<Match>;
}

/// The shapes, in the order they are tried.
pub(crate) const SHAPES: &[&dyn Shape] = &[&ClimbEnds];

/// Tests: the shapes' names, in the order they are tried (the oracle cases' table checks each has its cases).
#[cfg(any(test, feature = "test-hooks"))]
pub fn shape_names() -> Vec<&'static str> {
    SHAPES.iter().map(|s| s.name()).collect()
}

/// Whether shape `name` is turned off for this run.
pub(crate) fn off(name: &str) -> bool {
    let var = format!("RERAIL_SHAPE_{}", name.to_ascii_uppercase().replace(' ', "_"));
    std::env::var("RERAIL_SHAPES").is_ok_and(|v| v == "off") || std::env::var(var).is_ok_and(|v| v == "off")
}

/// The strata with each shape's matches joined: a match's relations' strata become the stratum of its first relation,
/// in that stratum's place. `None`: no shape matched; the strata as they are.
pub(crate) fn regroup(p: &body::Placed, sccs: &super::super::front::Components, deps: &super::super::front::Edges) -> Option<super::super::front::Components> {
    let mut claimed = BTreeSet::new();
    let mut matches = Vec::new();
    for s in SHAPES.iter().filter(|s| !off(s.name())) {
        let found = s.find(&ShapeView { rules: p, strata: sccs, deps, claimed: &claimed });
        for m in found {
            if m.takes.iter().any(|r| claimed.contains(r)) {
                continue;
            }
            claimed.extend(m.takes.iter().cloned());
            matches.push(m);
        }
    }
    if matches.is_empty() {
        return None;
    }
    let at = |r: &str| sccs.iter().position(|c| c.iter().any(|x| x == r));
    let mut into: Vec<Option<usize>> = vec![None; sccs.len()];
    for m in &matches {
        let Some(j) = m.takes.first().and_then(|r| at(r)) else { continue };
        for r in &m.takes[1..] {
            if let Some(i) = at(r) && i != j {
                into[i] = Some(j);
            }
        }
    }
    let mut out: Vec<Vec<&str>> = sccs.iter().map(|c| c.iter().collect()).collect();
    for (i, to) in into.iter().enumerate() {
        if let Some(j) = *to {
            let moved: Vec<&str> = sccs[i].iter().collect();
            out[j].extend(moved);
        }
    }
    Some(super::super::front::Components::of_names(out.into_iter().enumerate().filter(|(i, _)| into[*i].is_none()).map(|(_, s)| s)))
}

/// The climb's ends (decision 0561): a stratum of one relation whose rules each read a keyed walk's rows once,
/// positively, and otherwise only relations made before the walk, joins the walk's stratum, where the climb answers
/// it at each row (setfix's keyed walk form). `RERAIL_CLIMB_ENDS=off` keeps the strata apart too.
struct ClimbEnds;

impl Shape for ClimbEnds {
    fn name(&self) -> &'static str {
        "climb ends"
    }
    fn find(&self, v: &ShapeView<'_>) -> Vec<Match> {
        if std::env::var("RERAIL_CLIMB_ENDS").is_ok_and(|x| x == "off") {
            return Vec::new();
        }
        super::climb_end_matches(v.rules, v.strata, v.deps).into_iter().filter(|m| !m.iter().any(|r| v.claimed.contains(r))).map(|takes| Match { shape: "climb ends", takes }).collect()
    }
}
