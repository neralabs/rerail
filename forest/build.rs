//! The forest's build step: MLDB's random forest kernels (`src/mldb/`, one C source for every target, through
//! UKL's shims) preprocessed into Metal source, as MLDB's `compileFromSourceFile` did with the C preprocessor.
//! `rerail-ukl` does it in-process (decision 0440 §4), with the same predefines and search order, so the build
//! needs no C compiler and no Metal toolchain (the gpu lane compiles the source on the device at run time).

use rerail_ukl::{Backend, Preprocess};
use std::path::PathBuf;

/// The Metal language version the shims see. They test only that it is defined (`ukl_shim.h` picks the Metal shim).
const METAL_VERSION: u32 = 310;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/mldb");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("src");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("randomforest_kernels.metal");
    let main = root.join("mldb/plugins/jml/randomforest_kernels_ukl.h");
    let text = Preprocess::new(Backend::Metal { version: METAL_VERSION })
        .dir(root.to_string_lossy())
        .run(&main.to_string_lossy())
        .unwrap_or_else(|e| panic!("preprocessing the forest's kernels: {e}"));
    std::fs::write(&out, text).expect("write the preprocessed kernels");
}
