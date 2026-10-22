//! One policy set compiled for a call configuration, and its evaluation over one file's facts.
use crate::ast::{self, Bound, Compiler, Env, Name, Names, Query};
use crate::facts::Facts;
use crate::policy::{self, Scope};
use crate::rel::{self, RelPlan};
use crate::sql::engine::{Schema, Table};
use crate::sql::value::V;
use regex::Regex;
use serde_json::{Map, Value as Json};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone)]
pub enum Anchor {
    Span,
    FileStart,
    Capture(Name),
}

#[derive(Clone)]
pub enum Proj {
    Path,
    Directory,
    Name,
    Stem,
    Capture(Name),
    Text(Name),
    Kind(Name),
    Start(Name),
    End(Name),
    Line(Name),
    Regex(Box<Proj>, Regex, usize),
}

pub struct Input {
    pub query: Query,
    pub fields: Vec<String>,
    pub projs: Vec<Proj>,
    pub each: Option<Name>,
    pub anchor: Anchor,
    pub paths: Option<Scope>,
    /// Same-node regexes with named groups (for `capture.GROUP` and regex `each`).
    pub node_regexes: Vec<Regex>,
}

pub struct Viol {
    pub code: String,
    pub anchor: Anchor,
    pub each: Option<Name>,
}

pub enum RegexSource {
    Content,
    Path,
}

pub struct RegexClause {
    pub source: RegexSource,
    pub rx: Regex,
    pub include: Vec<Regex>,
    pub exclude: Vec<Regex>,
    pub lines: Option<bool>,
    pub paths: Option<Scope>,
}

pub enum Kind {
    ForbidAst { input: usize },
    ForbidRegex(RegexClause),
    Budget { input: usize, lte: u64 },
    Relational(Box<RelPlan>),
}

pub struct Clause {
    pub id: String,
    pub kind: Kind,
    pub viol: Viol,
}

pub struct Rule {
    pub id: String,
    pub check: String,
    pub scope: Scope,
    pub clauses: Vec<Clause>,
    pub inputs: Vec<Input>,
    pub names: Names,
}

fn proj_parse(s: &Json, names: &mut Names) -> Result<Proj, String> {
    if let Some(o) = s.as_object() {
        let src = proj_parse(o.get("source").ok_or("projection source")?, names)?;
        let rx = Regex::new(o["regex"].as_str().ok_or("projection regex")?).map_err(|e| e.to_string())?;
        let g = o["group"].as_u64().ok_or("projection group")? as usize;
        return Ok(Proj::Regex(Box::new(src), rx, g));
    }
    let s = s.as_str().ok_or("projection")?;
    Ok(match s {
        "file.path" => Proj::Path,
        "file.directory" => Proj::Directory,
        "file.name" => Proj::Name,
        "file.stem" => Proj::Stem,
        _ => {
            let rest = s.strip_prefix("capture.").ok_or_else(|| format!("projection {s}"))?;
            match rest.split_once('.') {
                None => Proj::Capture(names.id(rest)),
                Some((n, "text")) => Proj::Text(names.id(n)),
                Some((n, "kind")) => Proj::Kind(names.id(n)),
                Some((n, "startByte")) => Proj::Start(names.id(n)),
                Some((n, "endByte")) => Proj::End(names.id(n)),
                Some((n, "startLine")) => Proj::Line(names.id(n)),
                _ => return Err(format!("projection {s}")),
            }
        }
    })
}

fn anchor_parse(s: Option<&Json>, names: &mut Names) -> Result<Anchor, String> {
    Ok(match s.and_then(Json::as_str).unwrap_or("witness.span") {
        "witness.span" => Anchor::Span,
        "file.start" => Anchor::FileStart,
        a => Anchor::Capture(names.id(a.strip_prefix("capture.").ok_or_else(|| format!("anchor {a}"))?)),
    })
}

/// Regex atoms testing the root node itself, through all/any/utils (not not/relations).
fn node_regexes(rule: &ast::Rule, utils: &[ast::Rule], out: &mut Vec<Regex>, seen: &mut std::collections::BTreeSet<usize>) {
    match rule {
        ast::Rule::Regex(r) => {
            if r.capture_names().flatten().next().is_some() {
                out.push(r.clone());
            }
        }
        ast::Rule::All(v) | ast::Rule::Any(v) => v.iter().for_each(|r| node_regexes(r, utils, out, seen)),
        ast::Rule::Util(i) => {
            if seen.insert(*i) {
                node_regexes(&utils[*i], utils, out, seen);
            }
        }
        _ => {}
    }
}

