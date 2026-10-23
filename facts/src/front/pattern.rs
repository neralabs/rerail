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
//!   `field` = the first child under the field (neighbour), or it and its descendants (`end`).
//! - **Run-time refusals** (the content fails, as the executor's `unsupported`): a `value` test
//!   or a `DECODED` projection that decodes a literal holding a `\N{...}` escape, counted only
//!   where the executor evaluates it (the reach of a conjunct in an `all` chain: every earlier
//!   conjunct not FALSE); a query without `syntax` over a content with a syntax error.
use crate::ir::{Anchor, CmpOp, CoreRel, Expr, FileSource, Func, InputDef, InputKind, JoinKind, Lit, Lowered, Projection, Rel};
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
    Rel { has: bool, field: Option<String>, end: bool, rule: Box<Rule> },
    FilePath { regex: Option<String>, not_regex: Option<String> },
    Value { eq: Option<String>, regex: Option<String> },
    /// `matches: id`, inlined (kept as a node for `root_kinds`' shared visited set).
    Util(String, Box<Rule>),
    Never,
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
        let end = match o.get("stopBy") {
            None => false,
            Some(Json::String(s)) if s == "neighbor" => false,
            Some(Json::String(s)) if s == "end" => true,
            Some(_) => return Err(format!("{at}/stopBy: a stop rule")),
        };
        let field = o.get("field").and_then(Json::as_str).map(str::to_owned);
        if field.is_some() && !has {
            return Err(format!("{at}: `inside` with `field`"));
        }
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
        Ok(Rule::Rel { has, field, end, rule: Box::new(inner) })
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
        Rule::Not(x) | Rule::Util(_, x) | Rule::Rel { rule: x, .. } => reads_values(x),
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

