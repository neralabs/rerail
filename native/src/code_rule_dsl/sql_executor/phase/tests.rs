use super::*;

fn initialize(db: &rusqlite::Connection, store: &versions::Store, name: &str) {
    store
        .create(db, name, versions::Schema::integers(2, 1))
        .unwrap();
    store
        .load_rows(
            name,
            vec![vec![Some(1), Some(5)], vec![Some(2), Some(8)]],
            false,
        )
        .unwrap();
}
#[test]
fn dependent_phase_outputs_evaluate_once_and_publish_atomically() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = versions::Store::default();
    initialize(&db, &store, "phase_a");
    initialize(&db, &store, "phase_b");
    store
        .create(&db, "phase_keys", versions::Schema::integers(1, 1))
        .unwrap();
    store
        .load_rows("phase_keys", vec![vec![Some(1)]], false)
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    db.create_scalar_function(
        c"phase_probe",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            calls.fetch_add(1, Ordering::Relaxed);
            ctx.get::<i64>(0)
        },
    )
    .unwrap();
    let pool = scratch::Pool::default();
    let mut w = Work::new(&db, &pool);
    w.versions = Some(&store);
    let before = store.version("phase_a");
    let mut phase = Phase::default();
    let next = phase.relation(2, "SELECT c0,phase_probe(c1+1) FROM phase_a WHERE c0=1");
    let other = phase.relation(2, &format!("SELECT c0,c1+10 FROM {next}"));
    phase.patch(&store, "phase_a", "phase_keys", &next).unwrap();
    phase
        .patch(&store, "phase_b", "phase_keys", &other)
        .unwrap();
    phase.execute(&w).unwrap();
    assert_eq!(
        count.load(Ordering::Relaxed),
        1,
        "a materialized SQL expression must run once"
    );
    assert_eq!(
        store.version("phase_a").rows(),
        vec![vec![Some(1), Some(6)], vec![Some(2), Some(8)]]
    );
    assert_eq!(
        store.version("phase_b").rows(),
        vec![vec![Some(1), Some(16)], vec![Some(2), Some(8)]]
    );
    assert_eq!(before.row_id(0), store.version("phase_a").row_id(0));
    let a = store.version("phase_a");
    let b = store.version("phase_b");
    let mut invalid = Phase::default();
    let good = invalid.relation(2, "SELECT 1,99");
    let bad = invalid.relation(2, "SELECT 1,NULL");
    invalid
        .patch(&store, "phase_a", "phase_keys", &good)
        .unwrap();
    invalid
        .patch(&store, "phase_b", "phase_keys", &bad)
        .unwrap();
    assert!(invalid.execute(&w).is_err());
    assert!(a.same(&store.version("phase_a")) && b.same(&store.version("phase_b")));
    let mut failed = Phase::default();
    let good = failed.relation(2, "SELECT 1,99");
    let bad = failed.relation(2, "SELECT 1,abs(-9223372036854775808)");
    failed
        .patch(&store, "phase_a", "phase_keys", &good)
        .unwrap();
    failed.patch(&store, "phase_b", "phase_keys", &bad).unwrap();
    assert!(failed.execute(&w).is_err());
    assert!(a.same(&store.version("phase_a")) && b.same(&store.version("phase_b")));
}

#[test]
fn private_readonly_roots_need_no_sql_schema_and_restore_after_rollback() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = versions::Store::default();
    store
        .create_readonly(&db, "readonly_rows", versions::Schema::integers(2, 1))
        .unwrap();
    store
        .load_rows("readonly_rows", vec![vec![Some(1), Some(7)]], false)
        .unwrap();
    let count = || {
        db.query_row(
            "SELECT count(*) FROM sqlite_temp_schema WHERE name='readonly_rows'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(count(), 0);
    assert_eq!(
        db.query_row("SELECT c1 FROM readonly_rows WHERE c0=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert!(
        db.execute("INSERT INTO readonly_rows VALUES(2,8)", [])
            .is_err()
    );
    let before = store.version("readonly_rows");
    store.begin().unwrap();
    db.execute_batch("SAVEPOINT readonly_undo").unwrap();
    store.remove(&db, "readonly_rows").unwrap();
    store
        .create_readonly(&db, "readonly_rows", versions::Schema::integers(2, 1))
        .unwrap();
    store
        .load_rows("readonly_rows", vec![vec![Some(2), Some(9)]], false)
        .unwrap();
    db.execute_batch("ROLLBACK TO readonly_undo; RELEASE readonly_undo")
        .unwrap();
    store.rollback(&db).unwrap();
    assert!(before.same(&store.version("readonly_rows")));
    assert_eq!(count(), 0);
    assert!(db.execute("DELETE FROM readonly_rows", []).is_err());
    let fork = store.fork_scope(0).unwrap();
    {
        let _scope = store.activate(fork).unwrap();
        assert_eq!(store.version("readonly_rows").rows(), before.rows());
        assert!(
            db.execute("INSERT INTO readonly_rows VALUES(3,9)", [])
                .is_err()
        );
    }
    store.close(&db).unwrap();
    assert!(db.prepare("SELECT * FROM readonly_rows").is_err());
}

#[test]
fn computed_keys_publish_updates_and_deletions_without_helper_tables() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = versions::Store::default();
    initialize(&db, &store, "computed_a");
    initialize(&db, &store, "computed_b");
    let pool = scratch::Pool::default();
    let mut w = Work::new(&db, &pool);
    w.versions = Some(&store);
    let mut phase = Phase::default();
    let keys = phase.relation(1, "SELECT c0 FROM computed_a WHERE c1>5");
    let changed = phase.relation(
        2,
        &format!("SELECT a.c0,a.c1+7 FROM {keys} k JOIN computed_a a ON a.c0=k.c0"),
    );
    let empty = phase.relation(2, "SELECT 1,1 WHERE 0");
    phase
        .patch_computed(&store, "computed_a", &keys, &changed)
        .unwrap();
    phase
        .patch_computed(&store, "computed_b", &keys, &empty)
        .unwrap();
    phase.execute(&w).unwrap();
    assert_eq!(
        store.version("computed_a").rows(),
        vec![vec![Some(1), Some(5)], vec![Some(2), Some(15)]]
    );
    assert_eq!(
        store.version("computed_b").rows(),
        vec![vec![Some(1), Some(5)]]
    );
    let before = store.version("computed_a");
    let mut invalid = Phase::default();
    let keys = invalid.relation(1, "SELECT NULL");
    let values = invalid.relation(2, "SELECT 1,9");
    invalid
        .patch_computed(&store, "computed_a", &keys, &values)
        .unwrap();
    assert!(invalid.execute(&w).is_err());
    assert!(before.same(&store.version("computed_a")));
}
