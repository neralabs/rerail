//! T's `prune` on the version (decision 0281's path, step 6d): the rules of relations no output reads, directly
//! or through other rules, dropped, and the request rules none of whose readers is read (decision 0174). The record
//! of each request's readers follows the rules kept; a native nothing reads is dropped with them. Facts stay. A
//! reader outside the version (another part's, decision 0345) keeps its ask.

use rerail_blocks::vector::Vector;
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
            let at: Vector<usize> = (0..rules.len()).filter(|&i| head(i) == req.as_str()).collect();
            if at.len() == readers.len() {
                for (i, by) in at.into_iter().zip(readers) {
                    gate[i] = Some(by);
                }
            }
        }
        let mut body_of: HashMap<&str, Vector<usize>> = HashMap::new();
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
                super::demand::edit_native_rows(b, name, &dropped_natives, crate::code_rule_dsl::logic::program_image::Rows::new());
            }
        }
        Ok(())
    }
}

/// The rules no row can reach (decision 0563 §3, A2): given the inputs the run knows are empty (`empty`), a relation
/// is empty when only rules make it and each of its rules joins an empty relation. A rule with a positive atom (not an
/// aggregate's) over an empty relation is dropped; a negated or aggregated one keeps its rule, whose relation is empty
/// when it runs. Then, from what the dropped rules read alone, the relations no rule left reads go with their rules
/// (a demand only they read): never an output, one of `keep` (a part's exports, which the other part reads), a
/// native's or a request. The general `Prune` is not run on a part: its roots are the outputs alone, and a part's
/// exports are none of them.
pub struct Unfillable {
    pub empty: BTreeSet<String>,
    pub keep: BTreeSet<String>,
    /// How many rules it dropped as joining an empty relation, how many as read by those alone, and the relations it
    /// found empty.
    pub dropped: usize,
    pub unread: usize,
    pub found: BTreeSet<String>,
}

impl Unfillable {
    pub fn new(empty: impl IntoIterator<Item = String>, keep: impl IntoIterator<Item = String>) -> Self {
        Unfillable { empty: empty.into_iter().collect(), keep: keep.into_iter().collect(), dropped: 0, unread: 0, found: BTreeSet::new() }
    }
}

impl ProgramPass for Unfillable {
    const NAME: &'static str = "the rules no row can reach";
    fn settings(&self) -> Vec<u8> {
        let mut key: Vec<u8> = self.empty.iter().flat_map(|r| r.bytes().chain([0])).collect();
        key.push(1);
        key.extend(self.keep.iter().flat_map(|r| r.bytes().chain([0])));
        key
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        use crate::code_rule_dsl::logic::program_image::{Lit as L, RelId};
        let v = cx.v();
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let about = About::of(v);
        let positive = |r: &RuleRef<'_>| -> Vec<RelId> {
            r.body().filter_map(|l| match l.view() {
                L::Atom(a) if !a.neg() => Some(a.rel()),
                _ => None,
            }).collect()
        };
        let reads = |r: &RuleRef<'_>| -> Vec<RelId> { r.body().filter_map(|l| l.atom()).map(|a| a.rel()).collect() };
        // Empty: the inputs known empty, then each relation only rules make (no input, fact or native's) every rule
        // of which joins an empty one. Seeded from the inputs alone: a relation some phase or the engine binds beside
        // its rules (a phase's `load`) is never taken for empty because its rules make nothing here.
        let inputs: HashSet<&str> = v.inputs().map(|a| v.text(a)).collect();
        let facts: HashSet<String> = crate::code_rule_dsl::logic::program_image::fields::ProgramFields::fact_names(&**v).into_iter().collect();
        let natives: HashSet<RelId> = about.natives.iter().map(|n| n.rel).collect();
        let native_reads: HashSet<RelId> = about.natives.iter().flat_map(|n| n.reads.iter().copied()).collect();
        let bodies: Vec<Vec<RelId>> = rules.iter().map(positive).collect();
        let mut by_head: HashMap<RelId, Vec<usize>> = HashMap::new();
        for (i, r) in rules.iter().enumerate() {
            by_head.entry(r.head().rel()).or_default().push(i);
        }
        // The rules each relation is read by: positively (the forward closure), at all (the backward one).
        let mut positive_readers: HashMap<RelId, Vec<usize>> = HashMap::new();
        for (i, body) in bodies.iter().enumerate() {
            for &a in body {
                positive_readers.entry(a).or_default().push(i);
            }
        }
        let reads_of: Vec<Vec<RelId>> = rules.iter().map(reads).collect();
        let mut readers: HashMap<RelId, usize> = HashMap::new();
        for rs in &reads_of {
            for &a in rs {
                *readers.entry(a).or_default() += 1;
            }
        }
        // Forward, as a worklist: a rule is dead once one of its positive atoms is empty, and a relation only rules
        // make is empty once every rule of it is.
        let mut empty: HashSet<RelId> = self.empty.iter().filter(|r| inputs.contains(r.as_str())).filter_map(|r| v.rel(r)).collect();
        let mut dead = vec![false; rules.len()];
        let mut live_rules: HashMap<RelId, usize> = by_head.iter().map(|(&h, at)| (h, at.len())).collect();
        let mut todo: Vec<RelId> = empty.iter().copied().collect();
        while let Some(rel) = todo.pop() {
            for &i in positive_readers.get(&rel).into_iter().flatten() {
                if std::mem::replace(&mut dead[i], true) {
                    continue;
                }
                let h = rules[i].head().rel();
                let left = live_rules.get_mut(&h).expect("a rule's head");
                *left -= 1;
                let name = v.rel_name(h);
                if *left == 0 && !inputs.contains(name) && !facts.contains(name) && !natives.contains(&h) && empty.insert(h) {
                    todo.push(h);
                }
            }
        }
        let mut gone: HashSet<usize> = (0..rules.len()).filter(|&i| dead[i]).collect();
        let dropped = gone.len();
        // Backward, as a worklist: a relation no rule left reads goes with its rules, unless something outside the
        // rules reads it.
        let outputs: HashSet<&str> = v.outputs().map(|a| v.text(a)).collect();
        let kept = |rel: RelId| {
            let name = v.rel_name(rel);
            outputs.contains(name)
                || self.keep.contains(name)
                || natives.contains(&rel)
                || native_reads.contains(&rel)
                || crate::code_rule_dsl::logic::plan::corpus::guests::REQUESTS.contains(&name)
        };
        let mut todo: Vec<usize> = gone.iter().copied().collect();
        while let Some(i) = todo.pop() {
            for &rel in &reads_of[i] {
                let left = readers.get_mut(&rel).expect("a read");
                *left -= 1;
                if *left > 0 || kept(rel) {
                    continue;
                }
                for &j in by_head.get(&rel).into_iter().flatten() {
                    if gone.insert(j) {
                        todo.push(j);
                    }
                }
            }
        }
        self.found.extend(empty.iter().map(|&a| v.rel_name(a).to_owned()));
        if gone.is_empty() {
            return Ok(());
        }
        self.dropped += dropped;
        self.unread += gone.len() - dropped;
        let ids: HashSet<_> = gone.iter().map(|&i| rules[i].id()).collect();
        w.keep(|r| !ids.contains(&r.id()))?;
        Ok(())
    }
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
