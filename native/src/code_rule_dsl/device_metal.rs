//! Metal, as the engine's device (`rerail-device-metal`): the implementations of the logic's and the transducer's
//! device interfaces (`logic::device::{Device, SolverDevice}`, `transducer::packed::device::MatcherDevice`) over the Metal
//! code, registered as the engine loads (`crate::install_devices`). macOS only; the core never names it.
use crate::code_rule_dsl::logic::device::{ColumnStats, Device, JoinPlan, Refused, Report, SolverDevice, StatColumn, lattice, nearest};
use crate::code_rule_dsl::logic::native::setfix::{Fresh, Interner, JoinRule, Store, device as joins};
use crate::code_rule_dsl::transducer::packed::{self, Error};

/// The Metal device.
pub(crate) struct Metal;

/// The one instance the registries hold.
pub(crate) static METAL: Metal = Metal;

/// Registers Metal as the logic's and the transducer's device.
pub fn register() {
    crate::code_rule_dsl::logic::device::register("rerail-device-metal", &METAL);
    crate::code_rule_dsl::transducer::packed::device::register(&METAL);
    // Kind-set axes solved on the device in a batch's chunk block (decision 0289 §2).
    crate::code_rule_dsl::facts::nearest::install_chunk_axis(packed::metal_join::axis_in_chunk);
    crate::code_rule_dsl::prof::register_device_memory(|| (packed::metal_backend::reserved_mb(), packed::metal_backend::driver_mb()));
}

/// This crate's own test binary, which the engine does not load: Metal and the language packs its tests read are
/// registered as the binary loads, as the engine registers them (`crate::install_devices`, `install_languages`), with
/// what a job carries of its run (`run_state::CARRIERS`), so the device's tests run on the device and no test's CPU
/// worker meets a document before its pack is registered.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        super::register();
        // What a job carries of its run: the program's list, as boot registers it (`logic::CARRIERS`).
        crate::code_rule_dsl::logic::run_state::CARRIERS.register("rerail-device-metal's tests", crate::code_rule_dsl::logic::CARRIERS);
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        crate::test_languages();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
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

impl Device for Metal {
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

impl SolverDevice for Metal {
    fn join_available(&self) -> bool {
        packed::metal_join::available()
    }

    fn compile_join(&self, join: &JoinRule, spec: &str, own_texts: &[(usize, usize)]) -> Result<JoinPlan, String> {
        joins::compile(join, spec, own_texts).map(|plan| JoinPlan(std::sync::Arc::new(plan)))
    }

    fn refuse(&self, why: String) {
        joins::refuse(why);
    }

    fn run_joins(&self, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
        joins::run(&plans(rules)?, store, names, fresh)
    }

    fn submit(&self, rules: &[(&JoinPlan, &JoinRule, usize)], names: &Interner) -> Result<Option<u64>, String> {
        joins::submit(&plans(rules)?, names)
    }