impl Rule {
    pub fn compile(document: &Json, config: &Map<String, Json>, scope_override: Option<&Json>, grammar: &rerail_engine::code_rule_dsl::tape::Grammar) -> Result<Rule, String> {
        let doc = policy::resolve(document, config)?;
        let id = doc["id"].as_str().unwrap_or("").to_owned();
        let check = id.trim_start_matches("data-layer-").replace('-', "_");
        let scope = Scope::new(scope_override.or(doc.get("scope")))?;
        let mut utils: BTreeMap<String, Json> = BTreeMap::new();
        for c in doc["clauses"].as_array().ok_or("clauses")? {
            if let Some(u) = c.get("define").and_then(|d| d.get("utils")).and_then(Json::as_object) {
                for (k, v) in u {
                    utils.insert(k.clone(), v.clone());
                }
            }
        }
        let mut names = Names::default();
        let mut inputs: Vec<Input> = Vec::new();
        let mut keys: BTreeMap<String, usize> = BTreeMap::new();
        let mut clauses = Vec::new();
        let mut add_input = |ev: &Json, names: &mut Names, inputs: &mut Vec<Input>| -> Result<(usize, Vec<String>), String> {
            let key = serde_json::to_string(ev).unwrap();
            let project = ev.get("project").and_then(Json::as_object).cloned().unwrap_or_default();
            // Projected fields in name order (serde_json without preserve_order).
            let mut fields: Vec<String> = project.keys().cloned().collect();
            fields.sort();
            if let Some(&i) = keys.get(&key) {
                return Ok((i, fields));
            }
            let ast_json = ev.get("query").and_then(|q| q.get("ast")).ok_or("evidence query must be ast")?;
            let mut comp = Compiler::new(names, grammar, &utils);
            let query = comp.query(ast_json)?;
            let projs = fields.iter().map(|f| proj_parse(&project[f], names)).collect::<Result<Vec<_>, _>>()?;
            let each = ev.get("each").and_then(Json::as_str).map(|e| names.id(e));
            let anchor = anchor_parse(ev.get("anchor"), names)?;
            let paths = ev.get("paths").map(|p| {
                let mut p = p.clone();
                if p.get("include").is_none() {
                    p["include"] = serde_json::json!(["**"]);
                }
                if p.get("exclude").is_none() {
                    p["exclude"] = serde_json::json!([]);
                }
                Scope::new(Some(&p))
            }).transpose()?;
            let mut nr = Vec::new();
            node_regexes(&query.rule, &query.utils, &mut nr, &mut Default::default());
            for rx in &nr {
                for g in rx.capture_names().flatten() {
                    names.id(g);
                }
            }
            inputs.push(Input { query, fields: fields.clone(), projs, each, anchor, paths, node_regexes: nr });
            keys.insert(key, inputs.len() - 1);
            Ok((inputs.len() - 1, fields))
        };
        for c in doc["clauses"].as_array().unwrap() {
            if c.get("define").is_some() {
                continue;
            }
            let cid = c["id"].as_str().unwrap_or("").to_owned();
            let v = &c["violation"];
            let viol = Viol {
                code: v["code"].as_str().unwrap_or("").to_owned(),
                anchor: anchor_parse(v.get("anchor"), &mut names)?,
                each: v.get("each").and_then(Json::as_str).map(|e| names.id(e)),
            };
            let kind = if let Some(f) = c.get("forbid") {
                if let Some(a) = f.get("ast") {
                    let ev = serde_json::json!({ "as": "row", "query": { "ast": a }, "project": {} });
                    let (i, _) = add_input(&ev, &mut names, &mut inputs).map_err(|e| format!("{cid}: {e}"))?;
                    Kind::ForbidAst { input: i }
                } else if let Some(r) = f.get("regex") {
                    let rx_list = |k: &str| -> Result<Vec<Regex>, String> {
                        r.get(k).and_then(Json::as_array).into_iter().flatten().map(|x| Regex::new(x.as_str().unwrap_or("")).map_err(|e| e.to_string())).collect()
                    };
                    if r.get("content").is_some() {
                        return Err(format!("{cid}: regex content gate unsupported"));
                    }
                    let lines = match r.get("lines") {
                        None => None,
                        Some(l) => {
                            if l.get("terminators").and_then(Json::as_str).unwrap_or("python") != "python" {
                                return Err(format!("{cid}: terminators"));
                            }
                            Some(l.get("first").and_then(Json::as_bool).unwrap_or(false))
                        }
                    };
                    Kind::ForbidRegex(RegexClause {
                        source: match r["source"].as_str() {
                            Some("file.content") => RegexSource::Content,
                            Some("file.path") => RegexSource::Path,
                            s => return Err(format!("{cid}: regex source {s:?}")),
                        },
                        rx: Regex::new(r["pattern"].as_str().ok_or("pattern")?).map_err(|e| e.to_string())?,
                        include: rx_list("include")?,
                        exclude: rx_list("exclude")?,
                        lines,
                        paths: r.get("paths").map(|p| Scope::new(Some(p))).transpose()?,
                    })
                } else {
                    return Err(format!("{cid}: forbid form"));
                }
            } else if let Some(b) = c.get("budget") {
                let ev = b["query"]["evidence"].clone();
                let gb: Vec<&str> = b.get("group_by").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).collect()).unwrap_or_default();
                if gb != ["row.path", "row.line"] || b["count"].get("distinct").is_some() {
                    return Err(format!("{cid}: budget shape unsupported"));
                }
                let proj = ev.get("project").cloned().unwrap_or_default();
                if proj.get("path").and_then(Json::as_str) != Some("file.path") || !proj.get("line").and_then(Json::as_str).is_some_and(|l| l.ends_with(".startLine")) {
                    return Err(format!("{cid}: budget projection unsupported"));
                }
                let (i, _) = add_input(&ev, &mut names, &mut inputs).map_err(|e| format!("{cid}: {e}"))?;
                Kind::Budget { input: i, lte: b["count"]["lte"].as_u64().ok_or("lte")? }
            } else if let Some(r) = c.get("relational") {
                let mut ins = Vec::new();
                let mut named: Vec<(&String, &Json)> = r["inputs"].as_object().ok_or("inputs")?.iter().collect();
                named.sort_by(|a, b| a.0.cmp(b.0));
                for (name, inp) in named {
                    let (i, fields) = add_input(&inp["evidence"], &mut names, &mut inputs).map_err(|e| format!("{cid}/{name}: {e}"))?;
                    ins.push((name.clone(), i, fields));
                }
                Kind::Relational(Box::new(RelPlan::compile(r, ins, grammar).map_err(|e| format!("{cid}: {e}"))?))
            } else {
                return Err(format!("{cid}: clause kind"));
            };
            clauses.push(Clause { id: cid, kind, viol });
        }
        Ok(Rule { id, check, scope, clauses, inputs, names })
    }
}

