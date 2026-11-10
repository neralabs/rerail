//! A combined run's fact spaces (decision 0091): the library's relations are canonical, made once over every
//! document the run reads, and each policy's own rules see only the documents of its scope.
//!
//! When the libraries are held once (decision 0089), the library is a part of its own, under its own names, and no
//! policy's: canonical by construction. Otherwise (`RERAIL_LIBRARIES=copied`, or one program naming its libraries)
//! `combine` names each policy's relations `p<i>_…`, an included library's among them. Here:
//!
//! - A relation the policies mark global (`.global`, or `.global library` for an included file's, as
//!   `rr::read_file` writes it) is *canonical* when every policy's copy is congruent to the others
//!   (`simplify::identical_classes`: equal rules, relations named by class, variables by order). The simplifier then
//!   merges the copies into one (`s_…`). A `.global` the policies define differently is an error; a library's that a
//!   policy changed (it adds rules, or the library reads a relation the policy defines) stays each policy's own.
//! - Canonical relations, and every relation of a policy they read, are *shared*: computed over every document.
//! - Each other rule of a policy whose scope reads fewer documents than the run has a *guard* on each read of a base or
//!   shared relation whose location no read of its own relations already gives: `!rr_scope_not(d)` of the read's
//!   document (its document column, or `@doc(n)` of its node, a shift of the node id: no rows). A policy's own
//!   relations are then made from its documents' rows only, by induction, so their locations need no guard.
//!
//! Without guards, a policy's cross-file rule would join rows of documents only another policy's scope selects.

use std::collections::{BTreeMap, BTreeSet};

use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Program, Term};

/// What `combine` knows of one policy: its prefix, its own relations (prefixed), its globals (unprefixed), its scope.
pub(super) struct Policy {
    pub prefix: String,
    pub own: BTreeSet<String>,
    pub named: BTreeSet<String>,
    pub library: BTreeSet<String>,
    pub scope: Option<Vec<(Vec<String>, Vec<String>)>>,
}

/// What `guard` did, for the profile.
#[derive(Default, Debug)]
pub(in crate::code_rule_dsl) struct Report {
    /// Global relations with one copy for the run (by name), and how many policies' copies each merged.
    pub canonical: BTreeMap<String, usize>,
    /// Copies of the engine's relations made canonical (alike in several policies).
    pub engine: usize,
    /// Library relations a policy changed: each policy's own.
    pub demoted: Vec<String>,
    /// Policies whose rules got guards, and the guards added.
    pub guarded: Vec<(String, usize)>,
}

