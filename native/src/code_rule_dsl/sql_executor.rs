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
/// are populated. Results survive between calls until an input is rebound or the session closes.
#[napi]
pub struct CodeRuleSqlExecutor {
    db: Database,
    store: ContentStore,
    inputs: BTreeMap<String, Arc<dyn Source>>,
    generated: Vec<(String, bool)>,
}

#[napi]
impl CodeRuleSqlExecutor {
    #[napi(constructor)]
    pub fn new(store_path: String) -> Result<Self> {
        let store = ContentStore::open(Path::new(&store_path)).map_err(error)?;
        let db = Database::open(&store).map_err(error)?;
        super::logic::native::register(&db).map_err(error)?;
        strings::register(&db).map_err(error)?;
        Ok(Self {
            db,
            store,
            inputs: BTreeMap::new(),
            generated: Vec::new(),
        })
    }

    /// Replace the session's inputs with core fact virtual tables over a repository snapshot.
    /// Build everything before replacing bindings; a malformed/unparsed file fails the call.
    #[napi]
    pub fn bind_snapshot(&mut self, snapshot_json: String) -> Result<()> {
        let (snapshot, _) = super::eval::parse_snapshot(&snapshot_json, false)?;
        let mut seen = BTreeSet::new();
        let mut documents = Vec::new();
        for file in &snapshot.files {
            if !seen.insert(&file.path) {
                return Err(error(format!("duplicate snapshot path: {}", file.path)));
            }
            let facts = match core::build_cached(file, &self.store).map_err(error)? {
                Built::Facts(facts) if !facts.syntax_errors => facts,
                _ => return Err(error(format!("{}: core facts are incomplete", file.path))),
            };
            let key = self.store.intern_atoms(&[&file.path]).map_err(error)?[0];
            documents.push(syntax::Document {
                key,
                path: file.path.clone(),
                core: String::new(),
                digest: [0; 32],
                facts: Arc::new(facts),
            });
        }
        let sources =
            syntax::shaped(&self.store, documents, &syntax::Projection::core()).map_err(error)?;
        self.replace_sources(sources)
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
        self.replace_sources(sources)
    }

    /// Bind additional integer facts, including rows emitted by a native AST matcher.
    /// `relation_json` is {name, columns: [name, ...], rows: [[integer, ...], ...]}.
    #[napi]
    pub fn bind_relation(&mut self, relation_json: String) -> Result<()> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            name: String,
            columns: Vec<String>,
            rows: Vec<Vec<i64>>,
        }
        let input: Input = serde_json::from_str(&relation_json).map_err(error)?;
        if !identifier(&input.name) || reserved(&input.name) {
            return Err(error("a fact relation needs a non-reserved SQL identifier"));
        }
        let stored: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?1 COLLATE NOCASE)",
                [&input.name],
                |r| r.get(0),
            )
            .map_err(error)?;
        if stored || input.name.eq_ignore_ascii_case("atoms") {
            return Err(error("a fact binding cannot replace a stored relation"));
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
        if self
            .inputs
            .keys()
            .any(|name| name.eq_ignore_ascii_case(&input.name) && *name != input.name)
        {
            return Err(error("fact names must be unique ignoring case"));
        }
        if self
            .inputs
            .get(&input.name)
            .is_some_and(|s| s.columns() != input.columns)
        {
            return Err(error("rebinding a fact cannot change its columns"));
        }
        let mut source =
            Relation::new(&input.columns.iter().map(String::as_str).collect::<Vec<_>>());
        for row in input.rows.into_iter().collect::<BTreeSet<_>>() {
            source.push(&row);
        }
        self.clear_results()?;
        let source: Arc<dyn Source> = Arc::new(source);
        self.db
            .bind(&input.name, Arc::clone(&source))
            .map_err(error)?;
        self.inputs.insert(input.name, source);
        Ok(())
    }

    /// Lower an inline `.rr` program to SQL without running it. Its `.input` declarations read
    /// the session's bound facts, previous outputs, or existing database tables.
    #[napi]
    pub fn lower(&self, program: String, dialect: String) -> Result<String> {
        let emitted = self.compile(front::prune(rr::parse(&program).map_err(error)?), &dialect)?;
        serde_json::to_string(&json!({"setup": emitted.setup, "program": emitted.program}))
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
            front::prune(rr::parse(&program).map_err(error)?),
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
            front::prune(rr::parse_file(Path::new(&path)).map_err(error)?),
            &dialect,
            max_waves.unwrap_or(1024),
        )
    }

    /// One parameterized read query, including CTEs and joins over stored, virtual and derived
    /// relations. JSON parameters are positional scalars. Blobs return {blob: [byte, ...]}.
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
                    ValueRef::Integer(n) => json!(n),
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
        _ => return Err(error("SQL parameters must be scalar JSON values")),
    })
}

