//! A binary positive join projected across a hidden key. Keep correlated requests together.
//! A frequently requested endpoint can share its side's witnesses across the requested pairs.

use crate::code_rule_dsl::logic::profile::author_joins::{Access, Join};
use super::{DEFERRED_PAIRS, PREFIX, atom, plain, upstream};
use crate::code_rule_dsl::logic::front::{
    AggOp, Atom, Literal, Program, Rule, Term, components_of,
};
use crate::code_rule_dsl::logic::plan::keeps;
use crate::code_rule_dsl::typing::file_local;
use std::collections::{BTreeMap, BTreeSet};

const SHARE_AFTER: i64 = 8;

#[derive(Default)]
pub(in crate::code_rule_dsl::logic::plan::corpus) struct Plans {
    pub pairs: Vec<String>,
    pub one_sided: Vec<String>,
    pub factors: Vec<String>,
    pub shared: Vec<String>,
    pub cached: bool,
    pub author: Vec<Join>,
}

fn guard(rule: &Rule) -> Option<&Atom> {
    let head = rule.head.rel.as_str();
    rule.body.iter().find_map(|lit| match lit {
        Literal::Atom(a)
            if !a.neg
                && !a.gated
                && a.args == rule.head.args
                && [PREFIX, upstream::PREFIX, DEFERRED_PAIRS]
                    .iter()
                    .any(|p| a.rel.strip_prefix(p) == Some(head)) =>
        {
            Some(a)
        }
        _ => None,
    })
}

fn split(rule: &Rule) -> Option<(String, Vec<Vec<Literal>>)> {
    let [Term::Var(a), Term::Var(b)] = rule.head.args.as_slice() else {
        return None;
    };
    if a == b
        || !rule.body.iter().all(|l| match l {
            Literal::Atom(a) => {
                !a.neg && !a.gated && a.args.iter().all(|t| plain(t) || matches!(t, Term::Wild))
            }
            Literal::Cmp { l, r, op } => *op != "=" && plain(l) && plain(r),
            _ => false,
        })
    {
        return None;
    }
    let request = guard(rule);
    let mut producer = rule.clone();
    producer
        .body
        .retain(|lit| request.is_none_or(|g| lit.atom() != Some(g)));
    let (key, parts) = keeps::factors(&producer)?;
    (parts.len() == 2
        && parts.iter().all(|part| {
            part.iter()
                .filter_map(Literal::atom)
                .any(|a| a.vars().contains(&key))
        }))
    .then_some((key, parts))
}

fn reader_binds(rule: &Rule, read: &Atom, cols: &[usize]) -> bool {
    let mut bound = BTreeSet::new();
    for &c in cols {
        if !plain(&read.args[c]) {
            return false;
        }
        read.args[c].vars(&mut bound);
    }
    bound.is_empty()
        || rule.body.iter().any(|lit| {
            matches!(lit, Literal::Atom(a)
        if a.rel != read.rel && !a.neg && !a.gated && bound.is_subset(&a.vars()))
        })
}

