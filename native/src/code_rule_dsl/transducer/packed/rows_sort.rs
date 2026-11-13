//! Rows ordered by a key column on the host: moved into the executor (`rerail_accel::rows`, decision 0100 §1).

#[cfg(any(test, feature = "test-hooks"))]
pub use rerail_accel::rows::*;
