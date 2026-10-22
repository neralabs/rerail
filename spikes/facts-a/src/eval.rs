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
                let mut plan = RelPlan::compile(r, ins, grammar).map_err(|e| format!("{cid}: {e}"))?;
                crate::subst::plan_clause(r, &mut plan, &inputs, &mut names, grammar, &utils).map_err(|e| format!("{cid}: {e}"))?;
                Kind::Relational(Box::new(plan))
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

/// Spike A3: a dump group's member tables in one pass over the facts (`subst::DumpGroup`): per
/// member, its own selection over its candidate roots in preorder; the shared gate once per node;
/// the captures from the facts (`$NODE` named, the parent when named, per field the first child
/// with that field when named). Rows, their order and the anchors are `Input::rows`' for each
/// member. `None` when any evaluation refused or decoded a named escape: the caller then runs
/// the members' own queries, so failures stay exactly the original path's.
pub fn dump_rows(rule: &Rule, g: &crate::subst::DumpGroup, plan: &RelPlan, ctx: &FileCtx<'_>) -> Option<(Vec<Table>, Vec<(u32, u32, u32)>)> {
    let f = ctx.f;
    let mut tables: Vec<Table> = g.members.iter().map(|m| Table { schema: plan.input_schemas[m.plan_input].clone(), rows: Vec::new() }).collect();
    let mut anchors = Vec::new();
    if f.syntax_errors {
        // `syntax: valid` (checked at compile time): no rows.
        return Some((tables, anchors));
    }
    let saved = crate::pyvalue::NAMED_ESCAPES.with(|n| n.replace(0));
    let gate = ast::Eval::new(f, ctx.path, &g.gate);
    // Gate verdict per node: 0 not yet evaluated, 1 holds, 2 does not.
    let mut verdict = vec![0u8; f.len()];
    let path: Rc<str> = Rc::from(ctx.path);
    let mut refused = false;
    let mut env = Env::default();
    // The ancestor capture: X per node (memoised), and per node the nearest proper ancestor
    // where X holds (`NONE`: none; `PENDING`: not computed yet).
    let anc_ev = g.ancestor.as_ref().map(|(_, xq)| ast::Eval::new(f, ctx.path, xq));
    let mut near = if anc_ev.is_some() { vec![PENDING; f.len()] } else { Vec::new() };
    let mut xval = if anc_ev.is_some() { vec![0u8; f.len()] } else { Vec::new() };
    let mut chain: Vec<u32> = Vec::new();
    for (mi, m) in g.members.iter().enumerate() {
        let inp = &rule.inputs[m.rule_input];
        let ev = ast::Eval::new(f, ctx.path, &m.sel);
        let rows = &mut tables[mi].rows;
        for root in ev.roots() {
            if ev.match_root(root).is_none() {
                continue;
            }
            let v = &mut verdict[root as usize];
            if *v == 0 {
                *v = if gate.match_root(root).is_some() { 1 } else { 2 };
            }
            if *v != 1 || !f.named(root) {
                continue;
            }
            env.vars.clear();
            env.vars.push((m.tail.node, Bound::Node(root)));
            if let Some(p) = m.tail.parent {
                if let Some(par) = f.parent(root).filter(|&x| f.named(x)) {
                    env.vars.push((p, Bound::Node(par)));
                }
            }
            for &(name, code) in &m.tail.fields {
                let Some(code) = code else { continue };
                if let Some(c) = f.children(root).iter().copied().find(|&c| f.field[c as usize] == code) {
                    if f.named(c) {
                        env.vars.push((name, Bound::Node(c)));
                    }
                }
            }
            if let (Some((aname, _)), Some(xev)) = (&g.ancestor, &anc_ev) {
                let a = nearest_where(f, xev, root, &mut near, &mut xval, &mut chain);
                if a != crate::facts::NONE && f.named(a) {
                    env.vars.push((*aname, Bound::Node(a)));
                }
            }
            let (s, e) = (f.start[root as usize], f.end[root as usize]);
            let mut row = Vec::with_capacity(3 + inp.projs.len());
            row.push(V::Text(path.clone()));
            row.push(V::Int(s as i64));
            row.push(V::Int(e as i64));
            for p in &inp.projs {
                row.push(match p {
                    Proj::Capture(n) | Proj::Text(n) => match env.get(*n) {
                        Some(Bound::Node(x)) => V::text(f.text(x)),
                        _ => V::Null,
                    },
                    Proj::Kind(n) => match env.get(*n) {
                        Some(Bound::Node(x)) => V::Text(crate::facts::name(f.kind[x as usize])),
                        _ => V::Null,
                    },
                    Proj::Start(n) => env.get(*n).map_or(V::Null, |b| V::Int(gate.range_of(b).0 as i64)),
                    Proj::End(n) => env.get(*n).map_or(V::Null, |b| V::Int(gate.range_of(b).1 as i64)),
                    Proj::Line(n) => env.get(*n).map_or(V::Null, |b| V::Int(line_of(f, gate.range_of(b).0) as i64)),
                    _ => project(p, &gate, &env, &[], ctx, &inp.query),
                });
            }
            rows.push(row.into_boxed_slice());
            anchors.push((s, e, line_of(f, s)));
        }
        if ev.unsupported.borrow().is_some() {
            refused = true;
            break;
        }
    }
    if gate.unsupported.borrow().is_some() {
        refused = true;
    }
    if crate::pyvalue::NAMED_ESCAPES.with(|n| n.replace(saved)) > 0 {
        refused = true;
    }
    (!refused).then_some((tables, anchors))
}

