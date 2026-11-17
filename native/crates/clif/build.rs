//! Sets the cfg `cranelift_clmul` when the Cranelift this builds against has `clmul`, a
//! carry-less multiply: the vendored one does (`native/vendor/cranelift-codegen-0.125.4/
//! PATCHES.md`), and says so through its `links` key. Upstream's does not. The translator
//! (`rust`) then writes a `clmul` as `vmull_p64`, as the compiler's build script sets it there.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(cranelift_clmul)");
    println!("cargo::rerun-if-env-changed=DEP_CRANELIFT_CODEGEN_CLMUL");
    if std::env::var_os("DEP_CRANELIFT_CODEGEN_CLMUL").is_some() {
        println!("cargo::rustc-cfg=cranelift_clmul");
    }
}
