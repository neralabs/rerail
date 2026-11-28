// ukl/delta/sort.h: a heap sort of u32s where they lie (a block's), in place, by an order its user names: what an
// operation that sorts ids by their rows' values sorts with, one item at a time. O(n log n), no room.
#pragma once
#include <ukl/delta/op.h>

// `name(c, a, n)`: the `n` u32s at `a` sorted by `LESS(c, x, y)`, a strict total order over values of type `Ctx`.
#define DELTA_SORT(name, Ctx, LESS)                                                                                 \
    UKL_INLINE void name##_down(Ctx c, CLIF_BLOCK_SPACE uint32_t* a, uint32_t i, uint32_t n) {                     \
        for (uint32_t k; (k = 2u * i + 1u) < n; i = k) {                                                           \
            if (k + 1u < n && LESS(c, a[k], a[k + 1u])) k++;                                                       \
            if (!LESS(c, a[i], a[k])) return;                                                                      \
            uint32_t x = a[i]; a[i] = a[k]; a[k] = x;                                                              \
        }                                                                                                          \
    }                                                                                                              \
    UKL_INLINE void name(Ctx c, CLIF_BLOCK_SPACE uint32_t* a, uint32_t n) {                                         \
        for (uint32_t i = n / 2u; i > 0u; i--) name##_down(c, a, i - 1u, n);                                       \
        for (uint32_t e = n; e > 1u; e--) {                                                                        \
            uint32_t x = a[0]; a[0] = a[e - 1u]; a[e - 1u] = x;                                                    \
            name##_down(c, a, 0u, e - 1u);                                                                         \
        }                                                                                                          \
    }
