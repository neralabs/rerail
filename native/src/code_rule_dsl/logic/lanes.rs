//! The engine's compute lanes (decision 0098 §2): one executor per binding, shared by every stratum of the process.
//!
//! - Under the **CPU binding**, the CPU's lanes: the cores and, where the CPU has one, its matrix unit (SME2). The
//!   CPU is a target in its own right (0095 §7), and SME is part of it. SQL forms are also available.
//! - Under the **Metal binding**, every lane this machine has: the cores, SME, the GPU and the Neural Engine. Each
//!   launch goes where `rerail_accel::price` places it: the least energy among the lanes within the parity band of
//!   the first to finish.
//!
//! The SQL binding accepts only SQL forms. Connection-bound SQL keeps a scored owner-thread gate.
//!
//! Which native binding a thread's run has is the placement planner's (`plan::placement::set_device`).
//!
//! CPU and SQL jobs share one pool of `RERAIL_THREADS` threads (the available cores when unset). Each binding keeps a
//! scoreboard of its jobs (`rerail_accel::score`): `RERAIL_SCOREBOARD=<file>` traces every job, and the strata loop
//! appends the scoreboard at each chunk's end.

use rerail_accel::Executor;
use rerail_accel::exec::{Backend, CpuBackend};
use rerail_accel::sql::SqlBackend;
use std::sync::{Arc, LazyLock};

/// Whether each executor was made: the profile reports only those.
static MADE: [std::sync::atomic::AtomicBool; 3] = [const { std::sync::atomic::AtomicBool::new(false) }; 3];

static CPU: LazyLock<Executor> = LazyLock::new(|| {
    MADE[0].store(true, std::sync::atomic::Ordering::Relaxed);
    super::costs::start();
    let all = Executor::new();
    let mut lanes: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend::default()), Arc::new(SqlBackend)];
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if all.lanes().contains(&rerail_accel::Lane::Sme) {
        lanes.extend(rerail_accel::sme::backend());
    }
    drop(all);
    Executor::with(lanes)
});
static ALL: LazyLock<Executor> = LazyLock::new(|| {
    MADE[1].store(true, std::sync::atomic::Ordering::Relaxed);
    super::costs::start();
    // Every submission the executor makes is checked against the engine's launch budget (decision 0100 §2).
    #[cfg(target_os = "macos")]
    crate::code_rule_dsl::transducer::packed::metal_sort::install_guard();
    Executor::new()
});

static SQL: LazyLock<Executor> = LazyLock::new(|| {
    MADE[2].store(true, std::sync::atomic::Ordering::Relaxed);
    Executor::sql_only()
});

/// Every rule job submitted in SQL mode must have a SQL form.
pub(in crate::code_rule_dsl) fn sql_executor() -> &'static Executor { &SQL }

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
    if MADE[2].load(std::sync::atomic::Ordering::Relaxed) {
        out.push(("sql binding", SQL.score()));
    }
    out
}
