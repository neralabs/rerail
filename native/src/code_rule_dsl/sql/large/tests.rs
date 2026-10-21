use super::*;

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
            Schema::of("data(id INTEGER PRIMARY KEY,n)".into()).unwrap(),
        ),
        ("keys".into(), Schema::of("keys(value)".into()).unwrap()),
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
