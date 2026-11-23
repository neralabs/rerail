//! Upkeep: maintenance jobs run in the background when the machine is idle or close to it (decision 0190).
//!
//! A job is anything that makes later work cheaper and can wait: collecting the store, keeping SQLite's files
//! small and its statistics fresh, building search indexes, training the cost model. Each part of the engine
//! registers its own ([`register`]); one thread ([`SCHEDULER`]) runs them, in bounded steps, under four limits:
//!
//! - **When.** A job is due ([`Due`]) when it has work. Work that can wait ([`Due::Idle`]) runs only while the
//!   machine is idle or close to it: the share of all cores busy, over the last poll, below
//!   [`Settings::idle_busy`], twice in a row. Work that cannot ([`Due::Urgent`], a store past its size limit) runs
//!   at once. Work kept waiting past its job's bound (`starve`) runs as urgent work does, so a machine never
//!   idle still gets it, at the lowest priority.
//! - **Power.** On battery, only urgent work runs, unless [`Settings::battery_min`] allows more above a charge.
//!   In low power mode or at a serious thermal state, only urgent work runs.
//! - **CPU.** The thread runs at the background quality of service (the efficiency cores, the lowest I/O
//!   priority) with its disk I/O throttled. Each step is bounded in time ([`Settings::step`]); after a step the
//!   thread sleeps so that the job's threads average at most [`Settings::cpu`] of one core.
//! - **Memory.** A job is given [`Settings::memory`] bytes ([`Slice::memory`]) and keeps within them (a SQLite
//!   cache, the samples read). Idle work waits while the process's footprint is above its budget
//!   (`budget::memory`): the engine is busy with its own work.
//!
//! The settings come from the environment once (`RERAIL_UPKEEP`, `RERAIL_UPKEEP_CPU`, `RERAIL_UPKEEP_MB`,
//! `RERAIL_UPKEEP_IDLE`, `RERAIL_UPKEEP_BATTERY`, `RERAIL_UPKEEP_JOBS`, `RERAIL_UPKEEP_LOG`), or [`configure`].
//! [`run_now`] runs every job's work to the end on the caller's thread, idle or not (the `upkeep` command).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// When a job has work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Due {
    /// None.
    No,
    /// Work that waits for an idle machine; after `starve` of waiting it runs as urgent work does.
    Idle { starve: Duration },
    /// Work that runs at once (still at the background quality of service, and within the CPU share).
    Urgent,
}

/// A job's shape: the priority its steps run at, and whether they wait for an idle machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Qos {
    /// Work that can wait: [`Due::Idle`] waits for an idle machine (or its `starve`), and every step runs at the
    /// background quality of service (the efficiency cores, I/O throttled).
    #[default]
    Background,
    /// Work behind a run, small and wanted soon (the frames a keep decision left unreachable): any due work runs at
    /// once, idle machine or not, on battery or not, at the utility quality of service, and the process finishes it
    /// before it exits ([`finish_soon`]). Still within the CPU share on the scheduler's thread.
    Utility,
}

/// What a step left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// More work: the job is asked again (after the CPU share's pause).
    More,
    /// The job's work is done for now: it is asked again at the next poll.
    Done,
}

/// A step's bounds: return by `until`, use at most `memory` bytes, and stop early when [`Slice::over`] says so.
pub struct Slice<'a> {
    pub until: Instant,
    pub memory: usize,
    stop: &'a AtomicBool,
    /// The file the job works on ([`Job::file`]): a run over it, in any process, stops the step.
    file: Option<std::path::PathBuf>,
    /// Whether a run stops the step: not once the job's work is past its deadline ([`Settings::deadline`]).
    yields: bool,
    /// A job that is not a run's own upkeep (its QoS not [`Qos::Utility`]): a run finishing ([`finish_soon`]) stops
    /// its step, which the process's exit waits for.
    background: bool,
}

impl<'a> Slice<'a> {
    /// Whether the step should return now: its time is up, the process is stopping, or a run began (in this
    /// process, or over the job's file in another: [`runs_active`]). A step stopped by a run leaves its work due.
    pub fn over(&self) -> bool {
        Instant::now() >= self.until || self.stop.load(Ordering::Acquire) || (self.yields && runs_active(self.file.as_deref())) || (self.background && finishing())
    }

    /// Whether a run began, or the process is stopping, as an owned check for what outlives a borrow of the slice
    /// and does not split at the step's time bound: a SQLite progress handler interrupting a transaction that is one
    /// piece (`SQLITE_INTERRUPT`, rolled back), a training given up between its phases.
    pub fn stopper(&self) -> impl Fn() -> bool + Send + Sync + 'static {
        let (file, yields, background, up) = (self.file.clone(), self.yields, self.background, upkeep());
        move || up.stop.load(Ordering::Acquire) || (yields && up.runs.load(Ordering::Acquire) > 0) || (yields && file.as_deref().is_some_and(others_run_over)) || (background && up.finishing.load(Ordering::Acquire) > 0)
    }

    /// A slice of `d` from now (tests, and jobs run by hand).
    pub fn for_test(d: Duration, memory: usize, stop: &AtomicBool) -> Slice<'_> {
        Slice { until: Instant::now() + d, memory, stop, file: None, yields: true, background: false }
    }

    /// The slice of a job over `file`: it yields to the runs over that file too ([`run_begins`]).
    pub fn over_file(self, file: &std::path::Path) -> Slice<'a> {
        Slice { file: Some(file.to_path_buf()), ..self }
    }
}

/// A probe of other processes' runs on a store: the store's path, its runs lock file (if it opened), when the probe
/// last looked, and whether another process held the lock then.
type ProbeHeld = (std::path::PathBuf, Option<std::fs::File>, Instant, bool);

/// The machine's upkeep (decision 0442): its jobs and their scheduler, the runs under way on it, the probes of other
/// processes' runs, the settings in force.
#[derive(Default)]
struct Upkeep {
    /// Runs under way on the machine ([`run_begins`]).
    runs: std::sync::atomic::AtomicUsize,
    probes: Mutex<Vec<ProbeHeld>>,
    settings: OnceLock<Mutex<Settings>>,
    jobs: Mutex<Vec<Entry>>,
    stop: AtomicBool,
    /// Runs finishing ([`finish_soon`]): a background job's step stops, and none starts, while one is.
    finishing: std::sync::atomic::AtomicUsize,
    /// Held while a step runs: [`finish_soon`] waits for a step the scheduler's thread has taken out.
    stepping: Mutex<()>,
    scheduler: OnceLock<Option<std::thread::Thread>>,
}

fn upkeep() -> std::sync::Arc<Upkeep> {
    rerail_context::machine::<Upkeep>()
}