/// Determine which restrictions the readers provide. Fanout sharing and small pair caches
/// evaluate their size guards on the current relations.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn rewrite(
    p: &mut Program,
    adaptive: bool,
    cache_pairs: bool,
) -> Plans {
    let mut plans = Plans::default();
    let mut definitions = BTreeMap::<_, usize>::new();
    let mut readers = BTreeMap::<_, Vec<_>>::new();
    for (i, r) in p.rules.iter().enumerate() {
        *definitions.entry(r.head.rel.clone()).or_default() += 1;
        for (j, l) in r.body.iter().enumerate() {
            if let Some(a) = l.atom() {
                readers.entry(a.rel.clone()).or_default().push((i, j));
            }
        }
    }
    // A connected-demand pass may already have restricted the producer. Include its final guard
    // even if this pass finds no additional factorization.
    let mut explained = BTreeSet::new();
    for rule in &p.rules {
        if let Some(request) = guard(rule)
            && explained.insert(rule.head.rel.clone())
        {
            plans.author.push(Join {
                relation: rule.head.rel.to_string(),
                access: Access::Requests { table: request.rel.to_string(), columns: p.decls[&rule.head.rel].iter().map(|c| c.0.to_string()).collect() },
                readers: Vec::new(),
                placement: None,
            });
        }
    }
    let mut candidates = BTreeSet::new();
    let mut guarded = Vec::new();
    for (i, rule) in p.rules.iter().enumerate() {
        let rel = rule.head.rel.as_str();
        if rule.head.args.len() != 2
            || rule.body.len() < 2
            || !readers.contains_key(&rule.head.rel)
            || !upstream::eligible(p, rel)
            || p.qualifiers.get(rel).is_some_and(|q| !q.is_empty())
            || p.tapes.contains_key(rel)
            || p.demand.iter().any(|r| r == rel)
            || p.shapes.iter().any(|s| s.head == rel || s.lit == rel)
            || p.asked_by.values().flatten().any(|refs| refs.contains(rel))
        {
            continue;
        }
        if let Some((key, parts)) = split(rule) {
            if guard(rule).is_some() {
                if adaptive {
                    guarded.push((i, key, parts));
                }
            } else if definitions[&rule.head.rel] == 1 {
                candidates.insert(rule.head.rel.clone());
            }
        }
    }
    if candidates.is_empty() && guarded.is_empty() {
        return plans;
    }
    let Ok(sccs) = components_of(p) else {
        return plans;
    };
    let recursive: BTreeSet<_> = sccs
        .into_iter()
        .filter(|c| c.len() > 1)
        .flatten()
        .chain(
            p.rules
                .iter()
                .filter(|r| {
                    r.body
                        .iter()
                        .any(|l| l.atom().is_some_and(|a| a.rel == r.head.rel))
                })
                .map(|r| r.head.rel.to_string()),
        )
        .collect();
    let mut pairs = Vec::new();
    let mut broad = BTreeSet::new();
    let mut open_readers = BTreeMap::new();
    for name in candidates {
        let rel = name.as_str();
        if recursive.contains(rel) {
            continue;
        }
        let uses = &readers[&name];
        let mut descriptions = BTreeSet::new();
        for &(i, j) in uses {
            let reader = &p.rules[i];
            let read = reader.body[j].atom().unwrap();
            let open: Vec<_> = (0..2).filter(|c| !reader_binds(reader, read, &[*c]))
                .map(|c| p.decls[&name][c].0.as_str()).collect();
            if !open.is_empty() {
                descriptions.insert((reader.head.rel.to_string(), format!("supplies no lookup value for {}", open.join(", "))));
            }
        }
        open_readers.insert(name.to_string(), descriptions.into_iter().collect::<Vec<_>>());
        if uses
            .iter()
            .any(|&(i, j)| !matches!(&p.rules[i].body[j], Literal::Atom(a) if !a.gated))
        {
            continue;
        }
        let mask = [&[0, 1][..], &[0][..], &[1][..]].into_iter().find(|cols| {
            uses.iter()
                .all(|&(i, j)| reader_binds(&p.rules[i], p.rules[i].body[j].atom().unwrap(), cols))
        });
        if let Some(cols) = mask {
            let positive = uses.iter().all(|&(i, j)| !p.rules[i].body[j].atom().unwrap().neg);
            pairs.push((name, cols, positive));
        } else if uses
            .iter()
            .all(|&(i, j)| !p.rules[i].body[j].atom().unwrap().neg)
        {
            broad.insert(rel.to_owned());
        }
    }
    // Inserting a guard shifts the literal positions. Classify all reads before changing any rule.
    for (name, cols, positive) in pairs {
        let rel = name.as_str();
        if upstream::install_named(p, rel, cols, &BTreeSet::new(), false, DEFERRED_PAIRS) {
            p.qualifiers
                .entry(rel.to_owned())
                .or_default()
                .insert("no_inline".into());
            plans.author.push(Join {
                relation: rel.to_owned(),
                access: Access::Requests { table: format!("{DEFERRED_PAIRS}{rel}"), columns: cols.iter().map(|&c| p.decls[rel][c].0.to_string()).collect() },
                readers: open_readers.remove(rel).unwrap_or_default(),
                placement: None,
            });
            if cols.len() == 1 {
                plans.one_sided.push(format!("{rel} column {}", cols[0]));
                // A request for one endpoint can still match the entire opposite factor.
                // Keep its demand guard on that factor and let positive readers consume the join directly.
                if adaptive && positive { broad.insert(rel.to_owned()); }
                continue;
            }
            plans.pairs.push(rel.to_owned());
            if adaptive {
                let i = p.rules.iter().position(|r| r.head.rel == name).unwrap();
                if let Some((key, parts)) = split(&p.rules[i]) {
                    guarded.push((i, key, parts));
                }
            }
        }
    }
    for (i, key, parts) in guarded {
        if !recursive.contains(p.rules[i].head.rel.as_str()) {
            if let Some(name) = share(p, i, &key, parts) {
                plans.shared.push(name);
            }
        }
    }
    let (factors, choices) = file_local::factorize_intermediates(p, &broad, adaptive && cache_pairs);
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
    plans
}

