//! Exact structural budgets for bulk terminal runs, before witness clipping.
use super::super::metal_backend::MappedBuffer;
use super::super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::*;

fn body(words: &mut Vec<u32>, header: Option<usize>, kind: u16, alternative: u16, shape: usize) {
    let fork = words.len();
    if matches!(shape, 3 | 7 | 8 | 9) {
        words.push(word(FORK, 0, 0));
    }
    if header.is_none() && shape == 1 {
        for _ in 0..3 {
            let pc = words.len();
            words.push(word(GOTO, 0, (pc + 1) as u16));
        }
    }
    words.extend([word(KIND, 0, kind), word(CAPTURE, 0, 1)]);
    if shape == 6 {
        words.push(word(CAPTURE, 0, 1));
    }
    if matches!(shape, 3 | 7 | 8 | 9) {
        let jump = words.len();
        words.push(word(GOTO, 0, 0));
        let alternate = words.len();
        words[fork] = word(FORK, 0, alternate as u16);
        for _ in 0..if shape >= 8 { 0 } else { 6 } {
            let pc = words.len();
            words.push(word(GOTO, 0, (pc + 1) as u16));
        }
        words.extend([word(KIND, 0, alternative), word(CAPTURE, 0, 2)]);
        let join = words.len();
        words[jump] = word(GOTO, 0, join as u16);
    }
    words.push(word(REPEAT_BUMP, 0, 0));
    if let Some(header) = header {
        words.push(word(GOTO, 0, header as u16));
    }
}

fn programs() -> Vec<Program> {
    let language = SupportLang::Python.get_ts_language();
    let module = language.id_for_node_kind("module", true);
    let comment = language.id_for_node_kind("comment", true);
    let assignment = language.id_for_node_kind("expression_statement", true);
    let assignment_body = language.id_for_node_kind("assignment", true);
    let integer = language.id_for_node_kind("integer", true);
    let identifier = language.id_for_node_kind("identifier", true);
    (0..10)
        .map(|shape| {
            let siblings = shape == 5 || shape == 7;
            let mut words = vec![
                word(SAVE_CURSOR, 0, 0),
                word(
                    KIND,
                    0,
                    if shape == 4 {
                        identifier
                    } else if shape >= 8 {
                        assignment_body
                    } else if siblings {
                        comment
                    } else {
                        module
                    },
                ),
                word(CAPTURE, 0, 0),
                word(REPEAT_BEGIN, 0, 0),
            ];
            if !siblings {
                words.push(word(CHILD, if shape == 9 { 0 } else { NAMED }, 0));
            }
            let (kind, alternate) = if shape >= 8 {
                (identifier, integer)
            } else {
                (comment, assignment)
            };
            body(&mut words, None, kind, alternate, shape);
            let header = words.len();
            words.extend([
                word(FORK, 0, 0),
                word(NEXT, if shape == 9 { 0 } else { NAMED }, 0),
            ]);
            body(&mut words, Some(header), kind, alternate, shape);
            let exit = words.len();
            words[header] = word(FORK, 0, exit as u16);
            words.extend([word(REPEAT_END, 0, 0), word(RESTORE_CURSOR, 0, 0)]);
            if shape == 2 {
                for _ in 0..7 {
                    let pc = words.len();
                    words.push(word(GOTO, 0, (pc + 1) as u16));
                }
            }
            words.extend([word(CAPTURE, 0, 0), word(EMIT, 0, 0)]);
            let program = Program::new(
                words,
                vec![b"root".to_vec(), b"item".to_vec(), b"alternate".to_vec()],
                vec![],
                vec![0, 1, 2],
                vec![],
            )
            .unwrap()
            .with_tree_sitter_query_semantics()
            .unwrap();
            assert!(
                terminal_repeat_proof::prove(&program).is_some(),
                "shape={shape}"
            );
            program
        })
        .collect()
}

