//! Tier 2 on the CPU, on the compiler's front end (`rerail-compiler`) and tier 1's lowering (`rerail-cpu-block`): the
//! stream program's words lowered to Cranelift (`native_cpu_stream`), the tape they write (`native_cpu_tape`), the
//! functions they are cut into (`native_cpu_clusters`), and the parser of both tiers (`native_cpu_stream::Parser`).
//! Tier 2 read over tier 1's records where the device left them is `tier2_on`. The deprecated Rust lowerings (decision 0550:
//! `rust_stream`, `clif_rust`) are here too: the JIT's direct format 16 runs are decided by `rust_stream`'s facts
//! (`direct_keys`, `open_bound`), where the Rust takes the program. The sources stay in the compiler's folder
//! (`crates/compiler/src`); this crate mounts them.

// The compiler's passes hand each other plain tuples of tables; naming each would add nothing.
#![allow(clippy::type_complexity)]

#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;

// The crates below under their modules' names, so the mounted sources' `crate::records`, `crate::block`, ... read them.
#[allow(unused_imports)]
use rerail_compiler::*;
#[allow(unused_imports)]
use rerail_cpu_block::*;
// The Bitlex compiler, which only the ladder and the tests read.
#[cfg(any(test, feature = "test-hooks"))]
#[allow(unused_imports)]
use rerail_bitlex::*;

#[path = "../../compiler/src/native_cpu_stream.rs"]
pub mod native_cpu_stream;
#[path = "../../compiler/src/native_cpu_tape.rs"]
pub mod native_cpu_tape;
#[path = "../../compiler/src/native_cpu_clusters.rs"]
pub mod native_cpu_clusters;
#[path = "../../compiler/src/clif_rust.rs"]
pub mod clif_rust;
#[path = "../../compiler/src/rust_stream.rs"]
pub mod rust_stream;
/// The Bitlex ladder's checks, for this crate's tests and the engine's.
#[cfg(any(test, feature = "test-hooks"))]
#[path = "../../compiler/src/ladder.rs"]
pub mod ladder;

#[cfg(test)]
#[path = "../../compiler/src/token_local_tests.rs"]
mod token_local_tests;
#[cfg(test)]
#[path = "../../compiler/src/stream_tests.rs"]
mod stream_tests;
#[cfg(test)]
#[path = "../../compiler/src/records_tests.rs"]
mod records_tests;
#[cfg(test)]
#[path = "../../compiler/src/remade_tests.rs"]
mod remade_tests;
#[cfg(test)]
#[path = "../../compiler/src/totals_tests.rs"]
mod totals_tests;
#[cfg(test)]
#[path = "../../compiler/src/delay_tests.rs"]
mod delay_tests;
#[cfg(test)]
#[path = "../../compiler/src/gating_tests.rs"]
mod gating_tests;
#[cfg(test)]
#[path = "../../compiler/src/regions_tests.rs"]
mod regions_tests;
#[cfg(test)]
#[path = "../../compiler/src/outline_tests.rs"]
mod outline_tests;
#[cfg(test)]
#[path = "../../compiler/src/tier2_tests.rs"]
mod tier2_tests;
#[cfg(test)]
#[path = "../../compiler/src/ladder_tests.rs"]
mod ladder_tests;

/// Tier 2 over tier 1's buffers as the Metal backend left them (the device layer's: Metal and Vulkan).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub mod tier2_on;
