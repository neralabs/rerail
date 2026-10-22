use super::*;

#[test]
fn column_deltas_preserve_update_snapshots_nulls_types_and_indexes() {
    let mut schemas = BTreeMap::from([(
        "data".into(),
        Schema::of("data(k INTEGER PRIMARY KEY,a,b,INDEX (a))".into()).unwrap(),
    )]);
    let statements = [
        "UPDATE data SET a = b,b = a WHERE k != 4",
        "UPDATE data SET a = (SELECT sum(b) FROM data),b = 4294967296 WHERE a IS NULL",
        "UPDATE data SET a = NULL WHERE k = 4",
        "UPDATE data SET a = 'changed',b = NULL WHERE b > 100",
        "UPDATE data SET a = 12,b = a WHERE a IS NULL",
        "UPDATE data SET b = 77 WHERE a = 'changed'",
        "UPDATE data SET b = 88 WHERE k = 123456",
    ];
    let sql = render_steps(&mut schemas, &statements);
    assert!(sql.contains("p_patch_data"));
    assert!(!sql.contains("p_replace_data"));
    let db = Connection::open_in_memory().unwrap();
    let oracle = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    oracle.execute_batch("CREATE TABLE data(k INTEGER PRIMARY KEY,a,b); CREATE INDEX by_a ON data(a); INSERT INTO data VALUES(7,NULL,11),(-3,22,NULL),(42,33,44),(4,NULL,55)").unwrap();
    frames
        .program_rows(
            "data",
            vec![
                vec![Value::Integer(7), Value::Null, Value::Integer(11)],
                vec![Value::Integer(-3), Value::Integer(22), Value::Null],
                vec![Value::Integer(42), Value::Integer(33), Value::Integer(44)],
                vec![Value::Integer(4), Value::Null, Value::Integer(55)],
            ],
            false,
            false,
        )
        .unwrap();
    // Build a lookup before mutation; subsequent reads must not reuse stale values.
    db.query_row("SELECT count(*) FROM data WHERE a=22", [], |r| {
        r.get::<_, i64>(0)
    })
    .unwrap();
    for _ in 0..2 {
        let (_, counts) = super::super::super::profiling::frame_counts(|| {
            db.prepare(&sql).unwrap().query([]).unwrap().next().unwrap();
        });
        assert_eq!(counts[4], 0, "updates must not republish complete rows");
        assert!(
            counts[9] > 0,
            "updated cells must remain visible to profiling"
        );
        for statement in statements {
            oracle.execute(statement, []).unwrap();
        }
        let read = |db: &Connection, sql: &str| {
            db.prepare(sql)
                .unwrap()
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Value>(1)?,
                        r.get::<_, Value>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(
            read(&db, "SELECT k,a,b FROM data ORDER BY k"),
            read(&oracle, "SELECT k,a,b FROM data ORDER BY k")
        );
        assert_eq!(
            read(&db, "SELECT k,a,b FROM data WHERE a='changed' ORDER BY k"),
            read(
                &oracle,
                "SELECT k,a,b FROM data WHERE a='changed' ORDER BY k"
            )
        );
    }
}

#[test]
fn publication_chunks_keep_rowids_nulls_and_ignore_conflicts_across_boundaries() {
    let mut schemas = BTreeMap::from([
        (
            "input".into(),
            Schema::of("input(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "output".into(),
            Schema::of("output(k INTEGER PRIMARY KEY,value)".into()).unwrap(),
        ),
    ]);
    let sql = render_steps(
        &mut schemas,
        &[
            "INSERT INTO output SELECT NULL,CASE WHEN k%2=0 THEN k ELSE NULL END FROM input",
            "INSERT OR IGNORE INTO output SELECT k,k FROM input",
        ],
    );
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    for count in [8193, 4097] {
        frames
            .program_rows(
                "input",
                (1..=count).map(|k| vec![Value::Integer(k)]).collect(),
                true,
                false,
            )
            .unwrap();
        frames.clear("output");
        db.prepare_cached(&sql)
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap();
        let actual = db
            .query_row(
                "SELECT count(*),min(k),max(k),sum(value),sum(value IS NULL) FROM output",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .unwrap();
        let even = count / 2;
        assert_eq!(actual, (count, 1, count, even * (even + 1), count - even));
    }
}

#[test]
fn sparse_dispatch_covers_high_pcs_without_aliasing_low_bits() {
    let values = [
        (0, "11"),
        (7, "12"),
        (8, "13"),
        (1023, "14"),
        (4096, "15"),
        (10019, "16"),
    ];
    let sql = format!(
        "WITH driver(pc) AS (VALUES(?1)) SELECT {} FROM driver",
        dispatch(&values, "-1", 13)
    );
    let db = Connection::open_in_memory().unwrap();
    let mut statement = db.prepare(&sql).unwrap();
    for pc in 0..16384 {
        let expected = values
            .iter()
            .find(|(id, _)| *id == pc)
            .map_or(-1, |(_, v)| v.parse::<i64>().unwrap());
        assert_eq!(
            statement
                .query_row([pc as i64], |r| r.get::<_, i64>(0))
                .unwrap(),
            expected,
            "pc {pc}"
        );
    }
}

#[test]
fn outer_join_keys_remain_nullable_on_both_sides_of_membership() {
    for query in [
        "SELECT a.k,b.k IN(SELECT k FROM chosen),b.k NOT IN(SELECT k FROM chosen) FROM data a LEFT JOIN data b ON b.k=a.k+100",
        "SELECT a.k,a.k IN(SELECT b.k FROM data a LEFT JOIN data b ON b.k=a.k+100),a.k NOT IN(SELECT b.k FROM data a LEFT JOIN data b ON b.k=a.k+100) FROM data a",
    ] {
        let mut schemas = BTreeMap::from([
            (
                "data".into(),
                Schema::of("data(k INTEGER PRIMARY KEY)".into()).unwrap(),
            ),
            (
                "chosen".into(),
                Schema::of("chosen(k INTEGER PRIMARY KEY)".into()).unwrap(),
            ),
            (
                "output".into(),
                Schema::of("output(k INTEGER PRIMARY KEY,yes,no)".into()).unwrap(),
            ),
        ]);
        let insert = format!("INSERT INTO output {query}");
        let sql = render_steps(&mut schemas, &[&insert]);
        let db = Connection::open_in_memory().unwrap();
        let frames = Arc::new(FrameStore::default());
        storage::register(&db, &frames, &schemas).unwrap();
        frames
            .program_rows(
                "data",
                vec![vec![Value::Integer(1)], vec![Value::Integer(2)]],
                true,
                false,
            )
            .unwrap();
        frames
            .program_rows("chosen", vec![vec![Value::Integer(1)]], true, false)
            .unwrap();
        let read = |sql: &str| {
            db.prepare(sql)
                .unwrap()
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Value>(1)?,
                        r.get::<_, Value>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let expected = read(query);
        assert!(
            expected
                .iter()
                .all(|(_, yes, no)| *yes == Value::Null && *no == Value::Null)
        );
        db.prepare(&sql).unwrap().query([]).unwrap().next().unwrap();
        assert_eq!(read("SELECT k,yes,no FROM output"), expected);
    }
}

#[test]
fn immutable_demands_keep_batch_constraints_and_key_sets_use_direct_probes() {
    assert!(
        materialize::membership(
            "SELECT n.id FROM node n WHERE n.file IN (SELECT file FROM runs WHERE run=3)"
        )
        .is_none()
    );
    // A mutable membership set must refresh at each SQL control iteration.
    assert!(
        materialize::membership(
            "SELECT n.id FROM node n WHERE n.file IN (SELECT file FROM changing)"
        )
        .is_some()
    );
    let mut schemas = BTreeMap::from([
        (
            "input".into(),
            Schema::of("input(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "chosen".into(),
            Schema::of("chosen(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "output".into(),
            Schema::of("output(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
    ]);
    let sql = render_steps(
        &mut schemas,
        &["INSERT INTO output SELECT f.k FROM input f WHERE f.k IN (SELECT k FROM chosen)"],
    );
    assert!(!schemas.keys().any(|s| s.starts_with("p_cte_")));
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    frames
        .program_rows(
            "input",
            (1..=1000).map(|i| vec![Value::Integer(i)]).collect(),
            true,
            false,
        )
        .unwrap();
    for value in [17, 41] {
        frames
            .program_rows("chosen", vec![vec![Value::Integer(value)]], true, false)
            .unwrap();
        frames.clear("output");
        let mut statement = db.prepare_cached(&sql).unwrap();
        statement.query([]).unwrap().next().unwrap();
        assert_eq!(
            db.query_row("SELECT k FROM output", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            value
        );
        assert!(statement.get_status(rusqlite::StatementStatus::VmStep) < 30_000);
    }
}

#[test]
fn pipelines_inline_only_one_direct_consumer() {
    assert!(materialize::pipeline("t", &["SELECT * FROM t"]));
    assert!(!materialize::pipeline("t", &["SELECT * FROM t JOIN t b"]));
    assert!(!materialize::pipeline(
        "t",
        &["SELECT * FROM a WHERE EXISTS(SELECT 1 FROM t)"]
    ));
    assert!(!materialize::pipeline(
        "t",
        &["SELECT * FROM a CROSS JOIN t"]
    ));
    let mut schemas = BTreeMap::from([
        (
            "input".into(),
            Schema::of("input(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "output".into(),
            Schema::of("output(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
    ]);
    let sql = render_steps(
        &mut schemas,
        &["WITH t(v) AS MATERIALIZED (SELECT k+1 FROM input) INSERT INTO output SELECT v FROM t"],
    );
    assert!(!schemas.keys().any(|s| s.starts_with("p_cte_")));
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    for value in [17, 41] {
        frames
            .program_rows("input", vec![vec![Value::Integer(value)]], true, false)
            .unwrap();
        frames.clear("output");
        db.prepare_cached(&sql)
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(
            db.query_row("SELECT k FROM output", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            value + 1
        );
    }
}

#[test]
fn nested_cte_membership_keeps_its_local_binding() {
    let mut schemas = BTreeMap::from([
        (
            "input".into(),
            Schema::of("input(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "output".into(),
            Schema::of("output(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
    ]);
    let sql = render_steps(
        &mut schemas,
        &[
            "WITH s(k) AS (WITH u(v) AS (SELECT k FROM input) SELECT k FROM input WHERE k IN (SELECT v FROM u)) INSERT INTO output SELECT k FROM s",
        ],
    );
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    for value in [17, 41] {
        frames
            .program_rows("input", vec![vec![Value::Integer(value)]], true, false)
            .unwrap();
        frames.clear("output");
        db.prepare_cached(&sql)
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(
            db.query_row("SELECT k FROM output", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            value
        );
    }
}

fn render_steps(schemas: &mut BTreeMap<String, Schema>, statements: &[&str]) -> String {
    let rule = crate::code_rule_dsl::test_support::compiled(
        serde_json::json!({"rule":{"kind":"identifier"}}),
    );
    let grammars = crate::code_rule_dsl::test_support::grammar_matchers(&rule);
    let compiled = super::super::compile(&grammars.iter().collect::<Vec<_>>()).unwrap();
    let plan = compiled.plan().unwrap();
    schemas.insert(
        "p_results".into(),
        Schema::of("p_results(clause,file,node,payload,round)".into()).unwrap(),
    );
    let mut builder = Builder {
        plan: &plan,
        schemas,
        code: Vec::new(),
        stages: BTreeSet::new(),
        serial: 0,
        blocks: HashMap::new(),
        pending: Vec::new(),
        calls: Vec::new(),
        phase: 0,
        free: HashMap::new(),
        scopes: Vec::new(),
    };
    for sql in statements {
        builder.statement(sql).unwrap();
    }
    builder.render().unwrap()
}

#[test]
fn hoisted_membership_preserves_nulls_negation_and_empty_sets() {
    let mut schemas = BTreeMap::from([
        (
            "data".into(),
            Schema::of("data(id,n,PRIMARY KEY(n,id))".into()).unwrap(),
        ),
        (
            "keys".into(),
            Schema::of("keys(value,PRIMARY KEY(value))".into()).unwrap(),
        ),
        (
            "output".into(),
            Schema::of("output(id INTEGER PRIMARY KEY,yes,no)".into()).unwrap(),
        ),
    ]);
    let sql = render_steps(
        &mut schemas,
        &[
            "DELETE FROM output",
            "INSERT INTO output SELECT id,n IN (SELECT value FROM keys),n NOT IN (SELECT value FROM keys) FROM data",
        ],
    );
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    frames
        .program_rows(
            "data",
            vec![
                vec![Value::Integer(1), Value::Null],
                vec![Value::Integer(2), Value::Integer(2)],
                vec![Value::Integer(3), Value::Integer(3)],
            ],
            true,
            false,
        )
        .unwrap();
    for values in [vec![vec![Value::Null], vec![Value::Integer(2)]], vec![]] {
        frames.program_rows("keys", values, true, false).unwrap();
        let expected = db
            .prepare(
                "SELECT id,n IN(SELECT value FROM keys),n NOT IN(SELECT value FROM keys) FROM data",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Value>(1)?,
                    r.get::<_, Value>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        db.prepare(&sql).unwrap().query([]).unwrap().next().unwrap();
        let actual = db
            .prepare("SELECT id,yes,no FROM output")
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Value>(1)?,
                    r.get::<_, Value>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn joined_update_demands_materialize_once_and_temporary_schemas_are_reused() {
    let mut schemas = BTreeMap::from([
        (
            "data".into(),
            Schema::of("data(id INTEGER PRIMARY KEY,n INTEGER)".into()).unwrap(),
        ),
        (
            "changed".into(),
            Schema::of("changed(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "edges".into(),
            Schema::of("edges(k INTEGER PRIMARY KEY,s INTEGER,INDEX (s))".into()).unwrap(),
        ),
    ]);
    let update = "UPDATE data SET n = 1 WHERE id IN (SELECT e.s FROM changed c CROSS JOIN edges e ON e.k=c.k)";
    let sql = render_steps(&mut schemas, &[update, update, update]);
    assert_eq!(
        schemas.keys().filter(|s| s.starts_with("p_cte_")).count(),
        1
    );
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    storage::register(&db, &frames, &schemas).unwrap();
    frames
        .program_rows(
            "data",
            (1..=1000)
                .map(|i| vec![Value::Integer(i), Value::Integer(0)])
                .collect(),
            true,
            false,
        )
        .unwrap();
    frames
        .program_rows(
            "changed",
            (1..=500).map(|i| vec![Value::Integer(i)]).collect(),
            true,
            false,
        )
        .unwrap();
    frames
        .program_rows(
            "edges",
            (1..=1000)
                .map(|i| vec![Value::Integer(i), Value::Integer(i)])
                .collect(),
            true,
            false,
        )
        .unwrap();
    let mut query = db.prepare(&sql).unwrap();
    query.query([]).unwrap().next().unwrap();
    assert_eq!(
        db.query_row("SELECT sum(n) FROM data", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        500
    );
    assert!(query.get_status(rusqlite::StatementStatus::VmStep) < 300_000);
}

#[test]
fn cte_bindings_do_not_rename_columns_with_the_same_name() {
    let names = HashMap::from_iter([("k".to_owned(), "p_cte_6".to_owned())]);
    assert_eq!(
        materialize::bindings("SELECT k.k FROM k WHERE k.k=7", &names),
        "SELECT k.k FROM p_cte_6 AS k WHERE k.k=7"
    );
    assert_eq!(
        materialize::bindings("SELECT p.k FROM k p", &names),
        "SELECT p.k FROM p_cte_6 p"
    );
}

#[test]
fn materialized_inputs_refresh_at_every_recursive_back_edge() {
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    let schemas = BTreeMap::from([(
        "data".into(),
        Schema::of("data(k INTEGER PRIMARY KEY,n INTEGER)".into()).unwrap(),
    )]);
    storage::register(&db, &frames, &schemas).unwrap();
    frames
        .program_rows(
            "data",
            vec![vec![Value::Integer(1), Value::Integer(0)]],
            true,
            false,
        )
        .unwrap();
    let expression=dml::statement(&schemas,"WITH old AS MATERIALIZED (SELECT * FROM data) INSERT INTO data SELECT k+1,n+1 FROM old WHERE k=(SELECT MAX(k) FROM old)").unwrap();
    let query = format!(
        "WITH RECURSIVE driver(tick,token) AS (VALUES(0,0) UNION ALL SELECT tick+1,{expression} FROM driver WHERE tick<4), ready AS MATERIALIZED(SELECT token FROM driver WHERE tick=4) SELECT MAX(n) FROM ready CROSS JOIN data"
    );
    let n: i64 = db.query_row(&query, [], |r| r.get(0)).unwrap();
    assert_eq!(
        n, 4,
        "a cached independent materialization would repeatedly append n=1"
    );
}

#[test]
fn inserts_expand_defaults_and_updates_and_deletes_keep_their_sql_predicates() {
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    let schemas = BTreeMap::from([(
        "data".into(),
        Schema::of("data(id INTEGER PRIMARY KEY,n,dirty INTEGER DEFAULT 1)".into()).unwrap(),
    )]);
    storage::register(&db, &frames, &schemas).unwrap();
    for sql in [
        "INSERT INTO data(n) VALUES (7),(8)",
        "UPDATE data SET dirty = 0 WHERE n=7",
        "DELETE FROM data WHERE n=8",
    ] {
        let expression = dml::statement(&schemas, sql).unwrap();
        let query = format!("WITH driver(tick) AS (VALUES(0)) SELECT {expression} FROM driver");
        db.query_row(&query, [], |r| r.get::<_, i64>(0)).unwrap();
    }
    let rows = db
        .prepare("SELECT id,n,dirty FROM data")
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows, [(1, 7, 0)]);
}

#[test]
fn primary_key_updates_probe_the_key_set_instead_of_rebuilding_it_per_row() {
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    let schemas = BTreeMap::from([
        (
            "data".into(),
            Schema::of("data(id INTEGER PRIMARY KEY,n INTEGER)".into()).unwrap(),
        ),
        (
            "keys".into(),
            Schema::of("keys(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
    ]);
    storage::register(&db, &frames, &schemas).unwrap();
    frames
        .program_rows(
            "data",
            (1..=1000)
                .map(|i| vec![Value::Integer(i), Value::Integer(0)])
                .collect(),
            true,
            false,
        )
        .unwrap();
    frames
        .program_rows(
            "keys",
            (1..=500).map(|i| vec![Value::Integer(i)]).collect(),
            true,
            false,
        )
        .unwrap();
    let expression = dml::statement(
        &schemas,
        "UPDATE data SET n = 1 WHERE id IN (SELECT k FROM keys)",
    )
    .unwrap();
    let mut query = db
        .prepare(&format!(
            "WITH driver(tick) AS(VALUES(0)) SELECT {expression} FROM driver"
        ))
        .unwrap();
    query.query_row([], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(
        db.query_row("SELECT sum(n) FROM data", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        500
    );
    assert!(query.get_status(rusqlite::StatementStatus::VmStep) < 100_000);
}

#[test]
fn membership_inside_insert_producers_also_uses_key_probes() {
    let db = Connection::open_in_memory().unwrap();
    let frames = Arc::new(FrameStore::default());
    let schemas = BTreeMap::from([
        (
            "data".into(),
            Schema::of("data(id INTEGER PRIMARY KEY,n INTEGER)".into()).unwrap(),
        ),
        (
            "keys".into(),
            Schema::of("keys(k INTEGER PRIMARY KEY)".into()).unwrap(),
        ),
        (
            "output".into(),
            Schema::of("output(id INTEGER PRIMARY KEY,n INTEGER)".into()).unwrap(),
        ),
    ]);
    storage::register(&db, &frames, &schemas).unwrap();
    frames
        .program_rows(
            "data",
            (1..=1000)
                .map(|i| vec![Value::Integer(i), Value::Integer(1)])
                .collect(),
            true,
            false,
        )
        .unwrap();
    frames
        .program_rows(
            "keys",
            (1..=500).map(|i| vec![Value::Integer(i)]).collect(),
            true,
            false,
        )
        .unwrap();
    let expression = dml::statement(
        &schemas,
        "INSERT INTO output SELECT id,n FROM data WHERE id IN (SELECT k FROM keys)",
    )
    .unwrap();
    let mut query = db
        .prepare(&format!(
            "WITH driver(tick) AS(VALUES(0)) SELECT {expression} FROM driver"
        ))
        .unwrap();
    query.query_row([], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(frames.rows("output"), Some(500));
    assert!(
        query.get_status(rusqlite::StatementStatus::VmStep) < 100_000,
        "{} VM instructions",
        query.get_status(rusqlite::StatementStatus::VmStep)
    );
    let expression = dml::statement(
        &schemas,
        "INSERT INTO output SELECT id,n FROM data WHERE id NOT IN (SELECT k FROM keys)",
    )
    .unwrap();
    let mut query = db
        .prepare(&format!(
            "WITH driver(tick) AS(VALUES(0)) SELECT {expression} FROM driver"
        ))
        .unwrap();
    query.query_row([], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(frames.rows("output"), Some(1000));
    assert!(query.get_status(rusqlite::StatementStatus::VmStep) < 100_000);
}