fn reference(
    programs: &[Program],
    file: &TapeDoc,
    anchors: &[Vec<Vec<u32>>],
    budget: usize,
) -> BatchOutcome {
    let mut expected = BatchOutcome::successful(vec![vec![Vec::new(); programs.len()]]);
    for (index, program) in programs.iter().enumerate() {
        match program.execute_terminal_reference_with_work_limit(
            file,
            anchors[0][index].clone(),
            budget,
        ) {
            Ok(mut rows) => {
                for row in &mut rows {
                    row.task_ordinal = index as u32;
                }
                expected.outputs[0][index] = rows;
            }
            Err(error) => expected.errors[0][index] = Some(error),
        }
    }
    expected
}

fn gpu(
    programs: &[Program],
    file: &TapeDoc,
    anchors: &[Vec<Vec<u32>>],
    budget: u32,
    limit: u32,
) -> BatchOutcome {
    metal::objc::rc::autoreleasepool(|| {
        let _guard = metal_backend::metal_execution_guard().unwrap();
        let resident = ResidentInput::new(
            metal_grid::MetalGridInput::build(programs, std::slice::from_ref(file), anchors)
                .unwrap(),
            Some(&vec![limit; programs.len()]),
        )
        .unwrap();
        let plan = ExecutionPlan::new(&resident, programs).unwrap();
        let device = metal::Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let data = metal_register_execution::upload(&device, &resident).unwrap();
        let roots = MappedBuffer::zeroed::<u32>(&device, resident.input.grid.roots.len()).unwrap();
        let command = queue.new_command_buffer();
        metal_parallel::encode(
            command,
            &metal_backend::pipeline_named(include_str!("metal_resident.metal"), "register_roots")
                .unwrap(),
            "register_roots",
            &[&data.buffer, &roots.buffer],
            &[],
            resident.input.grid.roots.len().div_ceil(256).max(1),
            None,
        );
        metal_backend::launch::commit_checked(command).unwrap();
        metal_backend::launch::wait_checked(command).unwrap();
        let output = metal_terminal_repeat::execute_with_work_limit(
            &device, &queue, &resident, &plan, &data, &roots, budget,
        )
        .unwrap();
        let (results, captures) = output.candidates.read().unwrap();
        PackedExecution::from_grid_records(
            Arc::clone(&resident.input.packed),
            &resident.input.grid.evaluation_spans,
            results,
            captures,
            output.errors,
        )
        .unwrap()
        .into_outcome()
        .unwrap()
    })
}

#[test]
fn terminal_runs_preserve_every_structural_budget_edge_and_failure_before_limits() {
    let programs = programs();
    let file = tape("# a\nx = 1\n# b\n# c\n");
    let assignment_kind = SupportLang::Python
        .get_ts_language()
        .id_for_node_kind("assignment", true);
    let assignment_node = (0..file.transducer_len() as u32)
        .find(|&node| file.transducer_kind(node) == assignment_kind)
        .unwrap();
    let anchors = [programs
        .iter()
        .enumerate()
        .map(|(index, _)| {
            vec![if index >= 8 {
                assignment_node
            } else {
                u32::from(index == 5 || index == 7)
            }]
        })
        .collect::<Vec<_>>()];
    let budgets = (0..=64).chain([MAX_STEPS_PER_PATH]).collect::<Vec<_>>();
    let expected = budgets
        .iter()
        .map(|&budget| {
            let expected = reference(&programs, &file, &anchors, budget);
            let native = native_cpu::execute_terminal_reference_with_work_limit(
                &programs,
                std::slice::from_ref(&file),
                &anchors,
                None,
                budget as u32,
            )
            .unwrap()
            .into_outcome()
            .unwrap();
            assert_eq!(native.errors, expected.errors, "CPU budget={budget}");
            assert_eq!(native.outputs, expected.outputs, "CPU budget={budget}");
            expected
        })
        .collect::<Vec<_>>();
    assert!(
        expected[0].errors[0]
            .iter()
            .all(|error| *error == Some(Error::WorkLimit))
    );
    assert_eq!(expected[2].errors[0][4], None);
    assert!(expected[2].outputs[0][4].is_empty());
    assert_eq!(expected[18].errors[0][0], Some(Error::WorkLimit));
    assert_eq!(expected[19].errors[0][0], None);
    assert!(!expected[19].outputs[0][0].is_empty());
    let complete = expected.last().unwrap();
    assert_eq!(complete.outputs[0][8].len(), 1);
    assert_eq!(complete.outputs[0][8][0].captures.len(), 4);
    assert_eq!(complete.outputs[0][9].len(), 2);
    assert!(
        complete.outputs[0][9]
            .iter()
            .all(|row| row.captures.len() == 3)
    );
    let public = execute_with_backend_outcomes(
        ExecutionBackend::Metal,
        &programs,
        std::slice::from_ref(&file),
        &anchors,
    )
    .unwrap();
    assert_eq!(&public, complete);
    for (budget, expected) in budgets.into_iter().zip(expected) {
        for limit in [0, 1] {
            let actual = gpu(&programs, &file, &anchors, budget as u32, limit);
            assert_eq!(
                actual.errors, expected.errors,
                "GPU budget={budget}, limit={limit}"
            );
            let mut outputs = expected.outputs.clone();
            for rows in &mut outputs[0] {
                rows.truncate(limit as usize);
            }
            assert_eq!(
                actual.outputs, outputs,
                "GPU budget={budget}, limit={limit}"
            );
        }
    }
}