/// An input's rows for one file.
pub struct Rows {
    pub table: Table,
    /// Anchor spans with their lines (0-based) and witness spans.
    pub anchors: Vec<(u32, u32, u32)>,
    pub witnesses: Vec<(u32, u32)>,
    pub unsupported: Option<String>,
}

pub struct FileCtx<'a> {
    pub f: &'a Facts,
    pub path: &'a str,
    pub source: &'a str,
}

fn line_of(f: &Facts, byte: u32) -> u32 {
    f.lines.line(byte as usize) as u32
}

impl Input {
    pub fn rows(&self, ctx: &FileCtx<'_>, schema: &Schema) -> Rows {
        let mut out = Rows { table: Table { schema: schema.clone(), rows: Vec::new() }, anchors: Vec::new(), witnesses: Vec::new(), unsupported: None };
        let f = ctx.f;
        match self.query.syntax {
            ast::SyntaxMode::Valid if f.syntax_errors => return out,
            ast::SyntaxMode::Invalid if !f.syntax_errors => return out,
            ast::SyntaxMode::Unspecified if f.syntax_errors => {
                out.unsupported = Some("query without `syntax` on a file with syntax errors".into());
                return out;
            }
            _ => {}
        }
        let ev = ast::Eval::new(f, ctx.path, &self.query);
        let path: Rc<str> = Rc::from(ctx.path);
        let each_is_group = self.each.is_some_and(|e| !self.query_binds(e));
        for root in ev.roots() {
            let Some(env) = ev.match_root(root) else { continue };
            let mut emit = |env: Env, groups: &[(Name, (u32, u32), String)]| {
                let witness = (f.start[root as usize], f.end[root as usize]);
                let span = match &self.anchor {
                    Anchor::Span => witness,
                    Anchor::FileStart => (0, 0),
                    Anchor::Capture(n) => capture_range(&ev, &env, groups, *n).unwrap_or(witness),
                };
                let mut row = vec![V::Text(path.clone()), V::Int(span.0 as i64), V::Int(span.1 as i64)];
                for p in &self.projs {
                    row.push(project(p, &ev, &env, groups, ctx, &self.query));
                }
                out.table.rows.push(row.into_boxed_slice());
                let line = if matches!(self.anchor, Anchor::FileStart) { 0 } else { line_of(f, span.0) };
                out.anchors.push((span.0, span.1, line));
                out.witnesses.push(witness);
            };
            match self.each {
                Some(name) if each_is_group => {
                    // Regex-group `each`: one witness per new range of the group, over every
                    // holding same-node regex's matches.
                    let text = f.text(root);
                    let base = f.start[root as usize];
                    let gname = ev_name(&self.query, name);
                    let mut seen = Vec::new();
                    for (ri, rx) in self.node_regexes.iter().enumerate() {
                        let Some(gname) = gname.as_deref() else { continue };
                        if rx.capture_names().flatten().all(|n| n != gname) {
                            continue;
                        }
                        for caps in rx.captures_iter(text) {
                            let Some(m) = caps.name(gname) else { continue };
                            let r = (base + m.start() as u32, base + m.end() as u32);
                            if seen.contains(&r) {
                                continue;
                            }
                            seen.push(r);
                            let mut groups = Vec::new();
                            for n in rx.capture_names().flatten() {
                                if let Some(g) = caps.name(n) {
                                    groups.push((self.query_name(n), (base + g.start() as u32, base + g.end() as u32), g.as_str().to_owned()));
                                }
                            }
                            // Other regexes' first-match groups not shadowed.
                            for (oi, orx) in self.node_regexes.iter().enumerate() {
                                if oi == ri {
                                    continue;
                                }
                                if let Some(oc) = orx.captures(text) {
                                    for n in orx.capture_names().flatten() {
                                        if groups.iter().any(|(gn, _, _)| Some(*gn) == self.query_name_opt(n)) {
                                            continue;
                                        }
                                        if let Some(g) = oc.name(n) {
                                            groups.push((self.query_name(n), (base + g.start() as u32, base + g.end() as u32), g.as_str().to_owned()));
                                        }
                                    }
                                }
                            }
                            emit(env.clone(), &groups);
                        }
                    }
                }
                Some(name) => {
                    let groups = self.first_groups(f, root);
                    let mut envs = Vec::new();
                    ev.enumerate(root, env, name, |e| envs.push(e));
                    for e in envs {
                        emit(e, &groups);
                    }
                }
                None => {
                    let groups = self.first_groups(f, root);
                    emit(env, &groups);
                }
            }
            if ev.unsupported.borrow().is_some() {
                break;
            }
        }
        if let Some(u) = ev.unsupported.borrow_mut().take() {
            out.unsupported = Some(u);
        }
        if crate::pyvalue::NAMED_ESCAPES.with(|n| n.replace(0)) > 0 && out.unsupported.is_none() {
            out.unsupported = Some("`\\N{...}` escape (needs the Unicode name table)".into());
        }
        out
    }

