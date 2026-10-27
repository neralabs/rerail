//! Batch-local exact active keys and compact pending-offer peer domains.
use super::super::metal_backend::pipeline_named;
use super::super::metal_parallel::{Scan, encode};
use super::{Error, MappedBuffer};
use metal::{CommandBufferRef, Device};

pub(super) struct Input<'a> {
    pub columns: &'a MappedBuffer,
    pub spans: &'a MappedBuffer,
    pub digits: &'a MappedBuffer,
    pub cursors: &'a MappedBuffer,
    pub repeats: &'a MappedBuffer,
    pub captures: &'a MappedBuffer,
    pub candidates: &'a MappedBuffer,
    pub expansion: &'a Scan,
    pub task_scan: &'a Scan,
    pub meta: &'a MappedBuffer,
    pub status: &'a MappedBuffer,
}

pub(super) struct Work {
    active: Option<super::active::Work>,
    pending: Option<Pending>,
}
impl Work {
    pub(super) fn new(
        device: &Device,
        output: usize,
        pairs: usize,
        cells: usize,
        tasks: usize,
        needs: super::super::metal_successor_prepare::ComparisonNeeds,
    ) -> Result<Self, Error> {
        Ok(Self {
            active: needs
                .active
                .then(|| {
                    super::active::Work::new(device, output, needs.active_states.unwrap_or(output))
                })
                .transpose()?,
            pending: needs
                .pending
                .then(|| Pending::new(device, output, pairs, cells, tasks))
                .transpose()?,
        })
    }
    pub(super) fn into_publication(self) -> Option<(MappedBuffer, MappedBuffer)> {
        self.pending.map(|pending| (pending.offers, pending.alive))
    }
    pub(super) fn active_slots(&self) -> usize {
        self.active.as_ref().map_or(0, |work| work.capacity)
    }
    pub(super) fn heads(&self) -> Option<&MappedBuffer> {
        self.active.as_ref().map(|work| &work.heads)
    }
    #[cfg(test)]
    pub(super) fn offers(&self) -> Option<&MappedBuffer> {
        self.pending.as_ref().map(|work| &work.offers)
    }
    #[cfg(test)]
    pub(super) fn alive(&self) -> Option<&MappedBuffer> {
        self.pending.as_ref().map(|work| &work.alive)
    }
    pub(super) fn encode(
        &self,
        command: &CommandBufferRef,
        source: &str,
        parameters: &[u32],
        input: Input<'_>,
    ) -> Result<(), Error> {
        if let Some(active) = &self.active {
            active.encode(
                command,
                source,
                parameters,
                super::active::Input {
                    columns: input.columns,
                    spans: input.spans,
                    digits: input.digits,
                    cursors: input.cursors,
                    repeats: input.repeats,
                    captures: input.captures,
                    candidates: input.candidates,
                    total: input.expansion.total(),
                    meta: input.meta,
                    status: input.status,
                },
            )?;
        }
        if let Some(pending) = &self.pending {
            pending.encode(command, source, parameters, input)?;
        }
        Ok(())
    }
}

