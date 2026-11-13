// Real workloads W1..W5 from the data-layer corpus (13,088 Python documents).
// Offsets in the corpus files are u64 little-endian; kernels read their low 32-bit halves
// (`off2[2 * i]`), which is exact because every offset is < 2^32 and keeps 4-byte buffer offsets legal.
#include <metal_stdlib>
using namespace metal;

// Exclusive scan across the threadgroup (threads a multiple of 32). Every thread must call it.
// sh needs 33 uints. Returns the exclusive prefix; `total` gets the threadgroup sum.
inline uint tg_exscan(uint v, threadgroup uint* sh, uint lane, uint sg, uint nsg, thread uint& total) {
  uint x = simd_prefix_exclusive_sum(v);
  if (lane == 31) sh[sg] = x + v;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if (sg == 0) {
    uint s = lane < nsg ? sh[lane] : 0;
    uint ps = simd_prefix_exclusive_sum(s);
    if (lane < nsg) sh[lane] = ps;
    if (lane == 31) sh[32] = ps + s;
  }
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint r = x + sh[sg];
  total = sh[32];
  threadgroup_barrier(mem_flags::mem_threadgroup);
  return r;
}

// ---------------------------------------------------------------------------------------------
// W1 / W5: bit gram C[i][j] = popcount(A_i & B_j) over u64 words. 64x64 output tile, 256 threads,
// 4x4 outputs per thread, 8 words staged per step (2 x 4.2 KB threadgroup memory). Any n, m, kw.
// stats[0] += number of nonzero outputs, stats[1] += sum of outputs (only cells written).
// UPPER = 1: only j > i (tiles wholly below the diagonal exit at once).
#ifndef OUT_T
#define OUT_T uint
#endif
#ifndef UPPER
#define UPPER 0
#endif
kernel void bitgram(device const ulong* A [[buffer(0)]], device const ulong* B [[buffer(1)]],
                    device OUT_T* C [[buffer(2)]], device atomic_uint* stats [[buffer(3)]],
                    constant uint& n [[buffer(4)]], constant uint& m [[buffer(5)]], constant uint& kw [[buffer(6)]],
                    uint2 tg [[threadgroup_position_in_grid]], uint2 tp [[thread_position_in_threadgroup]],
                    uint t [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]]) {
  if (UPPER && tg.x < tg.y) return;
  threadgroup ulong As[8][65];
  threadgroup ulong Bs[8][65];
  const uint row0 = tg.y * 64, col0 = tg.x * 64;
  uint acc[4][4];
  for (uint r = 0; r < 4; r++) for (uint c = 0; c < 4; c++) acc[r][c] = 0;
  const uint lr = t / 4, lw = (t % 4) * 2;
  for (uint k0 = 0; k0 < kw; k0 += 8) {
    for (uint q = 0; q < 2; q++) {
      uint gk = k0 + lw + q;
      uint ga = row0 + lr, gb = col0 + lr;
      As[lw + q][lr] = (ga < n && gk < kw) ? A[ga * kw + gk] : 0ul;
      Bs[lw + q][lr] = (gb < m && gk < kw) ? B[gb * kw + gk] : 0ul;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint w = 0; w < 8; w++) {
      ulong a[4], b[4];
      for (uint r = 0; r < 4; r++) a[r] = As[w][tp.y + 16 * r];
      for (uint c = 0; c < 4; c++) b[c] = Bs[w][tp.x + 16 * c];
      for (uint r = 0; r < 4; r++) for (uint c = 0; c < 4; c++) acc[r][c] += uint(popcount(a[r] & b[c]));
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
  }
  uint nz = 0, sum = 0;
  for (uint r = 0; r < 4; r++) for (uint c = 0; c < 4; c++) {
    uint i = row0 + tp.y + 16 * r, j = col0 + tp.x + 16 * c;
    if (i < n && j < m && (!UPPER || j > i)) {
      C[i * m + j] = OUT_T(acc[r][c]);
      nz += acc[r][c] != 0;
      sum += acc[r][c];
    }
  }
  nz = simd_sum(nz); sum = simd_sum(sum);
  if (lane == 0) {
    atomic_fetch_add_explicit(&stats[0], nz, memory_order_relaxed);
    atomic_fetch_add_explicit(&stats[1], sum, memory_order_relaxed);
  }
}

// ---------------------------------------------------------------------------------------------
// W2: within-document equality join count between reads and bindings (local name ids < NID).
#define NID 2764

kernel void zero_u32(device uint* p [[buffer(0)]], constant uint& n [[buffer(1)]], uint i [[thread_position_in_grid]]) {
  if (i < n) p[i] = 0;
}

// (b) direct-indexed: one threadgroup per document, an NID-entry count table in threadgroup memory (11 KB).
kernel void w2_direct(device const ushort* R [[buffer(0)]], device const ushort* Bd [[buffer(1)]],
                      device const uint* roff2 [[buffer(2)]], device const uint* boff2 [[buffer(3)]],
                      device uint* out [[buffer(4)]], uint doc [[threadgroup_position_in_grid]],
                      uint t [[thread_index_in_threadgroup]], uint nt [[threads_per_threadgroup]],
                      uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]],
                      uint nsg [[simdgroups_per_threadgroup]]) {
  threadgroup atomic_uint T[NID];
  threadgroup uint part[32];
  for (uint i = t; i < NID; i += nt) atomic_store_explicit(&T[i], 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint b0 = boff2[2 * doc], b1 = boff2[2 * doc + 2];
  for (uint e = b0 + t; e < b1; e += nt) atomic_fetch_add_explicit(&T[Bd[e]], 1u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint r0 = roff2[2 * doc], r1 = roff2[2 * doc + 2];
  uint c = 0;
  for (uint e = r0 + t; e < r1; e += nt) c += atomic_load_explicit(&T[R[e]], memory_order_relaxed);
  c = simd_sum(c);
  if (lane == 0) part[sg] = c;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if (t == 0) { uint s = 0; for (uint i = 0; i < nsg; i++) s += part[i]; out[doc] = s; }
}

// (a) brute-force tiles: tile = (doc, first read, first binding) covering up to 1024 reads x 512 bindings.
// 256 threads hold 4 reads each in registers; the 512 binding ids are staged in threadgroup memory.
// Invalid read slots hold 0xFFFF, which never equals an id (< NID). out[doc] accumulates atomically.
kernel void w2_brute(device const ushort* R [[buffer(0)]], device const ushort* Bd [[buffer(1)]],
                     device const uint* roff2 [[buffer(2)]], device const uint* boff2 [[buffer(3)]],
                     device const uint4* tiles [[buffer(4)]], device atomic_uint* out [[buffer(5)]],
                     uint tile [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                     uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {
  threadgroup ushort sb[512];
  threadgroup uint part[8];
  uint4 T = tiles[tile];
  uint doc = T.x, r0 = T.y, b0 = T.z;
  uint rEnd = min(roff2[2 * doc + 2], r0 + 1024), bEnd = min(boff2[2 * doc + 2], b0 + 512);
  uint nb = bEnd - b0;
  for (uint i = t; i < nb; i += 256) sb[i] = Bd[b0 + i];
  threadgroup_barrier(mem_flags::mem_threadgroup);
  ushort ra[4];
  for (uint q = 0; q < 4; q++) { uint idx = r0 + t + 256 * q; ra[q] = idx < rEnd ? R[idx] : ushort(0xFFFF); }
  uint c = 0;
  for (uint j = 0; j < nb; j++) {
    ushort b = sb[j];
    c += uint(ra[0] == b) + uint(ra[1] == b) + uint(ra[2] == b) + uint(ra[3] == b);
  }
  c = simd_sum(c);
  if (lane == 0) part[sg] = c;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if (t == 0) {
    uint s = 0; for (uint i = 0; i < 8; i++) s += part[i];
    if (s) atomic_fetch_add_explicit(&out[doc], s, memory_order_relaxed);
  }
}

// (c) materialise rows: exclusive scan of the per-document counts (single threadgroup of 1024).
kernel void scan_single(device const uint* v [[buffer(0)]], device uint* ex [[buffer(1)]], constant uint& n [[buffer(2)]],
                        uint t [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]],
                        uint sg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]]) {
  threadgroup uint sh[33];
  uint ch = (n + 1023) / 1024, a = min(t * ch, n), b = min(a + ch, n);
  uint s = 0;
  for (uint i = a; i < b; i++) s += v[i];
  uint total;
  uint base = tg_exscan(s, sh, lane, sg, nsg, total);
  for (uint i = a; i < b; i++) { ex[i] = base; base += v[i]; }
  if (t == 0) ex[n] = total;
}

// (c) emit (read index, binding index) rows, both local to the document, at docOff[doc].
// Count bindings per id in T, scan T in place to starts, scatter binding indices with an atomic cursor
// (afterwards T[id] = end of id = start of id+1), then each read writes its run of matches.
// SB is a device scratch of binding indices grouped by id (one entry per binding).
kernel void w2_emit(device const ushort* R [[buffer(0)]], device const ushort* Bd [[buffer(1)]],
                    device const uint* roff2 [[buffer(2)]], device const uint* boff2 [[buffer(3)]],
                    device const uint* docOff [[buffer(4)]], device ushort* SB [[buffer(5)]],
                    device uint2* rows [[buffer(6)]], uint doc [[threadgroup_position_in_grid]],
                    uint t [[thread_index_in_threadgroup]], uint nt [[threads_per_threadgroup]],
                    uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]],
                    uint nsg [[simdgroups_per_threadgroup]]) {
  threadgroup atomic_uint T[NID];
  threadgroup uint sh[33];
  for (uint i = t; i < NID; i += nt) atomic_store_explicit(&T[i], 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint b0 = boff2[2 * doc], b1 = boff2[2 * doc + 2];
  for (uint e = b0 + t; e < b1; e += nt) atomic_fetch_add_explicit(&T[Bd[e]], 1u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  // in-place exclusive scan of T: thread t owns a contiguous chunk
  uint ch = (NID + nt - 1) / nt, a = min(t * ch, uint(NID)), z = min(a + ch, uint(NID));
  uint s = 0;
  for (uint i = a; i < z; i++) s += atomic_load_explicit(&T[i], memory_order_relaxed);
  uint tot;
  uint base = tg_exscan(s, sh, lane, sg, nsg, tot);
  for (uint i = a; i < z; i++) { uint c = atomic_load_explicit(&T[i], memory_order_relaxed); atomic_store_explicit(&T[i], base, memory_order_relaxed); base += c; }
  threadgroup_barrier(mem_flags::mem_threadgroup);
  for (uint e = b0 + t; e < b1; e += nt) {
    uint pos = atomic_fetch_add_explicit(&T[Bd[e]], 1u, memory_order_relaxed);
    SB[b0 + pos] = ushort(e - b0);
  }
  threadgroup_barrier(mem_flags::mem_threadgroup | mem_flags::mem_device);
  uint r0 = roff2[2 * doc], r1 = roff2[2 * doc + 2];
  uint carry = docOff[doc];
  for (uint cb = r0; cb < r1; cb += nt) {   // uniform trip count across the threadgroup
    uint e = cb + t, cnt = 0, st = 0;
    if (e < r1) {
      uint id = R[e];
      uint en = atomic_load_explicit(&T[id], memory_order_relaxed);
      st = id ? atomic_load_explicit(&T[id - 1], memory_order_relaxed) : 0u;
      cnt = en - st;
    }
    uint chunkTotal;
    uint off = tg_exscan(cnt, sh, lane, sg, nsg, chunkTotal) + carry;
    for (uint k = 0; k < cnt; k++) rows[off + k] = uint2(e - r0, uint(SB[b0 + st + k]));
    carry += chunkTotal;
  }
}

// ---------------------------------------------------------------------------------------------
// W3: reachability closure over the condensed DAG, rows of kw u64 words (kw <= 192, i.e. 6 per lane).

// OR into acc (6 words per lane) every row k of M whose bit is set in the row held by the SIMD-group as
// `mine` (6 words per lane: word lane + 32 q). Set bits are walked SIMD-wide: the lowest lane that still has
// a bit is elected, its bit broadcast, and all lanes OR that row. Returns the number of rows OR'ed.
// Bounded: at most 64 * 32 iterations per q.
inline uint or_rows(thread ulong* mine, thread ulong* acc, device const ulong* M, uint kw, uint lane) {
  // Bits are taken BATCH at a time and their rows loaded together, so the loads overlap instead of
  // each OR waiting on one row's latency (the critical path of a high-out-degree row).
  constexpr uint BATCH = 4;  // 8 measured 3-4x slower (register pressure)
  uint rows = 0;
  for (uint q = 0; q < 6; q++) {
    ulong word = mine[q];
    for (uint it = 0; it < 64 * 32; it++) {
      uint ks[BATCH], nk = 0;
      for (uint j = 0; j < BATCH; j++) {
        uint src = simd_min(word != 0 ? lane : 32u);
        if (src == 32) break;
        uint2 w2 = simd_shuffle(as_type<uint2>(word), ushort(src));  // ulong is not a valid simd_shuffle type
        ks[nk++] = ((src + 32 * q) * 64) + ctz(as_type<ulong>(w2));
        if (lane == src) word &= word - 1;
      }
      if (nk == 0) break;
      ulong v[BATCH][6];
      for (uint j = 0; j < BATCH; j++) for (uint x = 0; x < 6; x++) {
        uint c = lane + 32 * x;
        v[j][x] = (j < nk && c < kw) ? M[(ulong)ks[j] * kw + c] : 0ul;
      }
      for (uint j = 0; j < BATCH; j++) for (uint x = 0; x < 6; x++) acc[x] |= v[j][x];
      rows += nk;
      if (nk < BATCH) break;
    }
  }
  return rows;
}

// (a) one squaring step, row-OR form: Mn[i] = M[i] | OR_{k in M[i]} M[k]. One SIMD-group per row.
// flags[step] = 1 if any row changed. If flags[step-1] == 0 (already a fixpoint) the step just copies.
kernel void w3_square(device const ulong* M [[buffer(0)]], device ulong* Mn [[buffer(1)]],
                      device atomic_uint* flags [[buffer(2)]], constant uint& nr [[buffer(3)]],
                      constant uint& kw [[buffer(4)]], constant uint& step [[buffer(5)]],
                      uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]],
                      uint nsg [[simdgroups_per_threadgroup]], uint lane [[thread_index_in_simdgroup]]) {
  uint row = tg * nsg + sg;
  if (row >= nr) return;
  device const ulong* mr = M + (ulong)row * kw;
  ulong acc[6], orig[6];
  for (uint q = 0; q < 6; q++) { uint w = lane + 32 * q; orig[q] = w < kw ? mr[w] : 0ul; acc[q] = orig[q]; }
  bool converged = step > 0 && atomic_load_explicit(&flags[step - 1], memory_order_relaxed) == 0;
  uint rows = converged ? 0 : or_rows(orig, acc, M, kw, lane);
  if (lane == 0 && rows) atomic_fetch_add_explicit(&flags[16 + step], rows, memory_order_relaxed);
  bool changed = false;
  for (uint q = 0; q < 6; q++) { uint w = lane + 32 * q; if (w < kw) { Mn[(ulong)row * kw + w] = acc[q]; changed |= acc[q] != orig[q]; } }
  if (simd_any(changed) && lane == 0) atomic_store_explicit(&flags[step], 1u, memory_order_relaxed);
}

