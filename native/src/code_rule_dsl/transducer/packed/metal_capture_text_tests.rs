//! Shared terminal capture predicates preserve quantifiers and publication.
use super::*;

#[test]
fn query_text_quantifiers_preserve_empty_groups_duplicate_captures_and_transform_order() {
    let mut programs = Vec::new();
    for prefix in [false, true] {
        for any in [false, true] {
            for negated in [false, true] {
                programs.push(
                    Program::new_with_capture_predicates(
                        vec![
                            word(CAPTURE, 0, 0),
                            word(CAPTURE, 0, 0),
                            word(NEXT, NAMED, 0),
                            word(CAPTURE, 0, 0),
                            word(ASSERT, 0, 0),
                            word(EMIT, 0, 0),
                        ],
                        vec![
                            if prefix {
                                b"#".to_vec()
                            } else {
                                b"#KEEP".to_vec()
                            },
                            b"value".to_vec(),
                            b"role".to_vec(),
                            b"kept".to_vec(),
                        ],
                        vec![if prefix {
                            TextPredicate::Prefix(0)
                        } else {
                            TextPredicate::Equal(0)
                        }],
                        vec![CapturePredicate::Text {
                            capture: 0,
                            predicate: 0,
                            negated,
                            any,
                        }],
                        vec![1],
                        vec![],
                    )
                    .unwrap()
                    .with_tree_sitter_query_semantics()
                    .unwrap()
                    .with_directives(vec![
                        Directive::Set {
                            key: 2,
                            value: Some(3),
                        },
                        Directive::Offset {
                            capture: 0,
                            offsets: [0, 1, 0, 0],
                        },
                        Directive::Downcase { capture: 0 },
                    ])
                    .unwrap(),
                );
            }
        }
    }
    for any in [false, true] {
        for negated in [false, true] {
            programs.push(
                Program::new_with_capture_predicates(
                    vec![word(CAPTURE, 0, 1), word(ASSERT, 0, 0), word(EMIT, 0, 1)],
                    vec![b"#KEEP".to_vec(), b"absent".to_vec(), b"present".to_vec()],
                    vec![TextPredicate::Equal(0)],
                    vec![CapturePredicate::Text {
                        capture: 0,
                        predicate: 0,
                        negated,
                        any,
                    }],
                    vec![1, 2],
                    vec![],
                )
                .unwrap()
                .with_tree_sitter_query_semantics()
                .unwrap(),
            );
        }
    }
    for negated in [false, true] {
        let template = &programs[2];
        let mut strings = template.strings.clone();
        strings.push(b"#".to_vec());
        programs.push(
            Program::new_with_capture_predicates(
                template.words.clone(),
                strings,
                vec![TextPredicate::Equal(0), TextPredicate::Prefix(4)],
                vec![
                    CapturePredicate::Text {
                        capture: 0,
                        predicate: 0,
                        negated: false,
                        any: true,
                    },
                    CapturePredicate::Text {
                        capture: 0,
                        predicate: 1,
                        negated,
                        any: false,
                    },
                ],
                template.labels.clone(),
                template.longs.clone(),
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
            .with_directives(template.directives.clone())
            .unwrap(),
        );
    }
    let files = [tape("#KEEP\n#DROP\n#KEEP\n")];
    let anchors = [vec![vec![1, 2]; programs.len()]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [0, 0, 2, 2, 2, 0, 2, 0, 0, 0, 0, 0, 2, 0]
    );
    for outputs in &expected[0][..8] {
        for output in outputs {
            assert_eq!(output.captures.len(), 3);
            assert_eq!(output.captures[0], output.captures[1]);
            assert_eq!(output.properties.len(), 1);
            assert!(
                output
                    .captures
                    .iter()
                    .all(|capture| matches!(capture.text.as_deref(), Some(b"keep" | b"drop")))
            );
        }
    }
    for limit in [0, 1, 2, u32::MAX] {
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
fn shared_query_capture_text_batches_programs_and_files_without_output_directives() {
    let language = SupportLang::Python.get_ts_language();
    let programs = [
        "((comment) @value (#eq? @value \"#KEEP\"))",
        "((comment) @value (#not-eq? @value \"#DROP\"))",
    ]
    .iter()
    .map(|query| Program::compile_tree_sitter_query_pattern(query, &language).unwrap())
    .collect::<Vec<_>>();
    let mut first_launches = None;
    for count in [1, 128] {
        let files = vec![tape("#DROP\n#KEEP\n#LAST\n"); count];
        let anchors = vec![vec![vec![1, 2, 3]; programs.len()]; count];
        let mut expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0].iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);
        for file in &mut expected {
            for outputs in file {
                outputs.truncate(1);
            }
        }
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[1, 1]),
        )
        .and_then(|execution| execution.into_outcome())
        .and_then(BatchOutcome::into_result);
        assert_eq!(actual, Ok(expected));
        let trace = scope.snapshot();
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
        if let Some(first) = first_launches {
            assert!(trace.len() <= first + 16);
        } else {
            first_launches = Some(trace.len());
        }
        eprintln!(
            "CAPTURE_TEXT_EVIDENCE {}",
            serde_json::json!({"files":count,"programs":programs.len(),"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|record|record.peak_application_buffer_bytes).max()})
        );
    }
}
