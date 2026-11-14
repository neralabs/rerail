// G4: Metal 4 tensor ops (Metal Performance Primitives matmul2d) from runtime-compiled MSL.
// C (M x N) = A (M x K) * B (K x N), all row-major in device buffers, wrapped as inline tensors.
// (The header comment in MPPTensorOpsMatMul2d.h says static_slice; this SDK spells it slice<E0, E1>.)
// Tensor extents are (columns, rows): extent(0) is the fastest-varying index.
// Macros: TA, TB, TC element types; TM x TN output tile per threadgroup; NSG simdgroups.
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

kernel void mpp_mm(device TA* a [[buffer(0)]], device TB* b [[buffer(1)]], device TC* c [[buffer(2)]],
                   constant uint& M [[buffer(3)]], constant uint& N [[buffer(4)]], constant uint& K [[buffer(5)]],
                   uint2 tgid0 [[threadgroup_position_in_grid]], uint2 ntg [[threadgroups_per_grid]]) {
#if defined(SWZ) && SWZ > 0
  // Blocked raster order: walk SWZ tile-rows together across all tile-columns, so the A rows of the
  // SWZ tile-rows and one B tile-column are live in cache at once (instead of streaming all of B per tile-row).
  uint id = tgid0.y * ntg.x + tgid0.x;
  uint band = id / (SWZ * ntg.x), inb = id % (SWZ * ntg.x);
  uint2 tgid = uint2(inb / SWZ, band * SWZ + inb % SWZ);
  if (tgid.y >= ntg.y) return;
#else
  uint2 tgid = tgid0;
#endif
  auto tA = tensor<device TA, dextents<int32_t, 2>, tensor_inline>(a, dextents<int32_t, 2>(K, M));
  auto tB = tensor<device TB, dextents<int32_t, 2>, tensor_inline>(b, dextents<int32_t, 2>(N, K));
  auto tC = tensor<device TC, dextents<int32_t, 2>, tensor_inline>(c, dextents<int32_t, 2>(N, M));
  constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent));
  matmul2d<desc, execution_simdgroups<NSG>> op;
  if (tgid.y * TM + TM <= M && tgid.x * TN + TN <= N) {
    auto mA = tA.slice<dynamic_extent, TM>(0, tgid.y * TM);
    auto mB = tB.slice<TN, dynamic_extent>(tgid.x * TN, 0);
    auto mC = tC.slice<TN, TM>(tgid.x * TN, tgid.y * TM);
    op.run(mA, mB, mC);
  } else {
    auto mA = tA.slice(0, tgid.y * TM);
    auto mB = tB.slice(tgid.x * TN, 0);
    auto mC = tC.slice(tgid.x * TN, tgid.y * TM);
    op.run(mA, mB, mC);
  }
}
