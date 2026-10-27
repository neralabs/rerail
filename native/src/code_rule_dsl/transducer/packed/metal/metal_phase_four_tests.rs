//! Production publication contracts, independent of predicate scheduling details.
use super::*;

#[path = "metal_capture_arguments_tests.rs"]
mod capture_arguments;
#[path = "metal_capture_operator_tests.rs"]
mod capture_operators;
#[path = "metal_capture_text_tests.rs"]
mod capture_text;
#[path = "metal_inline_predicate_tests.rs"]
mod inline_predicates;
#[path = "metal_literal_predicate_tests.rs"]
mod literal_predicates;
#[path = "metal_lua_frontier_tests.rs"]
mod lua_frontier;
#[path = "metal_output_operator_tests.rs"]
mod output_operators;
#[path = "metal_output_page_tests.rs"]
mod output_pages;
#[path = "metal_program_table_tests.rs"]
mod program_tables;
#[path = "metal_query_dispatch_tests.rs"]
mod query_dispatch;
#[path = "metal_query_inline_predicate_tests.rs"]
mod query_inline_predicates;
#[path = "metal_regex_frontier_tests.rs"]
mod regex_frontier;
#[path = "metal_strip_output_tests.rs"]
mod strip_output;
#[path = "metal_terminal_repeat_work_tests.rs"]
mod terminal_repeat_work;
#[path = "metal_value_request_contract_tests.rs"]
mod value_request_contract;

#[test]
fn quantified_middle_captures_preserve_each_owned_node_across_all_resident_roots() {
    let query = "(module (comment)+ (comment) @middle (comment)+)";
    let programs =
        [
            Program::compile_tree_sitter_query_pattern(
                query,
                &SupportLang::Python.get_ts_language(),
            )
            .unwrap(),
        ];
    let source = (0..128)
        .map(|index| format!("# {index}\n"))
        .collect::<String>();
    let files = [tape(&source)];
    let anchors = [vec![
        (0..files[0].transducer_len() as u32).collect::<Vec<_>>(),
    ]];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), 126);
    for (index, output) in expected[0][0].iter().enumerate() {
        assert_eq!(output.captures.len(), 1);
        let node = index as u32 + 2;
        assert_eq!(output.captures[0].node, node);
        assert_eq!(output.captures[0].span, files[0].transducer_span(node));
    }
    assert_eq!(
        execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap(),
        expected
    );
    let scope = metal_backend::launch::LaunchScope::enter();
    let started = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
    let trace = scope.snapshot();
    eprintln!(
        "QUANTIFIED_MIDDLE_ATTEMPT {}",
        serde_json::json!({
            "attempts":trace.len(),"encoded":trace.iter().filter(|r|r.encoded).count(),
            "completed":trace.iter().filter(|r|r.completed).count(),"seconds":started.elapsed().as_secs_f64(),
            "peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),
            "lastKernels":trace.iter().rev().take(20).map(|r|&r.kernel).collect::<Vec<_>>(),
            "error":actual.as_ref().err().map(|e|format!("{e:?}")),
        })
    );
    assert_eq!(actual, Ok(expected));
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
}

#[test]
fn shared_typed_output_scales_files_and_programs_without_commands_per_capture() {
    let programs = [b"alpha".as_slice(), b"beta".as_slice()]
        .into_iter()
        .map(|value| {
            Program::new(
                vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"text".to_vec(), b"role".to_vec(), value.to_vec()],
                vec![],
                vec![0],
                vec![],
            )
            .unwrap()
            .with_directives(vec![
                Directive::Set {
                    key: 1,
                    value: Some(2),
                },
                Directive::SetCapture {
                    capture: 0,
                    key: 1,
                    value: Some(2),
                },
                Directive::Downcase { capture: 0 },
            ])
            .unwrap()
        })
        .collect::<Vec<_>>();
    let mut first_launches = None;
    for count in [1, 128] {
        let files = vec![tape("# UPPER\n"); count];
        let anchors = vec![vec![vec![0]; programs.len()]; count];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            expected.iter().flatten().map(Vec::len).sum::<usize>(),
            2 * count
        );
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
        let elapsed = start.elapsed().as_secs_f64();
        let trace = scope.snapshot();
        assert_eq!(actual, Ok(expected));
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
        assert!(trace.len() <= 600);
        if let Some(first) = first_launches {
            assert!(trace.len() <= first + 16);
        } else {
            first_launches = Some(trace.len());
        }
        eprintln!(
            "TYPED_OUTPUT_EVIDENCE {}",
            serde_json::json!({
                "files": count, "programs": programs.len(), "results": count * 2,
                "captures": count * 4, "launches": trace.len(), "seconds": elapsed,
                "peakMetalBytes": trace.iter().map(|record| record.peak_application_buffer_bytes).max(),
            })
        );
    }
}

