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
    // The thread pool started before anything is timed.
    rayon::broadcast(|_| ());
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
    // The same on one thread, for the parallel passes' gain.
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let (pb1, _) = one.install(|| m.time("pack from the frozen view, one thread", || packed::pack(&v, &spec, kind::FACTS)));
    let block3 = Block::anon(kind::FACTS, 1 << 30);
    let rel3: &Relation = Relation::create(&block3, &spec, None);
    RelW { block: &block3, rel: rel3, lock: &lock }.extend(&flat);
    one.install(|| m.time("pack straight from open rows, one thread", || packed::pack_open(rel3, &spec, kind::FACTS)));
    let pk: &packed::Packed = pb.root();
    assert_eq!(pk.digest(), v.digest(), "packed reads the same");
    assert_eq!(pb1.root::<packed::Packed>().digest(), v.digest(), "on one thread, the same");
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
// Packing (frame of reference): the column's minimum and maximum first, then each thread packs one group of 32
// values, `PB` bits each (compiled in), into exactly PB words: no word is shared between threads, so no atomics.
kernel void minmax(device const uint* v [[buffer(0)]], device atomic_uint* mm [[buffer(1)]], constant uint& n [[buffer(2)]],
                   uint i [[thread_position_in_grid]], uint t [[threads_per_grid]]) {
    uint lo = 0xFFFFFFFFu, hi = 0u;
    for (uint j = i; j < n; j += t) { uint x = v[j]; lo = min(lo, x); hi = max(hi, x); }
    lo = simd_min(lo);
    hi = simd_max(hi);
    if (simd_is_first()) { atomic_fetch_min_explicit(&mm[0], lo, memory_order_relaxed); atomic_fetch_max_explicit(&mm[1], hi, memory_order_relaxed); }
}
constant uint PB [[function_constant(1)]];
// One group's 32 values into PB words at `o`, word k at o[k * stride].
inline void pack_group(device const uint* v, uint base, uint n, uint lo, device uint* o, uint stride) {
    uint cur = 0u, have = 0u, k = 0u;
    #pragma unroll
    for (uint r = 0u; r < 32u; r++) {
        uint j = base + r;
        uint x = (j < n ? v[j] : lo) - lo;
        cur |= x << have;
        have += PB;
        if (have >= 32u) { o[k * stride] = cur; k++; have -= 32u; cur = have == 0u ? 0u : x >> (PB - have); }
    }
}
// The CPU's layout: group g's words at g * PB (the same bit stream as 64-bit words).
kernel void pack_seq(device const uint* v [[buffer(0)]], device uint* w [[buffer(1)]], constant uint& n [[buffer(2)]],
                     constant uint& lo [[buffer(3)]], uint g [[thread_position_in_grid]]) {
    pack_group(v, g * 32u, n, lo, w + g * PB, 1u);
}
// The scan's layout: word k of group g at k * groups + g, so a simdgroup's stores are coalesced.
kernel void pack_inter(device const uint* v [[buffer(0)]], device uint* w [[buffer(1)]], constant uint& n [[buffer(2)]],
                       constant uint& lo [[buffer(3)]], uint g [[thread_position_in_grid]], uint groups [[threads_per_grid]]) {
    pack_group(v, g * 32u, n, lo, w + g, groups);
}
// Loads coalesced through threadgroup memory: 64 threads load their 64 groups (8 KB, rows padded to 33 words so
// a thread's reads fall in distinct banks), then each packs its own.
kernel void pack_tg(device const uint* v [[buffer(0)]], device uint* w [[buffer(1)]], constant uint& n [[buffer(2)]],
                    constant uint& lo [[buffer(3)]], uint g [[thread_position_in_grid]], uint l [[thread_position_in_threadgroup]],
                    uint tg [[threadgroup_position_in_grid]]) {
    threadgroup uint vals[64 * 33];
    uint base = tg * 64u * 32u;
    for (uint r = 0u; r < 32u; r++) {
        uint j = base + r * 64u + l;
        uint q = r * 64u + l;
        vals[(q >> 5) * 33u + (q & 31u)] = j < n ? v[j] : lo;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint cur = 0u, have = 0u, k = 0u;
    device uint* o = w + g * PB;
    #pragma unroll
    for (uint r = 0u; r < 32u; r++) {
        uint x = vals[l * 33u + r] - lo;
        cur |= x << have;
        have += PB;
        if (have >= 32u) { o[k] = cur; k++; have -= 32u; cur = have == 0u ? 0u : x >> (PB - have); }
    }
}
// RUNS and LINE on the GPU, in 32-bit operations. The line through the first and last values is
// `v0 + floor(j * slope / 2^16)`, as the CPU's. Every residual is within the column's range of 0 (the line runs
// between the first and last values), so when the range is under 2^31 the line and the residuals are exact modulo
// 2^32. `slope` comes as its high (signed) and low 32 bits: j * slope / 2^16 is j * sh * 2^16 plus j * sl / 2^16,
// the latter from the 64-bit product's two halves (`mulhi`).
inline uint line_at(uint j, uint v0, int sh, uint sl) {
    uint plo = j * sl, phi = mulhi(j, sl);
    return v0 + ((j * (uint)sh) << 16) + ((phi << 16) | (plo >> 16));
}
// The statistics a choice needs, in one read, as threadgroup partials the host folds: the range, the runs that
// start (a value unlike the one before it), and the residuals' range.
kernel void stats(device const uint* v [[buffer(0)]], device int* part [[buffer(1)]], constant uint& n [[buffer(2)]],
                  constant uint& v0 [[buffer(3)]], constant int& sh [[buffer(4)]], constant uint& sl [[buffer(5)]],
                  uint i [[thread_position_in_grid]], uint t [[threads_per_grid]], uint l [[thread_position_in_threadgroup]],
                  uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]]) {
    uint lo = 0xFFFFFFFFu, hi = 0u, runs = 0u;
    int rlo = 0x7FFFFFFF, rhi = -0x7FFFFFFF;
    for (uint j = i; j < n; j += t) {
        uint x = v[j];
        lo = min(lo, x);
        hi = max(hi, x);
        runs += (j == 0u || x != v[j - 1u]) ? 1u : 0u;
        int d = (int)(x - line_at(j, v0, sh, sl));
        rlo = min(rlo, d);
        rhi = max(rhi, d);
    }
    lo = simd_min(lo);
    hi = simd_max(hi);
    runs = simd_sum(runs);
    rlo = simd_min(rlo);
    rhi = simd_max(rhi);
    threadgroup int s[8 * 5];
    if (simd_is_first()) { s[sg * 5u] = (int)lo; s[sg * 5u + 1u] = (int)hi; s[sg * 5u + 2u] = (int)runs; s[sg * 5u + 3u] = rlo; s[sg * 5u + 4u] = rhi; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (l == 0u) {
        for (uint k = 1u; k < 8u; k++) {
            lo = min(lo, (uint)s[k * 5u]); hi = max(hi, (uint)s[k * 5u + 1u]); runs += (uint)s[k * 5u + 2u];
            rlo = min(rlo, s[k * 5u + 3u]); rhi = max(rhi, s[k * 5u + 4u]);
        }
        part[tg * 5u] = (int)lo; part[tg * 5u + 1u] = (int)hi; part[tg * 5u + 2u] = (int)runs; part[tg * 5u + 3u] = rlo; part[tg * 5u + 4u] = rhi;
    }
}
// LINE: each thread packs one group of 32 residuals (less their minimum) into PB words, as PACK's groups.
kernel void pack_line(device const uint* v [[buffer(0)]], device uint* w [[buffer(1)]], constant uint& n [[buffer(2)]],
                      constant uint& v0 [[buffer(3)]], constant int& sh [[buffer(4)]], constant uint& sl [[buffer(5)]],
                      constant int& rmin [[buffer(6)]], uint g [[thread_position_in_grid]]) {
    uint cur = 0u, have = 0u, k = 0u;
    device uint* o = w + g * PB;
    #pragma unroll
    for (uint r = 0u; r < 32u; r++) {
        uint j = g * 32u + r;
        uint x = j < n ? v[j] - line_at(j, v0, sh, sl) - (uint)rmin : 0u;
        cur |= x << have;
        have += PB;
        if (have >= 32u) { o[k] = cur; k++; have -= 32u; cur = have == 0u ? 0u : x >> (PB - have); }
    }
}
// RUNS, three steps. A thread takes a group of 32 values and a threadgroup 256 groups (8,192 values).
// 1. The runs that start in each threadgroup's values.
inline uint starts_in(device const uint* v, uint g, uint n) {
    uint c = 0u;
    for (uint r = 0u; r < 32u; r++) { uint j = g * 32u + r; c += (j < n && (j == 0u || v[j] != v[j - 1u])) ? 1u : 0u; }
    return c;
}
kernel void run_counts(device const uint* v [[buffer(0)]], device uint* counts [[buffer(1)]], constant uint& n [[buffer(2)]],
                       uint g [[thread_position_in_grid]], uint l [[thread_position_in_threadgroup]],
                       uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]]) {
    uint c = simd_sum(starts_in(v, g, n));
    threadgroup uint s[8];
    if (simd_is_first()) s[sg] = c;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (l == 0u) { uint t = 0u; for (uint k = 0u; k < 8u; k++) t += s[k]; counts[tg] = t; }
}
// 2. Their exclusive prefix sum, by one threadgroup of 1,024 threads, each over `per` consecutive counts.
kernel void scan_counts(device uint* counts [[buffer(0)]], constant uint& m [[buffer(1)]], uint l [[thread_position_in_threadgroup]],
                        uint sg [[simdgroup_index_in_threadgroup]]) {
    uint per = (m + 1023u) / 1024u, from = l * per;
    uint mine = 0u;
    for (uint k = from; k < min(from + per, m); k++) mine += counts[k];
    uint before = simd_prefix_exclusive_sum(mine);
    threadgroup uint s[32];
    if (l % 32u == 31u) s[sg] = before + mine;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint k = 0u; k < sg; k++) before += s[k];
    for (uint k = from; k < min(from + per, m); k++) { uint c = counts[k]; counts[k] = before; before += c; }
}
// Bits `x` (under 2^bits, bits < 32) at bit `at` of words a thread may share with its neighbours: ORed atomically.
inline void put(device atomic_uint* w, ulong at, uint x, uint bits) {
    uint k = (uint)(at >> 5), sh = (uint)(at & 31u);
    atomic_fetch_or_explicit(&w[k], x << sh, memory_order_relaxed);
    if (sh + bits > 32u) atomic_fetch_or_explicit(&w[k + 1u], x >> (32u - sh), memory_order_relaxed);
}
// 3. Each thread finds its first run (its threadgroup's offset, and the runs of the groups before it in the
// threadgroup), then writes each run that starts in its group: the value less the minimum, and the position.
kernel void run_write(device const uint* v [[buffer(0)]], device const uint* offs [[buffer(1)]], device atomic_uint* data [[buffer(2)]],
                      device atomic_uint* runs [[buffer(3)]], constant uint& n [[buffer(4)]], constant uint& lo [[buffer(5)]],
                      constant uint& bits [[buffer(6)]], constant uint& rbits [[buffer(7)]],
                      uint g [[thread_position_in_grid]], uint l [[thread_position_in_threadgroup]],
                      uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]]) {
    uint c = starts_in(v, g, n);
    uint r = simd_prefix_exclusive_sum(c);
    threadgroup uint s[8];
    if (l % 32u == 31u) s[sg] = r + c;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint k = 0u; k < sg; k++) r += s[k];
    r += offs[tg];
    for (uint q = 0u; q < 32u; q++) {
        uint j = g * 32u + q;
        if (j < n && (j == 0u || v[j] != v[j - 1u])) {
            put(data, (ulong)r * bits, v[j] - lo, bits);
            put(runs, (ulong)r * rbits, j, rbits);
            r++;
        }
    }
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
    // Packing on the GPU against the CPU's, the same column: the bit stream must be the CPU's, word for word.
    let pipe_pb = |name: &str, bits: u32| {
        let fc = FunctionConstantValues::new();
        fc.set_constant_value_at_index((&raw const bits).cast(), MTLDataType::UInt, 1);
        device.new_compute_pipeline_state_with_function(&lib.get_function(name, Some(fc)).unwrap()).unwrap()
    };
    let mm = device.new_buffer(8, opts);
    let mmp = pipe("minmax");
    let time = |f: &dyn Fn(&ComputeCommandEncoderRef)| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..6 {
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            f(enc);
            enc.end_encoding();
            let t = Instant::now();
            cmd.commit();
            cmd.wait_until_completed();
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        best
    };
    let t_mm = time(&|enc| {
        unsafe { *(mm.contents() as *mut [u32; 2]) = [u32::MAX, 0] };
        enc.set_compute_pipeline_state(&mmp);
        enc.set_buffer(0, Some(&bp), 0);
        enc.set_buffer(1, Some(&mm), 0);
        enc.set_bytes(2, 4, (&raw const n).cast());
        enc.dispatch_threads(MTLSize::new(1 << 20, 1, 1), MTLSize::new(256, 1, 1));
    });
    let [lo, hi] = unsafe { *(mm.contents() as *const [u32; 2]) };
    let pbits = 32 - (hi - lo).leading_zeros();
    assert_eq!((lo as i64, pbits), (e.base, e.bits), "the GPU's range is the CPU's");
    let groups = (n as u64).div_ceil(32);
    let out_seq = device.new_buffer(groups * pbits as u64 * 4 + 8, opts);
    let out_inter = device.new_buffer(groups * pbits as u64 * 4 + 8, opts);
    let out_tg = device.new_buffer(groups * pbits as u64 * 4 + 8, opts);
    let packing = |name: &str, out: &Buffer, tgs: u64| {
        let p = pipe_pb(name, pbits);
        time(&|enc| {
            enc.set_compute_pipeline_state(&p);
            enc.set_buffer(0, Some(&bp), 0);
            enc.set_buffer(1, Some(out), 0);
            enc.set_bytes(2, 4, (&raw const n).cast());
            enc.set_bytes(3, 4, (&raw const lo).cast());
            enc.dispatch_threads(MTLSize::new(groups, 1, 1), MTLSize::new(tgs, 1, 1));
        })
    };
    let t_seq = packing("pack_seq", &out_seq, 256);
    let t_inter = packing("pack_inter", &out_inter, 256);
    let t_tg = packing("pack_tg", &out_tg, 64);
    let seq: &[u32] = unsafe { std::slice::from_raw_parts(out_seq.contents().cast(), groups as usize * bits) };
    assert_eq!(seq, &words32[..groups as usize * bits], "the GPU packs the CPU's bit stream");
    let tgw: &[u32] = unsafe { std::slice::from_raw_parts(out_tg.contents().cast(), groups as usize * bits) };
    assert_eq!(tgw, seq, "through threadgroup memory, the same");
    let iw: &[u32] = unsafe { std::slice::from_raw_parts(out_inter.contents().cast(), groups as usize * bits) };
    assert_eq!(iw, &inter[..], "the interleaved layout is the scan's");
    // The CPU: the range and the packing of the same values (`choose` + `write`, as packing a column), on one
    // thread and on all of them (pieces of 64K values in parallel).
    let vals: Vec<i64> = plain.iter().map(|&x| x as i64).collect();
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let cpu = |pool: Option<&rayon::ThreadPool>| {
        let run = || {
            let (t, i0) = (Instant::now(), retired());
            let ch = col::choose(&vals);
            let t_choose = t.elapsed().as_secs_f64() * 1e3;
            let mut data = vec![0u64; ch.data_words];
            let t = Instant::now();
            col::write(&ch, &vals, &mut data, &mut []);
            let t_write = t.elapsed().as_secs_f64() * 1e3;
            let insn = retired() - i0;
            assert_eq!(data, e.data);
            (t_choose, t_write, insn)
        };
        match pool {
            Some(p) => p.install(run),
            None => run(),
        }
    };
    let cpu1 = cpu(Some(&one));
    let cpun = cpu(None);
    let gbs = |bytes: usize, ms: f64| bytes as f64 / ms / 1e6;
    println!("GPU, 64M values under 2^18 (best of 6, warm, buffers kept):");
    println!("  scan   plain  {:>8} MB {:>7.2} ms  {:>6.1} GB/s", plain.len() * 4 >> 20, sp, gbs(plain.len() * 4, sp));
    println!("  scan   packed {:>8} MB {:>7.2} ms  {:>6.1} GB/s of packed bytes ({} bits)", e.data.len() * 8 >> 20, sk, gbs(e.data.len() * 8, sk), e.bits);
    println!("  scan   grouped{:>8} MB {:>7.2} ms  {:>6.1} GB/s of packed bytes (bits compiled in, 32 a thread)", inter.len() * 4 >> 20, sg, gbs(inter.len() * 4, sg));
    println!("  gather plain  16M reads {:>7.2} ms", gp);
    println!("  gather packed 16M reads {:>7.2} ms", gk);
    println!("packing 64M values ({} MB in, {} MB out, {} bits):", plain.len() * 4 >> 20, seq.len() * 4 >> 20, pbits);
    println!("  GPU min/max                  {:>7.2} ms", t_mm);
    println!("  GPU pack, CPU layout         {:>7.2} ms  {:>6.1} GB/s read+written", t_seq, gbs(plain.len() * 4 + seq.len() * 4, t_seq));
    println!("  GPU pack, interleaved        {:>7.2} ms  {:>6.1} GB/s", t_inter, gbs(plain.len() * 4 + seq.len() * 4, t_inter));
    println!("  GPU pack, loads via tg mem   {:>7.2} ms  {:>6.1} GB/s", t_tg, gbs(plain.len() * 4 + seq.len() * 4, t_tg));
    for (what, (c, w, insn)) in [("one thread", cpu1), (&*format!("{} threads", rayon::current_num_threads()), cpun)] {
        println!("  CPU choose + write, {:<11} {:>7.2} + {:>6.2} ms  ({} insn, {:.1} a value)", what, c, w, insn, insn as f64 / vals.len() as f64);
    }
    drop((vals, plain));
    runs_and_line(&device, &queue, &lib, &one);
}

