// ukl/lz4.h: an LZ4 block decoded by one SIMD group (decisions 0400 §5, 0336 §8, 0156 §2.1).
//
// The block's matches stay inside it (no dictionary), as a 16z piece's do. Every lane reads the same token; the lanes
// copy a literal run, or a match, `UKL_SIMD_WIDTH` bytes at a time, and a match closer than that in rounds of its
// distance, so a round reads only bytes written before it. Output goes straight to device memory: no threadgroup
// room, so no block is too long.
//
// Where output byte `p` goes is the caller's (`Out`): a struct with a method `at(p)` giving its address. A contiguous
// output is `UklLz4Contiguous`; a 16z piece's is its array's part, then its side list's part.

#pragma once
#include <ukl/simd.h>

struct UklLz4Contiguous {
    UKL_GLOBAL uint8_t* dst;
    UKL_METHOD UKL_GLOBAL uint8_t* at(uint32_t p) const { return dst + p; }
};

// An LZ4 length past its 4-bit field: bytes of 255, then one below. False when it runs past `end`.
UKL_INLINE bool ukl_lz4_length(UKL_GLOBAL const uint8_t* s, UKL_PRIVATE uint32_t& ip, uint32_t end, UKL_PRIVATE uint32_t& n)
{
    uint32_t c;
    do {
        if (ip >= end)
            return false;
        c = s[ip++];
        n += c;
    } while (c == 255u);
    return true;
}

// `n` bytes from `src` to output bytes `op..op + n`, the lanes in turn.
template <typename Out> UKL_INLINE void ukl_sg_copy_out(Out out, uint32_t op, UKL_GLOBAL const uint8_t* src, uint32_t n, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        *out.at(op + i) = src[i];
}

// `n` output bytes from `dist` back to `op..op + n` (they may overlap), in rounds a round's reads were written before.
template <typename Out> UKL_INLINE void ukl_sg_copy_back(Out out, uint32_t op, uint32_t dist, uint32_t n, uint32_t lane)
{
    uint32_t step = dist < UKL_SIMD_WIDTH ? dist : UKL_SIMD_WIDTH;
    for (uint32_t at = 0u; at < n; at += step) {
        uint32_t i = at + lane;
        if (lane < step && i < n)
            *out.at(op + i) = *out.at(op + i - dist);
        ukl_sg_barrier();
    }
}

// The block `s[0..len]` decoded to `out`: the bytes written, or `UKL_NONE` on a malformed block or one that would
// write past `cap` or read before its output's start.
template <typename Out> UKL_INLINE uint32_t ukl_lz4_decode_sg(UKL_GLOBAL const uint8_t* s, uint32_t len, Out out, uint32_t cap, uint32_t lane)
{
    uint32_t ip = 0u, op = 0u;
    while (true) {
        if (ip >= len)
            return UKL_NONE;
        uint32_t tok = s[ip++];
        uint32_t lit = tok >> 4;
        if (lit == 15u && !ukl_lz4_length(s, ip, len, lit))
            return UKL_NONE;
        if (ip + lit > len || op + lit > cap)
            return UKL_NONE;
        ukl_sg_copy_out(out, op, s + ip, lit, lane);
        ip += lit;
        op += lit;
        // The last sequence is literals alone, and ends the block.
        if (ip == len)
            return op;
        if (ip + 2u > len)
            return UKL_NONE;
        uint32_t dist = (uint32_t)s[ip] | ((uint32_t)s[ip + 1u] << 8);
        ip += 2u;
        uint32_t ml = tok & 15u;
        if (ml == 15u && !ukl_lz4_length(s, ip, len, ml))
            return UKL_NONE;
        ml += 4u;
        if (dist == 0u || dist > op || op + ml > cap)
            return UKL_NONE;
        ukl_sg_barrier();
        ukl_sg_copy_back(out, op, dist, ml, lane);
        op += ml;
    }
}
