    use super::super::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
    use super::*;

    const CAPABILITIES: &str = include_str!("../capabilities.json");

    fn compile(forbid: Value, violation: Value) -> Result<CompiledCodeRuleDsl> {
        compile_clause(json!({ "id": "c", "forbid": forbid, "violation": violation }))
    }

    fn compile_clause(clause: Value) -> Result<CompiledCodeRuleDsl> {
        let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
        capabilities["factRelations"] = json!({ "symbols": {} });
        let document = json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "mode": "block", "clauses": [clause] });
        CodeRuleDslRuntime::compile(
            document.to_string(),
            "json".to_owned(),
            capabilities.to_string(),
        )
    }

    fn ast(rule: Value) -> Value {
        json!({ "ast": { "language": "typescript", "rule": rule } })
    }

    fn violation(anchor: &str, message: &str) -> Value {
        json!({ "anchor": anchor, "code": "X", "message": message })
    }

    fn with_fix(anchor: &str, fix: &str) -> Value {
        json!({ "anchor": anchor, "code": "X", "message": "m", "fix": fix })
    }

    fn violations_for(compiled: &CompiledCodeRuleDsl, language: &str, source: &str) -> Vec<Value> {
        let snapshot = json!({
            "facts": [],
            "files": [{ "language": language, "path": "a.ts", "source": source }],
            "format": "rerail.repository-snapshot/v1",
            "revision": 0,
        });
        let output = compiled
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect("evaluation succeeds");
        let result: Value = serde_json::from_str(&output).expect("JSON");
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"].as_array().expect("findings").clone()
    }

    fn only(compiled: &CompiledCodeRuleDsl, source: &str) -> Value {
        let found = violations_for(compiled, "typescript", source);
        assert_eq!(found.len(), 1, "{found:?}");
        found[0].clone()
    }

    fn point(span: &Value) -> (u64, u64, u64, u64) {
        let field = |name: &str| span[name].as_u64().expect(name);
        (
            field("startLine"),
            field("startColumn"),
            field("endLine"),
            field("endColumn"),
        )
    }

    fn rejection(result: Result<CompiledCodeRuleDsl>) -> String {
        result.err().expect("compile is rejected").reason
    }

    #[test]
    fn messages_keep_escaped_and_digit_dollar_names_literal() {
        // Review finding 5 on #2561: a message had no way to say `$HOME` or `$100`. `\$NAME` is
        // literal text, and a name starting with a digit, which nothing can bind, stays text too
        // (ast-grep renders both empty). Fix templates keep ast-grep's reading.
        let compiled = compile(
            ast(json!({ "pattern": "foo($A)" })),
            violation("witness.span", r"set \$HOME to $A, pay $100, keep \x"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            only(&compiled, "foo(bar);\n")["message"],
            r"set $HOME to bar, pay $100, keep \x"
        );
        let literal = compile(
            json!({ "regex": { "source": "file.content", "pattern": "foo" } }),
            violation("witness.span", r"costs $5, see \$DOC"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(only(&literal, "foo\n")["message"], "costs $5, see $DOC");
        assert_eq!(
            rejection(compile(
                ast(json!({ "pattern": "foo($A)" })),
                with_fix("witness.span", "bar($5)")
            )),
            "/clauses/0/violation/fix: `$5` names no capture; the query can bind: A"
        );
    }

    #[test]
    fn a_regex_without_named_groups_carries_its_literal_fix() {
        // Review finding 4 on #2561: the group-less fast path built witnesses without a fix.
        for pattern in ["debugger;", "(?<D>debugger;)"] {
            let compiled = compile(
                json!({ "regex": { "source": "file.content", "pattern": pattern } }),
                with_fix("witness.span", ""),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason));
            let found = only(&compiled, "debugger;\n");
            assert_eq!(found["fix"]["text"], "", "{pattern}: {found}");
            assert_eq!(point(&found["fix"]["span"]), (0, 0, 0, 9), "{pattern}");
        }
    }

    #[test]
    fn templates_split_metavariables_like_ast_grep() {
        let names = |text: &str| {
            Template::parse(text)
                .vars()
                .map(|(name, multi)| format!("{}{name}", if multi { "$$$" } else { "$" }))
                .collect::<Vec<_>>()
        };
        assert_eq!(names("Found $A in $$$ARGS."), ["$A", "$$$ARGS"]);
        assert_eq!(names("$$A"), ["$A"]);
        assert_eq!(names("costs $ 5, $lower, $$$$A"), ["$$$A"]);
        assert_eq!(names("$A1_B-c"), ["$A1_B"]);
        let rendered = Template::parse("<$A|$B>")
            .render(|name, _| (name == "A").then_some(Cow::Borrowed("x")));
        assert_eq!(rendered, "<x|>");
        assert!(Template::parse("plain $ text").is_literal());
    }

    /// `${NAME}` (a Rerail extension; ast-grep 0.45.3 renders it literally) names a variable next
    /// to text that would continue its name, in messages and fixes alike; `\${NAME}` is a
    /// backslash before the variable, where `\$NAME` stays the escape.
    #[test]
    fn braced_variables_sit_next_to_name_characters() {
        let names = |text: &str| {
            Template::parse(text)
                .vars()
                .map(|(name, multi)| format!("{}{name}", if multi { "$$$" } else { "$" }))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("${A}B $$${ARGS}_1 ${a} ${A $${B}"),
            ["$A", "$$$ARGS", "$B"]
        );
        let compiled = compile(
            ast(json!({ "pattern": "f($A, $$$R)" })),
            violation("witness.span", r"${A}_X [$$${R}] \${A} \$A ${a}"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            only(&compiled, "f(b, c, d);\n")["message"],
            r"b_X [c, d] \b $A ${a}"
        );
        let fixed = |fix: &str, source: &str| {
            let compiled = compile(
                ast(json!({ "pattern": "f($A, $$$R)" })),
                with_fix("witness.span", fix),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason));
            only(&compiled, source)["fix"]["text"].clone()
        };
        assert_eq!(
            fixed("g_${A}_SUFFIX(${A}X, $$${R}) $${A}$", "f(b, c, d);\n"),
            "g_b_SUFFIX(bX, c, d) $b$"
        );
        // Re-indented as the unbraced template is.
        let source = "function k() {\n  f(a, {\n    b,\n  });\n}\n";
        assert_eq!(
            fixed("if (x) {\n  ${A}Z($$$R);\n}", source),
            json!(
                fixed("if (x) {\n  $A($$$R);\n}", source)
                    .as_str()
                    .expect("text")
                    .replacen("a(", "aZ(", 1)
            )
        );
        let regex = compile(
            json!({ "regex": { "source": "file.content", "pattern": "(?<N>[0-9]+)" } }),
            with_fix("witness.span", "${N}0"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(only(&regex, "7\n")["fix"]["text"], "70");
        assert_eq!(
            rejection(compile(
                ast(json!({ "pattern": "f($A)" })),
                violation("witness.span", "${NOPE}")
            )),
            r"/clauses/0/violation/message: `$NOPE` names no capture; the query can bind: A; write `\$NOPE` for literal text"
        );
    }

    #[test]
    fn long_or_multi_line_captures_are_cut_in_messages() {
        assert_eq!(message_capture("short"), "short");
        assert_eq!(message_capture("f(\n  a,\n)"), "f(…");
        let long = "x".repeat(MESSAGE_CAPTURE_CHARS + 5);
        let cut = message_capture(&long);
        assert_eq!(cut.chars().count(), MESSAGE_CAPTURE_CHARS);
        assert!(cut.ends_with('…'));
        let exact = "y".repeat(MESSAGE_CAPTURE_CHARS);
        assert_eq!(message_capture(&exact), exact);
    }

    #[test]
    fn witness_start_and_end_are_zero_width_points() {
        let source = "let a = 1;\nconsole.log(a,\n  b);\n";
        let rule = json!({ "pattern": "console.log($$$ARGS)" });
        let span = only(
            &compile(ast(rule.clone()), violation("witness.span", "m")).unwrap(),
            source,
        );
        assert_eq!(point(&span["span"]), (1, 0, 2, 4));
        let start = only(
            &compile(ast(rule.clone()), violation("witness.start", "m")).unwrap(),
            source,
        );
        assert_eq!(point(&start["span"]), (1, 0, 1, 0));
        assert_eq!(start["span"]["startByte"], start["span"]["endByte"]);
        let end = only(
            &compile(ast(rule), violation("witness.end", "m")).unwrap(),
            source,
        );
        assert_eq!(point(&end["span"]), (2, 4, 2, 4));
        // Anchors never change violation identity: the id stays the witness's.
        assert_eq!(start["id"], span["id"]);
        assert_eq!(end["id"], span["id"]);
    }

    #[test]
    fn capture_anchors_point_at_the_capture() {
        let source = "foo(first, second, third);\n";
        let rule = || ast(json!({ "pattern": "foo($A, $$$REST)" }));
        let single = compile(rule(), violation("capture.A", "m")).unwrap();
        assert_eq!(point(&only(&single, source)["span"]), (0, 4, 0, 9));
        let multi = compile(rule(), violation("capture.REST", "m")).unwrap();
        assert_eq!(point(&only(&multi, source)["span"]), (0, 11, 0, 24));
        // A capture bound through a relation anchors on the related node's capture.
        let related = compile(
            ast(json!({
                "kind": "identifier",
                "regex": "^second$",
                "inside": { "pattern": "foo($$$ARGS)", "stopBy": "end" },
            })),
            violation("capture.ARGS", "m"),
        )
        .unwrap();
        assert_eq!(point(&only(&related, source)["span"]), (0, 4, 0, 24));
    }

    #[test]
    fn a_capture_absent_at_runtime_falls_back_to_the_witness_span() {
        let compiled = compile(
            ast(json!({ "any": [{ "pattern": "foo($A)" }, { "pattern": "bar()" }] })),
            violation("capture.A", "call of $A"),
        )
        .unwrap();
        let found = violations_for(&compiled, "typescript", "foo(x);\nbar();\n");
        assert_eq!(found.len(), 2);
        assert_eq!(point(&found[0]["span"]), (0, 4, 0, 5));
        assert_eq!(found[0]["variables"], json!({}));
        assert_eq!(found[0]["message"], "call of x");
        assert_eq!(point(&found[1]["span"]), (1, 0, 1, 5));
        assert_eq!(found[1]["variables"], json!({ "anchorFallback": true }));
        assert_eq!(found[1]["message"], "call of ");
    }

    #[test]
    fn messages_interpolate_single_multi_and_long_captures() {
        let compiled = compile(
            ast(json!({ "pattern": "$F($A, $$$REST)" })),
            violation("witness.span", "Call $F($A) with [$$$REST]"),
        )
        .unwrap();
        let found = only(&compiled, "run(first, a + b, c);\n");
        assert_eq!(found["message"], "Call run(first) with [a + b, c]");
        let found = only(&compiled, &format!("run({}, x);\n", "v".repeat(100)));
        let cut = "v".repeat(MESSAGE_CAPTURE_CHARS - 1);
        assert_eq!(found["message"], format!("Call run({cut}…) with [x]"));
        let bare = compile(
            ast(json!({ "pattern": "$F($$$ARGS)" })),
            violation("witness.span", "$F takes [$$$ARGS]"),
        )
        .unwrap();
        assert_eq!(only(&bare, "run();\n")["message"], "run takes []");
    }

    #[test]
    fn ast_fixes_substitute_captures_and_reindent() {
        let compiled = compile(
            ast(json!({ "pattern": "console.log($$$ARGS)" })),
            with_fix("witness.span", "logger.info($$$ARGS)"),
        )
        .unwrap();
        let found = only(
            &compiled,
            "function f() {\n  console.log(a, {\n    b,\n  });\n}\n",
        );
        assert_eq!(found["fix"]["text"], "logger.info(a, {\n    b,\n  })");
        assert_eq!(point(&found["fix"]["span"]), (1, 2, 3, 4));
        assert_eq!(found["fix"]["span"]["startByte"], 17);
        let block = compile(
            ast(json!({ "pattern": "console.log($A)" })),
            with_fix("witness.end", "if (debug) {\n  console.log($A);\n}"),
        )
        .unwrap();
        let found = only(&block, "function f() {\n  console.log(a);\n}\n");
        assert_eq!(
            found["fix"]["text"],
            "if (debug) {\n    console.log(a);\n  }"
        );
        assert_eq!(point(&found["span"]), (1, 16, 1, 16));

        // Each dialect compiles its own template: the same rule fixes JSX in a .tsx file.
        let jsx = compile(
            ast(json!({ "pattern": "<button $$$ATTRS>$$$BODY</button>" })),
            with_fix("witness.span", "<Button $$$ATTRS>$$$BODY</Button>"),
        )
        .unwrap();
        let found = violations_for(
            &jsx,
            "tsx",
            "const v = <button type=\"button\">Go</button>;\n",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0]["fix"]["text"],
            "<Button type=\"button\">Go</Button>"
        );
        assert!(violations_for(&jsx, "typescript", "const v = 1;\n").is_empty());
    }

    #[test]
    fn captures_bound_in_utilities_and_constraints_reach_violations() {
        // A `matches` target shares the caller's environment, so its captures anchor, interpolate,
        // and fix; a recursive utility is walked once.
        let util = || {
            json!({ "ast": {
                "language": "typescript",
                "rule": { "kind": "return_statement", "has": { "matches": "tail" } },
                "utils": { "tail": { "any": [
                    { "pattern": "await $E" },
                    { "kind": "parenthesized_expression", "has": { "matches": "tail" } },
                ] } },
            } })
        };
        let compiled = compile(
            util(),
            json!({ "anchor": "capture.E", "code": "X", "message": "awaits $E", "fix": "return $E;" }),
        )
        .unwrap();
        let found = only(
            &compiled,
            "async function f() {\n  return ((await g()));\n}\n",
        );
        assert_eq!(found["message"], "awaits g()");
        assert_eq!(point(&found["span"]), (1, 17, 1, 20));
        assert_eq!(found["fix"]["text"], "return g();");
        assert_eq!(point(&found["fix"]["span"]), (1, 2, 1, 23));

        // A constraint runs on the captured node with the same environment; what it binds counts.
        let constrained = || {
            json!({ "ast": {
                "language": "typescript",
                "rule": { "pattern": "$F($A)" },
                "constraints": { "A": { "pattern": "$L + $R" } },
            } })
        };
        let compiled = compile(
            constrained(),
            json!({ "anchor": "capture.R", "code": "X", "message": "$F adds $L and $R", "fix": "$F(`${$L}${$R}`)" }),
        )
        .unwrap();
        let found = only(&compiled, "log(a + b);\nlog(c);\n");
        assert_eq!(found["message"], "log adds a and b");
        assert_eq!(point(&found["span"]), (0, 8, 0, 9));
        assert_eq!(found["fix"]["text"], "log(`${a}${b}`)");

        // Names bound only under `not`, in a utility or a constraint, never exist.
        for (forbid, name, bindable) in [
            (
                json!({ "ast": {
                    "language": "typescript",
                    "rule": { "matches": "call" },
                    "utils": { "call": { "kind": "call_expression", "not": { "pattern": "bar($N)" } } },
                } }),
                "N",
                "the query binds no captures",
            ),
            (
                json!({ "ast": {
                    "language": "typescript",
                    "rule": { "pattern": "$F($A)" },
                    "constraints": { "A": { "kind": "identifier", "not": { "pattern": "$M.x" } } },
                } }),
                "M",
                "the query can bind: A, F",
            ),
        ] {
            assert_eq!(
                rejection(compile(forbid, violation(&format!("capture.{name}"), "m"))),
                format!(
                    "/clauses/0/violation/anchor: `capture.{name}` names no capture; {bindable}"
                )
            );
        }
    }

    #[test]
    fn regex_named_groups_are_captures() {
        let pattern = "(?<KEY>[a-z]+)=(?<VALUE>\\d+)(?<unit>ms)?";
        let regex = || json!({ "regex": { "source": "file.content", "pattern": pattern } });
        let compiled = compile(
            regex(),
            json!({ "anchor": "capture.VALUE", "code": "X", "message": "set $KEY to $VALUE", "fix": "$KEY: $VALUE" }),
        )
        .unwrap();
        let found = only(&compiled, "  width=10\n");
        assert_eq!(point(&found["span"]), (0, 8, 0, 10));
        assert_eq!(found["message"], "set width to 10");
        assert_eq!(found["fix"]["text"], "width: 10");
        assert_eq!(point(&found["fix"]["span"]), (0, 2, 0, 10));
        let captures = found["captures"].as_object().expect("captures");
        assert_eq!(captures.keys().collect::<Vec<_>>(), ["KEY", "VALUE"]);
        let unit = compile(regex(), violation("capture.unit", "m")).unwrap();
        let found = violations_for(&unit, "typescript", "a=1ms b=2\n");
        assert_eq!(point(&found[0]["span"]), (0, 3, 0, 5));
        assert_eq!(found[1]["variables"], json!({ "anchorFallback": true }));
    }

    #[test]
    fn named_groups_in_a_same_node_regex_capture_sub_spans() {
        let comment = compile(
            ast(json!({ "kind": "comment", "any": [
                { "regex": "^//+(?<BODY>[^\\s/].*)$" },
                { "regex": "^/\\*\\*?\\s*(?<BODY>.*?)\\s*\\*/$" },
            ] })),
            violation("capture.BODY", "comment `$BODY`"),
        )
        .unwrap();
        let found = violations_for(&comment, "typescript", "x(); ////Tight\n/** doc */\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(point(&found[0]["span"]), (0, 9, 0, 14));
        assert_eq!(found[0]["message"], "comment `Tight`");
        assert_eq!(point(&found[1]["span"]), (1, 4, 1, 7));
        assert_eq!(found[1]["captures"]["BODY"][0]["startByte"], 19);
        // A metavariable of the same name wins over the group.
        let shadowed = compile(
            ast(json!({ "pattern": "foo($A)", "regex": "^(?<A>f)oo" })),
            violation("capture.A", "$A"),
        )
        .unwrap();
        let found = only(&shadowed, "foo(bar);\n");
        assert_eq!(point(&found["span"]), (0, 4, 0, 7));
        assert_eq!(found["message"], "bar");
        // Groups of a regex that tests another node (a relation or `not`) are not captures.
        for rule in [
            json!({ "kind": "identifier", "inside": { "kind": "call_expression", "regex": "^(?<G>f)" } }),
            json!({ "kind": "identifier", "not": { "regex": "^(?<G>f)" } }),
        ] {
            let reason = rejection(compile(ast(rule), violation("capture.G", "m")));
            assert_eq!(
                reason,
                "/clauses/0/violation/anchor: `capture.G` names no capture; the query binds no captures"
            );
        }
        let reason = rejection(compile(
            ast(json!({ "kind": "comment", "regex": "^//(?<B>.*)" })),
            with_fix("witness.span", "// $B"),
        ));
        assert_eq!(
            reason,
            "/clauses/0/violation/fix: `$B` is a regex group inside an ast node; an ast fix can use only metavariables"
        );
    }

    #[test]
    fn select_when_reports_the_select_side_captures() {
        let compiled = compile(
            json!({ "select_when": {
                "select": ast(json!({ "pattern": "foo($A)" })),
                "exists": { "regex": { "source": "file.content", "pattern": "(?<MARK>@legacy)" } },
            } }),
            violation("capture.A", "foo of $A"),
        )
        .unwrap();
        let found = only(&compiled, "// @legacy\nfoo(bar);\n");
        assert_eq!(found["message"], "foo of bar");
        assert_eq!(point(&found["span"]), (1, 4, 1, 7));
    }

    #[test]
    fn anchors_messages_and_fixes_are_checked_against_the_query() {
        let pattern = || ast(json!({ "pattern": "foo($A)", "not": { "pattern": "foo($B)" } }));
        for (forbid, violation, expected) in [
            (
                pattern(),
                violation("subject.span", "m"),
                "/clauses/0/violation/anchor: unknown anchor `subject.span`; expected witness.span, witness.start, witness.end, file.start, or capture.<NAME>",
            ),
            (
                pattern(),
                violation("capture.B", "m"),
                "/clauses/0/violation/anchor: `capture.B` names no capture; the query can bind: A",
            ),
            (
                pattern(),
                violation("witness.span", "bad $NAME"),
                "/clauses/0/violation/message: `$NAME` names no capture; the query can bind: A; write `\\$NAME` for literal text",
            ),
            (
                json!({ "regex": { "source": "file.content", "pattern": "x" } }),
                violation("capture.X", "m"),
                "/clauses/0/violation/anchor: `capture.X` names no capture; the query binds no captures",
            ),
            (
                json!({ "regex": { "source": "file.content", "pattern": "(?<X>x)" } }),
                violation("witness.span", "$$$X"),
                "/clauses/0/violation/message: `$$$X`: regex groups are single captures; use `$X`",
            ),
            (
                json!({ "regex": { "source": "file.path", "pattern": "(?<X>x)" } }),
                with_fix("witness.span", "y"),
                "/clauses/0/violation/fix: a file.path regex match is not source text a fix can replace",
            ),
            (
                json!({ "regex": { "source": "file.path", "pattern": "(?<X>x)" } }),
                violation("capture.X", "m"),
                "/clauses/0/violation/anchor: `capture.X`: a file.path regex match is the whole file, with no source location to anchor within",
            ),
            (
                json!({ "file": {} }),
                violation("capture.X", "m"),
                "/clauses/0/violation/anchor: `capture.X`: a file query's witness is the whole file, with no source location to anchor within",
            ),
            (
                json!({ "file": {} }),
                with_fix("witness.span", "y"),
                "/clauses/0/violation/fix: a file query's witness is the whole file, not source text a fix can replace",
            ),
            (
                json!({ "file": {} }),
                json!({ "anchor": "witness.span", "code": "X", "message": "m", "each": "X" }),
                "/clauses/0/violation/each: each enumerates the captures of an ast query",
            ),
            (
                json!({ "file": { "pattern": "x" } }),
                violation("witness.span", "m"),
                "unknown field `pattern` at /clauses/0/forbid/file",
            ),
            (
                json!({ "file": {}, "regex": { "source": "file.path", "pattern": "x" } }),
                violation("witness.span", "m"),
                "/clauses/0/forbid must contain exactly one query source",
            ),
            (
                pattern(),
                json!({ "anchor": "witness.span", "code": "X", "message": "m", "fix": ["x"] }),
                "/clauses/0/violation/fix must be an ast-grep fix template string or { template, range }",
            ),
            (
                pattern(),
                with_fix("witness.span", "bar($B)"),
                "/clauses/0/violation/fix: `$B` names no capture; the query can bind: A",
            ),
        ] {
            assert_eq!(rejection(compile(forbid, violation)), expected);
        }
        for removed in [
            json!({ "regex": { "source": "file.content", "pattern": "(x)", "captures": ["x"] } }),
            json!({ "ast": { "language": "typescript", "rule": { "pattern": "f($A)" }, "capture": "A" } }),
        ] {
            let reason = rejection(compile(removed, violation("witness.span", "m")));
            assert!(reason.starts_with("unknown field `capture"), "{reason}");
        }
    }

    fn ranged(fix: Value) -> Value {
        json!({ "anchor": "witness.span", "code": "X", "message": "m", "fix": fix })
    }

    #[test]
    fn a_fix_range_replaces_a_capture_inside_the_witness() {
        // no-div-regex: ESLint replaces the `=` after the opening slash with `[=]`.
        let compiled = compile(
            ast(json!({ "kind": "regex", "regex": "^/(?<EQ>=)" })),
            ranged(json!({ "template": "[=]", "range": "capture.EQ" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "const r = /=foo/;\n";
        let fix = &only(&compiled, source)["fix"];
        assert_eq!(fix["text"], "[=]");
        assert_eq!(
            (
                fix["span"]["startByte"].as_u64(),
                fix["span"]["endByte"].as_u64()
            ),
            (Some(11), Some(12))
        );
        // A metavariable and a transform of it; `witness.span` is the whole witness.
        let compiled = compile(
            json!({ "ast": {
                "language": "typescript",
                "rule": { "pattern": "f($A, $B)" },
                "transform": { "U": { "convert": { "source": "$A", "toCase": "upperCase" } } },
            } }),
            ranged(json!({ "template": "$U", "range": "capture.A" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let fix = &only(&compiled, "f(abc, d);\n")["fix"];
        assert_eq!(fix["text"], "ABC");
        assert_eq!(fix["span"]["startByte"], 2);
        let compiled = compile(
            ast(json!({ "pattern": "f($$$A)" })),
            ranged(json!({ "template": "[$$$A]", "range": "capture.A" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let fix = &only(&compiled, "f(a, b);\n")["fix"];
        assert_eq!(
            (
                fix["text"].as_str(),
                fix["span"]["startByte"].as_u64(),
                fix["span"]["endByte"].as_u64()
            ),
            (Some("[a, b]"), Some(2), Some(6))
        );
        // A capture outside the witness gets no fix; the violation stays.
        let compiled = compile(
            ast(json!({ "kind": "identifier", "regex": "^a$", "inside": { "stopBy": "end", "pattern": "$P", "kind": "call_expression" } })),
            ranged(json!({ "template": "x", "range": "capture.P" })),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(only(&compiled, "f(a);\n").get("fix"), None);
        for (fix, expected) in [
            (
                json!({ "template": "x", "range": "capture.Z" }),
                "/clauses/0/violation/fix/range: `capture.Z` names no capture; the query can bind: A",
            ),
            (
                json!({ "template": "x", "range": "witness.start" }),
                "/clauses/0/violation/fix/range: unknown range `witness.start`; expected witness.span or capture.<NAME>",
            ),
            (
                json!({ "template": "x" }),
                "/clauses/0/violation/fix/range is required: witness.span or capture.<NAME>",
            ),
            (
                json!({ "template": 1, "range": "capture.A" }),
                "/clauses/0/violation/fix/template must be an ast-grep fix template string",
            ),
            (
                json!({ "template": "x", "range": "capture.A", "at": 1 }),
                "unknown field `at` at /clauses/0/violation/fix",
            ),
        ] {
            assert_eq!(
                rejection(compile(ast(json!({ "pattern": "f($A)" })), ranged(fix))),
                expected
            );
        }
        let regex = json!({ "regex": { "source": "file.content", "pattern": "(?<X>a)" } });
        assert_eq!(
            rejection(compile(
                regex,
                ranged(json!({ "template": "b", "range": "capture.X" }))
            )),
            "/clauses/0/violation/fix/range: a fix range names a capture of an ast query"
        );
    }

    /// Capture names and witness regexes summarize each utility once and map the summary at each
    /// call, so a diamond chain costs each utility a constant number of expansions.
    #[test]
    fn parameterized_utility_diamonds_summarize_violations_in_linear_visits() {
        use crate::code_rule_dsl::test_support::{
            DIAMOND_DEPTH, Walk, compiled_parameterized_utility_diamonds, grammar_matchers,
            linear_walk, reset_walks, walks,
        };
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let grammar = &grammar_matchers(&compiled)[0];
        let bound = linear_walk(DIAMOND_DEPTH);
        reset_walks();
        let mut names = BTreeSet::new();
        ast_captures(
            &grammar.matcher,
            &grammar.utils,
            &mut UtilitySummary::new(),
            &mut names,
        );
        assert!(names.contains("X"), "{names:?}");
        assert!(
            walks(Walk::AstCaptures) <= bound,
            "capture names expanded {} utilities (bound {bound})",
            walks(Walk::AstCaptures)
        );
        witness_groups(&grammar.matcher, &grammar.utils);
        assert!(
            walks(Walk::NodeRegexes) <= bound,
            "witness regexes expanded {} utilities (bound {bound})",
            walks(Walk::NodeRegexes)
        );
    }

    /// A utility reached along many paths tests the witness once: its regex describes it once.
    #[test]
    fn utility_diamonds_contribute_each_witness_regex_once() {
        use crate::code_rule_dsl::test_support::{compiled, grammar_matchers, utility_diamonds};
        let mut utils = utility_diamonds(12);
        utils["u0"] = json!({ "kind": "identifier", "regex": "^(?<N>x)$" });
        let compiled = compiled(json!({ "rule": { "matches": "u12" }, "utils": utils }));
        let grammar = &grammar_matchers(&compiled)[0];
        assert_eq!(witness_regexes(&grammar.matcher, &grammar.utils).len(), 1);
        assert_eq!(witness_groups(&grammar.matcher, &grammar.utils), ["N"]);
    }

    /// Compiling a query collects capture names from its rule and every shared utility with one
    /// summary per utility, so doubling a utility chain at most doubles that work.
    #[test]
    fn capture_names_of_a_compiled_utility_chain_grow_linearly() {
        use crate::code_rule_dsl::test_support::{
            Walk, compiled_utility_diamonds, reset_walks, walks,
        };
        let expansions = |depth| {
            reset_walks();
            compiled_utility_diamonds(depth);
            walks(Walk::AstCaptures)
        };
        let (half, full) = (expansions(6), expansions(12));
        assert!(
            full <= 2 * half,
            "capture names expanded {half} utilities at depth 6, {full} at depth 12"
        );
    }

    /// `W` binds its parameter through `V`, which binds its own through `W` or an outward relation:
    /// the violation may anchor on what `W` binds whichever `any` branch the walk meets first.
    #[test]
    fn captures_bound_through_recursive_calls_do_not_depend_on_branch_order() {
        use crate::code_rule_dsl::test_support::{
            mutually_recursive_calls, recursive_call_branch_orders,
        };
        for branches in recursive_call_branch_orders() {
            let forbid = json!({ "ast": { "language": "typescript",
                "rule": { "kind": "expression_statement", "any": branches },
                "utils": mutually_recursive_calls() } });
            if let Err(error) = compile(forbid, violation("capture.X", "x")) {
                panic!("{branches}: {}", error.reason);
            }
        }
    }
