//! Checked broad dispatch and saturating scans for compact GPU work.
use super::Error;
use super::metal_backend::launch::CheckedComputeEncoder;
use super::metal_backend::{MappedBuffer, pipeline_named};
use metal::{BufferRef, CommandBufferRef, ComputePipelineState, Device, MTLSize};

// The measured bound limits work per lane and keeps every ulong column below
// 2^55. Larger domains retain the broad hierarchy; no cross-group spin occurs.
const COOPERATIVE_SCAN_LIMIT: usize = 8_388_608;

pub(super) const SCAN_SOURCE: &str = include_str!("metal_parallel.metal");

/// At most one distinct task key can enter for each current row.
pub(super) fn task_group_capacity(rows: usize) -> Result<usize, Error> {
    let slots = rows
        .checked_add(rows.div_ceil(2))
        .ok_or(Error::BackendCapacity)?;
    u32::try_from(slots).map_err(|_| Error::BackendCapacity)?;
    Ok(slots)
}

pub(super) fn encode(
    command: &CommandBufferRef,
    pipeline: &ComputePipelineState,
    label: &str,
    buffers: &[&BufferRef],
    parameters: &[u32],
    groups: usize,
    indirect: Option<(&BufferRef, u64)>,
) {
    let encoder = command.new_compute_command_encoder();
    encoder.set_label(label);
    encoder.set_compute_pipeline_state(pipeline);
    encoder.checked_set_buffers(command, buffers);
    if !parameters.is_empty() {
        encoder.set_bytes(
            buffers.len() as u64,
            std::mem::size_of_val(parameters) as u64,
            parameters.as_ptr().cast(),
        );
    }
    if let Some((buffer, offset)) = indirect {
        encoder.checked_dispatch_thread_groups_indirect(
            command,
            buffer,
            offset,
            MTLSize::new(256, 1, 1),
        );
    } else {
        encoder.checked_dispatch_thread_groups(
            command,
            MTLSize::new(groups as u64, 1, 1),
            MTLSize::new(256, 1, 1),
        );
    }
    encoder.end_encoding();
}

pub(super) struct Scan {
    pub values: MappedBuffer,
    count: usize,
    sums: Vec<(usize, MappedBuffer)>,
}

impl Scan {
    pub(super) fn encode_independent_pair(
        &self,
        other: &Self,
        command: &CommandBufferRef,
    ) -> Result<(), Error> {
        self.validate_independent(other)?;
        if self.count.max(other.count) > COOPERATIVE_SCAN_LIMIT {
            self.encode(command)?;
            return other.encode(command);
        }
        let pipeline = pipeline_named(SCAN_SOURCE, "compact_scan_cooperative_domains")?;
        encode(
            command,
            &pipeline,
            "compact_scan_cooperative_domains",
            &[
                &self.values.buffer,
                self.total(),
                &other.values.buffer,
                other.total(),
            ],
            &[self.count as u32, other.count as u32],
            1,
            None,
        );
        Ok(())
    }
    pub fn new(device: &Device, count: usize) -> Result<Self, Error> {
        let mut sums = Vec::new();
        let mut size = count;
        loop {
            let groups = size.div_ceil(256).max(1);
            sums.push((groups, MappedBuffer::zeroed::<[u32; 4]>(device, groups)?));
            if groups == 1 {
                break;
            }
            size = groups;
        }
        Ok(Self {
            values: MappedBuffer::zeroed::<[u32; 4]>(device, count)?,
            count,
            sums,
        })
    }

    fn cooperative(&self) -> bool {
        (257..=COOPERATIVE_SCAN_LIMIT).contains(&self.count)
    }

    pub fn total(&self) -> &BufferRef {
        &self.sums.last().unwrap().1.buffer
    }

    pub(super) fn levels(&self) -> usize {
        self.sums.len()
    }

    /// Every level reads the GPU-owned length and uses checked indirect bounds.
    pub(super) fn encode_counted(
        &self,
        command: &CommandBufferRef,
        length: &BufferRef,
        arguments: &BufferRef,
        argument_offset: u64,
    ) -> Result<(), Error> {
        if self.cooperative() {
            let pipeline = pipeline_named(SCAN_SOURCE, "compact_scan_cooperative_counted")?;
            encode(
                command,
                &pipeline,
                "compact_scan_cooperative_counted",
                &[&self.values.buffer, self.total(), length],
                &[self.count as u32],
                1,
                None,
            );
            return Ok(());
        }
        let scan = pipeline_named(SCAN_SOURCE, "compact_scan_counted")?;
        let add = pipeline_named(SCAN_SOURCE, "compact_add_parent_counted")?;
        for (index, (_, sums)) in self.sums.iter().enumerate() {
            let values = if index == 0 {
                &*self.values.buffer
            } else {
                &*self.sums[index - 1].1.buffer
            };
            encode(
                command,
                &scan,
                "compact_scan_counted",
                &[values, values, &sums.buffer, length],
                &[index as u32],
                0,
                Some((arguments, argument_offset + index as u64 * 12)),
            );
        }
        for index in (0..self.sums.len() - 1).rev() {
            let values = if index == 0 {
                &*self.values.buffer
            } else {
                &*self.sums[index - 1].1.buffer
            };
            encode(
                command,
                &add,
                "compact_add_parent_counted",
                &[values, &self.sums[index].1.buffer, length],
                &[index as u32],
                0,
                Some((arguments, argument_offset + index as u64 * 12)),
            );
        }
        Ok(())
    }

