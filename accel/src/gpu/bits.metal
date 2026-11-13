// Bit product: C[i][j] = popcount(A_i & B_j) over rows of `kw` u64 words (the spikes' W1/W5 bit gram).
//
// One threadgroup per 64x64 output tile, 256 threads, 4x4 outputs per thread; 8 words of each side staged per step
// (2 x 4.2 KB of threadgroup memory). The grid is (column tiles, row tiles).
//
// mode 0: every count, into C (n x m, row-major).
// mode 1: only whether count >= min[j], as bits of `mask` (n rows of `mw` u32 words, bit j of a row in word j / 32).
//         A tile's 64 columns are exactly two mask words of each of its rows, so the tile gathers them in threadgroup
//         memory (512 B) and writes them whole: no device atomics, and no zero fill beforehand.
kernel void bp_tile(device const ulong* A [[buffer(0)]], device const ulong* B [[buffer(1)]],
                    device uint* C [[buffer(2)]], device uint* mask [[buffer(3)]],
                    device const uint* minv [[buffer(4)]], constant uint& n [[buffer(5)]],
                    constant uint& m [[buffer(6)]], constant uint& kw [[buffer(7)]],
                    constant uint& mode [[buffer(8)]], constant uint& mw [[buffer(9)]],
                    uint2 tg [[threadgroup_position_in_grid]], uint2 tp [[thread_position_in_threadgroup]],
                    uint t [[thread_index_in_threadgroup]]) {
  threadgroup ulong As[8][65];
  threadgroup ulong Bs[8][65];
  threadgroup atomic_uint tmask[128];
  const uint row0 = tg.y * 64, col0 = tg.x * 64;
  if (t < 128) atomic_store_explicit(&tmask[t], 0u, memory_order_relaxed);
  threadgroup_barrier(mem_flags::mem_threadgroup);  // zeroed before any thread ORs (kw = 0 has no other barrier)
  uint acc[4][4];
  for (uint r = 0; r < 4; r++) for (uint c = 0; c < 4; c++) acc[r][c] = 0;
  const uint lr = t / 4, lw = (t % 4) * 2;
  for (uint k0 = 0; k0 < kw; k0 += 8) {
    for (uint q = 0; q < 2; q++) {
      uint gk = k0 + lw + q;
      uint ga = row0 + lr, gb = col0 + lr;
      As[lw + q][lr] = (ga < n && gk < kw) ? A[(ulong)ga * kw + gk] : 0ul;
      Bs[lw + q][lr] = (gb < m && gk < kw) ? B[(ulong)gb * kw + gk] : 0ul;
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
  for (uint r = 0; r < 4; r++) for (uint c = 0; c < 4; c++) {
    uint lrow = tp.y + 16 * r, lcol = tp.x + 16 * c;
    uint i = row0 + lrow, j = col0 + lcol;
    if (i < n && j < m) {
      if (mode == 0) {
        C[(ulong)i * m + j] = acc[r][c];
      } else if (acc[r][c] >= minv[j]) {
        atomic_fetch_or_explicit(&tmask[lrow * 2 + lcol / 32], 1u << (lcol % 32), memory_order_relaxed);
      }
    }
  }
  if (mode == 1) {
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (t < 128) {
      uint i = row0 + t / 2, w = col0 / 32 + t % 2;
      if (i < n && w < mw) mask[(ulong)i * mw + w] = atomic_load_explicit(&tmask[t], memory_order_relaxed);
    }
  }
}

// The thresholded pairs: mask word `w` writes its set bits as (row, column) at its scanned offset `ex[w]`.
// Words are row-major and bits ascend, so the pairs come out in (i, j) order. Writes past `cap` are dropped (the
// host sees the total and runs this pass again into a buffer of the exact size).
kernel void bp_scatter(device const uint* mask [[buffer(0)]], device const uint* ex [[buffer(1)]],
                       device uint2* pairs [[buffer(2)]], constant uint& cap [[buffer(3)]],
                       constant uint& mw [[buffer(4)]], constant uint& nw [[buffer(5)]],
                       uint w [[thread_position_in_grid]]) {
  if (w >= nw) return;
  uint x = mask[w];
  uint o = ex[w];
  uint i = w / mw, j0 = (w % mw) * 32;
  for (uint k = 0; k < 32 && x != 0; k++) {
    uint b = ctz(x);
    x &= x - 1;
    if (o < cap) pairs[o] = uint2(i, j0 + b);
    o++;
  }
}
