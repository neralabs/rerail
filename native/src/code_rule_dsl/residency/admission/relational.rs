//! Bounded relational queries over native AST evidence. Policy queries read only
//! their input tables and earlier stages in a fresh in-memory SQLite connection.
use crate::code_rule_dsl::eval::{evidence::Rows, failure, violation_from_witness};
use crate::code_rule_dsl::json::{hex_digest, invalid};
use crate::code_rule_dsl::model::{CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Snapshot};
use crate::code_rule_dsl::violations::{Captures, Witness};
use napi::Result;
use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use rusqlite::hooks::Authorization;
mod authorizer;
pub(in crate::code_rule_dsl) mod compile;
mod pool;
mod storage;

mod admission;
mod anchors;
mod batch;
pub(in crate::code_rule_dsl) mod database;
use database::Database;
pub(in crate::code_rule_dsl) mod deferred;
mod forest;
mod inputs;
mod insert;
pub(in crate::code_rule_dsl) mod producer;
mod reachability;
pub(in crate::code_rule_dsl) mod schema;
mod source;
pub(in crate::code_rule_dsl) mod syntax;
mod table;
pub(in crate::code_rule_dsl) use reachability::Reachability;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Stage {
    pub(in crate::code_rule_dsl) name: String,
    pub(in crate::code_rule_dsl) query: String,
    pub(in crate::code_rule_dsl) indexes: Vec<Vec<String>>,
    pub(in crate::code_rule_dsl) reachability: Option<Reachability>,
    pub(in crate::code_rule_dsl) deferred: BTreeMap<String, String>,
    pub(in crate::code_rule_dsl) stored_columns: Vec<String>,
    pub(in crate::code_rule_dsl) output_columns: Vec<String>,
    pub(in crate::code_rule_dsl) declaration: Arc<str>,
}
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct References {
    pub(in crate::code_rule_dsl) query: crate::code_rule_dsl::model::Query,
    pub(in crate::code_rule_dsl) key: crate::code_rule_dsl::calls::contracts::Key,
}

/// Source bytes per syntax node, at most: Python averages 7.9 bytes a token over the data-layer
/// corpus (9,021 files), and a tree has more nodes than tokens.
const SOURCE_BYTES_PER_NODE: usize = 8;

pub(in crate::code_rule_dsl) const REFERENCE_FIELDS: &[&str] = &[
    "target_path",
    "target_start",
    "target_end",
    "source_name",
    "target_name",
    "target_class",
];

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Plan {
    pub(in crate::code_rule_dsl) require_nonempty: Option<usize>,
    pub(in crate::code_rule_dsl) syntax: Option<Arc<syntax::Spec>>,
    pub(in crate::code_rule_dsl) per_file: bool,
    pub(in crate::code_rule_dsl) references: Option<References>,
    pub(in crate::code_rule_dsl) locations: BTreeSet<String>,
    pub(in crate::code_rule_dsl) inputs: Vec<(String, FactQuery)>,
    pub(in crate::code_rule_dsl) stages: Vec<Stage>,
    pub(in crate::code_rule_dsl) query: String,
    pub(in crate::code_rule_dsl) key: Vec<String>,
    pub(in crate::code_rule_dsl) code: Option<String>,
    pub(in crate::code_rule_dsl) last_uses: BTreeMap<String, usize>,
    pub(in crate::code_rule_dsl) reads_source: bool,
}

