    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    /// A call's captures are its utility's, mapped at the call: each utility of a diamond chain is
    /// collected a constant number of times, not once per call path.
    #[test]
    fn parameterized_utility_diamonds_collect_captures_in_linear_visits() {
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        reset_walks();
        let mut variables = std::collections::BTreeMap::new();
        crate::code_rule_dsl::matcher::collect_variables(matcher, &mut variables);
        assert!(variables.contains_key("X"), "{:?}", variables.keys());
        let bound = linear_walk(DIAMOND_DEPTH);
        assert!(
            walks(Walk::CaptureVariables) <= bound,
            "capture collection expanded {} utilities (bound {bound})",
            walks(Walk::CaptureVariables)
        );
    }

    #[test]
    fn stop_by_must_be_neighbor_end_or_a_rule() {
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "has": { "kind": "identifier", "stopBy": 3 } }),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/has/stopBy must be \"neighbor\", \"end\" or a rule object"
        );
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "has": { "kind": "identifier", "stopBy": "far" } }),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/has/stopBy has unsupported value `far`; expected \"neighbor\", \"end\" or a rule object"
        );
        // A stop rule is an ast rule: its shape is validated like any sub-rule.
        let reason = rejection(compile_typescript_rule(json!({
            "kind": "identifier",
            "inside": { "kind": "call_expression", "stopBy": { "kind": "program", "field": "body" } },
        })));
        assert_eq!(
            reason,
            "unknown field `field` at /clauses/0/forbid/ast/rule/inside/stopBy"
        );
        let reason = rejection(compile_typescript_rule(json!({
            "kind": "identifier",
            "inside": { "kind": "call_expression", "stopBy": { "kind": "no_such_kind" } },
        })));
        assert!(
            reason.starts_with("/clauses/0/forbid/ast/rule/inside/stopBy/kind: "),
            "{reason}"
        );
    }

    #[test]
    fn non_string_kind_and_regex_are_rejected_anywhere() {
        for (rule, path) in [
            (json!({ "kind": ["identifier"] }), "rule/kind"),
            (
                json!({ "pattern": "foo()", "kind": ["call_expression"] }),
                "rule/kind",
            ),
            (
                json!({ "kind": "call_expression", "has": { "kind": 7 } }),
                "rule/has/kind",
            ),
            (json!({ "any": [{ "regex": ["^a"] }] }), "rule/any/0/regex"),
            (
                json!({ "kind": "identifier", "regex": { "source": "^a" } }),
                "rule/regex",
            ),
            (json!({ "kind": "  " }), "rule/kind"),
        ] {
            let reason = rejection(compile_typescript_rule(rule.clone()));
            assert_eq!(
                reason,
                format!("/clauses/0/forbid/ast/{path} must be a non-empty string"),
                "{rule}"
            );
        }
    }

    #[test]
    fn pattern_byte_limits_apply_to_every_pattern_source() {
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxAstPatternBytes"] = json!(8);
            capabilities["limits"]["maxRegexPatternBytes"] = json!(4);
        });
        let ast = |rule: Value| {
            clause_document(json!({
                "id": "c",
                "forbid": { "ast": { "language": "typescript", "rule": rule } },
                "violation": violation(),
            }))
        };
        assert!(
            compile_document_with(ast(json!({ "pattern": "f($A, 1)" })), &capabilities).is_ok()
        );
        assert!(
            compile_document_with(
                ast(json!({ "kind": "identifier", "regex": "^abc" })),
                &capabilities
            )
            .is_ok()
        );
        for (rule, expected) in [
            (
                json!({ "pattern": "f($A, 10)" }),
                "/clauses/0/forbid/ast/rule/pattern is 9 bytes; capabilities.limits.maxAstPatternBytes allows at most 8",
            ),
            (
                json!({ "pattern": { "context": "class A { x = 1 }", "selector": "public_field_definition" } }),
                "/clauses/0/forbid/ast/rule/pattern/context is 17 bytes; capabilities.limits.maxAstPatternBytes allows at most 8",
            ),
            (
                json!({ "kind": "identifier", "not": { "regex": "^abcd" } }),
                "/clauses/0/forbid/ast/rule/not/regex is 5 bytes; capabilities.limits.maxRegexPatternBytes allows at most 4",
            ),
        ] {
            assert_eq!(
                rejection(compile_document_with(ast(rule), &capabilities)),
                expected
            );
        }
        let regex = clause_document(json!({
            "id": "c",
            "forbid": { "regex": { "source": "file.path", "pattern": "é{2}" } },
            "violation": violation(),
        }));
        assert_eq!(
            rejection(compile_document_with(regex, &capabilities)),
            "/clauses/0/forbid/regex/pattern is 5 bytes; capabilities.limits.maxRegexPatternBytes allows at most 4"
        );
        let missing = capabilities_with(|capabilities| {
            capabilities["limits"]
                .as_object_mut()
                .expect("limits")
                .remove("maxAstPatternBytes");
        });
        assert_eq!(
            rejection(compile_document_with(
                ast(json!({ "kind": "identifier" })),
                &missing
            )),
            "capabilities.limits.maxAstPatternBytes must be a positive integer"
        );
    }

    #[test]
    fn kindless_rules_are_accepted_and_test_stored_tokens() {
        // ast-grep 0.45.3 rejects a rule with no inferable kinds ("Rule must specify a set of AST
        // kinds to match"). Rerail keeps accepting it: it is the only way to report an anonymous
        // token (`kind` names named kinds only), such as `==`, `,` or `default`. Such a rule tests
        // every node the tape holds: a stored token, never one the grammar implies (decision 0161), so
        // the parentheses of `(1)` are not there to report.
        let compiled = compile_typescript_rule(json!({
            "inside": { "kind": "parenthesized_expression" },
            "not": { "kind": "number" },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "(1);\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert!(matched_texts(&result, source).is_empty());
        let operator = compile_typescript_rule(json!({
            "regex": "^==$",
            "inside": { "kind": "binary_expression" },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "a == b;\n";
        let result = evaluate_file(&operator, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["=="]);
    }

    #[test]
    fn abstract_kinds_match_the_nodes_the_parser_reported_as_members() {
        // A supertype (`expression`, `statement`, ...) resolves to a symbol no node carries, so
        // ast-grep's kind test never matches it. Rerail matches the nodes the parser reported as
        // its members, which the tape stores per node (decision 0025): membership is contextual,
        // so the assigned `a` is a `pattern`, not an `expression`, while `b` is one.
        for kind in ["expression", "statement", "declaration", "primary_expression", "pattern"] {
            compile_typescript_rule(json!({ "kind": kind })).unwrap_or_else(|error| panic!("{kind}: {}", error.reason));
        }
        let source = "a = b;\n";
        let expression = compile_typescript_rule(json!({ "kind": "expression" })).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&expression, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["a = b", "b"]);
        let statement = compile_typescript_rule(json!({ "kind": "statement" })).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&statement, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["a = b;"]);
        // Under a relation too.
        let python = clause_document(json!({
            "id": "c",
            "forbid": { "ast": { "language": "python", "rule": { "kind": "assignment", "has": { "kind": "expression" } } } },
            "violation": violation(),
        }));
        compile_document_with(python, &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        // Concrete kinds, including ones named like a keyword, still compile.
        for kind in ["expression_statement", "await_expression", "identifier"] {
            compile_typescript_rule(json!({ "kind": kind }))
                .unwrap_or_else(|error| panic!("{kind}: {}", error.reason));
        }
    }

    #[test]
    fn python_patterns_reject_dollar_names_that_are_not_metavariables() {
        // Python has no `$`, so ast-grep rewrites `$` before an uppercase letter or `_` to `µ`;
        // `$At` becomes the literal identifier `µAt` (names allow A-Z, 0-9, _ only) and the
        // pattern can never match. ast-grep 0.45.3 accepts it silently.
        let python = |pattern: &str| {
            compile_document_with(
                clause_document(json!({
                    "id": "c",
                    "forbid": { "ast": { "language": "python", "rule": { "pattern": pattern } } },
                    "violation": violation(),
                })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        for (pattern, name) in [
            ("time.$At", "$At"),
            ("f($$$Args)", "$$$Args"),
            ("x = '$Name'", "$Name"),
            ("$_x", "$_x"),
        ] {
            assert_eq!(
                rejection(python(pattern)),
                format!(
                    "/clauses/0/forbid/ast/rule/pattern: `{name}` is not a metavariable (names use A-Z, 0-9 and _) and Python source has no `$`, so the pattern never matches (python grammar)"
                ),
                "{pattern}"
            );
        }
        for pattern in ["time.$AT", "f($$$ARGS)", "$_", "f($A1, $$$)"] {
            python(pattern).unwrap_or_else(|error| panic!("{pattern}: {}", error.reason));
        }
        // A `µ` written in the pattern is Python's own identifier character, not a rewritten `$`.
        let compiled = python("µfoo()").unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "µfoo()\nfoo()\n";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(matched_texts(&result, source), ["µfoo()"]);
        assert!(rejection(python("µfoo($At)")).contains("`$At` is not a metavariable"));
        // `$` is part of TypeScript identifiers: `$At` stays the literal identifier, as upstream.
        let compiled = compile_typescript_rule(json!({ "pattern": "time.$At" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "time.sleep(1);\ntime.$At;\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["time.$At"]);
    }

    #[test]
    fn shape_errors_are_reported_once_without_a_grammar() {
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "has": { "kinds": "identifier" } }),
        ));
        assert_eq!(
            reason,
            "unknown field `kinds` at /clauses/0/forbid/ast/rule/has"
        );
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "regex": "(" }),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/regex: Parsing text matcher fails."
        );
        // Grammar-dependent failures keep naming the grammar.
        let reason = rejection(compile_typescript_rule(json!({ "pattern": ")(" })));
        assert!(
            reason.ends_with("(typescript, tsx, javascript grammars)"),
            "{reason}"
        );
    }

    #[test]
    fn c_and_cpp_calls_that_parse_as_declarations_are_refused_with_their_contextual_form() {
        // At file scope C and C++ allow only declarations, so tree-sitter reads `gets($A)` as a
        // C `macro_type_specifier` and `std::atoi($A)` as a C++ declaration of `$A`: neither can
        // match the call a function body holds (#835).
        let compile = |language: &str, pattern: Value| {
            compile_document_with(
                clause_document(json!({
                    "id": "c",
                    "forbid": { "ast": { "language": language, "rule": { "pattern": pattern } } },
                    "violation": violation(),
                })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let matches = |language: &str, pattern: Value, path: &str, source: &str| {
            let compiled = compile(language, pattern.clone())
                .unwrap_or_else(|error| panic!("{language} {pattern}: {}", error.reason));
            matched_texts(&evaluate_file(&compiled, language, path, source), source)
        };
        let c = "void f(char *b) { gets(b); int x = atoi(b); foo.bar(b); x = gets(b); }\n";
        let cpp = "void f(char *b) { std::atoi(b); int x = std::atoi(b); }\n";
        for (language, pattern, read_as, path, source, call) in [
            ("c", "gets($A)", "macro_type_specifier", "a.c", c, "gets(b)"),
            ("c", "$F($A)", "macro_type_specifier", "a.c", c, "gets(b)"),
            ("c", "gets($$$A)", "macro_type_specifier", "a.c", c, "gets(b)"),
            ("c", "gets(b)", "macro_type_specifier", "a.c", c, "gets(b)"),
            ("c", "gets($A)\n", "macro_type_specifier", "a.c", c, "gets(b)"),
            ("c", " gets($A)", "macro_type_specifier", "a.c", c, "gets(b)"),
            ("cpp", "std::atoi($A)", "declaration", "a.cpp", cpp, "std::atoi(b)"),
            ("cpp", "$X::atoi($A)", "declaration", "a.cpp", cpp, "std::atoi(b)"),
        ] {
            let written = pattern.trim();
            let reason = rejection(compile(language, json!(pattern)));
            assert_eq!(
                reason,
                format!(
                    "/clauses/0/forbid/ast/rule/pattern: `{written}` parses as a `{read_as}` at file \
                     scope, not the `call_expression` a function body holds; to match the \
                     `call_expression` write \
                     {{\"context\":\"void f() {{ {written}; }}\",\"selector\":\"call_expression\"}}, \
                     or for the `{read_as}` a `context` that holds it with \"selector\": \
                     \"{read_as}\" ({language} grammar)"
                ),
                "{language} {pattern:?}"
            );
            // The form the refusal names compiles, and matches every call, statement or not.
            let at = reason.find("write ").expect("a contextual form") + "write ".len();
            let contextual = serde_json::Deserializer::from_str(&reason[at..])
                .into_iter::<Value>()
                .next()
                .expect("a contextual form")
                .expect("JSON");
            let texts = matches(language, contextual.clone(), path, source);
            assert_eq!(
                texts.iter().filter(|text| text.starts_with(call)).count(),
                2,
                "{language} {contextual}: {texts:?}"
            );
        }
        for (language, pattern, path, matched) in [
            ("c", json!("gets($A);"), "a.c", "gets(b);"),
            ("c", json!("foo.bar($A)"), "a.c", "foo.bar(b)"),
            ("c", json!("x = gets($A)"), "a.c", "x = gets(b)"),
            ("cpp", json!("atoi($A)"), "a.cpp", "atoi(b)"),
            ("cpp", json!("foo.bar($A)"), "a.cpp", "foo.bar(b)"),
        ] {
            let texts = matches(language, pattern.clone(), path, c);
            assert!(
                texts.iter().any(|text| text.starts_with(matched)),
                "{language} {pattern}: {texts:?}"
            );
        }
        // A macro's type is the other reading: the refusal's `context` with its kind matches it.
        let source = "void f(void) { STACK_OF(X509) *certs; }\n";
        assert!(rejection(compile("c", json!("STACK_OF($T)"))).contains("macro_type_specifier"));
        let pattern = json!({ "context": "STACK_OF($T) *x;", "selector": "macro_type_specifier" });
        let texts = matches("c", pattern, "a.c", source);
        assert!(
            texts.len() == 1 && texts[0].starts_with("STACK_OF(X509)"),
            "{texts:?}"
        );
        // A statement that is no expression keeps its own reading.
        compile("c", json!("if ($A) $B")).unwrap_or_else(|error| panic!("{}", error.reason));
    }

