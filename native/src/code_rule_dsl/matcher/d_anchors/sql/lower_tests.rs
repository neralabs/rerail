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
fn finite_kind_alternatives_use_one_scoped_filter_before_negation() {
    let store = super::super::store::Store::temporary().unwrap();
    let doc = super::super::tests::document(
        "const first = target(42); // comment\nconst second = other;",
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
    );
    let fid = store.put("a.ts", &doc).unwrap();
    let kind = |code| AnchorPlan {
        root_kinds: Some([code].into()),
        condition: Condition::Always,
        exact: true,
    };
    // Real data-layer syntax exclusions contain nested finite kind alternatives.
    // Include duplicate arms and a utility; they must remain one predicate.
    let mut branches: Vec<_> = (1..=64).map(kind).collect();
    branches.push(AnchorPlan {
        root_kinds: Some((1..=32).collect()),
        condition: Condition::Any(vec![kind(1), kind(2), kind(1)]),
        exact: true,
    });
    branches.push(AnchorPlan {
        root_kinds: None,
        condition: Condition::Utility(std::sync::Arc::new(kind(3))),
        exact: true,
    });
    let alternatives = AnchorPlan {
        root_kinds: None,
        condition: Condition::Any(branches),
        exact: true,
    };
    for negative in [false, true] {
        let mut lower = compiler();
        let scope = lower
            .relation(
                format!(
                    "SELECT {fid},value FROM generate_series(0,{},1) WHERE value%2=0",
                    doc.transducer_len() - 1
                ),
                true,
            )
            .unwrap();
        // A decoder, a kind filter and (for negation) a subtraction suffice.
        lower.limit = lower.ctes.len() + 3;
        let plan = if negative {
            AnchorPlan {
                root_kinds: None,
                condition: Condition::Not(Box::new(alternatives.clone())),
                exact: true,
            }
        } else {
            alternatives.clone()
        };
        let selected = lower.plan(&plan, scope);
        assert!(
            selected.is_ok(),
            "finite kind alternatives must fit one filter, negative={negative}"
        );
        let selected = selected.unwrap();
        assert!(selected.exact && !lower.partial);
        let statement = finish_decoded(
            lower.ctes,
            vec![format!("SELECT 0,fid,pre FROM {}", selected.name)],
            lower.parameters,
            lower.indexed,
            lower.decoding,
        );
        let mut rows = Vec::new();
        super::super::execute::rows(&store, &statement, |_, _, pre| rows.push(pre)).unwrap();
        rows.sort_unstable();
        let expected: Vec<_> = (0..doc.transducer_len())
            .filter(|pre| {
                pre % 2 == 0
                    && ((1..=64).contains(&doc.transducer_kind(u32::try_from(*pre).unwrap()))
                        != negative)
            })
            .collect();
        assert_eq!(rows, expected);
    }
}

#[test]
fn exact_kind_boolean_filters_do_not_materialize_a_subtracted_copy() {
    let store = super::super::store::Store::temporary().unwrap();
    let doc = super::super::tests::document(
        "const first = target(42); // comment\nconst second = other;",
        crate::code_rule_dsl::language::GrammarDialect::TypeScript,
    );
    let fid = store.put("a.ts", &doc).unwrap();
    let kind = |code| AnchorPlan {
        root_kinds: Some([code].into()),
        condition: Condition::Always,
        exact: true,
    };
    // Real data-layer syntax exclusions contain nested finite kind alternatives.
    // Include duplicate arms and a utility; they must remain one predicate.
    let mut branches: Vec<_> = (1..=64).map(kind).collect();
    branches.push(AnchorPlan {
        root_kinds: Some((1..=32).collect()),
        condition: Condition::Any(vec![kind(1), kind(2), kind(1)]),
        exact: true,
    });
    branches.push(AnchorPlan {
        root_kinds: None,
        condition: Condition::Utility(std::sync::Arc::new(kind(3))),
        exact: true,
    });
    let alternatives = AnchorPlan {
        root_kinds: None,
        condition: Condition::Any(branches),
        exact: true,
    };
    for negative in [false, true] {
        let mut lower = compiler();
        let scope = lower
            .relation(
                format!(
                    "SELECT {fid},value FROM generate_series(0,{},1) WHERE value%2=0",
                    doc.transducer_len() - 1
                ),
                true,
            )
            .unwrap();
        // A decoder and one kind predicate suffice, including for negation.
        lower.limit = lower.ctes.len() + 2;
        let plan = if negative {
            AnchorPlan {
                root_kinds: None,
                condition: Condition::Not(Box::new(alternatives.clone())),
                exact: true,
            }
        } else {
            alternatives.clone()
        };
        let selected = lower.plan(&plan, scope);
        assert!(
            selected.is_ok(),
            "finite kind alternatives must fit one filter, negative={negative}"
        );
        let selected = selected.unwrap();
        assert!(selected.exact && !lower.partial);
        let statement = finish_decoded(
            lower.ctes,
            vec![format!("SELECT 0,fid,pre FROM {}", selected.name)],
            lower.parameters,
            lower.indexed,
            lower.decoding,
        );
        let mut rows = Vec::new();
        super::super::execute::rows(&store, &statement, |_, _, pre| rows.push(pre)).unwrap();
        rows.sort_unstable();
        let expected: Vec<_> = (0..doc.transducer_len())
            .filter(|pre| {
                pre % 2 == 0
                    && ((1..=64).contains(&doc.transducer_kind(u32::try_from(*pre).unwrap()))
                        != negative)
            })
            .collect();
        assert_eq!(rows, expected);
    }
}

