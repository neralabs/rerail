//! Rule bodies simplified before planning (decision 0038), so a long or repeated rule costs what it says:
//!
//! - **Implied atoms drop.** A positive atom goes when another positive atom of the same relation has, at every
//!   position, the same term, or any term where it has `_`: `doc($d, $p, _)` beside `doc($d, $p, $m)`.
//! - **A kind test on a set member drops.** `node(_, $a, "k", _, _)` goes where `$a` is the ancestor of a
//!   `nearest($n, $a, set)` or `inside_<set>($n, $a)` of the same rule and the set is exactly `{k}`.
//! - **Identical relations merge.** Derived relations whose rules are equal once their heads and variables are
//!   renamed in order are one relation; readers read the first. Repeated to a fixpoint, so relations over merged
//!   ones merge too. Outputs, inputs, lattices, relations with facts and demanded relations are kept as they are.

use super::front::{AggOp, Atom, Literal, Program, Rule, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What was simplified.
#[derive(Debug, Default, PartialEq)]
pub(in crate::code_rule_dsl) struct Report {
    pub(in crate::code_rule_dsl) implied: usize,
    pub(in crate::code_rule_dsl) kind_tests: usize,
    /// Merged relations: (kept, merged into it).
    pub(in crate::code_rule_dsl) merged: Vec<(String, String)>,
}

pub(in crate::code_rule_dsl) fn simplify(prog: &mut Program) -> Report {
    let mut report = Report::default();
    for rule in &mut prog.rules {
        report.kind_tests += drop_kind_tests(rule, &prog.kindsets, &prog.decls);
        report.implied += drop_implied(rule);
    }
    if merge(prog, &mut report) {
        // A merge can leave a literal twice (`!m1($n), !m2($n)` became `!m1($n), !m1($n)`).
        for rule in &mut prog.rules {
            report.implied += drop_implied(rule);
        }
    }
    if super::profile::on() {
        let (i, k, m) = (report.implied, report.kind_tests, report.merged.len());
        super::profile::decision("simplified bodies (decision 0038)", || {
            format!("{i} implied atoms and {k} kind tests on set members dropped; {m} relations merged into an identical one")
        });
    }
    report
}

/// Whether `b` implies `a`: the same relation, both positive, and at each position the same term or `_` in `a`.
fn implies(b: &Atom, a: &Atom) -> bool {
    b.rel == a.rel && !a.neg && !b.neg && a.args.len() == b.args.len() && a.args.iter().zip(&b.args).all(|(x, y)| x == y || *x == Term::Wild)
}

fn drop_implied(rule: &mut Rule) -> usize {
    let atoms: Vec<Option<&Atom>> = rule.body.iter().map(|l| if let Literal::Atom(a) = l { Some(a) } else { None }).collect();
    // A dropped atom is implied by one that stays: by a different atom, or by an equal one earlier. Any literal
    // equal to an earlier one goes too.
    let drop: BTreeSet<usize> = (0..atoms.len())
        .filter(|&i| {
            if rule.body[..i].contains(&rule.body[i]) {
                return true;
            }
            let Some(a) = atoms[i] else { return false };
            (0..atoms.len()).any(|j| j != i && atoms[j].is_some_and(|b| implies(b, a) && b != a))
        })
        .collect();
    let n = drop.len();
    let mut at = 0;
    rule.body.retain(|_| {
        at += 1;
        !drop.contains(&(at - 1))
    });
    n
}

fn drop_kind_tests(rule: &mut Rule, kindsets: &BTreeMap<String, Vec<String>>, decls: &BTreeMap<String, Vec<(String, String)>>) -> usize {
    // `node`'s columns as declared: the node and its kind.
    let Some(cols) = decls.get("node") else { return 0 };
    let (Some(n), Some(k)) = (cols.iter().position(|(c, _)| c == "n"), cols.iter().position(|(c, _)| c == "kind")) else { return 0 };
    // The members of a single-kind set, by variable: `nearest`'s and `inside`'s ancestor, `has`'s descendant.
    let mut member: HashMap<String, &str> = HashMap::new();
    for l in &rule.body {
        let (a, set) = match l {
            Literal::Nearest { a, kindset, .. } => (a.clone(), kindset.as_str()),
            Literal::Atom(x) if !x.neg && x.args.len() == 2 => match super::front::axis(&x.rel) {
                Some((set, has)) => match &x.args[if has { 0 } else { 1 }] {
                    Term::Var(a) => (a.clone(), set),
                    _ => continue,
                },
                None => continue,
            },
            _ => continue,
        };
        if let Some([kind]) = kindsets.get(set).map(Vec::as_slice) {
            member.insert(a, kind.as_str());
        }
    }
    if member.is_empty() {
        return 0;
    }
    let before = rule.body.len();
    rule.body.retain(|l| {
        let Literal::Atom(x) = l else { return true };
        if x.rel != "node" || x.neg || x.args.len() != cols.len() {
            return true;
        }
        let only = x.args.iter().enumerate().all(|(c, t)| c == n || c == k || *t == Term::Wild);
        let tested = match (&x.args[n], &x.args[k]) {
            (Term::Var(a), Term::Str(kind)) => member.get(a) == Some(&kind.as_str()),
            _ => false,
        };
        !(only && tested)
    });
    before - rule.body.len()
}

/// A rule with its head named `@` and its variables numbered in order of first appearance.
fn canonical(rule: &Rule) -> String {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut name = |v: &str| -> String {
        let k = names.len();
        names.entry(v.to_owned()).or_insert_with(|| format!("v{k}")).clone()
    };
    fn term(t: &Term, name: &mut dyn FnMut(&str) -> String) -> Term {
        match t {
            Term::Var(v) => Term::Var(name(v)),
            Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| term(a, name)).collect()),
            t => t.clone(),
        }
    }
    let atom = |a: &Atom, name: &mut dyn FnMut(&str) -> String| Atom { args: a.args.iter().map(|t| term(t, name)).collect(), ..a.clone() };
    let mut head = atom(&rule.head, &mut name);
    let own = rule.head.rel.clone();
    head.rel = "@".into();
    let body: Vec<Literal> = rule
        .body
        .iter()
        .map(|l| match l {
            Literal::Atom(a) => {
                let mut a = atom(a, &mut name);
                if a.rel == own {
                    a.rel = "@".into();
                }
                Literal::Atom(a)
            }
            Literal::Cmp { l, op, r } => Literal::Cmp { l: term(l, &mut name), op, r: term(r, &mut name) },
            Literal::Agg { var, op, over, atom: a } => {
                let var = name(var);
                let over = over.as_ref().map(|o| name(o));
                let op = match op {
                    AggOp::Join { by, sep } => AggOp::Join { by: by.iter().map(|k| name(k)).collect(), sep: sep.clone() },
                    op => op.clone(),
                };
                Literal::Agg { var, op, over, atom: atom(a, &mut name) }
            }
            Literal::Nearest { n, a, kindset } => Literal::Nearest { n: name(n), a: name(a), kindset: kindset.clone() },
            Literal::Ast { document, node, matcher, captures } => Literal::Ast {
                document: name(document),
                node: name(node),
                matcher: matcher.clone(),
                captures: captures.iter().map(|(c, v)| (c.clone(), name(v))).collect(),
            },
        })
        .collect();
    Rule { head, body }.to_string()
}

