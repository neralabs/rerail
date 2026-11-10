//! A time limit for each test of the engine's test binary, enforced from inside it.
//!
//! libtest runs each test on its own thread, named after the test (`code_rule_dsl::…::name`). A watchdog thread,
//! started before `main`, lists the process's threads once a second and times each test thread from when it first
//! sees it. A test past its limit is named on stderr and the process exits with status 124, as `timeout` does: a
//! hung test (a deadlock, a run that never ends) fails in minutes, not hours.
//!
//! - The limit is `DEFAULT` (10 minutes) unless `RERAIL_TEST_TIMEOUT=<seconds>` sets another; `0` turns it off.
//! - A test that is known to run longer asks for more with `allow`, as its first line. The environment variable,
//!   when set, still caps it.
//!
//! A test waiting for a free test thread has no thread yet, so its wait is not counted.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A test's limit when it asks for none.
const DEFAULT: Duration = Duration::from_secs(600);

/// The limits tests asked for (`allow`), by thread name as the watchdog reads it (`MAX_NAME` bytes at most).
static ALLOWED: Mutex<Option<HashMap<String, Duration>>> = Mutex::new(None);

/// The longest thread name the system reports (macOS: 64 bytes with its NUL; Linux: 16).
#[cfg(target_os = "macos")]
const MAX_NAME: usize = 63;
#[cfg(not(target_os = "macos"))]
const MAX_NAME: usize = 15;

/// Lets the calling test run for up to `limit` (capped by `RERAIL_TEST_TIMEOUT` when it is set).
pub(crate) fn allow(limit: Duration) {
    let Some(name) = std::thread::current().name().map(short) else { return };
    ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(name, limit);
}

fn short(name: &str) -> String {
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// The limit from the environment: `None` when unset, `Some(None)` when off (`0`).
fn configured() -> Option<Option<Duration>> {
    let v = std::env::var("RERAIL_TEST_TIMEOUT").ok()?;
    let secs: u64 = v.trim().parse().ok()?;
    Some((secs > 0).then(|| Duration::from_secs(secs)))
}

// Started before `main`, so every run of the test binary has it, whatever the filter.
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
static START: extern "C" fn() = start;

extern "C" fn start() {
    // `--list` and the like start no test; a watchdog then is harmless, but `Some(None)` turns it off.
    if matches!(configured(), Some(None)) {
        return;
    }
    let _ = std::thread::Builder::new().name("test watchdog".into()).spawn(watch);
}

fn watch() {
    let env = configured().flatten();
    let mut seen: HashMap<(u64, String), Instant> = HashMap::new();
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let now = Instant::now();
        let threads = threads();
        seen.retain(|k, _| threads.contains(k));
        for k in threads {
            // A test's thread is named by its path; the engine's own threads (`rerail-cpu 3`) are not.
            if !k.1.contains("::") {
                continue;
            }
            let since = *seen.entry(k.clone()).or_insert(now);
            let asked = ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&k.1).copied());
            let limit = match (asked, env) {
                (Some(a), Some(e)) => a.min(e),
                (Some(a), None) => a,
                (None, Some(e)) => e,
                (None, None) => DEFAULT,
            };
            let ran = now - since;
            if ran > limit {
                fail(&k.1, ran, limit);
            }
        }
    }
}

/// Names the test and exits at once: `exit` would run atexit handlers, which may wait on a lock a hung thread holds.
fn fail(name: &str, ran: Duration, limit: Duration) -> ! {
    let msg = format!(
        "\ntest watchdog: `{name}` ran {} s, past its limit of {} s (RERAIL_TEST_TIMEOUT, or the test's `test_watchdog::allow`); exiting with status 124\n",
        ran.as_secs(),
        limit.as_secs()
    );
    unsafe {
        libc::write(2, msg.as_ptr().cast(), msg.len());
        libc::_exit(124)
    }
}

/// The process's threads, as (system thread id, name).
#[cfg(target_os = "macos")]
fn threads() -> Vec<(u64, String)> {
    const PROC_PIDLISTTHREADS: libc::c_int = 6;
    let pid = std::process::id() as libc::c_int;
    let mut ids = vec![0u64; 512];
    let n = loop {
        let bytes = (ids.len() * 8) as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid, PROC_PIDLISTTHREADS, 0, ids.as_mut_ptr().cast(), bytes) };
        if got <= 0 {
            return Vec::new();
        }
        if got < bytes {
            break got as usize / 8;
        }
        ids.resize(ids.len() * 2, 0);
    };
    ids[..n]
        .iter()
        .filter_map(|&id| {
            let mut info: libc::proc_threadinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::proc_threadinfo>() as libc::c_int;
            let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTHREADINFO, id, (&raw mut info).cast(), size) };
            (got == size).then(|| (id, unsafe { std::ffi::CStr::from_ptr(info.pth_name.as_ptr()) }.to_string_lossy().into_owned()))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn threads() -> Vec<(u64, String)> {
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let id: u64 = e.file_name().to_str()?.parse().ok()?;
            let name = std::fs::read_to_string(e.path().join("comm")).ok()?;
            Some((id, name.trim_end().to_owned()))
        })
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn threads() -> Vec<(u64, String)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The watchdog sees this test's own thread, by its name.
    #[test]
    fn the_watchdog_sees_the_running_test() {
        let me = short(std::thread::current().name().unwrap());
        assert!(threads().iter().any(|(_, n)| *n == me), "{me} not among {:?}", threads());
    }

    /// A test past its limit ends the process with status 124 (run as a child: the binary, this test alone).
    #[test]
    fn a_test_past_its_limit_ends_the_run() {
        if std::env::var_os("RERAIL_WATCHDOG_CHILD").is_some() {
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let t0 = Instant::now();
        let out = std::process::Command::new(exe)
            .args(["test_watchdog::tests::a_test_past_its_limit_ends_the_run", "--exact", "--nocapture"])
            .env("RERAIL_WATCHDOG_CHILD", "1")
            .env("RERAIL_TEST_TIMEOUT", "2")
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(124), "{err}");
        assert!(err.contains("a_test_past_its_limit_ends_the_run` ran"), "{err}");
        assert!(t0.elapsed() < Duration::from_secs(20));
    }
}
