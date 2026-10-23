//! Policy front end: a `rerail.policy/v1` document, resolved for a configuration, to
//! unclassified clauses over the algebra (`Rel<AnySource>`). Pattern queries stay leaf relations
//! (`InputKind::Pattern`), compiled by the leaf executor through [`LeafCheck`]; SQL is lowered by
//! `front::sql`. Every refused construct is collected, named, with where it occurs.
use super::sql::{self, Catalog};
use crate::ir::{Anchor, AnySource, ClosureStage, Column, Expr, Graph, InputDef, InputId, InputKind, LocalId, Partition, Projection, Refusal, Rel, Stage, StageDef, SyntaxSpec, SyntaxTable, Violation};
use crate::policy;
use crate::sql::parse;
use crate::sql::value::Aff;
use serde_json::{Map, Value as Json};
use std::collections::BTreeMap;

/// The pattern leaf executor's compile check.
pub trait LeafCheck {
    /// Compiles a resolved `ast` query (with the document's `define.utils`) as the leaf executor
    /// does. `Ok` lists the constructs it compiles but refuses per content at run time; `Err`
    /// names the construct it refuses.
    fn pattern(&mut self, ast: &Json, utils: &BTreeMap<String, Json>) -> Result<Vec<String>, String>;
    /// Whether the regex leaf executor implements a `forbid.regex` spec (native keys checked
    /// already); `Err` names the construct it lacks.
    fn regex(&mut self, spec: &Json) -> Result<(), String>;
}

/// A clause before classification.
pub struct FrontClause {
    pub id: String,
    /// `relational.partitionBy` as declared (`$option` resolved); `None` for `forbid`/`budget`.
    pub declared: Option<Partition>,
    /// `None` when the clause has a refusal.
    pub body: Option<ClauseBody>,
}

pub enum ClauseBody {
    Forbid { input: InputId, violation: Violation },
    Budget { input: InputId, closure: Vec<ClosureStage>, condition: Option<Json>, group_by: Vec<String>, distinct: Vec<String>, lte: u64, violation: Violation },
    Relational { syntax: Option<SyntaxSpec>, stages: Vec<Stage<AnySource>>, fin: Rel<AnySource>, key: Vec<String>, code: Option<String>, locations: Vec<String>, violation: Violation },
}

/// A rule before classification.
pub struct Front {
    pub rule: String,
    pub inputs: Vec<InputDef>,
    /// Per input: the cross-file construct its pattern reads (a `calls` atom), if any.
    pub input_cross: Vec<Option<String>>,
    pub clauses: Vec<FrontClause>,
    /// Clauses whose `when` does not hold under the configuration (never compiled).
    pub disabled: Vec<String>,
    pub report_when: Option<u64>,
    pub refusals: Vec<Refusal>,
    pub runtime_refusals: Vec<Refusal>,
    /// Plan-wide numbering of `WITH` bindings.
    pub next_local: LocalId,
}

fn obj<'j>(v: &'j Json, at: &str) -> Result<&'j Map<String, Json>, String> {
    v.as_object().ok_or_else(|| format!("{at}: expected an object"))
}

fn check_keys(o: &Map<String, Json>, allowed: &[&str], at: &str) -> Result<(), String> {
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!("{at}: key `{k}`"));
        }
    }
    Ok(())
}

pub fn projection(v: &Json) -> Result<Projection, String> {
    if let Some(o) = v.as_object() {
        check_keys(o, &["source", "regex", "group"], "projection")?;
        let source = projection(o.get("source").ok_or("projection source")?)?;
        let regex = o.get("regex").and_then(Json::as_str).ok_or("projection regex")?.to_owned();
        regex::Regex::new(&regex).map_err(|e| format!("projection regex: {e}"))?;
        let group = o.get("group").and_then(Json::as_u64).ok_or("projection group")? as u32;
        return Ok(Projection::Regex { source: Box::new(source), regex, group });
    }
    let s = v.as_str().ok_or("projection: expected a string")?;
    Ok(match s {
        "file.path" => Projection::Path,
        "file.directory" => Projection::Directory,
        "file.name" => Projection::Name,
        "file.stem" => Projection::Stem,
        "file.module" => Projection::Module,
        "file.package" => Projection::Package,
        _ => {
            let rest = s.strip_prefix("capture.").ok_or_else(|| format!("projection {s}"))?;
            match rest.split_once('.') {
                None => Projection::Capture(rest.into()),
                Some((n, "text")) => Projection::Text(n.into()),
                Some((n, "kind")) => Projection::Kind(n.into()),
                Some((n, "startByte")) => Projection::StartByte(n.into()),
                Some((n, "endByte")) => Projection::EndByte(n.into()),
                Some((n, "startLine")) => Projection::StartLine(n.into()),
                _ => return Err(format!("projection {s}")),
            }
        }
    })
}

