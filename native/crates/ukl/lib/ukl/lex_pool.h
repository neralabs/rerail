// ukl/lex_pool.h: the header of tier 1's pool on the device (`<ukl/lex_run.h>`, decision 0726): the host's layout,
// and what tier 1's phases say, which the token pass and tier 2's kernels read (the records, the refusal).

#pragma once

#if defined(UKL_CPU) || defined(UKL_VEC)
typedef UKL_GLOBAL uint32_t* UklLexPool;
#else
typedef UKL_COHERENT uint32_t* UklLexPool;
#endif

#define UKL_LEX_H 32u
// Set by the host: the chunks, the source's length, the blocks a chunk, the trees' stride; M's, K's and the entries'
// offsets.
#define UKL_LEX_CHUNKS 0u
#define UKL_LEX_LEN 1u
#define UKL_LEX_BLOCKS 2u
#define UKL_LEX_TN 3u
#define UKL_LEX_OM 4u
#define UKL_LEX_OK 5u
#define UKL_LEX_OE 6u
// Said by the phases: whether the file is refused, its records, its typed records (every type's), each type's.
#define UKL_LEX_REFUSED 8u
#define UKL_LEX_COUNT 9u
#define UKL_LEX_TYPED 10u
#define UKL_LEX_TYPE0 11u
// Each phase's tickets.
#define UKL_LEX_TICKET 24u

