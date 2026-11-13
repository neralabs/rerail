//! Shared SQLite backup boundary for failure capture and the offline replay host.
use rusqlite::{Connection, Result, ffi};
use std::ffi::CStr;

pub(crate) fn copy(source: &Connection, from: &CStr, target: &Connection, to: &CStr) -> Result<()> {
    // SAFETY: both owned connections remain live. This handle has exactly one finish call.
    unsafe {
        let backup =
            ffi::sqlite3_backup_init(target.handle(), to.as_ptr(), source.handle(), from.as_ptr());
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
    Ok(())
}