pub fn anchor(v: Option<&Json>) -> Result<Anchor, String> {
    Ok(match v.and_then(Json::as_str).unwrap_or("witness.span") {
        "witness.span" => Anchor::Span,
        "witness.start" => Anchor::Start,
        "witness.end" => Anchor::End,
        "file.start" => Anchor::FileStart,
        a => Anchor::Capture(a.strip_prefix("capture.").ok_or_else(|| format!("anchor {a}"))?.into()),
    })
}

fn violation(c: &Json) -> Result<Violation, String> {
    let v = c.get("violation").ok_or("violation")?;
    Ok(Violation {
        code: v.get("code").and_then(Json::as_str).unwrap_or("").to_owned(),
        anchor: anchor(v.get("anchor"))?,
        each: v.get("each").and_then(Json::as_str).map(str::to_owned),
    })
}

/// Whether a JSON rule (through the utilities it names) holds a key.
fn mentions(v: &Json, key: &str, utils: &BTreeMap<String, Json>, seen: &mut Vec<String>) -> bool {
    match v {
        Json::Object(o) => o.iter().any(|(k, x)| {
            if k == key {
                return true;
            }
            if k == "matches" {
                if let Some(id) = x.as_str() {
                    if !seen.iter().any(|s| s == id) {
                        seen.push(id.to_owned());
                        if let Some(u) = utils.get(id) {
                            if mentions(u, key, utils, seen) {
                                return true;
                            }
                        }
                    }
                }
            }
            mentions(x, key, utils, seen)
        }),
        Json::Array(a) => a.iter().any(|x| mentions(x, key, utils, seen)),
        _ => false,
    }
}

/// Rewrite `file-path-gate` (docs/facts/REWRITES.md): an ast query's `file: { path: C }` gate
/// becomes the rule `all: [{ filePath: C }, rule]`. Both are `TextCondition::holds(file.path)`
/// (unanchored `regex` must match, `notRegex` must not), constant over a file's nodes and
/// binding nothing, so every root's verdict is the gate's conjunction with the rule's.
/// Side conditions: `file` holds `path` only (module keys read other contents), and the query
/// sets `syntax` (without it a gated-out file still reports partial-source failures natively).
fn file_path_gate(query: &mut Json) {
    let Some(q) = query.as_object_mut() else { return };
    let only_path = q.get("file").and_then(Json::as_object).is_some_and(|f| f.len() == 1 && f.contains_key("path"));
    if !only_path || !q.contains_key("syntax") {
        return;
    }
    let file = q.remove("file").unwrap();
    let mut cond = file["path"].clone();
    if crate::mutant::on(11) {
        // Mutant 11 (dropped side condition): the gate loses its `notRegex`.
        if let Some(o) = cond.as_object_mut() {
            o.remove("notRegex");
        }
    }
    let rule = q.remove("rule").unwrap_or(Json::Null);
    q.insert("rule".into(), serde_json::json!({ "all": [{ "filePath": cond }, rule] }));
}

/// A `when` on a Python ast query (native `compile_python_when`): `option` conditions on the
/// document's options and `pythonVersion` against Ruff's default 3.10.
pub fn when_holds(when: &Json, options: &BTreeMap<String, Json>) -> Result<bool, String> {
    let w = obj(when, "when")?;
    check_keys(w, &["option", "pythonVersion"], "when")?;
    let mut holds = true;
    if let Some(opt) = w.get("option") {
        for (name, cond) in obj(opt, "when.option")? {
            let value = options.get(name).ok_or_else(|| format!("when.option: unknown option {name}"))?;
            holds &= condition_holds(value, cond)?;
        }
    }
    if let Some(range) = w.get("pythonVersion") {
        let r = obj(range, "pythonVersion")?;
        let minor = |k: &str| -> Result<Option<u32>, String> {
            r.get(k)
                .map(|v| v.as_str().and_then(|s| s.strip_prefix("3.")).and_then(|m| m.parse().ok()).ok_or_else(|| format!("pythonVersion.{k}")))
                .transpose()
        };
        let (min, max) = (minor("min")?, minor("max")?);
        holds &= min.is_none_or(|m| m <= 10) && max.is_none_or(|m| 10 <= m);
    }
    Ok(holds)
}

