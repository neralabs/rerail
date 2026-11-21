//! Demand that would join what a request keeps apart (decisions 0065 §2a, 0431 §8). A rewrite must not join
//! components the program keeps apart. A demanded relation `R` that a request reads (through the rules it reads) has one
//! demand, `magic_R_ad`, fed by every rule that reads `R`. When one of those rules asks for `R` after a literal that
//! reads the request's answer (`parsed`, through `ref_site`), the demand pulls `R`, and everything between the request
//! and `R`, into the request's own component: `rr_embed_req -> … -> tsub -> unparen -> magic_unparen_bf -> ref_site
//! -> … -> parsed -> rr_embed_req`. The program has no such cycle; its demand made it. A negation or a count inside
//! that component (`RankForm`'s, made after) is then refused by the gate.
//!
//! Each such `R` is split: its component is copied (`R__<request>`), and the rules whose demand reads the answer read
//! the copy, with its own demand. The rest keep `R`, whose demand no longer reads what the request answers. The
//! copies are demanded relations like any other, and their rules differ from `R`'s by their guards, so no merge of
//! identical relations folds them back. A split's copy reads the relations below it under its own demand, which may
//! need a split in turn: each round splits what the last left, until no demand joins a request's component.

use super::{M, Made, show_tm};
use crate::code_rule_dsl::logic::front::{Edges, path, tarjan};
use crate::code_rule_dsl::logic::program_image::{About, AtomRef, Builder, Rewrite, RuleId, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Each request and the relations its answer produces, which the program reads: `about.answers`, and the guest
/// relations `rr_embed_req` answers in the fixpoint (`guests::answered`, decision 0431 §8), which the gate makes its
/// answers only later (`answers::gate`).
pub(super) fn requests(v: &Version, about: &About) -> BTreeMap<String, BTreeSet<String>> {
    use super::super::super::guests;
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (q, p) in &about.answers {
        out.entry(v.rel_name(*q).to_owned()).or_default().insert(v.rel_name(*p).to_owned());
    }
    // Each guest request the fixpoint answers (`guests::answering`: a string's, `@parse`'s, `@splice`'s), with what it
    // produces.
    for (req, (produced, _)) in guests::answering(v) {
        out.entry(req).or_default().extend(produced);
    }
    out
}

/// The program's reads: each rule's head to each relation its body reads (negated or aggregated: `true`), and each
/// relation a request produces to the request.
pub(super) fn graph(v: &Version, requests: &BTreeMap<String, BTreeSet<String>>) -> Edges {
    let mut edges: Edges = BTreeMap::new();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel()).to_owned();
        for l in r.body() {
            if let Some(a) = l.atom() {
                let negative = a.neg() || matches!(l.view(), crate::code_rule_dsl::logic::program_image::Lit::Agg { .. });
                edges.entry(head.clone()).or_default().insert((v.rel_name(a.rel()).to_owned(), negative));
            }
        }
    }
    answered(&mut edges, requests);
    edges
}

fn answered(edges: &mut Edges, requests: &BTreeMap<String, BTreeSet<String>>) {
    for (q, produced) in requests {
        for p in produced {
            edges.entry(p.clone()).or_default().insert((q.clone(), false));
        }
    }
}

/// What `from` reaches through `edges`, `from` left out unless a cycle returns to it.
fn reach(edges: &Edges, from: &str) -> BTreeSet<String> {
    let (mut seen, mut todo) = (BTreeSet::new(), vec![from.to_owned()]);
    while let Some(x) = todo.pop() {
        for (w, _) in edges.get(&x).into_iter().flatten() {
            if seen.insert(w.clone()) {
                todo.push(w.clone());
            }
        }
    }
    seen
}

fn reversed(edges: &Edges) -> Edges {
    let mut out: Edges = BTreeMap::new();
    for (x, ws) in edges {
        for (w, neg) in ws {
            out.entry(w.clone()).or_default().insert((x.clone(), *neg));
        }
    }
    out
}

/// A demanded relation to split: `rel`'s component copied for `callers`, the rules whose demand for it reads what
/// `request` answers; `why`, the cycle the demand made, each read with the rule that makes it.
pub(super) struct Split {
    pub rel: String,
    pub request: String,
    pub callers: BTreeSet<RuleId>,
    pub why: String,
}

