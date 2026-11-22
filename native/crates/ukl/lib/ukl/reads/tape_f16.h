// ukl/reads/tape_f16.h: format 16's reads over the tape in place (decision 0576): `clif_f16_*`, format 16's body
// (`<ukl/body/tape_f16.h>`) read through the tape's one load, and the reads that walk the tape (a parent) or
// read its newline index, which a 16z tape holds in place too. Included by `<ukl/reads/tape.h>` and
// `<ukl/reads/tape_z.h>`.

#pragma once
#include <ukl/reads/tape_common.h>

// The body's helpers: the view where the kernel holds it, the bytes in place.
UKL_INLINE uint32_t clif_f16_view(CLIF_T_ARGS, uint32_t a) { return clif_view(v, a); }
UKL_INLINE uint64_t clif_f16_ld(CLIF_T_ARGS, uint32_t base, uint32_t off, uint32_t w) { return clif_t_ld(t, base, off, w); }

#define CLIF_16(name) clif_f16_##name
#define CLIF_16_ARGS CLIF_T_ARGS
#define CLIF_16_PASS t, v
// The side read, the rare case, a call where the kernel uses no SIMD group (`UKL_SIMD_GROUP_FREE`: the interpreters,
// the sweep, the join view kernels). In a kernel that requires a sub-group size, on an Arc 140V at 16 lanes, the
// called read misread (the 16z reads' test: node 1's child 2 read 11 for none); inlined, or at 32 lanes, it read
// right, and on CUDA and the CPU called too. So there it is inlined (decision 0576 §19).
#if defined(UKL_SIMD_GROUP_FREE)
#define CLIF_16_RARE UKL_COLD
#else
#define CLIF_16_RARE UKL_INLINE
#endif
#include <ukl/body/tape_f16.h>

// The node's parent, or NONE at the root: the node before is the parent (an inner node), or a leaf naming the
// previous sibling or, every K-th sibling, the parent. Each step moves back, and a step that would not (a slot no
// tape's) ends the walk.
UKL_INLINE uint32_t clif_f16_parent(CLIF_T_ARGS, uint32_t i)
{
    uint32_t j = i;
    while (j != 0u) {
        uint32_t before = j - 1u;
        uint64_t s = clif_f16_slot(t, v, before);
        if ((s & 1u) == 0u)
            return before;
        uint32_t step = (uint32_t)(s >> 2);
        if (step - 1u >= j)
            return CLIF_T_NONE;
        j -= step;
        if ((s & 2u) != 0u)
            return j;
    }
    return CLIF_T_NONE;
}

// ---- the newline index (`rerail_tape_format::newlines`) ---------------------------------------------------------------

// The index's u32 field `k` (0 the newlines' count, 1 the low parts' width, 2 and 3 the zero and one samples' counts,
// 4 the high bits' words).
UKL_INLINE uint32_t clif_f16_lines_field(CLIF_T_ARGS, uint32_t k) { return (uint32_t)clif_t_ld(t, clif_view(v, CLIF_T_LINES), 4u * k, 4u); }

// Where the index's part at `offset` bytes past its first fields lies.
UKL_INLINE uint32_t clif_f16_lines_part(CLIF_T_ARGS, uint32_t offset) { return clif_view(v, CLIF_T_LINES) + offset + CLIF_T_LINES_HEAD; }

// Newline `i`'s low part (`l` bits).
UKL_INLINE uint64_t clif_f16_line_low(CLIF_T_ARGS, uint32_t i, uint32_t l)
{
    uint32_t before = (clif_f16_lines_field(t, v, 2u) + clif_f16_lines_field(t, v, 3u) + clif_f16_lines_field(t, v, 4u)) * 8u;
    uint64_t bit = (uint64_t)i * (uint64_t)l;
    uint64_t word = clif_t_ld(t, clif_f16_lines_part(t, v, before), (uint32_t)(bit >> 3), 8u);
    return (word >> (bit & 7u)) & (((uint64_t)1 << (l & 63u)) - 1u);
}

// The position in the index's high bits of its `j`-th one (`ones` 1) or zero (0): from the sample before it, a word
// at a time. Within the index's words: past them (a `j` it does not hold, a line past the last) the position past its
// last word. Unbounded, a word of none was a step of zero, and past the index lies the rest of the slab, zeroed: a
// select there walked it all (2026-11-20, a GPU held; decision 0463).
UKL_INLINE uint32_t clif_f16_line_select(CLIF_T_ARGS, uint32_t ones, uint32_t j)
{
    uint32_t zs = clif_f16_lines_field(t, v, 2u);
    uint32_t os = clif_f16_lines_field(t, v, 3u);
    uint32_t sample = clif_f16_lines_part(t, v, (ones != 0u ? zs * 8u : 0u) + (j >> CLIF_T_LINES_SAMPLE_SHIFT) * 8u);
    uint32_t w = (uint32_t)clif_t_ld(t, sample, 0u, 4u);
    uint32_t before = (uint32_t)clif_t_ld(t, sample, 4u, 4u);
    uint32_t high = clif_f16_lines_part(t, v, (zs + os) * 8u);
    uint32_t words = clif_f16_lines_field(t, v, 4u);
    for (;;) {
        if (w >= words)
            return words * 64u;
        uint64_t word = clif_t_ld(t, high, w * 8u, 8u);
        uint64_t x = ones != 0u ? word : ~word;
        uint32_t through = before + ukl_popcount64(x);
        if (through > j)
            return w * 64u + clif_t_select_in_word(x, j - before);
        w += 1u;
        before = through;
    }
}

// The line byte `x` lies in (from 0): the newlines before it, by the index (the zeros before and after its bucket,
// then a search of the bucket's low parts).
UKL_INLINE uint32_t clif_f16_line(CLIF_T_ARGS, uint32_t x)
{
    if (clif_f16_lines_field(t, v, 0u) == 0u)
        return 0u;
    uint32_t l = clif_f16_lines_field(t, v, 1u);
    uint32_t h = x >> (l & 31u);
    uint64_t xl = x & ((1u << (l & 31u)) - 1u);
    uint32_t lo = h == 0u ? 0u : clif_f16_line_select(t, v, 0u, h - 1u) - (h - 1u);
    uint32_t hi = clif_f16_line_select(t, v, 0u, h) - h;
    // The bucket's first newline whose low part is at or past `x`'s.
    while (lo < hi) {
        uint32_t middle = (lo + hi) >> 1;
        if (clif_f16_line_low(t, v, middle, l) < xl)
            lo = middle + 1u;
        else
            hi = middle;
    }
    return lo;
}

// Line `k`'s first byte: 0 for the first; else one past newline `k - 1` (its bucket, from its one's position, and
// its low part).
UKL_INLINE uint32_t clif_f16_line_start(CLIF_T_ARGS, uint32_t k)
{
    if (k == 0u)
        return 0u;
    uint32_t i = k - 1u;
    uint32_t bucket = clif_f16_line_select(t, v, 1u, i) - i;
    uint32_t l = clif_f16_lines_field(t, v, 1u);
    return ((bucket << (l & 31u)) | (uint32_t)clif_f16_line_low(t, v, i, l)) + 1u;
}