impl CodeRuleSqlExecutor {
    /// The owned connection, for Rust extensions registering SQL functions or virtual tables.
    /// It remains borrowed for the lifetime of any statement or cursor made through it.
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.db
    }

    fn replace_sources(&mut self, sources: Vec<(&'static str, Arc<dyn Source>)>) -> Result<()> {
        self.clear_results()?;
        self.db.remove_bindings().map_err(error)?;
        self.inputs.clear();
        for (name, source) in sources {
            self.db.bind(name, Arc::clone(&source)).map_err(error)?;
            self.inputs.insert(name.to_owned(), source);
        }
        Ok(())
    }

    fn compile(&self, mut program: front::Program, dialect: &str) -> Result<plan::emit::Emitted> {
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
        for (name, lattice) in &program.lattice {
            if lattice.lattice() != "flat" {
                return Err(error(format!(
                    "{name}: the SQL wave executor supports flat lattices; {} needs a native producer",
                    lattice.lattice()
                )));
            }
        }
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
        plan::emit::emit(&lowered, &self.db, "main", "sql-extension", None, &batch).map_err(error)
    }

    fn run_program(
        &mut self,
        program: front::Program,
        dialect: &str,
        max_waves: u32,
    ) -> Result<String> {
        let emitted = self.compile(program, dialect)?;
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
        }
        let mut unique = BTreeSet::new();
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
            if exists
                || self
                    .inputs
                    .keys()
                    .any(|input| input.eq_ignore_ascii_case(name))
                || self
                    .generated
                    .iter()
                    .any(|(old, _)| old.eq_ignore_ascii_case(name))
            {
                return Err(error(format!(
                    "{name}: SQL output already exists; rebind inputs to clear derived results"
                )));
            }
        }
        self.db
            .execute_batch("SAVEPOINT sql_extension_run")
            .map_err(error)?;
        let result = (|| {
            for statement in &emitted.setup {
                // The split-position table is invariant across programs in this session.
                if statement.starts_with("CREATE TEMP TABLE rdl_int")
                    && self.db.query_row("SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE name = 'rdl_int')", [], |r| r.get::<_, bool>(0)).map_err(error)?
                {
                    continue;
                }
                self.db.execute_batch(statement).map_err(error)?;
            }
            let waves =
                schedule::execute_sql(&self.db, &emitted.program, max_waves).map_err(error)?;
            self.db
                .execute_batch("RELEASE sql_extension_run")
                .map_err(error)?;
            Ok(waves)
        })();
        let waves = match result {
            Ok(waves) => waves,
            Err(e) => {
                let _ = self
                    .db
                    .execute_batch("ROLLBACK TO sql_extension_run; RELEASE sql_extension_run");
                self.clear_control()?;
                self.db.reset_execution();
                return Err(e);
            }
        };
        self.generated.extend(objects);
        serde_json::to_string(
            &json!({"waves": waves, "setup": emitted.setup, "program": emitted.program}),
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
        self.db.reset_execution();
        Ok(())
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
