//! Complete executed-query trace. Callbacks only read SQLite counters; EXPLAIN runs later,
//! outside callbacks and timed spans, while the connection's schema still exists.

#[path = "sql/coverage.rs"]
mod coverage;
#[path = "sql/timing.rs"]
mod timing;
mod snapshot;
use rusqlite::{Connection, ffi};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, c_int, c_uint, c_void};
use std::fs::{File, create_dir_all};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime};
use timing::{Started, Timing};

static DIRECTORY: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| std::env::var_os("RERAIL_SQL_PROFILE_DIR").map(PathBuf::from));
static ORIGIN: LazyLock<(Instant, u128)> = LazyLock::new(|| {
    (
        Instant::now(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros(),
    )
});
static CONTEXT_FILTER: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("RERAIL_SQL_PROFILE_SCOPE").ok());
pub(in crate::code_rule_dsl) fn captures_scope(scope: &str) -> bool {
    DIRECTORY.is_some()
        && CONTEXT_FILTER
            .as_deref()
            .is_none_or(|wanted| wanted == scope)
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);
const STATUSES: [c_int; 4] = [
    ffi::SQLITE_STMTSTATUS_VM_STEP,
    ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP,
    ffi::SQLITE_STMTSTATUS_SORT,
    ffi::SQLITE_STMTSTATUS_AUTOINDEX,
];

thread_local! {
    static FRAME: Cell<[u64; 9]> = const { Cell::new([0; 9]) };
    static ACTIVE: Cell<usize> = const { Cell::new(0) };
    static EXPLAINING: Cell<bool> = const { Cell::new(false) };
}

pub(in crate::code_rule_dsl) fn explaining() -> bool {
    EXPLAINING.get()
}

struct Explanation(bool);
impl Drop for Explanation {
    fn drop(&mut self) {
        EXPLAINING.set(self.0);
    }
}

/// Cumulative callback counts, read at statement boundaries. No clocks in row callbacks.
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum FrameEvent {
    Scan,
    Lookup,
    Next,
    Column,
    IndexBuild = 5,
    IndexedRows,
}

#[inline]
pub(in crate::code_rule_dsl) fn frame(event: FrameEvent, count: usize) {
    if ACTIVE.get() != 0 {
        FRAME.with(|counter| {
            let mut values = counter.get();
            values[event as usize] += count as u64;
            counter.set(values);
        });
    }
}

/// Time only cache misses, including any scan-order preparation needed by the lookup.
/// These spans are contained in SQL time; they must not be added to the CPU budget.
pub(in crate::code_rule_dsl) fn index_build(rows: usize) -> IndexBuild {
    let started = (ACTIVE.get() != 0).then(Started::now);
    frame(FrameEvent::IndexBuild, 1);
    frame(FrameEvent::IndexedRows, rows);
    IndexBuild(started)
}

pub(in crate::code_rule_dsl) struct IndexBuild(Option<Started>);

impl Drop for IndexBuild {
    fn drop(&mut self) {
        if let Some(started) = self.0 {
            let elapsed = started.elapsed();
            FRAME.with(|counter| {
                let mut values = counter.get();
                values[7] += elapsed.cpu;
                values[8] += elapsed.wall;
                counter.set(values);
            });
        }
    }
}

const ORIGINS: [&str; 3] = ["engine", "sqliteInternal", "unknown"];

struct Running {
    origin: usize,
    started: Started,
    start_ns: u64,
    counters: [i64; 4],
    frame: [u64; 9],
    changes: i64,
    rows: u64,
    query: usize,
    context: Value,
}

#[derive(Default)]
struct State {
    explained: usize,
    running: HashMap<usize, Running>,
    ids: HashMap<String, usize>,
    queries: Vec<Value>,
    context: Value,
    sql_time: Timing,
    error: Option<String>,
    events: usize,
    filtered_notifications: usize,
    sql_started: usize,
    sql_completed: usize,
    origin_started: [usize; 3],
    origin_completed: [usize; 3],
    enabled: bool,
    writer: Option<BufWriter<File>>,
}

