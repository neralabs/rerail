//! Joins deferred to their readers' requests (decision 0488, 0477 on cost-classifier-metal-rf). A binary positive
//! join projected across a hidden key keeps correlated requests together: the pairs or the endpoint its readers
//! request drive it (`upstream::install_named`, `DEFERRED_PAIRS`). A frequently requested endpoint shares its side's
//! witnesses across the requested pairs (`share`). A join its readers request nothing of is held as its two reduced
//! factors, which meet in each reader, with a small pair cache where several read it (`file_local::factorize_intermediates`).
//!
//! On the version (decision 0276): each step a derivation of the one before (`Program::derive`), its rules read in place.

use super::{DEFERRED_PAIRS, PREFIX, Program, rewritten, same, upstream};
use crate::code_rule_dsl::logic::plan::body::{atom_vars, lit_vars};
use crate::code_rule_dsl::logic::profile::author_joins::{Access, Join};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut as _, ProgramFields as F};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, RuleId, RuleRef, T, TermRef, Version};
use crate::code_rule_dsl::typing::file_local;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Requests of one endpoint past which its side's witnesses are shared (`share`).
const SHARE_AFTER: i64 = 8;

/// What the pass did, for its decisions.
#[derive(Default)]
pub(in crate::code_rule_dsl::logic::plan::corpus) struct Plans {
    pub pairs: Vec<String>,
    pub one_sided: Vec<String>,
    pub factors: Vec<String>,
    pub shared: Vec<String>,
    pub cached: bool,
    pub author: Vec<Join>,
}

/// The name of variable `s` of rule `r`.
fn var_name<'v>(v: &'v Version, r: RuleRef<'_>, s: crate::code_rule_dsl::logic::program_image::Slot) -> &'v str {
    v.text(r.var(s))
}

/// The variables literal `l` of rule `r` reads, by name.
fn names_of(v: &Version, r: RuleRef<'_>, l: crate::code_rule_dsl::logic::program_image::LitRef<'_>) -> BTreeSet<String> {
    lit_vars(l).iter().map(|s| var_name(v, r, s).to_owned()).collect()
}

/// The place of rule `r`'s request: a positive, ungated atom over its head's own terms, of a demand made for its head
/// (`product_demand::PREFIX`, `upstream::PREFIX`, `DEFERRED_PAIRS`).
fn guard(v: &Version, r: RuleRef<'_>) -> Option<usize> {
    let head = v.rel_name(r.head().rel());
    let args: Vec<TermRef<'_>> = r.head().args().collect();
    r.body().position(|l| match l.view() {
        Lit::Atom(a) => {
            !a.neg()
                && !a.gated()
                && a.args().count() == args.len()
                && a.args().zip(&args).all(|(x, y)| same(x, *y))
                && [PREFIX, upstream::PREFIX, DEFERRED_PAIRS].iter().any(|p| v.rel_name(a.rel()).strip_prefix(p) == Some(head))
        }
        _ => false,
    })
}

/// Rule `r` of two distinct head variables, its body positive atoms of plain terms and comparisons other than `=`,
/// split (its request aside) at a key into two parts, each with an atom reading the key: the key, and the parts'
/// literals by place.
fn split(v: &Version, r: RuleRef<'_>) -> Option<(String, Vec<Vec<usize>>)> {
    let head: Vec<TermRef<'_>> = r.head().args().collect();
    let [TermRef::Var(a), TermRef::Var(b)] = head.as_slice() else { return None };
    if a == b {
        return None;
    }
    let plain = |t: TermRef<'_>| super::plain(t) || matches!(t, TermRef::Wild);
    let fits = r.body().all(|l| match l.view() {
        Lit::Atom(a) => !a.neg() && !a.gated() && a.args().all(plain),
        Lit::Cmp { l, op, r } => op != CmpOp::Eq && super::plain(l) && super::plain(r),
        _ => false,
    });
    if !fits {
        return None;
    }
    let (key, parts) = crate::code_rule_dsl::logic::plan::keeps::factors_without(v, r, guard(v, r))?;
    let lits: Vec<_> = r.body().collect();
    let keyed = |part: &Vec<usize>| part.iter().any(|&i| matches!(lits[i].view(), Lit::Atom(a) if atom_vars(a).iter().any(|s| var_name(v, r, s) == key)));
    (parts.len() == 2 && parts.iter().all(keyed)).then_some((key, parts))
}

