//! SQL extension sessions over the content store. Fact/rule programs use the existing typed
//! SQL emitter and wave scheduler; AST matching remains a separate native fact producer.

use super::facts::core::{self, Built};
use super::facts::vtab::{Relation, Source, syntax};
use super::logic::sql::select;
use super::logic::{front, plan, rr, strings};
use super::waves::{owner::Database, schedule};
use crate::content_store::ContentStore;
use napi::{Error, Result};
use napi_derive::napi;
use rusqlite::OptionalExtension;
use rusqlite::types::{Value as SqlValue, ValueRef};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
pub mod catalog;
pub mod compiler;
pub mod explain;
pub mod constants;
// A row of a corpus binding is `PROBE | document << 32 | row`: a 64-bit target only.
#[cfg(target_pointer_width = "64")]
pub mod corpus;
pub mod documents;
pub mod indexes;
pub mod live;
pub mod maintenance;
pub mod overlay;
pub mod phase;
pub mod producers;
pub mod profiling;
pub mod recursive;
pub mod reductions;
pub mod scratch;
pub mod temporal;
pub mod updates;
#[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
pub mod verification;
pub mod versions;
pub mod walks;
use super::logic::sql::lattices;
use super::logic::sql::program;
pub mod runtime;
pub mod session;
pub mod wire;

fn error(e: impl std::fmt::Display) -> Error {
    Error::from_reason(e.to_string())
}

fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 120
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// An explicit SQL execution context. The persistent content store is `main`; bound fact
/// virtual tables and derived relations are local to this connection. No per-node SQL tables
/// are populated. Results survive until their inputs change, their outputs are replaced, or
/// the session closes. Independent results survive other input changes.
#[napi]
pub struct CodeRuleSqlExecutor {
    #[napi(skip)] pub db: Database,
    #[napi(skip)] pub store: ContentStore,
    #[napi(skip)] pub inputs: BTreeMap<String, Arc<dyn Source>>,
    #[napi(skip)] pub generated: Vec<(String, bool)>,
    #[napi(skip)] pub executions: Vec<session::Execution>,
    limits: runtime::Limits,
    lattice_extensions: BTreeMap<String, schedule::LatticeOperations>,
    input_shapes: BTreeMap<String, (usize, Option<Vec<String>>)>,
    mutable_inputs: BTreeSet<String>,
    #[napi(skip)] pub live_inputs: BTreeMap<String, Arc<live::Live>>,
    #[napi(skip)] pub documents: Option<documents::Documents>,
    #[napi(skip)] pub scratch: scratch::Pool,
    #[napi(skip)] pub versions: versions::Store,
    next_trace: usize,
    stats_ready: std::cell::Cell<bool>,
    #[napi(skip)] pub statistics: session::Statistics,
    catalog_dirty: std::cell::Cell<bool>,
    control_ready: std::cell::Cell<bool>,
    #[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
    #[napi(skip)] pub cold_oracle: bool,
}

#[napi]
impl CodeRuleSqlExecutor {
    #[napi(constructor)]
    pub fn new(store_path: String) -> Result<Self> {
        let store = ContentStore::open(Path::new(&store_path)).map_err(error)?;
        let db = Database::open(&store).map_err(error)?;
        super::logic::native::register(&db).map_err(error)?;
        strings::register(&db).map_err(error)?;
        wire::register(&db)?;
        lattices::builtins(&db)?;
        reductions::register(&db)?;
        Ok(Self {
            db,
            store,
            inputs: BTreeMap::new(),
            generated: Vec::new(),
            executions: Vec::new(),
            limits: runtime::Limits::default(),
            lattice_extensions: BTreeMap::new(),
            input_shapes: BTreeMap::new(),
            mutable_inputs: BTreeSet::new(),
            live_inputs: BTreeMap::new(),
            documents: None,
            scratch: scratch::Pool::default(),
            versions: versions::Store::default(),
            next_trace: 0,
            stats_ready: std::cell::Cell::new(false),
            statistics: session::Statistics::default(),
            catalog_dirty: std::cell::Cell::new(true),
            control_ready: std::cell::Cell::new(false),
            #[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
            cold_oracle: false,
        })
    }

