//! One policy set compiled for a call configuration, and its evaluation over one file's facts.
use crate::ast::{self, Bound, Compiler, Env, Name, Names, Query};
use crate::counts::{Counts, InputCounts, StageCounts};
use crate::facts::Facts;
use crate::policy::{self, Scope};
use crate::rel::{self, RelPlan};
use crate::sql::engine::{Column, Schema, Table, TextCell, U32_NULL};
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
    /// Those regexes' group names and their capture ids in the rule's name table. The input
    /// carries them, so its rows never depend on which rule a caller evaluated last.
    pub groups: Vec<(String, Name)>,
    /// The input lowered by the pattern front end and planned physically (`lowered.rs`), or why
    /// not (then the leaf executor evaluates it, `Input::rows`).
    pub phys: Result<crate::lowered::Plan, String>,
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
    /// Match-text filters, the content gate and `lines` (`ast/lines.rs`).
    pub options: ast::lines::Options,
    pub paths: Option<Scope>,
}

pub enum Kind {
    ForbidAst { input: usize },
    ForbidRegex(RegexClause),
    /// `budget` (native query/crossfile.rs:436-509): rows kept by `filter` (the evidence `where`),
    /// grouped by the `group` columns of the input's table (none: one group), a group of more
    /// than `lte` rows reported at its first row. With a `closure` (closure.rs) the rows keep
    /// their projected values and `filter` reads them and the closure's outputs after every file.
    Budget { input: usize, lte: u64, group: Vec<usize>, filter: Option<Where>, closure: Option<Box<crate::closure::Closure>> },
    Relational(Box<RelPlan>),
}

/// An evidence `where` over one relation's rows (native query/crossfile.rs:97-267 `Predicate`):
/// `eq`/`ne` compare the operands' JSON text, `lt`/`le`/`gt`/`ge` order two strings by UTF-16
/// code unit (anything else does not order).
pub enum Where {
    Constant(bool),
    All(Vec<Where>),
    Any(Vec<Where>),
    Not(Box<Where>),
    Equal(Operand, Operand, bool),
    Ordered(Operand, Operand, std::cmp::Ordering, bool),
}

/// A field (a column of the input's table, or after the projected fields a closure output) or a
/// literal: a string, number, boolean or null (an array or object literal is refused).
pub enum Operand {
    Field(usize),
    /// A literal; compiled rules are shared across workers, so it holds no `V` (`Rc`) and is
    /// read as one per comparison.
    Literal(Val),
}

use crate::closure::Val;

/// A table value as a `where` reads it.
fn val_of(v: V) -> Val {
    match v {
        V::Null => Val::Null,
        V::Int(i) => Val::Int(i),
        V::Real(r) => Val::Real(r),
        V::Text(t) => Val::Text((*t).into()),
    }
}

impl Where {
    /// `column` maps `alias.field` to its table column; `None` for another alias's or a
    /// non-projected field, which is refused when the alias is this relation's.
    fn compile(w: &Json, alias: &str, column: &dyn Fn(&str) -> Option<usize>) -> Result<Where, String> {
        let operand = |v: &Json| -> Result<Operand, String> {
            if let Some(t) = v.as_str().filter(|t| t.split_once('.').is_some_and(|(a, _)| a == alias)) {
                return column(t).map(Operand::Field).ok_or_else(|| format!("`{t}` names no projected field"));
            }
            Ok(Operand::Literal(match v {
                Json::Null => Val::Null,
                Json::String(s) => Val::Text(s.as_str().into()),
                Json::Number(n) if n.is_i64() => Val::Int(n.as_i64().unwrap()),
                Json::Number(n) if n.is_f64() => Val::Real(n.as_f64().unwrap()),
                Json::Bool(b) => Val::Bool(*b),
                _ => return Err(format!("literal {v} unsupported")),
            }))
        };
        let Some(o) = w.as_object() else { return Ok(Where::Constant(false)) };
        let items = |v: &Json| -> Result<Vec<Where>, String> { v.as_array().into_iter().flatten().map(|i| Where::compile(i, alias, column)).collect() };
        if let Some(v) = o.get("all") {
            return Ok(Where::All(items(v)?));
        }
        if let Some(v) = o.get("any") {
            return Ok(Where::Any(items(v)?));
        }
        if let Some(v) = o.get("not") {
            return Ok(Where::Not(Box::new(Where::compile(v, alias, column)?)));
        }
        let pair = |k: &str| o.get(k).and_then(Json::as_array).filter(|a| a.len() == 2);
        for (k, eq) in [("eq", true), ("ne", false)] {
            if let Some(a) = pair(k) {
                return Ok(Where::Equal(operand(&a[0])?, operand(&a[1])?, eq));
            }
        }
        use std::cmp::Ordering::{Greater, Less};
        for (k, ord, holds) in [("lt", Less, true), ("le", Greater, false), ("gt", Greater, true), ("ge", Less, false)] {
            if let Some(a) = pair(k) {
                return Ok(Where::Ordered(operand(&a[0])?, operand(&a[1])?, ord, holds));
            }
        }
        Ok(Where::Constant(false))
    }

    fn holds(&self, t: &Table, r: usize) -> bool {
        self.holds_by(&|c| val_of(t.value(r, c)))
    }

    /// The predicate over a row whose column `c` reads as `read(c)`: `eq`/`ne` by JSON text
    /// (values of different types differ), order on strings only.
    fn holds_by(&self, read: &dyn Fn(usize) -> Val) -> bool {
        let get = |o: &Operand| match o {
            Operand::Field(c) => read(*c),
            Operand::Literal(l) => l.clone(),
        };
        match self {
            Where::Constant(b) => *b,
            Where::All(v) => v.iter().all(|w| w.holds_by(read)),
            Where::Any(v) => v.iter().any(|w| w.holds_by(read)),
            Where::Not(w) => !w.holds_by(read),
            Where::Equal(a, b, eq) => {
                let same = match (get(a), get(b)) {
                    (Val::Real(x), Val::Real(y)) => x.to_bits() == y.to_bits(),
                    (x, y) => x == y,
                };
                same == *eq
            }
            // `lt`: order is Less; `le`: order is not Greater; `gt`: Greater; `ge`: not Less.
            Where::Ordered(a, b, ord, holds) => match (get(a), get(b)) {
                (Val::Text(x), Val::Text(y)) => (x.encode_utf16().cmp(y.encode_utf16()) == *ord) == *holds,
                _ => false,
            },
        }
    }
}