/// Whether a run is finishing ([`finish_soon`]).
fn finishing() -> bool {
    upkeep().finishing.load(Ordering::Acquire) > 0
}

/// A run under way, from [`run_begins`] to its drop. While one is, upkeep yields to it: a step in progress stops at
/// its next check ([`Slice::over`]), and no step starts. Its work stays due, for after the run.
pub struct RunGuard {
    _locks: Vec<std::fs::File>,
    on: std::sync::Arc<Upkeep>,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.on.runs.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The lock file beside `file` that the runs over it hold shared (`<file>.runs.lock`).
fn runs_lock(file: &std::path::Path) -> std::path::PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".runs.lock");
    std::path::PathBuf::from(name)
}

/// A run begins: upkeep in this process yields until it ends, and upkeep in other processes over `files` (its store,
/// the cost statistics) yields too, through a shared lock on each one's `.runs.lock`. A file whose directory is
/// missing is skipped.
pub fn run_begins(files: &[&std::path::Path]) -> RunGuard {
    let on = upkeep();
    on.runs.fetch_add(1, Ordering::AcqRel);
    let locks = files
        .iter()
        .filter_map(|f| {
            let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(runs_lock(f)).ok()?;
            // Shared: runs do not wait for each other. A probe holds the lock exclusively for an instant. (`flock`;
            // on Windows `LockFileEx`.)
            lock.lock_shared().is_ok().then_some(lock)
        })
        .collect();
    RunGuard { _locks: locks, on }
}

/// Whether a run is under way: in this process, or (with `file`) in another over that file. The other processes'
/// answer is a non-blocking exclusive probe of `file`'s `.runs.lock`, asked at most every 25 ms a file.
pub fn runs_active(file: Option<&std::path::Path>) -> bool {
    upkeep().runs.load(Ordering::Acquire) > 0 || file.is_some_and(others_run_over)
}

/// Whether a run over `file` holds its `.runs.lock` (another process's, or this one's through [`run_begins`]).
fn others_run_over(file: &std::path::Path) -> bool {
    {
        let st_ = upkeep();
        let mut probes = st_.probes.lock().unwrap_or_else(|p| p.into_inner());
        let at = match probes.iter().position(|(p, ..)| p == file) {
            Some(i) => i,
            None => {
                let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(runs_lock(file)).ok();
                probes.push((file.to_path_buf(), lock, Instant::now() - Duration::from_secs(1), false));
                probes.len() - 1
            }
        };
        let (_, lock, asked, active) = &mut probes[at];
        if asked.elapsed() >= Duration::from_millis(25) {
            *asked = Instant::now();
            *active = lock.as_ref().is_some_and(|l| {
                if l.try_lock().is_ok() {
                    let _ = l.unlock();
                    false
                } else {
                    true
                }
            });
        }
        *active
    }
}

/// The file beside `file` that records when each job over it last finished (`<file>.upkeep`: `name\tunix seconds`
/// lines). A process's runs are short, so the time work has waited for them is counted across processes from here.
fn finishes(file: &std::path::Path) -> std::path::PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".upkeep");
    std::path::PathBuf::from(name)
}

/// When job `name` over `file` last finished: its record, else the file's creation (a job that never finished).
fn last_finished(file: &std::path::Path, name: &str) -> Option<std::time::SystemTime> {
    let recorded = std::fs::read_to_string(finishes(file)).ok().and_then(|text| {
        text.lines().find_map(|l| l.split_once('\t').filter(|(n, _)| *n == name).and_then(|(_, t)| t.parse::<u64>().ok()))
    });
    match recorded {
        Some(t) => Some(std::time::UNIX_EPOCH + Duration::from_secs(t)),
        None => std::fs::metadata(file).ok().and_then(|m| m.created().or_else(|_| m.modified()).ok()),
    }
}

/// Records that job `name` over `file` finished now: the record rewritten whole, then renamed into place.
fn record_finish(file: &std::path::Path, name: &str) {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = finishes(file);
    let mut text: String = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_once('\t').is_some_and(|(n, _)| n != name))
        .map(|l| format!("{l}\n"))
        .collect();
    text.push_str(&format!("{name}\t{at}\n"));
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// A maintenance job. `due` is called at every poll and must be cheap; `step` does a bounded part of the work.
pub trait Job: Send {
    /// Its name, unique among the jobs registered (an instance's, e.g. `sqlite content-v10.sqlite`). The kind is
    /// the part before the first space: what `RERAIL_UPKEEP_JOBS` names.
    fn name(&self) -> String;
    fn due(&mut self) -> Due;
    /// The threads a step keeps busy (the CPU share's pause is scaled by them).
    fn threads(&self) -> usize {
        1
    }
    fn step(&mut self, slice: &Slice) -> Result<Step, String>;
    /// Its shape ([`Qos`]): background work by default.
    fn qos(&self) -> Qos {
        Qos::Background
    }
    /// The file it works on (a store, the cost statistics): a run over it in another process stops its steps, as
    /// any run in this process does ([`runs_active`]). `None`: only this process's runs.
    fn file(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// The settings (decision 0190 §3).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Whether the scheduler runs (`RERAIL_UPKEEP=off`: no).
    pub on: bool,
    /// The share of one core the jobs average, 0–1 (`RERAIL_UPKEEP_CPU`, a percentage; 25).
    pub cpu: f64,
    /// The memory a job is given (`RERAIL_UPKEEP_MB`; 128, at most an eighth of the process's budget).
    pub memory: usize,
    /// The share of all cores busy below which the machine is idle (`RERAIL_UPKEEP_IDLE`, a percentage; 25).
    pub idle_busy: f64,
    /// On battery, the charge (percent) above which idle work runs; `None`: never (`RERAIL_UPKEEP_BATTERY`).
    pub battery_min: Option<f64>,
    /// The longest step.
    pub step: Duration,
    /// The time between polls on mains power (on battery, four times it).
    pub poll: Duration,
    /// The job kinds left out (`RERAIL_UPKEEP_JOBS=-sqlite,-cost-model`) or the only ones run (`=collect,sqlite`).
    pub only: Option<Vec<String>>,
    pub without: Vec<String>,
    /// Each step printed to standard error (`RERAIL_UPKEEP_LOG=1`).
    pub log: bool,
    /// How long due work yields to runs (`RERAIL_UPKEEP_DEADLINE`, seconds; 600): past it, counted from the job's
    /// last finish, its steps run through the runs (decision 0190 §9). Idle work's deadline is its `starve`, when
    /// that is longer.
    pub deadline: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            cpu: 0.25,
            memory: (128usize << 20).min(crate::budget::memory() / 8),
            idle_busy: 0.25,
            battery_min: None,
            step: Duration::from_millis(200),
            poll: Duration::from_secs(10),
            only: None,
            without: Vec::new(),
            log: false,
            deadline: Duration::from_secs(600),
        }
    }
}

