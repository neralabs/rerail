// ukl/z16_pieces.h: a finished format 16 tape's 16z pieces read where they lie (decision 0726, S6): its placement
// from its header, each piece's two spans (block `j / 4`, array `j % 4`: its column at the block's rows, then its side
// list's part), and the copies a group makes of them. 16z by phases (`<ukl/t2_phases.h>`'s Z) stores them.

#pragma once
#include <ukl/lz4_encode.h>

// A block's rows, a checkpoint's, the 16z head's bytes and where its fields start, the fewest bytes offered to LZ4,
// the tail's runs.
#define UKL_Z16_Z 1024u
#define UKL_Z16_K 8u
#define UKL_Z16_HEAD 120u
#define UKL_Z16_HZ 88u
#define UKL_Z16_LZ4_MIN 16u
#define UKL_Z16_COPY 4096u

// The tape's placement, from its header (`f16::header`).
typedef struct UklZ16P {
    uint32_t n, count, cw, stride, blocks, lines, end;
    uint32_t w[3];
    uint32_t side[4];
    uint32_t parts[4];
} UklZ16P;

UKL_INLINE uint32_t ukl_z16_u32(UKL_GLOBAL const uint8_t* t, uint32_t at)
{
    return (uint32_t)t[at] | ((uint32_t)t[at + 1u] << 8) | ((uint32_t)t[at + 2u] << 16) | ((uint32_t)t[at + 3u] << 24);
}

UKL_INLINE UklZ16P ukl_z16_placement(UKL_GLOBAL const uint8_t* t)
{
    UklZ16P p;
    p.n = ukl_z16_u32(t, 24u);
    p.count = (p.n + UKL_Z16_Z - 1u) / UKL_Z16_Z;
    p.cw = t[10];
    for (uint32_t k = 0u; k < 3u; k++)
        p.w[k] = t[36u + k];
    p.stride = ukl_z16_u32(t, 40u);
    p.blocks = ukl_z16_u32(t, 44u);
    for (uint32_t k = 0u; k < 4u; k++) {
        p.side[k] = ukl_z16_u32(t, 48u + 4u * k);
        p.parts[k] = ukl_z16_u32(t, 64u + 4u * k);
    }
    p.lines = ukl_z16_u32(t, 80u);
    p.end = ukl_z16_u32(t, 84u);
    return p;
}

// A row's bytes before column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints); column `q`'s cell.
UKL_INLINE uint32_t ukl_z16_before(UKL_PRIVATE const UklZ16P* p, uint32_t r)
{
    return r == 0u ? 0u : r == 1u ? 1u : r == 2u ? 1u + p->w[0] : r == 3u ? 1u + p->w[0] + p->w[1] : 1u + p->w[0] + p->w[1] + p->w[2];
}
UKL_INLINE uint32_t ukl_z16_cell(UKL_PRIVATE const UklZ16P* p, uint32_t q) { return q == 0u ? 1u : p->w[q - 1u]; }
UKL_INLINE uint32_t ukl_z16_rows(UKL_PRIVATE const UklZ16P* p, uint32_t b) { return p->n - b * UKL_Z16_Z < UKL_Z16_Z ? p->n - b * UKL_Z16_Z : UKL_Z16_Z; }
UKL_INLINE uint32_t ukl_z16_column(UKL_PRIVATE const UklZ16P* p, uint32_t b, uint32_t r)
{
    return p->blocks + b * p->stride + ukl_z16_rows(p, b) * ukl_z16_before(p, r);
}
UKL_INLINE uint32_t ukl_z16_part(UKL_GLOBAL const uint8_t* t, UKL_PRIVATE const UklZ16P* p, uint32_t q, uint32_t j)
{
    return ukl_z16_u32(t, p->parts[q] + 4u * j);
}

// Piece `j`'s bytes where they lie, and their number.
UKL_INLINE UklLz4Spans ukl_z16_piece(UKL_GLOBAL const uint8_t* t, UKL_PRIVATE const UklZ16P* p, uint32_t j, UKL_PRIVATE uint32_t* raw)
{
    uint32_t b = j / 4u, q = j % 4u;
    UklLz4Spans s;
    s.a = t + ukl_z16_column(p, b, q);
    s.an = ukl_z16_rows(p, b) * ukl_z16_cell(p, q);
    uint32_t from = ukl_z16_part(t, p, q, b);
    s.b = t + p->side[q] + from;
    *raw = s.an + ukl_z16_part(t, p, q, b + 1u) - from;
    return s;
}

// `n` bytes from `from` (two spans, as a piece's) to `to`, a byte a lane.
UKL_INLINE void ukl_z16_copy(UklLz4Spans s, uint32_t n, UKL_GLOBAL uint8_t* to, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        to[i] = ukl_lz4e_byte(s, i);
}
UKL_INLINE void ukl_z16_copy1(UKL_GLOBAL const uint8_t* from, uint32_t n, UKL_GLOBAL uint8_t* to, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        to[i] = from[i];
}
UKL_INLINE void ukl_z16_zeros(UKL_GLOBAL uint8_t* to, uint32_t n, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        to[i] = 0u;
}
UKL_INLINE void ukl_z16_put32(UKL_GLOBAL uint8_t* to, uint32_t v)
{
    for (uint32_t k = 0u; k < 4u; k++)
        to[k] = (uint8_t)(v >> (8u * k));
}

UKL_INLINE uint32_t ukl_z16_round4(uint32_t x) { return (x + 3u) & ~3u; }