// (b) level-wise: rows of one level (successors are all at lower levels, already final).
// Rch[c] = A[c] | OR_{s in A[c]} Rch[s]. One SIMD-group per row; order[] lists rows level by level.
kernel void w3_level(device const ulong* A [[buffer(0)]], device ulong* Rch [[buffer(1)]],
                     device const uint* order [[buffer(2)]], constant uint& lo [[buffer(3)]],
                     constant uint& hi [[buffer(4)]], constant uint& kw [[buffer(5)]],
                     uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]],
                     uint nsg [[simdgroups_per_threadgroup]], uint lane [[thread_index_in_simdgroup]]) {
  uint idx = lo + tg * nsg + sg;
  if (idx >= hi) return;
  uint row = order[idx];
  device const ulong* ar = A + (ulong)row * kw;
  ulong acc[6];
  ulong mine[6];
  for (uint q = 0; q < 6; q++) { uint w = lane + 32 * q; mine[q] = w < kw ? ar[w] : 0ul; acc[q] = mine[q]; }
  or_rows(mine, acc, Rch, kw, lane);
  for (uint q = 0; q < 6; q++) { uint w = lane + 32 * q; if (w < kw) Rch[(ulong)row * kw + w] = acc[q]; }
}

// Document-level reach pairs from the closure: per component row c,
// size(c) * (sum of size(d) over reachable d + (size(c) if size(c) > 1)). out: one u64 (as two u32) per row.
kernel void w3_pairs(device const ulong* Rch [[buffer(0)]], device const uint* sizes [[buffer(1)]],
                     device ulong* out [[buffer(2)]], constant uint& nr [[buffer(3)]], constant uint& kw [[buffer(4)]],
                     uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]],
                     uint nsg [[simdgroups_per_threadgroup]], uint lane [[thread_index_in_simdgroup]]) {
  uint row = tg * nsg + sg;
  if (row >= nr) return;
  uint s = 0;
  for (uint w = lane; w < kw; w += 32) {
    ulong word = Rch[(ulong)row * kw + w];
    for (uint b = 0; b < 64 && word != 0; b++) { s += sizes[w * 64 + ctz(word)]; word &= word - 1; }
  }
  s = simd_sum(s);
  if (lane == 0) { uint sc = sizes[row]; out[row] = (ulong)sc * (ulong)(s + (sc > 1 ? sc : 0)); }
}

