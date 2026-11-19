//! Aggregate demand's analyses (restricting private aggregate helpers to the keys their consumers ask): what may
//! reach what (`Reach`, `depends`), and which helpers a program makes (`helper`; `whole::RetypeHelpers` types their columns). The pass runs on
//! the version, after relation and world lowering, so its dependency checks see the complete graph
//! (`lowering_tail::AggregateDemand`).
use crate::code_rule_dsl::logic::front;
use std::collections::{BTreeMap, BTreeSet};

/// The relations that reach each target through `deps`, found once a target (a reverse walk) and dropped when the
/// edges change. `guards` asks it of every literal of every consumer, so a walk each was quadratic in the program.
/// The reverse edges are made once too, for every target's walk until the edges change, not again for each target
/// (O-lower-aggregate-demand).
#[derive(Default)]
pub struct Reach {
    to: std::cell::RefCell<BTreeMap<String, std::rc::Rc<BTreeSet<String>>>>,
    rev: std::cell::RefCell<Option<BTreeMap<String, Vec<String>>>>,
}

impl Reach {
    pub fn reaches(&self, deps: &BTreeMap<String, BTreeSet<String>>, start: &str, target: &str) -> bool {
        if start == target {
            return true;
        }
        let set = self.to.borrow().get(target).cloned();
        let set = set.unwrap_or_else(|| {
            let mut rev = self.rev.borrow_mut();
            let rev = rev.get_or_insert_with(|| {
                let mut rev: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for (from, tos) in deps {
                    for t in tos {
                        rev.entry(t.clone()).or_default().push(from.clone());
                    }
                }
                rev
            });
            let mut seen = BTreeSet::new();
            let mut todo = vec![target];
            while let Some(at) = todo.pop() {
                for f in rev.get(at).into_iter().flatten() {
                    if seen.insert(f.clone()) {
                        todo.push(f.as_str());
                    }
                }
            }
            let set = std::rc::Rc::new(seen);
            self.to.borrow_mut().insert(target.to_owned(), set.clone());
            set
        });
        set.contains(start)
    }

    /// `name` now reads `demand`, which reads `guards`. A path the new edges make ends with `demand` -> a guard, then
    /// old edges from the guard to the target, so only a target a guard is or reaches (or `demand`) gains relations
    /// that reach it, cycles closed included: only those walks are dropped, and the reverse edges gain the new ones.
    /// Cleared whole, every placement walked every target again.
    pub fn placed(&self, name: &str, demand: &str, guards: &BTreeSet<String>) {
        if let Some(rev) = self.rev.borrow_mut().as_mut() {
            rev.entry(demand.to_owned()).or_default().push(name.to_owned());
            for g in guards {
                rev.entry(g.clone()).or_default().push(demand.to_owned());
            }
        }
        self.to.borrow_mut().retain(|t, into| t != demand && !guards.contains(t) && guards.iter().all(|g| !into.contains(g)));
    }
}

/// Whether `start` depends on `target` through `edges` (`analyses::strata`'s: a relation to those it reads).
pub fn depends(edges: &front::Edges, start: &str, target: &str) -> bool {
    let mut todo = vec![start];
    let mut seen = BTreeSet::new();
    while let Some(at) = todo.pop() {
        if at == target {
            return true;
        }
        if seen.insert(at) {
            todo.extend(edges.get(at).into_iter().flatten().map(|(w, _)| w.as_str()));
        }
    }
    false
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Each candidate stratified whole, as before the shortcut (the test of the shortcut, which the engine hosts).
    pub static FULL_CHECKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn full_checks() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    return FULL_CHECKS.with(std::cell::Cell::get);
    #[cfg(not(any(test, feature = "test-hooks")))]
    false
}

// cratify: `tests` is rerail-logic's (it mounts it)

/// Whether `rel` is a helper the lowering makes (`rr_ctx…`, a context's `rr_cover…` groups, `rr_agg…`, and their
/// demand `rr_demand_agg_…`), under any program's prefix.
pub fn helper(rel: &str) -> bool {
    rel.contains("rr_ctx") || rel.contains("rr_cover") || rel.contains("rr_agg")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    /// After a placement, every target's answers are those of a walk of the new edges from scratch, a cycle the
    /// placement closes included: `Reach::placed` drops exactly the walks the new edges change.
    #[test]
    fn a_placement_keeps_only_the_walks_it_leaves_unchanged() {
        let edges = |list: &[(&str, &str)]| {
            let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for (from, to) in list {
                deps.entry((*from).to_owned()).or_default().insert((*to).to_owned());
            }
            deps
        };
        // `out` reads `agg`, which reads `base`; `side` reads `other`; `guard` reads `base`, `loop` reads `out`.
        let before = [("out", "agg"), ("agg", "base"), ("side", "other"), ("guard", "base"), ("loop", "out")];
        let names = ["out", "agg", "base", "side", "other", "guard", "loop", "demand"];
        for guards in [vec!["guard"], vec!["loop"], vec!["other", "base"]] {
            let mut deps = edges(&before);
            let reach = super::Reach::default();
            // Every walk made before the placement.
            for t in names {
                reach.reaches(&deps, "out", t);
            }
            let guards: BTreeSet<String> = guards.iter().map(|g| (*g).to_owned()).collect();
            reach.placed("agg", "demand", &guards);
            deps.entry("agg".to_owned()).or_default().insert("demand".to_owned());
            deps.entry("demand".to_owned()).or_default().extend(guards.iter().cloned());
            let fresh = super::Reach::default();
            for start in names {
                for target in names {
                    assert_eq!(reach.reaches(&deps, start, target), fresh.reaches(&deps, start, target), "{start} -> {target}, guards {guards:?}");
                }
            }
        }
    }
}
