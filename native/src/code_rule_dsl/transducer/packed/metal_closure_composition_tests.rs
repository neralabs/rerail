//! Serial first-visit oracle and GPU-only handoff through readiness.
use super::super::super::metal_ready_batch::{self, ReadyInput};
use super::*;

fn expected(rows: &[State], resident: &ResidentInput) -> Vec<State> {
    let mut ordered = (0..rows.len()).collect::<Vec<_>>();
    ordered.sort_by_key(|&i| (rows[i].task, &rows[i].position));
    let mut visited = BTreeSet::new();
    let mut survivors = BTreeMap::new();
    for i in ordered {
        let row = &rows[i];
        if row.kind == 1 {
            survivors.insert(i, row.clone());
            continue;
        }
        let mut node = row.node;
        let mut final_node = None;
        while node != u32::MAX {
            let key = (
                row.task,
                row.pc,
                node,
                row.cursors.clone(),
                row.repeats.clone(),
                row.captures.clone(),
            );
            if !visited.insert(key) {
                break;
            }
            final_node = Some(node);
            node = resident.input.packed.nodes[node as usize].next_any;
        }
        if let Some(node) = final_node {
            let mut result = row.clone();
            result.node = node;
            result.pc = 6;
            result.position.push(1);
            survivors.insert(i, result);
        }
    }
    survivors.into_values().collect()
}

fn compose(rows: &[State], tasks: usize, advance: bool) -> Vec<State> {
    let p = closure_program();
    let fixture = fixture_sources(&p, tasks, &["# first\n# second\n# third\n".to_owned()]);
    compose_resident(rows, tasks, advance, fixture)
}

fn compose_resident(
    rows: &[State],
    tasks: usize,
    advance: bool,
    fixture: (ResidentInput, ExecutionPlan),
) -> Vec<State> {
    compose_bounded(rows, tasks, advance, fixture, false)
}

