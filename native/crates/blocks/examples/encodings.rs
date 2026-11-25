//! What the two frozen forms cost to make and to read: a relation frozen in place (sorted in its chunks, hash
//! indexes) against the same rows packed (exactly sized, bit-packed columns), on the CPU; and a column scanned
//! and gathered on the GPU, plain against packed.
//!
//! ```text
//! cargo run -p rerail-blocks --profile profiling --example encodings
//! ```

use rerail_blocks::block::{Block, kind};
// The device's columns (macOS, Linux and Windows, decision 0440 step C) are its only readers.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
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
    let spec = Spec { widths: vec![4; 3], sort: Some(0), indexes: vec![vec![0], vec![1]], min_log: 10 };

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

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    gpu();
}

/// The kernels, in UKL, on the process's device (`RERAIL_GPU` chooses its backend; decision 0440 step C). `BITS` and
/// `PB` are compiled in, each value a program of its own (`program_with`'s defines).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
const SRC: &str = r#"
#include <ukl/backend.h>
#include <ukl/simd.h>
#ifndef BITS
#define BITS 1u
#endif
#ifndef PB
#define PB 1u
#endif
// A thread's lane in its SIMD group, and its group in a threadgroup of 256 (`TG`).
#define TG 256u
#define LANE (lid % UKL_SIMD_WIDTH)
#define SGS (TG / UKL_SIMD_WIDTH)
// Signed values min'd and max'd as unsigned ones: their order with the sign bit flipped.
UKL_INLINE uint32_t ord(int32_t x) { return (uint32_t)x ^ 0x80000000u; }
UKL_INLINE int32_t unord(uint32_t x) { return (int32_t)(x ^ 0x80000000u); }
UKL_INLINE uint32_t umin(uint32_t a, uint32_t b) { return a < b ? a : b; }
UKL_INLINE uint32_t umax(uint32_t a, uint32_t b) { return a > b ? a : b; }

// Every kernel's parameters: a column of `n` values, and what each kernel reads of the rest.
typedef struct P { uint32_t n; uint32_t bits; uint32_t t; uint32_t lo; uint32_t v0; uint32_t sh; uint32_t sl; uint32_t rmin; uint32_t rbits; uint32_t m; uint32_t pad0; uint32_t pad1; } P;
UKL_REFLECT_STRUCT(P, n, bits, t, lo, v0, sh, sl, rmin, rbits, m, pad0, pad1)

