// ukl/delta/runs.h: a δ runs block (`rerail_delta_core::runs`): a run's `(row, diff)` pairs, column by column, then
// the diffs. A writer claims its places with one atomic add, then writes its cells. A claim past the capacity writes
// nothing and counts what was needed, and the driver runs the step again at that size.
#pragma once
#include <ukl/delta/op.h>

// Claims `n` places at once: the first, or NONE when they do not all fit (the claims past the capacity counted).
UKL_INLINE uint32_t runs_claim(Blk r, uint32_t n) {
    uint32_t cap = U32(r, RUNS_CAP_AT);
    uint32_t old = ukl_atomic_add((UKL_GLOBAL uint32_t*)(r + RUNS_CLAIMED_AT), n);
    if (old + n > cap) {
        ukl_atomic_add((UKL_GLOBAL uint32_t*)(r + RUNS_NEEDED_AT), old + n - (old > cap ? old : cap));
        return NONE;
    }
    return old;
}

// Place `k`'s cell in column `c`, at the column's width.
UKL_INLINE void runs_set(Blk r, uint32_t k, uint32_t c, int64_t v) {
    uint64_t at = U64(r, RUNS_COL_AT + 8u * c);
    if (r[RUNS_TYPES_AT + c] != 0u) I64(r, at + 8u * k) = v;
    else I32(r, at + 4u * k) = (int32_t)v;
}

// Place `k`'s diff.
UKL_INLINE void runs_diff(Blk r, uint32_t k, int32_t d) { I32(r, U64(r, RUNS_DIFF_AT) + 4u * k) = d; }

// Its rows: the claims within its capacity.
UKL_INLINE uint32_t runs_len(CBlk r) {
    uint32_t n = U32(r, RUNS_CLAIMED_AT), cap = U32(r, RUNS_CAP_AT);
    return n < cap ? n : cap;
}

// Row `k`'s cell in column `c`, and its diff.
UKL_INLINE int64_t runs_get(CBlk r, uint32_t k, uint32_t c) {
    uint64_t at = U64(r, RUNS_COL_AT + 8u * c);
    return r[RUNS_TYPES_AT + c] != 0u ? I64(r, at + 8u * k) : (int64_t)I32(r, at + 4u * k);
}
UKL_INLINE int32_t runs_sign(CBlk r, uint32_t k) { return I32(r, U64(r, RUNS_DIFF_AT) + 4u * k); }
