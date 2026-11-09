//! The gpu lane (decision 0096). Not built yet: no backend, so the executor never places work here.

use crate::exec::Backend;
use std::sync::Arc;

/// This lane's backend, when it can run here.
pub fn backend() -> Option<Arc<dyn Backend>> {
    None
}
