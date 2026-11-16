//! What the two frozen forms cost to make and to read: a relation frozen in place (sorted in its chunks, hash
//! indexes) against the same rows packed (exactly sized, bit-packed columns), on the CPU; and a column scanned
//! and gathered on the GPU, plain against packed.
//!
//! ```text
//! cargo run -p rerail-blocks --profile profiling --example encodings
//! ```

use rerail_blocks::block::{Block, kind};
use rerail_blocks::col;
use rerail_blocks::packed;
use rerail_blocks::relation::{self, RelW, Relation, Spec, View};
use rerail_blocks::retired;
use std::time::Instant;

const N: usize = 1_000_000;

/// The comparison test's rows: (caller, callee, doc), about 10% repeats.
fn rows() -> Vec<i64> {
    let mut v = Vec::with_capacity(N * 3);
    let mut x: u64 = 0x2545F4914F6CDD1D;
    for i in 0..N {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let j = if i % 10 == 9 { (x as usize) % i.max(1) } else { i };
        v.extend_from_slice(&[(j * 7 % 400_000) as i64, (j * 13 % 200_000) as i64, (j / 300) as i64]);
    }
    v
}

struct M {
    lines: Vec<String>,
}

impl M {
    fn time<T>(&mut self, what: &str, f: impl FnOnce() -> T) -> T {
        let (i0, t0) = (retired(), Instant::now());
        let out = f();
        self.lines.push(format!("  {:<52} {:>14} insn {:>9.1} ms", what, retired() - i0, t0.elapsed().as_secs_f64() * 1e3));
        out
    }
}

fn main() {
    let flat = rows();
    let mut m = M { lines: Vec::new() };
    let spec = Spec { widths: vec![4; 3], order: vec![0, 1, 2], indexes: vec![vec![0], vec![1]], min_log: 10 };

    // In place: rows claimed into chunks, frozen where they are.
    let block = Block::anon(kind::FACTS, 1 << 30);
    let lock = std::sync::Mutex::new(());
    let rel: &Relation = Relation::create(&block, &spec, None);
    RelW { block: &block, rel, lock: &lock }.extend(&flat);
    let st = m.time("freeze in place (sort in place, hash index on col 1)", || relation::freeze(&block, rel, &spec, None));
    let v = View { base: None, top: rel };
    let in_place = rel.held_bytes();

    // Packed, two ways: from the frozen view (a roll-up's path), and straight from open rows.
    let (pb, ps) = m.time("pack from the frozen view", || packed::pack(&v, &spec, kind::FACTS));
    let block2 = Block::anon(kind::FACTS, 1 << 30);
    let rel2: &Relation = Relation::create(&block2, &spec, None);
    RelW { block: &block2, rel: rel2, lock: &lock }.extend(&flat);
    let (_pb2, ps2) = m.time("pack straight from open rows (no in-place freeze)", || packed::pack_open(rel2, &spec, kind::FACTS));
    let pk: &packed::Packed = pb.root();
    assert_eq!(pk.digest(), v.digest(), "packed reads the same");
    assert_eq!(ps.bytes, ps2.bytes);

    let key = |i: usize| (i * 7919 % 200_000) as i64;
    let a = m.time("in place: 1M probes on col 1 (hash bucket)", || {
        let mut s = 0i64;
        for i in 0..N {
            for id in v.probe_iter(1, &[key(i)]) {
                s = s.wrapping_add(v.get(id, 2));
            }
        }
        s
    });
    let b = m.time("packed: 1M probes on col 1 (permutation index)", || {
        let mut s = 0i64;
        for i in 0..N {
            for p in pk.probe(&[1], &[key(i)]) {
                s = s.wrapping_add(pk.get(p, 2));
            }
        }
        s
    });
    assert_eq!(a, b);
    let key0 = |i: usize| (i * 7919 % 400_000) as i64;
    let a = m.time("in place: 1M probes on col 0 (binary search, sorted)", || {
        let mut s = 0i64;
        for i in 0..N {
            for id in v.probe_iter(0, &[key0(i)]) {
                s = s.wrapping_add(v.get(id, 2));
            }
        }
        s
    });
    let b = m.time("packed: 1M probes on col 0 (binary search on runs)", || {
        let mut s = 0i64;
        for i in 0..N {
            for p in pk.probe(&[0], &[key0(i)]) {
                s = s.wrapping_add(pk.get(p, 2));
            }
        }
        s
    });
    assert_eq!(a, b);
    let a = m.time("in place: scan col 1, every row", || {
        let mut s = 0i64;
        for id in v.ordered_iter() {
            s = s.wrapping_add(v.get(id, 1));
        }
        s
    });
    let b = m.time("packed: scan col 1, every row (get)", || {
        let mut s = 0i64;
        for p in 0..pk.len() {
            s = s.wrapping_add(pk.get(p, 1));
        }
        s
    });
    let c = m.time("packed: scan col 1, every row (for_each)", || {
        let mut s = 0i64;
        pk.col(1).for_each(|x| s = s.wrapping_add(x));
        s
    });
    assert!(a == b && b == c);

    println!("{} rows pushed, {} live", N, st.live);
    println!("in place: {} bytes ({:.1} B/row); packed: {} bytes ({:.1} B/row, index {} B); cols {:?}", in_place, in_place as f64 / st.live as f64, ps.bytes, ps.bytes as f64 / st.live as f64, ps.index_bytes, ps.cols);
    println!("{}", m.lines.join("\n"));

    #[cfg(target_os = "macos")]
    gpu();
}

