//! Lint rules' syntax: `rerail.policy/v1` documents in YAML or JSON, and the same documents in the logic AST
//! (03 §12 B12, "one AST, two concrete syntaxes that round-trip losslessly"; 05 Phase 6 row 3).
//!
//! Each `forbid` clause's matcher becomes a rule whose body is the matcher atom,
//!
//! ```text
//! violation("<policy>", "<clause>", D, N) :- ast(D, N, {"language": …, "rule": …}).
//! ```
//!
//! and the rest of the document (its header, `define` clauses, each clause's `violation` block) is kept as a
//! `.policy { … }` directive, its forbid clauses holding `"forbid": "ast"` where the matcher was. The matcher
//! stays opaque here: W's loader (`compile/*`) validates it and §3 lowers it to a transducer program; `define`
//! clauses' shared `utils` and options are the matcher's too. Rules carry no SQL.

use super::Error;
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::{Builder, Lit, T, TermRef, Version};
use serde_json::Value;
use std::sync::Arc;

const HEAD: &str = "violation";
pub const MARK: &str = "ast";

/// A document's text, YAML or JSON (JSON is YAML).
pub fn read(text: &str) -> Result<Value, Error> {
    serde_yaml_ng::from_str(text).map_err(|e| Error::Syntax(format!("bad policy document: {e}")))
}

fn str_field<'a>(v: &'a Value, key: &str, what: &str) -> Result<&'a str, Error> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| Error::Syntax(format!("{what} has no string `{key}`")))
}

/// The program of policy documents `docs`: each forbid matcher a rule into `violation`, each document's rest a
/// `.policy`, in order.
pub fn program(docs: &[Value]) -> Result<Arc<Version>, Error> {
    let bad = |e: String| Error::Syntax(e);
    let mut b = Builder::base();
    let mut headers = Vec::new();
    for doc in docs {
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
            let args = [T::Str(&policy), T::Str(id), T::Var("D"), T::Var("N")];
            b.rule().head(HEAD, &args).ast(T::Var("D"), T::Var("N"), &matcher.to_string(), &[]).finish().map_err(bad)?;
            clause["forbid"] = Value::String(MARK.to_owned());
        }
        headers.push(header);
    }
    b.fields().declare(HEAD, &[("policy", "sym"), ("clause", "sym"), ("d", "int"), ("n", "node")]);
    b.fields().set_outputs(&[HEAD]);
    if !headers.is_empty() {
        b.fields().set_field(Field::Policies(&headers));
    }
    b.finish().map(|(v, _)| v).map_err(bad)
}

/// The policy documents `v` holds, each with its matchers back from its `violation` rules.
pub fn documents(v: &Version) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    let text = |t: TermRef<'_>| match t {
        TermRef::Str(a) => Some(v.text(a)),
        _ => None,
    };
    for header in v.policies().map_err(Error::Syntax)? {
        let mut doc = header.clone();
        let policy = str_field(&header, "id", "a policy")?.to_owned();
        for clause in doc.get_mut("clauses").and_then(Value::as_array_mut).into_iter().flatten() {
            if clause.get("forbid").and_then(Value::as_str) != Some(MARK) {
                continue;
            }
            let id = str_field(clause, "id", "a clause")?.to_owned();
            let matcher = v
                .rules()
                .filter(|r| {
                    let mut args = r.head().args();
                    v.rel_name(r.head().rel()) == HEAD && args.next().and_then(text) == Some(policy.as_str()) && args.next().and_then(text) == Some(id.as_str())
                })
                .find_map(|r| {
                    let mut body = r.body();
                    match (body.next().map(|l| l.view()), body.next()) {
                        (Some(Lit::Ast { matcher, .. }), None) => Some(v.text(matcher)),
                        _ => None,
                    }
                })
                .ok_or_else(|| Error::Syntax(format!("policy {policy}: no matcher rule for clause {id}")))?;
            let matcher: Value = serde_json::from_str(matcher).map_err(|e| Error::Syntax(format!("policy {policy} clause {id}: {e}")))?;
            clause["forbid"] = serde_json::json!({ MARK: matcher });
        }
        out.push(doc);
    }
    Ok(out)
}

/// The `.rerail` text of a program of policies: its declarations, outputs, `.policy` documents and rules.
pub fn text(v: &Arc<Version>) -> String {
    crate::code_rule_dsl::logic::rr::print_version(v).expect("matcher rules print as .rerail")
}

// cratify: `tests` is rerail-logic's (it mounts it)