#[test]
fn shared_typed_output_rejects_inconsistent_structural_payloads() {
    use metal_backend::MappedBuffer;
    use metal_batch_abi::{FinalCaptureRecord, NONE, ResultRecord};
    use metal_candidate_batch::CandidateBatch;
    use metal_grid_plan::ResidentInput;
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"value".to_vec(), b"role".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::Set {
        key: 1,
        value: None,
    }])
    .unwrap();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&[program], &[tape("# VALUE\n")], &[vec![vec![1]]])
            .unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    let node = resident.input.packed.nodes[1];
    for defect in 0..11 {
        let _scope = metal_backend::launch::LaunchScope::enter();
        let mut row = ResultRecord {
            start: 1,
            capture_count: 1,
            ..ResultRecord::default()
        };
        let mut cap = FinalCaptureRecord {
            node: 1,
            start: node.source_start,
            end: node.source_end,
            text_start: NONE,
            retained: 1,
            ..FinalCaptureRecord::default()
        };
        let mut result_count = 1;
        let mut capture_count = 1;
        let mut errors = vec![None];
        match defect {
            0 => capture_count = 0,
            1 => row.capture_start = 1,
            2 => capture_count = 2,
            3 => cap.label = 1,
            4 => cap.node = 0,
            5 => cap.end -= 1,
            6 => row.start = 0,
            7 => cap.text_start = 0,
            8 => row.property_count = 1,
            9 => result_count = 0,
            10 => errors[0] = Some(Error::InvalidOperand(0)),
            _ => unreachable!(),
        }
        let batch = CandidateBatch {
            results: MappedBuffer::from_slice(&device, &[row]).unwrap(),
            captures: MappedBuffer::from_slice(&device, &[cap; 2]).unwrap(),
            result_count,
            capture_count,
        };
        let programs =
            metal_capture_predicates::PredicatePrograms::new(&device, &queue, &resident, &data)
                .unwrap();
        let actual = metal_typed_output::execute(
            &device,
            &queue,
            &resident,
            &data,
            vec![batch],
            errors,
            &programs,
        );
        assert_eq!(actual.err(), Some(Error::BackendFailure), "defect {defect}");
    }
}

#[test]
fn transformed_text_snapshots_and_utf8_offsets_preserve_directive_order() {
    let base = Program::new(
        vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"value".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [
        vec![
            Directive::Downcase { capture: 0 },
            Directive::Offset {
                capture: 0,
                offsets: [0, 4, 0, 0],
            },
        ],
        vec![
            Directive::Offset {
                capture: 0,
                offsets: [0, 2, 0, 0],
            },
            Directive::Downcase { capture: 0 },
            Directive::Offset {
                capture: 0,
                offsets: [0, 2, 0, 0],
            },
        ],
        vec![
            Directive::Offset {
                capture: 0,
                offsets: [0, 3, 0, 0],
            },
            Directive::Downcase { capture: 0 },
        ],
    ]
    .into_iter()
    .map(|directives| base.clone().with_directives(directives).unwrap())
    .collect::<Vec<_>>();
    let files = [tape("# ÉCOLE\n"), tape("# ÄBC\n")];
    let anchors = vec![vec![vec![1]; programs.len()]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected[0].iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1, 0]
    );
    assert_eq!(
        expected[0][0][0].captures[0].text.as_deref(),
        Some("# École".as_bytes())
    );
    assert_eq!(
        expected[0][1][0].captures[0].text.as_deref(),
        Some("École".as_bytes())
    );
    for file in &expected {
        for output in file.iter().flatten() {
            assert_eq!(output.captures[0], output.captures[1]);
        }
    }
    assert_eq!(
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
        Ok(expected)
    );
}

