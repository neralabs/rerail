//! GPU closure over one bounded batch of complete-value sibling domains.
use super::Error;
use super::metal_backend::launch::check_submission;
use super::metal_backend::{MappedBuffer, pipeline_named};
use super::metal_parallel::encode as dispatch;
use metal::{BufferRef, CommandBufferRef, Device};

const SOURCE: &str = include_str!("metal_bulk_closure.metal");

/// Domains are disjoint [first cell, cell count] ranges. Entries are ordered
/// [domain index, offset] pairs; their array position is logical priority.
/// The upstream producer must group complete values before packing these ranges.
pub(super) struct ClosureRanges {
    pub domains: MappedBuffer,
    pub entries: MappedBuffer,
    pub allowed: MappedBuffer,
    /// GPU-owned [valid domains, valid entries, valid cells].
    pub counts: MappedBuffer,
    pub domain_capacity: usize,
    pub entry_capacity: usize,
    pub cell_capacity: usize,
    pub generation: u64,
}

pub(super) struct ClosureExits {
    /// [entry index, final absolute cell address], in logical entry order.
    pub exits: MappedBuffer,
    /// GPU-owned [valid exits, ceil(exits/256), 1, 1].
    pub counts: MappedBuffer,
    pub capacity: usize,
    pub generation: u64,
}

// Each level reads its GPU-owned length. A single workgroup needs no parent
// carry pass. Larger capacities add bounded hierarchical levels.
struct Scan {
    values: MappedBuffer,
    totals: Vec<MappedBuffer>,
}
impl Scan {
    fn new(device: &Device, capacity: usize) -> Result<Self, Error> {
        let mut totals = Vec::new();
        let mut count = capacity;
        loop {
            count = count.div_ceil(256).max(1);
            totals.push(MappedBuffer::zeroed::<[u32; 2]>(device, count)?);
            if count == 1 {
                break;
            }
        }
        Ok(Self {
            values: MappedBuffer::zeroed::<[u32; 2]>(device, capacity)?,
            totals,
        })
    }
    fn encode(
        &self,
        command: &CommandBufferRef,
        lengths: &BufferRef,
        arguments: &BufferRef,
        argument_offset: u64,
        length_index: u32,
        sum: bool,
    ) -> Result<(), Error> {
        for level in 0..self.totals.len() {
            let values = if level == 0 {
                &*self.values.buffer
            } else {
                &*self.totals[level - 1].buffer
            };
            dispatch(
                command,
                &pipeline_named(SOURCE, "closure_scan")?,
                "closure_scan",
                &[values, &self.totals[level].buffer, lengths],
                &[length_index, level as u32, u32::from(sum), 0],
                0,
                Some((arguments, argument_offset + level as u64 * 12)),
            );
        }
        for level in (0..self.totals.len() - 1).rev() {
            let values = if level == 0 {
                &*self.values.buffer
            } else {
                &*self.totals[level - 1].buffer
            };
            dispatch(
                command,
                &pipeline_named(SOURCE, "closure_add_carries")?,
                "closure_add_carries",
                &[values, &self.totals[level].buffer, lengths],
                &[length_index, level as u32, u32::from(sum), 0],
                0,
                Some((arguments, argument_offset + level as u64 * 12)),
            );
        }
        Ok(())
    }
}

