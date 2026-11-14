//! Bounded typed publication batches, from Pulling-it-all-together's anchor store. SQLite owns
//! stored rows and transaction rollback.
use rusqlite::{Connection, Result, limits::Limit, params_from_iter, types::Value};

pub(super) fn insert<const N: usize>(
    db: &Connection,
    prefix: &str,
    suffix: &str,
    rows: impl IntoIterator<Item = [Value; N]>,
) -> Result<()> {
    let limit = usize::try_from(db.limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)?).unwrap_or(0);
    if N == 0 || limit < N {
        return Err(rusqlite::Error::InvalidParameterCount(N, limit));
    }
    // Bound both SQL text and transient bindings; never build a corpus-sized parameter vector.
    let batch = (4096.min(limit) / N).max(1);
    let mut rows = rows.into_iter().peekable();
    while rows.peek().is_some() {
        let values: Vec<_> = rows.by_ref().take(batch).flatten().collect();
        let row = format!(
            "({})",
            std::iter::repeat_n("?", N).collect::<Vec<_>>().join(",")
        );
        let sql = format!(
            "{prefix} VALUES {} {suffix}",
            std::iter::repeat_n(row, values.len() / N)
                .collect::<Vec<_>>()
                .join(",")
        );
        db.execute(&sql, params_from_iter(values))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_respects_variable_limits_preserves_types_and_rolls_back() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE rows(n INTEGER PRIMARY KEY,t,b)")
            .unwrap();
        db.set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 6)
            .unwrap();
        let tx = db.unchecked_transaction().unwrap();
        insert(
            &tx,
            "INSERT INTO rows",
            "",
            (0..5).map(|n| {
                [
                    Value::Integer(n),
                    Value::Text(format!("row{n}")),
                    Value::Blob(vec![0, n as u8]),
                ]
            }),
        )
        .unwrap();
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM rows WHERE typeof(t)='text' AND typeof(b)='blob'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
        assert!(
            insert(
                &tx,
                "INSERT INTO rows",
                "",
                [[Value::Integer(0), Value::Null, Value::Null]]
            )
            .is_err()
        );
        tx.rollback().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM rows", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        insert::<3>(&db, "INSERT INTO rows", "", []).unwrap();
    }
}
