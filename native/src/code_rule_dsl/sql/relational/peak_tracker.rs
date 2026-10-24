//! SQL-supplied numeric utility peaks. No matcher, AST, allowance or SQL execution.
use rusqlite::{Connection, Error, Result, functions::FunctionFlags};
use std::sync::{Arc, Mutex};
#[derive(Default)]
pub(super) struct Tracker {
    next: i64,
    frames: Vec<Frame>,
}
struct Frame {
    token: i64,
    incoming: i64,
    peak: i64,
}
impl Tracker {
    pub(super) fn reset(&mut self) {
        self.frames.clear();
    }
}
fn invalid(s: &str) -> Error {
    Error::ModuleError(format!("utility peak: {s}"))
}
fn level(level: i64) -> Result<i64> {
    if (0..=256).contains(&level) {
        Ok(level)
    } else {
        Err(invalid("level outside 0..256"))
    }
}
pub(super) fn register(db: &Connection) -> Result<Arc<Mutex<Tracker>>> {
    let state = Arc::new(Mutex::new(Tracker::default()));
    let open = state.clone();
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY;
    db.create_scalar_function("relation_peak_open", 1, flags, move |ctx| {
        let incoming = level(ctx.get(0)?)?;
        let mut state = open.lock().unwrap();
        if state
            .frames
            .last()
            .is_some_and(|frame| incoming < frame.incoming)
        {
            return Err(invalid("nested incoming level decreased"));
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("token overflow"))?;
        let token = state.next;
        state.frames.push(Frame {
            token,
            incoming,
            peak: incoming,
        });
        Ok(token)
    })?;
    let note = state.clone();
    db.create_scalar_function("relation_peak_note", 1, flags, move |ctx| {
        let reached = level(ctx.get(0)?)?;
        let mut state = note.lock().unwrap();
        if let Some(frame) = state.frames.last_mut() {
            if reached < frame.incoming {
                return Err(invalid("reached level below incoming"));
            }
            frame.peak = frame.peak.max(reached);
        }
        Ok(1_i64)
    })?;
    let close = state.clone();
    db.create_scalar_function("relation_peak_close", 1, flags, move |ctx| {
        let token = ctx.get::<i64>(0)?;
        let mut state = close.lock().unwrap();
        if state.frames.last().is_none_or(|frame| frame.token != token) {
            return Err(invalid("close is not the active frame"));
        }
        let frame = state.frames.pop().unwrap();
        if let Some(parent) = state.frames.last_mut() {
            parent.peak = parent.peak.max(frame.peak);
        }
        Ok(frame.peak - frame.incoming)
    })?;
    Ok(state)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_misses_and_replayed_peaks_merge_before_work_failure() {
        let db = Connection::open_in_memory().unwrap();
        register(&db).unwrap();
        let outer: i64 = db
            .query_row("SELECT relation_peak_open(10)", [], |r| r.get(0))
            .unwrap();
        db.query_row::<i64, _, _>("SELECT relation_peak_note(11)", [], |r| r.get(0))
            .unwrap();
        let inner: i64 = db
            .query_row("SELECT relation_peak_open(11)", [], |r| r.get(0))
            .unwrap();
        db.query_row::<i64, _, _>("SELECT relation_peak_note(17)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [inner], |r| r.get(0))
                .unwrap(),
            6
        );
        db.query_row::<i64, _, _>("SELECT relation_peak_note(12)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [outer], |r| r.get(0))
                .unwrap(),
            7
        );
    }
    #[test]
    fn guards_lifo_validation_and_reset_leave_no_partial_frame() {
        let db = Connection::open_in_memory().unwrap();
        let tracker = register(&db).unwrap();
        let n: i64 = db
            .query_row(
                "SELECT count(*) FROM (SELECT relation_peak_open(0) WHERE 0)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        let a: i64 = db
            .query_row("SELECT relation_peak_open(0)", [], |r| r.get(0))
            .unwrap();
        let b: i64 = db
            .query_row("SELECT relation_peak_open(1)", [], |r| r.get(0))
            .unwrap();
        assert!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [a], |r| r.get(0))
                .is_err()
        );
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [b], |r| r.get(0))
                .unwrap(),
            0
        );
        tracker.lock().unwrap().reset();
        assert!(
            db.query_row::<i64, _, _>("SELECT relation_peak_close(?1)", [a], |r| r.get(0))
                .is_err()
        );
        for level in [-1, 257] {
            assert!(
                db.query_row::<i64, _, _>("SELECT relation_peak_note(?1)", [level], |r| r.get(0))
                    .is_err()
            );
        }
    }
}