fn share(p: &mut Program, i: usize, key: &str, parts: Vec<Vec<Literal>>) -> Option<String> {
    let rule = &p.rules[i];
    // Share a compound calculation. A single indexed input already answers this probe in place.
    let (side, part) = parts
        .iter()
        .enumerate()
        .max_by_key(|(_, part)| part.len())?;
    if part.len() < 2 {
        return None;
    }
    let vars: BTreeSet<_> = part.iter().flat_map(Literal::vars).collect();
    let col = rule
        .head
        .args
        .iter()
        .position(|t| matches!(t, Term::Var(v) if vars.contains(v.as_str())))?;
    let endpoint = rule.head.args[col].clone();
    let key_term = Term::Var(key.into());
    let key_ty = part.iter().filter_map(Literal::atom).find_map(|a| {
        let c = a.args.iter().position(|t| *t == key_term)?;
        p.decls.get(&a.rel)?.get(c).map(|(_, ty)| ty.clone())
    })?;
    let endpoint_col = (
        "endpoint".into(),
        p.decls.get(&rule.head.rel)?.get(col)?.1.clone(),
    );
    let request = guard(rule)?.clone();
    let mut args = [Term::Wild, Term::Wild];
    args[col] = endpoint.clone();
    let group = atom(request.rel.to_string(), &args);
    let base = file_local::helper_name(
        "rr_deferred_",
        &rule.head.rel,
        &rule.to_string(),
        |n| {
            ["hot", "factor"]
                .iter()
                .any(|suffix| p.decls.contains_key(format!("{n}_{suffix}").as_str()))
        },
        &mut Default::default(),
    );
    let hot = atom(format!("{base}_hot"), std::slice::from_ref(&endpoint));
    let factor = atom(format!("{base}_factor"), &[endpoint.clone(), key_term]);
    let mut count = "rr_fanout".to_owned();
    while hot.vars().contains(&count) {
        count.push('_');
    }
    let hot_rule = Rule {
        head: hot.clone(),
        body: [
            Literal::Atom(group.clone()),
            Literal::Agg {
                var: count.clone(),
                op: AggOp::Count,
                over: None,
                atom: group,
            },
            Literal::Cmp {
                l: Term::Var(count.into()),
                op: ">=",
                r: Term::Int(SHARE_AFTER),
            },
        ]
        .into(),
    };
    let mut materialize = vec![Literal::Atom(hot.clone())];
    materialize.extend(part.iter().cloned());
    let mut use_factor = vec![Literal::Atom(request), Literal::Atom(factor.clone())];
    use_factor.extend(parts[1 - side].iter().cloned());
    let hot_answer = Rule {
        head: rule.head.clone(),
        body: use_factor.into(),
    };
    let label = format!("{} column {} ({} literals)", rule.head.rel, col, part.len());
    p.decls
        .insert(hot.rel.clone(), [endpoint_col.clone()].into());
    p.decls.insert(
        factor.rel.clone(),
        [endpoint_col, ("witness".into(), key_ty)].into(),
    );
    for a in [&hot, &factor] {
        p.qualifiers
            .entry(a.rel.to_string())
            .or_default()
            .insert("no_inline".to_owned());
    }
    let mut cold = hot;
    cold.neg = true;
    p.rules[i].body.push(Literal::Atom(cold));
    p.rules.extend([
        hot_rule,
        Rule {
            head: factor,
            body: materialize.into(),
        },
        hot_answer,
    ]);
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::{
        plan::{corpus, output::Outputs},
        rr::parse,
    };
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

    fn execute(p: &Program, case: &str) -> BTreeMap<String, Vec<Vec<String>>> {
        let root = crate::code_rule_dsl::fixtures::temp().with_extension(case);
        std::fs::create_dir_all(&root).unwrap();
        let out = root.join("out");
        corpus::run_read(
            p,
            &root,
            &out,
            "program",
            Binding::Cpu,
            4096,
            &Default::default(),
            &serde_json::Value::Null,
        )
        .unwrap();
        let outputs = Outputs::open(&out).unwrap();
        let rows = p
            .outputs
            .iter()
            .map(|rel| {
                let mut rows = outputs.get(rel).map(|r| r.shown()).unwrap_or_default();
                rows.sort();
                (rel.clone(), rows)
            })
            .collect();
        drop(outputs);
        std::fs::remove_dir_all(root).unwrap();
        rows
    }

    #[test]
    fn hot_and_cold_requests_share_only_their_own_answers() {
        let mut p = parse(&format!(
            "{SOURCE}
            .decl yes(a: number, b: number)
            .decl missing(a: number, b: number)
            .output yes, missing
            yes(a,b) :- request(a,b), wide(a,b).
            missing(a,b) :- request(a,b), !wide(a,b).
        "
        ))
        .unwrap();
        let expected = execute(&p, "deferred-control");
        let plans = rewrite(&mut p, true, true);
        assert_eq!(plans.pairs, ["wide"]);
        assert_eq!(plans.shared.len(), 1);
        let hot = p
            .decls
            .keys()
            .find(|r| r.ends_with("_hot"))
            .unwrap()
            .to_string();
        let factor = p
            .decls
            .keys()
            .find(|r| r.ends_with("_factor"))
            .unwrap()
            .to_string();
        p.outputs.extend([hot.clone(), factor.clone()]);
        let mut actual = execute(&p, "deferred-adaptive");
        assert_eq!(actual.remove(&hot).unwrap(), [["1"]]);
        assert_eq!(actual.remove(&factor).unwrap(), [["1", "10"], ["1", "20"]]);
        assert_eq!(actual, expected);
        assert_eq!(actual["yes"], [["1", "1"], ["2", "1"], ["3", "2"]]);
        assert_eq!(actual["missing"].len(), 8);
    }

    #[test]
    fn broad_readers_share_reduced_factors_and_fresh_witnesses() {
        let mut p = parse(&format!(
            "{SOURCE}
            .decl pairs(a: number, b: number)
            .decl diagonal(a: number)
            .output pairs, diagonal
            pairs(a,b) :- wide(a,b), enabled(fz1_k).
            diagonal(a) :- wide(a,a), wide(a,_).
        "
        ))
        .unwrap();
        let expected = execute(&p, "deferred-broad-control");
        let plans = rewrite(&mut p, true, true);
        assert_eq!(plans.factors.len(), 1);
        assert!(!p.decls.contains_key("wide"));
        assert_eq!(execute(&p, "deferred-broad"), expected);
        assert_eq!(expected["pairs"].len(), 3);
    }

    #[test]
    fn one_requested_endpoint_can_stream_the_other_factor() {
        let mut p = parse(&format!("{SOURCE}
            .decl selected(a: number,b: number)
            .output selected
            selected(a,b) :- enabled(a), wide(a,b).
        ")).unwrap();
        let plans = rewrite(&mut p, true, true);
        assert_eq!(plans.one_sided, ["wide column 0"]);
        assert_eq!(plans.factors.len(), 1);
        assert!(!p.decls.contains_key("wide"));
        assert_eq!(execute(&p, "stream-endpoint")["selected"], [["1", "1"], ["2", "1"]]);
    }

    #[test]
    fn reused_product_cache_has_an_exact_witness_limit() {
        for size in [64, 65] {
            let mut src = String::from("
                .decl lhs(a: number, k: number)
                .decl rhs(b: number, k: number)
                .decl wide(a: number, b: number)
                .decl lefts(a: number)
                .decl rights(b: number)
                .output lefts, rights
                wide(a,b) :- lhs(a,k), rhs(b,k).
                lefts(a) :- wide(a,_).
                rights(b) :- wide(_,b).
            ");
            for n in 0..size { src.push_str(&format!("lhs({n},0).\n")); }
            for n in 0..64 { src.push_str(&format!("rhs({n},0).\n")); }
            let mut p = parse(&src).unwrap();
            let plan = rewrite(&mut p, true, true);
            assert_eq!(plan.factors.len(), 1);
            let guard = p.nullary.iter().find(|r| r.starts_with("rr_pair_cache_")).unwrap();
            assert_eq!(p.decls[guard.as_str()].len(), 1);
            assert!(p.rules.iter().any(|r| r.head.rel == guard.as_str() && r.head.args.as_slice() == [Term::Int(1)]));
            let pairs = p.decls.keys().find(|r| r.starts_with("rr_pair_cache_") && r.ends_with("_pairs")).unwrap().to_string();
            p.outputs.push(pairs.clone());
            let rows = execute(&p, &format!("bounded-pairs-{size}"));
            assert_eq!(rows[&pairs].len(), if size == 64 { 4096 } else { 0 });
            let expected = |n| (0..n).map(|i: usize| vec![i.to_string()]).collect::<BTreeSet<_>>();
            assert_eq!(rows["lefts"].iter().cloned().collect::<BTreeSet<_>>(), expected(size));
            assert_eq!(rows["rights"].iter().cloned().collect::<BTreeSet<_>>(), expected(64));
        }
    }

    /// A reused product whose readers, or the rules over them, read an answer only a node's document gives (`value`)
    /// is factorized but not cached: a cache's readers are the cross-file part's, which has no documents to answer
    /// it from. Decided from the program, so a run is never prepared again to find it (decision 0477).
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
            let mut p = parse(&src(reader)).unwrap();
            let plan = rewrite(&mut p, true, true);
            assert_eq!(plan.factors.len(), 1, "{reader}: factorized");
            p.nullary.iter().any(|r| r.starts_with("rr_pair_cache_"))
        };
        assert!(cached("lefts(a) :- wide(a,_)."), "a plain reader keeps its cache");
        assert!(!cached("lefts(a) :- wide(a,_), value(a, _)."), "a reader of `value`");
        assert!(!cached("lefts(a) :- wide(a,_).\n over(a) :- lefts(a), value(a, _)."), "a rule over a reader reads `value`");
    }

    #[test]
    fn aggregate_recursive_and_open_negative_readers_keep_their_relation() {
        for reader in [
            "out(a) :- enabled(a), !wide(_,_).",
            "out(a) :- a = count : { wide(_,_) }.",
            "wide(a,b) :- wide(a,k), rhs(b,k). out(a) :- wide(a,_).",
        ] {
            let mut p = parse(&format!(
                "{SOURCE} .decl out(a: number)\n.output out\n{reader}"
            ))
            .unwrap();
            let before = p.rules.clone();
            let plans = rewrite(&mut p, true, true);
            assert!(plans.pairs.is_empty() && plans.factors.is_empty() && plans.shared.is_empty());
            assert_eq!(p.rules, before);
        }
    }
}
