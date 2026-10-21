use super::{graph::Graph, profile::Profile};
use crate::code_rule_dsl::sql::{frames::FrameStore, timing::Started};
use rusqlite::{
    Connection, Result, StatementStatus, ffi,
    functions::{Aggregate, Context, FunctionFlags},
    types::Value,
};
use serde_json::{Value as Json, json};
use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
struct Buffer {
    rows: Vec<Vec<Value>>,
}
struct Publish {
    frames: AssertUnwindSafe<Arc<FrameStore>>,
    name: String,
    calls: Arc<AtomicUsize>,
    rows: Arc<AtomicUsize>,
    cells: Arc<AtomicUsize>,
    profile: Arc<Profile>,
}
impl Aggregate<Buffer, String> for Publish {
    fn init(&self, _: &mut Context<'_>) -> Result<Buffer> {
        Ok(Buffer::default())
    }
    fn step(&self, ctx: &mut Context<'_>, acc: &mut Buffer) -> Result<()> {
        acc.rows.push(
            (0..ctx.len())
                .map(|i| ctx.get(i))
                .collect::<Result<Vec<Value>>>()?,
        );
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, acc: Option<Buffer>) -> Result<String> {
        let rows = acc.unwrap_or_default().rows;
        let started = Started::now();
        self.frames.0.publish_spike_rows(&self.name, &rows)?;
        self.profile.publication(
            &self.name,
            rows.len(),
            rows.iter().map(Vec::len).sum(),
            started.elapsed().cpu,
        );
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.rows.fetch_add(rows.len(), Ordering::Relaxed);
        self.cells
            .fetch_add(rows.iter().map(Vec::len).sum(), Ordering::Relaxed);
        Ok(self.name.clone())
    }
}
struct Trace {
    count: AtomicUsize,
    statement: AtomicUsize,
}

unsafe extern "C" fn trace(
    mask: u32,
    context: *mut std::ffi::c_void,
    statement: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> i32 {
    if mask == ffi::SQLITE_TRACE_STMT {
        // SAFETY: execute keeps this counter alive until it removes the trace callback.
        let state = unsafe { &*(context as *const Trace) };
        state.count.fetch_add(1, Ordering::Relaxed);
        state.statement.store(statement as usize, Ordering::Relaxed);
    }
    0
}
#[derive(Debug)]
pub(super) struct Execution {
    pub hits: Vec<(usize, usize, u32, Json)>,
    pub metrics: Json,
}

struct TraceGuard<'a>(&'a Connection);
impl Drop for TraceGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
        // SAFETY: the connection outlives this guard; callbacks are removed before stack counters drop.
        unsafe {
            ffi::sqlite3_trace_v2(self.0.handle(), 0, None, std::ptr::null_mut());
        }
    }
}
pub(super) fn execute(
    connection: &Connection,
    frames: &Arc<FrameStore>,
    graph: &Graph,
    sql: &str,
) -> Result<Execution> {
    execute_program(connection, frames, graph, &[sql.to_owned()], false)
}

pub(super) fn execute_program(
    connection: &Connection,
    frames: &Arc<FrameStore>,
    graph: &Graph,
    queries: &[String],
    profiling: bool,
) -> Result<Execution> {
    execute_observed(connection, frames, graph, queries, profiling, None)
}

