//! Compare captured production SQL schedules in isolated processes, including freed-page residency.
use crate::{
    clock::thread_cpu_ns,
    lifetime::{Action, Input, Schedule},
    store::Store,
    working_set,
};
use rusqlite::{Result, params_from_iter, types::Value};
use serde_json::{Value as Json, json};

fn heap(reset: bool) -> Result<(i64, i64)> {
    let (mut current, mut high) = (0, 0);
    // SAFETY: SQLite writes two scalar outputs. This isolated replay owns its SQLite workload.
    let code = unsafe {
        rusqlite::ffi::sqlite3_status64(
            rusqlite::ffi::SQLITE_STATUS_MEMORY_USED,
            &mut current,
            &mut high,
            i32::from(reset),
        )
    };
    if code != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ));
    }
    Ok((current, high))
}

pub fn run(capture: &std::path::Path, layout: &str, output: &std::path::Path) -> Result<()> {
    measure(
        capture,
        layout,
        output,
        (crate::session::CACHE_KIB, 512),
        working_set::snapshot,
        "ownership",
    )
}

/// Replay the selected production schedule with cheap counters and explicit physical budgets.
/// Unlike the ownership observer, this observer never scans retained database objects.
pub fn run_cache(
    capture: &std::path::Path,
    main_kib: i64,
    temp_kib: i64,
    output: &std::path::Path,
) -> Result<()> {
    if main_kib <= 0 || temp_kib <= 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    measure(
        capture,
        "scheduled",
        output,
        (main_kib, temp_kib),
        cache_snapshot,
        "cache-counters",
    )
}

fn cache_snapshot(db: &rusqlite::Connection, live: usize) -> Result<Json> {
    let mut counters = json!({});
    for (name, kind) in [
        ("cacheBytes", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED),
        ("cacheHits", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_HIT),
        ("cacheMisses", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS),
        ("cacheWrites", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE),
        ("cacheSpills", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_SPILL),
    ] {
        let (mut current, mut high) = (0i64, 0i64);
        // SAFETY: this replay thread owns the live connection and both scalar outputs.
        let code = unsafe {
            rusqlite::ffi::sqlite3_db_status64(db.handle(), kind, &mut current, &mut high, 0)
        };
        if code != rusqlite::ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ));
        }
        counters[name] = current.into();
    }
    Ok(json!({"liveRelations":live,"sqlite":counters}))
}

fn count_steps(statement: &rusqlite::Statement<'_>, totals: &mut [u64; 3]) {
    for (total, kind) in totals.iter_mut().zip([
        rusqlite::StatementStatus::VmStep,
        rusqlite::StatementStatus::FullscanStep,
        rusqlite::StatementStatus::Sort,
    ]) {
        *total += statement.get_status(kind) as u64;
    }
}

