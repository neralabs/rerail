//! The GPU lane's cfg, as rerail-accel's: `gpu_lane` where the `gpu` feature is on and the device layer is (macOS,
//! Linux and Windows).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if std::env::var_os("CARGO_FEATURE_GPU").is_some() && (os == "macos" || os == "linux" || os == "windows") {
        println!("cargo:rustc-cfg=gpu_lane");
    }
}