    /// Replace the session's inputs with core fact virtual tables over a repository snapshot.
    /// Build everything before replacing bindings; a malformed/unparsed file fails the call.
    #[napi]
    pub fn bind_snapshot(&mut self, snapshot_json: String) -> Result<()> {
        self.profiled("bind_snapshot", |this| {
            let (docs, sources) = documents::bind(&this.store, &snapshot_json)?;
            this.replace_sources(sources.into_iter().collect())?;
            this.documents = Some(docs);
            Ok(())
        })
    }

    /// Maintain retained SQL results across a complete replacement snapshot. Only changed
    /// files rebuild native facts; additions and removals use the same atomic delta scheduler.
    #[napi]
    pub fn apply_snapshot(
        &mut self,
        snapshot_json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.profiled("apply_snapshot", |this| {
            this.change_snapshot(snapshot_json, max_waves)
        })
    }

    /// Apply an atomic {upsert: [snapshot files], delete: [paths]} batch. Unmentioned files
    /// retain their native facts and metadata without another inventory scan.
    #[napi]
    pub fn apply_file_changes(
        &mut self,
        changes_json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.profiled("apply_file_changes", |this| {
            this.change_files(changes_json, max_waves)
        })
    }

    /// Bind current core facts from the database, without source files or a new snapshot.
    /// With no path list, use every currently bound core document in `scope`.
    #[napi]
    pub fn bind_stored(&mut self, scope: String, paths: Option<Vec<String>>) -> Result<()> {
        self.profiled("bind_stored", |this| {
            let paths = match paths {
                Some(paths) => paths,
                None => {
                    let prefix = format!("{scope}#core:");
                    this.db.prepare("SELECT substr(sym_text(name_atom), length(?1) + 1) FROM name_bindings WHERE substr(sym_text(name_atom), 1, length(?1)) = ?1 AND digest <> zeroblob(32) ORDER BY name_atom").map_err(error)?
                        .query_map([prefix], |r| r.get::<_, String>(0)).map_err(error)?
                        .collect::<rusqlite::Result<Vec<_>>>().map_err(error)?
                }
            };
            if paths.iter().collect::<BTreeSet<_>>().len() != paths.len() {
                return Err(error("stored document paths must be distinct"));
            }
            let documents =
                super::facts::families::stored_documents(&this.store, &scope, &paths).map_err(error)?;
            if documents.len() != paths.len() || documents.iter().any(|d| d.facts.syntax_errors) {
                return Err(error(
                    "requested stored documents have missing or incomplete core facts",
                ));
            }
            let sources = syntax::shaped(&this.store, documents.clone(), &syntax::Projection::core())
                .map_err(error)?;
            syntax::fresh(&this.store, &documents).map_err(error)?;
            this.replace_sources(
                sources
                    .into_iter()
                    .map(|(n, s)| (n.to_owned(), s))
                    .collect(),
            )
        })
    }

