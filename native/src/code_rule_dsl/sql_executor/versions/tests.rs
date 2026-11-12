use super::*;

#[test]
fn diagnostic_aliases_share_roots_and_restore_publication_after_rollback() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create_readonly(&db, "counts", Schema::integers(2, 1))
        .unwrap();
    store
        .load_rows("counts", vec![vec![Some(1), Some(3)]], false)
        .unwrap();
    let source = store.version("counts");
    let columns = vec!["c0".into(), "changes".into()];
    store
        .alias_readonly(&db, "published_counts", source.clone(), columns.clone())
        .unwrap();
    let alias = store.version("published_counts");
    assert!(Arc::ptr_eq(
        source.full.as_ref().unwrap(),
        alias.full.as_ref().unwrap()
    ));
    assert_eq!(source.row_id(0), alias.row_id(0));
    assert_eq!(
        db.query_row("SELECT changes FROM published_counts WHERE c0=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert!(db.execute("DELETE FROM published_counts", []).is_err());
    super::super::super::facts::vtab::bound_source(&db, "published_counts")
        .unwrap()
        .prepare_index(&[1]);
    assert!(store.version("published_counts").indexed(1));
    let tx = store.transaction(&db).unwrap();
    let changed = source
        .patched_rows(vec![vec![Some(1)]], vec![vec![Some(1), Some(4)]], false)
        .unwrap();
    store
        .alias_readonly(&db, "published_counts", changed, columns)
        .unwrap();
    assert!(store.version("published_counts").indexed(1));
    assert_eq!(
        db.query_row("SELECT changes FROM published_counts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    drop(tx);
    assert!(alias.same(&store.version("published_counts")));
    assert_eq!(
        db.query_row("SELECT changes FROM published_counts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    store.close(&db).unwrap();
}

#[test]
fn selective_indexes_keep_compound_batches_sparse_and_transactional() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create(&db, "batch_rows", Schema::integers(3, 2))
        .unwrap();
    store.restrict_indexes("batch_rows", &[]).unwrap();
    db.execute_batch("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<20000) INSERT INTO batch_rows SELECT 1,i,i%7 FROM n").unwrap();
    let before = store.version("batch_rows");
    assert!(!before.indexed(1));
    assert!(before.indexed(0));
    let reads = store.columns_read();
    let n: i64 = db
        .query_row(
            "SELECT count(*) FROM batch_rows WHERE c0='1' AND c1 IN(17,811,NULL,17,'wrong')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
    assert!(store.columns_read() - reads < 100);
    let plan: String = db
        .query_row(
            "EXPLAIN QUERY PLAN SELECT * FROM batch_rows WHERE c0=1 AND c1 IN(17,811)",
            [],
            |r| r.get(3),
        )
        .unwrap();
    assert!(plan.contains("16777216"), "{plan}");
    db.execute_batch("CREATE TEMP TABLE changed(a,b);INSERT INTO changed VALUES(1,17),(2,811)")
        .unwrap();
    let tx = store.transaction(&db).unwrap();
    let reads = store.columns_read();
    assert_eq!(
        db.execute(
            "DELETE FROM batch_rows WHERE (c0,c1) IN(SELECT a,b FROM changed)",
            []
        )
        .unwrap(),
        1
    );
    assert!(store.columns_read() - reads < 100);
    assert_eq!(before.len(), 20000);
    assert_eq!(store.version("batch_rows").len(), 19999);
    super::super::super::facts::vtab::bound_source(&db, "batch_rows")
        .unwrap()
        .prepare_index(&[2]);
    assert!(store.version("batch_rows").indexed(2));
    assert_eq!(
        db.query_row("SELECT count(*) FROM batch_rows WHERE c2=3", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2857 - 1
    );
    drop(tx);
    assert!(before.same(&store.version("batch_rows")));
    assert!(!store.version("batch_rows").indexed(2));
    assert!(!store.version("batch_rows").empty_like().indexed(1));
}

#[test]
fn bulk_indexes_have_the_canonical_shape_and_allow_edits() {
    let entries = (0..4096).map(|i| (key([Some(i)]), i)).collect::<Vec<_>>();
    for hashed in [false, true] {
        let bulk = Node::bulk(entries.clone(), hashed);
        let mut incremental = None;
        for (key, value) in entries.iter().rev() {
            incremental = if hashed {
                Node::set(&incremental, key.clone(), Some(*value))
            } else {
                Node::set_index(&incremental, key.clone(), Some(*value))
            };
        }
        fn shape(n: &Link<i64>, out: &mut Vec<(Vec<Option<i64>>, usize, [u8; 32])>) {
            if let Some(n) = n {
                out.push((n.key.to_vec(), n.size, n.digest));
                shape(&n.left, out);
                shape(&n.right, out);
            }
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        shape(&bulk, &mut a);
        shape(&incremental, &mut b);
        assert_eq!(a, b);
        let changed = if hashed {
            Node::set(&bulk, key([Some(17)]), None)
        } else {
            Node::set_index(&bulk, key([Some(17)]), None)
        };
        assert_eq!(tree::size(&changed), 4095);
        assert!(Node::get(&changed, &[Some(17)]).is_none());
    }
}
pub fn data(db: &rusqlite::Connection, name: &str) -> Vec<Vec<Option<i64>>> {
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
    let reads = store.columns_read();
    let rows = db
        .prepare("SELECT c0,c1 FROM version_extrema WHERE c1=17 UNION SELECT c0,c1 FROM version_extrema WHERE c1=18")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows, vec![(1, 17), (1, 18)]);
    assert!(store.columns_read() - reads < 100);
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

#[test]
fn native_rollback_restores_removed_tables_and_roots_across_sql_savepoints() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create(&db, "version_lifecycle", Schema::integers(2, 1))
        .unwrap();
    db.execute("INSERT INTO version_lifecycle VALUES(1,2)", [])
        .unwrap();
    let before = store.version("version_lifecycle");
    db.execute_batch("SAVEPOINT lifecycle").unwrap();
    store.begin().unwrap();
    store
        .replace("version_lifecycle", before.empty_like())
        .unwrap();
    store.remove(&db, "version_lifecycle").unwrap();
    store
        .create(&db, "version_lifecycle", Schema::integers(2, 1))
        .unwrap();
    db.execute("INSERT INTO version_lifecycle VALUES(3,4)", [])
        .unwrap();
    store.remove(&db, "version_lifecycle").unwrap();
    store
        .create(&db, "version_lifecycle", Schema::integers(2, 1))
        .unwrap();
    db.execute("INSERT INTO version_lifecycle VALUES(5,6)", [])
        .unwrap();
    store.rollback(&db).unwrap();
    db.execute_batch("ROLLBACK TO lifecycle; RELEASE lifecycle")
        .unwrap();
    assert!(before.same(&store.version("version_lifecycle")));
    assert_eq!(data(&db, "version_lifecycle"), vec![vec![Some(1), Some(2)]]);
    db.execute("INSERT INTO version_lifecycle VALUES(7,8)", [])
        .unwrap();
    assert_eq!(store.version("version_lifecycle").len(), 2);
    db.execute_batch("SAVEPOINT automatic_rollback").unwrap();
    store.begin().unwrap();
    store.remove(&db, "version_lifecycle").unwrap();
    store
        .create(&db, "version_interrupted", Schema::integers(2, 1))
        .unwrap();
    db.execute("INSERT INTO version_interrupted VALUES(3,4)", [])
        .unwrap();
    // SQLite can roll back the entire transaction when a write is interrupted.
    db.execute_batch("ROLLBACK").unwrap();
    store.rollback(&db).unwrap();
    assert_eq!(store.version("version_lifecycle").len(), 2);
    assert!(db.prepare("SELECT * FROM version_interrupted").is_err());
    assert_eq!(
        data(&db, "version_lifecycle"),
        vec![vec![Some(1), Some(2)], vec![Some(7), Some(8)]]
    );
    store.close(&db).unwrap();
}

#[test]
fn scopes_reuse_sql_slots_and_restore_binding_cells_when_tables_return_on_rollback() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create(&db, "scoped_rows", Schema::integers(2, 1))
        .unwrap();
    db.execute("INSERT INTO scoped_rows VALUES(1,2)", [])
        .unwrap();
    let schema = db
        .query_row("PRAGMA temp.schema_version", [], |r| r.get::<_, i64>(0))
        .unwrap();
    let next = store.fork_scope(0).unwrap();
    assert_eq!(
        db.query_row("PRAGMA temp.schema_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        schema
    );
    assert!(Arc::ptr_eq(
        store.version("scoped_rows").full.as_ref().unwrap(),
        store.version_at(next, "scoped_rows").full.as_ref().unwrap()
    ));
    {
        let _scope = store.activate(next).unwrap();
        db.execute("UPDATE scoped_rows SET c1=9", []).unwrap();
        assert_eq!(data(&db, "scoped_rows"), vec![vec![Some(1), Some(9)]]);
        {
            let _original = store.activate(0).unwrap();
            assert_eq!(data(&db, "scoped_rows"), vec![vec![Some(1), Some(2)]]);
        }
        assert_eq!(data(&db, "scoped_rows"), vec![vec![Some(1), Some(9)]]);
    }
    assert_eq!(data(&db, "scoped_rows"), vec![vec![Some(1), Some(2)]]);
    db.execute_batch("SAVEPOINT scoped_lifecycle").unwrap();
    store.begin().unwrap();
    store.remove_all(&db, "scoped_rows").unwrap();
    store
        .create(&db, "scoped_rows", Schema::integers(2, 1))
        .unwrap();
    db.execute("INSERT INTO scoped_rows VALUES(3,4)", [])
        .unwrap();
    store.rollback(&db).unwrap();
    db.execute_batch("ROLLBACK TO scoped_lifecycle; RELEASE scoped_lifecycle")
        .unwrap();
    assert_eq!(data(&db, "scoped_rows"), vec![vec![Some(1), Some(2)]]);
    {
        let _scope = store.activate(next).unwrap();
        assert_eq!(data(&db, "scoped_rows"), vec![vec![Some(1), Some(9)]]);
        db.execute("UPDATE scoped_rows SET c1=11", []).unwrap();
        assert_eq!(store.version("scoped_rows").value(0, 1), 11);
    }
    assert_eq!(store.version("scoped_rows").value(0, 1), 2);
    store.close(&db).unwrap();
}

#[test]
fn interrupted_slot_creation_releases_the_module_before_it_can_be_owned() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    db.progress_handler(1, Some(|| true)).unwrap();
    assert!(
        store
            .create(&db, "version_failed", Schema::integers(2, 1))
            .is_err()
    );
    db.progress_handler(0, None::<fn() -> bool>).unwrap();
    assert!(!store.owns("version_failed"));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM pragma_module_list WHERE name='version_failed'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
