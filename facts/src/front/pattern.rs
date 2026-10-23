//! Pattern front end (INVARIANTS I3): a resolved `ast` matcher query lowered to the algebra over
//! one content's core relations (`ir::CoreRel`: `file`, `node`, `string_value`,
//! `string_value_unknown`, `syntax_error`). The lowering replicates the leaf executor's
//! semantics (Spike A `ast.rs`, exact against native on both corpora) construct by construct;
//! anything else is refused with the construct named (I9).
//!
//! Semantics replicated:
//!
//! - **Candidates.** Every node in preorder (`node` order), restricted to the query's root
//!   kinds when the rule determines them (`ast.rs::root_kinds`); one row per matching root.
//! - **Verdicts are three-valued** (Kleene, SQL's): a `value` test on a literal whose value is
//!   unknown is NULL; `all`/`any`/`not` combine as `AND`/`OR`/`NOT`; a relation search
//!   (`has`/`inside`) is TRUE if a candidate is TRUE, NULL if none is and one is NULL, else
//!   FALSE (capture-free inner rules only, so the executor never stops a search early on an
//!   unknown). A root is kept iff its verdict is TRUE (the filter).
//! - **Relations** over the preorder interval property (I2): `has` neighbour = the children (all
//!   of them, named or not), `has … stopBy: end` = `(id, subtree_end)`, `inside` neighbour = the
//!   parent, `inside … stopBy: end` = the ancestors (`a < id < subtree_end(a)`); `has` with a
//!   `field` = the first child under the field (neighbour), or it and its descendants (`end`);
//!   `inside` with a `field` = an ancestor the walk entered through its first child under the
//!   field. A stop rule `S` (two-valued) cuts the walk: a candidate is reached iff no node
//!   strictly between it and the anchor satisfies `S` (`ast.rs` tests the candidate, then the stop).
//! - **Captures** bound on the root chain: the root (`$X` conjuncts), and a relation search
//!   binding one metavariable (`any[REL{… $X …}, $_]`, optional, or `REL{… $X …}`): the first
//!   candidate in search order (preorder for `has`, nearest first for `inside`) where the inner
//!   rule holds, as a key join `LEFT JOIN node x ON x.id = (first candidate)`.
//! - **Run-time refusals** (the content fails, as the executor's `unsupported`): a `value` test
//!   or a `DECODED` projection that decodes a literal holding a `\N{...}` escape, counted only
//!   where the executor evaluates it (the reach of a conjunct in an `all` chain: every earlier
//!   conjunct not FALSE); a query without `syntax` over a content with a syntax error.
use crate::ir::{Anchor, ArithOp, CmpOp, Column, CoreRel, Expr, FileSource, Func, InputDef, InputKind, JoinKind, Lit, Lowered, Projection, Rel, Total};
use crate::sql::value::Aff;
use serde_json::{Map, Value as Json};
use std::collections::BTreeMap;

type E = Expr<FileSource>;
type R = Rel<FileSource>;


/// The executor's message for a `\N{...}` escape decoded during an input's evaluation.
pub const NAMED_ESCAPE: &str = "`\\N{...}` escape (needs the Unicode name table)";
pub const NO_SYNTAX: &str = "query without `syntax` on a file with syntax errors";

const RULE_KEYS: &[&str] = &[
    "pattern", "kind", "regex", "capture", "parse", "nthChild", "filePath", "all", "any", "not", "matches", "inside", "has", "count", "compare", "value", "stopBy", "field",
];
const PY_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];
const STRING_KINDS: &[&str] = &["concatenated_string", "string"];

// The candidate row: `file` ⋈ `node`.
const PATH: u16 = 0;
const ID: u16 = 1;
const KIND: u16 = 2;
const PARENT: u16 = 3;
const START: u16 = 4;
const END: u16 = 5;
const SUBTREE_END: u16 = 8;
const NAMED: u16 = 9;
const FIELD: u16 = 11;
// `node` alone (no path column first).
const N_ID: u16 = 0;
const N_PARENT: u16 = 2;
const N_KIND: u16 = 1;
const N_START: u16 = 3;
const N_END: u16 = 4;
const N_START_LINE: u16 = 5;
const N_SUBTREE_END: u16 = 7;
const N_FIELD: u16 = 10;

/// The matcher rule, as `ast.rs` compiles it (term order and `Never` folding kept).
#[derive(Clone, Debug)]
enum Rule {
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Not(Box<Rule>),
    Kind(String),
    Regex(String),
    /// `$X` (named) / `$$X`: binds the node; `named` requires a named node.
    Meta { name: Option<String>, named: bool },
    Literal { kind: String, text: String },
    Rel { has: bool, field: Option<String>, stop: Stop, rule: Box<Rule> },
    FilePath { regex: Option<String>, not_regex: Option<String> },
    Value { eq: Option<String>, regex: Option<String> },
    /// `matches: id`, inlined (kept as a node for `root_kinds`' shared visited set).
    Util(String, Box<Rule>),
    Never,
}

/// A relation's `stopBy`.
#[derive(Clone, Debug)]
enum Stop {
    Neighbor,
    End,
    Rule(Box<Rule>),
}

struct Front<'a> {
    utils: &'a BTreeMap<String, Json>,
    known_field: &'a dyn Fn(&str) -> Option<bool>,
    stack: Vec<String>,
    /// Field names whose existence in the grammar decides a `Never` fold (see `lower_input`).
    fields: Vec<String>,
}

fn obj<'j>(v: &'j Json, at: &str) -> Result<&'j Map<String, Json>, String> {
    v.as_object().ok_or_else(|| format!("{at}: expected an object"))
}

