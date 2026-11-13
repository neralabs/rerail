//! E3 on Metal: literal-pool LZ decoded on the GPU, one thread per checkpoint segment (K sequences), against
//! LZ with output references, which can only run one thread per block (a copy may read any earlier output).
//!
//! Every block of a corpus goes in one dispatch, with the language's dictionary shared by all threads (the
//! user's point: dictionaries are shared across GPU invocations). The output is checked against the CPU's.

use crate::lp::{self, Refs};
use metal::foreign_types::ForeignTypeRef;
use metal::{Device, MTLResourceOptions, MTLSize};

const SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Task { uint out; uint lit_base; uint lit_off; uint seq; uint nseq; uint out_base; uint seq_base; uint ck_base; uint k; uint first; };

static inline uint varint(device const uchar* b, thread uint& at) {
    uint v = 0, s = 0;
    for (uint i = 0; i < 5; i++) {
        uchar c = b[at++];
        v |= uint(c & 0x7f) << s;
        if (c < 0x80) break;
        s += 7;
    }
    return v;
}

struct Seq { uint lits; uint op; uint len; uint arg; };

static inline Seq next_seq(device const uchar* seqs, thread uint& at) {
    Seq s;
    uchar tok = seqs[at++];
    s.lits = tok >> 4;
    if (s.lits == 15) s.lits += varint(seqs, at);
    s.op = tok & 15;
    s.len = 0; s.arg = 0;
    if (s.op == 1) { s.len = varint(seqs, at) + 8; s.arg = seqs[at++]; }
    else if (s.op == 2) { s.len = varint(seqs, at) + 4; s.arg = varint(seqs, at); }
    else if (s.op == 3) { s.len = varint(seqs, at) + 1; s.arg = varint(seqs, at); }
    return s;
}

// One sequence's op (after its literals) into out[o..]: a run, or a copy from the dictionary then the block's
// literals (refs_output == 0) or its output (refs_output != 0). Returns the bytes written.
static inline uint op_out(Seq s, device const uchar* lits, device const uchar* dict, device uchar* out, uint o,
                          uint lit_base, uint out_base, uint dict_len, uint refs_output) {
    if (s.op == 1) {
        for (uint j = 0; j < s.len; j++) out[o + j] = uchar(s.arg);
        return s.len;
    }
    if (s.op == 2) {
        for (uint j = 0; j < s.len; j++) {
            uint q = s.arg + j;
            uchar b;
            if (q < dict_len) b = dict[q];
            else if (refs_output != 0) b = out[out_base + q - dict_len];
            else b = lits[lit_base + q - dict_len];
            out[o + j] = b;
        }
        return s.len;
    }
    return 0;
}

