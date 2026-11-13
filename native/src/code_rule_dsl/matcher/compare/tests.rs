    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    /// Shorthand keys whose nearest earlier shorthand key `$PREV` compares `op` to them.
    fn key_order(op: &str, collation: &str, source: &str) -> Vec<String> {
        key_order_with(
            json!({
                "kind": "shorthand_property_identifier",
                "pattern": "$KEY",
                "follows": { "stopBy": "end", "kind": "shorthand_property_identifier", "pattern": "$PREV" },
                "compare": { "left": "PREV", "op": op, "right": "KEY", "collation": collation },
            }),
            source,
        )
    }

    fn key_order_with(rule: Value, source: &str) -> Vec<String> {
        let rule = compiled(json!({ "rule": rule }));
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
            .into_iter()
            .map(|text| text.split(' ').next().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn compare_orders_two_captures_under_a_collation() {
        let source = "const o = { b, a, C, _x, B };\n";
        assert_eq!(key_order("gt", "ordinal", source), ["a", "C", "B"]);
        // `a` > `C` only by code unit; folded to lower case `_x` sorts before letters, folded
        // to upper case after them (the two case-insensitive orders differ exactly here).
        assert_eq!(key_order("gt", "fold-lower", source), ["a", "_x"]);
        assert_eq!(key_order("gt", "fold-upper", source), ["a", "B"]);
        // Lowercase first swaps ASCII case: `b a C _x B` compare as `B A c _X b`.
        assert_eq!(key_order("gt", "lowercase-first", source), ["a", "_x"]);
        let cases = [
            ("eq", "fold-lower", "const o = { a, A, b };\n", vec!["A"]),
            ("ne", "ordinal", "const o = { a, a, b };\n", vec!["b"]),
            (
                "le",
                "ordinal",
                "const o = { b, b, a, c };\n",
                vec!["b", "c"],
            ),
            ("lt", "ordinal", "const o = { b, b, a, c };\n", vec!["c"]),
            (
                "ge",
                "ordinal",
                "const o = { b, b, a, c };\n",
                vec!["b", "a"],
            ),
        ];
        for (op, collation, source, expected) in cases {
            assert_eq!(key_order(op, collation, source), expected, "{op} {source}");
        }
    }

    #[test]
    fn compare_uses_utf16_order_and_can_drop_quotes() {
        // U+FF61 sorts after U+1F600 by code point, before it by UTF-16 code unit (JavaScript `<`).
        let source = "f('\u{1F600}', '\u{FF61}');\nf('b', \"a\");\nf(\"b\", 'a');\n";
        let pairs = |op: &str, text: &str| {
            let rule = compiled(json!({ "rule": {
                "pattern": "f($A, $B)",
                "compare": { "left": "A", "op": op, "right": "B", "text": text },
            } }));
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(' ').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        // Source text: `'` (0x27) sorts after `"` (0x22), whatever the content.
        assert_eq!(pairs("gt", "source"), ["f('b',"]);
        assert_eq!(pairs("gt", "unquoted"), ["f('b',", "f(\"b\","]);
        assert_eq!(pairs("lt", "unquoted"), ["f('\u{1F600}',"]);
    }

    #[test]
    fn compare_reads_the_bindings_so_far_and_relations_search_past_failures() {
        // At the top level `follows` has bound the nearest earlier key (`a`) before the
        // comparison runs, and `c` > `b` is never tried.
        assert_eq!(
            key_order("gt", "ordinal", "const o = { c, a, b };\n"),
            ["a"]
        );
        // Inside the relation the comparison is part of the related node's rule, so the walk
        // goes on to the first earlier key that sorts after this one: every out-of-order key.
        let any_earlier_greater = json!({
            "kind": "shorthand_property_identifier",
            "pattern": "$KEY",
            "follows": {
                "stopBy": "end",
                "kind": "shorthand_property_identifier",
                "pattern": "$PREV",
                "compare": { "left": "PREV", "op": "gt", "right": "KEY" },
            },
        });
        assert_eq!(
            key_order_with(any_earlier_greater, "const o = { c, a, b, d };\n"),
            ["a", "b"]
        );
        // An unbound side fails the comparison: only the branch that binds `$PREV` passes.
        let either = json!({
            "kind": "shorthand_property_identifier",
            "pattern": "$KEY",
            "any": [
                { "follows": { "stopBy": "end", "kind": "shorthand_property_identifier", "pattern": "$PREV" } },
                { "regex": "^" },
            ],
            "compare": { "left": "PREV", "op": "ne", "right": "KEY" },
        });
        assert_eq!(key_order_with(either, "const o = { a, b };\n"), ["b"]);
        // Under `not`, the comparison reads the same bindings.
        let not_sorted = json!({
            "kind": "shorthand_property_identifier",
            "pattern": "$KEY",
            "follows": { "stopBy": "end", "kind": "shorthand_property_identifier", "pattern": "$PREV" },
            "not": { "compare": { "left": "PREV", "op": "le", "right": "KEY" } },
        });
        // `not` reads `$PREV`, which the relation in the same object binds, so it runs after it,
        // as it does in an ordered `all`.
        assert_eq!(
            key_order_with(not_sorted, "const o = { b, a, c };\n"),
            ["a"]
        );
        let ordered = json!({ "all": [
            { "kind": "shorthand_property_identifier", "pattern": "$KEY",
              "follows": { "stopBy": "end", "kind": "shorthand_property_identifier", "pattern": "$PREV" } },
            { "not": { "compare": { "left": "PREV", "op": "le", "right": "KEY" } } },
        ] });
        assert_eq!(key_order_with(ordered, "const o = { b, a, c };\n"), ["a"]);
    }

    fn python_texts(ast: Value, source: &str) -> Vec<String> {
        let mut ast = ast;
        ast["language"] = json!("python");
        let rule = compile_document_with(
            clause_document(
                json!({ "id": "c", "forbid": { "ast": ast }, "violation": violation() }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&rule, "python", "a.py", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
            .into_iter()
            .map(|text| text.split(' ').next().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn value_compares_what_captures_evaluate_to() {
        // Ruff F601: a dict key equal, by Python `==`, to an earlier one.
        let pair = |key: &str, value: &str| json!({ "context": format!("{{{key}: {value}}}"), "selector": "pair" });
        let repeated = json!({ "rule": {
            "pattern": pair("$K", "$V"),
            "follows": { "stopBy": "end", "pattern": pair("$J", "$W"), "compare": { "left": "J", "op": "eq", "right": "K", "text": "value" } },
        } });
        let source =
            "d = {1: a, 1.0: b, 'x': c, \"x\": d, y: e, y: f, True: g, (1, 2): h, (1.0, 2): i}\n";
        // The relation binds `$J`, so an unknown key (`y`) ends its search: which pair would bind
        // first is unknown. `True` equals `1`, but the walk back from it stops at `y`.
        assert_eq!(python_texts(repeated, source), ["1.0:", "\"x\":", "(1.0,"]);
        // `count` keeps nothing it binds, so it searches on past an unknown key.
        let counted = json!({ "rule": {
            "pattern": pair("$K", "$V"),
            "count": { "follows": { "stopBy": "end", "pattern": pair("$J", "$W"), "compare": { "left": "J", "op": "eq", "right": "K", "text": "value" } }, "min": 1 },
        } });
        assert_eq!(
            python_texts(counted, source),
            ["1.0:", "\"x\":", "True:", "(1.0,"]
        );
        // JavaScript: numbers order numerically, strings by code unit.
        let rule = compiled(json!({ "rule": {
            "pattern": "f($A, $B)",
            "compare": { "left": "A", "op": "gt", "right": "B", "text": "value" },
        } }));
        let source = "f(10, 9);\nf('10', '9');\nf(0x10, 1e1);\nf(x, 1);\nf(1n, 0);\nf(NaN, 0);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            [
                "f(10, 9) A=10 B=9",
                "f(0x10, 1e1) A=0x10 B=1e1",
                "f(1n, 0) A=1n B=0"
            ]
        );
        // A collation applies where both values are strings.
        let rule = compiled(json!({ "rule": {
            "pattern": "f($A, $B)",
            "compare": { "left": "A", "op": "eq", "right": "B", "text": "value", "collation": "fold-lower" },
        } }));
        let source = "f('A', \"a\");\nf('\\x41', 'a');\nf(1, 1.0);\nf('1', 1);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source)
                .iter()
                .map(|text| text.split(' ').next().unwrap_or_default())
                .collect::<Vec<_>>(),
            ["f('A',", "f('\\x41',", "f(1,"]
        );
        // An unknown side leaves the comparison unknown, so its negation reports nothing.
        let rule = compiled(json!({ "rule": {
            "pattern": "f($A, $B)",
            "not": { "compare": { "left": "A", "op": "eq", "right": "B", "text": "value" } },
        } }));
        let source = "f(1, 2);\nf(x, 2);\nf(2, 2);\nf(g(), 1);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["f(1, 2) A=1 B=2"]);
    }

    /// JavaScript orders values as its relational operators do (IsLessThan): primitives convert
    /// with ToNumeric, strings against BigInts with StringToBigInt, and an undefined comparison
    /// (NaN) is false. A regex converts to its string (`/a/ <= 1` is false); an object or an array
    /// converts in a way this does not model.
    #[test]
    fn value_orders_javascript_values_as_relational_operators_do() {
        let rule = compiled(json!({ "rule": {
            "kind": "binary_expression",
            "pattern": "$A <= $B",
            "not": { "compare": { "left": "A", "op": "le", "right": "B", "text": "value" } },
        } }));
        let big = "1".repeat(45);
        let source = format!(
            "null <= 1;\n'1' <= 2;\nfalse <= true;\n'' <= 0;\n1n <= '1';\n1n <= '{big}';\n\
             2 <= 1;\nNaN <= NaN;\nundefined <= 0;\n'a' <= 1;\n'1.5' <= 2n;\n'b' <= 'a';\n'{big}' <= 1n;\n/a/ <= 1;\n/b/ <= '/c/';\n\
             [1] <= 2;\n({{}}) <= 1;\nx <= 1;\n"
        );
        let result = evaluate_file(&rule, "typescript", "a.ts", &source);
        assert_eq!(result["failures"], json!([]), "{result}");
        let lines = matched_texts(&result, &source)
            .into_iter()
            .map(|text| text.split(" A=").next().unwrap_or_default().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            [
                "2 <= 1".to_owned(),
                "NaN <= NaN".to_owned(),
                "undefined <= 0".to_owned(),
                "'a' <= 1".to_owned(),
                "'1.5' <= 2n".to_owned(),
                "'b' <= 'a'".to_owned(),
                format!("'{big}' <= 1n"),
                "/a/ <= 1".to_owned(),
            ]
        );
        // Each order, over the same pairs: `lt`, `gt` and `ge` hold where `<`, `>` and `>=` do.
        let pairs = |op: &str| {
            let rule = compiled(json!({ "rule": {
                "pattern": "f($A, $B)",
                "compare": { "left": "A", "op": op, "right": "B", "text": "value" },
            } }));
            let source =
                "f(null, 0);\nf('1', 1n);\nf(true, '1');\nf(NaN, 1);\nf('x', 'y');\nf(-0, 0);\n";
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(' ').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(pairs("lt"), ["f('x',"]);
        assert_eq!(pairs("gt"), Vec::<String>::new());
        assert_eq!(pairs("ge"), ["f(null,", "f('1',", "f(true,", "f(-0,"]);
        assert_eq!(
            pairs("le"),
            ["f(null,", "f('1',", "f(true,", "f('x',", "f(-0,"]
        );
        // Equality stays SameValue (`Object.is`): `null <= 0` holds, `null` is not `0`.
        assert_eq!(pairs("eq"), Vec::<String>::new());
    }

    /// A key without a static name (ESLint's `getStaticPropertyName` returns null) equals no
    /// name, not even another nameless one, as every ESLint rule that compares names skips null;
    /// an order against it stays unknown, since sort-keys skips it whichever way a port asks.
    #[test]
    fn key_without_a_static_name_equals_nothing() {
        let names = |compare: Value| {
            let rule = compiled(json!({ "rule": {
                "kind": "binary_expression",
                "pattern": "$A == $B",
                "all": [compare],
            } }));
            let source = "a[b] == a.c;\na.c == a[b];\na[b] == a[b];\na[b] == a[c];\na.c == a['c'];\na.c == a.d;\n";
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(" A=").next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        let compare =
            |op: &str| json!({ "compare": { "left": "A", "op": op, "right": "B", "text": "key" } });
        assert_eq!(
            names(json!({ "not": compare("eq") })),
            [
                "a[b] == a.c",
                "a.c == a[b]",
                "a[b] == a[b]",
                "a[b] == a[c]",
                "a.c == a.d"
            ]
        );
        assert_eq!(names(compare("eq")), ["a.c == a['c']"]);
        assert_eq!(
            names(compare("ne")),
            [
                "a[b] == a.c",
                "a.c == a[b]",
                "a[b] == a[b]",
                "a[b] == a[c]",
                "a.c == a.d"
            ]
        );
        // Orders: known names only.
        assert_eq!(names(compare("lt")), ["a.c == a.d"]);
        assert_eq!(names(json!({ "not": compare("lt") })), ["a.c == a['c']"]);
    }

    /// `key` over a computed key's expression reads it as ESLint's `getStaticStringValue` does, so
    /// comparing through it is comparing through the property that holds it.
    #[test]
    fn key_reads_a_computed_keys_expression_as_its_property() {
        let found = |pattern: &str, op: &str| {
            let rule = compiled(json!({ "rule": {
                "kind": "object",
                "pattern": { "context": format!("x = {pattern};"), "selector": "object" },
                "compare": { "left": "A", "op": op, "right": "B", "text": "key" },
            } }));
            let source = "x = { b: 1, [`a`]: 2 };\nx = { b: 1, [/a/]: 2 };\nx = { a: 1, [`b`]: 2 };\n\
                          x = { b: 1, [c]: 2 };\nx = { b: 1, [`a${c}`]: 2 };\nx = { b: 1, [(`b`)]: 2 };\n\
                          x = { b: 1, [true]: 2 };\nx = { b: 1, [0x1n]: 2 };\n";
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(" A=").next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        // Through the inner expression.
        let inner = "{ $A: $X, [$B]: $Y }";
        assert_eq!(
            found(inner, "gt"),
            [
                "{ b: 1, [`a`]: 2 }",
                "{ b: 1, [/a/]: 2 }",
                "{ b: 1, [0x1n]: 2 }"
            ]
        );
        assert_eq!(found(inner, "eq"), ["{ b: 1, [(`b`)]: 2 }"]);
        // Through the pairs: the same verdicts.
        let pairs = "{ $A, $B }";
        assert_eq!(
            found(pairs, "gt"),
            [
                "{ b: 1, [`a`]: 2 }",
                "{ b: 1, [/a/]: 2 }",
                "{ b: 1, [0x1n]: 2 }"
            ]
        );
        assert_eq!(found(pairs, "eq"), ["{ b: 1, [(`b`)]: 2 }"]);
    }

    #[test]
    fn key_compares_static_property_names() {
        // ESLint no-dupe-keys: a key named as an earlier one.
        let rule = compiled(json!({ "rule": {
            "any": [{ "kind": "pair" }, { "kind": "method_definition" }, { "kind": "shorthand_property_identifier" }],
            "pattern": "$P",
            "follows": {
                "stopBy": "end",
                "any": [{ "kind": "pair" }, { "kind": "method_definition" }, { "kind": "shorthand_property_identifier" }],
                "pattern": "$Q",
                "compare": { "left": "Q", "op": "eq", "right": "P", "text": "key" },
            },
        } }));
        let source = "const o = { a: 1, 'a': 2, ['a']: 3, [a]: 4, 0x1: 5, 1: 6, b() {}, get b() { return 1; }, c: 7, c, 'd\\u0065': 8, de: 9 };\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source)
                .iter()
                .map(|text| text.split(' ').next().unwrap_or_default())
                .collect::<Vec<_>>(),
            ["'a':", "['a']:", "1:", "get", "c", "de:"]
        );
        // Keys order by collation, as sort-keys does.
        let rule = compiled(json!({ "rule": {
            "kind": "pair",
            "pattern": "$P",
            "follows": { "stopBy": "end", "kind": "pair", "pattern": "$Q" },
            "compare": { "left": "Q", "op": "gt", "right": "P", "text": "key", "collation": "fold-lower" },
        } }));
        let source = "const o = { B: 1, a: 2, 'c': 3, [`b`]: 4 };\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source)
                .iter()
                .map(|text| text.split(' ').next().unwrap_or_default())
                .collect::<Vec<_>>(),
            ["a:", "[`b`]:"]
        );
        let reason = rejection(compile_document_with(
            clause_document(
                json!({ "id": "c", "violation": violation(), "forbid": { "ast": { "language": "python", "rule": {
                "pattern": "f($A, $B)", "compare": { "left": "A", "op": "eq", "right": "B", "text": "key" },
            } } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/compare/text: `key` reads JavaScript and PHP property names; Python has none"
        );
    }

    /// `member` names a key as typescript-eslint's getStaticMemberAccessValue does: a computed key
    /// by what it folds to, through bindings and builtins (`['f' + '']` and `[k]` are `f`), a private
    /// name without its `#`. A key whose value is not known compares neither way.
    #[test]
    fn member_names_keys_as_typescript_eslint_does() {
        let members = json!({ "any": [{ "kind": "method_definition" }, { "kind": "public_field_definition" }] });
        let rule = |op: &str| {
            compiled(json!({ "rule": {
                "all": [members.clone()],
                "pattern": "$P",
                "follows": { "stopBy": "end", "all": [members.clone()], "pattern": "$Q",
                             "compare": { "left": "Q", "op": op, "right": "P", "text": "member" } },
            } }))
        };
        let source = "const k = 'f';\nclass C { f() {} ['f' + '']() {} [k]() {} #g = 1; g = 2; [String(1)]() {} 1() {} [x]() {} [x]() {} }\n";
        let found = |op: &str| {
            let result = evaluate_file(&rule(op), "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(" P=").next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            found("eq"),
            ["['f' + '']() {}", "[k]() {}", "g = 2", "1() {}"]
        );
        // `key` reads ESLint's getStaticPropertyName, which folds nothing and names no private key.
        let key = compiled(json!({ "rule": {
            "all": [members.clone()],
            "pattern": "$P",
            "follows": { "stopBy": "end", "all": [members.clone()], "pattern": "$Q",
                         "compare": { "left": "Q", "op": "eq", "right": "P", "text": "key" } },
        } }));
        let result = evaluate_file(&key, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(" P=").next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>(),
            Vec::<String>::new()
        );
        let reason = rejection(compile_document_with(
            clause_document(
                json!({ "id": "c", "violation": violation(), "forbid": { "ast": { "language": "python", "rule": {
                "pattern": "f($A, $B)", "compare": { "left": "A", "op": "eq", "right": "B", "text": "member" },
            } } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/compare/text: `member` reads JavaScript and PHP property names; Python has none"
        );
    }

    /// `name` compares what ESTree names a node: an identifier's name with its escapes decoded and a
    /// string literal's value, so a string key equals a function's name (func-name-matching) and an
    /// escaped export name its plain alias (no-useless-rename). Any other node has no name.
    #[test]
    fn name_compares_string_values_with_identifier_names() {
        let pairs = |op: &str| {
            let rule = compiled(json!({ "rule": {
                "pattern": { "context": "({ $K: function $N() {} })", "selector": "pair" },
                "compare": { "left": "K", "op": op, "right": "N", "text": "name" },
            } }));
            let source = "({ 'a\\u0062': function ab() {}, 'x': function y() {}, \\u0061: function a() {}, [`c`]: function c() {}, 1: function d() {} });\n";
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(':').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(pairs("eq"), ["'a\\u0062'", "\\u0061"]);
        // A template key and a number have no name: never equal, always unequal.
        assert_eq!(pairs("ne"), ["'x'", "[`c`]", "1"]);
        let renames = compiled(json!({ "rule": {
            "any": [{ "pattern": "export { $A as $B }" }, { "pattern": "import { $A as $B } from 'm'" }],
            "compare": { "left": "A", "op": "eq", "right": "B", "text": "name" },
        } }));
        let source = "export { \\u0061 as a };\nexport { b as c };\nimport { 'x' as x } from 'm';\nimport { 'y' as z } from 'm';\n";
        let result = evaluate_file(&renames, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source)
                .into_iter()
                .map(|text| text.split(';').next().unwrap_or_default().to_owned())
                .collect::<Vec<_>>(),
            ["export { \\u0061 as a }", "import { 'x' as x } from 'm'"]
        );
    }

    #[test]
    fn natural_collations_order_digit_runs_by_number() {
        // ESLint sort-keys with `natural: true`: `a9` before `a10`, capitals before small letters.
        let source = "const o = { a10, a9, A11, b };\n";
        assert_eq!(key_order("gt", "ordinal", source), ["A11"]);
        assert_eq!(key_order("gt", "natural", source), ["a9", "A11"]);
        assert_eq!(key_order("gt", "natural-fold-lower", source), ["a9"]);
        let source = "const o = { x2, x10, X3 };\n";
        assert_eq!(key_order("gt", "ordinal", source), ["x10", "X3"]);
        assert_eq!(key_order("gt", "natural", source), ["X3"]);
        assert_eq!(key_order("gt", "natural-fold-lower", source), ["X3"]);
    }

    #[test]
    fn structure_modes_compare_python_as_ruff_does() {
        let pairs = |text: &str, op: &str, source: &str| {
            python_texts(
                json!({ "rule": { "pattern": "f($A, $B)", "compare": { "left": "A", "op": op, "right": "B", "text": text } } }),
                source,
            )
        };
        let source = "f((1, 'a'), (1, \"a\"))\nf(-1, -(1))\nf([1, 0x2], [1, 2])\nf(1, 1.0)\nf(1, True)\nf('a' 'b', 'ab' '')\nf(f'{x}', f\"{x}\")\n";
        // ComparableExpr: type-strict, spelling-blind.
        assert_eq!(
            pairs("structure", "eq", source),
            ["f((1,", "f(-1,", "f([1,", "f('a'", "f(f'{x}',"]
        );
        // HashableExpr: numbers fold by value.
        assert_eq!(
            pairs("structure-hash", "eq", source),
            [
                "f((1,",
                "f(-1,",
                "f([1,",
                "f(1,",
                "f(1,",
                "f('a'",
                "f(f'{x}',"
            ]
        );
        // ComparableLiteral: implicit concatenation part by part.
        assert_eq!(
            pairs("structure-parts", "eq", source),
            ["f((1,", "f(-1,", "f([1,", "f(f'{x}',"]
        );
        assert_eq!(pairs("structure", "ne", source), ["f(1,", "f(1,"]);
        // A capture whose structure is not known (an unknown `\N{...}` name) compares neither
        // equal nor unequal.
        let unknown = "f('\\N{NOT A NAME}', 'a')\nf('b', 'a')\n";
        assert_eq!(pairs("structure", "eq", unknown), Vec::<String>::new());
        assert_eq!(pairs("structure", "ne", unknown), ["f('b',"]);
        for (compare, expected) in [
            (
                json!({ "left": "A", "op": "lt", "right": "B", "text": "structure-hash" }),
                "/clauses/0/forbid/ast/rule/compare/op: `structure-hash` compares for equality only; use eq or ne",
            ),
            (
                json!({ "left": "A", "op": "eq", "right": "B", "text": "structure", "collation": "fold-lower" }),
                "/clauses/0/forbid/ast/rule/compare/collation: `structure` compares structures exactly; use ordinal",
            ),
            (
                json!({ "left": "A", "op": "eq", "right": "B", "text": "structure-parts" }),
                "/clauses/0/forbid/ast/rule/compare/text: the structural modes compare Python and PHP syntax; use tokens for TypeScript",
            ),
        ] {
            let reason = rejection(compile_typescript_rule(
                json!({ "pattern": "f($A, $B)", "compare": compare }),
            ));
            assert_eq!(reason, expected);
        }
    }

    #[test]
    fn tokens_compare_leaves_without_comments_or_white_space() {
        let rule = compiled(json!({ "rule": {
            "pattern": "f($A, $B)",
            "compare": { "left": "A", "op": "eq", "right": "B", "text": "tokens" },
        } }));
        let source = "f(a + b, a+b);\nf(a /* x */ + b, a + b);\nf('a', \"a\");\nf(a.b, a . b);\nf((a), a);\nf(`a ${b}`, `a ${ b }`);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source)
                .iter()
                .map(|text| text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split(", ")
                    .next()
                    .unwrap_or_default())
                .collect::<Vec<_>>(),
            ["f(a + b", "f(a /* x */ + b", "f(a.b", "f(`a ${b}`"]
        );
        let python = python_texts(
            json!({ "rule": { "pattern": "f($A, $B)", "compare": { "left": "A", "op": "ne", "right": "B", "text": "tokens" } } }),
            "f(a + b, a+b)\nf(a, b)\nf(x[1], x[ 1 ])\n",
        );
        assert_eq!(python, ["f(a,"]);
        for (compare, expected) in [
            (
                json!({ "left": "A", "op": "lt", "right": "B", "text": "tokens" }),
                "/clauses/0/forbid/ast/rule/compare/op: `tokens` compares for equality only; use eq or ne",
            ),
            (
                json!({ "left": "A", "op": "eq", "right": "B", "text": "tokens", "collation": "fold-lower" }),
                "/clauses/0/forbid/ast/rule/compare/collation: `tokens` compares tokens exactly; use ordinal",
            ),
        ] {
            let reason = rejection(compile_typescript_rule(
                json!({ "pattern": "f($A, $B)", "compare": compare }),
            ));
            assert_eq!(reason, expected);
        }
    }

    /// `node` tells two captures apart by identity: the same node, not the same text.
    #[test]
    fn node_compares_captures_by_identity() {
        let source = "for x in y:\n    i += 1\nfor x in y:\n    i += 1\n";
        let rule = |op: &str, text: &str| {
            json!({ "rule": { "kind": "for_statement", "pattern": "$A",
                "follows": { "stopBy": "end", "kind": "for_statement", "pattern": "$B",
                             "compare": { "left": "A", "op": op, "right": "B", "text": text } } } })
        };
        // The second loop follows a loop that is not itself, though its text is the same.
        assert_eq!(python_texts(rule("ne", "node"), source), ["for"]);
        assert_eq!(
            python_texts(rule("ne", "source"), source),
            Vec::<String>::new()
        );
        assert_eq!(
            python_texts(rule("eq", "node"), source),
            Vec::<String>::new()
        );
        let typescript = compiled(json!({ "rule": { "pattern": "f($A, $B)",
            "compare": { "left": "A", "op": "eq", "right": "B", "text": "node" } } }));
        let result = evaluate_file(&typescript, "typescript", "a.ts", "f(a, a);\n");
        assert_eq!(matched_texts(&result, "f(a, a);\n"), Vec::<String>::new());
        for (compare, expected) in [
            (
                json!({ "left": "A", "op": "lt", "right": "B", "text": "node" }),
                "/clauses/0/forbid/ast/rule/compare/op: `node` compares for equality only; use eq or ne",
            ),
            (
                json!({ "left": "A", "op": "eq", "right": "B", "text": "node", "collation": "fold-lower" }),
                "/clauses/0/forbid/ast/rule/compare/collation: `node` compares nodes exactly; use ordinal",
            ),
        ] {
            let reason = rejection(compile_typescript_rule(
                json!({ "pattern": "f($A, $B)", "compare": compare }),
            ));
            assert_eq!(reason, expected);
        }
    }

    #[test]
    fn index_compares_estree_element_positions() {
        // no-self-assign's array patterns: the i-th element on each side, holes counted.
        let rule = compiled(json!({ "rule": {
            "kind": "identifier",
            "pattern": "$R",
            "inside": { "kind": "array", "inside": { "kind": "assignment_expression",
                "has": { "kind": "array_pattern", "has": { "kind": "identifier", "pattern": "$L", "all": [
                    { "compare": { "left": "L", "op": "eq", "right": "R", "text": "tokens" } },
                    { "compare": { "left": "L", "op": "eq", "right": "R", "text": "index" } },
                ] } } } },
        } }));
        let source = "[a, b] = [a, c];\n[a, b] = [b, a];\n[a, b] = [, b];\n[, a] = [a];\n[c, /* x */ b] = [c, b];\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["a L=a R=a", "b L=b R=b", "c L=c R=c", "b L=b R=b"]
        );
        // Orders compare the indices; a node outside a list has none, and compares false.
        let order = |op: &str| {
            let rule = compiled(json!({ "rule": {
                "pattern": "f($A, $B, $$$)",
                "compare": { "left": "A", "op": op, "right": "B", "text": "index" },
            } }));
            let source = "f(x, y, z);\nf(y, x, z, w);\n";
            matched_texts(&evaluate_file(&rule, "typescript", "a.ts", source), source).len()
        };
        assert_eq!(
            (order("lt"), order("gt"), order("ne"), order("eq")),
            (2, 0, 2, 0)
        );
        let outside = compiled(json!({ "rule": {
            "pattern": "$A + $B",
            "compare": { "left": "A", "op": "eq", "right": "A", "text": "index" },
        } }));
        let source = "a + b;\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&outside, "typescript", "a.ts", source),
                source
            ),
            Vec::<String>::new()
        );
        // Python parameters paired with a call's arguments.
        let python = python_texts(
            json!({ "rule": {
                "kind": "function_definition",
                "all": [
                    { "has": { "kind": "parameters", "has": { "kind": "identifier", "pattern": "$P" } } },
                    { "has": { "stopBy": "end", "kind": "argument_list", "has": { "kind": "identifier", "pattern": "$A", "all": [
                        { "compare": { "left": "P", "op": "eq", "right": "A", "text": "tokens" } },
                        { "compare": { "left": "P", "op": "eq", "right": "A", "text": "index" } },
                    ] } } },
                ],
            } }),
            "def f(a, b):\n    g(a, b)\ndef h(a, b):\n    g(b, a)\n",
        );
        assert_eq!(python, ["def"]);
        let reason = rejection(compile_typescript_rule(json!({ "pattern": "f($A, $B)",
            "compare": { "left": "A", "op": "lt", "right": "B", "text": "index", "collation": "natural" } })));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/compare/collation: `index` compares positions exactly; use ordinal"
        );
    }

    #[test]
    fn commute_swaps_the_operands_of_listed_operators() {
        // ESLint no-dupe-else-if's `equal`: `||` and `&&` operands in either order, recursively,
        // parentheses aside; any other node by its tokens.
        let rule = |commute: Value| {
            compiled(json!({ "rule": {
                "pattern": "f($A, $B)",
                "compare": { "left": "A", "op": "eq", "right": "B", "text": "tokens", "commute": commute },
            } }))
        };
        let source = "f(a || b, b || a);\nf(a && b, (b && a));\nf((a && b) || c, c || (b && a));\nf(a || b || c, c || a || b);\nf(a || b || c, c || (a || b));\nf(a ?? b, b ?? a);\nf(a | b, b | a);\nf(a && b, b || a);\nf(a + b, a + b);\n";
        let lines = |commute: Value| {
            let result = evaluate_file(&rule(commute), "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            result["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|violation| violation["span"]["startLine"].as_u64().expect("line") + 1)
                .collect::<Vec<_>>()
        };
        assert_eq!(lines(json!(["||", "&&"])), [1, 2, 3, 5, 9]);
        assert_eq!(lines(json!(["|"])), [7, 9]);
        let python = python_texts(
            json!({ "rule": { "pattern": "f($A, $B)",
                "compare": { "left": "A", "op": "ne", "right": "B", "text": "tokens", "commute": ["or", "and"] } } }),
            "f(a or b, b or a)\nf((a and b) or c, c or (b and a))\nf(a or b, a and b)\n",
        );
        assert_eq!(python, ["f(a"]);
        for (compare, expected) in [
            (
                json!({ "left": "A", "op": "eq", "right": "B", "commute": ["||"] }),
                "/clauses/0/forbid/ast/rule/compare/commute: commute applies to text tokens only",
            ),
            (
                json!({ "left": "A", "op": "eq", "right": "B", "text": "tokens", "commute": [] }),
                "/clauses/0/forbid/ast/rule/compare/commute must be a non-empty array of operator texts",
            ),
            (
                json!({ "left": "A", "op": "eq", "right": "B", "text": "tokens", "commute": "||" }),
                "/clauses/0/forbid/ast/rule/compare/commute must be a non-empty array of operator texts",
            ),
        ] {
            let reason = rejection(compile_typescript_rule(
                json!({ "pattern": "f($A, $B)", "compare": compare }),
            ));
            assert_eq!(reason, expected);
        }
    }
