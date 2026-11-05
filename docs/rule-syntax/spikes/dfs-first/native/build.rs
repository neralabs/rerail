// Compiles gpu/dfs_gpu.mm (Metal host code) with the system clang and links it, so the GPU path is
// reachable from cargo alone. macOS only.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=gpu/dfs_gpu.mm");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let obj = out.join("dfs_gpu.o");
    let lib = out.join("libdfs_gpu.a");
    let ok = Command::new("clang")
        .args(["-fobjc-arc", "-std=c++17", "-O2", "-c", "gpu/dfs_gpu.mm", "-o"])
        .arg(&obj)
        .status()
        .expect("clang");
    assert!(ok.success(), "clang failed on gpu/dfs_gpu.mm");
    let ok = Command::new("ar").arg("rcs").arg(&lib).arg(&obj).status().expect("ar");
    assert!(ok.success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=dfs_gpu");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rustc-link-lib=objc");
}
