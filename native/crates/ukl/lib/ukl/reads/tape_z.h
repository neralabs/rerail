// ukl/reads/tape_z.h: the tape's reads for kernels that read 16z tapes too (decisions 0353 step 4b, 0524): a block
// decoded into the rolling cache the view names where a read falls, read there as format 16. The same functions as
// `<ukl/reads/tape.h>`, written by hand in `<ukl/reads/tape_body.h>`.
#pragma once
#ifdef CLIF_READS_TAPE
#error "the tape's reads twice: <ukl/reads/tape.h> and <ukl/reads/tape_z.h> are one set"
#endif
#define CLIF_READS_TAPE_Z 1
#include <ukl/reads/tape_body.h>
