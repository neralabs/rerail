    use super::*;
    #[test]
    fn a_successful_lease_returns_empty_backing_and_resets_source_and_function_budgets() {
        let pool = Pool::default();
        let old = Arc::new(AtomicUsize::new(0));
        {
            let db = pool.take(1024, old.clone()).unwrap();
            db.storage
                .borrow_mut()
                .declare(&db, "input", "CREATE TABLE input(value)".into(), true)
                .unwrap();
            db.execute("INSERT INTO input VALUES(7)", []).unwrap();
            db.storage.borrow_mut().written("input", true);
            let file = crate::code_rule_dsl::model::FileInput::new(
                "old.py".into(),
                "python".into(),
                Arc::new("old".into()),
                None,
                None,
                "test",
            )
            .unwrap();
            super::super::source::install(
                &db,
                Some(&crate::code_rule_dsl::model::Snapshot {
                    revision: 1,
                    files: vec![file],
                }),
                Arc::new(AtomicUsize::new(100)),
            )
            .unwrap();
            assert_eq!(
                db.query_row("SELECT source_text('old.py',0,3)", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "old"
            );
            db.reusable.set(true);
        }
        let work = Arc::new(AtomicUsize::new(100));
        let db = pool.take(1024, work.clone()).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM input", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT rx_matches('a','a')", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT source_text('old.py',0,3)", [], |r| r
                .get::<_, Option<String>>(0))
                .unwrap(),
            None
        );
        assert_eq!(old.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(work.load(std::sync::atomic::Ordering::Relaxed) < 100);
    }
    #[test]
    fn incomplete_or_failed_consumers_cannot_recycle_partial_rows() {
        let pool = Pool::default();
        {
            let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
            db.execute_batch("CREATE TABLE unfinished(value);INSERT INTO unfinished VALUES(1)")
                .unwrap();
        }
        let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
        assert!(db.prepare("SELECT * FROM unfinished").is_err());
    }
