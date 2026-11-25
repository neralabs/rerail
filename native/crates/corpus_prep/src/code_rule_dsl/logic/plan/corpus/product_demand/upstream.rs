//! Projected reader demand around a product. An optional restriction is installed only when every read supplies
//! the selected columns independently and the new demand sits outside the relation's dependency component.

use super::{Context, PREFIX as PRODUCT, Part, Program, connect, plain, rewritten, same, value, write_demand};
use crate::code_rule_dsl::logic::front::name::List;
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, Lit, RuleId, RuleRef, Slot, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const PREFIX: &str = crate::code_rule_dsl::logic::plan::body::READER_DEMAND;

fn is_demand(rel: &str) -> bool {
    rel.starts_with(PREFIX) || rel.starts_with(PRODUCT)
}

pub(super) fn eligible(program: &Program<'_>, rel: &str) -> bool {
    let f = &program.f;
    program.columns(rel).is_some()
        && !f.outputs.iter().any(|r| r == rel)
        && !f.inputs.iter().any(|r| r == rel)
        && !f.facts.contains(rel)
        && !f.lattice.contains_key(rel)
        && !f.natives.contains_key(rel)
        && !f.natives.values().any(|native| native.reads().contains(&rel))
        && !f.answers.contains_key(rel)
        && !f.answers.values().flatten().any(|answer| answer == rel)
        && !f.gates.iter().any(|(name, gate)| name == rel || gate.rel == rel || gate.src.as_deref() == Some(rel))
        && !f.asked_by.contains_key(rel)
        && !f.demand.iter().any(|demand| demand == "*")
        && !program.qualified(rel, "no_magic")
        && !is_demand(rel)
}

/// `atom`'s arguments at `columns`, where each is plain.
fn keys<'v>(atom: AtomRef<'v>, columns: &[usize]) -> Option<Vec<TermRef<'v>>> {
    let args: Vec<TermRef<'v>> = atom.args().collect();
    columns.iter().map(|&i| args.get(i).copied().filter(|t| plain(*t))).collect()
}

/// The ties a demand for `read` keeps (decision 0652): `read`'s location columns `locs` hold one document's nodes in
/// every row, so where the demand's positive reads (`parts`) bind two of their variables, the rows it asks for are
/// those whose two are of one document too, `@doc(x) = @doc(y)`. A superset still of what the reader joins, and the
/// split sees the demand on one document, as the reader is (UP008's `super_first(a, f)` asked for by `a` alone, with
/// `f`'s class path: every name met every class path of the corpus).
fn same_document_ties(rule: RuleRef<'_>, read: AtomRef<'_>, locs: &[usize], parts: &[Part]) -> Vec<(usize, usize)> {
    let mut bound = VarSet::default();
    for part in parts {
        if let Part::Lit(j) = *part
            && let Some(a) = rule.body().nth(j).and_then(|l| l.atom())
            && !a.neg()
            && !a.gated()
        {
            bound.extend(&atom_vars(a));
        }
    }
    let args: Vec<TermRef<'_>> = read.args().collect();
    let mut seen: Vec<(usize, Slot)> = Vec::new();
    for &c in locs {
        if let Some(TermRef::Var(s)) = args.get(c).copied()
            && bound.contains(s)
            && !seen.iter().any(|(_, t)| *t == s)
        {
            seen.push((c, s));
        }
    }
    seen.iter().skip(1).map(|&(c, _)| (seen[0].0, c)).collect()
}

fn vars_of(terms: &[TermRef<'_>]) -> VarSet {
    let mut out = VarSet::default();
    for t in terms {
        term_vars(*t, &mut out);
    }
    out
}

fn filter_term(v: &Version, term: TermRef<'_>) -> bool {
    match term {
        // `@doc(a) = @doc(b)` keeps a request on its reader's document (decision 0652): left out, the request joined
        // every file's rows by a value.
        TermRef::Call(f, args) => matches!(v.text(f), "prefix" | "suffix" | "glob" | "set" | "doc") && args.into_iter().all(|t| filter_term(v, t)),
        _ => plain(term),
    }
}

/// Each relation's readers in the program as it is: the heads of the rules that read it, the natives that read it, a
/// request's answers.
fn reader_graph<'p>(program: &'p Program<'_>) -> BTreeMap<&'p str, Vec<&'p str>> {
    let v = program.v();
    let mut readers = BTreeMap::<&str, Vec<&str>>::new();
    for rule in v.rules() {
        let head = v.rel_name(rule.head().rel());
        for literal in rule.body() {
            if let Some(a) = literal.atom() {
                readers.entry(v.rel_name(a.rel())).or_default().push(head);
            }
        }
    }
    for (name, native) in &program.f.natives {
        for read in native.reads() {
            readers.entry(read).or_default().push(name);
        }
    }
    for (request, answers) in &program.f.answers {
        readers.entry(request).or_default().extend(answers.iter().map(String::as_str));
    }
    readers
}