fn valid_meta(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

impl Front<'_> {
    fn rule(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let r = obj(v, at)?;
        for k in r.keys() {
            if !RULE_KEYS.contains(&k.as_str()) {
                return Err(format!("{at}: rule key `{k}`"));
            }
        }
        if r.contains_key("stopBy") || r.contains_key("field") {
            return Err(format!("{at}: stopBy/field outside a relation"));
        }
        for k in ["capture", "parse", "nthChild", "count", "compare"] {
            if r.contains_key(k) {
                return Err(format!("{at}: `{k}` (not lowered to the algebra)"));
            }
        }
        let mut terms = Vec::new();
        if let Some(p) = r.get("pattern") {
            let p = p.as_str().ok_or_else(|| format!("{at}/pattern: contextual pattern"))?;
            terms.push(pattern(p.trim(), at)?);
        }
        if let Some(k) = r.get("kind").and_then(Json::as_str) {
            terms.push(Rule::Kind(k.to_owned()));
        }
        if let Some(rx) = r.get("regex").and_then(Json::as_str) {
            regex::Regex::new(rx).map_err(|e| format!("{at}/regex: {e}"))?;
            terms.push(Rule::Regex(rx.to_owned()));
        }
        if let Some(fp) = r.get("filePath") {
            let o = obj(fp, "filePath")?;
            let get = |k: &str| -> Result<Option<String>, String> {
                o.get(k)
                    .map(|v| {
                        let s = v.as_str().unwrap_or("");
                        regex::Regex::new(s).map(|_| s.to_owned()).map_err(|e| e.to_string())
                    })
                    .transpose()
            };
            terms.push(Rule::FilePath { regex: get("regex")?, not_regex: get("notRegex")? });
        }
        for key in ["all", "any"] {
            let Some(items) = r.get(key).and_then(Json::as_array) else { continue };
            let compiled = items.iter().enumerate().map(|(i, it)| self.rule(it, &format!("{at}/{key}/{i}"))).collect::<Result<Vec<_>, _>>()?;
            terms.push(if key == "all" {
                if compiled.iter().any(|c| matches!(c, Rule::Never)) { Rule::Never } else { Rule::All(compiled) }
            } else {
                let kept: Vec<_> = compiled.into_iter().filter(|c| !matches!(c, Rule::Never)).collect();
                if kept.is_empty() { Rule::Never } else { Rule::Any(kept) }
            });
        }
        if let Some(n) = r.get("not") {
            terms.push(Rule::Not(Box::new(self.rule(n, &format!("{at}/not"))?)));
        }
        if let Some(m) = r.get("matches") {
            let id = m.as_str().ok_or_else(|| format!("{at}/matches: parameterised `matches`"))?;
            let u = self.utils.get(id).ok_or_else(|| format!("{at}/matches: unknown utility {id}"))?;
            if self.stack.iter().any(|s| s == id) {
                return Err(format!("{at}/matches: recursive utility {id}"));
            }
            self.stack.push(id.to_owned());
            let inner = self.rule(u, &format!("utils/{id}"))?;
            self.stack.pop();
            terms.push(Rule::Util(id.to_owned(), Box::new(inner)));
        }
        for key in ["inside", "has"] {
            if let Some(rel) = r.get(key) {
                terms.push(self.relation(rel, key == "has", &format!("{at}/{key}"))?);
            }
        }
        if let Some(v) = r.get("value") {
            let o = obj(v, &format!("{at}/value"))?;
            for k in o.keys() {
                if !["eq", "regex"].contains(&k.as_str()) {
                    return Err(format!("{at}/value: key `{k}`"));
                }
            }
            let regex = o.get("regex").and_then(Json::as_str).map(str::to_owned);
            if let Some(rx) = &regex {
                regex::Regex::new(rx).map_err(|e| format!("{at}/value/regex: {e}"))?;
            }
            terms.push(Rule::Value { eq: o.get("eq").and_then(Json::as_str).map(str::to_owned), regex });
        }
        if terms.is_empty() {
            return Err(format!("{at}: empty rule"));
        }
        // `ast.rs` reorders a group only when a term reads a sibling's capture; captures are
        // root-chain metavariables here, never read by a sibling, so the order is as written.
        Ok(if terms.len() == 1 { terms.pop().unwrap() } else { Rule::All(terms) })
    }

    fn relation(&mut self, rel: &Json, has: bool, at: &str) -> Result<Rule, String> {
        let o = obj(rel, at)?;
        // `ast.rs::Compiler::stop` compiles the stop rule before the relation's own rule.
        let stop = match o.get("stopBy") {
            None => Stop::Neighbor,
            Some(Json::String(s)) if s == "neighbor" => Stop::Neighbor,
            Some(Json::String(s)) if s == "end" => Stop::End,
            Some(v @ Json::Object(_)) => Stop::Rule(Box::new(self.rule(v, &format!("{at}/stopBy"))?)),
            Some(_) => return Err(format!("{at}/stopBy")),
        };
        let field = o.get("field").and_then(Json::as_str).map(str::to_owned);
        let mut nested = o.clone();
        nested.remove("stopBy");
        nested.remove("field");
        let inner = self.rule(&Json::Object(nested), at)?;
        if matches!(inner, Rule::Never) {
            return Ok(Rule::Never);
        }
        if let Some(f) = &field {
            match (self.known_field)(f) {
                Some(false) => return Ok(Rule::Never),
                Some(true) => {}
                None => self.fields.push(f.clone()),
            }
        }
        if let Stop::Rule(s) = &stop {
            // The walk ends at an unknown stop verdict (`search_open`); a binding stop rule binds
            // into a fresh environment. Neither is lowered.
            if field.is_some() {
                return Err(format!("{at}: `field` with a stop rule"));
            }
            if reads_values(s) || reads_values(&inner) {
                return Err(format!("{at}: a `value` test under a stop-rule relation"));
            }
            if binds(s) {
                return Err(format!("{at}/stopBy: a stop rule binding a metavariable"));
            }
        }
        Ok(Rule::Rel { has, field, stop, rule: Box::new(inner) })
    }
}