/// A rule as the trial wrote it.
fn show(v: &Version, m: &Made<'_>) -> String {
    let head = match &m.head {
        Some((rel, args)) => format!("{rel}({})", args.iter().map(|t| show_tm(v, m.rule, *t)).collect::<Vec<_>>().join(", ")),
        None => v.show_rule(m.rule).split(" :-").next().unwrap_or_default().to_owned(),
    };
    format!("{head} :- {}.", m.body.iter().map(|l| l.show(v, m.rule)).collect::<Vec<_>>().join(", "))
}

/// The demanded relations whose demand, in the trial's rules `made`, joins a request's component with relations the
/// request reads, and the rules that make it so. A round splits first the demands a caller's literals pull from the
/// answer directly (its roots); only once none is left, those pulled through another demand that is (a copy's, whose
/// rules read the relations below it under the copy's demand).
pub(super) fn joins(v: &Version, about: &About, made: &[Made<'_>], program: &Edges, requests: &BTreeMap<String, BTreeSet<String>>) -> Vec<Split> {
    let head_of = |m: &Made<'_>| super::head_of(v, m).to_owned();
    // Each demand and the relation it guards: an adorned rule's first literal.
    let mut guards: BTreeMap<String, String> = BTreeMap::new();
    for m in made.iter().filter(|m| m.head.is_none()) {
        if let Some(M::Guard(g, _)) = m.body.first()
            && g.starts_with("magic_")
        {
            guards.insert(g.clone(), head_of(m));
        }
    }
    // The trial's reads, with the rule that makes each, for the cycle's text.
    let mut trial: Edges = BTreeMap::new();
    let mut by: BTreeMap<(String, String), &Made<'_>> = BTreeMap::new();
    for m in made {
        let h = head_of(m);
        for l in &m.body {
            if let Some((rel, neg, agg)) = l.atom(v) {
                trial.entry(h.clone()).or_default().insert((rel.to_owned(), neg || agg));
                by.entry((h.clone(), rel.to_owned())).or_insert(m);
            }
        }
    }
    answered(&mut trial, requests);
    let comps = tarjan(&program.keys().cloned().chain(program.values().flatten().map(|(w, _)| w.clone())).collect(), program);
    let comp_of = |x: &str| comps.iter().find(|c| c.contains(x));
    let kept = |rel: &str| about.lattice.iter().all(|l| v.rel_name(l.rel) != rel) && about.natives.iter().all(|n| v.rel_name(n.rel) != rel);
    let (mut roots, mut through) = (Vec::new(), Vec::new());
    let back = reversed(program);
    for q in requests.keys() {
        // What reads the answer (and so the request), and what the request reads.
        let mut up = reach(&back, q);
        up.insert(q.clone());
        let down = reach(program, q);
        // The demands whose rules read the answer, directly or through another such demand.
        let mut pulled: BTreeSet<&str> = BTreeSet::new();
        loop {
            let more: Vec<&str> = made
                .iter()
                .filter_map(|m| m.head.as_ref().map(|(h, _)| (h.as_str(), m)))
                .filter(|(h, m)| h.starts_with("magic_") && !pulled.contains(h) && m.body.iter().any(|l| l.atom(v).is_some_and(|(r, _, _)| up.contains(r) || pulled.contains(r))))
                .map(|(h, _)| h)
                .collect();
            if more.is_empty() {
                break;
            }
            pulled.extend(more);
        }
        for (g, rel) in &guards {
            if !down.contains(rel) || up.contains(rel) || !kept(rel) {
                continue;
            }
            let own = comp_of(rel);
            let (mut direct, mut indirect) = (BTreeMap::new(), BTreeMap::new());
            for m in made.iter().filter(|m| m.head.as_ref().is_some_and(|(h, _)| h == g)) {
                let caller = v.rel_name(m.rule.head().rel());
                if own.is_some_and(|c| c.contains(caller)) {
                    continue;
                }
                let reads = |set: &dyn Fn(&str) -> bool| m.body.iter().filter_map(|l| l.atom(v)).map(|(x, _, _)| x).find(|x| set(x)).map(str::to_owned);
                if let Some(x) = reads(&|x| up.contains(x)) {
                    direct.entry(m.rule.id()).or_insert((x, m));
                } else if let Some(x) = reads(&|x| pulled.contains(x)) {
                    indirect.entry(m.rule.id()).or_insert((x, m));
                }
            }
            for (found, into) in [(direct, &mut roots), (indirect, &mut through)] {
                let Some((x, first)) = found.values().next() else { continue };
                // The cycle: the request reads `rel`, `rel` its demand, the demand `x`, and `x` reaches the request.
                let mut steps = path(&trial, q, rel);
                steps.push(g.clone());
                steps.extend(path(&trial, x, q));
                let mut rules = Vec::new();
                for w in steps.windows(2) {
                    let (a, b) = (w[0].trim_start_matches('!'), w[1].trim_start_matches('!'));
                    let text = match (a, b) {
                        _ if a == g && b == x => show(v, first),
                        _ if requests.get(b).is_some_and(|p| p.contains(a)) => format!("{a} is {b}'s answer"),
                        _ => by.get(&(a.to_owned(), b.to_owned())).map_or_else(|| "?".to_owned(), |m| show(v, m)),
                    };
                    rules.push(format!("    {a} -> {b}: {text}"));
                }
                let callers: BTreeSet<RuleId> = found.keys().copied().collect();
                let moved: Vec<String> = found.values().map(|(_, m)| format!("    {}", v.show_rule(m.rule))).collect();
                let why = format!(
                    "{rel}'s demand {g} joins {q}'s component: {}\n{}\n  the rules that read the copy ({}):\n{}",
                    steps.join(" -> "),
                    rules.join("\n"),
                    callers.len(),
                    moved.join("\n")
                );
                into.push(Split { rel: rel.clone(), request: q.clone(), callers, why });
            }
        }
    }
    if roots.is_empty() { through } else { roots }
}

/// A rule copied with the relations of `map` renamed, in its head and its body.
struct Rename<'a> {
    v: &'a Version,
    map: &'a BTreeMap<String, String>,
}

