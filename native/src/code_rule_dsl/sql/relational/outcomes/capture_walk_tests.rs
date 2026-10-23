use super::super::tests::measure;
use super::reached_tests::reached;
use super::tests::policy;
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::{Value, json};

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
fn never() -> Value {
    json!({"regex":"^never$"})
}

#[test]
fn ordered_captures_test_next_candidate_before_pending_unknown_stop() {
    for captured in [false, true] {
        let mut body = json!({"kind":"expression_statement","regex":"^b;","stopBy":{"regex":"^a;","testUnknown":".*"}});
        if captured {
            body["pattern"] = json!("$X");
        }
        let result = parity(
            json!({"kind":"statement_block","has":body}),
            &["{ a; b; c; }", "{ a; c; b; }"],
        );
        assert_eq!(result["output"][0].as_array().unwrap().len(), 1);
        assert_eq!(
            result["output"][1].as_array().unwrap().len(),
            usize::from(!captured)
        );
    }
}

#[test]
fn ordered_captures_preserve_strict_unknown_and_no_backtracking() {
    let result = parity(
        json!({"kind":"arguments","has":{"kind":"identifier","pattern":"$X","testUnknown":"^u","stopBy":never()}}),
        &["f(u,a); g(a,u);"],
    );
    assert_eq!(result["matches"], 1);
    let result = parity(
        json!({"kind":"arguments","all":[
            {"has":{"kind":"identifier","pattern":"$X","testUnknown":"^never$","stopBy":never()}},
            {"has":{"kind":"identifier","regex":"^b$","pattern":"$X","stopBy":never()}}
        ]}),
        &["f(a,b); g(b,a);"],
    );
    assert_eq!(result["matches"], 1);
}

#[test]
fn ordered_capture_equality_replaces_with_exact_selected_witness() {
    let result = parity(
        json!({"kind":"array","pattern":"$X","testUnknown":"^never$","follows":{"kind":"array","pattern":"$X","stopBy":never()}}),
        &["f([a],[a],[b]);", "g([a],[b],[b]);"],
    );
    assert_eq!(result["matches"], 2);
    assert!(result["captureEqualityRequests"].as_u64().unwrap() > 0);
    for file in result["output"].as_array().unwrap() {
        for row in file.as_array().unwrap() {
            assert!(row[1][0].as_u64().unwrap() < row[0].as_u64().unwrap());
        }
    }
}

#[test]
fn ordered_capture_boolean_branches_restore_and_withhold_bindings() {
    let cases = [
        json!({"any":[{"all":[{"pattern":"$X"},never()]},{"kind":"identifier","pattern":"$Y","testUnknown":"^u"}]}),
        json!({"kind":"identifier","all":[{"pattern":"$X"},{"pattern":"$Y"},{"testUnknown":"^u"}]}),
        json!({"kind":"identifier","all":[{"pattern":"$X","testUnknown":"^u"},{"pattern":"$X","regex":"^a$"}]}),
        json!({"kind":"identifier","all":[{"pattern":"$X","testUnknown":"^u"},{"pattern":"$X","regex":"^a$"},never()]}),
        json!({"kind":"identifier","all":[{"not":{"all":[{"pattern":"$X"},never()]}},{"pattern":"$Y"},{"testUnknown":"^u"}]}),
        json!({"kind":"identifier","any":[{"pattern":"$X","testUnknown":"^u"},{"pattern":"$Y"}]}),
    ];
    for mut body in cases {
        body["stopBy"] = never();
        parity(
            json!({"kind":"arguments","has":body}),
            &["f(u,a); g(a,u); h();"],
        );
    }
}

#[test]
fn ordered_stops_use_private_capture_environment() {
    let result = parity(
        json!({"kind":"expression_statement","regex":"^b;","pattern":"$X","testUnknown":"^never$",
        "follows":{"regex":"^target;","stopBy":{"pattern":"$X"}}}),
        &["{ target; a; b; }"],
    );
    assert_eq!(result["matches"], 0);
    let result = parity(
        json!({"kind":"arguments","has":{"kind":"identifier","regex":"^a$","stopBy":{"kind":"array","pattern":"$PRIVATE","testUnknown":"^never$"}}}),
        &["f([a],a);"],
    );
    assert_eq!(result["matches"], 1);
    assert_eq!(result["output"][0][0][1], json!([]));
}

#[test]
fn ordered_count_probes_reset_captures_and_discard_exports() {
    let result = parity(
        json!({"kind":"arguments","count":{"has":{"kind":"identifier","pattern":"$X","testUnknown":"^never$","stopBy":never()},"min":0,"max":10,"as":"N"}}),
        &["f(a,b); g();"],
    );
    assert_eq!(result["output"][0][0][1], json!(["2"]));
    for bounds in [json!({"min":1}), json!({"min":0,"max":10,"as":"N"})] {
        let mut count = bounds;
        count["has"] =
            json!({"kind":"identifier","pattern":"$X","testUnknown":"^u","stopBy":never()});
        parity(
            json!({"kind":"arguments","count":count}),
            &["f(u,a); g(a,b);"],
        );
    }
}

