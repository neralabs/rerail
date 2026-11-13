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
                   constant uint& nblocks [[buffer(10)]],
                   constant uint& maxd [[buffer(11)]],
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
            if (j != NONE) { end[j] = pos; l[dep] = NONE; }
            else { closes[2 * (b.close_at + nc)] = pos; closes[2 * (b.close_at + nc) + 1] = dep; nc++; }
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
                          constant uint& ncloses [[buffer(4)]],
                          constant uint& maxd [[buffer(5)]],
                          uint id [[thread_position_in_grid]]) {
    if (id >= ncloses) return;
    uint d = uint(closes[2 * id + 1]);
    end[snap[ulong(close_block[id]) * maxd + d]] = closes[2 * id];
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
    pub seconds: [f64; 4],
    pub blocks: usize,
    pub unresolved_closes: usize,
}

/// Runs the three passes over every file's blocks. Nodes are numbered across files in order (a file's parent
/// is offset by its first node).
pub fn derive(files: &[FileBlocks], max_depth: usize) -> Result<Derived, String> {
    use metal::{Device, MTLResourceOptions, MTLSize};
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
    let (mut lead, mut pops, mut trails) = (Vec::new(), Vec::new(), Vec::new());
    let (mut blks, mut fl, mut node_block, mut close_block) = (Vec::<Blk>::new(), Vec::<u32>::new(), Vec::<u32>::new(), Vec::<u32>::new());
    let mut close_at = 0u32;
    for (fi, f) in files.iter().enumerate() {
        let (n0, t0) = (lead.len() as u32, trails.len() as u32);
        lead.extend(f.events.opens.iter().map(|o| o.lead));
        pops.extend(f.events.opens.iter().map(|o| o.pops));
        trails.extend_from_slice(&f.events.trails);
        fl.extend([blks.len() as u32, f.places.len() as u32]);
        let mut t = t0;
        for (k, (pl, &(n, nt))) in f.places.iter().zip(&f.counts).enumerate() {
            let b = blks.len() as u32;
            // Closes this block cannot resolve: the entry-stack nodes it closes. The file's last block closes
            // everything down to depth 0 (its closes after the last open are not in `min_depth`).
            let unresolved = if k + 1 == f.places.len() { pl.entry_depth } else { pl.entry_depth - pl.min_depth };
            blks.push(Blk { first: n0 + pl.first_node as u32, n, trail: t, nt, base: pl.base, entry: pl.entry_depth, file: fi as u32, close_at, min_depth: pl.min_depth });
            node_block.extend(std::iter::repeat_n(b, n as usize));
            close_block.extend(std::iter::repeat_n(b, unresolved as usize));
            close_at += unresolved;
            t += nt;
        }
    }
    // The end sentinel, so pass B reads the last block's close range.
    let nblocks = blks.len();
    blks.push(Blk { first: 0, n: 0, trail: 0, nt: 0, base: 0, entry: 0, file: 0, close_at, min_depth: 0 });
    let nn = lead.len();
    let maxd = max_depth as u32 + 1;
    let device = Device::system_default().ok_or("no Metal device")?;
    let lib = device.new_library_with_source(SOURCE, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let pipe = |name: &str| -> Result<metal::ComputePipelineState, String> {
        let f = lib.get_function(name, None).map_err(|e| e.to_string())?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
    };
    let (pa, pb, pc, pcc) = (pipe("pass_a")?, pipe("pass_b")?, pipe("pass_c")?, pipe("pass_c_closes")?);
    let opts = MTLResourceOptions::StorageModeShared;
    let bytes = |p: *const u8, n: usize| device.new_buffer_with_data(p.cast(), n.max(4) as u64, opts);
    let u32s = |v: &[u32]| if v.is_empty() { bytes([0u8; 4].as_ptr(), 4) } else { bytes(v.as_ptr().cast(), v.len() * 4) };
    let bblk = bytes(blks.as_ptr().cast(), blks.len() * size_of::<Blk>());
    let (blead, bpops, btr, bfiles, bnb, bcb) = (u32s(&lead), u32s(&pops), u32s(&trails), u32s(&fl), u32s(&node_block), u32s(&close_block));
    let zeros = |n: usize| device.new_buffer(n.max(4) as u64, opts);
    let (bstart, bend, bdepth, bparent) = (zeros(nn * 8), zeros(nn * 8), zeros(nn * 4), zeros(nn * 4));
    let (blast, bsnap, bcloses) = (zeros(nblocks * maxd as usize * 4), zeros(nblocks * maxd as usize * 4), zeros(close_at as usize * 16));
    let queue = device.new_command_queue();
    let (nb, nf, nnu, ncl) = (nblocks as u32, files.len() as u32, nn as u32, close_at);
    let mut secs = [0.0; 4];
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
    for _ in 0..2 {
        secs[0] = run(&pa, &[&bblk, &blead, &bpops, &btr, &bstart, &bend, &bdepth, &bparent, &blast, &bcloses], &[nb, maxd], nb);
        secs[1] = run(&pb, &[&bblk, &bfiles, &blast, &bsnap, &bcloses], &[nf, maxd], nf);
        secs[2] = run(&pc, &[&bnb, &bparent, &bsnap], &[nnu, maxd], nnu);
        secs[3] = run(&pcc, &[&bcloses, &bcb, &bsnap, &bend], &[ncl, maxd], ncl);
    }
    // SAFETY: shared buffers of the stated lengths; the GPU is done with them.
    let read = |b: &metal::Buffer, n: usize| unsafe { std::slice::from_raw_parts(b.contents().cast::<u8>(), n).to_vec() };
    let to64 = |v: Vec<u8>| v.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>();
    let to32 = |v: Vec<u8>| v.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>();
    Ok(Derived {
        start: to64(read(&bstart, nn * 8)),
        end: to64(read(&bend, nn * 8)),
        depth: to32(read(&bdepth, nn * 4)),
        parent: to32(read(&bparent, nn * 4)),
        seconds: secs,
        blocks: nblocks,
        unresolved_closes: close_at as usize,
    })
}
