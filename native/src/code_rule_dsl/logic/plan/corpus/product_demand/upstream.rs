//! Projected reader demand around a product. An optional restriction is installed only when every read supplies
//! the selected columns independently and the new demand sits outside the relation's dependency component.

use super::{Context, PREFIX as PRODUCT, atom, connect, plain};
use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Rule, Term, Value, components_of};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const PREFIX: &str = "rr_reader_demand_";

fn is_demand(rel: &str) -> bool {
    rel.starts_with(PREFIX) || rel.starts_with(PRODUCT)
}

pub(super) fn eligible(program: &Program, rel: &str) -> bool {
    program.decls.contains_key(rel)
        && !program.outputs.iter().any(|r| r == rel)
        && !program.inputs.iter().any(|r| r == rel)
        && !program.facts.contains_key(rel)
        && !program.lattice.contains_key(rel)
        && !program.natives.contains_key(rel)
        && !program.natives.values().any(|native| native.reads().contains(&rel))
        && !program.answers.contains_key(rel)
        && !program.answers.values().flatten().any(|answer| answer == rel)
        && !program
            .gates
            .iter()
            .any(|(name, gate)| name == rel || gate.rel == rel || gate.src.as_deref() == Some(rel))
        && !program.asked_by.contains_key(rel)
        && !program.demand.iter().any(|demand| demand == "*")
        && !program.qualified(rel, "no_magic")
        && !is_demand(rel)
}

fn keys(atom: &Atom, columns: &[usize]) -> Option<Vec<Term>> {
    columns
        .iter()
        .map(|&i| atom.args.get(i).filter(|term| plain(term)).cloned())
        .collect()
}

fn filter_term(term: &Term) -> bool {
    match term {
        Term::Call(f, args) => {
            matches!(f.as_str(), "prefix" | "suffix" | "glob" | "set") && args.iter().all(filter_term)
        }
        _ => plain(term),
    }
}

/// Follow relation reads backwards so a guard can drop any atom that would read the relation it bounds.
fn readers_of<'a>(program: &'a Program, target: &'a str) -> BTreeSet<&'a str> {
    let mut readers = BTreeMap::<&str, Vec<&str>>::new();
    for rule in &program.rules {
        for atom in rule.body.iter().filter_map(Literal::atom) {
            readers.entry(&atom.rel).or_default().push(&rule.head.rel);
        }
    }
    for (name, native) in &program.natives {
        for read in native.reads() {
            readers.entry(read).or_default().push(name);
        }
    }
    for (request, answers) in &program.answers {
        readers
            .entry(request)
            .or_default()
            .extend(answers.iter().map(String::as_str));
    }
    let mut found = BTreeSet::new();
    let mut pending = vec![target];
    while let Some(next) = pending.pop() {
        if found.insert(next) {
            pending.extend(readers.get(next).into_iter().flatten().copied());
        }
    }
    found
}

fn demand_body<'a>(
    rule: &'a Rule,
    dependent: &BTreeSet<&str>,
    products: &BTreeSet<String>,
    wanted: &BTreeSet<String>,
) -> Option<Vec<Literal>> {
    let mut groups: Vec<Context<'a>> = Vec::new();
    for literal in &rule.body {
        if let Literal::Atom(a) = literal
            && !a.neg
            && !a.gated
            && !dependent.contains(a.rel.as_str())
            && !products.contains(a.rel.as_str())
            && a.args.iter().all(|t| plain(t) || matches!(t, Term::Wild))
        {
            let vars = a.vars();
            // A reader's own guard already supplies these columns. Avoid repeating its other joins.
            if is_demand(&a.rel) && wanted.is_subset(&vars) {
                return Some(vec![literal.clone()]);
            }
            connect(&mut groups, literal, vars, false);
        }
    }
    let group = groups.into_iter().find(|group| wanted.is_subset(&group.vars))?;
    // Keep already-bound positive string and equality tests. Dropping other filters still gives a superset.
    Some(
        group
            .body
            .into_iter()
            .chain(rule.body.iter().filter(|literal| {
                matches!(literal, Literal::Cmp { l, r, .. } if filter_term(l) && filter_term(r))
                    && literal.vars().is_subset(&group.vars)
            }))
            .cloned()
            .collect(),
    )
}

fn install(
    program: &mut Program,
    rel: &str,
    cols: &[usize],
    products: &BTreeSet<String>,
    require_anchor: bool,
) -> bool {
    install_named(program, rel, cols, products, require_anchor, PREFIX)
}

