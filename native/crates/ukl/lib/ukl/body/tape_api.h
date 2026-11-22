// ukl/body/tape_api.h: the reads made of a format's own (decision 0576), written once and included once for each
// format by `<ukl/reads/tape.h>` and `<ukl/reads/tape_z.h>`, with, defined first, `CLIF_F(name)`: that format's
// reads' names (`clif_f15_*`, `clif_f16_*`, `clif_f16z_*`). It is undefined at the end. Not `#pragma once`.

#if !defined(CLIF_F)
#error "ukl/body/tape_api.h needs CLIF_F (ukl/reads/tape.h, tape_z.h)"
#endif

// The node's start, held to the source: a span from the tape reads no byte past it, whatever the tape's gaps hold.
UKL_INLINE uint32_t CLIF_F(start)(CLIF_T_ARGS, uint32_t i)
{
    return ukl_min(CLIF_F(start_stated)(t, v, i), clif_view(v, CLIF_T_SOURCE_LEN));
}

// The node's end, held between its start and the source's end: a reader's span is never reversed and never past the
// source.
UKL_INLINE uint32_t CLIF_F(end)(CLIF_T_ARGS, uint32_t i)
{
    uint32_t start = CLIF_F(start)(t, v, i);
    return ukl_min(ukl_max(CLIF_F(end_stated)(t, v, i), start), clif_view(v, CLIF_T_SOURCE_LEN));
}

// One past the node's subtree.
UKL_INLINE uint32_t CLIF_F(end_of)(CLIF_T_ARGS, uint32_t i) { return clif_t_hop(t, v, i, CLIF_F(size)(t, v, i)); }

// The node after the node's subtree while it is before `end` (its parent's subtree's end), else NONE.
UKL_INLINE uint32_t CLIF_F(next_in)(CLIF_T_ARGS, uint32_t i, uint32_t end)
{
    uint32_t after = CLIF_F(end_of)(t, v, i);
    return after < end ? after : CLIF_T_NONE;
}

// The node's next sibling, or NONE.
UKL_INLINE uint32_t CLIF_F(next)(CLIF_T_ARGS, uint32_t i)
{
    uint32_t parent = CLIF_F(parent)(t, v, i);
    return parent == CLIF_T_NONE ? CLIF_T_NONE : CLIF_F(next_in)(t, v, i, CLIF_F(end_of)(t, v, parent));
}

// The node's first child, or NONE.
UKL_INLINE uint32_t CLIF_F(first_child)(CLIF_T_ARGS, uint32_t i) { return CLIF_F(size)(t, v, i) > 1u ? i + 1u : CLIF_T_NONE; }

// The node's children, by hops over their subtrees.
UKL_INLINE uint32_t CLIF_F(child_count)(CLIF_T_ARGS, uint32_t i)
{
    uint32_t end = CLIF_F(end_of)(t, v, i);
    uint32_t n = 0u;
    for (uint32_t c = i + 1u; c < end; c = CLIF_F(end_of)(t, v, c))
        n += 1u;
    return n;
}

// The node's `k`-th child (from 0), by hops over its elder siblings' subtrees, or NONE.
UKL_INLINE uint32_t CLIF_F(child)(CLIF_T_ARGS, uint32_t i, uint32_t k)
{
    uint32_t end = CLIF_F(end_of)(t, v, i);
    for (uint32_t c = i + 1u; c < end; c = CLIF_F(end_of)(t, v, c)) {
        if (k == 0u)
            return c;
        k -= 1u;
    }
    return CLIF_T_NONE;
}

#undef CLIF_F