/// Follow relation reads backwards so a guard can drop any atom that would read the relation it bounds.
fn readers_of(readers: &BTreeMap<&str, Vec<&str>>, target: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut pending = vec![target];
    while let Some(next) = pending.pop() {
        if found.insert(next.to_owned()) {
            pending.extend(readers.get(next).into_iter().flatten().copied());
        }
    }
    found
}

/// The parts of `rule` a demand for `wanted` reads: its guard (`guard`: the guard's variables, the rule's first
/// literal) and its positive atoms of independent relations, connected, then its string and equality filters.
fn demand_body(
    v: &Version,
    rule: RuleRef<'_>,
    guard: Option<VarSet>,
    dependent: &BTreeSet<String>,
    products: &BTreeSet<String>,
    wanted: &VarSet,
) -> Option<Vec<Part>> {
    let mut groups: Vec<Context> = Vec::new();
    if let Some(vars) = guard {
        // The guard is a demand atom: it supplies the columns alone where it binds them all.
        if wanted.is_subset(&vars) {
            return Some(vec![Part::Guard]);
        }
        connect(&mut groups, Part::Guard, vars, false);
    }
    for (i, literal) in rule.body().enumerate() {
        if let Lit::Atom(a) = literal.view() {
            let rel = v.rel_name(a.rel());
            if !a.neg()
                && !a.gated()
                && !dependent.contains(rel)
                && !products.contains(rel)
                && a.args().all(|t| plain(t) || matches!(t, TermRef::Wild))
            {
                let vars = atom_vars(a);
                // A reader's own guard already supplies these columns. Avoid repeating its other joins.
                if is_demand(rel) && wanted.is_subset(&vars) {
                    return Some(vec![Part::Lit(i)]);
                }
                connect(&mut groups, Part::Lit(i), vars, false);
            }
        }
    }
    let group = groups.into_iter().find(|group| wanted.is_subset(&group.vars))?;
    // Keep already-bound positive string and equality tests. Dropping other filters still gives a superset.
    let filters = rule.body().enumerate().filter(|(_, literal)| {
        matches!(literal.view(), Lit::Cmp { l, r, .. } if filter_term(v, l) && filter_term(v, r)) && lit_vars(*literal).is_subset(&group.vars)
    });
    Some(group.body.into_iter().chain(filters.map(|(i, _)| Part::Lit(i))).collect())
}

/// A demand's rule `install` writes: of rule `rule`'s read at literal `read` (its key at the columns), its parts.
pub struct Proof {
    pub(super) rule: RuleId,
    pub(super) read: usize,
    own: bool,
    pub(super) parts: Vec<Part>,
    /// Pairs of the read's columns its demand ties to one document (`same_document_ties`): `@doc(x) = @doc(y)`.
    ties: Vec<(usize, usize)>,
}

fn install(program: &mut Program<'_>, rel: &str, cols: &[usize], products: &BTreeSet<String>, require_anchor: bool) -> Result<bool, String> {
    install_alone(program, rel, cols, &Ask { products, require_anchor, prefix: PREFIX, same_doc: None }, &|_, _| true)
}

/// What a restriction is installed under: the products its demand leaves out, whether a read must anchor it, and its
/// guard's prefix.
#[derive(Clone, Copy)]
struct Ask<'a> {
    products: &'a BTreeSet<String>,
    require_anchor: bool,
    prefix: &'a str,
    /// Each relation one document's in every row, with its location columns (`FileLocal::row_documents`): a demand
    /// keeps the tie its dropped read made between them (`same_document_ties`).
    same_doc: Option<&'a BTreeMap<String, Vec<usize>>>,
}

