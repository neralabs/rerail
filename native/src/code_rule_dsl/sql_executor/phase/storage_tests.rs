//! Storage-only publication can run inside SQLite without executing another SQL statement.
use super::*;
use parking_lot::Mutex;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};

#[test]
fn native_key_reads_keep_distinct_snapshots_of_the_same_source() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = versions::Store::default();
    for table in ["first", "second"] {
        store
            .create_readonly(&db, table, versions::Schema::integers(2, 1))
            .unwrap();
        store
            .load_rows(
                table,
                vec![vec![Some(1), Some(1)], vec![Some(2), Some(2)]],
                false,
            )
            .unwrap();
    }
    store
        .create_readonly(&db, "keys", versions::Schema::integers(1, 1))
        .unwrap();
    store.load_rows("keys", vec![vec![Some(1)]], false).unwrap();
    let mut phase = Phase::default();
    let first = phase.relation(2, "SELECT 1,11");
    phase.patch(&store, "first", "keys", &first).unwrap();
    store
        .replace(
            "keys",
            store
                .version("keys")
                .loaded(vec![vec![Some(2)]], false, true)
                .unwrap(),
        )
        .unwrap();
    let second = phase.relation(2, "SELECT 2,22");
    phase.patch(&store, "second", "keys", &second).unwrap();
    let (template, snapshots) = phase.compile().unwrap();
    assert_eq!(template.inputs(), vec!["first", "keys", "second", "keys"]);
    assert_eq!(snapshots[1].rows(), vec![vec![Some(1)]]);
    assert_eq!(snapshots[3].rows(), vec![vec![Some(2)]]);
    let mut buffers = template.buffers();
    let mut statement = db.prepare(&template.sql()).unwrap();
    let mut cursor = statement.query([]).unwrap();
    while let Some(row) = cursor.next().unwrap() {
        template
            .collect(
                row.get(0).unwrap(),
                vec![row.get(1).unwrap(), row.get(2).unwrap()],
                &mut buffers,
            )
            .unwrap();
    }
    let built = template.build(&snapshots, buffers).unwrap();
    assert_eq!(
        built.roots[0].1.rows(),
        vec![vec![Some(1), Some(11)], vec![Some(2), Some(2)]]
    );
    assert_eq!(
        built.roots[1].1.rows(),
        vec![vec![Some(1), Some(1)], vec![Some(2), Some(22)]]
    );
    assert_eq!(
        store.version("first").rows(),
        vec![vec![Some(1), Some(1)], vec![Some(2), Some(2)]]
    );
    let mut wrong = snapshots.clone();
    wrong[0] = versions::Version::empty(Arc::new(versions::Schema::integers(2, 2)));
    assert!(template.build(&wrong, template.buffers()).is_err());
    assert!(template.build(&snapshots[..3], template.buffers()).is_err());
    assert!(
        template
            .collect(-1, vec![], &mut template.buffers())
            .is_err()
    );
    assert!(
        template
            .collect(0, vec![Some(1)], &mut template.buffers())
            .is_err()
    );
}

struct Collector {
    template: Arc<Template>,
    snapshots: Arc<Mutex<Vec<versions::Version>>>,
    produced: Arc<Mutex<Option<Materialized>>>,
}
impl Aggregate<Buffers, i64> for Collector {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Buffers> {
        Ok(self.template.buffers())
    }
    fn step(&self, ctx: &mut Context<'_>, buffers: &mut Buffers) -> rusqlite::Result<()> {
        let cells = (1..=self.template.width())
            .map(|c| ctx.get::<Option<i64>>(c))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        self.template
            .collect(ctx.get(0)?, cells, buffers)
            .map_err(|e| rusqlite::Error::UserFunctionError(e.to_string().into()))
    }
    fn finalize(&self, _: &mut Context<'_>, buffers: Option<Buffers>) -> rusqlite::Result<i64> {
        *self.produced.lock() = None;
        let mut snapshots = self.snapshots.lock();
        let built = self
            .template
            .build(
                &snapshots,
                buffers.unwrap_or_else(|| self.template.buffers()),
            )
            .map_err(|e| rusqlite::Error::UserFunctionError(e.to_string().into()))?;
        // This fixture has replacement outputs only. Each new version becomes the next
        // iteration's explicit previous snapshot, without publishing it to the owner.
        *snapshots = built.roots.iter().map(|(_, v)| v.clone()).collect();
        *self.produced.lock() = Some(built);
        Ok(2)
    }
}

#[test]
fn sql_aggregate_reuses_the_phase_builder_across_iterations_and_failure() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let store = versions::Store::default();
    let mut nullable = versions::Schema::integers(2, 1);
    nullable.nullable[1] = true;
    store
        .create_readonly(&db, "nullable_target", nullable)
        .unwrap();
    store
        .create_readonly(&db, "strict_target", versions::Schema::integers(2, 1))
        .unwrap();
    for table in ["nullable_target", "strict_target"] {
        store
            .load_rows(table, vec![vec![Some(1), Some(5)]], false)
            .unwrap();
    }
    store
        .create_readonly(&db, "builder_input", versions::Schema::integers(2, 1))
        .unwrap();
    store
        .load_rows("builder_input", vec![vec![Some(1), Some(41)]], false)
        .unwrap();
    let mut phase = Phase::default();
    let source = phase.relation(
        2,
        "SELECT c0,CASE WHEN c1<0 THEN NULL ELSE c1+1 END FROM builder_input",
    );
    for table in ["nullable_target", "strict_target"] {
        phase.replace_all(&store, table, &source).unwrap();
    }
    let (template, snapshots) = phase.compile().unwrap();
    assert_eq!(
        template.reads.arms.len(),
        1,
        "Shared SQL results still have one read arm"
    );
    let template = Arc::new(template);
    let snapshots = Arc::new(Mutex::new(snapshots));
    let produced = Arc::new(Mutex::new(None));
    db.create_aggregate_function(
        c"rr_build_storage",
        3,
        FunctionFlags::SQLITE_UTF8,
        Collector {
            template: template.clone(),
            snapshots: snapshots.clone(),
            produced: produced.clone(),
        },
    )
    .unwrap();
    let sql = format!(
        "WITH feed(tag,c0,c1) AS ({}) SELECT rr_build_storage(tag,c0,c1) FROM feed",
        template.sql()
    );
    assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(snapshots.lock()[0].rows(), vec![vec![Some(1), Some(42)]]);
    store
        .replace(
            "builder_input",
            store
                .version("builder_input")
                .loaded(vec![vec![Some(2), Some(99)]], false, true)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(snapshots.lock()[0].rows(), vec![vec![Some(2), Some(100)]]);
    assert_eq!(snapshots.lock()[0].row_id(0), 2);
    let before = snapshots.lock().clone();
    store
        .replace(
            "builder_input",
            store
                .version("builder_input")
                .loaded(vec![vec![Some(3), Some(-1)]], false, true)
                .unwrap(),
        )
        .unwrap();
    assert!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).is_err());
    assert!(
        produced.lock().is_none(),
        "A failing later destination returns no partial roots"
    );
    assert!(
        snapshots
            .lock()
            .iter()
            .zip(&before)
            .all(|(a, b)| a.shares_rows(b))
    );
    for table in ["nullable_target", "strict_target"] {
        assert_eq!(
            store.version(table).rows(),
            vec![vec![Some(1), Some(5)]],
            "Owner roots stay unpublished inside the aggregate"
        );
    }
    store
        .replace("builder_input", store.version("builder_input").empty_like())
        .unwrap();
    assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert!(snapshots.lock().iter().all(|v| v.len() == 0));
}
