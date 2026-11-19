//! The building blocks of the preprocessor's grid passes (decision 0448 §3), `pp_blocks.metal`:
//! a scan, a segmented scan (add or max), a stable radix sort of 64-bit keys, a search and an
//! expand. Each is sized on the device: its count is a word of a device buffer, a plan kernel
//! makes the levels' counts and the indirect arguments, and the host encodes every dispatch
//! without reading one.
//!
//! Scratch is placed when a block is made, for a capacity the host knows (the arena's), so
//! encoding never allocates. A count past the capacity is the caller's to refuse before.

use crate::Error;
use crate::launch::CheckedComputeEncoder;
use crate::runtime::{MappedBuffer, pipeline_named_constant};
use metal::{BufferRef, CommandBufferRef, MTLSize};

/// The evaluator's one source: the tape reader and the front passes (`pp_eval.metal`), these
/// blocks, the work queue (`work_queue.metal`) and the expansion (`pp_expand.metal`).
pub const SOURCE: &str = concat!(include_str!("pp_eval.metal"), "\n", include_str!("pp_blocks.metal"), "\n", include_str!("work_queue.metal"), "\n", include_str!("pp_expand.metal"));

/// A dispatch's size: groups known to the host, or indirect arguments at a byte offset.
#[derive(Clone, Copy)]
pub enum Grid<'b> {
    Groups(u64),
    Indirect(&'b BufferRef, u64),
}

/// The threads of a group.
pub const TG: u64 = 256;
/// A scan block's elements.
const BLOCK: usize = 1024;

/// Encodes passes of `source` into a command buffer.
pub struct Enc<'a> {
    pub command: &'a CommandBufferRef,
    pub source: &'a str,
}

impl Enc<'_> {
    /// Kernel `name` (with `constant` at function constant 0) over `buffers`, then the words
    /// `words` each at the next index.
    pub fn run(&self, name: &str, constant: Option<u32>, buffers: &[&BufferRef], words: &[u32], grid: Grid<'_>) -> Result<(), Error> {
        let pipe = pipeline_named_constant(self.source, name, constant)?;
        let e = self.command.new_compute_command_encoder();
        e.set_compute_pipeline_state(&pipe);
        e.checked_set_buffers(self.command, buffers);
        for (k, w) in words.iter().enumerate() {
            e.set_bytes((buffers.len() + k) as u64, 4, std::ptr::from_ref(w).cast());
        }
        let group = MTLSize::new(TG, 1, 1);
        match grid {
            Grid::Groups(g) => e.checked_dispatch_thread_groups(self.command, MTLSize::new(g.max(1), 1, 1), group),
            Grid::Indirect(b, at) => e.checked_dispatch_thread_groups_indirect(self.command, b, at, group),
        }
        e.end_encoding();
        Ok(())
    }
}

fn zeroed(n: usize) -> Result<MappedBuffer, Error> {
    MappedBuffer::zeroed::<u32>(&crate::device()?, n.max(1))
}

/// An exclusive scan of up to `capacity` u32s, its total in `total`. Only the levels the
/// capacity needs are encoded: 2 launches to 1,024 elements, 4 to 2^20, 6 past it.
pub struct Scan {
    levels: u32,
    plan: MappedBuffer,
    sums1: MappedBuffer,
    out1: MappedBuffer,
    sums2: MappedBuffer,
    out2: MappedBuffer,
    pub total: MappedBuffer,
}

impl Scan {
    pub fn new(capacity: usize) -> Result<Self, Error> {
        let n1 = capacity.div_ceil(BLOCK).max(1);
        let n2 = n1.div_ceil(BLOCK).max(1);
        if n2 > BLOCK {
            return Err(Error::BackendCapacity);
        }
        let levels = if capacity <= BLOCK { 1 } else if capacity <= BLOCK * BLOCK { 2 } else { 3 };
        Ok(Scan { levels, plan: zeroed(20)?, sums1: zeroed(n1)?, out1: zeroed(n1)?, sums2: zeroed(n2)?, out2: zeroed(n2)?, total: zeroed(1)? })
    }

    /// `out` = the exclusive scan of `input`'s first `n[at]` words.
    pub fn encode(&self, enc: &Enc<'_>, n: &BufferRef, at: u32, input: &BufferRef, out: &BufferRef) -> Result<(), Error> {
        let plan = &self.plan.buffer;
        enc.run("pb_scan_plan", None, &[n, plan], &[at], Grid::Groups(1))?;
        match self.levels {
            // One block: its sum is the total.
            1 => enc.run("pb_scan_block", None, &[input, out, &self.total.buffer, plan], &[0], Grid::Groups(1)),
            // Blocks, then one block of their sums.
            2 => {
                enc.run("pb_scan_block", None, &[input, out, &self.sums1.buffer, plan], &[0], Grid::Indirect(plan, 16))?;
                enc.run("pb_scan_block", None, &[&self.sums1.buffer, &self.out1.buffer, &self.total.buffer, plan], &[1], Grid::Groups(1))?;
                enc.run("pb_scan_add", None, &[out, &self.out1.buffer, plan], &[0], Grid::Indirect(plan, 52))
            }
            _ => {
                enc.run("pb_scan_block", None, &[input, out, &self.sums1.buffer, plan], &[0], Grid::Indirect(plan, 16))?;
                enc.run("pb_scan_block", None, &[&self.sums1.buffer, &self.out1.buffer, &self.sums2.buffer, plan], &[1], Grid::Indirect(plan, 28))?;
                enc.run("pb_scan_block", None, &[&self.sums2.buffer, &self.out2.buffer, &self.total.buffer, plan], &[2], Grid::Indirect(plan, 40))?;
                enc.run("pb_scan_add", None, &[&self.out1.buffer, &self.out2.buffer, plan], &[1], Grid::Indirect(plan, 64))?;
                enc.run("pb_scan_add", None, &[out, &self.out1.buffer, plan], &[0], Grid::Indirect(plan, 52))
            }
        }
    }
}