#[test]
fn ordered_capture_faults_observe_pending_stop_and_withheld_inputs() {
    reached(
        json!({"kind":"statement_block","has":{"kind":"expression_statement","regex":"^b;","pattern":"$X","value":{"eq":1},"stopBy":{"regex":"^a;","testUnknown":".*"}}}),
        "{ a; b; }",
        Some("b;"),
        &["b;"],
        true,
    );
    reached(
        json!({"kind":"statement_block","has":{"kind":"expression_statement","regex":"^b;","pattern":"$X","stopBy":{"any":[{"regex":"^a;","testUnknown":".*"},{"kind":"expression_statement","value":{"constant":true}}]}}}),
        "{ a; c; b; }",
        Some("c;"),
        &["a;"],
        false,
    );
    reached(
        json!({"kind":"arguments","has":{"kind":"number","pattern":"$X","value":{"eq":1},"stopBy":{"kind":"number","value":{"eq":2}}}}),
        "f(1,2);",
        Some("2"),
        &["1"],
        false,
    );
    reached(
        json!({"kind":"arguments","has":{"kind":"identifier","all":[{"pattern":"$X","testUnknown":".*"},{"all":[{"pattern":"$X"},{"value":{"eq":1}}]}],"stopBy":never()}}),
        "f(u,a);",
        Some("u"),
        &[],
        false,
    );
}

#[test]
fn ordered_capture_admission_rejects_wide_local_programs() {
    let all: Vec<_> = (0..62)
        .map(|i| json!({"pattern":format!("$C{i}")}))
        .collect();
    let body = json!({"all":all,"testUnknown":"^never$","stopBy":never()});
    let p = policy(json!({"kind":"arguments","has":body}), json!({}));
    let Err(error) = super::super::compile(&grammar_matchers(&p)[0]) else {
        panic!("accepted 62 local capture slots");
    };
    assert!(error.contains("61 local capture slots"), "{error}");
}

#[test]
fn ordered_capture_walks_scale_with_reached_work() {
    let mut samples = vec![];
    for dimension in [
        "known-first",
        "late-match",
        "independent-roots",
        "input-witness-roots",
    ] {
        for size in [16, 64, 256] {
            let source = match dimension {
                "known-first" => format!("f({});", vec!["x"; size].join(",")),
                "late-match" => format!("f({},z);", vec!["a"; size - 1].join(",")),
                "input-witness-roots" => format!("f({});", vec!["[a]"; size].join(",")),
                _ => "f(x);".repeat(size),
            };
            let mut body = json!({"kind":"identifier","pattern":"$X","value":{"constant":false},"stopBy":never()});
            if dimension == "late-match" {
                body["regex"] = json!("^z$");
            }
            let mut result = if dimension == "input-witness-roots" {
                parity(
                    json!({"kind":"array","pattern":"$X","testUnknown":"^never$","follows":{"kind":"array","pattern":"$X","stopBy":never()}}),
                    &[&source],
                )
            } else {
                parity(json!({"kind":"arguments","has":body}), &[&source])
            };
            assert_eq!(
                result["valueRequests"],
                if dimension == "independent-roots" {
                    size
                } else if dimension == "input-witness-roots" {
                    0
                } else {
                    1
                }
            );
            assert_eq!(result["matchingStatements"], 1);
            assert_eq!(result["framePublications"], 0);
            result["dimension"] = json!(dimension);
            result["size"] = json!(size);
            samples.push(result);
        }
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap()
            );
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_CAPTURE_WALK_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&json!({"samples":samples})).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn ordered_capture_masks_use_all_61_bits_without_changing_walk_width() {
    let mut all: Vec<_> = (0..61)
        .map(|i| json!({"pattern":format!("$C{i}")}))
        .collect();
    all.push(json!({"testUnknown":"^never$"}));
    let result = parity(
        json!({"kind":"arguments","has":{"kind":"identifier","all":all,"stopBy":never()}}),
        &["f(a);"],
    );
    let captures = result["output"][0][0][1].as_array().unwrap();
    assert_eq!(captures.len(), 61);
    assert!(captures.iter().all(|c| c == &captures[0]));
    let walk = result["relations"]["relations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["operator"] == "ordered reached traversal")
        .unwrap();
    assert_eq!(walk["columns"].as_str().unwrap().split(',').count(), 10);
}

#[test]
fn ordered_captures_preserve_parent_and_sibling_stop_inclusivity() {
    for axis in ["inside", "follows", "precedes"] {
        for stop in [
            never(),
            json!({"kind":"identifier","testUnknown":"^u"}),
            json!({"pattern":"$PRIVATE"}),
        ] {
            parity(
                json!({"kind":"identifier",axis:{"pattern":"$X","testUnknown":"^u","stopBy":stop}}),
                &["f(u,a); g(a,u);", "h([a],b);"],
            );
        }
    }
}
