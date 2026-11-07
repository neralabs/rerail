//! Complete capture-operator truth tables and published values.
use super::*;

fn frame_program(predicate: CapturePredicate, query: bool) -> Program {
    let program = Program::new_with_capture_predicates(
        vec![
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 1),
            word(PARENT, 0, 0),
            word(PARENT, 0, 0),
            word(NEXT, NAMED, 0),
            word(CHILD, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 2),
            word(ASSERT, 0, 0),
            word(EMIT, 0, 0),
        ],
        [
            "joined", "left", "right", "missing", "role", "kept", "first", "third", "", "ir",
        ]
        .iter()
        .map(|text| text.as_bytes().to_vec())
        .collect(),
        vec![TextPredicate::Equal(6)],
        vec![predicate],
        vec![0, 1, 2, 3],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::Set {
        key: 4,
        value: Some(5),
    }])
    .unwrap();
    if query {
        program.with_tree_sitter_query_semantics().unwrap()
    } else {
        program
    }
}
fn frame() -> (TapeDoc, u32) {
    let file = tape("first(second)\nthird\n");
    let kind = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("identifier", true);
    let first = (0..file.transducer_len() as u32)
        .find(|&node| file.transducer_kind(node) == kind && file.transducer_text(node) == b"first")
        .unwrap();
    (file, first)
}

#[test]
fn native_relations_preserve_existential_positive_and_universal_negative_capture_groups() {
    let call = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("call", true);
    let programs = [false, true]
        .into_iter()
        .flat_map(|ancestor| {
            [false, true].into_iter().map(move |negated| {
                frame_program(
                    CapturePredicate::Relation {
                        capture: 0,
                        kinds: vec![call],
                        ancestor,
                        negated,
                    },
                    false,
                )
            })
        })
        .collect::<Vec<_>>();
    let (file, first) = frame();
    let files = [file];
    let anchors = [vec![vec![first]; programs.len()]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 0, 1, 0]
    );
    for outputs in &expected[0] {
        for output in outputs {
            assert_eq!(output.captures.len(), 4);
            let texts = output
                .captures
                .iter()
                .map(|capture| &files[0].transducer_source()[capture.span.clone()])
                .collect::<Vec<_>>();
            assert_eq!(texts, [b"first".as_slice(), b"first", b"third", b"third"]);
            assert_eq!(output.properties, [(4, Some(5))]);
        }
    }
    assert_eq!(
        execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors),
        Ok(expected)
    );
}