pub struct Clause {
    pub id: String,
    pub kind: Kind,
    pub viol: Viol,
    /// Its plan's class (`physplan::clause_cost`, docs/facts/PHASE-3.md § P3b), and the class of
    /// a content where a per-content check fails (`fallback`): what its algebraic budget scales
    /// with (`crate::budget`).
    pub class: rerail_facts::phys::Class,
    pub fallback: rerail_facts::phys::Class,
    /// The class overlay's declaration (`packs/data-layer/candidates/classes.json`, set by
    /// `repo::compile_checks`): an unbounded clause without one is warned about.
    pub declared: Option<crate::physplan::Declared>,
}

pub struct Rule {
    pub id: String,
    pub check: String,
    pub scope: Scope,
    pub clauses: Vec<Clause>,
    pub inputs: Vec<Input>,
    pub names: Names,
    /// Per input, the last clause that reads it: its rows are dropped after that clause.
    pub last_use: Vec<usize>,
    /// The physical predicates of the rule's lowered inputs (memoised per file, shared by them).
    pub preds: crate::lowered::Preds,
    /// `define.report_when.count.gt`: the rule's findings are reported only when there are more
    /// than this many over the evaluation (native eval/mod.rs `evaluate_code_rule`).
    pub report_count_gt: Option<u64>,
    /// Per clause, its plan digest (`crate::memo::digest`): the memo's key for its results.
    pub digests: Vec<[u8; 32]>,
}

/// Compiled rules are one immutable object per process, shared by every worker (G2.5).
const _: () = {
    const fn shared<T: Send + Sync>() {}
    shared::<Rule>();
};

