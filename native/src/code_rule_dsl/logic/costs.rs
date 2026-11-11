//! The cost statistics (decision 0100 §14): every job the executor runs, as a sample of what it was, what its price
//! expected and what it cost (`rerail_accel::score::Sample`), kept in a store across runs.
//!
//! - **Written back** by a thread of its own, which sleeps a second between writes: it takes the samples the
//!   executor kept and writes them in one transaction. A run's end flushes what is left ([`flush`]).
//! - **Read before the first wave** (decision 0104), when the statistics start (`start`, which a corpus run calls
//!   before its first wave): this machine's recent samples, and the prices' per-unit rates fitted from them
//!   (`rerail_accel::price::learn_from`). Without samples, the prices are the priors.
//!
//! The writer's thread and the training's work behind the run: what they send to SQLite no wave waits for.
//!
//! - **The learned model** (`rerail_accel::cost_model`): after the rates, the model saved for this machine (table
//!   `models`) is put in place, and a thread of its own, at the background quality of service and in a pool of two
//!   threads apart from the engine's, trains a model on the samples read. A model that beats the rates on the
//!   newest fifth of them is put in place and saved; one that does not is taken out, with the saved one. Training
//!   runs again once [`RETRAIN_AFTER`] more samples are written, at most every [`RETRAIN_EVERY`].
//!
//! The store is `RERAIL_COSTS` (a SQLite file; `off`: none), else `~/.cache/rerail/costs.sqlite`. Its table keeps
//! each sample's features as they were measured.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// The samples kept per machine: older ones are deleted past this.
const KEPT: i64 = 500_000;
/// The recent samples a run's prices are fitted from, and its model trained on.
const READ: i64 = 20_000;
/// The fewest samples a model is trained from.
const MIN_TRAIN: usize = 500;
/// The samples written after a training that start another.
pub const RETRAIN_AFTER: usize = 2_000;
/// The least time between two trainings.
pub const RETRAIN_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

fn path() -> Option<PathBuf> {
    match std::env::var("RERAIL_COSTS") {
        Ok(v) if v == "off" || v.is_empty() => None,
        Ok(v) => Some(PathBuf::from(v)),
        // Tests write no statistics unless asked to.
        Err(_) if cfg!(test) => None,
        Err(_) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/rerail/costs.sqlite")),
    }
}

/// This machine, as its samples are keyed: the architecture, its cores, and its GPU.
fn machine() -> String {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    #[cfg(target_os = "macos")]
    let gpu = rerail_accel::gpu::shared::get().map(|s| s.device.name().to_owned()).unwrap_or_default();
    #[cfg(not(target_os = "macos"))]
    let gpu = String::new();
    format!("{} {cores} cores {gpu}", std::env::consts::ARCH).trim().to_owned()
}

fn open(path: &PathBuf) -> rusqlite::Result<rusqlite::Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let db = rusqlite::Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS samples(
           at REAL, machine TEXT, threads INTEGER, lane TEXT, forms TEXT, launches INTEGER, units REAL,
           estimate REAL, host_estimate REAL, wall REAL, cpu REAL, waited REAL, phases TEXT,
           cold INTEGER, ok INTEGER, load REAL);
         CREATE INDEX IF NOT EXISTS samples_by_machine ON samples(machine, at);
         CREATE TABLE IF NOT EXISTS models(machine TEXT PRIMARY KEY, at REAL, trained_on INTEGER, comparison TEXT, model BLOB);",
    )?;
    Ok(db)
}

/// The writer's store, once opened.
static WRITER: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

