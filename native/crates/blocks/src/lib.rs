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

pub mod alloc;
pub mod atoms;
pub mod block;
pub mod bloom;
pub mod chunk;
pub mod col;
#[cfg(any(unix, windows))]
pub mod deferrable;
pub mod facts;
pub mod free;
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
pub mod sort;

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

/// How a type is declared in UKL, and the declarations it needs first: C's fixed-width types, a structure a
/// `typedef`, its assertions `UKL_STATIC_ASSERT`s, for a kernel on every backend (decision 0440).
pub trait KernelField {
    fn ty() -> String;
    fn decl(name: &str) -> String {
        format!("{} {}", Self::ty(), name)
    }
    fn deps(_out: &mut Vec<String>) {}
}

macro_rules! scalar {
    ($($t:ty => $c:literal),*) => {$(
        impl KernelField for $t { fn ty() -> String { $c.into() } }
    )*};
}
scalar!(u8 => "uint8_t", u16 => "uint16_t", u32 => "uint32_t", u64 => "uint64_t", i32 => "int32_t", i64 => "int64_t");
// A field that changes while its block is open (a counter, a chunk's offset, a span set at freeze) is an atomic:
// written through a shared reference, which Rust allows only by interior mutability. On ARM a relaxed load is a
// plain load. UKL declares it as the plain type; a kernel that claims passes it to `ukl_atomic_add`.
scalar!(std::sync::atomic::AtomicU32 => "uint32_t", std::sync::atomic::AtomicU64 => "uint64_t", std::sync::atomic::AtomicI64 => "int64_t");

impl<T: KernelField, const N: usize> KernelField for [T; N] {
    fn ty() -> String {
        T::ty()
    }
    fn decl(name: &str) -> String {
        format!("{} {}[{}]", T::ty(), name, N)
    }
    fn deps(out: &mut Vec<String>) {
        T::deps(out)
    }
}

/// The UKL declarations of `T` and of everything it nests: structs in dependency order, each followed by its size and
/// offset assertions against the Rust layout.
pub fn ukl_of<T: KernelField>() -> String {
    let mut out = Vec::new();
    T::deps(&mut out);
    out.concat()
}

/// Declares a structure that lives in a block: `#[repr(C)]`, every byte a named field (padding is an error, so
/// a kernel cannot lay it out differently), never `Copy`, and with its UKL text and offset assertions.
#[macro_export]
macro_rules! structure {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[repr(C)]
        pub struct $name { $($(#[$fm])* pub $f: $t),* }
        const _: () = {
            let sum = 0 $(+ core::mem::size_of::<$t>())*;
            assert!(sum == core::mem::size_of::<$name>(), concat!(stringify!($name), ": implicit padding; name it as a field"));
        };
        unsafe impl $crate::Pod for $name {}
        impl $crate::KernelField for $name {
            fn ty() -> String { stringify!($name).into() }
            fn deps(out: &mut Vec<String>) {
                $(<$t as $crate::KernelField>::deps(out);)*
                let mut s = format!("typedef struct {} {{\n", stringify!($name));
                $(s += &format!("    {};\n", <$t as $crate::KernelField>::decl(stringify!($f)));)*
                s += &format!("}} {};\n", stringify!($name));
                s += &format!("UKL_STATIC_ASSERT(sizeof({0}) == {1}, \"{0}: size\");\n", stringify!($name), core::mem::size_of::<$name>());
                $(s += &format!("UKL_STATIC_ASSERT(UKL_OFFSETOF({0}, {1}) == {2}, \"{0}.{1}\");\n",
                    stringify!($name), stringify!($f), core::mem::offset_of!($name, $f));)*
                if !out.contains(&s) { out.push(s); }
            }
        }
    };
}

/// The hash every index and every kernel uses for a key: one value at a time, the same arithmetic in UKL.
pub const SEED: u64 = 0x243F_6A88_85A3_08D3;
/// The arithmetic is the CLIF's `rel_key_mix` (decision 0325 step 5), which the kernels and the JIT call.
#[inline(always)]
pub fn mix(h: u64, v: i64) -> u64 {
    let h = (h ^ v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^ (h >> 29)
}
#[inline]
pub fn key_hash(vals: &[i64]) -> u64 {
    vals.iter().fold(SEED, |h, &v| mix(h, v))
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
