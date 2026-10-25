//! Test-only physical cuts. Every CTAS/index execution is part of matching cost.
//! Explain runs between stages; later stages are not prepared before their inputs exist.
use super::tests::ExecutionTrace;
use crate::code_rule_dsl::sql::timing::{Started, Timing};
use rusqlite::{Connection, StatementStatus};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct Report {
    pub(super) executions: usize,
    pub(super) vm: i64,
    pub(super) automatic_index: i64,
    pub(super) fullscan: i64,
    pub(super) sorts: i64,
    pub(super) prepare: Timing,
    pub(super) execution: Timing,
    records: Vec<Value>,
    sql_bytes: usize,
    final_bytes: usize,
}
impl Report {
    pub(super) fn active(&self) -> bool {
        !self.records.is_empty()
    }
    pub(super) fn expected_statements(&self) -> usize {
        self.records.len() + 1
    }
    pub(super) fn profile(&self) -> Value {
        json!({"statements":self.records,"stageExecutions":self.executions,"expectedMatchingStatements":self.expected_statements(),"vmSteps":self.vm,"automaticIndexRows":self.automatic_index,"fullscanSteps":self.fullscan,"sorts":self.sorts,"prepareCpuNs":self.prepare.cpu,"prepareWallNs":self.prepare.wall,"executionCpuNs":self.execution.cpu,"executionWallNs":self.execution.wall,"stageSqlBytes":self.sql_bytes,"finalSqlBytes":self.final_bytes,"totalSqlBytes":self.sql_bytes+self.final_bytes,"perStatementSqlByteLimit":256000,"matchingStatementLimit":100,"explainIncludedInExecution":false})
    }
}

// Skip literals, quoted identifiers and comments. These words validate only the
// permitted statement class; SQLite still parses and validates each statement.
fn words(sql: &str) -> Result<Vec<String>, String> {
    let b = sql.as_bytes();
    let mut i = 0;
    let mut out = vec![];
    let mut ended = false;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"--") {
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i += 2;
            while i + 1 < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            if i + 1 == b.len() {
                return Err("spike stages: unterminated SQL comment".into());
            }
            i += 2;
            continue;
        }
        if ended {
            return Err("spike stages: each stage must contain exactly one statement".into());
        }
        if b[i] == b';' {
            ended = true;
            i += 1;
            continue;
        }
        if matches!(b[i], b'\'' | b'"' | b'`' | b'[') {
            let close = if b[i] == b'[' { b']' } else { b[i] };
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if b[i] == close {
                    i += 1;
                    if i < b.len() && b[i] == close {
                        i += 1;
                        continue;
                    }
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err("spike stages: unterminated SQL quote".into());
            }
            continue;
        }
        if b[i].is_ascii_alphanumeric() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(sql[start..i].to_ascii_uppercase());
        } else {
            i += 1;
        }
    }
    Ok(out)
}
fn validate(sql: &str) -> Result<(), String> {
    if sql.len() > 256000 {
        return Err("spike stages: statement exceeds 256000 SQL bytes".into());
    }
    let words = words(sql)?;
    let mut w = words.iter().map(String::as_str);
    if w.next() != Some("CREATE") {
        return Err("spike stages: only CTAS and CREATE INDEX physical cuts are permitted".into());
    }
    let mut kind = w.next();
    if matches!(kind, Some("TEMP" | "TEMPORARY")) {
        kind = w.next();
    }
    let okay = match kind {
        Some("TABLE") => words
            .windows(2)
            .any(|p| p[0] == "AS" && matches!(p[1].as_str(), "SELECT" | "WITH" | "VALUES")),
        Some("INDEX") => w.any(|word| word == "ON"),
        Some("UNIQUE") => w.next() == Some("INDEX") && w.any(|word| word == "ON"),
        _ => false,
    };
    if okay {
        Ok(())
    } else {
        Err("spike stages: expected CREATE TABLE AS query or CREATE INDEX".into())
    }
}

fn purpose(relations: &Value, ordinal: usize) -> Option<&str> {
    relations
        .get("spikeStagePurposes")
        .and_then(|v| v.get(ordinal))
        .and_then(Value::as_str)
        .or_else(|| relations.get("spikePurpose").and_then(Value::as_str))
}
fn label(relations: &Value, ordinal: usize) -> String {
    purpose(relations, ordinal).map_or_else(
        || format!("spike stage {ordinal}"),
        |p| format!("spike stage {ordinal} ({p})"),
    )
}