    /// Bind integer or symbol facts, including rows emitted by a native AST matcher.
    /// `relation_json` is {name, columns, rows, types?}. Strings become symbol atoms; tagged
    /// decimal integers preserve values outside JavaScript's safe range.
    #[napi]
    pub fn bind_relation(&mut self, relation_json: String) -> Result<()> {
        self.profiled("bind_relation", |this| {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                name: String,
                columns: Vec<String>,
                rows: Vec<Vec<Value>>,
                types: Option<Vec<String>>,
            }
            let input: Input = serde_json::from_str(&relation_json).map_err(error)?;
            let logical_arity = input.columns.len();
            let logical_types = input.types.clone();
            if let Some(types) = &input.types {
                if types.len() != input.columns.len()
                    || types
                        .iter()
                        .any(|t| !matches!(t.as_str(), "number" | "symbol" | "node" | "loc"))
                {
                    return Err(error(
                        "fact types must match columns and be number, symbol, node, or loc",
                    ));
                }
            }
            let boolean = input.columns.is_empty();
            if boolean && input.rows.iter().any(|row| !row.is_empty()) {
                return Err(error("Boolean fact rows must have zero columns"));
            }
            let mut input = input;
            if boolean {
                input.columns.push("__presence".into());
                for row in &mut input.rows {
                    row.push(json!(1));
                }
                input.types = None;
            }
            let columns: BTreeSet<String> = input
                .columns
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if input.columns.is_empty()
                || columns.len() != input.columns.len()
                || !input.columns.iter().all(|c| identifier(c))
            {
                return Err(error("fact columns must be distinct SQL identifiers"));
            }
            if input.rows.iter().any(|r| r.len() != input.columns.len()) {
                return Err(error("a fact row does not have the declared arity"));
            }
            this.check_input(&input.name, &input.columns, logical_arity)?;
            let symbols: BTreeSet<&str> = input
                .rows
                .iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let names: Vec<&str> = symbols.into_iter().collect();
            let atoms = this.store.intern_atoms(&names).map_err(error)?;
            let symbols: BTreeMap<_, _> = names.into_iter().zip(atoms).collect();
            let values = input
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .enumerate()
                        .map(|(i, value)| {
                            wire::fact(&symbols, value, input.types.as_ref().map(|t| t[i].as_str()))
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<BTreeSet<_>>>()?;
            let source = Arc::new(live::Live::from_rows(input.columns, values));
            this.install_input(input.name.clone(), source.clone(), logical_arity, logical_types, true)?;
            this.live_inputs.insert(input.name, source);
            Ok(())
        })
    }

    /// Whether `name` may be bound as a fact relation of these columns.
    fn check_input(&self, name: &str, columns: &[String], logical_arity: usize) -> Result<()> {
        if self.documents.is_some() && self.inputs.contains_key(name) && !self.mutable_inputs.contains(name) {
            return Err(error("native snapshot families cannot be replaced with bindRelation"));
        }
        if self.input_shapes.get(name).is_some_and(|(arity, _)| *arity != logical_arity) {
            return Err(error("rebinding a fact cannot change its logical arity"));
        }
        if !identifier(name) || reserved(name) {
            return Err(error("a fact relation needs a non-reserved SQL identifier"));
        }
        let stored: bool = self
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?1 COLLATE NOCASE)", [name], |r| r.get(0))
            .map_err(error)?;
        if stored || name.eq_ignore_ascii_case("atoms") {
            return Err(error("a fact binding cannot replace a stored relation"));
        }
        if self.inputs.keys().any(|n| n.eq_ignore_ascii_case(name) && n != name) {
            return Err(error("fact names must be unique ignoring case"));
        }
        if self.inputs.get(name).is_some_and(|s| s.columns() != columns) {
            return Err(error("rebinding a fact cannot change its columns"));
        }
        Ok(())
    }

    /// Bind a checked fact relation, invalidating what read the one it replaces. A mutable one takes
    /// `applyChanges`.
    fn install_input(&mut self, name: String, source: Arc<dyn Source>, logical_arity: usize, types: Option<Vec<String>>, mutable: bool) -> Result<()> {
        self.invalidate(&BTreeSet::from([name.clone()]))?;
        self.db.bind(&name, Arc::clone(&source)).map_err(error)?;
        if mutable {
            self.mutable_inputs.insert(name.clone());
        } else {
            self.mutable_inputs.remove(&name);
        }
        self.live_inputs.remove(&name);
        self.input_shapes.insert(name.clone(), (logical_arity, types));
        self.inputs.insert(name, source);
        Ok(())
    }

    /// Bind the relations of the newest corpus run of `root` from what its per-file memo already wrote
    /// (`docs/sql-access/01-design.md` §2, option C): each export's rows and each file-local output's lines, as a
    /// read-only input under its own name and declared columns, plus `corpus_document(doc, path)` from each file's
    /// document key to its stored path. Nothing is evaluated, and frames are decoded per document when a statement
    /// reads them (`corpus`). `relations` limits which are bound; a name the run's memo does not hold is an error.
    #[napi]
    pub fn bind_corpus(&mut self, root: String, relations: Option<Vec<String>>) -> Result<String> {
        #[cfg(not(target_pointer_width = "64"))]
        {
            let _ = (root, relations);
            return Err(error("bindCorpus needs a 64-bit target: its rows are document << 32 | row"));
        }
        #[cfg(target_pointer_width = "64")]
        self.bind_corpus_64(root, relations)
    }

