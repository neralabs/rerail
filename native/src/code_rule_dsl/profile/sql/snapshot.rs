//! Preserve the failed SQL owner's temporary relations for bounded offline diagnosis.
use rusqlite::{Connection, Result};
use std::path::Path;

fn save(_source: &Connection, _path: &Path) -> Result<()> {
    Err(rusqlite::Error::InvalidQuery)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_snapshot_keeps_temp_values_indexes_and_source_ownership() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let source = Connection::open_in_memory()?;
        source.execute_batch("CREATE TABLE unrelated_main(value);INSERT INTO unrelated_main VALUES(99);CREATE TEMP TABLE evidence(path TEXT,owner INTEGER,value);CREATE INDEX evidence_owner ON evidence(path,owner);INSERT INTO evidence VALUES('é.py',3,NULL),('é.py',7,'body');")?;
        let changed=source.total_changes();
        let path=dir.path().join("failure.sqlite");
        save(&source,&path)?;
        assert_eq!(source.total_changes(),changed);
        source.execute_batch("DROP TABLE evidence")?;
        let captured=Connection::open(&path)?;
        let rows=captured.prepare("SELECT owner,value FROM evidence ORDER BY owner")?.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?)))?.collect::<Result<Vec<_>>>()?;
        assert_eq!(rows,[(3,None),(7,Some("body".into()))]);
        assert_eq!(captured.query_row("SELECT count(*) FROM sqlite_schema WHERE name='evidence_owner'",[],|r|r.get::<_,i64>(0))?,1);
        assert_eq!(captured.query_row("SELECT count(*) FROM sqlite_schema WHERE name='unrelated_main'",[],|r|r.get::<_,i64>(0))?,0);
        Ok(())
    }
}
