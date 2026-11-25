//! The digests (`digest`) of the sources the kernels here were written from: rerail-ukl-vec's tests write them
//! with the kernels (`UKL_RS_WRITE=1`), and fail if they differ.

/// `label_passes.uglykl` as the engine assembles it for a batch of format 16 (`label_parents_w8`, `label_leaves_w8`).
pub const LABEL_PASSES_16: u64 = 0x0c753df66dbb7f65;

/// `join_stages.uglykl` as the join program (`JOIN_PROGRAM`, decision 0603) for views of format 0 (several), 16 and
/// 16z: `join_run`, written by hand (the oracle of rerail-ukl-vec's Rust of it), which reads the views' format as the
/// compiled reads do, so one Rust for each.
pub const JOIN_PROGRAM: [u64; 3] = [0x519e167bc84d86cf, 0x6d9bcb7efa1f8b59, 0xe3d2a28c1611f9b6];
