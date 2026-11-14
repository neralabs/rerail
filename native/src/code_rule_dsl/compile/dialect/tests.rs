    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn jsx_rules_match_tsx_and_stay_inert_for_plain_ecmascript() {
        for rule in [
            json!({ "kind": "jsx_opening_element" }),
            json!({ "pattern": "<button $$$A>$$$B</button>" }),
        ] {
            let compiled = compile_typescript_rule(rule.clone())
                .unwrap_or_else(|error| panic!("{rule} compiles: {}", error.reason));
            assert_violations(&evaluate_file(&compiled, "tsx", "view.tsx", JSX_SOURCE), 1);
            assert_violations(
                &evaluate_file(
                    &compiled,
                    "typescript",
                    "plain.ts",
                    "export const n: number = 1;\n",
                ),
                0,
            );
            assert_violations(
                &evaluate_file(&compiled, "javascript", "plain.js", "const n = 1;\n"),
                0,
            );
        }
    }

    #[test]
    fn rules_every_dialect_rejects_name_each_grammar() {
        let error = compile_typescript_rule(json!({ "kind": "not_a_real_kind" }))
            .err()
            .expect("unknown kind is rejected");
        assert!(
            error.reason.contains("/clauses/0/forbid/ast/rule/kind"),
            "{}",
            error.reason
        );
        assert!(error.reason.contains("not_a_real_kind"), "{}", error.reason);
        assert!(
            error
                .reason
                .ends_with("(typescript, tsx, javascript grammars)"),
            "{}",
            error.reason
        );

        let error = compile_typescript_rule(
            json!({ "kind": "jsx_opening_element", "has": { "kind": "type_assertion" } }),
        )
        .err()
        .expect("a rule mixing JSX and angle-bracket assertions is rejected");
        assert_eq!(
            error.reason,
            "/clauses/0/forbid/ast/rule/kind: Kind `jsx_opening_element` is invalid. (typescript grammar); \
             /clauses/0/forbid/ast/rule/has/kind: Kind `type_assertion` is invalid. (tsx grammar); \
             /clauses/0/forbid/ast/rule/has/kind: Kind `type_assertion` is invalid. (javascript grammar)"
        );
    }

    #[test]
    fn typescript_rules_match_every_ecmascript_dialect_where_expressible() {
        for rule in [
            json!({ "kind": "return_statement" }),
            json!({ "pattern": "const $A: any = $B" }),
        ] {
            let compiled = compile_typescript_rule(rule.clone())
                .unwrap_or_else(|error| panic!("{rule} compiles: {}", error.reason));
            let source = "export function f(x: unknown) {\n  const y: any = x;\n  return y;\n}\n";
            assert_violations(
                &evaluate_file(&compiled, "typescript", "plain.ts", source),
                1,
            );
            assert_violations(&evaluate_file(&compiled, "tsx", "view.tsx", source), 1);
        }
        let compiled = compile_typescript_rule(json!({ "kind": "return_statement" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_violations(
            &evaluate_file(
                &compiled,
                "javascript",
                "plain.js",
                "export function f(x) { return x; }\n",
            ),
            1,
        );
    }

    #[test]
    fn php_rules_compile_and_match_php_only_sources() {
        let document = clause_document(json!({
            "id": "php-match",
            "forbid": {
                "ast": {
                    "language": "php",
                    "rule": { "pattern": "echo $VALUE;" }
                }
            },
            "violation": violation()
        }));
        let capabilities = serde_json::from_str(CAPABILITIES).expect("capabilities");
        let compiled = compile_document_with(document, &capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_violations(
            &evaluate_file(&compiled, "php", "sample.php", "<?php\necho $value;\n"),
            1,
        );
    }

    #[test]
    fn a_sub_rule_one_dialect_cannot_express_never_matches_there() {
        let plain = "export function f(x: number) {\n  return g(x);\n}\n";
        // Under `not`: a JSX construct cannot occur in a .ts file, so the negation holds.
        let compiled = compile_typescript_rule(json!({
            "kind": "call_expression",
            "not": { "inside": { "kind": "jsx_expression", "stopBy": "end" } },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_violations(
            &evaluate_file(&compiled, "typescript", "plain.ts", plain),
            1,
        );
        let jsx = "export const V = () => <div>{g(1)}</div>;\nh(2);\n";
        assert_violations(&evaluate_file(&compiled, "tsx", "view.tsx", jsx), 1);

        // In one `any` branch: the other branch still works for .ts files.
        let compiled = compile_typescript_rule(json!({
            "any": [{ "kind": "jsx_attribute" }, { "pattern": "g($A)" }],
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_violations(
            &evaluate_file(&compiled, "typescript", "plain.ts", plain),
            1,
        );
        assert_violations(
            &evaluate_file(
                &compiled,
                "tsx",
                "view.tsx",
                "export const V = () => <b id=\"x\">{g(1)}</b>;\n",
            ),
            2,
        );

        // Positive position: inert in the plain dialect, live in tsx.
        let compiled = compile_typescript_rule(json!({
            "kind": "call_expression",
            "inside": { "kind": "jsx_expression", "stopBy": "end" },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_violations(
            &evaluate_file(&compiled, "typescript", "plain.ts", plain),
            0,
        );
        assert_violations(&evaluate_file(&compiled, "tsx", "view.tsx", jsx), 1);

        // A rule no dialect can express in full is still an error naming each dialect.
        let reason = rejection(compile_typescript_rule(json!({
            "any": [{ "kind": "jsx_attribute" }, { "kind": "type_assertion" }],
        })));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/any/0/kind: Kind `jsx_attribute` is invalid. (typescript grammar); \
             /clauses/0/forbid/ast/rule/any/1/kind: Kind `type_assertion` is invalid. (tsx grammar); \
             /clauses/0/forbid/ast/rule/any/1/kind: Kind `type_assertion` is invalid. (javascript grammar)"
        );
    }

    #[test]
    fn a_policy_targets_rr_files_through_the_shipped_capabilities() {
        // `capabilities.json` names `rr` (`native:rr`), so a clause over `.rr` files compiles
        // against the engine's own tree (decision 0061) and reads its kinds.
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "c",
                "forbid": { "ast": { "language": "rr", "rule": { "kind": "head" } } },
                "violation": violation(),
            })),
            &serde_json::from_str::<serde_json::Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "a(x) :- b(x).\n";
        let result = evaluate_file(&compiled, "rr", "rules.rr", source);
        assert_eq!(matched_texts(&result, source), ["a(x)"], "{result}");
    }

    #[test]
    fn a_policy_targets_json_files_through_the_shipped_capabilities() {
        // `capabilities.json` names `json`, a supported language parsed by its Bitlex grammar in
        // tree-sitter-json's kinds, so a clause over `.json` files compiles and reads them.
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "c",
                "forbid": { "ast": { "language": "json", "rule": { "kind": "pair" } } },
                "violation": violation(),
            })),
            &serde_json::from_str::<serde_json::Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "{\"a\": 1}\n";
        let result = evaluate_file(&compiled, "json", "data.json", source);
        assert_eq!(matched_texts(&result, source), ["\"a\": 1"], "{result}");
    }
