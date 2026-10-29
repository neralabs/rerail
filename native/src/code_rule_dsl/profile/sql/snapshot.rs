//! Preserve the failed SQL owner's temporary relations for bounded offline diagnosis.
use rusqlite::{Connection, Result};
#[path = "backup.rs"]
mod backup;
use std::{
    ffi::CStr,
    path::{Path, PathBuf},
    sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

fn save(source: &Connection, path: &Path, schema: &CStr) -> Result<()> {
    let target = Connection::open(path)?;
    backup::copy(source, schema, &target, c"main")
}

/// No callback or successful-query overhead: invoked only after an engine failure.
/// Main and temporary databases are separate owners and must both be preserved.
pub(in crate::code_rule_dsl) fn capture(
    source: &Connection,
    context: serde_json::Value,
) -> Result<Option<PathBuf>> {
    static DIRECTORY: LazyLock<Option<PathBuf>> =
        LazyLock::new(|| std::env::var_os("RERAIL_SQL_FAILURE_DIR").map(PathBuf::from));
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let Some(directory) = DIRECTORY.as_ref() else {
        return Ok(None);
    };
    std::fs::create_dir_all(directory).map_err(super::io_error)?;
    let stem = format!(
        "failure-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let main = directory.join(format!("{stem}.main.sqlite"));
    let temp = directory.join(format!("{stem}.temp.sqlite"));
    save(source, &main, c"main")?;
    let temp_error = save(source, &temp, c"temp").err().map(|e| e.to_string());
    let manifest = directory.join(format!("{stem}.json"));
    let data = serde_json::json!({"format":"rerail.sql-failure-snapshot/v1","context":context,
        "main":main.file_name().unwrap().to_string_lossy(),
        "temp":temp_error.is_none().then(||temp.file_name().unwrap().to_string_lossy().into_owned()),
        "tempError":temp_error,"limitations":["SQLite tables, indexes, and view definitions are copied. Native virtual-table frames and source callbacks require their original producers."]});
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&data).expect("failure snapshot metadata"),
    )
    .map_err(super::io_error)?;
    Ok(Some(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_snapshot_keeps_temp_values_indexes_and_source_ownership() -> Result<()> {
        let dir = super::super::tests::tempdir();
        let source = Connection::open_in_memory()?;
        source.execute_batch("CREATE TABLE unrelated_main(value);INSERT INTO unrelated_main VALUES(99);CREATE TEMP TABLE evidence(path TEXT,owner INTEGER,value);CREATE INDEX evidence_owner ON evidence(path,owner);INSERT INTO evidence VALUES('é.py',3,NULL),('é.py',7,'body');")?;
        let changed = source.total_changes();
        let path = dir.path().join("failure.sqlite");
        save(&source, &path, c"temp")?;
        assert_eq!(source.total_changes(), changed);
        source.execute_batch("DROP TABLE evidence")?;
        let captured = Connection::open(&path)?;
        let rows = captured
            .prepare("SELECT owner,value FROM evidence ORDER BY owner")?
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(rows, [(3, None), (7, Some("body".into()))]);
        assert_eq!(
            captured.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='evidence_owner'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        assert_eq!(
            captured.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='unrelated_main'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        Ok(())
    }
}