/// The guards and the canonical relations of `all`, the policies combined (see the module).
pub(super) fn guard(all: &mut Program, policies: &[Policy]) -> Result<Report, String> {
    let mut report = Report::default();
    let classes = crate::code_rule_dsl::logic::simplify::identical_classes(all);
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
    // The engine's relations (`rr_…`: lowering's helpers, guest contexts, aggregates), alike in several policies, are
    // canonical too: they are made from the library's rules as much as from a policy's. The scope's own are not.
    let engine = |rel: &str| policies.iter().find_map(|p| rel.strip_prefix(p.prefix.as_str())).is_some_and(|r| r.starts_with(front::RESERVED) && !r.starts_with("rr_scope") && !r.starts_with("rr_report"));
    for class in &classes {
        let spaces: BTreeSet<usize> = class.iter().filter_map(|r| policies.iter().position(|p| p.own.contains(r))).collect();
        if spaces.len() > 1 && class.iter().all(|r| engine(r)) {
            report.engine += class.len();
            canonical.extend(class.iter().cloned());
        }
    }
    // Shared: the canonical relations and every relation of a policy they read, to a fixpoint.
    let owned: BTreeSet<&String> = policies.iter().flat_map(|p| &p.own).collect();
    let mut shared = canonical;
    loop {
        let before = shared.len();
        let reads: Vec<String> = all
            .rules
            .iter()
            .filter(|r| shared.contains(&r.head.rel))
            .flat_map(|r| r.body.iter().filter_map(Literal::atom).map(|a| a.rel.clone()))
            .chain(all.natives.iter().filter(|(rel, _)| shared.contains(*rel)).flat_map(|(_, n)| n.reads().into_iter().map(str::to_owned)))
            .filter(|rel| owned.contains(rel))
            .collect();
        shared.extend(reads);
        if shared.len() == before {
            break;
        }
    }
    // Guard groups: each policy narrower than the run, its rules guarded by its scope. Shared relations are made over
    // every document the run reads (decision 0091 §3): a scope limits where a policy's own rules apply, not which
    // facts exist.
    let n = policies.len();
    let scope_of = |i: usize| policies[i].scope.as_ref().map(|s| s.iter().cloned().collect::<BTreeSet<_>>());
    let outside_of = |i: usize| format!("{}{}", policies[i].prefix, front::SCOPE_NOT);
    let narrows = |i: usize| scope_of(i).is_some() && policies[i].own.contains(&outside_of(i)) && (0..n).any(|j| scope_of(j) != scope_of(i));
    let owners: BTreeMap<&str, usize> = policies.iter().enumerate().flat_map(|(i, p)| p.own.iter().map(move |r| (r.as_str(), i))).collect();
    let owner = |rel: &str| owners.get(rel).copied();
    let machinery = |rel: &str| {
        let r = owner(rel).and_then(|i| rel.strip_prefix(policies[i].prefix.as_str())).unwrap_or(rel);
        r.starts_with("rr_scope") || r.starts_with("rr_report")
    };
    let mut groups: Vec<(BTreeSet<String>, String)> = Vec::new();
    for i in (0..n).filter(|&i| narrows(i)) {
        let heads = policies[i].own.iter().filter(|r| !shared.contains(*r) && !machinery(r)).cloned().collect();
        groups.push((heads, outside_of(i)));
    }
    // `RERAIL_SCOPE_GUARDS=off`: none, as before this record (to measure what the guards cost and what they change).
    if groups.is_empty() || std::env::var("RERAIL_SCOPE_GUARDS").is_ok_and(|v| v == "off") {
        return Ok(report);
    }
    let (_, locations, _) = crate::code_rule_dsl::typing::file_local::split_located(all);
    let typed = |rel: &str, c: usize, ty: &[&str]| all.decls.get(rel).and_then(|cols| cols.get(c)).is_some_and(|(_, t)| ty.contains(&t.as_str()));
    let derived: BTreeSet<String> = all.rules.iter().map(|r| r.head.rel.clone()).chain(all.natives.keys().cloned()).filter(|r| !all.inputs.contains(r)).collect();
    // Per relation, where a row's document is read: (document columns, node columns, span or node columns). A node's
    // document is its id's high half (`@doc`); a span's is `docof`'s, as `file_local::locate` reads it.
    let place = |rel: &str| -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let width = all.decls.get(rel).map_or(0, Vec::len);
        if derived.contains(rel) {
            let cols = locations.get(rel).cloned().unwrap_or_default();
            let of = |ty: &[&str]| cols.iter().copied().filter(|&c| typed(rel, c, ty)).collect::<Vec<_>>();
            (of(&["int", "number"]), of(&["node"]), of(&["loc", "span"]))
        } else {
            let of = |ty: &[&str]| (0..width).filter(|&c| typed(rel, c, ty)).collect::<Vec<_>>();
            (crate::code_rule_dsl::typing::file_local::document_column(rel).into_iter().collect(), of(&["node"]), of(&["loc", "span"]))
        }
    };
    // One rule's guards: on each read whose location no read of `covers` gives. Returns the guards added.
    let guard_rule = |rule: &mut front::Rule, covers: &BTreeSet<String>, outside: &str| -> usize {
        let var = |t: Option<&Term>| if let Some(Term::Var(v)) = t { Some(v.clone()) } else { None };
        let mut covered: BTreeSet<String> = BTreeSet::new();
        for a in rule.body.iter().filter_map(|l| if let Literal::Atom(a) = l { Some(a) } else { None }).filter(|a| !a.neg && covers.contains(&a.rel)) {
            let (docs, nodes, spans) = place(&a.rel);
            covered.extend(docs.into_iter().chain(nodes).chain(spans).filter_map(|c| var(a.args.get(c))));
        }
        let mut guards: Vec<(usize, Vec<Literal>)> = Vec::new();
        let mut fresh = 0;
        // A check of a bound document, never a relation to start from: the documents out of the scope, negated.
        let at = |d: &str| Literal::Atom(Atom { rel: outside.to_owned(), args: vec![Term::Var(d.to_owned())], neg: true, gated: false });
        let of_node = |n: &str, fresh: &mut usize| -> Vec<Literal> {
            let g = format!("{}{}", crate::code_rule_dsl::logic::simplify::GUARD_VAR, *fresh);
            *fresh += 1;
            vec![Literal::Cmp { l: Term::Var(g.clone()), op: "=", r: Term::Call("doc".to_owned(), vec![Term::Var(n.to_owned())]) }, at(&g)]
        };
        let of_span = |n: &str, fresh: &mut usize| -> Vec<Literal> {
            let g = format!("{}{}", crate::code_rule_dsl::logic::simplify::GUARD_VAR, *fresh);
            *fresh += 1;
            vec![Literal::Atom(Atom { rel: "docof".to_owned(), args: vec![Term::Var(n.to_owned()), Term::Var(g.clone())], neg: false, gated: false }), at(&g)]
        };
        for (k, l) in rule.body.iter().enumerate() {
            match l {
                Literal::Atom(a) if !a.neg && !covers.contains(&a.rel) && !machinery(&a.rel) => {
                    let (docs, nodes, spans) = place(&a.rel);
                    let dvs: Vec<String> = docs.iter().filter_map(|&c| var(a.args.get(c))).collect();
                    let nvs: Vec<String> = nodes.iter().filter_map(|&c| var(a.args.get(c))).collect();
                    let svs: Vec<String> = spans.iter().filter_map(|&c| var(a.args.get(c))).collect();
                    let mut g = Vec::new();
                    if !derived.contains(&a.rel) {
                        // A base row is one document's: one guard, unless a location of it is covered.
                        if !dvs.iter().chain(&nvs).chain(&svs).any(|v| covered.contains(v)) {
                            // By the node where there is one (a shift: the read stays a test of that node, which
                            // `simplify` inlines), else the document column, else a span's `docof`.
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
                        for v in nvs {
                            if covered.insert(v.clone()) {
                                g.extend(of_node(&v, &mut fresh));
                            }
                        }
                        for v in svs {
                            if covered.insert(v.clone()) {
                                g.extend(of_span(&v, &mut fresh));
                            }
                        }
                    }
                    if !g.is_empty() {
                        guards.push((k, g));
                    }
                }
                Literal::Ast { document, .. } if covered.insert(document.clone()) => guards.push((k, vec![at(document)])),
                _ => {}
            }
        }
        let mut added = 0;
        for (k, g) in guards.into_iter().rev() {
            added += g.iter().filter(|l| matches!(l, Literal::Atom(a) if a.neg)).count();
            rule.body.splice(k + 1..k + 1, g);
        }
        added
    };
    let mut rules = all.rules.clone();
    // Each rule's groups, in order: one pass over the rules, not one per group.
    let mut groups_of: rustc_hash::FxHashMap<&str, Vec<usize>> = Default::default();
    for (k, (heads, _)) in groups.iter().enumerate() {
        for h in heads {
            groups_of.entry(h.as_str()).or_default().push(k);
        }
    }
    let mut added = vec![0usize; groups.len()];
    for r in rules.iter_mut() {
        let Some(ks) = groups_of.get(r.head.rel.as_str()) else { continue };
        for &k in ks {
            added[k] += guard_rule(r, &groups[k].0, &groups[k].1);
        }
    }
    for ((_, outside), n) in groups.iter().zip(added) {
        report.guarded.push((outside.clone(), n));
    }
    drop(place);
    all.rules = rules;
    Ok(report)
}
