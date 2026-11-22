//! UKL on the CPU as vector code between sync points (decision 0567).
//!
//! A kernel's way here is passes from tape to tape, each one reading its input in place and writing its output once:
//!
//! 1. **The source** (`source`): the kernel preprocessed for the `Vec` backend (`rerail_ukl::Backend::Vec`): plain C
//!    whose every operation of the machine is one intrinsic, `__ukl_*` (`ukl_vec_shim.h`, `ukl/backend.h`'s `UKL_VEC`
//!    section: `__ukl_local_id(d)`, `__ukl_barrier()`, `__ukl_sg_shuffle(v, l)`, `__ukl_atomic_add(p, v)`, ...),
//!    which the lowering knows by name. Pointers have no address space, types are C's fixed-width ones, a
//!    threadgroup's storage and a kernel's entry are attributes (`ukl_local`, `ukl_kernel`).
//! 2. **The C tape** (`c_tape`): tree-sitter-c's, by `rerail-c-lean`.

use rerail_c_lean::{Dialect, Tape};

pub mod cl;
pub mod code;
pub mod jit;
pub mod ctape;
pub mod exec;
pub mod fiber;
pub mod k;
pub mod lower;
pub mod ops;
pub mod pressure;
pub mod run;
pub mod ssa;
pub mod ty;
pub mod vector;

/// A failure on the way: the preprocessor's errors, or the parser's refusal, with where.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// A kernel preprocessed for this path: the expanded text, its tokens and their origins (`rerail_c_pp::tokens`), and
/// the paths of the files the origins name.
pub struct Source {
    pub expanded: rerail_ukl::Expanded,
    pub paths: Vec<String>,
    /// Each file's lines' starts (empty if its text is not at hand).
    pub lines: Vec<Vec<u32>>,
}

impl Source {
    /// The expanded text.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.expanded.text).unwrap_or("")
    }

    /// Where byte `at` of the expanded text came from: `path:line:column`, or the expansion it is in.
    pub fn place(&self, at: u32) -> String {
        match self.expanded.origin(at) {
            Some((o, off)) if o.exp == 0 => self.at(o.file, off),
            Some((o, _)) => format!("{} (in an expansion)", self.at(o.file, o.off)),
            None => format!("byte {at}"),
        }
    }

    /// Byte `off` of file `file`, as `path:line:column`.
    fn at(&self, file: u32, off: u32) -> String {
        let path = self.paths.get(file as usize).map_or("?", String::as_str);
        match self.lines.get(file as usize).filter(|l| !l.is_empty()) {
            Some(l) => {
                let line = l.partition_point(|&s| s <= off);
                format!("{path}:{line}:{}", off - l[line - 1] + 1)
            }
            None => format!("{path}+{off}"),
        }
    }
}

/// The preprocessing of this path: the `Vec` backend's, its SIMD groups 32 lanes.
pub fn preprocess() -> rerail_ukl::Preprocess {
    rerail_ukl::Preprocess::new(rerail_ukl::Backend::Vec).define("UKL_SIMD_WIDTH", Some("32u"))
        // A scan's tile is 16 values a thread, as the CPU backend's (decision 0527 §3): a group's operation is many
        // of the vector tape's nodes, a thread's fold of a value a few.
        .define("UKL_SCAN_VT", Some("16u"))
}

/// The kernel `text` at `path` preprocessed for this path (`preprocess`), with `dirs` searched for its own includes.
pub fn source(path: &str, text: &str, dirs: &[&str]) -> Result<Source, Error> {
    let mut p = preprocess().file(path, text);
    for d in dirs {
        p = p.dir(*d);
    }
    source_of(&p, path)
}

/// `main` preprocessed by `p` (a `preprocess` with its files and defines).
pub fn source_of(p: &rerail_ukl::Preprocess, main: &str) -> Result<Source, Error> {
    let (expanded, paths) = p.expanded(main).map_err(|e| Error(e.0.join("\n")))?;
    let lines = paths
        .iter()
        .map(|path| p.text_of(path).map_or_else(Vec::new, |t| std::iter::once(0).chain(t.iter().enumerate().filter(|&(_, &c)| c == b'\n').map(|(i, _)| i as u32 + 1)).collect()))
        .collect();
    Ok(Source { expanded, paths, lines })
}

/// The vectorizer's build: a hash of the code that makes a kernel's code tape (`build.rs`). A store that keeps code
/// tapes keys them by it.
pub const BUILD: &str = env!("UKL_VEC_BUILD");

/// The program of `main` preprocessed by `p`, and its preprocessed text (what a store of its kernels keys them by).
pub fn program_and_text(p: &rerail_ukl::Preprocess, main: &str) -> Result<(lower::Program, String), Error> {
    let s = source_of(p, main)?;
    let tape = c_tape(&s)?;
    Ok((lower::lower(&s, &tape)?, s.text().to_string()))
}

/// The program of `main` preprocessed by `p`: its source, C tape and kernel tape.
pub fn program(p: &rerail_ukl::Preprocess, main: &str) -> Result<lower::Program, Error> {
    let s = source_of(p, main)?;
    let tape = c_tape(&s)?;
    lower::lower(&s, &tape)
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
