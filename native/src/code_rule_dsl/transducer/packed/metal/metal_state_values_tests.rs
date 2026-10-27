//! Independent complete-value oracle; storage addresses do not define equality.
use super::Error;
use super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard, pipeline_named};
use super::metal_parallel;
use super::metal_state_values::{self, *};
use metal::Device;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct State {
    key: [u32; KEY_FIELDS],
    cursors: Vec<[u32; 4]>,
    repeats: Vec<[u32; 4]>,
    captures: Vec<[u32; 4]>,
}

fn fixture(device: &Device, states: &[(State, u32)]) -> (ValueFrontier, StateValues, Vec<u32>) {
    let n = states.len();
    let mut columns = vec![u32::MAX; (n + 7) * COLUMNS];
    let mut cursors = Vec::new();
    let mut repeats = Vec::new();
    let mut captures = Vec::new();
    for (index, (state, ordinal)) in states.iter().enumerate() {
        for field in 0..KEY_FIELDS {
            columns[field * n + index] = state.key[field];
        }
        // Every state receives independent storage, including equal full values.
        for (field, count_field, arena, entries) in [
            (CURSOR_START, CURSOR_COUNT, &mut cursors, &state.cursors),
            (REPEAT_START, REPEAT_COUNT, &mut repeats, &state.repeats),
            (CAPTURE_START, CAPTURE_COUNT, &mut captures, &state.captures),
        ] {
            columns[field * n + index] = arena.len() as u32;
            columns[count_field * n + index] = entries.len() as u32;
            arena.extend_from_slice(entries);
        }
        columns[ORDINAL * n + index] = *ordinal;
    }
    let used = [cursors.len(), repeats.len(), captures.len()];
    for arena in [&mut cursors, &mut repeats, &mut captures] {
        arena.extend([[u32::MAX; 4]; 7]);
    }
    let input = ValueFrontier {
        columns: MappedBuffer::from_slice(device, &columns).unwrap(),
        counts: MappedBuffer::from_slice(
            device,
            &[n as u32, used[2] as u32, n.div_ceil(256) as u32, 1, 1],
        )
        .unwrap(),
        capacity: n + 7,
        generation: 7,
    };
    let values = StateValues {
        cursors: MappedBuffer::from_slice(device, &cursors).unwrap(),
        repeats: MappedBuffer::from_slice(device, &repeats).unwrap(),
        captures: MappedBuffer::from_slice(device, &captures).unwrap(),
        counts: MappedBuffer::from_slice(device, &[used[0] as u32, used[1] as u32]).unwrap(),
        cursor_capacity: cursors.len(),
        repeat_capacity: repeats.len(),
        capture_capacity: captures.len(),
        generation: 7,
    };
    (input, values, columns)
}

fn output(device: &Device, capacity: usize) -> ValueFrontier {
    ValueFrontier {
        columns: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity * COLUMNS]).unwrap(),
        counts: MappedBuffer::from_slice(device, &[u32::MAX; 5]).unwrap(),
        capacity,
        generation: 7,
    }
}