impl Settings {
    /// The defaults, as the environment changes them. A setting that does not parse is reported and left out.
    pub fn from_env() -> Self {
        let mut s = Settings::default();
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let pct = |k: &str| {
            var(k).and_then(|v| match v.parse::<f64>() {
                Ok(p) if (0.0..=100.0).contains(&p) => Some(p / 100.0),
                _ => {
                    eprintln!("rerail: {k}={v} is not a percentage; left out");
                    None
                }
            })
        };
        if let Some(v) = var("RERAIL_UPKEEP") {
            s.on = !matches!(v.as_str(), "off" | "0" | "false");
        }
        if let Some(p) = pct("RERAIL_UPKEEP_CPU") {
            s.cpu = p.max(0.01);
        }
        if let Some(p) = pct("RERAIL_UPKEEP_IDLE") {
            s.idle_busy = p;
        }
        if let Some(v) = var("RERAIL_UPKEEP_BATTERY") {
            s.battery_min = match v.as_str() {
                "off" | "never" => None,
                _ => pct("RERAIL_UPKEEP_BATTERY").map(|p| p * 100.0),
            };
        }
        if let Some(v) = var("RERAIL_UPKEEP_MB") {
            match v.parse::<usize>() {
                Ok(mb) if mb > 0 => s.memory = mb << 20,
                _ => eprintln!("rerail: RERAIL_UPKEEP_MB={v} is not a size in MB; left out"),
            }
        }
        if let Some(v) = var("RERAIL_UPKEEP_JOBS") {
            let (out, only): (Vec<&str>, Vec<&str>) = v.split(',').map(str::trim).filter(|j| !j.is_empty()).partition(|j| j.starts_with('-'));
            s.without = out.iter().map(|j| j[1..].to_owned()).collect();
            s.only = (!only.is_empty()).then(|| only.iter().map(|j| j.to_string()).collect());
        }
        s.log = var("RERAIL_UPKEEP_LOG").is_some_and(|v| v != "0");
        if let Some(v) = var("RERAIL_UPKEEP_DEADLINE") {
            match v.parse::<u64>() {
                Ok(secs) => s.deadline = Duration::from_secs(secs),
                _ => eprintln!("rerail: RERAIL_UPKEEP_DEADLINE={v} is not a number of seconds; left out"),
            }
        }
        s
    }

    /// How long work due as `due` yields to runs; `None`: always (a whole `VACUUM`, whose `starve` is unbounded).
    pub fn deadline_of(&self, due: Due) -> Option<Duration> {
        match due {
            Due::No => None,
            Due::Urgent => Some(self.deadline),
            Due::Idle { starve } if starve == Duration::MAX => None,
            Due::Idle { starve } => Some(starve.max(self.deadline)),
        }
    }

    /// Whether jobs of this name's kind run.
    pub fn runs(&self, name: &str) -> bool {
        let kind = name.split(' ').next().unwrap_or(name);
        !self.without.iter().any(|k| k == kind) && self.only.as_ref().is_none_or(|o| o.iter().any(|k| k == kind))
    }
}

/// The settings in force.
pub fn settings() -> Settings {
    let up = upkeep();
    up.settings.get_or_init(|| Mutex::new(Settings::from_env())).lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Replaces the settings (a command's flags; tests).
pub fn configure(s: Settings) {
    let up = upkeep();
    *up.settings.get_or_init(|| Mutex::new(Settings::from_env())).lock().unwrap_or_else(|p| p.into_inner()) = s;
}

/// The machine's state, as the scheduler reads it at each poll.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Conditions {
    /// The share of all cores busy since the last reading, 0–1.
    pub busy: f64,
    pub on_battery: bool,
    /// The battery's charge in percent (none: no battery).
    pub charge: Option<f64>,
    pub low_power: bool,
    /// `NSProcessInfoThermalState`: 0 nominal, 1 fair, 2 serious, 3 critical.
    pub thermal: u8,
    /// The process's physical footprint, bytes (Linux: its resident memory).
    pub footprint: usize,
}

/// What may run under `c`, by the settings: (urgent work, idle work).
pub fn allowed(c: &Conditions, idle_twice: bool, s: &Settings) -> (bool, bool) {
    let hot = c.low_power || c.thermal >= 2;
    let battery_ok = !c.on_battery || s.battery_min.is_some_and(|m| c.charge.is_some_and(|ch| ch >= m));
    let idle = idle_twice && battery_ok && !hot && c.footprint <= crate::budget::memory();
    (true, idle)
}

/// The CPU ticks of all cores so far: (busy, total). (0, 0) where they are not read, which `read_conditions`
/// takes as busy.
pub fn cpu_ticks() -> (u64, u64) {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut n: libc::natural_t = 0;
        let mut info: libc::processor_info_array_t = std::ptr::null_mut();
        let mut count: libc::mach_msg_type_number_t = 0;
        #[allow(deprecated)]
        let host = libc::mach_host_self();
        if libc::host_processor_info(host, libc::PROCESSOR_CPU_LOAD_INFO, &mut n, &mut info, &mut count) != 0 {
            return (0, 0);
        }
        let ticks = std::slice::from_raw_parts(info as *const u32, count as usize);
        let (mut busy, mut total) = (0u64, 0u64);
        // Per core: user, system, idle, nice.
        for c in ticks.as_chunks::<4>().0 {
            busy += c[0] as u64 + c[1] as u64 + c[3] as u64;
            total += c.iter().map(|&t| t as u64).sum::<u64>();
        }
        #[allow(deprecated)]
        let task = libc::mach_task_self();
        libc::vm_deallocate(task, info as libc::vm_address_t, (count as usize * std::mem::size_of::<libc::integer_t>()) as libc::vm_size_t);
        (busy, total)
    }
    // /proc/stat's first line, every core's: user, nice, system, idle, iowait, irq, softirq, steal (guest time is
    // in user's already). Busy is all but idle and iowait: a core waiting on I/O runs nothing.
    #[cfg(target_os = "linux")]
    {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else { return (0, 0) };
        let t: Vec<u64> = stat.lines().next().unwrap_or("").split_whitespace().skip(1).take(8).filter_map(|v| v.parse().ok()).collect();
        if t.len() < 5 {
            return (0, 0);
        }
        let total: u64 = t.iter().sum();
        (total - t[3] - t[4], total)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    (0, 0)
}

/// The battery's state: (on battery, charge in percent). Without a battery: (false, None).
pub fn power() -> (bool, Option<f64>) {
    #[cfg(target_os = "macos")]
    unsafe {
        use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
        use core_foundation_sys::base::{CFRelease, CFTypeRef};
        use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
        use core_foundation_sys::number::{CFNumberGetValue, CFNumberRef, kCFNumberDoubleType};
        use core_foundation_sys::string::{CFStringCreateWithBytes, CFStringGetCString, CFStringRef, kCFStringEncodingUTF8};
        #[link(name = "IOKit", kind = "framework")]
        unsafe extern "C" {
            fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
            fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
            fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> CFDictionaryRef;
            fn IOPSGetProvidingPowerSourceType(blob: CFTypeRef) -> CFStringRef;
        }
        let key = |s: &str| CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as _, kCFStringEncodingUTF8, 0);
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return (false, None);
        }
        let mut on_battery = false;
        let kind = IOPSGetProvidingPowerSourceType(blob);
        if !kind.is_null() {
            let mut buf = [0i8; 64];
            if CFStringGetCString(kind, buf.as_mut_ptr(), buf.len() as _, kCFStringEncodingUTF8) != 0 {
                on_battery = std::ffi::CStr::from_ptr(buf.as_ptr()).to_bytes() == b"Battery Power";
            }
        }
        let mut charge = None;
        let list = IOPSCopyPowerSourcesList(blob);
        if !list.is_null() {
            let (cur, max) = (key("Current Capacity"), key("Max Capacity"));
            let num = |d: CFDictionaryRef, k: CFStringRef| {
                let v = CFDictionaryGetValue(d, k.cast()) as CFNumberRef;
                let mut x = 0f64;
                (!v.is_null() && CFNumberGetValue(v, kCFNumberDoubleType, (&mut x as *mut f64).cast())).then_some(x)
            };
            for i in 0..CFArrayGetCount(list) {
                let d = IOPSGetPowerSourceDescription(blob, CFArrayGetValueAtIndex(list, i));
                if let (false, Some(c), Some(m)) = (d.is_null(), num(d, cur), num(d, max))
                    && m > 0.0
                {
                    charge = Some(100.0 * c / m);
                    break;
                }
            }
            CFRelease(cur.cast());
            CFRelease(max.cast());
            CFRelease(list.cast());
        }
        CFRelease(blob);
        (on_battery, charge)
    }
    #[cfg(not(target_os = "macos"))]
    (false, None)
}

