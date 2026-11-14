//! Diagnostic-only replay inputs, captured before a candidate statement runs (`RERAIL_SQL_PROFILE_DIR`).
//! From Pulling-it-all-together's `matcher/anchors/sql/capture.rs`: P backed up its whole private
//! anchor database; here the capture is a store holding only the statement's tapes' prefilter
//! rows, the store-wide counts and the vocabulary (`ContentStore::capture_prefilter`). Dropped:
//! the producer manifest and origins (not in W) and the per-stage graph nodes (decoded relations
//! moved to transducers).
use super::super::lower::Statement;
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::types::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(in crate::code_rule_dsl::prefilter) fn save(store: &ContentStore, statement: &Statement) -> StoreResult<()> {
    match std::env::var_os("RERAIL_SQL_PROFILE_DIR") {
        Some(directory) => save_to(store, statement, directory.as_ref()).map(drop),
        None => Ok(()),
    }
}

/// Writes the capture of `statement` into `directory`; returns its JSON's path.
pub(in crate::code_rule_dsl::prefilter) fn save_to(
    store: &ContentStore,
    statement: &Statement,
    directory: &std::path::Path,
) -> StoreResult<std::path::PathBuf> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::fs::create_dir_all(directory)?;
    let stem = format!("candidate-{}-{id}", std::process::id());
    let path = directory.join(format!("{stem}.sqlite"));
    store.capture_prefilter(&path, &statement.files)?;
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
    let stages:Vec<_>=statement.stages.iter().enumerate().map(|(i,stage)|{
        serde_json::json!({"name":stage.name,"sql":stage.sql,"slot":statement.ownership.slot(i),"members":statement.ownership.members(i),"index":stage.index,"temporary":stage.temporary})
    }).collect();
    let output = serde_json::json!({"format":"rerail.candidate-replay/v4","database":path.file_name().unwrap().to_string_lossy(),"language":statement.language,"stages":stages,"queries":statement.queries,"schedule":statement.schedule.actions,"peakLiveRelations":statement.schedule.peak_relations,"graph":statement.graph,"setup":statement.ownership.setup,"parameters":parameters,"admissions":statement.admissions,"files":statement.files});
    let json = directory.join(format!("{stem}.json"));
    std::fs::write(&json, serde_json::to_vec_pretty(&output).map_err(|e| StoreError::Invalid(e.to_string()))?)?;
    Ok(json)
}
