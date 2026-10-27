//! Complete-value oracles for the positioned -> ranked -> positioned boundary.
use super::super::metal_bulk_closure::{self, ClosureExits, ClosureRanges};
use super::super::metal_closure_continuation::{self, ClosureResult};
use super::super::metal_closure_prepare::{self, ClosureInput, PreparedRanges};
use super::super::metal_position_order::{self, OrderedFrontier};
use super::super::metal_state_values::ValueFrontier;
use super::*;

#[path = "metal_closure_composition_tests.rs"]
mod composition;
#[path = "metal_position_order_tests.rs"]
mod ordering_cases;
#[path = "metal_closure_reconstruction_validation_tests.rs"]
mod reconstruction_cases;

pub(super) fn closure_program() -> Program {
    program(vec![
        word(REPEAT_BEGIN, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(FORK, GREEDY, 6),
        word(NEXT, 0, 0),
        word(REPEAT_BUMP, 0, 0),
        word(GOTO, 0, 2),
        word(REPEAT_END, 0, 0),
        word(EMIT, 0, 0),
    ])
    .with_tree_sitter_query_semantics()
    .unwrap()
}
fn rows() -> Vec<State> {
    [(1, 4), (3, 0), (3, 3), (2, 1)]
        .into_iter()
        .map(|(node, digit)| {
            let mut s = state(
                if digit == 0 { 7 } else { 2 },
                if digit == 0 { 1 } else { 0 },
            );
            s.node = node;
            s.position = vec![digit];
            s.cursors = vec![[1, 1, 0, 0]];
            s.repeats = vec![[0, 9, 0, 0], [1, 0, 0, 0]];
            s.captures = vec![[1, 0, 0, 7]; 3];
            s
        })
        .collect()
}
fn batch(device: &Device, rows: &[State], capacity: usize) -> ReadyBatch {
    let mut ids = vec![u32::MAX; capacity * 4];
    let mut counts = [0u32; 17];
    for (i, row) in rows.iter().enumerate() {
        let list = if row.kind == 1 { 3 } else { 1 };
        ids[list * capacity + counts[list] as usize] = i as u32;
        counts[list] += 1;
    }
    for k in 0..4 {
        counts[4 + k * 3] = counts[k].div_ceil(256);
        counts[5 + k * 3] = 1;
        counts[6 + k * 3] = 1;
    }
    ReadyBatch {
        indices: MappedBuffer::from_slice(device, &ids).unwrap(),
        counts: MappedBuffer::from_slice(device, &counts).unwrap(),
        capacity,
        generation: 8,
    }
}
fn ordered(device: &Device, capacity: usize) -> OrderedFrontier {
    OrderedFrontier {
        states: ValueFrontier {
            columns: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity * 13]).unwrap(),
            counts: MappedBuffer::from_slice(device, &[u32::MAX; 5]).unwrap(),
            capacity,
            generation: 8,
        },
        originals: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity]).unwrap(),
    }
}
#[test]
fn explicit_ordering_preserves_full_values_original_entries_and_rank_gaps() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let p = closure_program();
    let rows = rows();
    let (resident, plan) = fixture(&p, 1);
    let data = super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, &rows, &p);
    let batch = batch(&device, &rows, input.states.capacity);
    let output = ordered(&device, input.states.capacity);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending = metal_position_order::encode(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &batch,
        },
        &output,
    )
    .expect("full positions must convert to logically ordered closure entries");
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    assert_eq!(read::<u32>(&output.states.counts, 5), vec![3, 12, 1, 1, 1]);
    assert_eq!(
        &read::<u32>(&output.originals, output.states.capacity)[..3],
        &[3, 2, 0]
    );
    let actual = read::<u32>(&output.states.columns, output.states.capacity * 13);
    let original = read::<u32>(&input.states.columns, rows.len() * 12);
    for (rank, source) in [3usize, 2, 0].into_iter().enumerate() {
        for field in 0..12 {
            assert_eq!(
                actual[field * 3 + rank],
                original[field * 4 + source],
                "field={field}, rank={rank}"
            );
        }
        assert_eq!(actual[12 * 3 + rank], rank as u32 + 1);
    }
    assert!(actual[39..].iter().all(|&v| v == u32::MAX));
}

#[test]
fn closure_continuations_restore_original_values_and_keep_repeat_end_unexecuted() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let p = closure_program();
    let rows = rows();
    let (resident, plan) = fixture(&p, 1);
    let data = super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, &rows, &p);
    let batch = batch(&device, &rows, input.states.capacity);
    let ranked = ordered(&device, input.states.capacity);
    // An independent ranked producer isolates the reconstruction red from ordering.
    let original = read::<u32>(&input.states.columns, rows.len() * 12);
    let mut columns = vec![0; 39];
    for (rank, source) in [3usize, 2, 0].into_iter().enumerate() {
        for field in 0..12 {
            columns[field * 3 + rank] = original[field * 4 + source];
        }
        columns[36 + rank] = rank as u32 + 1;
    }
    write(&ranked.states.columns, &columns);
    write(&ranked.states.counts, &[3u32, 12, 1, 1, 1]);
    write(&ranked.originals, &[3u32, 2, 0]);
    let ranges = PreparedRanges {
        ranges: ClosureRanges {
            domains: MappedBuffer::zeroed::<[u32; 2]>(&device, 4).unwrap(),
            entries: MappedBuffer::zeroed::<[u32; 2]>(&device, 4).unwrap(),
            allowed: MappedBuffer::zeroed::<u32>(&device, 16).unwrap(),
            counts: MappedBuffer::zeroed::<u32>(&device, 3).unwrap(),
            domain_capacity: 4,
            entry_capacity: 4,
            cell_capacity: 16,
            generation: 8,
        },
        nodes: MappedBuffer::zeroed::<u32>(&device, 16).unwrap(),
    };
    let exits = ClosureExits {
        exits: MappedBuffer::zeroed::<[u32; 2]>(&device, 4).unwrap(),
        counts: MappedBuffer::zeroed::<u32>(&device, 4).unwrap(),
        capacity: 4,
        generation: 8,
    };
    let output = storage(&device, 8, 128, 9);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let prepared = metal_closure_prepare::encode(
        &device,
        command,
        ClosureInput {
            resident: &resident,
            programs: std::slice::from_ref(&p),
            data: &data,
            states: &ranked.states,
            values: &input.values,
        },
        &ranges,
    )
    .unwrap();
    let closed = metal_bulk_closure::encode(&device, command, &ranges.ranges, &exits).unwrap();
    let pending = metal_closure_continuation::encode(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &batch,
        },
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
    .expect("closure exits must reconstruct the original complete values");
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    prepared.check_completed().unwrap();
    closed.check_completed().unwrap();
    pending.check_completed().unwrap();
    let mut expected = vec![rows[0].clone(), rows[1].clone(), rows[3].clone()];
    for index in [0, 2] {
        expected[index].pc = 6;
        expected[index].position.push(1);
    }
    expected[2].node = 3;
    assert_eq!(observe(&output, &p), expected);
}