/// A restriction `install` writes, planned on the program as it is: `rel`'s guard `name` at its columns `cols`, the
/// producers it guards, the demand's rules (`proofs`) and constant rows, and the relations those rules read.
struct Plan {
    rel: String,
    name: String,
    cols: Vec<usize>,
    producers: BTreeSet<RuleId>,
    proofs: Vec<Proof>,
    rows: Vec<List<Value>>,
    columns: Vec<(String, String)>,
    reads: BTreeSet<String>,
}

/// `rel`'s restriction at `cols` as `ask` asks, if one can be installed: every read supplies the columns, and
/// `restricts` finds the demand's rules restrict the relation. `dependent` is what reads `rel` (`readers_of`).
fn plan(
    program: &Program<'_>,
    rel: &str,
    cols: &[usize],
    ask: &Ask<'_>,
    dependent: &BTreeSet<String>,
    restricts: &dyn Fn(&Version, &[Proof]) -> bool,
) -> Option<Plan> {
    let Ask { products, require_anchor, prefix, same_doc } = *ask;
    let name = format!("{prefix}{rel}");
    if program.columns(&name).is_some() {
        return None;
    }
    let v = program.v();
    let mut anchored = false;
    let mut proofs = Vec::new();
    let mut rows: Vec<List<Value>> = Vec::new();
    let mut producers = BTreeSet::new();
    let mut readers = 0;
    for rule in v.rules() {
        let own = program.head_rel(rule) == rel;
        let own_key = if own { keys(rule.head(), cols)? } else { Vec::new() };
        if own {
            producers.insert(rule.id());
        }
        for (i, literal) in rule.body().enumerate() {
            let Some(read) = literal.atom().filter(|a| v.rel_name(a.rel()) == rel) else {
                continue;
            };
            if read.gated() {
                return None;
            }
            // For aggregates, the prefix below must bind every chosen column outside the aggregate.
            // Inner columns stay out of the guard, so the aggregate still sees every row of each group.
            let key = keys(read, cols)?;
            let same_key = key.len() == own_key.len() && key.iter().zip(&own_key).all(|(a, b)| same(*a, *b));
            if own && same_key && matches!(literal.view(), Lit::Atom(_)) && !read.neg() {
                continue;
            }
            readers += 1;
            if readers > 48 {
                return None;
            }
            let wanted = vars_of(&key);
            if wanted.is_empty() {
                anchored = true;
                rows.push(key.iter().map(|t| value(v, *t)).collect());
                continue;
            }
            // For a recursive walk, start with the parent's requested key and derive the child's key.
            let guard = own.then(|| vars_of(&own_key));
            let parts = demand_body(v, rule, guard, dependent, products, &wanted)?;
            if !own {
                anchored |= parts.iter().any(|part| {
                    let Part::Lit(j) = *part else { return false };
                    match rule.body().nth(j).expect("a literal of the rule").view() {
                        Lit::Atom(a) => is_demand(v.rel_name(a.rel())) && wanted.is_subset(&atom_vars(a)),
                        Lit::Cmp { l: TermRef::Call(f, _), .. } => matches!(v.text(f), "prefix" | "glob" | "suffix"),
                        _ => false,
                    }
                });
            }
            let ties = same_doc.and_then(|m| m.get(rel)).map(|locs| same_document_ties(rule, read, locs, &parts)).unwrap_or_default();
            proofs.push(Proof { rule: rule.id(), read: i, own, parts, ties });
        }
    }
    if producers.is_empty() || readers == 0 || (require_anchor && !anchored) || !restricts(v, &proofs) {
        return None;
    }
    let columns: Vec<(String, String)> = {
        let all = program.columns(rel).unwrap_or_default();
        cols.iter().map(|&i| all[i].clone()).collect()
    };
    let mut reads = BTreeSet::new();
    for proof in &proofs {
        let r = v.rule(proof.rule).expect("a reader of the version");
        for part in &proof.parts {
            if let Part::Lit(j) = *part
                && let Some(a) = r.body().nth(j).and_then(|l| l.atom())
            {
                reads.insert(v.rel_name(a.rel()).to_owned());
            }
        }
    }
    Some(Plan { rel: rel.to_owned(), name, cols: cols.to_vec(), producers, proofs, rows, columns, reads })
}

