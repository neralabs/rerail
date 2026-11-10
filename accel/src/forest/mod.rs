//! The forest: MLDB's Metal random forest trainer (`mldb/`, copied from mldbai/mldb@e3caed6f, branch
//! opencl-random-forests), being brought into the gpu lane in place.
//!
//! The kernels are one C source (`mldb/plugins/jml/randomforest_kernels_common.h`) written against UKL's shims; the
//! build step preprocesses `randomforest_kernels.metal` with the Metal shim into Metal source, compiled here once
//! per process on the shared device (`gpu::shared`).

// The copied host sources, each translated to Rust where it sits (upstream paths under `mldb/`).
#[path = "mldb/utils/fixed_point_accum.rs"]
pub mod fixed_point_accum;
#[path = "mldb/arch/bit_range_ops.rs"]
pub mod bit_range_ops;
#[path = "mldb/block/memory_region.rs"]
pub mod memory_region;
#[path = "mldb/utils/buckets.rs"]
pub mod buckets;
#[path = "mldb/core/bucket.rs"]
pub mod bucket;

use metal::{ComputePipelineState, Library};
use std::sync::OnceLock;

/// The kernels' Metal source, as the build step preprocessed it.
pub const SOURCE: &str = include_str!(concat!(env!("OUT_DIR"), "/randomforest_kernels.metal"));

/// The trainer's kernels (`randomforest_kernels.metal`), in the order a tree's training runs them.
pub const KERNELS: [&str; 9] = [
    "decompressRowsKernel",
    "testFeatureKernel",
    "getPartitionSplitsKernel",
    "bestPartitionSplitKernel",
    "assignPartitionNumbersKernel",
    "clearBucketsKernel",
    "updatePartitionNumbersKernel",
    "updateBucketsKernel",
    "fixupBucketsKernel",
];

/// The kernels' library, compiled on the shared device the first time it is asked for.
pub fn library() -> Result<&'static Library, String> {
    static LIB: OnceLock<Result<Library, String>> = OnceLock::new();
    LIB.get_or_init(|| {
        let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
        objc::rc::autoreleasepool(|| shared.device.new_library_with_source(SOURCE, &metal::CompileOptions::new()).map_err(|e| format!("forest: compiling the kernels: {e}")))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The pipeline of kernel `name`.
pub fn pipeline(name: &str) -> Result<ComputePipelineState, String> {
    let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
    let f = library()?.get_function(name, None).map_err(|e| format!("forest: {name}: {e}"))?;
    shared.device.new_compute_pipeline_state_with_function(&f).map_err(|e| format!("forest: {name}: {e}"))
}

#[cfg(test)]
mod tests {
    /// Every kernel of the trainer compiles from the one C source, through the Metal shim, on this device.
    #[test]
    fn forest_kernels_compile_on_metal() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no Metal device: skipped");
            return;
        }
        for name in super::KERNELS {
            let p = super::pipeline(name).unwrap_or_else(|e| panic!("{e}"));
            assert!(p.max_total_threads_per_threadgroup() > 0, "{name}");
        }
    }
}
