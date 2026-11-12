//! Metal, as the engine's device (`rerail-device-metal`): the implementations of the logic's and the transducer's
//! device interfaces (`logic::device::SolverDevice`, `transducer::packed::device::MatcherDevice`) over the Metal
//! code, registered as the engine loads (`crate::install_devices`). macOS only; the core never names it.
use crate::code_rule_dsl::logic::device::{JoinPlan, Refused, Report, SolverDevice, lattice, nearest};
use crate::code_rule_dsl::logic::native::setfix::{Fresh, Interner, JoinRule, Store, device as joins};
use crate::code_rule_dsl::transducer::packed::{self, Error};

/// The Metal device.
pub(crate) struct Metal;

/// The one instance the registries hold.
pub(crate) static METAL: Metal = Metal;

/// Registers Metal as the logic's and the transducer's device.
pub fn register() {
    crate::code_rule_dsl::logic::device::register(&METAL);
    crate::code_rule_dsl::transducer::packed::device::register(&METAL);
    crate::code_rule_dsl::prof::register_device_memory(|| (packed::metal_backend::reserved_mb(), packed::metal_backend::driver_mb()));
}

fn refused(error: Error) -> Refused {
    match error {
        Error::UnsupportedBackend => Refused::NoDevice,
        other => Refused::Failed(format!("{other:?}")),
    }
}

/// The device's own plans of `rules`, each with its rule and head relation.
fn plans<'a>(rules: &[(&'a JoinPlan, &'a JoinRule, usize)]) -> Result<Vec<(&'a joins::Plan, &'a JoinRule, usize)>, String> {
    rules
        .iter()
        .map(|(plan, join, head)| {
            let plan = plan.0.downcast_ref::<joins::Plan>().ok_or_else(|| "a join plan of another device".to_owned())?;
            Ok((plan, *join, *head))
        })
        .collect()
}

impl SolverDevice for Metal {
    fn join_available(&self) -> bool {
        packed::metal_join::available()
    }

    fn compile_join(&self, join: &JoinRule, spec: &str) -> Result<JoinPlan, String> {
        joins::compile(join, spec).map(|plan| JoinPlan(std::sync::Arc::new(plan)))
    }

    fn refuse(&self, why: String) {
        joins::refuse(why);
    }

    fn run_joins(&self, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
        joins::run(&plans(rules)?, store, names, fresh)
    }

    fn reads_own(&self, plan: &JoinPlan) -> bool {
        plan.0.downcast_ref::<joins::Plan>().is_some_and(joins::reads_own)
    }

    fn fixpoint(
        &self,
        seeds: &[(&JoinPlan, &JoinRule, usize)],
        deltas: &[(&JoinPlan, &JoinRule, usize)],
        heads: &[usize],
        store: &Store,
        names: &Interner,
        fresh: &mut [Fresh],
    ) -> Result<Vec<u64>, String> {
        joins::fixpoint(&plans(seeds)?, &plans(deltas)?, heads, store, names, fresh)
    }

    fn take_report(&self) -> Option<Report> {
        joins::take_report()
    }

    fn lattice(&self, system: &lattice::System<'_>) -> Result<lattice::Solved, Refused> {
        packed::metal_lattice::solve(system).map_err(refused)
    }

    fn nearest(&self, nodes: usize, target: &[u32], done: &[u8], how: nearest::Buffers) -> Result<nearest::Solved, Refused> {
        packed::metal_nearest::solve_with(nodes, target, done, how).map_err(refused)
    }

    fn install_launch_guard(&self) {
        packed::metal_sort::install_guard();
    }
}

pub mod matcher {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use crate::code_rule_dsl::tape::{TapeDoc, TapeSource};
    use crate::code_rule_dsl::transducer::packed::device::{Engine, MatcherDevice, Outcome, PageConsumer, RootJobs};
    use crate::code_rule_dsl::transducer::packed::{
        BatchOutcome, Error, ExecutionBackend, LoweredMatcher, LoweredWave, PackedExecution, Program, matcher_metal, metal_grid_backend as grid,
        metal_program_backend as programs,
    };

