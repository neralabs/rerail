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
        let rows = STATEMENT_PROFILE
            .take()
            .unwrap_or_default()
            .values()
            .map(|(_, rows, _)| rows)
            .sum::<usize>();
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(expected)
        );
        assert_eq!(result["failures"], json!([]));
        assert!(
            rows > 0 && rows < 32 * depth,
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
fn capture_postings_reduce_materialized_candidates_in_both_verdict_modes() {
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
        STATEMENT_PROFILE.set(Some(Default::default()));
        let result = same(&rule, &snapshot(&[("typescript", "a.ts", &source)]));
        assert_eq!(result["violations"].as_array().map(Vec::len), Some(calls));
        let profile = STATEMENT_PROFILE.take().unwrap_or_default();
        let narrowed: usize = profile
            .iter()
            .filter(|(sql, _)| sql.contains("needle.leaf"))
            .map(|(_, (_, rows, _))| rows)
            .sum();
        assert!(
            narrowed > 0 && narrowed <= calls * 3,
            "three={three}: {narrowed} restricted rows"
        );
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
