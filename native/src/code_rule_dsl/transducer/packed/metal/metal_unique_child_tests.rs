//! Exact complete-work bounds come from the immutable tree, not guessed fan-out.
use super::*;
use crate::policy_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::{LanguageExt, SupportLang};

fn tape(source: &str) -> TapeDoc {
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}

fn assert_outputs(actual: &[Vec<Vec<Output>>], expected: &[Vec<Vec<Output>>]) {
    assert_eq!(actual.len(), expected.len());
    for (file, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(actual.len(), expected.len(), "file={file}");
        for (program, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                actual.len(),
                expected.len(),
                "file={file}, program={program}"
            );
            for (row, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(
                    actual, expected,
                    "file={file}, program={program}, row={row}"
                );
            }
        }
    }
}

fn assert_complete(
    programs: &[Program],
    file: TapeDoc,
    selected: Vec<Vec<u32>>,
    counts: &[usize],
) -> usize {
    let files = [file];
    let anchors = [selected];
    let expected = execute_batch(programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0].iter().map(Vec::len).collect::<Vec<_>>(), counts);
    let scope = metal_backend::launch::LaunchScope::enter();
    let started = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, programs, &files, &anchors);
    let trace = scope.snapshot();
    eprintln!(
        "UNIQUE_CHILD_EVIDENCE {}",
        serde_json::json!({
            "programs": programs.len(), "roots": anchors[0].iter().map(Vec::len).sum::<usize>(),
            "seconds": started.elapsed().as_secs_f64(), "launches": trace.len(),
            "peakMetalBytes": trace.iter().map(|r| r.peak_application_buffer_bytes).max(),
            "error": format!("{:?}", actual.as_ref().err()),
            "largestLive": trace.iter().max_by_key(|record|record.application_buffer_bytes),
        })
    );
    let actual = actual.expect(
        "bounded child joins must complete under the original allocation and launch guards",
    );
    assert_outputs(&actual, &expected);
    assert!(!trace.is_empty() && trace.len() <= 600);
    assert!(trace.iter().all(|r| r.encoded
        && r.submitted
        && r.completed
        && r.peak_application_buffer_bytes <= 2_000_000_000));
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "register_roots")
            .count(),
        1
    );
    trace
        .iter()
        .map(|record| record.peak_application_buffer_bytes)
        .max()
        .unwrap()
}

