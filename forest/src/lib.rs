//! rerail's forest: MLDB's random forest trainer (`mldb/`, copied from mldbai/mldb@e3caed6f, branch
//! opencl-random-forests), training and predicting on the device, and the cost model built on it (`cost_model`).
//! The device is Metal on macOS and CUDA on Linux (`device`, decision 0440); elsewhere the crate is empty.
//!
//! The kernels are one C source (`mldb/plugins/jml/randomforest_kernels_common.h`) written against UKL's shims; the
//! build step preprocesses `randomforest_kernels_ukl.h` with the Metal shim into Metal source; on CUDA it is
//! preprocessed at run time for the device's architecture. Either is compiled once per process on the shared device
//! (`gpu::shared`; randomforest_kernels_metal.rs).

#![cfg(any(target_os = "macos", target_os = "linux", windows))]

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
#[path = "mldb/plugins/jml/jml/feature.rs"]
pub mod feature;
#[path = "mldb/plugins/jml/jml/split.rs"]
pub mod split;
#[path = "mldb/plugins/jml/jml/tree.rs"]
pub mod tree;
#[path = "mldb/plugins/jml/jml/decision_tree.rs"]
pub mod decision_tree;
#[path = "mldb/plugins/jml/jml/committee.rs"]
pub mod committee;
/// JML (`mldb/plugins/jml/jml/`): features, splits, trees, decision trees and committees.
pub mod jml {
    pub use super::{committee, decision_tree, feature, split, tree};
}
#[path = "mldb/plugins/jml/dataset_feature_space.rs"]
pub mod dataset_feature_space;
#[path = "mldb/plugins/jml/randomforest_types.rs"]
pub mod randomforest_types;
#[path = "mldb/plugins/jml/randomforest_kernels.rs"]
pub mod randomforest_kernels;
#[path = "mldb/plugins/jml/randomforest_recursive.rs"]
pub mod randomforest_recursive;
// The device layer: `rerail-gpu` (decision 0440), under the names the forest's code uses.
pub use rerail_gpu::{compute_kernel, compute_kernel_grid, device};
#[cfg(target_os = "linux")]
pub use rerail_gpu::compute_kernel_cuda;
#[cfg(target_os = "macos")]
pub use rerail_gpu::compute_kernel_metal;
#[path = "mldb/plugins/jml/randomforest_kernels_metal.rs"]
pub mod randomforest_kernels_metal;
#[path = "mldb/plugins/jml/randomforest_kernels_grid.rs"]
pub mod randomforest_kernels_grid;
#[path = "mldb/plugins/jml/randomforest.rs"]
pub mod randomforest;
#[path = "mldb/plugins/jml/randomforest_procedure.rs"]
pub mod randomforest_procedure;
#[path = "mldb/plugins/jml/randomforest_predict.rs"]
pub mod randomforest_predict;
#[cfg(test)]
#[path = "mldb/plugins/jml/testing/rf_kernel_test.rs"]
mod rf_kernel_test;

pub use randomforest_kernels_metal::{function, library, KERNELS};
#[cfg(target_os = "macos")]
pub use randomforest_kernels_metal::SOURCE;

pub mod cost_model;

/// The process's device and queue (`rerail_gpu::shared`), where the engine takes them from (`rerail_accel`).
pub mod gpu {
    pub use rerail_gpu::shared;
}

/// The process's user + system CPU seconds.
pub fn process_cpu() -> f64 {
    rerail_gpu::process_cpu_seconds()
}

/// Sorts keys and gives each distinct one once, in order (a numeric column's values, `from_column_device`).
pub type DistinctKeys = fn(&[i64]) -> Result<Vec<i64>, String>;

static DISTINCT_KEYS: std::sync::OnceLock<DistinctKeys> = std::sync::OnceLock::new();

/// Gives the forest a sort to find a column's distinct keys with (the engine's radix sort on the device, which this
/// crate cannot depend on); without one, the host sorts them. Only the first call counts.
pub fn set_distinct_keys(sort: DistinctKeys) {
    let _ = DISTINCT_KEYS.set(sort);
}

fn distinct_keys(keys: &[i64]) -> Result<Vec<i64>, String> {
    if let Some(sort) = DISTINCT_KEYS.get() {
        return sort(keys);
    }
    use rayon::prelude::*;
    let mut sorted = keys.to_vec();
    rerail_context::on_cores(|| sorted.par_sort_unstable());
    sorted.dedup();
    Ok(sorted)
}
