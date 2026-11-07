use super::*;
pub(super) fn data(db: &rusqlite::Connection, name: &str) -> Vec<Vec<Option<i64>>> {
    let mut stmt = db
        .prepare(&format!("SELECT * FROM {name} ORDER BY 1,2"))
        .unwrap();
    let width = stmt.column_count();
    stmt.query_map([], |r| (0..width).map(|c| r.get(c)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}
#[test]
fn sql_versions_preserve_keys_nulls_conflicts_and_transactional_forks() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    let mut tx = store.transaction(&db).unwrap();
    let mut schema = Schema::integers(3, 2);
    schema.nullable[2] = true;
    tx.create("version_rows", schema).unwrap();
    db.execute("INSERT INTO version_rows VALUES(1,2,NULL),(3,4,9)", [])
        .unwrap();
    assert_eq!(
        db.execute("INSERT OR IGNORE INTO version_rows VALUES(1,2,5)", [])
            .unwrap(),
        0
    );
    assert!(
        db.execute("INSERT INTO version_rows VALUES(1,2,5)", [])
            .is_err()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM version_rows WHERE c2=0", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let before = store.version("version_rows");
    tx.fork("version_rows", "version_fork").unwrap();
    db.execute("UPDATE version_fork SET c2=7 WHERE c0=1 AND c1=2", [])
        .unwrap();
    db.execute("INSERT OR REPLACE INTO version_fork VALUES(3,4,8)", [])
        .unwrap();
    assert_eq!(
        data(&db, "version_rows"),
        vec![
            vec![Some(1), Some(2), None],
            vec![Some(3), Some(4), Some(9)]
        ]
    );
    assert_eq!(
        data(&db, "version_fork"),
        vec![
            vec![Some(1), Some(2), Some(7)],
            vec![Some(3), Some(4), Some(8)]
        ]
    );
    assert_eq!(before.len(), 2);
    tx.commit();
    {
        let mut tx = store.transaction(&db).unwrap();
        tx.fork("version_fork", "failed_fork").unwrap();
        db.execute("DELETE FROM version_rows", []).unwrap();
        db.execute("UPDATE version_fork SET c2=100", []).unwrap();
        assert!(
            db.execute("INSERT INTO version_fork VALUES(NULL,1,0)", [])
                .is_err()
        );
    }
    assert!(before.same(&store.version("version_rows")));
    assert_eq!(
        data(&db, "version_fork"),
        vec![
            vec![Some(1), Some(2), Some(7)],
            vec![Some(3), Some(4), Some(8)]
        ]
    );
    assert!(db.prepare("SELECT * FROM failed_fork").is_err());
    store.remove(&db, "version_rows").unwrap();
    store.remove(&db, "version_fork").unwrap();
}

#[test]
fn canonical_versions_find_sparse_changes_without_traversing_unchanged_rows() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    let mut tx = store.transaction(&db).unwrap();
    tx.create("version_large", Schema::integers(2, 1)).unwrap();
    {
        let mut insert = db
            .prepare("INSERT INTO version_large VALUES(?1,?2)")
            .unwrap();
        for n in 0..20000i64 {
            insert.execute([n, n * 3]).unwrap();
        }
    }
    let before = store.version("version_large");
    tx.fork("version_large", "version_new").unwrap();
    db.execute("DELETE FROM version_new WHERE c0=17", [])
        .unwrap();
    db.execute("UPDATE version_new SET c1=41 WHERE c0=10000", [])
        .unwrap();
    db.execute("INSERT INTO version_new VALUES(30000,9)", [])
        .unwrap();
    let (plus, minus, visited) = before.difference(&store.version("version_new"));
    assert_eq!(
        plus,
        vec![vec![Some(10000), Some(41)], vec![Some(30000), Some(9)]]
    );
    assert_eq!(
        minus,
        vec![vec![Some(17), Some(51)], vec![Some(10000), Some(30000)]]
    );
    assert!(visited < 300, "visited {visited} nodes");
    eprintln!("version diff: {visited} nodes over 20,000 rows");
    assert_eq!(before.len(), 20000);
    assert_eq!(store.version("version_new").len(), 20000);
    tx.commit();
    store.close(&db).unwrap();
}

#[test]
fn native_sql_cursors_pin_their_version_during_writes() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    let mut tx = store.transaction(&db).unwrap();
    tx.create("version_cursor", Schema::integers(2, 1)).unwrap();
    db.execute("INSERT INTO version_cursor VALUES(1,5),(2,6),(3,7)", [])
        .unwrap();
    let mut statement = db.prepare("SELECT c1 FROM version_cursor").unwrap();
    let mut cursor = statement.query([]).unwrap();
    assert_eq!(cursor.next().unwrap().unwrap().get::<_, i64>(0).unwrap(), 5);
    db.execute("UPDATE version_cursor SET c1=c1+100", [])
        .unwrap();
    assert_eq!(cursor.next().unwrap().unwrap().get::<_, i64>(0).unwrap(), 6);
    assert_eq!(cursor.next().unwrap().unwrap().get::<_, i64>(0).unwrap(), 7);
    drop(cursor);
    drop(statement);
    tx.commit();
    assert_eq!(
        data(&db, "version_cursor"),
        vec![
            vec![Some(1), Some(105)],
            vec![Some(2), Some(106)],
            vec![Some(3), Some(107)]
        ]
    );
    store.close(&db).unwrap();
}

