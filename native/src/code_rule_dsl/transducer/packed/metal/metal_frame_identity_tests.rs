//! Frame identity depends on complete values, not allocation addresses or hashes.
use super::Error;
use super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard, pipeline_named};
use super::metal_frame_identity::{self, CountedFrames, FrameIdentities};
use super::metal_parallel;
use metal::Device;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Frame {
    task: u32,
    kind: u32,
    values: Vec<[u32; 4]>,
}

fn input(device: &Device, frames: &[Frame]) -> CountedFrames {
    let mut values = Vec::new();
    let descriptors = frames
        .iter()
        .map(|frame| {
            let start = values.len() as u32;
            values.extend_from_slice(&frame.values);
            [frame.task, frame.kind, start, frame.values.len() as u32]
        })
        .chain(std::iter::repeat_n([u32::MAX; 4], 7))
        .collect::<Vec<_>>();
    let used = values.len();
    values.extend([[u32::MAX; 4]; 7]);
    CountedFrames {
        descriptors: MappedBuffer::from_slice(device, &descriptors).unwrap(),
        values: MappedBuffer::from_slice(device, &values).unwrap(),
        counts: MappedBuffer::from_slice(
            device,
            &[
                frames.len() as u32,
                used as u32,
                frames.len().div_ceil(256) as u32,
                1,
                1,
            ],
        )
        .unwrap(),
        capacity: descriptors.len(),
        value_capacity: values.len(),
        generation: 7,
    }
}

fn output(device: &Device, capacity: usize) -> FrameIdentities {
    FrameIdentities {
        ids: MappedBuffer::from_slice(device, &vec![u32::MAX; capacity]).unwrap(),
        counts: MappedBuffer::zeroed::<u32>(device, 5).unwrap(),
        capacity,
        generation: 7,
    }
}

fn check(frames: &[Frame], bucket_bits: u32) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let mut input = input(&device, frames);
    input.counts = MappedBuffer::zeroed::<u32>(&device, 5).unwrap();
    let output = output(&device, input.capacity);
    let queue = device.new_command_queue();
    let scope = LaunchScope::enter();
    let command = queue.new_command_buffer();
    let producer = pipeline_named(PIPELINE, "produce").unwrap();
    metal_parallel::encode(
        command,
        &producer,
        "frame_test_produce",
        &[&input.counts.buffer],
        &[
            frames.len() as u32,
            frames.iter().map(|f| f.values.len() as u32).sum(),
        ],
        1,
        None,
    );
    let pending =
        metal_frame_identity::encode(&device, command, &input, &output, bucket_bits).unwrap();
    let observed = MappedBuffer::from_slice(&device, &vec![u32::MAX; output.capacity]).unwrap();
    let consumer = pipeline_named(PIPELINE, "consume").unwrap();
    metal_parallel::encode(
        command,
        &consumer,
        "frame_test_consume",
        &[
            &output.ids.buffer,
            &output.counts.buffer,
            &input.descriptors.buffer,
            &input.values.buffer,
            &observed.buffer,
        ],
        &[],
        0,
        Some((&output.counts.buffer, 8)),
    );
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    pending.check_completed().unwrap();
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    assert_eq!(
        counts,
        [
            frames.len() as u32,
            0,
            frames.len().div_ceil(256) as u32,
            1,
            1
        ]
    );
    let ids = unsafe {
        std::slice::from_raw_parts(output.ids.buffer.contents().cast::<u32>(), output.capacity)
    };
    assert!(ids[frames.len()..].iter().all(|&id| id == u32::MAX));
    let observed = unsafe {
        std::slice::from_raw_parts(observed.buffer.contents().cast::<u32>(), output.capacity)
    };
    assert!(
        observed[frames.len()..]
            .iter()
            .all(|&value| value == u32::MAX)
    );
    let mut by_value = BTreeMap::new();
    let mut by_id = BTreeMap::new();
    for (i, frame) in frames.iter().enumerate() {
        assert_eq!(
            observed[i],
            frame.values.last().map_or(u32::MAX, |cell| cell[1])
        );
        if frame.values.is_empty() {
            assert_eq!(ids[i], 0);
        } else {
            assert!(ids[i] > 0 && ids[i] as usize <= frames.len());
            assert_eq!(&frames[ids[i] as usize - 1], frame);
            if let Some(prior) = by_value.insert(frame, ids[i]) {
                assert_eq!(prior, ids[i], "equal complete values share one identity");
            }
            if let Some(prior) = by_id.insert(ids[i], frame) {
                assert_eq!(prior, frame, "unequal complete values never share identity");
            }
        }
    }
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
        "FRAME_IDENTITIES frames={} launches={} peak_bytes={}",
        frames.len(),
        trace.len(),
        trace
            .iter()
            .map(|d| d.peak_application_buffer_bytes)
            .max()
            .unwrap()
    );
    trace.len()
}

