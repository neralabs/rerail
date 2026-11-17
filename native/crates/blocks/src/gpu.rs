//! Metal: kernels that read and write block structures in place. Blocks are wrapped as no-copy shared buffers
//! (shared memory: nothing is uploaded). The structs come from `structure!` (`msl_of`), with offset assertions,
//! so this file holds no copy of any layout. The accessors below (`rows_get`, `v_live`, `rel_group`, …) are the
//! layer's MSL half, written once, and named as the host's are.
//!
//! The join: `out(l_out, r_out) :- left(.., l_key, ..), right(.., r_key, ..), l_out != r_out`, with `left` and
//! `right` each a base + delta view and `right` probed by its index. It runs in two passes, as 0266 §5 says:
//!
//! 1. A count pass gives each left row's matches.
//! 2. The host reserves the total (chunks made between passes, never during one).
//! 3. A write pass claims rows with one atomic per thread, and writes them.
//!
//! A claim past what was reserved writes nothing and adds to `needed`; the launcher reruns the write pass at the
//! exact size.

use crate::block::Block;
use crate::relation::Relation;
use crate::{MslField, msl_of, structure};
use metal::{Buffer, CommandQueue, CompileOptions, ComputePipelineState, Device, MTLResourceOptions, MTLSize};
use std::ffi::c_void;
use std::sync::atomic::Ordering::*;

pub const NONE: u64 = u64::MAX;

structure! {
    /// A join's arguments: each relation's offset in its block (`NONE`: no base), and the columns.
    pub struct JoinArgs {
        pub lbase: u64,
        pub ltop: u64,
        pub rbase: u64,
        pub rtop: u64,
        pub out: u64,
        pub n: u32,
        pub idx: u32,
        pub lkey: u32,
        pub lout: u32,
        pub rkey: u32,
        pub rout: u32,
    }
}

const LIB: &str = r#"
constant ulong NONE = 0xFFFFFFFFFFFFFFFFul;
constant ulong SEED = 0x243F6A8885A308D3ul;
inline ulong mix(ulong h, long v) { h ^= ulong(v); h *= 0x9E3779B97F4A7C15ul; return h ^ (h >> 29); }

// A self-relative reference: the field's own address plus its offset.
inline device char* follow(device const void* f, long off) { return (device char*)((device const char*)f + off); }

// Rows: chunk k holds 2^(min_log + k) rows; row i's chunk is one clz.
inline uint2 chunk_of(uint i, uint min_log) {
    ulong q = (ulong(i) >> min_log) + 1ul;
    uint k = 63u - uint(clz(q));
    ulong first = ((1ul << k) - 1ul) << min_log;
    return uint2(k, uint(ulong(i) - first));
}
inline device ColDir* rows_col(device const Rows* r, uint c) { return (device ColDir*)follow(&r->cols, r->cols.off) + c; }
inline device char* rows_at(device const Rows* r, uint i, uint c, thread uint& w) {
    uint2 kl = chunk_of(i, r->min_log);
    device ColDir* d = rows_col(r, c);
    w = d->width;
    return follow(d, d->chunk[kl.x]) + ulong(kl.y) * ulong(w);
}
inline long rows_get(device const Rows* r, uint i, uint c) {
    uint w; device char* p = rows_at(r, i, c, w);
    return w == 4u ? long(*(device uint*)p) : *(device long*)p;
}
inline void rows_put(device const Rows* r, uint i, uint c, long v) {
    uint w; device char* p = rows_at(r, i, c, w);
    if (w == 4u) { *(device uint*)p = uint(v); } else { *(device long*)p = v; }
}

inline bool dead_bit(device const Relation* r, uint id) {
    ulong w = ulong(id >> 6);
    if (w >= r->dead.len) return false;
    device const ulong* d = (device const ulong*)follow(&r->dead, r->dead.off);
    return ((d[w] >> ulong(id & 63u)) & 1ul) != 0ul;
}