/// RUNS and LINE packed on the GPU, each against the CPU's encoder word for word: the choice's statistics (range,
/// runs, the line's residuals) as threadgroup partials folded on the host, then the writes.
#[cfg(target_os = "macos")]
fn runs_and_line(device: &metal::Device, queue: &metal::CommandQueue, lib: &metal::Library, one: &rayon::ThreadPool) {
    use metal::*;
    let n: u32 = 32 << 20;
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    // Sorted, in runs of 1 to 64 equal values, rising by 1 to 1,000 between runs: RUNS wins (a line misses by far).
    let mut runs_col = Vec::with_capacity(n as usize);
    let mut v = 0u32;
    while runs_col.len() < n as usize {
        let len = 1 + (rnd() % 64) as usize;
        runs_col.extend(std::iter::repeat_n(v, len.min(n as usize - runs_col.len())));
        v += 1 + (rnd() % 1000) as u32;
    }
    // Ids in canonical order: three a row and a little noise, so LINE wins.
    let line_col: Vec<u32> = (0..n).map(|i| 50_000 + i * 3 + (rnd() % 5) as u32).collect();
    let opts = MTLResourceOptions::StorageModeShared;
    let pipe = |name: &str, pb: Option<u32>| {
        let f = match pb {
            Some(b) => {
                let fc = FunctionConstantValues::new();
                fc.set_constant_value_at_index((&raw const b).cast(), MTLDataType::UInt, 1);
                lib.get_function(name, Some(fc)).unwrap()
            }
            None => lib.get_function(name, None).unwrap(),
        };
        device.new_compute_pipeline_state_with_function(&f).unwrap()
    };
    let best = |f: &dyn Fn(&CommandBufferRef)| -> f64 {
        let mut b = f64::MAX;
        for _ in 0..6 {
            let cmd = queue.new_command_buffer();
            f(cmd);
            let t = Instant::now();
            cmd.commit();
            cmd.wait_until_completed();
            b = b.min(t.elapsed().as_secs_f64() * 1e3);
        }
        b
    };
    let set = |enc: &ComputeCommandEncoderRef, i: u64, x: &dyn std::any::Any| {
        if let Some(v) = x.downcast_ref::<u32>() {
            enc.set_bytes(i, 4, (v as *const u32).cast());
        } else if let Some(v) = x.downcast_ref::<i64>() {
            enc.set_bytes(i, 8, (v as *const i64).cast());
        }
    };
    let (threads, tgs) = (1u64 << 20, 256u64);
    let stats_p = pipe("stats", None);
    println!("RUNS and LINE on the GPU, 32M values (best of 6, warm):");
    for (what, col) in [("runs", &runs_col), ("line", &line_col)] {
        let vals: Vec<i64> = col.iter().map(|&x| x as i64).collect();
        let want = col::encode(&vals);
        let cpu1 = one.install(|| {
            let t = Instant::now();
            col::encode(&vals);
            t.elapsed().as_secs_f64() * 1e3
        });
        let t = Instant::now();
        col::encode(&vals);
        let cpun = t.elapsed().as_secs_f64() * 1e3;
        drop(vals);
        let bv = device.new_buffer_with_data(col.as_ptr().cast(), n as u64 * 4, opts);
        // The choice in one read: the range, the runs and the line's residuals, folded from threadgroup partials.
        let parts = (threads / tgs) as usize;
        let part = device.new_buffer(parts as u64 * 20, opts);
        let (v0, vn) = (col[0], col[n as usize - 1]);
        let slope = (((vn as i128 - v0 as i128) << 16) / (n as i128 - 1)) as i64;
        let (sh, sl) = ((slope >> 32) as i32, slope as u32);
        let t_stats = best(&|cmd| {
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&stats_p);
            enc.set_buffer(0, Some(&bv), 0);
            enc.set_buffer(1, Some(&part), 0);
            set(enc, 2, &n);
            set(enc, 3, &v0);
            set(enc, 4, &(sh as u32));
            set(enc, 5, &sl);
            enc.dispatch_threads(MTLSize::new(threads, 1, 1), MTLSize::new(tgs, 1, 1));
            enc.end_encoding();
        });
        let p: &[i32] = unsafe { std::slice::from_raw_parts(part.contents().cast(), parts * 5) };
        let (lo, hi, nruns, rlo, rhi) = p.chunks(5).fold((u32::MAX, 0u32, 0u64, i32::MAX, i32::MIN), |(a, b, r, c, d), q| {
            (a.min(q[0] as u32), b.max(q[1] as u32), r + q[2] as u64, c.min(q[3]), d.max(q[4]))
        });
        // The residuals are exact when the range is under 2^31; past it, LINE is not tried here.
        let line_ok = hi - lo < 1 << 31;
        let (rlo, rhi) = (rlo as i64, rhi as i64);
        // The same choice as the CPU's, by exact size.
        let bits_for = |r: u64| 64 - r.leading_zeros();
        let words = |k: u64, b: u32| (k * b as u64).div_ceil(64) + 1;
        let (bits, rbits, lbits) = (bits_for((hi - lo) as u64), bits_for(n as u64), bits_for((rhi - rlo) as u64));
        let sizes = [words(n as u64, bits), words(nruns, bits) + words(nruns, rbits), if line_ok { words(n as u64, lbits) } else { u64::MAX }];
        let tag = if sizes[1] <= sizes[0].min(sizes[2]) { col::RUNS } else if sizes[2] < sizes[0] { col::LINE } else { col::PACK };
        assert_eq!(tag, want.tag, "{what}: the GPU chooses as the CPU does");
        let t_write;
        let got: (Vec<u32>, Vec<u32>);
        if tag == col::RUNS {
            assert_eq!((lo as i64, bits, rbits, nruns as u32), (want.base, want.bits, want.rbits, want.nruns), "{what}: the choice");
            let m = (n as u64).div_ceil(32 * tgs);
            let offs = device.new_buffer(m * 4, opts);
            let (dw, rw) = ((nruns * bits as u64).div_ceil(32) + 2, (nruns * rbits as u64).div_ceil(32) + 2);
            let (bd, br) = (device.new_buffer(dw * 4, opts), device.new_buffer(rw * 4, opts));
            let (cp, sp, wp) = (pipe("run_counts", None), pipe("scan_counts", None), pipe("run_write", None));
            let m32 = m as u32;
            t_write = best(&|cmd| {
                let blit = cmd.new_blit_command_encoder();
                blit.fill_buffer(&bd, NSRange::new(0, dw * 4), 0);
                blit.fill_buffer(&br, NSRange::new(0, rw * 4), 0);
                blit.end_encoding();
                let enc = cmd.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&cp);
                enc.set_buffer(0, Some(&bv), 0);
                enc.set_buffer(1, Some(&offs), 0);
                set(enc, 2, &n);
                enc.dispatch_threads(MTLSize::new(m * tgs, 1, 1), MTLSize::new(tgs, 1, 1));
                enc.end_encoding();
                let enc = cmd.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&sp);
                enc.set_buffer(0, Some(&offs), 0);
                set(enc, 1, &m32);
                enc.dispatch_threads(MTLSize::new(1024, 1, 1), MTLSize::new(1024, 1, 1));
                enc.end_encoding();
                let enc = cmd.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&wp);
                enc.set_buffer(0, Some(&bv), 0);
                enc.set_buffer(1, Some(&offs), 0);
                enc.set_buffer(2, Some(&bd), 0);
                enc.set_buffer(3, Some(&br), 0);
                set(enc, 4, &n);
                set(enc, 5, &lo);
                set(enc, 6, &bits);
                set(enc, 7, &rbits);
                enc.dispatch_threads(MTLSize::new(m * tgs, 1, 1), MTLSize::new(tgs, 1, 1));
                enc.end_encoding();
            });
            let read = |b: &Buffer, k: u64| unsafe { std::slice::from_raw_parts(b.contents().cast::<u32>(), k as usize) }.to_vec();
            got = (read(&bd, (nruns * bits as u64).div_ceil(32)), read(&br, (nruns * rbits as u64).div_ceil(32)));
        } else {
            assert_eq!(tag, col::LINE);
            assert_eq!((v0 as i64, slope, rlo, lbits), (want.base, want.slope, want.rmin, want.bits), "{what}: the choice");
            let groups = (n as u64).div_ceil(32);
            let bd = device.new_buffer(groups * lbits as u64 * 4 + 8, opts);
            let lp = pipe("pack_line", Some(lbits));
            t_write = best(&|cmd| {
                let enc = cmd.new_compute_command_encoder();
                enc.set_compute_pipeline_state(&lp);
                enc.set_buffer(0, Some(&bv), 0);
                enc.set_buffer(1, Some(&bd), 0);
                set(enc, 2, &n);
                set(enc, 3, &v0);
                set(enc, 4, &(sh as u32));
                set(enc, 5, &sl);
                set(enc, 6, &(rlo as i32 as u32));
                enc.dispatch_threads(MTLSize::new(groups, 1, 1), MTLSize::new(tgs, 1, 1));
                enc.end_encoding();
            });
            let k = (n as u64 * lbits as u64).div_ceil(32) as usize;
            got = (unsafe { std::slice::from_raw_parts(bd.contents().cast::<u32>(), k) }.to_vec(), Vec::new());
        }
        let as32 = |w: &[u64], k: usize| -> Vec<u32> { unsafe { std::slice::from_raw_parts(w.as_ptr().cast::<u32>(), k) }.to_vec() };
        assert_eq!(got.0, as32(&want.data, got.0.len()), "{what}: the GPU's words are the CPU's");
        assert_eq!(got.1, as32(&want.runs, got.1.len()), "{what}: the GPU's run starts are the CPU's");
        println!(
            "  {what}: {} bits{}  GPU: choose {:.2} + write {:.2} ms   CPU encode: one thread {:.1} ms, all {:.1} ms",
            want.bits,
            if tag == col::RUNS { format!(", {} runs of {} bits", nruns, rbits) } else { String::new() },
            t_stats,
            t_write,
            cpu1,
            cpun
        );
    }
}
