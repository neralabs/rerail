// Closure over the condensed graph (the spikes' W3 (b), level by level). The host finds the components (Tarjan's,
// as the reference does), their DAG as a CSR of successor components, and each component's level (1 + the
// highest level of its successors). On the device, R holds one bit row per component, `words` u64 words wide:
//
// 1. cl_init: each node sets, in its component's row, its own bit when the component is cyclic, and the bit of
//    every successor in another component (device atomics on u32 halves: the result is the same in any order);
// 2. cl_level, once per level from 1 up: each component ORs in its successors' rows, which are final (lower levels);
// 3. cl_gather: node v's row is its component's.

kernel void cl_init(device const uint* off [[buffer(0)]], device const uint* dst [[buffer(1)]],
                    device const uint* comp [[buffer(2)]], device const uint* cyclic [[buffer(3)]],
                    device atomic_uint* R [[buffer(4)]], constant uint& n [[buffer(5)]],
                    constant uint& words [[buffer(6)]], uint v [[thread_position_in_grid]]) {
  if (v >= n) return;
  const uint c = comp[v];
  device atomic_uint* row = R + (ulong)c * words * 2;
  if (cyclic[c]) atomic_fetch_or_explicit(&row[v / 32], 1u << (v % 32), memory_order_relaxed);
  for (uint e = off[v]; e < off[v + 1]; e++) {
    const uint w = dst[e];
    if (comp[w] != c) atomic_fetch_or_explicit(&row[w / 32], 1u << (w % 32), memory_order_relaxed);
  }
}

// One SIMD-group per component of the level; lanes stride over the row's words.
kernel void cl_level(device ulong* R [[buffer(0)]], device const uint* order [[buffer(1)]],
                     device const uint* soff [[buffer(2)]], device const uint* sdst [[buffer(3)]],
                     constant uint& lo [[buffer(4)]], constant uint& hi [[buffer(5)]],
                     constant uint& words [[buffer(6)]], uint tg [[threadgroup_position_in_grid]],
                     uint sg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]],
                     uint lane [[thread_index_in_simdgroup]]) {
  const uint idx = lo + tg * nsg + sg;
  if (idx >= hi) return;
  const uint c = order[idx];
  const uint s0 = soff[c], s1 = soff[c + 1];
  device ulong* row = R + (ulong)c * words;
  for (uint w = lane; w < words; w += 32) {
    ulong acc = row[w];
    for (uint k = s0; k < s1; k++) acc |= R[(ulong)sdst[k] * words + w];
    row[w] = acc;
  }
}

kernel void cl_gather(device const ulong* R [[buffer(0)]], device const uint* comp [[buffer(1)]],
                      device ulong* out [[buffer(2)]], constant uint& n [[buffer(3)]],
                      constant uint& words [[buffer(4)]], uint i [[thread_position_in_grid]]) {
  if (i >= n * words) return;
  const uint v = i / words, w = i % words;
  out[i] = R[(ulong)comp[v] * words + w];
}
