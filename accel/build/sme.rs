//! The sme lane's build step (none yet).

pub fn run() {
    println!("cargo:rerun-if-changed=build/sme.rs");
}
