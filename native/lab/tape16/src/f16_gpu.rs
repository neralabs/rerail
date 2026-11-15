//! E26 (t39 step 18): format 16's fixed blocks decoded on Metal, every block of a set in one dispatch.
//!
//! The blocks are E24's at rest with one change (`f16::at_rest`): N = 1,024 nodes an array, LZ4 HC over the
//! array's dictionary or raw, the checkpoints raw, and each block carrying its own nodes' part of the array's
//! side list. E24 kept each side list as one LZ4 block a file, which a GPU decodes on one thread (up to 6 KB
//! serially) and which a random read must decode whole. A block's matches reach only the block and the
//! dictionary, so any block decodes alone (0156 §2.1). Two kernels:
//!
//! - **thread**: one thread a block, decoding as the reference library does, straight into the arrays;
//! - **simd**: one SIMD group (32 lanes) a block. Every lane reads the same token (one broadcast load), the
//!   lanes copy a literal run or a match 32 bytes at a time into threadgroup memory, and the block is written
//!   out whole: its array part to the array, its side part to the side list. A match closer than 32 bytes
//!   copies in rounds of its distance. A block past the threadgroup's room would go to the thread kernel in
//!   the same command buffer (none did).
//!
//! Both are checked byte for byte against the decoded arrays, and timed warm (the best of five dispatches
//! after one untimed, the pipelines built before). The CPU line is the reference library over the same blocks,
//! one thread and every core.

use crate::f16::{AtRest, Stored};
use crate::measure::cost;
use metal::{Device, MTLResourceOptions, MTLSize};

/// The most bytes a block decodes to in the simd kernel's threadgroup memory: N nodes at two bytes, and its
/// side-list part.
const ROOM: usize = 3072;
/// SIMD groups (blocks) a threadgroup: 4 × 3 KiB of threadgroup memory.
const GROUPS: usize = 4;

const SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Blk { uint src; uint src_len; uint out; uint out_len; uint side_out; uint side_len; uint dict; uint dict_len; uint lz; uint pad; };

static inline uint ext(device const uchar* s, thread uint& ip, uint n) {
    uint c;
    do { c = s[ip++]; n += c; } while (c == 255);
    return n;
}

kernel void lz4_thread(device const Blk* blks [[buffer(0)]], device const uchar* in [[buffer(1)]],
                       device const uchar* dicts [[buffer(2)]], device uchar* out [[buffer(3)]],
                       device const uint* which [[buffer(4)]], constant uint& n [[buffer(5)]],
                       uint id [[thread_position_in_grid]]) {
    if (id >= n) return;
    Blk b = blks[which[id]];
    device const uchar* s = in + b.src;
    // Decoded byte p: the array's part, then the side list's.
    device uchar* o = out + b.out;
    device uchar* side = out + b.side_out - b.out_len;
    #define AT(p) ((p) < b.out_len ? o[p] : side[p])
    if (b.lz == 0) {
        for (uint i = 0; i < b.out_len + b.side_len; i++) AT(i) = s[i];
        return;
    }
    device const uchar* d = dicts + b.dict + b.dict_len;
    uint ip = 0, op = 0;
    while (ip < b.src_len) {
        uint tok = s[ip++];
        uint lit = tok >> 4;
        if (lit == 15) lit = ext(s, ip, lit);
        for (uint i = 0; i < lit; i++) AT(op + i) = s[ip + i];
        ip += lit; op += lit;
        if (ip >= b.src_len) break;
        uint off = uint(s[ip]) | (uint(s[ip + 1]) << 8);
        ip += 2;
        uint ml = tok & 15;
        if (ml == 15) ml = ext(s, ip, ml);
        ml += 4;
        for (uint i = 0; i < ml; i++) {
            int q = int(op + i) - int(off);
            AT(op + i) = q >= 0 ? AT(uint(q)) : d[q];
        }
        op += ml;
    }
    #undef AT
}

