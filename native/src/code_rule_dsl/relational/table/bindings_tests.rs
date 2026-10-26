use super::*;
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
