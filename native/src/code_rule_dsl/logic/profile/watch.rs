//! A dump of a run that has not ended (decision 0084): the profile so far, what every thread has open, and every
//! thread's native stack, written before the process is stopped, so a hung or slow run says where it was.
//!
//! `watch()` (the `rerail-engine` binary calls it first) blocks SIGTERM, SIGINT, SIGHUP and SIGUSR1 in every thread
//! and takes them on one thread of its own:
//!
//! - **SIGTERM, SIGINT, SIGHUP** (what `timeout` sends): dump into `<dir>/interrupted`, then exit with 128 + the signal.
//! - **SIGUSR1:** dump into `<dir>/snapshot-<n>` and go on.
//! - **`RERAIL_DEADLINE=<seconds>`:** at that wall time after start, a snapshot (`<dir>/deadline`), and go on: for a
//!   run stopped by SIGKILL, which cannot be caught.
//!
//! - **The memory ceiling's backstop** (decision 0462): each run watches its own ceiling and stops itself, naming the
//!   relations it held (`native::width`). A path that does not stop when told (a SQL stratum, the device) is stopped
//!   here: the process's resident memory is read every 50 ms, and once it has stayed past `RERAIL_MEMORY_MAX_MB`
//!   (`memory_ceiling`) for `GRACE`, the run is dumped as at a deadline and the process exits with code 3. A runaway
//!   cannot take the machine, on any path.
//!
//! `<dir>` is the profile's directory (`RERAIL_PROFILE`), else `RERAIL_DUMP`. With neither, a signal stops the run as
//! before, saying so. The dump is the profile report of everything recorded so far (spans that closed, SQL
//! statements that ended, plans, the explain section, memory), with the open spans first (`open_spans`), plus
//! `stacks.txt`: every thread's native stack, from `sample` on macOS and `eu-stack` or `gdb` elsewhere.

#![cfg_attr(target_family = "wasm", allow(dead_code))]

use std::path::{Path, PathBuf};

fn destination() -> Option<PathBuf> {
    super::directory().or_else(|| std::env::var_os("RERAIL_DUMP").map(PathBuf::from))
}

/// Writes a dump into `dir`: the report with the open spans first, and the stacks. `why`: what asked for it.
pub(in crate::code_rule_dsl) fn dump(dir: &Path, why: &str, started: std::time::Instant) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // The stacks first: the run goes on meanwhile, and they are what the open spans are read against.
    let stacks = stacks(&dir.join("stacks.txt"));
    let mut header = format!(
        "Dump of a run that has not ended ({why}, {:.1} s wall after start; pid {}).\nSpans still open are listed first and are not yet in the totals; SQL statements are in the totals once they end. Native stacks: {stacks}.\n\n",
        started.elapsed().as_secs_f64(),
        std::process::id()
    );
    header.push_str(&super::open_spans());
    header.push('\n');
    super::report::write(dir, &header)
}

/// Every thread's native stack into `out`; says how it was made.
fn stacks(out: &Path) -> String {
    let pid = std::process::id().to_string();
    let tries: Vec<(&str, Vec<String>)> = if cfg!(target_os = "macos") {
        // One second of samples, every thread, symbolized; `-mayDie` keeps what it has if the process exits.
        vec![("/usr/bin/sample", vec![pid.clone(), "1".into(), "-mayDie".into(), "-file".into(), out.display().to_string()])]
    } else {
        vec![
            ("eu-stack", vec!["-p".into(), pid.clone()]),
            ("gdb", vec!["-batch".into(), "-p".into(), pid.clone(), "-ex".into(), "thread apply all bt".into()]),
        ]
    };
    for (tool, args) in tries {
        let child = std::process::Command::new(tool).args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn();
        let Ok(mut child) = child else { continue };
        // Bounded: a tool that does not finish in 30 s is stopped, and the dump goes on without it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return format!("{tool} did not finish in 30 s");
                }
            }
        }
        let mut text = String::new();
        if let Some(mut o) = child.stdout.take() {
            let _ = std::io::Read::read_to_string(&mut o, &mut text);
        }
        if !text.is_empty() {
            let _ = std::fs::write(out, &text);
        }
        if out.exists() {
            return format!("stacks.txt ({tool})");
        }
    }
    "none (no stack tool found)".to_owned()
}

