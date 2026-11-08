//! Development probe: SQL session phases and SQLite work counters. Results stay outside the repo.
//! cargo run --profile profiling --example sql-executor-bench -- <store> <request.json>
use rerail_engine::tools::CodeRuleSqlExecutor;
use rusqlite::ffi;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, ffi::c_void, time::Instant};

#[derive(Default)]
struct Trace(RefCell<BTreeMap<String, [u64; 6]>>);

unsafe extern "C" fn trace(
    _: u32,
    context: *mut c_void,
    statement: *mut c_void,
    elapsed: *mut c_void,
) -> i32 {
    // SQLite calls this synchronously. The boxed state and connection outlive the registration.
    let state = unsafe { &*context.cast::<Trace>() };
    let statement = statement.cast::<ffi::sqlite3_stmt>();
    let (sql, counters) = unsafe {
        let sql = std::ffi::CStr::from_ptr(ffi::sqlite3_sql(statement))
            .to_string_lossy()
            .into_owned();
        let count = |op| ffi::sqlite3_stmt_status(statement, op, 1) as u64;
        (
            sql,
            [
                1,
                *elapsed.cast::<u64>(),
                count(ffi::SQLITE_STMTSTATUS_VM_STEP),
                count(ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP),
                count(ffi::SQLITE_STMTSTATUS_SORT),
                count(ffi::SQLITE_STMTSTATUS_AUTOINDEX),
            ],
        )
    };
    let mut rows = state.0.borrow_mut();
    let row = rows.entry(sql).or_default();
    for (a, b) in row.iter_mut().zip(counters) {
        *a += b;
    }
    0
}

#[cfg(unix)]
fn usage() -> (f64, f64) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    assert_eq!(result, 0);
    let usage = unsafe { usage.assume_init() };
    let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    // Darwin reports bytes; Linux reports KiB.
    let rss = usage.ru_maxrss as f64
        / if cfg!(target_os = "macos") {
            1048576.0
        } else {
            1024.0
        };
    (seconds(usage.ru_utime) + seconds(usage.ru_stime), rss)
}

#[cfg(not(unix))]
fn usage() -> (f64, f64) {
    (0.0, 0.0)
}

