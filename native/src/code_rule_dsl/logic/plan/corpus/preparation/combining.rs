//! Combining's passes on the combined program's version (decision 0281's path, step 7a): what the combine's tail
//! (`corpus::tail_version`) does to the programs merged, one manager pass each.
//!
//! - `FactSpaces`: the canonical relations and each narrower policy's guards (decision 0091).
//! - `same_relations`: the relations that are one made one after combining.
//! - `FoldOwn`: the copies hiding made for guests in the run's own language folded back.

use super::simplify::{Made, Out, Tm, write_rule};
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::plan::corpus::scoping::{Policy, Report};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::manager::{Cx, Last, ProgramPass, ProgramW, RuleOut, RulePass, Select, Step};
use crate::code_rule_dsl::logic::program_image::build::{Rewrite, Site, T};
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, Cell, CmpOp, Lit, RuleRef, TermRef, Version};
use crate::code_rule_dsl::typing::file_local::FileLocal;
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Result<T> = std::result::Result<T, String>;

/// The guards and the canonical relations of the policies combined (`scoping`'s module says what each is).
pub(in crate::code_rule_dsl::logic::plan::corpus) struct FactSpaces<'a> {
    pub(in crate::code_rule_dsl::logic::plan::corpus) policies: &'a [Policy],
    pub(in crate::code_rule_dsl::logic::plan::corpus) report: Report,
    /// What `again` reads back (decision 0393): the canonical and shared relations, when guards were written.
    kept: Option<(BTreeSet<String>, BTreeSet<String>)>,
}

impl<'a> FactSpaces<'a> {
    pub(in crate::code_rule_dsl::logic::plan::corpus) fn new(policies: &'a [Policy]) -> Self {
        FactSpaces { policies, report: Report::default(), kept: None }
    }

    /// The canonical relations of `classes` (the congruence classes), the report's canonical, engine and demoted
    /// relations made beside them; an error where a global relation is defined two ways.
    fn canonical(policies: &[Policy], classes: &[Vec<String>], report: &mut Report) -> Result<BTreeSet<String>> {
        let class_of: BTreeMap<&str, usize> = classes.iter().enumerate().flat_map(|(k, c)| c.iter().map(move |r| (r.as_str(), k))).collect();
        let mut canonical: BTreeSet<String> = BTreeSet::new();
        let names: BTreeSet<&String> = policies.iter().flat_map(|p| p.named.iter().chain(&p.library)).collect();
        for name in names {
            let copies: Vec<(usize, String)> = policies
                .iter()
                .enumerate()
                .filter(|(_, p)| p.named.contains(name) || p.library.contains(name))
                .map(|(i, p)| (i, format!("{}{name}", p.prefix)))
                .filter(|(i, r)| policies[*i].own.contains(r))
                .collect();
            // A library held once (decision 0089) left the policies no copy of it.
            if copies.is_empty() {
                continue;
            }
            let alike = copies.windows(2).all(|w| matches!((class_of.get(w[0].1.as_str()), class_of.get(w[1].1.as_str())), (Some(a), Some(b)) if a == b));
            if alike {
                report.canonical.insert(name.clone(), copies.len());
                canonical.extend(copies.into_iter().map(|(_, r)| r));
            } else if copies.iter().any(|(i, _)| policies[*i].named.contains(name)) {
                let first = class_of.get(copies[0].1.as_str());
                let other = copies.iter().find(|(_, r)| class_of.get(r.as_str()) != first || first.is_none()).map_or(copies[0].0, |c| c.0);
                return Err(format!("`.global {name}`: the policies {} and {other} of the run define it differently; a global relation has one definition", copies[0].0));
            } else {
                report.demoted.push(name.clone());
            }
        }
        // The engine's relations (`rr_…`: lowering's helpers, guest contexts, aggregates), alike in several policies,
        // are canonical too: they are made from the library's rules as much as from a policy's. The scope's own are
        // not.
        let engine = |rel: &str| policies.iter().find_map(|p| rel.strip_prefix(p.prefix.as_str())).is_some_and(|r| r.starts_with(front::RESERVED) && !r.starts_with("rr_scope") && !r.starts_with("rr_report"));
        for class in classes {
            let spaces: BTreeSet<usize> = class.iter().filter_map(|r| policies.iter().position(|p| p.own.contains(r))).collect();
            if spaces.len() > 1 && class.iter().all(|r| engine(r)) {
                report.engine += class.len();
                canonical.extend(class.iter().cloned());
            }
        }
        Ok(canonical)
    }

    /// The groups of relations each narrower policy guards (its own, but the shared and its machinery), each with
    /// its outside relation.
    fn groups(policies: &[Policy], shared: &BTreeSet<String>) -> Vec<(BTreeSet<String>, String)> {
        let n = policies.len();
        let scope_of = |i: usize| policies[i].scope.as_ref().map(|s| s.iter().cloned().collect::<BTreeSet<_>>());
        let outside_of = |i: usize| format!("{}{}", policies[i].prefix, front::SCOPE_NOT);
        let narrows = |i: usize| scope_of(i).is_some() && policies[i].own.contains(&outside_of(i)) && (0..n).any(|j| scope_of(j) != scope_of(i));
        let mut groups: Vec<(BTreeSet<String>, String)> = Vec::new();
        for i in (0..n).filter(|&i| narrows(i)) {
            let heads = policies[i].own.iter().filter(|r| !shared.contains(*r) && !machinery(policies, r)).cloned().collect();
            groups.push((heads, outside_of(i)));
        }
        groups
    }
}

/// A relation's places for its guards: its document, node and span columns (`derived`: by the file-local split's
/// locations; an input's by its declared types).
fn place_of(f: Fields<'_>, locations: &BTreeMap<String, Vec<usize>>, derived: bool, rel: &str) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let typed = |c: usize, ty: &[&str]| f.column_type(rel, c).is_some_and(|t| ty.contains(&t));
    let width = f.columns(rel).map_or(0, |c| c.len());
    if derived {
        let cols = locations.get(rel).cloned().unwrap_or_default();
        let of = |ty: &[&str]| cols.iter().copied().filter(|&c| typed(c, ty)).collect::<Vec<_>>();
        (of(&["int", "number"]), of(&["node"]), of(&["loc", "span"]))
    } else {
        let of = |ty: &[&str]| (0..width).filter(|&c| typed(c, ty)).collect::<Vec<_>>();
        (crate::code_rule_dsl::typing::file_local::document_column(rel).into_iter().collect(), of(&["node"]), of(&["loc", "span"]))
    }
}