kernel void decode(device const Task* tasks [[buffer(0)]],
                   device const uchar* seqs [[buffer(1)]],
                   device const uchar* lits [[buffer(2)]],
                   device const uchar* dict [[buffer(3)]],
                   device uchar* out [[buffer(4)]],
                   constant uint& ntasks [[buffer(5)]],
                   constant uint& dict_len [[buffer(6)]],
                   constant uint& refs_output [[buffer(7)]],
                   device const uint* checks [[buffer(8)]],
                   uint id [[thread_position_in_grid]]) {
    if (id >= ntasks) return;
    Task t = tasks[id];
    uint at = t.seq, o = t.out, li = t.lit_base + t.lit_off;
    for (uint i = t.first; i < t.first + t.nseq; i++) {
        Seq s = next_seq(seqs, at);
        for (uint j = 0; j < s.lits; j++) out[o + j] = lits[li + j];
        o += s.lits; li += s.lits;
        if (s.op == 3) {
            // Replay: locate each replayed sequence from its checkpoint (at most K-1 headers), then emit it.
            for (uint r = i - s.arg; r < i - s.arg + s.len; r++) {
                uint c = t.ck_base + (r / t.k) * 3;
                uint ra = t.seq_base + checks[c + 2], rl = t.lit_base + checks[c + 1];
                for (uint w = 0; w < r % t.k; w++) rl += next_seq(seqs, ra).lits;
                Seq x = next_seq(seqs, ra);
                for (uint j = 0; j < x.lits; j++) out[o + j] = lits[rl + j];
                o += x.lits;
                o += op_out(x, lits, dict, out, o, t.lit_base, t.out_base, dict_len, 0);
            }
        } else {
            o += op_out(s, lits, dict, out, o, t.lit_base, t.out_base, dict_len, refs_output);
        }
    }
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct Task {
    out: u32,
    lit_base: u32,
    lit_off: u32,
    seq: u32,
    nseq: u32,
    out_base: u32,
    seq_base: u32,
    ck_base: u32,
    k: u32,
    first: u32,
}

pub struct GpuRun {
    pub threads: usize,
    pub bytes_out: usize,
    /// Warm: the second of two identical dispatches (no compile, buffers resident).
    pub gpu_seconds: f64,
    /// Span LZ's sequence-index form: the index pass (part of `gpu_seconds`).
    pub index_seconds: f64,
    pub correct: bool,
}

/// Decodes every block on the GPU. `refs`: `Literals` uses one thread per checkpoint segment, `Output` one per
/// block (its copies may read output another segment writes). `Replay` as `Literals`.
pub fn decode(blocks: &[Vec<u8>], dict: &[u8], refs: Refs) -> Result<GpuRun, String> {
    let device = Device::system_default().ok_or("no Metal device")?;
    let lib = device.new_library_with_source(SOURCE, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let f = lib.get_function("decode", None).map_err(|e| e.to_string())?;
    let pipe = device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())?;
    let (mut seqs, mut lits, mut tasks, mut checks) = (Vec::new(), Vec::new(), Vec::new(), Vec::<u32>::new());
    let mut out_len = 0usize;
    let mut expected = Vec::new();
    let pool = lp::Dict::new(dict.to_vec());
    for c in blocks {
        let p = lp::parts(c);
        let (sb, lb, ob) = (seqs.len() as u32, lits.len() as u32, out_len as u32);
        seqs.extend_from_slice(p.seqs);
        lits.extend_from_slice(p.lits);
        let cb = checks.len() as u32;
        checks.extend(p.checks.iter().flatten());
        let k = p.k as u32;
        match refs {
            Refs::Literals | Refs::Replay => {
                for (i, ck) in p.checks.iter().enumerate() {
                    let nseq = (p.nseq - i * p.k).min(p.k) as u32;
                    let first = (i * p.k) as u32;
                    tasks.push(Task { out: ob + ck[0], lit_base: lb, lit_off: ck[1], seq: sb + ck[2], nseq, out_base: ob, seq_base: sb, ck_base: cb, k, first });
                }
            }
            Refs::Output => tasks.push(Task { out: ob, lit_base: lb, lit_off: 0, seq: sb, nseq: p.nseq as u32, out_base: ob, seq_base: sb, ck_base: cb, k, first: 0 }),
        }
        out_len += p.len;
        expected.extend(lp::decompress(c, &pool, refs));
    }
    if tasks.is_empty() {
        return Ok(GpuRun { threads: 0, bytes_out: 0, gpu_seconds: 0.0, index_seconds: 0.0, correct: true });
    }
    let opts = MTLResourceOptions::StorageModeShared;
    // An empty slice's pointer is dangling: a buffer of one zero byte stands in.
    let buf = |b: &[u8]| {
        let b = if b.is_empty() { &[0u8][..] } else { b };
        device.new_buffer_with_data(b.as_ptr().cast(), b.len() as u64, opts)
    };
    // SAFETY: `Task` is plain u32s, `repr(C)`.
    let task_bytes = unsafe { std::slice::from_raw_parts(tasks.as_ptr().cast::<u8>(), tasks.len() * size_of::<Task>()) };
    // SAFETY: u32s as bytes.
    let check_bytes = unsafe { std::slice::from_raw_parts(checks.as_ptr().cast::<u8>(), checks.len() * 4) };
    let (bt, bs, bl, bd, bc) = (buf(task_bytes), buf(&seqs), buf(&lits), buf(dict), buf(check_bytes));
    let bo = device.new_buffer(out_len.max(1) as u64, opts);
    let queue = device.new_command_queue();
    let (n, dl, ro) = (tasks.len() as u32, dict.len() as u32, (refs == Refs::Output) as u32);
    let mut secs = 0.0;
    for _ in 0..2 {
        let cmd = queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        enc.set_compute_pipeline_state(&pipe);
        for (i, b) in [&bt, &bs, &bl, &bd, &bo].iter().enumerate() {
            enc.set_buffer(i as u64, Some(b), 0);
        }
        for (i, v) in [n, dl, ro].iter().enumerate() {
            enc.set_bytes(5 + i as u64, 4, (v as *const u32).cast());
        }
        enc.set_buffer(8, Some(&bc), 0);
        let w = pipe.thread_execution_width().min(64);
        enc.dispatch_threads(MTLSize::new(u64::from(n), 1, 1), MTLSize::new(w, 1, 1));
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
        secs = gpu_seconds(cmd);
    }
    // SAFETY: the buffer is shared storage of `out_len` bytes and the GPU is done with it.
    let got = unsafe { std::slice::from_raw_parts(bo.contents().cast::<u8>(), out_len) };
    Ok(GpuRun { threads: tasks.len(), bytes_out: out_len, gpu_seconds: secs, index_seconds: 0.0, correct: got == &expected[..] })
}

/// A completed command buffer's GPU time, in seconds.
pub fn command_seconds(command: &metal::CommandBufferRef) -> f64 {
    gpu_seconds(command)
}

fn gpu_seconds(command: &metal::CommandBufferRef) -> f64 {
    unsafe extern "C" {
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn objc_msgSend();
    }
    // SAFETY: `GPUStartTime` and `GPUEndTime` take no arguments and return a CFTimeInterval (a double).
    let send: unsafe extern "C" fn(*const std::ffi::c_void, *mut std::ffi::c_void) -> f64 =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let object = command.as_ptr().cast::<std::ffi::c_void>().cast_const();
    let (start, end) = unsafe { (send(object, sel_registerName(c"GPUStartTime".as_ptr())), send(object, sel_registerName(c"GPUEndTime".as_ptr()))) };
    (end - start).max(0.0)
}

pub(crate) const SPAN_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

// One thread per checkpoint segment: its block (global index), output offset, body and literal offsets
// within the block, how many sequences it decodes, and the index of its first.
struct Task { uint out; uint block; uint at; uint li; uint nseq; uint si; };
// Each block's first byte in the global sequence and literal buffers, its first checkpoint, K, and its
// first entry in the sequence table (the index pass's output), and its dictionary (a range of the dictionary
// buffer, so one dispatch can hold several languages).
struct Block { uint seq_base; uint lit_base; uint ck_base; uint k; uint tab_base; uint dict_base; uint dict_len; };
// A checkpoint's literal and body offsets, for the sequence-index form's walk.
struct Check { uint li; uint at; };
struct Frame { uint block; uint at; uint li; uint left; uint si; };
struct Tok { uint lits; uint op; uint len; uint arg; uint blk; uint at; uint li; uint seq; bool by_seq; };

static inline uint varint(device const uchar* b, thread uint& at) {
    uint v = 0, s = 0;
    for (uint i = 0; i < 5; i++) {
        uchar c = b[at++];
        v |= uint(c & 0x7f) << s;
        if (c < 0x80) break;
        s += 7;
    }
    return v;
}

// A sequence header (`span::tok`). fmt: 0 full, 1 split, 2 split without lengths, 3 sequence index (found
// by walking from its checkpoint), 4 sequence index (found in the index pass's table).
static inline Tok tok(device const uchar* s, thread uint& at, uint b, uint li, uint si, uint fmt) {
    uint here = at;
    uchar t = s[at++];
    Tok k;
    k.lits = t >> 4;
    if (k.lits == 15) k.lits += varint(s, at);
    k.op = t & 15;
    k.len = 0; k.arg = 0; k.blk = b; k.at = 0; k.li = 0; k.seq = 0; k.by_seq = false;
    if (k.op == 1) {
        k.len = varint(s, at) + 8;
        k.arg = s[at++];
    } else if (k.op == 2) {
        k.len = varint(s, at) + 4;
        k.arg = varint(s, at);
    } else if (k.op == 4 || k.op == 5) {
        if (fmt >= 3) {
            k.by_seq = true;
            if (k.op == 4) {
                k.seq = si - (varint(s, at) + 1);
            } else {
                k.blk = b - (varint(s, at) + 1);
                k.seq = varint(s, at);
            }
            k.len = varint(s, at) + 1;
        } else {
            uint back = fmt == 0 ? varint(s, at) : (k.op == 5 ? varint(s, at) + 1 : 0);
            k.len = varint(s, at) + 1;
            uint body = varint(s, at), lit = varint(s, at);
            if (fmt < 2) varint(s, at); // the span's output length: a reader that skips uses it
            k.blk = b - back;
            k.at = back == 0 ? here - body : body;
            k.li = back == 0 ? li - lit : lit;
        }
    }
    return k;
}

kernel void decode_span(device const Task* tasks [[buffer(0)]],
                        device const uchar* seqs [[buffer(1)]],
                        device const uchar* lits [[buffer(2)]],
                        device const uchar* dict [[buffer(3)]],
                        device uchar* out [[buffer(4)]],
                        constant uint& ntasks [[buffer(5)]],
                        constant uint& dict_len [[buffer(6)]],
                        device const Block* blocks [[buffer(7)]],
                        device const Check* checks [[buffer(8)]],
                        constant uint& fmt [[buffer(9)]],
                        device const uint2* tab [[buffer(10)]],
                        uint id [[thread_position_in_grid]]) {
    if (id >= ntasks) return;
    Task t = tasks[id];
    uint o = t.out;
    // The depth bound (4 in the lab) plus the segment itself.
    Frame st[8];
    int top = 0;
    st[0] = Frame{t.block, t.at, t.li, t.nseq, t.si};
    while (top >= 0) {
        if (st[top].left == 0) { top--; continue; }
        st[top].left--;
        uint b = st[top].block;
        device const uchar* s = seqs + blocks[b].seq_base;
        device const uchar* l = lits + blocks[b].lit_base;
        device const uchar* dict_b = dict + blocks[b].dict_base;
        uint dl = blocks[b].dict_len;
        uint at = st[top].at, li = st[top].li, si = st[top].si;
        Tok k = tok(s, at, b, li, si, fmt);
        for (uint j = 0; j < k.lits; j++) out[o + j] = l[li + j];
        o += k.lits;
        if (k.op == 1) {
            for (uint j = 0; j < k.len; j++) out[o + j] = uchar(k.arg);
            o += k.len;
        } else if (k.op == 2) {
            for (uint j = 0; j < k.len; j++) {
                uint q = k.arg + j;
                out[o + j] = q < dl ? dict_b[q] : l[q - dl];
            }
            o += k.len;
        }
        st[top].at = at;
        st[top].li = li + k.lits;
        st[top].si = si + 1;
        if ((k.op == 4 || k.op == 5) && top < 7) {
            uint sat = k.at, sli = k.li, ssi = 0;
            if (k.by_seq && fmt == 4) {
                uint2 e = tab[blocks[k.blk].tab_base + k.seq];
                sat = e.x; sli = e.y; ssi = k.seq;
            } else if (k.by_seq) {
                // From the target's checkpoint, at most K - 1 headers on.
                Block tb = blocks[k.blk];
                uint ck = k.seq / tb.k;
                Check c = checks[tb.ck_base + ck];
                sat = c.at; sli = c.li;
                device const uchar* ts = seqs + tb.seq_base;
                for (uint i = ck * tb.k; i < k.seq; i++) {
                    Tok h = tok(ts, sat, k.blk, sli, i, fmt);
                    sli += h.lits;
                }
                ssi = k.seq;
            }
            top++;
            st[top] = Frame{k.blk, sat, sli, k.len, ssi};
        }
    }
}

// The index pass: one thread per checkpoint segment walks its headers once and records each sequence's
// body and literal offsets, so the span pass finds a sequence-index target without a walk.
kernel void index_seqs(device const Task* tasks [[buffer(0)]],
                       device const uchar* seqs [[buffer(1)]],
                       device const Block* blocks [[buffer(7)]],
                       constant uint& ntasks [[buffer(5)]],
                       device uint2* tab [[buffer(10)]],
                       uint id [[thread_position_in_grid]]) {
    if (id >= ntasks) return;
    Task t = tasks[id];
    Block b = blocks[t.block];
    device const uchar* s = seqs + b.seq_base;
    uint at = t.at, li = t.li;
    for (uint i = 0; i < t.nseq; i++) {
        tab[b.tab_base + t.si + i] = uint2(at, li);
        Tok h = tok(s, at, t.block, li, t.si + i, 3);
        li += h.lits;
    }
}
"#;

/// The span pass's inputs: tasks, the block table and checkpoints, and the expected output.
pub(crate) struct SpanInputs {
    pub(crate) tasks: Vec<[u32; 6]>,
    pub(crate) blocks: Vec<[u32; 7]>,
    pub(crate) checks: Vec<[u32; 2]>,
    /// Sequences so far: the size of the index pass's table.
    pub(crate) seqs: u32,
}

impl SpanInputs {
    pub(crate) fn new() -> Self {
        SpanInputs { tasks: Vec::new(), blocks: Vec::new(), checks: Vec::new(), seqs: 0 }
    }
    /// A block's tasks and table entries; `checks` are (output, literal, body) offsets.
    /// `dict`: the block's dictionary as (first byte, length) in the dictionary buffer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add(&mut self, out_base: u32, seq_base: u32, lit_base: u32, checks: &[[u32; 3]], nseq: usize, k: usize, dict: (u32, u32)) {
        let g = self.blocks.len() as u32;
        self.blocks.push([seq_base, lit_base, self.checks.len() as u32, k as u32, self.seqs, dict.0, dict.1]);
        self.seqs += nseq as u32;
        for (i, ck) in checks.iter().enumerate() {
            self.checks.push([ck[1], ck[2]]);
            let n = (nseq - i * k).min(k) as u32;
            self.tasks.push([out_base + ck[0], g, ck[2], ck[1], n, (i * k) as u32]);
        }
    }
}

