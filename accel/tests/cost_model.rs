//! The learned cost model in placement (decision 0100 §14): a model installed for a launch's lane and name decides
//! its lane in place of the rates; without one, the rates do. Its own test binary: the installed model is the
//! process's.
#![cfg(target_os = "macos")]

use rerail_accel::cost_model::{CostConfig, CostModel, install, predict_batch, sample};
use rerail_accel::score::Sample;
use rerail_accel::{Batch, Executor, Kernel, Lane, Launch};
use std::sync::Arc;

#[derive(Debug)]
struct Priced;

impl rerail_accel::kernel::Program for Priced {
    fn name(&self) -> &'static str {
        "a model-priced program"
    }
    fn units(&self) -> f64 {
        1e9
    }
    // The rates send it to the GPU: far cheaper there.
    fn per_unit(&self, lane: Lane) -> Option<f64> {
        match lane {
            Lane::Cpu => Some(1e-8),
            Lane::Gpu => Some(1e-12),
            _ => None,
        }
    }
    fn run(&self, lane: Lane) -> Result<rerail_accel::kernel::ProgramOutput, String> {
        Ok(Arc::new(lane))
    }
}

/// The samples say the opposite of the rates: on the cores it is all but free, on the GPU a second.
fn samples() -> Vec<Sample> {
    let at = |lane: Lane, i: usize, wall: f64| Sample {
        lane,
        forms: "a model-priced program".into(),
        launches: 1,
        units: 1e9 * (1.0 + (i % 5) as f64 * 0.01),
        estimate: 0.0,
        host_estimate: 0.0,
        wall: wall * (1.0 + (i % 7) as f64 * 0.01),
        cpu: wall * 0.5,
        waited: 0.0,
        phases: String::new(),
        cold: false,
        ok: true,
        load: 1.0,
        threads: 1,
    };
    (0..200).flat_map(|i| [at(Lane::Cpu, i, 1e-5), at(Lane::Gpu, i, 1.0)]).collect()
}

#[test]
fn an_installed_model_places_by_its_costs_and_the_rates_without_it() {
    let ex = Executor::new();
    if !ex.lanes().contains(&Lane::Gpu) {
        return;
    }
    let k = Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(Priced)));
    let batch = || Batch(vec![Launch::new(k.clone())]);
    install(None);
    assert_eq!(ex.place("test", &batch()).unwrap().0[0].0, Lane::Gpu, "the rates: the GPU");

    let config = CostConfig { forest: rerail_forest_config(), ..Default::default() };
    let model = CostModel::train(&samples().iter().map(sample).collect::<Vec<_>>(), &config).unwrap();
    install(Some(Arc::new(model)));
    let p = predict_batch(&[(&k, vec![Lane::Cpu, Lane::Gpu])], 1.0);
    let (cpu, gpu) = (p[0][Lane::Cpu.index()].unwrap(), p[0][Lane::Gpu.index()].unwrap());
    assert!(cpu.wall < 1e-4 && gpu.wall > 0.5, "{cpu:?} {gpu:?}");
    let (placed, _) = ex.place("test", &batch()).unwrap();
    assert_eq!(placed[0].0, Lane::Cpu, "the model: the cores");
    let (done, _) = ex.run(batch()).unwrap();
    assert_eq!(done[0].lane, Lane::Cpu);

    // A launch the model does not know keeps the rates.
    let other = Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(Unknown)));
    assert_eq!(predict_batch(&[(&other, vec![Lane::Cpu, Lane::Gpu])], 1.0)[0], [None; 4]);
    install(None);
    assert_eq!(ex.place("test", &batch()).unwrap().0[0].0, Lane::Gpu, "the model taken out: the rates again");
}

#[derive(Debug)]
struct Unknown;

impl rerail_accel::kernel::Program for Unknown {
    fn name(&self) -> &'static str {
        "a program the model never saw"
    }
    fn units(&self) -> f64 {
        1e9
    }
    fn per_unit(&self, lane: Lane) -> Option<f64> {
        matches!(lane, Lane::Cpu | Lane::Gpu).then_some(1e-9)
    }
    fn run(&self, lane: Lane) -> Result<rerail_accel::kernel::ProgramOutput, String> {
        Ok(Arc::new(lane))
    }
}

/// A small forest: the test's data is simple.
fn rerail_forest_config() -> rerail_accel::cost_model::ForestConfig {
    rerail_accel::cost_model::ForestConfig { feature_vector_samplings: 20, ..CostConfig::default().forest }
}
