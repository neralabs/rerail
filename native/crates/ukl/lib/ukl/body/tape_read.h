// ukl/body/tape_read.h: the tape's public reads, `clif_tape_*` (decision 0576): the one place a read asks the view
// for its format, then makes that format's read. A 16z read (with `<ukl/reads/tape_z.h>`) is checked here, once, by
// the one check (`CLIF_Z_READ`). Included once, by
// `<ukl/reads/tape.h>` or `<ukl/reads/tape_z.h>`, after every format's reads.

#if !defined(CLIF_READS_TAPE) && !defined(CLIF_READS_TAPE_Z)
#error "ukl/body/tape_read.h is included by ukl/reads/tape.h or tape_z.h"
#endif

#ifdef CLIF_READS_TAPE_Z
#define CLIF_TAPE_Z(type, name, args) \
    if (clif_format == CLIF_T_F16Z_FORMAT) CLIF_Z_READ(type, t, v, clif_f16z_##name args)
#else
#define CLIF_TAPE_Z(type, name, args)
#endif

// `clif_tape_<name>(t, v, ...)`: format 16's read, 16z's, else format 15's, with `link`: `UKL_INLINE`, or `UKL_COLD`
// for a large read (decision 0548: three formats' bodies and 16z's check are called, not copied into each caller).
#define CLIF_TAPE_READ(link, type, name, params, args)                   \
    link type clif_tape_##name params                                    \
    {                                                                    \
        uint32_t clif_format = clif_view(v, CLIF_T_FORMAT);              \
        if (clif_format == 16u)                                          \
            return clif_f16_##name args;                                 \
        CLIF_TAPE_Z(type, name, args)                                    \
        return clif_f15_##name args;                                     \
    }

// The node's subtree's size (itself and its descendants), and its code's number in its table (format 16: its
// language's).
CLIF_TAPE_READ(UKL_COLD, uint32_t, size, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, number, (CLIF_T_ARGS, uint32_t i), (t, v, i))
// The node's start and end as the tape states them (what a check of the tape reads), held to the source (what a
// kernel's span reads), and its length in bytes.
CLIF_TAPE_READ(UKL_COLD, uint32_t, start_stated, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_COLD, uint32_t, end_stated, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, start, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, end, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, length, (CLIF_T_ARGS, uint32_t i), (t, v, i))
// The node's start given the node before's (`prev`; any value for the first node): in a scan, format 16's from the
// one before, a gap's add, or its checkpoint.
CLIF_TAPE_READ(UKL_COLD, uint32_t, start_next, (CLIF_T_ARGS, uint32_t i, uint32_t prev), (t, v, i, prev))
// The tree: the node's parent (NONE at the root), one past its subtree, the node after its subtree before `end`, its
// next sibling, its first child, its children's count and its `k`-th child (NONE for none).
CLIF_TAPE_READ(UKL_INLINE, uint32_t, parent, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, end_of, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, next_in, (CLIF_T_ARGS, uint32_t i, uint32_t end), (t, v, i, end))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, next, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, first_child, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, child_count, (CLIF_T_ARGS, uint32_t i), (t, v, i))
CLIF_TAPE_READ(UKL_INLINE, uint32_t, child, (CLIF_T_ARGS, uint32_t i, uint32_t k), (t, v, i, k))
// The line byte `x` lies in (from 0), and line `k`'s first byte.
CLIF_TAPE_READ(UKL_COLD, uint32_t, line, (CLIF_T_ARGS, uint32_t x), (t, v, x))
CLIF_TAPE_READ(UKL_COLD, uint32_t, line_start, (CLIF_T_ARGS, uint32_t k), (t, v, k))

// Format 16's slot of the node (16z's too): an inner node's subtree size, shifted up by one; a leaf's step back to its
// previous sibling or (bit 1) its parent, shifted up by two, with bit 0 set. Format 15 has no slots: its tape is read
// as format 16's would be, as the reads always have.
UKL_INLINE uint64_t clif_tape_slot(CLIF_T_ARGS, uint32_t i)
{
    uint32_t clif_format = clif_view(v, CLIF_T_FORMAT);
    CLIF_TAPE_Z(uint64_t, slot, (t, v, i))
    return clif_f16_slot(t, v, i);
}