    #[cfg(target_pointer_width = "64")]
    fn bind_corpus_64(&mut self, root: String, relations: Option<Vec<String>>) -> Result<String> {
        let canonical = std::fs::canonicalize(&root).map_or(root.clone(), |p| p.to_string_lossy().into_owned());
        let (recorded, frames) = plan::corpus::memo::read_run(&self.store, &canonical)
            .map_err(error)?
            .ok_or_else(|| error(format!("{root}: no corpus run is recorded for this root")))?;
        let wanted: Vec<String> = match relations {
            Some(names) => {
                if let Some(missing) = names.iter().find(|n| !recorded.contains_key(*n)) {
                    return Err(error(format!("{missing}: not a relation of the run's memo")));
                }
                names
            }
            None => recorded.keys().cloned().collect(),
        };
        let run = Arc::new(corpus::Run::open(frames)?);
        let mut bound = serde_json::Map::new();
        for rel in &wanted {
            // An export's columns, or `{"lines": columns}` for a file-local output's rendered lines.
            let value: Value = serde_json::from_str(&recorded[rel]).map_err(error)?;
            let (form, decl) = match value.get("lines") {
                Some(cols) => (corpus::Form::Lines, cols.clone()),
                None => (corpus::Form::Rows, value),
            };
            let decl: Vec<(String, String)> = serde_json::from_value(decl).map_err(error)?;
            let columns = if decl.is_empty() { vec!["__presence".to_owned()] } else { catalog::view_columns(&decl) };
            self.check_input(rel, &columns, decl.len())?;
            let types = decl.iter().map(|(_, t)| t.clone()).collect();
            let source = corpus::Corpus::new(Arc::clone(&run), rel, columns, types, form, self.store.atoms_handle());
            let bound_types = source.bound_types();
            bound.insert(rel.clone(), json!(if form == corpus::Form::Lines { "lines" } else { "rows" }));
            self.install_input(rel.clone(), Arc::new(source), decl.len(), bound_types, false)?;
        }
        let paths: Vec<&str> = run.documents().map(|(_, p)| p).collect();
        let atoms = self.store.intern_atoms(&paths).map_err(error)?;
        let documents: Vec<Value> = run.documents().zip(atoms).map(|((key, _), path)| json!([key, path])).collect();
        self.bind_relation(
            json!({"name": "corpus_document", "columns": ["doc", "path"], "types": ["number", "symbol"], "rows": documents}).to_string(),
        )?;
        serde_json::to_string(&json!({"files": run.len(), "relations": bound})).map_err(error)
    }

    /// Clear generated results while keeping bound facts and connection-local extensions.
    #[napi]
    pub fn reset(&mut self) -> Result<()> {
        self.profiled("reset", |this| this.clear_results())
    }

    /// Atomically insert/delete fact rows and maintain dependent retained programs.
    #[napi]
    pub fn apply_changes(
        &mut self,
        changes_json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.profiled("apply_changes", |this| {
            this.change_facts(changes_json, max_waves)
        })
    }

    /// Limits apply independently to each subsequent run/query. An empty object removes limits.
    #[napi]
    pub fn set_limits(&mut self, limits_json: String) -> Result<()> {
        self.limits = serde_json::from_str(&limits_json).map_err(error)?;
        Ok(())
    }

    /// Reuse an index on a session-owned result for later rules and ad hoc queries.
    #[napi]
    pub fn create_index(&self, relation: String, columns: Vec<u32>) -> Result<()> {
        self.profiled_read("create_index", |this| {
            this.result_index(&relation, &columns)
        })
    }

    /// Register SQL functions implementing an associative, commutative, idempotent lattice join.
    /// The aggregate and binary merge must agree; widening must terminate ascending chains.
    #[napi]
    pub fn register_lattice(&mut self, name: String, specification_json: String) -> Result<()> {
        self.profiled("register_lattice", |this| {
            if !identifier(&name) || matches!(name.as_str(), "flat" | "min" | "max") {
                return Err(error("custom lattices need a distinct SQL identifier"));
            }
            let mut value: Value = serde_json::from_str(&specification_json).map_err(error)?;
            let top = wire::exact(&value["top"])?;
            value["top"] = json!(top);
            let ops: schedule::LatticeOperations = serde_json::from_value(value).map_err(error)?;
            lattices::validate(&this.db, &ops)?;
            this.clear_results()?;
            this.lattice_extensions.insert(name, ops);
            Ok(())
        })
    }

