//! Semantic and work-volume gates for the benchmark-only compiler paths.

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
    if !crate::code_rule_dsl::sql::diagnostics::shared_search() {
        return;
    }
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