/// One derivation of the program with each of `plans` installed: its producers guarded, its guard declared, its
/// demand's rules after the rules, in the plans' order.
fn derive_plans(program: &Program<'_>, plans: &[Plan]) -> Result<std::sync::Arc<Version>, String> {
    let by_producer: BTreeMap<RuleId, &Plan> = plans.iter().flat_map(|p| p.producers.iter().map(move |r| (*r, p))).collect();
    let mut qualifiers = program.f.qualifiers.clone();
    for plan in plans {
        qualifiers.entry(plan.name.clone()).or_default().extend(["no_inline".to_owned(), "no_magic".to_owned()]);
    }
    // These relations are guarded once the restrictions hold. Keep explicit demand on other relations.
    let demand: Vec<String> = program.f.demand.iter().filter(|d| !plans.iter().any(|p| &p.rel == *d)).cloned().collect();
    program.derive(
        &mut |first, v, r, b| {
            let Some(plan) = by_producer.get(&r.id()) else {
                return Ok(false);
            };
            let key = keys(r.head(), &plan.cols).expect("a producer's plain key");
            let guard: Vec<T<'_>> = key.iter().map(|t| t_of(r, *t)).collect();
            let mut rb = rewritten(first, v, r, b).atom(plan.name.as_str(), &guard);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            rb.finish().map(|_| true)
        },
        &mut |v, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            for plan in plans {
                let pairs: Vec<(&str, &str)> = plan.columns.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
                b.fields().declare(&plan.name, &pairs);
                if !plan.rows.is_empty() {
                    b.fields().set_facts(&plan.name, &plan.rows);
                }
                for proof in &plan.proofs {
                    let r = v.rule(proof.rule).expect("a reader of the version");
                    let read = r.body().nth(proof.read).and_then(|l| l.atom()).expect("the read");
                    let head: Vec<T<'_>> = keys(read, &plan.cols).expect("a plain key").iter().map(|t| t_of(r, *t)).collect();
                    let guard: Vec<T<'_>> = if proof.own {
                        keys(r.head(), &plan.cols).expect("a producer's plain key").iter().map(|t| t_of(r, *t)).collect()
                    } else {
                        Vec::new()
                    };
                    let ties: Vec<(T<'_>, T<'_>)> = proof.ties.iter().map(|&(x, y)| (t_of(r, read.args().nth(x).expect("a location column")), t_of(r, read.args().nth(y).expect("a location column")))).collect();
                    write_demand(v, r, b, &plan.name, &head, (&plan.name, &guard), &proof.parts, &ties)?;
                }
            }
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            b.fields().set_field(Field::Demand(&demand));
            Ok(())
        },
    )
}

/// Whether each plan's guard sits outside its relation's component in `made`.
fn apart(made: &Version, plans: &[Plan]) -> bool {
    Program::components(made).is_ok_and(|parts| {
        parts.iter().all(|part| plans.iter().all(|p| !(part.contains(p.rel.as_str()) && part.contains(p.name.as_str()))))
    })
}

/// `install` under another prefix, alone.
fn install_alone(
    program: &mut Program<'_>,
    rel: &str,
    cols: &[usize],
    ask: &Ask<'_>,
    restricts: &dyn Fn(&Version, &[Proof]) -> bool,
) -> Result<bool, String> {
    let dependent = readers_of(&reader_graph(program), rel);
    let Some(plan) = plan(program, rel, cols, ask, &dependent, restricts) else {
        return Ok(false);
    };
    let plans = [plan];
    let made = derive_plans(program, &plans)?;
    let apart = apart(&made, &plans);
    if apart {
        program.set(made);
    }
    Ok(apart)
}

