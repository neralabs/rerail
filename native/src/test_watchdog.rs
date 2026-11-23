//! A time limit for each test of the engine's test binary, enforced from inside it.
//!
//! libtest runs each test on its own thread, named after the test (`code_rule_dsl::…::name`). A watchdog thread,
//! started before `main`, lists the process's threads once a second (an eighth of the limit, when that is less:
//! `tick`) and times each test thread from when it first sees it. A test past its limit is named on stderr and the process exits with status 124, as `timeout` does: a
//! hung test (a deadlock, a run that never ends) fails in minutes, not hours.
//!
//! - The limit is `DEFAULT` (10 minutes) unless `RERAIL_TEST_TIMEOUT=<seconds>` sets another (a fraction too: `0.5`);
//!   `0` turns it off.
//! - A test that is known to run longer asks for more with `allow`, as its first line. The environment variable,
//!   when set, still caps it.
//!
//! A test waiting for a free test thread has no thread yet, so its wait is not counted.
//!
//! A segmentation fault or bus error kills the process with no word of which test it was in. The watchdog's handler
//! of both writes the faulting thread's name and the tests running then, and hands the fault on to the handler it
//! replaced (Rust's, which reports a stack overflow, or the default one).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A test's limit when it asks for none.
const DEFAULT: Duration = Duration::from_secs(600);

/// The limits tests asked for (`allow`), by the thread's `Key`.
static ALLOWED: Mutex<Option<HashMap<Key, Duration>>> = Mutex::new(None);

/// How an allowance knows its test's thread. Linux cuts a thread's name to 15 bytes, `code_rule_dsl::` for nearly
/// every test, so a name there would grant one test's allowance to every test: the key is the thread's system id. On
/// macOS the name (63 bytes) is the test's path, and on Windows (a thread's description, uncut) all of it.
#[cfg(target_os = "linux")]
type Key = u64;
#[cfg(not(target_os = "linux"))]
type Key = String;

/// The calling thread's key.
#[cfg(target_os = "linux")]
fn mine() -> Option<Key> {
    // SAFETY: no arguments, no failure. The system call, not glibc's `gettid` (2.30): the Linux engine links against
    // 2.17.
    Some(unsafe { libc::syscall(libc::SYS_gettid) } as u64)
}
#[cfg(not(target_os = "linux"))]
fn mine() -> Option<Key> {
    std::thread::current().name().map(short)
}

/// A listed thread's key (`threads`: its system id and its name).
#[cfg(target_os = "linux")]
fn key(thread: &(u64, String)) -> &Key {
    &thread.0
}
#[cfg(not(target_os = "linux"))]
fn key(thread: &(u64, String)) -> &Key {
    &thread.1
}

/// The longest thread name the system reports (macOS: 64 bytes with its NUL; Linux: 16; Windows: a thread's
/// description, whole).
#[cfg(target_os = "macos")]
const MAX_NAME: usize = 63;
#[cfg(windows)]
const MAX_NAME: usize = usize::MAX;
#[cfg(not(any(target_os = "macos", windows)))]
const MAX_NAME: usize = 15;

/// Lets the calling test run for up to `limit` (capped by `RERAIL_TEST_TIMEOUT` when it is set).
pub(crate) fn allow(limit: Duration) {
    let Some(me) = mine() else { return };
    ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(me, limit);
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
    let secs: f64 = v.trim().parse().ok().filter(|s: &f64| s.is_finite() && *s >= 0.0)?;
    Some((secs > 0.0).then(|| Duration::from_secs_f64(secs)))
}

/// How often the watchdog lists the threads under limit `env` (the environment's): once a second, or an eighth of a
/// shorter limit (at least 50 ms), so a limit under a few seconds is kept to within an eighth of it. The watchdog's
/// own tests ran a child to a 2 s limit and a fault past the first 1 s listing, 5.8 s of the suite between them.
fn tick(env: Option<Duration>) -> Duration {
    env.map_or(Duration::from_secs(1), |l| (l / 8).clamp(Duration::from_millis(50), Duration::from_secs(1)))
}

