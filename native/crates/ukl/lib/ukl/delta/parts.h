// ukl/delta/parts.h: a parts block (`rerail_delta_twins::drop`): the partitions an operation names, opened by the host,
// put where they lie, then sealed (sorted, each once) before another reads them.
#pragma once
#include <ukl/delta/op.h>

#define PART(ps, k) I64(ps, PARTS_AT + 8u * (k))

UKL_INLINE uint32_t parts_len(CBlk ps) { return (uint32_t)U64(ps, PARTS_N_AT); }
UKL_INLINE int64_t parts_get(CBlk ps, uint32_t k) { return PART(ps, k); }
UKL_INLINE void parts_put(Blk ps, uint32_t k, int64_t p) { PART(ps, k) = p; }

// The first `n` places sealed: sorted (an insertion sort, in place), each partition once. How many it lists.
UKL_INLINE uint32_t parts_seal(Blk ps, uint32_t n) {
    for (uint32_t a = 1u; a < n; a++) {
        int64_t v = PART(ps, a);
        uint32_t b = a;
        for (; b > 0u && PART(ps, b - 1u) > v; b--) PART(ps, b) = PART(ps, b - 1u);
        PART(ps, b) = v;
    }
    uint32_t k = 0u;
    for (uint32_t a = 0u; a < n; a++)
        if (k == 0u || PART(ps, a) != PART(ps, k - 1u)) PART(ps, k++) = PART(ps, a);
    U64(ps, BODY_AT) = 16u + 8u * k;
    U64(ps, PARTS_N_AT) = k;
    return k;
}