#[test]
fn predicate_producers_run_once_before_join_fanout() {
    use rusqlite::functions::FunctionFlags;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let store = super::super::store::Store::temporary().unwrap();
    store
        .db
        .execute_batch(
            "CREATE TABLE points(fid INTEGER,pre INTEGER,PRIMARY KEY(fid,pre));
        INSERT INTO points SELECT 1,value FROM generate_series(0,1023,1);
        CREATE TABLE demands(fid INTEGER,pre INTEGER);
        INSERT INTO demands SELECT fid,pre FROM points CROSS JOIN generate_series(0,15,1)",
        )
        .unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    let observed = calls.clone();
    store
        .db
        .create_scalar_function(
            c"producer_predicate",
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            move |ctx| {
                observed.fetch_add(1, Ordering::Relaxed);
                Ok(ctx.get::<i64>(0)? % 2 == 0)
            },
        )
        .unwrap();
    let statement = finish_decoded(vec!["selected(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM points WHERE producer_predicate(pre))".into()],
        vec!["SELECT 0,s.fid,s.pre FROM demands d CROSS JOIN selected s ON s.fid=d.fid AND s.pre=d.pre".into()],
        vec![], BTreeSet::new(), HashMap::new());
    let mut rows = 0;
    super::super::execute::rows(&store, &statement, |_, _, _| rows += 1).unwrap();
    assert_eq!(rows, 512 * 16);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1024,
        "one relational reader can still repeat native work through fan-out"
    );
}

#[test]
fn single_reader_projection_pipelines_stream_without_private_row_copies() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let store = super::super::store::Store::temporary().unwrap();
    store.db.execute_batch("CREATE TABLE points(fid INTEGER,pre INTEGER); INSERT INTO points SELECT 1,value FROM generate_series(0,4095,1)").unwrap();
    let measure = |layers: usize| {
        let mut ctes = vec!["p0(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM points)".into()];
        for i in 1..=layers {
            ctes.push(format!(
                "p{i}(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM p{})",
                i - 1
            ));
        }
        let statement = finish_decoded(
            ctes,
            vec![format!("SELECT 0,fid,pre FROM p{layers}")],
            vec![],
            BTreeSet::new(),
            HashMap::new(),
        );
        let ticks = Arc::new(AtomicU64::new(0));
        let observed = ticks.clone();
        store
            .db
            .progress_handler(
                100,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        let mut rows = Vec::new();
        super::super::execute::rows(&store, &statement, |_, fid, pre| rows.push((fid, pre)))
            .unwrap();
        rows.sort_unstable();
        assert_eq!(rows, (0..4096).map(|pre| (1, pre)).collect::<Vec<_>>());
        ticks.load(Ordering::Relaxed)
    };
    let direct = measure(0);
    let pipeline = measure(16);
    assert!(
        pipeline <= direct * 2,
        "a projection pipeline must not copy its rows at every step: {direct} -> {pipeline}"
    );
}