pub(in crate::code_rule_dsl) fn identifier(value: &str) -> bool {
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
fn sql_error(error: rusqlite::Error) -> napi::Error {
    invalid(format!("relational query: {error}"))
}

pub(in crate::code_rule_dsl) fn connection(max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Database> {
    let db = validation_connection(max_pattern, work)?;
    db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA analysis_limit=1000;")
        .map_err(sql_error)?;
    Ok(db)
}

/// Compiler sessions prepare authored queries and retain only empty stage schemas.
/// Runtime storage configuration belongs to `connection`, not schema validation.
pub(in crate::code_rule_dsl) fn validation_connection(
    max_pattern: usize,
    work: Arc<AtomicUsize>,
) -> Result<Database> {
    configure(Database::open().map_err(sql_error)?, max_pattern, work)
}
fn configure(db: Database, max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Database> {
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
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    for (name, arity) in [("rx_matches", 2), ("rx_extract", 3), ("rx_replace", 3)] {
        let cache = parking_lot::Mutex::new(HashMap::<String, regex::Regex>::new());
        let work = Arc::clone(&work);
        db.create_scalar_function(name, arity, flags, move |ctx| {
            let text = ctx.get::<Option<String>>(0)?.unwrap_or_default();
            let pattern = ctx.get::<String>(1)?;
            let fail =
                |message: &str| rusqlite::Error::UserFunctionError(message.to_owned().into());
            let cost = text.len().div_ceil(256).max(1);
            if pattern.len() > max_pattern
                || work
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(cost)
                    })
                    .is_err()
            {
                return Err(fail("regular expression work or pattern budget exceeded"));
            }
            let mut cache = cache.lock();
            if !cache.contains_key(&pattern) {
                let compiled =
                    regex::Regex::new(&pattern).map_err(|error| fail(&error.to_string()))?;
                if cache.len() >= 64 {
                    cache.clear();
                }
                cache.insert(pattern.clone(), compiled);
            }
            let regex = &cache[&pattern];
            match name {
                "rx_matches" => Ok(rusqlite::types::Value::Integer(i64::from(
                    regex.is_match(&text),
                ))),
                "rx_extract" => {
                    let group = usize::try_from(ctx.get::<i64>(2)?)
                        .map_err(|_| fail("regex group must be nonnegative"))?;
                    if group >= regex.captures_len() {
                        return Err(fail("unknown regex group"));
                    }
                    Ok(regex
                        .captures(&text)
                        .and_then(|matched| {
                            matched
                                .get(group)
                                .map(|part| rusqlite::types::Value::Text(part.as_str().to_owned()))
                        })
                        .unwrap_or(rusqlite::types::Value::Null))
                }
                _ => {
                    let by: String = ctx.get(2)?;
                    if by.len() > max_pattern {
                        return Err(fail("replacement exceeds pattern budget"));
                    }
                    Ok(rusqlite::types::Value::Text(
                        regex.replace_all(&text, by.as_str()).into_owned(),
                    ))
                }
            }
        })
        .map_err(sql_error)?;
    }
    source::install(&db, None, work)?;
    Ok(db)
}

/// Preparation checks the complete statement, including its function calls.
/// Neither attached databases, metadata, pragmas, writes nor nondeterministic
/// functions are available to a policy. CTEs and JSON table functions can join
/// only the finite relations supplied here.
pub(in crate::code_rule_dsl) fn columns(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<Vec<String>> {
    inspect(db, query, tables).map(|(columns, _, _)| columns)
}

pub(in crate::code_rule_dsl) fn inspect(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<(Vec<String>, BTreeSet<String>, bool)> {
    inspect_fields(db, query, tables)
        .map(|(columns, fields, source)| (columns, fields.into_keys().collect(), source))
}
pub(in crate::code_rule_dsl) type FieldReads = BTreeMap<String, BTreeSet<String>>;
pub(in crate::code_rule_dsl) fn inspect_fields(
    db: &Connection,
    query: &str,
    tables: &BTreeSet<String>,
) -> Result<(Vec<String>, FieldReads, bool)> {
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
        ))
    })();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)
        .map_err(sql_error)?;
    result
}

