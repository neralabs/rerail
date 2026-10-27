//! Independent serial first-visit oracle and same-command GPU composition.
use super::Error;
use super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::metal_backend::{MappedBuffer, metal_execution_guard, pipeline_named};
use super::metal_bulk_closure::{self, ClosureExits, ClosureRanges};
use super::metal_parallel;
use metal::Device;
use std::collections::BTreeSet;

const IO: &str = r#"
#include <metal_stdlib>
using namespace metal;
kernel void closure_test_produce(device uint* output [[buffer(0)]],constant uint3& counts [[buffer(1)]],uint tid [[thread_position_in_grid]]) {
 if(tid==0u) { output[0]=counts.x; output[1]=counts.y; output[2]=counts.z; }
}
kernel void closure_test_consume(device const uint2* input [[buffer(0)]],device const uint* counts [[buffer(1)]],device uint2* output [[buffer(2)]],uint tid [[thread_position_in_grid]]) {
 if(tid<counts[0]) output[tid]=input[tid];
}
"#;

fn oracle(domains: &[[u32; 2]], entries: &[[u32; 2]], allowed: &[u32]) -> Vec<[u32; 2]> {
    let mut visited = vec![false; allowed.len()];
    let mut output = Vec::new();
    for (priority, &[domain, offset]) in entries.iter().enumerate() {
        let [first, count] = domains[domain as usize];
        let mut at = (first + offset) as usize;
        if visited[at] {
            continue;
        }
        loop {
            visited[at] = true;
            let next = at + 1;
            if next == (first + count) as usize || allowed[next] == 0 || visited[next] {
                break;
            }
            at = next;
        }
        output.push([priority as u32, at as u32]);
    }
    output
}

fn check(domains: &[[u32; 2]], entries: &[[u32; 2]], allowed: &[u32]) -> usize {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let scope = LaunchScope::enter();
    let mut padded_domains = domains.to_vec();
    padded_domains.extend([[u32::MAX; 2]; 7]);
    let mut padded_entries = entries.to_vec();
    padded_entries.extend([[u32::MAX; 2]; 7]);
    let mut padded_allowed = allowed.to_vec();
    padded_allowed.extend([u32::MAX; 7]);
    let input = ClosureRanges {
        domains: MappedBuffer::from_slice(&device, &padded_domains).unwrap(),
        entries: MappedBuffer::from_slice(&device, &padded_entries).unwrap(),
        allowed: MappedBuffer::from_slice(&device, &padded_allowed).unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 3]).unwrap(),
        domain_capacity: padded_domains.len(),
        entry_capacity: padded_entries.len(),
        cell_capacity: padded_allowed.len(),
        generation: 9,
    };
    let output = ClosureExits {
        exits: MappedBuffer::from_slice(&device, &vec![[u32::MAX; 2]; input.entry_capacity])
            .unwrap(),
        counts: MappedBuffer::from_slice(&device, &[u32::MAX; 4]).unwrap(),
        capacity: input.entry_capacity,
        generation: 9,
    };
    let consumer =
        MappedBuffer::from_slice(&device, &vec![[u32::MAX; 2]; output.capacity]).unwrap();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    metal_parallel::encode(
        command,
        &pipeline_named(IO, "closure_test_produce").unwrap(),
        "closure_test_produce",
        &[&input.counts.buffer],
        &[
            domains.len() as u32,
            entries.len() as u32,
            allowed.len() as u32,
            0,
        ],
        1,
        None,
    );
    let pending = metal_bulk_closure::encode(&device, command, &input, &output).unwrap();
    metal_parallel::encode(
        command,
        &pipeline_named(IO, "closure_test_consume").unwrap(),
        "closure_test_consume",
        &[
            &output.exits.buffer,
            &output.counts.buffer,
            &consumer.buffer,
        ],
        &[],
        0,
        Some((&output.counts.buffer, 4)),
    );
    let submitted = std::time::Instant::now();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
    let submission_seconds = submitted.elapsed().as_secs_f64();
    pending.check_completed().unwrap();
    let expected = oracle(domains, entries, allowed);
    let counts =
        unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 4) };
    assert_eq!(
        counts,
        [
            expected.len() as u32,
            expected.len().div_ceil(256) as u32,
            1,
            1
        ]
    );
    for buffer in [&output.exits, &consumer] {
        let actual = unsafe {
            std::slice::from_raw_parts(buffer.buffer.contents().cast::<[u32; 2]>(), output.capacity)
        };
        assert_eq!(&actual[..expected.len()], expected);
        assert!(actual[expected.len()..].iter().all(|&v| v == [u32::MAX; 2]));
    }
    let trace = scope.snapshot();
    assert!(
        trace
            .iter()
            .all(|d| d.encoded && d.submitted && d.completed)
    );
    assert_eq!(
        trace
            .iter()
            .map(|d| d.command)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert!(
        trace.len() <= 24,
        "too many closure launches: {}",
        trace.len()
    );
    eprintln!(
        "GPU_CLOSURE {}",
        serde_json::json!({"domains":domains.len(),"entries":entries.len(),"cells":allowed.len(),"survivors":expected.len(),"launchesIncludingProducerConsumer":trace.len(),"submissionCompletionSeconds":submission_seconds,"peakMetalBytes":trace.iter().map(|d|d.peak_application_buffer_bytes).max().unwrap()})
    );
    trace.len()
}

