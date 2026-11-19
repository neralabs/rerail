//! Bind independent node ranges to the tuples their readers can actually use.
//!
//! An intermediate product is safe to restrict to a superset of every read's keys. Build that superset from
//! each reader's connected positive context. When that context would itself form a product, require keys from
//! the reader's callers first. Reject a product with a reader that cannot supply a connected demand.

use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Rule, Term, Value, components_of};
use crate::code_rule_dsl::logic::plan::fields::Fields;
use crate::code_rule_dsl::logic::program_image::Version;
use std::collections::{BTreeMap, BTreeSet};

mod nullable;
mod upstream;

pub(in crate::code_rule_dsl::logic::plan) const PREFIX: &str = "rr_product_demand_";

fn atom(name: String, args: &[Term]) -> Atom {
    Atom {
        rel: name.into(),
        args: args.to_vec().into(),
        neg: false,
        gated: false,
    }
}

#[derive(Default)]
struct Context<'a> {
    vars: BTreeSet<String>,
    body: Vec<&'a Literal>,
    reads_input: bool,
}

fn connect<'a>(groups: &mut Vec<Context<'a>>, literal: &'a Literal, vars: BTreeSet<String>, reads_input: bool) {
    if vars.is_empty() {
        return;
    }
    let mut joined = Context {
        vars,
        body: vec![literal],
        reads_input,
    };
    groups.retain_mut(|other| {
        if joined.vars.is_disjoint(&other.vars) {
            return true;
        }
        joined.vars.append(&mut other.vars);
        joined.body.append(&mut other.body);
        joined.reads_input |= other.reads_input;
        false
    });
    groups.push(joined);
}

/// Relations that depend on the corpus, as opposed to constants defined by the program.
fn corpus_relations(program: &Program) -> BTreeSet<&str> {
    let mut readers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut pending: Vec<&str> = program.inputs.iter().map(String::as_str).collect();
    for rule in &program.rules {
        for literal in &rule.body {
            match literal {
                Literal::Atom(a) if !a.neg => readers.entry(&a.rel).or_default().push(&rule.head.rel),
                Literal::Ast { .. } | Literal::Nearest { .. } => pending.push(&rule.head.rel),
                _ => {}
            }
        }
    }
    let mut varies = BTreeSet::new();
    while let Some(rel) = pending.pop() {
        if varies.insert(rel) {
            pending.extend(readers.get(rel).into_iter().flatten().copied());
        }
    }
    varies
}

fn plain(term: &Term) -> bool {
    matches!(term, Term::Var(_) | Term::Int(_) | Term::Str(_) | Term::Branded(_))
}

fn disconnected_nodes(rule: &Rule, program: &Program, varies: &BTreeSet<&str>) -> bool {
    let fixed: BTreeSet<&str> = rule
        .body
        .iter()
        .filter_map(|literal| match literal {
            Literal::Cmp {
                l: Term::Var(v),
                op: "=",
                r,
            } if plain(r) && !matches!(r, Term::Var(_)) => Some(v.as_str()),
            _ => None,
        })
        .collect();
    let node_vars: BTreeSet<&str> = rule
        .head
        .args
        .iter()
        .zip(&program.decls[&rule.head.rel])
        .filter_map(|(term, (_, ty))| match term {
            Term::Var(v) if matches!(ty.as_str(), "node" | "loc") && !fixed.contains(v.as_str()) => Some(v.as_str()),
            _ => None,
        })
        .collect();
    if node_vars.len() < 2 {
        return false;
    }
    let mut groups = Vec::new();
    for literal in &rule.body {
        let reads_input = match literal {
            Literal::Atom(a) if !a.neg => varies.contains(a.rel.as_str()),
            Literal::Agg { atom, .. } => varies.contains(atom.rel.as_str()),
            Literal::Nearest { .. } | Literal::Ast { .. } => true,
            Literal::Cmp { op: "=", .. } => false,
            _ => continue,
        };
        connect(&mut groups, literal, literal.vars(), reads_input);
    }
    groups
        .iter()
        .filter(|group| group.reads_input && group.vars.iter().any(|v| node_vars.contains(v.as_str())))
        .take(2)
        .count()
        == 2
}

