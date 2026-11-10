//! The engine's compute lanes (decision 0098 §2): one executor per binding, shared by every stratum of the process.
//!
//! - Under the **CPU binding**, the CPU's lanes: the cores and, where the CPU has one, its matrix unit (SME2). The
//!   CPU is a target in its own right (0095 §7), and SME is part of it.
//! - Under the **Metal binding**, every lane this machine has: the cores, SME, the GPU and the Neural Engine. Each
//!   launch goes where `rerail_accel::price` places it: the least energy among the lanes within the parity band of
//!   the first to finish.
//!
//! Which binding a thread's run has is the placement planner's (`plan::placement::set_device`).
//!
//! Both executors run the cores' jobs on one pool of `RERAIL_THREADS` threads (4 when unset), and each keeps a
//! scoreboard of its jobs (`rerail_accel::score`): `RERAIL_SCOREBOARD=<file>` traces every job, and the strata loop
//! appends the scoreboard at each chunk's end.

use rerail_accel::Executor;
use rerail_accel::exec::{Backend, CpuBackend};
use std::sync::{Arc, LazyLock};

/// Whether each executor was made: the profile reports only those.
static MADE: [std::sync::atomic::AtomicBool; 2] = [std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false)];

static CPU: LazyLock<Executor> = LazyLock::new(|| {
    MADE[0].store(true, std::sync::atomic::Ordering::Relaxed);
    let all = Executor::new();
    let mut lanes: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend::default())];
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if all.lanes().contains(&rerail_accel::Lane::Sme) {
        lanes.extend(rerail_accel::sme::backend());
    }
    drop(all);
    Executor::with(lanes)
});
static ALL: LazyLock<Executor> = LazyLock::new(|| {
    MADE[1].store(true, std::sync::atomic::Ordering::Relaxed);
    // Every submission the executor makes is checked against the engine's launch budget (decision 0100 §2).
    #[cfg(target_os = "macos")]
    crate::code_rule_dsl::transducer::packed::metal_sort::install_guard();
    Executor::new()
});

/// The executor for this thread's run.
pub(in crate::code_rule_dsl) fn executor() -> &'static Executor {
    if super::plan::placement::device() { &ALL } else { &CPU }
}

/// The executor of a binding: every lane under the device binding (`device`), the cores' (and SME's) otherwise. For
/// a job on another thread than the run's (whose binding is the run's thread's, [`executor`]).
pub(in crate::code_rule_dsl) fn executor_for(device: bool) -> &'static Executor {
    if device { &ALL } else { &CPU }
}

/// Each executor made so far, by binding, and its scoreboard: for the profile.
pub(in crate::code_rule_dsl) fn scores() -> Vec<(&'static str, rerail_accel::score::Score)> {
    let mut out = Vec::new();
    if MADE[0].load(std::sync::atomic::Ordering::Relaxed) {
        out.push(("cpu binding", CPU.score()));
    }
    if MADE[1].load(std::sync::atomic::Ordering::Relaxed) {
        out.push(("metal binding", ALL.score()));
    }
    out
}
