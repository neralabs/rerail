use super::*;

#[test]
fn scheduled_execution_releases_all_tables_after_success_and_a_later_packet_error() {
    let store = super::super::store::Store::temporary().unwrap();
    let mut statement = finish_decoded(
        vec![
            "early(fid,pre) AS MATERIALIZED (VALUES(1,2))".into(),
            "late(fid,pre) AS MATERIALIZED (VALUES(1,3))".into(),
        ],
        vec![
            "SELECT 0,fid,pre FROM early".into(),
            "SELECT 1,fid,pre FROM late".into(),
        ],
        vec![],
        BTreeSet::from(["early".into(), "late".into()]),
        HashMap::new(),
    );
    let mut rows = Vec::new();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 2), (1, 1, 3)]);
    statement.queries[1] = "SELECT 1,fid,missing_column FROM late".into();
    statement.reschedule();
    rows.clear();
    assert!(
        super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).is_err()
    );
    assert_eq!(rows, [(0, 1, 2)]);
    assert_eq!(store.db.query_row("SELECT count(*) FROM temp.sqlite_schema WHERE name IN ('early','late','early_keys','late_keys')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    statement.queries[1] = "SELECT 1,fid,pre FROM late".into();
    statement.reschedule();
    rows.clear();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 2), (1, 1, 3)]);
}

#[test]
fn a_required_universal_property_fill_covers_independent_subset_demands() {
    let store = super::super::store::Store::temporary().unwrap();
    let lang = crate::code_rule_dsl::language::GrammarDialect::TypeScript;
    let doc = super::super::tests::document("target(value);", lang);
    let fid = store.put("a.ts", &doc).unwrap();
    let mut lower = compiler();
    lower
        .ctes
        .push(format!("cf(fid) AS MATERIALIZED (SELECT {fid})"));
    lower.ctes.push("all_nodes(fid,pre) AS NOT MATERIALIZED (SELECT f.fid,p.value FROM cf JOIN file f ON f.fid=cf.fid CROSS JOIN generate_series(0,f.nodes-1,1) p)".into());
    let subset = lower
        .relation(format!("SELECT {fid},0 UNION SELECT {fid},1"), true)
        .unwrap();
    let a = lower.decoded(&subset, Property::Node);
    let all = lower.decoded(&lower.all(), Property::Node);
    let statement = finish_decoded(
        lower.ctes,
        vec![format!(
            "SELECT 0 AS qid,fid,pre FROM {a} UNION ALL SELECT 1,fid,pre FROM {all} ORDER BY qid,fid,pre"
        )],
        lower.parameters,
        lower.indexed,
        lower.decoding,
    );
    assert_eq!(
        statement
            .stages
            .iter()
            .filter(|stage| stage.name == "decode_all_nodes")
            .count(),
        1,
        "one covering fill, regardless of declaration order"
    );
    assert!(
        !statement
            .stages
            .iter()
            .any(|s| s.temporary && s.name == "all_nodes"),
        "the retained structural key already indexes every node"
    );
    assert!(
        statement
            .queries
            .iter()
            .all(|sql| !sql.contains("nera_nodes(")),
        "decoding belongs to preparation"
    );
    assert!(
        !statement
            .stages
            .iter()
            .any(|stage| stage.temporary && stage.name == subset.name),
        "a covered subset needs no materialization for decoding"
    );
    let mut rows = Vec::new();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows.len(), doc.transducer_len() + 2);
    #[cfg(feature = "code-rule-profile")]
    assert_eq!(
        store.readers.0.lock().counters.decoded_nodes,
        doc.transducer_len() as u64
    );
}

#[test]
fn typed_candidate_keys_keep_both_interval_index_bounds() {
    let store = super::super::store::Store::temporary().unwrap();
    store.db.execute_batch("CREATE TABLE input(fid,pre); INSERT INTO input WITH RECURSIVE n(pre) AS (SELECT 0 UNION ALL SELECT pre+1 FROM n WHERE pre<2047) SELECT 1,pre FROM n; CREATE TABLE bounds(fid INTEGER,lo INTEGER,hi INTEGER); INSERT INTO bounds SELECT fid,pre,pre+1 FROM input;").unwrap();
    let statement=finish_decoded(vec!["candidate(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM input)".into()],vec!["SELECT 0 AS qid,s.fid,s.pre FROM bounds r CROSS JOIN candidate s WHERE s.fid=r.fid AND s.pre>=r.lo AND s.pre<r.hi".into()],Vec::new(),BTreeSet::from(["candidate".into()]),HashMap::new());
    for stage in &statement.stages {
        store.db.execute_batch(&stage.sql).unwrap();
        if let Some(keys) = stage.index {
            store
                .db
                .execute_batch(&format!(
                    "CREATE INDEX {}_keys ON {}({keys})",
                    stage.name, stage.name
                ))
                .unwrap();
        }
    }
    let mut explain = store
        .db
        .prepare(&format!("EXPLAIN QUERY PLAN {}", statement.queries[0]))
        .unwrap();
    let plans = explain
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plans
            .iter()
            .any(|p| p.contains("pre>?") && p.contains("pre<?")),
        "lost a range bound: {plans:?}"
    );
    let mut query = store.db.prepare(&statement.queries[0]).unwrap();
    let count = query
        .query_map([], |_| Ok(()))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .len();
    assert_eq!(count, 2048);
    assert!(
        query.get_status(rusqlite::StatementStatus::VmStep) < 200_000,
        "interval lookup must stay linear"
    );
}

