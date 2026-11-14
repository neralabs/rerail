//! The JSON suite's helpers, shared with the engine's Metal measurements: a JSON step compiled
//! (`form`), the files a variable names (`files_in`), a file's short name (`base`).

use rerail_compiler::ladder::{Compiled, DIR, compile_step};
use std::path::{Path, PathBuf};

pub fn form(name: &str) -> Compiled {
    let source = std::fs::read_to_string(format!("{DIR}/{name}.bitlex")).expect("reads");
    compile_step(name, &source)
}

/// The files of the directory in `var`, sorted.
pub fn files_in(var: &str) -> Vec<PathBuf> {
    let dir = std::env::var(var).unwrap_or_else(|_| panic!("set {var}"));
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir).expect("a directory").map(|e| e.expect("an entry").path()).filter(|p| p.is_file()).collect();
    out.sort();
    out
}

pub fn base(p: &Path) -> String {
    p.file_name().expect("a name").to_string_lossy().into_owned()
}
