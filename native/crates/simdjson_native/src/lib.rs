//! The platform's part of reading JSON: simdjson (the C++ library, vendored at
//! `native/vendor/simdjson-5.0.5`) and the v9np kernels (`kernel.inc`), compiled for each of
//! simdjson's implementations the target has. Everything here is native code behind a C ABI;
//! `rerail-simdjson` is the portable side (the `Tape`, the index writer).
//!
//! - `structurals`: simdjson's DOM parser, which picks its implementation at run time, and the
//!   structural index its stage 1 found. Any CPU.
//! - `Kernel`: stage 1 with the source's SHA-256 in it and our stage 2, which writes the tape into the
//!   caller's buffers (`Sink`). One for each implementation, with the CPU's SHA-256 instructions and
//!   without: `Kernel::ALL` lists them best first, and `Kernel::runs` says which this CPU runs.

use std::ffi::{CStr, c_char};

unsafe extern "C" {
    fn rerail_simdjson_structurals(src: *const c_char, len: usize, indexes: *mut *const u32, count: *mut u32) -> i32;
    fn rerail_simdjson_kernel_runs(kernel: u32) -> i32;
    fn rerail_simdjson_v9np(kernel: u32, src: *const c_char, len: usize, sink: *mut Sink) -> i32;
    fn rerail_simdjson_v9np_jsonc(kernel: u32, src: *const c_char, len: usize, sink: *mut Sink) -> i32;
    fn rerail_simdjson_stage1(kernel: u32, src: *const c_char, len: usize, hashed: i32) -> i64;
    fn rerail_simdjson_v9np_rewalks(kernel: u32) -> u64;
    fn rerail_simdjson_release_above(keep: usize);
    fn rerail_simdjson_error_message(code: i32) -> *const c_char;
    fn rerail_simdjson_implementation() -> *const c_char;
}

/// The tape's buffers as a kernel's stage 2 writes them (`rerail_v9np_sink`): the caller's, whose
/// pointers, lengths and capacities it reads, grown through `grow` (which `ctx` is passed to).
#[repr(C)]
pub struct Sink {
    pub col: [*mut u8; 5],
    pub len: [usize; 5],
    pub cap: [usize; 5],
    pub width: [u8; 3],
    pub ctx: *mut std::ffi::c_void,
    pub grow: unsafe extern "C" fn(*mut Sink, *const usize),
    pub kind: [u16; 12],
    pub key_field: u16,
    pub value_field: u16,
    pub codes: u32,
    pub code_kind: [u16; 36],
    pub code_field: [u16; 36],
    pub code_value: [u8; 36],
    pub digest: [u8; 32],
}

/// A v9np kernel: simdjson's stage 1 for one of its implementations, the source hashed in it, and our
/// stage 2. Numbered as the shim numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Kernel {
    Arm64Sha = 0,
    Arm64 = 1,
    IcelakeSha = 2,
    Icelake = 3,
    HaswellSha = 4,
    Haswell = 5,
    WestmereSha = 6,
    Westmere = 7,
}

