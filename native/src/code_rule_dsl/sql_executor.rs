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
mod catalog;
mod constants;
mod corpus;
mod documents;
mod indexes;
mod live;
mod maintenance;
mod overlay;
mod producers;
mod recursive;
mod reductions;
mod scratch;
mod updates;
mod versions;
mod walks;
use super::logic::sql::lattices;
use super::logic::sql::program;
mod runtime;
mod session;
mod wire;

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
    db: Database,
    store: ContentStore,
    inputs: BTreeMap<String, Arc<dyn Source>>,
    generated: Vec<(String, bool)>,
    executions: Vec<session::Execution>,
    limits: runtime::Limits,
    lattice_extensions: BTreeMap<String, schedule::LatticeOperations>,
    input_shapes: BTreeMap<String, (usize, Option<Vec<String>>)>,
    mutable_inputs: BTreeSet<String>,
    live_inputs: BTreeMap<String, Arc<live::Live>>,
    documents: Option<documents::Documents>,
    scratch: scratch::Pool,
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
        })
    }

    /// Replace the session's inputs with core fact virtual tables over a repository snapshot.
    /// Build everything before replacing bindings; a malformed/unparsed file fails the call.
    #[napi]
    pub fn bind_snapshot(&mut self, snapshot_json: String) -> Result<()> {
        let (docs, sources) = documents::bind(&self.store, &snapshot_json)?;
        self.replace_sources(sources.into_iter().collect())?;
        self.documents = Some(docs);
        Ok(())
    }

    /// Maintain retained SQL results across a complete replacement snapshot. Only changed
    /// files rebuild native facts; additions and removals use the same atomic delta scheduler.
    #[napi]
    pub fn apply_snapshot(
        &mut self,
        snapshot_json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.change_snapshot(snapshot_json, max_waves)
    }

    /// Apply an atomic {upsert: [snapshot files], delete: [paths]} batch. Unmentioned files
    /// retain their native facts and metadata without another inventory scan.
    #[napi]
    pub fn apply_file_changes(
        &mut self,
        changes_json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.change_files(changes_json, max_waves)
    }

    /// Bind current core facts from the database, without source files or a new snapshot.
    /// With no path list, use every currently bound core document in `scope`.
    #[napi]
    pub fn bind_stored(&mut self, scope: String, paths: Option<Vec<String>>) -> Result<()> {
        let paths = match paths {
            Some(paths) => paths,
            None => {
                let prefix = format!("{scope}#core:");
                self.db.prepare("SELECT substr(sym_text(name_atom), length(?1) + 1) FROM name_bindings WHERE substr(sym_text(name_atom), 1, length(?1)) = ?1 AND digest <> zeroblob(32) ORDER BY name_atom").map_err(error)?
                    .query_map([prefix], |r| r.get::<_, String>(0)).map_err(error)?
                    .collect::<rusqlite::Result<Vec<_>>>().map_err(error)?
            }
        };
        if paths.iter().collect::<BTreeSet<_>>().len() != paths.len() {
            return Err(error("stored document paths must be distinct"));
        }
        let documents =
            super::waves::matches::documents(&self.store, &scope, &paths).map_err(error)?;
        if documents.len() != paths.len() || documents.iter().any(|d| d.facts.syntax_errors) {
            return Err(error(
                "requested stored documents have missing or incomplete core facts",
            ));
        }
        let sources = syntax::shaped(&self.store, documents.clone(), &syntax::Projection::core())
            .map_err(error)?;
        syntax::fresh(&self.store, &documents).map_err(error)?;
        self.replace_sources(
            sources
                .into_iter()
                .map(|(n, s)| (n.to_owned(), s))
                .collect(),
        )
    }

    /// Bind integer or symbol facts, including rows emitted by a native AST matcher.
    /// `relation_json` is {name, columns, rows, types?}. Strings become symbol atoms; tagged
    /// decimal integers preserve values outside JavaScript's safe range.
    #[napi]
    pub fn bind_relation(&mut self, relation_json: String) -> Result<()> {
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
        self.check_input(&input.name, &input.columns, logical_arity)?;
        let symbols: BTreeSet<&str> = input
            .rows
            .iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let names: Vec<&str> = symbols.into_iter().collect();
        let atoms = self.store.intern_atoms(&names).map_err(error)?;
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
        self.install_input(input.name.clone(), source.clone(), logical_arity, logical_types, true)?;
        self.live_inputs.insert(input.name, source);
        Ok(())
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
        self.clear_results()
    }

    /// Atomically insert/delete fact rows and maintain dependent retained programs.
    #[napi]
    pub fn apply_changes(
        &mut self,
        changes_json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.change_facts(changes_json, max_waves)
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
        self.result_index(&relation, &columns)
    }

    /// Register SQL functions implementing an associative, commutative, idempotent lattice join.
    /// The aggregate and binary merge must agree; widening must terminate ascending chains.
    #[napi]
    pub fn register_lattice(&mut self, name: String, specification_json: String) -> Result<()> {
        if !identifier(&name) || matches!(name.as_str(), "flat" | "min" | "max") {
            return Err(error("custom lattices need a distinct SQL identifier"));
        }
        let mut value: Value = serde_json::from_str(&specification_json).map_err(error)?;
        let top = wire::exact(&value["top"])?;
        value["top"] = json!(top);
        let ops: schedule::LatticeOperations = serde_json::from_value(value).map_err(error)?;
        lattices::validate(&self.db, &ops)?;
        self.clear_results()?;
        self.lattice_extensions.insert(name, ops);
        Ok(())
    }

    /// Lower an inline `.rr` program to SQL without running it. Its `.input` declarations read
    /// the session's bound facts, previous outputs, or existing database tables.
    #[napi]
    pub fn lower(&self, program: String, dialect: String) -> Result<String> {
        let program = producers::prune(rr::parse(&program).map_err(error)?);
        let arities: BTreeMap<_, _> = program
            .decls
            .iter()
            .map(|(name, _)| (name.clone(), program::logical_arity(&program, name)))
            .collect();
        let (emitted, _) = self.compile(program, &dialect)?;
        serde_json::to_string(&json!({"setup": emitted.setup, "program": emitted.program, "logical_arities":arities,"indexes":indexes::wanted(&emitted)}))
            .map_err(error)
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
        self.run_program(
            producers::prune(rr::parse(&program).map_err(error)?),
            &dialect,
            max_waves.unwrap_or(1024),
        )
    }

    /// As `run`, with `core.rr` and the file's includes read by the normal rule front end.
    #[napi]
    pub fn run_file(
        &mut self,
        path: String,
        dialect: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.run_program(
            producers::prune(rr::parse_file(Path::new(&path)).map_err(error)?),
            &dialect,
            max_waves.unwrap_or(1024),
        )
    }

    /// One parameterized read query, including CTEs and joins over stored, virtual and derived
    /// relations. JSON parameters are positional scalars or tagged integers. Blobs return
    /// {blob: [byte, ...]}; unsafe JavaScript integers return {integer: decimal-string}.
    #[napi]
    pub fn query(
        &self,
        sql: String,
        parameters_json: Option<String>,
        max_rows: Option<u32>,
    ) -> Result<String> {
        let parameters: Vec<Value> =
            serde_json::from_str(parameters_json.as_deref().unwrap_or("[]")).map_err(error)?;
        let parameters = parameters
            .into_iter()
            .map(parameter)
            .collect::<Result<Vec<_>>>()?;
        let _budget = self.limits.enter(&self.db)?;
        let mut statement = self.db.prepare(&sql).map_err(error)?;
        if !statement.readonly() || statement.column_count() == 0 {
            return Err(error(
                "query requires a read-only SQL statement with result columns",
            ));
        }
        let columns: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
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
        serde_json::to_string(&json!({"columns": columns, "rows": rows, "truncated": truncated}))
            .map_err(error)
    }
}

