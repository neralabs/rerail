//! Complete production results, including routes that used to stop after one wave.
use super::*;

fn closure() -> Program {
    Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 7),
            word(NEXT, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 3),
            word(REPEAT_END, 0, 0),
            word(LAST, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"last".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
}

fn parity(programs: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>]) -> (usize, usize) {
    let expected = execute_batch(programs, files, Some(anchors)).unwrap();
    let scope = LaunchScope::enter();
    let start = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, programs, files, anchors);
    let trace = scope.snapshot();
    eprintln!(
        "PRODUCTION_DYNAMIC {}",
        serde_json::json!({
            "files":files.len(), "programs":programs.len(), "seconds":start.elapsed().as_secs_f64(),
            "launches":trace.len(), "peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),
            "kernels":trace.iter().map(|r|&r.kernel).collect::<Vec<_>>()
        })
    );
    assert_eq!(actual, Ok(expected));
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(trace.len() <= 600);
    assert_eq!(
        trace
            .iter()
            .filter(|r| r.kernel == "register_roots")
            .count(),
        1
    );
    let seeds = trace.iter().filter(|r| r.kernel == "seed_scatter").count();
    let repeats = trace
        .iter()
        .filter(|r| r.kernel == "repeat_root_counts")
        .count();
    assert!(seeds <= 1 && repeats <= 1 && seeds + repeats >= 1);
    assert!(trace.iter().all(
        |r| r.peak_application_buffer_bytes <= super::super::metal_backend::METAL_BUFFER_LIMIT
    ));
    (
        trace.len(),
        trace
            .iter()
            .filter(|r| r.kernel == "successor_publish")
            .count(),
    )
}

#[test]
fn closure_publishes_the_last_sibling_after_complete_reconstruction() {
    let mut launches = Vec::new();
    for siblings in [1, 130, 4096] {
        let (_, file, _) = convergence_case(siblings.max(2));
        let p = closure();
        let expected = p.execute_roots(&file, [1]).unwrap();
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].captures.len(), 1);
        assert_eq!(expected[0].captures[0].node, 1);
        launches.push(parity(&[p], &[file], &[vec![vec![1]]]));
    }
    assert!(
        launches
            .iter()
            .all(|&(_, generations)| generations == launches[0].1),
        "chain length must not add generations"
    );
}

fn independent_closures(tasks: usize) {
    let (_, file, _) = convergence_case(3);
    parity(
        &[closure()],
        &vec![file; tasks],
        &vec![vec![vec![1]]; tasks],
    );
}
#[test]
fn closure_completes_128_independent_tasks() {
    independent_closures(128);
}
#[test]
fn closure_completes_4096_independent_tasks() {
    independent_closures(4096);
}

#[test]
fn mixed_register_and_dynamic_programs_keep_complete_file_program_root_order() {
    let (converging, file, _) = convergence_case(3);
    let ordinary =
        |words| Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
    let programs = [
        ordinary(vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)]),
        converging,
        ordinary(vec![
            word(NEXT, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ]),
        closure(),
        ordinary(vec![
            word(FOLLOWING, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ]),
    ];
    let roots = vec![vec![1, 2, 3]; programs.len()];
    parity(&programs, &[file.clone(), file], &[roots.clone(), roots]);
}