    pub(super) fn encode_pair(
        &self,
        other: &Self,
        command: &CommandBufferRef,
    ) -> Result<(), Error> {
        if self.count != other.count || self.sums.is_empty() || self.sums.len() != other.sums.len()
        {
            return Err(Error::BackendFailure);
        }
        self.validate_independent(other)?;
        self.encode_equal_pair(other, command)
    }

    fn validate_independent(&self, other: &Self) -> Result<(), Error> {
        let mut owned = std::collections::BTreeSet::new();
        for scan in [self, other] {
            if scan.sums.is_empty() {
                return Err(Error::BackendFailure);
            }
            let mut count = scan.count;
            for (buffer, length) in std::iter::once((&scan.values, scan.count))
                .chain(scan.sums.iter().map(|(n, b)| (b, *n)))
            {
                if !owned.insert((&*buffer.buffer as *const BufferRef) as usize)
                    || length.checked_mul(16).ok_or(Error::BackendCapacity)? as u64
                        > buffer.buffer.length()
                {
                    return Err(Error::BackendFailure);
                }
            }
            for (index, (groups, _)) in scan.sums.iter().enumerate() {
                let expected = count.div_ceil(256).max(1);
                if *groups != expected || (index + 1 == scan.sums.len()) != (*groups == 1) {
                    return Err(Error::BackendFailure);
                }
                count = *groups;
            }
        }
        Ok(())
    }

    fn encode_equal_pair(&self, other: &Self, command: &CommandBufferRef) -> Result<(), Error> {
        if self.cooperative() {
            let pipeline = pipeline_named(SCAN_SOURCE, "compact_scan_cooperative_pair")?;
            encode(
                command,
                &pipeline,
                "compact_scan_cooperative_pair",
                &[
                    &self.values.buffer,
                    self.total(),
                    &other.values.buffer,
                    other.total(),
                ],
                &[self.count as u32],
                1,
                None,
            );
            return Ok(());
        }
        let pipeline = pipeline_named(SCAN_SOURCE, "compact_scan_pair")?;
        for index in 0..self.sums.len() {
            let (count, a, b) = if index == 0 {
                (self.count, &self.values, &other.values)
            } else {
                (
                    self.sums[index - 1].0,
                    &self.sums[index - 1].1,
                    &other.sums[index - 1].1,
                )
            };
            encode(
                command,
                &pipeline,
                "compact_scan_pair",
                &[
                    &a.buffer,
                    &a.buffer,
                    &self.sums[index].1.buffer,
                    &b.buffer,
                    &b.buffer,
                    &other.sums[index].1.buffer,
                ],
                &[u32::try_from(count).map_err(|_| Error::BackendCapacity)?],
                self.sums[index].0,
                None,
            );
        }
        let pipeline = pipeline_named(SCAN_SOURCE, "compact_add_parent_pair")?;
        for index in (0..self.sums.len() - 1).rev() {
            let (count, a, b) = if index == 0 {
                (self.count, &self.values, &other.values)
            } else {
                (
                    self.sums[index - 1].0,
                    &self.sums[index - 1].1,
                    &other.sums[index - 1].1,
                )
            };
            encode(
                command,
                &pipeline,
                "compact_add_parent_pair",
                &[
                    &a.buffer,
                    &self.sums[index].1.buffer,
                    &b.buffer,
                    &other.sums[index].1.buffer,
                ],
                &[u32::try_from(count).map_err(|_| Error::BackendCapacity)?],
                count.div_ceil(256),
                None,
            );
        }
        Ok(())
    }

    pub fn encode(&self, command: &CommandBufferRef) -> Result<(), Error> {
        self.encode_labeled(command, "compact_scan", "compact_add_parent")
    }
    pub(super) fn encode_labeled(
        &self,
        command: &CommandBufferRef,
        scan_label: &str,
        add_label: &str,
    ) -> Result<(), Error> {
        if self.cooperative() {
            let pipeline = pipeline_named(SCAN_SOURCE, "compact_scan_cooperative")?;
            let label = if scan_label == "compact_scan" {
                "compact_scan_cooperative"
            } else {
                scan_label
            };
            encode(
                command,
                &pipeline,
                label,
                &[&self.values.buffer, self.total()],
                &[self.count as u32],
                1,
                None,
            );
            return Ok(());
        }
        let scan = pipeline_named(SCAN_SOURCE, "compact_scan")?;
        let add = pipeline_named(SCAN_SOURCE, "compact_add_parent")?;
        for (index, (groups, sums)) in self.sums.iter().enumerate() {
            let (count, values) = if index == 0 {
                (self.count, &*self.values.buffer)
            } else {
                (self.sums[index - 1].0, &*self.sums[index - 1].1.buffer)
            };
            encode(
                command,
                &scan,
                scan_label,
                &[values, values, &sums.buffer],
                &[u32::try_from(count).map_err(|_| Error::BackendCapacity)?],
                *groups,
                None,
            );
        }
        for index in (0..self.sums.len() - 1).rev() {
            let (count, values) = if index == 0 {
                (self.count, &*self.values.buffer)
            } else {
                (self.sums[index - 1].0, &*self.sums[index - 1].1.buffer)
            };
            encode(
                command,
                &add,
                add_label,
                &[values, &self.sums[index].1.buffer],
                &[u32::try_from(count).map_err(|_| Error::BackendCapacity)?],
                count.div_ceil(256),
                None,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
