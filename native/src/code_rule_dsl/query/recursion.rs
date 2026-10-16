//! Statements run from within a statement: a utility (`tq_util`, its depth limited as the matcher
//! limits `matches`, by `matcher::enter_util_level`), a parameterized call's utility (`tq_call`),
//! and a term too deep to stay inline. SQLite allows a function to step another statement of the
//! same connection; a statement cannot re-enter itself, so each nesting depth has its own
//! prepared copy, cached per connection for the connection's life.

use rusqlite::ffi;
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_int};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};

/// How deep statements may nest: every utility level may hold a few terms of its own statement.
/// The stack limit (`matcher::util_stack_exceeded`) is normally reached first.
pub(super) const MAX_DEPTH: u32 = 4 * 256;

struct Prepared(*mut ffi::sqlite3_stmt);

// SAFETY: a statement is only stepped by the thread holding its connection (the connection's
// slot lock in `with_file`), and finalized when its connection is released or dropped.
unsafe impl Send for Prepared {}

impl Drop for Prepared {
    fn drop(&mut self) {
        // SAFETY: the statement was prepared on a connection that is still open: entries are
        // removed (`forget`) before their connection closes.
        unsafe {
            ffi::sqlite3_finalize(self.0);
        }
    }
}

/// A nested statement's SQL, hashed once when it is compiled: violation its prepared statement then
/// reads the hash and the text's address, not the text (a utility's statement runs once per node
/// it is asked at, and its text may be tens of kilobytes).
#[derive(Default)]
pub(super) struct Sql {
    text: Arc<str>,
    hash: u64,
}

impl Sql {
    pub(super) fn new(text: String) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        Self {
            text: Arc::from(text),
            hash: hasher.finish(),
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.text
    }
}

/// Prepared statements by connection and depth, then by SQL text (the text is what a term index
/// means in the catalog that compiled it; indices alone differ between catalogs), found by the
/// text's hash. Keyed by the connection, not the thread, so a connection released from another
/// thread takes its statements with it.
type Statements = HashMap<(usize, u32), HashMap<u64, Vec<(Arc<str>, Prepared)>>>;
static PREPARED: LazyLock<Mutex<Statements>> = LazyLock::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Finalizes the statements prepared on a connection; call before it closes.
pub(super) fn forget(db: *mut ffi::sqlite3) {
    PREPARED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|&(connection, _), _| connection != db as usize);
}

/// How many recursive utility statements this thread is inside.
pub(super) fn depth() -> u32 {
    DEPTH.get()
}

fn error_message(db: *mut ffi::sqlite3) -> String {
    // SAFETY: `db` is the open connection the failing call ran on.
    unsafe { CStr::from_ptr(ffi::sqlite3_errmsg(db)) }
        .to_string_lossy()
        .into_owned()
}

/// Runs `sql` (one row, one column) with integer-or-NULL parameters at the next recursion depth.
/// `Err` carries SQLite's message, or the depth limit's.
pub(super) fn run(
    db: *mut ffi::sqlite3,
    sql: &Sql,
    parameters: &[Option<i64>],
) -> Result<Option<i64>, String> {
    let depth = DEPTH.get();
    if depth >= MAX_DEPTH || crate::code_rule_dsl::matcher::util_stack_exceeded() {
        return Err(crate::code_rule_dsl::matcher::UtilLimit::Depth
            .message()
            .to_owned());
    }
    let statement = (|| -> Result<*mut ffi::sqlite3_stmt, String> {
        let mut prepared = PREPARED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let level = prepared.entry((db as usize, depth)).or_default();
        let entries = level.entry(sql.hash).or_default();
        // The same text compiled into another catalog is the same statement; it keeps the latest
        // catalog's text, so that catalog's next lookups compare addresses only.
        for (text, statement) in entries.iter_mut() {
            if Arc::ptr_eq(text, &sql.text) {
                return Ok(statement.0);
            }
            if **text == *sql.text {
                *text = Arc::clone(&sql.text);
                return Ok(statement.0);
            }
        }
        let text = CString::new(sql.as_str()).map_err(|error| error.to_string())?;
        let mut statement = std::ptr::null_mut();
        // SAFETY: `db` is open; `text` outlives the call; the handle is finalized on drop.
        let code = unsafe {
            ffi::sqlite3_prepare_v2(db, text.as_ptr(), -1, &mut statement, std::ptr::null_mut())
        };
        if code != ffi::SQLITE_OK {
            return Err(error_message(db));
        }
        entries.push((Arc::clone(&sql.text), Prepared(statement)));
        Ok(statement)
    })()?;
    DEPTH.set(depth + 1);
    // SAFETY: `statement` belongs to `db`, is not running (each depth has its own), and is reset
    // before it is used again.
    let result = unsafe {
        for (index, parameter) in parameters.iter().enumerate() {
            let position = index as c_int + 1;
            match parameter {
                Some(value) => ffi::sqlite3_bind_int64(statement, position, *value),
                None => ffi::sqlite3_bind_null(statement, position),
            };
        }
        let code = ffi::sqlite3_step(statement);
        let result = match code {
            ffi::SQLITE_ROW => {
                if ffi::sqlite3_column_type(statement, 0) == ffi::SQLITE_NULL {
                    Ok(None)
                } else {
                    Ok(Some(ffi::sqlite3_column_int64(statement, 0)))
                }
            }
            ffi::SQLITE_DONE => Ok(None),
            _ => Err(error_message(db)),
        };
        ffi::sqlite3_reset(statement);
        ffi::sqlite3_clear_bindings(statement);
        result
    };
    DEPTH.set(depth);
    result
}
