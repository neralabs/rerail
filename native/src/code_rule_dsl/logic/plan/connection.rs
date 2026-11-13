//! Functions and adopted-input bindings live as long as their SQLite connection.
use rusqlite::{Connection, Result, ffi};
use std::ffi::c_void;
#[derive(Default)]
struct State {
    installed: u32,
    bindings: crate::code_rule_dsl::facts::vtab::Modules,
}
unsafe extern "C" fn destroy(pointer: *mut c_void) {
    unsafe {
        drop(Box::from_raw(pointer.cast::<State>()));
    }
}
fn state(db: &Connection) -> Result<*mut State> {
    // SAFETY: this private key holds only State. SQLite destroys it on close,
    // replacement, or installation failure. The connection serializes these calls.
    unsafe {
        let conn = db.handle();
        let key = c"rerail.plan.connection";
        let mut p = ffi::sqlite3_get_clientdata(conn, key.as_ptr()).cast::<State>();
        if p.is_null() {
            p = Box::into_raw(Box::new(State::default()));
            let code = ffi::sqlite3_set_clientdata(conn, key.as_ptr(), p.cast(), Some(destroy));
            if code != ffi::SQLITE_OK {
                return Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None));
            }
        }
        Ok(p)
    }
}
fn once(
    db: &Connection,
    bit: u32,
    enabled: bool,
    install: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !enabled {
        return install();
    }
    let marker = state(db)?;
    if unsafe { (*marker).installed } & bit != 0 {
        return Ok(());
    }
    install()?;
    unsafe {
        (*marker).installed |= bit;
    }
    Ok(())
}
pub(crate) fn run(db: &Connection, bit: u32, install: impl FnOnce() -> Result<()>) -> Result<()> {
    once(db, bit, true, install)
}
/// Keep adopted-input binding cells alive while cached statements reference them.
pub(crate) fn bind(
    db: &Connection,
    name: &str,
    source: std::sync::Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
) -> Result<()> {
    // SAFETY: state is owned by this live connection. Modules mutates its binding map internally.
    unsafe {
        (*state(db)?).bindings.bind(db, name, source)?;
    }
    Ok(())
}
#[test]
fn repeated_registration_expires_cached_statements_but_connection_setup_does_not() {
    use rusqlite::{StatementStatus, functions::FunctionFlags};
    for enabled in [false, true] {
        for _ in 0..2 {
            let db = Connection::open_in_memory().unwrap();
            let install = || {
                db.create_scalar_function(c"rr_spike_tick", 0, FunctionFlags::SQLITE_UTF8, |_| {
                    Ok(7i64)
                })
            };
            once(&db, 1, enabled, install).unwrap();
            let mut stmt = db.prepare_cached("SELECT rr_spike_tick()").unwrap();
            assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 7);
            stmt.reset_status(StatementStatus::RePrepare);
            once(&db, 1, enabled, install).unwrap();
            assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 7);
            assert_eq!(
                stmt.get_status(StatementStatus::RePrepare),
                i32::from(!enabled)
            );
        }
    }
}
