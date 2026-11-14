//! Scratch classes share exact NFA work, owners, and failure behavior.
use super::*;
use metal_backend::MappedBuffer;
use metal_grid_plan::ResidentInput;
use metal_regex_frontier::{
    RegexBatch, RegexInput, RegexProgram, RegexRequest, ScratchPlan, SharedWork,
};

#[test]
fn mixed_scratch_classes_preserve_exact_budget_edges_and_inherited_work() {
    let cases = [
        ("^a+$", "aaaa".to_owned(), false),
        ("^a+b$", "aaaa".to_owned(), false),
        ("", String::new(), false),
        (r"\bαβ\b", "αβ".to_owned(), false),
        ("^a+$", "a".repeat(4095), false),
        ("^a+$", "a".repeat(4096), false),
        ("^a+b$", "a".repeat(4097), false),
        ("^a{256}b$", "a".to_owned(), false),
        ("^é+$", "é".repeat(2047), false),
        ("%u+", "ABC".to_owned(), true),
        ("%u+", "α".to_owned(), true),
    ];
    let programs = cases
        .iter()
        .map(|(pattern, _, lua)| {
            if !lua {
                return regex_program(pattern, false);
            }
            Program::new_with_capture_predicates(
                vec![
                    word(CAPTURE, 0, 0),
                    word(CAPTURE, 0, 0),
                    word(ASSERT, 0, 0),
                    word(EMIT, 0, 0),
                ],
                vec![pattern.as_bytes().to_vec(), b"value".to_vec()],
                vec![TextPredicate::LuaPattern {
                    source: 0,
                    compiled: LuaPattern::parse(pattern.as_bytes()).unwrap(),
                }],
                vec![CapturePredicate::Text {
                    capture: 0,
                    predicate: 0,
                    negated: false,
                    any: false,
                }],
                vec![1],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap()
        })
        .collect::<Vec<_>>();
    let files = cases
        .iter()
        .map(|(_, text, _)| tape(text))
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
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
    let code_words =
        metal_register_execution::read::<u32>(&code.code, code.code.buffer.length() as usize / 4)
            .unwrap();
    for plan in [ScratchPlan::Device, ScratchPlan::CooperativePredicates] {
        let before = scope.snapshot().len();
        let mut rows = Vec::new();
        let mut expected = Vec::new();
        let mut inherited = Vec::new();
        let mut scratch = 0;
        let mut short = 0;
        let mut long = 0;
        let mut wide = 0;
        for _batch in 0..5 {
            for (index, (pattern, text, lua)) in cases.iter().enumerate() {
                let oracle = &programs[index].value_programs[0];
                let (outcome, cost) = match oracle {
                    value::ValueProgram::Regex(nfa) => nfa.test_work_count(text.as_bytes()),
                    value::ValueProgram::Lua(lua) => lua.test_work_count(text.as_bytes()),
                    _ => unreachable!(),
                };
                let truth = if *lua {
                    text == "ABC"
                } else {
                    regex::Regex::new(pattern).unwrap().is_match(text)
                };
                assert_eq!(outcome, Ok(truth));
                assert!(cost > 0 && cost + 1 < MAX_STEPS_PER_PATH);
                let pg = resident.input.packed.programs[index];
                let width = code_words[4 + pg.value_start as usize * 4 + 1];
                let local = !lua && width <= 256 && text.len() < 4096;
                short += usize::from(local);
                long += usize::from(text.len() >= 4096);
                wide += usize::from(!lua && width > 256);
                let physical = if matches!(plan, ScratchPlan::CooperativePredicates) && local {
                    0
                } else {
                    width
                };
                let file = resident.input.packed.files[index];
                for prefix in [false, true] {
                    for budget in [cost - 1, cost, cost + 1] {
                        let owner = rows.len() as u32;
                        rows.push(RegexRequest {
                            value: pg.value_start,
                            source_start: file.source_start,
                            source_length: file.source_length,
                            budget: if prefix {
                                MAX_STEPS_PER_PATH as u32
                            } else {
                                budget as u32
                            },
                            scratch,
                            width,
                            owner,
                            evaluation: owner,
                            match_start: u32::MAX,
                            match_end: u32::MAX,
                            ..Default::default()
                        });
                        scratch += physical;
                        inherited.push(if prefix {
                            (MAX_STEPS_PER_PATH - budget) as u32
                        } else {
                            0
                        });
                        expected.push(match oracle {
                            value::ValueProgram::Regex(nfa) => {
                                nfa.test_match_with_budget(text.as_bytes(), budget)
                            }
                            value::ValueProgram::Lua(lua) => {
                                lua.test_match_with_budget(text.as_bytes(), budget)
                            }
                            _ => unreachable!(),
                        });
                    }
                }
            }
        }
        assert!(short > 0 && long > 0 && wide > 0);
        let count = rows.len();
        let work = MappedBuffer::from_slice(&device, &inherited).unwrap();
        let failures = MappedBuffer::zeroed::<u32>(&device, count).unwrap();
        let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let actual = code
            .execute(
                &device,
                &queue,
                RegexInput {
                    data: &data,
                    status: &status,
                    budget_owners: count,
                },
                RegexBatch {
                    scratch_plan: plan,
                    requests: MappedBuffer::from_slice(&device, &rows).unwrap(),
                    ids: MappedBuffer::from_slice(&device, &(0..count as u32).collect::<Vec<_>>())
                        .unwrap(),
                    count,
                    scratch_slots: scratch as usize,
                },
                SharedWork {
                    values: &work,
                    failures: &failures,
                    evaluations: count,
                    abort_evaluation: true,
                },
            )
            .unwrap();
        let actual = metal_register_execution::read::<RegexRequest>(&actual, count).unwrap();
        let work = metal_register_execution::read::<u32>(&work, count).unwrap();
        let failures = metal_register_execution::read::<u32>(&failures, count).unwrap();
        for (index, ((outcome, used), actual)) in expected.into_iter().zip(actual).enumerate() {
            let observed = match actual.status {
                1 => Ok(false),
                2 => Ok(true),
                3 => Err(Error::WorkLimit),
                value => panic!("request {index} has invalid terminal status {value}"),
            };
            assert_eq!(observed, outcome, "request {index}");
            assert_eq!(actual.used as usize, used, "request {index}");
            assert_eq!(
                work[index],
                inherited[index] + used as u32,
                "request {index}"
            );
            assert_eq!(
                failures[index],
                if outcome == Err(Error::WorkLimit) {
                    2
                } else {
                    0
                }
            );
            let original = rows[index];
            assert_eq!(
                (
                    actual.value,
                    actual.source_start,
                    actual.source_length,
                    actual.budget,
                    actual.scratch,
                    actual.width,
                    actual.owner,
                    actual.evaluation
                ),
                (
                    original.value,
                    original.source_start,
                    original.source_length,
                    original.budget,
                    original.scratch,
                    original.width,
                    original.owner,
                    original.evaluation
                )
            );
            assert_eq!(
                (
                    actual.mode,
                    actual.search_start,
                    actual.match_start,
                    actual.match_end
                ),
                (0, 0, u32::MAX, u32::MAX)
            );
        }
        let trace = scope.snapshot();
        let trace = &trace[before..];
        assert_eq!(
            trace
                .iter()
                .filter(|r| r.kernel == "regex_frontier_advance")
                .count(),
            2
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
    }
}

#[test]
fn cooperative_scratch_rejects_inconsistent_geometry_before_semantic_work() {
    let programs = [
        regex_program("^a+$", false),
        regex_program("^a{256}b$", false),
    ];
    let files = [tape(&"a".repeat(4096))];
    let resident = ResidentInput::new(
        metal_grid::MetalGridInput::build(&programs, &files, &[vec![vec![0]; 2]]).unwrap(),
        None,
    )
    .unwrap();
    let _guard = metal_backend::metal_execution_guard().unwrap();
    let scope = metal_backend::launch::LaunchScope::enter();
    let device = metal::Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let data = metal_register_execution::upload(&device, &resident).unwrap();
    let code = RegexProgram::new(&device, &resident.input.packed).unwrap();
    for fault in 0..8 {
        let pg = resident.input.packed.programs[usize::from(fault == 5)];
        let width = resident.input.packed.values[pg.value_start as usize]
            .state_count
            .next_power_of_two();
        if fault == 5 {
            assert!(width > 256);
        }
        let mut row = RegexRequest {
            value: pg.value_start,
            source_length: 4,
            budget: MAX_STEPS_PER_PATH as u32,
            width,
            match_start: u32::MAX,
            match_end: u32::MAX,
            ..Default::default()
        };
        let mut slots = 0;
        match fault {
            0 => row.scratch = 1,
            1 => slots = width,
            2 => row.width *= 2,
            3 => row.mode = 1,
            4 => row.source_length = 4096,
            5 => {}
            6 => {} // A smaller byte unit cannot use the producer's one-chunk proof.
            7 => row.source_start = 4096,
            _ => unreachable!(),
        }
        let rows = MappedBuffer::from_slice(&device, &[row]).unwrap();
        let before = scope.snapshot().len();
        let status = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let work = MappedBuffer::from_slice(&device, &[17u32]).unwrap();
        let failures = MappedBuffer::zeroed::<u32>(&device, 1).unwrap();
        let input = RegexInput {
            data: &data,
            status: &status,
            budget_owners: 1,
        };
        let batch = RegexBatch {
            scratch_plan: ScratchPlan::CooperativePredicates,
            requests: MappedBuffer {
                buffer: rows.buffer.clone(),
            },
            ids: MappedBuffer::from_slice(&device, &[0u32]).unwrap(),
            count: 1,
            scratch_slots: slots as usize,
        };
        let shared = SharedWork {
            values: &work,
            failures: &failures,
            evaluations: 1,
            abort_evaluation: true,
        };
        let result = if fault == 6 {
            code.execute_with_unit_size(&device, &queue, input, batch, shared, 8)
        } else {
            code.execute(&device, &queue, input, batch, shared)
        };
        assert_eq!(result.err(), Some(Error::BackendFailure), "fault {fault}");
        assert_eq!(
            metal_register_execution::read::<RegexRequest>(&rows, 1).unwrap(),
            [row]
        );
        assert_eq!(
            metal_register_execution::read::<u32>(&work, 1).unwrap(),
            [17]
        );
        assert_eq!(
            metal_register_execution::read::<u32>(&failures, 1).unwrap(),
            [0]
        );
        let trace = scope.snapshot();
        let trace = &trace[before..];
        assert_eq!(trace.len(), 1, "fault {fault}");
        assert!(trace.iter().all(|r| r.kernel == "value_frontier_preflight"
            && r.encoded
            && r.submitted
            && r.completed));
    }
}
