use super::*;

#[test]
fn file_and_parent_filters_do_not_visit_equal_node_ids_in_other_files() -> Result<()> {
    use crate::code_rule_dsl::{model::FileInput,relational::syntax};
    use std::sync::atomic::AtomicUsize;
    let file=FileInput::new("f0.py".into(),"python".into(),Arc::new("f(a,b,c,d,e,f,g,h,i,j)\n".into()),None,None,"test").unwrap();
    let facts=syntax::Facts::build(&file).unwrap();
    let files=(0..128).map(|i|(format!("f{i}.py"),Arc::clone(&facts))).collect();
    let spec:syntax::Spec=serde_json::from_value(serde_json::json!({"name":"nodes","gate":"gate","owners":{}})).unwrap();
    let source=syntax::sources(&spec,&files,Arc::new(AtomicUsize::new(usize::MAX))).unwrap().remove(0).1;
    let db=super::super::Database::open()?;
    let binding=Binding { schema:Arc::new(Metadata {columns:source.columns(),types:source.column_types()}),current:Arc::new(parking_lot::RwLock::new(source)) };
    let stats=register_inner(&db,"scoped_nodes",binding)?;
    let parent:i64=db.query_row("SELECT parent FROM scoped_nodes WHERE __path='f7.py' AND kind='identifier' ORDER BY id DESC LIMIT 1",[],|row|row.get(0))?;
    let query=|sql:&str|->Result<Vec<i64>>{db.prepare(sql)?.query_map([parent],|row|row.get(0))?.collect()};
    let expected=query("SELECT id FROM scoped_nodes WHERE __path='f7.py' AND +parent=?1 ORDER BY id")?;
    assert!(!expected.is_empty());
    stats.candidates.store(0,Ordering::Relaxed);
    assert_eq!(query("SELECT id FROM scoped_nodes WHERE __path='f7.py' AND parent=?1 ORDER BY id")?,expected);
    assert!(stats.candidates.load(Ordering::Relaxed)<=expected.len() as u64+1,"scoped lookup visited {} candidates for {} matches",stats.candidates.load(Ordering::Relaxed),expected.len());
    Ok(())
}
struct Rows(Vec<i64>);
impl Source for Rows {
    fn columns(&self) -> Vec<String> {
        vec!["value".into()]
    }
    fn column_types(&self) -> Vec<String> {
        vec!["INTEGER".into()]
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        self.len()
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    fn lookup(&self, _: usize, _: &[Value]) -> Result<Option<Vec<usize>>> {
        Ok(None)
    }
    fn column(&self, row: usize, _: usize, ctx: &mut Context) -> Result<()> {
        ctx.set_result(&self.0[row])
    }
    fn charge(&self, _: usize) -> Result<()> {
        Ok(())
    }
}
#[test]
fn completed_lease_releases_native_payloads_and_preserves_other_active_owners() -> Result<()> {
    use super::super::pool::Pool;
    use std::sync::atomic::AtomicUsize;
    let pool = Pool::default();
    let first = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
    let second = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
    let payload = Arc::new(Rows(vec![8]));
    let weak = Arc::downgrade(&payload);
    bind(&second, "__context_payload", payload.clone())?;
    drop(payload);
    let schema: i64 = second.query_row("PRAGMA temp.schema_version", [], |r| r.get(0))?;
    drop(first);
    assert_eq!(
        second.query_row("SELECT value FROM __context_payload", [], |r| r
            .get::<_, i64>(0))?,
        8
    );
    drop(second);
    assert!(
        weak.upgrade().is_none(),
        "completed owner retained native rows in the reusable module"
    );
    let third = pool.take(1024, Arc::new(AtomicUsize::new(100))).unwrap();
    assert_eq!(
        third.query_row("PRAGMA temp.schema_version", [], |r| r.get::<_, i64>(0))?,
        schema
    );
    assert_eq!(
        third.query_row("SELECT count(*) FROM __context_payload", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}
#[test]
fn rebinding_preserves_open_cursor_schema_and_releases_old_payloads() -> Result<()> {
    let db = super::super::Database::open()?;
    let first = Arc::new(Rows(vec![1, 2]));
    let weak = Arc::downgrade(&first);
    bind(&db, "__context_native", first.clone())?;
    drop(first);
    let mut statement = db.prepare_cached("SELECT value FROM __context_native")?;
    let mut rows = statement.query([])?;
    assert_eq!(rows.next()?.unwrap().get::<_, i64>(0)?, 1);
    bind(&db, "__context_native", Arc::new(Rows(vec![8])))?;
    assert_eq!(rows.next()?.unwrap().get::<_, i64>(0)?, 2);
    assert!(rows.next()?.is_none());
    drop(rows);
    drop(statement);
    let mut statement = db.prepare_cached("SELECT value FROM __context_native")?;
    assert_eq!(statement.query_row([], |r| r.get::<_, i64>(0))?, 8);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        statement.get_status(rusqlite::StatementStatus::RePrepare),
        0
    );
    drop(statement);
    let binding = db.modules.borrow()["__context_native"].clone();
    binding.clear();
    assert_eq!(
        db.query_row("SELECT count(*) FROM __context_native", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn cached_execution_plans_distinguish_empty_and_populated_native_estimates() -> Result<()> {
    use std::collections::BTreeMap;
    let db = super::super::Database::open()?;
    let sources =
        |values| BTreeMap::from([("native".into(), Arc::new(Rows(values)) as Arc<dyn Source>)]);
    let empty = sources(vec![]);
    let populated = sources(vec![1, 2, 3, 4]);
    assert_ne!(plan_signature(&empty), plan_signature(&populated));
    bind(&db, "native", empty["native"].clone())?;
    db.plan_signature.set(plan_signature(&empty));
    assert_eq!(
        db.prepare_execution("SELECT count(*) FROM native")?
            .query_row([], |r| r.get::<_, i64>(0))?,
        0
    );
    bind(&db, "native", populated["native"].clone())?;
    db.plan_signature.set(plan_signature(&populated));
    assert_eq!(
        db.prepare_execution("SELECT count(*) FROM native")?
            .query_row([], |r| r.get::<_, i64>(0))?,
        4
    );
    Ok(())
}
