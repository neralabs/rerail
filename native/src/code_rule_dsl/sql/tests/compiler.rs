//! Semantic and work-volume gates for SQL compiler optimizations.

use super::hard_cases::agreed;
#[cfg(feature = "unbounded-work")]
use super::hard_cases::compile;
#[cfg(feature = "unbounded-work")]
use super::{same, snapshot};
#[cfg(feature = "unbounded-work")]
use crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE;
use serde_json::json;

#[test]
fn sibling_candidates_preserve_captures_without_sql_frame_advances() {
    use crate::code_rule_dsl::sql::profiling::frame_counts;

    let blocks = 64;
    let source =
        format!("{{ marker(a); target(a); {} }}\n", "decoy(a); ".repeat(12)).repeat(blocks);
    for axis in ["precedes", "follows"] {
        for three in [false, true] {
            let mut rule = json!({
                "kind": "expression_statement", "regex": "^target",
                axis: { "stopBy": "end", "kind": "expression_statement", "pattern": "$X" }
            });
            if three {
                rule["all"] = json!([{ "testUnknown": "^never$" }]);
            }
            let (found, work) =
                frame_counts(|| agreed("typescript", json!({ "rule": rule }), &source));
            let capture = if axis == "follows" {
                "marker(a);"
            } else {
                "decoy(a);"
            };
            assert_eq!(found, vec![format!("target(a); X={capture}"); blocks]);
            assert_eq!(
                work[2], 0,
                "{axis}, three={three}: SQL must not walk matcher frames"
            );
        }
    }
}

#[test]
fn shared_search_preserves_outer_captures_and_commits_the_first_inner_capture() {
    for three in [false, true] {
        let mut ast = json!({ "rule": {
            "pattern": "$F($$$)",
            "has": { "stopBy": "end", "kind": "identifier", "regex": "^[ab]$", "pattern": "$X" }
        } });
        if three {
            ast["rule"]["all"] = json!([{ "testUnknown": "^never$" }]);
        }
        assert_eq!(
            agreed("typescript", ast, "f(a, b);\ng(c, b);\n"),
            ["f(a, b) F=f X=a", "g(c, b) F=g X=b"]
        );
    }
}

#[test]
fn eligible_nodes_in_the_gap_between_roots_do_not_match_either_root() {
    let ast = json!({ "rule": {
        "kind": "expression_statement", "regex": "^[fg]\\(",
        "has": { "stopBy": "end", "kind": "identifier", "regex": "^target$", "pattern": "$X" }
    } });
    assert!(agreed("typescript", ast, "f(a);\ntarget;\ng(b);\n").is_empty());
}

#[test]
fn fused_filters_keep_unknown_verdicts_and_later_false_terms_can_still_decide() {
    for regex in ["^[ab]$", "^missing$"] {
        let ast = json!({ "rule": {
            "kind": "identifier",
            "not": { "all": [
                { "testUnknown": "^a$" },
                { "kind": "identifier" },
                { "regex": regex }
            ] }
        } });
        let found = agreed("typescript", ast, "a;\nb;\nc;\n");
        if regex == "^missing$" {
            assert_eq!(found, ["a", "b", "c"]);
        } else {
            assert_eq!(found, ["c"]);
        }
    }
}

#[cfg(feature = "unbounded-work")]
#[test]
fn shared_search_avoids_quadratic_rows_for_early_late_and_missing_matches() {
    let depth = 128;
    let mut source = (0..depth)
        .map(|i| format!("if (a{i}) {{ f(a, b);\n"))
        .collect::<String>();
    source.push_str("target;\n");
    source.push_str(&"}\n".repeat(depth));
    for (kind, regex, pattern, expected) in [
        ("parenthesized_expression", ".", "$X", depth),
        ("identifier", "^target$", "$X", depth),
        ("call_expression", ".", "f($X, $X)", 0),
    ] {
        let rule = compile(
            "typescript",
            json!({ "rule": {
            "kind": "if_statement",
            "has": { "stopBy": "end", "kind": kind, "regex": regex, "pattern": pattern }
        } }),
        );
        STATEMENT_PROFILE.set(Some(Default::default()));
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        let profile = STATEMENT_PROFILE.take().unwrap_or_default();
        let executed = profile.values().map(|(_, _, runs)| runs).sum::<usize>();
        let rows = profile.values().map(|(_, rows, _)| rows).sum::<usize>();
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(expected)
        );
        assert_eq!(result["failures"], json!([]));
        assert!(
            executed > 0 && rows < 32 * depth,
            "{regex}: {rows} rows for {depth} roots"
        );
    }
}

