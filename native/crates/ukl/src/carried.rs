// The carried files: the shims, UKL's common headers and the library, compiled into the crate. Included by the
// crate (`lib.rs`) and by its build script (`build.rs`), which computes their fingerprint once: one list for both.

/// The shims and UKL's common headers, copied from MLDB under the paths they have there (`mldb/`, its licence beside them).
const HEADERS: &[(&str, &str)] = &[
    ("mldb/block/ukl/ukl", include_str!("../mldb/block/ukl/ukl")),
    ("mldb/block/ukl/ukl_shim.h", include_str!("../mldb/block/ukl/ukl_shim.h")),
    ("mldb/block/ukl/ukl_macros.h", include_str!("../mldb/block/ukl/ukl_macros.h")),
    ("mldb/block/ukl/ukl_auto_macros.h", include_str!("../mldb/block/ukl/ukl_auto_macros.h")),
    ("mldb/builtin/metal/ukl/ukl_metal_shim.h", include_str!("../mldb/builtin/metal/ukl/ukl_metal_shim.h")),
    ("mldb/builtin/cuda/ukl/ukl_cuda_shim.h", include_str!("../mldb/builtin/cuda/ukl/ukl_cuda_shim.h")),
    ("mldb/builtin/cpu/ukl/ukl_cpu_shim.h", include_str!("../mldb/builtin/cpu/ukl/ukl_cpu_shim.h")),
    ("mldb/builtin/opencl/ukl/ukl_opencl_shim.h", include_str!("../mldb/builtin/opencl/ukl/ukl_opencl_shim.h")),
    ("mldb/builtin/vec/ukl/ukl_vec_shim.h", include_str!("../mldb/builtin/vec/ukl/ukl_vec_shim.h")),
];

