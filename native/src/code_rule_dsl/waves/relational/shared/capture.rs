//! Diagnostic copies preserve SQLite types and producer bindings without querying each relation.
use super::super::{Database, Plan, Stage};
use rusqlite::{Connection, Result, ffi};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

pub(in crate::code_rule_dsl::waves::relational) fn save(
    db: &Database,
    rule: &str,
    clause: &str,
    plan: &Plan,
    aliases: &BTreeMap<String, String>,
    stages: &[Stage],
) -> Result<()> {
    let Some(directory) = std::env::var_os("RERAIL_SQL_PROFILE_DIR") else {
        return Ok(());
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let directory = std::path::PathBuf::from(directory);
    let stem = format!(
        "relational-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let target = Connection::open(directory.join(format!("{stem}.sqlite")))?;
    // SAFETY: both connections remain live on this thread, with no open input transaction.
    // The backup handle is finished exactly once, including failed steps.
    unsafe {
        let backup = ffi::sqlite3_backup_init(
            target.handle(),
            c"main".as_ptr(),
            db.handle(),
            c"temp".as_ptr(),
        );
        if backup.is_null() {
            return Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::sqlite3_errcode(target.handle())),
                None,
            ));
        }
        let step = ffi::sqlite3_backup_step(backup, -1);
        let finish = ffi::sqlite3_backup_finish(backup);
        if step != ffi::SQLITE_DONE || finish != ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(
                ffi::Error::new(if step == ffi::SQLITE_DONE {
                    finish
                } else {
                    step
                }),
                None,
            ));
        }
    }
    let inputs = plan
        .inputs
        .iter()
        .map(|(name, input)| {
            (
                name,
                std::iter::once("__path")
                    .chain(["__start", "__end"])
                    .chain(input.ast.project.iter().map(|p| p.field.as_str()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let stages=plan.stages.iter().zip(stages).map(|(logical,physical)|serde_json::json!({"name":logical.name,"physical":physical.name,"inputs":logical.inputs,"query":physical.query.as_ref(),"declaration":physical.declaration.as_ref()})).collect::<Vec<_>>();
    let report = serde_json::json!({"format":"rerail.relational-capture/v1","rule":rule,"clause":clause,"aliases":aliases,"inputs":inputs,"stages":stages,"cuts":plan.cuts,"database":format!("{stem}.sqlite")});
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&report).expect("serializable producer capture"),
    )
    .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))
}
