//! Relational clauses: evidence inputs as tables, the `syntax` spec's node/edge/ownership tables
//! from facts, SQL stages (CTAS semantics, deferred columns, reachability), and the final query's
//! rows as findings with the native engine's anchoring and identity rules.
use crate::facts::{self, Facts};
use crate::sql::engine::{self, Catalog, Db, PE, Schema, Statement, Table};
use crate::sql::value::{self as sv, Aff, V};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

pub fn input_schema(fields: &[String]) -> Schema {
    let mut cols = vec!["__path".to_string(), "__start".to_string(), "__end".to_string()];
    let mut affs = vec![Aff::Text, Aff::Integer, Aff::Integer];
    for f in fields {
        cols.push(f.clone());
        affs.push(Aff::None);
    }
    Schema { cols, affs }
}

#[derive(Clone)]
pub struct Owner {
    pub kind: String,
    pub outermost: bool,
}

#[derive(Clone)]
pub struct SyntaxSpec {
    pub name: String,
    pub gate: String,
    pub fields: Vec<(String, u16)>,
    pub columns: Vec<String>,
    pub exclude: Vec<u32>,
    pub empty_text: Vec<u32>,
    pub edges: Option<String>,
    pub ownership: Option<String>,
    pub owners: BTreeMap<String, Owner>,
}

#[derive(Clone, Copy, PartialEq)]
enum SynCol {
    Path,
    Start,
    End,
    Id,
    Parent,
    Kind,
    Line,
    Text,
    Ordinal,
    ParentStart,
    ParentEnd,
    ParentKind,
    Field(usize, u8),
}

