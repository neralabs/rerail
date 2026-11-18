//! The C preprocessing words' compiled code built ahead of time (decision 0348, stage 4b): with
//! the `generated` feature, the JIT's functions as Rust (`build.rs`,
//! `native_cpu_pp::rust_source`), its entry `pp_run` and its fingerprint `SOURCE`.
//! `rerail-c-directives` runs it (`native_cpu_pp::run_aot`) against the interpreter,
//! and times them beside the JIT's code of the same IR.

#[cfg(feature = "generated")]
#[allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_imports, unused_parens, non_snake_case, clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/pp.rs"));
}

/// The ahead-of-time build: its entry, its fingerprint, and its functions structured and in all
/// (`clif_rust`'s `STRUCTURED`). One for C and C++: the dialect is a run's.
pub type Build = (unsafe extern "C" fn(u64) -> u64, u64, (usize, usize));

/// The build (none without the `generated` feature).
#[cfg(feature = "generated")]
pub const BUILD: Option<Build> = Some((generated::pp_run, generated::SOURCE, generated::STRUCTURED));

#[cfg(not(feature = "generated"))]
pub const BUILD: Option<Build> = None;
