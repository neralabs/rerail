//! Never-shipped tuning and statement diagnostics for the unbounded benchmark build.

use super::timing::Timing;
use rusqlite::{Connection, StatementStatus};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static TRACE: LazyLock<Option<(String, String)>> = LazyLock::new(|| {
    std::env::var("RERAIL_SQL_TRACE_DIR").ok().map(|directory| {
        (
            directory,
            std::env::var("RERAIL_SQL_TRACE_FILTER").unwrap_or_default(),
        )
    })
});

thread_local! {
    static RECORDS: RefCell<BTreeMap<String, Value>> = const { RefCell::new(BTreeMap::new()) };
}

pub(super) fn enabled(sql: &str) -> bool {
    TRACE
        .as_ref()
        .is_some_and(|(_, filter)| sql.contains(filter))
}

pub(super) fn configure(connection: &Connection) -> rusqlite::Result<()> {
    if let Ok(mode) = std::env::var("RERAIL_SQL_TEMP_JOURNAL") {
        if !matches!(mode.as_str(), "OFF" | "MEMORY") {
            return Err(rusqlite::Error::InvalidParameterName(
                "RERAIL_SQL_TEMP_JOURNAL".into(),
            ));
        }
        connection.pragma_update(Some("temp"), "journal_mode", mode)?;
    }
    for (variable, pragma, minimum, maximum) in [
        ("RERAIL_SQL_CACHE_KIB", "cache_size", 1_i64, 1 << 24),
        ("RERAIL_SQL_AUTOMATIC_INDEX", "automatic_index", 0, 1),
        ("RERAIL_SQL_SORT_THREADS", "threads", 0, 16),
    ] {
        if let Ok(value) = std::env::var(variable) {
            let value = value
                .parse::<i64>()
                .ok()
                .filter(|n| (minimum..=maximum).contains(n))
                .ok_or_else(|| rusqlite::Error::InvalidParameterName(variable.to_owned()))?;
            connection.pragma_update(
                None,
                pragma,
                if pragma == "cache_size" {
                    -value
                } else {
                    value
                },
            )?;
        }
    }
    if let Ok(value) = std::env::var("RERAIL_SQL_TEMP_CACHE_KIB") {
        let value = value
            .parse::<i64>()
            .ok()
            .filter(|n| (1..=1 << 24).contains(n))
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("RERAIL_SQL_TEMP_CACHE_KIB".to_owned())
            })?;
        connection.pragma_update(Some("temp"), "cache_size", -value)?;
    }
    if let Some((directory, _)) = TRACE.as_ref() {
        std::fs::create_dir_all(directory).expect("create SQL diagnostic directory");
        let options = connection
            .prepare("PRAGMA compile_options")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let metadata = json!({"compileOptions":options,
            "mainCacheSize":connection.pragma_query_value(None,"cache_size",|row| row.get::<_,i64>(0))?,
            "tempCacheSize":connection.pragma_query_value(Some("temp"),"cache_size",|row| row.get::<_,i64>(0))?});
        std::fs::write(
            format!("{directory}/metadata-{}.json", std::process::id()),
            serde_json::to_vec(&metadata).expect("serialize SQL metadata"),
        )
        .expect("write SQL metadata");
    }
    Ok(())
}

pub(super) fn first_probe() -> bool {
    std::env::var("RERAIL_SQL_FIRST_PROBE").is_ok_and(|value| matches!(value.as_str(), "1" | "all"))
}

pub(super) fn first_probe_all() -> bool {
    std::env::var("RERAIL_SQL_FIRST_PROBE").is_ok_and(|value| value == "all")
}

pub(super) fn joined_diff() -> bool {
    std::env::var("RERAIL_SQL_ANSWER_DIFF").is_ok_and(|value| value == "join")
}

pub(super) fn counters(statement: &rusqlite::Statement<'_>) -> [i64; 4] {
    [
        StatementStatus::VmStep,
        StatementStatus::FullscanStep,
        StatementStatus::Sort,
        StatementStatus::AutoIndex,
    ]
    .map(|status| i64::from(statement.reset_status(status)))
}

pub(super) fn record<'a>(
    connection: &Connection,
    sql: &str,
    time: Timing,
    rows: usize,
    counters: [i64; 4],
    inputs: impl Iterator<Item = (&'a str, i64)>,
) {
    if !enabled(sql) {
        return;
    }
    RECORDS.with_borrow_mut(|records| {
        let entry = records.entry(sql.to_owned()).or_insert_with(|| {
            let plan = connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .and_then(|mut statement| {
                    statement
                        .query_map([], |row| {
                            Ok(json!([
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(3)?
                            ]))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .map_or_else(
                    |error| json!({"error": error.to_string()}),
                    |plan| json!(plan),
                );
            json!({"sql":sql,"plan":plan,"runs":0,"rows":0,"wallNs":0,"cpuNs":0,
                "vmSteps":0,"fullscanSteps":0,"sorts":0,"autoIndexRows":0,"inputPeakRows":{}})
        });
        for (key, count) in [
            ("runs", 1),
            ("rows", rows as i64),
            ("wallNs", time.wall as i64),
            ("cpuNs", time.cpu as i64),
            ("vmSteps", counters[0]),
            ("fullscanSteps", counters[1]),
            ("sorts", counters[2]),
            ("autoIndexRows", counters[3]),
        ] {
            entry[key] = json!(entry[key].as_i64().unwrap_or(0) + count);
        }
        for (name, count) in inputs {
            entry["inputPeakRows"][name] = json!(
                entry["inputPeakRows"][name]
                    .as_i64()
                    .unwrap_or(0)
                    .max(count)
            );
        }
    });
}

pub(super) fn flush() {
    let Some((directory, _)) = TRACE.as_ref() else {
        return;
    };
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    RECORDS.with_borrow_mut(|records| {
        if records.is_empty() {
            return;
        }
        std::fs::create_dir_all(directory).expect("create SQL diagnostic directory");
        let path = format!(
            "{directory}/{}-{}.json",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let bytes = serde_json::to_vec(&*records).expect("serialize SQL diagnostics");
        std::fs::write(path, bytes).expect("write SQL diagnostics");
        records.clear();
    });
}