#[test]
fn typed_output_preserves_late_operand_failures_without_publishing_failed_evaluations() {
    let good = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"value".to_vec(), b"role".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::Set {
        key: 1,
        value: None,
    }])
    .unwrap();
    let bad = Program::new(
        vec![
            word(FORK, 0, 3),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"value".to_vec(), b"role".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![Directive::Set {
        key: 1,
        value: None,
    }])
    .unwrap();
    let programs = [good, bad];
    let files = [tape("FIRST\n"), tape("SECOND\n")];
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
        vec![vec![None, Some(Error::InvalidOperand(3))]; 2]
    );
    for limit in [0, 1, 2] {
        let mut outputs = expected.outputs.clone();
        for evaluation in outputs.iter_mut().flatten() {
            evaluation.truncate(limit);
        }
        let actual = metal_program_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit as u32; 2]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|batch| (batch.outputs, batch.errors)),
            Ok((outputs, expected.errors.clone()))
        );
    }
}

#[test]
fn resident_output_operators_and_line_addresses_reject_contradictory_data_before_dispatch() {
    use metal_grid_plan::{ExecutionPlan, ResidentInput};
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"value".to_vec(), b"role".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap()
    .with_directives(vec![
        Directive::Set {
            key: 1,
            value: None,
        },
        Directive::SetCapture {
            capture: 0,
            key: 1,
            value: None,
        },
        Directive::Offset {
            capture: 0,
            offsets: [0, 2, 0, 0],
        },
        Directive::Downcase { capture: 0 },
    ])
    .unwrap();
    for defect in 0..10 {
        let grid = metal_grid::MetalGridInput::build(
            std::slice::from_ref(&program),
            &[tape("# VALUE\n")],
            &[vec![vec![1]]],
        )
        .unwrap();
        let mut resident = ResidentInput::new(grid, None).unwrap();
        let plan = ExecutionPlan::new(&resident, std::slice::from_ref(&program)).unwrap();
        let batch = Arc::make_mut(&mut resident.input.packed);
        let start = batch.programs[0].output_start as usize;
        match defect {
            0 => batch.programs[0].output_start = u32::MAX,
            1 => batch.programs[0].output_count = u32::MAX,
            2 => batch.assertion_data[start] = 7,
            3 => batch.assertion_data[start + 1] = 2,
            4 => batch.assertion_data[start + 6 + 1] = 1,
            5 => batch.assertion_data[start + 12 + 1] = 1,
            6 => batch.files[0].line_start = u32::MAX,
            7 => batch.files[0].line_count = 1,
            8 => batch.line_starts[1] -= 1,
            9 => batch.source[0] = 0xff,
            _ => unreachable!(),
        }
        assert_eq!(
            plan.validate(&resident),
            Err(Error::InvalidAnchor),
            "defect {defect}"
        );
    }
}