fn reserved(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "atoms"
        || name.starts_with("__rr_update_")
        || ["sqlite_", "wave_", "delta_", "new_", "rdl_", catalog::PREFIX]
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

    fn replace_sources(&mut self, sources: Vec<(String, Arc<dyn Source>)>) -> Result<()> {
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

    /// Lower and emit a program. Its SQL producers' steps are added last; their relations are
    /// returned beside the program, each with the session inputs its query reads.
    fn compile(&self, mut program: front::Program, dialect: &str) -> Result<(plan::emit::Emitted, BTreeMap<String, Vec<String>>)> {
        let producers = producers::take(&mut program)?;
        for name in &program.inputs {
            if let Some((arity, types)) = self.input_shapes.get(name)
                && let Some(cols) = program.decls.get(name)
            {
                if program::logical_arity(&program, name) != *arity {
                    return Err(error(format!(
                        "{name}: input logical arity differs from its declaration"
                    )));
                }
                if let Some(types) = types {
                    for ((_, declared), actual) in cols.iter().zip(types) {
                        let actual = match actual.as_str() {
                            "number" => "int",
                            "symbol" => "sym",
                            "loc" => "node",
                            other => other,
                        };
                        if actual != declared {
                            return Err(error(format!(
                                "{name}: bound {actual} column differs from declared {declared}"
                            )));
                        }
                    }
                }
            }
        }
        program::physical(&mut program);
        // The file front end includes all of core.rr. An extension needs bindings only for
        // inputs its retained rules or outputs actually read, not every possible core family.
        let read: BTreeSet<String> = program
            .rules
            .iter()
            .flat_map(|r| {
                r.body
                    .iter()
                    .filter_map(front::Literal::atom)
                    .map(|a| a.rel.clone())
            })
            .chain(program.outputs.iter().cloned())
            .collect();
        let unused: Vec<String> = program
            .inputs
            .iter()
            .filter(|name| !read.contains(*name))
            .cloned()
            .collect();
        for name in unused {
            program.inputs.retain(|input| *input != name);
            program.decls.remove(&name);
            program.facts.remove(&name);
        }
        let mut lattice_operations = BTreeMap::new();
        for (name, lattice) in &program.lattice {
            if lattice.lattice() == "flat" {
                continue;
            }
            let mut ops = lattices::specification(lattice.lattice())
                .or_else(|| self.lattice_extensions.get(lattice.lattice()).cloned())
                .or_else(|| lattices::registered(&self.db, lattice.lattice()).ok())
                .ok_or_else(|| {
                    error(format!(
                        "{name}: register SQL operations for lattice {}",
                        lattice.lattice()
                    ))
                })?;
            ops.after = lattice.widen;
            if ops.after.is_some() && ops.widen.is_none() {
                return Err(error(format!(
                    "{name}: this lattice needs a registered widening function"
                )));
            }
            lattice_operations.insert(name.clone(), ops);
        }
        let tops = program
            .lattice
            .iter()
            .map(|(name, l)| {
                (
                    name.clone(),
                    (
                        l.keys,
                        lattice_operations
                            .get(name)
                            .map_or(super::logic::native::TOP, |ops| ops.top),
                    ),
                )
            })
            .collect();
        program::tops(&mut program, &tops)?;
        let dialect = super::language::dialect_by_label(dialect)
            .ok_or_else(|| error(format!("unknown dialect: {dialect}")))?;
        let lowered = plan::lower::lower(&program, dialect, &mut |names| {
            self.store
                .intern_atoms(names)
                .map_err(|e| plan::Error::Plan(e.to_string()))
        })
        .map_err(error)?;
        let mut batch = HashMap::new();
        let has_wave_stats: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name = 'wave_stats')",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        for name in &lowered.program.inputs {
            if !identifier(name) || reserved(name) {
                return Err(error(format!("invalid input relation: {name}")));
            }
            let statement = self
                .db
                .prepare(&format!("SELECT * FROM {}", quote(name)))
                .map_err(error)?;
            let columns = statement.column_names();
            if columns.len() != lowered.program.decls[name].len() {
                return Err(error(format!(
                    "{name}: input arity differs from its declaration"
                )));
            }
            let source = select::Source::positional(name, &columns);
            let rows = match self.inputs.get(name) {
                Some(source) => source.estimated_len() as f64,
                // Planning reads recorded counts, never rescans an input table. An arbitrary
                // SQL table with no recorded count gets an estimate; this changes plans only.
                None if self.executions.iter().any(|e| e.counts.contains_key(name)) => {
                    self.executions
                        .iter()
                        .rev()
                        .find_map(|e| e.counts.get(name))
                        .copied()
                        .unwrap() as f64
                }
                None if has_wave_stats => self
                    .db
                    .query_row(
                        "SELECT rows FROM temp.wave_stats WHERE tbl = ?1",
                        [name],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(error)?
                    .map_or(1_000.0, |n| n as f64),
                None => 1_000.0,
            };
            batch.insert(name.clone(), (source, rows));
        }
        let mut emitted =
            plan::emit::emit(&lowered, &self.db, "main", "sql-extension", None, &batch)
                .map_err(error)?;
        use sha2::Digest;
        let key = format!(
            "{:x}",
            sha2::Sha256::digest(format!("{program:?}:{dialect:?}"))
        );
        lattices::transfers(&mut emitted, &tops);
        emitted.for_session(&key[..16], &self.inputs.keys().cloned().collect());
        emitted.program.lattice_operations = lattice_operations;
        let other = producers::steps(&self.db, &mut emitted, &program, &producers)?;
        // A table a query reads that is a session input is watched under the input's own name.
        let inputs = |name: &String| self.inputs.keys().find(|i| i.eq_ignore_ascii_case(name)).cloned();
        let producers = other.into_iter().map(|(rel, reads)| (rel, reads.iter().filter_map(inputs).collect())).collect();
        Ok((emitted, producers))
    }

    fn run_program(
        &mut self,
        program: front::Program,
        dialect: &str,
        max_waves: u32,
    ) -> Result<String> {
        let outputs = program.outputs.clone();
        let declared = Arc::new(program.clone());
        let arities: BTreeMap<_, _> = program
            .decls
            .iter()
            .map(|(name, _)| (name.clone(), program::logical_arity(&program, name)))
            .collect();
        let (emitted, producers) = self.compile(program, dialect)?;
        let indexes = indexes::wanted(&emitted);
        let mut objects: Vec<(String, bool)> = emitted
            .views
            .iter()
            .map(|(name, _)| (name.clone(), false))
            .collect();
        for (name, arity) in &emitted.program.relations {
            if name.starts_with("__") || reserved(name) {
                return Err(error(format!("{name}: reserved SQL wave relation")));
            }
            if *arity == 0 {
                return Err(error("SQL wave relations need at least one column"));
            }
            objects.extend([
                (name.clone(), true),
                (format!("new_{name}"), true),
                (format!("delta_{name}"), true),
            ]);
            if emitted
                .program
                .lattice_operations
                .get(name)
                .is_some_and(|ops| ops.after.is_some())
            {
                objects.push((format!("__sql_changes_{name}"), true));
            }
        }
        let mut unique = BTreeSet::new();
        let owned: BTreeSet<_> = self
            .generated
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        for (name, _) in &objects {
            if !identifier(name)
                || name.to_ascii_lowercase().starts_with("wave_")
                || name.to_ascii_lowercase().starts_with("rdl_")
                || name.eq_ignore_ascii_case("atoms")
                || !unique.insert(name.to_ascii_lowercase())
            {
                return Err(error(format!(
                    "invalid or overlapping wave relation: {name}"
                )));
            }
            let exists: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?1 COLLATE NOCASE UNION ALL SELECT 1 FROM temp.sqlite_schema WHERE name = ?1 COLLATE NOCASE)", [name], |r| r.get(0)).map_err(error)?;
            if (exists && !owned.contains(name.as_str()))
                || self
                    .inputs
                    .keys()
                    .any(|input| input.eq_ignore_ascii_case(name))
            {
                return Err(error(format!(
                    "{name}: SQL output already exists; rebind inputs to clear derived results"
                )));
            }
        }
        self.db
            .execute_batch("SAVEPOINT sql_extension_run")
            .map_err(error)?;
        let previous = (self.generated.clone(), self.executions.clone());
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
        let result = (|| {
            self.remove_executions(&affected)?;
            self.clear_control()?;
            self.db.reset_execution();
            let _budget = limits.enter(&self.db)?;
            for statement in &emitted.setup {
                // The split-position table is invariant across programs in this session.
                if statement.starts_with("CREATE TEMP TABLE rdl_int")
                    && self.db.query_row("SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name = 'rdl_int')", [], |r| r.get::<_, bool>(0)).map_err(error)?
                {
                    continue;
                }
                self.db.execute_batch(statement).map_err(error)?;
            }
            let waves = schedule::execute_sql_with(&self.db, &emitted.program, max_waves, || {
                for (table, columns) in &indexes {
                    session::index(&self.db, table, columns)?;
                }
                Ok(())
            })
            .map_err(error)?;
            self.db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS wave_stats(tbl TEXT PRIMARY KEY, rows INTEGER NOT NULL) WITHOUT ROWID").map_err(error)?;
            let mut counts: BTreeMap<String, u64> = emitted
                .program
                .relations
                .iter()
                .map(|(r, _)| (r.clone(), 0))
                .collect();
            for row in self
                .db
                .prepare("SELECT tbl,rows FROM wave_stats")
                .map_err(error)?
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
                })
                .map_err(error)?
            {
                let (name, n) = row.map_err(error)?;
                if counts.contains_key(&name) {
                    counts.insert(name, n);
                }
            }
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
            };
            let mut work = maintenance::Work::new(&self.db, &self.scratch);
            let materialized_views = maintenance::track_views(&mut work, &mut execution)?;
            let constant_tables = constants::prepare(&self.db, &mut execution)?;
            // These views used to inherit native estimates. Their retained SQL tables need
            // actual index statistics before support joins choose their access paths.
            for (name, _) in &emitted.views {
                self.db
                    .execute_batch(&format!("ANALYZE temp.{}", quote(name)))
                    .map_err(error)?;
            }
            let mut initializations = walks::prepare(&mut work, &mut execution, None)?;
            let mut potential = execution.inputs.clone();
            initializations.extend(reductions::prepare(
                &mut work,
                &mut execution,
                &mut potential,
                None,
            )?);
            initializations.sort();
            initializations.dedup();
            recursive::prepare(&self.db, &mut execution)?;
            work.cleanup()?;
            let preparation = json!({"initializations":initializations,"materialized_views":materialized_views,"constant_tables":constant_tables,"scratch_tables_created":work.created});
            self.db
                .execute_batch("RELEASE sql_extension_run")
                .map_err(error)?;
            Ok((waves, execution, work.pool, preparation))
        })();
        let (waves, execution, pool, preparation) = match result {
            Ok(value) => value,
            Err(e) => {
                let _ = self
                    .db
                    .execute_batch("ROLLBACK TO sql_extension_run; RELEASE sql_extension_run");
                self.generated = previous.0;
                self.executions = previous.1;
                self.db.reset_execution();
                return Err(e);
            }
        };
        self.scratch = pool;
        self.generated.extend(execution.objects.iter().cloned());
        self.executions.push(execution);
        self.refresh_catalog()?;
        serde_json::to_string(
            &json!({"waves": waves, "setup": emitted.setup, "program": emitted.program,"logical_arities":arities,"indexes":indexes,"preparation":preparation}),
        )
        .map_err(error)
    }

    fn clear_results(&mut self) -> Result<()> {
        // Views first, then their backing tables. Input virtual tables are untouched.
        for table in [false, true] {
            for (name, is_table) in self
                .generated
                .iter()
                .rev()
                .filter(|(_, is_table)| *is_table == table)
            {
                let kind = if *is_table { "TABLE" } else { "VIEW" };
                self.db
                    .execute_batch(&format!("DROP {kind} IF EXISTS temp.{}", quote(name)))
                    .map_err(error)?;
            }
        }
        self.clear_control()?;
        self.generated.clear();
        self.executions.clear();
        self.scratch.clear(&self.db)?;
        self.db.reset_execution();
        self.refresh_catalog()
    }

    fn clear_control(&self) -> Result<()> {
        for name in [
            "wave_step",
            "wave_read",
            "wave_delta",
            "wave_pending",
            "wave_after",
            "wave_document",
            "wave_stats",
        ] {
            self.db
                .execute_batch(&format!("DROP TABLE IF EXISTS temp.{name}"))
                .map_err(error)?;
        }
        Ok(())
    }
}

impl Drop for CodeRuleSqlExecutor {
    fn drop(&mut self) {
        let _ = self.db.remove_bindings();
        let _ = crate::content_store::atoms::Atoms::for_connection(&self.db).flush(&self.db);
    }
}

#[cfg(test)]
mod tests;
