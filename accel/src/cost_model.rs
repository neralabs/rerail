//! The learned cost model in the prices (decision 0100 §14): `rerail-forest`'s two regression forests over the
//! executor's samples (rerail-accel-gpu's, through [`crate::models`]), asked once per batch for every launch on every
//! lane that can run it ([`predict_batch`]),
//! and placement then weighs each lane by the model's cost where it gave one ([`crate::price::Prices::cost_predicted`]),
//! by the rates elsewhere.
//!
//! **Installed only when it wins.** [`train`] holds out the newest fifth of the samples, trains on the rest, and
//! compares the model's errors on the held-out jobs with the rates' (the medians, [`crate::price::fit_rates`],
//! fitted on the same rest) and with what the prices estimated when the jobs ran ([`Comparison`]). Only a model
//! no worse than both is trained again on every sample and installed ([`install`]); otherwise the rates stay.
//!
//! **Read without a lock.** Each thread keeps its copy of the installed model, taken again only when the version
//! moves, as the learned rates are.
//!
//! **No strings, no trees, on the placement path.** With its copy a thread keeps, for that model (the model's own
//! memo, `LearnedCost::predict_batch`): each (kernel name,
//! lane)'s job key, found by the name's address (`Kernel::name` is a `&'static str`) the first time it is asked;
//! and a memo of predictions by the job's buckets (`CostMemo`): the forests see a job only through its buckets, and
//! a batch's jobs share all but their units'. A launch is a lookup by address, its units' bucket (a binary search)
//! and a lookup in the memo; the forests are walked only for buckets not seen.

use crate::kernel::Kernel;
use crate::lanes::Lane;
use crate::models::{LearnedCost, models};
use crate::score::Sample;
use std::sync::{Arc, RwLock};

/// A launch's cost on a lane, as the model predicts it: its one-launch job's wall and host CPU seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Predicted {
    pub wall: f64,
    pub cpu: f64,
}

/// Errors of a set of predictions against what the jobs cost, on the log scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Errors {
    pub jobs: usize,
    /// The root mean square of ln(predicted / actual).
    pub rms_log: f64,
    /// The median of |ln(predicted / actual)|.
    pub median_log: f64,
    /// The share predicted within 2× of the actual.
    pub within_2x: f64,
}

impl Errors {
    pub fn of(pairs: &[(f64, f64)]) -> Errors {
        let mut e: Vec<f64> = pairs.iter().map(|&(p, a)| (p.max(1e-9) / a.max(1e-9)).ln()).filter(|v| v.is_finite()).collect();
        if e.is_empty() {
            return Errors::default();
        }
        let n = e.len() as f64;
        let rms_log = (e.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
        let within_2x = e.iter().filter(|v| v.abs() <= std::f64::consts::LN_2).count() as f64 / n;
        e.iter_mut().for_each(|v| *v = v.abs());
        e.sort_by(f64::total_cmp);
        Errors { jobs: e.len(), rms_log, median_log: e[e.len() / 2], within_2x }
    }
}

impl std::fmt::Display for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} jobs: rms {:.3} (×{:.2}), median ×{:.2}, {:.0}% within 2×", self.jobs, self.rms_log, self.rms_log.exp(), self.median_log.exp(), self.within_2x * 100.0)
    }
}

/// The model against the rates on the held-out jobs (warm and successful: what pricing is for).
#[derive(Clone, Debug, Default)]
pub struct Comparison {
    pub trained_on: usize,
    pub held_out: usize,
    /// The jobs the model prices (it knows their lane and names), of the held out.
    pub model_covers: usize,
    /// Wall, on the jobs the rates have a learned rate for and the model knows: the model, the medians, and the
    /// prices' estimates when the jobs ran.
    pub rated_model: Errors,
    pub rated_medians: Errors,
    pub rated_recorded: Errors,
    /// Wall and CPU on every held-out job the model knows: the model, and the prices' estimates when they ran.
    pub wall_model: Errors,
    pub wall_recorded: Errors,
    pub cpu_model: Errors,
    pub cpu_recorded: Errors,
}

impl Comparison {
    /// The model is no worse than the medians where both price, and than the recorded estimates.
    pub fn model_wins(&self) -> bool {
        self.model_covers > 0
            && (self.rated_model.jobs == 0 || self.rated_model.rms_log <= self.rated_medians.rms_log)
            && self.wall_model.rms_log <= self.wall_recorded.rms_log
    }
}