#[test]
fn overlapping_property_demands_decode_once_and_survive_batch_boundaries() {
    let store = super::super::store::Store::temporary().unwrap();
    let lang = crate::code_rule_dsl::language::GrammarDialect::TypeScript;
    let doc = super::super::tests::document("target(value);", lang);
    let fid = store.put("a.ts", &doc).unwrap();
    let mut lower = compiler();
    let a = lower
        .relation(format!("SELECT {fid},0 UNION SELECT {fid},1"), true)
        .unwrap();
    let b = lower
        .relation(format!("SELECT {fid},1 UNION SELECT {fid},2"), true)
        .unwrap();
    let pa = lower.decoded(&a, Property::Node);
    let pb = lower.decoded(&b, Property::Node);
    let sa = lower.decoded(&a, Property::Span);
    let sb = lower.decoded(&b, Property::Span);
    let statement = finish_decoded(
        lower.ctes,
        vec![format!(
            "SELECT 0 AS qid,fid,pre FROM {pa} WHERE parent>=-1 AND end>pre AND kind>=0 AND field>=0 AND flags>=0 UNION ALL SELECT 1,fid,pre FROM {pb} UNION ALL SELECT 2,fid,pre FROM {sa} WHERE start>=0 AND len>=0 UNION ALL SELECT 3,fid,pre FROM {sb} ORDER BY qid,fid,pre"
        )],
        lower.parameters,
        lower.indexed,
        lower.decoding,
    );
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    let mut rows = Vec::new();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows.len(), 8);
    #[cfg(feature = "code-rule-profile")]
    {
        let reads = store.readers.0.lock().counters.since(before);
        assert_eq!(
            reads.decoded_nodes, 3,
            "one decoded record per distinct node"
        );
        assert_eq!(reads.decoded_spans, 3, "one decoded span per distinct node");
        assert!(
            reads.reads[0] <= 3 && reads.reads[4] <= 3,
            "block reads must not exceed node demands"
        );
        assert_eq!(
            reads.reads[6], 0,
            "structural and span properties retain no source text"
        );
    }
    #[cfg(feature = "code-rule-profile")]
    let before = store.readers.0.lock().counters;
    super::super::execute::rows(&store, &statement, |_, _, _| {}).unwrap();
    #[cfg(feature = "code-rule-profile")]
    assert_eq!(
        store.readers.0.lock().counters.since(before).reads,
        [0; 7],
        "SQL properties survive the next batch"
    );
    store
        .put(
            "a.ts",
            &super::super::tests::document("updated(other);", lang),
        )
        .unwrap();
    for table in ["decoded_properties", "decoded_spans", "decoded_children"] {
        assert_eq!(
            store
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "replacement invalidates every shared property"
        );
    }
}

fn compiler() -> Lower {
    Lower::new(1)
}

#[test]
fn demanded_span_keys_remain_a_boundary_before_source_predicates() {
    use rusqlite::functions::FunctionFlags;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let store = super::super::store::Store::temporary().unwrap();
    let doc = super::super::tests::document(
        &"target(value);".repeat(300),
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
    );
    let fid = store.put("a.ts", &doc).unwrap();
    store.db.execute_batch(&format!("WITH spans AS MATERIALIZED (SELECT value AS pre,nera_span({fid},value) AS packed FROM generate_series(0,{})) INSERT INTO decoded_spans SELECT {fid},pre,nera_word(packed,0),nera_word(packed,1) FROM spans",doc.transducer_len()-1)).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    store
        .db
        .create_scalar_function(c"expensive", 2, FunctionFlags::SQLITE_UTF8, move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        })
        .unwrap();
    let mut lower = compiler();
    let scope = lower.relation(format!("SELECT {fid},1"), true).unwrap();
    let spans = lower.decoded(&scope, Property::Span);
    let statement = finish_decoded(
        lower.ctes,
        vec![format!(
            "SELECT 0,fid,pre FROM {spans} WHERE expensive(start,len)"
        )],
        lower.parameters,
        lower.indexed,
        lower.decoding,
    );
    let mut rows = Vec::new();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, fid, 1)]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(
        statement.queries[0].contains(&format!("{spans}(fid,pre,start,len) AS MATERIALIZED")),
        "expensive consumers must stay outside the selected span relation"
    );
}

#[test]
fn repeated_kind_restriction_is_idempotent_even_through_intermediate_sets() {
    let mut lower = compiler();
    let a = lower.relation("SELECT 1,1".into(), true).unwrap();
    let b = lower
        .relation("SELECT 1,1 UNION SELECT 2,2".into(), true)
        .unwrap();
    let c = lower
        .relation("SELECT 1,1 UNION SELECT 3,3".into(), true)
        .unwrap();
    let ab = lower.combine(&a, &b, "INTERSECT", true).unwrap();
    let abc = lower.combine(&ab, &c, "INTERSECT", true).unwrap();
    let before = lower.ctes.len();
    let again = lower.combine(&abc, &b, "INTERSECT", false).unwrap();
    assert_eq!(again.name, abc.name);
    assert!(
        !again.exact,
        "set reuse must preserve conservative exactness"
    );
    assert_eq!(
        lower.ctes.len(),
        before,
        "an implied restriction adds no sort or materialization"
    );
}

