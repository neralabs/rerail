// ukl/body/tape_f16.h: format 16's reads (`rerail_tape_format::f16`, decision 0434), written once (decision 0576) and
// included twice: over the tape in place (`<ukl/reads/tape_f16.h>`: `clif_f16_*`) and over a 16z block decoded into a
// cache entry (`<ukl/reads/tape_f16z.h>`: `clif_f16b_*`, the block's own node numbers). Included with, defined first:
// - `CLIF_16(name)`: the instance's name of each read;
// - `CLIF_16_ARGS`, `CLIF_16_PASS`: the reads' leading parameters, and those passed on;
// - the body's two helpers, `CLIF_16(view)(.., a)` (the view's u32 at byte `a`) and `CLIF_16(ld)(.., base, off, w)`
//   (the `w` bytes at byte `base + off`): every byte the body reads is read by them.
// The macros are undefined at the end. Not `#pragma once`.

#if !defined(CLIF_16) || !defined(CLIF_16_ARGS) || !defined(CLIF_16_PASS)
#error "ukl/body/tape_f16.h needs CLIF_16, CLIF_16_ARGS and CLIF_16_PASS (ukl/reads/tape_f16.h, tape_f16z.h)"
#endif

// Width `k` (a nibble of the view's widths), and column `k`'s offset factor.
UKL_INLINE uint32_t CLIF_16(width)(CLIF_16_ARGS, uint32_t k) { return (CLIF_16(view)(CLIF_16_PASS, CLIF_T_WIDTHS) >> (4u * k)) & 15u; }
UKL_INLINE uint32_t CLIF_16(columns)(CLIF_16_ARGS, uint32_t k) { return CLIF_16(view)(CLIF_16_PASS, CLIF_T_COLUMNS + 4u * k); }

// Column `c` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints) of block `b`: blocks are PART nodes at the view's
// stride from block 0's start, each its columns end to end at its rows (the last block's are fewer), column `c` at
// `rows * columns[c]`.
UKL_INLINE uint32_t CLIF_16(column)(CLIF_16_ARGS, uint32_t c, uint32_t b)
{
    uint32_t base = b * CLIF_16(view)(CLIF_16_PASS, CLIF_T_STRIDE) + CLIF_16(columns)(CLIF_16_PASS, 0u);
    if (c == 0u)
        return base;
    uint32_t rows = ukl_min(CLIF_16(view)(CLIF_16_PASS, CLIF_T_NODES) - (b << 10), CLIF_T_PART);
    return base + rows * CLIF_16(columns)(CLIF_16_PASS, c);
}

// Checkpoint `c`: the start of node `c * K`, in its block's checkpoints.
UKL_INLINE uint64_t CLIF_16(checkpoint)(CLIF_16_ARGS, uint32_t c)
{
    uint32_t cw = CLIF_16(width)(CLIF_16_PASS, 3u);
    uint32_t checkpoints = CLIF_16(column)(CLIF_16_PASS, 4u, c >> 7);
    return CLIF_16(ld)(CLIF_16_PASS, checkpoints, (c & (CLIF_T_PART / CLIF_T_K - 1u)) * cw, 8u) & clif_t_mask(cw);
}

// Node `i`'s value in side list `k` (0 codes' low parts, 1 slots, 2 gaps, 3 lengths), the node listed. A part (PART
// nodes) holds, for each of its SUB blocks, the end of its run of offsets (a u16), then the offsets (each the low
// byte of a listed node), then the values; every offset into the part is held to its end, the next part's start.
UKL_INLINE uint64_t CLIF_16(side)(CLIF_16_ARGS, uint32_t k, uint32_t i)
{
    uint32_t side = CLIF_16(view)(CLIF_16_PASS, CLIF_T_SIDE + 4u * k);
    uint32_t parts = CLIF_16(view)(CLIF_16_PASS, CLIF_T_PARTS + 4u * k);
    uint32_t p = i >> 10;
    uint32_t start = (uint32_t)CLIF_16(ld)(CLIF_16_PASS, parts, p * 4u, 4u);
    uint32_t limit = (uint32_t)CLIF_16(ld)(CLIF_16_PASS, parts, p * 4u + 4u, 4u);
    uint32_t blocks = (CLIF_16(view)(CLIF_16_PASS, CLIF_T_NODES) + (CLIF_T_BLOCK - 1u)) >> 8;
    uint32_t nsub = ukl_min(CLIF_T_SUB, blocks - p * CLIF_T_SUB);
    uint32_t j = (i >> 8) & (CLIF_T_SUB - 1u);
    // The block's run of offsets: from the end of the one before (0 for the part's first) to its own end.
    uint32_t lo = j == 0u ? 0u : (uint32_t)CLIF_16(ld)(CLIF_16_PASS, side, ukl_min(start + 2u * (j - 1u), limit), 2u);
    uint32_t hi = (uint32_t)CLIF_16(ld)(CLIF_16_PASS, side, ukl_min(start + 2u * j, limit), 2u);
    uint32_t offsets = start + 2u * nsub;
    uint32_t low = i & (CLIF_T_BLOCK - 1u);
    // The first offset at or past the node's low byte.
    while (lo < hi) {
        uint32_t middle = (lo + hi) >> 1;
        if ((uint32_t)CLIF_16(ld)(CLIF_16_PASS, side, ukl_min(offsets + middle, limit), 1u) < low)
            lo = middle + 1u;
        else
            hi = middle;
    }
    // The values follow the part's offsets.
    uint32_t vw = (CLIF_16(view)(CLIF_16_PASS, CLIF_T_WIDTHS) >> ((4u * k + 16u) & 31u)) & 15u;
    uint32_t count = (uint32_t)CLIF_16(ld)(CLIF_16_PASS, side, ukl_min(start + 2u * (nsub - 1u), limit), 2u);
    uint32_t at = ukl_min(offsets + count + lo * vw, limit);
    return CLIF_16(ld)(CLIF_16_PASS, side, at, 8u) & clif_t_mask(vw);
}

