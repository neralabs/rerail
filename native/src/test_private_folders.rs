//! The engine's test binaries make their folders the user's alone (decision 0574): a umask of `077`, set before
//! `main`, as the watchdog starts (`test_watchdog`). A fixture's folder (`tempfile::tempdir`, `create_dir_all`) is
//! made with the umask's mode, and a host's `002` made them `775`, which the mapped files' rules refuse. The rules'
//! own refusals are tested by setting the modes they refuse (`mapped::tests`, `tools::launch_tests`).

#[cfg(unix)]
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static START: extern "C" fn() = start;

#[cfg(unix)]
extern "C" fn start() {
    // SAFETY: `umask` sets the process's file mode mask; it cannot fail and touches no memory.
    unsafe { libc::umask(0o077) };
}
