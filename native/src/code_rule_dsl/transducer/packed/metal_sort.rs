//! Relations sorted by a key on the device, and the equi-joins that probe them: moved into the executor's GPU lane
//! (`rerail_accel::gpu::join`, decision 0100 §1). What stays here is the engine's side of the one Metal manager:
//! its launch budget as the check of every relational submission.

use super::Error;
use super::metal_backend::launch::LaunchScope;
use metal::CommandQueue;

pub use rerail_accel::gpu::join::Held;
/// The relational lab's (`setfix::lab`) and the tests' names for the device's joins.
#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_accel::gpu::join::{Column, Keyed, probe};

/// The one queue of relational work (decision 0100 §2), shared with the executor's GPU lane.
pub(super) fn queue() -> Result<CommandQueue, Error> {
    install_guard();
    rerail_accel::gpu::join::queue().map_err(|_| Error::UnsupportedBackend)
}

/// Makes the engine's launch budget the check of every submission the executor makes (once per process).
pub(in crate::code_rule_dsl) fn install_guard() {
    rerail_accel::gpu::shared::set_guard(|| {
        let _scope = LaunchScope::enter();
        super::metal_backend::launch::check_submission().map_err(|e| format!("{e:?}"))
    });
}
