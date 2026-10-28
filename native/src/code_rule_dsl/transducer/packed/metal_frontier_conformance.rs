//! Independent exact-state and completed-capture oracles with actual dispatch evidence.
use super::metal_backend::launch::LaunchScope;
use super::metal_backend::{BufferObservation, MappedBuffer, metal_execution_guard};
use super::metal_frontier_reduce::{
    self, CAPTURE_COUNT, CAPTURE_START, COLUMNS, Frontier, KEY_COLUMNS, ORDINAL,
};
use super::*;
use metal::Device;

pub(super) fn convergence_case(siblings: usize) -> (Program, TapeDoc, Vec<Output>) {
    use crate::code_rule_dsl::tape::{Grammar, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect as SupportLang;

    // NEXT reaches the join through bounded register work. FOLLOWING reaches the
    // same join through a variable expansion. The later captured branch must keep
    // ordinal 1: the already admitted join must not emit another raw candidate.
    let mut program = Program::new(
        vec![
            word(SAVE_CURSOR, 0, 0),
            word(REPEAT_BEGIN, 0, 0),
            word(FORK, 0, 12),
            word(FORK, 0, 8),
            word(NEXT, NAMED, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(GOTO, 0, 11),
            word(STOP, 0, 0),
            word(FOLLOWING, NAMED, 0),
            word(RESTORE_CURSOR, 0, 0),
            word(GOTO, 0, 11),
            word(EMIT, 0, 0),
            word(CAPTURE, 0, 1),
            word(EMIT, 0, 1),
        ],
        vec![b"empty".to_vec(), b"captured".to_vec()],
        vec![],
        vec![0, 1],
        vec![],
    )
    .unwrap();
    program.tree_sitter_query = true;
    program.validate().unwrap();

    let source = Arc::new("# node\n".repeat(siblings));
    let grammar = Grammar::new(SupportLang::Python).unwrap();
    let native = StrDoc::new(source.as_str(), SupportLang::Python);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    let file = TapeDoc::restore(source, grammar, &image, &|| false).unwrap();
    let expected = vec![Output {
        task_ordinal: 0,
        local_ordinal: 1,
        start: 1,
        label: 1,
        captures: vec![Capture {
            label: 1,
            node: 1,
            span: 0..6,
            text: None,
            properties: vec![],
        }],
        properties: vec![],
        dynamic_properties: vec![],
    }];
    assert_eq!(program.execute_roots(&file, [1]).unwrap(), expected);
    (program, file, expected)
}

#[test]
fn converging_structural_waves_preserve_admission_and_raw_candidate_ordinals() {
    for siblings in [2, 130] {
        let (program, file, expected) = convergence_case(siblings);
        let actual = execute_with_backend(
            ExecutionBackend::Metal,
            std::slice::from_ref(&program),
            &[file],
            &[vec![vec![1]]],
        );
        assert_eq!(actual, Ok(vec![vec![expected]]), "siblings={siblings}");
    }
}

#[test]
fn production_dynamic_generations_preserve_complete_results_across_independent_tasks() {
    for tasks in [1, 128, 4096] {
        let (program, file, _) = convergence_case(2);
        let programs = [program];
        let files = vec![file; tasks];
        let anchors = vec![vec![vec![1]]; tasks];
        let expected = execute_batch(&programs, &files, Some(&anchors)).unwrap();
        let scope = LaunchScope::enter();
        let actual = execute_with_backend(ExecutionBackend::Metal, &programs, &files, &anchors);
        let trace = scope.snapshot();
        eprintln!(
            "production tasks={tasks} attempted={} encoded={} completed={}",
            trace.len(),
            trace.iter().filter(|r| r.encoded).count(),
            trace.iter().filter(|r| r.completed).count()
        );
        assert_eq!(actual, Ok(expected), "tasks={tasks}");
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        assert!(trace.len() <= 600);
    }
}

#[derive(Clone)]
struct State {
    key: [u32; KEY_COLUMNS],
    captures: Vec<CaptureIdentity>,
}

fn cap(label: u16, node: u32, start: usize, end: usize) -> CaptureIdentity {
    CaptureIdentity {
        label,
        node,
        start,
        end,
    }
}

fn oracle(states: &[State]) -> Vec<u32> {
    let mut groups = BTreeMap::<[u32; KEY_COLUMNS], Vec<(usize, Vec<CaptureIdentity>)>>::new();
    for (index, state) in states.iter().enumerate() {
        let members = groups.entry(state.key).or_default();
        if members
            .iter()
            .any(|(_, existing)| capture_identity_containment(existing, &state.captures).0)
        {
            continue;
        }
        members.retain(|(_, existing)| !capture_identity_containment(&state.captures, existing).0);
        members.push((index, state.captures.clone()));
    }
    let mut survivors = groups
        .values()
        .flatten()
        .map(|(index, _)| *index as u32)
        .collect::<Vec<_>>();
    survivors.sort_unstable();
    survivors
}

fn exact_oracle(states: &[State]) -> Vec<u32> {
    let mut seen = BTreeSet::new();
    states
        .iter()
        .enumerate()
        .filter_map(|(index, state)| {
            let captures = state
                .captures
                .iter()
                .map(|c| (c.label, c.node, c.start, c.end))
                .collect::<Vec<_>>();
            seen.insert((state.key, captures)).then_some(index as u32)
        })
        .collect()
}

fn run(states: &[State], bucket_bits: u32) -> usize {
    run_reduction(states, bucket_bits, false)
}

fn run_reduction(states: &[State], bucket_bits: u32, active: bool) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let mut columns = vec![0; states.len() * COLUMNS];
    let mut captures = Vec::<[u32; 4]>::new();
    for (index, state) in states.iter().enumerate() {
        for (column, value) in state.key.iter().enumerate() {
            columns[column * states.len() + index] = *value;
        }
        columns[CAPTURE_START * states.len() + index] = captures.len() as u32;
        columns[CAPTURE_COUNT * states.len() + index] = state.captures.len() as u32;
        columns[ORDINAL * states.len() + index] = index as u32;
        captures.extend(
            state
                .captures
                .iter()
                .map(|c| [c.node, u32::from(c.label), c.start as u32, c.end as u32]),
        );
    }
    let frontier = Frontier {
        columns: MappedBuffer::from_slice(&device, &columns).unwrap(),
        len: states.len(),
        generation: 7,
    };
    let capture_count = captures.len();
    let captures = MappedBuffer::from_slice(&device, &captures).unwrap();
    let scope = LaunchScope::enter();
    let allocations = BufferObservation::enter();
    let reducer = if active {
        metal_frontier_reduce::reduce_active
    } else {
        metal_frontier_reduce::reduce_completed
    };
    let result = reducer(&device, &frontier, &captures, capture_count, bucket_bits);
    let result = result.unwrap();
    let output_bytes = result.columns.buffer.length() as usize;
    assert_eq!(
        allocations.wait_for_live_bytes(output_bytes),
        output_bytes,
        "completed reductions must release all scratch allocations"
    );
    assert_eq!(result.generation, 7, "reduction is not a structural wave");
    let actual = unsafe {
        std::slice::from_raw_parts(
            result.columns.buffer.contents().cast::<u32>(),
            result.len * COLUMNS,
        )
    };
    let expected = if active {
        exact_oracle(states)
    } else {
        oracle(states)
    };
    assert_eq!(&actual[ORDINAL * result.len..], expected);
    for (out, &input) in expected.iter().enumerate() {
        for column in 0..COLUMNS {
            assert_eq!(
                actual[column * result.len + out],
                columns[column * states.len() + input as usize]
            );
        }
    }
    let trace = scope.snapshot();
    assert_eq!(
        trace
            .iter()
            .map(|r| r.command)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert_eq!(trace.iter().filter(|r| r.indirect).count(), 2);
    assert!(
        trace
            .iter()
            .all(|r| r.encoded && r.submitted && r.completed)
    );
    assert!(
        trace
            .iter()
            .all(|r| r.peak_application_buffer_bytes <= super::metal_backend::METAL_BUFFER_LIMIT)
    );
    assert!(
        trace.len() <= 20,
        "broad reduction must not schedule individual states or histories"
    );
    eprintln!(
        "FRONTIER_REDUCTION_EVIDENCE {}",
        serde_json::json!({
            "states":states.len(), "survivors":result.len, "activeExact":active, "dispatches":trace,
        })
    );
    trace.len()
}

#[test]
fn compact_reduction_preserves_full_keys_order_multiplicity_and_hash_collisions() {
    let a = cap(0, 10, 0, 1);
    let b = cap(1, 10, 0, 1);
    let c = cap(0, 11, 0, 1);
    let key = [0; KEY_COLUMNS];
    let mut states = vec![
        State {
            key,
            captures: vec![],
        },
        State {
            key,
            captures: vec![a],
        },
        State {
            key,
            captures: vec![a, a],
        },
        State {
            key,
            captures: vec![a, b],
        },
        State {
            key,
            captures: vec![b, a],
        },
        State {
            key,
            captures: vec![c],
        },
        State {
            key,
            captures: vec![a, a],
        },
    ];
    for column in 0..KEY_COLUMNS {
        let mut other = key;
        other[column] = 1;
        states.push(State {
            key: other,
            captures: vec![a],
        });
    }
    run_reduction(&states, 0, true);
    run(&states, 0); // Every hash collides. Full identity must still determine membership.
}

#[test]
fn compact_reduction_scales_independent_segments_in_bounded_broad_stages() {
    for segments in [1, 128, 4096] {
        let mut states = Vec::new();
        for segment in 0..segments {
            let mut key = [0; KEY_COLUMNS];
            key[0] = segment;
            for captures in [vec![], vec![cap(0, 10, 0, 1)], vec![cap(0, 10, 0, 1); 2]] {
                states.push(State { key, captures });
            }
        }
        run_reduction(&states, 14, true);
        run(&states, 14);
    }
}

#[test]
fn compact_reduction_parallelizes_long_capture_histories() {
    let captures = (0..8192)
        .map(|n| cap(0, n, n as usize, n as usize + 1))
        .collect::<Vec<_>>();
    let states = [
        State {
            key: [0; KEY_COLUMNS],
            captures: captures[..8191].to_vec(),
        },
        State {
            key: [0; KEY_COLUMNS],
            captures: captures.clone(),
        },
        State {
            key: [0; KEY_COLUMNS],
            captures,
        },
    ];
    run_reduction(&states, 0, true);
    run(&states, 0);
}

#[test]
fn compact_reduction_matches_ordered_oracle_across_capture_sequences() {
    let mut seed = 27_u32;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut states = Vec::new();
    for group in 0..64 {
        let mut key = [0; KEY_COLUMNS];
        key[0] = group;
        for _ in 0..16 {
            let mut captures = (0..random() % 16)
                .map(|_| {
                    let node = random() % 12;
                    // Distinct nodes can have identical geometry, including zero width.
                    let start = node / 3;
                    cap((random() % 3) as u16, node, start as usize, start as usize)
                })
                .collect::<Vec<_>>();
            captures.sort_by(|a, b| compare_capture_nodes(*a, *b));
            states.push(State { key, captures });
        }
    }
    run_reduction(&states, 4, true);
    run(&states, 4);
}

#[test]
fn compact_reduction_refuses_pair_exhaustion_without_successful_publication() {
    let guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let frontier = Frontier {
        columns: MappedBuffer::zeroed::<u32>(&device, 4097 * COLUMNS).unwrap(),
        len: 4097,
        generation: 0,
    };
    let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    let scope = LaunchScope::enter();
    let result = metal_frontier_reduce::reduce_completed(&device, &frontier, &captures, 0, 0);
    drop(guard);
    assert!(matches!(result, Err(Error::BackendCapacity)));
    assert!(scope.snapshot().iter().all(|r| r.completed));
}

#[test]
fn compact_reduction_cannot_reset_the_complete_evaluation_budget() {
    let guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let frontier = Frontier {
        columns: MappedBuffer::zeroed::<u32>(&device, COLUMNS).unwrap(),
        len: 1,
        generation: 0,
    };
    let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    let scope = LaunchScope::enter();
    // Fifty-four commands consume 594 launches. The next cannot submit its allowed prefix.
    for _ in 0..54 {
        let _nested = LaunchScope::enter();
        metal_frontier_reduce::reduce_completed(&device, &frontier, &captures, 0, 0).unwrap();
    }
    let result = metal_frontier_reduce::reduce_completed(&device, &frontier, &captures, 0, 0);
    drop(guard);
    assert!(matches!(result, Err(Error::BackendLaunchLimit)));
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|r| r.encoded).count(), 600);
    assert_eq!(trace.iter().filter(|r| r.submitted).count(), 594);
    assert_eq!(trace.iter().filter(|r| r.completed).count(), 594);
    assert!(trace.iter().skip(600).all(|r| !r.encoded));
}