// ---------------------------------------------------------------------------------------------
// W4: kind histogram per document (NK kinds, padded to 128 bins). One threadgroup per document.
kernel void w4_hist(device const ushort* K [[buffer(0)]], device const uint* off2 [[buffer(1)]],
                    device uint* H [[buffer(2)]], constant uint& nk [[buffer(3)]],
                    uint doc [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                    uint nt [[threads_per_threadgroup]]) {
  threadgroup atomic_uint h[128];
  for (uint i = t; i < 128; i += nt) atomic_store_explicit(&h[i], 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint a = off2[2 * doc], b = off2[2 * doc + 2];
  for (uint e = a + t; e < b; e += nt) atomic_fetch_add_explicit(&h[K[e]], 1u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  for (uint i = t; i < nk; i += nt) H[doc * nk + i] = atomic_load_explicit(&h[i], memory_order_relaxed);
}

// Same, with one sub-histogram per SIMD-group (less contention on the few dominant kinds), summed at the end.
kernel void w4_hist_sg(device const ushort* K [[buffer(0)]], device const uint* off2 [[buffer(1)]],
                       device uint* H [[buffer(2)]], constant uint& nk [[buffer(3)]],
                       uint doc [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                       uint nt [[threads_per_threadgroup]], uint sg [[simdgroup_index_in_threadgroup]],
                       uint nsg [[simdgroups_per_threadgroup]]) {
  threadgroup atomic_uint h[8][128];
  for (uint i = t; i < 8 * 128; i += nt) atomic_store_explicit(&h[i / 128][i % 128], 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint a = off2[2 * doc], b = off2[2 * doc + 2];
  for (uint e = a + t; e < b; e += nt) atomic_fetch_add_explicit(&h[sg][K[e]], 1u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  for (uint i = t; i < nk; i += nt) {
    uint s = 0;
    for (uint g = 0; g < nsg; g++) s += atomic_load_explicit(&h[g][i], memory_order_relaxed);
    H[doc * nk + i] = s;
  }
}