#[test]
fn typed_properties_offsets_and_text_preserve_duplicate_captures_across_execution_routes() {
    let mut programs = vec![
        Program::new(
            vec![word(CAPTURE, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
            vec![b"value".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap(),
        Program::new(
            vec![
                word(CHILD, 0, 0),
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"value".to_vec()],
            vec![],
            vec![0],
            vec![],
        )
        .unwrap(),
        Program::compile_tree_sitter_query_pattern(
            "((comment)+ @value)",
            &SupportLang::Python.get_ts_language(),
        )
        .unwrap(),
    ];
    for program in &mut programs {
        let base = program.strings.len() as u16;
        program
            .strings
            .extend([b"role".to_vec(), b"accepted".to_vec(), b"present".to_vec()]);
        *program = program
            .clone()
            .with_directives(vec![
                Directive::Set {
                    key: base,
                    value: Some(base + 1),
                },
                Directive::Set {
                    key: base + 2,
                    value: None,
                },
                Directive::SetCapture {
                    capture: 0,
                    key: base,
                    value: None,
                },
                Directive::SetCapture {
                    capture: 0,
                    key: base,
                    value: Some(base + 1),
                },
                Directive::Offset {
                    capture: 0,
                    offsets: [0, 2, 0, 0],
                },
                Directive::Downcase { capture: 0 },
            ])
            .unwrap();
    }
    let files = [tape("# FIRST\n# SECOND\n"), tape("# OTHER\n# LAST\n")];
    let anchors = vec![vec![vec![0], vec![0], vec![1]]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    for evaluations in &expected {
        assert_eq!(
            evaluations.iter().map(Vec::len).collect::<Vec<_>>(),
            [1, 2, 1]
        );
        for output in evaluations.iter().flatten() {
            assert_eq!(output.properties.len(), 2);
            for capture in &output.captures {
                assert_eq!(capture.properties.len(), 2);
                assert!(
                    capture
                        .text
                        .as_ref()
                        .is_some_and(|text| !text.iter().any(u8::is_ascii_uppercase))
                );
            }
        }
        assert_eq!(evaluations[0][0].captures[0], evaluations[0][0].captures[1]);
        assert_eq!(evaluations[1][0].captures[0], evaluations[1][0].captures[1]);
    }
    assert_eq!(
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
        Ok(expected)
    );
}

#[test]
fn transformed_candidate_rejection_precedes_accepted_witness_limits() {
    let program = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"value".to_vec()],
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
    ])
    .unwrap();
    let files = [tape("#\n# KEEP\n"), tape("#\n# OTHER\n")];
    let anchors = vec![vec![vec![1, 2]]; files.len()];
    let expected = execute_batch(std::slice::from_ref(&program), &files, Some(&anchors)).unwrap();
    assert_eq!(
        expected
            .iter()
            .map(|file| file[0].len())
            .collect::<Vec<_>>(),
        [1, 1]
    );
    assert_eq!(
        expected[0][0][0].captures[0].text.as_deref(),
        Some(b"keep".as_slice())
    );
    assert_eq!(
        expected[1][0][0].captures[0].text.as_deref(),
        Some(b"other".as_slice())
    );
    for limit in [0, 1, 2] {
        let mut limited = expected.clone();
        for file in &mut limited {
            file[0].truncate(limit as usize);
        }
        let actual = metal_program_backend::execute_packed_with_result_limits(
            std::slice::from_ref(&program),
            &files,
            &anchors,
            Some(&[limit]),
        )
        .and_then(|execution| execution.into_outcome())
        .and_then(BatchOutcome::into_result);
        assert_eq!(actual, Ok(limited), "accepted witness limit {limit}");
    }
}

#[test]
fn prior_phase_optional_query_completes_structural_work_before_output_stages() {
    let program = Program::compile_tree_sitter_query_pattern(
        "(if_statement alternative: (_ (_) @conditional.inner)?) @conditional.outer",
        &SupportLang::Python.get_ts_language(),
    )
    .unwrap();
    assert!(program.value_programs.is_empty());
    assert!(program.capture_programs.is_empty());
    assert!(program.output_program.instructions.is_empty());
    let source = "if one:\n    pass\nelif two:\n    pass\nelif three:\n    pass\nelse:\n    first()\n    second()\n";
    let files = [tape(source)];
    let anchors = [vec![vec![1]]];
    let expected = execute_batch(std::slice::from_ref(&program), &files, Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), 5);
    for (output, text) in
        expected[0][0]
            .iter()
            .zip(["two", "pass", "three", "pass", "first()\n    second()"])
    {
        assert_eq!(output.captures.len(), 2);
        assert_eq!(&source[output.captures[1].span.clone()], text);
        assert_eq!(output.captures[0].span, 0..87);
    }
    let scope = metal_backend::launch::LaunchScope::enter();
    let start = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, &[program], &files, &anchors);
    let trace = scope.snapshot();
    eprintln!(
        "OPTIONAL_QUERY_AUDIT {}",
        serde_json::json!({
            "seconds": start.elapsed().as_secs_f64(),
            "attempts": trace.len(),
            "encoded": trace.iter().filter(|r| r.encoded).count(),
            "completed": trace.iter().filter(|r| r.completed).count(),
            "peakMetalBytes": trace.iter().map(|r| r.peak_application_buffer_bytes).max(),
            "kernels": trace.iter().map(|r| &r.kernel).collect::<Vec<_>>()
        })
    );
    assert_eq!(actual, Ok(expected));
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
}

