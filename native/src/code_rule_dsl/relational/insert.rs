//! Bounded SQLite input batches shared by AST evidence and reference edges.
use super::{invalid, quote, sql_error};
use napi::Result;
use rusqlite::{Connection, params_from_iter, types::Value};
use std::sync::atomic::{AtomicUsize, Ordering};

const DEFAULT_ROWS: usize = 32;
const DEFAULT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
struct Limits {
    rows: usize,
    bytes: usize,
}

#[derive(Debug)]
struct Stats {
    rows: usize,
    batch_rows: usize,
    statements: usize,
    max_buffered_bytes: usize,
}

pub(super) fn insert_rows<R: IntoIterator<Item = Value>>(
    db: &Connection,
    name: &str,
    width: usize,
    rows: impl IntoIterator<Item = R>,
    work: &AtomicUsize,
    exhausted: &str,
) -> Result<()> {
    #[cfg(feature = "code-rule-profile")]
    let mut span = crate::code_rule_dsl::profile::trace::Span::start("sql-input", name);
    #[cfg(feature = "code-rule-profile")]
    let requested = std::env::var("RERAIL_SQL_INSERT_BATCH")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_ROWS)
        .clamp(1, 256);
    #[cfg(not(feature = "code-rule-profile"))]
    let requested = DEFAULT_ROWS;
    let result = write_rows(
        db,
        name,
        width,
        rows,
        work,
        exhausted,
        Limits {
            rows: requested,
            bytes: DEFAULT_BYTES,
        },
    );
    #[cfg(feature = "code-rule-profile")]
    span.detail(|| match &result {
        Ok(stats) => serde_json::json!({"rows":stats.rows,"batchRows":stats.batch_rows,
            "insertStatements":stats.statements,"maxBufferedValueBytes":stats.max_buffered_bytes}),
        Err(_) => serde_json::json!({"failed":true}),
    });
    result.map(|stats| {
        // Keep the counters available in ordinary builds and unit tests too.
        let _ = (
            stats.rows,
            stats.batch_rows,
            stats.statements,
            stats.max_buffered_bytes,
        );
    })
}

fn payload_bytes(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Integer(_) | Value::Real(_) => 8,
        Value::Text(value) => value.len(),
        Value::Blob(value) => value.len(),
    }
}

