//! Build steps per lane, each in its own file under `build/`, run only when its lane's feature is on.

#[path = "build/sme.rs"]
mod sme;
#[path = "build/ane.rs"]
mod ane;
#[path = "build/forest.rs"]
mod forest;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let on = |f: &str| std::env::var_os(format!("CARGO_FEATURE_{}", f.to_uppercase())).is_some();
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if on("sme") && macos {
        sme::run();
    }
    if on("ane") && macos {
        ane::run();
    }
    if on("gpu") && macos {
        forest::run();
    }
}
