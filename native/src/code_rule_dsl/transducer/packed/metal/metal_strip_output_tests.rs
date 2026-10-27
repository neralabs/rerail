//! Strip output truth is independent of continuation layout and GPU scheduling.
use super::*;

fn strip_program(pattern: &str) -> Program {
    Program::new(
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
    .unwrap()
}

fn manual_strip_cases() -> [(&'static str, &'static str, &'static str); 16] {
    [
        ("^a", "aaa", "aa"),
        ("$", "é", "é"),
        (r"\b", "ab é", "ab é"),
        ("^a|b", "aaab", "aa"),
        ("b$|^a", "aaab", "aa"),
        ("a|aa", "aaab", "b"),
        ("aa|a", "aaab", "b"),
        ("a+?", "aaab", "b"),
        ("", "é😀", "é😀"),
        ("x*", "aé", "aé"),
        ("a*$", "éaaa", "é"),
        ("aba|a", "ababaa", "b"),
        ("a|aba", "ababaa", "bb"),
        ("é", "AéB", "AB"),
        ("n", "BaNaNa", "BaNaNa"),
        ("a", "AaaB", "AB"),
    ]
}

fn check_strip_truth(backend: ExecutionBackend) {
    for (pattern, source, text) in manual_strip_cases() {
        for query in [false, true] {
            let base = strip_program(pattern);
            let programs = [if query {
                base.with_tree_sitter_query_semantics().unwrap()
            } else {
                base
            }];
            let files = [tape(source)];
            let anchors = [vec![vec![0]]];
            let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
            assert_eq!(
                expected[0][0].len(),
                1,
                "pattern={pattern:?}, query={query}"
            );
            let output = &expected[0][0][0];
            assert_eq!(output.task_ordinal, 0);
            assert_eq!(output.local_ordinal, 0);
            assert_eq!(output.captures.len(), 2);
            assert_eq!(output.captures[0], output.captures[1]);
            assert_eq!(output.captures[0].node, 0);
            assert_eq!(output.captures[0].span, 0..source.len());
            assert_eq!(output.captures[0].text.as_deref(), Some(text.as_bytes()));
            assert_eq!(
                execute_with_backend(backend, &programs, &files, &anchors),
                Ok(expected),
                "pattern={pattern:?}, source={source:?}, query={query}"
            );
        }
    }
}

fn check_strip_snapshots(backend: ExecutionBackend) {
    for (source, patterns, expected_text) in [
        ("AAA", [None, Some("^a")], "aa"),
        ("aaab", [Some("a"), Some("b")], ""),
        ("éAAA", [None, Some("a")], "é"),
        ("baaa", [Some("b"), Some("$")], "aaa"),
    ] {
        let mut base = strip_program("");
        let directives = patterns
            .into_iter()
            .map(|pattern| {
                if let Some(pattern) = pattern {
                    let index = base.strings.len() as u16;
                    base.strings.push(pattern.as_bytes().to_vec());
                    Directive::Strip {
                        capture: 0,
                        source: index,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }
                } else {
                    Directive::Downcase { capture: 0 }
                }
            })
            .collect::<Vec<_>>();
        let programs = [base.with_directives(directives).unwrap()];
        let files = [tape(source)];
        let anchors = [vec![vec![0]]];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(expected[0][0].len(), 1);
        let captures = &expected[0][0][0].captures;
        assert_eq!(captures.len(), 2);
        assert_eq!(captures[0], captures[1]);
        assert_eq!(captures[0].text.as_deref(), Some(expected_text.as_bytes()));
        assert_eq!(
            execute_with_backend(backend, &programs, &files, &anchors),
            Ok(expected),
            "source={source:?}, patterns={patterns:?}"
        );
    }
}

#[test]
fn native_strip_preserves_unmatched_prefixes_full_context_and_empty_utf8_matches() {
    check_strip_truth(ExecutionBackend::CpuJit);
}
#[test]
fn native_strip_consumes_each_prior_transformed_capture_snapshot() {
    check_strip_snapshots(ExecutionBackend::CpuJit);
}
#[test]
fn shared_strip_preserves_unmatched_prefixes_full_context_and_empty_utf8_matches() {
    check_strip_truth(ExecutionBackend::Metal);
}
#[test]
fn shared_strip_consumes_each_prior_transformed_capture_snapshot() {
    check_strip_snapshots(ExecutionBackend::Metal);
}

#[test]
fn complete_strip_output_rejects_one_candidate_that_exhausts_its_shared_capture_budget() {
    let pattern = "^# a+$";
    let source = format!("# {}", "a".repeat(80_000));
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let (first, first_cost) = nfa.test_find_work_count_from(source.as_bytes(), 0);
    assert_eq!(first, Some([0, source.len() as u32]));
    let (last, last_cost) = nfa.test_find_work_count_from(source.as_bytes(), source.len());
    assert_eq!(last, None);
    let one_capture = first_cost + last_cost;
    assert!(one_capture * 2 < MAX_STEPS_PER_PATH);
    assert!(one_capture * 4 > MAX_STEPS_PER_PATH);
    for query in [false, true] {
        let base = Program::new(
            vec![
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 0),
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
        let files = [tape(&source)];
        let anchors = [vec![vec![0]]];
        assert_eq!(
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors),
            Err(Error::WorkLimit)
        );
        assert_eq!(
            execute_batch(&programs, &files, Some(&anchors)),
            Err(Error::WorkLimit),
            "query={query}"
        );
        assert_eq!(
            execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors),
            Err(Error::WorkLimit)
        );
    }
}

fn strip_work(pattern: &str, source: &str) -> usize {
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let mut search = 0;
    let mut cost = 0;
    loop {
        let (found, used) = nfa.test_find_work_count_from(source.as_bytes(), search);
        cost += used;
        let Some([first, last]) = found else { break };
        search = last as usize;
        if first == last {
            if search == source.len() {
                break;
            }
            search += source[search..].chars().next().unwrap().len_utf8();
        }
    }
    cost
}

#[test]
fn shared_strip_continuations_preserve_exact_work_and_deletions_at_tiny_unit_boundaries() {
    use metal_backend::MappedBuffer;
    use metal_grid_plan::ResidentInput;
    use metal_regex_frontier::{
        RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork, StripSnapshot,
    };
    let long = format!("{}B", "a".repeat(512));
    let mut cases = manual_strip_cases().to_vec();
    cases.extend([
        ("a.*z|a", "aaaaab", "b"),
        ("a+", long.as_str(), "B"),
        ("", "", ""),
    ]);
    for (pattern, source, expected) in cases {
        assert_eq!(
            regex::Regex::new(pattern).unwrap().replace_all(source, ""),
            expected
        );
        let cost = strip_work(pattern, source);
        let programs = [strip_program(pattern)];
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(&programs, &[tape(source)], &[vec![vec![0]]])
                .unwrap(),
            None,
        )
        .unwrap();
        let value = resident.input.packed.programs[0].value_start;
        let width = resident.input.packed.values[value as usize]
            .state_count
            .next_power_of_two();
        for unit in [Some(8), None] {
            for limit in [cost * 2 - 1, cost * 2, cost * 2 + 1] {
                let guard = metal_backend::metal_execution_guard().unwrap();
                let device = metal::Device::system_default().unwrap();
                let queue = device.new_command_queue();
                let data = metal_register_execution::upload(&device, &resident).unwrap();
                let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
                let source_bytes = source.as_bytes().repeat(2);
                let bytes = MappedBuffer::from_slice(&device, &source_bytes).unwrap();
                let mask =
                    MappedBuffer::from_slice(&device, &vec![1u32; source_bytes.len()]).unwrap();
                let requests = (0..2)
                    .map(|index| RegexRequest {
                        value,
                        source_start: index * source.len() as u32,
                        source_length: source.len() as u32,
                        budget: limit as u32,
                        width,
                        scratch: index * width,
                        mode: 2,
                        match_start: u32::MAX,
                        match_end: u32::MAX,
                        ..RegexRequest::default()
                    })
                    .collect::<Vec<_>>();
                let batch = RegexBatch {
                    scratch_plan: ScratchPlan::Device,
                    requests: MappedBuffer::from_slice(&device, &requests).unwrap(),
                    ids: MappedBuffer::from_slice(&device, &[0u32, 1]).unwrap(),
                    count: 2,
                    scratch_slots: width as usize * 2,
                };
                let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
                let work = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
                let failures = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
                let scope = metal_backend::launch::LaunchScope::enter();
                let input = RegexInput {
                    data: &data,
                    status: &status,
                    budget_owners: 1,
                };
                let shared = SharedWork {
                    values: &work,
                    failures: &failures,
                    evaluations: 1,
                    abort_evaluation: true,
                };
                let snapshot = StripSnapshot {
                    bytes: &bytes,
                    length: source_bytes.len(),
                    mask: &mask,
                    work_limit: limit as u32,
                };
                let result = if let Some(unit) = unit {
                    code.execute_strip_in_units(
                        &device,
                        &queue,
                        input,
                        batch,
                        shared,
                        (snapshot, unit),
                    )
                } else {
                    code.execute_strip(&device, &queue, input, batch, shared, snapshot)
                };
                let trace = scope.snapshot();
                drop(scope);
                drop(guard);
                let finished = result.expect("valid shared Strip requests");
                let actual = metal_register_execution::read::<RegexRequest>(&finished, 2).unwrap();
                let used = metal_register_execution::read::<u32>(&work, 1).unwrap()[0];
                let error = metal_register_execution::read::<u32>(&failures, 1).unwrap()[0];
                assert!(trace.len() <= 600);
                assert!(
                    trace
                        .iter()
                        .all(|r| r.encoded && r.submitted && r.completed)
                );
                if limit < cost * 2 {
                    assert_eq!(error, 2, "pattern={pattern:?}, unit={unit:?}");
                    assert_eq!(used, limit as u32 + 1);
                    assert!(actual.iter().any(|r| r.status == 3));
                    assert!(actual.iter().all(|r| matches!(r.status, 1 | 3 | 5)));
                } else {
                    assert_eq!(error, 0);
                    assert_eq!(used, cost as u32 * 2, "pattern={pattern:?}, unit={unit:?}");
                    assert!(
                        actual
                            .iter()
                            .all(|r| r.status == 1 && r.used == cost as u32)
                    );
                    let mask =
                        metal_register_execution::read::<u32>(&mask, source_bytes.len()).unwrap();
                    assert!(mask.iter().all(|&keep| keep <= 1));
                    for capture in 0..2 {
                        let start = capture * source.len();
                        let text = source_bytes[start..start + source.len()]
                            .iter()
                            .zip(&mask[start..start + source.len()])
                            .filter_map(|(&byte, &keep)| (keep == 1).then_some(byte))
                            .collect::<Vec<_>>();
                        assert_eq!(
                            text,
                            expected.as_bytes(),
                            "pattern={pattern:?}, unit={unit:?}"
                        );
                    }
                }
                eprintln!(
                    "STRIP_UNITS_EVIDENCE {}",
                    serde_json::json!({
                        "pattern":pattern,"bytes":source.len(),"unit":unit,"budget":limit,"used":used,"failure":error,
                        "launches":trace.len(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),
                    })
                );
            }
        }
    }
}

