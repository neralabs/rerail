use super::*;
#[test]
fn borrowed_tree_differences_match_set_differences_for_dense_and_disjoint_versions() {
    for seed in 0..64_i64 {
        let choose = |i: i64, salt: i64| {
            (i.wrapping_mul(6364136223846793005)
                .wrapping_add(seed * salt) as u64)
                .rotate_left((seed % 63) as u32)
                % 5
                < 3
        };
        let old = (0..512).filter(|&i| choose(i, 17)).collect::<BTreeSet<_>>();
        let new = (128..640)
            .filter(|&i| choose(i, 37))
            .collect::<BTreeSet<_>>();
        let build = |rows: &BTreeSet<i64>| {
            Node::bulk(rows.iter().map(|&i| (key([Some(i)]), i)).collect(), true)
        };
        let before = build(&old);
        let after = build(&new);
        let (mut plus, mut minus, mut visited) = (vec![], vec![], 0);
        Node::difference(&before, &after, &mut plus, &mut minus, &mut visited);
        assert_eq!(
            plus,
            new.difference(&old).copied().collect::<Vec<_>>(),
            "seed {seed}"
        );
        assert_eq!(
            minus,
            old.difference(&new).copied().collect::<Vec<_>>(),
            "seed {seed}"
        );
        let mut unchanged = 0;
        Node::difference(&before, &before, &mut vec![], &mut vec![], &mut unchanged);
        assert_eq!(unchanged, 1);
    }
}
#[test]
fn equal_phase_outputs_share_validated_roots_without_weakening_keys_or_constraints() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    for (name, keys) in [
        ("reuse_source", 1),
        ("reuse_a", 1),
        ("reuse_b", 2),
        ("reuse_c", 1),
    ] {
        store.create(&db, name, Schema::integers(2, keys)).unwrap();
    }
    store.restrict_indexes("reuse_a", &[]).unwrap();
    let rows = (0..1000)
        .map(|i| vec![Some(i), Some(i + 1)])
        .collect::<Vec<_>>();
    store
        .load_rows("reuse_source", rows.clone(), false)
        .unwrap();
    let pool = super::super::scratch::Pool::default();
    let mut work = super::super::scratch::Work::new(&db, &pool);
    work.versions = Some(&store);
    let mut phase = super::super::phase::Phase::default();
    let values = phase.relation(2, "SELECT * FROM reuse_source ORDER BY c0");
    phase.load_empty(&store, "reuse_a", &values).unwrap();
    phase.load_empty(&store, "reuse_b", &values).unwrap();
    let keys = phase.view(1, &format!("SELECT c0 FROM {values}"));
    phase
        .patch_computed(&store, "reuse_c", &keys, &values)
        .unwrap();
    phase.execute(&work).unwrap();
    let a = store.version("reuse_a");
    let b = store.version("reuse_b");
    let c = store.version("reuse_c");
    assert!(Arc::ptr_eq(
        a.full.as_ref().unwrap(),
        c.full.as_ref().unwrap()
    ));
    assert!(
        a.empty_like()
            .patched_rows_reusing(
                (1..1000).map(|i| vec![Some(i)]).collect(),
                rows.clone(),
                false,
                &[a.clone()]
            )
            .is_err(),
        "root reuse must validate patch containment first"
    );
    assert!(Arc::ptr_eq(
        a.full.as_ref().unwrap(),
        b.full.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        a.ids.as_ref().unwrap(),
        b.ids.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        b.primary.as_ref().unwrap(),
        b.full.as_ref().unwrap()
    ));
    assert!(
        !a.secondary[1] && b.secondary[1],
        "each output retains its requested indexes"
    );
    let mut invalid = rows.clone();
    invalid[1][0] = Some(0);
    assert!(a.empty_like().reuse_empty_rows(&invalid, &a).is_none());
    let narrow = Version::empty(Arc::new(Schema::integers(2, 1)));
    assert!(
        narrow.reuse_empty_rows(&rows, &b).is_none(),
        "a wider key cannot prove a narrower key"
    );
    let mut schema = Schema::integers(2, 1);
    schema.positive.insert(1, true);
    assert!(
        Version::empty(Arc::new(schema))
            .reuse_empty_rows(&rows, &a)
            .is_none()
    );
    let mut reordered = rows;
    reordered.reverse();
    assert!(
        a.empty_like().reuse_empty_rows(&reordered, &a).is_none(),
        "new row IDs follow the actual input order"
    );
}
#[test]
fn temporal_empty_and_identical_deltas_share_roots_and_keep_snapshot_row_ids() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create(&db, "delta_source", Schema::integers(2, 1))
        .unwrap();
    let empty = store.version("delta_source");
    store
        .load_rows(
            "delta_source",
            (0..2000).map(|i| vec![Some(i), Some(i + 1)]).collect(),
            false,
        )
        .unwrap();
    let full = store.version("delta_source");
    let pool = super::super::scratch::Pool::default();
    let mut work = super::super::scratch::Work::new(&db, &pool);
    work.versions = Some(&store);
    let add = super::super::temporal::delta(&mut work, &empty, &full).unwrap();
    assert_eq!((add.inserted, add.deleted), (2000, 0));
    let plus = store.available(&add.plus).unwrap();
    assert!(Arc::ptr_eq(
        plus.full.as_ref().unwrap(),
        full.full.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        plus.ids.as_ref().unwrap(),
        full.ids.as_ref().unwrap()
    ));
    let remove = super::super::temporal::delta(&mut work, &full, &empty).unwrap();
    let minus = store.available(&remove.minus).unwrap();
    assert!(Arc::ptr_eq(
        minus.full.as_ref().unwrap(),
        full.full.as_ref().unwrap()
    ));
    let unchanged = super::super::temporal::delta(&mut work, &full, &full).unwrap();
    assert_eq!((unchanged.inserted, unchanged.deleted), (0, 0));
    assert_eq!(
        work.version_nodes, 3,
        "these differences must not visit or rebuild each row"
    );
    // A later write keeps the delta's immutable snapshot and its original row IDs.
    db.execute_batch("UPDATE delta_source SET c1=c1+9 WHERE c0=0")
        .unwrap();
    assert_eq!(plus.rows()[0], vec![Some(0), Some(1)]);
    assert!(plus.same(&minus));
    let changed = store.version("delta_source");
    let difference = super::super::temporal::delta(&mut work, &full, &changed).unwrap();
    assert_eq!((difference.inserted, difference.deleted), (1, 1));
    let added = store.available(&difference.plus).unwrap();
    let removed = store.available(&difference.minus).unwrap();
    let new_row = Node::get(&changed.primary, &[Some(0)]).unwrap();
    let old_row = Node::get(&full.primary, &[Some(0)]).unwrap();
    assert!(Arc::ptr_eq(
        &added.full.as_ref().unwrap().value.values,
        &new_row.values
    ));
    assert!(Arc::ptr_eq(
        &removed.full.as_ref().unwrap().value.values,
        &old_row.values
    ));
    assert_eq!(added.full.as_ref().unwrap().value.id, new_row.id);
    work.cleanup().unwrap();
}
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

