//! Relational kernels on every compute lane, and the executor that batches their launches (decision 0098).
//!
//! The logic engine's planner recognizes a few physical forms in a stratum's joins: a counting table per document,
//! a histogram per document, a bit product (overlap counts of set-valued rows), and a transitive closure. Each form
//! is a [`Kernel`] over plain columns, with one meaning, written once in [`cpu`] as the reference. Each lane runs it
//! its own way:
//!
//! - [`Lane::Cpu`]: the cores (NEON), on the executor's pool;
//! - [`Lane::Sme`]: the CPU's matrix unit, one worker per cluster (`sme`);
//! - [`Lane::Gpu`]: Metal compute, a batch as one command buffer (`gpu`);
//! - [`Lane::Ane`]: the Neural Engine through Core ML, against fixed relations held as 1-bit constants (`ane`).
//!
//! The [`exec::Executor`] takes a [`exec::Batch`] of independent launches (a level's), places each on a lane by
//! price, load and energy ([`price`]), and runs them as jobs: a job a launch on the cores' pool (`RERAIL_THREADS`
//! threads), one submission a batch on each other lane. Every lane runs at the same time, and the [`score`] board
//! counts every job. Every lane's output is the reference's, exactly: the tests compare them.

pub mod budget;
pub mod col;
pub mod cost_model;
pub mod cpu;
pub mod exec;
pub mod kernel;
pub mod lanes;
pub mod price;
pub mod rows;
pub mod score;
pub mod stream;
pub mod sql;

#[cfg(all(feature = "sme", target_os = "macos", target_arch = "aarch64"))]
pub mod sme;

#[cfg(all(feature = "gpu", target_os = "macos"))]
pub mod gpu;

#[cfg(all(feature = "ane", target_os = "macos"))]
pub mod ane;

pub use exec::{Batch, Done, Executor, Job, Launch};
pub use col::{Col, Gate};
pub use kernel::{Bits, Kernel, Output, PairView, Pairs};
pub use lanes::Lane;