/// Retain plans and partial counters when a candidate hits its execution budget or fails.
pub(super) fn execute_observed(
    connection: &Connection,
    frames: &Arc<FrameStore>,
    graph: &Graph,
    queries: &[String],
    profiling: bool,
    observation: Option<&mut Json>,
) -> Result<Execution> {
    super::scan::configure(connection, profiling)?;
    let profile = Profile::new(profiling);
    profile.register(connection)?;
    let calls = Arc::new(AtomicUsize::new(0));
    let published = Arc::new(AtomicUsize::new(0));
    let cells = Arc::new(AtomicUsize::new(0));
    for (handle, input) in &graph.handles {
        frames.clear(handle);
        let columns = graph.relations[*input]
            .columns
            .iter()
            .map(|c| {
                format!(
                    "{}{}",
                    c.name,
                    if c.name == "k" {
                        " INTEGER PRIMARY KEY"
                    } else if c.integer {
                        " INTEGER"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let names = &graph.relations[*input].columns;
        let mut declaration = columns;
        if names.iter().any(|c| c.name == "file") && names.iter().any(|c| c.name == "node") {
            declaration.push_str(", INDEX (file,node)");
        }
        if names.iter().any(|c| c.name == "pk") {
            declaration.push_str(", INDEX (pk)");
        }
        frames.register(connection, &format!("{handle}({declaration})"))?;
        connection.create_aggregate_function(
            format!("publish_{handle}").as_str(),
            -1,
            FunctionFlags::SQLITE_UTF8,
            Publish {
                frames: AssertUnwindSafe(Arc::clone(frames)),
                name: handle.clone(),
                calls: Arc::clone(&calls),
                rows: Arc::clone(&published),
                cells: Arc::clone(&cells),
                profile: Arc::clone(&profile),
            },
        )?;
    }
    let release_store = AssertUnwindSafe(Arc::clone(frames));
    connection.create_scalar_function(
        "release_buffers",
        2,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let token: String = ctx.get(0)?;
            let names: String = ctx.get(1)?;
            let names: Vec<String> = serde_json::from_str(&names)
                .map_err(|e| rusqlite::Error::ModuleError(e.to_string()))?;
            for name in names {
                release_store.0.clear(&name);
            }
            Ok(token)
        },
    )?;
    let mut hits = Vec::new();
    let mut steps = Vec::new();
    let mut error = None;
    for sql in queries {
        match execute_statement(connection, sql, &profile) {
            Ok((step, fault)) => {
                hits.extend(step.hits);
                steps.push(step.metrics);
                if fault.is_some() {
                    error = fault;
                    break;
                }
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    let metric_names = [
        "sqlExecutions",
        "prepareCpuNs",
        "prepareWallNs",
        "cpuNs",
        "wallNs",
        "vmSteps",
        "fullscanSteps",
        "sorts",
        "automaticIndexRows",
        "sqlBytes",
    ];
    let mut metrics = serde_json::Map::new();
    for name in metric_names {
        metrics.insert(
            name.into(),
            json!(
                steps
                    .iter()
                    .map(|m| m[name].as_u64().unwrap_or(0))
                    .sum::<u64>()
            ),
        );
    }
    metrics.insert("statements".into(), json!(steps));
    metrics.insert("publishedSets".into(), json!(calls.load(Ordering::Relaxed)));
    metrics.insert(
        "publishedRows".into(),
        json!(published.load(Ordering::Relaxed)),
    );
    metrics.insert(
        "publishedCells".into(),
        json!(cells.load(Ordering::Relaxed)),
    );
    metrics.insert("peakFrameRows".into(), json!(frames.peak()));
    metrics.insert("relations".into(), json!(graph.relations.len()));
    metrics.insert("sharedRelations".into(), json!(graph.intern_hits));
    if profiling {
        let diagnostic_started = Started::now();
        metrics.insert("profile".into(), profile.report(graph));
        let explain_started = Started::now();
        metrics.insert(
            "explain".into(),
            json!(
                queries
                    .iter()
                    .map(|sql| super::profile::explain(connection, sql))
                    .collect::<Vec<_>>()
            ),
        );
        metrics.insert("explainCpuNs".into(), json!(explain_started.elapsed().cpu));
        metrics.insert(
            "diagnosticCpuNs".into(),
            json!(diagnostic_started.elapsed().cpu),
        );
    }
    let mut metrics = Json::Object(metrics);
    if profiling {
        super::profile::summarize(&mut metrics, graph, &[]);
    }
    if let Some(observation) = observation {
        *observation = metrics.clone();
    }
    if let Some(error) = error {
        return Err(error);
    }
    Ok(Execution { hits, metrics })
}

fn execute_statement(
    connection: &Connection,
    sql: &str,
    profile: &Arc<Profile>,
) -> Result<(Execution, Option<rusqlite::Error>)> {
    let prepared = Started::now();
    let mut statement = connection.prepare(sql)?;
    let prepare = prepared.elapsed();
    let count = Trace {
        count: AtomicUsize::new(0),
        statement: AtomicUsize::new(0),
    };
    // Count actual SQLite statement executions, including any nested executions in callbacks.
    unsafe {
        ffi::sqlite3_trace_v2(
            connection.handle(),
            ffi::SQLITE_TRACE_STMT,
            Some(trace),
            (&count as *const Trace).cast_mut().cast(),
        );
    }
    let trace_guard = TraceGuard(connection);
    let ticks = Arc::new(AtomicUsize::new(0));
    let progress = Arc::clone(&ticks);
    let wall = Instant::now();
    let sampled = Arc::clone(profile);
    connection.progress_handler(
        10_000,
        Some(move || {
            sampled.sampled_vm.fetch_add(10_000, Ordering::Relaxed);
            progress.fetch_add(1, Ordering::Relaxed) > 50_000 || wall.elapsed().as_secs() > 30
        }),
    )?;
    profile.sampled_vm.store(0, Ordering::Relaxed);
    let started = Started::now();
    let result = (|| -> Result<_> {
        let mut rows = statement.query([])?;
        let mut hits = Vec::new();
        while let Some(row) = rows.next()? {
            let clause: i64 = row.get(0)?;
            if clause >= 0 {
                let payload: String = row.get(3)?;
                hits.push((
                    clause as usize,
                    row.get::<_, i64>(1)? as usize,
                    row.get::<_, i64>(2)? as u32,
                    serde_json::from_str(&payload)
                        .map_err(|e| rusqlite::Error::ModuleError(e.to_string()))?,
                ));
            }
        }
        Ok(hits)
    })();
    let elapsed = started.elapsed();
    drop(trace_guard);
    let executions = count.count.load(Ordering::Relaxed);
    assert_eq!(
        executions, 1,
        "whole-call execution must issue exactly one SQL statement"
    );
    let scan = if profile.enabled {
        super::scan::read(count.statement.load(Ordering::Relaxed))
    } else {
        Json::Null
    };
    let metrics = json!({"scans":scan,"sqlExecutions":executions,"prepareCpuNs":prepare.cpu,"prepareWallNs":prepare.wall,"cpuNs":elapsed.cpu,"wallNs":elapsed.wall,
        "vmSteps":statement.get_status(StatementStatus::VmStep),"fullscanSteps":statement.get_status(StatementStatus::FullscanStep),"sorts":statement.get_status(StatementStatus::Sort),
        "automaticIndexRows":statement.get_status(StatementStatus::AutoIndex),"sqlBytes":sql.len()});
    Ok(match result {
        Ok(hits) => (Execution { hits, metrics }, None),
        Err(e) => (
            Execution {
                hits: Vec::new(),
                metrics,
            },
            Some(e),
        ),
    })
}
