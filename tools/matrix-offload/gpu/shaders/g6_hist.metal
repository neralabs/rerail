// G6: segmented histogram. codes: u16 < 1024; segment s covers [off[s], off[s+1]); H[s*1024 + code] = count.
#include <metal_stdlib>
using namespace metal;

// One threadgroup per segment; 4 KB threadgroup atomic histogram.
kernel void hist_tg(device const ushort* codes [[buffer(0)]], device const uint* off [[buffer(1)]],
                    device uint* H [[buffer(2)]], uint seg [[threadgroup_position_in_grid]],
                    uint t [[thread_index_in_threadgroup]], uint nt [[threads_per_threadgroup]]) {
  threadgroup atomic_uint h[1024];
  for (uint i = t; i < 1024; i += nt) atomic_store_explicit(&h[i], 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint s0 = off[seg], s1 = off[seg + 1];
  for (uint e = s0 + t; e < s1; e += nt) atomic_fetch_add_explicit(&h[codes[e]], 1u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  for (uint i = t; i < 1024; i += nt) H[(ulong)seg * 1024 + i] = atomic_load_explicit(&h[i], memory_order_relaxed);
}

#ifndef SPT
#define SPT 4
#endif
// One SIMD-group per segment, SPT segments per threadgroup, each with its own 4 KB histogram.
kernel void hist_sg(device const ushort* codes [[buffer(0)]], device const uint* off [[buffer(1)]],
                    device uint* H [[buffer(2)]], constant uint& nseg [[buffer(3)]],
                    uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]],
                    uint lane [[thread_index_in_simdgroup]]) {
  threadgroup atomic_uint h[SPT][1024];
  uint seg = tg * SPT + sg;
  if (seg >= nseg) return;
  for (uint i = lane; i < 1024; i += 32) atomic_store_explicit(&h[sg][i], 0u, memory_order_relaxed);
  simdgroup_barrier(mem_flags::mem_threadgroup);
  uint s0 = off[seg], s1 = off[seg + 1];
  for (uint e = s0 + lane; e < s1; e += 32) atomic_fetch_add_explicit(&h[sg][codes[e]], 1u, memory_order_relaxed);
  simdgroup_barrier(mem_flags::mem_threadgroup);
  for (uint i = lane; i < 1024; i += 32) H[(ulong)seg * 1024 + i] = atomic_load_explicit(&h[sg][i], memory_order_relaxed);
}

// Device atomics straight into H (H zeroed by a blit in the same command buffer).
kernel void hist_dev(device const ushort* codes [[buffer(0)]], device const uint* off [[buffer(1)]],
                     device atomic_uint* H [[buffer(2)]], uint seg [[threadgroup_position_in_grid]],
                     uint t [[thread_index_in_threadgroup]], uint nt [[threads_per_threadgroup]]) {
  uint s0 = off[seg], s1 = off[seg + 1];
  for (uint e = s0 + t; e < s1; e += nt)
    atomic_fetch_add_explicit(&H[(ulong)seg * 1024 + codes[e]], 1u, memory_order_relaxed);
}

