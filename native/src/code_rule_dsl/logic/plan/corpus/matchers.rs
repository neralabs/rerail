//! A program's `ast(d, n, { … })` atoms, run by the transducer's matchers (decision 0106).
//!
//! Each atom becomes an input relation, one per distinct matcher and captured names: `rr_ast_<k>(n)` without
//! captures, `rr_ast_<k>_<m>(n, x1, …, xm)` with `m` (decision 0107), and `docof(n, d)` where the rule reads `d`
//! elsewhere. Each chunk fills the relations by running the matchers' wave over its documents' tapes
//! (`facts::vtab::syntax::with_matchers`); a captured matcher's rows read W's bindings at each matched root. The
//! joins read the matches as any base relation.
use crate::code_rule_dsl::facts::vtab::syntax::Matcher;
use crate::code_rule_dsl::logic::front::{Atom, Literal, Program, Term};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

/// tree-sitter-python's grammar (`grammar.json`), parsed once.
fn python_grammar() -> Option<&'static serde_json::Value> {
    static GRAMMAR: std::sync::LazyLock<Option<serde_json::Value>> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../../../../grammars/tree-sitter-python/src/grammar.json")).ok());
    GRAMMAR.as_ref()
}

/// The named kinds a grammar expression's own nodes can be: its symbols, a hidden, inlined or supertype rule read
/// through, named aliases. `inside`: every named node under it as well, not only its own.
fn named_kinds(grammar: &serde_json::Value, e: &serde_json::Value, inside: bool, through: &mut BTreeSet<String>, out: &mut BTreeSet<String>) {
    let names = |key: &str| grammar[key].as_array().into_iter().flatten().any(|v| v.as_str() == e["name"].as_str());
    match e["type"].as_str() {
        Some("SYMBOL") => {
            let Some(n) = e["name"].as_str() else { return };
            if n.starts_with('_') || names("inline") || names("supertypes") {
                if through.insert(n.to_owned()) && let Some(rule) = grammar["rules"].get(n) {
                    named_kinds(grammar, rule, inside, through, out);
                }
            } else {
                out.insert(n.to_owned());
                if inside && through.insert(n.to_owned()) && let Some(rule) = grammar["rules"].get(n) {
                    named_kinds(grammar, rule, inside, through, out);
                }
            }
        }
        Some("ALIAS") => {
            if e["named"] == true && let Some(v) = e["value"].as_str() {
                out.insert(v.to_owned());
            }
            if inside {
                named_kinds(grammar, &e["content"], inside, through, out);
            }
        }
        _ => {
            if let Some(c) = e.get("content") {
                named_kinds(grammar, c, inside, through, out);
            }
            for m in e["members"].as_array().into_iter().flatten() {
                named_kinds(grammar, m, inside, through, out);
            }
        }
    }
}

/// The kinds a node in field `field` can be, by tree-sitter-python's grammar, whatever its parent: every rule's
/// `field`'s content's own named nodes. `None` without the grammar, or for a field it does not name.
pub(in crate::code_rule_dsl) fn field_kinds(field: &str) -> Option<BTreeSet<String>> {
    static KEPT: std::sync::LazyLock<Mutex<HashMap<String, Option<BTreeSet<String>>>>> = std::sync::LazyLock::new(Default::default);
    if let Some(k) = KEPT.lock().unwrap_or_else(|p| p.into_inner()).get(field) {
        return k.clone();
    }
    let grammar = python_grammar()?;
    fn fields(grammar: &serde_json::Value, e: &serde_json::Value, field: &str, found: &mut bool, out: &mut BTreeSet<String>) {
        if e["type"] == "FIELD" && e["name"] == field {
            *found = true;
            named_kinds(grammar, &e["content"], false, &mut BTreeSet::new(), out);
        }
        if let Some(c) = e.get("content") {
            fields(grammar, c, field, found, out);
        }
        for m in e["members"].as_array().into_iter().flatten() {
            fields(grammar, m, field, found, out);
        }
    }
    let (mut found, mut out) = (false, BTreeSet::new());
    for (_, rule) in grammar["rules"].as_object().into_iter().flatten() {
        fields(grammar, rule, field, &mut found, &mut out);
    }
    let kinds = found.then_some(out);
    KEPT.lock().unwrap_or_else(|p| p.into_inner()).insert(field.to_owned(), kinds.clone());
    kinds
}

/// Whether a node of kind `kind` has no named node under it, by tree-sitter-python's grammar (a token, an external,
/// a rule of anonymous tokens only): its text is a token's. `false` without the grammar.
pub(in crate::code_rule_dsl) fn leaf_kind(kind: &str) -> bool {
    let Some(grammar) = python_grammar() else { return false };
    let Some(rule) = grammar["rules"].get(kind) else { return true };
    let mut out = BTreeSet::new();
    named_kinds(grammar, rule, false, &mut BTreeSet::new(), &mut out);
    out.is_empty()
}

