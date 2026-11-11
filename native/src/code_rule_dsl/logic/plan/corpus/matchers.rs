//! A program's `ast(d, n, { … })` atoms, run by the transducer's matchers (decision 0106).
//!
//! Each atom becomes an input relation, one per distinct matcher and captured names: `rr_ast_<k>(n)` without
//! captures, `rr_ast_<k>_<m>(n, x1, …, xm)` with `m` (decision 0107), and `docof(n, d)` where the rule reads `d`
//! elsewhere. Each chunk fills the relations by running the matchers' wave over its documents' tapes
//! (`facts::vtab::syntax::with_matchers`); a captured matcher's rows read W's bindings at each matched root. The
//! joins read the matches as any base relation.
use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Term};
use std::collections::{BTreeSet, HashMap};

pub(in crate::code_rule_dsl) use crate::code_rule_dsl::facts::grammar_shape::single_field;
#[cfg(test)]
use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};

pub(in crate::code_rule_dsl) use crate::code_rule_dsl::facts::core::ast_matches::{MOST_CAPTURES, PREFIX, columns};

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
pub(in crate::code_rule_dsl) fn absorb(prog: &mut Program, capture: bool) -> usize {
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
pub(in crate::code_rule_dsl) fn lower(prog: &mut Program) -> Vec<(String, String, Vec<String>)> {
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
        assert_eq!(absorb(&mut prog.clone(), false), 1);
        let mut captured = prog.clone();
        assert_eq!(absorb(&mut captured, true), 2, "with captures, the field the head reads is part of the matcher");
        let b = captured.rules.iter().find(|r| r.head.rel == "b").unwrap().body.clone();
        assert_eq!(b.len(), 1);
        let Literal::Ast { captures, matcher, node, .. } = &b[0] else { panic!("not a matcher: {:?}", b[0]) };
        assert_eq!(node, "c");
        assert_eq!(captures, &vec![("C1".to_owned(), "f".to_owned())], "the field's node captured");
        let m: serde_json::Value = serde_json::from_str(matcher).unwrap();
        assert_eq!(m["rule"]["all"][1]["has"]["pattern"], "$C1");
        assert_eq!(absorb(&mut prog, false), 1);
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

    /// `any`'s arms bind into the environment only when they match, so two arms binding one name share nothing; a
    /// name two parts of one `all` bind is a join the labels do not carry, and stays refused.
    #[test]
    fn any_arms_binding_one_name_lower() {
        let lower = |rule: &str| crate::code_rule_dsl::waves::policies::lower_matcher(&format!(r#"{{"language": "python", "rule": {rule}}}"#), None).map(|_| ());
        lower(r#"{"any": [{"all": [{"kind": "call"}, {"pattern": "$X"}]}, {"all": [{"kind": "attribute"}, {"pattern": "$X"}]}]}"#).unwrap();
        let err = lower(r#"{"all": [{"pattern": "$X"}, {"has": {"pattern": "$X", "stopBy": "neighbor"}}]}"#).unwrap_err();
        assert!(err.contains("shared binding"), "{err}");
    }

    /// A comparison of captures, which the labels do not carry, is dropped where that only widens the matcher; one
    /// under an `any` arm left with nothing is not.
    #[test]
    fn a_matcher_relaxed_where_it_only_widens() {
        use crate::code_rule_dsl::waves::policies::relaxed;
        let m = |rule: &str| serde_json::from_str::<serde_json::Value>(&format!(r#"{{"language": "python", "rule": {rule}}}"#)).unwrap();
        let got = relaxed(&m(r#"{"all": [{"kind": "call"}, {"pattern": "$A"}, {"not": {"compare": {"left": "A", "right": "A", "op": "ne"}}}]}"#), false).unwrap();
        assert_eq!(got["rule"], serde_json::json!({"all": [{"kind": "call"}, {"pattern": "$A"}]}));
        let got = relaxed(&m(r#"{"kind": "call", "has": {"compare": {"left": "A", "right": "B", "op": "eq"}, "stopBy": "neighbor"}}"#), false).unwrap();
        assert_eq!(got["rule"], serde_json::json!({"kind": "call"}));
        assert!(relaxed(&m(r#"{"any": [{"kind": "call"}, {"compare": {"left": "A", "right": "B", "op": "eq"}}]}"#), false).is_none());
        assert!(relaxed(&m(r#"{"kind": "call"}"#), false).is_none());
        // A path test is W's to confirm; an `any` with an arm left matching every node goes with it.
        let got = relaxed(&m(r#"{"all": [{"kind": "call"}, {"any": [{"filePath": {"regex": "x"}}, {"kind": "call"}]}]}"#), false).unwrap();
        assert_eq!(got["rule"], serde_json::json!({"all": [{"kind": "call"}]}));
        // Every negation in a positive place, only where asked.
        let not = m(r#"{"all": [{"kind": "call"}, {"not": {"kind": "identifier"}}]}"#);
        assert!(relaxed(&not, false).is_none());
        assert_eq!(relaxed(&not, true).unwrap()["rule"], serde_json::json!({"all": [{"kind": "call"}]}));
        // The relaxed matcher lowers; its roots are confirmed by W's.
        let (_, _, captor) = crate::code_rule_dsl::waves::policies::lower_matcher(
            r#"{"language": "python", "rule": {"all": [{"kind": "call"}, {"has": {"pattern": "$A", "field": "function", "stopBy": "neighbor"}}, {"not": {"compare": {"left": "A", "right": "A", "op": "ne", "text": "node"}}}]}}"#,
            None,
        )
        .unwrap();
        assert!(captor.verify);
    }

    /// A field's kinds and a kind's leafhood from the grammar: what a long-text twin is made for.
    #[test]
    fn field_kinds_and_leaves_from_the_grammar() {
        assert_eq!(field_kinds("attribute"), Some(BTreeSet::from(["identifier".to_owned()])));
        assert!(field_kinds("function").is_some_and(|k| k.contains("call") && k.contains("attribute")));
        assert_eq!(field_kinds("no_such_field"), None);
        assert!(leaf_kind("identifier") && leaf_kind("integer"));
        assert!(!leaf_kind("call") && !leaf_kind("decorated_definition"));
    }

    /// A field's multiplicity from the grammar: `import os, sys` holds two names, a subscript several indexes.
    #[test]
    fn single_fields_from_the_grammar() {
        assert!(single_field("call", "function"));
        assert!(single_field("attribute", "object"));
        assert!(single_field("function_definition", "name"));
        assert!(!single_field("import_statement", "name"));
        assert!(!single_field("subscript", "subscript"));
        assert!(!single_field("identifier", "name"), "an alias target");
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
        assert_eq!(found.len(), 2, "one distinct matcher without captures, one with");
        let rel = &found[0].0;
        let body = |head: &str| prog.rules.iter().find(|r| r.head.rel == head).unwrap().body.clone();
        let reads = |head: &str| body(head).iter().filter_map(|l| l.atom().map(|a| a.rel.clone())).collect::<Vec<_>>();
        assert_eq!(reads("a"), vec![rel.clone()]);
        assert_eq!(reads("b"), vec![rel.clone(), "docof".to_owned()]);
        // A captured matcher's relation has the root and a column a capture (decision 0107).
        assert_eq!(found[1].0, "rr_ast_1_1");
        assert_eq!(found[1].2, vec!["X".to_owned()]);
        assert_eq!(reads("c"), vec!["rr_ast_1_1".to_owned()]);
        let Some(Literal::Atom(c)) = body("c").into_iter().next() else { panic!("not an atom") };
        assert_eq!(c.args, vec![Term::Var("n".into()), Term::Var("x".into())]);
        assert_eq!(columns("rr_ast_1_1"), &["n", "x1"]);
        assert_eq!(columns("rr_ast_0"), &["n"]);
        assert!(prog.inputs.contains(rel) && prog.decls.contains_key(rel));
    }

    /// A `filePath` test below a matcher's root conjuncts (here in an `any`) is the document's: the matcher holds in
    /// a file its path passes, by that branch, and elsewhere only by the other.
    #[test]
    fn a_nested_file_path_is_the_documents() {
        let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("nested-file-path");
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("a/x.py"), "f()\n").unwrap();
        std::fs::write(root.join("b/y.py"), "f()\nzz()\n").unwrap();
        let policy = root.with_extension("policy.rr");
        let rule = r#"{"language": "python", "rule": {"all": [{"kind": "call"}, {"any": [{"filePath": {"regex": "^a/"}}, {"has": {"field": "function", "regex": "^zz"}}]}]}}"#;
        std::fs::write(&policy, format!(".language python\n.violation hit(at: node) code \"X1\" \"hit\"\nhit(n) :- ast(d, n, {rule}).\n")).unwrap();
        let report = crate::code_rule_dsl::tools::run_policies(&[policy.clone()], &root, &root.join("out"), &serde_json::Value::Null).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&policy);
        let mut hits: Vec<(String, i64)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["file"].as_str().or(d["path"].as_str()).unwrap_or("").rsplit('/').next().unwrap_or("").to_owned(), d["line"].as_i64().or(d["start_line"].as_i64()).unwrap_or(-1)))
            .collect();
        hits.sort();
        assert_eq!(hits.iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["x.py", "y.py"], "{report:#}");
    }
}
