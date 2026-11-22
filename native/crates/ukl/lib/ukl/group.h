// ukl/group.h: threadgroup operations, over the SIMD-group ones (decision 0440 §5).
//
// For a 1-D threadgroup whose size is a multiple of `UKL_SIMD_WIDTH`, at most `UKL_TG_MAX` threads. Every thread of
// the group calls them (they hold barriers). `scratch` is threadgroup memory of `UKL_TG_SCRATCH` words (a kernel's
// `SHARED` row): the call uses it and leaves it free for the next one. With at most `UKL_SIMD_WIDTH` SIMD groups,
// one SIMD group's operations combine their partials; with more (a narrow width), one thread does.

#pragma once
#include <ukl/simd.h>

#ifndef UKL_TG_MAX
#define UKL_TG_MAX 1024u
#endif
// A word per SIMD group of the largest threadgroup, and the total.
#define UKL_TG_SCRATCH (UKL_TG_MAX / UKL_SIMD_WIDTH + 1u)

// A thread's part of a scan: the sum of the threads before it, and of the whole group.
typedef struct UklScan {
    uint32_t exclusive;
    uint32_t total;
} UklScan;

// The group's sum (wrapping), in every thread.
UKL_INLINE uint32_t ukl_tg_sum(uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t lane = lid % UKL_SIMD_WIDTH, sg = lid / UKL_SIMD_WIDTH, groups = lsz / UKL_SIMD_WIDTH;
    // A group of one SIMD group: its sum is the group's.
    if (groups <= 1u)
        return ukl_sg_sum(v __UKL_ARGS);
    uint32_t s = ukl_sg_sum(v __UKL_ARGS);
    if (lane == 0u)
        scratch[sg] = s;
    ukl_tg_barrier();
    // Every SIMD group sums the groups' partials itself: no second round through memory.
    uint32_t acc = 0u;
    for (uint32_t i = lane; i < groups; i += UKL_SIMD_WIDTH)
        acc += scratch[i];
    uint32_t t = ukl_sg_sum(acc __UKL_ARGS);
    ukl_tg_barrier();
    return t;
}

// The group's bitwise OR (`and_`: AND) of `v`, in every thread: a butterfly in each SIMD group, then the SIMD
// groups' partials.
UKL_INLINE uint32_t ukl_tg_bits(uint32_t v, bool and_, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t lane = lid % UKL_SIMD_WIDTH, sg = lid / UKL_SIMD_WIDTH, groups = lsz / UKL_SIMD_WIDTH;
    for (uint32_t m = 1u; m < UKL_SIMD_WIDTH; m <<= 1u) {
        uint32_t w = ukl_sg_shuffle_xor(v, m __UKL_ARGS);
        v = and_ ? (v & w) : (v | w);
    }
    if (lane == 0u)
        scratch[sg] = v;
    ukl_tg_barrier();
    uint32_t acc = and_ ? ~0u : 0u;
    for (uint32_t i = 0u; i < groups; i++)
        acc = and_ ? (acc & scratch[i]) : (acc | scratch[i]);
    ukl_tg_barrier();
    return acc;
}
UKL_INLINE uint32_t ukl_tg_or(uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch __UKL_PARAMS) { return ukl_tg_bits(v, false, lid, lsz, scratch __UKL_ARGS); }
UKL_INLINE uint32_t ukl_tg_and(uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch __UKL_PARAMS) { return ukl_tg_bits(v, true, lid, lsz, scratch __UKL_ARGS); }

// The exclusive scan of `v` over the group's threads in order, and the group's total.
UKL_INLINE UklScan ukl_tg_exclusive_sum(uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t lane = lid % UKL_SIMD_WIDTH, sg = lid / UKL_SIMD_WIDTH, groups = lsz / UKL_SIMD_WIDTH;
    UklScan r;
    // A group of one SIMD group: its scan is the group's.
    if (groups <= 1u) {
        uint32_t t = 0u, inc = ukl_sg_inclusive_sum_total(v, lane, &t __UKL_ARGS);
        r.exclusive = inc - v;
        r.total = t;
        return r;
    }
    uint32_t inc = ukl_sg_inclusive_sum(v, lane __UKL_ARGS);
    if (lane == UKL_SIMD_WIDTH - 1u)
        scratch[sg] = inc;
    ukl_tg_barrier();
    if (groups <= UKL_SIMD_WIDTH) {
        uint32_t p = lane < groups ? scratch[lane] : 0u;
        uint32_t pin = ukl_sg_inclusive_sum(p, lane __UKL_ARGS);
        r.exclusive = ukl_sg_shuffle(pin - p, sg __UKL_ARGS) + inc - v;
        r.total = ukl_sg_shuffle(pin, UKL_SIMD_WIDTH - 1u __UKL_ARGS);
    } else {
        // More groups than lanes: one thread scans the partials in place, the total after them.
        if (lid == 0u) {
            uint32_t run = 0u;
            for (uint32_t i = 0u; i < groups; i++) {
                uint32_t q = scratch[i];
                scratch[i] = run;
                run += q;
            }
            scratch[groups] = run;
        }
        ukl_tg_barrier();
        r.exclusive = scratch[sg] + inc - v;
        r.total = scratch[groups];
    }
    ukl_tg_barrier();
    return r;
}
