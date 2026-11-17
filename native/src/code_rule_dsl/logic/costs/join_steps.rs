//! The join steps' estimates and what they made (decision 0354): the data of the learned correction of the planner's
//! estimates (`rerail_accel::join_model`), kept in the cost store beside the executor's job samples (`samples`).
//!
//! The driver submits each line's steps at the end of its component (`submit`); the cost statistics' writer takes
//! them with its samples and writes them in its transaction (`take`, `insert`). The queue holds at most
//! [`QUEUE_LIMIT`] steps: past it a line's are dropped, measurements and not results.

use parking_lot::Mutex;
use rusqlite::Connection;

pub use rerail_accel::join_model::StepSample;

/// The steps queued for the writer at most.
const QUEUE_LIMIT: usize = 100_000;
/// The steps kept per machine: older ones are deleted past this (by an eighth, then back to it).
const KEPT: i64 = 400_000;
/// The recent steps a comparison reads.
pub const READ: i64 = 200_000;

static QUEUE: Mutex<Vec<StepSample>> = Mutex::new(Vec::new());

/// Whether steps are kept: the cost statistics have a store (not `RERAIL_COSTS=off`, not a test).
pub fn enabled() -> bool {
    super::file().is_some()
}

/// Queues a component's steps for the writer.
pub fn submit(steps: Vec<StepSample>) {
    if steps.is_empty() {
        return;
    }
    let mut q = QUEUE.lock();
    let room = QUEUE_LIMIT.saturating_sub(q.len());
    q.extend(steps.into_iter().take(room));
}

/// The steps queued since the last write.
pub fn take() -> Vec<StepSample> {
    std::mem::take(&mut *QUEUE.lock())
}

pub fn create(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS join_steps(
           at REAL, machine TEXT, basis TEXT, access TEXT, source TEXT, bound INTEGER, step INTEGER, steps INTEGER,
           est_in REAL, est_fan REAL, card REAL, solving INTEGER, seed INTEGER, actual_in REAL, actual_out REAL);
         CREATE INDEX IF NOT EXISTS join_steps_by_machine ON join_steps(machine, at);",
    )
}

pub fn insert(tx: &rusqlite::Transaction<'_>, machine: &str, at: f64, steps: &[StepSample]) -> rusqlite::Result<()> {
    if steps.is_empty() {
        return Ok(());
    }
    let mut ins = tx.prepare_cached("INSERT INTO join_steps VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)")?;
    for s in steps {
        ins.execute(rusqlite::params![at, machine, s.basis, s.access, s.source, s.bound, s.step, s.steps, s.est_in, s.est_fan, s.card, s.solving, s.seed, s.actual_in, s.actual_out])?;
    }
    Ok(())
}

/// Past `KEPT` steps of `machine` by an eighth, the oldest go, back to `KEPT`.
pub fn trim(db: &Connection, machine: &str) {
    let over = db.query_row("SELECT 1 FROM join_steps WHERE machine = ?1 ORDER BY at DESC LIMIT 1 OFFSET ?2", rusqlite::params![machine, KEPT + KEPT / 8], |_| Ok(())).is_ok();
    if over && let Ok(cut) = db.query_row("SELECT at FROM join_steps WHERE machine = ?1 ORDER BY at DESC LIMIT 1 OFFSET ?2", rusqlite::params![machine, KEPT], |r| r.get::<_, f64>(0)) {
        let _ = db.execute("DELETE FROM join_steps WHERE machine = ?1 AND at < ?2", rusqlite::params![machine, cut]);
    }
}

/// `machine`'s most recent steps, at most `limit`, oldest first.
pub fn read(db: &Connection, machine: &str, limit: i64) -> rusqlite::Result<Vec<StepSample>> {
    let mut st = db.prepare(
        "SELECT basis, access, source, bound, step, steps, est_in, est_fan, card, solving, seed, actual_in, actual_out FROM
           (SELECT rowid, * FROM join_steps WHERE machine = ?1 ORDER BY at DESC, rowid DESC LIMIT ?2) ORDER BY at, rowid",
    )?;
    let rows = st.query_map(rusqlite::params![machine, limit], |r| {
        Ok(StepSample {
            basis: r.get(0)?,
            access: r.get(1)?,
            source: r.get(2)?,
            bound: r.get(3)?,
            step: r.get(4)?,
            steps: r.get(5)?,
            est_in: r.get(6)?,
            est_fan: r.get(7)?,
            card: r.get(8)?,
            solving: r.get(9)?,
            seed: r.get(10)?,
            actual_in: r.get(11)?,
            actual_out: r.get(12)?,
        })
    })?;
    rows.collect()
}

/// A step's record's first words, as a category: "postings" of "postings, containment 0.2".
pub fn category(text: &str, words: usize) -> String {
    let cut = text.find(|c: char| c == '(' || c == ',' || c == ';' || c == ':' || c.is_ascii_digit()).unwrap_or(text.len());
    let s: Vec<&str> = text[..cut].split_whitespace().take(words).collect();
    if s.is_empty() { "none".to_owned() } else { s.join(" ") }
}

/// The columns a step's access names: "lookup on (c0, c2)" two, "scan" none.
pub fn bound_columns(access: &str) -> u32 {
    match (access.find('('), access.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 => access[a + 1..b].split(',').count() as u32,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_steps_record_reads_as_categories() {
        assert_eq!(super::category("postings", 3), "postings");
        assert_eq!(super::category("scan of the component", 4), "scan of the component");
        assert_eq!(super::category("lookup on (c0, c2)", 1), "lookup");
        assert_eq!(super::category("", 3), "none");
        assert_eq!(super::bound_columns("lookup on (c0, c2)"), 2);
        assert_eq!(super::bound_columns("scan"), 0);
        assert_eq!(super::bound_columns("anti-join on ()"), 0);
    }
}