fn condition_holds(value: &Json, cond: &Json) -> Result<bool, String> {
    let Some(fields) = cond.as_object() else { return Ok(value == cond) };
    if fields.len() != 1 {
        return Err("when.option condition".into());
    }
    let (k, operand) = fields.iter().next().unwrap();
    match k.as_str() {
        "in" => Ok(operand.as_array().ok_or("when in")?.contains(value)),
        "includes" => Ok(value.as_array().ok_or("when includes")?.contains(operand)),
        "not" => Ok(!condition_holds(value, operand)?),
        o => Err(format!("when.option condition `{o}`")),
    }
}

/// Columns of an input relation: `__path, __start, __end`, then the fields in name order.
pub fn input_schema(fields: &[(String, Projection)]) -> Vec<Column> {
    let mut s = vec![Column::new("__path", Aff::Text), Column::new("__start", Aff::Integer), Column::new("__end", Aff::Integer)];
    s.extend(fields.iter().map(|(f, _)| Column::new(f, Aff::None)));
    s
}

/// The `syntax` spec's tables (spec-rel § 2.3–2.6).
fn syntax_schemas(spec: &Map<String, Json>) -> Result<(Vec<Column>, Option<Vec<Column>>, Option<Vec<Column>>), String> {
    check_keys(spec, &["name", "gate", "fields", "columns", "excludeKinds", "emptyTextKinds", "edges", "ownership", "owners"], "syntax")?;
    let mut cols: Vec<Column> = [
        ("__path", Aff::Text),
        ("__start", Aff::Integer),
        ("__end", Aff::Integer),
        ("id", Aff::None),
        ("parent", Aff::None),
        ("edge_parent", Aff::None),
        ("kind", Aff::None),
        ("line", Aff::None),
        ("text", Aff::None),
        ("ordinal", Aff::None),
        ("parent_start", Aff::None),
        ("parent_end", Aff::None),
        ("parent_kind", Aff::None),
        ("ancestor_start", Aff::None),
        ("ancestor_end", Aff::None),
        ("ancestor_kind", Aff::None),
    ]
    .iter()
    .map(|(n, a)| Column::new(n, *a))
    .collect();
    for f in spec.get("fields").and_then(Json::as_array).into_iter().flatten() {
        let f = f.as_str().ok_or("syntax.fields")?;
        for s in ["id", "start", "end", "kind", "source"] {
            cols.push(Column::new(&format!("{f}_{s}"), Aff::None));
        }
    }
    if let Some(keep) = spec.get("columns").and_then(Json::as_array).filter(|a| !a.is_empty()) {
        cols.retain(|c| keep.iter().any(|k| k.as_str() == Some(&c.name)));
    }
    let edges = spec.get("edges").map(|_| ["__path", "src", "dst", "ordinal"].iter().map(|n| Column::new(n, if *n == "__path" { Aff::Text } else { Aff::None })).collect());
    let ownership = spec.get("ownership").map(|_| {
        let mut c = vec![Column::new("__path", Aff::Text), Column::new("id", Aff::None)];
        // Owner columns in name order (a BTreeMap natively).
        let mut owners: Vec<&String> = spec.get("owners").and_then(Json::as_object).map(|o| o.keys().collect()).unwrap_or_default();
        owners.sort();
        c.extend(owners.into_iter().map(|o| Column::new(o, Aff::None)));
        c
    });
    Ok((cols, edges, ownership))
}

/// `reference_edges`: an input-shaped table (`__path, __start, __end`, then the native
/// `REFERENCE_FIELDS` in their order, `relational.rs:48-55`, `:640-679`).
fn reference_edges_schema() -> Vec<Column> {
    let mut s = vec![Column::new("__path", Aff::Text), Column::new("__start", Aff::Integer), Column::new("__end", Aff::Integer)];
    s.extend(["target_path", "target_start", "target_end", "source_name", "target_name", "target_class"].iter().map(|n| Column::new(n, Aff::None)));
    s
}