/// Writes the samples the executor kept since the last write: one transaction.
fn write() {
    let (samples, dropped) = rerail_accel::score::take_samples();
    if samples.is_empty() {
        return;
    }
    let Some(path) = path() else { return };
    let mut w = WRITER.lock().unwrap_or_else(|p| p.into_inner());
    if w.is_none() {
        *w = open(&path).ok();
        // Past `KEPT`, the oldest of this machine's samples go.
        if let Some(db) = w.as_ref() {
            let _ = db.execute("DELETE FROM samples WHERE machine = ?1 AND at < (SELECT at FROM samples WHERE machine = ?1 ORDER BY at DESC LIMIT 1 OFFSET ?2)", rusqlite::params![machine(), KEPT]);
        }
    }
    let Some(db) = w.as_mut() else { return };
    let machine = machine();
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let r = (|| -> rusqlite::Result<()> {
        let tx = db.transaction()?;
        {
            let mut ins = tx.prepare_cached("INSERT INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)")?;
            for s in &samples {
                ins.execute(rusqlite::params![at, machine, s.threads, s.lane.name(), s.forms, s.launches, s.units, s.estimate, s.host_estimate, s.wall, s.cpu, s.waited, s.phases, s.cold, s.ok, s.load])?;
            }
        }
        tx.commit()
    })();
    match r {
        Ok(()) => {
            WRITTEN.fetch_add(samples.len(), std::sync::atomic::Ordering::Relaxed);
        }
        Err(e) => crate::code_rule_dsl::logic::profile::decision("cost statistics", || format!("{} samples not written: {e}", samples.len())),
    }
    if dropped > 0 {
        crate::code_rule_dsl::logic::profile::decision("cost statistics", || format!("{dropped} samples dropped: the writer fell behind"));
    }
}

/// This machine's most recent samples (at most `limit`), oldest first.
fn read_samples(db: &rusqlite::Connection, limit: i64) -> rusqlite::Result<Vec<rerail_accel::score::Sample>> {
    let mut q = db.prepare(
        "SELECT lane, forms, launches, units, estimate, host_estimate, wall, cpu, waited, phases, cold, ok, load, threads
         FROM samples WHERE machine = ?1 ORDER BY at DESC LIMIT ?2",
    )?;
    let rows = q.query_map(rusqlite::params![machine(), limit], |r| {
        let lane: String = r.get(0)?;
        Ok(rerail_accel::score::Sample {
            lane: rerail_accel::Lane::ALL.into_iter().find(|l| l.name() == lane).unwrap_or(rerail_accel::Lane::Cpu),
            forms: r.get(1)?,
            launches: r.get(2)?,
            units: r.get(3)?,
            estimate: r.get(4)?,
            host_estimate: r.get(5)?,
            wall: r.get(6)?,
            cpu: r.get(7)?,
            waited: r.get(8)?,
            phases: r.get(9)?,
            cold: r.get(10)?,
            ok: r.get(11)?,
            load: r.get(12)?,
            threads: r.get(13)?,
        })
    })?;
    let mut samples: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
    samples.reverse();
    Ok(samples)
}

/// Reads this machine's recent samples and fits the prices' rates from them; puts the saved model in place, then
/// trains one ([`train`]).
fn read() {
    let Some(path) = path() else { return };
    if !path.exists() {
        return;
    }
    let r = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).and_then(|db| read_samples(&db, READ));
    match r {
        Ok(samples) => {
            let fitted = rerail_accel::price::learn_from(&samples);
            *FITTED.lock().unwrap_or_else(|p| p.into_inner()) = Some((samples.len(), fitted.len()));
            #[cfg(target_os = "macos")]
            {
                load_saved(&path);
                train(path, samples);
            }
        }
        Err(e) => {
            eprintln!("rerail: cost statistics not read from {}: {e}", path.display());
            *FITTED.lock().unwrap_or_else(|p| p.into_inner()) = Some((0, 0));
        }
    }
}

/// Samples written since the last training began.
static WRITTEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// A training is running.
static TRAINING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// When the last training began.
static TRAINED_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// The model's state, for the profile.
static MODEL_STATE: Mutex<String> = Mutex::new(String::new());

fn model_state(s: String) {
    *MODEL_STATE.lock().unwrap_or_else(|p| p.into_inner()) = s;
}

/// Puts this machine's saved model in place, if there is one that reads.
#[cfg(target_os = "macos")]
fn load_saved(path: &PathBuf) {
    let Ok(db) = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
    let saved: rusqlite::Result<(Vec<u8>, i64)> = db.query_row("SELECT model, trained_on FROM models WHERE machine = ?1", [machine()], |r| Ok((r.get(0)?, r.get(1)?)));
    // No `models` table yet (a store from before it) reads as no model.
    let Ok((bytes, trained_on)) = saved else { return };
    match rerail_accel::cost_model::CostModel::from_bytes(&bytes) {
        Ok(m) => {
            rerail_accel::cost_model::install(Some(std::sync::Arc::new(m)));
            model_state(format!("the saved model ({trained_on} samples) in place; training a new one"));
        }
        Err(e) => model_state(format!("the saved model does not read ({e}); training a new one")),
    }
}

