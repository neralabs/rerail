use super::*;
use rusqlite::{Connection, StatementStatus, params_from_iter};

#[test]
fn regex_value_selection_does_not_probe_the_global_vocabulary_for_every_admitted_file() {
    let db = Connection::open_in_memory().unwrap();
    crate::code_rule_dsl::matcher::anchors::sql::postings::install(&db).unwrap();
    db.execute_batch("CREATE TABLE cf(fid INTEGER PRIMARY KEY);
        CREATE TABLE vidx(lang INTEGER,term TEXT,fid INTEGER,kind INTEGER,postings BLOB,
            PRIMARY KEY(lang,term,fid,kind)) WITHOUT ROWID;
        CREATE INDEX vidx_file ON vidx(fid);
        CREATE TABLE vocabulary_term(lang INTEGER,expression TEXT,term TEXT,
            PRIMARY KEY(lang,expression,term)) WITHOUT ROWID;
        INSERT INTO cf WITH RECURSIVE files(fid) AS (SELECT 1 UNION ALL SELECT fid+1 FROM files WHERE fid<32) SELECT fid FROM files;
        INSERT INTO vidx SELECT 1,'vocab00000',fid,1,X'01' FROM cf;
        INSERT INTO vidx WITH RECURSIVE words(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM words WHERE n<19999)
            SELECT 1,printf('vocab%05d',n),999,1,X'01' FROM words;
        INSERT INTO vocabulary_term SELECT DISTINCT lang,'^vocab',term FROM vidx;")
        .unwrap();
    let mut lower = Lower::new(1);
    lower.text(&lower.all(), "^vocab", false).unwrap();
    let values = lower.reuse.values.values().next().unwrap();
    let query = super::super::Definitions::new(lower.ctes)
        .render(&format!("SELECT fid,pre FROM {}", values.name));
    let mut statement = db.prepare(&query).unwrap();
    let mut actual = statement
        .query_map(params_from_iter(lower.parameters), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    actual.sort_unstable();
    assert_eq!(actual, (1..=32).map(|fid| (fid, 1)).collect::<Vec<_>>());
    let steps = statement.get_status(StatementStatus::VmStep);
    assert!(
        steps < 20_000,
        "32 admitted postings must not create 20,000 × 32 vocabulary probes: {steps} VM steps"
    );
}
