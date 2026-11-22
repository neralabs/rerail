// ukl/reads/tape_f15.h: format 15's reads (decision 0576), for format 15 alone: `clif_f15_*`. Its columns are rows of
// 2 or 4 bytes, read in place by the tape's one load (`clif_t_ld`). Included by `<ukl/reads/tape.h>` and
// `<ukl/reads/tape_z.h>`.

#pragma once
#include <ukl/reads/tape_common.h>

// Row `i` of a column of width 2 or 4.
UKL_INLINE uint32_t clif_f15_cell(UKL_GLOBAL const uint8_t* t, uint32_t column, uint32_t width, uint32_t i)
{
    return width == 2u ? (uint32_t)clif_t_ld(t, column, i * 2u, 2u) : (uint32_t)clif_t_ld(t, column, i * 4u, 4u);
}

// Column `k`'s offset, and width `k` (a nibble of the view's widths).
UKL_INLINE uint32_t clif_f15_column(CLIF_T_ARGS, uint32_t k) { return clif_view(v, CLIF_T_COLUMNS + 4u * k); }
UKL_INLINE uint32_t clif_f15_width(CLIF_T_ARGS, uint32_t k) { return (clif_view(v, CLIF_T_WIDTHS) >> (4u * k)) & 15u; }

// The node's subtree's size: at least 1.
UKL_INLINE uint32_t clif_f15_size(CLIF_T_ARGS, uint32_t i)
{
    return ukl_max(clif_f15_cell(t, clif_f15_column(t, v, 1u), clif_f15_width(t, v, 0u), i), 1u);
}

// The node's code's number.
UKL_INLINE uint32_t clif_f15_number(CLIF_T_ARGS, uint32_t i) { return (uint32_t)clif_t_ld(t, clif_f15_column(t, v, 0u), i * 2u, 2u); }

// The node's start as stated: its checkpoint (every 16 nodes) and its offset from it.
UKL_INLINE uint32_t clif_f15_start_stated(CLIF_T_ARGS, uint32_t i)
{
    uint32_t checkpoint = (uint32_t)clif_t_ld(t, clif_f15_column(t, v, 2u), (i >> CLIF_T_F15_STRIDE_SHIFT) * 4u, 4u);
    return checkpoint + clif_f15_cell(t, clif_f15_column(t, v, 3u), clif_f15_width(t, v, 1u), i);
}

// The node's end as stated: a tail from the node's own start, or from its last descendant's.
UKL_INLINE uint32_t clif_f15_end_stated(CLIF_T_ARGS, uint32_t i)
{
    uint32_t own = clif_view(v, CLIF_T_WIDTHS) & CLIF_T_F15_TAIL_FROM_START;
    uint32_t from = own != 0u ? i : i + clif_f15_size(t, v, i) - 1u;
    return clif_f15_start_stated(t, v, from) + clif_f15_cell(t, clif_f15_column(t, v, 4u), clif_f15_width(t, v, 2u), i);
}

UKL_INLINE uint32_t clif_f15_length(CLIF_T_ARGS, uint32_t i) { return clif_f15_end_stated(t, v, i) - clif_f15_start_stated(t, v, i); }

// The node's start in a scan: format 15 has no gaps to add, so its start, held to the source.
UKL_INLINE uint32_t clif_f15_start_next(CLIF_T_ARGS, uint32_t i, uint32_t prev)
{
    return ukl_min(clif_f15_start_stated(t, v, i), clif_view(v, CLIF_T_SOURCE_LEN));
}

// The node's parent, or NONE at the root: the nearest node before whose subtree holds the node, back over at most 64
// nodes, else a descent from the root over the siblings' subtrees.
UKL_INLINE uint32_t clif_f15_parent(CLIF_T_ARGS, uint32_t i)
{
    if (i == 0u)
        return CLIF_T_NONE;
    uint32_t j = i;
    for (uint32_t k = 0u; j != 0u && k < 64u; k++) {
        j -= 1u;
        if (clif_t_hop(t, v, j, clif_f15_size(t, v, j)) > i)
            return j;
    }
    // At each level, the child whose subtree holds the node.
    uint32_t at = 0u;
    for (;;) {
        uint32_t c = at + 1u;
        uint32_t after = clif_t_hop(t, v, c, clif_f15_size(t, v, c));
        while (after <= i) {
            c = after;
            after = clif_t_hop(t, v, c, clif_f15_size(t, v, c));
        }
        if (c == i)
            return at;
        at = c;
    }
}

// The last line start at or before byte `x`.
UKL_INLINE uint32_t clif_f15_line(CLIF_T_ARGS, uint32_t x)
{
    uint32_t lines = clif_view(v, CLIF_T_LINES);
    uint32_t lo = 1u, hi = clif_view(v, CLIF_T_LINE_COUNT);
    while (lo < hi) {
        uint32_t middle = (lo + hi) >> 1;
        if ((uint32_t)clif_t_ld(t, lines, middle * 4u, 4u) > x)
            hi = middle;
        else
            lo = middle + 1u;
    }
    return lo - 1u;
}

// Line `k`'s first byte.
UKL_INLINE uint32_t clif_f15_line_start(CLIF_T_ARGS, uint32_t k) { return (uint32_t)clif_t_ld(t, clif_view(v, CLIF_T_LINES), k * 4u, 4u); }