#[test]
fn native_versions_use_lazy_ordered_group_probes_and_recheck_other_filters() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    let mut tx = store.transaction(&db).unwrap();
    tx.create("version_extrema", Schema::integers(2, 2))
        .unwrap();
    {
        let mut insert = db
            .prepare("INSERT INTO version_extrema VALUES(1,?1)")
            .unwrap();
        for n in 0..20000i64 {
            insert.execute([n]).unwrap();
        }
    }
    let binding = store.modules.get("version_extrema").unwrap();
    binding
        .counters()
        .columns
        .store(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        db.query_row(
            "SELECT c1 FROM version_extrema WHERE c0=1 ORDER BY c1 DESC LIMIT 1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        19999
    );
    assert_eq!(
        db.query_row(
            "SELECT c1 FROM version_extrema WHERE c0=1 ORDER BY c1 LIMIT 1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        binding
            .counters()
            .columns
            .load(std::sync::atomic::Ordering::Relaxed)
            < 20
    );
    assert_eq!(
        db.query_row(
            "SELECT c1 FROM version_extrema WHERE c0=1 AND abs(c1)>5 ORDER BY c1 LIMIT 1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        6
    );
    tx.commit();
    store.close(&db).unwrap();
}

#[test]
fn version_hashes_are_independent_of_insertion_order_and_checks_precede_mutation() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    let mut tx = store.transaction(&db).unwrap();
    let schema = Schema::generated(
        "c0 INTEGER NOT NULL,n INTEGER NOT NULL CHECK(n>0),value INTEGER,PRIMARY KEY(c0)",
    )
    .unwrap();
    tx.create("version_forward", schema.clone()).unwrap();
    tx.create("version_backward", schema).unwrap();
    for name in ["version_forward", "version_backward"] {
        let mut insert = db
            .prepare(&format!("INSERT INTO {name} VALUES(?1,1,NULL)"))
            .unwrap();
        let order = if name == "version_forward" {
            (0..1000).collect::<Vec<_>>()
        } else {
            (0..1000).rev().collect::<Vec<_>>()
        };
        for i in order {
            insert.execute([i]).unwrap();
        }
    }
    let before = store.version("version_forward");
    let (plus, minus, visited) = before.difference(&store.version("version_backward"));
    assert!(plus.is_empty() && minus.is_empty());
    assert_eq!(visited, 1);
    assert!(
        db.execute("UPDATE version_forward SET n=0 WHERE c0=17", [])
            .is_err()
    );
    assert!(before.same(&store.version("version_forward")));
    assert_eq!(
        db.execute(
            "INSERT OR IGNORE INTO version_forward VALUES(2000,0,NULL)",
            []
        )
        .unwrap(),
        0
    );
    assert!(before.same(&store.version("version_forward")));
    tx.commit();
    store.close(&db).unwrap();
}