const PENDING: u32 = u32::MAX - 1;

/// The nearest proper ancestor of `n` where `x` holds (`inside {all[X, $A]}, stopBy: X` with X
/// two-valued: the walk stops at the first ancestor where X holds), memoised in `near`.
fn nearest_where(f: &Facts, x: &ast::Eval<'_>, n: u32, near: &mut [u32], xval: &mut [u8], chain: &mut Vec<u32>) -> u32 {
    chain.clear();
    let mut cur = n;
    let ans = loop {
        if near[cur as usize] != PENDING {
            break near[cur as usize];
        }
        chain.push(cur);
        match f.parent(cur) {
            None => break crate::facts::NONE,
            Some(p) => {
                let v = &mut xval[p as usize];
                if *v == 0 {
                    *v = if x.match_root(p).is_some() { 1 } else { 2 };
                }
                if *v == 1 {
                    break p;
                }
                cur = p;
            }
        }
    };
    for &c in chain.iter() {
        near[c as usize] = ans;
    }
    ans
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
    /// (rule index, input index) -> (row builds, nanoseconds): AST input evaluation.
    pub input_ns: BTreeMap<(usize, usize), (u64, u64)>,
    /// (rule index, clause index, SQL stage name) -> (executions, nanoseconds).
    pub stage_ns: BTreeMap<(usize, usize, String), (u64, u64)>,
}

impl Output {
    fn add_stages(&mut self, ri: usize, ci: usize, stages: Vec<(String, u64)>) {
        for (name, ns) in stages {
            let e = self.stage_ns.entry((ri, ci, name)).or_default();
            e.0 += 1;
            e.1 += ns;
        }
    }
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
        for (k, (n, ns)) in other.input_ns {
            let e = self.input_ns.entry(k).or_default();
            e.0 += n;
            e.1 += ns;
        }
        for (k, (n, ns)) in other.stage_ns {
            let e = self.stage_ns.entry(k).or_default();
            e.0 += n;
            e.1 += ns;
        }
    }
}