/// `install` under another prefix for each of `requests`, in order: the deferred joins' pair relations
/// (`DEFERRED_PAIRS`, decision 0488), which of them installed. None is installed unless `restricts` finds the
/// demand's rules restrict the relation (a request that asks for every key costs a join of its own and restricts
/// nothing).
///
/// The requests go in rounds: a round's are planned on the program the round before left, made in one derivation,
/// and its components found once. A round ends before a request whose plan an earlier one of the round changes: what
/// reads its relation (`readers_of`) holds that one's relation (its producers would read the new guard) or a relation
/// its demand reads (the demand would read this one). A round one of whose guards is not apart is installed a request
/// at a time, as before: a restriction only adds edges, so a guard apart in the round's whole program is apart in any
/// part of it, and the rounds install what one request at a time would.
pub fn install_requests(
    program: &mut Program<'_>,
    requests: &[(String, &[usize])],
    prefix: &str,
    same_doc: &BTreeMap<String, Vec<usize>>,
    restricts: &dyn Fn(&str, &[usize], &Version, &[Proof]) -> bool,
) -> Result<Vec<bool>, String> {
    let none = BTreeSet::new();
    let ask = Ask { products: &none, require_anchor: false, prefix, same_doc: Some(same_doc) };
    let mut installed = vec![false; requests.len()];
    let mut next = 0;
    while next < requests.len() {
        let mut round: Vec<(usize, Plan)> = Vec::new();
        let mut end = next;
        {
            let graph = reader_graph(program);
            while let Some((rel, cols)) = requests.get(end) {
                let dependent = readers_of(&graph, rel);
                if round.iter().any(|(_, p)| dependent.contains(&p.rel) || !p.reads.is_disjoint(&dependent)) {
                    break;
                }
                if let Some(plan) = plan(program, rel, cols, &ask, &dependent,&|v, proofs| restricts(rel, cols, v, proofs)) {
                    round.push((end, plan));
                }
                end += 1;
            }
        }
        next = end;
        if round.is_empty() {
            continue;
        }
        let (at, plans): (Vec<usize>, Vec<Plan>) = round.into_iter().unzip();
        let made = derive_plans(program, &plans)?;
        if apart(&made, &plans) {
            program.set(made);
            for i in at {
                installed[i] = true;
            }
            continue;
        }
        for i in at {
            let (rel, cols) = &requests[i];
            installed[i] = install_alone(program, rel, cols, &ask, &|v, proofs| restricts(rel, cols, v, proofs))?;
        }
    }
    Ok(installed)
}

fn node_columns(program: &Program<'_>, rel: &str) -> Vec<usize> {
    program
        .columns(rel)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter(|(_, (_, ty))| matches!(ty.as_str(), "node" | "loc"))
        .map(|(i, _)| i)
        .collect()
}

