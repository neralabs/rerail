//! A spike of decision 0266. A block is one of the engine's few big things (a chunk, a phase's facts, a
//! program, a run's outputs, a wave's journal). It holds every structure that belongs to that thing, nested
//! below a typed root, with self-relative references held only by descriptors.
//!
//! Its layers:
//!
//! - **The block** (`block`): one mapping (anonymous, or a file), bump-allocated, frozen once, and checked by its
//!   header (magic, kind, state, length).
//! - **References** (`refs`): `Ptr<T>` and `Span<T>`, each relative to its own address, so a range of a block
//!   moves by `memcpy`.
//! - **Rows** (`rows`): columns in chunks that double. A row's chunk is one `clz`, and claims are atomic, so
//!   many host threads or GPU threads can append.
//! - **Relations** (`relation`): rows, kills, dead bits, a canonical order, hash indexes and statistics, all in
//!   one descriptor. `freeze` builds what reads need, so that they come in a fixed order. `View` reads a full
//!   relation, or a base and its delta, as one.
//! - **Blocks of the engine:** `facts`, `chunk`, `program` and `journal`. `gpu` holds the join's UKL
//!   kernels, which read and write the same structures through generated UKL, on every backend.
//!
//! Descriptors are never `Copy`: a reference is only meaningful at its own address, so a descriptor is only ever
//! reached by reference into its block.

// blocks' `clippy.toml` lists the twins' disallowed paths (decision 0686): denied only where the twins' rules hold, in
// the layer writing the twins call (`relation::layer_write`); the rest of blocks may use them.
#![allow(clippy::disallowed_methods, clippy::disallowed_types, clippy::disallowed_macros)]

pub mod alloc;
pub mod atoms;
pub mod bistate;
pub mod blob;
pub mod block;
pub mod bloom;
pub mod change;
pub mod chunk;
pub mod col;
#[cfg(any(unix, windows))]
pub mod deferrable;
pub mod facts;
pub mod free;
pub mod gens;
pub mod ids;
/// A relation's reads in UKL, for every kernel that reads a relation in its block (decision 0325; the host's are
/// written directly, decision 0536, and the CLIF's are the JIT's, `rerail_clif::relation`): the library's header,
/// which a kernel source including it is preprocessed with (the device layer's programs; decision 0440 §3.3).
pub const RELATION_READS: &str = "#include <ukl/reads/relation.h>\n";
pub mod journal;
pub mod kept;
pub mod ledger;
pub mod maps;
pub mod mapped;
pub use rerail_vector::vector;
pub mod mem;
pub mod packed;
pub mod ranged;
pub mod program;
pub mod refs;
pub mod region;
pub mod order;
pub mod relation;
pub mod rows;
pub mod scratch;
pub mod sketch;
pub mod sort;
pub mod table;
pub mod layer;

/// The flag an anonymous mapping takes to be address space reserved, its pages committed as they are written: Linux's
/// `MAP_NORESERVE`. Without it Linux's overcommit heuristic refuses a reservation larger than memory and swap (a 64
/// GiB scratch region on a 16 GiB runner), which macOS grants as it maps every anonymous region so.
#[cfg(target_os = "linux")]
pub(crate) const RESERVE_ONLY: libc::c_int = libc::MAP_NORESERVE;
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) const RESERVE_ONLY: libc::c_int = 0;
#[cfg(windows)]
mod windows;
pub mod gpu;

pub use block::{Block, Cells, Frozen, Header};
pub use refs::{Ptr, Span};

/// A type whose bytes are its value (`rerail_vector::Pod`, below the tape format: decision 0628 §3).
pub use rerail_vector::Pod;

/// How a type is declared in UKL, and the declarations it needs first: the floor's one definition
/// (`rerail_delta_core::kernel`), written into a `fmt::Write` with no allocator; `structure!` declares a structure
/// that lives in a block.
pub use rerail_delta_core::kernel::{Decls, KernelField};
pub use rerail_delta_core::structure;
/// The hash every index and every kernel uses for a key: the floor's (`rerail_delta_core::hash`).
pub use rerail_delta_core::hash::{SEED, key_hash, mix};

/// The UKL declarations of `T` and of everything it nests: structs in dependency order, each followed by its size and
/// offset assertions against the Rust layout.
pub fn ukl_of<T: KernelField>() -> String {
    struct Out {
        names: Vec<&'static str>,
        text: String,
    }
    impl Decls for Out {
        fn declare(&mut self, name: &'static str, text: &dyn Fn(&mut dyn std::fmt::Write) -> std::fmt::Result) -> std::fmt::Result {
            if self.names.contains(&name) {
                return Ok(());
            }
            self.names.push(name);
            text(&mut self.text)
        }
    }
    let mut out = Out { names: Vec::new(), text: String::new() };
    T::deps(&mut out).expect("a String takes every write");
    out.text
}

/// Instructions retired by this process so far (macOS), for measuring work, not wall time.
pub fn retired() -> u64 {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        if libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) == 0 {
            return info.ri_instructions;
        }
    }
    0
}

/// This process's physical footprint in bytes (macOS), the figure a memory cap is judged by.
pub fn footprint() -> u64 {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        if libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) == 0 {
            return info.ri_phys_footprint;
        }
    }
    0
}

// The crate's own tests make their folders the user's alone, as the engine's test binaries do (decision 0574): a
// host's umask of `002` made a fixture's folder `775`, which the mapped files' rules refuse.
#[cfg(test)]
#[path = "../../../src/test_private_folders.rs"]
mod test_private_folders;
