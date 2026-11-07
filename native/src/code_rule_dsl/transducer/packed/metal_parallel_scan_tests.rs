//! Paired scans preserve eight independent count meanings and checked saturation.
use super::super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::super::metal_backend::metal_execution_guard;
use super::*;

fn oracle(values: &[[u32; 4]]) -> (Vec<[u32; 4]>, [u32; 4]) {
    let mut offsets = Vec::new();
    let mut sum = [0u32; 4];
    for value in values {
        offsets.push(sum);
        for (total, add) in sum.iter_mut().zip(value) {
            *total = total.saturating_add(*add);
        }
    }
    (offsets, sum)
}
#[test]
fn independent_arenas_share_each_scan_level_without_mixing_columns_or_overflows() {
    for (count, expected_launches) in [
        (0, 1),
        (1, 1),
        (255, 1),
        (256, 1),
        (257, 1),
        (65_536, 1),
        (65_537, 1),
    ] {
        let a = (0..count)
            .map(|i| [(i % 5) as u32, 0, u32::MAX - 2, 1])
            .collect::<Vec<_>>();
        let b = (0..count)
            .map(|i| [0, (i % 11) as u32, 1, u32::MAX / 100])
            .collect::<Vec<_>>();
        let expected_a = oracle(&a);
        let expected_b = oracle(&b);
        let (actual_a, actual_b, launches) = metal::objc::rc::autoreleasepool(|| {
            let _guard = metal_execution_guard().unwrap();
            let scope = LaunchScope::enter();
            let device = Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let left = Scan::new(&device, count).unwrap();
            let right = Scan::new(&device, count).unwrap();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    a.as_ptr(),
                    left.values.buffer.contents().cast(),
                    count,
                );
                std::ptr::copy_nonoverlapping(
                    b.as_ptr(),
                    right.values.buffer.contents().cast(),
                    count,
                );
            }
            let command = queue.new_command_buffer();
            left.encode_pair(&right, command).unwrap();
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            let get = |scan: &Scan| {
                let offsets =
                    super::super::metal_register_execution::read::<[u32; 4]>(&scan.values, count)
                        .unwrap();
                let sum = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
                (offsets, sum)
            };
            (get(&left), get(&right), scope.snapshot().len())
        });
        assert_eq!(actual_a, expected_a, "left count {count}");
        assert_eq!(actual_b, expected_b, "right count {count}");
        assert_eq!(
            launches, expected_launches,
            "bounded independent arenas must finish in one dispatch at count {count}"
        );
    }
}
#[test]
fn paired_scans_reject_aliases_and_different_domains_before_dispatch() {
    let (alias, different, trace) = {
        let _guard = metal_execution_guard().unwrap();
        let scope = LaunchScope::enter();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let one = Scan::new(&device, 1).unwrap();
        let two = Scan::new(&device, 2).unwrap();
        let command = queue.new_command_buffer();
        (
            one.encode_pair(&one, command),
            one.encode_pair(&two, command),
            scope.snapshot(),
        )
    };
    assert_eq!(alias, Err(Error::BackendFailure));
    assert_eq!(different, Err(Error::BackendFailure));
    assert!(trace.is_empty());
}

#[test]
fn independent_scan_domains_refuse_aliases_and_inconsistent_hierarchies_before_dispatch() {
    let _guard = metal_execution_guard().unwrap();
    let scope = LaunchScope::enter();
    let before = scope.snapshot().len();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    let left = Scan::new(&device, 1).unwrap();
    let mut right = Scan::new(&device, 257).unwrap();
    let command = queue.new_command_buffer();
    assert_eq!(
        left.encode_independent_pair(&left, command),
        Err(Error::BackendFailure)
    );
    right.sums[0].0 += 1;
    assert_eq!(
        left.encode_independent_pair(&right, command),
        Err(Error::BackendFailure)
    );
    assert_eq!(scope.snapshot().len(), before);
}

