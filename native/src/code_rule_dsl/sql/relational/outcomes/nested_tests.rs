use super::super::tests::measure;
use super::reached_tests::reached;
use super::tests::policy;
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::{Value, json};

#[inline(never)]
fn parity(rule: Value, sources: &[&str]) -> Value {
    let p = policy(rule, json!({}));
    measure(
        &grammar_matchers(&p)[0],
        &sources
            .iter()
            .enumerate()
            .map(|(i, s)| (format!("{i}.ts"), (*s).into()))
            .collect::<Vec<_>>(),
    )
}
#[inline(never)]
fn inner() -> Value {
    json!({"kind":"number","pattern":"$X","value":{"eq":1},"stopBy":"end"})
}
#[inline(never)]
fn array() -> Value {
    json!({"kind":"array","pattern":"$A","has":inner(),"stopBy":"neighbor"})
}
#[inline(never)]
fn rule() -> Value {
    json!({"kind":"arguments","has":array()})
}

#[test]
fn nested_walks_return_distinct_outer_and_inner_witnesses() {
    let report = parity(rule(), &["f([1],[2]); g([0],[1]);", "h([0]); i([],[1]);"]);
    assert_eq!(report["matches"], 3);
    for file in report["output"].as_array().unwrap() {
        for row in file.as_array().unwrap() {
            assert_ne!(row[1][0], row[1][1]);
            assert!(row[1][0].as_u64().unwrap() < row[1][1].as_u64().unwrap());
        }
    }
    assert!(report["typedReturnRows"].as_u64().unwrap() > 0);
    assert_eq!(report["matchingStatements"], 1);
    assert_eq!(report["framePublications"], 0);
}

#[test]
fn nested_walks_preserve_atomic_call_order_and_decided_tails() {
    reached(rule(), "f([1],[2]);", Some("2"), &["1"], false);
    reached(rule(), "f([0],[1],[2]);", Some("2"), &["0", "1"], false);
    reached(rule(), "f([0],[2]);", Some("2"), &["0", "2"], true);
    // A prefix value must not run again when a correlated CTE has several consumers.
    let body = json!({"kind":"array","pattern":"$A","all":[{"value":{"constant":true}},{"has":inner()}],"stopBy":"neighbor"});
    reached(
        json!({"kind":"arguments","has":body}),
        "f([1],[2]);",
        Some("2"),
        &["[1]", "1"],
        false,
    );
    // The same hazard exists inside the compact node-local Boolean compiler.
    reached(
        json!({"kind":"arguments","has":{"kind":"number","all":[{"value":{"eq":1}},{"any":[{"pattern":"$X"},{"regex":"^never$"}]}]}}),
        "f(1,2);",
        Some("2"),
        &["1"],
        false,
    );
}

#[test]
fn nested_walks_preserve_unknowns_and_pending_stop_boundaries() {
    let mut body = array();
    body["has"] = json!({"any":[{"kind":"identifier"},{"kind":"number"}],"pattern":"$X","value":{"eq":1},"stopBy":"end"});
    let grammar = json!({"kind":"arguments","has":body});
    let report = parity(grammar.clone(), &["f([x],[1]); g([1],[x]);"]);
    assert_eq!(report["matches"], 1);
    reached(grammar, "f([x],[2]);", Some("2"), &["x"], false);
    let stop = json!({"regex":"^\\[0\\];$","testUnknown":".*"});
    let grammar = json!({"kind":"statement_block","has":{"kind":"expression_statement","pattern":"$A","has":inner(),"stopBy":stop}});
    let report = parity(
        grammar.clone(),
        &["{ [0]; [1]; [2]; }", "{ [0]; [3]; [1]; }"],
    );
    assert_eq!(report["matches"], 1);
    reached(grammar, "{ [0]; [3]; [1]; }", Some("1"), &["0", "3"], false);
    let stop = json!({"any":[{"regex":"^\\[0\\];$","testUnknown":".*"},{"kind":"expression_statement","value":{"constant":true}}]});
    reached(
        json!({"kind":"statement_block","has":{"kind":"expression_statement","pattern":"$A","has":inner(),"stopBy":stop}}),
        "{ [0]; [3]; [1]; }",
        Some("[3];"),
        &["0", "[0];", "3"],
        false,
    );
}

#[test]
fn nested_boolean_capture_results_restore_and_withhold() {
    for body in [
        json!({"kind":"array","any":[{"all":[{"has":inner()},{"regex":"^never$"}]},{"pattern":"$Y"}],"stopBy":"neighbor"}),
        json!({"kind":"array","all":[{"not":{"has":inner()}},{"pattern":"$Y"}],"stopBy":"neighbor"}),
        json!({"kind":"array","all":[{"has":{"kind":"identifier","pattern":"$X","testUnknown":".*","stopBy":"end"}},{"has":{"kind":"identifier","pattern":"$X","value":{"eq":1},"stopBy":"end"}}],"stopBy":"neighbor"}),
    ] {
        parity(
            json!({"kind":"arguments","has":body}),
            &["f([0],[1]); g([x],[1]); h([],[2]);"],
        );
    }
    let report = parity(
        json!({"kind":"arguments","all":[{"has":{"kind":"number","pattern":"$X","value":{"constant":true}}},{"has":array()}]}),
        &["f(1,[1]); g(2,[1]);"],
    );
    assert_eq!(report["matches"], 1);
}