fn pattern(p: &str, at: &str) -> Result<Rule, String> {
    if let Some(rest) = p.strip_prefix("$$") {
        if rest.starts_with('$') || !valid_meta(rest) {
            return Err(format!("{at}: pattern {p}"));
        }
        return Ok(Rule::Meta { name: (rest != "_").then(|| rest.to_owned()), named: false });
    }
    if let Some(rest) = p.strip_prefix('$') {
        if !valid_meta(rest) {
            return Err(format!("{at}: pattern {p}"));
        }
        return Ok(Rule::Meta { name: (rest != "_").then(|| rest.to_owned()), named: true });
    }
    let lit = |kind: &str| Ok(Rule::Literal { kind: kind.into(), text: p.into() });
    match p {
        "True" => lit("true"),
        "False" => lit("false"),
        "None" => lit("none"),
        _ if p.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !PY_KEYWORDS.contains(&p) => lit("identifier"),
        _ => Err(format!("{at}: pattern `{p}` (beyond a metavariable or a single name)")),
    }
}

/// `ast.rs::root_kinds`, with its visited set shared across the walk.
fn root_kinds(r: &Rule, seen: &mut Vec<String>) -> Option<Vec<String>> {
    match r {
        Rule::Kind(k) => Some(vec![k.clone()]),
        Rule::Literal { kind, .. } => Some(vec![kind.clone()]),
        Rule::All(items) => items.iter().find_map(|i| root_kinds(i, seen)),
        Rule::Any(items) => {
            let mut all = Vec::new();
            for i in items {
                all.extend(root_kinds(i, seen)?);
            }
            all.sort();
            all.dedup();
            Some(all)
        }
        Rule::Util(id, inner) => {
            if seen.contains(id) {
                None
            } else {
                seen.push(id.clone());
                root_kinds(inner, seen)
            }
        }
        _ => None,
    }
}

/// Whether a rule reads literal values (so its verdict can be unknown).
fn reads_values(r: &Rule) -> bool {
    match r {
        Rule::Value { .. } => true,
        Rule::All(v) | Rule::Any(v) => v.iter().any(reads_values),
        Rule::Not(x) | Rule::Util(_, x) => reads_values(x),
        Rule::Rel { rule, stop, .. } => reads_values(rule) || matches!(stop, Stop::Rule(s) if reads_values(s)),
        _ => false,
    }
}

/// Whether a rule binds a metavariable.
fn binds(r: &Rule) -> bool {
    match r {
        Rule::Meta { name, .. } => name.is_some(),
        Rule::All(v) | Rule::Any(v) => v.iter().any(binds),
        Rule::Not(x) | Rule::Util(_, x) | Rule::Rel { rule: x, .. } => binds(x),
        _ => false,
    }
}

/// A root-chain capture by a relation search: the first candidate on the axis (search order)
/// where `inner` holds binds `name`.
struct Cap {
    name: String,
    has: bool,
    field: Option<String>,
    stop: Stop,
    inner: Rule,
}

/// `REL{X}` where `X` binds exactly one metavariable as a top-level `$NAME` (alone, or a
/// conjunct of an `all` whose other conjuncts bind nothing), two-valued, or `None`.
fn capture_of(r: &Rule) -> Option<Cap> {
    let Rule::Rel { has, field, stop, rule } = r else { return None };
    if reads_values(rule) || matches!(stop, Stop::Rule(s) if binds(s) || reads_values(s)) {
        return None;
    }
    let name = match &**rule {
        Rule::Meta { name: Some(n), .. } => n.clone(),
        Rule::All(items) => {
            let mut found = None;
            for it in items {
                match it {
                    Rule::Meta { name: Some(n), .. } if found.is_none() => found = Some(n.clone()),
                    other if binds(other) => return None,
                    _ => {}
                }
            }
            found?
        }
        _ => return None,
    };
    Some(Cap { name, has: *has, field: field.clone(), stop: stop.clone(), inner: (**rule).clone() })
}

/// The metavariables a root-chain (`all` nested in `all`, utilities inlined) binds to the root,
/// the relation searches on it that bind one metavariable (`caps`), and the value tests on that
/// chain with their reach (every earlier conjunct not FALSE).
fn root_chain(r: &Rule, bound: &mut Vec<String>, caps: &mut Vec<Cap>) -> Result<(), String> {
    match r {
        Rule::All(items) => {
            for it in items {
                root_chain(it, bound, caps)?;
            }
            Ok(())
        }
        Rule::Util(_, inner) => root_chain(inner, bound, caps),
        Rule::Meta { name: Some(n), .. } => {
            bound.push(n.clone());
            Ok(())
        }
        // `any[REL{… $X …}, $_]`: the search binds X when it finds a candidate; otherwise `$_`
        // (a named root) holds and nothing is bound. `REL{… $X …}` alone binds or fails.
        Rule::Any(items) if items.len() == 2 && matches!(items[1], Rule::Meta { name: None, named: true }) && capture_of(&items[0]).is_some() => {
            caps.push(capture_of(&items[0]).unwrap());
            Ok(())
        }
        Rule::Rel { .. } if binds(r) && capture_of(r).is_some() => {
            caps.push(capture_of(r).unwrap());
            Ok(())
        }
        other => {
            if binds(other) {
                return Err("a metavariable under `any`/`not`/a relation (captures are lowered on the root chain only)".into());
            }
            Ok(())
        }
    }
}

