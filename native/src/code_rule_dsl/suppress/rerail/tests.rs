    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::test_support::*;
    use ast_grep_language::LanguageExt;
    use serde_json::{Value, json};

    #[test]
    fn reads_rule_ids_from_a_directive_comment() {
        for (comment, ids) in [
            ("// rerail-ignore a", vec!["a"]),
            ("// rerail-ignore a, b c,d", vec!["a", "b", "c", "d"]),
            (
                "//rerail-ignore acme:no-print -- generated code",
                vec!["acme:no-print"],
            ),
            ("// rerail-ignore a --- b", vec!["a"]),
            ("// rerail-ignore a-b --x", vec!["a-b", "--x"]),
            (
                "# rerail-ignore complexity-cyclomatic-python",
                vec!["complexity-cyclomatic-python"],
            ),
            ("/* rerail-ignore x.y_z/w */", vec!["x.y_z/w"]),
            ("/**\n * rerail-ignore a\n * b\n */", vec!["a"]),
            ("/** rerail-ignore a */", vec!["a"]),
            ("{{-- rerail-ignore a --}}", vec!["a"]),
            ("<!-- rerail-ignore a -->", vec!["a"]),
            ("<!-- rerail-ignore a", vec!["a"]),
            ("--> rerail-ignore a -- generated", vec!["a"]),
            ("# rerail-ignore a\n", vec!["a"]),
            // No id: nothing.
            ("// rerail-ignore", vec![]),
            ("// rerail-ignore -- reason", vec![]),
            ("// rerail-ignore\n", vec![]),
            // Not the directive.
            ("// rerail-ignored a", vec![]),
            ("// rerail-ignore-next a", vec![]),
            ("// Rerail-Ignore a", vec![]),
            ("// see rerail-ignore a", vec![]),
            ("## rerail-ignore a", vec![]),
        ] {
            assert_eq!(rule_ids(comment), ids, "{comment:?}");
        }
    }

    /// The covered ranges of a source in `language`, as the texts they cover.
    fn covered_texts(language: GrammarDialect, source: &str) -> Vec<(String, Vec<String>)> {
        let root = language.ast_grep(source);
        RerailIgnores::read(&root.root(), source)
            .covers
            .into_iter()
            .map(|cover| {
                let end = cover.range.end.min(source.len());
                (source[cover.range.start..end].to_owned(), cover.rules)
            })
            .collect()
    }

    fn cover(text: &str, rules: &[&str]) -> (String, Vec<String>) {
        (
            text.to_owned(),
            rules.iter().map(|rule| (*rule).to_owned()).collect(),
        )
    }

    #[test]
    fn a_directive_covers_its_node() {
        let source = "#!/usr/bin/env node\n// header\n// rerail-ignore whole\nconst a = 1;\n// rerail-ignore lead\nfunction f() {\n  g();\n}\nfunction h() {}\nb(); // rerail-ignore trail\nc(); d(); // rerail-ignore last\nconst e = k( // rerail-ignore open\n  1,\n);\nm(n, // rerail-ignore inner\n  o);\nfunction p() {\n  q();\n  // rerail-ignore end\n}\nconst s = \"// rerail-ignore string\";\n";
        assert_eq!(
            covered_texts(GrammarDialect::TypeScript, source),
            [
                cover(source, &["whole"]),
                cover("function f() {\n  g();\n}", &["lead"]),
                cover("b();", &["trail"]),
                cover("d();", &["last"]),
                cover("const e = k( // rerail-ignore open\n  1,\n);", &["open"]),
                cover("m(n, // rerail-ignore inner\n  o)", &["inner"]),
            ]
        );
    }

    #[test]
    fn a_directive_covers_its_node_in_python_and_php() {
        let python = "# rerail-ignore whole\nimport os\n# rerail-ignore lead\ndef f():\n    a = 1\n    return a\n\ndef g():\n    pass\nx = h(  # rerail-ignore open\n    1)\nif y:  # rerail-ignore cond\n    z()\ndef k():\n    # rerail-ignore body\n    a()\n    b()\n";
        assert_eq!(
            covered_texts(GrammarDialect::Python, python),
            [
                cover(python, &["whole"]),
                cover("def f():\n    a = 1\n    return a", &["lead"]),
                cover("x = h(  # rerail-ignore open\n    1)", &["open"]),
                cover("if y:  # rerail-ignore cond\n    z()", &["cond"]),
                cover("a()", &["body"]),
            ]
        );
        let php = "<?php\n/** rerail-ignore whole */\n$a = 1;\n# rerail-ignore lead\nfunction f() {\n    return 1;\n}\n$b = g(); // rerail-ignore trail\n$c = \"# rerail-ignore string\";\n";
        assert_eq!(
            covered_texts(GrammarDialect::Php, php),
            [
                cover(php, &["whole"]),
                cover("function f() {\n    return 1;\n}", &["lead"]),
                cover("$b = g();", &["trail"]),
            ]
        );
    }

    /// A leading directive before a block that starts at no statement covers the whole block:
    /// SQL's `BEGIN … END` starts at `BEGIN`, a labelled Rust block at its label. Only Python's
    /// `block` starts at its first statement, which the directive then leads.
    #[test]
    fn a_leading_directive_covers_a_whole_block_outside_python() {
        let sql = "SELECT 0;\n-- rerail-ignore lead\nBEGIN;\nSELECT 1;\nEND;\nSELECT 2;\n";
        assert_eq!(
            covered_texts(GrammarDialect::Sql, sql),
            [cover("BEGIN;\nSELECT 1;\nEND", &["lead"])]
        );
        let rust = "fn f(x: u8) {\n    match x {\n        _ =>\n        // rerail-ignore lead\n        'a: { g(); }\n    }\n}\n";
        assert_eq!(
            covered_texts(GrammarDialect::Rust, rust),
            [cover("'a: { g(); }", &["lead"])]
        );
    }

    /// One code rule, `id`, reporting each node of `kind` in `language` (a Blade template's
    /// compiled view), with `suppress` where given.
    fn rule(id: &str, language: &str, kind: &str, suppress: Option<Value>) -> Value {
        let mut violation = violation();
        if let Some(suppress) = suppress {
            violation["suppress"] = suppress;
        }
        let mut ast = json!({ "language": language, "rule": { "kind": kind } });
        if language == "blade" {
            ast["embedded"] = json!("compiled-view");
        }
        json!({ "format": "rerail.policy/v1", "id": id, "revision": 1, "mode": "block",
            "clauses": [{ "id": "c", "violation": violation, "forbid": { "ast": ast } }] })
    }

    fn reported(document: Value, language: &str, path: &str, source: &str) -> Vec<String> {
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, language, path, source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    }

    /// Through the engine, with no `suppress`: the file-top directive covers the file, a leading
    /// one the next node (a violation on its third line too, none in the next function), a
    /// trailing one its statement, and another rule's id nothing.
    #[test]
    fn the_engine_drops_ignored_witnesses_in_every_language() {
        let source = "function f() {\n  a();\n  b();\n}\n// rerail-ignore calls\nfunction g() {\n  c();\n  d();\n}\nfunction h() {\n  e();\n}\nk( // rerail-ignore calls\n  l(),\n);\nm(); // rerail-ignore other\n";
        let document = rule("calls", "typescript", "call_expression", None);
        assert_eq!(
            reported(document.clone(), "typescript", "a.ts", source),
            ["a()", "b()", "e()", "m()"]
        );
        let top = format!("// rerail-ignore other, calls -- generated\n{source}");
        assert!(reported(document, "typescript", "a.ts", &top).is_empty());

        let python = "def f():\n    a()\n\n# rerail-ignore calls\ndef g():\n    b()\n    c()\n\nd()  # rerail-ignore calls\ne()  # rerail-ignore other\ns = \"# rerail-ignore calls\"\n";
        assert_eq!(
            reported(
                rule("calls", "python", "call", None),
                "python",
                "a.py",
                python
            ),
            ["a()", "e()"]
        );

        let php = "<?php\nfunction f() {\n    a();\n}\n// rerail-ignore calls\nfunction g() {\n    b();\n    c();\n}\nd(); # rerail-ignore calls\ne(); /* rerail-ignore other */\n";
        let php_rule = rule("calls", "php", "function_call_expression", None);
        assert_eq!(
            reported(php_rule.clone(), "php", "a.php", php),
            ["a()", "e()"]
        );
        let top = php.replacen("<?php\n", "<?php\n/* rerail-ignore calls */\n", 1);
        assert!(reported(php_rule, "php", "a.php", &top).is_empty());

        let blade = "<div>{{ a() }}</div>\n{{-- rerail-ignore calls --}}\n<p>{{ b() }}</p>\n@php\n    // rerail-ignore calls\n    c();\n    d();\n@endphp\n<span>{{ e() }}</span>\n";
        let blade_rule = rule("calls", "blade", "function_call_expression", None);
        assert_eq!(
            reported(blade_rule.clone(), "blade", "a.blade.php", blade),
            ["a()", "d()", "e()"]
        );
        let top = format!("{{{{-- rerail-ignore calls --}}}}\n{blade}");
        assert!(reported(blade_rule, "blade", "a.blade.php", &top).is_empty());

        let rust = "fn f() {\n    a();\n}\n// rerail-ignore calls\nfn g() {\n    b();\n    c();\n}\nfn h() {\n    d(); /* rerail-ignore calls */\n    e(); // rerail-ignore other\n    let s = \"// rerail-ignore calls\";\n}\n";
        let rust_rule = rule("calls", "rust", "call_expression", None);
        assert_eq!(
            reported(rust_rule.clone(), "rust", "a.rs", rust),
            ["a()", "e()"]
        );
        let top = format!("// rerail-ignore calls\n{rust}");
        assert!(reported(rust_rule, "rust", "a.rs", &top).is_empty());
    }

    /// A trailing directive after a list element's `,` or a class field's `;` covers that element
    /// or field (the ones before it on its line too), not the argument list, object or class body
    /// holding it: the later elements and members stay reported.
    #[test]
    fn a_trailing_directive_past_a_separator_covers_the_element_before_it() {
        let call = "foo(\n  a(), // rerail-ignore calls\n  b(),\n  c(),\n);\n";
        let pair = "foo(\n  a(), b(), // rerail-ignore calls\n  c(),\n);\n";
        let object = "const o = {\n  a: 1, // rerail-ignore keys\n  b: 2,\n};\n";
        let class = "class A {\n  x = f(); // rerail-ignore members\n  m() {}\n  n() {}\n}\n";
        let python = "foo(\n    a(),  # rerail-ignore calls\n    b(),\n)\n";
        for (language, source, expected) in [
            (GrammarDialect::TypeScript, call, cover("a()", &["calls"])),
            (
                GrammarDialect::TypeScript,
                pair,
                cover("a(), b()", &["calls"]),
            ),
            (GrammarDialect::TypeScript, object, cover("a: 1", &["keys"])),
            (
                GrammarDialect::TypeScript,
                class,
                cover("x = f()", &["members"]),
            ),
            (GrammarDialect::Python, python, cover("a()", &["calls"])),
        ] {
            assert_eq!(covered_texts(language, source), [expected], "{source:?}");
        }

        let calls = rule("calls", "typescript", "call_expression", None);
        assert_eq!(
            reported(calls, "typescript", "a.ts", call),
            [call.trim_end().trim_end_matches(';'), "b()", "c()"]
        );
        let mut members = rule("members", "typescript", "public_field_definition", None);
        members["clauses"][0]["forbid"]["ast"]["rule"] = json!({ "any": [
            { "kind": "public_field_definition" }, { "kind": "method_definition" }
        ] });
        assert_eq!(
            reported(members, "typescript", "a.ts", class),
            ["m() {}", "n() {}"]
        );
    }

    /// A covered node's range excludes its end: a leading directive over `a();` leaves `b();`,
    /// which starts where `a();` ends, reported, and a trailing one covers `b();` only. An end
    /// anchor names its witness's last byte, so `a();`'s, at `b();`'s start, stays covered by the
    /// directive over `a();` and out of the one over `b();`.
    #[test]
    fn a_directive_covers_no_sibling_starting_where_its_node_ends() {
        let leading = "function f() {\n// rerail-ignore probe\na();b();\n}\n";
        let spaced = "function f() {\n// rerail-ignore probe\na(); b();\n}\n";
        let trailing = "a();b(); // rerail-ignore probe\n";
        for (source, expected) in [(leading, "b();"), (spaced, "b();"), (trailing, "a();")] {
            for (anchor, at) in [
                ("witness.span", source.find(expected).expect("statement")),
                ("witness.start", source.find(expected).expect("statement")),
                (
                    "witness.end",
                    source.find(expected).expect("statement") + expected.len(),
                ),
            ] {
                let mut document = rule("probe", "typescript", "expression_statement", None);
                document["clauses"][0]["violation"]["anchor"] = json!(anchor);
                let compiled = compile_document_with(
                    document,
                    &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
                )
                .unwrap_or_else(|error| panic!("{}", error.reason));
                let result = evaluate_file(&compiled, "typescript", "a.ts", source);
                assert_eq!(result["failures"], json!([]), "{result}");
                let starts = result["violations"]
                    .as_array()
                    .expect("violations")
                    .iter()
                    .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
                    .collect::<Vec<_>>();
                assert_eq!(starts, [at as u64], "{anchor} in {source:?}");
            }
        }
    }