struct Ctx<'a> {
    utils: BTreeMap<String, Json>,
    options: BTreeMap<String, Json>,
    leaves: &'a mut dyn LeafCheck,
    keys: BTreeMap<String, InputId>,
}

impl Front {
    fn refuse(&mut self, clause: &str, at: &str, construct: impl Into<String>) {
        self.refusals.push(Refusal { clause: clause.into(), at: at.into(), construct: construct.into() });
    }

    /// An evidence object (or a forbid's `ast`, wrapped) as an input; identical evidence objects
    /// share one input.
    fn input(&mut self, ctx: &mut Ctx<'_>, clause: &str, at: &str, name: &str, ev: &Json) -> Result<InputId, ()> {
        let key = serde_json::to_string(ev).unwrap();
        if let Some(&i) = ctx.keys.get(&key) {
            return Ok(i);
        }
        let r: Result<InputDef, String> = (|| {
            let o = obj(ev, "evidence")?;
            check_keys(o, &["as", "paths", "query", "project", "where", "closure", "each", "anchor"], "evidence")?;
            let q = obj(o.get("query").ok_or("evidence.query")?, "evidence.query")?;
            check_keys(q, &["ast"], "evidence.query")?;
            let ast = q.get("ast").ok_or("evidence.query.ast")?;
            let mut fields = Vec::new();
            for (f, p) in o.get("project").and_then(Json::as_object).into_iter().flatten() {
                fields.push((f.clone(), projection(p)?));
            }
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            let schema = input_schema(&fields);
            Ok(InputDef {
                name: name.to_owned(),
                kind: InputKind::Pattern { query: ast.clone() },
                fields,
                anchor: anchor(o.get("anchor"))?,
                each: o.get("each").and_then(Json::as_str).map(str::to_owned),
                paths: o.get("paths").cloned(),
                schema,
            })
        })();
        let def = match r {
            Ok(d) => d,
            Err(e) => {
                self.refuse(clause, at, e);
                return Err(());
            }
        };
        let InputKind::Pattern { query } = &def.kind else { unreachable!() };
        let mut query = query.clone();
        if let Some(w) = query.get("when").cloned() {
            match when_holds(&w, &ctx.options) {
                Ok(true) => {
                    query.as_object_mut().unwrap().remove("when");
                }
                Ok(false) => {
                    self.refuse(clause, at, "`when` false on an evidence query (an input that never applies)");
                    return Err(());
                }
                Err(e) => {
                    self.refuse(clause, at, e);
                    return Err(());
                }
            }
        }
        file_path_gate(&mut query);
        let mut local_utils = ctx.utils.clone();
        if let Some(u) = query.get("utils").and_then(Json::as_object) {
            local_utils.extend(u.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let cross = if query.get("rule").is_some_and(|r| mentions(r, "calls", &local_utils, &mut Vec::new())) {
            Some("pattern atom `calls` (the reference graph)".to_owned())
        } else if def.fields.iter().any(|(_, p)| matches!(p, Projection::Module | Projection::Package)) {
            Some("projection `file.module`/`file.package` (package markers of other contents)".to_owned())
        } else {
            None
        };
        match ctx.leaves.pattern(&query, &ctx.utils) {
            Ok(runtime) => {
                for c in runtime {
                    self.runtime_refusals.push(Refusal { clause: clause.into(), at: at.into(), construct: c });
                }
            }
            Err(e) => {
                self.refuse(clause, at, format!("pattern: {e}"));
                return Err(());
            }
        }
        let mut def = def;
        def.kind = InputKind::Pattern { query };
        self.inputs.push(def);
        self.input_cross.push(cross);
        let id = (self.inputs.len() - 1) as InputId;
        ctx.keys.insert(key, id);
        Ok(id)
    }

    fn regex_input(&mut self, ctx: &mut Ctx<'_>, clause: &str, spec: &Json) -> Result<InputId, String> {
        // The native `forbid.regex` keys (`compile/mod.rs:606-765`).
        let o = obj(spec, "forbid.regex")?;
        check_keys(o, &["source", "pattern", "include", "exclude", "lines", "paths", "content"], "forbid.regex")?;
        match o.get("source").and_then(Json::as_str) {
            Some("file.content") | Some("file.path") => {}
            s => return Err(format!("forbid.regex: source {s:?}")),
        }
        regex::Regex::new(o.get("pattern").and_then(Json::as_str).ok_or("forbid.regex.pattern")?).map_err(|e| format!("forbid.regex.pattern: {e}"))?;
        if let Some(l) = o.get("lines") {
            check_keys(obj(l, "forbid.regex.lines")?, &["include", "exclude", "first", "terminators", "state", "indentation"], "forbid.regex.lines")?;
        }
        if let Some(c) = o.get("content") {
            check_keys(obj(c, "forbid.regex.content")?, &["include", "exclude"], "forbid.regex.content")?;
        }
        ctx.leaves.regex(spec).map_err(|e| format!("regex leaf: {e}"))?;
        let def = InputDef { name: clause.into(), kind: InputKind::Regex { spec: spec.clone() }, fields: vec![], anchor: Anchor::Span, each: None, paths: None, schema: input_schema(&[]) };
        self.inputs.push(def);
        self.input_cross.push(None);
        Ok((self.inputs.len() - 1) as InputId)
    }
}

fn closure_stages(v: &Json) -> Result<Vec<ClosureStage>, String> {
    let strs = |o: &Map<String, Json>, k: &str| -> Result<Vec<String>, String> {
        match o.get(k) {
            None => Ok(vec![]),
            Some(Json::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_owned).ok_or_else(|| format!("closure.{k}"))).collect(),
            Some(_) => Err(format!("closure.{k}")),
        }
    };
    let mut out = Vec::new();
    for st in v.as_array().ok_or("closure")? {
        let o = obj(st, "closure")?;
        check_keys(o, &["name", "seeds", "seed_field", "edges", "extends", "partition_by", "contains"], "closure")?;
        let edges = match o.get("edges") {
            None => None,
            Some(e) => {
                let e = obj(e, "closure.edges")?;
                Some((e.get("from").and_then(Json::as_str).ok_or("closure.edges.from")?.to_owned(), e.get("to").and_then(Json::as_str).ok_or("closure.edges.to")?.to_owned()))
            }
        };
        out.push(ClosureStage {
            name: o.get("name").and_then(Json::as_str).ok_or("closure.name")?.to_owned(),
            seeds: strs(o, "seeds")?,
            seed_field: o.get("seed_field").and_then(Json::as_str).map(str::to_owned),
            extends: strs(o, "extends")?,
            partition_by: strs(o, "partition_by")?,
            edges,
            contains: o.get("contains").and_then(Json::as_object).into_iter().flatten().map(|(k, v)| Ok((k.clone(), v.as_str().ok_or("closure.contains")?.to_owned()))).collect::<Result<_, String>>()?,
        });
    }
    Ok(out)
}

fn lower_sql(text: &str, catalog: &Catalog, next_local: &mut LocalId) -> Result<(Rel<AnySource>, Vec<Column>), String> {
    let q = parse::parse_query(text).map_err(|e| format!("SQL: {e}"))?;
    sql::lower_query(&q, catalog, next_local).map_err(|e| format!("SQL: {e}"))
}

/// The front end over one document under `config`.
pub fn front(document: &Json, config: &Map<String, Json>, leaves: &mut dyn LeafCheck) -> Result<Front, String> {
    let doc = policy::resolve(document, config)?;
    let options = policy::option_values(document, config)?;
    let rule = doc.get("id").and_then(Json::as_str).unwrap_or("").to_owned();
    let mut utils = BTreeMap::new();
    let mut report_when = None;
    for c in doc["clauses"].as_array().ok_or("clauses")? {
        if let Some(d) = c.get("define") {
            if let Some(u) = d.get("utils").and_then(Json::as_object) {
                utils.extend(u.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            if let Some(rw) = d.get("report_when") {
                report_when = Some(rw.get("count").and_then(|c| c.get("gt")).and_then(Json::as_u64).ok_or("report_when.count.gt")?);
            }
        }
    }
    let mut f = Front { rule, inputs: vec![], input_cross: vec![], clauses: vec![], disabled: vec![], report_when, refusals: vec![], runtime_refusals: vec![], next_local: 0 };
    let mut ctx = Ctx { utils, options, leaves, keys: BTreeMap::new() };
    for c in doc["clauses"].as_array().unwrap() {
        if c.get("define").is_some() {
            continue;
        }
        let id = c.get("id").and_then(Json::as_str).unwrap_or("").to_owned();
        let before = f.refusals.len();
        let viol = match violation(c) {
            Ok(v) => v,
            Err(e) => {
                f.refuse(&id, "violation", e);
                f.clauses.push(FrontClause { id, declared: None, body: None });
                continue;
            }
        };
        let (declared, body) = if let Some(fb) = c.get("forbid") {
            if let Some(ast) = fb.get("ast") {
                if let Some(w) = ast.get("when") {
                    match when_holds(w, &ctx.options) {
                        Ok(true) => {}
                        Ok(false) => {
                            f.disabled.push(id);
                            continue;
                        }
                        Err(e) => {
                            f.refuse(&id, "forbid.ast.when", e);
                            f.clauses.push(FrontClause { id, declared: None, body: None });
                            continue;
                        }
                    }
                }
                let ev = serde_json::json!({ "as": "row", "query": { "ast": ast }, "project": {} });
                let input = f.input(&mut ctx, &id, "forbid.ast", &id, &ev);
                (None, input.ok().map(|input| ClauseBody::Forbid { input, violation: viol }))
            } else if let Some(r) = fb.get("regex") {
                match f.regex_input(&mut ctx, &id, r) {
                    Ok(input) => (None, Some(ClauseBody::Forbid { input, violation: viol })),
                    Err(e) => {
                        f.refuse(&id, "forbid.regex", e);
                        (None, None)
                    }
                }
            } else {
                f.refuse(&id, "forbid", "forbid form other than ast/regex");
                (None, None)
            }
        } else if let Some(b) = c.get("budget") {
            (None, budget(&mut f, &mut ctx, &id, b, viol))
        } else if let Some(r) = c.get("relational") {
            relational(&mut f, &mut ctx, &id, r, viol)
        } else {
            f.refuse(&id, "clause", "clause form other than forbid/budget/relational");
            (None, None)
        };
        let body = if f.refusals.len() > before { None } else { body };
        f.clauses.push(FrontClause { id, declared, body });
    }
    Ok(f)
}

fn budget(f: &mut Front, ctx: &mut Ctx<'_>, id: &str, b: &Json, viol: Violation) -> Option<ClauseBody> {
    let r: Result<_, String> = (|| {
        let o = obj(b, "budget")?;
        check_keys(o, &["query", "group_by", "count"], "budget")?;
        let ev = o.get("query").and_then(|q| q.get("evidence")).ok_or("budget.query.evidence")?;
        let alias = ev.get("as").and_then(Json::as_str).ok_or("evidence.as")?.to_owned();
        let field = |s: &Json| -> Result<String, String> {
            let s = s.as_str().ok_or("budget operand")?;
            s.strip_prefix(&format!("{alias}.")).map(str::to_owned).ok_or_else(|| format!("budget operand {s}"))
        };
        let group_by = o.get("group_by").and_then(Json::as_array).into_iter().flatten().map(field).collect::<Result<Vec<_>, _>>()?;
        let count = obj(o.get("count").ok_or("budget.count")?, "budget.count")?;
        check_keys(count, &["lte", "distinct"], "budget.count")?;
        let lte = count.get("lte").and_then(Json::as_u64).ok_or("budget.count.lte")?;
        let distinct = count.get("distinct").and_then(Json::as_array).into_iter().flatten().map(field).collect::<Result<Vec<_>, _>>()?;
        let closure = ev.get("closure").map(closure_stages).transpose()?.unwrap_or_default();
        Ok((ev.clone(), group_by, lte, distinct, closure, ev.get("where").cloned()))
    })();
    match r {
        Err(e) => {
            f.refuse(id, "budget", e);
            None
        }
        Ok((ev, group_by, lte, distinct, closure, condition)) => {
            let input = f.input(ctx, id, "budget.query.evidence", id, &ev).ok()?;
            Some(ClauseBody::Budget { input, closure, condition, group_by, distinct, lte, violation: viol })
        }
    }
}

fn relational(f: &mut Front, ctx: &mut Ctx<'_>, id: &str, r: &Json, viol: Violation) -> (Option<Partition>, Option<ClauseBody>) {
    let Some(o) = r.as_object() else {
        f.refuse(id, "relational", "relational: expected an object");
        return (None, None);
    };
    if let Err(e) = check_keys(o, &["inputs", "stages", "query", "key", "code", "references", "locations", "partitionBy", "syntax"], "relational") {
        f.refuse(id, "relational", e);
        return (None, None);
    }
    let declared = match o.get("partitionBy") {
        Some(Json::String(s)) if s == "file" => Partition::File,
        None | Some(Json::Null) => Partition::Repository,
        Some(Json::String(s)) if s == "repository" => Partition::Repository,
        Some(v) => {
            f.refuse(id, "relational.partitionBy", format!("partitionBy {v}"));
            return (None, None);
        }
    };
    let mut catalog = Catalog::default();
    let mut named: Vec<(&String, &Json)> = o.get("inputs").and_then(Json::as_object).map(|m| m.iter().collect()).unwrap_or_default();
    named.sort_by(|a, b| a.0.cmp(b.0));
    let mut input_ids = BTreeMap::new();
    for (name, inp) in named {
        let at = format!("inputs/{name}");
        let Some(ev) = inp.get("evidence") else {
            f.refuse(id, &at, "relational input without evidence");
            continue;
        };
        if ev.get("where").is_some() || ev.get("closure").is_some() {
            f.refuse(id, &at, "evidence where/closure on a relational input");
            continue;
        }
        if let Ok(i) = f.input(ctx, id, &at, name, ev) {
            catalog.insert(name, AnySource::Input(i), f.inputs[i as usize].schema.clone());
            input_ids.insert(name.clone(), i);
        }
    }
    let mut syntax = None;
    if let Some(s) = o.get("syntax") {
        match s.as_object().ok_or_else(|| "syntax: expected an object".to_string()).and_then(|so| syntax_schemas(so).map(|t| (so, t))) {
            Err(e) => f.refuse(id, "syntax", e),
            Ok((so, (nodes, edges, ownership))) => {
                let gate = so.get("gate").and_then(Json::as_str).and_then(|g| input_ids.get(g).copied());
                match gate {
                    None => f.refuse(id, "syntax.gate", "syntax gate is not an input"),
                    Some(gate) => {
                        let name = |k: &str| so.get(k).and_then(Json::as_str).map(str::to_owned);
                        if let Some(n) = name("name") {
                            catalog.insert(&n, AnySource::Syntax(SyntaxTable::Nodes), nodes.clone());
                        }
                        if let (Some(n), Some(e)) = (name("edges"), &edges) {
                            catalog.insert(&n, AnySource::Syntax(SyntaxTable::Edges), e.clone());
                        }
                        if let (Some(n), Some(w)) = (name("ownership"), &ownership) {
                            catalog.insert(&n, AnySource::Syntax(SyntaxTable::Ownership), w.clone());
                        }
                        syntax = Some(SyntaxSpec { spec: s.clone(), gate, nodes, edges, ownership });
                    }
                }
            }
        }
    }
    if let Some(refs) = o.get("references") {
        let _ = refs;
        catalog.insert("reference_edges", AnySource::ReferenceEdges, reference_edges_schema());
    }
    let mut stages: Vec<Stage<AnySource>> = Vec::new();
    for st in o.get("stages").and_then(Json::as_array).into_iter().flatten() {
        let name = st.get("name").and_then(Json::as_str).unwrap_or("").to_owned();
        let at = format!("stages/{name}");
        let res: Result<Stage<AnySource>, String> = (|| {
            let so = obj(st, "stage")?;
            check_keys(so, &["name", "query", "indexes", "reachability", "deferred"], "stage")?;
            if let Some(rch) = so.get("reachability") {
                let ro = obj(rch, "reachability")?;
                check_keys(ro, &["roots", "edges", "batchSize", "graph", "syntax"], "reachability")?;
                let (roots, rs) = lower_sql(ro.get("roots").and_then(Json::as_str).ok_or("reachability.roots")?, &catalog, &mut f.next_local)?;
                if rs.len() < 2 || rs[0].name != "__path" || rs[1].name != "id" {
                    return Err("reachability roots must return __path, id".into());
                }
                let edges = ro.get("edges").and_then(Json::as_str).map(|e| lower_sql(e, &catalog, &mut f.next_local)).transpose()?;
                if let Some((_, es)) = &edges {
                    if es.len() < 3 || es[0].name != "__path" || es[1].name != "src" || es[2].name != "dst" {
                        return Err("reachability edges must return __path, src, dst".into());
                    }
                }
                let graph = if ro.contains_key("syntax") {
                    if edges.is_some() || ro.contains_key("graph") {
                        return Err("syntax reachability with edges/graph".into());
                    }
                    Graph::Syntax
                } else {
                    match ro.get("graph").and_then(Json::as_str) {
                        None | Some("directed") => Graph::Directed,
                        Some("forest") => Graph::Forest,
                        Some(g) => return Err(format!("reachability graph {g}")),
                    }
                };
                let schema = vec![Column::new("__path", Aff::Text), Column::new("root", Aff::None), Column::new("node", Aff::None)];
                return Ok(Stage { name: name.clone(), schema, def: StageDef::Reach { roots, edges: edges.map(|e| e.0), graph } });
            }
            let (rel, schema) = lower_sql(so.get("query").and_then(Json::as_str).ok_or("stage query")?, &catalog, &mut f.next_local)?;
            let Some(d) = so.get("deferred").and_then(Json::as_object) else {
                return Ok(Stage { name: name.clone(), schema, def: StageDef::Query { rel } });
            };
            // Deferred columns: expressions over the stored columns, replacing in place or
            // appended (spec-rel § 3.3), lowered into a Project.
            let stored: Vec<(usize, Column)> = schema.iter().enumerate().filter(|(_, c)| !d.contains_key(&c.name)).map(|(i, c)| (i, c.clone())).collect();
            let stored_schema: Vec<Column> = stored.iter().map(|(_, c)| c.clone()).collect();
            let rebase = |e: Expr<AnySource>| rebase_cols(e, &stored.iter().map(|(i, _)| *i as u16).collect::<Vec<_>>());
            let mut out_schema = schema.clone();
            let mut exprs: Vec<Expr<AnySource>> = (0..schema.len()).map(|i| Expr::Col { depth: 0, col: i as u16 }).collect();
            for (col, text) in d {
                let pe = parse::parse_expr(text.as_str().ok_or("deferred")?).map_err(|e| format!("deferred {col}: SQL: {e}"))?;
                let (e, _) = sql::lower_scalar(&pe, &stored_schema, &mut f.next_local).map_err(|e| format!("deferred {col}: {e}"))?;
                let e = rebase(e);
                match out_schema.iter().position(|c| c.name == *col) {
                    Some(p) => {
                        out_schema[p].aff = Aff::None;
                        exprs[p] = e;
                    }
                    None => {
                        out_schema.push(Column::new(col, Aff::None));
                        exprs.push(e);
                    }
                }
            }
            Ok(Stage { name: name.clone(), schema: out_schema.clone(), def: StageDef::Query { rel: Rel::Project { input: Box::new(rel), exprs, schema: out_schema } } })
        })();
        match res {
            Ok(stage) => {
                catalog.insert(&name, AnySource::Stage(stages.len() as u32), stage.schema.clone());
                stages.push(stage);
            }
            Err(e) => {
                f.refuse(id, &at, e);
                return (Some(declared), None);
            }
        }
    }
    let fin = match o.get("query").and_then(Json::as_str).ok_or_else(|| "final query".to_string()).and_then(|q| lower_sql(q, &catalog, &mut f.next_local)) {
        Ok((rel, schema)) => {
            for required in ["path", "start_byte", "end_byte"] {
                if !schema.iter().any(|c| c.name == required) {
                    f.refuse(id, "query", format!("final query must select {required}"));
                }
            }
            rel
        }
        Err(e) => {
            f.refuse(id, "query", e);
            return (Some(declared), None);
        }
    };
    let strs = |k: &str| o.get(k).and_then(Json::as_array).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect::<Vec<_>>();
    let body = ClauseBody::Relational { syntax, stages, fin, key: strs("key"), code: o.get("code").and_then(Json::as_str).map(str::to_owned), locations: strs("locations"), violation: viol };
    (Some(declared), Some(body))
}

/// Maps `Col { depth: 0, col: i }` (an index into the stored columns) to `map[i]` (its index in
/// the query's output).
fn rebase_cols(e: Expr<AnySource>, map: &[u16]) -> Expr<AnySource> {
    // Mutant 12 (wrong column map): deferred expressions read the query's column at the stored
    // position.
    crate::ir::map_cols(e, &mut |depth, col| if depth == 0 && !crate::mutant::on(12) { map[col as usize] } else { col })
}
