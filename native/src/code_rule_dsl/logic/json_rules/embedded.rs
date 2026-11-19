//! Main's terms over an embedded program (decision 0222): a clause's `ast.embedded`, the `embedded` term, and `host`
//! read over a program's node.
//!
//! A file that embeds a program (a Vue component's view: its scripts and template expressions as one ECMAScript
//! program) has it as a guest document (`vue/view.rerail` asks for it; `corpus::guests` builds it with main's view
//! builder). The program is parsed in the dialect its file chose (a `<script lang="ts">`'s TypeScript), so a rule over
//! it is written once per dialect the embedding may give, as main compiles an `embedded` rule once per dialect: each
//! copy enters the program through `guest` with its dialect named, and its kinds are that grammar's (decision 0058
//! §2).
//!
//! ```text
//! // ast: { language: vue, embedded: script-block, rule: { kind: arrow_function } }
//! f(n) :- docof(n, g), (guest(g, _, _, "typescript") ; guest(g, _, _, "tsx") ; guest(g, _, _, "javascript")),
//!     kind(n, "arrow_function").
//! // { host: { kind: template_element } } on a program node x: the construct x stands for
//! host_node(x, h), kind(h, "template_element")
//! // { embedded: R } on a file node v: a program node standing for bytes inside v matches R
//! embedded_1(n) :- host_node(y, h), (h = n ; within(h, n)), docof(y, g), guest(g, _, _, "typescript"), R(y).
//! ```

use super::lower::{R, lower};
use super::patterns::Patterns;
use super::program::{Program, Vars};
use super::value::Value;

/// The dialects a program embedded through `via` in a `lang` file may be parsed as, by label, in registry order.
pub(super) fn dialects(lang: &str, via: &str) -> Vec<&'static str> {
    use crate::code_rule_dsl::language::{dialect_by_label, label};
    let Some(host) = dialect_by_label(lang) else { return Vec::new() };
    let mut out: Vec<&'static str> = Vec::new();
    for &(v, d) in host.embeds() {
        if v.id() == via
            && let Some(l) = label(d)
            && !out.contains(&l)
        {
            out.push(l);
        }
    }
    out
}

/// The one embedding a `lang` file has, by its registry id: what an `embedded` term reads.
pub(super) fn only_via(lang: &str) -> Option<&'static str> {
    let host = crate::code_rule_dsl::language::dialect_by_label(lang)?;
    let mut vias = host.embeds().iter().map(|(v, _)| v.id());
    let first = vias.next()?;
    vias.all(|v| v == first).then_some(first)
}

/// The embedding clause `c` reads: its `ast.embedded`, or the file's one embedding where an `embedded` term is
/// in it (its utils' `host` reads that program's nodes too).
pub(super) fn clause_via(lang: &str, c: &Value) -> Option<String> {
    let ast = c.get("forbid").and_then(|f| f.get("ast"));
    if let Some(via) = ast.and_then(|a| a.get("embedded")).and_then(Value::as_str) {
        return Some(via.to_owned());
    }
    ast.filter(|a| has_key(a, "embedded")).and_then(|_| only_via(lang)).map(str::to_owned)
}

/// The embedding some clause of a policy reads (`clause_via`).
pub(super) fn reads_program(lang: &str, clauses: &[Value]) -> Option<String> {
    clauses.iter().find_map(|c| clause_via(lang, c))
}

/// Whether `key` is a key of `v` or of an object inside it.
fn has_key(v: &Value, key: &str) -> bool {
    match v {
        Value::Obj(o) => o.iter().any(|(k, x)| k == key || has_key(x, key)),
        Value::List(l) => l.iter().any(|x| has_key(x, key)),
        _ => false,
    }
}

/// The literals that put node `v` in the program the file embeds through `via`, in one of its dialects: a
/// disjunction, one rule per dialect once read.
pub(super) fn entry(lang: &str, via: &str, v: &str, vs: &mut Vars) -> R<String> {
    let labels = dialects(lang, via);
    if labels.is_empty() {
        return Err(format!("a {lang} file embeds no program through `{via}`"));
    }
    let g = vs.fresh("g");
    let guests: Vec<String> = labels.iter().map(|l| format!("guest({g}, _, _, \"{l}\")")).collect();
    Ok(format!("docof({v}, {g}), ({})", guests.join(" ; ")))
}

/// `host` over program node `v`: the innermost file node over the bytes v stands for, matching `rule`.
pub(super) fn host(px: &Patterns, prog: &mut Program, rule: &Value, v: &str, vs: &mut Vars) -> R<Vec<String>> {
    let h = vs.fresh("h");
    let mut lits = vec![format!("host_node({v}, {h})")];
    lits.extend(lower(px, prog, rule, &h, vs, &[])?);
    Ok(lits)
}

/// The `embedded` term on file node `v`: some node of the file's program, standing for bytes inside v, matches
/// `rule`. A helper relation, one rule per dialect.
pub(super) fn term(px: &Patterns, prog: &mut Program, rule: &Value, v: &str) -> R<Vec<String>> {
    let lang = prog.lang.clone();
    let Some(via) = prog.embedded.clone().or_else(|| only_via(&lang).map(str::to_owned)) else {
        return Err(format!("a {lang} file embeds no program for `embedded` to read"));
    };
    let labels = dialects(&lang, &via);
    let outer = prog.embedded.replace(via);
    let name = prog.helper("embedded", "the JSON `embedded`: a node of the file's program, standing for bytes inside n, matches its rule");
    for l in labels {
        let mut vs = Vars::default();
        let (y, h, g) = (vs.fresh("y"), vs.fresh("h"), vs.fresh("g"));
        let mut lits = vec![format!("host_node({y}, {h})"), format!("({h} = n ; within({h}, n))"), format!("docof({y}, {g})"), format!("guest({g}, _, _, \"{l}\")")];
        lits.extend(lower(px, prog, rule, &y, &mut vs, &[])?);
        prog.add_rule(format!("{name}(n)"), lits);
    }
    prog.embedded = outer;
    Ok(vec![format!("{name}({v})")])
}
