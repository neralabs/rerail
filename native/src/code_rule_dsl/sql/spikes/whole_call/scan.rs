//! Optional SQLite-native loop measurements. The bundled library exposes this API only when
//! built with SQLITE_ENABLE_STMT_SCANSTATUS; ordinary builds have no link to that optional symbol.
use rusqlite::{Connection, Result};
use serde_json::{Value, json};

pub(super) fn configure(connection: &Connection, enabled: bool) -> Result<()> {
    #[cfg(rerail_scanstatus)]
    {
        // SAFETY: SQLite owns the connection and accepts a nullable output pointer here.
        let rc = unsafe {
            rusqlite::ffi::sqlite3_db_config(
                connection.handle(),
                rusqlite::ffi::SQLITE_DBCONFIG_STMT_SCANSTATUS,
                i32::from(enabled),
                std::ptr::null_mut::<i32>(),
            )
        };
        if rc != rusqlite::ffi::SQLITE_OK {
            return Err(rusqlite::Error::ModuleError(format!(
                "scanstatus configuration failed: {rc}"
            )));
        }
    }
    #[cfg(not(rerail_scanstatus))]
    let _ = (connection, enabled);
    Ok(())
}
#[cfg(not(rerail_scanstatus))]
pub(super) fn read(_: usize) -> Value {
    json!({"available":false,"reason":"build with LIBSQLITE3_FLAGS=-DSQLITE_ENABLE_STMT_SCANSTATUS"})
}

#[cfg(rerail_scanstatus)]
pub(super) fn read(statement: usize) -> Value {
    use rusqlite::ffi;
    use std::ffi::{CStr, c_char, c_void};
    if statement == 0 {
        return json!({"available":true,"loops":[]});
    }
    let statement = statement as *mut ffi::sqlite3_stmt;
    let mut loops = Vec::new();
    for index in 0.. {
        let mut visited = 0i64;
        // SAFETY: read runs after query completion, before the Statement is dropped. SQLite
        // writes the documented type for each opcode, and owns returned strings until drop.
        let found = unsafe {
            ffi::sqlite3_stmt_scanstatus_v2(
                statement,
                index,
                ffi::SQLITE_SCANSTAT_NVISIT,
                ffi::SQLITE_SCANSTAT_COMPLEX,
                (&mut visited as *mut i64).cast::<c_void>(),
            )
        };
        if found != 0 {
            break;
        }
        let int = |opcode| {
            let mut value = 0i32;
            unsafe {
                ffi::sqlite3_stmt_scanstatus_v2(
                    statement,
                    index,
                    opcode,
                    ffi::SQLITE_SCANSTAT_COMPLEX,
                    (&mut value as *mut i32).cast::<c_void>(),
                );
            };
            value
        };
        let int64 = |opcode| {
            let mut value = 0i64;
            unsafe {
                ffi::sqlite3_stmt_scanstatus_v2(
                    statement,
                    index,
                    opcode,
                    ffi::SQLITE_SCANSTAT_COMPLEX,
                    (&mut value as *mut i64).cast::<c_void>(),
                );
            };
            value
        };
        let string = |opcode| {
            let mut value = std::ptr::null::<c_char>();
            unsafe {
                ffi::sqlite3_stmt_scanstatus_v2(
                    statement,
                    index,
                    opcode,
                    ffi::SQLITE_SCANSTAT_COMPLEX,
                    (&mut value as *mut *const c_char).cast::<c_void>(),
                );
            };
            if value.is_null() {
                None
            } else {
                Some(
                    unsafe { CStr::from_ptr(value) }
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        };
        let mut estimated = 0f64;
        unsafe {
            ffi::sqlite3_stmt_scanstatus_v2(
                statement,
                index,
                ffi::SQLITE_SCANSTAT_EST,
                ffi::SQLITE_SCANSTAT_COMPLEX,
                (&mut estimated as *mut f64).cast::<c_void>(),
            );
        }
        let count = int64(ffi::SQLITE_SCANSTAT_NLOOP);
        let actual = if count > 0 && visited >= 0 {
            Some(visited as f64 / count as f64)
        } else {
            None
        };
        let detail = string(ffi::SQLITE_SCANSTAT_EXPLAIN);
        let mut flags = Vec::new();
        if count > 10000 && actual.is_some_and(|r| r < 0.1) {
            flags.push("many_empty_probes");
        }
        if count > 1
            && visited > 10000
            && actual.is_some_and(|r| r > 10.)
            && detail.as_deref().is_some_and(|s| s.starts_with("SCAN "))
        {
            flags.push("repeated_scan");
        }
        let estimate_error = actual.filter(|_| estimated > 0.).map(|n| n / estimated);
        if estimate_error.is_some_and(|r| r > 10.) {
            flags.push("underestimated_cardinality");
        }
        loops.push(json!({"planId":int(ffi::SQLITE_SCANSTAT_SELECTID),"parent":int(ffi::SQLITE_SCANSTAT_PARENTID),"name":string(ffi::SQLITE_SCANSTAT_NAME),"detail":detail,"loops":count,"rowsVisited":visited,"estimatedRowsPerLoop":estimated,"actualRowsPerLoop":actual,"estimateErrorRatio":estimate_error,"hardwareCycles":int64(ffi::SQLITE_SCANSTAT_NCYCLE),"flags":flags}));
    }
    loops.sort_by_key(|v| std::cmp::Reverse(v["hardwareCycles"].as_i64().unwrap_or(0)));
    json!({"available":true,"loops":loops,"interpretation":"plan IDs join EXPLAIN QUERY PLAN; hardware cycles are inclusive and must not be summed as CPU time"})
}
