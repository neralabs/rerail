//! UKL kernels the engine runs as Rust (decision 0567 step 3). Each is rerail-ukl-vec's Rust (`rs.rs`) of a kernel's
//! vector tape, checked in (rerail-ukl-vec's tests write it again and fail if it differs), with the digest of the
//! source and defines it was written from (`sources.rs`). The CPU device runs a kernel here in place of its
//! compiler's build where a program's text and defines have that digest (`kernels`), and its compiler's build
//! elsewhere: another source, another format, another processor.

#[cfg(target_arch = "aarch64")]
pub mod label_parents_w8;
#[cfg(target_arch = "aarch64")]
pub mod label_leaves_w8;
pub mod join_program;
pub mod lanes;
mod sources;
#[cfg(test)]
mod sort_merge_spike;

pub use sources::*;

/// The digests of `join_stages.uglykl` as the join program (`JOIN_PROGRAM`, decisions 0603, 0674) for views of format
/// 0 (several), 16 and 16z, on a device other than Metal (`metal_join::program_pipe`): `join_run`, written by hand
/// (`join_program`), which reads the views' format as the compiled reads do, so one Rust for each. Not `sources.rs`, which rerail-ukl-vec's tests write: device-metal's test (`join_rust`) holds these to
/// the source.
pub const JOIN_PROGRAM: [u64; 3] = [0x256eb4067a507908, 0x50b71a4a8e6e4cce, 0x3e26479571f84a3f];

/// A group's launch, as a kernel's Rust reads it: its group's position, its threads' (each dimension's), and the
/// groups'.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Launch {
    pub group: [u32; 3],
    pub block: [u32; 3],
    pub groups: [u32; 3],
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
        let l = Launch { group: [g, 0, 0], block: [block, 1, 1], groups: [groups, 1, 1] };
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

/// The join program's kernel (decision 0674: its chain's driver): on the CPU it runs only as its SIMD Rust
/// (`join_program::join_run`), never its UKL's build, until the user says otherwise. Every other kernel runs its Rust
/// where it has it, else its UKL.
pub const JOIN_KERNELS: [&str; 1] = ["join_run"];

/// Whether `text`, specialized by `defines`, is the join program (`JOIN_PROGRAM` defined, its kernels compiled).
pub fn is_join_program(text: &str, defines: &[(&str, &str)]) -> bool {
    (defines.iter().any(|(n, _)| *n == "JOIN_PROGRAM") || text.contains("#define JOIN_PROGRAM ")) && text.contains(", join_run,")
}

/// The kernels of the program `text` (specialized by `defines`) that run as Rust on this processor: each its name
/// and Rust. None where no kernel here was written from that text and those defines.
///
/// The join program whose text or defines are not the ones `join_program` was written from is an error: its UKL's build
/// never runs in the Rust's place (it is orders of magnitude slower, and nothing would say so).
pub fn kernels(text: &str, defines: &[(&str, &str)]) -> Result<Vec<(&'static str, Kernel)>, String> {
    let d = digest(text, defines);
    let mut out: Vec<(&'static str, Kernel)> = Vec::new();
    #[cfg(target_arch = "aarch64")]
    if d == LABEL_PASSES_16 {
        out.push(("label_parents", label_parents_w8::label_parents_w8));
    }
    if JOIN_PROGRAM.contains(&d) {
        out.push(("join_run", join_program::join_run));
    } else if is_join_program(text, defines) {
        return Err(format!(
            "the join program's Rust (rerail_ukl_rust::join_program::join_run) was not written from this source (digest {d:#018x}): \
             bring join_program to the source and JOIN_PROGRAM to its digests. Its UKL's build is not run in the Rust's place."
        ));
    }
    Ok(out)
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
    /// A join program that is not the source `join_run` was written from is refused: its UKL's build does not run in
    /// the Rust's place.
    #[test]
    fn a_changed_join_program_is_refused() {
        let text = "#define JOIN_PROGRAM 1\nDEFINE_KERNEL2(join, join_run, ...)\n";
        let e = super::kernels(text, &[]).unwrap_err();
        assert!(e.contains("join_run") && e.contains("not written from this source"), "{e}");
        // The same source without `JOIN_PROGRAM` compiles none of those kernels.
        assert!(super::kernels("DEFINE_KERNEL2(join, join_run, ...)\n", &[]).unwrap().is_empty());
    }

    #[test]
    fn another_program_has_no_rust() {
        assert!(super::kernels("kernel text", &[("UKL_CONSTANT_0", "16")]).unwrap().is_empty());
    }
}
