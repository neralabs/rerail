use super::super::super::metal_backend::launch::{LaunchScope, commit_checked, wait_checked};
use super::super::super::metal_backend::{MappedBuffer, metal_execution_guard};
use super::*;
use metal::Device;

#[test]
fn publication_tiles_preserve_every_tied_key_and_inactive_cell_in_one_dispatch() {
    let mut measured = Vec::new();
    for count in [0usize, 1, 2, 255, 256, 257, 1025, 65_537] {
        let _guard = metal_execution_guard().unwrap();
        let scope = LaunchScope::enter();
        let before = scope.snapshot().len();
        let device = Device::system_default().unwrap();
        let queue = device.new_command_queue();
        let capacity = count + 7;
        let metadata = (0..count)
            .map(|i| [(i % 7) as u32, (i % 5) as u32, (count - i) as u32, i as u32])
            .collect::<Vec<_>>();
        let input = (0..count as u32)
            .rev()
            .chain(std::iter::repeat_n(u32::MAX, 7))
            .collect::<Vec<_>>();
        let mut expected = input.clone();
        for tile in expected[..count].chunks_mut(256) {
            tile.sort_by_key(|&id| {
                let m = metadata[id as usize];
                (m[0], m[1], m[2])
            });
        }
        let header = MappedBuffer::from_slice(&device, &[count as u32]).unwrap();
        let mut metadata = metadata;
        metadata.extend(std::iter::repeat_n([u32::MAX; 4], 7));
        let metadata = MappedBuffer::from_slice(&device, &metadata).unwrap();
        let ids = [
            MappedBuffer::from_slice(&device, &input).unwrap(),
            MappedBuffer::from_slice(&device, &vec![u32::MAX; capacity]).unwrap(),
        ];
        let status = MappedBuffer::from_slice(&device, &[0u32]).unwrap();
        let started = std::time::Instant::now();
        let command = queue.new_command_buffer();
        let current = encode_tiles(
            command,
            &header.buffer,
            &metadata.buffer,
            [&ids[0].buffer, &ids[1].buffer],
            &status.buffer,
            capacity,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        let seconds = started.elapsed().as_secs_f64();
        let actual =
            super::super::super::metal_register_execution::read::<u32>(&ids[current], capacity)
                .unwrap();
        assert_eq!(actual, expected, "complete tile order count={count}");
        let trace = scope.snapshot();
        let trace = &trace[before..];
        assert!(
            trace
                .iter()
                .all(|r| r.encoded && r.submitted && r.completed)
        );
        eprintln!(
            "PUBLICATION_TILE_EVIDENCE rows={count} seconds={seconds} dispatches={}",
            trace.len()
        );
        measured.push((count, trace.len()));
    }
    assert!(
        measured.iter().all(|&(_, count)| count == 1),
        "complete publication tile order requires one dispatch: {measured:?}"
    );
}

#[test]
fn publication_tiles_reject_aliased_and_undersized_owners_before_dispatch() {
    let _guard = metal_execution_guard().unwrap();
    let scope = LaunchScope::enter();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    for defect in 0..7 {
        let header = MappedBuffer::from_slice(&device, &[4u32]).unwrap();
        let metadata = MappedBuffer::from_slice(&device, &[[0u32; 4]; 4]).unwrap();
        let ids = [
            MappedBuffer::from_slice(&device, &[0u32, 1, 2, 3]).unwrap(),
            MappedBuffer::zeroed::<u32>(&device, 4).unwrap(),
        ];
        let status = MappedBuffer::from_slice(&device, &[0u32]).unwrap();
        let short = MappedBuffer {
            buffer: device.new_buffer(1, metal::MTLResourceOptions::StorageModeShared),
        };
        let mut buffers = [
            &*header.buffer,
            &*metadata.buffer,
            &*ids[0].buffer,
            &*ids[1].buffer,
            &*status.buffer,
        ];
        match defect {
            0 => buffers[3] = buffers[2],
            1 => buffers[1] = &short.buffer,
            2 => buffers[3] = &short.buffer,
            3 => buffers[0] = &short.buffer,
            4 => buffers[4] = &short.buffer,
            5 => buffers[4] = buffers[0],
            _ => buffers[3] = buffers[1],
        }
        let before = scope.snapshot().len();
        let command = queue.new_command_buffer();
        assert_eq!(
            encode_tiles(
                command,
                buffers[0],
                buffers[1],
                [buffers[2], buffers[3]],
                buffers[4],
                4
            ),
            Err(Error::BackendFailure),
            "defect={defect}"
        );
        assert_eq!(
            scope.snapshot().len(),
            before,
            "invalid ownership must refuse before dispatch"
        );
    }
}

#[test]
fn publication_tiles_refuse_invalid_gpu_lengths_indices_and_prior_failure_without_output() {
    let _guard = metal_execution_guard().unwrap();
    let device = Device::system_default().unwrap();
    let queue = device.new_command_queue();
    for defect in 0..3 {
        let header =
            MappedBuffer::from_slice(&device, &[if defect == 0 { 5u32 } else { 4 }]).unwrap();
        let metadata = MappedBuffer::from_slice(&device, &[[0u32; 4]; 4]).unwrap();
        let input = if defect == 1 {
            [u32::MAX, 1, 2, 3]
        } else {
            [0, 1, 2, 3]
        };
        let ids = [
            MappedBuffer::from_slice(&device, &input).unwrap(),
            MappedBuffer::from_slice(&device, &[u32::MAX; 4]).unwrap(),
        ];
        let status = MappedBuffer::from_slice(&device, &[u32::from(defect == 2)]).unwrap();
        let command = queue.new_command_buffer();
        let current = encode_tiles(
            command,
            &header.buffer,
            &metadata.buffer,
            [&ids[0].buffer, &ids[1].buffer],
            &status.buffer,
            4,
        )
        .unwrap();
        commit_checked(command).unwrap();
        wait_checked(command).unwrap();
        assert_eq!(
            super::super::super::metal_register_execution::read::<u32>(&status, 1).unwrap(),
            [if defect == 2 { 1 } else { 2 }]
        );
        assert_eq!(
            super::super::super::metal_register_execution::read::<u32>(&ids[current], 4).unwrap(),
            [u32::MAX; 4]
        );
    }
}
