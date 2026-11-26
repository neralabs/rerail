//! Relations sorted by a key on the device, and the equi-joins that probe them: moved into the executor's GPU lane
//! (`rerail_accel_gpu::gpu::join`, decision 0100 §1). Their launches are charged to the evaluation's launch budget as
//! every device submission's are (`rerail_accel::gpu::launch`).

use super::Error;

pub use rerail_accel_gpu::gpu::join::Held;
/// The relational lab's (`setfix::lab`) and the tests' names for the device's joins.
#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_accel_gpu::gpu::join::{Column, Keyed, probe};

/// Whether the process has a device for relational work (decision 0100 §2, its one queue shared with the
/// executor's GPU lane).
pub(super) fn ready() -> Result<(), Error> {
    rerail_accel_gpu::gpu::join::device().map(|_| ()).map_err(|_| Error::UnsupportedBackend)
}