/// The segmented scan's operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegOp {
    Add = 0,
    Max = 1,
}

/// An inclusive segmented scan of up to `capacity` u32s, each with a head flag (a word, 0 or 1).
pub struct SegScan {
    plan: MappedBuffer,
    seen0: MappedBuffer,
    levels: [(MappedBuffer, MappedBuffer, MappedBuffer, MappedBuffer); 2],
    last: (MappedBuffer, MappedBuffer, MappedBuffer, MappedBuffer),
}

impl SegScan {
    pub fn new(capacity: usize) -> Result<Self, Error> {
        let n1 = capacity.div_ceil(BLOCK).max(1);
        let n2 = n1.div_ceil(BLOCK).max(1);
        if n2 > BLOCK {
            return Err(Error::BackendCapacity);
        }
        // A level's aggregates (values, flags), its scan, and its `seen`.
        let level = |n: usize| -> Result<_, Error> { Ok((zeroed(n)?, zeroed(n)?, zeroed(n)?, zeroed(n)?)) };
        Ok(SegScan { plan: zeroed(20)?, seen0: zeroed(capacity)?, levels: [level(n1)?, level(n2)?], last: level(1)? })
    }

    /// `out` = the inclusive scan of `input`'s first `n[at]` words, cut at `heads`.
    pub fn encode(&self, enc: &Enc<'_>, op: SegOp, n: &BufferRef, at: u32, input: &BufferRef, heads: &BufferRef, out: &BufferRef) -> Result<(), Error> {
        let plan = &self.plan.buffer;
        let c = Some(op as u32);
        let ([(v1, f1, o1, s1), (v2, f2, o2, s2)], (v3, f3, _, _)) = (&self.levels, &self.last);
        enc.run("pb_scan_plan", None, &[n, plan], &[at], Grid::Groups(1))?;
        enc.run("pb_seg_block", c, &[input, heads, out, &self.seen0.buffer, &v1.buffer, &f1.buffer, plan], &[0], Grid::Indirect(plan, 16))?;
        enc.run("pb_seg_block", c, &[&v1.buffer, &f1.buffer, &o1.buffer, &s1.buffer, &v2.buffer, &f2.buffer, plan], &[1], Grid::Indirect(plan, 28))?;
        enc.run("pb_seg_block", c, &[&v2.buffer, &f2.buffer, &o2.buffer, &s2.buffer, &v3.buffer, &f3.buffer, plan], &[2], Grid::Indirect(plan, 40))?;
        enc.run("pb_seg_add", c, &[&o1.buffer, &s1.buffer, &o2.buffer, plan], &[1], Grid::Indirect(plan, 64))?;
        enc.run("pb_seg_add", c, &[out, &self.seen0.buffer, &o1.buffer, plan], &[0], Grid::Indirect(plan, 52))
    }
}

/// A stable LSD radix sort of up to `capacity` u64 keys, each with a u32 payload (its index
/// before the sort, by `iota`).
pub struct Sort {
    plan: MappedBuffer,
    hist: MappedBuffer,
    scanned: MappedBuffer,
    scan: Scan,
    pub keys_b: MappedBuffer,
    pub vals_a: MappedBuffer,
    pub vals_b: MappedBuffer,
}

impl Sort {
    pub fn new(capacity: usize) -> Result<Self, Error> {
        let h = capacity.div_ceil(TG as usize).max(1) * 256;
        let device = crate::device()?;
        Ok(Sort {
            plan: zeroed(8)?,
            hist: zeroed(h)?,
            scanned: zeroed(h)?,
            scan: Scan::new(h)?,
            keys_b: MappedBuffer::zeroed::<u64>(&device, capacity.max(1))?,
            vals_a: zeroed(capacity)?,
            vals_b: zeroed(capacity)?,
        })
    }

    /// Sorts `keys` (the first `n[at]`) by their low `bits` bits, a multiple of 8, stably. The
    /// keys end in `keys` and their payloads in `vals_a` (the passes are even), or in `keys_b` and
    /// `vals_b`: `sorted` says which.
    pub fn encode(&self, enc: &Enc<'_>, n: &BufferRef, at: u32, keys: &BufferRef, bits: u32) -> Result<(), Error> {
        let plan = &self.plan.buffer;
        enc.run("pb_sort_plan", None, &[n, plan], &[at], Grid::Groups(1))?;
        enc.run("pb_iota", None, &[&self.vals_a.buffer, plan], &[], Grid::Indirect(plan, 16))?;
        for p in 0..bits.div_ceil(8) {
            let (a, b): ([&BufferRef; 2], [&BufferRef; 2]) = ([keys, &self.vals_a.buffer], [&self.keys_b.buffer, &self.vals_b.buffer]);
            let ([ki, vi], [ko, vo]) = if p % 2 == 0 { (a, b) } else { (b, a) };
            enc.run("pb_hist", None, &[ki, &self.hist.buffer, plan], &[8 * p], Grid::Indirect(plan, 16))?;
            self.scan.encode(enc, plan, 2, &self.hist.buffer, &self.scanned.buffer)?;
            enc.run("pb_scatter", None, &[ki, vi, ko, vo, &self.scanned.buffer, plan], &[8 * p], Grid::Indirect(plan, 16))?;
        }
        Ok(())
    }

    /// Where the sorted keys and payloads are after `bits`: in the input and `vals_a`, or in
    /// `keys_b` and `vals_b`.
    pub fn in_input(bits: u32) -> bool {
        bits.div_ceil(8) % 2 == 0
    }
}

#[cfg(test)]
mod tests;