#[test]
fn capture_demand_keeps_nullable_composite_and_occurrence_semantics() {
    for three in [false, true] {
        for (binding, source) in [
            ("$X($$$)", "f(f);\ng(g);\nh(x);\n"),
            ("f($X)", "f(a + b);\nf(a);\nf((a));\n"),
        ] {
            let mut ast = json!({ "rule": { "all": [
                { "pattern": binding },
                { "has": { "stopBy": "end", "all": [
                    { "kind": "identifier" }, { "pattern": "$X" }
                ] } }
            ] } });
            if three {
                ast["rule"]["all"][1]["has"]["all"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, json!({ "testUnknown": "^never$" }));
            }
            let found = agreed("typescript", ast, source);
            assert!(!found.is_empty());
        }
    }
    // A nullable slot must match freely, and disjunction must not inherit only one arm's guard.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "any": [{ "pattern": "f($X)" }, { "pattern": "g($$$)" }] },
        { "has": { "stopBy": "end", "kind": "identifier", "pattern": "$X" } }
    ] } });
    assert_eq!(
        agreed("typescript", ast, "f(a);\ng(b);\n"),
        ["f(a) X=a", "g(b) X=g"]
    );
}

#[test]
fn capture_demand_does_not_cross_binding_or_an_optional_branch() {
    let ast = json!({ "rule": { "pattern": "$X($$$)", "has": {
        "stopBy": "end", "any": [
            { "kind": "number", "pattern": "$Y" },
            { "kind": "identifier", "pattern": "$X" }
        ]
    } } });
    assert_eq!(
        agreed("typescript", ast, "f(1);\ng(g);\n"),
        ["f(1) X=f", "g(g) X=g"]
    );
}

#[cfg(feature = "unbounded-work")]
#[test]
fn capture_relations_batch_roots_in_both_verdict_modes() {
    let calls = 100;
    let source = format!("f({}f);\n", "a, ".repeat(128)).repeat(calls);
    for three in [false, true] {
        let mut terms = vec![json!({ "pattern": "$F($$$)" })];
        if three {
            terms.push(json!({ "testUnknown": "^never$" }));
        }
        terms.push(json!({ "has": { "kind": "arguments", "has": {
            "stopBy": "end", "kind": "identifier", "pattern": "$F"
        } } }));
        let rule = compile("typescript", json!({ "rule": { "all": terms } }));
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        assert_eq!(result["violations"].as_array().map(Vec::len), Some(calls));
        let profile = super::fragment_boundary(&rule, &source);
        assert_eq!(profile["filters"], 1);
        assert_eq!(profile["roots"], calls);
        assert_eq!(profile["rows"], calls);
    }
}

#[test]
fn recursive_search_preserves_capture_occurrences() {
    let source = "f(a);\ng(z);\nh(a + b);\nf(f(a), f(a));\n";
    for three in [false, true] {
        let mut terms = vec![json!({ "pattern": "$F($$$)" })];
        if three {
            terms.push(json!({ "testUnknown": "^never$" }));
        }
        terms.push(json!({ "has": { "kind": "arguments", "matches": "find" } }));
        let ast = json!({ "rule": { "all": terms }, "utils": { "find": { "any": [
            { "kind": "identifier", "pattern": "$F" },
            { "has": { "stopBy": "neighbor", "matches": "find" } }
        ] } } });
        assert_eq!(agreed("typescript", ast, source), ["f(f(a), f(a)) F=f"]);
    }
}

#[cfg(feature = "unbounded-work")]
#[test]
fn recursive_capture_search_keeps_only_confirmed_outputs_at_the_sql_boundary() {
    let pairs = 50;
    let prefix = "a + ".repeat(128);
    let source = format!("f({prefix}a);\nf({prefix}f);\n").repeat(pairs);
    for three in [false, true] {
        let mut terms = vec![json!({ "pattern": "$F($$$)" })];
        if three {
            terms.push(json!({ "testUnknown": "^never$" }));
        }
        terms.push(json!({ "has": { "kind": "arguments", "matches": "find" } }));
        let rule = compile(
            "typescript",
            json!({ "rule": { "all": terms }, "utils": { "find": { "any": [
            { "kind": "identifier", "pattern": "$F" },
            { "has": { "stopBy": "neighbor", "matches": "find" } }
        ] } } }),
        );
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        assert_eq!(result["violations"].as_array().map(Vec::len), Some(pairs));
        let profile = super::fragment_boundary(&rule, &source);
        assert_eq!(profile["filters"], 1);
        assert_eq!(profile["rows"], pairs);
    }
}