#[test]
fn compact_reduction_rejects_invalid_capture_ranges_before_indirect_access() {
    let guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    let mut results = Vec::new();
    for start in [u32::MAX, 0] {
        let mut columns = [0; COLUMNS];
        columns[CAPTURE_START] = start;
        columns[CAPTURE_COUNT] = 2;
        let frontier = Frontier {
            columns: MappedBuffer::from_slice(&device, &columns).unwrap(),
            len: 1,
            generation: 0,
        };
        let _scope = LaunchScope::enter();
        results.push(metal_frontier_reduce::reduce_completed(
            &device, &frontier, &captures, 0, 0,
        ));
    }
    drop(guard);
    assert!(
        results
            .iter()
            .all(|result| matches!(result, Err(Error::BackendFailure)))
    );
}

#[test]
fn compact_scans_preserve_saturated_prefixes_beyond_512_rows() {
    use super::metal_backend::launch::{commit_checked, wait_checked};
    let guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let scan = super::metal_parallel::Scan::new(&device, 515).unwrap();
    unsafe {
        let values = scan.values.buffer.contents().cast::<[u32; 4]>();
        for i in 0..515 {
            values.add(i).write([u32::MAX - 1, 1, 0, 0]);
        }
    }
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    scan.encode(command).unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let offsets = unsafe {
        std::slice::from_raw_parts(scan.values.buffer.contents().cast::<[u32; 4]>(), 515)
    }
    .to_vec();
    let total = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
    drop(guard);
    assert_eq!(total, [u32::MAX, 515, 0, 0]);
    for (index, offset) in offsets.iter().enumerate() {
        assert_eq!(
            *offset,
            [
                ((u64::from(u32::MAX) - 1) * index as u64).min(u64::from(u32::MAX)) as u32,
                index as u32,
                0,
                0,
            ]
        );
    }
    assert_eq!(scope.snapshot().len(), 1);
}

