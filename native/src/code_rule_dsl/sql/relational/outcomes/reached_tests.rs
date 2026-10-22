use super::super::tests::measure;
use super::tests::policy;
use crate::code_rule_dsl::{
    matcher::{node_at, value_probe},
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

fn parity(rule: Value, source: &str) -> Value {
    let p = policy(rule, json!({}));
    measure(&grammar_matchers(&p)[0], &[("a.ts".into(), source.into())])
}

/// Faults occur in the same atomic native ValueTest in both engines, never in a composition callback.
pub(super) fn reached(
    rule: Value,
    source: &str,
    fault: Option<&str>,
    expected: &[&str],
    fails: bool,
) {
    let p = policy(rule, json!({}));
    let grammar = &grammar_matchers(&p)[0];
    let doc = tape(grammar.language, source);
    let tree = AstGrep::doc(doc.clone());
    let (native, native_calls) = value_probe::run(fault, || {
        let mut work = usize::MAX;
        crate::code_rule_dsl::matcher::with_file_memo(&tree, || {
            for id in 0..doc.transducer_len() as u32 {
                grammar.match_root(node_at(&tree, id), &mut work)?;
            }
            Ok::<_, crate::code_rule_dsl::matcher::UtilLimit>(())
        })
    });
    assert_eq!(native.is_err(), fails, "native calls: {native_calls:?}");
    let prefix = if fails {
        native_calls
            .iter()
            .position(|text| Some(text.as_str()) == fault)
            .unwrap()
            + 1
    } else {
        native_calls.len()
    };
    assert_eq!(
        &native_calls[..prefix],
        expected,
        "native calls through first failure"
    );
    let compiled = super::super::compile(grammar).unwrap();
    let mut executions = 0;
    let (sql, calls) = value_probe::run(fault, || {
        exec::with_terminals(
            vec![(doc, Default::default())],
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let mut statement = db.prepare(&compiled.sql).unwrap();
                let trace = super::super::tests::ExecutionTrace::start(db);
                let result = (|| {
                    let mut rows = statement.query([])?;
                    while rows.next()?.is_some() {}
                    Ok::<_, rusqlite::Error>(())
                })();
                executions = trace.count();
                result
            },
        )
    });
    assert_eq!(executions, 1);
    println!(
        "REACHED_TRACE {}",
        json!({"source":source,"fault":fault,"nativeCalls":native_calls,"sqlCalls":calls,"nativeCallsAfterFailure":&native_calls[prefix..],"nativeFailureCode":native.as_ref().err().map(|e|format!("{e:?}")),"sqlFailureCode":sql.as_ref().err().map(ToString::to_string),"matchingStatements":executions})
    );
    assert_eq!(sql.is_err(), fails, "SQL calls: {calls:?}");
    assert_eq!(calls, expected, "SQL reached calls");
    if let (Err(native), Err(sql)) = (native, sql) {
        assert!(sql.to_string().contains(&format!("{native:?}")), "{sql}");
    }
}

#[test]
fn reached_values_preserve_native_truth_and_ordered_boolean_guards() {
    for rule in [
        json!({"kind":"identifier","value":{"type":"number"}}),
        json!({"kind":"number","not":{"value":{"eq":2}}}),
        json!({"kind":"arguments","has":{"stopBy":"end","value":{"eq":2}}}),
        json!({"kind":"arguments","count":{"has":{"value":{"type":"number"}},"min":0,"max":10,"as":"N"}}),
    ] {
        parity(rule, "f(1,2,x); g();\n");
    }
    reached(
        json!({"kind":"number","any":[{"regex":".*"},{"value":{"eq":1}}]}),
        "1;",
        Some("1"),
        &[],
        false,
    );
    reached(
        json!({"kind":"number","all":[{"regex":"^absent$"},{"value":{"eq":1}}]}),
        "1;",
        Some("1"),
        &[],
        false,
    );
    // A later false kind cannot erase a reached failure through root-demand pruning.
    reached(
        json!({"all":[{"value":{"eq":1}},{"kind":"identifier"}]}),
        "1;",
        Some("1"),
        &["1;", "1;", "1"],
        true,
    );
    reached(
        json!({"kind":"number","value":{"eq":1}}),
        "1;",
        Some("1"),
        &["1"],
        true,
    );
}

#[test]
fn reached_search_and_count_stop_before_later_value_failures() {
    let source = "f(1,2,3);";
    reached(
        json!({"kind":"arguments","has":{"kind":"number","value":{"eq":1}}}),
        source,
        Some("2"),
        &["1"],
        false,
    );
    reached(
        json!({"kind":"arguments","has":{"kind":"number","value":{"eq":2}}}),
        source,
        Some("2"),
        &["1", "2"],
        true,
    );
    reached(
        json!({"kind":"arguments","count":{"has":{"kind":"number","value":{"type":"number"}},"min":1}}),
        source,
        Some("2"),
        &["1"],
        false,
    );
    reached(
        json!({"kind":"arguments","count":{"has":{"kind":"number","value":{"type":"number"}},"max":0}}),
        source,
        Some("2"),
        &["1"],
        false,
    );
    reached(
        json!({"kind":"arguments","count":{"has":{"kind":"number","value":{"type":"number"}},"min":0,"max":10,"as":"N"}}),
        source,
        Some("2"),
        &["1", "2"],
        true,
    );
}