    fn query_binds(&self, name: Name) -> bool {
        // A metavariable `each` (the rule binds it) vs a regex group `each`.
        fn walk(r: &ast::Rule, utils: &[ast::Rule], name: Name, seen: &mut std::collections::BTreeSet<usize>) -> bool {
            match r {
                ast::Rule::MetaVar { name: Some(n), .. } | ast::Rule::Capture(n) => *n == name,
                ast::Rule::All(v) | ast::Rule::Any(v) => v.iter().any(|x| walk(x, utils, name, seen)),
                ast::Rule::Not(x) => walk(x, utils, name, seen),
                ast::Rule::Has { rule, .. } | ast::Rule::Inside { rule, .. } | ast::Rule::Field { rule, .. } => walk(rule, utils, name, seen),
                ast::Rule::NthChild { of_rule: Some(x), .. } => walk(x, utils, name, seen),
                ast::Rule::Parse(p) => walk(&p.rule, utils, name, seen),
                ast::Rule::Util(i) => seen.insert(*i) && walk(&utils[*i], utils, name, seen),
                _ => false,
            }
        }
        walk(&self.query.rule, &self.query.utils, name, &mut Default::default())
    }

    fn query_name(&self, n: &str) -> Name {
        self.query_name_opt(n).unwrap_or(Name::MAX)
    }
    fn query_name_opt(&self, n: &str) -> Option<Name> {
        GROUP_NAMES.with(|g| g.borrow().get(n).copied())
    }

    /// First-match named groups of the same-node regexes at `root`.
    fn first_groups(&self, f: &Facts, root: u32) -> Vec<(Name, (u32, u32), String)> {
        if self.node_regexes.is_empty() {
            return Vec::new();
        }
        let text = f.text(root);
        let base = f.start[root as usize];
        let mut groups: Vec<(Name, (u32, u32), String)> = Vec::new();
        for rx in &self.node_regexes {
            if let Some(c) = rx.captures(text) {
                for n in rx.capture_names().flatten() {
                    let id = self.query_name(n);
                    if groups.iter().any(|(g, _, _)| *g == id) {
                        continue;
                    }
                    if let Some(g) = c.name(n) {
                        groups.push((id, (base + g.start() as u32, base + g.end() as u32), g.as_str().to_owned()));
                    }
                }
            }
        }
        groups
    }
}

thread_local! {
    /// Capture names of the rule being evaluated, by name (for regex groups).
    pub static GROUP_NAMES: std::cell::RefCell<BTreeMap<String, Name>> = const { std::cell::RefCell::new(BTreeMap::new()) };
}