fn proj_parse(s: &Json, names: &mut Names) -> Result<Proj, String> {
    if let Some(o) = s.as_object() {
        let src = proj_parse(o.get("source").ok_or("projection source")?, names)?;
        let rx = crate::rx::get(o["regex"].as_str().ok_or("projection regex")?).map_err(|e| e.to_string())?;
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
        let options = policy::option_values(document, config)?;
        let id = doc["id"].as_str().unwrap_or("").to_owned();
        let check = id.trim_start_matches("data-layer-").replace('-', "_");
        let scope = Scope::new(scope_override.or(doc.get("scope")))?;
        let mut utils: BTreeMap<String, Json> = BTreeMap::new();
        let mut report_count_gt = None;
        for c in doc["clauses"].as_array().ok_or("clauses")? {
            if let Some(u) = c.get("define").and_then(|d| d.get("utils")).and_then(Json::as_object) {
                for (k, v) in u {
                    utils.insert(k.clone(), v.clone());
                }
            }
            if let Some(rw) = c.get("define").and_then(|d| d.get("report_when")) {
                if report_count_gt.is_some() {
                    return Err("report_when must be declared once per document".into());
                }
                let count = rw.as_object().filter(|o| o.len() == 1).and_then(|o| o.get("count")).and_then(Json::as_object).filter(|c| c.len() == 1);
                report_count_gt = Some(count.and_then(|c| c.get("gt")).and_then(Json::as_u64).ok_or("/report_when must be {count: {gt: N}}")?);
            }
        }
        let mut names = Names::default();
        let mut inputs: Vec<Input> = Vec::new();
        let mut keys: BTreeMap<String, usize> = BTreeMap::new();
        let mut clauses = Vec::new();
        // One compiled utility table for every input query without local utilities.
        let mut util_table: Option<ast::UtilTable> = None;
        let mut preds = crate::lowered::Preds::default();
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
            comp.shared = util_table.clone();
            comp.options = Some(options.clone());
            let query = comp.query(ast_json)?;
            if util_table.is_none() {
                util_table = comp.shared.take();
            }
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
            let mut groups: Vec<(String, Name)> = Vec::new();
            for rx in &nr {
                for g in rx.capture_names().flatten() {
                    if groups.iter().all(|(n, _)| n != g) {
                        groups.push((g.to_owned(), names.id(g)));
                    }
                }
            }
            let phys = crate::lowered::lower_and_plan(ev, ast_json, &fields, &utils, grammar, &mut preds);
            inputs.push(Input { query, fields: fields.clone(), projs, each, anchor, paths, node_regexes: nr, groups, phys });
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
                    let options = ast::lines::options(r).map_err(|e| format!("{cid}: {e}"))?;
                    Kind::ForbidRegex(RegexClause {
                        source: match r["source"].as_str() {
                            Some("file.content") => RegexSource::Content,
                            Some("file.path") => RegexSource::Path,
                            s => return Err(format!("{cid}: regex source {s:?}")),
                        },
                        rx: crate::rx::get(r["pattern"].as_str().ok_or("pattern")?).map_err(|e| e.to_string())?,
                        options,
                        paths: r.get("paths").map(|p| Scope::new(Some(p))).transpose()?,
                    })
                } else {
                    return Err(format!("{cid}: forbid form"));
                }
            } else if let Some(b) = c.get("budget") {
                let ev = b["query"]["evidence"].clone();
                if b["count"].get("distinct").is_some() {
                    return Err(format!("{cid}: budget count.distinct unsupported"));
                }
                let alias = ev.get("as").and_then(Json::as_str).unwrap_or("row").to_owned();
                let gb: Vec<String> = b.get("group_by").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).map(str::to_owned).collect()).unwrap_or_default();
                let (i, fields) = add_input(&ev, &mut names, &mut inputs).map_err(|e| format!("{cid}: {e}"))?;
                let closure = ev.get("closure").map(|c| crate::closure::Closure::compile(c, &fields, &format!("{cid}/evidence"))).transpose()?.map(Box::new);
                // Table columns: __path, __start, __end, then the projected fields in name order;
                // a `where` reads the closure's outputs after them.
                let outputs: &[String] = closure.as_ref().map_or(&[], |c| &c.outputs);
                let column = |operand: &str| -> Option<usize> {
                    let field = operand.strip_prefix(alias.as_str())?.strip_prefix('.')?;
                    fields.iter().position(|f| f == field).map(|p| p + 3)
                };
                let group = gb
                    .iter()
                    .map(|g| column(g).ok_or_else(|| format!("{cid}: group_by {g} is not a projected field of {alias}")))
                    .collect::<Result<Vec<_>, String>>()?;
                let read = |operand: &str| -> Option<usize> {
                    column(operand).or_else(|| {
                        let field = operand.strip_prefix(alias.as_str())?.strip_prefix('.')?;
                        outputs.iter().position(|o| o == field).map(|p| p + 3 + fields.len())
                    })
                };
                let filter = ev.get("where").map(|w| Where::compile(w, &alias, &read)).transpose().map_err(|e| format!("{cid}/where: {e}"))?;
                Kind::Budget { input: i, lte: b["count"]["lte"].as_u64().ok_or("lte")?, group, filter, closure }
            } else if let Some(r) = c.get("relational") {
                let mut ins = Vec::new();
                let mut named: Vec<(&String, &Json)> = r["inputs"].as_object().ok_or("inputs")?.iter().collect();
                named.sort_by(|a, b| a.0.cmp(b.0));
                for (name, inp) in named {
                    let (i, fields) = add_input(&inp["evidence"], &mut names, &mut inputs).map_err(|e| format!("{cid}/{name}: {e}"))?;
                    ins.push((name.clone(), i, fields));
                }
                let mut plan = RelPlan::compile(r, ins, grammar).map_err(|e| format!("{cid}: {e}"))?;
                plan.schedule_releases();
                Kind::Relational(Box::new(plan))
            } else {
                return Err(format!("{cid}: clause kind"));
            };
            clauses.push(Clause { id: cid, kind, viol, class: rerail_facts::phys::Class::Unbounded, fallback: rerail_facts::phys::Class::Unbounded, declared: None });
        }
        let last_use = last_use(&clauses, inputs.len());
        // Plan digests (`crate::memo`): the rule without its clauses, its `define` clauses, the
        // call's scope, then per clause its resolved text, its inputs' physical plans, and its
        // stages' physical EXPLAIN.
        let mut rule_level = doc.clone();
        if let Some(o) = rule_level.as_object_mut() {
            o.remove("clauses");
        }
        let all = doc["clauses"].as_array().unwrap();
        let defines: Vec<&Json> = all.iter().filter(|c| c.get("define").is_some()).collect();
        let input_text = |i: usize| match &inputs[i].phys {
            Ok(p) => p.digest_text(&preds),
            Err(e) => format!("leaf executor: {e}"),
        };
        let digests = all
            .iter()
            .filter(|c| c.get("define").is_none())
            .zip(&clauses)
            .map(|(cj, c)| {
                let (reads, stages): (Vec<usize>, Vec<String>) = match &c.kind {
                    Kind::ForbidAst { input } | Kind::Budget { input, .. } => (vec![*input], Vec::new()),
                    Kind::ForbidRegex(_) => (Vec::new(), Vec::new()),
                    Kind::Relational(plan) => (plan.inputs.iter().map(|(_, i)| *i).collect(), crate::physplan::clause(plan).iter().map(|(n, p)| format!("{n}\n{}", rerail_facts::phys::explain(p))).collect()),
                };
                crate::memo::digest(&rule_level, &defines, scope_override, cj, &reads.into_iter().map(input_text).collect::<Vec<_>>(), &stages)
            })
            .collect();
        let mut rule = Rule { id, check, scope, clauses, inputs, names, last_use, preds, report_count_gt, digests };
        // Each clause's class (and fallback), and the K1 keys its relational plan checks per
        // content (`rel::run`), from its physical plan: once per rule. A class proven without
        // them (equal to its fallback) needs no key checked.
        for ci in 0..rule.clauses.len() {
            let cost = crate::physplan::clause_cost(&rule, &rule.clauses[ci]);
            let c = &mut rule.clauses[ci];
            (c.class, c.fallback) = (cost.class, cost.fallback);
            if let Kind::Relational(plan) = &mut c.kind {
                plan.checked_keys = if cost.proven() { Vec::new() } else { cost.checked_keys };
            }
        }
        Ok(rule)
    }

    /// Keeps only the clauses `keep` selects (by id), with their digests; inputs no kept clause
    /// reads are never evaluated (`last_use` recomputed). The bench runs one clause this way.
    pub fn retain_clauses(&mut self, keep: impl Fn(&str) -> bool) {
        let sel: Vec<bool> = self.clauses.iter().map(|c| keep(&c.id)).collect();
        let mut s = sel.iter();
        self.digests.retain(|_| *s.next().unwrap());
        let mut s = sel.iter();
        self.clauses.retain(|_| *s.next().unwrap());
        self.last_use = last_use(&self.clauses, self.inputs.len());
    }

    /// Per clause, whether its per-file result is a function of the file's content alone, the
    /// path aside where it only labels the result (a finding's, a failure's or a budget row's
    /// path, the first column of a repository clause's input rows, which every input fills with
    /// the path): the memo stores such a clause's result once per content (`crate::memo`). A
    /// clause qualifies when no input it reads has a `paths` filter, a path projection or a
    /// `filePath` term (matcher or physical plan), a regex clause reads the content without a
    /// `paths` filter, a budget neither groups nor filters on the path column, and a relational
    /// clause is a repository clause (a per-file one's stages may read `__path`). The rule's
    /// scope selects clauses by path before the memo is read, so it is not part of the result.
    pub fn content_pure(&self) -> Vec<bool> {
        fn proj_path(p: &Proj) -> bool {
            match p {
                Proj::Path | Proj::Directory | Proj::Name | Proj::Stem => true,
                Proj::Regex(src, ..) => proj_path(src),
                Proj::Capture(_) | Proj::Text(_) | Proj::Kind(_) | Proj::Start(_) | Proj::End(_) | Proj::Line(_) => false,
            }
        }
        let input_pure = |i: usize| {
            let inp = &self.inputs[i];
            inp.paths.is_none()
                && !inp.projs.iter().any(proj_path)
                && !ast::reads_path(&inp.query)
                && inp.phys.as_ref().map_or(true, |p| !p.reads_path(&self.preds))
        };
        fn where_reads(w: &Where, col: usize) -> bool {
            match w {
                Where::Constant(_) => false,
                Where::All(v) | Where::Any(v) => v.iter().any(|w| where_reads(w, col)),
                Where::Not(w) => where_reads(w, col),
                Where::Equal(a, b, _) | Where::Ordered(a, b, ..) => [a, b].iter().any(|o| matches!(o, Operand::Field(c) if *c == col)),
            }
        }
        self.clauses
            .iter()
            .map(|c| match &c.kind {
                Kind::ForbidAst { input } => input_pure(*input),
                Kind::ForbidRegex(rc) => matches!(rc.source, RegexSource::Content) && rc.paths.is_none(),
                Kind::Budget { input, group, filter, .. } => input_pure(*input) && !group.contains(&0) && !filter.as_ref().is_some_and(|w| where_reads(w, 0)),
                Kind::Relational(plan) => !plan.per_file && plan.inputs.iter().all(|(_, i)| input_pure(*i)),
            })
            .collect()
    }
}

