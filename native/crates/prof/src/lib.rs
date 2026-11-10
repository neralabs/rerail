//! Wall time by stage, for profiling (O28): with `RERAIL_PROF` set, `time` adds each call's wall
//! time to its stage and `report` prints and clears them; without it, `time` only calls. Stages
//! may nest and run on several threads, so their times overlap and need not sum. A crate of its
//! own so that the engine's crates (the Metal runtime's) count into the same stages.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_PROF").is_some());
static STAGES: Mutex<BTreeMap<String, (Duration, usize)>> = Mutex::new(BTreeMap::new());

pub fn time<T>(stage: &str, f: impl FnOnce() -> T) -> T {
    if !*ON {
        return f();
    }
    let start = Instant::now();
    // A top-level stage's process CPU (user + sys) as well: its own where nothing overlaps it.
    let top = stage.starts_with("slice.") || stage.starts_with("demand.") || stage.starts_with("labels.");
    let cpu = top.then(process_cpu);
    let out = f();
    let elapsed = start.elapsed();
    let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cpu) = cpu {
        let entry = stages.entry(format!("cpu_ms.{stage}")).or_default();
        entry.1 += ((process_cpu() - cpu).as_secs_f64() * 1e3) as usize;
    }
    let entry = stages.entry(stage.to_owned()).or_default();
    entry.0 += elapsed;
    entry.1 += 1;
    out
}

/// CPU time on wasm32 (the browser build): the engine runs on one thread of one Web Worker, which holds no
/// CPU clock, so the time since the first read stands in for user + sys CPU.
#[cfg(target_family = "wasm")]
pub fn single_thread_cpu() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// The process's CPU time so far (user + sys).
#[cfg(target_family = "wasm")]
fn process_cpu() -> Duration {
    single_thread_cpu()
}

/// The process's CPU time so far (user + sys).
#[cfg(not(target_family = "wasm"))]
fn process_cpu() -> Duration {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let time = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// Prints the stages' times under `title` and clears them.

pub fn report(title: &str) {
    if !*ON {
        return;
    }
    let stages = std::mem::take(&mut *STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    eprintln!("prof {title}:");
    for (stage, (elapsed, calls)) in stages {
        eprintln!("  {stage:<40} {:>10.1} ms {calls:>7} calls", elapsed.as_secs_f64() * 1e3);
    }
}

/// A stage timed until it drops.
pub struct Stage(Option<(&'static str, Instant, Option<Duration>)>);

impl Stage {
    pub fn start(stage: &'static str) -> Self {
        Self(ON.then(|| (stage, Instant::now(), (stage.starts_with("labels.") || stage.starts_with("demand.") || stage == "size.texts").then(process_cpu))))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some((stage, start, cpu)) = self.0.take() {
            let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cpu) = cpu {
                stages.entry(format!("cpu_ms.{stage}")).or_default().1 += ((process_cpu() - cpu).as_secs_f64() * 1e3) as usize;
            }
            let entry = stages.entry(stage.to_owned()).or_default();
            entry.0 += start.elapsed();
            entry.1 += 1;
        }
    }
}

/// Adds `amount` (bytes, records) to `stage`'s count, shown as its calls.
pub fn count(stage: &'static str, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage.to_owned()).or_default().1 += amount;
    }
}

/// Whether stages are timed (`RERAIL_PROF`).
pub fn on() -> bool {
    *ON
}

/// `count` with a name made at run time.
pub fn count_named(stage: String, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage).or_default().1 += amount;
    }
}

/// The stages, locked: for a reading of its own (the engine's memory peaks), kept as the
/// largest seen.
pub fn stages() -> std::sync::MutexGuard<'static, BTreeMap<String, (Duration, usize)>> {
    STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
