// ukl/blx64.h: a 64-byte block's operations as Bitlex's tier 1 writes them (decision 0726), a thread a chunk: the
// operations of `rerail_simd8::portable` (`s::`), byte by byte, so tier 1's passes printed as UKL
// (`bitlex_lex_rust::ukl`) compute what its Rust computes, bit for bit.

#pragma once
#include <ukl/backend.h>

typedef struct UklB64 {
    uint8_t b[64];
} UklB64;

// The block at `p`.
UKL_INLINE UklB64 ukl_b64_load(UKL_GLOBAL const uint8_t* p)
{
    UklB64 r;
    for (uint32_t i = 0u; i < 64u; i++)
        r.b[i] = p[i];
    return r;
}

// The block of a table's 64 bytes.
UKL_INLINE UklB64 ukl_b64_load_t(UKL_TABLE_SPACE const uint8_t* p)
{
    UklB64 r;
    for (uint32_t i = 0u; i < 64u; i++)
        r.b[i] = p[i];
    return r;
}

// Each byte `x`.
UKL_INLINE UklB64 ukl_b64_splat(uint32_t x)
{
    UklB64 r;
    for (uint32_t i = 0u; i < 64u; i++)
        r.b[i] = (uint8_t)x;
    return r;
}

UKL_INLINE UklB64 ukl_b64_and(UklB64 a, UklB64 b)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] &= b.b[i];
    return a;
}

UKL_INLINE UklB64 ukl_b64_or(UklB64 a, UklB64 b)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] |= b.b[i];
    return a;
}

UKL_INLINE UklB64 ukl_b64_xor(UklB64 a, UklB64 b)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] ^= b.b[i];
    return a;
}

UKL_INLINE UklB64 ukl_b64_high_nibbles(UklB64 a)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = (uint8_t)(a.b[i] >> 4);
    return a;
}

UKL_INLINE UklB64 ukl_b64_low_nibbles(UklB64 a)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = (uint8_t)(a.b[i] & 15u);
    return a;
}

// Each byte `t[i]` for its value `i` (16 or more: 0, as a shuffle's).
UKL_INLINE UklB64 ukl_b64_lookup_16(UklB64 a, UKL_TABLE_SPACE const uint8_t* t)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = a.b[i] < 16u ? t[a.b[i]] : (uint8_t)0u;
    return a;
}

// Each byte's classes by the nibble tables: `lo[b & 15] & hi[b >> 4]`.
UKL_INLINE UklB64 ukl_b64_classify(UklB64 a, UKL_TABLE_SPACE const uint8_t* lo, UKL_TABLE_SPACE const uint8_t* hi)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = (uint8_t)(lo[a.b[i] & 15u] & hi[a.b[i] >> 4]);
    return a;
}

// 0xff where `a & b` is not 0, else 0.
UKL_INLINE UklB64 ukl_b64_test(UklB64 a, UklB64 b)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = (a.b[i] & b.b[i]) != 0u ? (uint8_t)0xffu : (uint8_t)0u;
    return a;
}

// 0xff where a byte is `x` or above.
UKL_INLINE UklB64 ukl_b64_ge(UklB64 a, uint32_t x)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = a.b[i] >= x ? (uint8_t)0xffu : (uint8_t)0u;
    return a;
}

// Each byte's `k`-th before it (1 to 3), the bytes before the block from `before`.
UKL_INLINE UklB64 ukl_b64_prev(UklB64 a, UklB64 before, uint32_t k)
{
    UklB64 r;
    for (uint32_t i = 0u; i < 64u; i++)
        r.b[i] = i >= k ? a.b[i - k] : before.b[64u - k + i];
    return r;
}

// Byte by byte, `a - b`, 0 where `b` is more.
UKL_INLINE UklB64 ukl_b64_gt_bits(UklB64 a, UklB64 b)
{
    for (uint32_t i = 0u; i < 64u; i++)
        a.b[i] = a.b[i] > b.b[i] ? (uint8_t)(a.b[i] - b.b[i]) : (uint8_t)0u;
    return a;
}

// Bit `i`: byte `i` is `x`.
UKL_INLINE uint64_t ukl_b64_eq_mask(UklB64 a, uint32_t x)
{
    uint64_t m = 0ull;
    for (uint32_t i = 0u; i < 64u; i++)
        m |= (uint64_t)(a.b[i] == x) << i;
    return m;
}

// Bit `i`: byte `i`'s high bit.
UKL_INLINE uint64_t ukl_b64_to_bitmask(UklB64 a)
{
    uint64_t m = 0ull;
    for (uint32_t i = 0u; i < 64u; i++)
        m |= (uint64_t)(a.b[i] >> 7) << i;
    return m;
}

// Bit `i`: byte `i` is not 0.
UKL_INLINE uint64_t ukl_b64_nonzero(UklB64 a)
{
    uint64_t m = 0ull;
    for (uint32_t i = 0u; i < 64u; i++)
        m |= (uint64_t)(a.b[i] != 0u) << i;
    return m;
}

// The 32 bytes at `p`: the bits of the bytes that are `x` (`*a`) and of those that are `y` (`*b`) (`s::marks32`).
UKL_INLINE void ukl_b32_marks(UKL_GLOBAL const uint8_t* p, uint32_t x, uint32_t y, UKL_PRIVATE uint32_t* a, UKL_PRIVATE uint32_t* b)
{
    uint32_t ma = 0u, mb = 0u;
    for (uint32_t i = 0u; i < 32u; i++) {
        ma |= (uint32_t)(p[i] == x) << i;
        mb |= (uint32_t)(p[i] == y) << i;
    }
    *a = ma;
    *b = mb;
}
