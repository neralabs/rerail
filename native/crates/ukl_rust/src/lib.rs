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
pub mod lanes;
mod sources;
#[cfg(test)]
mod sort_merge_spike;

pub use sources::*;

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

/// The kernels of the program `text` (specialized by `defines`) that run as Rust on this processor: each its name
/// and Rust. None where the program defines no kernel that has Rust here.
///
/// A program that defines a kernel with Rust here, specialized as that Rust's (`written_for`), but whose text or
/// defines are not the ones the Rust was written from, is an error: never run another build of it in the Rust's
/// place. The interpreter or the host compiler runs it orders of magnitude slower, and nothing would say so.
pub fn kernels(text: &str, defines: &[(&str, &str)]) -> Result<Vec<(&'static str, Kernel)>, String> {
    let d = digest(text, defines);
    let mut out: Vec<(&'static str, Kernel)> = Vec::new();
    #[cfg(target_arch = "aarch64")]
    if d == LABEL_PASSES_16 {
        out.push(("label_parents", label_parents_w8::label_parents_w8));
    }
    if JOIN_PROGRAM.contains(&d) {
        out.extend([("join_grid", join_run::join_grid as Kernel), ("join_phase", join_run::join_phase), ("join_chain", join_run::join_chain)]);
    }
    if out.is_empty()
        && let Some(name) = written_for(text, defines)
    {
        return Err(format!(
            "the kernel {name} has Rust here (rerail_ukl_rust), but this program is not the source it was written from (digest {d:#018x}): \
             bring rerail_ukl_rust to the source and its digests to it. Another build of it is not run in the Rust's place."
        ));
    }
    Ok(out)
}

/// Whether `text`, specialized by `defines`, compiles a kernel that has Rust on this processor: that kernel's name.
/// The join program's kernels where `JOIN_PROGRAM` is defined; `label_parents` (aarch64) at view format 16.
fn written_for(text: &str, defines: &[(&str, &str)]) -> Option<&'static str> {
    let defined = |name: &str| defines.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string()).or_else(|| {
        let at = text.find(&format!("#define {name} "))? + name.len() + 9;
        text[at..].split_whitespace().next().map(str::to_owned)
    });
    let has = |kernel: &str| text.contains(&format!(", {kernel},"));
    if defined("JOIN_PROGRAM").is_some() && has("join_phase") {
        return Some("join_phase");
    }
    #[cfg(target_arch = "aarch64")]
    if defined("UKL_CONSTANT_0").as_deref() == Some("16") && has("label_parents") {
        return Some("label_parents");
    }
    None
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
    /// A program that compiles a kernel with Rust here, but is not the source its Rust was written from, is refused:
    /// no other build runs in the Rust's place.
    #[test]
    fn a_changed_source_of_a_kernel_with_rust_is_refused() {
        let text = "#define JOIN_PROGRAM 1\nDEFINE_KERNEL2(join, join_phase, ...)\n";
        let e = super::kernels(text, &[]).unwrap_err();
        assert!(e.contains("join_phase") && e.contains("not the source"), "{e}");
        // The same source without `JOIN_PROGRAM` compiles none of those kernels.
        assert!(super::kernels("DEFINE_KERNEL2(join, join_phase, ...)\n", &[]).unwrap().is_empty());
    }

    #[test]
    fn another_program_has_no_rust() {
        assert!(super::kernels("kernel text", &[("UKL_CONSTANT_0", "16")]).unwrap().is_empty());
    }
}