// Value `i` of array `a` (0 slots, 1 gaps, 2 lengths): one word's load, or (all ones: escaped) its side list's.
UKL_INLINE uint64_t CLIF_16(get)(CLIF_16_ARGS, uint32_t a, uint32_t i)
{
    uint32_t w = CLIF_16(width)(CLIF_16_PASS, a);
    uint32_t array = CLIF_16(column)(CLIF_16_PASS, 1u + a, i >> 10);
    uint64_t ones = clif_t_mask(w);
    uint64_t x = CLIF_16(ld)(CLIF_16_PASS, array, (i & (CLIF_T_PART - 1u)) * w, 8u) & ones;
    return x == ones ? CLIF_16(side)(CLIF_16_PASS, a + 1u, i) : x;
}

// The node's slot: an inner node's subtree size, shifted up by one; a leaf's step back to its previous sibling or
// (bit 1) its parent, shifted up by two, with bit 0 set.
UKL_INLINE uint64_t CLIF_16(slot)(CLIF_16_ARGS, uint32_t i) { return CLIF_16(get)(CLIF_16_PASS, 0u, i); }

// The node's subtree's size: at least 1, whatever the slot holds.
UKL_INLINE uint32_t CLIF_16(size)(CLIF_16_ARGS, uint32_t i)
{
    uint64_t s = CLIF_16(slot)(CLIF_16_PASS, i);
    return (s & 1u) != 0u ? 1u : ukl_max((uint32_t)(s >> 1), 1u);
}

// The node's code's number in its language's table: its byte, or past the cutoff, the byte's range and its low part.
UKL_INLINE uint32_t CLIF_16(number)(CLIF_16_ARGS, uint32_t i)
{
    uint32_t byte = (uint32_t)CLIF_16(ld)(CLIF_16_PASS, CLIF_16(column)(CLIF_16_PASS, 0u, i >> 10), i & (CLIF_T_PART - 1u), 1u);
    uint32_t cut = CLIF_16(view)(CLIF_16_PASS, CLIF_T_CUT);
    if (byte < cut)
        return byte;
    uint32_t low = (uint32_t)CLIF_16(side)(CLIF_16_PASS, 0u, i);
    return cut + (byte - cut) * CLIF_16(view)(CLIF_16_PASS, CLIF_T_PER) + low;
}

// The node's start as stated: its checkpoint, then the gaps of the nodes after it, to the node's own.
UKL_INLINE uint32_t CLIF_16(start_stated)(CLIF_16_ARGS, uint32_t i)
{
    uint32_t c = i >> 3;
    uint64_t from = CLIF_16(checkpoint)(CLIF_16_PASS, c);
    uint32_t first = c * CLIF_T_K;
    uint32_t n = i - first;
    if (n == 0u)
        return (uint32_t)from;
    // At a byte's width, the gaps of nodes `first + 1 ..= i` are bytes 1 to `n` of the word at the checkpoint's node:
    // summed by folding the word, unless one is escaped (all ones: its side list's).
    if (CLIF_16(width)(CLIF_16_PASS, 1u) == 1u) {
        uint64_t x = CLIF_16(ld)(CLIF_16_PASS, CLIF_16(column)(CLIF_16_PASS, 2u, first >> 10), first & (CLIF_T_PART - 1u), 8u);
        uint64_t gaps = (~(uint64_t)0 >> ((7u - n) * 8u)) & ~(uint64_t)0xff;
        x &= gaps;
        uint64_t nx = ~x;
        uint64_t ff = ~(((nx & 0x7f7f7f7f7f7f7f7ful) + 0x7f7f7f7f7f7f7f7ful) | nx) & 0x8080808080808080ul;
        if ((ff & gaps) == 0u) {
            x = (x & 0x00ff00ff00ff00fful) + ((x >> 8) & 0x00ff00ff00ff00fful);
            x = (x & 0x0000ffff0000fffful) + ((x >> 16) & 0x0000ffff0000fffful);
            x = (x & 0xfffffffful) + (x >> 32);
            return (uint32_t)(from + x);
        }
    }
    // Else gap by gap: at most K - 1 values.
    uint64_t sum = from;
    for (uint32_t j = first + 1u; j <= i; j++)
        sum += CLIF_16(get)(CLIF_16_PASS, 1u, j);
    return (uint32_t)sum;
}

// The node's length, and its end as stated: its start and its length.
UKL_INLINE uint32_t CLIF_16(length)(CLIF_16_ARGS, uint32_t i) { return (uint32_t)CLIF_16(get)(CLIF_16_PASS, 2u, i); }
UKL_INLINE uint32_t CLIF_16(end_stated)(CLIF_16_ARGS, uint32_t i) { return CLIF_16(start_stated)(CLIF_16_PASS, i) + CLIF_16(length)(CLIF_16_PASS, i); }

// The node's start in a scan: the node before's (`prev`) and its gap, or at a checkpoint's node, the checkpoint.
UKL_INLINE uint32_t CLIF_16(start_next)(CLIF_16_ARGS, uint32_t i, uint32_t prev)
{
    if ((i & (CLIF_T_K - 1u)) != 0u)
        return prev + (uint32_t)CLIF_16(get)(CLIF_16_PASS, 1u, i);
    return (uint32_t)CLIF_16(checkpoint)(CLIF_16_PASS, i >> 3);
}

#undef CLIF_16
#undef CLIF_16_ARGS
#undef CLIF_16_PASS
