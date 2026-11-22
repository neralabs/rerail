// ukl/reads/tape.h: the tape's reads for kernels that read tapes of format 15 and 16 (decisions 0310, 0524, 0576):
// each format's reads by name (`clif_f15_*`, `clif_f16_*`), and the public reads (`clif_tape_*`), which ask the view
// for its format once. A kernel that reads 16z tapes too includes `<ukl/reads/tape_z.h>` instead: the two are one set
// of functions.
#pragma once
#ifdef CLIF_READS_TAPE_Z
#error "the tape's reads twice: <ukl/reads/tape.h> and <ukl/reads/tape_z.h> are one set"
#endif
#define CLIF_READS_TAPE 1
#include <ukl/reads/tape_common.h>
#include <ukl/reads/tape_f15.h>
#include <ukl/reads/tape_f16.h>
#define CLIF_F(name) clif_f15_##name
#include <ukl/body/tape_api.h>
#define CLIF_F(name) clif_f16_##name
#include <ukl/body/tape_api.h>
#include <ukl/body/tape_read.h>
