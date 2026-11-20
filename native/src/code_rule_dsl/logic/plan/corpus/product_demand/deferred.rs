//! Joins deferred to their readers' requests (decision 0477). A binary positive join over a key its head projects
//! away (`wide(a, b) :- lhs(a, k), rhs(b, k)`) is driven by what its readers request: the pairs, or one endpoint
//! (`DEFERRED_PAIRS`, a demand relation as `upstream`'s), or else kept as its two factors, which meet in each reader
//! (`file_local::factorize_intermediates`), with a pair cache where the factors are small. A frequently requested
//! endpoint shares its side's witnesses across the pairs requested of it (`share`).
//!
//! On the version, as `product_demand`'s other steps (decision 0276): each rewrite is a derivation of the program
//! (`Program`), and the pass publishes the last as an edit of its input.

use super::{DEFERRED_PAIRS, PREFIX, Program, plain, rewritten, same, upstream};
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::plan::corpus::preparation::Deferral;
use crate::code_rule_dsl::logic::profile::author_joins::{Access, Join, TOPIC};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit, RuleRef, T, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// An endpoint requested by this many pairs shares its side's witnesses (`share`).
const SHARE_AFTER: i64 = 8;

/// What the pass did: the relations driven by their requested pairs, by one requested endpoint (`rel column c`), the
/// lines of those kept as factors, the endpoints shared, whether a pair cache was made; and the report of each join
/// (`profile::author_joins`).
#[derive(Default)]
pub(in crate::code_rule_dsl::logic::plan::corpus) struct Plans {
    pub pairs: Vec<String>,
    pub one_sided: Vec<String>,
    pub factors: Vec<String>,
    pub shared: Vec<String>,
    pub cached: bool,
    pub author: Vec<Join>,
}

