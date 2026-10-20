//! Bounded relational queries over native AST evidence. Policy queries read only
//! their input tables and earlier stages in a fresh in-memory SQLite connection.
use super::eval::{evidence::Rows, failure, violation_from_witness};
use super::json::{hex_digest, invalid};
use super::model::{CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Snapshot};
use super::violations::{Captures, Witness};
use napi::Result;
use rusqlite::functions::FunctionFlags;
use rusqlite::hooks::{AuthAction, Authorization};
use rusqlite::{Connection, params_from_iter};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
pub(super) struct Stage {
    pub(super) name: String,
    pub(super) query: String,
    pub(super) indexes: Vec<Vec<String>>,
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
    pub(super) references: Option<References>,
    pub(super) inputs: Vec<(String, FactQuery)>,
    pub(super) stages: Vec<Stage>,
    pub(super) query: String,
    pub(super) key: Vec<String>,
    pub(super) code: Option<String>,
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
fn sql_error(error: rusqlite::Error) -> napi::Error {
    invalid(format!("relational query: {error}"))
}

pub(super) fn connection(max_pattern: usize, work: Arc<AtomicUsize>) -> Result<Connection> {
    let db = Connection::open_in_memory().map_err(sql_error)?;
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
    db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA analysis_limit=1000;")
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
    Ok(db)
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
    let tables = tables.clone();
    db.authorizer(Some(
        move |context: rusqlite::hooks::AuthContext<'_>| match context.action {
            AuthAction::Select | AuthAction::Recursive => Authorization::Allow,
            AuthAction::Read { table_name, .. }
                if tables.contains(table_name)
                    || matches!(table_name, "json_each" | "json_tree") =>
            {
                Authorization::Allow
            }
            AuthAction::Function { function_name }
                if matches!(
                    function_name.to_ascii_lowercase().as_str(),
                    "abs"
                        | "char"
                        | "cast"
                        | "coalesce"
                        | "count"
                        | "dense_rank"
                        | "group_concat"
                        | "hex"
                        | "ifnull"
                        | "iif"
                        | "instr"
                        | "json"
                        | "json_array"
                        | "json_array_length"
                        | "json_each"
                        | "json_extract"
                        | "json_group_array"
                        | "json_group_object"
                        | "json_object"
                        | "json_quote"
                        | "json_tree"
                        | "json_type"
                        | "json_valid"
                        | "length"
                        | "like"
                        | "glob"
                        | "lower"
                        | "ltrim"
                        | "max"
                        | "min"
                        | "nullif"
                        | "printf"
                        | "quote"
                        | "rank"
                        | "replace"
                        | "row_number"
                        | "rtrim"
                        | "substr"
                        | "substring"
                        | "sum"
                        | "total"
                        | "trim"
                        | "typeof"
                        | "unicode"
                        | "upper"
                        | "rx_matches"
                        | "rx_extract"
                        | "rx_replace"
                ) =>
            {
                Authorization::Allow
            }
            _ => Authorization::Deny,
        },
    ))
    .map_err(sql_error)?;
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
        Ok(columns)
    })();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)
        .map_err(sql_error)?;
    result
}

