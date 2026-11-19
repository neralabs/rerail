    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn document(clauses: Value) -> Value {
        json!({ "format": "rerail.policy/v1", "id": "shared", "revision": 1, "mode": "steer", "clauses": clauses })
    }

    fn query(id: &str, language: &str, ast: Value) -> Value {
        let mut ast = ast;
        ast["language"] = json!(language);
        json!({ "id": id, "forbid": { "ast": ast }, "violation": violation() })
    }

    fn compile(clauses: Value) -> napi::Result<crate::code_rule_dsl::CompiledCodeRuleDsl> {
        compile_document_with(
            document(clauses),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    fn clause_ids(result: &Value) -> Vec<String> {
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["clauseId"].as_str().expect("check id").to_owned())
            .collect()
    }

    #[test]
    fn a_define_shares_its_utilities_with_every_query_of_its_language() {
        let compiled = compile(json!([
            { "id": "shared", "define": { "language": "typescript", "utils": {
                "call-of": { "kind": "call_expression", "has": { "field": "function", "matches": "named-f" } },
                "named-f": { "kind": "identifier", "regex": "^f$" },
            } } },
            query("statement", "typescript", json!({ "rule": { "kind": "expression_statement", "has": { "matches": "call-of" } } })),
            query("argument", "typescript", json!({
                "rule": { "kind": "arguments", "has": { "matches": "local" } },
                "utils": { "local": { "matches": "call-of" } },
            })),
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "f(f());\ng();\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(clause_ids(&result), ["statement", "argument"], "{result}");
    }

    #[test]
    fn a_shared_utility_compiles_with_each_querys_options() {
        let util = json!({ "kind": "identifier", "regex": "^window$", "resolves": { "global": ["window"] } });
        let compiled = compile(json!([
            { "id": "shared", "define": { "language": "typescript", "utils": { "window": util } } },
            query("declared", "typescript", json!({
                "rule": { "matches": "window" },
                "languageOptions": { "globals": { "window": "readonly" } },
            })),
            query("undeclared", "typescript", json!({ "rule": { "matches": "window" }, "languageOptions": {} })),
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "window.x;\n");
        assert_eq!(clause_ids(&result), ["declared"], "{result}");
    }

    #[test]
    fn queries_sharing_a_define_each_read_only_the_utilities_they_reach() {
        let define = json!({ "id": "shared", "define": { "language": "typescript", "utils": {
            "named": { "kind": "identifier", "pattern": "$NAME" },
            "number": { "kind": "number" },
        } } });
        let named = query(
            "named",
            "typescript",
            json!({ "rule": { "matches": "named" } }),
        );
        let compared = query(
            "compared",
            "typescript",
            json!({ "rule": {
            "kind": "binary_expression",
            "has": { "matches": "number" },
            "compare": { "left": "NAME", "op": "eq", "right": "NAME" },
        } }),
        );
        // `named` binds `$NAME` for the query that reaches it, not for one compiled after it.
        assert_eq!(
            rejection(compile(json!([define, named, compared]))),
            "/clauses/2/forbid/ast/rule/compare/left: `$NAME` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        let number = query(
            "number",
            "typescript",
            json!({ "rule": { "matches": "number" } }),
        );
        let compiled = compile(json!([define, named, number]))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "f(1);\n");
        assert_eq!(clause_ids(&result), ["named", "number"], "{result}");
    }

    #[test]
    fn a_shared_utility_compiles_alike_whichever_query_reaches_it_first() {
        let define = json!({ "id": "shared", "define": { "language": "typescript", "utils": {
            "bind": { "params": ["X"], "rule": { "kind": "identifier", "pattern": "$X" } },
            "call": { "kind": "call_expression", "all": [
                { "has": { "matches": { "bind": { "X": "$N" } } } },
                { "has": { "kind": "identifier", "pattern": "$M" }, "compare": { "left": "N", "op": "eq", "right": "M" } },
                { "has": { "kind": "identifier", "pattern": "$N" } },
            ] },
        } } });
        let name = query(
            "name",
            "typescript",
            json!({ "rule": { "matches": { "bind": { "X": "$A" } } } }),
        );
        let call = query(
            "call",
            "typescript",
            json!({ "rule": { "matches": "call" } }),
        );
        // A query with a utility of its own compiles the shared ones in a table of its own.
        let own = query(
            "call",
            "typescript",
            json!({ "rule": { "matches": "local" }, "utils": { "local": { "matches": "call" } } }),
        );
        for clauses in [
            json!([define, call]),
            json!([define, name, call]),
            json!([define, call, name]),
            json!([define, name, own]),
        ] {
            let compiled = compile(clauses.clone())
                .unwrap_or_else(|error| panic!("{clauses}: {}", error.reason));
            let result = evaluate_file(&compiled, "typescript", "a.ts", "f(1);\n");
            let mut ids = clause_ids(&result);
            ids.retain(|id| id == "call");
            assert_eq!(ids, ["call"], "{clauses}: {result}");
        }
    }

    #[test]
    fn a_shared_utilitys_all_order_conflict_is_settled_by_each_query_that_reaches_it() {
        let define = json!({ "id": "shared", "define": { "language": "typescript", "utils": {
            "same": { "all": [
                { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } }, "compare": { "left": "A", "op": "eq", "right": "F" } },
                { "has": { "field": "function", "pattern": "$F" } },
            ] },
        } } });
        let bound = query(
            "bound",
            "typescript",
            json!({ "rule": { "kind": "call_expression", "has": { "field": "function", "pattern": "$F" }, "matches": "same" } }),
        );
        let unbound = query(
            "unbound",
            "typescript",
            json!({ "rule": { "matches": "same" } }),
        );
        let compiled =
            compile(json!([define, bound])).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "f(f);\ng(f);\n");
        assert_eq!(clause_ids(&result), ["bound"], "{result}");
        for clauses in [
            json!([define, bound, unbound]),
            json!([define, unbound, bound]),
        ] {
            assert_eq!(
                rejection(compile(clauses)),
                "/clauses/0/define/utils/same/all/0 reads `$F`, which only /clauses/0/define/utils/same/all/1 binds, after it: an `all` runs its items in the order written; move all/1 before all/0"
            );
        }
    }

    #[test]
    fn a_shared_utility_with_a_calls_term_reports_as_if_each_query_listed_it() {
        let sleeps = json!({ "kind": "call", "calls": { "has": {
            "stopBy": "end", "kind": "call", "has": { "field": "function", "regex": "^sleep$" }
        } } });
        let define = json!({ "id": "shared", "define": { "language": "python", "utils": { "sleeps": sleeps } } });
        // Two queries the shared table would serve (the second finds it given up), and one that
        // lists the utility itself.
        let first = query(
            "first",
            "python",
            json!({ "rule": { "matches": "sleeps" } }),
        );
        let second = query(
            "second",
            "python",
            json!({ "rule": { "matches": "sleeps" } }),
        );
        let listed = query(
            "listed",
            "python",
            json!({ "rule": { "matches": "own" }, "utils": { "own": sleeps } }),
        );
        let compiled = compile(json!([define, first, second, listed]))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "def pause():\n    sleep(1)\n\ndef rest():\n    pass\n\npause()\nrest()\n";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        // Each reports the one call that reaches `sleep`: `pause()`, not `rest()` or `sleep(1)`.
        let mut ids = clause_ids(&result);
        ids.sort();
        assert_eq!(ids, ["first", "listed", "second"], "{result}");
    }

    #[test]
    fn a_shared_utility_is_invisible_to_another_language_and_reports_nothing() {
        let define = json!({ "id": "shared", "define": { "language": "typescript", "utils": { "u": { "kind": "identifier" } } } });
        assert_eq!(
            rejection(compile(json!([
                define,
                query("py", "python", json!({ "rule": { "matches": "u" } }))
            ]))),
            "/clauses/1/forbid/ast/rule/matches: utility rule `u` is not defined in this query's utils or a define of its language"
        );
        assert_eq!(
            rejection(compile(json!([define]))),
            "/clauses must hold a clause besides define: a define reports nothing"
        );
    }

    #[test]
    fn a_shared_utility_is_refused_semantics_its_language_lacks_used_or_not() {
        let ts = |util: Value| json!({ "id": "shared", "define": { "language": "typescript", "utils": { "u": util } } });
        let calls =
            json!({ "kind": "call_expression", "calls": { "kind": "function_declaration" } });
        assert_eq!(
            rejection(compile(json!([
                ts(calls.clone()),
                query(
                    "q",
                    "typescript",
                    json!({ "rule": { "kind": "identifier" } })
                )
            ]))),
            "/clauses/0/define/utils/u/calls: `calls` is not available for typescript policies"
        );
        assert_eq!(
            rejection(compile(json!([
                ts(calls),
                query("q", "typescript", json!({ "rule": { "matches": "u" } }))
            ]))),
            "/clauses/0/define/utils/u/calls: `calls` is not available for typescript policies"
        );
    }

    #[test]
    fn defines_are_checked_where_they_are_written() {
        let ts = |utils: Value| json!({ "id": "shared", "define": { "language": "typescript", "utils": utils } });
        let rule = query(
            "q",
            "typescript",
            json!({ "rule": { "kind": "identifier" } }),
        );
        for (clauses, reason) in [
            (
                json!([{ "id": "d", "define": { "language": "typescript", "utils": {} }, "violation": violation() }, rule]),
                "/clauses/0/violation: a define clause reports nothing, so it has no violation",
            ),
            (
                json!([{ "id": "d", "define": { "utils": { "u": { "kind": "identifier" } } } }, rule]),
                "/clauses/0/define/language is required: utilities compile for one language",
            ),
            (
                json!([{ "id": "d", "define": { "language": "ruby", "utils": {} } }, rule]),
                "/clauses/0/define/language must equal python, typescript, javascript or php",
            ),
            (
                json!([{ "id": "d", "define": {} }, rule]),
                "/clauses/0/define must hold utils, options, config or report_when",
            ),
            (
                json!([{ "id": "d", "define": { "language": "typescript", "utils": {}, "rules": {} } }, rule]),
                "unknown field `rules` at /clauses/0/define",
            ),
            (
                json!([{ "id": "d", "define": { "language": "typescript", "utils": {} }, "forbid": {} }, rule]),
                "/clauses/0 must contain exactly one constraint form",
            ),
            (
                json!([ts(json!({ "u": { "kind": "identifier" } })), { "id": "e", "define": { "language": "typescript", "utils": { "u": { "kind": "number" } } } }, rule]),
                "/clauses/1/define/utils/u: utility rule `u` is already defined by another define of typescript",
            ),
            (
                json!([ts(json!({ "u": { "kind": "no_such_kind" } })), rule]),
                "/clauses/0/define/utils/u/kind: Kind `no_such_kind` is invalid. (typescript, tsx, javascript grammars)",
            ),
            (
                json!([ts(json!({ "u": { "not": { "matches": "u" } } })), rule]),
                "/clauses/0/define/utils/u: utility rule `u` has a cyclic dependency in its `matches` sub-rule",
            ),
            (
                json!([
                    ts(json!({ "u": { "matches": "local" } })),
                    query(
                        "q",
                        "typescript",
                        json!({ "rule": { "matches": "u" }, "utils": { "local": { "kind": "identifier" } } })
                    )
                ]),
                "/clauses/0/define/utils/u/matches: utility rule `local` is not defined in this query's utils or a define of its language",
            ),
            (
                json!([
                    ts(json!({ "u": { "kind": "identifier" } })),
                    query(
                        "q",
                        "typescript",
                        json!({ "rule": { "matches": "u" }, "utils": { "u": { "kind": "number" } } })
                    )
                ]),
                "/clauses/1/forbid/ast/utils/u: utility rule `u` is also defined by a define of typescript; rename one",
            ),
            (
                json!([ts(json!({ "u": { "kind": "identifier" } })), { "id": "shared", "forbid": { "regex": { "source": "file.content", "pattern": "x" } }, "violation": violation() }]),
                "duplicate clause id `shared` at /clauses/1",
            ),
        ] {
            assert_eq!(rejection(compile(clauses)), reason);
        }
        // A utility only one dialect can express is shared like a query's own.
        compile(json!([
            ts(json!({ "jsx": { "kind": "jsx_self_closing_element" } })),
            rule
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }

    #[test]
    fn shared_utilities_recurse_and_a_query_compiles_only_those_it_reaches() {
        let compiled = compile(json!([
            { "id": "shared", "define": { "language": "typescript", "utils": {
                "tail-await": { "any": [
                    { "kind": "await_expression" },
                    { "kind": "parenthesized_expression", "has": { "matches": "tail-await" } },
                ] },
                "jsx-only": { "kind": "jsx_self_closing_element" },
            } } },
            query("tail", "typescript", json!({ "rule": { "kind": "return_statement", "has": { "matches": "tail-await" } } })),
        ]))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "async function f() { return ((await x)); }\n";
        assert_violations(&evaluate_file(&compiled, "typescript", "a.ts", source), 1);
        let grammar = &grammar_matchers(&compiled)[0];
        let names = &grammar.utils.names;
        let reached = grammar.query_utils.iter().map(|&index| &names[index]);
        assert_eq!(reached.collect::<Vec<_>>(), ["tail-await"]);
        let filled = (0..names.len()).filter(|&index| grammar.utils.rule(index).is_some());
        assert_eq!(
            filled.map(|index| &names[index]).collect::<Vec<_>>(),
            ["tail-await"]
        );
    }
