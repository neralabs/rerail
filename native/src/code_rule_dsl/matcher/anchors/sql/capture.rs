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
    save_variant(store, statement, nodes, "normal", files)?;
    if let Some(original) = &statement.original {
        let original_nodes =
            store
                .graph
                .lock()
                .register(&original.graph, &original.parameters, original.language);
        save_variant(store, original, &original_nodes, "uncompacted", files)?;
    }
    Ok(())
}

fn save_variant(
    store: &Store,
    statement: &Statement,
    nodes: &HashMap<String, Node>,
    variant: &str,
    files: &[i64],
) -> Result<()> {
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
    let output = serde_json::json!({"format":"rerail.candidate-replay/v2","variant":variant,"database":path.file_name().unwrap().to_string_lossy(),"stages":stages,"queries":statement.queries,"schedule":statement.schedule.actions,"peakLiveRelations":statement.schedule.peak_relations,"parameters":parameters,"files":files,"fileScope":statement.file_scope,"fileVersions":versions});
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&output).expect("serializable candidate capture"),
    )
    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))
}
