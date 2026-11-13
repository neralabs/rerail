// G5: intersection counts. Pair p: sorted unique u16 arrays A[p*L .. p*L+L) and B[p*L ..).
#include <metal_stdlib>
using namespace metal;

// One thread per pair: branchless merge, bounded by 2L steps.
kernel void isect_thread(device const ushort* A [[buffer(0)]], device const ushort* B [[buffer(1)]],
                         device uint* out [[buffer(2)]], constant uint& L [[buffer(3)]], constant uint& P [[buffer(4)]],
                         uint p [[thread_position_in_grid]]) {
  if (p >= P) return;
  device const ushort* a = A + (ulong)p * L;
  device const ushort* b = B + (ulong)p * L;
  uint i = 0, j = 0, c = 0;
  for (uint s = 0; s < 2 * L; s++) {
    if (i >= L || j >= L) break;
    ushort x = a[i], y = b[j];
    c += (x == y);
    i += (x <= y);
    j += (y <= x);
  }
  out[p] = c;
}

// One SIMD-group per pair: each lane takes A elements lane, lane+32, ... and binary-searches B.
kernel void isect_simd_bsearch(device const ushort* A [[buffer(0)]], device const ushort* B [[buffer(1)]],
                               device uint* out [[buffer(2)]], constant uint& L [[buffer(3)]], constant uint& P [[buffer(4)]],
                               uint gsg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]],
                               uint tg [[threadgroup_position_in_grid]], uint lane [[thread_index_in_simdgroup]]) {
  uint p = tg * nsg + gsg;
  if (p >= P) return;
  device const ushort* a = A + (ulong)p * L;
  device const ushort* b = B + (ulong)p * L;
  uint c = 0;
  for (uint t = lane; t < L; t += 32) {
    ushort x = a[t];
    uint base = 0, len = L;
    for (uint it = 0; it < 16 && len > 1; it++) {
      uint half_ = len / 2;
      base = (b[base + half_] <= x) ? base + half_ : base;
      len -= half_;
    }
    c += (b[base] == x);
  }
  c = simd_sum(c);
  if (lane == 0) out[p] = c;
}

// One SIMD-group per pair, merge-path style: lane owns a contiguous chunk of A, finds its start in B
// by binary search, then merges forward. Bounded by chunk + L steps.
kernel void isect_simd_merge(device const ushort* A [[buffer(0)]], device const ushort* B [[buffer(1)]],
                             device uint* out [[buffer(2)]], constant uint& L [[buffer(3)]], constant uint& P [[buffer(4)]],
                             uint gsg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]],
                             uint tg [[threadgroup_position_in_grid]], uint lane [[thread_index_in_simdgroup]]) {
  uint p = tg * nsg + gsg;
  if (p >= P) return;
  device const ushort* a = A + (ulong)p * L;
  device const ushort* b = B + (ulong)p * L;
  uint ch = (L + 31) / 32;
  uint i = min(lane * ch, L), iend = min(i + ch, L);
  uint c = 0;
  if (i < iend) {
    ushort x0 = a[i];
    uint base = 0, len = L;   // lower_bound(x0)
    for (uint it = 0; it < 16 && len > 0; it++) {
      uint half_ = len / 2;
      bool lt = b[base + half_] < x0;
      base = lt ? base + half_ + 1 : base;
      len = lt ? len - half_ - 1 : half_;
    }
    uint j = base;
    for (uint s = 0; s < ch + L; s++) {
      if (i >= iend || j >= L) break;
      ushort x = a[i], y = b[j];
      c += (x == y);
      i += (x <= y);
      j += (y <= x);
    }
  }
  c = simd_sum(c);
  if (lane == 0) out[p] = c;
}

// Bitmaps: 8 KB (2048 uint) each, pair p = (pa[p], pb[p]) into a pool. One SIMD-group per pair.
kernel void isect_bitmap(device const uint4* pool [[buffer(0)]], device const uint* pa [[buffer(1)]],
                         device const uint* pb [[buffer(2)]], device uint* out [[buffer(3)]], constant uint& P [[buffer(4)]],
                         uint gsg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]],
                         uint tg [[threadgroup_position_in_grid]], uint lane [[thread_index_in_simdgroup]]) {
  uint p = tg * nsg + gsg;
  if (p >= P) return;
  device const uint4* a = pool + (ulong)pa[p] * 512;
  device const uint4* b = pool + (ulong)pb[p] * 512;
  uint4 acc = 0;
  for (uint t = 0; t < 16; t++) acc += popcount(a[lane + 32 * t] & b[lane + 32 * t]);
  uint c = simd_sum(acc.x + acc.y + acc.z + acc.w);
  if (lane == 0) out[p] = c;
}