/// `verdict IS v` (TRUE/FALSE, never NULL).
fn is(e: E, v: i64, negated: bool) -> E {
    Expr::Is { left: Box::new(e), right: Box::new(int(v)), negated }
}

/// Whether a two-valued rule holds only on string literals (its root kinds are string kinds).
fn strings_only(r: &Rule) -> bool {
    !reads_values(r) && root_kinds(r, &mut Vec::new()).is_some_and(|k| !k.is_empty() && k.iter().all(|k| STRING_KINDS.contains(&k.as_str())))
}

/// The reach of every `value` test (the nodes the executor evaluates it at, which decide its
/// `\N{...}` refusals), as predicates over a `nodes()` row: `reach` is the reach of `r`. In an
/// `all` a conjunct runs where every earlier one is not FALSE, in an `any` where every earlier
/// one is not TRUE (capture-free: an unknown does not end it); a relation search tests its
/// candidates in search order until the first TRUE one. `strings`: the node is a string literal
/// wherever `r` runs (a test on any other expression decodes values the algebra does not hold).
fn value_reach(r: &Rule, reach: E, strings: bool, out: &mut Vec<E>) -> Result<(), String> {
    if !reads_values(r) {
        return Ok(());
    }
    match r {
        Rule::All(items) => {
            let (mut cond, mut strings) = (vec![reach], strings);
            for it in items {
                value_reach(it, all_of(cond.clone()), strings, out)?;
                strings |= strings_only(it);
                cond.push(is(verdict(it), 0, true));
            }
            Ok(())
        }
        Rule::Any(items) => {
            let mut cond = vec![reach];
            for it in items {
                value_reach(it, all_of(cond.clone()), strings, out)?;
                cond.push(is(verdict(it), 1, true));
            }
            Ok(())
        }
        Rule::Not(x) | Rule::Util(_, x) => value_reach(x, reach, strings, out),
        Rule::Value { .. } => {
            if !strings {
                return Err("a `value` test on nodes that are not only string literals".into());
            }
            out.push(reach);
            Ok(())
        }
        Rule::Rel { has, field, stop, rule } => {
            // A candidate `x` (depth 1) is tested when some anchor `a` (depth 0) reaches it: `a`
            // is reached, `x` is on `a`'s axis, and no candidate `d` of `a` before `x` holds.
            let (axis, s) = axis_of(*has, field.as_deref(), stop);
            let before = filter(
                nodes(),
                all_of(vec![
                    axis_pred(&axis, s.as_ref()),
                    cmp(if axis.upward() { CmpOp::Gt } else { CmpOp::Lt }, col(ID), up(2, ID)),
                    is(verdict(rule), 1, false),
                ]),
            );
            let anchor = filter(nodes(), all_of(vec![swap_rows(&axis_pred(&axis, s.as_ref()), 0), reach, Expr::Exists { rel: Box::new(before), negated: true }]));
            value_reach(rule, exists(anchor), false, out)
        }
        _ => Ok(()),
    }
}

/// An axis predicate with its two rows swapped: written over (candidate depth 0, anchor depth
/// 1) at subquery level `level`, returned over (anchor, candidate).
fn swap_rows(e: &E, level: u16) -> E {
    let sw = |d: u16| if d == level { level + 1 } else if d == level + 1 { level } else { d };
    let b = |x: &E| Box::new(swap_rows(x, level));
    match e {
        Expr::Col { depth, col } => Expr::Col { depth: sw(*depth), col: *col },
        Expr::Lit(l) => Expr::Lit(l.clone()),
        Expr::Not(x) => Expr::Not(b(x)),
        Expr::And(x, y) => Expr::And(b(x), b(y)),
        Expr::Or(x, y) => Expr::Or(b(x), b(y)),
        Expr::Cmp(op, x, y, a) => Expr::Cmp(*op, b(x), b(y), *a),
        Expr::Is { left, right, negated } => Expr::Is { left: b(left), right: b(right), negated: *negated },
        Expr::IsNull { e, negated } => Expr::IsNull { e: b(e), negated: *negated },
        Expr::Func(f, args) => Expr::Func(*f, args.iter().map(|a| swap_rows(a, level)).collect()),
        Expr::Case { operand, whens, else_ } => Expr::Case {
            operand: operand.as_ref().map(|o| b(o)),
            whens: whens.iter().map(|(w, t, a)| (swap_rows(w, level), swap_rows(t, level), *a)).collect(),
            else_: else_.as_ref().map(|x| b(x)),
        },
        Expr::InList { e, list, negated } => Expr::InList { e: b(e), list: list.iter().map(|(x, a)| (swap_rows(x, level), *a)).collect(), negated: *negated },
        Expr::Exists { rel, negated } => Expr::Exists { rel: Box::new(swap_rel(rel, level + 1)), negated: *negated },
        Expr::Scalar(rel) => Expr::Scalar(Box::new(swap_rel(rel, level + 1))),
        other => unreachable!("not in an axis predicate: {other:?}"),
    }
}

