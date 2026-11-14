    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};
    use std::cell::Cell;
    use std::collections::BTreeSet;

    /// Each call of a parameterized utility binds, under its own caller's name, what the utility
    /// binds: a second call with another mapping makes its capture visible too.
    #[test]
    fn every_call_of_a_parameterized_utility_binds_its_callers_names() {
        let call = |capture: &str| json!({ "bind": { "P": capture } });
        let compiled = compiled(json!({
            "rule": { "kind": "call_expression", "matches": { "same": { "Q": "$F" } } },
            "utils": {
                "bind": { "params": ["P"], "rule": { "pattern": "$P" } },
                "same": { "params": ["Q"], "rule": { "all": [
                    { "has": { "field": "function", "matches": call("$Q") } },
                    { "has": { "field": "arguments", "has": { "kind": "identifier", "matches": call("$X") } } },
                    { "compare": { "left": "X", "op": "eq", "right": "Q", "text": "tokens" } },
                ] } },
            },
        }));
        let source = "f(f);\nf(g);\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&compiled, "typescript", "a.ts", source),
                source
            ),
            ["f(f) F=f"]
        );
    }

    /// A parameterized utility's frame names come from each utility it calls once per walk, so a
    /// diamond chain compiles each of its patterns a constant number of times.
    #[test]
    fn parameterized_utility_diamonds_scope_names_in_linear_pattern_compilations() {
        let utils = parameterized_utility_diamonds(DIAMOND_DEPTH);
        let utils = super::super::utils::util_map(Some(&utils), "/utils").expect("utilities");
        let compilations = Cell::new(0);
        let pattern_names = |pattern: &Value| {
            compilations.set(compilations.get() + 1);
            pattern
                .as_str()
                .and_then(|pattern| pattern.strip_prefix('$'))
                .map(|name| BTreeSet::from([name.to_owned()]))
                .unwrap_or_default()
        };
        let top = &utils[&format!("u{DIAMOND_DEPTH}")];
        let scopes = super::frame_scope(top, &utils, &pattern_names);
        assert!(scopes[0].names.contains("P"));
        let bound = linear_walk(DIAMOND_DEPTH);
        assert!(
            compilations.get() <= bound,
            "frame names compiled {} patterns (bound {bound})",
            compilations.get()
        );
    }

    /// Every `all` order conflict is settled against one index of the query's binding sites: its
    /// patterns compile once per query, not once per conflict, and not at all without a conflict.
    #[test]
    fn all_order_conflicts_compile_binding_sites_once_per_query() {
        let binders = [
            json!({ "has": { "field": "function", "pattern": "$F" } }),
            json!({ "has": { "field": "arguments", "has": { "pattern": "$A", "kind": "identifier" } } }),
        ];
        let compare =
            json!({ "compare": { "left": "F", "op": "eq", "right": "A", "text": "tokens" } });
        // Each utility reads `$F` and `$A` before binding them, which the query's pattern excuses.
        let utilities = DIAMOND_DEPTH;
        let utils = (0..utilities)
            .map(|index| {
                let rule = json!({ "all": [compare, binders[0], binders[1]] });
                (format!("c{index}"), rule)
            })
            .collect::<serde_json::Map<_, _>>();
        reset_walks();
        compiled(json!({ "rule": { "pattern": "$F($A)" }, "utils": utils }));
        let bound = linear_walk(utilities);
        assert!(
            walks(Walk::BindingSitePatterns) <= bound,
            "settling conflicts compiled {} binding-site patterns (bound {bound})",
            walks(Walk::BindingSitePatterns)
        );
        reset_walks();
        compiled(
            json!({ "rule": { "pattern": "$F($A)", "all": [binders[0], binders[1], compare] } }),
        );
        assert_eq!(walks(Walk::BindingSitePatterns), 0);
    }

    #[test]
    fn compare_names_must_be_single_captures_the_query_binds() {
        let reject = |rule: Value| rejection(compile_typescript_ast(json!({ "rule": rule })));
        let with = |compare: Value| json!({ "pattern": "f($A, $$$REST)", "not": { "pattern": "f($N, $$$)" }, "compare": compare });
        assert_eq!(
            reject(with(json!({ "left": "A", "op": "lt", "right": "N" }))),
            "/clauses/0/forbid/ast/rule/compare/right: `$N` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        assert_eq!(
            reject(with(json!({ "left": "REST", "op": "lt", "right": "A" }))),
            "/clauses/0/forbid/ast/rule/compare/left: `$$$REST` captures a node list; compare applies to single-node captures only"
        );
        assert_eq!(
            reject(with(json!({ "left": "$A", "op": "lt", "right": "A" }))),
            "/clauses/0/forbid/ast/rule/compare/left: `$A` is not a metavariable name (A-Z, 0-9, _, without `$`)"
        );
        assert_eq!(
            reject(with(json!({ "left": "A", "right": "A" }))),
            "/clauses/0/forbid/ast/rule/compare/op is required"
        );
        assert_eq!(
            reject(with(
                json!({ "left": "A", "op": "lt", "right": "A", "collation": "locale" })
            )),
            "/clauses/0/forbid/ast/rule/compare/collation: unknown value `locale`; expected one of ordinal, fold-lower, fold-upper, lowercase-first, natural, natural-fold-lower"
        );
        assert_eq!(
            reject(with(
                json!({ "left": "A", "op": "lt", "right": "A", "numeric": true })
            )),
            "unknown field `numeric` at /clauses/0/forbid/ast/rule/compare"
        );
        // An unused utility that binds its own names compiles like any other.
        compiled(json!({
            "rule": { "pattern": "f($A)" },
            "utils": { "unused": { "pattern": "g($B, $C)", "compare": { "left": "B", "op": "lt", "right": "C" } } },
        }));
        // Names a utility or a constraint binds are captures; so is a comparison on its own.
        compiled(json!({
            "rule": { "pattern": "f($A)", "matches": "g" },
            "utils": { "g": { "has": { "pattern": "$G", "kind": "identifier" }, "compare": { "left": "A", "op": "ne", "right": "G" } } },
        }));
        compiled(json!({
            "rule": { "pattern": "f($A)" },
            "constraints": { "A": { "pattern": "$B", "compare": { "left": "B", "op": "eq", "right": "A" } } },
        }));
    }

    #[test]
    fn a_comparison_reads_names_bound_in_its_own_negation_count_or_stop_rule() {
        // ESLint E1: a comparison inside `not`, `count` or a stop rule reads the bindings that
        // sub-rule makes, since each threads one environment through its own terms.
        let later = json!({
            "stopBy": "end",
            "kind": "pair",
            "has": { "kind": "property_identifier", "pattern": "$J" },
            "compare": { "left": "J", "op": "eq", "right": "K" },
        });
        let base = |rest: Value| {
            let mut rule = json!({ "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$K" } });
            rule.as_object_mut()
                .expect("object")
                .extend(rest.as_object().expect("object").clone());
            rule
        };
        let source = "x = { a: 1, b: 2, a: 3, c: 4 };\n";
        // The `not` reads `$K`, so it runs after the relation that binds it.
        let last_of_each = compile_typescript_rule(json!({
            "kind": "pair",
            "all": [base(json!({})), { "not": { "precedes": later.clone() } }],
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            matched_texts(
                &evaluate_file(&last_of_each, "typescript", "a.ts", source),
                source
            ),
            ["b: 2 K=b", "a: 3 K=a", "c: 4 K=c"]
        );
        let repeated = compile_typescript_rule(base(
            json!({ "count": { "follows": later.clone(), "min": 1 } }),
        ))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            matched_texts(
                &evaluate_file(&repeated, "typescript", "a.ts", source),
                source
            ),
            ["a: 3 K=a"]
        );
        // A stop rule runs in a fresh environment: it may compare what it binds itself, but the
        // caller's `$K` is unbound there.
        assert_eq!(
            rejection(compile_typescript_rule(base(json!({
                "inside": { "kind": "program", "stopBy": { "kind": "object", "has": { "kind": "pair", "pattern": "$J" }, "compare": { "left": "J", "op": "eq", "right": "K" } } },
            })))),
            "/clauses/0/forbid/ast/rule/inside/stopBy/compare/right: `$K` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        compile_typescript_rule(base(json!({
            "inside": { "kind": "program", "stopBy": { "kind": "object", "has": { "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$J" } }, "not": { "has": { "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$I" }, "compare": { "left": "I", "op": "eq", "right": "J" } } } } },
        })))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        // A name bound in one `not` is still unbound in a sibling `not`, and outside both.
        assert_eq!(
            rejection(compile_typescript_rule(base(json!({
                "all": [
                    { "not": { "has": { "kind": "property_identifier", "pattern": "$J" } } },
                    { "not": { "compare": { "left": "J", "op": "eq", "right": "K" } } },
                ],
            })))),
            "/clauses/0/forbid/ast/rule/all/1/not/compare/left: `$J` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        // A utility referenced inside the negation binds into it too.
        compiled(json!({
            "rule": base(json!({ "not": { "precedes": { "stopBy": "end", "matches": "key-j", "compare": { "left": "J", "op": "eq", "right": "K" } } } })),
            "utils": { "key-j": { "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$J" } } },
        }));
    }

    /// The ESLint suite's M10 run 2 repro: a utility's comparison reads what its caller binds
    /// under `not`, `any` or a relation, as the same term written inline does.
    #[test]
    fn a_utility_comparison_reads_what_its_caller_binds() {
        let util = json!({ "u": { "kind": "member_expression",
            "has": { "kind": "property_identifier", "pattern": "$P" },
            "all": [{ "compare": { "left": "N", "op": "eq", "right": "P", "text": "name" } }] } });
        let inline = util["u"].clone();
        let source = "var foo = o.foo; var bar = o.baz;\n";
        let texts = |rule: Value, utils: Option<Value>| {
            let query = match utils {
                Some(utils) => json!({ "rule": rule, "utils": utils }),
                None => json!({ "rule": rule }),
            };
            let compiled =
                compile_typescript_ast(query).unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        let name = json!({ "kind": "identifier", "pattern": "$N" });
        // Under `not`: a declarator whose name is not the property it reads.
        let not = |call: Value| json!({ "kind": "variable_declarator", "not": { "all": [{ "has": name }, { "has": call }] } });
        assert_eq!(
            texts(not(json!({ "matches": "u" })), Some(util.clone())),
            ["bar = o.baz"]
        );
        assert_eq!(texts(not(inline.clone()), None), ["bar = o.baz"]);
        // Under `any`, and through a relation: declarators that read their own name.
        let any = |call: Value| json!({ "kind": "variable_declarator", "has": name, "any": [{ "has": call }, { "regex": "^never$" }] });
        assert_eq!(
            texts(any(json!({ "matches": "u" })), Some(util.clone())),
            ["foo = o.foo N=foo P=foo"]
        );
        assert_eq!(texts(any(inline), None), ["foo = o.foo N=foo P=foo"]);
        let relation = json!({ "kind": "identifier", "pattern": "$N",
            "inside": { "kind": "variable_declarator", "stopBy": "neighbor", "has": { "matches": "u" } } });
        assert_eq!(texts(relation, Some(util.clone())), ["foo N=foo P=foo"]);
        // A utility called only where the name is unbound is still refused.
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "kind": "variable_declarator", "has": { "matches": "u" }, "not": { "has": name } },
                "utils": util,
            }))),
            "/clauses/0/forbid/ast/utils/u/all/0/compare/left: `$N` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
    }

    #[test]
    fn count_bounds_are_validated() {
        let reject = |count: Value| {
            rejection(compile_typescript_rule(
                json!({ "kind": "program", "count": count }),
            ))
        };
        let has = json!({ "kind": "if_statement" });
        assert_eq!(
            reject(json!({ "has": has })),
            "/clauses/0/forbid/ast/rule/count must set min to at least 1, or set max; otherwise every node passes"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": 0 })),
            "/clauses/0/forbid/ast/rule/count must set min to at least 1, or set max; otherwise every node passes"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": 3, "max": 2 })),
            "/clauses/0/forbid/ast/rule/count: min must not exceed max"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": -1 })),
            "/clauses/0/forbid/ast/rule/count/min must be a non-negative integer"
        );
        assert_eq!(
            reject(json!({ "has": has, "inside": has, "min": 1 })),
            "/clauses/0/forbid/ast/rule/count must contain exactly one relation: inside, has, precedes, or follows"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": 1, "each": true })),
            "unknown field `each` at /clauses/0/forbid/ast/rule/count"
        );
        assert_eq!(
            reject(json!({ "has": { "kind": "if_statement", "stopBy": "far" }, "min": 1 })),
            "/clauses/0/forbid/ast/rule/count/has/stopBy has unsupported value `far`; expected \"neighbor\", \"end\" or a rule object"
        );
        // `as` names a metavariable; `offset` is an integer, and only shifts what `as` binds.
        assert_eq!(
            reject(json!({ "has": has, "min": 1, "offset": 1 })),
            "/clauses/0/forbid/ast/rule/count/offset is added to the value `as` binds; set `as`"
        );
        for name in ["ccn", "$CCN"] {
            assert_eq!(
                reject(json!({ "has": has, "min": 1, "as": name })),
                format!(
                    "/clauses/0/forbid/ast/rule/count/as: `{name}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
                )
            );
        }
        assert_eq!(
            reject(json!({ "has": has, "min": 1, "as": "CCN", "offset": 0.5 })),
            "/clauses/0/forbid/ast/rule/count/offset must be an integer"
        );
        // A count is a matcher on its own; comparisons inside it are checked like any other.
        compile_typescript_rule(json!({ "count": { "has": has, "min": 1 } })).expect("count alone");
        assert_eq!(
            reject(
                json!({ "has": { "pattern": "$A", "compare": { "left": "A", "op": "lt", "right": "B" } }, "min": 1 })
            ),
            "/clauses/0/forbid/ast/rule/count/has/compare/right: `$B` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
    }

    #[test]
    fn nth_child_of_rule_comparisons_share_the_containing_scope() {
        let undefined = json!({
            "kind": "identifier",
            "nthChild": {
                "position": 1,
                "ofRule": { "compare": { "left": "MISSING", "op": "eq", "right": "MISSING" } }
            }
        });
        assert_eq!(
            rejection(compile_typescript_rule(undefined)),
            "/clauses/0/forbid/ast/rule/nthChild/ofRule/compare/left: `$MISSING` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        compile_typescript_rule(json!({
            "kind": "identifier",
            "not": {
                "all": [
                    { "nthChild": { "position": 1, "ofRule": { "pattern": "$X" } } },
                    { "compare": { "left": "X", "op": "eq", "right": "X" } }
                ]
            }
        }))
        .expect("ofRule binding is visible inside its containing negation");
    }

    /// A comparison in a utility's frame may read what a call there binds through a cycle of
    /// parameterized calls, whichever `any` branch the walk naming the frame meets first.
    #[test]
    fn frame_names_bound_through_recursive_calls_do_not_depend_on_branch_order() {
        for branches in recursive_call_branch_orders() {
            let mut utils = mutually_recursive_calls();
            utils.insert(
                "Z".to_owned(),
                json!({ "params": ["P"], "rule": { "kind": "expression_statement", "any": branches,
                    "compare": { "left": "X", "op": "eq", "right": "P" } } }),
            );
            if let Err(error) = compile_typescript_ast(json!({
                "rule": { "pattern": "$C", "matches": { "Z": { "P": "$C" } } },
                "utils": utils,
            })) {
                panic!("{branches}: {}", error.reason);
            }
        }
    }

    /// The utility walk beside M11a's keys: a utility's comparison reads what its caller's `parse`
    /// exports and what its caller's `formatArgs` binds, under `not` too, as the inline term does.
    #[test]
    fn a_utility_comparison_reads_parse_and_format_args_captures_its_caller_binds() {
        let compile = |rule: Value, utils: Option<Value>| {
            let mut ast = json!({ "language": "python", "rule": rule });
            if let Some(utils) = utils {
                ast["utils"] = utils;
            }
            compile_document_with(
                clause_document(
                    json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation() }),
                ),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let texts = |rule: Value, utils: Option<Value>, source: &str| {
            let compiled = compile(rule, utils).unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "python", "a.py", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        // A call none of whose direct string arguments spells a mapping key its `%` format reads.
        let reads_key = json!({ "has": { "kind": "argument_list", "has": { "kind": "string", "pattern": "$Q",
            "compare": { "left": "Q", "op": "eq", "right": "K", "text": "unquoted" } } } });
        let keyed = |call: Value| {
            json!({ "kind": "call", "not": { "all": [
                { "has": { "kind": "string", "stopBy": "end", "parse": { "grammar": "percentFormat",
                    "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } } } },
                call,
            ] } })
        };
        let source = "print('%(a)s' % d, 'a')\nprint('%(a)s' % d, 'b')\n";
        let utils = json!({ "u": reads_key });
        assert_eq!(
            texts(keyed(json!({ "matches": "u" })), Some(utils), source),
            ["print('%(a)s' % d, 'b')"]
        );
        assert_eq!(
            texts(keyed(reads_key.clone()), None, source),
            ["print('%(a)s' % d, 'b')"]
        );
        // The same with the key a `%` format's mapping leaves unused, which `formatArgs` binds.
        let reads_unused = json!({ "has": { "kind": "argument_list", "has": { "kind": "string", "pattern": "$Q",
            "compare": { "left": "Q", "op": "eq", "right": "U", "text": "source" } } } });
        let unused = |call: Value| {
            json!({ "kind": "call", "not": { "all": [
                { "has": { "stopBy": "end", "pattern": "$S % $R",
                    "formatArgs": { "style": "percent", "string": "S", "args": "R",
                        "unused": { "kind": "pair", "has": { "kind": "string", "pattern": "$U" } } } } },
                call,
            ] } })
        };
        let source =
            "print('%(a)s' % {'a': 1, 'b': 2}, 'b')\nprint('%(a)s' % {'a': 1, 'b': 2}, 'c')\n";
        let utils = json!({ "v": reads_unused });
        let called = texts(unused(json!({ "matches": "v" })), Some(utils), source);
        assert_eq!(called, ["print('%(a)s' % {'a': 1, 'b': 2}, 'c')"]);
        assert_eq!(called, texts(unused(reads_unused.clone()), None, source));
        // Called where neither binds the name, the utility is still refused.
        let refused = |rule: Value, utils: Value| rejection(compile(rule, Some(utils)));
        assert!(
            refused(
                json!({ "kind": "call", "has": { "matches": "u" } }),
                json!({ "u": reads_key })
            )
            .contains("`$K` is never bound where this comparison runs"),
        );
        assert!(
            refused(
                json!({ "kind": "call", "has": { "matches": "v" } }),
                json!({ "v": reads_unused })
            )
            .contains("`$U` is never bound where this comparison runs"),
        );
    }