// A relation as readers see it: a base and its delta, or a full relation (base null).
struct View { device const Relation* base; device const Relation* top; };
inline long v_get(View v, uint id, uint c) {
    return id < v.top->base_rows ? rows_get(&v.base->rows, id, c) : rows_get(&v.top->rows, id - v.top->base_rows, c);
}
inline bool v_live(View v, uint id) {
    if (dead_bit(v.top, id)) return false;
    return id >= v.top->base_rows || !dead_bit(v.base, id);
}
inline device const Index* rel_index(device const Relation* r, uint idx) {
    return (device const Index*)follow(&r->indexes, r->indexes.off) + idx;
}
// A key's group in one relation's index: (start, len) into its rows; bounded by the table's size.
inline uint2 rel_group(device const Relation* r, uint idx, ulong h) {
    device const Index* ix = rel_index(r, idx);
    if (ix->buckets.len == 0ul) return uint2(0u);
    device const Bucket* b = (device const Bucket*)follow(&ix->buckets, ix->buckets.off);
    ulong s = h & ix->mask;
    for (ulong n = 0ul; n <= ix->mask; n++) {
        device const Bucket* x = b + ((s + n) & ix->mask);
        if (x->len == 0u) return uint2(0u);
        if (x->hash == h) return uint2(x->start, x->len);
    }
    return uint2(0u);
}
inline device const uint* rel_group_rows(device const Relation* r, uint idx) {
    device const Index* ix = rel_index(r, idx);
    return (device const uint*)follow(&ix->rows, ix->rows.off);
}
inline View view_of(device char* b1, device char* b2, ulong base, ulong top) {
    View v;
    v.base = base == NONE ? (device const Relation*)nullptr : (device const Relation*)(b1 + base);
    v.top = (device const Relation*)(b2 + top);
    return v;
}

inline uint join_row(View L, View R, constant JoinArgs& a, uint id, device const Relation* out, uint at, bool write) {
    if (!v_live(L, id)) return 0u;
    long k = v_get(L, id, a.lkey);
    long lo = v_get(L, id, a.lout);
    ulong h = mix(SEED, k);
    uint n = 0u;
    for (uint side = 0u; side < 2u; side++) {
        device const Relation* r = side == 0u ? R.base : R.top;
        if (r == nullptr) continue;
        uint2 g = rel_group(r, a.idx, h);
        device const uint* rows = rel_group_rows(r, a.idx);
        for (uint j = 0u; j < g.y; j++) {
            uint rid = rows[g.x + j];
            if (!v_live(R, rid) || v_get(R, rid, a.rkey) != k) continue;
            long ro = v_get(R, rid, a.rout);
            if (ro == lo) continue;
            if (write) { rows_put(&out->rows, at + n, 0u, lo); rows_put(&out->rows, at + n, 1u, ro); }
            n++;
        }
    }
    return n;
}

kernel void join_count(constant JoinArgs& a [[buffer(0)]], device char* b1 [[buffer(1)]], device char* b2 [[buffer(2)]],
                       device uint* counts [[buffer(3)]], uint id [[thread_position_in_grid]]) {
    if (id >= a.n) return;
    View L = view_of(b1, b2, a.lbase, a.ltop);
    View R = view_of(b1, b2, a.rbase, a.rtop);
    counts[id] = join_row(L, R, a, id, (device const Relation*)nullptr, 0u, false);
}

kernel void join_write(constant JoinArgs& a [[buffer(0)]], device char* b1 [[buffer(1)]], device char* b2 [[buffer(2)]],
                       device uint* counts [[buffer(3)]], device char* b3 [[buffer(4)]], uint id [[thread_position_in_grid]]) {
    if (id >= a.n) return;
    uint c = counts[id];
    if (c == 0u) return;
    device Relation* out = (device Relation*)(b3 + a.out);
    uint at = atomic_fetch_add_explicit((device atomic_uint*)&out->rows.len, c, memory_order_relaxed);
    if (at + c > out->rows.reserved) {
        atomic_fetch_add_explicit((device atomic_uint*)&out->rows.needed, c, memory_order_relaxed);
        return;
    }
    View L = view_of(b1, b2, a.lbase, a.ltop);
    View R = view_of(b1, b2, a.rbase, a.rtop);
    join_row(L, R, a, id, out, at, true);
}
"#;

/// The kernels' whole source: the generated structs (with their assertions), then the accessors and kernels.
pub fn source() -> String {
    let mut s = String::from("#include <metal_stdlib>\nusing namespace metal;\n");
    s += &msl_of::<Relation>();
    let mut more = Vec::new();
    JoinArgs::deps(&mut more);
    s += &more.concat();
    s += LIB;
    s
}

pub struct Gpu {
    pub device: Device,
    queue: CommandQueue,
    count: ComputePipelineState,
    write: ComputePipelineState,
}

