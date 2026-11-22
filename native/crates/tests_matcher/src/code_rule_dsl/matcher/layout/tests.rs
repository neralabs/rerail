    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn python(rule: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        compile_document_with(
            clause_document(json!({ "id": "c", "violation": violation(),
                "forbid": { "ast": { "language": "python", "rule": rule } } })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason))
    }

    #[test]
    fn span_counts_the_line_breaks_inside_the_node() {
        let one_line = compiled(
            json!({ "rule": { "kind": "if_statement", "layout": { "span": { "eq": 0 } } } }),
        );
        let source = "if (a) b();\nif (c)\n  d();\nif (e) {\n}\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&one_line, "typescript", "a.ts", source),
                source
            ),
            ["if (a) b();"]
        );
        let two = compiled(
            json!({ "rule": { "kind": "if_statement", "layout": { "span": { "min": 1, "max": 1 } } } }),
        );
        assert_eq!(
            matched_texts(&evaluate_file(&two, "typescript", "a.ts", source), source),
            ["if (c)\n  d();", "if (e) {\n}"]
        );
        // ESLint breaks lines at CR, CRLF, U+2028 and U+2029 too; Ruff not at the last two.
        let string =
            compiled(json!({ "rule": { "kind": "string", "layout": { "span": { "eq": 1 } } } }));
        let source = "x = 'a\u{2028}b';\ny = `c\r\nd`;\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&string, "typescript", "a.ts", source),
                source
            ),
            ["'a\u{2028}b'"]
        );
        let string = python(json!({ "kind": "string", "layout": { "span": { "min": 1 } } }));
        let source = "x = 'a\u{2028}b'\ny = '''c\rd'''\n";
        assert_eq!(
            matched_texts(&evaluate_file(&string, "python", "a.py", source), source),
            ["'''c\rd'''"]
        );
    }

    #[test]
    fn start_reads_the_node_s_first_byte() {
        // Ruff's file-level diagnostics sit at byte 0, a shebang's too.
        let first = python(json!({ "kind": "comment", "layout": { "start": { "eq": 0 } } }));
        let source = "#!/usr/bin/env python\nimport os  # a comment\n";
        assert_eq!(
            matched_texts(&evaluate_file(&first, "python", "a.py", source), source),
            ["#!/usr/bin/env python"]
        );
        let later = python(
            json!({ "kind": "identifier", "layout": { "start": { "min": 30, "max": 40 } } }),
        );
        assert_eq!(
            matched_texts(&evaluate_file(&later, "python", "a.py", source), source),
            Vec::<String>::new()
        );
        let source = "import os\n";
        assert_eq!(
            matched_texts(&evaluate_file(&first, "python", "a.py", source), source),
            Vec::<String>::new()
        );
        let statement =
            python(json!({ "kind": "import_statement", "layout": { "start": { "eq": 0 } } }));
        assert_eq!(
            matched_texts(&evaluate_file(&statement, "python", "a.py", source), source),
            ["import os"]
        );
    }

    #[test]
    fn a_missing_neighbour_fails_the_gap_unless_the_file_s_edge_stands_in() {
        let gap = |side: &str, edge: bool, breaks: u64| {
            compiled(json!({ "rule": { "kind": "comment", "layout": {
                side: { "to": "token", "edge": edge, "breaks": { "eq": breaks } } } } }))
        };
        let source = "\n// head\nf(); // tail\n";
        let texts = |compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl| {
            matched_texts(
                &evaluate_file(compiled, "typescript", "a.ts", source),
                source,
            )
        };
        assert_eq!(texts(&gap("before", false, 1)), Vec::<String>::new());
        assert_eq!(texts(&gap("before", true, 1)), ["// head"]);
        assert_eq!(texts(&gap("after", false, 1)), ["// head"]);
        assert_eq!(texts(&gap("after", true, 1)), ["// head", "// tail"]);
        // The file's start stands in as a token on the first line.
        let blank = compiled(json!({ "rule": { "kind": "comment", "layout": {
            "before": { "to": "token", "edge": true, "blank": { "eq": 0 } } } } }));
        assert_eq!(texts(&blank), ["// head", "// tail"]);
    }

    #[test]
    fn comments_count_as_neighbours_only_when_asked() {
        let sibling = |comments: bool| {
            compiled(json!({ "rule": { "kind": "identifier", "regex": "^b$",
                "layout": { "before": { "to": "sibling", "comments": comments, "breaks": { "eq": 1 } } } } }))
        };
        let source = "f(a,\n  /* c */ b);\ng(a, /* c */\n  b);\n";
        let texts = |compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl| {
            matched_texts(
                &evaluate_file(compiled, "typescript", "a.ts", source),
                source,
            )
        };
        assert_eq!(texts(&sibling(false)), ["b", "b"]);
        assert_eq!(texts(&sibling(true)), ["b"]);
        // A capture must lie wholly on its side of the node.
        let capture = compiled(json!({ "rule": { "kind": "identifier", "regex": "^a$",
            "inside": { "kind": "call_expression", "pattern": "$C" },
            "layout": { "before": { "to": "capture.C", "breaks": { "min": 0 } } } } }));
        assert_eq!(texts(&capture), Vec::<String>::new());
    }

    #[test]
    fn a_negated_gap_is_unsound_under_syntax_errors() {
        let source = "f(a, b\nlet = ;\n";
        let failures = |rule: Value| {
            let result = evaluate_file(
                &compiled(json!({ "rule": rule })),
                "typescript",
                "a.ts",
                source,
            );
            result["failures"]
                .as_array()
                .expect("failures")
                .iter()
                .map(|failure| failure["code"].as_str().expect("code").to_owned())
                .collect::<Vec<_>>()
        };
        let gap = json!({ "before": { "to": "token", "breaks": { "eq": 0 } } });
        assert_eq!(
            failures(json!({ "kind": "identifier", "layout": gap })),
            ["source-partial"]
        );
        assert_eq!(
            failures(json!({ "kind": "identifier", "not": { "layout": gap } })),
            ["source-incomplete"]
        );
        assert_eq!(
            failures(json!({ "kind": "identifier", "not": { "layout": { "span": { "eq": 0 } } } })),
            ["source-partial"]
        );
    }
