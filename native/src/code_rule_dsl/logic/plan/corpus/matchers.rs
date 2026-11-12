//! A program's `ast(d, n, { … })` atoms, run by the transducer's matchers (decision 0106).
//!
//! Each atom becomes an input relation, one per distinct matcher and captured names: `rr_ast_<k>(n)` without
//! captures, `rr_ast_<k>_<m>(n, x1, …, xm)` with `m` (decision 0107), and `docof(n, d)` where the rule reads `d`
//! elsewhere. Each chunk fills the relations by running the matchers' wave over its documents' tapes
//! (`facts::vtab::syntax::with_matchers`); a captured matcher's rows read W's bindings at each matched root. The
//! joins read the matches as any base relation.
use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Term};
use std::collections::{BTreeSet, HashMap};

pub use crate::code_rule_dsl::facts::grammar_shape::single_field;
#[cfg(any(test, feature = "test-hooks"))]
use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};

pub use crate::code_rule_dsl::facts::core::ast_matches::{MOST_CAPTURES, PREFIX, columns};

/// The relation of the `k`th distinct matcher, capturing `m` names.
fn relation(k: usize, m: usize) -> String {
    if m == 0 { format!("{PREFIX}{k}") } else { format!("{PREFIX}{k}_{m}") }
}

fn atom(rel: &str, args: Vec<Term>) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg: false, gated: false })
}

/// The tree-shaped parts of a Python program's rules as `ast` atoms (decision 0106 §4): a root `r` with
/// `kind(r, "K")` and fields `field(r, "F", x), kind(x, "K2")` whose `x` no other literal or the head reads becomes one
/// matcher, `{"all": [{"kind": "K"}, {"has": {"field": "F", "kind": "K2", "stopBy": "neighbor"}}, …]}`. With
/// `capture`, a field the rule reads further is part of it too, its node the matcher's capture (`"pattern": "$C1"`,
/// `C1: x`: decision 0107). Only a field a node of kind `K` holds at most one of ([`single_field`]): a matcher's
/// `field` is the field's first node, where `field(r, "F", x)` is each of them (`import os, sys`'s two names). A field
/// tested without a kind stays a join. The number of rules rewritten.
pub fn absorb(prog: &mut Program, capture: bool) -> usize {
    let mut rewritten = 0;
    let mut fresh = 0;
    for rule in &mut prog.rules {
        // A kind or field name as the program holds it (`python:call`), as the matcher names it (`call`).
        let str_of = |t: &Term| match t {
            Term::Str(s) => Some(s.strip_prefix("python:").unwrap_or(s).to_owned()),
            _ => None,
        };
        let var_of = |t: &Term| match t {
            Term::Var(v) => Some(v.clone()),
            _ => None,
        };
        let positive = |l: &Literal, rel: &str| match l {
            Literal::Atom(a) if !a.neg && a.rel == rel => Some(a.clone()),
            _ => None,
        };
        // How many literals (and the head) read each variable.
        let mut uses: HashMap<String, usize> = HashMap::new();
        for v in rule.head.args.iter().filter_map(var_of) {
            *uses.entry(v).or_default() += 1;
        }
        for l in &rule.body {
            for v in l.vars() {
                *uses.entry(v).or_default() += 1;
            }
        }
        let mut drop = BTreeSet::new();
        let mut added = Vec::new();
        for (ri, l) in rule.body.iter().enumerate() {
            let Some(root) = positive(l, "kind") else { continue };
            let (Some(r), Some(k)) = (var_of(&root.args[0]), str_of(&root.args[1])) else { continue };
            if drop.contains(&ri) {
                continue;
            }
            let mut parts = vec![serde_json::json!({"kind": k})];
            let mut captures: Vec<(String, String)> = Vec::new();
            let mut taken = vec![ri];
            for (fi, f) in rule.body.iter().enumerate() {
                let Some(field) = positive(f, "field") else { continue };
                if drop.contains(&fi) || field.args.len() != 3 || var_of(&field.args[0]).as_deref() != Some(r.as_str()) {
                    continue;
                }
                let (Some(name), Some(x)) = (str_of(&field.args[1]), var_of(&field.args[2])) else { continue };
                let read_again = uses.get(&x).is_some_and(|&n| n > 2);
                if uses.get(&x).is_none_or(|&n| n < 2) || (read_again && !capture) || x == r {
                    continue;
                }
                let kind = rule.body.iter().enumerate().find_map(|(ki, kl)| {
                    let a = positive(kl, "kind")?;
                    (var_of(&a.args[0]).as_deref() == Some(x.as_str())).then(|| str_of(&a.args[1]).map(|k2| (ki, k2)))?
                });
                let Some((ki, k2)) = kind else { continue };
                if !single_field(&k, &name) {
                    continue;
                }
                if read_again {
                    let name_of = format!("C{}", captures.len() + 1);
                    parts.push(serde_json::json!({"has": {"field": name, "kind": k2, "pattern": format!("${name_of}"), "stopBy": "neighbor"}}));
                    captures.push((name_of, x));
                } else {
                    parts.push(serde_json::json!({"has": {"field": name, "kind": k2, "stopBy": "neighbor"}}));
                }
                taken.extend([fi, ki]);
            }
            if parts.len() < 2 || captures.len() > MOST_CAPTURES {
                continue;
            }
            drop.extend(taken);
            let matcher = serde_json::json!({"language": "python", "rule": {"all": parts}}).to_string();
            fresh += 1;
            added.push(Literal::Ast { document: format!("rr_ast_d{fresh}"), node: r, matcher, captures });
        }
        if added.is_empty() {
            continue;
        }
        let body = std::mem::take(&mut rule.body);
        rule.body = added.into_iter().chain(body.into_iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, l)| l)).collect();
        rewritten += 1;
    }
    rewritten
}

