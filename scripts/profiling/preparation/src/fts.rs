//! Isolate FTS publication and commit boundaries; use the production positional declaration.
use rusqlite::{Connection, params};
use serde_json::{Value, json};

const FILES: i64 = 1024;
const PACKET: i64 = 32;

pub fn experiment() -> Value {
    let mut results = Vec::new();
    for trial in 0..3 {
        let mut modes = [
            "packet-transactions",
            "packet-statements-one-transaction",
            "one-bulk-statement",
        ];
        modes.rotate_left(trial % 3);
        for mode in modes {
            results.push(measure(mode, trial));
        }
    }
    json!({"format":"rerail.fts-publication/v1", "sqliteVersion":rusqlite::version(),
        "files":FILES,"tokensPerFile":256,"packetFiles":PACKET,"results":results,
        "scope":"Only FTS publication and commit CPU. Source staging, declaration, and verification are outside timing. No SQL tracing in these CPU controls. Every write is INSERT SELECT over a batch."})
}

fn measure(mode: &str, trial: usize) -> Value {
    let directory = tempfile::tempdir().unwrap();
    let db = Connection::open(directory.path().join("index.sqlite")).unwrap();
    crate::session::configure(&db).unwrap();
    let declaration =
        include_str!("../../../../native/src/code_rule_dsl/matcher/anchors/sql/schema.sql");
    for sql in declaration.split(';').take(2) {
        db.execute_batch(sql).unwrap();
    }
    db.execute_batch("CREATE TABLE docs(id INTEGER PRIMARY KEY,tokens TEXT NOT NULL)")
        .unwrap();
    let tokens = (0..256)
        .map(|n| format!("k{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    db.execute("WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<?1) INSERT INTO docs SELECT id,?2 FROM ids",params![FILES,tokens]).unwrap();
    let cpu = crate::clock::thread_cpu_ns().expect("thread CPU clock");
    let started = std::time::Instant::now();
    let mut insert_statements = 0;
    let mut commits = 0;
    if mode == "one-bulk-statement" {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute("INSERT INTO ftsk(rowid,k) SELECT id,tokens FROM docs", [])
            .unwrap();
        tx.commit().unwrap();
        insert_statements = 1;
        commits = 1;
    } else {
        let mut whole = if mode == "packet-statements-one-transaction" {
            Some(db.unchecked_transaction().unwrap())
        } else {
            None
        };
        for first in (1..=FILES).step_by(PACKET as usize) {
            let packet = if whole.is_none() {
                Some(db.unchecked_transaction().unwrap())
            } else {
                None
            };
            db.execute(
                "INSERT INTO ftsk(rowid,k) SELECT id,tokens FROM docs WHERE id>=?1 AND id<?2",
                params![first, first + PACKET],
            )
            .unwrap();
            insert_statements += 1;
            if let Some(tx) = packet {
                tx.commit().unwrap();
                commits += 1;
            }
        }
        if let Some(tx) = whole.take() {
            tx.commit().unwrap();
            commits += 1;
        }
    }
    let cpu_ns = crate::clock::thread_cpu_ns().unwrap() - cpu;
    let wall_ns = started.elapsed().as_nanos();
    let segments_before: i64 = db
        .query_row("SELECT count(DISTINCT segid) FROM ftsk_idx", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM ftsk WHERE ftsk MATCH 'k7'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        FILES
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM ftsk_inst WHERE term='k7' AND offset=7",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        FILES
    );
    db.execute("DELETE FROM ftsk WHERE rowid=512", []).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM ftsk WHERE ftsk MATCH 'k7'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        FILES - 1
    );
    db.execute("INSERT INTO ftsk(rowid,k) VALUES(512,'changed')", [])
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM ftsk WHERE ftsk MATCH 'changed'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let incompatible = Connection::open_in_memory()
        .unwrap()
        .execute_batch(
            "CREATE VIRTUAL TABLE f USING fts5(k,content='',contentless_delete=1,columnsize=0)",
        )
        .unwrap_err()
        .to_string();
    assert!(incompatible.contains("incompatible"));
    let segments: i64 = db
        .query_row("SELECT count(DISTINCT segid) FROM ftsk_idx", [], |r| {
            r.get(0)
        })
        .unwrap();
    json!({"mode":mode,"trial":trial,"cpuNs":cpu_ns,"wallNs":wall_ns,"hostInsertStatements":insert_statements,"commits":commits,
        "segmentsAfterPublication":segments_before,"segmentsAfterVerificationEdits":segments,"positionalAndDeleteParity":true,"columnsizeZeroError":incompatible})
}