#[test]
fn physical_ownership_preserves_streaming_projection_boundaries() {
    let store = super::super::store::Store::temporary().unwrap();
    store.db.execute_batch("CREATE TABLE points(fid INTEGER,pre INTEGER); INSERT INTO points SELECT 1,value FROM generate_series(0,4095,1)").unwrap();
    let measure = |layers: usize| {
        let mut ctes = vec!["p0(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM points)".into()];
        for i in 1..=layers {
            ctes.push(format!(
                "p{i}(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM p{})",
                i - 1
            ));
        }
        let mut statement = finish_decoded(
            ctes,
            vec![format!("SELECT 0,fid,pre FROM p{layers}")],
            vec![],
            BTreeSet::from([format!("p{layers}")]),
            HashMap::new(),
        );
        // Production rebuilds physical statements from the dependency graph.
        statement.own();
        let mut rows = 0;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        let ticks = Arc::new(AtomicU64::new(0));
        let observed = ticks.clone();
        store
            .db
            .progress_handler(
                100,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        super::super::execute::rows(&store, &statement, |_, _, _| rows += 1).unwrap();
        assert_eq!(rows, 4096);
        ticks.load(Ordering::Relaxed)
    };
    let direct = measure(0);
    let pipeline = measure(16);
    assert!(
        pipeline <= direct * 2,
        "physical ownership must retain streaming aliases: {direct} -> {pipeline}"
    );
}

#[test]
fn alternatives_do_not_repeatedly_materialize_a_growing_union() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let measure = |count: u16| {
        let store = super::super::store::Store::temporary().unwrap();
        store
            .db
            .execute_batch(&format!(
                "CREATE TABLE points(fid INTEGER,pre INTEGER,bucket INTEGER);
            INSERT INTO points SELECT 1,value,value/64 FROM generate_series(0,{},1);
            CREATE INDEX point_bucket ON points(bucket,fid,pre)",
                u32::from(count) * 64 - 1
            ))
            .unwrap();
        let mut lower = compiler();
        let mut alternatives = Vec::new();
        for bucket in 0..count {
            // Bind a narrow kind relation to a deterministic indexed fixture.
            let selected = lower
                .relation(
                    format!("SELECT fid,pre FROM points WHERE bucket={bucket}"),
                    true,
                )
                .unwrap();
            alternatives.push(selected);
        }
        // Repeated alternatives retain set semantics.
        alternatives.push(alternatives[0].clone());
        // Exercise general set union independently of finite kind normalization.
        let selected = lower.union(&alternatives).unwrap();
        let statement = finish_decoded(
            lower.ctes,
            vec![format!(
                "SELECT 0,fid,pre FROM {} ORDER BY fid,pre",
                selected.name
            )],
            lower.parameters,
            lower.indexed,
            lower.decoding,
        );
        let ticks = Arc::new(AtomicU64::new(0));
        let observed = ticks.clone();
        store
            .db
            .progress_handler(
                100,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        let mut rows = Vec::new();
        super::super::execute::rows(&store, &statement, |_, fid, pre| rows.push((fid, pre)))
            .unwrap();
        assert_eq!(
            rows,
            (0..usize::from(count) * 64)
                .map(|pre| (1, pre))
                .collect::<Vec<_>>()
        );
        ticks.load(Ordering::Relaxed)
    };
    let small = measure(16);
    let large = measure(32);
    assert!(
        large <= small * 3,
        "doubling alternative output must not quadruple SQL work: {small} -> {large}"
    );
}

#[test]
fn failed_literal_admission_restores_the_previous_kind_domain() {
    use super::super::super::{Required, Requirement};
    for previous in [None, Some(BTreeSet::from([25]))] {
        let mut lower = compiler();
        if let Some(previous) = &previous {
            lower
                .reuse
                .kinds
                .insert("all_nodes".into(), previous.clone());
        }
        lower.limit = 0;
        assert!(
            lower
                .pattern(
                    &[Requirement {
                        value: Required::Terminal(17, "target".into()),
                        parents: vec![]
                    }],
                    lower.all()
                )
                .is_err()
        );
        assert_eq!(
            lower.reuse.kinds.get("all_nodes"),
            previous.as_ref(),
            "failed admission must not restrict a later consumer"
        );
    }
}

#[test]
fn an_unrestricted_alternative_gates_other_producers_before_admission() {
    use super::super::super::{Required, Requirement};
    let mut lower = compiler();
    let scope = lower.relation("SELECT 1,2".into(), true).unwrap();
    lower.reuse.kinds.insert(scope.name.clone(), [17].into());
    // No additional producer can fit. The optional capture arm already covers
    // this entire kind domain, so other arms must never enter preparation.
    lower.limit = lower.ctes.len();
    let plan = AnchorPlan {
        root_kinds: None,
        exact: false,
        condition: Condition::Any(vec![
            AnchorPlan {
                root_kinds: None,
                exact: false,
                condition: Condition::Pattern(vec![Requirement {
                    value: Required::Terminal(17, "unrequested".into()),
                    parents: vec![None; 32],
                }]),
            },
            AnchorPlan {
                root_kinds: Some([17].into()),
                condition: Condition::Always,
                exact: false,
            },
        ]),
    };
    let selected = lower.plan(&plan, scope.clone()).unwrap();
    assert_eq!(selected.name, scope.name);
    assert!(
        !selected.exact,
        "a broad capture prefilter cannot justify subtraction"
    );
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
fn impossible_kind_branch_does_not_prepare_its_structural_or_source_producers() {
    let mut lower = compiler();
    let scope = lower.kinds(&BTreeSet::from([1])).unwrap();
    let before = lower.ctes.len();
    let branch = AnchorPlan {
        root_kinds: Some(BTreeSet::from([2])),
        exact: false,
        condition: Condition::Relation {
            inner: Box::new(AnchorPlan {
                root_kinds: Some(BTreeSet::from([3])),
                exact: true,
                condition: Condition::Text {
                    regex: regex::Regex::new("expensive_source_predicate").unwrap(),
                    probe: crate::code_rule_dsl::matcher::d_anchors::text::TextProbe::compile(
                        "expensive_source_predicate",
                    ),
                },
            }),
            direction: Direction::Has,
            recursive: true,
            field: None,
        },
    };
    let selected = lower.plan(&branch, scope).unwrap();
    assert!(
        lower.ctes.len() <= before + 1,
        "an impossible kind branch constructs {} extra relations",
        lower.ctes.len() - before
    );
    assert!(
        lower.decoding.is_empty(),
        "an empty branch still requests decoded properties"
    );
    assert!(
        lower.source_predicates.is_empty(),
        "an empty branch still requests source confirmation"
    );
    let statement = finish_decoded(
        lower.ctes,
        vec![format!("SELECT 0,fid,pre FROM {}", selected.name)],
        lower.parameters,
        lower.indexed,
        lower.decoding,
    );
    assert!(
        statement
            .stages
            .iter()
            .all(|s| !s.sql.contains("nera_nodes"))
    );
}

#[test]
fn candidate_transport_does_not_sort_each_sql_union_arm() {
    use crate::code_rule_dsl::language::{self, GrammarDialect};
    let store = super::super::store::Store::temporary().unwrap();
    let lang = GrammarDialect::TypeScript;
    let fid = store
        .put(
            "a.ts",
            &super::super::tests::document(&"target(value);\n".repeat(2000), lang),
        )
        .unwrap();
    let plans = ["identifier", "call_expression"].map(|kind| AnchorPlan {
        root_kinds: Some(BTreeSet::from([lang.kinds().kind_id(kind)])),
        exact: true,
        condition: Condition::Always,
    });
    let statement = batch(
        &store,
        &plans.iter().collect::<Vec<_>>(),
        u32::from(language::tape_tag(lang).unwrap()),
        Some(fid),
        None,
    );
    assert!(
        statement.stages.is_empty(),
        "single-reader kind streams need no prepared relations"
    );
    super::super::execute::request(&store, &[fid]).unwrap();
    let mut rows = 0;
    let mut sorts = 0;
    for sql in &statement.queries {
        let mut q = store.db.prepare(sql).unwrap();
        let n = q.parameter_count();
        let mut cursor = q
            .query(rusqlite::params_from_iter(&statement.parameters[..n]))
            .unwrap();
        while cursor.next().unwrap().is_some() {
            rows += 1;
        }
        drop(cursor);
        sorts += q.get_status(rusqlite::StatementStatus::Sort);
    }
    assert_eq!(rows, 6000);
    assert_eq!(sorts, 0, "candidate transport sorts its result arms");
}