/// Encodes the span pass: its buffers 0–9.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_span(enc: &metal::ComputeCommandEncoderRef, pipe: &metal::ComputePipelineState, bufs: [&metal::Buffer; 5], blocks: &metal::Buffer, checks: &metal::Buffer, tab: &metal::Buffer, n: u32, dict_len: u32, fmt: u32) {
    enc.set_compute_pipeline_state(pipe);
    for (i, b) in bufs.iter().enumerate() {
        enc.set_buffer(i as u64, Some(b), 0);
    }
    enc.set_bytes(5, 4, (&n as *const u32).cast());
    enc.set_bytes(6, 4, (&dict_len as *const u32).cast());
    enc.set_buffer(7, Some(blocks), 0);
    enc.set_buffer(8, Some(checks), 0);
    enc.set_bytes(9, 4, (&fmt as *const u32).cast());
    enc.set_buffer(10, Some(tab), 0);
    let w = pipe.thread_execution_width().min(64);
    enc.dispatch_threads(MTLSize::new(u64::from(n), 1, 1), MTLSize::new(w, 1, 1));
}

/// The index pass alone, in its own command buffer: its GPU seconds.
pub(crate) fn run_index(queue: &metal::CommandQueue, pipe: &metal::ComputePipelineState, tasks: &metal::Buffer, seqs: &metal::Buffer, blocks: &metal::Buffer, tab: &metal::Buffer, n: u32) -> f64 {
    let cmd = queue.new_command_buffer();
    let enc = cmd.new_compute_command_encoder();
    enc.set_compute_pipeline_state(pipe);
    enc.set_buffer(0, Some(tasks), 0);
    enc.set_buffer(1, Some(seqs), 0);
    enc.set_bytes(5, 4, (&n as *const u32).cast());
    enc.set_buffer(7, Some(blocks), 0);
    enc.set_buffer(10, Some(tab), 0);
    let w = pipe.thread_execution_width().min(64);
    enc.dispatch_threads(MTLSize::new(u64::from(n), 1, 1), MTLSize::new(w, 1, 1));
    enc.end_encoding();
    cmd.commit();
    cmd.wait_until_completed();
    gpu_seconds(cmd)
}