// 32-bit operations only (the GPU emulates 64-bit shifts): the packed 64-bit words read as pairs of 32-bit
// words, the same bit stream on a little-endian machine. `bits` < 32.
UKL_INLINE uint32_t unpack(UKL_GLOBAL const uint32_t* w, uint32_t i, uint32_t bits) {
    uint32_t bit = i * bits;
    uint32_t k = bit >> 5, sh = bit & 31u;
    uint32_t lo = w[k] >> sh;
    uint32_t hi = sh == 0u ? 0u : (w[k + 1u] << (32u - sh));
    return (lo | hi) & ((1u << bits) - 1u);
}
DEFINE_KERNEL2(enc, scan_plain, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, out, "[1]", ARGS, P, a, ,
               GID0, uint32_t, i, , LID0, uint32_t, lid, )
{
    uint32_t s = 0u, t = a.n / 16u;
    for (uint32_t r = 0u; r < 16u; r++) s += v[i + r * t];
    s = ukl_sg_sum(s __UKL_ARGS);
    if (LANE == 0u) ukl_atomic_add(out, s);
}
DEFINE_KERNEL2(enc, scan_packed, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, out, "[1]", ARGS, P, a, ,
               GID0, uint32_t, i, , LID0, uint32_t, lid, )
{
    uint32_t s = 0u, t = a.n / 16u;
    for (uint32_t r = 0u; r < 16u; r++) s += unpack(v, i + r * t, a.bits);
    s = ukl_sg_sum(s __UKL_ARGS);
    if (LANE == 0u) ukl_atomic_add(out, s);
}
// `BITS` compiled in: a thread decodes a group of 32 values, which is exactly BITS aligned 32-bit words, each
// loaded once; the shifts are constants after unrolling. Group g's words are interleaved across threads
// (word k of group g at k * groups + g), so a SIMD group's loads are coalesced.
DEFINE_KERNEL2(enc, scan_group, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, out, "[1]", ARGS, P, a, ,
               GID0, uint32_t, g, , LID0, uint32_t, lid, )
{
    uint32_t groups = a.n / 32u;
    uint32_t s = 0u, mask = (1u << BITS) - 1u;
    uint32_t cur = v[g], k = 1u, have = 32u;
    for (uint32_t r = 0u; r < 32u; r++) {
        uint32_t x;
        if (have >= BITS) { x = cur & mask; cur = have == BITS ? 0u : cur >> BITS; have -= BITS; }
        else { uint32_t nx = v[k * groups + g]; k++; x = (cur | (nx << have)) & mask; cur = have == 0u ? nx >> BITS : nx >> (BITS - have); have += 32u - BITS; }
        s += x;
    }
    s = ukl_sg_sum(s __UKL_ARGS);
    if (LANE == 0u) ukl_atomic_add(out, s);
}
// Packing (frame of reference): the column's minimum and maximum first (`t` threads in the grid), then each
// thread packs one group of 32 values, `PB` bits each (compiled in), into exactly PB words: no word is shared
// between threads, so no atomics.
DEFINE_KERNEL2(enc, minmax, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, mm, "[2]", ARGS, P, a, ,
               GID0, uint32_t, i, , LID0, uint32_t, lid, )
{
    uint32_t lo = 0xFFFFFFFFu, hi = 0u;
    for (uint32_t j = i; j < a.n; j += a.t) { uint32_t x = v[j]; lo = umin(lo, x); hi = umax(hi, x); }
    lo = ukl_sg_min(lo __UKL_ARGS);
    hi = ukl_sg_max(hi __UKL_ARGS);
    if (LANE == 0u) { ukl_atomic_min(&mm[0], lo); ukl_atomic_max(&mm[1], hi); }
}
// One group's 32 values into PB words at `o`, word k at o[k * stride].
UKL_INLINE void pack_group(UKL_GLOBAL const uint32_t* v, uint32_t base, uint32_t n, uint32_t lo, UKL_GLOBAL uint32_t* o, uint32_t stride) {
    uint32_t cur = 0u, have = 0u, k = 0u;
    for (uint32_t r = 0u; r < 32u; r++) {
        uint32_t j = base + r;
        uint32_t x = (j < n ? v[j] : lo) - lo;
        cur |= x << have;
        have += PB;
        if (have >= 32u) { o[k * stride] = cur; k++; have -= 32u; cur = have == 0u ? 0u : x >> (PB - have); }
    }
}
// The CPU's layout: group g's words at g * PB (the same bit stream as 64-bit words).
DEFINE_KERNEL2(enc, pack_seq, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, w, "[n]", ARGS, P, a, , GID0, uint32_t, g, )
{
    pack_group(v, g * 32u, a.n, a.lo, w + g * PB, 1u);
}
// The scan's layout: word k of group g at k * groups + g (`t`: the groups), so a SIMD group's stores are coalesced.
DEFINE_KERNEL2(enc, pack_inter, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, w, "[n]", ARGS, P, a, , GID0, uint32_t, g, )
{
    pack_group(v, g * 32u, a.n, a.lo, w + g, a.t);
}
// Loads coalesced through threadgroup memory: 64 threads load their 64 groups (8 KB, rows padded to 33 words so
// a thread's reads fall in distinct banks), then each packs its own.
DEFINE_KERNEL2(enc, pack_tg, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, w, "[n]", ARGS, P, a, ,
               SHARED, uint32_t, vals, 2112, GID0, uint32_t, g, , LID0, uint32_t, l, , TGID0, uint32_t, tg, )
{
    uint32_t base = tg * 64u * 32u;
    for (uint32_t r = 0u; r < 32u; r++) {
        uint32_t j = base + r * 64u + l;
        uint32_t q = r * 64u + l;
        vals[(q >> 5) * 33u + (q & 31u)] = j < a.n ? v[j] : a.lo;
    }
    ukl_tg_barrier();
    uint32_t cur = 0u, have = 0u, k = 0u;
    UKL_GLOBAL uint32_t* o = w + g * PB;
    for (uint32_t r = 0u; r < 32u; r++) {
        uint32_t x = vals[l * 33u + r] - a.lo;
        cur |= x << have;
        have += PB;
        if (have >= 32u) { o[k] = cur; k++; have -= 32u; cur = have == 0u ? 0u : x >> (PB - have); }
    }
}
// RUNS and LINE on the GPU, in 32-bit operations. The line through the first and last values is
// `v0 + floor(j * slope / 2^16)`, as the CPU's. Every residual is within the column's range of 0 (the line runs
// between the first and last values), so when the range is under 2^31 the line and the residuals are exact modulo
// 2^32. `slope` comes as its high (signed) and low 32 bits: j * slope / 2^16 is j * sh * 2^16 plus j * sl / 2^16,
// the latter from the 64-bit product's two halves.
UKL_INLINE uint32_t line_at(uint32_t j, uint32_t v0, uint32_t sh, uint32_t sl) {
    uint64_t p = (uint64_t)j * sl;
    uint32_t plo = (uint32_t)p, phi = (uint32_t)(p >> 32);
    return v0 + ((j * sh) << 16) + ((phi << 16) | (plo >> 16));
}
// The statistics a choice needs, in one read, as threadgroup partials the host folds: the range, the runs that
// start (a value unlike the one before it), and the residuals' range. `t` threads in the grid.
DEFINE_KERNEL2(enc, stats, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, int32_t, part, "[groups * 5]", ARGS, P, a, ,
               SHARED, uint32_t, s, 1280, GID0, uint32_t, i, , LID0, uint32_t, lid, , TGID0, uint32_t, tg, )
{
    uint32_t lo = 0xFFFFFFFFu, hi = 0u, runs = 0u;
    int32_t rlo = 0x7FFFFFFF, rhi = -0x7FFFFFFF;
    for (uint32_t j = i; j < a.n; j += a.t) {
        uint32_t x = v[j];
        lo = umin(lo, x);
        hi = umax(hi, x);
        runs += (j == 0u || x != v[j - 1u]) ? 1u : 0u;
        int32_t d = (int32_t)(x - line_at(j, a.v0, a.sh, a.sl));
        rlo = d < rlo ? d : rlo;
        rhi = d > rhi ? d : rhi;
    }
    lo = ukl_sg_min(lo __UKL_ARGS);
    hi = ukl_sg_max(hi __UKL_ARGS);
    runs = ukl_sg_sum(runs __UKL_ARGS);
    rlo = unord(ukl_sg_min(ord(rlo) __UKL_ARGS));
    rhi = unord(ukl_sg_max(ord(rhi) __UKL_ARGS));
    uint32_t sg = lid / UKL_SIMD_WIDTH;
    if (LANE == 0u) { s[sg * 5u] = lo; s[sg * 5u + 1u] = hi; s[sg * 5u + 2u] = runs; s[sg * 5u + 3u] = (uint32_t)rlo; s[sg * 5u + 4u] = (uint32_t)rhi; }
    ukl_tg_barrier();
    if (lid == 0u) {
        for (uint32_t k = 1u; k < SGS; k++) {
            lo = umin(lo, s[k * 5u]); hi = umax(hi, s[k * 5u + 1u]); runs += s[k * 5u + 2u];
            int32_t c = (int32_t)s[k * 5u + 3u], d = (int32_t)s[k * 5u + 4u];
            rlo = c < rlo ? c : rlo; rhi = d > rhi ? d : rhi;
        }
        part[tg * 5u] = (int32_t)lo; part[tg * 5u + 1u] = (int32_t)hi; part[tg * 5u + 2u] = (int32_t)runs; part[tg * 5u + 3u] = rlo; part[tg * 5u + 4u] = rhi;
    }
}
// LINE: each thread packs one group of 32 residuals (less their minimum) into PB words, as PACK's groups.
DEFINE_KERNEL2(enc, pack_line, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, w, "[n]", ARGS, P, a, , GID0, uint32_t, g, )
{
    uint32_t cur = 0u, have = 0u, k = 0u;
    UKL_GLOBAL uint32_t* o = w + g * PB;
    for (uint32_t r = 0u; r < 32u; r++) {
        uint32_t j = g * 32u + r;
        uint32_t x = j < a.n ? v[j] - line_at(j, a.v0, a.sh, a.sl) - a.rmin : 0u;
        cur |= x << have;
        have += PB;
        if (have >= 32u) { o[k] = cur; k++; have -= 32u; cur = have == 0u ? 0u : x >> (PB - have); }
    }
}
// RUNS, three steps. A thread takes a group of 32 values and a threadgroup 256 groups (8,192 values).
// 1. The runs that start in each threadgroup's values.
UKL_INLINE uint32_t starts_in(UKL_GLOBAL const uint32_t* v, uint32_t g, uint32_t n) {
    uint32_t c = 0u;
    for (uint32_t r = 0u; r < 32u; r++) { uint32_t j = g * 32u + r; c += (j < n && (j == 0u || v[j] != v[j - 1u])) ? 1u : 0u; }
    return c;
}
DEFINE_KERNEL2(enc, run_counts, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, counts, "[m]", ARGS, P, a, ,
               SHARED, uint32_t, s, 256, GID0, uint32_t, g, , LID0, uint32_t, lid, , TGID0, uint32_t, tg, )
{
    uint32_t c = ukl_sg_sum(starts_in(v, g, a.n) __UKL_ARGS);
    if (LANE == 0u) s[lid / UKL_SIMD_WIDTH] = c;
    ukl_tg_barrier();
    if (lid == 0u) { uint32_t t = 0u; for (uint32_t k = 0u; k < SGS; k++) t += s[k]; counts[tg] = t; }
}
// 2. Their exclusive prefix sum, by one threadgroup of 256 threads, each over `per` consecutive counts.
DEFINE_KERNEL2(enc, scan_counts, RWBUFFER, uint32_t, counts, "[m]", ARGS, P, a, ,
               SHARED, uint32_t, s, 256, LID0, uint32_t, lid, )
{
    uint32_t m = a.m, per = (m + TG - 1u) / TG, from = lid * per, to = umin(from + per, m);
    uint32_t mine = 0u;
    for (uint32_t k = from; k < to; k++) mine += counts[k];
    uint32_t before = ukl_sg_exclusive_sum(mine, LANE __UKL_ARGS);
    uint32_t sg = lid / UKL_SIMD_WIDTH;
    if (LANE == UKL_SIMD_WIDTH - 1u) s[sg] = before + mine;
    ukl_tg_barrier();
    for (uint32_t k = 0u; k < sg; k++) before += s[k];
    for (uint32_t k = from; k < to; k++) { uint32_t c = counts[k]; counts[k] = before; before += c; }
}
// Bits `x` (under 2^bits, bits < 32) at bit `at` of words a thread may share with its neighbours: ORed atomically.
UKL_INLINE void put(UKL_GLOBAL uint32_t* w, uint64_t at, uint32_t x, uint32_t bits) {
    uint32_t k = (uint32_t)(at >> 5), sh = (uint32_t)(at & 31u);
    ukl_atomic_or(&w[k], x << sh);
    if (sh + bits > 32u) ukl_atomic_or(&w[k + 1u], x >> (32u - sh));
}
// 3. Each thread finds its first run (its threadgroup's offset, and the runs of the groups before it in the
// threadgroup), then writes each run that starts in its group: the value less the minimum, and the position.
DEFINE_KERNEL2(enc, run_write, ROBUFFER, uint32_t, v, "[n]", ROBUFFER, uint32_t, offs, "[m]", RWBUFFER, uint32_t, data, "[words]",
               RWBUFFER, uint32_t, runs, "[words]", ARGS, P, a, , SHARED, uint32_t, s, 256,
               GID0, uint32_t, g, , LID0, uint32_t, lid, , TGID0, uint32_t, tg, )
{
    uint32_t c = starts_in(v, g, a.n);
    uint32_t r = ukl_sg_exclusive_sum(c, LANE __UKL_ARGS);
    uint32_t sg = lid / UKL_SIMD_WIDTH;
    if (LANE == UKL_SIMD_WIDTH - 1u) s[sg] = r + c;
    ukl_tg_barrier();
    for (uint32_t k = 0u; k < sg; k++) r += s[k];
    r += offs[tg];
    for (uint32_t q = 0u; q < 32u; q++) {
        uint32_t j = g * 32u + q;
        if (j < a.n && (j == 0u || v[j] != v[j - 1u])) {
            put(data, (uint64_t)r * a.bits, v[j] - a.lo, a.bits);
            put(runs, (uint64_t)r * a.rbits, j, a.rbits);
            r++;
        }
    }
}
UKL_INLINE uint32_t at(uint32_t i, uint32_t n) { return (i * 2654435761u) % n; }
DEFINE_KERNEL2(enc, gather_plain, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, out, "[1]", ARGS, P, a, ,
               GID0, uint32_t, i, , LID0, uint32_t, lid, )
{
    uint32_t s = ukl_sg_sum(v[at(i, a.n)] __UKL_ARGS);
    if (LANE == 0u) ukl_atomic_add(out, s);
}
DEFINE_KERNEL2(enc, gather_packed, ROBUFFER, uint32_t, v, "[n]", RWBUFFER, uint32_t, out, "[1]", ARGS, P, a, ,
               GID0, uint32_t, i, , LID0, uint32_t, lid, )
{
    uint32_t s = ukl_sg_sum(unpack(v, at(i, a.n), a.bits) __UKL_ARGS);
    if (LANE == 0u) ukl_atomic_add(out, s);
}
"#;

