//! Ordinary assertions consume the current capture frame at their executed PC.
use super::*;

struct OrdinaryAssertionCases {
    programs: Vec<Program>,
    files: Vec<TapeDoc>,
    anchors: Vec<Vec<Vec<u32>>>,
    counts: Vec<usize>,
}

fn ordinary_assertion_cases() -> OrdinaryAssertionCases {
    let language = SupportLang::Python.get_ts_language();
    let call = language.id_for_node_kind("call", true);
    let mut cases = vec![
        (
            vec![
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(PARENT, 0, 0),
                word(PARENT, 0, 0),
                word(NEXT, NAMED, 0),
                word(CHILD, NAMED, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            CapturePredicate::Text {
                capture: 0,
                predicate: 0,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            vec![
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(RESTORE_CURSOR, 0, 5),
                word(EMIT, 0, 0),
            ],
            CapturePredicate::Text {
                capture: 0,
                predicate: 1,
                negated: false,
                any: false,
            },
            0,
        ),
        (
            vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            CapturePredicate::Text {
                capture: 0,
                predicate: 1,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            vec![word(ASSERT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            CapturePredicate::Kinds {
                capture: 0,
                kinds: vec![call],
                negated: false,
            },
            1,
        ),
        (
            vec![
                word(CAPTURE, 0, 0),
                word(FORK, 0, 5),
                word(ASSERT, 0, 0),
                word(EMIT, 0, 0),
                word(STOP, 0, 0),
                word(EMIT, 0, 0),
            ],
            CapturePredicate::Text {
                capture: 0,
                predicate: 1,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            vec![
                word(SAVE_CURSOR, 0, 0),
                word(PARENT, 0, 0),
                word(PARENT, 0, 0),
                word(NEXT, NAMED, 0),
                word(CHILD, NAMED, 0),
                word(CAPTURE, 0, 0),
                word(RESTORE_CURSOR, 0, 0),
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 1),
                word(ASSERT, 0, 0),
                word(EMIT, 0, 0),
            ],
            CapturePredicate::Equal {
                left: 0,
                right: 1,
                negated: true,
                any: false,
            },
            1,
        ),
    ];
    let mut twice = cases[0].0.clone();
    twice.insert(twice.len() - 1, word(ASSERT, 0, 0));
    cases.push((
        twice,
        CapturePredicate::Text {
            capture: 0,
            predicate: 0,
            negated: false,
            any: false,
        },
        0,
    ));
    let counts = cases.iter().map(|case| case.2).collect();
    let programs = cases
        .into_iter()
        .map(|(words, predicate, _)| {
            Program::new_with_capture_predicates(
                words,
                vec![
                    b"left".to_vec(),
                    b"right".to_vec(),
                    b"first".to_vec(),
                    b"third".to_vec(),
                ],
                vec![TextPredicate::Equal(2), TextPredicate::Equal(3)],
                vec![predicate],
                vec![0, 1],
                vec![],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let file = tape("first(second)\nthird\n");
    let first = (0..file.transducer_len() as u32)
        .find(|&node| file.transducer_text(node) == b"first")
        .unwrap();
    let anchors = vec![vec![vec![first]; programs.len()]];
    OrdinaryAssertionCases {
        programs,
        files: vec![file],
        anchors,
        counts,
    }
}

fn ordinary_assertion_expected(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
    counts: &[usize],
) -> BatchOutcome {
    let expected =
        execute_with_backend_outcomes(ExecutionBackend::CpuInterpreter, programs, files, anchors)
            .unwrap();
    assert_eq!(expected.errors, vec![vec![None; programs.len()]]);
    assert_eq!(
        expected.outputs[0].iter().map(Vec::len).collect::<Vec<_>>(),
        counts
    );
    let first = &expected.outputs[0][0][0];
    assert_eq!(first.captures.len(), 2);
    assert_eq!(files[0].transducer_text(first.captures[0].node), b"first");
    assert_eq!(files[0].transducer_text(first.captures[1].node), b"third");
    let paired = &expected.outputs[0][5][0];
    assert_eq!(
        paired
            .captures
            .iter()
            .map(|capture| (capture.label, files[0].transducer_text(capture.node)))
            .collect::<Vec<_>>(),
        [(0, b"first".as_slice()), (1, b"first"), (0, b"third")]
    );
    expected
}

#[test]
fn native_ordinary_assertions_observe_executed_capture_prefixes_and_skip_unused_predicates() {
    let OrdinaryAssertionCases {
        programs,
        files,
        anchors,
        counts,
    } = ordinary_assertion_cases();
    let expected = ordinary_assertion_expected(&programs, &files, &anchors, &counts);
    let actual =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(
        actual.errors, expected.errors,
        "an earlier rejection must prevent the later invalid RESTORE"
    );
    assert_eq!(actual.outputs, expected.outputs);
}

#[test]
fn metal_ordinary_assertions_preserve_prefixes_pair_order_and_branch_local_rejection() {
    let OrdinaryAssertionCases {
        programs,
        files,
        anchors,
        counts,
    } = ordinary_assertion_cases();
    let expected = ordinary_assertion_expected(&programs, &files, &anchors, &counts);
    for limit in [0, 1] {
        let mut outputs = expected.outputs.clone();
        for rows in &mut outputs[0] {
            rows.truncate(limit as usize);
        }
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&vec![limit; programs.len()]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|outcome| (outcome.outputs, outcome.errors)),
            Ok((outputs, expected.errors.clone())),
            "accepted limit={limit}"
        );
    }
}

#[test]
fn native_query_assertions_share_one_budget_across_successive_predicates() {
    let pattern = "^# (?:a|aa)*b$";
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let (source, cost) = [20_000, 40_000, 60_000, 80_000]
        .into_iter()
        .find_map(|length| {
            let source = format!("# {}", "a".repeat(length));
            let (outcome, cost) = nfa.test_work_count(source.as_bytes());
            (outcome == Ok(false) && cost > MAX_STEPS_PER_PATH / 2 && cost <= MAX_STEPS_PER_PATH)
                .then_some((source, cost))
        })
        .unwrap();
    assert!(2 * cost > MAX_STEPS_PER_PATH);
    let programs = [1, 2]
        .into_iter()
        .map(|count| {
            let mut words = vec![word(CAPTURE, 0, 0)];
            words.extend((0..count).map(|index| word(ASSERT, 0, index)));
            words.push(word(EMIT, 0, 0));
            Program::new_with_capture_predicates(
                words,
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                }],
                vec![
                    CapturePredicate::Text {
                        capture: 0,
                        predicate: 0,
                        negated: true,
                        any: false
                    };
                    count as usize
                ],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = [tape(&source)];
    let anchors = [vec![vec![0]; programs.len()]];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(expected.errors, [vec![None, Some(Error::WorkLimit)]]);
    assert_eq!(expected.outputs[0][0].len(), 1);
    assert!(expected.outputs[0][1].is_empty());
    let actual =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(actual.errors, expected.errors);
    assert_eq!(actual.outputs, expected.outputs);
}

fn ordered_inline_work_cases() -> (Vec<Program>, Vec<TapeDoc>, Vec<Vec<Vec<u32>>>) {
    let pattern = "^# (?:a|aa)*b$";
    let source = format!("# {}", "a".repeat(200_000));
    assert_eq!(
        value::ByteNfa::compile(pattern)
            .unwrap()
            .test_work_count(source.as_bytes())
            .0,
        Err(Error::WorkLimit)
    );
    let words = [
        vec![
            word(CAPTURE, 0, 0),
            word(ASSERT, 0, 0),
            word(RESTORE_CURSOR, 0, 5),
            word(EMIT, 0, 0),
        ],
        vec![
            word(FORK, 0, 5),
            word(RESTORE_CURSOR, 0, 5),
            word(STOP, 0, 0),
            word(STOP, 0, 0),
            word(STOP, 0, 0),
            word(CAPTURE, 0, 0),
            word(ASSERT, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![
            word(FORK, 0, 5),
            word(CAPTURE, 0, 0),
            word(ASSERT, 0, 0),
            word(EMIT, 0, 0),
            word(STOP, 0, 0),
            word(RESTORE_CURSOR, 0, 5),
            word(EMIT, 0, 0),
        ],
    ];
    let programs = words
        .into_iter()
        .map(|words| {
            Program::new_with_capture_predicates(
                words,
                vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
                vec![TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                }],
                vec![CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                }],
                vec![0],
                vec![],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = vec![tape(&source), tape("# short")];
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    (programs, files, anchors)
}
fn ordered_inline_work_expected(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> BatchOutcome {
    let expected =
        execute_with_backend_outcomes(ExecutionBackend::CpuInterpreter, programs, files, anchors)
            .unwrap();
    assert_eq!(
        expected.errors,
        [
            vec![
                Some(Error::WorkLimit),
                Some(Error::InvalidOperand(1)),
                Some(Error::WorkLimit)
            ],
            vec![
                None,
                Some(Error::InvalidOperand(1)),
                Some(Error::InvalidOperand(5))
            ]
        ]
    );
    assert!(expected.outputs.iter().flatten().all(Vec::is_empty));
    expected
}
#[test]
fn native_inline_work_failures_keep_their_executed_position_before_later_operand_errors() {
    let (programs, files, anchors) = ordered_inline_work_cases();
    let expected = ordered_inline_work_expected(&programs, &files, &anchors);
    let actual =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(actual.errors, expected.errors);
    assert_eq!(actual.outputs, expected.outputs);
}
#[test]
fn metal_inline_work_failures_compete_with_operand_failures_in_logical_execution_order() {
    let (programs, files, anchors) = ordered_inline_work_cases();
    let expected = ordered_inline_work_expected(&programs, &files, &anchors);
    for limit in [0, 1] {
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 3]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|outcome| (outcome.outputs, outcome.errors)),
            Ok((expected.outputs.clone(), expected.errors.clone())),
            "accepted limit={limit}"
        );
    }
}

#[test]
fn metal_inline_text_values_compare_long_literals_and_regexes_before_capturing() {
    let full = format!("# {}END", "a".repeat(128));
    let prefix = format!("# {}", "a".repeat(128));
    let pattern = "^# a+END$";
    let programs = [
        TextPredicate::Regex {
            source: 3,
            compiled: regex::Regex::new(pattern).unwrap(),
        },
        TextPredicate::Equal(0),
        TextPredicate::Prefix(1),
    ]
    .into_iter()
    .map(|predicate| {
        Program::new(
            vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![
                full.as_bytes().to_vec(),
                prefix.as_bytes().to_vec(),
                b"text".to_vec(),
                pattern.as_bytes().to_vec(),
            ],
            vec![predicate],
            vec![2],
            vec![],
        )
        .unwrap()
    })
    .collect::<Vec<_>>();
    let files = [
        tape(&full),
        tape("# mismatch"),
        tape(&format!("{prefix}BAD")),
    ];
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(
        expected
            .outputs
            .iter()
            .map(|file| file.iter().map(Vec::len).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [vec![1, 1, 1], vec![0, 0, 0], vec![0, 0, 1]]
    );
    assert!(expected.errors.iter().flatten().all(Option::is_none));
    for limit in [0, 1] {
        let mut outputs = expected.outputs.clone();
        for file in &mut outputs {
            for rows in file {
                rows.truncate(limit as usize);
            }
        }
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 3]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|outcome| (outcome.outputs, outcome.errors)),
            Ok((outputs, expected.errors.clone())),
            "accepted limit={limit}"
        );
    }
}

#[test]
fn metal_inline_text_work_is_inherited_across_successful_values_before_later_errors() {
    let pattern = "^# (?:a|aa)*$";
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let (source, cost) = [20_000, 40_000, 60_000, 80_000, 100_000, 130_000]
        .into_iter()
        .find_map(|length| {
            let source = format!("# {}", "a".repeat(length));
            let (outcome, cost) = nfa.test_work_count(source.as_bytes());
            (outcome == Ok(true) && cost > MAX_STEPS_PER_PATH / 2 && cost + 3 <= MAX_STEPS_PER_PATH)
                .then_some((source, cost))
        })
        .unwrap();
    assert!(cost * 2 > MAX_STEPS_PER_PATH);
    let programs = [
        vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![
            word(TEXT, 0, 0),
            word(TEXT, 0, 0),
            word(RESTORE_CURSOR, 0, 5),
            word(EMIT, 0, 0),
        ],
    ]
    .into_iter()
    .map(|words| {
        Program::new(
            words,
            vec![b"text".to_vec(), pattern.as_bytes().to_vec()],
            vec![TextPredicate::Regex {
                source: 1,
                compiled: regex::Regex::new(pattern).unwrap(),
            }],
            vec![0],
            vec![],
        )
        .unwrap()
    })
    .collect::<Vec<_>>();
    let files = [tape(&source), tape("# short")];
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(
        expected.errors,
        [vec![None, Some(Error::WorkLimit)], vec![None, None]]
    );
    assert_eq!(expected.outputs[0][0].len(), 1);
    assert!(expected.outputs[0][1].is_empty());
    for limit in [0, 1] {
        let mut outputs = expected.outputs.clone();
        for file in &mut outputs {
            for rows in file {
                rows.truncate(limit as usize);
            }
        }
        let scope = metal_backend::launch::LaunchScope::enter();
        let started = std::time::Instant::now();
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 2]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|outcome| (outcome.outputs, outcome.errors)),
            Ok((outputs, expected.errors.clone())),
            "accepted limit={limit}, one value costs{cost}"
        );
        let trace = scope.snapshot();
        let code_buffers = trace
            .iter()
            .filter(|record| record.kernel == "regex_frontier_advance")
            .map(|record| record.buffers[1].0)
            .collect::<std::collections::BTreeSet<_>>();
        eprintln!(
            "INLINE_VALUE_RESIDENCY_EVIDENCE {}",
            serde_json::json!({"limit":limit,"launches":trace.len(),"compiledRuleBuffers":code_buffers.len(),"seconds":started.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|record|record.peak_application_buffer_bytes).max()})
        );
        assert_eq!(
            code_buffers.len(),
            1,
            "successive value continuations must share one immutable compiled rule table"
        );
    }
}

#[test]
fn terminal_capture_conversion_preserves_results_after_named_expansion_rejects_all_children() {
    let tuple = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("tuple", true);
    let program = Program::new_with_capture_predicates(
        vec![
            word(CAPTURE, 0, 0),
            word(ASSERT, 0, 0),
            word(FORK, 0, 5),
            word(EMIT, 0, 0),
            word(STOP, 0, 0),
            word(CHILD, NAMED, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"tuple".to_vec()],
        vec![],
        vec![CapturePredicate::Kind {
            capture: 0,
            kind: tuple,
            negated: false,
            any: false,
        }],
        vec![0],
        vec![],
    )
    .unwrap();
    let file = tape("()");
    let root = (0..file.transducer_len() as u32)
        .find(|&node| file.transducer_kind(node) == tuple)
        .unwrap();
    assert_eq!(file.transducer_text(root), b"()");
    let files = vec![file; 128];
    let programs = [program];
    let anchors = vec![vec![vec![root]]; files.len()];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert!(expected.errors.iter().flatten().all(Option::is_none));
    assert!(
        expected
            .outputs
            .iter()
            .all(|file| file[0].len() == 1 && file[0][0].captures.len() == 1)
    );
    let actual =
        execute_with_backend_outcomes(ExecutionBackend::Metal, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(actual.errors, expected.errors);
    assert_eq!(actual.outputs, expected.outputs);
}