fn phase(
    trace: &Trace,
    session: &mut CodeRuleSqlExecutor,
    name: &str,
    phases: &mut Vec<Value>,
    run: impl FnOnce(&mut CodeRuleSqlExecutor) -> napi::Result<Value>,
) -> napi::Result<Value> {
    trace.0.borrow_mut().clear();
    let (cpu, _) = usage();
    let start = Instant::now();
    let before = session.sql_profile_totals();
    let result = run(session);
    let wall = start.elapsed().as_secs_f64();
    let (after, rss) = usage();
    let mut statements = std::mem::take(&mut *trace.0.borrow_mut())
        .into_iter()
        .collect::<Vec<_>>();
    statements.sort_by_key(|(_, c)| std::cmp::Reverse(c[2]));
    let mut total = [0u64; 6];
    for (_, counts) in &statements {
        for (a, b) in total.iter_mut().zip(counts) {
            *a += b;
        }
    }
    if let (Some(before), Some(after)) = (before, session.sql_profile_totals()) {
        total = std::array::from_fn(|i| after[i].saturating_sub(before[i]));
    }
    phases.push(json!({"phase":name, "wall_s":wall, "cpu_s":after-cpu, "peak_rss_mb":rss,
        "statements":total[0], "sqlite_ms":total[1] as f64/1e6, "vm_steps":total[2],
        "fullscan_steps":total[3], "sorts":total[4], "autoindex_rows":total[5],
        "sql":statements.into_iter().map(|(sql,c)|json!({"sql":sql,"runs":c[0],"ms":c[1] as f64/1e6,"vm":c[2],"scan":c[3],"sort":c[4],"autoindex":c[5]})).collect::<Vec<_>>(),
        "error":result.as_ref().err().map(ToString::to_string)}));
    result
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: sql-executor-bench <store> <request.json>".into());
    }
    let request: Value = serde_json::from_str(&std::fs::read_to_string(&args[1])?)?;
    let trace_state = Box::<Trace>::default();
    let mut phases = Vec::new();
    let (cpu, _) = usage();
    let start = Instant::now();
    let mut session = CodeRuleSqlExecutor::new(args[0].clone())?;
    phases.push(json!({"phase":"open","wall_s":start.elapsed().as_secs_f64(),"cpu_s":usage().0-cpu,"peak_rss_mb":usage().1}));
    let handle = unsafe { session.connection().handle() };
    let shared_profile = session.sql_profile_totals().is_some();
    if !shared_profile && std::env::var("RERAIL_SQL_BENCH_UNPROFILED").as_deref() != Ok("1") {
        unsafe {
            ffi::sqlite3_trace_v2(
                handle,
                ffi::SQLITE_TRACE_PROFILE as u32,
                Some(trace),
                std::ptr::from_ref(&*trace_state).cast_mut().cast(),
            );
        }
    }
    let result = (|| -> napi::Result<Value> {
        phase(&trace_state, &mut session, "bind", &mut phases, |session| {
            if let Some(snapshot) = request.get("snapshot") {
                session.bind_snapshot(snapshot.to_string())?;
            }
            if let Some(stored) = request.get("stored") {
                let paths = stored
                    .get("paths")
                    .map(|p| serde_json::from_value(p.clone()))
                    .transpose()
                    .map_err(|e| napi::Error::from_reason(e.to_string()))?;
                session.bind_stored(stored["scope"].as_str().unwrap().into(), paths)?;
            }
            for relation in request["relations"].as_array().into_iter().flatten() {
                session.bind_relation(relation.to_string())?;
            }
            for sql in request["setup_sql"].as_array().into_iter().flatten() {
                session
                    .connection()
                    .execute_batch(sql.as_str().unwrap())
                    .map_err(|e| napi::Error::from_reason(e.to_string()))?;
            }
            Ok(Value::Null)
        })?;
        let mut plan = Value::Null;
        let mut execution = Value::Null;
        if let Some(program) = request["program"].as_str() {
            let dialect = request["dialect"].as_str().unwrap_or("python");
            plan = phase(
                &trace_state,
                &mut session,
                "lower",
                &mut phases,
                |session| {
                    session.lower(program.into(), dialect.into()).and_then(|s| {
                        serde_json::from_str(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })
                },
            )?;
            execution = phase(&trace_state, &mut session, "run", &mut phases, |session| {
                session
                    .run(
                        program.into(),
                        dialect.into(),
                        request["max_waves"].as_u64().map(|n| n as u32),
                    )
                    .and_then(|s| {
                        serde_json::from_str(&s)
                            .map_err(|e| napi::Error::from_reason(e.to_string()))
                    })
            })?;
        }
        for (i, sql) in request["after_run_sql"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            phase(
                &trace_state,
                &mut session,
                &format!("after-run-{i}"),
                &mut phases,
                |session| {
                    session
                        .connection()
                        .execute_batch(sql.as_str().unwrap())
                        .map_err(|e| napi::Error::from_reason(e.to_string()))?;
                    Ok(Value::Null)
                },
            )?;
        }
        let mut updates = Vec::new();
        for (i, batch) in request["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            updates.push(phase(
                &trace_state,
                &mut session,
                &format!("update-{i}"),
                &mut phases,
                |session| {
                    session
                        .apply_changes(
                            batch.to_string(),
                            request["max_waves"].as_u64().map(|n| n as u32),
                        )
                        .and_then(|s| {
                            serde_json::from_str(&s)
                                .map_err(|e| napi::Error::from_reason(e.to_string()))
                        })
                },
            )?);
        }
        for (i, snapshot) in request["snapshots"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            updates.push(phase(
                &trace_state,
                &mut session,
                &format!("snapshot-{i}"),
                &mut phases,
                |session| {
                    session
                        .apply_snapshot(
                            snapshot.to_string(),
                            request["max_waves"].as_u64().map(|n| n as u32),
                        )
                        .and_then(|s| {
                            serde_json::from_str(&s)
                                .map_err(|e| napi::Error::from_reason(e.to_string()))
                        })
                },
            )?);
        }
        for (i, batch) in request["file_changes"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            updates.push(phase(
                &trace_state,
                &mut session,
                &format!("file-edit-{i}"),
                &mut phases,
                |session| {
                    session
                        .apply_file_changes(
                            batch.to_string(),
                            request["max_waves"].as_u64().map(|n| n as u32),
                        )
                        .and_then(|s| {
                            serde_json::from_str(&s)
                                .map_err(|e| napi::Error::from_reason(e.to_string()))
                        })
                },
            )?);
        }
        let mut results = Vec::new();
        for (i, query) in request["queries"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let sql = query.as_str().or_else(|| query["sql"].as_str()).unwrap();
            let parameters = query.get("parameters").map(Value::to_string);
            results.push(phase(
                &trace_state,
                &mut session,
                &format!("query-{i}"),
                &mut phases,
                |session| {
                    session
                        .query(sql.into(), parameters, Some(10000))
                        .and_then(|s| {
                            serde_json::from_str(&s)
                                .map_err(|e| napi::Error::from_reason(e.to_string()))
                        })
                },
            )?);
        }
        let mut repeated = Vec::new();
        if request["repeat"].as_bool() == Some(true) {
            let program = request["program"].as_str().unwrap();
            repeated.push(json!({"same_program": session.run(program.into(), "python".into(), None).map_err(|e|e.to_string())}));
        }
        Ok(
            json!({"execution":execution,"plan":plan,"queries":results,"repeated":repeated,"updates":updates}),
        )
    })();
    if !shared_profile {
        unsafe {
            ffi::sqlite3_trace_v2(handle, 0, None, std::ptr::null_mut());
        }
    }
    let output = json!({"request":args[1],"phases":phases,"result":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string)});
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