/// The metavariables a root-chain (`all` nested in `all`, utilities inlined) binds to the root,
/// and the value tests on that chain with their reach (every earlier conjunct not FALSE).
fn root_chain(r: &Rule, reach: &[E], bound: &mut Vec<String>, values: &mut Vec<(Vec<E>, Rule)>) -> Result<(), String> {
    match r {
        Rule::All(items) => {
            let mut reach = reach.to_vec();
            for it in items {
                root_chain(it, &reach, bound, values)?;
                // The next conjunct runs unless this one is FALSE (`match_all`: an unknown
                // conjunct does not stop the chain).
                reach.push(Expr::Is { left: Box::new(verdict(it)), right: Box::new(Expr::Lit(Lit::Int(0))), negated: true });
            }
            Ok(())
        }
        Rule::Util(_, inner) => root_chain(inner, reach, bound, values),
        Rule::Meta { name: Some(n), .. } => {
            bound.push(n.clone());
            Ok(())
        }
        Rule::Value { .. } => {
            values.push((reach.to_vec(), r.clone()));
            Ok(())
        }
        other => {
            if binds(other) {
                return Err("a metavariable under `any`/`not`/a relation (captures are lowered on the root chain only)".into());
            }
            if reads_values(other) {
                return Err("a `value` test under `any`/`not`/a relation (its evaluation reach, which decides `\\N{...}` refusals, is lowered on the root chain only)".into());
            }
            Ok(())
        }
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
/// The candidate rows: `file ⋈ node`, in node order.
fn nodes() -> R {
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
        Rule::Rel { has, field, end, rule } => {
            let cands = candidates(*has, field.as_deref(), *end);
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

/// The candidate rows of a relation search from the anchor (the enclosing row, depth 1).
fn candidates(has: bool, field: Option<&str>, end: bool) -> R {
    let pred = match (has, field, end) {
        (true, None, false) => eq(col(PARENT), up(1, ID)),
        (true, None, true) => and(cmp(CmpOp::Gt, col(ID), up(1, ID)), cmp(CmpOp::Lt, col(ID), up(1, SUBTREE_END))),
        (false, None, false) => eq(col(ID), up(1, PARENT)),
        (false, None, true) => and(cmp(CmpOp::Lt, col(ID), up(1, ID)), cmp(CmpOp::Gt, col(SUBTREE_END), up(1, ID))),
        (true, Some(f), false) => {
            // The first child under the field: no earlier sibling under it.
            let earlier = filter(scan(CoreRel::Node), all_of(vec![eq(col(N_PARENT), up(1, PARENT)), eq(col(N_FIELD), text(f)), cmp(CmpOp::Lt, col(N_ID), up(1, ID))]));
            all_of(vec![eq(col(PARENT), up(1, ID)), eq(col(FIELD), text(f)), Expr::Exists { rel: Box::new(earlier), negated: true }])
        }
        (true, Some(f), true) => {
            // The first child under the field and its descendants: some first field child `d`
            // of the anchor with d ≤ id < subtree_end(d).
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
        (false, Some(_), _) => unreachable!("refused in `relation`"),
    };
    filter(nodes(), pred)
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
    // Root chain: captures and value tests with their reach.
    let (mut bound, mut values) = (Vec::new(), Vec::new());
    root_chain(&rule, &[], &mut bound, &mut values)?;
    if !values.is_empty() && !front.fields.is_empty() {
        return Err(format!("`field: {}` of unknown grammar status next to a `value` test", front.fields.join(", ")));
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
    if (!values.is_empty() || decoded.is_some()) && !strings_only {
        return Err("a `value` test or decoded value on roots that are not only string literals".into());
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
    let escape = exists(filter(scan(CoreRel::StringValueUnknown), and(eq(col(0), up(1, ID)), eq(col(1), int(1)))));
    let mut refuse_if = Vec::new();
    if syntax.is_none() {
        refuse_if.push((scan(CoreRel::SyntaxError), NO_SYNTAX.to_owned()));
    }
    for (reach, _) in &values {
        let mut c = gate.clone();
        c.extend(reach.iter().cloned());
        c.push(eq(col(NAMED), int(1)));
        c.push(escape.clone());
        refuse_if.push((filter(nodes(), all_of(c)), NAMED_ESCAPE.to_owned()));
    }
    // Projections.
    let root_text = |n: &str| -> Option<E> { bound.iter().any(|b| b == n).then(node_text) };
    let decoded_value = || -> E {
        let v = Expr::Scalar(Box::new(Rel::Project { input: Box::new(filter(scan(CoreRel::StringValue), eq(col(0), up(1, ID)))), exprs: vec![col(1)], schema: vec![crate::ir::Column::new("value", Aff::None)] }));
        Expr::Func(Func::Coalesce, vec![v, text("")])
    };
    fn proj(p: &Projection, root_text: &dyn Fn(&str) -> Option<E>, decoded: Option<&str>, decoded_value: &dyn Fn() -> E, bound: &[String]) -> Result<E, String> {
        let root = |n: &str| bound.iter().any(|b| b == n);
        Ok(match p {
            Projection::Path => col(PATH),
            Projection::Text(n) if decoded == Some(n.as_str()) => decoded_value(),
            Projection::Capture(n) | Projection::Text(n) => root_text(n).ok_or_else(|| format!("projection of `{n}` (not a root capture)"))?,
            Projection::Kind(n) if root(n) => col(KIND),
            Projection::StartByte(n) if root(n) => col(START),
            Projection::EndByte(n) if root(n) => col(END),
            Projection::Regex { source, regex, group } => {
                let s = proj(source, root_text, decoded, decoded_value, bound)?;
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
        a => return Err(format!("anchor {a:?}")),
    }
    for (_, p) in &input.fields {
        exprs.push(proj(p, &root_text, decoded.as_deref(), &decoded_value, &bound)?);
    }
    let mut keep = gate.clone();
    keep.push(v.clone());
    if decoded.is_some() {
        let mut c = keep.clone();
        c.push(escape);
        refuse_if.push((filter(nodes(), all_of(c)), NAMED_ESCAPE.to_owned()));
    }
    let rows = Rel::Project { input: Box::new(filter(nodes(), all_of(keep))), exprs, schema: input.schema.clone() };
    Ok(Lowered { rows, refuse_if })
}
