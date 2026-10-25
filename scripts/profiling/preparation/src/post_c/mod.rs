//! Isolated prototypes for the remaining post-C execution boundaries.
mod guard;
use crate::schema::{self, Column, Columns, Registry, cache};
pub use guard::inference_guard;
use rusqlite::{Connection, Result, functions::FunctionFlags, hooks::Authorization};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
const MAX_SQL_BYTES: usize = 1_048_576;
type Schemas = Vec<BTreeMap<String, Columns>>;

fn inference_bytes(query: &str, names: &[String]) -> usize {
    query.len() + names.iter().map(|name| 2 * name.len() + 48).sum::<usize>() + 128
}

#[derive(Deserialize)]
struct Program {
    name: String,
    inputs: BTreeMap<String, String>,
    stages: Vec<Stage>,
}
#[derive(Deserialize)]
struct Stage {
    name: String,
    query: String,
}
struct Pending {
    name: String,
    query: String,
    names: Vec<String>,
    key: cache::Key,
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn inspect(db: &Connection, query: &str, schemas: &Registry) -> Result<(Vec<String>, Vec<String>)> {
    let allowed = schemas.keys().cloned().collect::<BTreeSet<_>>();
    let reads = crate::authorizer::install(db, &allowed)?;
    let output = (|| {
        let statement = db.prepare(query)?;
        if !statement.readonly() || statement.parameter_count() != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(statement
            .column_names()
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>())
    })();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)?;
    let names = output?;
    if names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>()
        .len()
        != names.len()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let inputs = reads.fields.lock().keys().cloned().collect();
    Ok((names, inputs))
}

fn flush(
    db: &Connection,
    pending: &mut Vec<Pending>,
    schemas: &mut Registry,
    cache: &mut cache::Cache,
    statements: &mut usize,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let mut projection = Vec::new();
    let mut sources = Vec::new();
    for (i, stage) in pending.iter().enumerate() {
        sources.push(format!("({}) q{i}", stage.query));
        projection.extend(
            stage.names.iter().enumerate().map(|(j, name)| {
                format!("q{i}.{} AS {}", quote(name), quote(&format!("s{i}c{j}")))
            }),
        );
    }
    let temporary = format!("__inference_{}", *statements);
    let query = format!(
        "SELECT {} FROM {}",
        projection.join(","),
        sources.join(" CROSS JOIN ")
    );
    db.execute_batch(&format!(
        "CREATE TEMP TABLE {} AS {}",
        quote(&temporary),
        schema::inference::empty(&query)
    ))?;
    *statements += 1;
    let all = schema::describe(db, &temporary)?;
    let mut offset = 0;
    for stage in pending.drain(..) {
        let columns = stage
            .names
            .iter()
            .enumerate()
            .map(|(i, name)| Column {
                name: name.clone(),
                declared_type: all[offset + i].declared_type.clone(),
            })
            .collect::<Columns>();
        offset += columns.len();
        let columns = Arc::new(columns);
        schema::bind(db, &stage.name, schema::declaration(&stage.name, &columns))?;
        cache.insert(stage.key, columns.clone());
        schemas.insert(stage.name, columns);
    }
    assert_eq!(offset, all.len());
    Ok(())
}

fn compile(programs: &[Program], batch: bool) -> Result<(usize, Schemas)> {
    let mut cache = cache::Cache::default();
    let mut statements = 0;
    let mut results = Vec::new();
    for program in programs {
        let db = Connection::open_in_memory()?;
        db.set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH,
            MAX_SQL_BYTES as i32,
        )?;
        // Compilation must not evaluate scalar functions, even on a cold miss.
        for (name, arity) in [
            ("rx_matches", 2),
            ("rx_extract", 3),
            ("rx_replace", 3),
            ("source_text", 3),
        ] {
            db.create_scalar_function(
                name,
                arity,
                FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
                |_| -> Result<i64> { Err(rusqlite::Error::InvalidQuery) },
            )?;
        }
        let mut schemas = Registry::new();
        for (name, declaration) in &program.inputs {
            schemas.insert(
                name.clone(),
                Arc::new(schema::register(&db, name, declaration.clone())?),
            );
        }
        let mut pending = Vec::<Pending>::new();
        for stage in &program.stages {
            let (names, reads) = inspect(&db, &stage.query, &schemas)?;
            let width: usize = pending.iter().map(|stage| stage.names.len()).sum();
            let bytes = pending
                .iter()
                .map(|stage| inference_bytes(&stage.query, &stage.names))
                .sum::<usize>()
                + inference_bytes(&stage.query, &names);
            if !pending.is_empty()
                && (reads
                    .iter()
                    .any(|name| pending.iter().any(|p| &p.name == name))
                    || pending.len() == 16
                    || width + names.len() > 1500
                    || bytes > MAX_SQL_BYTES - 1024)
            {
                flush(&db, &mut pending, &mut schemas, &mut cache, &mut statements)?;
                // Flushing changes declared types, never names or authorized
                // relations. CTAS below prepares the query with the final types;
                // another authorizer prepare would repeat the same discovery.
            }
            let key = cache::Key::new(&stage.query, &schemas, reads.iter(), std::iter::empty());
            if let Some(columns) = cache.get(&key) {
                let columns = columns.clone();
                schema::bind(&db, &stage.name, schema::declaration(&stage.name, &columns))?;
                schemas.insert(stage.name.clone(), columns);
            } else if batch && inference_bytes(&stage.query, &names) <= MAX_SQL_BYTES - 1024 {
                let columns = names
                    .iter()
                    .map(|name| Column {
                        name: name.clone(),
                        declared_type: String::new(),
                    })
                    .collect::<Columns>();
                schema::bind(&db, &stage.name, schema::declaration(&stage.name, &columns))?;
                schemas.insert(stage.name.clone(), Arc::new(columns));
                pending.push(Pending {
                    name: stage.name.clone(),
                    query: stage.query.clone(),
                    names,
                    key,
                });
            } else {
                db.execute_batch(&format!(
                    "CREATE TEMP TABLE {} AS {}",
                    quote(&stage.name),
                    schema::inference::empty(&stage.query)
                ))?;
                statements += 1;
                let columns = Arc::new(schema::describe(&db, &stage.name)?);
                cache.insert(key, columns.clone());
                schemas.insert(stage.name.clone(), columns);
            }
        }
        flush(&db, &mut pending, &mut schemas, &mut cache, &mut statements)?;
        results.push(
            schemas
                .into_iter()
                .map(|(name, columns)| (name, (*columns).clone()))
                .collect(),
        );
    }
    Ok((statements, results))
}