impl ProgramPass for FactSpaces<'_> {
    const NAME: &'static str = "fact spaces";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let policies = self.policies;
        let report = &mut self.report;
        let s_classes = crate::code_rule_dsl::logic::profile::span(|| "fact spaces: congruence classes".into());
        let classes = cx.get::<super::merge::Congruence>()?.groups.clone();
        drop(s_classes);
        let canonical = Self::canonical(policies, &classes, report)?;
        let canon = canonical.clone();
        // Shared: the canonical relations and every relation of a policy they read, to a fixpoint.
        let s_shared = crate::code_rule_dsl::logic::profile::span(|| "fact spaces: the shared relations".into());
        let owned: BTreeSet<&String> = policies.iter().flat_map(|p| &p.own).collect();
        let mut shared = canonical;
        loop {
            let before = shared.len();
            let reads: Vec<String> = v
                .rules()
                .filter(|r| shared.contains(v.rel_name(r.head().rel())))
                .flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>())
                .chain(f.natives().iter().filter(|n| shared.contains(v.rel_name(n.rel))).flat_map(|n| n.reads.iter().map(|&r| v.rel_name(r).to_owned())))
                .filter(|rel| owned.contains(rel))
                .collect();
            shared.extend(reads);
            if shared.len() == before {
                break;
            }
        }
        drop(s_shared);
        let groups = Self::groups(policies, &shared);
        if groups.is_empty() || std::env::var("RERAIL_SCOPE_GUARDS").is_ok_and(|x| x == "off") {
            return Ok(());
        }
        let split = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "fact spaces: the file-local split's locations".into());
            cx.get::<FileLocal>()?
        };
        let locations = &split.locations;
        let derived: BTreeSet<String> =
            v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).chain(f.natives().iter().map(|n| v.rel_name(n.rel).to_owned())).filter(|r| !f.is_input(r)).collect();
        let place = |rel: &str| place_of(f, locations, derived.contains(rel), rel);
        // Each relation's places, asked once.
        let s_places = crate::code_rule_dsl::logic::profile::span(|| "fact spaces: the places".into());
        let mut places: BTreeMap<String, (Vec<usize>, Vec<usize>, Vec<usize>)> = BTreeMap::new();
        // `docof` too: a span's guard reads it, and a later group's guards read the guards an earlier one made.
        places.insert("docof".to_owned(), place("docof"));
        for r in v.rules() {
            for a in r.body().filter_map(|l| l.atom()) {
                let rel = v.rel_name(a.rel());
                if !places.contains_key(rel) {
                    places.insert(rel.to_owned(), place(rel));
                }
            }
        }
        // Each rule's groups, in order: one pass over the rules, not one per group.
        let mut groups_of: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (k, (heads, _)) in groups.iter().enumerate() {
            for h in heads {
                groups_of.entry(h.clone()).or_default().push(k);
            }
        }
        drop(s_places);
        let mut guards = Guards { policies, groups: &groups, groups_of, places, derived, added: vec![0; groups.len()] };
        w.derive()?;
        w.apply(cx, &mut &mut guards, &Select::All)?;
        for ((_, outside), n) in groups.iter().zip(&guards.added) {
            report.guarded.push((outside.clone(), *n));
        }
        self.kept = Some((canon, shared));
        Ok(())
    }

    fn plan(&self) -> Vec<crate::code_rule_dsl::logic::program_image::manager::Row> {
        use crate::code_rule_dsl::logic::front::Value;
        let Some((canonical, shared)) = &self.kept else { return Vec::new() };
        let s = |x: &str| Value::Str(x.to_owned().into());
        let n = |k: usize| Value::Int(k as i64);
        let mut rows = vec![vec![s("guards")], vec![s("engine"), n(self.report.engine)]];
        rows.extend(canonical.iter().map(|r| vec![s("canon"), s(r)]));
        rows.extend(shared.iter().map(|r| vec![s("shared"), s(r)]));
        rows.extend(self.report.canonical.iter().map(|(r, k)| vec![s("canonical"), s(r), n(*k)]));
        rows.extend(self.report.demoted.iter().map(|r| vec![s("demoted"), s(r)]));
        rows.extend(self.report.guarded.iter().map(|(r, k)| vec![s("guarded"), s(r), n(*k)]));
        rows
    }

    /// Over an edit (decision 0393): the guards of the edit's added rules alone, onto the last output, where its
    /// decisions over the whole are the last run's. The canonical relations are made again from the congruence
    /// classes and must be the last run's; no rule of the edit may have a shared head (so the shared relations are
    /// the last run's); the file-local split must place every relation as it did; and a head the edit changed must
    /// stay derived or not as it was. Otherwise it runs whole.
    fn again(&mut self, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool> {
        use crate::code_rule_dsl::logic::front::Value;
        if !last.delta.decls.is_empty() || std::env::var("RERAIL_SCOPE_GUARDS").is_ok_and(|x| x == "off") {
            return Ok(false);
        }
        let text = |x: &Value| if let Value::Str(s) = x { Some(s.to_string()) } else { None };
        let int = |x: &Value| if let Value::Int(n) = x { usize::try_from(*n).ok() } else { None };
        let (mut canon, mut shared, mut was, mut ran) = (BTreeSet::new(), BTreeSet::new(), Report::default(), false);
        for r in last.plan {
            let Some(kind) = r.first().and_then(text) else { return Ok(false) };
            match (kind.as_str(), &r[1..]) {
                ("guards", []) => ran = true,
                ("engine", [k]) => was.engine = int(k).ok_or("a fact spaces plan's count")?,
                ("canon", [x]) => drop(canon.insert(text(x).ok_or("a fact spaces plan's relation")?)),
                ("shared", [x]) => drop(shared.insert(text(x).ok_or("a fact spaces plan's relation")?)),
                ("canonical", [x, k]) => drop(was.canonical.insert(text(x).ok_or("a fact spaces plan's name")?, int(k).ok_or("a fact spaces plan's count")?)),
                ("demoted", [x]) => was.demoted.push(text(x).ok_or("a fact spaces plan's name")?),
                ("guarded", [x, k]) => was.guarded.push((text(x).ok_or("a fact spaces plan's relation")?, int(k).ok_or("a fact spaces plan's count")?)),
                _ => return Ok(false),
            }
        }
        if !ran {
            return Ok(false);
        }
        let v = cx.v();
        let mut heads: BTreeSet<String> = BTreeSet::new();
        for id in last.delta.removed.iter() {
            let r = last.input.rule(id).ok_or("a removed rule is not its last input's")?;
            heads.insert(last.input.rel_name(r.head().rel()).to_owned());
        }
        for &id in &last.delta.added {
            let r = v.rule(id).ok_or("an added rule is not its input's")?;
            heads.insert(v.rel_name(r.head().rel()).to_owned());
        }
        if heads.iter().any(|h| shared.contains(h)) {
            return Ok(false);
        }
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let policies = self.policies;
        let classes = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "fact spaces: congruence classes".into());
            cx.get::<super::merge::Congruence>()?.groups.clone()
        };
        let mut report = Report::default();
        if Self::canonical(policies, &classes, &mut report)? != canon || report.canonical != was.canonical || report.demoted != was.demoted || report.engine != was.engine {
            return Ok(false);
        }
        let derived_in = |of: &Version, rel: &str| of.rel(rel).is_some_and(|id| of.heads(id).next().is_some());
        let natives: BTreeSet<&str> = f.natives().iter().map(|n| v.rel_name(n.rel)).collect();
        if heads.iter().any(|h| derived_in(v, h) != derived_in(last.input, h)) {
            return Ok(false);
        }
        let split = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "fact spaces: the file-local split's locations".into());
            let now = cx.get::<FileLocal>()?;
            let then = crate::code_rule_dsl::logic::program_image::manager::analysis::<FileLocal>(last.input)?;
            if now.locations != then.locations {
                return Ok(false);
            }
            now
        };
        let locations = &split.locations;
        // Every removed rule made one rule of the last output (a guard keeps or rewrites a rule, never drops one).
        if last.delta.removed.iter().any(|id| last.output.derived(id).count() != 1) {
            return Ok(false);
        }
        let groups = Self::groups(policies, &shared);
        let mut groups_of: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (k, (heads, _)) in groups.iter().enumerate() {
            for h in heads {
                groups_of.entry(h.clone()).or_default().push(k);
            }
        }
        let is_derived = |rel: &str| (derived_in(v, rel) || natives.contains(rel)) && !f.is_input(rel);
        let mut places: BTreeMap<String, (Vec<usize>, Vec<usize>, Vec<usize>)> = BTreeMap::new();
        let mut derived: BTreeSet<String> = BTreeSet::new();
        let mut note = |rel: &str| {
            if !places.contains_key(rel) {
                let dv = is_derived(rel);
                if dv {
                    derived.insert(rel.to_owned());
                }
                places.insert(rel.to_owned(), place_of(f, locations, dv, rel));
            }
        };
        note("docof");
        for &id in &last.delta.added {
            let r = v.rule(id).ok_or("an added rule is not its input's")?;
            for a in r.body().filter_map(|l| l.atom()) {
                note(v.rel_name(a.rel()));
            }
        }
        // The guards each removed rule had, taken off the last run's counts.
        let outside: Vec<&str> = groups.iter().map(|(_, o)| o.as_str()).collect();
        let guards_in = |of: &Version, r: RuleRef<'_>, o: &str| r.body().filter_map(|l| l.atom()).filter(|a| a.neg() && of.rel_name(a.rel()) == o).count();
        let mut counts: Vec<usize> = was.guarded.iter().map(|(_, k)| *k).collect();
        if counts.len() != groups.len() || was.guarded.iter().zip(&outside).any(|((o, _), g)| o != g) {
            return Ok(false);
        }
        for id in last.delta.removed.iter() {
            let (Some(src), Some(made)) = (last.input.rule(id), last.output.derived(id).next()) else { return Ok(false) };
            for (k, o) in outside.iter().enumerate() {
                counts[k] = counts[k].saturating_sub(guards_in(last.output, made, o).saturating_sub(guards_in(last.input, src, o)));
            }
        }
        let mut guards = Guards { policies, groups: &groups, groups_of, places, derived, added: vec![0; groups.len()] };
        crate::code_rule_dsl::logic::program_image::manager::rules_again(&mut &mut guards, cx, last, w)?;
        report.guarded = outside.iter().zip(counts.iter().zip(&guards.added)).map(|(o, (k, a))| ((*o).to_owned(), k + a)).collect();
        self.report = report;
        self.kept = Some((canon, shared));
        Ok(true)
    }
}

/// `scoping`'s machinery: a policy's scope and report relations, which take no guard.
fn machinery(policies: &[Policy], rel: &str) -> bool {
    let r = policies.iter().find(|p| p.own.contains(rel)).and_then(|p| rel.strip_prefix(p.prefix.as_str())).unwrap_or(rel);
    r.starts_with("rr_scope") || r.starts_with("rr_report")
}

/// Each guarded rule's guards, after the reads whose location no read of its own relations gives.
struct Guards<'a> {
    policies: &'a [Policy],
    groups: &'a [(BTreeSet<String>, String)],
    groups_of: BTreeMap<String, Vec<usize>>,
    places: BTreeMap<String, (Vec<usize>, Vec<usize>, Vec<usize>)>,
    derived: BTreeSet<String>,
    added: Vec<usize>,
}

impl RulePass for &mut Guards<'_> {
    const NAME: &'static str = "fact spaces: guards";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let Some(ks) = self.groups_of.get(v.rel_name(r.head().rel())) else { return Ok(Step::Keep) };
        // The body as it grows, group by group: each own literal, or a guard made after one.
        let mut body: Vec<Out<'_>> = r.body().map(Out::Own).collect();
        // A made literal's position stays its own literal's: guards follow the literal they guard.
        let mut any = false;
        for &k in ks {
            let (covers, outside) = (&self.groups[k].0, self.groups[k].1.as_str());
            let mut added = 0;
            body = self.guard(cx, r, body, covers, outside, &mut added);
            self.added[k] += added;
            any |= added > 0;
        }
        if !any {
            return Ok(Step::Keep);
        }
        write_rule(out, v, r, &body, None)
    }
}

impl Guards<'_> {
    /// One rule's guards, on each read whose location no read of `covers` gives, over a body some groups guarded
    /// already. `added`: the guards added.
    fn guard<'v>(&self, cx: &Cx<'v>, r: RuleRef<'v>, body: Vec<Out<'v>>, covers: &BTreeSet<String>, outside: &str, added: &mut usize) -> Vec<Out<'v>> {
        let v = cx.v();
        let empty = (Vec::new(), Vec::new(), Vec::new());
        let place = |rel: &str| self.places.get(rel).unwrap_or(&empty);
        let var = |t: Option<Tm>| if let Some(Tm::Var(x)) = t { Some(x) } else { None };
        // The body's literals, by text, for what the guards read: (relation, negated, arguments) of a plain atom.
        let atom_of = |o: &Out<'v>| -> Option<(String, bool, Vec<Tm>)> {
            match o {
                Out::Own(l) => match l.view() {
                    Lit::Atom(a) => Some((v.rel_name(a.rel()).to_owned(), a.neg(), a.args().map(|t| Tm::of(v, r, t)).collect())),
                    _ => None,
                },
                Out::Made(Made::Atom { rel, args, neg, .. }) => Some((rel.clone(), *neg, args.clone())),
                Out::Made(_) => None,
            }
        };
        let mut covered: BTreeSet<String> = BTreeSet::new();
        for (rel, neg, args) in body.iter().filter_map(atom_of) {
            if neg || !covers.contains(&rel) {
                continue;
            }
            let (docs, nodes, spans) = place(&rel);
            covered.extend(docs.iter().chain(nodes).chain(spans).filter_map(|&c| var(args.get(c).cloned())));
        }
        let mut guards: Vec<(usize, Vec<Made>)> = Vec::new();
        // The guards' variables, `rr_sd<k>`, numbered per group from 0.
        let mut fresh = 0;
        // A check of a bound document, never a relation to start from: the documents out of the scope, negated.
        let at = |d: &str| Made::Atom { rel: outside.to_owned(), args: vec![Tm::Var(d.to_owned())], neg: true, gated: false };
        let of_node = |n: &str, fresh: &mut usize| -> Vec<Made> {
            let g = format!("{}{}", super::simplify::GUARD_VAR, *fresh);
            *fresh += 1;
            vec![Made::Cmp { l: Tm::Var(g.clone()), op: CmpOp::Eq, r: Tm::Call("doc".into(), vec![Tm::Var(n.to_owned())]) }, at(&g)]
        };
        let of_span = |n: &str, fresh: &mut usize| -> Vec<Made> {
            let g = format!("{}{}", super::simplify::GUARD_VAR, *fresh);
            *fresh += 1;
            vec![Made::Atom { rel: "docof".into(), args: vec![Tm::Var(n.to_owned()), Tm::Var(g.clone())], neg: false, gated: false }, at(&g)]
        };
        for (k, o) in body.iter().enumerate() {
            if let Out::Own(l) = o
                && let Lit::Ast { document, .. } = l.view()
            {
                let d = v.text(r.var(document)).to_owned();
                if covered.insert(d.clone()) {
                    guards.push((k, vec![at(&d)]));
                }
                continue;
            }
            let Some((rel, neg, args)) = atom_of(o) else { continue };
            if neg || covers.contains(&rel) || machinery(self.policies, &rel) {
                continue;
            }
            let (docs, nodes, spans) = place(&rel);
            let dvs: Vec<String> = docs.iter().filter_map(|&c| var(args.get(c).cloned())).collect();
            let nvs: Vec<String> = nodes.iter().filter_map(|&c| var(args.get(c).cloned())).collect();
            let svs: Vec<String> = spans.iter().filter_map(|&c| var(args.get(c).cloned())).collect();
            let mut g = Vec::new();
            if !self.derived.contains(&rel) {
                // A base row is one document's: one guard, unless a location of it is covered.
                if !dvs.iter().chain(&nvs).chain(&svs).any(|x| covered.contains(x)) {
                    // By the node where there is one (a shift: the read stays a test of that node, which `simplify`
                    // inlines), else the document column, else a span's `docof`.
                    g = match (nvs.first(), dvs.first(), svs.first()) {
                        (Some(n), ..) => of_node(n, &mut fresh),
                        (None, Some(d), _) => vec![at(d)],
                        (None, None, Some(n)) => of_span(n, &mut fresh),
                        (None, None, None) => Vec::new(),
                    };
                }
                covered.extend(dvs.into_iter().chain(nvs).chain(svs));
            } else {
                // A shared relation's located columns may be of several documents: each guarded.
                for d in dvs {
                    if covered.insert(d.clone()) {
                        g.push(at(&d));
                    }
                }
                for x in nvs {
                    if covered.insert(x.clone()) {
                        g.extend(of_node(&x, &mut fresh));
                    }
                }
                for x in svs {
                    if covered.insert(x.clone()) {
                        g.extend(of_span(&x, &mut fresh));
                    }
                }
            }
            if !g.is_empty() {
                guards.push((k, g));
            }
        }
        let mut body = body;
        for (k, g) in guards.into_iter().rev() {
            *added += g.iter().filter(|l| matches!(l, Made::Atom { neg: true, .. })).count();
            body.splice(k + 1..k + 1, g.into_iter().map(Out::Made));
        }
        body
    }
}

