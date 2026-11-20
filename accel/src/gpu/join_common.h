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

// A sort's screen (decision 0506, `join::Screen`): a blocked Bloom filter in the host's format, read through the reads
// written once in CLIF (`clif_bloom_*`, `<ukl/reads/relation.h>`), its descriptor at byte `at` of `f`.
#include <ukl/reads/relation.h>

// Whether the screen may hold `key` (none at `at` 0xffffffff: every key): a hash and one load. A key it refuses is
// in no run.
UKL_INLINE bool screen_may_hold(UKL_GLOBAL const uint8_t* f, uint32_t at, uint64_t key)
{
  if (at == 0xffffffffu) return true;
  return clif_bloom_may(f, (uint64_t)at, clif_bloom_hash(f, 0u, key)) != 0u;
}

// Adds `key` to the screen at byte `at` of `f`: its word's two 32-bit halves set with atomics (bit b of 64-bit word
// w is bit b & 31 of 32-bit word 2w + b / 32, the host's bytes).
UKL_INLINE void screen_add(UKL_GLOBAL uint8_t* f, uint32_t at, uint64_t key)
{
  uint64_t h = clif_bloom_hash(f, 0u, key);
  uint64_t m = clif_bloom_mask(f, 0u, h);
  UKL_GLOBAL const int64_t* d = (UKL_GLOBAL const int64_t*)(f + at);
  uint32_t w = (uint32_t)(h >> 32) & (uint32_t)((uint64_t)d[1] - 1u);
  UKL_GLOBAL uint32_t* words = (UKL_GLOBAL uint32_t*)(f + (uint64_t)at + (uint64_t)d[0]);
  if ((uint32_t)m != 0u) ukl_atomic_or(&words[2u * w], (uint32_t)m);
  if ((uint32_t)(m >> 32) != 0u) ukl_atomic_or(&words[2u * w + 1u], (uint32_t)(m >> 32));
}