impl SyntaxSpec {
    pub fn parse(v: &Json, grammar: &rerail_engine::code_rule_dsl::tape::Grammar) -> Result<Self, String> {
        let o = v.as_object().ok_or("syntax")?;
        for k in o.keys() {
            if !["name", "gate", "fields", "columns", "excludeKinds", "emptyTextKinds", "edges", "ownership", "owners"].contains(&k.as_str()) {
                return Err(format!("syntax key {k}"));
            }
        }
        let strs = |k: &str| -> Vec<String> { o.get(k).and_then(Json::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default() };
        let fields = strs("fields").into_iter().map(|f| {
            let code = grammar.field_code(&f).unwrap_or(0);
            (f, code)
        }).collect();
        let mut owners = BTreeMap::new();
        if let Some(ow) = o.get("owners").and_then(Json::as_object) {
            for (k, v) in ow {
                owners.insert(k.clone(), Owner { kind: v["kind"].as_str().unwrap_or("").to_owned(), outermost: v.get("outermost").and_then(Json::as_bool).unwrap_or(false) });
            }
        }
        Ok(Self {
            name: o["name"].as_str().ok_or("syntax.name")?.to_owned(),
            gate: o["gate"].as_str().ok_or("syntax.gate")?.to_owned(),
            fields,
            columns: strs("columns"),
            exclude: strs("excludeKinds").iter().map(|k| facts::intern(k)).collect(),
            empty_text: strs("emptyTextKinds").iter().map(|k| facts::intern(k)).collect(),
            edges: o.get("edges").and_then(Json::as_str).map(str::to_owned),
            ownership: o.get("ownership").and_then(Json::as_str).map(str::to_owned),
            owners,
        })
    }

    fn node_columns(&self) -> Vec<(String, SynCol)> {
        let mut cols: Vec<(String, SynCol)> = [
            ("__path", SynCol::Path),
            ("__start", SynCol::Start),
            ("__end", SynCol::End),
            ("id", SynCol::Id),
            ("parent", SynCol::Parent),
            ("edge_parent", SynCol::Parent),
            ("kind", SynCol::Kind),
            ("line", SynCol::Line),
            ("text", SynCol::Text),
            ("ordinal", SynCol::Ordinal),
            ("parent_start", SynCol::ParentStart),
            ("parent_end", SynCol::ParentEnd),
            ("parent_kind", SynCol::ParentKind),
            ("ancestor_start", SynCol::ParentStart),
            ("ancestor_end", SynCol::ParentEnd),
            ("ancestor_kind", SynCol::ParentKind),
        ]
        .iter()
        .map(|(n, c)| (n.to_string(), *c))
        .collect();
        for (fi, (f, _)) in self.fields.iter().enumerate() {
            for (i, s) in ["id", "start", "end", "kind", "source"].iter().enumerate() {
                cols.push((format!("{f}_{s}"), SynCol::Field(fi, i as u8)));
            }
        }
        if !self.columns.is_empty() {
            cols.retain(|(n, _)| self.columns.contains(n));
        }
        cols
    }

    pub fn schemas(&self) -> Vec<(String, Schema)> {
        let cols = self.node_columns();
        let affs = cols.iter().map(|(_, c)| match c {
            SynCol::Path => Aff::Text,
            SynCol::Start | SynCol::End => Aff::Integer,
            _ => Aff::None,
        }).collect();
        let mut out = vec![(self.name.clone(), Schema { cols: cols.into_iter().map(|(n, _)| n).collect(), affs })];
        if let Some(e) = &self.edges {
            out.push((e.clone(), Schema { cols: vec!["__path".into(), "src".into(), "dst".into(), "ordinal".into()], affs: vec![Aff::Text, Aff::None, Aff::None, Aff::None] }));
        }
        if let Some(o) = &self.ownership {
            let mut cols = vec!["__path".to_string(), "id".to_string()];
            cols.extend(self.owners.keys().cloned());
            let n = cols.len();
            out.push((o.clone(), Schema { cols, affs: std::iter::once(Aff::Text).chain(std::iter::repeat_n(Aff::None, n - 1)).collect() }));
        }
        out
    }

    /// The node, edge and ownership tables of one file, as columns over the facts: integers 4
    /// bytes a row, kinds as symbols, texts as spans of the source read on first use, the path
    /// once.
    pub fn tables(&self, f: &Facts, path: &str) -> Vec<(String, Table)> {
        use engine::{Column, TextCell, U32_NULL};
        let schemas = self.schemas();
        let cols = self.node_columns();
        let path_col = || Rc::new(Column::Const(V::Text(Rc::from(path))));
        let n = f.len();
        // Selection: named rows not excluded, ordinals per raw parent.
        let mut ordinals: rustc_hash::FxHashMap<u32, u32> = Default::default();
        let mut selected: Vec<(u32, u32)> = Vec::new();
        for id in 0..n as u32 {
            if !f.named(id) {
                continue;
            }
            if self.exclude.contains(&f.kind[id as usize]) {
                continue;
            }
            let o = ordinals.entry(f.parent[id as usize]).or_insert(0);
            selected.push((id, *o));
            *o += 1;
        }
        let source: Rc<str> = Rc::from(f.source.as_str());
        let span = |id: u32| TextCell::Span(f.start[id as usize], f.end[id as usize]);
        // Per field, each selected row's first child with that field (`facts::NONE`: none).
        let field_children: Vec<Vec<u32>> = self
            .fields
            .iter()
            .map(|&(_, code)| selected.iter().map(|&(id, _)| if code == 0 { None } else { f.children(id).iter().copied().find(|&c| f.field[c as usize] == code) }.unwrap_or(facts::NONE)).collect())
            .collect();
        let parents: Vec<u32> = selected.iter().map(|&(id, _)| f.parent(id).unwrap_or(facts::NONE)).collect();
        let ints = |of: &dyn Fn(usize) -> Option<u32>| Rc::new(Column::U32((0..selected.len()).map(|i| of(i).unwrap_or(U32_NULL)).collect()));
        let node = |x: u32| (x != facts::NONE).then_some(x);
        let node_cols: Vec<Rc<Column>> = cols
            .iter()
            .map(|(_, c)| match *c {
                SynCol::Path => path_col(),
                SynCol::Start => ints(&|i| Some(f.start[selected[i].0 as usize])),
                SynCol::End => ints(&|i| Some(f.end[selected[i].0 as usize])),
                SynCol::Id => ints(&|i| Some(selected[i].0)),
                SynCol::Parent => ints(&|i| node(parents[i])),
                SynCol::Kind => Rc::new(Column::Sym(selected.iter().map(|&(id, _)| f.kind[id as usize]).collect())),
                SynCol::Line => ints(&|i| Some(f.start_line[selected[i].0 as usize].checked_sub(1).expect("start lines are 1-based"))),
                SynCol::Text => Rc::new(Column::text(source.clone(), selected.iter().map(|&(id, _)| if self.empty_text.contains(&f.kind[id as usize]) { TextCell::Val(V::text("")) } else { span(id) }).collect())),
                SynCol::Ordinal => ints(&|i| Some(selected[i].1)),
                SynCol::ParentStart => ints(&|i| node(parents[i]).map(|x| f.start[x as usize])),
                SynCol::ParentEnd => ints(&|i| node(parents[i]).map(|x| f.end[x as usize])),
                SynCol::ParentKind => Rc::new(Column::Sym(parents.iter().map(|&x| node(x).map_or(facts::NONE, |x| f.kind[x as usize])).collect())),
                SynCol::Field(fi, part) => {
                    let ch = &field_children[fi];
                    match part {
                        0 => ints(&|i| node(ch[i])),
                        1 => ints(&|i| node(ch[i]).map(|c| f.start[c as usize])),
                        2 => ints(&|i| node(ch[i]).map(|c| f.end[c as usize])),
                        3 => Rc::new(Column::Sym(ch.iter().map(|&c| node(c).map_or(facts::NONE, |c| f.kind[c as usize])).collect())),
                        _ => Rc::new(Column::text(source.clone(), ch.iter().map(|&c| node(c).map_or(TextCell::Val(V::Null), span)).collect())),
                    }
                }
            })
            .collect();
        let mut out = vec![(self.name.clone(), Table::new(schemas[0].1.clone(), node_cols, selected.len()))];
        let mut si = 1;
        if let Some(e) = &self.edges {
            let rows: Vec<(u32, u32, u32)> = selected.iter().zip(&parents).filter(|(_, p)| **p != facts::NONE).map(|(&(id, ord), &par)| (par, id, ord)).collect();
            let cols = vec![
                path_col(),
                Rc::new(Column::U32(rows.iter().map(|r| r.0).collect())),
                Rc::new(Column::U32(rows.iter().map(|r| r.1).collect())),
                Rc::new(Column::U32(rows.iter().map(|r| r.2).collect())),
            ];
            out.push((e.clone(), Table::new(schemas[si].1.clone(), cols, rows.len())));
            si += 1;
        }
        if let Some(o) = &self.ownership {
            // Owner values over all named rows (excluded kinds included), in preorder.
            let mut cols = vec![path_col(), Rc::new(Column::U32(selected.iter().map(|&(id, _)| id).collect()))];
            for owner in self.owners.values() {
                let ok = facts::intern(&owner.kind);
                let mut v = vec![facts::NONE; n];
                for id in 0..n as u32 {
                    if !f.named(id) {
                        continue;
                    }
                    let pv = f.parent(id).filter(|&par| f.named(par)).map(|par| v[par as usize]).filter(|&x| x != facts::NONE);
                    let own = (f.kind[id as usize] == ok).then_some(id);
                    let val = if owner.outermost { pv.or(own) } else { own.or(pv) };
                    v[id as usize] = val.unwrap_or(facts::NONE);
                }
                cols.push(Rc::new(Column::U32(selected.iter().map(|&(id, _)| if v[id as usize] == facts::NONE { U32_NULL } else { v[id as usize] }).collect())));
            }
            out.push((o.clone(), Table::new(schemas[si].1.clone(), cols, selected.len())));
        }
        out
    }

    /// `Facts::span`: a named, non-excluded node with exactly this range.
    pub fn has_span(&self, f: &Facts, start: u32, end: u32) -> bool {
        (0..f.len() as u32).any(|id| f.named(id) && f.start[id as usize] == start && f.end[id as usize] == end && !self.exclude.contains(&f.kind[id as usize]))
    }
}

pub enum StagePlan {
    Query { name: String, stmt: Statement, deferred: Vec<(usize, PE, Statement)>, schema: Schema },
    Reach { name: String, roots: Statement, edges: Option<Statement>, forest: bool, syntax: bool },
}

pub struct RelPlan {
    pub inputs: Vec<(String, usize)>,
    pub input_schemas: Vec<Schema>,
    pub syntax: Option<SyntaxSpec>,
    pub stages: Vec<StagePlan>,
    pub fin: Statement,
    pub key: Vec<String>,
    pub code: Option<String>,
    pub per_file: bool,
    pub locations: BTreeSet<String>,
    /// Spike A3: per stage, its fixed substitution (`subst::plan_clause`), if its shape has one.
    pub kernels: Vec<Option<crate::subst::Kernel>>,
    /// Spike A3: the node-dumping inputs produced in one pass over the facts.
    pub dump: Option<crate::subst::DumpGroup>,
    /// Per stage, the stored tables (lowercase) whose last reader it is: dropped, with their
    /// clause-level indexes, once it has run (`schedule_releases`).
    pub release: Vec<Vec<String>>,
}

impl RelPlan {
    pub fn compile(rel: &Json, inputs: Vec<(String, usize, Vec<String>)>, grammar: &rerail_engine::code_rule_dsl::tape::Grammar) -> Result<Self, String> {
        let o = rel.as_object().ok_or("relational")?;
        if o.contains_key("references") {
            return Err("references unsupported".into());
        }
        let per_file = matches!(o.get("partitionBy").and_then(Json::as_str), Some("file"));
        let mut catalog = Catalog::new();
        let mut input_schemas = Vec::new();
        let mut ins = Vec::new();
        for (name, idx, fields) in inputs {
            let s = input_schema(&fields);
            catalog.insert(name.to_ascii_lowercase(), s.clone());
            input_schemas.push(s);
            ins.push((name, idx));
        }
        let syntax = o.get("syntax").map(|s| SyntaxSpec::parse(s, grammar)).transpose()?;
        if let Some(s) = &syntax {
            for (n, sc) in s.schemas() {
                catalog.insert(n.to_ascii_lowercase(), sc);
            }
        }
        let mut stages = Vec::new();
        for st in o.get("stages").and_then(Json::as_array).into_iter().flatten() {
            let name = st["name"].as_str().ok_or("stage name")?.to_owned();
            if let Some(r) = st.get("reachability") {
                let roots = engine::plan(r["roots"].as_str().ok_or("roots")?, &catalog).map_err(|e| format!("stage {name} roots: {e}"))?;
                let edges = r.get("edges").and_then(Json::as_str).map(|e| engine::plan(e, &catalog)).transpose().map_err(|e| format!("stage {name} edges: {e}"))?;
                let forest = r.get("graph").and_then(Json::as_str) == Some("forest");
                let syntax_reach = r.get("syntax").is_some();
                catalog.insert(name.to_ascii_lowercase(), Schema { cols: vec!["__path".into(), "root".into(), "node".into()], affs: vec![Aff::Text, Aff::None, Aff::None] });
                stages.push(StagePlan::Reach { name, roots, edges, forest, syntax: syntax_reach });
                continue;
            }
            let q = st["query"].as_str().ok_or("stage query")?;
            let stmt = engine::plan(q, &catalog).map_err(|e| format!("stage {name}: {e}"))?;
            let mut schema = stmt.schema().clone();
            // CTAS declared types (TEXT/INT/NUM/REAL/none) from expression affinity.
            let mut deferred = Vec::new();
            if let Some(d) = st.get("deferred").and_then(Json::as_object) {
                let stored = Schema {
                    cols: schema.cols.iter().filter(|c| !d.contains_key(*c)).cloned().collect(),
                    affs: schema.cols.iter().zip(&schema.affs).filter(|(c, _)| !d.contains_key(*c)).map(|(_, a)| *a).collect(),
                };
                for (col, expr) in d {
                    let (pe, s) = engine::plan_scalar(expr.as_str().ok_or("deferred")?, &stored).map_err(|e| format!("stage {name} deferred {col}: {e}"))?;
                    let pos = match schema.cols.iter().position(|c| c == col) {
                        Some(p) => p,
                        None => {
                            schema.cols.push(col.clone());
                            schema.affs.push(Aff::None);
                            schema.cols.len() - 1
                        }
                    };
                    schema.affs[pos] = Aff::None;
                    deferred.push((pos, pe, s));
                }
                // Deferred expressions read stored columns by their stored positions.
                let stored_pos: Vec<usize> = schema.cols.iter().enumerate().filter(|(_, c)| !d.contains_key(*c)).map(|(i, _)| i).collect();
                let _ = stored_pos;
            }
            catalog.insert(name.to_ascii_lowercase(), schema.clone());
            stages.push(StagePlan::Query { name, stmt, deferred, schema });
        }
        let fin = engine::plan(o["query"].as_str().ok_or("final query")?, &catalog).map_err(|e| format!("final query: {e}"))?;
        for required in ["path", "start_byte", "end_byte"] {
            if !fin.schema().cols.iter().any(|c| c == required) {
                return Err(format!("final query must select {required}"));
            }
        }
        let key = o.get("key").and_then(Json::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        let code = o.get("code").and_then(Json::as_str).map(str::to_owned);
        let locations = o.get("locations").and_then(Json::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        Ok(Self { inputs: ins, input_schemas, syntax, stages, fin, key, code, per_file, locations, kernels: Vec::new(), dump: None, release: Vec::new() })
    }

    /// Computes `release` from every statement's reads (stages, deferred columns, kernels); the
    /// final query's tables are dropped with the clause. A table no stage reads goes at the
    /// stage that produces it (inputs: the first stage).
    pub fn schedule_releases(&mut self) {
        let n = self.stages.len();
        let mut last: BTreeMap<String, usize> = BTreeMap::new();
        for (name, _) in &self.inputs {
            last.insert(name.to_ascii_lowercase(), 0);
        }
        if let Some(s) = &self.syntax {
            for (name, _) in s.schemas() {
                last.insert(name.to_ascii_lowercase(), 0);
            }
        }
        for (i, st) in self.stages.iter().enumerate() {
            let mut reads: Vec<String> = match st {
                StagePlan::Query { stmt, deferred, .. } => stmt.reads.iter().chain(deferred.iter().flat_map(|(_, _, s)| s.reads.iter())).cloned().collect(),
                StagePlan::Reach { roots, edges, .. } => roots.reads.iter().chain(edges.iter().flat_map(|e| e.reads.iter())).cloned().collect(),
            };
            if let Some(k) = self.kernels.get(i).and_then(Option::as_ref) {
                reads.extend(crate::subst::kernel_reads(k));
            }
            for r in reads {
                last.insert(r.to_ascii_lowercase(), i);
            }
            let (StagePlan::Query { name, .. } | StagePlan::Reach { name, .. }) = st;
            last.entry(name.to_ascii_lowercase()).or_insert(i);
        }
        for r in &self.fin.reads {
            last.insert(r.to_ascii_lowercase(), n);
        }
        self.release = vec![Vec::new(); n];
        for (name, i) in last {
            if i < n {
                self.release[i].push(name);
            }
        }
    }
}

/// One partition's evaluation context.
pub struct Partition<'a> {
    /// Input tables in `plan.inputs` order.
    pub inputs: Vec<Rc<Table>>,
    /// Anchor spans: (path, start, end) -> line (0-based).
    pub anchors: rustc_hash::FxHashMap<(Rc<str>, u32, u32), u32>,
    /// Per-file facts for syntax tables and syntax-span anchoring (per-file partitions).
    pub facts: Option<(&'a Facts, &'a str)>,
    /// Paths whose rows feed the syntax spec (gate witnesses).
    pub source: &'a str,
    pub path: &'a str,
}

pub struct RelOutcome {
    pub findings: Vec<(String, u32, String)>,
    pub failure: Option<String>,
    pub stage_ns: Vec<(String, u64)>,
}

fn store(t: Table, schema: &Schema) -> Table {
    // CTAS converts stored values by each column's declared affinity (on read).
    let len = t.len();
    let cols = t.cols.iter().take(schema.cols.len()).zip(&schema.affs).map(|(c, a)| engine::Column::with_aff(c, *a)).collect();
    Table::new(schema.clone(), cols, len)
}

/// `FACTS_MEMTRACE=1`: per stored stage, its rows, the bytes its columns hold themselves
/// (values, text payloads, gather indexes), the clause's stored tables and indexes so far, the
/// process's current RSS and live heap, and this thread's heap peak since the previous trace.
fn memtrace(name: &str, t: &Table, db: &Db<'_>) {
    if std::env::var_os("FACTS_MEMTRACE").is_none() {
        return;
    }
    let (tables, indexes) = db.bytes();
    eprintln!(
        "memtrace {name}: {} rows x {} cols ~{} MB, db tables {} MB indexes {} MB, rss {} MB, heap {} MB, peak since last trace {} MB",
        t.len(),
        t.schema.cols.len(),
        t.bytes() >> 20,
        tables >> 20,
        indexes >> 20,
        crate::repo::cur_rss() >> 20,
        crate::repo::heap_in_use() >> 20,
        crate::repo::thread_peak() >> 20
    );
    if std::env::var("FACTS_MEMTRACE").is_ok_and(|v| v == "2") {
        for (n, t) in &db.tables {
            for (c, col) in t.schema.cols.iter().zip(&t.cols) {
                if col.bytes() >= 1 << 20 {
                    eprintln!("    {n}.{c}: {} MB", col.bytes() >> 20);
                }
            }
        }
    }
    crate::repo::thread_peak_reset();
}

/// One `run`'s tables for `facts-a stage-check` (the gate's `a-vs-reference`): every input and
/// syntax table the clause read, then every stage it stored, by lower-case name, in order; `None`
/// for a stage a kernel absorbed (never stored). A failing stage and everything after it are absent.
pub type Traced = Vec<(String, Option<Rc<Table>>)>;

type TraceHook = Box<dyn FnMut(&RelPlan, &str, &Traced)>;

thread_local! {
    /// Set by `stage-check` on each thread that evaluates clauses; `None` (the default) traces nothing.
    pub static TRACE: std::cell::RefCell<Option<TraceHook>> = std::cell::RefCell::new(None);
}

/// Hands the traced tables to the hook when `run` returns, on every path.
struct TraceGuard<'p> {
    plan: &'p RelPlan,
    path: String,
    tables: Option<Traced>,
}

impl Drop for TraceGuard<'_> {
    fn drop(&mut self) {
        if let Some(t) = self.tables.take() {
            TRACE.with(|h| {
                if let Some(h) = h.borrow_mut().as_mut() {
                    h(self.plan, &self.path, &t);
                }
            });
        }
    }
}