/// A column of 64M values under 2^18 on the GPU: 4 bytes a value plain, 18 bits packed. A scan reads every value
/// once; a gather reads 16M values at scattered positions.
#[cfg(target_os = "macos")]
fn gpu() {
    use metal::*;
    const SRC: &str = r#"
#include <metal_stdlib>
using namespace metal;
// 32-bit operations only (the GPU emulates 64-bit shifts): the packed 64-bit words read as pairs of 32-bit
// words, the same bit stream on a little-endian machine. `bits` < 32.
inline uint unpack(device const uint* w, uint i, uint bits) {
    uint bit = i * bits;
    uint k = bit >> 5, sh = bit & 31u;
    uint lo = w[k] >> sh;
    uint hi = sh == 0u ? 0u : (w[k + 1u] << (32u - sh));
    return (lo | hi) & ((1u << bits) - 1u);
}
kernel void scan_plain(device const uint* v [[buffer(0)]], device atomic_uint* out [[buffer(1)]], constant uint& n [[buffer(2)]],
                       uint i [[thread_position_in_grid]]) {
    uint s = 0u, t = n / 16u;
    for (uint r = 0u; r < 16u; r++) s += v[i + r * t];
    s = simd_sum(s);
    if (simd_is_first()) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}
kernel void scan_packed(device const uint* w [[buffer(0)]], device atomic_uint* out [[buffer(1)]], constant uint& n [[buffer(2)]],
                        constant uint& bits [[buffer(3)]], uint i [[thread_position_in_grid]]) {
    uint s = 0u, t = n / 16u;
    for (uint r = 0u; r < 16u; r++) s += unpack(w, i + r * t, bits);
    s = simd_sum(s);
    if (simd_is_first()) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}
// `BITS` compiled in: a thread decodes a group of 32 values, which is exactly BITS aligned 32-bit words, each
// loaded once; the shifts are constants after unrolling. Group g's words are interleaved across threads
// (word k of group g at k * groups + g), so a simdgroup's loads are coalesced.
constant uint BITS [[function_constant(0)]];
kernel void scan_group(device const uint* w [[buffer(0)]], device atomic_uint* out [[buffer(1)]], constant uint& n [[buffer(2)]],
                       uint g [[thread_position_in_grid]]) {
    uint groups = n / 32u;
    uint s = 0u, mask = (1u << BITS) - 1u;
    uint cur = w[g], k = 1u, have = 32u;
    #pragma unroll
    for (uint v = 0u; v < 32u; v++) {
        uint x;
        if (have >= BITS) { x = cur & mask; cur = have == BITS ? 0u : cur >> BITS; have -= BITS; }
        else { uint nx = w[k * groups + g]; k++; x = (cur | (nx << have)) & mask; cur = have == 0u ? nx >> BITS : nx >> (BITS - have); have += 32u - BITS; }
        s += x;
    }
    s = simd_sum(s);
    if (simd_is_first()) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}
inline uint at(uint i, uint n) { return (i * 2654435761u) % n; }
kernel void gather_plain(device const uint* v [[buffer(0)]], device atomic_uint* out [[buffer(1)]], constant uint& n [[buffer(2)]],
                         uint i [[thread_position_in_grid]]) {
    uint s = v[at(i, n)];
    s = simd_sum(s);
    if (simd_is_first()) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}
kernel void gather_packed(device const uint* w [[buffer(0)]], device atomic_uint* out [[buffer(1)]], constant uint& n [[buffer(2)]],
                          constant uint& bits [[buffer(3)]], uint i [[thread_position_in_grid]]) {
    uint s = unpack(w, at(i, n), bits);
    s = simd_sum(s);
    if (simd_is_first()) atomic_fetch_add_explicit(out, s, memory_order_relaxed);
}
"#;
    let device = Device::system_default().expect("a Metal device");
    let lib = device.new_library_with_source(SRC, &CompileOptions::new()).unwrap();
    let pipe = |n: &str| device.new_compute_pipeline_state_with_function(&lib.get_function(n, None).unwrap()).unwrap();
    let pipe_bits = |n: &str, bits: u32| {
        let fc = FunctionConstantValues::new();
        fc.set_constant_value_at_index((&raw const bits).cast(), MTLDataType::UInt, 0);
        device.new_compute_pipeline_state_with_function(&lib.get_function(n, Some(fc)).unwrap()).unwrap()
    };
    let queue = device.new_command_queue();
    let n: u32 = 64 << 20;
    let vals: Vec<i64> = (0..n as u64).map(|i| ((i * 2654435761) % (1 << 18)) as i64).collect();
    let e = col::encode(&vals);
    assert_eq!(e.tag, col::PACK);
    let plain: Vec<u32> = vals.iter().map(|&x| x as u32).collect();
    drop(vals);
    let opts = MTLResourceOptions::StorageModeShared;
    let bp = device.new_buffer_with_data(plain.as_ptr().cast(), (plain.len() * 4) as u64, opts);
    let bk = device.new_buffer_with_data(e.data.as_ptr().cast(), (e.data.len() * 8) as u64, opts);
    let out = device.new_buffer(4, opts);
    let run = |p: &ComputePipelineState, buf: &Buffer, threads: u64, bits: Option<u32>| -> (f64, u32) {
        let mut best = f64::MAX;
        let mut sum = 0;
        for _ in 0..6 {
            unsafe { *(out.contents() as *mut u32) = 0 };
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(p);
            enc.set_buffer(0, Some(buf), 0);
            enc.set_buffer(1, Some(&out), 0);
            enc.set_bytes(2, 4, (&raw const n).cast());
            if let Some(b) = bits {
                enc.set_bytes(3, 4, (&raw const b).cast());
            }
            enc.dispatch_threads(MTLSize::new(threads, 1, 1), MTLSize::new(256, 1, 1));
            enc.end_encoding();
            let t = Instant::now();
            cmd.commit();
            cmd.wait_until_completed();
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
            sum = unsafe { *(out.contents() as *const u32) };
        }
        (best, sum)
    };
    let (sp, a) = run(&pipe("scan_plain"), &bp, n as u64 / 16, None);
    let (sk, b) = run(&pipe("scan_packed"), &bk, n as u64 / 16, Some(e.bits));
    assert_eq!(a, b, "the packed scan sums the same");
    // The grouped layout: group g's 32 values in BITS words, word k at k * groups + g.
    let groups = n as usize / 32;
    let bits = e.bits as usize;
    let words32: &[u32] = unsafe { std::slice::from_raw_parts(e.data.as_ptr().cast(), e.data.len() * 2) };
    let mut inter = vec![0u32; groups * bits];
    for g in 0..groups {
        for k in 0..bits {
            inter[k * groups + g] = words32[g * bits + k];
        }
    }
    let bg = device.new_buffer_with_data(inter.as_ptr().cast(), (inter.len() * 4) as u64, opts);
    let (sg, c) = run(&pipe_bits("scan_group", e.bits), &bg, groups as u64, None);
    assert_eq!(a, c, "the grouped scan sums the same");
    let g = 16u64 << 20;
    let (gp, a) = run(&pipe("gather_plain"), &bp, g, None);
    let (gk, b) = run(&pipe("gather_packed"), &bk, g, Some(e.bits));
    assert_eq!(a, b, "the packed gather sums the same");
    let gbs = |bytes: usize, ms: f64| bytes as f64 / ms / 1e6;
    println!("GPU, 64M values under 2^18 (best of 6, warm, buffers kept):");
    println!("  scan   plain  {:>8} MB {:>7.2} ms  {:>6.1} GB/s", plain.len() * 4 >> 20, sp, gbs(plain.len() * 4, sp));
    println!("  scan   packed {:>8} MB {:>7.2} ms  {:>6.1} GB/s of packed bytes ({} bits)", e.data.len() * 8 >> 20, sk, gbs(e.data.len() * 8, sk), e.bits);
    println!("  scan   grouped{:>8} MB {:>7.2} ms  {:>6.1} GB/s of packed bytes (bits compiled in, 32 a thread)", inter.len() * 4 >> 20, sg, gbs(inter.len() * 4, sg));
    println!("  gather plain  16M reads {:>7.2} ms", gp);
    println!("  gather packed 16M reads {:>7.2} ms", gk);
}
