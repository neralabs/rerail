//! Exact worklist oracles: readiness is a task barrier, never physical arrival order.
use super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard};
use super::metal_grid::MetalGridInput;
use super::metal_grid_plan::{ExecutionPlan, ResidentInput};
use super::metal_ready_batch::{self, ReadyBatch, ReadyInput};
use super::metal_state_values::{
    COLUMNS, FrontierRecords, INITIAL_COLUMNS, POSITIONED_COLUMNS, StateValues, ValueFrontier,
};
use super::*;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::tree_sitter::StrDoc;
use crate::code_rule_dsl::language::GrammarDialect as SupportLang;
use metal::Device;

fn fixture(tasks: usize) -> (ResidentInput, ExecutionPlan) {
    let words = vec![
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, 6),
        word(NEXT, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 2),
        word(REPEAT_END, 0, 0),
        word(REPEAT_BEGIN, 0, 1),
        word(REPEAT_BUMP, 0, 1),
        word(FORK, GREEDY, 13),
        word(NEXT, 0, 0),
        word(REPEAT_BUMP, 0, 1),
        word(GOTO, 0, 9),
        word(REPEAT_END, 0, 1),
        word(EMIT, 0, 0),
    ];
    let ordinary = Program::new(words, vec![b"match".to_vec()], vec![], vec![0], vec![]).unwrap();
    let query = ordinary.clone().with_tree_sitter_query_semantics().unwrap();
    fixture_programs(tasks, [query, ordinary])
}
fn fixture_programs(tasks: usize, programs: [Program; 2]) -> (ResidentInput, ExecutionPlan) {
    let source = Arc::new("# comment\n".repeat(tasks.max(3)));
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let doc = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let roots = (0..tasks as u32).collect::<Vec<_>>();
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[doc], &[vec![roots, vec![0]]]).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    (resident, plan)
}
fn row(task: u32, pc: u32, node: u32, program: u32, kind: u32, ordinal: u32) -> [u32; COLUMNS] {
    [task, pc, node, 0, program, kind, 0, 0, 0, 1, 0, 0, ordinal]
}
fn input(device: &Device, rows: &[[u32; COLUMNS]]) -> (ValueFrontier, StateValues) {
    input_records(device, rows)
}
fn input_records<const FIELDS: usize>(
    device: &Device,
    rows: &[[u32; COLUMNS]],
) -> (FrontierRecords<FIELDS>, StateValues) {
    let mut columns = vec![u32::MAX; (rows.len() + 7) * FIELDS];
    for (i, row) in rows.iter().enumerate() {
        for c in 0..FIELDS {
            columns[c * rows.len() + i] = row[c];
        }
    }
    (
        FrontierRecords {
            columns: MappedBuffer::from_slice(device, &columns).unwrap(),
            counts: MappedBuffer::from_slice(
                device,
                &[rows.len() as u32, 0, rows.len().div_ceil(256) as u32, 1, 1],
            )
            .unwrap(),
            capacity: rows.len() + 7,
            generation: 5,
        },
        StateValues {
            cursors: MappedBuffer::zeroed::<u32>(device, 0).unwrap(),
            repeats: MappedBuffer::from_slice(device, &[[0u32, 1, 0, 0]]).unwrap(),
            captures: MappedBuffer::zeroed::<u32>(device, 0).unwrap(),
            counts: MappedBuffer::from_slice(device, &[0u32, 1]).unwrap(),
            cursor_capacity: 0,
            repeat_capacity: 1,
            capture_capacity: 0,
            generation: 5,
        },
    )
}
fn output<const FIELDS: usize>(device: &Device, capacity: usize) -> ReadyBatch<FIELDS> {
    assert!([COLUMNS, POSITIONED_COLUMNS].contains(&FIELDS));
    ReadyBatch {
        indices: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity * 4]).unwrap(),
        counts: MappedBuffer::from_slice(device, &[u32::MAX; 17]).unwrap(),
        capacity,
        generation: 5,
    }
}
fn words(buffer: &MappedBuffer, count: usize) -> Vec<u32> {
    unsafe { std::slice::from_raw_parts(buffer.buffer.contents().cast(), count).to_vec() }
}
fn lists<const FIELDS: usize>(output: &ReadyBatch<FIELDS>, expected: &[Vec<u32>; 4]) {
    let counts = words(&output.counts, 16);
    let indices = words(&output.indices, output.capacity * 4);
    for k in 0..4 {
        assert_eq!(counts[k], expected[k].len() as u32, "list {k}");
        assert_eq!(
            &counts[4 + k * 3..7 + k * 3],
            &[expected[k].len().div_ceil(256) as u32, 1, 1]
        );
        assert_eq!(
            &indices[k * output.capacity..k * output.capacity + expected[k].len()],
            &expected[k]
        );
        assert!(
            indices[k * output.capacity + expected[k].len()..(k + 1) * output.capacity]
                .iter()
                .all(|&x| x == u32::MAX)
        );
    }
}

