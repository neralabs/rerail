// ukl/blx64.h: a 64-byte block's operations as Bitlex's tier 1 writes them (decision 0726), a thread a chunk: the
// operations of `rerail_simd8::portable` (`s::`), so tier 1's passes printed as UKL (`bitlex_lex_rust::ukl`) compute
// what its Rust computes, bit for bit.
//
// A block is eight 64-bit words, byte `i` at bits `8 * (i % 8)` of word `i / 8` (SWAR): the operations on whole
// blocks are word operations, and a byte is reached by a shift of its word, never by its address in a private array.
// The M3's back end read bytes written into a 64-byte private array back with each pair swapped (byte `i` as byte
// `i ^ 1`; the M1 Max and the CPU read them right), and registers are the GPU's own form of a block. A 16-byte table
// is read once into two words at constant offsets, each byte's entry shifted out of them (a shuffle in registers, as
// `tbl` and `pshufb`), so no load's address is made from a loaded byte (decision 0540).

#pragma once
#include <ukl/backend.h>
#include <ukl/memory.h>

typedef struct UklB64 {
    uint64_t w[8];
} UklB64;

#define UKL_B64_ONES 0x0101010101010101ull
#define UKL_B64_LOW7 0x7f7f7f7f7f7f7f7full
#define UKL_B64_HIGH 0x8080808080808080ull
#define UKL_B64_NIBBLE 0x0f0f0f0f0f0f0f0full

// Byte `k` (below 8) of word `x`.
UKL_INLINE uint32_t ukl_b64_at(uint64_t x, uint32_t k) { return (uint32_t)(x >> (8u * k)) & 255u; }

// The block at `p`.
UKL_INLINE UklB64 ukl_b64_load(UKL_GLOBAL const uint8_t* p)
{
    UklB64 r;
    for (uint32_t j = 0u; j < 8u; j++)
        r.w[j & 7u] = ukl_ld_u64(p, 8u * j);
    return r;
}

// The block of a table's 64 bytes.
UKL_INLINE UklB64 ukl_b64_load_t(UKL_TABLE_SPACE const uint8_t* p)
{
    UklB64 r;
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t x = 0u;
        for (uint32_t k = 0u; k < 8u; k++)
            x |= (uint64_t)p[8u * j + k] << (8u * k);
        r.w[j & 7u] = x;
    }
    return r;
}

// Each byte `x`.
UKL_INLINE UklB64 ukl_b64_splat(uint32_t x)
{
    UklB64 r;
    for (uint32_t j = 0u; j < 8u; j++)
        r.w[j & 7u] = (uint64_t)(x & 255u) * UKL_B64_ONES;
    return r;
}

UKL_INLINE UklB64 ukl_b64_and(UklB64 a, UklB64 b)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] &= b.w[j & 7u];
    return a;
}

UKL_INLINE UklB64 ukl_b64_or(UklB64 a, UklB64 b)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] |= b.w[j & 7u];
    return a;
}

UKL_INLINE UklB64 ukl_b64_xor(UklB64 a, UklB64 b)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] ^= b.w[j & 7u];
    return a;
}

UKL_INLINE UklB64 ukl_b64_high_nibbles(UklB64 a)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] = (a.w[j & 7u] >> 4) & UKL_B64_NIBBLE;
    return a;
}

UKL_INLINE UklB64 ukl_b64_low_nibbles(UklB64 a)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] &= UKL_B64_NIBBLE;
    return a;
}

// A 16-byte table in two words, its bytes little-endian.
typedef struct UklNib16 {
    uint64_t w0;
    uint64_t w1;
} UklNib16;

UKL_INLINE UklNib16 ukl_nib16(UKL_TABLE_SPACE const uint8_t* t)
{
    UklNib16 r;
    r.w0 = 0u;
    r.w1 = 0u;
    for (uint32_t k = 0u; k < 8u; k++) {
        r.w0 |= (uint64_t)t[k] << (8u * k);
        r.w1 |= (uint64_t)t[8u + k] << (8u * k);
    }
    return r;
}

// Entry `x` (below 16) of a table in words.
UKL_INLINE uint32_t ukl_nib16_at(UklNib16 t, uint32_t x) { return ukl_b64_at((x & 8u) != 0u ? t.w1 : t.w0, x & 7u); }

// Each byte `t[i]` for its value `i` (16 or more: 0, as a shuffle's).
UKL_INLINE UklB64 ukl_b64_lookup_16(UklB64 a, UKL_TABLE_SPACE const uint8_t* t)
{
    UklNib16 n = ukl_nib16(t);
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t x = a.w[j & 7u], r = 0u;
        for (uint32_t k = 0u; k < 8u; k++) {
            uint32_t b = ukl_b64_at(x, k);
            r |= (uint64_t)(b < 16u ? ukl_nib16_at(n, b) : 0u) << (8u * k);
        }
        a.w[j & 7u] = r;
    }
    return a;
}

