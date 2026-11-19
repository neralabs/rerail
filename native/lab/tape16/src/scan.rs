//! Positions, depth, parents and ends on the GPU (0156 §4, T5), from events and block placements.
//!
//! - **Pass A, one thread per block.** The thread starts from its placement (base position, entry depth)
//!   and walks its events. It derives each node's start, depth, and end, plus its parent when the parent
//!   opened in the same block. It keeps `last[d]`, the most recent node opened at depth d in this block. A
//!   close or a parent that reaches above the block (a node opened in an earlier block) is left unresolved.
//! - **Pass B, one thread per file.** It gives each block the open node at each depth below its entry depth,
//!   from the previous block's snapshot and that block's still-open nodes: O(entry depth) per block.
//! - **Pass C.** One thread per node resolves its parent from its block's snapshot. One thread per
//!   unresolved close resolves that node's end.
//!
//! Every pass is bounded: a block's events, a file's blocks times the depth, one node.
//!
//! Each node's subtree end (the index after its last descendant: the next sibling's index) comes out of the
//! same closes as its end position.
//!
//! [`derive`] takes the events from the host. [`Passes::run`] takes them in device buffers, as the column
//! pipeline (`columns`) leaves them.

use crate::blocks::Placement;
use crate::events::Events;

pub const UNRESOLVED: u32 = 1 << 31;
pub const NONE: u32 = u32::MAX;

pub const SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Blk { uint first; uint n; uint trail; uint nt; ulong base; uint entry; uint file; uint close_at; uint min_depth; };
#define NONE 0xffffffffu
#define UNRESOLVED 0x80000000u
#define BACK 0x80000000u

static inline ulong apply(ulong p, uint d) { return (d & BACK) ? p - ulong(d & ~BACK) : p + ulong(d); }

kernel void pass_a(device const Blk* blocks [[buffer(0)]],
                   device const uint* lead [[buffer(1)]],
                   device const uint* pops [[buffer(2)]],
                   device const uint* trails [[buffer(3)]],
                   device ulong* start [[buffer(4)]],
                   device ulong* end [[buffer(5)]],
                   device uint* depth [[buffer(6)]],
                   device uint* parent [[buffer(7)]],
                   device uint* last [[buffer(8)]],
                   device ulong* closes [[buffer(9)]],
                   device uint* sub [[buffer(10)]],
                   constant uint& nblocks [[buffer(11)]],
                   constant uint& maxd [[buffer(12)]],
                   uint id [[thread_position_in_grid]]) {
    if (id >= nblocks) return;
    Blk b = blocks[id];
    device uint* l = last + ulong(id) * maxd;
    for (uint d = 0; d < maxd; d++) l[d] = NONE;
    ulong pos = b.base;
    uint dep = b.entry, t = b.trail, nc = 0;
    for (uint k = 0; k <= b.n; k++) {
        uint np = k < b.n ? pops[b.first + k] : (b.trail + b.nt) - t;
        for (uint c = 0; c < np; c++) {
            pos = apply(pos, trails[t++]);
            dep--;
            uint j = l[dep];
            if (j != NONE) { end[j] = pos; sub[j] = b.first + k; l[dep] = NONE; }
            else {
                ulong c = 3 * ulong(b.close_at + nc);
                closes[c] = pos; closes[c + 1] = dep; closes[c + 2] = b.first + k;
                nc++;
            }
        }
        if (k == b.n) break;
        uint i = b.first + k;
        pos = apply(pos, lead[i]);
        start[i] = pos;
        depth[i] = dep;
        parent[i] = dep == 0 ? NONE : (l[dep - 1] != NONE ? l[dep - 1] : (UNRESOLVED | (dep - 1)));
        l[dep] = i;
        dep++;
    }
}

