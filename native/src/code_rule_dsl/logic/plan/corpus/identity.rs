//! A measurement (`RERAIL_IDENTITY_MEASURE`, never a mode to run in): how much of a session's change to each relation
//! is a node's identity changing, not a fact changing. Each relation's old and new rows are compared as they are, and
//! again with every node replaced by a *mangled name* made from the syntax rows alone, as a compiler names what it
//! emits: the document's path, then from the root each node's kind and name (its `name` field's text) or, for one with
//! no name, its kind and ordinal among its parent's children of that kind. A fact whose nodes keep their mangled names
//! is the same fact, however the nodes were numbered.

use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::native::component::InRel;
use rustc_hash::{FxHashMap, FxHashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

type Input<'a> = &'a dyn Fn(&str) -> Option<Arc<dyn Source>>;

fn rows_of(src: &dyn Source) -> Vec<Vec<i64>> {
    let w = src.columns().len();
    (0..src.len()).map(|i| (0..w).map(|c| src.value(i, c)).collect()).collect()
}

fn mix(parts: &[i64]) -> i64 {
    let mut h = rustc_hash::FxHasher::default();
    parts.hash(&mut h);
    // Tagged so a mangled name never equals a raw id.
    (h.finish() as i64) | i64::MIN
}

/// Each node's mangled name, from one side's `node`, `child`, `field`, `text` and `doc` rows.
fn names(input: Input, name_fields: &FxHashSet<i64>) -> FxHashMap<i64, i64> {
    let get = |r: &str| input(r).map(|s| rows_of(s.as_ref())).unwrap_or_default();
    let (node, child, field, text, doc) = (get("node"), get("child"), get("field"), get("text"), get("doc"));
    // A document by its key (`doc`'s rows are not kept for the old side): the key is its path's.
    let _ = &doc;
    let kind: FxHashMap<i64, (i64, i64)> = node.iter().map(|r| (r[1], (r[0], r[2]))).collect();
    let text: FxHashMap<i64, i64> = text.iter().map(|r| (r[0], r[1])).collect();
    let named: FxHashMap<i64, i64> = field.iter().filter(|r| name_fields.contains(&r[1])).filter_map(|r| text.get(&r[2]).map(|&t| (r[0], t))).collect();
    let mut parent: FxHashMap<i64, i64> = FxHashMap::default();
    let mut kids: FxHashMap<i64, Vec<(i64, i64)>> = FxHashMap::default();
    for r in &child {
        parent.insert(r[1], r[0]);
        kids.entry(r[0]).or_default().push((r[2], r[1]));
    }
    // Each child's segment: kind and name, or kind and ordinal; a repeated (kind, name) also takes an ordinal.
    let mut segment: FxHashMap<i64, i64> = FxHashMap::default();
    for list in kids.values_mut() {
        list.sort_unstable();
        let mut seen: FxHashMap<(i64, i64, bool), i64> = FxHashMap::default();
        for &(_, c) in list.iter() {
            let k = kind.get(&c).map_or(-1, |x| x.1);
            let (label, is_name) = named.get(&c).map_or((0, false), |&t| (t, true));
            let o = seen.entry((k, label, is_name)).or_insert(0);
            segment.insert(c, mix(&[k, label, i64::from(is_name), *o]));
            *o += 1;
        }
    }
    let mut out: FxHashMap<i64, i64> = FxHashMap::default();
    for &n in kind.keys() {
        // The chain up to the root, then names down it.
        let mut chain = vec![n];
        while let Some(&p) = parent.get(chain.last().unwrap()) {
            if out.contains_key(&p) {
                break;
            }
            chain.push(p);
        }
        for &m in chain.iter().rev() {
            if out.contains_key(&m) {
                continue;
            }
            let v = match parent.get(&m) {
                Some(p) => mix(&[out[p], segment.get(&m).copied().unwrap_or(0)]),
                None => {
                    let (d, k) = kind.get(&m).copied().unwrap_or((-1, -1));
                    mix(&[d, k])
                }
            };
            out.insert(m, v);
        }
    }
    out
}

/// For each relation: rows old and new, changed as they are, and changed by mangled names; and by stratum.
pub(in crate::code_rule_dsl) fn measure(
    prog: &front::Program,
    now: &[(String, usize, Arc<InRel>)],
    old: &dyn Fn(&str) -> Option<Arc<InRel>>,
    new_input: Input,
    old_input: Input,
    name_fields: &FxHashSet<i64>,
) {
    let (na, nb) = (names(old_input, name_fields), names(new_input, name_fields));
    // Old and new nodes matched as a tree diff would: by id where the id is in both, else by a mangled name that one
    // old node alone has and whose old id is gone. A matched new node stands for its old one; the old side is as it is.
    let mut by_name: FxHashMap<i64, Option<i64>> = FxHashMap::default();
    for (&id, &n) in &na {
        if !nb.contains_key(&id) {
            by_name.entry(n).and_modify(|e| *e = None).or_insert(Some(id));
        }
    }
    let a: FxHashMap<i64, i64> = FxHashMap::default();
    let b: FxHashMap<i64, i64> = nb.iter().filter(|(id, _)| !na.contains_key(*id)).filter_map(|(&id, n)| by_name.get(n).copied().flatten().map(|o| (id, o))).collect();
    eprintln!("RERAIL_IDENTITY matched: {} new nodes have new ids; {} of them matched to an old node by name", nb.keys().filter(|id| !na.contains_key(*id)).count(), b.len());
    let (a, b) = (a, b);

    let node_cols = |rel: &str| -> Vec<usize> {
        prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect())
    };
    let positional = |rel: &str| prog.decls.get(rel).is_some_and(|cols| cols.iter().any(|(_, t)| t == "loc" || t == "span"));
    let flat = |h: &InRel, w: usize| -> Option<Vec<Vec<i64>>> { h.live_rows().map(|r| r.chunks(w.max(1)).map(<[i64]>::to_vec).collect()) };
    let delta = |o: &[Vec<i64>], n: &[Vec<i64>], cols: &[usize], map: Option<(&FxHashMap<i64, i64>, &FxHashMap<i64, i64>)>| -> usize {
        let key = |r: &Vec<i64>, m: Option<&FxHashMap<i64, i64>>| -> Vec<i64> {
            let mut r = r.clone();
            if let Some(m) = m {
                for &c in cols {
                    if let Some(&v) = r.get(c).and_then(|x| m.get(x)) {
                        r[c] = v;
                    }
                }
            }
            r
        };
        let so: FxHashSet<Vec<i64>> = o.iter().map(|r| key(r, map.map(|x| x.0))).collect();
        let sn: FxHashSet<Vec<i64>> = n.iter().map(|r| key(r, map.map(|x| x.1))).collect();
        so.symmetric_difference(&sn).count()
    };
    let mut per: FxHashMap<String, (usize, usize, usize, usize)> = FxHashMap::default();
    let mut measure_one = |rel: &str, o: Vec<Vec<i64>>, n: Vec<Vec<i64>>| {
        let cols = node_cols(rel);
        let raw = delta(&o, &n, &cols, None);
        let named = if raw == 0 { 0 } else { delta(&o, &n, &cols, Some((&a, &b))) };
        per.insert(rel.to_owned(), (o.len(), n.len(), raw, named));
    };
    for rel in &prog.inputs {
        if let (Some(o), Some(n)) = (old_input(rel), new_input(rel)) {
            measure_one(rel, rows_of(o.as_ref()), rows_of(n.as_ref()));
        }
    }
    for (rel, w, h) in now {
        let Some(o) = old(rel) else { continue };
        // A view's rows are not held: left out.
        let (Some(o), Some(n)) = (flat(&o, *w), flat(h, *w)) else { continue };
        measure_one(rel, o, n);
    }
    let mut list: Vec<(&String, &(usize, usize, usize, usize))> = per.iter().collect();
    list.sort_by_key(|(r, v)| (std::cmp::Reverse(v.2), (*r).clone()));
    let (mut raw, mut named, mut churn_only, mut changed) = (0, 0, 0, 0);
    for &(rel, &(o, n, r, m)) in &list {
        if r > 0 {
            changed += 1;
            if m == 0 {
                churn_only += 1;
            }
        }
        raw += r;
        named += m;
        let base = if prog.inputs.iter().any(|i| i == rel) { " input" } else { "" };
        let pos = if positional(rel) { " positional" } else { "" };
        eprintln!("RERAIL_IDENTITY {rel}{base}{pos}: old {o}, new {n}, changed {r}, by names {m}");
    }
    eprintln!("RERAIL_IDENTITY total: {} relations, {changed} changed, {churn_only} of them only by identity; rows changed {raw}, by names {named}", list.len());
    // By stratum, in order: the first stratum whose outputs change by names while all it reads is unchanged by names
    // is where real change starts; one whose outputs change only by identity propagates churn.
    let Ok((sccs, _)) = front::strata(prog) else { return };
    let reads = super::retain::Reads::of(prog);
    let (mut quiet, mut churn, mut real, mut silent) = (0, 0, 0, 0);
    for scc in &sccs {
        let out: (usize, usize) = scc.iter().filter_map(|r| per.get(r.as_str())).fold((0, 0), |s, v| (s.0 + v.2, s.1 + v.3));
        let inn: (usize, usize) = reads.of_scc(scc).iter().filter(|r| !scc.contains(*r)).filter_map(|r| per.get(r.as_str())).fold((0, 0), |s, v| (s.0 + v.2, s.1 + v.3));
        match (inn.0 > 0, out.0 > 0, out.1 > 0) {
            (false, false, _) => quiet += 1,
            (_, true, false) => churn += 1,
            (_, true, true) => real += 1,
            (true, false, _) => silent += 1,
        }
    }
    eprintln!("RERAIL_IDENTITY strata: {} in all; {quiet} read and made no change; {silent} read a change and made none; {churn} made changes that are identity only; {real} made changes by names", sccs.len());
}
