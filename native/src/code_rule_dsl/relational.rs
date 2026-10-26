//! Bounded relational queries over native AST evidence. Policy queries read only
//! their input tables and earlier stages in a fresh in-memory SQLite connection.
use super::eval::{evidence::Rows, failure, violation_from_witness};
use super::json::{hex_digest, invalid};
use super::model::{CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Snapshot};
use super::violations::{Captures, Witness};
use napi::Result;
use rusqlite::Connection;
use rusqlite::hooks::Authorization;
mod authorizer;
pub(super) mod pipeline;
pub(super) mod pool;
mod shared;
pub(super) use shared::Versions;
mod storage;

mod admission;
mod anchors;
mod batch;
pub(super) mod database;
use database::Database;
pub(super) mod deferred;
mod forest;
mod functions;
mod inputs;
mod insert;
mod producer;
mod reachability;
pub(super) mod schema;
mod source;
pub(super) mod syntax;
mod table;
pub(super) use reachability::Reachability;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
pub(super) struct Stage {
    pub(super) name: String,
    pub(super) inputs: BTreeSet<String>,
    pub(super) reads_source: bool,
    pub(super) recursive: bool,
    pub(super) query: String,
    pub(super) indexes: Vec<Vec<String>>,
    pub(super) reachability: Option<Reachability>,
    pub(super) deferred: BTreeMap<String, String>,
    pub(super) stored_columns: Vec<String>,
    pub(super) output_columns: Vec<String>,
    pub(super) declaration: Arc<str>,
}
#[derive(Clone)]
pub(super) struct References {
    pub(super) query: super::model::Query,
    pub(super) key: super::calls::contracts::Key,
}

pub(super) const REFERENCE_FIELDS: &[&str] = &[
    "target_path",
    "target_start",
    "target_end",
    "source_name",
    "target_name",
    "target_class",
];

#[derive(Clone)]
pub(super) struct Plan {
    pub(super) require_nonempty: Option<usize>,
    pub(super) syntax: Option<Arc<syntax::Spec>>,
    pub(super) per_file: bool,
    pub(super) references: Option<References>,
    pub(super) locations: BTreeSet<String>,
    pub(super) inputs: Vec<(String, FactQuery)>,
    pub(super) stages: Vec<Stage>,
    pub(super) query: String,
    pub(super) key: Vec<String>,
    pub(super) code: Option<String>,
    pub(super) last_uses: BTreeMap<String, usize>,
    pub(super) reads_source: bool,
}

pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with("__")
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()))
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
pub(super) fn sql_error(error: rusqlite::Error) -> napi::Error {
    invalid(format!("relational query: {error}"))
}

pub(super) fn connection(max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Database> {
    let db = validation_connection(max_pattern, work)?;
    db.execute_batch("PRAGMA temp_store=FILE; PRAGMA cache_spill=ON; PRAGMA cache_size=-1024; PRAGMA temp.cache_size=-512; PRAGMA analysis_limit=1000;")
        .map_err(sql_error)?;
    Ok(db)
}

/// Compiler sessions prepare authored queries and retain only empty stage schemas.
/// Runtime storage configuration belongs to `connection`, not schema validation.
pub(super) fn validation_connection(
    max_pattern: usize,
    work: Arc<AtomicUsize>,
) -> Result<Database> {
    configure(Database::open().map_err(sql_error)?, max_pattern, work)
}
fn configure(db: Database, max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Database> {
    configure_in_place(&db, max_pattern, work)?;
    Ok(db)
}
fn configure_in_place(db: &Database, max_pattern: usize, work: Arc<AtomicUsize>) -> Result<()> {
    db.progress_handler(0, None::<fn() -> bool>)
        .map_err(sql_error)?;
    db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        16 * 1024 * 1024,
    )
    .map_err(sql_error)?;
    db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH,
        1024 * 1024,
    )
    .map_err(sql_error)?;
    functions::install(db, max_pattern, Arc::clone(&work)).map_err(sql_error)?;
    source::install(db, None, work)?;
    Ok(())
}