pub fn eval_file(rules: &[Rule], ctx: &FileCtx<'_>, out: &mut Output, repo: &mut dyn crate::spill::RepoSink, budget: &mut BTreeMap<(usize, usize), Vec<(String, u32)>>) {
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
            let e = out.input_ns.entry((ri, i)).or_default();
            e.0 += 1;
            e.1 += t0.elapsed().as_nanos() as u64;
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
                    // Spike A3: a dump group's members come from one pass over the facts.
                    let mut from_dump: Vec<Option<Table>> = (0..plan.inputs.len()).map(|_| None).collect();
                    if let Some(g) = &plan.dump {
                        let td = Instant::now();
                        let dumped = dump_rows(rule, g, plan, ctx);
                        out.add_stages(ri, ci, vec![("view:dump".into(), td.elapsed().as_nanos() as u64)]);
                        if let Some((member_tables, member_anchors)) = dumped {
                            if crate::subst::check_mode() {
                                for (m, t) in g.members.iter().zip(&member_tables) {
                                    let orig = get(m.rule_input, out).expect("member input in scope");
                                    if let Err(e) = crate::subst::same_rows(&t.rows, &orig.table.rows) {
                                        panic!("FACTS_A3_CHECK: dump {} at {}: {e}", plan.inputs[m.plan_input].0, ctx.path);
                                    }
                                    assert!(orig.unsupported.is_none());
                                }
                                let mut want: Vec<(u32, u32, u32)> = g.members.iter().flat_map(|m| get(m.rule_input, out).unwrap().anchors.clone()).collect();
                                let mut got = member_anchors.clone();
                                want.sort_unstable();
                                got.sort_unstable();
                                assert!(want == got, "FACTS_A3_CHECK: dump anchors at {}", ctx.path);
                            }
                            for (s, e, l) in member_anchors {
                                anchors.insert((path.clone(), s, e), l);
                            }
                            for (m, t) in g.members.iter().zip(member_tables) {
                                from_dump[m.plan_input] = Some(t);
                            }
                        }
                    }
                    for (k, (_, idx)) in plan.inputs.iter().enumerate() {
                        if let Some(t) = from_dump[k].take() {
                            tables.push(t);
                            continue;
                        }
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
                        out.add_stages(ri, ci, r.stage_ns);
                        if let Some(m) = r.failure {
                            fail(out, m);
                        }
                        for (p, line, code) in r.findings {
                            out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p, line: line + 1, code: if code.is_empty() { clause.viol.code.clone() } else { code } });
                        }
                    } else {
                        let anchors = anchors.into_iter().map(|((_, s, e), l)| (s, e, l)).collect();
                        repo.push((ri, ci), crate::spill::FileRows { path: path.clone(), tables: tables.into_iter().map(|t| t.rows).collect(), anchors });
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

/// Repository partitions and budgets, after every file. `load` returns one repository clause's
/// per-file rows (from the spill files or memory); clauses are loaded and run one at a time.
pub fn finish(rules: &[Rule], mut load: impl FnMut((usize, usize)) -> Vec<crate::spill::FileRows>, budget: BTreeMap<(usize, usize), Vec<(String, u32)>>, out: &mut Output) {
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
    for (ri, rule) in rules.iter().enumerate() {
        for (ci, clause) in rule.clauses.iter().enumerate() {
            let Kind::Relational(plan) = &clause.kind else { continue };
            if plan.per_file {
                continue;
            }
            let t0 = Instant::now();
            // Files in path order, each file's rows in their order.
            let mut files = load((ri, ci));
            files.sort_by(|a, b| a.path.cmp(&b.path));
            let mut inputs: Vec<Table> = plan.input_schemas.iter().map(|s| Table { schema: s.clone(), rows: Vec::new() }).collect();
            let mut anchors: rustc_hash::FxHashMap<(Rc<str>, u32, u32), u32> = Default::default();
            for fr in files {
                for (t, rows) in inputs.iter_mut().zip(fr.tables) {
                    t.rows.extend(rows);
                }
                for (s, e, l) in fr.anchors {
                    anchors.insert((fr.path.clone(), s, e), l);
                }
            }
            out.add_stages(ri, ci, vec![("spill:load".into(), t0.elapsed().as_nanos() as u64)]);
            let r = rel::run(plan, rel::Partition { inputs, anchors, facts: None, source: "", path: "" });
            out.add_stages(ri, ci, r.stage_ns);
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
