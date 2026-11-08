use super::*;
#[test]
fn bulk_computed_values_keep_virtual_table_integer_checks() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    for name in ["typed_row", "typed_bulk"] {
        store.create(&db, name, Schema::integers(2, 1)).unwrap();
    }
    let select = "SELECT 3,2 UNION ALL SELECT 4,5";
    db.execute(&format!("INSERT INTO typed_row {select}"), [])
        .unwrap();
    store.load_select(&db, "typed_bulk", select, false).unwrap();
    assert!(
        store
            .version("typed_row")
            .same(&store.version("typed_bulk"))
    );
    let old = store.version("typed_bulk");
    for select in [
        "SELECT '3.0',2",
        "SELECT 3,2.0",
        "SELECT 1.5,2",
        "SELECT 3,NULL",
    ] {
        assert!(
            db.execute(&format!("INSERT INTO typed_row {select}"), [])
                .is_err()
        );
        assert!(
            store
                .select_available(&db, "typed_bulk", select, false, true)
                .is_err()
        );
        assert!(old.same(&store.version("typed_bulk")));
    }
}

#[test]
fn bulk_roots_preserve_cursors_row_ids_constraints_and_native_savepoints() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create(&db, "bulk_rows", Schema::integers(2, 1))
        .unwrap();
    store.restrict_indexes("bulk_rows", &[1]).unwrap();
    store
        .load_rows(
            "bulk_rows",
            (0..2000).map(|i| vec![Some(i), Some(i + 1)]).collect(),
            false,
        )
        .unwrap();
    let before = store.version("bulk_rows");
    store
        .select_available(&db, "bulk_rows", "SELECT * FROM bulk_rows", false, true)
        .unwrap();
    let identical = store.version("bulk_rows");
    assert!(Arc::ptr_eq(
        before.full.as_ref().unwrap(),
        identical.full.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        before.ids.as_ref().unwrap(),
        identical.ids.as_ref().unwrap()
    ));
    let source = super::super::super::facts::vtab::bound_source(&db, "bulk_rows").unwrap();
    let frozen = source.frozen().unwrap();
    assert!(
        Arc::ptr_eq(&frozen, &source.frozen().unwrap()),
        "lookups must share the immutable version"
    );
    let mut statement = db
        .prepare("SELECT rowid,c0,c1 FROM bulk_rows ORDER BY c0")
        .unwrap();
    let mut cursor = statement.query([]).unwrap();
    assert_eq!(cursor.next().unwrap().unwrap().get::<_, i64>(1).unwrap(), 0);
    store.begin().unwrap();
    store.savepoint();
    db.execute_batch("SAVEPOINT inner_roots").unwrap();
    store
        .select_available(
            &db,
            "bulk_rows",
            "SELECT c0,c1+5 FROM bulk_rows",
            false,
            true,
        )
        .unwrap();
    let after = store.version("bulk_rows");
    for at in 0..2000 {
        assert_eq!(before.row_id(at), after.row_id(at));
    }
    assert_eq!(cursor.next().unwrap().unwrap().get::<_, i64>(2).unwrap(), 2);
    assert_eq!(after.value(1, 1), 7);
    drop(cursor);
    drop(statement);
    store
        .create(&db, "inner_rows", Schema::integers(2, 2))
        .unwrap();
    store.rollback(&db).unwrap();
    db.execute_batch("ROLLBACK TO inner_roots; RELEASE inner_roots")
        .unwrap();
    assert!(before.same(&store.version("bulk_rows")));
    assert!(!store.contains("inner_rows"));
    store.savepoint();
    db.execute_batch("SAVEPOINT committed_inner").unwrap();
    store
        .select_available(
            &db,
            "bulk_rows",
            "SELECT c0,c1+9 FROM bulk_rows",
            false,
            true,
        )
        .unwrap();
    store.commit();
    db.execute_batch("RELEASE committed_inner").unwrap();
    store.rollback(&db).unwrap();
    assert!(before.same(&store.version("bulk_rows")));
    assert!(
        store
            .load_rows("bulk_rows", vec![vec![Some(3), Some(1)]], false)
            .is_err()
    );
    assert!(before.same(&store.version("bulk_rows")));
}

#[test]
fn bulk_load_matches_sql_ignore_and_builds_the_same_merkle_root() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    for name in ["by_row", "by_bulk"] {
        store.create(&db, name, Schema::integers(2, 1)).unwrap();
    }
    let input = vec![
        vec![Some(3), Some(9)],
        vec![None, Some(1)],
        vec![Some(7), Some(2)],
        vec![Some(3), Some(5)],
    ];
    for row in &input {
        db.execute(
            "INSERT OR IGNORE INTO by_row VALUES(?1,?2)",
            rusqlite::params_from_iter(row),
        )
        .unwrap();
    }
    assert_eq!(store.load_rows("by_bulk", input, true).unwrap(), 2);
    assert!(store.version("by_row").same(&store.version("by_bulk")));
    for c in 0..2 {
        assert_eq!(
            store.version("by_row").lookup(c, &[2, 3, 7, 9]),
            store.version("by_bulk").lookup(c, &[2, 3, 7, 9])
        );
    }
}