/// Every kernel's parameters (the source's `P`).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct P {
    n: u32,
    bits: u32,
    t: u32,
    lo: u32,
    v0: u32,
    sh: u32,
    sl: u32,
    rmin: u32,
    rbits: u32,
    m: u32,
    pad0: u32,
    pad1: u32,
}
// SAFETY: `#[repr(C)]` u32s.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
unsafe impl rerail_gpu::Pod for P {}

/// The device the kernels run on, and their programs by their compiled-in widths.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
struct Dev {
    shared: &'static rerail_gpu::shared::Shared,
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
type Buf = rerail_device::device::Buffer;
/// A kernel's launch: the kernel, its buffers by name, its parameters, its threadgroups and their size.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
type Launch<'a> = (&'a rerail_gpu::device::ComputeFunction, &'a [(&'a str, &'a Buf)], P, u64, u64);

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl Dev {
    /// The kernel `name`, `BITS` and `PB` compiled in where given.
    fn kernel(&self, name: &str, bits: Option<u32>, pb: Option<u32>) -> &'static rerail_gpu::device::ComputeFunction {
        let (b, p) = (bits.map(|v| format!("{v}u")), pb.map(|v| format!("{v}u")));
        let defines: Vec<(&str, &str)> = [("BITS", b.as_deref()), ("PB", p.as_deref())].into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect();
        let program = rerail_gpu::kernels::program_with("encodings.uglykl", SRC, &defines).unwrap_or_else(|e| panic!("the kernels compile: {e}"));
        program.function(name).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Runs each of `launches` in order, in one submission, and waits.
    fn submit(&self, launches: &[Launch<'_>]) {
        let mut q = rerail_gpu::device::ComputeQueue::new(&self.shared.queue);
        for &(f, bufs, p, groups, local) in launches {
            let mut l = rerail_gpu::kernels::launch(&q, f).value("a", &p);
            for &(name, b) in bufs {
                l = l.buffer(name, b);
            }
            l.run(&[groups], &[local]).unwrap_or_else(|e| panic!("{e}"));
        }
        q.flush().await_().unwrap();
    }

    /// The best of 6 runs of `before` (on the host, untimed) then `launches`, in ms.
    fn best(&self, before: &dyn Fn(), launches: &[Launch<'_>]) -> f64 {
        let mut b = f64::MAX;
        for _ in 0..6 {
            before();
            let t = Instant::now();
            self.submit(launches);
            b = b.min(t.elapsed().as_secs_f64() * 1e3);
        }
        b
    }
}

/// The device half's columns divided by 2^`ENCODINGS_GPU_SHIFT` (default 0): for a slow device (the CPU's) or a
/// quick look; at most 6, so a threadgroup's 8,192 values divide every column.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn shift() -> u32 {
    std::env::var("ENCODINGS_GPU_SHIFT").ok().and_then(|v| v.parse().ok()).unwrap_or(0u32).min(6)
}

/// A count as the lab prints it: in M, or in K under a million.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn count(k: u64) -> String {
    if k >= 1 << 20 { format!("{}M", k >> 20) } else { format!("{}K", k >> 10) }
}

/// A buffer of `k` u32s.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn words(k: u64) -> Buf {
    rerail_gpu::kernels::alloc::<u32>(k as usize).unwrap()
}