/// Whether any rule of the version is a node product (`disconnected_nodes`, read on the version: decision 0281's
/// path): the tree is decoded for `rewrite` only then, so a program with none decodes nothing.
pub(super) fn has_products(v: &Version, f: &Fields<'_>) -> bool {
    use crate::code_rule_dsl::logic::plan::body::{VarSet, lit_vars};
    use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit as L, TermRef};
    // The relations that depend on the corpus (`corpus_relations`).
    let mut readers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut pending: Vec<&str> = f.inputs().collect();
    for rule in v.rules() {
        let head = v.rel_name(rule.head().rel());
        for l in rule.body() {
            match l.view() {
                L::Atom(a) if !a.neg() => readers.entry(v.rel_name(a.rel())).or_default().push(head),
                L::Ast { .. } | L::Nearest { .. } => pending.push(head),
                _ => {}
            }
        }
    }
    let mut varies = BTreeSet::new();
    while let Some(rel) = pending.pop() {
        if varies.insert(rel) {
            pending.extend(readers.get(rel).into_iter().flatten().copied());
        }
    }
    let constant = |t: TermRef<'_>| matches!(t, TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_));
    v.rules().any(|rule| {
        let head = rule.head();
        let head_rel = v.rel_name(head.rel());
        if !f.declared(head_rel) {
            return false;
        }
        let fixed: Vec<_> = rule.body().filter_map(|l| match l.view() {
            L::Cmp { l: TermRef::Var(x), op: CmpOp::Eq, r } if constant(r) => Some(x),
            _ => None,
        }).collect();
        let node_vars = VarSet::of(head.args().enumerate().filter_map(|(c, t)| match t {
            TermRef::Var(x) if matches!(f.column_type(head_rel, c), Some("node" | "loc")) && !fixed.contains(&x) => Some(x),
            _ => None,
        }));
        if node_vars.len() < 2 {
            return false;
        }
        // Each literal's variables joined into groups (`connect`), and whether a group reads the corpus.
        let mut groups: Vec<(VarSet, bool)> = Vec::new();
        for l in rule.body() {
            let reads_input = match l.view() {
                L::Atom(a) if !a.neg() => varies.contains(v.rel_name(a.rel())),
                L::Agg { atom, .. } => varies.contains(v.rel_name(atom.rel())),
                L::Nearest { .. } | L::Ast { .. } => true,
                L::Cmp { op: CmpOp::Eq, .. } => false,
                _ => continue,
            };
            let mut joined = (lit_vars(l), reads_input);
            if joined.0.is_empty() {
                continue;
            }
            groups.retain(|(vars, reads)| {
                if joined.0.is_disjoint(vars) {
                    return true;
                }
                joined.0.extend(vars);
                joined.1 |= *reads;
                false
            });
            groups.push(joined);
        }
        groups.iter().filter(|(vars, reads)| *reads && !vars.is_disjoint(&node_vars)).take(2).count() == 2
    })
}

fn refusal(relation: &str, reason: impl std::fmt::Display) -> String {
    format!("unbounded node Cartesian product in `{relation}`: {reason}")
}

fn guard(rule: &Rule) -> Literal {
    Literal::Atom(atom(format!("{PREFIX}{}", rule.head.rel), &rule.head.args))
}

fn contexts<'a>(rule: &'a Rule, guard: Option<&'a Literal>, demanded: &BTreeSet<String>) -> Vec<Context<'a>> {
    let mut groups = Vec::new();
    for literal in guard.into_iter().chain(&rule.body) {
        // A demand can only use independent evidence. Ignore filters, aggregate results, and the relations
        // whose answers are being restricted. Dropping these conditions only adds keys.
        if let Literal::Atom(a) = literal
            && !a.neg
            && !demanded.contains(a.rel.as_str())
            && a.args.iter().all(|t| !matches!(t, Term::Call(..)))
        {
            connect(&mut groups, literal, a.vars(), false);
        }
    }
    groups
}

fn check_target(program: &Program, rel: &str) -> Result<(), String> {
    if program.outputs.iter().any(|r| r == rel)
        || program.inputs.iter().any(|r| r == rel)
        || program.facts.contains_key(rel)
        || program.lattice.contains_key(rel)
        || program.natives.contains_key(rel)
        || program.asked_by.contains_key(rel)
        || program.qualified(rel, "no_magic")
    {
        return Err(refusal(rel, "the relation must be computed in full"));
    }
    if program.decls.contains_key(format!("{PREFIX}{rel}").as_str()) {
        return Err(refusal(rel, "the demand relation's name is already in use"));
    }
    if program
        .rules
        .iter()
        .filter(|r| r.head.rel == rel)
        .any(|r| !r.head.args.iter().all(plain))
    {
        return Err(refusal(rel, "a computed head cannot be bounded by a tuple demand"));
    }
    Ok(())
}

