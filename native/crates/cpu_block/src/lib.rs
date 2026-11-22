//! Tier 1 on the CPU, on the compiler's front end (`rerail-compiler`): block programs lowered to Cranelift
//! (`native_cpu_block`), their byte-local values on the byte vectors (`native_cpu_lanes`, the front end's `lane_plan`
//! lowered), the outputs the token loop makes from the bytes (`native_cpu_bytes`, `byte_needs` lowered), the walks
//! (`native_cpu_walk`) and row programs (`native_cpu_rows`). The sources stay in the compiler's folder
//! (`crates/compiler/src`); this crate mounts them.

// The compiler's passes hand each other plain tuples of tables; naming each would add nothing.
#![allow(clippy::type_complexity)]

#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;

// The crates below under their modules' names, so the mounted sources' `crate::records`, `crate::block`, ... read them.
#[allow(unused_imports)]
use rerail_compiler::*;

#[path = "../../compiler/src/native_cpu_block.rs"]
pub mod native_cpu_block;
#[path = "../../compiler/src/native_cpu_bytes.rs"]
pub mod native_cpu_bytes;
#[path = "../../compiler/src/native_cpu_lanes.rs"]
pub mod native_cpu_lanes;
#[path = "../../compiler/src/native_cpu_walk.rs"]
pub mod native_cpu_walk;
/// Row programs' Cranelift definer (`block::BlockProgram::rows`, decision 0095 §2).
#[path = "../../compiler/src/native_cpu_rows.rs"]
pub mod native_cpu_rows;
