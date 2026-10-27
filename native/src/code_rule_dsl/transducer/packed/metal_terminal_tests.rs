//! Behavioral terminal boundaries: complete identity, raw numbering, containment, ABI.
use super::super::metal_batch_abi::{FinalCaptureRecord, ResultRecord};
use super::super::metal_terminal::{self, TerminalOutput};
use super::*;

fn query() -> Program {
    let p = closure_tests::closure_program();
    let mut words = p.words.clone();
    words[7] = word(EMIT, 0, 1);
    Program::new(
        words,
        vec![b"first".to_vec(), b"second".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap()
    .with_tree_sitter_query_semantics()
    .unwrap()
}
fn capture(resident: &ResidentInput, node: u32, label: u32) -> [u32; 4] {
    let n = resident.input.packed.nodes[node as usize];
    [node, label, n.source_start, n.source_end]
}
fn batch(device: &Device, rows: &[State], capacity: usize) -> ReadyBatch {
    let mut ids = vec![u32::MAX; capacity * 4];
    let mut counts = [0u32; 17];
    for (i, s) in rows.iter().enumerate() {
        let list = match s.kind {
            1 | 3 => 3,
            6 | 7 => 2,
            _ => 0,
        };
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
fn output(device: &Device, capacity: usize, captures: usize) -> TerminalOutput {
    TerminalOutput {
        results: MappedBuffer::from_slice(device, &vec![[u32::MAX; 10]; capacity]).unwrap(),
        captures: MappedBuffer::from_slice(device, &vec![[u32::MAX; 9]; captures]).unwrap(),
        counts: MappedBuffer::from_slice(device, &[u32::MAX; 2]).unwrap(),
        capacity,
        capture_capacity: captures,
        generation: 8,
    }
}
fn attempt(
    p: &Program,
    rows: &[State],
    tasks: usize,
    configure: impl FnOnce(&mut Storage, &mut ReadyBatch, &mut TerminalOutput),
) -> Result<(Vec<ResultRecord>, Vec<FinalCaptureRecord>), Error> {
    attempt_resident(p, rows, fixture(p, tasks), configure)
}
fn attempt_resident(
    p: &Program,
    rows: &[State],
    fixture: (ResidentInput, ExecutionPlan),
    configure: impl FnOnce(&mut Storage, &mut ReadyBatch, &mut TerminalOutput),
) -> Result<(Vec<ResultRecord>, Vec<FinalCaptureRecord>), Error> {
    let _guard = metal_execution_guard().unwrap();
    let scope = super::super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let (resident, plan) = fixture;
    let data = super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut input = upload(&device, rows, p);
    let mut ready = batch(&device, rows, input.states.capacity);
    let mut out = output(&device, rows.len() + 7, input.values.capture_capacity + 7);
    configure(&mut input, &mut ready, &mut out);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let started = std::time::Instant::now();
    let pending = metal_terminal::encode(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &ready,
        },
        &out,
    )?;
    commit_checked(command)?;
    wait_checked(command)?;
    let elapsed = started.elapsed();
    let trace = scope.snapshot();
    eprintln!(
        "terminal records={} launches={} peak_bytes={} wall_ms={:.3}",
        rows.len(),
        trace.len(),
        trace
            .iter()
            .map(|r| r.peak_application_buffer_bytes)
            .max()
            .unwrap_or(0),
        elapsed.as_secs_f64() * 1000.0
    );
    let result = pending.check_completed();
    let counts = read::<u32>(&out.counts, 2);
    if result.is_err() {
        assert_eq!(
            counts,
            [0, 0],
            "failed producers cannot publish a permitted prefix"
        );
    }
    let n = counts[0] as usize;
    let c = counts[1] as usize;
    assert!(n <= out.capacity && c <= out.capture_capacity);
    let records = read::<[u32; 10]>(&out.results, out.capacity);
    let captures = read::<[u32; 9]>(&out.captures, out.capture_capacity);
    assert!(records[n..].iter().all(|&r| r == [u32::MAX; 10]));
    assert!(captures[c..].iter().all(|&r| r == [u32::MAX; 9]));
    let completion = result?;
    assert!(completion.errors.iter().all(Option::is_none));
    let results = read::<ResultRecord>(&out.results, n);
    let captures = read::<FinalCaptureRecord>(&out.captures, c);
    // Exercise the shared public storage boundary, including its strict ordering contract.
    super::super::packed_batch::PackedExecution::from_grid_records(
        resident.input.packed.clone(),
        &resident.input.grid.evaluation_spans,
        results.clone(),
        captures.clone(),
        vec![None; resident.input.grid.evaluation_spans.len()],
    )?
    .into_outcome()?;
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    Ok((results, captures))
}
fn run(p: &Program, rows: &[State], tasks: usize) -> (Vec<ResultRecord>, Vec<FinalCaptureRecord>) {
    attempt(p, rows, tasks, |_, _, _| {})
        .expect("terminal candidates must cross the typed publication boundary")
}
fn expected(
    task: u32,
    ordinal: u32,
    start: u32,
    label: u32,
    offset: u32,
    captures: &[[u32; 4]],
) -> (ResultRecord, Vec<FinalCaptureRecord>) {
    (
        ResultRecord {
            task_ordinal: task,
            local_ordinal: ordinal,
            start,
            label,
            capture_start: offset,
            capture_count: captures.len() as u32,
            ..ResultRecord::default()
        },
        captures
            .iter()
            .map(|&[node, label, start, end]| FinalCaptureRecord {
                label,
                node,
                start,
                end,
                text_start: u32::MAX,
                retained: 1,
                ..FinalCaptureRecord::default()
            })
            .collect(),
    )
}
fn query_rows(p: &Program, tasks: usize) -> Vec<State> {
    let (resident, _) = fixture(p, tasks);
    let a = capture(&resident, 1, 0);
    let b = capture(&resident, 2, 0);
    let c = capture(&resident, 3, 0);
    let d = capture(&resident, 3, 1);
    (0..tasks as u32)
        .flat_map(|task| {
            [
                vec![b],
                vec![a],
                vec![a, c],
                vec![a, c],
                vec![a, c],
                vec![d],
                vec![],
            ]
            .into_iter()
            .enumerate()
            .map(move |(i, captures)| {
                let mut s = state(7, 1);
                s.task = task;
                s.position = vec![i as u32];
                s.captures = captures;
                if i == 4 {
                    s.cursors = vec![[0, 2, 0, 0]];
                }
                s
            })
        })
        .collect()
}
#[test]
fn terminal_admission_precedes_raw_ordinals_containment_and_query_publication_order() {
    let p = query();
    let mut rows = query_rows(&p, 2);
    let (resident, _) = fixture(&p, 2);
    // Arrival order cannot choose the exact-state winner or final query order.
    rows.reverse();
    let (actual, captures) = run(&p, &rows, 2);
    let mut want = vec![];
    let mut cells = vec![];
    for task in 0..2 {
        for (ordinal, ids) in [
            (2, vec![(1, 0), (3, 0)]),
            (0, vec![(2, 0)]),
            (4, vec![(3, 1)]),
        ] {
            let caps = ids
                .into_iter()
                .map(|(n, l)| capture(&resident, n, l))
                .collect::<Vec<_>>();
            let (r, c) = expected(task, ordinal, task + 1, 1, cells.len() as u32, &caps);
            want.push(r);
            cells.extend(c);
        }
    }
    assert_eq!(actual, want);
    assert_eq!(captures, cells);
}
#[test]
fn ordinary_and_nonrepeating_query_terminals_preserve_occurrences_and_empty_matches() {
    for query in [false, true] {
        let mut p = program(vec![word(EMIT, 0, 0)]);
        if query {
            p = p.with_tree_sitter_query_semantics().unwrap();
        }
        let (resident, _) = fixture(&p, 1);
        let cap = capture(&resident, 2, 0);
        let mut rows = vec![];
        for (i, captures) in [vec![cap], vec![], vec![cap]].into_iter().enumerate() {
            let mut s = state(0, 3);
            s.position = vec![i as u32];
            s.captures = captures;
            rows.push(s);
        }
        rows.swap(0, 2);
        let (actual, captures) = run(&p, &rows, 1);
        let order = if query { vec![1, 0, 2] } else { vec![0, 1, 2] };
        let mut want = vec![];
        let mut cells = vec![];
        for i in order {
            let caps = if i == 1 { vec![] } else { vec![cap] };
            let (r, c) = expected(0, i, 1, 0, cells.len() as u32, &caps);
            want.push(r);
            cells.extend(c);
        }
        assert_eq!(actual, want);
        assert_eq!(captures, cells);
    }
}
#[test]
fn terminal_publication_rejects_duplicate_positions_and_incomplete_barriers_atomically() {
    let p = query();
    let rows = query_rows(&p, 1);
    for case in 0..3 {
        let mut bad = rows.clone();
        match case {
            0 => bad[1].position = bad[0].position.clone(),
            1 => {
                bad[1].kind = 0;
                bad[1].pc = 6;
            }
            _ => {
                bad[1].captures[0][2] += 1;
            }
        }
        assert_eq!(
            attempt(&p, &bad, 1, |_, _, _| {}),
            Err(Error::BackendFailure),
            "case={case}"
        );
    }
}
#[test]
fn terminal_publication_rejects_capacity_and_stale_generation() {
    let p = query();
    let rows = query_rows(&p, 1);
    assert_eq!(
        attempt(&p, &rows, 1, |_, _, o| o.capacity = 2),
        Err(Error::BackendCapacity)
    );
    assert_eq!(
        attempt(&p, &rows, 1, |_, _, o| o.capture_capacity = 1),
        Err(Error::BackendCapacity)
    );
    assert_eq!(
        attempt(&p, &rows, 1, |_, _, o| o.generation += 1),
        Err(Error::BackendFailure)
    );
    assert_eq!(
        attempt(&p, &rows, 1, |_, b, _| b.generation += 1),
        Err(Error::BackendFailure)
    );
}
#[test]
fn terminal_publication_handles_empty_frontiers_and_long_shared_position_prefixes() {
    let p = query();
    assert_eq!(run(&p, &[], 1), (vec![], vec![]));
    let mut rows = query_rows(&p, 1);
    let expected = run(&p, &rows, 1);
    for s in &mut rows {
        let mut path = vec![7; 1025];
        path.extend(&s.position);
        s.position = path;
    }
    rows.rotate_left(3);
    assert_eq!(run(&p, &rows, 1), expected);
}
#[test]
fn terminal_publication_scales_across_128_tasks_without_cross_task_containment() {
    scaling(128);
}
#[test]
fn terminal_publication_scales_across_4096_tasks_without_cross_task_containment() {
    scaling(4096);
}
fn scaling(tasks: usize) {
    let p = query();
    let mut rows = query_rows(&p, tasks);
    rows.reverse();
    let (actual, captures) = run(&p, &rows, tasks);
    assert_eq!(actual.len(), tasks * 3);
    assert_eq!(captures.len(), tasks * 4);
    let (resident, _) = fixture(&p, tasks);
    let mut want = vec![];
    let mut cells = vec![];
    for task in 0..tasks as u32 {
        for (ordinal, ids) in [
            (2, vec![(1, 0), (3, 0)]),
            (0, vec![(2, 0)]),
            (4, vec![(3, 1)]),
        ] {
            let caps = ids
                .into_iter()
                .map(|(n, l)| capture(&resident, n, l))
                .collect::<Vec<_>>();
            let (r, c) = expected(task, ordinal, task + 1, 1, cells.len() as u32, &caps);
            want.push(r);
            cells.extend(c);
        }
    }
    assert_eq!(actual, want);
    assert_eq!(captures, cells);
}

#[test]
fn complete_terminal_identity_keeps_pc_node_cursor_and_repeat_distinctions_before_numbering() {
    let original = query();
    let mut words = original.words.clone();
    words.push(word(EMIT, 0, 0));
    let p = Program::new(words, original.strings.clone(), vec![], vec![0, 1], vec![])
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap();
    let (resident, _) = fixture(&p, 1);
    let cap = capture(&resident, 1, 0);
    let last = capture(&resident, 3, 1);
    let mut rows = (0..6)
        .map(|i| {
            let mut s = state(7, 1);
            s.position = vec![i];
            s.captures = vec![cap];
            s
        })
        .collect::<Vec<_>>();
    rows[1].pc = 8;
    rows[2].node = 2;
    rows[3].cursors = vec![[1, 2, 0, 0]];
    rows[4].repeats = vec![[0, 7, 0, 0]];
    rows[5].captures = vec![last];
    rows.reverse();
    let (actual, captures) = run(&p, &rows, 1);
    let (a, mut cells) = expected(0, 0, 1, 1, 0, &[cap]);
    let (b, c) = expected(0, 5, 1, 1, 1, &[last]);
    cells.extend(c);
    assert_eq!(actual, vec![a, b]);
    assert_eq!(captures, cells);
}
#[test]
fn terminal_containment_capacity_failure_cannot_publish_admitted_candidates() {
    let p = query();
    let fixture = fixture_sources(&p, 1, &["# comment\n".repeat(5000)]);
    let rows = (1..=5000)
        .map(|node| {
            let mut s = state(7, 1);
            s.node = node;
            s.position = vec![node];
            s
        })
        .collect::<Vec<_>>();
    // Different complete keys pass admission. One task's 25M containment pairs
    // exceed the existing 16M bounded comparison capacity before any payload copy.
    assert_eq!(
        attempt_resident(&p, &rows, fixture, |_, _, _| {}),
        Err(Error::BackendCapacity)
    );
}
#[test]
fn terminal_grouping_capacity_failure_cannot_publish_a_valid_prefix() {
    let p = query();
    let rows = (0..5000)
        .map(|i| {
            let mut s = state(7, 1);
            s.position = vec![i];
            s
        })
        .collect::<Vec<_>>();
    assert_eq!(
        attempt(&p, &rows, 1, |_, _, _| {}),
        Err(Error::BackendCapacity)
    );
}
#[test]
fn terminal_publication_rejects_valid_pending_offers_and_corrupt_ready_lists() {
    let p = query();
    let mut rows = query_rows(&p, 1);
    let mut pending = state(6, 6);
    pending.position = vec![9, 1];
    pending.body = vec![9, 0];
    pending.offer = vec![9, 0];
    rows.push(pending);
    assert_eq!(
        attempt(&p, &rows, 1, |_, _, _| {}),
        Err(Error::BackendFailure)
    );
    rows.pop();
    assert_eq!(
        attempt(&p, &rows, 1, |_, b, _| {
            let mut ids = read::<u32>(&b.indices, b.capacity * 4);
            ids[b.capacity * 3 + 1] = ids[b.capacity * 3];
            write(&b.indices, &ids);
        }),
        Err(Error::BackendFailure)
    );
}

#[test]
fn gpu_successors_and_fresh_readiness_feed_terminal_publication_without_host_payload_roundtrip() {
    use super::super::metal_ready_batch::{self, ReadyInput};
    let _guard = metal_execution_guard().unwrap();
    let scope = super::super::metal_backend::launch::LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let p = query();
    let (resident, plan) = fixture(&p, 2);
    let data = super::super::metal_register_execution::upload(&device, &resident).unwrap();
    let mut rows = query_rows(&p, 2);
    for s in &mut rows {
        s.kind = 0;
    }
    rows.reverse();
    let input = upload(&device, &rows, &p);
    let initial = batch(&device, &rows, input.states.capacity);
    let next = storage(
        &device,
        input.states.capacity,
        input.control.digit_capacity * 4,
        9,
    );
    let mut ready = batch(&device, &[], next.states.capacity);
    ready.generation = 9;
    let mut out = output(&device, rows.len(), 64);
    out.generation = 9;
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let advanced = metal_successor::encode(
        &device,
        &queue,
        command,
        SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &input.states,
            values: &input.values,
            control: &input.control,
            ready: &initial,
        },
        SuccessorOutput {
            states: &next.states,
            values: &next.values,
            control: &next.control,
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
            states: &next.states,
            values: &next.values,
        },
        &ready,
    )
    .unwrap();
    let terminal = metal_terminal::encode(
        &device,
        command,
        &SuccessorInput {
            resident: &resident,
            plan: &plan,
            data: &data,
            states: &next.states,
            values: &next.values,
            control: &next.control,
            ready: &ready,
        },
        &out,
    )
    .unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    advanced.check_completed().unwrap();
    scheduled.check_completed().unwrap();
    assert!(
        terminal
            .check_completed()
            .unwrap()
            .errors
            .iter()
            .all(Option::is_none)
    );
    assert_eq!(read::<u32>(&out.counts, 2), vec![6, 8]);
    let mut want = vec![];
    let mut cells = vec![];
    for task in 0..2 {
        for (ordinal, ids) in [
            (2, vec![(1, 0), (3, 0)]),
            (0, vec![(2, 0)]),
            (4, vec![(3, 1)]),
        ] {
            let caps = ids
                .into_iter()
                .map(|(n, l)| capture(&resident, n, l))
                .collect::<Vec<_>>();
            let (r, c) = expected(task, ordinal, task + 1, 1, cells.len() as u32, &caps);
            want.push(r);
            cells.extend(c);
        }
    }
    assert_eq!(read::<ResultRecord>(&out.results, 6), want);
    assert_eq!(read::<FinalCaptureRecord>(&out.captures, 8), cells);
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    eprintln!(
        "terminal successor handoff launches={} peak_bytes={}",
        trace.len(),
        trace
            .iter()
            .map(|r| r.peak_application_buffer_bytes)
            .max()
            .unwrap_or(0)
    );
}

#[test]
fn terminal_publication_resolves_roots_and_labels_across_files_and_programs() {
    let p = query();
    let mut words = p.words.clone();
    words[7] = word(EMIT, 0, 0);
    let other = Program::new(words, p.strings.clone(), vec![], vec![0, 1], vec![])
        .unwrap()
        .with_tree_sitter_query_semantics()
        .unwrap();
    let programs = [p.clone(), other];
    let docs = ["# first\n# second\n", "# different length\n# last\n"]
        .into_iter()
        .map(|text| {
            let source = Arc::new(text.to_owned());
            let grammar = Grammar::new(SupportLang::Python).unwrap();
            let native = StrDoc::new(source.as_str(), SupportLang::Python);
            let image =
                TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
            TapeDoc::restore(source, grammar, &image, &|| false).unwrap()
        })
        .collect::<Vec<_>>();
    let resident = ResidentInput::new(
        MetalGridInput::build(
            &programs,
            &docs,
            &[vec![vec![1], vec![2]], vec![vec![2], vec![1]]],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let plan = ExecutionPlan::new(&resident, &programs).unwrap();
    let mut rows = vec![];
    let mut program_ids = vec![];
    let mut want = vec![];
    let mut cells = vec![];
    for e in &resident.input.grid.evaluation_spans {
        let file = resident.input.packed.files[e.file as usize];
        let root = if e.file == e.program { 1 } else { 2 };
        let node = file.node_start + root;
        let cap = capture(&resident, node, 0);
        let mut s = state(7, 1);
        s.task = e.semantic_start;
        s.node = file.node_start + 1;
        s.captures = vec![cap];
        s.position = vec![0];
        rows.push(s);
        program_ids.push(e.program);
        let (r, c) = expected(
            e.semantic_start,
            0,
            node,
            1 - e.program,
            cells.len() as u32,
            &[cap],
        );
        want.push(r);
        cells.extend(c);
    }
    let (actual, captures) = attempt_resident(&p, &rows, (resident, plan), |input, _, _| {
        let mut columns = read::<u32>(&input.states.columns, rows.len() * 12);
        for (i, pg) in program_ids.iter().enumerate() {
            columns[4 * rows.len() + i] = *pg;
        }
        write(&input.states.columns, &columns);
    })
    .unwrap();
    assert_eq!(actual, want);
    assert_eq!(captures, cells);
}

#[test]
fn terminal_publication_rejects_invalid_ordinary_capture_order_and_preserves_query_ties() {
    let ordinary = Program::new(
        vec![word(EMIT, 0, 0)],
        vec![b"a".to_vec(), b"b".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    let (resident, _) = fixture(&ordinary, 1);
    let a = capture(&resident, 1, 0);
    let b = capture(&resident, 1, 1);
    let mut s = state(0, 3);
    s.captures = vec![a, b];
    let (r, c) = expected(0, 0, 1, 0, 0, &[a, b]);
    assert_eq!(run(&ordinary, &[s.clone()], 1), (vec![r], c));
    s.captures = vec![b, a];
    assert_eq!(
        attempt(&ordinary, &[s.clone()], 1, |_, _, _| {}),
        Err(Error::BackendFailure)
    );
    let query = ordinary.with_tree_sitter_query_semantics().unwrap();
    let (r, c) = expected(0, 0, 1, 0, 0, &[b, a]);
    assert_eq!(run(&query, &[s], 1), (vec![r], c));
}

#[test]
fn staged_terminal_storage_preserves_full_results_without_overlapping_future_scratch() {
    for tasks in [4_096, 32_768] {
        metal::objc::rc::autoreleasepool(|| {
            let _guard = metal_execution_guard().unwrap();
            let scope = super::super::metal_backend::launch::LaunchScope::enter();
            let device = Device::system_default().unwrap();
            let p = query();
            let (resident, plan) = fixture(&p, tasks);
            let data = super::super::metal_register_execution::upload(&device, &resident).unwrap();
            let mut rows = query_rows(&p, tasks);
            rows.reverse();
            let input = upload(&device, &rows, &p);
            let ready = batch(&device, &rows, input.states.capacity);
            let queue = device.new_command_queue();
            let before = super::super::metal_backend::buffer_usage().0;
            let first = scope.snapshot().len();
            let allocations = super::super::metal_backend::BufferObservation::enter();
            let started = std::time::Instant::now();
            let (out, completion) = metal_terminal::execute(
                &device,
                &queue,
                &SuccessorInput {
                    resident: &resident,
                    plan: &plan,
                    data: &data,
                    states: &input.states,
                    values: &input.values,
                    control: &input.control,
                    ready: &ready,
                },
            )
            .unwrap();
            assert!(completion.errors.iter().all(Option::is_none));
            let counts = read::<u32>(&out.counts, 2);
            assert_eq!(counts, [3 * tasks as u32, 4 * tasks as u32]);
            assert_eq!(out.capacity, counts[0] as usize);
            assert_eq!(out.capture_capacity, counts[1] as usize);
            let mut want = Vec::new();
            let mut cells = Vec::new();
            for task in 0..tasks as u32 {
                for (ordinal, ids) in [
                    (2, vec![(1, 0), (3, 0)]),
                    (0, vec![(2, 0)]),
                    (4, vec![(3, 1)]),
                ] {
                    let captures = ids
                        .into_iter()
                        .map(|(node, label)| capture(&resident, node, label))
                        .collect::<Vec<_>>();
                    let (result, captures) =
                        expected(task, ordinal, task + 1, 1, cells.len() as u32, &captures);
                    want.push(result);
                    cells.extend(captures);
                }
            }
            assert_eq!(read::<ResultRecord>(&out.results, counts[0] as usize), want);
            assert_eq!(
                read::<FinalCaptureRecord>(&out.captures, counts[1] as usize),
                cells
            );
            let trace = scope.snapshot();
            let trace = &trace[first..];
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            let ordering = trace
                .iter()
                .filter(|r| {
                    r.kernel.starts_with("position_") || r.kernel.starts_with("terminal_order_")
                })
                .map(|r| r.application_buffer_bytes.saturating_sub(before))
                .max()
                .unwrap();
            let peak = trace
                .iter()
                .map(|r| r.peak_application_buffer_bytes.saturating_sub(before))
                .max()
                .unwrap();
            eprintln!(
                "TERMINAL_STORAGE_EVIDENCE {}",
                serde_json::json!({"rows":rows.len(),"tasks":tasks,"orderingBytes":ordering,"peakAdditionalBytes":peak,"dispatches":trace.len(),"seconds":started.elapsed().as_secs_f64()})
            );
            let retained = out.results.buffer.length()
                + out.captures.buffer.length()
                + out.counts.buffer.length();
            assert_eq!(
                allocations.wait_for_live_bytes(retained as usize),
                retained as usize,
                "terminal scratch must retire before returned output is consumed"
            );
            assert!(
                ordering <= 112 * input.states.capacity + 1024 * 1024,
                "ordering must exclude future projection and admission scratch: {ordering}"
            );
            assert!(
                peak <= 224 * input.states.capacity + 1024 * 1024,
                "terminal stages must retire completed scratch before exact publication: {peak}"
            );
        });
    }
}