impl State {
    fn query(&mut self, sql: &str, kind: &str) -> usize {
        // Native descriptors and SQL cannot collide, but their catalogue uses one id space.
        let key = format!("{kind}:{sql}");
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let id = self.queries.len();
        self.ids.insert(key, id);
        self.queries.push(json!({"id":id,"kind":kind,"sql":sql}));
        id
    }

    fn write(&mut self, event: Value) {
        self.events += 1;
        if let Some(writer) = &mut self.writer {
            let result = serde_json::to_writer(&mut *writer, &event)
                .map_err(std::io::Error::other)
                .and_then(|()| writer.write_all(b"\n"));
            if let Err(error) = result {
                self.error = Some(error.to_string());
            }
        }
    }
}

/// Must drop before its connection. The Box keeps callback data stable when Session moves.
/// Each connection runs on one worker at a time, as required by rusqlite.
pub(in crate::code_rule_dsl) struct Profiler {
    database: *mut ffi::sqlite3,
    state: RefCell<State>,
    stem: PathBuf,
    context_filter: Option<String>,
}

// SAFETY: rusqlite's Connection may move between workers. Its callback is invoked synchronously
// by that connection only; neither the pointer nor RefCell is accessed concurrently.
unsafe impl Send for Profiler {}

impl Profiler {
    fn accepts(&self, context: &Value) -> bool {
        self.context_filter.as_deref().is_none_or(|wanted| {
            ["scope", "component"]
                .iter()
                .any(|key| context.get(*key).and_then(Value::as_str) == Some(wanted))
        })
    }
    pub(in crate::code_rule_dsl) fn context(&self, value: Value) {
        self.state.borrow_mut().context = value;
    }

    pub(in crate::code_rule_dsl) fn stage(&self, value: &str) {
        self.state.borrow_mut().context["stage"] = json!(value);
    }
    /// # Safety
    /// The returned profiler must be dropped before its connection closes.
    pub(in crate::code_rule_dsl) unsafe fn configured(
        connection: &Connection,
    ) -> rusqlite::Result<Option<Box<Self>>> {
        DIRECTORY
            .as_ref()
            // SAFETY: forwarded from the caller; Session owns the callback and connection.
            .map(|path| {
                unsafe { Self::new(connection, path) }.map(|mut profiler| {
                    profiler.context_filter = CONTEXT_FILTER.clone();
                    profiler
                })
            })
            .transpose()
    }

