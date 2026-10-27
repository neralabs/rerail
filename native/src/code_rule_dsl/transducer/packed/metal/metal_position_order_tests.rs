//! Ordering is task-local full-path order, independent of physical arrival.
use super::*;

fn check(
    rows: &[State],
    tasks: usize,
    configure: impl FnOnce(&mut Storage, &mut ReadyBatch, &mut OrderedFrontier),
    failure: Option<Error>,
) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let scope = super::super::super::metal_backend::launch::LaunchScope::enter();
    let before = scope.snapshot().len();
    let device = Device::system_default().unwrap();
    let p = closure_program();
    let (resident, plan) = fixture(&p, tasks);
    let data = super::super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut input = upload(&device, rows, &p);
    let mut batch = batch(&device, rows, input.states.capacity);
    let mut output = ordered(&device, input.states.capacity);
    configure(&mut input, &mut batch, &mut output);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let result = metal_position_order::encode(
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
    );
    let actual = match result {
        Ok(pending) => {
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            pending.check_completed()
        }
        Err(e) => {
            assert_eq!(Some(e), failure);
            assert_eq!(read::<u32>(&output.states.counts, 5), vec![u32::MAX; 5]);
            return 0;
        }
    };
    assert_eq!(actual, failure.clone().map_or(Ok(()), Err));
    let headers = read::<u32>(&output.states.counts, 5);
    let columns = read::<u32>(&output.states.columns, output.states.capacity * 13);
    let originals = read::<u32>(&output.originals, output.states.capacity);
    if failure.is_some() {
        assert_eq!(headers, vec![0, 0, 0, 1, 1]);
        assert!(columns.iter().chain(&originals).all(|&v| v == u32::MAX));
        return 0;
    }
    let mut ids = (0..rows.len()).collect::<Vec<_>>();
    ids.sort_by_key(|&i| (rows[i].task, &rows[i].position));
    let mut ranks = BTreeMap::<u32, u32>::new();
    let selected = ids
        .into_iter()
        .filter_map(|i| {
            let rank = ranks.entry(rows[i].task).or_default();
            let entry = (rows[i].kind == 0).then_some((i, *rank));
            *rank += 1;
            entry
        })
        .collect::<Vec<_>>();
    let m = selected.len();
    assert_eq!(
        headers,
        vec![
            m as u32,
            rows.iter().map(|s| s.captures.len() as u32).sum(),
            m.div_ceil(256) as u32,
            1,
            1
        ]
    );
    let input_columns = read::<u32>(&input.states.columns, rows.len() * 12);
    for (at, &(original, rank)) in selected.iter().enumerate() {
        assert_eq!(originals[at], original as u32);
        for k in 0..12 {
            assert_eq!(
                columns[k * m + at],
                input_columns[k * rows.len() + original]
            );
        }
        assert_eq!(columns[12 * m + at], rank);
    }
    assert!(
        columns[m * 13..]
            .iter()
            .chain(&originals[m..])
            .all(|&v| v == u32::MAX)
    );
    scope.snapshot()[before..]
        .iter()
        .filter(|record| {
            matches!(
                record.kernel.as_str(),
                "position_merge" | "position_local_sort"
            )
        })
        .count()
}

#[test]
fn local_position_tiles_preserve_complete_long_paths_and_original_rows_with_fewer_dispatches() {
    let template = rows()[0].clone();
    let mut measured = Vec::new();
    for count in [17usize, 257, 1025, 4097] {
        let rows = (0..count)
            .map(|i| {
                let mut row = template.clone();
                row.task = (i % 2) as u32;
                row.position = vec![7; 1025];
                row.position.push((count - i) as u32);
                row
            })
            .collect::<Vec<_>>();
        let stages = check(&rows, 2, |_, _, _| {}, None);
        let maximum = (count + 7).next_power_of_two().ilog2() as usize - 3;
        eprintln!("POSITION_TILE_EVIDENCE rows={count} orderDispatches={stages}");
        measured.push((count, stages, maximum));
    }
    assert!(
        measured.iter().all(|&(_, count, maximum)| count <= maximum),
        "local tiles must preserve complete order with three fewer dispatches: {measured:?}"
    );
}

#[test]
fn ordering_uses_full_paths_prefix_lengths_and_independent_task_ranks() {
    let mut rows = rows();
    let paths = [vec![7; 1025], vec![], vec![7; 1024], vec![7; 1026]];
    for (s, p) in rows.iter_mut().zip(paths) {
        s.position = p;
    }
    let mut second = rows.clone();
    for s in &mut second {
        s.task = 1;
    }
    rows.extend(second);
    for _ in 0..rows.len() {
        rows.rotate_left(1);
        check(&rows, 2, |_, _, _| {}, None);
    }
}

#[test]
fn ordering_accepts_empty_zero_capacity_and_terminal_only_frontiers() {
    check(&[], 1, |_, _, _| {}, None);
    check(
        &[],
        1,
        |input, batch, output| {
            input.states.capacity = 0;
            input.control.state_capacity = 0;
            batch.capacity = 0;
            output.states.capacity = 0;
        },
        None,
    );
    check(&[rows()[1].clone()], 1, |_, _, _| {}, None);
}

#[test]
fn ordering_rejects_duplicate_positions_and_invalid_values_without_payloads() {
    let mut duplicate = rows();
    duplicate[0].position = duplicate[2].position.clone();
    check(&duplicate, 1, |_, _, _| {}, Some(Error::BackendFailure));
    check(
        &rows(),
        1,
        |input, _, _| {
            write(&input.control.spans, &[u32::MAX]);
        },
        Some(Error::BackendFailure),
    );
    check(
        &rows(),
        1,
        |input, _, _| {
            write(&input.values.captures, &[[1u32, u32::MAX, 0, 7]]);
        },
        Some(Error::BackendFailure),
    );
    check(
        &rows(),
        1,
        |_, _, output| output.states.capacity = 2,
        Some(Error::BackendCapacity),
    );
}

#[test]
fn ordering_rejects_stale_generations_and_output_aliases_before_dispatch() {
    check(
        &rows(),
        1,
        |_, _, out| out.states.generation = 7,
        Some(Error::BackendFailure),
    );
    check(
        &rows(),
        1,
        |input, _, _| input.control.generation = 7,
        Some(Error::BackendFailure),
    );
    check(
        &rows(),
        1,
        |_, batch, _| batch.generation = 7,
        Some(Error::BackendFailure),
    );
    check(
        &rows(),
        1,
        |input, _, output| {
            output.originals = MappedBuffer {
                buffer: input.states.columns.buffer.to_owned(),
            };
        },
        Some(Error::BackendFailure),
    );
}

#[test]
fn ordering_rejects_a_partial_ready_header_batch() {
    check(
        &rows(),
        1,
        |_, batch, _| {
            let cap = batch.capacity;
            let mut ids = vec![u32::MAX; cap * 4];
            ids[cap] = 0;
            ids[cap + 1] = 2;
            ids[cap * 2] = 3;
            ids[cap * 3] = 1;
            write(&batch.indices, &ids);
            write(
                &batch.counts,
                &[0u32, 2, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
            );
        },
        Some(Error::BackendFailure),
    );
}