#[test]
fn frame_identity_preserves_full_cursor_and_repeat_values_under_collisions() {
    let cursor = |values| Frame {
        task: 0,
        kind: 0,
        values,
    };
    let repeat = |values| Frame {
        task: 0,
        kind: 1,
        values,
    };
    let frames = vec![
        cursor(vec![]),
        repeat(vec![]),
        cursor(vec![[0, 2, 0, 0], [65535, 9, 0, 0]]),
        cursor(vec![[0, 2, 0, 0], [65535, 9, 0, 0]]),
        cursor(vec![[0, 2, 0, 0], [65535, 10, 0, 0]]),
        cursor(vec![[0, 2, 0, 0], [65534, 9, 0, 0]]),
        repeat(vec![[0, 2, 0, 0], [1, 9, 0, 0]]),
        repeat(vec![[0, 2, 0, 0], [1, 9, 0, 0]]),
        repeat(vec![[0, 9, 0, 0], [1, 2, 0, 0]]),
        cursor(vec![[0, 2, 0, 0], [1, 9, 0, 0]]),
        repeat(vec![[0, 2, 0, 0]]),
        repeat(vec![[0, 2, 0, 0], [1, 2, 0, 0]]),
        Frame {
            task: 1,
            kind: 0,
            values: vec![[0, 2, 0, 0], [65535, 9, 0, 0]],
        },
    ];
    check(&frames, 0);
    let long = cursor((0..8192).map(|i| [i, i + 1, 0, 0]).collect());
    let mut changed = long.clone();
    changed.values[8191][1] += 1;
    check(&[long.clone(), long, changed], 0);
    check(&[], 0);
}

#[test]
fn frame_identity_rejects_noncanonical_or_inconsistent_frames() {
    let device = Device::system_default().unwrap();
    let cases = [
        Frame {
            task: 0,
            kind: 0,
            values: vec![[3, 1, 0, 0], [2, 2, 0, 0]],
        },
        Frame {
            task: 0,
            kind: 0,
            values: vec![[3, 1, 0, 0], [3, 2, 0, 0]],
        },
        Frame {
            task: 0,
            kind: 0,
            values: vec![[65536, 1, 0, 0]],
        },
        Frame {
            task: 0,
            kind: 0,
            values: vec![[0, u32::MAX, 0, 0]],
        },
        Frame {
            task: 0,
            kind: 1,
            values: vec![[1, 1, 0, 0]],
        },
        Frame {
            task: 0,
            kind: 1,
            values: vec![[0, 65536, 0, 0]],
        },
        Frame {
            task: 0,
            kind: 2,
            values: vec![],
        },
        Frame {
            task: 0,
            kind: 0,
            values: vec![[0, 1, 1, 0]],
        },
    ];
    for frame in cases {
        let _guard = metal_execution_guard().unwrap();
        let input = input(&device, &[frame]);
        let output = output(&device, input.capacity);
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_frame_identity::encode(&device, command, &input, &output, 0).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
        let counts =
            unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
        assert_eq!(counts, [0, 0, 0, 1, 1]);
        let ids = unsafe {
            std::slice::from_raw_parts(output.ids.buffer.contents().cast::<u32>(), output.capacity)
        };
        assert!(ids.iter().all(|&id| id == u32::MAX));
    }
}

const PIPELINE: &str = r#"
#include <metal_stdlib>
using namespace metal;
kernel void produce(device uint* counts [[buffer(0)]],constant uint2& p [[buffer(1)]],uint tid [[thread_position_in_grid]]) {
 if(tid) return;
 counts[0]=p.x; counts[1]=p.y; counts[2]=(p.x+255u)/256u; counts[3]=counts[4]=1u;
}
kernel void consume(device const uint* ids [[buffer(0)]],device const uint* counts [[buffer(1)]],
 device const uint4* descriptors [[buffer(2)]],device const uint4* values [[buffer(3)]],
 device uint* observed [[buffer(4)]],uint tid [[thread_position_in_grid]]) {
 if(tid>=counts[0]) return;
 uint id=ids[tid];
 if(!id) { observed[tid]=0xffffffffu; return; }
 uint4 frame=descriptors[id-1u];
 observed[tid]=values[frame.z+frame.w-1u].y;
}
"#;

#[test]
fn frame_identity_scales_independent_segments_in_broad_stages() {
    let mut launches = Vec::new();
    for segments in [1, 128, 4096] {
        let frames = (0..segments)
            .flat_map(|task| {
                let frame = Frame {
                    task,
                    kind: 0,
                    values: vec![[0, 42, 0, 0], [9, 73, 0, 0]],
                };
                [frame.clone(), frame]
            })
            .collect::<Vec<_>>();
        launches.push(check(&frames, 13));
    }
    assert!(launches.iter().all(|&count| count > 0 && count <= 13));
    assert!(
        launches.iter().all(|&count| count == launches[0]),
        "independent segments must share the same bounded broad stages"
    );
}

