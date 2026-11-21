// ukl/simd.h: SIMD-group operations (decision 0440 §3.1, §5).
//
// Convergent: every lane of the SIMD group calls each one, passing its own lane (`LANE` row, or `lid %
// UKL_SIMD_WIDTH` in a 1-D threadgroup). Each operation with a backend form (`UKL_NATIVE_*`, ukl/backend.h) has its
// reference form beside it (`_ref`), built on shuffles and the ballot alone; the library's tests check the two equal.

#pragma once
#include <ukl/bits.h>

// The width is the backend's (`UKL_SIMD_WIDTH`: 32 on Metal, CUDA and the CPU; an OpenCL device's own, or 32
// emulated). A ballot or match is a 32-bit mask, so a wider group (AMD's 64) needs a wider mask type first.
#if UKL_SIMD_WIDTH < 1 || UKL_SIMD_WIDTH > 32 || (UKL_SIMD_WIDTH & (UKL_SIMD_WIDTH - 1)) != 0
#  error "ukl/simd.h: UKL_SIMD_WIDTH is a power of two from 1 to 32 (decision 0440 §3.5)"
#endif

// Reductions: every lane gets the group's sum, minimum or maximum (sums wrap at 2^32).
UKL_INLINE uint32_t ukl_sg_sum_ref(uint32_t v __UKL_PARAMS)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2u; d > 0u; d >>= 1)
        v += ukl_sg_shuffle_xor(v, d __UKL_ARGS);
    return v;
}
UKL_INLINE uint32_t ukl_sg_min_ref(uint32_t v __UKL_PARAMS)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2u; d > 0u; d >>= 1) {
        uint32_t u = ukl_sg_shuffle_xor(v, d __UKL_ARGS);
        v = u < v ? u : v;
    }
    return v;
}
UKL_INLINE uint32_t ukl_sg_max_ref(uint32_t v __UKL_PARAMS)
{
    for (uint32_t d = UKL_SIMD_WIDTH / 2u; d > 0u; d >>= 1) {
        uint32_t u = ukl_sg_shuffle_xor(v, d __UKL_ARGS);
        v = u > v ? u : v;
    }
    return v;
}

// Scans: lane `i` gets the sum of lanes `0..=i` (inclusive) or `0..i` (exclusive).
UKL_INLINE uint32_t ukl_sg_inclusive_sum_ref(uint32_t v, uint32_t lane __UKL_PARAMS)
{
    for (uint32_t d = 1u; d < UKL_SIMD_WIDTH; d <<= 1) {
        uint32_t u = ukl_sg_shuffle_up(v, d __UKL_ARGS);
        if (lane >= d)
            v += u;
    }
    return v;
}
UKL_INLINE uint32_t ukl_sg_exclusive_sum_ref(uint32_t v, uint32_t lane __UKL_PARAMS) { return ukl_sg_inclusive_sum_ref(v, lane __UKL_ARGS) - v; }

#if defined(UKL_NATIVE_SG_REDUCE)
UKL_INLINE uint32_t ukl_sg_sum(uint32_t v __UKL_PARAMS) { return ukl_sg_sum_native(v __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_min(uint32_t v __UKL_PARAMS) { return ukl_sg_min_native(v __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_max(uint32_t v __UKL_PARAMS) { return ukl_sg_max_native(v __UKL_ARGS); }
#else
UKL_INLINE uint32_t ukl_sg_sum(uint32_t v __UKL_PARAMS) { return ukl_sg_sum_ref(v __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_min(uint32_t v __UKL_PARAMS) { return ukl_sg_min_ref(v __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_max(uint32_t v __UKL_PARAMS) { return ukl_sg_max_ref(v __UKL_ARGS); }
#endif

#if defined(UKL_NATIVE_SG_SCAN)
UKL_INLINE uint32_t ukl_sg_inclusive_sum(uint32_t v, uint32_t lane __UKL_PARAMS) { return ukl_sg_inclusive_sum_native(v __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum(uint32_t v, uint32_t lane __UKL_PARAMS) { return ukl_sg_exclusive_sum_native(v __UKL_ARGS); }
#else
UKL_INLINE uint32_t ukl_sg_inclusive_sum(uint32_t v, uint32_t lane __UKL_PARAMS) { return ukl_sg_inclusive_sum_ref(v, lane __UKL_ARGS); }
UKL_INLINE uint32_t ukl_sg_exclusive_sum(uint32_t v, uint32_t lane __UKL_PARAMS) { return ukl_sg_exclusive_sum_ref(v, lane __UKL_ARGS); }
#endif

// The inclusive sum of the SIMD group's `v` up to this lane, and in `*total` the whole group's: the backend's in one
// operation (`UKL_NATIVE_SG_TOTALS`), else the scan and its last lane's.
UKL_INLINE uint32_t ukl_sg_inclusive_sum_total(uint32_t v, uint32_t lane, UKL_PRIVATE uint32_t* total __UKL_PARAMS)
{
#if defined(UKL_NATIVE_SG_TOTALS)
    (void)lane;
    return ukl_sg_inclusive_sum_total_native(v, total __UKL_ARGS);
#else
    uint32_t inc = ukl_sg_inclusive_sum(v, lane __UKL_ARGS);
    *total = ukl_sg_shuffle(inc, UKL_SIMD_WIDTH - 1u __UKL_ARGS);
    return inc;
#endif
}

// Votes: the lanes whose `b` holds, as a mask; how many; how many below this lane (a compaction's slot); the
// lowest such lane (`UKL_NONE` when none).
UKL_INLINE uint32_t ukl_sg_count(bool b __UKL_PARAMS) { return ukl_popcount32(ukl_sg_ballot(b __UKL_ARGS)); }
UKL_INLINE uint32_t ukl_sg_count_below(bool b, uint32_t lane __UKL_PARAMS) { return ukl_rank32(ukl_sg_ballot(b __UKL_ARGS), lane); }
UKL_INLINE uint32_t ukl_sg_first(bool b __UKL_PARAMS)
{
    uint32_t m = ukl_sg_ballot(b __UKL_ARGS);
    return m == 0u ? UKL_NONE : ukl_ctz32(m);
}

// The lanes whose `v` equals this lane's, as a mask (this lane's bit set). With it, a lane's rank among the equal
// lanes before it (`ukl_rank32(m, lane)`) and whether it is the last of them (`m >> lane == 1`): a stable order within
// the group with no atomic deciding it (the radix sort's scatter).
UKL_INLINE uint32_t ukl_sg_match_ref(uint32_t v __UKL_PARAMS)
{
    uint32_t m = 0u;
    for (uint32_t j = 0u; j < UKL_SIMD_WIDTH; j++)
        if (ukl_sg_shuffle(v, j __UKL_ARGS) == v)
            m |= 1u << j;
    return m;
}
#if defined(UKL_NATIVE_SG_MATCH)
UKL_INLINE uint32_t ukl_sg_match(uint32_t v __UKL_PARAMS) { return ukl_sg_match_native(v __UKL_ARGS); }
#else
UKL_INLINE uint32_t ukl_sg_match(uint32_t v __UKL_PARAMS) { return ukl_sg_match_ref(v __UKL_ARGS); }
#endif

// Lane `from`'s value, in every lane.
UKL_INLINE uint32_t ukl_sg_broadcast(uint32_t v, uint32_t from __UKL_PARAMS) { return ukl_sg_shuffle(v, from __UKL_ARGS); }
