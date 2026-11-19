// G3: bit-matrix product C[i][j] = popcount(A_i & B_j). A: n rows, B: m rows, kw uint words per row.
#include <metal_stdlib>
using namespace metal;

// One thread per output, operands straight from device memory (caches only).
kernel void bm_naive(device const uint4* A [[buffer(0)]], device const uint4* B [[buffer(1)]],
                     device uint* C [[buffer(2)]], constant uint& n [[buffer(3)]],
                     constant uint& m [[buffer(4)]], constant uint& kw [[buffer(5)]],
                     uint2 g [[thread_position_in_grid]]) {
  if (g.x >= m || g.y >= n) return;
  uint kq = kw / 4;
  device const uint4* a = A + g.y * kq;
  device const uint4* b = B + g.x * kq;
  uint4 acc = 0;
  for (uint w = 0; w < kq; w++) acc += popcount(a[w] & b[w]);
  C[g.y * m + g.x] = acc.x + acc.y + acc.z + acc.w;
}

// Tiled: 256 threads (16x16), each owns TM x TM outputs; tile = 16*TM rows of A and of B.
// K is staged KC words at a time in threadgroup memory, stored word-major ([w][row]).
#ifndef TM
#define TM 4
#endif
#ifndef KC
#define KC 16
#endif
#define TILE (16 * TM)
#define PAD 1
kernel void bm_tiled(device const uint4* A [[buffer(0)]], device const uint4* B [[buffer(1)]],
                     device uint* C [[buffer(2)]], constant uint& n [[buffer(3)]],
                     constant uint& m [[buffer(4)]], constant uint& kw [[buffer(5)]],
                     uint2 tg [[threadgroup_position_in_grid]], uint2 tp [[thread_position_in_threadgroup]],
                     uint t [[thread_index_in_threadgroup]]) {
  threadgroup uint As[KC][TILE + PAD];
  threadgroup uint Bs[KC][TILE + PAD];
  const uint kq = kw / 4;
  const uint row0 = tg.y * TILE, col0 = tg.x * TILE;
  constexpr uint QPR = KC / 4;             // uint4 per row per stage
  constexpr uint ROWS_PER_PASS = 256 / QPR;
  constexpr uint PASSES = TILE / ROWS_PER_PASS;
  const uint lr = t / QPR, lq = t % QPR;
  uint acc[TM][TM];
  for (uint r = 0; r < TM; r++) for (uint c = 0; c < TM; c++) acc[r][c] = 0;
  for (uint k0 = 0; k0 < kq; k0 += QPR) {
    for (uint p = 0; p < PASSES; p++) {
      uint row = lr + p * ROWS_PER_PASS;
      uint4 va = A[(row0 + row) * kq + k0 + lq];
      uint4 vb = B[(col0 + row) * kq + k0 + lq];
      As[lq * 4 + 0][row] = va.x; As[lq * 4 + 1][row] = va.y; As[lq * 4 + 2][row] = va.z; As[lq * 4 + 3][row] = va.w;
      Bs[lq * 4 + 0][row] = vb.x; Bs[lq * 4 + 1][row] = vb.y; Bs[lq * 4 + 2][row] = vb.z; Bs[lq * 4 + 3][row] = vb.w;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint w = 0; w < KC; w++) {
      uint a[TM], b[TM];
      for (uint r = 0; r < TM; r++) a[r] = As[w][tp.y + 16 * r];
      for (uint c = 0; c < TM; c++) b[c] = Bs[w][tp.x + 16 * c];
      for (uint r = 0; r < TM; r++) for (uint c = 0; c < TM; c++) acc[r][c] += popcount(a[r] & b[c]);
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
  }
  for (uint r = 0; r < TM; r++) for (uint c = 0; c < TM; c++)
    C[(row0 + tp.y + 16 * r) * m + col0 + tp.x + 16 * c] = acc[r][c];
}