/// Whether anything merged.
fn merge(prog: &mut Program, report: &mut Report) -> bool {
    let mut any = false;
    loop {
        // Each mergeable relation's rules, canonical and sorted, with its declared types.
        let mut by_rel: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for r in &prog.rules {
            by_rel.entry(r.head.rel.as_str()).or_default().push(canonical(r));
        }
        let keep = |rel: &str| {
            prog.outputs.iter().any(|o| o == rel)
                || prog.inputs.iter().any(|i| i == rel)
                || prog.lattice.contains_key(rel)
                || prog.facts.get(rel).is_some_and(|f| !f.is_empty())
                || prog.demand.iter().any(|d| d == rel)
                || prog.gates.iter().any(|(g, _)| g == rel)
        };
        let mut first: HashMap<(Vec<String>, Vec<String>), String> = HashMap::new();
        let mut rename: BTreeMap<String, String> = BTreeMap::new();
        for (rel, mut rules) in by_rel {
            if keep(rel) {
                continue;
            }
            rules.sort();
            let types: Vec<String> = prog.decls.get(rel).map(|c| c.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default();
            match first.get(&(rules.clone(), types.clone())) {
                Some(kept) => {
                    rename.insert(rel.to_owned(), kept.clone());
                }
                None => {
                    first.insert((rules, types), rel.to_owned());
                }
            }
        }
        if rename.is_empty() {
            return any;
        }
        any = true;
        prog.rules.retain(|r| !rename.contains_key(&r.head.rel));
        for rule in &mut prog.rules {
            for l in &mut rule.body {
                if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = l
                    && let Some(to) = rename.get(&a.rel)
                {
                    a.rel = to.clone();
                }
            }
        }
        for (from, to) in rename {
            prog.decls.remove(&from);
            report.merged.push((to, from));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implied_atoms_kind_tests_and_identical_relations_go() {
        let mut prog = super::super::rr::parse(
            r#"
            .decl node(d: number, n: node, kind: symbol, start: number, end: number)
            .decl doc(d: number, path: symbol, module: symbol)
            .input node
            .input doc
            .decl k0(kind: symbol)
            k0("function_definition").
            .decl b1(l: node, h: node)
            .decl b2(l: node, h: node)
            .decl m1(n: node)
            .decl m2(n: node)
            .decl out(n: node, p: symbol)
            .output out
            b1(low, high) :- inside(low, s, k0), node(_, s, "function_definition", _, _), inside(s, high, k0).
            b2(x, y) :- inside(x, z, k0), node(_, z, "function_definition", _, _), inside(z, y, k0).
            m1(n) :- inside(n, v, k0), !b1(n, v).
            m2(q) :- inside(q, w, k0), !b2(q, w).
            out(n, p) :- node(d, n, "call", _, _), !m1(n), !m2(n), doc(d, p, m), doc(d, p, _).
            "#,
        )
        .unwrap();
        let report = simplify(&mut prog);
        // `doc($d, $p, _)`, and the second `!m1($n)` the merge made.
        assert_eq!(report.implied, 2);
        // The kind test on a member of k0 = {function_definition}, in b1 and b2.
        assert_eq!(report.kind_tests, 2);
        // b2 merges into b1, then m2 (now equal to m1) into m1.
        assert_eq!(report.merged, vec![("b1".to_owned(), "b2".to_owned()), ("m1".to_owned(), "m2".to_owned())]);
        let out: Vec<String> = prog.rules.iter().filter(|r| r.head.rel == "out").map(|r| r.to_string()).collect();
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("!m1($n), doc") && !out[0].contains("m2") && !out[0].contains("doc($d, $p, _)"), "{}", out[0]);
        assert!(!prog.decls.contains_key("b2") && !prog.decls.contains_key("m2"));
    }
}
