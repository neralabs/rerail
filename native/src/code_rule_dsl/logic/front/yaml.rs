//! Lint rules' syntax: `rerail.policy/v1` documents in YAML or JSON, and the same documents in the logic AST
//! (03 §12 B12, "one AST, two concrete syntaxes that round-trip losslessly"; 05 Phase 6 row 3).
//!
//! Each `forbid` clause's matcher becomes a rule whose body is the matcher atom,
//!
//! ```text
//! violation("<policy>", "<clause>", $D, $N) :- ast($D, $N, {"language": …, "rule": …}).
//! ```
//!
//! and the rest of the document (its header, `define` clauses, each clause's `violation` block) is kept as a
//! `.policy { … }` directive, its forbid clauses holding `"forbid": "ast"` where the matcher was. The matcher
//! stays opaque here: W's loader (`compile/*`) validates it and §3 lowers it to a transducer program; `define`
//! clauses' shared `utils` and options are the matcher's too. Rules carry no SQL.

use super::{Atom, Error, Literal, Program, Rule, Term};
use serde_json::Value;

const HEAD: &str = "violation";
const MARK: &str = "ast";

/// A document's text, YAML or JSON (JSON is YAML).
pub(in crate::code_rule_dsl) fn read(text: &str) -> Result<Value, Error> {
    serde_yaml_ng::from_str(text).map_err(|e| Error::Syntax(format!("bad policy document: {e}")))
}

fn str_field<'a>(v: &'a Value, key: &str, what: &str) -> Result<&'a str, Error> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| Error::Syntax(format!("{what} has no string `{key}`")))
}

/// Adds a policy document to `prog`: its forbid matchers as rules into `violation`, the rest as a `.policy`.
pub(in crate::code_rule_dsl) fn add(prog: &mut Program, doc: &Value) -> Result<(), Error> {
    let policy = str_field(doc, "id", "a policy")?.to_owned();
    let mut header = doc.clone();
    let clauses = header
        .get_mut("clauses")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error::Syntax(format!("policy {policy} has no clauses")))?;
    for clause in clauses {
        let Some(forbid) = clause.get("forbid") else { continue };
        let Some(matcher) = forbid.as_object().filter(|f| f.len() == 1).and_then(|f| f.get(MARK)) else {
            return Err(Error::Syntax(format!("policy {policy}: a forbid clause holds one `ast` matcher")));
        };
        let id = str_field(clause, "id", "a clause")?;
        let args = vec![Term::Str(policy.clone()), Term::Str(id.to_owned()), Term::Var("D".to_owned()), Term::Var("N".to_owned())];
        let body = vec![Literal::Ast { document: "D".to_owned(), node: "N".to_owned(), matcher: matcher.to_string() }];
        prog.rules.push(Rule { head: Atom { rel: HEAD.to_owned(), args, neg: false, gated: false }, body });
        clause["forbid"] = Value::String(MARK.to_owned());
    }
    prog.decls.entry(HEAD.to_owned()).or_insert_with(|| {
        [("policy", "sym"), ("clause", "sym"), ("d", "int"), ("n", "node")].map(|(c, t)| (c.to_owned(), t.to_owned())).to_vec()
    });
    if !prog.outputs.iter().any(|o| o == HEAD) {
        prog.outputs.push(HEAD.to_owned());
    }
    prog.policies.push(header);
    Ok(())
}

/// The policy documents `prog` holds, each with its matchers back from its `violation` rules.
pub(in crate::code_rule_dsl) fn documents(prog: &Program) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    for header in &prog.policies {
        let mut doc = header.clone();
        let policy = str_field(header, "id", "a policy")?.to_owned();
        for clause in doc.get_mut("clauses").and_then(Value::as_array_mut).into_iter().flatten() {
            if clause.get("forbid").and_then(Value::as_str) != Some(MARK) {
                continue;
            }
            let id = str_field(clause, "id", "a clause")?.to_owned();
            let matcher = prog
                .rules
                .iter()
                .filter(|r| r.head.rel == HEAD && r.head.args.get(..2) == Some(&[Term::Str(policy.clone()), Term::Str(id.clone())]))
                .find_map(|r| match r.body.as_slice() {
                    [Literal::Ast { matcher, .. }] => Some(matcher),
                    _ => None,
                })
                .ok_or_else(|| Error::Syntax(format!("policy {policy}: no matcher rule for clause {id}")))?;
            let matcher: Value = serde_json::from_str(matcher).map_err(|e| Error::Syntax(format!("policy {policy} clause {id}: {e}")))?;
            clause["forbid"] = serde_json::json!({ MARK: matcher });
        }
        out.push(doc);
    }
    Ok(out)
}

/// The Datalog text of a program of policies: its declarations, outputs, `.policy` documents and rules.
pub(in crate::code_rule_dsl) fn text(prog: &Program) -> String {
    let mut out = String::new();
    for (rel, cols) in &prog.decls {
        let cols: Vec<String> = cols.iter().map(|(c, t)| format!("{c}: {t}")).collect();
        out += &format!(".decl {rel}({})\n", cols.join(", "));
    }
    if !prog.outputs.is_empty() {
        out += &format!(".output {}\n", prog.outputs.join(", "));
    }
    for p in &prog.policies {
        out += &format!(".policy {p}\n");
    }
    for r in &prog.rules {
        out += &format!("{r}.\n");
    }
    out
}

#[cfg(test)]
mod tests;