#[test]
fn nested_counts_isolate_probe_captures_and_keep_exact_exports() {
    let exact = json!({"kind":"arguments","count":{"has":array(),"min":0,"max":10,"as":"N"}});
    let report = parity(exact.clone(), &["f([1],[1],[0]); g([]);"]);
    assert_eq!(report["output"][0][0][1], json!(["2"]));
    reached(exact, "f([1],[2]);", Some("2"), &["1", "2"], true);
    reached(
        json!({"kind":"arguments","count":{"has":array(),"min":1}}),
        "f([1],[2]);",
        Some("2"),
        &["1"],
        false,
    );
    let body = json!({"kind":"array","pattern":"$A","count":{"has":{"kind":"number","value":{"constant":true},"stopBy":"end"},"min":1,"as":"N"},"stopBy":"neighbor"});
    let report = parity(
        json!({"kind":"arguments","has":body}),
        &["f([1,2],[3]); g([],[]);"],
    );
    assert_eq!(report["matches"], 1);
    assert_eq!(report["output"][0][0][1][1], "2");
}

#[test]
fn nested_stops_are_private_and_preserve_axis_order() {
    let stop =
        json!({"has":{"kind":"number","pattern":"$PRIVATE","value":{"eq":1},"stopBy":"end"}});
    reached(
        json!({"kind":"arguments","has":{"kind":"array","pattern":"$A","testUnknown":"^never$","stopBy":stop}}),
        "f([1],[2]);",
        Some("1"),
        &[],
        false,
    );
    reached(
        json!({"kind":"array","regex":"^\\[0\\]$","precedes":{"kind":"array","pattern":"$A","has":inner(),"stopBy":stop}}),
        "f([0],[1],[2]);",
        Some("1"),
        &["1"],
        true,
    );
    for axis in ["inside", "follows", "precedes"] {
        parity(
            json!({"kind":"number",axis:{"kind":"array","pattern":"$A","has":inner(),"stopBy":{"regex":"^never$"}}}),
            &["f([1,0],[2]); g([[1]]);"],
        );
    }
}

#[test]
fn nested_depth_is_bounded_before_sql_execution() {
    let mut body = inner();
    for depth in 1..=4 {
        body = json!({"kind":"array","has":body,"stopBy":"neighbor"});
        let grammar = json!({"kind":"arguments","has":body});
        if depth < 4 {
            let source = format!("f({}1{});", "[".repeat(depth), "]".repeat(depth));
            parity(grammar, &[&source]);
        } else {
            let p = policy(grammar, json!({}));
            let Err(error) = super::super::compile(&grammar_matchers(&p)[0]) else {
                panic!("accepted five traversal levels");
            };
            assert!(error.contains("depth exceeds 4"), "{error}");
        }
    }
}

#[test]
fn nested_walks_scale_with_reached_inner_outer_and_root_work() {
    let mut samples = vec![];
    for dimension in [
        "known-first-inner-width",
        "late-inner-match",
        "late-outer-match",
        "independent-roots",
    ] {
        for size in [16, 64, 256] {
            let source = match dimension {
                "known-first-inner-width" => {
                    format!("f([1,{}],[2]);", vec!["2"; size - 1].join(","))
                }
                "late-inner-match" => format!("f([{},1],[2]);", vec!["0"; size - 1].join(",")),
                "late-outer-match" => format!("f({},[1],[2]);", vec!["[0]"; size - 1].join(",")),
                _ => "f([1],[2]);".repeat(size),
            };
            let mut report = parity(rule(), &[&source]);
            assert_eq!(
                report["valueRequests"],
                if dimension == "known-first-inner-width" {
                    1
                } else {
                    size
                }
            );
            assert_eq!(report["matchingStatements"], 1);
            assert_eq!(report["framePublications"], 0);
            report["dimension"] = json!(dimension);
            report["size"] = json!(size);
            samples.push(report);
        }
    }
    // Write evidence before the growth gate so a failure retains its work counters.
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_NESTED_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&json!({"samples":samples})).unwrap(),
        )
        .unwrap();
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap(),
                "{}",
                pair[1]["dimension"]
            );
        }
    }
}

#[test]
fn nested_unknown_stop_matches_native_without_legacy_baseline() {
    use crate::code_rule_dsl::{
        matcher,
        sql::{exec, tests::differential::tape},
    };
    use ast_grep_core::AstGrep;
    // The legacy SQL compiler fails this fixture with "no such table: u".
    // Compare exact true/unknown roots directly with the independent native oracle.
    let p = policy(
        json!({"has":{"testUnknown":"u","stopBy":{"has":{"testUnknown":"s"}}}}),
        json!({}),
    );
    let grammar = &grammar_matchers(&p)[0];
    let doc = tape(grammar.language, "u; s; a; f(u,s);");
    let tree = AstGrep::doc(doc.clone());
    let mut expected = Vec::new();
    matcher::with_file_memo(&tree, || {
        for id in 0..doc.transducer_len() as u32 {
            let truth = matcher::leaf_verdict(
                &grammar.matcher,
                matcher::node_at(&tree, id),
                &Default::default(),
            )
            .unwrap();
            if truth != Some(false) {
                expected.push((id, if truth == Some(true) { 1u8 } else { 2u8 }));
            }
        }
    });
    let compiled = super::super::compile(grammar).unwrap();
    let prefix = compiled.sql.split_once("-- final matches").unwrap().0;
    let demand = compiled.relations["outcomeDemand"].as_str().unwrap();
    let output = compiled.relations["outcomeOutput"].as_str().unwrap();
    exec::with_terminals(
        vec![(doc, Default::default())],
        compiled.terminals,
        |db, batch, _| {
            super::super::leaves::register(db, batch).unwrap();
            let actual = db.prepare(&format!("{prefix}SELECT d.node,o.truth FROM {demand} d JOIN {output} o ON o.k=d.k WHERE o.truth<>0 ORDER BY d.node"))
            .unwrap().query_map([], |r| Ok((r.get::<_,u32>(0)?,r.get::<_,u8>(1)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            assert_eq!(actual, expected);
        },
    );
}