#[test]
fn compact_reduction_keeps_capture_prefixes_required_by_native_continuations() {
    // Native query: (module [(comment) @x @y (comment) @x]+ @z).
    // Appending @z to [x,y] and [x] produces two distinct completed matches.
    let x = cap(0, 1, 0, 6);
    let y = cap(1, 1, 0, 6);
    let states = [vec![x, y], vec![x], vec![x], vec![y, x]].map(|captures| State {
        key: [0; KEY_COLUMNS],
        captures,
    });
    assert_eq!(exact_oracle(&states), vec![0, 1, 3]);
    run_reduction(&states, 0, true);
}

#[test]
fn counted_reduction_connects_gpu_producer_and_consumer_in_one_command() {
    for (capacity, count, unique) in [(0, 0, 1), (32, 0, 1), (32, 6, 3), (1025, 513, 257)] {
        run_counted_pipeline(capacity, count, unique);
    }
}

fn run_counted_pipeline(capacity: usize, count: u32, unique: u32) {
    use super::metal_backend::launch::{commit_checked, wait_checked};
    use super::metal_backend::pipeline_named;
    use super::metal_frontier_reduce::{CountedFrontier, encode_active};
    use super::metal_parallel::encode;

    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let make_frontier = || CountedFrontier {
        columns: MappedBuffer::from_slice(&device, &vec![u32::MAX; capacity * COLUMNS]).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap(),
        capacity,
        generation: 7,
    };
    let input = make_frontier();
    let output = make_frontier();
    let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    let consumed = MappedBuffer::from_slice(&device, &vec![u32::MAX; capacity]).unwrap();
    let source = r#"
#include <metal_stdlib>
using namespace metal;
kernel void produce_counted_states(device uint* columns [[buffer(0)]],
 device uint* counts [[buffer(1)]],constant uint2& parameters [[buffer(2)]],uint tid [[thread_position_in_grid]]) {
 uint n=parameters.x,unique=parameters.y;
 if(tid==0u) { counts[0]=n; counts[1]=0u; counts[2]=(n+255u)/256u; counts[3]=counts[4]=1u; }
 if(tid>=n) return;
 for(uint field=0u;field<11u;field++) columns[field*n+tid]=field==0u?tid%unique:(field==10u?tid:0u);
}
kernel void consume_counted_states(device const uint* columns [[buffer(0)]],
 device const uint* counts [[buffer(1)]],device uint* consumed [[buffer(2)]],
 constant uint& capacity [[buffer(3)]],uint tid [[thread_position_in_grid]]) {
 if(tid<counts[0] && tid<capacity) consumed[tid]=columns[tid];
}
"#;
    let producer = pipeline_named(source, "produce_counted_states").unwrap();
    let consumer = pipeline_named(source, "consume_counted_states").unwrap();
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    encode(
        command,
        &producer,
        "produce_counted_states",
        &[&input.columns.buffer, &input.counts.buffer],
        &[count, unique],
        (count as usize).div_ceil(256).max(1),
        None,
    );
    let pending = encode_active(&device, command, &input, &output, &captures, 0, 0).unwrap();
    encode(
        command,
        &consumer,
        "consume_counted_states",
        &[
            &output.columns.buffer,
            &output.counts.buffer,
            &consumed.buffer,
        ],
        &[capacity as u32],
        0,
        Some((&output.counts.buffer, 8)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    let survivors = count.min(unique);
    assert_eq!(counts, &[survivors, 0, survivors.div_ceil(256), 1, 1]);
    let actual = unsafe {
        std::slice::from_raw_parts(
            output.columns.buffer.contents().cast::<u32>(),
            survivors as usize * COLUMNS,
        )
    };
    let expected = (0..COLUMNS)
        .flat_map(|field| {
            (0..survivors).map(move |index| {
                if field == 0 || field == ORDINAL {
                    index
                } else {
                    0
                }
            })
        })
        .collect::<Vec<u32>>();
    assert_eq!(actual, expected);
    let actual =
        unsafe { std::slice::from_raw_parts(consumed.buffer.contents().cast::<u32>(), capacity) };
    assert_eq!(
        &actual[..survivors as usize],
        (0..survivors).collect::<Vec<_>>()
    );
    assert!(
        actual[survivors as usize..]
            .iter()
            .all(|&slot| slot == u32::MAX),
        "consumer must not visit inactive capacity slots"
    );
    assert_eq!(output.generation, input.generation);
    let trace = scope.snapshot();
    eprintln!(
        "COUNTED_FRONTIER_EVIDENCE {}",
        serde_json::json!({
            "capacity": capacity, "states": count, "survivors": survivors, "dispatches": trace,
        })
    );
    assert_eq!(
        trace
            .iter()
            .map(|dispatch| dispatch.command)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert!(
        trace
            .iter()
            .all(|dispatch| dispatch.encoded && dispatch.submitted && dispatch.completed)
    );
}

#[test]
fn counted_reduction_rejects_invalid_lengths_and_capacity_without_publication() {
    use super::metal_backend::launch::{commit_checked, wait_checked};
    use super::metal_frontier_reduce::{CountedFrontier, encode_active};

    // Each malformed header must refuse the evaluation and publish no usable work.
    for (header, capacity, expected) in [
        ([3, 0, 1, 1, 1], 2, Error::BackendFailure),
        ([2, 1, 1, 1, 1], 2, Error::BackendFailure),
        ([2, 0, 2, 1, 1], 2, Error::BackendFailure),
        ([2, 0, 1, 2, 1], 2, Error::BackendFailure),
        ([2, 0, 1, 1, 0], 2, Error::BackendFailure),
        ([2, 0, 1, 1, 1], 1, Error::BackendCapacity),
    ] {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let mut columns = vec![0; COLUMNS * 2];
        columns[1] = 1; // Two distinct complete states, so neither can disappear.
        columns[ORDINAL * 2 + 1] = 1;
        let input = CountedFrontier {
            columns: MappedBuffer::from_slice(&device, &columns).unwrap(),
            counts: MappedBuffer::from_slice(&device, &header).unwrap(),
            capacity: 2,
            generation: 0,
        };
        let output = CountedFrontier {
            columns: MappedBuffer::from_slice(&device, &vec![u32::MAX; COLUMNS * capacity])
                .unwrap(),
            counts: MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap(),
            capacity,
            generation: 0,
        };
        let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
        let _scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = encode_active(&device, command, &input, &output, &captures, 0, 0).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(
            pending.check_completed(),
            Err(expected),
            "header={header:?}"
        );
        let counts =
            unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
        assert_eq!(counts, &[0, 0, 0, 1, 1]);
        let output = unsafe {
            std::slice::from_raw_parts(
                output.columns.buffer.contents().cast::<u32>(),
                COLUMNS * capacity,
            )
        };
        assert_eq!(output, vec![u32::MAX; COLUMNS * capacity]);
    }
}

#[test]
fn counted_reduction_requires_matching_generation_and_completed_command() {
    use super::metal_frontier_reduce::{CountedFrontier, encode_active};
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let make_frontier = |generation| CountedFrontier {
        columns: MappedBuffer::zeroed::<u32>(&device, COLUMNS).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[1u32, 0, 1, 1, 1]).unwrap(),
        capacity: 1,
        generation,
    };
    let input = make_frontier(0);
    let mut output = make_frontier(1);
    let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    assert!(matches!(
        encode_active(&device, command, &input, &output, &captures, 0, 0),
        Err(Error::BackendFailure)
    ));
    assert!(scope.snapshot().is_empty());
    output.generation = 0;
    assert!(matches!(
        encode_active(&device, command, &input, &input, &captures, 0, 0),
        Err(Error::BackendFailure)
    ));
    assert!(scope.snapshot().is_empty());
    let pending = encode_active(&device, command, &input, &output, &captures, 0, 0).unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|dispatch| !dispatch.submitted && !dispatch.completed)
    );
}