    /// Lower an inline `.rr` program to SQL without running it. Its `.input` declarations read
    /// the session's bound facts, previous outputs, or existing database tables.
    #[napi]
    pub fn lower(&mut self, program: String, dialect: String) -> Result<String> {
        self.profiled("lower", |this| {
            let program = producers::prune(rr::parse(&program).map_err(error)?);
            let arities: BTreeMap<_, _> = program
                .decls
                .iter()
                .map(|(name, _)| (name.clone(), program::logical_arity(&program, name)))
                .collect();
            let (emitted, _) = this.compile(program, &dialect)?;
            serde_json::to_string(&json!({"setup": emitted.setup, "program": emitted.program, "logical_arities":arities,"indexes":indexes::wanted(&emitted),"analysis":emitted.analysis.as_ref()}))
                .map_err(error)
        })
    }

    /// Logical proofs and executor choices for all retained programs, without evaluating facts.
    #[napi]
    pub fn explain(&self) -> Result<String> {
        self.profiled_read("explain", |this| serde_json::to_string(&this.explain_value()).map_err(error))
    }

    /// Run the emitted SQL waves to a fixpoint. The returned plan can be inspected or replayed;
    /// outputs and `wave_*` diagnostics stay readable through `query`.
    #[napi]
    pub fn run(
        &mut self,
        program: String,
        dialect: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.profiled("run", |this| {
            this.run_program(
                producers::prune(rr::parse(&program).map_err(error)?),
                &dialect,
                max_waves.unwrap_or(1024),
            )
        })
    }

    /// As `run`, with `core.rr` and the file's includes read by the normal rule front end.
    #[napi]
    pub fn run_file(
        &mut self,
        path: String,
        dialect: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.profiled("run_file", |this| {
            this.run_program(
                producers::prune(rr::parse_file(Path::new(&path)).map_err(error)?),
                &dialect,
                max_waves.unwrap_or(1024),
            )
        })
    }

    /// One parameterized read query, including CTEs and joins over stored, virtual and derived
    /// relations. JSON parameters are positional scalars or tagged integers. Blobs return
    /// {blob: [byte, ...]}; unsafe JavaScript integers return {integer: decimal-string}.
    #[napi]
    pub fn query(
        &mut self,
        sql: String,
        parameters_json: Option<String>,
        max_rows: Option<u32>,
    ) -> Result<String> {
        self.profiled("query", |this| {
            this.ensure_catalog(&sql)?;
            let parameters: Vec<Value> =
                serde_json::from_str(parameters_json.as_deref().unwrap_or("[]")).map_err(error)?;
            let parameters = parameters
                .into_iter()
                .map(parameter)
                .collect::<Result<Vec<_>>>()?;
            let budget = this.limits.enter(this.db.connection_mut())?;
            let result = (|| {
                let mut statement = this.db.prepare(&sql).map_err(error)?;
                if !statement.readonly() || statement.column_count() == 0 {
                    return Err(error(
                        "query requires a read-only SQL statement with result columns",
                    ));
                }
                let columns: Vec<String> = statement.column_names().map(str::to_owned).collect();
                let mut cursor = statement
                    .query(rusqlite::params_from_iter(parameters))
                    .map_err(error)?;
                let limit = max_rows.unwrap_or(10_000) as usize;
                let mut rows = Vec::new();
                let mut truncated = false;
                while let Some(row) = cursor.next().map_err(error)? {
                    if rows.len() == limit {
                        truncated = true;
                        break;
                    }
                    let mut values = Vec::new();
                    for c in 0..columns.len() {
                        values.push(match row.get_ref(c).map_err(error)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => wire::integer(n),
                            ValueRef::Real(n) => json!(n),
                            ValueRef::Text(s) => {
                                Value::String(std::str::from_utf8(s).map_err(error)?.to_owned())
                            }
                            ValueRef::Blob(b) => json!({"blob": b}),
                        });
                    }
                    rows.push(values);
                }
                serde_json::to_string(
                    &json!({"columns": columns, "rows": rows, "truncated": truncated}),
                )
                .map_err(error)
            })();
            budget.leave(this.db.connection_mut());
            result
        })
    }
}

