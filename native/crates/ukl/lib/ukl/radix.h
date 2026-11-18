// ukl/radix.h: a stable LSD radix sort of keys with a value each (decision 0400 §5; the gpu lane's sort,
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

struct UklRadixArgs {
    uint32_t n;
    // The digit's lowest bit.
    uint32_t shift;
    uint32_t tiles;
};
UKL_REFLECT_STRUCT(UklRadixArgs, n, shift, tiles)

template <typename K> UKL_INLINE uint32_t ukl_radix_digit(K key, uint32_t shift) { return (uint32_t)(key >> shift) & (UKL_RADIX_DIGITS - 1u); }

// Row `r` of round `k` of tile `tile`, thread `t`.
UKL_INLINE uint32_t ukl_radix_row(uint32_t tile, uint32_t k, uint32_t t) { return tile * UKL_RADIX_TILE + k * UKL_RADIX_THREADS + t; }

// The tile's digit histogram, into `hist` digit-major. `h`: `UKL_RADIX_DIGITS` threadgroup words.
template <typename K>
UKL_INLINE void ukl_radix_hist_tile(UKL_GLOBAL const K* keys, UKL_GLOBAL uint32_t* hist, UklRadixArgs args, UKL_LOCAL uint32_t* h, uint32_t tile, uint32_t t)
{
    h[t] = 0u;
    ukl_tg_barrier();
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        uint32_t r = ukl_radix_row(tile, k, t);
        if (r < args.n)
            ukl_atomic_add_local(&h[ukl_radix_digit(keys[r], args.shift)], 1u);
    }
    ukl_tg_barrier();
    hist[t * args.tiles + tile] = h[t];
}

// The tile's rows at their places. `counts`: `UKL_RADIX_GROUPS * UKL_RADIX_DIGITS` threadgroup words (the round's
// rows of each digit, by SIMD group); `before`, `base`: `UKL_RADIX_DIGITS` each (the digit's rows in the earlier
// rounds; its offset for the tile).
template <typename K>
UKL_INLINE void ukl_radix_scatter_tile(UKL_GLOBAL const K* keys, UKL_GLOBAL const uint32_t* values, UKL_GLOBAL const uint32_t* offsets,
                                       UKL_GLOBAL K* outKeys, UKL_GLOBAL uint32_t* outValues, UklRadixArgs args, UKL_LOCAL uint32_t* counts,
                                       UKL_LOCAL uint32_t* before, UKL_LOCAL uint32_t* base, uint32_t tile, uint32_t t)
{
    uint32_t lane = t % UKL_SIMD_WIDTH, sg = t / UKL_SIMD_WIDTH;
    base[t] = offsets[t * args.tiles + tile];
    before[t] = 0u;
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        uint32_t r = ukl_radix_row(tile, k, t);
        bool live = r < args.n;
        K key = live ? keys[r] : (K)0;
        // A row past the end has a digit of its own, equal to no live row's.
        uint32_t d = live ? ukl_radix_digit(key, args.shift) : UKL_RADIX_DIGITS;
        uint32_t m = ukl_sg_match(d);
        uint32_t rank = ukl_rank32(m, lane);
        bool last = (m >> lane) == 1u;
        for (uint32_t s = 0u; s < UKL_RADIX_GROUPS; s++)
            counts[s * UKL_RADIX_DIGITS + t] = 0u;
        ukl_tg_barrier();
        if (live && last)
            counts[sg * UKL_RADIX_DIGITS + d] = rank + 1u;
        ukl_tg_barrier();
        // Thread `t` turns digit `t`'s counts by SIMD group into the groups' starts, and its round's total.
        uint32_t acc = 0u;
        for (uint32_t s = 0u; s < UKL_RADIX_GROUPS; s++) {
            uint32_t c = counts[s * UKL_RADIX_DIGITS + t];
            counts[s * UKL_RADIX_DIGITS + t] = acc;
            acc += c;
        }
        ukl_tg_barrier();
        if (live) {
            uint32_t at = base[d] + before[d] + counts[sg * UKL_RADIX_DIGITS + d] + rank;
            outKeys[at] = key;
            outValues[at] = values[r];
        }
        ukl_tg_barrier();
        before[t] += acc;
        ukl_tg_barrier();
    }
}

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
        ukl_radix_hist_tile(keys, hist, args, h, tile, t);                                                              \
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
        ukl_radix_scatter_tile(keys, values, offsets, outKeys, outValues, args, counts, before, base, tile, t);          \
    }

UKL_DEFINE_RADIX_KERNELS(u32, uint32_t)
UKL_DEFINE_RADIX_KERNELS(u64, uint64_t)
