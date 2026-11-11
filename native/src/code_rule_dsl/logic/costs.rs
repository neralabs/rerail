//! The cost statistics (decision 0100 §14): every job the executor runs, as a sample of what it was, what its price
//! expected and what it cost (`rerail_accel::score::Sample`), kept in a store across runs.
//!
//! - **Written back** by a thread of its own, which sleeps a second between writes: it takes the samples the
//!   executor kept and writes them in one transaction. A run's end flushes what is left ([`flush`]).
//! - **Read lazily, on another thread.** When the first executor is made, a job on the pool reads this machine's
//!   recent samples and fits the prices' per-unit rates from them (`rerail_accel::price::learn_from`). Until it
//!   has, the prices are the priors.
//!
//! The store is `RERAIL_COSTS` (a SQLite file; `off`: none), else `~/.cache/rerail/costs.sqlite`. Its table keeps
//! each sample's features as they were measured, so a regressor can be trained on it later.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub(in crate::code_rule_dsl) mod sql_plans;

/// The samples kept per machine: older ones are deleted past this.
const KEPT: i64 = 500_000;
/// The recent samples a run's prices are fitted from.
const READ: i64 = 20_000;

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
         CREATE INDEX IF NOT EXISTS samples_by_machine ON samples(machine, at);",
    )?;
    sql_plans::create(&db)?;
    Ok(db)
}

/// The writer's store, once opened.
static WRITER: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

/// Writes the samples the executor kept since the last write: one transaction.
fn write() {
    let Some(path) = path() else { return };
    let mut w = WRITER.lock().unwrap_or_else(|p| p.into_inner());
    let (samples, dropped) = rerail_accel::score::take_samples();
    let plans = sql_plans::take();
    if samples.is_empty() && plans.is_empty() {
        return;
    }
    if w.is_none() {
        *w = open(&path).ok();
        // Past `KEPT`, the oldest of this machine's samples go.
        if let Some(db) = w.as_ref() {
            let _ = db.execute("DELETE FROM samples WHERE machine = ?1 AND at < (SELECT at FROM samples WHERE machine = ?1 ORDER BY at DESC LIMIT 1 OFFSET ?2)", rusqlite::params![machine(), KEPT]);
        }
    }
    let Some(db) = w.as_mut() else {
        sql_plans::retry(plans);
        return;
    };
    let (machine, threads) = (machine(), rerail_accel::exec::threads() as i64);
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let r = (|| -> rusqlite::Result<Vec<i64>> {
        let tx = db.transaction()?;
        if !samples.is_empty() {
            let mut ins = tx.prepare_cached("INSERT INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)")?;
            for s in &samples {
                ins.execute(rusqlite::params![at, machine, threads, s.lane.name(), s.forms, s.launches, s.units, s.estimate, s.host_estimate, s.wall, s.cpu, s.waited, s.phases, s.cold, s.ok, s.load])?;
            }
        }
        let ids = if plans.is_empty() {
            Vec::new()
        } else {
            sql_plans::insert(&tx, &machine, threads, at, &plans)?
        };
        tx.commit()?;
        Ok(ids)
    })();
    match r {
        Ok(ids) if !ids.is_empty() => sql_plans::learn(ids, &plans),
        Ok(_) => {}
        Err(e) => {
            crate::code_rule_dsl::logic::profile::decision("cost statistics", || format!("{} job samples and {} SQL reports not written: {e}", samples.len(), plans.len()));
            sql_plans::retry(plans);
        }
    }
    if dropped > 0 {
        crate::code_rule_dsl::logic::profile::decision("cost statistics", || format!("{dropped} samples dropped: the writer fell behind"));
    }
}

/// Reads this machine's recent samples and fits the prices' rates from them.
fn read() {
    let Some(path) = path() else { return };
    if !path.exists() {
        return;
    }
    let r = (|| -> rusqlite::Result<Vec<rerail_accel::score::Sample>> {
        let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let machine = machine();
        if let Err(e) = sql_plans::read(&db, &machine, rerail_accel::exec::threads() as i64) {
            eprintln!("rerail: SQL planning observations not read from {}: {e}", path.display());
        }
        let mut q = db.prepare(
            "SELECT lane, forms, launches, units, estimate, host_estimate, wall, cpu, waited, phases, cold, ok, load
             FROM samples WHERE machine = ?1 ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = q.query_map(rusqlite::params![machine, READ], |r| {
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
            })
        })?;
        rows.collect()
    })();
    match r {
        Ok(samples) => {
            let fitted = rerail_accel::price::learn_from(&samples);
            *FITTED.lock().unwrap_or_else(|p| p.into_inner()) = Some((samples.len(), fitted.len()));
        }
        Err(e) => {
            eprintln!("rerail: cost statistics not read from {}: {e}", path.display());
            *FITTED.lock().unwrap_or_else(|p| p.into_inner()) = Some((0, 0));
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
        sql_plans::enable();
        rerail_accel::score::keep_samples(true);
        let _ = std::thread::Builder::new().name("rerail-costs".into()).spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                write();
            }
        });
        #[cfg(not(target_family = "wasm"))]
        rayon::spawn(|| rerail_accel::exec::as_job("the cost statistics read", || {
            let _ = std::panic::catch_unwind(read);
        }));
    });
}

/// Writes what is left: a run's end.
pub fn flush() {
    write();
}

/// For the profile: the samples read, the rates fitted and the rates in place.
pub(in crate::code_rule_dsl) fn summary() -> String {
    let fitted = *FITTED.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = match (path(), fitted) {
        (None, _) => "off (RERAIL_COSTS=off, or a test)\n".to_owned(),
        (Some(p), None) => format!("{}: not read yet\n", p.display()),
        (Some(p), Some((n, k))) => format!("{}: {n} samples read, {k} rates fitted\n", p.display()),
    };
    for l in rerail_accel::price::learned_rates() {
        s.push_str(&format!("  {:<4} {:<28} {:>12.3e} s a unit  ({} samples)\n", l.lane.name(), l.name, l.per_unit, l.samples));
    }
    s
}