fn write_rows<R: IntoIterator<Item = Value>>(
    db: &Connection,
    name: &str,
    width: usize,
    rows: impl IntoIterator<Item = R>,
    work: &AtomicUsize,
    exhausted: &str,
    limits: Limits,
) -> Result<Stats> {
    let variables = db
        .limit(rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER)
        .map_err(sql_error)? as usize;
    if width == 0 || width > variables {
        return Err(invalid("relational input exceeds SQLite's parameter limit"));
    }
    let batch_rows = limits.rows.clamp(1, 256).min(variables / width);
    let tuple = format!("({})", vec!["?"; width].join(","));
    let sql = |count| {
        format!(
            "INSERT INTO {} VALUES {}",
            quote(name),
            vec![tuple.as_str(); count].join(",")
        )
    };
    let mut full_statement = None;
    let mut statements = 0;
    let mut flush = |values: &mut Vec<Value>| -> Result<()> {
        let count = values.len() / width;
        if count == batch_rows {
            if full_statement.is_none() {
                full_statement = Some(db.prepare(&sql(batch_rows)).map_err(sql_error)?);
            }
            full_statement
                .as_mut()
                .unwrap()
                .execute(params_from_iter(values.iter()))
                .map_err(sql_error)?;
        } else {
            // Byte-bound batches can have different row counts. The connection's
            // bounded statement cache also reuses these partial INSERT shapes.
            db.prepare_cached(&sql(count))
                .map_err(sql_error)?
                .execute(params_from_iter(values.iter()))
                .map_err(sql_error)?;
        }
        statements += 1;
        values.clear();
        Ok(())
    };
    let mut values = Vec::with_capacity(batch_rows * width);
    let mut scratch = Vec::with_capacity(width);
    let (mut count, mut bytes, mut maximum) = (0, 0usize, 0);
    for row in rows {
        if work
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            })
            .is_err()
            || crate::code_rule_dsl::budget::step_budget_exhausted()
        {
            return Err(invalid(exhausted));
        }
        scratch.extend(row);
        if scratch.len() != width {
            return Err(invalid(
                "relational input row has an unexpected column count",
            ));
        }
        let row_bytes = scratch.iter().map(payload_bytes).sum::<usize>();
        if !values.is_empty() && bytes.saturating_add(row_bytes) > limits.bytes {
            flush(&mut values)?;
            bytes = 0;
        }
        values.append(&mut scratch);
        bytes = bytes.saturating_add(row_bytes);
        maximum = maximum.max(bytes);
        count += 1;
        // An oversized individual row retains the original single-row behavior.
        if values.len() / width == batch_rows || bytes >= limits.bytes {
            flush(&mut values)?;
            bytes = 0;
        }
    }
    if !values.is_empty() {
        flush(&mut values)?;
    }
    Ok(Stats {
        rows: count,
        batch_rows,
        statements,
        max_buffered_bytes: maximum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_nullable_unicode_and_numeric_rows_with_a_tail() {
        let db = Connection::open_in_memory().unwrap();
        db.execute("CREATE TABLE facts (name,annotation,offset)", [])
            .unwrap();
        let work = AtomicUsize::new(131);
        let rows = (0..131).map(|i| {
            [
                Value::Text(format!("ä{i}")),
                if i % 2 == 0 {
                    Value::Null
                } else {
                    Value::Text("str".into())
                },
                Value::Integer(i),
            ]
        });
        let stats = write_rows(
            &db,
            "facts",
            3,
            rows,
            &work,
            "exhausted",
            Limits {
                rows: 32,
                bytes: DEFAULT_BYTES,
            },
        )
        .unwrap();
        assert_eq!(
            (stats.rows, stats.batch_rows, stats.statements),
            (131, 32, 5)
        );
        assert_eq!(work.load(Ordering::Relaxed), 0);
        let rows = db
            .prepare("SELECT name,annotation,offset FROM facts ORDER BY offset")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 131);
        assert_eq!(rows[130], ("ä130".into(), None, 130));
        assert_eq!(rows[1], ("ä1".into(), Some("str".into()), 1));
    }

    #[test]
    fn parameter_and_payload_limits_bound_batches() {
        let db = Connection::open_in_memory().unwrap();
        db.execute("CREATE TABLE facts (name,value)", []).unwrap();
        db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 6)
            .unwrap();
        let stats = write_rows(
            &db,
            "facts",
            2,
            (0..7).map(|i| [Value::Null, Value::Integer(i)]),
            &AtomicUsize::new(100),
            "exhausted",
            Limits {
                rows: 32,
                bytes: DEFAULT_BYTES,
            },
        )
        .unwrap();
        assert_eq!((stats.batch_rows, stats.statements), (3, 3));
        db.execute("DELETE FROM facts", []).unwrap();
        let rows = ["aaaa", "bbbb", "123456789", "c"].map(|s| [Value::Text(s.into()), Value::Null]);
        let stats = write_rows(
            &db,
            "facts",
            2,
            rows,
            &AtomicUsize::new(100),
            "exhausted",
            Limits { rows: 32, bytes: 8 },
        )
        .unwrap();
        assert_eq!(
            (stats.rows, stats.statements, stats.max_buffered_bytes),
            (4, 3, 9)
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM facts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
    }

    #[test]
    fn empty_inputs_need_no_statement_and_exhaustion_is_an_error() {
        let db = Connection::open_in_memory().unwrap();
        let stats = write_rows(
            &db,
            "absent",
            1,
            std::iter::empty::<[Value; 1]>(),
            &AtomicUsize::new(0),
            "exhausted",
            Limits {
                rows: 32,
                bytes: DEFAULT_BYTES,
            },
        )
        .unwrap();
        assert_eq!((stats.rows, stats.statements), (0, 0));
        db.execute("CREATE TABLE facts (value)", []).unwrap();
        let error = write_rows(
            &db,
            "facts",
            1,
            [[Value::Null]],
            &AtomicUsize::new(0),
            "input row budget exceeded",
            Limits {
                rows: 32,
                bytes: DEFAULT_BYTES,
            },
        )
        .unwrap_err();
        assert!(error.reason.contains("input row budget exceeded"));
        assert_eq!(
            db.query_row("SELECT count(*) FROM facts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