struct Pending {
    comparisons: Scan,
    peers: Scan,
    peer_slots: MappedBuffer,
    items: MappedBuffer,
    domains: MappedBuffer,
    pairs: MappedBuffer,
    pub offers: MappedBuffer,
    pub alive: MappedBuffer,
    output: usize,
    pair_capacity: usize,
    work: usize,
}
impl Pending {
    pub(super) fn new(
        device: &Device,
        output: usize,
        pair_capacity: usize,
        work: usize,
        tasks: usize,
    ) -> Result<Self, Error> {
        Ok(Self {
            comparisons: Scan::new(device, output)?,
            peers: Scan::new(device, tasks)?,
            peer_slots: MappedBuffer::zeroed::<[u32; 2]>(device, output)?,
            items: MappedBuffer::zeroed::<u32>(device, output)?,
            domains: MappedBuffer::zeroed::<[u32; 2]>(device, output)?,
            pairs: MappedBuffer::zeroed::<[u32; 4]>(device, pair_capacity)?,
            offers: MappedBuffer::zeroed::<u32>(device, output)?,
            alive: MappedBuffer::zeroed::<u32>(device, output)?,
            output,
            pair_capacity,
            work,
        })
    }
    pub(super) fn encode(
        &self,
        command: &CommandBufferRef,
        source: &str,
        parameters: &[u32],
        input: Input<'_>,
    ) -> Result<(), Error> {
        let Input {
            columns,
            spans,
            digits,
            cursors,
            repeats,
            captures,
            candidates,
            expansion,
            task_scan,
            meta,
            status,
        } = input;
        let out = self.output;
        let pair_capacity = self.pair_capacity;
        let work = self.work;
        let comparisons = &self.comparisons;
        let peers = &self.peers;
        let peer_slots = &self.peer_slots;
        let task_items = &self.items;
        let domains = &self.domains;
        let pairs = &self.pairs;
        let offers = &self.offers;
        let alive = &self.alive;
        let st = &status.buffer;
        let run = |name, buffers: &[&metal::BufferRef], count: usize| -> Result<(), Error> {
            encode(
                command,
                &pipeline_named(source, name)?,
                name,
                buffers,
                parameters,
                count.div_ceil(256).max(1),
                None,
            );
            Ok(())
        };
        run(
            "successor_peer_counts",
            &[
                &columns.buffer,
                &candidates.buffer,
                expansion.total(),
                &meta.buffer,
                &peers.values.buffer,
                &peer_slots.buffer,
                st,
            ],
            out,
        )?;
        peers.encode(command)?;
        run(
            "successor_peer_items",
            &[
                &candidates.buffer,
                expansion.total(),
                &meta.buffer,
                &peer_slots.buffer,
                &peers.values.buffer,
                &task_items.buffer,
                st,
            ],
            out,
        )?;
        run(
            "successor_pair_counts",
            &[
                &columns.buffer,
                &spans.buffer,
                &candidates.buffer,
                expansion.total(),
                &peers.values.buffer,
                peers.total(),
                &comparisons.values.buffer,
                &domains.buffer,
                &meta.buffer,
                st,
            ],
            out,
        )?;
        comparisons.encode(command)?;
        run(
            "successor_pair_initialize",
            &[comparisons.total(), &pairs.buffer, st],
            pair_capacity,
        )?;
        run(
            "successor_compare",
            &[
                &columns.buffer,
                &spans.buffer,
                &digits.buffer,
                &cursors.buffer,
                &repeats.buffer,
                &captures.buffer,
                expansion.total(),
                &domains.buffer,
                &task_items.buffer,
                &comparisons.values.buffer,
                comparisons.total(),
                &pairs.buffer,
                st,
            ],
            work,
        )?;
        run(
            "successor_ranks",
            &[
                &columns.buffer,
                &spans.buffer,
                &digits.buffer,
                expansion.total(),
                &domains.buffer,
                &task_items.buffer,
                &comparisons.values.buffer,
                comparisons.total(),
                &pairs.buffer,
                &meta.buffer,
                &candidates.buffer,
                st,
            ],
            pair_capacity,
        )?;
        run(
            "successor_rank_ancestors",
            &[
                &columns.buffer,
                &candidates.buffer,
                expansion.total(),
                &domains.buffer,
                &task_items.buffer,
                &comparisons.values.buffer,
                comparisons.total(),
                &pairs.buffer,
                &spans.buffer,
                &meta.buffer,
                st,
                &task_scan.values.buffer,
                &offers.buffer,
            ],
            pair_capacity.max(out),
        )?;
        run(
            "successor_owners",
            &[
                &columns.buffer,
                &spans.buffer,
                &candidates.buffer,
                expansion.total(),
                &domains.buffer,
                &task_items.buffer,
                &comparisons.values.buffer,
                comparisons.total(),
                &pairs.buffer,
                &meta.buffer,
                st,
            ],
            pair_capacity,
        )?;
        run(
            "successor_offer_liveness",
            &[
                &columns.buffer,
                expansion.total(),
                &domains.buffer,
                &task_items.buffer,
                &comparisons.values.buffer,
                comparisons.total(),
                &pairs.buffer,
                &meta.buffer,
                &alive.buffer,
                st,
                &candidates.buffer,
            ],
            pair_capacity.max(out),
        )?;
        Ok(())
    }
}