fn operator_cases() -> Vec<(CapturePredicate, usize)> {
    let language = SupportLang::Python.get_ts_language();
    let identifier = language.id_for_node_kind("identifier", true);
    let call = language.id_for_node_kind("call", true);
    let mut cases = vec![
        (
            CapturePredicate::Equal {
                left: 0,
                right: 1,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Equal {
                left: 0,
                right: 2,
                negated: false,
                any: false,
            },
            0,
        ),
        (
            CapturePredicate::Equal {
                left: 0,
                right: 2,
                negated: true,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Equal {
                left: 3,
                right: 0,
                negated: true,
                any: false,
            },
            0,
        ),
        (
            CapturePredicate::SameLine {
                left: 1,
                right: 2,
                negated: false,
            },
            0,
        ),
        (
            CapturePredicate::SameLine {
                left: 1,
                right: 2,
                negated: true,
            },
            1,
        ),
        (
            CapturePredicate::SameLine {
                left: 0,
                right: 1,
                negated: false,
            },
            1,
        ),
        (
            CapturePredicate::Kind {
                capture: 0,
                kind: identifier,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Kind {
                capture: 0,
                kind: identifier,
                negated: true,
                any: true,
            },
            0,
        ),
        (
            CapturePredicate::Kinds {
                capture: 0,
                kinds: vec![call],
                negated: false,
            },
            0,
        ),
        (
            CapturePredicate::Kinds {
                capture: 0,
                kinds: vec![call],
                negated: true,
            },
            1,
        ),
    ];
    for negated in [false, true] {
        cases.push((
            CapturePredicate::Kinds {
                capture: 3,
                kinds: vec![identifier],
                negated,
            },
            1,
        ));
        cases.push((
            CapturePredicate::Relation {
                capture: 0,
                kinds: vec![call],
                ancestor: false,
                negated,
            },
            usize::from(!negated),
        ));
        cases.push((
            CapturePredicate::Relation {
                capture: 0,
                kinds: vec![call],
                ancestor: true,
                negated,
            },
            usize::from(!negated),
        ));
        cases.push((
            CapturePredicate::Relation {
                capture: 3,
                kinds: vec![call],
                ancestor: true,
                negated,
            },
            usize::from(!negated),
        ));
        for any in [false, true] {
            cases.push((
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![6, 7],
                    negated,
                    any,
                },
                usize::from(!negated),
            ));
            cases.push((
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![6],
                    negated,
                    any,
                },
                usize::from(any),
            ));
            cases.push((
                CapturePredicate::AnyOf {
                    capture: 0,
                    strings: vec![],
                    negated,
                    any,
                },
                usize::from(negated),
            ));
            cases.push((
                CapturePredicate::AnyOf {
                    capture: 3,
                    strings: vec![6, 7],
                    negated,
                    any,
                },
                0,
            ));
            cases.push((
                CapturePredicate::Contains {
                    capture: 0,
                    strings: vec![9],
                    negated,
                    any,
                },
                usize::from(!negated),
            ));
            cases.push((
                CapturePredicate::Contains {
                    capture: 0,
                    strings: vec![6],
                    negated,
                    any,
                },
                usize::from(any),
            ));
            cases.push((
                CapturePredicate::Contains {
                    capture: 0,
                    strings: vec![],
                    negated,
                    any,
                },
                0,
            ));
            cases.push((
                CapturePredicate::Contains {
                    capture: 0,
                    strings: vec![8],
                    negated,
                    any,
                },
                usize::from(!negated),
            ));
            cases.push((
                CapturePredicate::Contains {
                    capture: 3,
                    strings: vec![6],
                    negated,
                    any,
                },
                usize::from(!negated),
            ));
        }
    }
    cases
}

#[test]
fn shared_capture_operators_preserve_pairing_quantifiers_empty_groups_and_properties() {
    let cases = operator_cases();
    let programs = cases
        .iter()
        .map(|(predicate, _)| frame_program(predicate.clone(), true))
        .collect::<Vec<_>>();
    let (file, first) = frame();
    let files = [file];
    let anchors = [vec![vec![first]; programs.len()]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        cases.iter().map(|(_, count)| *count).collect::<Vec<_>>()
    );
    for output in expected[0].iter().flatten() {
        assert_eq!(output.captures.len(), 4);
        assert_eq!(output.properties, [(4, Some(5))]);
    }
    for limit in [0, 1] {
        let mut limited = expected.clone();
        for outputs in &mut limited[0] {
            outputs.truncate(limit as usize);
        }
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&vec![limit; programs.len()]),
        )
        .and_then(|execution| execution.into_outcome())
        .and_then(BatchOutcome::into_result);
        assert_eq!(actual, Ok(limited), "accepted limit={limit}");
    }
}

#[test]
fn native_capture_operators_preserve_the_complete_group_truth_table() {
    let cases = operator_cases();
    let programs = cases
        .iter()
        .map(|(predicate, _)| frame_program(predicate.clone(), false))
        .collect::<Vec<_>>();
    let (file, first) = frame();
    let files = [file];
    let anchors = [vec![vec![first]; programs.len()]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        cases.iter().map(|(_, count)| *count).collect::<Vec<_>>()
    );
    assert_eq!(
        execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors),
        Ok(expected)
    );
}

