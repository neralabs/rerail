//! Joins deferred to their readers' requests (decision 0506), on the version. A binary positive join projected
//! across a hidden key (`wide(a, b) :- lhs(a, k), rhs(b, k)`) is driven by what its readers request: the pairs
//! (or one endpoint) every reader binds become reader demand on it (`upstream::install_named`, `DEFERRED_PAIRS`);
//! a request for one endpoint that is frequent shares its side's witnesses across the pairs requested (`share`);
//! and a join its readers read broadly is kept as its two factors, each reduced by the other's keys, which meet in
//! each reader (`file_local::factorize_intermediates`), its pairs cached while the factors' sizes allow.

use super::{DEFERRED_PAIRS, PREFIX, Program, plain, rewritten, same, upstream};
use crate::code_rule_dsl::logic::front::ast::AggOp;
use crate::code_rule_dsl::logic::plan::body::atom_vars;
use crate::code_rule_dsl::logic::profile::author_joins::{Access, Join};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
use crate::code_rule_dsl::logic::program_image::{AtomRef, CmpOp, Lit, RuleRef, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The requests of one endpoint past which its side's witnesses are shared (`share`).
const SHARE_AFTER: i64 = 8;

/// What the pass did: the relations driven by requested pairs, by one requested endpoint, kept as factors, with an
/// endpoint's work shared; whether a pair cache was admitted; and each rewritten join for the authors' report.
#[derive(Default)]
pub(in crate::code_rule_dsl::logic::plan::corpus) struct Plans {
    pub pairs: Vec<String>,
    pub one_sided: Vec<String>,
    pub factors: Vec<String>,
    pub shared: Vec<String>,
    pub cached: bool,
    pub author: Vec<Join>,
}

/// The place in rule `r`'s body of its request guard: a positive atom of the head's own terms, of a demand relation
/// named after the head.
fn guard(v: &Version, r: RuleRef<'_>) -> Option<usize> {
    let head = v.rel_name(r.head().rel());
    let args: Vec<TermRef<'_>> = r.head().args().collect();
    r.body().position(|l| match l.view() {
        Lit::Atom(a) => {
            let read: Vec<TermRef<'_>> = a.args().collect();
            !a.neg()
                && !a.gated()
                && read.len() == args.len()
                && read.iter().zip(&args).all(|(x, y)| same(*x, *y))
                && [PREFIX, upstream::PREFIX, DEFERRED_PAIRS].iter().any(|p| v.rel_name(a.rel()).strip_prefix(p) == Some(head))
        }
        _ => false,
    })
}

/// A variable's name in rule `r`.
fn var_name(v: &Version, r: RuleRef<'_>, t: TermRef<'_>) -> Option<String> {
    match t {
        TermRef::Var(s) => Some(v.text(r.var(s)).to_owned()),
        _ => None,
    }
}

/// Rule `r` of two distinct head variables, of positive atoms over plain terms and comparisons other than
/// equalities, split at a hidden key into two parts that each read it (`keeps::factors_without`, its request guard
/// left out): the key's name and the parts' literals by place.
fn split(v: &Version, r: RuleRef<'_>) -> Option<(String, Vec<Vec<usize>>)> {
    let head: Vec<TermRef<'_>> = r.head().args().collect();
    let [TermRef::Var(a), TermRef::Var(b)] = head.as_slice() else { return None };
    if a == b
        || !r.body().all(|l| match l.view() {
            Lit::Atom(at) => !at.neg() && !at.gated() && at.args().all(|t| plain(t) || matches!(t, TermRef::Wild)),
            Lit::Cmp { l, op, r } => op != CmpOp::Eq && plain(l) && plain(r),
            _ => false,
        })
    {
        return None;
    }
    let (key, parts) = crate::code_rule_dsl::logic::plan::keeps::factors_without(v, r, guard(v, r))?;
    let lits: Vec<_> = r.body().collect();
    let keyed = |part: &Vec<usize>| {
        part.iter().any(|&i| matches!(lits[i].view(), Lit::Atom(a) if atom_vars(a).iter().any(|s| v.text(r.var(s)) == key)))
    };
    (parts.len() == 2 && parts.iter().all(keyed)).then_some((key, parts))
}

/// Whether reader `rule` binds read `read`'s columns `cols` from elsewhere: each is plain, and its variables are all
/// read by another positive atom of the rule (or it has none).
fn reader_binds(v: &Version, rule: RuleRef<'_>, read: AtomRef<'_>, cols: &[usize]) -> bool {
    let args: Vec<TermRef<'_>> = read.args().collect();
    let mut bound = BTreeSet::new();
    for &c in cols {
        let Some(t) = args.get(c).copied().filter(|t| plain(*t)) else { return false };
        bound.extend(var_name(v, rule, t));
    }
    let rel = v.rel_name(read.rel());
    bound.is_empty()
        || rule.body().any(|l| match l.view() {
            Lit::Atom(a) => {
                let vars: BTreeSet<String> = atom_vars(a).iter().map(|s| v.text(rule.var(s)).to_owned()).collect();
                v.rel_name(a.rel()) != rel && !a.neg() && !a.gated() && bound.is_subset(&vars)
            }
            _ => false,
        })
}

/// A read of a relation: its rule, the literal's place, and whether it is a plain atom (not an aggregate's) and
/// negated.
struct Read {
    rule: usize,
    lit: usize,
    atom: bool,
    neg: bool,
    gated: bool,
}

/// The program's version, made a whole `Arc` (the pass's input where nothing was made yet).
fn current(p: &Program<'_>, input: &Arc<Version>) -> Arc<Version> {
    p.made.clone().unwrap_or_else(|| Arc::clone(input))
}

/// `no_inline` on each of `rels`, the program's qualifiers kept.
fn no_inline(p: &mut Program<'_>, rels: &BTreeSet<String>) -> Result<(), String> {
    if rels.is_empty() {
        return Ok(());
    }
    let mut qualifiers = p.f.qualifiers.clone();
    for rel in rels {
        qualifiers.entry(rel.clone()).or_default().insert("no_inline".to_owned());
    }
    let made = p.derive(&mut |_, _, _, _| Ok(false), &mut |_, b| {
        use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
        b.fields().set_field(Field::Qualifiers(&qualifiers));
        Ok(())
    })?;
    p.set(made);
    Ok(())
}

/// Version `input` with its joins deferred to their readers' requests (the module comment), and what was done.
/// `adaptive`: an endpoint's work shared and pair caches admitted (`cache_pairs`); else the static rewrites alone.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn defer(input: &Arc<Version>, adaptive: bool, cache_pairs: bool) -> Result<(Arc<Version>, Plans), String> {
    let mut p = Program::of(input);
    let plans = rewrite(&mut p, input, adaptive, cache_pairs)?;
    Ok((current(&p, input), plans))
}