#[test]
fn progressive_walks_preserve_first_capture_and_exact_or_threshold_counts() {
    let source = "f(a, b, a);\ng(b, a);\nh(c);\n";
    for three in [false, true] {
        for axis in ["has", "inside", "precedes", "follows"] {
            for stop in ["neighbor", "end"] {
                let mut rule = json!({"pattern":"$F($$$)", axis:{"stopBy":stop,"kind":"identifier","pattern":"$X"}});
                if three {
                    rule["all"] = json!([{"testUnknown":"^never$"}]);
                }
                agreed("typescript", json!({"rule":rule}), source);
            }
        }
        for min in 0..=3 {
            for max in [None, Some(0), Some(2), Some(5)] {
                if (min == 0 && max.is_none()) || max.is_some_and(|max| max < min) {
                    continue;
                }
                for exact in [false, true] {
                    let mut count = json!({"min":min,"has":{"stopBy":"end","kind":"identifier","pattern":"$F"}});
                    if let Some(max) = max {
                        count["max"] = json!(max);
                    }
                    if exact {
                        count["as"] = json!("N");
                        count["offset"] = json!(-2);
                    }
                    let mut rule = json!({"pattern":"$F($$$)","count":count});
                    if three {
                        rule["all"] = json!([{"testUnknown":"^never$"}]);
                    }
                    agreed("typescript", json!({"rule":rule}), source);
                }
            }
        }
    }
}

#[test]
fn value_only_demands_preserve_each_callers_capture_occurrence() {
    for three in [false, true] {
        let mut rule = json!({"pattern":"$F($$$)", "any":[{"inside":{"stopBy":"end", "kind":"program", "matches":"absent"}},{"kind":"call_expression"}]});
        if three {
            rule["all"] = json!([{"testUnknown":"^never$"}]);
        }
        let ast = json!({"rule":rule,"utils":{
            "absent":{"all":[{"not":{"pattern":"$F"}},{"not":{"has":{"matches":"absent"}}}]}
        }});
        let got = agreed("typescript", ast, "f(a);\nf(b);\ng(c);\n");
        assert_eq!(got, ["f(a) F=f", "f(b) F=f", "g(c) F=g"]);
    }
}

#[test]
fn large_candidate_walks_stay_inside_one_native_batch() {
    let source = format!("f(a{});", ", b".repeat(2048));
    for count in [false, true] {
        let ast = if count {
            json!({"rule":{"pattern":"f($$$)","count":{"min":3,"has":{"stopBy":"end","kind":"identifier","pattern":"$X"}}}})
        } else {
            json!({"rule":{"pattern":"f($$$)","has":{"stopBy":"end","kind":"identifier","pattern":"$X"}}})
        };
        let rule = super::hard_cases::compile("typescript", ast.clone());
        let got = agreed("typescript", ast, &source);
        assert_eq!(got.len(), 1);
        let profile = super::fragment_boundary(&rule, &source);
        assert_eq!(profile["filters"], 1);
        assert_eq!(profile["roots"], 1);
        assert_eq!(profile["rows"], 1);
    }
}

#[test]
fn shared_value_demands_return_the_winning_candidate_occurrence() {
    for three in [false, true] {
        let mut rule = json!({"pattern":"$F($$$)","inside":{"stopBy":"end","kind":"program","matches":"find"}});
        if three {
            rule["all"] = json!([{"testUnknown":"^never$"}]);
        }
        let ast = json!({"rule":rule,"utils":{"find":{"any":[{"kind":"identifier","pattern":"$F"},{"has":{"matches":"find"}}]}}});
        // `same` inside `agreed` compares complete spans, including the shared first `f`.
        assert_eq!(
            agreed("typescript", ast, "f(a);\nf(b);\ng(c);\n"),
            ["f(a) F=f", "f(b) F=f", "g(c) F=g"]
        );
    }
}
