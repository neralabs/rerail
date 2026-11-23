//! The preprocessor tape (decision 0436) by its Bitlex grammar, `bitlex_pp/pp.bitlex` (C++'s; C's
//! leaves out its lines marked `C++ only`), compiled once a process to tier 1 and tier 2 on
//! Cranelift, the interpreter and Metal. Its reference is the lean writer,
//! `rerail_c_pp::tape::image`: a file the grammar reads gives the same bytes, and one it refuses
//! goes to the lean writer.

use crate::{Compiled, compile_kinds};
use rerail_c_lean::pp::Options;
use rerail_c_pp::tape::{FIELDS, KINDS};

/// The grammar, C++'s.
pub const PP: &str = include_str!("../bitlex_pp/pp.bitlex");

/// The grammar's text for C (`cpp` false: its `C++ only` lines left out) or C++ (its `C only`
/// lines left out).
pub fn grammar(cpp: bool) -> String {
    let drop = if cpp { "# C only" } else { "# C++ only" };
    PP.lines().filter(|l| !l.trim_end().ends_with(drop)).map(|l| format!("{l}\n")).collect()
}

/// The lexer's options the tape is read with: C23 or C++26, GNU (the lean writer's).
pub fn options(cpp: bool) -> Options {
    if cpp { Options::cpp(26, true) } else { Options::c(23, true) }
}

static C: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();
static CPP: std::sync::OnceLock<Result<Compiled, String>> = std::sync::OnceLock::new();

/// The process's compiled grammar for C or C++, compiled the first time.
pub fn compiled(cpp: bool) -> Result<&'static Compiled, &'static str> {
    let cell = if cpp { &CPP } else { &C };
    cell.get_or_init(|| compile_kinds(&grammar(cpp), KINDS, FIELDS)).as_ref().map_err(String::as_str)
}

/// `src`'s tape: the grammar's, or the lean writer's where the grammar refuses the file.
pub fn image(src: &[u8], cpp: bool) -> rerail_tape_format::Vector<u8> {
    compiled(cpp).ok().and_then(|c| c.image(src)).unwrap_or_else(|| rerail_c_pp::tape::image(src, options(cpp)))
}

#[cfg(test)]
#[path = "pp/tests.rs"]
mod tests;
