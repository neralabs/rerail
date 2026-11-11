//! The learned cost model in the prices (decision 0100 §14): `rerail-forest`'s two regression forests over the
//! executor's samples, asked once per batch for every launch on every lane that can run it ([`predict_batch`]),
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

use crate::kernel::Kernel;
use crate::lanes::Lane;
use crate::score::Sample;

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

#[cfg(target_os = "macos")]
mod on {
    use super::*;
    use rerail_forest::cost_model::{CostConfig, CostModel, CostQuery, CostSample};
    use std::sync::{Arc, RwLock};

    static MODEL: RwLock<Option<Arc<CostModel>>> = RwLock::new(None);
    static VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    thread_local! {
        static COPY: std::cell::RefCell<(u64, Option<Arc<CostModel>>)> = const { std::cell::RefCell::new((0, None)) };
    }

    pub fn sample(s: &Sample) -> CostSample {
        CostSample { lane: s.lane.name().to_owned(), forms: s.forms.clone(), launches: s.launches, units: s.units, cold: s.cold, ok: s.ok, load: s.load, threads: s.threads, wall: s.wall, cpu: s.cpu }
    }

    pub fn current() -> Option<Arc<CostModel>> {
        let version = VERSION.load(std::sync::atomic::Ordering::Acquire);
        if version == 0 {
            return None;
        }
        COPY.with_borrow_mut(|(seen, copy)| {
            if *seen != version {
                *copy = MODEL.read().ok().and_then(|m| m.clone());
                *seen = version;
            }
            copy.clone()
        })
    }

    pub fn install(model: Option<Arc<CostModel>>) {
        if let Ok(mut m) = MODEL.write() {
            *m = model;
            VERSION.fetch_add(1, std::sync::atomic::Ordering::Release);
        }
    }

    pub fn predict_batch(kernels: &[(&Kernel, Vec<Lane>)], load: f64) -> Vec<[Option<Predicted>; 4]> {
        let mut out = vec![[None; 4]; kernels.len()];
        let Some(model) = current() else { return out };
        let threads = crate::exec::threads() as u32;
        let mut at = Vec::new();
        let mut qs = Vec::new();
        for (i, (k, lanes)) in kernels.iter().enumerate() {
            for &l in lanes {
                at.push((i, l.index()));
                qs.push(CostQuery { lane: l.name(), forms: k.name(), launches: 1, units: k.units(), cold: false, load, threads });
            }
        }
        // A failed prediction leaves the rates' prices: placement never fails on the model.
        if let Ok(p) = model.predict(&qs) {
            for ((i, l), p) in at.into_iter().zip(p) {
                out[i][l] = p.map(|p| Predicted { wall: p.wall, cpu: p.cpu });
            }
        }
        out
    }

    /// The comparison of a model trained on all but the newest fifth of `samples` (oldest first), on that fifth;
    /// and the model.
    pub fn compare(samples: &[Sample], config: &CostConfig) -> Result<(Comparison, CostModel), String> {
        let cut = samples.len() - samples.len() / 5;
        let (train, test) = samples.split_at(cut);
        let model = CostModel::train(&train.iter().map(sample).collect::<Vec<_>>(), config)?;
        let rates = crate::price::fit_rates(train);
        let prices = crate::price::Prices::default();
        let test: Vec<&Sample> = test.iter().filter(|s| s.ok && !s.cold && s.wall > 0.0).collect();
        let qs: Vec<CostSample> = test.iter().map(|s| sample(s)).collect();
        let queries: Vec<CostQuery> = qs.iter().map(|s| CostQuery { lane: &s.lane, forms: &s.forms, launches: s.launches, units: s.units, cold: s.cold, load: s.load, threads: s.threads }).collect();
        let predicted = model.predict(&queries)?;
        let (mut rated, mut wall, mut cpu) = ((Vec::new(), Vec::new(), Vec::new()), (Vec::new(), Vec::new()), (Vec::new(), Vec::new()));
        for (s, p) in test.iter().zip(&predicted) {
            let Some(p) = p else { continue };
            wall.0.push((p.wall, s.wall));
            wall.1.push((s.estimate, s.wall));
            cpu.0.push((p.cpu, s.cpu));
            cpu.1.push((s.host_estimate, s.cpu));
            // The medians' job: the lane's submission and launches, and the rate times the units (`fit_rates`).
            let name = s.forms.split(',').next().unwrap_or("");
            if s.forms.split(',').all(|f| f == name) {
                if let Some(r) = rates.iter().find(|r| r.lane == s.lane && r.name == name) {
                    let f = prices.fixed[s.lane.index()];
                    rated.0.push((p.wall, s.wall));
                    rated.1.push((f.submission + f.launch * f64::from(s.launches) + r.per_unit * s.units, s.wall));
                    rated.2.push((s.estimate, s.wall));
                }
            }
        }
        let c = Comparison {
            trained_on: model.trained_on,
            held_out: test.len(),
            model_covers: wall.0.len(),
            rated_model: Errors::of(&rated.0),
            rated_medians: Errors::of(&rated.1),
            rated_recorded: Errors::of(&rated.2),
            wall_model: Errors::of(&wall.0),
            wall_recorded: Errors::of(&wall.1),
            cpu_model: Errors::of(&cpu.0),
            cpu_recorded: Errors::of(&cpu.1),
        };
        Ok((c, model))
    }
}

#[cfg(target_os = "macos")]
pub use on::{compare, current, install, sample};

#[cfg(target_os = "macos")]
pub use rerail_forest::cost_model::{CostConfig, CostModel};

/// The forests' training settings ([`CostConfig::forest`]).
#[cfg(target_os = "macos")]
pub use rerail_forest::randomforest_procedure::RandomForestProcedureConfig as ForestConfig;

/// Puts the calling thread at a quality of service (the model's training runs at utility).
#[cfg(target_os = "macos")]
pub use rerail_forest::randomforest_procedure::{Qos, set_qos};

/// Every launch's predicted cost on each of its lanes (by lane index), one model call for the batch: `None` where
/// there is no model, it does not know the launch's lane and name, or the prediction failed.
pub fn predict_batch(kernels: &[(&Kernel, Vec<Lane>)], load: f64) -> Vec<[Option<Predicted>; 4]> {
    #[cfg(target_os = "macos")]
    return on::predict_batch(kernels, load);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = load;
        vec![[None; 4]; kernels.len()]
    }
}

/// What [`train`] did.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct Trained {
    pub comparison: Comparison,
    /// The model installed (trained on every sample), when it won; else none is.
    pub model: Option<std::sync::Arc<CostModel>>,
}

/// Compares a model with the rates on `samples` (oldest first) and, when it wins, trains it on all of them and
/// installs it. With fewer than `min` samples, nothing is trained.
#[cfg(target_os = "macos")]
pub fn train(samples: &[Sample], config: &CostConfig, min: usize) -> Result<Trained, String> {
    if samples.len() < min {
        return Err(format!("{} samples, fewer than {min}", samples.len()));
    }
    let (comparison, _) = compare(samples, config)?;
    if !comparison.model_wins() {
        return Ok(Trained { comparison, model: None });
    }
    let all: Vec<_> = samples.iter().map(sample).collect();
    let model = std::sync::Arc::new(CostModel::train(&all, config)?);
    // Its kernels compiled here, not on the first placement that asks; one that fails leaves the rates.
    model.warm()?;
    install(Some(model.clone()));
    Ok(Trained { comparison, model: Some(model) })
}