pub(super) fn initial_child_prefix_program(mode: u8) -> Program {
    let language = SupportLang::Python.get_ts_language();
    let call = language.id_for_node_kind("call", true);
    let identifier = language.id_for_node_kind("identifier", true);
    Program::new(
        vec![
            word(KIND, 0, call),
            word(REPEAT_BEGIN, 0, 9),
            word(SAVE_CURSOR, 0, 0),
            word(CHILD, mode, 0),
            word(
                FIELD,
                0,
                language.field_id_for_name("function").unwrap().get(),
            ),
            word(KIND, 0, language.id_for_node_kind("attribute", true)),
            word(SAVE_CURSOR, 0, 1),
            word(CHILD, mode, 0),
            word(
                FIELD,
                0,
                language.field_id_for_name("object").unwrap().get(),
            ),
            word(KIND, 0, identifier),
            word(CAPTURE, 0, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(CHILD, mode, 0),
            word(
                FIELD,
                0,
                language.field_id_for_name("arguments").unwrap().get(),
            ),
            word(KIND, 0, language.id_for_node_kind("argument_list", true)),
            word(CHILD, mode, 0),
            word(KIND, 0, identifier),
            word(CAPTURE, 0, 1),
            word(REPEAT_END, 0, 9),
            word(EMIT, 0, 0),
        ],
        vec![b"object".to_vec(), b"argument".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
}

#[test]
fn initial_ranges_preserve_named_descendant_sibling_and_restore_semantics() {
    let language = SupportLang::Python.get_ts_language();
    for op in [CHILD, DESCEND, FOLLOWING] {
        for mode in [NAMED, 0] {
            let file = tape(&"f(a,b,(c+d),e)\n".repeat(8));
            let kind = language.id_for_node_kind(
                if op == FOLLOWING {
                    "identifier"
                } else {
                    "argument_list"
                },
                true,
            );
            let roots = (0..file.transducer_len() as u32)
                .filter(|&node| {
                    file.transducer_kind(node) == kind
                        && (op != FOLLOWING || file.transducer_text(node) == b"a")
                })
                .collect::<Vec<_>>();
            assert_eq!(roots.len(), 8);
            let program = Program::new(
                vec![
                    word(REPEAT_BEGIN, 0, 9),
                    word(SAVE_CURSOR, 0, 0),
                    word(op, mode, 0),
                    word(FLAGS, NAMED, 0),
                    word(CAPTURE, 0, 0),
                    word(REPEAT_END, 0, 9),
                    word(EMIT, 0, 0),
                ],
                vec![b"node".to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap();
            let count = match op {
                CHILD => 32,
                DESCEND => 56,
                FOLLOWING => 24,
                _ => unreachable!(),
            };
            assert_complete(&[program], file, vec![roots], &[count]);
        }
    }
    for mode in [NAMED, 0] {
        let file = tape(&"f(a,b,(c+d),e)\n".repeat(8));
        let kind = language.id_for_node_kind("argument_list", true);
        let roots = (0..file.transducer_len() as u32)
            .filter(|&node| file.transducer_kind(node) == kind)
            .collect::<Vec<_>>();
        let program = Program::new(
            vec![
                word(REPEAT_BEGIN, 0, 9),
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, mode, 0),
                word(FLAGS, NAMED, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(CAPTURE, 0, 0),
                word(REPEAT_END, 0, 9),
                word(EMIT, 0, 0),
            ],
            vec![b"parent".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap();
        assert_complete(&[program], file, vec![roots], &[8]);
    }
}

#[test]
fn initial_child_ranges_preserve_every_occurrence_without_full_candidate_workspaces() {
    let language = SupportLang::Python.get_ts_language();
    for mode in [NAMED, 0] {
        let file = tape(&"f(a,b,c,d)\n".repeat(512));
        let arguments = language.id_for_node_kind("argument_list", true);
        let roots = (0..file.transducer_len() as u32)
            .filter(|&node| file.transducer_kind(node) == arguments)
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 512);
        let program = Program::new(
            vec![
                word(REPEAT_BEGIN, 0, 9),
                word(SAVE_CURSOR, 0, 0),
                word(CHILD, mode, 0),
                word(FLAGS, NAMED, 0),
                word(CAPTURE, 0, 0),
                word(REPEAT_END, 0, 9),
                word(EMIT, 0, 0),
            ],
            vec![b"argument".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap();
        let truth = execute_batch(
            std::slice::from_ref(&program),
            std::slice::from_ref(&file),
            Some(&[vec![roots.clone()]]),
        )
        .unwrap();
        assert_eq!(truth[0][0].len(), 2048);
        for (i, row) in truth[0][0].iter().enumerate() {
            assert_eq!(row.captures.len(), 1);
            assert_eq!(row.captures[0].label, 0);
            assert_eq!(
                file.transducer_text(row.captures[0].node),
                [b"a", b"b", b"c", b"d"][i % 4]
            );
        }
        let peak = assert_complete(&vec![program; 64], file, vec![roots; 64], &[2048; 64]);
        assert!(
            peak < 64 * 1024 * 1024,
            "bounded range work must not allocate complete intermediate records for potential child rows: {peak}"
        );
    }
}

#[test]
fn initial_first_child_prefix_keeps_the_first_named_occurrence_in_a_wide_list() {
    let source = format!("f({})\n", vec!["v"; 8193].join(","));
    let language = SupportLang::Python.get_ts_language();
    for mode in [NAMED, 0] {
        let file = tape(&source);
        let arguments = language.id_for_node_kind("argument_list", true);
        let roots = (0..file.transducer_len() as u32)
            .filter(|&node| file.transducer_kind(node) == arguments)
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 1);
        let program = Program::new(
            vec![
                word(KIND, 0, arguments),
                word(REPEAT_BEGIN, 0, 9),
                word(CHILD, mode, 0),
                word(FIRST, NAMED, 0),
                word(KIND, 0, language.id_for_node_kind("identifier", true)),
                word(CAPTURE, 0, 0),
                word(REPEAT_END, 0, 9),
                word(EMIT, 0, 0),
            ],
            vec![b"first".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap();
        let truth = execute_batch(
            std::slice::from_ref(&program),
            std::slice::from_ref(&file),
            Some(&[vec![roots.clone()]]),
        )
        .unwrap();
        assert_eq!(truth[0][0].len(), 1);
        assert_eq!(truth[0][0][0].captures.len(), 1);
        assert_eq!(truth[0][0][0].captures[0].span, 2..3);
        assert_eq!(truth[0][0][0].captures[0].label, 0);
        assert_eq!(file.transducer_text(truth[0][0][0].captures[0].node), b"v");
        let peak = assert_complete(&vec![program; 64], file, vec![roots; 64], &[1; 64]);
        assert!(
            peak < 16 * 1024 * 1024,
            "a proved first-child prefix must not reserve full state for every later sibling: {peak}"
        );
    }
}

#[test]
fn initial_unique_child_prefixes_keep_captures_without_replicating_failed_siblings() {
    let source = "f().target(v)\n".repeat(2048) + "a.target(v)\n";
    let file = tape(&source);
    let language = SupportLang::Python.get_ts_language();
    let call = language.id_for_node_kind("call", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| {
            file.transducer_kind(node) == call
                && file.transducer_text(node).ends_with(b".target(v)")
        })
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 2049);
    let program = initial_child_prefix_program(NAMED);
    let truth = execute_batch(
        std::slice::from_ref(&program),
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone()]]),
    )
    .unwrap();
    let truth = &truth[0][0];
    assert_eq!(truth.len(), 1);
    assert_eq!(truth[0].captures.len(), 2);
    for (capture, label, text) in [
        (&truth[0].captures[0], 0, b"a".as_slice()),
        (&truth[0].captures[1], 1, b"v".as_slice()),
    ] {
        assert_eq!(capture.label, label);
        assert_eq!(file.transducer_text(capture.node), text);
        assert!(file.transducer_span(capture.node).start >= source.len() - "a.target(v)\n".len());
    }
    let peak = assert_complete(&vec![program; 64], file, vec![roots; 64], &[1; 64]);
    assert!(
        peak < 16 * 1024 * 1024,
        "roots proved to have no result must not reserve initial prefix frames: {peak}"
    );
}

#[test]
fn initial_eligibility_preserves_semantic_failures_before_a_later_false_guard() {
    let language = SupportLang::Python.get_ts_language();
    let call = language.id_for_node_kind("call", true);
    for op in [RESTORE_CURSOR, REPEAT_BUMP, REPEAT_END] {
        for insertion in [9, 10] {
            let file = tape("f().target(v)\na.target(v)\n");
            let root = (0..file.transducer_len() as u32)
                .find(|&node| {
                    file.transducer_kind(node) == call
                        && file.transducer_text(node) == b"f().target(v)"
                })
                .unwrap();
            let mut program = initial_child_prefix_program(NAMED);
            program.words.insert(
                insertion,
                word(op, 0, if op == RESTORE_CURSOR { 7 } else { 8 }),
            );
            program.validate().unwrap();
            let programs = [program];
            let files = [file];
            let anchors = [vec![vec![root]]];
            let expected = execute_batch(&programs, &files, Some(&anchors));
            if insertion == 9 {
                assert!(
                    matches!(expected, Err(Error::InvalidOperand(_))),
                    "the fixture must reach its semantic failure before the identifier guard: {expected:?}"
                );
            } else {
                assert_eq!(
                    expected,
                    Ok(vec![vec![vec![]]]),
                    "the identifier guard must stop before the semantic failure"
                );
            }
            let scope = metal_backend::launch::LaunchScope::enter();
            let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
            assert_eq!(actual, expected, "opcode={op}, insertion={insertion}");
            assert!(scope.snapshot().iter().all(|record| record.completed));
        }
    }
}

#[test]
fn bounded_child_fields_complete_two_million_matches_without_frontier_replication() {
    let source = "def f():\n    pass\n".repeat(32768);
    let file = tape(&source);
    let language = SupportLang::Python.get_ts_language();
    let kind = language.id_for_node_kind("function_definition", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == kind)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 32768);
    let program = Program::compile_tree_sitter_query_pattern(
        "(function_definition body: (block) @body)",
        &language,
    )
    .unwrap();
    assert_complete(&vec![program; 64], file, vec![roots; 64], &[32768; 64]);
}

#[test]
fn initial_child_guards_prune_nonempty_mismatches_before_successor_reservation() {
    let source = "f(1)\n".repeat(32768) + "if flag:\n    pass\n";
    let file = tape(&source);
    let language = SupportLang::Python.get_ts_language();
    let call = language.id_for_node_kind("call", true);
    let conditional = language.id_for_node_kind("if_statement", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| matches!(file.transducer_kind(node), kind if kind == call || kind == conditional))
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 32769);
    let program = Program::new(
        vec![
            word(SAVE_CURSOR, 0, 0),
            word(CAPTURE, 0, 0),
            word(CHILD, NAMED, 0),
            word(KIND, 0, language.id_for_node_kind("block", true)),
            word(DESCEND, NAMED, 0),
            word(CAPTURE, 0, 1),
            word(RESTORE_CURSOR, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"parent".to_vec(), b"descendant".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    let peak = assert_complete(&vec![program; 64], file, vec![roots; 64], &[1; 64]);
    assert!(
        peak < 32 * 1024 * 1024,
        "64 active roots must not retain bookkeeping for two million rejected tasks: {peak}"
    );
}

#[test]
fn terminal_repeat_storage_excludes_completed_register_tasks_and_preserves_witness_limits() {
    let file = tape(&"# node\n".repeat(256));
    let language = SupportLang::Python.get_ts_language();
    let direct = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"direct".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let repeated =
        Program::compile_tree_sitter_query_pattern("(comment)+ @run", &language).unwrap();
    let mut programs = vec![direct; 8192];
    programs.push(repeated);
    let files = [file];
    let mut selected = vec![(1..=256).collect::<Vec<_>>(); 8192];
    selected.push(vec![1]);
    let anchors = [selected];
    let mut expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0].last().unwrap().len(), 1);
    assert_eq!(expected[0].last().unwrap()[0].captures.len(), 256);
    for row in &mut expected[0][..8192] {
        row.clear();
    }
    let mut limits = vec![0; 8192];
    limits.push(1);
    let resident = metal_grid_plan::ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &anchors).unwrap(),
        Some(&limits),
    )
    .unwrap();
    let plan = metal_grid_plan::ExecutionPlan::new(&resident, &programs).unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let actual = metal_grid_executor::GuardedExecutor::new(resident, plan)
        .unwrap()
        .execute()
        .and_then(|output| output.into_outcome())
        .and_then(|output| output.into_result())
        .unwrap();
    assert_outputs(&actual, &expected);
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .all(|record| record.encoded && record.submitted && record.completed)
    );
    assert!(trace.len() <= 600);
    let peak = trace
        .iter()
        .map(|record| record.peak_application_buffer_bytes)
        .max()
        .unwrap();
    eprintln!(
        "SPARSE_REPEAT_EVIDENCE {}",
        serde_json::json!({"peakMetalBytes":peak,"launches":trace.len()})
    );
    assert!(
        peak < 180 * 1024 * 1024,
        "one repeated root must not reserve repeat descriptors for two million completed register tasks: {peak}"
    );
}

#[test]
fn unique_and_repeated_child_fields_preserve_all_occurrences_in_one_evaluation() {
    let source = "import a, b, c\nf(x)\n";
    let file = tape(source);
    let language = SupportLang::Python.get_ts_language();
    let mut programs = [
        "(call function: (identifier) @function)",
        "(import_statement name: (dotted_name) @part)",
    ]
    .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
    .to_vec();
    programs.push(
        Program::new(
            vec![
                word(KIND, 0, language.id_for_node_kind("argument_list", true)),
                word(CHILD, 0, 0),
                word(FIELD, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"unfielded".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap(),
    );
    let roots = (0..file.transducer_len() as u32).collect::<Vec<_>>();
    assert_complete(&programs, file, vec![roots; 3], &[1, 3, 3]);
}

#[test]
fn repeated_initial_program_shapes_preserve_runtime_operands_and_complete_outputs() {
    let file = tape("f(a,b)\n");
    let language = SupportLang::Python.get_ts_language();
    let arguments = language.id_for_node_kind("argument_list", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == arguments)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    let programs = (0..8192)
        .map(|index| {
            let mut words = vec![word(REPEAT_BEGIN, 0, 9)];
            if index % 3 == 0 {
                words.push(word(SAVE_CURSOR, 0, 0));
            }
            words.extend([
                word(CHILD, NAMED, 0),
                word(FLAGS, NAMED, 0),
                word(CAPTURE, 0, index % 2),
            ]);
            if index % 3 != 0 {
                words.push(word(CAPTURE, 0, 1 - index % 2));
            }
            words.extend([word(REPEAT_END, 0, 9), word(EMIT, 0, 0)]);
            Program::new(
                words,
                vec![b"first".to_vec(), b"second".to_vec()],
                vec![],
                vec![0, 1],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let anchors = vec![roots; programs.len()];
    let truth = execute_batch(
        &programs,
        std::slice::from_ref(&file),
        Some(std::slice::from_ref(&anchors)),
    )
    .unwrap();
    for (program, rows) in truth[0].iter().enumerate() {
        assert_eq!(rows.len(), 2);
        for (child, row) in rows.iter().enumerate() {
            assert_eq!(row.captures.len(), if program % 3 == 0 { 1 } else { 2 });
            for (at, capture) in row.captures.iter().enumerate() {
                assert_eq!(capture.label as usize, (program + at) % 2);
                assert_eq!(file.transducer_text(capture.node), [b"a", b"b"][child]);
            }
        }
    }
    assert_complete(&programs, file, anchors, &vec![2; programs.len()]);
}

#[test]
fn completed_finite_programs_preserve_all_results_without_later_successor_workspaces() {
    let source = "f(a,b,c,d)\n".repeat(512);
    let file = tape(&source);
    let language = SupportLang::Python.get_ts_language();
    let arguments = language.id_for_node_kind("argument_list", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == arguments)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 512);
    let body = vec![
        word(REPEAT_BEGIN, 0, 9),
        word(SAVE_CURSOR, 0, 0),
        word(CHILD, NAMED, 0),
        word(FLAGS, NAMED, 0),
        word(CAPTURE, 0, 0),
        word(REPEAT_END, 0, 9),
        word(EMIT, 0, 0),
    ];
    let make = |words| {
        Program::new(words, vec![b"value".to_vec()], vec![], vec![0], vec![])
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
    };
    let finite = make(body.clone());
    let mut continuation = vec![word(GOTO, 0, 2), word(STOP, 0, 0)];
    continuation.extend(body);
    let continuation = make(continuation);
    for program in [&finite, &continuation] {
        let truth = execute_batch(
            std::slice::from_ref(program),
            std::slice::from_ref(&file),
            Some(&[vec![roots.clone()]]),
        )
        .unwrap();
        assert_eq!(truth[0][0].len(), 2048);
        for (ordinal, output) in truth[0][0].iter().enumerate() {
            assert_eq!(output.captures.len(), 1);
            assert_eq!(output.captures[0].label, 0);
            assert_eq!(
                file.transducer_text(output.captures[0].node),
                [b"a".as_slice(), b"b", b"c", b"d"][ordinal % 4]
            );
        }
    }
    let programs = vec![finite; 64]
        .into_iter()
        .chain(vec![continuation; 64])
        .collect::<Vec<_>>();
    let peak = assert_complete(&programs, file, vec![roots; 128], &[2048; 128]);
    assert!(
        peak < 80 * 1024 * 1024,
        "completed finite output must not overlap later successor workspaces: {peak}"
    );
}

#[test]
fn rejected_bounded_variants_preserve_complete_outputs_without_retaining_failed_frames() {
    let file = tape(&"# match\nf()\n".repeat(128));
    let language = SupportLang::Python.get_ts_language();
    let comment = language.id_for_node_kind("comment", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == comment)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 128);
    // Each repetition begins at a comment. Its next named sibling is an
    // expression statement, so every nonempty bounded body variant fails.
    let p = Program::new(
        vec![
            word(GOTO, 0, 2),
            word(STOP, 0, 0),
            word(REPEAT_BEGIN, 0, 9),
            word(KIND, 0, comment),
            word(CAPTURE, 0, 0),
            word(REPEAT_BUMP, 0, 9),
            word(FORK, 0, 12),
            word(NEXT, NAMED, 0),
            word(KIND, 0, comment),
            word(CAPTURE, 0, 0),
            word(REPEAT_BUMP, 0, 9),
            word(GOTO, 0, 6),
            word(REPEAT_END, 0, 9),
            word(EMIT, 0, 0),
        ],
        vec![b"run".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let expected = execute_batch(
        std::slice::from_ref(&p),
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone()]]),
    )
    .unwrap();
    assert_eq!(expected[0][0].len(), 128);
    for (&node, output) in roots.iter().zip(&expected[0][0]) {
        assert_eq!(
            output.captures,
            vec![Capture {
                node,
                label: 0,
                span: file.transducer_span(node),
                text: None,
                properties: vec![]
            }]
        );
        assert_eq!(file.transducer_text(node), b"# match");
    }
    let peak = assert_complete(&vec![p; 64], file, vec![roots; 64], &[128; 64]);
    assert!(
        peak < 64 * 1024 * 1024,
        "failed bounded variants must retire before finite frame allocation: {peak}"
    );
}

#[test]
fn rejected_bounded_register_alternatives_preserve_results_without_maximum_output_reservation() {
    let file = tape(&"# match\n".repeat(512));
    let language = SupportLang::Python.get_ts_language();
    let comment = language.id_for_node_kind("comment", true);
    let identifier = language.id_for_node_kind("identifier", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == comment)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 512);
    // Every earlier alternative fails its real kind guard. The last one must
    // retain the full ordered branch position and its original capture value.
    let mut words = vec![word(GOTO, 0, 2), word(STOP, 0, 0)];
    for _ in 0..31 {
        let next = u16::try_from(words.len() + 4).unwrap();
        words.extend([
            word(FORK, 0, next),
            word(KIND, 0, identifier),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ]);
    }
    words.extend([
        word(KIND, 0, comment),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ]);
    let p = Program::new(words, vec![b"last".to_vec()], vec![], vec![0], vec![])
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap();
    let expected = execute_batch(
        std::slice::from_ref(&p),
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone()]]),
    )
    .unwrap();
    assert_eq!(expected[0][0].len(), roots.len());
    for (&node, output) in roots.iter().zip(&expected[0][0]) {
        assert_eq!(
            output.captures,
            vec![Capture {
                node,
                label: 0,
                span: file.transducer_span(node),
                text: None,
                properties: vec![]
            }]
        );
        assert_eq!(file.transducer_text(node), b"# match");
    }
    let peak = assert_complete(&vec![p; 64], file, vec![roots; 64], &[512; 64]);
    assert!(
        peak < 64 * 1024 * 1024,
        "register output storage must follow completed survivor counts: {peak}"
    );
}