#[test]
fn terminal_work_charges_empty_domains_and_preserves_segment_boundaries_across_workgroups() {
    let program = programs().remove(0);
    let programs = [program];
    let anchors = [vec![vec![0]]];
    let empty = tape("");
    for budget in 0..=6 {
        let expected = reference(&programs, &empty, &anchors, budget);
        assert_eq!(
            expected.errors[0][0],
            (budget < 5).then_some(Error::WorkLimit)
        );
        assert!(expected.outputs[0][0].is_empty());
        let native = native_cpu::execute_terminal_reference_with_work_limit(
            &programs,
            std::slice::from_ref(&empty),
            &anchors,
            None,
            budget as u32,
        )
        .unwrap()
        .into_outcome()
        .unwrap();
        assert_eq!(native, expected);
        for limit in [0, 1] {
            assert_eq!(
                gpu(&programs, &empty, &anchors, budget as u32, limit),
                expected
            );
        }
    }
    let file = tape(&"# a\n# b\nx = 1\n".repeat(100));
    let complete = reference(&programs, &file, &anchors, MAX_STEPS_PER_PATH);
    assert_eq!(complete.errors, [vec![None]]);
    assert_eq!(complete.outputs[0][0].len(), 100);
    assert!(
        complete.outputs[0][0]
            .iter()
            .all(|row| row.captures.len() == 4)
    );
    for budget in [18, 19] {
        let expected = reference(&programs, &file, &anchors, budget);
        assert_eq!(
            expected.errors[0][0],
            (budget == 18).then_some(Error::WorkLimit)
        );
        let native = native_cpu::execute_terminal_reference_with_work_limit(
            &programs,
            std::slice::from_ref(&file),
            &anchors,
            None,
            budget as u32,
        )
        .unwrap()
        .into_outcome()
        .unwrap();
        assert_eq!(native, expected);
        for limit in [0, 1] {
            let scope = metal_backend::launch::LaunchScope::enter();
            let actual = gpu(&programs, &file, &anchors, budget as u32, limit);
            assert_eq!(actual.errors, expected.errors);
            let mut outputs = expected.outputs.clone();
            outputs[0][0].truncate(limit as usize);
            assert_eq!(actual.outputs, outputs);
            let trace = scope.snapshot();
            assert!(trace.len() <= 600);
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            eprintln!(
                "TERMINAL_WORK_EVIDENCE {}",
                serde_json::json!({
                    "budget":budget,"limit":limit,"segments":100,"launches":trace.len(),
                    "peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max(),
                })
            );
        }
    }
}
