// ukl/group.h: threadgroup operations, over the SIMD-group ones (decision 0400 §5).
//
// For a 1-D threadgroup whose size is a multiple of `UKL_SIMD_WIDTH`, at most `UKL_SIMD_WIDTH` squared. Every thread
// of the group calls them (they hold barriers). `scratch` is threadgroup memory of `UKL_TG_SCRATCH` words (a
// kernel's `SHARED` row): the call uses it and leaves it free for the next one.

#pragma once
#include <ukl/simd.h>

#define UKL_TG_SCRATCH UKL_SIMD_WIDTH

// A thread's part of a scan: the sum of the threads before it, and of the whole group.
typedef struct UklScan {
    uint32_t exclusive;
    uint32_t total;
} UklScan;

// The group's sum (wrapping), in every thread.
UKL_INLINE uint32_t ukl_tg_sum(uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch)
{
    uint32_t lane = lid % UKL_SIMD_WIDTH, sg = lid / UKL_SIMD_WIDTH, groups = lsz / UKL_SIMD_WIDTH;
    uint32_t s = ukl_sg_sum(v);
    if (lane == 0u)
        scratch[sg] = s;
    ukl_tg_barrier();
    // Every SIMD group sums the groups' partials itself: no second round through memory.
    uint32_t t = ukl_sg_sum(lane < groups ? scratch[lane] : 0u);
    ukl_tg_barrier();
    return t;
}

// The exclusive scan of `v` over the group's threads in order, and the group's total.
UKL_INLINE UklScan ukl_tg_exclusive_sum(uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch)
{
    uint32_t lane = lid % UKL_SIMD_WIDTH, sg = lid / UKL_SIMD_WIDTH, groups = lsz / UKL_SIMD_WIDTH;
    uint32_t inc = ukl_sg_inclusive_sum(v, lane);
    if (lane == UKL_SIMD_WIDTH - 1u)
        scratch[sg] = inc;
    ukl_tg_barrier();
    uint32_t p = lane < groups ? scratch[lane] : 0u;
    uint32_t pin = ukl_sg_inclusive_sum(p, lane);
    UklScan r;
    r.exclusive = ukl_sg_shuffle(pin - p, sg) + inc - v;
    r.total = ukl_sg_shuffle(pin, UKL_SIMD_WIDTH - 1u);
    ukl_tg_barrier();
    return r;
}
