// ukl/lz4_encode.h: an LZ4 block encoded by one SIMD group, greedily (decision 0726, S6): LZ4's fast parse, the
// first match the walk meets taken, its candidates found and its length measured by the group's lanes at once.
//
// The walk is the group's, in step. From the walk's place `p` the lanes take the window `p..p + UKL_SIMD_WIDTH`, a
// position a lane, and each its 4-byte word's hash. A position's candidate is the latest earlier one with its hash:
// the nearest lane below with an equal hash (`ukl_sg_match`), else the table's. The first lane whose candidate holds
// its word, within 64 KB, starts the match (a ballot): every position up to it goes in the table, the literals before
// it and the match go out, and the walk goes on past the match. A window with none puts all its positions in the
// table and goes on past it. So the parse is the serial one (each position looked up, then put in the table, the
// first match taken, the positions inside a match skipped) at every width, on every backend, byte for byte.
//
// A match's length is measured `UKL_SIMD_WIDTH` bytes a round (a byte a lane, the first that differs by a ballot), and
// literals are copied a byte a lane. LZ4's limits hold, so LZ4's own decoder takes the block: no match starts in the
// last 12 bytes (`MFLIMIT`), and the last 5 are literals.
//
// The table is the caller's threadgroup memory, `UKL_LZ4E_TABLE` words, set here before it is read.
//
// A block's bytes may lie in two places (`ukl_lz4_encode2_sg`: a 16z piece is its array's column, then its side list's
// part, each read where it lies in the format 16 tape), and a run may only count (`write` false: the length, nothing
// written), so pieces are placed by a scan of their lengths and then written where they go.

#pragma once
#include <ukl/simd.h>

#ifndef UKL_LZ4E_HASH_LOG
#define UKL_LZ4E_HASH_LOG 12u
#endif
#define UKL_LZ4E_TABLE (1u << UKL_LZ4E_HASH_LOG)
#define UKL_LZ4E_MFLIMIT 12u
#define UKL_LZ4E_LASTLITERALS 5u
#define UKL_LZ4E_DISTANCE_MAX 65535u

// The most bytes a block of `n` bytes encodes to (LZ4's `LZ4_compressBound`).
UKL_INLINE uint32_t ukl_lz4_bound(uint32_t n) { return n + n / 255u + 16u; }

// A block's bytes: `an` at `a`, then the rest at `b`.
typedef struct UklLz4Spans {
    UKL_GLOBAL const uint8_t* a;
    UKL_GLOBAL const uint8_t* b;
    uint32_t an;
} UklLz4Spans;

// Byte `p`.
UKL_INLINE uint8_t ukl_lz4e_byte(UklLz4Spans s, uint32_t p) { return p < s.an ? s.a[p] : s.b[p - s.an]; }

// The 4 bytes at `p`, little-endian.
UKL_INLINE uint32_t ukl_lz4e_word(UklLz4Spans s, uint32_t p)
{
    return (uint32_t)ukl_lz4e_byte(s, p) | ((uint32_t)ukl_lz4e_byte(s, p + 1u) << 8) | ((uint32_t)ukl_lz4e_byte(s, p + 2u) << 16) |
           ((uint32_t)ukl_lz4e_byte(s, p + 3u) << 24);
}

// LZ4's hash of a word (Knuth's multiplier), `UKL_LZ4E_HASH_LOG` bits.
UKL_INLINE uint32_t ukl_lz4e_hash(uint32_t x) { return (x * 2654435761u) >> (32u - UKL_LZ4E_HASH_LOG); }

// The highest lane of a mask that has one.
UKL_INLINE uint32_t ukl_lz4e_highest(ukl_mask_t m) { return 63u - ukl_clz64((uint64_t)m); }

// LZ4's length past its 4-bit field, `len` (15 or more taken off): bytes of 255, then the rest, at `op`; the next byte.
UKL_INLINE uint32_t ukl_lz4e_length(UKL_GLOBAL uint8_t* out, uint32_t op, uint32_t len, bool write, uint32_t lane)
{
    uint32_t k = len / 255u;
    if (write) {
        for (uint32_t i = lane; i < k; i += UKL_SIMD_WIDTH)
            out[op + i] = 255u;
        if (lane == 0u)
            out[op + k] = (uint8_t)(len % 255u);
    }
    return op + k + 1u;
}

