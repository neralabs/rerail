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

/// The file this code was loaded from (a program, or a library loaded into one): the process's to the end, so
/// found once.
fn image() -> Option<PathBuf> {
    static IMAGE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    IMAGE.get_or_init(|| loaded_from(image as *const u8 as usize)).clone()
}

/// The image whose segments hold `address`, by the loader's list of images. `dladdr` answers the same, but finds the
/// symbol nearest the address too: a search of the 415 MB debug engine's symbols, 84 ms of a run's 350 on its main
/// thread (decision 0285).
#[cfg(target_os = "macos")]
fn loaded_from(address: usize) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    const LC_SEGMENT_64: u32 = 0x19;
    #[repr(C)]
    struct Header {
        magic: u32,
        cputype: i32,
        cpusubtype: i32,
        filetype: u32,
        ncmds: u32,
        sizeofcmds: u32,
        flags: u32,
        reserved: u32,
    }
    #[repr(C)]
    struct Segment {
        cmd: u32,
        cmdsize: u32,
        segname: [u8; 16],
        vmaddr: u64,
        vmsize: u64,
    }
    // The loader's functions (`<mach-o/dyld.h>`), in libSystem: the `libc` crate's copies are deprecated for
    // another crate's.
    unsafe extern "C" {
        fn _dyld_image_count() -> u32;
        fn _dyld_get_image_header(index: u32) -> *const Header;
        fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
        fn _dyld_get_image_name(index: u32) -> *const std::ffi::c_char;
    }
    // SAFETY: the loader's own list: each index below the count names a loaded image, its header mapped, its load
    // commands following the header, each at least as long as the command it begins (`cmdsize`).
    unsafe {
        for i in 0.._dyld_image_count() {
            let header = _dyld_get_image_header(i);
            if header.is_null() || (*header).magic != 0xfeed_facf {
                continue;
            }
            let slide = _dyld_get_image_vmaddr_slide(i) as usize;
            let mut at = header.cast::<u8>().add(std::mem::size_of::<Header>());
            for _ in 0..(*header).ncmds {
                let segment = at.cast::<Segment>();
                if (*segment).cmd == LC_SEGMENT_64 {
                    let start = ((*segment).vmaddr as usize).wrapping_add(slide);
                    if (*segment).vmsize > 0 && (start..start + (*segment).vmsize as usize).contains(&address) {
                        let name = _dyld_get_image_name(i);
                        if name.is_null() {
                            return None;
                        }
                        let name = std::ffi::CStr::from_ptr(name);
                        return Some(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())));
                    }
                }
                at = at.add((*segment).cmdsize as usize);
            }
        }
    }
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
fn loaded_from(address: usize) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: `dladdr` reads only the address, and fills `info` with pointers into the loader's own tables.
    if unsafe { libc::dladdr(address as *const libc::c_void, &mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: a non-null `dli_fname` is a NUL-terminated path the loader keeps for the image's lifetime.
    let name = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
}

#[cfg(not(unix))]
fn loaded_from(_address: usize) -> Option<PathBuf> {
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

    /// The image is found by the loader's list of images, as `dladdr` names it, in a few milliseconds at most.
    #[test]
    fn the_image_is_the_one_the_loader_names() {
        let t = std::time::Instant::now();
        let found = super::loaded_from(super::image as *const u8 as usize).expect("the image");
        let took = t.elapsed();
        let exe = std::env::current_exe().unwrap();
        assert_eq!(found.canonicalize().unwrap(), exe.canonicalize().unwrap());
        assert!(took < std::time::Duration::from_millis(5), "{took:?}");
        assert_eq!(super::loaded_from(0), None, "an address no image holds");
    }
}