#[test]
fn difference_from_itself_is_empty_and_remains_sound_under_negation() {
    let mut lower = compiler();
    let a = lower
        .relation("SELECT 1,1 UNION SELECT 1,2".into(), true)
        .unwrap();
    let empty = lower.combine(&a, &a, "EXCEPT", true).unwrap();
    let all = lower.combine(&a, &empty, "EXCEPT", true).unwrap();
    let statement = finish_decoded(
        lower.ctes,
        vec![format!(
            "SELECT 0 AS qid,fid,pre FROM {} UNION ALL SELECT 1 AS qid,fid,pre FROM {} ORDER BY qid,fid,pre",
            empty.name, all.name
        )],
        lower.parameters,
        lower.indexed,
        lower.decoding,
    );
    let store = super::super::store::Store::temporary().unwrap();
    let mut rows = Vec::new();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(1, 1, 1), (1, 1, 2)]);
}

#[test]
fn identical_generated_relations_share_storage_without_changing_exactness() {
    let mut lower = compiler();
    let a = lower.relation("SELECT 1,1".into(), true).unwrap();
    let b = lower.relation("SELECT 1,1".into(), false).unwrap();
    assert_eq!(a.name, b.name);
    assert!(!b.exact);
    assert_eq!(lower.ctes.len(), 1);
}

#[test]
fn empty_union_still_deduplicates_a_projected_input() {
    let mut lower = compiler();
    let a = lower
        .relation("SELECT 1,1 UNION ALL SELECT 1,1".into(), true)
        .unwrap();
    let empty = lower.relation("SELECT 0,0 WHERE 0".into(), true).unwrap();
    let unique = lower.combine(&empty, &a, "UNION", true).unwrap();
    let statement = finish_decoded(
        lower.ctes,
        vec![format!("SELECT 0,fid,pre FROM {}", unique.name)],
        lower.parameters,
        lower.indexed,
        lower.decoding,
    );
    let store = super::super::store::Store::temporary().unwrap();
    let mut rows = Vec::new();
    super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p))).unwrap();
    assert_eq!(rows, [(0, 1, 1)]);
}

#[test]
fn shared_candidate_owner_cleans_up_after_a_query_error_and_retries_with_fresh_completion() {
    let store = super::super::store::Store::temporary().unwrap();
    for fail in [true, false, false] {
        let mut statement = finish_decoded(
            vec![
                "a(fid,pre) AS MATERIALIZED (VALUES(1,2))".into(),
                "b(fid,pre) AS MATERIALIZED (SELECT 1,3 WHERE 0)".into(),
            ],
            vec![
                "SELECT 0,fid,pre FROM a".into(),
                if fail {
                    "SELECT 1,fid,missing FROM b".into()
                } else {
                    "SELECT 1,fid,pre FROM b".into()
                },
            ],
            vec![],
            BTreeSet::from(["a".into(), "b".into()]),
            HashMap::new(),
        );
        statement.own();
        let mut rows = Vec::new();
        let result =
            super::super::execute::rows(&store, &statement, |q, f, p| rows.push((q, f, p)));
        assert_eq!(result.is_err(), fail);
        assert_eq!(rows, [(0, 1, 2)]);
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT count(*) FROM temp.sqlite_schema WHERE name LIKE 'candidate_keys%'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn a_union_of_restrictions_is_one_compound_set_within_their_scope() {
    let mut lower = compiler();
    let scope = lower
        .relation("SELECT 1,1 UNION SELECT 1,2".into(), true)
        .unwrap();
    let a = lower.relation("SELECT 1,1".into(), true).unwrap();
    let b = lower.relation("SELECT 1,2".into(), true).unwrap();
    let a = lower.combine(&scope, &a, "INTERSECT", true).unwrap();
    let b = lower.combine(&scope, &b, "INTERSECT", true).unwrap();
    let empty = lower.relation("SELECT 0,0 WHERE 0".into(), true).unwrap();
    let before = lower.ctes.len();
    let union = lower.union(vec![a.clone(), empty, b.clone(), a]).unwrap();
    assert_eq!(
        lower.ctes.len(),
        before + 1,
        "one compound UNION, without the empty seed"
    );
    let again = lower.combine(&union, &scope, "INTERSECT", true).unwrap();
    assert_eq!(
        again.name, union.name,
        "a union of subsets is within their scope"
    );
    assert_eq!(lower.ctes.len(), before + 1);
    let other = lower.relation("SELECT 2,2".into(), true).unwrap();
    let narrowed = lower.combine(&union, &other, "INTERSECT", true).unwrap();
    assert_ne!(
        narrowed.name, union.name,
        "containment is not membership in every set"
    );
}
