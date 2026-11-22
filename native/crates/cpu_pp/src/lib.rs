//! The C preprocessor's words on the CPU, on the compiler's front end (`rerail-compiler`, whose `pp_words` are the
//! words and their interpreter): the program lowered to Cranelift (`native_cpu_pp`), and the same IR as UKL for the
//! device (`clif_ukl`). The sources stay in the compiler's folder (`crates/compiler/src`); this crate mounts them.

// The compiler's passes hand each other plain tuples of tables; naming each would add nothing.
#![allow(clippy::type_complexity)]

#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;

// The crates below under their modules' names, so the mounted sources' `crate::records`, `crate::block`, ... read them.
#[allow(unused_imports)]
use rerail_compiler::*;

#[path = "../../compiler/src/native_cpu_pp.rs"]
pub mod native_cpu_pp;
#[path = "../../compiler/src/clif_ukl.rs"]
pub mod clif_ukl;