#[test]
fn counted_reduction_preserves_ordered_captures_and_rejects_invalid_ranges() {
    use super::metal_backend::launch::{commit_checked, wait_checked};
    use super::metal_frontier_reduce::{CountedFrontier, encode_active};
    let x = cap(0, 1, 0, 6);
    let y = cap(1, 1, 0, 6);
    let states = [vec![x, y], vec![x], vec![x], vec![y, x], vec![x, x]].map(|captures| State {
        key: [0; KEY_COLUMNS],
        captures,
    });
    for invalid_range in [false, true] {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let mut columns = vec![0; states.len() * COLUMNS];
        let mut captures = Vec::new();
        for (index, state) in states.iter().enumerate() {
            columns[CAPTURE_START * states.len() + index] = captures.len() as u32;
            columns[CAPTURE_COUNT * states.len() + index] = state.captures.len() as u32;
            columns[ORDINAL * states.len() + index] = index as u32;
            captures.extend(
                state
                    .captures
                    .iter()
                    .map(|c| [c.node, u32::from(c.label), c.start as u32, c.end as u32]),
            );
        }
        if invalid_range {
            columns[CAPTURE_START * states.len()] = u32::MAX;
        }
        let capture_count = captures.len();
        let captures = MappedBuffer::from_slice(&device, &captures).unwrap();
        let input = CountedFrontier {
            columns: MappedBuffer::from_slice(&device, &columns).unwrap(),
            counts: MappedBuffer::from_slice(
                &device,
                &[states.len() as u32, capture_count as u32, 1, 1, 1],
            )
            .unwrap(),
            capacity: states.len(),
            generation: 7,
        };
        let output = CountedFrontier {
            columns: MappedBuffer::from_slice(&device, &vec![u32::MAX; states.len() * COLUMNS])
                .unwrap(),
            counts: MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap(),
            capacity: states.len(),
            generation: 7,
        };
        let scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = encode_active(
            &device,
            command,
            &input,
            &output,
            &captures,
            capture_count,
            0,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        let counts =
            unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
        let actual = unsafe {
            std::slice::from_raw_parts(
                output.columns.buffer.contents().cast::<u32>(),
                states.len() * COLUMNS,
            )
        };
        if invalid_range {
            assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
            assert_eq!(counts, &[0, 0, 0, 1, 1]);
            assert_eq!(actual, vec![u32::MAX; states.len() * COLUMNS]);
        } else {
            pending.check_completed().unwrap();
            let survivors = exact_oracle(&states);
            assert_eq!(survivors, [0, 1, 3, 4]);
            assert_eq!(counts, &[4, capture_count as u32, 1, 1, 1]);
            for field in 0..COLUMNS {
                for (out, &index) in survivors.iter().enumerate() {
                    assert_eq!(
                        actual[field * survivors.len() + out],
                        columns[field * states.len() + index as usize]
                    );
                }
            }
            assert_eq!(
                &actual[survivors.len() * COLUMNS..],
                vec![u32::MAX; COLUMNS]
            );
        }
        let trace = scope.snapshot();
        assert_eq!(trace.len(), 12);
        assert!(
            trace
                .iter()
                .all(|dispatch| dispatch.encoded && dispatch.submitted && dispatch.completed)
        );
    }
}

#[test]
fn counted_reduction_shares_the_evaluation_launch_budget_across_commands() {
    use super::metal_backend::launch::{commit_checked, wait_checked};
    use super::metal_frontier_reduce::{CountedFrontier, encode_active};
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let make_frontier = || CountedFrontier {
        columns: MappedBuffer::zeroed::<u32>(&device, COLUMNS).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[1u32, 0, 1, 1, 1]).unwrap(),
        capacity: 1,
        generation: 0,
    };
    let input = make_frontier();
    let output = make_frontier();
    let captures = MappedBuffer::zeroed::<u32>(&device, 0).unwrap();
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    for _ in 0..50 {
        let _nested = LaunchScope::enter();
        let command = queue.new_command_buffer();
        let pending = encode_active(&device, command, &input, &output, &captures, 0, 0).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
    }
    let command = queue.new_command_buffer();
    let pending = encode_active(&device, command, &input, &output, &captures, 0, 0).unwrap();
    assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
    assert_eq!(pending.check_completed(), Err(Error::BackendLaunchLimit));
    let trace = scope.snapshot();
    assert_eq!(
        trace.iter().filter(|dispatch| dispatch.encoded).count(),
        600
    );
    assert_eq!(
        trace.iter().filter(|dispatch| dispatch.submitted).count(),
        600
    );
    assert_eq!(
        trace.iter().filter(|dispatch| dispatch.completed).count(),
        600
    );
    assert!(trace.iter().skip(600).all(|dispatch| !dispatch.encoded));
}

// Uses G's admission spike fixtures, which 05 does not copy (test spikes)
#[cfg(any())]
#[test]
fn unsupported_greedy_reconvergence_is_rejected_before_backend_selection() {
    let (program, _, _) = super::admission_spike::advancing_greedy_reconvergence();
    assert_eq!(
        program.with_tree_sitter_query_semantics().err(),
        Some(Error::InvalidGreedyRegion(4))
    );
}

#[path = "metal_production_dynamic_tests.rs"]
mod production;