/// The relations that are one relation under two names made one, after the transforms that run after `simplify`
/// (guest hiding's copies, the programs combined, their fact spaces): `simplify`'s congruence again
/// (`MergeIdentical`), between two drops of repeated rules (`DropRepeated`), and its decision. Hiding copied `rr_ctx…`
/// helpers and copies of copies (`rr_in_annotation_0_annotation_2_wrap`) whose rules are another's. One merge finds
/// every class at once: its congruence is the greatest, and it compares a relation's rules as a set, so a repeated rule
/// does not keep a relation from its twin. The merge's renaming can make rules repeat; they are dropped after it, and
/// make nothing more mergeable. (A loop of merges until no rule repeated was 2-3 congruences a call, 1.5 s of the dl1
/// batch's 13.8 s CPU.) `RERAIL_SAME=off` keeps them apart.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn same_relations(p: &mut crate::code_rule_dsl::logic::program_image::manager::Pipeline<'_>) -> Result<()> {
    if std::env::var("RERAIL_SAME").is_ok_and(|x| x == "off") {
        return Ok(());
    }
    let rules = p.current()?.size();
    let _span = crate::code_rule_dsl::logic::profile::span(|| format!("merge the relations that are one ({rules} rules)"));
    let (mut repeated, mut merge) = (super::merge::DropRepeated::default(), super::merge::MergeIdentical::default());
    p.program(&mut repeated).program(&mut merge).program(&mut repeated);
    if merge.merged > 0 || repeated.0 > 0 {
        let left = p.current()?.size();
        crate::code_rule_dsl::logic::profile::decision("merged after hiding and combining", || {
            format!("{} relations merged into an identical one ({} classes shared across programs), {} rules that repeat another dropped; {left} rules of {rules} left", merge.merged, merge.shared, repeated.0)
        });
    }
    Ok(())
}

/// The copies of `label`'s own guests folded into their originals (`FoldOwn`), and then the rules that repeat another
/// dropped and the relations that are one merged, as `preparation::run` merges (decision 0058): the run's edit, once
/// its language is known.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn fold_own_guests(v: std::sync::Arc<Version>, label: &str) -> Result<std::sync::Arc<Version>> {
    let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("fold own guests", v);
    let mut fold = FoldOwn { label, folded: 0 };
    p.program(&mut fold);
    if fold.folded > 0 {
        let rules = p.current()?.size();
        let (mut repeated, mut merge, mut again) = (super::merge::DropRepeated::default(), super::merge::MergeIdentical::default(), super::merge::DropRepeated::default());
        p.program(&mut repeated).program(&mut merge).program(&mut again);
        let left = p.current()?.size();
        let folded = fold.folded;
        crate::code_rule_dsl::logic::profile::decision("copies of guests in the run's language folded (decision 0058)", || {
            format!("{folded} copies and kind sets of {label} guests; then {} repeated rules dropped and {} relations merged, {rules} -> {left} rules", repeated.0 + again.0, merge.merged)
        });
    }
    for d in p.decisions() {
        crate::code_rule_dsl::logic::profile::decision(&d.what, || d.detail.clone());
    }
    Ok(p.finish()?.0)
}

/// Folds the copies hiding made for guests in the run's own language back into their originals, once the run's
/// language (`label`) is known (decision 0058). Such a guest's documents are shown (`GUEST_KEYS..HIDDEN_KEYS`, a
/// guest whose grammar's label is the run's: `guests::key`) and its kinds are the run's (`python:x` resolves as `x`),
/// so a copy specialized to it holds its original's rows: its rules differ only by the seeded column's bound, which
/// those documents are under, and by the qualified kinds. Its readers read the original, its rules go, and a
/// qualified kind of `label` is written plainly; a clone (O110) is then its rule again, dropped as a repeat by
/// `DropRepeated`.
/// `folded`: how many copies and kind sets it folded. `RERAIL_SHOWN_COPIES=on` keeps them (to measure what they cost).
pub(in crate::code_rule_dsl::logic::plan::corpus) struct FoldOwn<'a> {
    pub(in crate::code_rule_dsl::logic::plan::corpus) label: &'a str,
    pub(in crate::code_rule_dsl::logic::plan::corpus) folded: usize,
}

impl ProgramPass for FoldOwn<'_> {
    const NAME: &'static str = "fold own guests' copies";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        use crate::code_rule_dsl::facts::vtab::syntax::{RUN_VALUE_KEYS, HIDDEN_KEYS};
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let label = self.label;
        if std::env::var("RERAIL_SHOWN_COPIES").is_ok_and(|x| x == "on") {
            return Ok(());
        }
        let dialect = crate::code_rule_dsl::logic::plan::corpus::hiding::dialect_label;
        // `rr_in_<g>_<c>_…<orig>`, every `<g>` the run's own (as `Hide` names them: `g` with non-alphanumerics as `_`).
        // The run's language and its fragments (decision 0466).
        let grammars: Vec<&str> = crate::code_rule_dsl::language::fragments().iter().map(|(n, ..)| *n).chain([label]).filter(|g| dialect(g) == label).collect();
        let own_mid = |mut mid: &str| -> bool {
            if mid.is_empty() {
                return false;
            }
            while !mid.is_empty() {
                let Some(rest) = grammars.iter().find_map(|g| mid.strip_prefix(*g).and_then(|r| r.strip_prefix('_'))) else { return false };
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                let Some(rest) = rest[digits..].strip_prefix('_').filter(|_| digits > 0) else { return false };
                mid = rest;
            }
            true
        };
        let declared: BTreeSet<&str> = f.decls().iter().map(|d| v.text(d.name())).collect();
        let mut folded: HashMap<String, String> = HashMap::new();
        for rel in declared.iter().filter(|r| r.starts_with("rr_in_")) {
            let tail = &rel["rr_in_".len()..];
            // The longest original that leaves an own prefix.
            let orig = (0..tail.len()).filter(|&i| tail.is_char_boundary(i)).map(|i| (&tail[..i], &tail[i..])).find(|(mid, orig)| !orig.starts_with("rr_in_") && declared.contains(*orig) && own_mid(mid));
            if let Some((_, orig)) = orig {
                folded.insert((*rel).to_owned(), orig.to_owned());
            }
        }
        let set_names: BTreeSet<&str> = f.kindsets().iter().map(|(s, _)| v.text(*s)).collect();
        let sets: HashMap<String, String> = set_names
            .iter()
            .filter_map(|k| {
                let tail = k.strip_prefix("rr_in_")?;
                grammars.iter().find_map(|g| tail.strip_prefix(*g).and_then(|r| r.strip_prefix('_'))).filter(|set| set_names.contains(*set)).map(|set| ((*k).to_owned(), set.to_owned()))
            })
            .collect();
        // A tree axis over a copied set (`Hide`'s `rr_inside_rr_in_<g>_<set>`, `rr_has_…`) reads the original set's
        // axis: the copied set goes, and an axis left on it names no set, so the transducer made it with no rows and
        // its negation held for every node (`!inside(t, _, S)` and `within_stop` in a clone over an annotation's node;
        // main's #862, ported from the tree's `fold_own`).
        let axes: BTreeMap<String, String> = declared
            .iter()
            .copied()
            .chain(f.inputs())
            .filter_map(|rel| {
                let (set, has) = front::axis(rel)?;
                sets.get(set).map(|orig| (rel.to_owned(), format!("{}{orig}", if has { front::DESCENDANTS } else { front::AXIS })))
            })
            .collect();
        if folded.is_empty() && sets.is_empty() {
            return Ok(());
        }
        // An original holds its copies' rows: the bounds of an annotation's take the annotations in (`RUN_VALUE_KEYS`
        // to `HIDDEN_KEYS`), as its copies' seeded columns did. A copy for a code guest of the run's language holds rows
        // under the narrow bound already. A rule that does not reach an annotation keeps the bound.
        let originals = annotated(&folded);
        let narrowing = Narrowing { widened: &originals, asking: asking(v) };
        let read = narrowing.reads(v, &folded);
        let (narrow, wide) = (i64::from(RUN_VALUE_KEYS), i64::from(HIDDEN_KEYS));
        let within = one_document(v, &f, &read, &narrowing, &folded, [narrow, narrow << 32])?;
        // Each such copy's columns and the node columns it bounds.
        let filters: Vec<(String, usize, Vec<usize>)> = within
            .iter()
            .filter_map(|rel| {
                let cols = f.columns(rel)?;
                Some((rel.clone(), cols.len(), (0..cols.len()).filter(|&c| cols[c].1 == "node").collect()))
            })
            .collect();
        let plain = format!("{label}:");
        // The guests' requests (`guests::REQUESTS`; their answers are written later), and any a run answers by its
        // inputs (`plan::requests_first`): the strata in their cones run before them.
        let requests: BTreeSet<String> = crate::code_rule_dsl::logic::plan::corpus::guests::REQUESTS
            .iter()
            .filter(|q| v.rel(q).is_some())
            .map(|q| (*q).to_owned())
            .chain(f.answers().iter().filter(|&&(_, by)| f.is_input(v.rel_name(by))).map(|&(q, _)| v.rel_name(q).to_owned()))
            .collect();
        // A copy within one document is its original's rows under the bound (`one_document`); the rest diffed where
        // they can be (`Diffed`), reading the former as change points too; the others derived again.
        let diffed = Diffed::new(v, &folded, &read, &within, &narrowing, &requests, narrow);
        let twins = twins(v, &folded);
        let mut rules = Folding { twins: &twins, folded: &folded, axes: &axes, sets: &sets, originals: &originals, narrowing: &narrowing, read: &read, within: &within, filters, diffed: &diffed, narrow, wide, plain: &plain };
        w.derive()?;
        w.apply(cx, &mut rules, &crate::code_rule_dsl::logic::program_image::manager::Select::All)?;
        // The fields edited in the same derivation.
        let b = w.derive()?;
        for rel in folded.keys() {
            b.fields().undeclare(rel);
        }
        // Each axis copy is declared as its original, where its original was not.
        for (copy, orig) in &axes {
            if let Some(cols) = v.columns_of(copy) {
                if !v.declares(orig) {
                    let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
                    b.fields().declare(orig, &pairs);
                }
                b.fields().undeclare(copy);
            }
        }
        let named = |b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, c: Cell, gone: &dyn Fn(&str) -> bool| matches!(c, Cell::Str(a) if gone(b.text(a)));
        let mut qualifiers: Vec<Vec<Cell>> = b.about_rows("qualifier").into_iter().filter(|r| !named(b, r[1], &|x| folded.contains_key(x))).collect();
        let narrowed: Vec<Vec<Cell>> = qualifiers
            .iter()
            .filter_map(|r| match r[1] {
                Cell::Str(a) if read.contains(b.text(a)) => Some((r.clone(), narrow_name(b.text(a)))),
                _ => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(mut row, name)| {
                row[1] = Cell::Str(b.atom(&name));
                row
            })
            .collect();
        qualifiers.extend(narrowed);
        b.fields().set_about_rows("qualifier", qualifiers);
        declare_narrow(v, b, &read, &within, &narrowing)?;
        diffed.declare(b);
        if !diffed.of.is_empty() {
            crate::code_rule_dsl::logic::profile::decision("a narrow copy as its original less the annotations' change (decision 0165 §3)", || {
                format!("{} of {} copies diffed ({} within one document): {}", diffed.of.len(), read.len(), within.len(), diffed.of.iter().cloned().collect::<Vec<_>>().join(", "))
            });
        }
        // The request record's readers (`hiding::fold_own`'s `rename_readers`, decision 0331).
        b.rename_asked_readers(&|r| folded.get(r).cloned());
        for about in ["kindset", "kindset_kind"] {
            let rows: Vec<Vec<Cell>> = b.about_rows(about).into_iter().filter(|r| !named(b, r[1], &|x| sets.contains_key(x))).collect();
            b.fields().set_about_rows(about, rows);
        }
        // A list names the originals; an axis copy's place is its original's, unless the original is listed already.
        let renamed = |list: Vec<String>| -> Vec<String> {
            let mut list: Vec<String> = list.into_iter().map(|r| folded.get(&r).cloned().unwrap_or(r)).collect();
            for (copy, orig) in &axes {
                let Some(i) = list.iter().position(|r| r == copy) else { continue };
                if list.contains(orig) {
                    list.remove(i);
                } else {
                    list[i] = orig.clone();
                }
            }
            list
        };
        let moved = |r: &str| folded.contains_key(r) || axes.contains_key(r);
        let inputs = renamed(f.inputs().map(str::to_owned).collect());
        let outputs = renamed(f.outputs().map(str::to_owned).collect());
        if f.inputs().any(moved) {
            b.fields().set_inputs(&inputs);
        }
        if f.outputs().any(moved) {
            b.fields().set_outputs(&outputs);
        }
        if f.demand().any(|r| moved(r) || read.contains(r)) {
            let mut demand = renamed(f.demand().map(str::to_owned).collect());
            demand.extend(f.demand().filter(|r| read.contains(*r)).map(narrow_name));
            let p = super::demand::about_p(b);
            let rows: Vec<Vec<Cell>> = demand.iter().enumerate().map(|(k, r)| vec![p, Cell::Int(k as i64), Cell::Str(b.atom(r))]).collect();
            b.fields().set_about_rows("demand", rows);
        }
        // The request record's readers renamed as the reads are (`Program::rename_readers`): a request of a folded
        // copy serves its original (decision 0331).
        let was = b.asked();
        let mut asked = was.clone();
        for readers in asked.values_mut().flatten() {
            if readers.iter().any(|x| folded.contains_key(x)) {
                *readers = std::mem::take(readers).into_iter().map(|x| folded.get(&x).cloned().unwrap_or(x)).collect();
            }
            // A request serving an original kept narrow serves its narrow copy's readers too.
            let narrowed: Vec<String> = readers.iter().filter(|x| read.contains(*x)).map(|x| narrow_name(x)).collect();
            readers.extend(narrowed);
        }
        if asked != was {
            b.fields().set_asked(&asked);
        }
        self.folded += folded.len() + sets.len();
        Ok(())
    }
}