fn reserved(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "atoms"
        || name.starts_with("__rr_update_")
        || catalog::owns(&name)
        || ["sqlite_", "wave_", "delta_", "new_", "rdl_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn parameter(value: Value) -> Result<SqlValue> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(b)),
        Value::Number(n) if n.is_i64() => SqlValue::Integer(n.as_i64().unwrap()),
        Value::Number(n) if n.is_u64() => {
            return Err(error(
                "integer SQL parameter exceeds the signed 64-bit range",
            ));
        }
        Value::Number(n) => SqlValue::Real(
            n.as_f64()
                .ok_or_else(|| error("SQL parameter is out of range"))?,
        ),
        Value::String(s) => SqlValue::Text(s),
        Value::Object(_) => SqlValue::Integer(wire::exact(&value)?),
        _ => return Err(error("SQL parameters must be scalars or tagged integers")),
    })
}

impl CodeRuleSqlExecutor {
    /// The owned connection, for Rust extensions registering SQL functions or virtual tables.
    /// It remains borrowed for the lifetime of any statement or cursor made through it.
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.db
    }

    pub fn replace_sources(&mut self, sources: Vec<(String, Arc<dyn Source>)>) -> Result<()> {
        self.clear_results()?;
        self.db.remove_bindings().map_err(error)?;
        self.inputs.clear();
        self.documents = None;
        self.input_shapes.clear();
        self.mutable_inputs.clear();
        self.live_inputs.clear();
        self.db
            .bind_many(sources.iter().map(|(n, s)| (n.as_str(), s.clone())))
            .map_err(error)?;
        for (name, source) in sources {
            self.input_shapes
                .insert(name.to_owned(), (source.columns().len(), None));
            self.inputs.insert(name.to_owned(), source);
        }
        Ok(())
    }

    fn run_program(
        &mut self,
        program: front::Program,
        dialect: &str,
        max_waves: u32,
    ) -> Result<String> {
        self.statistics.materialize(&self.db)?;
        let outputs = program.outputs.clone();
        let declared = Arc::new(program.clone());
        let arities: BTreeMap<_, _> = program
            .decls
            .iter()
            .map(|(name, _)| (name.clone(), program::logical_arity(&program, name)))
            .collect();
        let (mut emitted, producers) = self.compile(program, dialect)?;
        let objects = self.output_objects(&emitted)?;
        let projection_relations = session::normalize_views(&mut emitted);
        let indexes = indexes::wanted(&emitted);
        self.db
            .execute_batch("SAVEPOINT sql_extension_run")
            .map_err(error)?;
        if let Err(e) = self.versions.begin() {
            let _ = self.db.execute_batch("RELEASE sql_extension_run");
            return Err(e);
        }
        let trace_namespace = self.next_trace;
        self.next_trace += 1;
        let previous = (
            self.generated.clone(),
            self.executions.clone(),
            self.control_ready.get(),
        );
        let mut affected: BTreeSet<usize> = self
            .executions
            .iter()
            .enumerate()
            .filter(|(_, run)| {
                run.objects
                    .iter()
                    .any(|(name, _)| objects.iter().any(|(n, _)| n == name))
            })
            .map(|(i, _)| i)
            .collect();
        let changed = affected
            .iter()
            .flat_map(|i| {
                self.executions[*i]
                    .objects
                    .iter()
                    .map(|(name, _)| name.clone())
            })
            .collect();
        affected.extend(self.affected(&changed));
        let limits = self.limits.clone();
        let mut budget = None;
        let result = (|| {
            self.remove_executions(&affected)?;
            self.clear_control()?;
            self.db.reset_execution();
            budget = Some(limits.enter(self.db.connection_mut())?);
            let needs_integers = emitted
                .queries
                .iter()
                .flatten()
                .chain(emitted.views.iter().map(|(_, q)| q))
                .any(constants::uses_integer_domain);
            for statement in &emitted.setup {
                let setup = statement.trim_start();
                if !needs_integers
                    && (setup.starts_with("CREATE TEMP TABLE rdl_int(")
                        || setup.starts_with("INSERT INTO rdl_int "))
                {
                    continue;
                }
                // The split-position table is invariant across programs in this session.
                if statement.starts_with("CREATE TEMP TABLE rdl_int")
                    && self.db.query_row("SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name = 'rdl_int')", [], |r| r.get::<_, bool>(0)).map_err(error)?
                {
                    continue;
                }
                self.db.execute_batch(statement).map_err(error)?;
            }
            #[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
            let retained = !self.cold_oracle;
            #[cfg(not(any(any(test, feature = "test-hooks"), feature = "headless")))]
            let retained = true;
            let minimal_setup = emitted.queries.iter().all(Option::is_some);
            if minimal_setup { schedule::declare_outputs(&self.db, &emitted.program).map_err(error)?; }
            else { schedule::declare_sql(&self.db, &emitted.program).map_err(error)?; }
            for (table, columns) in &indexes {
                if minimal_setup && !emitted.program.relations.iter().any(|(r, _)| r == table) { continue; }
                session::index(&self.db, table, columns).map_err(error)?;
            }
            self.control_ready.set(true);
            self.db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS wave_stats(tbl TEXT PRIMARY KEY,rows INTEGER NOT NULL) WITHOUT ROWID").map_err(error)?;
            let counts = emitted
                .program
                .relations
                .iter()
                .map(|(r, _)| (r.clone(), 0))
                .collect();
            let owned: BTreeSet<_> = objects.iter().map(|(name, _)| name.as_str()).collect();
            let mut dependencies: BTreeSet<String> = emitted.inputs.iter().cloned().collect();
            dependencies.extend(producers.values().flatten().cloned());
            for q in emitted
                .queries
                .iter()
                .flatten()
                .chain(emitted.views.iter().map(|(_, q)| q))
            {
                for r in q.reads() {
                    if let Some(name) = r.program_relation()
                        && !owned.contains(name)
                    {
                        dependencies.insert(name.into());
                    }
                }
            }
            let mut execution = session::Execution {
                inputs: dependencies,
                objects: objects.clone(),
                counts,
                emitted: Arc::new(emitted.clone()),
                program: declared.clone(),
                producers: producers.clone(),
                outputs: outputs.clone(),
                max_waves,
                reductions: None,
                recursive: BTreeMap::new(),
                temporal: BTreeMap::new(),
            };
            let mut work = maintenance::Work::new(&self.db, &self.scratch);
            work.versions = Some(&self.versions);
            work.profiler = self.db.profiler();
            let materialized_views = 0;
            let constant_tables = constants::prepare(&self.db, &mut execution)?;
            let native_initial = if retained {
                temporal::initial::run(&mut work, &self.versions, &mut execution, trace_namespace, &emitted.program, !minimal_setup)?
            } else { None };
            let mut initializations = Vec::new();
            let waves = if let Some(waves) = native_initial {
                waves
            } else {
                let _scope = profiling::scope(&self.db, "cold execution");
                if minimal_setup {
                    schedule::declare_sql(&self.db, &emitted.program).map_err(error)?;
                    for (table, columns) in &indexes {
                        session::index(&self.db, table, columns).map_err(error)?;
                    }
                }
                #[cfg(any(any(test, feature = "test-hooks"), feature = "headless"))]
                let retain = !self.cold_oracle;
                #[cfg(not(any(any(test, feature = "test-hooks"), feature = "headless")))]
                let retain = true;
                let waves = if retain {
                    let (waves, initialized) = reductions::cold::execute(&self.db, &mut work, &mut execution, max_waves)?;
                    initializations = initialized;
                    waves
                } else {
                    schedule::execute_declared(&self.db, &emitted.program, max_waves).map_err(error)?
                };
                let mut stats = self
                    .db
                    .prepare("SELECT tbl,rows FROM wave_stats")
                    .map_err(error)?;
                for row in stats
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                    .map_err(error)?
                {
                    let (r, n) = row.map_err(error)?;
                    if execution.counts.contains_key(&r) {
                        execution.counts.insert(r, n as u64);
                    }
                }
                if retain {
                    for (name,_) in &emitted.views {
                        self.db.execute_batch(&format!("ANALYZE temp.{}",quote(name))).map_err(error)?;
                    }
                    let _support = profiling::scope(&self.db, "support preparation");
                    let walks = walks::prepare(&mut work, &mut execution, None)?;
                    if !walks.is_empty() { reductions::cold::after_walks(&work, &mut execution)?; }
                    initializations.extend(walks);
                    let mut potential = execution.inputs.clone();
                    initializations.extend(reductions::prepare(
                        &mut work,
                        &mut execution,
                        &mut potential,
                        None,
                    )?);
                    initializations.sort();
                    initializations.dedup();
                    drop(_support);
                    let _recursive = profiling::scope(&self.db, "recursive preparation");
                    temporal::prepare(&mut work, &self.versions, &mut execution, trace_namespace)?;
                    recursive::prepare(&self.db, &mut execution)?;
                }
                waves
            };
            let statistics = self.executions.iter().flat_map(|e|&e.counts).chain(execution.counts.iter()).map(|(r,n)|vec![SqlValue::Text(r.clone()),SqlValue::Integer(*n as i64)])
                .chain(self.inputs.iter().map(|(r,s)|vec![SqlValue::Text(r.clone()),SqlValue::Integer(s.estimated_len() as i64)])).collect();
            if execution.temporal.len()==1 && execution.temporal.get(&0).is_some_and(|t|t.stable_phases(&execution.emitted.program)) {
                self.statistics.enable(&self.db,statistics)?;
            } else {
                plan::insert_many(&self.db,"INSERT OR REPLACE INTO wave_stats(tbl,rows)",statistics).map_err(error)?;
            }
            work.cleanup()?;
            let preparation = json!({"initializations":initializations,"materialized_views":materialized_views,"projection_relations":projection_relations,"constant_tables":constant_tables,"scratch_tables_created":work.created,"summary_transition_reuses":work.summary_reuses,"recursive_frames":execution.temporal.values().map(temporal::Trace::frames).sum::<usize>(),"initial_execution":if native_initial.is_some(){"retained"}else{"cold"}});
            self.db
                .execute_batch("RELEASE sql_extension_run")
                .map_err(error)?;
            Ok((waves, execution, work.pool, preparation))
        })();
        if let Some(budget) = budget {
            budget.leave(self.db.connection_mut());
        }
        let (waves, execution, pool, preparation) = match result {
            Ok(value) => {
                self.versions.commit();
                value
            }
            Err(e) => {
                let native_rollback = self.versions.rollback(&self.db);
                let _ = self
                    .db
                    .execute_batch("ROLLBACK TO sql_extension_run; RELEASE sql_extension_run");
                self.statistics.forget(&self.db)?;
                self.generated = previous.0;
                self.executions = previous.1;
                self.control_ready.set(previous.2);
                self.db.reset_execution();
                native_rollback?;
                return Err(e);
            }
        };
        self.scratch = pool;
        self.generated.extend(execution.objects.iter().cloned());
        self.executions.push(execution);
        self.stats_ready.set(true);
        self.refresh_catalog()?;
        serde_json::to_string(
            &json!({"waves": waves, "outputs": outputs, "setup": emitted.setup, "program": emitted.program,"logical_arities":arities,"indexes":indexes,"preparation":preparation,"analysis":emitted.analysis.as_ref()}),
        )
        .map_err(error)
    }

    fn clear_results(&mut self) -> Result<()> {
        self.db.profile_checkpoint();
        // Views first, then their backing tables. Input virtual tables are untouched.
        for table in [false, true] {
            for (name, is_table) in self
                .generated
                .iter()
                .rev()
                .filter(|(_, is_table)| *is_table == table)
            {
                if self.versions.owns(name) {
                    self.versions.remove_all(&self.db, name)?;
                    continue;
                }
                let kind = if *is_table { "TABLE" } else { "VIEW" };
                self.db
                    .execute_batch(&format!("DROP {kind} IF EXISTS temp.{}", quote(name)))
                    .map_err(error)?;
            }
        }
        self.clear_control()?;
        self.generated.clear();
        self.executions.clear();
        self.scratch.clear(&self.db, &self.versions)?;
        self.db.reset_execution();
        self.refresh_catalog()
    }
}

// cratify: `tests` is rerail-engine's (it mounts it)
