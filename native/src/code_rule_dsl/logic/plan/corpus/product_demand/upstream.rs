//! Projected reader demand around a product. An optional restriction is installed only when every read supplies
//! the selected columns independently and the new demand sits outside the relation's dependency component.

use super::{Context, PREFIX as PRODUCT, Part, Program, connect, plain, rewritten, same, value, write_demand};
use crate::code_rule_dsl::logic::front::name::List;
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::{AggRef, AtomRef, Lit, RuleId, RuleRef, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const PREFIX: &str = "rr_reader_demand_";

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

fn vars_of(terms: &[TermRef<'_>]) -> VarSet {
    let mut out = VarSet::default();
    for t in terms {
        term_vars(*t, &mut out);
    }
    out
}

fn filter_term(v: &Version, term: TermRef<'_>) -> bool {
    match term {
        TermRef::Call(f, args) => matches!(v.text(f), "prefix" | "suffix" | "glob" | "set") && args.into_iter().all(|t| filter_term(v, t)),
        _ => plain(term),
    }
}

/// Follow relation reads backwards so a guard can drop any atom that would read the relation it bounds.
fn readers_of(program: &Program<'_>, target: &str) -> BTreeSet<String> {
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
pub(super) struct Proof {
    pub(super) rule: RuleId,
    pub(super) read: usize,
    own: bool,
    pub(super) parts: Vec<Part>,
}

fn install(program: &mut Program<'_>, rel: &str, cols: &[usize], products: &BTreeSet<String>, require_anchor: bool) -> Result<bool, String> {
    install_named(program, rel, cols, products, require_anchor, PREFIX, &|_, _| true)
}

/// `install` under another prefix: the deferred joins' pair relations (`DEFERRED_PAIRS`, decision 0488). None is
/// installed unless `restricts` finds the demand's rules restrict the relation (a request that asks for every key
/// costs a join of its own and restricts nothing).
pub(super) fn install_named(
    program: &mut Program<'_>,
    rel: &str,
    cols: &[usize],
    products: &BTreeSet<String>,
    require_anchor: bool,
    prefix: &str,
    restricts: &dyn Fn(&Version, &[Proof]) -> bool,
) -> Result<bool, String> {
    let name = format!("{prefix}{rel}");
    if program.columns(&name).is_some() {
        return Ok(false);
    }
    let dependent = readers_of(program, rel);
    let v = program.v();
    let mut anchored = false;
    let mut proofs = Vec::new();
    let mut rows: Vec<List<Value>> = Vec::new();
    let mut producers = BTreeSet::new();
    let mut readers = 0;
    for rule in v.rules() {
        let own = program.head_rel(rule) == rel;
        let own_key = if own {
            match keys(rule.head(), cols) {
                Some(key) => key,
                None => return Ok(false),
            }
        } else {
            Vec::new()
        };
        if own {
            producers.insert(rule.id());
        }
        for (i, literal) in rule.body().enumerate() {
            let Some(read) = literal.atom().filter(|a| v.rel_name(a.rel()) == rel) else {
                continue;
            };
            if read.gated() {
                return Ok(false);
            }
            // For aggregates, the prefix below must bind every chosen column outside the aggregate.
            // Inner columns stay out of the guard, so the aggregate still sees every row of each group.
            let Some(key) = keys(read, cols) else { return Ok(false) };
            let same_key = key.len() == own_key.len() && key.iter().zip(&own_key).all(|(a, b)| same(*a, *b));
            if own && same_key && matches!(literal.view(), Lit::Atom(_)) && !read.neg() {
                continue;
            }
            readers += 1;
            if readers > 48 {
                return Ok(false);
            }
            let wanted = vars_of(&key);
            if wanted.is_empty() {
                anchored = true;
                rows.push(key.iter().map(|t| value(v, *t)).collect());
                continue;
            }
            // For a recursive walk, start with the parent's requested key and derive the child's key.
            let guard = own.then(|| vars_of(&own_key));
            let Some(parts) = demand_body(v, rule, guard, &dependent, products, &wanted) else { return Ok(false) };
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
            proofs.push(Proof { rule: rule.id(), read: i, own, parts });
        }
    }
    if producers.is_empty() || readers == 0 || (require_anchor && !anchored) || !restricts(v, &proofs) {
        return Ok(false);
    }
    let columns: Vec<(String, String)> = {
        let all = program.columns(rel).unwrap_or_default();
        cols.iter().map(|&i| all[i].clone()).collect()
    };
    let mut qualifiers = program.f.qualifiers.clone();
    qualifiers.entry(name.clone()).or_default().extend(["no_inline".to_owned(), "no_magic".to_owned()]);
    // This relation is guarded once the restriction holds. Keep explicit demand on other relations.
    let demand: Vec<String> = program.f.demand.iter().filter(|d| *d != rel).cloned().collect();
    let made = program.derive(
        &mut |first, v, r, b| {
            if !producers.contains(&r.id()) {
                return Ok(false);
            }
            let key = keys(r.head(), cols).expect("a producer's plain key");
            let guard: Vec<T<'_>> = key.iter().map(|t| t_of(r, *t)).collect();
            let mut rb = rewritten(first, v, r, b).atom(name.as_str(), &guard);
            for l in r.body() {
                rb = rb.lit(v, r, l);
            }
            rb.finish().map(|_| true)
        },
        &mut |v, b| {
            use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
            let pairs: Vec<(&str, &str)> = columns.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            b.fields().declare(&name, &pairs);
            if !rows.is_empty() {
                b.fields().set_facts(&name, &rows);
            }
            for proof in &proofs {
                let r = v.rule(proof.rule).expect("a reader of the version");
                let read = r.body().nth(proof.read).and_then(|l| l.atom()).expect("the read");
                let head: Vec<T<'_>> = keys(read, cols).expect("a plain key").iter().map(|t| t_of(r, *t)).collect();
                let guard: Vec<T<'_>> = if proof.own {
                    keys(r.head(), cols).expect("a producer's plain key").iter().map(|t| t_of(r, *t)).collect()
                } else {
                    Vec::new()
                };
                write_demand(v, r, b, &name, &head, (&name, &guard), &proof.parts)?;
            }
            b.fields().set_field(Field::Qualifiers(&qualifiers));
            b.fields().set_field(Field::Demand(&demand));
            Ok(())
        },
    )?;
    let apart = Program::components(&made).is_ok_and(|parts| parts.iter().all(|part| !(part.contains(rel) && part.contains(&name))));
    if apart {
        program.set(made);
    }
    Ok(apart)
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

fn restrict(program: &mut Program<'_>, rel: &str, products: &BTreeSet<String>, tuple: bool, require_anchor: bool) -> Result<bool, String> {
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
pub(super) fn restore_order_witnesses(program: &mut Program<'_>, narrowed: &BTreeSet<String>) -> Result<usize, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::parse;

    #[test]
    fn bound_columns_keep_wildcards_and_scope_the_point_walk() {
        let v = parse(
            "
            .decl kind(n: node, k: symbol)
            .decl child(n: node, c: node, i: number)
            .decl chosen(x: node)
            .input kind, child, chosen
            .decl peel(x: node, y: node)
            peel(x, x) :- kind(x, k), k != \"wrap\".
            peel(x, y) :- kind(x, \"wrap\"), child(x, c, _), peel(c, y).
            .decl missing(x: node)
            .output missing
            missing(x) :- chosen(x), !peel(x, _).
        ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert!(restrict(&mut p, "peel", &BTreeSet::new(), false, false).unwrap());
        let specs = p.shown(Some("rr_reader_demand_peel"));
        assert!(specs.iter().any(|r| r.contains("chosen($x)")));
        assert!(specs.iter().any(|r| r.starts_with("rr_reader_demand_peel($c)")
            && r.contains("rr_reader_demand_peel($x)")
            && r.contains("child($x, $c")));
    }

    #[test]
    fn a_negated_reader_supplies_its_filter_to_the_tuple_demand() {
        let v = parse(
            "
            .decl attrs(n: node, r: node, s: symbol)
            .decl a(n: node)
            .decl b(n: node)
            .input attrs, a, b
            .decl product(r: node, n: node)
            product(r, n) :- a(r), b(n).
            .decl reader(r: node, n: node)
            reader(r, n) :- attrs(n, r, _), product(r, n).
            .decl out(n: node)
            .output out
            out(n) :- attrs(n, r, s), prefix(\"_\", s), !reader(r, n).
            ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        let rewritten = super::super::rewrite(&mut p, true).unwrap();
        assert!(rewritten.products.contains("product"));
        assert!(rewritten.readers.contains("reader"));
        let proof = p.shown(Some("rr_reader_demand_reader")).remove(0);
        assert!(proof.contains("@prefix"), "{proof}");
        assert!(p.shown(Some("rr_product_demand_product")).iter().any(|r| r.contains("rr_reader_demand_reader(")));
    }

    #[test]
    fn a_node_maximum_retains_its_group_witness() {
        let v = parse(
            "
            .decl node_order(n: node, k: number)
            .decl site(n: node, b: node)
            .decl asked(n: node)
            .input node_order, site, asked
            .decl rr_ordered0(n: node, k: number)
            rr_ordered0(n, k) :- site(n, b), node_order(b, k).
            .decl selected(n: node, b: node)
            .output selected
            selected(n, b) :- asked(n), k = max r : { rr_ordered0(n, r) }, node_order(b, k).
            ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert_eq!(
            restore_order_witnesses(&mut p, &BTreeSet::from(["selected".to_owned()])).unwrap(),
            1
        );
        let selected = p.shown(Some("selected")).remove(0);
        assert!(selected.ends_with("site($n, $b)"), "{selected}");
    }

    #[test]
    fn an_open_reader_preserves_the_whole_relation() {
        let v = parse(
            "
            .decl edge(x: node, y: node)
            .input edge
            .decl all(x: node, y: node)
            all(x, y) :- edge(x, y).
            .decl out(x: node, y: node)
            .output out
            out(x, y) :- all(x, y).
        ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert!(!restrict(&mut p, "all", &BTreeSet::new(), true, false).unwrap());
        assert!(p.columns("rr_reader_demand_all").is_none());
    }
}