#[test]
fn shared_strip_rejects_invalid_snapshot_arenas_before_any_semantic_mutation() {
    use metal_backend::MappedBuffer;
    use metal_grid_plan::ResidentInput;
    use metal_regex_frontier::{
        RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork, StripSnapshot,
    };
    #[derive(Clone, Copy, Debug)]
    enum Defect {
        Control,
        MaskWeight,
        Overlap,
        Mode,
        Progress,
        SplitScalar,
        Budget,
        OwnerLimit,
        MaskExtent,
        SourceExtent,
        SourceAlias,
        WorkAlias,
        ScratchPrefix,
    }
    let source = "éaa";
    let programs = [strip_program("a")];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &[tape(source)], &[vec![vec![0]]]).unwrap(),
        None,
    )
    .unwrap();
    let value = resident.input.packed.programs[0].value_start;
    let width = resident.input.packed.values[value as usize]
        .state_count
        .next_power_of_two();
    for defect in [
        Defect::Control,
        Defect::MaskWeight,
        Defect::Overlap,
        Defect::Mode,
        Defect::Progress,
        Defect::SplitScalar,
        Defect::Budget,
        Defect::OwnerLimit,
        Defect::MaskExtent,
        Defect::SourceExtent,
        Defect::SourceAlias,
        Defect::WorkAlias,
        Defect::ScratchPrefix,
    ] {
        let guard = metal_backend::metal_execution_guard().unwrap();
        let device = metal::Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let data = metal_register_execution::upload(&device, &resident).unwrap();
        let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
        let source_bytes = source.as_bytes().repeat(2);
        let bytes = MappedBuffer::from_slice(&device, &source_bytes).unwrap();
        let mut initial_mask = vec![1u32; source_bytes.len()];
        if matches!(defect, Defect::MaskWeight) {
            initial_mask[0] = 2;
        }
        if matches!(defect, Defect::MaskExtent) {
            initial_mask.pop();
        }
        let mask = MappedBuffer::from_slice(&device, &initial_mask).unwrap();
        let mut rows = (0..2)
            .map(|index| RegexRequest {
                value,
                source_start: index * source.len() as u32,
                source_length: source.len() as u32,
                budget: MAX_STEPS_PER_PATH as u32,
                width,
                scratch: index * width,
                mode: 2,
                match_start: u32::MAX,
                match_end: u32::MAX,
                ..RegexRequest::default()
            })
            .collect::<Vec<_>>();
        let mut scratch = width as usize * 2;
        match defect {
            Defect::Overlap => rows[1].source_start = 0,
            Defect::Mode => rows[0].mode = 1,
            Defect::Progress => rows[0].progress = 1,
            Defect::SplitScalar => rows[0].source_length = 1,
            Defect::Budget => rows[0].budget += 1,
            Defect::ScratchPrefix => {
                for row in &mut rows {
                    row.scratch += 1;
                }
                scratch += 1;
            }
            _ => {}
        }
        let request_buffer = MappedBuffer::from_slice(&device, &rows).unwrap();
        let request_owner = request_buffer.buffer.clone();
        let request_words = rows.len() * std::mem::size_of::<RegexRequest>() / 4;
        let request_before =
            metal_register_execution::read::<u32>(&request_buffer, request_words).unwrap();
        let work_before = vec![u32::from(matches!(defect, Defect::WorkAlias)); source_bytes.len()];
        let work = MappedBuffer::from_slice(&device, &work_before).unwrap();
        let failures = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let scope = metal_backend::launch::LaunchScope::enter();
        let result = code.execute_strip(
            &device,
            &queue,
            RegexInput {
                data: &data,
                status: &status,
                budget_owners: 1,
            },
            RegexBatch {
                scratch_plan: ScratchPlan::Device,
                requests: request_buffer,
                ids: MappedBuffer::from_slice(&device, &[0u32, 1]).unwrap(),
                count: 2,
                scratch_slots: scratch,
            },
            SharedWork {
                values: &work,
                failures: &failures,
                evaluations: 1,
                abort_evaluation: true,
            },
            StripSnapshot {
                bytes: if matches!(defect, Defect::SourceAlias) {
                    &code.code
                } else {
                    &bytes
                },
                length: source_bytes.len() + usize::from(matches!(defect, Defect::SourceExtent)),
                mask: if matches!(defect, Defect::WorkAlias) {
                    &work
                } else {
                    &mask
                },
                work_limit: MAX_STEPS_PER_PATH as u32
                    + u32::from(matches!(defect, Defect::OwnerLimit)),
            },
        );
        let trace = scope.snapshot();
        drop(scope);
        drop(guard);
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        if matches!(defect, Defect::Control) {
            let finished = result.expect("valid snapshot control");
            let rows = metal_register_execution::read::<RegexRequest>(&finished, 2).unwrap();
            let cost = strip_work("a", source) as u32;
            assert!(rows.iter().all(|r| r.status == 1 && r.used == cost));
            assert_eq!(
                metal_register_execution::read::<u32>(&work, 1).unwrap(),
                vec![cost * 2]
            );
            let mask = metal_register_execution::read::<u32>(&mask, source_bytes.len()).unwrap();
            let text = source_bytes
                .iter()
                .zip(mask)
                .filter_map(|(&byte, keep)| (keep == 1).then_some(byte))
                .collect::<Vec<_>>();
            assert_eq!(text, "éé".as_bytes());
        } else {
            assert!(
                matches!(result, Err(Error::BackendFailure)),
                "defect={defect:?}"
            );
            assert_eq!(
                metal_register_execution::read::<u32>(&work, work_before.len()).unwrap(),
                work_before,
                "defect={defect:?}"
            );
            assert_eq!(
                metal_register_execution::read::<u32>(&failures, 1).unwrap(),
                vec![0]
            );
            assert_eq!(
                metal_register_execution::read::<u32>(&mask, initial_mask.len()).unwrap(),
                initial_mask
            );
            let after = unsafe {
                std::slice::from_raw_parts(request_owner.contents().cast::<u32>(), request_words)
            };
            assert_eq!(after, request_before.as_slice(), "defect={defect:?}");
        }
        assert_eq!(
            metal_register_execution::read::<u8>(&bytes, source_bytes.len()).unwrap(),
            source_bytes
        );
    }
}