/// Per input, the last clause that reads it.
fn last_use(clauses: &[Clause], inputs: usize) -> Vec<usize> {
    let mut last = vec![0; inputs];
    for (ci, c) in clauses.iter().enumerate() {
        match &c.kind {
            Kind::ForbidAst { input } | Kind::Budget { input, .. } => last[*input] = ci,
            Kind::Relational(plan) => plan.inputs.iter().for_each(|(_, i)| last[*i] = ci),
            Kind::ForbidRegex(_) => {}
        }
    }
    last
}

/// An input's rows for one file.
pub struct Rows {
    pub table: Rc<Table>,
    /// Anchor spans with their lines (0-based) and witness spans.
    pub anchors: Vec<(u32, u32, u32)>,
    pub witnesses: Vec<(u32, u32)>,
    pub unsupported: Option<String>,
    /// The evaluation's counts (`crate::counts`).
    pub counts: crate::counts::InputCounts,
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
    pub fn rows(&self, ctx: &FileCtx<'_>, schema: &Schema, memo: &ast::FileMemo, source: &Rc<str>) -> Rows {
        let mut out = Rows { table: Rc::new(Table::empty(schema.clone())), anchors: Vec::new(), witnesses: Vec::new(), unsupported: None, counts: Default::default() };
        out.counts.nodes = ctx.f.len() as u64;
        // Columns: path, start, end, then the projections; a capture's text is its node's byte
        // range of the source (`Column::Text`), read when a stage reads it; offsets and lines are
        // 4 bytes a row, kinds symbols, file-level values stored once.
        enum Col {
            Vals(Vec<V>),
            Text(Vec<TextCell>),
            U32(Vec<u32>),
            Sym(Vec<u32>),
            File,
        }
        let is_text = |p: &Proj| match p {
            Proj::Capture(_) => true,
            Proj::Text(n) => !self.query.decoded.iter().any(|(t, _)| t == n),
            _ => false,
        };
        let col_of = |p: &Proj| match p {
            _ if is_text(p) => Col::Text(Vec::new()),
            Proj::Path | Proj::Directory | Proj::Name | Proj::Stem => Col::File,
            Proj::Start(_) | Proj::End(_) | Proj::Line(_) => Col::U32(Vec::new()),
            Proj::Kind(_) => Col::Sym(Vec::new()),
            _ => Col::Vals(Vec::new()),
        };
        let mut cols: Vec<Col> = [Col::File, Col::U32(Vec::new()), Col::U32(Vec::new())].into_iter().chain(self.projs.iter().map(col_of)).collect();
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
        let ev = ast::Eval::shared(f, ctx.path, &self.query, memo);
        let path: Rc<str> = Rc::from(ctx.path);
        let each_is_group = self.each.is_some_and(|e| !self.query_binds(e));
        for root in ev.roots() {
            let Some(env) = ev.match_root(root) else { continue };
            out.counts.roots += 1;
            let mut emit = |env: Env, groups: &[(Name, (u32, u32), String)]| {
                let witness = (f.start[root as usize], f.end[root as usize]);
                let span = match &self.anchor {
                    Anchor::Span => witness,
                    Anchor::FileStart => (0, 0),
                    Anchor::Capture(n) => capture_range(&ev, &env, groups, *n).unwrap_or(witness),
                };
                for (c, v) in cols[1..3].iter_mut().zip([span.0, span.1]) {
                    if let Col::U32(c) = c {
                        c.push(v);
                    }
                }
                for (c, p) in cols[3..].iter_mut().zip(&self.projs) {
                    match c {
                        Col::Text(c) => c.push(match p {
                            Proj::Capture(n) | Proj::Text(n) => match env.get(*n) {
                                Some(Bound::Node(x)) => TextCell::Span(f.start[x as usize], f.end[x as usize]),
                                _ => TextCell::Val(project(p, &ev, &env, groups, ctx, &self.query)),
                            },
                            _ => TextCell::Val(project(p, &ev, &env, groups, ctx, &self.query)),
                        }),
                        Col::U32(c) => c.push(match project(p, &ev, &env, groups, ctx, &self.query) {
                            V::Int(x) => u32::try_from(x).expect("offsets and lines fit in u32"),
                            _ => U32_NULL,
                        }),
                        Col::Sym(c) => {
                            let Proj::Kind(n) = p else { unreachable!("only kind projections are symbols") };
                            c.push(match env.get(*n) {
                                Some(Bound::Node(x)) => f.kind[x as usize],
                                _ => crate::facts::NONE,
                            });
                        }
                        Col::File => {}
                        Col::Vals(c) => c.push(project(p, &ev, &env, groups, ctx, &self.query)),
                    }
                }
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
                    let gname = self.group_name(name);
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
        let n = out.witnesses.len();
        let file_value = |i: usize| if i == 0 { V::Text(path.clone()) } else { project(&self.projs[i - 3], &ev, &Env::default(), &[], ctx, &self.query) };
        let cols = cols
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                Rc::new(match c {
                    Col::Vals(v) => Column::Vals(v),
                    Col::Text(t) => Column::text(source.clone(), t),
                    Col::U32(v) => Column::U32(v),
                    Col::Sym(v) => Column::Sym(v),
                    Col::File => Column::Const(file_value(i)),
                })
            })
            .collect();
        out.table = Rc::new(Table::new(schema.clone(), cols, n));
        out.counts.rows = n as u64;
        // The leaf executor's physical work: its rule tests (`ast::Eval::steps`).
        out.counts.pred_tests = ev.steps.get();
        out
    }

