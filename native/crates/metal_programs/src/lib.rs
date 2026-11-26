//! The compiler's programs on the device, over rerail-metal's runtime: the block programs (`block_program`, its
//! serial form `block_program::serial`) and the preprocessor's passes (`pp_on`). Apart from the runtime, so that the
//! runtime (which the language layer's tapes are placed by) needs no compiler, and an edit to the compiler rebuilds
//! only the crates that run its programs.
#![cfg(any(target_os = "macos", target_os = "linux", windows))]
#![allow(clippy::missing_safety_doc)]

pub mod block_program;
pub mod pp_on;

// The runtime's modules, by the paths these files had in it.
use rerail_metal::{Error, lane, launch, pool, runtime, slabs};

// A resident source's margin is the block programs' widest window back (`block_native::BACK`): the runtime keeps
// its own constant, and this crate, which sees both, holds them equal.
const _: () = assert!(rerail_metal::slabs::SOURCE_MARGIN == rerail_compiler::block_native::BACK);

/// Releases the serial block programs' kernels (the residency's drop, with `rerail_metal::runtime::drop_pipelines`).
pub fn drop_kernels() {
    block_program::serial::drop_kernels();
}