#[test]
fn gpu_bulk_closure_preserves_exact_overlapping_exits_in_bounded_broad_stages() {
    for (groups, launches) in [(1u32, 14), (128, 20), (4096, 20)] {
        let domains = (0..groups).map(|i| [i * 8, 8]).collect::<Vec<_>>();
        let entries = (0..groups)
            .rev()
            .flat_map(|domain| [5, 2, 5, 0].map(|offset| [domain, offset]))
            .collect::<Vec<_>>();
        let actual = check(&domains, &entries, &vec![1; groups as usize * 8]);
        assert_eq!(actual, launches);
    }
}

#[test]
fn gpu_bulk_closure_matches_all_short_ordered_entries_and_guard_partitions() {
    let mut domains = Vec::new();
    let mut entries = Vec::new();
    let mut allowed = Vec::new();
    for len in 1..=5u32 {
        for mask in 0..(1u32 << len) {
            for count in 0..=3u32 {
                for mut encoded in 0..len.pow(count) {
                    let domain = domains.len() as u32;
                    domains.push([allowed.len() as u32, len]);
                    allowed.extend((0..len).map(|i| u32::from(mask & (1 << i) != 0)));
                    for _ in 0..count {
                        entries.push([domain, encoded % len]);
                        encoded /= len;
                    }
                }
            }
        }
    }
    assert_eq!(domains.len(), 6740);
    check(&domains, &entries, &allowed);
}

#[test]
fn gpu_bulk_closure_preserves_boundaries_empty_inputs_and_deeper_scan_levels() {
    check(&[], &[], &[]);
    for n in [1u32, 255, 256, 257, 511, 1024, 65535, 65536, 65537] {
        let domains = [[0, n]];
        let entries = (0..n)
            .rev()
            .step_by(29)
            .map(|offset| [0, offset])
            .collect::<Vec<_>>();
        let allowed = (0..n)
            .map(|i| u32::from(i % 127 != 0 && i % 257 != 0))
            .collect::<Vec<_>>();
        check(&domains, &entries, &allowed);
        // No entry is also a valid empty result over a nonempty resident range.
        if n == 257 {
            check(&domains, &[], &allowed);
        }
    }
}

#[test]
fn gpu_bulk_closure_compacts_a_late_survivor_across_three_entry_scan_levels() {
    // The last entry owns the first cell. All preceding duplicates except the
    // first are suppressed, so the late survivor must receive compact slot 1.
    let mut entries = vec![[0u32, 1]; 65536];
    entries.push([0, 0]);
    assert_eq!(oracle(&[[0, 2]], &entries, &[1, 1]), [[0, 1], [65536, 0]]);
    check(&[[0, 2]], &entries, &[1, 1]);
}

fn fixture(device: &Device) -> (ClosureRanges, ClosureExits) {
    (
        ClosureRanges {
            domains: MappedBuffer::from_slice(device, &[[0u32, 4], [4, 4]]).unwrap(),
            entries: MappedBuffer::from_slice(device, &[[0u32, 2], [0, 0], [1, 0]]).unwrap(),
            allowed: MappedBuffer::from_slice(device, &[1u32; 8]).unwrap(),
            counts: MappedBuffer::from_slice(device, &[2u32, 3, 8]).unwrap(),
            domain_capacity: 2,
            entry_capacity: 3,
            cell_capacity: 8,
            generation: 9,
        },
        ClosureExits {
            exits: MappedBuffer::from_slice(device, &[[u32::MAX; 2]; 4]).unwrap(),
            counts: MappedBuffer::from_slice(device, &[u32::MAX; 4]).unwrap(),
            capacity: 4,
            generation: 9,
        },
    )
}

