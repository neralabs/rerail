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
        // An original holds its copies' rows: its rules' bounds take the annotations in (`RUN_VALUE_KEYS` to
        // `HIDDEN_KEYS`), as its copies' seeded columns did. A rule that does not reach an annotation keeps the bound.
        let originals: BTreeSet<String> = folded.values().cloned().collect();
        let (narrow, wide) = (i64::from(RUN_VALUE_KEYS), i64::from(HIDDEN_KEYS));
        let plain = format!("{label}:");
        let mut rules = Folding { folded: &folded, axes: &axes, sets: &sets, originals: &originals, narrow, wide, plain: &plain };
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
        let qualifiers: Vec<Vec<Cell>> = b.about_rows("qualifier").into_iter().filter(|r| !named(b, r[1], &|x| folded.contains_key(x))).collect();
        b.fields().set_about_rows("qualifier", qualifiers);
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
        if f.demand().any(moved) {
            let p = super::demand::about_p(b);
            let rows: Vec<Vec<Cell>> = renamed(f.demand().map(str::to_owned).collect()).iter().enumerate().map(|(k, r)| vec![p, Cell::Int(k as i64), Cell::Str(b.atom(r))]).collect();
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
    folded: &'a HashMap<String, String>,
    /// Each tree axis over a copied kind set, and the original set's axis.
    axes: &'a BTreeMap<String, String>,
    sets: &'a HashMap<String, String>,
    originals: &'a BTreeSet<String>,
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
        let mut rw = Fold { v, at: self, original: self.originals.contains(head), changed: false, plain: HashMap::new(), ids: HashMap::new() };
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
        if !rw.changed {
            return Ok(Step::Keep);
        }
        // The plain names, as the builder's atoms.
        let names: Vec<(AtomId, String)> = rw.plain.iter().map(|(a, n)| (*a, n.clone())).collect();
        for (a, n) in names {
            let id = out.builder().atom(&n);
            rw.plain.insert(a, n);
            rw.ids.insert(a, id);
        }
        out.rewrite(&mut rw)
    }
}

struct Fold<'a, 'v> {
    v: &'v Version,
    at: &'a Folding<'a>,
    original: bool,
    changed: bool,
    /// Each qualified kind or field constant, by its atom, and its plain name.
    plain: HashMap<AtomId, String>,
    /// The plain names as the builder's atoms.
    ids: HashMap<AtomId, AtomId>,
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
        if self.at.folded.contains_key(rel) || self.at.axes.contains_key(rel) {
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
        self.at.folded.get(rel).or_else(|| self.at.axes.get(rel)).map(|o| (o.clone(), a.neg(), a.gated()))
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