kernel void lz4_simd(device const Blk* blks [[buffer(0)]], device const uchar* in [[buffer(1)]],
                     device const uchar* dicts [[buffer(2)]], device uchar* out [[buffer(3)]],
                     device const uint* which [[buffer(4)]], constant uint& n [[buffer(5)]],
                     uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]],
                     uint lane [[thread_index_in_simdgroup]]) {
    threadgroup uchar room[GROUPS * ROOM];
    uint id = tg * GROUPS + sg;
    if (id >= n) return;
    Blk b = blks[which[id]];
    device const uchar* s = in + b.src;
    device uchar* dst = out + b.out;
    device uchar* side = out + b.side_out;
    if (b.lz == 0) {
        for (uint i = lane; i < b.out_len; i += 32) dst[i] = s[i];
        for (uint i = lane; i < b.side_len; i += 32) side[i] = s[b.out_len + i];
        return;
    }
    threadgroup uchar* o = room + sg * ROOM;
    device const uchar* d = dicts + b.dict + b.dict_len;
    uint ip = 0, op = 0;
    while (ip < b.src_len) {
        uint tok = s[ip++];
        uint lit = tok >> 4;
        if (lit == 15) lit = ext(s, ip, lit);
        for (uint i = lane; i < lit; i += 32) o[op + i] = s[ip + i];
        ip += lit; op += lit;
        if (ip >= b.src_len) break;
        uint off = uint(s[ip]) | (uint(s[ip + 1]) << 8);
        ip += 2;
        uint ml = tok & 15;
        if (ml == 15) ml = ext(s, ip, ml);
        ml += 4;
        simdgroup_barrier(mem_flags::mem_threadgroup);
        // Rounds of at most 32 bytes, and of the distance when it is shorter: each round reads only bytes
        // written before it.
        uint step = min(off, 32u);
        for (uint r = 0; r < ml; r += step) {
            uint i = r + lane;
            if (lane < step && i < ml) {
                int q = int(op + i) - int(off);
                o[op + i] = q >= 0 ? o[q] : d[q];
            }
            simdgroup_barrier(mem_flags::mem_threadgroup);
        }
        op += ml;
    }
    simdgroup_barrier(mem_flags::mem_threadgroup);
    for (uint i = lane; i < b.out_len; i += 32) dst[i] = o[i];
    for (uint i = lane; i < b.side_len; i += 32) side[i] = o[b.out_len + i];
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct Blk {
    src: u32,
    src_len: u32,
    out: u32,
    out_len: u32,
    side_out: u32,
    side_len: u32,
    dict: u32,
    dict_len: u32,
    lz: u32,
    pad: u32,
}

#[derive(Default, Clone)]
pub struct Row {
    pub blocks: usize,
    pub lz_blocks: usize,
    pub stored: usize,
    pub decoded: usize,
    /// GPU seconds, warm: the thread kernel over every block; the simd kernel with the big blocks on the
    /// thread kernel; one block alone (the thread kernel's latency).
    pub thread: f64,
    pub simd: f64,
    /// The largest block's decoded bytes; blocks past the threadgroup's room (on the thread kernel).
    pub largest: usize,
    pub over_room: usize,
    pub one: f64,
    /// CPU seconds, the reference library: one thread, every core.
    pub cpu1: f64,
    pub cpun: f64,
    pub cores: usize,
    pub correct: bool,
}

fn bytes_of<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: plain `repr(C)` integers, read as bytes.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

pub fn run(a: &AtRest) -> Result<Row, String> {
    let device = Device::system_default().ok_or("no Metal device")?;
    let src = SOURCE.replace("GROUPS", &GROUPS.to_string()).replace("ROOM", &ROOM.to_string());
    let lib = device.new_library_with_source(&src, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let pipe = |name: &str| -> Result<metal::ComputePipelineState, String> {
        let f = lib.get_function(name, None).map_err(|e| e.to_string())?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
    };
    let (p_thread, p_simd) = (pipe("lz4_thread")?, pipe("lz4_simd")?);
    let mut dicts = Vec::new();
    let mut at = Vec::new();
    for d in &a.dicts {
        at.push(dicts.len() as u32);
        dicts.extend_from_slice(d);
    }
    let blks: Vec<Blk> = a
        .blocks
        .iter()
        .map(|b| {
            let (dict, dict_len) = (at[b.array as usize], a.dicts[b.array as usize].len() as u32);
            Blk { src: b.src, src_len: b.src_len, out: b.out, out_len: b.out_len, side_out: b.side_out, side_len: b.side_len, dict, dict_len, lz: u32::from(b.lz), pad: 0 }
        })
        .collect();
    let all: Vec<u32> = (0..blks.len() as u32).collect();
    let fits = |i: u32| (blks[i as usize].out_len + blks[i as usize].side_len) as usize <= ROOM;
    let small: Vec<u32> = all.iter().copied().filter(|&i| fits(i)).collect();
    let big: Vec<u32> = all.iter().copied().filter(|&i| !fits(i)).collect();
    let one: Vec<u32> = all.iter().copied().filter(|&i| blks[i as usize].lz != 0).take(1).collect();
    let opts = MTLResourceOptions::StorageModeShared;
    let buf = |b: &[u8]| {
        let b = if b.is_empty() { &[0u8][..] } else { b };
        device.new_buffer_with_data(b.as_ptr().cast(), b.len() as u64, opts)
    };
    let (bb, bin, bd) = (buf(bytes_of(&blks)), buf(&a.data), buf(&dicts));
    let (ball, bsmall, bbig, bone) = (buf(bytes_of(&all)), buf(bytes_of(&small)), buf(bytes_of(&big)), buf(bytes_of(&one)));
    let bout = device.new_buffer(a.expected.len().max(1) as u64, opts);
    let queue = device.new_command_queue();
    let clear = || {
        // SAFETY: shared storage, the GPU idle between dispatches.
        unsafe { std::ptr::write_bytes(bout.contents().cast::<u8>(), 0, a.expected.len()) };
    };
    let check = || -> bool {
        // SAFETY: shared storage of `expected.len()` bytes, the GPU done.
        let got = unsafe { std::slice::from_raw_parts(bout.contents().cast::<u8>(), a.expected.len()) };
        got == &a.expected[..]
    };
    // One command buffer: each (pipeline, index list, count) dispatched in turn.
    let dispatch = |parts: &[(&metal::ComputePipelineState, &metal::Buffer, usize, bool)]| -> f64 {
        let cmd = queue.new_command_buffer();
        for &(p, which, n, simd) in parts {
            if n == 0 {
                continue;
            }
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(p);
            for (i, b) in [&bb, &bin, &bd, &bout, which].iter().enumerate() {
                enc.set_buffer(i as u64, Some(b), 0);
            }
            let n32 = n as u32;
            enc.set_bytes(5, 4, (&n32 as *const u32).cast());
            if simd {
                enc.dispatch_thread_groups(MTLSize::new(n.div_ceil(GROUPS) as u64, 1, 1), MTLSize::new(32 * GROUPS as u64, 1, 1));
            } else {
                let w = p.thread_execution_width().min(64);
                enc.dispatch_threads(MTLSize::new(n as u64, 1, 1), MTLSize::new(w, 1, 1));
            }
            enc.end_encoding();
        }
        cmd.commit();
        cmd.wait_until_completed();
        crate::gpu::command_seconds(cmd)
    };
    let best = |parts: &[(&metal::ComputePipelineState, &metal::Buffer, usize, bool)]| -> (f64, bool) {
        clear();
        dispatch(parts);
        let ok = check();
        let t = (0..5).map(|_| dispatch(parts)).fold(f64::INFINITY, f64::min);
        (t, ok)
    };
    let (thread, ok1) = best(&[(&p_thread, &ball, all.len(), false)]);
    let (simd, ok2) = best(&[(&p_simd, &bsmall, small.len(), true), (&p_thread, &bbig, big.len(), false)]);
    let one_s = (0..5).map(|_| dispatch(&[(&p_thread, &bone, one.len(), false)])).fold(f64::INFINITY, f64::min);
    // The CPU: the reference library over the same blocks, into the same layout. Each block decodes to a
    // buffer of its own (the library writes one span), then its two parts are copied to their places.
    struct Out(*mut u8);
    // SAFETY: the blocks' output ranges are disjoint, so threads given different blocks never write the same
    // byte.
    unsafe impl Sync for Out {}
    let decode = |range: &[Stored], out: &Out| {
        let mut tmp = vec![0u8; ROOM.max(range.iter().map(|b| (b.out_len + b.side_len) as usize).max().unwrap_or(0))];
        for b in range {
            let (m, sl) = (b.out_len as usize, b.side_len as usize);
            let s = &a.data[b.src as usize..(b.src + b.src_len) as usize];
            let t = &mut tmp[..m + sl];
            if b.lz {
                crate::disk::lz4_decode(s, t, &a.dicts[b.array as usize]);
            } else {
                t.copy_from_slice(s);
            }
            // SAFETY: both ranges lie in the output (built from it) and belong to this block alone.
            unsafe {
                std::ptr::copy_nonoverlapping(t.as_ptr(), out.0.add(b.out as usize), m);
                std::ptr::copy_nonoverlapping(t.as_ptr().add(m), out.0.add(b.side_out as usize), sl);
            }
        }
    };
    let mut out = vec![0u8; a.expected.len()];
    let o = Out(out.as_mut_ptr());
    decode(&a.blocks, &o);
    let mut cpu1 = f64::INFINITY;
    for _ in 0..3 {
        let (_, k) = cost(|| decode(&a.blocks, &o));
        cpu1 = cpu1.min(k.nanos as f64 * 1e-9);
    }
    let ok3 = out == a.expected;
    out.iter_mut().for_each(|b| *b = 0);
    let o = Out(out.as_mut_ptr());
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per = a.blocks.len().div_ceil(cores).max(1);
    let mut cpun = f64::INFINITY;
    for _ in 0..3 {
        let t0 = std::time::Instant::now();
        std::thread::scope(|sc| {
            for chunk in a.blocks.chunks(per) {
                let (o, decode) = (&o, &decode);
                sc.spawn(move || decode(chunk, o));
            }
        });
        cpun = cpun.min(t0.elapsed().as_secs_f64());
    }
    let ok4 = out == a.expected;
    Ok(Row {
        blocks: a.blocks.len(),
        lz_blocks: a.blocks.iter().filter(|b| b.lz).count(),
        stored: a.data.len(),
        decoded: a.expected.len(),
        thread,
        simd,
        largest: a.blocks.iter().map(|b| (b.out_len + b.side_len) as usize).max().unwrap_or(0),
        over_room: big.len(),
        one: one_s,
        cpu1,
        cpun,
        cores,
        correct: ok1 && ok2 && ok3 && ok4,
    })
}

pub fn print_header() {
    println!("| language | files | nodes | blocks (LZ4) | stored MB | at rest, B a node (E24's form) | decoded MB | largest block (blocks past {ROOM} B) | GPU thread a block | GPU simd group a block | one block alone | CPU 1 thread | CPU every core | right |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
}

pub fn print(lang: &str, a: &AtRest, r: &Row) {
    let nn = a.nodes.max(1) as f64;
    let t = |s: f64| format!("{:.2} ns/node, {:.1} GB/s", s * 1e9 / nn, r.decoded as f64 / s * 1e-9);
    println!(
        "| {lang} | {} | {} | {} ({}) | {:.2} | {:.3} ({:.3}) | {:.2} | {} ({}) | {} | {} | {:.1} µs | {} | {} ({}) | {} |",
        a.files,
        a.nodes,
        r.blocks,
        r.lz_blocks,
        r.stored as f64 / 1e6,
        r.stored as f64 / nn,
        a.e24_bytes as f64 / nn,
        r.decoded as f64 / 1e6,
        r.largest,
        r.over_room,
        t(r.thread),
        t(r.simd),
        r.one * 1e6,
        t(r.cpu1),
        t(r.cpun),
        r.cores,
        if r.correct { "yes" } else { "NO" }
    );
}
