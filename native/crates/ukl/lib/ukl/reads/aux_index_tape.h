// ukl/reads/aux_index_tape.h: the reads of a tape and its auxiliary index together (decision 0656): a child's place among its
// siblings and a parent's i-th child, from the auxiliary index's every-K-th children and at most K - 1 steps of the tape.
// `aux_index::place` and `aux_index::child` in Rust; `rerail-ukl-vec`'s `aux_index_tests` hold them equal.
//
// The tape's reads are `<ukl/reads/tape.h>`'s, or `tape_z.h`'s where a kernel included that first.

#pragma once
#if !defined(CLIF_READS_TAPE_Z)
#include <ukl/reads/tape.h>
#endif
#include <ukl/reads/aux_index.h>

// Where the tape's auxiliary index lies in a home that holds one (decision 0656 step 3), counted as its presence's
// offset is (from `t`): after the presence (the view's `presence`), its bytes rounded up to 16 from it;
// `f16::presence::aux_at` in Rust. 0 where the tape has no presence. Whether the home holds one is the host's to know
// (`Tape16::aux_bytes`; every home the engine writes since layout 8 does): past a presence that is its home's last
// part lie no bytes of the home to read.
UKL_INLINE uint32_t ukl_aux_in_home(CLIF_T_ARGS)
{
    uint32_t at = clif_view(v, CLIF_T_PRESENCE);
    if (at == 0u)
        return 0u;
    uint32_t words = (uint32_t)clif_t_ld(t, at, 0u, 4u) + (uint32_t)clif_t_ld(t, at, 4u, 4u);
    uint32_t len = 16u + 8u * words * (uint32_t)clif_t_ld(t, at, 8u, 4u) + 4u * (uint32_t)clif_t_ld(t, at, 12u, 4u);
    return at + ((len + 15u) & ~15u);
}

// `node`'s place among its siblings (0 for a first child, and for the root).
UKL_INLINE uint32_t ukl_aux_place(CLIF_T_ARGS, UKL_AUX_ARGS, uint32_t node)
{
    uint32_t p = clif_tape_parent(t, v, node);
    if (p == CLIF_T_NONE)
        return 0u;
    uint64_t e = ukl_aux_eighth_at_or_before(s, p, node);
    uint32_t c = p + 1u, i = 0u;
    if ((uint32_t)e != UKL_AUX_NONE) {
        c = (uint32_t)e;
        i = (uint32_t)(e >> 32) * UKL_AUX_K;
    }
    while (c != node) {
        c = clif_tape_end_of(t, v, c);
        i++;
    }
    return i;
}

// `parent`'s child `i`, or `CLIF_T_NONE`.
UKL_INLINE uint32_t ukl_aux_child(CLIF_T_ARGS, UKL_AUX_ARGS, uint32_t parent, uint32_t i)
{
    uint32_t end = clif_tape_end_of(t, v, parent);
    uint32_t c = i < UKL_AUX_K ? parent + 1u : ukl_aux_eighth(s, parent, i / UKL_AUX_K);
    if (c == UKL_AUX_NONE || c >= end)
        return CLIF_T_NONE;
    for (uint32_t k = 0u; k < i % UKL_AUX_K; k++) {
        c = clif_tape_end_of(t, v, c);
        if (c >= end)
            return CLIF_T_NONE;
    }
    return c;
}
