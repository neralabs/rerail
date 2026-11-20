//! Build steps per lane, each in its own file under `build/`, run only when its lane's feature is on.

#[path = "build/sme.rs"]
mod sme;
#[path = "build/ane.rs"]
mod ane;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let on = |f: &str| std::env::var_os(format!("CARGO_FEATURE_{}", f.to_uppercase())).is_some();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let macos = os == "macos";
    // The GPU lane: the `gpu` feature where the device layer is (`rerail-device`: macOS, Linux and Windows).
    if on("gpu") && (macos || os == "linux" || os == "windows") {
        println!("cargo:rustc-cfg=gpu_lane");
    }
    if on("sme") && macos {
        sme::run();
    }
    if on("ane") && macos {
        ane::run();
    }
}
