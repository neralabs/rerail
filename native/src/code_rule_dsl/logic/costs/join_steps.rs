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
    )?;
    // Columns added after the table was first made: a store written before them gets them, empty.
    let has: Vec<String> = db.prepare("SELECT name FROM pragma_table_info('join_steps')")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for (col, ty) in [("literal", "TEXT"), ("key_ndv", "REAL"), ("origin", "TEXT"), ("run", "INTEGER"), ("relation", "TEXT"), ("bound_mask", "INTEGER"), ("cols", "INTEGER")] {
        if !has.iter().any(|h| h == col) {
            db.execute_batch(&format!("ALTER TABLE join_steps ADD COLUMN {col} {ty}"))?;
        }
    }
    Ok(())
}

pub fn insert(tx: &rusqlite::Transaction<'_>, machine: &str, at: f64, steps: &[StepSample]) -> rusqlite::Result<()> {
    if steps.is_empty() {
        return Ok(());
    }
    let mut ins = tx.prepare_cached(
        "INSERT INTO join_steps(at, machine, basis, access, source, bound, step, steps, est_in, est_fan, card, solving, seed,
           actual_in, actual_out, literal, key_ndv, origin, run, relation, bound_mask, cols)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
    )?;
    // The run: this process (a comparison holds out a run's steps whole).
    let run = std::process::id() as i64;
    for s in steps {
        ins.execute(rusqlite::params![at, machine, s.basis, s.access, s.source, s.bound, s.step, s.steps, s.est_in, s.est_fan, s.card, s.solving, s.seed, s.actual_in, s.actual_out, s.literal, s.key_ndv, s.origin, run, s.relation, s.bound_mask as i64, s.cols])?;
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

/// `machine`'s most recent steps, at most `limit`, oldest first, each with its run (0 where not kept).
pub fn read(db: &Connection, machine: &str, limit: i64) -> rusqlite::Result<Vec<(i64, StepSample)>> {
    let mut st = db.prepare(
        "SELECT basis, access, source, bound, step, steps, est_in, est_fan, card, solving, seed, actual_in, actual_out,
           coalesce(literal, ''), coalesce(key_ndv, 0), coalesce(origin, 'ran'), coalesce(run, 0),
           coalesce(relation, ''), coalesce(bound_mask, 0), coalesce(cols, 0) FROM
           (SELECT rowid, * FROM join_steps WHERE machine = ?1 ORDER BY at DESC, rowid DESC LIMIT ?2) ORDER BY at, rowid",
    )?;
    let rows = st.query_map(rusqlite::params![machine, limit], |r| {
        Ok((r.get(16)?, StepSample {
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
            literal: r.get(13)?,
            key_ndv: r.get(14)?,
            origin: r.get(15)?,
            relation: r.get(17)?,
            bound_mask: r.get::<_, i64>(18)? as u64,
            cols: r.get(19)?,
        }))
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

/// Decision 0354, the model's choice: with `RERAIL_JOIN_MODEL=1`, a join model trained on this machine's stored
/// steps prices every searched line's candidates (`joins::set_pricer`). Until it is trained, and without it, the
/// planner's estimates choose.
pub fn model_wanted() -> bool {
    std::env::var("RERAIL_JOIN_MODEL").is_ok_and(|v| v == "1")
}

/// A candidate order's price by the model: the planner's price, scaled by the rows the model predicts over the
/// rows the statistics estimate, each step's rows the step before's times its fanout, corrected by its predicted
/// target (`targets`, one a step). An order whose steps could not be read keeps its price.
pub fn model_price(est: f64, steps: &[StepSample], targets: &[f64]) -> Option<f64> {
    let first = steps.first()?;
    let (mut planned, mut predicted) = (first.est_in.max(0.0), first.est_in.max(0.0));
    let (mut sum_planned, mut sum_predicted) = (planned, predicted);
    for (s, &t) in steps.iter().zip(targets) {
        let fan = s.est_fan.max(0.0);
        planned *= fan;
        predicted *= fan * t.exp();
        sum_planned += planned;
        sum_predicted += predicted;
    }
    (sum_planned > 0.0 && sum_predicted.is_finite()).then(|| est * sum_predicted / sum_planned)
}

#[cfg(target_os = "macos")]
struct ModelPricer(rerail_accel::join_model::JoinModel);

#[cfg(target_os = "macos")]
impl crate::code_rule_dsl::logic::plan::joins::Pricer for ModelPricer {
    /// Every step of every candidate of the round in one prediction (on the device, past a few steps), then each
    /// candidate's price from its steps' targets.
    fn price(&self, round: &[crate::code_rule_dsl::logic::plan::joins::Choice]) -> Option<Vec<Vec<f64>>> {
        let steps: Vec<&StepSample> = round.iter().flat_map(|c| c.candidates.iter().flat_map(|k| &k.steps)).collect();
        if steps.is_empty() {
            return None;
        }
        let targets = self.0.fanouts(&steps).ok()?;
        let mut at = 0;
        Some(
            round
                .iter()
                .map(|c| {
                    c.candidates
                        .iter()
                        .map(|k| {
                            let t = &targets[at..at + k.steps.len()];
                            at += k.steps.len();
                            model_price(k.est, &k.steps, t).unwrap_or(k.est)
                        })
                        .collect()
                })
                .collect(),
        )
    }
}

/// Trains the join model on the stored steps of `machine` and sets it as the batch pricer (called on the cost
/// statistics' read thread, with `RERAIL_JOIN_MODEL=1`).
#[cfg(target_os = "macos")]
pub fn install_model(db: &Connection, machine: &str) {
    let t = std::time::Instant::now();
    let cpu = rerail_accel::exec::thread_cpu();
    let gpu = rerail_forest_gpu_seconds();
    let steps: Vec<StepSample> = match read(db, machine, READ) {
        Ok(r) => r.into_iter().map(|r| r.1).collect(),
        Err(e) => return eprintln!("rerail: join model: steps not read: {e}"),
    };
    if steps.len() < 10_000 {
        return eprintln!("rerail: join model: {} steps stored, under 10,000: the planner's estimates choose", steps.len());
    }
    let config = rerail_accel::cost_model::CostConfig::default().forest;
    match rerail_accel::join_model::JoinModel::train(&steps, &config) {
        Ok(m) => {
            eprintln!("rerail: join model in place: trained on {} steps in {:.2} s: {:.2} s of this thread's CPU, {:.2} s of the GPU (the forest's device: {})", steps.len(), t.elapsed().as_secs_f64(), rerail_accel::exec::thread_cpu() - cpu, rerail_forest_gpu_seconds() - gpu, m.on_device());
            crate::code_rule_dsl::logic::plan::joins::set_pricer(Some(std::sync::Arc::new(ModelPricer(m))));
        }
        Err(e) => eprintln!("rerail: join model: not trained: {e}"),
    }
}

/// The GPU seconds of every command buffer the forests awaited so far (training's and prediction's).
#[cfg(target_os = "macos")]
fn rerail_forest_gpu_seconds() -> f64 {
    rerail_accel::join_model::gpu_seconds_awaited()
}