pub fn affinity(
    input: &Path,
    output: &Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let programs: Vec<Program> = serde_json::from_slice(&std::fs::read(input)?)?;
    let mut trials = Vec::new();
    for trial in 0..5 {
        let mut modes = [false, true];
        modes.rotate_left(trial % 2);
        let mut reference = None;
        for batch in modes {
            // SAFETY: this standalone host owns all SQLite work on this thread.
            unsafe { rusqlite::ffi::sqlite3_memory_highwater(1) };
            let start = crate::composition::cpu_ms();
            let (count, columns) = compile(&programs, batch)?;
            let cpu = crate::composition::cpu_ms() - start;
            // SAFETY: read the process-wide SQLite high-water counter after completion.
            let heap_peak = unsafe { rusqlite::ffi::sqlite3_memory_highwater(0) };
            if let Some(reference) = &reference {
                assert_eq!(reference, &columns);
            } else {
                reference = Some(columns.clone());
            }
            trials.push(json!({"trial":trial,"batched":batch,"statements":count,"cpuMs":cpu,"sqliteHeapPeakBytes":heap_peak,"columnsMatch":true}));
        }
    }
    let report = json!({"format":"rerail.post-c-affinity-spike/v1","programs":programs.iter().map(|p|&p.name).collect::<Vec<_>>(),"trials":trials,"scope":"Cold compiler metadata, including bounded cache sharing. Uses production schema module, typed cache keys and authorizer. Every inference statement counts. Input declarations use prepare-only modules. No authored expression executes. This is a component replay, not the complete guard."});
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batching_preserves_sqlite_cast_blob_null_compound_and_window_affinity() {
        let programs=vec![Program { name:"affinity".into(), inputs:BTreeMap::from([("input".into(),"CREATE TABLE input(i INTEGER,t TEXT,b BLOB,r REAL)".into())]), stages:vec![
            Stage{name:"a".into(),query:"SELECT CAST(t AS TEXT) text,CAST(i AS REAL) real,b,CASE WHEN i THEN NULL ELSE b END mixed,row_number() OVER(ORDER BY i) ordinal FROM input".into()},
            Stage{name:"b".into(),query:"SELECT CAST(i AS BLOB) blob,CAST(t AS NUMERIC) numeric,t COLLATE NOCASE text FROM input UNION ALL SELECT b,r,t FROM input".into()},
            Stage{name:"c".into(),query:"SELECT a.*,b.numeric,typeof(a.text) kind FROM a JOIN b ON a.real=b.numeric".into()},
        ]}];
        let serial = compile(&programs, false).unwrap();
        let batched = compile(&programs, true).unwrap();
        assert_eq!(serial.1, batched.1);
        assert!(batched.0 < serial.0);
    }
    #[test]
    fn compilation_still_rejects_unreachable_invalid_and_forbidden_sql() {
        for query in [
            "SELECT absent FROM input",
            "SELECT * FROM sqlite_schema",
            "DELETE FROM input",
        ] {
            let programs = vec![Program {
                name: "invalid".into(),
                inputs: BTreeMap::from([("input".into(), "CREATE TABLE input(i INTEGER)".into())]),
                stages: vec![Stage {
                    name: "a".into(),
                    query: query.into(),
                }],
            }];
            assert!(compile(&programs, true).is_err());
        }
    }

    #[test]
    fn independent_inference_stays_inside_the_engine_sql_length_limit() {
        let programs = vec![Program {
            name: "bounded".into(),
            inputs: BTreeMap::from([("input".into(), "CREATE TABLE input(i INTEGER)".into())]),
            stages: (0..3)
                .map(|i| Stage {
                    name: format!("stage_{i}"),
                    query: format!("SELECT '{}' AS value FROM input", "x".repeat(550_000 + i)),
                })
                .collect(),
        }];
        let serial = compile(&programs, false).unwrap();
        let batched = compile(&programs, true).unwrap();
        assert_eq!(serial.1, batched.1);
        assert_eq!(
            batched.0, 3,
            "the two large independent queries cannot share a statement"
        );
    }
}
