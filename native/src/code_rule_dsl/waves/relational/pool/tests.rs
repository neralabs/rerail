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
            super::super::source::bind(
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
    fn rebinding_functions_preserves_prepared_statements_and_isolates_source_and_work() {
        use crate::code_rule_dsl::model::{FileInput, Snapshot};
        let snapshot = |source: &str| Snapshot {
            revision: 1,
            files: vec![
                FileInput::new(
                    "active.py".into(),
                    "python".into(),
                    Arc::new(source.into()),
                    None,
                    None,
                    "test",
                )
                .unwrap(),
            ],
        };
        let pool = Pool::default();
        let old = Arc::new(AtomicUsize::new(100));
        let first = pool.take(1024, old.clone()).unwrap();
        super::super::source::bind(&first, Some(&snapshot("one")), old.clone()).unwrap();
        let sql = "SELECT source_text('active.py',0,3),rx_matches('one','one')";
        let mut statement = first.prepare_cached(sql).unwrap();
        assert_eq!(
            statement.query_row([], |r| r.get::<_, String>(0)).unwrap(),
            "one"
        );
        let previous = old.load(std::sync::atomic::Ordering::Relaxed);
        let current = Arc::new(AtomicUsize::new(100));
        let second = pool.take(1024, current.clone()).unwrap();
        super::super::source::bind(&second, Some(&snapshot("two")), current.clone()).unwrap();
        assert_eq!(
            statement.query_row([], |r| r.get::<_, String>(0)).unwrap(),
            "two"
        );
        assert_eq!(
            statement.get_status(rusqlite::StatementStatus::RePrepare),
            0
        );
        assert_eq!(old.load(std::sync::atomic::Ordering::Relaxed), previous);
        assert!(current.load(std::sync::atomic::Ordering::Relaxed) < 100);
        first.activate(1024, old.clone()).unwrap();
        super::super::source::bind(&first, Some(&snapshot("one")), old).unwrap();
        assert_eq!(
            statement.query_row([], |r| r.get::<_, String>(0)).unwrap(),
            "one"
        );
    }
    #[test]
    fn an_open_sql_cursor_prevents_switching_its_execution_binding() {
        let pool = Pool::default();
        let first = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
        first
            .execute_batch("CREATE TABLE frame(v);INSERT INTO frame VALUES(1),(2)")
            .unwrap();
        let mut statement = first.prepare("SELECT v FROM frame").unwrap();
        let mut rows = statement.query([]).unwrap();
        assert!(rows.next().unwrap().is_some());
        assert!(pool.take(1024, Arc::new(AtomicUsize::new(100))).is_err());
        assert_eq!(rows.next().unwrap().unwrap().get::<_, i64>(0).unwrap(), 2);
        drop(rows);
        drop(statement);
        assert!(pool.take(1024, Arc::new(AtomicUsize::new(100))).is_ok());
    }
    #[test]
    fn incomplete_or_failed_consumers_cannot_recycle_partial_rows() {
        let pool = Pool::default();
        {
            let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
            let name = db.private("unfinished");
            db.storage
                .borrow_mut()
                .declare(&db, &name, format!("CREATE TABLE {name}(value)"), true)
                .unwrap();
            db.execute_batch(&format!("INSERT INTO {name} VALUES(1)"))
                .unwrap();
            db.storage.borrow_mut().written(&name, true);
        }
        let db = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
        assert!(db.prepare("SELECT * FROM __private_1_unfinished").is_err());
    }