#[test]
fn successor_expansion_preserves_named_occurrences_without_eager_candidate_value_arenas() {
    let file = tape(&"f(a,b,c,d,e,f,g,h)\n".repeat(256));
    let language = SupportLang::Python.get_ts_language();
    let arguments = language.id_for_node_kind("argument_list", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == arguments)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 256);
    let p = Program::new(
        vec![
            word(GOTO, 0, 2),
            word(STOP, 0, 0),
            word(CHILD, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"value".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let expected = execute_batch(
        std::slice::from_ref(&p),
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone()]]),
    )
    .unwrap();
    assert_eq!(expected[0][0].len(), 2048);
    for (ordinal, output) in expected[0][0].iter().enumerate() {
        assert_eq!(output.captures.len(), 1);
        let capture = &output.captures[0];
        assert_eq!(capture.label, 0);
        assert_eq!(capture.span, file.transducer_span(capture.node));
        assert_eq!(capture.text, None);
        assert!(capture.properties.is_empty());
        assert_eq!(
            file.transducer_text(capture.node),
            [b"a".as_slice(), b"b", b"c", b"d", b"e", b"f", b"g", b"h"][ordinal % 8]
        );
    }
    let peak = assert_complete(&vec![p; 64], file, vec![roots; 64], &[2048; 64]);
    assert!(
        peak < 64 * 1024 * 1024,
        "successor value storage must follow complete surviving candidates: {peak}"
    );
}