#[test]
fn initial_readiness_preserves_exact_roots_and_rejects_inconsistent_producers() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(3);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let queue = device.new_command_queue();
    // Tasks 0 and 3 legitimately select the same root for different Programs.
    for defect in 0..13 {
        let mut columns = vec![0u32, 1, 2, 3, 0, 1, 2, 0, 0, 0, 0, 1];
        match defect {
            1 => columns[4] = 1, // Same file, wrong original root.
            2 => {
                columns[1] = 0;
                columns[5] = 0;
            } // Duplicate task/root.
            3 => columns[8] = 1, // Wrong Program for task zero.
            8 => columns[0] = u32::MAX,
            9 => columns[8] = u32::MAX,
            11 => columns = vec![0, 1, 2, 0, 1, 2, 0, 0, 0, u32::MAX, u32::MAX, u32::MAX],
            12 => {
                columns[0] = 3;
                columns[4] = 0;
                columns[8] = 1;
            }
            _ => {}
        }
        let states = FrontierRecords::<INITIAL_COLUMNS> {
            columns: MappedBuffer::from_slice(&device, &columns).unwrap(),
            counts: MappedBuffer::from_slice(
                &device,
                &[
                    if defect == 11 { 3u32 } else { 4 },
                    u32::from(defect == 7),
                    1,
                    1,
                    1,
                ],
            )
            .unwrap(),
            capacity: if defect == 10 { 4097 } else { 4 },
            generation: u64::from(defect == 4),
        };
        let values = StateValues {
            cursors: MappedBuffer::zeroed::<[u32; 4]>(&device, 0).unwrap(),
            repeats: MappedBuffer::zeroed::<[u32; 4]>(&device, 0).unwrap(),
            captures: MappedBuffer::zeroed::<[u32; 4]>(&device, 0).unwrap(),
            counts: MappedBuffer::from_slice(&device, &[u32::from(defect == 6), 0]).unwrap(),
            cursor_capacity: usize::from(defect == 5),
            repeat_capacity: 0,
            capture_capacity: 0,
            generation: states.generation,
        };
        let output = ReadyBatch::<INITIAL_COLUMNS> {
            indices: MappedBuffer::from_slice(&device, &[u32::MAX; 4]).unwrap(),
            counts: MappedBuffer::from_slice(&device, &[u32::MAX; 17]).unwrap(),
            capacity: 4,
            generation: states.generation,
        };
        let result = metal::objc::rc::autoreleasepool(|| {
            let command = queue.new_command_buffer();
            let pending = metal_ready_batch::encode(
                &device,
                command,
                ReadyInput {
                    resident: &resident,
                    plan: &plan,
                    data: &data,
                    states: &states,
                    values: &values,
                },
                &output,
            )?;
            commit_checked(command)?;
            wait_checked(command)?;
            pending.check_completed()
        });
        if defect == 0 {
            assert_eq!(result, Ok(()));
            assert_eq!(words(&output.indices, 4), [0, 1, 2, 3]);
            assert_eq!(
                words(&output.counts, 17),
                [4, 0, 0, 0, 1, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0]
            );
        } else {
            assert_eq!(result, Err(Error::BackendFailure), "defect {defect}");
            assert_eq!(
                words(&output.indices, 4),
                [u32::MAX; 4],
                "defect {defect} published a ready row"
            );
        }
    }
}
#[test]
fn readiness_holds_late_arrivals_and_later_regions_without_blocking_independent_tasks() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(3);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let rows = vec![
        row(0, 2, 1, 0, 0, 8),
        row(1, 9, 2, 0, 0, 12),
        row(1, 2, 3, 0, 0, 19),
        row(0, 1, 2, 0, 0, 7),
        row(1, 2, 1, 0, 0, 4),
        row(0, 14, 1, 0, 1, 99),
        row(3, 2, 1, 1, 2, 21),
        row(2, 2, 1, 0, 4, 6),
        row(2, 2, 2, 0, 0, 3),
    ];
    for remove_predecessor in [false, true] {
        let mut rows = rows.clone();
        if remove_predecessor {
            rows.remove(3);
        }
        let (states, values) = input(&device, &rows);
        let output = output(&device, rows.len() + 7);
        let before = words(&states.columns, states.capacity * COLUMNS);
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_ready_batch::encode(
            &device,
            command,
            ReadyInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &states,
                values: &values,
            },
            &output,
        )
        .expect("GPU readiness must produce exact stable worklists");
        assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
        // Re-encode on a fresh command because the completion check consumes its owner.
        let command = queue.new_command_buffer();
        let pending = metal_ready_batch::encode(
            &device,
            command,
            ReadyInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &states,
                values: &values,
            },
            &output,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
        let expected = if remove_predecessor {
            [vec![5, 6], vec![0, 2, 3], vec![1, 7], vec![4]]
        } else {
            [vec![3, 6, 7], vec![2, 4], vec![0, 1, 8], vec![5]]
        };
        lists(&output, &expected);
        assert_eq!(words(&states.columns, states.capacity * COLUMNS), before);
    }
}

