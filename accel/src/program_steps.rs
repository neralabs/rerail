//! Each program's measured steps (decision 0334), as the cost store keeps them (`program_steps`): the rates a task's
//! predicted steps are made from (the engine's `transducer::packed::evaluation_costs`), and the runs' observations, which wait
//! here for the store's writer (the engine's `logic::costs`, `drain`) only while one is on (`keep`).
//!
//! The rates are read from the store at a run's start (`install_sums`): the store's statistics, as the executor's
//! fitted prices are, not results kept for a later run. Each observation is folded into them as it is made
//! (`observe`), whether or not the writer is on: a misprediction reprices the program for every evaluation that has
//! not started yet (the wave's later chunks, the restarts), not only for the next run.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

/// Observations kept for the writer at most; past it they are dropped (and counted).
const PENDING_MOST: usize = 100_000;

/// A program's sums are halved past this many evaluations, as the store's (`logic::costs`): its rate follows its
/// recent runs.
pub const RATE_EVALUATIONS: u64 = 1_024;

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

/// The machine's program rates and their observations' queue (decision 0442: its learned cost model's).
#[derive(Default)]
struct Steps {
    keep: AtomicBool,
    dropped: AtomicU64,
    pending: Mutex<Vec<Observation>>,
    rates: RwLock<Option<HashMap<String, Sums>>>,
}

fn steps() -> std::sync::Arc<Steps> {
    rerail_context::machine::<Steps>()
}

/// A program's steps and sizes summed over its evaluations: its rate is their ratio.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Sums {
    steps: f64,
    size: f64,
    samples: u64,
}

impl Sums {
    fn rate(self) -> Option<Rate> {
        (self.size > 0.0 && self.samples > 0).then(|| Rate { steps_per_unit: self.steps / self.size, samples: self.samples })
    }

    fn add(&mut self, steps: f64, size: f64) {
        if self.samples > RATE_EVALUATIONS {
            self.steps /= 2.0;
            self.size /= 2.0;
            self.samples /= 2;
        }
        self.steps += steps;
        self.size += size;
        self.samples += 1;
    }
}

/// Keeps observations for the store's writer (it is on), or not.
pub fn keep(on: bool) {
    steps().keep.store(on, Ordering::Relaxed);
}

pub fn keeping() -> bool {
    steps().keep.load(Ordering::Relaxed)
}

/// Folds each observation into its program's rate, and keeps it for the writer while one is on.
pub fn observe(observations: impl IntoIterator<Item = Observation>) {
    let observations: Vec<Observation> = observations.into_iter().filter(|observation| observation.size > 0).collect();
    if observations.is_empty() {
        return;
    }
    {
        let held = steps();
        let mut rates = held.rates.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let rates = rates.get_or_insert_with(HashMap::new);
        for observation in &observations {
            // Get before insert: no key string made for a program already priced.
            match rates.get_mut(&observation.program) {
                Some(sums) => sums.add(observation.steps as f64, observation.size as f64),
                None => {
                    let mut sums = Sums::default();
                    sums.add(observation.steps as f64, observation.size as f64);
                    rates.insert(observation.program.clone(), sums);
                }
            }
        }
    }
    if !steps().keep.load(Ordering::Relaxed) {
        return;
    }
    let held = steps();
    let mut pending = held.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for observation in observations {
        if pending.len() >= PENDING_MOST {
            steps().dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            pending.push(observation);
        }
    }
}

/// The observations since the last drain, and how many were dropped.
pub fn drain() -> (Vec<Observation>, u64) {
    let taken = std::mem::take(&mut *steps().pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
    (taken, steps().dropped.swap(0, Ordering::Relaxed))
}

/// The store's sums (program, steps, size, evaluations), for this run's predictions.
pub fn install_sums(sums: impl IntoIterator<Item = (String, f64, f64, u64)>) {
    let sums = sums.into_iter().map(|(program, steps, size, samples)| (program, Sums { steps, size, samples })).collect();
    *steps().rates.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sums);
}

/// Rates as they are (a test's): each as one evaluation of size 1.
pub fn install(rates: HashMap<String, Rate>) {
    install_sums(rates.into_iter().map(|(program, rate)| (program, rate.steps_per_unit, 1.0, rate.samples.max(1))));
}

pub fn rate(program: &str) -> Option<Rate> {
    steps().rates.read().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref()?.get(program).copied()?.rate()
}

