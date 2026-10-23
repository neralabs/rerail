//! The AST query language of rerail's DSL (the subset the data-layer policies use), evaluated over
//! per-file facts. Semantics mirror the native matcher (ast-grep-config 0.45.3 plus rerail's
//! extensions): every node is a candidate root in preorder; a root yields at most one match, the
//! first-success binding with no backtracking; relations bind from the first related node that
//! matches; verdicts are three-valued in a clause that reads values.
mod arguments;
mod layout;
mod parse_ext;
mod pattern;
pub mod lines;

use crate::facts::{self, Facts, NONE};
use crate::pyvalue::{self, PyValue};
use regex::Regex;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

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
    /// `precedes`: a later sibling (native `StopBy::find_sibling`, forward).
    Precedes { rule: Box<Rule>, stop: Stop },
    /// `follows`: an earlier sibling (native `StopBy::find_sibling`, backward).
    Follows { rule: Box<Rule>, stop: Stop },
    Field { inside: bool, field: u16, rule: Box<Rule>, stop: Stop },
    Util(usize),
    /// `matches` with parameters (native `UtilCall`): the utility runs in a frame of its own;
    /// `args` are (parameter, caller's capture) in the utility's parameter order.
    Call { util: usize, args: Vec<(Name, Name)> },
    FilePath { regex: Option<Regex>, not_regex: Option<Regex> },
    NthChild { step: i32, offset: i32, of_rule: Option<Box<Rule>>, reverse: bool },
    Value(ValueTest),
    /// `transform`: a side names a variable of the query's `transform` (native
    /// `Comparison::transform`).
    Compare { left: Name, right: Name, op: CompareOp, text: CompareText, transform: bool },
    /// `relation` is `Has`, `Inside`, `Precedes` or `Follows`; `binding` is `as` with `offset`.
    Count { relation: Box<Rule>, min: usize, max: Option<usize>, binding: Option<(Name, i64)> },
    /// `layout` (ast/layout.rs).
    Layout(Box<layout::Layout>),
    /// A structural pattern (ast/pattern.rs); metavariables and single names compile to
    /// `MetaVar`/`Literal` above.
    Pattern(Box<pattern::Pattern>),
    /// `arguments` (ast/arguments.rs): binds call arguments to signature slots.
    Arguments(Box<arguments::Arguments>),
    /// `host` inside a text grammar's tree rule (ast/parse_ext.rs); binds nothing.
    Host(Box<parse_ext::Host>),
    Parse(Box<ParseTest>),
    /// `binds` without usage filters or sub-rules (bindings.rs): the node names a binding that
    /// passes the filters (`py_binding`); binds nothing, reads no values.
    Binds(Box<crate::bindings::Filter>),
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
    /// Each capture's index among the elements of the list holding it (native
    /// `CompareText::Index`, compare.rs:287-337).
    Index,
}

#[derive(Clone)]
pub struct ValueTest {
    /// `type`: Python type names; only `str` and `bytes` (the value decoder tells only those).
    pub types: Vec<String>,
    pub eq: Option<String>,
    pub regex: Option<Regex>,
}

#[derive(Clone)]
pub struct ParseTest {
    pub grammar: ParseGrammar,
    pub rule: Rule,
    /// pythonFString's `interpolation` marker (default `{}`).
    pub marker: String,
    /// The tree rule holds a `host` (parse_ext.rs `compile_tree_rule`).
    pub hosted: bool,
}
#[derive(Clone, Copy, PartialEq)]
pub enum ParseGrammar {
    PythonFString,
    IdentifierWords,
    Annotation,
}

/// A capture binding: a node of the file, or a node of a parsed literal (`parse`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    Node(u32),
    /// Index into the file's parsed-literal side table.
    Parsed(u32),
}

/// Bindings: captures, `$$$` lists, and text variables (ast-grep's transformed variables: a
/// count's `as`).
#[derive(Clone, Default, Debug)]
pub struct Env {
    pub vars: Vec<(Name, Bound)>,
    pub multi: Vec<(Name, Vec<Bound>)>,
    pub texts: Vec<(Name, String)>,
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
    pub fn text(&self, name: Name) -> Option<&str> {
        self.texts.iter().find(|(n, _)| *n == name).map(|(_, t)| t.as_str())
    }
    fn set_text(&mut self, name: Name, text: String) {
        if let Some(slot) = self.texts.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = text;
        } else {
            self.texts.push((name, text));
        }
    }
}

/// One `transform` variable: `replace` over its source's text (native `Trans::Replace`), or the
/// source's decoded value (`value: true`, only in the `DECODED` shape `decoded` lists).
pub struct TransformStep {
    pub name: Name,
    pub source: Name,
    pub default: Option<String>,
    pub value: bool,
    pub regex: Regex,
    pub by: String,
}

/// The query's transforms, ordered so each source is computed before it is read.
#[derive(Default)]
pub struct Transform {
    pub steps: Vec<TransformStep>,
}

impl Transform {
    fn contains(&self, name: Name) -> bool {
        self.steps.iter().any(|s| s.name == name)
    }
    /// Indices of the steps `name` is computed from, itself included (native `Transform::chain`).
    fn chain(&self, name: Name) -> BTreeSet<usize> {
        let mut chain = BTreeSet::new();
        let mut next = Some(name);
        while let Some(key) = next.take() {
            let Some(i) = self.steps.iter().position(|s| s.name == key) else { break };
            if chain.insert(i) {
                next = Some(self.steps[i].source);
            }
        }
        chain
    }
    /// The captures `name` is computed from (native `Transform::roots`).
    fn roots(&self, name: Name) -> BTreeSet<Name> {
        self.chain(name)
            .into_iter()
            .map(|i| self.steps[i].source)
            .filter(|s| !self.contains(*s))
            .collect()
    }
}

/// `regex.replace_all(text, by)` with `$1`/`${name}`/`$$` expansion, as native `replace_bounded`.
fn replace_all(text: &str, regex: &Regex, by: &str) -> String {
    let mut output = String::new();
    let mut end = 0;
    for captures in regex.captures_iter(text) {
        let matched = captures.get(0).expect("whole match");
        output.push_str(&text[end..matched.start()]);
        let mut cursor = 0;
        while let Some(dollar) = by[cursor..].find('$') {
            let dollar = cursor + dollar;
            output.push_str(&by[cursor..dollar]);
            let reference = dollar + 1;
            let Some(first) = by[reference..].chars().next() else {
                output.push('$');
                cursor = reference;
                break;
            };
            if first == '$' {
                output.push('$');
                cursor = reference + 1;
                continue;
            }
            let (name, next) = if first == '{' {
                let body = reference + 1;
                let Some(close) = by[body..].find('}') else {
                    output.push('$');
                    cursor = reference;
                    continue;
                };
                (&by[body..body + close], body + close + 1)
            } else if first.is_ascii_alphanumeric() || first == '_' {
                let next = by[reference..]
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .map_or(by.len(), |o| reference + o);
                (&by[reference..next], next)
            } else {
                output.push('$');
                cursor = reference;
                continue;
            };
            let capture = name.parse::<usize>().ok().and_then(|i| captures.get(i)).or_else(|| captures.name(name));
            if let Some(c) = capture {
                output.push_str(c.as_str());
            }
            cursor = next;
        }
        if cursor < by.len() {
            output.push_str(&by[cursor..]);
        }
        end = matched.end();
    }
    output.push_str(&text[end..]);
    output
}

/// A policy set's compiled utility table: rules, per utility whether it mentions no capture
/// (its verdict is a function of the node), and the capture summaries.
#[derive(Clone)]
pub struct UtilTable {
    pub utils: Arc<[Rule]>,
    pub capture_free: Arc<[bool]>,
    pub summary: Arc<Summary>,
}

/// A compiled `ast` query.
pub struct Query {
    pub rule: Rule,
    /// The utility table, shared by the policy set's queries without local utilities.
    pub utils: Arc<[Rule]>,
    /// Per utility: it mentions no capture (its verdict is a function of the node).
    pub util_capture_free: Arc<[bool]>,
    pub summary: Arc<Summary>,
    pub transform: Arc<Transform>,
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
    /// Per utility: its parameters (`{ params, rule }`), empty for a plain utility.
    util_params: Vec<Vec<String>>,
    /// The policy set's compiled utility table, shared by every query without local utilities
    /// or transforms (set by the first such query; callers carry it across compilers of one
    /// policy set).
    pub shared: Option<UtilTable>,
    /// What capture ordering reads of the utilities while compiling: none yet (native's unfilled
    /// table), a draft, or the final table.
    summary: Arc<Summary>,
    transform: Arc<Transform>,
    /// The call's option values (`policy::option_values`), for `ast.when`.
    pub options: Option<BTreeMap<String, Json>>,
}

fn obj<'j>(v: &'j Json, at: &str) -> Result<&'j serde_json::Map<String, Json>, String> {
    v.as_object().ok_or_else(|| format!("{at}: expected object"))
}