// Each byte's classes by the nibble tables: `lo[b & 15] & hi[b >> 4]`.
UKL_INLINE UklB64 ukl_b64_classify(UklB64 a, UKL_TABLE_SPACE const uint8_t* lo, UKL_TABLE_SPACE const uint8_t* hi)
{
    UklNib16 l = ukl_nib16(lo), h = ukl_nib16(hi);
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t x = a.w[j & 7u], r = 0u;
        for (uint32_t k = 0u; k < 8u; k++) {
            uint32_t b = ukl_b64_at(x, k);
            r |= (uint64_t)(ukl_nib16_at(l, b & 15u) & ukl_nib16_at(h, b >> 4)) << (8u * k);
        }
        a.w[j & 7u] = r;
    }
    return a;
}

// Each byte's high bit where it is not 0, the rest 0 (no carry leaves a byte: its low 7 bits plus 0x7f is at most
// 0xfe).
UKL_INLINE uint64_t ukl_b64_nonzero_high(uint64_t x) { return (((x & UKL_B64_LOW7) + UKL_B64_LOW7) | x) & UKL_B64_HIGH; }

// 0xff where `a & b` is not 0, else 0.
UKL_INLINE UklB64 ukl_b64_test(UklB64 a, UklB64 b)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] = (ukl_b64_nonzero_high(a.w[j & 7u] & b.w[j & 7u]) >> 7) * 255u;
    return a;
}

// 0xff where a byte is `x` or above.
UKL_INLINE UklB64 ukl_b64_ge(UklB64 a, uint32_t x)
{
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t v = a.w[j & 7u], r = 0u;
        for (uint32_t k = 0u; k < 8u; k++)
            r |= (uint64_t)(ukl_b64_at(v, k) >= x ? 255u : 0u) << (8u * k);
        a.w[j & 7u] = r;
    }
    return a;
}

// Each byte's `k`-th before it (1 to 3), the bytes before the block from `before`.
UKL_INLINE UklB64 ukl_b64_prev(UklB64 a, UklB64 before, uint32_t k)
{
    UklB64 r;
    uint32_t s = 8u * k;
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t below = j == 0u ? before.w[7] : a.w[(j - 1u) & 7u];
        r.w[j & 7u] = (a.w[j & 7u] << s) | (below >> (64u - s));
    }
    return r;
}

// Byte by byte, `a - b`, 0 where `b` is more.
UKL_INLINE UklB64 ukl_b64_gt_bits(UklB64 a, UklB64 b)
{
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t x = a.w[j & 7u], y = b.w[j & 7u], r = 0u;
        for (uint32_t k = 0u; k < 8u; k++) {
            uint32_t p = ukl_b64_at(x, k), q = ukl_b64_at(y, k);
            r |= (uint64_t)(p > q ? p - q : 0u) << (8u * k);
        }
        a.w[j & 7u] = r;
    }
    return a;
}

// Bit `8 * j + k` from each byte's high bit (`high`: only high bits set).
UKL_INLINE uint64_t ukl_b64_gather_high(UklB64 high)
{
    uint64_t m = 0u;
    for (uint32_t j = 0u; j < 8u; j++) {
        uint64_t x = high.w[j & 7u];
        for (uint32_t k = 0u; k < 8u; k++)
            m |= ((x >> (8u * k + 7u)) & 1u) << (8u * j + k);
    }
    return m;
}

// Bit `i`: byte `i` is `x`.
UKL_INLINE uint64_t ukl_b64_eq_mask(UklB64 a, uint32_t x)
{
    uint64_t s = (uint64_t)(x & 255u) * UKL_B64_ONES;
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] = ukl_b64_nonzero_high(a.w[j & 7u] ^ s) ^ UKL_B64_HIGH;
    return ukl_b64_gather_high(a);
}

// Bit `i`: byte `i`'s high bit.
UKL_INLINE uint64_t ukl_b64_to_bitmask(UklB64 a)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] &= UKL_B64_HIGH;
    return ukl_b64_gather_high(a);
}

// Bit `i`: byte `i` is not 0.
UKL_INLINE uint64_t ukl_b64_nonzero(UklB64 a)
{
    for (uint32_t j = 0u; j < 8u; j++)
        a.w[j & 7u] = ukl_b64_nonzero_high(a.w[j & 7u]);
    return ukl_b64_gather_high(a);
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