#[test]
fn shared_capture_operators_compare_long_utf8_spans_and_chunk_crossing_needles() {
    let text = format!("{}é{}tail", "a".repeat(71), "b".repeat(87));
    let source = format!("{text}\n");
    let needle = format!("{}é{}", "a".repeat(12), "b".repeat(75));
    let strings = vec![
        text.as_bytes().to_vec(),
        text.as_bytes()[..73].to_vec(),
        format!("{}X", &text[..text.len() - 1]).into_bytes(),
        needle.into_bytes(),
        format!("{}Z", "b".repeat(65)).into_bytes(),
        Vec::new(),
    ];
    let cases = vec![
        (
            CapturePredicate::Text {
                capture: 0,
                predicate: 0,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Text {
                capture: 0,
                predicate: 1,
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Text {
                capture: 0,
                predicate: 2,
                negated: true,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![2, 0],
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::AnyOf {
                capture: 0,
                strings: vec![2],
                negated: true,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Contains {
                capture: 0,
                strings: vec![3],
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Contains {
                capture: 0,
                strings: vec![4],
                negated: false,
                any: false,
            },
            0,
        ),
        (
            CapturePredicate::Contains {
                capture: 0,
                strings: vec![3, 4],
                negated: true,
                any: true,
            },
            1,
        ),
        (
            CapturePredicate::Contains {
                capture: 0,
                strings: vec![5],
                negated: false,
                any: false,
            },
            1,
        ),
        (
            CapturePredicate::Equal {
                left: 0,
                right: 1,
                negated: false,
                any: false,
            },
            1,
        ),
    ];
    let programs = cases
        .iter()
        .map(|(predicate, _)| {
            Program::new_with_capture_predicates(
                vec![
                    word(CAPTURE, 0, 0),
                    word(CAPTURE, 0, 1),
                    word(ASSERT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                strings.clone(),
                vec![
                    TextPredicate::Equal(0),
                    TextPredicate::Prefix(1),
                    TextPredicate::Equal(2),
                ],
                vec![predicate.clone()],
                vec![0, 1],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = [tape(&source), tape("short\n")];
    let anchors = vec![vec![vec![1]; programs.len()]; files.len()];
    // Node one is the expression statement and excludes the final newline.
    assert_eq!(files[0].transducer_text(1), text.as_bytes());
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        cases.iter().map(|(_, count)| *count).collect::<Vec<_>>()
    );
    for limit in [0, 1] {
        let mut limited = expected.clone();
        for file in &mut limited {
            for rows in file {
                rows.truncate(limit as usize);
            }
        }
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&vec![limit; programs.len()]),
        )
        .and_then(|execution| execution.into_outcome())
        .and_then(BatchOutcome::into_result);
        assert_eq!(actual, Ok(limited), "accepted limit={limit}");
    }
}

#[test]
fn shared_capture_pairs_apply_negation_before_any_or_all_and_zip_occurrences() {
    let cases = [
        (false, false, 0),
        (false, true, 1),
        (true, false, 0),
        (true, true, 1),
    ];
    let programs = cases
        .iter()
        .map(|&(negated, any, _)| {
            Program::new_with_capture_predicates(
                vec![
                    word(CAPTURE, 0, 0),
                    word(PARENT, 0, 0),
                    word(CHILD, NAMED, 0),
                    word(NEXT, NAMED, 0),
                    word(CHILD, NAMED, 0),
                    word(CAPTURE, 0, 1),
                    word(PARENT, 0, 0),
                    word(PARENT, 0, 0),
                    word(PARENT, 0, 0),
                    word(NEXT, NAMED, 0),
                    word(CHILD, NAMED, 0),
                    word(CAPTURE, 0, 0),
                    word(CAPTURE, 0, 1),
                    word(ASSERT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![b"left".to_vec(), b"right".to_vec()],
                vec![],
                vec![CapturePredicate::Equal {
                    left: 0,
                    right: 1,
                    negated,
                    any,
                }],
                vec![0, 1],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let (file, first) = frame();
    let files = [file];
    let anchors = [vec![vec![first]; programs.len()]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        cases.iter().map(|case| case.2).collect::<Vec<_>>()
    );
    for row in expected[0].iter().flatten() {
        let text = row
            .captures
            .iter()
            .map(|capture| files[0].transducer_text(capture.node))
            .collect::<Vec<_>>();
        assert_eq!(text, [b"first".as_slice(), b"second", b"third", b"third"]);
    }
    assert_eq!(
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
        Ok(expected)
    );
}

#[test]
fn shared_ancestor_sets_exclude_self_cross_scan_groups_and_file_boundaries() {
    let language = SupportLang::Python.get_ts_language();
    let call = language.id_for_node_kind("call", true);
    let module = language.id_for_node_kind("module", true);
    let identifier = language.id_for_node_kind("identifier", true);
    let programs = [
        vec![call],
        vec![module],
        vec![identifier, call, identifier],
        vec![],
    ]
    .into_iter()
    .flat_map(|kinds| {
        [false, true].into_iter().map(move |negated| {
            Program::new_with_capture_predicates(
                vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
                vec![b"node".to_vec()],
                vec![],
                vec![CapturePredicate::Relation {
                    capture: 0,
                    kinds: kinds.clone(),
                    ancestor: true,
                    negated,
                }],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
    })
    .collect::<Vec<_>>();
    let files = [
        tape(&"a(b(c))\n".repeat(100)),
        tape("plain\n"),
        tape("# tail\n"),
    ];
    assert!(files[0].transducer_len() > 256);
    let anchors = files
        .iter()
        .map(|file| vec![(0..file.transducer_len() as u32).collect::<Vec<_>>(); programs.len()])
        .collect::<Vec<_>>();
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[1][0].len(), 0);
    assert_eq!(expected[1][1].len(), files[1].transducer_len());
    assert_eq!(expected[0][6].len(), 0);
    assert_eq!(expected[0][7].len(), files[0].transducer_len());
    assert!(!expected[0][2].iter().any(|row| row.start == 0));
    assert_eq!(
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
        Ok(expected)
    );
}

#[test]
fn shared_capture_operator_batches_use_broad_stages_as_files_grow() {
    let call = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("call", true);
    let programs = [
        CapturePredicate::Contains {
            capture: 0,
            strings: vec![1],
            negated: false,
            any: false,
        },
        CapturePredicate::Relation {
            capture: 0,
            kinds: vec![call],
            ancestor: true,
            negated: false,
        },
    ]
    .into_iter()
    .map(|predicate| {
        Program::new_with_capture_predicates(
            vec![
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"text".to_vec(), b"ir".to_vec()],
            vec![],
            vec![predicate],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap()
    })
    .collect::<Vec<_>>();
    let mut first_launches = None;
    for count in [1, 128] {
        let (file, first) = frame();
        let files = vec![file; count];
        let anchors = vec![vec![vec![first]; programs.len()]; count];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            expected.iter().flatten().map(Vec::len).sum::<usize>(),
            2 * count
        );
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        assert_eq!(
            execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
            Ok(expected)
        );
        let trace = scope.snapshot();
        if let Some(first) = first_launches {
            assert!(trace.len() <= first + 16, "{} launches", trace.len());
        } else {
            assert!(trace.len() < 40, "{} launches", trace.len());
            first_launches = Some(trace.len());
        }
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
        eprintln!(
            "CAPTURE_OPERATOR_SCALING_EVIDENCE {}",
            serde_json::json!({"files":count,"programs":programs.len(),"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|record|record.peak_application_buffer_bytes).max()})
        );
    }
}

#[test]
fn general_capture_predicates_gate_later_regex_work_and_keep_errors_evaluation_scoped() {
    let source = format!("# {}", "a".repeat(200_000));
    let pattern = "^# (?:a|aa)*b$";
    assert_eq!(
        value::ByteNfa::compile(pattern)
            .unwrap()
            .test_work_count(source.as_bytes())
            .0,
        Err(Error::WorkLimit)
    );
    let identifier = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("identifier", true);
    let programs = [
        CapturePredicate::AnyOf {
            capture: 0,
            strings: vec![1],
            negated: false,
            any: false,
        },
        CapturePredicate::Kind {
            capture: 0,
            kind: identifier,
            negated: false,
            any: false,
        },
        CapturePredicate::Contains {
            capture: 0,
            strings: vec![2],
            negated: false,
            any: false,
        },
    ]
    .into_iter()
    .map(|first| {
        Program::new_with_capture_predicates(
            vec![
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(ASSERT, 0, 1),
                word(EMIT, 0, 0),
            ],
            vec![
                b"text".to_vec(),
                source.as_bytes().to_vec(),
                b"Z".to_vec(),
                pattern.as_bytes().to_vec(),
            ],
            vec![TextPredicate::Regex {
                source: 3,
                compiled: regex::Regex::new(pattern).unwrap(),
            }],
            vec![
                first,
                CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                },
            ],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
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
        [
            vec![Some(Error::WorkLimit), None, None],
            vec![None, None, None]
        ]
    );
    assert!(expected.outputs.iter().flatten().all(Vec::is_empty));
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
fn ordinary_capture_operators_apply_the_complete_truth_table_at_the_assertion() {
    let cases = operator_cases();
    let programs = cases
        .iter()
        .map(|(predicate, _)| frame_program(predicate.clone(), false))
        .collect::<Vec<_>>();
    let (file, first) = frame();
    let files = [file];
    let anchors = [vec![vec![first]; programs.len()]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        cases.iter().map(|(_, count)| *count).collect::<Vec<_>>()
    );
    for limit in [0, 1] {
        let mut outputs = expected.clone();
        for rows in &mut outputs[0] {
            rows.truncate(limit as usize);
        }
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&vec![limit; programs.len()]),
        )
        .and_then(|execution| execution.into_outcome())
        .and_then(BatchOutcome::into_result);
        assert_eq!(actual, Ok(outputs), "accepted limit={limit}");
    }
}