/// Whether a node of kind `kind` holds at most one node in field `field`, by tree-sitter-python's grammar: the field
/// once in the kind's rule (its hidden and inlined rules read through), under no repeat. A kind some rule aliases a
/// node to, or a rule that reads itself, is taken to hold several (so its parts stay joins).
pub(in crate::code_rule_dsl) fn single_field(kind: &str, field: &str) -> bool {
    use serde_json::Value;
    static GRAMMAR: std::sync::LazyLock<Option<Value>> = std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../../../../grammars/tree-sitter-python/src/grammar.json")).ok());
    static KEPT: std::sync::LazyLock<Mutex<HashMap<(String, String), bool>>> = std::sync::LazyLock::new(Default::default);
    let Some(grammar) = GRAMMAR.as_ref() else { return false };
    if let Some(&kept) = KEPT.lock().unwrap_or_else(|p| p.into_inner()).get(&(kind.to_owned(), field.to_owned())) {
        return kept;
    }
    let rules = &grammar["rules"];
    let names = |key: &str| -> BTreeSet<String> { grammar[key].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect() };
    let (inline, supertypes) = (names("inline"), names("supertypes"));
    fn aliased(e: &Value, out: &mut BTreeSet<String>) {
        if e["type"] == "ALIAS" && e["named"] == true {
            if let Some(v) = e["value"].as_str() {
                out.insert(v.to_owned());
            }
        }
        if let Some(c) = e.get("content") {
            aliased(c, out);
        }
        e["members"].as_array().into_iter().flatten().for_each(|m| aliased(m, out));
    }
    let mut aliases = BTreeSet::new();
    rules.as_object().into_iter().flatten().for_each(|(_, r)| aliased(r, &mut aliases));
    // At most `MANY`: 0, 1 or several of the field.
    const MANY: u8 = 2;
    fn count(e: &Value, field: &str, rules: &Value, inline: &BTreeSet<String>, supertypes: &BTreeSet<String>, seen: &mut Vec<String>) -> u8 {
        let mut c = |e: &Value, seen: &mut Vec<String>| count(e, field, rules, inline, supertypes, seen);
        match e["type"].as_str().unwrap_or("") {
            "FIELD" => (u8::from(e["name"] == field) + c(&e["content"], seen)).min(MANY),
            "SEQ" => e["members"].as_array().into_iter().flatten().map(|m| c(m, seen)).fold(0, |a, b| (a + b).min(MANY)),
            "CHOICE" => e["members"].as_array().into_iter().flatten().map(|m| c(m, seen)).max().unwrap_or(0),
            "REPEAT" | "REPEAT1" => {
                if c(&e["content"], seen) > 0 { MANY } else { 0 }
            }
            "ALIAS" if e["named"] == true => 0,
            "SYMBOL" => {
                let name = e["name"].as_str().unwrap_or("");
                if supertypes.contains(name) || !(name.starts_with('_') || inline.contains(name)) || rules.get(name).is_none() {
                    return 0;
                }
                if seen.iter().any(|s| s == name) {
                    return MANY;
                }
                seen.push(name.to_owned());
                let n = c(&rules[name], seen);
                seen.pop();
                n
            }
            _ => e.get("content").map_or(0, |content| c(content, seen)),
        }
    }
    let single = !kind.starts_with('_') && !aliases.contains(kind) && rules.get(kind).is_some_and(|rule| count(rule, field, rules, &inline, &supertypes, &mut Vec::new()) <= 1);
    KEPT.lock().unwrap_or_else(|p| p.into_inner()).insert((kind.to_owned(), field.to_owned()), single);
    single
}

/// The prefix of a matcher's relation.
pub(in crate::code_rule_dsl) const PREFIX: &str = "rr_ast_";

/// A matcher relation's columns: the root `n`, then one a capture.
const COLUMNS: [&str; 16] = ["n", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15"];

/// The most captures an atom may name.
const MOST_CAPTURES: usize = COLUMNS.len() - 1;

/// The columns of matcher relation `rel` (`rr_ast_<k>` or `rr_ast_<k>_<m>`).
pub(in crate::code_rule_dsl) fn columns(rel: &str) -> &'static [&'static str] {
    let width = rel.strip_prefix(PREFIX).and_then(|rest| rest.split_once('_')).and_then(|(_, m)| m.parse::<usize>().ok()).unwrap_or(0);
    &COLUMNS[..=width.min(MOST_CAPTURES)]
}

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

/// The process's lowered matchers, by their text and their base policy's: lowered once.
type Lowered = Result<(Arc<crate::code_rule_dsl::transducer::packed::LoweredMatcher>, Vec<crate::code_rule_dsl::matcher::DocumentGate>, Arc<crate::code_rule_dsl::waves::policies::Captor>), String>;
static LOWERED: std::sync::LazyLock<Mutex<HashMap<(String, String), Lowered>>> = std::sync::LazyLock::new(Default::default);

/// Each relation's matcher lowered for the transducer, over the program's first `.policy` (its utils). A matcher that
/// does not lower fails the run, naming it.
pub(in crate::code_rule_dsl) fn compiled(matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<crate::code_rule_dsl::facts::vtab::syntax::Matchers, String> {
    let base = policies.first();
    let base_text = base.map(|b| b.to_string()).unwrap_or_default();
    let _span = crate::code_rule_dsl::logic::profile::span(|| "ast matchers lowered".into());
    matchers
        .iter()
        .map(|(rel, text, captures)| {
            let key = (text.clone(), base_text.clone());
            let got = {
                let mut kept = LOWERED.lock().unwrap_or_else(|p| p.into_inner());
                kept.entry(key)
                    .or_insert_with(|| crate::code_rule_dsl::waves::policies::lower_matcher(text, base).map(|(m, g, c)| (Arc::new(m), g, Arc::new(c))))
                    .clone()
            };
            let (lowered, gates, captor) = got.map_err(|why| format!("{rel}: the matcher {text} does not lower: {why}"))?;
            Ok(Matcher { rel: rel.clone(), lowered, gates, captures: captures.clone(), captor })
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
}
