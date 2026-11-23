// ukl/clif.h: what the reads written once in CLIF (decision 0310) are written over, beyond the library's loads
// (`ukl/memory.h`): their two hooks, which a kernel source may define before it includes the reads, and the casts of
// the base to the coherent memory a cache's reads and writes go through (decision 0353 step 4b).
//
// The reads: the tape's, written by hand to the CLIF's functions and held to them by tests (`<ukl/reads/tape.h>`,
// `tape_z.h`, each format's reads once; decisions 0524, 0576), and the rest generated (`rerail_clif::ukl`: `view.h`, `ids.h`,
// `relation.h`).

#pragma once
#include <ukl/memory.h>

// The address space of a tape's view: the thread's own copy unless a kernel reads it where the batch holds it
// (`#define CLIF_VIEW_SPACE UKL_GLOBAL`, as the join kernels do).
#ifndef CLIF_VIEW_SPACE
#define CLIF_VIEW_SPACE UKL_PRIVATE
#endif

// The address space a block's reads (a relation's, a view entry's, the lines') take its base in and load its words
// from: the device's, unless a kernel reads blocks that other groups of its dispatch write (`CLIF_BLOCKS_COHERENT`
// defined first, as a join program's worker does, decision 0603), where a backend's caches are not coherent across
// the device. The CPU's memory is the host's.
#ifndef CLIF_BLOCK_SPACE
#if defined(CLIF_BLOCKS_COHERENT) && !defined(UKL_CPU)
#define CLIF_BLOCK_SPACE UKL_COHERENT
#define __CLIF_BLOCK_SPACE_COHERENT 1
#else
#define CLIF_BLOCK_SPACE UKL_GLOBAL
#endif
#endif

// A structure `T` in a block, read whole into `dst` (a kernel's parameters another group wrote, decision 0603).
// Metal's coherent memory is a qualifier a structure's copy keeps, so there it is a copy. Elsewhere in coherent
// blocks its 8-byte words are each a coherent load (`ukl_ldc_u64`), into a union with the structure (the words
// stored through a cast pointer and the fields read were two types at one address, which a compiler may reorder):
// CUDA's coherence is C++'s `volatile`, which gives a structure no implicit copy, and OpenCL 1.2's coherent loads
// are its own (`UKL_COHERENT_LOAD64`). The structure's size is a multiple of 8.
#if defined(__CLIF_BLOCK_SPACE_COHERENT) && !defined(UKL_METAL)
#define clif_struct(T, dst, p)                                                                                          \
    do {                                                                                                                \
        UKL_STATIC_ASSERT(sizeof(T) % 8u == 0u, "clif_struct: a structure of 8-byte words");                          \
        union { T t; uint64_t w[sizeof(T) / 8u]; } __clif_u;                                                            \
        for (uint32_t __clif_w = 0u; __clif_w < (uint32_t)(sizeof(T) / 8u); __clif_w++)                                \
            __clif_u.w[__clif_w] = ukl_ldc_u64((UKL_COHERENT const uint8_t*)(p), __clif_w * 8u);                        \
        (dst) = __clif_u.t;                                                                                             \
    } while (0)
#else
#define clif_struct(T, dst, p) ((dst) = *(p))
#endif

// The view's 32-bit field at byte `a`: a kernel may make a field a constant (the label passes' format).
#ifndef clif_view
#define clif_view(v, a) (v)[(a) / 4u]
#endif

// The base, as memory another thread of the dispatch writes: read, and written.
// Casts, so that a base in either space (`CLIF_BLOCK_SPACE`'s or the tape's) is taken.
#define clif_coherent(t) ((UKL_COHERENT const uint8_t*)(t))
#define clif_written(t) ((UKL_COHERENT uint8_t*)(t))
