//! Complete executed-query trace. Callbacks only read SQLite counters; EXPLAIN runs later,
//! outside callbacks and timed spans, while the connection's schema still exists.

use super::timing::{Started, Timing};
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

static DIRECTORY: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| std::env::var_os("RERAIL_SQL_PROFILE_DIR").map(PathBuf::from));
static ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);
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
}

/// Cumulative callback counts, read at statement boundaries. No clocks in row callbacks.
#[derive(Clone, Copy)]
pub(super) enum FrameEvent {
    Scan,
    Lookup,
    Next,
    Column,
    Insert,
    IndexBuild,
    IndexedRows,
}

#[inline]
pub(super) fn frame(event: FrameEvent, count: usize) {
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
pub(super) fn index_build(rows: usize) -> IndexBuild {
    let started = (ACTIVE.get() != 0).then(Started::now);
    frame(FrameEvent::IndexBuild, 1);
    frame(FrameEvent::IndexedRows, rows);
    IndexBuild(started)
}

pub(super) struct IndexBuild(Option<Started>);

/// Standalone replay measurements use counters without installing a connection trace.
#[cfg(test)]
pub(super) fn frame_counts<T>(body: impl FnOnce() -> T) -> (T, [u64; 9]) {
    struct Active;
    impl Drop for Active {
        fn drop(&mut self) {
            ACTIVE.set(ACTIVE.get() - 1);
        }
    }
    let before = FRAME.get();
    ACTIVE.set(ACTIVE.get() + 1);
    let _active = Active;
    let result = body();
    (result, std::array::from_fn(|i| FRAME.get()[i] - before[i]))
}

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

struct Running {
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
    running: HashMap<usize, Running>,
    ids: HashMap<String, usize>,
    queries: Vec<Value>,
    relational: Vec<Value>,
    context: Value,
    sql_time: Timing,
    error: Option<String>,
    events: usize,
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
pub(super) struct Profiler {
    database: *mut ffi::sqlite3,
    state: RefCell<State>,
    stem: PathBuf,
}

// SAFETY: rusqlite's Connection may move between workers. Its callback is invoked synchronously
// by that connection only; neither the pointer nor RefCell is accessed concurrently.
unsafe impl Send for Profiler {}

impl Profiler {
    /// # Safety
    /// The returned profiler must be dropped before its connection closes.
    pub(super) unsafe fn configured(
        connection: &Connection,
    ) -> rusqlite::Result<Option<Box<Self>>> {
        DIRECTORY
            .as_ref()
            // SAFETY: forwarded from the caller; Session owns the callback and connection.
            .map(|path| unsafe { Self::new(connection, path) })
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
        Ok(this)
    }

    pub(super) fn operation(
        &self,
        sql: &str,
        at: (&str, usize),
        depth: usize,
        inputs: Vec<(String, i64)>,
    ) -> Operation<'_> {
        let mut state = self.state.borrow_mut();
        let query = state.query(sql, "operator");
        let context =
            json!({"scope":at.0,"statement":at.1,"depth":depth,"operator":query,"inputs":inputs});
        let previous = std::mem::replace(&mut state.context, context);
        Operation {
            profiler: self,
            query,
            previous: Some(previous),
            sql_time: state.sql_time,
            started: Started::now(),
            start_ns: now(),
            rows: None,
        }
    }

    /// Explains every newly observed statement without running its body or tracing EXPLAIN.
    pub(super) fn checkpoint(&self, connection: &Connection) -> rusqlite::Result<()> {
        self.state.borrow_mut().enabled = false;
        let result = {
            let pending: Vec<_> = self
                .state
                .borrow()
                .queries
                .iter()
                .filter(|q| q["kind"] == "sql" && q.get("plan").is_none())
                .map(|q| {
                    (
                        q["id"].as_u64().unwrap() as usize,
                        q["sql"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            for (id, sql) in pending {
                let plan = explain(connection, &format!("EXPLAIN QUERY PLAN {sql}"));
                let bytecode = explain(connection, &format!("EXPLAIN {sql}"));
                let mut state = self.state.borrow_mut();
                state.queries[id]["plan"] = plan;
                state.queries[id]["bytecode"] = bytecode;
            }
            self.save()
        };
        self.state.borrow_mut().enabled = true;
        result
    }

    pub(super) fn relational(&self, profile: Value) {
        self.state.borrow_mut().relational.push(profile);
    }

    fn save(&self) -> rusqlite::Result<()> {
        let mut state = self.state.borrow_mut();
        if let Some(writer) = &mut state.writer {
            writer.flush().map_err(io_error)?;
        }
        if let Some(error) = &state.error {
            return Err(io_error(std::io::Error::other(error.clone())));
        }
        let data = json!({"version":1,"pid":std::process::id(),"events":state.events,
            "clock":"process monotonic nanoseconds","cpuClock":if cfg!(unix) {"thread CPU"} else {"unavailable"},
            "writtenAtUnixMs":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis(),
            "queries":state.queries,"relationalBatches":state.relational});
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
        }
    }
}

pub(super) struct Operation<'a> {
    profiler: &'a Profiler,
    query: usize,
    previous: Option<Value>,
    sql_time: Timing,
    started: Started,
    start_ns: u64,
    rows: Option<usize>,
}

impl Operation<'_> {
    pub(super) fn finish(mut self, rows: usize) {
        self.rows = Some(rows);
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
            "rows":self.rows,"completed":self.rows.is_some(),"context":context,
            "thread":format!("{:?}",std::thread::current().id())}));
        state.context = self.previous.take().unwrap_or(Value::Null);
    }
}