pub(super) fn install_named(
    program: &mut Program,
    rel: &str,
    cols: &[usize],
    products: &BTreeSet<String>,
    require_anchor: bool,
    prefix: &str,
) -> bool {
    let name = format!("{prefix}{rel}");
    if program.decls.contains_key(name.as_str()) {
        return false;
    }
    let dependent = readers_of(program, rel);
    let mut anchored = false;
    let mut proofs = Vec::new();
    let mut rows = Vec::new();
    let mut producers = Vec::new();
    let mut readers = 0;
    for (i, rule) in program.rules.iter().enumerate() {
        let own = rule.head.rel == rel;
        let Some(own_key) = (!own).then(Vec::new).or_else(|| keys(&rule.head, cols)) else {
            return false;
        };
        let own_guard = Literal::Atom(atom(name.clone(), &own_key));
        if own {
            producers.push((i, own_guard.clone()));
        }
        for literal in &rule.body {
            let Some(read) = literal.atom().filter(|a| a.rel == rel) else {
                continue;
            };
            if read.gated {
                return false;
            }
            // For aggregates, the prefix below must bind every chosen column outside the aggregate.
            // Inner columns stay out of the guard, so the aggregate still sees every row of each group.
            let Some(key) = keys(read, cols) else { return false };
            if own && key == own_key && matches!(literal, Literal::Atom(_)) && !read.neg {
                continue;
            }
            readers += 1;
            if readers > 48 {
                return false;
            }
            let head = atom(name.clone(), &key);
            let wanted = head.vars();
            if wanted.is_empty() {
                anchored = true;
                rows.push(
                    key.into_iter()
                        .map(|t| match t {
                            Term::Int(n) => Value::Int(n),
                            Term::Str(s) => Value::Str(s),
                            Term::Branded(n) => Value::Branded(n),
                            _ => unreachable!("a constant key"),
                        })
                        .collect(),
                );
                continue;
            }
            let body = if own {
                // For a recursive walk, start with the parent's requested key and derive the child's key.
                let mut reader = rule.clone();
                reader.body.insert(0, own_guard.clone());
                demand_body(&reader, &dependent, products, &wanted)
            } else {
                demand_body(rule, &dependent, products, &wanted)
            };
            let Some(body) = body else { return false };
            if !own {
                anchored |= body.iter().any(|literal| match literal {
                    Literal::Atom(a) => is_demand(&a.rel) && wanted.is_subset(&a.vars()),
                    Literal::Cmp {
                        l: Term::Call(f, _), ..
                    } => {
                        matches!(f.as_str(), "prefix" | "glob" | "suffix")
                    }
                    _ => false,
                });
            }
            proofs.push(Rule {
                head,
                body: body.into(),
            });
        }
    }
    if producers.is_empty() || readers == 0 || (require_anchor && !anchored) {
        return false;
    }
    let original_len = program.rules.len();
    let columns = cols.iter().map(|&i| program.decls[rel][i].clone()).collect();
    program.decls.insert(name.clone().into(), columns);
    if !rows.is_empty() {
        program.facts.insert(name.clone(), rows);
    }
    for (i, guard) in &producers {
        program.rules[*i].body.insert(0, guard.clone());
    }
    program.rules.extend(proofs);
    if components_of(program).is_ok_and(|parts| parts.iter().all(|part| !(part.contains(rel) && part.contains(&name))))
    {
        // This relation is already guarded. Keep explicit demand on other relations.
        program.demand.retain(|demand| demand != rel);
        program
            .qualifiers
            .entry(name)
            .or_default()
            .extend(["no_inline".to_owned(), "no_magic".to_owned()]);
        return true;
    }
    for (i, _) in &producers {
        program.rules[*i].body.remove(0);
    }
    program.rules.truncate(original_len);
    program.decls.remove(name.as_str());
    program.facts.remove(&name);
    false
}

fn node_columns(program: &Program, rel: &str) -> Vec<usize> {
    program
        .decls
        .get(rel)
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, (_, ty))| matches!(ty.as_str(), "node" | "loc"))
        .map(|(i, _)| i)
        .collect()
}

fn restrict(program: &mut Program, rel: &str, products: &BTreeSet<String>, tuple: bool, require_anchor: bool) -> bool {
    if products.contains(rel) || !eligible(program, rel) {
        return false;
    }
    let nodes = node_columns(program, rel);
    if (tuple && nodes.len() > 1 && install(program, rel, &nodes, products, require_anchor))
        || nodes
            .into_iter()
            .any(|i| install(program, rel, &[i], products, require_anchor))
    {
        return true;
    }
    // A symbol can also bound a read, for example a requested qualified name.
    let symbols: Vec<_> = program.decls[rel]
        .iter()
        .enumerate()
        .filter(|(_, (_, ty))| matches!(ty.as_str(), "sym" | "symbol"))
        .map(|(i, _)| i)
        .collect();
    symbols.into_iter().any(|i| install(program, rel, &[i], products, true))
}