impl TraceGuard<'_> {
    fn push(&mut self, name: &str, t: Option<&Rc<Table>>) {
        if let Some(v) = self.tables.as_mut() {
            v.push((name.to_ascii_lowercase(), t.cloned()));
        }
    }
}

pub fn run(plan: &RelPlan, part: Partition<'_>) -> RelOutcome {
    let mut out = RelOutcome { findings: Vec::new(), failure: None, stage_ns: Vec::new() };
    let tracing = TRACE.with(|h| h.borrow().is_some());
    let mut trace = TraceGuard { plan, path: part.path.to_owned(), tables: tracing.then(Vec::new) };
    let mut db = Db::new(part.source, part.path);
    for ((name, _), t) in plan.inputs.iter().zip(part.inputs) {
        trace.push(name, Some(&t));
        db.tables.insert(name.to_ascii_lowercase(), t);
    }
    let mut syntax_tables_present = false;
    if let Some(spec) = &plan.syntax {
        // Syntax rows exist for files the gate input has witnesses in.
        let gate_has_rows = db.tables.get(&spec.gate.to_ascii_lowercase()).is_some_and(|t| !t.is_empty());
        match (part.facts, gate_has_rows) {
            (Some((f, path)), true) => {
                if f.syntax_errors {
                    out.failure = Some("syntax source is incomplete".into());
                    return out;
                }
                for (n, t) in spec.tables(f, path) {
                    let t = Rc::new(t);
                    trace.push(&n, Some(&t));
                    db.tables.insert(n.to_ascii_lowercase(), t);
                }
                syntax_tables_present = true;
            }
            _ => {
                for (n, s) in spec.schemas() {
                    let t = Rc::new(Table::empty(s));
                    trace.push(&n, Some(&t));
                    db.tables.insert(n.to_ascii_lowercase(), t);
                }
            }
        }
    }
    let check = crate::subst::check_mode();
    for (si, st) in plan.stages.iter().enumerate() {
        let t0 = std::time::Instant::now();
        let kernel = plan.kernels.get(si).and_then(Option::as_ref);
        match (st, kernel) {
            (StagePlan::Query { name, .. }, Some(crate::subst::Kernel::Absorbed)) if !check => {
                // Read only by a later kernel, which reads its sources directly.
                trace.push(name, None);
            }
            (StagePlan::Query { name, stmt, schema, .. }, Some(k)) if !matches!(k, crate::subst::Kernel::Absorbed) => {
                let t = match crate::subst::run(k, &db, schema) {
                    Ok(t) => t,
                    Err(e) => {
                        out.failure = Some(format!("relational stage {name}: {e}"));
                        return out;
                    }
                };
                let ns = t0.elapsed().as_nanos() as u64;
                if check {
                    let sql = engine::execute(&db, stmt).map(|s| store(s, stmt.schema()));
                    match sql {
                        Ok(s) => {
                            if let Err(e) = crate::subst::same_rows(&t, &s) {
                                panic!("FACTS_A3_CHECK: stage {name} at {}: {e}", part.path);
                            }
                            if s.schema.cols != t.schema.cols || s.schema.affs != t.schema.affs {
                                panic!("FACTS_A3_CHECK: stage {name}: schema differs");
                            }
                        }
                        Err(e) => panic!("FACTS_A3_CHECK: stage {name}: SQL failed ({e}), kernel succeeded"),
                    }
                }
                memtrace(name, &t, &db);
                let t = Rc::new(t);
                trace.push(name, Some(&t));
                db.tables.insert(name.to_ascii_lowercase(), t);
                out.stage_ns.push((format!("view:{name}"), ns));
            }
            (StagePlan::Query { name, stmt, deferred, schema }, _) => {
                let t = match engine::execute(&db, stmt) {
                    Ok(t) => t,
                    Err(e) => {
                        out.failure = Some(format!("relational stage {name}: {e}"));
                        return out;
                    }
                };
                let mut t = store(t, stmt.schema());
                if !deferred.is_empty() {
                    // Deferred columns are evaluated over the stored columns.
                    let n = t.len();
                    let null_col = || Rc::new(engine::Column::Vals(vec![V::Null; n]));
                    let col_at = |i: usize| t.cols.get(i).cloned().unwrap_or_else(null_col);
                    let stored_cols: Vec<usize> = (0..schema.cols.len()).filter(|i| !deferred.iter().any(|(p, _, _)| p == i)).collect();
                    let stored = Table::new(Schema::default(), stored_cols.iter().map(|&i| col_at(i)).collect(), n);
                    let mut cols: Vec<Rc<engine::Column>> = (0..schema.cols.len()).map(col_at).collect();
                    for (pos, pe, s) in deferred {
                        let vals = match engine::eval_scalar_rows(&db, s, pe, &stored) {
                            Ok(v) => v,
                            Err(e) => {
                                out.failure = Some(format!("relational stage {name} deferred: {e}"));
                                return out;
                            }
                        };
                        cols[*pos] = Rc::new(engine::Column::Vals(vals));
                    }
                    t = Table::new(schema.clone(), cols, n);
                }
                if std::env::var_os("FACTS_DEBUG").is_some() {
                    eprintln!("stage {name}: {} rows {:?}", t.len(), t.schema.cols);
                    for r in 0..t.len().min(4) {
                        eprintln!("    {:?}", t.row(r));
                    }
                }
                memtrace(&name, &t, &db);
                let t = Rc::new(t);
                trace.push(name, Some(&t));
                db.tables.insert(name.to_ascii_lowercase(), t);
                out.stage_ns.push((name.clone(), t0.elapsed().as_nanos() as u64));
            }
            (StagePlan::Reach { name, roots, edges, forest, syntax }, _) => {
                let r = match reach(&db, roots, edges.as_ref(), *forest, if *syntax { part.facts.filter(|_| syntax_tables_present) } else { None }, plan.syntax.as_ref(), *syntax) {
                    Ok(t) => t,
                    Err(e) => {
                        out.failure = Some(format!("relational stage {name}: {e}"));
                        return out;
                    }
                };
                let r = Rc::new(r);
                trace.push(name, Some(&r));
                db.tables.insert(name.to_ascii_lowercase(), r);
                out.stage_ns.push((name.clone(), t0.elapsed().as_nanos() as u64));
            }
        }
        for name in plan.release.get(si).into_iter().flatten() {
            db.release(name);
        }
    }
    let t0 = std::time::Instant::now();
    let fin = match engine::execute(&db, &plan.fin) {
        Ok(t) => t,
        Err(e) => {
            out.failure = Some(format!("relational query: {e}"));
            return out;
        }
    };
    out.stage_ns.push(("<final>".into(), t0.elapsed().as_nanos() as u64));
    let sch = fin.schema.clone();
    let ci = |n: &str| sch.cols.iter().position(|c| c == n);
    let (pi, si, ei) = (ci("path").unwrap(), ci("start_byte").unwrap(), ci("end_byte").unwrap());
    let code_i = plan.code.as_ref().and_then(|c| ci(c));
    let key_i: Vec<usize> = plan.key.iter().filter_map(|k| ci(k)).collect();
    let mut seen = std::collections::HashSet::new();
    for row in (0..fin.len()).map(|i| fin.row(i)) {
        let V::Text(path) = &row[pi] else {
            out.failure = Some("relational path must be text".into());
            out.findings.clear();
            return out;
        };
        let (V::Int(s), V::Int(e)) = (&row[si], &row[ei]) else {
            out.failure = Some("relational start_byte must be a source byte offset".into());
            out.findings.clear();
            return out;
        };
        if *s < 0 || *e < 0 {
            out.failure = Some("relational start_byte must be a source byte offset".into());
            out.findings.clear();
            return out;
        }
        let (s, e) = (*s as u32, *e as u32);
        let line = if let Some(l) = part.anchors.get(&(path.clone(), s, e)) {
            Some(*l)
        } else if let (Some((f, fp)), Some(spec), true) = (part.facts, plan.syntax.as_ref(), syntax_tables_present) {
            (fp == &**path && spec.has_span(f, s, e)).then(|| f.lines.line(s as usize) as u32)
        } else {
            None
        };
        let line = match line {
            Some(l) => l,
            None if s == 0 && e == 0 && plan.locations.contains(&**path) => 0,
            None => {
                out.failure = Some(format!("relational results must anchor to an input witness span ({path} {s}..{e})"));
                out.findings.clear();
                return out;
            }
        };
        let code = match code_i {
            None => None,
            Some(i) => match &row[i] {
                V::Text(c) if !c.is_empty() && c.len() <= 256 => Some(c.to_string()),
                _ => {
                    out.failure = Some("relational code must be nonempty text of at most 256 bytes".into());
                    out.findings.clear();
                    return out;
                }
            },
        };
        let identity: Vec<String> = key_i.iter().map(|&i| format!("{:?}", sv::dkey(&row[i]))).collect();
        if !seen.insert((code.clone(), path.to_string(), s, e, identity)) {
            out.failure = Some("relational results have duplicate identities".into());
            out.findings.clear();
            return out;
        }
        out.findings.push((path.to_string(), line, code.unwrap_or_default()));
    }
    out
}

