//! Maintenance regressions for retained SQL layouts. The engine has no layout switches.
use super::{replay::SCHEMA, store::Store};
use rusqlite::Result;

#[test]
#[ignore = "requires diagnostic candidate database captures"]
fn replay_retained_layouts() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("RERAIL_PREPARATION_CAPTURE_DIR").expect("capture directory"),
    );
    let trial = std::env::var("RERAIL_PREPARATION_TRIAL")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .unwrap();
    super::replay::experiment(&dir, trial);
}

#[test]
fn retained_empty_results_and_reused_file_ids_follow_revision_cascades() {
    let store = Store::temporary().unwrap();
    let doc = super::tests::document(
        "target(value);",
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
    );
    let fid = store.put("a.ts", &doc).unwrap();
    store.db.execute_batch(SCHEMA).unwrap();
    store
        .db
        .execute(
            "INSERT INTO lab_ready SELECT 7,fid,revision FROM lab_revision WHERE fid=?1",
            [fid],
        )
        .unwrap();
    let revision: i64 = store
        .db
        .query_row(
            "SELECT revision FROM lab_revision WHERE fid=?1",
            [fid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM lab_ready", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    store.remove_known(fid).unwrap();
    let next = store.put("b.ts", &doc).unwrap();
    assert_eq!(fid, next, "exercise SQLite rowid reuse");
    assert!(
        store
            .db
            .query_row(
                "SELECT revision FROM lab_revision WHERE fid=?1",
                [next],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
            > revision
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM lab_ready", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn retained_keys_preserve_membership_and_both_preorder_range_bounds() {
    let store = Store::temporary().unwrap();
    let doc = super::tests::document(
        &"target(value);".repeat(512),
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
    );
    assert!(doc.transducer_len() > 2048);
    let fid = store.put("a.ts", &doc).unwrap();
    store.db.execute_batch(SCHEMA).unwrap();
    store
        .db
        .execute("INSERT INTO lab_request VALUES(?1)", [fid])
        .unwrap();
    store.db.execute("INSERT INTO lab_node SELECT 7,r.fid,r.revision,p.value FROM lab_revision r CROSS JOIN generate_series(0,2047,1) p",[]).unwrap();
    store.db.execute_batch("INSERT INTO lab_ready SELECT 7,fid,revision FROM lab_revision; CREATE TABLE bounds(fid INTEGER,lo INTEGER,hi INTEGER); INSERT INTO bounds SELECT fid,pre,pre+1 FROM lab_node; CREATE TEMP VIEW candidates AS SELECT n.fid,n.pre FROM lab_node n JOIN lab_ready r USING(relation_id,fid,revision) JOIN lab_request q ON q.fid=n.fid WHERE n.relation_id=7").unwrap();
    for (sql, range) in [
        (
            "SELECT s.fid,s.pre FROM bounds b CROSS JOIN candidates s WHERE s.fid=b.fid AND s.pre>=b.lo AND s.pre<b.hi",
            true,
        ),
        (
            "SELECT b.fid,b.lo FROM bounds b WHERE EXISTS(SELECT 1 FROM candidates s WHERE s.fid=b.fid AND s.pre=b.lo)",
            false,
        ),
    ] {
        let plans = store
            .db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert!(
            plans.iter().any(|p| if range {
                p.contains("pre>?") && p.contains("pre<?")
            } else {
                p.contains("pre=?")
            }),
            "lost ordinal constraint: {plans:?}"
        );
        let mut query = store.db.prepare(sql).unwrap();
        assert_eq!(
            query
                .query_map([], |r| r.get::<_, i64>(1))
                .unwrap()
                .collect::<Result<Vec<_>>>()
                .unwrap()
                .len(),
            2048
        );
        assert!(
            query.get_status(rusqlite::StatementStatus::VmStep) < 200_000,
            "retained lookup must not scan complete file postings"
        );
    }
}

#[test]
fn retained_output_limit_rolls_back_rows_without_publishing_an_empty_result() {
    let store = Store::temporary().unwrap();
    let doc = super::tests::document(
        "target(value);",
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
    );
    store.put("a.ts", &doc).unwrap();
    store.db.execute_batch(SCHEMA).unwrap();
    store.db.execute_batch("SAVEPOINT admitted").unwrap();
    let inserted=store.db.execute("INSERT INTO lab_node SELECT 7,r.fid,r.revision,p.value FROM lab_revision r JOIN file f ON f.fid=r.fid CROSS JOIN generate_series(0,f.nodes-1,1) p",[]).unwrap();
    assert!(inserted > 4);
    store
        .db
        .execute_batch("ROLLBACK TO admitted; RELEASE admitted")
        .unwrap();
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM lab_node", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM lab_ready", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let nodes = store
        .db
        .query_row("SELECT sum(nodes) FROM file", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert_eq!(
        nodes as usize,
        doc.transducer_len(),
        "the complete scratch result remains available"
    );
}

#[test]
fn one_hundred_edits_retire_facts_rows_and_predicates() {
    let store = Store::temporary().unwrap();
    store.db.execute_batch(SCHEMA).unwrap();
    let mut pages = Vec::new();
    for revision in 0..100 {
        let source = format!("target(value_{revision});");
        let doc = super::tests::document(
            &source,
            crate::code_rule_dsl::language::GrammarDialect::TypeScript,
        );
        let fid = store.put("a.ts", &doc).unwrap();
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM lab_ready", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        store
            .db
            .execute("DELETE FROM vocabulary_predicate", [])
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO vocabulary_predicate VALUES(1,?1,0)",
                [if revision % 2 == 0 {
                    "^target$"
                } else {
                    "^value_"
                }],
            )
            .unwrap();
        store
            .db
            .execute(include_str!("vocabulary-fill.sql"), [])
            .unwrap();
        store
            .db
            .execute_batch(
                "UPDATE vocabulary_predicate SET ready=1; DELETE FROM vocabulary_pending",
            )
            .unwrap();
        store.db.execute("INSERT INTO lab_node SELECT 7,r.fid,r.revision,p.value FROM lab_revision r JOIN file f ON f.fid=r.fid CROSS JOIN generate_series(0,f.nodes-1,1) p",[]).unwrap();
        store
            .db
            .execute(
                "INSERT INTO lab_ready SELECT 7,fid,revision FROM lab_revision WHERE fid=?1",
                [fid],
            )
            .unwrap();
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM lab_node", [], |r| r.get::<_, i64>(0))
                .unwrap() as usize,
            doc.transducer_len()
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM lab_ready", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        if revision % 10 == 9 {
            store
                .db
                .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")
                .unwrap();
            pages.push(
                store
                    .db
                    .query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
            );
        }
    }
    assert!(
        pages.last().unwrap() <= &(pages[4] + 32),
        "page growth after warmup: {pages:?}"
    );
}