/// Anchor the readers of a mandatory product before its own demand is derived.
pub(super) fn start(program: &mut Program, products: &BTreeSet<String>) -> BTreeSet<String> {
    let readers: BTreeSet<_> = program
        .rules
        .iter()
        .filter(|r| {
            r.body
                .iter()
                .filter_map(Literal::atom)
                .any(|a| products.contains(a.rel.as_str()))
        })
        .map(|r| r.head.rel.to_string())
        .filter(|r| !products.contains(r))
        .collect();
    let mut peers = BTreeSet::new();
    for rule in &program.rules {
        let negated: Vec<_> = rule
            .body
            .iter()
            .filter_map(Literal::atom)
            .filter(|atom| atom.neg)
            .collect();
        for read in negated.iter().filter(|atom| readers.contains(atom.rel.as_str())) {
            let Some(column) = node_columns(program, &read.rel).first().copied() else {
                continue;
            };
            let Some(Term::Var(anchor)) = read.args.get(column) else {
                continue;
            };
            // Other tests of the anchor or of a fixed name can share the demand. Tests of unrelated keys
            // keep their own plan, which can be cheaper than enumerating them from the candidate.
            peers.extend(
                negated
                    .iter()
                    .filter(|atom| {
                        atom.args.iter().any(|term| {
                            matches!(term, Term::Var(v) if v == anchor)
                                || matches!(term, Term::Str(_) | Term::Branded(_))
                        })
                    })
                    .map(|atom| atom.rel.to_string()),
            );
        }
    }
    let mut narrowed = BTreeSet::new();
    for rel in readers.iter().chain(peers.difference(&readers)).take(24) {
        if restrict(program, rel, products, readers.contains(rel), false) {
            narrowed.insert(rel.clone());
        }
    }
    narrowed
}

/// Carry keys to the inputs of guarded rules. Restriction stays optional when a helper also has an open reader.
pub(super) fn propagate(program: &mut Program, products: &BTreeSet<String>, narrowed: &mut BTreeSet<String>) {
    let mut tried = narrowed.clone();
    let mut budget = 96;
    for _ in 0..4 {
        let candidates: BTreeSet<_> = program
            .rules
            .iter()
            .filter(|r| is_demand(&r.head.rel) || r.body.iter().filter_map(Literal::atom).any(|a| is_demand(&a.rel)))
            .flat_map(|r| r.body.iter().filter_map(Literal::atom))
            .map(|a| a.rel.to_string())
            .filter(|r| !tried.contains(r) && eligible(program, r))
            .collect();
        let before = narrowed.len();
        for rel in candidates.into_iter().take(budget) {
            budget -= 1;
            tried.insert(rel.clone());
            if restrict(program, &rel, products, false, true) {
                narrowed.insert(rel);
            }
        }
        if budget == 0 || before == narrowed.len() {
            break;
        }
    }
}

