//! Dependency-directed invalidation of live join plans. Slot zero is a relation's row count;
//! slot c + 1 is column c's distribution (NDV, fanout and complete value frequencies).
use super::{Constant, Literal, Model, Rule, Step, Term, near};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
struct Version {
    value: u64,
    readers: Vec<usize>,
    priced: Option<(Vec<f64>, Option<HashMap<Constant, i64>>)>,
}

pub struct Watch {
    versions: Vec<Vec<Version>>,
    delta_of: Vec<Option<usize>>,
    delta_readers: Vec<Vec<usize>>,
    delta_priced: Vec<Option<f64>>,
    deltas: Vec<i64>,
    dirty: Vec<bool>,
    /// Per probe, its steps' observed fanouts as last published (`observed`).
    fans: Vec<Vec<Option<f64>>>,
    whole_pending: Vec<bool>,
    required: Vec<Vec<usize>>,
    rows: Vec<Option<i64>>,
}

impl Watch {
    pub fn new(rels: &BTreeMap<String, usize>, probes: &[(&Rule, usize, Vec<Step>)]) -> Self {
        let mut watch = Self {
            versions: (0..rels.len()).map(|_| Vec::new()).collect(),
            delta_of: Vec::new(),
            delta_readers: vec![Vec::new(); rels.len()], delta_priced: vec![None; rels.len()],
            deltas: vec![0; rels.len()], dirty: vec![true; probes.len()],
            fans: vec![Vec::new(); probes.len()],
            whole_pending: vec![true; probes.len()],
            required: Vec::new(), rows: vec![None; rels.len()],
        };
        for (p, (rule, delta, _)) in probes.iter().enumerate() {
            let delta_rel = rule.body[*delta].atom().and_then(|a| rels.get(a.rel.as_str())).copied();
            watch.delta_of.push(delta_rel);
            watch.required.push(rule.body.iter().filter_map(|lit| match lit {
                Literal::Atom(a) if !a.neg => rels.get(a.rel.as_str()).copied(),
                _ => None,
            }).collect());
            if let Some(r) = delta_rel { watch.delta_readers[r].push(p); }
            for atom in rule.body.iter().filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a), _ => None,
            }) {
                let Some(&r) = rels.get(atom.rel.as_str()) else { continue };
                let slots = &mut watch.versions[r];
                slots.resize_with(slots.len().max(atom.args.len() + 1), Version::default);
                let used = std::iter::once(0).chain(atom.args.iter().enumerate()
                    .filter(|(_, t)| !matches!(t, Term::Wild)).map(|(c, _)| c + 1));
                for slot in used {
                    if !slots[slot].readers.contains(&p) { slots[slot].readers.push(p); }
                }
            }
        }
        watch
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn changed(&mut self, rel: usize, slot: usize) {
        let Some(version) = self.versions[rel].get_mut(slot) else { return };
        version.value += 1;
        for &p in &version.readers { self.dirty[p] = true; }
    }
    /// Keep raw versions, but publish pricing changes at the planner's existing drift threshold.
    /// Compare against the last publication so small increments eventually cross it. Key-domain changes
    /// always publish, even when counts and fanouts are unchanged.
    pub fn record(&mut self, rel: usize, slot: usize, model: &Model, name: &str) {
        let Some(version) = self.versions[rel].get_mut(slot) else { return };
        version.value += 1;
        if version.readers.is_empty() { return; }
        let key = (name.to_owned(), slot.saturating_sub(1));
        let (sizes, freq) = if slot == 0 {
            (vec![model.card(name)], None)
        } else {
            (vec![model.ndv.get(&key).copied().unwrap_or(1.0), model.stats.fan.get(&key).copied().unwrap_or(1.0)], model.stats.dfreq.get(&key))
        };
        let same = version.priced.as_ref().is_some_and(|(old, before)| near(old, &sizes) && match (before.as_ref(), freq) {
            (None, None) => true,
            (Some(a), Some(b)) => a.len() == b.len() && a.iter().all(|(k, n)| b.get(k).is_some_and(|m| (*n == 0) == (*m == 0) && near(&[*n as f64], &[*m as f64]))),
            _ => false,
        });
        if !same {
            version.priced = Some((sizes, freq.cloned()));
            for &p in &version.readers { self.dirty[p] = true; }
        }
    }
    pub fn rows(&mut self, rel: usize, rows: i64) {
        let old = self.rows[rel].replace(rows);
        if old.is_none_or(|old| (old == 0) != (rows == 0))
            && let Some(version) = self.versions[rel].first() {
                for &p in &version.readers { self.dirty[p] = true; }
            }
    }
    pub fn delta(&mut self, rel: usize, rows: i64) {
        self.deltas[rel] = rows;
        if self.delta_priced[rel].is_none_or(|old| !near(&[old], &[rows as f64])) {
            self.delta_priced[rel] = Some(rows as f64);
            for &p in &self.delta_readers[rel] { self.dirty[p] = true; }
        }
    }
    /// A probe's steps' observed fanouts (`observed_fans`), which price those steps first: a fanout seen for the
    /// first time, or moved past the drift threshold, makes the probe's plan dirty. `pfn`'s probe priced
    /// `pcall__ins` at 0.01 rows a binding, saw 20.8, and was not planned again.
    pub fn observed(&mut self, probe: usize, fans: Vec<Option<f64>>) {
        let old = &self.fans[probe];
        let moved = fans.iter().enumerate().any(|(i, f)| match (old.get(i).copied().flatten(), *f) {
            (None, Some(_)) => true,
            (Some(a), Some(b)) => !near(&[a.max(0.01)], &[b.max(0.01)]),
            _ => false,
        });
        if moved {
            self.dirty[probe] = true;
            self.fans[probe] = fans;
        }
    }
    pub fn take(&mut self, probe: usize) -> bool {
        if self.delta_of[probe].is_some_and(|r| self.deltas[r] == 0) { return false; }
        if self.required[probe].iter().any(|&r| self.rows[r] == Some(0)) { return false; }
        std::mem::replace(&mut self.dirty[probe], false)
    }
    /// Price a whole join once its required inputs can produce rows. An empty
    /// seed wave defers this work; it does not discard the later replan.
    pub fn take_whole(&mut self, probe: usize) -> bool {
        if self.required[probe].iter().any(|&r| self.rows[r] == Some(0)) { return false; }
        std::mem::replace(&mut self.whole_pending[probe], false)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[test]
    fn only_affected_active_plans_reconsider_their_signatures() {
        let program = crate::code_rule_dsl::logic::rr::parse(
            ".decl a(x:number,y:number)\n.decl b(x:number)\n.decl c(x:number)\n.decl p(x:number)\n.decl q(x:number)\np(x) :- a(x,_), b(x).\nq(x) :- c(x).\n").unwrap();
        let rels = BTreeMap::from([("a".into(),0),("b".into(),1),("c".into(),2)]);
        let probes = vec![(&program.rules[0],0,vec![]),(&program.rules[1],0,vec![])];
        let mut w = Watch::new(&rels,&probes);
        assert!(!w.take(0), "empty deltas defer planning, including the first wave");
        w.delta(0,5); w.delta(2,2);
        assert!(w.take(0)); assert!(w.take(1));
        assert!(!w.take(0)); assert!(!w.take(1));
        w.changed(0,2); // Unused wildcard column.
        assert!(!w.take(0));
        w.changed(1,1); // A changed histogram matters, even with unchanged counts.
        assert!(w.take(0)); assert!(!w.take(1));
        assert_eq!(w.versions[1][1].value,1);
        w.delta(0,0); w.changed(0,0);
        assert!(!w.take(0));
        w.delta(0,1); assert!(w.take(0), "retain invalidation until the delta can execute");
        // A step's fanout observed for the first time, then moved past the drift threshold, replans the probe;
        // a small move does not.
        w.observed(0, vec![None, Some(0.01)]); assert!(w.take(0));
        w.observed(0, vec![None, Some(0.015)]); assert!(!w.take(0));
        w.observed(0, vec![None, Some(20.8)]); assert!(w.take(0));
    }
}
