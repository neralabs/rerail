//! Query text values retain their early structural PC and inherited path budget.
use super::*;

#[test]
fn query_inline_text_values_preserve_false_paths_and_inherited_work_before_later_operands() {
    let pattern = "^# (?:a|aa)*$";
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let (source, cost) = [20_000, 40_000, 60_000, 80_000, 100_000, 130_000]
        .into_iter()
        .find_map(|length| {
            let source = format!("# {}", "a".repeat(length));
            let (matched, cost) = nfa.test_work_count(source.as_bytes());
            (matched == Ok(true) && cost > MAX_STEPS_PER_PATH / 2 && cost + 3 < MAX_STEPS_PER_PATH)
                .then_some((source, cost))
        })
        .unwrap();
    assert!(cost > MAX_STEPS_PER_PATH / 2 && cost + 3 < MAX_STEPS_PER_PATH);
    let programs = [
        vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![
            word(TEXT, 0, 0),
            word(TEXT, 0, 0),
            word(RESTORE_CURSOR, 0, 5),
            word(EMIT, 0, 0),
        ],
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(TEXT, 0, 0),
            word(REPEAT_END, 0, 0),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(TEXT, 0, 0),
            word(TEXT, 0, 0),
            word(RESTORE_CURSOR, 0, 5),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
    ]
    .into_iter()
    .map(|words| {
        Program::new(
            words,
            vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
            vec![TextPredicate::Regex {
                source: 1,
                compiled: regex::Regex::new(pattern).unwrap(),
            }],
            vec![0],
            vec![],
        )
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap()
    })
    .collect::<Vec<_>>();
    let files = [tape(&source), tape("# rejected")];
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
            vec![None, Some(Error::WorkLimit), None, Some(Error::WorkLimit)],
            vec![None; 4]
        ]
    );
    assert_eq!(expected.outputs[0][0].len(), 1);
    assert!(expected.outputs[0][1].is_empty());
    assert_eq!(expected.outputs[0][2].len(), 1);
    assert!(expected.outputs[0][3].is_empty());
    let native =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(native.errors, expected.errors);
    assert_eq!(native.outputs, expected.outputs);
    for limit in [0, 1] {
        let mut outputs = expected.outputs.clone();
        for rows in outputs.iter_mut().flatten() {
            rows.truncate(limit as usize);
        }
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 4]),
        )
        .and_then(|e| e.into_outcome())
        .unwrap();
        assert_eq!(actual.errors, expected.errors);
        assert_eq!(actual.outputs, outputs);
    }
}
struct QueryWorkSource {
    pattern: &'static str,
    source: String,
    cost: usize,
}