/// Low power mode, and the thermal state (`NSProcessInfo`).
pub fn heat() -> (bool, u8) {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc::runtime::Object;
        use objc::{class, msg_send, sel, sel_impl};
        let info: *mut Object = msg_send![class!(NSProcessInfo), processInfo];
        if info.is_null() {
            return (false, 0);
        }
        let low: bool = msg_send![info, isLowPowerModeEnabled];
        let thermal: isize = msg_send![info, thermalState];
        (low, thermal.clamp(0, 3) as u8)
    }
    #[cfg(not(target_os = "macos"))]
    (false, 0)
}

/// The process's physical footprint, bytes (Linux: its resident memory).
pub fn footprint() -> usize {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        if libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&mut info as *mut libc::rusage_info_v4).cast()) == 0 {
            return info.ri_phys_footprint as usize;
        }
        0
    }
    // Linux has no footprint: its resident pages (/proc/self/statm's second figure) are the nearest.
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
        let pages: usize = statm.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        // SAFETY: sysconf reads a constant.
        pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(0) as usize
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    0
}

/// The calling thread at upkeep's priority: the background quality of service (the efficiency cores, and disk
/// I/O throttled: a thread's own I/O policy would reset its quality of service, so none is set). Jobs that start
/// threads of their own (a training pool) call it on each. Where a quality of service is refused (`EPERM`, seen
/// under `cargo test`), the thread is put in Darwin's background state instead (`PRIO_DARWIN_BG`: the lowest CPU
/// and I/O priority). Which one took: [`lowered`].
pub fn lower_this_thread() {
    #[cfg(target_os = "macos")]
    unsafe {
        if libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0) != 0 {
            libc::setpriority(PRIO_DARWIN_THREAD, 0, PRIO_DARWIN_BG);
        }
    }
}

/// The calling thread at `qos`'s priority, for one step: [`lower_this_thread`] for background work; the utility
/// quality of service (out of Darwin's background state, if the thread was put there) for utility work.
fn set_thread_qos(qos: Qos) {
    match qos {
        Qos::Background => lower_this_thread(),
        Qos::Utility => {
            #[cfg(target_os = "macos")]
            unsafe {
                if libc::getpriority(PRIO_DARWIN_THREAD, 0) == 1 {
                    libc::setpriority(PRIO_DARWIN_THREAD, 0, 0);
                }
                libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
            }
        }
    }
}

#[cfg(target_os = "macos")]
const PRIO_DARWIN_THREAD: libc::c_int = 3;
#[cfg(target_os = "macos")]
const PRIO_DARWIN_BG: libc::c_int = 0x1000;

