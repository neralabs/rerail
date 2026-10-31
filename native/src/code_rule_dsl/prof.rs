//! Wall time by stage, for profiling the wave (O28): with `RERAIL_PROF` set, `time` adds each
//! call's wall time to its stage and `report` prints and clears them; without it, `time` only
//! calls. Stages may nest and run on several threads, so their times overlap and need not sum.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_PROF").is_some());
static STAGES: Mutex<BTreeMap<String, (Duration, usize)>> = Mutex::new(BTreeMap::new());

pub(in crate::code_rule_dsl) fn time<T>(stage: &'static str, f: impl FnOnce() -> T) -> T {
    if !*ON {
        return f();
    }
    let start = Instant::now();
    let out = f();
    let elapsed = start.elapsed();
    let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = stages.entry(stage.to_owned()).or_default();
    entry.0 += elapsed;
    entry.1 += 1;
    out
}

/// Prints the stages' times under `title` and clears them.
#[cfg_attr(not(test), allow(dead_code))]
pub(in crate::code_rule_dsl) fn report(title: &str) {
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
pub(in crate::code_rule_dsl) struct Stage(Option<(&'static str, Instant)>);

impl Stage {
    pub(in crate::code_rule_dsl) fn start(stage: &'static str) -> Self {
        Self(ON.then(|| (stage, Instant::now())))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some((stage, start)) = self.0.take() {
            let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = stages.entry(stage.to_owned()).or_default();
            entry.0 += start.elapsed();
            entry.1 += 1;
        }
    }
}

/// Adds `amount` (bytes, records) to `stage`'s count, shown as its calls.
pub(in crate::code_rule_dsl) fn count(stage: &'static str, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage.to_owned()).or_default().1 += amount;
    }
}

/// Whether stages are timed (`RERAIL_PROF`).
pub(in crate::code_rule_dsl) fn on() -> bool {
    *ON
}

/// `count` with a name made at run time.
pub(in crate::code_rule_dsl) fn count_named(stage: String, amount: usize) {
    if *ON {
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        stages.entry(stage).or_default().1 += amount;
    }
}

/// Records the process's peak resident memory so far, in MB, as `stage`'s count (the largest seen).
pub(in crate::code_rule_dsl) fn peak(stage: &str) {
    if *ON {
        let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        // macOS reports bytes; Linux, kilobytes.
        let mb = if cfg!(target_os = "macos") { usage.ru_maxrss as usize >> 20 } else { usage.ru_maxrss as usize >> 10 };
        let mut stages = STAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = stages.entry(format!("peak_mb.{stage}")).or_default();
        entry.1 = entry.1.max(mb);
    }
}