pub(super) fn create_input(db: &Connection, name: &str, fields: &[String]) -> Result<()> {
    db.execute(&format!("CREATE TABLE {} (__path TEXT NOT NULL, __start INTEGER NOT NULL, __end INTEGER NOT NULL{})",quote(name),fields.iter().map(|field|format!(",{}",quote(field))).collect::<String>()),[]).map_err(sql_error)?;
    Ok(())
}
pub(super) fn create_stage(db: &Connection, stage: &Stage, materialize: bool) -> Result<()> {
    db.execute(
        &format!(
            "CREATE TEMP {} {} AS {}",
            if materialize { "TABLE" } else { "VIEW" },
            quote(&stage.name),
            stage.query
        ),
        [],
    )
    .map_err(sql_error)?;
    if materialize {
        for (index, fields) in stage.indexes.iter().enumerate() {
            db.execute(
                &format!(
                    "CREATE INDEX {} ON {} ({})",
                    quote(&format!("__{}_{}", stage.name, index)),
                    quote(&stage.name),
                    fields
                        .iter()
                        .map(|field| quote(field))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                [],
            )
            .map_err(sql_error)?;
        }
        if !stage.indexes.is_empty() {
            db.execute(&format!("ANALYZE temp.{}", quote(&stage.name)), [])
                .map_err(sql_error)?;
        }
    }
    Ok(())
}
fn sql_value(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match value {
        Value::Null => Sql::Null,
        Value::Bool(value) => Sql::Integer(i64::from(*value)),
        Value::Number(value) => value
            .as_i64()
            .map(Sql::Integer)
            .unwrap_or_else(|| Sql::Real(value.as_f64().unwrap_or_default())),
        Value::String(value) => Sql::Text(value.clone()),
        _ => Sql::Text(value.to_string()),
    }
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

pub(super) fn evaluate(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    plan: &Plan,
    rows: Vec<Rows>,
    _snapshot: &Snapshot,
) -> Evaluation {
    let mut result = Evaluation::default();
    if let Some(path) = rows.iter().flat_map(|rows| rows.incomplete.iter()).next() {
        result.failures.push(failure(
            &clause.id,
            "source-incomplete",
            "A relational input is incomplete",
            Some(path),
        ));
        return result;
    }
    let count = rows.iter().map(|rows| rows.witnesses.len()).sum();
    let work = Arc::new(AtomicUsize::new(
        code_rule.capabilities.limits.visits_for(count),
    ));
    let run = || -> Result<Vec<Value>> {
        let db = connection(
            code_rule.capabilities.limits.max_regex_pattern_bytes,
            Arc::clone(&work),
        )?;
        db.execute_batch("BEGIN").map_err(sql_error)?;
        let mut tables = BTreeSet::new();
        for ((name, input), rows) in plan.inputs.iter().zip(&rows) {
            #[cfg(feature = "code-rule-profile")]
            if std::env::var_os("RERAIL_CODE_RULE_RELATIONAL_PROFILE").is_some() {
                eprintln!(
                    "RERAIL_CODE_RULE_RELATIONAL_INPUT {}",
                    json!({"table":name,"rows":rows.witnesses.len()})
                );
            }
            let fields = input
                .ast
                .project
                .iter()
                .map(|project| project.field.clone())
                .collect::<Vec<_>>();
            create_input(&db, name, &fields)?;
            let sql = format!(
                "INSERT INTO {} VALUES ({})",
                quote(name),
                vec!["?"; fields.len() + 3].join(",")
            );
            let mut statement = db.prepare(&sql).map_err(sql_error)?;
            for witness in &rows.witnesses {
                if work
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_err()
                    || super::budget::step_budget_exhausted()
                {
                    return Err(invalid("relational input row budget exceeded"));
                }
                let projected = witness.captures.projection().and_then(Value::as_object);
                let mut values = vec![
                    rusqlite::types::Value::Text(witness.path.clone()),
                    rusqlite::types::Value::Integer(witness.span.start_byte as i64),
                    rusqlite::types::Value::Integer(witness.span.end_byte as i64),
                ];
                values.extend(fields.iter().map(|field| {
                    sql_value(
                        projected
                            .and_then(|fields| fields.get(field))
                            .unwrap_or(&Value::Null),
                    )
                }));
                statement
                    .execute(params_from_iter(values))
                    .map_err(sql_error)?;
            }
            tables.insert(name.clone());
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
                "reference_edges",
                &REFERENCE_FIELDS
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect::<Vec<_>>(),
            )?;
            let mut statement = db
                .prepare("INSERT INTO reference_edges VALUES (?,?,?,?,?,?,?,?,?)")
                .map_err(sql_error)?;
            for edge in graph.edges {
                if !code_rule.scope.matches(&edge.path) {
                    continue;
                }
                if work
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_err()
                    || super::budget::step_budget_exhausted()
                {
                    return Err(invalid("native reference row budget exceeded"));
                }
                statement
                    .execute(rusqlite::params![
                        edge.path.as_ref(),
                        edge.source.start,
                        edge.source.end,
                        edge.target_path.as_ref(),
                        edge.target.start,
                        edge.target.end,
                        edge.source_name.text(),
                        edge.target_name.text(),
                        i64::from(edge.target.flags & super::calls::summary::CLASS != 0)
                    ])
                    .map_err(sql_error)?;
            }
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
                guard
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_err()
                    || super::budget::step_budget_exhausted()
            }),
        )
        .map_err(sql_error)?;
        for stage in &plan.stages {
            columns(&db, &stage.query, &tables)?;
            #[cfg(feature = "code-rule-profile")]
            let started = std::time::Instant::now();
            let materialized = create_stage(&db, stage, true);
            #[cfg(feature = "code-rule-profile")]
            if std::env::var_os("RERAIL_CODE_RULE_RELATIONAL_PROFILE").is_some() {
                eprintln!(
                    "RERAIL_CODE_RULE_RELATIONAL {}",
                    json!({"stage":stage.name,"elapsedMs":started.elapsed().as_secs_f64()*1000.0,"workLeft":work.load(Ordering::Relaxed),"failed":materialized.is_err()})
                );
            }
            materialized.map_err(|error| {
                invalid(format!("relational stage {}: {}", stage.name, error.reason))
            })?;
            tables.insert(stage.name.clone());
        }
        let names = columns(&db, &plan.query, &tables)?;
        let mut statement = db.prepare(&plan.query).map_err(sql_error)?;
        let mut query = statement.query([]).map_err(sql_error)?;
        let anchors = rows
            .iter()
            .flat_map(|rows| rows.witnesses.iter())
            .map(|witness| {
                (
                    (
                        witness.path.as_str(),
                        witness.span.start_byte,
                        witness.span.end_byte,
                    ),
                    (witness.span, witness.ignored),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut findings = Vec::new();
        let mut identities = BTreeSet::new();
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
            let (span, ignored) = *anchors.get(&(path, start, end)).ok_or_else(|| {
                invalid("relational results must anchor to an input witness span")
            })?;
            if ignored {
                continue;
            }
            let mut output_clause = clause.clone();
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
        Ok(findings)
    };
    match run() {
        Ok(findings) => {
            result.metrics.witnesses = findings.len();
            result.violations = findings;
        }
        Err(error) => result.failures.push(failure(
            &clause.id,
            "relational-failed",
            &error.reason,
            None,
        )),
    }
    result
}