#[test]
fn ordinary_dynamic_expansion_preserves_duplicate_captures_and_empty_results() {
    let (_, file, _) = convergence_case(3);
    let p = Program::new(
        vec![
            word(FOLLOWING, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    parity(
        std::slice::from_ref(&p),
        std::slice::from_ref(&file),
        &[vec![vec![1, 2, 3]]],
    );
    parity(&[p], &[file], &[vec![vec![3]]]);
}

#[test]
fn dynamic_capture_ties_preserve_ordinary_label_order_and_query_insertion_order() {
    let (_, file, _) = convergence_case(3);
    let ordinary = Program::new(
        vec![
            word(FOLLOWING, NAMED, 0),
            word(CAPTURE, 0, 2),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 0),
        ],
        vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
        vec![],
        vec![0, 1, 2],
        vec![],
    )
    .unwrap();
    let query = ordinary.clone().with_tree_sitter_query_semantics().unwrap();
    for (program, labels) in [(ordinary, vec![0, 1, 2]), (query, vec![2, 0, 1])] {
        let expected = program.execute_roots(&file, [1]).unwrap();
        assert_eq!(expected.len(), 2);
        for result in &expected {
            assert_eq!(
                result
                    .captures
                    .iter()
                    .map(|capture| capture.label)
                    .collect::<Vec<_>>(),
                labels
            );
        }
        parity(&[program], std::slice::from_ref(&file), &[vec![vec![1]]]);
    }
}

#[test]
fn wide_cursor_frames_and_capture_sequences_complete_within_evaluation_budget() {
    for slots in [31, 32, 33, 64] {
        let mut words = vec![word(CHILD, NAMED, 0)];
        for slot in 0..slots {
            words.extend([word(SAVE_CURSOR, 0, slot), word(NEXT, NAMED, 0)]);
        }
        for slot in (0..slots).rev() {
            words.extend([word(RESTORE_CURSOR, 0, slot), word(CAPTURE, 0, 0)]);
        }
        words.push(word(EMIT, 0, 0));
        let program =
            Program::new(words, vec![b"comment".to_vec()], vec![], vec![0], vec![]).unwrap();
        let (_, file, _) = convergence_case(usize::from(slots) + 3);
        let expected = program.execute_roots(&file, [0]).unwrap();
        assert_eq!(expected.len(), 3);
        for (offset, output) in expected.iter().enumerate() {
            assert_eq!(output.captures.len(), usize::from(slots));
            assert_eq!(
                output
                    .captures
                    .iter()
                    .map(|capture| capture.node)
                    .collect::<Vec<_>>(),
                ((offset as u32 + 1)..(offset as u32 + 1 + u32::from(slots))).collect::<Vec<_>>()
            );
        }
        parity(&[program], &[file], &[vec![vec![0]]]);
    }
}

#[test]
fn bounded_query_regions_reduce_after_identity_erasure_and_at_control_joins() {
    let (_, file, _) = convergence_case(4);
    let words = [
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(CHILD, NAMED, 0),
            word(PARENT, 0, 0),
            word(CAPTURE, 0, 0),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(FORK, 0, 6),
            word(FLAGS, 0, 0),
            word(CAPTURE, 0, 0),
            word(GOTO, 0, 8),
            word(STOP, 0, 0),
            word(CAPTURE, 0, 0),
            word(GOTO, 0, 8),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
    ];
    for words in words {
        let program = Program::new(words, vec![b"node".to_vec()], vec![], vec![0], vec![])
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap();
        let expected = program.execute_roots(&file, [0]).unwrap();
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].captures.len(), 1);
        assert_eq!(expected[0].captures[0].node, 0);
        parity(&[program], std::slice::from_ref(&file), &[vec![vec![0]]]);
    }
}

#[test]
fn repeated_captures_complete_long_sibling_sequences_without_per_sibling_launches() {
    use ast_grep_language::{LanguageExt, SupportLang};
    let program = Program::compile_tree_sitter_query_pattern(
        "(module (comment)+ @fold)",
        &SupportLang::Python.get_ts_language(),
    )
    .unwrap();
    let mut launches = Vec::new();
    for siblings in [2, 33, 130] {
        let (_, file, _) = convergence_case(siblings);
        let expected = program.execute_roots(&file, [0]).unwrap();
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].captures.len(), siblings);
        assert_eq!(
            expected[0]
                .captures
                .iter()
                .map(|capture| capture.node)
                .collect::<Vec<_>>(),
            (1..=siblings as u32).collect::<Vec<_>>()
        );
        launches.push(parity(std::slice::from_ref(&program), &[file], &[vec![vec![0]]]).0);
    }
    assert!(
        launches[2] - launches[0] <= 32,
        "sibling growth must use broad stages: {launches:?}"
    );
}

