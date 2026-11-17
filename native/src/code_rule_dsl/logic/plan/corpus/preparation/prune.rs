//! `front::prune` on the version (decision 0281's path, step 6d): the rules of relations no output reads, directly
//! or through other rules, dropped, and the request rules none of whose readers is read (decision 0174). The record
//! of each request's readers follows the rules kept; a native nothing reads is dropped with them. Facts stay.

use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, Cell, RuleRef};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// How many rules it dropped, over its runs.
#[derive(Default)]
pub struct Prune(pub usize);

impl ProgramPass for Prune {
    const NAME: &'static str = "prune";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let about = About::of(v);
        let roots: Vec<String> = v.outputs().map(|a| v.text(a).to_owned()).collect();
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let head = |i: usize| v.rel_name(rules[i].head().rel());
        // Each request's readers, rule by rule (`Program::asked_by`, the `asked` rows).
        let b0 = w.input().clone();
        let asked: BTreeMap<String, Vec<BTreeSet<String>>> = {
            let mut out: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
            for f in b0.about_facts().into_iter().filter(|f| b0.text(f.name()) == "asked") {
                for row in f.rows() {
                    let row: Vec<Cell> = row.collect();
                    if let [_, Cell::Str(req), Cell::Int(k), Cell::Str(by)] = row.as_slice() {
                        let k = *k as usize;
                        let e = out.entry(b0.text(*req).to_owned()).or_default();
                        if e.len() <= k {
                            e.resize_with(k + 1, BTreeSet::new);
                        }
                        e[k].insert(b0.text(*by).to_owned());
                    }
                }
            }
            out
        };
        let mut gate: Vec<Option<&BTreeSet<String>>> = vec![None; rules.len()];
        for (req, readers) in asked.iter().filter(|(req, _)| !roots.contains(req)) {
            let at: Vec<usize> = (0..rules.len()).filter(|&i| head(i) == req.as_str()).collect();
            if at.len() == readers.len() {
                for (i, by) in at.into_iter().zip(readers) {
                    gate[i] = Some(by);
                }
            }
        }
        let mut body_of: HashMap<&str, Vec<usize>> = HashMap::new();
        for i in 0..rules.len() {
            body_of.entry(head(i)).or_default().push(i);
        }
        let native_reads: HashMap<&str, Vec<&str>> = about.natives.iter().map(|n| (v.rel_name(n.rel), n.reads.iter().map(|&r| v.rel_name(r)).collect())).collect();
        let reads = |i: usize| -> Vec<String> { rules[i].body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect() };
        let mut kept = vec![false; rules.len()];
        let (mut need, mut todo): (BTreeSet<String>, Vec<String>) = (BTreeSet::new(), roots.clone());
        loop {
            while let Some(r) = todo.pop() {
                if !need.insert(r.clone()) {
                    continue;
                }
                if let Some(rs) = native_reads.get(r.as_str()) {
                    todo.extend(rs.iter().map(|x| (*x).to_owned()));
                }
                for &i in body_of.get(r.as_str()).into_iter().flatten() {
                    if gate[i].is_none() {
                        kept[i] = true;
                        todo.extend(reads(i));
                    }
                }
            }
            for (i, by) in gate.iter().enumerate() {
                if let Some(by) = by
                    && !kept[i]
                    && need.contains(head(i))
                    && by.iter().any(|b| need.contains(b))
                {
                    kept[i] = true;
                    todo.extend(reads(i));
                }
            }
            if todo.is_empty() {
                break;
            }
        }
        // The record follows the rules kept.
        let mut asked_after: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
        for (req, readers) in &asked {
            let mine: Vec<bool> = (0..rules.len()).filter(|&i| head(i) == req.as_str()).map(|i| kept[i]).collect();
            let mut readers = readers.clone();
            if mine.len() == readers.len() {
                let mut it = mine.iter();
                readers.retain(|_| *it.next().unwrap());
            }
            let has_rules = (0..rules.len()).any(|i| kept[i] && head(i) == req.as_str());
            if !readers.is_empty() || has_rules {
                asked_after.insert(req.clone(), readers);
            }
        }
        let dropped_natives: HashSet<String> = native_reads.keys().filter(|r| !need.contains(**r)).map(|r| (*r).to_owned()).collect();
        let dropped = kept.iter().filter(|k| !**k).count();
        if dropped == 0 && asked_after == asked && dropped_natives.is_empty() {
            return Ok(());
        }
        self.0 += dropped;
        let gone: HashSet<_> = rules.iter().zip(&kept).filter(|(_, k)| !**k).map(|(r, _)| r.id()).collect();
        if !gone.is_empty() {
            w.keep(|r| !gone.contains(&r.id()))?;
        }
        let b = w.on()?;
        if asked_after != asked {
            let p = super::demand::about_p(b);
            let mut rows = Vec::new();
            for (req, readers) in &asked_after {
                for (k, by) in readers.iter().enumerate() {
                    for x in by {
                        let (q, y) = (b.atom(req), b.atom(x));
                        rows.push(vec![p, Cell::Str(q), Cell::Int(k as i64), Cell::Str(y)]);
                    }
                }
            }
            b.set_about_rows("asked", rows);
        }
        // The natives' rows (each names its native in column 1).
        if !dropped_natives.is_empty() {
            for (name, _) in crate::code_rule_dsl::logic::program_image::about::RELATION_COLUMNS.iter().filter(|(n, _)| n.starts_with("native_")) {
                super::demand::edit_native_rows(b, name, &dropped_natives, Vec::new());
            }
        }
        Ok(())
    }
}
