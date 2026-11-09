//! The engine's compute lanes (decision 0096 §2): one executor per binding, shared by every stratum of the process.
//!
//! - Under the **CPU binding**, the CPU's lanes: the cores and, where the CPU has one, its matrix unit (SME2). The
//!   CPU is a target in its own right (0095 §7), and SME is part of it.
//! - Under the **Metal binding**, every lane this machine has: the cores, SME, the GPU and the Neural Engine. Each
//!   launch goes where `rerail_accel::price` places it: the least energy among the lanes within the parity band of
//!   the first to finish.
//!
//! Which binding a thread's run has is the placement planner's (`plan::placement::set_device`).

use rerail_accel::Executor;
use rerail_accel::exec::{Backend, CpuBackend};
use std::sync::{Arc, LazyLock};

static CPU: LazyLock<Executor> = LazyLock::new(|| {
    let all = Executor::new();
    let mut lanes: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend)];
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if all.lanes().contains(&rerail_accel::Lane::Sme) {
        lanes.extend(rerail_accel::sme::backend());
    }
    drop(all);
    Executor::with(lanes)
});
static ALL: LazyLock<Executor> = LazyLock::new(Executor::new);

/// The executor for this thread's run.
pub(in crate::code_rule_dsl) fn executor() -> &'static Executor {
    if super::plan::placement::device() { &ALL } else { &CPU }
}