#[test]
fn shared_strip_batches_programs_files_properties_and_predicates_with_one_immutable_code_owner() {
    let programs = ["^# ", "a|aa", "aa|a", r"\b", "é", "AAA"]
        .into_iter()
        .map(|pattern| {
            Program::new_with_capture_predicates(
                vec![
                    word(CAPTURE, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(ASSERT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![
                    b"text".to_vec(),
                    pattern.as_bytes().to_vec(),
                    b"(?s).*".to_vec(),
                ],
                vec![TextPredicate::Regex {
                    source: 2,
                    compiled: regex::Regex::new("(?s).*").unwrap(),
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
            .with_tree_sitter_query_semantics()
            .unwrap()
            .with_directives(vec![
                Directive::Set {
                    key: 0,
                    value: Some(1),
                },
                Directive::Strip {
                    capture: 0,
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                },
                Directive::Downcase { capture: 0 },
                Directive::SetCapture {
                    capture: 0,
                    key: 0,
                    value: Some(1),
                },
                Directive::Strip {
                    capture: 0,
                    source: 1,
                    compiled: regex::Regex::new(pattern).unwrap(),
                },
            ])
            .unwrap()
        })
        .collect::<Vec<_>>();
    let mut stages = None;
    for files_count in [1, 128] {
        let files = vec![tape("# AAA banana Éé aaab\n"); files_count];
        let anchors = vec![vec![vec![0]; programs.len()]; files_count];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        for file in &expected {
            assert_eq!(file.len(), programs.len());
            for rows in file {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].properties, vec![(0, Some(1))]);
                assert_eq!(rows[0].captures.len(), 2);
                assert_eq!(rows[0].captures[0], rows[0].captures[1]);
                assert_eq!(rows[0].captures[0].properties, vec![(0, Some(1))]);
                assert!(rows[0].captures[0].text.is_some());
            }
        }
        let cpu_start = std::time::Instant::now();
        assert_eq!(
            execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors),
            Ok(expected.clone())
        );
        let cpu_seconds = cpu_start.elapsed().as_secs_f64();
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
        let seconds = start.elapsed().as_secs_f64();
        let trace = scope.snapshot();
        drop(scope);
        assert_eq!(actual, Ok(expected));
        assert!(trace.len() <= 600);
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        let code = trace
            .iter()
            .filter(|r| r.kernel == "regex_frontier_advance")
            .map(|r| r.buffers[1].0)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            code.len(),
            1,
            "query predicates and all Strip directives share code"
        );
        let mut semantic = std::collections::BTreeMap::new();
        for record in &trace {
            if record.kernel.starts_with("strip_") || record.kernel == "regex_frontier_advance" {
                *semantic.entry(record.kernel.clone()).or_insert(0u32) += 1;
            }
        }
        if let Some(first) = &stages {
            assert_eq!(&semantic, first);
        } else {
            stages = Some(semantic.clone());
        }
        eprintln!(
            "STRIP_BATCH_EVIDENCE {}",
            serde_json::json!({
                "files":files_count,"programs":programs.len(),"results":files_count*programs.len(),"captures":files_count*programs.len()*2,
                "launches":trace.len(),"seconds":seconds,"cpuSeconds":cpu_seconds,"codeOwners":code.len(),"semanticStages":semantic,
                "peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),
            })
        );
    }
}
