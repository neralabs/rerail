use super::super::tests::measure;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::node_at,
    sql::tests::differential::tape,
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

pub(super) fn policy(rule: Value, utils: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"outcome-semantics","revision":1,"mode":"steer",
        "scope":{"include":["**/*"],"exclude":[]},"clauses":[{"id":"rule","forbid":{"ast":{"language":"typescript","rule":rule,"utils":utils}},
        "violation":{"code":"O","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}

fn checked(rule: Value, source: &str, expected: &[&str]) -> Value {
    let p = policy(rule, json!({}));
    let grammar = &grammar_matchers(&p)[0];
    assert!(grammar.three_valued);
    let result = measure(grammar, &[("a.ts".into(), source.into())]);
    let tree = AstGrep::doc(tape(grammar.language, source));
    let texts: Vec<_> = result["output"][0]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            node_at(&tree, row[0].as_u64().unwrap() as u32)
                .text()
                .into_owned()
        })
        .collect();
    assert_eq!(texts, expected);
    assert_eq!(result["matchingStatements"], 1);
    assert_eq!(result["framePublications"], 0);
    result
}

fn first_binding() -> Value {
    json!({"has":{"kind":"arguments","has":{"kind":"identifier","pattern":"$X","testUnknown":"^u"}}})
}

#[test]
fn outcomes_distinguish_false_unknown_and_missing_captures() {
    checked(
        json!({"kind":"identifier","testUnknown":"^u"}),
        "f(a,u1);\n",
        &["f", "a"],
    );
    checked(
        json!({"kind":"identifier","not":{"testUnknown":"^u"}}),
        "f(a,u1);\n",
        &[],
    );
    let source = "f(u1);\ng(a);\n";
    checked(
        json!({"kind":"call_expression","not":{"all":[first_binding(),
        {"has":{"stopBy":"end","kind":"identifier","regex":"^zzz","pattern":"$X"}}]}}),
        source,
        &["g(a)"],
    );
    checked(
        json!({"kind":"call_expression","not":{"all":[first_binding(),
        {"has":{"stopBy":"end","kind":"identifier","regex":"^zzz"}}]}}),
        source,
        &["f(u1)", "g(a)"],
    );
}

#[test]
fn outcomes_withholding_propagates_and_independent_false_still_decides() {
    let middle = json!({"has":{"kind":"identifier","pattern":"$Y","not":{"pattern":"$X"}}});
    for (last, expected) in [
        (
            json!({"has":{"stopBy":"end","regex":"^zzz","pattern":"$Y"}}),
            vec!["g(a)"],
        ),
        (json!({"regex":"^zzz"}), vec!["f(u1)", "g(a)"]),
    ] {
        checked(
            json!({"kind":"call_expression","not":{"all":[first_binding(),middle,last]}}),
            "f(u1);\ng(a);\n",
            &expected,
        );
    }
    // Even an existing value becomes unavailable after an unknown redefinition.
    checked(
        json!({"kind":"identifier","all":[{"pattern":"$X"},{"not":{"all":[
        {"pattern":"$X","testUnknown":"^u"},{"pattern":"$X","regex":"^zzz"}]}}]}),
        "u1; a;\n",
        &["a"],
    );
}

#[test]
fn outcomes_alternatives_commit_only_the_first_decisive_branch() {
    let source = "f(u1);\ng(a);\n";
    checked(
        json!({"kind":"call_expression","any":[first_binding(),{"pattern":"$F"}]}),
        source,
        &["g(a)"],
    );
    checked(
        json!({"kind":"call_expression","any":[
        {"has":{"kind":"arguments","has":{"kind":"identifier","testUnknown":"^u"}}},{"pattern":"$F"}]}),
        source,
        &["f(u1)", "g(a)"],
    );
    checked(
        json!({"kind":"call_expression","not":{"any":[
        {"has":{"kind":"arguments","has":{"kind":"identifier","testUnknown":"^u"}}},{"regex":"^zzz"}]}}),
        "f(u1);\ng(1);\n",
        &["g(1)"],
    );
    // Failed branches must not leave X bound to the callee when the fallback binds its argument.
    checked(
        json!({"kind":"call_expression","any":[
        {"all":[{"has":{"kind":"identifier","pattern":"$X"}},{"regex":"^zzz"}]},first_binding()]}),
        "f(a);\n",
        &["f(a)"],
    );
}

#[test]
fn outcomes_search_preserves_unknown_barriers_and_does_not_backtrack() {
    let source = "f(u1,a);\ng(a,u1);\n";
    checked(
        json!({"kind":"arguments","has":{"kind":"identifier","pattern":"$X","testUnknown":"^u"}}),
        source,
        &["(a,u1)"],
    );
    checked(
        json!({"kind":"arguments","has":{"kind":"identifier","testUnknown":"^u"}}),
        source,
        &["(u1,a)", "(a,u1)"],
    );
    checked(
        json!({"kind":"arguments","all":[
        {"has":{"kind":"identifier","pattern":"$X","testUnknown":"^u"}},
        {"has":{"kind":"identifier","regex":"^b$","pattern":"$X"}}]}),
        "f(a,b);\ng(b,a);\n",
        &["(b,a)"],
    );
}

#[test]
fn outcomes_counts_use_intervals_and_export_only_exact_values() {
    let source = "f(a,b,u1);\ng(a,u1);\nh(u1,u2);\n";
    for (bounds, expected) in [
        (json!({"min":2}), vec![]),
        (json!({"min":3}), vec!["(a,u1)", "(u1,u2)"]),
        (json!({"max":1}), vec!["(a,b,u1)"]),
        (
            json!({"min":4,"max":4}),
            vec!["(a,b,u1)", "(a,u1)", "(u1,u2)"],
        ),
    ] {
        let mut count = bounds;
        count["has"] = json!({"kind":"identifier","testUnknown":"^u"});
        checked(
            json!({"kind":"arguments","not":{"count":count}}),
            source,
            &expected,
        );
    }
    let result = checked(
        json!({"kind":"arguments","count":{"has":{"kind":"identifier","testUnknown":"^u"},"min":0,"max":10,"as":"N","offset":3}}),
        "f(a,b);\ng(a,u1);\nh();\n",
        &["(a,b)", "()"],
    );
    assert_eq!(result["output"][0][0][1], json!(["5"]));
    assert_eq!(result["output"][0][1][1], json!(["3"]));
    checked(
        json!({"kind":"arguments","count":{"has":{"kind":"identifier","testUnknown":"^u"},"max":0}}),
        "f();\ng(a);\nh(u1);\n",
        &["()"],
    );
}

#[test]
fn outcomes_count_probes_preserve_nested_three_valued_boolean_tests() {
    for (probe, expected) in [
        (
            json!({"kind":"identifier","any":[{"testUnknown":"^u"},{"regex":"^z$"}]}),
            vec!["()"],
        ),
        (
            json!({"kind":"identifier","not":{"testUnknown":"^u"}}),
            vec!["(a)", "()"],
        ),
    ] {
        checked(
            json!({"kind":"arguments","not":{"count":{"has":probe,"min":1}}}),
            "f(u1);\ng(a);\nh();\n",
            &expected,
        );
    }
}

#[test]
fn outcomes_composite_capture_equality_keeps_the_later_witness() {
    let result = checked(
        json!({"kind":"arguments","all":[
            {"has":{"kind":"array","pattern":"$X","testUnknown":"^\\[u"}},
            {"has":{"kind":"array","pattern":"$X","follows":{"stopBy":"end","kind":"array"}}}
        ]}),
        "f([a],[a]);\ng([a],[b]);\nh([u],[u]);\n",
        &["([a],[a])"],
    );
    assert!(result["captureEqualityRequests"].as_u64().unwrap() > 0);
}

#[test]
fn outcomes_walks_and_utility_aliases_preserve_order_and_file_identity() {
    for axis in ["inside", "has", "follows", "precedes"] {
        for stop in ["neighbor", "end"] {
            let p = policy(
                json!({"all":[{"kind":"identifier"},{axis:{"stopBy":stop,"matches":"probe"}}]}),
                json!({"probe":{"any":[{"kind":"identifier","pattern":"$X","testUnknown":"^u"},{"kind":"arguments","testUnknown":"^never$"}]}}),
            );
            measure(
                &grammar_matchers(&p)[0],
                &[
                    ("a.ts".into(), "f(a,u1,b);\n".into()),
                    ("b.ts".into(), "g(u1,a);\n".into()),
                ],
            );
        }
    }
}

#[test]
fn outcomes_reached_demands_skip_decided_and_withheld_terms() {
    let result = checked(
        json!({"all":[{"regex":"^never$"},{"testUnknown":"."}]}),
        "f(a);\n",
        &[],
    );
    let p = policy(json!({"testUnknown":"."}), json!({}));
    let grammar = &grammar_matchers(&p)[0];
    assert_eq!(
        result["regexRequests"],
        tape(grammar.language, "f(a);\n").transducer_len()
    );
    checked(
        json!({"any":[{"regex":"(?s).*"},{"testUnknown":"."}]}),
        "x",
        &["x", "x", "x"],
    );
}

#[test]
fn outcomes_reject_unproved_recursive_outcomes() {
    let p = policy(
        json!({"matches":"r"}),
        json!({"r":{"any":[{"testUnknown":"u"},{"inside":{"stopBy":"neighbor","matches":"r"}}]}}),
    );
    let Err(error) = super::super::compile(&grammar_matchers(&p)[0]) else {
        panic!("accepted unproved recursive outcomes")
    };
    assert!(error.contains("recursive outcomes"), "{error}");
}

#[test]
fn outcomes_scaling() {
    let mut samples = Vec::new();
    for size in [16, 64, 256] {
        let rule = json!({"kind":"call_expression","not":{"all":[first_binding(),
            {"has":{"stopBy":"end","kind":"identifier","regex":"^zzz","pattern":"$X"}}]}});
        let source = "f(u1);\ng(a);\n".repeat(size);
        let p = policy(rule, json!({}));
        let mut result = measure(
            &grammar_matchers(&p)[0],
            &[("conjunction.ts".into(), source)],
        );
        assert_eq!(result["matches"], size);
        result["dimension"] = json!("independent-conjunctions");
        result["size"] = json!(size);
        samples.push(result);
    }
    for size in [16, 64, 256] {
        let source = format!(
            "f({});\n",
            ["a", "u1"]
                .into_iter()
                .cycle()
                .take(size)
                .collect::<Vec<_>>()
                .join(",")
        );
        let p = policy(
            json!({"kind":"arguments","count":{"has":{"kind":"identifier","testUnknown":"^u"},"min":1}}),
            json!({}),
        );
        let mut result = measure(&grammar_matchers(&p)[0], &[("count.ts".into(), source)]);
        assert_eq!(result["matches"], 1);
        assert_eq!(
            result["regexRequests"], 1,
            "known first candidate must stop the existence probe"
        );
        result["dimension"] = json!("count-width");
        result["size"] = json!(size);
        samples.push(result);
    }
    for size in [16, 64, 256] {
        let source = format!("f({});\n", vec!["u1"; size].join(","));
        let p = policy(
            json!({"kind":"arguments","count":{"has":{"kind":"identifier","testUnknown":"^u"},"min":1}}),
            json!({}),
        );
        let mut result = measure(&grammar_matchers(&p)[0], &[("uncertain.ts".into(), source)]);
        assert_eq!(result["matches"], 0);
        assert_eq!(result["outcomes"].as_array().unwrap().len(), 1);
        assert_eq!(result["outcomes"][0][2], 2);
        assert_eq!(result["regexRequests"], size);
        result["dimension"] = json!("uncertain-count-width");
        result["size"] = json!(size);
        samples.push(result);
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap(),
                "superlinear outcome work"
            );
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_OUTCOME_EVIDENCE") {
        std::fs::write(path,serde_json::to_string_pretty(&json!({"scope":"Explicit outcome kernels, not whole-job or resource-limit parity","samples":samples})).unwrap()).unwrap();
    }
}
