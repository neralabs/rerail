//! The ane lane's build step (none yet).

pub fn run() {
    println!("cargo:rerun-if-changed=build/ane.rs");
}
