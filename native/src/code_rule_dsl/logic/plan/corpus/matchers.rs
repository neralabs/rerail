//! A program's `ast(d, n, { … })` atoms, run by the transducer's matchers (decision 0106).
//!
//! Each atom without captures becomes an input relation `rr_ast_<k>(n)`, one per distinct matcher, and `docof(n, d)`
//! where the rule reads `d` elsewhere. Each chunk fills the relations by running every matcher's labels and roots over
//! its documents' tapes (`facts::vtab::syntax::with_matchers`); the joins read the matches as any base relation.
//! An atom with captures is left as written: the matchers give their roots only (decision 0106, open).
use crate::code_rule_dsl::facts::vtab::syntax::Matcher;
use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Term};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

/// The prefix of a matcher's relation.
pub(in crate::code_rule_dsl) const PREFIX: &str = "rr_ast_";

fn atom(rel: &str, args: Vec<Term>) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned(), args, neg: false, gated: false })
}

/// Rewrites `prog`'s `ast` atoms without captures over the matchers' relations: each relation and its matcher's text.
pub(in crate::code_rule_dsl) fn lower(prog: &mut Program) -> Vec<(String, String)> {
    let mut texts: Vec<String> = Vec::new();
    for rule in &mut prog.rules {
        if !rule.body.iter().any(|l| matches!(l, Literal::Ast { captures, .. } if captures.is_empty())) {
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
                Literal::Ast { document, node, matcher, captures } if captures.is_empty() => {
                    let k = texts.iter().position(|t| t == matcher).unwrap_or_else(|| {
                        texts.push(matcher.clone());
                        texts.len() - 1
                    });
                    rule.body.push(atom(&format!("{PREFIX}{k}"), vec![Term::Var(node.clone())]));
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
    for (k, text) in texts.into_iter().enumerate() {
        let rel = format!("{PREFIX}{k}");
        prog.decls.insert(rel.clone(), vec![("n".to_owned(), "node".to_owned())]);
        if !prog.inputs.contains(&rel) {
            prog.inputs.push(rel.clone());
        }
        out.push((rel, text));
    }
    if !out.is_empty() && !prog.inputs.iter().any(|r| r == "docof") && prog.rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == "docof"))) {
        prog.decls.entry("docof".to_owned()).or_insert_with(|| vec![("n".to_owned(), "loc".to_owned()), ("d".to_owned(), "number".to_owned())]);
        prog.inputs.push("docof".to_owned());
    }
    out
}

/// The process's lowered matchers, by their text and their base policy's: lowered once.
type Lowered = Result<(Arc<crate::code_rule_dsl::transducer::packed::LoweredMatcher>, Vec<crate::code_rule_dsl::matcher::DocumentGate>), String>;
static LOWERED: std::sync::LazyLock<Mutex<HashMap<(String, String), Lowered>>> = std::sync::LazyLock::new(Default::default);

/// Each relation's matcher lowered for the transducer, over the program's first `.policy` (its utils). A matcher that
/// does not lower fails the run, naming it.
pub(in crate::code_rule_dsl) fn compiled(matchers: &[(String, String)], policies: &[serde_json::Value]) -> Result<Vec<Matcher>, String> {
    let base = policies.first();
    let base_text = base.map(|b| b.to_string()).unwrap_or_default();
    let _span = crate::code_rule_dsl::logic::profile::span(|| "ast matchers lowered".into());
    matchers
        .iter()
        .map(|(rel, text)| {
            let key = (text.clone(), base_text.clone());
            let got = {
                let mut kept = LOWERED.lock().unwrap_or_else(|p| p.into_inner());
                kept.entry(key)
                    .or_insert_with(|| crate::code_rule_dsl::waves::policies::lower_matcher(text, base).map(|(m, g)| (Arc::new(m), g)))
                    .clone()
            };
            let (lowered, gates) = got.map_err(|why| format!("{rel}: the matcher {text} does not lower: {why}"))?;
            Ok(Matcher { rel: rel.clone(), lowered, gates })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `ast` atom without captures reads its matcher's relation, one per distinct matcher; `docof` joins the node
    /// to its document only where the rule reads the document; an atom with captures is left as written.
    #[test]
    fn ast_atoms_read_their_matchers_relations() {
        let src = r#"
.decl a(n: node)
.decl b(n: node, d: number)
.decl c(n: node, x: node)
.output a, b, c
a(n) :- ast(d, n, {"language": "python", "rule": {"kind": "call"}}).
b(n, d) :- ast(d, n, {"language": "python", "rule": {"kind": "call"}}).
c(n, x) :- ast(d, n, {"language": "python", "rule": {"pattern": "f($X)"}}, X: x).
"#;
        let mut prog = crate::code_rule_dsl::logic::rr::parse(src).unwrap();
        let found = lower(&mut prog);
        assert_eq!(found.len(), 1, "one distinct matcher without captures");
        let rel = &found[0].0;
        let body = |head: &str| prog.rules.iter().find(|r| r.head.rel == head).unwrap().body.clone();
        let reads = |head: &str| body(head).iter().filter_map(|l| l.atom().map(|a| a.rel.clone())).collect::<Vec<_>>();
        assert_eq!(reads("a"), vec![rel.clone()]);
        assert_eq!(reads("b"), vec![rel.clone(), "docof".to_owned()]);
        assert!(body("c").iter().any(|l| matches!(l, Literal::Ast { .. })), "a capture stays an ast atom");
        assert!(prog.inputs.contains(rel) && prog.decls.contains_key(rel));
    }
}