#[test]
fn empty_patch_publishes_canonical_roots_with_sparse_row_ids_and_indexes() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = Store::default();
    store
        .create(&db, "empty_patch", Schema::integers(2, 1))
        .unwrap();
    let empty = store.version("empty_patch");
    let rows = (0..10000)
        .rev()
        .map(|i| vec![Some(i), Some(i % 13)])
        .collect::<Vec<_>>();
    let keys = rows.iter().map(|r| r[..1].to_vec()).collect();
    let patched = empty.patched_rows(keys, rows.clone(), false).unwrap();
    let loaded = empty.loaded(rows, false, false).unwrap();
    assert!(patched.same(&loaded));
    for row in [0, 1, 8000, 9999] {
        assert_eq!(patched.row_id(row), loaded.row_id(row));
    }
    assert_eq!(patched.lookup(1, &[3]), loaded.lookup(1, &[3]));
    assert!(
        empty
            .patched_rows(vec![vec![Some(1)]], vec![vec![Some(2), Some(4)]], false)
            .is_err()
    );
    assert!(
        empty
            .patched_rows(vec![vec![Some(1)]], vec![vec![Some(1), None]], false)
            .is_err()
    );
    assert!(
        empty
            .patched_rows(
                vec![vec![Some(1)]],
                vec![vec![Some(1), Some(4)], vec![Some(1), Some(5)]],
                false
            )
            .is_err()
    );
    assert_eq!(store.version("empty_patch").len(), 0);
}

