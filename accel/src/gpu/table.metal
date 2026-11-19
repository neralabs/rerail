// Count table and histogram: one threadgroup per partition, a TABLE-entry counter table in threadgroup memory
// (8 KB; the spikes' W2 (b) and W4).
//
// Codes are 16-bit, so a table of TABLE counters covers a window of codes. A partition runs one pass per window up
// to its largest code (one pass while codes stay below TABLE, as the engine's local ids do; at most 32). Each pass
// reads the partition's rows again, so the cost grows with the code range, never the result.

// The largest of `col[a..b]` (0 when empty), through the threadgroup. Every thread must call it.
inline uint tg_max(device const ushort* col, uint a, uint b, threadgroup atomic_uint* mx, uint t) {
  if (t == 0) atomic_store_explicit(mx, 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint m = 0;
  for (uint e = a + t; e < b; e += TG) m = max(m, uint(col[e]));
  m = simd_max(m);
  if (simd_is_first()) atomic_fetch_max_explicit(mx, m, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  uint r = atomic_load_explicit(mx, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);
  return r;
}

// Per left row of partition p, the right rows of p with its code. Every left row is written exactly once: in the
// pass of its code's window, or with 0 when its code is above the partition's right codes.
kernel void ct_count(device const ushort* left [[buffer(0)]], device const ushort* right [[buffer(1)]],
                     device const uint* loff [[buffer(2)]], device const uint* roff [[buffer(3)]],
                     device uint* counts [[buffer(4)]], uint p [[threadgroup_position_in_grid]],
                     uint t [[thread_index_in_threadgroup]]) {
  threadgroup atomic_uint T[TABLE];
  threadgroup atomic_uint mx;
  const uint l0 = loff[p], l1 = loff[p + 1], r0 = roff[p], r1 = roff[p + 1];
  if (l0 == l1) return;  // uniform across the threadgroup
  const uint windows = r1 > r0 ? tg_max(right, r0, r1, &mx, t) / TABLE + 1 : 0u;
  for (uint e = l0 + t; e < l1; e += TG) if (uint(left[e]) / TABLE >= windows) counts[e] = 0;
  for (uint w = 0; w < windows; w++) {
    const uint base = w * TABLE;
    for (uint i = t; i < TABLE; i += TG) atomic_store_explicit(&T[i], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint e = r0 + t; e < r1; e += TG) {
      uint c = uint(right[e]) - base;
      if (c < TABLE) atomic_fetch_add_explicit(&T[c], 1u, memory_order_relaxed);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint e = l0 + t; e < l1; e += TG) {
      uint c = uint(left[e]) - base;
      if (c < TABLE) counts[e] = atomic_load_explicit(&T[c], memory_order_relaxed);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
  }
}

// The pairs: each left row with matches walks its partition's right rows in order and writes the equal ones at its
// scanned offset `ex[li]`. So pairs are in left order, then right order, as the reference's. The right codes are
// staged through threadgroup memory, TABLE at a time (4 KB). Writes past `cap` are dropped (the host reruns the pass
// into a buffer of the exact size).
kernel void ct_scatter(device const ushort* left [[buffer(0)]], device const ushort* right [[buffer(1)]],
                       device const uint* loff [[buffer(2)]], device const uint* roff [[buffer(3)]],
                       device const uint* counts [[buffer(4)]], device const uint* ex [[buffer(5)]],
                       device uint2* pairs [[buffer(6)]], constant uint& cap [[buffer(7)]],
                       uint p [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {
  threadgroup ushort R[TABLE];
  threadgroup atomic_uint any;
  const uint l0 = loff[p], l1 = loff[p + 1], r0 = roff[p], r1 = roff[p + 1];
  if (l0 == l1 || r0 == r1) return;  // uniform: no pairs
  for (uint lb = l0; lb < l1; lb += TG) {  // the same trip count for every thread
    const uint li = lb + t;
    const uint want = li < l1 ? counts[li] : 0u;
    if (t == 0) atomic_store_explicit(&any, 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (simd_any(want != 0) && simd_is_first()) atomic_store_explicit(&any, 1u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    const bool skip = atomic_load_explicit(&any, memory_order_relaxed) == 0;
    threadgroup_barrier(mem_flags::mem_threadgroup);  // every thread has read `any` before the next round resets it
    if (skip) continue;  // uniform
    const ushort c = li < l1 ? left[li] : ushort(0);
    uint o = want != 0 ? ex[li] : 0u, got = 0;
    for (uint rb = r0; rb < r1; rb += TABLE) {
      const uint len = min(TABLE, r1 - rb);
      threadgroup_barrier(mem_flags::mem_threadgroup);
      for (uint i = t; i < len; i += TG) R[i] = right[rb + i];
      threadgroup_barrier(mem_flags::mem_threadgroup);
      if (got < want) {
        for (uint i = 0; i < len; i++) {
          if (R[i] == c) {
            if (o < cap) pairs[o] = uint2(li, rb + i);
            o++;
            got++;
          }
        }
      }
    }
  }
}

// Histogram: counts of each code below `width` per partition, H[p * width + c]. A code at or above `width` sets
// `bad` (the input is invalid; the host reports it).
kernel void hist(device const ushort* codes [[buffer(0)]], device const uint* off [[buffer(1)]],
                 device uint* H [[buffer(2)]], constant uint& width [[buffer(3)]],
                 device atomic_uint* bad [[buffer(4)]], uint p [[threadgroup_position_in_grid]],
                 uint t [[thread_index_in_threadgroup]]) {
  threadgroup atomic_uint h[TABLE];
  const uint a = off[p], b = off[p + 1];
  const uint windows = (width + TABLE - 1) / TABLE;
  for (uint w = 0; w < windows; w++) {
    const uint base = w * TABLE, len = min(TABLE, width - base);
    for (uint i = t; i < len; i += TG) atomic_store_explicit(&h[i], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint e = a + t; e < b; e += TG) {
      uint code = uint(codes[e]);
      uint c = code - base;
      if (c < len) atomic_fetch_add_explicit(&h[c], 1u, memory_order_relaxed);
      else if (w == 0 && code >= width) atomic_store_explicit(bad, 1u, memory_order_relaxed);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    device uint* out = H + (ulong)p * width + base;
    for (uint i = t; i < len; i += TG) out[i] = atomic_load_explicit(&h[i], memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
  }
}