/// Guard the products before splitting the program by document or planning its joins.
pub(super) fn rewrite(program: &mut Program, narrow_readers: bool) -> Result<Rewritten, String> {
    let varies = corpus_relations(program);
    let products: BTreeSet<String> = program
        .rules
        .iter()
        .filter(|rule| program.decls.contains_key(&rule.head.rel) && disconnected_nodes(rule, program, &varies))
        .map(|rule| rule.head.rel.to_string())
        .collect();
    if narrow_readers && !products.is_empty() {
        let mut strict = program.clone();
        let products = rewrite_products(&mut strict, products)?;
        let mut trial = program.clone();
        let mut readers = upstream::start(&mut trial, &products);
        if !readers.is_empty() {
            if let Ok(products) = rewrite_products(&mut trial, products.clone()) {
                upstream::propagate(&mut trial, &products, &mut readers);
                let order_witnesses = upstream::restore_order_witnesses(&mut trial, &readers);
                let present_witnesses = nullable::restore(&mut trial);
                *program = trial;
                return Ok(Rewritten {
                    products,
                    readers,
                    order_witnesses,
                    present_witnesses,
                });
            }
        }
        *program = strict;
        return Ok(Rewritten {
            products,
            readers: BTreeSet::new(),
            order_witnesses: 0,
            present_witnesses: 0,
        });
    }
    Ok(Rewritten {
        products: rewrite_products(program, products)?,
        readers: BTreeSet::new(),
        order_witnesses: 0,
        present_witnesses: 0,
    })
}

pub(super) struct Rewritten {
    pub products: BTreeSet<String>,
    pub readers: BTreeSet<String>,
    pub order_witnesses: usize,
    pub present_witnesses: usize,
}