pub(in crate::code_rule_dsl) fn create_input(db: &Database, name: &str, fields: &[String]) -> Result<()> {
    db.storage
        .borrow_mut()
        .declare(db, name, input_schema(name, fields), true)
        .map_err(sql_error)?;
    Ok(())
}
pub(in crate::code_rule_dsl) fn input_schema(name: &str, fields: &[String]) -> String {
    format!(
        "CREATE TABLE {} (__path TEXT NOT NULL, __start INTEGER NOT NULL, __end INTEGER NOT NULL{})",
        quote(name),
        fields
            .iter()
            .map(|field| format!(",{}", quote(field)))
            .collect::<String>()
    )
}
pub(in crate::code_rule_dsl) fn create_stage(
    db: &Database,
    stage: &Stage,
    materialize: bool,
    work: &Arc<AtomicUsize>,
) -> Result<()> {
    if materialize && let Some(spec) = &stage.reachability {
        return reachability::create(db, stage, spec, work);
    }
    #[cfg(feature = "code-rule-profile")]
    if materialize && std::env::var_os("RERAIL_SQL_QUERY_PLANS").is_some() {
        let mut span = crate::code_rule_dsl::profile::trace::Span::start("sql-plan", &stage.name);
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
    let mut span = crate::code_rule_dsl::profile::trace::Span::start("sql-materialize", &stage.name);
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
            let _span = crate::code_rule_dsl::profile::trace::Span::start("sql-index", &stage.name);
            // Index names are global: name them after the physical table, which a stage that
            // defers columns (`__stage_<name>`) and one that defers none (`<name>`) do not share.
            let name = format!("__{}_{}", physical, index);
            db.storage
                .borrow_mut()
                .index(
                    db,
                    &physical,
                    &name,
                    format!(
                        "CREATE INDEX {} ON {} ({})",
                        quote(&name),
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
        let _span = crate::code_rule_dsl::profile::trace::Span::start("sql-analyze", &stage.name);
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

pub(in crate::code_rule_dsl) struct Job<'a> {
    pub code_rule: &'a CompiledCodeRule,
    pub clause: &'a CompiledClause,
    pub inputs: Inputs,
    pub budget: crate::code_rule_dsl::budget::Context,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Preparation {
    Gate,
    Body,
}

pub(in crate::code_rule_dsl) struct Inputs {
    pub gate: Option<Rows>,
    pub rows: Option<Vec<Rows>>,
    pub facts: syntax::Files,
    pub snapshot: Snapshot,
    pub file_local: bool,
    /// Source bytes of the files the clause reads, where its snapshot does not carry them (a
    /// repository clause that reads no source has an empty snapshot); 0 means the snapshot's.
    pub read_bytes: usize,
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
}
impl<'a> Session<'a> {
    fn new(
        code_rule: &'a CompiledCodeRule,
        clause: &'a CompiledClause,
        mut inputs: Inputs,
        metrics: &mut crate::code_rule_dsl::model::Metrics,
        pool: &pool::Pool,
    ) -> Result<Self> {
        let snapshot = &inputs.snapshot;
        let supplied = inputs.facts;
        let limits = &code_rule.capabilities.limits;
        admission::limits(code_rule, snapshot)?;
        let crate::code_rule_dsl::model::ClauseKind::Relational(plan) = &clause.kind else {
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
                    crate::code_rule_dsl::eval::stateless_calls(
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
                        create_input(&db, name, &fields)?;
                        let before = db.total_changes();
                        let rows = inputs::stream(
                            &db, name, input, &work, code_rule, clause, snapshot, metrics,
                        )?;
                        db.storage
                            .borrow_mut()
                            .written(name, db.total_changes() != before);
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
        // The stages' work grows with the code they read, not with how few rows a rule asks for:
        // a rule that projects narrow candidate rows instead of copying the tree joins as much.
        // So the allowance is per node of the scoped source (estimated at no fewer than one per
        // `SOURCE_BYTES_PER_NODE` bytes) when that is more than per input row.
        let read = snapshot
            .files
            .iter()
            .filter(|f| code_rule.scope.matches(&f.path))
            .map(|f| f.source.len())
            .sum::<usize>()
            .max(inputs.read_bytes)
            / SOURCE_BYTES_PER_NODE;
        let spent = usize::MAX - work.load(Ordering::Relaxed);
        let allowance = limits.visits_for(count.max(read));
        if spent > allowance {
            return Err(invalid("relational input row budget exceeded"));
        }
        work.store(allowance - spent, Ordering::Relaxed);
        let anchors = anchors::collect(plan, &rows);
        if plan.reads_source {
            let visible = Snapshot {
                revision: snapshot.revision,
                files: snapshot
                    .files
                    .iter()
                    .filter(|file| code_rule.scope.matches(&file.path))
                    .cloned()
                    .collect(),
            };
            source::install(&db, Some(&visible), Arc::clone(&work))?;
        }
        let mut tables = BTreeSet::new();
        if let Some(spec) = &plan.syntax {
            for name in std::iter::once(&spec.name)
                .chain(spec.edges.iter())
                .chain(spec.ownership.iter())
            {
                db.storage
                    .borrow_mut()
                    .discard(&db, name)
                    .map_err(sql_error)?;
            }
            tables.extend(syntax::load(&db, spec, &syntax_files, Arc::clone(&work))?);
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
            create_input(&db, name, &fields)?;
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
                name,
                fields.len() + 3,
                rows.witnesses.iter().map(|witness| {
                    use rusqlite::types::ValueRef;
                    let location = crate::code_rule_dsl::violations::anchored_span(&input.anchor, witness).0;
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
                .written(name, db.total_changes() != before);
            tables.insert(name.clone());
            // SQL owns the copied values; only compact anchors remain necessary.
        }
        if let Some(reference) = &plan.references {
            let graph = crate::code_rule_dsl::calls::reference_rows(reference.key)
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
                "reference_edges",
                &REFERENCE_FIELDS
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect::<Vec<_>>(),
            )?;
            let before = db.total_changes();
            insert::insert_rows(
                &db,
                "reference_edges",
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
                                edge.target.flags & crate::code_rule_dsl::calls::summary::CLASS != 0,
                            )),
                        ]
                    }),
                &work,
                "native reference row budget exceeded",
            )?;
            db.storage
                .borrow_mut()
                .written("reference_edges", db.total_changes() != before);
            tables.insert("reference_edges".to_owned());
        }
        db.execute_batch("COMMIT").map_err(sql_error)?;
        let guard = Arc::clone(&work);
        // One relational work unit is one input row, regex chunk, or bounded
        // batch of 64 SQLite VM instructions. The shared allowance also polls
        // each batch; SQL opcodes are not AST node visits.
        db.progress_handler(
            64,
            Some(move || {
                #[cfg(feature = "code-rule-profile")]
                if crate::code_rule_dsl::profile::sql::explaining() {
                    return false;
                }
                guard
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_err()
                    || crate::code_rule_dsl::budget::step_budget_exhausted()
            }),
        )
        .map_err(sql_error)?;

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
        })
    }
    fn release(&mut self) -> Result<()> {
        // Explain before the last-use release removes the query's inputs.
        #[cfg(feature = "code-rule-profile")]
        self.db.checkpoint().map_err(sql_error)?;
        let stage = &self.plan.stages[self.next];
        self.tables.insert(stage.name.clone());
        for (name, _) in self
            .plan
            .last_uses
            .iter()
            .filter(|(_, last)| **last == self.next)
        {
            release_owned(&self.db, name, &self.plan.stages)?;
            self.tables.remove(name);
        }
        self.next += 1;
        Ok(())
    }
    fn advance(&mut self) -> Result<Option<producer::Demand>> {
        let _reader = producer::ReadOnly::enter();
        while let Some(stage) = self.plan.stages.get(self.next) {
            #[cfg(feature = "code-rule-profile")]
            self.db.stage(&stage.name);
            columns(&self.db, &stage.query, &self.tables)?;
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
                .operation(&format!("/*RELATIONAL_STAGE*/ {}", stage.name), self.next);
            create_stage(&self.db, stage, true, &self.work)
                .map_err(|e| invalid(format!("relational stage {}: {}", stage.name, e.reason)))?;
            #[cfg(feature = "code-rule-profile")]
            operation
                .into_iter()
                .for_each(|operation| operation.finish_unobserved());
            self.release()?;
        }
        Ok(None)
    }
    fn publish(&mut self, output: producer::Completed) -> Result<()> {
        let _reader = producer::ReadOnly::enter();
        let stage = &self.plan.stages[self.next];
        #[cfg(feature = "code-rule-profile")]
        self.db.stage(&stage.name);
        #[cfg(feature = "code-rule-profile")]
        let operation = self
            .db
            .operation(&format!("/*RELATIONAL_PUBLISH*/ {}", stage.name), self.next);
        self.db
            .storage
            .borrow_mut()
            .discard(&self.db, &stage.name)
            .map_err(sql_error)?;
        output.publish(&self.db, &stage.name, &self.work)?;
        #[cfg(feature = "code-rule-profile")]
        operation
            .into_iter()
            .for_each(|operation| operation.finish_unobserved());
        self.release()
    }
    fn finish(&self) -> Result<Vec<Value>> {
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
        let _span = crate::code_rule_dsl::profile::trace::Span::start("sql-result", "query");
        let names = columns(db, &plan.query, tables)?;
        let mut statement = db.prepare(&plan.query).map_err(sql_error)?;
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
                        .then_some((crate::code_rule_dsl::violations::Span::default(), false))
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
                sql_violation: None,
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
        self.finished.set(true);
        Ok(findings)
    }
}
fn release_owned(db: &Database, name: &str, stages: &[Stage]) -> Result<()> {
    if let Some(stage) = stages.iter().find(|s| s.name == name)
        && !stage.deferred.is_empty()
    {
        db.storage
            .borrow_mut()
            .retire(db, &deferred::physical_name(stage))
            .map_err(sql_error)?;
        return Ok(());
    }
    if db
        .storage
        .borrow_mut()
        .retire(db, name)
        .map_err(sql_error)?
    {
        return Ok(());
    }
    deferred::release(db, name, stages)
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        if !self.finished.get() {
            return;
        }
        if self.db.progress_handler(0, None::<fn() -> bool>).is_err() {
            return;
        }
        #[cfg(feature = "code-rule-profile")]
        if self.db.checkpoint().is_err() {
            return;
        }
        for name in &self.tables {
            if release_owned(&self.db, name, &self.plan.stages).is_err() {
                return;
            }
        }
        self.db.reusable.set(true);
    }
}

