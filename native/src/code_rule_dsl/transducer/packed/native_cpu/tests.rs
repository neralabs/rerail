    use super::*;
    use crate::code_rule_dsl::{
        tape::{Grammar, TapeImage},
        transducer::packed::{CAPTURE, EMIT, KIND, Program, native_ir, word},
    };
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
    use ast_grep_core::tree_sitter::LanguageExt;
    use std::sync::Arc;

    #[test]
    fn demand_zero_cpu_workspace_has_typed_zeroed_storage() {
        let mut values = DemandZeroRegion::<u32>::new(17).unwrap();
        assert_eq!(&*values, &[0; 17]);
        values[8] = 42;
        assert_eq!(values[8], 42);
        assert!(DemandZeroRegion::<u32>::new(0).unwrap().is_empty());
    }

    fn tape(source: &str) -> TapeDoc {
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source, SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
    }

    #[test]
    fn structural_interpreter_publication_preserves_raw_results() {
        let files = [tape(
            "# first\n# second\n# third\ndef example(value: tuple[int, None]):\n    return value\n",
        )];
        for query in [
            "(comment)+ @comment.around",
            "(identifier) @identifier",
            "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))",
        ] {
            let programs = [Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap()];
            let kinds = programs[0].leading_kinds().unwrap();
            let anchors = vec![vec![
                (0..files[0].transducer_len() as u32)
                    .filter(|&node| kinds.contains(&files[0].transducer_kind(node)))
                    .collect::<Vec<_>>(),
            ]];
            let expected = super::execute_batch(&programs, &files, &anchors).unwrap();
            let actual = crate::code_rule_dsl::transducer::packed::interpreter_reference(
                &programs,
                &files[0],
                &anchors[0],
                &[u32::MAX],
            )
            .unwrap()
            .into_outcome()
            .unwrap();
            assert_eq!(actual.outputs, expected, "{query}");
            assert_eq!(actual.errors, vec![vec![None]]);
        }
    }

    #[test]
    fn interpreter_publication_preserves_every_output_directive_table() {
        let files = [tape("ALPHA\n'application/json'\n")];
        let queries = [
            r#"((identifier) @inner @outer (#offset! @inner 0 1 0 -1) (#downcase! @outer) (#set! role "trimmed") (#set! flag) (#set! @outer kind "raw"))"#,
            "((string (string_content) @mime) (#set-lang-from-mimetype! @mime))",
        ];
        for (index, query) in queries.iter().enumerate() {
            let program = Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap();
            let roots = (0..files[0].transducer_len() as u32).collect::<Vec<_>>();
            let anchors = vec![vec![roots.clone()]];
            let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
                std::slice::from_ref(&program),
                &files,
                Some(&anchors),
            )
            .unwrap();
            assert_eq!(expected[0][0].len(), 1, "{query}");
            let row = &expected[0][0][0];
            let resolve = |(key, value): &(u16, Option<u16>)| {
                (
                    program.strings[usize::from(*key)].clone(),
                    value.map(|value| program.strings[usize::from(value)].clone()),
                )
            };
            if index == 0 {
                assert_eq!(row.captures.len(), 2);
                assert_eq!(
                    row.captures
                        .iter()
                        .map(|capture| program.label_name(capture.label).unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    ["inner".to_owned(), "outer".to_owned()]
                );
                assert_eq!(row.captures[0].span, 1..4);
                assert_eq!(row.captures[1].span, 0..5);
                assert_eq!(row.captures[0].text, None);
                assert_eq!(row.captures[1].text.as_deref(), Some(b"alpha".as_slice()));
                assert_eq!(
                    row.properties.iter().map(&resolve).collect::<Vec<_>>(),
                    [
                        (b"role".to_vec(), Some(b"trimmed".to_vec())),
                        (b"flag".to_vec(), None),
                    ]
                );
                assert!(row.captures[0].properties.is_empty());
                assert_eq!(
                    row.captures[1]
                        .properties
                        .iter()
                        .map(&resolve)
                        .collect::<Vec<_>>(),
                    [(b"kind".to_vec(), Some(b"raw".to_vec())),]
                );
                assert!(row.dynamic_properties.is_empty());
            } else {
                assert_eq!(row.captures.len(), 1);
                assert_eq!(row.captures[0].span, 7..23);
                assert_eq!(row.captures[0].text, None);
                assert!(row.properties.is_empty());
                assert_eq!(
                    row.dynamic_properties,
                    [(b"language".to_vec(), Some(b"json".to_vec())),]
                );
            }
            #[cfg(target_os = "macos")]
            let scope = crate::code_rule_dsl::transducer::packed::MetalTestScope::enter();
            let actual = crate::code_rule_dsl::transducer::packed::interpreter_reference(
                std::slice::from_ref(&program),
                &files[0],
                std::slice::from_ref(&roots),
                &[u32::MAX],
            )
            .expect("the independent publication oracle must support output directives")
            .into_outcome()
            .unwrap();
            assert_eq!(actual.outputs, expected, "{query}");
            assert_eq!(actual.errors, vec![vec![None]]);
            #[cfg(target_os = "macos")]
            assert!(scope.snapshot().is_empty());
        }
    }

    #[test]
    fn interpreter_publication_preserves_limits_errors_and_original_task_gaps() {
        use crate::code_rule_dsl::transducer::packed::{RESTORE_CURSOR, interpreter_reference};
        let files = [tape("ALPHA\n'application/json'\n")];
        let language = SupportLang::Python.get_ts_language();
        let programs = [
            Program::compile_tree_sitter_query_pattern(
                "((identifier) @name (#set! role \"name\"))",
                &language,
            )
            .unwrap(),
            Program::new(
                vec![word(RESTORE_CURSOR, 0, 0), word(EMIT, 0, 0)],
                vec![b"error".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap(),
            Program::compile_tree_sitter_query_pattern(
                "((string (string_content) @mime) (#set-lang-from-mimetype! @mime))",
                &language,
            )
            .unwrap(),
        ];
        let roots = (0..files[0].transducer_len() as u32).collect::<Vec<_>>();
        let anchors = vec![roots.clone(), vec![0], roots.clone()];
        let limits = [0, 0, 1];
        let mut expected = crate::code_rule_dsl::transducer::packed::execute_batch_outcomes(
            &programs,
            &files,
            std::slice::from_ref(&anchors),
        )
        .unwrap();
        expected.outputs[0][0].clear();
        expected.outputs[0][2].truncate(1);
        let actual = interpreter_reference(&programs, &files[0], &anchors, &limits)
            .unwrap()
            .into_outcome()
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual.errors,
            [vec![None, Some(Error::InvalidOperand(0)), None]]
        );
        assert_eq!(actual.outputs[0][2].len(), 1);
        let string = roots
            .iter()
            .copied()
            .find(|&node| {
                files[0].transducer_kind(node) == language.id_for_node_kind("string", true)
            })
            .unwrap();
        let row = &actual.outputs[0][2][0];
        assert_eq!(row.start, string);
        assert_eq!(
            row.task_ordinal as usize,
            roots.len() + 1 + roots.iter().position(|&node| node == string).unwrap()
        );
        assert_eq!(
            row.dynamic_properties,
            [(b"language".to_vec(), Some(b"json".to_vec()))]
        );
        assert!(matches!(
            interpreter_reference(&programs, &files[0], &anchors[..2], &limits),
            Err(Error::InvalidAnchor)
        ));
        assert!(matches!(
            interpreter_reference(&programs, &files[0], &anchors, &limits[..2]),
            Err(Error::InvalidAnchor)
        ));
        let invalid_roots = vec![vec![1, 0], vec![0], roots];
        assert!(matches!(
            interpreter_reference(&programs, &files[0], &invalid_roots, &limits),
            Err(Error::InvalidAnchor)
        ));
    }

    #[test]
    #[ignore = "pinned pattern 277 and argparse output-directive reference regression"]
    fn interpreter_output_reference_preserves_pinned_argparse_results() {
        let repository =
            std::path::PathBuf::from(std::env::var("TEMPO_ALL_SCM_REPOSITORY").unwrap());
        let query_root = std::env::var("TEMPO_PYTHON_SCM_QUERY_DIR").unwrap();
        let mut paths = std::fs::read_dir(query_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "scm"))
            .collect::<Vec<_>>();
        paths.sort();
        let patterns = paths
            .iter()
            .flat_map(|path| {
                Program::tree_sitter_top_level_patterns(&std::fs::read_to_string(path).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(patterns.len(), 335);
        let program = Program::compile_tree_sitter_query_pattern(
            &patterns[277],
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        assert!(!program.output_program.instructions.is_empty());
        let original =
            std::fs::read_to_string(repository.join("Lib/test/test_argparse.py")).unwrap();
        for (source, root_count, match_count) in [
            (original.clone(), 3284, 0),
            (
                format!("{original}\nre.compile(r'CPU_REFERENCE_PROOF')\n"),
                3285,
                1,
            ),
        ] {
            let files = [tape(&source)];
            let call = SupportLang::Python
                .get_ts_language()
                .id_for_node_kind("call", true);
            let roots = (0..files[0].transducer_len() as u32)
                .filter(|&node| files[0].transducer_kind(node) == call)
                .collect::<Vec<_>>();
            assert_eq!(roots.len(), root_count);
            let anchors = vec![vec![roots.clone()]];
            let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
                std::slice::from_ref(&program),
                &files,
                Some(&anchors),
            )
            .unwrap();
            assert_eq!(expected[0][0].len(), match_count);
            if match_count == 1 {
                let row = &expected[0][0][0];
                assert_eq!(row.task_ordinal as usize, root_count - 1);
                assert_eq!(
                    row.captures
                        .iter()
                        .map(|capture| (
                            program.label_name(capture.label).unwrap().to_owned(),
                            source[capture.span.clone()].to_owned(),
                        ))
                        .collect::<Vec<_>>(),
                    [
                        ("_re".to_owned(), "re".to_owned()),
                        (
                            "injection.content".to_owned(),
                            "CPU_REFERENCE_PROOF".to_owned()
                        ),
                    ]
                );
                assert_eq!(
                    row.properties
                        .iter()
                        .map(|(key, value)| (
                            program.string_value(*key).unwrap().to_owned(),
                            value.map(|value| program.string_value(value).unwrap().to_owned()),
                        ))
                        .collect::<Vec<_>>(),
                    [("injection.language".to_owned(), Some("regex".to_owned())),]
                );
            }
            let actual = crate::code_rule_dsl::transducer::packed::interpreter_reference(
                std::slice::from_ref(&program),
                &files[0],
                std::slice::from_ref(&roots),
                &[u32::MAX],
            )
            .expect("the pinned output-directive reference must complete without JIT workspace")
            .into_outcome()
            .unwrap();
            assert_eq!(actual.outputs, expected);
            assert_eq!(actual.errors, vec![vec![None]]);
            eprintln!(
                "TEMPO_ARGPARSE_REFERENCE_EVIDENCE {}",
                serde_json::json!({
                    "roots": roots.len(), "matches": actual.outputs[0][0].len(),
                    "captures": actual.outputs[0][0].iter().map(|row| row.captures.len()).sum::<usize>(),
                    "matchProperties": actual.outputs[0][0].iter().map(|row| row.properties.len()).sum::<usize>(),
                })
            );
        }
    }

    #[test]
    #[ignore = "offline independent interpreter timing for the CPU corpus reference"]
    fn cpu_quantifier_interpreter_reference_profile() {
        let path = std::env::var("TEMPO_CPU_QUANTIFIER_SOURCE").unwrap();
        let source = std::fs::read_to_string(path).unwrap();
        let files = [tape(&source)];
        let programs = [Program::compile_tree_sitter_query_pattern(
            "(comment)+ @comment.around",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let kinds = programs[0].leading_kinds().unwrap();
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32)
                .filter(|&node| kinds.contains(&files[0].transducer_kind(node)))
                .collect::<Vec<_>>(),
        ]];
        let started = std::time::Instant::now();
        let output =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        eprintln!(
            "TEMPO_INTERPRETER_QUANTIFIER_REFERENCE roots={} matches={} captures={} seconds={:.6}",
            anchors[0][0].len(),
            output[0][0].len(),
            output[0][0]
                .iter()
                .map(|result| result.captures.len())
                .sum::<usize>(),
            started.elapsed().as_secs_f64()
        );
    }

    #[test]
    fn nested_optional_types_keep_interpreter_candidate_ordinals() {
        let query = "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))))";
        let programs = [Program::compile_tree_sitter_query_pattern(
            query,
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap()];
        let files = [tape(
            "fields: ClassVar[dict[str, Field[str]]]\nfile: IO[str] | IO[bytes] | None\n",
        )];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        assert_eq!(
            super::execute_batch(&programs, &files, &anchors).unwrap(),
            expected
        );
    }

    #[test]
    fn shared_cpu_query_storage_isolated_between_semantic_roots() {
        let program = Program::compile_tree_sitter_query_pattern(
            "(comment)+ @comment.around",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap();
        let files = [tape("# first\n# second\nvalue\n# third\n")];
        let anchors = vec![vec![
            (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
        ]];
        let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
            std::slice::from_ref(&program),
            &files,
            Some(&anchors),
        )
        .unwrap();
        let actual = super::execute_batch(&[program], &files, &anchors).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn callback_free_cpu_jit_executes_the_shared_packed_batch() {
        let grammar = SupportLang::Python.get_ts_language();
        let kind = grammar.id_for_node_kind("identifier", true);
        let program = Program::new(
            vec![word(KIND, 0, kind), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let files = vec![tape("first\n"), tape("second\n")];
        let anchors = files
            .iter()
            .map(|file| vec![(0..file.transducer_len() as u32).collect::<Vec<_>>()])
            .collect::<Vec<_>>();
        let expected = crate::code_rule_dsl::transducer::packed::execute_batch(
            std::slice::from_ref(&program),
            &files,
            Some(&anchors),
        )
        .unwrap();
        let actual = super::execute_batch(&[program], &files, &anchors).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn callback_free_cpu_jit_uses_per_program_packed_value_ranges() {
        use crate::code_rule_dsl::transducer::packed::{Directive, TEXT, TextPredicate};

        let programs = [b"FIRST".to_vec(), b"SECOND".to_vec()]
            .into_iter()
            .map(|expected| {
                Program::new(
                    vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                    vec![expected.clone(), b"match".to_vec()],
                    vec![TextPredicate::Equal(0)],
                    vec![1],
                    Vec::new(),
                )
                .unwrap()
                .with_directives(vec![Directive::Downcase { capture: 0 }])
                .unwrap()
            })
            .collect::<Vec<_>>();
        let files = vec![tape("FIRST"), tape("SECOND")];
        let anchors = files
            .iter()
            .map(|file| {
                programs
                    .iter()
                    .map(|_| (0..file.transducer_len() as u32).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let expected =
            crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                .unwrap();
        let actual = super::execute_batch(&programs, &files, &anchors).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn cranelift_task_shell_writes_status_without_a_rust_callout() {
        let program = Program::new(
            vec![word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_task_shell(&native).unwrap();
        let mut status = TaskStatus {
            error: 1,
            result_count: 2,
            capture_count: 3,
            program_counter: 4,
            final_capture_count: 5,
            property_count: 6,
            output_byte_count: 7,
            reserved: 8,
        };
        compiled.execute(&mut status);
        assert_eq!(status, TaskStatus::default());
    }

    #[test]
    fn straight_line_cranelift_executes_kind_flags_size_and_emit() {
        use crate::code_rule_dsl::transducer::packed::{FLAGS, KIND, SIZE};

        let program = Program::new(
            vec![
                word(KIND, 0, 7),
                word(FLAGS, 1, 2),
                word(SIZE, 0, 4),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            source_start: 3,
            source_end: 7,
            kind: 7,
            flags: 1,
            ..MetalNode::default()
        }];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 9,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 1,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(results[0].task_ordinal, 9);
        assert_eq!(results[0].local_ordinal, 0);
        assert_eq!(results[0].label, 0);
    }

    #[test]
    fn straight_line_cranelift_rejects_without_emitting() {
        use crate::code_rule_dsl::transducer::packed::KIND;

        let program = Program::new(
            vec![word(KIND, 0, 8), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            kind: 7,
            ..MetalNode::default()
        }];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 1,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(
            status,
            TaskStatus {
                reserved: 1,
                ..TaskStatus::default()
            }
        );
    }

    #[test]
    fn cranelift_scheduler_preserves_fallthrough_before_deferred_fork_order() {
        use crate::code_rule_dsl::transducer::packed::{FORK, STOP};

        let program = Program::new(
            vec![
                word(FORK, 0, 3),
                word(EMIT, 0, 0),
                word(STOP, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"first".to_vec(), b"second".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 1];
        let captures = [LinkRecord::default(); 11];
        let mut final_captures = [FinalCaptureRecord::default(); 2];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: 1,
            current_steps: 0,
            current_captures: 11,
            current_cursors: 12,
            current_repeats: 13,
            current_following: 0,
            current_capture_hash: 15,
            current_cursor_hash: 16,
            current_repeat_hash: 17,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: captures.as_ptr(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 2,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: final_captures.as_mut_ptr(),
            final_capture_capacity: final_captures.len() as u32,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 2);
        assert_eq!(
            (
                context.current_captures,
                context.current_cursors,
                context.current_repeats,
                context.current_following,
                context.current_capture_hash,
                context.current_cursor_hash,
                context.current_repeat_hash,
            ),
            (11, 12, 13, 0, 15, 16, 17)
        );
        assert_eq!([results[0].label, results[1].label], [0, 1]);
        assert_eq!([results[0].local_ordinal, results[1].local_ordinal], [0, 1]);
    }

    #[test]
    fn cranelift_output_sorts_results_by_start_then_first_capture_node() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, FORK, NEXT, STOP};

        let program = Program::new(
            vec![
                word(FORK, 0, 5),
                word(NEXT, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
                word(STOP, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let nodes = [
            MetalNode {
                next_any: 1,
                next_named: 1,
                parent: NONE,
                source_end: 2,
                ..MetalNode::default()
            },
            MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_start: 1,
                source_end: 2,
                ..MetalNode::default()
            },
        ];
        let mut pending = [PendingRecord::default(); 1];
        let mut captures = [LinkRecord::default(); 2];
        let mut final_captures = [FinalCaptureRecord::default(); 2];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.current_node = 0;
        context.pending = pending.as_mut_ptr();
        context.pending_quota_limit = 1;
        context.task_work_limit = 32;
        context.path_work_limit = 32;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = captures.len() as u32;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = results.len() as u32;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = final_captures.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 2);
        assert_eq!(
            [
                final_captures[results[0].capture_start as usize].node,
                final_captures[results[1].capture_start as usize].node,
            ],
            [0, 1],
        );
        assert_eq!([results[0].local_ordinal, results[1].local_ordinal], [1, 0]);
    }

    #[test]
    fn cranelift_greedy_fork_replaces_one_stable_pending_token() {
        use crate::code_rule_dsl::transducer::packed::{FORK, GOTO};

        let program = Program::new(
            vec![
                word(FORK, 1, 5),
                word(FORK, 0, 3),
                word(GOTO, 0, 3),
                word(FORK, 1, 5),
                word(EMIT, 0, 0),
                word(EMIT, 0, 1),
            ],
            vec![b"first".to_vec(), b"second".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
        .unwrap();
        let mut native = native_ir::lower(&program).unwrap();
        native.tree_sitter_query = true;
        native.has_repeats = true;
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 2];
        let mut query_buckets = [QueryBucketRecord::default(); 8];
        let mut query_members = [QueryMemberRecord::default(); 6];
        let mut queued_states = [QueuedStateRecord::default(); 1];
        let mut greedy_entries = [GreedyEntryRecord::default(); 1];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: pending.len() as u32,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 64,
            path_work_limit: 64,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: query_buckets.as_mut_ptr(),
            query_bucket_capacity: query_buckets.len() as u32,
            query_members: query_members.as_mut_ptr(),
            query_member_count: 0,
            query_member_quota_limit: query_members.len() as u32,
            queued_states: queued_states.as_mut_ptr(),
            queued_state_quota_limit: queued_states.len() as u32,
            greedy_entries: greedy_entries.as_mut_ptr(),
            greedy_entry_quota_limit: greedy_entries.len() as u32,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(results[0].label, 0);
        assert_eq!(greedy_entries[0].occupied, 2);
        assert_eq!(greedy_entries[0].path.pc, 5);
        assert!(greedy_entries[0].path.steps > 1);
        assert_eq!(context.pending_count, 0);

        query_buckets.fill(QueryBucketRecord::default());
        query_members.fill(QueryMemberRecord::default());
        queued_states.fill(QueuedStateRecord::default());
        greedy_entries.fill(GreedyEntryRecord::default());
        pending.fill(PendingRecord::default());
        results.fill(ResultRecord::default());
        status = TaskStatus::default();
        context.pending_count = 0;
        context.current_steps = 0;
        context.query_member_count = 0;
        context.greedy_entry_quota_limit = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn cranelift_following_keeps_one_lazy_continuation_for_all_and_named_siblings() {
        use crate::code_rule_dsl::transducer::packed::metal_batch_abi::PENDING_FOLLOWING;
        use crate::code_rule_dsl::transducer::packed::{FOLLOWING, NAMED};

        fn run(mode: u8, expected: u32, expected_last: u32) {
            let program = Program::new(
                vec![word(FOLLOWING, mode, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                Vec::new(),
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let mut native = native_ir::lower(&program).unwrap();
            native.tree_sitter_query = true;
            native.has_repeats = true;
            let compiled = compile_straight_line(&native).unwrap();
            let mut nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                ..MetalNode::default()
            }; 4];
            nodes[2].source_start = 4;
            nodes[2].source_end = 4;
            match (mode, expected) {
                (_, 0) => {}
                (0, 1) => nodes[0].next_any = 3,
                (0, 3) => {
                    nodes[0].next_any = 1;
                    nodes[1].next_any = 2;
                    nodes[2].next_any = 3;
                }
                (NAMED, 2) => {
                    nodes[0].next_named = 2;
                    nodes[2].next_named = 3;
                }
                _ => panic!("unsupported following test shape"),
            }
            let mut pending = [PendingRecord::default(); 1];
            let mut query_buckets = [QueryBucketRecord::default(); 8];
            let mut query_members = [QueryMemberRecord::default(); 4];
            let mut queued_states = [QueuedStateRecord::default(); 1];
            let mut results = [ResultRecord::default(); 3];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: nodes.len() as u32,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: pending.as_mut_ptr(),
                pending_count: 0,
                pending_quota_limit: pending.len() as u32,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 16,
                path_work_limit: 16,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: query_buckets.as_mut_ptr(),
                query_bucket_capacity: query_buckets.len() as u32,
                query_members: query_members.as_mut_ptr(),
                query_member_count: 0,
                query_member_quota_limit: query_members.len() as u32,
                queued_states: queued_states.as_mut_ptr(),
                queued_state_quota_limit: queued_states.len() as u32,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
                derived: std::ptr::null(),
                derived_classes: 0,
                derived_words: 0,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, u32::from(expected > 0));
            assert_eq!(context.current_node, expected_last);
            assert_eq!(context.pending_count, 0);
            assert_eq!(context.query_member_count, expected + 1);
            if expected > 1 {
                assert_eq!(pending[0].kind, PENDING_FOLLOWING);
                assert_eq!(queued_states[0].occupied, 2);
            }
        }

        run(0, 0, 0);
        run(0, 1, 3);
        run(0, 3, 3);
        run(NAMED, 2, 3);
    }

    #[test]
    fn cranelift_query_admission_suppresses_an_exact_reentered_state() {
        use crate::code_rule_dsl::transducer::packed::{FORK, GOTO};

        let program = Program::new(
            vec![
                word(FORK, 0, 3),
                word(FORK, 0, 3),
                word(GOTO, 0, 3),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let mut native = native_ir::lower(&program).unwrap();
        native.tree_sitter_query = true;
        native.has_repeats = true;
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 1];
        let mut query_buckets = [QueryBucketRecord::default(); 8];
        let mut query_members = [QueryMemberRecord::default(); 4];
        let mut queued_states = [QueuedStateRecord::default(); 1];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: 1,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 64,
            path_work_limit: 64,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: query_buckets.as_mut_ptr(),
            query_bucket_capacity: query_buckets.len() as u32,
            query_members: query_members.as_mut_ptr(),
            query_member_count: 0,
            query_member_quota_limit: query_members.len() as u32,
            queued_states: queued_states.as_mut_ptr(),
            queued_state_quota_limit: queued_states.len() as u32,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(context.pending_count, 0);
        assert_eq!(context.query_member_count, 4);
        assert_eq!(queued_states[0].occupied, 2);
    }

    #[test]
    fn cranelift_scheduler_reports_work_and_pending_capacity_exactly() {
        use crate::code_rule_dsl::transducer::packed::FORK;

        let program = Program::new(
            vec![word(FORK, 0, 1), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 2,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);

        let mut pending = [PendingRecord::default(); 1];
        status = TaskStatus::default();
        context.pending = pending.as_mut_ptr();
        context.pending_quota_limit = 1;
        context.task_work_limit = 0;
        context.path_work_limit = 0;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 4);
        assert_eq!(status.result_count, 0);

        status = TaskStatus::default();
        context.current_steps = u32::MAX;
        context.task_work_limit = u32::MAX;
        context.path_work_limit = u32::MAX;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 4);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn scheduler_error_invalidates_results_written_by_an_earlier_path() {
        use crate::code_rule_dsl::transducer::packed::FORK;

        let program = Program::new(
            vec![word(FORK, 0, 2), word(EMIT, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut pending = [PendingRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: 1,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: 1,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);
        assert_eq!(status.output_byte_count, 0);
    }

    #[test]
    fn cranelift_scheduler_relations_compare_complete_packed_states() {
        let program = Program::new(
            vec![word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [
            MetalNode {
                source_start: 0,
                source_end: 1,
                ..MetalNode::default()
            },
            MetalNode {
                source_start: 2,
                source_end: 2,
                ..MetalNode::default()
            },
        ];
        let captures = [
            LinkRecord {
                previous: 0,
                first: 7,
                second: 0,
            },
            LinkRecord {
                previous: 1,
                first: 7,
                second: 1,
            },
            LinkRecord {
                previous: 1,
                first: 7,
                second: 1,
            },
            LinkRecord {
                previous: 1,
                first: 8,
                second: 1,
            },
        ];
        let cursors = [
            LinkRecord {
                previous: 0,
                first: 0,
                second: 5,
            },
            LinkRecord {
                previous: 1,
                first: 1,
                second: 7,
            },
            LinkRecord {
                previous: 1,
                first: 1,
                second: 7,
            },
            LinkRecord {
                previous: 1,
                first: 1,
                second: 8,
            },
        ];
        let repeats = [
            RepeatRecord {
                previous: 0,
                value: 2,
                previous_hash: 0,
            },
            RepeatRecord {
                previous: 1,
                value: 3,
                previous_hash: 0,
            },
            RepeatRecord {
                previous: 1,
                value: 3,
                previous_hash: 0,
            },
            RepeatRecord {
                previous: 1,
                value: 4,
                previous_hash: 0,
            },
        ];
        let mut query_buckets = [QueryBucketRecord::default(); 4];
        let mut query_members = [QueryMemberRecord::default(); 2];
        let mut queued_states = [QueuedStateRecord::default(); 2];
        let mut greedy_entries = [GreedyEntryRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: captures.as_ptr(),
            cursors: cursors.as_ptr(),
            repeats: repeats.as_ptr(),
            query_buckets: query_buckets.as_mut_ptr(),
            query_bucket_capacity: query_buckets.len() as u32,
            query_members: query_members.as_mut_ptr(),
            query_member_count: 0,
            query_member_quota_limit: query_members.len() as u32,
            queued_states: queued_states.as_mut_ptr(),
            queued_state_quota_limit: queued_states.len() as u32,
            greedy_entries: greedy_entries.as_mut_ptr(),
            greedy_entry_quota_limit: greedy_entries.len() as u32,
            maximum_cursor_slot: 2,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: std::ptr::null_mut(),
            result_quota_limit: 0,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        unsafe {
            assert_eq!((compiled.cursor_equal)(&mut context, 2, 3), 1);
            assert_eq!((compiled.cursor_equal)(&mut context, 2, 4), 0);
            assert_eq!((compiled.repeat_equal)(&mut context, 2, 3), 1);
            assert_eq!((compiled.repeat_equal)(&mut context, 2, 4), 0);
            assert_eq!((compiled.capture_contains)(&mut context, 2, 1), 1);
            assert_eq!((compiled.capture_contains)(&mut context, 1, 2), 0);
            assert_eq!((compiled.capture_contains)(&mut context, 2, 3), 1);
            assert_eq!((compiled.capture_contains)(&mut context, 2, 4), 0);
            assert_eq!(
                (compiled.exact_state_equal)(&mut context, 2, 2, 2, 3, 3, 3),
                1
            );
            assert_eq!(
                (compiled.exact_state_equal)(&mut context, 2, 2, 2, 4, 4, 4),
                0
            );
            context.current_captures = 1;
            context.current_cursors = 2;
            context.current_repeats = 2;
            context.current_capture_hash = 11;
            context.current_cursor_hash = 12;
            context.current_repeat_hash = 13;
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 1);
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 0);
            context.current_captures = 2;
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 1);
            context.current_captures = 4;
            assert_eq!((compiled.queue_insert)(&mut context, 7, 0), 2);
            context.current_captures = 1;
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 1);
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 0);
            context.current_captures = 2;
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 1);
            context.current_captures = 4;
            // All three capture sequences are distinct. The third cannot replace
            // either retained state to conceal exhaustion of this two-slot arena.
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 2);
            context.current_captures = 2;
            assert_eq!((compiled.query_admit)(&mut context, 5, 0), 0);
            context.current_captures = 1;
            context.current_node = 0;
            let inserted = (compiled.greedy_schedule)(&mut context, 5);
            assert_eq!(inserted >> 32, 1);
            let slot = inserted as u32;
            context.current_node = 1;
            let replaced = (compiled.greedy_schedule)(&mut context, 5);
            assert_eq!(replaced >> 32, 0);
            assert_eq!(replaced as u32, slot);
            context.current_node = 0;
            assert_eq!((compiled.greedy_take)(&mut context, slot), 5);
            assert_eq!(context.current_node, 1);
            assert_eq!((compiled.greedy_take)(&mut context, slot), u64::MAX);
            assert_eq!(greedy_entries[slot as usize].occupied, 2);
            let reused = (compiled.greedy_schedule)(&mut context, 6);
            assert_eq!(reused >> 32, 1);
            assert_eq!(reused as u32, slot);
            assert_eq!((compiled.greedy_schedule)(&mut context, 7), u64::MAX);
            context.greedy_entry_quota_limit = 0;
            assert_eq!((compiled.greedy_schedule)(&mut context, 5), u64::MAX);
        }
        assert_eq!(context.query_member_count, 2);
        assert_eq!(
            query_members.map(|member| (member.captures, member.capture_count)),
            [(1, 1), (2, 2)]
        );

        // Compare all short histories over two identities at equal geometry and
        // a later zero-width node. Histories can arrive in arbitrary geometry order.
        let nodes = [
            MetalNode {
                source_start: 0,
                source_end: 6,
                ..MetalNode::default()
            },
            MetalNode {
                source_start: 0,
                source_end: 6,
                ..MetalNode::default()
            },
            MetalNode {
                source_start: 9,
                source_end: 9,
                ..MetalNode::default()
            },
        ];
        let alphabet = [(0_u16, 0_u32), (1, 0), (0, 1), (0, 2)];
        let mut sequences = vec![vec![]];
        let mut level = vec![vec![]];
        for _ in 0..3 {
            level = level
                .iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |&item| {
                        let mut sequence = prefix.clone();
                        sequence.push(item);
                        sequence
                    })
                })
                .collect();
            sequences.extend(level.clone());
        }
        let mut links = Vec::new();
        let mut heads = Vec::new();
        let ordered = sequences
            .iter()
            .map(|sequence| {
                let mut head = 0;
                let mut ordered = Vec::new();
                for &(label, node) in sequence {
                    links.push(LinkRecord {
                        previous: head,
                        first: u32::from(label),
                        second: node,
                    });
                    head = links.len() as u32;
                    let value = nodes[node as usize];
                    ordered.push(super::super::CaptureIdentity {
                        label,
                        node,
                        start: value.source_start as usize,
                        end: value.source_end as usize,
                    });
                }
                heads.push(head);
                ordered.sort_by(|a, b| super::super::compare_capture_nodes(*a, *b));
                ordered
            })
            .collect::<Vec<_>>();
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.captures = links.as_ptr();
        for (left, captures) in ordered.iter().enumerate() {
            for (right, other) in ordered.iter().enumerate() {
                let (contains, reverse) =
                    super::super::capture_identity_containment(captures, other);
                unsafe {
                    assert_eq!(
                        (compiled.capture_contains)(&mut context, heads[left], heads[right]),
                        u32::from(contains),
                        "left={captures:?}, right={other:?}"
                    );
                    assert_eq!(
                        (compiled.exact_state_equal)(
                            &mut context,
                            heads[left],
                            0,
                            0,
                            heads[right],
                            0,
                            0
                        ),
                        u32::from(contains && reverse),
                        "left={captures:?}, right={other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn cranelift_scheduler_capacity_matrix_has_exact_boundaries() {
        use crate::code_rule_dsl::transducer::packed::{FORK, KIND};

        #[derive(Clone, Copy)]
        struct Capacities {
            pending: u32,
            buckets: u32,
            members: u32,
            queued: u32,
            greedy: u32,
            work: u32,
        }

        fn run(program: &Program, query: bool, capacities: Capacities) -> TaskStatus {
            let mut native = native_ir::lower(program).unwrap();
            native.tree_sitter_query = query;
            native.has_repeats = query;
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                kind: 0,
                ..MetalNode::default()
            }];
            let mut pending = [PendingRecord::default(); 4];
            let mut buckets = [QueryBucketRecord::default(); 16];
            let mut members = [QueryMemberRecord::default(); 32];
            let mut queued = [QueuedStateRecord::default(); 4];
            let mut greedy = [GreedyEntryRecord::default(); 4];
            let mut results = [ResultRecord::default(); 8];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: 1,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: pending.as_mut_ptr(),
                pending_count: 0,
                pending_quota_limit: capacities.pending,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: capacities.work,
                path_work_limit: capacities.work,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: buckets.as_mut_ptr(),
                query_bucket_capacity: capacities.buckets,
                query_members: members.as_mut_ptr(),
                query_member_count: 0,
                query_member_quota_limit: capacities.members,
                queued_states: queued.as_mut_ptr(),
                queued_state_quota_limit: capacities.queued,
                greedy_entries: greedy.as_mut_ptr(),
                greedy_entry_quota_limit: capacities.greedy,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
                derived: std::ptr::null(),
                derived_classes: 0,
                derived_words: 0,
            };
            compiled.execute(&mut context);
            status
        }

        let normal_forks = Program::new(
            vec![
                word(FORK, 0, 4),
                word(FORK, 0, 3),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let greedy_forks = Program::new(
            vec![
                word(FORK, 1, 4),
                word(FORK, 1, 3),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let query = Program::new(
            vec![word(KIND, 0, 0), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let ample = Capacities {
            pending: 4,
            buckets: 16,
            members: 32,
            queued: 4,
            greedy: 4,
            work: 64,
        };
        for (capacity, succeeds) in [(0, false), (2, true), (1, false), (3, true)] {
            let status = run(
                &normal_forks,
                false,
                Capacities {
                    pending: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "pending capacity {capacity}");
            let status = run(
                &query,
                false,
                Capacities {
                    work: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "work limit {capacity}");
        }
        for (capacity, succeeds) in [(0, false), (2, true), (1, false), (3, true)] {
            let status = run(
                &query,
                true,
                Capacities {
                    buckets: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "bucket capacity {capacity}");
            if capacity == 0 {
                assert_eq!(status.program_counter, 0);
            }
            let status = run(
                &query,
                true,
                Capacities {
                    members: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "member capacity {capacity}");
        }
        for (capacity, succeeds) in [(0, false), (2, true), (1, false), (3, true)] {
            let status = run(
                &normal_forks,
                true,
                Capacities {
                    queued: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "queued capacity {capacity}");
            let status = run(
                &greedy_forks,
                false,
                Capacities {
                    greedy: capacity,
                    ..ample
                },
            );
            assert_eq!(status.error == 0, succeeds, "greedy capacity {capacity}");
        }
    }

    #[test]
    fn cranelift_lowers_capture_cursor_and_repeat_link_creation() {
        use crate::code_rule_dsl::transducer::packed::{
            CAPTURE, CHILD, REPEAT_BEGIN, REPEAT_BUMP, REPEAT_END, RESTORE_CURSOR, SAVE_CURSOR,
        };

        let program = Program::new(
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, 0, 0),
                word(CAPTURE, 0, 0),
                word(REPEAT_BEGIN, 0, 3),
                word(REPEAT_BUMP, 0, 3),
                word(REPEAT_END, 0, 3),
                word(RESTORE_CURSOR, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let mut nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            descendant_end: 3,
            ..MetalNode::default()
        }; 3];
        nodes[0].child_count = 2;
        nodes[1].parent = 0;
        nodes[2].parent = 0;
        let children = [1_u32, 2];
        let mut pending = [PendingRecord::default(); 1];
        let mut captures = [LinkRecord::default(); 2];
        let mut cursors = [LinkRecord::default(); 1];
        let mut repeats = [RepeatRecord::default(); 2];
        let mut final_captures = [FinalCaptureRecord::default(); 2];
        let mut results = [ResultRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: children.as_ptr(),
            child_count: children.len() as u32,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: pending.as_mut_ptr(),
            pending_count: 0,
            pending_quota_limit: pending.len() as u32,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 64,
            path_work_limit: 64,
            captures: captures.as_mut_ptr(),
            cursors: cursors.as_mut_ptr(),
            repeats: repeats.as_mut_ptr(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 1,
            capture_count: 0,
            capture_quota_limit: captures.len() as u32,
            cursor_count: 0,
            cursor_quota_limit: cursors.len() as u32,
            repeat_count: 0,
            repeat_quota_limit: repeats.len() as u32,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: final_captures.as_mut_ptr(),
            final_capture_capacity: final_captures.len() as u32,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 2);
        assert_eq!(status.capture_count, 2);
        assert_eq!(status.final_capture_count, 2);
        assert_eq!(status.property_count, 0);
        assert_eq!(context.capture_count, 2);
        assert_eq!(context.cursor_count, 1);
        assert_eq!(context.repeat_count, 2);
        assert_eq!([captures[0].second, captures[1].second], [1, 2]);
        assert_eq!([results[0].capture_start, results[1].capture_start], [0, 1]);
        assert_eq!([results[0].capture_count, results[1].capture_count], [1, 1]);
        assert_eq!(
            cursors[0],
            LinkRecord {
                previous: 0,
                first: 0,
                second: 0
            }
        );
        assert_eq!([repeats[0].value, repeats[1].value], [3, 3]);
        assert_eq!(context.current_node, 0);
        let mix = |hash: u32, value: u32| {
            hash ^ value
                .wrapping_add(0x9e37_79b9)
                .wrapping_add(hash << 6)
                .wrapping_add(hash >> 2)
        };
        assert_eq!(context.current_capture_hash, mix(0, mix(0, 2)));
        assert_eq!(context.current_cursor_hash, mix(0, NONE) ^ mix(0, 0));
        assert_eq!(context.current_repeat_hash, 0);

        status = TaskStatus::default();
        pending.fill(PendingRecord::default());
        context.current_node = 0;
        context.pending_count = 0;
        context.current_steps = 0;
        context.current_captures = 0;
        context.current_cursors = 0;
        context.current_repeats = 0;
        context.current_capture_hash = 0;
        context.current_cursor_hash = 0;
        context.current_repeat_hash = 0;
        context.capture_count = 0;
        context.cursor_count = 0;
        context.repeat_count = 0;
        context.capture_quota_limit = 1;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);

        status = TaskStatus::default();
        context.current_node = 0;
        context.pending_count = 0;
        context.current_steps = 0;
        context.current_captures = 0;
        context.current_cursors = 0;
        context.current_repeats = 0;
        context.current_capture_hash = 0;
        context.current_cursor_hash = 0;
        context.current_repeat_hash = 0;
        context.capture_count = 0;
        context.cursor_count = 0;
        context.repeat_count = 0;
        context.capture_quota_limit = 2;
        context.cursor_quota_limit = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);

        status = TaskStatus::default();
        context.current_node = 0;
        context.pending_count = 0;
        context.current_steps = 0;
        context.current_captures = 0;
        context.current_cursors = 0;
        context.current_repeats = 0;
        context.current_capture_hash = 0;
        context.current_cursor_hash = 0;
        context.current_repeat_hash = 0;
        context.capture_count = 0;
        context.cursor_count = 0;
        context.repeat_count = 0;
        context.cursor_quota_limit = 1;
        context.repeat_quota_limit = 1;
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);
    }

    #[test]
    fn cranelift_lowers_child_and_descendant_multitarget_navigation() {
        use crate::code_rule_dsl::transducer::packed::{CHILD, DESCEND, NAMED};

        fn run(operation: u8, mode: u8, _expected: u32) {
            let program = Program::new(
                vec![word(operation, mode, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                Vec::new(),
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let mut native = native_ir::lower(&program).unwrap();
            native.tree_sitter_query = true;
            native.has_repeats = true;
            let compiled = compile_straight_line(&native).unwrap();
            let mut nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                descendant_end: 4,
                ..MetalNode::default()
            }; 4];
            nodes[0].child_count = 3;
            nodes[1].parent = 0;
            nodes[1].flags = 1;
            nodes[2].parent = 0;
            nodes[2].source_start = 3;
            nodes[2].source_end = 3;
            nodes[3].parent = 0;
            nodes[3].flags = 1;
            let children = [1_u32, 2, 3];
            let mut pending = [PendingRecord::default(); 2];
            let mut query_buckets = [QueryBucketRecord::default(); 8];
            let mut query_members = [QueryMemberRecord::default(); 8];
            let mut queued_states = [QueuedStateRecord::default(); 2];
            let mut results = [ResultRecord::default(); 3];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: nodes.len() as u32,
                children: children.as_ptr(),
                child_count: children.len() as u32,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: pending.as_mut_ptr(),
                pending_count: 0,
                pending_quota_limit: pending.len() as u32,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 32,
                path_work_limit: 32,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: query_buckets.as_mut_ptr(),
                query_bucket_capacity: query_buckets.len() as u32,
                query_members: query_members.as_mut_ptr(),
                query_member_count: 0,
                query_member_quota_limit: query_members.len() as u32,
                queued_states: queued_states.as_mut_ptr(),
                queued_state_quota_limit: queued_states.len() as u32,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
                derived: std::ptr::null(),
                derived_classes: 0,
                derived_words: 0,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0, "operation {operation} mode {mode}");
            assert_eq!(status.result_count, 1, "operation {operation} mode {mode}");
            assert_eq!(context.current_node, 3, "operation {operation} mode {mode}");
            assert_eq!(context.pending_count, 0);
        }

        run(CHILD, 0, 3);
        run(CHILD, NAMED, 2);
        run(DESCEND, 0, 3);
        run(DESCEND, NAMED, 2);
    }

    fn execute_generated_text(
        predicate: crate::code_rule_dsl::transducer::packed::TextPredicate,
        strings: Vec<Vec<u8>>,
        source: &[u8],
        value_capacity: usize,
    ) -> TaskStatus {
        use crate::code_rule_dsl::transducer::packed::TEXT;

        let program = Program::new(
            vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
            strings,
            vec![predicate],
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut work = vec![0_u32; value_capacity];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 1 << 20;
        context.path_work_limit = 1 << 20;
        context.value_work = work.as_mut_ptr();
        context.value_capacity = work.len() as u32;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = results.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        status
    }

    #[test]
    fn cranelift_assert_lowers_single_capture_programs() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CAPTURE, CapturePredicate, TextPredicate};

        for (predicate, strings, source, expected) in [
            (
                CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                },
                vec![b"name".to_vec()],
                b"name".as_slice(),
                1,
            ),
            (
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![0, 1],
                    negated: false,
                    any: false,
                },
                vec![b"other".to_vec(), b"name".to_vec()],
                b"name".as_slice(),
                1,
            ),
            (
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![0],
                    negated: true,
                    any: false,
                },
                vec![b"name".to_vec()],
                b"name".as_slice(),
                0,
            ),
            (
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![0],
                    negated: false,
                    any: false,
                },
                vec![b"longer".to_vec()],
                b"".as_slice(),
                0,
            ),
            (
                CapturePredicate::Kind {
                    capture: 0,
                    kind: 1,
                    negated: true,
                    any: false,
                },
                vec![b"capture".to_vec()],
                b"".as_slice(),
                1,
            ),
            (
                CapturePredicate::Contains {
                    capture: 0,
                    strings: vec![0],
                    negated: false,
                    any: false,
                },
                vec![b"amp".to_vec()],
                b"example".as_slice(),
                1,
            ),
            (
                CapturePredicate::Relation {
                    capture: 0,
                    kinds: vec![1],
                    ancestor: true,
                    negated: true,
                },
                vec![b"capture".to_vec()],
                b"".as_slice(),
                1,
            ),
        ] {
            let text_predicates = if matches!(predicate, CapturePredicate::Text { .. }) {
                vec![TextPredicate::Equal(0)]
            } else {
                Vec::new()
            };
            let program = Program::new_with_capture_predicates(
                vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
                strings,
                text_predicates,
                vec![predicate],
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_end: source.len() as u32,
                ..MetalNode::default()
            }];
            let mut captures = [LinkRecord::default(); 1];
            let mut final_captures = [FinalCaptureRecord::default(); 1];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context: NativeTaskContext = unsafe { mem::zeroed() };
            context.nodes = nodes.as_ptr();
            context.node_count = 1;
            context.source = source.as_ptr();
            context.source_length = source.len() as u32;
            context.current_node = 0;
            context.task_work_limit = 1 << 20;
            context.path_work_limit = 1 << 20;
            context.captures = captures.as_mut_ptr();
            context.capture_quota_limit = 1;
            context.results = results.as_mut_ptr();
            context.result_quota_limit = 1;
            context.final_captures = final_captures.as_mut_ptr();
            context.final_capture_capacity = final_captures.len() as u32;
            context.status = &mut status;
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, expected);
        }
    }

    #[test]
    fn cranelift_assert_lowers_paired_capture_programs() {
        use crate::code_rule_dsl::transducer::packed::{ASSERT, CAPTURE, CapturePredicate};

        for predicate in [
            CapturePredicate::Equal {
                left: 0,
                right: 1,
                negated: false,
                any: false,
            },
            CapturePredicate::SameLine {
                left: 0,
                right: 1,
                negated: false,
            },
        ] {
            let program = Program::new_with_capture_predicates(
                vec![
                    word(CAPTURE, 0, 1),
                    word(CAPTURE, 0, 0),
                    word(ASSERT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"left".to_vec(), b"right".to_vec()],
                Vec::new(),
                vec![predicate],
                vec![0, 1],
                Vec::new(),
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let source = b"same";
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_end: source.len() as u32,
                ..MetalNode::default()
            }];
            let mut captures = [LinkRecord::default(); 2];
            let mut final_captures = [FinalCaptureRecord::default(); 2];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context: NativeTaskContext = unsafe { mem::zeroed() };
            context.nodes = nodes.as_ptr();
            context.node_count = 1;
            context.source = source.as_ptr();
            context.source_length = source.len() as u32;
            context.current_node = 0;
            context.task_work_limit = 1 << 20;
            context.path_work_limit = 1 << 20;
            context.captures = captures.as_mut_ptr();
            context.capture_quota_limit = captures.len() as u32;
            context.results = results.as_mut_ptr();
            context.result_quota_limit = 1;
            context.final_captures = final_captures.as_mut_ptr();
            context.final_capture_capacity = final_captures.len() as u32;
            context.status = &mut status;
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, 1);
            assert_eq!([final_captures[0].label, final_captures[1].label], [0, 1]);
        }
    }

    #[test]
    fn cranelift_output_writes_typed_match_properties() {
        use crate::code_rule_dsl::transducer::packed::Directive;

        let program = Program::new(
            vec![
                word(crate::code_rule_dsl::transducer::packed::CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"key".to_vec(), b"value".to_vec(), b"match".to_vec()],
            Vec::new(),
            vec![2],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![
            Directive::Set {
                key: 0,
                value: Some(1),
            },
            Directive::SetCapture {
                capture: 0,
                key: 0,
                value: Some(1),
            },
        ])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            ..MetalNode::default()
        }];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut properties = [PropertyRecord::default(); 2];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.properties = properties.as_mut_ptr();
        context.property_quota_limit = 2;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(status.property_count, 2);
        assert_eq!(results[0].property_count, 1);
        assert_eq!(properties[0].kind, 0);
        assert_eq!(properties[0].key, 0);
        assert_eq!(properties[0].value, 1);
        assert_eq!(properties[1].kind, 1);
        assert_eq!(properties[1].owner, 0);
        assert_eq!(final_captures[0].property_count, 1);
    }

    #[test]
    fn cranelift_output_applies_capture_offsets() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::Offset {
            capture: 0,
            offsets: [0, 1, 0, 0],
        }])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let source = b"abc";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let line_starts = [0_u32];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.line_starts = line_starts.as_ptr();
        context.line_count = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!((final_captures[0].start, final_captures[0].end), (1, 3));
    }

    #[test]
    fn cranelift_output_rejects_invalid_offsets_without_visible_records() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::Offset {
            capture: 0,
            offsets: [0, 99, 0, 0],
        }])
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let source = b"abc";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let line_starts = [0_u32];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.line_starts = line_starts.as_ptr();
        context.line_count = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 0);
        assert_eq!(status.final_capture_count, 0);
        assert_eq!(status.property_count, 0);
    }

    #[test]
    fn cranelift_output_selects_adjacent_captures_in_metal_order() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, CHILD, Directive, NEXT};

        let program = Program::new(
            vec![
                word(CHILD, 0, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 0, 0),
                word(CAPTURE, 0, 0),
                word(NEXT, 0, 0),
                word(CAPTURE, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![b"source".to_vec(), b"target".to_vec()],
            Vec::new(),
            vec![0, 1],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::SelectAdjacent {
            capture: 0,
            target: 1,
        }])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let source = b"a\n\nb t";
        let mut nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            descendant_end: 4,
            ..MetalNode::default()
        }; 4];
        nodes[0].child_count = 1;
        nodes[1].parent = 0;
        nodes[1].next_any = 2;
        nodes[1].source_end = 1;
        nodes[2].parent = 0;
        nodes[2].next_any = 3;
        nodes[2].source_start = 3;
        nodes[2].source_end = 4;
        nodes[3].parent = 0;
        nodes[3].source_start = 5;
        nodes[3].source_end = 6;
        let children = [1_u32];
        let line_starts = [0_u32, 2, 3];
        let mut captures = [LinkRecord::default(); 3];
        let mut final_captures = [FinalCaptureRecord::default(); 3];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = nodes.len() as u32;
        context.children = children.as_ptr();
        context.child_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 32;
        context.path_work_limit = 32;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = captures.len() as u32;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.line_starts = line_starts.as_ptr();
        context.line_count = line_starts.len() as u32;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = final_captures.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(
            [
                final_captures[0].retained,
                final_captures[1].retained,
                final_captures[2].retained,
            ],
            [0, 1, 1],
        );
    }

    #[test]
    fn cranelift_output_downcases_into_bounded_transformed_storage() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![
            Directive::Downcase { capture: 0 },
            Directive::Downcase { capture: 0 },
        ])
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let source = "ÄABCz".as_bytes();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut bytes = [0_u8; 16];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 16;
        context.path_work_limit = 16;
        context.captures = captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.output_bytes = bytes.as_mut_ptr();
        context.output_byte_quota_limit = bytes.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.output_byte_count, (source.len() * 2) as u32);
        assert_eq!(&bytes[..source.len()], "Äabcz".as_bytes());
        assert_eq!(&bytes[source.len()..source.len() * 2], "Äabcz".as_bytes());
        assert_eq!(final_captures[0].text_start, source.len() as u32);
        assert_eq!(final_captures[0].text_length, source.len() as u32);
    }

    #[test]
    fn cranelift_output_strips_ordered_nfa_matches() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"n".to_vec(), b"capture".to_vec()],
            Vec::new(),
            vec![1],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::Strip {
            capture: 0,
            source: 0,
            compiled: regex::Regex::new("n").unwrap(),
        }])
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let source = b"banana";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut raw_captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut value_work = [0_u32; 256];
        let mut bytes = [0_u8; 16];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 4096;
        context.path_work_limit = 4096;
        context.captures = raw_captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.value_work = value_work.as_mut_ptr();
        context.value_capacity = value_work.len() as u32;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.output_bytes = bytes.as_mut_ptr();
        context.output_byte_quota_limit = bytes.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(status.output_byte_count, 4);
        assert_eq!(&bytes[..4], b"baaa");
        assert_eq!(final_captures[0].text_start, 0);
        assert_eq!(final_captures[0].text_length, 4);
    }

    #[test]
    fn cranelift_output_normalizes_language_from_mime() {
        use crate::code_rule_dsl::transducer::packed::{CAPTURE, Directive};

        let program = Program::new(
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"capture".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap()
        .with_directives(vec![Directive::SetLanguageFromMimeType { capture: 0 }])
        .unwrap();
        let compiled = compile_straight_line(&native_ir::lower(&program).unwrap()).unwrap();
        let source = b"text/module";
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            source_end: source.len() as u32,
            ..MetalNode::default()
        }];
        let mut raw_captures = [LinkRecord::default(); 1];
        let mut final_captures = [FinalCaptureRecord::default(); 1];
        let mut results = [ResultRecord::default(); 1];
        let mut properties = [PropertyRecord::default(); 1];
        let mut bytes = [0_u8; 32];
        let mut status = TaskStatus::default();
        let mut context: NativeTaskContext = unsafe { mem::zeroed() };
        context.nodes = nodes.as_ptr();
        context.node_count = 1;
        context.source = source.as_ptr();
        context.source_length = source.len() as u32;
        context.current_node = 0;
        context.task_work_limit = 64;
        context.path_work_limit = 64;
        context.captures = raw_captures.as_mut_ptr();
        context.capture_quota_limit = 1;
        context.results = results.as_mut_ptr();
        context.result_quota_limit = 1;
        context.final_captures = final_captures.as_mut_ptr();
        context.final_capture_capacity = 1;
        context.properties = properties.as_mut_ptr();
        context.property_quota_limit = 1;
        context.output_bytes = bytes.as_mut_ptr();
        context.output_byte_quota_limit = bytes.len() as u32;
        context.status = &mut status;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        assert_eq!(status.property_count, 1);
        assert_eq!(status.output_byte_count, 18);
        assert_eq!(&bytes[..18], b"javascriptlanguage");
        assert_eq!(results[0].dynamic_count, 1);
        assert_eq!(properties[0].kind, 2);
        assert_eq!(properties[0].value, 1);
        assert_eq!(properties[0].value_start, 0);
        assert_eq!(properties[0].value_length, 10);
        assert_eq!(properties[0].key_start, 10);
        assert_eq!(properties[0].key_length, 8);
    }

    #[test]
    fn cranelift_text_lowers_lua_value_programs() {
        use crate::code_rule_dsl::transducer::packed::{LuaPattern, TextPredicate};

        for (pattern, source, expected) in [
            (b"^%a+$".as_slice(), b"name".as_slice(), 1),
            (b"^%a+$".as_slice(), b"name1".as_slice(), 0),
            (b"^(%a+)%s+%1$".as_slice(), b"go go".as_slice(), 1),
            (b"^%b()$".as_slice(), b"(a(b)c)".as_slice(), 1),
            (b"^a.-b$".as_slice(), b"axxxbxxb".as_slice(), 1),
            (b"%f[%a]word".as_slice(), b"a word".as_slice(), 1),
            (b"%f[%a]word".as_slice(), b"sword".as_slice(), 0),
            (b"^[-][-](%s?)@$".as_slice(), b"-- @".as_slice(), 1),
            (b"^()value$".as_slice(), b"value".as_slice(), 1),
            (b"^%d*$".as_slice(), b"12345".as_slice(), 1),
            (b"^.$".as_slice(), b"?".as_slice(), 1),
            (b"^x$".as_slice(), b"x".as_slice(), 1),
            (b"^%c$".as_slice(), b"\n".as_slice(), 1),
            (b"^%d$".as_slice(), b"7".as_slice(), 1),
            (b"^%g$".as_slice(), b"!".as_slice(), 1),
            (b"^%l$".as_slice(), b"q".as_slice(), 1),
            (b"^%p$".as_slice(), b"!".as_slice(), 1),
            (b"^%s$".as_slice(), b"\t".as_slice(), 1),
            (b"^%u$".as_slice(), b"Q".as_slice(), 1),
            (b"^%w$".as_slice(), b"7".as_slice(), 1),
            (b"^%x$".as_slice(), b"f".as_slice(), 1),
            (b"^%z$".as_slice(), b"\0".as_slice(), 1),
            (b"^%A$".as_slice(), b"7".as_slice(), 1),
            (b"^[a-c]$".as_slice(), b"b".as_slice(), 1),
            (b"^[^a-c]$".as_slice(), b"z".as_slice(), 1),
            (b"^a?$".as_slice(), b"".as_slice(), 1),
            (b"^a+$".as_slice(), b"aaa".as_slice(), 1),
            (b"^a*$".as_slice(), b"".as_slice(), 1),
        ] {
            let status = execute_generated_text(
                TextPredicate::LuaPattern {
                    source: 1,
                    compiled: LuaPattern::parse(pattern).unwrap(),
                },
                vec![b"match".to_vec(), pattern.to_vec()],
                source,
                4096,
            );
            assert_eq!(status.error, 0, "Lua {pattern:?} on {source:?}");
            assert_eq!(
                status.result_count, expected,
                "Lua {pattern:?} on {source:?}"
            );
        }
    }

    #[test]
    fn cranelift_lua_reports_exact_capacity_errors() {
        use crate::code_rule_dsl::transducer::packed::{LuaPattern, TextPredicate};

        let predicate = || TextPredicate::LuaPattern {
            source: 1,
            compiled: LuaPattern::parse(b"^a$").unwrap(),
        };
        let strings = || vec![b"match".to_vec(), b"^a$".to_vec()];
        let status = execute_generated_text(predicate(), strings(), b"a", 1);
        assert_eq!(status.error, 2);
        let status = execute_generated_text(predicate(), strings(), b"a", 2);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
    }

    #[test]
    fn cranelift_regex_matches_the_byte_oracle_across_look_states() {
        use crate::code_rule_dsl::transducer::packed::TextPredicate;

        let cases: &[(&str, &[u8])] = &[
            (r"^$", b""),
            (r"^a", b"abc"),
            (r"a$", b"ba"),
            (r"(?m)^b", b"a\nb"),
            (r"(?m)b$", b"b\na"),
            (r"(?-u:\bword\b)", b" word "),
            (r"(?-u:\Bword\B)", b"swords"),
            (r"(?u)\bπ\b", " π ".as_bytes()),
            (r"(?u)\Bπ\B", "xπx".as_bytes()),
            (r"(?:ab|a)c", b"ac"),
            (r"(?:a*?)(?:ab|b)", b"aaab"),
            (r"a?", b""),
            (r"[a-z]+", b"abc"),
            (r"[^a]+", b"bbb"),
            (r".", &[0xff]),
        ];
        for &(pattern, source) in cases {
            let expected = u32::from(regex::bytes::Regex::new(pattern).unwrap().is_match(source));
            let status = execute_generated_text(
                TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                },
                vec![b"match".to_vec(), pattern.as_bytes().to_vec()],
                source,
                16_384,
            );
            assert_eq!(status.error, 0, "regex {pattern:?} on {source:?}");
            assert_eq!(
                status.result_count, expected,
                "regex {pattern:?} on {source:?}"
            );
        }
    }

    #[test]
    fn cranelift_regex_uses_the_exact_metal_workspace_boundary() {
        use crate::code_rule_dsl::transducer::packed::{TextPredicate, value::ByteNfa};

        let pattern = r"^a(?:bc|d)+$";
        let required = ByteNfa::compile(pattern).unwrap().states.len() * 4;
        let predicate = || TextPredicate::Regex {
            source: 1,
            compiled: regex::Regex::new(pattern).unwrap(),
        };
        let strings = || vec![b"match".to_vec(), pattern.as_bytes().to_vec()];
        let status = execute_generated_text(predicate(), strings(), b"abcbcd", required - 1);
        assert_eq!(status.error, 2);
        let status = execute_generated_text(predicate(), strings(), b"abcbcd", required);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
        let status = execute_generated_text(predicate(), strings(), b"abcbcd", required + 1);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);
    }

    #[test]
    fn cranelift_text_lowers_regex_value_programs() {
        use crate::code_rule_dsl::transducer::packed::TextPredicate;

        for (pattern, source, expected) in [
            (r"^a(?:bc|d)+$", b"abcbcd".as_slice(), 1),
            (r"^a(?:bc|d)+$", b"abce".as_slice(), 0),
            (r"(?u)\bπ\b", " π ".as_bytes(), 1),
            (r"(?u)\bπ\b", "xπx".as_bytes(), 0),
        ] {
            let status = execute_generated_text(
                TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                },
                vec![b"match".to_vec(), pattern.as_bytes().to_vec()],
                source,
                4096,
            );
            assert_eq!(status.error, 0, "regex {pattern:?} on {source:?}");
            assert_eq!(
                status.result_count, expected,
                "regex {pattern:?} on {source:?}"
            );
        }
    }

    #[test]
    fn cranelift_text_lowers_equal_and_prefix_value_programs() {
        use crate::code_rule_dsl::transducer::packed::{TEXT, TextPredicate};

        for (predicate, source, expected) in [
            (TextPredicate::Equal(1), b"value".as_slice(), 1),
            (TextPredicate::Equal(1), b"values".as_slice(), 0),
            (TextPredicate::Prefix(1), b"values".as_slice(), 1),
            (TextPredicate::Prefix(1), b"val".as_slice(), 0),
        ] {
            let program = Program::new(
                vec![word(TEXT, 0, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec(), b"value".to_vec()],
                vec![predicate],
                vec![0],
                Vec::new(),
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_end: source.len() as u32,
                ..MetalNode::default()
            }];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: 1,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: source.as_ptr(),
                source_length: source.len() as u32,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: std::ptr::null_mut(),
                pending_count: 0,
                pending_quota_limit: 0,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 4,
                path_work_limit: 4,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: std::ptr::null_mut(),
                query_bucket_capacity: 0,
                query_members: std::ptr::null_mut(),
                query_member_count: 0,
                query_member_quota_limit: 0,
                queued_states: std::ptr::null_mut(),
                queued_state_quota_limit: 0,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
                derived: std::ptr::null(),
                derived_classes: 0,
                derived_words: 0,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, expected);
        }
    }

    #[test]
    fn cranelift_supertype_reads_checked_persisted_memberships() {
        use crate::code_rule_dsl::transducer::packed::SUPERTYPE;

        let program = Program::new(
            vec![word(SUPERTYPE, 0, 9), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            ..MetalNode::default()
        }];
        let memberships = [
            MetalSupertype { node: 0, kind: 9 },
            MetalSupertype { node: 0, kind: 9 },
        ];
        let malformed = [MetalSupertype { node: 1, kind: 9 }];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: memberships.as_ptr(),
            supertype_count: memberships.len() as u32,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 4,
            path_work_limit: 4,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);

        status = TaskStatus::default();
        context.current_steps = 0;
        context.supertype_count = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 0);

        status = TaskStatus::default();
        context.current_steps = 0;
        context.supertypes = malformed.as_ptr();
        context.supertype_count = 1;
        compiled.execute(&mut context);
        assert_eq!(status.error, 1);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn cranelift_absent_field_scans_the_checked_child_range() {
        use crate::code_rule_dsl::transducer::packed::ABSENT_FIELD;

        let program = Program::new(
            vec![word(ABSENT_FIELD, 0, 7), word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let mut nodes = [MetalNode {
            parent: NONE,
            next_any: NONE,
            next_named: NONE,
            ..MetalNode::default()
        }; 3];
        nodes[0].child_count = 2;
        nodes[1].field = 3;
        nodes[2].field = 7;
        nodes[2].flags = 0b1100;
        let children = [1_u32, 2];
        let mut results = [ResultRecord::default(); 1];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: nodes.len() as u32,
            children: children.as_ptr(),
            child_count: children.len() as u32,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 4,
            path_work_limit: 4,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: results.as_mut_ptr(),
            result_quota_limit: results.len() as u32,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 0);

        let mut without_field = nodes;
        without_field[0].child_count = 1;
        status = TaskStatus::default();
        context.nodes = without_field.as_ptr();
        context.current_steps = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 0);
        assert_eq!(status.result_count, 1);

        let mut malformed = without_field;
        malformed[0].child_start = 2;
        malformed[0].child_count = 1;
        status = TaskStatus::default();
        context.nodes = malformed.as_ptr();
        context.current_steps = 0;
        compiled.execute(&mut context);
        assert_eq!(status.error, 1);
        assert_eq!(status.result_count, 0);
    }

    #[test]
    fn cranelift_long_size_uses_the_verified_u64_constant() {
        use crate::code_rule_dsl::transducer::packed::SIZE;

        for (mode, expected_results) in [(0x80, 1), (0x81, 1), (0x82, 1)] {
            let program = Program::new(
                vec![word(SIZE, mode, 0), word(EMIT, 0, 0)],
                vec![b"match".to_vec()],
                Vec::new(),
                vec![0],
                vec![70_000],
            )
            .unwrap();
            let native = native_ir::lower(&program).unwrap();
            let compiled = compile_straight_line(&native).unwrap();
            let nodes = [MetalNode {
                parent: NONE,
                next_any: NONE,
                next_named: NONE,
                source_start: 1,
                source_end: 70_001,
                ..MetalNode::default()
            }];
            let mut results = [ResultRecord::default(); 1];
            let mut status = TaskStatus::default();
            let mut context = NativeTaskContext {
                nodes: nodes.as_ptr(),
                node_count: 1,
                children: std::ptr::null(),
                child_count: 0,
                supertypes: std::ptr::null(),
                supertype_count: 0,
                source: std::ptr::null(),
                source_length: 0,
                values: std::ptr::null(),
                value_count: 0,
                nfa_states: std::ptr::null(),
                nfa_state_count: 0,
                nfa_data: std::ptr::null(),
                nfa_data_count: 0,
                nfa_ranges: std::ptr::null(),
                nfa_range_count: 0,
                lua_instructions: std::ptr::null(),
                lua_instruction_count: 0,
                lua_members: std::ptr::null(),
                lua_member_count: 0,
                value_work: std::ptr::null_mut(),
                value_start: 0,
                value_capacity: 0,
                current_node: 0,
                start_node: 0,
                task_ordinal: 0,
                pending: std::ptr::null_mut(),
                pending_count: 0,
                pending_quota_limit: 0,
                current_steps: 0,
                current_captures: 0,
                current_cursors: 0,
                current_repeats: 0,
                current_following: 0,
                current_capture_hash: 0,
                current_cursor_hash: 0,
                current_repeat_hash: 0,
                task_work_limit: 4,
                path_work_limit: 4,
                captures: std::ptr::null(),
                cursors: std::ptr::null(),
                repeats: std::ptr::null(),
                query_buckets: std::ptr::null_mut(),
                query_bucket_capacity: 0,
                query_members: std::ptr::null_mut(),
                query_member_count: 0,
                query_member_quota_limit: 0,
                queued_states: std::ptr::null_mut(),
                queued_state_quota_limit: 0,
                greedy_entries: std::ptr::null_mut(),
                greedy_entry_quota_limit: 0,
                maximum_cursor_slot: 0,
                capture_count: 0,
                capture_quota_limit: 0,
                cursor_count: 0,
                cursor_quota_limit: 0,
                repeat_count: 0,
                repeat_quota_limit: 0,
                results: results.as_mut_ptr(),
                result_quota_limit: results.len() as u32,
                line_starts: std::ptr::null(),
                line_count: 0,
                final_captures: std::ptr::null_mut(),
                final_capture_capacity: 0,
                properties: std::ptr::null_mut(),
                property_quota_limit: 0,
                output_bytes: std::ptr::null_mut(),
                output_byte_quota_limit: 0,
                status: &mut status,
                derived: std::ptr::null(),
                derived_classes: 0,
                derived_words: 0,
            };
            compiled.execute(&mut context);
            assert_eq!(status.error, 0);
            assert_eq!(status.result_count, expected_results);
        }
    }

    #[test]
    fn straight_line_cranelift_reports_exact_result_capacity() {
        let program = Program::new(
            vec![word(EMIT, 0, 0)],
            vec![b"match".to_vec()],
            Vec::new(),
            vec![0],
            Vec::new(),
        )
        .unwrap();
        let native = native_ir::lower(&program).unwrap();
        let compiled = compile_straight_line(&native).unwrap();
        let nodes = [MetalNode::default()];
        let mut status = TaskStatus::default();
        let mut context = NativeTaskContext {
            nodes: nodes.as_ptr(),
            node_count: 1,
            children: std::ptr::null(),
            child_count: 0,
            supertypes: std::ptr::null(),
            supertype_count: 0,
            source: std::ptr::null(),
            source_length: 0,
            values: std::ptr::null(),
            value_count: 0,
            nfa_states: std::ptr::null(),
            nfa_state_count: 0,
            nfa_data: std::ptr::null(),
            nfa_data_count: 0,
            nfa_ranges: std::ptr::null(),
            nfa_range_count: 0,
            lua_instructions: std::ptr::null(),
            lua_instruction_count: 0,
            lua_members: std::ptr::null(),
            lua_member_count: 0,
            value_work: std::ptr::null_mut(),
            value_start: 0,
            value_capacity: 0,
            current_node: 0,
            start_node: 0,
            task_ordinal: 0,
            pending: std::ptr::null_mut(),
            pending_count: 0,
            pending_quota_limit: 0,
            current_steps: 0,
            current_captures: 0,
            current_cursors: 0,
            current_repeats: 0,
            current_following: 0,
            current_capture_hash: 0,
            current_cursor_hash: 0,
            current_repeat_hash: 0,
            task_work_limit: 16,
            path_work_limit: 16,
            captures: std::ptr::null(),
            cursors: std::ptr::null(),
            repeats: std::ptr::null(),
            query_buckets: std::ptr::null_mut(),
            query_bucket_capacity: 0,
            query_members: std::ptr::null_mut(),
            query_member_count: 0,
            query_member_quota_limit: 0,
            queued_states: std::ptr::null_mut(),
            queued_state_quota_limit: 0,
            greedy_entries: std::ptr::null_mut(),
            greedy_entry_quota_limit: 0,
            maximum_cursor_slot: 0,
            capture_count: 0,
            capture_quota_limit: 0,
            cursor_count: 0,
            cursor_quota_limit: 0,
            repeat_count: 0,
            repeat_quota_limit: 0,
            results: std::ptr::null_mut(),
            result_quota_limit: 0,
            line_starts: std::ptr::null(),
            line_count: 0,
            final_captures: std::ptr::null_mut(),
            final_capture_capacity: 0,
            properties: std::ptr::null_mut(),
            property_quota_limit: 0,
            output_bytes: std::ptr::null_mut(),
            output_byte_quota_limit: 0,
            status: &mut status,
            derived: std::ptr::null(),
            derived_classes: 0,
            derived_words: 0,
        };
        compiled.execute(&mut context);
        assert_eq!(status.error, 2);
        assert_eq!(status.result_count, 0);
    }
    #[test]
    fn strip_transform_work_is_owned_by_each_candidate_across_captures_and_searches() {
        use crate::code_rule_dsl::transducer::packed::{Directive, value::ByteNfa};
        let pattern = "^# a+$";
        let nfa = ByteNfa::compile(pattern).unwrap();
        let (source, cost) = [40_000, 80_000, 120_000, 160_000]
            .into_iter()
            .find_map(|length| {
                let source = format!("# {}", "a".repeat(length));
                let (found, first) = nfa.test_find_work_count_from(source.as_bytes(), 0);
                assert_eq!(found, Some([0, source.len() as u32]));
                let (found, last) = nfa.test_find_work_count_from(source.as_bytes(), source.len());
                assert_eq!(found, None);
                let cost = (first + last) * 2;
                (cost > super::super::MAX_STEPS_PER_PATH / 2
                    && cost < super::super::MAX_STEPS_PER_PATH - 128)
                    .then_some((source, cost))
            })
            .expect("two capture searches fit one budget but two candidates do not");
        let files = [tape(&format!("{source}\n{source}\n"))];
        let anchors = [vec![vec![0]]];
        for query in [false, true] {
            let base = Program::new(
                vec![
                    word(super::super::CHILD, 1, 0),
                    word(CAPTURE, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![Directive::Strip {
                capture: 0,
                source: 1,
                compiled: regex::Regex::new(pattern).unwrap(),
            }])
            .unwrap();
            let programs = [if query {
                base.with_tree_sitter_query_semantics().unwrap()
            } else {
                base
            }];
            let expected =
                crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                    .unwrap();
            assert_eq!(expected[0][0].len(), 2);
            for (index, output) in expected[0][0].iter().enumerate() {
                assert_eq!(output.start, 0);
                assert_eq!(output.captures[0].node, index as u32 + 1);
                assert_eq!(output.captures.len(), 2);
                assert_eq!(output.captures[0], output.captures[1]);
                assert_eq!(output.captures[0].text.as_deref(), Some(b"".as_slice()));
            }
            for limit in [cost - 1, cost, cost + 1] {
                let actual = execute_batch_packed_with_work_limit(
                    &programs,
                    &files,
                    &anchors,
                    None,
                    limit as u32,
                )
                .and_then(PackedExecution::into_outcome)
                .and_then(BatchOutcome::into_result);
                if limit < cost {
                    assert_eq!(actual, Err(Error::WorkLimit));
                } else {
                    assert_eq!(
                        actual,
                        Ok(expected.clone()),
                        "query={query}, limit={limit}, candidate cost={cost}"
                    );
                }
            }
        }
    }
    #[test]
    fn strip_searches_use_full_input_nfa_work_at_every_budget_boundary() {
        use crate::code_rule_dsl::transducer::packed::{Directive, value::ByteNfa};
        for (pattern, source) in [
            ("n", "banana"),
            ("a|aa", "aaab"),
            ("aa|a", "aaab"),
            ("", "é😀"),
            (r"\b", "ab é"),
            ("a*", "aaab"),
            ("^a|b", "aaab"),
        ] {
            let nfa = ByteNfa::compile(pattern).unwrap();
            let mut cost = 0;
            let mut search = 0;
            loop {
                let (found, used) = nfa.test_find_work_count_from(source.as_bytes(), search);
                cost += used;
                let Some([start, end]) = found else { break };
                search = end as usize;
                if start == end {
                    if search == source.len() {
                        break;
                    }
                    search += source[search..].chars().next().unwrap().len_utf8();
                }
            }
            cost *= 2;
            let programs = [Program::new(
                vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![Directive::Strip {
                capture: 0,
                source: 1,
                compiled: regex::Regex::new(pattern).unwrap(),
            }])
            .unwrap()];
            let files = [tape(source)];
            let anchors = [vec![vec![0]]];
            let expected =
                crate::code_rule_dsl::transducer::packed::execute_batch(&programs, &files, Some(&anchors))
                    .unwrap();
            assert_eq!(expected[0][0].len(), 1);
            assert_eq!(expected[0][0][0].captures.len(), 2);
            assert_eq!(expected[0][0][0].captures[0], expected[0][0][0].captures[1]);
            for limit in [cost - 1, cost, cost + 1] {
                let actual = execute_batch_packed_with_work_limit(
                    &programs,
                    &files,
                    &anchors,
                    None,
                    limit as u32,
                )
                .and_then(PackedExecution::into_outcome)
                .and_then(BatchOutcome::into_result);
                if limit < cost {
                    assert_eq!(
                        actual,
                        Err(Error::WorkLimit),
                        "pattern={pattern:?}, cost={cost}, limit={limit}"
                    );
                } else {
                    assert_eq!(
                        actual,
                        Ok(expected.clone()),
                        "pattern={pattern:?}, cost={cost}, limit={limit}"
                    );
                }
            }
        }
    }