#[test]
fn bounded_body_admission_preserves_guarded_capture_identity_and_multiplicity() {
    let language = SupportLang::Python.get_ts_language();
    let comment = language.id_for_node_kind("comment", true);
    let identifier = language.id_for_node_kind("identifier", true);
    let cases: [(bool, &[u16], &[u16]); 7] = [
        (true, &[], &[]),
        (false, &[], &[]),
        (true, &[1], &[2]),
        (true, &[1, 2], &[2, 1]),
        (true, &[1, 1], &[1]),
        (false, &[1], &[2]),
        (true, &[1], &[1]),
    ];
    let programs = cases
        .iter()
        .map(|&(passes, earlier, later)| {
            let mut words = vec![
                word(CHILD, NAMED, 0),
                word(FIRST, NAMED, 0),
                word(NEXT, NAMED, 0),
                word(CAPTURE, 0, 0),
                word(REPEAT_BEGIN, 0, 0),
                word(REPEAT_BUMP, 0, 0),
                word(FORK, 0, 0),
                word(NEXT, NAMED, 0),
                word(FORK, 0, 0),
            ];
            words.extend(earlier.iter().map(|&label| word(CAPTURE, 0, label)));
            words.push(word(KIND, 0, if passes { comment } else { identifier }));
            let earlier_jump = words.len();
            words.push(word(GOTO, 0, 0));
            let alternate = words.len();
            words[8] = word(FORK, 0, alternate as u16);
            words.extend([word(FLAGS, 0, 0); 2]);
            words.extend(later.iter().map(|&label| word(CAPTURE, 0, label)));
            let join = words.len();
            words[earlier_jump] = word(GOTO, 0, join as u16);
            words.extend([word(REPEAT_BUMP, 0, 0), word(GOTO, 0, 6)]);
            let exit = words.len();
            words[6] = word(FORK, 0, exit as u16);
            words.extend([word(REPEAT_END, 0, 0), word(EMIT, 0, 0)]);
            Program::new(
                words,
                vec![b"root".to_vec(), b"a".to_vec(), b"b".to_vec()],
                vec![],
                vec![0, 1, 2],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = [tape("# a\n# b\n# c\n")];
    let anchors = [vec![vec![0]; programs.len()]];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(expected.errors, [vec![None; 7]]);
    assert_eq!(
        expected.outputs[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1, 2, 2, 1, 1, 1]
    );
    for (index, outputs) in expected.outputs[0].iter().enumerate() {
        for output in outputs {
            assert_eq!(output.captures[0].label, 0);
            assert_eq!(output.captures[0].node, 2);
            assert!(output.captures[1..].iter().all(|capture| capture.node == 3));
        }
        if index == 4 {
            assert_eq!(
                outputs[0]
                    .captures
                    .iter()
                    .map(|capture| capture.label)
                    .collect::<Vec<_>>(),
                [0, 1, 1]
            );
        }
        if index == 5 {
            assert_eq!(
                outputs[0]
                    .captures
                    .iter()
                    .map(|capture| capture.label)
                    .collect::<Vec<_>>(),
                [0, 2]
            );
        }
    }
    for backend in [ExecutionBackend::CpuJit, ExecutionBackend::Metal] {
        let actual = execute_with_backend_outcomes(backend, &programs, &files, &anchors).unwrap();
        assert_eq!(actual.errors, expected.errors);
        assert_eq!(actual.outputs, expected.outputs);
    }
}

#[test]
fn bounded_body_join_admits_equal_frames_before_charging_later_work() {
    let QueryWorkSource {
        pattern,
        source,
        cost,
    } = near_query_work_source();
    let padding = MAX_STEPS_PER_PATH - cost - 32;
    let programs = (0..4)
        .map(|remaining| {
            let mut words = vec![
                word(TEXT, 0, 0),
                word(CHILD, NAMED, 0),
                word(FIRST, NAMED, 0),
                word(NEXT, NAMED, 0),
                word(CAPTURE, 0, 0),
            ];
            words.extend(vec![word(FLAGS, 0, 0); padding]);
            words.extend([word(REPEAT_BEGIN, 0, 0), word(REPEAT_BUMP, 0, 0)]);
            let header = words.len();
            words.extend([
                word(FORK, 0, 0),
                word(NEXT, NAMED, 0),
                word(FORK, 0, 0),
                word(GOTO, 0, 0),
            ]);
            let later = words.len();
            words[header + 2] = word(FORK, 0, later as u16);
            words.extend(vec![word(FLAGS, 0, 0); 23 - remaining]);
            let join = words.len();
            words[header + 3] = word(GOTO, 0, join as u16);
            words.extend([word(REPEAT_BUMP, 0, 0), word(GOTO, 0, header as u16)]);
            let exit = words.len();
            words[header] = word(FORK, 0, exit as u16);
            words.extend([word(REPEAT_END, 0, 0), word(EMIT, 0, 0)]);
            Program::new(
                words,
                vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
                vec![TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                }],
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
    assert_eq!(
        expected.errors,
        [vec![Some(Error::WorkLimit), None, None, None]]
    );
    for (index, rows) in expected.outputs[0].iter().enumerate() {
        assert_eq!(rows.len(), usize::from(index != 0));
        if index != 0 {
            assert_eq!(rows[0].captures.len(), 1);
            assert_eq!(rows[0].captures[0].node, 2);
        }
    }
    let native =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(native.errors, expected.errors);
    assert_eq!(native.outputs, expected.outputs);
    for limit in [0, 1] {
        let scope = metal_backend::launch::LaunchScope::enter();
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 4]),
        )
        .unwrap()
        .into_outcome()
        .unwrap();
        assert_eq!(actual.errors, expected.errors);
        let mut outputs = expected.outputs.clone();
        for rows in &mut outputs[0] {
            rows.truncate(limit as usize);
        }
        assert_eq!(actual.outputs, outputs);
        let trace = scope.snapshot();
        assert!(trace.len() <= 600);
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        eprintln!(
            "QUERY_BODY_JOIN_EVIDENCE {}",
            serde_json::json!({
                "limit": limit, "launches": trace.len(),
                "peakMetalBytes": trace.iter().map(|r| r.peak_application_buffer_bytes).max()
            })
        );
    }
}

fn near_query_work_source() -> QueryWorkSource {
    let pattern = "^# (?:a|aa)*\\n# b\\n# c\\n$";
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let make_source = |length| format!("# {}\n# b\n# c\n", "a".repeat(length));
    let base = 20_000;
    let (matched, base_cost) = nfa.test_work_count(make_source(base).as_bytes());
    let (next, next_cost) = nfa.test_work_count(make_source(base + 1).as_bytes());
    assert_eq!((matched, next), (Ok(true), Ok(true)));
    let stride = next_cost.checked_sub(base_cost).unwrap();
    assert!(stride > 0);
    let estimate = base + (MAX_STEPS_PER_PATH - 80 - base_cost) / stride;
    let (source, cost) = (estimate.saturating_sub(2)..=estimate + 2)
        .find_map(|length| {
            let source = make_source(length);
            let (matched, cost) = nfa.test_work_count(source.as_bytes());
            (matched == Ok(true) && (64..=96).contains(&(MAX_STEPS_PER_PATH - cost)))
                .then_some((source, cost))
        })
        .unwrap();
    QueryWorkSource {
        pattern,
        source,
        cost,
    }
}

#[test]
fn query_bulk_and_bounded_jobs_keep_exact_budget_edges_and_logical_failure_precedence() {
    let QueryWorkSource {
        pattern,
        source,
        cost,
    } = near_query_work_source();
    let language = SupportLang::Python.get_ts_language();
    let comment = language.id_for_node_kind("comment", true);
    let identifier = language.id_for_node_kind("identifier", true);
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    for shape in 0..3 {
        for remaining in [0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 40] {
            let mut words = vec![
                word(TEXT, 0, 0),
                word(CHILD, NAMED, 0),
                word(FIRST, NAMED, 0),
                word(CAPTURE, 0, 0),
            ];
            let padding = MAX_STEPS_PER_PATH - cost - words.len() - 2 - remaining;
            words.extend(vec![word(FLAGS, 0, 0); padding]);
            words.extend([word(REPEAT_BEGIN, 0, 0), word(REPEAT_BUMP, 0, 0)]);
            let header = words.len();
            words.push(word(FORK, if shape == 2 { 0 } else { GREEDY }, 0));
            words.push(word(NEXT, if shape == 2 { NAMED } else { 0 }, 0));
            if shape == 2 {
                let fork = words.len();
                words.push(word(FORK, 0, 0));
                words.extend([word(KIND, 0, comment), word(CAPTURE, 0, 1)]);
                let jump = words.len();
                words.push(word(GOTO, 0, 0));
                let alternate = words.len();
                words[fork] = word(FORK, 0, alternate as u16);
                words.extend(vec![word(FLAGS, 0, 0); 16]);
                words.extend([word(KIND, 0, identifier), word(CAPTURE, 0, 1)]);
                let bump = words.len();
                words[jump] = word(GOTO, 0, bump as u16);
            } else {
                words.push(word(KIND, 0, if shape == 0 { identifier } else { comment }));
            }
            words.extend([word(REPEAT_BUMP, 0, 0), word(GOTO, 0, header as u16)]);
            let exit = words.len();
            words[header] = word(FORK, if shape == 2 { 0 } else { GREEDY }, exit as u16);
            words.push(word(REPEAT_END, 0, 0));
            let restore = words.len();
            words.extend([word(RESTORE_CURSOR, 0, 5), word(EMIT, 0, 0)]);
            programs.push(
                Program::new(
                    words,
                    vec![
                        b"item".to_vec(),
                        b"iteration".to_vec(),
                        pattern.as_bytes().to_vec(),
                    ],
                    vec![TextPredicate::Regex {
                        source: 2,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }],
                    vec![0, 1],
                    vec![],
                )
                .unwrap()
                .with_tree_sitter_query_semantics()
                .unwrap(),
            );
            cases.push((shape, remaining, restore));
        }
    }
    let files = [tape(&source)];
    let anchors = [vec![vec![0]; programs.len()]];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    for (index, &(shape, remaining, restore)) in cases.iter().enumerate() {
        let minimum = match shape {
            0 => 3,
            1 => 13,
            2 => 19,
            _ => unreachable!(),
        };
        assert_eq!(
            expected.errors[0][index],
            Some(if remaining < minimum {
                Error::WorkLimit
            } else {
                Error::InvalidOperand(restore)
            }),
            "shape={shape}, remaining={remaining}, value cost={cost}"
        );
        assert!(expected.outputs[0][index].is_empty());
    }
    let native =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(native.errors, expected.errors);
    assert_eq!(native.outputs, expected.outputs);
    for limit in [0, 1] {
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&vec![limit; programs.len()]),
        )
        .and_then(|e| e.into_outcome());
        let trace = scope.snapshot();
        let mut kernels = std::collections::BTreeMap::<String, usize>::new();
        for record in &trace {
            *kernels.entry(record.kernel.clone()).or_default() += 1;
        }
        eprintln!(
            "QUERY_WORK_JOB_ATTEMPT {}",
            serde_json::json!({
                "limit":limit, "error":actual.as_ref().err().map(|error|format!("{error:?}")),
                "launches":trace.len(), "kernels":kernels,
                "lastKernels":trace.iter().rev().take(20).map(|record|&record.kernel).collect::<Vec<_>>()
            })
        );
        let actual = actual.unwrap();
        assert_eq!(actual.errors, expected.errors);
        assert_eq!(actual.outputs, expected.outputs);
        assert!(trace.len() <= 600);
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        eprintln!(
            "QUERY_WORK_JOB_EVIDENCE {}",
            serde_json::json!({"limit":limit,"programs":programs.len(),"valueCost":cost,"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
        );
    }
}

#[test]
fn bulk_work_accounts_for_the_losing_attempt_before_an_earlier_entry_or_guard() {
    let QueryWorkSource {
        pattern,
        source,
        cost,
    } = near_query_work_source();
    let language = SupportLang::Python.get_ts_language();
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    for (kind, minimum, duplicate) in [
        (language.id_for_node_kind("comment", true), 5, false),
        (language.id_for_node_kind("identifier", true), 3, false),
        (language.id_for_node_kind("comment", true), 0, true),
    ] {
        for remaining in 0..=if duplicate { 2 } else { 6 } {
            let mut words = vec![
                word(TEXT, 0, 0),
                word(CHILD, NAMED, 0),
                word(FIRST, NAMED, 0),
                word(CAPTURE, 0, 0),
            ];
            words.extend(vec![
                word(FLAGS, 0, 0);
                MAX_STEPS_PER_PATH - cost - 28 - remaining
            ]);
            words.push(word(REPEAT_BEGIN, 0, 0));
            let fork = words.len();
            words.push(word(FORK, 0, 0));
            if !duplicate {
                words.push(word(NEXT, 0, 0));
            }
            let older_jump = words.len();
            words.push(word(GOTO, 0, 0));
            let later = words.len();
            words[fork] = word(FORK, 0, later as u16);
            if duplicate {
                words.extend([word(GOTO, 0, (later + 2) as u16), word(STOP, 0, 0)]);
            }
            // The duplicate reaches the identical BUMP with no work left.
            // Admission suppresses it before that instruction can fail.
            words.extend(vec![word(FLAGS, 0, 0); 20]);
            let later_jump = words.len();
            words.push(word(GOTO, 0, 0));
            let bump = words.len();
            words[older_jump] = word(GOTO, 0, bump as u16);
            words[later_jump] = word(GOTO, 0, bump as u16);
            words.push(word(REPEAT_BUMP, 0, 0));
            let header = words.len();
            words.push(word(FORK, GREEDY, 0));
            words.extend([
                word(NEXT, 0, 0),
                word(KIND, 0, kind),
                word(REPEAT_BUMP, 0, 0),
                word(GOTO, 0, header as u16),
            ]);
            let exit = words.len();
            words[header] = word(FORK, GREEDY, exit as u16);
            words.extend([word(REPEAT_END, 0, 0), word(EMIT, 0, 0)]);
            programs.push(
                Program::new(
                    words,
                    vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
                    vec![TextPredicate::Regex {
                        source: 1,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }],
                    vec![0],
                    vec![],
                )
                .unwrap()
                .with_tree_sitter_query_semantics()
                .unwrap(),
            );
            cases.push((kind, minimum, remaining));
        }
    }
    let files = [tape(&source)];
    let anchors = [vec![vec![0]; programs.len()]];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    for (index, &(kind, minimum, remaining)) in cases.iter().enumerate() {
        assert_eq!(
            expected.errors[0][index],
            (remaining < minimum).then_some(Error::WorkLimit),
            "kind={kind}, remaining={remaining}"
        );
        assert_eq!(
            expected.outputs[0][index].len(),
            usize::from(remaining >= minimum)
        );
        if remaining >= minimum {
            assert_eq!(expected.outputs[0][index][0].captures.len(), 1);
            assert_eq!(expected.outputs[0][index][0].captures[0].node, 1);
        }
    }
    let native =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(native.errors, expected.errors);
    assert_eq!(native.outputs, expected.outputs);
    let scope = metal_backend::launch::LaunchScope::enter();
    let actual =
        execute_with_backend_outcomes(ExecutionBackend::Metal, &programs, &files, &anchors);
    let trace = scope.snapshot();
    let mut kernels = std::collections::BTreeMap::<String, usize>::new();
    for record in &trace {
        *kernels.entry(record.kernel.clone()).or_default() += 1;
    }
    eprintln!(
        "QUERY_BULK_CUT_ATTEMPT {}",
        serde_json::json!({"error":actual.as_ref().err().map(|error|format!("{error:?}")),"launches":trace.len(),"kernels":kernels,"lastKernels":trace.iter().rev().take(20).map(|record|&record.kernel).collect::<Vec<_>>() })
    );
    let actual = actual.unwrap();
    assert_eq!(actual.errors, expected.errors);
    assert_eq!(actual.outputs, expected.outputs);
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
}

#[test]
fn query_join_suppresses_later_duplicate_work_but_retains_errors_before_the_join() {
    let QueryWorkSource {
        pattern,
        source,
        cost,
    } = near_query_work_source();
    let rejected_kind = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("identifier", true);
    let programs = (0..12)
        .map(|case| {
            let remaining = case % 4;
            let words = if case < 4 {
                let mut words = vec![
                    word(TEXT, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(REPEAT_BEGIN, 0, 0),
                    word(FORK, 0, 5),
                    word(GOTO, 0, 0),
                ];
                words.extend(vec![
                    word(FLAGS, 0, 0);
                    MAX_STEPS_PER_PATH - cost - 4 - remaining
                ]);
                let later_jump = words.len();
                words.push(word(GOTO, 0, 0));
                let join = words.len();
                words[4] = word(GOTO, 0, join as u16);
                words[later_jump] = word(GOTO, 0, join as u16);
                words.extend([word(REPEAT_END, 0, 0), word(EMIT, 0, 0)]);
                words
            } else if case < 8 {
                // The first logical branch reaches the final join at PC 5.
                // The later branch reaches a higher-PC join that leads back
                // to PC 5. Component order must hold PC 5 until that path runs.
                let mut words = vec![
                    word(TEXT, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(REPEAT_BEGIN, 0, 0),
                    word(FORK, 0, 7),
                    word(GOTO, 0, 5),
                    word(REPEAT_END, 0, 0),
                    word(EMIT, 0, 0),
                    word(FORK, 0, 10),
                    word(KIND, 0, rejected_kind),
                    word(GOTO, 0, 0),
                ];
                words.extend(vec![
                    word(FLAGS, 0, 0);
                    MAX_STEPS_PER_PATH - cost - 6 - remaining
                ]);
                let later_jump = words.len();
                words.push(word(GOTO, 0, 0));
                let join = words.len();
                words[9] = word(GOTO, 0, join as u16);
                words[later_jump] = word(GOTO, 0, join as u16);
                words.push(word(GOTO, 0, 5));
                words
            } else {
                // The bounded repetition's exit shares its closing instruction
                // with a later branch. Its fused footer must stop before that
                // admitted join, just as a scalar prefix does.
                let mut words = vec![
                    word(TEXT, 0, 0),
                    word(CHILD, NAMED, 0),
                    word(FIRST, NAMED, 0),
                    word(CAPTURE, 0, 0),
                    word(REPEAT_BEGIN, 0, 0),
                    word(FORK, 0, 14),
                    word(REPEAT_BUMP, 0, 0),
                    word(FORK, 0, 12),
                    word(NEXT, 0, 0),
                    word(KIND, 0, rejected_kind),
                    word(REPEAT_BUMP, 0, 0),
                    word(GOTO, 0, 7),
                    word(REPEAT_END, 0, 0),
                    word(EMIT, 0, 0),
                ];
                words.extend(vec![
                    word(FLAGS, 0, 0);
                    MAX_STEPS_PER_PATH - cost - 6 - remaining
                ]);
                words.push(word(GOTO, 0, 12));
                words
            };
            Program::new(
                words,
                vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
                vec![TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                }],
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
    assert_eq!(
        expected.errors,
        [vec![
            Some(Error::WorkLimit),
            None,
            None,
            None,
            Some(Error::WorkLimit),
            None,
            None,
            None,
            Some(Error::WorkLimit),
            None,
            None,
            None
        ]]
    );
    for (index, outputs) in expected.outputs[0].iter().enumerate() {
        assert_eq!(outputs.len(), usize::from(index % 4 != 0));
        if index % 4 != 0 {
            assert_eq!(outputs[0].captures.len(), 1);
            assert_eq!(outputs[0].captures[0].node, u32::from(index >= 8));
        }
    }
    let native =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(native.errors, expected.errors);
    assert_eq!(native.outputs, expected.outputs);
    for limit in [0, 1] {
        let scope = metal_backend::launch::LaunchScope::enter();
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&vec![limit; programs.len()]),
        )
        .unwrap()
        .into_outcome()
        .unwrap();
        assert_eq!(actual.errors, expected.errors);
        let mut outputs = expected.outputs.clone();
        for outputs in &mut outputs[0] {
            outputs.truncate(limit as usize);
        }
        assert_eq!(actual.outputs, outputs);
        let trace = scope.snapshot();
        assert!(trace.len() <= 600);
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
        eprintln!(
            "QUERY_JOIN_EVIDENCE {}",
            serde_json::json!({"limit":limit,"programs":programs.len(),"launches":trace.len(),"peakMetalBytes":trace.iter().map(|record|record.peak_application_buffer_bytes).max()})
        );
    }
}