/// Trains a model on `samples` (oldest first) on a thread of its own, unless one is training: in a pool of two
/// threads at the background quality of service, apart from the engine's. A model that wins is put in place and
/// saved; else the model in place, and the saved one, are taken out.
#[cfg(target_os = "macos")]
fn train(path: PathBuf, samples: Vec<rerail_accel::score::Sample>) {
    use std::sync::atomic::Ordering;
    if TRAINING.swap(true, Ordering::AcqRel) {
        return;
    }
    WRITTEN.store(0, Ordering::Relaxed);
    *TRAINED_AT.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::time::Instant::now());
    let spawned = rerail_accel::exec::service("rerail-cost-model", None, move || {
        crate::code_rule_dsl::logic::profile::behind_the_run();
        rerail_accel::cost_model::set_background_qos();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .thread_name(|i| format!("rerail-cost-model-{i}"))
            .start_handler(|_| rerail_accel::cost_model::set_background_qos())
            .build();
        let started = std::time::Instant::now();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let config = rerail_accel::cost_model::CostConfig::default();
            match &pool {
                Ok(pool) => pool.install(|| rerail_accel::cost_model::train(&samples, &config, MIN_TRAIN)),
                Err(e) => Err(e.to_string()),
            }
        }));
        let took = started.elapsed().as_secs_f64();
        match r {
            Ok(Ok(t)) => {
                let saved = save(&path, t.model.as_deref(), &t.comparison);
                if t.model.is_none() {
                    rerail_accel::cost_model::install(None);
                }
                model_state(format!(
                    "{} ({took:.1} s){}\n{}",
                    if t.model.is_some() { "the model is in place" } else { "the model lost to the rates: not in place" },
                    saved.err().map_or(String::new(), |e| format!("; not saved: {e}")),
                    t.comparison
                ));
            }
            Ok(Err(e)) => model_state(format!("no model trained: {e}")),
            Err(_) => model_state("the model's training panicked".into()),
        }
        TRAINING.store(false, Ordering::Release);
    });
    if spawned.is_err() {
        TRAINING.store(false, Ordering::Release);
    }
}

/// Saves the model for this machine, or takes the saved one out when there is none.
#[cfg(target_os = "macos")]
fn save(path: &PathBuf, model: Option<&rerail_accel::cost_model::CostModel>, comparison: &rerail_accel::cost_model::Comparison) -> rusqlite::Result<()> {
    let db = open(path)?;
    match model {
        Some(m) => {
            let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
            db.execute(
                "INSERT OR REPLACE INTO models VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![machine(), at, m.trained_on as i64, comparison.to_string(), m.to_bytes()],
            )?;
        }
        None => {
            db.execute("DELETE FROM models WHERE machine = ?1", [machine()])?;
        }
    }
    Ok(())
}

/// Trains again when enough samples were written since the last training, and long enough ago (the writer's
/// thread, after a write).
fn maybe_retrain() {
    #[cfg(target_os = "macos")]
    {
        use std::sync::atomic::Ordering;
        if WRITTEN.load(Ordering::Relaxed) < RETRAIN_AFTER || TRAINING.load(Ordering::Acquire) {
            return;
        }
        if TRAINED_AT.lock().unwrap_or_else(|p| p.into_inner()).is_some_and(|t| t.elapsed() < RETRAIN_EVERY) {
            return;
        }
        let Some(path) = path() else { return };
        let r = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).and_then(|db| read_samples(&db, READ));
        if let Ok(samples) = r {
            train(path, samples);
        }
    }
}

/// The samples read and the rates fitted, once the read has run.
static FITTED: Mutex<Option<(usize, usize)>> = Mutex::new(None);