    unsafe fn new(connection: &Connection, directory: &Path) -> rusqlite::Result<Box<Self>> {
        create_dir_all(directory)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let stem = directory.join(format!(
            "profile-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let file = File::create(stem.with_extension("jsonl"))
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let mut this = Box::new(Self {
            // SAFETY: the profiler is owned by Session and dropped before this connection.
            database: unsafe { connection.handle() },
            state: RefCell::new(State {
                enabled: true,
                writer: Some(BufWriter::new(file)),
                ..State::default()
            }),
            stem,
            context_filter: None,
        });
        let _ = *ORIGIN;
        // SAFETY: SQLite retains the stable Box address until Drop unregisters this callback.
        unsafe {
            ffi::sqlite3_trace_v2(
                this.database,
                ffi::SQLITE_TRACE_STMT | ffi::SQLITE_TRACE_PROFILE | ffi::SQLITE_TRACE_ROW,
                Some(trace),
                (&mut *this as *mut Self).cast(),
            );
        }
        coverage::record(&this.stem, "opened")?;
        Ok(this)
    }

    pub(in crate::code_rule_dsl) fn operation(
        &self,
        sql: &str,
        at: (&str, usize),
        depth: usize,
        inputs: Vec<(String, i64)>,
    ) -> Operation<'_> {
        let mut state = self.state.borrow_mut();
        let query = state.query(sql, "operator");
        let mut context = state.context.clone();
        if !context.is_object() {
            context = json!({});
        }
        for (key, value) in
            json!({"scope":at.0,"statement":at.1,"depth":depth,"operator":query,"inputs":inputs})
                .as_object()
                .unwrap()
        {
            context[key] = value.clone();
        }
        let previous = std::mem::replace(&mut state.context, context);
        Operation {
            profiler: self,
            query,
            previous: Some(previous),
            sql_time: state.sql_time,
            started: Started::now(),
            start_ns: now(),
            rows: None,
            completed: false,
        }
    }

    /// Explains every newly observed statement without running its body or tracing EXPLAIN.
    pub(in crate::code_rule_dsl) fn checkpoint(
        &self,
        connection: &Connection,
    ) -> rusqlite::Result<()> {
        self.explain_pending(connection);
        self.save()
    }

    /// Capture plans before their physical inputs are released, without rewriting the report.
    pub(in crate::code_rule_dsl) fn explain_pending(&self, connection: &Connection) {
        let _explanation = Explanation(EXPLAINING.replace(true));
        self.state.borrow_mut().enabled = false;
        {
            let pending: Vec<_> = self
                .state
                .borrow()
                .queries
                .iter()
                .skip(self.state.borrow().explained)
                .filter(|q| q["kind"] == "sql" && q.get("plan").is_none())
                .map(|q| {
                    (
                        q["id"].as_u64().unwrap() as usize,
                        q["sql"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            for (id, sql) in pending {
                // CTAS already created its output table. Explain its SELECT body while
                // all input tables still exist, rather than attempting a second CREATE.
                let plan_sql = plan_sql(&sql);
                let plan = explain(connection, &format!("EXPLAIN QUERY PLAN {plan_sql}"));
                let bytecode = explain(connection, &format!("EXPLAIN {plan_sql}"));
                let mut state = self.state.borrow_mut();
                state.queries[id]["planSql"] = json!(plan_sql);
                state.queries[id]["plan"] = plan;
                state.queries[id]["bytecode"] = bytecode;
            }
            let mut state = self.state.borrow_mut();
            state.explained = state.queries.len();
        };
        self.state.borrow_mut().enabled = true;
    }

    fn save(&self) -> rusqlite::Result<()> {
        let mut state = self.state.borrow_mut();
        if let Some(writer) = &mut state.writer {
            writer.flush().map_err(io_error)?;
        }
        if let Some(error) = &state.error {
            return Err(io_error(std::io::Error::other(error.clone())));
        }
        let origins: serde_json::Map<String, Value> = ORIGINS.iter().enumerate().map(|(i, name)| ((*name).to_owned(), json!({"started":state.origin_started[i], "completed":state.origin_completed[i], "running":state.origin_started[i].saturating_sub(state.origin_completed[i])}))).collect();
        let data = json!({"version":1,"pid":std::process::id(),"events":state.events,
            "clock":"process monotonic nanoseconds","originUnixUs":ORIGIN.1,"cpuClock":if cfg!(unix) {"thread CPU"} else {"unavailable"},
            "writtenAtUnixMs":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis(),
            "queries":state.queries,"error":state.error,
            "executionCoverage":{"source":"code-rule-dsl","fromSessionStart":self.context_filter.is_none(),"contextFilter":self.context_filter,"filteredStatementNotifications":state.filtered_notifications,
                "started":state.sql_started,"completed":state.sql_completed,"running":state.running.len(), "origins":origins}});
        let file = File::create(self.stem.with_extension("json")).map_err(io_error)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &data)
            .map_err(|e| io_error(std::io::Error::other(e)))?;
        writer.flush().map_err(io_error)?;
        Ok(())
    }
}

fn io_error(error: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

impl Drop for Profiler {
    fn drop(&mut self) {
        // SAFETY: Session declares this field before its connection, so the handle is open.
        unsafe {
            ffi::sqlite3_trace_v2(self.database, 0, None, std::ptr::null_mut());
        }
        if let Err(error) = self.save() {
            eprintln!("SQL profile write failed: {error}");
        } else if let Err(error) = coverage::record(&self.stem, "closed") {
            eprintln!("SQL profile coverage write failed: {error}");
        }
    }
}

pub(in crate::code_rule_dsl) struct Operation<'a> {
    profiler: &'a Profiler,
    query: usize,
    previous: Option<Value>,
    sql_time: Timing,
    started: Started,
    start_ns: u64,
    rows: Option<usize>,
    completed: bool,
}

impl Operation<'_> {
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn finish(mut self, rows: usize) {
        self.rows = Some(rows);
        self.completed = true;
    }

    pub(in crate::code_rule_dsl) fn finish_unobserved(mut self) {
        self.completed = true;
    }
}

impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        let mut state = self.profiler.state.borrow_mut();
        let sql = state.sql_time.since(self.sql_time);
        let context = state.context.clone();
        state.write(json!({"kind":"operator","query":self.query,"startNs":self.start_ns,
            "wallNs":elapsed.wall,"cpuNs":elapsed.cpu,
            "selfWallNs":elapsed.wall.saturating_sub(sql.wall),"selfCpuNs":elapsed.cpu.saturating_sub(sql.cpu),
            "rows":self.rows,"completed":self.completed,"context":context,
            "thread":format!("{:?}",std::thread::current().id())}));
        state.context = self.previous.take().unwrap_or(Value::Null);
    }
}

fn now() -> u64 {
    ORIGIN.0.elapsed().as_nanos() as u64
}

fn plan_sql(sql: &str) -> &str {
    static CTAS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r#"(?is)^\s*CREATE\s+(?:TEMP(?:ORARY)?\s+)?TABLE\s+(?:"(?:[^"]|"")*"|[a-zA-Z_][a-zA-Z_0-9]*)\s+AS\s+(.+)$"#
    ).unwrap()
    });
    CTAS.captures(sql)
        .and_then(|c| c.get(1))
        .map_or(sql, |m| m.as_str())
}