fn reach(db: &Db<'_>, roots: &Statement, edges: Option<&Statement>, forest: bool, facts_ref: Option<(&Facts, &str)>, spec: Option<&SyntaxSpec>, syntax: bool) -> Result<Table, String> {
    let schema = Schema { cols: vec!["__path".into(), "root".into(), "node".into()], affs: vec![Aff::Text, Aff::None, Aff::None] };
    let rt = engine::execute(db, roots)?;
    let mut root_keys: Vec<(V, V)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in (0..rt.len()).map(|i| rt.row(i)) {
        if r[0].is_null() || r[1].is_null() {
            continue;
        }
        if seen.insert((sv::dkey(&r[0]), sv::dkey(&r[1]))) {
            root_keys.push((r[0].clone(), r[1].clone()));
        }
    }
    let mut rows = Vec::new();
    if syntax {
        let spec = spec.ok_or("syntax reachability without a syntax spec")?;
        let Some((f, path)) = facts_ref else {
            if root_keys.is_empty() {
                return Ok(Table::from_rows(schema, rows));
            }
            return Err("syntax root is outside the eligible files".into());
        };
        // Interval membership over the facts: per root, the eligible (named, not excluded) nodes
        // of `[id, subtree_end)` in preorder; nothing per row is stored.
        let eligible: Vec<u32> = (0..f.len() as u32).filter(|&n| f.named(n) && !spec.exclude.contains(&f.kind[n as usize])).collect();
        let (mut roots, mut lo, mut offs) = (Vec::with_capacity(root_keys.len()), Vec::with_capacity(root_keys.len()), Vec::with_capacity(root_keys.len()));
        let mut total = 0u32;
        for (p, id) in &root_keys {
            let V::Text(ps) = p else { return Err("syntax root path".into()) };
            if &**ps != path {
                return Err("syntax root is outside the eligible files".into());
            }
            let V::Int(id) = id else { return Err("syntax root must be an integer".into()) };
            let id = *id as u32;
            if id as usize >= f.len() || !f.named(id) {
                return Err("syntax root is not an eligible node".into());
            }
            let (a, b) = (eligible.partition_point(|&n| n < id), eligible.partition_point(|&n| n < f.subtree_end[id as usize]));
            roots.push(id);
            lo.push(a as u32);
            offs.push(total);
            total += (b - a) as u32;
        }
        let (roots, lo, offs, eligible): (Rc<[u32]>, Rc<[u32]>, Rc<[u32]>, Rc<[u32]>) = (roots.into(), lo.into(), offs.into(), eligible.into());
        let interval = |node| Rc::new(engine::Column::Interval { roots: roots.clone(), lo: lo.clone(), offs: offs.clone(), eligible: eligible.clone(), node });
        let cols = vec![Rc::new(engine::Column::Const(V::text(path))), interval(false), interval(true)];
        return Ok(Table::new(schema, cols, total as usize));
    }
    let et = engine::execute(db, edges.ok_or("reachability edges")?)?;
    // Adjacency by (path, src) over distinct edges; keys compare by storage class (affinity
    // stripped).
    let mut adj: rustc_hash::FxHashMap<(sv::DKey, sv::DKey), Vec<V>> = Default::default();
    let mut eseen = std::collections::HashSet::new();
    let mut parents: rustc_hash::FxHashMap<(sv::DKey, sv::DKey), sv::DKey> = Default::default();
    for r in (0..et.len()).map(|i| et.row(i)) {
        if r[0].is_null() || r[1].is_null() || r[2].is_null() {
            continue;
        }
        let k = (sv::dkey(&r[0]), sv::dkey(&r[1]), sv::dkey(&r[2]));
        if !eseen.insert(k) {
            continue;
        }
        if forest {
            let child = (sv::dkey(&r[0]), sv::dkey(&r[2]));
            if parents.insert(child, sv::dkey(&r[1])).is_some() {
                return Err("forest node has multiple parents".into());
            }
        }
        adj.entry((sv::dkey(&r[0]), sv::dkey(&r[1]))).or_default().push(r[2].clone());
    }
    for (p, root) in &root_keys {
        let pk = sv::dkey(p);
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![root.clone()];
        visited.insert(sv::dkey(root));
        let mut order = Vec::new();
        while let Some(n) = stack.pop() {
            order.push(n.clone());
            if let Some(ch) = adj.get(&(pk.clone(), sv::dkey(&n))) {
                for c in ch {
                    if visited.insert(sv::dkey(c)) {
                        stack.push(c.clone());
                    } else if forest {
                        return Err("forest contains a cycle".into());
                    }
                }
            }
        }
        for n in order {
            rows.push(vec![p.clone(), root.clone(), n].into_boxed_slice());
        }
    }
    Ok(Table::from_rows(schema, rows))
}