/// Rewrites `prog`'s `ast` atoms over the matchers' relations: each relation, its matcher's text and the names it
/// captures (an atom naming more than [`MOST_CAPTURES`] is left as written).
pub fn lower(prog: &mut Program) -> Vec<(String, String, Vec<String>)> {
    let mut texts: Vec<(String, Vec<String>)> = Vec::new();
    for rule in &mut prog.rules {
        if !rule.body.iter().any(|l| matches!(l, Literal::Ast { captures, .. } if captures.len() <= MOST_CAPTURES)) {
            continue;
        }
        let body = std::mem::take(&mut rule.body);
        // The variables each literal reads, and the head's.
        let mut head = BTreeSet::new();
        for t in &rule.head.args {
            if let Term::Var(v) = t {
                head.insert(v.clone());
            }
        }
        let reads: Vec<BTreeSet<String>> = body.iter().map(Literal::vars).collect();
        for (i, l) in body.iter().enumerate() {
            match l {
                Literal::Ast { document, node, matcher, captures } if captures.len() <= MOST_CAPTURES => {
                    let names: Vec<String> = captures.iter().map(|(name, _)| name.clone()).collect();
                    let k = texts.iter().position(|(t, n)| t == matcher && *n == names).unwrap_or_else(|| {
                        texts.push((matcher.clone(), names.clone()));
                        texts.len() - 1
                    });
                    let mut args = vec![Term::Var(node.clone())];
                    args.extend(captures.iter().map(|(_, var)| Term::Var(var.clone())));
                    rule.body.push(atom(&relation(k, names.len()), args));
                    let elsewhere = head.contains(document) || reads.iter().enumerate().any(|(j, r)| j != i && r.contains(document));
                    if elsewhere {
                        rule.body.push(atom("docof", vec![Term::Var(node.clone()), Term::Var(document.clone())]));
                    }
                }
                l => rule.body.push(l.clone()),
            }
        }
    }
    let mut out = Vec::new();
    for (k, (text, names)) in texts.into_iter().enumerate() {
        let rel = relation(k, names.len());
        // A name of the rule's `transform` binds text (a symbol); every other name, and the root, a node.
        let made = transformed(&text);
        let types = std::iter::once("node").chain(names.iter().map(|name| if made.contains(name) { "sym" } else { "node" }));
        prog.decls.insert(rel.clone(), columns(&rel).iter().zip(types).map(|(c, t)| ((*c).to_owned(), t.to_owned())).collect());
        if !prog.inputs.contains(&rel) {
            prog.inputs.push(rel.clone());
        }
        out.push((rel, text, names));
    }
    // `docof` is an input unless the program derives it already (`hiding` writes it as a view where documents are
    // hidden): then the rules read the view, and an input of the same name would have no statistics.
    let derived = prog.rules.iter().any(|r| r.head.rel == "docof");
    if !out.is_empty() && !derived && !prog.inputs.iter().any(|r| r == "docof") && prog.rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == "docof"))) {
        prog.decls.entry("docof".to_owned()).or_insert_with(|| vec![("n".to_owned(), "loc".to_owned()), ("d".to_owned(), "number".to_owned())]);
        prog.inputs.push("docof".to_owned());
    }
    out
}

/// The names an `ast` atom's matcher text makes in its `transform`: they bind text, not nodes.
fn transformed(text: &str) -> BTreeSet<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("transform")?.as_object().map(|t| t.keys().cloned().collect()))
        .unwrap_or_default()
}

/// Each relation's matcher compiled for the transducer, over the program's first `.policy` (its utils), by the
/// registered compiler (`facts::vtab::syntax::AstCompiler`). A matcher that does not lower fails the run, naming it.
pub(in crate::code_rule_dsl) fn compiled(matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "ast matchers lowered".into());
    crate::code_rule_dsl::facts::vtab::syntax::compile_matchers(matchers, policies)
}
