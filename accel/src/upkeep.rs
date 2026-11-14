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
}

impl Slice<'_> {
    /// Whether the step should return now: its time is up, or the process is stopping.
    pub fn over(&self) -> bool {
        Instant::now() >= self.until || self.stop.load(Ordering::Acquire)
    }

    /// A slice of `d` from now (tests, and jobs run by hand).
    pub fn for_test(d: Duration, memory: usize, stop: &AtomicBool) -> Slice<'_> {
        Slice { until: Instant::now() + d, memory, stop }
    }
}

/// A maintenance job. `due` is called at every poll and must be cheap; `step` does a bounded part of the work.
pub trait Job: Send {
    /// Its name, unique among the jobs registered (an instance's, e.g. `sqlite content-v4.sqlite`). The kind is
    /// the part before the first space: what `RERAIL_UPKEEP_JOBS` names.
    fn name(&self) -> String;
    fn due(&mut self) -> Due;
    /// The threads a step keeps busy (the CPU share's pause is scaled by them).
    fn threads(&self) -> usize {
        1
    }
    fn step(&mut self, slice: &Slice) -> Result<Step, String>;
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
        s
    }

    /// Whether jobs of this name's kind run.
    pub fn runs(&self, name: &str) -> bool {
        let kind = name.split(' ').next().unwrap_or(name);
        !self.without.iter().any(|k| k == kind) && self.only.as_ref().is_none_or(|o| o.iter().any(|k| k == kind))
    }
}

static SETTINGS: OnceLock<Mutex<Settings>> = OnceLock::new();

fn settings_cell() -> &'static Mutex<Settings> {
    SETTINGS.get_or_init(|| Mutex::new(Settings::from_env()))
}

/// The settings in force.
pub fn settings() -> Settings {
    settings_cell().lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Replaces the settings (a command's flags; tests).
pub fn configure(s: Settings) {
    *settings_cell().lock().unwrap_or_else(|p| p.into_inner()) = s;
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
    /// The process's physical footprint, bytes.
    pub footprint: usize,
}

/// What may run under `c`, by the settings: (urgent work, idle work).
pub fn allowed(c: &Conditions, idle_twice: bool, s: &Settings) -> (bool, bool) {
    let hot = c.low_power || c.thermal >= 2;
    let battery_ok = !c.on_battery || s.battery_min.is_some_and(|m| c.charge.is_some_and(|ch| ch >= m));
    let idle = idle_twice && battery_ok && !hot && c.footprint <= crate::budget::memory();
    (true, idle)
}

/// The CPU ticks of all cores so far: (busy, total).
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
        for c in ticks.chunks_exact(4) {
            busy += c[0] as u64 + c[1] as u64 + c[3] as u64;
            total += c.iter().map(|&t| t as u64).sum::<u64>();
        }
        #[allow(deprecated)]
        let task = libc::mach_task_self();
        libc::vm_deallocate(task, info as libc::vm_address_t, (count as usize * std::mem::size_of::<libc::integer_t>()) as libc::vm_size_t);
        (busy, total)
    }
    #[cfg(not(target_os = "macos"))]
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
#[allow(unexpected_cfgs)] // objc 0.2's macros test a `cargo-clippy` feature.
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

/// The process's physical footprint, bytes.
pub fn footprint() -> usize {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info: libc::rusage_info_v4 = std::mem::zeroed();
        if libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&mut info as *mut libc::rusage_info_v4).cast()) == 0 {
            return info.ri_phys_footprint as usize;
        }
        0
    }
    #[cfg(not(target_os = "macos"))]
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

static JOBS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static STOP: AtomicBool = AtomicBool::new(false);
static SCHEDULER: OnceLock<Option<std::thread::Thread>> = OnceLock::new();

/// A registered job; dropping it takes the job out (a store closed).
pub struct Registered(u64);

impl Drop for Registered {
    fn drop(&mut self) {
        JOBS.lock().unwrap_or_else(|p| p.into_inner()).retain(|e| e.id != self.0);
    }
}

/// Adds a job. It runs once the scheduler is started ([`start`], by a process that lives long enough: the
/// daemon, a run's statistics writer) or by [`run_now`].
pub fn register(job: Box<dyn Job>) -> Registered {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let name = job.name();
    JOBS.lock().unwrap_or_else(|p| p.into_inner()).push(Entry {
        id,
        job,
        report: JobReport { name, ..Default::default() },
        due_since: None,
        held_until: None,
        last_ran: None,
    });
    kick();
    Registered(id)
}

/// Starts the scheduler's thread, once (when the settings have it on).
pub fn start() {
    SCHEDULER.get_or_init(|| {
        if !settings().on {
            return None;
        }
        crate::exec::service("rerail-upkeep", None, scheduler).ok().map(|h| h.thread().clone())
    });
}

/// Asks the scheduler to poll now (a job that has just become due).
pub fn kick() {
    if let Some(Some(t)) = SCHEDULER.get() {
        t.unpark();
    }
}

