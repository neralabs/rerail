//! Public regex behavior and shared work budgets, independent of NFA scheduling.
use super::*;

fn regex_program(pattern: &str, negated: bool) -> Program {
    Program::new_with_capture_predicates(
        vec![
            word(CAPTURE, 0, 0),
            word(CAPTURE, 0, 0),
            word(ASSERT, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![pattern.as_bytes().to_vec(), b"value".to_vec()],
        vec![TextPredicate::Regex {
            source: 0,
            compiled: regex::Regex::new(pattern).unwrap(),
        }],
        vec![CapturePredicate::Text {
            capture: 0,
            predicate: 0,
            negated,
            any: false,
        }],
        vec![1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
}

#[test]
fn shared_regex_queries_preserve_unicode_looks_priority_and_duplicate_captures() {
    let patterns = [
        "",
        "^$",
        "a|ab",
        "a+",
        "a+?",
        "(?s:.)*b",
        "^# [a-z]+$",
        r"\bαβ\b",
        r"\Bα",
        r"\b{start}α",
        r"β\b{end}",
        r"\b{start-half}α",
        r"β\b{end-half}",
        r"(?-u:\b)α",
        r"(?-u:\B)α",
        r"(?-u:\b{start})a",
        r"a(?-u:\b{end})",
        r"(?-u:\b{start-half})a",
        r"a(?-u:\b{end-half})",
        "(?m)^x$",
        "(?mR)^x$",
        "(?s:.)",
    ];
    let sources = [
        "",
        "abc",
        "αβ",
        "café 42",
        "éx\n",
        "\r\nx\r\n",
        "x\n",
        "# aaaa",
        "abbb",
        "# KEEP\n# DROP",
        "💥x\né",
    ];
    let programs = patterns
        .iter()
        .flat_map(|pattern| [regex_program(pattern, false), regex_program(pattern, true)])
        .collect::<Vec<_>>();
    let files = sources
        .iter()
        .map(|source| tape(source))
        .collect::<Vec<_>>();
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    for (file, source) in sources.iter().enumerate() {
        for (pattern_index, pattern) in patterns.iter().enumerate() {
            let span = files[file].transducer_span(0);
            let matches = regex::Regex::new(pattern)
                .unwrap()
                .is_match(&source[span.clone()]);
            for (variant, negated) in [false, true].into_iter().enumerate() {
                let outputs = &expected[file][pattern_index * 2 + variant];
                assert_eq!(
                    outputs.len(),
                    usize::from(matches != negated),
                    "source={source:?}, pattern={pattern}, negated={negated}"
                );
                for output in outputs {
                    assert_eq!(output.captures.len(), 2);
                    assert_eq!(output.captures[0], output.captures[1]);
                    assert_eq!(output.captures[0].span, span);
                }
            }
        }
    }
    let scope = metal_backend::launch::LaunchScope::enter();
    let start = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
    assert_eq!(actual, Ok(expected));
    let trace = scope.snapshot();
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|record| record.encoded && record.submitted && record.completed)
    );
    eprintln!(
        "REGEX_FRONTIER_EVIDENCE {}",
        serde_json::json!({"programs":programs.len(),"files":files.len(),"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|record|record.peak_application_buffer_bytes).max()})
    );
}

#[test]
fn captured_regex_work_is_shared_after_false_values_and_invalidates_only_its_evaluation() {
    let pattern = "^# (?:a|aa)*b$";
    let nfa = value::ByteNfa::compile(pattern).unwrap();
    let mut fixture = None;
    for length in [20_000, 40_000, 60_000, 80_000] {
        let source = format!("# {}", "a".repeat(length));
        let (outcome, cost) = nfa.test_work_count(source.as_bytes());
        if outcome == Ok(false) && cost > MAX_STEPS_PER_PATH / 2 && cost <= MAX_STEPS_PER_PATH {
            fixture = Some((source, cost));
            break;
        }
    }
    let (source, cost) =
        fixture.expect("one value must fit while both exceed the shared predicate budget");
    assert!(cost * 2 > MAX_STEPS_PER_PATH);
    let good = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"value".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let programs = [good, regex_program(pattern, false)];
    let files = [tape("# short"), tape(&format!("# ab\n{source}"))];
    let anchors = vec![vec![vec![0], vec![1]], vec![vec![0], vec![1, 2]]];
    let expected = execute_with_backend_outcomes(
        ExecutionBackend::CpuInterpreter,
        &programs,
        &files,
        &anchors,
    )
    .unwrap();
    assert_eq!(
        expected.errors,
        vec![vec![None, None], vec![None, Some(Error::WorkLimit)]]
    );
    assert_eq!(
        expected
            .outputs
            .iter()
            .map(|file| file.iter().map(Vec::len).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [vec![1, 0], vec![1, 0]]
    );
    for limit in [0, 1] {
        let mut outputs = expected.outputs.clone();
        for file in &mut outputs {
            for evaluation in file {
                evaluation.truncate(limit as usize);
            }
        }
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
            "accepted limit={limit}, first-value work={cost}"
        );
    }
}

#[test]
fn cooperative_regex_frontiers_preserve_exact_budget_edges_and_saved_utf8_boundaries() {
    use metal_backend::MappedBuffer;
    use metal_grid_plan::ResidentInput;
    use metal_regex_frontier::{
        RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork,
    };
    let mut cases = vec![
        ("^a+$".to_owned(), "aaaa".to_owned()),
        ("^a+b$".to_owned(), "aaaa".to_owned()),
        ("^".to_owned(), "é".to_owned()),
        (r"\bαβ\b".to_owned(), "αβ".to_owned()),
        ("(?mR)^x$".to_owned(), "\r\nx\r\n".to_owned()),
        ("(?:ab|a)*b".to_owned(), "aaaaaab".to_owned()),
    ];
    let unit_bytes = 8u32;
    for length in [
        unit_bytes as usize - 2,
        unit_bytes as usize - 1,
        unit_bytes as usize,
    ] {
        cases.push(("é$".to_owned(), format!("{}é", "a".repeat(length))));
    }
    let programs = cases
        .iter()
        .map(|(pattern, _)| regex_program(pattern, false))
        .collect::<Vec<_>>();
    let files = cases.iter().map(|(_, text)| tape(text)).collect::<Vec<_>>();
    let input = ResidentInput::new(
        metal_grid::MetalGridInput::build(
            &programs,
            &files,
            &vec![vec![vec![0]; programs.len()]; files.len()],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &input).unwrap();
    let code = RegexProgram::new(&device, &input.input.packed).unwrap();
    let mut requests = Vec::new();
    let mut expected = Vec::new();
    let mut scratch = 0u32;
    for (index, (pattern, text)) in cases.iter().enumerate() {
        let nfa = value::ByteNfa::compile(pattern).unwrap();
        let (outcome, cost) = nfa.test_work_count(text.as_bytes());
        assert_eq!(
            outcome,
            Ok(regex::Regex::new(pattern).unwrap().is_match(text))
        );
        assert!(cost > 0 && cost + 1 < MAX_STEPS_PER_PATH);
        let pg = input.input.packed.programs[index];
        let value = input.input.packed.values[pg.value_start as usize];
        let file = input.input.packed.files[index];
        for budget in [cost - 1, cost, cost + 1] {
            let owner = requests.len() as u32;
            let width = value.state_count.next_power_of_two();
            requests.push(RegexRequest {
                value: pg.value_start,
                source_start: file.source_start,
                source_length: file.source_length,
                budget: budget as u32,
                scratch,
                width,
                owner,
                evaluation: owner,
                match_start: u32::MAX,
                match_end: u32::MAX,
                ..RegexRequest::default()
            });
            scratch += width;
            expected.push(nfa.test_match_with_budget(text.as_bytes(), budget));
        }
    }
    let count = requests.len();
    let requests = MappedBuffer::from_slice(&device, &requests).unwrap();
    let ids = MappedBuffer::from_slice(&device, &(0..count as u32).collect::<Vec<_>>()).unwrap();
    let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
    let work = MappedBuffer::zeroed::<u32>(&device, count).unwrap();
    let failures = MappedBuffer::zeroed::<u32>(&device, count).unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let requests = code
        .execute_with_unit_size(
            &device,
            &queue,
            RegexInput {
                data: &data,
                status: &status,
                budget_owners: count,
            },
            RegexBatch {
                scratch_plan: ScratchPlan::Device,
                requests,
                ids,
                count,
                scratch_slots: scratch as usize,
            },
            SharedWork {
                values: &work,
                failures: &failures,
                evaluations: count,
                abort_evaluation: true,
            },
            unit_bytes,
        )
        .unwrap();
    let actual = metal_register_execution::read::<RegexRequest>(&requests, count).unwrap();
    let work = metal_register_execution::read::<u32>(&work, count).unwrap();
    let failures = metal_register_execution::read::<u32>(&failures, count).unwrap();
    for (index, ((expected, used), actual)) in expected.into_iter().zip(actual).enumerate() {
        let outcome = match actual.status {
            1 => Ok(false),
            2 => Ok(true),
            3 => Err(Error::WorkLimit),
            status => panic!("unfinished or inconsistent request {index}: {status}"),
        };
        assert_eq!(outcome, expected, "request {index}");
        assert_eq!(actual.used as usize, used, "request {index}");
        assert_eq!(work[index] as usize, used, "request {index}");
        assert_eq!(
            failures[index],
            if outcome == Err(Error::WorkLimit) {
                2
            } else {
                0
            },
            "request {index}"
        );
    }
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .filter(|record| record.kernel == "regex_frontier_advance")
            .count()
            >= 2
    );
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|record| record.encoded && record.submitted && record.completed)
    );
}

#[test]
fn shared_regex_requests_reject_invalid_spans_budgets_and_continuations_without_device_faults() {
    use metal_backend::MappedBuffer;
    use metal_grid_plan::ResidentInput;
    use metal_regex_frontier::{
        RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork,
    };
    let programs = [regex_program("é$", false)];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &[tape("é")], &[vec![vec![0]]]).unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
    let value = resident.input.packed.values[0];
    let width = value.state_count.next_power_of_two();
    for defect in 0..17 {
        let mut request = RegexRequest {
            source_length: 2,
            budget: MAX_STEPS_PER_PATH as u32,
            width,
            match_start: u32::MAX,
            match_end: u32::MAX,
            ..RegexRequest::default()
        };
        match defect {
            0 => request.owner = 1,
            1 => request.evaluation = 1,
            2 => request.budget += 1,
            3 => request.scratch = 1,
            4 => request.width = 0,
            5 => request.source_start = u32::MAX,
            6 => request.source_length = u32::MAX,
            7 => request.phase = 1,
            8 => request.mode = 2,
            9 => request.status = 2,
            10 => {
                request.source_start = 1;
                request.source_length = 1;
            }
            11 => request.source_length = 1,
            12 => {
                request.phase = 1;
                request.at = 1;
                request.used = 1;
            }
            13 => request.used = 1,
            14 => request.match_start = 1,
            15 => request.match_end = 1,
            16 => request.mode = 3,
            _ => unreachable!(),
        }
        let requests = MappedBuffer::from_slice(&device, &[request]).unwrap();
        let ids = MappedBuffer::from_slice(&device, &[0u32]).unwrap();
        let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let work = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let failures = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let scope = metal_backend::launch::LaunchScope::enter();
        let result = code.execute(
            &device,
            &queue,
            RegexInput {
                data: &data,
                status: &status,
                budget_owners: 1,
            },
            RegexBatch {
                scratch_plan: ScratchPlan::Device,
                requests,
                ids,
                count: 1,
                scratch_slots: width as usize,
            },
            SharedWork {
                values: &work,
                failures: &failures,
                evaluations: 1,
                abort_evaluation: true,
            },
        );
        assert_eq!(result.err(), Some(Error::BackendFailure), "defect {defect}");
        assert!(
            scope
                .snapshot()
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
    }
}

#[test]
fn cooperative_regex_find_preserves_ordered_ranges_full_input_context_and_exact_work() {
    use metal_backend::MappedBuffer;
    use metal_grid_plan::ResidentInput;
    use metal_regex_frontier::{
        RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork,
    };
    #[derive(Debug, Eq, PartialEq)]
    enum Outcome {
        Find(Option<[u32; 2]>),
        Predicate(bool),
    }
    let cases = [
        ("a+", "zaaab", 0, Some([1, 4])),
        ("a+?", "zaaab", 0, Some([1, 2])),
        ("a|ab", "zab", 0, Some([1, 2])),
        ("ab|a", "zab", 0, Some([1, 3])),
        ("(?:ab|a)*b", "aaab", 0, Some([0, 4])),
        ("(?:ab|a)b", "ab", 0, Some([0, 2])),
        ("a{2,4}", "zaaaaab", 0, Some([1, 5])),
        ("(?:a?)*", "aaa", 0, Some([0, 3])),
        ("(?:a|aa)+?b", "aaaab", 0, Some([0, 5])),
        ("a*?", "aaa", 0, Some([0, 0])),
        ("a*", "aaa", 0, Some([0, 3])),
        ("^", "é", 0, Some([0, 0])),
        ("$", "é", 0, Some([2, 2])),
        ("$", "é", 1, Some([2, 2])),
        ("é$", "aé", 1, Some([1, 3])),
        (r"\bαβ\b", "z αβ!", 0, Some([2, 6])),
        (r"\bα", "xα", 1, None),
        ("(?mR)^x$", "\r\nx\r\n", 0, Some([2, 3])),
        ("(?s:.)+", "é💥", 0, Some([0, 6])),
        ("^a", "za", 1, None),
        ("x", "é", 0, None),
        ("$", "a", 1, Some([1, 1])),
        ("", "", 0, Some([0, 0])),
        (
            "a+",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab",
            0,
            Some([0, 69]),
        ),
        (
            "a+?b",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab",
            0,
            Some([0, 70]),
        ),
        (
            "a.*z|a",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab",
            0,
            Some([0, 1]),
        ),
    ];
    let programs = cases
        .iter()
        .map(|(pattern, _, _, _)| regex_program(pattern, false))
        .collect::<Vec<_>>();
    let files = cases
        .iter()
        .map(|(_, source, _, _)| tape(source))
        .collect::<Vec<_>>();
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(
            &programs,
            &files,
            &vec![vec![vec![0]; programs.len()]; files.len()],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    for unit in [Some(8), None] {
        metal::objc::rc::autoreleasepool(|| {
            let guard = metal_backend::metal_execution_guard().unwrap();
            let device = metal::Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let data = metal_register_execution::upload(&device, &resident).unwrap();
            let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
            let mut requests = Vec::new();
            let mut expected = Vec::new();
            let mut scratch = 0;
            for (index, &(pattern, source, start, truth)) in cases.iter().enumerate() {
                let nfa = value::ByteNfa::compile(pattern).unwrap();
                let standard = regex::Regex::new(pattern)
                    .unwrap()
                    .find_at(source, start)
                    .map(|m| [m.start() as u32, m.end() as u32]);
                assert_eq!(
                    standard, truth,
                    "manual range: {pattern}, {source}, start={start}"
                );
                let (found, cost) = nfa.test_find_work_count_from(source.as_bytes(), start);
                assert_eq!(found, truth, "independent find oracle: {pattern}");
                assert!(cost > 0 && cost + 1 < MAX_STEPS_PER_PATH);
                let pg = resident.input.packed.programs[index];
                let width = resident.input.packed.values[pg.value_start as usize]
                    .state_count
                    .next_power_of_two();
                let file = resident.input.packed.files[index];
                for budget in [0, cost - 1, cost, cost + 1] {
                    let owner = requests.len() as u32;
                    requests.push(RegexRequest {
                        value: pg.value_start,
                        source_start: file.source_start,
                        source_length: file.source_length,
                        search_start: start as u32,
                        at: start as u32,
                        budget: budget as u32,
                        scratch,
                        width,
                        mode: 1,
                        owner,
                        evaluation: owner,
                        match_start: u32::MAX,
                        match_end: u32::MAX,
                        ..RegexRequest::default()
                    });
                    scratch += width;
                    let (outcome, used) =
                        nfa.test_find_with_budget_from(source.as_bytes(), start, budget);
                    expected.push((outcome.map(Outcome::Find), used));
                }
                let (truth, used) = nfa.test_work_count(source.as_bytes());
                assert_eq!(
                    truth,
                    Ok(regex::Regex::new(pattern).unwrap().is_match(source))
                );
                let owner = requests.len() as u32;
                requests.push(RegexRequest {
                    value: pg.value_start,
                    source_start: file.source_start,
                    source_length: file.source_length,
                    budget: used as u32,
                    scratch,
                    width,
                    owner,
                    evaluation: owner,
                    match_start: u32::MAX,
                    match_end: u32::MAX,
                    ..RegexRequest::default()
                });
                scratch += width;
                expected.push((truth.map(Outcome::Predicate), used));
            }
            let count = requests.len();
            let batch = RegexBatch {
                scratch_plan: ScratchPlan::Device,
                requests: MappedBuffer::from_slice(&device, &requests).unwrap(),
                ids: MappedBuffer::from_slice(&device, &(0..count as u32).collect::<Vec<_>>())
                    .unwrap(),
                count,
                scratch_slots: scratch as usize,
            };
            let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
            let work = MappedBuffer::zeroed::<u32>(&device, count).unwrap();
            let failures = MappedBuffer::zeroed::<u32>(&device, count).unwrap();
            let scope = metal_backend::launch::LaunchScope::enter();
            let input = RegexInput {
                data: &data,
                status: &status,
                budget_owners: count,
            };
            let shared = SharedWork {
                values: &work,
                failures: &failures,
                evaluations: count,
                abort_evaluation: true,
            };
            let started = std::time::Instant::now();
            let result = match unit {
                Some(unit) => {
                    code.execute_with_unit_size(&device, &queue, input, batch, shared, unit)
                }
                None => code.execute(&device, &queue, input, batch, shared),
            };
            let trace = scope.snapshot();
            drop(scope);
            drop(guard);
            let finished = result.expect("shared find must accept valid bounded requests");
            let actual = metal_register_execution::read::<RegexRequest>(&finished, count).unwrap();
            let costs = metal_register_execution::read::<u32>(&work, count).unwrap();
            let errors = metal_register_execution::read::<u32>(&failures, count).unwrap();
            for (index, ((truth, used), request)) in expected.into_iter().zip(actual).enumerate() {
                let outcome = match request.status {
                    1 => {
                        assert_eq!([request.match_start, request.match_end], [u32::MAX; 2]);
                        Ok(if request.mode == 0 {
                            Outcome::Predicate(false)
                        } else {
                            Outcome::Find(None)
                        })
                    }
                    2 => Ok(if request.mode == 0 {
                        assert_eq!([request.match_start, request.match_end], [u32::MAX; 2]);
                        Outcome::Predicate(true)
                    } else {
                        Outcome::Find(Some([request.match_start, request.match_end]))
                    }),
                    3 => Err(Error::WorkLimit),
                    status => panic!("unfinished find {index}: {status}"),
                };
                assert_eq!(outcome, truth, "find request {index}, unit={unit:?}");
                assert_eq!(request.used as usize, used, "request work {index}");
                assert_eq!(costs[index] as usize, used, "shared work {index}");
                assert_eq!(
                    errors[index],
                    if outcome == Err(Error::WorkLimit) {
                        2
                    } else {
                        0
                    }
                );
            }
            assert!(trace.len() <= 600);
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            if unit.is_some() {
                assert!(
                    trace
                        .iter()
                        .filter(|r| r.kernel == "regex_frontier_advance")
                        .count()
                        > 1
                );
            }
            eprintln!(
                "REGEX_FIND_EVIDENCE {}",
                serde_json::json!({"unit":unit,"requests":count,"launches":trace.len(),"seconds":started.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
            );
        });
    }
}

#[test]
fn shared_regex_batches_keep_finite_structure_in_registers_as_files_grow() {
    let programs = [
        regex_program("^#KEEP$", false),
        regex_program("^#K.*", false),
    ];
    let mut first_launches = None;
    for count in [1, 128] {
        let files = vec![tape("#KEEP"); count];
        let anchors = vec![vec![vec![0]; programs.len()]; count];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        assert_eq!(
            expected.iter().flatten().map(Vec::len).sum::<usize>(),
            count * 2
        );
        let scope = metal_backend::launch::LaunchScope::enter();
        let start = std::time::Instant::now();
        let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
        assert_eq!(actual, Ok(expected));
        let trace = scope.snapshot();
        if let Some(first) = first_launches {
            assert!(trace.len() <= first + 16);
        } else {
            assert!(
                trace.len() < 40,
                "a finite structural prefix and a short shared NFA batch must fit the normal broad-stage target; observed {} launches",
                trace.len()
            );
            first_launches = Some(trace.len());
        }
        assert!(
            trace
                .iter()
                .all(|record| record.encoded && record.submitted && record.completed)
        );
        eprintln!(
            "REGEX_BATCH_SCALING_EVIDENCE {}",
            serde_json::json!({"files":count,"programs":programs.len(),"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|record|record.peak_application_buffer_bytes).max()})
        );
    }
}

#[test]
fn empty_regex_capture_groups_reject_without_evaluating_present_unrelated_values() {
    let pattern = "^# (?:a|aa)*b$";
    let source = format!("# {}", "a".repeat(200_000));
    assert_eq!(
        value::ByteNfa::compile(pattern)
            .unwrap()
            .test_work_count(source.as_bytes())
            .0,
        Err(Error::WorkLimit)
    );
    let programs = [false, true]
        .into_iter()
        .flat_map(|negated| {
            [false, true].into_iter().map(move |any| {
                Program::new_with_capture_predicates(
                    vec![word(CAPTURE, 0, 1), word(ASSERT, 0, 0), word(EMIT, 0, 1)],
                    vec![
                        pattern.as_bytes().to_vec(),
                        b"absent".to_vec(),
                        b"present".to_vec(),
                    ],
                    vec![TextPredicate::Regex {
                        source: 0,
                        compiled: regex::Regex::new(pattern).unwrap(),
                    }],
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
                .unwrap()
            })
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
    assert_eq!(expected.errors, [vec![None; 4]]);
    assert_eq!(expected.outputs, [vec![Vec::<Output>::new(); 4]]);
    let scope = metal_backend::launch::LaunchScope::enter();
    let actual =
        execute_with_backend_outcomes(ExecutionBackend::Metal, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(
        (actual.outputs, actual.errors),
        (expected.outputs, expected.errors)
    );
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .all(|record| record.kernel != "regex_frontier_advance")
    );
    assert!(
        trace
            .iter()
            .all(|record| record.encoded && record.submitted && record.completed)
    );
}

#[test]
fn short_regex_predicates_preserve_complete_values_without_per_request_device_state() {
    let roots = 65_536u32;
    let pattern = "^# (?:ABC|ABD|AB|A){2,4}$";
    let source = (0..roots)
        .map(|index| match index % 4 {
            0 => "# ABCABD\n",
            1 => "# ABCABX\n",
            2 => "# AAA\n",
            _ => "# αβ\n",
        })
        .collect::<String>();
    let file = tape(&source);
    assert_eq!(file.transducer_len(), roots as usize + 1);
    let programs = [regex_program(pattern, false)
        .with_directives(vec![
            Directive::Set {
                key: 1,
                value: None,
            },
            Directive::Downcase { capture: 0 },
        ])
        .unwrap()];
    let anchors = [vec![(1..=roots).collect::<Vec<_>>()]];
    let expected = execute_batch(&programs, std::slice::from_ref(&file), Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), roots as usize / 2);
    for output in &expected[0][0] {
        assert_eq!(output.captures.len(), 2);
        assert_eq!(output.properties, [(1, None)]);
        let text = output.captures[0].text.as_deref().unwrap();
        assert!(text == b"# abcabd" || text == b"# aaa");
        assert_eq!(output.captures[0], output.captures[1]);
    }
    let observation = metal_backend::BufferObservation::enter();
    let scope = metal_backend::launch::LaunchScope::enter();
    let started = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &[file], &anchors);
    assert_eq!(actual, Ok(expected));
    let trace = scope.snapshot();
    let peak = trace
        .iter()
        .map(|r| r.peak_application_buffer_bytes)
        .max()
        .unwrap();
    assert_eq!(observation.wait_for_live_bytes(0), 0);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    eprintln!(
        "SHORT_REGEX_STORAGE roots={roots} peakMetalBytes={peak} dispatches={} seconds={}",
        trace.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(
        trace.len() <= 39,
        "one bounded request chunk exceeds its dispatch allowance: {}",
        trace.len()
    );
    assert!(
        peak <= 64 * 1024 * 1024,
        "short predicate state exceeds the bounded workspace: {peak}"
    );
}

#[path = "metal_regex_storage_tests.rs"]
mod storage;
