use super::super::metal_backend::{launch::LaunchScope, metal_execution_guard};
use super::super::{CaptureIdentity, capture_identity_containment};
use super::*;

fn frontier(
    device: &Device,
    rows: &[(u32, bool, Vec<[u32; 4]>)],
) -> (CountedFrontier, MappedBuffer, usize, Vec<Vec<u32>>) {
    let n = rows.len();
    let mut columns = vec![0u32; n * 11];
    let mut captures = vec![];
    let mut first = 0;
    let mut records = vec![];
    for (i, (task, query, values)) in rows.iter().enumerate() {
        if i == 0 || rows[i - 1].0 != *task {
            first = i;
        }
        let fields = [
            *task,
            if *query { 0 } else { (i - first + 1) as u32 },
            0,
            0,
            0,
            0,
            0,
            0,
            captures.len() as u32,
            values.len() as u32,
            (i - first) as u32,
        ];
        for (f, value) in fields.iter().enumerate() {
            columns[f * n + i] = *value;
        }
        records.push(fields.to_vec());
        captures.extend_from_slice(values);
    }
    let count = captures.len();
    (
        CountedFrontier {
            columns: MappedBuffer::from_slice(device, &columns).unwrap(),
            counts: MappedBuffer::from_slice(
                device,
                &[n as u32, count as u32, n.div_ceil(256) as u32, 1, 1],
            )
            .unwrap(),
            capacity: n,
            generation: 17,
        },
        MappedBuffer::from_slice(device, &captures).unwrap(),
        count,
        records,
    )
}
fn records(frontier: &CountedFrontier) -> Vec<Vec<u32>> {
    let n = unsafe { frontier.counts.buffer.contents().cast::<u32>().read() } as usize;
    assert!(n <= frontier.capacity);
    let columns = unsafe {
        std::slice::from_raw_parts(frontier.columns.buffer.contents().cast::<u32>(), n * 11)
    };
    (0..n)
        .map(|i| (0..11).map(|f| columns[f * n + i]).collect())
        .collect()
}
fn oracle(rows: &[(u32, bool, Vec<[u32; 4]>)]) -> Vec<usize> {
    let identities = rows
        .iter()
        .map(|(_, _, values)| {
            values
                .iter()
                .map(|v| CaptureIdentity {
                    node: v[0],
                    label: v[1] as u16,
                    start: v[2] as usize,
                    end: v[3] as usize,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut retained = vec![];
    for (i, (task, query, _)) in rows.iter().enumerate() {
        if !query {
            retained.push(i);
            continue;
        }
        if retained.iter().any(|&j| {
            rows[j].0 == *task
                && rows[j].1
                && capture_identity_containment(&identities[j], &identities[i]).0
        }) {
            continue;
        }
        retained.retain(|&j| {
            rows[j].0 != *task
                || !rows[j].1
                || !capture_identity_containment(&identities[i], &identities[j]).0
        });
        retained.push(i);
    }
    retained
}
#[test]
fn terminal_anchor_preserves_complete_ordered_containment_across_keys_and_occurrences() {
    metal::objc::rc::autoreleasepool(|| {
        let _guard = metal_execution_guard().unwrap();
        let scope = LaunchScope::enter();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let atoms = [
            [1, 0, 0, 3],
            [1, 1, 0, 3],
            [2, 0, 0, 3],
            [3, 0, 4, 4],
            [4, 0, 4, 4],
            [5, 0, 5, 7],
        ];
        let mut sequences = vec![vec![]];
        for a in atoms {
            sequences.push(vec![a]);
            for b in atoms {
                if !((b[2] < a[2]) || (b[2] == a[2] && b[3] > a[3])) {
                    sequences.push(vec![a, b]);
                }
            }
        }
        let mut rows = vec![];
        // Every sequence becomes the first maximum in a different query task.
        // Reverse ties, duplicate entries, empty lists, and zero-width identities
        // keep their ordered meanings. Ordinary rows preserve every occurrence.
        for (task, anchor) in sequences.iter().enumerate() {
            rows.push((task as u32 * 3, true, anchor.clone()));
            for sequence in &sequences {
                rows.push((task as u32 * 3, true, sequence.clone()));
            }
            for sequence in &sequences {
                rows.push((task as u32 * 3 + 1, false, sequence.clone()));
                rows.push((task as u32 * 3 + 1, false, sequence.clone()));
            }
        }
        let expected = oracle(&rows);
        let (input, captures, count, original) = frontier(&device, &rows);
        let pruned = anchor_projection(&device, &queue, input, &captures, count).unwrap();
        let output = CountedFrontier {
            columns: MappedBuffer::zeroed::<[u32; 11]>(&device, pruned.capacity).unwrap(),
            counts: MappedBuffer::zeroed::<u32>(&device, 5).unwrap(),
            capacity: pruned.capacity,
            generation: 17,
        };
        metal_frontier_reduce::execute_completed(
            &device, &queue, &pruned, &output, &captures, count, 16,
        )
        .unwrap();
        assert_eq!(
            records(&output),
            expected
                .into_iter()
                .map(|i| original[i].clone())
                .collect::<Vec<_>>()
        );
        let trace = scope.snapshot();
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
    });
}
#[test]
fn terminal_anchor_rejects_invalid_projected_rows_and_capture_values() {
    metal::objc::rc::autoreleasepool(|| {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        for case in 0..12 {
            let rows = vec![
                (2, true, vec![[1, 0, 0, 3], [2, 1, 4, 4]]),
                (2, true, vec![[1, 0, 0, 3]]),
                (5, false, vec![]),
            ];
            let (input, captures, count, _) = frontier(&device, &rows);
            let n = rows.len();
            let s = input.columns.buffer.contents().cast::<u32>();
            let c = captures.buffer.contents().cast::<u32>();
            let h = input.counts.buffer.contents().cast::<u32>();
            unsafe {
                match case {
                    0 => s.add(1).write(1),                    // descending task
                    1 => s.add(10 * n + 1).write(9),           // raw ordinal gap
                    2 => s.add(n + 1).write(2),                // mixed scopes in a query task
                    3 => s.add(n + 2).write(7),                // incorrect ordinary scope
                    4 => s.add(3 * n).write(1),                // unexpected key field
                    5 => s.add(8 * n + 1).write(count as u32), // capture range overflow
                    6 => c.add(4).write(u32::MAX),             // invalid node
                    7 => c.add(1).write(65536),                // invalid label
                    8 => c.add(2).write(9),                    // inverted geometry
                    9 => c.add(6).write(0),                    // unsorted geometry
                    10 => h.add(2).write(99),                  // malformed count header
                    11 => h.add(1).write(count as u32 + 1),
                    _ => unreachable!(),
                }
            }
            assert!(
                matches!(
                    anchor_projection(&device, &queue, input, &captures, count),
                    Err(Error::BackendFailure)
                ),
                "case={case}"
            );
        }
    });
}
#[test]
fn terminal_anchor_retains_incomparable_rows_and_the_existing_pair_refusal() {
    metal::objc::rc::autoreleasepool(|| {
        let _guard = metal_execution_guard().unwrap();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let rows = (0..4097u32)
            .map(|i| (0, true, vec![[i, 0, i * 2, i * 2 + 1]]))
            .collect::<Vec<_>>();
        let (input, captures, count, expected) = frontier(&device, &rows);
        let pruned = anchor_projection(&device, &queue, input, &captures, count).unwrap();
        assert_eq!(records(&pruned), expected);
        let output = CountedFrontier {
            columns: MappedBuffer::zeroed::<[u32; 11]>(&device, pruned.capacity).unwrap(),
            counts: MappedBuffer::zeroed::<u32>(&device, 5).unwrap(),
            capacity: pruned.capacity,
            generation: 17,
        };
        assert!(matches!(
            metal_frontier_reduce::execute_completed(
                &device, &queue, &pruned, &output, &captures, count, 12
            ),
            Err(Error::BackendCapacity)
        ));
    });
}
