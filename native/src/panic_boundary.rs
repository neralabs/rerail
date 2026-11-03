//! Where a Rust panic stops: at the Node-API boundary, as an error, rather than in the process.
//!
//! The engine runs inside the daemon, which every coding-agent session on the machine shares. A
//! panic that unwinds out of a Node-API callback aborts that process (Rust aborts on unwinding out
//! of an `extern "C"` function), and every rule in every session stops with it. Synchronous
//! exports take `#[napi(catch_unwind)]`; an `AsyncTask` runs `compute` inside napi-rs's own
//! `extern "C"` worker callback, which the attribute does not cover, so each `compute` runs its
//! work through [`contain_panic`].
//!
//! A caught panic does not prove the workspace consistent. Workspace state sits behind
//! `std::sync::Mutex`, which a panic poisons; the call that meets the poisoned lock closes the
//! workspace (`close_poisoned`), so it and every later call fail as on a closed workspace, which
//! no host retries, and the session's checks fail rather than run on it. The CPU slot applies
//! share guards no data and does not poison, so a panic in one apply never blocks the next.

use napi::{Error, Result, Status};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `work`, returning its result, or a `GenericFailure` naming `what` and the panic message
/// when it panics.
pub(crate) fn contain_panic<T>(what: &str, work: impl FnOnce() -> Result<T>) -> Result<T> {
    // Unwind-safe in effect: what a panic could leave half-updated is behind a mutex it poisons.
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic with no message".to_owned());
        Err(Error::new(
            Status::GenericFailure,
            format!("{what} panicked: {message}"),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::contain_panic;

    #[test]
    fn a_panic_becomes_an_error_naming_the_work_and_the_message() {
        let error = contain_panic::<()>("workspace apply", || panic!("index out of range"))
            .expect_err("a panic is an error");
        assert_eq!(error.reason, "workspace apply panicked: index out of range");
        let formatted = contain_panic::<()>("workspace apply", || panic!("{} of {}", 3, 2))
            .expect_err("a formatted panic is an error");
        assert_eq!(formatted.reason, "workspace apply panicked: 3 of 2");
    }
}
