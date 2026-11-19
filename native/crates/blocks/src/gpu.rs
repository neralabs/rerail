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

// A relation's reads are written once in CLIF (`relation_reads.metal`, generated: decision 0325); these name them as
// the host's are. A read takes a base and the descriptor's offset from it: here the descriptor itself, at 0.
inline device const uchar* desc(device const void* p) { return (device const uchar*)p; }
inline long rows_get(device const Rows* r, uint i, uint c) { return long(clif_rows_get(desc(r), 0ul, i, c)); }
inline void rows_put(device const Rows* r, uint i, uint c, long v) {
    device uchar* p = (device uchar*)desc(r) + clif_rows_at(desc(r), 0ul, i, c);
    if (clif_rows_width(desc(r), 0ul, c) == 4u) { *(device uint*)p = uint(v); } else { *(device long*)p = v; }
}
inline bool dead_bit(device const Relation* r, uint id) { return clif_rel_dead(desc(r), 0ul, id) != 0u; }

// A relation as readers see it: a base and its delta, or a full relation (base null).
struct View { device const Relation* base; device const Relation* top; };
inline long v_get(View v, uint id, uint c) {
    return id < v.top->base_rows ? rows_get(&v.base->rows, id, c) : rows_get(&v.top->rows, id - v.top->base_rows, c);
}
inline bool v_live(View v, uint id) {
    if (dead_bit(v.top, id)) return false;
    return id >= v.top->base_rows || !dead_bit(v.base, id);
}
// A key's group in one relation's index: (start, len) into its rows; bounded by the table's size.
inline uint2 rel_group(device const Relation* r, uint idx, ulong h) {
    ulong g = clif_rel_group(desc(r), 0ul, idx, h);
    return uint2(uint(g >> 32), uint(g));
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
        for (uint j = 0u; j < g.y; j++) {
            uint rid = clif_rel_group_row(desc(r), 0ul, a.idx, g.x + j);
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
    s += crate::RELATION_READS;
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
    /// The join kernels on `device`, dispatched on `queue`: the process's one device and queue (decision 0100 §2,
    /// `rerail_metal::device()`/`queue()`), which this crate, below their owner, does not make.
    pub fn on(device: Device, queue: CommandQueue) -> Result<Gpu, String> {
        // A kernel that does not compile is fatal (decision 0382).
        let fatal = |name: &str, what: &str, error: String| -> ! { panic!("Metal: the {what} of `{name}` does not compile (decision 0382: fatal): {error}") };
        let lib = device.new_library_with_source(&source(), &CompileOptions::new()).unwrap_or_else(|e| fatal("join", "library", e));
        let pipe = |name: &str| -> ComputePipelineState {
            let f = lib.get_function(name, None).unwrap_or_else(|e| fatal(name, "function", e));
            device.new_compute_pipeline_state_with_function(&f).unwrap_or_else(|e| fatal(name, "pipeline", e))
        };
        let (count, write) = (pipe("join_count"), pipe("join_write"));
        Ok(Gpu { device, queue, count, write })
    }

    /// The block as a no-copy shared buffer: its pages are the device's too. It covers what the block has used
    /// (rounded to 1 MiB), not its whole reservation: a command buffer's cost grows with the length of the buffers
    /// it makes resident (measured: 2 GiB wrapped cost 120 ms a join, 256 MiB 17 ms).
    /// The block's device buffer: kept by the block, so a dispatch never pays a fresh buffer's mapping.
    pub fn wrap(&self, b: &Block) -> Buffer {
        b.gpu_buffer(&self.device)
    }

    /// A fresh no-copy wrap of `len` bytes of the block, not kept: for measuring what keeping saves.
    pub fn wrap_len(&self, b: &Block, len: usize) -> Buffer {
        let len = len.div_ceil(1 << 20).max(1) << 20;
        self.device.new_buffer_with_bytes_no_copy(b.base_ptr() as *const c_void, len.min(b.reserved()) as u64, MTLResourceOptions::StorageModeShared, None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::kind;
    use crate::relation::{RelW, Spec, freeze};
    use std::sync::Mutex;

    /// The kernels read a relation as the host does, through the reads written once (decision 0325): every row's
    /// value in each column (a 4-byte and an 8-byte one, over many chunks), its dead bit, and its key's index group,
    /// for a frozen base and a frozen delta of it in another block.
    #[test]
    fn the_device_reads_a_relation_as_the_host_does() {
        let Some(device) = Device::system_default() else { return };
        let spec = Spec { widths: vec![8, 4], order: vec![0], indexes: vec![vec![1]], min_log: 4 };
        let lock = Mutex::new(());
        let base_block = Block::anon(kind::FACTS, 16 << 20);
        let base = Relation::create(&base_block, &spec, None);
        let rows: Vec<i64> = (0..5_000i64).flat_map(|i| [(i * 7919) << 33 | i, i % 37]).collect();
        RelW { block: &base_block, rel: base, lock: &lock }.extend(&rows);
        freeze(&base_block, base, &spec, None);
        let top_block = Block::anon(kind::FACTS, 16 << 20);
        let top = Relation::create(&top_block, &spec, Some(base));
        let w = RelW { block: &top_block, rel: top, lock: &lock };
        for id in (0..base.rows.len()).step_by(13) {
            w.kill(id);
        }
        w.extend(&(0..700i64).flat_map(|i| [-i - 1, 40 + i % 5]).collect::<Vec<_>>());
        freeze(&top_block, top, &spec, Some(base));

        const KERNEL: &str = r#"
kernel void reads_probe(device const uchar* b [[buffer(0)]], constant ulong& rel [[buffer(1)]], constant uint& n [[buffer(2)]],
                        device ulong* out [[buffer(3)]], uint id [[thread_position_in_grid]]) {
    if (id >= n) return;
    device ulong* o = out + id * 4u;
    device const Relation* r = (device const Relation*)(b + rel);
    o[0] = clif_rel_get(b, rel, r->base_rows + id, 0u);
    o[1] = clif_rel_get(b, rel, r->base_rows + id, 1u);
    o[2] = clif_rel_dead(b, rel, id);
    o[3] = clif_rel_group(b, rel, 0u, mix(SEED, long(o[1])));
}
"#;
        let lib = device.new_library_with_source(&(source() + KERNEL), &CompileOptions::new()).expect("the kernels compile");
        let pipe = device.new_compute_pipeline_state_with_function(&lib.get_function("reads_probe", None).unwrap()).unwrap();
        let queue = device.new_command_queue();
        for (block, rel) in [(&base_block, base), (&top_block, top)] {
            let n = rel.rows.len();
            let at = block.offset_of(std::ptr::from_ref(rel)) as u64;
            let out = device.new_buffer(u64::from(n) * 32, MTLResourceOptions::StorageModeShared);
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipe);
            enc.set_buffer(0, Some(&block.gpu_buffer(&device)), 0);
            enc.set_bytes(1, 8, (&raw const at).cast());
            enc.set_bytes(2, 4, (&raw const n).cast());
            enc.set_buffer(3, Some(&out), 0);
            enc.dispatch_threads(MTLSize::new(u64::from(n), 1, 1), MTLSize::new(64, 1, 1));
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            // SAFETY: the kernel wrote 4 u64s a row, and its command completed.
            let got = unsafe { std::slice::from_raw_parts(out.contents().cast::<u64>(), n as usize * 4) };
            for i in 0..n {
                let id = rel.base_rows + i;
                let key = rel.own(id, 1);
                let group = rel.group(0, crate::key_hash(&[key]));
                let start = if group.is_empty() { 0 } else { (group.as_ptr() as usize - rel.index(0).rows.as_slice().as_ptr() as usize) / 4 };
                let want = [rel.own(id, 0) as u64, key as u64, u64::from(rel.dead_bit(i)), ((start as u64) << 32) | group.len() as u64];
                assert_eq!(&got[i as usize * 4..][..4], &want, "row {id} of a relation of {n}");
            }
        }
    }
}