/// Sets the first u32s of `b`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn set(b: &Buf, v: &[u32]) {
    // SAFETY: the buffer's host view, at least `v.len()` u32s, no command running.
    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), b.contents().cast::<u32>(), v.len()) };
}

/// Zeroes the first `k` u32s of `b`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn zero(b: &Buf, k: u64) {
    // SAFETY: the buffer's host view, at least `k` u32s, no command running.
    unsafe { std::ptr::write_bytes(b.contents().cast::<u32>(), 0, k as usize) };
}

/// A column of 64M values under 2^18 on the GPU: 4 bytes a value plain, 18 bits packed. A scan reads every value
/// once; a gather reads 16M values at scattered positions.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn gpu() {
    let dev = Dev { shared: rerail_gpu::shared::get().expect("a device") };
    let read = |b: &Buf, k: usize| -> Vec<u32> { rerail_gpu::kernels::read(b, k) };
    let n: u32 = (64 << 20) >> shift();
    let vals: Vec<i64> = (0..n as u64).map(|i| ((i * 2654435761) % (1 << 18)) as i64).collect();
    let e = col::encode(&vals);
    assert_eq!(e.tag, col::PACK);
    let plain: Vec<u32> = vals.iter().map(|&x| x as u32).collect();
    drop(vals);
    let bp = rerail_gpu::kernels::buffer_of(&plain).unwrap();
    let bk = rerail_gpu::kernels::buffer_of(&e.data).unwrap();
    let out = words(1);
    let p = P { n, bits: e.bits, ..Default::default() };
    // A reduction into `out`: (best ms, the sum).
    let reduce = |k: &rerail_gpu::device::ComputeFunction, buf: &Buf, threads: u64| -> (f64, u32) {
        let ms = dev.best(&|| zero(&out, 1), &[(k, &[("v", buf), ("out", &out)], p, threads / 256, 256)]);
        (ms, read(&out, 1)[0])
    };
    let (sp, a) = reduce(dev.kernel("scan_plain", None, None), &bp, n as u64 / 16);
    let (sk, b) = reduce(dev.kernel("scan_packed", None, None), &bk, n as u64 / 16);
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
    let bg = rerail_gpu::kernels::buffer_of(&inter).unwrap();
    let (sg, c) = reduce(dev.kernel("scan_group", Some(e.bits), None), &bg, groups as u64);
    assert_eq!(a, c, "the grouped scan sums the same");
    let g = (16u64 << 20) >> shift();
    let (gp, a) = reduce(dev.kernel("gather_plain", None, None), &bp, g);
    let (gk, b) = reduce(dev.kernel("gather_packed", None, None), &bk, g);
    assert_eq!(a, b, "the packed gather sums the same");
    // Packing on the GPU against the CPU's, the same column: the bit stream must be the CPU's, word for word.
    let mm = words(2);
    let mt = (1u64 << 20) >> shift();
    let t_mm = dev.best(&|| set(&mm, &[u32::MAX, 0]), &[(dev.kernel("minmax", None, None), &[("v", &bp), ("mm", &mm)], P { t: mt as u32, ..p }, mt / 256, 256)]);
    let [lo, hi] = [read(&mm, 2)[0], read(&mm, 2)[1]];
    let pbits = 32 - (hi - lo).leading_zeros();
    assert_eq!((lo as i64, pbits), (e.base, e.bits), "the GPU's range is the CPU's");
    let groups = (n as u64).div_ceil(32);
    let (out_seq, out_inter, out_tg) = (words(groups * pbits as u64 + 2), words(groups * pbits as u64 + 2), words(groups * pbits as u64 + 2));
    let packing = |name: &str, out: &Buf, local: u64| {
        let k = dev.kernel(name, None, Some(pbits));
        dev.best(&|| (), &[(k, &[("v", &bp), ("w", out)], P { lo, t: groups as u32, ..p }, groups / local, local)])
    };
    let t_seq = packing("pack_seq", &out_seq, 256);
    let t_inter = packing("pack_inter", &out_inter, 256);
    let t_tg = packing("pack_tg", &out_tg, 64);
    let seq = read(&out_seq, groups as usize * bits);
    assert_eq!(seq, &words32[..groups as usize * bits], "the GPU packs the CPU's bit stream");
    assert_eq!(read(&out_tg, groups as usize * bits), seq, "through threadgroup memory, the same");
    assert_eq!(read(&out_inter, groups as usize * bits), inter, "the interleaved layout is the scan's");
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
    let mib = |bytes: usize| bytes >> 20;
    println!("GPU, {} values under 2^18 (best of 6, warm, buffers kept):", count(n as u64));
    println!("  scan   plain  {:>8} MB {:>7.2} ms  {:>6.1} GB/s", mib(plain.len() * 4), sp, gbs(plain.len() * 4, sp));
    println!("  scan   packed {:>8} MB {:>7.2} ms  {:>6.1} GB/s of packed bytes ({} bits)", mib(e.data.len() * 8), sk, gbs(e.data.len() * 8, sk), e.bits);
    println!("  scan   grouped{:>8} MB {:>7.2} ms  {:>6.1} GB/s of packed bytes (bits compiled in, 32 a thread)", mib(inter.len() * 4), sg, gbs(inter.len() * 4, sg));
    println!("  gather plain  {} reads {:>7.2} ms", count(g), gp);
    println!("  gather packed {} reads {:>7.2} ms", count(g), gk);
    println!("packing {} values ({} MB in, {} MB out, {} bits):", count(n as u64), mib(plain.len() * 4), mib(seq.len() * 4), pbits);
    println!("  GPU min/max                  {:>7.2} ms", t_mm);
    println!("  GPU pack, CPU layout         {:>7.2} ms  {:>6.1} GB/s read+written", t_seq, gbs(plain.len() * 4 + seq.len() * 4, t_seq));
    println!("  GPU pack, interleaved        {:>7.2} ms  {:>6.1} GB/s", t_inter, gbs(plain.len() * 4 + seq.len() * 4, t_inter));
    println!("  GPU pack, loads via tg mem   {:>7.2} ms  {:>6.1} GB/s", t_tg, gbs(plain.len() * 4 + seq.len() * 4, t_tg));
    for (what, (c, w, insn)) in [("one thread", cpu1), (&*format!("{} threads", rayon::current_num_threads()), cpun)] {
        println!("  CPU choose + write, {:<11} {:>7.2} + {:>6.2} ms  ({} insn, {:.1} a value)", what, c, w, insn, insn as f64 / vals.len() as f64);
    }
    drop((vals, plain));
    runs_and_line(&dev, &one);
}

