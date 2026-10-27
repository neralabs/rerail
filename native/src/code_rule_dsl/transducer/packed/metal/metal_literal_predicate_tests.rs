//! Finite literal predicates preserve public behavior across structural routes.
use super::*;

fn program(literal: &[u8], prefix: bool, route: usize) -> Program {
    let simple = vec![
        word(TEXT, 0, 0),
        word(CAPTURE, 0, 0),
        word(CAPTURE, 0, 0),
        word(EMIT, 0, 0),
    ];
    let mut words = match route {
        0 => simple,
        1 => vec![
            word(FORK, 0, 5),
            word(TEXT, 0, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(TEXT, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        2 => {
            let mut words = vec![word(CHILD, NAMED, 0)];
            words.extend(simple);
            words
        }
        3 => vec![
            word(CHILD, NAMED, 0),
            word(FORK, 0, 6),
            word(TEXT, 0, 0),
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(TEXT, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        _ => unreachable!(),
    };
    let result = Program::new(
        std::mem::take(&mut words),
        vec![literal.to_vec(), b"value".to_vec()],
        vec![if prefix {
            TextPredicate::Prefix(0)
        } else {
            TextPredicate::Equal(0)
        }],
        vec![1],
        vec![],
    )
    .unwrap();
    if route == 3 {
        result.with_tree_sitter_query_semantics().unwrap()
    } else {
        result
    }
}

#[test]
fn finite_literals_filter_before_witness_limits_across_register_and_dynamic_branches() {
    for length in [0, 1, 63, 64] {
        let literal = if length == 0 {
            String::new()
        } else {
            format!("#{}", "A".repeat(length - 1))
        };
        let first = if literal.is_empty() { "#" } else { &literal };
        let files = [tape(&format!("{first}\n{first}X\n#MISS\n"))];
        let programs = [false, true]
            .into_iter()
            .flat_map(|prefix| {
                (0..4).map({
                    let literal = &literal;
                    move |route| program(literal.as_bytes(), prefix, route)
                })
            })
            .collect::<Vec<_>>();
        let anchors = [programs
            .iter()
            .enumerate()
            .map(|(index, _)| {
                if index % 4 < 2 {
                    (1..files[0].transducer_len() as u32).collect()
                } else {
                    vec![0]
                }
            })
            .collect::<Vec<_>>()];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), usize::from(length != 0));
        assert_eq!(expected[0][4].len(), if length <= 1 { 3 } else { 2 });
        for outputs in &expected[0] {
            for output in outputs {
                assert!(!output.captures.is_empty());
            }
        }
        for limit in [0, 1, 2, u32::MAX] {
            let mut limited = expected.clone();
            for outputs in &mut limited[0] {
                outputs.truncate(limit as usize);
            }
            let scope = metal_backend::launch::LaunchScope::enter();
            let actual = metal_program_backend::execute_packed_with_result_limits(
                &programs,
                &files,
                &anchors,
                Some(&[limit; 8]),
            )
            .and_then(|execution| execution.into_outcome())
            .and_then(BatchOutcome::into_result);
            assert_eq!(
                actual,
                Ok(limited),
                "length={length}, accepted limit={limit}"
            );
            let trace = scope.snapshot();
            assert!(trace.len() <= 600);
            assert!(
                trace
                    .iter()
                    .all(|record| record.encoded && record.submitted && record.completed)
            );
        }
    }
}

#[test]
fn finite_literal_register_work_stays_broad_across_programs_and_files() {
    let programs = [
        program(b"#KEEP", false, 0),
        program(b"#", true, 1),
        program(b"#DROP", false, 0),
    ];
    let mut first_launches = None;
    for count in [1, 128] {
        let files = vec![tape("#DROP\n#KEEP\n#KEEPER\n"); count];
        let anchors = vec![vec![vec![1, 2, 3]; programs.len()]; count];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
            [1, 6, 1]
        );
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
        assert_eq!(actual, Ok(expected));
        let trace = scope.snapshot();
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
        assert!(
            trace
                .iter()
                .all(|record| !record.kernel.contains("successor")
                    && !record.kernel.contains("frontier"))
        );
        if let Some(first) = first_launches {
            assert!(trace.len() <= first + 8);
        } else {
            first_launches = Some(trace.len());
        }
        eprintln!(
            "LITERAL_PREDICATE_EVIDENCE {}",
            serde_json::json!({"files":count,"programs":programs.len(),"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|record| record.peak_application_buffer_bytes).max()})
        );
    }
}