fn check(states: &[(State, u32)], bucket_bits: u32) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (mut input, mut values, source) = fixture(&device, states);
    input.counts = MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap();
    values.counts = MappedBuffer::from_slice(&device, &[u32::MAX; 2]).unwrap();
    let output = output(&device, input.capacity);
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    let command = queue.new_command_buffer();
    let producer = pipeline_named(PIPELINE, "produce_value_lengths").unwrap();
    metal_parallel::encode(
        command,
        &producer,
        "produce_value_lengths",
        &[&input.counts.buffer, &values.counts.buffer],
        &[
            states.len() as u32,
            states.iter().map(|(s, _)| s.cursors.len() as u32).sum(),
            states.iter().map(|(s, _)| s.repeats.len() as u32).sum(),
            states.iter().map(|(s, _)| s.captures.len() as u32).sum(),
        ],
        1,
        None,
    );
    let pending =
        metal_state_values::encode(&device, command, &input, &output, &values, bucket_bits)
            .unwrap();
    let consumed = MappedBuffer::from_slice(&device, &vec![[u32::MAX; 3]; input.capacity]).unwrap();
    let consumer = pipeline_named(PIPELINE, "consume_state_values").unwrap();
    metal_parallel::encode(
        command,
        &consumer,
        "consume_state_values",
        &[
            &output.columns.buffer,
            &output.counts.buffer,
            &values.cursors.buffer,
            &values.repeats.buffer,
            &consumed.buffer,
        ],
        &[],
        0,
        Some((&output.counts.buffer, 8)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    let mut chosen = BTreeMap::new();
    for (index, (state, ordinal)) in states.iter().enumerate() {
        // Ordinary occurrences remain distinct even when their values match.
        let occurrence = matches!(state.key[5], 2 | 3 | 5).then_some(index);
        let entry = chosen
            .entry((state, occurrence))
            .or_insert((*ordinal, index));
        *entry = (*entry).min((*ordinal, index));
    }
    let mut expected = chosen
        .into_values()
        .map(|(_, index)| index)
        .collect::<Vec<_>>();
    expected.sort_unstable();
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    let captures: usize = states.iter().map(|(s, _)| s.captures.len()).sum();
    assert_eq!(
        counts,
        [
            expected.len() as u32,
            captures as u32,
            expected.len().div_ceil(256) as u32,
            1,
            1
        ]
    );
    let actual = unsafe {
        std::slice::from_raw_parts(
            output.columns.buffer.contents().cast::<u32>(),
            output.capacity * COLUMNS,
        )
    };
    for (out, &original) in expected.iter().enumerate() {
        for field in 0..COLUMNS {
            assert_eq!(
                actual[field * expected.len() + out],
                source[field * states.len() + original],
                "complete survivor column {field}"
            );
        }
    }
    assert!(
        actual[expected.len() * COLUMNS..]
            .iter()
            .all(|&value| value == u32::MAX)
    );
    let consumed = unsafe {
        std::slice::from_raw_parts(
            consumed.buffer.contents().cast::<[u32; 3]>(),
            input.capacity,
        )
    };
    for (out, &original) in expected.iter().enumerate() {
        let (state, ordinal) = &states[original];
        assert_eq!(
            consumed[out],
            [
                *ordinal,
                state.cursors.last().map_or(u32::MAX, |c| c[1]),
                state.repeats.last().map_or(u32::MAX, |r| r[1])
            ]
        );
    }
    assert!(
        consumed[expected.len()..]
            .iter()
            .all(|&item| item == [u32::MAX; 3])
    );
    let trace = scope.snapshot();
    assert_eq!(
        trace
            .iter()
            .map(|d| d.command)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert!(
        trace
            .iter()
            .all(|d| d.peak_application_buffer_bytes <= super::metal_backend::METAL_BUFFER_LIMIT)
    );
    assert!(
        trace
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
    eprintln!(
        "STATE_VALUES states={} survivors={} launches={} peak_bytes={}",
        states.len(),
        expected.len(),
        trace.len(),
        trace
            .iter()
            .map(|d| d.peak_application_buffer_bytes)
            .max()
            .unwrap()
    );
    trace.len()
}

fn base() -> State {
    State {
        key: [3, 4, 5, 0, 2, 0],
        cursors: vec![[0, 7, 0, 0], [65535, 8, 0, 0]],
        repeats: vec![[0, 2, 0, 0], [1, 9, 0, 0]],
        captures: vec![[7, 1, 0, 4], [8, 2, 0, 4]],
    }
}

#[test]
fn state_values_compare_complete_frames_and_captures_under_collisions() {
    let base = base();
    let mut cases = vec![(base.clone(), 19), (base.clone(), 3)];
    for field in 0..KEY_FIELDS {
        let mut changed = base.clone();
        changed.key[field] += 1;
        cases.push((changed, 31));
    }
    let mut changed = base.clone();
    changed.cursors[1][0] -= 1;
    cases.push((changed, 33));
    let mut changed = base.clone();
    changed.cursors[1][1] += 1;
    cases.push((changed, 35));
    let mut changed = base.clone();
    changed.repeats[0][1] = 9;
    changed.repeats[1][1] = 2;
    cases.push((changed, 37));
    let mut changed = base.clone();
    changed.repeats.pop();
    cases.push((changed, 39));
    let mut changed = base.clone();
    changed.captures.reverse();
    cases.push((changed, 41));
    let mut changed = base.clone();
    changed.captures.push(changed.captures[1]);
    cases.push((changed, 43));
    let mut changed = base.clone();
    changed.captures[1][0] += 1;
    cases.push((changed, 45));
    let mut changed = base;
    changed.captures[1][1] += 1;
    cases.push((changed, 47));
    check(&cases, 0);
}

const PIPELINE: &str = r#"
#include <metal_stdlib>
using namespace metal;
kernel void produce_value_lengths(device uint* counts [[buffer(0)]],device uint* lengths [[buffer(1)]],
 constant uint4& p [[buffer(2)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 counts[0]=p.x; counts[1]=p.w; counts[2]=(p.x+255u)/256u; counts[3]=counts[4]=1u;
 lengths[0]=p.y; lengths[1]=p.z;
}
kernel void consume_state_values(device const uint* states [[buffer(0)]],device const uint* counts [[buffer(1)]],
 device const uint4* cursors [[buffer(2)]],device const uint4* repeats [[buffer(3)]],device uint* observed [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 uint n=counts[0]; if(tid>=n) return;
 uint cs=states[6u*n+tid],cn=states[7u*n+tid],rs=states[8u*n+tid],rn=states[9u*n+tid];
 observed[3u*tid]=states[12u*n+tid];
 observed[3u*tid+1u]=cn?cursors[cs+cn-1u].y:0xffffffffu;
 observed[3u*tid+2u]=rn?repeats[rs+rn-1u].y:0xffffffffu;
}
"#;

#[test]
fn state_values_preserve_long_frames_and_empty_values() {
    let mut state = base();
    state.cursors = (0..8192).map(|i| [i, i + 1, 0, 0]).collect();
    state.repeats = (0..8192).map(|i| [i, i % 13, 0, 0]).collect();
    state.captures = (0..8192).map(|i| [i, 1, i * 2, i * 2 + 1]).collect();
    let mut cases = vec![(state.clone(), 10), (state.clone(), 2)];
    let mut changed = state.clone();
    changed.cursors[8191][1] += 1;
    cases.push((changed, 20));
    let mut changed = state.clone();
    changed.repeats[8191][1] += 1;
    cases.push((changed, 30));
    let mut changed = state;
    changed.captures[8191][0] += 1;
    cases.push((changed, 40));
    check(&cases, 0);
    let empty = State {
        key: [0; KEY_FIELDS],
        cursors: vec![],
        repeats: vec![],
        captures: vec![],
    };
    check(&[(empty.clone(), 6), (empty, 3)], 0);
    check(&[], 0);
}

#[test]
fn state_values_scale_independent_segments_in_broad_stages() {
    let mut launches = Vec::new();
    for segments in [1, 128, 4096] {
        let states = (0..segments)
            .flat_map(|task| {
                let mut state = base();
                state.key[0] = task;
                [(state.clone(), 9), (state, 3)]
            })
            .collect::<Vec<_>>();
        launches.push(check(&states, 13));
    }
    assert!(launches.iter().all(|&count| count > 0 && count <= 14));
    assert!(
        launches.iter().all(|&count| count == launches[0]),
        "independent segments must share the same bounded broad stages"
    );
}

fn refused(
    input: &ValueFrontier,
    values: &StateValues,
    output: &ValueFrontier,
    device: &Device,
    error: Error,
) {
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending = metal_state_values::encode(device, command, input, output, values, 0).unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(pending.check_completed(), Err(error.clone()));
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    assert_eq!(counts, [0, 0, 0, 1, 1]);
    let columns = unsafe {
        std::slice::from_raw_parts(
            output.columns.buffer.contents().cast::<u32>(),
            output.capacity * COLUMNS,
        )
    };
    assert!(columns.iter().all(|&v| v == u32::MAX));
    groups_refused(input, values, output.capacity, device, error);
}

#[test]
fn state_values_reject_malformed_values_without_partial_publication() {
    for case in 0..12 {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let mut state = base();
        match case {
            0 => state.cursors[1][0] = 0,
            1 => state.cursors[1][0] = 65536,
            2 => state.cursors[1][1] = u32::MAX,
            3 => state.cursors[1][2] = 1,
            4 => state.repeats[1][0] = 0,
            5 => state.repeats[1][1] = 65536,
            6 => state.repeats[1][3] = 1,
            7 => state.captures[1][0] = u32::MAX,
            8 => state.captures[1][1] = 65536,
            9 => state.captures[1][2] = 5,
            10 => state.captures[1][3] = 5, // Earlier geometry must precede the first capture.
            11 => state.key[5] = 6,         // Unknown continuation kinds cannot enter execution.
            _ => unreachable!(),
        }
        let (input, values, _) = fixture(&device, &[(state, 1)]);
        refused(
            &input,
            &values,
            &output(&device, input.capacity),
            &device,
            Error::BackendFailure,
        );
    }
}

#[test]
fn state_values_reject_inconsistent_lengths_ranges_and_capacity() {
    for case in 0..13 {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let (input, values, _) = fixture(&device, &[(base(), 1)]);
        unsafe {
            let header = input.counts.buffer.contents().cast::<u32>();
            let lengths = values.counts.buffer.contents().cast::<u32>();
            let columns = input.columns.buffer.contents().cast::<u32>();
            match case {
                0 => header.write(input.capacity as u32 + 1),
                1 => header.add(1).write(values.capture_capacity as u32 + 1),
                2 => header.add(2).write(2),
                3 => header.add(3).write(0),
                4 => header.add(4).write(0),
                5 => lengths.write(values.cursor_capacity as u32 + 1),
                6 => lengths.add(1).write(values.repeat_capacity as u32 + 1),
                7 => columns.add(CURSOR_START).write(u32::MAX),
                8 => columns.add(REPEAT_COUNT).write(u32::MAX),
                9 => columns.add(CAPTURE_START).write(u32::MAX),
                10 => lengths.write(1), // Allocated but invalid tail cannot satisfy the span.
                11 => lengths.add(1).write(1),
                _ => (),
            }
        }
        refused(
            &input,
            &values,
            &output(&device, if case == 12 { 0 } else { input.capacity }),
            &device,
            if case == 12 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            },
        );
    }
}

#[test]
fn state_values_require_distinct_buffers_generation_and_completion() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (input, mut values, _) = fixture(&device, &[(base(), 1)]);
    let mut output = output(&device, input.capacity);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    output.generation += 1;
    assert!(matches!(
        metal_state_values::encode(&device, command, &input, &output, &values, 0),
        Err(Error::BackendFailure)
    ));
    output.generation = input.generation;
    values.generation += 1;
    assert!(matches!(
        metal_state_values::encode(&device, command, &input, &output, &values, 0),
        Err(Error::BackendFailure)
    ));
    values.generation = input.generation;
    let distinct = values.repeats;
    values.repeats = MappedBuffer {
        buffer: values.cursors.buffer.clone(),
    };
    assert!(matches!(
        metal_state_values::encode(&device, command, &input, &output, &values, 0),
        Err(Error::BackendFailure)
    ));
    values.repeats = distinct;
    let capacity = values.cursor_capacity;
    values.cursor_capacity = values.cursors.buffer.length() as usize / 16 + 1;
    assert!(matches!(
        metal_state_values::encode(&device, command, &input, &output, &values, 0),
        Err(Error::BackendFailure)
    ));
    values.cursor_capacity = capacity;
    assert!(scope.snapshot().is_empty());
    let pending =
        metal_state_values::encode(&device, command, &input, &output, &values, 0).unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|d| !d.submitted && !d.completed)
    );
}

#[test]
fn state_values_refuse_pair_exhaustion_without_partial_results() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (input, values, _) = fixture(&device, &vec![(base(), 1); 4097]);
    refused(
        &input,
        &values,
        &output(&device, input.capacity),
        &device,
        Error::BackendCapacity,
    );
}

#[test]
fn state_values_cannot_reset_the_evaluation_launch_budget() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (input, values, _) = fixture(&device, &[(base(), 1)]);
    let output = output(&device, input.capacity);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    for _ in 0..50 {
        let _nested = LaunchScope::enter();
        let command = queue.new_command_buffer();
        let pending =
            metal_state_values::encode(&device, command, &input, &output, &values, 0).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
    }
    let command = queue.new_command_buffer();
    let pending =
        metal_state_values::encode(&device, command, &input, &output, &values, 0).unwrap();
    assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
    assert_eq!(pending.check_completed(), Err(Error::BackendLaunchLimit));
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|d| d.encoded).count(), 600);
    assert_eq!(trace.iter().filter(|d| d.submitted).count(), 600);
    assert_eq!(trace.iter().filter(|d| d.completed).count(), 600);
    assert!(trace.iter().skip(600).all(|d| !d.encoded));
}

