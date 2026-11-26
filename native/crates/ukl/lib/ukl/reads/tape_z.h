// ukl/reads/tape_z.h: the tape's reads for kernels that read 16z tapes too (decisions 0353 step 4b, 0524, 0576): a
// block decoded into the rolling cache the view names where a read falls, read there as format 16. Each format's
// reads by name (`clif_f16_*`, `clif_f16z_*`), and the public reads (`clif_tape_*`), which ask the view
// for its format once. The same functions as `<ukl/reads/tape.h>`, and 16z's.
#pragma once
#ifdef CLIF_READS_TAPE
#error "the tape's reads twice: <ukl/reads/tape.h> and <ukl/reads/tape_z.h> are one set"
#endif
#define CLIF_READS_TAPE_Z 1
#include <ukl/reads/tape_common.h>
#include <ukl/reads/tape_f16.h>
#include <ukl/reads/tape_f16z.h>
#define CLIF_F(name) clif_f16_##name
#include <ukl/body/tape_api.h>
#define CLIF_F(name) clif_f16z_##name
#include <ukl/body/tape_api.h>
#include <ukl/body/tape_read.h>