#[test]
fn independent_scan_domains_keep_every_saturated_prefix_with_one_dispatch() {
    let mut measured = Vec::new();
    for (left_count, right_count) in [
        (0usize, 257usize),
        (1, 0),
        (257, 2),
        (65_537, 257),
        (2_800_001, 10_938),
    ] {
        let _guard = metal_execution_guard().unwrap();
        let scope = LaunchScope::enter();
        let before = scope.snapshot().len();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let left = Scan::new(&device, left_count).unwrap();
        let right = Scan::new(&device, right_count).unwrap();
        let left_values = (0..left_count)
            .map(|i| [(i % 13) as u32, u32::MAX / 31, 0, 1])
            .collect::<Vec<_>>();
        let right_values = (0..right_count)
            .map(|i| [u32::MAX - 1, 0, (i % 17) as u32, 3])
            .collect::<Vec<_>>();
        unsafe {
            std::ptr::copy_nonoverlapping(
                left_values.as_ptr(),
                left.values.buffer.contents().cast(),
                left_count,
            );
            std::ptr::copy_nonoverlapping(
                right_values.as_ptr(),
                right.values.buffer.contents().cast(),
                right_count,
            );
        }
        let started = std::time::Instant::now();
        let command = queue.new_command_buffer();
        left.encode_independent_pair(&right, command).unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        let execution_seconds = started.elapsed().as_secs_f64();
        for (scan, values) in [(&left, &left_values), (&right, &right_values)] {
            let actual = super::super::metal_register_execution::read::<[u32; 4]>(
                &scan.values,
                values.len(),
            )
            .unwrap();
            let total = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
            assert_eq!((actual, total), oracle(values));
        }
        let all = scope.snapshot();
        let trace = &all[before..];
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        eprintln!(
            "INDEPENDENT_SCAN_EVIDENCE {}",
            serde_json::json!({"left":left_count,"right":right_count,"seconds":started.elapsed().as_secs_f64(),"executionSeconds":execution_seconds,"dispatches":trace.len()})
        );
        measured.push((left_count, right_count, trace.len()));
    }
    assert!(
        measured.iter().all(|&(_, _, count)| count == 1),
        "independent bounded scan domains require one dispatch: {measured:?}"
    );
}

#[test]
fn bounded_scans_preserve_every_prefix_and_saturated_total_in_one_dispatch() {
    for count in [
        0, 1, 255, 256, 257, 65_536, 65_537, 1_048_577, 8_388_608, 8_388_609,
    ] {
        metal::objc::rc::autoreleasepool(|| {
            let _guard = metal_execution_guard().unwrap();
            let scope = LaunchScope::enter();
            let device = Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let values = (0..count)
                .map(|i| [(i % 7) as u32, 0, u32::MAX - 2, (i % 17) as u32])
                .collect::<Vec<_>>();
            let expected = oracle(&values);
            let scan = Scan::new(&device, count).unwrap();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    values.as_ptr(),
                    scan.values.buffer.contents().cast(),
                    count,
                );
            }
            let started = std::time::Instant::now();
            let command = queue.new_command_buffer();
            scan.encode(command).unwrap();
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            let seconds = started.elapsed().as_secs_f64();
            let actual =
                super::super::metal_register_execution::read::<[u32; 4]>(&scan.values, count)
                    .unwrap();
            let total = unsafe { scan.total().contents().cast::<[u32; 4]>().read() };
            assert_eq!((actual, total), expected, "count={count}");
            let trace = scope.snapshot();
            eprintln!(
                "BOUNDED_SCAN_EVIDENCE {}",
                serde_json::json!({"rows":count,"seconds":seconds,"dispatches":trace.len()})
            );
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            assert_eq!(
                trace.len(),
                if count <= 8_388_608 { 1 } else { 5 },
                "count={count}"
            );
        });
    }
}

#[test]
fn gpu_counted_bounded_scans_preserve_inactive_values_and_every_live_prefix() {
    for count in [0usize, 1, 255, 256, 257, 65_536, 65_537] {
        metal::objc::rc::autoreleasepool(|| {
            let _guard = metal_execution_guard().unwrap();
            let scope = LaunchScope::enter();
            let device = Device::system_default().unwrap();
            let queue = device.new_command_queue();
            let capacity = 65_537;
            let values = (0..capacity)
                .map(|i| [(i % 7) as u32, u32::MAX - 2, 0, (i % 17) as u32])
                .collect::<Vec<_>>();
            let (mut expected, total) = oracle(&values[..count]);
            expected.extend_from_slice(&values[count..]);
            let scan = Scan::new(&device, capacity).unwrap();
            let length = MappedBuffer::from_slice(&device, &[count as u32]).unwrap();
            let mut n = count;
            let args = (0..scan.levels())
                .flat_map(|_| {
                    let groups = n.div_ceil(256).max(1);
                    n = groups;
                    [groups as u32, 1, 1]
                })
                .collect::<Vec<_>>();
            let arguments = MappedBuffer::from_slice(&device, &args).unwrap();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    values.as_ptr(),
                    scan.values.buffer.contents().cast(),
                    capacity,
                );
            }
            let command = queue.new_command_buffer();
            scan.encode_counted(command, &length.buffer, &arguments.buffer, 0)
                .unwrap();
            commit_checked(command).unwrap();
            wait_checked(command).unwrap();
            let actual =
                super::super::metal_register_execution::read::<[u32; 4]>(&scan.values, capacity)
                    .unwrap();
            assert_eq!(actual, expected, "count={count}");
            assert_eq!(
                unsafe { scan.total().contents().cast::<[u32; 4]>().read() },
                total,
                "count={count}"
            );
            let trace = scope.snapshot();
            assert!(
                trace
                    .iter()
                    .all(|r| r.encoded && r.submitted && r.completed)
            );
            assert_eq!(trace.len(), 1, "GPU-owned count={count}");
        });
    }
}