pub(in crate::code_rule_dsl) fn evaluate(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    _plan: &Plan,
    rows: Vec<Rows>,
    snapshot: &Snapshot,
) -> Evaluation {
    evaluate_batch(vec![Job {
        code_rule,
        clause,
        inputs: Inputs {
            gate: None,
            file_local: false,
            read_bytes: 0,
            rows: Some(rows),
            facts: Vec::new(),
            snapshot: snapshot.clone(),
        },
        budget: crate::code_rule_dsl::budget::Context::capture(),
    }])
    .pop()
    .unwrap_or_default()
}

/// SQL remains the consumer; capacity partitions bound suspended sessions.
pub(in crate::code_rule_dsl) fn evaluate_batch(jobs: Vec<Job<'_>>) -> Vec<Evaluation> {
    use crate::code_rule_dsl::phase_clock;
    phase_clock::timed(&phase_clock::RELATIONAL, || batch::evaluate(jobs))
}

/// Restore documents only when the scheduler prepares this consumer.
pub(in crate::code_rule_dsl) fn evaluate_prepared(
    jobs: Vec<Job<'_>>,
    prepare: impl FnMut(&mut Inputs, &CompiledCodeRule, &CompiledClause, Preparation) -> Result<()>,
) -> Vec<Evaluation> {
    use crate::code_rule_dsl::phase_clock;
    phase_clock::timed(&phase_clock::RELATIONAL, || batch::evaluate_prepared(jobs, prepare))
}
