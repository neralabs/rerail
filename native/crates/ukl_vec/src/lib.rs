//! UKL on the CPU as vector code between sync points (decision 0567).
//!
//! A kernel's way here is passes from tape to tape, each one reading its input in place and writing its output once:
//!
//! 1. **The source** (`source`): the kernel preprocessed as the OpenCL backend's is, OpenCL C with the device's
//!    32-lane sub-groups (`UKL_OPENCL_SUBGROUPS`). One C dialect for the GPU drivers and this path: the library's
//!    intrinsics are OpenCL's builtins (`get_local_id`, `barrier`, `sub_group_*`, `atomic_*`), which the lowering
//!    knows by name. OpenCL's address spaces are written as attributes (`__global` as `__attribute__((ukl_global))`),
//!    which C's grammar reads where a qualifier goes.
//! 2. **The C tape** (`c_tape`): tree-sitter-c's, by `rerail-c-lean`.

use rerail_c_lean::{Dialect, Tape};

pub mod ctape;
pub mod k;
pub mod lower;
pub mod ops;
pub mod ty;

/// A failure on the way: the preprocessor's errors, or the parser's refusal, with where.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// OpenCL C's words that C's grammar has no place for, as what it has. A pointer here has no address space: the CPU's
/// memory is one, so `__global`, `__local`, `__constant` and `__private` are nothing. A threadgroup's own storage is
/// marked where it is declared (`__UKL_LOCAL_STORAGE`, the shim's `SHARED` rows), and a kernel's entry by an
/// attribute.
const SPELLINGS: &[(&str, &str)] = &[
    ("__global", ""),
    ("__local", ""),
    ("__constant", ""),
    ("__private", ""),
    ("__UKL_LOCAL_STORAGE", "__attribute__((ukl_local))"),
    ("__kernel", "__attribute__((ukl_kernel))"),
    // OpenCL's scalar names, which C's grammar cannot tell are types (`(uchar)x` is a cast only to one that knows):
    // its fixed-width types, which it reads as types.
    ("uchar", "uint8_t"),
    ("ushort", "uint16_t"),
    ("uint", "uint32_t"),
    ("ulong", "uint64_t"),
];

/// A kernel preprocessed for this path: the expanded text, its tokens and their origins (`rerail_c_pp::tokens`), and
/// the paths of the files the origins name.
pub struct Source {
    pub expanded: rerail_ukl::Expanded,
    pub paths: Vec<String>,
}

impl Source {
    /// The expanded text.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.expanded.text).unwrap_or("")
    }

    /// Where byte `at` of the expanded text came from: `path+offset`, or the expansion it is in.
    pub fn place(&self, at: u32) -> String {
        match self.expanded.origin(at) {
            Some((o, off)) if o.exp == 0 => format!("{}+{off}", self.paths.get(o.file as usize).map_or("?", String::as_str)),
            Some((o, _)) => format!("{}+{} (in an expansion)", self.paths.get(o.file as usize).map_or("?", String::as_str), o.off),
            None => format!("byte {at}"),
        }
    }
}

/// The kernel `text` at `path` preprocessed for this path: OpenCL C, its sub-groups 32 lanes, with `dirs` searched
/// for its own includes.
pub fn source(path: &str, text: &str, dirs: &[&str]) -> Result<Source, Error> {
    let mut p = rerail_ukl::Preprocess::new(rerail_ukl::Backend::OpenCl).file(path, text).define("UKL_OPENCL_SUBGROUPS", None).define("UKL_SIMD_WIDTH", Some("32u")).define("__UKL_VEC", None);
    for d in dirs {
        p = p.dir(*d);
    }
    for (word, spelled) in SPELLINGS {
        p = p.define(word, Some(spelled));
    }
    let (expanded, paths) = p.expanded(path).map_err(|e| Error(e.0.join("\n")))?;
    Ok(Source { expanded, paths })
}

/// The C tape of `source` (tree-sitter-c's, from its tokens), or where the parser refused it.
pub fn c_tape(source: &Source) -> Result<Tape, Error> {
    let x = &source.expanded;
    rerail_c_lean::tape_of_tokens(&x.text, &x.toks, Dialect::C).map_err(|r| {
        let text = source.text();
        let at = (rerail_c_lean::refused(r).start as usize).min(text.len());
        let (from, to) = (text.floor_char_boundary(at.saturating_sub(120)), text.ceil_char_boundary((at + 60).min(text.len())));
        Error(format!("the C parser refused the kernel ({r:?}) at {}: {:?} | {:?}", source.place(at as u32), &text[from..at], &text[at..to]))
    })
}

#[cfg(test)]
mod tests;
