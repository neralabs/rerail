//! The process's life, from its start to its report: what the profile's spans cannot see. A run's spans begin in
//! `main`; before it, the system loads the binary, and a new binary's first run waits there for the system's check
//! of it (on macOS, `syspolicyd`'s assessment: 12 to 15 s for the 415 MB debug engine, 0.01 s on its next run). The
//! report's own work (the roofline's calibration, once per binary and day) comes after the CPU the spans account
//! for. And child processes (Git, Watchman) have CPU of their own. A cold run measured from outside (`time`) shows
//! all of these, and the spans none of them.

use std::sync::OnceLock;
use std::time::SystemTime;

static ENTERED: OnceLock<SystemTime> = OnceLock::new();

/// Marks the process's `main`: called first thing (`watch::watch`). Later calls change nothing.
pub fn entered() {
    if ENTERED.set(SystemTime::now()).is_ok() {
        #[cfg(windows)]
        windows::own_job();
    }
}

/// When the system started this process (its fork), if it says.
pub fn started() -> Option<SystemTime> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `proc_pidinfo` fills at most `size` bytes of `info`, a plain C struct, for this process.
        let info = unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            let n = libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size);
            if n != size {
                return None;
            }
            info
        };
        Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(info.pbi_start_tvsec) + std::time::Duration::from_micros(info.pbi_start_tvusec))
    }
    #[cfg(windows)]
    {
        windows::started()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// User and system CPU, in seconds, of this process (all its threads) or of the child processes it waited for.
pub fn cpu(children: bool) -> f64 {
    #[cfg(unix)]
    {
        // SAFETY: fills `u`, a plain C struct.
        let u = unsafe {
            let mut u: libc::rusage = std::mem::zeroed();
            libc::getrusage(if children { libc::RUSAGE_CHILDREN } else { libc::RUSAGE_SELF }, &mut u);
            u
        };
        let s = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        s(u.ru_utime) + s(u.ru_stime)
    }
    #[cfg(windows)]
    {
        let (user, kernel) = rerail_prof::windows_cpu(false);
        let own = (user + kernel).as_secs_f64();
        if children { windows::job_cpu().map_or(0.0, |all| (all - own).max(0.0)) } else { own }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = children;
        0.0
    }
}

/// Windows has no `RUSAGE_CHILDREN`: the process puts itself in a job of its own at `main` (`entered`), which its
/// child processes join as they start, and the job's accounting counts every process in it, those exited too. Its
/// children's CPU is the job's less its own. Nothing is held: the job is the process's, found from it.
#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation, QueryInformationJobObject};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    /// Puts the process in a new job (nested in any it is in already). The job lives while a process is in it, so its
    /// handle is closed at once. If it cannot be made, the job queried is the one the process is in, if any.
    pub(super) fn own_job() {
        // SAFETY: a new unnamed job with default security; the pseudo-handle names this process.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                AssignProcessToJobObject(job, GetCurrentProcess());
                CloseHandle(job);
            }
        }
    }

    /// The user and kernel CPU seconds of every process of the process's job: none outside a job.
    pub(super) fn job_cpu() -> Option<f64> {
        // SAFETY: a null handle names the calling process's job; the out-pointer is the class's struct, of its size.
        let info = unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            let size = std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32;
            let got = QueryInformationJobObject(std::ptr::null_mut(), JobObjectBasicAccountingInformation, (&raw mut info).cast(), size, std::ptr::null_mut());
            if got == 0 {
                return None;
            }
            info
        };
        // 100 ns ticks.
        Some((info.TotalUserTime + info.TotalKernelTime) as f64 / 1e7)
    }

    /// When the system created the process.
    pub(super) fn started() -> Option<std::time::SystemTime> {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the pseudo-handle names this process, and every out-pointer is a FILETIME.
        if unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return None;
        }
        // 100 ns ticks since 1601-01-01, which is 11,644,473,600 s before the Unix epoch.
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let since_1601 = std::time::Duration::from_nanos(ticks.checked_mul(100)?);
        std::time::SystemTime::UNIX_EPOCH.checked_add(since_1601.checked_sub(std::time::Duration::from_secs(11_644_473_600))?)
    }
}

/// The process as its report begins.
#[derive(Debug, Clone)]
pub struct Life {
    /// Wall seconds from the process's start to its `main` (`entered`): none when either is unknown.
    pub before_main: Option<f64>,
    /// Wall seconds from `main` (or the process's start) to now.
    pub to_now: Option<f64>,
    /// The process's CPU (all threads, user + system), and that of its child processes waited for.
    pub cpu: f64,
    pub children: f64,
}

pub fn now() -> Life {
    let now = SystemTime::now();
    let started = started();
    let entered = ENTERED.get().copied();
    let secs = |a: SystemTime, b: SystemTime| b.duration_since(a).ok().map(|d| d.as_secs_f64());
    Life {
        before_main: started.zip(entered).and_then(|(s, e)| secs(s, e)),
        to_now: entered.or(started).and_then(|from| secs(from, now)),
        cpu: cpu(false),
        children: cpu(true),
    }
}

/// The report's section: the process's life, then `after` (work the report itself did, as (what, wall s, CPU s)).
pub fn section(life: &Life, after: &[(String, f64, f64)]) -> String {
    use std::fmt::Write as _;
    let mut r = String::from("== The process, start to report (what no span sees; `time` counts it all)\n");
    match life.before_main {
        Some(s) => {
            let _ = writeln!(r, "  {s:>8.2} s wall before main: the system loading the binary (a new binary's first run waits here for the system's check of it)");
        }
        None => r.push_str("         - before main: not known (no start time, or main not marked)\n"),
    }
    if let Some(s) = life.to_now {
        let _ = writeln!(r, "  {s:>8.2} s wall from main to the report");
    }
    let _ = writeln!(r, "  {:>8.2} s CPU of the process to the report, every thread (user + system)", life.cpu);
    let _ = writeln!(r, "  {:>8.2} s CPU of the child processes it waited for", life.children);
    for (what, wall, cpu) in after {
        let _ = writeln!(r, "  {cpu:>8.2} s CPU, {wall:.2} s wall, after it: {what}");
    }
    r
}

#[cfg(test)]
mod tests {
    /// The process's start is before its `main` and its report; a child process's CPU is counted apart from its own.
    #[test]
    fn a_processs_life_names_its_start_its_main_and_its_children() {
        super::entered();
        let spin = std::time::Instant::now();
        let mut x = 0u64;
        while spin.elapsed() < std::time::Duration::from_millis(30) {
            x = std::hint::black_box(x.wrapping_add(1));
        }
        let before = super::cpu(true);
        #[cfg(unix)]
        std::process::Command::new("/bin/sh").args(["-c", "i=0; while [ $i -lt 20000 ]; do i=$((i+1)); done"]).status().unwrap();
        #[cfg(windows)]
        std::process::Command::new("cmd").args(["/C", "for /L %i in (1,1,200000) do @rem"]).status().unwrap();
        let life = super::now();
        #[cfg(any(target_os = "macos", windows))]
        {
            assert!(life.before_main.is_some_and(|s| s >= 0.0), "{life:?}");
            assert!(life.to_now.is_some_and(|s| s >= 0.0), "{life:?}");
        }
        assert!(life.cpu >= 0.02, "the spin counted: {life:?}");
        assert!(life.children > before, "the child's CPU counted: {life:?}");
        let text = super::section(&life, &[("the roofs measured".into(), 1.5, 1.25)]);
        assert!(text.contains("before main") && text.contains("child processes") && text.contains("1.25 s CPU, 1.50 s wall, after it: the roofs measured"), "{text}");
    }
}