    impl MatcherDevice for super::Metal {
        fn execute_packed(&self, engine: Engine, batch: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>], result_limits: Option<&[u32]>) -> Result<PackedExecution, Error> {
            match engine {
                Engine::Interpreter => {
                    let engine = programs::compile_interpreter()?;
                    programs::execute_packed_with_result_limits(&engine, batch, files, anchors, result_limits)
                }
                Engine::Jit => {
                    let engine = crate::code_rule_dsl::prof::time("roots.jit", || programs::compile_jit_cached(batch))?;
                    let _run = crate::code_rule_dsl::prof::Stage::start("roots.metal");
                    programs::execute_packed_with_result_limits(&engine, batch, files, anchors, result_limits)
                }
            }
        }

        fn execute_outcomes(&self, backend: ExecutionBackend, batch: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>]) -> Result<BatchOutcome, Error> {
            match backend {
                ExecutionBackend::MetalInterpreter => {
                    let engine = programs::compile_interpreter()?;
                    programs::execute_outcomes(&engine, batch, files, anchors)
                }
                ExecutionBackend::MetalJit => {
                    let engine = programs::compile_jit_cached(batch)?;
                    programs::execute_outcomes(&engine, batch, files, anchors)
                }
                ExecutionBackend::Metal => grid::execute_outcomes(batch, files, anchors),
                _ => Err(Error::UnsupportedBackend),
            }
        }

        fn warm(&self, batch: &[Program]) -> Result<(), Error> {
            programs::compile_jit_cached(batch).map(|_| ())
        }

        fn jit_ready(&self, batch: &[Program]) -> Result<bool, Error> {
            programs::jit_ready(batch)
        }

        fn compile_interpreter(&self) -> Result<(), Error> {
            programs::compile_interpreter_cached().map(|_| ())
        }

        fn compile_background(&self, batch: &[Program]) -> Result<(), Error> {
            programs::compile_sweep_cached()?;
            programs::compile_jit_cached(batch).map(|_| ())
        }

        fn execute_shared_corpus(&self, batch: &[Program], files: &[TapeDoc], anchors: &[Vec<Arc<[u32]>>], result_limits: &[u32]) -> Result<PackedExecution, Error> {
            grid::execute_shared_corpus(batch, files, anchors, result_limits)
        }

        fn execute_shared_corpus_pages(&self, batch: &[Program], files: &[TapeSource], anchors: &[Vec<Arc<[u32]>>], result_limits: &[u32], consumer: &mut PageConsumer<'_>) -> Result<(), Error> {
            grid::execute_shared_corpus_pages(batch, files, anchors, result_limits, consumer)
        }

        fn matcher_roots(&self, matcher: &LoweredMatcher, tapes: &[TapeDoc]) -> Result<(Vec<BTreeSet<u32>>, Outcome), Error> {
            matcher_metal::matcher_roots(matcher, tapes)
        }

        fn wave_roots(&self, wave: &LoweredWave, tapes: &[TapeDoc], roots: &[Vec<Vec<u32>>]) -> Result<(Vec<Vec<BTreeSet<u32>>>, Outcome), Error> {
            matcher_metal::wave_roots(wave, tapes, roots)
        }

        fn wave_labelled(&self, wave: &LoweredWave, tapes: &[TapeDoc], active: &[Vec<bool>], roots: Option<&RootJobs<'_>>) -> Result<(Vec<TapeDoc>, Outcome), Error> {
            matcher_metal::wave_labelled(wave, tapes, active, roots)
        }

        fn warm_labels(&self) {
            matcher_metal::warm();
        }

        fn drop_label_pipelines(&self) {
            programs::label_passes::drop_pipelines();
        }
        fn span_ends(&self, natives: &[&rerail_compiler::block_native::BlockNative], batch: &[crate::code_rule_dsl::transducer::packed::block::BatchFile<'_>]) -> Option<Vec<Vec<u64>>> {
            use programs::block_program::{ClassStrategy, Executor, LaneWidth, run_split};
            let run = run_split(natives, batch, LaneWidth::W64, Executor::Generated(ClassStrategy::Lookup), 64).ok()?;
            Some(run.files.into_iter().map(|mut r| r.outputs.swap_remove(0)).collect())
        }
        fn execution_metrics(&self) -> (usize, usize) {
            #[cfg(any(test, feature = "test-hooks"))]
            return programs::metrics();
            #[cfg(not(any(test, feature = "test-hooks")))]
            (0, 0)
        }
        fn reset_execution_metrics(&self) {
            #[cfg(any(test, feature = "test-hooks"))]
            programs::reset_metrics();
        }
    }
}