const RULE_KEYS: &[&str] = &[
    "pattern", "kind", "regex", "capture", "parse", "nthChild", "filePath", "all", "any", "not",
    "matches", "inside", "has", "precedes", "follows", "count", "compare", "layout", "value",
    "stopBy", "field", "arguments", "host", "binds",
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
            util_params: Vec::new(),
            shared: None,
            summary: Arc::default(),
            transform: Arc::default(),
            options: None,
        }
    }

    pub fn query(&mut self, ast: &Json) -> Result<Query, String> {
        let ast = obj(ast, "ast")?;
        for k in ast.keys() {
            if !["language", "syntax", "rule", "utils", "transform", "file", "when"].contains(&k.as_str()) {
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
        // A utility is a rule, or `{ params, rule }` (native compile/utils.rs:54-110).
        self.util_params = Vec::with_capacity(self.util_json.len());
        for (i, j) in self.util_json.iter().enumerate() {
            let params = match j.as_object() {
                Some(o) if o.contains_key("params") => {
                    let at = format!("utils/{}", self.util_ids[i]);
                    if o.keys().any(|k| k != "params" && k != "rule") {
                        return Err(format!("{at}: a parameterised utility holds params and rule only"));
                    }
                    let names = o["params"].as_array().filter(|a| !a.is_empty()).ok_or_else(|| format!("{at}/params must be a non-empty array"))?;
                    let mut params: Vec<String> = Vec::new();
                    for p in names {
                        let p = p.as_str().filter(|p| valid_meta(p)).ok_or_else(|| format!("{at}/params: capture names"))?;
                        if params.iter().any(|q| q == p) {
                            return Err(format!("{at}/params: `{p}` listed twice"));
                        }
                        params.push(p.to_owned());
                    }
                    params
                }
                _ => Vec::new(),
            };
            self.util_params.push(params);
        }
        let syntax = match ast.get("syntax").and_then(Json::as_str) {
            Some("valid") => SyntaxMode::Valid,
            Some("invalid") => SyntaxMode::Invalid,
            None => SyntaxMode::Unspecified,
            Some(o) => return Err(format!("syntax {o}")),
        };
        let (transform, decoded) = self.transform(ast.get("transform"))?;
        let transform = Arc::new(transform);
        self.transform = transform.clone();
        // Utilities, compiled in id order (indices are stable). Without query-local utilities
        // or transforms every query of a policy set has the same table, compiled once and
        // shared (`shared`). As native (compile/mod.rs:1107-1126), utilities compile twice: first
        // with no utility readable by the capture ordering, then against that draft. (Native
        // recompiles only when the first pass read an unfilled utility; a pass that read none
        // orders alike against any table, so always recompiling is the same.)
        let local = ast.get("utils").and_then(Json::as_object).is_some_and(|o| !o.is_empty()) || !transform.steps.is_empty();
        let table = match (&self.shared, local) {
            (Some(s), false) => s.clone(),
            _ => {
                self.summary = Arc::new(Summary::of(&[], transform.clone()));
                let draft = self.compile_utils()?;
                self.summary = Arc::new(Summary::of(&draft, transform.clone()));
                let utils = self.compile_utils()?;
                let summary = Arc::new(Summary::of(&utils, transform.clone()));
                // Verdicts of capture-free utilities are memoised per node (`Eval::util`).
                let free: Vec<bool> = utils.iter().map(|u| summary.mentions(u).is_empty()).collect();
                let t = UtilTable { utils: utils.into(), capture_free: free.into(), summary };
                if !local {
                    self.shared = Some(t.clone());
                }
                t
            }
        };
        self.summary = table.summary.clone();
        // `file: { path }` with `syntax` set: the rule `all: [{ filePath }, rule]` (the
        // file-path-gate rewrite, docs/facts/REWRITES.md; rerail_facts `file_path_gate`).
        let mut rule_json = ast.get("rule").ok_or("ast.rule missing")?.clone();
        if let Some(file) = ast.get("file") {
            let fo = obj(file, "ast.file")?;
            if fo.len() != 1 || !fo.contains_key("path") {
                return Err("ast.file: only `path` (module and package conditions read other files' facts)".into());
            }
            if !ast.contains_key("syntax") {
                return Err("ast.file without `syntax` (natively a gated-out file reports no partial-source failure)".into());
            }
            rule_json = serde_json::json!({ "all": [{ "filePath": fo["path"] }, rule_json] });
        }
        // `when` (native `compile_python_when`): a query whose condition is false reads no file.
        let applies = match ast.get("when") {
            None => true,
            Some(w) => {
                let options = self.options.as_ref().ok_or("ast.when: the call's options are not known here")?;
                rerail_facts::front::rule::when_holds(w, options)?
            }
        };
        let rule = self.rule(&rule_json, "rule")?;
        let rule = if applies { rule } else { Rule::Never };
        let three_valued = reads_values(&rule, &table.utils, &mut BTreeSet::new());
        let root_kinds = if applies { root_kinds(&rule, &table.utils, &mut BTreeSet::new()) } else { Some(Vec::new()) };
        Ok(Query {
            rule,
            utils: table.utils,
            util_capture_free: table.capture_free,
            summary: table.summary,
            transform,
            syntax,
            three_valued,
            decoded,
            root_kinds,
        })
    }

    fn compile_utils(&mut self) -> Result<Vec<Rule>, String> {
        let mut utils = Vec::with_capacity(self.util_json.len());
        for i in 0..self.util_json.len() {
            let j = self.util_json[i].clone();
            let at = format!("utils/{}", self.util_ids[i]);
            let body = if self.util_params[i].is_empty() { &j } else { j.get("rule").ok_or_else(|| format!("{at}/rule missing"))? };
            utils.push(self.rule(body, &at)?);
        }
        Ok(utils)
    }

    /// `transform` (native compile/transform.rs): `replace` steps only. Returns the steps in an
    /// order where each source is computed before it is read, and the `DECODED` shapes.
    fn transform(&mut self, t: Option<&Json>) -> Result<(Transform, Vec<(Name, Name)>), String> {
        let mut steps = Vec::new();
        let mut decoded = Vec::new();
        let Some(t) = t else { return Ok((Transform::default(), decoded)) };
        for (var, spec) in obj(t, "transform")? {
            let at = format!("transform/{var}");
            let so = obj(spec, &at)?;
            let rep = match (so.len(), so.get("replace")) {
                (1, Some(r)) => obj(r, &at)?,
                _ => return Err(format!("{at}: only replace")),
            };
            if rep.keys().any(|k| !["source", "replace", "by", "value", "default"].contains(&k.as_str())) {
                return Err(format!("{at}: replace keys"));
            }
            let src = rep.get("source").and_then(Json::as_str).unwrap_or("");
            let source = match src.strip_prefix('$') {
                Some(n) if !n.starts_with('$') && valid_meta(n) => self.names.id(n),
                _ => return Err(format!("{at}: source {src} (only `$NAME`)")),
            };
            let value = match rep.get("value") {
                None | Some(Json::Bool(false)) => false,
                Some(Json::Bool(true)) => true,
                _ => return Err(format!("{at}/value")),
            };
            let pattern = rep.get("replace").and_then(Json::as_str).ok_or_else(|| format!("{at}/replace"))?;
            let by = rep.get("by").and_then(Json::as_str).ok_or_else(|| format!("{at}/by"))?.to_owned();
            let default = rep.get("default").map(|d| d.as_str().map(str::to_owned).ok_or_else(|| format!("{at}/default"))).transpose()?;
            let name = self.names.id(var);
            if value {
                if pattern != "\\b\\B" || !by.is_empty() || default.is_some() {
                    return Err(format!("{at}: transform value with a replacement unsupported"));
                }
                decoded.push((name, source));
            }
            let regex = crate::rx::get(pattern).map_err(|e| format!("{at}/replace: {e}"))?;
            steps.push(TransformStep { name, source, default, value, regex, by });
        }
        // Each step after the step computing its source (a stable topological order).
        let mut ordered: Vec<TransformStep> = Vec::with_capacity(steps.len());
        while !steps.is_empty() {
            let ready = steps
                .iter()
                .position(|s| !steps.iter().any(|o| o.name == s.source))
                .ok_or("transform: cyclic sources")?;
            ordered.push(steps.remove(ready));
        }
        Ok((Transform { steps: ordered }, decoded))
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
        // Terms in native's order (compile/ast_rule.rs:397-923): atomic, the ordered group, then
        // count, compare, layout, value.
        let mut terms = Vec::new();
        if let Some(p) = r.get("pattern") {
            let p = p.as_str().ok_or_else(|| format!("{at}/pattern: contextual patterns unsupported"))?;
            terms.push(self.pattern(p, at)?);
        }
        if let Some(k) = r.get("kind").and_then(Json::as_str) {
            terms.push(self.kind(k)?);
        }
        if let Some(rx) = r.get("regex").and_then(Json::as_str) {
            terms.push(Rule::Regex(crate::rx::get(rx).map_err(|e| format!("{at}/regex: {e}"))?));
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
        if let Some(a) = r.get("arguments") {
            terms.push(self.compile_arguments(a, &format!("{at}/arguments"))?);
        }
        if let Some(fp) = r.get("filePath") {
            let o = obj(fp, "filePath")?;
            let get = |k: &str| -> Result<Option<Regex>, String> {
                o.get(k)
                    .map(|v| crate::rx::get(v.as_str().unwrap_or("")).map_err(|e| e.to_string()))
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
        for key in ["all", "any"] {
            let Some(items) = r.get(key).and_then(Json::as_array) else { continue };
            group += 1;
            let compiled = items
                .iter()
                .enumerate()
                .map(|(i, it)| self.rule(it, &format!("{at}/{key}/{i}")))
                .collect::<Result<Vec<_>, _>>()?;
            if key == "all" {
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
        if let Some(n) = r.get("not") {
            terms.push(Rule::Not(Box::new(self.rule(n, &format!("{at}/not"))?)));
            group += 1;
        }
        if let Some(m) = r.get("matches") {
            terms.push(self.matches(m, &format!("{at}/matches"))?);
            group += 1;
        }
        for key in ["inside", "has", "precedes", "follows"] {
            if let Some(rel) = r.get(key) {
                terms.push(self.relation(rel, key, &format!("{at}/{key}"))?);
                group += 1;
            }
        }
        // `binds` after the relations, in the ordered group (native compile/ast_rule.rs:739-754).
        if let Some(b) = r.get("binds") {
            terms.push(Rule::Binds(Box::new(crate::bindings::Filter::compile(b, &format!("{at}/binds"))?)));
            group += 1;
        }
        if group > 1 {
            order_by_captures(&mut terms, from, &self.summary)?;
        }
        if let Some(c) = r.get("count") {
            terms.push(self.count(c, &format!("{at}/count"))?);
        }
        if let Some(c) = r.get("compare") {
            terms.push(self.compare(c, &format!("{at}/compare"))?);
        }
        if let Some(l) = r.get("layout") {
            terms.push(self.compile_layout(l, &format!("{at}/layout"))?);
        }
        if let Some(v) = r.get("value") {
            terms.push(self.value(v, &format!("{at}/value"))?);
        }
        // `host` (a text grammar's tree rule only) is the last term, as native's
        // `compile_sub_rule` pushes it.
        if let Some(h) = r.get("host") {
            terms.push(self.compile_host(h, at)?);
        }
        if terms.iter().any(|t| matches!(t, Rule::Never)) {
            return Ok(Rule::Never);
        }
        if terms.is_empty() {
            return Err(format!("{at}: empty rule"));
        }
        Ok(if terms.len() == 1 { terms.pop().unwrap() } else { Rule::All(terms) })
    }

    /// `matches`: a utility's id, or a parameterised call `{ id: { PARAM: "$CAPTURE" } }`
    /// (native compile/ast_rule.rs:215-285).
    fn matches(&mut self, m: &Json, at: &str) -> Result<Rule, String> {
        let index = |c: &Self, id: &str| c.util_ids.iter().position(|u| u == id).ok_or_else(|| format!("{at}: unknown utility {id}"));
        match m {
            Json::String(id) => {
                let i = index(self, id)?;
                if !self.util_params[i].is_empty() {
                    return Err(format!("{at}: utility {id} takes parameters"));
                }
                Ok(Rule::Util(i))
            }
            Json::Object(call) if call.len() == 1 => {
                let (id, arguments) = call.iter().next().unwrap();
                let i = index(self, id)?;
                let params = self.util_params[i].clone();
                if params.is_empty() {
                    return Err(format!("{at}: utility {id} takes no parameters"));
                }
                let arguments = obj(arguments, at)?;
                if let Some(k) = arguments.keys().find(|k| !params.contains(k)) {
                    return Err(format!("{at}/{id}/{k}: not a parameter"));
                }
                let mut args = Vec::with_capacity(params.len());
                for p in &params {
                    let caller = arguments
                        .get(p)
                        .and_then(Json::as_str)
                        .and_then(|t| t.strip_prefix('$'))
                        .filter(|n| valid_meta(n))
                        .ok_or_else(|| format!("{at}/{id}/{p} must name a capture"))?;
                    args.push((self.names.id(p), self.names.id(caller)));
                }
                Ok(Rule::Call { util: i, args })
            }
            _ => Err(format!("{at}: must call one utility")),
        }
    }

    fn pattern(&mut self, written: &str, at: &str) -> Result<Rule, String> {
        let p = written.trim();
        if let Some(rest) = p.strip_prefix("$$").filter(|r| !r.starts_with('$') && valid_meta(r)) {
            return Ok(Rule::MetaVar { name: (rest != "_").then(|| self.names.id(rest)), named: false });
        }
        if let Some(rest) = p.strip_prefix('$').filter(|r| valid_meta(r)) {
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
            // Anything else: ast-grep's structural pattern (ast/pattern.rs).
            _ => self.compile_pattern_text(written, at),
        }
    }

    fn stop(&mut self, o: &serde_json::Map<String, Json>, at: &str) -> Result<Stop, String> {
        Ok(match o.get("stopBy") {
            None => Stop::Neighbor,
            Some(Json::String(s)) if s == "neighbor" => Stop::Neighbor,
            Some(Json::String(s)) if s == "end" => Stop::End,
            Some(v @ Json::Object(_)) => Stop::Rule(Box::new(self.rule(v, &format!("{at}/stopBy"))?)),
            Some(_) => return Err(format!("{at}/stopBy")),
        })
    }

    fn relation(&mut self, rel: &Json, key: &str, at: &str) -> Result<Rule, String> {
        let o = obj(rel, at)?;
        let stop = self.stop(o, at)?;
        let field = o.get("field").and_then(Json::as_str);
        if field.is_some() && !matches!(key, "inside" | "has") {
            return Err(format!("{at}/field: only inside and has take a field"));
        }
        let mut nested = o.clone();
        nested.remove("stopBy");
        nested.remove("field");
        let inner = self.rule(&Json::Object(nested), at)?;
        if matches!(inner, Rule::Never) {
            return Ok(Rule::Never);
        }
        let rule = Box::new(inner);
        Ok(match (key, field) {
            ("inside", None) => Rule::Inside { rule, stop },
            ("has", None) => Rule::Has { rule, stop },
            ("precedes", None) => Rule::Precedes { rule, stop },
            ("follows", None) => Rule::Follows { rule, stop },
            (_, Some(name)) => match self.grammar.field_code(name) {
                Some(code) => Rule::Field { inside: key == "inside", field: code, rule, stop },
                None => Rule::Never,
            },
            _ => unreachable!("relation keys"),
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

    /// `count` (native matcher/count.rs, compile/ast_rule.rs:805-821): the relation's `field`
    /// is not read, as native drops `compile_relation(..).2` there.
    fn count(&mut self, c: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(c, at)?;
        let keys = ["has", "inside", "precedes", "follows"];
        if let Some(k) = o.keys().find(|k| !keys.contains(&k.as_str()) && !["min", "max", "as", "offset"].contains(&k.as_str())) {
            return Err(format!("{at}: count key {k}"));
        }
        let min = o.get("min").and_then(Json::as_u64).unwrap_or(0) as usize;
        let max = o.get("max").and_then(Json::as_u64).map(|m| m as usize);
        let binding = match o.get("as") {
            None => None,
            Some(n) => {
                let n = n.as_str().filter(|n| valid_meta(n)).ok_or_else(|| format!("{at}/as must be a capture name"))?;
                let offset = o.get("offset").map(|x| x.as_i64().ok_or_else(|| format!("{at}/offset"))).transpose()?.unwrap_or(0);
                Some((self.names.id(n), offset))
            }
        };
        let (key, rel) = keys
            .iter()
            .find_map(|k| o.get(*k).map(|v| (*k, v)))
            .ok_or_else(|| format!("{at}: count needs a relation"))?;
        let ro = obj(rel, at)?;
        let stop = self.stop(ro, at)?;
        let mut nested = ro.clone();
        nested.remove("stopBy");
        nested.remove("field");
        let inner = Box::new(self.rule(&Json::Object(nested), at)?);
        let relation = match key {
            "has" => Rule::Has { rule: inner, stop },
            "inside" => Rule::Inside { rule: inner, stop },
            "precedes" => Rule::Precedes { rule: inner, stop },
            _ => Rule::Follows { rule: inner, stop },
        };
        Ok(Rule::Count { relation: Box::new(relation), min, max, binding })
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
            Some("index") => CompareText::Index,
            Some(t) => return Err(format!("{at}: text {t}")),
        };
        for k in o.keys() {
            if !["left", "right", "op", "text"].contains(&k.as_str()) {
                return Err(format!("{at}: compare key {k}"));
            }
        }
        let (left, right) = (self.names.id(left.trim_start_matches('$')), self.names.id(right.trim_start_matches('$')));
        let t = &self.transform;
        let transform = t.contains(left) || t.contains(right);
        if transform {
            if text != CompareText::Source {
                return Err(format!("{at}/text: a transform variable compares as source"));
            }
            if [left, right].iter().any(|n| t.chain(*n).iter().any(|&i| t.steps[i].value)) {
                return Err(format!("{at}: a transform reading a value is compared (value text unsupported)"));
            }
        }
        Ok(Rule::Compare { left, right, op, text, transform })
    }

    fn value(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(v, at)?;
        for k in o.keys() {
            if !["eq", "regex", "type"].contains(&k.as_str()) {
                return Err(format!("{at}: value key {k} unsupported"));
            }
        }
        let types: Vec<String> = match o.get("type") {
            None => Vec::new(),
            Some(Json::String(s)) => vec![s.clone()],
            Some(Json::Array(a)) if !a.is_empty() => a.iter().map(|x| x.as_str().map(str::to_owned).ok_or_else(|| format!("{at}/type must hold strings"))).collect::<Result<_, _>>()?,
            Some(_) => return Err(format!("{at}/type must be a string or an array of strings")),
        };
        if let Some(t) = types.iter().find(|t| *t != "str" && *t != "bytes") {
            return Err(format!("{at}: value type {t} unsupported (the decoder tells str and bytes only)"));
        }
        let eq = match o.get("eq") {
            None => None,
            Some(Json::String(s)) => Some(s.clone()),
            Some(_) => return Err(format!("{at}: value.eq only strings")),
        };
        let regex = o
            .get("regex")
            .map(|r| crate::rx::get(r.as_str().unwrap_or("")).map_err(|e| format!("{at}: {e}")))
            .transpose()?;
        Ok(Rule::Value(ValueTest { types, eq, regex }))
    }

    fn parse(&mut self, p: &Json, at: &str) -> Result<Rule, String> {
        let o = obj(p, at)?;
        let grammar = match o.get("grammar").and_then(Json::as_str) {
            Some("pythonFString") => ParseGrammar::PythonFString,
            Some("identifierWords") => ParseGrammar::IdentifierWords,
            Some("annotation") => ParseGrammar::Annotation,
            g => return Err(format!("{at}: grammar {g:?}")),
        };
        if o.contains_key("flags") {
            return Err(format!("{at}/flags: unsupported"));
        }
        let marker = match o.get("interpolation") {
            None => "{}".to_owned(),
            Some(Json::String(m)) if grammar == ParseGrammar::PythonFString && m.len() <= 1024 => m.clone(),
            Some(_) => return Err(format!("{at}/interpolation: a string of at most 1024 bytes, pythonFString only")),
        };
        let mut nested = o.clone();
        nested.remove("grammar");
        nested.remove("interpolation");
        let nested = Json::Object(nested);
        let (rule, hosted) = match grammar {
            ParseGrammar::Annotation => (self.rule(&nested, at)?, false),
            _ => self.compile_tree_rule(&nested, at, grammar)?,
        };
        Ok(Rule::Parse(Box::new(ParseTest { grammar, rule, marker, hosted })))
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
// Capture analysis (native matcher/captures.rs, captures/semantic.rs): what a rule binds, reads
// and mentions, with utilities summarised as least fixpoints (native `UtilitySummary`).

type NameSet = BTreeSet<Name>;
static EMPTY: NameSet = BTreeSet::new();

/// Per-utility summaries over one utility table. An empty table is native's unfilled one: every
/// utility reads as binding, reading and mentioning nothing.
#[derive(Default)]
pub struct Summary {
    binds: Vec<NameSet>,
    reads: Vec<NameSet>,
    mentions: Vec<NameSet>,
    exported: Vec<NameSet>,
    transform: Arc<Transform>,
}

fn outward(names: &NameSet, args: &[(Name, Name)]) -> NameSet {
    args.iter().filter(|(p, _)| names.contains(p)).map(|(_, c)| *c).collect()
}

impl Summary {
    pub fn of(utils: &[Rule], transform: Arc<Transform>) -> Summary {
        let n = utils.len();
        let mut s = Summary { binds: vec![NameSet::new(); n], reads: vec![NameSet::new(); n], mentions: vec![NameSet::new(); n], exported: vec![NameSet::new(); n], transform };
        // Each summary is monotone in the summaries it reads: iterate from empty to the least
        // fixpoint (what native's per-component walks reach).
        loop {
            let next: Vec<NameSet> = utils.iter().map(|u| s.binds(u)).collect();
            if next == s.binds {
                break;
            }
            s.binds = next;
        }
        loop {
            let next: Vec<NameSet> = utils.iter().map(|u| s.exported(u)).collect();
            if next == s.exported {
                break;
            }
            s.exported = next;
        }
        loop {
            let next: Vec<NameSet> = utils.iter().map(|u| s.binds_reads(u).1).collect();
            if next == s.reads {
                break;
            }
            s.reads = next;
        }
        loop {
            let next: Vec<NameSet> = utils.iter().map(|u| s.mentions(u)).collect();
            if next == s.mentions {
                break;
            }
            s.mentions = next;
        }
        s
    }

    fn of_util(v: &[NameSet], i: usize) -> &NameSet {
        v.get(i).unwrap_or(&EMPTY)
    }

    /// Native `collect_binds`.
    pub fn binds(&self, rule: &Rule) -> NameSet {
        let mut out = NameSet::new();
        self.binds_into(rule, &mut out);
        out
    }
    fn binds_into(&self, rule: &Rule, out: &mut NameSet) {
        match rule {
            Rule::MetaVar { name: Some(n), .. } | Rule::Capture(n) => {
                out.insert(*n);
            }
            Rule::Pattern(p) => out.extend(p.binds()),
            Rule::Arguments(a) => out.extend(a.binds()),
            Rule::All(v) | Rule::Any(v) => v.iter().for_each(|r| self.binds_into(r, out)),
            Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Precedes { rule, .. } | Rule::Follows { rule, .. } | Rule::Field { rule, .. } => self.binds_into(rule, out),
            Rule::NthChild { of_rule: Some(r), .. } => self.binds_into(r, out),
            Rule::Util(i) => out.extend(Self::of_util(&self.binds, *i)),
            Rule::Call { util, args } => out.extend(outward(Self::of_util(&self.binds, *util), args)),
            Rule::Count { binding: Some((n, _)), .. } => {
                out.insert(*n);
            }
            Rule::Parse(p) => out.extend(self.exported(&p.rule)),
            _ => {}
        }
    }

    /// Native `parse_exported`: what a parsed tree's rule exports.
    fn exported(&self, rule: &Rule) -> NameSet {
        let mut out = NameSet::new();
        self.exported_into(rule, &mut out);
        out
    }
    fn exported_into(&self, rule: &Rule, out: &mut NameSet) {
        match rule {
            Rule::MetaVar { name: Some(n), .. } | Rule::Capture(n) => {
                out.insert(*n);
            }
            Rule::Pattern(p) => out.extend(p.binds()),
            Rule::Parse(p) => self.exported_into(&p.rule, out),
            Rule::Util(i) => out.extend(Self::of_util(&self.exported, *i)),
            Rule::Call { util, args } => out.extend(outward(Self::of_util(&self.exported, *util), args)),
            Rule::All(v) | Rule::Any(v) => v.iter().for_each(|r| self.exported_into(r, out)),
            Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Precedes { rule, .. } | Rule::Follows { rule, .. } => self.exported_into(rule, out),
            _ => {}
        }
    }

    /// The captures a comparison reads: each side, or the captures a transform variable on that
    /// side is computed from (native `comparison_reads`).
    fn comparison_reads(&self, left: Name, right: Name, transform: bool) -> NameSet {
        [left, right]
            .into_iter()
            .flat_map(|n| if transform && self.transform.contains(n) { self.transform.roots(n) } else { [n].into_iter().collect() })
            .collect()
    }

    /// Native `binds_and_reads`.
    pub fn binds_reads(&self, rule: &Rule) -> (NameSet, NameSet) {
        match rule {
            Rule::MetaVar { .. } | Rule::Capture(_) | Rule::Pattern(_) | Rule::Arguments(_) => (self.binds(rule), NameSet::new()),
            Rule::Compare { left, right, transform, .. } => (NameSet::new(), self.comparison_reads(*left, *right, *transform)),
            Rule::Layout(l) => (NameSet::new(), l.reads().into_iter().collect()),
            Rule::Any(items) => {
                let (mut b, mut r) = (NameSet::new(), NameSet::new());
                for it in items {
                    let (ib, ir) = self.binds_reads(it);
                    b.extend(ib);
                    r.extend(ir);
                }
                (b, r)
            }
            Rule::All(items) => {
                let (mut b, mut r) = (NameSet::new(), NameSet::new());
                for it in items {
                    let (ib, ir) = self.binds_reads(it);
                    r.extend(ir.into_iter().filter(|n| !b.contains(n)));
                    b.extend(ib);
                }
                (b, r)
            }
            Rule::Not(it) => {
                let (mut r, ir) = self.binds_reads(it);
                r.extend(ir);
                (NameSet::new(), r)
            }
            Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Precedes { rule, .. } | Rule::Follows { rule, .. } | Rule::Field { rule, .. } => self.binds_reads(rule),
            Rule::NthChild { of_rule: Some(r), .. } => self.binds_reads(r),
            Rule::Count { relation, binding, .. } => (binding.iter().map(|(n, _)| *n).collect(), self.binds_reads(relation).1),
            Rule::Util(i) => (Self::of_util(&self.binds, *i).clone(), Self::of_util(&self.reads, *i).clone()),
            Rule::Call { util, args } => (outward(Self::of_util(&self.binds, *util), args), outward(Self::of_util(&self.reads, *util), args)),
            // A parse binds its exports and reads nothing of the caller.
            Rule::Parse(_) => (self.binds(rule), NameSet::new()),
            _ => (NameSet::new(), NameSet::new()),
        }
    }

    /// Native `collect_mentions`.
    pub fn mentions(&self, rule: &Rule) -> NameSet {
        let mut out = NameSet::new();
        self.mentions_into(rule, &mut out);
        out
    }
    fn mentions_into(&self, rule: &Rule, out: &mut NameSet) {
        match rule {
            Rule::MetaVar { name: Some(n), .. } | Rule::Capture(n) => {
                out.insert(*n);
            }
            Rule::Pattern(p) => out.extend(p.binds()),
            Rule::Arguments(a) => out.extend(a.binds()),
            Rule::Compare { left, right, transform, .. } => out.extend(self.comparison_reads(*left, *right, *transform)),
            Rule::Layout(l) => out.extend(l.reads()),
            Rule::All(v) | Rule::Any(v) => v.iter().for_each(|r| self.mentions_into(r, out)),
            Rule::Not(r) => self.mentions_into(r, out),
            Rule::Has { rule, .. } | Rule::Inside { rule, .. } | Rule::Precedes { rule, .. } | Rule::Follows { rule, .. } | Rule::Field { rule, .. } => self.mentions_into(rule, out),
            Rule::NthChild { of_rule: Some(r), .. } => self.mentions_into(r, out),
            Rule::Count { relation, binding, .. } => {
                out.extend(binding.iter().map(|(n, _)| *n));
                self.mentions_into(relation, out);
            }
            Rule::Util(i) => out.extend(Self::of_util(&self.mentions, *i)),
            Rule::Call { args, .. } => out.extend(args.iter().map(|(_, c)| *c)),
            Rule::Parse(p) => out.extend(self.exported(&p.rule)),
            _ => {}
        }
    }
}

fn order_by_captures(terms: &mut Vec<Rule>, from: usize, s: &Summary) -> Result<(), String> {
    // Native compile/order.rs `order_by_captures`: a group term reading what a sibling binds
    // runs after it; otherwise the written order stays.
    let early: NameSet = terms[..from].iter().flat_map(|t| s.binds(t)).collect();
    let group = &terms[from..];
    let bound: Vec<NameSet> = group.iter().map(|t| s.binds(t)).collect();
    let needs: Vec<NameSet> = group.iter().map(|t| s.binds_reads(t).1.into_iter().filter(|n| !early.contains(n)).collect()).collect();
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

/// Whether a query's rule (through its utilities) can bind any capture.
pub fn binds_any(q: &Query) -> bool {
    !q.summary.binds(&q.rule).is_empty()
}

/// Whether a query's rule binds or mentions the capture `name` (a metavariable `each` rather
/// than a regex group's).
pub fn binds_name(q: &Query, name: Name) -> bool {
    q.summary.binds(&q.rule).contains(&name) || q.summary.mentions(&q.rule).contains(&name)
}

fn reads_values(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>) -> bool {
    match rule {
        Rule::Value(_) | Rule::Parse(_) => true,
        Rule::Arguments(a) => a.reads_values(),
        Rule::Host(h) => h.sub_rules().into_iter().any(|r| reads_values(r, utils, seen)),
        Rule::All(v) | Rule::Any(v) => v.iter().any(|r| reads_values(r, utils, seen)),
        Rule::Not(r) => reads_values(r, utils, seen),
        Rule::Has { rule, stop } | Rule::Inside { rule, stop } | Rule::Precedes { rule, stop } | Rule::Follows { rule, stop } | Rule::Field { rule, stop, .. } => {
            reads_values(rule, utils, seen) || matches!(stop, Stop::Rule(s) if reads_values(s, utils, seen))
        }
        Rule::NthChild { of_rule: Some(r), .. } => reads_values(r, utils, seen),
        Rule::Count { relation, .. } => reads_values(relation, utils, seen),
        Rule::Util(i) | Rule::Call { util: i, .. } => seen.insert(*i) && reads_values(&utils[*i], utils, seen),
        _ => false,
    }
}

/// Kinds every match's root must have, when the rule's top-level `all` fixes them.
fn root_kinds(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>) -> Option<Vec<u32>> {
    match rule {
        Rule::Kind(k) => Some(vec![*k]),
        Rule::Literal { kind, .. } => Some(vec![*kind]),
        Rule::Pattern(p) => p.root_kind().map(|k| vec![k]),
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
    parsed_hosts: std::cell::RefCell<rustc_hash::FxHashMap<(u32, u8, String), Option<(u32, u32)>>>,
    pub unsupported: std::cell::RefCell<Option<String>>,
    /// Utility verdicts and relation memos; shared by the queries of one policy set over one
    /// file that have the same utility table and valuedness (`FileMemo`).
    memo: Rc<MemoSet>,
    pub steps: std::cell::Cell<u64>,
    /// Refusals so far (`taint`).
    refusals: std::cell::Cell<u64>,
    /// Utility levels entered (`matches`, parameterised calls).
    depth: std::cell::Cell<u32>,
}

/// Memoised verdicts over one file's nodes. Capture-free utility verdicts per utility, dense over
/// the nodes (0 not yet, 1 true, 2 false; one byte a node, allocated on the utility's first
/// verdict); relation memos (`RelMemo`) by the rule's address. Both are pure functions of the
/// node for one utility table and one valuedness (unknown verdicts are never stored as definite),
/// so queries that share those share the memo.
#[derive(Default)]
pub struct MemoSet {
    util: std::cell::RefCell<Vec<crate::trits::Trits>>,
    /// `inside`/`has` with `stopBy: end` over a rule that mentions no capture and parses
    /// nothing, memoised per node, keyed by the rule's address.
    rel: std::cell::RefCell<rustc_hash::FxHashMap<usize, RelMemo>>,
}

/// One file's memo sets for one policy set, by (utility table, valuedness).
#[derive(Default)]
pub struct FileMemo {
    sets: std::cell::RefCell<rustc_hash::FxHashMap<(usize, bool), Rc<MemoSet>>>,
}

/// Per memoisable rule: whether it qualifies, and per node (0 not yet, 1 true, 2 false, 3
/// unknown) the rule's verdict, `inside {rule, stopBy: end}`'s and `has {rule, stopBy: end}`'s.
#[derive(Default)]
struct RelMemo {
    ok: bool,
    at: crate::trits::Trits,
    inside: crate::trits::Trits,
    has: crate::trits::Trits,
    /// `inside {rule, field, stopBy: end}` (the field is fixed by the `Field` rule holding it).
    finside: crate::trits::Trits,
}

const T: u8 = 1;
const F: u8 = 2;
const U: u8 = 3;

fn parses(rule: &Rule, utils: &[Rule], seen: &mut BTreeSet<usize>) -> bool {
    match rule {
        Rule::Parse(_) | Rule::Host(_) => true,
        Rule::All(v) | Rule::Any(v) => v.iter().any(|r| parses(r, utils, seen)),
        Rule::Not(r) => parses(r, utils, seen),
        Rule::Has { rule, stop } | Rule::Inside { rule, stop } | Rule::Precedes { rule, stop } | Rule::Follows { rule, stop } | Rule::Field { rule, stop, .. } => parses(rule, utils, seen) || matches!(stop, Stop::Rule(s) if parses(s, utils, seen)),
        Rule::NthChild { of_rule: Some(r), .. } => parses(r, utils, seen),
        Rule::Count { relation, .. } => parses(relation, utils, seen),
        Rule::Util(i) | Rule::Call { util: i, .. } => seen.insert(*i) && parses(&utils[*i], utils, seen),
        _ => false,
    }
}

impl<'a> Eval<'a> {
    pub fn new(f: &'a Facts, path: &'a str, q: &'a Query) -> Self {
        Self::with_memo(f, path, q, Rc::default())
    }

    /// An evaluation sharing `memo`'s verdicts with the policy set's other queries over this
    /// file. A query with a `transform` reads decoded values its own way: it memoises alone.
    pub fn shared(f: &'a Facts, path: &'a str, q: &'a Query, memo: &FileMemo) -> Self {
        let set = if q.decoded.is_empty() {
            memo.sets.borrow_mut().entry((Arc::as_ptr(&q.utils) as *const Rule as usize, q.three_valued)).or_default().clone()
        } else {
            Rc::default()
        };
        Self::with_memo(f, path, q, set)
    }

    fn with_memo(f: &'a Facts, path: &'a str, q: &'a Query, memo: Rc<MemoSet>) -> Self {
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
            memo,
            steps: Default::default(),
            refusals: Default::default(),
            depth: Default::default(),
        }
    }

    fn refuse(&self, why: String) {
        self.refusals.set(self.refusals.get() + 1);
        let mut u = self.unsupported.borrow_mut();
        if u.is_none() {
            *u = Some(why);
        }
    }

    /// `binds` (native matcher/binds.rs `Binds::match_node`, Python, declaration-only, no
    /// sub-rules): some binding named at the node passes the filters. A parsed literal's node
    /// names none in native's table; `binds` inside `parse` is a compile error there, refused here.
    fn match_binds(&self, filter: &crate::bindings::Filter, node: Bound) -> bool {
        let Bound::Node(n) = node else {
            self.refuse("binds on a parsed literal's node (native: not supported inside parse)".into());
            return false;
        };
        match self.f.bindings() {
            Ok(all) => crate::bindings::at(all, n).iter().any(|b| filter.passes(b)),
            Err(why) => {
                self.refuse(why);
                false
            }
        }
    }

    /// Counts of refusals and of `\N{...}` escapes decoded so far: a verdict whose evaluation
    /// changed either is not memoised, so a query reusing the memo refuses exactly as its own
    /// evaluation would.
    fn taint(&self) -> (u64, u64) {
        (self.refusals.get(), crate::pyvalue::NAMED_ESCAPES.with(|n| n.get()))
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
            Rule::Util(i) => {
                if !self.enter_level() {
                    return false;
                }
                let r = self.util(*i, node, env);
                self.depth.set(self.depth.get() - 1);
                r
            }
            Rule::Call { util, args } => self.call(*util, args, node, env),
            Rule::FilePath { regex, not_regex } => {
                regex.as_ref().is_none_or(|r| r.is_match(self.path))
                    && not_regex.as_ref().is_none_or(|r| !r.is_match(self.path))
            }
            Rule::Has { rule, stop } => self.has(node, rule, stop, env),
            Rule::Inside { rule, stop } => self.inside(node, rule, stop, env),
            Rule::Precedes { rule, stop } => {
                let (once, all) = self.sibling_walk(node, true);
                self.find(stop, once, &all, rule, env)
            }
            Rule::Follows { rule, stop } => {
                let (once, all) = self.sibling_walk(node, false);
                self.find(stop, once, &all, rule, env)
            }
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
            Rule::Compare { left, right, op, text, transform } => match self.compare(*left, *right, *op, *text, *transform, env) {
                Some(v) => v,
                None => {
                    self.unknown.set(true);
                    false
                }
            },
            // A count runs with `violation.each` suspended (native matcher/mod.rs:562).
            Rule::Count { relation, min, max, binding } => self.suspended(|| self.count(node, relation, *min, *max, *binding, env)),
            Rule::Layout(l) => self.match_layout(l, node, env),
            Rule::Pattern(p) => self.match_pattern(p, node, env),
            Rule::Arguments(a) => self.match_arguments(a, node, env),
            Rule::Host(h) => self.match_host(h, node, env),
            Rule::Parse(p) => self.parse(node, p, env),
            Rule::Binds(filter) => self.match_binds(filter, node),
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
                let m = self.q.summary.mentions(item);
                if !m.is_disjoint(names) {
                    names.extend(self.q.summary.binds(item));
                    continue;
                }
            }
            if !self.m(item, node, &mut trial) {
                if self.three && self.unknown.get() {
                    self.unknown.set(false);
                    unknown = true;
                    let b = self.q.summary.binds(item);
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
        !self.q.summary.binds(rule).is_empty()
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
            Bound::Node(n) if self.q.util_capture_free[i] && !self.each_active() => Some((i, n)),
            _ => None,
        };
        // Mutant 53 (the historical utility-memo regression, G3b.2): the memoised verdict is not
        // read, so every use evaluates the utility again (same verdicts; quadratic work).
        if let Some((i, n)) = memo_key.filter(|_| !crate::mutant::on(53)) {
            match self.memo.util.borrow().get(i).map_or(0, |v| v.get(n)) {
                1 => return true,
                2 => return false,
                _ => {}
            }
        }
        // Whether this evaluation itself was unknown decides memoization, not an unknown the
        // caller had already accumulated; the caller's flag is restored on top.
        let saved = self.three && self.unknown.replace(false);
        let t0 = self.taint();
        let r = self.m(&self.q.utils[i], node, env);
        if let Some((i, n)) = memo_key {
            if !(self.three && self.unknown.get()) && self.taint() == t0 {
                let mut util = self.memo.util.borrow_mut();
                if util.len() <= i {
                    util.resize_with(i + 1, Default::default);
                }
                util[i].set(self.f.len(), n, if r { 1 } else { 2 });
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
                if let (Stop::End, Bound::Node(n)) = (stop, node) {
                    if let Some(key) = self.memo_slot(rule) {
                        return self.memo_finish(self.inside_end_memo(key, rule, n));
                    }
                }
                let anc = self.ancestors(node);
                self.find(stop, None, &anc, rule, env)
            }
        }
    }

    /// Spike A3: the relation memo's slot for `rule`, when it qualifies (it mentions no capture
    /// and parses nothing, so its verdict at a node is a pure function of the node, as the
    /// utility memo assumes) and no `each` enumeration is active.
    fn memo_slot(&self, rule: &Rule) -> Option<usize> {
        if self.each_active() {
            return None;
        }
        let key = rule as *const Rule as usize;
        let mut memo = self.memo.rel.borrow_mut();
        let e = memo.entry(key).or_insert_with(|| {
            let m = self.q.summary.mentions(rule);
            RelMemo { ok: m.is_empty() && !parses(rule, &self.q.utils, &mut BTreeSet::new()), ..Default::default() }
        });
        e.ok.then_some(key)
    }

    /// The rule's verdict at one node (true, false, unknown), as a relation search tests a
    /// candidate: evaluated once per node, with the caller's unknown flag set aside.
    fn verdict_at(&self, key: usize, rule: &Rule, n: u32) -> u8 {
        let v = self.memo.rel.borrow()[&key].at.get(n);
        if v != 0 {
            return v;
        }
        let saved = self.unknown.replace(false);
        let mut env = Env::default();
        let t0 = self.taint();
        let r = self.m(rule, Bound::Node(n), &mut env);
        let unknown = self.three && self.unknown.get();
        self.unknown.set(saved);
        let v = if r { T } else if unknown { U } else { F };
        if self.taint() != t0 {
            return v;
        }
        self.memo.rel.borrow_mut().get_mut(&key).unwrap().at.set(self.f.len(), n, v);
        v
    }

    /// `inside {rule, stopBy: end}` at `n`: true when an ancestor's verdict is true, else
    /// unknown when one is unknown, else false. Ancestors are tested upwards until the first
    /// true one or an ancestor whose answer is already known, so every verdict is first
    /// evaluated exactly when the unmemoised walk first evaluates it.
    fn inside_end_memo(&self, key: usize, rule: &Rule, n: u32) -> u8 {
        let combine = |r: u8, v: u8| if r == T || v == T { T } else if r == U || v == U { U } else { F };
        let mut chain: Vec<(u32, u8)> = Vec::new();
        let t0 = self.taint();
        let mut c = n;
        let mut v = loop {
            let known = self.memo.rel.borrow()[&key].inside.get(c);
            if known != 0 {
                break known;
            }
            let Some(p) = self.f.parent(c) else {
                chain.push((c, F));
                break F;
            };
            let r = self.verdict_at(key, rule, p);
            chain.push((c, r));
            if r == T {
                break T;
            }
            c = p;
        };
        let clean = self.taint() == t0;
        let mut memo = self.memo.rel.borrow_mut();
        let inside = &mut memo.get_mut(&key).unwrap().inside;
        for &(c, r) in chain.iter().rev() {
            v = combine(r, v);
            if clean {
                inside.set(self.f.len(), c, v);
            }
        }
        v
    }

    /// `has {rule, stopBy: end}` at `n`: the descendants in preorder until the first true
    /// verdict; a descendant whose own answer is known is not entered again.
    fn has_end_memo(&self, key: usize, rule: &Rule, n: u32) -> u8 {
        let known = self.memo.rel.borrow()[&key].has.get(n);
        if known != 0 {
            return known;
        }
        let end = self.f.subtree_end[n as usize];
        let t0 = self.taint();
        let mut i = n + 1;
        let mut unknown = false;
        let mut res = F;
        while i < end {
            let r = self.verdict_at(key, rule, i);
            if r == T {
                res = T;
                break;
            }
            unknown |= r == U;
            match self.memo.rel.borrow()[&key].has.get(i) {
                0 => i += 1,
                T => {
                    res = T;
                    break;
                }
                h => {
                    unknown |= h == U;
                    i = self.f.subtree_end[i as usize];
                }
            }
        }
        if res != T && unknown {
            res = U;
        }
        if self.taint() != t0 {
            return res;
        }
        self.memo.rel.borrow_mut().get_mut(&key).unwrap().has.set(self.f.len(), n, res);
        res
    }

    /// A relation search's answer from an existence verdict over a rule that binds nothing
    /// (`search_start`/`search_finish`): unknown sets the caller's flag, otherwise it stays.
    fn memo_finish(&self, v: u8) -> bool {
        if v == U && self.three {
            self.unknown.set(true);
        }
        v == T
    }

    fn has(&self, node: Bound, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
        if let (Stop::End, Bound::Node(n)) = (stop, node) {
            if let Some(key) = self.memo_slot(rule) {
                return self.memo_finish(self.has_end_memo(key, rule, n));
            }
        }
        self.has_unmemoised(node, rule, stop, env)
    }

    fn has_unmemoised(&self, node: Bound, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
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
        if let (Stop::End, Bound::Node(n)) = (stop, node) {
            if let Some(key) = self.memo_slot(rule) {
                return self.memo_finish(self.field_inside_end_memo(key, rule, field, n));
            }
        }
        self.field_inside_unmemoised(node, field, rule, stop, env)
    }

    /// `inside {rule, field, stopBy: end}` at `n`: an ancestor counts when the walk entered it
    /// through `field` (its first child with that field is the walk's previous node) and its
    /// verdict holds; true, else unknown, else false, as `inside_end_memo`.
    fn field_inside_end_memo(&self, key: usize, rule: &Rule, field: u16, n: u32) -> u8 {
        let combine = |r: u8, v: u8| if r == T || v == T { T } else if r == U || v == U { U } else { F };
        let mut chain: Vec<(u32, u8)> = Vec::new();
        let t0 = self.taint();
        let mut c = n;
        let mut v = loop {
            let known = self.memo.rel.borrow()[&key].finside.get(c);
            if known != 0 {
                break known;
            }
            let Some(p) = self.f.parent(c) else {
                chain.push((c, F));
                break F;
            };
            let entered = self.field_child(Bound::Node(p), field) == Some(Bound::Node(c));
            let r = if entered { self.verdict_at(key, rule, p) } else { F };
            chain.push((c, r));
            if r == T {
                break T;
            }
            c = p;
        };
        let clean = self.taint() == t0;
        let mut memo = self.memo.rel.borrow_mut();
        let fi = &mut memo.get_mut(&key).unwrap().finside;
        for &(c, r) in chain.iter().rev() {
            v = combine(r, v);
            if clean {
                fi.set(self.f.len(), c, v);
            }
        }
        v
    }

    fn field_inside_unmemoised(&self, node: Bound, field: u16, rule: &Rule, stop: &Stop, env: &mut Env) -> bool {
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

    /// `count` (native matcher/count.rs `Count::holds`).
    fn count(&self, node: Bound, relation: &Rule, min: usize, max: Option<usize>, binding: Option<(Name, i64)>, env: &mut Env) -> bool {
        let three = self.three;
        let saved = three && self.unknown.replace(false);
        let mut found = 0usize;
        let mut unknown = 0usize;
        let mut open = false;
        let bound = binding.is_some();
        let mut test = |this: &Self, rule: &Rule, c: Bound| -> bool {
            let mut probe = env.clone();
            if this.m(rule, c, &mut probe) {
                found += 1;
                // Past `max` the answer is no; at `min` with no `max` it is yes, unless the
                // count is bound, which needs every node counted.
                if max.is_some_and(|m| found > m) || (max.is_none() && !bound && found >= min) {
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
            Rule::Inside { rule, stop } | Rule::Precedes { rule, stop } | Rule::Follows { rule, stop } => {
                let walk: Vec<Bound> = match relation {
                    Rule::Inside { .. } => match stop {
                        Stop::Neighbor => self.parent(node).into_iter().collect(),
                        _ => self.ancestors(node),
                    },
                    _ => {
                        let (once, all) = self.sibling_walk(node, matches!(relation, Rule::Precedes { .. }));
                        match stop {
                            Stop::Neighbor => once.into_iter().collect(),
                            _ => all,
                        }
                    }
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
        let exact = !bound || high == Some(found);
        let holds = exact && found >= min && max.is_none_or(|m| high.is_some_and(|h| h <= m));
        if holds {
            if let Some((name, offset)) = binding {
                let value = i128::try_from(found).unwrap_or(i128::MAX) + i128::from(offset);
                env.set_text(name, value.to_string());
            }
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
                match parse_ext::text_node_value(pn.kind, &pn.text) {
                    Some(v) => v,
                    None => return Some(false),
                }
            }
        };
        if matches!(value, PyValue::Unknown) {
            return None;
        }
        // `type` (native matcher/value.rs:269-275, value/mod.rs `is_type`): the value's Python
        // type name is one of the listed ones; compile admits only `str` and `bytes`.
        if !test.types.is_empty() {
            let own = match &value {
                PyValue::Str(_) => "str",
                PyValue::Bytes(_) => "bytes",
                _ => "",
            };
            if !test.types.iter().any(|t| t == own) {
                return Some(false);
            }
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

    // --- utility levels, parameterised calls (native matcher/call.rs, mod.rs:556-591) ---

    /// Enters one utility level; at native's depth limit (`MAX_UTIL_DEPTH`) the evaluation
    /// refuses (native fails the clause for the file).
    fn enter_level(&self) -> bool {
        if self.depth.get() >= 256 {
            self.refuse("utility recursion deeper than 256 levels (native UtilLimit)".into());
            return false;
        }
        self.depth.set(self.depth.get() + 1);
        true
    }

    /// Native `unify_capture`: binds `to` in `target` to what the `from` captures hold in
    /// `source`, unifying them by text with each other and with an existing binding; of equal
    /// nodes the first in the source is bound. Parsed-literal bindings stand for native's
    /// exported texts, which agree only with an equal text at an equal range. `None` where they
    /// differ.
    fn unify(&self, source: &Env, from: &[Name], target: &mut Env, to: Name) -> Option<()> {
        let held = target.get(to).filter(|b| matches!(b, Bound::Node(_)));
        let mut chosen: Option<Bound> = None;
        for b in from.iter().filter_map(|f| source.get(*f)).filter(|b| matches!(b, Bound::Node(_))) {
            if let Some(agreed) = held.or(chosen) {
                (self.text_of(agreed) == self.text_of(b)).then_some(())?;
            }
            if held.is_none() && chosen.is_none_or(|k| self.range_of(b) < self.range_of(k)) {
                chosen = Some(b);
            }
        }
        if let Some(b) = chosen {
            if matches!(target.get(to), Some(Bound::Parsed(_))) {
                self.refuse("parameterised call: a node and an exported text under one name".into());
                return None;
            }
            target.set(to, b);
        }
        // `$$$` lists.
        let held = target.multi.iter().find(|(n, _)| *n == to).map(|(_, v)| v.clone()).unwrap_or_default();
        let agree = |l: &[Bound], r: &[Bound]| l.len() == r.len() && l.iter().zip(r).all(|(a, b)| self.text_of(*a) == self.text_of(*b));
        let mut chosen: Option<Vec<Bound>> = None;
        for f in from {
            let Some((_, nodes)) = source.multi.iter().find(|(n, _)| n == f) else { continue };
            if nodes.is_empty() {
                continue;
            }
            let agreed = if held.is_empty() { chosen.as_deref() } else { Some(held.as_slice()) };
            if let Some(agreed) = agreed {
                agree(agreed, nodes).then_some(())?;
            }
            if held.is_empty() && chosen.as_ref().is_none_or(|kept| nodes.iter().map(|b| self.range_of(*b)).lt(kept.iter().map(|b| self.range_of(*b)))) {
                chosen = Some(nodes.clone());
            }
        }
        if let Some(nodes) = chosen {
            target.multi.retain(|(n, _)| *n != to);
            target.multi.push((to, nodes));
        }
        // Exported texts (parsed-literal bindings) and text variables.
        for f in from {
            if let Some(Bound::Parsed(p)) = source.get(*f) {
                match target.get(to) {
                    Some(Bound::Parsed(q)) => {
                        let (a, b) = (self.range_of(Bound::Parsed(p)), self.range_of(Bound::Parsed(q)));
                        (a == b && self.text_of(Bound::Parsed(p)) == self.text_of(Bound::Parsed(q))).then_some(())?;
                    }
                    None => target.set(to, Bound::Parsed(p)),
                    Some(Bound::Node(_)) => {
                        self.refuse("parameterised call: a node and an exported text under one name".into());
                        return None;
                    }
                }
            }
            if let Some(text) = source.text(*f) {
                match target.text(to) {
                    Some(existing) => (existing == text).then_some(())?,
                    None => target.set_text(to, text.to_owned()),
                }
            }
        }
        Some(())
    }

    /// A parameterised utility call at `node` (native `call::call` and `match_extension`).
    fn call(&self, util: usize, args: &[(Name, Name)], node: Bound, env: &mut Env) -> bool {
        if !self.enter_level() {
            return false;
        }
        let matched = (|| {
            let mut frame = Env::default();
            for (param, caller) in args {
                self.unify(env, &[*caller], &mut frame, *param)?;
            }
            // `violation.each` in the frame: the enumerated capture is excluded under the
            // parameter it is passed as, nothing where it is not passed (native `each::framed`).
            let active = self.each_active();
            let renamed = if active {
                let name = self.each.borrow().as_ref().map(|e| e.0).unwrap();
                args.iter().find(|(_, c)| *c == name).map(|(p, _)| (name, *p))
            } else {
                None
            };
            let body = |frame: &mut Env| self.m(&self.q.utils[util], node, frame);
            let ok = match (active, renamed) {
                (false, _) => body(&mut frame),
                (true, None) => self.suspended(|| body(&mut frame)),
                (true, Some((caller, param))) => {
                    self.each.borrow_mut().as_mut().unwrap().0 = param;
                    let ok = body(&mut frame);
                    self.each.borrow_mut().as_mut().unwrap().0 = caller;
                    ok
                }
            };
            ok.then_some(frame)
        })();
        self.depth.set(self.depth.get() - 1);
        // A failed body leaves the unknown flag as it set it (native: `Err(())` sets UNKNOWN).
        let Some(frame) = matched else { return false };
        let mut out = env.clone();
        let returned = args.iter().any(|(p, c)| {
            (frame.get(*p).is_some() && out.get(*c).is_none())
                || (frame.multi.iter().any(|(n, v)| n == p && !v.is_empty()) && !out.multi.iter().any(|(n, v)| n == c && !v.is_empty()))
                || (frame.text(*p).is_some() && out.text(*c).is_none())
        });
        if returned {
            let mut unified: Vec<Name> = Vec::new();
            for (_, caller) in args {
                if unified.contains(caller) {
                    continue;
                }
                unified.push(*caller);
                let params: Vec<Name> = args.iter().filter(|(_, c)| c == caller).map(|(p, _)| *p).collect();
                if self.unify(&frame, &params, &mut out, *caller).is_none() {
                    return false;
                }
            }
        }
        *env = out;
        true
    }

    // --- siblings (native tape.rs:2197-2228, relation.rs `find_sibling`) ---

    /// `precedes`/`follows` from `node`: the neighbour (`next`: the first later sibling ending
    /// after this node's end; `prev`: the previous sibling) and the walk (`next_all`/`prev_all`
    /// from the walk anchor: the first sibling, from this node, ending after its start; for the
    /// root, from its first child).
    fn sibling_walk(&self, node: Bound, forward: bool) -> (Option<Bound>, Vec<Bound>) {
        let (start, end) = self.range_of(node);
        let parent = self.parent(node);
        let (sibs, at): (Vec<Bound>, Option<usize>) = match parent {
            Some(p) => {
                let s: Vec<Bound> = self.children(p).collect();
                let i = s.iter().position(|c| *c == node);
                (s, i)
            }
            None => {
                let s: Vec<Bound> = self.children(node).collect();
                let i = (!s.is_empty()).then_some(0);
                (s, i)
            }
        };
        let once = match (parent, at) {
            (Some(_), Some(i)) => {
                if forward {
                    sibs[i + 1..].iter().copied().find(|c| self.range_of(*c).1 > end)
                } else {
                    i.checked_sub(1).map(|j| sibs[j])
                }
            }
            _ => None,
        };
        let all = match at {
            None => Vec::new(),
            Some(mut a) => {
                let ends_after = |b: Bound| self.range_of(b).1 > start;
                let found = if ends_after(sibs[a]) {
                    while a > 0 && ends_after(sibs[a - 1]) {
                        a -= 1;
                    }
                    true
                } else {
                    loop {
                        a += 1;
                        if a >= sibs.len() {
                            break false;
                        }
                        if ends_after(sibs[a]) {
                            break true;
                        }
                    }
                };
                match (found, forward) {
                    (false, _) => Vec::new(),
                    (true, true) => sibs[a + 1..].to_vec(),
                    (true, false) => sibs[..a].iter().rev().copied().collect(),
                }
            }
        };
        (once, all)
    }

    // --- comparisons (native matcher/compare.rs `Comparison::holds`) ---

    /// `Some(verdict)`, or `None` where a transform's value is not known.
    fn compare(&self, left: Name, right: Name, op: CompareOp, text: CompareText, transform: bool, env: &Env) -> Option<bool> {
        if let (Some(l), Some(r)) = (env.get(left), env.get(right)) {
            let eq = match text {
                CompareText::Source => self.text_of(l) == self.text_of(r),
                CompareText::Node => self.range_of(l) == self.range_of(r) && self.kind_of(l) == self.kind_of(r),
                CompareText::Index => match (self.element_index(l), self.element_index(r)) {
                    (Some(a), Some(b)) => a == b,
                    _ => return Some(false),
                },
            };
            return Some(eq != (op == CompareOp::Ne));
        }
        // A text variable or a transform variable is text: compared as source text only.
        let side = |name: Name| -> Option<Option<String>> {
            if let Some(b) = env.get(name) {
                return Some(Some(self.text_of(b)));
            }
            if let Some(t) = env.text(name) {
                return Some(Some(t.to_owned()));
            }
            if transform { Some(self.transform_text(name, env)) } else { Some(None) }
        };
        let (l, r) = (side(left)?, side(right)?);
        Some(match (text, l, r) {
            (CompareText::Source, Some(l), Some(r)) => (l == r) != (op == CompareOp::Ne),
            _ => false,
        })
    }

    /// A list element's 0-based index: the `,` tokens before it among its parent's children,
    /// parentheses transparent (native compare.rs `element_index`).
    fn element_index(&self, b: Bound) -> Option<usize> {
        const LISTS: &[&str] = &[
            "array", "array_pattern", "arguments", "formal_parameters", "object", "object_pattern",
            "type_arguments", "type_parameters", "tuple_type", "named_imports", "export_clause", "list",
            "tuple", "set", "dictionary", "argument_list", "parameters", "lambda_parameters",
            "list_pattern", "tuple_pattern", "pattern_list", "expression_list",
        ];
        let Bound::Node(mut node) = b else { return None };
        let kind = |n: u32| facts::name(self.f.kind[n as usize]);
        if !self.f.named(node) || matches!(&*kind(node), "comment" | "html_comment" | "line_comment" | "block_comment") {
            return None;
        }
        let mut parent = self.f.parent(node)?;
        while &*kind(parent) == "parenthesized_expression" {
            node = parent;
            parent = self.f.parent(node)?;
        }
        if !LISTS.contains(&&*kind(parent)) {
            return None;
        }
        let mut commas = 0;
        for &c in self.f.children(parent) {
            if c == node {
                return Some(commas);
            }
            if !self.f.named(c) && self.f.text(c) == "," {
                commas += 1;
            }
        }
        None
    }

    /// A transform variable's text over the bindings so far (native `Transform::text`); `None`
    /// where a capture it derives from is unbound.
    fn transform_text(&self, name: Name, env: &Env) -> Option<String> {
        let t = &self.q.transform;
        let chain = t.chain(name);
        if chain.is_empty() {
            return None;
        }
        let mut scratch: Vec<(Name, String)> = env.texts.clone();
        for i in chain {
            let step = &t.steps[i];
            // The source's text: a capture's (a node, or an exported text), else a variable.
            let bound = env.get(step.source);
            let text = match bound {
                Some(b) => Some(self.text_of(b)),
                None => scratch.iter().find(|(n, _)| *n == step.source).map(|(_, s)| s.clone()),
            }
            .or_else(|| step.default.clone());
            let text = text?;
            let out = replace_all(&text, &step.regex, &step.by);
            // ast-grep de-indents a multi-line result relative to its source node.
            if out.contains('\n') && matches!(bound, Some(Bound::Node(_))) {
                self.refuse("transform: a multi-line result is de-indented (unsupported)".into());
                return None;
            }
            scratch.retain(|(n, _)| *n != step.name);
            scratch.push((step.name, out));
        }
        scratch.into_iter().find(|(n, _)| *n == name).map(|(_, s)| s)
    }

    // --- parse (matcher/parse.rs): text grammars (ast/parse_ext.rs), and annotation over
    // `string_type_definition` ---
    fn parse(&self, node: Bound, p: &ParseTest, env: &mut Env) -> bool {
        let Bound::Node(n) = node else { return false };
        // Trees cached per (host node, grammar, interpolation marker): `Some(range)` a tree,
        // `(NONE, NONE)` an unknown value, `None` no tree.
        let key = (n, p.grammar as u8, p.marker.clone());
        let cached = self.parsed_hosts.borrow().get(&key).copied();
        let tree = match cached {
            Some(t) => t,
            None => {
                let t = match p.grammar {
                    ParseGrammar::Annotation => {
                        // Native `ParseTest::tree_at`: a host of kind string/concatenated_string
                        // holds a tree iff it is a string type definition of the file's scope
                        // table (S4, SCHEMA § 1.3).
                        let kind = facts::name(self.f.kind[n as usize]);
                        if &*kind != "string" && &*kind != "concatenated_string" {
                            None
                        } else {
                            match self.f.typedefs() {
                                Err(why) => {
                                    self.refuse(why);
                                    None
                                }
                                Ok(defs) => crate::typedefs::at(defs, self.f.start[n as usize] as usize, self.f.end[n as usize] as usize).map(|a| {
                                    let mut parsed = self.parsed.borrow_mut();
                                    let base = parsed.len() as u32;
                                    for rec in &a.nodes {
                                        let parent = if rec.parent == NONE { NONE } else { base + rec.parent };
                                        parsed.push(ParsedNode { kind: rec.kind, start: rec.start, end: rec.end, text: rec.text.clone(), parent, named: rec.named });
                                    }
                                    (base, parsed.len() as u32)
                                }),
                            }
                        }
                    }
                    _ => self.text_tree(n, p.grammar, &p.marker),
                };
                self.parsed_hosts.borrow_mut().insert(key, t);
                t
            }
        };
        let Some(tree) = tree else { return false };
        match p.grammar {
            ParseGrammar::Annotation => {
                // The tree's rule runs on its root in an environment of its own; its captures are
                // exported to the host.
                let mut inner = Env::default();
                if !self.m(&p.rule, Bound::Parsed(tree.0), &mut inner) {
                    return false;
                }
                for (name, b) in inner.vars {
                    parse_ext::export(env, name, b);
                }
                true
            }
            _ => self.match_text_tree(n, tree, &p.rule, p.hosted, env),
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
