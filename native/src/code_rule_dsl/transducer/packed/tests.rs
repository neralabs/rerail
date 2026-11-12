    use super::*;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn metal_backend_rejects_execution_without_platform_support() {
        assert!(matches!(
            execute_with_backend_packed_result_limits(ExecutionBackend::Metal, &[], &[], &[], None),
            Err(Error::UnsupportedBackend)
        ));
    }
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::matcher::Pattern;
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
    use ast_grep_core::tree_sitter::LanguageExt;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::hint::black_box;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// G's grid executor's result, or None where it refuses the evaluation's launch bound (99,
    /// list step 13): its dynamic frontier's waves exceed it on these parity programs (open item
    /// O33). The CPU backends' parity is still asserted.
    #[cfg(target_os = "macos")]
    fn grid<T>(result: Result<T, Error>) -> Option<T> {
        match result {
            Err(Error::BackendLaunchLimit) => {
                eprintln!("O33: the grid executor refused the launch bound; its parity is not checked here");
                None
            }
            result => Some(result.unwrap()),
        }
    }

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    fn identifier_program(text: &str) -> Program {
        let pattern = Pattern::try_new(text, SupportLang::Python).unwrap();
        Program::compile_ast_grep_literal(&pattern).unwrap()
    }

    #[test]
    fn sibling_edges_preserve_complete_outputs_with_linear_source_traversal() {
        let source = format!(
            "f({})\n",
            (0..128)
                .map(|i| format!("v{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let file = tape(&source);
        let arguments = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("argument_list", true);
        let root = (0..file.transducer_len() as u32)
            .find(|&node| file.transducer_kind(node) == arguments)
            .unwrap();
        let children = file.transducer_children(root).count();
        let expected = |span: std::ops::Range<usize>, start| {
            let node = (0..file.transducer_len() as u32)
                .find(|&node| file.transducer_span(node) == span)
                .unwrap();
            vec![Output {
                task_ordinal: 0,
                local_ordinal: 0,
                start,
                label: 0,
                captures: vec![Capture {
                    label: 0,
                    node,
                    span,
                    text: None,
                    properties: vec![],
                }],
                properties: vec![],
                dynamic_properties: vec![],
            }]
        };
        for edge in [FIRST, LAST] {
            for mode in [0, NAMED] {
                let program = Program::new(
                    vec![
                        word(CHILD, 0, 0),
                        word(edge, mode, 0),
                        word(CAPTURE, 0, 0),
                        word(EMIT, 0, 0),
                    ],
                    vec![b"edge".to_vec()],
                    vec![],
                    vec![0],
                    vec![],
                )
                .unwrap();
                let span = match (edge, mode) {
                    (FIRST, 0) => 1..2,
                    (FIRST, NAMED) => 2..4,
                    (LAST, 0) => source.len() - 2..source.len() - 1,
                    (LAST, NAMED) => source.len() - 6..source.len() - 2,
                    _ => unreachable!(),
                };
                let mut stats = ExecutionStats::default();
                let actual = program
                    .execute_roots_from(&file, [root], 0, Some(&mut stats))
                    .unwrap();
                assert_eq!(actual, expected(span, root));
                assert!(
                    stats.sibling_edge_visits <= 2 * children + 2,
                    "edge lookup must stay linear in the immutable child domain: {stats:?}"
                );
            }
        }
        // Named and all-sibling queries share a parent but have different edges.
        let mixed = Program::new(
            vec![
                word(CHILD, 0, 0),
                word(FORK, 0, 6),
                word(FIRST, NAMED, 0),
                word(LAST, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(FIRST, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"edge".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_eq!(
            mixed.execute_roots(&file, [root]).unwrap(),
            expected(1..2, root)
        );
        for edge in [FIRST, LAST] {
            let program = Program::new(
                vec![word(edge, NAMED, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"edge".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap();
            assert_eq!(
                program.execute_roots(&file, [0]).unwrap(),
                expected(0..source.len(), 0)
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn production_corpus_executes_all_files_in_one_packed_grid() {
        const FILE_COUNT: usize = 129;
        let kind = tape("value = other\n").transducer_kind(1);
        let programs = [Program::new(
            vec![word(KIND, 0, kind), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()];
        let document = tape("value = other\n");
        let roots = (0..document.transducer_len() as u32).collect::<Vec<_>>();
        let files = vec![document; FILE_COUNT];
        let anchors = vec![vec![Arc::<[u32]>::from(roots)]; FILE_COUNT];
        let execution = execute_production_corpus(&programs, &files, &anchors, &[32]).unwrap();
        assert_eq!(execution.physical_file_count(), FILE_COUNT);
        assert_eq!(execution.file_count(), FILE_COUNT);
        let oracle_anchors = anchors
            .iter()
            .map(|file| file.iter().map(|roots| roots.to_vec()).collect())
            .collect::<Vec<_>>();
        let expected = execute_batch(&programs, &files, Some(&oracle_anchors)).unwrap();
        assert_eq!(execution.into_outcome().unwrap().outputs, expected);
    }

    fn assert_jit_parity(programs: &[Program], tapes: &[TapeDoc]) {
        let anchors = tapes
            .iter()
            .map(|tape| {
                programs
                    .iter()
                    .map(|_| (0..tape.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(programs, tapes, Some(&anchors)).unwrap();
        for backend in [ExecutionBackend::CpuInterpreter, ExecutionBackend::CpuJit] {
            assert_eq!(
                execute_with_backend(backend, programs, tapes, &anchors).unwrap(),
                expected
            );
        }
        #[cfg(target_os = "macos")]
        {
            let backend = ExecutionBackend::Metal;
            if let Some(actual) = grid(execute_with_backend(backend, programs, tapes, &anchors))
                && actual != expected
            {
                let mismatch = actual
                    .iter()
                    .zip(&expected)
                    .position(|(actual, expected)| actual != expected)
                    .unwrap_or(actual.len().min(expected.len()));
                let output_mismatch = actual.get(mismatch).zip(expected.get(mismatch)).and_then(
                    |(actual, expected)| {
                        actual
                            .iter()
                            .zip(expected)
                            .position(|(actual, expected)| actual != expected)
                    },
                );
                panic!(
                    "backend {backend:?} diverged at evaluation {mismatch}, output {output_mismatch:?}: actual count {}, expected count {}, actual output {:?}, expected output {:?}",
                    actual.get(mismatch).map_or(0, Vec::len),
                    expected.get(mismatch).map_or(0, Vec::len),
                    output_mismatch.and_then(|index| actual.get(mismatch)?.get(index)),
                    output_mismatch.and_then(|index| expected.get(mismatch)?.get(index)),
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn assert_metal_parity(programs: &[Program], tapes: &[TapeDoc]) {
        let anchors = tapes
            .iter()
            .map(|tape| {
                programs
                    .iter()
                    .map(|_| (0..tape.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected = execute_batch(programs, tapes, Some(&anchors)).unwrap();
        if let Some(actual) = grid(metal_grid_backend::execute(programs, tapes, &anchors)) {
            assert_eq!(actual, expected);
        }
    }

    fn assert_scm_native_parity(query_source: &str, source: &str) {
        use tree_sitter::StreamingIterator;

        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(query_source, &language).unwrap();
        let tape = tape(source);
        let outputs = program.execute(&tape).unwrap();
        let actual = outputs
            .iter()
            .map(|output| {
                output
                    .captures
                    .iter()
                    .map(|capture| {
                        let string = program.labels[usize::from(capture.label)];
                        (
                            std::str::from_utf8(&program.strings[usize::from(string)])
                                .unwrap()
                                .to_owned(),
                            capture.span.start,
                            capture.span.end,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let query = tree_sitter::Query::new(&language, query_source).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut expected = Vec::new();
        while let Some(found) = matches.next() {
            expected.push(
                found
                    .captures()
                    .iter()
                    .map(|capture| {
                        (
                            query.capture_names()[capture.index as usize].to_owned(),
                            capture.node.start_byte(),
                            capture.node.end_byte(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(actual, expected, "query: {query_source}\nsource:\n{source}");

        // QueryMatch does not expose its structural root. Add one oracle-only capture and compare
        // root spans as an unordered relation, while the original query above remains the ordering
        // oracle. One common root capture does not change capture-containment relationships.
        const MATCH_CAPTURE: &str = "__tempo_match_span";
        let span_query_source = format!("{query_source} @{MATCH_CAPTURE}");
        let span_query = tree_sitter::Query::new(&language, &span_query_source).unwrap();
        let match_capture = span_query
            .capture_names()
            .iter()
            .position(|name| *name == MATCH_CAPTURE)
            .unwrap() as u32;
        let mut span_cursor = tree_sitter::QueryCursor::new();
        let mut span_matches =
            span_cursor.matches(&span_query, tree.root_node(), source.as_bytes());
        let mut expected_spans = Vec::new();
        while let Some(found) = span_matches.next() {
            let root = found
                .captures()
                .iter()
                .find(|capture| capture.index == match_capture)
                .unwrap()
                .node
                .byte_range();
            let captures = found
                .captures()
                .iter()
                .filter(|capture| capture.index != match_capture)
                .map(|capture| {
                    (
                        span_query.capture_names()[capture.index as usize].to_owned(),
                        capture.node.start_byte(),
                        capture.node.end_byte(),
                    )
                })
                .collect::<Vec<_>>();
            expected_spans.push((captures, root.start, root.end));
        }
        let mut actual_spans = outputs
            .iter()
            .map(|output| {
                (
                    output
                        .captures
                        .iter()
                        .map(|capture| {
                            let string = program.labels[usize::from(capture.label)];
                            (
                                std::str::from_utf8(&program.strings[usize::from(string)])
                                    .unwrap()
                                    .to_owned(),
                                capture.span.start,
                                capture.span.end,
                            )
                        })
                        .collect::<Vec<_>>(),
                    tape.transducer_span(output.start).start,
                    tape.transducer_span(output.start).end,
                )
            })
            .collect::<Vec<_>>();
        actual_spans.sort();
        expected_spans.sort();
        assert_eq!(
            actual_spans, expected_spans,
            "match spans differ for query: {query_source}\nsource:\n{source}"
        );
    }

    #[test]
    fn alternative_programs_expose_every_sound_root_kind() {
        let language = SupportLang::Python.get_ts_language();
        let program =
            Program::compile_tree_sitter_query_pattern("[(identifier) (string)] @item", &language)
                .unwrap();
        let expected = [
            language.id_for_node_kind("identifier", true),
            language.id_for_node_kind("string", true),
        ]
        .into_iter()
        .collect();
        assert_eq!(program.leading_kinds(), Some(expected));
    }

    #[test]
    fn complete_scm_file_parser_returns_top_level_patterns_without_a_copied_catalog() {
        let source = r##"
; leading comment
(class_definition) @definition

((identifier) @name
 (#eq? @name "escaped\\\"value"))

["+" "-"] @operator
"##;
        assert_eq!(
            Program::tree_sitter_top_level_patterns(source).unwrap(),
            [
                "(class_definition) @definition",
                "((identifier) @name\n (#eq? @name \"escaped\\\\\\\"value\"))",
                "[\"+\" \"-\"] @operator",
            ]
        );
        assert!(Program::tree_sitter_top_level_patterns("(module").is_err());
        assert!(Program::tree_sitter_top_level_patterns("[\"x").is_err());
    }

    #[test]
    fn scm_compiler_accepts_captureless_patterns_and_rejects_invalid_input() {
        let language = SupportLang::Python.get_ts_language();
        let captureless =
            Program::compile_tree_sitter_query_pattern("(module)", &language).unwrap();
        assert_eq!(captureless.capture_names().count(), 0);
        assert!(
            Program::compile_tree_sitter_query_pattern(
                "(module) @scope (identifier) @extra",
                &language,
            )
            .is_err()
        );
        assert!(
            Program::compile_tree_sitter_query_pattern("(not_a_python_kind) @x", &language)
                .is_err()
        );
        let first =
            Program::compile_tree_sitter_query_pattern("  (module)   @root.scope  ", &language)
                .unwrap();
        let second =
            Program::compile_tree_sitter_query_pattern("(module) @root.scope", &language).unwrap();
        assert_eq!(first.capture_names().collect::<Vec<_>>(), ["root.scope"]);
        assert_eq!(
            first.canonical_digest("python", b"(module) @root.scope"),
            second.canonical_digest("python", b"(module) @root.scope")
        );
    }

    #[test]
    fn scm_parser_lowers_nested_fields_alternatives_and_predicates() {
        assert_scm_native_parity(
            "(function_definition name: (identifier) @name body: (block (return_statement (identifier) @value))) @function",
            "def answer():\n    return result\n",
        );
        assert_scm_native_parity(
            "([(integer) (float)] @number (#match? @number \"^[12]\"))",
            "first = 1\nsecond = 3.0\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)+) @root",
            "a\nb\nc\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)? (expression_statement (integer) @tail)) @root",
            "a\n1\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)* (expression_statement (integer) @tail)) @root",
            "1\n",
        );
        assert_scm_native_parity(
            "(module (expression_statement (identifier) @item)+ (expression_statement (integer) @number)+) @root",
            "a\nb\n1\n2\n",
        );
        assert_scm_native_parity(
            "(module . (expression_statement [(identifier) (integer)] @item)+ .) @root",
            "a\n1\n",
        );
    }

    #[test]
    fn scm_quantifiers_match_native_for_groups_competing_paths_and_recovery() {
        let cases = [
            (
                "(module (expression_statement (identifier) @left)? (expression_statement (identifier) @right)? (expression_statement (integer) @tail)) @root",
                "a\nb\n1\n",
            ),
            (
                "(module ((expression_statement (identifier) @name) (expression_statement (integer) @value))+) @root",
                "a\n1\nb\n2\n",
            ),
            (
                "(module (expression_statement [(identifier) (integer)] @item)+ (expression_statement (string) @tail)) @root",
                "a\n1\nb\n\"done\"\n",
            ),
            (
                "(module (expression_statement (identifier) @item)* (expression_statement (integer) @tail)+) @root",
                "a\nb\n1\n2\n",
            ),
            (
                "(module (ERROR) @error+ ) @root",
                "def broken(:\n    pass\n",
            ),
            (
                "(module (function_definition (MISSING \":\")? @missing) @function) @root",
                "def broken()\n    pass\n",
            ),
            (
                "(module ((comment)+)+ @comments) @root",
                "# one\n# two\n# three\n",
            ),
            (
                "(module (comment)+ (expression_statement (integer)))",
                "# before\n1\n# after\n",
            ),
            (
                "(module ((comment)+) (expression_statement (integer)))",
                "# before\n1\n# after\n",
            ),
        ];
        for (query, source) in cases {
            assert_scm_native_parity(query, source);
        }
    }

    #[test]
    fn tree_sitter_capture_containment_uses_node_capture_and_span_identity() {
        let capture = |label, node, start, end| CaptureIdentity {
            label,
            node,
            start,
            end,
        };
        assert_eq!(capture_identity_containment(&[], &[]), (true, true));
        assert_eq!(
            capture_identity_containment(
                &[capture(0, 1, 0, 1), capture(0, 2, 2, 3)],
                &[capture(0, 1, 0, 1)],
            ),
            (true, false)
        );
        assert_eq!(
            capture_identity_containment(&[capture(0, 1, 0, 1)], &[capture(0, 2, 2, 3)],),
            (false, false)
        );
        assert_eq!(
            capture_identity_containment(&[capture(0, 1, 0, 1)], &[capture(1, 1, 0, 1)],),
            (false, false)
        );
        assert_eq!(
            capture_identity_containment(&[capture(0, 1, 4, 4)], &[capture(0, 2, 4, 4)],),
            (false, false),
            "distinct zero-width nodes at one point are not capture subsets",
        );
    }

    #[test]
    #[ignore = "offline inspection of compiled sibling-repeat instructions"]
    fn inspect_compiled_sibling_repeat_instructions() {
        let language = SupportLang::Python.get_ts_language();
        for query in [
            "(module (comment)+ (comment) @middle (comment)+)",
            "(module ((comment) @item)+)",
            "(module (comment)* (identifier) @id)",
        ] {
            let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
            eprintln!("QUERY {query}");
            for (pc, instruction) in program.words.iter().enumerate() {
                eprintln!("{pc}: {:?}", split(*instruction));
            }
            eprintln!("labels={:?}", program.labels);
        }
    }

    #[test]
    fn scm_quantified_runs_preserve_each_distinct_middle_capture() {
        let middle = "(module (comment)+ (comment) @middle (comment)+)";
        let captured_run = "(module ((comment) @item)+)";
        let captured_alternatives = "(module [(comment) @left (comment) @right]+)";
        let variable_width_alternatives = "(module [(comment) @item ((comment) (comment) @item)]+)";
        for count in 0..=8 {
            let source = (0..count)
                .map(|index| format!("# {index}\n"))
                .collect::<String>();
            assert_scm_native_parity(middle, &source);
            assert_scm_native_parity(captured_run, &source);
            assert_scm_native_parity(captured_alternatives, &source);
            assert_scm_native_parity(variable_width_alternatives, &source);
        }

        let stress = (0..128)
            .map(|index| format!("# {index}\n"))
            .collect::<String>();
        assert_scm_native_parity(middle, &stress);
        assert_scm_native_parity(captured_run, &stress);

        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(middle, &language).unwrap();
        let stress_tape = tape(&stress);
        let plain_started = Instant::now();
        let plain_output = program.execute(&stress_tape).unwrap();
        let plain_elapsed = plain_started.elapsed();

        let jit_programs = vec![program.clone()];
        let jit_compile_started = Instant::now();
        let native = native_ir::lower(&program).unwrap();
        let _compiled = native_cpu::compile_straight_line(&native).unwrap();
        let jit_compile_elapsed = jit_compile_started.elapsed();
        let jit_tapes = vec![stress_tape.clone()];
        let jit_roots = (0..stress_tape.transducer_len() as u32).collect::<Vec<_>>();
        let jit_anchors = vec![vec![jit_roots]];
        let jit_started = Instant::now();
        let jit_output = execute_with_backend(
            ExecutionBackend::CpuJit,
            &jit_programs,
            &jit_tapes,
            &jit_anchors,
        )
        .unwrap();
        let jit_elapsed = jit_started.elapsed();
        assert_eq!(jit_output[0][0], plain_output);
        #[cfg(target_os = "macos")]
        if let Some(actual) = grid(metal_grid_backend::execute(&jit_programs, &jit_tapes, &jit_anchors)) {
            assert_eq!(actual[0][0], plain_output);
        }
        if let Some(iterations) = std::env::var("TEMPO_QUANTIFIER_PROFILE_ITERATIONS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
        {
            for _ in 0..iterations {
                black_box(
                    execute_with_backend(
                        ExecutionBackend::CpuJit,
                        &jit_programs,
                        &jit_tapes,
                        &jit_anchors,
                    )
                    .unwrap(),
                );
            }
        }

        let started = Instant::now();
        let (stress_output, stats) = program.execute_with_stats(&stress_tape).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(stress_output.len(), 126);
        assert!(stats.explored_states < 100_000, "{stats:?}");
        assert!(stats.max_pending_paths < 512, "{stats:?}");
        assert!(stats.peak_retained_bytes < 2 * 1024 * 1024, "{stats:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?} {stats:?}");
        assert!(plain_elapsed < Duration::from_secs(2), "{plain_elapsed:?}");
        assert!(jit_elapsed < Duration::from_secs(2), "{jit_elapsed:?}");
        eprintln!(
            "128-comment quantifier stress: interpreter={plain_elapsed:?} jitCompile={jit_compile_elapsed:?} jit={jit_elapsed:?} instrumented={elapsed:?} {stats:?}"
        );

        assert_scm_native_parity(
            "((module ((comment) @item)+) (#eq? @item \"# 0\"))",
            "# 0\n# 1\n",
        );
    }

    #[test]
    fn scm_quantifier_zero_paths_preserve_anchor_obligations() {
        let cases = [
            (
                "(module . (comment)* (expression_statement (identifier) @value))",
                "value\n",
            ),
            (
                "(module . (comment)* (expression_statement (identifier) @value))",
                "1\nvalue\n",
            ),
            (
                "(module (expression_statement (identifier) @left) . (comment)* (expression_statement (identifier) @right))",
                "left\n1\nright\n",
            ),
            (
                "(module (expression_statement (identifier) @left) . (comment)* . (expression_statement (identifier) @right))",
                "left\n1\nright\n",
            ),
            (
                "(module (expression_statement (identifier) @left) (comment)* . (expression_statement (identifier) @right))",
                "left\n1\nright\n",
            ),
            (
                "(module (expression_statement (identifier) @value) (comment)* .)",
                "value\n",
            ),
            (
                "(module (expression_statement (identifier) @value) (comment)* .)",
                "value\n1\n",
            ),
            (
                "(module (expression_statement (identifier) @value) (expression_statement (integer))* .)",
                "value\n\"later\"\n",
            ),
        ];
        for (query, source) in cases {
            assert_scm_native_parity(query, source);
        }
    }

    #[test]
    fn scm_quantifiers_match_native_for_generated_sibling_counts() {
        for count in 0..=5 {
            let mut source = String::new();
            for index in 0..count {
                source.push_str(&format!("name_{index}\n"));
            }
            source.push_str("1\n");
            assert_scm_native_parity(
                "(module (expression_statement (identifier) @item)* (expression_statement (integer) @tail)) @root",
                &source,
            );
            assert_scm_native_parity(
                "(module (expression_statement (identifier))* (expression_statement (integer) @tail)) @root",
                &source,
            );
            if count > 0 {
                assert_scm_native_parity(
                    "(module (expression_statement (identifier) @item)+ (expression_statement (integer) @tail)) @root",
                    &source,
                );
            }
        }

        assert_scm_native_parity(
            "(module . (expression_statement (identifier) @left) . (expression_statement (identifier) @right)) @root",
            "left\n# an extra node\nright\n",
        );
        assert_scm_native_parity(
            "((module (expression_statement (identifier) @item)+) @root (#any-eq? @item \"name_1\"))",
            "name_0\nname_1\nname_2\n",
        );
        assert_scm_native_parity(
            "((module (expression_statement (identifier) @item)+) @root (#any-not-eq? @item \"name_1\"))",
            "name_0\nname_1\nname_2\n",
        );
    }

    #[test]
    fn cpu_jit_compiles_general_scm_control_flow() {
        let language = SupportLang::Python.get_ts_language();
        let source = "# 0\n# 1\n# 2\n# 3\nvalue = call(arg)\n";
        let programs = [
            "(module (comment)+ (comment) @middle (comment)+)",
            "(module ((comment) @item)+)",
            "(module (expression_statement (assignment left: (identifier) @left right: (call function: (identifier) @callee arguments: (argument_list (identifier) @argument)))) @root)",
            "((identifier) @name (#eq? @name \"value\"))",
            "((identifier) @name (#set! \"role\" \"definition\") (#downcase! @name))",
        ]
        .into_iter()
        .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
        .collect::<Vec<_>>();
        let tapes = vec![tape(source)];
        let roots = (0..tapes[0].transducer_len() as u32).collect::<Vec<_>>();
        let anchors = vec![programs.iter().map(|_| roots.clone()).collect::<Vec<_>>()];
        let expected = execute_batch(&programs, &tapes, Some(&anchors)).unwrap();
        assert_eq!(
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &tapes, &anchors).unwrap(),
            expected
        );
        #[cfg(target_os = "macos")]
        assert_metal_parity(&programs, &tapes);
    }

    #[test]
    fn all_backends_execute_every_verified_instruction_and_mode() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let function = grammar.field_id_for_name("function").unwrap().get();
        let alternative = grammar.field_id_for_name("alternative").unwrap().get();
        let labels = || vec![0];
        let strings = || vec![b"match".to_vec()];
        let plain =
            |words| Program::new(words, strings(), vec![], labels(), vec![100_000]).unwrap();
        let mut programs = vec![
            plain(vec![word(STOP, 0, 0)]),
            plain(vec![word(KIND, 0, identifier), word(EMIT, 0, 0)]),
            plain(vec![word(FIELD, 0, function), word(EMIT, 0, 0)]),
            plain(vec![word(FLAGS, 1, 0b1110), word(EMIT, 0, 0)]),
            plain(vec![word(FIRST, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(FIRST, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(LAST, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(LAST, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(ABSENT_FIELD, 0, alternative), word(EMIT, 0, 0)]),
            plain(vec![word(CHILD, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(CHILD, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(NEXT, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(NEXT, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(PARENT, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(DESCEND, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(DESCEND, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![word(FOLLOWING, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![word(FOLLOWING, NAMED, 0), word(EMIT, 0, 0)]),
            plain(vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(EMIT, 0, 0),
            ]),
            plain(vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![
                word(REPEAT_BEGIN, 0, 7),
                word(REPEAT_BUMP, 0, 7),
                word(REPEAT_END, 0, 7),
                word(EMIT, 0, 0),
            ]),
            plain(vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)]),
            plain(vec![
                word(FORK, GREEDY, 2),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
            ]),
            plain(vec![word(GOTO, 0, 1), word(EMIT, 0, 0)]),
        ];
        #[cfg(target_os = "macos")]
        let mut metal_programs = programs.clone();
        for required in 0_u8..=0x0f {
            for forbidden in 0_u16..=0x000f {
                if u16::from(required) & forbidden == 0 {
                    programs.push(plain(vec![
                        word(FLAGS, required, forbidden),
                        word(EMIT, 0, 0),
                    ]));
                }
            }
        }
        for mode in [0, 1, 2] {
            programs.push(plain(vec![word(SIZE, mode, 1), word(EMIT, 0, 0)]));
        }
        for mode in [0x80, 0x81, 0x82] {
            programs.push(plain(vec![word(SIZE, mode, 0), word(EMIT, 0, 0)]));
        }
        for predicate in [
            TextPredicate::Equal(0),
            TextPredicate::Prefix(0),
            TextPredicate::Regex {
                source: 0,
                compiled: Regex::new("match").unwrap(),
            },
            TextPredicate::LuaPattern {
                source: 0,
                compiled: LuaPattern::parse(b"match").unwrap(),
            },
        ] {
            let program = Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec(), b"output".to_vec()],
                vec![predicate],
                vec![1],
                vec![],
            )
            .unwrap();
            #[cfg(target_os = "macos")]
            metal_programs.push(program.clone());
            programs.push(program);
        }
        let assertion_program = Program::new_with_capture_predicates(
            vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec(), b"output".to_vec()],
            vec![],
            vec![CapturePredicate::Kind {
                capture: 0,
                kind: identifier,
                negated: false,
                any: false,
            }],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        #[cfg(target_os = "macos")]
        metal_programs.push(assertion_program.clone());
        programs.push(assertion_program);
        let source = "# first\nvalue = call(argument)\nif value:\n  pass\n";
        let python_tape = tape(source);
        assert_jit_parity(&programs, std::slice::from_ref(&python_tape));
        #[cfg(target_os = "macos")]
        {
            let alternatives = |blocks: Vec<Vec<u32>>| {
                let prefix = blocks.len() + 1;
                let mut at = prefix;
                let mut words = Vec::new();
                for block in &blocks {
                    words.push(word(FORK, 0, at as u16));
                    at += block.len();
                }
                words.push(word(STOP, 0, 0));
                words.extend(blocks.into_iter().flatten());
                plain(words)
            };
            let flags = alternatives(
                (0_u8..=0x0f)
                    .flat_map(|required| {
                        (0_u16..=0x000f).filter_map(move |forbidden| {
                            (u16::from(required) & forbidden == 0)
                                .then_some(vec![word(FLAGS, required, forbidden), word(EMIT, 0, 0)])
                        })
                    })
                    .collect(),
            );
            let sizes = alternatives(
                [0, 1, 2, 0x80, 0x81, 0x82]
                    .into_iter()
                    .map(|mode| {
                        vec![
                            word(SIZE, mode, u16::from(mode & SIZE_LONG == 0)),
                            word(EMIT, 0, 0),
                        ]
                    })
                    .collect(),
            );
            metal_programs.extend([flags, sizes]);
            assert_metal_parity(&metal_programs, std::slice::from_ref(&python_tape));
        }
        for words in [
            vec![word(RESTORE_CURSOR, 0, 0), word(EMIT, 0, 0)],
            vec![word(REPEAT_BUMP, 0, 7), word(EMIT, 0, 0)],
            vec![word(REPEAT_END, 0, 7), word(EMIT, 0, 0)],
        ] {
            let program = plain(words);
            let expected = program.execute_roots(&python_tape, [0]).unwrap_err();
            let actual = execute_with_backend(
                ExecutionBackend::CpuJit,
                std::slice::from_ref(&program),
                std::slice::from_ref(&python_tape),
                &[vec![vec![0]]],
            )
            .unwrap_err();
            assert_eq!(actual, expected);
            #[cfg(target_os = "macos")]
            {
                let actual = metal_grid_backend::execute(
                    std::slice::from_ref(&program),
                    std::slice::from_ref(&python_tape),
                    &[vec![vec![0]]],
                )
                .unwrap_err();
                assert_eq!(actual, expected);
            }
        }

        let js = SupportLang::JavaScript;
        let js_language = js.get_ts_language();
        let js_source = "run();\n";
        let supertype = Program::compile_tree_sitter_query_pattern(
            "(primary_expression/call_expression) @reference",
            &js_language,
        )
        .unwrap();
        let js_grammar = Grammar::new(js).unwrap();
        let js_native = StrDoc::new(js_source, js);
        let js_image =
            TapeImage::encode(js_native.tree.root_node(), &js_grammar, js_source, &|| {
                false
            })
            .unwrap();
        let js_tape = TapeDoc::restore(
            Arc::new(js_source.to_owned()),
            js_grammar,
            &js_image,
            &|| false,
        )
        .unwrap();
        assert_jit_parity(
            std::slice::from_ref(&supertype),
            std::slice::from_ref(&js_tape),
        );
        #[cfg(target_os = "macos")]
        assert_metal_parity(std::slice::from_ref(&supertype), &[js_tape]);

        let opcodes = programs
            .iter()
            .chain(std::iter::once(&supertype))
            .flat_map(|program| program.words.iter().map(|word| split(*word).0))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            opcodes,
            BTreeSet::from([
                STOP,
                KIND,
                FIELD,
                FLAGS,
                TEXT,
                SIZE,
                ASSERT,
                FIRST,
                LAST,
                ABSENT_FIELD,
                SUPERTYPE,
                CHILD,
                NEXT,
                PARENT,
                DESCEND,
                FOLLOWING,
                SAVE_CURSOR,
                RESTORE_CURSOR,
                CAPTURE,
                REPEAT_BEGIN,
                REPEAT_BUMP,
                REPEAT_END,
                FORK,
                GOTO,
                EMIT,
            ])
        );
    }

    #[test]
    fn scm_supertype_paths_use_persisted_hidden_ancestors() {
        use tree_sitter::StreamingIterator;

        let support = SupportLang::JavaScript;
        let language = support.get_ts_language();
        let source = "run();\n";
        let query = "(primary_expression/call_expression) @reference";
        let program = Program::compile_tree_sitter_query_pattern(query, &language).unwrap();
        let grammar = Grammar::new(support).unwrap();
        let native_doc = StrDoc::new(source, support);
        let image =
            TapeImage::encode(native_doc.tree.root_node(), &grammar, source, &|| false).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let actual = program
            .execute(&tape)
            .unwrap()
            .into_iter()
            .flat_map(|output| output.captures)
            .map(|capture| capture.span)
            .collect::<Vec<_>>();

        let query = tree_sitter::Query::new(&language, query).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, native_doc.tree.root_node(), source.as_bytes());
        let mut expected = Vec::new();
        while let Some(found) = matches.next() {
            expected.extend(
                found
                    .captures()
                    .iter()
                    .map(|capture| capture.node.byte_range()),
            );
        }
        assert_eq!(actual, expected);
        assert!(!actual.is_empty());
    }

    #[test]
    fn scm_host_predicates_filter_text_kinds_lines_and_relations() {
        let language = SupportLang::Python.get_ts_language();
        let source = "insert(foo)\nvalue = 1\na = (\n  b\n)\n";
        let cases = [
            ("((identifier) @x (#contains? @x \"sert\"))", vec!["insert"]),
            (
                "([(identifier) (integer)] @x (#not-kind-eq? @x \"integer\"))",
                vec!["insert", "foo", "value", "a", "b"],
            ),
            (
                "((identifier) @x (#has-parent? @x argument_list))",
                vec!["foo"],
            ),
            (
                "((identifier) @x (#has-ancestor? @x call))",
                vec!["insert", "foo"],
            ),
            (
                "((assignment left: (identifier) @left right: (parenthesized_expression (identifier) @right)) (#same-line? @left @right))",
                Vec::new(),
            ),
            (
                "((assignment left: (identifier) @left right: (parenthesized_expression (identifier) @right)) (#not-same-line? @left @right))",
                vec!["a", "b"],
            ),
        ];
        let tape = tape(source);
        let programs = cases
            .iter()
            .map(|(query, _)| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
            .collect::<Vec<_>>();
        for ((query, expected), program) in cases.into_iter().zip(&programs) {
            let actual = program
                .execute(&tape)
                .unwrap()
                .into_iter()
                .flat_map(|output| output.captures)
                .filter(|capture| program.label_name(capture.label) != Some("tree-sitter-query"))
                .map(|capture| source[capture.span].to_owned())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query: {query}");
        }
        #[cfg(target_os = "macos")]
        assert_metal_parity(&programs, std::slice::from_ref(&tape));
    }

    #[test]
    fn scm_output_directives_transform_text_adjacency_and_language() {
        let language = SupportLang::Python.get_ts_language();
        let source =
            "# old\n\n# near\n# nearest\ndef run():\n  pass\nRUBY\n'application/ecmascript'\n";
        let adjacency = Program::compile_tree_sitter_query_pattern(
            r#"((module (comment)+ @doc . (function_definition) @definition)
                 (#strip! @doc "^#\\s*")
                 (#select-adjacent! @doc @definition))"#,
            &language,
        )
        .unwrap();
        let label = |name: &[u8]| {
            adjacency
                .labels
                .iter()
                .position(|label| adjacency.strings[usize::from(*label)] == name)
                .unwrap() as u16
        };
        let mut captures = vec![
            Capture {
                label: label(b"doc"),
                node: 0,
                span: 0..5,
                text: None,
                properties: Vec::new(),
            },
            Capture {
                label: label(b"doc"),
                node: 1,
                span: 7..13,
                text: None,
                properties: Vec::new(),
            },
            Capture {
                label: label(b"doc"),
                node: 2,
                span: 14..23,
                text: None,
                properties: Vec::new(),
            },
            Capture {
                label: label(b"definition"),
                node: 3,
                span: 24..42,
                text: None,
                properties: Vec::new(),
            },
        ];
        adjacency
            .apply_directives(&mut captures, source.as_bytes())
            .unwrap()
            .unwrap();
        let transformed = captures
            .iter()
            .filter(|capture| capture.label == label(b"doc"))
            .map(|capture| std::str::from_utf8(capture.text.as_deref().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(transformed, ["near", "nearest"]);

        let query = r#"
          ((identifier) @language (#downcase! @language))
          ((string (string_content) @mime) (#set-lang-from-mimetype! @mime))
        "#;
        let directive_programs = Program::tree_sitter_top_level_patterns(query)
            .unwrap()
            .iter()
            .map(|pattern| Program::compile_tree_sitter_query_pattern(pattern, &language).unwrap())
            .collect::<Vec<_>>();
        let directive_tape = tape(source);
        let outputs = directive_programs
            .iter()
            .flat_map(|program| program.execute(&directive_tape).unwrap())
            .collect::<Vec<_>>();
        #[cfg(target_os = "macos")]
        {
            assert_metal_parity(&directive_programs, std::slice::from_ref(&directive_tape));
            assert_metal_parity(
                std::slice::from_ref(&adjacency),
                std::slice::from_ref(&directive_tape),
            );
        }
        assert!(
            outputs
                .iter()
                .flat_map(|output| &output.captures)
                .any(|capture| capture.text.as_deref() == Some(b"ruby"))
        );
        assert!(outputs.iter().any(|output| {
            output
                .dynamic_properties
                .iter()
                .any(|(key, value)| key == b"language" && value.as_deref() == Some(b"javascript"))
        }));

        let alias = Program::compile_tree_sitter_query_pattern(
            "((module (comment)+ @doc . (function_definition) @definition) (#set-adjacent! @doc @definition))",
            &language,
        );
        assert!(alias.is_ok());
    }

    #[test]
    fn scm_directives_adjust_capture_spans_and_retain_properties() {
        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#offset! @name 0 1 0 -1) (#set! role \"trimmed\") (#set! @name bo.commentstring \"// %s\"))",
            &language,
        )
        .unwrap();
        let value_tape = tape("value\n");
        let output = program.execute(&value_tape).unwrap();
        #[cfg(target_os = "macos")]
        assert_metal_parity(
            std::slice::from_ref(&program),
            std::slice::from_ref(&value_tape),
        );
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].captures[0].span, 1..4);
        assert_eq!(output[0].captures[0].properties.len(), 1);
        let (capture_key, capture_value) = output[0].captures[0].properties[0];
        assert_eq!(program.string_value(capture_key), Some("bo.commentstring"));
        assert_eq!(
            capture_value.and_then(|value| program.string_value(value)),
            Some("// %s")
        );
        assert_eq!(output[0].properties.len(), 1);
        let (key, value) = output[0].properties[0];
        assert_eq!(program.strings[usize::from(key)], b"role");
        assert_eq!(
            program.strings[usize::from(value.expect("property value"))],
            b"trimmed"
        );

        let invalid = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#offset! @name 0 -1 0 0))",
            &language,
        )
        .unwrap();
        assert!(invalid.execute(&tape("value\n")).unwrap().is_empty());

        let utf8_split = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#offset! @name 0 1 0 0))",
            &language,
        )
        .unwrap();
        assert!(utf8_split.execute(&tape("éx\n")).unwrap().is_empty());
    }

    #[test]
    fn lua_patterns_use_lua_byte_classes_and_repetition() {
        let cases = [
            ("^[A-Z].*[a-z]", b"ClassName".as_slice(), true),
            ("^[A-Z].*[a-z]", b"CONSTANT".as_slice(), false),
            ("^[%l_].*$", b"_member".as_slice(), true),
            ("^%u", b"Builder".as_slice(), true),
            ("^%([^\n]", b"(value".as_slice(), true),
            ("[^\n ]%)$", b"value)".as_slice(), true),
            ("^else", b"or_else".as_slice(), false),
        ];
        for (source, value, expected) in cases {
            let pattern = LuaPattern::parse(source.as_bytes()).unwrap();
            assert_eq!(pattern.matches(value), expected, "{source}");
        }
        assert!(LuaPattern::parse(b"(%a+)").unwrap().matches(b"name"));
        assert!(
            LuaPattern::parse(b"^[-][-](%s?)@")
                .unwrap()
                .matches(b"-- @")
        );
        assert!(LuaPattern::parse(b"%f[%a]word").unwrap().matches(b"a word"));
        assert!(!LuaPattern::parse(b"%f[%a]word").unwrap().matches(b"sword"));
        assert!(LuaPattern::parse(b"^%b()$").unwrap().matches(b"(a(b)c)"));
        assert!(!LuaPattern::parse(b"^%b()$").unwrap().matches(b"(a(b)"));
        assert!(
            LuaPattern::parse(b"^(%a+)%s+%1$")
                .unwrap()
                .matches(b"same same")
        );
        assert!(
            !LuaPattern::parse(b"^(%a+)%s+%1$")
                .unwrap()
                .matches(b"same other")
        );
        assert!(LuaPattern::parse(b"()value").unwrap().matches(b"value"));
        assert!(LuaPattern::parse(b"^a.-b$").unwrap().matches(b"axxxbxxb"));
        assert!(LuaPattern::parse(b"^%z$").unwrap().matches(b"\0"));
        assert!(LuaPattern::parse(b"^..$").unwrap().matches("é".as_bytes()));
        assert!(!LuaPattern::parse(b"^%a+$").unwrap().matches("é".as_bytes()));
        assert!(LuaPattern::parse(b"%1").is_err());
        assert!(LuaPattern::parse(b"()%1").is_err());
        assert!(LuaPattern::parse(b"(%a%1)").is_err());
        assert!(LuaPattern::parse(b"(%a+").is_err());
        assert!(LuaPattern::parse(b"%b(").is_err());
        assert!(LuaPattern::parse(b"%f%a").is_err());

        let bounded = LuaPattern::parse(b"a*").unwrap();
        let mut exhausted = MAX_STEPS_PER_PATH;
        assert!(bounded.matches_with_work(b"aaaa", &mut exhausted).is_err());

        let language = SupportLang::Python.get_ts_language();
        let program = Program::compile_tree_sitter_query_pattern(
            "((identifier) @name (#not-lua-match? @name \"^skip$\"))",
            &language,
        )
        .unwrap();
        let source = "keep = skip\n";
        let found = program
            .execute(&tape(source))
            .unwrap()
            .into_iter()
            .flat_map(|output| output.captures)
            .map(|capture| &source[capture.span])
            .collect::<Vec<_>>();
        assert_eq!(found, ["keep"]);
    }

    #[test]
    fn canonical_identity_changes_for_every_program_component() {
        fn changed(base: &Program, mutate: impl FnOnce(&mut Program)) {
            let original = base.canonical_digest("python", b"source");
            let mut changed = base.clone();
            mutate(&mut changed);
            assert_ne!(original, changed.canonical_digest("python", b"source"));
        }

        let language = SupportLang::Python.get_ts_language();
        let base =
            Program::compile_tree_sitter_query_pattern("(identifier) @symbol", &language).unwrap();
        assert_ne!(
            base.canonical_digest("python", b"source"),
            base.canonical_digest("javascript", b"source")
        );
        assert_ne!(
            base.canonical_digest("python", b"source"),
            base.canonical_digest("python", b"other source")
        );
        changed(&base, |program| program.words[0] ^= 1 << 8);
        changed(&base, |program| program.strings[0].push(b'x'));
        changed(&base, |program| {
            program.predicates.push(TextPredicate::Equal(0))
        });
        changed(&base, |program| program.labels[0] = 1);
        changed(&base, |program| program.longs.push(1));
        changed(&base, |program| {
            program.tree_sitter_query = !program.tree_sitter_query
        });
        changed(&base, |program| {
            program.directives.push(Directive::Set {
                key: 0,
                value: None,
            });
        });

        let capture_predicates = [
            CapturePredicate::Equal {
                left: 0,
                right: 0,
                negated: false,
                any: false,
            },
            CapturePredicate::Text {
                capture: 0,
                predicate: 0,
                negated: false,
                any: false,
            },
            CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![0],
                negated: false,
                any: false,
            },
            CapturePredicate::Kind {
                capture: 0,
                kind: 1,
                negated: false,
                any: false,
            },
        ];
        for predicate in capture_predicates {
            changed(&base, |program| program.capture_predicates.push(predicate));
        }
        let with_any_of = {
            let mut program = base.clone();
            program.capture_predicates.push(CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![0],
                negated: false,
                any: false,
            });
            program
        };
        changed(&with_any_of, |program| {
            let CapturePredicate::AnyOf { strings, .. } = &mut program.capture_predicates[0] else {
                unreachable!()
            };
            strings.push(1);
        });
    }

    #[test]
    fn fixed_words_execute_identifier_transducer() {
        let tape = tape("execute(x)\nother()\nexecute(y)\n");
        let program = identifier_program("execute");
        assert!(
            program
                .words
                .iter()
                .all(|word| word.to_le_bytes().len() == 4)
        );
        let output = program.execute(&tape).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].captures[0].span, 0..7);
        assert_eq!(output[1].captures[0].span, 19..26);
    }

    #[test]
    fn navigation_captures_eval_call_and_callee() {
        let grammar = SupportLang::Python.get_ts_language();
        let call = grammar.id_for_node_kind("call", true);
        let identifier = grammar.id_for_node_kind("identifier", true);
        let function = grammar.field_id_for_name("function").unwrap().get();
        let program = Program::new(
            vec![
                word(KIND, 0, call),
                word(CAPTURE, 0, 0),
                word(CHILD, 0, 0),
                word(FIELD, 0, function),
                word(KIND, 0, identifier),
                word(TEXT, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 2),
            ],
            vec![
                b"eval".to_vec(),
                b"call".to_vec(),
                b"callee".to_vec(),
                b"match".to_vec(),
            ],
            vec![TextPredicate::Equal(0)],
            vec![1, 2, 3],
            vec![],
        )
        .unwrap();
        let output = program
            .execute(&tape("eval(x)\nobj.eval(x)\nother(x)\n"))
            .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].captures[0].span, 0..7);
        assert_eq!(output[0].captures[1].span, 0..4);
    }

    #[test]
    fn alternatives_do_not_share_capture_mutation() {
        let program = Program::new(
            vec![
                word(
                    KIND,
                    0,
                    SupportLang::Python
                        .get_ts_language()
                        .id_for_node_kind("identifier", true),
                ),
                word(FORK, 0, 4),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 1),
                word(EMIT, 0, 1),
            ],
            vec![b"captured".to_vec(), b"match".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let output = program.execute(&tape("name\n")).unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].captures.len(), 1);
        assert!(output[1].captures.is_empty());
    }

    #[test]
    fn text_predicate_variants_use_declared_semantics() {
        let tape = tape("execute\n");
        for (predicate, source) in [
            (TextPredicate::Prefix(0), b"exec".to_vec()),
            (
                TextPredicate::Regex {
                    source: 0,
                    compiled: Regex::new("^exec(?:ute)?$").unwrap(),
                },
                b"^exec(?:ute)?$".to_vec(),
            ),
        ] {
            let program = Program::new(
                vec![
                    word(
                        KIND,
                        0,
                        SupportLang::Python
                            .get_ts_language()
                            .id_for_node_kind("identifier", true),
                    ),
                    word(TEXT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![source, b"match".to_vec()],
                vec![predicate],
                vec![1],
                vec![],
            )
            .unwrap();
            assert_eq!(program.execute(&tape).unwrap().len(), 1);
        }
    }

    #[test]
    fn capture_predicates_compare_text_sets_regex_kinds_and_negation() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let program = Program::new_with_capture_predicates(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 1),
                word(ASSERT, 0, 0),
                word(ASSERT, 0, 1),
                word(ASSERT, 0, 2),
                word(ASSERT, 0, 3),
                word(EMIT, 0, 2),
            ],
            vec![
                b"left".to_vec(),
                b"right".to_vec(),
                b"match".to_vec(),
                b"^same$".to_vec(),
                b"same".to_vec(),
                b"other".to_vec(),
            ],
            vec![TextPredicate::Regex {
                source: 3,
                compiled: Regex::new("^same$").unwrap(),
            }],
            vec![
                CapturePredicate::Equal {
                    left: 0,
                    right: 1,
                    negated: false,
                    any: false,
                },
                CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                },
                CapturePredicate::AnyOf {
                    capture: 1,
                    strings: vec![4, 5],
                    negated: false,
                    any: false,
                },
                CapturePredicate::Kind {
                    capture: 1,
                    kind: identifier,
                    negated: true,
                    any: false,
                },
            ],
            vec![0, 1, 2],
            vec![],
        )
        .unwrap();
        assert!(program.execute(&tape("same = same\n")).unwrap().is_empty());

        let mut predicates = program.capture_predicates.clone();
        predicates[3] = CapturePredicate::Kind {
            capture: 1,
            kind: identifier,
            negated: false,
            any: false,
        };
        let program = program.with_capture_predicates(predicates).unwrap();
        assert_eq!(program.execute(&tape("same = same\n")).unwrap().len(), 1);
        assert!(program.execute(&tape("same = other\n")).unwrap().is_empty());
    }

    #[test]
    fn quantified_capture_predicates_apply_negation_per_capture() {
        let identifier = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("identifier", true);
        let program = Program::new_with_capture_predicates(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"item".to_vec(), b"match".to_vec(), b"same".to_vec()],
            vec![],
            vec![CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![2],
                negated: true,
                any: false,
            }],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        assert!(program.execute(&tape("same = other\n")).unwrap().is_empty());
        assert_eq!(program.execute(&tape("left = right\n")).unwrap().len(), 1);
    }

    #[test]
    fn positional_and_negated_field_tests_filter_nodes() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let alternative = grammar.field_id_for_name("alternative").unwrap().get();
        let positions = Program::new(
            vec![
                word(KIND, 0, identifier),
                word(FIRST, NAMED, 0),
                word(LAST, NAMED, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_eq!(positions.execute(&tape("only\n")).unwrap().len(), 1);
        assert!(
            positions
                .execute(&tape("left + right\n"))
                .unwrap()
                .is_empty()
        );

        let no_else = Program::new(
            vec![
                word(KIND, 0, grammar.id_for_node_kind("if_statement", true)),
                word(ABSENT_FIELD, 0, alternative),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap();
        assert_eq!(no_else.execute(&tape("if ok:\n  pass\n")).unwrap().len(), 1);
        assert!(
            no_else
                .execute(&tape("if ok:\n  pass\nelse:\n  pass\n"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn traversal_flags_sizes_and_long_table_execute() {
        let grammar = SupportLang::Python.get_ts_language();
        let identifier = grammar.id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![
                word(DESCEND, NAMED, 0),
                word(KIND, 0, identifier),
                word(FLAGS, 1, 0b1110),
                word(SIZE, 0x82, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, NAMED, 0),
                word(PARENT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"identifier".to_vec(), b"match".to_vec()],
            vec![],
            vec![0, 1],
            vec![100_000],
        )
        .unwrap();
        // Every identifier is shorter than the long constant. Only identifiers with a later
        // named sibling survive NEXT, and PARENT returns their shared syntax parent.
        let output = program.execute(&tape("pair = (alpha, beta)\n")).unwrap();
        assert!(!output.is_empty());
        assert!(
            output
                .iter()
                .all(|item| item.captures[0].span.len() <= 100_000)
        );
    }

    #[test]
    fn backward_goto_requires_and_uses_monotonic_next() {
        let identifier = SupportLang::Python
            .get_ts_language()
            .id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![
                word(KIND, 0, identifier),
                word(CAPTURE, 0, 0),
                word(FORK, 0, 5),
                word(NEXT, NAMED, 0),
                word(GOTO, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"item".to_vec(), b"sequence".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap();
        let tape = tape("f(a, b, c)\n");
        let root = (0..tape.transducer_len() as u32)
            .find(|&node| tape.transducer_text(node) == b"a")
            .unwrap();
        let output = program.execute_roots(&tape, [root]).unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(output[0].captures.len(), 3);
        assert_eq!(output[1].captures.len(), 2);
        assert_eq!(output[2].captures.len(), 1);
    }

    #[test]
    fn batch_kind_dispatch_matches_individual_execution() {
        let tape = tape("execute(); commit(); execute()\n");
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let individual = programs
            .iter()
            .map(|program| program.execute(&tape).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(execute_many(&programs, &tape).unwrap(), individual);
    }

    #[test]
    fn bounded_program_batches_preserve_global_task_and_local_ordinals() {
        let files = [tape("execute(); commit()\n"), tape("commit(); execute()\n")];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let kind = split(program.words[0]).2;
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let expected =
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap();
        assert_eq!(
            execute_with_backend_bounded_limit(
                ExecutionBackend::CpuJit,
                &programs,
                &files,
                &anchors,
                1,
            )
            .unwrap()
            .into_result()
            .unwrap(),
            expected
        );
    }

    #[test]
    fn batch_runs_many_programs_files_and_exact_anchor_sets() {
        let files = [tape("execute(); commit()\n"), tape("commit(); execute()\n")];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let unanchored = execute_batch(&programs, &files, None).unwrap();
        assert_eq!(unanchored.len(), 2);
        assert!(
            unanchored
                .iter()
                .all(|file| file.iter().all(|matches| matches.len() == 1))
        );
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let (_, _, kind) = split(program.words[0]);
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let semantic = |mut batch: Vec<Vec<Vec<Output>>>| {
            for output in batch.iter_mut().flatten().flatten() {
                output.task_ordinal = 0;
                output.local_ordinal = 0;
            }
            batch
        };
        assert_eq!(
            semantic(execute_batch(&programs, &files, Some(&anchors)).unwrap()),
            semantic(unanchored)
        );
        let empty = vec![vec![Vec::new(), Vec::new()], vec![Vec::new(), Vec::new()]];
        assert!(
            execute_batch(&programs, &files, Some(&empty))
                .unwrap()
                .iter()
                .flatten()
                .all(Vec::is_empty)
        );
        let invalid = vec![
            vec![vec![u32::MAX], Vec::new()],
            vec![Vec::new(), Vec::new()],
        ];
        assert_eq!(
            execute_batch(&programs, &files, Some(&invalid)),
            Err(Error::InvalidAnchor)
        );
    }

#[test]
    fn query_independent_fts_feeds_batched_execution_and_scopes_ordinals_to_tapes() {
        let database = std::env::temp_dir().join(format!(
            "tempo-transducer-anchor-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let files = [
            tape("execute(); commit(); execute()\n"),
            tape("commit(); execute()\n"),
        ];
        let programs = [identifier_program("execute"), identifier_program("absent")];
        let source = store.put_source(b"source", &[]).unwrap();
        let tape_ids = [
            store.put_tape(source, 1, 1, 1, b"first").unwrap(),
            store.put_tape(source, 2, 1, 1, b"second").unwrap(),
        ];
        persist_search_index_batch(&store, &tape_ids, &files).unwrap();
        let identifier_kind = split(programs[0].words[0]).2;
        let kind_roots = load_kind_anchor_batch(&store, &tape_ids, &[identifier_kind]).unwrap();
        assert!(
            kind_roots
                .iter()
                .zip(&files)
                .all(|(roots, tape)| roots[0].as_ref().is_some_and(|roots| roots
                    .iter()
                    .all(|&root| tape.transducer_kind(root) == identifier_kind)))
        );
        let loaded = load_literal_anchor_batch(&store, &tape_ids, &programs).unwrap();
        assert!(
            loaded
                .iter()
                .all(|file| file[0].is_none() && file[1] == Some(Vec::new()))
        );
        let anchors = loaded
            .into_iter()
            .zip(&files)
            .map(|(file, tape)| {
                file.into_iter()
                    .map(|roots| {
                        roots.unwrap_or_else(|| (0..tape.transducer_len() as u32).collect())
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let semantic = |mut batch: Vec<Vec<Vec<Output>>>| {
            for output in batch.iter_mut().flatten().flatten() {
                output.task_ordinal = 0;
                output.local_ordinal = 0;
            }
            batch
        };
        assert_eq!(
            semantic(execute_batch(&programs, &files, Some(&anchors)).unwrap()),
            semantic(execute_batch(&programs, &files, None).unwrap())
        );

        let stale = store
            .put_tape(source, 3, 1, 1, b"changed revision")
            .unwrap();
        assert_eq!(
            load_literal_anchor_batch(&store, &[stale], &programs).unwrap(),
            vec![vec![None, None]]
        );
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

#[test]
    fn sqlite_virtual_table_drives_batched_matching_and_relational_composition() {
        let files = [
            tape("execute(); commit(); execute()\n"),
            tape("commit(); execute()\n"),
        ];
        let programs = [identifier_program("execute"), identifier_program("absent")];
        let database = std::env::temp_dir().join(format!(
            "tempo-sql-indexed-match-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let source = store.put_source(b"source", &[]).unwrap();
        let tape_ids = [
            store.put_tape(source, 1, 1, 1, b"first").unwrap(),
            store.put_tape(source, 2, 1, 1, b"second").unwrap(),
        ];
        persist_search_index_batch(&store, &tape_ids, &files).unwrap();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let registry = Arc::new(sql_backend::MatchRegistry::default());
        sql_backend::register_module(&connection, Arc::clone(&registry)).unwrap();
        let batch = registry
            .register_indexed_batch(&store, &tape_ids, &programs, &files)
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE expectations(program_index INTEGER PRIMARY KEY,min_matches INTEGER NOT NULL);\
                 INSERT INTO expectations VALUES(0,2),(1,1);",
            )
            .unwrap();
        let selected = connection
            .prepare(
                "WITH terminal AS MATERIALIZED (SELECT * FROM tempo_matches(?1)) \
                 SELECT e.program_index,count(DISTINCT CASE WHEN t.match_index IS NOT NULL \
                   THEN printf('%d:%d',t.file_index,t.match_index) END) matches \
                 FROM expectations e LEFT JOIN terminal t ON t.program_index=e.program_index \
                   AND t.complete=1 AND t.match_index IS NOT NULL \
                 GROUP BY e.program_index,e.min_matches \
                 HAVING count(DISTINCT CASE WHEN t.match_index IS NOT NULL \
                   THEN printf('%d:%d',t.file_index,t.match_index) END)>=e.min_matches \
                 ORDER BY e.program_index",
            )
            .unwrap()
            .query_map([batch], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(selected, vec![(0, 3)]);
        let negatives: i64 = connection
            .query_row(
                "SELECT count(*) FROM tempo_matches(?1) WHERE program_index=1 \
                 AND complete=1 AND match_index IS NULL AND error_code IS NULL",
                [batch],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(negatives, 2);
        assert_eq!(registry.execution_count(batch), Some(1));
        registry.remove_batch(batch);
        #[cfg(target_os = "macos")]
        {
            let anchors = files
                .iter()
                .map(|file| {
                    programs
                        .iter()
                        .map(|_| (0..file.transducer_len() as u32).collect())
                        .collect()
                })
                .collect::<Vec<Vec<Vec<u32>>>>();
            {
                let metal_batch = registry
                    .register_metal_batch(&programs, &files, &anchors)
                    .unwrap();

                let (matches, failures): (i64, i64) = connection
                    .query_row(
                        "SELECT count(DISTINCT CASE WHEN match_index IS NOT NULL THEN printf('%d:%d:%d',file_index,program_index,match_index) END),count(DISTINCT error_code) FROM tempo_matches(?1)",
                        [metal_batch],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(matches, 3);
                assert_eq!(failures, 0);
                assert_eq!(registry.execution_count(metal_batch), Some(1));

                registry.remove_batch(metal_batch);
            }
        }
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn cpu_jit_and_metal_anchor_scan_match_anchored_interpreter() {
        let files = [
            tape("execute(); commit(); execute()\n"),
            tape("commit(); execute()\n"),
        ];
        let programs = [identifier_program("execute"), identifier_program("commit")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|program| {
                        let kind = split(program.words[0]).2;
                        (0..file.transducer_len() as u32)
                            .filter(|&node| file.transducer_kind(node) == kind)
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap(),
            expected
        );
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        let registry = Arc::new(sql_backend::MatchRegistry::default());
        sql_backend::register_module(&connection, Arc::clone(&registry)).unwrap();
        let expected_matches = expected.iter().flatten().map(Vec::len).sum::<usize>() as i64;
        {
            let batch = registry
                .register_metal_batch(&programs, &files, &anchors)
                .unwrap();
            let matches: i64 = connection
                .query_row(
                    "SELECT count(*) FROM tempo_matches(?1) WHERE match_index IS NOT NULL",
                    [batch],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(matches, expected_matches);
            registry.remove_batch(batch);
        }
        let anchor_scan = metal_backend::prepare_anchor_scan(&programs, &files).unwrap();
        // This thread's compilations: the cache is the process's, and other tests fill it at the same time.
        let anchor_pipeline_count = metal_backend::pipelines_compiled_here();
        let gpu_anchors = anchor_scan.run().unwrap();
        assert_eq!(anchor_scan.run().unwrap(), gpu_anchors);
        let semantic = |mut batch: Vec<Vec<Vec<Output>>>| {
            for output in batch.iter_mut().flatten().flatten() {
                output.task_ordinal = 0;
                output.local_ordinal = 0;
            }
            batch
        };
        assert_eq!(
            semantic(execute_batch(&programs, &files, Some(&gpu_anchors)).unwrap()),
            semantic(expected.clone())
        );
        assert_eq!(
            semantic(materialize_literal_anchors(&programs, &files, &gpu_anchors).unwrap()),
            semantic(expected)
        );
        let filtered_scan = metal_backend::prepare_anchor_scan_with_eligibility(
            &programs,
            &files,
            Some(&[vec![0b01], vec![0b10]]),
        )
        .unwrap();
        assert_eq!(metal_backend::pipelines_compiled_here(), anchor_pipeline_count);
        let filtered = filtered_scan.run().unwrap();
        assert_eq!(filtered[0][0], gpu_anchors[0][0]);
        assert!(filtered[0][1].is_empty());
        assert!(filtered[1][0].is_empty());
        assert_eq!(filtered[1][1], gpu_anchors[1][1]);

        let many_programs = (0..40)
            .map(|_| identifier_program("execute"))
            .collect::<Vec<_>>();
        let many_masks = vec![vec![1, 1 << 7], vec![1, 1 << 7]];
        let many = metal_backend::prepare_anchor_scan_with_eligibility(
            &many_programs,
            &files,
            Some(&many_masks),
        )
        .unwrap()
        .run()
        .unwrap();
        for file in 0..files.len() {
            assert_eq!(many[file][0], gpu_anchors[file][0]);
            assert_eq!(many[file][39], gpu_anchors[file][0]);
            assert!(many[file][1..39].iter().all(Vec::is_empty));
        }
    }

    #[test]
    fn verifier_rejects_invalid_and_non_progressing_programs() {
        assert!(matches!(
            Program::new(vec![word(GOTO, 0, 0)], vec![], vec![], vec![], vec![]),
            Err(Error::NonProgressingCycle(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(TEXT, 0, 0), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidOperand(0))
        ));
        assert!(matches!(
            Program::new(
                vec![word(CHILD, 2, 0), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidMode(0))
        ));
        assert!(
            Program::new(
                vec![word(FORK, GREEDY, 1), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            )
            .is_ok()
        );
        assert!(matches!(
            Program::new(
                vec![word(FORK, 2, 1), word(STOP, 0, 0)],
                vec![],
                vec![],
                vec![],
                vec![]
            ),
            Err(Error::InvalidMode(0))
        ));
    }

    fn split_tree_sitter_patterns(text: &str) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut patterns = Vec::new();
        let mut depth = 0_i32;
        let mut start = None;
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b';' => {
                    index += 1;
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                    continue;
                }
                b'"' => {
                    if depth == 0
                        && let Some(start) = start.replace(index)
                    {
                        patterns.push(text[start..index].trim().to_owned());
                    }
                    index += 1;
                    while index < bytes.len() && bytes[index] != b'"' {
                        if bytes[index] == b'\\' {
                            index += 1;
                        }
                        index += 1;
                    }
                }
                b'(' | b'[' => {
                    if depth == 0
                        && let Some(start) = start.replace(index)
                    {
                        patterns.push(text[start..index].trim().to_owned());
                    }
                    depth += 1;
                }
                b')' | b']' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        if let Some(start) = start {
            patterns.push(text[start..].trim().to_owned());
        }
        patterns.retain(|pattern| !pattern.is_empty());
        patterns
    }

    fn collect_extension_files(root: &Path, extensions: &[&str], output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if path.file_name().is_none_or(|name| name != ".git") {
                    collect_extension_files(&path, extensions, output);
                }
            } else if file_type.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extensions.contains(&extension))
                && fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 4_000_000)
            {
                output.push(path);
            }
        }
    }

    fn collect_python(root: &Path, output: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !matches!(
                    path.file_name().and_then(|s| s.to_str()),
                    Some(".git" | ".venv" | "node_modules" | "__pycache__")
                ) {
                    collect_python(&path, output);
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("py") {
                output.push(path);
            }
        }
    }

    fn digest_ranges<'a>(ranges: impl IntoIterator<Item = &'a Range<usize>>) -> String {
        let mut digest = Sha256::new();
        for range in ranges {
            digest.update((range.start as u64).to_le_bytes());
            digest.update((range.end as u64).to_le_bytes());
        }
        crate::code_rule_dsl::prof::lower_hex(&digest.finalize())
    }

    fn median(mut values: Vec<Duration>) -> Duration {
        values.sort_unstable();
        values[values.len() / 2]
    }

    #[test]
    #[ignore = "controlled corpus benchmark"]
    fn benchmark_three_ast_grep_paths() {
        let root = PathBuf::from(
            std::env::var("TEMPO_TRANSDUCER_CORPUS")
                .unwrap_or_else(|_| "../../nera-data-layer".into()),
        );
        let limit = std::env::var("TEMPO_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("TEMPO_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let patterns = names
            .iter()
            .map(|name| Pattern::try_new(name, SupportLang::Python).unwrap())
            .collect::<Vec<_>>();
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(
            !paths.is_empty(),
            "benchmark corpus is empty: {}",
            root.display()
        );

        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let preparation = Instant::now();
        let mut native = Vec::with_capacity(paths.len());
        let mut tape_roots = Vec::with_capacity(paths.len());
        let mut tapes = Vec::with_capacity(paths.len());
        let mut source_bytes = 0usize;
        let mut parse_time = Duration::ZERO;
        let mut encode_restore_time = Duration::ZERO;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let started = Instant::now();
            let doc = StrDoc::new(&source, SupportLang::Python);
            parse_time += started.elapsed();
            let started = Instant::now();
            let image =
                TapeImage::encode(doc.tree.root_node(), &grammar, &source, &|| false).unwrap();
            let tape = TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                .unwrap();
            encode_restore_time += started.elapsed();
            native.push(AstGrep::doc(doc));
            tape_roots.push(AstGrep::doc(tape.clone()));
            tapes.push(tape);
        }
        let preparation_time = preparation.elapsed();

        let native_run = || {
            let mut ranges = Vec::new();
            for root in &native {
                for pattern in &patterns {
                    ranges.extend(root.root().find_all(pattern).map(|m| m.get_node().range()));
                }
            }
            ranges
        };
        let tape_run = || {
            let mut ranges = Vec::new();
            for root in &tape_roots {
                for pattern in &patterns {
                    ranges.extend(root.root().find_all(pattern).map(|m| m.get_node().range()));
                }
            }
            ranges
        };
        let transducer_run = || {
            let mut ranges = Vec::new();
            for tape in &tapes {
                for output in execute_many(&programs, tape).unwrap() {
                    ranges.extend(output.into_iter().map(|m| m.captures[0].span.clone()));
                }
            }
            ranges
        };

        let expected = native_run();
        let tape_result = tape_run();
        let transducer_result = transducer_run();
        assert_eq!(tape_result, expected, "tape ast-grep parity failed");
        assert_eq!(transducer_result, expected, "transducer parity failed");
        let digest = digest_ranges(&expected);

        let mut native_times = Vec::new();
        let mut tape_times = Vec::new();
        let mut transducer_times = Vec::new();
        for iteration in 0..=iterations {
            let order = if iteration % 2 == 0 {
                [0, 1, 2]
            } else {
                [2, 1, 0]
            };
            for engine in order {
                let started = Instant::now();
                let count = match engine {
                    0 => native_run().len(),
                    1 => tape_run().len(),
                    _ => transducer_run().len(),
                };
                black_box(count);
                let elapsed = started.elapsed();
                if iteration > 0 {
                    match engine {
                        0 => native_times.push(elapsed),
                        1 => tape_times.push(elapsed),
                        _ => transducer_times.push(elapsed),
                    }
                }
            }
        }
        let native_median = median(native_times);
        let tape_median = median(tape_times);
        let transducer_median = median(transducer_times);
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let report = json!({
            "benchmark": "tempo-fixed-word-transducer-v1",
            "corpus": fs::canonicalize(&root).unwrap(),
            "files": paths.len(), "sourceBytes": source_bytes, "patterns": names,
            "matches": expected.len(), "orderedRangeSha256": digest,
            "iterations": iterations,
            "preparationMs": ms(preparation_time), "parseMs": ms(parse_time),
            "tapeEncodeRestoreMs": ms(encode_restore_time),
            "medianMs": {
                "astGrepTree": ms(native_median),
                "astGrepTape": ms(tape_median),
                "transducerTape": ms(transducer_median),
            },
            "speedupVsTree": ms(native_median) / ms(transducer_median),
            "speedupVsTapeAstGrep": ms(tape_median) / ms(transducer_median),
        });
        println!("TRANSDUCER_BENCHMARK={report}");
    }

#[test]
    #[ignore = "full ast-index-spike source corpus indexing benchmark"]
    fn benchmark_full_fts_source_corpus() {
        let corpus = PathBuf::from(
            std::env::var("TEMPO_SOURCE_CORPUS")
                .expect("TEMPO_SOURCE_CORPUS must name ast-index-spike bench/corpus/repos"),
        );
        let database = std::env::temp_dir().join(format!(
            "tempo-full-fts-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-16384; \
                 CREATE TABLE search_index_sets(tape_id INTEGER PRIMARY KEY,index_version INTEGER,node_count INTEGER,complete INTEGER); \
                 CREATE VIRTUAL TABLE search_kind_fts USING fts5(token,content='',contentless_delete=1,detail=full,tokenize='ascii'); \
                 CREATE VIRTUAL TABLE search_value_fts USING fts5(token,content='',contentless_delete=1,detail=none,tokenize='ascii'); \
                 CREATE VIRTUAL TABLE search_text_fts USING fts5(token,content='',contentless_delete=1,detail=none,tokenize='unicode61 remove_diacritics 0');",
            )
            .unwrap();
        let languages: &[(&str, SupportLang, &[&str])] = &[
            ("python", SupportLang::Python, &["py"]),
            (
                "javascript",
                SupportLang::JavaScript,
                &["js", "mjs", "cjs", "jsx"],
            ),
            ("typescript", SupportLang::TypeScript, &["ts"]),
            ("tsx", SupportLang::Tsx, &["tsx"]),
            ("java", SupportLang::Java, &["java"]),
            ("csharp", SupportLang::CSharp, &["cs"]),
            (
                "cpp",
                SupportLang::Cpp,
                &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            ),
            ("c", SupportLang::C, &["c", "h"]),
            ("php", SupportLang::Php, &["php"]),
            ("bash", SupportLang::Bash, &["sh", "bash", "zsh"]),
            ("ruby", SupportLang::Ruby, &["rb"]),
            ("go", SupportLang::Go, &["go"]),
            ("rust", SupportLang::Rust, &["rs"]),
            ("kotlin", SupportLang::Kotlin, &["kt", "kts"]),
            ("scala", SupportLang::Scala, &["scala", "sc"]),
            ("swift", SupportLang::Swift, &["swift"]),
            ("html", SupportLang::Html, &["html", "htm"]),
            ("css", SupportLang::Css, &["css"]),
            ("lua", SupportLang::Lua, &["lua"]),
            ("json", SupportLang::Json, &["json"]),
        ];
        let selected_languages = std::env::var("TEMPO_BENCH_LANGS")
            .ok()
            .map(|value| value.split(',').map(str::to_owned).collect::<BTreeSet<_>>());
        let started = Instant::now();
        let mut next_id = 1_i64;
        let mut total_files = 0usize;
        let mut total_bytes = 0usize;
        let mut total_nodes = 0usize;
        let mut failures = Vec::new();
        let mut per_language = serde_json::Map::new();
        for &(name, language, extensions) in languages {
            if selected_languages
                .as_ref()
                .is_some_and(|selected| !selected.contains(name))
            {
                continue;
            }
            let root = if name == "tsx" {
                corpus.join("typescript")
            } else {
                corpus.join(name)
            };
            let mut paths = Vec::new();
            collect_extension_files(&root, extensions, &mut paths);
            paths.sort();
            let grammar = match Grammar::new(language) {
                Ok(grammar) => grammar,
                Err(error) => {
                    failures.push(format!("{name}: {error}"));
                    continue;
                }
            };
            let language_started = Instant::now();
            let mut files = 0usize;
            let mut bytes = 0usize;
            let mut nodes = 0usize;
            for chunk in paths.chunks(64) {
                let transaction = connection.transaction().unwrap();
                for path in chunk {
                    let Ok(source) = fs::read_to_string(path) else {
                        continue;
                    };
                    if source.is_empty() {
                        continue;
                    }
                    let native = StrDoc::new(&source, language);
                    let image =
                        match TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| {
                            false
                        }) {
                            Ok(image) => image,
                            Err(error) => {
                                failures.push(format!("{}: {error}", path.display()));
                                continue;
                            }
                        };
                    let tape =
                        TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                            .unwrap();
                    let document = search_index_document(
                        crate::content_store::TapeId::for_benchmark(next_id),
                        &tape,
                    );
                    transaction
                        .execute(
                            "INSERT INTO search_kind_fts(rowid,token) VALUES(?1,?2)",
                            rusqlite::params![next_id, document.kind_tokens],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "INSERT INTO search_value_fts(rowid,token) VALUES(?1,?2)",
                            rusqlite::params![next_id, document.value_tokens],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "INSERT INTO search_text_fts(rowid,token) VALUES(?1,?2)",
                            rusqlite::params![next_id, document.source_text],
                        )
                        .unwrap();
                    transaction
                        .execute(
                            "INSERT INTO search_index_sets VALUES(?1,?2,?3,1)",
                            rusqlite::params![
                                next_id,
                                POSITIONAL_INDEX_VERSION,
                                document.node_count
                            ],
                        )
                        .unwrap();
                    next_id += 1;
                    files += 1;
                    bytes += tape.transducer_source().len();
                    nodes += tape.transducer_len();
                }
                transaction.commit().unwrap();
            }
            total_files += files;
            total_bytes += bytes;
            total_nodes += nodes;
            per_language.insert(
                name.to_owned(),
                json!({"files":files,"sourceBytes":bytes,"nodes":nodes,
                       "seconds":language_started.elapsed().as_secs_f64()}),
            );
        }
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut sizes = [0_i64; 4];
        let mut statement = connection
            .prepare(
                "SELECT CASE WHEN name LIKE 'search_kind_fts%' THEN 1 \
                             WHEN name LIKE 'search_value_fts%' THEN 2 \
                             WHEN name LIKE 'search_text_fts%' THEN 3 ELSE 0 END family, \
                        sum(pgsize) FROM dbstat WHERE name LIKE 'search_%' GROUP BY family",
            )
            .unwrap();
        for row in statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .unwrap()
        {
            let (family, bytes) = row.unwrap();
            sizes[family as usize] = bytes;
        }
        println!(
            "FULL_FTS_SOURCE_BENCHMARK={}",
            json!({"files":total_files,"sourceBytes":total_bytes,"nodes":total_nodes,
                   "seconds":started.elapsed().as_secs_f64(),"storageBytes":sizes.iter().sum::<i64>(),
                   "storageBytesPerNode":sizes.iter().sum::<i64>() as f64/total_nodes as f64,
                   "storageBytesPerSourceByte":sizes.iter().sum::<i64>() as f64/total_bytes as f64,
                   "storageBreakdownBytes":{"metadata":sizes[0],"kindFts":sizes[1],
                     "valueFts":sizes[2],"sourceWordFts":sizes[3]},
                   "languages":per_language,"failures":failures,
                   "unsupported":["sql: no ast-grep-language grammar"]})
        );
        drop(statement);
        drop(connection);
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

    #[test]
    #[ignore = "full ast-index-spike tree-sitter query oracle benchmark"]
    fn benchmark_tree_sitter_query_oracle() {
        use tree_sitter::StreamingIterator;

        let query_root = PathBuf::from(
            std::env::var("TEMPO_QUERY_CORPUS")
                .expect("TEMPO_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let source_root = PathBuf::from(
            std::env::var("TEMPO_SOURCE_CORPUS")
                .expect("TEMPO_SOURCE_CORPUS must name ast-index-spike bench/corpus/repos"),
        );
        let languages: &[(&str, SupportLang, &[&str])] = &[
            ("python", SupportLang::Python, &["py"]),
            (
                "javascript",
                SupportLang::JavaScript,
                &["js", "mjs", "cjs", "jsx"],
            ),
            ("typescript", SupportLang::TypeScript, &["ts"]),
            ("tsx", SupportLang::Tsx, &["tsx"]),
            ("java", SupportLang::Java, &["java"]),
            ("csharp", SupportLang::CSharp, &["cs"]),
            (
                "cpp",
                SupportLang::Cpp,
                &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            ),
            ("c", SupportLang::C, &["c", "h"]),
            ("php", SupportLang::Php, &["php"]),
            ("bash", SupportLang::Bash, &["sh", "bash", "zsh"]),
            ("ruby", SupportLang::Ruby, &["rb"]),
            ("go", SupportLang::Go, &["go"]),
            ("rust", SupportLang::Rust, &["rs"]),
            ("kotlin", SupportLang::Kotlin, &["kt", "kts"]),
            ("scala", SupportLang::Scala, &["scala", "sc"]),
            ("swift", SupportLang::Swift, &["swift"]),
            ("html", SupportLang::Html, &["html", "htm"]),
            ("css", SupportLang::Css, &["css"]),
            ("lua", SupportLang::Lua, &["lua"]),
            ("json", SupportLang::Json, &["json"]),
        ];
        let selected_languages = std::env::var("TEMPO_QUERY_LANGS")
            .ok()
            .map(|value| value.split(',').map(str::to_owned).collect::<BTreeSet<_>>());
        let query_filter = std::env::var("TEMPO_QUERY_FILTER").ok();
        let source_limit = std::env::var("TEMPO_QUERY_SOURCE_BYTES")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1_000_000);
        let shard_count = std::env::var("TEMPO_QUERY_SHARD_COUNT")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(1);
        let shard_index = std::env::var("TEMPO_QUERY_SHARD_INDEX")
            .ok()
            .map(|value| value.parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(shard_count > 0 && shard_index < shard_count);
        let mut all_micros = Vec::new();
        let mut per_language = serde_json::Map::new();
        let mut total_matches = 0usize;
        let mut deadline_violations = 0usize;
        let mut slow_queries = Vec::new();
        for &(name, language, extensions) in languages {
            if selected_languages
                .as_ref()
                .is_some_and(|selected| !selected.contains(name))
            {
                continue;
            }
            let mut patterns = BTreeSet::new();
            let language_query_root = if name == "tsx" {
                query_root.join("typescript")
            } else {
                query_root.join(name)
            };
            for entry in fs::read_dir(language_query_root).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "scm") {
                    patterns.extend(split_tree_sitter_patterns(
                        &fs::read_to_string(path).unwrap(),
                    ));
                }
            }
            let grammar = language.get_ts_language();
            let queries = patterns
                .iter()
                .filter(|pattern| {
                    query_filter
                        .as_ref()
                        .is_none_or(|filter| pattern.contains(filter))
                })
                .filter_map(|pattern| {
                    tree_sitter::Query::new(&grammar, pattern)
                        .ok()
                        .map(|query| (pattern, query))
                })
                .collect::<Vec<_>>();
            let mut paths = Vec::new();
            let language_source_root = if name == "tsx" {
                source_root.join("typescript")
            } else {
                source_root.join(name)
            };
            collect_extension_files(&language_source_root, extensions, &mut paths);
            paths.sort();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar).unwrap();
            let mut documents = Vec::new();
            let mut source_bytes = 0usize;
            for (ordinal, path) in paths.into_iter().enumerate() {
                if ordinal % shard_count != shard_index {
                    continue;
                }
                let Ok(source) = fs::read(path) else { continue };
                if source.is_empty() || source_bytes + source.len() > source_limit {
                    continue;
                }
                if let Some(tree) = parser.parse(&source, None) {
                    source_bytes += source.len();
                    documents.push((source, tree));
                }
            }
            let language_started = Instant::now();
            let mut language_matches = 0usize;
            for (pattern, query) in &queries {
                let query_started = Instant::now();
                for (source, tree) in &documents {
                    let mut cursor = tree_sitter::QueryCursor::new();
                    let mut matches = cursor.matches(query, tree.root_node(), source.as_slice());
                    while let Some(found) = matches.next() {
                        language_matches += 1;
                        std::hint::black_box(found.captures().len());
                    }
                }
                let micros = query_started.elapsed().as_micros() as u64;
                if micros > 500_000 {
                    deadline_violations += 1;
                    slow_queries.push(json!({"language":name,"milliseconds":micros as f64/1_000.0,
                                            "pattern":pattern}));
                }
                all_micros.push(micros);
            }
            total_matches += language_matches;
            per_language.insert(
                name.to_owned(),
                json!({"queries":queries.len(),"files":documents.len(),"sourceBytes":source_bytes,
                       "matches":language_matches,"seconds":language_started.elapsed().as_secs_f64()}),
            );
        }
        all_micros.sort_unstable();
        let percentile = |numerator: usize, denominator: usize| {
            all_micros[(all_micros.len() - 1) * numerator / denominator] as f64 / 1_000.0
        };
        println!(
            "TREE_SITTER_QUERY_ORACLE_BENCHMARK={}",
            json!({"queries":all_micros.len(),"matches":total_matches,
                   "medianMs":percentile(1,2),"p95Ms":percentile(95,100),
                   "maxMs":all_micros.last().copied().unwrap_or_default() as f64/1_000.0,
                   "deadlineViolations":deadline_violations,"slowQueries":slow_queries,
                   "languages":per_language,
                   "unsupported":["SQL grammar","Tempo transducer compilation of .scm queries"]})
        );
    }

    #[test]
    #[ignore = "pinned Python SCM execution-signature profile"]
    fn profile_pinned_python_execution_signatures() {
        let query_root = PathBuf::from(
            std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR")
                .expect("TEMPO_PYTHON_SCM_QUERY_DIR must name pinned Python SCM queries"),
        );
        let grammar = SupportLang::Python.get_ts_language();
        let mut paths = fs::read_dir(query_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut signatures = BTreeMap::<String, usize>::new();
        let mut compiled_programs = Vec::new();
        let mut programs = 0_usize;
        for path in paths {
            let source = fs::read_to_string(path).unwrap();
            for pattern in Program::tree_sitter_top_level_patterns(&source).unwrap() {
                let program =
                    Program::compile_tree_sitter_query_pattern(&pattern, &grammar).unwrap();
                let opcodes = program
                    .words
                    .iter()
                    .map(|word| split(*word).0)
                    .collect::<Vec<_>>();
                let signature = format!(
                    "ops={opcodes:?};values={};captures={};outputs={}",
                    program.value_programs.len(),
                    program.capture_programs.len(),
                    program.output_program.instructions.len(),
                );
                *signatures.entry(signature).or_default() += 1;
                compiled_programs.push(program);
                programs += 1;
            }
        }
        let mut groups = signatures.into_iter().collect::<Vec<_>>();
        groups.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        println!(
            "TEMPO_PYTHON_EXECUTION_SIGNATURES {}",
            json!({"programs":programs,"signatures":groups.len(),"groups":groups})
        );
        assert_eq!(programs, 335);
        let grid_shapes = metal_grid::compile_program_shapes(&compiled_programs);
        let grid_signatures = grid_shapes
            .iter()
            .map(|shape| shape.signature)
            .collect::<BTreeSet<_>>();
        let grid_routes = grid_shapes
            .iter()
            .map(|shape| shape.route)
            .collect::<BTreeSet<_>>();
        let route_counts = grid_shapes
            .iter()
            .fold(BTreeMap::new(), |mut counts, shape| {
                *counts.entry(shape.route).or_insert(0_usize) += 1;
                counts
            });
        let structural_maxima = compiled_programs.iter().fold(
            execution_shape::ProgramRequirements::default(),
            |mut maxima, program| {
                let found = execution_shape::program_requirements(program);
                maxima.cursor_slots = maxima.cursor_slots.max(found.cursor_slots);
                maxima.capture_operations = maxima.capture_operations.max(found.capture_operations);
                maxima.child_sites = maxima.child_sites.max(found.child_sites);
                maxima.descendant_sites = maxima.descendant_sites.max(found.descendant_sites);
                maxima.following_sites = maxima.following_sites.max(found.following_sites);
                maxima.repeat_sites = maxima.repeat_sites.max(found.repeat_sites);
                maxima.instruction_count = maxima.instruction_count.max(found.instruction_count);
                maxima
            },
        );
        println!("TEMPO_PYTHON_EXECUTION_ROUTES {route_counts:?}");
        println!("TEMPO_PYTHON_STRUCTURAL_MAXIMA {structural_maxima:?}");
        // The spike counted opcode shapes. Resource classes can split those
        // shapes, but must never merge different opcode/ABI contracts.
        assert_eq!(groups.len(), 106);
        let mut contracts = BTreeMap::new();
        for (program, shape) in compiled_programs.iter().zip(&grid_shapes) {
            let contract = (
                program
                    .words
                    .iter()
                    .map(|word| split(*word).0)
                    .collect::<Vec<_>>(),
                program.value_programs.len(),
                program.capture_programs.len(),
                program.output_program.instructions.len(),
                shape.class,
                shape.route,
            );
            if let Some(previous) = contracts.insert(shape.signature, contract.clone()) {
                assert_eq!(previous, contract, "signature={}", shape.signature);
            }
        }
        println!(
            "TEMPO_PYTHON_GRID_SIGNATURES opcodeShapes={} resourceShapes={}",
            groups.len(),
            grid_signatures.len()
        );
        assert!(grid_routes.len() <= 10);
    }

#[test]
    #[ignore = "full ast-index-spike tree-sitter query corpus audit"]
    fn audit_tree_sitter_query_corpus() {
        let query_root = PathBuf::from(
            std::env::var("TEMPO_QUERY_CORPUS")
                .expect("TEMPO_QUERY_CORPUS must name ast-index-spike bench/corpus/queries"),
        );
        let languages = [
            ("python", SupportLang::Python),
            ("javascript", SupportLang::JavaScript),
            ("typescript", SupportLang::TypeScript),
            ("java", SupportLang::Java),
            ("csharp", SupportLang::CSharp),
            ("cpp", SupportLang::Cpp),
            ("c", SupportLang::C),
            ("php", SupportLang::Php),
            ("bash", SupportLang::Bash),
            ("ruby", SupportLang::Ruby),
            ("go", SupportLang::Go),
            ("rust", SupportLang::Rust),
            ("kotlin", SupportLang::Kotlin),
            ("scala", SupportLang::Scala),
            ("swift", SupportLang::Swift),
            ("html", SupportLang::Html),
            ("css", SupportLang::Css),
            ("lua", SupportLang::Lua),
            ("json", SupportLang::Json),
        ];
        let mut report = serde_json::Map::new();
        let mut corpus = Vec::new();
        let mut total = 0usize;
        for (name, language) in languages {
            let mut patterns = BTreeSet::new();
            for entry in fs::read_dir(query_root.join(name)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "scm") {
                    patterns.extend(split_tree_sitter_patterns(
                        &fs::read_to_string(path).unwrap(),
                    ));
                }
            }
            let grammar = language.get_ts_language();
            let mut valid = 0usize;
            let mut invalid = 0usize;
            let mut lowered = 0usize;
            let mut lowering_errors = BTreeMap::<String, usize>::new();
            for pattern in &patterns {
                match tree_sitter::Query::new(&grammar, pattern) {
                    Ok(query) if query.pattern_count() == 1 => {
                        valid += 1;
                        corpus.push(json!({"lang":name,"text":pattern}));
                        match Program::compile_tree_sitter_query_pattern(pattern, &grammar) {
                            Ok(_) => lowered += 1,
                            Err(error) => {
                                if std::env::var_os("TEMPO_QUERY_AUDIT_ERRORS").is_some() {
                                    eprintln!(
                                        "SCM_LOWERING_ERROR lang={name} error={error:?} pattern={pattern:?}"
                                    );
                                }
                                *lowering_errors.entry(format!("{error:?}")).or_default() += 1;
                            }
                        }
                    }
                    _ => invalid += 1,
                }
            }
            total += valid;
            report.insert(
                name.to_owned(),
                json!({"attempted": patterns.len(), "valid": valid, "invalid": invalid,
                       "lowered": lowered, "loweringErrors": lowering_errors,
                       "grammarSymbols": grammar.node_kind_count(),
                       "grammarFields": grammar.field_count()}),
            );
            if name == "typescript" {
                let tsx = SupportLang::Tsx.get_ts_language();
                let mut valid_tsx = 0usize;
                let mut lowered_tsx = 0usize;
                let mut errors = BTreeMap::<String, usize>::new();
                for pattern in &patterns {
                    if tree_sitter::Query::new(&tsx, pattern).is_err() {
                        continue;
                    }
                    valid_tsx += 1;
                    match Program::compile_tree_sitter_query_pattern(pattern, &tsx) {
                        Ok(_) => lowered_tsx += 1,
                        Err(error) => *errors.entry(format!("{error:?}")).or_default() += 1,
                    }
                }
                report.insert(
                    "tsx".to_owned(),
                    json!({"attempted": patterns.len(), "valid": valid_tsx,
                           "invalid": patterns.len() - valid_tsx, "lowered": lowered_tsx,
                           "loweringErrors": errors,
                           "grammarSymbols": tsx.node_kind_count(),
                           "grammarFields": tsx.field_count()}),
                );
            }
        }
        let mut sql_patterns = BTreeSet::new();
        for entry in fs::read_dir(query_root.join("sql")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "scm") {
                sql_patterns.extend(split_tree_sitter_patterns(
                    &fs::read_to_string(path).unwrap(),
                ));
            }
        }
        // G's TapeLanguage::Sql is W's SQL dialect.
        let sql = crate::code_rule_dsl::language::GrammarDialect::Sql.get_ts_language();
        let mut valid_sql = 0usize;
        let mut lowered_sql = 0usize;
        let mut sql_errors = BTreeMap::<String, usize>::new();
        for pattern in &sql_patterns {
            if tree_sitter::Query::new(&sql, pattern).is_err() {
                continue;
            }
            valid_sql += 1;
            match Program::compile_tree_sitter_query_pattern(pattern, &sql) {
                Ok(_) => lowered_sql += 1,
                Err(error) => *sql_errors.entry(format!("{error:?}")).or_default() += 1,
            }
        }
        report.insert(
            "sql".to_owned(),
            json!({"attempted": sql_patterns.len(), "valid": valid_sql,
                   "invalid": sql_patterns.len() - valid_sql, "lowered": lowered_sql,
                   "loweringErrors": sql_errors,
                   "grammarSymbols": sql.node_kind_count(), "grammarFields": sql.field_count()}),
        );
        if let Ok(path) = std::env::var("TEMPO_QUERY_CORPUS_OUTPUT") {
            fs::write(path, serde_json::to_vec_pretty(&corpus).unwrap()).unwrap();
        }
        println!(
            "TREE_SITTER_QUERY_AUDIT={}",
            json!({"validWithoutSql": total, "languages": report})
        );
    }

#[test]
    #[ignore = "controlled corpus FTS storage and lookup benchmark"]
    fn benchmark_fts_search_index() {
        let root = PathBuf::from(
            std::env::var("TEMPO_TRANSDUCER_CORPUS")
                .unwrap_or_else(|_| "../../nera-data-layer".into()),
        );
        let limit = std::env::var("TEMPO_TRANSDUCER_FILES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("TEMPO_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(!paths.is_empty());
        let database = std::env::temp_dir().join(format!(
            "tempo-fts-benchmark-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = crate::content_store::ContentStore::open(&database).unwrap();
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let mut tapes = Vec::with_capacity(paths.len());
        let mut tape_ids = Vec::with_capacity(paths.len());
        let mut source_bytes = 0usize;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let native = StrDoc::new(&source, SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
            // W's encode yields a FileImage; G's tape bytes are its primary's.
            let tape_bytes = image.primary.to_bytes().unwrap();
            let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
            tape_ids.push(store.put_tape(source_id, 1, 1, 2, &tape_bytes).unwrap());
            tapes.push(
                TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                    .unwrap(),
            );
        }
        let nodes = tapes.iter().map(TapeDoc::transducer_len).sum::<usize>();
        let started = Instant::now();
        persist_search_index_batch(&store, &tape_ids, &tapes).unwrap();
        let build = started.elapsed();
        let (storage_bytes, storage_parts) = store.search_index_storage_bytes().unwrap();
        let expected = execute_batch(&programs, &tapes, None).unwrap();
        let mut lookup_times = Vec::new();
        let mut selection_times = Vec::new();
        let mut execution_times = Vec::new();
        for _ in 0..iterations {
            let started = Instant::now();
            let loaded = load_literal_anchor_batch(&store, &tape_ids, &programs).unwrap();
            lookup_times.push(started.elapsed());
            let anchors = widen_anchor_candidates(&programs, &tapes, loaded);
            selection_times.push(started.elapsed());
            let started = Instant::now();
            assert_eq!(
                execute_batch(&programs, &tapes, Some(&anchors)).unwrap(),
                expected
            );
            execution_times.push(started.elapsed());
        }
        let started = Instant::now();
        let text_candidates =
            load_text_candidate_batch(&store, &tape_ids, &["TODO".to_owned()]).unwrap();
        let text_lookup = started.elapsed();
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        #[allow(unused_mut)]
        let mut report = json!({
            "benchmark": "tempo-contentless-fts-search-index-v1",
            "corpus": fs::canonicalize(&root).unwrap(),
            "files": paths.len(),
            "sourceBytes": source_bytes,
            "nodes": nodes,
            "patterns": names,
            "iterations": iterations,
            "buildMs": ms(build),
            "storageBytes": storage_bytes,
            "storageBreakdownBytes": {
                "metadata": storage_parts[0],
                "kindFts": storage_parts[1],
                "valueFts": storage_parts[2],
                "sourceWordFts": storage_parts[3],
            },
            "storageBytesPerNode": storage_bytes as f64 / nodes as f64,
            "storageBytesPerSourceByte": storage_bytes as f64 / source_bytes as f64,
            "medianSqlLookupMs": ms(median(lookup_times)),
            "medianSelectionMs": ms(median(selection_times)),
            "medianTerminalMs": ms(median(execution_times)),
            "textWordLookupMs": ms(text_lookup),
            "textWordCandidateFiles": text_candidates.iter().filter(|value| **value == Some(true)).count(),
        });
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let started = Instant::now();
            let eligibility = load_literal_eligibility_batch(&store, &tape_ids, &programs).unwrap();
            let masks = eligibility_masks(eligibility, programs.len());
            let sql_and_masks = started.elapsed();
            let started = Instant::now();
            let scan = metal_backend::prepare_anchor_scan_with_eligibility(
                &programs,
                &tapes,
                Some(&masks),
            )
            .unwrap();
            let prepare = started.elapsed();
            let mut fused = Vec::new();
            for _ in 0..iterations {
                let started = Instant::now();
                let anchors = scan.run().unwrap();
                assert_eq!(
                    materialize_literal_anchors(&programs, &tapes, &anchors).unwrap(),
                    expected
                );
                fused.push(started.elapsed());
            }
            report["metal"] = json!({
                "sqlAndEligibilityMasksMs": ms(sql_and_masks),
                "prepareMs": ms(prepare),
                "warmFusedTerminalMedianMs": ms(median(fused)),
            });
        }
        println!("FTS_SEARCH_INDEX_BENCHMARK={report}");
        drop(store);
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", database.display()));
        }
    }

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "Apple Silicon backend benchmark"]
    fn benchmark_anchored_cpu_and_gpu_anchor_scan() {
        let root = PathBuf::from(
            std::env::var("TEMPO_TRANSDUCER_CORPUS")
                .unwrap_or_else(|_| "../../nera-data-layer".into()),
        );
        let limit = std::env::var("TEMPO_TRANSDUCER_FILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        let iterations = std::env::var("TEMPO_TRANSDUCER_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7usize);
        let names = [
            "execute", "commit", "rollback", "flush", "select", "where", "join", "delete",
        ];
        let programs = names
            .iter()
            .map(|name| identifier_program(name))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        collect_python(&root, &mut paths);
        paths.sort();
        paths.truncate(limit);
        assert!(!paths.is_empty());
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let mut tapes = Vec::with_capacity(paths.len());
        let mut source_bytes = 0;
        for path in &paths {
            let source = fs::read_to_string(path).unwrap();
            source_bytes += source.len();
            let native = StrDoc::new(&source, SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
            tapes.push(
                TapeDoc::restore(Arc::new(source), Arc::clone(&grammar), &image, &|| false)
                    .unwrap(),
            );
        }
        let anchor_started = Instant::now();
        let anchors = tapes
            .iter()
            .map(|tape| {
                programs
                    .iter()
                    .map(|program| {
                        let kind = split(program.words[0]).2;
                        let predicate = split(program.words[1]).2;
                        let TextPredicate::Equal(string) =
                            program.predicates[usize::from(predicate)]
                        else {
                            unreachable!()
                        };
                        let expected = &program.strings[usize::from(string)];
                        (0..tape.transducer_len() as u32)
                            .filter(|&node| {
                                tape.transducer_kind(node) == kind
                                    && tape.transducer_text(node) == expected
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect::<Vec<Vec<Vec<u32>>>>();
        let anchor_time = anchor_started.elapsed();
        let started = Instant::now();
        let _cpu_jit = programs
            .iter()
            .map(|program| {
                let native = native_ir::lower(program).unwrap();
                native_cpu::compile_straight_line(&native).unwrap()
            })
            .collect::<Vec<_>>();
        let cpu_compile = started.elapsed();
        let started = Instant::now();
        let gpu_anchor_scan = metal_backend::prepare_anchor_scan(&programs, &tapes).unwrap();
        let gpu_anchor_prepare = started.elapsed();
        let gpu_anchors = gpu_anchor_scan.run().unwrap();
        assert_eq!(
            gpu_anchors, anchors,
            "GPU anchor scan changed preorder postings"
        );
        let unanchored_run = || execute_batch(&programs, &tapes, None).unwrap();
        let interpreted_run = || execute_batch(&programs, &tapes, Some(&anchors)).unwrap();
        let cpu_jit_run =
            || execute_with_backend(ExecutionBackend::CpuJit, &programs, &tapes, &anchors).unwrap();
        let ranges = |result: Vec<Vec<Vec<Output>>>| {
            result
                .into_iter()
                .flatten()
                .flatten()
                .map(|m| m.captures[0].span.clone())
                .collect::<Vec<_>>()
        };
        let expected = ranges(interpreted_run());
        assert_eq!(ranges(unanchored_run()), expected);
        assert_eq!(ranges(cpu_jit_run()), expected);
        let mut times: [Vec<Duration>; 4] = std::array::from_fn(|_| Vec::new());
        for iteration in 0..=iterations {
            for offset in 0..4 {
                let engine = (iteration + offset) % 4;
                let started = Instant::now();
                let count = match engine {
                    0 => ranges(unanchored_run()).len(),
                    1 => ranges(interpreted_run()).len(),
                    2 => ranges(cpu_jit_run()).len(),
                    _ => gpu_anchor_scan
                        .run()
                        .unwrap()
                        .iter()
                        .flatten()
                        .map(Vec::len)
                        .sum(),
                };
                black_box(count);
                if iteration > 0 {
                    times[engine].push(started.elapsed());
                }
            }
        }
        let medians = times.map(median);
        let mut gpu_anchor_dispatch = Vec::new();
        let mut gpu_anchor_reconstruction = Vec::new();
        let mut gpu_anchor_hits = 0;
        for _ in 0..iterations {
            let (profiled, profile) = gpu_anchor_scan.run_profiled().unwrap();
            assert_eq!(profiled, anchors);
            gpu_anchor_dispatch.push(profile.dispatch_and_wait);
            gpu_anchor_reconstruction.push(profile.reconstruction);
            gpu_anchor_hits = profile.hit_count;
        }
        let gpu_anchor_dispatch = median(gpu_anchor_dispatch);
        let gpu_anchor_reconstruction = median(gpu_anchor_reconstruction);
        let mut gpu_fused_terminal = Vec::new();
        for _ in 0..iterations {
            let started = Instant::now();
            let selected = gpu_anchor_scan.run().unwrap();
            let output = materialize_literal_anchors(&programs, &tapes, &selected).unwrap();
            assert_eq!(ranges(output), expected);
            gpu_fused_terminal.push(started.elapsed());
        }
        let gpu_fused_terminal = median(gpu_fused_terminal);
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let report = json!({"benchmark":"tempo-anchored-transducer-and-anchor-scan-v5","corpus":fs::canonicalize(&root).unwrap(),"files":paths.len(),"sourceBytes":source_bytes,"patterns":names,"matches":expected.len(),"orderedRangeSha256":digest_ranges(&expected),"iterations":iterations,"cpuAnchorBuildMs":ms(anchor_time),"prepareMs":{"gpuAnchorScan":ms(gpu_anchor_prepare)},"compileMs":{"cpuJit":ms(cpu_compile)},"medianMs":{"cpuUnanchored":ms(medians[0]),"cpuInterpreted":ms(medians[1]),"cpuJit":ms(medians[2]),"gpuAnchorScanWarm":ms(medians[3])},"gpuAnchorProfile":{"dispatchAndWaitMs":ms(gpu_anchor_dispatch),"reconstructionMs":ms(gpu_anchor_reconstruction),"fusedTerminalMs":ms(gpu_fused_terminal),"sparseHitCount":gpu_anchor_hits},"anchorSpeedup":ms(medians[0])/ms(medians[1]),"speedupVsCpuInterpreted":{"cpuJit":ms(medians[1])/ms(medians[2])}});
        println!("TRANSDUCER_BACKEND_BENCHMARK={report}");
    }
