//! UKL kernels the engine runs as Rust (decision 0567 step 3). Each is rerail-ukl-vec's Rust (`rs.rs`) of a kernel's
//! vector tape, checked in (rerail-ukl-vec's tests write it again and fail if it differs), with the digest of the
//! source and defines it was written from (`sources.rs`). The CPU device runs a kernel here in place of its
//! compiler's build where a program's text and defines have that digest (`kernels`), and its compiler's build
//! elsewhere: another source, another format, another processor.

#[cfg(target_arch = "aarch64")]
pub mod label_parents_w8;
#[cfg(target_arch = "aarch64")]
pub mod label_leaves_w8;
pub mod join_run;
mod sources;

pub use sources::*;

/// A group's launch, as a kernel's Rust reads it: its group's position, its threads' (each dimension's), and the
/// groups'; and the reads written once in CLIF as the CPU compiled them, each by its place in `rerail_clif::reads::READS`
/// (`rerail_clif::jit::Reads::table`), which a kernel's Rust calls (rerail-ukl-vec's `rs::Read`); null for a kernel
/// that calls none.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Launch {
    pub group: [u32; 3],
    pub block: [u32; 3],
    pub groups: [u32; 3],
    pub reads: *const *const u8,
}

/// The reads written once in CLIF, compiled for the CPU (`rerail_clif::jit::Reads`): their table in
/// `rerail_clif::reads::READS`' order, which a kernel's Rust calls through (`Launch::reads`), and the code it points
/// into, held as long as the table.
pub struct Reads {
    table: Vec<*const u8>,
    _code: Box<dyn std::any::Any>,
}

// SAFETY: the table and the code are written once, before `new`, and only read after: each read a function of its
// arguments, callable from any thread.
unsafe impl Send for Reads {}
unsafe impl Sync for Reads {}

impl Reads {
    /// # Safety
    ///
    /// `table` holds every compiled read in `READS`' order, its code `code`'s, unchanged while `code` lives.
    pub unsafe fn new(table: Vec<*const u8>, code: Box<dyn std::any::Any>) -> Self {
        Reads { table, _code: code }
    }

    /// The table, as `Launch::reads`.
    pub fn table(&self) -> *const *const u8 {
        self.table.as_ptr()
    }
}

/// Whether any of `host`'s kernels calls the compiled reads (and so needs `Launch::reads`).
pub fn calls_reads(host: &[(&'static str, Kernel)]) -> bool {
    host.iter().any(|(name, _)| *name == "join_run")
}

/// A kernel's Rust: one group's threads. `argv` holds the address of each argument's bytes (a buffer's: of its
/// address), then the shim's two past them: the groups' counts (none: null) and their first's position.
pub type Kernel = unsafe fn(*const *const u8, &Launch);

/// Kernel `k` over `groups` groups of `block` threads (1-D), `argv` its arguments (each the address of its bytes).
///
/// # Safety
///
/// `argv` holds `k`'s arguments, its buffers as large as the launch reads and writes.
pub unsafe fn launch(k: Kernel, argv: &[*const u8], groups: u32, block: u32) {
    for g in 0..groups {
        let l = Launch { group: [g, 0, 0], block: [block, 1, 1], groups: [groups, 1, 1], reads: std::ptr::null() };
        unsafe { k(argv.as_ptr(), &l) };
    }
}

/// A program's digest: FNV-1a over its defines (each `#define NAME VALUE` and a newline, as the preprocessor reads
/// them) and then its text, so a text with its defines written at its top has the digest of the text and defines.
pub fn digest(text: &str, defines: &[(&str, &str)]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h = (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (n, v) in defines {
        eat(format!("#define {n} {v}\n").as_bytes());
    }
    eat(text.as_bytes());
    h
}

/// The kernels of the program `text` (specialized by `defines`) that run as Rust on this processor: each its name
/// and Rust. None where no kernel here was written from that text and those defines.
pub fn kernels(text: &str, defines: &[(&str, &str)]) -> Vec<(&'static str, Kernel)> {
    let d = digest(text, defines);
    let mut out: Vec<(&'static str, Kernel)> = Vec::new();
    #[cfg(target_arch = "aarch64")]
    if d == LABEL_PASSES_16 {
        out.push(("label_parents", label_parents_w8::label_parents_w8));
    }
    if JOIN_PROGRAM.contains(&d) {
        out.push(("join_run", join_run::join_run));
    }
    out
}

#[cfg(test)]
mod tests {
    /// A program's defines are its text's top lines to the digest: the text the tests write the kernels from (the
    /// define written in it) and the engine's (the define given) are one program.
    #[test]
    fn a_define_given_is_a_define_written() {
        let text = "UKL_CONSTANT_U32(tape_format)\nkernel text\n";
        assert_eq!(super::digest(text, &[("UKL_CONSTANT_0", "16")]), super::digest(&format!("#define UKL_CONSTANT_0 16\n{text}"), &[]));
        assert_ne!(super::digest(text, &[("UKL_CONSTANT_0", "15")]), super::digest(text, &[("UKL_CONSTANT_0", "16")]));
    }

    /// Another text, or the same with another define, has no kernel here: its compiler's build runs.
    #[test]
    fn another_program_has_no_rust() {
        assert!(super::kernels("kernel text", &[("UKL_CONSTANT_0", "16")]).is_empty());
    }
}