#[test]
fn accepted_witness_limits_follow_shared_predicates_and_preserve_duplicate_captures() {
    let programs = [
        TextPredicate::Equal(0),
        TextPredicate::Prefix(0),
        TextPredicate::Regex {
            source: 0,
            compiled: regex::Regex::new("^# KEEP$").unwrap(),
        },
        TextPredicate::LuaPattern {
            source: 0,
            compiled: LuaPattern::parse(b"^# KEEP$").unwrap(),
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, predicate)| {
        Program::new(
            vec![
                word(TEXT, 0, 0),
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![
                if index < 2 {
                    b"# KEEP".to_vec()
                } else {
                    b"^# KEEP$".to_vec()
                },
                b"comment".to_vec(),
                b"role".to_vec(),
                b"accepted".to_vec(),
            ],
            vec![predicate],
            vec![1],
            vec![],
        )
        .unwrap()
        .with_directives(vec![
            Directive::Set {
                key: 2,
                value: Some(3),
            },
            Directive::Offset {
                capture: 0,
                offsets: [0, 2, 0, 0],
            },
            Directive::Downcase { capture: 0 },
        ])
        .unwrap()
    })
    .collect::<Vec<_>>();
    let files = [
        tape("# DROP\n# KEEP\n# DROP\n# KEEP\n"),
        tape("# KEEP\n# DROP\n"),
    ];
    let anchors = files
        .iter()
        .map(|file| vec![(1..file.transducer_len() as u32).collect::<Vec<_>>(); programs.len()])
        .collect::<Vec<_>>();
    let limits = [1, 2, 0, u32::MAX];
    let mut expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    for (file_index, evaluations) in expected.iter_mut().enumerate() {
        for (program_index, outputs) in evaluations.iter_mut().enumerate() {
            assert_eq!(outputs.len(), if file_index == 0 { 2 } else { 1 });
            for output in outputs.iter() {
                assert_eq!(output.captures.len(), 2);
                assert_eq!(output.captures[0], output.captures[1]);
                assert_eq!(output.captures[0].text.as_deref(), Some(b"keep".as_slice()));
            }
            outputs.truncate(limits[program_index] as usize);
        }
    }
    let actual = metal_program_backend::execute_packed_with_result_limits(
        &programs,
        &files,
        &anchors,
        Some(&limits),
    )
    .and_then(|execution| execution.into_outcome())
    .and_then(BatchOutcome::into_result);
    assert_eq!(actual, Ok(expected));
}

#[test]
fn late_failures_invalidate_only_their_evaluation_even_after_the_witness_limit() {
    let good = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let bad = Program::new(
        vec![
            word(FORK, 0, 3),
            word(CAPTURE, 0, 0),
            word(EMIT, 0, 0),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [good, bad];
    let files = [tape("first\n"), tape("second\n")];
    let anchors = vec![vec![vec![0]; 2]; 2];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(
        expected.errors,
        vec![vec![None, Some(Error::InvalidOperand(3))]; 2]
    );
    for limit in [0, 1] {
        let mut outputs = expected.outputs.clone();
        for evaluation in outputs.iter_mut().flatten() {
            evaluation.truncate(limit);
        }
        let actual = metal_program_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit as u32; 2]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|batch| (batch.outputs, batch.errors)),
            Ok((outputs, expected.errors.clone()))
        );
    }
}

#[test]
fn operand_failures_follow_logical_paths_and_source_root_order_instead_of_arrival_or_pc_order() {
    let good = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let ordinary = Program::new(
        vec![
            word(FORK, 0, 4),
            word(CHILD, NAMED, 0),
            word(GOTO, 0, 6),
            word(STOP, 0, 0),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
            word(CAPTURE, 0, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let query = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(FORK, 0, 5),
            word(CHILD, NAMED, 0),
            word(GOTO, 0, 7),
            word(STOP, 0, 0),
            word(REPEAT_END, 0, 2),
            word(EMIT, 0, 0),
            word(CAPTURE, 0, 0),
            word(RESTORE_CURSOR, 0, 0),
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
    let programs = [good, ordinary, query];
    let files = [tape("first(one)\n"), tape("second(two)\n")];
    let anchors = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let last = file.transducer_len() as u32 - 1;
            let roots = if index == 0 {
                vec![0, last]
            } else {
                vec![last]
            };
            vec![vec![0, last], roots.clone(), roots]
        })
        .collect::<Vec<_>>();
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(
        expected.errors,
        vec![
            vec![
                None,
                Some(Error::InvalidOperand(7)),
                Some(Error::InvalidOperand(8))
            ],
            vec![
                None,
                Some(Error::InvalidOperand(4)),
                Some(Error::InvalidOperand(5))
            ],
        ]
    );
    for limit in [0, 1, 2] {
        let mut outputs = expected.outputs.clone();
        for evaluation in outputs.iter_mut().flatten() {
            evaluation.truncate(limit);
        }
        let actual = metal_program_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit as u32; 3]),
        )
        .and_then(|execution| execution.into_outcome());
        assert_eq!(
            actual.map(|batch| (batch.outputs, batch.errors)),
            Ok((outputs, expected.errors.clone())),
            "limit={limit}"
        );
    }
}