fn compose_bounded(
    rows: &[State],
    tasks: usize,
    advance: bool,
    fixture: (ResidentInput, ExecutionPlan),
    exhaust_exits: bool,
) -> Vec<State> {
    let _guard = metal_execution_guard().unwrap();
    let scope = super::super::super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let p = closure_program();
    let (resident, plan) = fixture;
    let data = super::super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, rows, &p);
    let batch = batch(&device, rows, input.states.capacity);
    let ranked = ordered(&device, input.states.capacity);
    let cells = rows.len() * 3;
    let ranges = PreparedRanges {
        ranges: ClosureRanges {
            domains: MappedBuffer::zeroed::<[u32; 2]>(&device, rows.len()).unwrap(),
            entries: MappedBuffer::zeroed::<[u32; 2]>(&device, rows.len()).unwrap(),
            allowed: MappedBuffer::zeroed::<u32>(&device, cells).unwrap(),
            counts: MappedBuffer::zeroed::<u32>(&device, 3).unwrap(),
            domain_capacity: rows.len(),
            entry_capacity: rows.len(),
            cell_capacity: cells,
            generation: 8,
        },
        nodes: MappedBuffer::zeroed::<u32>(&device, cells).unwrap(),
    };
    let exits = ClosureExits {
        exits: MappedBuffer::zeroed::<[u32; 2]>(&device, rows.len()).unwrap(),
        counts: MappedBuffer::zeroed::<u32>(&device, 4).unwrap(),
        capacity: if exhaust_exits { 0 } else { rows.len() },
        generation: 8,
    };
    let output = storage(
        &device,
        rows.len() + 1,
        input.control.digit_capacity + rows.len(),
        9,
    );
    let ready = ReadyBatch {
        indices: MappedBuffer::from_slice(&device, &vec![u32::MAX; output.states.capacity * 4])
            .unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 17]).unwrap(),
        capacity: output.states.capacity,
        generation: 9,
    };
    let make_input = || SuccessorInput {
        resident: &resident,
        plan: &plan,
        data: &data,
        states: &input.states,
        values: &input.values,
        control: &input.control,
        ready: &batch,
    };
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let reconstructed = metal_closure_continuation::encode_composed(
        &device,
        command,
        &make_input(),
        ClosureResult {
            ordered: &ranked,
            ranges: &ranges,
            exits: &exits,
        },
        SuccessorOutput {
            states: &output.states,
            values: &output.values,
            control: &output.control,
        },
    )
    .unwrap();
    let scheduled = metal_ready_batch::encode(
        &device,
        command,
        ReadyInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &output.states,
            values: &output.values,
        },
        &ready,
    )
    .unwrap();
    let first_submission = std::time::Instant::now();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let first_submission_seconds = first_submission.elapsed().as_secs_f64();
    if exhaust_exits {
        assert_eq!(
            reconstructed.check_completed(),
            Err(Error::BackendCapacity),
            "closure overflow must invalidate reconstruction, including unrelated terminals"
        );
        scheduled.check_completed().unwrap();
        assert_eq!(read::<u32>(&output.states.counts, 5), [0, 0, 0, 1, 1]);
        assert_eq!(read::<u32>(&output.values.counts, 2), [0, 0]);
        assert_eq!(read::<u32>(&output.control.count, 1), [0]);
        assert_eq!(&read::<u32>(&ready.counts, 16)[..4], [0, 0, 0, 0]);
        assert_eq!(observe(&output, &p), Vec::<State>::new());
        return vec![];
    }
    reconstructed.check_completed().unwrap();
    scheduled.check_completed().unwrap();
    let expected = expected(rows, &resident);
    assert_eq!(observe(&output, &p), expected);
    let counts = read::<u32>(&ready.counts, 16);
    let terminals = expected.iter().filter(|s| s.kind == 1).count();
    assert_eq!(
        &counts[..4],
        &[(expected.len() - terminals) as u32, 0, 0, terminals as u32]
    );
    let indices = read::<u32>(&ready.indices, ready.capacity * 4);
    for (list, kind) in [(0, 0), (3, 1)] {
        let ids = expected
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (s.kind == kind).then_some(i as u32))
            .collect::<Vec<_>>();
        assert_eq!(
            &indices[list * ready.capacity..list * ready.capacity + ids.len()],
            ids
        );
        assert!(
            indices[list * ready.capacity + ids.len()..(list + 1) * ready.capacity]
                .iter()
                .all(|&i| i == u32::MAX)
        );
    }
    let trace = scope.snapshot();
    eprintln!(
        "CLOSURE_COMPOSITION {}",
        serde_json::json!({"tasks": tasks,"states": rows.len(),"launches": trace.len(),"peakMetalBytes": trace.iter().map(|r|r.peak_application_buffer_bytes).max().unwrap_or(0)})
    );
    assert!(trace.iter().all(|r| r.completed));
    if !advance {
        return expected;
    }
    let next = storage(
        &device,
        output.states.capacity,
        output.control.digit_capacity * 2,
        10,
    );
    let command = queue.new_command_buffer();
    let advanced = metal_successor::encode(
        &device,
        &queue,
        command,
        SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &output.states,
            values: &output.values,
            control: &output.control,
            ready: &ready,
        },
        SuccessorOutput {
            states: &next.states,
            values: &next.values,
            control: &next.control,
        },
    )
    .unwrap();
    let successor_submission = std::time::Instant::now();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let successor_submission_seconds = successor_submission.elapsed().as_secs_f64();
    advanced.check_completed().unwrap();
    let mut advanced_expected = expected;
    for s in &mut advanced_expected {
        if s.kind == 0 {
            s.pc = 7;
            s.kind = 1;
            assert_eq!(s.repeats.pop(), Some([1, 0, 0, 0]));
        }
    }
    assert_eq!(observe(&next, &p), advanced_expected);
    let trace = scope.snapshot();
    assert!(trace.iter().all(|r| r.completed));
    eprintln!(
        "CLOSURE_HANDOFF {}",
        serde_json::json!({
            "tasks":tasks,"states":rows.len(),"launches":trace.len(),
            "peakMetalBytes":trace.iter().map(|r|r.peak_application_buffer_bytes).max().unwrap_or(0),
            "closureSubmissionSeconds":first_submission_seconds,
            "successorSubmissionSeconds":successor_submission_seconds,
            "kernels":trace.iter().map(|r|&r.kernel).collect::<Vec<_>>()
        })
    );
    advanced_expected
}