/// Every job's record.
pub fn report() -> Vec<JobReport> {
    let jobs = JOBS.lock().unwrap_or_else(|p| p.into_inner());
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
    while !STOP.load(Ordering::Acquire) {
        let s = settings();
        let c = read_conditions(&mut ticks);
        idle_polls = if c.busy < s.idle_busy { idle_polls + 1 } else { 0 };
        let (_, idle) = allowed(&c, idle_polls >= 2, &s);
        let ran = run_due(&s, idle, &mut ticks, false);
        if !ran {
            std::thread::park_timeout(if c.on_battery { s.poll * 4 } else { s.poll });
        }
    }
}

/// One pass over the jobs: each one due and allowed runs its steps until done, until the machine is busy again
/// (idle work), or for at most a poll's time. Whether any step ran. With `all`, every due job runs to the end.
fn run_due(s: &Settings, idle: bool, ticks: &mut (u64, u64), all: bool) -> bool {
    let now = Instant::now();
    // Which jobs to run, oldest run first; the lock is not held while a job steps.
    let mut order: Vec<(Option<Instant>, u64, bool)> = Vec::new();
    {
        let mut jobs = JOBS.lock().unwrap_or_else(|p| p.into_inner());
        for e in jobs.iter_mut() {
            if !s.runs(&e.report.name) || e.held_until.is_some_and(|h| h > now) {
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
            let urgent = match due {
                Due::Urgent => true,
                Due::Idle { starve } => now.duration_since(since) >= starve,
                Due::No => false,
            };
            if all || urgent || idle {
                order.push((e.last_ran, e.id, urgent));
            }
        }
    }
    order.sort_by_key(|&(last, id, _)| (last, id));
    let mut ran = false;
    let pass_end = now + s.poll;
    for (_, id, urgent) in order {
        loop {
            if STOP.load(Ordering::Acquire) || (!all && Instant::now() >= pass_end) {
                return ran;
            }
            let Some(step) = step_job(id, s) else { break };
            ran = true;
            if step != Step::More {
                break;
            }
            // Idle work stops once the machine is busy again.
            if !all && !urgent && read_conditions(ticks).busy >= s.idle_busy {
                return ran;
            }
        }
    }
    ran
}

/// One step of job `id` (taken out of the list while it runs), then the CPU share's pause. `None`: the job is
/// gone, or failed.
fn step_job(id: u64, s: &Settings) -> Option<Step> {
    let mut e = {
        let mut jobs = JOBS.lock().unwrap_or_else(|p| p.into_inner());
        let i = jobs.iter().position(|e| e.id == id)?;
        jobs.swap_remove(i)
    };
    let threads = e.job.threads().max(1) as f64;
    let (c0, t0) = (crate::score::thread_cpu(), Instant::now());
    let slice = Slice { until: t0 + s.step, memory: s.memory, stop: &STOP };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.job.step(&slice))).unwrap_or_else(|_| Err("panicked".into()));
    let (wall, cpu) = (t0.elapsed().as_secs_f64(), crate::score::thread_cpu() - c0);
    e.report.steps += 1;
    e.report.wall += wall;
    e.report.cpu += cpu;
    e.last_ran = Some(Instant::now());
    let out = match r {
        Ok(step) => {
            e.report.failures = 0;
            if step == Step::Done {
                e.due_since = None;
            }
            Some(step)
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
        eprintln!("rerail upkeep: {} step {:.1} ms wall {:.1} ms CPU: {:?}", e.report.name, wall * 1e3, cpu * 1e3, out.map_or_else(|| e.report.last_error.clone().unwrap_or_default(), |o| format!("{o:?}")));
    }
    JOBS.lock().unwrap_or_else(|p| p.into_inner()).push(e);
    // The pause that keeps `threads` busy for `wall` within `cpu` of a core on average.
    let pause = wall * (threads / s.cpu - 1.0);
    if pause > 0.0 && !STOP.load(Ordering::Acquire) {
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
        if !run_due(&s, true, &mut ticks, true) {
            break;
        }
    }
    report()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter {
        name: &'static str,
        left: u32,
        due: Due,
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
    }

    #[test]
    fn settings_parse_and_name_kinds() {
        let s = Settings { only: Some(vec!["sqlite".into()]), without: vec!["collect".into()], ..Default::default() };
        assert!(s.runs("sqlite content-v4.sqlite"));
        assert!(!s.runs("collect content-v4.sqlite"));
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
        assert!(c.footprint > 0, "{c:?}");
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
    struct Probe(std::sync::Arc<Mutex<Vec<(String, Option<&'static str>)>>>);

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

    #[test]
    fn jobs_step_to_the_end_within_the_cpu_share() {
        let _a = register(Box::new(Counter { name: "test-a one", left: 3, due: Due::Idle { starve: Duration::from_secs(3600) } }));
        let _b = register(Box::new(Counter { name: "test-b one", left: 2, due: Due::Urgent }));
        configure(Settings { only: Some(vec!["test-a".into(), "test-b".into()]), ..Settings::default() });
        let mut ticks = cpu_ticks();
        // Not idle: only the urgent job runs.
        run_due(&settings(), false, &mut ticks, false);
        run_due(&settings(), false, &mut ticks, false);
        let steps = |n: &str| report().into_iter().find(|r| r.name == n).map(|r| r.steps);
        assert_eq!((steps("test-a one"), steps("test-b one")), (Some(0), Some(2)));
        let r = run_now();
        assert_eq!(r.iter().find(|r| r.name == "test-a one").map(|r| r.steps), Some(3));
        configure(Settings::default());
    }
}