    fn query_binds(&self, name: Name) -> bool {
        // A metavariable `each` (the rule binds it) vs a regex group `each`.
        ast::binds_name(&self.query, name)
    }

    fn query_name(&self, n: &str) -> Name {
        self.query_name_opt(n).unwrap_or(Name::MAX)
    }
    fn query_name_opt(&self, n: &str) -> Option<Name> {
        self.groups.iter().find(|(g, _)| g == n).map(|(_, id)| *id)
    }

    fn group_name(&self, id: Name) -> Option<&str> {
        self.groups.iter().find(|(_, i)| *i == id).map(|(g, _)| g.as_str())
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

fn capture_range(ev: &ast::Eval<'_>, env: &Env, groups: &[(Name, (u32, u32), String)], n: Name) -> Option<(u32, u32)> {
    if let Some(b) = env.get(n) {
        return Some(ev.range_of(b));
    }
    groups.iter().find(|(g, _, _)| *g == n).map(|(_, r, _)| *r)
}

pub(crate) fn project(p: &Proj, ev: &ast::Eval<'_>, env: &Env, groups: &[(Name, (u32, u32), String)], ctx: &FileCtx<'_>, q: &Query) -> V {
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
    /// Regex findings: the 1-based code-point column of the anchor's start in its line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
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
    /// (rule index, clause index) -> counts (`crate::counts`), summed over files and the
    /// repository phase.
    pub clause_counts: BTreeMap<(usize, usize), Counts>,
    /// (rule index, clause index, stage name) -> its counts summed over runs (`<final>`: the
    /// final query).
    pub stage_counts: BTreeMap<(usize, usize, String), StageCounts>,
    /// (rule index, input index) -> its evaluations' counts, physical ones included.
    pub input_counts: BTreeMap<(usize, usize), InputCounts>,
    /// When set, each clause's counts per file evaluation are also pushed here, in order (the
    /// caller drains it per file: `repo`'s memo stores them).
    pub file_counts: Option<Vec<((usize, usize), Counts)>>,
    /// (rule index, clause index) of clauses `eval_file` does not evaluate: their results for the
    /// current file come from elsewhere (`repo`'s memo).
    pub skip: rustc_hash::FxHashSet<(usize, usize)>,
}

/// Adds `b` into `a`: rows and physical work, fixpoint rounds round by round.
pub fn add_stage_counts(a: &mut StageCounts, b: &StageCounts) {
    a.rows += b.rows;
    if a.fix.len() < b.fix.len() {
        a.fix.resize(b.fix.len(), 0);
    }
    for (x, y) in a.fix.iter_mut().zip(&b.fix) {
        *x += y;
    }
    for (x, y) in a.phys.iter_mut().zip(&b.phys) {
        *x += y;
    }
}

impl Output {
    fn add_stages(&mut self, ri: usize, ci: usize, stages: Vec<(String, u64)>) {
        for (name, ns) in stages {
            let e = self.stage_ns.entry((ri, ci, name)).or_default();
            e.0 += 1;
            e.1 += ns;
        }
    }
    /// A clause's counts for one evaluation (`file`: a per-file one, kept for `file_counts`).
    fn add_counts(&mut self, ri: usize, ci: usize, c: Counts, file: bool) {
        *self.clause_counts.entry((ri, ci)).or_default() += &c;
        if file {
            if let Some(v) = self.file_counts.as_mut() {
                v.push(((ri, ci), c));
            }
        }
    }
    /// The last `rel::run`'s counts into `c` and the stage totals (K1 keys that did not hold
    /// count in `bounds_unproven`).
    fn add_run(&mut self, ri: usize, ci: usize, c: &mut Counts) {
        let r = rel::last_counts();
        c.leaf_rows += r.leaf_rows;
        c.bounds_unproven += r.unproven;
        for (name, s) in r.stages {
            c.add_stage(&s);
            add_stage_counts(self.stage_counts.entry((ri, ci, name)).or_default(), &s);
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
        for (k, c) in other.clause_counts {
            *self.clause_counts.entry(k).or_default() += &c;
        }
        for (k, s) in other.stage_counts {
            add_stage_counts(self.stage_counts.entry(k).or_default(), &s);
        }
        for (k, c) in other.input_counts {
            *self.input_counts.entry(k).or_default() += &c;
        }
    }
}

/// One budget row: its path, group key and witness line (0-based); with a closure, its projected
/// values (field order), which the closure and the `where` read after every file.
pub struct BudgetRow {
    pub path: String,
    pub key: String,
    pub line: u32,
    pub values: Box<[Val]>,
}

/// Budget rows per (rule, clause), in file order.
pub type BudgetRows = BTreeMap<(usize, usize), Vec<BudgetRow>>;

pub fn eval_file(rules: &[Rule], ctx: &FileCtx<'_>, out: &mut Output, repo: &mut dyn crate::spill::RepoSink, budget: &mut BudgetRows) {
    // The content's size for the algebraic budgets (G3b.3): `n` nodes, depth `d`.
    let (n, d) = (ctx.f.len() as u64, u64::from(ctx.f.depth()));
    // The file's text, shared by every input column that reads node text.
    let source: Rc<str> = Rc::from(ctx.source);
    for (ri, rule) in rules.iter().enumerate() {
        if !rule.scope.matches(ctx.path) {
            continue;
        }
        // Memoised verdicts shared by the rule's queries over this file.
        let memo = ast::FileMemo::default();
        // The physical predicates' truths, shared by the rule's lowered inputs over this file.
        let phys_state = std::cell::RefCell::new(crate::lowered::FileState::new(ctx.f, ctx.path));
        let cache: std::cell::RefCell<BTreeMap<usize, Rc<Rows>>> = Default::default();
        let get = |i: usize, out: &mut Output| -> Option<Rc<Rows>> {
            let inp = &rule.inputs[i];
            if inp.paths.as_ref().is_some_and(|p| !p.matches(ctx.path)) {
                return None;
            }
            if let Some(r) = cache.borrow().get(&i) {
                return Some(r.clone());
            }
            let t0 = Instant::now();
            let schema = rel::input_schema(&inp.fields);
            let phys = inp.phys.as_ref().ok().and_then(|plan| crate::lowered::rows(plan, &rule.preds, &mut phys_state.borrow_mut(), &schema, &source));
            let r = Rc::new(match phys {
                Some(rows) => rows,
                None => inp.rows(ctx, &schema, &memo, &source),
            });
            let e = out.input_ns.entry((ri, i)).or_default();
            e.0 += 1;
            e.1 += t0.elapsed().as_nanos() as u64;
            *out.input_counts.entry((ri, i)).or_default() += &r.counts;
            cache.borrow_mut().insert(i, r.clone());
            Some(r)
        };
        for (ci, clause) in rule.clauses.iter().enumerate() {
            if out.skip.contains(&(ri, ci)) {
                continue;
            }
            let t0 = Instant::now();
            let mut c = Counts::default();
            if std::env::var_os("FACTS_MEMTRACE").is_some() {
                crate::repo::thread_peak_reset();
            }
            let fail = |out: &mut Output, msg: String| {
                out.failures.push(Failure { check: rule.check.clone(), clause: clause.id.clone(), path: ctx.path.to_owned(), message: msg });
            };
            let limits = rerail_facts::budget::Limits::new(clause.class, clause.fallback, n, d);
            let (f0, b0) = (out.findings.len(), budget.get(&(ri, ci)).map_or(0, Vec::len));
            match &clause.kind {
                Kind::ForbidAst { input } => {
                    let inp = &rule.inputs[*input];
                    // Witnesses with `violation.each` and anchors.
                    let f = ctx.f;
                    match inp.query.syntax {
                        ast::SyntaxMode::Valid if f.syntax_errors => {}
                        ast::SyntaxMode::Invalid if !f.syntax_errors => {}
                        _ => {
                            let ev = ast::Eval::shared(f, ctx.path, &inp.query, &memo);
                            c.match_nodes = f.len() as u64;
                            for root in ev.roots() {
                                let Some(env) = ev.match_root(root) else { continue };
                                c.match_roots += 1;
                                let witness = (f.start[root as usize], f.end[root as usize]);
                                let mut envs = Vec::new();
                                match clause.viol.each {
                                    Some(name) => ev.enumerate(root, env, name, |e| envs.push(e)),
                                    None => envs.push(env),
                                }
                                let groups = inp.first_groups(f, root);
                                c.input_rows += envs.len() as u64;
                                for e in envs {
                                    let span = match &clause.viol.anchor {
                                        Anchor::Span => witness,
                                        Anchor::FileStart => (0, 0),
                                        Anchor::Capture(n) => capture_range(&ev, &e, &groups, *n).unwrap_or(witness),
                                    };
                                    let line = if matches!(clause.viol.anchor, Anchor::FileStart) { 0 } else { line_of(f, span.0) };
                                    out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: ctx.path.to_owned(), line: line + 1, code: clause.viol.code.clone(), column: None });
                                }
                            }
                            if let Some(u) = ev.unsupported.borrow_mut().take() {
                                fail(out, format!("unsupported: {u}"));
                            }
                            // The leaf executor's physical work (`steps`), as an input's.
                            *out.input_counts.entry((ri, *input)).or_default() += &InputCounts { nodes: c.match_nodes, roots: c.match_roots, rows: c.input_rows, pred_tests: ev.steps.get(), memo_hits: 0 };
                        }
                    }
                }
                Kind::ForbidRegex(rc) => {
                    if rc.paths.as_ref().is_none_or(|p| p.matches(ctx.path)) {
                        regex_findings(rc, ctx.path, ctx.source, &ctx.f.lines, &mut |line, column| {
                            out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: ctx.path.to_owned(), line: line + 1, code: clause.viol.code.clone(), column: Some(column) });
                        }, &clause.viol.anchor, &rule.names);
                    }
                }
                Kind::Budget { input, group, filter, closure, .. } => {
                    if let Some(rows) = get(*input, out) {
                        c.add_input(&rows.counts);
                        if let Some(u) = &rows.unsupported {
                            fail(out, format!("unsupported: {u}"));
                        }
                        let acc = budget.entry((ri, ci)).or_default();
                        // The group key: the `group_by` values, each typed (as native's JSON text
                        // of each value tells types apart).
                        let width = rule.inputs[*input].fields.len();
                        for k in 0..rows.table.len() {
                            // With a closure the `where` reads its outputs: applied in `finish`.
                            if closure.is_none() && filter.as_ref().is_some_and(|w| !w.holds(&rows.table, k)) {
                                continue;
                            }
                            let mut key = String::new();
                            for &c in group {
                                match rows.table.value(k, c) {
                                    V::Null => key.push_str("n\u{1f}"),
                                    V::Int(i) => key.push_str(&format!("i{i}\u{1f}")),
                                    V::Real(r) => key.push_str(&format!("r{r:?}\u{1f}")),
                                    V::Text(t) => key.push_str(&format!("t{}:{t}\u{1f}", t.len())),
                                }
                            }
                            let values = if closure.is_some() { (0..width).map(|c| val_of(rows.table.value(k, c + 3))).collect() } else { Box::default() };
                            // Reported at the evidence's anchor (native eval/evidence.rs `reported`).
                            acc.push(BudgetRow { path: ctx.path.to_owned(), key, line: rows.anchors[k].2, values });
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
                                c.add_input(&rows.counts);
                                if rows.unsupported.is_some() {
                                    unsupported = rows.unsupported.clone();
                                }
                                for (s, e, l) in &rows.anchors {
                                    anchors.insert((path.clone(), *s, *e), *l);
                                }
                                tables.push(rows.table.clone());
                            }
                            None => tables.push(Rc::new(Table::empty(plan.input_schemas[k].clone()))),
                        }
                    }
                    if std::env::var_os("FACTS_MEMTRACE").is_some() {
                        let bytes: usize = tables.iter().map(|t| t.bytes()).sum();
                        eprintln!("memtrace inputs of {}: {} tables ~{} MB, heap {} MB, peak since clause start {} MB", clause.id, tables.len(), bytes >> 20, crate::repo::heap_in_use() >> 20, crate::repo::thread_peak() >> 20);
                        crate::repo::thread_peak_reset();
                    }
                    if let Some(u) = unsupported {
                        fail(out, format!("unsupported: {u}"));
                    } else if plan.per_file {
                        let r = rel::run(plan, rel::Partition { inputs: tables, anchors, facts: Some((ctx.f, ctx.path)), source: ctx.source, path: ctx.path, budget: Some((limits, c.match_nodes + c.input_rows)) });
                        out.add_stages(ri, ci, r.stage_ns);
                        out.add_run(ri, ci, &mut c);
                        if let Some(m) = r.failure {
                            fail(out, m);
                        }
                        for (p, line, code) in r.findings {
                            out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p, line: line + 1, code: if code.is_empty() { clause.viol.code.clone() } else { code }, column: None });
                        }
                    } else {
                        let anchors = anchors.into_iter().map(|((_, s, e), l)| (s, e, l)).collect();
                        let rows = |t: &Table| -> Vec<Box<[V]>> { (0..t.len()).map(|r| t.row(r).into_boxed_slice()).collect() };
                        repo.push((ri, ci), crate::spill::FileRows { path: path.clone(), tables: tables.iter().map(|t| rows(t)).collect(), anchors });
                    }
                }
            }
            // G3b.3: a clause without stages answers for its matcher work (a relational clause's
            // run checked its own); over the limit it fails alone, its findings and rows dropped.
            if !matches!(clause.kind, Kind::Relational(_)) {
                if let Err(e) = rerail_facts::budget::verdict(c.match_nodes + c.input_rows, limits, false) {
                    out.findings.truncate(f0);
                    if let Some(v) = budget.get_mut(&(ri, ci)) {
                        v.truncate(b0);
                    }
                    fail(out, e);
                }
            }
            out.add_counts(ri, ci, c, true);
            let ns = t0.elapsed().as_nanos() as u64;
            let e = out.clause_ns.entry((ri, ci)).or_default();
            e.0 += 1;
            e.1 += ns;
            out.clause_samples.push(((ri as u16, ci as u16), ns));
            cache.borrow_mut().retain(|i, _| rule.last_use[*i] > ci);
            if std::env::var_os("FACTS_MEMTRACE").is_some() {
                eprintln!("memtrace clause {}: rss {} MB, heap {} MB, peak since last trace {} MB", clause.id, crate::repo::cur_rss() >> 20, crate::repo::heap_in_use() >> 20, crate::repo::thread_peak() >> 20);
            }
        }
        if std::env::var_os("FACTS_MEMTRACE").is_some() {
            let h0 = crate::repo::heap_in_use();
            drop(cache);
            let h1 = crate::repo::heap_in_use();
            drop(memo);
            let h2 = crate::repo::heap_in_use();
            eprintln!("memtrace rule {} done: input cache {} MB, memo {} MB, rss {} MB, heap {} MB", rule.check, (h0 - h1) >> 20, (h1 - h2) >> 20, crate::repo::cur_rss() >> 20, h2 >> 20);
        }
    }
}

