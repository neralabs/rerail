//! Lua predicate behavior through the production boundary and both CPU oracles.
use super::*;

fn lua_program(pattern: &str, query: bool) -> Program {
    let predicate = TextPredicate::LuaPattern {
        source: 1,
        compiled: LuaPattern::parse(pattern.as_bytes())
            .unwrap_or_else(|_| panic!("invalid Lua test pattern {pattern:?}")),
    };
    if query {
        Program::new_with_capture_predicates(
            vec![
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 0),
                word(ASSERT, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
            vec![predicate],
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
    } else {
        Program::new(
            vec![
                word(TEXT, 0, 0),
                word(CAPTURE, 0, 0),
                word(CAPTURE, 0, 0),
                word(EMIT, 0, 0),
            ],
            vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
            vec![predicate],
            vec![0],
            vec![],
        )
        .unwrap()
    }
}

#[test]
fn shared_lua_patterns_preserve_ordered_repetitions_captures_backreferences_and_byte_classes() {
    // The existing parser rejects position backreferences and escaped set
    // punctuation. Keep that upstream contract separate from match truth.
    assert!(LuaPattern::parse(b"^# ()a%1$").is_err());
    assert!(LuaPattern::parse(b"^# [%]%-%[]+$").is_err());
    let cases = [
        ("^# a?$", "# ", true),
        ("^# a?b$", "# ab", true),
        ("^# a?b$", "# cb", false),
        ("^# a+b$", "# aaab", true),
        ("^# a+b$", "# b", false),
        ("^# a*b$", "# b", true),
        ("^# a-b$", "# aaab", true),
        ("^# .-$", "# abc", true),
        ("^# (%a+)%s+%1$", "# alpha alpha", true),
        ("^# (%a+)%s+%1$", "# alpha beta", false),
        ("^# (a*)a%1$", "# aaa", true),
        ("^# (.)(.)%1%2$", "# éé", true),
        ("^# ()a$", "# a", true),
        ("^# %b()$", "# (a(b)c)", true),
        ("^# %b()$", "# (a(b)c", false),
        ("^# %f[%a]%a+%f[%A]$", "# word", true),
        ("^# %f[%d]%a+$", "# word", false),
        ("%f[%z]", "# end", true),
        ("^# [%a%d_]+$", "# Ab09_", true),
        ("^# [^%d]+$", "# abc9", false),
        ("^# %]%-%[$", "# ]-[", true),
        ("^# %u%l%d%p%s%x%w$", "# Aa7! f9", true),
        ("^# %A%L%D%P%S%X%W$", "# 1Aax!g_", true),
        ("", "# any", true),
        ("^$", "", true),
        ("^$", "# any", false),
        ("café", "# café", true),
        ("^# café$", "# café!", false),
    ];
    let programs = cases
        .iter()
        .flat_map(|(pattern, _, _)| [lua_program(pattern, false), lua_program(pattern, true)])
        .collect::<Vec<_>>();
    let files = cases
        .iter()
        .map(|(_, text, _)| tape(text))
        .collect::<Vec<_>>();
    let anchors = vec![vec![vec![0]; programs.len()]; files.len()];
    let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
    for (file, (_, text, accepted)) in cases.iter().enumerate() {
        for mode in 0..2 {
            let outputs = &expected[file][file * 2 + mode];
            assert_eq!(
                outputs.len(),
                usize::from(*accepted),
                "case{file}, query={mode}"
            );
            if *accepted {
                assert_eq!(outputs[0].captures.len(), 2);
                assert_eq!(outputs[0].captures[0], outputs[0].captures[1]);
                assert_eq!(outputs[0].captures[0].span, 0..text.len());
            }
        }
    }
    let native =
        execute_with_backend(ExecutionBackend::CpuJit, &programs, &files, &anchors).unwrap();
    assert_eq!(native, expected);
    let scope = metal_backend::launch::LaunchScope::enter();
    let start = std::time::Instant::now();
    let actual =
        execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors).unwrap();
    assert_eq!(actual, expected);
    let trace = scope.snapshot();
    assert!(trace.len() <= 600);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    eprintln!(
        "LUA_MATRIX_EVIDENCE {}",
        serde_json::json!({"programs":programs.len(),"files":files.len(),"launches":trace.len(),"seconds":start.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
    );
}

struct ValueCase<'a> {
    pattern: &'a str,
    text: &'a str,
    expected: Result<bool, Error>,
    regex: bool,
}

#[test]
fn lua_and_nfa_requests_keep_exact_costs_across_tiny_and_normal_units() {
    let cases = [
        ("^# (a*)a%1$", "# aaa", true),
        ("^# .-b$", "# aaab", true),
        ("^# (a?)b%1$", "# aba", true),
        ("^# %b()$", "# (())", true),
        ("^# %b()$", "# (()", false),
        ("^# %b\"\"$", "# \"x\"", false),
        ("é", "# é", true),
        ("%f[%z]", "# é", true),
        ("^# [^%a]+$", "# 9_!", true),
        ("^# ()a$", "# a", true),
        ("^# (.)(.)%1%2$", "# éé", true),
        ("^# a+$", "# aaaa", true),
    ];
    let cases = cases
        .iter()
        .enumerate()
        .map(|(index, &(pattern, text, expected))| ValueCase {
            pattern,
            text,
            expected: Ok(expected),
            regex: index + 1 == cases.len(),
        })
        .collect::<Vec<_>>();
    check_value_costs(&cases, &[Some(1), None]);
}

fn check_value_costs(cases: &[ValueCase<'_>], units: &[Option<u32>]) {
    use metal_regex_frontier::{
        RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork,
    };
    let programs = cases
        .iter()
        .map(|case| {
            if !case.regex {
                return lua_program(case.pattern, false);
            }
            Program::new(
                vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                vec![b"item".to_vec(), case.pattern.as_bytes().to_vec()],
                vec![TextPredicate::Regex {
                    source: 1,
                    compiled: regex::Regex::new(case.pattern).unwrap(),
                }],
                vec![0],
                vec![],
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = cases.iter().map(|case| tape(case.text)).collect::<Vec<_>>();
    let resident = metal_grid_plan::ResidentInput::new(
        metal_grid::MetalGridInput::build(
            &programs,
            &files,
            &vec![vec![vec![0]; programs.len()]; files.len()],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    for &unit in units {
        metal::objc::rc::autoreleasepool(|| {
            let scope = metal_backend::launch::LaunchScope::enter();
            let _guard = metal_backend::metal_execution_guard().unwrap();
            let device = metal::Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let data = metal_register_execution::upload(&device, &resident).unwrap();
            let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
            let words = metal_register_execution::read::<u32>(
                &code.code,
                code.code.buffer.length() as usize / 4,
            )
            .unwrap();
            let mut requests = Vec::new();
            let mut expected = Vec::new();
            let mut scratch = 0u32;
            for (index, case) in cases.iter().enumerate() {
                let text = case.text;
                let (outcome, cost) = match &programs[index].value_programs[0] {
                    value::ValueProgram::Lua(lua) => lua.test_work_count(text.as_bytes()),
                    value::ValueProgram::Regex(nfa) => nfa.test_work_count(text.as_bytes()),
                    _ => unreachable!(),
                };
                assert_eq!(outcome, case.expected, "case={index}");
                assert!(cost > 0 && cost <= MAX_STEPS_PER_PATH + 1);
                let value = resident.input.packed.programs[index].value_start;
                let file = resident.input.packed.files[index];
                let width = words[4 + value as usize * 4 + 1];
                let bound = cost.min(MAX_STEPS_PER_PATH);
                let mut budgets = vec![0, bound - 1, bound, (bound + 1).min(MAX_STEPS_PER_PATH)];
                budgets.dedup();
                for budget in budgets {
                    let owner = requests.len() as u32;
                    requests.push(RegexRequest {
                        value,
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
                    scratch = scratch.checked_add(width).unwrap();
                    expected.push(match &programs[index].value_programs[0] {
                        value::ValueProgram::Lua(lua) => {
                            lua.test_match_with_budget(text.as_bytes(), budget)
                        }
                        value::ValueProgram::Regex(nfa) => {
                            nfa.test_match_with_budget(text.as_bytes(), budget)
                        }
                        _ => unreachable!(),
                    });
                }
            }
            let count = requests.len();
            let requests = metal_backend::MappedBuffer::from_slice(&device, &requests).unwrap();
            let ids = metal_backend::MappedBuffer::from_slice(
                &device,
                &(0..count as u32).collect::<Vec<_>>(),
            )
            .unwrap();
            let status = metal_backend::MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
            let work = metal_backend::MappedBuffer::zeroed::<u32>(&device, count).unwrap();
            let failures = metal_backend::MappedBuffer::zeroed::<u32>(&device, count).unwrap();
            let input = RegexInput {
                data: &data,
                status: &status,
                budget_owners: count,
            };
            let batch = RegexBatch {
                scratch_plan: ScratchPlan::Device,
                requests,
                ids,
                count,
                scratch_slots: scratch as usize,
            };
            let shared = SharedWork {
                values: &work,
                failures: &failures,
                evaluations: count,
                abort_evaluation: false,
            };
            let started = std::time::Instant::now();
            let requests = match unit {
                Some(unit) => {
                    code.execute_with_unit_size(&device, &queue, input, batch, shared, unit)
                }
                None => code.execute(&device, &queue, input, batch, shared),
            }
            .unwrap();
            let actual = metal_register_execution::read::<RegexRequest>(&requests, count).unwrap();
            let work = metal_register_execution::read::<u32>(&work, count).unwrap();
            assert_eq!(
                metal_register_execution::read::<u32>(&status, 1).unwrap(),
                [0]
            );
            assert_eq!(
                metal_register_execution::read::<u32>(&failures, count).unwrap(),
                vec![0; count]
            );
            for (index, ((outcome, used), actual)) in expected.into_iter().zip(actual).enumerate() {
                let answer = match actual.status {
                    1 => Ok(false),
                    2 => Ok(true),
                    3 => Err(Error::WorkLimit),
                    status => panic!("request={index}, unfinished status={status}"),
                };
                assert_eq!(answer, outcome, "request={index}, unit={unit:?}");
                assert_eq!(actual.used as usize, used, "request={index}, unit={unit:?}");
                assert_eq!(work[index] as usize, used, "request={index}, unit={unit:?}");
            }
            let trace = scope.snapshot();
            assert!(trace.len() <= 600);
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            eprintln!(
                "LUA_UNITS_EVIDENCE {}",
                serde_json::json!({"unit":unit,"requests":count,"launches":trace.len(),"seconds":started.elapsed().as_secs_f64(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()})
            );
        });
    }
}

#[test]
fn long_lua_scans_and_backtracking_preserve_exact_work_with_bounded_shared_jobs() {
    let balanced = format!("# ({})", "a".repeat(600_000));
    let atom = format!("# {}", "a".repeat(200_000));
    let backreference = format!("# {0} {0}", "a".repeat(20_000));
    let ambiguous = format!("# {}", "a".repeat(40));
    check_value_costs(
        &[
            ValueCase {
                pattern: "^# %b()$",
                text: &balanced,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "^# a+$",
                text: &atom,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "^# (a+)%s%1$",
                text: &backreference,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "^# a*a*a*a*a*Z$",
                text: &ambiguous,
                expected: Err(Error::WorkLimit),
                regex: false,
            },
            ValueCase {
                pattern: "^# a+$",
                text: "# aa",
                expected: Ok(true),
                regex: true,
            },
        ],
        &[None],
    );
}

#[test]
fn lua_values_inherit_work_before_later_operands_and_share_one_code_owner() {
    let pattern = "^# %b()$";
    let source = format!("# ({})", "a".repeat(600_000));
    let predicate = || TextPredicate::LuaPattern {
        source: 1,
        compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
    };
    let programs = (0..6)
        .map(|shape| {
            let words = match shape {
                0 => vec![word(TEXT, 0, 0), word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
                1 => vec![
                    word(TEXT, 0, 0),
                    word(TEXT, 0, 0),
                    word(RESTORE_CURSOR, 0, 5),
                    word(EMIT, 0, 0),
                ],
                2 => vec![
                    word(REPEAT_BEGIN, 0, 0),
                    word(TEXT, 0, 0),
                    word(REPEAT_END, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(EMIT, 0, 0),
                ],
                3 => vec![
                    word(REPEAT_BEGIN, 0, 0),
                    word(TEXT, 0, 0),
                    word(TEXT, 0, 0),
                    word(RESTORE_CURSOR, 0, 5),
                    word(REPEAT_END, 0, 0),
                    word(EMIT, 0, 0),
                ],
                4 => vec![word(CAPTURE, 0, 0), word(ASSERT, 0, 0), word(EMIT, 0, 0)],
                5 => vec![
                    word(CAPTURE, 0, 0),
                    word(ASSERT, 0, 0),
                    word(ASSERT, 0, 1),
                    word(EMIT, 0, 0),
                ],
                _ => unreachable!(),
            };
            let count = if shape < 4 { 0 } else { shape - 3 };
            let assertions = (0..count)
                .map(|_| CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                })
                .collect();
            let program = Program::new_with_capture_predicates(
                words,
                vec![b"item".to_vec(), pattern.as_bytes().to_vec()],
                vec![predicate()],
                assertions,
                vec![0],
                vec![],
            )
            .unwrap();
            if shape >= 2 {
                program.with_tree_sitter_query_semantics().unwrap()
            } else {
                program
            }
        })
        .collect::<Vec<_>>();
    let value::ValueProgram::Lua(lua) = &programs[0].value_programs[0] else {
        unreachable!()
    };
    let (matched, cost) = lua.test_work_count(source.as_bytes());
    assert_eq!(matched, Ok(true));
    assert!(cost > MAX_STEPS_PER_PATH / 2 && cost + 8 < MAX_STEPS_PER_PATH);
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
            vec![
                None,
                Some(Error::WorkLimit),
                None,
                Some(Error::WorkLimit),
                None,
                Some(Error::WorkLimit)
            ],
            vec![None; 6]
        ]
    );
    for (index, rows) in expected.outputs[0].iter().enumerate() {
        assert_eq!(rows.len(), usize::from(index % 2 == 0));
    }
    assert!(expected.outputs[1].iter().all(Vec::is_empty));
    let native =
        execute_with_backend_outcomes(ExecutionBackend::CpuJit, &programs, &files, &anchors)
            .unwrap();
    assert_eq!(native, expected);
    for limit in [0, 1] {
        let scope = metal_backend::launch::LaunchScope::enter();
        let started = std::time::Instant::now();
        let actual = metal_grid_backend::execute_packed_with_result_limits(
            &programs,
            &files,
            &anchors,
            Some(&[limit; 6]),
        )
        .and_then(|execution| execution.into_outcome());
        let trace = scope.snapshot();
        drop(scope);
        if actual.is_err() {
            eprintln!(
                "LUA_INHERITED_REFUSAL {}",
                serde_json::json!({"error":actual.as_ref().err().map(|error|format!("{error:?}")),"launches":trace.len(),"lastKernels":trace.iter().rev().take(12).map(|r|&r.kernel).collect::<Vec<_>>()})
            );
            for (index, program) in programs.iter().enumerate() {
                let anchors = vec![vec![vec![0]]; files.len()];
                let result = metal_grid_backend::execute_packed_with_result_limits(
                    std::slice::from_ref(program),
                    &files,
                    &anchors,
                    Some(&[limit]),
                )
                .and_then(|execution| execution.into_outcome());
                eprintln!(
                    "LUA_INHERITED_PROGRAM {}",
                    serde_json::json!({"index":index,"error":result.as_ref().err().map(|e|format!("{e:?}")),"outcomes":result.as_ref().ok().map(|r|format!("{:?}",r.errors))})
                );
            }
        }
        let actual = actual.unwrap();
        assert_eq!(actual.errors, expected.errors);
        let mut outputs = expected.outputs.clone();
        for rows in outputs.iter_mut().flatten() {
            rows.truncate(limit as usize);
        }
        assert_eq!(actual.outputs, outputs);
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
            "inline and terminal Lua continuations must share immutable code"
        );
        eprintln!(
            "LUA_INHERITED_EVIDENCE {}",
            serde_json::json!({
                "limit":limit,"launches":trace.len(),"seconds":started.elapsed().as_secs_f64(),
                "compiledRuleBuffers":code.len(),"peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max()
            })
        );
    }
}

#[test]
fn lua_byte_chunks_resume_long_balances_atoms_backreferences_and_search_attempts() {
    let balanced = format!("# ({})", "a".repeat(600));
    let atom = format!("# {}", "a".repeat(600));
    let backreference = format!("# {0} {0}", "a".repeat(600));
    let search = format!("# {}Z", "a".repeat(600));
    check_value_costs(
        &[
            ValueCase {
                pattern: "^# %b()$",
                text: &balanced,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "^# a+$",
                text: &atom,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "^# (a+)%s%1$",
                text: &backreference,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "Z",
                text: &search,
                expected: Ok(true),
                regex: false,
            },
            ValueCase {
                pattern: "^# a+$",
                text: "# aa",
                expected: Ok(true),
                regex: true,
            },
        ],
        &[Some(8), None],
    );
}

#[test]
fn lua_predicate_batches_preserve_complete_values_without_unused_nfa_arenas() {
    let roots = 65_536u32;
    let source = (0..roots)
        .map(|index| match index % 4 {
            0 => "# ABCABD\n",
            1 => "# ABCABX\n",
            2 => "# aaa\n",
            _ => "# αβ\n",
        })
        .collect::<String>();
    let file = tape(&source);
    assert_eq!(file.transducer_len(), roots as usize + 1);
    let programs = [lua_program("^# %u+$", true)
        .with_directives(vec![
            Directive::Set {
                key: 0,
                value: None,
            },
            Directive::Downcase { capture: 0 },
        ])
        .unwrap()];
    let anchors = [vec![(1..=roots).collect::<Vec<_>>()]];
    let expected = execute_batch(&programs, std::slice::from_ref(&file), Some(&anchors)).unwrap();
    assert_eq!(expected[0][0].len(), roots as usize / 2);
    for output in &expected[0][0] {
        assert_eq!(output.properties, [(0, None)]);
        assert_eq!(output.captures.len(), 2);
        assert_eq!(output.captures[0], output.captures[1]);
        let text = output.captures[0].text.as_deref().unwrap();
        assert!(text == b"# abcabd" || text == b"# abcabx");
    }
    let observation = metal_backend::BufferObservation::enter();
    let scope = metal_backend::launch::LaunchScope::enter();
    let started = std::time::Instant::now();
    let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &[file], &anchors);
    assert_eq!(actual, Ok(expected));
    assert_eq!(observation.wait_for_live_bytes(0), 0);
    let trace = scope.snapshot();
    let peak = trace
        .iter()
        .map(|r| r.peak_application_buffer_bytes)
        .max()
        .unwrap();
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    eprintln!(
        "LUA_PREDICATE_STORAGE roots={roots} peakMetalBytes={peak} dispatches={} seconds={}",
        trace.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(
        peak <= 96 * 1024 * 1024,
        "Lua predicate scratch exceeds its bounded workspace: {peak}"
    );
}
