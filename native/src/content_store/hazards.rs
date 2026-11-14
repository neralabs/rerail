//! The profile's SQL hazards (decision 0104: no wave waits for SQL): the store's connections trace their statements and
//! its writer its waits in the profile (`rerail_prof`'s, below this crate), which counts them. With the profile off
//! (the store alone, its own tests), each is nothing.

use rerail_prof::code_rule_dsl::logic::profile;

pub(crate) fn on() -> bool {
    profile::tracing()
}

pub(crate) fn sql_in_wave(conn: &'static str, text: &str, ns: u64) {
    profile::sql_in_wave(conn, text, ns);
}

pub(crate) fn statement(conn: &'static str, text: &str) {
    profile::strict_statement(conn, text);
}

pub(crate) fn inside_waves() -> bool {
    profile::inside_waves()
}

pub(crate) fn wait_in_wave(what: &str, wall: f64) {
    profile::wait_in_wave(what, wall);
}

/// The profile's span for `what` (the writer's CPU under the profile's spans, a job at a time), open until dropped.
pub(crate) fn span(what: &'static str) -> Option<Box<dyn std::any::Any>> {
    Some(Box::new(profile::span(move || format!("store writer: {what}"))))
}

pub(crate) fn behind_the_run() {
    profile::behind_the_run();
}

/// A recomputation in a warm run (`RERAIL_WARM`, `profile::warm_fault`): nothing without it.
pub(crate) fn warm_fault(kind: &'static str, detail: impl FnOnce() -> String) {
    profile::warm_fault(kind, detail);
}

/// A commit on `db` in a warm run (`profile::warm_commit`), for a connection whose commit hook is the store's own.
///
/// # Safety
/// As `profile::warm_commit`: `db` is open, and this runs inside its commit hook.
pub(crate) unsafe fn warm_commit(db: *mut rusqlite::ffi::sqlite3) {
    // SAFETY: the caller's guarantee.
    unsafe { profile::warm_commit(db) };
}

/// A transaction of the store's writer (`profile::warm_scope`): its commit is not a fault of the warm check if every
/// job it ran is one of the writes a warm run may make (`profile::WARM_WRITES_ALLOWED`).
pub(crate) fn warm_scope() -> profile::WarmScope {
    profile::warm_scope()
}

/// The writer runs the job `what` in its open `warm_scope`.
pub(crate) fn warm_job(what: &str) {
    profile::warm_job(what);
}

/// Whether a warm run may queue the job `what` (`profile::WARM_WRITES_ALLOWED`).
pub(crate) fn warm_write_allowed(what: &str) -> bool {
    profile::warm_write_allowed(what)
}
