//! Shared SQLite storage settings for production and captured-input replay.
use rusqlite::{Connection, Result};

pub(super) const CACHE_KIB: i64 = 1024;

pub(super) fn configure(db: &Connection) -> Result<()> {
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL; PRAGMA temp_store=FILE; PRAGMA cache_spill=ON; PRAGMA mmap_size=0;")?;
    db.pragma_update(None, "cache_size", -CACHE_KIB)?;
    db.pragma_update(Some("temp"), "cache_size", -512_i64)?;
    db.execute_batch("CREATE TEMP TABLE request_file(fid INTEGER PRIMARY KEY) WITHOUT ROWID;
        CREATE TEMP TABLE structural_demand(fid INTEGER PRIMARY KEY,partial INTEGER NOT NULL) WITHOUT ROWID")?;
    Ok(())
}

pub(super) fn request(db: &Connection, files: &[i64]) -> Result<usize> {
    db.execute("DELETE FROM request_file", [])?;
    for files in files.chunks(128) {
        let sql = format!(
            "INSERT INTO request_file VALUES {}",
            std::iter::repeat_n("(?)", files.len())
                .collect::<Vec<_>>()
                .join(",")
        );
        db.execute(&sql, rusqlite::params_from_iter(files))?;
    }
    Ok(1 + files.len().div_ceil(128))
}
