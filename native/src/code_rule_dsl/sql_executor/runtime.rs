//! Per-operation limits. The handler is removed before rollback or connection reuse.
use super::*;
use std::time::{Duration, Instant};
thread_local! {static LIMITED:std::cell::Cell<bool>=const {std::cell::Cell::new(false)};}
pub fn limited()->bool {LIMITED.get()}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_steps: Option<u64>,
    /// One allowance for the complete session, including owner work and all SQL workers.
    pub max_submissions: Option<usize>,
    pub timeout_ms: Option<u64>,
}

/// A budget `Limits::enter` set on a connection. It stays there until `leave`, which callers run
/// before rollback or connection reuse.
#[must_use = "the budget's handler stays on the connection until it leaves"]
pub struct Budget(bool,bool);
impl Limits {
    pub fn enter(&self, db: &mut rusqlite::Connection) -> Result<Budget> {
        let max = self.max_steps;
        let timeout = self.timeout_ms.map(Duration::from_millis);
        let active = max.is_some() || timeout.is_some();
        if active {
            let start = Instant::now();
            let mut work = 0_u64;
            // Count short statements too. A coarse callback loses their partial intervals at
            // every VDBE reset and can leave a run of many small waves effectively unbounded.
            let interval = if max.is_some() { 1 } else { 1000 };
            db.progress_handler(
                interval,
                Some(move || {
                    if super::super::sql_profile::explaining() { return false; }
                    work = work.saturating_add(interval as u64);
                    max.is_some_and(|n| work >= n) || timeout.is_some_and(|d| start.elapsed() >= d)
                }),
            )
            .map_err(error)?;
        }
        let previous=LIMITED.replace(LIMITED.get()||active);
        Ok(Budget(active,previous))
    }
}
impl Budget {
    /// Removes the handler `enter` set.
    pub fn leave(self, db: &mut rusqlite::Connection) {
        LIMITED.set(self.1);
        if self.0 {
            let _ = db.progress_handler(0, None::<fn() -> bool>);
        }
    }
}
