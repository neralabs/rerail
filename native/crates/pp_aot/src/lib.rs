//! The C preprocessing words' compiled code built ahead of time (decision 0348, stage 4b): with
//! the `generated` feature, each module is the JIT's functions as Rust (`build.rs`,
//! `native_cpu_pp::rust_source`), its entry `pp_run`, its depth `K` and its fingerprint
//! `SOURCE`. `rerail-c-directives` runs them (`native_cpu_pp::run_aot`) against the interpreter,
//! and times them beside the JIT's code of the same IR.

#[cfg(feature = "generated")]
#[allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_imports, unused_parens, non_snake_case, clippy::all)]
mod generated {
    pub mod pp_k0 {
        include!(concat!(env!("OUT_DIR"), "/pp_k0.rs"));
    }
    pub mod pp_k16 {
        include!(concat!(env!("OUT_DIR"), "/pp_k16.rs"));
    }
}

/// An ahead-of-time build: its entry, its `#if` depth, its fingerprint, and its functions
/// structured and in all (`clif_rust`'s `STRUCTURED`). C's and C++'s: the dialect is a run's.
pub type Build = (unsafe extern "C" fn(u64) -> u64, usize, u64, (usize, usize));

/// Each build (none without the `generated` feature).
#[cfg(feature = "generated")]
pub const BUILDS: &[Build] = &[
    (generated::pp_k0::pp_run, generated::pp_k0::K, generated::pp_k0::SOURCE, generated::pp_k0::STRUCTURED),
    (generated::pp_k16::pp_run, generated::pp_k16::K, generated::pp_k16::SOURCE, generated::pp_k16::STRUCTURED),
];

#[cfg(not(feature = "generated"))]
pub const BUILDS: &[Build] = &[];
