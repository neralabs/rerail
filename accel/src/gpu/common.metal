// Shared pieces of the gpu lane's kernels: the threadgroup scan, the device-wide scan's two passes, a zero fill.
//
// Every 1-D kernel runs threadgroups of TG = 256 threads (8 SIMD-groups); the host dispatches whole threadgroups
// and each kernel checks its bounds. Every loop's trip count follows from its inputs.
#include <metal_stdlib>
using namespace metal;

constant uint TG = 256;
// Elements per threadgroup of the device-wide scan: 4 per thread.
constant uint SCAN_BLOCK = 1024;
// Counters in a threadgroup table (count table, histogram): 8 KB of u32, within the ~10 KB budget.
constant uint TABLE = 2048;

// Exclusive scan across the threadgroup (threads a multiple of 32, at most 1024). Every thread must call it.
// `sh` holds 33 uints. Returns the exclusive prefix; `total` gets the threadgroup's sum.
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

// The device-wide exclusive scan, first pass: block `b` scans its SCAN_BLOCK elements of `in` (their popcounts
// when `pop`) into `out`, and writes its sum to `sums[b]`.
kernel void scan_block(device const uint* in [[buffer(0)]], device uint* out [[buffer(1)]],
                       device uint* sums [[buffer(2)]], constant uint& n [[buffer(3)]],
                       constant uint& pop [[buffer(4)]], uint b [[threadgroup_position_in_grid]],
                       uint t [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]],
                       uint sg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]]) {
  threadgroup uint sh[33];
  uint base = b * SCAN_BLOCK + t * 4;
  uint v[4];
  uint s = 0;
  for (uint q = 0; q < 4; q++) {
    uint i = base + q;
    uint x = i < n ? in[i] : 0u;
    if (pop) x = popcount(x);
    v[q] = x;
    s += x;
  }
  uint total;
  uint p = tg_exscan(s, sh, lane, sg, nsg, total);
  for (uint q = 0; q < 4; q++) {
    uint i = base + q;
    if (i < n) out[i] = p;
    p += v[q];
  }
  if (t == 0) sums[b] = total;
}

// Second pass: each element adds its block's prefix (the scanned block sums).
kernel void scan_add(device uint* out [[buffer(0)]], device const uint* prefix [[buffer(1)]],
                     constant uint& n [[buffer(2)]], uint i [[thread_position_in_grid]]) {
  if (i < n) out[i] += prefix[i / SCAN_BLOCK];
}

kernel void zero_u32(device uint* p [[buffer(0)]], constant uint& n [[buffer(1)]], uint i [[thread_position_in_grid]]) {
  if (i < n) p[i] = 0;
}