#[test]
fn state_values_preserve_ordinary_occurrence_multiplicity() {
    // Active, completed, and named-navigation continuations of ordinary work
    // preserve each occurrence. Query admission remains exact-value reduction.
    for kind in [2, 3, 5] {
        let mut state = base();
        state.key[5] = kind;
        check(&[(state.clone(), 19), (state.clone(), 3), (state, 7)], 0);
    }
}

#[test]
fn state_values_apply_admission_per_continuation_in_mixed_batches() {
    let mut mixed = Vec::new();
    for kind in 0..6 {
        let mut state = base();
        state.key[5] = kind;
        mixed.extend([(state.clone(), 9), (state.clone(), 3), (state, 7)]);
    }
    check(&mixed, 0);
    for segments in [1, 128, 4096] {
        let states = (0..segments)
            .flat_map(|task| {
                let mut state = base();
                state.key[0] = task;
                state.key[5] = 2;
                [(state.clone(), 9), (state, 3)]
            })
            .collect::<Vec<_>>();
        let launches = check(&states, 13);
        assert!(launches > 0 && launches <= 14);
    }
}

const GROUP_CONSUMER: &str = r#"
#include <metal_stdlib>
using namespace metal;
kernel void consume_value_groups(device const uint* groups [[buffer(0)]],device const uint* counts [[buffer(1)]],
 device const uint* states [[buffer(2)]],device const uint* source_counts [[buffer(3)]],device uint4* output [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=counts[0]) return;
 uint rep=groups[tid],n=source_counts[0];
 output[tid]=rep<n?uint4(rep,states[12u*n+rep],states[2u*n+rep],states[11u*n+rep]):uint4(0xffffffffu);
}
"#;