/// RUNS and LINE packed on the GPU, each against the CPU's encoder word for word: the choice's statistics (range,
/// runs, the line's residuals) as threadgroup partials folded on the host, then the writes.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn runs_and_line(dev: &Dev, one: &rayon::ThreadPool) {
    let n: u32 = (32 << 20) >> shift();
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
    let read = |b: &Buf, k: usize| -> Vec<u32> { rerail_gpu::kernels::read(b, k) };
    let (threads, tgs) = ((1u64 << 20) >> shift(), 256u64);
    let stats_k = dev.kernel("stats", None, None);
    println!("RUNS and LINE on the GPU, {} values (best of 6, warm):", count(n as u64));
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
        let bv = rerail_gpu::kernels::buffer_of(col).unwrap();
        // The choice in one read: the range, the runs and the line's residuals, folded from threadgroup partials.
        let parts = (threads / tgs) as usize;
        let part = words(parts as u64 * 5);
        let (v0, vn) = (col[0], col[n as usize - 1]);
        let slope = (((vn as i128 - v0 as i128) << 16) / (n as i128 - 1)) as i64;
        let (sh, sl) = ((slope >> 32) as i32, slope as u32);
        let p = P { n, v0, sh: sh as u32, sl, t: threads as u32, ..Default::default() };
        let t_stats = dev.best(&|| (), &[(stats_k, &[("v", &bv), ("part", &part)], p, threads / tgs, tgs)]);
        let q: Vec<i32> = read(&part, parts * 5).into_iter().map(|w| w as i32).collect();
        let (lo, hi, nruns, rlo, rhi) = q.chunks(5).fold((u32::MAX, 0u32, 0u64, i32::MAX, i32::MIN), |(a, b, r, c, d), q| {
            (a.min(q[0] as u32), b.max(q[1] as u32), r + q[2] as u64, c.min(q[3]), d.max(q[4]))
        });
        // The residuals are exact when the range is under 2^31; past it, LINE is not tried here.
        let line_ok = hi - lo < 1 << 31;
        let (rlo, rhi) = (rlo as i64, rhi as i64);
        // The same choice as the CPU's, by exact size.
        let bits_for = |r: u64| 64 - r.leading_zeros();
        let size = |k: u64, b: u32| (k * b as u64).div_ceil(64) + 1;
        let (bits, rbits, lbits) = (bits_for((hi - lo) as u64), bits_for(n as u64), bits_for((rhi - rlo) as u64));
        let sizes = [size(n as u64, bits), size(nruns, bits) + size(nruns, rbits), if line_ok { size(n as u64, lbits) } else { u64::MAX }];
        let tag = if sizes[1] <= sizes[0].min(sizes[2]) { col::RUNS } else if sizes[2] < sizes[0] { col::LINE } else { col::PACK };
        assert_eq!(tag, want.tag, "{what}: the GPU chooses as the CPU does");
        let t_write;
        let got: (Vec<u32>, Vec<u32>);
        if tag == col::RUNS {
            assert_eq!((lo as i64, bits, rbits, nruns as u32), (want.base, want.bits, want.rbits, want.nruns), "{what}: the choice");
            let m = (n as u64).div_ceil(32 * tgs);
            let offs = words(m);
            let (dw, rw) = ((nruns * bits as u64).div_ceil(32) + 2, (nruns * rbits as u64).div_ceil(32) + 2);
            let (bd, br) = (words(dw), words(rw));
            let (ck, sk, wk) = (dev.kernel("run_counts", None, None), dev.kernel("scan_counts", None, None), dev.kernel("run_write", None, None));
            let p = P { n, lo, bits, rbits, m: m as u32, ..Default::default() };
            // The run words are ORed in: zeroed before each run (on the host, untimed).
            t_write = dev.best(&|| {
                zero(&bd, dw);
                zero(&br, rw);
            }, &[
                (ck, &[("v", &bv), ("counts", &offs)], p, m, tgs),
                (sk, &[("counts", &offs)], p, 1, tgs),
                (wk, &[("v", &bv), ("offs", &offs), ("data", &bd), ("runs", &br)], p, m, tgs),
            ]);
            got = (read(&bd, (nruns * bits as u64).div_ceil(32) as usize), read(&br, (nruns * rbits as u64).div_ceil(32) as usize));
        } else {
            assert_eq!(tag, col::LINE);
            assert_eq!((v0 as i64, slope, rlo, lbits), (want.base, want.slope, want.rmin, want.bits), "{what}: the choice");
            let groups = (n as u64).div_ceil(32);
            let bd = words(groups * lbits as u64 + 2);
            let lk = dev.kernel("pack_line", None, Some(lbits));
            t_write = dev.best(&|| (), &[(lk, &[("v", &bv), ("w", &bd)], P { rmin: rlo as i32 as u32, ..p }, groups / tgs, tgs)]);
            let k = (n as u64 * lbits as u64).div_ceil(32) as usize;
            got = (read(&bd, k), Vec::new());
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
