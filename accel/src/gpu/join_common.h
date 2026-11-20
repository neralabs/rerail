// What every join kernel shares (accel's `join.uglykl`, `join_sort.uglykl`, the probe's and the tile's, and the
// engine's stages): the threadgroup, the parameter block, and a threadgroup's exclusive prefix sum. Joined in front
// of each source as text (`join::COMMON`).

#include <ukl/backend.h>
#include <ukl/simd.h>

// Threads a threadgroup of every join kernel, and its SIMD groups.
#define JOIN_TG 256u
#define JOIN_SGS (JOIN_TG / UKL_SIMD_WIDTH)

// A join kernel's parameters (`Pass::run`): up to eight words, then up to eight longs (a keys' bases). Scalars, as
// the reflection of an `ARGS` structure takes them.
typedef struct JoinP {
  uint32_t p0, p1, p2, p3, p4, p5, p6, p7;
  int64_t b0, b1, b2, b3, b4, b5, b6, b7;
} JoinP;
UKL_REFLECT_STRUCT(JoinP, p0, p1, p2, p3, p4, p5, p6, p7, b0, b1, b2, b3, b4, b5, b6, b7)

// The exclusive prefix of `mine` over the threadgroup's threads, in thread order; the threadgroup's total into
// `*total`. Every thread of the threadgroup calls it (a barrier inside). `part`: a word a SIMD group, `JOIN_SGS`
// of them (256 at a width of 1).
UKL_INLINE uint32_t join_tg_prefix(uint32_t mine, uint32_t t, uint32_t lane, UKL_LOCAL uint32_t* part, UKL_PRIVATE uint32_t* total __UKL_PARAMS)
{
  uint32_t inc = ukl_sg_inclusive_sum(mine, lane __UKL_ARGS);
  uint32_t sg = t / UKL_SIMD_WIDTH;
  if (lane == UKL_SIMD_WIDTH - 1u) part[sg] = inc;
  ukl_tg_barrier();
  uint32_t off = 0u, all = 0u;
  for (uint32_t k = 0u; k < JOIN_SGS; k++) { uint32_t v = part[k]; if (k < sg) off += v; all += v; }
  *total = all;
  return off + inc - mine;
}
