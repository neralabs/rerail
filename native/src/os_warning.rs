//! A warning, once per process, when it runs on a macOS whose allocator is known to abort the engine's work.
//!
//! On macOS 26.2 (M1 Max, 2026-11-23) the tests-tape binary aborts in `free` ("pointer being freed was not
//! allocated") in tree-sitter's parse and walk, with no engine code in the failing loop. The abort follows the
//! allocations the binary makes at load; the same loop alone never aborts. Guard Malloc, ASan, stack logging and an
//! injected library each hide it. The same binary passes on 26.4.1 and on 27.2.1. So a run on 26.0 to 26.3 is told
//! that an abort there is the system's, and that updating macOS is the fix.
//!
//! Shared by the CLI (`bin/rerail-engine.rs`) and every test binary (`test_watchdog.rs`, by path).

/// The first macOS release seen to pass, and the first seen to abort.
const FIXED: (u32, u32) = (26, 4);
const BROKEN_FROM: (u32, u32) = (26, 0);

/// The running macOS's version as (major, minor), from `kern.osproductversion`; `None` elsewhere.
#[cfg(target_os = "macos")]
fn macos_version() -> Option<(u32, u32)> {
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    // SAFETY: the name is NUL-terminated; the kernel writes at most `len` bytes into `buf` and sets `len`.
    let got = unsafe { libc::sysctlbyname(c"kern.osproductversion".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    if got != 0 {
        return None;
    }
    let text = std::str::from_utf8(&buf[..len.min(buf.len())]).ok()?.trim_end_matches('\0');
    let mut parts = text.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0)))
}

#[cfg(not(target_os = "macos"))]
fn macos_version() -> Option<(u32, u32)> {
    None
}

/// The warning for `version`, where it is one of the releases whose allocator aborts.
fn message(version: (u32, u32)) -> Option<String> {
    (version >= BROKEN_FROM && version < FIXED).then(|| {
        format!(
            "warning: macOS {}.{}: this release's allocator is known to abort the engine's tree-sitter work in `free` \
             (\"pointer being freed was not allocated\"; seen on 26.2, not on 26.4.1 or 27.2.1). An abort like that \
             here is the system's: update macOS to {}.{} or later.\n",
            version.0, version.1, FIXED.0, FIXED.1
        )
    })
}

/// Writes the warning to stderr, once per process, on an affected macOS.
pub fn warn() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(text) = macos_version().and_then(message) {
            // Straight to the descriptor: this runs before `main` in a test binary, and takes no lock of std's.
            #[cfg(unix)]
            // SAFETY: a write of a live buffer of its own length.
            unsafe {
                libc::write(2, text.as_ptr().cast(), text.len());
            }
            #[cfg(not(unix))]
            eprint!("{text}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::message;

    #[test]
    fn warns_on_the_releases_whose_allocator_aborts() {
        assert!(message((26, 2)).is_some_and(|m| m.contains("macOS 26.2")));
        assert!(message((26, 0)).is_some());
        assert!(message((26, 3)).is_some());
        assert!(message((26, 4)).is_none());
        assert!(message((27, 2)).is_none());
        assert!(message((15, 7)).is_none());
    }
}