/// `FoldOwn`'s edit of each rule.
struct Folding<'a> {
    /// A rule's wide twins (`twin_key`): rules of the same head that read an annotation's copy where it reads the
    /// original, by their text with each copy named as its original; each with those literals so named.
    twins: &'a HashMap<String, Vec<BTreeSet<String>>>,
    folded: &'a HashMap<String, String>,
    /// Each tree axis over a copied kind set, and the original set's axis.
    axes: &'a BTreeMap<String, String>,
    sets: &'a HashMap<String, String>,
    /// The originals widened: an annotation's (`annotated`).
    originals: &'a BTreeSet<String>,
    narrowing: &'a Narrowing<'a>,
    /// The originals kept narrow for the rules that read them narrow (`Narrowing::reads`).
    read: &'a BTreeSet<String>,
    /// Those whose narrow copy is their rows under the narrow bound (`one_document`), not their rules again.
    within: &'a BTreeSet<String>,
    /// Each of `within`'s columns' count, and its node columns, bounded in its copy.
    filters: Vec<(String, usize, Vec<usize>)>,
    /// Those whose narrow copy is its original less what the annotations change (`Diffed`): the rest are derived
    /// again under the narrow bound.
    diffed: &'a Diffed,
    narrow: i64,
    wide: i64,
    plain: &'a str,
}

impl RulePass for Folding<'_> {
    const NAME: &'static str = "fold own guests' copies: rules";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step> {
        let v = cx.v();
        let head = v.rel_name(r.head().rel());
        if self.folded.contains_key(head) {
            return Ok(Step::Drop);
        }
        let sees = self.narrowing.sees(head);
        let mut rw = Fold { v, at: self, original: self.originals.contains(head), narrow_reads: sees, head: None, changed: false, plain: HashMap::new(), ids: HashMap::new(), renames: Vec::new(), atoms: 0 };
        // Asked once to find whether anything changes; the rule is kept as it is where nothing does.
        for l in r.body() {
            match l.view() {
                Lit::Atom(a) => rw.look(a),
                Lit::Agg { atom, .. } => rw.look(atom),
                Lit::Cmp { op: CmpOp::Lt, l: left, r: TermRef::Int(b) } if rw.original && rw.widened(b).is_some() => {
                    debug_assert!(!has_int(left, b), "fold_own: a bound compared with itself");
                    rw.changed = true
                }
                Lit::Nearest { kindset, .. } if self.sets.contains_key(v.text(kindset)) => rw.changed = true,
                _ => {}
            }
        }
        let copy = self.read.contains(head) && !self.within.contains(head);
        if !rw.changed && !copy {
            return Ok(Step::Keep);
        }
        // The plain names, as the builder's atoms.
        let names: Vec<(AtomId, String)> = rw.plain.iter().map(|(a, n)| (*a, n.clone())).collect();
        for (a, n) in names {
            let id = out.builder().atom(&n);
            rw.plain.insert(a, n);
            rw.ids.insert(a, id);
        }
        if !copy && sees && !rw.original
            && let Some(step) = self.beside_twin(v, r, head, &rw, out)?
        {
            return Ok(step);
        }
        let step = out.rewrite(&mut rw)?;
        if copy && self.diffed.of.contains(head) {
            self.diffed.write(v, r, head, &rw, out)?;
        } else if copy {
            // As hiding left it (decision 0165 §3), for the rules that read it narrow: not widened, reading the others'
            // narrow copies.
            let mut narrow = Fold { original: false, narrow_reads: true, head: Some(narrow_name(head)), changed: true, plain: rw.plain.clone(), ids: rw.ids.clone(), ..rw };
            out.rewrite(&mut narrow)?;
        }
        Ok(step)
    }
    /// The narrow copy of each original within one document (`one_document`): its rows whose nodes are under the narrow
    /// bound, `rr_narrow_<orig>(c0, …) :- <orig>(c0, …), $c < RUN_VALUE_KEYS << 32` for each node column `c`.
    fn finish(&mut self, _cx: &Cx<'_>, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>) -> Result<()> {
        for (rel, width, nodes) in &self.filters {
            let names: Vec<String> = (0..*width).map(|c| format!("rr_c{c}")).collect();
            let args: Vec<T<'_>> = names.iter().map(|n| T::Var(n)).collect();
            let head = narrow_name(rel);
            let mut rule = b.rule().head(head.as_str(), &args).atom(rel.as_str(), &args);
            for &c in nodes {
                rule = rule.cmp(T::Var(&names[c]), CmpOp::Lt, T::Int(self.narrow << 32));
            }
            rule.finish()?;
        }
        self.diffed.finish(b, self.narrow, self.wide)
    }
}

impl<'a> Folding<'a> {
    /// Rule `r` of `head` (not widened, reading narrow) beside a wide twin: a rule of the same head that reads an
    /// annotation's copy (folded, so the original whole) where `r` reads the original (so its narrow copy), and is `r`
    /// otherwise. A row `r` derives that the twin does not reads, at one of those places, a row the narrow copy holds
    /// and the original does not (`rr_added_`), or for a negation one the original holds and the copy does not
    /// (`rr_gone_`). So `r` is written once a place, that place read as its delta: the deltas' rows, not the tree's.
    /// Where the places' originals are not all diffed, or one reaches `head` (its delta's negation would close a
    /// cycle), `None`: written whole.
    fn beside_twin(&self, v: &Version, r: RuleRef<'_>, head: &str, rw: &Fold<'a, '_>, out: &mut RuleOut<'_, '_>) -> Result<Option<Step>> {
        let Some(sets) = self.twins.get(&twin_key(v, r, &|n| n.to_owned())) else { return Ok(None) };
        let mut places: Vec<(usize, String)> = Vec::new();
        let mut at = 0;
        for l in r.body() {
            let (a, agg) = match l.view() {
                Lit::Atom(a) => (a, false),
                Lit::Agg { atom, .. } => (atom, true),
                _ => continue,
            };
            let rel = v.rel_name(a.rel());
            let text = v.show_lit(r, l);
            if self.read.contains(rel) && sets.iter().any(|t| t.contains(&text)) {
                if agg || !self.diffed.of.contains(rel) || self.diffed.reaches(rel, head) || self.diffed.reaches(&narrow_name(rel), head) {
                    return Ok(None);
                }
                places.push((at, if a.neg() { gone_name(rel) } else { added_name(rel) }));
            }
            at += 1;
        }
        if places.is_empty() {
            return Ok(Some(Step::Drop));
        }
        let mut step = Step::Drop;
        for (i, (place, delta)) in places.into_iter().enumerate() {
            let mut f = Fold { v, at: rw.at, original: false, narrow_reads: true, head: None, changed: true, plain: rw.plain.clone(), ids: rw.ids.clone(), renames: vec![(place, delta)], atoms: 0 };
            let s = out.rewrite(&mut f)?;
            if i == 0 {
                step = s;
            }
        }
        Ok(Some(step))
    }
}

/// A rule's text with each relation named by `name`, its body's literals in order: the key of its wide twins.
fn twin_key(v: &Version, r: RuleRef<'_>, name: &dyn Fn(&str) -> String) -> String {
    let head = rename_rels(&v.show_atom(r, r.head()), name);
    let mut lits: Vec<String> = r.body().map(|l| rename_rels(&v.show_lit(r, l), name)).collect();
    lits.sort_unstable();
    format!("{head} :- {}", lits.join(", "))
}

