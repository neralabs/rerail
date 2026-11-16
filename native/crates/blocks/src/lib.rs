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
//! - **Blocks of the engine:** `facts`, `chunk`, `program` and `journal`. On macOS, `gpu` holds the Metal
//!   kernels, which read and write the same structures through generated MSL.
//!
//! Descriptors are never `Copy`: a reference is only meaningful at its own address, so a descriptor is only ever
//! reached by reference into its block.

pub mod block;
pub mod chunk;
pub mod facts;
pub mod journal;
pub mod program;
pub mod refs;
pub mod relation;
pub mod rows;
#[cfg(target_os = "macos")]
pub mod gpu;

pub use block::{Block, Frozen, Header};
pub use refs::{Ptr, Span};

/// A type whose bytes are its value: no pointers, no padding the compiler chose (`structure!` checks).
///
/// # Safety
/// Implementors are `#[repr(C)]` with every byte a named field, and hold no address.
pub unsafe trait Pod: 'static {}

/// How a type is declared in MSL, and the declarations it needs first.
pub trait MslField {
    fn ty() -> String;
    fn decl(name: &str) -> String {
        format!("{} {}", Self::ty(), name)
    }
    fn deps(_out: &mut Vec<String>) {}
}

macro_rules! scalar {
    ($($t:ty => $m:literal),*) => {$(
        unsafe impl Pod for $t {}
        impl MslField for $t { fn ty() -> String { $m.into() } }
    )*};
}
scalar!(u8 => "uchar", u16 => "ushort", u32 => "uint", u64 => "ulong", i32 => "int", i64 => "long");
// A field that changes while its block is open (a counter, a chunk's offset, a span set at freeze) is an atomic:
// written through a shared reference, which Rust allows only by interior mutability. On ARM a relaxed load is a
// plain load. MSL declares it as the plain type; a kernel that claims casts it to `atomic_uint`.
scalar!(std::sync::atomic::AtomicU32 => "uint", std::sync::atomic::AtomicU64 => "ulong", std::sync::atomic::AtomicI64 => "long");

unsafe impl<T: Pod, const N: usize> Pod for [T; N] {}
impl<T: MslField, const N: usize> MslField for [T; N] {
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

/// The MSL declarations of `T` and of everything it nests: structs in dependency order, each followed by its
/// size and offset assertions against the Rust layout.
pub fn msl_of<T: MslField>() -> String {
    let mut out = Vec::new();
    T::deps(&mut out);
    out.concat()
}

/// Declares a structure that lives in a block: `#[repr(C)]`, every byte a named field (padding is an error, so
/// MSL cannot lay it out differently), never `Copy`, and with its MSL text and offset assertions.
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
        impl $crate::MslField for $name {
            fn ty() -> String { stringify!($name).into() }
            fn deps(out: &mut Vec<String>) {
                $(<$t as $crate::MslField>::deps(out);)*
                let mut s = format!("struct {} {{\n", stringify!($name));
                $(s += &format!("    {};\n", <$t as $crate::MslField>::decl(stringify!($f)));)*
                s += "};\n";
                s += &format!("static_assert(sizeof({0}) == {1}, \"{0}: size\");\n", stringify!($name), core::mem::size_of::<$name>());
                $(s += &format!("static_assert(__builtin_offsetof({0}, {1}) == {2}, \"{0}.{1}\");\n",
                    stringify!($name), stringify!($f), core::mem::offset_of!($name, $f));)*
                if !out.contains(&s) { out.push(s); }
            }
        }
    };
}

/// The hash every index and every kernel uses for a key: one value at a time, the same arithmetic in MSL.
pub const SEED: u64 = 0x243F_6A88_85A3_08D3;
#[inline]
pub fn mix(h: u64, v: i64) -> u64 {
    let mut h = h ^ (v as u64);
    h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
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