// Started before `main`, so every run of the test binary has it, whatever the filter.
#[used]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(target_os = "linux", unsafe(link_section = ".init_array"))]
// The C runtime's initializers, which it calls before `main` (MSVC).
#[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
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
    // After `main` has started: Rust installs its own handlers there, which ours then hand on to.
    std::thread::sleep(Duration::from_millis(200));
    #[cfg(unix)]
    faults::install();
    let every = tick(env);
    loop {
        std::thread::sleep(every);
        let now = Instant::now();
        let threads = threads();
        // A thread's id may be reused once it ends: its allowance ends with it. (Only threads seen before: one that
        // started since this listing may already have asked.)
        let ended: Vec<Key> = seen.keys().filter(|k| !threads.contains(k)).map(|k| key(k).to_owned()).collect();
        seen.retain(|k, _| threads.contains(k));
        if let Some(allowed) = ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            for k in &ended {
                allowed.remove(k);
            }
        }
        #[cfg(unix)]
        faults::running(threads.iter().map(|(_, n)| n.as_str()).filter(|n| n.contains("::")));
        for k in threads {
            // A test's thread is named by its path; the engine's own threads (`rerail-cpu 3`) are not.
            if !k.1.contains("::") {
                continue;
            }
            let since = *seen.entry(k.clone()).or_insert(now);
            let asked = ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key(&k)).copied());
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
        "\ntest watchdog: `{name}` ran {:.1} s, past its limit of {:.1} s (RERAIL_TEST_TIMEOUT, or the test's `test_watchdog::allow`); exiting with status 124\n",
        ran.as_secs_f64(),
        limit.as_secs_f64()
    );
    #[cfg(unix)]
    unsafe {
        libc::write(2, msg.as_ptr().cast(), msg.len());
        libc::_exit(124)
    }
    // No `_exit` here: the process ends at once all the same, its other threads with it.
    #[cfg(not(unix))]
    {
        use std::io::Write as _;
        let _ = std::io::stderr().write_all(msg.as_bytes());
        std::process::exit(124)
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

/// Windows: the process's threads, each named by its description (`rerail_prof::windows_threads`).
#[cfg(windows)]
fn threads() -> Vec<(u64, String)> {
    rerail_prof::windows_threads().into_iter().map(|(id, name)| (u64::from(id), name)).collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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

    /// An allowance is the asking test's alone, also where another test's thread has the same name as the system
    /// reports it (Linux: `test_watchdog::` for both).
    #[test]
    fn an_allowance_is_the_asking_tests_alone() {
        allow(Duration::from_secs(1200));
        let other = std::thread::Builder::new()
            .name("test_watchdog::tests::another_test".into())
            .spawn(|| (mine(), threads()))
            .unwrap()
            .join()
            .unwrap();
        let (other, listed) = (other.0.unwrap(), other.1);
        let shown = short("test_watchdog::tests::another_test");
        assert!(listed.iter().any(|t| *key(t) == other && t.1 == shown), "{other:?} as {shown} not among {listed:?}");
        let allowed = ALLOWED.lock().unwrap_or_else(|e| e.into_inner());
        let allowed = allowed.as_ref().unwrap();
        assert_eq!(allowed.get(&mine().unwrap()), Some(&Duration::from_secs(1200)));
        assert_eq!(allowed.get(&other), None);
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
            .env("RERAIL_TEST_TIMEOUT", "0.4")
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(124), "{err}");
        // Named as the system reports the thread (`MAX_NAME` bytes: all of it on macOS and Windows, `test_watchdog::`
        // on Linux).
        assert!(err.contains(&format!("`{}` ran", short("test_watchdog::tests::a_test_past_its_limit_ends_the_run"))), "{err}");
        assert!(t0.elapsed() < Duration::from_secs(20));
    }

    /// A segmentation fault names its thread and the tests running, and still ends the process by the signal (a
    /// child, as above). Unix only: Windows has no signals, and the handler is Unix's.
    #[cfg(unix)]
    #[test]
    fn a_segmentation_fault_names_the_test() {
        if std::env::var_os("RERAIL_WATCHDOG_CHILD").is_some() {
            // Past the watchdog's first listing of the tests (200 ms after `main`, then every 125 ms under the
            // parent's limit of 1 s).
            std::thread::sleep(Duration::from_millis(500));
            // Its fault writes no core: where the kernel hands cores to a program (a `|` core pattern, which no core
            // limit stops), the dump was most of the test's time. Still ended by the signal.
            // SAFETY: a flag of this process alone.
            #[cfg(target_os = "linux")]
            unsafe {
                libc::prctl(libc::PR_SET_DUMPABLE, 0);
            }
            // SAFETY: none: the fault is the test.
            unsafe { std::ptr::null_mut::<u64>().write_volatile(1) };
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .args(["test_watchdog::tests::a_segmentation_fault_names_the_test", "--exact", "--nocapture"])
            .env("RERAIL_WATCHDOG_CHILD", "1")
            .env("RERAIL_TEST_TIMEOUT", "1")
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(out.status.signal(), Some(libc::SIGSEGV), "{err}");
        let name = short("test_watchdog::tests::a_segmentation_fault_names_the_test");
        assert!(err.contains(&format!("a segmentation fault on thread `{name}`")), "{err}");
        assert!(err.contains(&format!("  {name}\n")), "{err}");
    }
}

/// The fault handler (module docs): the names it writes are kept in a fixed buffer the watchdog fills each second, so
/// the handler only reads memory and writes to stderr. Unix only: a signal handler.
#[cfg(unix)]
mod faults {
    use std::cell::UnsafeCell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Buffer(UnsafeCell<[u8; 8192]>);
    // SAFETY: written by the watchdog thread only; read by a fault handler, which may see a line half-written.
    unsafe impl Sync for Buffer {}
    static RUNNING: Buffer = Buffer(UnsafeCell::new([0; 8192]));
    static LEN: AtomicUsize = AtomicUsize::new(0);
    static mut OLD: [std::mem::MaybeUninit<libc::sigaction>; 2] = [std::mem::MaybeUninit::uninit(); 2];

    /// The tests running now, a line each, for the handler.
    pub(super) fn running<'a>(names: impl Iterator<Item = &'a str>) {
        // SAFETY: the one writer (see `Buffer`).
        let buf = unsafe { &mut *RUNNING.0.get() };
        let mut n = 0;
        for name in names {
            let line = name.as_bytes();
            if n + line.len() + 3 > buf.len() {
                break;
            }
            buf[n..n + 2].copy_from_slice(b"  ");
            buf[n + 2..n + 2 + line.len()].copy_from_slice(line);
            buf[n + 2 + line.len()] = b'\n';
            n += line.len() + 3;
        }
        LEN.store(n, Ordering::Release);
    }

    pub(super) fn install() {
        for (i, sig) in [libc::SIGSEGV, libc::SIGBUS].into_iter().enumerate() {
            // SAFETY: a handler of plain reads and writes; the old action is kept for it to restore.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handler as extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(sig, &action, (&raw mut OLD[i]).cast());
            }
        }
    }

    /// `v` in hex, into a buffer of the handler's stack (a handler allocates nothing).
    fn hex(v: usize, out: &mut [u8; 18]) -> &[u8] {
        out[0] = b'0';
        out[1] = b'x';
        for i in 0..16 {
            let d = (v >> ((15 - i) * 4)) & 0xf;
            out[2 + i] = b"0123456789abcdef"[d];
        }
        &out[..]
    }

    /// The faulting instruction's address and its return address (the caller's), where the platform's context names
    /// them: a fault in JIT code has no symbols, and these place it.
    #[allow(unused_variables)]
    fn pc_lr(ctx: *mut libc::c_void) -> Option<(usize, usize)> {
        if ctx.is_null() {
            return None;
        }
        // SAFETY: the kernel's context for this signal (`SA_SIGINFO`), read where each platform lays it out.
        unsafe {
            let uc = ctx as *const libc::ucontext_t;
            #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
            return Some(((*uc).uc_mcontext.pc as usize, (*uc).uc_mcontext.regs[30] as usize));
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            return Some(((*(*uc).uc_mcontext).__ss.__pc as usize, (*(*uc).uc_mcontext).__ss.__lr as usize));
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            return Some(((*uc).uc_mcontext.gregs[libc::REG_RIP as usize] as usize, 0));
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            return Some(((*(*uc).uc_mcontext).__ss.__rip as usize, 0));
            #[allow(unreachable_code)]
            None
        }
    }

    extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        let write = |b: &[u8]| unsafe {
            libc::write(2, b.as_ptr().cast(), b.len());
        };
        let mut name = [0u8; 64];
        // SAFETY: the calling thread's name into a buffer of its size.
        unsafe { libc::pthread_getname_np(libc::pthread_self(), name.as_mut_ptr().cast(), name.len()) };
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        write(if sig == libc::SIGBUS { b"\ntest watchdog: a bus error on thread `" } else { b"\ntest watchdog: a segmentation fault on thread `" });
        write(&name[..end]);
        let mut buf = [0u8; 18];
        write(b"`");
        if !info.is_null() {
            write(b" at address ");
            // SAFETY: the kernel's siginfo for a fault, which names its address.
            write(hex(unsafe { (*info).si_addr() } as usize, &mut buf));
        }
        if let Some((pc, lr)) = pc_lr(ctx) {
            write(b", instruction ");
            write(hex(pc, &mut buf));
            write(b", return address ");
            write(hex(lr, &mut buf));
        }
        write(b"; the tests running:\n");
        let len = LEN.load(std::sync::atomic::Ordering::Acquire);
        // SAFETY: reads what the watchdog wrote (see `Buffer`).
        write(unsafe { &(&*RUNNING.0.get())[..len] });
        // The replaced handler takes the fault when it recurs on return.
        let i = usize::from(sig == libc::SIGBUS);
        // SAFETY: the action `install` saved.
        unsafe { libc::sigaction(sig, (&raw const OLD[i]).cast(), std::ptr::null_mut()) };
    }
}
