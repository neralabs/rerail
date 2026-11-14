    use super::{WalkFilter, bounded, remembered_peak, reset_remembered_peak, with_file};
    use crate::code_rule_dsl::budget::MatchBudget;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::matcher::{FileScopeSlot, RequiredText};
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::code_rule_dsl::test_support::*;
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn tape_file(source: &str, language: GrammarDialect) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(language).unwrap();
        let native = StrDoc::new(source, language);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        )
    }

    #[test]
    fn postings_the_deadline_cut_short_test_nothing_and_are_not_remembered() {
        let file = tape_file("f(g(h(1)), [k]);\n", GrammarDialect::TypeScript);
        let calls = file
            .root()
            .dfs()
            .find(|node| node.kind() == "call_expression")
            .expect("a call")
            .kind_id();
        let rule = |rule: Value| {
            let compiled =
                compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
            Arc::new(WalkFilter {
                kinds: [calls].into(),
                needle: RequiredText::of_node(&grammar_matchers(&compiled)[0].matcher),
            })
        };
        let (any, holding_g) = (
            rule(json!({ "kind": "call_expression" })),
            rule(json!({ "kind": "call_expression", "regex": "g" })),
        );
        let program = file.root();
        let tested = |filter: &Arc<WalkFilter>| {
            bounded(filter, &program, |tape, ordinal| {
                tape.transducer_descendants(ordinal)
            })
            .expect("a two-valued walk over the file")
            .len()
        };
        with_file(&file, || {
            // The kind index, then postings searched for a needle, each cut short.
            for (filter, listed) in [(&any, 3), (&holding_g, 2)] {
                let budget = MatchBudget::expired_for_test();
                assert_eq!(tested(filter), 0);
                assert!(budget.observed(), "the overrun was not recorded");
                drop(budget);
                assert_eq!(tested(filter), listed);
            }
        });
    }

    #[test]
    fn verdicts_of_past_enumeration_rounds_are_dropped() {
        const IDENTIFIERS: usize = 60;
        let source = format!(
            "f({});\n",
            (0..IDENTIFIERS)
                .map(|index| format!("a{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut violation = violation();
        violation["each"] = json!("X");
        let compiled = compile_document_with(
            clause_document(
                json!({ "id": "c", "violation": violation, "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "call_expression",
                          "has": { "kind": "arguments", "has": { "matches": "name" } } },
                "utils": { "name": { "kind": "identifier", "pattern": "$X" } },
            } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        reset_remembered_peak();
        let result = evaluate_file(&compiled, "typescript", "a.ts", &source);
        assert_violations(&result, IDENTIFIERS);
        // Round `n` remembers the `n` names it passes over again: every round's kept, the
        // verdicts would grow with the square of the names.
        assert!(
            remembered_peak() <= 3 * IDENTIFIERS,
            "{} verdicts remembered at once",
            remembered_peak()
        );
    }

    #[test]
    fn a_remembered_verdict_over_a_parsed_tree_replays_its_sibling_probes() {
        let file = tape_file("x: \"List[int, str]\"\n", GrammarDialect::Python);
        let compiled = compile_document_with(
            clause_document(
                json!({ "id": "c", "violation": violation(), "forbid": { "ast": {
                "language": "python",
                "rule": { "matches": "annotated" },
                "utils": {
                    "annotated": { "kind": "string", "parse": { "grammar": "annotation",
                        "has": { "stopBy": "end", "matches": "second" } } },
                    "second": { "kind": "identifier",
                                "nthChild": { "position": 2, "ofRule": { "kind": "identifier" } } },
                },
            } } }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let grammar = &grammar_matchers(&compiled)[0];
        let string = file
            .root()
            .dfs()
            .find(|node| node.kind() == "string")
            .expect("string");
        let spent = || {
            let mut work = 1_000;
            let matched = grammar.match_root(string.clone(), &mut work).expect("fits");
            assert!(matched.is_some());
            1_000 - work
        };
        let _slot = FileScopeSlot::install(Vec::new(), None, false);
        // The file's scope table, a cache of its own, is filled first.
        with_file(&file, spent);
        with_file(&file, || {
            reset_remembered_peak();
            let computed = spent();
            assert_eq!(remembered_peak(), 1, "the verdict was not remembered");
            // The annotation's siblings are not the file's: probing them fills no cache.
            assert_eq!(spent(), computed, "the remembered verdict spent less");
        });
    }

    #[test]
    fn a_verdict_reading_its_clauses_own_caches_spends_for_another_clause_what_computing_it_would()
    {
        let file = tape_file("f(a, b, c);\n", GrammarDialect::TypeScript);
        // Two clauses with an equal context-free utility, whose verdicts are shared by digest; its
        // counted siblings are cached for the `nthChild` of one clause alone.
        let compile = || {
            compile_document_with(
                clause_document(
                    json!({ "id": "c", "violation": violation(), "forbid": { "ast": {
                    "language": "typescript",
                    "rule": { "matches": "second" },
                    "utils": { "second": { "kind": "identifier",
                        "nthChild": { "position": 2, "ofRule": { "kind": "identifier" } } } },
                } } }),
                ),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason))
        };
        let (first, second) = (compile(), compile());
        let (first, second) = (&grammar_matchers(&first)[0], &grammar_matchers(&second)[0]);
        let b = file
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier" && node.text() == "b")
            .expect("b");
        let spent = |clause: &crate::code_rule_dsl::matcher::GrammarMatcher| {
            let mut work = 1_000;
            let matched = clause.match_root(b.clone(), &mut work).expect("fits");
            assert!(matched.is_some());
            1_000 - work
        };
        let alone = with_file(&file, || spent(second));
        let after_the_other = with_file(&file, || {
            spent(first);
            spent(second)
        });
        assert_eq!(after_the_other, alone, "another check's verdict spent less");
    }