/// The place in `rule`'s body of its requests guard: a positive atom of a demand relation of its head (`PREFIX`,
/// `upstream::PREFIX`, `DEFERRED_PAIRS`) over the head's own terms.
fn guard(v: &Version, rule: RuleRef<'_>) -> Option<usize> {
    let head = v.rel_name(rule.head().rel());
    let args: Vec<TermRef<'_>> = rule.head().args().collect();
    rule.body().position(|l| match l.view() {
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

/// `rule` split into the two parts that meet at a key its head projects away (`keeps::factors_without`, its requests
/// guard left out), each part reading the key: its key's name and the parts by place. `None` for a rule whose head
/// is not two variables, whose body holds anything but positive atoms of plain terms and comparisons other than `=`,
/// or that splits otherwise.
fn split(v: &Version, rule: RuleRef<'_>) -> Option<(String, Vec<Vec<usize>>)> {
    let head: Vec<TermRef<'_>> = rule.head().args().collect();
    let [TermRef::Var(a), TermRef::Var(b)] = head.as_slice() else {
        return None;
    };
    let fits = rule.body().all(|l| match l.view() {
        Lit::Atom(x) => !x.neg() && !x.gated() && x.args().all(|t| plain(t) || matches!(t, TermRef::Wild)),
        Lit::Cmp { l, op, r } => op != CmpOp::Eq && plain(l) && plain(r),
        _ => false,
    });
    if a == b || !fits {
        return None;
    }
    let (key, parts) = crate::code_rule_dsl::logic::plan::keeps::factors_without(v, rule, guard(v, rule))?;
    let lits: Vec<_> = rule.body().collect();
    let keyed = |part: &Vec<usize>| part.iter().any(|&i| matches!(lits[i].view(), Lit::Atom(x) if atom_vars(x).iter().any(|s| v.text(rule.var(s)) == key)));
    (parts.len() == 2 && parts.iter().all(keyed)).then_some((key, parts))
}

/// Whether `rule`'s read at `j` has its columns `cols` bound by the rest of the body: each a plain term, and their
/// variables all read by one positive atom of another relation (or none).
fn reader_binds(v: &Version, rule: RuleRef<'_>, j: usize, cols: &[usize]) -> bool {
    let lits: Vec<_> = rule.body().collect();
    let Some(read) = lits[j].atom() else { return false };
    let args: Vec<TermRef<'_>> = read.args().collect();
    let mut bound = VarSet::new();
    for &c in cols {
        let Some(&t) = args.get(c) else { return false };
        if !plain(t) {
            return false;
        }
        term_vars(t, &mut bound);
    }
    let rel = v.rel_name(read.rel());
    bound.is_empty() || lits.iter().any(|l| matches!(l.view(), Lit::Atom(a) if v.rel_name(a.rel()) != rel && !a.neg() && !a.gated() && bound.is_subset(&atom_vars(a))))
}

/// The pass over `p` (its input `input`): which restrictions the readers give each binary join (the module comment),
/// and the rewrites made. `adaptive`: endpoints shared and reduced factors for broad readers; `cache_pairs`: pair
/// caches admitted besides.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn rewrite(p: &mut Program<'_>, input: &Arc<Version>, adaptive: bool, cache_pairs: bool) -> Result<Plans, String> {
    let mut plans = Plans::default();
    // The join relations by the readers they could be driven by, read off the program before any rewrite (a guard
    // inserted moves the places).
    let (pairs, mut guarded, mut broad, mut open_readers, recursive) = {
        let v = p.v();
        let rules: Vec<RuleRef<'_>> = v.rules().collect();
        let mut definitions = BTreeMap::<&str, usize>::new();
        let mut readers = BTreeMap::<&str, Vec<(usize, usize)>>::new();
        for (i, r) in rules.iter().enumerate() {
            *definitions.entry(p.head_rel(*r)).or_default() += 1;
            for (j, l) in r.body().enumerate() {
                if let Some(a) = l.atom() {
                    readers.entry(v.rel_name(a.rel())).or_default().push((i, j));
                }
            }
        }
        // A relation the connected demand already restricted is reported with its guard, whatever this pass finds.
        let mut explained = BTreeSet::new();
        for rule in &rules {
            if let Some(at) = guard(v, *rule)
                && explained.insert(p.head_rel(*rule))
            {
                let request = rule.body().nth(at).and_then(|l| l.atom()).map(|a| v.rel_name(a.rel()).to_owned()).expect("the guard");
                let columns = p.columns(p.head_rel(*rule)).unwrap_or_default().into_iter().map(|(c, _)| c).collect();
                plans.author.push(Join { relation: p.head_rel(*rule).to_owned(), access: Access::Requests { table: request, columns }, readers: Vec::new(), placement: None });
            }
        }
        let tapes = v.tape_columns()?;
        let shapes = v.shape_list()?;
        let mut candidates = BTreeSet::new();
        let mut guarded: Vec<(String, usize)> = Vec::new();
        let mut nth: BTreeMap<&str, usize> = BTreeMap::new();
        for rule in &rules {
            let rel = p.head_rel(*rule);
            let n = {
                let k = nth.entry(rel).or_default();
                *k += 1;
                *k - 1
            };
            if rule.head().args().count() != 2
                || rule.body().count() < 2
                || !readers.contains_key(rel)
                || !upstream::eligible(p, rel)
                || p.f.qualifiers.get(rel).is_some_and(|q| !q.is_empty())
                || tapes.contains_key(rel)
                || p.f.demand.iter().any(|r| r == rel)
                || shapes.iter().any(|s| s.head == rel || s.lit == rel)
                || p.f.asked_by.values().flatten().any(|refs| refs.contains(rel))
            {
                continue;
            }
            if split(v, *rule).is_some() {
                if guard(v, *rule).is_some() {
                    if adaptive {
                        guarded.push((rel.to_owned(), n));
                    }
                } else if definitions[rel] == 1 {
                    candidates.insert(rel);
                }
            }
        }
        if candidates.is_empty() && guarded.is_empty() {
            return Ok(plans);
        }
        let Ok(sccs) = Program::components(v) else {
            return Ok(plans);
        };
        let recursive: BTreeSet<String> = sccs
            .into_iter()
            .filter(|c| c.len() > 1)
            .flatten()
            .chain(rules.iter().filter(|r| r.body().any(|l| l.atom().is_some_and(|a| a.rel() == r.head().rel()))).map(|r| p.head_rel(*r).to_owned()))
            .collect();
        let mut pairs: Vec<(String, &'static [usize], bool)> = Vec::new();
        let mut broad = BTreeSet::new();
        let mut open_readers: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for rel in candidates {
            if recursive.contains(rel) {
                continue;
            }
            let uses = &readers[rel];
            let columns = p.columns(rel).unwrap_or_default();
            let mut descriptions = BTreeSet::new();
            for &(i, j) in uses {
                let open: Vec<&str> = (0..2).filter(|c| !reader_binds(v, rules[i], j, &[*c])).map(|c| columns[c].0.as_str()).collect();
                if !open.is_empty() {
                    descriptions.insert((p.head_rel(rules[i]).to_owned(), format!("supplies no lookup value for {}", open.join(", "))));
                }
            }
            open_readers.insert(rel.to_owned(), descriptions.into_iter().collect());
            let lit = |i: usize, j: usize| rules[i].body().nth(j).expect("a read");
            if uses.iter().any(|&(i, j)| !matches!(lit(i, j).view(), Lit::Atom(a) if !a.gated())) {
                continue;
            }
            let positive = uses.iter().all(|&(i, j)| lit(i, j).atom().is_some_and(|a| !a.neg()));
            const MASKS: [&[usize]; 3] = [&[0, 1], &[0], &[1]];
            if let Some(cols) = MASKS.into_iter().find(|cols| uses.iter().all(|&(i, j)| reader_binds(v, rules[i], j, cols))) {
                pairs.push((rel.to_owned(), cols, positive));
            } else if positive {
                broad.insert(rel.to_owned());
            }
        }
        (pairs, guarded, broad, open_readers, recursive)
    };
    // The requests installed, once every read is classified.
    let mut no_inline = Vec::new();
    for (rel, cols, positive) in pairs {
        if !upstream::install_named(p, &rel, cols, &BTreeSet::new(), false, DEFERRED_PAIRS)? {
            continue;
        }
        no_inline.push(rel.clone());
        let columns = p.columns(&rel).unwrap_or_default();
        plans.author.push(Join {
            relation: rel.clone(),
            access: Access::Requests { table: format!("{DEFERRED_PAIRS}{rel}"), columns: cols.iter().map(|&c| columns[c].0.clone()).collect() },
            readers: open_readers.remove(&rel).unwrap_or_default(),
            placement: None,
        });
        if cols.len() == 1 {
            plans.one_sided.push(format!("{rel} column {}", cols[0]));
            // A request for one endpoint can still match the whole opposite factor: its guard stays on that factor,
            // and the positive readers read the join's factors directly.
            if adaptive && positive {
                broad.insert(rel);
            }
            continue;
        }
        plans.pairs.push(rel.clone());
        if adaptive {
            let v = p.v();
            if v.rules().find(|r| p.head_rel(*r) == rel).is_some_and(|r| split(v, r).is_some()) {
                guarded.push((rel, 0));
            }
        }
    }
    for (rel, n) in guarded {
        if !recursive.contains(&rel)
            && let Some(label) = share(p, &rel, n)?
        {
            plans.shared.push(label);
        }
    }
    let current = p.made.clone().unwrap_or_else(|| Arc::clone(input));
    if let Some((made, lines, choices)) = crate::code_rule_dsl::typing::file_local::factorize_intermediates(&current, p.first(), &broad, adaptive && cache_pairs)? {
        p.set(made);
        plans.factors = lines;
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
    // A relation its requests drive is read whole by each reader, not inlined into them; one handed on as its factors
    // is gone.
    no_inline.retain(|rel| p.columns(rel).is_some());
    if !no_inline.is_empty() {
        let mut qualifiers = p.f.qualifiers.clone();
        for rel in &no_inline {
            qualifiers.entry(rel.clone()).or_default().insert("no_inline".to_owned());
        }
        let made = p.derive(&mut |_, _, _, _| Ok(false), &mut |_, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            Ok(())
        })?;
        p.set(made);
    }
    Ok(plans)
}

/// The `n`th rule of `rel`, driven by its requests, sharing an endpoint's witnesses (`rel column c`): where a
/// requested endpoint is asked for by at least `SHARE_AFTER` pairs (`hot`), its larger part's rows for it are made
/// once (`factor(endpoint, witness)`), and each requested pair reads them with the other part; the rule itself
/// answers the endpoints that are not hot. None where the larger part is a single literal (an indexed input answers
/// the probe in place) or the rule does not split so.
fn share(p: &mut Program<'_>, rel: &str, n: usize) -> Result<Option<String>, String> {
    use crate::code_rule_dsl::logic::front::ast::AggOp;
    let v = p.v();
    let Some(rule) = v.rules().filter(|r| p.head_rel(*r) == rel).nth(n) else { return Ok(None) };
    let Some((key, parts)) = split(v, rule) else { return Ok(None) };
    let Some((side, part)) = parts.iter().enumerate().max_by_key(|(_, part)| part.len()) else { return Ok(None) };
    if part.len() < 2 {
        return Ok(None);
    }
    let lits: Vec<_> = rule.body().collect();
    let name = |s| v.text(rule.var(s)).to_owned();
    let vars: BTreeSet<String> = part.iter().flat_map(|&i| lit_vars(lits[i]).iter().map(name).collect::<Vec<_>>()).collect();
    let head: Vec<TermRef<'_>> = rule.head().args().collect();
    let Some(col) = head.iter().position(|t| matches!(t, TermRef::Var(s) if vars.contains(&name(*s)))) else { return Ok(None) };
    let TermRef::Var(at) = head[col] else { return Ok(None) };
    let endpoint = name(at);
    let key_ty = part.iter().find_map(|&i| {
        let a = lits[i].atom()?;
        let c = a.args().position(|t| matches!(t, TermRef::Var(s) if name(s) == key))?;
        p.columns(v.rel_name(a.rel()))?.get(c).map(|(_, ty)| ty.clone())
    });
    let (Some(key_ty), Some(endpoint_ty), Some(request_at)) = (key_ty, p.columns(rel).and_then(|c| c.get(col).map(|(_, t)| t.clone())), guard(v, rule)) else {
        return Ok(None);
    };
    let request = lits[request_at].atom().map(|a| v.rel_name(a.rel()).to_owned()).expect("the guard");
    let base = crate::code_rule_dsl::typing::file_local::helper_name("rr_deferred_", rel, &v.show_rule(rule), |n| ["hot", "factor"].iter().any(|s| p.columns(&format!("{n}_{s}")).is_some()), &mut Default::default());
    let (hot, factor) = (format!("{base}_hot"), format!("{base}_factor"));
    let mut count = "rr_fanout".to_owned();
    while count == endpoint {
        count.push('_');
    }
    let label = format!("{rel} column {col} ({} literals)", part.len());
    let (part, other, id) = (part.clone(), parts[1 - side].clone(), rule.id());
    let mut qualifiers = p.f.qualifiers.clone();
    for r in [&hot, &factor] {
        qualifiers.entry(r.clone()).or_default().insert("no_inline".to_owned());
    }
    let made = p.derive(
        &mut |first, v, r, b| {
            if r.id() != id {
                return Ok(false);
            }
            // The rule answers the cold endpoints.
            let mut rb = rewritten(first, v, r, b);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            rb.not(hot.as_str(), &[T::Var(&endpoint)]).finish().map(|_| true)
        },
        &mut |v, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            let r = v.rule(id).expect("the shared rule");
            let lits: Vec<_> = r.body().collect();
            b.fields().declare(&hot, &[("endpoint", endpoint_ty.as_str())]);
            b.fields().declare(&factor, &[("endpoint", endpoint_ty.as_str()), ("witness", key_ty.as_str())]);
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            let mut group = [T::Wild, T::Wild];
            group[col] = T::Var(&endpoint);
            // hot(e) :- request(…e…), n = count : { request(…e…) }, n >= SHARE_AFTER.
            b.rule()
                .head(hot.as_str(), &[T::Var(&endpoint)])
                .atom(request.as_str(), &group)
                .agg(T::Var(&count), &AggOp::Count, None, request.as_str(), &group)
                .cmp(T::Var(&count), CmpOp::Ge, T::Int(SHARE_AFTER))
                .finish()?;
            // factor(e, k) :- hot(e), the larger part.
            let mut rb = b.rule().head(factor.as_str(), &[T::Var(&endpoint), T::Var(&key)]).atom(hot.as_str(), &[T::Var(&endpoint)]);
            for &i in &part {
                rb = rb.lit(v, r, lits[i]);
            }
            rb.finish()?;
            // The hot endpoints' pairs: the request, the factor, the other part.
            let mut rb = b.rule().head_from(v, r, r.head()).lit(v, r, lits[request_at]).atom(factor.as_str(), &[T::Var(&endpoint), T::Var(&key)]);
            for &i in &other {
                rb = rb.lit(v, r, lits[i]);
            }
            rb.finish().map(drop)
        },
    )?;
    p.set(made);
    Ok(Some(label))
}

/// The deferred joins pass (decision 0477) under a deferral (`preparation::Deferral`): its decisions, and the report
/// of each join (`profile::author_joins::TOPIC`, which `preparation::front` reads back for the pair caches).
pub(in crate::code_rule_dsl::logic::plan::corpus) struct DeferJoins {
    deferral: Deferral,
}

impl DeferJoins {
    pub(in crate::code_rule_dsl::logic::plan::corpus) fn new(deferral: &Deferral) -> DeferJoins {
        DeferJoins { deferral: deferral.clone() }
    }
}

impl ProgramPass for DeferJoins {
    const NAME: &'static str = "joins deferred to their readers' requests";

    fn settings(&self) -> Vec<u8> {
        self.deferral.settings()
    }

    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let version: &Arc<Version> = cx.v();
        let mut program = Program::of(version);
        let plans = rewrite(&mut program, version, self.deferral.adaptive, self.deferral.cache_pairs)?;
        cx.decide("deferred joins", || {
            format!("pair demand: {:?}; endpoint demand: {:?}; reduced factors: {:?}; fanout sharing: {:?}", plans.pairs, plans.one_sided, plans.factors, plans.shared)
        });
        for mut join in plans.author {
            if let Access::Factors { cache: None } = &join.access {
                join.placement = self.deferral.placement.get(&join.relation).cloned();
            }
            cx.decide(TOPIC, || serde_json::to_string(&join).expect("a join's report"));
        }
        if program.first() {
            return Ok(());
        }
        super::publish(version, program.v(), w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::plan::{corpus, output::Outputs};
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
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

    /// `v` with the pass run on it: the version it leaves, and what it did.
    fn rewritten(v: &Arc<Version>) -> (Arc<Version>, Plans) {
        let mut p = Program::of(v);
        let plans = rewrite(&mut p, v, true, true).unwrap();
        (p.made.clone().unwrap_or_else(|| Arc::clone(v)), plans)
    }

    /// `v` with `rels` outputs too.
    fn with_outputs(v: &Arc<Version>, rels: &[&str]) -> Arc<Version> {
        use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
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

    fn declared(v: &Version, suffix: &str) -> String {
        F::declared_names(v).into_iter().find(|r| r.ends_with(suffix)).unwrap()
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
        let (made, plans) = rewritten(&v);
        assert_eq!(plans.pairs, ["wide"]);
        assert_eq!(plans.shared.len(), 1);
        let (hot, factor) = (declared(&made, "_hot"), declared(&made, "_factor"));
        let made = with_outputs(&made, &[&hot, &factor]);
        let mut actual = execute(&made, "deferred-adaptive");
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
        let (made, plans) = rewritten(&v);
        assert_eq!(plans.factors.len(), 1);
        assert!(F::columns_of(&*made, "wide").is_none());
        assert_eq!(execute(&made, "deferred-broad"), expected);
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
        let (made, plans) = rewritten(&v);
        assert_eq!(plans.one_sided, ["wide column 0"]);
        assert_eq!(plans.factors.len(), 1);
        assert!(F::columns_of(&*made, "wide").is_none());
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
            let (made, plan) = rewritten(&parse(&src));
            assert_eq!(plan.factors.len(), 1);
            let guard = made.nullary_names().unwrap().into_iter().find(|r| r.starts_with("rr_pair_cache_")).unwrap();
            assert_eq!(F::columns_of(&*made, &guard).unwrap().len(), 1);
            assert!(made.rules().any(|r| made.rel_name(r.head().rel()) == guard && matches!(r.head().args().collect::<Vec<_>>().as_slice(), [TermRef::Int(1)])));
            let pairs = F::declared_names(&*made).into_iter().find(|r| r.starts_with("rr_pair_cache_") && r.ends_with("_pairs")).unwrap();
            let made = with_outputs(&made, &[&pairs]);
            let rows = execute(&made, &format!("bounded-pairs-{size}"));
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
            let mut p = Program::of(&v);
            let plans = rewrite(&mut p, &v, true, true).unwrap();
            assert!(plans.pairs.is_empty() && plans.factors.is_empty() && plans.shared.is_empty());
            assert!(p.first(), "the rules as they were");
        }
    }
}
