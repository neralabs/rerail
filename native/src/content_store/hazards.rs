//! The profile's SQL hazards (decision 0104: no wave waits for SQL), as the engine reports them: the store's
//! connections trace their statements and its writer its waits here, and the engine, whose profile counts them, sets
//! where they go as it loads ([`set`]). Unset (the store alone, its own tests), each is nothing.

/// Where the store's hazards go: whether the profile is on, a statement run (its connection, text and time), a wait
/// (what, wall seconds), and a write queued behind the run.
#[derive(Clone, Copy)]
pub struct Hazards {
    pub on: fn() -> bool,
    pub sql_in_wave: fn(&'static str, &str, u64),
    pub wait_in_wave: fn(&str, f64),
    pub behind_the_run: fn(),
    /// A statement about to run (its connection and text): the strict mode's check (`RERAIL_SQL_STRICT`).
    pub statement: fn(&'static str, &str),
    /// A span of the profile, open until the value is dropped: the writer's job, by what it writes.
    pub span: fn(&'static str) -> Box<dyn std::any::Any>,
}

static HAZARDS: std::sync::OnceLock<Hazards> = std::sync::OnceLock::new();

/// Sets where the store's hazards go; the first setting stays.
pub fn set(hazards: Hazards) {
    let _ = HAZARDS.set(hazards);
}

pub(crate) fn on() -> bool {
    HAZARDS.get().is_some_and(|h| (h.on)())
}

pub(crate) fn sql_in_wave(conn: &'static str, text: &str, ns: u64) {
    if let Some(h) = HAZARDS.get() {
        (h.sql_in_wave)(conn, text, ns);
    }
}

pub(crate) fn statement(conn: &'static str, text: &str) {
    if let Some(h) = HAZARDS.get() {
        (h.statement)(conn, text);
    }
}

pub(crate) fn wait_in_wave(what: &str, wall: f64) {
    if let Some(h) = HAZARDS.get() {
        (h.wait_in_wave)(what, wall);
    }
}

/// The profile's span for `what`, open until dropped (`None`: unset).
pub(crate) fn span(what: &'static str) -> Option<Box<dyn std::any::Any>> {
    HAZARDS.get().map(|h| (h.span)(what))
}

pub(crate) fn behind_the_run() {
    if let Some(h) = HAZARDS.get() {
        (h.behind_the_run)();
    }
}