/// Whether reader `r`'s read at literal `at` has its columns `cols` bound by the rule's other atoms: each a plain
/// term, and their variables all read by one positive, ungated atom of another relation (or none to bind).
fn reader_binds(v: &Version, r: RuleRef<'_>, at: usize, cols: &[usize]) -> bool {
    let lits: Vec<_> = r.body().collect();
    let Some(read) = lits[at].atom() else { return false };
    let args: Vec<TermRef<'_>> = read.args().collect();
    let mut bound = BTreeSet::new();
    for &c in cols {
        if !super::plain(args[c]) {
            return false;
        }
        if let TermRef::Var(s) = args[c] {
            bound.insert(var_name(v, r, s).to_owned());
        }
    }
    bound.is_empty()
        || lits.iter().any(|l| match l.view() {
            Lit::Atom(a) => {
                a.rel() != read.rel() && !a.neg() && !a.gated() && bound.iter().all(|x| atom_vars(a).iter().any(|s| var_name(v, r, s) == x))
            }
            _ => false,
        })
}

/// `rel`'s column names.
fn column_names(p: &Program<'_>, rel: &str) -> Vec<String> {
    p.columns(rel).unwrap_or_default().into_iter().map(|(c, _)| c).collect()
}

/// `rels` qualified `no_inline` in a step of their own.
fn no_inline(p: &mut Program<'_>, rels: &[&str]) -> Result<(), String> {
    let mut qualifiers = p.f.qualifiers.clone();
    for rel in rels {
        qualifiers.entry((*rel).to_owned()).or_default().insert("no_inline".to_owned());
    }
    let made = p.derive(&mut |_, _, _, _| Ok(false), &mut |_, b| {
        b.fields().set_field(Field::Qualifiers(&qualifiers));
        Ok(())
    })?;
    p.set(made);
    Ok(())
}

