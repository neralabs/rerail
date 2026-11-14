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