fn explain(connection: &Connection, sql: &str) -> Value {
    let result = (|| {
        let mut statement = connection.prepare(sql)?;
        let count = statement.column_count();
        // Unbound parameters remain NULL. EXPLAIN returns instructions, never executes the body.
        let mut rows = statement.raw_query();
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            let mut values = Vec::new();
            for column in 0..count {
                use rusqlite::types::ValueRef;
                values.push(match row.get_ref(column)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(value) => json!(value),
                    ValueRef::Real(value) => json!(value),
                    ValueRef::Text(value) | ValueRef::Blob(value) => {
                        json!(String::from_utf8_lossy(value))
                    }
                });
            }
            result.push(values);
        }
        Ok::<_, rusqlite::Error>(result)
    })();
    result.map_or_else(|e| json!({"error":e.to_string()}), |rows| json!(rows))
}

/// Never unwinds across SQLite's C ABI. The callback does not prepare SQL, bind values, or
/// reset counters (the existing diagnostics also read them). Raw pointers live for this call.
unsafe extern "C" fn trace(
    code: c_uint,
    context: *mut c_void,
    statement: *mut c_void,
    trace_sql: *mut c_void,
) -> c_int {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: installed with the stable Profiler address; SQLite supplies a live statement.
        let profiler = unsafe { &*context.cast::<Profiler>() };
        let mut state = profiler.state.borrow_mut();
        if !state.enabled {
            return;
        }
        let pointer = statement.cast::<ffi::sqlite3_stmt>();
        let key = pointer as usize;
        if !profiler.accepts(&state.context) && !state.running.contains_key(&key) {
            if code == ffi::SQLITE_TRACE_STMT {
                state.filtered_notifications += 1;
            }
            return;
        }
        match code {
            ffi::SQLITE_TRACE_STMT => {
                // Ignore repeated trigger notifications for an already running statement.
                if state.running.contains_key(&key) {
                    return;
                }
                // SAFETY: SQLite owns this statement and its nul-terminated SQL until reset.
                let sql = unsafe { ffi::sqlite3_sql(pointer) };
                if sql.is_null() {
                    return;
                }
                let canonical = unsafe { CStr::from_ptr(sql) }.to_bytes();
                // SQLite prefixes nested VDBE execution with an extra "-- ". Compare
                // against canonical SQL so authored comments remain engine submissions.
                // An open result cursor alone never marks another host query as internal.
                let origin = if trace_sql.is_null() {
                    2
                } else {
                    let observed = unsafe { CStr::from_ptr(trace_sql.cast()) }.to_bytes();
                    if observed == canonical {
                        0
                    } else if observed.strip_prefix(b"-- ") == Some(canonical) {
                        1
                    } else {
                        2
                    }
                };
                let sql = String::from_utf8_lossy(canonical);
                let query = state.query(&sql, "sql");
                state.sql_started += 1;
                state.origin_started[origin] += 1;
                let context = state.context.clone();
                let counters = STATUSES.map(|status| unsafe {
                    i64::from(ffi::sqlite3_stmt_status(pointer, status, 0))
                });
                let changes = unsafe { ffi::sqlite3_total_changes64(profiler.database) };
                state.running.insert(
                    key,
                    Running {
                        origin,
                        started: Started::now(),
                        start_ns: now(),
                        counters,
                        changes,
                        rows: 0,
                        query,
                        context,
                        frame: FRAME.get(),
                    },
                );
                ACTIVE.set(ACTIVE.get() + 1);
            }
            ffi::SQLITE_TRACE_ROW => {
                if let Some(running) = state.running.get_mut(&key) {
                    running.rows += 1;
                }
            }
            ffi::SQLITE_TRACE_PROFILE => {
                let Some(running) = state.running.remove(&key) else {
                    return;
                };
                state.sql_completed += 1;
                state.origin_completed[running.origin] += 1;
                ACTIVE.set(ACTIVE.get().saturating_sub(1));
                let elapsed = running.started.elapsed();
                state.sql_time.add(elapsed);
                let counters = STATUSES.map(|status| unsafe {
                    i64::from(ffi::sqlite3_stmt_status(pointer, status, 0))
                });
                let counters: Vec<_> = counters
                    .iter()
                    .zip(running.counters)
                    .map(|(a, b)| a.saturating_sub(b))
                    .collect();
                let frame: Vec<_> = FRAME
                    .get()
                    .iter()
                    .zip(running.frame)
                    .map(|(a, b)| a.saturating_sub(b))
                    .collect();
                let changes = unsafe { ffi::sqlite3_total_changes64(profiler.database) }
                    .saturating_sub(running.changes);
                state.write(json!({"kind":"sql","origin":ORIGINS[running.origin],"query":running.query,"startNs":running.start_ns,
                    "wallNs":elapsed.wall,"cpuNs":elapsed.cpu,"selfWallNs":elapsed.wall,"selfCpuNs":elapsed.cpu,
                    "rows":running.rows,"changes":changes,"counters":counters,"frame":frame,
                    "context":running.context,"thread":format!("{:?}",std::thread::current().id())}));
            }
            _ => {}
        }
    }));
    if result.is_err() {
        // Do not let a broken diagnostic silently claim complete coverage.
        let profiler = unsafe { &*context.cast::<Profiler>() };
        if let Ok(mut state) = profiler.state.try_borrow_mut() {
            state.error = Some("SQLite trace callback panicked".to_owned());
        }
    }
    0
}

#[cfg(test)]
#[path = "sql/tests.rs"]
mod tests;
