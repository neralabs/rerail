//! Immutable kind tables, cooperative ancestor scans, and stable capture indexes.
use super::Error;
use super::metal_backend::MappedBuffer;
use super::metal_backend::launch::{commit_checked, wait_checked};
use super::metal_capture_arguments::{ArgumentInput, direct};
use super::metal_grid_plan::ResidentInput;
use super::metal_output_dispatch::SOURCE;
use metal::{CommandQueueRef, Device};
use std::collections::BTreeMap;

pub(super) struct CaptureKinds {
    pub code: MappedBuffer,
    pub ancestors: MappedBuffer,
}
pub(super) struct CaptureGroups {
    pub ids: MappedBuffer,
    pub ranks: MappedBuffer,
}
impl CaptureKinds {
    pub(super) fn new(
        device: &Device,
        queue: &CommandQueueRef,
        resident: &ResidentInput,
        data: &MappedBuffer,
    ) -> Result<Self, Error> {
        let batch = &resident.input.packed;
        let mut sets = BTreeMap::<Vec<u32>, u32>::new();
        let mut members = Vec::new();
        let mut assertion_sets = vec![u32::MAX; batch.assertions.len()];
        let mut domains = BTreeMap::<u32, u32>::new();
        for (index, a) in batch.assertions.iter().enumerate() {
            if !matches!(a.kind, 3 | 5) {
                continue;
            }
            let mut kinds = batch.assertion_data
                [a.data_start as usize..(a.data_start + a.data_count) as usize]
                .to_vec();
            kinds.sort_unstable();
            kinds.dedup();
            let next = size(sets.len())?;
            let set = *sets.entry(kinds.clone()).or_insert_with(|| {
                members.push(kinds);
                next
            });
            assertion_sets[index] = set;
            if a.kind == 5 && a.flags & 4 != 0 {
                let next = size(domains.len())?;
                domains.entry(set).or_insert(next);
            }
        }
        let mut code = vec![
            size(assertion_sets.len())?,
            size(sets.len())?,
            size(domains.len())?,
            6,
            0,
            0,
        ];
        code.extend(assertion_sets);
        code[4] = size(code.len())?;
        let descriptors = code.len();
        code.resize(descriptors + members.len() * 3, 0);
        for (set, kinds) in members.into_iter().enumerate() {
            let offset = size(code.len())?;
            code[descriptors + set * 3] = offset;
            code[descriptors + set * 3 + 1] = size(kinds.len())?;
            code[descriptors + set * 3 + 2] =
                domains.get(&(set as u32)).copied().unwrap_or(u32::MAX);
            code.extend(kinds);
        }
        code[5] = size(code.len())?;
        let mut domain_sets = vec![0; domains.len()];
        for (&set, &domain) in &domains {
            domain_sets[domain as usize] = set;
        }
        code.extend(domain_sets);
        let code = MappedBuffer::from_slice(device, &code)?;
        let nodes = batch.nodes.len();
        let domain_count = domains.len();
        let total = nodes
            .checked_mul(domain_count)
            .ok_or(Error::BackendCapacity)?;
        size(total)?;
        let ancestors = MappedBuffer::zeroed::<u32>(device, total)?;
        let mut levels = Vec::new();
        let mut count = nodes;
        if domain_count != 0 {
            loop {
                let groups = count.div_ceil(256).max(1);
                levels.push((
                    count,
                    MappedBuffer::zeroed::<u32>(
                        device,
                        groups
                            .checked_mul(domain_count)
                            .ok_or(Error::BackendCapacity)?,
                    )?,
                ));
                if groups == 1 {
                    break;
                }
                count = groups;
            }
        }
        let command = queue.new_command_buffer();
        if domain_count != 0 {
            encode(
                command,
                "predicate_ancestor_init",
                &[&data.buffer, &code.buffer, &ancestors.buffer],
                nodes,
                domain_count,
                total.div_ceil(256),
            )?;
        }
        commit_checked(command)?;
        wait_checked(command)?;
        let command = queue.new_command_buffer();
        for (level, (count, sums)) in levels.iter().enumerate() {
            let values = if level == 0 {
                &ancestors
            } else {
                &levels[level - 1].1
            };
            encode(
                command,
                "predicate_ancestor_scan",
                &[&values.buffer, &sums.buffer],
                *count,
                domain_count,
                count.div_ceil(256).max(1) * domain_count,
            )?;
        }
        for level in (0..levels.len().saturating_sub(1)).rev() {
            let values = if level == 0 {
                &ancestors
            } else {
                &levels[level - 1].1
            };
            let count = levels[level].0;
            encode(
                command,
                "predicate_ancestor_add",
                &[&values.buffer, &levels[level].1.buffer],
                count,
                domain_count,
                (count * domain_count).div_ceil(256),
            )?;
        }
        commit_checked(command)?;
        wait_checked(command)?;
        Ok(Self { code, ancestors })
    }
}
impl CaptureGroups {
    pub(super) fn prepare(
        device: &Device,
        queue: &CommandQueueRef,
        input: &ArgumentInput<'_>,
    ) -> Result<Self, Error> {
        let mut ids = [
            MappedBuffer::zeroed::<u32>(device, input.parameters.captures as usize)?,
            MappedBuffer::zeroed::<u32>(device, input.parameters.captures as usize)?,
        ];
        let ranks = MappedBuffer::zeroed::<u32>(device, input.parameters.captures as usize)?;
        let maximum = MappedBuffer::zeroed::<u32>(device, 1)?;
        let command = queue.new_command_buffer();
        input.run(
            command,
            "predicate_group_init",
            &[&input.spans.buffer, &ids[0].buffer, &maximum.buffer],
            (input.parameters.results.max(input.parameters.captures)) as usize,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        let maximum = unsafe { maximum.buffer.contents().cast::<u32>().read() };
        if maximum > input.parameters.captures {
            return Err(Error::BackendFailure);
        }
        let command = queue.new_command_buffer();
        let mut current = 0;
        for width in (0..32)
            .map(|shift| 1u32 << shift)
            .take_while(|&width| width < maximum)
        {
            let mut p = input.parameters;
            p.merge_width = width;
            direct(
                command,
                "predicate_group_merge",
                &[
                    &input.spans.buffer,
                    &input.captures.buffer,
                    &input.owners.buffer,
                    &ids[current].buffer,
                    &ids[1 - current].buffer,
                ],
                &p,
                p.captures as usize,
            )?;
            current = 1 - current;
        }
        input.run(
            command,
            "predicate_group_inverse",
            &[&ids[current].buffer, &ranks.buffer],
            input.parameters.captures as usize,
        )?;
        commit_checked(command)?;
        wait_checked(command)?;
        if current == 1 {
            ids.swap(0, 1);
        }
        Ok(Self {
            ids: ids.into_iter().next().ok_or(Error::BackendFailure)?,
            ranks,
        })
    }
}
fn size(n: usize) -> Result<u32, Error> {
    u32::try_from(n).map_err(|_| Error::BackendCapacity)
}
fn encode(
    command: &metal::CommandBufferRef,
    name: &str,
    buffers: &[&metal::BufferRef],
    count: usize,
    domains: usize,
    groups: usize,
) -> Result<(), Error> {
    super::metal_parallel::encode(
        command,
        &super::metal_backend::pipeline_named(SOURCE, name)?,
        name,
        buffers,
        &[size(count)?, size(domains)?],
        groups.max(1),
        None,
    );
    Ok(())
}
