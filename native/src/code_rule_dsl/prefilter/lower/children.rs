//! Field-qualified child requests share transport and SQL execution, while their semantic
//! bindings and completion keys remain distinct. SQL owns both present and absent answers.
pub(super) fn fill(scopes: impl IntoIterator<Item = (u16, String)>) -> (Vec<String>, &'static str) {
    let arms = scopes
        .into_iter()
        .map(|(field, scope)| format!("SELECT fid,pre,{field} AS field FROM {scope}"))
        .collect();
    let requests = super::waves::compound("fid,pre,field", arms);
    (vec![
        format!("needed(fid,pre,field) AS MATERIALIZED (SELECT DISTINCT s.fid,s.pre,s.field FROM ({requests}) s LEFT JOIN decoded_children d ON d.fid=s.fid AND d.pre=s.pre AND d.field=s.field WHERE d.fid IS NULL)"),
        "decoded(fid,pre,field,child) AS MATERIALIZED (SELECT s.fid,s.pre,s.field,nera_child(s.fid,s.pre,s.field) FROM needed s)".into(),
    ],"INSERT INTO decoded_children(fid,pre,field,child) SELECT fid,pre,field,child FROM decoded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, Result, functions::FunctionFlags};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    type Calls = Arc<Mutex<Vec<(i64, i64, i64)>>>;
    fn database(calls: Calls, fail: Arc<AtomicBool>) -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE decoded_children(fid INTEGER,pre INTEGER,field INTEGER,child INTEGER,PRIMARY KEY(fid,pre,field)) WITHOUT ROWID;CREATE TABLE left_keys(fid INTEGER,pre INTEGER);CREATE TABLE right_keys(fid INTEGER,pre INTEGER);INSERT INTO left_keys VALUES(1,1),(1,2),(1,2);INSERT INTO right_keys VALUES(1,2),(1,3)")?;
        db.create_scalar_function(c"nera_child", 3, FunctionFlags::SQLITE_UTF8, move |ctx| {
            let key = (ctx.get::<i64>(0)?, ctx.get::<i64>(1)?, ctx.get::<i64>(2)?);
            calls.lock().unwrap().push(key);
            if fail.load(Ordering::Relaxed) && key.1 == 2 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(if key.2 == 2 { None } else { Some(key.1 + 10) })
        })?;
        Ok(db)
    }
    fn sql() -> String {
        let (ctes, body) = fill([
            (1, "left_keys".into()),
            (1, "right_keys".into()),
            (2, "left_keys".into()),
        ]);
        format!("WITH {} {body}", ctes.join(","))
    }
    #[test]
    fn overlap_fields_absence_and_later_consumers_preserve_exact_completion() -> Result<()> {
        let calls = Calls::default();
        let db = database(calls.clone(), Arc::default())?;
        db.execute(&sql(), [])?;
        let mut actual = calls.lock().unwrap().clone();
        actual.sort_unstable();
        assert_eq!(
            actual,
            [(1, 1, 1), (1, 1, 2), (1, 2, 1), (1, 2, 2), (1, 3, 1)]
        );
        let rows = db
            .prepare("SELECT pre,field,child FROM decoded_children ORDER BY pre,field")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(
            rows,
            [
                (1, 1, Some(11)),
                (1, 2, None),
                (2, 1, Some(12)),
                (2, 2, None),
                (3, 1, Some(13))
            ]
        );
        db.execute(&sql(), [])?;
        assert_eq!(
            calls.lock().unwrap().len(),
            5,
            "NULL is a completed answer, not missing coverage"
        );
        db.execute_batch("DELETE FROM left_keys;DELETE FROM right_keys")?;
        db.execute(&sql(), [])?;
        assert_eq!(
            calls.lock().unwrap().len(),
            5,
            "empty demand performs no lookup"
        );
        db.execute_batch("INSERT INTO left_keys VALUES(2,1)")?;
        db.execute(&sql(), [])?;
        assert_eq!(
            calls.lock().unwrap().len(),
            7,
            "a different source owns different keys"
        );
        db.execute_batch(
            "DELETE FROM decoded_children WHERE fid=1;INSERT INTO right_keys VALUES(1,1)",
        )?;
        db.execute(&sql(), [])?;
        assert_eq!(
            calls.lock().unwrap().len(),
            8,
            "generation invalidation removes old completion"
        );
        Ok(())
    }
    #[test]
    fn failed_publication_preserves_existing_rows_and_retry_completes_missing_keys() -> Result<()> {
        let calls = Calls::default();
        let fail = Arc::new(AtomicBool::new(true));
        let db = database(calls.clone(), fail.clone())?;
        db.execute("INSERT INTO decoded_children VALUES(9,9,2,NULL)", [])?;
        let tx = db.unchecked_transaction()?;
        assert!(tx.execute(&sql(), []).is_err());
        tx.rollback()?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM decoded_children", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        fail.store(false, Ordering::Relaxed);
        calls.lock().unwrap().clear();
        let tx = db.unchecked_transaction()?;
        tx.execute(&sql(), [])?;
        tx.commit()?;
        assert_eq!(calls.lock().unwrap().len(), 5);
        assert_eq!(
            db.query_row("SELECT count(*) FROM decoded_children", [], |r| r
                .get::<_, i64>(0))?,
            6
        );
        Ok(())
    }
    #[test]
    fn many_fields_and_scopes_use_one_statement_without_losing_field_tags() -> Result<()> {
        let calls = Calls::default();
        let db = database(calls.clone(), Arc::default())?;
        db.execute_batch("CREATE TABLE input(fid,pre);INSERT INTO input VALUES(1,1)")?;
        let (ctes, body) = fill((0..700).map(|field| (field, "input".into())));
        db.execute(&format!("WITH {} {body}", ctes.join(",")), [])?;
        let count = db.query_row(
            "SELECT count(*),count(DISTINCT field) FROM decoded_children",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        assert_eq!(count, (700, 700));
        assert_eq!(calls.lock().unwrap().len(), 700);
        Ok(())
    }
}
