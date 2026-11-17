//! Each program's measured steps (decision 0334), as the cost store keeps them (`program_steps`): the rates a task's
//! predicted steps are made from (the engine's `transducer::packed::evaluation_costs`), and the runs' observations, which wait
//! here for the store's writer (the engine's `logic::costs`, `drain`) only while one is on (`keep`).
//!
//! The rates are read from the store once a run (`install`): the store's statistics, as the executor's fitted prices
//! are, not results kept for a later run.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// Observations kept for the writer at most; past it they are dropped (and counted).
const PENDING_MOST: usize = 100_000;

/// A program's measured cost: steps per unit of size, over `samples` evaluations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub steps_per_unit: f64,
    pub samples: u64,
}

/// One evaluation's run.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    /// The program's digest (the transducer's `Program::plain_digest`).
    pub program: String,
    /// The steps its tasks took: at least the budget where one stopped over it.
    pub steps: u64,
    pub size: u64,
    pub predicted: u64,
    /// A task stopped over its budget: a misprediction, restarted.
    pub over_budget: bool,
}

static KEEP: AtomicBool = AtomicBool::new(false);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static PENDING: Mutex<Vec<Observation>> = Mutex::new(Vec::new());
static RATES: RwLock<Option<HashMap<String, Rate>>> = RwLock::new(None);

/// Keeps observations for the store's writer (it is on), or not.
pub fn keep(on: bool) {
    KEEP.store(on, Ordering::Relaxed);
}

pub fn keeping() -> bool {
    KEEP.load(Ordering::Relaxed)
}

pub fn observe(observations: impl IntoIterator<Item = Observation>) {
    if !KEEP.load(Ordering::Relaxed) {
        return;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for observation in observations {
        if pending.len() >= PENDING_MOST {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        } else {
            pending.push(observation);
        }
    }
}

/// The observations since the last drain, and how many were dropped.
pub fn drain() -> (Vec<Observation>, u64) {
    let taken = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    (taken, DROPPED.swap(0, Ordering::Relaxed))
}

/// The store's rates, for this run's predictions.
pub fn install(rates: HashMap<String, Rate>) {
    *RATES.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(rates);
}

pub fn rate(program: &str) -> Option<Rate> {
    RATES.read().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref()?.get(program).copied()
}