/// The span kernel's mode: the pointer form, and for the sequence-index form whether the index pass runs.
pub(crate) fn span_mode(ptr: crate::span::Ptr, index: bool) -> u32 {
    if ptr == crate::span::Ptr::Seq && index { 4 } else { ptr.code() }
}

/// Span LZ on the GPU: `files` are each file's compressed blocks (`codec::compress_file`), all decoded in one
/// dispatch, one thread per checkpoint segment of every block.
pub fn decode_span(files: &[Vec<Vec<u8>>], dict: &[u8], ptr: crate::span::Ptr, index: bool) -> Result<GpuRun, String> {
    let device = Device::system_default().ok_or("no Metal device")?;
    let lib = device.new_library_with_source(SPAN_SOURCE, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let f = lib.get_function("decode_span", None).map_err(|e| e.to_string())?;
    let pipe = device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())?;
    let fi = lib.get_function("index_seqs", None).map_err(|e| e.to_string())?;
    let ipipe = device.new_compute_pipeline_state_with_function(&fi).map_err(|e| e.to_string())?;
    let pool = lp::Dict::new(dict.to_vec());
    let (mut seqs, mut lits, mut inputs, mut expected) = (Vec::new(), Vec::new(), SpanInputs::new(), Vec::new());
    for file in files {
        for (b, c) in file.iter().enumerate() {
            let p = lp::parts(c);
            inputs.add(expected.len() as u32, seqs.len() as u32, lits.len() as u32, &p.checks, p.nseq, p.k, (0, dict.len() as u32));
            seqs.extend_from_slice(p.seqs);
            lits.extend_from_slice(p.lits);
            expected.extend(crate::span::decompress(file, b, &pool, ptr));
        }
    }
    if inputs.tasks.is_empty() {
        return Ok(GpuRun { threads: 0, bytes_out: 0, gpu_seconds: 0.0, index_seconds: 0.0, correct: true });
    }
    let opts = MTLResourceOptions::StorageModeShared;
    let buf = |b: &[u8]| {
        let b = if b.is_empty() { &[0u8][..] } else { b };
        device.new_buffer_with_data(b.as_ptr().cast(), b.len() as u64, opts)
    };
    // SAFETY: arrays of u32s as bytes.
    let as_bytes = |v: &[u32]| unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 4) };
    let (bt, bs, bl, bd) = (buf(as_bytes(inputs.tasks.as_flattened())), buf(&seqs), buf(&lits), buf(dict));
    let (bb, bc) = (buf(as_bytes(inputs.blocks.as_flattened())), buf(as_bytes(inputs.checks.as_flattened())));
    let bo = device.new_buffer(expected.len().max(1) as u64, opts);
    let btab = device.new_buffer(u64::from(inputs.seqs.max(1)) * 8, opts);
    let queue = device.new_command_queue();
    let n = inputs.tasks.len() as u32;
    let mode = span_mode(ptr, index);
    let (mut secs, mut isecs) = (0.0, 0.0);
    for _ in 0..2 {
        isecs = if mode == 4 { run_index(&queue, &ipipe, &bt, &bs, &bb, &btab, n) } else { 0.0 };
        let cmd = queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        encode_span(enc, &pipe, [&bt, &bs, &bl, &bd, &bo], &bb, &bc, &btab, n, dict.len() as u32, mode);
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
        secs = gpu_seconds(cmd) + isecs;
    }
    // SAFETY: shared storage of `expected.len()` bytes; the GPU is done with it.
    let got = unsafe { std::slice::from_raw_parts(bo.contents().cast::<u8>(), expected.len()) };
    Ok(GpuRun { threads: inputs.tasks.len(), bytes_out: expected.len(), gpu_seconds: secs, index_seconds: isecs, correct: got == &expected[..] })
}

