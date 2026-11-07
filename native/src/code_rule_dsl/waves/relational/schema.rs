//! Opaque, prepare-only schemas. SQLite parses declarations and validates queries;
//! registration executes no DDL and never expands a dependency into a SQL view.
use rusqlite::{
    Connection, Result,
    vtab::{Context, Filters, IndexInfo, Module, VTab, VTabConnection, VTabCursor},
};
use std::{
    borrow::Cow,
    ffi::{CStr, CString, c_int},
};
#[path = "schema/cache.rs"]
pub(crate) mod cache;
#[path = "schema/inference.rs"]
pub(crate) mod inference;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Column {
    pub(crate) name: String,
    pub(crate) declared_type: String,
}
pub(crate) type Columns = Vec<Column>;
pub(crate) type SharedColumns = std::sync::Arc<Columns>;
pub(crate) type Registry = std::collections::BTreeMap<String, SharedColumns>;

pub(crate) fn describe(db: &Connection, name: &str) -> Result<Columns> {
    Ok(db
        .prepare(&format!("SELECT * FROM {}", quote(name)))?
        .columns()
        .iter()
        .map(|column| Column {
            name: column.name().to_owned(),
            declared_type: column.decl_type().unwrap_or("").to_owned(),
        })
        .collect())
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub(crate) fn declaration(name: &str, columns: &[Column]) -> String {
    format!(
        "CREATE TABLE {} ({})",
        quote(name),
        columns
            .iter()
            .map(|column| {
                if column.declared_type.is_empty() {
                    quote(&column.name)
                } else {
                    format!("{} {}", quote(&column.name), quote(&column.declared_type))
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn failure(message: &str) -> rusqlite::Error {
    rusqlite::Error::ModuleError(message.to_owned())
}

pub(crate) fn register(db: &Connection, name: &str, declaration: String) -> Result<Columns> {
    install(db, name, declaration)?;
    describe(db, name)
}

/// Resolve and validate an already-known declaration without copying its columns again.
pub(crate) fn bind(db: &Connection, name: &str, declaration: String) -> Result<()> {
    install(db, name, declaration)?;
    db.prepare(&format!("SELECT * FROM {}", quote(name)))?;
    Ok(())
}

fn install(db: &Connection, name: &str, declaration: String) -> Result<()> {
    // CREATE TABLE rejects SQLite's reserved object names, even for unused inputs.
    if name.to_ascii_lowercase().starts_with("sqlite_") {
        return Err(failure("object name reserved for internal use"));
    }
    let module = CString::new(name).map_err(|_| failure("invalid schema name"))?;
    let declaration =
        CString::new(declaration).map_err(|_| failure("invalid schema declaration"))?;
    const MODULE: Module<Schema> = Module::eponymous_only_module();
    db.create_module(module.as_c_str(), &MODULE, Some(declaration))?;
    // Both callers force xConnect outside the authored-query authorizer. Unused
    // declarations still receive SQLite's duplicate-column and schema checks.
    Ok(())
}

#[repr(C)]
struct Schema {
    base: rusqlite::ffi::sqlite3_vtab,
}
#[repr(C)]
struct Cursor {
    base: rusqlite::ffi::sqlite3_vtab_cursor,
}

// SAFETY: Both structs place the SQLite base first. xConnect copies the owned
// declaration through SQLite; neither a table nor a cursor borrows its bytes.
unsafe impl<'a> VTab<'a> for Schema {
    type Aux = CString;
    type Cursor = Cursor;
    fn connect(
        _: &mut VTabConnection,
        aux: Option<&CString>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        Ok((
            Cow::Owned(
                aux.ok_or_else(|| failure("missing compiler schema"))?
                    .clone(),
            ),
            Self {
                base: rusqlite::ffi::sqlite3_vtab::default(),
            },
        ))
    }
    fn best_index(&self, _: &mut IndexInfo) -> Result<bool> {
        Ok(true)
    }
    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: rusqlite::ffi::sqlite3_vtab_cursor::default(),
        })
    }
}

// SAFETY: The SQLite base is first and the cursor has no row storage or borrows.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, _: c_int, _: Option<&str>, _: &Filters<'_>) -> Result<()> {
        Err(failure("compiler schemas cannot be executed"))
    }
    fn next(&mut self) -> Result<()> {
        Err(failure("compiler schemas cannot be executed"))
    }
    fn eof(&self) -> bool {
        true
    }
    fn column(&self, _: &mut Context, _: c_int) -> Result<()> {
        Err(failure("compiler schemas cannot be executed"))
    }
    fn rowid(&self) -> Result<i64> {
        Err(failure("compiler schemas cannot be executed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn registration_executes_no_sql_and_rejects_execution() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let executions = AtomicUsize::new(0);
        unsafe extern "C" fn trace(
            _: u32,
            state: *mut std::ffi::c_void,
            _: *mut std::ffi::c_void,
            _: *mut std::ffi::c_void,
        ) -> c_int {
            // SAFETY: the counter remains live until the callback is removed below.
            unsafe { &*state.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
            0
        }
        // SAFETY: the connection and callback counter are live on this thread.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                db.handle(),
                rusqlite::ffi::SQLITE_TRACE_STMT,
                Some(trace),
                (&executions as *const AtomicUsize).cast_mut().cast(),
            );
        }
        let result = (|| -> Result<()> {
            register(
                &db,
                "input",
                "CREATE TABLE input(path TEXT, id INTEGER, value)".to_owned(),
            )?;
            db.prepare("SELECT i.path,i.id,j.value FROM input i JOIN input j ON i.id=j.id")?;
            bind(
                &db,
                "cached",
                "CREATE TABLE cached(path TEXT,id INT)".to_owned(),
            )?;
            db.prepare("SELECT path,id FROM cached")?;
            Ok(())
        })();
        // SAFETY: remove the callback before its counter goes out of scope.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut());
        }
        result?;
        assert_eq!(executions.load(Ordering::Relaxed), 0);
        assert!(
            db.query_row("SELECT path FROM input", [], |row| row.get::<_, String>(0))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn preserves_column_names_and_ctas_affinity_without_evaluating_expressions() -> Result<()> {
        let normal = Connection::open_in_memory()?;
        let compiler = Connection::open_in_memory()?;
        let declaration = "CREATE TABLE input(__path TEXT NOT NULL, __start INTEGER NOT NULL, __end INTEGER NOT NULL, value, real_value REAL, numeric_value NUMERIC, bytes BLOB)";
        normal.execute_batch(declaration)?;
        register(&compiler, "input", declaration.to_owned())?;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        compiler.create_scalar_function(
            "must_not_run",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::Relaxed);
                Err::<i64, _>(rusqlite::Error::UserFunctionError(
                    "authored expression executed".into(),
                ))
            },
        )?;
        let query = "SELECT __path,__start,__end,value,real_value,numeric_value,bytes,CAST(value AS TEXT) AS cast_text,CAST(value AS INTEGER) AS cast_int,__start+1 AS expression FROM input";
        for db in [&normal, &compiler] {
            db.execute_batch(&format!(
                "CREATE TEMP TABLE stage AS SELECT * FROM ({query}) LIMIT 0"
            ))?;
        }
        fn columns(db: &Connection) -> Result<Vec<(String, String)>> {
            db.prepare("PRAGMA table_info(stage)")?
                .query_map([], |r| Ok((r.get(1)?, r.get(2)?)))?
                .collect()
        }
        assert_eq!(columns(&normal)?, columns(&compiler)?);
        assert_eq!(
            columns(&compiler)?
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>(),
            [
                "TEXT", "INT", "INT", "", "REAL", "NUM", "", "TEXT", "INT", ""
            ]
        );
        compiler.execute_batch(
            "CREATE TEMP TABLE deferred AS SELECT must_not_run() AS value FROM input LIMIT 0",
        )?;
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[test]
    fn validates_unused_declarations_and_keeps_unknown_columns_invalid() -> Result<()> {
        let db = Connection::open_in_memory()?;
        assert!(
            register(
                &db,
                "duplicate",
                "CREATE TABLE duplicate(value, VALUE)".to_owned()
            )
            .is_err()
        );
        assert!(
            register(
                &db,
                "sqlite_reserved",
                "CREATE TABLE sqlite_reserved(value)".to_owned()
            )
            .is_err()
        );
        register(
            &db,
            "input",
            "CREATE TABLE input(path TEXT, id INTEGER)".to_owned(),
        )?;
        assert!(db.prepare("SELECT missing FROM input").is_err());
        assert!(db.prepare("SELECT * FROM other").is_err());
        assert!(
            db.execute_batch("CREATE VIRTUAL TABLE leak USING input")
                .is_err()
        );
        Ok(())
    }
}