// A sequence at `op`: the `lit` literals from byte `anchor`, then (where `ml` is not 0) a match of `ml` bytes `dist`
// back. The next byte, or `UKL_NONE` past `cap`; nothing written where `write` is false.
UKL_INLINE uint32_t ukl_lz4e_sequence(UklLz4Spans s, uint32_t anchor, uint32_t lit, uint32_t dist, uint32_t ml, UKL_GLOBAL uint8_t* out,
                                      uint32_t op, uint32_t cap, bool write, uint32_t lane)
{
    // Its bytes exactly, so a block given the room its count found fits it.
    uint32_t need = 1u + (lit >= 15u ? (lit - 15u) / 255u + 1u : 0u) + lit;
    if (ml != 0u)
        need += 2u + (ml - 4u >= 15u ? (ml - 19u) / 255u + 1u : 0u);
    if (op + need > cap)
        return UKL_NONE;
    uint32_t token = op++;
    uint32_t tok = (lit >= 15u ? 15u : lit) << 4;
    if (lit >= 15u)
        op = ukl_lz4e_length(out, op, lit - 15u, write, lane);
    if (write)
        for (uint32_t i = lane; i < lit; i += UKL_SIMD_WIDTH)
            out[op + i] = ukl_lz4e_byte(s, anchor + i);
    op += lit;
    if (ml != 0u) {
        if (write && lane == 0u) {
            out[op] = (uint8_t)(dist & 255u);
            out[op + 1u] = (uint8_t)(dist >> 8);
        }
        op += 2u;
        uint32_t l = ml - 4u;
        tok |= l >= 15u ? 15u : l;
        if (l >= 15u)
            op = ukl_lz4e_length(out, op, l - 15u, write, lane);
    }
    if (write && lane == 0u)
        out[token] = (uint8_t)tok;
    return op;
}

// The block of `n` bytes `s` as one LZ4 block at `out` (only counted where `write` is false): its length, or
// `UKL_NONE` where it would pass `cap` (`ukl_lz4_bound(n)` never does). Every lane of the group calls it, `table` the
// group's `UKL_LZ4E_TABLE` words.
UKL_INLINE uint32_t ukl_lz4_encode2_sg(UklLz4Spans s, uint32_t n, UKL_GLOBAL uint8_t* out, uint32_t cap, bool write, UKL_LOCAL uint32_t* table,
                                       uint32_t lane __UKL_PARAMS)
{
    // The table's positions plus one; 0 none.
    for (uint32_t i = lane; i < UKL_LZ4E_TABLE; i += UKL_SIMD_WIDTH)
        table[i] = 0u;
    ukl_sg_barrier();
    uint32_t p = 0u, anchor = 0u, op = 0u;
    if (n > UKL_LZ4E_MFLIMIT) {
        uint32_t mflimit = n - UKL_LZ4E_MFLIMIT, matchlimit = n - UKL_LZ4E_LASTLITERALS;
        while (p <= mflimit) {
            uint32_t q = p + lane;
            bool in = q <= mflimit;
            uint32_t w = in ? ukl_lz4e_word(s, q) : 0u;
            // A lane past the limit has a hash of its own, which no lane below it shares.
            uint32_t h = in ? ukl_lz4e_hash(w) : UKL_LZ4E_TABLE + lane;
            ukl_mask_t same = ukl_sg_match(h __UKL_ARGS);
            ukl_mask_t below = same & ukl_mask_below(lane);
            uint32_t cand = below != 0u ? p + ukl_lz4e_highest(below) + 1u : (in ? table[h] : 0u);
            bool hit = in && cand != 0u && q - (cand - 1u) <= UKL_LZ4E_DISTANCE_MAX && ukl_lz4e_word(s, cand - 1u) == w;
            uint32_t f = ukl_sg_first(hit __UKL_ARGS);
            ukl_sg_barrier();
            // Every position up to the match's start (all the window's where none) in the table: each hash's last.
            uint32_t last = f == UKL_NONE ? UKL_SIMD_WIDTH - 1u : f;
            if (in && lane <= last && (same & ukl_mask_below(last + 1u)) >> lane == 1u)
                table[h] = q + 1u;
            ukl_sg_barrier();
            if (f == UKL_NONE) {
                p += UKL_SIMD_WIDTH;
                continue;
            }
            // The match at `p + f`, from its candidate: 4 bytes known, the rest a round of bytes at a time.
            uint32_t at = p + f;
            uint32_t from = ukl_sg_broadcast(cand, f __UKL_ARGS) - 1u;
            uint32_t ml = 4u;
            while (true) {
                uint32_t k = ml + lane;
                bool same_byte = at + k < matchlimit && ukl_lz4e_byte(s, from + k) == ukl_lz4e_byte(s, at + k);
                uint32_t miss = ukl_sg_first(!same_byte __UKL_ARGS);
                if (miss == UKL_NONE) {
                    ml += UKL_SIMD_WIDTH;
                    continue;
                }
                ml += miss;
                break;
            }
            op = ukl_lz4e_sequence(s, anchor, at - anchor, at - from, ml, out, op, cap, write, lane);
            if (op == UKL_NONE)
                return UKL_NONE;
            p = at + ml;
            anchor = p;
        }
    }
    // The last literals.
    op = ukl_lz4e_sequence(s, anchor, n - anchor, 0u, 0u, out, op, cap, write, lane);
    ukl_sg_barrier();
    return op;
}

// The block `s[0..n]` as one LZ4 block at `out`, as `ukl_lz4_encode2_sg`.
UKL_INLINE uint32_t ukl_lz4_encode_sg(UKL_GLOBAL const uint8_t* s, uint32_t n, UKL_GLOBAL uint8_t* out, uint32_t cap, UKL_LOCAL uint32_t* table,
                                      uint32_t lane __UKL_PARAMS)
{
    UklLz4Spans sp;
    sp.a = s;
    sp.b = s;
    sp.an = n;
    return ukl_lz4_encode2_sg(sp, n, out, cap, true, table, lane __UKL_ARGS);
}
