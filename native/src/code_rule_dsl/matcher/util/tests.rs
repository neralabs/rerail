    use super::*;
    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn utils_recurse_through_relations_to_any_depth() {
        let rule = compiled(json!({
            "rule": { "kind": "return_statement", "has": { "matches": "tail-await" } },
            "utils": {
                "tail-await": { "any": [
                    { "kind": "await_expression" },
                    { "kind": "parenthesized_expression", "has": { "matches": "tail-await" } },
                    { "kind": "sequence_expression", "has": { "matches": "tail-await" } },
                ] },
            },
        }));
        let source = "async function f() { return ((((await x)))); }\n\
                      async function g() { return (x); }\n\
                      async function h() { return await y; }\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["return ((((await x))));", "return await y;"],
            "{result}"
        );
    }

    #[test]
    fn utils_share_the_callers_metavariable_bindings() {
        // `$F` is bound by the rule before the utility runs, so the utility's `$F` must agree.
        let rule = compiled(json!({
            "rule": { "pattern": "$F($A)", "matches": "calls-itself" },
            "utils": { "calls-itself": { "pattern": "$F($F)" } },
        }));
        let source = "g(g);\ng(h);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["g(g) A=g F=g"], "{result}");

        // A utility's own captures are visible to the rule's other terms and in the witness.
        let rule = compiled(json!({
            "rule": { "all": [{ "matches": "call" }, { "has": { "stopBy": "end", "pattern": "$ARG" , "kind": "number" } }] },
            "utils": { "call": { "pattern": "$CALLEE($$$)" } },
        }));
        let source = "f(1);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["f(1) ARG=1 CALLEE=f"],
            "{result}"
        );
    }

    #[test]
    fn utils_may_use_other_utils_and_constraints_may_use_utils() {
        let rule = compiled(json!({
            "rule": { "pattern": "$FN($$$ARGS)", "matches": "hook-call" },
            "utils": {
                "hook-call": { "has": { "kind": "identifier", "matches": "hook-name" } },
                "hook-name": { "regex": "^use[A-Z]" },
            },
            "constraints": { "FN": { "matches": "hook-name" } },
        }));
        let source = "useState(1);\nfoo(2);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["useState(1) ARGS=1 FN=useState"],
            "{result}"
        );
    }

    #[test]
    fn constraints_filter_single_captures_after_the_rule_matches() {
        let rule = compiled(json!({
            "rule": { "pattern": "$FN($$$A)" },
            "constraints": { "FN": { "regex": "^use[A-Z]" } },
        }));
        let source = "useState(1);\nuser(2);\nuseEffect(f);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            [
                "useState(1) A=1 FN=useState",
                "useEffect(f) A=f FN=useEffect"
            ],
            "{result}"
        );

        // A capture made by a relation is constrained like any other.
        let rule = compiled(json!({
            "rule": {
                "pattern": "$HOOK($$$)",
                "inside": { "stopBy": "end", "kind": "if_statement", "has": { "pattern": "($COND)" } },
            },
            "constraints": { "HOOK": { "regex": "^use[A-Z]" }, "COND": { "kind": "identifier" } },
        }));
        let source = "if (ready) { useState(1); }\nif (a.b) { useMemo(g); }\nif (ok) { log(); }\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["useState(1) COND=ready HOOK=useState"],
            "{result}"
        );

        // `not` in a constraint; a metavariable left unbound (an `any` branch not taken) is not checked.
        let rule = compiled(json!({
            "rule": { "any": [{ "pattern": "$X + $Y" }, { "pattern": "-$X" }] },
            "constraints": { "Y": { "not": { "kind": "number" } } },
        }));
        let source = "a + 1;\na + b;\n-c;\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["a + b X=a Y=b", "-c X=c"],
            "{result}"
        );
    }

    #[test]
    fn util_recursion_deeper_than_the_limit_fails_the_clause() {
        let rule = compiled(json!({
            "rule": { "kind": "expression_statement", "has": { "stopBy": "end", "matches": "nested" } },
            // Every array whose innermost array is empty: the recursion runs to the bottom.
            "utils": { "nested": { "kind": "array", "any": [
                { "not": { "has": { "kind": "array" } } },
                { "has": { "matches": "nested" } },
            ] } },
        }));
        let depth = MAX_UTIL_DEPTH as usize;
        let source = |levels: usize| format!("x = {}{};\n", "[".repeat(levels), "]".repeat(levels));
        // Within both limits in any build: debug frames take about 15 KiB per level of this rule.
        let shallow = source(50);
        let result = evaluate_file(&rule, "typescript", "a.ts", &shallow);
        assert_eq!(result["failures"], json!([]), "{result}");
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
        let deep = source(depth + 50);
        let result = evaluate_file(&rule, "typescript", "a.ts", &deep);
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"],
            json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Depth.message(), "path": "a.ts" }]),
            "{result}"
        );
        assert!(
            UtilLimit::Depth
                .message()
                .contains(&MAX_UTIL_DEPTH.to_string())
        );
        assert!(
            UtilLimit::Depth
                .message()
                .contains(&(MAX_UTIL_STACK_BYTES >> 20).to_string())
        );
    }

    #[test]
    fn util_evaluations_are_metered_by_the_visited_node_budget() {
        // Each level re-walks every descendant before its cheap test fails: exponential in depth.
        // The cheap test reads text a walk cannot select nodes by (a `kind` or a literal would let
        // the walk pass over every node before recursing).
        let document = clause_document(json!({
            "id": "ast-match",
            "violation": violation(),
            "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "program", "matches": "slow" },
                "utils": { "slow": { "has": { "stopBy": "end", "all": [{ "matches": "slow" }, { "not": { "regex": "." } }] } } },
            } },
        }));
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxVisitedNodes"] = json!(10_000)
        });
        let rule = compile_document_with(document, &capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = format!("{}x;{}\n", "{".repeat(30), "}".repeat(30));
        let result = evaluate_file(&rule, "typescript", "a.ts", &source);
        assert_eq!(
            result["failures"],
            json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Work.message(), "path": "a.ts" }]),
            "{result}"
        );
        // The same rule on a shallow file finishes inside the budget.
        let result = evaluate_file(&rule, "typescript", "a.ts", "{ x; }\n");
        assert_violations(&result, 0);
    }