/// Each rule that reads an annotation's copy (`folded`), by its text with the copies named as their originals
/// (`twin_key`), and its literals that read one, so named.
fn twins(v: &Version, folded: &HashMap<String, String>) -> HashMap<String, Vec<BTreeSet<String>>> {
    let name = |n: &str| folded.get(n).cloned().unwrap_or_else(|| n.to_owned());
    let mut out: HashMap<String, Vec<BTreeSet<String>>> = HashMap::new();
    for r in v.rules() {
        if folded.contains_key(v.rel_name(r.head().rel())) {
            continue;
        }
        let lits: BTreeSet<String> = r
            .body()
            .filter(|l| l.atom().is_some_and(|a| folded.contains_key(v.rel_name(a.rel()))))
            .map(|l| rename_rels(&v.show_lit(r, l), &name))
            .collect();
        if !lits.is_empty() {
            out.entry(twin_key(v, r, &name)).or_default().push(lits);
        }
    }
    out
}

/// A rule's or literal's text with each relation's name (an identifier outside a string, not a variable) `name`d.
fn rename_rels(text: &str, name: &dyn Fn(&str) -> String) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let j = (j + 1).min(b.len());
            out.push_str(&text[i..j]);
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            let word = &text[i..j];
            if i > 0 && matches!(b[i - 1], b'$' | b'#' | b'@') {
                out.push_str(word);
            } else {
                out.push_str(&name(word));
            }
            i = j;
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    out
}

/// The name of `orig` as hiding left it, for the rules that read it and are not its own (`Narrowing`).
fn narrow_name(orig: &str) -> String {
    format!("rr_narrow_{orig}")
}

/// The originals of copies made for an annotation (`rr_in_…annotation_<c>_…<orig>`). A copy for a code guest of the
/// run's language holds rows under the narrow bound already; one for an annotation holds an annotation's, which its
/// original takes in once widened.
fn annotated(folded: &HashMap<String, String>) -> BTreeSet<String> {
    let for_annotation = |copy: &str, orig: &str| {
        let mid = &copy["rr_in_".len()..copy.len() - orig.len()];
        mid.starts_with("annotation_") || mid.contains("_annotation_")
    };
    folded.iter().filter(|(copy, orig)| for_annotation(copy, orig)).map(|(_, orig)| orig.clone()).collect()
}

/// The originals kept narrow (`read`) whose narrow copy is the widened original's rows under the narrow bound, not its
/// rules made again. Every row of such an original is of one document and made from that document's rows alone: for
/// each rule,
/// - its head has a node column, and every location it holds is of one document, joined through positive atoms whose
///   rows are each of one document (a base relation's locations, `hide::located`, but a guest's, which join documents;
///   a relation found so; `nearest`; an equality);
/// - each bound of its own (`$x < RUN_VALUE_KEYS`, a node's or a document's) is of that document;
/// - each read of an original kept narrow (positive, negated or aggregated) is of one found so, at a location of that
///   document.
///
/// Widened, such a rule makes in a document under the narrow bound the rows it made there before: its bounds hold
/// there, the originals it reads hold there what they held, and every other relation is the same in both. Its copy
/// holds no row of a document over the bound, which a rule that reads no annotation would not see alone. A row joined
/// to another document by a value (`pair(n) :- e(n), text(n, t), asg(m), text(m, t)`), or bounded in another one, is
/// made again (#923's RUF060 beside C417); so is one whose original would read its own copy once the fold is done (a
/// cycle through the copy), and a native operator's, a lattice's or one with facts, which its copy repeats
/// (`declare_narrow`).
fn one_document(v: &Version, f: &Fields<'_>, read: &BTreeSet<String>, narrowing: &Narrowing<'_>, folded: &HashMap<String, String>, bounds: [i64; 2]) -> Result<BTreeSet<String>> {
    if read.is_empty() {
        return Ok(BTreeSet::new());
    }
    let natives = v.native_map()?;
    let lattice = v.lattice_decls()?;
    let (_, base) = super::hide::bases(v);
    let name = |a: AtomRef<'_>| -> &str {
        let r = v.rel_name(a.rel());
        folded.get(r).map_or(r, String::as_str)
    };
    let mut rules_of: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        if !folded.contains_key(head) {
            rules_of.entry(head).or_default().push(r);
        }
    }
    let node = |rel: &str, c: usize| f.column_type(rel, c) == Some("node");
    let place = |rel: &str, c: usize| matches!(f.column_type(rel, c), Some("node" | "loc"));
    let var = |t: TermRef<'_>| match t {
        TermRef::Var(s) => Some(s.0 as usize),
        _ => None,
    };
    fn find(uf: &mut [usize], x: usize) -> usize {
        let mut x = x;
        while uf[x] != x {
            uf[x] = uf[uf[x]];
            x = uf[x];
        }
        x
    }
    fn join(uf: &mut [usize], vars: &[usize]) {
        for w in vars.windows(2) {
            let (x, y) = (find(uf, w[0]), find(uf, w[1]));
            uf[x] = y;
        }
    }
    // Whether rule `r` keeps each row in one document (and, `narrowed`, makes it from that document's rows, bounded
    // there), where the relations in `ok` keep theirs.
    let holds = |r: RuleRef<'_>, narrowed: bool, ok: &BTreeSet<&str>| -> bool {
        let mut uf: Vec<usize> = (0..r.var_count()).collect();
        for l in r.body() {
            match l.view() {
                Lit::Atom(a) if !a.neg() => {
                    let rel = name(a);
                    let args: Vec<TermRef<'_>> = a.args().collect();
                    let vars: Vec<usize> = if rules_of.contains_key(rel) {
                        if !ok.contains(rel) {
                            continue;
                        }
                        (0..args.len()).filter(|&c| place(rel, c)).filter_map(|c| var(args[c])).collect()
                    } else {
                        if super::super::guests::READS.contains(&rel) || natives.contains_key(rel) {
                            continue;
                        }
                        let (cols, document) = super::hide::located(rel, &base);
                        cols.iter().copied().chain(document).filter_map(|c| args.get(c).copied().and_then(var)).collect()
                    };
                    join(&mut uf, &vars);
                }
                Lit::Nearest { n, a, .. } => join(&mut uf, &[n.0 as usize, a.0 as usize]),
                Lit::Cmp { op: CmpOp::Eq, l: TermRef::Var(x), r: TermRef::Var(y) } => join(&mut uf, &[x.0 as usize, y.0 as usize]),
                _ => {}
            }
        }
        let head = r.head();
        let hrel = v.rel_name(head.rel());
        // A node column to bound its copy's rows by; every location it holds of one document.
        if !(0..head.args().count()).any(|c| node(hrel, c)) {
            return false;
        }
        let mut doc = None;
        for (_, t) in head.args().enumerate().filter(|&(c, _)| place(hrel, c)) {
            let Some(x) = var(t) else { return false };
            let x = find(&mut uf, x);
            if doc.is_some_and(|d| d != x) {
                return false;
            }
            doc = Some(x);
        }
        let Some(doc) = doc else { return false };
        if !narrowed {
            return true;
        }
        r.body().all(|l| match l.view() {
            Lit::Cmp { op: CmpOp::Lt, l: left, r: TermRef::Int(b) } if bounds.contains(&b) => var(left).is_some_and(|x| find(&mut uf, x) == doc),
            Lit::Atom(a) | Lit::Agg { atom: a, .. } => {
                let rel = name(a);
                !read.contains(rel) || ok.contains(rel) && a.args().enumerate().any(|(c, t)| place(rel, c) && var(t).is_some_and(|x| find(&mut uf, x) == doc))
            }
            _ => true,
        })
    };
    // The greatest set: each relation is taken to keep its rows in one document until one of its rules does not. A
    // rule holds over fewer relations no more often, so a relation that leaves the set rechecks only its readers.
    let mut readers: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for (head, rs) in &rules_of {
        for r in rs {
            for l in r.body() {
                if let Lit::Atom(a) | Lit::Agg { atom: a, .. } = l.view() {
                    readers.entry(name(a)).or_default().insert(*head);
                }
            }
        }
    }
    let mut ok: BTreeSet<&str> = rules_of.keys().copied().filter(|r| !natives.contains_key(*r)).collect();
    let mut queue: BTreeSet<&str> = ok.clone();
    while let Some(rel) = queue.pop_first() {
        if !ok.contains(rel) || rules_of[rel].iter().all(|&r| holds(r, read.contains(rel), &ok)) {
            continue;
        }
        ok.remove(rel);
        queue.extend(readers.get(rel).into_iter().flatten().copied().filter(|h| ok.contains(h)));
    }
    let mut within: BTreeSet<&str> = read.iter().map(String::as_str).filter(|rel| ok.contains(rel) && !lattice.contains_key(*rel) && v.fact_values(rel, &Default::default()).is_none()).collect();
    // The program once folded: each rule's reads (a narrow copy's where it reads narrow, `Narrowing::sees`; a native
    // operator's as `declare_narrow` renames them), and each narrow copy's: its original's where it is within one
    // document, else its original's rules made narrow. A copy in a cycle with its original is made again.
    let reads_of = |rel: &str| -> Vec<&str> {
        let mut out: Vec<&str> = rules_of.get(rel).into_iter().flatten().flat_map(|r| r.body().filter_map(|l| l.atom()).map(name)).collect();
        if let Some(n) = natives.get(rel) {
            out.extend(n.reads().iter().map(|r| folded.get(*r).map_or(*r, String::as_str)));
        }
        out
    };
    let narrowed = |r: &str| if read.contains(r) { narrow_name(r) } else { r.to_owned() };
    let mut nodes: BTreeSet<String> = rules_of.keys().map(|r| (*r).to_owned()).chain(natives.keys().cloned()).collect();
    nodes.extend(read.iter().map(|r| narrow_name(r)));
    let edges_of = |n: &str, within: &BTreeSet<&str>| -> BTreeSet<(String, bool)> {
        let to: BTreeSet<(String, bool)> = match n.strip_prefix("rr_narrow_").filter(|o| read.contains(*o)) {
            Some(orig) if within.contains(orig) => BTreeSet::from([(orig.to_owned(), false)]),
            Some(orig) => reads_of(orig).into_iter().map(|r| (narrowed(r), false)).collect(),
            None if narrowing.sees(n) => reads_of(n).into_iter().map(|r| (narrowed(r), false)).collect(),
            None => reads_of(n).into_iter().map(|r| (r.to_owned(), false)).collect(),
        };
        to.into_iter().filter(|(t, _)| nodes.contains(t)).collect()
    };
    // Only a narrow copy's edges change as `within` shrinks: they are made again for the copies that leave it.
    let mut edges: front::Edges = nodes.iter().map(|n| (n.clone(), edges_of(n, &within))).collect();
    loop {
        let comps = front::tarjan(&nodes, &edges);
        let comp_of: HashMap<&str, usize> = comps.iter().enumerate().flat_map(|(i, c)| c.iter().map(move |r| (r.as_str(), i))).collect();
        let cyclic: Vec<&str> = within.iter().copied().filter(|r| comp_of.get(*r).is_some_and(|c| comp_of.get(narrow_name(r).as_str()) == Some(c))).collect();
        if cyclic.is_empty() {
            break;
        }
        for r in cyclic {
            within.remove(r);
            let copy = narrow_name(r);
            let to = edges_of(&copy, &within);
            edges.insert(copy, to);
        }
    }
    Ok(within.into_iter().map(str::to_owned).collect())
}