fn swap_rel(r: &R, level: u16) -> R {
    match r {
        Rel::Scan { .. } => r.clone(),
        Rel::Join { kind, left, right, on } => Rel::Join { kind: *kind, left: Box::new(swap_rel(left, level)), right: Box::new(swap_rel(right, level)), on: on.as_ref().map(|o| swap_rows(o, level)) },
        Rel::Filter { input, pred } => Rel::Filter { input: Box::new(swap_rel(input, level)), pred: swap_rows(pred, level) },
        Rel::Project { input, exprs, schema } => Rel::Project { input: Box::new(swap_rel(input, level)), exprs: exprs.iter().map(|e| swap_rows(e, level)).collect(), schema: schema.clone() },
        other => unreachable!("not in an axis predicate: {other:?}"),
    }
}

fn col(c: u16) -> E {
    Expr::Col { depth: 0, col: c }
}
fn up(d: u16, c: u16) -> E {
    Expr::Col { depth: d, col: c }
}
fn text(s: &str) -> E {
    Expr::Lit(Lit::Text(s.into()))
}
fn int(i: i64) -> E {
    Expr::Lit(Lit::Int(i))
}
fn eq(a: E, b: E) -> E {
    Expr::Cmp(CmpOp::Eq, Box::new(a), Box::new(b), (Aff::None, Aff::None))
}
fn cmp(op: CmpOp, a: E, b: E) -> E {
    Expr::Cmp(op, Box::new(a), Box::new(b), (Aff::None, Aff::None))
}
fn and(a: E, b: E) -> E {
    Expr::And(Box::new(a), Box::new(b))
}
fn or(a: E, b: E) -> E {
    Expr::Or(Box::new(a), Box::new(b))
}
fn not(a: E) -> E {
    Expr::Not(Box::new(a))
}
fn all_of(v: Vec<E>) -> E {
    v.into_iter().reduce(and).unwrap_or(int(1))
}
fn exists(r: R) -> E {
    Expr::Exists { rel: Box::new(r), negated: false }
}
fn scan(c: CoreRel) -> R {
    Rel::Scan { src: FileSource::Core(c), schema: c.schema() }
}
fn filter(r: R, pred: E) -> R {
    Rel::Filter { input: Box::new(r), pred }
}
/// The candidate rows: `file ⋈ node`, in node order (the schema every axis predicate and verdict
/// reads: `path`, then `node`'s columns).
pub fn nodes() -> R {
    Rel::Join { kind: JoinKind::Inner, left: Box::new(scan(CoreRel::File)), right: Box::new(scan(CoreRel::Node)), on: None }
}
/// The node's source text.
fn node_text() -> E {
    Expr::Func(Func::SourceText, vec![col(PATH), col(START), col(END)])
}
fn rx(t: E, p: &str) -> E {
    Expr::Func(Func::RxMatches, vec![t, text(p)])
}

/// The rule's three-valued verdict at the depth-0 candidate row.
fn verdict(r: &Rule) -> E {
    match r {
        Rule::All(items) => all_of(items.iter().map(verdict).collect()),
        Rule::Any(items) => items.iter().map(verdict).reduce(or).unwrap_or(int(0)),
        Rule::Not(x) => not(verdict(x)),
        Rule::Util(_, x) => verdict(x),
        Rule::Kind(k) => and(eq(col(KIND), text(k)), eq(col(NAMED), int(1))),
        Rule::Regex(p) => rx(node_text(), p),
        Rule::Meta { named: true, .. } => eq(col(NAMED), int(1)),
        Rule::Meta { named: false, .. } => int(1),
        Rule::Literal { kind, text: t } => all_of(vec![eq(col(KIND), text(kind)), eq(col(NAMED), int(1)), eq(node_text(), text(t))]),
        Rule::FilePath { regex, not_regex } => {
            let mut v = Vec::new();
            if let Some(p) = regex {
                v.push(rx(col(PATH), p));
            }
            if let Some(p) = not_regex {
                v.push(not(rx(col(PATH), p)));
            }
            all_of(v)
        }
        Rule::Never => int(0),
        Rule::Value { eq: want, regex } => {
            // A named string literal: TRUE/FALSE from its `str` value, NULL when its value is
            // unknown, FALSE for a bytes literal (in neither relation).
            let mut conds = vec![eq(col(0), up(1, ID))];
            if let Some(w) = want {
                conds.push(eq(col(1), text(w)));
            }
            if let Some(p) = regex {
                conds.push(rx(col(1), p));
            }
            let str_holds = exists(filter(scan(CoreRel::StringValue), all_of(conds)));
            let unknown = exists(filter(scan(CoreRel::StringValueUnknown), eq(col(0), up(1, ID))));
            Expr::Case {
                operand: None,
                whens: vec![(not(eq(col(NAMED), int(1))), int(0), (Aff::None, Aff::None)), (str_holds, int(1), (Aff::None, Aff::None)), (unknown, Expr::Lit(Lit::Null), (Aff::None, Aff::None))],
                else_: Some(Box::new(int(0))),
            }
        }
        Rule::Rel { has, field, stop, rule } => {
            let (axis, s) = axis_of(*has, field.as_deref(), stop);
            let cands = filter(nodes(), axis_pred(&axis, s.as_ref()));
            let v = verdict(rule);
            if !reads_values(rule) {
                return exists(filter(cands, v));
            }
            let hit = exists(filter(cands.clone(), v.clone()));
            let open = exists(filter(cands, Expr::IsNull { e: Box::new(v), negated: false }));
            Expr::Case { operand: None, whens: vec![(hit, int(1), (Aff::None, Aff::None)), (open, Expr::Lit(Lit::Null), (Aff::None, Aff::None))], else_: Some(Box::new(int(0))) }
        }
    }
}