    fn resume(&self, ticket: u64, rules: &[(&JoinPlan, &JoinRule, usize)], store: &Store, names: &Interner, fresh: &mut [Fresh]) -> Result<Vec<u64>, String> {
        joins::resume(ticket, &plans(rules)?, store, names, fresh)
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

    fn column_stats(&self, cols: &[StatColumn<'_>], k: usize) -> Result<(Vec<ColumnStats>, usize), Refused> {
        joins::column_stats(cols, k).map_err(Refused::Failed)
    }
}

pub mod matcher {
    use std::collections::BTreeSet;

    use crate::code_rule_dsl::tape::TapeDoc;
    use crate::code_rule_dsl::transducer::packed::device::{Engine, MatcherDevice, Outcome, RootJobs};
    use crate::code_rule_dsl::transducer::packed::{
        BatchOutcome, Error, ExecutionBackend, LoweredMatcher, Budget, LoweredWave, PackedExecution, Program, matcher_metal,
        metal_program_backend as programs,
    };

    impl MatcherDevice for super::Metal {
        fn execute_packed(&self, engine: Engine, batch: &[Program], files: &[TapeDoc], anchors: &[Vec<Vec<u32>>], result_limits: Option<&[u32]>, budget: Budget) -> Result<PackedExecution, Error> {
            match engine {
                Engine::Interpreter => {
                    let engine = programs::compile_interpreter()?;
                    programs::execute_packed_budgeted(&engine, batch, files, anchors, result_limits, budget)
                }
                Engine::Jit => {
                    let engine = crate::code_rule_dsl::prof::time("roots.jit", || programs::compile_jit_cached(batch))?;
                    let _run = crate::code_rule_dsl::prof::Stage::start("roots.metal");
                    programs::execute_packed_budgeted(&engine, batch, files, anchors, result_limits, budget)
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
            programs::compile_sweep_cached(crate::code_rule_dsl::transducer::packed::metal_program_backend::label_passes::run_format())?;
            programs::compile_jit_cached(batch).map(|_| ())
        }

        fn matcher_roots(&self, matcher: &LoweredMatcher, tapes: &[TapeDoc]) -> Result<(Vec<BTreeSet<u32>>, Outcome), Error> {
            matcher_metal::matcher_roots(matcher, tapes)
        }

        fn wave_roots(&self, wave: &LoweredWave, tapes: &[TapeDoc], roots: crate::code_rule_dsl::transducer::packed::device::ClauseRoots<'_>) -> Result<(Vec<Vec<BTreeSet<u32>>>, Outcome), Error> {
            matcher_metal::wave_roots(wave, tapes, roots)
        }

        fn wave_labelled(&self, wave: &LoweredWave, tapes: &[TapeDoc], active: &[Vec<bool>], roots: Option<&RootJobs<'_>>) -> Result<(Vec<TapeDoc>, Outcome), Error> {
            matcher_metal::wave_labelled(wave, tapes, active, roots)
        }

        fn wave_pattern_sources(&self, wave: &LoweredWave, format: u32) -> Result<Vec<String>, Error> {
            matcher_metal::wave_pattern_sources(wave, format)
        }

        fn pattern_variant(&self, format: u32) -> usize {
            crate::code_rule_dsl::transducer::packed::metal_program_backend::label_passes::pattern_variant(format)
        }

        fn patterns_ready(&self, sources: &[String], format: u32) -> bool {
            crate::code_rule_dsl::transducer::packed::metal_program_backend::label_passes::patterns_ready(sources, format)
        }

        fn warm_labels(&self) {
            matcher_metal::warm();
        }

        fn kernels_ahead(&self) -> Vec<crate::code_rule_dsl::transducer::packed::device::KernelAhead> {
            let mut v = crate::code_rule_dsl::transducer::packed::metal_join::kernels_ahead();
            v.extend(programs::kernels_ahead());
            v
        }

        fn drop_label_pipelines(&self) {
            programs::label_passes::drop_pipelines();
        }
        fn span_ends(&self, natives: &[&rerail_compiler::block_native::BlockNative], batch: &[crate::code_rule_dsl::transducer::packed::block::BatchFile<'_>]) -> Result<Option<Vec<Vec<u64>>>, Error> {
            use programs::block_program::{ClassStrategy, Executor, LaneWidth, generated_ready, run_split};
            // Each set of patterns is a kernel of its own: until its compile (on a thread of its own) is done, the
            // interpreter's pipeline runs the batch, so no launch waits on the compiler.
            let executor = if generated_ready(natives, LaneWidth::W64) { Executor::Generated(ClassStrategy::Lookup) } else { Executor::Interpreter };
            // A form Metal lacks is refused before anything runs, and recorded (`metal-form`, decision 0202): the cores'.
            // Every other error is the run's (decision 0382).
            match run_split(natives, batch, LaneWidth::W64, executor, 64) {
                Ok(run) => Ok(Some(run.files.into_iter().map(|mut r| r.outputs.swap_remove(0)).collect())),
                Err(Error::UnsupportedBackend) => Ok(None),
                Err(error) => Err(error),
            }
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