/// How the calling thread is lowered: `Some("qos")` (the background quality of service), `Some("darwin-bg")`
/// (Darwin's background state), `None` (not lowered).
pub fn lowered() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut qos = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        libc::pthread_get_qos_class_np(libc::pthread_self(), &mut qos, std::ptr::null_mut());
        if qos as u32 == libc::qos_class_t::QOS_CLASS_BACKGROUND as u32 {
            return Some("qos");
        }
        if libc::getpriority(PRIO_DARWIN_THREAD, 0) == 1 {
            return Some("darwin-bg");
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// A job's record, as [`report`] gives it.
#[derive(Clone, Debug, Default)]
pub struct JobReport {
    pub name: String,
    pub steps: u64,
    /// The steps' wall and the scheduler thread's CPU in them, seconds.
    pub wall: f64,
    pub cpu: f64,
    /// Since when the job has been due and not run (none: not due, or running).
    pub waiting: Option<f64>,
    pub last_error: Option<String>,
    /// Failures in a row (three hold the job for an hour).
    pub failures: u32,
}

struct Entry {
    id: u64,
    job: Box<dyn Job>,
    report: JobReport,
    due_since: Option<Instant>,
    held_until: Option<Instant>,
    last_ran: Option<Instant>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A registered job; dropping it takes the job out (a store closed).
pub struct Registered(u64, std::sync::Arc<Upkeep>);

impl Drop for Registered {
    fn drop(&mut self) {
        self.1.jobs.lock().unwrap_or_else(|p| p.into_inner()).retain(|e| e.id != self.0);
    }
}

/// Adds a job. It runs once the scheduler is started ([`start`], by a process that lives long enough: the
/// daemon, a run's statistics writer) or by [`run_now`].
pub fn register(job: Box<dyn Job>) -> Registered {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let name = job.name();
    upkeep().jobs.lock().unwrap_or_else(|p| p.into_inner()).push(Entry {
        id,
        job,
        report: JobReport { name, ..Default::default() },
        due_since: None,
        held_until: None,
        last_ran: None,
    });
    kick();
    Registered(id, upkeep())
}

/// Adds a job for the process's life, unless one of its name over its file is registered already: the runs over a
/// store share its job, which the registry itself holds (no guard to keep).
pub fn register_for_process(job: Box<dyn Job>) {
    let (name, file) = (job.name(), job.file());
    {
        let st_ = upkeep();
        let jobs = st_.jobs.lock().unwrap_or_else(|p| p.into_inner());
        if jobs.iter().any(|e| e.report.name == name && e.job.file() == file) {
            return;
        }
    }
    std::mem::forget(register(job));
}

/// Takes out every job over `file` (a store closed and about to be deleted while the process goes on: a test's own),
/// a step under way waited for. A job's connection to the file closes with it: kept, it held the deleted file open
/// until the job's next poll, and the process's jobs grew by one a store.
pub fn forget_file(file: &std::path::Path) {
    let canonical = std::fs::canonicalize(file).ok();
    let over = |f: Option<std::path::PathBuf>| f.is_some_and(|f| f == file || canonical.as_ref().is_some_and(|c| std::fs::canonicalize(&f).is_ok_and(|g| g == *c)));
    let st_ = upkeep();
    let _stepping = st_.stepping.lock().unwrap_or_else(|p| p.into_inner());
    let gone: Vec<Entry> = {
        let st_ = upkeep();
        let mut jobs = st_.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let (gone, kept) = std::mem::take(&mut *jobs).into_iter().partition(|e| over(e.job.file()));
        *jobs = kept;
        gone
    };
    // Dropped outside the registry's lock: a job's drop closes its connections.
    drop(gone);
}

/// Retire jobs for stores that their owner is about to remove. Call after closing
/// the users of those stores. Wait for a step that already took a job out of the list.
pub fn retire_files_under(directory: &std::path::Path) {
    let st_ = upkeep();
    let _stepping = st_.stepping.lock().unwrap_or_else(|p| p.into_inner());
    st_.jobs.lock().unwrap_or_else(|p| p.into_inner())
        .retain(|entry| !entry.job.file().is_some_and(|file| file.starts_with(directory)));
}

/// Starts the scheduler's thread, once (when the settings have it on).
pub fn start() {
    upkeep().scheduler.get_or_init(|| {
        if !settings().on {
            return None;
        }
        crate::exec::service("rerail-upkeep", None, scheduler).ok().map(|h| h.thread().clone())
    });
}

/// Asks the scheduler to poll now (a job that has just become due).
pub fn kick() {
    let st_ = upkeep();
    if let Some(Some(t)) = st_.scheduler.get() {
        t.unpark();
    }
}

/// Every job's record.
pub fn report() -> Vec<JobReport> {
    let st_ = upkeep();
    let jobs = st_.jobs.lock().unwrap_or_else(|p| p.into_inner());
    jobs.iter().map(|e| JobReport { waiting: e.due_since.map(|t| t.elapsed().as_secs_f64()), ..e.report.clone() }).collect()
}

/// The machine's state now, with `busy` over the ticks since `last` (which it updates).
pub fn read_conditions(last: &mut (u64, u64)) -> Conditions {
    let now = cpu_ticks();
    let (db, dt) = (now.0.saturating_sub(last.0), now.1.saturating_sub(last.1));
    *last = now;
    let (on_battery, charge) = power();
    let (low_power, thermal) = heat();
    Conditions { busy: if dt == 0 { 1.0 } else { db as f64 / dt as f64 }, on_battery, charge, low_power, thermal, footprint: footprint() }
}

fn scheduler() {
    lower_this_thread();
    let mut ticks = cpu_ticks();
    let mut idle_polls = 0u32;
    while !upkeep().stop.load(Ordering::Acquire) {
        let s = settings();
        let c = read_conditions(&mut ticks);
        idle_polls = if c.busy < s.idle_busy { idle_polls + 1 } else { 0 };
        let (_, idle) = allowed(&c, idle_polls >= 2, &s);
        let ran = run_due(&s, idle, &mut ticks, Pass::Scheduler);
        if !ran {
            std::thread::park_timeout(if c.on_battery { s.poll * 4 } else { s.poll });
        }
    }
}

/// Which pass: the scheduler's; every due job to the end ([`run_now`]); the utility jobs to the end, with no pause
/// ([`finish_soon`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Scheduler,
    All,
    Utility,
}

/// One pass over the jobs: each one due and allowed runs its steps until done, until the machine is busy again
/// (idle work), or for at most a poll's time. Whether any step ran.
fn run_due(s: &Settings, idle: bool, ticks: &mut (u64, u64), pass: Pass) -> bool {
    let all = pass != Pass::Scheduler;
    let now = Instant::now();
    // Which jobs to run, oldest run first; the lock is not held while a job steps.
    let mut order: Vec<(Option<Instant>, u64, bool, bool)> = Vec::new();
    {
        let st_ = upkeep();
        let mut jobs = st_.jobs.lock().unwrap_or_else(|p| p.into_inner());
        for e in jobs.iter_mut() {
            if !s.runs(&e.report.name) || e.held_until.is_some_and(|h| h > now) {
                continue;
            }
            let utility = e.job.qos() == Qos::Utility;
            if (pass == Pass::Utility || finishing()) && !utility {
                continue;
            }
            let due = e.job.due();
            let since = match due {
                Due::No => {
                    e.due_since = None;
                    continue;
                }
                _ => *e.due_since.get_or_insert(now),
            };
            // A run under way comes first: the job's work waits for it, still due, until its deadline. Past that
            // (counted from the job's last finish, in any process) its steps run through the runs.
            let file = e.job.file();
            let yields = if runs_active(file.as_deref()) {
                let waited = match file.as_deref().and_then(|f| last_finished(f, &e.report.name)) {
                    Some(t) => t.elapsed().unwrap_or_default(),
                    None => now.duration_since(since),
                };
                if s.deadline_of(due).is_none_or(|d| waited < d) {
                    continue;
                }
                false
            } else {
                true
            };
            let urgent = utility || !yields || match due {
                Due::Urgent => true,
                Due::Idle { starve } => now.duration_since(since) >= starve,
                Due::No => false,
            };
            if all || urgent || idle {
                order.push((e.last_ran, e.id, urgent, yields));
            }
        }
    }
    order.sort_by_key(|&(last, id, ..)| (last, id));
    let mut ran = false;
    let pass_end = now + s.poll;
    for (_, id, urgent, yields) in order {
        loop {
            if upkeep().stop.load(Ordering::Acquire) || (!all && Instant::now() >= pass_end) {
                return ran;
            }
            // `finish_soon` holds the stepping lock through its pass.
            let stepped = if pass == Pass::Utility { step_job_held(id, s, false, yields, None) } else { step_job(id, s, true, yields) };
            let Some((step, file)) = stepped else { break };
            ran = true;
            if step != Step::More {
                break;
            }
            if yields && runs_active(file.as_deref()) {
                return ran;
            }
            // Idle work stops once the machine is busy again.
            if !all && !urgent && read_conditions(ticks).busy >= s.idle_busy {
                return ran;
            }
        }
    }
    ran
}

/// One step of job `id` (taken out of the list while it runs), at its priority, then the CPU share's pause (with
/// `pause`). `None`: the job is gone, or failed.
fn step_job(id: u64, s: &Settings, pause: bool, yields: bool) -> Option<(Step, Option<std::path::PathBuf>)> {
    let st_ = upkeep();
    let stepping = st_.stepping.lock().unwrap_or_else(|p| p.into_inner());
    step_job_held(id, s, pause, yields, Some(stepping))
}

/// `step_job`, the stepping lock held by the caller (`finish_soon`, through its whole pass) or given to it (`stepping`,
/// let go before the step's pause).
fn step_job_held(id: u64, s: &Settings, pause: bool, yields: bool, stepping: Option<std::sync::MutexGuard<'_, ()>>) -> Option<(Step, Option<std::path::PathBuf>)> {
    let mut e = {
        let st_ = upkeep();
        let mut jobs = st_.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let i = jobs.iter().position(|e| e.id == id)?;
        jobs.swap_remove(i)
    };
    let threads = e.job.threads().max(1) as f64;
    // The scheduler's thread is background between steps; the caller's ([`run_now`], [`finish_soon`]) is the
    // caller's own.
    let st_ = upkeep();
    let scheduler = st_.scheduler.get().and_then(Option::as_ref).is_some_and(|t| t.id() == std::thread::current().id());
    let qos = e.job.qos();
    if scheduler && qos != Qos::Background {
        set_thread_qos(qos);
    }
    let (c0, t0) = (crate::score::thread_cpu(), Instant::now());
    let file = e.job.file();
    let st_ = upkeep();
    let slice = Slice { until: t0 + s.step, memory: s.memory, stop: &st_.stop, file: file.clone(), yields, background: qos != Qos::Utility };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.job.step(&slice))).unwrap_or_else(|p| {
        crate::exec::abort_over_budget(p);
        Err("panicked".into())
    });
    let (wall, cpu) = (t0.elapsed().as_secs_f64(), crate::score::thread_cpu() - c0);
    if scheduler && qos != Qos::Background {
        set_thread_qos(Qos::Background);
    }
    e.report.steps += 1;
    e.report.wall += wall;
    e.report.cpu += cpu;
    e.last_ran = Some(Instant::now());
    let out = match r {
        Ok(step) => {
            e.report.failures = 0;
            if step == Step::Done {
                e.due_since = None;
                if let Some(f) = &file {
                    record_finish(f, &e.report.name);
                }
            }
            Some((step, file))
        }
        Err(err) => {
            e.report.failures += 1;
            if e.report.failures >= 3 {
                e.held_until = Some(Instant::now() + Duration::from_secs(3600));
            }
            e.report.last_error = Some(err);
            None
        }
    };
    if s.log {
        eprintln!("rerail upkeep: {} step {:.1} ms wall {:.1} ms CPU: {:?}", e.report.name, wall * 1e3, cpu * 1e3, out.as_ref().map_or_else(|| e.report.last_error.clone().unwrap_or_default(), |o| format!("{:?}", o.0)));
    }
    upkeep().jobs.lock().unwrap_or_else(|p| p.into_inner()).push(e);
    drop(stepping);
    // The pause that keeps `threads` busy for `wall` within `cpu` of a core on average.
    let pause = if pause { wall * (threads / s.cpu - 1.0) } else { 0.0 };
    if pause > 0.0 && !upkeep().stop.load(Ordering::Acquire) {
        std::thread::park_timeout(Duration::from_secs_f64(pause.min(60.0)));
    }
    out
}

