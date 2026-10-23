use super::*;

#[test]
fn incremental_explanations_survive_input_release_and_only_visit_new_queries() {
    let directory = tempfile::tempdir().unwrap();
    let connection = Connection::open_in_memory().unwrap();
    // SAFETY: this profiler drops before its connection.
    let profiler = unsafe { Profiler::new(&connection, directory.path()) }.unwrap();
    connection.execute_batch("CREATE TEMP TABLE seed(x); INSERT INTO seed VALUES(1); CREATE TEMP TABLE result AS SELECT * FROM seed").unwrap();
    profiler.explain_pending(&connection);
    let first = profiler.state.borrow().explained;
    connection
        .execute_batch("DROP TABLE seed; SELECT * FROM result")
        .unwrap();
    profiler.explain_pending(&connection);
    assert!(profiler.state.borrow().explained > first);
    connection.execute_batch("DROP TABLE result").unwrap();
    profiler.checkpoint(&connection).unwrap();
    let state = profiler.state.borrow();
    assert_eq!(state.explained, state.queries.len());
    for query in &state.queries {
        if query["sql"]
            .as_str()
            .unwrap()
            .contains("SELECT * FROM seed")
            || query["sql"] == "SELECT * FROM result"
        {
            assert!(query["plan"].is_array(), "{query}");
        }
    }
}

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
    let operator = profiler.operation("result query", ("clause", 7), 2, vec![("data".into(), 3)]);
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

#[test]
fn explains_ctas_before_release_without_spending_work_or_executing_body() {
    let directory = std::env::temp_dir().join(format!(
        "rerail-profile-ctas-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let connection = Connection::open_in_memory().unwrap();
    // SAFETY: the profiler drops before its connection.
    let profiler = unsafe { Profiler::new(&connection, &directory) }.unwrap();
    profiler.context(json!({"scope":"relational", "clauseId":"c", "path":"a.py"}));
    profiler.stage("derived");
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let observed = std::sync::Arc::clone(&calls);
    connection
        .create_scalar_function(
            "tick",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |context| {
                observed.fetch_add(1, Ordering::Relaxed);
                context.get::<i64>(0)
            },
        )
        .unwrap();
    connection.execute_batch("CREATE TABLE input(n); INSERT INTO input VALUES(1),(2); CREATE TEMP TABLE derived AS SELECT tick(n) AS n FROM input WHERE n > 0").unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    let work = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&work);
    connection
        .progress_handler(
            1,
            Some(move || {
                if !explaining() {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
                false
            }),
        )
        .unwrap();
    profiler.checkpoint(&connection).unwrap();
    assert_eq!(work.load(Ordering::Relaxed), 0);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(!explaining());
    let data: Value =
        serde_json::from_slice(&std::fs::read(profiler.stem.with_extension("json")).unwrap())
            .unwrap();
    let query = data["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| {
            q["sql"]
                .as_str()
                .unwrap()
                .trim_start()
                .starts_with("CREATE TEMP TABLE derived")
        })
        .unwrap();
    assert_eq!(
        query["planSql"],
        "SELECT tick(n) AS n FROM input WHERE n > 0"
    );
    assert!(query["plan"].is_array() && query["bytecode"].is_array());
    connection.execute_batch("DROP TABLE input").unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM derived", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(work.load(Ordering::Relaxed) > 0);
    drop(profiler);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn successful_unobserved_rows_do_not_claim_an_empty_or_incomplete_operation() {
    let directory = std::env::temp_dir().join(format!(
        "rerail-profile-rows-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let connection = Connection::open_in_memory().unwrap();
    // SAFETY: the profiler drops before its connection.
    let profiler = unsafe { Profiler::new(&connection, &directory) }.unwrap();
    profiler.context(json!({"scope":"relational", "clauseId":"c", "path":"a.py"}));
    let operation = profiler.operation("materialize", ("relational", 0), 0, Vec::new());
    connection
        .execute_batch("CREATE TABLE result AS SELECT 1")
        .unwrap();
    operation.finish_unobserved();
    profiler.checkpoint(&connection).unwrap();
    let events: Vec<Value> = std::fs::read_to_string(profiler.stem.with_extension("jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let event = events.iter().find(|e| e["kind"] == "operator").unwrap();
    assert_eq!(event["completed"], true);
    assert!(event["rows"].is_null());
    assert_eq!(event["context"]["clauseId"], "c");
    assert_eq!(event["context"]["path"], "a.py");
    drop(profiler);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn callback_survives_a_session_move_between_workers() {
    let directory = std::env::temp_dir().join(format!(
        "rerail-profile-move-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let connection = Connection::open_in_memory().unwrap();
    // SAFETY: the profiler and connection move together; the callback drops first.
    let profiler = unsafe { Profiler::new(&connection, &directory) }.unwrap();
    // Tuple fields drop in order, including if the worker panics.
    let owned = (Some(profiler), connection);
    fn worker(mut owned: (Option<Box<Profiler>>, Connection)) -> PathBuf {
        owned
            .1
            .execute_batch("CREATE TABLE data(n); INSERT INTO data VALUES(1)")
            .unwrap();
        owned.0.as_ref().unwrap().checkpoint(&owned.1).unwrap();
        let stem = owned.0.as_ref().unwrap().stem.clone();
        drop(owned.0.take());
        owned.1.execute_batch("INSERT INTO data VALUES(2)").unwrap();
        stem.parent().unwrap().to_path_buf()
    }
    let directory = std::thread::spawn(move || worker(owned)).join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