fn ev_name(_q: &Query, n: Name) -> Option<String> {
    GROUP_NAMES.with(|g| g.borrow().iter().find(|(_, v)| **v == n).map(|(k, _)| k.clone()))
}

fn capture_range(ev: &ast::Eval<'_>, env: &Env, groups: &[(Name, (u32, u32), String)], n: Name) -> Option<(u32, u32)> {
    if let Some(b) = env.get(n) {
        return Some(ev.range_of(b));
    }
    groups.iter().find(|(g, _, _)| *g == n).map(|(_, r, _)| *r)
}

fn project(p: &Proj, ev: &ast::Eval<'_>, env: &Env, groups: &[(Name, (u32, u32), String)], ctx: &FileCtx<'_>, q: &Query) -> V {
    let f = ctx.f;
    match p {
        Proj::Path => V::text(ctx.path),
        Proj::Directory => V::text(ctx.path.rsplit_once('/').map_or("", |(d, _)| d)),
        Proj::Name => V::text(ctx.path.rsplit_once('/').map_or(ctx.path, |(_, n)| n)),
        Proj::Stem => {
            let name = ctx.path.rsplit_once('/').map_or(ctx.path, |(_, n)| n);
            V::text(name.split_once('.').map_or(name, |(s, _)| s))
        }
        Proj::Capture(n) => match env.get(*n) {
            Some(b) => V::text(&ev.text_of(b)),
            None => groups.iter().find(|(g, _, _)| g == n).map_or(V::Null, |(_, _, t)| V::text(t)),
        },
        Proj::Text(n) => {
            if let Some((_, src)) = q.decoded.iter().find(|(t, _)| t == n) {
                // `DECODED`: str() of the source capture's value, "" when unknown or unbound.
                return V::text(&match env.get(*src) {
                    Some(Bound::Node(node)) => match crate::pyvalue::decode(f, node) {
                        Ok(crate::pyvalue::PyValue::Str(s)) => s,
                        Ok(crate::pyvalue::PyValue::Other) => {
                            *ev.unsupported.borrow_mut() = Some("DECODED of a non-string constant".into());
                            String::new()
                        }
                        Ok(_) => String::new(),
                        Err(e) => {
                            *ev.unsupported.borrow_mut() = Some(e);
                            String::new()
                        }
                    },
                    _ => String::new(),
                });
            }
            match env.get(*n) {
                Some(b) => V::text(&ev.text_of(b)),
                None => groups.iter().find(|(g, _, _)| g == n).map_or(V::Null, |(_, _, t)| V::text(t)),
            }
        }
        Proj::Kind(n) => match env.get(*n) {
            Some(Bound::Node(node)) => V::Text(crate::facts::name(f.kind[node as usize])),
            _ => V::Null,
        },
        Proj::Start(n) => capture_range(ev, env, groups, *n).map_or(V::Null, |r| V::Int(r.0 as i64)),
        Proj::End(n) => capture_range(ev, env, groups, *n).map_or(V::Null, |r| V::Int(r.1 as i64)),
        Proj::Line(n) => capture_range(ev, env, groups, *n).map_or(V::Null, |r| V::Int(line_of(f, r.0) as i64)),
        Proj::Regex(src, rx, g) => match project(src, ev, env, groups, ctx, q) {
            V::Text(t) => rx.captures(&t).and_then(|c| c.get(*g)).map_or(V::Null, |m| V::text(m.as_str())),
            _ => V::Null,
        },
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Finding {
    pub check: String,
    pub clause: String,
    pub path: String,
    pub line: u32,
    pub code: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Failure {
    pub check: String,
    pub clause: String,
    pub path: String,
    pub message: String,
}

#[derive(Default)]
pub struct Output {
    pub findings: Vec<Finding>,
    pub failures: Vec<Failure>,
    /// (rule index, clause index) -> (calls, nanoseconds)
    pub clause_ns: BTreeMap<(usize, usize), (u64, u64)>,
    /// One sample per (clause, file) evaluation: ((rule index, clause index), nanoseconds).
    pub clause_samples: Vec<((u16, u16), u64)>,
    pub input_ns: u64,
}

impl Output {
    /// Folds another worker's output in (findings and failures are multisets; order is free).
    pub fn merge(&mut self, other: Output) {
        self.findings.extend(other.findings);
        self.failures.extend(other.failures);
        for (k, (calls, ns)) in other.clause_ns {
            let e = self.clause_ns.entry(k).or_default();
            e.0 += calls;
            e.1 += ns;
        }
        self.clause_samples.extend(other.clause_samples);
        self.input_ns += other.input_ns;
    }
}

/// Repository-partition accumulators: per (rule, clause), input tables and anchors.
#[derive(Default)]
pub struct RepoAcc {
    pub inputs: BTreeMap<(usize, usize), (Vec<Table>, rustc_hash::FxHashMap<(Rc<str>, u32, u32), u32>, bool)>,
}

/// A value that may cross threads (`V` shares text through `Rc`).
pub enum SendV {
    Null,
    Int(i64),
    Real(f64),
    Text(Box<str>),
}

/// A worker's `RepoAcc` and budget rows, detached from its `Rc`s so they can reach the
/// merging thread.
#[derive(Default)]
pub struct SendAcc {
    inputs: Vec<((usize, usize), Vec<(Schema, Vec<Box<[SendV]>>)>, Vec<((Box<str>, u32, u32), u32)>, bool)>,
    budget: BTreeMap<(usize, usize), Vec<(String, u32)>>,
}

impl RepoAcc {
    pub fn detach(self, budget: BTreeMap<(usize, usize), Vec<(String, u32)>>) -> SendAcc {
        let send_v = |v: &V| match v {
            V::Null => SendV::Null,
            V::Int(i) => SendV::Int(*i),
            V::Real(r) => SendV::Real(*r),
            V::Text(s) => SendV::Text(Box::from(&**s)),
        };
        let inputs = self
            .inputs
            .into_iter()
            .map(|(k, (tables, anchors, flag))| {
                let tables = tables.into_iter().map(|t| (t.schema, t.rows.iter().map(|r| r.iter().map(send_v).collect()).collect())).collect();
                let anchors = anchors.into_iter().map(|((p, s, e), l)| ((Box::from(&*p), s, e), l)).collect();
                (k, tables, anchors, flag)
            })
            .collect();
        SendAcc { inputs, budget }
    }
}

/// The merge of every worker's `SendAcc`, back on one thread. Repeated texts (paths, kinds)
/// share one `Rc` again. Row order within a table is free: `finish` sorts rows by path, stably,
/// and a file's rows all come from the one worker that evaluated it.
pub fn attach(parts: Vec<SendAcc>) -> (RepoAcc, BTreeMap<(usize, usize), Vec<(String, u32)>>) {
    let mut texts: rustc_hash::FxHashMap<Box<str>, Rc<str>> = Default::default();
    let mut text = |s: Box<str>| -> Rc<str> {
        if let Some(r) = texts.get(&s) {
            return r.clone();
        }
        let r: Rc<str> = Rc::from(&*s);
        texts.insert(s, r.clone());
        r
    };
    let mut repo = RepoAcc::default();
    let mut budget: BTreeMap<(usize, usize), Vec<(String, u32)>> = BTreeMap::new();
    for part in parts {
        for (k, tables, anchors, flag) in part.inputs {
            let n = tables.len();
            let acc = repo.inputs.entry(k).or_insert_with(|| ((0..n).map(|_| Table::default()).collect(), Default::default(), false));
            for (slot, (schema, rows)) in acc.0.iter_mut().zip(tables) {
                slot.schema = schema;
                slot.rows.extend(rows.into_iter().map(|r| {
                    r.into_vec()
                        .into_iter()
                        .map(|v| match v {
                            SendV::Null => V::Null,
                            SendV::Int(i) => V::Int(i),
                            SendV::Real(x) => V::Real(x),
                            SendV::Text(s) => V::Text(text(s)),
                        })
                        .collect()
                }));
            }
            for ((p, s, e), l) in anchors {
                acc.1.insert((text(p), s, e), l);
            }
            acc.2 |= flag;
        }
        for (k, rows) in part.budget {
            budget.entry(k).or_default().extend(rows);
        }
    }
    (repo, budget)
}

pub fn eval_file(rules: &[Rule], ctx: &FileCtx<'_>, out: &mut Output, repo: &mut RepoAcc, budget: &mut BTreeMap<(usize, usize), Vec<(String, u32)>>) {
    for (ri, rule) in rules.iter().enumerate() {
        if !rule.scope.matches(ctx.path) {
            continue;
        }
        GROUP_NAMES.with(|g| {
            let mut g = g.borrow_mut();
            g.clear();
            for (i, n) in rule.names.names.iter().enumerate() {
                g.insert(n.clone(), i as Name);
            }
        });
        let mut cache: BTreeMap<usize, Rc<Rows>> = BTreeMap::new();
        let mut get = |i: usize, out: &mut Output| -> Option<Rc<Rows>> {
            let inp = &rule.inputs[i];
            if inp.paths.as_ref().is_some_and(|p| !p.matches(ctx.path)) {
                return None;
            }
            if let Some(r) = cache.get(&i) {
                return Some(r.clone());
            }
            let t0 = Instant::now();
            let schema = rel::input_schema(&inp.fields);
            let r = Rc::new(inp.rows(ctx, &schema));
            out.input_ns += t0.elapsed().as_nanos() as u64;
            cache.insert(i, r.clone());
            Some(r)
        };
        for (ci, clause) in rule.clauses.iter().enumerate() {
            let t0 = Instant::now();
            let fail = |out: &mut Output, msg: String| {
                out.failures.push(Failure { check: rule.check.clone(), clause: clause.id.clone(), path: ctx.path.to_owned(), message: msg });
            };
            match &clause.kind {
                Kind::ForbidAst { input } => {
                    let inp = &rule.inputs[*input];
                    // Witnesses with `violation.each` and anchors.
                    let f = ctx.f;
                    match inp.query.syntax {
                        ast::SyntaxMode::Valid if f.syntax_errors => {}
                        ast::SyntaxMode::Invalid if !f.syntax_errors => {}
                        _ => {
                            let ev = ast::Eval::new(f, ctx.path, &inp.query);
                            for root in ev.roots() {
                                let Some(env) = ev.match_root(root) else { continue };
                                let witness = (f.start[root as usize], f.end[root as usize]);
                                let mut envs = Vec::new();
                                match clause.viol.each {
                                    Some(name) => ev.enumerate(root, env, name, |e| envs.push(e)),
                                    None => envs.push(env),
                                }
                                let groups = inp.first_groups(f, root);
                                for e in envs {
                                    let span = match &clause.viol.anchor {
                                        Anchor::Span => witness,
                                        Anchor::FileStart => (0, 0),
                                        Anchor::Capture(n) => capture_range(&ev, &e, &groups, *n).unwrap_or(witness),
                                    };
                                    let line = if matches!(clause.viol.anchor, Anchor::FileStart) { 0 } else { line_of(f, span.0) };
                                    out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: ctx.path.to_owned(), line: line + 1, code: clause.viol.code.clone() });
                                }
                            }
                            if let Some(u) = ev.unsupported.borrow_mut().take() {
                                fail(out, format!("unsupported: {u}"));
                            }
                        }
                    }
                }
                Kind::ForbidRegex(rc) => {
                    if rc.paths.as_ref().is_none_or(|p| p.matches(ctx.path)) {
                        regex_findings(rc, ctx, &mut |line| {
                            out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: ctx.path.to_owned(), line: line + 1, code: clause.viol.code.clone() });
                        }, &clause.viol.anchor, &rule.names);
                    }
                }
                Kind::Budget { input, .. } => {
                    if let Some(rows) = get(*input, out) {
                        if let Some(u) = &rows.unsupported {
                            fail(out, format!("unsupported: {u}"));
                        }
                        let acc = budget.entry((ri, ci)).or_default();
                        // Groups by (path, start line of the root); the first row's witness reports.
                        for (k, row) in rows.table.rows.iter().enumerate() {
                            let line = match &row[4] {
                                V::Int(l) => *l as u32,
                                _ => u32::MAX,
                            };
                            let wline = line_of(ctx.f, rows.witnesses[k].0);
                            let _ = line;
                            acc.push((ctx.path.to_owned(), wline));
                        }
                    }
                }
                Kind::Relational(plan) => {
                    let mut tables = Vec::new();
                    let mut anchors: rustc_hash::FxHashMap<(Rc<str>, u32, u32), u32> = Default::default();
                    let mut unsupported = None;
                    let path: Rc<str> = Rc::from(ctx.path);
                    for (k, (_, idx)) in plan.inputs.iter().enumerate() {
                        match get(*idx, out) {
                            Some(rows) => {
                                if rows.unsupported.is_some() {
                                    unsupported = rows.unsupported.clone();
                                }
                                for (s, e, l) in &rows.anchors {
                                    anchors.insert((path.clone(), *s, *e), *l);
                                }
                                tables.push(Table { schema: rows.table.schema.clone(), rows: rows.table.rows.clone() });
                            }
                            None => tables.push(Table { schema: plan.input_schemas[k].clone(), rows: Vec::new() }),
                        }
                    }
                    if let Some(u) = unsupported {
                        fail(out, format!("unsupported: {u}"));
                    } else if plan.per_file {
                        let r = rel::run(plan, rel::Partition { inputs: tables, anchors, facts: Some((ctx.f, ctx.path)), source: ctx.source, path: ctx.path });
                        if let Some(m) = r.failure {
                            fail(out, m);
                        }
                        for (p, line, code) in r.findings {
                            out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p, line: line + 1, code: if code.is_empty() { clause.viol.code.clone() } else { code } });
                        }
                    } else {
                        let acc = repo.inputs.entry((ri, ci)).or_insert_with(|| (plan.input_schemas.iter().map(|s| Table { schema: s.clone(), rows: Vec::new() }).collect(), Default::default(), false));
                        for (k, t) in tables.into_iter().enumerate() {
                            acc.0[k].rows.extend(t.rows);
                        }
                        acc.1.extend(anchors);
                    }
                }
            }
            let ns = t0.elapsed().as_nanos() as u64;
            let e = out.clause_ns.entry((ri, ci)).or_default();
            e.0 += 1;
            e.1 += ns;
            out.clause_samples.push(((ri as u16, ci as u16), ns));
        }
    }
}

