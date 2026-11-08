//! A dump of a run that has not ended (decision 0082): the profile so far, what every thread has open, and every
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
    super::write(dir, &header)
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
/// thread starts, so they inherit the blocked signals. A no-op off Unix.
pub fn watch() {
    #[cfg(all(unix, not(target_family = "wasm")))]
    {
        let started = std::time::Instant::now();
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
        use std::sync::atomic::{AtomicU32, Ordering};
        static SNAPSHOTS: AtomicU32 = AtomicU32::new(0);
        let _ = std::thread::Builder::new().name("rerail-watch".into()).spawn(move || {
            loop {
                let mut sig = 0;
                // SAFETY: waits for one of the blocked signals in `set`.
                if unsafe { libc::sigwait(&set, &mut sig) } != 0 {
                    return;
                }
                let stop = sig != libc::SIGUSR1;
                match destination() {
                    Some(dir) => {
                        let dir = if stop { dir.join("interrupted") } else { dir.join(format!("snapshot-{}", SNAPSHOTS.fetch_add(1, Ordering::Relaxed) + 1)) };
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
        if let Some(seconds) = std::env::var("RERAIL_DEADLINE").ok().and_then(|s| s.parse::<f64>().ok()).filter(|s| *s > 0.0) {
            let _ = std::thread::Builder::new().name("rerail-deadline".into()).spawn(move || {
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
