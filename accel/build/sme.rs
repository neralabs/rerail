//! The sme lane's build step: the BMOPA kernels in C with ACLE SME intrinsics (`src/sme/kernels.c`), built by `cc`.
//!
//! `-mcpu=apple-m4` turns on SME2 without plain SVE: `-march=armv9.2-a+sme2` makes the compiler emit SVE (`cntd`)
//! outside streaming mode, which faults on Apple silicon. The kernels are built at `-O2` in every profile, so a debug
//! build times the same code as a release one.

pub fn run() {
    println!("cargo:rerun-if-changed=build/sme.rs");
    println!("cargo:rerun-if-changed=src/sme/kernels.c");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64") {
        return;
    }
    let mut build = cc::Build::new();
    build.file("src/sme/kernels.c").flag("-mcpu=apple-m4").flag("-Wall").opt_level(2).debug(false);
    build.compile("rerail_sme");
    // The SME ABI's support routines (`__arm_tpidr2_save`, entered with each new ZA state) live in clang's
    // compiler-rt, which the clang driver links and rustc's link line does not: link it by name.
    let out = build.get_compiler().to_command().arg("--print-file-name=libclang_rt.osx.a").output().expect("ask the C compiler for compiler-rt");
    let rt = std::path::PathBuf::from(String::from_utf8(out.stdout).expect("a path").trim());
    assert!(rt.is_file(), "compiler-rt not found ({}): the SME kernels need its __arm_tpidr2_save", rt.display());
    println!("cargo:rustc-link-search=native={}", rt.parent().unwrap().display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
