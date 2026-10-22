//! The AST query language of rerail's DSL (the subset the batch-6 policies use), evaluated over
//! per-file facts. Semantics mirror the native matcher (ast-grep-config 0.45.3 plus rerail's
//! extensions): every node is a candidate root in preorder; a root yields at most one match, the
//! first-success binding with no backtracking; relations bind from the first related node that
//! matches; verdicts are three-valued in a clause that reads values.
use crate::facts::{self, Facts, NONE};
use crate::pyvalue::{self, PyValue};
use regex::Regex;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};

pub type Name = u16;

#[derive(Clone)]
pub enum Stop {
    Neighbor,
    End,
    Rule(Box<Rule>),
}

#[derive(Clone)]
pub enum Rule {
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Not(Box<Rule>),
    /// Named kind: kind symbol and the node is named.
    Kind(u32),
    Regex(Regex),
    /// `$X` (named only) / `$$X` (any node); `name` None for `$_`/`$$_`.
    MetaVar { name: Option<Name>, named: bool },
    /// A literal pattern compiled to a terminal: kind and exact text.
    Literal { kind: u32, text: String },
    Has { rule: Box<Rule>, stop: Stop },
    Inside { rule: Box<Rule>, stop: Stop },
    Field { inside: bool, field: u16, rule: Box<Rule>, stop: Stop },
    Util(usize),
    FilePath { regex: Option<Regex>, not_regex: Option<Regex> },
    NthChild { step: i32, offset: i32, of_rule: Option<Box<Rule>>, reverse: bool },
    Value(ValueTest),
    Compare { left: Name, right: Name, op: CompareOp, text: CompareText },
    Count { relation: Box<Rule>, min: usize, max: Option<usize> },
    Parse(Box<ParseTest>),
    /// `capture: NAME` inside a parse.
    Capture(Name),
    Never,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CompareOp {
    Eq,
    Ne,
}
#[derive(Clone, Copy, PartialEq)]
pub enum CompareText {
    Source,
    Node,
}

#[derive(Clone)]
pub struct ValueTest {
    pub eq: Option<String>,
    pub regex: Option<Regex>,
}

#[derive(Clone)]
pub struct ParseTest {
    pub grammar: ParseGrammar,
    pub rule: Rule,
}
#[derive(Clone, Copy, PartialEq)]
pub enum ParseGrammar {
    PythonFString,
    Annotation,
}

/// A capture binding: a node of the file, or a node of a parsed literal (`parse`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    Node(u32),
    /// Index into the file's parsed-literal side table.
    Parsed(u32),
}

#[derive(Clone, Default, Debug)]
pub struct Env {
    pub vars: Vec<(Name, Bound)>,
}
impl Env {
    pub fn get(&self, name: Name) -> Option<Bound> {
        self.vars.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
    }
    fn set(&mut self, name: Name, b: Bound) {
        if let Some(slot) = self.vars.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = b;
        } else {
            self.vars.push((name, b));
        }
    }
}

/// A compiled `ast` query.
pub struct Query {
    pub rule: Rule,
    pub utils: Vec<Rule>,
    pub syntax: SyntaxMode,
    pub three_valued: bool,
    /// `transform: { NAME: { replace: { source: $STRING, value: true, replace: "\b\B", by: "" } } }`
    pub decoded: Vec<(Name, Name)>,
    /// Kinds a root must have, when the rule fixes them (a sound prefilter).
    pub root_kinds: Option<Vec<u32>>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SyntaxMode {
    Valid,
    Invalid,
    Unspecified,
}

/// Capture names, interned per compiled policy set.
#[derive(Default)]
pub struct Names {
    pub names: Vec<String>,
    ids: BTreeMap<String, Name>,
}
impl Names {
    pub fn id(&mut self, name: &str) -> Name {
        if let Some(&i) = self.ids.get(name) {
            return i;
        }
        let i = self.names.len() as Name;
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), i);
        i
    }
    pub fn get(&self, name: &str) -> Option<Name> {
        self.ids.get(name).copied()
    }
}

pub struct Compiler<'a> {
    pub names: &'a mut Names,
    pub grammar: &'a rerail_engine::code_rule_dsl::tape::Grammar,
    util_ids: Vec<String>,
    util_json: Vec<Json>,
}

fn obj<'j>(v: &'j Json, at: &str) -> Result<&'j serde_json::Map<String, Json>, String> {
    v.as_object().ok_or_else(|| format!("{at}: expected object"))
}

const RULE_KEYS: &[&str] = &[
    "pattern", "kind", "regex", "capture", "parse", "nthChild", "filePath", "all", "any", "not",
    "matches", "inside", "has", "count", "compare", "value", "stopBy", "field",
];

impl<'a> Compiler<'a> {
    pub fn new(
        names: &'a mut Names,
        grammar: &'a rerail_engine::code_rule_dsl::tape::Grammar,
        utils: &BTreeMap<String, Json>,
    ) -> Self {
        Self {
            names,
            grammar,
            util_ids: utils.keys().cloned().collect(),
            util_json: utils.values().cloned().collect(),
        }
    }

    pub fn query(&mut self, ast: &Json) -> Result<Query, String> {
        let ast = obj(ast, "ast")?;
        for k in ast.keys() {
            if !["language", "syntax", "rule", "utils", "transform"].contains(&k.as_str()) {
                return Err(format!("unsupported ast key {k}"));
            }
        }
        if ast.get("language").and_then(Json::as_str) != Some("python") {
            return Err("only python".into());
        }
        if let Some(local) = ast.get("utils") {
            for (k, v) in obj(local, "utils")? {
                if self.util_ids.contains(k) {
                    return Err(format!("utility {k} defined twice"));
                }
                self.util_ids.push(k.clone());
                self.util_json.push(v.clone());
            }
        }
        let syntax = match ast.get("syntax").and_then(Json::as_str) {
            Some("valid") => SyntaxMode::Valid,
            Some("invalid") => SyntaxMode::Invalid,
            None => SyntaxMode::Unspecified,
            Some(o) => return Err(format!("syntax {o}")),
        };
        let rule = self.rule(ast.get("rule").ok_or("ast.rule missing")?, "rule")?;
        // Utilities reachable from the rule, compiled in id order (indices are stable).
        let mut utils = Vec::new();
        for i in 0..self.util_ids.len() {
            let j = self.util_json[i].clone();
            utils.push(self.rule(&j, &format!("utils/{}", self.util_ids[i]))?);
        }
        let mut decoded = Vec::new();
        if let Some(t) = ast.get("transform") {
            for (var, spec) in obj(t, "transform")? {
                let rep = spec
                    .get("replace")
                    .ok_or_else(|| format!("transform {var}: only replace"))?;
                let src = rep.get("source").and_then(Json::as_str).unwrap_or("");
                if rep.get("value") != Some(&Json::Bool(true))
                    || rep.get("replace").and_then(Json::as_str) != Some("\\b\\B")
                    || rep.get("by").and_then(Json::as_str) != Some("")
                    || !src.starts_with('$')
                    || spec.as_object().map(|o| o.len()) != Some(1)
                {
                    return Err(format!("transform {var}: unsupported shape"));
                }
                let target = self.names.id(var);
                let source = self.names.id(&src[1..]);
                decoded.push((target, source));
            }
        }
        let three_valued = reads_values(&rule, &utils, &mut BTreeSet::new());
        let root_kinds = root_kinds(&rule, &utils, &mut BTreeSet::new());
        Ok(Query { rule, utils, syntax, three_valued, decoded, root_kinds })
    }

    fn kind(&self, kind: &str) -> Result<Rule, String> {
        // `kind` names a named kind; an unknown name is a compile gap (never matches).
        Ok(Rule::Kind(facts::intern(kind)))
    }

