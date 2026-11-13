//! Format 16 decoded on the GPU straight into the columns readers take (E7): kind, field, flags,
//! membership mask, start, end, parent and subtree end, one array each, with nothing returned to the host
//! between passes.
//!
//! E7 found that readers sweep kind and flags over every node, and take the other parts sparsely, except in
//! tree walks (parents, spans, siblings). Columns serve both: a sweep reads one array, and a walk reads one
//! entry per step (the next sibling of `i` is `sub[i]`).
//!
//! The passes, each a dispatch of its own so each has its own GPU time:
//!
//! 1. Huffman: one thread per checkpoint segment decodes its sequence and literal bytes (`gpu::HUFF_SOURCE`).
//! 2. Index (span LZ's sequence-index form only): one thread per segment records each sequence's offsets.
//! 3. Span: one thread per segment writes its block's byte planes (`gpu::SPAN_SOURCE`).
//! 4. Unplane: one thread per node gathers its record from the planes. One thread per close gathers that
//!    close's trailing distance.
//! 5. Scan: positions, depth, parents and subtree ends (`scan`).

use crate::blocks::Placement;
use crate::gpu::{self, SpanInputs};
use crate::huff;
use crate::scan::{self, Meta, Passes};
use crate::span::Ptr;

const SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

// A laid-out block (byte planes, `layout::lay`): where it starts in the span pass's output, its opens and
// closes, and its first node and first close globally.
struct UB { uint out; uint n; uint m; uint node; uint trail; };

kernel void unplane_nodes(device const UB* blocks [[buffer(0)]],
                          device const uint* node_block [[buffer(1)]],
                          device const uchar* laid [[buffer(2)]],
                          device ushort* kind [[buffer(3)]],
                          device uchar* field [[buffer(4)]],
                          device uchar* flags [[buffer(5)]],
                          device ushort* memb [[buffer(6)]],
                          device uint* lead [[buffer(7)]],
                          device uint* pops [[buffer(8)]],
                          constant uint& nnodes [[buffer(9)]],
                          uint id [[thread_position_in_grid]]) {
    if (id >= nnodes) return;
    UB b = blocks[node_block[id]];
    uint i = id - b.node, n = b.n;
    // The header (4 u32s), then the planes: kind 2, field 1, flags 1, membership 2, lead 4, pops 4.
    device const uchar* p = laid + b.out + 16 + i;
    kind[id] = ushort(p[0]) | ushort(p[n]) << 8;
    field[id] = p[2 * n];
    flags[id] = p[3 * n];
    memb[id] = ushort(p[4 * n]) | ushort(p[5 * n]) << 8;
    lead[id] = uint(p[6 * n]) | uint(p[7 * n]) << 8 | uint(p[8 * n]) << 16 | uint(p[9 * n]) << 24;
    pops[id] = uint(p[10 * n]) | uint(p[11 * n]) << 8 | uint(p[12 * n]) << 16 | uint(p[13 * n]) << 24;
}

kernel void unplane_trails(device const UB* blocks [[buffer(0)]],
                           device const uint* trail_block [[buffer(1)]],
                           device const uchar* laid [[buffer(2)]],
                           device uint* trails [[buffer(3)]],
                           constant uint& ntrails [[buffer(4)]],
                           uint id [[thread_position_in_grid]]) {
    if (id >= ntrails) return;
    UB b = blocks[trail_block[id]];
    uint j = id - b.trail, m = b.m;
    device const uchar* q = laid + b.out + 16 + 14 * b.n + j;
    trails[id] = uint(q[0]) | uint(q[m]) << 8 | uint(q[2 * m]) << 16 | uint(q[3 * m]) << 24;
}
"#;

/// A file as the pipeline takes it: its language, its entropy-coded span LZ blocks (byte planes inside),
/// placements, and each block's (opens, closes).
pub struct FileIn<'a> {
    /// Which of `decode`'s languages (its dictionary and tables).
    pub lang: usize,
    pub coded: &'a [Vec<u8>],
    pub places: &'a [Placement],
    pub counts: &'a [(u32, u32)],
}

