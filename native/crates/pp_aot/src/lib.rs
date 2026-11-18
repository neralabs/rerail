//! The C preprocessing words' compiled code built ahead of time (decision 0348, stage 4b): with
//! the `generated` feature, each module is the JIT's functions as Rust (`build.rs`,
//! `native_cpu_pp::rust_source`), its entry `pp_run`, its depth `K` and its fingerprint
//! `SOURCE`. `rerail-c-directives` runs them (`native_cpu_pp::run_aot`) against the interpreter,
//! and times them beside the JIT's code of the same IR.

#[cfg(feature = "generated")]
#[allow(unused_mut, unused_variables, unused_assignments, unreachable_code, unused_labels, unused_unsafe, unused_imports, unused_parens, non_snake_case, clippy::all)]
mod generated {
    pub mod pp_c_k0 {
        include!(concat!(env!("OUT_DIR"), "/pp_c_k0.rs"));
    }
    pub mod pp_cpp_k0 {
        include!(concat!(env!("OUT_DIR"), "/pp_cpp_k0.rs"));
    }
    pub mod pp_cpp_k4 {
        include!(concat!(env!("OUT_DIR"), "/pp_cpp_k4.rs"));
    }
}

/// An ahead-of-time build: its dialect (C++ or not), its entry, its `#if` depth, its
/// fingerprint, and its functions structured and in all (`clif_rust`'s `STRUCTURED`).
pub type Build = (bool, unsafe extern "C" fn(u64) -> u64, usize, u64, (usize, usize));

/// Each build (none without the `generated` feature).
#[cfg(feature = "generated")]
pub const BUILDS: &[Build] = &[
    (false, generated::pp_c_k0::pp_run, generated::pp_c_k0::K, generated::pp_c_k0::SOURCE, generated::pp_c_k0::STRUCTURED),
    (true, generated::pp_cpp_k0::pp_run, generated::pp_cpp_k0::K, generated::pp_cpp_k0::SOURCE, generated::pp_cpp_k0::STRUCTURED),
    (true, generated::pp_cpp_k4::pp_run, generated::pp_cpp_k4::K, generated::pp_cpp_k4::SOURCE, generated::pp_cpp_k4::STRUCTURED),
];

#[cfg(not(feature = "generated"))]
pub const BUILDS: &[Build] = &[];