#[must_use = "Check closure failures after the caller-owned command completes"]
pub(super) struct PendingClosure {
    command: metal::CommandBuffer,
    owners: Scan,
    ends: Scan,
    counts: Scan,
    heads: MappedBuffer,
    seeds: MappedBuffer,
    lengths: MappedBuffer,
    arguments: MappedBuffer,
    status: MappedBuffer,
}
impl PendingClosure {
    pub(super) fn status_buffer(&self) -> &BufferRef {
        &self.status.buffer
    }
    pub(super) fn check_completed(self) -> Result<(), Error> {
        check_submission()?;
        if self.command.status() != metal::MTLCommandBufferStatus::Completed {
            return Err(Error::BackendFailure);
        }
        let status = unsafe { self.status.buffer.contents().cast::<u32>().read() };
        if status & 2 != 0 {
            Err(Error::BackendFailure)
        } else if status != 0 {
            Err(Error::BackendCapacity)
        } else {
            Ok(())
        }
    }
}
pub(super) fn encode(
    device: &Device,
    command: &CommandBufferRef,
    input: &ClosureRanges,
    output: &ClosureExits,
) -> Result<PendingClosure, Error> {
    check_submission()?;
    if input.generation != output.generation {
        return Err(Error::BackendFailure);
    }
    let buffers = [
        (&input.domains, input.domain_capacity, 8),
        (&input.entries, input.entry_capacity, 8),
        (&input.allowed, input.cell_capacity, 4),
        (&input.counts, 3, 4),
        (&output.exits, output.capacity, 8),
        (&output.counts, 4, 4),
    ];
    for (index, (buffer, capacity, width)) in buffers.iter().enumerate() {
        u32::try_from(*capacity).map_err(|_| Error::BackendCapacity)?;
        let bytes = capacity.checked_mul(*width).ok_or(Error::BackendCapacity)?;
        if bytes as u64 > buffer.buffer.length()
            || buffers[index + 1..]
                .iter()
                .any(|(other, _, _)| std::ptr::eq(&*buffer.buffer, &*other.buffer))
        {
            return Err(Error::BackendFailure);
        }
    }
    let owners = Scan::new(device, input.cell_capacity)?;
    let ends = Scan::new(device, input.cell_capacity)?;
    let counts = Scan::new(device, input.entry_capacity)?;
    let cell_levels = owners.totals.len();
    let entry_levels = counts.totals.len();
    let pending = PendingClosure {
        command: command.to_owned(),
        owners,
        ends,
        counts,
        heads: MappedBuffer::zeroed::<u32>(device, input.cell_capacity)?,
        seeds: MappedBuffer::zeroed::<u32>(device, input.cell_capacity)?,
        lengths: MappedBuffer::zeroed::<u32>(device, 3)?,
        arguments: MappedBuffer::zeroed::<u32>(device, (cell_levels + entry_levels + 1) * 3)?,
        status: MappedBuffer::zeroed::<u32>(device, 1)?,
    };
    let parameters = [
        input.domain_capacity as u32,
        input.entry_capacity as u32,
        input.cell_capacity as u32,
        output.capacity as u32,
        cell_levels as u32,
        entry_levels as u32,
    ];
    let launch =
        |name, buffers: &[&BufferRef], indirect: Option<(&BufferRef, u64)>| -> Result<(), Error> {
            dispatch(
                command,
                &pipeline_named(SOURCE, name)?,
                name,
                buffers,
                &parameters,
                1,
                indirect,
            );
            Ok(())
        };
    let cells = Some((&*pending.arguments.buffer, 0));
    let entries = Some((&*pending.arguments.buffer, cell_levels as u64 * 12));
    let domains = Some((
        &*pending.arguments.buffer,
        (cell_levels + entry_levels) as u64 * 12,
    ));
    launch(
        "closure_initialize",
        &[
            &input.counts.buffer,
            &pending.lengths.buffer,
            &pending.arguments.buffer,
            &pending.status.buffer,
            &output.counts.buffer,
        ],
        None,
    )?;
    launch(
        "closure_initialize_cells",
        &[
            &input.allowed.buffer,
            &pending.seeds.buffer,
            &pending.heads.buffer,
            &pending.lengths.buffer,
            &pending.status.buffer,
        ],
        cells,
    )?;
    launch(
        "closure_domains",
        &[
            &input.domains.buffer,
            &pending.heads.buffer,
            &pending.lengths.buffer,
            &pending.status.buffer,
        ],
        domains,
    )?;
    launch(
        "closure_entries",
        &[
            &input.domains.buffer,
            &input.entries.buffer,
            &pending.seeds.buffer,
            &pending.lengths.buffer,
            &pending.status.buffer,
        ],
        entries,
    )?;
    launch(
        "closure_runs",
        &[
            &input.allowed.buffer,
            &pending.seeds.buffer,
            &pending.heads.buffer,
            &pending.owners.values.buffer,
            &pending.lengths.buffer,
            &pending.status.buffer,
        ],
        cells,
    )?;
    pending.owners.encode(
        command,
        &pending.lengths.buffer,
        &pending.arguments.buffer,
        0,
        2,
        false,
    )?;
    launch(
        "closure_ends",
        &[
            &pending.owners.values.buffer,
            &pending.heads.buffer,
            &pending.ends.values.buffer,
            &pending.lengths.buffer,
            &pending.status.buffer,
        ],
        cells,
    )?;
    pending.ends.encode(
        command,
        &pending.lengths.buffer,
        &pending.arguments.buffer,
        0,
        2,
        false,
    )?;
    launch(
        "closure_count",
        &[
            &input.domains.buffer,
            &input.entries.buffer,
            &pending.owners.values.buffer,
            &pending.counts.values.buffer,
            &pending.lengths.buffer,
            &pending.status.buffer,
        ],
        entries,
    )?;
    pending.counts.encode(
        command,
        &pending.lengths.buffer,
        &pending.arguments.buffer,
        cell_levels as u64 * 12,
        1,
        true,
    )?;
    launch(
        "closure_boundary",
        &[
            &pending.counts.totals.last().unwrap().buffer,
            &output.counts.buffer,
            &pending.status.buffer,
        ],
        None,
    )?;
    launch(
        "closure_scatter",
        &[
            &input.domains.buffer,
            &input.entries.buffer,
            &pending.owners.values.buffer,
            &pending.ends.values.buffer,
            &pending.counts.values.buffer,
            &output.exits.buffer,
            &output.counts.buffer,
            &pending.lengths.buffer,
        ],
        entries,
    )?;
    check_submission()?;
    Ok(pending)
}
