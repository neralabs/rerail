//! Statement-start accounting shares SQLite's existing profiler hook when profiling is enabled.
use super::*;
use rusqlite::{Connection, ffi};
use std::collections::HashMap;
use std::ffi::{CStr, c_int, c_uint, c_void};

pub struct Observed {
    database: usize,
    ledger: Arc<Ledger>,
    reservation: Option<Arc<Reservation>>,
    running: Mutex<HashMap<usize, Running>>,
}
struct Running {
    receipt: Option<(String, [i32; 4])>,
    vm_before: Option<i32>,
}
impl Observed {
    pub fn ledger(&self) -> Arc<Ledger> {
        self.ledger.clone()
    }
    pub(crate) fn start(&self, statement: *mut ffi::sqlite3_stmt) {
        let mut running = self.running.lock();
        if running.contains_key(&(statement as usize)) {
            return;
        }
        // SAFETY: SQLite supplies a live statement to its synchronous trace callback.
        let sql = unsafe { ffi::sqlite3_sql(statement) };
        let sql = if sql.is_null() {
            std::borrow::Cow::Borrowed("")
        } else {
            unsafe { CStr::from_ptr(sql) }.to_string_lossy()
        };
        let receipt = self.ledger.listening().then(|| {
            let counters = statuses().map(|s| unsafe { ffi::sqlite3_stmt_status(statement, s, 0) });
            (sql.to_string(), counters)
        });
        let vm_before = self.ledger.measure_vm_steps.load(Relaxed).then(|| {
            receipt.as_ref().map_or_else(
                || unsafe {
                    ffi::sqlite3_stmt_status(statement, ffi::SQLITE_STMTSTATUS_VM_STEP, 0)
                },
                |(_, counters)| counters[0],
            )
        });
        running.insert(statement as usize, Running { receipt, vm_before });
        drop(running);
        let first = sql.split_whitespace().next().unwrap_or("");
        let recovery =
            first.eq_ignore_ascii_case("ROLLBACK") || first.eq_ignore_ascii_case("RELEASE");
        let admitted = self
            .reservation
            .as_ref()
            .map_or_else(|| self.ledger.charge(recovery, false), |r| r.charge());
        if !admitted {
            // SAFETY: the owned connection remains live; SQLite supports interruption here.
            unsafe { ffi::sqlite3_interrupt(self.database as *mut ffi::sqlite3) };
        }
    }
    pub(crate) fn end(&self, statement: *mut ffi::sqlite3_stmt) {
        let running = self.running.lock().remove(&(statement as usize));
        if let Some(Running { receipt, vm_before }) = running {
            let mut measured_steps = None;
            if let Some((sql, before)) = receipt {
                let after =
                    statuses().map(|s| unsafe { ffi::sqlite3_stmt_status(statement, s, 0) });
                measured_steps = Some(after[0]);
                self.ledger.record(Receipt {
                    sql,
                    counters: std::array::from_fn(|i| {
                        after[i].saturating_sub(before[i]).max(0) as u64
                    }),
                });
            }
            if let Some(before) = vm_before {
                let after = measured_steps.unwrap_or_else(|| unsafe {
                    ffi::sqlite3_stmt_status(statement, ffi::SQLITE_STMTSTATUS_VM_STEP, 0)
                });
                self.ledger
                    .vm_steps
                    .fetch_add(after.saturating_sub(before).max(0) as u64, Relaxed);
            }
        }
    }
}
pub struct Observer {
    pub shared: Arc<Observed>,
}
impl Observer {
    /// A later profiler can adopt `shared`.
    ///
    /// # Safety
    ///
    /// The observer must drop before the connection: it registers a raw pointer to itself with SQLite.
    pub unsafe fn install(
        db: &Connection,
        ledger: Arc<Ledger>,
        reservation: Option<Arc<Reservation>>,
    ) -> Box<Self> {
        let observer = Box::new(Self {
            shared: Arc::new(Observed {
                database: unsafe { db.handle() } as usize,
                ledger,
                reservation,
                running: Mutex::new(HashMap::new()),
            }),
        });
        unsafe {
            ffi::sqlite3_trace_v2(
                db.handle(),
                ffi::SQLITE_TRACE_STMT | ffi::SQLITE_TRACE_PROFILE,
                Some(trace),
                Arc::as_ptr(&observer.shared) as *mut c_void,
            );
        }
        observer
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_trace_v2(
                self.shared.database as *mut ffi::sqlite3,
                0,
                None,
                std::ptr::null_mut(),
            );
        }
    }
}
unsafe extern "C" fn trace(
    code: c_uint,
    data: *mut c_void,
    stmt: *mut c_void,
    _: *mut c_void,
) -> c_int {
    if crate::code_rule_dsl::sql_profile::explaining() {
        return 0;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let observer = unsafe { &*data.cast::<Observed>() };
        match code {
            ffi::SQLITE_TRACE_STMT => observer.start(stmt.cast()),
            ffi::SQLITE_TRACE_PROFILE => observer.end(stmt.cast()),
            _ => {}
        }
    }));
    if result.is_err() {
        let observer = unsafe { &*data.cast::<Observed>() };
        unsafe {
            ffi::sqlite3_interrupt(observer.database as *mut ffi::sqlite3);
        }
    }
    0
}

fn statuses() -> [i32; 4] {
    [
        ffi::SQLITE_STMTSTATUS_VM_STEP,
        ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP,
        ffi::SQLITE_STMTSTATUS_SORT,
        ffi::SQLITE_STMTSTATUS_AUTOINDEX,
    ]
}
