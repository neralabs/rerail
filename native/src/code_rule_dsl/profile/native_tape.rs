//! Isolated profile API over the real native tape. No policy decisions live here.
mod bindings;
use crate::code_rule_dsl::{
    budget,
    json::{invalid, json_error},
    language::GrammarDialect,
    lines::{LineTable, Terminators},
    tape::{Grammar, TapeDoc, TapeImage},
};
use ast_grep_core::source::SgNode;
use napi_derive::napi;
use rerail_reachability_spike::rows::{Charge, Source, register};
use rusqlite::{Connection, Result, functions::FunctionFlags, types::Value, vtab::Context};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::Instant;

struct TapeSource {
    path: String,
    doc: TapeDoc,
    charge: Charge,
    kinds: Mutex<Option<HashMap<String, Vec<usize>>>>,
    owners: Vec<[Option<u32>; 3]>,
    owner_postings: [HashMap<u32, Vec<usize>>; 3],
    owner_work: AtomicU64,
}
impl TapeSource {
    fn new(path: String, source: String, charge: Charge) -> napi::Result<Self> {
        let source = Arc::new(source);
        let grammar = Grammar::new(GrammarDialect::Python).map_err(invalid)?;
        let check = || charge(1).is_err();
        let image = TapeImage::parse(&grammar, &source, &check)
            .ok_or_else(|| invalid("parse interrupted"))?
            .map_err(invalid)?;
        let lines = Arc::new(LineTable::new(&source, Terminators::Newlines));
        let doc =
            TapeDoc::restore_with_lines(source, grammar, &image, lines, &check).map_err(invalid)?;
        let mut owners: Vec<[Option<u32>; 3]> = Vec::new();
        for i in 0..doc.transducer_len() {
            charge(1).map_err(|e| invalid(e.to_string()))?;
            let mut current = doc
                .transducer_parent(i as u32)
                .map_or([None; 3], |p| owners[p as usize]);
            match doc.node(i as u32).kind().as_ref() {
                "function_definition" => {
                    current[0] = Some(i as u32);
                    if current[2].is_none() {
                        current[2] = Some(i as u32);
                    }
                }
                "class_definition" => current[1] = Some(i as u32),
                _ => {}
            }
            owners.push(current);
        }
        let mut owner_postings: [HashMap<u32, Vec<usize>>; 3] =
            std::array::from_fn(|_| HashMap::new());
        for (i, owners) in owners.iter().enumerate() {
            charge(1).map_err(|e| invalid(e.to_string()))?;
            for (family, owner) in owners.iter().enumerate() {
                if let Some(owner) = owner {
                    owner_postings[family].entry(*owner).or_default().push(i);
                }
            }
        }
        Ok(Self {
            owner_postings,
            owner_work: AtomicU64::new(0),
            path,
            doc,
            charge,
            kinds: Mutex::new(None),
            owners,
        })
    }
    fn valid(&self, id: i64) -> Option<u32> {
        u32::try_from(id)
            .ok()
            .filter(|i| (*i as usize) < self.doc.transducer_len())
    }
    fn ancestor(&self, root: i64, node: i64) -> Option<bool> {
        let root = self.valid(root)?;
        let node = self.valid(node)?;
        Some(node >= root && node < self.doc.transducer_descendants(root).end)
    }
}
impl Source for TapeSource {
    fn columns(&self) -> Vec<String> {
        [
            "__path",
            "id",
            "parent",
            "kind",
            "start_byte",
            "end_byte",
            "text",
            "field",
            "named",
            "fn",
            "cls",
            "top_fn",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }
    fn column_types(&self) -> Vec<String> {
        [
            "TEXT", "INTEGER", "INTEGER", "TEXT", "INTEGER", "INTEGER", "TEXT", "INTEGER",
            "INTEGER", "INTEGER", "INTEGER", "INTEGER",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }
    fn len(&self) -> usize {
        self.doc.transducer_len()
    }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let n = match c {
            1 => 1,
            2 => 8,
            3 => self.len().div_ceil(50),
            9..=11 => self.len().div_ceil(self.owner_postings[c - 9].len().max(1)),
            _ => self.len(),
        };
        if batch {
            n.saturating_mul(256).min(self.len())
        } else {
            n.max(1)
        }
    }
    fn indexed(&self, c: usize) -> bool {
        matches!(c, 0 | 1 | 2 | 3 | 9 | 10 | 11)
    }
    fn charge(&self, count: usize) -> Result<()> {
        (self.charge)(count)
    }
    fn lookup(&self, c: usize, values: &[Value]) -> Result<Option<Vec<usize>>> {
        let mut rows = Vec::new();
        if matches!(c, 0 | 3)
            && values
                .iter()
                .any(|v| !matches!(v, Value::Text(_) | Value::Null))
        {
            return Ok(None);
        }
        if !matches!(c, 0 | 3)
            && values
                .iter()
                .any(|v| !matches!(v, Value::Integer(_) | Value::Null))
        {
            return Ok(None);
        }
        match c {
            0 => {
                if values.iter().any(|v| v == &Value::Text(self.path.clone())) {
                    return Ok(None);
                }
            }
            1 => {
                for v in values {
                    if let Value::Integer(id) = v
                        && let Some(id) = self.valid(*id)
                    {
                        rows.push(id as usize);
                    }
                }
            }
            3 => {
                let mut kinds = self
                    .kinds
                    .lock()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                if kinds.is_none() {
                    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
                    for i in 0..self.len() {
                        self.charge(1)?;
                        map.entry(self.doc.node(i as u32).kind().into_owned())
                            .or_default()
                            .push(i);
                    }
                    *kinds = Some(map);
                }
                for v in values {
                    if let Value::Text(kind) = v
                        && let Some(ids) = kinds.as_ref().unwrap().get(kind)
                    {
                        rows.extend_from_slice(ids);
                    }
                }
            }
            2 => {
                for v in values {
                    if let Value::Integer(id) = v
                        && let Some(id) = self.valid(*id)
                    {
                        rows.extend(self.doc.transducer_children(id).map(|i| i as usize));
                    }
                }
            }
            9..=11 => {
                for value in values {
                    self.charge(1)?;
                    self.owner_work.fetch_add(1, Ordering::Relaxed);
                    if let Value::Integer(id) = value
                        && let Some(id) = self.valid(*id)
                        && let Some(postings) = self.owner_postings[c - 9].get(&id)
                    {
                        self.charge(postings.len())?;
                        self.owner_work
                            .fetch_add(postings.len() as u64, Ordering::Relaxed);
                        rows.extend_from_slice(postings);
                    }
                }
            }
            _ => return Ok(None),
        }
        rows.sort_unstable();
        rows.dedup();
        Ok(Some(rows))
    }
    fn column(&self, row: usize, col: usize, ctx: &mut Context) -> Result<()> {
        let id = row as u32;
        let node = self.doc.node(id);
        match col {
            0 => ctx.set_result(&self.path),
            1 => ctx.set_result(&(row as i64)),
            2 => ctx.set_result(&self.doc.transducer_parent(id).map(i64::from)),
            3 => ctx.set_result(&node.kind().as_ref()),
            4 => ctx.set_result(&(node.range().start as i64)),
            5 => ctx.set_result(&(node.range().end as i64)),
            6 => ctx.set_result(&self.doc.transducer_text_str(id)),
            7 => ctx.set_result(&i64::from(self.doc.transducer_field(id))),
            8 => ctx.set_result(&i64::from(self.doc.transducer_flags(id) & 1)),
            9..=11 => ctx.set_result(&self.owners[row][col - 9].map(i64::from)),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

/// Experimental snapshot object: compile once, then compare SQL access strategies.
#[napi]
pub struct NativeSqlSpike {
    db: Connection,
    source: Arc<TapeSource>,
    stats: Arc<rerail_reachability_spike::rows::Counters>,
    build_ms: f64,
}
#[napi]
impl NativeSqlSpike {
    #[napi(constructor)]
    pub fn new(path: String, source: String, backend: String) -> napi::Result<Self> {
        let start = Instant::now();
        let work = Arc::new(AtomicUsize::new(200_000_000));
        let charge = super::native_rows::charge(Arc::clone(&work));
        let source = Arc::new(TapeSource::new(path, source, Arc::clone(&charge))?);
        let db = Connection::open_in_memory().map_err(|e| invalid(e.to_string()))?;
        db.progress_handler(
            1000,
            Some(move || {
                work.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                    v.checked_sub(1000)
                })
                .is_err()
                    || budget::step_budget_exhausted()
            }),
        )
        .map_err(|e| invalid(e.to_string()))?;
        db.execute_batch("PRAGMA temp_store=MEMORY;PRAGMA analysis_limit=1000;")
            .map_err(|e| invalid(e.to_string()))?;
        let stats =
            register(&db, "native_nodes", source.clone()).map_err(|e| invalid(e.to_string()))?;
        let shared = Arc::clone(&source);
        db.create_scalar_function(c"is_ancestor", 2, FunctionFlags::SQLITE_UTF8, move |c| {
            shared.charge(1)?;
            Ok(shared.ancestor(c.get(0)?, c.get(1)?).map(i64::from))
        })
        .map_err(|e| invalid(e.to_string()))?;
        let shared = Arc::clone(&source);
        db.create_scalar_function(c"node_text", 1, FunctionFlags::SQLITE_UTF8, move |c| {
            shared.charge(1)?;
            Ok(shared
                .valid(c.get(0)?)
                .map(|i| shared.doc.transducer_text_str(i).to_owned()))
        })
        .map_err(|e| invalid(e.to_string()))?;
        let shared = Arc::clone(&source);
        db.create_scalar_function(c"owner", 1, FunctionFlags::SQLITE_UTF8, move |c| {
            shared.charge(1)?;
            Ok(shared
                .valid(c.get(0)?)
                .and_then(|i| shared.owners[i as usize][0])
                .map(i64::from))
        })
        .map_err(|e| invalid(e.to_string()))?;
        let registry =
            rerail_reachability_spike::register_graph(&db).map_err(|e| invalid(e.to_string()))?;
        rerail_reachability_spike::register_flow(&db).map_err(|e| invalid(e.to_string()))?;
        let tree = Arc::clone(&source);
        let handle = std::sync::OnceLock::<i64>::new();
        db.create_scalar_function(c"tree_handle", 0, FunctionFlags::SQLITE_UTF8, move |_| {
            if let Some(id) = handle.get() {
                return Ok(*id);
            }
            let nodes = (0..tree.len() as i64).collect::<Vec<_>>();
            let mut edges = Vec::new();
            for id in 0..tree.len() {
                tree.charge(1)?;
                if let Some(p) = tree.doc.transducer_parent(id as u32) {
                    edges.push([i64::from(p), id as i64]);
                }
            }
            let graph = rerail_reachability_spike::graph::Graph::new_tree(
                nodes,
                edges,
                200_000_000,
                100_000,
            )?;
            let mut registry = registry.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            registry.next += 1;
            let id = registry.next;
            registry.graphs.insert(id, Arc::new(Mutex::new(graph)));
            let _ = handle.set(id);
            Ok(id)
        })
        .map_err(|e| invalid(e.to_string()))?;
        bindings::register(&db, Arc::clone(&source)).map_err(|e| invalid(e.to_string()))?;
        match backend.as_str() {
            "vtab"=>db.execute_batch("CREATE TEMP VIEW nodes AS SELECT * FROM native_nodes"),
            "materialized"=>db.execute_batch("CREATE TEMP TABLE nodes AS SELECT * FROM native_nodes;CREATE INDEX nodes_kind ON nodes(kind);CREATE INDEX nodes_parent ON nodes(parent);CREATE UNIQUE INDEX nodes_id ON nodes(id);ANALYZE nodes;"),
            "narrow"=>db.execute_batch("CREATE TEMP TABLE nodes AS SELECT __path,id,parent,kind,start_byte,end_byte,named,fn,cls,top_fn FROM native_nodes;CREATE INDEX nodes_kind ON nodes(kind);CREATE INDEX nodes_parent ON nodes(parent);CREATE UNIQUE INDEX nodes_id ON nodes(id);ANALYZE nodes;"),
            _=>return Err(invalid("backend must be vtab, materialized, or narrow")),
        }.map_err(|e|invalid(e.to_string()))?;
        Ok(Self {
            db,
            source,
            stats,
            build_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }
    #[napi]
    pub fn query(&self, sql: String) -> napi::Result<String> {
        let started = Instant::now();
        let mut statement = self.db.prepare(&sql).map_err(|e| invalid(e.to_string()))?;
        if !statement.readonly() {
            return Err(invalid("spike queries must be read-only"));
        }
        let count = statement.column_count();
        let mut query = statement.query([]).map_err(|e| invalid(e.to_string()))?;
        let mut rows = Vec::new();
        while let Some(row) = query.next().map_err(|e| invalid(e.to_string()))? {
            let mut cells = Vec::new();
            for i in 0..count {
                cells.push(match row.get_ref(i).map_err(|e| invalid(e.to_string()))? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(v) => json!(v),
                    rusqlite::types::ValueRef::Real(v) => json!(v),
                    rusqlite::types::ValueRef::Text(v) => {
                        json!(std::str::from_utf8(v).map_err(|e| invalid(e.to_string()))?)
                    }
                    _ => return Err(invalid("non-scalar spike result")),
                });
            }
            rows.push(cells);
        }
        serde_json::to_string(&json!({"rows":rows,"queryMs":started.elapsed().as_secs_f64()*1000.0,"buildMs":self.build_ms,"nodes":self.source.len(),"stats":self.stats.json(),"ownerLookupWork":self.source.owner_work.load(Ordering::Relaxed),"ownerIndexEntries":self.source.owner_postings.iter().flat_map(|m|m.values()).map(Vec::len).sum::<usize>()})).map_err(json_error)
    }
}