/// Starts the statistics, once: the executor keeps its samples, a thread writes them, and a job reads the stored
/// ones and fits the prices. Without a store (`RERAIL_COSTS=off`, or in tests) nothing is kept.
pub(in crate::code_rule_dsl) fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if path().is_none() {
            return;
        }
        rerail_accel::score::keep_samples(true);
        let _ = rerail_accel::exec::service("rerail-costs", None, || {
            crate::code_rule_dsl::logic::profile::behind_the_run();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                write();
                maybe_retrain();
            }
        });
        let _ = std::panic::catch_unwind(read);
    });
}

/// Writes what is left: a run's end.
pub fn flush() {
    // The command's end: its samples go to the statistics store as the writer thread's would, behind the run.
    crate::code_rule_dsl::logic::profile::behind(write);
}

/// For the profile: the samples read, the rates fitted and the rates in place.
pub(in crate::code_rule_dsl) fn summary() -> String {
    let fitted = *FITTED.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = match (path(), fitted) {
        (None, _) => "off (RERAIL_COSTS=off, or a test)\n".to_owned(),
        (Some(p), None) => format!("{}: not read yet\n", p.display()),
        (Some(p), Some((n, k))) => format!("{}: {n} samples read, {k} rates fitted\n", p.display()),
    };
    let state = MODEL_STATE.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if !state.is_empty() {
        s.push_str(&format!("  model: {}\n", state.replace('\n', "\n  ")));
    }
    for l in rerail_accel::price::learned_rates() {
        s.push_str(&format!("  {:<4} {:<28} {:>12.3e} s a unit  ({} samples)\n", l.lane.name(), l.name, l.per_unit, l.samples));
    }
    s
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The learned model against the rates on this machine's samples (decision 0100 §14): the store at
    /// `RERAIL_COSTS`, read only. Prints the comparison; asserts nothing about who wins.
    #[test]
    #[ignore = "reads the cost store: RERAIL_COSTS=<costs.sqlite> --ignored --nocapture"]
    fn the_model_against_the_rates() {
        let path = super::path().expect("RERAIL_COSTS");
        let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let limit: i64 = std::env::var("COSTS_READ").ok().and_then(|v| v.parse().ok()).unwrap_or(super::READ);
        let samples = super::read_samples(&db, limit).unwrap();
        let config = rerail_accel::cost_model::CostConfig::default();
        let (cpu, t) = (rerail_accel::exec::process_cpu(), std::time::Instant::now());
        let (c, model) = rerail_accel::cost_model::compare(&samples, &config).unwrap();
        eprintln!("{} samples; trained in {:.2} s wall {:.2} s CPU; {model:?}", samples.len(), t.elapsed().as_secs_f64(), rerail_accel::exec::process_cpu() - cpu);
        eprintln!("{c}");
        eprintln!("model wins: {}", c.model_wins());
    }

    /// Training on the store's samples saves a model that wins, and a saved model reads back and is put in place
    /// (`COSTS_COPY`: a copy of a store with enough samples; it is written).
    #[test]
    #[ignore = "writes the store it is given: COSTS_COPY=<a copy of costs.sqlite> --ignored --nocapture"]
    fn a_trained_model_is_saved_and_put_in_place_again() {
        use std::sync::atomic::Ordering;
        let path = std::path::PathBuf::from(std::env::var("COSTS_COPY").expect("COSTS_COPY"));
        let db = super::open(&path).unwrap();
        db.execute("DELETE FROM models", []).unwrap();
        let samples = super::read_samples(&db, super::READ).unwrap();
        let t = std::time::Instant::now();
        super::train(path.clone(), samples);
        while super::TRAINING.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let state = super::MODEL_STATE.lock().unwrap().clone();
        eprintln!("trained in {:.1} s: {state}", t.elapsed().as_secs_f64());
        let installed = rerail_accel::cost_model::current().expect("the model in place");
        let (trained_on, bytes): (i64, Vec<u8>) = db.query_row("SELECT trained_on, model FROM models WHERE machine = ?1", [super::machine()], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(trained_on as usize, installed.trained_on);
        assert_eq!(bytes, installed.to_bytes());
        eprintln!("saved: {} bytes", bytes.len());
        rerail_accel::cost_model::install(None);
        super::load_saved(&path);
        let loaded = rerail_accel::cost_model::current().expect("the saved model in place");
        assert_eq!(loaded.to_bytes(), bytes);
    }
}