#[test]
fn admitted_successor_guards_reject_losing_children_before_publication_allocation() {
    let file = tape(&"f(a,bbbb,cccc,dddd,eeee,ffff,gggg,hhhh)\n".repeat(512));
    let language = SupportLang::Python.get_ts_language();
    let arguments = language.id_for_node_kind("argument_list", true);
    let roots = (0..file.transducer_len() as u32)
        .filter(|&node| file.transducer_kind(node) == arguments)
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 512);
    let p = Program::new(
        vec![
            word(GOTO, 0, 2),
            word(STOP, 0, 0),
            word(CHILD, NAMED, 0),
            word(SIZE, 0, 1),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"value".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let expected = execute_batch(
        std::slice::from_ref(&p),
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone()]]),
    )
    .unwrap();
    assert_eq!(expected[0][0].len(), 512);
    for output in &expected[0][0] {
        assert_eq!(output.captures.len(), 1);
        assert_eq!(output.captures[0].label, 0);
        assert_eq!(file.transducer_text(output.captures[0].node), b"a");
    }
    let peak = assert_complete(&vec![p; 64], file, vec![roots; 64], &[512; 64]);
    assert!(
        peak < 32 * 1024 * 1024,
        "read-only successor guards must reject losing children before output allocation: {peak}"
    );
}

#[test]
fn optional_function_body_views_preserve_the_saved_name_capture() {
    let language = SupportLang::Python.get_ts_language();
    let p=Program::compile_tree_sitter_query_pattern("((function_definition name: (identifier) @_name body: (block)? @test.inside) @test.around (#match? @_name \"^test_\"))",&language).unwrap();
    let _views = metal_dynamic_execution::ViewConformanceScope::enter();
    for count in [1, 65, 4097] {
        let source = (0..count)
            .map(|i| format!("def test_value_{i}():\n    pass\n"))
            .collect::<String>();
        let file = tape(&source);
        let kind = language.id_for_node_kind("function_definition", true);
        let roots = (0..file.transducer_len() as u32)
            .filter(|&node| file.transducer_kind(node) == kind)
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), count);
        assert_complete(std::slice::from_ref(&p), file, vec![roots], &[count]);
    }
}