pub fn restrict(program: &mut Program<'_>, rel: &str, products: &BTreeSet<String>, tuple: bool, require_anchor: bool) -> Result<bool, String> {
    if products.contains(rel) || !eligible(program, rel) {
        return Ok(false);
    }
    let nodes = node_columns(program, rel);
    if tuple && nodes.len() > 1 && install(program, rel, &nodes, products, require_anchor)? {
        return Ok(true);
    }
    for i in nodes {
        if install(program, rel, &[i], products, require_anchor)? {
            return Ok(true);
        }
    }
    // A symbol can also bound a read, for example a requested qualified name.
    let symbols: Vec<usize> = program
        .columns(rel)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter(|(_, (_, ty))| matches!(ty.as_str(), "sym" | "symbol"))
        .map(|(i, _)| i)
        .collect();
    for i in symbols {
        if install(program, rel, &[i], products, true)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Anchor the readers of a mandatory product before its own demand is derived.
pub(super) fn start(program: &mut Program<'_>, products: &BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    let (readers, peers) = {
        let v = program.v();
        let readers: BTreeSet<String> = v
            .rules()
            .filter(|r| r.body().filter_map(|l| l.atom()).any(|a| products.contains(v.rel_name(a.rel()))))
            .map(|r| program.head_rel(r).to_owned())
            .filter(|r| !products.contains(r))
            .collect();
        let mut peers = BTreeSet::new();
        for rule in v.rules() {
            let negated: Vec<AtomRef<'_>> = rule.body().filter_map(|l| l.atom()).filter(|atom| atom.neg()).collect();
            for read in negated.iter().filter(|atom| readers.contains(v.rel_name(atom.rel()))) {
                let Some(column) = node_columns(program, v.rel_name(read.rel())).first().copied() else {
                    continue;
                };
                let Some(TermRef::Var(anchor)) = read.args().nth(column) else {
                    continue;
                };
                // Other tests of the anchor or of a fixed name can share the demand. Tests of unrelated keys
                // keep their own plan, which can be cheaper than enumerating them from the candidate.
                peers.extend(
                    negated
                        .iter()
                        .filter(|atom| atom.args().any(|term| matches!(term, TermRef::Var(x) if x == anchor) || matches!(term, TermRef::Str(_) | TermRef::Branded(_))))
                        .map(|atom| v.rel_name(atom.rel()).to_owned()),
                );
            }
        }
        (readers, peers)
    };
    let mut narrowed = BTreeSet::new();
    let order: Vec<String> = readers.iter().chain(peers.difference(&readers)).take(24).cloned().collect();
    for rel in order {
        if restrict(program, &rel, products, readers.contains(&rel), false)? {
            narrowed.insert(rel);
        }
    }
    Ok(narrowed)
}

/// Carry keys to the inputs of guarded rules. Restriction stays optional when a helper also has an open reader.
pub(super) fn propagate(program: &mut Program<'_>, products: &BTreeSet<String>, narrowed: &mut BTreeSet<String>) -> Result<(), String> {
    let mut tried = narrowed.clone();
    let mut budget = 96;
    for _ in 0..4 {
        let candidates: BTreeSet<String> = {
            let v = program.v();
            v.rules()
                .filter(|r| is_demand(program.head_rel(*r)) || r.body().filter_map(|l| l.atom()).any(|a| is_demand(v.rel_name(a.rel()))))
                .flat_map(|r| r.body().filter_map(|l| l.atom()))
                .map(|a| v.rel_name(a.rel()).to_owned())
                .filter(|r| !tried.contains(r) && eligible(program, r))
                .collect()
        };
        let before = narrowed.len();
        for rel in candidates.into_iter().take(budget) {
            budget -= 1;
            tried.insert(rel.clone());
            if restrict(program, &rel, products, false, true)? {
                narrowed.insert(rel);
            }
        }
        if budget == 0 || before == narrowed.len() {
            break;
        }
    }
    Ok(())
}

/// An argument of a witness atom `restore_order_witnesses` appends to a rule: argument `k` of the rule's literal `j`
/// (`Rule(j, k)`), of the helper's (`Helper(helper, j, k)`), or `_`.
enum Arg {
    Rule(usize, usize),
    Helper(RuleId, usize, usize),
    Wild,
}

impl Arg {
    fn term<'v>(&self, v: &'v Version, r: RuleRef<'v>) -> T<'v> {
        let arg = |r: RuleRef<'v>, j: usize, k: usize| {
            let a = r.body().nth(j).and_then(|l| l.atom()).expect("an atom of the rule");
            t_of(r, a.args().nth(k).expect("an argument of the atom"))
        };
        match *self {
            Arg::Rule(j, k) => arg(r, j, k),
            Arg::Helper(h, j, k) => arg(v.rule(h).expect("the helper"), j, k),
            Arg::Wild => T::Wild,
        }
    }
}

