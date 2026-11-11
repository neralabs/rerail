//! The forest's build step: MLDB's random forest kernels (`src/mldb/`, one C source for every target, through
//! UKL's shims) preprocessed into Metal source by the C preprocessor, as MLDB's `compileFromSourceFile` did:
//! `c++ -C -CC -xc++ -I. -I block/ukl -I builtin/metal/ukl -D__METAL_VERSION__ -D__METAL_MACOS__
//! -D__UKL_METAL_PREPROCESSING -E`. The Metal compiler is not needed here (the gpu lane compiles source on the
//! device at run time), so the build works without Xcode's Metal toolchain.

use std::path::PathBuf;

/// The Metal language version the shims see. They test only that it is defined (`ukl_shim.h` picks the Metal shim).
const METAL_VERSION: &str = "310";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/mldb");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let root = PathBuf::from("src");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("randomforest_kernels.metal");
    let compiler = cc::Build::new().cpp(true).get_compiler();
    let status = compiler
        .to_command()
        .args(["-C", "-CC", "-xc++"])
        .arg(format!("-I{}", root.display()))
        .arg(format!("-I{}", root.join("mldb/block/ukl").display()))
        .arg(format!("-I{}", root.join("mldb/builtin/metal/ukl").display()))
        .arg(format!("-D__METAL_VERSION__={METAL_VERSION}"))
        .args(["-D__METAL_MACOS__=1", "-D__UKL_METAL_PREPROCESSING=1", "-E"])
        .arg(root.join("mldb/plugins/jml/randomforest_kernels.metal"))
        .arg("-o")
        .arg(&out)
        .status()
        .expect("run the C preprocessor over the forest's kernels");
    assert!(status.success(), "preprocessing the forest's kernels failed ({status})");
}
