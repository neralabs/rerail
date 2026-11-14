// G4 (older path): simdgroup_matrix 8x8, half inputs, float accumulate. Each simdgroup owns a 32x32
// output block (4x4 fragments); 4 simdgroups per threadgroup = 64x64. Operands load straight from
// device memory (no threadgroup staging). M, N multiples of 64; K multiple of 8.
#include <metal_stdlib>
using namespace metal;
kernel void sg_mm(device const half* A [[buffer(0)]], device const half* B [[buffer(1)]],
                  device float* C [[buffer(2)]], constant uint& M [[buffer(3)]],
                  constant uint& N [[buffer(4)]], constant uint& K [[buffer(5)]],
                  uint2 tgid [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]]) {
  uint row0 = tgid.y * 64 + (sg / 2) * 32;
  uint col0 = tgid.x * 64 + (sg % 2) * 32;
  simdgroup_float8x8 acc[4][4];
  for (uint i = 0; i < 4; i++) for (uint j = 0; j < 4; j++) acc[i][j] = make_filled_simdgroup_matrix<float, 8, 8>(0.0f);
  for (uint k = 0; k < K; k += 8) {
    simdgroup_half8x8 a[4], b[4];
    for (uint i = 0; i < 4; i++) simdgroup_load(a[i], A + (ulong)(row0 + 8 * i) * K + k, K);
    for (uint j = 0; j < 4; j++) simdgroup_load(b[j], B + (ulong)k * N + col0 + 8 * j, N);
    for (uint i = 0; i < 4; i++) for (uint j = 0; j < 4; j++) simdgroup_multiply_accumulate(acc[i][j], a[i], b[j], acc[i][j]);
  }
  for (uint i = 0; i < 4; i++) for (uint j = 0; j < 4; j++)
    simdgroup_store(acc[i][j], C + (ulong)(row0 + 8 * i) * N + col0 + 8 * j, N);
}

// One-hot comparison baseline: C[r][:] = B[code[r]][:] as a plain gather (what the one-hot matmul computes).
kernel void gather_rows(device const ushort* code [[buffer(0)]], device const char* B [[buffer(1)]],
                        device int* C [[buffer(2)]], constant uint& R [[buffer(3)]], constant uint& N [[buffer(4)]],
                        uint2 g [[thread_position_in_grid]]) {
  if (g.y >= R || g.x >= N) return;
  C[(ulong)g.y * N + g.x] = B[code[g.y] * N + g.x];
}