/// Repository partitions and budgets, after every file.
pub fn finish(rules: &[Rule], repo: RepoAcc, budget: BTreeMap<(usize, usize), Vec<(String, u32)>>, out: &mut Output) {
    for ((ri, ci), mut rows) in budget {
        let rule = &rules[ri];
        let clause = &rule.clauses[ci];
        let Kind::Budget { lte, .. } = &clause.kind else { continue };
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut counts: Vec<((String, u32), u64)> = Vec::new();
        let mut index: std::collections::HashMap<(String, u32), usize> = Default::default();
        for r in rows {
            match index.get(&r) {
                Some(&i) => counts[i].1 += 1,
                None => {
                    index.insert(r.clone(), counts.len());
                    counts.push((r, 1));
                }
            }
        }
        for ((p, line), n) in counts {
            if n > *lte {
                out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p, line: line + 1, code: clause.viol.code.clone() });
            }
        }
    }
    // Every repository-partitioned clause runs once, over the rows of whatever files reached it
    // (none at all is still one partition).
    let mut repo = repo;
    for (ri, rule) in rules.iter().enumerate() {
        for (ci, clause) in rule.clauses.iter().enumerate() {
            if let Kind::Relational(plan) = &clause.kind {
                if !plan.per_file {
                    repo.inputs.entry((ri, ci)).or_insert_with(|| (plan.input_schemas.iter().map(|s| Table { schema: s.clone(), rows: Vec::new() }).collect(), Default::default(), false));
                }
            }
        }
    }
    for ((ri, ci), (mut inputs, anchors, _)) in repo.inputs {
        let rule = &rules[ri];
        let clause = &rule.clauses[ci];
        let Kind::Relational(plan) = &clause.kind else { continue };
        let t0 = Instant::now();
        // Rows in path order, files in path order (stable within a file).
        for t in inputs.iter_mut() {
            t.rows.sort_by(|a, b| match (&a[0], &b[0]) {
                (V::Text(x), V::Text(y)) => x.cmp(y),
                _ => std::cmp::Ordering::Equal,
            });
        }
        let r = rel::run(plan, rel::Partition { inputs, anchors, facts: None, source: "", path: "" });
        if let Some(m) = r.failure {
            out.failures.push(Failure { check: rule.check.clone(), clause: clause.id.clone(), path: String::new(), message: m });
        }
        for (p, line, code) in r.findings {
            out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p, line: line + 1, code: if code.is_empty() { clause.viol.code.clone() } else { code } });
        }
        let e = out.clause_ns.entry((ri, ci)).or_default();
        e.0 += 1;
        e.1 += t0.elapsed().as_nanos() as u64;
    }
}

