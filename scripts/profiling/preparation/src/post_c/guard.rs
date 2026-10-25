//! Reproduce eager CTE initialization with the production prepare-only adapter.
use super::quote;
use crate::schema;
use rusqlite::{Connection, functions::FunctionFlags};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Deserialize)]
struct Input {
    inputs: BTreeMap<String, String>,
    sql: String,
}

pub fn inference_guard(path: &Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let input: Input = serde_json::from_slice(&std::fs::read(path)?)?;
    let select = input
        .sql
        .split_once(" AS ")
        .ok_or("expected captured CREATE TABLE ... AS SELECT")?
        .1;
    let mut results = Vec::new();
    for (name, query) in [
        ("original", select.to_owned()),
        ("empty-left", schema::inference::empty(select)),
    ] {
        let db = Connection::open_in_memory()?;
        let counter = Arc::new(AtomicUsize::new(0));
        for (name, arity) in [
            ("source_text", 3),
            ("rx_matches", 2),
            ("rx_extract", 3),
            ("rx_replace", 3),
        ] {
            let counter = counter.clone();
            db.create_scalar_function(name, arity, FunctionFlags::SQLITE_UTF8, move |_| {
                counter.fetch_add(1, Ordering::Relaxed);
                Err::<String, _>(rusqlite::Error::InvalidQuery)
            })?;
        }
        for (name, declaration) in &input.inputs {
            schema::register(&db, name, declaration.clone())?;
        }
        let result = db.execute_batch(&format!(
            "CREATE TEMP TABLE {} AS {query}",
            quote("metadata")
        ));
        let columns = if result.is_ok() {
            Some(
                schema::describe(&db, "metadata")?
                    .into_iter()
                    .map(|c| json!({"name":c.name,"type":c.declared_type}))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        results.push(json!({"variant":name,"passed":result.is_ok(),"error":result.err().map(|e|e.to_string()),"authoredFunctionCalls":counter.load(Ordering::Relaxed),"columns":columns}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"sqliteVersion":rusqlite::version(),"results":results})
        )?
    );
    Ok(())
}
