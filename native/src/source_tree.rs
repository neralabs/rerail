//! Where this engine reads its own `.rerail` files as it runs (the builtin typing programs under
//! `native/src/code_rule_dsl/typing/programs`, the rule library under `docs/rule-syntax/lib`): its checkout, or the
//! installed package that ships those files at the same paths.
//!
//! It is found as the program runs, not written into the build (decision 0224). A crate that wrote
//! its checkout's path in (`env!("CARGO_MANIFEST_DIR")`) was a different crate in every checkout, and
//! so was every crate above it: a compiler cache shared by the checkouts could reuse none of them.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The rule library, under the root.
const LIBRARY: &str = "docs/rule-syntax/lib";
/// The engine's builtin typing programs, under the root.
const PROGRAMS: &str = "native/src/code_rule_dsl/typing/programs";

/// The root the engine's `.rerail` files are read under: a checkout (the directory that holds `native/Cargo.toml`
/// and `docs/rule-syntax`), or an installed package (the directory that holds the library's `core.rerail` and the
/// typing programs' `sources.rerail`, as `@neralabs/rerail` ships them). The first found of:
///
/// - `RERAIL_SOURCE_ROOT`;
/// - a checkout or an installed package above the file this code was loaded from: the program, a test binary, or
///   the Node addon, built into its checkout's `target/` or installed in its package's `native/`;
/// - a checkout above the package `cargo run` or `cargo test` runs (`CARGO_MANIFEST_DIR`, as the program runs);
/// - a checkout above the working directory.
///
/// `None` when none is found.
///
/// A search each call (the loader's lookup, a canonical path, a walk up the directories: 1.9 M instructions): a
/// caller that makes many paths finds the directories once and hands them on (`phase0::Dirs`), not this.
pub fn root() -> Option<PathBuf> {
    find(std::env::var_os("RERAIL_SOURCE_ROOT"), image(), [std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from), std::env::current_dir().ok()])
}

/// The root's `native/` directory: `native` (as the working directory has it) when no root is found, so that what
/// is read from it is not found, as before.
pub fn native() -> PathBuf {
    root().map_or_else(|| PathBuf::from("native"), |root| root.join("native"))
}

/// `root()` from what it reads: the override, the loaded image, and the other places a checkout is looked for above.
fn find(source_root: Option<OsString>, image: Option<PathBuf>, others: [Option<PathBuf>; 2]) -> Option<PathBuf> {
    if let Some(root) = source_root {
        return Some(PathBuf::from(root));
    }
    let from_image = image.and_then(|image| above(&image, is_root_or_installed));
    from_image.or_else(|| others.into_iter().flatten().find_map(|start| above(&start, is_root)))
}

/// The nearest directory at or above `start` (canonical) that passes `test`.
fn above(start: &Path, test: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let start = dunce::canonicalize(start).unwrap_or_else(|_| start.to_owned());
    start.ancestors().find(|dir| test(dir)).map(Path::to_owned)
}

fn is_root(dir: &Path) -> bool {
    dir.join("native/Cargo.toml").is_file() && dir.join("docs/rule-syntax").is_dir()
}

/// A checkout's root (`is_root`) or an installed package's (`is_installed`). `docs/rule-syntax` is asked first, as
/// both hold it: a directory that is neither costs one look, as it does `is_root`.
fn is_root_or_installed(dir: &Path) -> bool {
    dir.join("docs/rule-syntax").is_dir() && (dir.join("native/Cargo.toml").is_file() || is_installed(dir))
}