fn rewrite(p: &mut Program<'_>, input: &Arc<Version>, adaptive: bool, cache_pairs: bool) -> Result<Plans, String> {
    let mut plans = Plans::default();
    let v = p.v();
    let rules: Vec<RuleRef<'_>> = v.rules().collect();
    let head_of = |r: RuleRef<'_>| v.rel_name(r.head().rel()).to_owned();
    let mut definitions = BTreeMap::<String, usize>::new();
    let mut readers = BTreeMap::<String, Vec<Read>>::new();
    for (i, r) in rules.iter().enumerate() {
        *definitions.entry(head_of(*r)).or_default() += 1;
        for (j, l) in r.body().enumerate() {
            let read = match l.view() {
                Lit::Atom(a) => Some((a, true)),
                Lit::Agg { atom, .. } => Some((atom, false)),
                _ => None,
            };
            if let Some((a, atom)) = read {
                readers.entry(v.rel_name(a.rel()).to_owned()).or_default().push(Read { rule: i, lit: j, atom, neg: a.neg(), gated: a.gated() });
            }
        }
    }
    let read_atom = |read: &Read| -> AtomRef<'_> {
        match rules[read.rule].body().nth(read.lit).expect("a read's literal").view() {
            Lit::Atom(a) => a,
            Lit::Agg { atom, .. } => atom,
            _ => unreachable!("a read"),
        }
    };
    let columns = |rel: &str| p.columns(rel).unwrap_or_default();
    // A connected-demand pass may already have restricted the producer: its final guard is reported even where this
    // pass finds nothing more to do.
    let mut explained = BTreeSet::new();
    for r in &rules {
        if let Some(at) = guard(v, *r)
            && explained.insert(head_of(*r))
        {
            let Some(Lit::Atom(request)) = r.body().nth(at).map(|l| l.view()) else { unreachable!("a guard") };
            plans.author.push(Join {
                relation: head_of(*r),
                access: Access::Requests { table: v.rel_name(request.rel()).to_owned(), columns: columns(&head_of(*r)).into_iter().map(|c| c.0).collect() },
                readers: Vec::new(),
                placement: None,
            });
        }
    }
    let tapes = F::tapes(v);
    let shapes = F::shapes(v);
    let mut candidates = BTreeSet::new();
    let mut guarded: Vec<String> = Vec::new();
    for r in &rules {
        let rel = head_of(*r);
        if r.head().args().count() != 2
            || r.body().count() < 2
            || !readers.contains_key(&rel)
            || !upstream::eligible(p, &rel)
            || p.f.qualifiers.get(&rel).is_some_and(|q| !q.is_empty())
            || tapes.contains_key(&rel)
            || p.f.demand.iter().any(|d| *d == rel)
            || shapes.iter().any(|s| s.head == rel || s.lit == rel)
            || p.f.asked_by.values().flatten().any(|refs| refs.contains(&rel))
        {
            continue;
        }
        if split(v, *r).is_some() {
            if guard(v, *r).is_some() {
                if adaptive {
                    guarded.push(v.show_rule(*r));
                }
            } else if definitions[&rel] == 1 {
                candidates.insert(rel);
            }
        }
    }
    if candidates.is_empty() && guarded.is_empty() {
        return Ok(plans);
    }
    let Ok(sccs) = Program::components(v) else { return Ok(plans) };
    let recursive: BTreeSet<String> = sccs
        .into_iter()
        .filter(|c| c.len() > 1)
        .flatten()
        .chain(rules.iter().filter(|r| r.body().any(|l| matches!(l.view(), Lit::Atom(a) if a.rel() == r.head().rel()))).map(|r| head_of(*r)))
        .collect();
    let mut pairs: Vec<(String, Vec<usize>, bool)> = Vec::new();
    let mut broad = BTreeSet::new();
    let mut open_readers: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for name in candidates {
        if recursive.contains(&name) {
            continue;
        }
        let uses = &readers[&name];
        let cols = columns(&name);
        let mut descriptions = BTreeSet::new();
        for read in uses {
            let reader = rules[read.rule];
            let open: Vec<&str> = (0..2).filter(|c| !reader_binds(v, reader, read_atom(read), &[*c])).map(|c| cols[c].0.as_str()).collect();
            if !open.is_empty() {
                descriptions.insert((head_of(reader), format!("supplies no lookup value for {}", open.join(", "))));
            }
        }
        open_readers.insert(name.clone(), descriptions.into_iter().collect());
        if uses.iter().any(|read| !read.atom || read.gated) {
            continue;
        }
        let mask = [&[0, 1][..], &[0][..], &[1][..]].into_iter().find(|cols| uses.iter().all(|read| reader_binds(v, rules[read.rule], read_atom(read), cols)));
        let positive = uses.iter().all(|read| !read.neg);
        if let Some(cols) = mask {
            pairs.push((name, cols.to_vec(), positive));
        } else if positive {
            broad.insert(name);
        }
    }
    // Every read is classified before any rule changes.
    let mut inline_off = BTreeSet::new();
    for (rel, cols, positive) in pairs {
        if !upstream::install_named(p, &rel, &cols, &BTreeSet::new(), false, DEFERRED_PAIRS)? {
            continue;
        }
        inline_off.insert(rel.clone());
        let names = p.columns(&rel).unwrap_or_default();
        plans.author.push(Join {
            relation: rel.clone(),
            access: Access::Requests { table: format!("{DEFERRED_PAIRS}{rel}"), columns: cols.iter().map(|&c| names[c].0.clone()).collect() },
            readers: open_readers.remove(&rel).unwrap_or_default(),
            placement: None,
        });
        if let [c] = cols.as_slice() {
            plans.one_sided.push(format!("{rel} column {c}"));
            // A request for one endpoint can still match the whole of the other factor: its demand guard stays on
            // that factor, and positive readers read the join's factors directly.
            if adaptive && positive {
                broad.insert(rel);
            }
            continue;
        }
        plans.pairs.push(rel.clone());
        if adaptive {
            let v = p.v();
            if let Some(r) = v.rules().find(|r| v.rel_name(r.head().rel()) == rel)
                && split(v, r).is_some()
            {
                guarded.push(v.show_rule(r));
            }
        }
    }
    no_inline(p, &inline_off)?;
    for shown in guarded {
        let v = p.v();
        let Some(r) = v.rules().find(|r| v.show_rule(*r) == shown) else { continue };
        if recursive.contains(v.rel_name(r.head().rel())) {
            continue;
        }
        if let Some(name) = share(p, &shown)? {
            plans.shared.push(name);
        }
    }
    if let Some((made, factors, choices)) = crate::code_rule_dsl::typing::file_local::factorize_intermediates(&current(p, input), &broad, adaptive && cache_pairs)? {
        p.set(made);
        plans.factors = factors;
        plans.cached = choices.values().any(Option::is_some);
        for (relation, cache) in choices {
            let readers = open_readers.remove(&relation).unwrap_or_default();
            plans.author.push(Join { relation, access: Access::Factors { cache }, readers, placement: None });
        }
    }
    for (relation, readers) in open_readers {
        if !readers.is_empty() {
            plans.author.push(Join { relation, access: Access::Full, readers, placement: None });
        }
    }
    Ok(plans)
}