/// `input` with its binary joins deferred to their readers' requests (the module comment): the version (`input`
/// where nothing was), and what was done. Fanout sharing (`adaptive`) and the pair caches (`cache_pairs`) test their
/// sizes on the relations as they are when the rules run.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn rewrite(input: &Arc<Version>, adaptive: bool, cache_pairs: bool) -> Result<(Arc<Version>, Plans), String> {
    let mut p = Program::of(input);
    let mut plans = Plans::default();
    let v = p.v();
    let rules: Vec<RuleRef<'_>> = v.rules().collect();
    let mut definitions = BTreeMap::<String, usize>::new();
    let mut readers = BTreeMap::<String, Vec<(usize, usize)>>::new();
    for (i, r) in rules.iter().enumerate() {
        *definitions.entry(p.head_rel(*r).to_owned()).or_default() += 1;
        for (j, l) in r.body().enumerate() {
            if let Some(a) = l.atom() {
                readers.entry(v.rel_name(a.rel()).to_owned()).or_default().push((i, j));
            }
        }
    }
    // A connected-demand pass may already have restricted the producer: its request is the join's access, even
    // where this pass finds no more to do.
    let mut explained = BTreeSet::new();
    for r in &rules {
        let head = p.head_rel(*r);
        if let Some(at) = guard(v, *r)
            && explained.insert(head.to_owned())
        {
            let table = r.body().nth(at).and_then(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).expect("the request");
            plans.author.push(Join { relation: head.to_owned(), access: Access::Requests { table, columns: column_names(&p, head) }, readers: Vec::new(), placement: None });
        }
    }
    let shapes = F::shapes(v);
    let tapes = F::tapes(v);
    let mut candidates = BTreeSet::new();
    // The joins already requested, each by its relation and its place among the relation's rules.
    let mut guarded: Vec<(String, usize)> = Vec::new();
    let mut nth = BTreeMap::<&str, usize>::new();
    for r in &rules {
        let rel = p.head_rel(*r);
        let place = nth.entry(rel).or_default();
        let at = *place;
        *place += 1;
        if r.head().args().count() != 2
            || r.body().count() < 2
            || !readers.contains_key(rel)
            || !upstream::eligible(&p, rel)
            || p.f.qualifiers.get(rel).is_some_and(|q| !q.is_empty())
            || tapes.contains_key(rel)
            || p.f.demand.iter().any(|d| d == rel)
            || shapes.iter().any(|s| s.head == rel || s.lit == rel)
            || p.f.asked_by.values().flatten().any(|refs| refs.contains(rel))
        {
            continue;
        }
        if split(v, *r).is_some() {
            if guard(v, *r).is_some() {
                if adaptive {
                    guarded.push((rel.to_owned(), at));
                }
            } else if definitions[rel] == 1 {
                candidates.insert(rel.to_owned());
            }
        }
    }
    if candidates.is_empty() && guarded.is_empty() {
        return Ok((input.clone(), plans));
    }
    let Ok(sccs) = Program::components(v) else {
        return Ok((input.clone(), plans));
    };
    let recursive: BTreeSet<String> = sccs
        .into_iter()
        .filter(|c| c.len() > 1)
        .flatten()
        .chain(rules.iter().filter(|r| r.body().any(|l| l.atom().is_some_and(|a| a.rel() == r.head().rel()))).map(|r| p.head_rel(*r).to_owned()))
        .collect();
    let mut pairs = Vec::new();
    let mut broad = BTreeSet::new();
    let mut open_readers = BTreeMap::new();
    for name in candidates {
        if recursive.contains(&name) {
            continue;
        }
        let uses = &readers[&name];
        let columns = column_names(&p, &name);
        let mut descriptions = BTreeSet::new();
        for &(i, j) in uses {
            let open: Vec<&str> = (0..2).filter(|c| !reader_binds(v, rules[i], j, &[*c])).map(|c| columns[c].as_str()).collect();
            if !open.is_empty() {
                descriptions.insert((p.head_rel(rules[i]).to_owned(), format!("supplies no lookup value for {}", open.join(", "))));
            }
        }
        open_readers.insert(name.clone(), descriptions.into_iter().collect::<Vec<_>>());
        let lit = |i: usize, j: usize| rules[i].body().nth(j).expect("a read");
        if uses.iter().any(|&(i, j)| !matches!(lit(i, j).view(), Lit::Atom(a) if !a.gated())) {
            continue;
        }
        let negated = |i: usize, j: usize| lit(i, j).atom().is_some_and(|a| a.neg());
        let mask = [&[0, 1][..], &[0][..], &[1][..]].into_iter().find(|cols| uses.iter().all(|&(i, j)| reader_binds(v, rules[i], j, cols)));
        let positive = uses.iter().all(|&(i, j)| !negated(i, j));
        if let Some(cols) = mask {
            pairs.push((name, cols.to_vec(), positive));
        } else if positive {
            broad.insert(name);
        }
    }
    // A request installed changes the program's rules: every read is classified first, above.
    drop(rules);
    for (name, cols, positive) in pairs {
        let rel = name.as_str();
        if !upstream::install_named(&mut p, rel, &cols, &BTreeSet::new(), false, DEFERRED_PAIRS)? {
            continue;
        }
        no_inline(&mut p, &[rel])?;
        let columns = column_names(&p, rel);
        plans.author.push(Join {
            relation: rel.to_owned(),
            access: Access::Requests { table: format!("{DEFERRED_PAIRS}{rel}"), columns: cols.iter().map(|&c| columns[c].clone()).collect() },
            readers: open_readers.remove(rel).unwrap_or_default(),
            placement: None,
        });
        if cols.len() == 1 {
            plans.one_sided.push(format!("{rel} column {}", cols[0]));
            // A request for one endpoint can still match the whole of the other factor: its request stays on that
            // factor, and positive readers read the join directly.
            if adaptive && positive {
                broad.insert(rel.to_owned());
            }
            continue;
        }
        plans.pairs.push(rel.to_owned());
        if adaptive {
            guarded.push((rel.to_owned(), 0));
        }
    }
    for (rel, at) in guarded {
        if !recursive.contains(&rel)
            && let Some(label) = share(&mut p, &rel, at)?
        {
            plans.shared.push(label);
        }
    }
    let now = p.made.clone().unwrap_or_else(|| input.clone());
    let (factored, factors, choices) = file_local::factorize_intermediates(&now, &broad, adaptive && cache_pairs)?;
    let out = factored.unwrap_or(now);
    plans.factors = factors;
    plans.cached = choices.values().any(Option::is_some);
    for (relation, cache) in choices {
        let readers = open_readers.remove(&relation).unwrap_or_default();
        plans.author.push(Join { relation, access: Access::Factors { cache }, readers, placement: None });
    }
    for (relation, readers) in open_readers {
        if !readers.is_empty() {
            plans.author.push(Join { relation, access: Access::Full, readers, placement: None });
        }
    }
    Ok((out, plans))
}