fn regex_findings(rc: &RegexClause, ctx: &FileCtx<'_>, emit: &mut dyn FnMut(u32), anchor: &Anchor, names: &Names) {
    let keep = |t: &str| rc.include.iter().all(|r| r.is_match(t)) && !rc.exclude.iter().any(|r| r.is_match(t));
    match rc.source {
        RegexSource::Path => {
            for m in rc.rx.find_iter(ctx.path) {
                if keep(m.as_str()) {
                    emit(0);
                }
            }
        }
        RegexSource::Content => {
            let text = ctx.source;
            match rc.lines {
                None => {
                    for caps in rc.rx.captures_iter(text) {
                        let m = caps.get(0).unwrap();
                        if !keep(m.as_str()) {
                            continue;
                        }
                        let start = match anchor {
                            Anchor::Capture(n) => {
                                let name = &names.names[*n as usize];
                                caps.name(name).map_or(m.start(), |g| g.start())
                            }
                            _ => m.start(),
                        };
                        emit(line_of(ctx.f, start as u32));
                    }
                }
                Some(first) => {
                    for (li, line) in python_splitlines(text).into_iter().enumerate() {
                        for m in rc.rx.find_iter(line) {
                            if keep(m.as_str()) {
                                emit(li as u32);
                                if first {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Python `str.splitlines()` boundaries: lines without their terminators; a final empty line
/// after the last terminator is not a line.
fn python_splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let term = matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if term {
            out.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|(_, n)| *n == '\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}