impl Kernel {
    /// Every kernel, best first: on a CPU, the first that `runs` is the fastest it has (wider vectors
    /// first; the SHA-256 instructions before portable code).
    pub const ALL: [Kernel; 8] = [
        Kernel::Arm64Sha,
        Kernel::Arm64,
        Kernel::IcelakeSha,
        Kernel::Icelake,
        Kernel::HaswellSha,
        Kernel::Haswell,
        Kernel::WestmereSha,
        Kernel::Westmere,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kernel::Arm64Sha => "simdjson v9np: arm64, SHA2",
            Kernel::Arm64 => "simdjson v9np: arm64",
            Kernel::IcelakeSha => "simdjson v9np: icelake, SHA-NI",
            Kernel::Icelake => "simdjson v9np: icelake",
            Kernel::HaswellSha => "simdjson v9np: haswell, SHA-NI",
            Kernel::Haswell => "simdjson v9np: haswell",
            Kernel::WestmereSha => "simdjson v9np: westmere, SHA-NI",
            Kernel::Westmere => "simdjson v9np: westmere",
        }
    }

    /// Whether this build has the kernel and this CPU runs it: simdjson's check of its
    /// implementation's instructions, and the SHA-256 instructions where it hashes with them.
    pub fn runs(self) -> bool {
        // SAFETY: reads the shim's table.
        unsafe { rerail_simdjson_kernel_runs(self as u32) != 0 }
    }

    /// The fastest kernel this CPU runs.
    pub fn best() -> Option<Kernel> {
        Kernel::ALL.into_iter().find(|k| k.runs())
    }

    /// Writes `src`'s tape and digest into `sink`: 0, simdjson's error code, or -1 where the kernel does
    /// not run here (or `src` is past 2 GB).
    ///
    /// # Safety
    /// `sink`'s `col`, `len` and `cap` describe buffers `grow` can grow, with `ctx` what it expects.
    pub unsafe fn tape(self, src: &[u8], sink: &mut Sink) -> i32 {
        // SAFETY: `src` is read for its length; the caller vouches for the sink.
        unsafe { rerail_simdjson_v9np(self as u32, src.as_ptr().cast(), src.len(), sink) }
    }

    /// As `tape`, `src` read as JSONC: each comment (`//` to the line's end, `/* */`) blanked for stage 1
    /// and written to the tape as a leaf where tree-sitter-json places an extra. For a file `tape`
    /// refused; a strict file gives `tape`'s tape.
    ///
    /// # Safety
    /// As `tape`.
    pub unsafe fn tape_jsonc(self, src: &[u8], sink: &mut Sink) -> i32 {
        // SAFETY: as `tape`.
        unsafe { rerail_simdjson_v9np_jsonc(self as u32, src.as_ptr().cast(), src.len(), sink) }
    }

    /// Stage 1 alone, for a measurement: the implementation's own, or with `hashed` ours (the source
    /// hashed in it). The structural index's length, simdjson's error code negated, or -1.
    pub fn stage1(self, src: &[u8], hashed: bool) -> i64 {
        // SAFETY: `src` is read for its length.
        unsafe { rerail_simdjson_stage1(self as u32, src.as_ptr().cast(), src.len(), i32::from(hashed)) }
    }

    /// This thread's stage 2 walks made again at other widths (a column's prediction wrong).
    pub fn rewalks(self) -> u64 {
        // SAFETY: reads a thread-local counter.
        unsafe { rerail_simdjson_v9np_rewalks(self as u32) }
    }
}

/// A thread keeps its parsers' buffers for files up to this size; a larger file's are freed once
/// it is read (simdjson's DOM parser holds about 14 bytes a source byte).
pub const KEEP: usize = 1 << 20;

/// Frees this thread's parsers and buffers where they hold more than `keep` bytes of source.
pub fn release_above(keep: usize) {
    // SAFETY: frees this thread's own buffers, which no reference outlives (`structurals` lends its
    // index only within its call).
    unsafe { rerail_simdjson_release_above(keep) }
}

/// Why simdjson refused a document: its error code and message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub i32);

impl Error {
    pub fn message(self) -> &'static str {
        // SAFETY: simdjson's messages are static, NUL-terminated strings.
        unsafe { CStr::from_ptr(rerail_simdjson_error_message(self.0)) }.to_str().unwrap_or("simdjson error")
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Parses `src` with simdjson's DOM parser and gives `read` its structural index; `Err` where simdjson
/// refuses it (invalid JSON, an empty document, a number past its range, nesting past 1,024).
pub fn structurals<R>(src: &[u8], read: impl FnOnce(&[u32]) -> R) -> Result<R, Error> {
    let (mut indexes, mut count) = (std::ptr::null(), 0u32);
    // SAFETY: `src` is read for `len` bytes; the index stays valid until this thread's next call,
    // which nothing in `read` can make before it returns (the slice is not kept past it).
    let code = unsafe { rerail_simdjson_structurals(src.as_ptr().cast(), src.len(), &mut indexes, &mut count) };
    let result = if code != 0 {
        Err(Error(code))
    } else {
        let index = if count == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(indexes, count as usize) } };
        Ok(read(index))
    };
    if src.len() > KEEP {
        release_above(KEEP);
    }
    result
}

/// The implementation simdjson's DOM parser chose for this CPU (`arm64`, `haswell`, …).
pub fn implementation() -> &'static str {
    // SAFETY: the name is a static string of simdjson's.
    unsafe { CStr::from_ptr(rerail_simdjson_implementation()) }.to_str().unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A target has a kernel for each implementation simdjson compiles for it, and this CPU runs one.
    #[test]
    fn this_cpu_runs_a_kernel() {
        let runs: Vec<_> = Kernel::ALL.into_iter().filter(|k| k.runs()).map(Kernel::name).collect();
        eprintln!("simdjson's implementation: {}; kernels this CPU runs: {runs:?}", implementation());
        assert!(Kernel::best().is_some(), "no kernel runs on this CPU ({})", implementation());
        // A kernel without the SHA-256 instructions runs wherever its implementation does.
        for pair in Kernel::ALL.chunks(2) {
            assert!(!pair[0].runs() || pair[1].runs(), "{:?} runs and {:?} does not", pair[0], pair[1]);
        }
    }
}
