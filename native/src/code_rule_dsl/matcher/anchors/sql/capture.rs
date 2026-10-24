//! Diagnostic-only replay inputs, captured before candidate preparation changes the database.
use super::{graph::Node, lower::Statement, store::Store};
use rusqlite::{Connection, Result, ffi, types::Value};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(super) fn save(
    store: &Store,
    statement: &Statement,
    nodes: &HashMap<String, Node>,
    files: &[i64],
) -> Result<()> {
    let variant = "normal";
    let Some(directory) = std::env::var_os("RERAIL_SQL_PROFILE_DIR") else {
        return Ok(());
    };
    if !store.db.is_autocommit() {
        return Ok(());
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)
        .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;
    let stem = format!("candidate-{}-{id}", std::process::id());
    let path = directory.join(format!("{stem}.sqlite"));
    let target = Connection::open(&path)?;
    // SAFETY: both connections remain live throughout backup. The backup handle has exactly
    // one finish call, including failed steps. Source BLOB cursors are already closed.
    unsafe {
        let backup = ffi::sqlite3_backup_init(
            target.handle(),
            c"main".as_ptr(),
            store.db.handle(),
            c"main".as_ptr(),
        );
        if backup.is_null() {
            return Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::sqlite3_errcode(target.handle())),
                None,
            ));
        }
        let code = ffi::sqlite3_backup_step(backup, -1);
        let finish = ffi::sqlite3_backup_finish(backup);
        if code != ffi::SQLITE_DONE || finish != ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(if code != ffi::SQLITE_DONE {
                    code
                } else {
                    finish
                }),
                None,
            ));
        }
    }
    let parameters: Vec<_> = statement
        .parameters
        .iter()
        .map(|value| match value {
            Value::Null => serde_json::json!({"kind":"null"}),
            Value::Integer(v) => serde_json::json!({"kind":"integer","value":v}),
            Value::Real(v) => serde_json::json!({"kind":"real","value":v}),
            Value::Text(v) => serde_json::json!({"kind":"text","value":v}),
            Value::Blob(v) => serde_json::json!({"kind":"blob","value":v}),
        })
        .collect();
    let stages:Vec<_>=statement.stages.iter().map(|stage|{
        let node=nodes.get(&stage.name);
        let definition=statement.graph.iter().find_map(|cte|cte.strip_prefix(&format!("{}(",stage.name)).and_then(|s|s.split_once(") AS ")));
        serde_json::json!({"name":stage.name,"sql":stage.sql,"index":stage.index,"temporary":stage.temporary,"relationId":node.map(|n|n.id),"class":node.map(|n|format!("{:?}",n.class)),"columns":definition.map(|d|d.0),"body":definition.map(|d|d.1)})
    }).collect();
    let mut versions = store
        .db
        .prepare("SELECT fid,lang,hex(digest) FROM file ORDER BY fid")?;
    let versions = versions
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;
    let generation = serde_json::to_string(&versions).expect("source versions");
    let mut manifest = statement
        .producers
        .manifest(store.db.path().unwrap_or_default(), |_| {
            crate::code_rule_dsl::producers::External {
                name: format!("syntax-tape-language-{}", statement.language),
                generation: generation.clone(),
            }
        });
    let origins = store.origins.lock();
    for binding in manifest["bindings"].as_array_mut().expect("binding array") {
        let consumers = binding["consumers"]
            .as_array()
            .expect("consumer array")
            .iter()
            .flat_map(|consumer| {
                let label = consumer.as_str().expect("consumer label");
                let names = label
                    .strip_prefix("candidate-result.")
                    .and_then(|s| s.split_once(' '))
                    .and_then(|(id, _)| id.parse::<usize>().ok())
                    .and_then(|qid| statement.consumers.get(qid))
                    .and_then(|id| origins.get(id));
                match names {
                    Some(names) => names
                        .iter()
                        .map(|name| format!("{name}: {label}"))
                        .collect::<Vec<_>>(),
                    None => vec![label.to_owned()],
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        binding["consumers"] = serde_json::json!(consumers);
        binding["origin"] = consumers.into_iter().collect::<Vec<_>>().join(", ").into();
    }
    drop(origins);
    std::fs::write(
        directory.join(format!("producer-plan-{}-{id}.json", std::process::id())),
        serde_json::to_vec_pretty(&manifest).expect("producer manifest"),
    )
    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;
    let output = serde_json::json!({"format":"rerail.candidate-replay/v2","variant":variant,"database":path.file_name().unwrap().to_string_lossy(),"stages":stages,"queries":statement.queries,"schedule":statement.schedule.actions,"peakLiveRelations":statement.schedule.peak_relations,"parameters":parameters,"admissions":statement.admissions,"files":files,"fileScope":statement.file_scope,"fileVersions":versions});
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&output).expect("serializable candidate capture"),
    )
    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))
}
