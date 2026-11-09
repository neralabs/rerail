//! Relational kernels on every compute lane, and the executor that batches their launches (decision 0098).
//!
//! The logic engine's planner recognizes a few physical forms in a stratum's joins: a counting table per document,
//! a histogram per document, a bit product (overlap counts of set-valued rows), and a transitive closure. Each form
//! is a [`Kernel`] over plain columns, with one meaning, written once in [`cpu`] as the reference. Each lane runs it
//! its own way:
//!
//! - [`Lane::Cpu`]: the cores (NEON), on the rayon pool;
//! - [`Lane::Sme`]: the CPU's matrix unit, one worker per cluster (`sme`);
//! - [`Lane::Gpu`]: Metal compute, a batch as one command buffer (`gpu`);
//! - [`Lane::Ane`]: the Neural Engine through Core ML, against fixed relations held as 1-bit constants (`ane`).
//!
//! The [`exec::Executor`] takes a [`exec::Batch`] of independent launches (a wave), places each on a lane by price,
//! load and energy ([`price`]), and submits each lane's share as one submission. The lanes run at the same time.
//! Every lane's output is the reference's, exactly: the tests compare them.

pub mod cpu;
pub mod exec;
pub mod kernel;
pub mod lanes;
pub mod price;

#[cfg(all(feature = "sme", target_os = "macos", target_arch = "aarch64"))]
pub mod sme;

#[cfg(all(feature = "gpu", target_os = "macos"))]
pub mod gpu;

#[cfg(all(feature = "ane", target_os = "macos"))]
pub mod ane;

pub use exec::{Batch, Done, Executor, Launch};
pub use kernel::{Bits, Kernel, Output};
pub use lanes::Lane;
