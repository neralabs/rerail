//! Anchor-selection laboratory: a policy's anchor plans, lowered by the current code and run over
//! captured batch databases (`RERAIL_SQL_PROFILE_DIR`). Seconds per iteration, no addon build, no
//! corpus parse. Reports SQLite VM steps (load-independent) and rows per plan (for parity).
//!
//! RERAIL_LAB_POLICY=<policy.json>[:<policy.json>...] RERAIL_LAB_CAPTURES=<dir> [RERAIL_LAB_LIMIT=n]
//!   [RERAIL_LAB_PER_PLAN=1] (each plan lowered and run alone, after the batch)
//!   [RERAIL_LAB_BATCH=1] (production batches: each capture's requested files, captures spread
//!     over the run, and a warm vocabulary, which a corpus store fills once per term)
//!   [RERAIL_LAB_DUMP=<dir>] (writes each policy's lowered stages per language)
//!   [RERAIL_LAB_FALLBACK=1] (lowers as the pure-SQL fallback, `lower::fallback`: relations read
//!     the tape through `nera_tree`)
//!   cargo test --release --lib -- --ignored anchor_lab --nocapture
//!
//! From PIAT's `matcher/anchors/sql/lab.rs`, pointed at the content store: a
//! capture is a store holding one batch's prefilter rows (`capture`), opened as a `ContentStore`,
//! statements are lowered with the capture's statistics (`stats::inputs`) and run on its
//! connection. Gone with decoding: clearing `decoded_*` tables and the structural fill.
pub mod capture;
// The lab's runs and tests (`lab_tests.rs`, with `lab/replay.rs`): tests the engine hosts (they read above this
// crate; boot mounts them).