/// The columns, read back for checking, and each pass's GPU seconds.
pub struct Columns {
    pub kind: Vec<u16>,
    pub field: Vec<u8>,
    pub flags: Vec<u8>,
    pub memb: Vec<u16>,
    pub derived: scan::Derived,
    /// Huffman, index, span, unplane, then the scan's four.
    pub seconds: [f64; 8],
    pub threads: usize,
    pub bytes_in: usize,
    pub laid_bytes: usize,
}

pub const PASSES: [&str; 8] = ["huff", "index", "span", "unplane", "scan A", "scan B", "scan C", "closes"];

/// A language's dictionary and Huffman tables.
pub struct Lang<'a> {
    pub dict: &'a [u8],
    pub tables: &'a (huff::Table, huff::Table),
}

/// Every file in one set of dispatches, whatever its language: each Huffman task names its language's
/// tables, and each block its dictionary.
pub fn decode(files: &[FileIn], langs: &[Lang], ptr: Ptr, max_depth: usize) -> Result<Columns, String> {
    use metal::{Device, MTLResourceOptions, MTLSize};
    let device = Device::system_default().ok_or("no Metal device")?;
    let meta = Meta::new(files.iter().map(|f| (f.places, f.counts)));
    // Host preparation: tables of offsets only; no block is decoded on the host.
    let (mut sbits, mut lbits, mut htasks, mut inputs) = (Vec::new(), Vec::new(), Vec::<[u32; 7]>::new(), SpanInputs::new());
    let (mut dict, mut dict_at) = (Vec::new(), Vec::new());
    for l in langs {
        dict_at.push((dict.len() as u32, l.dict.len() as u32));
        dict.extend_from_slice(l.dict);
    }
    let (mut ub, mut trail_block) = (Vec::<[u32; 5]>::new(), Vec::<u32>::new());
    let (mut nseq, mut nlit, mut out, mut bytes_in, mut gb) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for f in files {
        for (c, &(n, m)) in f.coded.iter().zip(f.counts) {
            bytes_in += c.len();
            let p = huff::coded(c);
            let checks: Vec<[u32; 3]> = p.checks.iter().map(|c| [c[0], c[1], c[2]]).collect();
            inputs.add(out as u32, nseq as u32, nlit as u32, &checks, p.nseq, p.k, dict_at[f.lang]);
            let (sb, lb) = (sbits.len() as u64 * 8, lbits.len() as u64 * 8);
            sbits.extend_from_slice(p.seq_bits);
            lbits.extend_from_slice(p.lit_bits);
            for (i, ck) in p.checks.iter().enumerate() {
                let next = p.checks.get(i + 1);
                let s_end = next.map_or(p.nseq_bytes, |n| n[2] as usize) as u32;
                let l_end = next.map_or(p.nlit, |n| n[1] as usize) as u32;
                htasks.push([nseq as u32 + ck[2], s_end - ck[2], (sb + u64::from(ck[3])) as u32, nlit as u32 + ck[1], l_end - ck[1], (lb + u64::from(ck[4])) as u32, f.lang as u32]);
            }
            let (node, trail) = meta.bases[gb];
            ub.push([out as u32, n, m, node, trail]);
            trail_block.extend(std::iter::repeat_n(gb as u32, m as usize));
            nseq += p.nseq_bytes;
            nlit += p.nlit;
            out += p.len;
            gb += 1;
        }
    }
    sbits.extend_from_slice(&[0; 4]);
    lbits.extend_from_slice(&[0; 4]);
    let opts = MTLResourceOptions::StorageModeShared;
    let buf = |b: &[u8]| {
        let b = if b.is_empty() { &[0u8][..] } else { b };
        device.new_buffer_with_data(b.as_ptr().cast(), b.len() as u64, opts)
    };
    // SAFETY: arrays of u32s as bytes.
    let as_bytes = |v: &[u32]| unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 4) };
    let lut = |t: &huff::Table| -> Vec<u8> { t.lut.iter().flat_map(|&(s, l)| (u16::from(s) | u16::from(l) << 8).to_le_bytes()).collect() };
    let zeros = |n: usize| device.new_buffer(n.max(4) as u64 + 8, opts);
    let compile = |src: &str, name: &str| -> Result<metal::ComputePipelineState, String> {
        let lib = device.new_library_with_source(src, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
        let f = lib.get_function(name, None).map_err(|e| e.to_string())?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
    };
    let (hp, ip, sp) = (compile(gpu::HUFF_SOURCE, "huff")?, compile(gpu::SPAN_SOURCE, "index_seqs")?, compile(gpu::SPAN_SOURCE, "decode_span")?);
    let (np, tp) = (compile(SOURCE, "unplane_nodes")?, compile(SOURCE, "unplane_trails")?);
    let (bh, bsb, blb, bsl, bll) = (buf(as_bytes(htasks.as_flattened())), buf(&sbits), buf(&lbits), buf(&langs.iter().flat_map(|l| lut(&l.tables.0)).collect::<Vec<_>>()), buf(&langs.iter().flat_map(|l| lut(&l.tables.1)).collect::<Vec<_>>()));
    drop((sbits, lbits));
    let (bseq, blit) = (zeros(nseq), zeros(nlit));
    let (bt, bd) = (buf(as_bytes(inputs.tasks.as_flattened())), buf(&dict));
    let (bb, bc, btab) = (buf(as_bytes(inputs.blocks.as_flattened())), buf(as_bytes(inputs.checks.as_flattened())), zeros(inputs.seqs as usize * 8));
    let blaid = zeros(out);
    let (bub, bnb, btb) = (buf(as_bytes(ub.as_flattened())), buf(as_bytes(meta.node_block())), buf(as_bytes(&trail_block)));
    let nn = meta.nodes;
    let (bkind, bfield, bflags, bmemb, blead, bpops, btrails) = (zeros(nn * 2), zeros(nn), zeros(nn), zeros(nn * 2), zeros(nn * 4), zeros(nn * 4), zeros(meta.trails * 4));
    let passes = Passes::new(&device, &meta, max_depth)?;
    let queue = device.new_command_queue();
    let n = inputs.tasks.len() as u32;
    let mode = gpu::span_mode(ptr, true);
    let dispatch = |p: &metal::ComputePipelineState, bufs: &[&metal::Buffer], consts: &[u32], threads: u32| -> f64 {
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
        gpu::command_seconds(cmd)
    };
    let mut secs = [0.0; 8];
    // Twice: the second run is warm (pipelines compiled, buffers resident).
    for _ in 0..2 {
        secs[0] = dispatch(&hp, &[&bh, &bsb, &blb, &bsl, &bll, &bseq, &blit], &[n], n);
        secs[1] = if mode == 4 { gpu::run_index(&queue, &ip, &bt, &bseq, &bb, &btab, n) } else { 0.0 };
        secs[2] = {
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            gpu::encode_span(enc, &sp, [&bt, &bseq, &blit, &bd, &blaid], &bb, &bc, &btab, n, dict.len() as u32, mode);
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            gpu::command_seconds(cmd)
        };
        secs[3] = dispatch(&np, &[&bub, &bnb, &blaid, &bkind, &bfield, &bflags, &bmemb, &blead, &bpops], &[nn as u32], nn as u32)
            + dispatch(&tp, &[&bub, &btb, &blaid, &btrails], &[meta.trails as u32], meta.trails as u32);
        let s = passes.run(&queue, &meta, &blead, &bpops, &btrails);
        secs[4..].copy_from_slice(&s);
    }
    // SAFETY: shared buffers of at least the stated lengths; the GPU is done with them.
    let read = |b: &metal::Buffer, len: usize| unsafe { std::slice::from_raw_parts(b.contents().cast::<u8>(), len).to_vec() };
    let to16 = |v: Vec<u8>| v.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>();
    Ok(Columns {
        kind: to16(read(&bkind, nn * 2)),
        field: read(&bfield, nn),
        flags: read(&bflags, nn),
        memb: to16(read(&bmemb, nn * 2)),
        derived: passes.read(&meta, [secs[4], secs[5], secs[6], secs[7]]),
        seconds: secs,
        threads: n as usize,
        bytes_in,
        laid_bytes: out,
    })
}
