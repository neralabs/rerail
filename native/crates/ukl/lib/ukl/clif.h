// ukl/clif.h: what the reads written once in CLIF (decision 0310) are written over, beyond the library's loads
// (`ukl/memory.h`): their two hooks, which a kernel source may define before it includes the reads, and the casts of
// the base to the coherent memory a cache's reads and writes go through (decision 0353 step 4b).
//
// The reads: the tape's, written by hand to the CLIF's functions and held to them by tests (`<ukl/reads/tape.h>`,
// `tape_z.h`, both `tape_body.h`; decision 0524), and the rest generated (`rerail_clif::ukl`: `view.h`, `ids.h`,
// `relation.h`).

#pragma once
#include <ukl/memory.h>

// The address space of a tape's view: the thread's own copy unless a kernel reads it where the batch holds it
// (`#define CLIF_VIEW_SPACE UKL_GLOBAL`, as the join kernels do).
#ifndef CLIF_VIEW_SPACE
#define CLIF_VIEW_SPACE UKL_PRIVATE
#endif

// The view's 32-bit field at byte `a`: a kernel may make a field a constant (the label passes' format).
#ifndef clif_view
#define clif_view(v, a) (v)[(a) / 4u]
#endif

// The base, as memory another thread of the dispatch writes: read, and written.
UKL_INLINE UKL_COHERENT const uint8_t* clif_coherent(UKL_GLOBAL const uint8_t* t) { return (UKL_COHERENT const uint8_t*)t; }
UKL_INLINE UKL_COHERENT uint8_t* clif_written(UKL_GLOBAL const uint8_t* t) { return (UKL_COHERENT uint8_t*)t; }
