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

/// The tree-shaped parts of a Python program's rules as `ast` atoms (decision 0106 §4): a root `r` with
/// `kind(r, "K")` and fields `field(r, "F", x), kind(x, "K2")` whose `x` no other literal or the head reads becomes one
/// matcher, `{"all": [{"kind": "K"}, {"has": {"field": "F", "kind": "K2", "stopBy": "neighbor"}}, …]}`. A field the
/// rule reads further, or tests without a kind, stays a join. The number of rules rewritten.
pub(in crate::code_rule_dsl) fn absorb(prog: &mut Program) -> usize {
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
            let mut taken = vec![ri];
            for (fi, f) in rule.body.iter().enumerate() {
                let Some(field) = positive(f, "field") else { continue };
                if drop.contains(&fi) || field.args.len() != 3 || var_of(&field.args[0]).as_deref() != Some(r.as_str()) {
                    continue;
                }
                let (Some(name), Some(x)) = (str_of(&field.args[1]), var_of(&field.args[2])) else { continue };
                if uses.get(&x) != Some(&2) {
                    continue;
                }
                let kind = rule.body.iter().enumerate().find_map(|(ki, kl)| {
                    let a = positive(kl, "kind")?;
                    (var_of(&a.args[0]).as_deref() == Some(x.as_str())).then(|| str_of(&a.args[1]).map(|k2| (ki, k2)))?
                });
                let Some((ki, k2)) = kind else { continue };
                parts.push(serde_json::json!({"has": {"field": name, "kind": k2, "stopBy": "neighbor"}}));
                taken.extend([fi, ki]);
            }
            if parts.len() < 2 {
                continue;
            }
            drop.extend(taken);
            let matcher = serde_json::json!({"language": "python", "rule": {"all": parts}}).to_string();
            fresh += 1;
            added.push(Literal::Ast { document: format!("rr_ast_d{fresh}"), node: r, matcher, captures: Vec::new() });
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
pub(in crate::code_rule_dsl) fn compiled(matchers: &[(String, String)], policies: &[serde_json::Value]) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
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
        .collect::<Result<Vec<_>, String>>()
        .and_then(crate::code_rule_dsl::facts::vtab::syntax::Matchers::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `ast` atom without captures reads its matcher's relation, one per distinct matcher; `docof` joins the node
    /// to its document only where the rule reads the document; an atom with captures is left as written.
    /// `kind`, `field`, `kind` with the field's node read nowhere else becomes one matcher; a field read again stays.
    #[test]
    fn tree_shaped_parts_become_matchers() {
        let src = r#"
.decl a(c: node)
.decl b(c: node, f: node)
.output a, b
a(c) :- kind(c, "call"), field(c, "function", f), kind(f, "attribute").
b(c, f) :- kind(c, "call"), field(c, "function", f), kind(f, "attribute").
"#;
        let mut prog = crate::code_rule_dsl::logic::rr::parse(src).unwrap();
        assert_eq!(absorb(&mut prog), 1);
        let body = |head: &str| prog.rules.iter().find(|r| r.head.rel == head).unwrap().body.clone();
        let a = body("a");
        assert_eq!(a.len(), 1);
        let Literal::Ast { matcher, node, .. } = &a[0] else { panic!("not a matcher: {:?}", a[0]) };
        assert_eq!(node, "c");
        let m: serde_json::Value = serde_json::from_str(matcher).unwrap();
        assert_eq!(m["rule"]["all"][1]["has"]["field"], "function");
        assert_eq!(m["rule"]["all"][1]["has"]["kind"], "attribute");
        assert_eq!(body("b").len(), 3, "a field the head reads stays a join");
    }

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
