//! T's `prune` on the version (decision 0281's path, step 6d): the rules of relations no output reads, directly
//! or through other rules, dropped, and the request rules none of whose readers is read (decision 0174). The record
//! of each request's readers follows the rules kept; a native nothing reads is dropped with them. Facts stay. A
//! reader outside the version (another part's, decision 0345) keeps its ask.

use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, RuleRef};
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
        let asked = w.input().asked();
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
        // A reader the version neither declares nor makes is another program's: a part prepared alone (decision
        // 0345) whose request another part reads, or a reader gone from the program (merged, renamed or inlined since
        // the requests were made, Typing-support's 52e04565d). Whether it is read is not this version's to know, so
        // its ask is kept, as an ungated rule is. A reader it declares and makes nothing of is unread: its ask goes.
        let outside = |b: &str| v.decl_of(b).is_none() && !body_of.contains_key(b);
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
                    && by.iter().any(|b| need.contains(b) || outside(b))
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
        let dropped: BTreeSet<&str> = (0..rules.len()).filter(|&i| !kept[i] && !need.contains(head(i))).map(head).collect();
        let mut asked_after: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
        for (req, readers) in &asked {
            let mine: Vec<bool> = (0..rules.len()).filter(|&i| head(i) == req.as_str()).map(|i| kept[i]).collect();
            let mut readers = readers.clone();
            if mine.len() == readers.len() {
                let mut it = mine.iter();
                readers.retain(|_| *it.next().unwrap());
            }
            // A reader `prune` drops is not needed: it leaves the record, beside one it keeps, so a later gate (the
            // embed slice's) does not take it for one gone by a rename, whose requests stay.
            for by in &mut readers {
                if by.iter().any(|b| dropped.contains(b.as_str())) && by.iter().any(|b| !dropped.contains(b.as_str())) {
                    by.retain(|b| !dropped.contains(b.as_str()));
                }
            }
            let has_rules = (0..rules.len()).any(|i| kept[i] && head(i) == req.as_str());
            if !readers.is_empty() || has_rules {
                asked_after.insert(req.clone(), readers);
            }
        }
        // A reader the prune drops is not needed: it leaves the record where its request has readers left, as the
        // tree's prune takes it out (`front::prune`), so a later gate does not take it for one gone by a rename.
        let dropped_readers: HashSet<&str> = (0..rules.len()).filter(|&i| !kept[i] && !need.contains(head(i))).map(head).collect();
        for readers in asked_after.values_mut().flatten() {
            if readers.iter().any(|b| dropped_readers.contains(b.as_str())) && readers.iter().any(|b| !dropped_readers.contains(b.as_str())) {
                readers.retain(|b| !dropped_readers.contains(b.as_str()));
            }
        }
        let dropped_natives: HashSet<String> = native_reads.keys().filter(|r| !need.contains(**r)).map(|r| (*r).to_owned()).collect();
        // An attribute answered on request (decision 0174) whose readers were all pruned has no request and no answer
        // left: it is an input again, as the tree's prune makes it (734317304).
        let fd = cx.get::<crate::code_rule_dsl::logic::plan::fields::FieldsData>().map_err(|e| e.to_string())?;
        let f = crate::code_rule_dsl::logic::plan::fields::Fields::over(v, &fd);
        let derives = |rel: &str| (0..rules.len()).any(|i| kept[i] && head(i) == rel);
        let mut revived: Vec<String> = Vec::new();
        for attr in crate::code_rule_dsl::logic::rr::lower::ON_REQUEST.iter().flat_map(|round| round.iter()) {
            if f.declared(attr.rel) && !f.is_input(attr.rel) && !derives(attr.rel) && !derives(attr.req) && !revived.iter().any(|x| x == attr.rel) {
                revived.push(attr.rel.to_owned());
            }
        }
        let dropped = kept.iter().filter(|k| !**k).count();
        if dropped == 0 && asked_after == asked && dropped_natives.is_empty() && revived.is_empty() {
            return Ok(());
        }
        self.0 += dropped;
        let gone: HashSet<_> = rules.iter().zip(&kept).filter(|(_, k)| !**k).map(|(r, _)| r.id()).collect();
        if !gone.is_empty() {
            w.keep(|r| !gone.contains(&r.id()))?;
        }
        let inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).chain(revived.iter().cloned()).collect();
        let b = w.on()?;
        if !revived.is_empty() {
            b.fields().set_inputs(&inputs);
        }
        if asked_after != asked {
            b.fields().set_asked(&asked_after);
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

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