impl std::fmt::Display for Comparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "trained on {}, {} held out, the model prices {}", self.trained_on, self.held_out, self.model_covers)?;
        writeln!(f, "  wall, jobs with a learned rate: model {}", self.rated_model)?;
        writeln!(f, "                                  medians {}", self.rated_medians)?;
        writeln!(f, "                                  recorded {}", self.rated_recorded)?;
        writeln!(f, "  wall, jobs the model knows:     model {}", self.wall_model)?;
        writeln!(f, "                                  recorded {}", self.wall_recorded)?;
        writeln!(f, "  cpu, jobs the model knows:      model {}", self.cpu_model)?;
        write!(f, "                                  recorded {}", self.cpu_recorded)
    }
}

/// The machine's learned model (decision 0442), and its version: 0 before one is installed, else a number no other
/// machine's model has (`NEXT`), so a thread's copy (`COPY`) and the model's memo of it are never another machine's.
#[derive(Default)]
struct Model {
    model: RwLock<Option<Arc<dyn LearnedCost>>>,
    version: std::sync::atomic::AtomicU64,
}
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn model() -> Arc<Model> {
    rerail_context::machine::<Model>()
}

thread_local! {
    static COPY: std::cell::RefCell<(u64, Option<Arc<dyn LearnedCost>>)> = const { std::cell::RefCell::new((0, None)) };
}

/// The model in place, if one is.
pub fn current() -> Option<Arc<dyn LearnedCost>> {
    crate::price::first_price();
    let held = model();
    let version = held.version.load(std::sync::atomic::Ordering::Acquire);
    if version == 0 {
        return None;
    }
    COPY.with_borrow_mut(|(seen, copy)| {
        if *seen != version {
            *copy = held.model.read().ok().and_then(|m| m.clone());
            *seen = version;
        }
        copy.clone()
    })
}

/// Puts `model` in place, or takes the one in place out.
pub fn install(model: Option<Arc<dyn LearnedCost>>) {
    let held = self::model();
    if let Ok(mut m) = held.model.write() {
        *m = model;
        held.version.store(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Release);
    }
}

/// Every launch's predicted cost on each of its lanes (by lane index), one model call for the batch: `None` where
/// there is no model, it does not know the launch's lane and name, or the prediction failed.
pub fn predict_batch(kernels: &[(&Kernel, Vec<Lane>)], load: f64) -> Vec<[Option<Predicted>; Lane::COUNT]> {
    let Some(model) = current() else { return vec![[None; Lane::COUNT]; kernels.len()] };
    let version = self::model().version.load(std::sync::atomic::Ordering::Acquire);
    model.predict_batch(version, kernels, load, crate::exec::threads() as u32)
}

/// A saved model, read again (`LearnedCost::to_bytes`).
pub fn from_bytes(bytes: &[u8]) -> Result<Arc<dyn LearnedCost>, String> {
    models()?.cost_from_bytes(bytes)
}

/// The comparison of a model trained on all but the newest fifth of `samples` (oldest first), on that fifth; and the
/// model. Nothing is put in place.
pub fn compare(samples: &[Sample]) -> Result<(Comparison, Arc<dyn LearnedCost>), String> {
    models()?.compare_cost(samples)
}

/// Puts the calling thread at the quality of service the models train at (utility); nothing without models.
pub fn utility() {
    if let Ok(m) = models() {
        m.utility();
    }
}

/// What [`train`] did.
pub struct Trained {
    pub comparison: Comparison,
    /// The model installed (trained on every sample), when it won; else none is.
    pub model: Option<Arc<dyn LearnedCost>>,
}

/// Compares a model with the rates on `samples` (oldest first) and, when it wins, trains it on all of them and
/// installs it. With fewer than `min` samples, nothing is trained.
pub fn train(samples: &[Sample], min: usize) -> Result<Trained, String> {
    train_until(samples, min, &|| false)
}

/// [`train`], given up between its phases (the comparison, the training, the kernels' compile) once `stop` says so:
/// upkeep's retraining yields to a run that begins (`upkeep::Slice::stopper`). Nothing is put in place then.
pub fn train_until(samples: &[Sample], min: usize, stop: &dyn Fn() -> bool) -> Result<Trained, String> {
    const STOPPED: &str = "stopped: a run began";
    if samples.len() < min {
        return Err(format!("{} samples, fewer than {min}", samples.len()));
    }
    let models = models()?;
    let (comparison, _) = models.compare_cost(samples)?;
    if !comparison.model_wins() {
        return Ok(Trained { comparison, model: None });
    }
    if stop() {
        return Err(STOPPED.into());
    }
    let model = models.train_cost(samples)?;
    if stop() {
        return Err(STOPPED.into());
    }
    // Its kernels compiled here, not on the first placement that asks; one that fails leaves the rates.
    model.warm()?;
    install(Some(model.clone()));
    Ok(Trained { comparison, model: Some(model) })
}
