    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn path_and_node_lookups_do_not_enumerate_other_files() {
        let file = FileInput::new(
            "seed.py".into(),
            "python".into(),
            Arc::new("value = 1\n".into()),
            None,
            None,
            "test",
        )
        .unwrap();
        let facts = Facts::build(&file).unwrap();
        let spec:Spec=serde_json::from_value(serde_json::json!({"name":"nodes","gate":"gate","columns":["__path","id","parent","kind"]})).unwrap();
        let meter = Arc::new(Meter {
            work: Arc::new(AtomicUsize::new(1_000_000)),
            steps: AtomicUsize::new(0),
        });
        let sources = (0..128)
            .map(|i| {
                load_one(
                    &spec,
                    &format!("f{i:03}.py"),
                    Arc::clone(&facts),
                    Arc::clone(&meter),
                )
                .unwrap()
                .remove(0)
            })
            .collect::<Vec<_>>();
        let db = Connection::open_in_memory().unwrap();
        let stats =
            super::super::table::register(&db, "nodes", Arc::new(Union::new(sources))).unwrap();
        let count:i64=db.query_row("SELECT count(*) FROM nodes child CROSS JOIN nodes parent ON parent.__path=child.__path AND parent.id=child.parent WHERE child.kind='identifier'",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 128);
        assert!(
            stats.candidates.load(Ordering::Relaxed) < 128 * 8,
            "scoped node joins enumerated {} candidates",
            stats.candidates.load(Ordering::Relaxed)
        );
        let ids = db
            .prepare("SELECT id FROM nodes WHERE __path='f000.py' ORDER BY id LIMIT 2")
            .unwrap()
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let actual=db.prepare("SELECT __path,id FROM nodes WHERE __path IN ('f001.py','f002.py') AND id IN (?1,?2) ORDER BY __path,id").unwrap().query_map([ids[0],ids[1]],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        assert_eq!(
            actual,
            vec![
                ("f001.py".into(), ids[0]),
                ("f001.py".into(), ids[1]),
                ("f002.py".into(), ids[0]),
                ("f002.py".into(), ids[1])
            ]
        );
        assert!(stats.batches.load(Ordering::Relaxed) > 0);
    }
    #[test]
    fn path_and_span_lookups_do_not_enumerate_other_files() {
        let file = FileInput::new(
            "seed.py".into(),
            "python".into(),
            Arc::new("value = 1\n".repeat(64)),
            None,
            None,
            "test",
        )
        .unwrap();
        let facts = Facts::build(&file).unwrap();
        let spec: Spec = serde_json::from_value(serde_json::json!({
            "name":"nodes","gate":"gate","columns":["__path","__start","__end","id"]
        }))
        .unwrap();
        let meter = Arc::new(Meter {
            work: Arc::new(AtomicUsize::new(5_000_000)),
            steps: AtomicUsize::new(0),
        });
        let sources = (0..32)
            .map(|i| {
                load_one(
                    &spec,
                    &format!("f{i:03}.py"),
                    Arc::clone(&facts),
                    Arc::clone(&meter),
                )
                .unwrap()
                .remove(0)
            })
            .collect::<Vec<_>>();
        let db = Connection::open_in_memory().unwrap();
        let stats =
            super::super::table::register(&db, "nodes", Arc::new(Union::new(sources))).unwrap();
        let demand = (0..32)
            .map(|i| format!("('f{i:03}.py',0,5)"))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "WITH demand(path,lo,hi) AS MATERIALIZED (VALUES {demand}) SELECT count(*) FROM demand d CROSS JOIN nodes n ON n.__path=d.path AND n.__start>=d.lo AND n.__end<=d.hi"
        );
        let count: i64 = db.query_row(&query, [], |r| r.get(0)).unwrap();
        assert_eq!(count, 32);
        assert!(
            stats.candidates.load(Ordering::Relaxed) < 32 * 8,
            "scoped ranges enumerated {} candidates",
            stats.candidates.load(Ordering::Relaxed)
        );
        let count: i64 = db.query_row("SELECT count(*) FROM nodes WHERE __path IN ('f000.py','f001.py') AND __start>=0 AND __end<=5", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
        assert!(stats.batches.load(Ordering::Relaxed) > 0);
    }