#[test]
fn frame_identity_refuses_invalid_counts_ranges_and_capacity_without_publication() {
    for case in 0..9 {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let mut input = input(
            &device,
            &[Frame {
                task: 0,
                kind: 0,
                values: vec![[0, 42, 0, 0]],
            }],
        );
        let mut header = [1, 1, 1, 1, 1];
        match case {
            0 => header[0] = input.capacity as u32 + 1,
            1 => header[1] = input.value_capacity as u32 + 1,
            2 => header[2] = 2,
            3 => header[3] = 0,
            4 => header[4] = 2,
            5 => header[1] = 0,
            6 => {
                input.descriptors =
                    MappedBuffer::from_slice(&device, &[[0u32, 0, u32::MAX, 1]; 8]).unwrap()
            }
            7 => {
                input.descriptors =
                    MappedBuffer::from_slice(&device, &[[0u32, 0, 1, u32::MAX]; 8]).unwrap()
            }
            _ => (),
        }
        input.counts = MappedBuffer::from_slice(&device, &header).unwrap();
        let output = output(&device, if case == 8 { 0 } else { input.capacity });
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_frame_identity::encode(&device, command, &input, &output, 0).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(
            pending.check_completed(),
            Err(if case == 8 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            }),
            "case={case}"
        );
        let counts =
            unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
        assert_eq!(counts, [0, 0, 0, 1, 1]);
        let ids = unsafe {
            std::slice::from_raw_parts(output.ids.buffer.contents().cast::<u32>(), output.capacity)
        };
        assert!(ids.iter().all(|&id| id == u32::MAX));
    }
}

#[test]
fn frame_identity_checks_generation_buffers_and_command_completion() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let input = input(&device, &[]);
    let mut output = output(&device, input.capacity);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    output.generation += 1;
    assert!(matches!(
        metal_frame_identity::encode(&device, command, &input, &output, 0),
        Err(Error::BackendFailure)
    ));
    output.generation = input.generation;
    let distinct = output.ids;
    output.ids = MappedBuffer {
        buffer: input.descriptors.buffer.clone(),
    };
    assert!(matches!(
        metal_frame_identity::encode(&device, command, &input, &output, 0),
        Err(Error::BackendFailure)
    ));
    output.ids = distinct;
    let capacity = output.capacity;
    output.capacity = output.ids.buffer.length() as usize / 4 + 1;
    assert!(matches!(
        metal_frame_identity::encode(&device, command, &input, &output, 0),
        Err(Error::BackendFailure)
    ));
    output.capacity = capacity;
    assert!(scope.snapshot().is_empty());
    let pending = metal_frame_identity::encode(&device, command, &input, &output, 0).unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    assert!(
        scope
            .snapshot()
            .iter()
            .all(|d| !d.submitted && !d.completed)
    );
}

#[test]
fn frame_identity_cannot_reset_the_evaluation_launch_budget() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let input = input(
        &device,
        &[Frame {
            task: 0,
            kind: 1,
            values: vec![[0, 7, 0, 0]],
        }],
    );
    let output = output(&device, input.capacity);
    let scope = LaunchScope::enter();
    let queue = device.new_command_queue();
    for _ in 0..54 {
        let _nested = LaunchScope::enter();
        let command = queue.new_command_buffer();
        let pending = metal_frame_identity::encode(&device, command, &input, &output, 0).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        pending.check_completed().unwrap();
    }
    let command = queue.new_command_buffer();
    let pending = metal_frame_identity::encode(&device, command, &input, &output, 0).unwrap();
    assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
    assert_eq!(pending.check_completed(), Err(Error::BackendLaunchLimit));
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|d| d.encoded).count(), 600);
    assert_eq!(trace.iter().filter(|d| d.submitted).count(), 594);
    assert_eq!(trace.iter().filter(|d| d.completed).count(), 594);
    assert!(trace.iter().skip(600).all(|d| !d.encoded));
}

#[test]
fn frame_identity_refuses_pair_exhaustion_without_partial_ids() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let frames = vec![
        Frame {
            task: 0,
            kind: 1,
            values: vec![[0, 7, 0, 0]]
        };
        4097
    ];
    let input = input(&device, &frames);
    let output = output(&device, input.capacity);
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let pending = metal_frame_identity::encode(&device, command, &input, &output, 0).unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    assert_eq!(pending.check_completed(), Err(Error::BackendCapacity));
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 5) };
    assert_eq!(counts, [0, 0, 0, 1, 1]);
    let ids = unsafe {
        std::slice::from_raw_parts(output.ids.buffer.contents().cast::<u32>(), output.capacity)
    };
    assert!(ids.iter().all(|&id| id == u32::MAX));
}
