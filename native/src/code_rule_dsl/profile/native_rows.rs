//! Profile-only input adapter. It preserves actual matcher and reference-graph facts.
use crate::code_rule_dsl::{budget, json::invalid};
use rerail_reachability_spike::rows::{OwnedRows, register};
use rusqlite::{Connection, types::Value};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) fn charge(work: Arc<AtomicUsize>) -> rerail_reachability_spike::rows::Charge {
    Arc::new(move |count| {
        if work
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(count)
            })
            .is_err()
            || budget::step_budget_exhausted()
        {
            Err(rusqlite::Error::UserFunctionError(
                "interrupted: native fact work budget exceeded".into(),
            ))
        } else {
            Ok(())
        }
    })
}
pub(crate) fn load<R: IntoIterator<Item = Value>>(
    db: &Connection,
    name: &str,
    rows: impl IntoIterator<Item = R>,
    work: Arc<AtomicUsize>,
) -> napi::Result<()> {
    let mut span = super::trace::Span::start("sql-input", name);
    let columns = db
        .prepare(&format!("SELECT * FROM \"{name}\" LIMIT 0"))
        .map_err(|e| invalid(e.to_string()))?
        .column_names()
        .iter()
        .map(|v| (*v).to_owned())
        .collect::<Vec<_>>();
    let charge = charge(work);
    let mut values = Vec::new();
    for row in rows {
        charge(1).map_err(|e| invalid(e.to_string()))?;
        values.push(row.into_iter().collect());
    }
    let count = values.len();
    let mut source =
        OwnedRows::new(columns, values, vec![0], charge).map_err(|e| invalid(e.to_string()))?;
    source.types = db
        .prepare(&format!("PRAGMA table_info(\"{name}\")"))
        .map_err(|e| invalid(e.to_string()))?
        .query_map([], |r| r.get::<_, String>(2))
        .map_err(|e| invalid(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| invalid(e.to_string()))?;
    db.execute_batch(&format!("DROP TABLE \"{name}\";"))
        .map_err(|e| invalid(e.to_string()))?;
    register(db, name, Arc::new(source)).map_err(|e| invalid(e.to_string()))?;
    span.detail(|| serde_json::json!({"rows":count,"insertStatements":0,"nativeRows":true}));
    Ok(())
}
