//! The engine's Metal runtime, in a crate of its own so that the GPU code that needs no engine
//! types rebuilds without the engine: the device, the buffer budget and its reservations,
//! mapped no-copy buffers (`MappedBuffer`), the execution lock, compiled pipelines, and the
//! launch layer's per-evaluation budget (`launch`).
//!
//! `test-hooks` gives each test thread its own buffer ledger (`test_pool`) and the buffer
//! observations, for the engine's tests as for this crate's.
#![cfg(target_os = "macos")]
#![allow(clippy::missing_safety_doc)]

/// Block programs on Metal (decision 0066): the interpreter and generated MSL, over a batch of
/// files read in place from source slabs.
pub mod block_program;
pub mod buffer_observation;
pub mod launch;
pub mod launch_budget;
/// Pre-pinned blocks (decision 0039): the device's scratch buffers, made once and reused.
pub mod pool;
pub mod runtime;
/// The slab residency's arena and sources (decision 0032).
pub mod slabs;
#[cfg(any(test, feature = "test-hooks"))]
pub mod test_pool;

pub use rerail_compiler::Error;

/// The system's default device, once.
pub static METAL_DEVICE: std::sync::LazyLock<Option<metal::Device>> = std::sync::LazyLock::new(metal::Device::system_default);
