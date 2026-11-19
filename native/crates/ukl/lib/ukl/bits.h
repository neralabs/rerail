// ukl/bits.h: bits within a word, and bitsets over a buffer's words (decision 0400 §5). Thread level.

#pragma once
#include <ukl/backend.h>

// "No such position": a search that finds nothing.
#define UKL_NONE 0xffffffffu

// The lesser and the greater of two values of one type (the backend's `min` and `max`, which not every backend has
// for every width).
#define UKL_DEFINE_MIN_MAX(T)                                                                                   \
    UKL_INLINE UKL_OVERLOAD T ukl_min(T a, T b) { return a < b ? a : b; }                                       \
    UKL_INLINE UKL_OVERLOAD T ukl_max(T a, T b) { return a < b ? b : a; }
UKL_DEFINE_MIN_MAX(uint32_t)
UKL_DEFINE_MIN_MAX(int32_t)
UKL_DEFINE_MIN_MAX(uint64_t)
UKL_DEFINE_MIN_MAX(int64_t)
UKL_DEFINE_MIN_MAX(float)

// The `n` lowest bits set (all of them from 32 or 64 up).
UKL_INLINE uint32_t ukl_mask32(uint32_t n) { return n >= 32u ? 0xffffffffu : (1u << n) - 1u; }
UKL_INLINE uint64_t ukl_mask64(uint32_t n) { return n >= 64u ? ~(uint64_t)0 : (((uint64_t)1) << n) - 1u; }

// `n` bits of `v` from bit `off` up (zero past the word's end).
UKL_INLINE uint32_t ukl_bits32(uint32_t v, uint32_t off, uint32_t n) { return off >= 32u ? 0u : (v >> off) & ukl_mask32(n); }
UKL_INLINE uint64_t ukl_bits64(uint64_t v, uint32_t off, uint32_t n) { return off >= 64u ? 0u : (v >> off) & ukl_mask64(n); }

// Rank: the set bits of `w` below bit `i`.
UKL_INLINE uint32_t ukl_rank32(uint32_t w, uint32_t i) { return ukl_popcount32(w & ukl_mask32(i)); }
UKL_INLINE uint32_t ukl_rank64(uint64_t w, uint32_t i) { return ukl_popcount64(w & ukl_mask64(i)); }

// Select: the position of set bit `k` of `w` (from 0, lowest first), or the word's width when it has no more.
// A halving search on counts: five steps for 32 bits, six for 64, the same on every input.
UKL_INLINE uint32_t ukl_select32(uint32_t w, uint32_t k)
{
    if (k >= ukl_popcount32(w))
        return 32u;
    uint32_t pos = 0u;
    for (uint32_t span = 16u; span > 0u; span >>= 1) {
        uint32_t c = ukl_popcount32(w & ukl_mask32(span));
        if (k >= c) {
            k -= c;
            w >>= span;
            pos += span;
        }
    }
    return pos;
}

UKL_INLINE uint32_t ukl_select64(uint64_t w, uint32_t k)
{
    if (k >= ukl_popcount64(w))
        return 64u;
    uint32_t pos = 0u;
    for (uint32_t span = 32u; span > 0u; span >>= 1) {
        uint32_t c = ukl_popcount64(w & ukl_mask64(span));
        if (k >= c) {
            k -= c;
            w >>= span;
            pos += span;
        }
    }
    return pos;
}

// A bitset: bit `i` is bit `i % 32` of word `i / 32`.
UKL_INLINE bool ukl_bitset_test(UKL_GLOBAL const uint32_t* words, uint32_t i) { return ((words[i >> 5] >> (i & 31u)) & 1u) != 0u; }

// Sets bit `i` (atomically: other threads set bits of the same word); whether it was set before.
UKL_INLINE bool ukl_bitset_set(UKL_GLOBAL uint32_t* words, uint32_t i)
{
    uint32_t bit = 1u << (i & 31u);
    return (ukl_atomic_or(&words[i >> 5], bit) & bit) != 0u;
}

// The first set bit at or after `from`, below `nbits`; `UKL_NONE` when there is none. A word at a time.
UKL_INLINE uint32_t ukl_bitset_next(UKL_GLOBAL const uint32_t* words, uint32_t nbits, uint32_t from)
{
    if (from >= nbits)
        return UKL_NONE;
    uint32_t wi = from >> 5;
    uint32_t w = words[wi] & ~ukl_mask32(from & 31u);
    uint32_t last = (nbits - 1u) >> 5;
    while (w == 0u) {
        if (wi == last)
            return UKL_NONE;
        w = words[++wi];
    }
    uint32_t i = (wi << 5) + ukl_ctz32(w);
    return i < nbits ? i : UKL_NONE;
}