/// The guest demand and the contexts only it reads, transitively (`hide::asking`, over the version).
fn asking(v: &Version) -> BTreeSet<String> {
    let mut readers: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        for a in r.body().filter_map(|l| l.atom()) {
            readers.entry(v.rel_name(a.rel())).or_default().insert(head);
        }
    }
    let mut asking: BTreeSet<String> = BTreeSet::from([crate::code_rule_dsl::logic::plan::corpus::guests::DEMAND.to_owned()]);
    loop {
        let more: Vec<String> = v
            .declared_names()
            .into_iter()
            .filter(|r| super::hide::is_context(r) && !asking.contains(r.as_str()))
            .filter(|r| readers.get(r.as_str()).is_some_and(|by| by.iter().all(|h| asking.contains(*h))))
            .collect();
        if more.is_empty() {
            return asking;
        }
        asking.extend(more);
    }
}

/// Which rules read the widened originals narrow (decision 0165 §3). Such a rule reached no annotation (it would
/// read a copy), so it reads the original under the narrow bound, `rr_narrow_<orig>`: the original's rules before
/// they are widened, each reading the others' narrow ones too. Without it a rule's rows depended on what else the run
/// held: a program whose rules read a copy widened the original another program's rule read, and that rule saw the
/// annotations (RUF060 found `x: "1 in []"` beside C417 only; main's #923).
struct Narrowing<'w> {
    widened: &'w BTreeSet<String>,
    /// The guest demand and the contexts only it reads (`asking`).
    asking: BTreeSet<String>,
}

impl Narrowing<'_> {
    /// Whether the rules of `head` read the originals narrow: not a widened original's own, and not asking for what
    /// every reader is answered from.
    fn sees(&self, head: &str) -> bool {
        !self.widened.contains(head) && !self.asks(head)
    }

    /// Whether `rel` is kept narrow for the rules that read it narrow.
    fn narrowed(&self, rel: &str) -> bool {
        self.widened.contains(rel) && !self.asks(rel)
    }

    /// Whether `rel` asks for what every reader is answered from, so it reads the originals widened: a guest request
    /// and the contexts only it reads (`guests::REQUESTS`, `asking`; asking is not seeing, decision 0165 §4), or the
    /// on-request rewrite's attributes, their requests, eager halves and widened copies (decision 0174). Read narrow, a
    /// request asks too little for the widened original's readers.
    fn asks(&self, rel: &str) -> bool {
        use crate::code_rule_dsl::logic::rr::lower::{made_on_request, on_request_rule};
        self.asking.contains(rel) || crate::code_rule_dsl::logic::plan::corpus::guests::REQUESTS.contains(&rel) || on_request_rule(rel) || made_on_request(rel)
    }

    /// The originals kept narrow that a rule reading them narrow reads, and those their rules read in turn. The
    /// folded copies' rules go, and are not followed.
    fn reads(&self, v: &Version, folded: &HashMap<String, String>) -> BTreeSet<String> {
        let mut rules_of: HashMap<&str, Vec<RuleRef<'_>>> = HashMap::new();
        for r in v.rules() {
            rules_of.entry(v.rel_name(r.head().rel())).or_default().push(r);
        }
        let mut todo: Vec<RuleRef<'_>> = v
            .rules()
            .filter(|r| {
                let head = v.rel_name(r.head().rel());
                !folded.contains_key(head) && self.sees(head)
            })
            .collect();
        let mut read = BTreeSet::new();
        while let Some(rule) = todo.pop() {
            for l in rule.body() {
                let (Lit::Atom(a) | Lit::Agg { atom: a, .. }) = l.view() else { continue };
                let rel = v.rel_name(a.rel());
                if self.narrowed(rel) && read.insert(rel.to_owned()) {
                    todo.extend(rules_of.get(rel).into_iter().flatten().copied());
                }
            }
        }
        read
    }
}

/// What the program says of each original kept narrow (`read`) by name, said of `rr_narrow_<orig>` too: its
/// declaration, facts, tape columns, lattice, nullary encoding, join shapes and native operator (its qualifiers, demand
/// and the request rules that serve its readers are `FoldOwn`'s). A native operator whose rule reads narrow reads the
/// narrow copies: its rows are made from the relations its fields name, its rule only orders it after them.
fn declare_narrow(v: &Version, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, read: &BTreeSet<String>, within: &BTreeSet<String>, narrowing: &Narrowing<'_>) -> Result<()> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    if read.is_empty() {
        return Ok(());
    }
    for orig in read {
        if let Some(cols) = v.columns_of(orig) {
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            b.fields().declare(&narrow_name(orig), &pairs);
        }
        if let Some(rows) = v.fact_values(orig, &Default::default()) {
            b.fields().set_facts(&narrow_name(orig), &rows);
        }
    }
    let copy = |name: &str| read.contains(name).then(|| narrow_name(name));
    let mut tapes = v.tape_columns()?;
    let extra: Vec<_> = tapes.iter().filter_map(|(r, c)| copy(r).map(|n| (n, c.clone()))).collect();
    if !extra.is_empty() {
        tapes.extend(extra);
        b.fields().set_field(Field::Tapes(&tapes));
    }
    let mut lattice = v.lattice_decls()?;
    let extra: Vec<_> = lattice.iter().filter_map(|(r, l)| copy(r).map(|n| (n, l.clone()))).collect();
    if !extra.is_empty() {
        lattice.extend(extra);
        b.fields().set_field(Field::Lattice(&lattice));
    }
    let nullary: Vec<String> = v.nullary_names()?.iter().filter_map(|r| copy(r)).collect();
    if !nullary.is_empty() {
        b.fields().add_nullary(&nullary);
    }
    let mut shapes = v.shape_list()?;
    // A copy within one document reads its original (`one_document`): its rule is not the original's.
    let extra: Vec<_> = shapes.iter().filter(|s| !within.contains(&s.head)).filter_map(|s| copy(&s.head).map(|n| front::Shape { head: n, ..s.clone() })).collect();
    if !extra.is_empty() {
        shapes.extend(extra);
        b.fields().set_field(Field::Shapes(&shapes));
    }
    let mut natives = v.native_map()?;
    let mut changed = false;
    for (_, native) in natives.iter_mut().filter(|(head, _)| narrowing.sees(head)) {
        if native.reads().iter().any(|r| read.contains(*r)) {
            native.rename_reads(|r| if read.contains(r) { narrow_name(r) } else { r.to_owned() });
            changed = true;
        }
    }
    let extra: Vec<_> = natives.iter().filter_map(|(r, n)| copy(r).map(|c| (c, n.clone()))).collect();
    changed |= !extra.is_empty();
    natives.extend(extra);
    if changed {
        b.fields().set_field(Field::Natives(&natives));
    }
    Ok(())
}

struct Fold<'a, 'v> {
    v: &'v Version,
    at: &'a Folding<'a>,
    original: bool,
    /// Whether its reads of the originals kept narrow read their narrow copies (`Narrowing::sees`).
    narrow_reads: bool,
    /// The head it is written under: a narrow copy's (`narrow_name`); else its own.
    head: Option<String>,
    changed: bool,
    /// Each qualified kind or field constant, by its atom, and its plain name.
    plain: HashMap<AtomId, String>,
    /// The plain names as the builder's atoms.
    ids: HashMap<AtomId, AtomId>,
    /// Atoms written as other relations (`Diffed`), positive: by the atom's place among the body's atoms (its atoms
    /// and aggregates' atoms, in order). A change point read as its delta, or a recursive read as the candidates.
    renames: Vec<(usize, String)>,
    /// The atoms written so far (`renames`).
    atoms: usize,
}

/// Whether term `t` is, or holds, the int `b`.
fn has_int(t: TermRef<'_>, b: i64) -> bool {
    match t {
        TermRef::Int(x) => x == b,
        TermRef::Call(_, args) => args.into_iter().any(|a| has_int(a, b)),
        _ => false,
    }
}

impl Fold<'_, '_> {
    /// Column `c` of relation `rel`, a kind or field constant `unqualify` writes plainly.
    fn named_column(rel: &str, c: usize) -> bool {
        matches!((rel, c), ("kind" | "field", 1) | ("node", 2))
    }
    fn look(&mut self, a: AtomRef<'_>) {
        let rel = self.v.rel_name(a.rel());
        if self.at.folded.contains_key(rel) || self.at.axes.contains_key(rel) || self.narrow_reads && self.at.read.contains(rel) {
            self.changed = true;
        }
        for (c, t) in a.args().enumerate() {
            if let TermRef::Str(s) = t
                && Self::named_column(rel, c)
                && let Some(name) = self.v.text(s).strip_prefix(self.at.plain)
            {
                self.plain.insert(s, name.to_owned());
                self.changed = true;
            }
        }
    }
    fn widened(&self, b: i64) -> Option<i64> {
        if b == self.at.narrow << 32 {
            Some(self.at.wide << 32)
        } else if b == self.at.narrow {
            Some(self.at.wide)
        } else {
            None
        }
    }
}

impl Rewrite for Fold<'_, '_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        let rel = self.v.rel_name(a.rel());
        let at = self.atoms;
        self.atoms += 1;
        if let Some((_, name)) = self.renames.iter().find(|(p, _)| *p == at) {
            return Some((name.clone(), false, a.gated()));
        }
        if self.narrow_reads && self.at.read.contains(rel) {
            return Some((narrow_name(rel), a.neg(), a.gated()));
        }
        self.at.folded.get(rel).or_else(|| self.at.axes.get(rel)).map(|o| (o.clone(), a.neg(), a.gated()))
    }
    fn head_as(&mut self, _a: AtomRef<'_>) -> Option<String> {
        self.head.clone()
    }
    fn arg(&mut self, a: AtomRef<'_>, c: usize, t: TermRef<'_>) -> Option<T<'static>> {
        let TermRef::Str(s) = t else { return None };
        if !Self::named_column(self.v.rel_name(a.rel()), c) {
            return None;
        }
        self.ids.get(&s).map(|&a| T::StrAtom(a))
    }
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        let Site::Cmp(l) = site else { return None };
        let Lit::Cmp { op: CmpOp::Lt, l: left, r: TermRef::Int(b) } = l.view() else { return None };
        // The right operand: the left holds no such int (`look` checks).
        match t {
            TermRef::Int(x) if self.original && x == b && !has_int(left, b) => self.widened(b).map(T::Int),
            _ => None,
        }
    }
    fn kindset(&mut self, set: AtomId) -> Option<String> {
        self.at.sets.get(self.v.text(set)).cloned()
    }
}