    fn rule(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let r = obj(v, at)?;
        for k in r.keys() {
            if !RULE_KEYS.contains(&k.as_str()) {
                return Err(format!("{at}: unsupported rule key `{k}`"));
            }
        }
        if r.contains_key("stopBy") || r.contains_key("field") {
            return Err(format!("{at}: stopBy/field outside a relation"));
        }
        let mut terms = Vec::new();
        if let Some(p) = r.get("pattern") {
            let p = p.as_str().ok_or_else(|| format!("{at}/pattern: contextual patterns unsupported"))?;
            terms.push(self.pattern(p, at)?);
        }
        if let Some(k) = r.get("kind").and_then(Json::as_str) {
            terms.push(self.kind(k)?);
        }
        if let Some(rx) = r.get("regex").and_then(Json::as_str) {
            terms.push(Rule::Regex(Regex::new(rx).map_err(|e| format!("{at}/regex: {e}"))?));
        }
        if let Some(c) = r.get("capture") {
            let c = c.as_str().ok_or("capture must be a name")?;
            terms.push(Rule::Capture(self.names.id(c.trim_start_matches('$'))));
        }
        if let Some(p) = r.get("parse") {
            terms.push(self.parse(p, &format!("{at}/parse"))?);
        }
        let nth = r.get("nthChild");
        let of_rule = nth.is_some_and(|n| n.get("ofRule").is_some());
        if let Some(n) = nth.filter(|_| !of_rule) {
            terms.push(self.nth_child(n, at)?);
        }
        if let Some(fp) = r.get("filePath") {
            let o = obj(fp, "filePath")?;
            let get = |k: &str| -> Result<Option<Regex>, String> {
                o.get(k)
                    .map(|v| Regex::new(v.as_str().unwrap_or("")).map_err(|e| e.to_string()))
                    .transpose()
            };
            terms.push(Rule::FilePath { regex: get("regex")?, not_regex: get("notRegex")? });
        }
        let from = terms.len();
        let mut group = 0usize;
        if let Some(n) = nth.filter(|_| of_rule) {
            terms.push(self.nth_child(n, at)?);
            group += 1;
        }
        let mut has_all = false;
        for key in ["all", "any"] {
            let Some(items) = r.get(key).and_then(Json::as_array) else { continue };
            group += 1;
            let compiled = items
                .iter()
                .enumerate()
                .map(|(i, it)| self.rule(it, &format!("{at}/{key}/{i}")))
                .collect::<Result<Vec<_>, _>>()?;
            if key == "all" {
                has_all = true;
                terms.push(if compiled.iter().any(|c| matches!(c, Rule::Never)) {
                    Rule::Never
                } else {
                    Rule::All(compiled)
                });
            } else {
                let kept: Vec<_> = compiled.into_iter().filter(|c| !matches!(c, Rule::Never)).collect();
                terms.push(if kept.is_empty() { Rule::Never } else { Rule::Any(kept) });
            }
        }
        let _ = has_all;
        if let Some(n) = r.get("not") {
            terms.push(Rule::Not(Box::new(self.rule(n, &format!("{at}/not"))?)));
            group += 1;
        }
        if let Some(m) = r.get("matches") {
            let id = m.as_str().ok_or("parameterised matches unsupported")?;
            let idx = self
                .util_ids
                .iter()
                .position(|u| u == id)
                .ok_or_else(|| format!("{at}/matches: unknown utility {id}"))?;
            terms.push(Rule::Util(idx));
            group += 1;
        }
        for key in ["inside", "has"] {
            if let Some(rel) = r.get(key) {
                terms.push(self.relation(rel, key == "inside", &format!("{at}/{key}"))?);
                group += 1;
            }
        }
        if group > 1 {
            order_by_captures(&mut terms, from, &self.util_rules_for_analysis()?)?;
        }
        if let Some(c) = r.get("count") {
            terms.push(self.count(c, &format!("{at}/count"))?);
        }
        if let Some(c) = r.get("compare") {
            terms.push(self.compare(c, &format!("{at}/compare"))?);
        }
        if let Some(v) = r.get("value") {
            terms.push(self.value(v, &format!("{at}/value"))?);
        }
        if terms.is_empty() {
            return Err(format!("{at}: empty rule"));
        }
        Ok(if terms.len() == 1 { terms.pop().unwrap() } else { Rule::All(terms) })
    }

    /// Utilities compiled shallowly for capture analysis during ordering. Recursion through
    /// utilities is handled by name: analysis reads `self.util_json` lazily.
    fn util_rules_for_analysis(&mut self) -> Result<UtilJson, String> {
        Ok(UtilJson { ids: self.util_ids.clone(), json: self.util_json.clone() })
    }

    fn pattern(&mut self, p: &str, at: &str) -> Result<Rule, String> {
        let p = p.trim();
        if let Some(rest) = p.strip_prefix("$$") {
            if rest.starts_with('$') {
                return Err(format!("{at}: `$$$` at the root is invalid"));
            }
            if !valid_meta(rest) {
                return Err(format!("{at}: pattern {p}"));
            }
            return Ok(Rule::MetaVar { name: (rest != "_").then(|| self.names.id(rest)), named: false });
        }
        if let Some(rest) = p.strip_prefix('$') {
            if !valid_meta(rest) {
                return Err(format!("{at}: pattern {p}"));
            }
            return Ok(Rule::MetaVar { name: (rest != "_").then(|| self.names.id(rest)), named: true });
        }
        // Literal patterns: a single token the Python grammar reads as one named leaf.
        match p {
            "True" => Ok(Rule::Literal { kind: facts::intern("true"), text: p.into() }),
            "False" => Ok(Rule::Literal { kind: facts::intern("false"), text: p.into() }),
            "None" => Ok(Rule::Literal { kind: facts::intern("none"), text: p.into() }),
            _ if p.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !PY_KEYWORDS.contains(&p) =>
            {
                Ok(Rule::Literal { kind: facts::intern("identifier"), text: p.into() })
            }
            _ => Err(format!("{at}: pattern `{p}` unsupported (only metavariables and single names)")),
        }
    }

    fn relation(&mut self, rel: &Json, inside: bool, at: &str) -> Result<Rule, String> {
        let o = obj(rel, at)?;
        let stop = match o.get("stopBy") {
            None => Stop::Neighbor,
            Some(Json::String(s)) if s == "neighbor" => Stop::Neighbor,
            Some(Json::String(s)) if s == "end" => Stop::End,
            Some(v @ Json::Object(_)) => Stop::Rule(Box::new(self.rule(v, &format!("{at}/stopBy"))?)),
            Some(_) => return Err(format!("{at}/stopBy")),
        };
        let field = o.get("field").and_then(Json::as_str);
        let mut nested = o.clone();
        nested.remove("stopBy");
        nested.remove("field");
        let inner = self.rule(&Json::Object(nested), at)?;
        if matches!(inner, Rule::Never) {
            return Ok(Rule::Never);
        }
        Ok(match field {
            None => {
                if inside {
                    Rule::Inside { rule: Box::new(inner), stop }
                } else {
                    Rule::Has { rule: Box::new(inner), stop }
                }
            }
            Some(name) => match self.grammar.field_code(name) {
                Some(code) => Rule::Field { inside, field: code, rule: Box::new(inner), stop },
                None => Rule::Never,
            },
        })
    }

