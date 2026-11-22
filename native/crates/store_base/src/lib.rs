//! The content store's base: its keys, ids, errors and payloads (`content_store::base`), and the modules every part
//! of the store reads that read no `ContentStore`: the segments, the rows, the postings, the vocabulary,
//! compression, the journal, the hazards, the scope leases and the private files. With the analysis scope's owner
//! and names (`analysis_scope::base`), the checkout's source tree, the prefilter's probes and `Canonical`. Below
//! the store (rerail-store), with its observer and git, which re-exports it.
//!
//! The sources are the engine's, in place (`native/src`): only the build moved.

// The store's files name the trait below as `rerail_store::Canonical`, as every crate that compiles them does.
extern crate self as rerail_store;

/// A path's canonical name: absolute, its links resolved, and the one name the engine gives it on every system. On
/// Windows that is the drive's name (`C:\a\b`), not the verbatim one std's `canonicalize` returns (`\\?\C:\a\b`), in
/// which `/` is no separator and which compares unequal to the drive's (`dunce`). Elsewhere it is std's. The engine's
/// and the store's every canonical path (`.canonical()`, not `.canonicalize()`), of anything that names a path.
pub trait Canonical {
    fn canonical(&self) -> std::io::Result<std::path::PathBuf>;
}

impl<P: AsRef<std::path::Path> + ?Sized> Canonical for P {
    fn canonical(&self) -> std::io::Result<std::path::PathBuf> {
        dunce::canonicalize(self)
    }
}

#[path = "../../../src/content_store"]
pub mod content_store {
    pub mod base;
    pub use base::*;
    /// The analysis scope's owner, its run's source and the scopes' names; the scope's records are the store's.
    pub mod analysis_scope {
        pub mod base;
        pub use base::*;
    }
    pub mod bounded_insert;
    pub mod compression;
    pub use compression::{compress, decompress, decompress_chunks, decompress_head, decompress_into};
    pub mod hazards;
    pub mod journal;
    pub mod postings;
    #[allow(unused_imports)]
    pub use postings::append_delta;
    #[cfg(any(test, feature = "test-hooks"))]
    pub use postings::segment_positions;
    pub mod private_files;
    pub use private_files::prepare_cache_directory;
    pub mod read_cache;
    pub mod rows;
    pub mod scope_lease;
    pub mod segments;
    pub use segments::{SealedSegments, SegmentBytes, SegmentWriter};
    pub mod vocabulary;
    pub mod working_set;
    pub use rerail_blocks::block::Flush;
}
/// The checkout the engine runs from, found as it runs (decision 0224).
#[path = "../../../src/source_tree.rs"]
pub mod source_tree;
/// A regex's literal probe (the prefilter's; the store's vocabulary uses it too).
#[path = "../../../src/code_rule_dsl/prefilter/probes.rs"]
pub mod probes;
