// G6, matrix-unit formulation (separate file so a compile failure cannot hide the other G6 kernels).
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

// Matrix-unit formulation. Code c = hi*32 + lo. For a chunk of 32 elements build
//   L (32 x 32, hi x element) and R (32 x 32, element x lo) one-hot int8 tiles in threadgroup memory;
// H_seg (32 x 32, hi x lo) += L * R on the matrix unit (int8 x int8 -> int32), accumulated in a
// cooperative tensor across the segment, then stored. One SIMD-group per segment, 4 per threadgroup.
#define KC 32
kernel void hist_mm(device const ushort* codes [[buffer(0)]], device const uint* off [[buffer(1)]],
                    device int* H [[buffer(2)]], constant uint& nseg [[buffer(3)]],
                    uint tg [[threadgroup_position_in_grid]], uint sg [[simdgroup_index_in_threadgroup]],
                    uint lane [[thread_index_in_simdgroup]]) {
  threadgroup int8_t tl[4][32 * KC];
  threadgroup int8_t tr[4][KC * 32];
  uint seg = tg * 4 + sg;
  if (seg >= nseg) return;
  uint s0 = off[seg], s1 = off[seg + 1];
  constexpr auto desc = matmul2d_descriptor(32, 32, KC, false, false, false,
                                            matmul2d_descriptor::mode::multiply_accumulate);
  matmul2d<desc, execution_simdgroup> op;
  auto Lt = tensor<threadgroup int8_t, dextents<int32_t, 2>, tensor_inline>(&tl[sg][0], dextents<int32_t, 2>(KC, 32));
  auto Rt = tensor<threadgroup int8_t, dextents<int32_t, 2>, tensor_inline>(&tr[sg][0], dextents<int32_t, 2>(32, KC));
  auto cT = op.get_destination_cooperative_tensor<decltype(Lt), decltype(Rt), int>();
  // (The MPP header comment says get_mask / "#pragma unroll full"; this SDK has is_valid_element.)
  for (uint16_t i = 0; i < cT.get_capacity(); ++i) if (cT.is_valid_element(i)) cT[i] = 0;
  threadgroup uint4* lw = (threadgroup uint4*)&tl[sg][0];
  threadgroup uint4* rw = (threadgroup uint4*)&tr[sg][0];
  uint nch = (s1 - s0 + KC - 1) / KC;
  for (uint ch = 0; ch < nch; ch++) {
    lw[lane] = 0; lw[lane + 32] = 0;   // 1024 bytes = 64 uint4 per tile
    rw[lane] = 0; rw[lane + 32] = 0;
    simdgroup_barrier(mem_flags::mem_threadgroup);
    uint e = s0 + ch * KC + lane;
    if (e < s1) {
      uint c = codes[e];
      tl[sg][(c >> 5) * KC + lane] = 1;
      tr[sg][lane * 32 + (c & 31)] = 1;
    }
    simdgroup_barrier(mem_flags::mem_threadgroup);
    op.run(Lt, Rt, cT);
    simdgroup_barrier(mem_flags::mem_threadgroup);
  }
  auto Ht = tensor<device int, dextents<int32_t, 2>, tensor_inline>(H + (ulong)seg * 1024, dextents<int32_t, 2>(32, 32));
  cT.store(Ht);
}
