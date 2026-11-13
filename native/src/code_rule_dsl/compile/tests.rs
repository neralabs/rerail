    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    #[test]
    fn a_planned_language_or_capability_is_language_not_supported() {
        let capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
        let forbid = |language: &str| json!({ "id": "c", "forbid": { "ast": { "language": language, "rule": { "kind": "identifier" } } }, "violation": violation() });
        for language in crate::code_rule_dsl::language::PLANNED_LANGUAGES {
            assert_eq!(
                rejection(compile_document_with(
                    clause_document(forbid(language)),
                    &capabilities
                )),
                format!(
                    "language-not-supported: /clauses/0/forbid/ast/language: `{language}` is not supported yet"
                )
            );
        }
        let mut defined = clause_document(forbid("python"));
        defined["clauses"] = json!([
            { "id": "d", "define": { "language": "exemplar", "utils": { "u": { "kind": "identifier" } } } },
            forbid("python")
        ]);
        assert_eq!(
            rejection(compile_document_with(defined, &capabilities)),
            "language-not-supported: /clauses/0/define/language: `exemplar` is not supported yet"
        );
        assert_eq!(
            super::unsupported_semantic(
                super::Support::NotSupportedYet,
                "/r/calls",
                "calls",
                "typescript"
            )
            .unwrap_err()
            .reason,
            "language-not-supported: /r/calls: `calls` is not supported yet for typescript policies"
        );
        assert_eq!(
            super::unsupported_semantic(super::Support::Refused, "/r/calls", "calls", "typescript")
                .unwrap_err()
                .reason,
            "/r/calls: `calls` is not available for typescript policies"
        );
    }

    #[test]
    fn duplicate_keys_are_rejected_in_json_and_yaml() {
        // A JSON object keeps the last of two equal keys, so the first term vanished silently.
        // ast-grep 0.45.3 rejects the rule ("duplicate field `not`").
        let json_source = r#"{"format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": [{
            "id": "c",
            "forbid": {"ast": {"language": "python", "rule":
                {"kind": "identifier", "not": {"regex": "^a$"}, "not": {"regex": "^b$"}}}},
            "violation": {"anchor": "witness.span", "code": "X", "message": "x"}}]}"#;
        let reason = rejection(CodeRuleDslRuntime::compile(
            json_source.to_owned(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(
            reason.starts_with("duplicate key `not` at /clauses/0/forbid/ast/rule at line 4"),
            "{reason}"
        );
        let yaml_source = "format: rerail.policy/v1\nid: p\nrevision: 1\nid: q\nclauses: []\n";
        let reason = rejection(CodeRuleDslRuntime::compile(
            yaml_source.to_owned(),
            "yaml".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(
            reason.starts_with("invalid YAML: duplicate key `id` at /"),
            "{reason}"
        );
        // Equal keys in different objects are fine.
        let distinct = r#"{"format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": [{
            "id": "c",
            "forbid": {"ast": {"language": "python", "rule":
                {"kind": "identifier", "not": {"regex": "^a$", "not": {"regex": "^b$"}}}}},
            "violation": {"anchor": "witness.span", "code": "X", "message": "x"}}]}"#;
        CodeRuleDslRuntime::compile(
            distinct.to_owned(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }

    #[test]
    fn violation_variables_are_validated() {
        let forbid = json!({ "regex": { "source": "file.content", "pattern": "x" } });
        let compile = |violation: Value| {
            compile_document_with(
                clause_document(json!({ "id": "c", "forbid": forbid, "violation": violation })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let mut variables = violation();
        variables["variables"] = json!({ "name": "x" });
        assert!(rejection(compile(variables)).starts_with(
            "/clauses/0/violation/variables: violation variables are not supported yet"
        ));
        let mut empty = violation();
        empty["variables"] = json!({});
        assert!(compile(empty).is_ok());
    }

    fn regex_document(violation: Value) -> Value {
        clause_document(json!({
            "id": "c",
            "forbid": { "regex": { "source": "file.content", "pattern": "x" } },
            "violation": violation,
        }))
    }

    #[test]
    fn mode_is_required_and_severity_is_unknown() {
        let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
        let mut missing = regex_document(violation());
        missing.as_object_mut().expect("document").remove("mode");
        let mut non_string = regex_document(violation());
        non_string["mode"] = json!(1);
        let mut severity_name = regex_document(violation());
        severity_name["mode"] = json!("advisory");
        for document in [missing, non_string, severity_name] {
            assert_eq!(
                rejection(compile_document_with(document, &capabilities)),
                "/mode must equal log, steer, or block"
            );
        }
        let mut with_severity = violation();
        with_severity["severity"] = json!("error");
        assert_eq!(
            rejection(compile_document_with(
                regex_document(with_severity),
                &capabilities
            )),
            "unknown field `severity` at /clauses/0/violation"
        );
    }

    #[test]
    fn violations_carry_the_rule_mode() {
        let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
        for mode in ["log", "steer", "block"] {
            let mut document = regex_document(violation());
            document["mode"] = json!(mode);
            let compiled = compile_document_with(document, &capabilities)
                .unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "typescript", "a.ts", "x;\n");
            assert_violations(&result, 1);
            assert_eq!(result["violations"][0]["mode"], mode);
            assert!(result["violations"][0].get("severity").is_none());
        }
    }

    /// A constraint may name what a call binds through a cycle of parameterized calls, whichever
    /// `any` branch the walk collecting the query's metavariables meets first.
    #[test]
    fn constraints_on_captures_bound_through_recursive_calls_do_not_depend_on_branch_order() {
        for branches in recursive_call_branch_orders() {
            if let Err(error) = compile_typescript_ast(json!({
                "rule": { "kind": "expression_statement", "any": branches },
                "constraints": { "X": { "regex": "^g" } },
                "utils": mutually_recursive_calls(),
            })) {
                panic!("{branches}: {}", error.reason);
            }
        }
    }
