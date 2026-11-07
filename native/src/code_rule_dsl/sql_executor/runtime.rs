//! Per-operation limits. The handler is removed before rollback or connection reuse.
use super::*;
use std::time::{Duration, Instant};

#[derive(Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Limits {
    pub max_steps: Option<u64>,
    pub timeout_ms: Option<u64>,
}

pub(super) struct Budget<'a>(&'a rusqlite::Connection, bool);
impl Limits {
    pub fn enter<'a>(&self, db: &'a rusqlite::Connection) -> Result<Budget<'a>> {
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
                    work = work.saturating_add(interval as u64);
                    max.is_some_and(|n| work >= n) || timeout.is_some_and(|d| start.elapsed() >= d)
                }),
            )
            .map_err(error)?;
        }
        Ok(Budget(db, active))
    }
}
impl Drop for Budget<'_> {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.progress_handler(0, None::<fn() -> bool>);
        }
    }
}