/// A file the facts do not cover (not Python: a `.rs` file). Natively an AST clause over a Python
/// grammar reads no such file; a regex clause reads its text with the language's line table, whose
/// terminators are `Newlines` for every adapter but ECMAScript's (native language/adapter.rs).
pub fn eval_text_file(rules: &[Rule], path: &str, source: &str, out: &mut Output) {
    use rerail_engine::code_rule_dsl::lines::{LineTable, Terminators};
    let lines = LineTable::new(source, Terminators::Newlines);
    for rule in rules {
        if !rule.scope.matches(path) {
            continue;
        }
        for clause in &rule.clauses {
            let Kind::ForbidRegex(rc) = &clause.kind else { continue };
            if rc.paths.as_ref().is_none_or(|p| p.matches(path)) {
                regex_findings(rc, path, source, &lines, &mut |line, column| {
                    out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: path.to_owned(), line: line + 1, code: clause.viol.code.clone(), column: Some(column) });
                }, &clause.viol.anchor, &rule.names);
            }
        }
    }
}

/// Repository partitions and budgets, after every file. `load` returns one repository clause's
/// per-file rows (from the spill files or memory); clauses are loaded and run one at a time.
pub fn finish(rules: &[Rule], mut load: impl FnMut((usize, usize)) -> Vec<crate::spill::FileRows>, budget: BudgetRows, out: &mut Output) {
    for ((ri, ci), mut rows) in budget {
        let rule = &rules[ri];
        let clause = &rule.clauses[ci];
        let Kind::Budget { lte, filter, closure, .. } = &clause.kind else { continue };
        // Files in path order, each file's rows in preorder (a stable sort); groups in first-row
        // order, each reported at its first row.
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        if let Some(closure) = closure {
            // Native fails the derived relation when some file's rows are unknown (that file's
            // failure is reported above), or when it spends more than `maxVisitedNodes` steps.
            let incomplete = out.failures.iter().any(|f| f.check == rule.check && f.clause == clause.id);
            let derived = if incomplete { Err(crate::closure::INCOMPLETE) } else { closure.derive(&rows.iter().map(|r| &*r.values).collect::<Vec<_>>()) };
            let outputs = match derived {
                Ok(o) => o,
                Err((code, message)) => {
                    out.failures.push(Failure { check: rule.check.clone(), clause: clause.id.clone(), path: String::new(), message: format!("{code}: {message}") });
                    continue;
                }
            };
            let width = rows.first().map_or(0, |r| r.values.len());
            let kept: Vec<bool> = outputs.iter().zip(&rows).map(|(o, r)| filter.as_ref().is_none_or(|w| w.holds_by(&|c| if c - 3 < width { r.values[c - 3].clone() } else { Val::Bool(o[c - 3 - width]) }))).collect();
            let mut kept = kept.into_iter();
            rows.retain(|_| kept.next().unwrap_or(false));
        }
        let mut groups: Vec<(usize, u64)> = Vec::new();
        let mut index: std::collections::HashMap<&str, usize> = Default::default();
        for (j, r) in rows.iter().enumerate() {
            match index.get(r.key.as_str()) {
                Some(&g) => groups[g].1 += 1,
                None => {
                    index.insert(&r.key, groups.len());
                    groups.push((j, 1));
                }
            }
        }
        for (first, n) in groups {
            if n > *lte {
                let BudgetRow { path: p, line, .. } = &rows[first];
                out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p.clone(), line: line + 1, code: clause.viol.code.clone(), column: None });
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
            let mut input_rows: Vec<Vec<Box<[V]>>> = plan.input_schemas.iter().map(|_| Vec::new()).collect();
            let mut anchors: rustc_hash::FxHashMap<(Rc<str>, u32, u32), u32> = Default::default();
            for fr in files {
                for (t, rows) in input_rows.iter_mut().zip(fr.tables) {
                    t.extend(rows);
                }
                for (s, e, l) in fr.anchors {
                    anchors.insert((fr.path.clone(), s, e), l);
                }
            }
            out.add_stages(ri, ci, vec![("spill:load".into(), t0.elapsed().as_nanos() as u64)]);
            let inputs = plan.input_schemas.iter().zip(input_rows).map(|(s, rows)| Rc::new(Table::from_rows(s.clone(), rows))).collect();
            let r = rel::run(plan, rel::Partition { inputs, anchors, facts: None, source: "", path: "", budget: None });
            out.add_stages(ri, ci, r.stage_ns);
            let mut c = Counts::default();
            out.add_run(ri, ci, &mut c);
            out.add_counts(ri, ci, c, false);
            if let Some(m) = r.failure {
                out.failures.push(Failure { check: rule.check.clone(), clause: clause.id.clone(), path: String::new(), message: m });
            }
            for (p, line, code) in r.findings {
                out.findings.push(Finding { check: rule.check.clone(), clause: clause.id.clone(), path: p, line: line + 1, code: if code.is_empty() { clause.viol.code.clone() } else { code }, column: None });
            }
            let e = out.clause_ns.entry((ri, ci)).or_default();
            e.0 += 1;
            e.1 += t0.elapsed().as_nanos() as u64;
        }
    }
    // `report_when`: a rule with no more findings than its gate reports none (native
    // eval/mod.rs `evaluate_code_rule`, over the whole evaluation).
    for rule in rules {
        if let Some(gt) = rule.report_count_gt {
            if out.findings.iter().filter(|f| f.check == rule.check).count() as u64 <= gt {
                out.findings.retain(|f| f.check != rule.check);
            }
        }
    }
}