fn independent_tasks(tasks: usize) -> (Vec<State>, (ResidentInput, ExecutionPlan)) {
    let source = Arc::new("# first\n# second\n# third\n".to_owned());
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let doc = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let p = closure_program();
    let resident = ResidentInput::new(
        MetalGridInput::build(
            std::slice::from_ref(&p),
            &vec![doc; tasks],
            &vec![vec![vec![1]]; tasks],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, std::slice::from_ref(&p)).unwrap();
    let mut all = Vec::new();
    for (task, file) in resident.input.packed.files.iter().enumerate() {
        for mut row in rows() {
            row.task = task as u32;
            row.node += file.node_start;
            for cell in &mut row.cursors {
                cell[1] += file.node_start;
            }
            for cell in &mut row.captures {
                cell[0] += file.node_start;
                cell[2] += file.source_start;
                cell[3] += file.source_start;
            }
            all.push(row);
        }
    }
    all.reverse();
    (all, (resident, plan))
}

#[test]
fn closure_handoff_scales_across_128_independent_tasks() {
    let (rows, fixture) = independent_tasks(128);
    compose_resident(&rows, 128, true, fixture);
}

#[test]
fn closure_handoff_scales_across_4096_independent_tasks() {
    let (rows, fixture) = independent_tasks(4096);
    compose_resident(&rows, 4096, true, fixture);
}

#[test]
fn complete_validation_refuses_reuse_for_unvalidated_frontiers_and_commands() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let p = closure_program();
    let rows = rows();
    let (resident, plan) = fixture_sources(&p, 1, &["# first\n# second\n# third\n".into()]);
    let (other_resident, other_plan) =
        fixture_sources(&p, 1, &["# first\n# second\n# third\n".into()]);
    let equivalent_plan = ExecutionPlan::new(&resident, std::slice::from_ref(&p)).unwrap();
    let mut data =
        super::super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut other_data =
        super::super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut original = upload(&device, &rows, &p);
    let mut foreign = upload(&device, &rows, &p);
    let mut original_ready = batch(&device, &rows, original.states.capacity);
    let mut foreign_ready = batch(&device, &rows, foreign.states.capacity);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    macro_rules! input {
        () => {
            SuccessorInput {
                resident: &resident,
                plan: &plan,
                data: &data,
                states: &original.states,
                values: &original.values,
                control: &original.control,
                ready: &original_ready,
            }
        };
    }
    let checked =
        super::super::super::metal_frontier_validation::encode(&device, command, &input!())
            .unwrap();
    macro_rules! reject {
        ($input:expr, $command:expr, $reason:expr) => {
            assert!(
                matches!(
                    checked.validate_binding(&$input, $command),
                    Err(Error::BackendFailure)
                ),
                "{}",
                $reason
            );
        };
    }
    macro_rules! replace {
        ($original:expr, $foreign:expr) => {
            std::mem::swap(&mut $original, &mut $foreign);
            reject!(input!(), command, stringify!($original));
            std::mem::swap(&mut $original, &mut $foreign);
            checked.validate_binding(&input!(), command).unwrap();
        };
    }
    // Identical bytes in a different allocation do not carry a validation proof.
    replace!(original.states.columns, foreign.states.columns);
    replace!(original.states.counts, foreign.states.counts);
    replace!(original.values.cursors, foreign.values.cursors);
    replace!(original.values.repeats, foreign.values.repeats);
    replace!(original.values.captures, foreign.values.captures);
    replace!(original.values.counts, foreign.values.counts);
    replace!(original.control.spans, foreign.control.spans);
    replace!(original.control.digits, foreign.control.digits);
    replace!(original.control.count, foreign.control.count);
    replace!(original_ready.indices, foreign_ready.indices);
    replace!(original_ready.counts, foreign_ready.counts);
    replace!(data, other_data);
    macro_rules! resize {
        ($capacity:expr) => {
            $capacity -= 1;
            reject!(input!(), command, stringify!($capacity));
            $capacity += 1;
        };
    }
    resize!(original.values.cursor_capacity);
    resize!(original.values.repeat_capacity);
    resize!(original.values.capture_capacity);
    resize!(original.control.digit_capacity);
    original.states.capacity -= 1;
    original.control.state_capacity -= 1;
    original_ready.capacity -= 1;
    reject!(
        input!(),
        command,
        "same storage with different frontier bounds"
    );
    original.states.capacity += 1;
    original.control.state_capacity += 1;
    original_ready.capacity += 1;
    original.states.generation += 1;
    original.values.generation += 1;
    original.control.generation += 1;
    original_ready.generation += 1;
    reject!(
        input!(),
        command,
        "coherently retagged but unvalidated generation"
    );
    original.states.generation -= 1;
    original.values.generation -= 1;
    original.control.generation -= 1;
    original_ready.generation -= 1;
    reject!(
        SuccessorInput {
            plan: &equivalent_plan,
            ..input!()
        },
        command,
        "another proof owner"
    );
    reject!(
        SuccessorInput {
            resident: &other_resident,
            plan: &other_plan,
            ..input!()
        },
        command,
        "another resident owner"
    );
    reject!(
        input!(),
        queue.new_command_buffer(),
        "an unsubmitted command is not another command's dependency"
    );
    checked.validate_binding(&input!(), command).unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    checked.check_completed().unwrap();
    reject!(
        input!(),
        command,
        "a submitted command cannot accept more consumers"
    );
    reject!(
        input!(),
        queue.new_command_buffer(),
        "completion does not prove shared inputs stayed immutable"
    );
}

#[test]
fn closure_composition_matches_serial_first_visit_for_all_arrival_permutations() {
    let base = rows();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    if order.into_iter().collect::<BTreeSet<_>>().len() != 4 {
                        continue;
                    }
                    let rows = order.map(|i| base[i].clone());
                    compose(&rows, 1, false);
                }
            }
        }
    }
}

#[test]
fn closure_continuation_reenters_readiness_and_executes_repeat_end_under_one_guard() {
    compose(&rows(), 1, true);
}

#[test]
fn closure_composition_keeps_distinct_full_value_domains_and_long_positions() {
    let mut rows = rows();
    rows[0].cursors.push([2, 2, 0, 0]);
    rows[2].captures.push([1, 0, 0, 7]);
    for row in &mut rows {
        let digit = row.position[0];
        row.position = vec![0; 1025];
        row.position.push(digit);
    }
    compose(&rows, 1, false);
}

#[test]
fn closure_failure_cannot_publish_a_partial_frontier_or_ready_batch() {
    let p = closure_program();
    let fixture = fixture_sources(&p, 1, &["# first\n# second\n# third\n".into()]);
    compose_bounded(&rows(), 1, false, fixture, true);
}