impl Rewrite for Rename<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.map.get(self.v.rel_name(a.rel())).map(|n| (n.clone(), a.neg(), a.gated()))
    }
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        self.map.get(self.v.rel_name(a.rel())).cloned()
    }
}

/// `v` with each split made: each relation of the split relation's component copied (`<r>__<request>`, numbered past
/// a name taken), its callers reading the copies. The program and the copies' names.
pub(super) fn split(v: &Version, splits: &[Split], program: &Edges) -> Result<(Arc<Version>, Vec<(String, String)>), String> {
    let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    let comps = tarjan(&program.keys().cloned().chain(program.values().flatten().map(|(w, _)| w.clone())).collect(), program);
    let mut taken: BTreeSet<String> = v.decls().iter().map(|d| v.text(d.name()).to_owned()).collect();
    let (mut callers, mut copies): (BTreeMap<RuleId, BTreeMap<String, String>>, Vec<BTreeMap<String, String>>) = (BTreeMap::new(), Vec::new());
    let mut names = Vec::new();
    for s in splits {
        let comp = comps.iter().find(|c| c.contains(&s.rel)).cloned().unwrap_or_else(|| BTreeSet::from([s.rel.clone()]));
        let mut map = BTreeMap::new();
        for x in comp.iter().filter(|x| derived.contains(x.as_str())) {
            let base = format!("{x}__{}", s.request);
            let name = (0..).map(|k| if k == 0 { base.clone() } else { format!("{base}{k}") }).find(|n| !taken.contains(n)).expect("a free name");
            taken.insert(name.clone());
            names.push((x.clone(), name.clone()));
            map.insert(x.clone(), name);
        }
        for &c in &s.callers {
            callers.entry(c).or_default().extend(map.iter().map(|(a, b)| (a.clone(), b.clone())));
        }
        copies.push(map);
    }
    let mut b = Builder::derive(v);
    for r in v.rules() {
        match callers.get(&r.id()) {
            Some(map) => {
                b.rewrite(v, r, &mut Rename { v, map }).finish()?;
            }
            None => {
                b.keep(v, r);
            }
        }
    }
    for map in &copies {
        for r in v.rules().filter(|r| map.contains_key(v.rel_name(r.head().rel()))) {
            b.rewrite(v, r, &mut Rename { v, map }).finish()?;
        }
        for (x, copy) in map {
            let d = v.decl_of(x).ok_or_else(|| format!("{x} is not declared"))?;
            let cols: Vec<(String, String)> = d.cols().map(|(n, t)| (v.text(n).to_owned(), v.text(t).to_owned())).collect();
            let cols: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.fields().declare(copy, &cols);
        }
    }
    Ok((b.finish()?.0, names))
}