fn check_groups(states: &[(State, u32)], bucket_bits: u32) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (mut input, mut values, _) = fixture(&device, states);
    input.counts = MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap();
    values.counts = MappedBuffer::from_slice(&device, &[u32::MAX; 2]).unwrap();
    let output = ValueGroups {
        representatives: MappedBuffer::from_slice(&device, &vec![u32::MAX; input.capacity])
            .unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 5]).unwrap(),
        capacity: input.capacity,
        generation: input.generation,
    };
    let consumer = MappedBuffer::from_slice(&device, &vec![[u32::MAX; 4]; input.capacity]).unwrap();
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let captures = states
        .iter()
        .map(|(s, _)| s.captures.len() as u32)
        .sum::<u32>();
    metal_parallel::encode(
        command,
        &pipeline_named(PIPELINE, "produce_value_lengths").unwrap(),
        "produce_value_lengths",
        &[&input.counts.buffer, &values.counts.buffer],
        &[
            states.len() as u32,
            states.iter().map(|(s, _)| s.cursors.len() as u32).sum(),
            states.iter().map(|(s, _)| s.repeats.len() as u32).sum(),
            captures,
        ],
        1,
        None,
    );
    let pending =
        metal_state_values::encode_groups(&device, command, &input, &output, &values, bucket_bits)
            .unwrap();
    metal_parallel::encode(
        command,
        &pipeline_named(GROUP_CONSUMER, "consume_value_groups").unwrap(),
        "consume_value_groups",
        &[
            &output.representatives.buffer,
            &output.counts.buffer,
            &input.columns.buffer,
            &input.counts.buffer,
            &consumer.buffer,
        ],
        &[],
        0,
        Some((&output.counts.buffer, 8)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    let mut representatives = BTreeMap::new();
    let expected = states
        .iter()
        .enumerate()
        .map(|(index, (state, _))| {
            let occurrence = matches!(state.key[5], 2 | 3 | 5).then_some(index);
            *representatives
                .entry((state, occurrence))
                .or_insert(index as u32)
        })
        .collect::<Vec<_>>();
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    assert_eq!(
        counts,
        [
            states.len() as u32,
            captures,
            states.len().div_ceil(256) as u32,
            1,
            1
        ]
    );
    let actual = unsafe {
        std::slice::from_raw_parts(
            output.representatives.buffer.contents().cast::<u32>(),
            output.capacity,
        )
    };
    assert_eq!(&actual[..states.len()], expected);
    assert!(actual[states.len()..].iter().all(|&v| v == u32::MAX));
    let consumed = unsafe {
        std::slice::from_raw_parts(
            consumer.buffer.contents().cast::<[u32; 4]>(),
            output.capacity,
        )
    };
    for (index, &rep) in expected.iter().enumerate() {
        let (state, ordinal) = &states[rep as usize];
        assert_eq!(
            consumed[index],
            [rep, *ordinal, state.key[2], state.captures.len() as u32]
        );
    }
    assert!(consumed[states.len()..].iter().all(|&v| v == [u32::MAX; 4]));
    let trace = scope.snapshot();
    assert_eq!(
        trace
            .iter()
            .map(|d| d.command)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert!(
        trace
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
    assert!(trace.len() <= 18);
    eprintln!(
        "VALUE_GROUPS {}",
        serde_json::json!({"states":states.len(),"groups":representatives.len(),"launchesIncludingProducerConsumer":trace.len(),"peakMetalBytes":trace.iter().map(|d|d.peak_application_buffer_bytes).max().unwrap()})
    );
    trace.len()
}

#[test]
fn state_value_groups_preserve_complete_membership_and_ordinary_occurrences_under_collisions() {
    let base = base();
    let mut states = vec![(base.clone(), 99), (base.clone(), 3)];
    for field in 0..KEY_FIELDS {
        let mut different = base.clone();
        different.key[field] += 1;
        states.push((different, 7));
    }
    let mut different = base.clone();
    different.cursors[1][1] += 1;
    states.push((different, 8));
    let mut different = base.clone();
    different.repeats[0][1] += 1;
    states.push((different, 9));
    let mut different = base.clone();
    different.captures.reverse();
    states.push((different, 10));
    let mut different = base.clone();
    different.captures.push(different.captures[1]);
    states.push((different, 11));
    let mut different = base.clone();
    different.captures[1][0] += 1;
    states.push((different, 12));
    for kind in [2, 3, 5] {
        let mut ordinary = base.clone();
        ordinary.key[5] = kind;
        states.extend([(ordinary.clone(), 0), (ordinary, 0)]);
    }
    let mut long = base.clone();
    long.cursors = (0..256).map(|i| [i * 100, i + 1, 0, 0]).collect();
    long.repeats = (0..512).map(|i| [i, i % 255, 0, 0]).collect();
    long.captures = (0..768).map(|i| [i + 1, 1, i, i + 1]).collect();
    states.extend([(long.clone(), 23), (long.clone(), 5)]);
    long.captures[767][0] += 1;
    states.push((long, 17));
    let empty = State {
        cursors: vec![],
        repeats: vec![],
        captures: vec![],
        ..base
    };
    states.extend([(empty.clone(), 13), (empty, 2)]);
    check_groups(&states, 0);
}

fn group_output(device: &Device, capacity: usize) -> ValueGroups {
    ValueGroups {
        representatives: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity.max(1)])
            .unwrap(),
        counts: MappedBuffer::from_slice(device, &[u32::MAX; 5]).unwrap(),
        capacity,
        generation: 7,
    }
}

fn groups_refused(
    input: &ValueFrontier,
    values: &StateValues,
    capacity: usize,
    device: &Device,
    error: Error,
) {
    let output = group_output(device, capacity);
    let consumed = MappedBuffer::from_slice(device, &vec![[u32::MAX; 4]; capacity.max(1)]).unwrap();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending =
        metal_state_values::encode_groups(device, command, input, &output, values, 0).unwrap();
    metal_parallel::encode(
        command,
        &pipeline_named(GROUP_CONSUMER, "consume_value_groups").unwrap(),
        "consume_value_groups",
        &[
            &output.representatives.buffer,
            &output.counts.buffer,
            &input.columns.buffer,
            &input.counts.buffer,
            &consumed.buffer,
        ],
        &[],
        0,
        Some((&output.counts.buffer, 8)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(pending.check_completed(), Err(error));
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    assert_eq!(counts, [0, 0, 0, 1, 1]);
    let actual = unsafe {
        std::slice::from_raw_parts(
            output.representatives.buffer.contents().cast::<u32>(),
            capacity.max(1),
        )
    };
    assert!(actual.iter().all(|&v| v == u32::MAX));
    let actual = unsafe {
        std::slice::from_raw_parts(
            consumed.buffer.contents().cast::<[u32; 4]>(),
            capacity.max(1),
        )
    };
    assert!(actual.iter().all(|&v| v == [u32::MAX; 4]));
}

#[test]
fn state_value_groups_scale_independent_segments_with_bounded_broad_stages() {
    let mut launches = Vec::new();
    for segments in [1u32, 128, 4096] {
        let states = (0..segments)
            .flat_map(|task| {
                let mut s = base();
                s.key[0] = task;
                [(s.clone(), 9), (s, 3)]
            })
            .collect::<Vec<_>>();
        launches.push(check_groups(&states, 13));
    }
    assert!(launches.iter().all(|&count| count > 0 && count <= 13));
    assert!(
        launches.iter().all(|&count| count == launches[0]),
        "independent groups must share the same bounded broad stages"
    );
}

#[test]
fn state_value_groups_publish_empty_and_singleton_membership_without_tail_access() {
    check_groups(&[], 0);
    check_groups(&[(base(), 17)], 0);
    let empty = State {
        cursors: vec![],
        repeats: vec![],
        captures: vec![],
        ..base()
    };
    check_groups(&[(empty.clone(), 5), (empty, 1)], 0);
}

#[test]
fn state_value_groups_require_matching_storage_generation_and_completion() {
    let device = Device::system_default().unwrap();
    for case in 0..7 {
        let _guard = metal_execution_guard().unwrap();
        let (mut input, mut values, _) = fixture(&device, &[(base(), 1)]);
        let mut output = group_output(&device, input.capacity);
        match case {
            0 => output.generation += 1,
            1 => values.generation += 1,
            2 => output.representatives.buffer = input.columns.buffer.clone(),
            3 => output.counts.buffer = values.counts.buffer.clone(),
            4 => output.capacity = usize::MAX,
            5 => input.capacity = input.columns.buffer.length() as usize / (COLUMNS * 4) + 1,
            6 => values.cursor_capacity = values.cursors.buffer.length() as usize / 16 + 1,
            _ => unreachable!(),
        }
        let scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        assert_eq!(
            metal_state_values::encode_groups(&device, command, &input, &output, &values, 0).err(),
            Some(if case == 4 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            })
        );
        assert!(scope.snapshot().is_empty());
    }
    let _guard = metal_execution_guard().unwrap();
    let (input, values, _) = fixture(&device, &[(base(), 1)]);
    let output = group_output(&device, input.capacity);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending =
        metal_state_values::encode_groups(&device, command, &input, &output, &values, 0).unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|d| !d.submitted && !d.completed)
    );
}

#[test]
fn state_value_groups_cannot_reset_the_evaluation_launch_budget() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let (input, values, _) = fixture(&device, &[(base(), 1)]);
    let output = group_output(&device, input.capacity);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    for _ in 0..54 {
        let _nested = LaunchScope::enter();
        let command = queue.new_command_buffer();
        let pending =
            metal_state_values::encode_groups(&device, command, &input, &output, &values, 0)
                .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
    }
    let command = queue.new_command_buffer();
    let pending =
        metal_state_values::encode_groups(&device, command, &input, &output, &values, 0).unwrap();
    assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
    assert_eq!(pending.check_completed(), Err(Error::BackendLaunchLimit));
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|d| d.encoded).count(), 600);
    assert_eq!(trace.iter().filter(|d| d.completed).count(), 594);
    assert!(
        trace
            .iter()
            .skip(600)
            .all(|d| !d.encoded && !d.submitted && !d.completed)
    );
}