/// A relation search's axis: which nodes it tests from the anchor, in search order (preorder
/// for `has`, nearest first for `inside`). `*Until` carry a stop rule's verdict.
#[derive(Clone, Debug, PartialEq)]
pub enum Axis {
    /// `has` neighbour.
    Children,
    /// `has … stopBy: end`.
    Descendants,
    /// `has … stopBy: S`: descendants reached without passing a node where `S` holds.
    DescendantsUntil,
    /// `inside` neighbour.
    Parent,
    /// `inside … stopBy: end`.
    Ancestors,
    /// `inside … stopBy: S`: ancestors reached without passing a node where `S` holds.
    AncestorsUntil,
    /// `has` neighbour with `field`: the first child under the field.
    FieldChild(String),
    /// `has … stopBy: end` with `field`: that child and its descendants.
    FieldSubtree(String),
    /// `inside` neighbour with `field`: the parent, when the anchor is its first child under the field.
    InsideField(String),
    /// `inside … stopBy: end` with `field`: the ancestors entered through their first child under the field.
    InsideFieldEnd(String),
}

impl Axis {
    /// Whether the search walks towards the root (nearest candidate first).
    pub fn upward(&self) -> bool {
        matches!(self, Axis::Parent | Axis::Ancestors | Axis::AncestorsUntil | Axis::InsideField(_) | Axis::InsideFieldEnd(_))
    }
}

fn axis_of(has: bool, field: Option<&str>, stop: &Stop) -> (Axis, Option<E>) {
    match (has, field, stop) {
        (true, None, Stop::Neighbor) => (Axis::Children, None),
        (true, None, Stop::End) => (Axis::Descendants, None),
        (true, None, Stop::Rule(s)) => (Axis::DescendantsUntil, Some(verdict(s))),
        (false, None, Stop::Neighbor) => (Axis::Parent, None),
        (false, None, Stop::End) => (Axis::Ancestors, None),
        (false, None, Stop::Rule(s)) => (Axis::AncestorsUntil, Some(verdict(s))),
        (true, Some(f), Stop::Neighbor) => (Axis::FieldChild(f.to_owned()), None),
        (true, Some(f), Stop::End) => (Axis::FieldSubtree(f.to_owned()), None),
        (false, Some(f), Stop::Neighbor) => (Axis::InsideField(f.to_owned()), None),
        (false, Some(f), Stop::End) => (Axis::InsideFieldEnd(f.to_owned()), None),
        (_, Some(_), Stop::Rule(_)) => unreachable!("refused in `relation`"),
    }
}

/// The predicate selecting an axis's candidates: over the candidate row (`nodes()`, depth 0) and
/// the anchor (a `nodes()` row, depth 1). `stop`: the stop rule's verdict over a `nodes()` row
/// (`*Until` axes only).
pub fn axis_pred(axis: &Axis, stop: Option<&E>) -> E {
    let desc = || and(cmp(CmpOp::Gt, col(ID), up(1, ID)), cmp(CmpOp::Lt, col(ID), up(1, SUBTREE_END)));
    let anc = || and(cmp(CmpOp::Lt, col(ID), up(1, ID)), cmp(CmpOp::Gt, col(SUBTREE_END), up(1, ID)));
    // No earlier sibling of the node at depth `d` (a `nodes()` row) under field `f`.
    let first_under = |f: &str, d: u16| Expr::Exists {
        rel: Box::new(filter(scan(CoreRel::Node), all_of(vec![eq(col(N_PARENT), up(d, PARENT)), eq(col(N_FIELD), text(f)), cmp(CmpOp::Lt, col(N_ID), up(d, ID))]))),
        negated: true,
    };
    match axis {
        Axis::Children => eq(col(PARENT), up(1, ID)),
        Axis::Descendants => desc(),
        Axis::DescendantsUntil => {
            // No `b` with anchor < b < candidate on the candidate's ancestor path where S(b).
            let between = all_of(vec![cmp(CmpOp::Gt, col(ID), up(2, ID)), cmp(CmpOp::Lt, col(ID), up(1, ID)), cmp(CmpOp::Gt, col(SUBTREE_END), up(1, ID)), stop.expect("stop verdict").clone()]);
            and(desc(), not(exists(filter(nodes(), between))))
        }
        Axis::Parent => eq(col(ID), up(1, PARENT)),
        Axis::Ancestors => anc(),
        Axis::AncestorsUntil => {
            // No ancestor `b` of the anchor deeper than the candidate where S(b).
            let between = all_of(vec![cmp(CmpOp::Gt, col(ID), up(1, ID)), cmp(CmpOp::Lt, col(ID), up(2, ID)), cmp(CmpOp::Gt, col(SUBTREE_END), up(2, ID)), stop.expect("stop verdict").clone()]);
            and(anc(), not(exists(filter(nodes(), between))))
        }
        Axis::FieldChild(f) => all_of(vec![eq(col(PARENT), up(1, ID)), eq(col(FIELD), text(f)), first_under(f, 1)]),
        Axis::FieldSubtree(f) => {
            // Some first field child `d` of the anchor with d ≤ id < subtree_end(d).
            let earlier = filter(scan(CoreRel::Node), all_of(vec![eq(col(N_PARENT), up(1, N_PARENT)), eq(col(N_FIELD), text(f)), cmp(CmpOp::Lt, col(N_ID), up(1, N_ID))]));
            let first = filter(
                scan(CoreRel::Node),
                all_of(vec![
                    eq(col(N_PARENT), up(2, ID)),
                    eq(col(N_FIELD), text(f)),
                    Expr::Exists { rel: Box::new(earlier), negated: true },
                    cmp(CmpOp::Ge, up(1, ID), col(N_ID)),
                    cmp(CmpOp::Lt, up(1, ID), col(N_SUBTREE_END)),
                ]),
            );
            exists(first)
        }
        Axis::InsideField(f) => all_of(vec![eq(col(ID), up(1, PARENT)), eq(up(1, FIELD), text(f)), first_under(f, 2)]),
        Axis::InsideFieldEnd(f) => {
            // The candidate's child `c` on the path to the anchor is its first child under `f`.
            let path_child = all_of(vec![eq(col(PARENT), up(1, ID)), cmp(CmpOp::Le, col(ID), up(2, ID)), cmp(CmpOp::Gt, col(SUBTREE_END), up(2, ID)), eq(col(FIELD), text(f)), first_under(f, 1)]);
            and(anc(), exists(filter(nodes(), path_child)))
        }
    }
}

