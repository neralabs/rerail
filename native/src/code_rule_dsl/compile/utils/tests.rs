    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn same_node_util_cycles_are_rejected_and_relation_recursion_is_not() {
        for (utils, cycle) in [
            (json!({ "a": { "matches": "a" } }), "a"),
            (
                json!({ "a": { "all": [{ "kind": "number" }, { "matches": "b" }] }, "b": { "any": [{ "matches": "a" }] } }),
                "a",
            ),
            (json!({ "a": { "not": { "matches": "a" } } }), "a"),
        ] {
            let reason = rejection(compile_typescript_ast(json!({
                "rule": { "matches": "a" },
                "utils": utils,
            })));
            assert_eq!(
                reason,
                format!(
                    "/clauses/0/forbid/ast/utils/{cycle}: utility rule `{cycle}` has a cyclic dependency in its `matches` sub-rule"
                )
            );
        }
        for relation in ["has", "inside", "precedes", "follows"] {
            compiled(json!({
                "rule": { "matches": "a" },
                "utils": { "a": { "kind": "number", relation: { "matches": "a" } } },
            }));
        }
    }

    #[test]
    fn matches_must_name_a_defined_utility() {
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "kind": "number", "not": { "matches": "missing" } } })
            )),
            "/clauses/0/forbid/ast/rule/not/matches: utility rule `missing` is not defined in this query's utils or a define of its language"
        );
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "matches": { "wrap": { "INNER": { "kind": "number" } } } },
                "utils": { "wrap": { "kind": "number" } },
            }))),
            "/clauses/0/forbid/ast/rule/matches: utility rule `wrap` takes no parameters; name it (\"matches\": \"wrap\")"
        );
        assert_eq!(
            rejection(compile_typescript_ast(json!({ "rule": { "matches": 3 } }))),
            "/clauses/0/forbid/ast/rule/matches must be the id of a utility rule"
        );
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "kind": "number" }, "utils": { "a(b)": { "kind": "number" } } })
            )),
            "/clauses/0/forbid/ast/utils: invalid utility rule id `a(b)`"
        );
        // Unused utilities are still validated, as upstream compiles every one.
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "kind": "number" }, "utils": { "a": { "stopBy": "end" } } })
            )),
            "unknown field `stopBy` at /clauses/0/forbid/ast/utils/a"
        );
    }

    #[test]
    fn constraints_must_name_a_single_node_capture() {
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "pattern": "$F()" }, "constraints": { "G": { "kind": "identifier" } } })
            )),
            "/clauses/0/forbid/ast/constraints/G: metavariable `$G` is not defined by the rule, its utils, or its constraints"
        );
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "pattern": "f($$$ARGS)" }, "constraints": { "ARGS": { "kind": "number" } } })
            )),
            "/clauses/0/forbid/ast/constraints/ARGS: `$$$ARGS` captures a node list; constraints apply to single-node captures only"
        );
        // Defined by a utility or by another constraint, even under `not`, is defined.
        compiled(json!({
            "rule": { "kind": "call_expression", "matches": "callee" },
            "utils": { "callee": { "has": { "kind": "identifier", "pattern": "$F" } } },
            "constraints": { "F": { "pattern": "$G" }, "G": { "not": { "regex": "^_" } } },
        }));
    }

    #[test]
    fn utils_and_constraints_compile_per_dialect() {
        // A JSX-only utility never matches in plain TypeScript and the rest of the rule stays live.
        let rule = compiled(json!({
            "rule": { "any": [{ "matches": "jsx" }, { "pattern": "g($A)" }] },
            "utils": { "jsx": { "kind": "jsx_self_closing_element" } },
        }));
        assert_violations(&evaluate_file(&rule, "typescript", "a.ts", "g(1);\n"), 1);
        assert_violations(
            &evaluate_file(&rule, "tsx", "a.tsx", "const v = <br />;\ng(1);\n"),
            2,
        );

        // A utility or constraint no dialect can express is an error, even when it is unused.
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "kind": "number" },
                "utils": { "bad": { "kind": "no_such_kind" } },
            }))),
            "/clauses/0/forbid/ast/utils/bad/kind: Kind `no_such_kind` is invalid. (typescript, tsx, javascript grammars)"
        );
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "pattern": "$F()" },
                "constraints": { "F": { "kind": "no_such_kind" } },
            }))),
            "/clauses/0/forbid/ast/constraints/F/kind: Kind `no_such_kind` is invalid. (typescript, tsx, javascript grammars)"
        );
    }