// Pass B: per file, the open node at each depth below each block's entry depth (`snap`): only those are
// asked for (an unresolved parent or close is above the block). Below the previous block's least depth the
// previous snapshot holds; from there up, the previous block's still-open nodes (pass A's `last`) do.
kernel void pass_b(device const Blk* blocks [[buffer(0)]],
                   device const uint* files [[buffer(1)]],
                   device const uint* last [[buffer(2)]],
                   device uint* snap [[buffer(3)]],
                   device const ulong* closes [[buffer(4)]],
                   constant uint& nfiles [[buffer(5)]],
                   constant uint& maxd [[buffer(6)]],
                   uint id [[thread_position_in_grid]]) {
    if (id >= nfiles) return;
    uint b0 = files[2 * id], nb = files[2 * id + 1];
    for (uint b = b0 + 1; b < b0 + nb; b++) {
        device const uint* prev = snap + ulong(b - 1) * maxd;
        device const uint* l = last + ulong(b - 1) * maxd;
        device uint* s = snap + ulong(b) * maxd;
        uint lo = blocks[b - 1].min_depth;
        for (uint d = 0; d < blocks[b].entry; d++) s[d] = d < lo ? prev[d] : l[d];
    }
}

kernel void pass_c(device const uint* node_block [[buffer(0)]],
                   device uint* parent [[buffer(1)]],
                   device const uint* snap [[buffer(2)]],
                   constant uint& nnodes [[buffer(3)]],
                   constant uint& maxd [[buffer(4)]],
                   uint id [[thread_position_in_grid]]) {
    if (id >= nnodes) return;
    uint p = parent[id];
    if (p != NONE && (p & UNRESOLVED)) parent[id] = snap[ulong(node_block[id]) * maxd + (p & ~UNRESOLVED)];
}

kernel void pass_c_closes(device const ulong* closes [[buffer(0)]],
                          device const uint* close_block [[buffer(1)]],
                          device const uint* snap [[buffer(2)]],
                          device ulong* end [[buffer(3)]],
                          device uint* sub [[buffer(4)]],
                          constant uint& ncloses [[buffer(5)]],
                          constant uint& maxd [[buffer(6)]],
                          uint id [[thread_position_in_grid]]) {
    if (id >= ncloses) return;
    uint d = uint(closes[3 * id + 1]);
    uint j = snap[ulong(close_block[id]) * maxd + d];
    end[j] = closes[3 * id];
    sub[j] = uint(closes[3 * id + 2]);
}
"#;

/// A file's events and block placements, as pass A reads them.
pub struct FileBlocks<'a> {
    pub events: &'a Events,
    pub places: Vec<Placement>,
    /// Each block's open count and close count.
    pub counts: Vec<(u32, u32)>,
}

/// What the GPU derived, for checking against the CPU's decode.
pub struct Derived {
    pub start: Vec<u64>,
    pub end: Vec<u64>,
    pub depth: Vec<u32>,
    pub parent: Vec<u32>,
    /// The index after each node's subtree.
    pub sub: Vec<u32>,
    pub seconds: [f64; 4],
    pub blocks: usize,
    pub unresolved_closes: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Blk {
    first: u32,
    n: u32,
    trail: u32,
    nt: u32,
    base: u64,
    entry: u32,
    file: u32,
    close_at: u32,
    min_depth: u32,
}

/// The passes' block table, from placements and counts alone (no events): what a reader has before any
/// block is decoded.
pub struct Meta {
    blks: Vec<Blk>,
    files: Vec<u32>,
    node_block: Vec<u32>,
    close_block: Vec<u32>,
    pub nodes: usize,
    pub trails: usize,
    pub blocks: usize,
    pub closes: usize,
    /// Each block's first node and first trail, globally.
    pub bases: Vec<(u32, u32)>,
}

impl Meta {
    /// `files`: per file, its placements and each block's (opens, closes).
    pub fn new<'a>(files: impl IntoIterator<Item = (&'a [Placement], &'a [(u32, u32)])>) -> Self {
        let mut m = Meta { blks: Vec::new(), files: Vec::new(), node_block: Vec::new(), close_block: Vec::new(), nodes: 0, trails: 0, blocks: 0, closes: 0, bases: Vec::new() };
        let mut close_at = 0u32;
        for (fi, (places, counts)) in files.into_iter().enumerate() {
            let (n0, mut t) = (m.nodes as u32, m.trails as u32);
            m.files.extend([m.blks.len() as u32, places.len() as u32]);
            for (k, (pl, &(n, nt))) in places.iter().zip(counts).enumerate() {
                let b = m.blks.len() as u32;
                // Closes this block cannot resolve: the entry-stack nodes it closes. The file's last block
                // closes everything down to depth 0 (its closes after the last open are not in `min_depth`).
                let unresolved = if k + 1 == places.len() { pl.entry_depth } else { pl.entry_depth - pl.min_depth };
                let first = n0 + pl.first_node as u32;
                m.blks.push(Blk { first, n, trail: t, nt, base: pl.base, entry: pl.entry_depth, file: fi as u32, close_at, min_depth: pl.min_depth });
                m.bases.push((first, t));
                m.node_block.extend(std::iter::repeat_n(b, n as usize));
                m.close_block.extend(std::iter::repeat_n(b, unresolved as usize));
                close_at += unresolved;
                t += nt;
                m.nodes += n as usize;
            }
            m.trails = t as usize;
        }
        m.blocks = m.blks.len();
        m.closes = close_at as usize;
        // The end sentinel, so pass B reads the last block's close range.
        m.blks.push(Blk { first: 0, n: 0, trail: 0, nt: 0, base: 0, entry: 0, file: 0, close_at, min_depth: 0 });
        m
    }

