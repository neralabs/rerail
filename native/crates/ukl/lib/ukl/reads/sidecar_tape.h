// ukl/reads/sidecar_tape.h: the reads of a tape and its sidecar together (decision 0656): a child's place among its
// siblings and a parent's i-th child, from the sidecar's every-K-th children and at most K - 1 steps of the tape.
// `sidecar::place` and `sidecar::child` in Rust; `rerail-ukl-vec`'s `sidecar_tests` hold them equal.
//
// The tape's reads are `<ukl/reads/tape.h>`'s, or `tape_z.h`'s where a kernel included that first.

#pragma once
#if !defined(CLIF_READS_TAPE_Z)
#include <ukl/reads/tape.h>
#endif
#include <ukl/reads/sidecar.h>

// `node`'s place among its siblings (0 for a first child, and for the root).
UKL_INLINE uint32_t ukl_sc_place(CLIF_T_ARGS, UKL_SC_ARGS, uint32_t node)
{
    uint32_t p = clif_tape_parent(t, v, node);
    if (p == CLIF_T_NONE)
        return 0u;
    uint64_t e = ukl_sc_eighth_at_or_before(s, p, node);
    uint32_t c = p + 1u, i = 0u;
    if ((uint32_t)e != UKL_SC_NONE) {
        c = (uint32_t)e;
        i = (uint32_t)(e >> 32) * UKL_SC_K;
    }
    while (c != node) {
        c = clif_tape_end_of(t, v, c);
        i++;
    }
    return i;
}

// `parent`'s child `i`, or `CLIF_T_NONE`.
UKL_INLINE uint32_t ukl_sc_child(CLIF_T_ARGS, UKL_SC_ARGS, uint32_t parent, uint32_t i)
{
    uint32_t end = clif_tape_end_of(t, v, parent);
    uint32_t c = i < UKL_SC_K ? parent + 1u : ukl_sc_eighth(s, parent, i / UKL_SC_K);
    if (c == UKL_SC_NONE || c >= end)
        return CLIF_T_NONE;
    for (uint32_t k = 0u; k < i % UKL_SC_K; k++) {
        c = clif_tape_end_of(t, v, c);
        if (c >= end)
            return CLIF_T_NONE;
    }
    return c;
}