pub(crate) const HUFF_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

// One thread per checkpoint segment: decode its sequence bytes and its literal bytes from their bit offsets.
// `table`: which language's tables (a multiple of 4096 entries into each LUT buffer).
struct HTask { uint seq_out; uint seq_n; uint seq_bit; uint lit_out; uint lit_n; uint lit_bit; uint table; };

static inline void run(device const uchar* bits, uint bit, uint n, device const ushort* lut, device uchar* out) {
    for (uint i = 0; i < n; i++) {
        uint b = bit >> 3;
        uint w = uint(bits[b]) | uint(bits[b + 1]) << 8 | uint(bits[b + 2]) << 16;
        ushort e = lut[(w >> (bit & 7)) & 4095];
        out[i] = uchar(e & 255);
        bit += e >> 8;
    }
}

kernel void huff(device const HTask* tasks [[buffer(0)]],
                 device const uchar* seq_bits [[buffer(1)]],
                 device const uchar* lit_bits [[buffer(2)]],
                 device const ushort* seq_lut [[buffer(3)]],
                 device const ushort* lit_lut [[buffer(4)]],
                 device uchar* seqs [[buffer(5)]],
                 device uchar* lits [[buffer(6)]],
                 constant uint& ntasks [[buffer(7)]],
                 uint id [[thread_position_in_grid]]) {
    if (id >= ntasks) return;
    HTask t = tasks[id];
    run(seq_bits, t.seq_bit, t.seq_n, seq_lut + t.table * 4096, seqs + t.seq_out);
    run(lit_bits, t.lit_bit, t.lit_n, lit_lut + t.table * 4096, lits + t.lit_out);
}
"#;

