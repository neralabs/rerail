    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::model::{ClauseKind, Query};
    use crate::code_rule_dsl::test_support::*;
    use ast_grep_language::LanguageExt;
    use serde_json::{Value, json};

    fn compiled_sound(ast: Value) -> bool {
        grammar_matchers(&compiled(ast))
            .iter()
            .all(|grammar| grammar.sound_under_errors)
    }

    fn syntax_errors(language: GrammarDialect, source: &str) -> SyntaxErrors {
        let root = language.ast_grep(source);
        SyntaxErrors::collect(root.root().get_doc().tree.root_node())
    }

    #[test]
    fn syntax_errors_cover_error_and_missing_nodes_only() {
        assert!(syntax_errors(GrammarDialect::TypeScript, "debugger;\n").is_empty());
        let source = "debugger;\nfunction f( {\n  debugger;\n";
        let errors = syntax_errors(GrammarDialect::TypeScript, source);
        assert!(!errors.is_empty());
        assert!(!errors.overlaps(&(0..9)), "the leading statement is clean");
        assert!(
            errors.overlaps(&(0..source.len())),
            "the program contains the error"
        );
        let lines = LineTable::new(source, crate::code_rule_dsl::lines::Terminators::Eslint);
        assert!(errors.partial_message(&lines).contains("first at line 2"));
    }

    fn reading(errors: &SyntaxErrors, source: &str) -> Result<String, String> {
        let lines = LineTable::new(source, crate::code_rule_dsl::lines::Terminators::Eslint);
        match errors.reading(&lines, source.len()) {
            SyntaxReading::Partial(message) => Ok(message),
            SyntaxReading::Unread(message) => Err(message),
        }
    }

    #[test]
    fn a_file_whose_regions_cover_half_of_it_or_more_is_unread() {
        let source = "0123456789\n0123456789\n";
        let region =
            |range: Range<usize>| SyntaxErrors::from_ranges(std::iter::once(range).collect());
        // 10 of 22 bytes, then 11 of 22: the share is of bytes, the boundary is unread.
        assert!(reading(&region(12..22), source).is_ok());
        assert_eq!(
            reading(&region(11..22), source),
            Err(
                "file did not parse: error regions cover 50% of it, first at line 2, column 1"
                    .into()
            )
        );
        // Overlapping regions count once; zero-width MISSING points count nothing.
        let errors = SyntaxErrors::from_ranges(vec![0..6, 3..10, 15..15, 20..20]);
        assert!(reading(&errors, source).is_ok());
        assert_eq!(
            reading(&region(0..22), source),
            Err(
                "file did not parse: error regions cover 100% of it, first at line 1, column 1"
                    .into()
            )
        );
        // An empty file's MISSING point covers nothing of it.
        assert!(reading(&region(0..0), "").is_ok());
    }

    #[test]
    fn a_grammar_that_gave_up_leaves_the_file_unread() {
        // tree-sitter-javascript recovers the leading statement, then reads the prose as one
        // ERROR to the end: 95% of the bytes.
        let source = format!(
            "debugger;\n{}",
            "Install it: run the script, then open the page.\n".repeat(5)
        );
        let errors = syntax_errors(GrammarDialect::JavaScript, &source);
        assert_eq!(
            reading(&errors, &source),
            Err(
                "file did not parse: error regions cover 95% of it, first at line 2, column 1"
                    .into()
            )
        );
        let partial = "debugger; // clean text keeps the error region under half the file\nfunction f( {\n  debugger;\n";
        assert!(reading(&syntax_errors(GrammarDialect::TypeScript, partial), partial).is_ok());
    }

    #[test]
    fn missing_tokens_suppress_only_the_witnesses_that_contain_them() {
        // An unclosed body parses with a zero-width MISSING `}` after its last statement and no
        // ERROR node. A MISSING child sits at its parent's end, so a witness that merely abuts
        // the point is suppressed too (`x = 1;` here); the earlier `debugger;` is clean.
        let source = "function f() {\n  debugger;\n  x = 1;\n";
        let errors = syntax_errors(GrammarDialect::TypeScript, source);
        let point = source.len() - 1;
        assert_eq!(errors.regions.len(), 1);
        assert_eq!(errors.regions[0], point..point);
        let debugger = compile_typescript_rule(json!({ "kind": "debugger_statement" }))
            .expect("rule compiles");
        let result = evaluate_file(&debugger, "typescript", "open.ts", source);
        assert_eq!(result["violations"][0]["span"]["startLine"], 1, "{result}");
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
        let function = compile_typescript_rule(json!({ "kind": "function_declaration" }))
            .expect("rule compiles");
        let result = evaluate_file(&function, "typescript", "open.ts", source);
        assert_eq!(
            result["violations"],
            json!([]),
            "the declaration spans the MISSING token"
        );
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
    }

    /// Each document of a file has its own syntax errors (`embed`): a program apart from its
    /// host's tree (tests' HTML scripts) has none of the host's, and hides no binding of its own;
    /// a program with errors of its own has them as ranges of the file; a program read through the
    /// host's errors (Blade's compiled view) has the template's too.
    #[test]
    fn each_document_has_its_own_errors_as_ranges_of_the_file() {
        use crate::code_rule_dsl::embed::test_script::html;
        use std::sync::Arc;
        let of = |source: &str| {
            let doc = html(source);
            let host = doc.root().get_doc().clone();
            let guest = host.guests().expect("owned")[0].doc.clone();
            (SyntaxErrors::of_doc(&host), SyntaxErrors::of_doc(&guest))
        };

        let (host, clean) = of("<script>let x = 1;</script>\n<div <p\n");
        assert!(!host.is_empty(), "the host has a syntax error");
        assert!(clean.is_empty());
        assert!(!clean.hide_bindings());

        let source = "<script>let = ;</script>\n<div <p\n";
        let (_, broken) = of(source);
        assert!(broken.hide_bindings());
        let script = source.find("let").unwrap()..source.find("</script>").unwrap();
        assert!(broken.overlaps(&script));
        assert!(!broken.overlaps(&(source.len() - 3..source.len())));

        let grammar = crate::code_rule_dsl::tape::Grammar::new(GrammarDialect::Blade).unwrap();
        let source = "{{ $a + }}\n{{-- open\n";
        let image =
            crate::code_rule_dsl::blade::tape::encode_template(&grammar, source, &|| false)
                .unwrap();
        let blade =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let template = SyntaxErrors::of_doc(&blade);
        let view = SyntaxErrors::of_doc(&blade.guests().unwrap()[0].doc);
        let comment = source.find("{{--").unwrap();
        assert!(template.overlaps(&(comment..source.len())));
        assert!(!template.overlaps(&(0..10)), "the echo's PHP is the view's");
        assert!(view.overlaps(&(0..10)), "the view's own error");
        assert!(view.overlaps(&(comment..source.len())), "the template's");
        assert!(view.hide_bindings());
    }

    #[test]
    fn syntax_error_regions_merge_and_treat_missing_as_a_point() {
        let errors = SyntaxErrors {
            regions: vec![4..4, 10..20],
        };
        assert!(
            errors.overlaps(&(0..4)),
            "a range ending at a MISSING point touches it"
        );
        assert!(
            errors.overlaps(&(4..8)),
            "a range starting at a MISSING point touches it"
        );
        assert!(
            !errors.overlaps(&(5..10)),
            "ranges are half-open for real regions"
        );
        assert!(
            errors.overlaps(&(12..12)),
            "an empty witness inside a region overlaps"
        );
        assert!(!errors.overlaps(&(20..20)));
        assert!(!errors.overlaps(&(20..30)));
        assert!(errors.overlaps(&(0..100)));
    }

    #[test]
    fn negated_relations_leaving_the_witness_are_unsound_under_errors() {
        let sound = |rule: Value| {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compile_typescript_rule(rule)
                .expect("rule compiles")
                .inner
                .clauses[0]
                .kind
            else {
                unreachable!()
            };
            matchers.iter().all(|grammar| grammar.sound_under_errors)
        };
        assert!(sound(json!({ "kind": "debugger_statement" })));
        assert!(sound(
            json!({ "kind": "program", "not": { "has": { "kind": "class_declaration", "stopBy": "end" } } })
        ));
        assert!(sound(
            json!({ "kind": "debugger_statement", "inside": { "kind": "function_declaration", "stopBy": "end" } })
        ));
        assert!(sound(
            json!({ "kind": "debugger_statement", "follows": { "kind": "import_statement" } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "inside": { "kind": "function_declaration" } } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "any": [{ "precedes": { "kind": "return_statement" } }] } })
        ));
        // `stopBy: end` walks the same parent's siblings, so a positive claim stays sound.
        assert!(sound(
            json!({ "kind": "debugger_statement", "follows": { "kind": "import_statement", "stopBy": "end" } })
        ));
        assert!(sound(
            json!({ "kind": "debugger_statement", "precedes": { "kind": "return_statement", "stopBy": "end" } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "not": { "precedes": { "kind": "return_statement" } } })
        ));
        assert!(!sound(
            json!({ "kind": "debugger_statement", "inside": { "kind": "function_declaration", "not": { "has": { "kind": "return_statement" } } } })
        ));
    }

    #[test]
    fn never_matching_sub_rules_stay_sound_under_errors() {
        // In the plain dialect the JSX sub-rule compiles to `Never`; it has no relations, so the
        // rule stays sound and witnesses outside the error region are still reported.
        let compiled = compile_typescript_rule(json!({
            "kind": "debugger_statement",
            "not": { "has": { "kind": "jsx_expression", "stopBy": "end" } },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.clauses[0].kind
        else {
            unreachable!()
        };
        assert!(matchers.iter().all(|grammar| grammar.sound_under_errors));
        let result = evaluate_file(
            &compiled,
            "typescript",
            "open.ts",
            "debugger; // clean text keeps the error region under half the file\nfunction f( {\n",
        );
        assert_eq!(result["violations"][0]["span"]["startLine"], 0, "{result}");
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");

        // A rule that is `Never` as a whole in the plain dialect has no matcher there at all.
        let compiled = compile_typescript_rule(json!({ "kind": "jsx_attribute" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(
            &compiled,
            "typescript",
            "open.ts",
            "debugger; // clean text keeps the error region under half the file\nfunction f( {\n",
        );
        assert_eq!(result["violations"], json!([]), "{result}");
    }

    #[test]
    fn stateless_evaluation_reports_witnesses_outside_error_regions() {
        let compiled = compile_typescript_rule(json!({ "kind": "debugger_statement" }))
            .expect("rule compiles");
        let result = evaluate_file(
            &compiled,
            "typescript",
            "api.test.ts",
            // A missing comma, which TypeScript rejects too: the ERROR stays whatever the grammar fixes.
            "const m = importOriginal(1 load(\"./api\"));\ndebugger;\n",
        );
        assert_eq!(result["status"], "failed", "{result}");
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
        assert_eq!(result["violations"][0]["span"]["startLine"], 1, "{result}");
        assert_eq!(
            result["failures"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
        assert_eq!(result["failures"][0]["path"], "api.test.ts", "{result}");

        let unsound = compile_typescript_rule(json!({
            "kind": "debugger_statement",
            "not": { "follows": { "kind": "import_statement", "stopBy": "end" } },
        }))
        .expect("rule compiles");
        let result = evaluate_file(
            &unsound,
            "typescript",
            "api.test.ts",
            "const m = importOriginal(1 load(\"./api\"));\ndebugger;\n",
        );
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
    }

    #[test]
    fn stop_rules_are_absence_claims_under_errors() {
        let sound = |rule: Value| {
            grammar_matchers(&compile_typescript_rule(rule).expect("rule compiles"))
                .iter()
                .all(|grammar| grammar.sound_under_errors)
        };
        // Inside the witness a stop rule reads cleanly parsed nodes only.
        assert!(sound(json!({
            "kind": "function_declaration",
            "has": { "kind": "return_statement", "stopBy": { "kind": "function_declaration" } },
        })));
        // Walking ancestors or siblings past nodes that did not stop claims they are absent.
        for relation in ["inside", "precedes", "follows"] {
            assert!(!sound(json!({
                "kind": "identifier",
                relation: { "kind": "class_declaration", "stopBy": { "kind": "function_declaration" } },
            })));
        }
        // A stop rule under `has` that itself leaves the node it tests is unsound too.
        assert!(!sound(json!({
            "kind": "function_declaration",
            "has": { "kind": "return_statement", "stopBy": { "inside": { "kind": "class_body" } } },
        })));
    }

    #[test]
    fn utils_and_constraints_are_as_sound_under_errors_as_what_they_reach() {
        let sound = |ast: Value| {
            grammar_matchers(&compiled(ast))
                .iter()
                .all(|grammar| grammar.sound_under_errors)
        };
        // Recursion that stays inside the witness, and positive relations, stay sound.
        assert!(sound(json!({
            "rule": { "kind": "return_statement", "has": { "matches": "tail" } },
            "utils": { "tail": { "any": [{ "kind": "await_expression" }, { "kind": "parenthesized_expression", "has": { "matches": "tail" } }] } },
        })));
        assert!(sound(json!({
            "rule": { "kind": "identifier", "matches": "in-loop" },
            "utils": { "in-loop": { "inside": { "stopBy": "end", "kind": "for_statement" } } },
        })));
        // A negated relation leaving the witness is unsound wherever it hides, including behind a
        // reference, a recursive one, or a reference reached under `not`.
        assert!(!sound(json!({
            "rule": { "kind": "identifier", "matches": "top-level" },
            "utils": { "top-level": { "not": { "inside": { "stopBy": "end", "kind": "function_declaration" } } } },
        })));
        assert!(!sound(json!({
            "rule": { "kind": "identifier", "not": { "matches": "in-function" } },
            "utils": { "in-function": { "inside": { "stopBy": "end", "kind": "function_declaration" } } },
        })));
        assert!(!sound(json!({
            "rule": { "kind": "identifier", "matches": "walk" },
            "utils": { "walk": { "any": [
                { "kind": "program" },
                { "inside": { "matches": "walk" }, "not": { "follows": { "kind": "comment" } } },
            ] } },
        })));
        // A constraint's negation reads the captured node: fine inside the witness, not outside it.
        assert!(sound(json!({
            "rule": { "pattern": "$F($$$)" },
            "constraints": { "F": { "not": { "has": { "kind": "this" } } } },
        })));
        assert!(!sound(json!({
            "rule": { "kind": "return_statement", "follows": { "pattern": "$PREV" } },
            "constraints": { "PREV": { "not": { "has": { "stopBy": "end", "kind": "call_expression" } } } },
        })));
    }

    #[test]
    fn reading_a_capture_an_outward_relation_may_leave_unbound_is_unsound_under_errors() {
        // Review finding 3 on #2561. `any` tolerates the outward relation failing, and it fails
        // when an ERROR sibling swallowed the earlier node: `$PREV` is then unbound, so the negated
        // comparison holds vacuously and so does a constraint on `$PREV` (unbound captures are
        // skipped). The equivalent `not: { follows }` is already unsound.
        let optional_prev = json!([
            { "follows": { "kind": "expression_statement", "pattern": "$PREV" } },
            { "regex": "." },
        ]);
        let differs = json!({
            "kind": "expression_statement",
            "pattern": "$CUR",
            "any": optional_prev,
            "not": { "compare": { "left": "PREV", "op": "eq", "right": "CUR" } },
        });
        let compiled = compile_typescript_rule(differs.clone()).expect("rule compiles");
        assert!(
            !grammar_matchers(&compiled)
                .iter()
                .any(|grammar| grammar.sound_under_errors),
            "a negated comparison of a possibly unbound outward capture"
        );
        let source = "f();\nf(;\nf();\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
        let constrained = compiled_sound(json!({
            "rule": { "kind": "expression_statement", "any": optional_prev },
            "constraints": { "PREV": { "regex": "^g" } },
        }));
        assert!(
            !constrained,
            "a constraint skipped when the outward capture is unbound"
        );
        // Without `any` the relation must hold for the witness to exist, so the capture is bound.
        assert!(compiled_sound(json!({
            "rule": { "kind": "expression_statement", "pattern": "$CUR", "all": [
                { "follows": { "kind": "expression_statement", "pattern": "$PREV" } },
                { "not": { "compare": { "left": "PREV", "op": "eq", "right": "CUR" } } },
            ] },
        })));
        assert!(compiled_sound(json!({
            "rule": { "kind": "expression_statement", "follows": { "kind": "expression_statement", "pattern": "$PREV" } },
            "constraints": { "PREV": { "regex": "^g" } },
        })));
        // A capture made inside the witness is bound or not by clean source alone.
        assert!(compiled_sound(json!({
            "rule": { "kind": "call_expression", "pattern": "$F($$$)", "any": [{ "has": { "kind": "arguments", "pattern": "$ARGS" } }, { "regex": "." }],
                "not": { "compare": { "left": "F", "op": "eq", "right": "ARGS" } } },
            // `kind`: the regex tests the argument list's text (a named-only tape keeps no `(` to test).
            "constraints": { "ARGS": { "kind": "arguments", "regex": "^\\(" } },
        })));
        // Positive comparisons stay sound: an unbound side makes them fail, a false negative.
        assert!(compiled_sound(json!({
            "rule": { "kind": "expression_statement", "pattern": "$CUR", "any": optional_prev,
                "compare": { "left": "PREV", "op": "eq", "right": "CUR" } },
        })));
    }

    #[test]
    fn parameterized_calls_track_optional_outward_captures_per_mapping() {
        let optional_previous = json!({
            "params": ["P"],
            "rule": { "any": [
                { "follows": { "kind": "expression_statement", "pattern": "$P" } },
                { "regex": "." }
            ] }
        });
        let rule = json!({
            "rule": {
                "pattern": "$C",
                "all": [
                    { "matches": { "previous": { "P": "$A" } } },
                    { "matches": { "previous": { "P": "$B" } } },
                    { "not": { "compare": { "left": "B", "op": "eq", "right": "C" } } }
                ]
            },
            "utils": { "previous": optional_previous }
        });
        let compiled = compile_typescript_ast(rule).expect("rule compiles");
        assert!(
            !grammar_matchers(&compiled)
                .iter()
                .all(|grammar| grammar.sound_under_errors),
            "each caller capture can be unbound when the outward relation is swallowed by syntax errors"
        );
        let result = evaluate_file(&compiled, "typescript", "a.ts", "a();\na(;\na();\n");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"][0]["code"], "source-incomplete",
            "{result}"
        );
    }

    #[test]
    fn utility_capture_analysis_memoizes_completed_diamond_paths() {
        let mut utils = serde_json::Map::new();
        utils.insert("u0".to_owned(), json!({ "kind": "identifier" }));
        for depth in 1..=8 {
            let prior = format!("u{}", depth - 1);
            utils.insert(
                format!("u{depth}"),
                json!({ "all": [{ "matches": prior }, { "matches": prior }] }),
            );
        }
        reset_walks();
        let _ = compiled(json!({
            "rule": { "matches": "u8" },
            "utils": utils,
        }));
        assert!(
            walks(Walk::OptionalCaptures) <= 36,
            "a completed utility/context must be analyzed once, not once per diamond path"
        );
    }

    /// A parameterized call's summary is its utility's, in the utility's own names, whatever the
    /// mapping: each utility is analysed a constant number of times, not once per call path.
    #[test]
    fn parameterized_utility_diamonds_are_analysed_in_linear_visits() {
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        reset_walks();
        sound_under_errors(matcher);
        let bound = linear_walk(DIAMOND_DEPTH);
        assert!(
            walks(Walk::OptionalCaptures) <= bound,
            "optional capture analysis expanded {} utilities (bound {bound})",
            walks(Walk::OptionalCaptures)
        );
        assert!(
            walks(Walk::SoundVerdict) <= bound,
            "the soundness verdict expanded {} utilities (bound {bound})",
            walks(Walk::SoundVerdict)
        );
    }

    #[test]
    fn soundness_verdict_memoizes_completed_diamond_paths() {
        let mut utils = serde_json::Map::new();
        utils.insert("u0".to_owned(), json!({ "kind": "identifier" }));
        for depth in 1..=16 {
            let prior = format!("u{}", depth - 1);
            utils.insert(
                format!("u{depth}"),
                json!({ "all": [{ "matches": prior }, { "matches": prior }] }),
            );
        }
        reset_walks();
        let _ = compiled(json!({
            "rule": { "matches": "u16" },
            "utils": utils,
        }));
        assert!(
            walks(Walk::SoundVerdict) <= 64,
            "a completed utility/context must be checked once, not once per diamond path"
        );
    }

    #[test]
    fn counts_are_as_sound_under_errors_as_the_negated_relation() {
        let sound = |rule: Value| {
            grammar_matchers(&compile_typescript_rule(rule).expect("rule compiles"))
                .iter()
                .all(|grammar| grammar.sound_under_errors)
        };
        assert!(sound(json!({
            "kind": "function_declaration",
            "count": { "has": { "kind": "if_statement", "stopBy": { "kind": "arrow_function" } }, "min": 2 },
        })));
        for relation in ["inside", "precedes", "follows"] {
            assert!(!sound(json!({
                "kind": "if_statement",
                "count": { relation: { "kind": "if_statement", "stopBy": "end" }, "min": 2 },
            })));
        }
    }

    #[test]
    fn utility_diamonds_compile_in_linear_utility_visits() {
        let compilers: [fn(usize) -> crate::code_rule_dsl::CompiledCodeRuleDsl; 2] = [
            compiled_utility_diamonds,
            compiled_parameterized_utility_diamonds,
        ];
        for compile in compilers {
            reset_walks();
            compile(30);
            assert!(
                walks(Walk::OptionalCaptures) <= 128,
                "optional capture analysis visited {} utility contexts",
                walks(Walk::OptionalCaptures)
            );
            assert!(
                walks(Walk::SoundVerdict) <= 128,
                "soundness visited {} utility contexts",
                walks(Walk::SoundVerdict)
            );
        }
    }

    /// A capture bound through a cycle of parameterized calls under an outward relation under
    /// `any` is optional whichever call a walk summarizes first: a summary computed while the other
    /// was still being summarized holds only part of it, and must not stand for the whole.
    #[test]
    fn optional_captures_through_recursive_calls_do_not_depend_on_branch_order() {
        for branches in recursive_call_branch_orders() {
            let mut utils = mutually_recursive_calls();
            utils.insert("bindX".to_owned(), json!({ "pattern": "$X" }));
            let sound = compiled_sound(json!({
                "rule": { "kind": "expression_statement", "any": branches },
                "constraints": { "X": { "regex": "^g" } },
                "utils": utils,
            }));
            assert!(
                !sound,
                "a constraint on `$X`, which `W` may leave unbound, is skipped under errors ({branches})"
            );
        }
    }

    /// A self-recursive call binding its parameter through an outward relation under `any`: the
    /// negated comparison of what the inner call binds reads an optional capture, also when the
    /// root's call sits under `any` itself (the inner call's context).
    #[test]
    fn a_self_recursive_calls_optional_capture_is_unsound_wherever_it_is_called() {
        let utils = json!({
            "W": { "params": ["P"], "rule": { "pattern": "$N", "any": [
                { "follows": { "kind": "expression_statement", "pattern": "$P" } },
                { "has": { "all": [
                    { "matches": { "W": { "P": "$Q" } } },
                    { "not": { "compare": { "left": "Q", "op": "eq", "right": "N" } } }
                ] } }
            ] } },
        });
        for rule in [
            json!({ "kind": "expression_statement", "matches": { "W": { "P": "$X" } } }),
            json!({ "kind": "expression_statement", "any": [
                { "matches": { "W": { "P": "$X" } } }, { "kind": "identifier" } ] }),
        ] {
            assert!(
                !compiled_sound(json!({ "rule": rule, "utils": utils.clone() })),
                "{rule}"
            );
        }
    }