#[test]
fn pending_greedy_exits_stay_held_and_do_not_hold_their_own_header_barrier() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(1);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let rows = [
        row(0, 2, 1, 0, 0, 0),
        row(0, 6, 1, 0, 6, 1),
        row(1, 6, 1, 1, 7, 2),
    ];
    let (states, values) = input_records::<POSITIONED_COLUMNS>(&device, &rows);
    let output = output(&device, rows.len() + 7);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending = metal_ready_batch::encode(
        &device,
        command,
        ReadyInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &states,
            values: &values,
        },
        &output,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    lists(&output, &[vec![], vec![0], vec![1, 2], vec![]]);
}
#[test]
fn readiness_storage_tracks_dynamic_tasks_after_a_large_completed_register_domain() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (dynamic, file, _) = super::metal_frontier_conformance::convergence_case(8192);
    let direct = Program::new(
        vec![word(CAPTURE, 0, 0), word(EMIT, 0, 0)],
        vec![b"direct".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let mut programs = vec![direct; 128];
    programs.push(dynamic);
    let mut selected = vec![(1..=8192).collect::<Vec<_>>(); 128];
    selected.push(vec![1, 2]);
    let resident = ResidentInput::new(
        MetalGridInput::build(&programs, &[file], &[selected]).unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let first = 128 * 8192;
    let rows = [
        row(first, 0, 1, 128, 0, 0),
        row(first + 1, 11, 2, 128, 1, 0),
    ];
    let (states, values) = input(&device, &rows);
    let output = output(&device, rows.len() + 7);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let allocations = super::metal_backend::BufferObservation::enter();
    let pending = metal_ready_batch::encode(
        &device,
        command,
        ReadyInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &states,
            values: &values,
        },
        &output,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let scheduling_bytes = allocations.live_bytes();
    pending.check_completed().unwrap();
    lists(&output, &[vec![0], vec![], vec![], vec![1]]);
    assert!(
        scheduling_bytes < 4_000_000,
        "two dynamic tasks must schedule without bookkeeping for a million completed register tasks: {scheduling_bytes}"
    );
    assert!(scope.snapshot().iter().all(|record| record.completed));
}

#[test]
fn ready_batches_scale_across_tasks_and_scan_boundaries_with_stable_original_indices() {
    let device = Device::system_default().unwrap();
    for tasks in [0, 1, 128, 4096, 16_384, 21_845, 21_846, 43_691] {
        let _guard = metal_execution_guard().unwrap();
        let (resident, plan) = fixture(tasks);
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let mut rows = Vec::new();
        let mut closure = Vec::new();
        let mut held = Vec::new();
        for task in 0..tasks as u32 {
            held.push(rows.len() as u32);
            rows.push(row(task, 9, 1, 0, 0, 30));
            for (node, ordinal) in [(3, 19), (1, 4)] {
                closure.push(rows.len() as u32);
                rows.push(row(task, 2, node, 0, 0, ordinal));
            }
        }
        let (states, values) = input(&device, &rows);
        let output = output(&device, rows.len() + 7);
        let scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_ready_batch::encode(
            &device,
            command,
            ReadyInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &states,
                values: &values,
            },
            &output,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
        lists(&output, &[vec![], closure, held, vec![]]);
        let trace = scope.snapshot();
        let selections = trace
            .iter()
            .filter(|r| {
                !matches!(
                    r.kernel.as_str(),
                    "ready_initialize" | "ready_classify" | "ready_nodes"
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            selections.len(),
            if rows.len() + 7 <= 8_388_608 { 1 } else { 3 },
            "stable selection uses one cooperative block or counted block prefixes"
        );
        assert!(trace.len() < 20);
        assert!(trace.iter().all(|r| r.completed));
        eprintln!(
            "READY_BATCH_EVIDENCE {}",
            serde_json::json!({"tasks":tasks,"states":rows.len(),"dispatches":trace})
        );
    }
}

#[test]
fn broad_ready_selection_preserves_every_original_index_in_one_cooperative_dispatch() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture(1);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let queue = device.new_command_queue();
    let mut measured = Vec::new();
    for count in [65_529usize, 65_536, 131_073, 2_800_001] {
        let rows = (0..count)
            .map(|i| row(0, if i % 3 == 0 { 9 } else { 2 }, 1, 0, 0, i as u32))
            .collect::<Vec<_>>();
        let closure = (0..count as u32).filter(|i| i % 3 != 0).collect::<Vec<_>>();
        let held = (0..count as u32).filter(|i| i % 3 == 0).collect::<Vec<_>>();
        let (states, values) = input(&device, &rows);
        let output = output(&device, states.capacity);
        let scope = LaunchScope::enter();
        let before = scope.snapshot().len();
        let started = std::time::Instant::now();
        let command = queue.new_command_buffer();
        let pending = metal_ready_batch::encode(
            &device,
            command,
            ReadyInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &states,
                values: &values,
            },
            &output,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
        let execution_seconds = started.elapsed().as_secs_f64();
        lists(&output, &[vec![], closure, held, vec![]]);
        let trace = scope.snapshot();
        let trace = &trace[before..];
        let selections = trace
            .iter()
            .filter(|r| {
                !matches!(
                    r.kernel.as_str(),
                    "ready_initialize" | "ready_classify" | "ready_nodes"
                )
            })
            .count();
        eprintln!(
            "BROAD_READY_SELECTION_EVIDENCE {}",
            serde_json::json!({"rows":count,"seconds":started.elapsed().as_secs_f64(),"executionSeconds":execution_seconds,"selectionDispatches":selections,"dispatches":trace.len()})
        );
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        measured.push((count, selections));
    }
    assert!(
        measured.iter().all(|&(_, count)| count == 1),
        "the complete stable selections must each finish in one cooperative dispatch: {measured:?}"
    );
}
#[test]
fn malformed_or_undersized_batches_publish_no_work_and_leave_payload_untouched() {
    let device = Device::system_default().unwrap();
    for case in 0..9 {
        let _guard = metal_execution_guard().unwrap();
        let (resident, plan) = fixture(3);
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let mut rows = vec![row(0, 2, 1, 0, 0, 0), row(0, 2, 2, 0, 0, 1)];
        match case {
            0 => rows[0][0] = 4,
            1 => rows[0][1] = 15,
            2 => rows[0][2] = u32::MAX,
            3 => rows[0][4] = 1,
            4 => rows[0][5] = 6,
            5 => rows[0][7] = 1,
            _ => {}
        }
        let (states, values) = input(&device, &rows);
        let output = output(&device, if case == 8 { 1 } else { 9 });
        if case == 6 {
            unsafe { states.counts.buffer.contents().cast::<u32>().write(10) };
        }
        if case == 7 {
            unsafe {
                states
                    .counts
                    .buffer
                    .contents()
                    .cast::<u32>()
                    .add(2)
                    .write(10)
            };
        }
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_ready_batch::encode(
            &device,
            command,
            ReadyInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &states,
                values: &values,
            },
            &output,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(
            pending.check_completed(),
            Err(if case == 8 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            }),
            "case {case}"
        );
        lists(&output, &[vec![], vec![], vec![], vec![]]);
    }
}

#[test]
fn readiness_rejects_stale_generation_aliases_and_changed_program_proofs_before_dispatch() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let scope = LaunchScope::enter();
    for case in 0..5 {
        let (mut resident, plan) = fixture(3);
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let (states, mut values) = input(&device, &[row(0, 2, 1, 0, 0, 0)]);
        let mut output = output(&device, 8);
        match case {
            0 => output.generation += 1,
            1 => values.generation += 1,
            2 => {
                output.indices = MappedBuffer {
                    buffer: states.columns.buffer.clone(),
                }
            }
            3 => output.capacity = 10000,
            4 => {
                Arc::make_mut(&mut resident.input.packed).words[3] = word(NEXT, NAMED, 0);
            }
            _ => unreachable!(),
        }
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        assert_eq!(
            metal_ready_batch::encode(
                &device,
                command,
                ReadyInput {
                    resident: &resident,
                    plan: &plan,
                    data: &data,
                    states: &states,
                    values: &values
                },
                &output
            )
            .err(),
            Some(if case == 4 {
                Error::InvalidAnchor
            } else {
                Error::BackendFailure
            })
        );
    }
    assert!(scope.snapshot().is_empty());
}

#[test]
fn general_query_headers_advance_only_the_lowest_node_and_ordinary_occurrences_all_advance() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let ordinary = Program::new(
        vec![
            word(REPEAT_BEGIN, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(FORK, GREEDY, 7),
            word(NEXT, 0, 0),
            word(FIRST, 0, 0),
            word(REPEAT_BUMP, 0, 0),
            word(GOTO, 0, 2),
            word(REPEAT_END, 0, 0),
            word(EMIT, 0, 0),
        ],
        vec![b"match".to_vec()],
        vec![],
        vec![0],
        vec![],
    )
    .unwrap();
    let query = ordinary.clone().with_tree_sitter_query_semantics().unwrap();
    assert!(super::closure_proof::regions(&query).unwrap().is_empty());
    let (resident, plan) = fixture_programs(1, [query, ordinary]);
    let data = super::metal_register_execution::upload(&device, &resident).unwrap();
    let rows = [
        row(0, 2, 3, 0, 0, 9),
        row(0, 2, 1, 0, 0, 8),
        row(0, 2, 2, 0, 0, 7),
        row(0, 2, 1, 0, 0, 6),
        row(1, 2, 3, 1, 2, 5),
        row(1, 2, 1, 1, 2, 4),
        row(1, 2, 1, 1, 2, 3),
        row(1, 8, 1, 1, 3, 2),
    ];
    let (states, values) = input(&device, &rows);
    let output = output(&device, rows.len() + 7);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending = metal_ready_batch::encode(
        &device,
        command,
        ReadyInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &states,
            values: &values,
        },
        &output,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    lists(&output, &[vec![1, 3, 4, 5, 6], vec![], vec![0, 2], vec![7]]);
}

#[test]
fn fused_ready_rejects_uninitialized_work_and_foreign_output_bindings() {
    metal::objc::rc::autoreleasepool(|| {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let (resident, plan) = fixture(1);
        let data = super::metal_register_execution::upload(&device, &resident).unwrap();
        let foreign_data = super::metal_register_execution::upload(&device, &resident).unwrap();
        for case in 0..8 {
            let (mut states, mut values) =
                input_records::<POSITIONED_COLUMNS>(&device, &[row(0, 2, 1, 0, 0, 0)]);
            let prepared = metal_ready_batch::FusedReady::new(
                &device,
                &resident,
                &plan,
                &data,
                metal_ready_batch::FusedReadyFrame {
                    columns: &states.columns,
                    counts: &states.counts,
                    value_counts: &values.counts,
                    capacity: states.capacity,
                    generation: 5,
                    values: [0, 1, 0],
                },
            )
            .unwrap();
            let mut output = output::<POSITIONED_COLUMNS>(&device, states.capacity);
            output.counts = MappedBuffer {
                buffer: prepared.counts.buffer.clone(),
            };
            if case != 0 {
                let source = concat!(
                    include_str!("metal_resident.metal"),
                    include_str!("metal_input_layout.metal"),
                    include_str!("metal_task_groups.metal"),
                    include_str!("metal_ready_batch.metal")
                );
                let source = format!("#define INPUT_COLUMNS 12u\n{source}");
                let command = queue.new_command_buffer();
                for (name, buffers, count) in [
                    (
                        "ready_initialize",
                        vec![
                            &*states.counts.buffer,
                            &values.counts.buffer,
                            &prepared.tasks.buffer,
                            &prepared.status.buffer,
                            &prepared.counts.buffer,
                        ],
                        prepared.parameters()[2] as usize,
                    ),
                    (
                        "ready_classify",
                        vec![
                            &*data.buffer,
                            &states.columns.buffer,
                            &states.counts.buffer,
                            &values.counts.buffer,
                            &prepared.proof.buffer,
                            &prepared.tasks.buffer,
                            &prepared.status.buffer,
                            &prepared.counts.buffer,
                        ],
                        states.capacity,
                    ),
                ] {
                    super::metal_parallel::encode(
                        command,
                        &super::metal_backend::pipeline_named(&source, name).unwrap(),
                        name,
                        &buffers,
                        prepared.parameters(),
                        count.div_ceil(256).max(1),
                        None,
                    );
                }
                commit_checked(command).unwrap();
                wait_checked(command).unwrap();
                prepared.check_status().unwrap();
            }
            match case {
                1 => output.counts = MappedBuffer::zeroed::<u32>(&device, 17).unwrap(),
                3 => output.generation += 1,
                4 => states.generation += 1,
                5 => {
                    states.columns = MappedBuffer::from_slice(
                        &device,
                        &words(&states.columns, states.capacity * POSITIONED_COLUMNS),
                    )
                    .unwrap()
                }
                6 => {
                    states.counts =
                        MappedBuffer::from_slice(&device, &words(&states.counts, 5)).unwrap()
                }
                7 => values.counts = MappedBuffer::from_slice(&device, &[0u32, 1]).unwrap(),
                _ => {}
            }
            let scope = LaunchScope::enter();
            let before = scope.snapshot().len();
            let command = queue.new_command_buffer();
            let pending = prepared.encode_selection(
                command,
                ReadyInput {
                    resident: &resident,
                    plan: &plan,
                    data: if case == 2 { &foreign_data } else { &data },
                    states: &states,
                    values: &values,
                },
                &output,
            );
            if case == 0 {
                let pending = pending.unwrap();
                commit_checked(command).unwrap();
                wait_checked(command).unwrap();
                assert!(matches!(
                    pending.check_completed(),
                    Err(Error::BackendFailure)
                ));
                assert!(
                    words(&output.indices, output.capacity * 4)
                        .iter()
                        .all(|&v| v == u32::MAX)
                );
                assert!(
                    scope
                        .snapshot()
                        .iter()
                        .all(|r| r.encoded && r.submitted && r.completed)
                );
            } else {
                assert!(matches!(pending, Err(Error::BackendFailure)), "case={case}");
                assert!(
                    scope.snapshot().len() == before,
                    "foreign work must reject before dispatch"
                );
            }
        }
    });
}
