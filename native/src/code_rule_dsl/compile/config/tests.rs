    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn document(options: Value, config: Value, clauses: Value) -> Value {
        let mut all =
            vec![json!({ "id": "options", "define": { "options": options, "config": config } })];
        all.extend(clauses.as_array().expect("checks").iter().cloned());
        json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": all })
    }

    fn query(id: &str, ast: Value) -> Value {
        let mut ast = ast;
        ast["language"] = json!("typescript");
        json!({ "id": id, "forbid": { "ast": ast }, "violation": violation() })
    }

    fn compile(document: Value) -> napi::Result<crate::code_rule_dsl::CompiledCodeRuleDsl> {
        compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    fn clause_ids(document: Value, source: &str) -> Vec<String> {
        let compiled = compile(document).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                let span = &violation["span"];
                let (start, end) = (
                    span["startByte"].as_u64().expect("start"),
                    span["endByte"].as_u64().expect("end"),
                );
                format!(
                    "{} {}",
                    violation["clauseId"].as_str().expect("id"),
                    &source[start as usize..end as usize]
                )
            })
            .collect()
    }

    /// no-unused-vars' shape: an ignore pattern that is no filter until set, a clause per `args`
    /// value, and scopes that `vars` narrows.
    fn unused(config: Value) -> Value {
        document(
            json!({
                "vars": { "type": "string", "enum": ["all", "local"], "default": "all" },
                "args": { "type": "string", "enum": ["all", "after-used", "none"], "default": "after-used" },
                "varsIgnorePattern": { "type": "string", "nullable": true, "default": null },
            }),
            config,
            json!([
                query(
                    "vars",
                    json!({
                        "rule": {
                            "kind": "identifier",
                            "inside": { "kind": "variable_declarator", "field": "name" },
                            "all": [
                                { "any": [
                                    { "inside": { "kind": "function_declaration", "stopBy": "end" } },
                                    { "$option": "vars", "cases": { "all": { "kind": "identifier" }, "local": { "regex": "[^\\s\\S]" } } },
                                ] },
                                { "$option": "varsIgnorePattern", "cases": { "null": [] }, "default": [{ "not": { "regex": { "$option": "varsIgnorePattern" } } }], "spread": true },
                            ],
                        },
                    })
                ),
                query(
                    "args",
                    json!({
                        "rule": { "kind": "identifier", "inside": { "kind": "required_parameter" } },
                        "when": { "option": { "args": { "in": ["all", "after-used"] } } },
                    })
                ),
            ]),
        )
    }

    #[test]
    fn options_resolve_references_before_the_document_is_validated() {
        let source = "const a = 1;\nconst _b = 2;\nfunction f(p) { const c = 3; }\n";
        assert_eq!(
            clause_ids(unused(json!({})), source),
            ["vars a", "vars _b", "args p", "vars c"]
        );
        assert_eq!(
            clause_ids(
                unused(json!({ "vars": "local", "varsIgnorePattern": "^_", "args": "none" })),
                source
            ),
            ["vars c"]
        );
        assert_eq!(
            clause_ids(unused(json!({ "varsIgnorePattern": "^_" })), source),
            ["vars a", "args p", "vars c"]
        );
    }

    #[test]
    fn configured_sql_values_are_utf8_text_expressions() {
        let source = json!({
            "clauses": [{"id": "values", "define": {"options": {
                "names": {"type": "array", "items": "string", "default": ["O'Neil.py:build", "x\u{0000}y", "é"]}
            }}}],
            "query": {"$option": "names", "escape": "sql", "map": " UNION ALL SELECT {} AS name", "join": "", "template": "SELECT '' AS name WHERE 0{}"}
        });
        let (resolved, _) = super::resolve(&source).unwrap();
        assert_eq!(resolved["query"], "SELECT '' AS name WHERE 0 UNION ALL SELECT CAST(X'4f274e65696c2e70793a6275696c64' AS TEXT) AS name UNION ALL SELECT CAST(X'780079' AS TEXT) AS name UNION ALL SELECT CAST(X'c3a9' AS TEXT) AS name");
    }

    #[test]
    fn references_escape_map_join_and_fill_templates() {
        let options = json!({
            "allowed": { "type": "array", "default": ["a.b", "c"] },
            "prefix": { "type": "string", "default": "x" },
        });
        let clauses = json!([query(
            "names",
            json!({
                "rule": { "kind": "identifier", "any": [
                    { "regex": { "$option": "allowed", "escape": "regex", "join": "|", "template": "^(?:{})$" } },
                    { "regex": { "$option": "prefix", "template": "^{}_" } },
                ] },
            })
        )]);
        let source = "a.b; ab; c; x_1; y_1;\n";
        assert_eq!(
            clause_ids(
                document(options.clone(), json!({}), clauses.clone()),
                source
            ),
            ["names c", "names x_1"]
        );
        // `map` fills a template per item: here a `scope` list.
        let mut mapped = document(options, json!({ "allowed": ["src", "lib"] }), clauses);
        mapped["scope"] =
            json!({ "include": { "$option": "allowed", "map": "{}/**" }, "exclude": [] });
        let compiled = compile(mapped).unwrap_or_else(|error| panic!("{}", error.reason));
        assert!(
            compiled.in_scope("src/a.ts".to_owned()) && compiled.in_scope("lib/b.ts".to_owned())
        );
        assert!(!compiled.in_scope("test/c.ts".to_owned()));
    }

    #[test]
    fn a_reference_among_the_clauses_spreads_clauses_into_them() {
        let options = json!({ "on": { "type": "boolean", "default": true } });
        let spread = |clauses: Value| json!({ "$option": "on", "cases": { "false": [] }, "default": clauses, "spread": true });
        let class = query("c", json!({ "rule": { "kind": "class_declaration" } }));
        let number = query("n", json!({ "rule": { "kind": "number" } }));
        let source = "class A {}\n1;\n";
        let on = document(options.clone(), json!({}), json!([spread(json!([class]))]));
        assert_eq!(clause_ids(on, source), ["c class A {}"]);
        let both = |config: Value| {
            document(
                options.clone(),
                config,
                json!([spread(json!([class])), number]),
            )
        };
        assert_eq!(clause_ids(both(json!({})), source), ["c class A {}", "n 1"]);
        assert_eq!(clause_ids(both(json!({ "on": false })), source), ["n 1"]);
        // A reference that stands for one clause, unspread, is that clause.
        let single = document(
            options.clone(),
            json!({}),
            json!([{ "$option": "on", "cases": { "true": class, "false": number } }]),
        );
        assert_eq!(clause_ids(single, source), ["c class A {}"]);
        for (document, reason) in [
            (
                document(
                    options.clone(),
                    json!({ "on": false }),
                    json!([spread(json!([class]))]),
                ),
                "/clauses must hold a clause besides define: a define reports nothing",
            ),
            (
                document(
                    options.clone(),
                    json!({}),
                    json!([
                        spread(
                            json!([{ "id": "d", "define": { "options": { "x": { "type": "boolean", "default": true } } } }])
                        ),
                        class,
                    ]),
                ),
                "/clauses/1: a define is written among the clauses, not made by a $option reference",
            ),
            (
                document(
                    options,
                    json!({}),
                    json!([
                        { "$option": "on", "define": { "options": {} } },
                        class,
                    ]),
                ),
                "unknown field `define` at /clauses/1",
            ),
        ] {
            assert_eq!(rejection(compile(document)), reason);
        }
    }

    #[test]
    fn when_option_gates_a_query_on_the_documents_options() {
        let options =
            json!({ "allow": { "type": "array", "enum": ["methods", "arrows"], "default": [] } });
        let clauses = |when: Value| {
            json!([query(
                "q",
                json!({ "rule": { "kind": "number" }, "when": { "option": when } })
            )])
        };
        for (config, when, reports) in [
            (json!({}), json!({ "allow": [] }), true),
            // No condition holds for every value: the neutral gate a reference can choose.
            (json!({}), json!({}), true),
            (
                json!({}),
                json!({ "allow": { "includes": "methods" } }),
                false,
            ),
            (
                json!({ "allow": ["methods"] }),
                json!({ "allow": { "includes": "methods" } }),
                true,
            ),
            (
                json!({ "allow": ["methods"] }),
                json!({ "allow": { "not": { "includes": "methods" } } }),
                false,
            ),
            (
                json!({ "allow": ["arrows"] }),
                json!({ "allow": { "in": [["arrows"], ["methods"]] } }),
                true,
            ),
        ] {
            let ids = clause_ids(document(options.clone(), config, clauses(when)), "1;\n");
            assert_eq!(!ids.is_empty(), reports, "{ids:?}");
        }
    }
    #[test]
    fn integral_fractional_options_select_the_same_case_as_javascript() {
        let options = json!({ "level": { "type": "number", "default": 1.0 } });
        let selected = query(
            "selected",
            json!({
                "rule": {
                    "$option": "level",
                    "cases": { "1": { "kind": "number" } },
                },
            }),
        );
        assert_eq!(
            clause_ids(document(options, json!({}), json!([selected])), "1;\n"),
            ["selected 1"]
        );
    }

    #[test]
    fn integral_integer_options_accept_decimal_and_exponent_forms() {
        let clauses = json!([query("number", json!({ "rule": { "kind": "number" } }))]);
        let scalar = json!({ "level": { "type": "integer", "default": 1.0 } });
        assert_eq!(
            clause_ids(
                document(scalar, json!({ "level": 1e0 }), clauses.clone()),
                "1;\n"
            ),
            ["number 1"]
        );
        let items = json!({ "levels": { "type": "array", "items": "integer", "default": [1.0] } });
        assert_eq!(
            clause_ids(document(items, json!({ "levels": [1e0] }), clauses), "1;\n"),
            ["number 1"]
        );
    }

    #[test]
    fn integer_array_enum_normalizes_its_items() {
        let options = json!({
            "levels": { "type": "array", "items": "integer", "enum": [1, 2], "default": [1] }
        });
        assert_eq!(
            clause_ids(
                document(
                    options,
                    json!({}),
                    json!([query("number", json!({ "rule": { "kind": "number" } }))]),
                ),
                "1;\n",
            ),
            ["number 1"]
        );
    }

    #[test]
    fn nullable_integer_array_keeps_null_for_option_cases() {
        let options = json!({ "levels": { "type": "array", "items": "integer", "nullable": true, "default": null } });
        let selected = query(
            "selected",
            json!({
                "rule": {
                    "$option": "levels",
                    "cases": {
                        "null": { "kind": "number" },
                        "default": { "kind": "identifier" }
                    }
                }
            }),
        );
        assert_eq!(
            clause_ids(document(options, json!({}), json!([selected])), "1;\n"),
            ["selected 1"]
        );
    }

    #[test]
    fn integer_options_reject_out_of_range_decimal_numbers() {
        assert_eq!(
            rejection(compile(document(
                json!({ "level": { "type": "integer", "default": 1e20 } }),
                json!({}),
                json!([query("number", json!({ "rule": { "kind": "number" } }))]),
            ))),
            "/clauses/0/define/options/level/default: 100000000000000000000 is not exactly representable as a JavaScript number"
        );
    }

    #[test]
    fn option_numbers_are_the_javascript_numbers_every_entrypoint_reads() {
        let options = json!({
            "level": { "type": "number", "default": 1.0 },
            "count": { "type": "integer", "enum": [3], "default": 3.0 },
            "tiny": { "type": "number", "default": 1.5e-7 },
            "ratio": { "type": "number", "default": 0.00001 },
            "zero": { "type": "number", "default": -0.0 },
        });
        let cases = |id: &str, option: &str, key: &str| {
            query(
                id,
                json!({ "rule": { "$option": option, "cases": { key: { "kind": "number" } } } }),
            )
        };
        let clauses = json!([
            cases("case", "level", "1"),
            cases("count", "count", "3"),
            cases("exponent", "tiny", "1.5e-7"),
            cases("zero", "zero", "0"),
            query(
                "text",
                json!({ "rule": { "kind": "number", "regex": { "$option": "ratio", "template": "^{}$" } } })
            ),
            query(
                "version",
                json!({ "rule": { "kind": "identifier", "regex": { "$option": "level", "template": "^v{}$" } } })
            ),
        ]);
        // The defaults as written: `1.0`, `3.0`, `1.5e-7`, `0.00001` and `-0.0` read as
        // JavaScript reads them.
        let mut ids = clause_ids(
            document(options.clone(), json!({}), clauses.clone()),
            "0.00001;\nv1;\n",
        );
        ids.sort();
        assert_eq!(
            ids,
            [
                "case 0.00001",
                "count 0.00001",
                "exponent 0.00001",
                "text 0.00001",
                "version v1",
                "zero 0.00001",
            ]
        );
        // An enum holds `1`, so `1.0` configures it, as a number or an integer.
        for kind in ["number", "integer"] {
            let options = json!({ "level": { "type": kind, "enum": [1, 2], "default": 2 } });
            assert_eq!(
                clause_ids(
                    document(
                        options,
                        json!({ "level": 1.0 }),
                        json!([cases("case", "level", "1")])
                    ),
                    "1;\n"
                ),
                ["case 1"],
                "{kind}"
            );
        }
        let beyond = json!(9_007_199_254_740_993u64);
        let rule = json!([query("q", json!({ "rule": { "kind": "number" } }))]);
        for (options, config, reason) in [
            (
                json!({ "n": { "type": "integer", "default": beyond } }),
                json!({}),
                "/clauses/0/define/options/n/default: 9007199254740993 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "integer", "default": -9_007_199_254_740_992i64 } }),
                json!({}),
                "/clauses/0/define/options/n/default: -9007199254740992 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "array", "items": "integer", "enum": [1, beyond], "default": [] } }),
                json!({}),
                "/clauses/0/define/options/n/enum/1: 9007199254740993 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "integer", "default": 1 } }),
                json!({ "n": beyond }),
                "/clauses/0/define/config/n: 9007199254740993 is not exactly representable as a JavaScript number",
            ),
            (
                json!({ "n": { "type": "number", "default": 1e21 } }),
                json!({}),
                "/clauses/0/define/options/n/default: 1e+21 is not exactly representable as a JavaScript number",
            ),
        ] {
            assert_eq!(
                rejection(compile(document(options, config, rule.clone()))),
                reason
            );
        }
    }

    #[test]
    fn options_and_references_are_checked_where_they_are_written() {
        let rule = query("q", json!({ "rule": { "kind": "number" } }));
        let string = json!({ "s": { "type": "string", "default": "a" } });
        for (document, reason) in [
            (
                document(
                    json!({ "s": { "type": "text", "default": "a" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/type must equal boolean, string, number, integer or array",
            ),
            (
                document(
                    json!({ "s": { "type": "string" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default is required",
            ),
            (
                document(
                    json!({ "s": { "type": "string", "default": 1 } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default is not a value of the option's type",
            ),
            (
                document(
                    json!({ "s": { "type": "string", "enum": ["a"], "default": "b" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default is not one of the option's enum values",
            ),
            (
                document(
                    json!({ "s": { "type": "array", "items": "integer", "default": [1.5] } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default/0 is not an allowed item of the option",
            ),
            (
                document(
                    json!({ "s": { "type": "string", "default": null } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/s/default: null is not allowed; the option is not nullable",
            ),
            (
                document(
                    json!({ "1s": { "type": "string", "default": "a" } }),
                    json!({}),
                    json!([rule]),
                ),
                "/clauses/0/define/options/1s: an option's name starts with a letter or _ and holds letters, digits, _, - and .",
            ),
            (
                document(string.clone(), json!({ "t": "a" }), json!([rule])),
                "/clauses/0/define/config/t: `t` is not an option this document declares",
            ),
            (
                document(string.clone(), json!({ "s": true }), json!([rule])),
                "/clauses/0/define/config/s is not a value of the option's type",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "t" } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/$option: `t` is not an option this document declares",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "spread": true } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/spread: only an array's item may spread",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "cases": { "b": "x" } } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/cases: no case for `a` and no default",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "join": "|" } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/rule/regex/join: the option's value is not an array",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "regex": { "$option": "s", "suffix": "x" } } })
                    )]),
                ),
                "unknown field `suffix` at /clauses/1/forbid/ast/rule/regex",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "kind": "number" }, "when": { "option": { "t": "a" } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/when/option: `t` is not an option this document declares",
            ),
            (
                document(
                    string.clone(),
                    json!({}),
                    json!([query(
                        "q",
                        json!({ "rule": { "kind": "number" }, "when": { "option": { "s": { "startsWith": "a" } } } })
                    )]),
                ),
                "/clauses/1/forbid/ast/when/option/s: unknown condition `startsWith`; expected in, includes or not",
            ),
            (
                json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "steer", "clauses": [
                    { "id": "a", "define": { "options": string } },
                    { "id": "b", "define": { "options": string } },
                    rule,
                ] }),
                "/clauses/1/define/options/s: option `s` is already declared by another define",
            ),
        ] {
            assert_eq!(rejection(compile(document)), reason);
        }
        // An option's own declaration is read as written: a reference there is its default's text.
        compile(document(
            json!({ "s": { "type": "string", "default": "$option" } }),
            json!({}),
            json!([rule]),
        ))
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }
