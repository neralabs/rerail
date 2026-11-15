//! The checkout this engine runs from: where it reads its own `.rr` files as it runs (the builtin
//! typing programs under `native/src/code_rule_dsl/typing/programs`, the rule library under
//! `docs/rule-syntax/lib`).
//!
//! It is found as the program runs, not written into the build (decision 0224). A crate that wrote
//! its checkout's path in (`env!("CARGO_MANIFEST_DIR")`) was a different crate in every checkout, and
//! so was every crate above it: a compiler cache shared by the checkouts could reuse none of them.
use std::path::{Path, PathBuf};

/// The checkout's root: the directory that holds `native/Cargo.toml` and `docs/rule-syntax`. The first
/// found of:
///
/// - `RERAIL_SOURCE_ROOT`;
/// - a directory above the file this code was loaded from: the program, a test binary, or the Node
///   addon, each built into its checkout's `target/`;
/// - one above the package `cargo run` or `cargo test` runs (`CARGO_MANIFEST_DIR`, as the program runs);
/// - one above the working directory.
///
/// `None` outside a checkout: an installed engine has no source tree.
pub fn root() -> Option<PathBuf> {
    find()
}

/// The checkout's `native/` directory: `native` (as the working directory has it) outside a checkout,
/// so that what is read from it is not found, as before outside one.
pub fn native() -> PathBuf {
    root().map_or_else(|| PathBuf::from("native"), |root| root.join("native"))
}

fn find() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("RERAIL_SOURCE_ROOT") {
        return Some(PathBuf::from(root));
    }
    let starts = [image(), std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from), std::env::current_dir().ok()];
    starts.into_iter().flatten().find_map(|start| {
        let start = start.canonicalize().unwrap_or(start);
        start.ancestors().find(|dir| is_root(dir)).map(Path::to_owned)
    })
}

fn is_root(dir: &Path) -> bool {
    dir.join("native/Cargo.toml").is_file() && dir.join("docs/rule-syntax").is_dir()
}

/// The file this code was loaded from (a program, or a library loaded into one).
#[cfg(unix)]
fn image() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: `dladdr` reads only the address, and fills `info` with pointers into the loader's own tables.
    if unsafe { libc::dladdr(image as *const libc::c_void, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: a non-null `dli_fname` is a NUL-terminated path the loader keeps for the image's lifetime.
    let name = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
}

#[cfg(not(unix))]
fn image() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_test_binary_finds_its_checkout() {
        let root = super::root().expect("the checkout");
        assert!(root.join("native/Cargo.toml").is_file());
        assert_eq!(super::native(), root.join("native"));
    }
}
