    use super::{Position, named_sibling_visits, parse_position, reset_named_sibling_visits};
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn texts(rule: Value, source: &str) -> Vec<String> {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    }

    #[test]
    fn positions_parse_as_upstream_an_plus_b() {
        let parsed = |text: &str| parse_position(text).map(|p| (p.step, p.offset));
        assert_eq!(parsed("3"), Ok((0, 3)));
        assert_eq!(parsed("2n+1"), Ok((2, 1)));
        assert_eq!(parsed(" -n + 3 "), Ok((-1, 3)));
        assert_eq!(parsed("n"), Ok((1, 0)));
        assert_eq!(parsed("+5"), Ok((0, 5)));
        assert_eq!(parsed("-2n-1"), Ok((-2, -1)));
        assert_eq!(parsed("2x"), Err("illegal character `x`".to_owned()));
        for bad in ["", "+", "n+", "2n3", "nn", "++1", "1+"] {
            assert_eq!(parsed(bad), Err("invalid syntax".to_owned()), "{bad}");
        }
        let odd = Position { step: 2, offset: 1 };
        assert_eq!(
            (0..6).map(|index| odd.holds(index)).collect::<Vec<_>>(),
            [true, false, true, false, true, false]
        );
        let first_three = Position {
            step: -1,
            offset: 3,
        };
        assert_eq!(
            (0..5)
                .map(|index| first_three.holds(index))
                .collect::<Vec<_>>(),
            [true, true, true, false, false]
        );
    }

    #[test]
    fn nth_child_counts_named_siblings_from_one() {
        let source = "f(a, b, c);\n[x, , y];\n";
        assert_eq!(
            texts(json!({ "kind": "identifier", "nthChild": 2 }), source),
            ["b", "y"]
        );
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "nthChild": "2n+1", "inside": { "kind": "arguments" } }),
                source
            ),
            ["a", "c"]
        );
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "nthChild": { "position": 1, "reverse": true } }),
                source
            ),
            ["c", "y"]
        );
        // ofRule counts only the siblings it matches, and binds what it binds on the node itself.
        let source = "f(1, a, 2, b);\n";
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "nthChild": { "position": 2, "ofRule": { "kind": "identifier", "pattern": "$X" } } }),
                source,
            ),
            ["b X=b"]
        );
        // A root has no parent, so no position.
        assert_eq!(
            texts(json!({ "kind": "program", "nthChild": 1 }), "a;\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn flat_named_siblings_are_visited_once_per_parent() {
        let source = format!(
            "f({});",
            (0..2000)
                .map(|index| format!("a{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        reset_named_sibling_visits();
        assert_eq!(
            texts(json!({ "kind": "identifier", "nthChild": "n" }), &source).len(),
            2001
        );
        assert!(
            named_sibling_visits() <= 4_000,
            "visited {} named siblings",
            named_sibling_visits()
        );
    }

    #[test]
    fn of_rule_positions_over_a_flat_list_stay_within_a_linear_budget() {
        const ARGUMENTS: usize = 500;
        let source = format!(
            "f({});\n",
            (0..ARGUMENTS)
                .map(|index| if index % 2 == 0 {
                    format!("a{index}")
                } else {
                    index.to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        // A quadratic probe of every sibling for every candidate (250,000 steps) exceeds this
        // budget; one pass over each parent's children stays far below it.
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxVisitedNodes"] = json!(20_000);
        });
        let rule = json!({ "kind": "identifier", "inside": { "kind": "arguments" },
            "nthChild": { "position": "2n", "ofRule": { "kind": "identifier" } } });
        let compiled = compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": {
                "language": "typescript", "rule": rule } }, "violation": violation() })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        assert_eq!(result["failures"], json!([]), "{result}");
        let expected = (0..ARGUMENTS)
            .step_by(2)
            .skip(1)
            .step_by(2)
            .map(|index| format!("a{index}"))
            .collect::<Vec<_>>();
        assert_eq!(matched_texts(&result, &source), expected);
    }

    #[test]
    fn a_sibling_of_unknown_verdict_leaves_the_position_unknown() {
        let source = "f(a, q, b);\ng(c, d);\n";
        let of_rule =
            json!({ "position": 2, "ofRule": { "kind": "identifier", "testUnknown": "^q$" } });
        assert_eq!(
            texts(json!({ "kind": "identifier", "nthChild": of_rule }), source),
            ["d"]
        );
        // Unknown is neither true nor false: its negation holds nowhere in `f`'s arguments.
        assert_eq!(
            texts(
                json!({ "kind": "identifier", "inside": { "kind": "arguments" }, "not": { "nthChild": of_rule } }),
                source,
            ),
            ["c"]
        );
    }

    #[test]
    fn nth_child_shapes_are_checked() {
        for (value, expected) in [
            (
                json!("2x"),
                "/clauses/0/forbid/ast/rule/nthChild: illegal character `x`",
            ),
            (
                json!("n+"),
                "/clauses/0/forbid/ast/rule/nthChild: invalid syntax",
            ),
            (
                json!(-1),
                "/clauses/0/forbid/ast/rule/nthChild must be a non-negative integer, an An+B string, or { position, ofRule, reverse }",
            ),
            (
                json!({ "ofRule": { "kind": "identifier" } }),
                "/clauses/0/forbid/ast/rule/nthChild/position is required",
            ),
            (
                json!({ "position": 1, "of": {} }),
                "unknown field `of` at /clauses/0/forbid/ast/rule/nthChild",
            ),
            (
                json!({ "position": 1, "reverse": 1 }),
                "/clauses/0/forbid/ast/rule/nthChild/reverse must be a boolean",
            ),
            (
                json!({ "position": "x" }),
                "/clauses/0/forbid/ast/rule/nthChild/position: illegal character `x`",
            ),
            (
                json!({ "position": 1, "ofRule": { "kinds": "identifier" } }),
                "unknown field `kinds` at /clauses/0/forbid/ast/rule/nthChild/ofRule",
            ),
        ] {
            let rule = json!({ "kind": "identifier", "nthChild": value });
            assert_eq!(
                rejection(compile_typescript_rule(rule.clone())),
                expected,
                "{rule}"
            );
        }
    }

    #[test]
    fn extreme_offsets_evaluate_without_overflowing() {
        let source = "a;\nb;\n";
        assert_eq!(
            texts(
                json!({ "kind": "expression_statement", "nthChild": "n-2147483647" }),
                source
            ),
            ["a;", "b;"]
        );
        assert_eq!(
            texts(
                json!({ "kind": "expression_statement", "nthChild": "-n-2147483647" }),
                source
            ),
            Vec::<String>::new()
        );
    }