#[test]
fn reached_stop_order_differs_between_descendants_and_siblings() {
    // A matching descendant bypasses its failing stop; a sibling tests its stop first.
    reached(
        json!({"kind":"arguments","has":{"kind":"number","regex":"^1$","stopBy":{"value":{"eq":1}}},"testUnknown":"^never$"}),
        "f(1,2);",
        Some("1"),
        &["("],
        false,
    );
    reached(
        json!({"kind":"number","regex":"^0$","precedes":{"kind":"number","stopBy":{"value":{"eq":1}}}}),
        "f(0,1,2);",
        Some("1"),
        &[",", "1"],
        true,
    );
    // A true stop is inclusive; its descendants are not read.
    reached(
        json!({"kind":"arguments","has":{"kind":"number","value":{"eq":99},"stopBy":{"kind":"array"}}}),
        "f([1],2);",
        Some("1"),
        &["2"],
        false,
    );
    reached(
        json!({"kind":"arguments","has":{"kind":"number","value":{"eq":1},"stopBy":{"kind":"number","value":{"eq":1}}}}),
        "f(1,2);",
        Some("2"),
        &["1"],
        false,
    );
}

#[test]
fn reached_stops_preserve_unknown_frontiers_and_count_intervals() {
    let stop = json!({"any":[{"kind":"array","testUnknown":".*"},{"regex":"^never$"}]});
    let source = "f([1],2); g([1]); h(1);";
    for bounds in [
        json!({"min":1}),
        json!({"min":2}),
        json!({"max":0}),
        json!({"min":0,"max":10,"as":"N"}),
    ] {
        let mut count = bounds;
        count["has"] = json!({"kind":"number","stopBy":stop});
        parity(json!({"kind":"arguments","count":count}), source);
    }
    parity(
        json!({"kind":"arguments","has":{"kind":"number","stopBy":stop}}),
        source,
    );
    for axis in ["inside", "follows", "precedes"] {
        parity(
            json!({"kind":"number",axis:{"kind":"number","stopBy":{"testUnknown":"^[,]$"}}}),
            "f(1,2); g([3]);",
        );
    }
    // Unknown at a leaf still makes the upper bound open, even without actual hidden descendants.
    parity(
        json!({"kind":"arguments","count":{"has":{"kind":"number","stopBy":{"kind":"number","testUnknown":".*"}},"max":2}}),
        "f(1);",
    );
}

#[test]
fn reached_ordered_kernels_scale_without_reading_decided_tails() {
    let mut samples = vec![];
    for (dimension, number) in [("known-first", "1"), ("unknown-width", "x")] {
        for size in [16, 64, 256] {
            let source = format!("f({});", vec![number; size].join(","));
            let mut report = parity(
                json!({"kind":"arguments","has":{"any":[{"kind":"number"},{"kind":"identifier"}],"value":{"eq":1},"stopBy":{"regex":"^never$"}}}),
                &source,
            );
            assert_eq!(report["matchingStatements"], 1);
            assert_eq!(
                report["valueRequests"],
                if dimension == "known-first" { 1 } else { size }
            );
            report["dimension"] = json!(dimension);
            report["size"] = json!(size);
            samples.push(report);
        }
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap()
            );
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_REACHED_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&json!({"samples":samples})).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn reached_walk_axes_and_withheld_inputs_keep_effect_boundaries() {
    for axis in ["inside", "has", "follows", "precedes"] {
        for stop in [
            json!("neighbor"),
            json!("end"),
            json!({"any":[{"kind":"array"},{"kind":"identifier","testUnknown":"^x$"}]}),
        ] {
            let rule = json!({"kind":"number",axis:{"value":{"constant":true},"stopBy":stop}});
            let p = policy(rule, json!({}));
            measure(
                &grammar_matchers(&p)[0],
                &[
                    ("a.ts".into(), "f([1,x],2);".into()),
                    ("b.ts".into(), "g(3);".into()),
                ],
            );
        }
    }
    reached(
        json!({"kind":"arguments","all":[
            {"has":{"kind":"identifier","pattern":"$X","testUnknown":".*"}},
            {"all":[{"value":{"eq":1}},{"pattern":"$X"}]}
        ]}),
        "f(x);",
        Some("(x)"),
        &[],
        false,
    );
}

#[test]
fn reached_root_pruning_cannot_read_a_withheld_kind_test() {
    let report = parity(
        json!({"all":[
            {"pattern":"$X","testUnknown":".*"},
            {"all":[{"kind":"number"},{"pattern":"$X"}]}
        ]}),
        "x;",
    );
    assert_eq!(report["outcomes"].as_array().unwrap().len(), 3);
    assert_eq!(report["matches"], 0);
}
