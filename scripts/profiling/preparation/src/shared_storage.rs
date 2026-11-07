//! Connection contract for future shared SQL owners. Uses the production settings and authorizer.
use crate::{authorizer, session};
use rusqlite::{Connection, Result, hooks::Authorization};
use std::{collections::BTreeSet, sync::atomic::Ordering};

fn count(db: &Connection) -> Result<i64> {
    let mut query = db.prepare("SELECT fid,pre,start,len FROM source_spans")?;
    let rows = query
        .query_map([], |_| Ok(()))?
        .collect::<Result<Vec<_>>>()?;
    Ok(rows.len() as i64)
}
#[test]
fn writer_publication_scoped_readers_private_scratch_and_authorizer_agree() -> Result<()> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owners.sqlite");
    let writer = Connection::open(&path)?;
    session::configure(&writer)?;
    writer.execute_batch("CREATE TABLE source_spans(fid INTEGER,pre INTEGER,start INTEGER,len INTEGER,PRIMARY KEY(fid,pre)) WITHOUT ROWID;
        CREATE TABLE __stage_hidden(secret INTEGER); INSERT INTO __stage_hidden VALUES(99);
        INSERT INTO source_spans VALUES(1,1,0,4)")?;
    let a = Connection::open(&path)?;
    let b = Connection::open(&path)?;
    for reader in [&a, &b] {
        session::configure(reader)?;
    }
    a.execute_batch(
        "CREATE TEMP TABLE private_rows(x INTEGER); INSERT INTO private_rows VALUES(11)",
    )?;
    b.execute_batch(
        "CREATE TEMP TABLE private_rows(x INTEGER); INSERT INTO private_rows VALUES(22)",
    )?;
    let read = a.unchecked_transaction()?;
    assert_eq!(count(&read)?, 1);
    let publication = writer.unchecked_transaction()?;
    publication.execute("INSERT INTO source_spans VALUES(1,2,4,3)", [])?;
    assert_eq!(count(&b)?, 1, "incomplete publication is invisible");
    publication.commit()?;
    assert_eq!(
        count(&read)?,
        1,
        "an active reader retains its pinned snapshot"
    );
    assert_eq!(
        count(&b)?,
        2,
        "a later reader sees the complete publication"
    );
    read.commit()?;
    assert_eq!(count(&a)?, 2);
    let failed = writer.unchecked_transaction()?;
    failed.execute("INSERT INTO source_spans VALUES(1,3,7,2)", [])?;
    failed.rollback()?;
    assert_eq!(count(&a)?, 2, "failed publication retains no partial rows");

    let allowed = BTreeSet::from(["source_spans".into(), "private_rows".into()]);
    let observed = authorizer::install(&a, &allowed).expect("install scoped authorizer");
    assert_eq!(
        count(&a).expect("authorized owner read"),
        2,
        "the production authorizer admits the registered physical owner"
    );
    assert_eq!(
        a.query_row("SELECT x FROM private_rows", [], |r| r.get::<_, i64>(0))?,
        11
    );
    for query in [
        "SELECT * FROM __stage_hidden",
        "WITH source_spans AS (SELECT * FROM __stage_hidden) SELECT * FROM source_spans",
        "SELECT * FROM sqlite_schema",
        "INSERT INTO private_rows VALUES(99)",
    ] {
        assert!(
            a.prepare(query).is_err(),
            "forbidden authored query: {query}"
        );
    }
    assert_eq!(
        a.query_row("SELECT count(*) FROM source_spans", [], |r| r
            .get::<_, i64>(0))?,
        2
    );
    assert!(!observed.source.load(Ordering::Relaxed));
    assert!(observed.fields.lock().contains_key("source_spans"));
    a.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)?;
    assert_eq!(
        b.query_row("SELECT x FROM private_rows", [], |r| r.get::<_, i64>(0))?,
        22
    );
    drop(b);
    writer.execute("INSERT INTO source_spans VALUES(1,3,7,2)", [])?;
    assert_eq!(
        count(&a)?,
        3,
        "reader closure does not retire another reader's owner"
    );
    assert_eq!(
        a.query_row("SELECT x FROM private_rows", [], |r| r.get::<_, i64>(0))?,
        11
    );
    Ok(())
}
