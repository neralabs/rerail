// ukl/lz4.h: an LZ4 block decoded by one SIMD group (decisions 0440 §5, 0336 §8, 0156 §2.1).
//
// The block's matches stay inside it (no dictionary), as a 16z piece's do. Every lane reads the same token; the lanes
// copy a literal run, or a match, `UKL_SIMD_WIDTH` bytes at a time, and a match closer than that in rounds of its
// distance, so a round reads only bytes written before it. Output goes straight to device memory: no threadgroup
// room, so no block is too long.
//
// Where output byte `p` goes is the caller's: `ukl_lz4_decode_sg` writes a contiguous output (`UKL_GLOBAL uint8_t*`);
// another (a 16z piece's: its array's part, then its side list's part) is the same decoder instantiated over its own
// output type and address (`ukl/body/lz4_decode.h`, as below).

#pragma once
#include <ukl/simd.h>

// An LZ4 length past its 4-bit field: bytes of 255, then one below. False when it runs past `end`.
UKL_INLINE bool ukl_lz4_length(UKL_GLOBAL const uint8_t* s, UKL_PRIVATE uint32_t* ip, uint32_t end, UKL_PRIVATE uint32_t* n)
{
    uint32_t c;
    do {
        if (*ip >= end)
            return false;
        c = s[(*ip)++];
        *n += c;
    } while (c == 255u);
    return true;
}

// The contiguous output's decoder: `ukl_lz4_decode_sg(s, len, out, cap, lane)`.
#define UKL_LZ4_NAME(f) f
#define UKL_LZ4_OUT UKL_GLOBAL uint8_t*
#define UKL_LZ4_AT(out, p) ((out) + (p))
#include <ukl/body/lz4_decode.h>