/// Lowers a pattern input. `known_field(name)`: whether the grammar has the field (the executor
/// folds a relation over an unknown field to `Never`, which changes which conjuncts it evaluates);
/// `None` (not known) lowers a field relation only where that cannot change a refusal.
pub fn lower_input(input: &InputDef, utils: &BTreeMap<String, Json>, known_field: &dyn Fn(&str) -> Option<bool>) -> Result<Lowered, String> {
    let InputKind::Pattern { query } = &input.kind else { return Err("regex clause (not a pattern query)".into()) };
    let q = obj(query, "ast")?;
    for k in q.keys() {
        if !["language", "syntax", "rule", "utils", "transform"].contains(&k.as_str()) {
            return Err(format!("ast key `{k}`"));
        }
    }
    if q.get("language").and_then(Json::as_str) != Some("python") {
        return Err("a non-Python query".into());
    }
    if input.each.is_some() {
        return Err("`each` (one row per binding)".into());
    }
    if input.paths.is_some() {
        return Err("`evidence.paths` glob filter".into());
    }
    let mut all_utils = utils.clone();
    if let Some(u) = q.get("utils").and_then(Json::as_object) {
        all_utils.extend(u.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let mut front = Front { utils: &all_utils, known_field, stack: Vec::new(), fields: Vec::new() };
    let rule = front.rule(q.get("rule").ok_or("ast.rule missing")?, "rule")?;
    // Root chain: captures.
    let (mut bound, mut caps) = (Vec::new(), Vec::new());
    root_chain(&rule, &mut bound, &mut caps)?;
    {
        // A name bound twice compares instead of binding (`ast.rs` `MetaVar` with a binding).
        let mut names: Vec<&str> = bound.iter().map(String::as_str).chain(caps.iter().map(|c| c.name.as_str())).collect();
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            return Err("a metavariable bound twice on the root chain".into());
        }
    }
    let mut decoded: Option<String> = None;
    if let Some(t) = q.get("transform") {
        for (var, spec) in obj(t, "transform")? {
            let rep = spec.get("replace").ok_or_else(|| format!("transform {var}: only replace"))?;
            let src = rep.get("source").and_then(Json::as_str).unwrap_or("");
            if rep.get("value") != Some(&Json::Bool(true)) || rep.get("replace").and_then(Json::as_str) != Some("\\b\\B") || rep.get("by").and_then(Json::as_str) != Some("") || spec.as_object().map(|o| o.len()) != Some(1) {
                return Err(format!("transform {var}: shape beyond the decoded value"));
            }
            if !src.strip_prefix('$').is_some_and(|s| bound.iter().any(|b| b == s)) {
                return Err(format!("transform {var}: source {src} is not a root capture"));
            }
            if decoded.replace(var.clone()).is_some() {
                return Err("more than one transform".into());
            }
        }
    }
    let kinds = root_kinds(&rule, &mut Vec::new());
    let strings_only = kinds.as_ref().is_some_and(|k| !k.is_empty() && k.iter().all(|k| STRING_KINDS.contains(&k.as_str())));
    if decoded.is_some() && !strings_only {
        return Err("a decoded value on roots that are not only string literals".into());
    }
    let syntax = match q.get("syntax").and_then(Json::as_str) {
        Some("valid") => Some(false),
        Some("invalid") => Some(true),
        None => None,
        Some(o) => return Err(format!("syntax {o}")),
    };
    let mut gate: Vec<E> = Vec::new();
    if let Some(k) = &kinds {
        gate.push(Expr::InList { e: Box::new(col(KIND)), list: k.iter().map(|k| (text(k), (Aff::None, Aff::None))).collect(), negated: false });
    }
    if let Some(errors) = syntax {
        gate.push(Expr::Exists { rel: Box::new(scan(CoreRel::SyntaxError)), negated: !errors });
    }
    let v = verdict(&rule);
    // Every `value` test's reach (its refusals), from the candidates' gate.
    let mut values = Vec::new();
    value_reach(&rule, all_of(gate.clone()), strings_only, &mut values)?;
    if !values.is_empty() && !front.fields.is_empty() {
        return Err(format!("`field: {}` of unknown grammar status next to a `value` test", front.fields.join(", ")));
    }
    let escape = exists(filter(scan(CoreRel::StringValueUnknown), and(eq(col(0), up(1, ID)), eq(col(1), int(1)))));
    let mut refuse_if = Vec::new();
    if syntax.is_none() {
        refuse_if.push((scan(CoreRel::SyntaxError), NO_SYNTAX.to_owned()));
    }
    for reach in values {
        refuse_if.push((filter(nodes(), all_of(vec![reach, eq(col(NAMED), int(1)), escape.clone()])), NAMED_ESCAPE.to_owned()));
    }
    // Captures by relation searches: `LEFT JOIN node x ON x.id = (first candidate)`, appended
    // after the candidate row (`nodes()`, 12 columns), 11 columns each.
    let mut acc = filter(nodes(), {
        let mut keep = gate.clone();
        keep.push(v.clone());
        all_of(keep)
    });
    let mut cap_base: Vec<(String, u16)> = Vec::new();
    let mut width: u16 = 1 + CoreRel::Node.columns().len() as u16;
    for c in &caps {
        acc = Rel::Join { kind: JoinKind::Left, left: Box::new(acc), right: Box::new(scan(CoreRel::Node)), on: Some(eq(col(width + N_ID), capture_id(c))) };
        cap_base.push((c.name.clone(), width));
        width += CoreRel::Node.columns().len() as u16;
    }
    // Where a capture's node columns are: the root's (`nodes()` columns from 1) or a join's.
    let at = |n: &str| -> Option<u16> {
        if bound.iter().any(|b| b == n) {
            Some(1)
        } else {
            cap_base.iter().find(|(c, _)| c == n).map(|(_, b)| *b)
        }
    };
    let decoded_value = || -> E {
        let v = Expr::Scalar(Box::new(Rel::Project { input: Box::new(filter(scan(CoreRel::StringValue), eq(col(0), up(1, ID)))), exprs: vec![col(1)], schema: vec![Column::new("value", Aff::None)] }));
        Expr::Func(Func::Coalesce, vec![v, text("")])
    };
    // A capture's node column `c` (a `node` column index) at base `b`.
    let nc = |b: u16, c: u16| col(b + c);
    fn proj(p: &Projection, at: &dyn Fn(&str) -> Option<u16>, nc: &dyn Fn(u16, u16) -> E, decoded: Option<&str>, decoded_value: &dyn Fn() -> E) -> Result<E, String> {
        let base = |n: &str| at(n).ok_or_else(|| format!("projection of `{n}` (not bound on the root chain)"));
        Ok(match p {
            Projection::Path => col(PATH),
            Projection::Text(n) if decoded == Some(n.as_str()) => decoded_value(),
            Projection::Capture(n) | Projection::Text(n) => {
                let b = base(n)?;
                // NULL bounds (an unbound capture) read as NULL.
                Expr::Func(Func::SourceText, vec![col(PATH), nc(b, N_START), nc(b, N_END)])
            }
            Projection::Kind(n) => nc(base(n)?, N_KIND),
            Projection::StartByte(n) => nc(base(n)?, N_START),
            Projection::EndByte(n) => nc(base(n)?, N_END),
            // `node.start_line` is 1-based; the executor projects the 0-based line.
            Projection::StartLine(n) => Expr::Arith(ArithOp::Sub, Box::new(nc(base(n)?, N_START_LINE)), Box::new(int(1))),
            Projection::Regex { source, regex, group } => {
                let s = proj(source, at, nc, decoded, decoded_value)?;
                Expr::Case {
                    operand: None,
                    whens: vec![(Expr::IsNull { e: Box::new(s.clone()), negated: false }, Expr::Lit(Lit::Null), (Aff::None, Aff::None))],
                    else_: Some(Box::new(Expr::Func(Func::RxExtract, vec![s, text(regex), int(*group as i64)]))),
                }
            }
            other => return Err(format!("projection {other:?}")),
        })
    }
    let mut exprs = vec![col(PATH)];
    match &input.anchor {
        Anchor::Span => exprs.extend([col(START), col(END)]),
        Anchor::FileStart => exprs.extend([int(0), int(0)]),
        Anchor::Capture(n) if bound.iter().any(|b| b == n) => exprs.extend([col(START), col(END)]),
        // An unbound capture anchors at the witness.
        Anchor::Capture(n) if at(n).is_some() => {
            let b = at(n).unwrap();
            exprs.extend([Expr::Func(Func::Coalesce, vec![nc(b, N_START), col(START)]), Expr::Func(Func::Coalesce, vec![nc(b, N_END), col(END)])]);
        }
        a => return Err(format!("anchor {a:?}")),
    }
    for (_, p) in &input.fields {
        exprs.push(proj(p, &at, &nc, decoded.as_deref(), &decoded_value)?);
    }
    if decoded.is_some() {
        let mut c = gate.clone();
        c.push(v.clone());
        c.push(escape);
        refuse_if.push((filter(nodes(), all_of(c)), NAMED_ESCAPE.to_owned()));
    }
    let rows = Rel::Project { input: Box::new(acc), exprs, schema: input.schema.clone() };
    Ok(Lowered { rows, refuse_if })
}

/// A capture's node id over the candidate row (depth 0): the first candidate on its axis, in
/// search order, where its inner rule holds; NULL when none.
fn capture_id(c: &Cap) -> E {
    let (axis, s) = axis_of(c.has, c.field.as_deref(), &c.stop);
    let found = filter(filter(nodes(), axis_pred(&axis, s.as_ref())), verdict(&c.inner));
    // `node` order is preorder; an upward search takes the nearest (largest id) first.
    let ordered = if axis.upward() { Rel::Sort { input: Box::new(found), keys: vec![(ID, true)], limit: Some(int(1)), offset: None, total: Total::InputOrder } } else { found };
    Expr::Scalar(Box::new(Rel::Project { input: Box::new(ordered), exprs: vec![col(ID)], schema: vec![Column::new("id", Aff::None)] }))
}
