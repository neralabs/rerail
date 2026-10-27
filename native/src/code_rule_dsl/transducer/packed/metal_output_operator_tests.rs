//! Capture selection and source-derived properties preserve directive order.
use super::*;

fn observe_output<T>(
    scenario: &str,
    execute: impl FnOnce() -> Result<T, Error>,
) -> Result<T, Error> {
    let scope = metal_backend::launch::LaunchScope::enter();
    let start = std::time::Instant::now();
    let actual = execute();
    let seconds = start.elapsed().as_secs_f64();
    let trace = scope.snapshot();
    if actual.is_ok() {
        assert!(!trace.is_empty());
        assert!(trace.len() <= 600);
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
    }
    let mut kernels = std::collections::BTreeMap::<String, usize>::new();
    for record in &trace {
        *kernels.entry(record.kernel.clone()).or_default() += 1;
    }
    eprintln!(
        "OUTPUT_OPERATORS_EVIDENCE {}",
        serde_json::json!({
            "scenario": scenario, "launches": trace.len(), "seconds": seconds,
            "peakMetalBytes": trace.iter().map(|record| record.peak_application_buffer_bytes).max(),
            "kernels": kernels,
        })
    );
    actual
}

#[test]
fn selection_handles_empty_groups_missing_targets_and_zero_length_mime_values() {
    let make = |capture, directives| {
        let mut words = Vec::new();
        if let Some(label) = capture {
            words.push(word(CAPTURE, 0, label));
        }
        words.push(word(EMIT, 0, 0));
        Program::new(
            words,
            vec![b"value".to_vec(), b"target".to_vec()],
            vec![],
            vec![0, 1],
            vec![],
        )
        .unwrap()
        .with_directives(directives)
        .unwrap()
    };
    let programs = [
        make(
            Some(0),
            vec![Directive::SelectAdjacent {
                capture: 0,
                target: 1,
            }],
        ),
        make(
            None,
            vec![Directive::SelectAdjacent {
                capture: 0,
                target: 1,
            }],
        ),
        make(
            Some(1),
            vec![Directive::SelectAdjacent {
                capture: 0,
                target: 1,
            }],
        ),
        make(
            Some(0),
            vec![Directive::SetLanguageFromMimeType { capture: 0 }],
        ),
        make(
            None,
            vec![Directive::SetLanguageFromMimeType { capture: 0 }],
        ),
        make(
            Some(0),
            vec![
                Directive::SelectAdjacent {
                    capture: 0,
                    target: 0,
                },
                Directive::SetLanguageFromMimeType { capture: 0 },
            ],
        ),
    ];
    let files = [tape(""), tape("# VALUE\n")];
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    for file in &expected {
        assert_eq!(
            file.iter().map(Vec::len).collect::<Vec<_>>(),
            [1, 1, 1, 1, 0, 1]
        );
    }
    assert!(expected[0][1][0].captures.is_empty());
    assert_eq!(
        expected[0][3][0].dynamic_properties,
        vec![(b"language".to_vec(), Some(Vec::new()))]
    );
    assert_eq!(
        observe_output("selection-empty-groups", || {
            execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors)
        }),
        Ok(expected)
    );
}