/// The guarded rule shown as `shown` with a frequently requested endpoint's work shared: the larger part's
/// witnesses for each endpoint requested at least `SHARE_AFTER` times kept once (`…_factor(endpoint, witness)`, for
/// the endpoints in `…_hot`), and those requests answered from them; the rule itself answers the rest. A part of one
/// literal is an indexed input that answers the probe in place already. The line for the profile, or `None`.
fn share(p: &mut Program<'_>, shown: &str) -> Result<Option<String>, String> {
    let v = p.v();
    let Some(r) = v.rules().find(|r| v.show_rule(*r) == shown) else { return Ok(None) };
    let (Some(request_at), Some((key, parts))) = (guard(v, r), split(v, r)) else { return Ok(None) };
    let Some((side, part)) = parts.iter().enumerate().max_by_key(|(_, part)| part.len()) else { return Ok(None) };
    if part.len() < 2 {
        return Ok(None);
    }
    let lits: Vec<_> = r.body().collect();
    let names = |l: usize| crate::code_rule_dsl::logic::plan::body::lit_vars(lits[l]).iter().map(|s| v.text(r.var(s)).to_owned()).collect::<BTreeSet<_>>();
    let vars: BTreeSet<String> = part.iter().flat_map(|&l| names(l)).collect();
    let head: Vec<TermRef<'_>> = r.head().args().collect();
    let Some(col) = head.iter().position(|t| var_name(v, r, *t).is_some_and(|x| vars.contains(&x))) else { return Ok(None) };
    let endpoint = head[col];
    // The key's term, and its column's type, from the part's first atom that reads it.
    let Some((key_term, key_ty)) = part.iter().find_map(|&l| {
        let Lit::Atom(a) = lits[l].view() else { return None };
        let c = a.args().position(|t| var_name(v, r, t).as_deref() == Some(key.as_str()))?;
        let ty = F::columns_of(v, v.rel_name(a.rel()))?.get(c)?.1.clone();
        Some((a.args().nth(c)?, ty))
    }) else {
        return Ok(None);
    };
    let rel = v.rel_name(r.head().rel()).to_owned();
    let Some(endpoint_ty) = p.columns(&rel).and_then(|c| c.get(col).map(|c| c.1.clone())) else { return Ok(None) };
    let Lit::Atom(request) = lits[request_at].view() else { return Ok(None) };
    let request_rel = v.rel_name(request.rel()).to_owned();
    let declared: BTreeSet<String> = F::declared_names(v).into_iter().collect();
    let base = crate::code_rule_dsl::typing::file_local::helper_name(
        "rr_deferred_",
        &rel,
        shown,
        |n| ["hot", "factor"].iter().any(|s| declared.contains(&format!("{n}_{s}"))),
        &mut Default::default(),
    );
    let (hot, factor) = (format!("{base}_hot"), format!("{base}_factor"));
    let taken: BTreeSet<String> = (0..lits.len()).flat_map(names).collect();
    let mut count = "rr_fanout".to_owned();
    while taken.contains(&count) {
        count.push('_');
    }
    let label = format!("{rel} column {col} ({} literals)", part.len());
    let other = parts[1 - side].clone();
    let part = part.clone();
    let id = r.id();
    let mut qualifiers = p.f.qualifiers.clone();
    for n in [&hot, &factor] {
        qualifiers.entry(n.clone()).or_default().insert("no_inline".to_owned());
    }
    let made = p.derive(
        &mut |first, v, r, b| {
            if r.id() != id {
                return Ok(false);
            }
            // The rule answers the endpoints not hot.
            let mut rb = rewritten(first, v, r, b);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            rb.not(hot.as_str(), &[t_of(v, r, endpoint)]).finish().map(|_| true)
        },
        &mut |v, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            let r = v.rule(id).expect("the shared rule");
            let lits: Vec<_> = r.body().collect();
            let e = t_of(v, r, endpoint);
            b.fields().declare(&hot, &[("endpoint", endpoint_ty.as_str())]);
            b.fields().declare(&factor, &[("endpoint", endpoint_ty.as_str()), ("witness", key_ty.as_str())]);
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            // hot(e) :- request(…e…), n = count : request(…e…), n >= SHARE_AFTER.
            let mut group = vec![T::Wild, T::Wild];
            group[col] = e.clone();
            b.rule()
                .head(hot.as_str(), std::slice::from_ref(&e))
                .atom(request_rel.as_str(), &group)
                .agg(T::Var(&count), &AggOp::Count, None, request_rel.as_str(), &group)
                .cmp(T::Var(&count), CmpOp::Ge, T::Int(SHARE_AFTER))
                .finish()?;
            // factor(e, k) :- hot(e), the part.
            let mut rb = b.rule().head(factor.as_str(), &[e.clone(), t_of(v, r, key_term)]).atom(hot.as_str(), std::slice::from_ref(&e));
            for &l in &part {
                rb = rb.lit(v, r, lits[l]);
            }
            rb.finish()?;
            // The rule's own head, from its request, the endpoint's witnesses and the other part.
            let mut rb = b.rule().head_from(v, r, r.head()).lit(v, r, lits[request_at]).atom(factor.as_str(), &[e.clone(), t_of(v, r, key_term)]);
            for &l in &other {
                rb = rb.lit(v, r, lits[l]);
            }
            rb.finish()?;
            Ok(())
        },
    )?;
    p.set(made);
    Ok(Some(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::plan::{corpus, output::Outputs};
    use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut as _, ProgramFields as _};
    use crate::code_rule_dsl::logic::rr::parse_version;
    use crate::code_rule_dsl::typing::file_local::Binding;

    const SOURCE: &str = "
        .decl lhs(a: number, k: number)
        .decl rhs(b: number, k: number)
        .decl enabled(b: number)
        .decl request(a: number, b: number)
        .decl wide(a: number, b: number)
        wide(a,b) :- lhs(a,k), rhs(b,k), enabled(b).
        lhs(1,10). lhs(1,20). lhs(2,10). lhs(3,99).
        rhs(1,10). rhs(1,20). rhs(2,99). rhs(3,10). rhs(7,77).
        enabled(1). enabled(2). enabled(7).
        request(1,1). request(2,1). request(3,1). request(4,1).
        request(5,1). request(6,1). request(7,1). request(8,1).
        request(3,2). request(1,3). request(1,7).
    ";

    /// `v` with `more` outputs too.
    fn with_outputs(v: &Arc<Version>, more: &[String]) -> Arc<Version> {
        let mut outputs = v.output_names();
        outputs.extend(more.iter().cloned());
        let mut b = v.builder();
        b.fields().set_outputs(&outputs);
        b.finish_overlay().unwrap()
    }

    /// The rows of each of `v`'s outputs, sorted, from a run of it.
    fn execute(v: &Arc<Version>, case: &str) -> BTreeMap<String, Vec<Vec<String>>> {
        let root = crate::code_rule_dsl::fixtures::temp().with_extension(case);
        std::fs::create_dir_all(&root).unwrap();
        let out = root.join("out");
        corpus::run_read(v, &root, &out, "program", Binding::Cpu, 4096, &Default::default(), &serde_json::Value::Null).unwrap();
        let outputs = Outputs::open(&out).unwrap();
        let rows = v
            .output_names()
            .into_iter()
            .map(|rel| {
                let mut rows = outputs.get(&rel).map(|r| r.shown()).unwrap_or_default();
                rows.sort();
                (rel, rows)
            })
            .collect();
        drop(outputs);
        std::fs::remove_dir_all(root).unwrap();
        rows
    }

    fn declared(v: &Version, f: impl Fn(&str) -> bool) -> Option<String> {
        v.declared_names().into_iter().find(|r| f(r))
    }

    #[test]
    fn hot_and_cold_requests_share_only_their_own_answers() {
        let v = parse_version(&format!(
            "{SOURCE}
            .decl yes(a: number, b: number)
            .decl missing(a: number, b: number)
            .output yes, missing
            yes(a,b) :- request(a,b), wide(a,b).
            missing(a,b) :- request(a,b), !wide(a,b).
        "
        ))
        .unwrap();
        let expected = execute(&v, "deferred-control");
        let (made, plans) = defer(&v, true, true).unwrap();
        assert_eq!(plans.pairs, ["wide"]);
        assert_eq!(plans.shared.len(), 1);
        let hot = declared(&made, |r| r.ends_with("_hot")).unwrap();
        let factor = declared(&made, |r| r.ends_with("_factor")).unwrap();
        let mut actual = execute(&with_outputs(&made, &[hot.clone(), factor.clone()]), "deferred-adaptive");
        assert_eq!(actual.remove(&hot).unwrap(), [["1"]]);
        assert_eq!(actual.remove(&factor).unwrap(), [["1", "10"], ["1", "20"]]);
        assert_eq!(actual, expected);
        assert_eq!(actual["yes"], [["1", "1"], ["2", "1"], ["3", "2"]]);
        assert_eq!(actual["missing"].len(), 8);
    }

    #[test]
    fn broad_readers_share_reduced_factors_and_fresh_witnesses() {
        let v = parse_version(&format!(
            "{SOURCE}
            .decl pairs(a: number, b: number)
            .decl diagonal(a: number)
            .output pairs, diagonal
            pairs(a,b) :- wide(a,b), enabled(fz1_k).
            diagonal(a) :- wide(a,a), wide(a,_).
        "
        ))
        .unwrap();
        let expected = execute(&v, "deferred-broad-control");
        let (made, plans) = defer(&v, true, true).unwrap();
        assert_eq!(plans.factors.len(), 1);
        assert!(made.columns_of("wide").is_none());
        assert_eq!(execute(&made, "deferred-broad"), expected);
        assert_eq!(expected["pairs"].len(), 3);
    }

    #[test]
    fn one_requested_endpoint_can_stream_the_other_factor() {
        let v = parse_version(&format!(
            "{SOURCE}
            .decl selected(a: number,b: number)
            .output selected
            selected(a,b) :- enabled(a), wide(a,b).
        "
        ))
        .unwrap();
        let (made, plans) = defer(&v, true, true).unwrap();
        assert_eq!(plans.one_sided, ["wide column 0"]);
        assert_eq!(plans.factors.len(), 1);
        assert!(made.columns_of("wide").is_none());
        assert_eq!(execute(&made, "stream-endpoint")["selected"], [["1", "1"], ["2", "1"]]);
    }

    #[test]
    fn reused_product_cache_has_an_exact_witness_limit() {
        for size in [64, 65] {
            let mut src = String::from(
                "
                .decl lhs(a: number, k: number)
                .decl rhs(b: number, k: number)
                .decl wide(a: number, b: number)
                .decl lefts(a: number)
                .decl rights(b: number)
                .output lefts, rights
                wide(a,b) :- lhs(a,k), rhs(b,k).
                lefts(a) :- wide(a,_).
                rights(b) :- wide(_,b).
            ",
            );
            for n in 0..size {
                src.push_str(&format!("lhs({n},0).\n"));
            }
            for n in 0..64 {
                src.push_str(&format!("rhs({n},0).\n"));
            }
            let v = parse_version(&src).unwrap();
            let (made, plans) = defer(&v, true, true).unwrap();
            assert_eq!(plans.factors.len(), 1);
            let guard = made.nullary().into_iter().find(|r| r.starts_with("rr_pair_cache_")).unwrap();
            assert_eq!(made.columns_of(&guard).unwrap().len(), 1);
            assert!(made.rules().any(|r| made.rel_name(r.head().rel()) == guard && matches!(r.head().args().collect::<Vec<_>>()[..], [TermRef::Int(1)])));
            let pairs = declared(&made, |r| r.starts_with("rr_pair_cache_") && r.ends_with("_pairs")).unwrap();
            let rows = execute(&with_outputs(&made, std::slice::from_ref(&pairs)), &format!("bounded-pairs-{size}"));
            assert_eq!(rows[&pairs].len(), if size == 64 { 4096 } else { 0 });
            let expected = |n| (0..n).map(|i: usize| vec![i.to_string()]).collect::<BTreeSet<_>>();
            assert_eq!(rows["lefts"].iter().cloned().collect::<BTreeSet<_>>(), expected(size));
            assert_eq!(rows["rights"].iter().cloned().collect::<BTreeSet<_>>(), expected(64));
        }
    }

    /// A reused product whose readers, or the rules over them, read an answer only a node's document gives (`value`)
    /// is factorized but not cached: a cache's readers are the cross-file part's, which has no documents to answer
    /// it from. Decided from the program, so a run is never prepared again to find it (decision 0506; ported from
    /// cost-classifier-metal-rf's tree form of this test).
    #[test]
    fn a_product_read_with_a_documents_answer_is_not_cached() {
        let src = |reader: &str| format!("
            .decl lhs(a: number, k: number)
            .decl rhs(b: number, k: number)
            .decl wide(a: number, b: number)
            .decl value(n: number, v: symbol)
            .decl lefts(a: number)
            .decl rights(b: number)
            .decl over(a: number)
            .output rights, over
            wide(a,b) :- lhs(a,k), rhs(b,k).
            {reader}
            rights(b) :- wide(_,b).
            over(a) :- lefts(a).
        ");
        let cached = |reader: &str| {
            let v = parse_version(&src(reader)).unwrap();
            let (made, plans) = defer(&v, true, true).unwrap();
            assert_eq!(plans.factors.len(), 1, "{reader}: factorized");
            made.nullary().into_iter().any(|r| r.starts_with("rr_pair_cache_"))
        };
        assert!(cached("lefts(a) :- wide(a,_)."), "a plain reader keeps its cache");
        assert!(!cached("lefts(a) :- wide(a,_), value(a, _)."), "a reader of `value`");
        assert!(!cached("lefts(a) :- wide(a,_).\n over(a) :- lefts(a), value(a, _)."), "a rule over a reader reads `value`");
    }

    #[test]
    fn aggregate_recursive_and_open_negative_readers_keep_their_relation() {
        for reader in ["out(a) :- enabled(a), !wide(_,_).", "out(a) :- a = count : { wide(_,_) }.", "wide(a,b) :- wide(a,k), rhs(b,k). out(a) :- wide(a,_)."] {
            let v = parse_version(&format!("{SOURCE} .decl out(a: number)\n.output out\n{reader}")).unwrap();
            let (made, plans) = defer(&v, true, true).unwrap();
            assert!(plans.pairs.is_empty() && plans.factors.is_empty() && plans.shared.is_empty());
            assert_eq!(made.digest(), v.digest(), "{}", made.show());
        }
    }
}
