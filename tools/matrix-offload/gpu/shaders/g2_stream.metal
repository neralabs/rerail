// G2: DRAM bandwidth. Coalesced uint4 accesses: thread g touches g, g+G, g+2G, ... (G = grid size).
#include <metal_stdlib>
using namespace metal;

// Wrapping uint sum of the whole buffer; one partial per threadgroup (host adds the partials).
kernel void reduce_k(device const uint4* in [[buffer(0)]], device uint* partial [[buffer(1)]],
                     constant uint& n4 [[buffer(2)]], constant uint& iters [[buffer(3)]],
                     uint gid [[thread_position_in_grid]], uint gsz [[threads_per_grid]],
                     uint tg [[threadgroup_position_in_grid]], uint lane [[thread_index_in_simdgroup]],
                     uint sg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]]) {
  uint4 acc = 0;
  for (uint i = 0; i < iters; i++) {
    uint idx = gid + i * gsz;
    if (idx < n4) acc += in[idx];
  }
  uint s = simd_sum(acc.x + acc.y + acc.z + acc.w);
  threadgroup uint sh[32];
  if (lane == 0) sh[sg] = s;
  threadgroup_barrier(mem_flags::mem_threadgroup);
  if (sg == 0) {
    uint v = lane < nsg ? sh[lane] : 0;
    v = simd_sum(v);
    if (lane == 0) partial[tg] = v;
  }
}

kernel void copy_k(device const uint4* in [[buffer(0)]], device uint4* out [[buffer(1)]],
                   constant uint& n4 [[buffer(2)]], constant uint& iters [[buffer(3)]],
                   uint gid [[thread_position_in_grid]], uint gsz [[threads_per_grid]]) {
  for (uint i = 0; i < iters; i++) {
    uint idx = gid + i * gsz;
    if (idx < n4) out[idx] = in[idx];
  }
}