/// Takes the stop signals on a thread of its own, and the deadline; see the module. Call it before any other
/// thread starts, so they inherit the blocked signals. Off Unix there are no signals: the memory ceiling and the
/// deadline alone.
pub fn watch() {
    // The process's `main`: the profile's account of its life starts here (`life`).
    super::life::entered();
    #[cfg(not(target_family = "wasm"))]
    {
        let started = std::time::Instant::now();
        // The stop signals: Unix's alone (Windows has no signals to take).
        #[cfg(unix)]
        {
            let signals = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGUSR1];
            // SAFETY: a set of the signals above, blocked in this thread, and so in every thread it starts.
            let set = unsafe {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                for s in signals {
                    libc::sigaddset(&mut set, s);
                }
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
                set
            };
            let _ = rerail_accel::exec::service("rerail-watch", None, move || {
                // The snapshots this watcher has dumped: its thread's alone.
                let mut snapshots = 0u32;
                loop {
                    let mut sig = 0;
                    // SAFETY: waits for one of the blocked signals in `set`.
                    if unsafe { libc::sigwait(&set, &mut sig) } != 0 {
                        return;
                    }
                    let stop = sig != libc::SIGUSR1;
                    match destination() {
                        Some(dir) => {
                            let dir = if stop { dir.join("interrupted") } else { {
                                snapshots += 1;
                                dir.join(format!("snapshot-{snapshots}"))
                            } };
                            match dump(&dir, &format!("signal {sig}"), started) {
                                Ok(()) => eprintln!("rerail-engine: signal {sig}: dumped the run into {}", dir.display()),
                                Err(e) => eprintln!("rerail-engine: signal {sig}: the dump failed: {e}"),
                            }
                        }
                        None if stop => eprintln!("rerail-engine: signal {sig}: stopped (set RERAIL_PROFILE or RERAIL_DUMP to dump the run first)"),
                        None => eprintln!("rerail-engine: signal {sig}: nothing to dump into (set RERAIL_PROFILE or RERAIL_DUMP)"),
                    }
                    if stop {
                        std::process::exit(128 + sig);
                    }
                }
            });
        }
        if let Some(max) = memory_ceiling() {
            let _ = rerail_accel::exec::service("rerail-memory", None, move || {
                // The run stops itself first, naming its relations: this stops what did not.
                const GRACE: std::time::Duration = std::time::Duration::from_secs(2);
                let mut past: Option<std::time::Instant> = None;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let Some(now) = resident_bytes() else { return };
                    if now <= max {
                        past = None;
                        continue;
                    }
                    if past.get_or_insert_with(std::time::Instant::now).elapsed() < GRACE {
                        continue;
                    }
                    let why = format!("RERAIL_MEMORY_MAX_MB={}: {} MB resident", max >> 20, now >> 20);
                    if let Some(dir) = destination() {
                        let _ = dump(&dir.join("memory"), &why, started);
                    }
                    eprintln!(
                        "rerail-engine: stopped at the memory ceiling ({why}), {} s after the run was told to stop: the path \
                         that held the memory does not stop when told (a SQL stratum, the device, a cache).",
                        GRACE.as_secs()
                    );
                    std::process::exit(3);
                }
            });
        }
        if let Some(seconds) = std::env::var("RERAIL_DEADLINE").ok().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0) {
            let _ = rerail_accel::exec::service("rerail-deadline", None, move || {
                std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
                match destination() {
                    Some(dir) => match dump(&dir.join("deadline"), &format!("RERAIL_DEADLINE={seconds}"), started) {
                        Ok(()) => eprintln!("rerail-engine: deadline {seconds} s: dumped the run into {}", dir.join("deadline").display()),
                        Err(e) => eprintln!("rerail-engine: deadline {seconds} s: the dump failed: {e}"),
                    },
                    None => eprintln!("rerail-engine: deadline {seconds} s: nothing to dump into (set RERAIL_PROFILE or RERAIL_DUMP)"),
                }
            });
        }
    }
}

/// The memory ceiling in bytes: `RERAIL_MEMORY_MAX_MB`, else three quarters of the machine's memory; `None` with
/// `off` (or where neither the machine's memory nor the process's can be read).
pub fn memory_ceiling() -> Option<u64> {
    match std::env::var("RERAIL_MEMORY_MAX_MB").ok().as_deref().map(str::trim) {
        Some("off") | Some("0") => None,
        Some(mb) if mb.parse::<u64>().is_ok() => Some(mb.parse::<u64>().unwrap() << 20),
        _ => physical_bytes().map(|b| b / 4 * 3),
    }
    .filter(|_| resident_bytes().is_some())
}

/// The machine's memory, in bytes.
pub fn physical_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut size: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: `hw.memsize` is a u64; `size` and `len` describe its buffer.
        let code = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut size as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0) };
        (code == 0 && size > 0).then_some(size)
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_family = "wasm")))]
    {
        // SAFETY: sysconf has no pointer arguments.
        let (pages, page) = unsafe { (libc::sysconf(libc::_SC_PHYS_PAGES), libc::sysconf(libc::_SC_PAGESIZE)) };
        (pages > 0 && page > 0).then(|| pages as u64 * page as u64)
    }
    #[cfg(windows)]
    {
        rerail_prof::windows_physical_bytes()
    }
    #[cfg(not(any(all(unix, not(target_family = "wasm")), windows)))]
    {
        None
    }
}

/// The process's resident memory, in bytes (`None` where it cannot be read).
pub fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        unsafe extern "C" {
            // The task's own port (`mach_task_self()` is a macro over it; libc's function is deprecated).
            static mach_task_self_: libc::mach_port_t;
        }
        // SAFETY: task_info writes this C-layout buffer; count describes its capacity in words.
        let code = unsafe { libc::task_info(mach_task_self_, libc::MACH_TASK_BASIC_INFO, info.as_mut_ptr().cast(), &mut count) };
        // SAFETY: a successful task_info call initializes the output structure.
        (code == libc::KERN_SUCCESS).then(|| unsafe { info.assume_init() }.resident_size)
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages = status.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        // SAFETY: sysconf has no pointer arguments.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        (page_size > 0).then(|| pages.saturating_mul(page_size as u64))
    }
    #[cfg(windows)]
    {
        rerail_prof::windows_working_set()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}
