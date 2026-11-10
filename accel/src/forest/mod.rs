//! The forest: MLDB's Metal random forest trainer (`mldb/`, copied from mldbai/mldb@e3caed6f, branch
//! opencl-random-forests), being brought into the gpu lane in place.
//!
//! The kernels are one C source (`mldb/plugins/jml/randomforest_kernels_common.h`) written against UKL's shims; the
//! build step preprocesses `randomforest_kernels.metal` with the Metal shim into Metal source, compiled once per
//! process on the shared device (`gpu::shared`; randomforest_kernels_metal.rs).

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
/// JML (`mldb/plugins/jml/jml/`): features, splits, trees.
pub mod jml {
    pub use super::{feature, split, tree};
}
#[path = "mldb/plugins/jml/dataset_feature_space.rs"]
pub mod dataset_feature_space;
#[path = "mldb/plugins/jml/randomforest_types.rs"]
pub mod randomforest_types;
#[path = "mldb/plugins/jml/randomforest_kernels.rs"]
pub mod randomforest_kernels;
#[path = "mldb/plugins/jml/randomforest_recursive.rs"]
pub mod randomforest_recursive;
#[path = "mldb/builtin/metal/compute_kernel_metal.rs"]
pub mod compute_kernel_metal;
#[path = "mldb/plugins/jml/randomforest_kernels_metal.rs"]
pub mod randomforest_kernels_metal;
#[path = "mldb/block/compute_kernel_grid.rs"]
pub mod compute_kernel_grid;
#[path = "mldb/plugins/jml/randomforest_kernels_grid.rs"]
pub mod randomforest_kernels_grid;
#[path = "mldb/plugins/jml/randomforest.rs"]
pub mod randomforest;
#[path = "mldb/plugins/jml/randomforest_procedure.rs"]
pub mod randomforest_procedure;
#[cfg(test)]
#[path = "mldb/plugins/jml/testing/rf_kernel_test.rs"]
mod rf_kernel_test;

pub use randomforest_kernels_metal::{function, library, KERNELS, SOURCE};