fn now() -> u64 {
    ORIGIN.elapsed().as_nanos() as u64
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
    _: *mut c_void,
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
                let sql = unsafe { CStr::from_ptr(sql) }.to_string_lossy();
                let query = state.query(&sql, "sql");
                let context = state.context.clone();
                let counters = STATUSES.map(|status| unsafe {
                    i64::from(ffi::sqlite3_stmt_status(pointer, status, 0))
                });
                let changes = unsafe { ffi::sqlite3_total_changes64(profiler.database) };
                state.running.insert(
                    key,
                    Running {
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
                state.write(json!({"kind":"sql","query":running.query,"startNs":running.start_ns,
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
mod tests {
    use super::*;

    #[test]
    fn captures_helpers_cached_parameters_and_explains_without_executing() {
        let directory = std::env::temp_dir().join(format!(
            "rerail-profile-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let connection = Connection::open_in_memory().unwrap();
        // SAFETY: the profiler is explicitly dropped before the connection.
        let profiler = unsafe { Profiler::new(&connection, &directory) }.unwrap();
        connection
            .execute_batch("CREATE TABLE data(n INTEGER); BEGIN;")
            .unwrap();
        let mut statement = connection
            .prepare_cached("INSERT INTO data VALUES (?)")
            .unwrap();
        for n in 0..3 {
            statement.execute([n]).unwrap();
        }
        drop(statement);
        let operator =
            profiler.operation("result query", ("clause", 7), 2, vec![("data".into(), 3)]);
        for _ in 0..2 {
            let mut select = connection
                .prepare_cached("SELECT n FROM data ORDER BY n")
                .unwrap();
            let values = select
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(values, [0, 1, 2]);
        }
        operator.finish(6);
        connection.execute_batch("COMMIT").unwrap();
        profiler.checkpoint(&connection).unwrap();
        let catalogue: Value =
            serde_json::from_slice(&std::fs::read(profiler.stem.with_extension("json")).unwrap())
                .unwrap();
        let events: Vec<Value> = std::fs::read_to_string(profiler.stem.with_extension("jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let queries = catalogue["queries"].as_array().unwrap();
        for query in queries.iter().filter(|q| q["kind"] == "sql") {
            if query["sql"].as_str().unwrap().starts_with("CREATE TABLE") {
                // Deferred EXPLAIN cannot recreate a table that now exists. Retain the reason.
                assert!(
                    query["plan"]["error"]
                        .as_str()
                        .unwrap()
                        .contains("already exists")
                );
            } else {
                assert!(
                    query["plan"].is_array() && query["bytecode"].is_array(),
                    "{query}"
                );
            }
        }
        assert!(
            !queries
                .iter()
                .any(|q| q["sql"].as_str().unwrap().starts_with("EXPLAIN"))
        );
        let inserts: Vec<_> = events
            .iter()
            .filter(|event| {
                queries[event["query"].as_u64().unwrap() as usize]["sql"]
                    == "INSERT INTO data VALUES (?)"
            })
            .collect();
        assert_eq!(inserts.len(), 3);
        assert!(inserts.iter().all(|e| e["changes"] == 1));
        assert_eq!(inserts[0]["counters"], inserts[1]["counters"]);
        let selects: Vec<_> = events
            .iter()
            .filter(|event| {
                queries[event["query"].as_u64().unwrap() as usize]["sql"]
                    == "SELECT n FROM data ORDER BY n"
            })
            .collect();
        assert_eq!(selects.len(), 2);
        assert_eq!(selects[0]["counters"], selects[1]["counters"]);
        assert_eq!(selects[0]["rows"], 3);
        assert_eq!(selects[0]["context"]["statement"], 7);
        let parent = events.iter().find(|e| e["kind"] == "operator").unwrap();
        let children_cpu: u64 = selects.iter().map(|e| e["cpuNs"].as_u64().unwrap()).sum();
        assert_eq!(
            parent["selfCpuNs"].as_u64().unwrap() + children_cpu,
            parent["cpuNs"].as_u64().unwrap()
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM data", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        drop(profiler);
        // The callback is unregistered and the connection remains usable.
        connection
            .execute("INSERT INTO data VALUES (9)", [])
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn distinguishes_virtual_frame_work_from_native_shared_projection() {
        let directory = std::env::temp_dir().join(format!(
            "rerail-profile-frame-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let connection = Connection::open_in_memory().unwrap();
        // SAFETY: the profiler is explicitly dropped before the connection.
        let profiler = unsafe { Profiler::new(&connection, &directory) }.unwrap();
        let frames = std::sync::Arc::new(super::super::frames::FrameStore::default());
        frames
            .register(&connection, "f(k INTEGER PRIMARY KEY, x INTEGER)")
            .unwrap();
        frames
            .register(&connection, "g(k INTEGER PRIMARY KEY, x INTEGER)")
            .unwrap();
        connection
            .execute("INSERT INTO f VALUES (1,10),(2,10),(3,20)", [])
            .unwrap();
        for _ in 0..2 {
            let mut statement = connection
                .prepare_cached("SELECT x FROM f WHERE x = 10")
                .unwrap();
            let values = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(values, [10, 10]);
        }
        let sql = "INSERT INTO g SELECT a.k, a.x FROM f a";
        let operator = profiler.operation(sql, ("clause", 0), 0, vec![("f".into(), 3)]);
        assert_eq!(frames.select(&connection, sql).unwrap(), Some(3));
        operator.finish(3);
        profiler.checkpoint(&connection).unwrap();
        let state = profiler.state.borrow();
        assert!(
            !state
                .queries
                .iter()
                .any(|q| q["kind"] == "sql" && q["sql"] == sql)
        );
        let select_id = state
            .queries
            .iter()
            .find(|q| q["sql"] == "SELECT x FROM f WHERE x = 10")
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        drop(state);
        let events: Vec<Value> = std::fs::read_to_string(profiler.stem.with_extension("jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let selects: Vec<_> = events.iter().filter(|e| e["query"] == select_id).collect();
        assert_eq!(selects.len(), 2);
        assert_eq!(selects[0]["frame"][FrameEvent::Lookup as usize], 1);
        assert_eq!(selects[0]["frame"][FrameEvent::IndexBuild as usize], 1);
        assert_eq!(selects[0]["frame"][FrameEvent::IndexedRows as usize], 3);
        assert_eq!(selects[1]["frame"][FrameEvent::IndexBuild as usize], 0);
        assert!(selects[0]["frame"][8].as_u64().unwrap() > 0);
        assert_eq!(selects[1]["frame"][7], 0);
        assert_eq!(selects[1]["frame"][8], 0);
        assert!(selects[0]["frame"][7].as_u64().unwrap() <= selects[0]["cpuNs"].as_u64().unwrap());
        assert!(selects[0]["frame"][8].as_u64().unwrap() <= selects[0]["wallNs"].as_u64().unwrap());
        assert!(
            selects[0]["frame"][FrameEvent::Column as usize]
                .as_u64()
                .unwrap()
                >= 2
        );
        assert!(
            events
                .iter()
                .any(|e| e["kind"] == "operator" && e["rows"] == 3)
        );
        drop(profiler);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn incomplete_operator_and_failed_query_remain_visible() {
        let directory = std::env::temp_dir().join(format!(
            "rerail-profile-error-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let connection = Connection::open_in_memory().unwrap();
        // SAFETY: the profiler is explicitly dropped before the connection.
        let profiler = unsafe { Profiler::new(&connection, &directory) }.unwrap();
        connection
            .execute_batch("CREATE TABLE data(n INTEGER UNIQUE); INSERT INTO data VALUES(1)")
            .unwrap();
        {
            let _operator = profiler.operation("failed insert", ("stage", 0), 0, Vec::new());
            assert!(
                connection
                    .execute("INSERT INTO data VALUES(1)", [])
                    .is_err()
            );
        }
        profiler.checkpoint(&connection).unwrap();
        let events: Vec<Value> = std::fs::read_to_string(profiler.stem.with_extension("jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            events
                .iter()
                .any(|e| e["kind"] == "operator" && e["completed"] == false)
        );
        assert!(profiler.state.borrow().running.is_empty());
        drop(profiler);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
