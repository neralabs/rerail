// G1: empty kernel. The store never happens (tid is never 0xFFFFFFFF) but keeps the buffer bound.
#include <metal_stdlib>
using namespace metal;
kernel void empty_k(device uint* o [[buffer(0)]], uint tid [[thread_position_in_grid]]) {
  if (tid == 0xFFFFFFFFu) o[0] = 1;
}
