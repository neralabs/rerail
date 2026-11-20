//! Sets the cfgs for what the Cranelift this builds against has beyond upstream's, which the
//! vendored one says through its `links` key (`native/vendor/cranelift-codegen-0.125.4/
//! PATCHES.md`):
//! - `cranelift_clmul`: `clmul`, a carry-less multiply. Without it a toggle is six shift-XOR
//!   rounds.
//! - `cranelift_preserve_most`: the `preserve_most` calling convention, for tier 2's cold
//!   helpers. Without it they take the platform's default, and a caller's values in the
//!   caller-saved registers are spilled around their calls.
fn main() {
    for (cfg, key) in [("cranelift_clmul", "DEP_CRANELIFT_CODEGEN_CLMUL"), ("cranelift_preserve_most", "DEP_CRANELIFT_CODEGEN_PRESERVE_MOST")] {
        println!("cargo::rustc-check-cfg=cfg({cfg})");
        println!("cargo::rerun-if-env-changed={key}");
        if std::env::var_os(key).is_some() {
            println!("cargo::rustc-cfg={cfg}");
        }
    }
}