#[test]
fn sparse_patches_match_complete_rebuilds_and_preserve_all_index_rows() {
    let schema = Arc::new(Schema {
        columns: (0..4).map(|c| format!("c{c}")).collect(),
        keys: 2,
        nullable: vec![false, false, false, true],
        positive: BTreeMap::from([(2, false)]),
    });
    for indexes in [vec![1, 2, 3], vec![3]] {
        let mut current = Version::empty(schema.clone())
            .loaded(
                (0..1024)
                    .map(|i| {
                        vec![
                            Some(i / 8),
                            Some(i % 8),
                            Some(i % 5 + 1),
                            (i % 7 != 0).then_some(i % 3),
                        ]
                    })
                    .collect(),
                false,
                false,
            )
            .unwrap();
        current.restrict_indexes(&indexes);
        for edit in 0..64_i64 {
            let before = current.clone();
            let selected = (0..12)
                .map(|i| {
                    let k = (edit * 19 + i * 83) % 1152;
                    vec![Some(k / 8), Some(k % 8)]
                })
                .collect::<Vec<_>>();
            // Mix deletions, new rows, identical rows, nullable values, and
            // updates that keep some secondary index keys unchanged.
            let rows = selected
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(i, pk)| {
                    if (i + edit as usize) % 4 == 0 {
                        return None;
                    }
                    if i % 3 == 0 {
                        if let Some(old) = Node::get(&before.primary, pk) {
                            return Some(old.values.to_vec());
                        }
                    }
                    Some(vec![
                        pk[0],
                        pk[1],
                        Some((edit + i as i64) % 9),
                        (i % 5 != 0).then_some(edit % 3),
                    ])
                })
                .collect::<Vec<_>>();
            let mut complete = before.rows();
            complete.retain(|r| !selected.iter().any(|k| r[..2] == k[..]));
            complete.extend(rows.clone());
            let expected = before.loaded(complete, false, true).unwrap();
            current = before.patched_rows(selected, rows, false).unwrap();
            assert!(current.same(&expected), "edit {edit}");
            assert_eq!(current.next, expected.next);
            assert_eq!(current.distinct, expected.distinct);
            let entries = |root: &Link<Row>| {
                let mut rows = vec![];
                Node::prefix(root, &[], &mut rows);
                rows.into_iter()
                    .map(|r| (r.id, r.values.to_vec()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(entries(&current.full), entries(&expected.full));
            assert_eq!(entries(&current.primary), entries(&expected.primary));
            assert_eq!(entries(&current.ids), entries(&expected.ids));
            for c in 1..4 {
                assert_eq!(entries(&current.indexes[c]), entries(&expected.indexes[c]));
            }
        }
    }
}

#[test]
fn identical_sparse_patches_keep_every_root_and_validate_even_unused_rows() {
    let schema = Arc::new(Schema {
        columns: vec!["c0".into(), "c1".into()],
        keys: 1,
        nullable: vec![false, false],
        positive: BTreeMap::from([(1, true)]),
    });
    let before = Version::empty(schema)
        .loaded(
            (0..2000).map(|i| vec![Some(i), Some(i + 1)]).collect(),
            false,
            false,
        )
        .unwrap();
    let keys = vec![
        vec![Some(17)],
        vec![Some(500)],
        vec![Some(17)],
        vec![Some(3000)],
    ];
    let values = vec![vec![Some(500), Some(501)], vec![Some(17), Some(18)]];
    let after = before.patched_rows(keys, values.clone(), false).unwrap();
    let identical =
        |a: &Link<Row>, b: &Link<Row>| Arc::ptr_eq(a.as_ref().unwrap(), b.as_ref().unwrap());
    assert!(identical(&before.full, &after.full));
    assert!(identical(&before.primary, &after.primary));
    assert!(identical(&before.ids, &after.ids));
    assert!(identical(&before.indexes[1], &after.indexes[1]));
    assert_eq!(before.next, after.next);
    for invalid in [
        vec![vec![Some(17), None]],
        vec![vec![Some(17), Some(0)]],
        vec![vec![Some(17), Some(18)], vec![Some(17), Some(18)]],
        vec![vec![Some(17), Some(19)], vec![Some(500), Some(-1)]],
        vec![vec![Some(18), Some(19)]],
    ] {
        assert!(
            before
                .patched_rows(vec![vec![Some(17)], vec![Some(500)]], invalid, false)
                .is_err()
        );
    }
    assert_eq!(before.rows(), after.rows());
}