/// Each original's narrow copy written as the original less what the annotations change (decision 0165 §3), where it
/// can be: not derived again over the whole tree. The narrow rules differ from the wide ones at their change points: a
/// bound (an annotation's node or document the wide rule lets in), and a read of another narrow copy (its rows not its
/// original's). A row of the two can differ only where some derivation goes through a change point, so
/// - `rr_cand_<orig>` holds the heads of the derivations through one: each wide rule with one change point read as
///   its delta (an annotation's node or document; a read copy's rows gone, or a negated one's added), the rest as the
///   original reads them; and each narrow rule with one read copy read as its delta the other way (its rows added, or
///   a negated one's gone), the rest as the copy reads them;
/// - the copy is the original's rows that are no candidate, and the candidates its own rules derive, each rechecked;
/// - its rows gone and added (`rr_gone_`, `rr_added_`) are among the candidates: the deltas its readers read.
///
/// In a recursive component the candidates go on through its reads of itself. A removal's rule reads them as the
/// originals, or one as its candidates; an addition's each as the original or the candidates, one as the candidates
/// where no change point is read as its delta. By induction on a derivation's depth: a row of an original not in its
/// copy has a wide derivation through a change point or a candidate (its body's rows of the component not candidates
/// are the copy's, so else the narrow rule derives it too); a row of a copy not in its original has a narrow one
/// through a change point or a candidate (its body's rows of the component are each the original's or a candidate's,
/// so else the wide rule derives it).
///
/// An aggregate over a copy is a change point too: its value changes only in a group where the copy's rows did, so
/// the rule with the aggregate kept and a row gone or added in its group read beside it. A computed head term is
/// read back from the candidates as a variable equal to it.
///
/// Its cost is the annotations' share, not the tree's. An original is diffed where it does not reach its copy, its
/// rules read its component positively, outside aggregates, at most `SCC_READS` times,
/// no guest request reaches the copy (in a request's cone it runs before the values the request parses are
/// admitted, and the annotations' nodes `kind` holds then are not those a stratum after the request reads),
/// the original does not read the copy (the copy's negation of its candidates would close a cycle), it has no native,
/// lattice, facts or tape columns, and each copy it reads is diffed too. `RERAIL_NARROW_DIFF=off` derives every copy
/// again.
pub(super) struct Diffed {
    of: BTreeSet<String>,
    read: BTreeSet<String>,
    /// Each diffed original's columns.
    cols: HashMap<String, Vec<(String, String)>>,
    /// The node column of `kind`, and the document column of `docof`, as declared: the annotations' keys' types.
    node: Option<(String, String)>,
    doc: Option<(String, String)>,
    /// Each diffed original's recursive component (its own included): the originals it reads that read it.
    scc: HashMap<String, BTreeSet<String>>,
    /// The copies within one document (`one_document`) a diffed rule reads, each with its columns and its node
    /// columns: a change point as a diffed copy is, its rows gone those of a node past the narrow bound, none added.
    within: Vec<(String, Vec<(String, String)>, Vec<usize>)>,
    /// The program as written, each copy a node: who reads whom.
    edges: HashMap<String, BTreeSet<String>>,
}

/// The most reads of its component a diffed rule may make: an addition's rule is one per way of reading each.
const SCC_READS: usize = 3;

/// The annotations' nodes and documents (`Diffed`): a bound's delta.
const ANN_NODE: &str = "rr_ann_node";
const ANN_DOC: &str = "rr_ann_doc";

/// The strongly connected components of a graph (Kosaraju's, without recursion): each node's, by name.
fn components(edges: &HashMap<String, BTreeSet<String>>) -> HashMap<String, BTreeSet<String>> {
    let mut names: Vec<&str> = edges.keys().map(String::as_str).chain(edges.values().flatten().map(String::as_str)).collect();
    names.sort_unstable();
    names.dedup();
    let id: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut out_of: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    let mut into: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    for (from, tos) in edges {
        for to in tos {
            out_of[id[from.as_str()]].push(id[to.as_str()]);
            into[id[to.as_str()]].push(id[from.as_str()]);
        }
    }
    // Each node finished, in order.
    let mut done: Vec<usize> = Vec::with_capacity(names.len());
    let mut seen = vec![false; names.len()];
    for start in 0..names.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&mut (n, ref mut next)) = stack.last_mut() {
            if let Some(&m) = out_of[n].get(*next) {
                *next += 1;
                if !seen[m] {
                    seen[m] = true;
                    stack.push((m, 0));
                }
            } else {
                done.push(n);
                stack.pop();
            }
        }
    }
    let mut comp = vec![usize::MAX; names.len()];
    let mut count = 0;
    for &start in done.iter().rev() {
        if comp[start] != usize::MAX {
            continue;
        }
        comp[start] = count;
        let mut todo = vec![start];
        while let Some(n) = todo.pop() {
            for &m in &into[n] {
                if comp[m] == usize::MAX {
                    comp[m] = count;
                    todo.push(m);
                }
            }
        }
        count += 1;
    }
    let mut members: Vec<BTreeSet<String>> = vec![BTreeSet::new(); count];
    for (i, n) in names.iter().enumerate() {
        members[comp[i]].insert((*n).to_owned());
    }
    names.iter().enumerate().map(|(i, n)| ((*n).to_owned(), members[comp[i]].clone())).collect()
}

fn cand_name(orig: &str) -> String {
    format!("rr_cand_{orig}")
}
fn gone_name(orig: &str) -> String {
    format!("rr_gone_{orig}")
}
fn added_name(orig: &str) -> String {
    format!("rr_added_{orig}")
}

