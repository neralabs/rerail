//! The accelerator's device side (crate split 10d): the GPU lane (`gpu`: its kernels, the device joins and tiles,
//! decision 0098 §5), and the learned models rerail-forest trains and walks on the device (`cost_model`,
//! `join_model`; decisions 0100 §14 and 0354), registered as the engine boots (`install`). rerail-accel holds what
//! the crates below reach: the pool and the launch ledger, the slots the lane registers into (`gpu::BACKEND`,
//! `gpu::MAINTENANCE`), and the models' interface (`models`). The executor and every crate that leases, counts or
//! prices build without the device's stack (rerail-forest, rerail-gpu, UKL), and an edit to it rebuilds only the
//! crates above this one.

#[cfg(gpu_lane)]
pub mod gpu;

#[cfg(target_os = "macos")]
pub mod cost_model;
#[cfg(target_os = "macos")]
pub mod join_model;

#[cfg(target_os = "macos")]
mod forest {
    use super::{cost_model, join_model};
    use rerail_accel::cost_model::Comparison;
    use rerail_accel::join_model::StepSample;
    use rerail_accel::models::{LearnedCost, LearnedJoin, Models};
    use rerail_accel::score::Sample;
    use std::sync::Arc;

    /// The models on this machine's device: rerail-forest's, at their default settings.
    pub struct Forest;

    impl Models for Forest {
        fn cost_from_bytes(&self, bytes: &[u8]) -> Result<Arc<dyn LearnedCost>, String> {
            Ok(Arc::new(cost_model::Learned(cost_model::CostModel::from_bytes(bytes)?)))
        }
        fn compare_cost(&self, samples: &[Sample]) -> Result<(Comparison, Arc<dyn LearnedCost>), String> {
            let (comparison, model) = cost_model::compare(samples, &cost_model::CostConfig::default())?;
            Ok((comparison, Arc::new(cost_model::Learned(model))))
        }
        fn train_cost(&self, samples: &[Sample]) -> Result<Arc<dyn LearnedCost>, String> {
            let all: Vec<_> = samples.iter().map(cost_model::sample).collect();
            Ok(Arc::new(cost_model::Learned(cost_model::CostModel::train(&all, &cost_model::CostConfig::default())?)))
        }
        fn train_join(&self, steps: &[StepSample]) -> Result<Arc<dyn LearnedJoin>, String> {
            Ok(Arc::new(join_model::JoinModel::train(steps, &cost_model::CostConfig::default().forest)?))
        }
        fn utility(&self) {
            rerail_forest::randomforest_procedure::set_qos(rerail_forest::randomforest_procedure::Qos::Utility);
        }
        fn gpu_seconds_awaited(&self) -> f64 {
            rerail_device::compute_kernel_metal::gpu_seconds_awaited()
        }
    }
}

/// Registers the GPU lane and the learned models (on macOS, rerail-forest's; elsewhere none) as the engine boots:
/// from boot's `install`, or a test binary's load. Once a process; a second call does nothing.
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        #[cfg(gpu_lane)]
        rerail_accel::gpu::BACKEND.register("rerail-accel-gpu", gpu::backend);
        #[cfg(target_os = "macos")]
        {
            rerail_accel::gpu::MAINTENANCE.register("rerail-accel-gpu", || Box::new(gpu::kernels::Maintenance::new()));
            rerail_accel::models::MODELS.register("rerail-accel-gpu", &forest::Forest);
        }
    });
}