/// The library's headers, by the name a kernel includes them by (`#include <ukl/memory.h>`), each after the ones it
/// includes. Their tests are `rerail-gpu`'s (`library_tests`), on each device.
pub const LIBRARY: &[(&str, &str)] = &[
    ("ukl/backend.h", include_str!("../lib/ukl/backend.h")),
    ("ukl/bits.h", include_str!("../lib/ukl/bits.h")),
    ("ukl/dot.h", include_str!("../lib/ukl/dot.h")),
    ("ukl/memory.h", include_str!("../lib/ukl/memory.h")),
    ("ukl/simd.h", include_str!("../lib/ukl/simd.h")),
    ("ukl/group.h", include_str!("../lib/ukl/group.h")),
    ("ukl/matrix.h", include_str!("../lib/ukl/matrix.h")),
    // A body over a type (decision 0440 §3.5): its header includes it once per type, with its parameters defined.
    ("ukl/body/lz4_decode.h", include_str!("../lib/ukl/body/lz4_decode.h")),
    ("ukl/body/histogram_part.h", include_str!("../lib/ukl/body/histogram_part.h")),
    ("ukl/body/radix_tile.h", include_str!("../lib/ukl/body/radix_tile.h")),
    ("ukl/body/scan_tile.h", include_str!("../lib/ukl/body/scan_tile.h")),
    ("ukl/body/tape_f16.h", include_str!("../lib/ukl/body/tape_f16.h")),
    ("ukl/body/tape_api.h", include_str!("../lib/ukl/body/tape_api.h")),
    ("ukl/body/tape_read.h", include_str!("../lib/ukl/body/tape_read.h")),
    ("ukl/lz4.h", include_str!("../lib/ukl/lz4.h")),
    ("ukl/scan.h", include_str!("../lib/ukl/scan.h")),
    ("ukl/compact.h", include_str!("../lib/ukl/compact.h")),
    ("ukl/histogram.h", include_str!("../lib/ukl/histogram.h")),
    ("ukl/radix.h", include_str!("../lib/ukl/radix.h")),
    ("ukl/hash.h", include_str!("../lib/ukl/hash.h")),
    // The flat layer's working forms in place (*Flat Views and Lifecycles* step 7: `rerail_delta_core::flat`).
    ("ukl/flat.h", include_str!("../lib/ukl/flat.h")),
    ("ukl/workqueue.h", include_str!("../lib/ukl/workqueue.h")),
    ("ukl/stages.h", include_str!("../lib/ukl/stages.h")),
    ("ukl/clif.h", include_str!("../lib/ukl/clif.h")),
    // The tape's reads for the kernels, written by hand (decisions 0524, 0576): each format's reads, the reads made of
    // them (included once a format) and the public reads, which `tape.h` (format 16) and `tape_z.h` (16z too)
    // include.
    ("ukl/reads/tape_common.h", include_str!("../lib/ukl/reads/tape_common.h")),
    ("ukl/reads/tape_f16.h", include_str!("../lib/ukl/reads/tape_f16.h")),
    ("ukl/reads/tape_f16z.h", include_str!("../lib/ukl/reads/tape_f16z.h")),
    ("ukl/reads/tape.h", include_str!("../lib/ukl/reads/tape.h")),
    ("ukl/reads/tape_z.h", include_str!("../lib/ukl/reads/tape_z.h")),
    // A tape's auxiliary index (decision 0656), written by hand: its reads, and those of the tape and the auxiliary index together.
    ("ukl/reads/aux_index.h", include_str!("../lib/ukl/reads/aux_index.h")),
    ("ukl/reads/aux_index_tape.h", include_str!("../lib/ukl/reads/aux_index_tape.h")),
    // The other reads written once in CLIF (decision 0310), as `rerail_clif::ukl` writes them: generated, held here
    // so that every kernel finds them as it finds the rest of the library (`rerail-clif`'s tests write them again).
    ("ukl/reads/relation.h", include_str!("../lib/ukl/reads/relation.h")),
    ("ukl/reads/ids.h", include_str!("../lib/ukl/reads/ids.h")),
    ("ukl/reads/lines.h", include_str!("../lib/ukl/reads/lines.h")),
    ("ukl/reads/view.h", include_str!("../lib/ukl/reads/view.h")),
    ("ukl/body/view_reads.h", include_str!("../lib/ukl/body/view_reads.h")),
    // The tape's writer's ops for the kernels, written by hand (decisions 0434 §6, 0524).
    ("ukl/writer.h", include_str!("../lib/ukl/writer.h")),
    // The delta operations' library (decision 0686): the twins' block layouts (generated from them) and what every
    // operation's kernel shares; its kernels are `rerail-delta-exec`'s (`delta_exec/ukl/`).
    ("ukl/delta/layout.h", include_str!("../lib/ukl/delta/layout.h")),
    ("ukl/delta/op.h", include_str!("../lib/ukl/delta/op.h")),
    ("ukl/delta/runs.h", include_str!("../lib/ukl/delta/runs.h")),
    ("ukl/delta/parts.h", include_str!("../lib/ukl/delta/parts.h")),
    ("ukl/delta/trace.h", include_str!("../lib/ukl/delta/trace.h")),
    ("ukl/delta/docs.h", include_str!("../lib/ukl/delta/docs.h")),
    ("ukl/delta/cache.h", include_str!("../lib/ukl/delta/cache.h")),
    ("ukl/delta/text.h", include_str!("../lib/ukl/delta/text.h")),
    ("ukl/delta/atoms.h", include_str!("../lib/ukl/delta/atoms.h")),
    ("ukl/delta/sort.h", include_str!("../lib/ukl/delta/sort.h")),
    ("ukl/delta/par.h", include_str!("../lib/ukl/delta/par.h")),
    ("ukl/delta/rr.h", include_str!("../lib/ukl/delta/rr.h")),
    ("ukl/delta/relw.h", include_str!("../lib/ukl/delta/relw.h")),
    ("ukl/delta/sketch.h", include_str!("../lib/ukl/delta/sketch.h")),
];