#[test]
fn terminal_repetitions_preserve_root_capture_ties_mixed_routes_and_witness_limits() {
    use crate::policy_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::{LanguageExt, SupportLang};
    let program = Program::compile_tree_sitter_query_pattern(
        "(module ((comment) @a @b)+) @outer",
        &SupportLang::Python.get_ts_language(),
    )
    .unwrap();
    let sources = ["# first\n# second\nvalue\n# third\n", "# lone"];
    let files = sources
        .iter()
        .map(|source| {
            let grammar = Grammar::new(SupportLang::Python).unwrap();
            let native = StrDoc::new(source, SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
            TapeDoc::restore(Arc::new(source.to_string()), grammar, &image, &|| false).unwrap()
        })
        .collect::<Vec<_>>();
    let anchors = vec![vec![vec![0, 1]]; files.len()];
    let expected = execute_batch(std::slice::from_ref(&program), &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0][0]
            .iter()
            .map(|output| output.local_ordinal)
            .collect::<Vec<_>>(),
        vec![0, 3]
    );
    assert_eq!(
        expected[1][0][0]
            .captures
            .iter()
            .map(|capture| capture.node)
            .collect::<Vec<_>>(),
        vec![1, 1, 0]
    );
    for limit in [0, 1, 2, u32::MAX] {
        let mut limited = expected.clone();
        for outputs in limited.iter_mut().flatten() {
            outputs.truncate(limit as usize);
        }
        let actual = super::super::metal_program_backend::execute_packed_with_result_limits(
            std::slice::from_ref(&program),
            &files,
            &anchors,
            Some(&[limit]),
        )
        .and_then(|execution| execution.into_outcome())
        .and_then(BatchOutcome::into_result);
        assert_eq!(actual, Ok(limited));
    }
    let following = Program::new(
        vec![
            word(FOLLOWING, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    parity(
        &[program, following, closure()],
        &files,
        &vec![vec![vec![0, 1], vec![1], vec![1]]; files.len()],
    );
}

#[test]
fn repetition_routes_do_not_drop_programs_that_share_an_entry_shape() {
    use ast_grep_language::{LanguageExt, SupportLang};
    let patterns = [
        "(module ((comment) @item)+)",
        "(module ((comment) @item)*)",
        "(module ((comment) @item)+ (comment) @last)",
    ];
    let programs = patterns.map(|pattern| {
        Program::compile_tree_sitter_query_pattern(pattern, &SupportLang::Python.get_ts_language())
            .unwrap()
    });
    let (_, file, _) = convergence_case(2);
    let anchors = vec![vec![vec![0]; programs.len()]];
    let expected = execute_batch(&programs, std::slice::from_ref(&file), Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        vec![1, 1, 1]
    );
    assert_eq!(expected[0][2][0].captures.len(), 2);
    parity(&programs, &[file], &anchors);
}

#[test]
fn capturing_loop_continuations_preserve_all_results_with_bounded_successor_work() {
    use ast_grep_language::{LanguageExt, SupportLang};
    let programs = [
        "(module ((comment) @item)+ (comment) @last)",
        "(module ((comment) @item)*)",
        "(module [(comment) @left (comment) @right]+)",
    ]
    .map(|pattern| {
        Program::compile_tree_sitter_query_pattern(pattern, &SupportLang::Python.get_ts_language())
            .unwrap()
    });
    for siblings in [2, 8] {
        let (_, file, _) = convergence_case(siblings);
        let anchors = vec![vec![vec![0]; programs.len()]];
        let expected =
            execute_batch(&programs, std::slice::from_ref(&file), Some(&anchors)).unwrap();
        assert!(expected[0].iter().all(|outputs| !outputs.is_empty()));
        assert!(
            expected[0][0]
                .iter()
                .all(|output| output.captures.len() >= 2)
        );
        parity(&programs, &[file], &anchors);
    }
}

#[test]
fn uncaptured_loops_preserve_middle_captures_and_raw_ordinals() {
    use ast_grep_language::{LanguageExt, SupportLang};
    let programs = [
        "(module (comment)+ (comment) @middle (comment)+)",
        "(module ((comment) @item)+)",
        "(module [(comment) @left (comment) @right]+)",
    ]
    .map(|pattern| {
        Program::compile_tree_sitter_query_pattern(pattern, &SupportLang::Python.get_ts_language())
            .unwrap()
    });
    for siblings in [3, 6] {
        let (_, file, _) = convergence_case(siblings);
        let expected = programs[0].execute_roots(&file, [0]).unwrap();
        assert_eq!(expected.len(), siblings - 2);
        assert_eq!(
            expected
                .iter()
                .map(|output| output.captures[0].node)
                .collect::<Vec<_>>(),
            (2..siblings as u32).collect::<Vec<_>>()
        );
        parity(&programs, &[file], &[vec![vec![0]; programs.len()]]);
    }
}

#[test]
fn nested_optional_captures_preserve_every_raw_candidate_ordinal() {
    use crate::policy_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::{LanguageExt, SupportLang};
    let program = Program::compile_tree_sitter_query_pattern(
        "(type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type (_ [(identifier) (none)]? @type)))))",
        &SupportLang::Python.get_ts_language(),
    ).unwrap();
    let source = "fields: ClassVar[dict[str, Field[str]]]\nfile: IO[str] | IO[bytes] | None\n";
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source, SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let file = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    let roots = (0..file.transducer_len() as u32).collect::<Vec<_>>();
    let expected = program.execute_roots(&file, roots.iter().copied()).unwrap();
    assert!(!expected.is_empty());
    assert!(
        expected
            .iter()
            .all(|output| output.captures.iter().all(|capture| capture.label == 0))
    );
    let native = execute_with_backend(
        ExecutionBackend::CpuJit,
        std::slice::from_ref(&program),
        std::slice::from_ref(&file),
        &[vec![roots.clone()]],
    )
    .unwrap();
    let expected_batch = execute_batch(
        std::slice::from_ref(&program),
        std::slice::from_ref(&file),
        Some(&[vec![roots.clone()]]),
    )
    .unwrap();
    assert_eq!(native, expected_batch);
    parity(&[program], &[file], &[vec![roots]]);
}

#[test]
fn complete_routes_preserve_different_bodies_with_the_same_first_fork() {
    let make = |count| {
        let mut words = vec![word(FORK, 0, 3), word(CAPTURE, 0, 0), word(EMIT, 0, 0)];
        words.extend(vec![word(CAPTURE, 0, 0); count]);
        words.push(word(EMIT, 0, 0));
        Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap()
    };
    let (_, file, _) = convergence_case(2);
    for counts in [[1, 2, 33], [33, 2, 1]] {
        let programs = counts.map(make);
        let files = [file.clone(), file.clone()];
        let anchors = vec![vec![vec![0, 1]; programs.len()]; files.len()];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        for outputs in &expected {
            for (index, outputs) in outputs.iter().enumerate() {
                assert_eq!(
                    outputs
                        .iter()
                        .map(|output| output.captures.len())
                        .collect::<Vec<_>>(),
                    vec![1, counts[index], 1, counts[index]]
                );
            }
        }
        parity(&programs, &files, &anchors);
    }
}

#[test]
fn dynamic_witness_limits_apply_per_evaluation_after_terminal_ordering() {
    use super::super::metal_grid::MetalGridInput;
    use super::super::metal_grid_executor::GuardedExecutor;
    use super::super::metal_grid_plan::{ExecutionPlan, ResidentInput};
    let (converging, file, _) = convergence_case(3);
    let ordinary = Program::new(
        vec![
            word(FOLLOWING, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [ordinary, converging, closure()];
    let files = [file.clone(), file];
    let anchors = vec![vec![vec![1, 2, 3]; 3]; 2];
    for limit in [0, 1, 2, u32::MAX] {
        let mut expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        for outputs in expected.iter_mut().flatten() {
            outputs.truncate(limit as usize);
        }
        let resident = ResidentInput::new(
            MetalGridInput::build(&programs, &files, &anchors).unwrap(),
            Some(&[limit; 3]),
        )
        .unwrap();
        let plan = ExecutionPlan::new(&resident, &programs).unwrap();
        let actual = GuardedExecutor::new(resident, plan)
            .unwrap()
            .execute()
            .unwrap()
            .into_outcome()
            .unwrap()
            .into_result();
        assert_eq!(actual, Ok(expected), "limit={limit}");
    }
}

#[test]
fn deep_ancestor_expansion_keeps_capture_identity_without_history_commands() {
    use crate::policy_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::SupportLang;
    let program = Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(DESCEND, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"node".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let mut generations = Vec::new();
    for depth in [1, 128, 1024] {
        let source = Arc::new(format!("{}x{}", "[".repeat(depth), "]".repeat(depth)));
        let grammar = Grammar::new(SupportLang::Python).unwrap();
        let native = StrDoc::new(source.as_str(), SupportLang::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
        let file = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
        assert!(file.transducer_len() > depth);
        generations.push(parity(std::slice::from_ref(&program), &[file], &[vec![vec![0]]]).1);
    }
    assert_eq!(generations, vec![generations[0]; 3]);
}

#[test]
fn closure_proof_rejects_changed_long_guard_values() {
    use super::super::metal_grid::MetalGridInput;
    use super::super::metal_grid_plan::{ExecutionPlan, ResidentInput};
    let (_, file, _) = convergence_case(3);
    let p = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 7),
            word(NEXT, 0, 0),
            word(SIZE, SIZE_LONG, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 2),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![6],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let mut resident = ResidentInput::new(
        MetalGridInput::build(std::slice::from_ref(&p), &[file], &[vec![vec![1]]]).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &[p]).unwrap();
    Arc::make_mut(&mut resident.input.packed).longs[0] = 7;
    assert_eq!(plan.validate(&resident), Err(Error::InvalidAnchor));
}

#[test]
fn converging_closure_entries_share_exact_domains_and_preserve_first_visit() {
    let (_, file, _) = convergence_case(130);
    let program = Program::new(
        vec![
            word(CAPTURE, 0, 0),
            word(FOLLOWING, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 8),
            word(NEXT, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 4),
            word(REPEAT_END, 0, 0),
            word(LAST, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap();
    let expected = program.execute_roots(&file, [1]).unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].captures.len(), 1);
    assert_eq!(expected[0].captures[0].node, 1);
    parity(&[program], &[file], &[vec![vec![1]]]);
}