/// Restate the witness of a node minimum or maximum after reversing its order key. Its original atom tells
/// placement which document supplied the node. An empty group has no node_order row. Node-order keys in the tape
/// binding are injective, so a nonempty group's returned node is a witness of the original atom.
pub(super) fn restore_order_witnesses(program: &mut Program, narrowed: &BTreeSet<String>) -> usize {
    use crate::code_rule_dsl::logic::front::AggOp;
    if !program.inputs.iter().any(|r| r == "node_order")
        || program.facts.contains_key("node_order")
        || program.natives.contains_key("node_order")
        || program.rules.iter().any(|r| r.head.rel == "node_order")
    {
        return 0;
    }
    let mut edits = Vec::new();
    for (index, rule) in program.rules.iter().enumerate() {
        if !narrowed.contains(rule.head.rel.as_str()) {
            continue;
        }
        for literal in &rule.body {
            let Literal::Agg {
                var,
                op: AggOp::Min | AggOp::Max,
                over: Some(over),
                atom: aggregate,
            } = literal
            else {
                continue;
            };
            // The synthesized helper projects the ordered node to its preorder key.
            if !aggregate.rel.starts_with("rr_ordered")
                || program.inputs.iter().any(|r| *r == aggregate.rel)
                || program.facts.contains_key(aggregate.rel.as_str())
                || program.natives.contains_key(aggregate.rel.as_str())
                || aggregate.neg
                || aggregate.gated
            {
                continue;
            }
            let Some(Term::Var(result)) = rule.body.iter().find_map(|l| match l {
                Literal::Atom(a)
                    if !a.neg && a.rel == "node_order" && a.args.get(1) == Some(&Term::Var(var.clone().into())) =>
                {
                    a.args.first()
                }
                _ => None,
            }) else {
                continue;
            };
            let mut defs = program.rules.iter().filter(|r| r.head.rel == aggregate.rel);
            let Some(helper) = defs.next() else { continue };
            if defs.next().is_some() {
                continue;
            }
            let mut body = helper.body.iter().filter(|literal| match literal {
                Literal::Atom(a) => !is_demand(&a.rel),
                Literal::Cmp { .. } => false,
                _ => true,
            });
            let (Some(Literal::Atom(source)), Some(Literal::Atom(order)), None) =
                (body.next(), body.next(), body.next())
            else {
                continue;
            };
            let [Term::Var(node), Term::Var(order_key)] = order.args.as_slice() else {
                continue;
            };
            if source.neg || source.gated || order.neg || order.gated || order.rel != "node_order" || node == order_key
            {
                continue;
            }
            let outer: BTreeSet<_> = rule
                .body
                .iter()
                .filter_map(|l| match l {
                    Literal::Atom(a) if !a.neg => Some(a.vars()),
                    _ => None,
                })
                .flatten()
                .collect();
            let mut matched = false;
            let arguments: Option<Vec<_>> = source
                .args
                .iter()
                .map(|t| match t {
                    Term::Var(v) if v == node => {
                        matched = true;
                        Some(Term::Var(result.clone()))
                    }
                    Term::Var(v) => {
                        let pos = helper.head.args.iter().position(|h| h == &Term::Var(v.clone()))?;
                        match aggregate.args.get(pos)? {
                            Term::Var(v) if !outer.contains(v.as_str()) => Some(Term::Wild),
                            term if plain(term) || matches!(term, Term::Wild) => Some(term.clone()),
                            _ => None,
                        }
                    }
                    Term::Int(_) | Term::Str(_) | Term::Branded(_) | Term::Wild => Some(t.clone()),
                    _ => None,
                })
                .collect();
            let Some(pos) = helper.head.args.iter().position(|t| t == &Term::Var(order_key.clone())) else {
                continue;
            };
            if aggregate.args.get(pos) == Some(&Term::Var(over.clone().into()))
                && matched
                && let Some(arguments) = arguments
            {
                edits.push((index, Literal::Atom(atom(source.rel.to_string(), &arguments))));
            }
        }
    }
    let count = edits.len();
    for (index, atom) in edits {
        program.rules[index].body.push(atom);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::rr::parse;

    #[test]
    fn bound_columns_keep_wildcards_and_scope_the_point_walk() {
        let mut p = parse(
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
        assert!(restrict(&mut p, "peel", &BTreeSet::new(), false, false));
        let specs: Vec<_> = p
            .rules
            .iter()
            .filter(|r| r.head.rel == "rr_reader_demand_peel")
            .map(ToString::to_string)
            .collect();
        assert!(specs.iter().any(|r| r.contains("chosen($x)")));
        assert!(specs.iter().any(|r| r.starts_with("rr_reader_demand_peel($c)")
            && r.contains("rr_reader_demand_peel($x)")
            && r.contains("child($x, $c")));
    }

    #[test]
    fn a_negated_reader_supplies_its_filter_to_the_tuple_demand() {
        let mut p = parse(
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
        let rewritten = super::super::rewrite(&mut p, true).unwrap();
        assert!(rewritten.products.contains("product"));
        assert!(rewritten.readers.contains("reader"));
        let proof = p
            .rules
            .iter()
            .find(|r| r.head.rel == "rr_reader_demand_reader")
            .unwrap();
        assert!(proof.to_string().contains("@prefix"), "{proof}");
        assert!(p.rules.iter().any(|r| {
            r.head.rel == "rr_product_demand_product"
                && r.body
                    .iter()
                    .any(|l| l.atom().is_some_and(|a| a.rel == "rr_reader_demand_reader"))
        }));
    }

    #[test]
    fn a_node_maximum_retains_its_group_witness() {
        let mut p = parse(
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
        assert_eq!(
            restore_order_witnesses(&mut p, &BTreeSet::from(["selected".to_owned()])),
            1
        );
        let selected = p.rules.iter().find(|r| r.head.rel == "selected").unwrap();
        assert!(selected.to_string().ends_with("site($n, $b)"), "{selected}");
    }

    #[test]
    fn an_open_reader_preserves_the_whole_relation() {
        let mut p = parse(
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
        assert!(!restrict(&mut p, "all", &BTreeSet::new(), true, false));
        assert!(!p.decls.contains_key("rr_reader_demand_all"));
    }
}
