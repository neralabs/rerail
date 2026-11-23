//! Compiled programs on demand. `CpuJit` and `MetalJit` compile a batch's programs before they
//! run it; `CpuBackground` and `MetalBackground` ask for them in the background instead, and run
//! each batch on whichever is ready: the compiled programs once their compilation has finished,
//! the backend's interpreter until then. Metal's interpreter is the fallback, so a Metal
//! compilation always builds it (and the label passes' sweep kernel, which is the interpreter
//! with marks) before the programs' own kernel.
//!
//! A request is per plan and binding (`PlanId`, list step 16: the interpreter and the compiled
//! programs bind one plan): asked again while it runs it is not started twice, once done its plan
//! runs compiled, and once it failed the plan stays on the interpreter.

use super::{Error, ExecutionBackend, Program};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// A plan's identity (list step 16; 03 §3 "Interpreted, then JIT"): one digest of what a batch
/// runs, whatever binds it. The interpreter and the compiled programs are two bindings of the same
/// plan, so their requests and their readiness are keyed by it, per binding.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PlanId(pub [u8; 32]);

impl PlanId {
    /// A batch of programs' plan: their digests in order (`Program::plain_digest`, which holds
    /// set mode).
    pub fn of_programs(programs: &[Program]) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"rerail-plan-programs-v1\0");
        for program in programs {
            digest.update(program.plain_digest().as_bytes());
            digest.update(b"\0");
        }
        Self(digest.finalize().into())
    }

    /// The plan of a part of `self` (a wave's programs `start..end`, as a batch of its roots runs).
    pub fn part(&self, start: usize, end: usize) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"rerail-plan-part-v1\0");
        digest.update(self.0);
        digest.update((start as u64).to_le_bytes());
        digest.update((end as u64).to_le_bytes());
        Self(digest.finalize().into())
    }

    pub(in crate::code_rule_dsl) fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// A compiled binding of a plan.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Binding {
    CpuJit,
    MetalJit,
}

impl Binding {
    /// Its kind of form in the record of forms (`rerail_accel::forms`, decision 0202).
    fn form(self) -> &'static str {
        match self {
            Binding::CpuJit => "cpu-jit",
            Binding::MetalJit => "metal-jit",
        }
    }
}

#[derive(Default)]
struct Requests {
    running: BTreeSet<(Binding, PlanId)>,
    ready: BTreeSet<(Binding, PlanId)>,
    /// A form the binding has no code for (`UnsupportedBackend`): the plan's batches stay interpreted.
    unsupported: BTreeSet<(Binding, PlanId)>,
    /// A compile that failed, with its message: the plan's next batch stops the run with it (decision 0382).
    failed: BTreeMap<(Binding, PlanId), String>,
}

/// The compile queue's requests, by the caller's programs (decision 0442: the shared object's).
#[derive(Default)]
struct Queue(Mutex<Requests>);

fn queue() -> std::sync::Arc<Queue> {
    crate::code_rule_dsl::logic::profile::shared::<Queue>()
}

/// Whether `plan`'s `binding` is compiled (its request finished). A compile of it that failed stops the run here,
/// on the thread that asks (decision 0382): a failed compile is never answered by interpreting for good.
fn ready(binding: Binding, plan: PlanId) -> bool {
    let (ready, failed) = match queue().0.lock() {
        Ok(requests) => (requests.ready.contains(&(binding, plan)), requests.failed.get(&(binding, plan)).cloned()),
        Err(_) => return false,
    };
    // Out of the lock, so the panic leaves it whole for the other threads.
    if let Some(message) = failed {
        panic!("rerail: the {} compile of plan {} failed: {message}", binding.form(), plan.hex());
    }
    ready
}

