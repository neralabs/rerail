//! The learned models as the engine reaches them (crate split 10d-2): the cost model in the prices (`LearnedCost`,
//! [`crate::cost_model`]), the join model of the planner's fanouts (`LearnedJoin`, [`crate::join_model`]), and what
//! trains them and reads a saved one (`Models`).
//!
//! Their forests are rerail-forest's, trained and walked on the device, over the GPU's compiler (rerail-gpu, UKL). The
//! implementations are rerail-accel-gpu's, which registers them as the engine boots ([`MODELS`]): the executor and
//! every crate that prices with a model build without the device's stack, and an edit to it rebuilds only the crates
//! above rerail-accel-gpu. With none registered (a build off macOS, or a binary that does not boot) there is no
//! model: the prices are the rates, and the planner's estimates choose.

use crate::cost_model::{Comparison, Predicted};
use crate::join_model::StepSample;
use crate::kernel::Kernel;
use crate::lanes::Lane;
use crate::score::Sample;
use std::sync::Arc;

/// A learned model of what a launch costs on each lane (rerail-forest's two regression forests over the executor's
/// samples, decision 0100 §14).
pub trait LearnedCost: std::fmt::Debug + Send + Sync + 'static {
    /// The samples it was trained on.
    fn trained_on(&self) -> usize;
    /// The model as this machine saves it (`Models::cost_from_bytes` reads it again).
    fn to_bytes(&self) -> Vec<u8>;
    /// Compiles its kernels now, so no placement waits on them: an error leaves the rates.
    fn warm(&self) -> Result<(), String>;
    /// Every launch's cost on each of its lanes (by lane index), as [`crate::cost_model::predict_batch`] asks it:
    /// the model installed as `version`, for `threads` of the pool under `load`. A thread's memo of its predictions
    /// is the model's, for that version.
    fn predict_batch(&self, version: u64, kernels: &[(&Kernel, Vec<Lane>)], load: f64, threads: u32) -> Vec<[Option<Predicted>; Lane::COUNT]>;
}

/// A learned model of a join step's rows out over the statistics' estimate of them (decision 0354).
pub trait LearnedJoin: std::fmt::Debug + Send + Sync + 'static {
    /// A step's predicted target (its rows out over the statistics' estimate, on the log scale).
    fn fanout(&self, step: &StepSample) -> Result<f64, String>;
    /// The steps' targets, on the device past a few steps; the rounds asked at once share a dispatch.
    fn fanouts(&self, steps: &[&StepSample]) -> Result<Vec<f64>, String>;
    /// Whether the forest is on the device.
    fn on_device(&self) -> bool;
    /// Rounds asked, dispatches made, and rounds walked on the host, so far.
    fn counts(&self) -> [u64; 3];
    /// Seconds of CPU: bucketing, waiting, dispatching or walking.
    fn cpu(&self) -> [f64; 3];
    /// The steps it was trained on.
    fn trained_on(&self) -> usize;
    /// The rows a dispatch takes to the device from.
    fn device_rows(&self) -> usize;
    /// What its calibration measured: a dispatch's CPU, and a host walk's per step, in seconds.
    fn costs(&self) -> (f64, f64);
}

/// What trains the models and reads a saved cost model: rerail-accel-gpu's, on its device.
pub trait Models: Send + Sync {
    /// A cost model saved by `LearnedCost::to_bytes`.
    fn cost_from_bytes(&self, bytes: &[u8]) -> Result<Arc<dyn LearnedCost>, String>;
    /// A cost model trained on all but the newest fifth of `samples` (oldest first), compared on that fifth with the
    /// rates and with what the prices estimated when the jobs ran; and the model. `None` once `stop` says so, between
    /// the model's forests.
    fn compare_cost(&self, samples: &[Sample], stop: &dyn Fn() -> bool) -> Result<Option<(Comparison, Arc<dyn LearnedCost>)>, String>;
    /// A cost model trained on every one of `samples`; `None` once `stop` says so, between its forests.
    fn train_cost(&self, samples: &[Sample], stop: &dyn Fn() -> bool) -> Result<Option<Arc<dyn LearnedCost>>, String>;
    /// A join model trained on `steps`.
    fn train_join(&self, steps: &[StepSample]) -> Result<Arc<dyn LearnedJoin>, String>;
    /// Puts the calling thread at the quality of service the models train at (utility).
    fn utility(&self);
    /// The GPU seconds of every command buffer the forests awaited so far in this process.
    fn gpu_seconds_awaited(&self) -> f64;
}

/// The models' trainer, registered as the engine boots (rerail-accel-gpu's `install`).
pub static MODELS: rerail_registry::Slot<&'static dyn Models> = rerail_registry::Slot::new("the learned models (accel::models::MODELS)");

/// The registered models, or why there are none.
pub fn models() -> Result<&'static dyn Models, String> {
    MODELS.try_get().ok_or_else(|| "no learned models in this binary (none registered: accel::models::MODELS)".to_owned())
}