/// Runs every registered job that is due to the end on the caller's thread, idle or not, within the CPU share
/// (the `upkeep` command; tests). The jobs' records after.
pub fn run_now() -> Vec<JobReport> {
    let s = settings();
    let mut ticks = cpu_ticks();
    // Until a pass finds nothing due, at most a hundred passes.
    for _ in 0..100 {
        if !run_due(&s, true, &mut ticks, Pass::All) {
            break;
        }
    }
    report()
}

/// Runs the utility jobs' due work to the end on the caller's thread, with no pause: what a run leaves behind it
/// (a keep decision's unreachable frames) is done before the process exits, whether or not the scheduler's thread
/// got to it. A step the scheduler's thread is in is waited for. Nothing when upkeep is off.
pub fn finish_soon() {
    let s = settings();
    if !s.on {
        return;
    }
    // A step the scheduler's thread is in has its job out of the list: wait for it, and hold the scheduler off
    // through the pass, so a step it takes out next is not one this pass misses either. Not waited for, the process
    // exited under the step, and its work was rolled back (ep2's frames sweep, 31 runs in a row).
    let st_ = upkeep();
    // A background job's step the scheduler's thread is in (idle work: a cost model's fit, a kernel's compile) stops
    // at its next check, its work left due, and none starts: an idle machine at a run's end made the exit wait for a
    // whole fit (t50: 260 s and 15 GB past a 42 s run).
    struct Finishing(std::sync::Arc<Upkeep>);
    impl Drop for Finishing {
        fn drop(&mut self) {
            self.0.finishing.fetch_sub(1, Ordering::AcqRel);
        }
    }
    st_.finishing.fetch_add(1, Ordering::AcqRel);
    let _finishing = Finishing(std::sync::Arc::clone(&st_));
    let _stepping = st_.stepping.lock().unwrap_or_else(|p| p.into_inner());
    let mut ticks = cpu_ticks();
    for _ in 0..100 {
        if !run_due(&s, true, &mut ticks, Pass::Utility) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter {
        name: &'static str,
        left: u32,
        due: Due,
        qos: Qos,
    }

    /// A run finishing stops a background job's step (idle work the exit would wait for), not a utility job's, which
    /// `finish_soon` runs itself; and no background step starts while it is.
    #[test]
    fn a_run_finishing_stops_a_background_step_not_a_utility_one() {
        let stop = AtomicBool::new(false);
        let mk = |background| Slice { until: Instant::now() + Duration::from_secs(10), memory: 0, stop: &stop, file: None, yields: false, background };
        let (bg, ut) = (mk(true), mk(false));
        assert!(!bg.over() && !bg.stopper()() && !ut.over(), "nothing finishing");
        upkeep().finishing.fetch_add(1, Ordering::AcqRel);
        let (over, stopped, utility) = (bg.over(), bg.stopper()(), ut.over() || ut.stopper()());
        upkeep().finishing.fetch_sub(1, Ordering::AcqRel);
        assert!(over && stopped, "a background step stops while a run finishes");
        assert!(!utility, "a utility step goes on");
        assert!(!bg.over(), "and goes on again once it has finished");
    }

    /// A job over a file: a run over the file stops it until its deadline.
    struct OverFile(Counter, std::path::PathBuf);

    impl Job for OverFile {
        fn name(&self) -> String {
            self.0.name()
        }
        fn due(&mut self) -> Due {
            self.0.due()
        }
        fn step(&mut self, slice: &Slice) -> Result<Step, String> {
            assert!(!slice.over() || Instant::now() >= slice.until, "past its deadline, a run does not stop the step");
            self.0.step(slice)
        }
        fn file(&self) -> Option<std::path::PathBuf> {
            Some(self.1.clone())
        }
    }

    impl Job for Counter {
        fn name(&self) -> String {
            self.name.into()
        }
        fn due(&mut self) -> Due {
            if self.left == 0 { Due::No } else { self.due }
        }
        fn step(&mut self, _: &Slice) -> Result<Step, String> {
            self.left -= 1;
            Ok(if self.left == 0 { Step::Done } else { Step::More })
        }
        fn qos(&self) -> Qos {
            self.qos
        }
    }

    /// A store forgotten (`forget_file`) takes its jobs out, the process's own among them; another file's stay.
    #[test]
    fn a_forgotten_files_jobs_are_taken_out() {
        let dir = std::env::temp_dir();
        let (a, b) = (dir.join("rerail-upkeep-forget-a.sqlite"), dir.join("rerail-upkeep-forget-b.sqlite"));
        let job = |name: &'static str, file: &std::path::Path| Box::new(OverFile(Counter { name, left: 0, due: Due::No, qos: Qos::Background }, file.to_path_buf()));
        register_for_process(job("test-forget a", &a));
        register_for_process(job("test-forget b", &b));
        let names = || report().into_iter().map(|r| r.name).filter(|n| n.starts_with("test-forget")).collect::<Vec<_>>();
        assert_eq!(names().len(), 2);
        forget_file(&a);
        assert_eq!(names(), ["test-forget b"]);
        forget_file(&b);
        assert!(names().is_empty());
    }

    #[test]
    fn settings_parse_and_name_kinds() {
        let s = Settings { only: Some(vec!["sqlite".into()]), without: vec!["collect".into()], ..Default::default() };
        assert!(s.runs("sqlite content-v10.sqlite"));
        assert!(!s.runs("collect content-v10.sqlite"));
        assert!(!s.runs("cost-model"));
        let all = Settings::default();
        assert!(all.runs("cost-model") && all.cpu > 0.0 && all.memory > 0);
    }

    #[test]
    fn power_heat_and_load_read() {
        let mut last = cpu_ticks();
        std::thread::sleep(Duration::from_millis(200));
        let c = read_conditions(&mut last);
        assert!((0.0..=1.0).contains(&c.busy), "{c:?}");
        assert!(c.charge.is_none_or(|ch| (0.0..=100.5).contains(&ch)), "{c:?}");
        // The footprint is macOS's figure (`proc_pid_rusage`); elsewhere it is not read and is 0.
        assert!(c.footprint > 0 || cfg!(not(target_os = "macos")), "{c:?}");
        eprintln!("CONDITIONS {c:?}");
    }

    #[test]
    fn idle_work_waits_for_power_and_quiet() {
        let s = Settings::default();
        let quiet = Conditions { busy: 0.05, footprint: 1, ..Default::default() };
        assert_eq!(allowed(&quiet, true, &s), (true, true));
        assert_eq!(allowed(&quiet, false, &s), (true, false), "one quiet poll is not idle");
        assert_eq!(allowed(&Conditions { on_battery: true, charge: Some(90.0), ..quiet }, true, &s), (true, false));
        let on_battery = Settings { battery_min: Some(80.0), ..s.clone() };
        assert_eq!(allowed(&Conditions { on_battery: true, charge: Some(90.0), ..quiet }, true, &on_battery), (true, true));
        assert_eq!(allowed(&Conditions { on_battery: true, charge: Some(50.0), ..quiet }, true, &on_battery), (true, false));
        assert_eq!(allowed(&Conditions { thermal: 2, ..quiet }, true, &s), (true, false));
        assert_eq!(allowed(&Conditions { low_power: true, ..quiet }, true, &s), (true, false));
    }

    /// A job seen from inside: its thread's quality of service, and that the scheduler's own thread ran it.
    type Seen = std::sync::Arc<Mutex<Vec<(String, Option<&'static str>)>>>;
    struct Probe(Seen);

    impl Job for Probe {
        fn name(&self) -> String {
            "test-probe".into()
        }
        fn due(&mut self) -> Due {
            if self.0.lock().unwrap().len() < 3 { Due::Idle { starve: Duration::from_secs(3600) } } else { Due::No }
        }
        fn step(&mut self, _: &Slice) -> Result<Step, String> {
            self.0.lock().unwrap().push((std::thread::current().name().unwrap_or("").to_owned(), lowered()));
            Ok(Step::More)
        }
    }

    /// The scheduler's thread, started: on an idle machine (here: any machine, the threshold above 100%) it runs a
    /// due job's steps at the background quality of service, on its own thread, within a few polls.
    #[test]
    #[ignore = "starts the process's scheduler: --ignored --test-threads 1"]
    fn the_scheduler_runs_jobs_on_its_thread_at_background_priority() {
        configure(Settings { idle_busy: 1.01, poll: Duration::from_millis(100), only: Some(vec!["test-probe".into()]), ..Settings::default() });
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let _job = register(Box::new(Probe(seen.clone())));
        start();
        let t = Instant::now();
        while seen.lock().unwrap().len() < 3 && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let seen = seen.lock().unwrap().clone();
        eprintln!("STEPS {} after {:.2} s, lowered by {:?}", seen.len(), t.elapsed().as_secs_f64(), seen[0].1);
        assert!(seen.len() >= 3, "{seen:?}");
        assert!(seen.iter().all(|(n, q)| n == "rerail-upkeep" && q.is_some()), "{seen:?}");
    }

    /// Another process's run over a file stops upkeep over it: a shared lock on its `.runs.lock` (here held through
    /// a second open file, as another process holds it), seen by the exclusive probe; released, the probe sees none.
    #[test]
    fn a_run_in_another_process_is_seen_through_its_lock() {
        let dir = std::env::temp_dir().join(format!("rerail-upkeep-runs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("store.sqlite");
        assert!(!others_run_over(&store), "no run yet");
        let other = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(runs_lock(&store)).unwrap();
        other.lock_shared().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(others_run_over(&store), "the other run's shared lock is seen");
        let slice = Slice { until: Instant::now() + Duration::from_secs(10), memory: 0, stop: &upkeep().stop, file: Some(store.clone()), yields: true, background: false };
        assert!(slice.over() && slice.stopper()(), "a step over the file stops");
        drop(other);
        std::thread::sleep(Duration::from_millis(30));
        assert!(!others_run_over(&store), "released");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The test jobs' kinds, the same in every test that configures: each test's `configure` keeps the others' jobs.
    const TEST_JOBS: [&str; 7] = ["test-a", "test-b", "test-c", "test-d", "test-e", "test-f", "test-g"];

    /// Held by the tests that step the process's jobs (`finish_soon`, `run_due`): one's pass would step the other's.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Utility work whose step takes a while (a sweep of a big backlog).
    struct Slow {
        started: std::sync::Arc<AtomicBool>,
        done: std::sync::Arc<AtomicBool>,
    }

    impl Job for Slow {
        fn name(&self) -> String {
            "test-g slow".into()
        }
        fn due(&mut self) -> Due {
            if self.done.load(Ordering::Acquire) { Due::No } else { Due::Urgent }
        }
        fn qos(&self) -> Qos {
            Qos::Utility
        }
        fn step(&mut self, _: &Slice) -> Result<Step, String> {
            self.started.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(300));
            self.done.store(true, Ordering::Release);
            Ok(Step::Done)
        }
    }

    /// A step the scheduler's thread is in when the process ends is waited for (`STEPPING`): its job is out of the
    /// list while it steps, and the process exiting under it rolled its work back. On ep2 the frames sweep of a big
    /// backlog (10 s) was lost at every process's end, 31 runs in a row.
    #[test]
    fn the_process_end_waits_for_a_step_under_way() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        let (started, done) = (std::sync::Arc::new(AtomicBool::new(false)), std::sync::Arc::new(AtomicBool::new(false)));
        let g = register(Box::new(Slow { started: std::sync::Arc::clone(&started), done: std::sync::Arc::clone(&done) }));
        configure(Settings { only: Some(TEST_JOBS.map(String::from).to_vec()), ..Settings::default() });
        let id = g.0;
        // The scheduler's thread takes the job out and steps it.
        let scheduler = std::thread::spawn(move || step_job(id, &settings(), false, true));
        while !started.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        finish_soon();
        assert!(done.load(Ordering::Acquire), "the process's end waited for the step under way");
        scheduler.join().unwrap();
    }

    #[test]
    fn jobs_step_to_the_end_within_the_cpu_share() {
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        let idle = Due::Idle { starve: Duration::from_secs(3600) };
        let _a = register(Box::new(Counter { name: "test-a one", left: 3, due: idle, qos: Qos::Background }));
        let _b = register(Box::new(Counter { name: "test-b one", left: 2, due: Due::Urgent, qos: Qos::Background }));
        // Utility work runs at once though it says it can wait; `finish_soon` finishes it, and only it.
        let _c = register(Box::new(Counter { name: "test-c one", left: 2, due: idle, qos: Qos::Utility }));
        let _e = register(Box::new(Counter { name: "test-e one", left: 2, due: idle, qos: Qos::Background }));
        configure(Settings { only: Some(TEST_JOBS.map(String::from).to_vec()), ..Settings::default() });
        let mut ticks = cpu_ticks();
        // Not idle: only the urgent job runs.
        run_due(&settings(), false, &mut ticks, Pass::Scheduler);
        run_due(&settings(), false, &mut ticks, Pass::Scheduler);
        let steps = |n: &str| report().into_iter().find(|r| r.name == n).map(|r| r.steps);
        assert_eq!((steps("test-a one"), steps("test-b one"), steps("test-c one")), (Some(0), Some(2), Some(2)));
        // Utility work left to the process's end is finished there, with no scheduler; not the background work.
        let _d = register(Box::new(Counter { name: "test-d one", left: 2, due: idle, qos: Qos::Utility }));
        // A run under way comes first: nothing steps, and the work stays due for after it.
        let run = run_begins(&[]);
        finish_soon();
        assert_eq!(steps("test-d one"), Some(0), "a run is under way");
        drop(run);
        finish_soon();
        assert_eq!((steps("test-d one"), steps("test-e one")), (Some(2), Some(0)));
        let r = run_now();
        assert_eq!(r.iter().find(|r| r.name == "test-a one").map(|r| r.steps), Some(3));
        // Decision 0190 §9: work over a file that another process's run holds waits for the run until its
        // deadline, counted from the job's last finish (in any process: `<file>.upkeep`), then runs through it.
        {
            let dir = std::env::temp_dir().join(format!("rerail-upkeep-deadline-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("store.sqlite");
            std::fs::write(&file, b"").unwrap();
            let other = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(runs_lock(&file)).unwrap();
            other.lock_shared().unwrap();
            std::thread::sleep(Duration::from_millis(30));
            let _f = register(Box::new(OverFile(Counter { name: "test-f one", left: 2, due: Due::Urgent, qos: Qos::Background }, file.clone())));
            run_due(&settings(), true, &mut ticks, Pass::Scheduler);
            assert_eq!(steps("test-f one"), Some(0), "the file is new: within its deadline, the work waits for the run");
            std::fs::write(finishes(&file), "test-f one\t0\n").unwrap();
            run_due(&settings(), true, &mut ticks, Pass::Scheduler);
            assert_eq!(steps("test-f one"), Some(2), "last finished long ago: past its deadline, it runs through the run");
            let finished = last_finished(&file, "test-f one").unwrap();
            assert!(finished.elapsed().unwrap() < Duration::from_secs(60), "its finish is recorded");
            drop(other);
            let _ = std::fs::remove_dir_all(&dir);
        }
        configure(Settings::default());
    }
}
