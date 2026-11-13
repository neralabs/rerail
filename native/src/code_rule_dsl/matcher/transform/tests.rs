    use super::{Trans, replace_bounded};
    use crate::code_rule_dsl::CompiledCodeRuleDsl;
    use crate::code_rule_dsl::test_support::*;
    use regex::Regex;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn compile(ast: Value, violation: Value) -> napi::Result<CompiledCodeRuleDsl> {
        let mut ast = ast;
        if ast.get("language").is_none() {
            ast["language"] = json!("typescript");
        }
        compile_document_with(
            clause_document(json!({ "id": "c", "forbid": { "ast": ast }, "violation": violation })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    #[test]
    fn line_transforms_place_a_capture_with_its_documents_line_table() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use ast_grep_core::matcher::Pattern;
        use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
        use ast_grep_core::tree_sitter::StrDoc;
        use ast_grep_core::{AstGrep, Doc};
        let source = "a;\nb;\nlast;\n";
        let grammar = Grammar::new(GrammarDialect::TypeScript).unwrap();
        let native = StrDoc::new(source, GrammarDialect::TypeScript);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let root = AstGrep::doc(tape);
        let pattern = Pattern::try_new("last", GrammarDialect::TypeScript).unwrap();
        let node = root
            .root()
            .find(&pattern)
            .expect("identifier")
            .get_node()
            .clone();
        assert!(node.get_doc().get_source().ends_with("last;\n"));
        let mut env = MetaVarEnv::new();
        env.insert("X", node);
        let line = super::Trans::Line {
            source: super::Source {
                variable: MetaVariable::Capture("X".to_owned(), false),
                default: None,
                value: None,
            },
            end: false,
        };
        super::reset_line_table_constructions();
        assert_eq!(
            line.compute(&env, 1024).ok().flatten().as_deref(),
            Some("3")
        );
        assert_eq!(
            super::line_table_constructions(),
            0,
            "a tape document's own line table places the capture; none is built"
        );
    }

    fn output(ast: Value, message: &str, fix: Option<&str>, source: &str) -> Vec<(String, Value)> {
        let mut violation = json!({ "anchor": "witness.span", "code": "X", "message": message });
        if let Some(fix) = fix {
            violation["fix"] = json!(fix);
        }
        let compiled = compile(ast, violation).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                (
                    violation["message"].as_str().expect("message").to_owned(),
                    violation["fix"]["text"].clone(),
                )
            })
            .collect()
    }

    #[test]
    fn transforms_feed_messages_and_fixes() {
        let ast = json!({
            "rule": { "pattern": "console.log($A)" },
            "transform": {
                "TRIMMED": { "replace": { "source": "$A", "replace": "^['\"]\\s*|\\s*['\"]$", "by": "" } },
                "SHOUT": { "convert": { "source": "$TRIMMED", "toCase": "upperCase" } },
                "HEAD": { "substring": { "source": "$SHOUT", "endChar": 3 } },
                "SNAKE": { "convert": { "source": "$A", "toCase": "snakeCase", "separatedBy": ["caseChange"] } },
            },
        });
        let source = "console.log(' hello world ');\nconsole.log(userName);\n";
        assert_eq!(
            output(
                ast,
                "log $TRIMMED as $SHOUT ($HEAD, $SNAKE)",
                Some("logger.info(\"$SHOUT\")"),
                source
            ),
            [
                (
                    "log hello world as HELLO WORLD (HEL, '_ hello world ')".to_owned(),
                    json!("logger.info(\"HELLO WORLD\")")
                ),
                (
                    "log userName as USERNAME (USE, user_name)".to_owned(),
                    json!("logger.info(\"USERNAME\")")
                ),
            ]
        );
        // A source an `any` branch left unbound transforms to the empty text, as upstream.
        let ast = json!({
            "rule": { "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
            "transform": { "B": { "substring": { "source": "$A", "startChar": 1, "endChar": -1 } } },
        });
        assert_eq!(
            output(ast, "[$B]", None, "f(abcd);\ng();\n"),
            [
                ("[bc]".to_owned(), Value::Null),
                ("[]".to_owned(), Value::Null)
            ]
        );
    }

    /// `default` (a Rerail extension): the text an unbound source reads as, which the operation then
    /// transforms; without it an unbound source is the empty text and no operation runs, as
    /// upstream.
    #[test]
    fn a_default_is_the_text_of_an_unbound_source() {
        let ast = |default: Option<&str>| {
            let mut replace = json!({ "source": "$A", "replace": "^$", "by": "none" });
            if let Some(default) = default {
                replace["default"] = json!(default);
            }
            json!({
                "rule": { "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
                "transform": {
                    "B": { "replace": replace },
                    "C": { "substring": { "source": "$A", "startChar": 1, "default": "<unset>" } },
                },
            })
        };
        let source = "f(abcd);\ng();\nf('');\n";
        let messages = |default| {
            output(ast(default), "[$B] [$C]", None, source)
                .into_iter()
                .map(|(message, _)| message)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            messages(Some("")),
            ["[abcd] [bcd]", "[none] [unset>]", "[''] [']"]
        );
        assert_eq!(messages(None), ["[abcd] [bcd]", "[] [unset>]", "[''] [']"]);
        let ast = json!({
            "rule": { "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
            "transform": { "B": { "replace": { "source": "$A", "replace": "^$", "by": "none" } } },
        });
        assert_eq!(
            output(ast, "[$B]", Some("h($B)"), "f(abcd);\ng();\n"),
            [
                ("[abcd]".to_owned(), json!("h(abcd)")),
                ("[]".to_owned(), json!("h()"))
            ]
        );
        // A comparison reads the default where the source is unbound.
        let compared = json!({
            "rule": { "kind": "call_expression", "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }],
                      "has": { "kind": "arguments", "not": { "compare": { "left": "B", "op": "ne", "right": "U", "text": "source" } },
                               "pattern": "$U" } },
            "transform": { "B": { "replace": { "source": "$A", "replace": "^$", "by": "()", "default": "" } } },
        });
        assert_eq!(
            output(compared, "$B", None, "f(abcd);\ng();\n"),
            [("()".to_owned(), Value::Null)]
        );
    }

    /// `value: true` (a Rerail extension): the source capture's value (M9's `value`, not followed)
    /// as the host language converts it to a string, JavaScript's `String(v)` or Python's `str(v)`;
    /// a JavaScript identifier reads as its name, escapes decoded. An unknown value reads as an
    /// unbound source: the default, else the empty text.
    #[test]
    fn a_transform_reads_a_captures_value() {
        let ast = |language: &str| {
            json!({
                "language": language,
                "rule": { "kind": "call", "pattern": "f($A)" },
                "transform": {
                    "V": { "substring": { "source": "$A", "value": true, "default": "?" } },
                    "T": { "replace": { "source": "$A", "value": true, "replace": "^\\s+|\\s+$", "by": "" } },
                },
            })
        };
        let messages = |language: &str, source: &str| {
            let mut ast = ast(language);
            if language == "typescript" {
                ast["rule"]["kind"] = json!("call_expression");
            }
            let violation =
                json!({ "anchor": "witness.span", "code": "X", "message": "[$V] [$T]" });
            let compiled =
                compile(ast, violation).unwrap_or_else(|error| panic!("{}", error.reason));
            let path = if language == "python" { "a.py" } else { "a.ts" };
            let result = evaluate_file(&compiled, language, path, source);
            assert_eq!(result["failures"], json!([]), "{result}");
            result["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|violation| violation["message"].as_str().expect("message").to_owned())
                .collect::<Vec<_>>()
        };
        let source = "f('\\x61 b ');\nf(1e2);\nf(0x10);\nf(\\u0061);\nf({});\nf(null);\nf(`t${x}`);\nf(-0);\n";
        assert_eq!(
            messages("typescript", source),
            [
                "[a b ] [a b]",
                "[100] [100]",
                "[16] [16]",
                "[a] [a]",
                "[?] []",
                "[null] [null]",
                "[?] []",
                "[0] [0]"
            ]
        );
        let source =
            "f('a' 'b ')\nf(1e2)\nf(0x10)\nf(True)\nf(0.1 + 0.2)\nf(1e16)\nf(x)\nf(None)\nf(-7)\n";
        assert_eq!(
            messages("python", source),
            [
                "[ab ] [ab]",
                "[100.0] [100.0]",
                "[16] [16]",
                "[True] [True]",
                "[0.30000000000000004] [0.30000000000000004]",
                "[1e+16] [1e+16]",
                "[?] []",
                "[None] [None]",
                "[-7] [-7]",
            ]
        );
        // A comparison of a value that is not known is unknown, so a negated one reports nothing
        // there; with a default it reads the default.
        let differs = |default: Option<&str>| {
            let mut substring = json!({ "source": "$A", "value": true });
            if let Some(default) = default {
                substring["default"] = json!(default);
            }
            let ast = json!({
                "rule": { "pattern": "f($A, $B)",
                          "not": { "compare": { "left": "V", "op": "eq", "right": "B", "text": "source" } } },
                "transform": { "V": { "substring": substring } },
            });
            output(ast, "$A", None, "f(1e2, 100);\nf(1e2, 5);\nf(a.b, 5);\n")
                .into_iter()
                .map(|(message, _)| message)
                .collect::<Vec<_>>()
        };
        assert_eq!(differs(None), ["1e2"]);
        assert_eq!(differs(Some("")), ["1e2", "a.b"]);
        let reject = |transform: Value| {
            rejection(compile(
                json!({ "rule": { "pattern": "f($A, $$$R)" }, "transform": transform }),
                json!({ "anchor": "witness.span", "code": "X", "message": "m" }),
            ))
        };
        let prefix = "/clauses/0/forbid/ast/transform";
        assert_eq!(
            reject(json!({ "B": { "substring": { "source": "$$$R", "value": true } } })),
            format!("{prefix}/B/substring/value: a value is read from a single capture (`$NAME`)")
        );
        assert_eq!(
            reject(
                json!({ "B": { "substring": { "source": "$A" } }, "C": { "substring": { "source": "$B", "value": true } } })
            ),
            format!(
                "{prefix}/C/substring/value: `$B` is a transform variable, which is text; read the value of a capture"
            )
        );
        assert_eq!(
            reject(json!({ "B": { "substring": { "source": "$A", "value": "yes" } } })),
            format!("{prefix}/B/substring/value must be a boolean")
        );
    }

    #[test]
    fn line_lines_and_count_read_where_a_capture_stands() {
        // typescript-eslint 8.70.1's unified-signatures, a RuleTester case of
        // `tests/rules/unified-signatures.test.ts` ("For 3 or more overloads, mentions the line"):
        // the report at 5:6 names the other overload's line.
        let source = "\ninterface I {\n  a0(): void;\n  a0(x: string): string;\n  a0(x: number): void;\n}\n      ";
        let void = json!({ "has": { "kind": "type_annotation", "regex": "void" } });
        let mut other = json!({ "stopBy": "end", "kind": "method_signature", "pattern": "$O",
            "has": { "kind": "formal_parameters", "regex": "^\\(\\)$" } });
        other["all"] = json!([void.clone()]);
        let mut overload = json!({ "kind": "method_signature", "follows": other });
        overload["all"] = json!([void]);
        let ast = json!({
            "rule": { "kind": "required_parameter", "inside": { "kind": "formal_parameters", "inside": overload } },
            "transform": { "L": { "line": { "source": "$O" } } },
        });
        let message = "This overload and the one on line $L can be combined into one signature with an optional parameter.";
        let compiled = compile(
            ast,
            json!({ "anchor": "witness.span", "code": "X", "message": message }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        let violations = result["violations"].as_array().expect("findings");
        assert_eq!(violations.len(), 1, "{result}");
        assert_eq!(
            violations[0]["message"],
            "This overload and the one on line 3 can be combined into one signature with an optional parameter."
        );
        assert_eq!(
            (
                &violations[0]["span"]["startLine"],
                &violations[0]["span"]["startColumn"]
            ),
            (&json!(4), &json!(5)),
            "0-based: ESLint's 5:6"
        );
        // Counts: max-params' parameters, max-lines-per-function's lines; lines end where the
        // linter ends them (U+2028 in TypeScript).
        let ast = json!({
            "rule": { "kind": "function_declaration", "pattern": "function $F($$$P) { $$$ }" },
            "transform": {
                "N": { "count": { "source": "$$$P" } },
                "LINES": { "lines": { "source": "$F" } },
                "END": { "line": { "source": "$$$P", "edge": "end" } },
            },
        });
        let source =
            "function f(a, b,\n  c) {\n}\nfunction g() {}\nfunction h(x) { return `\u{2028}`; }\n";
        assert_eq!(
            output(
                ast,
                "$F: $N parameters, name on $LINES line, parameters end on line $END",
                None,
                source
            ),
            [
                (
                    "f: 3 parameters, name on 1 line, parameters end on line 2".to_owned(),
                    Value::Null
                ),
                (
                    "g: 0 parameters, name on 1 line, parameters end on line ".to_owned(),
                    Value::Null
                ),
                (
                    "h: 1 parameters, name on 1 line, parameters end on line 5".to_owned(),
                    Value::Null
                ),
            ]
        );
        let lines = json!({ "rule": { "kind": "function_declaration", "pattern": "$D" },
            "transform": { "N": { "lines": { "source": "$D" } } } });
        assert_eq!(
            output(lines, "$N", None, source)
                .into_iter()
                .map(|(message, _)| message)
                .collect::<Vec<_>>(),
            ["3", "1", "2"]
        );
        // Python's lines end at CR, LF and CRLF, not at U+2028.
        let python = compile(
            json!({ "language": "python", "rule": { "kind": "function_definition", "pattern": "$D" },
                "transform": { "S": { "line": { "source": "$D" } }, "E": { "line": { "source": "$D", "edge": "end" } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "$S-$E" }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "x = '\u{2028}'\rdef f():\r\n    pass\ndef g(): pass\n";
        let result = evaluate_file(&python, "python", "a.py", source);
        let messages = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message").to_owned())
            .collect::<Vec<_>>();
        assert_eq!(messages, ["2-3", "4-4"]);
        // Text has no lines: a transform variable is refused as their source.
        let refused = rejection(compile(
            json!({ "rule": { "pattern": "f($A)" }, "transform": {
                "T": { "substring": { "source": "$A" } }, "L": { "line": { "source": "$T" } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "m" }),
        ));
        assert_eq!(
            refused,
            "/clauses/0/forbid/ast/transform/L: `$T` is a transform variable, text; line, lines and count read a capture"
        );
    }

    #[test]
    fn to_precision_lays_out_a_number_as_ecmascript_does() {
        // ESLint's no-loss-of-precision compares a literal with `value.toPrecision(digits)`:
        // `9223372036854775808` is exact under 19 digits though `String` gives `…6000`. `Z` gives
        // the digits from another capture; `P` reads it, and is computed after it.
        let ast = json!({
            "rule": { "kind": "call_expression", "pattern": "f($N, $K)" },
            "transform": {
                "P": { "toPrecision": { "source": "$N", "digits": "$Z" } },
                "Q": { "toPrecision": { "source": "$N", "digits": 3 } },
                "Z": { "replace": { "source": "$K", "replace": "_", "by": "" } },
            },
        });
        let source = "f(9223372036854775808, 19);\nf(0x10, 2);\nf(1_000.5, 1_0);\nf(x, 2);\nf(1n, 2);\nf(5, 0);\n";
        let messages = output(ast, "[$P] [$Q]", None, source)
            .into_iter()
            .map(|(message, _)| message)
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            [
                "[9223372036854775808] [9.22e+18]",
                "[16] [16.0]",
                "[1000.500000] [1.00e+3]",
                "[] []",
                "[] []",
                "[] [5.00]",
            ]
        );
        for (transform, expected) in [
            (
                json!({ "P": { "toPrecision": { "source": "$N", "digits": 0 } } }),
                "/clauses/0/forbid/ast/transform/P/toPrecision/digits must be an integer from 1 to 100, or a `$NAME` variable",
            ),
            (
                json!({ "P": { "toPrecision": { "source": "$N" } } }),
                "/clauses/0/forbid/ast/transform/P/toPrecision/digits is required: an integer from 1 to 100, or a `$NAME` variable",
            ),
            (
                json!({ "P": { "toPrecision": { "source": "$N", "digits": "$$$D" } } }),
                "/clauses/0/forbid/ast/transform/P/toPrecision/digits: the digits are one text (`$NAME`), not a `$$$` list",
            ),
            (
                json!({ "P": { "toPrecision": { "source": "$N", "digits": "$D" } } }),
                "/clauses/0/forbid/ast/transform/P/digits: `$D` names no capture the query can bind",
            ),
            (
                json!({ "P": { "toPrecision": { "source": "$N", "digits": "$Q" } },
                        "Q": { "toPrecision": { "source": "$N", "digits": "$P" } } }),
                "/clauses/0/forbid/ast/transform/P: `P` has a cyclic dependency",
            ),
        ] {
            let ast = json!({
                "rule": { "kind": "call_expression", "pattern": "f($N, $K)" },
                "transform": transform,
            });
            let error = compile(
                ast,
                json!({ "anchor": "witness.span", "code": "X", "message": "x" }),
            )
            .err()
            .expect("refused");
            assert_eq!(error.reason, expected);
        }
    }

    #[test]
    fn to_precision_is_refused_outside_ecmascript() {
        // `toPrecision` lays a number out as ECMAScript does: a Python rule has no such numbers.
        let error = compile(
            json!({ "language": "python",
                "rule": { "kind": "call", "pattern": "f($N)" },
                "transform": { "P": { "toPrecision": { "source": "$N", "digits": 3 } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "x" }),
        )
        .err()
        .expect("refused");
        assert_eq!(
            error.reason,
            "/clauses/0/forbid/ast/transform/P/toPrecision: `toPrecision` lays a number out as ECMAScript's Number.prototype.toPrecision; Python numbers have no such layout"
        );
    }

    #[test]
    fn count_passes_over_comments() {
        // A `$$$` capture holds the comments between its nodes, as upstream's does; `count` counts
        // what ESLint's max-params and max-statements count, the parameters and statements.
        let ast = json!({
            "rule": {
                "kind": "function_declaration",
                "has": { "field": "parameters", "pattern": { "context": "function f($$$P) {}", "selector": "formal_parameters" } },
                "all": [{ "has": { "field": "body", "pattern": { "context": "function f() { $$$S }", "selector": "statement_block" } } }],
            },
            "transform": { "N": { "count": { "source": "$$$P" } }, "M": { "count": { "source": "$$$S" } } },
        });
        let source = "function f(a, /* c */ b) {}\nfunction g() { a; /* c */ b; // d\n}\nfunction h(/* only */) { /* none */ }\n";
        assert_eq!(
            output(ast, "$N $M ($$$P) {$$$S}", None, source),
            [
                ("2 0 (a, /* c */ b) {}".to_owned(), Value::Null),
                ("0 2 () {a; /* c */ b; // d}".to_owned(), Value::Null),
                ("0 0 (/* only */) {/* none */}".to_owned(), Value::Null),
            ]
        );
        // Python: a comment among the parameters.
        let python = compile(
            json!({ "language": "python",
                "rule": { "kind": "function_definition", "has": { "field": "parameters",
                    "pattern": { "context": "def f($$$P): pass", "selector": "parameters" } } },
                "transform": { "N": { "count": { "source": "$$$P" } } } }),
            json!({ "anchor": "witness.span", "code": "X", "message": "$N" }),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "def f(a,  # c\n      b):\n    x = 1\n";
        let result = evaluate_file(&python, "python", "a.py", source);
        assert_eq!(result["violations"][0]["message"], "2", "{result}");
    }

    #[test]
    fn a_regex_group_of_the_witness_is_a_transform_source() {
        // capitalized-comments' fix: the comment's first letter, lowercased, over the group.
        let ast = json!({
            "rule": { "kind": "comment", "regex": "^(?://|/\\*)[\\s*]*(?<L>\\p{Lu})(?<REST>x)?" },
            "transform": {
                "C": { "convert": { "source": "$L", "toCase": "lowerCase" } },
                "R": { "replace": { "source": "$REST", "default": "-", "replace": "^$", "by": "none" } },
                "AT": { "line": { "source": "$L" } },
            },
        });
        let mut violation = json!({ "anchor": "capture.L", "code": "X", "message": "$L to $C on line $AT, rest $R", "fix": { "template": "$C", "range": "capture.L" } });
        let compiled = compile(ast.clone(), violation.clone())
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "// Abc\n/* Éx */\n// abc\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        let rows = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                let start = violation["fix"]["span"]["startByte"].as_u64().expect("fix") as usize;
                let end = violation["fix"]["span"]["endByte"].as_u64().expect("fix") as usize;
                (
                    violation["message"].as_str().expect("message").to_owned(),
                    source[start..end].to_owned(),
                    violation["fix"]["text"].as_str().expect("fix").to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                (
                    "A to a on line 1, rest -".to_owned(),
                    "A".to_owned(),
                    "a".to_owned()
                ),
                (
                    "É to é on line 2, rest x".to_owned(),
                    "É".to_owned(),
                    "é".to_owned()
                ),
            ]
        );
        // With `each`, each match of the regex computes the transform from its own groups.
        let each = json!({ "rule": { "kind": "string", "regex": "(?<D>[0-9])" },
            "transform": { "N": { "replace": { "source": "$D", "replace": "^(.)$", "by": "<$1>" } } } });
        violation = json!({ "anchor": "capture.D", "code": "X", "each": "D", "message": "$N" });
        let compiled = compile(each, violation).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "f('1a2');\n");
        let mut messages = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message").to_owned())
            .collect::<Vec<_>>();
        messages.sort();
        assert_eq!(messages, ["<1>", "<2>"]);
        // A group is text: no value; and it is known only once the rule matched, so a comparison
        // cannot read a transform of it.
        let plain = json!({ "anchor": "witness.span", "code": "X", "message": "m" });
        assert_eq!(
            rejection(compile(
                json!({ "rule": { "kind": "comment", "regex": "(?<L>a)" },
                    "transform": { "V": { "substring": { "source": "$L", "value": true } } } }),
                plain.clone(),
            )),
            "/clauses/0/forbid/ast/transform/V/substring/value: `$L` is a regex group, text; read the value of a capture"
        );
        assert_eq!(
            rejection(compile(
                json!({ "rule": { "kind": "comment", "regex": "(?<L>a)", "has": { "stopBy": "end", "pattern": "$X" },
                    "compare": { "left": "C", "op": "eq", "right": "X", "text": "source" } },
                    "transform": { "C": { "substring": { "source": "$L" } } } }),
                plain,
            )),
            "/clauses/0/forbid/ast/rule/compare/left: `$C` transforms `$L`, which is never bound where this comparison runs"
        );
    }

    #[test]
    fn transforms_compare_as_text() {
        // no-duplicate-imports: module names compare trimmed.
        let ast = json!({
            "rule": {
                "kind": "import_statement",
                "has": { "kind": "string", "has": { "kind": "string_fragment", "pattern": "$M" } },
                "follows": { "stopBy": "end", "kind": "import_statement",
                    "has": { "kind": "string", "has": { "kind": "string_fragment", "pattern": "$P" } },
                    "compare": { "left": "PT", "op": "eq", "right": "MT" } },
            },
            "transform": {
                "MT": { "replace": { "source": "$M", "replace": "^\\s+|\\s+$", "by": "" } },
                "PT": { "replace": { "source": "$P", "replace": "^\\s+|\\s+$", "by": "" } },
            },
        });
        let source = "import a from ' m ';\nimport b from 'n';\nimport c from 'm';\n";
        assert_eq!(
            output(ast, "'$MT' import is duplicated.", None, source),
            [("'m' import is duplicated.".to_owned(), Value::Null)]
        );
    }

    /// A transform named like a capture: said plainly, before any use of the name is checked as
    /// a transform variable's (unified-signatures named its line `LA` beside a capture `$LA`, and
    /// its `index` comparison of `LA` was refused as a comparison of text).
    #[test]
    fn a_transform_named_like_a_capture_is_refused_as_such() {
        let violation = json!({ "anchor": "witness.span", "code": "X", "message": "m" });
        let rule = json!({ "kind": "call_expression", "has": { "field": "arguments",
            "pattern": { "context": "f($LA, $LB)", "selector": "arguments" } },
            "all": [{ "compare": { "left": "LA", "op": "eq", "right": "LB", "text": "index" } }] });
        assert_eq!(
            rejection(compile(
                json!({ "rule": rule, "transform": { "LA": { "line": { "source": "$LB" } } } }),
                violation.clone()
            )),
            "/clauses/0/forbid/ast/transform/LA: `LA` names a capture of the query (`$LA` at /clauses/0/forbid/ast/rule/has/pattern) and a transform variable; rename one"
        );
        // A capture a utility binds, a parse captures, or a call returns collides too.
        for (ast, at) in [
            (
                json!({ "rule": { "matches": "arg" }, "utils": { "arg": { "pattern": "f($N)" } },
                    "transform": { "N": { "line": { "source": "$N2" } } } }),
                "/clauses/0/forbid/ast/utils/arg/pattern",
            ),
            (
                json!({ "rule": { "kind": "comment", "parse": { "grammar": "eslintDirective", "has": { "kind": "item", "capture": "N" } } },
                    "transform": { "N": { "substring": { "source": "$X" } } } }),
                "/clauses/0/forbid/ast/rule/parse/has/capture",
            ),
        ] {
            let message = rejection(compile(ast, violation.clone()));
            assert!(
                message.starts_with(
                    "/clauses/0/forbid/ast/transform/N: `N` names a capture of the query (`$N` at "
                ) && message.contains(at),
                "{message}"
            );
        }
    }

    #[test]
    fn transforms_are_checked_against_the_query() {
        let violation = || json!({ "anchor": "witness.span", "code": "X", "message": "m" });
        let reject = |ast: Value, violation: Value| rejection(compile(ast, violation));
        let pattern = json!({ "pattern": "f($A, $C)" });
        let prefix = "/clauses/0/forbid/ast/transform";
        for (transform, expected) in [
            (
                json!({ "A": { "substring": { "source": "$C" } } }),
                format!(
                    "{prefix}/A: `A` names a capture of the query (`$A` at /clauses/0/forbid/ast/rule/pattern) and a transform variable; rename one"
                ),
            ),
            (
                json!({ "B": { "substring": { "source": "$E" } } }),
                format!("{prefix}/B/source: `$E` names no capture the query can bind"),
            ),
            (
                json!({ "B": { "substring": { "source": "$D" } }, "D": { "substring": { "source": "$B" } } }),
                format!("{prefix}/B: `B` has a cyclic dependency"),
            ),
            (
                json!({ "B": { "substring": { "source": "A" } } }),
                format!(
                    "{prefix}/B/substring/source: `A` should be a $-prefixed metavariable (`$NAME` or `$$$NAME`)"
                ),
            ),
            (
                json!({ "B": "substring($A, startChar=1)" }),
                format!(
                    "{prefix}/B must be an object with one of {}; the string form is not supported",
                    Trans::kinds_listed()
                ),
            ),
            (
                json!({ "B": { "rewrite": { "source": "$A", "rewriters": ["r"] } } }),
                format!(
                    "{prefix}/B/rewrite: rewrite is not supported yet; use replace, substring or convert"
                ),
            ),
            (
                json!({ "B": { "replace": { "source": "$A", "replace": "(", "by": "" } } }),
                format!("{prefix}/B/replace/replace: Parsing text matcher fails."),
            ),
            (
                json!({ "B": { "convert": { "source": "$A", "toCase": "titleCase" } } }),
                format!(
                    "{prefix}/B/convert/toCase: unknown value \"titleCase\"; expected one of lowerCase, upperCase, capitalize, camelCase, snakeCase, kebabCase, pascalCase"
                ),
            ),
            (
                json!({ "B": { "substring": { "source": "$A", "startChar": 1.5 } } }),
                format!("{prefix}/B/substring/startChar must be an integer"),
            ),
            (
                json!({ "b": { "substring": { "source": "$A" } } }),
                format!("{prefix}/b: `b` is not a metavariable name (A-Z, 0-9, _, without `$`)"),
            ),
            (
                json!({ "B": { "substring": { "source": "$A" }, "convert": { "source": "$A", "toCase": "lowerCase" } } }),
                format!(
                    "{prefix}/B must hold exactly one of {}",
                    Trans::kinds_listed()
                ),
            ),
            (
                json!({ "B": { "substring": { "source": "$A", "default": 1 } } }),
                format!("{prefix}/B/substring/default must be a string"),
            ),
        ] {
            let ast = json!({ "rule": pattern, "transform": transform });
            assert_eq!(reject(ast, violation()), expected);
        }
        let transform = json!({ "B": { "substring": { "source": "$A" } } });
        let mut multi = violation();
        multi["message"] = json!("$$$B");
        assert_eq!(
            reject(json!({ "rule": pattern, "transform": transform }), multi),
            "/clauses/0/violation/message: `$$$B`: a transform variable is text; use `$B`"
        );
        let mut anchor = violation();
        anchor["anchor"] = json!("capture.B");
        assert!(
            reject(json!({ "rule": pattern, "transform": transform }), anchor)
                .starts_with("/clauses/0/violation/anchor: `capture.B` names no capture")
        );
        let compare = |text: &str| json!({ "pattern": "f($A, $C)", "compare": { "left": "B", "op": "eq", "right": "C", "text": text } });
        assert_eq!(
            reject(
                json!({ "rule": compare("tokens"), "transform": transform }),
                violation()
            ),
            "/clauses/0/forbid/ast/rule/compare/text: a transform variable is text; compare it as source or unquoted"
        );
        compile(
            json!({ "rule": compare("unquoted"), "transform": transform }),
            violation(),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        // A comparison sees a transform only where its source is bound.
        let hidden = json!({ "pattern": "f($A, $C)", "inside": { "stopBy": {
            "kind": "program", "compare": { "left": "B", "op": "eq", "right": "C" },
        }, "kind": "program" } });
        assert_eq!(
            reject(
                json!({ "rule": hidden, "transform": transform }),
                violation()
            ),
            "/clauses/0/forbid/ast/rule/inside/stopBy/compare/left: `$B` transforms `$A`, which is never bound where this comparison runs"
        );
    }

    #[test]
    fn transform_output_is_limited_by_max_source_bytes() {
        let mut transform = serde_json::Map::new();
        let mut source = "$A".to_owned();
        for index in 0..20 {
            let name = format!("T{index}");
            transform.insert(
                name.clone(),
                json!({ "replace": { "source": source, "replace": "^.*$", "by": "$0$0" } }),
            );
            source = format!("${name}");
        }
        let ast = json!({
            "language": "typescript",
            "rule": { "pattern": "console.log($A)" },
            "transform": transform,
        });
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxSourceBytes"] = json!(64);
        });
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "c",
                "forbid": { "ast": ast },
                "violation": { "anchor": "witness.span", "code": "X", "message": source }
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", "console.log(x);\n");
        assert_eq!(
            result["failures"],
            json!([{
                "clauseId": "c",
                "code": "budget-exceeded",
                "message": "`matches` and `count` exceeded their work budget (maxVisitedNodes utility evaluations and counted nodes per file)",
                "path": "a.ts"
            }]),
            "{result}"
        );
    }

    #[test]
    fn replacement_budget_counts_expanded_output_not_match_length() {
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxSourceBytes"] = json!(64);
        });
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "c",
                "forbid": { "ast": {
                    "language": "typescript",
                    "rule": { "pattern": "console.log($A)" },
                    "transform": {
                        "B": { "replace": { "source": "$A", "replace": ".*", "by": "ok" } }
                    }
                }},
                "violation": { "anchor": "witness.span", "code": "X", "message": "$B" }
            })),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(
            &compiled,
            "typescript",
            "a.ts",
            "console.log(aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa);\n",
        );
        assert_eq!(result["failures"], json!([]), "{result}");
        assert_eq!(result["violations"][0]["message"], "ok");
    }

    #[test]
    fn bounded_replacement_matches_regex_replacement_syntax() {
        let regex = Regex::new("(?P<name>x)").expect("regex");
        for replacement in [
            "$", "$$", "$$1", "$1", "${1}", "$name", "${name}", "cost$", "$!", "é$name$",
        ] {
            assert_eq!(
                replace_bounded("x", &regex, replacement, 64)
                    .unwrap_or_else(|_| panic!("within budget")),
                regex.replace_all("x", replacement),
                "{replacement}",
            );
        }
    }