#[test]
fn gpu_bulk_closure_rejects_invalid_ranges_lengths_guards_and_capacity_without_output() {
    let device = Device::system_default().unwrap();
    for case in 0..14 {
        // Each malformed request is an independent evaluation.
        let _guard = metal_execution_guard().unwrap();
        let (mut input, mut output) = fixture(&device);
        match case {
            0 => input.domains = MappedBuffer::from_slice(&device, &[[1u32, 3], [4, 4]]).unwrap(),
            1 => input.domains = MappedBuffer::from_slice(&device, &[[0u32, 5], [4, 4]]).unwrap(),
            2 => input.domains = MappedBuffer::from_slice(&device, &[[0u32, 3], [4, 4]]).unwrap(),
            3 => input.domains = MappedBuffer::from_slice(&device, &[[0u32, 4], [4, 0]]).unwrap(),
            4 => {
                input.domains =
                    MappedBuffer::from_slice(&device, &[[0u32, 4], [4, u32::MAX]]).unwrap()
            }
            5 => {
                input.entries =
                    MappedBuffer::from_slice(&device, &[[0u32, 4], [0, 0], [1, 0]]).unwrap()
            }
            6 => {
                input.entries =
                    MappedBuffer::from_slice(&device, &[[2u32, 0], [0, 0], [1, 0]]).unwrap()
            }
            7 => {
                input.allowed =
                    MappedBuffer::from_slice(&device, &[1u32, 1, 1, 2, 1, 1, 1, 1]).unwrap()
            }
            8 => input.counts = MappedBuffer::from_slice(&device, &[2u32, u32::MAX, 8]).unwrap(),
            9 => input.counts = MappedBuffer::from_slice(&device, &[0u32, 3, 8]).unwrap(),
            10 => input.counts = MappedBuffer::from_slice(&device, &[2u32, 3, 7]).unwrap(),
            11 => output.capacity = 2,
            12 => input.counts = MappedBuffer::from_slice(&device, &[3u32, 3, 8]).unwrap(),
            13 => input.counts = MappedBuffer::from_slice(&device, &[2u32, 3, 9]).unwrap(),
            _ => unreachable!(),
        }
        let scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let pending = metal_bulk_closure::encode(&device, command, &input, &output).unwrap();
        let consumer = MappedBuffer::from_slice(&device, &[[u32::MAX; 2]; 4]).unwrap();
        metal_parallel::encode(
            command,
            &pipeline_named(IO, "closure_test_consume").unwrap(),
            "closure_test_consume",
            &[
                &output.exits.buffer,
                &output.counts.buffer,
                &consumer.buffer,
            ],
            &[],
            0,
            Some((&output.counts.buffer, 4)),
        );
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(
            pending.check_completed(),
            Err(if case == 11 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            }),
            "case={case}"
        );
        let counts =
            unsafe { std::slice::from_raw_parts(output.counts.buffer.contents().cast::<u32>(), 4) };
        assert_eq!(counts, [0, 0, 1, 1]);
        for buffer in [&output.exits, &consumer] {
            let actual = unsafe {
                std::slice::from_raw_parts(buffer.buffer.contents().cast::<[u32; 2]>(), 4)
            };
            assert_eq!(actual, [[u32::MAX; 2]; 4], "case={case}");
        }
        assert!(scope.snapshot().iter().all(|d| d.completed));
    }
}

#[test]
fn gpu_bulk_closure_rejects_generation_aliases_and_short_buffers_before_dispatch() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    for case in 0..5 {
        let (mut input, mut output) = fixture(&device);
        match case {
            0 => output.generation += 1,
            1 => output.exits.buffer = input.entries.buffer.clone(),
            2 => input.entries.buffer = input.domains.buffer.clone(),
            3 => input.cell_capacity = usize::MAX,
            4 => output.capacity = output.exits.buffer.length() as usize / 8 + 1,
            _ => unreachable!(),
        }
        let scope = LaunchScope::enter();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let actual = metal_bulk_closure::encode(&device, command, &input, &output).err();
        assert_eq!(
            actual,
            Some(if case == 3 {
                Error::BackendCapacity
            } else {
                Error::BackendFailure
            })
        );
        assert!(scope.snapshot().is_empty());
    }
}

#[test]
fn gpu_bulk_closure_requires_completion_and_shares_the_evaluation_launch_budget() {
    let device = Device::system_default().unwrap();
    let (input, output) = fixture(&device);
    let queue = device.new_command_queue();
    {
        let _guard = metal_execution_guard().unwrap();
        let command = queue.new_command_buffer();
        let pending = metal_bulk_closure::encode(&device, command, &input, &output).unwrap();
        assert_eq!(pending.check_completed(), Err(Error::BackendFailure));
    }
    let _guard = metal_execution_guard().unwrap();
    let scope = LaunchScope::enter();
    for attempt in 0..51 {
        let _nested = LaunchScope::enter();
        let command = queue.new_command_buffer();
        let result = metal_bulk_closure::encode(&device, command, &input, &output);
        if attempt == 50 {
            assert_eq!(result.err(), Some(Error::BackendLaunchLimit));
            assert_eq!(commit_checked(command), Err(Error::BackendLaunchLimit));
        } else {
            let pending = result.unwrap();
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            pending.check_completed().unwrap();
        }
    }
    let trace = scope.snapshot();
    assert_eq!(trace.iter().filter(|d| d.encoded).count(), 600);
    assert_eq!(trace.iter().filter(|d| d.completed).count(), 600);
    assert!(
        trace
            .iter()
            .filter(|d| !d.encoded)
            .all(|d| !d.submitted && !d.completed)
    );
}
