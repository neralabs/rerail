//! The learned cost model's forests (decision 0100 §14), rerail-forest's: the model as the prices ask it (`Learned`,
//! `LearnedCost`), with each thread's memo of its predictions, and its comparison with the rates. rerail-accel's
//! `cost_model` installs it and asks it; it reaches this through `models::MODELS`, which `install` registers.

use rerail_accel::cost_model::{Comparison, Errors, Predicted};
use rerail_accel::kernel::Kernel;
use rerail_accel::lanes::Lane;
use rerail_accel::models::LearnedCost;
use rerail_accel::score::Sample;
use rerail_forest::cost_model::{CostMemo, CostQuery, CostSample, JobKey};
pub use rerail_forest::cost_model::{CostConfig, CostModel};
/// The forests' training settings ([`CostConfig::forest`]).
pub use rerail_forest::randomforest_procedure::RandomForestProcedureConfig as ForestConfig;


thread_local! {
    /// This thread's keys and memo, for the model of the version it holds.
    static PRICED: std::cell::RefCell<Priced> = std::cell::RefCell::new(Priced::default());
}

/// A thread's job keys by (kernel name's address, lane), and its memo of predictions, for one model version.
#[derive(Default)]
struct Priced {
    version: u64,
    jobs: Vec<((usize, usize, u8), Option<JobKey>)>,
    memo: CostMemo,
}

impl Priced {
    /// The job key of `name` on `lane`, found by address (the string looked up once).
    fn job(&mut self, model: &CostModel, name: &'static str, lane: Lane) -> Option<JobKey> {
        let key = (name.as_ptr() as usize, name.len(), lane.index() as u8);
        if let Some((_, j)) = self.jobs.iter().find(|(k, _)| *k == key) {
            return *j;
        }
        let j = model.job(lane.name(), name);
        self.jobs.push((key, j));
        j
    }
}

pub fn sample(s: &Sample) -> CostSample {
    CostSample { lane: s.lane.name().to_owned(), forms: s.forms.clone(), launches: s.launches, units: s.units, cold: s.cold, ok: s.ok, load: s.load, threads: s.threads, wall: s.wall, cpu: s.cpu }
}

/// rerail-forest's model, as the prices ask it (`LearnedCost`).
#[derive(Debug)]
pub struct Learned(pub CostModel);

impl LearnedCost for Learned {
    fn trained_on(&self) -> usize {
        self.0.trained_on
    }
    fn to_bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }
    fn warm(&self) -> Result<(), String> {
        self.0.warm()
    }
    fn predict_batch(&self, version: u64, kernels: &[(&Kernel, Vec<Lane>)], load: f64, threads: u32) -> Vec<[Option<Predicted>; Lane::COUNT]> {
        predict_batch(&self.0, version, kernels, load, threads)
    }
}

fn predict_batch(model: &CostModel, version: u64, kernels: &[(&Kernel, Vec<Lane>)], load: f64, threads: u32) -> Vec<[Option<Predicted>; Lane::COUNT]> {
    let mut out = vec![[None; Lane::COUNT]; kernels.len()];
    let shared = model.shared_buckets(1, false, load, threads);
    PRICED.with_borrow_mut(|priced| {
        if priced.version != version {
            *priced = Priced { version, ..Default::default() };
        }
        for (i, (k, lanes)) in kernels.iter().enumerate() {
            let (name, units) = (k.name(), k.units());
            for &l in lanes {
                let Some(job) = priced.job(model, name, l) else { continue };
                // A failed prediction leaves the rates' price: placement never fails on the model.
                if let Ok(p) = priced.memo.get(model, model.key(job, shared, units)) {
                    out[i][l.index()] = Some(Predicted { wall: p.wall, cpu: p.cpu });
                }
            }
        }
    });
    out
}

/// The comparison of a model trained on all but the newest fifth of `samples` (oldest first), on that fifth;
/// and the model.
pub fn compare(samples: &[Sample], config: &CostConfig) -> Result<(Comparison, CostModel), String> {
    compare_unless(samples, config, &|| false)?.ok_or_else(|| "stopped".into())
}

/// `compare`, `None` once `stop` says so between its model's forests.
pub fn compare_unless(samples: &[Sample], config: &CostConfig, stop: &dyn Fn() -> bool) -> Result<Option<(Comparison, CostModel)>, String> {
    let cut = samples.len() - samples.len() / 5;
    let (train, test) = samples.split_at(cut);
    let Some(model) = CostModel::train_unless(&train.iter().map(sample).collect::<Vec<_>>(), config, stop)? else { return Ok(None) };
    let rates = rerail_accel::price::fit_rates(train);
    let prices = rerail_accel::price::Prices::default();
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
        if s.forms.split(',').all(|f| f == name)
            && let Some(r) = rates.iter().find(|r| r.lane == s.lane && r.name == name)
        {
            let f = prices.fixed[s.lane.index()];
            rated.0.push((p.wall, s.wall));
            rated.1.push((f.submission + (f.launch + r.per_launch) * f64::from(s.launches) + r.per_unit * s.units, s.wall));
            rated.2.push((s.estimate, s.wall));
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
    Ok(Some((c, model)))
}