fn measure(
    capture: &std::path::Path,
    layout: &str,
    output: &std::path::Path,
    budget: (i64, i64),
    observe: fn(&rusqlite::Connection, usize) -> Result<Json>,
    observation: &str,
) -> Result<()> {
    let process_start = crate::composition::cpu_ms();
    assert!(matches!(layout, "eager" | "scheduled"));
    let data: Json = serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
    let temporary = tempfile::NamedTempFile::new().unwrap();
    std::fs::copy(
        capture
            .parent()
            .unwrap()
            .join(data["database"].as_str().unwrap()),
        temporary.path(),
    )
    .unwrap();
    let store = Store::open(temporary.path())?;
    store.db.pragma_update(None, "cache_size", -budget.0)?;
    store
        .db
        .pragma_update(Some("temp"), "cache_size", -budget.1)?;
    store.db.execute_batch("BEGIN")?;
    if data["fileScope"] == true {
        crate::session::request(
            &store.db,
            &data["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect::<Vec<_>>(),
        )?;
    }
    let parameters: Vec<_> = data["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| match v["kind"].as_str().unwrap() {
            "integer" => Value::Integer(v["value"].as_i64().unwrap()),
            "real" => Value::Real(v["value"].as_f64().unwrap()),
            "text" => Value::Text(v["value"].as_str().unwrap().to_owned()),
            "null" => Value::Null,
            _ => panic!("unexpected capture parameter"),
        })
        .collect();
    let stages = data["stages"].as_array().unwrap();
    let queries: Vec<_> = data
        .get("queries")
        .and_then(Json::as_array)
        .map(|q| q.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
        .unwrap_or_else(|| vec![data["sql"].as_str().unwrap().to_owned()]);
    let inputs: Vec<_> = stages
        .iter()
        .map(|s| Input {
            name: s["name"].as_str().unwrap(),
            sql: s["sql"].as_str().unwrap(),
            temporary: s["temporary"] == true,
        })
        .collect();
    let started = thread_cpu_ns().unwrap();
    let schedule = Schedule::new(&inputs, &queries);
    let schedule_cpu_ms = (thread_cpu_ns().unwrap() - started) as f64 / 1e6;
    let eager: Vec<_> = (0..stages.len())
        .map(Action::Prepare)
        .chain((0..queries.len()).map(Action::Query))
        .chain(
            (0..stages.len())
                .rev()
                .filter(|&i| inputs[i].temporary)
                .map(Action::Release),
        )
        .collect();
    let actions = if layout == "eager" {
        &eager
    } else {
        &schedule.actions
    };
    let mut snapshots = vec![json!({"label":"start","operation":0,"sample":observe(&store.db,0)?})];
    let (mut live, mut peak, mut operation, mut next_sample) = (0usize, 0usize, 0usize, 128usize);
    let (mut prepare_ns, mut results_ns, mut release_ns) = (0u128, 0u128, 0u128);
    let mut rows = Vec::<(i64, i64, i64)>::new();
    let mut statements = Vec::new();
    let mut heap_peaks = Vec::new();
    let mut compile_ns = 0u128;
    let mut vm = [0u64; 3];
    for group in
        actions.chunk_by(|a, b| matches!(a, Action::Release(_)) && matches!(b, Action::Release(_)))
    {
        operation += group.len();
        let (before_heap, _) = heap(true)?;
        let started = thread_cpu_ns().unwrap();
        let mut boundary = false;
        store.readers.clear();
        match group[0] {
            Action::Prepare(i) => {
                let mut q = store.db.prepare(inputs[i].sql)?;
                compile_ns += thread_cpu_ns().unwrap() - started;
                let (prepared, prepare_peak) = heap(true)?;
                statements.push(json!({"name":inputs[i].name,"sqlBytes":inputs[i].sql.len(),"statementBytes":q.get_status(rusqlite::StatementStatus::MemUsed),"heapAfterPrepare":prepared,"preparePeakBytes":prepare_peak}));
                let n = q.parameter_count();
                q.execute(params_from_iter(&parameters[..n]))?;
                count_steps(&q, &mut vm);
                drop(q);
                store.readers.clear();
                if inputs[i].temporary {
                    live += 1;
                    peak = peak.max(live);
                }
                if let Some(columns) = stages[i]["index"].as_str() {
                    store.db.execute_batch(&format!(
                        "CREATE INDEX temp.\"{}_keys\" ON \"{}\"({columns})",
                        inputs[i].name, inputs[i].name
                    ))?;
                }
                prepare_ns += thread_cpu_ns().unwrap() - started;
            }
            Action::Query(i) => {
                let mut q = store.db.prepare(&queries[i])?;
                compile_ns += thread_cpu_ns().unwrap() - started;
                let (prepared, prepare_peak) = heap(true)?;
                statements.push(json!({"name":format!("results-{i}"),"sqlBytes":queries[i].len(),"statementBytes":q.get_status(rusqlite::StatementStatus::MemUsed),"heapAfterPrepare":prepared,"preparePeakBytes":prepare_peak}));
                let n = q.parameter_count();
                rows.extend(
                    q.query_map(params_from_iter(&parameters[..n]), |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })?
                    .collect::<Result<Vec<_>>>()?,
                );
                count_steps(&q, &mut vm);
                results_ns += thread_cpu_ns().unwrap() - started;
                boundary = true;
            }
            Action::Release(_) => {
                let sql = group
                    .iter()
                    .map(|a| {
                        let Action::Release(i) = *a else {
                            unreachable!()
                        };
                        format!("DROP TABLE temp.\"{}\";", inputs[i].name)
                    })
                    .collect::<String>();
                store.db.execute_batch(&sql)?;
                live -= group.len();
                release_ns += thread_cpu_ns().unwrap() - started;
                boundary = live == 0;
            }
        }
        let (after_heap, peak_heap) = heap(false)?;
        heap_peaks.push(json!({"action":group[0],"beforeBytes":before_heap,"afterBytes":after_heap,"peakBytes":peak_heap}));
        if operation >= next_sample || boundary {
            snapshots.push(json!({"label":format!("{:?}",group[0]),"operation":operation,"sample":observe(&store.db,live)?}));
            next_sample = (operation / 128 + 1) * 128;
        }
    }
    assert_eq!(live, 0);
    store.readers.clear();
    store.db.execute_batch("COMMIT")?;
    snapshots
        .push(json!({"label":"finished","operation":operation,"sample":observe(&store.db,0)?}));
    std::fs::write(
        output.with_extension("rows.json"),
        serde_json::to_vec(&rows).unwrap(),
    )
    .unwrap();
    let mut compile_peaks = statements.clone();
    compile_peaks.sort_by_key(|s| std::cmp::Reverse(s["preparePeakBytes"].as_i64().unwrap()));
    compile_peaks.truncate(20);
    statements.sort_by_key(|s| std::cmp::Reverse(s["statementBytes"].as_i64().unwrap()));
    heap_peaks.sort_by_key(|s| std::cmp::Reverse(s["peakBytes"].as_i64().unwrap()));
    statements.truncate(20);
    heap_peaks.truncate(20);
    let result = json!({"capture":capture,"layout":layout,"observation":observation,"replayProcessCpuMs":crate::composition::cpu_ms()-process_start,"processPeakRssBytes":crate::composition::peak_bytes(),"cacheBudgetKiB":{"main":budget.0,"temp":budget.1},"statementCounters":{"vmSteps":vm[0],"fullscanSteps":vm[1],"sorts":vm[2],"scope":"stage and result statements; index and release batches excluded"},"rows":rows.len(),"stages":stages.len(),"queries":queries.len(),"peakLiveRelations":peak,"scheduledPeakLiveRelations":schedule.peak_relations,"scheduleCpuMs":schedule_cpu_ms,"sqliteCompileCpuMs":compile_ns as f64/1e6,"largestStatements":statements,"largestCompilePeaks":compile_peaks,"largestHeapPeaks":heap_peaks,"heapPeakScope":"stepping, index construction, finalization; compilation peaks are separate","prepareCpuMs":prepare_ns as f64/1e6,"resultsCpuMs":results_ns as f64/1e6,"releaseCpuMs":release_ns as f64/1e6,"sqlCpuMs":(prepare_ns+results_ns+release_ns) as f64/1e6,"snapshots":snapshots});
    std::fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    println!(
        "{}",
        json!({"layout":layout,"rows":rows.len(),"peakLiveRelations":peak,"sqlCpuMs":result["sqlCpuMs"]})
    );
    Ok(())
}
