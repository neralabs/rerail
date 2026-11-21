// ukl/reads/tape.h: the tape's reads for kernels that read tapes of format 15 and 16 (decisions 0310, 0524), written by
// hand in `<ukl/reads/tape_body.h>`. A kernel that reads 16z tapes too includes `<ukl/reads/tape_z.h>` instead: the two
// are one set of functions.
#pragma once
#ifdef CLIF_READS_TAPE_Z
#error "the tape's reads twice: <ukl/reads/tape.h> and <ukl/reads/tape_z.h> are one set"
#endif
#define CLIF_READS_TAPE 1
#include <ukl/reads/tape_body.h>
