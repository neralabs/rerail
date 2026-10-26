use super::*;

fn fixture() -> (Connection, BTreeMap<String, i64>) {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE n(fid,pre,parent,end,kind,field,flags,PRIMARY KEY(fid,pre)) WITHOUT ROWID; CREATE INDEX n_kind ON n(kind,fid,pre); CREATE INDEX n_parent ON n(fid,parent,field,pre); CREATE TABLE file_kind_counts(fid,code,n); CREATE TEMP TABLE demand(scope PRIMARY KEY,fid,lo,hi) WITHOUT ROWID; INSERT INTO n VALUES(1,0,-1,4,145,0,0),(1,1,0,2,1,19,0),(1,2,0,4,145,0,0),(1,3,2,4,1,19,0),(2,0,-1,2,145,0,0),(2,1,0,2,1,19,0); INSERT INTO demand VALUES(0,1,0,4),(1,1,2,4),(2,2,0,2); INSERT INTO file_kind_counts VALUES(1,145,2),(1,1,20),(2,145,1),(2,1,10)").unwrap();
    let v = BTreeMap::from([
        ("kind:function_definition".into(), 145),
        ("kind:identifier".into(), 1),
        ("field:name".into(), 19),
    ]);
    (db, v)
}
#[test]
fn chosen_arm_uses_stored_stats_and_preserves_correlated_scopes() {
    let (db, v) = fixture();
    let mut m = Meter::default();
    assert_eq!(choose(&db, &v, &mut m).unwrap().0, "parent");
    let mut expected = vec![
        vec![0, 1, 0, 1],
        vec![0, 1, 2, 3],
        vec![1, 1, 2, 3],
        vec![2, 2, 0, 1],
    ];
    expected.sort();
    for choice in ["parent", "child"] {
        let mut r = m.statement(&db, &arm(&v, choice)).unwrap();
        r.sort();
        assert_eq!(r, expected);
    }
    db.execute_batch("UPDATE file_kind_counts SET n=1000 WHERE code=145; UPDATE file_kind_counts SET n=1 WHERE code=1").unwrap();
    assert_eq!(choose(&db, &v, &mut m).unwrap().0, "child");
    db.execute_batch("DELETE FROM demand").unwrap();
    assert!(m.statement(&db, &arm(&v, "parent")).unwrap().is_empty());
}
#[test]
fn materialization_and_fusion_preserve_bags_and_release_private_schema() {
    let (db, v) = fixture();
    let base = arm(&v, "parent");
    let mut expected = None;
    for variant in ["staged", "inline", "materialized", "fused"] {
        let mut r = fusion(&db, &base, variant, &mut Meter::default()).unwrap();
        r.sort();
        if let Some(e) = &expected {
            assert_eq!(&r, e);
        } else {
            expected = Some(r);
        }
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_temp_schema WHERE name='shared'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[test]
fn reused_statement_resets_parameters_without_stale_consumer_state() {
    let (db, v) = fixture();
    let base = arm(&v, "parent");
    let mut expected = None;
    for variant in ["private-schema", "reprepare", "reuse"] {
        let mut r = planning(&db, &base, variant, 3, &mut Meter::default()).unwrap();
        r.sort();
        if let Some(e) = &expected {
            assert_eq!(&r, e);
        } else {
            expected = Some(r);
        }
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_temp_schema WHERE name LIKE 'consumer_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}

#[test]
fn empty_demand_does_not_enter_an_expensive_inner_arm() {
    use rusqlite::functions::FunctionFlags;
    let (db, v) = fixture();
    db.execute_batch("DELETE FROM demand").unwrap();
    db.create_scalar_function(
        c"tripwire",
        1,
        FunctionFlags::SQLITE_UTF8,
        |_| -> rusqlite::Result<i64> { Err(rusqlite::Error::InvalidQuery) },
    )
    .unwrap();
    let sql = arm(&v, "parent") + " AND tripwire(c.pre)";
    assert!(Meter::default().statement(&db, &sql).unwrap().is_empty());
}

#[test]
fn native_batch_keeps_overlapping_scopes_and_releases_blob_handles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!(
        "../../../../../native/src/code_rule_dsl/matcher/anchors/sql/schema.sql"
    ))
    .unwrap();
    db.execute_batch("INSERT INTO kind VALUES(1,145,'function_definition',1),(1,1,'identifier',1); INSERT INTO field VALUES(1,19,'name')").unwrap();
    // Root function, a name, a non-name identifier, and another name. SQL/native retain
    // both matching children; the one-child scalar comparison is used only on real Python names.
    let mut rec = Vec::new();
    for words in [
        [145u16, 1, 4],
        [1 | (19 << 10), 1, 1],
        [1, 2, 1],
        [1 | (19 << 10), 3, 1],
    ] {
        for word in words {
            rec.extend(word.to_le_bytes());
        }
    }
    for fid in 1..=2 {
        db.execute("INSERT INTO file(fid,nodes,path,digest,lang,src,rec,spans,checkpoints,parb,sizb,flags) VALUES(?1,4,'test.py',x'01',1,x'',?2,x'',x'',x'',x'',x'01010101')",rusqlite::params![fid,rec]).unwrap();
        db.execute("INSERT INTO file_kind_counts VALUES(?1,145,1,x'00')", [fid])
            .unwrap();
        db.execute("INSERT INTO decoded_properties VALUES(?1,0,-1,4,145,0,1),(?1,1,0,2,1,19,1),(?1,2,0,3,1,0,1),(?1,3,0,4,1,19,1)",[fid]).unwrap();
    }
    drop(db);
    let store = crate::store::Store::open(&path).unwrap();
    facts(&store.db, &[1, 2], &mut Meter::default()).unwrap();
    // 300 distinct scope IDs ask for the same two parents. They are separate demands.
    store.db.execute_batch("CREATE TEMP TABLE demand(scope PRIMARY KEY,fid,lo,hi) WITHOUT ROWID; WITH RECURSIVE ids(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM ids WHERE i<299) INSERT INTO demand SELECT i,1+i%2,0,4 FROM ids").unwrap();
    let v = vocabulary(&store.db).unwrap();
    let (mut expected, _) =
        native::run(&store, &v, "parent", "sql", &mut Meter::default()).unwrap();
    expected.sort();
    let (mut actual, c) =
        native::run(&store, &v, "parent", "batch", &mut Meter::default()).unwrap();
    actual.sort();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 600);
    assert_eq!(c["filters"], 1);
    assert_eq!(c["visitedDirectChildren"], 900);
    assert_eq!(c["columnCallbacks"], 2400);
    store.db.execute_batch("DELETE FROM demand").unwrap();
    let (rows, c) = native::run(&store, &v, "parent", "batch", &mut Meter::default()).unwrap();
    assert!(rows.is_empty());
    assert_eq!(c["filters"], 0);
    // DDL succeeds after the operator. This fails if incremental BLOB handles leak.
    store.db.execute_batch("DROP TABLE n").unwrap();
}
