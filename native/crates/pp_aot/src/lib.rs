//! **Deprecated** (decision 0550): Rust from an intermediate form. A grammar's Rust is the Bitlex compiler's own (`bitlex_lex_rust`, `bitlex_rust`); no new use. Kept until Go-C-and-C-support and Parser-generators-final-shape remove theirs.
//!
//! The C preprocessing words' compiled code built ahead of time (decision 0348, stage 4b): with
//! the `generated` feature, the JIT's functions as Rust (`build.rs`,
//! `native_cpu_pp::rust_source`), its entry `pp_run` and its fingerprints. It is
//! `rerail-c-directives`' CPU path (`expand::native`); its tests check it against the
//! interpreter and time it beside the JIT's code of the same IR.

#[cfg(all(feature = "generated", target_arch = "aarch64"))]
#[allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_imports, unused_parens, non_snake_case, clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/pp.rs"));
}

/// The ahead-of-time build: one for C and C++ (the dialect is a run's).
#[derive(Clone, Copy, Debug)]
pub struct Build {
    /// Its entry, `pp_run` (`native_cpu_pp::run_aot` runs it).
    pub run: unsafe extern "C" fn(u64) -> u64,
    /// The fingerprint of the Rust it was compiled from (`native_cpu_pp::rust_source`).
    pub source: u64,
    /// Its functions structured, and in all (`clif_rust`'s `STRUCTURED`).
    pub structured: (usize, usize),
    /// The fingerprint of the words' text it was made from (`expand.rir`).
    pub words: u64,
    /// The harness's interface it was made for (`native_cpu_pp::ABI`).
    pub abi: u64,
}

/// The build (none without the `generated` feature, and none but on AArch64: the generated Rust is NEON's,
/// `clif_rust`; elsewhere the path is the JIT's code).
#[cfg(all(feature = "generated", target_arch = "aarch64"))]
pub const BUILD: Option<Build> = Some(Build { run: generated::pp_run, source: generated::SOURCE, structured: generated::STRUCTURED, words: generated::WORDS, abi: generated::ABI });

#[cfg(not(all(feature = "generated", target_arch = "aarch64")))]
pub const BUILD: Option<Build> = None;