/// A regex clause's findings in one file (native `eval/query.rs:721-802`): the matches of
/// `ast/lines.rs::matches`, each emitted as (0-based line, 1-based code-point column) of its
/// start or of the anchoring capture's start: under `lines` the scan's own line and the offset in
/// it (native eval/query.rs:754-762), else the file's line table's point; `file.path` matches
/// report line 0. The column is the harness's `[...prefix].length + 1` over native's span.
fn regex_findings(rc: &RegexClause, path: &str, source: &str, lines: &rerail_engine::code_rule_dsl::lines::LineTable, emit: &mut dyn FnMut(u32, u32), anchor: &Anchor, names: &Names) {
    let haystack = match rc.source {
        RegexSource::Path => path,
        RegexSource::Content => source,
    };
    ast::lines::matches(&rc.rx, haystack, &rc.options, &mut |m| {
        // Captures are read in the match's context (a line under `lines`), as native does.
        let context = &haystack[m.context.clone()];
        let start = match anchor {
            Anchor::Capture(n) => {
                let name = &names.names[*n as usize];
                rc.rx.captures_at(context, m.range.start).and_then(|caps| caps.name(name).map(|g| g.start())).unwrap_or(m.range.start)
            }
            _ => m.range.start,
        };
        let (line, prefix) = match (m.line, &rc.source) {
            (Some(line), _) => (line as u32, &context[..start]),
            (None, RegexSource::Path) => (0, &haystack[..m.context.start + start]),
            (None, RegexSource::Content) => {
                let at = m.context.start + start;
                let (line, column) = lines.point(at);
                (line as u32, &haystack[at - column..at])
            }
        };
        emit(line, prefix.chars().count() as u32 + 1)
    });
}