/// The `at`th rule of `rel`, requested (`guard`), with its larger part of two literals or more shared by endpoint
/// once `SHARE_AFTER` requests ask for that endpoint: `hot(e)` its endpoints so asked for, `factor(e, k)` their part's
/// witnesses, made once, and the rule read through them for a hot endpoint and as written for the others.
fn share(p: &mut Program<'_>, rel: &str, at: usize) -> Result<Option<String>, String> {
    let v = p.v();
    let Some(rule) = v.rules().filter(|r| p.head_rel(*r) == rel).nth(at) else { return Ok(None) };
    let (Some((key, parts)), Some(request_at)) = (split(v, rule), guard(v, rule)) else { return Ok(None) };
    let lits: Vec<_> = rule.body().collect();
    let Some((side, part)) = parts.iter().enumerate().max_by_key(|(_, part)| part.len()) else { return Ok(None) };
    if part.len() < 2 {
        return Ok(None);
    }
    let vars: BTreeSet<String> = part.iter().flat_map(|&i| names_of(v, rule, lits[i])).collect();
    let head: Vec<TermRef<'_>> = rule.head().args().collect();
    let Some(col) = head.iter().position(|t| matches!(t, TermRef::Var(s) if vars.contains(var_name(v, rule, *s)))) else { return Ok(None) };
    let TermRef::Var(e) = head[col] else { return Ok(None) };
    let endpoint = var_name(v, rule, e).to_owned();
    let key_ty = part.iter().find_map(|&i| {
        let a = lits[i].atom()?;
        let c = a.args().position(|t| matches!(t, TermRef::Var(s) if var_name(v, rule, s) == key))?;
        p.columns(v.rel_name(a.rel()))?.get(c).map(|(_, ty)| ty.clone())
    });
    let (Some(key_ty), Some(endpoint_ty)) = (key_ty, p.columns(rel).and_then(|c| c.get(col).map(|(_, ty)| ty.clone()))) else { return Ok(None) };
    let request = lits[request_at].atom().map(|a| v.rel_name(a.rel()).to_owned()).expect("the request");
    let base = file_local::helper_name("rr_deferred_", rel, &v.show_rule(rule), |n| ["hot", "factor"].iter().any(|s| p.columns(&format!("{n}_{s}")).is_some()), &mut Default::default());
    let (hot, factor) = (format!("{base}_hot"), format!("{base}_factor"));
    let mut count = "rr_fanout".to_owned();
    while count == endpoint {
        count.push('_');
    }
    let label = format!("{rel} column {col} ({} literals)", part.len());
    let (part, other): (Vec<usize>, Vec<usize>) = (part.clone(), parts[1 - side].clone());
    let id: RuleId = rule.id();
    let mut qualifiers = p.f.qualifiers.clone();
    for r in [&hot, &factor] {
        qualifiers.entry(r.clone()).or_default().insert("no_inline".to_owned());
    }
    let made = p.derive(
        &mut |first, v, r, b| {
            if r.id() != id {
                return Ok(false);
            }
            // A cold endpoint's requests keep the rule as written.
            let mut rb = rewritten(first, v, r, b);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            rb.not(hot.as_str(), &[T::Var(&endpoint)]).finish().map(|_| true)
        },
        &mut |v, b| {
            b.fields().declare(&hot, &[("endpoint", endpoint_ty.as_str())]);
            b.fields().declare(&factor, &[("endpoint", endpoint_ty.as_str()), ("witness", key_ty.as_str())]);
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            let r = v.rule(id).expect("the shared rule");
            let lits: Vec<_> = r.body().collect();
            let mut group = [T::Wild, T::Wild];
            group[col] = T::Var(&endpoint);
            b.rule()
                .head(hot.as_str(), &[T::Var(&endpoint)])
                .atom(request.as_str(), &group)
                .agg(T::Var(&count), &crate::code_rule_dsl::logic::front::ast::AggOp::Count, None, request.as_str(), &group)
                .cmp(T::Var(&count), CmpOp::Ge, T::Int(SHARE_AFTER))
                .finish()?;
            let mut rb = b.rule().head(factor.as_str(), &[T::Var(&endpoint), T::Var(&key)]).atom(hot.as_str(), &[T::Var(&endpoint)]);
            for &i in &part {
                rb = rb.lit(v, r, lits[i]);
            }
            rb.finish()?;
            let mut rb = b.rule().head_from(v, r, r.head()).lit(v, r, lits[request_at]).atom(factor.as_str(), &[T::Var(&endpoint), T::Var(&key)]);
            for &i in &other {
                rb = rb.lit(v, r, lits[i]);
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

    fn parse(text: &str) -> Arc<Version> {
        crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()
    }

    /// `v` with `rels` outputs too.
    fn with_outputs(v: &Arc<Version>, rels: &[&str]) -> Arc<Version> {
        let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(v);
        for rel in rels {
            b.fields().add_output(rel);
        }
        b.finish().unwrap().0
    }

    fn execute(v: &Arc<Version>, case: &str) -> BTreeMap<String, Vec<Vec<String>>> {
        let root = crate::code_rule_dsl::fixtures::temp().with_extension(case);
        std::fs::create_dir_all(&root).unwrap();
        let out = root.join("out");
        corpus::run_read(v, &root, &out, "program", Binding::Cpu, 4096, &Default::default(), &serde_json::Value::Null).unwrap();
        let outputs = Outputs::open(&out).unwrap();
        let rows = F::output_names(&**v)
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

    #[test]
    fn hot_and_cold_requests_share_only_their_own_answers() {
        let v = parse(&format!(
            "{SOURCE}
            .decl yes(a: number, b: number)
            .decl missing(a: number, b: number)
            .output yes, missing
            yes(a,b) :- request(a,b), wide(a,b).
            missing(a,b) :- request(a,b), !wide(a,b).
        "
        ));
        let expected = execute(&v, "deferred-control");
        let (v, plans) = rewrite(&v, true, true).unwrap();
        assert_eq!(plans.pairs, ["wide"]);
        assert_eq!(plans.shared.len(), 1);
        let names = F::declared_names(&*v);
        let hot = names.iter().find(|r| r.ends_with("_hot")).unwrap().clone();
        let factor = names.iter().find(|r| r.ends_with("_factor")).unwrap().clone();
        let v = with_outputs(&v, &[&hot, &factor]);
        let mut actual = execute(&v, "deferred-adaptive");
        assert_eq!(actual.remove(&hot).unwrap(), [["1"]]);
        assert_eq!(actual.remove(&factor).unwrap(), [["1", "10"], ["1", "20"]]);
        assert_eq!(actual, expected);
        assert_eq!(actual["yes"], [["1", "1"], ["2", "1"], ["3", "2"]]);
        assert_eq!(actual["missing"].len(), 8);
    }

    #[test]
    fn broad_readers_share_reduced_factors_and_fresh_witnesses() {
        let v = parse(&format!(
            "{SOURCE}
            .decl pairs(a: number, b: number)
            .decl diagonal(a: number)
            .output pairs, diagonal
            pairs(a,b) :- wide(a,b), enabled(fz1_k).
            diagonal(a) :- wide(a,a), wide(a,_).
        "
        ));
        let expected = execute(&v, "deferred-broad-control");
        let (v, plans) = rewrite(&v, true, true).unwrap();
        assert_eq!(plans.factors.len(), 1);
        assert!(!F::declares(&*v, "wide"));
        assert_eq!(execute(&v, "deferred-broad"), expected);
        assert_eq!(expected["pairs"].len(), 3);
    }

    #[test]
    fn one_requested_endpoint_can_stream_the_other_factor() {
        let v = parse(&format!(
            "{SOURCE}
            .decl selected(a: number,b: number)
            .output selected
            selected(a,b) :- enabled(a), wide(a,b).
        "
        ));
        let (v, plans) = rewrite(&v, true, true).unwrap();
        assert_eq!(plans.one_sided, ["wide column 0"]);
        assert_eq!(plans.factors.len(), 1);
        assert!(!F::declares(&*v, "wide"));
        assert_eq!(execute(&v, "stream-endpoint")["selected"], [["1", "1"], ["2", "1"]]);
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
            let (v, plan) = rewrite(&parse(&src), true, true).unwrap();
            assert_eq!(plan.factors.len(), 1);
            let guard = F::nullary(&*v).into_iter().find(|r| r.starts_with("rr_pair_cache_")).unwrap();
            assert_eq!(F::columns_of(&*v, &guard).unwrap().len(), 1);
            assert!(v.rules().any(|r| v.rel_name(r.head().rel()) == guard && matches!(r.head().args().collect::<Vec<_>>().as_slice(), [TermRef::Int(1)])));
            let pairs = F::declared_names(&*v).into_iter().find(|r| r.starts_with("rr_pair_cache_") && r.ends_with("_pairs")).unwrap();
            let v = with_outputs(&v, &[&pairs]);
            let rows = execute(&v, &format!("bounded-pairs-{size}"));
            assert_eq!(rows[&pairs].len(), if size == 64 { 4096 } else { 0 });
            let expected = |n| (0..n).map(|i: usize| vec![i.to_string()]).collect::<BTreeSet<_>>();
            assert_eq!(rows["lefts"].iter().cloned().collect::<BTreeSet<_>>(), expected(size));
            assert_eq!(rows["rights"].iter().cloned().collect::<BTreeSet<_>>(), expected(64));
        }
    }

    #[test]
    fn aggregate_recursive_and_open_negative_readers_keep_their_relation() {
        for reader in ["out(a) :- enabled(a), !wide(_,_).", "out(a) :- a = count : { wide(_,_) }.", "wide(a,b) :- wide(a,k), rhs(b,k). out(a) :- wide(a,_)."] {
            let v = parse(&format!("{SOURCE} .decl out(a: number)\n.output out\n{reader}"));
            let (after, plans) = rewrite(&v, true, true).unwrap();
            assert!(plans.pairs.is_empty() && plans.factors.is_empty() && plans.shared.is_empty());
            assert!(Arc::ptr_eq(&after, &v), "the version unchanged: {reader}");
        }
    }
}
