// ukl/simd.h: SIMD-group operations (decision 0440 §3.1, §5).
//
// Convergent: every lane of the SIMD group calls each one, passing its own lane (`LANE` row, or `lid %
// UKL_SIMD_WIDTH` in a 1-D threadgroup). Each operation with a backend form (`UKL_NATIVE_*`, ukl/backend.h) has its
// reference form beside it (`_ref`), built on shuffles and the ballot alone; the library's tests check the two equal.

#pragma once
#include <ukl/bits.h>

// The width is the backend's (`UKL_SIMD_WIDTH`: 32 on Metal, CUDA and the CPU; an OpenCL device's own, or 32
// emulated). A ballot or match is a 32-bit mask, so a wider group (AMD's 64) needs a wider mask type first.
#if UKL_SIMD_WIDTH < 1 || UKL_SIMD_WIDTH > 64 || (UKL_SIMD_WIDTH & (UKL_SIMD_WIDTH - 1)) != 0
#  error "ukl/simd.h: UKL_SIMD_WIDTH is a power of two from 1 to 64 (decision 0440 §3.5)"
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
UKL_INLINE uint32_t ukl_sg_count(bool b __UKL_PARAMS) { return ukl_mask_count(ukl_sg_ballot(b __UKL_ARGS)); }
UKL_INLINE uint32_t ukl_sg_count_below(bool b, uint32_t lane __UKL_PARAMS) { return ukl_mask_rank(ukl_sg_ballot(b __UKL_ARGS), lane); }
UKL_INLINE uint32_t ukl_sg_first(bool b __UKL_PARAMS)
{
    ukl_mask_t m = ukl_sg_ballot(b __UKL_ARGS);
    return m == 0u ? UKL_NONE : ukl_mask_lowest(m);
}

// The lanes whose `v` equals this lane's, as a mask (this lane's bit set). With it, a lane's rank among the equal
// lanes before it (`ukl_mask_rank(m, lane)`) and whether it is the last of them (`m >> lane == 1`): a stable order
// within the group with no atomic deciding it (the radix sort's scatter).
UKL_INLINE ukl_mask_t ukl_sg_match_ref(uint32_t v __UKL_PARAMS)
{
    ukl_mask_t m = 0u;
    for (uint32_t j = 0u; j < UKL_SIMD_WIDTH; j++)
        if (ukl_sg_shuffle(v, j __UKL_ARGS) == v)
            m |= ukl_mask_lane(j);
    return m;
}
#if defined(UKL_NATIVE_SG_MATCH)
UKL_INLINE ukl_mask_t ukl_sg_match(uint32_t v __UKL_PARAMS) { return ukl_sg_match_native(v __UKL_ARGS); }
#else
UKL_INLINE ukl_mask_t ukl_sg_match(uint32_t v __UKL_PARAMS) { return ukl_sg_match_ref(v __UKL_ARGS); }
#endif

// Lane `from`'s value, in every lane.
UKL_INLINE uint32_t ukl_sg_broadcast(uint32_t v, uint32_t from __UKL_PARAMS) { return ukl_sg_shuffle(v, from __UKL_ARGS); }

// The thread's place in the order the SIMD groups start: lane 0 takes a ticket for the group's lanes from `counter`
// (one atomic add a group, zero before the launch), every lane gets it plus its own lane, so a group started later
// has later places. A kernel that reads what earlier places wrote (a look-back) finds it written more often. Where
// the groups run in their ids' order (the CPU's backends, a group's chunks in turn), the place is the thread's id and
// no ticket is taken. Every thread of the launch calls it once, before any returns: the places are then the
// launch's threads, each once.
#if defined(UKL_VEC) || defined(UKL_CPU)
// A macro, so the id stays what it is (consecutive lanes) and the lane is not computed.
#define ukl_sg_ticket(counter, gid, lane) (gid)
#else
UKL_INLINE uint32_t ukl_sg_ticket(UKL_GLOBAL uint32_t* counter, uint32_t gid, uint32_t lane __UKL_PARAMS)
{
    (void)gid;
    uint32_t t = 0u;
    if (lane == 0u)
        t = ukl_atomic_add(counter, UKL_SIMD_WIDTH);
    return ukl_sg_shuffle(t, 0u __UKL_ARGS) + lane;
}
#endif