#[test]
fn query_order_after_capture_removal_uses_original_candidate_ordinals() {
    let program = Program::new(
        vec![
            word(FORK, 0, 5),
            word(CHILD, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 0),
            word(CHILD, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(NEXT, NAMED, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 0),
        ],
        vec![b"comment".to_vec(), b"target".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
    .with_directives(vec![Directive::SelectAdjacent {
        capture: 0,
        target: 1,
    }])
    .unwrap();
    let programs = [program];
    let files = [tape("# FIRST\n\n# SECOND\n")];
    let anchors = [vec![vec![0]]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0][0]
            .iter()
            .map(|output| output.local_ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(expected[0][0][2].captures.len(), 1);
    assert_eq!(
        expected[0][0][1].captures[0].node,
        expected[0][0][2].captures[0].node
    );
    assert_eq!(
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
        Ok(expected)
    );
}

#[test]
fn adjacent_capture_selection_preserves_properties_and_mime_snapshots_in_directive_order() {
    let program = Program::new(
        vec![
            word(CHILD, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(NEXT, NAMED, 0),
            word(CAPTURE, 0, 0),
            word(NEXT, NAMED, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 0),
        ],
        vec![
            b"comment".to_vec(),
            b"target".to_vec(),
            b"before".to_vec(),
            b"value".to_vec(),
            b"after".to_vec(),
        ],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::SetCapture {
            capture: 0,
            key: 2,
            value: Some(3),
        },
        Directive::SetLanguageFromMimeType { capture: 0 },
        Directive::SelectAdjacent {
            capture: 0,
            target: 1,
        },
        Directive::Downcase { capture: 0 },
        Directive::SetCapture {
            capture: 0,
            key: 4,
            value: Some(3),
        },
        Directive::SelectAdjacent {
            capture: 0,
            target: 1,
        },
        Directive::SetLanguageFromMimeType { capture: 0 },
    ])
    .unwrap();
    let programs = [program];
    let files = [
        tape("# FAR\n\n# CLOSE\nvalue\n"),
        tape("# FIRST\n# SECOND\nmodule\n"),
    ];
    let anchors = vec![vec![vec![0]]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0][0].captures.len(), 2);
    assert_eq!(expected[1][0][0].captures.len(), 3);
    assert_eq!(
        expected[0][0][0].captures[0].text.as_deref(),
        Some(b"# close".as_slice())
    );
    assert_eq!(
        expected[0][0][0].captures[0].properties,
        vec![(2, Some(3)), (4, Some(3))]
    );
    assert_eq!(
        expected[0][0][0].dynamic_properties,
        vec![
            (b"language".to_vec(), Some(b"# FAR".to_vec())),
            (b"language".to_vec(), Some(b"# CLOSE".to_vec())),
        ]
    );
    assert_eq!(
        observe_output("adjacent-captures-and-mime-snapshots", || {
            execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors)
        }),
        Ok(expected)
    );
}

#[test]
fn mime_properties_use_source_snapshots_and_reject_missing_captures_before_witness_limits() {
    let base = Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"mime".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::Offset {
            capture: 0,
            offsets: [0, 2, 0, 0],
        },
        Directive::Downcase { capture: 0 },
        Directive::SetLanguageFromMimeType { capture: 0 },
        Directive::Offset {
            capture: 0,
            offsets: [0, 0, 0, -1],
        },
        Directive::SetLanguageFromMimeType { capture: 0 },
    ])
    .unwrap();
    let absent = Program::new(
        vec![word(EMIT, 0, 0)],
        vec![b"mime".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::SetLanguageFromMimeType { capture: 0 }])
    .unwrap();
    let programs = [base, absent];
    let long = format!("# application/{}/PYTHON\n", "A".repeat(4097));
    let files = [
        "# importmap\n",
        "# module\n",
        "# application/ecmascript\n",
        "# text/ecmascript\n",
        &long,
    ]
    .into_iter()
    .map(tape)
    .collect::<Vec<_>>();
    let anchors = vec![vec![vec![1]; programs.len()]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    for (index, file) in expected.iter().enumerate() {
        assert_eq!(file[0].len(), 1);
        assert!(file[1].is_empty());
        assert_eq!(file[0][0].captures[0], file[0][0].captures[1]);
        assert_eq!(file[0][0].dynamic_properties.len(), 2);
        let first: &[u8] = if index == 0 {
            b"json".as_slice()
        } else if index < 4 {
            b"javascript"
        } else {
            b"PYTHON"
        };
        assert_eq!(file[0][0].dynamic_properties[0].1.as_deref(), Some(first));
    }
    assert_eq!(
        expected[4][0][0].dynamic_properties[1].1.as_deref(),
        Some(b"PYTHO".as_slice())
    );
    for limit in [0, 1, 2] {
        let mut outputs = expected.clone();
        for evaluation in outputs.iter_mut().flatten() {
            evaluation.truncate(limit as usize);
        }
        let actual = observe_output(&format!("mime-source-snapshots-limit-{limit}"), || {
            metal_grid_backend::execute_packed_with_result_limits(
                &programs,
                &files,
                &anchors,
                Some(&[limit; 2]),
            )
            .and_then(|execution| execution.into_outcome())
            .and_then(BatchOutcome::into_result)
        });
        assert_eq!(actual, Ok(outputs), "accepted witness limit {limit}");
    }
}
