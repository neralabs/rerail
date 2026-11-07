//! Native corpus coverage for the source-confirmation relation.
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::d_anchors::sql::{store::Store, tests::document},
};
use rusqlite::Result;

#[test]
fn typed_frames_match_sql_without_inflating_distinct_consumer_scopes() -> Result<()> {
    let store = Store::temporary()?;
    store.db.execute_batch(
        "INSERT INTO source_batch_definition VALUES(1,'target',1),(2,'é|other',0)",
    )?;
    let mut input = Vec::new();
    for (producer, path, text) in [
        (1, "a.ts", "target('é'); other();"),
        (2, "b.ts", "other('é'); target();"),
    ] {
        let doc = document(text, GrammarDialect::TypeScript);
        let fid = store.put(path, &doc)?;
        for pre in (0..doc.transducer_len()).filter(|pre| pre % 2 == 0) {
            input.push([producer, fid, pre as i64].map(rusqlite::types::Value::Integer));
        }
    }
    super::store::publication::insert(&store.db, "INSERT INTO source_batch_demand", "", input)?;
    store.db.execute(
        "UPDATE source_batch_stats SET n=(SELECT count(*) FROM source_batch_demand)",
        [],
    )?;
    let collect = |sql: &str| -> Result<Vec<(i64, i64, i64)>> {
        store
            .db
            .prepare(sql)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
    };
    let expected = collect(
        "SELECT d.producer,d.fid,d.pre FROM source_batch_demand d JOIN source_batch_definition p USING(producer) WHERE CASE WHEN p.literal THEN nera_equal(p.expression,d.fid,d.pre) ELSE nera_matches(p.expression,d.fid,d.pre) END ORDER BY 1,2,3",
    )?;
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    let actual = collect("SELECT producer,fid,pre FROM nera_source_batch(1) ORDER BY 1,2,3")?;
    assert_eq!(actual, expected);
    assert!(!actual.is_empty());
    #[cfg(feature = "code-rule-profile")]
    {
        let counters = store.readers.0.lock().counters.since(before);
        assert_eq!(counters.source_batches, 1);
        let expected: i64 = store
            .db
            .query_row("SELECT n FROM source_batch_stats", [], |r| r.get(0))?;
        assert_eq!(counters.source_input_rows, expected as u64);
        assert_eq!(counters.source_output_rows, actual.len() as u64);
    }
    Ok(())
}

#[test]
fn an_empty_frame_does_not_enter_the_native_producer() -> Result<()> {
    let store = Store::temporary()?;
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    let rows:i64=store.db.query_row("SELECT count(*) FROM source_batch_stats stats CROSS JOIN nera_source_batch(1) p WHERE stats.n>0",[],|r|r.get(0))?;
    assert_eq!(rows, 0);
    #[cfg(feature = "code-rule-profile")]
    assert_eq!(
        store.readers.0.lock().counters.since(before).source_batches,
        0
    );
    Ok(())
}