    fn nth_child(&mut self, n: &Json, at: &str) -> Result<Rule, String> {
        let (pos, of_rule, reverse) = match n {
            Json::Number(x) => (x.as_i64().ok_or("nthChild")? as i32, None, false),
            Json::Object(o) => {
                let pos = o.get("position").and_then(Json::as_i64).ok_or_else(|| format!("{at}/nthChild/position must be a number"))? as i32;
                let of_rule = o.get("ofRule").map(|r| self.rule(r, &format!("{at}/nthChild/ofRule"))).transpose()?;
                let reverse = o.get("reverse").and_then(Json::as_bool).unwrap_or(false);
                (pos, of_rule, reverse)
            }
            _ => return Err(format!("{at}/nthChild: only numbers")),
        };
        Ok(Rule::NthChild { step: 0, offset: pos, of_rule: of_rule.map(Box::new), reverse })
    }

    fn count(&mut self, c: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(c, at)?;
        let min = o.get("min").and_then(Json::as_u64).unwrap_or(0) as usize;
        let max = o.get("max").and_then(Json::as_u64).map(|m| m as usize);
        if o.contains_key("as") {
            return Err(format!("{at}: count binding unsupported"));
        }
        let (key, rel) = ["has", "inside"]
            .iter()
            .find_map(|k| o.get(*k).map(|v| (*k, v)))
            .ok_or_else(|| format!("{at}: count needs has/inside"))?;
        let ro = obj(rel, at)?;
        let stop = match ro.get("stopBy") {
            None => Stop::Neighbor,
            Some(Json::String(s)) if s == "neighbor" => Stop::Neighbor,
            Some(Json::String(s)) if s == "end" => Stop::End,
            Some(v @ Json::Object(_)) => Stop::Rule(Box::new(self.rule(v, at)?)),
            _ => return Err(format!("{at}/stopBy")),
        };
        let mut nested = ro.clone();
        nested.remove("stopBy");
        nested.remove("field");
        let inner = Box::new(self.rule(&Json::Object(nested), at)?);
        let relation = if key == "has" { Rule::Has { rule: inner, stop } } else { Rule::Inside { rule: inner, stop } };
        Ok(Rule::Count { relation: Box::new(relation), min, max })
    }

    fn compare(&mut self, c: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(c, at)?;
        let left = o.get("left").and_then(Json::as_str).ok_or("compare.left")?;
        let right = o.get("right").and_then(Json::as_str).ok_or("compare.right")?;
        let op = match o.get("op").and_then(Json::as_str) {
            Some("eq") => CompareOp::Eq,
            Some("ne") => CompareOp::Ne,
            o => return Err(format!("{at}: op {o:?}")),
        };
        let text = match o.get("text").and_then(Json::as_str) {
            Some("source") | None => CompareText::Source,
            Some("node") => CompareText::Node,
            Some(t) => return Err(format!("{at}: text {t}")),
        };
        for k in o.keys() {
            if !["left", "right", "op", "text"].contains(&k.as_str()) {
                return Err(format!("{at}: compare key {k}"));
            }
        }
        Ok(Rule::Compare { left: self.names.id(left.trim_start_matches('$')), right: self.names.id(right.trim_start_matches('$')), op, text })
    }

    fn value(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(v, at)?;
        for k in o.keys() {
            if !["eq", "regex"].contains(&k.as_str()) {
                return Err(format!("{at}: value key {k} unsupported"));
            }
        }
        let eq = match o.get("eq") {
            None => None,
            Some(Json::String(s)) => Some(s.clone()),
            Some(_) => return Err(format!("{at}: value.eq only strings")),
        };
        let regex = o
            .get("regex")
            .map(|r| Regex::new(r.as_str().unwrap_or("")).map_err(|e| format!("{at}: {e}")))
            .transpose()?;
        Ok(Rule::Value(ValueTest { eq, regex }))
    }

    fn parse(&mut self, p: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(p, at)?;
        let grammar = match o.get("grammar").and_then(Json::as_str) {
            Some("pythonFString") => ParseGrammar::PythonFString,
            Some("annotation") => ParseGrammar::Annotation,
            g => return Err(format!("{at}: grammar {g:?}")),
        };
        let mut nested = o.clone();
        nested.remove("grammar");
        let rule = self.rule(&Json::Object(nested), at)?;
        Ok(Rule::Parse(Box::new(ParseTest { grammar, rule })))
    }
}

const PY_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