pub struct HuffRun {
    pub threads: usize,
    pub bytes_out: usize,
    /// The Huffman pass and the span pass (with the index pass, when it runs), warm.
    pub huff_seconds: f64,
    pub lz_seconds: f64,
    pub index_seconds: f64,
    pub correct: bool,
}

/// Span LZ with the entropy stage on the GPU: a Huffman pass (one thread per checkpoint segment) into the
/// plain sequences and literals, then the span pass over them. `files` are each file's entropy-coded blocks.
pub fn decode_span_huff(files: &[Vec<Vec<u8>>], dict: &[u8], seq_t: &crate::huff::Table, lit_t: &crate::huff::Table, ptr: crate::span::Ptr, index: bool) -> Result<HuffRun, String> {
    use crate::huff;
    let device = Device::system_default().ok_or("no Metal device")?;
    let pipe = |src: &str, name: &str| -> Result<metal::ComputePipelineState, String> {
        let lib = device.new_library_with_source(src, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
        let f = lib.get_function(name, None).map_err(|e| e.to_string())?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
    };
    let (hp, sp, ip) = (pipe(HUFF_SOURCE, "huff")?, pipe(SPAN_SOURCE, "decode_span")?, pipe(SPAN_SOURCE, "index_seqs")?);
    let pool = lp::Dict::new(dict.to_vec());
    let (mut sbits, mut lbits) = (Vec::new(), Vec::new());
    let (mut htasks, mut inputs) = (Vec::<[u32; 7]>::new(), SpanInputs::new());
    let (mut nseq, mut nlit) = (0usize, 0usize);
    let mut expected = Vec::new();
    for file in files {
        let plain: Vec<Vec<u8>> = file.iter().map(|c| huff::decode_block(c, seq_t, lit_t)).collect();
        for (b, c) in file.iter().enumerate() {
            let p = huff::coded(c);
            let plain_checks: Vec<[u32; 3]> = p.checks.iter().map(|c| [c[0], c[1], c[2]]).collect();
            inputs.add(expected.len() as u32, nseq as u32, nlit as u32, &plain_checks, p.nseq, p.k, (0, dict.len() as u32));
            let (sb, lb) = (sbits.len() as u64 * 8, lbits.len() as u64 * 8);
            sbits.extend_from_slice(p.seq_bits);
            lbits.extend_from_slice(p.lit_bits);
            for (i, ck) in p.checks.iter().enumerate() {
                let next = p.checks.get(i + 1);
                let s_end = next.map_or(p.nseq_bytes, |n| n[2] as usize) as u32;
                let l_end = next.map_or(p.nlit, |n| n[1] as usize) as u32;
                htasks.push([nseq as u32 + ck[2], s_end - ck[2], (sb + u64::from(ck[3])) as u32, nlit as u32 + ck[1], l_end - ck[1], (lb + u64::from(ck[4])) as u32, 0]);
            }
            nseq += p.nseq_bytes;
            nlit += p.nlit;
            expected.extend(crate::span::decompress(&plain, b, &pool, ptr));
        }
    }
    if inputs.tasks.is_empty() {
        return Ok(HuffRun { threads: 0, bytes_out: 0, huff_seconds: 0.0, lz_seconds: 0.0, index_seconds: 0.0, correct: true });
    }
    sbits.extend_from_slice(&[0; 4]);
    lbits.extend_from_slice(&[0; 4]);
    let lut = |t: &huff::Table| -> Vec<u8> { t.lut.iter().flat_map(|&(s, l)| (u16::from(s) | u16::from(l) << 8).to_le_bytes()).collect() };
    let opts = MTLResourceOptions::StorageModeShared;
    let buf = |b: &[u8]| {
        let b = if b.is_empty() { &[0u8][..] } else { b };
        device.new_buffer_with_data(b.as_ptr().cast(), b.len() as u64, opts)
    };
    // SAFETY: arrays of u32s as bytes.
    let as_bytes = |v: &[u32]| unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 4) };
    let (bh, bsb, blb, bsl, bll) = (buf(as_bytes(htasks.as_flattened())), buf(&sbits), buf(&lbits), buf(&lut(seq_t)), buf(&lut(lit_t)));
    let (bseq, blit) = (device.new_buffer(nseq.max(1) as u64 + 8, opts), device.new_buffer(nlit.max(1) as u64 + 8, opts));
    let (bt, bd) = (buf(as_bytes(inputs.tasks.as_flattened())), buf(dict));
    let (bb, bc) = (buf(as_bytes(inputs.blocks.as_flattened())), buf(as_bytes(inputs.checks.as_flattened())));
    let bo = device.new_buffer(expected.len().max(1) as u64, opts);
    let btab = device.new_buffer(u64::from(inputs.seqs.max(1)) * 8, opts);
    let queue = device.new_command_queue();
    let n = inputs.tasks.len() as u32;
    let mode = span_mode(ptr, index);
    let (mut hs, mut ls, mut is) = (0.0, 0.0, 0.0);
    for _ in 0..2 {
        // Two command buffers, so each pass has its own GPU time.
        let cmd = queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        enc.set_compute_pipeline_state(&hp);
        for (i, b) in [&bh, &bsb, &blb, &bsl, &bll, &bseq, &blit].iter().enumerate() {
            enc.set_buffer(i as u64, Some(b), 0);
        }
        enc.set_bytes(7, 4, (&n as *const u32).cast());
        let w = hp.thread_execution_width().min(64);
        enc.dispatch_threads(MTLSize::new(u64::from(n), 1, 1), MTLSize::new(w, 1, 1));
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
        hs = gpu_seconds(cmd);
        is = if mode == 4 { run_index(&queue, &ip, &bt, &bseq, &bb, &btab, n) } else { 0.0 };
        let cmd = queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        encode_span(enc, &sp, [&bt, &bseq, &blit, &bd, &bo], &bb, &bc, &btab, n, dict.len() as u32, mode);
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
        ls = gpu_seconds(cmd) + is;
    }
    // SAFETY: shared storage of `expected.len()` bytes; the GPU is done with it.
    let got = unsafe { std::slice::from_raw_parts(bo.contents().cast::<u8>(), expected.len()) };
    Ok(HuffRun { threads: inputs.tasks.len(), bytes_out: expected.len(), huff_seconds: hs, lz_seconds: ls, index_seconds: is, correct: got == &expected[..] })
}