/// Preparation checks the complete statement, including its function calls.
/// Neither attached databases, metadata, pragmas, writes nor nondeterministic
/// functions are available to a policy. CTEs and JSON table functions can join
/// only the finite relations supplied here.
pub(super) fn columns(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<Vec<String>> {
    inspect(db, query, tables).map(|(columns, _, _)| columns)
}

pub(super) fn inspect(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<(Vec<String>, BTreeSet<String>, bool)> {
    inspect_fields(db, query, tables)
        .map(|(columns, fields, source)| (columns, fields.into_keys().collect(), source))
}
pub(super) type FieldReads = BTreeMap<String, BTreeSet<String>>;
pub(super) fn inspect_fields(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<(Vec<String>, FieldReads, bool)> {
    inspect_fields_recursive(db, query, tables)
        .map(|(cols, fields, source, _)| (cols, fields, source))
}
pub(super) fn inspect_fields_recursive(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<(Vec<String>, FieldReads, bool, bool)> {
    let observed = authorizer::install(db, tables).map_err(sql_error)?;
    let result = (|| {
        let statement = db.prepare(query).map_err(sql_error)?;
        if !statement.readonly() || statement.parameter_count() != 0 {
            return Err(invalid(
                "relational queries must be read-only SELECT statements without parameters",
            ));
        }
        let columns = statement
            .column_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        let distinct = columns
            .iter()
            .map(|column| column.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if distinct.len() != columns.len() || columns.is_empty() {
            return Err(invalid("relational query columns must have distinct names"));
        }
        Ok((
            columns,
            observed.fields.lock().clone(),
            observed.source.load(Ordering::Relaxed),
            observed.recursive.load(Ordering::Relaxed),
        ))
    })();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)
        .map_err(sql_error)?;
    result
}

pub(super) fn create_input(db: &Database, name: &str, fields: &[String]) -> Result<()> {
    db.storage
        .borrow_mut()
        .declare(db, name, input_schema(name, fields), true)
        .map_err(sql_error)?;
    Ok(())
}
pub(super) fn input_schema(name: &str, fields: &[String]) -> String {
    format!(
        "CREATE TABLE {} (__path TEXT NOT NULL, __start INTEGER NOT NULL, __end INTEGER NOT NULL{})",
        quote(name),
        fields
            .iter()
            .map(|field| format!(",{}", quote(field)))
            .collect::<String>()
    )
}
pub(super) fn create_stage(
    db: &Database,
    stage: &Stage,
    materialize: bool,
    work: &Arc<AtomicUsize>,
    _logical_name: &str,
) -> Result<()> {
    if materialize && let Some(spec) = &stage.reachability {
        return reachability::create(db, stage, spec, work);
    }
    #[cfg(feature = "code-rule-profile")]
    if materialize && std::env::var_os("RERAIL_SQL_QUERY_PLANS").is_some() {
        let mut span = super::profile::trace::Span::start("sql-plan", _logical_name);
        let plan = (|| -> rusqlite::Result<Vec<Value>> {
            let mut statement = db.prepare(&format!("EXPLAIN QUERY PLAN {}", stage.query))?;
            statement
                .query_map([], |row| {
                    Ok(json!({
                        "id":row.get::<_, i64>(0)?, "parent":row.get::<_, i64>(1)?,
                        "detail":row.get::<_, String>(3)?,
                    }))
                })?
                .collect()
        })();
        span.detail(|| match plan {
            Ok(rows) => json!({"query":stage.query,"plan":rows}),
            Err(error) => json!({"error":error.to_string()}),
        });
    }
    #[cfg(feature = "code-rule-profile")]
    let mut span = super::profile::trace::Span::start("sql-materialize", _logical_name);
    #[cfg(feature = "code-rule-profile")]
    let work_before = work.load(Ordering::Relaxed);
    let physical = if materialize {
        deferred::physical_name(stage)
    } else {
        stage.name.clone()
    };
    let query = if materialize && !stage.deferred.is_empty() {
        format!(
            "SELECT {} FROM ({})",
            stage
                .stored_columns
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(","),
            stage.query
        )
    } else {
        stage.query.clone()
    };
    let sql = if materialize {
        db.storage
            .borrow_mut()
            .declare(db, &physical, stage.declaration.to_string(), true)
            .map_err(sql_error)?;
        format!("INSERT INTO {} {query}", quote(&physical))
    } else {
        format!(
            "CREATE TEMP TABLE {} AS SELECT * FROM ({query}) LIMIT 0",
            quote(&physical)
        )
    };
    let mut statement = db.prepare(&sql).map_err(sql_error)?;
    let executed = statement.execute([]).map_err(sql_error);
    #[cfg(feature = "code-rule-profile")]
    {
        use rusqlite::StatementStatus;
        span.detail(|| json!({"vmSteps":statement.get_status(StatementStatus::VmStep),
            "fullscanSteps":statement.get_status(StatementStatus::FullscanStep),
            "sorts":statement.get_status(StatementStatus::Sort),
            "workBefore":work_before,"workAfter":work.load(Ordering::Relaxed),"autoIndexRows":statement.get_status(StatementStatus::AutoIndex),"failed":executed.is_err()}));
        drop(span);
    }
    let inserted = executed?;
    drop(statement);
    if materialize {
        db.storage.borrow_mut().written(&physical, inserted != 0);
    }
    if !materialize {
        for column in stage
            .deferred
            .keys()
            .filter(|name| !stage.stored_columns.contains(name))
        {
            // Replaced query columns already exist in the validation table.
            let exists = db
                .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&stage.name)))
                .map_err(sql_error)?
                .column_names()
                .contains(&column.as_str());
            if !exists {
                db.execute_batch(&format!(
                    "ALTER TABLE {} ADD COLUMN {}",
                    quote(&stage.name),
                    quote(column)
                ))
                .map_err(sql_error)?;
            }
        }
    }
    if materialize && inserted != 0 {
        for (index, fields) in stage.indexes.iter().enumerate() {
            #[cfg(feature = "code-rule-profile")]
            let _span = super::profile::trace::Span::start("sql-index", _logical_name);
            db.storage
                .borrow_mut()
                .index(
                    db,
                    &physical,
                    &format!("__{}_{}", stage.name, index),
                    format!(
                        "CREATE INDEX {} ON {} ({})",
                        quote(&format!("__{}_{}", stage.name, index)),
                        quote(&physical),
                        fields
                            .iter()
                            .map(|field| quote(field))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                )
                .map_err(sql_error)?;
        }
    }
    if materialize && !stage.indexes.is_empty() {
        #[cfg(feature = "code-rule-profile")]
        let _span = super::profile::trace::Span::start("sql-analyze", _logical_name);
        db.storage
            .borrow_mut()
            .analyze(db, &physical, inserted)
            .map_err(sql_error)?;
    }
    if materialize {
        deferred::view(db, stage)?;
    }
    Ok(())
}
fn read_value(value: rusqlite::types::ValueRef<'_>) -> Result<Value> {
    use rusqlite::types::ValueRef as Sql;
    Ok(match value {
        Sql::Null => Value::Null,
        Sql::Integer(value) => json!(value),
        Sql::Real(value) if value.is_finite() => json!(value),
        Sql::Text(value) => Value::String(
            std::str::from_utf8(value)
                .map_err(|_| invalid("relational result is not UTF-8"))?
                .to_owned(),
        ),
        _ => {
            return Err(invalid(
                "relational results must contain finite JSON scalars",
            ));
        }
    })
}

pub(super) struct Job<'a> {
    pub code_rule: &'a CompiledCodeRule,
    pub clause: &'a CompiledClause,
    pub inputs: Inputs,
    pub budget: super::budget::Context,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Preparation {
    Gate,
    Body,
}

pub(super) struct Inputs {
    pub versions: Option<Versions>,
    pub gate: Option<Rows>,
    pub rows: Option<Vec<Rows>>,
    pub facts: syntax::Files,
    pub snapshot: Snapshot,
    pub file_local: bool,
}
struct Session<'a> {
    code_rule: &'a CompiledCodeRule,
    clause: &'a CompiledClause,
    plan: &'a Plan,
    db: pool::Lease,
    finished: std::cell::Cell<bool>,
    syntax_files: syntax::Files,
    syntax_trees: Vec<producer::Tree>,
    anchors: anchors::Anchors,
    work: Arc<AtomicUsize>,
    tables: BTreeSet<String>,
    next: usize,
    aliases: BTreeMap<String, String>,
    physical_stages: Vec<Stage>,
    needed: BTreeSet<String>,
    visible: Snapshot,
}
impl<'a> Session<'a> {
    fn new(
        code_rule: &'a CompiledCodeRule,
        clause: &'a CompiledClause,
        mut inputs: Inputs,
        metrics: &mut super::model::Metrics,
        pool: &pool::Pool,
    ) -> Result<Self> {
        let snapshot = &inputs.snapshot;
        let supplied = inputs.facts;
        let limits = &code_rule.capabilities.limits;
        admission::limits(code_rule, snapshot)?;
        let super::model::ClauseKind::Relational(plan) = &clause.kind else {
            return Err(invalid("expected relational job"));
        };
        // Input writes spend one work unit per row. Adjust to the exact row/node allowance
        // below; no SQL stage can run before that check and all imports share this transaction.
        let work = Arc::new(AtomicUsize::new(usize::MAX));
        let db = pool.take(limits.max_regex_pattern_bytes, Arc::clone(&work))?;
        #[cfg(feature = "code-rule-profile")]
        db.context(json!({"scope":"relational", "ruleId":code_rule.id, "clauseId":clause.id,
            "path":(snapshot.files.len() == 1).then(|| snapshot.files[0].path.as_str()), "stage":"inputs"}));
        db.execute_batch("BEGIN").map_err(sql_error)?;
        let aliases = plan
            .inputs
            .iter()
            .map(|(name, _)| (name.clone(), db.private(name)))
            .chain(
                plan.references
                    .iter()
                    .map(|_| ("reference_edges".into(), db.private("reference_edges"))),
            )
            .collect::<BTreeMap<_, _>>();
        let gate_slot = plan.require_nonempty.filter(|_| inputs.gate.is_some());
        if let (Some(rows), Some(slot)) = (&mut inputs.rows, gate_slot) {
            rows[slot] = inputs.gate.take().expect("admitted gate");
        }
        let streamed = inputs.rows.is_none();
        let rows = match inputs.rows {
            Some(rows) => rows,
            None => {
                let _corpus = crate::code_rule_dsl::matcher::CorpusSlot::enter();
                let _calls = if inputs.file_local {
                    super::eval::stateless_calls(
                        code_rule,
                        snapshot,
                        std::slice::from_ref(clause),
                        metrics,
                    )?
                } else {
                    None
                };
                plan.inputs
                    .iter()
                    .enumerate()
                    .map(|(slot, (name, input))| {
                        if gate_slot == Some(slot) {
                            return Ok(inputs.gate.take().expect("admitted gate"));
                        }
                        let fields = input
                            .project
                            .iter()
                            .map(|p| p.field.clone())
                            .collect::<Vec<_>>();
                        let physical = &aliases[name];
                        create_input(&db, physical, &fields)?;
                        let before = db.total_changes();
                        let rows = inputs::stream(
                            &db,
                            (physical, name),
                            input,
                            &work,
                            code_rule,
                            clause,
                            snapshot,
                            metrics,
                        )?;
                        db.storage
                            .borrow_mut()
                            .written(physical, db.total_changes() != before);
                        Ok(rows)
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        if rows.iter().any(|r| !r.incomplete.is_empty()) {
            return Err(invalid("A relational input is incomplete"));
        }
        let syntax_files = if let Some(spec) = &plan.syntax {
            let gate = plan
                .inputs
                .iter()
                .position(|(n, _)| n == &spec.gate)
                .ok_or_else(|| invalid("missing syntax gate"))?;
            let paths = rows[gate]
                .witnesses
                .iter()
                .map(|w| w.path.as_str())
                .collect::<BTreeSet<_>>();
            snapshot
                .files
                .iter()
                .filter(|f| code_rule.scope.matches(&f.path) && paths.contains(f.path.as_str()))
                .map(|f| {
                    let facts = match supplied.iter().find(|(path, _)| path == &f.path) {
                        Some((_, facts)) => Arc::clone(facts),
                        None => syntax::Pool::get(f)?,
                    };
                    Ok((f.path.clone(), facts))
                })
                .collect::<Result<syntax::Files>>()?
        } else {
            Vec::new()
        };
        let count = rows.iter().map(|r| r.witnesses.len()).sum::<usize>()
            + syntax_files.iter().map(|(_, f)| f.len()).sum::<usize>();
        let syntax_trees = match &plan.syntax {
            Some(spec) => syntax::prepare_trees(spec, &syntax_files)?,
            None => Vec::new(),
        };
        let spent = usize::MAX - work.load(Ordering::Relaxed);
        let allowance = limits.visits_for(count);
        if spent > allowance {
            return Err(invalid("relational input row budget exceeded"));
        }
        work.store(allowance - spent, Ordering::Relaxed);
        let anchors = anchors::collect(plan, &rows);
        let visible = Snapshot {
            revision: snapshot.revision,
            files: snapshot
                .files
                .iter()
                .filter(|f| code_rule.scope.matches(&f.path))
                .cloned()
                .collect(),
        };
        if plan.reads_source {
            source::install(&db, Some(&visible), Arc::clone(&work))?;
        }
        let mut aliases = aliases;
        let versions = inputs
            .versions
            .unwrap_or_else(|| Versions::snapshot(snapshot));
        let domain = plan.per_file.then(|| {
            snapshot
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        let mut sources = BTreeMap::new();
        {
            let mut bindings = db.bindings.borrow_mut();
            for (name, _) in &plan.inputs {
                sources.insert(
                    name.clone(),
                    bindings.evidence(code_rule, &clause.id, name, &versions, &domain, 0),
                );
            }
            if plan.references.is_some() {
                sources.insert(
                    "reference_edges".into(),
                    bindings.evidence(
                        code_rule,
                        &clause.id,
                        "reference_edges",
                        &versions,
                        &domain,
                        db.id,
                    ),
                );
            }
            if plan.reads_source {
                sources.insert(
                    "__source_text".into(),
                    bindings.evidence(
                        code_rule,
                        &clause.id,
                        "__source_text",
                        &versions,
                        &domain,
                        0,
                    ),
                );
            }
        }
        let mut tables = BTreeSet::new();
        if let Some(spec) = &plan.syntax {
            let mut physical = (**spec).clone();
            physical.name = db.private(&spec.name);
            physical.edges = spec.edges.as_ref().map(|n| db.private(n));
            physical.ownership = spec.ownership.as_ref().map(|n| db.private(n));
            for (family, name) in [
                (0, Some(&spec.name)),
                (1, spec.edges.as_ref()),
                (2, spec.ownership.as_ref()),
            ] {
                if let Some(name) = name {
                    sources.insert(
                        name.clone(),
                        db.bindings.borrow_mut().source(spec, &syntax_files, family),
                    );
                    aliases.insert(name.clone(), db.private(name));
                    tables.insert(name.clone());
                }
            }
            for name in syntax::load(&db, &physical, &syntax_files, Arc::clone(&work))? {
                db.module(name);
            }
        }
        for (slot, ((name, input), rows)) in plan.inputs.iter().zip(rows).enumerate() {
            #[cfg(feature = "code-rule-profile")]
            if std::env::var_os("RERAIL_CODE_RULE_RELATIONAL_PROFILE").is_some() {
                eprintln!(
                    "RERAIL_CODE_RULE_RELATIONAL_INPUT {}",
                    json!({"table":name,"rows":rows.witnesses.len()})
                );
            }
            let fields = input
                .project
                .iter()
                .map(|project| project.field.clone())
                .collect::<Vec<_>>();
            if streamed && gate_slot != Some(slot) {
                tables.insert(name.clone());
                continue;
            }
            let physical = &aliases[name];
            create_input(&db, physical, &fields)?;
            // Bind projection positions once per shared schema, never search field names per row.
            let mut layouts = HashMap::new();
            for witness in &rows.witnesses {
                if let Some(projected) = witness.captures.projection() {
                    layouts
                        .entry(projected.schema_identity())
                        .or_insert_with(|| {
                            fields
                                .iter()
                                .map(|f| projected.index(f))
                                .collect::<Vec<_>>()
                        });
                }
            }
            let before = db.total_changes();
            insert::insert_rows(
                &db,
                physical,
                fields.len() + 3,
                rows.witnesses.iter().map(|witness| {
                    use rusqlite::types::ValueRef;
                    let location = super::violations::anchored_span(&input.anchor, witness).0;
                    let projected = witness.captures.projection();
                    let layout = projected.map(|p| &layouts[&p.schema_identity()]);
                    [
                        ValueRef::Text(witness.path.as_bytes()),
                        ValueRef::Integer(location.start_byte as i64),
                        ValueRef::Integer(location.end_byte as i64),
                    ]
                    .into_iter()
                    .chain(fields.iter().enumerate().map(move |(i, _)| {
                        projected
                            .zip(layout.and_then(|l| l[i]))
                            .map_or(ValueRef::Null, |(row, index)| row.sql(index))
                    }))
                    .map(insert::Borrowed)
                }),
                &work,
                "relational input row budget exceeded",
            )?;
            db.storage
                .borrow_mut()
                .written(physical, db.total_changes() != before);
            tables.insert(name.clone());
            // SQL owns the copied values; only compact anchors remain necessary.
        }
        if let Some(reference) = &plan.references {
            let graph = super::calls::reference_rows(reference.key)
                .ok_or_else(|| invalid("No native reference graph is installed"))?;
            if graph
                .incomplete
                .iter()
                .any(|path| code_rule.scope.matches(path))
            {
                return Err(invalid("The native reference graph is incomplete"));
            }
            create_input(
                &db,
                &aliases["reference_edges"],
                &REFERENCE_FIELDS
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect::<Vec<_>>(),
            )?;
            let before = db.total_changes();
            insert::insert_rows(
                &db,
                &aliases["reference_edges"],
                9,
                graph
                    .edges
                    .iter()
                    .filter(|edge| code_rule.scope.matches(&edge.path))
                    .map(|edge| {
                        use rusqlite::types::Value as Sql;
                        [
                            Sql::Text(edge.path.to_string()),
                            Sql::Integer(edge.source.start as i64),
                            Sql::Integer(edge.source.end as i64),
                            Sql::Text(edge.target_path.to_string()),
                            Sql::Integer(edge.target.start as i64),
                            Sql::Integer(edge.target.end as i64),
                            Sql::Text(edge.source_name.text().to_owned()),
                            Sql::Text(edge.target_name.text().to_owned()),
                            Sql::Integer(i64::from(
                                edge.target.flags & super::calls::summary::CLASS != 0,
                            )),
                        ]
                    }),
                &work,
                "native reference row budget exceeded",
            )?;
            db.storage
                .borrow_mut()
                .written(&aliases["reference_edges"], db.total_changes() != before);
            tables.insert("reference_edges".to_owned());
        }
        db.execute_batch("COMMIT").map_err(sql_error)?;
        let bindings =
            db.bindings
                .borrow_mut()
                .stages(plan, &sources, limits.max_regex_pattern_bytes);
        for stage in &plan.stages {
            let name = if stage.reachability.is_some() || !stage.deferred.is_empty() {
                db.private(&stage.name)
            } else {
                shared::physical(bindings[&stage.name])
            };
            aliases.insert(stage.name.clone(), name);
        }
        let physical_stages = plan
            .stages
            .iter()
            .map(|stage| {
                let inputs = stage
                    .inputs
                    .iter()
                    .map(|n| (n.clone(), aliases[n].clone()))
                    .collect();
                shared::stage(stage, &aliases[&stage.name], &inputs)
            })
            .collect::<Vec<_>>();
        let mut needed = plan
            .last_uses
            .iter()
            .filter(|(_, last)| **last == plan.stages.len())
            .map(|(n, _)| n.clone())
            .collect::<BTreeSet<_>>();
        needed.extend(
            plan.stages
                .iter()
                .filter(|s| s.reachability.is_some())
                .map(|s| s.name.clone()),
        );
        for (logical, physical) in plan.stages.iter().zip(&physical_stages).rev() {
            if needed.contains(&logical.name) {
                db.pin(&physical.name);
                if !db.storage.borrow().ready(&physical.name) {
                    needed.extend(logical.inputs.iter().cloned());
                }
            }
        }
        Ok(Self {
            code_rule,
            clause,
            plan,
            db,
            finished: std::cell::Cell::new(false),
            syntax_files,
            syntax_trees,
            anchors,
            work,
            tables,
            next: 0,
            aliases,
            physical_stages,
            needed,
            visible,
        })
    }
    fn activate(&self) -> Result<()> {
        self.db.activate(
            self.code_rule.capabilities.limits.max_regex_pattern_bytes,
            Arc::clone(&self.work),
        )?;
        if self.plan.reads_source {
            source::install(&self.db, Some(&self.visible), Arc::clone(&self.work))?;
        }
        #[cfg(feature = "code-rule-profile")]
        self.db.context(json!({"scope":"relational","ruleId":self.code_rule.id,"clauseId":self.clause.id,"path":(self.visible.files.len()==1).then(||self.visible.files[0].path.as_str())}));
        let guard = Arc::clone(&self.work);
        self.db
            .progress_handler(
                64,
                Some(move || {
                    #[cfg(feature = "code-rule-profile")]
                    if super::profile::sql::explaining() {
                        return false;
                    }
                    guard
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                            left.checked_sub(1)
                        })
                        .is_err()
                        || super::budget::step_budget_exhausted()
                }),
            )
            .map_err(sql_error)
    }
    fn query(
        &self,
        query: &str,
        inputs: impl Iterator<Item = String>,
    ) -> (String, BTreeSet<String>) {
        let aliases = inputs
            .filter(|n| self.tables.contains(n))
            .map(|n| (n.clone(), self.aliases[&n].clone()))
            .collect::<BTreeMap<_, _>>();
        let tables = aliases.values().cloned().collect();
        (shared::resolve(query, &aliases), tables)
    }
    fn release(&mut self) -> Result<()> {
        #[cfg(feature = "code-rule-profile")]
        self.db.checkpoint().map_err(sql_error)?;
        self.tables.insert(self.plan.stages[self.next].name.clone());
        // Logical last-use permissions remain per consumer. Physical shared facts
        // survive until owner eviction; private bindings leave with this lease.
        for (name, _) in self
            .plan
            .last_uses
            .iter()
            .filter(|(_, last)| **last == self.next)
        {
            self.tables.remove(name);
        }
        self.next += 1;
        Ok(())
    }
    fn advance(&mut self) -> Result<Option<producer::Demand>> {
        self.activate()?;
        let _reader = producer::ReadOnly::enter();
        while let Some(logical) = self.plan.stages.get(self.next) {
            let stage = &self.physical_stages[self.next];
            if !self.needed.contains(&logical.name) || self.db.storage.borrow().ready(&stage.name) {
                self.release()?;
                continue;
            }
            #[cfg(feature = "code-rule-profile")]
            self.db.stage(&logical.name);
            let (_, allowed) = self.query(&logical.query, logical.inputs.iter().cloned());
            columns(&self.db, &stage.query, &allowed)?;
            if let Some(spec) = stage.reachability.as_ref().filter(|s| s.syntax.is_some()) {
                return Ok(Some(syntax::demand(
                    &self.db,
                    spec,
                    &self.syntax_trees,
                    &self.syntax_files,
                    Arc::clone(&self.work),
                )?));
            }
            #[cfg(feature = "code-rule-profile")]
            let operation = self
                .db
                .operation(&format!("/*RELATIONAL_STAGE*/ {}", logical.name), self.next);
            let created = create_stage(&self.db, stage, true, &self.work, &logical.name);
            #[cfg(feature = "code-rule-profile")]
            if let Err(error) = &created {
                self.db.explain_pending();
                if let Err(snapshot_error) = super::profile::sql::snapshot::capture(
                    &self.db,
                    json!({
                        "rule":self.code_rule.id,"clause":self.clause.id,"stage":logical.name,
                        "query":stage.query,"error":error.reason,
                        "budget":{"sqlRemaining":self.work.load(Ordering::Relaxed),"clause":super::budget::profile()},
                    }),
                ) {
                    eprintln!("SQL failure snapshot failed: {snapshot_error}");
                }
            }
            created
                .map_err(|e| invalid(format!("relational stage {}: {}", logical.name, e.reason)))?;
            if stage.reachability.is_none() && stage.deferred.is_empty() {
                self.db.storage.borrow_mut().publish(&stage.name);
            }
            #[cfg(feature = "code-rule-profile")]
            operation.into_iter().for_each(|op| op.finish_unobserved());
            self.release()?;
        }
        Ok(None)
    }
    fn publish(&mut self, output: producer::Completed) -> Result<()> {
        self.activate()?;
        let stage = &self.physical_stages[self.next];
        #[cfg(feature = "code-rule-profile")]
        self.db.stage(&self.plan.stages[self.next].name);
        #[cfg(feature = "code-rule-profile")]
        let operation = self.db.operation(
            &format!(
                "/*RELATIONAL_PUBLISH*/ {}",
                self.plan.stages[self.next].name
            ),
            self.next,
        );
        output.publish(&self.db, &stage.name, &self.work)?;
        self.db.module(stage.name.clone());
        #[cfg(feature = "code-rule-profile")]
        operation.into_iter().for_each(|op| op.finish_unobserved());
        self.release()
    }
    fn finish(&self) -> Result<Vec<Value>> {
        let result = self.finish_rows();
        #[cfg(feature = "code-rule-profile")]
        if let Err(error) = &result {
            let (query, _) = self.query(&self.plan.query, self.tables.iter().cloned());
            if let Err(snapshot_error) = super::profile::sql::snapshot::capture(
                &self.db,
                json!({
                    "rule":self.code_rule.id,"clause":self.clause.id,"stage":"result","query":query,"error":error.reason,
                    "budget":{"sqlRemaining":self.work.load(Ordering::Relaxed),"clause":super::budget::profile()},
                }),
            ) {
                eprintln!("SQL result failure snapshot failed: {snapshot_error}");
            }
        }
        // Explain the result while its private inputs still exist, including on failure.
        #[cfg(feature = "code-rule-profile")]
        if let Err(error) = self.db.checkpoint() {
            eprintln!("SQL result profile checkpoint failed: {error}");
        }
        result
    }
    fn finish_rows(&self) -> Result<Vec<Value>> {
        self.activate()?;
        #[cfg(feature = "code-rule-profile")]
        self.db.stage("result");
        let _reader = producer::ReadOnly::enter();
        let Self {
            code_rule,
            clause,
            plan,
            db,
            syntax_files,
            anchors,
            tables,
            ..
        } = self;
        #[cfg(feature = "code-rule-profile")]
        let _span = super::profile::trace::Span::start("sql-result", "query");
        let (query, allowed) = self.query(&plan.query, tables.iter().cloned());
        let names = columns(db, &query, &allowed)?;
        let mut statement = db.prepare(&query).map_err(sql_error)?;
        let mut query = statement.query([]).map_err(sql_error)?;
        let mut findings = Vec::new();
        let mut identities = BTreeSet::new();
        let mut syntax_ignores = HashMap::new();
        while let Some(row) = query.next().map_err(sql_error)? {
            if findings.len() >= code_rule.capabilities.limits.max_witnesses {
                return Err(invalid("relational witness budget exceeded"));
            }
            let values = names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    Ok((
                        name.clone(),
                        read_value(row.get_ref(i).map_err(sql_error)?)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            let path = values
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("relational path must be text"))?;
            let number = |name: &str| {
                values
                    .get(name)
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| {
                        invalid(format!("relational {name} must be a source byte offset"))
                    })
            };
            let (start, end) = (number("start_byte")?, number("end_byte")?);
            let (span, ignored) = anchors
                .get(path)
                .and_then(|by_span| by_span.get(&(start, end)))
                .copied()
                .or_else(|| {
                    syntax_files
                        .iter()
                        .find(|(p, _)| p == path)
                        .and_then(|(_, f)| f.span(start, end, &plan.syntax.as_ref()?.exclude_kinds))
                        .map(|s| (s, false))
                })
                .or_else(|| {
                    (start == 0 && end == 0 && plan.locations.contains(path))
                        .then_some((super::violations::Span::default(), false))
                })
                .ok_or_else(|| {
                    invalid("relational results must anchor to an input witness span")
                })?;
            let syntax_ignored = clause.violation.ignored_by.as_deref().is_some_and(|rule| {
                syntax_files
                    .iter()
                    .find(|(p, _)| p == path)
                    .is_some_and(|(path, facts)| {
                        syntax_ignores
                            .entry(path.clone())
                            .or_insert_with(|| facts.read_ignores())
                            .ignores(rule, start)
                    })
            });
            if ignored || syntax_ignored {
                continue;
            }
            let mut output_clause = (*clause).clone();
            if let Some(field) = &plan.code {
                output_clause.violation.code = values
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty() && text.len() <= 256)
                    .ok_or_else(|| {
                        invalid("relational code must be nonempty text of at most 256 bytes")
                    })?
                    .to_owned();
            }
            let identity = plan
                .key
                .iter()
                .map(|field| {
                    values
                        .get(field)
                        .ok_or_else(|| invalid("relational result lacks its key"))
                })
                .collect::<Result<Vec<_>>>()?;
            let witness = Witness {
                path: path.to_owned(),
                span,
                captures: Captures::default(),
                message: None,
                fix: None,
                ignored: false,
            };
            let mut finding = violation_from_witness(code_rule, &output_clause, witness);
            finding["id"] = json!(format!(
                "sha256:{}",
                hex_digest(format!("{}\0{}", finding["id"], json!(identity)).as_bytes())
            ));
            if !identities.insert(finding["id"].clone().to_string()) {
                return Err(invalid(
                    "relational results have duplicate identities; select distinct rows or provide a distinguishing key",
                ));
            }
            finding["variables"]["row"] = json!(values);
            findings.push(finding);
        }
        drop(query);
        drop(statement);
        #[cfg(feature = "code-rule-profile")]
        shared::capture(
            db,
            &code_rule.id,
            &clause.id,
            plan,
            &self.aliases,
            &self.physical_stages,
        )
        .map_err(sql_error)?;
        self.finished.set(true);
        Ok(findings)
    }
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        if !self.finished.get() {
            self.db.healthy.set(false);
        }
        let _ = self.db.progress_handler(0, None::<fn() -> bool>);
        for stage in &self.physical_stages[..self.next] {
            if stage
                .reachability
                .as_ref()
                .is_some_and(|r| r.syntax.is_none())
            {
                let _ = deferred::release(&self.db, &stage.name, &self.physical_stages);
            }
        }
    }
}

pub(super) fn evaluate(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    _plan: &Plan,
    rows: Vec<Rows>,
    snapshot: &Snapshot,
    versions: Versions,
) -> Evaluation {
    evaluate_batch(vec![Job {
        code_rule,
        clause,
        inputs: Inputs {
            versions: Some(versions),
            gate: None,
            file_local: false,
            rows: Some(rows),
            facts: Vec::new(),
            snapshot: snapshot.clone(),
        },
        budget: super::budget::Context::capture(),
    }])
    .pop()
    .unwrap_or_default()
}

/// SQL remains the consumer; capacity partitions bound suspended sessions.
pub(super) fn evaluate_batch(jobs: Vec<Job<'_>>) -> Vec<Evaluation> {
    batch::evaluate(jobs)
}

/// Restore documents only when the scheduler prepares this consumer.
pub(super) fn evaluate_prepared(
    jobs: Vec<Job<'_>>,
    prepare: impl FnMut(&mut Inputs, &CompiledCodeRule, &CompiledClause, Preparation) -> Result<()>,
) -> Vec<Evaluation> {
    batch::evaluate_prepared(jobs, prepare)
}
