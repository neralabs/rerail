// The carried files: the shims, UKL's common headers and the library, compiled into the crate. Included by the
// crate (`lib.rs`) and by its build script (`build.rs`), which computes their fingerprint once: one list for both.

/// The shims and UKL's common headers, at their paths under `forest/src/`.
const HEADERS: &[(&str, &str)] = &[
    ("mldb/block/ukl/ukl", include_str!("../../../../forest/src/mldb/block/ukl/ukl")),
    ("mldb/block/ukl/ukl_shim.h", include_str!("../../../../forest/src/mldb/block/ukl/ukl_shim.h")),
    ("mldb/block/ukl/ukl_macros.h", include_str!("../../../../forest/src/mldb/block/ukl/ukl_macros.h")),
    ("mldb/block/ukl/ukl_auto_macros.h", include_str!("../../../../forest/src/mldb/block/ukl/ukl_auto_macros.h")),
    ("mldb/builtin/metal/ukl/ukl_metal_shim.h", include_str!("../../../../forest/src/mldb/builtin/metal/ukl/ukl_metal_shim.h")),
    ("mldb/builtin/cuda/ukl/ukl_cuda_shim.h", include_str!("../../../../forest/src/mldb/builtin/cuda/ukl/ukl_cuda_shim.h")),
    ("mldb/builtin/cpu/ukl/ukl_cpu_shim.h", include_str!("../../../../forest/src/mldb/builtin/cpu/ukl/ukl_cpu_shim.h")),
    ("mldb/builtin/opencl/ukl/ukl_opencl_shim.h", include_str!("../../../../forest/src/mldb/builtin/opencl/ukl/ukl_opencl_shim.h")),
];

/// The library's headers, by the name a kernel includes them by (`#include <ukl/memory.h>`), each after the ones it
/// includes. Their tests are `rerail-gpu`'s (`library_tests`), on each device.
pub const LIBRARY: &[(&str, &str)] = &[
    ("ukl/backend.h", include_str!("../lib/ukl/backend.h")),
    ("ukl/bits.h", include_str!("../lib/ukl/bits.h")),
    ("ukl/memory.h", include_str!("../lib/ukl/memory.h")),
    ("ukl/simd.h", include_str!("../lib/ukl/simd.h")),
    ("ukl/group.h", include_str!("../lib/ukl/group.h")),
    // A body over a type (decision 0440 §3.5): its header includes it once per type, with its parameters defined.
    ("ukl/body/lz4_decode.h", include_str!("../lib/ukl/body/lz4_decode.h")),
    ("ukl/body/histogram_part.h", include_str!("../lib/ukl/body/histogram_part.h")),
    ("ukl/body/radix_tile.h", include_str!("../lib/ukl/body/radix_tile.h")),
    ("ukl/lz4.h", include_str!("../lib/ukl/lz4.h")),
    ("ukl/scan.h", include_str!("../lib/ukl/scan.h")),
    ("ukl/compact.h", include_str!("../lib/ukl/compact.h")),
    ("ukl/histogram.h", include_str!("../lib/ukl/histogram.h")),
    ("ukl/radix.h", include_str!("../lib/ukl/radix.h")),
    ("ukl/hash.h", include_str!("../lib/ukl/hash.h")),
    ("ukl/workqueue.h", include_str!("../lib/ukl/workqueue.h")),
    ("ukl/pool.h", include_str!("../lib/ukl/pool.h")),
    ("ukl/grid.h", include_str!("../lib/ukl/grid.h")),
    ("ukl/clif.h", include_str!("../lib/ukl/clif.h")),
    // The tape's reads for the kernels, written by hand (decision 0524): one body, which `tape.h` reads formats 15
    // and 16 through and `tape_z.h` 16z too.
    ("ukl/reads/tape_body.h", include_str!("../lib/ukl/reads/tape_body.h")),
    ("ukl/reads/tape.h", include_str!("../lib/ukl/reads/tape.h")),
    ("ukl/reads/tape_z.h", include_str!("../lib/ukl/reads/tape_z.h")),
    // The other reads written once in CLIF (decision 0310), as `rerail_clif::ukl` writes them: generated, held here
    // so that every kernel finds them as it finds the rest of the library (`rerail-clif`'s tests write them again).
    ("ukl/reads/relation.h", include_str!("../lib/ukl/reads/relation.h")),
    ("ukl/reads/ids.h", include_str!("../lib/ukl/reads/ids.h")),
    ("ukl/reads/lines.h", include_str!("../lib/ukl/reads/lines.h")),
    ("ukl/reads/view.h", include_str!("../lib/ukl/reads/view.h")),
    // The tape's writer's ops for the kernels, written by hand (decisions 0434 §6, 0524).
    ("ukl/writer.h", include_str!("../lib/ukl/writer.h")),
];