/// An installed package's root: the library and the typing programs, at the paths the package ships them.
fn is_installed(dir: &Path) -> bool {
    dir.join(LIBRARY).join("core.rerail").is_file() && dir.join(PROGRAMS).join("sources.rerail").is_file()
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

#[cfg(windows)]
fn loaded_from(address: usize) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW};
    let mut module = std::ptr::null_mut();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    // SAFETY: with `FROM_ADDRESS` the name argument is only an address to look up, and `module` is written on success;
    // `UNCHANGED_REFCOUNT` takes no reference, so there is nothing to free.
    if unsafe { GetModuleHandleExW(flags, address as *const u16, &mut module) } == 0 {
        return None;
    }
    // The longest path Windows names (32,767 characters and the NUL).
    let mut name = vec![0u16; 32_768];
    // SAFETY: `name` holds `name.len()` characters, the size handed over; `module` is a module the loader holds.
    let length = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
    (length > 0 && length < name.len()).then(|| PathBuf::from(std::ffi::OsString::from_wide(&name[..length])))
}

#[cfg(not(any(unix, windows)))]
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
        assert_eq!(dunce::canonicalize(&found).unwrap(), dunce::canonicalize(&exe).unwrap());
        assert!(took < std::time::Duration::from_millis(5), "{took:?}");
        assert_eq!(super::loaded_from(0), None, "an address no image holds");
    }

    /// A directory laid out as a checkout, as an installed package (the addon in `native/`), or as neither.
    fn layout(dir: &std::path::Path, files: &[&str]) {
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
    }
    const CHECKOUT: &[&str] = &["native/Cargo.toml", "docs/rule-syntax/lib/core.rerail", "native/src/code_rule_dsl/typing/programs/sources.rerail", "target/debug/addon.node"];
    const INSTALLED: &[&str] = &["package.json", "docs/rule-syntax/lib/core.rerail", "native/src/code_rule_dsl/typing/programs/sources.rerail", "native/addon.node"];

    fn canonical(path: std::path::PathBuf) -> std::path::PathBuf {
        dunce::canonicalize(path).unwrap()
    }

    #[test]
    fn an_installed_package_is_the_root_above_its_addon_and_nowhere_else() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path(), INSTALLED);
        let addon = dir.path().join("native/addon.node");
        assert_eq!(super::find(None, Some(addon.clone()), [None, None]), Some(canonical(dir.path().to_owned())));
        // Not from the working directory or the package cargo runs: only the engine's own files are its library.
        assert_eq!(super::find(None, None, [Some(dir.path().to_owned()), Some(dir.path().join("native"))]), None);
        // A package missing either half of what it ships is not one.
        std::fs::remove_file(dir.path().join("native/src/code_rule_dsl/typing/programs/sources.rerail")).unwrap();
        assert_eq!(super::find(None, Some(addon), [None, None]), None);
    }

    #[test]
    fn a_checkout_is_the_root_above_its_build_its_package_or_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path(), CHECKOUT);
        let root = Some(canonical(dir.path().to_owned()));
        assert_eq!(super::find(None, Some(dir.path().join("target/debug/addon.node")), [None, None]), root);
        assert_eq!(super::find(None, None, [Some(dir.path().join("native")), None]), root);
        assert_eq!(super::find(None, Some(std::path::PathBuf::from("/")), [None, Some(dir.path().join("docs"))]), root);
    }

    #[test]
    fn the_nearest_root_above_the_addon_is_taken() {
        // The package installed inside a checkout reads its own files, which match its build.
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path(), CHECKOUT);
        let package = dir.path().join("node_modules/@neralabs/rerail");
        layout(&package, INSTALLED);
        assert_eq!(super::find(None, Some(package.join("native/addon.node")), [Some(dir.path().to_owned()), None]), Some(canonical(package)));
    }

    #[test]
    fn neither_layout_is_no_root_and_the_override_is_taken_as_given() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path(), &["native/addon.node", "docs/rule-syntax/lib/core.rerail"]);
        let addon = dir.path().join("native/addon.node");
        assert_eq!(super::find(None, Some(addon.clone()), [Some(dir.path().to_owned()), None]), None);
        let named = std::ffi::OsString::from("/no/such/checkout");
        assert_eq!(super::find(Some(named), Some(addon), [Some(dir.path().to_owned()), None]), Some(std::path::PathBuf::from("/no/such/checkout")));
    }
}