impl Diffed {
    fn new(v: &Version, folded: &HashMap<String, String>, read: &BTreeSet<String>, within: &BTreeSet<String>, narrowing: &Narrowing<'_>, requests: &BTreeSet<String>, narrow: i64) -> Diffed {
        let node = v.columns_of("kind").and_then(|c| c.first().cloned());
        let doc = v.columns_of("docof").and_then(|c| c.get(1).cloned());
        let mut out = Diffed { of: BTreeSet::new(), read: read.clone(), cols: HashMap::new(), node, doc, scc: HashMap::new(), within: Vec::new(), edges: HashMap::new() };
        if std::env::var("RERAIL_NARROW_DIFF").is_ok_and(|x| x == "off") || out.node.is_none() || out.doc.is_none() || read.is_empty() {
            return out;
        }
        let mut rules_of: HashMap<&str, Vec<RuleRef<'_>>> = HashMap::new();
        for r in v.rules() {
            let head = v.rel_name(r.head().rel());
            if !folded.contains_key(head) {
                rules_of.entry(head).or_default().push(r);
            }
        }
        let natives = v.native_map().unwrap_or_default();
        let lattice = v.lattice_decls().unwrap_or_default();
        let tapes = v.tape_columns().unwrap_or_default();
        let nullary = v.nullary_names().unwrap_or_default();
        let has_doc = out.doc.is_some();
        let shaped = |x: &str| -> bool {
            let Some(rs) = rules_of.get(x) else { return false };
            if natives.contains_key(x) || lattice.contains_key(x) || tapes.contains_key(x) || nullary.contains(x) || v.fact_values(x, &Default::default()).is_some() || v.columns_of(x).is_none() {
                return false;
            }
            rs.iter().all(|r| {
                r.body().all(|l| match l.view() {
                        Lit::Atom(_) | Lit::Agg { .. } => true,
                        Lit::Cmp { op: CmpOp::Lt, l, r: TermRef::Int(b) } if b == narrow << 32 => matches!(l, TermRef::Var(_)),
                        Lit::Cmp { op: CmpOp::Lt, l, r: TermRef::Int(b) } if b == narrow => matches!(l, TermRef::Var(_)) && has_doc,
                        _ => true,
                    })
            })
        };
        let debug = std::env::var_os("RERAIL_NARROW_DIFF_DEBUG").is_some();
        let mut of: BTreeSet<String> = read.iter().filter(|x| !within.contains(*x)).filter(|x| shaped(x) || (debug && { eprintln!("NDIFF {x}: shape"); false })).cloned().collect();
        // The program as written, each copy a node: who reads whom.
        let mut edges: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (head, rs) in &rules_of {
            let sees = narrowing.sees(head);
            for r in rs {
                for a in r.body().filter_map(|l| match l.view() {
                    Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(a),
                    _ => None,
                }) {
                    let rel = v.rel_name(a.rel());
                    let to = |narrow_reads: bool| if narrow_reads && read.contains(rel) { narrow_name(rel) } else { folded.get(rel).cloned().unwrap_or_else(|| rel.to_owned()) };
                    edges.entry((*head).to_owned()).or_default().insert(to(sees));
                    if read.contains(*head) {
                        edges.entry(narrow_name(head)).or_default().insert(to(true));
                    }
                }
            }
        }
        // Each one's component: what it reaches that reaches it.
        let reach = |from: &str| -> BTreeSet<String> {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut todo: Vec<&str> = vec![from];
            while let Some(n) = todo.pop() {
                for m in edges.get(n).into_iter().flatten() {
                    if seen.insert(m.clone()) {
                        todo.push(m);
                    }
                }
            }
            seen
        };
        let comps = components(&edges);
        let scc: HashMap<String, BTreeSet<String>> = of.iter().map(|x| (x.clone(), comps.get(x.as_str()).cloned().unwrap_or_else(|| BTreeSet::from([x.clone()])))).collect();
        // No original of a component reaches a copy of it: a copy is its original's rows less the candidates. Its
        // members reach what each does.
        let mut forward: HashMap<String, bool> = HashMap::new();
        of.retain(|x| {
            let c = &scc[x];
            let first = c.iter().next().expect("a member").clone();
            let ok = *forward.entry(first.clone()).or_insert_with(|| {
                let r = reach(&first);
                c.iter().all(|z| !r.contains(&narrow_name(z)))
            });
            if debug && !ok {
                eprintln!("NDIFF {x}: reads its copy");
            }
            ok
        });
        // A rule reads its component positively, outside aggregates, at most `SCC_READS` times.
        of.retain(|x| {
            let c = &scc[x];
            let ok = rules_of[x.as_str()].iter().all(|r| {
                let mut n = 0;
                r.body().all(|l| match l.view() {
                    Lit::Atom(a) if c.contains(v.rel_name(a.rel())) => {
                        n += 1;
                        !a.neg() && n <= SCC_READS
                    }
                    Lit::Agg { atom, .. } => !c.contains(v.rel_name(atom.rel())),
                    _ => true,
                })
            });
            if debug && !ok {
                eprintln!("NDIFF {x}: reads its component so");
            }
            ok
        });
        // No copy a request reaches (`plan::requests_first`'s cone): run before the request, it is computed before the
        // values the request parses are admitted, as the strings it asks for are; derived again, as it was.
        let mut cone: BTreeSet<String> = BTreeSet::new();
        for q in requests {
            if cone.insert(q.clone()) {
                cone.extend(reach(q));
            }
        }
        of.retain(|x| {
            let ok = !cone.contains(&narrow_name(x));
            if debug && !ok {
                eprintln!("NDIFF {x}: a request reaches its copy");
            }
            ok
        });
        // Each copy it reads diffed too: the largest such set.
        loop {
            let gone: Vec<String> = of
                .iter()
                .filter(|x| {
                    scc[x.as_str()].iter().any(|y| !of.contains(y))
                        || rules_of[x.as_str()].iter().any(|r| r.body().filter_map(|l| l.atom()).any(|a| {
                            let rel = v.rel_name(a.rel());
                            read.contains(rel) && !of.contains(rel) && !within.contains(rel)
                        }))
                })
                .cloned()
                .collect();
            if gone.is_empty() {
                break;
            }
            for x in gone {
                if debug {
                    eprintln!("NDIFF {x}: reads {:?}", rules_of[x.as_str()].iter().flat_map(|r| r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()))).filter(|r| read.contains(*r) && !of.contains(*r)).collect::<BTreeSet<_>>());
                }
                of.remove(&x);
            }
        }
        out.cols = of.iter().map(|x| (x.clone(), v.columns_of(x).expect("shaped"))).collect();
        let reads_within: BTreeSet<&str> = of
            .iter()
            .flat_map(|x| rules_of[x.as_str()].iter().flat_map(|r| r.body().filter_map(|l| match l.view() {
                Lit::Atom(a) | Lit::Agg { atom: a, .. } => Some(v.rel_name(a.rel())),
                _ => None,
            })))
            .filter(|rel| within.contains(*rel))
            .collect();
        out.within = reads_within
            .into_iter()
            .filter_map(|rel| {
                let cols = v.columns_of(rel)?;
                let nodes = (0..cols.len()).filter(|&c| cols[c].1 == "node").collect();
                Some((rel.to_owned(), cols, nodes))
            })
            .collect();
        out.scc = scc.into_iter().filter(|(x, _)| of.contains(x)).collect();
        out.of = of;
        out.edges = edges;
        out
    }

    /// Whether `from` reads `to`, through any number of rules.
    fn reaches(&self, from: &str, to: &str) -> bool {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut todo = vec![from];
        while let Some(n) = todo.pop() {
            for m in self.edges.get(n).into_iter().flatten() {
                if m == to {
                    return true;
                }
                if seen.insert(m) {
                    todo.push(m);
                }
            }
        }
        false
    }

    /// Rule `r` of diffed original `head` (its wide rule written already): its candidates' rules, and its recheck.
    /// A removal's rule is the wide one, an addition's the narrow one; each reads one change point as its delta,
    /// positively (a removal: an atom's rows gone, a negation's added; an addition the other way). A read is a change
    /// point where the two read it differently: not one the wide rule reads narrow itself (`Narrowing::sees`), nor
    /// of its component. A removal's reads its component as the originals, or one as its candidates; an addition's
    /// each as the original or the candidates (a row of a copy is one or the other), one as the candidates where no
    /// change point is read as its delta, and the candidates go on through them. A widened bound lets the wide rule
    /// through where the narrow does not: a removal's read of the annotations' keys there. An aggregate's value
    /// changes only in a group its copy's rows changed in: the rule with a row gone or added there read beside it.
    fn write(&self, v: &Version, r: RuleRef<'_>, head: &str, rw: &Fold<'_, '_>, out: &mut RuleOut<'_, '_>) -> Result<()> {
        let narrow = rw.at.narrow;
        let cand = cand_name(head);
        let fold = |wide: bool, head: String, renames: Vec<(usize, String)>| {
            let (original, narrow_reads) = if wide { (rw.original, rw.narrow_reads) } else { (false, true) };
            Fold { v, at: rw.at, original, narrow_reads, head: Some(head), changed: true, plain: rw.plain.clone(), ids: rw.ids.clone(), renames, atoms: 0 }
        };
        // The body's atoms, in order: each one's place, relation and sign.
        let mut atoms: Vec<(usize, String, bool)> = Vec::new();
        // Each aggregate: its atom's place, relation and terms.
        let mut aggs: Vec<(usize, String, Vec<T<'_>>)> = Vec::new();
        for l in r.body() {
            match l.view() {
                Lit::Atom(a) => atoms.push((atoms.len() + aggs.len(), v.rel_name(a.rel()).to_owned(), a.neg())),
                Lit::Agg { atom: a, .. } => {
                    let terms = a.args().map(|t| crate::code_rule_dsl::logic::program_image::build::t_of(r, t)).collect();
                    aggs.push((atoms.len() + aggs.len(), v.rel_name(a.rel()).to_owned(), terms));
                }
                _ => {}
            }
        }
        let comp = &self.scc[head];
        let own: Vec<(usize, String)> = atoms.iter().filter(|(_, rel, _)| comp.contains(rel)).map(|(i, rel, _)| (*i, rel.clone())).collect();
        let ways = |extra: Option<(usize, String)>, changed: bool| -> Vec<Vec<(usize, String)>> {
            (0u32..1 << own.len())
                .filter(|m| changed || *m != 0)
                .map(|m| {
                    let mut rn: Vec<(usize, String)> = own.iter().enumerate().map(|(k, (i, rel))| (*i, if m >> k & 1 == 1 { cand_name(rel) } else { rel.clone() })).collect();
                    rn.extend(extra.clone());
                    rn
                })
                .collect()
        };
        // A read is a change point where the two sides read it differently: the wide side reads a copy only where it
        // reads the narrow ones itself.
        let changes = |rel: &str| self.read.contains(rel) && !comp.contains(rel) && !rw.narrow_reads;
        for (at, rel, neg) in &atoms {
            if !changes(rel) {
                continue;
            }
            let (gone, added) = (gone_name(rel), added_name(rel));
            out.rewrite(&mut fold(true, cand.clone(), vec![(*at, if *neg { added.clone() } else { gone.clone() })]))?;
            for rn in ways(Some((*at, if *neg { gone } else { added })), true) {
                out.rewrite(&mut fold(false, cand.clone(), rn))?;
            }
        }
        for (_, rel, terms) in aggs.iter().filter(|(_, rel, _)| changes(rel)) {
            for delta in [gone_name(rel), added_name(rel)] {
                let mut f = fold(true, cand.clone(), Vec::new());
                out.rewriting(&mut f).atom(delta.as_str(), terms).finish()?;
                for rn in ways(None, true) {
                    let mut f = fold(false, cand.clone(), rn);
                    out.rewriting(&mut f).atom(delta.as_str(), terms).finish()?;
                }
            }
        }
        for (o, rel) in &own {
            out.rewrite(&mut fold(true, cand.clone(), vec![(*o, cand_name(rel))]))?;
        }
        for rn in ways(None, false) {
            out.rewrite(&mut fold(false, cand.clone(), rn))?;
        }
        // The widened bounds: the wide rule with the annotations' keys read at one.
        if rw.original {
            for l in r.body() {
                if let Lit::Cmp { op: CmpOp::Lt, l: TermRef::Var(s), r: TermRef::Int(b) } = l.view()
                    && (b == narrow << 32 || b == narrow)
                {
                    let ann = if b == narrow << 32 { ANN_NODE } else { ANN_DOC };
                    let mut f = fold(true, cand.clone(), Vec::new());
                    out.rewriting(&mut f).atom(ann, &[T::VarAtom(r.var(s))]).finish()?;
                }
            }
        }
        // A computed head term (`@cat(m, x)`) is read from the candidates as a variable equal to it.
        let terms: Vec<TermRef<'_>> = r.head().args().collect();
        let names: Vec<String> = (0..terms.len()).map(|i| format!("rr_h{i}")).collect();
        let args: Vec<T<'_>> = terms.iter().zip(&names).map(|(t, n)| if matches!(t, TermRef::Call(..)) { T::Var(n) } else { crate::code_rule_dsl::logic::program_image::build::t_of(r, *t) }).collect();
        // The recheck: the narrow rule over the candidates.
        let mut f = fold(false, narrow_name(head), Vec::new());
        let mut b = out.rewriting(&mut f).atom(cand.as_str(), &args);
        for (t, n) in terms.iter().zip(&names) {
            if matches!(t, TermRef::Call(..)) {
                b = b.cmp(T::Var(n), CmpOp::Eq, crate::code_rule_dsl::logic::program_image::build::t_of(r, *t));
            }
        }
        b.finish()?;
        Ok(())
    }

    /// Each diffed copy's rows, and its deltas; the annotations' nodes and documents.
    fn finish(&self, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, narrow: i64, wide: i64) -> Result<()> {
        if self.of.is_empty() {
            return Ok(());
        }
        // A copy within one document, as a change point: its original's rows with a node past the narrow bound are
        // gone from it, one rule a node column; none is added (`rr_added_` declared, with no rule).
        for (w, cols, nodes) in &self.within {
            let names: Vec<String> = (0..cols.len()).map(|i| format!("rr_x{i}")).collect();
            let xs: Vec<T<'_>> = names.iter().map(|n| T::Var(n)).collect();
            let g = gone_name(w);
            for &c in nodes {
                b.rule().head(g.as_str(), &xs).atom(w.as_str(), &xs).cmp(T::Var(&names[c]), CmpOp::Ge, T::Int(narrow << 32)).finish()?;
            }
        }
        for x in &self.of {
            let names: Vec<String> = (0..self.cols[x].len()).map(|i| format!("rr_x{i}")).collect();
            let xs: Vec<T<'_>> = names.iter().map(|n| T::Var(n)).collect();
            let (n, c, g, a) = (narrow_name(x), cand_name(x), gone_name(x), added_name(x));
            b.rule().head(n.as_str(), &xs).atom(x.as_str(), &xs).not(c.as_str(), &xs).finish()?;
            b.rule().head(g.as_str(), &xs).atom(c.as_str(), &xs).atom(x.as_str(), &xs).not(n.as_str(), &xs).finish()?;
            b.rule().head(a.as_str(), &xs).atom(c.as_str(), &xs).atom(n.as_str(), &xs).not(x.as_str(), &xs).finish()?;
        }
        // From every node's kind: a tape value has no `doc` row (`vtab::syntax`), and a run's guest rows are only those
        // its projection asks for (a pack's may not), so neither the documents nor `guest_of`. Rows of their own, so
        // a candidate's join starts from them; a document's from its nodes, with no join.
        // `kind` grows as the requests' values are admitted: a stratum's nodes are those admitted before it. No diffed
        // copy is in a request's cone (`Diffed::new`), so these are outside every cone too, and run after the requests
        // (`plan::requests_first` gives every stratum outside a cone an edge to its request).
        let (nv, dv) = (T::Var("rr_n"), T::Var("rr_d"));
        b.rule().head(ANN_NODE, &[nv.clone()]).atom("kind", &[nv.clone(), T::Wild]).cmp(nv.clone(), CmpOp::Ge, T::Int(narrow << 32)).cmp(nv.clone(), CmpOp::Lt, T::Int(wide << 32)).finish()?;
        b.rule().head(ANN_DOC, &[dv.clone()]).atom(ANN_NODE, &[nv.clone()]).cmp(dv.clone(), CmpOp::Eq, T::Call("doc", vec![nv])).finish()?;
        Ok(())
    }

    /// The relations it adds, declared as those they hold rows of.
    fn declare(&self, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>) {
        if self.of.is_empty() {
            return;
        }
        for x in &self.of {
            let pairs: Vec<(&str, &str)> = self.cols[x].iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            for name in [cand_name(x), gone_name(x), added_name(x)] {
                b.fields().declare(&name, &pairs);
            }
        }
        for (w, cols, _) in &self.within {
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            for name in [gone_name(w), added_name(w)] {
                b.fields().declare(&name, &pairs);
            }
        }
        if let Some((n, t)) = &self.node {
            b.fields().declare(ANN_NODE, &[(n.as_str(), t.as_str())]);
        }
        if let Some((n, t)) = &self.doc {
            b.fields().declare(ANN_DOC, &[(n.as_str(), t.as_str())]);
        }
        // Rows of their own, not a test inlined into each candidate's rule: the few a candidate's join starts from,
        // made whole (a reader's demand would join them with its keys, every node's).
        b.fields().add_qualifiers(&[(ANN_NODE, "no_inline"), (ANN_DOC, "no_inline"), (ANN_NODE, "no_magic"), (ANN_DOC, "no_magic")]);
    }
}