fn rewrite_products(program: &mut Program, mut demanded: BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    if demanded.is_empty() {
        return Ok(demanded);
    }
    for rel in &demanded {
        check_target(program, rel)?;
    }

    loop {
        let mut more = BTreeSet::new();
        let mut rules = Vec::new();
        let mut facts = BTreeMap::<String, Vec<_>>::new();
        for reader in &program.rules {
            let mut reads = reader
                .body
                .iter()
                .filter(|l| l.atom().is_some_and(|a| demanded.contains(a.rel.as_str())))
                .peekable();
            if reads.peek().is_none() {
                continue;
            }
            let own = demanded.contains(reader.head.rel.as_str());
            let own_guard = guard(reader);
            let available = contexts(reader, own.then_some(&own_guard), &demanded);
            for read in reads {
                let a = read.atom().expect("a read of a demanded relation");
                let from_reader = || format!("the read in `{}` has no connected demand", reader.head.rel);
                if !matches!(read, Literal::Atom(_)) || a.gated || !a.args.iter().all(plain) {
                    return Err(refusal(&a.rel, from_reader()));
                }
                let name = format!("{PREFIX}{}", a.rel);
                let keys = a.vars();
                if keys.is_empty() {
                    let row = a
                        .args
                        .iter()
                        .map(|t| match t {
                            Term::Int(n) => Value::Int(*n),
                            Term::Str(s) => Value::Str(s.clone()),
                            Term::Branded(n) => Value::Branded(*n),
                            _ => unreachable!("constant demand"),
                        })
                        .collect();
                    facts.entry(name).or_default().push(row);
                } else if let Some(source) = available.iter().find(|c| keys.is_subset(&c.vars)) {
                    rules.push(Rule {
                        head: atom(name, &a.args),
                        body: source.body.iter().map(|l| (*l).clone()).collect(),
                    });
                } else if !own
                    && contexts(reader, Some(&own_guard), &demanded)
                        .iter()
                        .any(|c| keys.is_subset(&c.vars))
                {
                    check_target(program, &reader.head.rel)?;
                    more.insert(reader.head.rel.to_string());
                } else {
                    return Err(refusal(&a.rel, from_reader()));
                }
            }
        }
        if !more.is_empty() {
            demanded.extend(more);
            continue;
        }

        for rel in &demanded {
            let name = format!("{PREFIX}{rel}");
            program
                .decls
                .insert(name.clone().into(), program.decls[rel.as_str()].clone());
            program
                .qualifiers
                .entry(name)
                .or_default()
                .extend(["no_inline".to_owned(), "no_magic".to_owned()]);
        }
        for rule in &mut program.rules {
            if demanded.contains(rule.head.rel.as_str()) {
                rule.body.insert(0, guard(rule));
            }
        }
        program.rules.extend(rules);
        program.facts.extend(facts);
        let components =
            components_of(program).map_err(|e| refusal(demanded.first().expect("at least one product"), e))?;
        for component in components.iter().filter(|c| c.len() > 1) {
            if let Some(rel) = component
                .iter()
                .find(|r| demanded.contains(*r) && component.contains(&format!("{PREFIX}{r}")))
            {
                return Err(refusal(rel, "the demand would depend on the product it bounds"));
            }
        }
        return Ok(demanded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::rr::parse;

    const INPUTS: &str = "
        .decl a(n: node)
        .decl b(n: node)
        .decl edge(from: node, to: node)
        .input a, b, edge
        .decl product(x: node, y: node)
        product(x, y) :- a(x), b(y).
    ";

    #[test]
    fn a_product_gets_only_the_pairs_its_readers_bind() {
        let mut program = parse(&format!(
            "{INPUTS}
            .decl relay(x: node, y: node)
            relay(x, y) :- product(x, y), a(x), b(y).
            .decl present(x: node, y: node)
            .decl missing(x: node, y: node)
            .output present, missing
            present(x, y) :- edge(x, y), relay(x, y).
            missing(x, y) :- edge(x, y), !relay(x, y).
        "
        ))
        .unwrap();
        assert_eq!(
            rewrite(&mut program, false).unwrap().products,
            BTreeSet::from(["product".to_owned(), "relay".to_owned()])
        );
        let rules: BTreeSet<String> = program.rules.iter().map(ToString::to_string).collect();
        assert!(rules.contains("product($x, $y) :- rr_product_demand_product($x, $y), a($x), b($y)"));
        assert!(rules.contains("rr_product_demand_relay($x, $y) :- edge($x, $y)"));
        assert!(rules.contains("present($x, $y) :- edge($x, $y), relay($x, $y)"));
        assert!(rules.contains("missing($x, $y) :- edge($x, $y), !relay($x, $y)"));
    }

    #[test]
    fn an_output_that_requires_every_pair_is_refused() {
        let mut program = parse(&format!("{INPUTS}\n.output product\n")).unwrap();
        let before = program.rules.clone();
        let error = rewrite(&mut program, true).err().unwrap();
        assert!(
            error.contains("unbounded node Cartesian product in `product`"),
            "{error}"
        );
        assert_eq!(program.rules, before);
    }

    #[test]
    fn connected_joins_and_program_constants_stay_as_written() {
        let mut program = parse(
            "
            .decl edge(from: node, to: node)
            .input edge
            .decl constant(n: node)
            constant(1).
            .decl out(x: node, y: node)
            .output out
            out(x, y) :- edge(x, z), edge(z, y).
            out(x, y) :- edge(x, _), constant(y).
            out(x, y) :- edge(x, _), k = max z : { edge(x, z) }, edge(y, k).
        ",
        )
        .unwrap();
        let before = program.rules.clone();
        assert!(rewrite(&mut program, false).unwrap().products.is_empty());
        assert_eq!(program.rules, before);
    }

    /// The version's detection (`has_products`, decision 0281's path) finds a product where the tree's does: the tree
    /// is decoded for the rewrite only then.
    #[test]
    fn the_version_finds_the_products_the_tree_finds() {
        let programs = [
            format!("{INPUTS}\n.output product\n"),
            format!("{INPUTS}\n.decl out(x: node)\n.output out\nout(x) :- product(x, _), edge(x, _).\n"),
            "
            .decl edge(from: node, to: node)
            .input edge
            .decl constant(n: node)
            constant(1).
            .decl out(x: node, y: node)
            .output out
            out(x, y) :- edge(x, z), edge(z, y).
            out(x, y) :- edge(x, _), constant(y).
            out(x, y) :- edge(x, _), k = max z : { edge(x, z) }, edge(y, k).
            out(x, y) :- edge(x, _), y = 1.
            "
            .to_owned(),
        ];
        for (i, text) in programs.iter().enumerate() {
            let program = parse(text).unwrap();
            let varies = corpus_relations(&program);
            let tree = program.rules.iter().any(|r| program.decls.contains_key(&r.head.rel) && disconnected_nodes(r, &program, &varies));
            let placed = crate::code_rule_dsl::logic::plan::body::Placed::of_program(&program).unwrap();
            let f = Fields::of(&placed).unwrap();
            assert_eq!(has_products(&placed.v, &f), tree, "program {i}");
            assert_eq!(tree, i < 2, "program {i}");
        }
    }
}