// Every word of `out[0..n)` set to `v` by the launch's own threads, before any of them reads one: the words in
// ranges of `UKL_FILL_RANGE`, each taken from `counts[0]` (started) by a SIMD group, filled by its lanes together
// and counted in `counts[UKL_FILL_APART]` (finished, after a fence; a cache line from the ranges taken, so the waits'
// polls do not slow the takes); a group that finds no range left waits until every range
// is finished. A range it waits for was taken by a group that is running and waits for nothing until it has filled
// it (decision 0540, point 1), and the wait is bounded: `false` after `UKL_FILL_SPINS` turns, when the caller must
// not read the words (its own answer, computed without them, is still right). both counts are 0 before the launch.
// Where the groups run in their ids' order (the CPU's), each thread takes ranges alone, and the first ones fill all.
#ifndef UKL_FILL_RANGE
#define UKL_FILL_RANGE 4096u
#endif
#ifndef UKL_FILL_APART
#define UKL_FILL_APART 32u
#endif
#ifndef UKL_FILL_SPINS
#define UKL_FILL_SPINS 65536u
#endif
#if defined(UKL_VEC) || defined(UKL_CPU)
UKL_INLINE bool ukl_sg_fill(UKL_GLOBAL uint32_t* out, uint32_t n, uint32_t v, UKL_GLOBAL uint32_t* counts)
{
    uint32_t ranges = (n + UKL_FILL_RANGE - 1u) / UKL_FILL_RANGE;
    for (;;) {
        uint32_t r = ukl_atomic_load(&counts[0]) < ranges ? ukl_atomic_add(&counts[0], 1u) : ranges;
        if (r >= ranges)
            break;
        uint32_t end = r * UKL_FILL_RANGE + UKL_FILL_RANGE < n ? r * UKL_FILL_RANGE + UKL_FILL_RANGE : n;
        for (uint32_t w = r * UKL_FILL_RANGE; w < end; w++)
            out[w] = v;
        ukl_fence_device();
        ukl_atomic_add(&counts[UKL_FILL_APART], 1u);
    }
    for (uint32_t t = 0u; t < UKL_FILL_SPINS; t++)
        if (ukl_atomic_load(&counts[UKL_FILL_APART]) >= ranges)
            return true;
    return false;
}
#define ukl_sg_fill_lanes(out, n, v, counts, lane) ukl_sg_fill(out, n, v, counts)
#else
UKL_INLINE bool ukl_sg_fill_lanes(UKL_GLOBAL uint32_t* out, uint32_t n, uint32_t v, UKL_GLOBAL uint32_t* counts, uint32_t lane __UKL_PARAMS)
{
    uint32_t ranges = (n + UKL_FILL_RANGE - 1u) / UKL_FILL_RANGE;
    // Done already, by a plain read: the count only grows, so a cached value that says so is right, and the groups
    // that start after the fill (most) read no atomic. One that does not say so goes on to the atomics.
    if (counts[UKL_FILL_APART] >= ranges) {
        ukl_fence_device();
        return true;
    }
    for (;;) {
        uint32_t r = 0u;
        if (lane == 0u)
            r = ukl_atomic_load(&counts[0]) < ranges ? ukl_atomic_add(&counts[0], 1u) : ranges;
        r = ukl_sg_shuffle(r, 0u __UKL_ARGS);
        if (r >= ranges)
            break;
        uint32_t end = r * UKL_FILL_RANGE + UKL_FILL_RANGE < n ? r * UKL_FILL_RANGE + UKL_FILL_RANGE : n;
        for (uint32_t w = r * UKL_FILL_RANGE + lane; w < end; w += UKL_SIMD_WIDTH)
            out[w] = v;
        ukl_fence_device();
        ukl_sg_barrier();
        if (lane == 0u)
            ukl_atomic_add(&counts[UKL_FILL_APART], 1u);
    }
    for (uint32_t t = 0u; t < UKL_FILL_SPINS; t++)
        if (ukl_atomic_load(&counts[UKL_FILL_APART]) >= ranges) {
            ukl_fence_device();
            return true;
        }
    return false;
}
#endif

// A SIMD group's places published: its bit in `seen` (a bit a group of `UKL_SIMD_WIDTH` places), set once its lanes'
// stores are done (a fence, the group's barrier, then lane 0's or), and read by `ukl_sg_published`. Every lane calls
// it. Where a group's chunks run in turn (the CPU's), its last lane sets it, after the others' stores; a place of the
// reader's own launch group (from `first`) is written already, so only earlier groups' bits are read.
#if defined(UKL_VEC) || defined(UKL_CPU)
UKL_INLINE void ukl_sg_publish(UKL_GLOBAL uint32_t* seen, uint32_t place, uint32_t lane)
{
    if (lane == UKL_SIMD_WIDTH - 1u)
        ukl_atomic_or(&seen[place >> 10], 1u << ((place >> 5) & 31u));
}
UKL_INLINE bool ukl_sg_published(UKL_GLOBAL uint32_t* seen, uint32_t place, uint32_t first)
{
    uint32_t w = ukl_atomic_load(&seen[place >> 10]);
    return place >= first || ((w >> ((place >> 5) & 31u)) & 1u) != 0u;
}
#else
UKL_INLINE void ukl_sg_publish(UKL_GLOBAL uint32_t* seen, uint32_t place, uint32_t lane __UKL_PARAMS)
{
    ukl_fence_device();
    ukl_sg_barrier();
    if (lane == 0u)
        ukl_atomic_or(&seen[place >> 10], 1u << ((place >> 5) & 31u));
}
UKL_INLINE bool ukl_sg_published(UKL_GLOBAL uint32_t* seen, uint32_t place, uint32_t first)
{
    (void)first;
    bool on = ((ukl_atomic_load(&seen[place >> 10]) >> ((place >> 5) & 31u)) & 1u) != 0u;
    ukl_fence_device();
    return on;
}
#endif