pub(super) fn run(db: &Connection, relations: &Value, final_sql: &str) -> Result<Report, String> {
    super::scan::configure(db, true).map_err(|e| e.to_string())?;
    let physical = relations
        .get("physicalStages")
        .and_then(Value::as_array)
        .is_some_and(|s| !s.is_empty());
    let Some(stages) = (if physical {
        relations.get("physicalStages")
    } else {
        relations.get("spikeStages")
    }) else {
        return Ok(Report::default());
    };
    let stages = stages
        .as_array()
        .ok_or("spike stages: spikeStages must be an array of SQL strings")?;
    if stages.is_empty() {
        return Ok(Report::default());
    }
    if stages.len() + 1 > 100 {
        return Err("spike stages: matching statement count exceeds 100".into());
    }
    if final_sql.len() > 256000 {
        return Err("spike stages: final statement exceeds 256000 SQL bytes".into());
    }
    let stages = stages
        .iter()
        .enumerate()
        .map(|(ordinal, value)| {
            let label = label(relations, ordinal);
            let sql = value
                .as_str()
                .ok_or_else(|| format!("{label} admission: stage must be a SQL string"))?;
            if physical {
                if sql.len() > 256000 {
                    return Err(format!("{label}: statement exceeds256000 bytes"));
                }
                let words = words(sql)?;
                let okay = words.starts_with(&[
                    "CREATE".into(),
                    "TEMP".into(),
                    "TABLE".into(),
                    "IF".into(),
                    "NOT".into(),
                    "EXISTS".into(),
                ]) || words.starts_with(&["DELETE".into(), "FROM".into()])
                    || words.starts_with(&["INSERT".into(), "INTO".into()]);
                if !okay {
                    return Err(format!("{label}: invalid common physical statement"));
                }
            } else {
                validate(sql).map_err(|error| format!("{label} admission: {error}"))?;
            }
            Ok(sql)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut report = Report {
        final_bytes: final_sql.len(),
        ..Default::default()
    };
    for (ordinal, sql) in stages.into_iter().enumerate() {
        let label = label(relations, ordinal);
        // Explain is deliberately before this execution, after earlier CTASes.
        let explain = db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .map_err(|e| format!("{label} explain prepare: {e}"))?
            .query_map([], |r| {
                Ok(json!([
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(3)?
                ]))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let started = Started::now();
        let mut stmt = db
            .prepare(sql)
            .map_err(|e| format!("{label} prepare: {e}"))?;
        let prepare = started.elapsed();
        let trace = ExecutionTrace::start(db);
        let started = Started::now();
        stmt.execute([])
            .map_err(|e| format!("{label} execution: {e}"))?;
        let execution = started.elapsed();
        let executions = trace.count();
        let scan = trace.scan();
        drop(trace);
        let vm = i64::from(stmt.get_status(StatementStatus::VmStep));
        let automatic_index = i64::from(stmt.get_status(StatementStatus::AutoIndex));
        let fullscan = i64::from(stmt.get_status(StatementStatus::FullscanStep));
        let sorts = i64::from(stmt.get_status(StatementStatus::Sort));
        report.executions += executions;
        report.vm += vm;
        report.automatic_index += automatic_index;
        report.fullscan += fullscan;
        report.sorts += sorts;
        report.prepare.add(prepare);
        report.execution.add(execution);
        report.sql_bytes += sql.len();
        // Diagnostic cardinality is separate from measured matching execution.
        // Only declared stage tables are queried; never accept arbitrary SQL here.
        let output = relations
            .get("spikeStageOutputs")
            .and_then(|outputs| outputs.get(ordinal))
            .and_then(Value::as_str);
        let output_rows = if let Some(table) = output {
            if table.is_empty()
                || !table
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                return Err(format!("{label}: invalid diagnostic output table"));
            }
            Some(
                db.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(|e| format!("{label} diagnostic cardinality: {e}"))?,
            )
        } else {
            None
        };
        report.records.push(json!({"scan":scan,"outputTable":output,"outputRows":output_rows,"cardinalityIncludedInExecution":false,"ordinal":ordinal,"purpose":purpose(relations,ordinal),"sql":sql,"sqlBytes":sql.len(),"explain":explain,"matchingStatements":executions,"vmSteps":vm,"automaticIndexRows":automatic_index,"fullscanSteps":fullscan,"sorts":sorts,"prepareCpuNs":prepare.cpu,"prepareWallNs":prepare.wall,"executionCpuNs":execution.cpu,"executionWallNs":execution.wall}));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage_dependencies_prepare_after_prior_execution_and_all_work_is_counted() {
        let db = Connection::open_in_memory().unwrap();
        let stages = json!({"spikeStages":["CREATE TEMP TABLE cut AS WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<20) SELECT x FROM n","CREATE UNIQUE INDEX cut_x ON cut(x)","CREATE TEMP TABLE next_cut AS SELECT x FROM cut WHERE x>10"],"spikeStageOutputs":["cut",null,"next_cut"]});
        let r = run(&db, &stages, "SELECT SUM(x) FROM next_cut").unwrap();
        assert_eq!(r.executions, 3);
        assert_eq!(r.records[0]["outputRows"], 20);
        assert_eq!(r.records[2]["outputRows"], 10);
        assert_eq!(r.expected_statements(), 4);
        assert!(r.vm > 0);
        assert_eq!(
            db.query_row("SELECT SUM(x) FROM next_cut", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            155
        );
        assert_eq!(
            r.vm,
            r.records
                .iter()
                .map(|r| r["vmSteps"].as_i64().unwrap())
                .sum::<i64>()
        );
    }
    #[test]
    fn cap_is_per_statement_and_combined_bytes_are_reported() {
        let db = Connection::open_in_memory().unwrap();
        let literal = "x".repeat(130000);
        let stages = json!({"spikeStages":[format!("CREATE TEMP TABLE a AS SELECT '{literal}' x"),format!("CREATE TEMP TABLE b AS SELECT '{literal}' x")]});
        let r = run(&db, &stages, "SELECT 1").unwrap();
        assert!(r.sql_bytes > 256000);
        assert_eq!(r.executions, 2);
        assert_eq!(
            r.profile()["totalSqlBytes"].as_u64().unwrap(),
            (r.sql_bytes + 8) as u64
        );
    }
    #[test]
    fn stage_admission_rejects_nonphysical_or_multiple_statements_and_keeps_per_statement_cap() {
        for sql in [
            "DROP TABLE t",
            "CREATE TABLE t(x)",
            "CREATE TEMP TABLE t AS SELECT 1; DELETE FROM t",
        ] {
            assert!(validate(sql).is_err());
        }
        assert!(validate("CREATE TEMP TABLE t AS SELECT ';' x /* ; DROP TABLE t */;").is_ok());
        let db = Connection::open_in_memory().unwrap();
        assert!(
            run(
                &db,
                &json!({"spikeStages":vec!["CREATE TABLE t AS SELECT 1";100]}),
                "SELECT 1"
            )
            .is_err()
        );
        let large = format!("CREATE TABLE t AS SELECT '{}' x", "x".repeat(256000));
        assert!(validate(&large).is_err());
    }
}

#[test]
fn common_physical_cuts_reset_before_prepared_query_reexecution() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE source(node INTEGER PRIMARY KEY); INSERT INTO source VALUES(1),(2),(3)",
    )
    .unwrap();
    let stages = json!({"physicalStages":["CREATE TEMP TABLE IF NOT EXISTS cut(node INTEGER PRIMARY KEY) WITHOUT ROWID","DELETE FROM cut","INSERT INTO cut SELECT node FROM source"]});
    let first = run(&db, &stages, "SELECT node FROM cut ORDER BY node").unwrap();
    assert_eq!(first.executions, 3);
    let mut query = db.prepare("SELECT node FROM cut ORDER BY node").unwrap();
    for expected in [vec![1, 2, 3], vec![], vec![9]] {
        db.execute("DELETE FROM source", []).unwrap();
        for node in &expected {
            db.execute("INSERT INTO source VALUES(?)", [node]).unwrap();
        }
        let report = run(&db, &stages, "SELECT node FROM cut ORDER BY node").unwrap();
        assert_eq!(report.executions, 3);
        assert_eq!(report.expected_statements(), 4);
        let trace = ExecutionTrace::start(&db);
        let actual = query
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(trace.count(), 1);
    }
}