    /// Each node's block (as a device buffer would hold it).
    pub fn node_block(&self) -> &[u32] {
        &self.node_block
    }
}

/// The four passes, compiled, with their buffers for one `Meta`.
pub struct Passes {
    pipes: [metal::ComputePipelineState; 4],
    blk: metal::Buffer,
    files: metal::Buffer,
    node_block: metal::Buffer,
    close_block: metal::Buffer,
    pub start: metal::Buffer,
    pub end: metal::Buffer,
    pub depth: metal::Buffer,
    pub parent: metal::Buffer,
    pub sub: metal::Buffer,
    last: metal::Buffer,
    snap: metal::Buffer,
    closes: metal::Buffer,
    maxd: u32,
}

impl Passes {
    pub fn new(device: &metal::Device, m: &Meta, max_depth: usize) -> Result<Self, String> {
        use metal::MTLResourceOptions;
        let lib = device.new_library_with_source(SOURCE, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
        let pipe = |name: &str| -> Result<metal::ComputePipelineState, String> {
            let f = lib.get_function(name, None).map_err(|e| e.to_string())?;
            device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
        };
        let opts = MTLResourceOptions::StorageModeShared;
        let bytes = |p: *const u8, n: usize| device.new_buffer_with_data(p.cast(), n.max(4) as u64, opts);
        let u32s = |v: &[u32]| if v.is_empty() { bytes([0u8; 4].as_ptr(), 4) } else { bytes(v.as_ptr().cast(), v.len() * 4) };
        let zeros = |n: usize| device.new_buffer(n.max(4) as u64, opts);
        let maxd = max_depth as u32 + 1;
        let (nn, nb) = (m.nodes, m.blocks);
        Ok(Passes {
            pipes: [pipe("pass_a")?, pipe("pass_b")?, pipe("pass_c")?, pipe("pass_c_closes")?],
            blk: bytes(m.blks.as_ptr().cast(), m.blks.len() * size_of::<Blk>()),
            files: u32s(&m.files),
            node_block: u32s(&m.node_block),
            close_block: u32s(&m.close_block),
            start: zeros(nn * 8),
            end: zeros(nn * 8),
            depth: zeros(nn * 4),
            parent: zeros(nn * 4),
            sub: zeros(nn * 4),
            last: zeros(nb * maxd as usize * 4),
            snap: zeros(nb * maxd as usize * 4),
            closes: zeros(m.closes * 24),
            maxd,
        })
    }

    /// The four passes over `lead`, `pops` and `trails` (device buffers, indexed as `Meta` numbers them):
    /// each pass's GPU seconds.
    pub fn run(&self, queue: &metal::CommandQueue, m: &Meta, lead: &metal::Buffer, pops: &metal::Buffer, trails: &metal::Buffer) -> [f64; 4] {
        use metal::MTLSize;
        let run = |p: &metal::ComputePipelineState, bufs: &[&metal::Buffer], consts: &[u32], threads: u32| -> f64 {
            if threads == 0 {
                return 0.0;
            }
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(p);
            for (i, b) in bufs.iter().enumerate() {
                enc.set_buffer(i as u64, Some(b), 0);
            }
            for (i, v) in consts.iter().enumerate() {
                enc.set_bytes((bufs.len() + i) as u64, 4, (v as *const u32).cast());
            }
            let w = p.thread_execution_width().min(64);
            enc.dispatch_threads(MTLSize::new(u64::from(threads), 1, 1), MTLSize::new(w, 1, 1));
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            crate::gpu::command_seconds(cmd)
        };
        let (nb, nf, nn, nc, maxd) = (m.blocks as u32, (m.files.len() / 2) as u32, m.nodes as u32, m.closes as u32, self.maxd);
        let [pa, pb, pc, pcc] = &self.pipes;
        [
            run(pa, &[&self.blk, lead, pops, trails, &self.start, &self.end, &self.depth, &self.parent, &self.last, &self.closes, &self.sub], &[nb, maxd], nb),
            run(pb, &[&self.blk, &self.files, &self.last, &self.snap, &self.closes], &[nf, maxd], nf),
            run(pc, &[&self.node_block, &self.parent, &self.snap], &[nn, maxd], nn),
            run(pcc, &[&self.closes, &self.close_block, &self.snap, &self.end, &self.sub], &[nc, maxd], nc),
        ]
    }

    /// The derived columns, read back.
    pub fn read(&self, m: &Meta, seconds: [f64; 4]) -> Derived {
        let nn = m.nodes;
        // SAFETY: shared buffers of the stated lengths; the GPU is done with them.
        let read = |b: &metal::Buffer, n: usize| unsafe { std::slice::from_raw_parts(b.contents().cast::<u8>(), n).to_vec() };
        let to64 = |v: Vec<u8>| v.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>();
        let to32 = |v: Vec<u8>| v.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>();
        Derived {
            start: to64(read(&self.start, nn * 8)),
            end: to64(read(&self.end, nn * 8)),
            depth: to32(read(&self.depth, nn * 4)),
            parent: to32(read(&self.parent, nn * 4)),
            sub: to32(read(&self.sub, nn * 4)),
            seconds,
            blocks: m.blocks,
            unresolved_closes: m.closes,
        }
    }
}

/// Runs the passes over every file's blocks, the events from the host. Nodes are numbered across files in
/// order (a file's parent is offset by its first node).
pub fn derive(files: &[FileBlocks], max_depth: usize) -> Result<Derived, String> {
    use metal::{Device, MTLResourceOptions};
    let m = Meta::new(files.iter().map(|f| (&f.places[..], &f.counts[..])));
    let (mut lead, mut pops, mut trails) = (Vec::new(), Vec::new(), Vec::new());
    for f in files {
        lead.extend(f.events.opens.iter().map(|o| o.lead));
        pops.extend(f.events.opens.iter().map(|o| o.pops));
        trails.extend_from_slice(&f.events.trails);
    }
    let device = Device::system_default().ok_or("no Metal device")?;
    let passes = Passes::new(&device, &m, max_depth)?;
    let opts = MTLResourceOptions::StorageModeShared;
    let u32s = |v: &[u32]| device.new_buffer_with_data(if v.is_empty() { [0u32].as_ptr() } else { v.as_ptr() }.cast(), (v.len().max(1) * 4) as u64, opts);
    let (bl, bp, bt) = (u32s(&lead), u32s(&pops), u32s(&trails));
    let queue = device.new_command_queue();
    let mut secs = [0.0; 4];
    for _ in 0..2 {
        secs = passes.run(&queue, &m, &bl, &bp, &bt);
    }
    Ok(passes.read(&m, secs))
}