/// What one join did on the device.
#[derive(Debug, Default, Clone, Copy)]
pub struct JoinRun {
    pub rows: u64,
    pub reruns: u32,
    pub wall_us: u64,
}

impl Gpu {
    pub fn new() -> Result<Gpu, String> {
        // The process's one device and queue (decision 0100 §2): no queue of its own.
        let shared = rerail_forest::gpu::shared::get().ok_or("no Metal device")?;
        let device = shared.device.clone();
        let lib = device.new_library_with_source(&source(), &CompileOptions::new())?;
        let pipe = |name: &str| -> Result<ComputePipelineState, String> {
            let f = lib.get_function(name, None)?;
            device.new_compute_pipeline_state_with_function(&f)
        };
        let (count, write) = (pipe("join_count")?, pipe("join_write")?);
        Ok(Gpu { device, queue: shared.queue.clone(), count, write })
    }

    /// The block as a no-copy shared buffer: its pages are the device's too. It covers what the block has used
    /// (rounded to 1 MiB), not its whole reservation: a command buffer's cost grows with the length of the buffers
    /// it makes resident (measured: 2 GiB wrapped cost 120 ms a join, 256 MiB 17 ms).
    pub fn wrap(&self, b: &Block) -> Buffer {
        self.wrap_len(b, b.used())
    }

    pub fn wrap_len(&self, b: &Block, len: usize) -> Buffer {
        let len = len.div_ceil(1 << 20).max(1) << 20;
        self.device.new_buffer_with_bytes_no_copy(
            b.base_ptr() as *const c_void,
            len.min(b.reserved()) as u64,
            MTLResourceOptions::StorageModeShared,
            None,
        )
    }

    fn dispatch(&self, p: &ComputePipelineState, args: &JoinArgs, b: [&Buffer; 3], counts: &Buffer) {
        let cmd = self.queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        enc.set_compute_pipeline_state(p);
        enc.set_bytes(0, size_of::<JoinArgs>() as u64, args as *const JoinArgs as *const c_void);
        enc.set_buffer(1, Some(b[0]), 0);
        enc.set_buffer(2, Some(b[1]), 0);
        enc.set_buffer(3, Some(counts), 0);
        enc.set_buffer(4, Some(b[2]), 0);
        enc.dispatch_threads(MTLSize::new(args.n.max(1) as u64, 1, 1), MTLSize::new(256, 1, 1));
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();
    }

    /// Runs the join: `left` and `right` read from `base` (if they have one) and `top`, and the rows are written
    /// into `out`, an open relation of `out_block`. `short` reserves nothing for them, to take the overflow path:
    /// the write pass is then run again at the exact size.
    pub fn join(&self, base: Option<&Block>, top: &Block, out_block: &Block, out: &Relation, args: &JoinArgs, short: bool) -> JoinRun {
        let t = std::time::Instant::now();
        let bt = self.wrap(top);
        let bb = base.map(|b| self.wrap(b));
        let b1 = bb.as_ref().unwrap_or(&bt);
        let counts = self.device.new_buffer((args.n.max(1) as u64) * 4, MTLResourceOptions::StorageModeShared);
        let none = self.device.new_buffer(4, MTLResourceOptions::StorageModeShared);
        self.dispatch(&self.count, args, [b1, &bt, &none], &counts);
        let top = out_block;
        let c = unsafe { std::slice::from_raw_parts(counts.contents() as *const u32, args.n as usize) };
        let total: u64 = c.iter().map(|&x| x as u64).sum();
        let len0 = out.rows.len();
        out.rows.reserve(top, len0 as u64 + if short { 0 } else { total });
        let mut run = JoinRun { rows: total, ..Default::default() };
        loop {
            // The out block is wrapped after its chunks are reserved: its used extent covers them.
            let bo = self.wrap(top);
            let bt = if std::ptr::eq(out_block.base_ptr(), bt.contents() as *mut u8) { self.wrap(out_block) } else { bt.clone() };
            self.dispatch(&self.write, args, [b1, &bt, &bo], &counts);
            let needed = out.rows.needed.load(Acquire);
            if needed == 0 {
                break;
            }
            // Overflow: nothing past `reserved` was written. Reset the claims, reserve the exact size, rerun.
            out.rows.needed.store(0, Release);
            out.rows.len.store(len0, Release);
            out.rows.reserve(top, len0 as u64 + total);
            run.reruns += 1;
        }
        run.wall_us = t.elapsed().as_micros() as u64;
        run
    }
}
