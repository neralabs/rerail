//! randomforest_kernels_metal.cc and randomforest_kernels_metal.h, in Rust.
//!
//! Jeremy Barnes, 8 September 2021. Copyright (c) 2021 Jeremy Barnes. This file is part of MLDB. Copyright 2016
//! mldb.ai inc. Apache-2.0 (see the LICENSE at `forest/src/mldb/`).
//!
//! Kernels for the random forest algorithm, on Metal: the `randomforest_kernels` library. Upstream registered a
//! generator that loaded a prebuilt `randomforest_metal.metallib` (or compiled the source file); here the build
//! step preprocesses `randomforest_kernels_ukl.h` through the Metal shim, and the library is compiled from that
//! source on the shared device (`gpu::shared`) the first time it is asked for, once per process. Its functions,
//! with their pipeline states and reflected arguments, are made once each.

use crate::device::{ComputeFunction, ComputeFunctionLibrary};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The kernels' Metal source, as the build step preprocessed it.
#[cfg(target_os = "macos")]
pub const SOURCE: &str = include_str!(concat!(env!("OUT_DIR"), "/randomforest_kernels.metal"));

/// The kernels for `backend` (CUDA, the CPU): the UKL source, carried in the binary, preprocessed in-process
/// through the backend's shim (`rerail-ukl`), as its compiler takes it.
pub fn ukl_source(preprocess: rerail_ukl::Preprocess) -> Result<String, String> {
    const ROOT: &str = "/<forest>";
    const MAIN: &str = "mldb/plugins/jml/randomforest_kernels_ukl.h";
    const COMMON: &str = "mldb/plugins/jml/randomforest_kernels_common.h";
    preprocess
        .dir(ROOT)
        .file(format!("{ROOT}/{MAIN}"), include_str!("randomforest_kernels_ukl.h"))
        .file(format!("{ROOT}/{COMMON}"), include_str!("randomforest_kernels_common.h"))
        .run(&format!("{ROOT}/{MAIN}"))
        .map_err(|e| e.to_string())
}

/// The library's name in the registry.
pub const LIBRARY_NAME: &str = "randomforest_kernels";

/// The trainer's kernels (`randomforest_kernels_ukl.h`), in the order a tree's training runs them, then the
/// forest's prediction and the feature space's columns.
pub const KERNELS: [&str; 16] = [
    "bagRowsKernel",
    "rootWeightsKernel",
    "decompressRowsKernel",
    "testFeatureKernel",
    "getPartitionSplitsKernel",
    "bestPartitionSplitKernel",
    "assignPartitionNumbersKernel",
    "clearBucketsKernel",
    "updatePartitionNumbersKernel",
    "updateBucketsKernel",
    "fixupBucketsKernel",
    "predictForestKernel",
    "numericBucketsKernel",
    "markCodesKernel",
    "categoricalBucketsKernel",
    "packBucketsKernel",
];

/// The kernels' library, compiled on the shared device the first time it is asked for.
pub fn library() -> Result<&'static ComputeFunctionLibrary, String> {
    static LIB: OnceLock<Result<ComputeFunctionLibrary, String>> = OnceLock::new();
    LIB.get_or_init(|| {
        let shared = crate::gpu::shared::get().ok_or("no device")?;
        #[cfg(target_os = "macos")]
        let source = if shared.device.is_cpu() { ukl_source(shared.device.preprocess())? } else { SOURCE.to_string() };
        #[cfg(target_os = "linux")]
        let source = ukl_source(shared.device.preprocess())?;
        ComputeFunctionLibrary::compile_from_source(&shared.device, &source, "randomforest_kernels_ukl.h")
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The library's function `name`, with its pipeline state and arguments (made once).
pub fn function(name: &str) -> Result<&'static ComputeFunction, String> {
    static FNS: OnceLock<Mutex<HashMap<String, &'static ComputeFunction>>> = OnceLock::new();
    let mut fns = FNS.get_or_init(Default::default).lock().unwrap();
    if let Some(f) = fns.get(name) {
        return Ok(f);
    }
    let shared = crate::gpu::shared::get().ok_or("no Metal device")?;
    let f: &'static ComputeFunction = Box::leak(Box::new(library()?.get_function(&shared.device, name)?));
    fns.insert(name.to_string(), f);
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{GridComputeFunctionArgumentDisposition as D, KernelBaseType};

    /// Every kernel of the trainer compiles from the one C source, through its backend's shim, on this device; their
    /// arguments reflect as the grid registrations bind them (buffers by name, the `args` structures by member,
    /// threadgroup memory).
    #[test]
    fn forest_kernels_compile_and_reflect() {
        if crate::gpu::shared::get().is_none() {
            eprintln!("no {} device: skipped", crate::device::backend());
            return;
        }
        for name in KERNELS {
            let f = function(name).unwrap_or_else(|e| panic!("{e}"));
            #[cfg(target_os = "macos")]
            if let Some(f) = f.metal() {
                assert!(f.compute_pipeline_state.max_total_threads_per_threadgroup() > 0, "{name}");
            }
            assert!(!f.get_argument_info().is_empty(), "{name}");
        }
        let tf = function("testFeatureKernel").unwrap();
        let arg = |n: &str| tf.get_argument_info().iter().find(|a| a.name == n).unwrap_or_else(|| panic!("testFeatureKernel: no {n}: {:?}", tf.get_argument_info().iter().map(|a| &a.name).collect::<Vec<_>>()));
        let args = arg("args");
        assert_eq!(args.disposition, D::Buffer);
        assert_eq!(args.ty.base_type.field("numRows").map(|f| f.offset), Some(0));
        assert_eq!(args.ty.base_type.field("maxLocalBuckets").map(|f| f.offset), Some(4));
        assert_eq!(arg("w").disposition, D::Threadgroup);
        let pb = arg("partitionBuckets");
        assert_eq!(pb.ty.base_type.width(), 12, "W32 on the device");
        assert!(matches!(&arg("decodedRows").ty.base_type, KernelBaseType::Basic { ty, .. } if ty.name == "f32"));
        assert_eq!(arg("decodedRows").ty.dims, 1);
    }
}
