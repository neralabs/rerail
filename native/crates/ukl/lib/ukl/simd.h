// ukl/simd.h: SIMD-group operations (decision 0400 §3.1, §5).
//
// Convergent: every lane of the SIMD group calls each one, passing its own lane (`LANE` row, or `lid %
// UKL_SIMD_WIDTH` in a 1-D threadgroup). Each operation with a backend form (`UKL_NATIVE_*`, ukl/backend.h) has its
// reference form beside it (`_ref`), built on shuffles and the ballot alone; the library's tests check the two equal.

#pragma once
#include <ukl/bits.h>

#if UKL_SIMD_WIDTH != 32
#  error "ukl/simd.h is written for SIMD groups of 32 lanes; a wider group needs its own forms (decision 0400 §3.1)"
#endif

// Reductions: every lane gets the group's sum, minimum or maximum (sums wrap at 2^32).
UKL_INLINE uint32_t ukl_sg_sum_ref(uint32_t v)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2u; d > 0u; d >>= 1)
        v += ukl_sg_shuffle_xor(v, d);
    return v;
}
UKL_INLINE uint32_t ukl_sg_min_ref(uint32_t v)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2u; d > 0u; d >>= 1) {
        uint32_t u = ukl_sg_shuffle_xor(v, d);
        v = u < v ? u : v;
    }
    return v;
}
UKL_INLINE uint32_t ukl_sg_max_ref(uint32_t v)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2u; d > 0u; d >>= 1) {
        uint32_t u = ukl_sg_shuffle_xor(v, d);
        v = u > v ? u : v;
    }
    return v;
}

// Scans: lane `i` gets the sum of lanes `0..=i` (inclusive) or `0..i` (exclusive).
UKL_INLINE uint32_t ukl_sg_inclusive_sum_ref(uint32_t v, uint32_t lane)
{
    for (uint32_t d = 1u; d < UKL_SIMD_WIDTH; d <<= 1) {
        uint32_t u = ukl_sg_shuffle_up(v, d);
        if (lane >= d)
            v += u;
    }
    return v;
}
UKL_INLINE uint32_t ukl_sg_exclusive_sum_ref(uint32_t v, uint32_t lane) { return ukl_sg_inclusive_sum_ref(v, lane) - v; }

#if defined(UKL_NATIVE_SG_REDUCE)
UKL_INLINE uint32_t ukl_sg_sum(uint32_t v) { return ukl_sg_sum_native(v); }
UKL_INLINE uint32_t ukl_sg_min(uint32_t v) { return ukl_sg_min_native(v); }
UKL_INLINE uint32_t ukl_sg_max(uint32_t v) { return ukl_sg_max_native(v); }
#else
UKL_INLINE uint32_t ukl_sg_sum(uint32_t v) { return ukl_sg_sum_ref(v); }
UKL_INLINE uint32_t ukl_sg_min(uint32_t v) { return ukl_sg_min_ref(v); }
UKL_INLINE uint32_t ukl_sg_max(uint32_t v) { return ukl_sg_max_ref(v); }
#endif

#if defined(UKL_NATIVE_SG_SCAN)
UKL_INLINE uint32_t ukl_sg_inclusive_sum(uint32_t v, uint32_t lane) { return ukl_sg_inclusive_sum_native(v); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum(uint32_t v, uint32_t lane) { return ukl_sg_exclusive_sum_native(v); }
#else
UKL_INLINE uint32_t ukl_sg_inclusive_sum(uint32_t v, uint32_t lane) { return ukl_sg_inclusive_sum_ref(v, lane); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum(uint32_t v, uint32_t lane) { return ukl_sg_exclusive_sum_ref(v, lane); }
#endif

// Votes: the lanes whose `b` holds, as a mask; how many; how many below this lane (a compaction's slot); the
// lowest such lane (`UKL_NONE` when none).
UKL_INLINE uint32_t ukl_sg_count(bool b) { return ukl_popcount32(ukl_sg_ballot(b)); }
UKL_INLINE uint32_t ukl_sg_count_below(bool b, uint32_t lane) { return ukl_rank32(ukl_sg_ballot(b), lane); }
UKL_INLINE uint32_t ukl_sg_first(bool b)
{
    uint32_t m = ukl_sg_ballot(b);
    return m == 0u ? UKL_NONE : ukl_ctz32(m);
}

// Lane `from`'s value, in every lane.
UKL_INLINE uint32_t ukl_sg_broadcast(uint32_t v, uint32_t from) { return ukl_sg_shuffle(v, from); }