/// Restate the witness of a node minimum or maximum after reversing its order key. Its original atom tells
/// placement which document supplied the node. An empty group has no node_order row. Node-order keys in the tape
/// binding are injective, so a nonempty group's returned node is a witness of the original atom.
pub fn restore_order_witnesses(program: &mut Program<'_>, narrowed: &BTreeSet<String>) -> Result<usize, String> {
    let f = &program.f;
    let v = program.v();
    if !f.inputs.iter().any(|r| r == "node_order")
        || f.facts.contains("node_order")
        || f.natives.contains_key("node_order")
        || v.rules().any(|r| program.head_rel(r) == "node_order")
    {
        return Ok(0);
    }
    // Each edit: the rule, the witness's relation, and its arguments.
    let mut edits: BTreeMap<RuleId, Vec<(String, Vec<Arg>)>> = BTreeMap::new();
    let mut count = 0;
    for rule in v.rules() {
        if !narrowed.contains(program.head_rel(rule)) {
            continue;
        }
        let body: Vec<_> = rule.body().collect();
        for (agg_at, literal) in body.iter().enumerate() {
            let Lit::Agg { var, op: AggRef::Min | AggRef::Max, over: Some(over), atom: aggregate } = literal.view() else {
                continue;
            };
            let agg_rel = v.rel_name(aggregate.rel());
            // The synthesized helper projects the ordered node to its preorder key.
            if !agg_rel.starts_with("rr_ordered")
                || f.inputs.iter().any(|r| r == agg_rel)
                || f.facts.contains(agg_rel)
                || f.natives.contains_key(agg_rel)
                || aggregate.neg()
                || aggregate.gated()
            {
                continue;
            }
            // The rule's literal and column of the order's node: `node_order(result, var)`.
            let Some(result) = body.iter().enumerate().find_map(|(j, l)| match l.view() {
                Lit::Atom(a) if !a.neg() && v.rel_name(a.rel()) == "node_order" && matches!(a.args().nth(1), Some(TermRef::Var(x)) if x == var) => {
                    matches!(a.args().next(), Some(TermRef::Var(_))).then_some(j)
                }
                _ => None,
            }) else {
                continue;
            };
            let mut defs = v.rules().filter(|r| program.head_rel(*r) == agg_rel);
            let Some(helper) = defs.next() else { continue };
            if defs.next().is_some() {
                continue;
            }
            let mut hbody = helper.body().enumerate().filter(|(_, literal)| match literal.view() {
                Lit::Atom(a) => !is_demand(v.rel_name(a.rel())),
                Lit::Cmp { .. } => false,
                _ => true,
            });
            let (Some((source_at, source)), Some((_, order)), None) = (hbody.next(), hbody.next(), hbody.next()) else {
                continue;
            };
            let (Lit::Atom(source), Lit::Atom(order)) = (source.view(), order.view()) else { continue };
            let order_args: Vec<TermRef<'_>> = order.args().collect();
            let [TermRef::Var(node), TermRef::Var(order_key)] = order_args[..] else {
                continue;
            };
            if source.neg() || source.gated() || order.neg() || order.gated() || v.rel_name(order.rel()) != "node_order" || node == order_key {
                continue;
            }
            let outer: VarSet = {
                let mut out = VarSet::default();
                for l in &body {
                    if let Lit::Atom(a) = l.view()
                        && !a.neg()
                    {
                        out.extend(&atom_vars(a));
                    }
                }
                out
            };
            let head_args: Vec<TermRef<'_>> = helper.head().args().collect();
            let agg_args: Vec<TermRef<'_>> = aggregate.args().collect();
            let mut matched = false;
            let arguments: Option<Vec<Arg>> = source
                .args()
                .enumerate()
                .map(|(k, t)| match t {
                    TermRef::Var(x) if x == node => {
                        matched = true;
                        Some(Arg::Rule(result, 0))
                    }
                    TermRef::Var(x) => {
                        let pos = head_args.iter().position(|h| matches!(h, TermRef::Var(y) if *y == x))?;
                        match *agg_args.get(pos)? {
                            TermRef::Var(y) if !outer.contains(y) => Some(Arg::Wild),
                            TermRef::Wild => Some(Arg::Wild),
                            term if plain(term) => Some(Arg::Rule(agg_at, pos)),
                            _ => None,
                        }
                    }
                    TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => Some(Arg::Helper(helper.id(), source_at, k)),
                    TermRef::Wild => Some(Arg::Wild),
                    _ => None,
                })
                .collect();
            let Some(pos) = head_args.iter().position(|t| matches!(t, TermRef::Var(y) if *y == order_key)) else {
                continue;
            };
            if matches!(agg_args.get(pos), Some(TermRef::Var(y)) if *y == over)
                && matched
                && let Some(arguments) = arguments
            {
                edits.entry(rule.id()).or_default().push((v.rel_name(source.rel()).to_owned(), arguments));
                count += 1;
            }
        }
    }
    if edits.is_empty() {
        return Ok(0);
    }
    let made = program.derive(
        &mut |first, v, r, b| {
            let Some(atoms) = edits.get(&r.id()) else { return Ok(false) };
            let body: Vec<_> = r.body().collect();
            let mut rb = rewritten(first, v, r, b);
            for l in &body {
                rb = rb.lit(v, r, *l);
            }
            for (rel, arguments) in atoms {
                let args: Vec<T<'_>> = arguments.iter().map(|a| a.term(v, r)).collect();
                rb = rb.atom(rel.as_str(), &args);
            }
            rb.finish().map(|_| true)
        },
        &mut |_, _| Ok(()),
    )?;
    program.set(made);
    Ok(count)
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
