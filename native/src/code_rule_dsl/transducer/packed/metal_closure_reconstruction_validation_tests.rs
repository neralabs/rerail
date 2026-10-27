//! Reject inconsistent producer data before publishing a usable continuation.
use super::*;

fn check(
    configure: impl FnOnce(&mut OrderedFrontier, &mut PreparedRanges, &mut ClosureExits, &mut Storage),
    failure: Option<Error>,
) {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let p = closure_program();
    let rows = rows();
    let (resident, plan) = fixture(&p, 1);
    let data = super::super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let input = upload(&device, &rows, &p);
    let batch = batch(&device, &rows, input.states.capacity);
    let mut ranked = ordered(&device, 4);
    let source = read::<u32>(&input.states.columns, 48);
    let mut columns = vec![0; 39];
    for (at, original) in [3, 2, 0].into_iter().enumerate() {
        for k in 0..12 {
            columns[k * 3 + at] = source[k * 4 + original];
        }
        columns[36 + at] = at as u32 + 1;
    }
    write(&ranked.states.columns, &columns);
    write(&ranked.states.counts, &[3u32, 12, 1, 1, 1]);
    write(&ranked.originals, &[3u32, 2, 0]);
    let mut ranges = PreparedRanges {
        ranges: ClosureRanges {
            domains: MappedBuffer::from_slice(&device, &[[0u32, 3]]).unwrap(),
            entries: MappedBuffer::from_slice(&device, &[[0u32, 1], [0, 2], [0, 0]]).unwrap(),
            allowed: MappedBuffer::from_slice(&device, &[1u32, 1, 1]).unwrap(),
            counts: MappedBuffer::from_slice(&device, &[1u32, 3, 3]).unwrap(),
            domain_capacity: 1,
            entry_capacity: 3,
            cell_capacity: 3,
            generation: 8,
        },
        nodes: MappedBuffer::from_slice(&device, &[1u32, 2, 3]).unwrap(),
    };
    let mut exits = ClosureExits {
        exits: MappedBuffer::from_slice(&device, &[[0u32, 2], [2, 0]]).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[2u32, 1, 1, 1]).unwrap(),
        capacity: 2,
        generation: 8,
    };
    let mut output = storage(&device, 8, 128, 9);
    configure(&mut ranked, &mut ranges, &mut exits, &mut output);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let result = metal_closure_continuation::encode(
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
    );
    let actual = match result {
        Ok(pending) => {
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            pending.check_completed()
        }
        Err(error) => {
            assert_eq!(Some(error), failure);
            assert_eq!(read::<u32>(&output.states.counts, 5), vec![u32::MAX; 5]);
            return;
        }
    };
    assert_eq!(actual, failure.clone().map_or(Ok(()), Err));
    if failure.is_none() {
        let mut expected = vec![rows[0].clone(), rows[1].clone(), rows[3].clone()];
        for i in [0, 2] {
            expected[i].pc = 6;
            expected[i].position.push(1);
        }
        expected[2].node = 3;
        assert_eq!(observe(&output, &p), expected);
        return;
    }
    assert_eq!(read::<u32>(&output.states.counts, 5), vec![0, 0, 0, 1, 1]);
    assert_eq!(read::<u32>(&output.values.counts, 2), vec![0, 0]);
    assert_eq!(read::<u32>(&output.control.count, 1), vec![0]);
    // Inspect every poisoned cell, including cells beyond a reduced logical
    // capacity. MappedBuffer's page padding was never part of the fixture.
    for (b, cells) in [
        (&output.states.columns, 8 * COLUMNS),
        (&output.values.cursors, 128 * 4),
        (&output.values.repeats, 128 * 4),
        (&output.values.captures, 128 * 4),
        (&output.control.spans, 8 * 6),
        (&output.control.digits, 128),
    ] {
        assert!(read::<u32>(b, cells).iter().all(|&v| v == u32::MAX));
    }
}

#[test]
fn reconstructed_values_match_an_independent_packed_closure_result() {
    check(|_, _, _, _| {}, None);
}

#[test]
fn reconstruction_rejects_invalid_original_entry_and_exit_mappings() {
    check(
        |o, _, _, _| write(&o.originals, &[u32::MAX]),
        Some(Error::BackendFailure),
    );
    check(
        |o, _, _, _| write(&o.originals, &[3u32, 3, 0]),
        Some(Error::BackendFailure),
    );
    check(
        |o, _, _, _| {
            let mut c = read::<u32>(&o.states.columns, 39);
            c[6] = 1;
            write(&o.states.columns, &c);
        },
        Some(Error::BackendFailure),
    );
    check(
        |o, _, _, _| {
            let mut c = read::<u32>(&o.states.columns, 39);
            c[37] = 0;
            write(&o.states.columns, &c);
        },
        Some(Error::BackendFailure),
    );
    check(
        |_, _, e, _| write(&e.exits, &[[3u32, 2]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, _, e, _| write(&e.exits, &[[2u32, 0], [0, 2]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, _, e, _| write(&e.exits, &[[0u32, 2], [0, 2]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, _, e, _| write(&e.exits, &[[0u32, 0]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, _, e, _| write(&e.exits, &[[0u32, 3]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, r, _, _| write(&r.ranges.entries, &[[1u32, 1]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, r, _, _| write(&r.ranges.domains, &[[2u32, 3]]),
        Some(Error::BackendFailure),
    );
    check(
        |_, r, _, _| write(&r.nodes, &[1u32, 2, u32::MAX]),
        Some(Error::BackendFailure),
    );
}

#[test]
fn reconstruction_rejects_a_valid_node_at_the_wrong_resident_cell() {
    check(
        |_, r, _, _| write(&r.nodes, &[1u32, 2, 1]),
        Some(Error::BackendFailure),
    );
}

#[test]
fn reconstruction_rejects_capacity_and_generation_mismatches() {
    check(
        |_, _, _, o| o.states.capacity = 1,
        Some(Error::BackendFailure),
    );
    check(
        |_, _, _, o| {
            o.states.capacity = 1;
            o.control.state_capacity = 1;
        },
        Some(Error::BackendCapacity),
    );
    check(
        |_, _, _, o| o.control.digit_capacity = 1,
        Some(Error::BackendCapacity),
    );
    check(
        |_, _, _, o| o.values.capture_capacity = 11,
        Some(Error::BackendCapacity),
    );
    check(
        |_, _, _, o| o.values.cursor_capacity = 3,
        Some(Error::BackendCapacity),
    );
    check(
        |_, _, _, o| o.values.repeat_capacity = 7,
        Some(Error::BackendCapacity),
    );
    check(
        |o, _, _, _| o.states.generation = 7,
        Some(Error::BackendFailure),
    );
    check(
        |_, r, _, _| r.ranges.generation = 7,
        Some(Error::BackendFailure),
    );
    check(|_, _, e, _| e.generation = 7, Some(Error::BackendFailure));
    check(
        |_, _, _, o| o.control.generation = 8,
        Some(Error::BackendFailure),
    );
    check(
        |o, _, _, s| {
            o.originals = MappedBuffer {
                buffer: s.control.digits.buffer.to_owned(),
            }
        },
        Some(Error::BackendFailure),
    );
}