/// Starts `compile` as a job on the pool unless a request for (`binding`, `plan`) runs, finished
/// or failed.
fn request(binding: Binding, plan: PlanId, compile: impl FnOnce() -> Result<(), Error> + Send + 'static) {
    let kind = binding.form();
    let held = queue();
    let Ok(mut requests) = held.0.lock() else { return };
    let key = (binding, plan);
    if requests.running.contains(&key) || requests.ready.contains(&key) || requests.unsupported.contains(&key) || requests.failed.contains_key(&key) {
        return;
    }
    crate::code_rule_dsl::prof::count("compilation.requests", 1);
    requests.running.insert(key);
    // The job compiles on the caller's shared object, and marks its request in the caller's queue: a pool thread
    // holds no run.
    let (shared, held) = (crate::code_rule_dsl::logic::profile::shared_here(), std::sync::Arc::clone(&held));
    crate::code_rule_dsl::logic::profile::cores_spawn(move || {
        let _in_run = crate::code_rule_dsl::logic::profile::within(shared);
        // A panic is a failed compilation, kept with its message for the plan's next batch, which stops the run
        // with it (`ready`): a job's own panic would end the process from the pool, away from the batch.
        let started = std::time::Instant::now();
        let done = rerail_accel::exec::as_job("a compilation", || std::panic::catch_unwind(std::panic::AssertUnwindSafe(compile)))
            .map_err(|panic| panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned())).unwrap_or_else(|| "a panic".into()));
        let done = match done {
            Ok(Ok(())) => Ok(()),
            Ok(Err(Error::UnsupportedBackend)) => Err(None),
            Ok(Err(error)) => Err(Some(format!("{error:?}"))),
            Err(message) => Err(Some(message)),
        };
        rerail_accel::forms::compiled(kind, &plan.hex(), "", started.elapsed().as_secs_f64(), done.is_ok());
        if let Ok(mut requests) = held.0.lock() {
            requests.running.remove(&key);
            match done {
                Ok(()) => {
                    requests.ready.insert(key);
                }
                Err(None) => {
                    requests.unsupported.insert(key);
                }
                Err(Some(message)) => {
                    requests.failed.insert(key, message);
                }
            }
        }
    });
}

/// The backend a batch of `programs` runs on, as `resolve_plan` under their own plan.
pub fn resolve(backend: ExecutionBackend, programs: &[Program]) -> Result<ExecutionBackend, Error> {
    resolve_plan(backend, PlanId::of_programs(programs), programs)
}

/// The backend a batch of `programs`, plan `plan`, runs on: a background backend's compiled
/// binding once `plan`'s is ready, else its interpreter, with the compiled binding requested (the
/// swap is at the next batch of the plan). Others are themselves.
pub fn resolve_plan(backend: ExecutionBackend, plan: PlanId, programs: &[Program]) -> Result<ExecutionBackend, Error> {
    let resolved = match backend {
        ExecutionBackend::CpuBackground => {
            if ready(Binding::CpuJit, plan) || super::cpu()?.compiled(programs)? {
                ExecutionBackend::CpuJit
            } else {
                let owned = programs.to_vec();
                let cpu = super::cpu()?;
                request(Binding::CpuJit, plan, move || owned.iter().try_for_each(|program| cpu.compile(program)));
                ExecutionBackend::CpuInterpreter
            }
        }
        ExecutionBackend::MetalBackground => {
            let metal = super::device::required()?;
            if ready(Binding::MetalJit, plan) || metal.jit_ready(programs)? {
                ExecutionBackend::MetalJit
            } else {
                // The fallback runs now, so it is built now (once per process).
                metal.compile_interpreter()?;
                let owned = programs.to_vec();
                request(Binding::MetalJit, plan, move || metal.compile_background(&owned));
                ExecutionBackend::MetalInterpreter
            }
        }
        other => return Ok(other),
    };
    if matches!(resolved, ExecutionBackend::CpuInterpreter | ExecutionBackend::MetalInterpreter) {
        let kind = if resolved == ExecutionBackend::CpuInterpreter { Binding::CpuJit } else { Binding::MetalJit }.form();
        rerail_accel::forms::interpreted(kind, &plan.hex(), &format!("{} programs", programs.len()), 1);
    }
    crate::code_rule_dsl::prof::count(
        match resolved {
            ExecutionBackend::CpuJit | ExecutionBackend::MetalJit => "compilation.batches_compiled",
            _ => "compilation.batches_interpreted",
        },
        1,
    );
    Ok(resolved)
}

/// Waits for every running compilation (tests: the batch after it runs compiled).
#[cfg(any(test, feature = "test-hooks"))]
pub fn settle() {
    rerail_accel::exec::help_until(|| queue().0.lock().map_or(true, |requests| requests.running.is_empty()).then_some(()));
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
