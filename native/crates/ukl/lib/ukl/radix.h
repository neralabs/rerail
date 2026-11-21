// ukl/radix.h: a stable LSD radix sort of keys with a value each (decision 0440 §5; the gpu lane's sort,
// accel/src/gpu/join.rs, decision 0095 §4).
//
// A pass sorts by one 8-bit digit; the host (`rerail_gpu::library::radix_sort_pairs`) runs one pass per digit of the
// keys' bits, low digit first:
//
//   1. `..._hist`: each tile's digit histogram, digit-major across the tiles (`hist[d * tiles + tile]`);
//   2. the histograms scanned (ukl/scan.h): each digit's offset for each tile;
//   3. `..._scatter`: each row at its digit's offset for its tile, after the tile's earlier rows of that digit.
//
// A tile is `UKL_RADIX_TILE` rows: `UKL_RADIX_THREADS` threads, a row each a round, `UKL_RADIX_ROUNDS` rounds. A
// row's place within its tile is the rows of its digit in the earlier rounds and in the earlier SIMD groups of its
// round, and its rank among the earlier lanes of its SIMD group with the same digit (`ukl_sg_match`). No atomic
// decides an order: the sort is stable, and the same on every run.

#pragma once
#include <ukl/group.h>

#define UKL_RADIX_BITS 8u
#define UKL_RADIX_DIGITS 256u
// A thread a digit: the histogram and the offsets are a word a thread.
#define UKL_RADIX_THREADS 256u
#define UKL_RADIX_ROUNDS 4u
#define UKL_RADIX_TILE (UKL_RADIX_THREADS * UKL_RADIX_ROUNDS)
#define UKL_RADIX_GROUPS (UKL_RADIX_THREADS / UKL_SIMD_WIDTH)
// The tile's threads a thread of a narrower group runs (decision 0523 §3), at most: `UKL_RADIX_THREADS / lsz` of
// them, a block between barriers looping over its own. The CPU's groups are narrow (each of a group's threads is a
// coroutine its barriers switch); a GPU's are `UKL_RADIX_THREADS`, its code a thread each.
#ifndef UKL_RADIX_VT_MAX
#if defined(UKL_CPU)
#define UKL_RADIX_VT_MAX 8u
#else
#define UKL_RADIX_VT_MAX 1u
#endif
#endif
// The counts by SIMD group (`UKL_RADIX_GROUPS * UKL_RADIX_DIGITS` words) fit a threadgroup's memory from 8 lanes up.
// Narrower, the sort's kernels are not defined (`rerail_gpu::library::radix_sort_pairs` says so), and the rest of a
// program that includes it builds.
#if UKL_SIMD_WIDTH >= 8

typedef struct UklRadixArgs {
    uint32_t n;
    // The digit's lowest bit.
    uint32_t shift;
    uint32_t tiles;
} UklRadixArgs;
UKL_REFLECT_STRUCT(UklRadixArgs, n, shift, tiles)

// Row `r` of round `k` of tile `tile`, thread `t`.
UKL_INLINE uint32_t ukl_radix_row(uint32_t tile, uint32_t k, uint32_t t) { return tile * UKL_RADIX_TILE + k * UKL_RADIX_THREADS + t; }

// The kernels for a key type: `ukl_radix_<Suffix>_hist`, `ukl_radix_<Suffix>_scatter`.
#define UKL_DEFINE_RADIX_KERNELS(Suffix, K)                                                                             \
    DEFINE_KERNEL2(ukl, ukl_radix_##Suffix##_hist,                                                                      \
                   ARGS,     UklRadixArgs, args, ,                                                                      \
                   ROBUFFER, K,            keys, "[n]",                                                                 \
                   RWBUFFER, uint32_t,     hist, "[256 * tiles]",                                                       \
                   SHARED,   uint32_t,     h,    UKL_RADIX_DIGITS,                                                      \
                   TGID0,    uint32_t,     tile, ,                                                                      \
                   LID0,     uint32_t,     t,    )                                                                      \
    {                                                                                                                   \
        ukl_radix_hist_tile_##Suffix(keys, hist, args, h, tile, t, UKL_RADIX_THREADS);                                                              \
    }                                                                                                                   \
    DEFINE_KERNEL2(ukl, ukl_radix_##Suffix##_scatter,                                                                   \
                   ARGS,     UklRadixArgs, args,      ,                                                                 \
                   ROBUFFER, K,            keys,      "[n]",                                                            \
                   ROBUFFER, uint32_t,     values,    "[n]",                                                            \
                   ROBUFFER, uint32_t,     offsets,   "[256 * tiles + 1]",                                              \
                   RWBUFFER, K,            outKeys,   "[n]",                                                            \
                   RWBUFFER, uint32_t,     outValues, "[n]",                                                            \
                   SHARED,   uint32_t,     counts,    UKL_RADIX_GROUPS * UKL_RADIX_DIGITS,                              \
                   SHARED,   uint32_t,     before,    UKL_RADIX_DIGITS,                                                 \
                   SHARED,   uint32_t,     base,      UKL_RADIX_DIGITS,                                                 \
                   TGID0,    uint32_t,     tile,      ,                                                                 \
                   LID0,     uint32_t,     t,         )                                                                 \
    {                                                                                                                   \
        ukl_radix_scatter_tile_##Suffix(keys, values, offsets, outKeys, outValues, args, counts, before, base, tile, t, UKL_RADIX_THREADS __UKL_ARGS);\
    }

#define UKL_RADIX_NAME(f) f##_u32
#define UKL_RADIX_KEY uint32_t
#include <ukl/body/radix_tile.h>
#define UKL_RADIX_NAME(f) f##_u64
#define UKL_RADIX_KEY uint64_t
#include <ukl/body/radix_tile.h>

UKL_DEFINE_RADIX_KERNELS(u32, uint32_t)
UKL_DEFINE_RADIX_KERNELS(u64, uint64_t)

#endif // UKL_SIMD_WIDTH >= 8