fn valid_meta(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

// ---------------------------------------------------------------------------------------------
// Capture analysis (captures.rs): binds / reads / mentions.

pub struct UtilJson {
    ids: Vec<String>,
    json: Vec<Json>,
}

fn binds(rule: &Rule, out: &mut BTreeSet<Name>, utils: &dyn Fn(usize) -> Option<BTreeSet<Name>>) {
    match rule {
        Rule::MetaVar { name: Some(n), .. } => {
            out.insert(*n);
        }
        Rule::Capture(n) => {
            out.insert(*n);
        }
        Rule::All(v) | Rule::Any(v) => v.iter().for_each(|r| binds(r, out, utils)),
        Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Field { rule, .. } => binds(rule, out, utils),
        Rule::NthChild { of_rule: Some(r), .. } => binds(r, out, utils),
        Rule::Util(i) => out.extend(utils(*i).unwrap_or_default()),
        Rule::Parse(p) => binds(&p.rule, out, utils),
        _ => {}
    }
}

/// binds/reads on compiled rules; utilities are summarised by recompiling-free json walks: the
/// policies' utilities bind only through patterns, so their summary is computed on demand from
/// the already compiled utility rules once the query is compiled. During ordering (before the
/// utilities are compiled) a utility is summarised from its JSON.
fn json_binds(v: &Json, uj: &UtilJson, seen: &mut BTreeSet<usize>, names: &Names) -> BTreeSet<Name> {
    let mut out = BTreeSet::new();
    fn walk(v: &Json, uj: &UtilJson, seen: &mut BTreeSet<usize>, names: &Names, out: &mut BTreeSet<Name>) {
        let Some(o) = v.as_object() else { return };
        for (k, x) in o {
            match k.as_str() {
                "pattern" => {
                    if let Some(p) = x.as_str() {
                        let p = p.trim().trim_start_matches('$');
                        if p != "_" {
                            if let Some(n) = names.get(p) {
                                out.insert(n);
                            }
                        }
                    }
                }
                "capture" => {
                    if let Some(n) = x.as_str().and_then(|c| names.get(c.trim_start_matches('$'))) {
                        out.insert(n);
                    }
                }
                "all" | "any" => {
                    for it in x.as_array().into_iter().flatten() {
                        walk(it, uj, seen, names, out);
                    }
                }
                "has" | "inside" | "parse" => walk(x, uj, seen, names, out),
                "nthChild" => {
                    if let Some(r) = x.get("ofRule") {
                        walk(r, uj, seen, names, out);
                    }
                }
                "matches" => {
                    if let Some(i) = x.as_str().and_then(|id| uj.ids.iter().position(|u| u == id)) {
                        if seen.insert(i) {
                            walk(&uj.json[i], uj, seen, names, out);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    walk(v, uj, seen, names, &mut out);
    out
}

fn order_by_captures(terms: &mut Vec<Rule>, from: usize, _uj: &UtilJson) -> Result<(), String> {
    // The native orders group terms so that a term reading a capture runs after the sibling that
    // binds it. Reads arise only from `not` (what its rule would bind) and `compare`; the policies'
    // `compare` terms come after the group anyway. A `not` among the group reading a sibling's
    // capture is the case to order.
    let util_binds = |_i: usize| -> Option<BTreeSet<Name>> { None };
    let early: BTreeSet<Name> = terms[..from].iter().flat_map(|t| {
        let mut s = BTreeSet::new();
        binds(t, &mut s, &util_binds);
        s
    }).collect();
    let group = &terms[from..];
    let bound: Vec<BTreeSet<Name>> = group.iter().map(|t| { let mut s = BTreeSet::new(); binds(t, &mut s, &util_binds); s }).collect();
    let needs: Vec<BTreeSet<Name>> = group
        .iter()
        .map(|t| reads(t).into_iter().filter(|n| !early.contains(n)).collect())
        .collect();
    let after: Vec<Vec<usize>> = needs
        .iter()
        .enumerate()
        .map(|(i, need)| {
            bound
                .iter()
                .enumerate()
                .filter(|(o, _)| *o != i)
                .filter_map(|(o, b)| need.iter().any(|n| b.contains(n)).then_some(o))
                .collect()
        })
        .collect();
    if after.iter().all(Vec::is_empty) {
        return Ok(());
    }
    if group.iter().any(|t| matches!(t, Rule::Util(_))) {
        return Err("capture ordering through utilities unsupported".into());
    }
    let mut placed = vec![false; group.len()];
    let mut order = Vec::new();
    while order.len() < group.len() {
        let next = (0..group.len())
            .find(|&i| !placed[i] && after[i].iter().all(|&b| placed[b]))
            .ok_or("capture ordering cycle")?;
        placed[next] = true;
        order.push(next);
    }
    let mut taken: Vec<Option<Rule>> = terms.drain(from..).map(Some).collect();
    for i in order {
        terms.push(taken[i].take().unwrap());
    }
    Ok(())
}

/// What a rule reads and does not bind itself (captures.rs `binds_and_reads().1`), without
/// utilities (refused above when they matter).
fn reads(rule: &Rule) -> BTreeSet<Name> {
    fn br(rule: &Rule) -> (BTreeSet<Name>, BTreeSet<Name>) {
        let none = |_i: usize| -> Option<BTreeSet<Name>> { None };
        match rule {
            Rule::MetaVar { .. } | Rule::Capture(_) => {
                let mut b = BTreeSet::new();
                binds(rule, &mut b, &none);
                (b, BTreeSet::new())
            }
            Rule::Compare { left, right, .. } => (BTreeSet::new(), [*left, *right].into_iter().collect()),
            Rule::Any(items) => {
                let (mut b, mut r) = (BTreeSet::new(), BTreeSet::new());
                for it in items {
                    let (ib, ir) = br(it);
                    b.extend(ib);
                    r.extend(ir);
                }
                (b, r)
            }
            Rule::All(items) => {
                let (mut b, mut r) = (BTreeSet::new(), BTreeSet::new());
                for it in items {
                    let (ib, ir) = br(it);
                    r.extend(ir.into_iter().filter(|n| !b.contains(n)));
                    b.extend(ib);
                }
                (b, r)
            }
            Rule::Not(it) => {
                let (mut r, ir) = br(it);
                r.extend(ir);
                (BTreeSet::new(), r)
            }
            Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Field { rule, .. } => br(rule),
            Rule::NthChild { of_rule: Some(r), .. } => br(r),
            Rule::Count { relation, .. } => (BTreeSet::new(), br(relation).1),
            Rule::Parse(p) => br(&p.rule),
            _ => (BTreeSet::new(), BTreeSet::new()),
        }
    }
    br(rule).1
}

fn mentions(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>, out: &mut BTreeSet<Name>) {
    match rule {
        Rule::MetaVar { name: Some(n), .. } | Rule::Capture(n) => {
            out.insert(*n);
        }
        Rule::Compare { left, right, .. } => {
            out.insert(*left);
            out.insert(*right);
        }
        Rule::All(v) | Rule::Any(v) => v.iter().for_each(|r| mentions(r, utils, seen, out)),
        Rule::Not(r) => mentions(r, utils, seen, out),
        Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Field { rule, .. } => mentions(rule, utils, seen, out),
        Rule::NthChild { of_rule: Some(r), .. } => mentions(r, utils, seen, out),
        Rule::Count { relation, .. } => mentions(relation, utils, seen, out),
        Rule::Util(i) => {
            if seen.insert(*i) {
                mentions(&utils[*i], utils, seen, out);
            }
        }
        Rule::Parse(p) => mentions(&p.rule, utils, seen, out),
        _ => {}
    }
}

fn binds_full(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>, out: &mut BTreeSet<Name>) {
    match rule {
        Rule::MetaVar { name: Some(n), .. } | Rule::Capture(n) => {
            out.insert(*n);
        }
        Rule::All(v) | Rule::Any(v) => v.iter().for_each(|r| binds_full(r, utils, seen, out)),
        Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Field { rule, .. } => binds_full(rule, utils, seen, out),
        Rule::NthChild { of_rule: Some(r), .. } => binds_full(r, utils, seen, out),
        Rule::Util(i) => {
            if seen.insert(*i) {
                binds_full(&utils[*i], utils, seen, out);
            }
        }
        Rule::Parse(p) => binds_full(&p.rule, utils, seen, out),
        _ => {}
    }
}

/// Whether a query's rule (through its utilities) can bind any capture.
pub fn binds_any(q: &Query) -> bool {
    let mut out = BTreeSet::new();
    binds_full(&q.rule, &q.utils, &mut BTreeSet::new(), &mut out);
    !out.is_empty()
}

fn reads_values(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>) -> bool {
    match rule {
        Rule::Value(_) | Rule::Parse(_) => true,
        Rule::All(v) | Rule::Any(v) => v.iter().any(|r| reads_values(r, utils, seen)),
        Rule::Not(r) => reads_values(r, utils, seen),
        Rule::Has { rule, stop } | Rule::Inside { rule, stop } | Rule::Field { rule, stop, .. } => {
            reads_values(rule, utils, seen) || matches!(stop, Stop::Rule(s) if reads_values(s, utils, seen))
        }
        Rule::NthChild { of_rule: Some(r), .. } => reads_values(r, utils, seen),
        Rule::Count { relation, .. } => reads_values(relation, utils, seen),
        Rule::Util(i) => seen.insert(*i) && reads_values(&utils[*i], utils, seen),
        _ => false,
    }
}

/// Kinds every match's root must have, when the rule's top-level `all` fixes them.
fn root_kinds(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>) -> Option<Vec<u32>> {
    match rule {
        Rule::Kind(k) => Some(vec![*k]),
        Rule::Literal { kind, .. } => Some(vec![*kind]),
        Rule::All(items) => items.iter().find_map(|r| root_kinds(r, utils, seen)),
        Rule::Any(items) => {
            let mut all = Vec::new();
            for it in items {
                all.extend(root_kinds(it, utils, seen)?);
            }
            all.sort_unstable();
            all.dedup();
            Some(all)
        }
        Rule::Util(i) => {
            if seen.insert(*i) {
                root_kinds(&utils[*i], utils, seen)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluation.

/// A parsed literal's node (`parse`): a `template` root with `literal`/`interpolation` children
/// for pythonFString; spans are host byte ranges; `text` is the node's text (decoded for literals).
#[derive(Clone, Debug)]
pub struct ParsedNode {
    pub kind: u32,
    pub start: u32,
    pub end: u32,
    pub text: String,
    pub parent: u32,
    pub named: bool,
}

pub struct Unsupported(pub String);

/// A node's children: borrowed from the facts' CSR for tree nodes (no allocation); parsed
/// literal trees are small and collected.
pub enum Kids<'a> {
    Node(std::slice::Iter<'a, u32>),
    Parsed(std::vec::IntoIter<Bound>),
}
impl Iterator for Kids<'_> {
    type Item = Bound;
    fn next(&mut self) -> Option<Bound> {
        match self {
            Kids::Node(it) => it.next().map(|&c| Bound::Node(c)),
            Kids::Parsed(it) => it.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Kids::Node(it) => it.size_hint(),
            Kids::Parsed(it) => it.size_hint(),
        }
    }
}
impl ExactSizeIterator for Kids<'_> {}

pub struct Eval<'a> {
    pub f: &'a Facts,
    pub path: &'a str,
    pub q: &'a Query,
    three: bool,
    unknown: std::cell::Cell<bool>,
    /// `violation.each`: the capture enumerated and the ranges reported so far.
    each: std::cell::RefCell<Option<(Name, Vec<(u32, u32)>, bool)>>,
    pub parsed: std::cell::RefCell<Vec<ParsedNode>>,
    parsed_hosts: std::cell::RefCell<rustc_hash::FxHashMap<u32, Option<(u32, u32)>>>,
    pub unsupported: std::cell::RefCell<Option<String>>,
    util_memo: std::cell::RefCell<rustc_hash::FxHashMap<(usize, u32), bool>>,
    util_capture_free: Vec<bool>,
    pub steps: std::cell::Cell<u64>,
}

impl<'a> Eval<'a> {
    pub fn new(f: &'a Facts, path: &'a str, q: &'a Query) -> Self {
        // Verdicts of capture-free utilities are memoised per node, as natively (a node's value
        // is a pure function of the node: `follow` is off in batch 6). Unknown verdicts are not.
        let util_capture_free = (0..q.utils.len())
            .map(|i| {
                let mut out = BTreeSet::new();
                mentions(&q.utils[i], &q.utils, &mut BTreeSet::new(), &mut out);
                out.is_empty()
            })
            .collect();
        Self {
            f,
            path,
            q,
            three: q.three_valued,
            unknown: Default::default(),
            each: Default::default(),
            parsed: Default::default(),
            parsed_hosts: Default::default(),
            unsupported: Default::default(),
            util_memo: Default::default(),
            util_capture_free,
            steps: Default::default(),
        }
    }

    fn refuse(&self, why: String) {
        let mut u = self.unsupported.borrow_mut();
        if u.is_none() {
            *u = Some(why);
        }
    }

    pub fn text_of(&self, b: Bound) -> String {
        match b {
            Bound::Node(n) => self.f.text(n).to_owned(),
            Bound::Parsed(p) => self.parsed.borrow()[p as usize].text.clone(),
        }
    }
    pub fn range_of(&self, b: Bound) -> (u32, u32) {
        match b {
            Bound::Node(n) => (self.f.start[n as usize], self.f.end[n as usize]),
            Bound::Parsed(p) => {
                let pn = &self.parsed.borrow()[p as usize];
                (pn.start, pn.end)
            }
        }
    }
    pub fn kind_of(&self, b: Bound) -> u32 {
        match b {
            Bound::Node(n) => self.f.kind[n as usize],
            Bound::Parsed(p) => self.parsed.borrow()[p as usize].kind,
        }
    }

    /// Candidate roots in preorder.
    pub fn roots(&self) -> Vec<u32> {
        match &self.q.root_kinds {
            Some(kinds) => {
                let mut v: Vec<u32> = kinds.iter().flat_map(|k| self.f.of_kind(*k).iter().copied()).collect();
                if kinds.len() > 1 {
                    v.sort_unstable();
                }
                v
            }
            None => (0..self.f.len() as u32).collect(),
        }
    }

    /// The root's match: its bindings where it matches (a root whose verdict is unknown does not).
    pub fn match_root(&self, node: u32) -> Option<Env> {
        self.unknown.set(false);
        let mut env = Env::default();
        self.m(&self.q.rule, Bound::Node(node), &mut env).then_some(env)
    }

    /// `violation.each` at a matching root: each distinct binding of `name`, as the native
    /// `enumerate` (run.rs) visits them.
    pub fn enumerate(&self, node: u32, first: Env, name: Name, mut visit: impl FnMut(Env)) {
        *self.each.borrow_mut() = Some((name, Vec::new(), true));
        let mut matched = first;
        let mut visited = false;
        loop {
            let fresh = {
                let mut each = self.each.borrow_mut();
                let (_, seen, _) = each.as_mut().unwrap();
                match matched.get(name) {
                    Some(b) => {
                        let r = self.range_of(b);
                        if seen.contains(&r) {
                            false
                        } else {
                            seen.push(r);
                            true
                        }
                    }
                    None => false,
                }
            };
            if fresh || !visited {
                self.suspended(|| visit(matched.clone()));
            }
            visited = true;
            if !fresh {
                break;
            }
            match self.match_root(node) {
                Some(next) => matched = next,
                None => break,
            }
        }
        *self.each.borrow_mut() = None;
    }

    fn each_active(&self) -> bool {
        self.each.borrow().as_ref().is_some_and(|(_, _, active)| *active)
    }
    fn excluded(&self, env: &Env) -> bool {
        let each = self.each.borrow();
        let Some((name, seen, true)) = each.as_ref() else { return false };
        env.get(*name).is_some_and(|b| seen.contains(&self.range_of(b)))
    }
    fn suspended<T>(&self, f: impl FnOnce() -> T) -> T {
        let prev = self.each.borrow().as_ref().map(|e| e.2);
        if let Some(e) = self.each.borrow_mut().as_mut() {
            e.2 = false;
        }
        let out = f();
        if let (Some(p), Some(e)) = (prev, self.each.borrow_mut().as_mut()) {
            e.2 = p;
        }
        out
    }

    // --- node navigation over facts and parsed literals ---
    fn children(&self, b: Bound) -> Kids<'a> {
        match b {
            Bound::Node(n) => Kids::Node(self.f.children(n).iter()),
            Bound::Parsed(p) => {
                let parsed = self.parsed.borrow();
                let kids: Vec<Bound> = parsed
                    .iter()
                    .enumerate()
                    .filter(|(i, pn)| pn.parent == p && *i as u32 != p)
                    .map(|(i, _)| Bound::Parsed(i as u32))
                    .collect();
                Kids::Parsed(kids.into_iter())
            }
        }
    }
    fn parent(&self, b: Bound) -> Option<Bound> {
        match b {
            Bound::Node(n) => self.f.parent(n).map(Bound::Node),
            Bound::Parsed(p) => {
                let parent = self.parsed.borrow()[p as usize].parent;
                (parent != NONE).then_some(Bound::Parsed(parent))
            }
        }
    }
    fn named(&self, b: Bound) -> bool {
        match b {
            Bound::Node(n) => self.f.named(n),
            Bound::Parsed(p) => self.parsed.borrow()[p as usize].named,
        }
    }
    fn descendants_preorder(&self, b: Bound, out: &mut Vec<Bound>) {
        match b {
            Bound::Node(n) => {
                let end = self.f.subtree_end[n as usize];
                out.extend((n + 1..end).map(Bound::Node));
            }
            Bound::Parsed(_) => {
                for c in self.children(b) {
                    out.push(c);
                    self.descendants_preorder(c, out);
                }
            }
        }
    }
    fn field_child(&self, b: Bound, field: u16) -> Option<Bound> {
        match b {
            Bound::Node(n) => self.f.children(n).iter().copied().find(|&c| self.f.field[c as usize] == field).map(Bound::Node),
            Bound::Parsed(_) => None,
        }
    }

    /// A trial: runs `f` on a copy of `env`, committing it when `f` succeeds.
    fn trial(&self, env: &mut Env, f: impl FnOnce(&mut Env) -> bool) -> bool {
        let mut t = env.clone();
        if f(&mut t) {
            *env = t;
            true
        } else {
            false
        }
    }

    pub fn m(&self, rule: &Rule, node: Bound, env: &mut Env) -> bool {
        self.steps.set(self.steps.get() + 1);
        match rule {
            Rule::Kind(k) => self.kind_of(node) == *k && self.named(node),
            Rule::Regex(rx) => match node {
                Bound::Node(n) => rx.is_match(self.f.text(n)),
                Bound::Parsed(p) => rx.is_match(&self.parsed.borrow()[p as usize].text),
            },
            Rule::MetaVar { name, named } => {
                if *named && !self.named(node) {
                    return false;
                }
                let Some(name) = name else { return true };
                match env.get(*name) {
                    None => {
                        env.set(*name, node);
                        true
                    }
                    Some(prev) => {
                        if self.match_exactly(prev, node) {
                            env.set(*name, node);
                            true
                        } else {
                            false
                        }
                    }
                }
            }
            Rule::Literal { kind, text } => {
                self.kind_of(node) == *kind && self.named(node) && self.text_of(node) == *text
            }
            Rule::Capture(name) => match env.get(*name) {
                None => {
                    env.set(*name, node);
                    true
                }
                Some(prev) => {
                    if self.match_exactly(prev, node) {
                        env.set(*name, node);
                        true
                    } else {
                        false
                    }
                }
            },
            Rule::All(items) => self.match_all(items, node, env),
            Rule::Any(items) => self.match_any(items, node, env),
            Rule::Not(inner) => self.match_not(inner, node, env),
            Rule::Util(i) => self.util(*i, node, env),
            Rule::FilePath { regex, not_regex } => {
                regex.as_ref().is_none_or(|r| r.is_match(self.path))
                    && not_regex.as_ref().is_none_or(|r| !r.is_match(self.path))
            }
            Rule::Has { rule, stop } => self.has(node, rule, stop, env),
            Rule::Inside { rule, stop } => self.inside(node, rule, stop, env),
            Rule::Field { inside, field, rule, stop } => {
                if *inside {
                    self.field_inside(node, *field, rule, stop, env)
                } else {
                    self.field_has(node, *field, rule, stop, env)
                }
            }
            Rule::NthChild { step, offset, of_rule, reverse } => self.nth_child(node, *step, *offset, of_rule.as_deref(), *reverse, env),
            Rule::Value(test) => match self.value_holds(test, node) {
                Some(v) => v,
                None => {
                    self.unknown.set(true);
                    false
                }
            },
            Rule::Compare { left, right, op, text } => {
                let (Some(l), Some(r)) = (env.get(*left), env.get(*right)) else { return false };
                let eq = match text {
                    CompareText::Source => self.text_of(l) == self.text_of(r),
                    CompareText::Node => self.range_of(l) == self.range_of(r) && self.kind_of(l) == self.kind_of(r),
                };
                match op {
                    CompareOp::Eq => eq,
                    CompareOp::Ne => !eq,
                }
            }
            Rule::Count { relation, min, max } => self.count(node, relation, *min, *max, env),
            Rule::Parse(p) => self.parse(node, p, env),
            Rule::Never => false,
        }
    }

    fn match_exactly(&self, a: Bound, b: Bound) -> bool {
        if a == b {
            return true;
        }
        let leaf = |x: Bound| self.named(x) && self.children(x).len() == 0;
        if leaf(a) || leaf(b) {
            return self.text_of(a) == self.text_of(b);
        }
        if self.kind_of(a) != self.kind_of(b) {
            return false;
        }
        // ast-grep compares kind ids; kind symbols coincide with ids for named/anonymous pairs
        // only up to the named flag.
        if self.named(a) != self.named(b) {
            return false;
        }
        let (ca, cb) = (self.children(a), self.children(b));
        ca.len() == cb.len() && ca.zip(cb).all(|(x, y)| self.match_exactly(x, y))
    }

    fn match_all(&self, items: &[Rule], node: Bound, env: &mut Env) -> bool {
        let mut trial = env.clone();
        let mut unknown = false;
        let mut withheld: Option<BTreeSet<Name>> = None;
        for item in items {
            if let Some(names) = withheld.as_mut() {
                let mut m = BTreeSet::new();
                mentions(item, &self.q.utils, &mut BTreeSet::new(), &mut m);
                if !m.is_disjoint(names) {
                    let mut b = BTreeSet::new();
                    binds_full(item, &self.q.utils, &mut BTreeSet::new(), &mut b);
                    names.extend(b);
                    continue;
                }
            }
            if !self.m(item, node, &mut trial) {
                if self.three && self.unknown.get() {
                    self.unknown.set(false);
                    unknown = true;
                    let mut b = BTreeSet::new();
                    binds_full(item, &self.q.utils, &mut BTreeSet::new(), &mut b);
                    if !b.is_empty() {
                        withheld.get_or_insert_default().extend(b);
                    }
                    continue;
                }
                return false;
            }
        }
        if unknown {
            self.unknown.set(true);
            return false;
        }
        *env = trial;
        true
    }

    fn match_any(&self, items: &[Rule], node: Bound, env: &mut Env) -> bool {
        let three = self.three;
        let saved = three && self.unknown.replace(false);
        let mut unknown = false;
        for item in items {
            let mut trial = env.clone();
            if self.m(item, node, &mut trial) && !(self.each_active() && self.excluded(&trial)) {
                if three {
                    self.unknown.set(saved);
                }
                *env = trial;
                return true;
            }
            if three && self.unknown.replace(false) {
                unknown = true;
                if self.may_bind(item) {
                    break;
                }
            }
        }
        if three {
            self.unknown.set(saved || unknown);
        }
        false
    }

    fn may_bind(&self, rule: &Rule) -> bool {
        let mut b = BTreeSet::new();
        binds_full(rule, &self.q.utils, &mut BTreeSet::new(), &mut b);
        !b.is_empty()
    }

    fn match_not(&self, inner: &Rule, node: Bound, env: &mut Env) -> bool {
        let three = self.three;
        let saved = three && self.unknown.replace(false);
        let mut probe = env.clone();
        let matched = self.suspended(|| self.m(inner, node, &mut probe));
        if matched {
            if three {
                self.unknown.set(saved);
            }
            false
        } else if three && self.unknown.replace(saved) {
            self.unknown.set(true);
            false
        } else {
            true
        }
    }

    fn util(&self, i: usize, node: Bound, env: &mut Env) -> bool {
        let memo_key = match node {
            Bound::Node(n) if self.util_capture_free[i] && !self.each_active() => Some((i, n)),
            _ => None,
        };
        if let Some(k) = memo_key {
            if let Some(&v) = self.util_memo.borrow().get(&k) {
                return v;
            }
        }
        // Whether this evaluation itself was unknown decides memoization, not an unknown the
        // caller had already accumulated; the caller's flag is restored on top.
        let saved = self.three && self.unknown.replace(false);
        let r = self.m(&self.q.utils[i], node, env);
        if let Some(k) = memo_key {
            if !(self.three && self.unknown.get()) {
                self.util_memo.borrow_mut().insert(k, r);
            }
        }
        if saved {
            self.unknown.set(true);
        }
        r
    }

    // --- relations (relation.rs) ---

    /// One candidate's test in a relation search. `Some(true)` = found (env committed),
    /// `Some(false)` = keep searching, `None` = strict search must end.
    fn search_test(&self, rule: &Rule, cand: Bound, env: &mut Env, s: &mut Search) -> Option<bool> {
        if s.each {
            let mut trial = env.clone();
            if self.m(rule, cand, &mut trial) {
                if self.excluded(&trial) {
                    return Some(false);
                }
                *env = trial;
                return Some(true);
            }
        } else {
            let mut trial = env.clone();
            if self.m(rule, cand, &mut trial) {
                *env = trial;
                return Some(true);
            }
        }
        if s.three && self.unknown.replace(false) {
            return self.search_open(rule, s);
        }
        Some(false)
    }
    fn search_open(&self, rule: &Rule, s: &mut Search) -> Option<bool> {
        s.unknown = true;
        if *s.strict.get_or_insert_with(|| self.may_bind(rule)) {
            None
        } else {
            Some(false)
        }
    }
    fn search_start(&self) -> Search {
        let three = self.three;
        Search {
            three,
            each: self.each_active(),
            saved: three && self.unknown.replace(false),
            unknown: false,
            strict: None,
        }
    }
    fn search_finish(&self, s: Search, found: bool) -> bool {
        if !s.three {
            return found;
        }
        if found {
            self.unknown.set(s.saved);
        } else {
            self.unknown.set(s.saved || s.unknown);
        }
        found
    }
    /// Whether a stop rule matches (`None` unknown). Fresh environment, `each` suspended.
    fn stops_at(&self, stop: &Rule, node: Bound) -> Option<bool> {
        self.suspended(|| {
            let mut fresh = Env::default();
            if !self.three {
                return Some(self.m(stop, node, &mut fresh));
            }
            let saved = self.unknown.replace(false);
            let stopped = self.m(stop, node, &mut fresh);
            let unknown = self.unknown.replace(saved);
            match (stopped, unknown) {
                (true, _) => Some(true),
                (false, false) => Some(false),
                (false, true) => None,
            }
        })
    }

    /// `StopBy::find` over `once` (the neighbour) or `all` (the axis walk).
    fn find(&self, stop: &Stop, once: Option<Bound>, all: &[Bound], rule: &Rule, env: &mut Env) -> bool {
        let mut s = self.search_start();
        let found = match stop {
            Stop::Neighbor => match once {
                Some(c) => self.search_test(rule, c, env, &mut s) == Some(true),
                None => false,
            },
            Stop::End => {
                let mut found = false;
                for &c in all {
                    match self.search_test(rule, c, env, &mut s) {
                        Some(true) => {
                            found = true;
                            break;
                        }
                        Some(false) => {}
                        None => break,
                    }
                }
                found
            }
            Stop::Rule(stop) => {
                let mut found = false;
                for &c in all {
                    let stopped = self.stops_at(stop, c);
                    match self.search_test(rule, c, env, &mut s) {
                        Some(true) => {
                            found = true;
                            break;
                        }
                        None => break,
                        Some(false) => {}
                    }
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            let _ = self.search_open(rule, &mut s);
                            break;
                        }
                    }
                }
                found
            }
        };
        self.search_finish(s, found)
    }

    fn ancestors(&self, node: Bound) -> Vec<Bound> {
        let mut v = Vec::new();
        let mut cur = self.parent(node);
        while let Some(p) = cur {
            v.push(p);
            cur = self.parent(p);
        }
        v
    }

    fn inside(&self, node: Bound, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
        match stop {
            Stop::Neighbor => self.find(stop, self.parent(node), &[], rule, env),
            _ => {
                let anc = self.ancestors(node);
                self.find(stop, None, &anc, rule, env)
            }
        }
    }

    fn has(&self, node: Bound, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
        let mut s = self.search_start();
        let found = match stop {
            Stop::Neighbor => {
                let mut found = false;
                for c in self.children(node) {
                    match self.search_test(rule, c, env, &mut s) {
                        Some(true) => {
                            found = true;
                            break;
                        }
                        Some(false) => {}
                        None => break,
                    }
                }
                found
            }
            Stop::End => {
                let mut all = Vec::new();
                self.descendants_preorder(node, &mut all);
                let mut found = false;
                for c in all {
                    match self.search_test(rule, c, env, &mut s) {
                        Some(true) => {
                            found = true;
                            break;
                        }
                        Some(false) => {}
                        None => break,
                    }
                }
                found
            }
            Stop::Rule(stop) => self.has_until(node, rule, stop, env, &mut s),
        };
        self.search_finish(s, found)
    }

    /// `has` with a stop rule: pre-order; a descendant the stop rule matches is tested and not
    /// entered; an unknown stop verdict is not entered either and leaves the answer open.
    fn has_until(&self, node: Bound, rule: &Rule, stop: &Rule, env: &mut Env, s: &mut Search) -> bool {
        let mut stack = vec![self.children(node).into_iter()];
        while let Some(level) = stack.last_mut() {
            let Some(child) = level.next() else {
                stack.pop();
                continue;
            };
            match self.search_test(rule, child, env, s) {
                Some(true) => return true,
                None => return false,
                Some(false) => {}
            }
            match self.stops_at(stop, child) {
                Some(false) => stack.push(self.children(child).into_iter()),
                Some(true) => {}
                None => {
                    let _ = self.search_open(rule, s);
                    return false;
                }
            }
        }
        false
    }

    /// `inside` with a field: an ancestor is tested only where the walk entered it through that
    /// field (relation.rs `find_inside`).
    fn field_inside(&self, node: Bound, field: u16, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
        let mut s = self.search_start();
        let mut previous = node;
        let mut found = false;
        let mut cur = self.parent(node);
        while let Some(anc) = cur {
            let entered = self.field_child(anc, field) == Some(previous);
            let stopped = match stop {
                Stop::Rule(stop) => Some(self.stops_at(stop, anc)),
                _ => None,
            };
            if entered {
                match self.search_test(rule, anc, env, &mut s) {
                    Some(true) => {
                        found = true;
                        break;
                    }
                    None => break,
                    Some(false) => {}
                }
            }
            match stop {
                Stop::Neighbor => break,
                Stop::End => {}
                Stop::Rule(_) => match stopped.unwrap() {
                    Some(false) => {}
                    Some(true) => break,
                    None => {
                        let _ = self.search_open(rule, &mut s);
                        break;
                    }
                },
            }
            previous = anc;
            cur = self.parent(anc);
        }
        self.search_finish(s, found)
    }

    /// `has` with a field: the first child with that field; `neighbor` tests it alone, `end` it
    /// and its descendants, a stop rule it and (unless it stops) its direct children.
    fn field_has(&self, node: Bound, field: u16, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
        let Some(child) = self.field_child(node, field) else {
            return false;
        };
        let mut s = self.search_start();
        let mut found = false;
        match stop {
            Stop::Neighbor => {
                found = self.search_test(rule, child, env, &mut s) == Some(true);
            }
            Stop::End => {
                let mut all = vec![child];
                self.descendants_preorder(child, &mut all);
                for c in all {
                    match self.search_test(rule, c, env, &mut s) {
                        Some(true) => {
                            found = true;
                            break;
                        }
                        None => break,
                        Some(false) => {}
                    }
                }
            }
            Stop::Rule(stop) => match self.search_test(rule, child, env, &mut s) {
                Some(true) => found = true,
                None => {}
                Some(false) => match self.stops_at(stop, child) {
                    Some(true) => {}
                    None => {
                        let _ = self.search_open(rule, &mut s);
                    }
                    Some(false) => {
                        for c in self.children(child) {
                            match self.search_test(rule, c, env, &mut s) {
                                Some(true) => {
                                    found = true;
                                    break;
                                }
                                None => break,
                                Some(false) => {}
                            }
                        }
                    }
                },
            },
        }
        self.search_finish(s, found)
    }

    fn nth_child(&self, node: Bound, step: i32, offset: i32, of_rule: Option<&Rule>, reverse: bool, env: &mut Env) -> bool {
        let Some(parent) = self.parent(node) else { return false };
        let named: Vec<Bound> = self.children(parent).filter(|c| self.named(*c)).collect();
        let counted: Vec<Bound> = match of_rule {
            None => named,
            Some(rule) => {
                let three = self.three;
                let saved = three && self.unknown.replace(false);
                let mut counted = Vec::new();
                let mut unknown = false;
                for c in named {
                    let mut probe = env.clone();
                    let matched = self.suspended(|| self.m(rule, c, &mut probe));
                    if matched {
                        counted.push(c);
                    } else if three && self.unknown.replace(false) {
                        unknown = true;
                        break;
                    }
                }
                if unknown {
                    self.unknown.set(true);
                    return false;
                }
                if three {
                    self.unknown.set(saved);
                }
                counted
            }
        };
        let Some(position) = counted.iter().position(|c| *c == node) else { return false };
        let index = if reverse { counted.len() - 1 - position } else { position };
        if let Some(rule) = of_rule {
            self.m(rule, node, env);
        }
        let index = index as i64 + 1;
        let (step, offset) = (i64::from(step), i64::from(offset));
        if step == 0 {
            index == offset
        } else {
            let n = index - offset;
            n / step >= 0 && n % step == 0
        }
    }

    fn count(&self, node: Bound, relation: &Rule, min: usize, max: Option<usize>, env: &mut Env) -> bool {
        let three = self.three;
        let saved = three && self.unknown.replace(false);
        let mut found = 0usize;
        let mut unknown = 0usize;
        let mut open = false;
        let mut test = |this: &Self, rule: &Rule, c: Bound| -> bool {
            let mut probe = env.clone();
            if this.m(rule, c, &mut probe) {
                found += 1;
                if max.is_some_and(|m| found > m) || (max.is_none() && found >= min) {
                    return false;
                }
            } else if three && this.unknown.replace(false) {
                unknown += 1;
            }
            true
        };
        match relation {
            Rule::Has { rule, stop } => match stop {
                Stop::Neighbor => {
                    for c in self.children(node) {
                        if !test(self, rule, c) {
                            break;
                        }
                    }
                }
                Stop::End => {
                    let mut all = Vec::new();
                    self.descendants_preorder(node, &mut all);
                    for c in all {
                        if !test(self, rule, c) {
                            break;
                        }
                    }
                }
                Stop::Rule(stop) => {
                    let mut stack = vec![self.children(node).into_iter()];
                    'walk: while let Some(level) = stack.last_mut() {
                        let Some(child) = level.next() else {
                            stack.pop();
                            continue;
                        };
                        if !test(self, rule, child) {
                            break 'walk;
                        }
                        match self.stops_at(stop, child) {
                            Some(false) => stack.push(self.children(child).into_iter()),
                            Some(true) => {}
                            None => open = true,
                        }
                    }
                }
            },
            Rule::Inside { rule, stop } => {
                let walk: Vec<Bound> = match stop {
                    Stop::Neighbor => self.parent(node).into_iter().collect(),
                    _ => self.ancestors(node),
                };
                for c in walk {
                    let stopped = match stop {
                        Stop::Rule(s) => self.stops_at(s, c),
                        _ => Some(false),
                    };
                    if !test(self, rule, c) {
                        break;
                    }
                    match stopped {
                        Some(false) => {}
                        Some(true) => break,
                        None => {
                            open = true;
                            break;
                        }
                    }
                }
            }
            _ => {
                if three {
                    self.unknown.set(saved);
                }
                return false;
            }
        }
        if three {
            self.unknown.set(saved);
        }
        let high = (!open).then_some(found + unknown);
        let holds = found >= min && max.is_none_or(|m| high.is_some_and(|h| h <= m));
        if holds {
            return true;
        }
        let fails = high.is_some_and(|h| h < min) || max.is_some_and(|m| found > m);
        if !fails && three {
            self.unknown.set(true);
        }
        false
    }

    // --- values (matcher/value.rs) ---
    fn value_holds(&self, test: &ValueTest, node: Bound) -> Option<bool> {
        let value = match node {
            Bound::Node(n) => {
                let kind = facts::name(self.f.kind[n as usize]);
                if !pyvalue::EXPRESSION_KINDS.contains(&&*kind) || !self.f.named(n) {
                    return Some(false);
                }
                match pyvalue::decode(self.f, n) {
                    Ok(v) => v,
                    Err(why) => {
                        self.refuse(why);
                        return Some(false);
                    }
                }
            }
            Bound::Parsed(p) => {
                // A parsed literal's `literal` node has its decoded text as a str value.
                let pn = self.parsed.borrow()[p as usize].clone();
                if &*facts::name(pn.kind) == "literal" {
                    PyValue::Str(pn.text)
                } else {
                    return Some(false);
                }
            }
        };
        if matches!(value, PyValue::Unknown) {
            return None;
        }
        if let Some(eq) = &test.eq {
            if !matches!(&value, PyValue::Str(s) if s == eq) {
                return Some(false);
            }
        }
        if let Some(rx) = &test.regex {
            let ok = match &value {
                PyValue::Str(s) => rx.is_match(s),
                _ => false,
            };
            if !ok {
                return Some(false);
            }
        }
        Some(true)
    }

    // --- parse (matcher/parse.rs): pythonFString only ---
    fn parse(&self, node: Bound, p: &ParseTest, env: &mut Env) -> bool {
        let Bound::Node(n) = node else { return false };
        match p.grammar {
            ParseGrammar::Annotation => {
                self.refuse("parse grammar `annotation` (Ruff string type definitions) is not in schema v0".into());
                false
            }
            ParseGrammar::PythonFString => {
                let kind = facts::name(self.f.kind[n as usize]);
                if &*kind != "string" && &*kind != "concatenated_string" {
                    return false;
                }
                // Cached per host node: Some(root) for a tree, None for NotString, root NONE for
                // an unknown value.
                let root = {
                    let cached = self.parsed_hosts.borrow().get(&n).copied();
                    match cached {
                        Some(r) => r,
                        None => {
                            let r = match crate::fstring::parse(self.f, n) {
                                Ok(crate::fstring::FParse::Tree(pieces)) => {
                                    let mut parsed = self.parsed.borrow_mut();
                                    let root = parsed.len() as u32;
                                    let text: String = pieces.iter().map(|p| p.text.as_str()).collect();
                                    let (s, e) = match (pieces.first(), pieces.last()) {
                                        (Some(a), Some(b)) => (a.start, b.end),
                                        _ => (self.f.start[n as usize], self.f.start[n as usize]),
                                    };
                                    parsed.push(ParsedNode { kind: facts::intern("template"), start: s, end: e, text, parent: NONE, named: true });
                                    for piece in pieces {
                                        parsed.push(ParsedNode { kind: facts::intern(piece.kind), start: piece.start, end: piece.end, text: piece.text, parent: root, named: true });
                                    }
                                    Some((root, parsed.len() as u32))
                                }
                                Ok(crate::fstring::FParse::NotString) => None,
                                Ok(crate::fstring::FParse::Unknown) => Some((NONE, NONE)),
                                Err(why) => {
                                    self.refuse(why);
                                    None
                                }
                            };
                            self.parsed_hosts.borrow_mut().insert(n, r);
                            r
                        }
                    }
                };
                let Some((root, _)) = root else { return false };
                if root == NONE {
                    self.unknown.set(true);
                    return false;
                }
                // The tree's rule runs in an environment of its own; what it binds is exported to
                // the host as span captures.
                let mut inner = Env::default();
                if !self.m(&p.rule, Bound::Parsed(root), &mut inner) {
                    return false;
                }
                for (name, b) in inner.vars {
                    env.set(name, b);
                }
                true
            }
        }
    }
}

struct Search {
    three: bool,
    each: bool,
    saved: bool,
    unknown: bool,
    strict: Option<bool>,
}
