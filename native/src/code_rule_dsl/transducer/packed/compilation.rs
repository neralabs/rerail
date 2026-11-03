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

use super::{Error, ExecutionBackend, Program, native_cpu};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex};
use std::thread::JoinHandle;

/// A plan's identity (list step 16; 03 §3 "Interpreted, then JIT"): one digest of what a batch
/// runs, whatever binds it. The interpreter and the compiled programs are two bindings of the same
/// plan, so their requests and their readiness are keyed by it, per binding.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(in crate::code_rule_dsl) struct PlanId(pub(in crate::code_rule_dsl) [u8; 32]);

impl PlanId {
    /// A batch of programs' plan: their digests in order (`Program::plain_digest`, which holds
    /// set mode).
    pub(in crate::code_rule_dsl) fn of_programs(programs: &[Program]) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"rerail-plan-programs-v1\0");
        for program in programs {
            digest.update(program.plain_digest().as_bytes());
            digest.update(b"\0");
        }
        Self(digest.finalize().into())
    }

    /// The plan of a part of `self` (a wave's programs `start..end`, as a batch of its roots runs).
    pub(in crate::code_rule_dsl) fn part(&self, start: usize, end: usize) -> Self {
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

#[derive(Default)]
struct Requests {
    running: BTreeMap<(Binding, PlanId), JoinHandle<()>>,
    ready: BTreeSet<(Binding, PlanId)>,
    failed: BTreeSet<(Binding, PlanId)>,
}

static REQUESTS: LazyLock<Mutex<Requests>> = LazyLock::new(Mutex::default);

/// Whether `plan`'s `binding` is compiled (its request finished).
fn ready(binding: Binding, plan: PlanId) -> bool {
    REQUESTS.lock().is_ok_and(|requests| requests.ready.contains(&(binding, plan)))
}

/// Starts `compile` on its own thread unless a request for (`binding`, `plan`) runs, finished or
/// failed.
fn request(binding: Binding, plan: PlanId, compile: impl FnOnce() -> Result<(), Error> + Send + 'static) {
    let Ok(mut requests) = REQUESTS.lock() else { return };
    let key = (binding, plan);
    requests.running.retain(|_, handle| !handle.is_finished());
    if requests.running.contains_key(&key) || requests.ready.contains(&key) || requests.failed.contains(&key) {
        return;
    }
    crate::code_rule_dsl::prof::count("compilation.requests", 1);
    let handle = std::thread::Builder::new().name("rerail-compile".to_owned()).spawn(move || {
        let done = compile();
        if let Ok(mut requests) = REQUESTS.lock() {
            if done.is_ok() {
                requests.ready.insert(key);
            } else {
                requests.failed.insert(key);
            }
        }
    });
    if let Ok(handle) = handle {
        requests.running.insert(key, handle);
    }
}

/// The backend a batch of `programs` runs on, as `resolve_plan` under their own plan.
pub(super) fn resolve(backend: ExecutionBackend, programs: &[Program]) -> Result<ExecutionBackend, Error> {
    resolve_plan(backend, PlanId::of_programs(programs), programs)
}

/// The backend a batch of `programs`, plan `plan`, runs on: a background backend's compiled
/// binding once `plan`'s is ready, else its interpreter, with the compiled binding requested (the
/// swap is at the next batch of the plan). Others are themselves.
pub(super) fn resolve_plan(backend: ExecutionBackend, plan: PlanId, programs: &[Program]) -> Result<ExecutionBackend, Error> {
    let resolved = match backend {
        ExecutionBackend::CpuBackground => {
            if ready(Binding::CpuJit, plan) || native_cpu::compiled(programs)? {
                ExecutionBackend::CpuJit
            } else {
                let owned = programs.to_vec();
                request(Binding::CpuJit, plan, move || {
                    owned.iter().try_for_each(|program| native_cpu::compile_cached(program).map(|_| ()))
                });
                ExecutionBackend::CpuInterpreter
            }
        }
        #[cfg(target_os = "macos")]
        ExecutionBackend::MetalBackground => {
            use super::metal_program_backend as metal;
            if ready(Binding::MetalJit, plan) || metal::jit_ready(programs)? {
                ExecutionBackend::MetalJit
            } else {
                // The fallback runs now, so it is built now (once per process).
                metal::compile_interpreter_cached()?;
                let owned = programs.to_vec();
                request(Binding::MetalJit, plan, move || {
                    metal::compile_sweep_cached()?;
                    metal::compile_jit_cached(&owned).map(|_| ())
                });
                ExecutionBackend::MetalInterpreter
            }
        }
        #[cfg(not(target_os = "macos"))]
        ExecutionBackend::MetalBackground => return Err(Error::UnsupportedBackend),
        other => return Ok(other),
    };
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
#[cfg(test)]
pub(super) fn settle() {
    let running = REQUESTS
        .lock()
        .map(|mut requests| std::mem::take(&mut requests.running))
        .unwrap_or_default();
    for (_, handle) in running {
        let _ = handle.join();
    }
}

#[cfg(test)]
#[path = "compilation_tests.rs"]
mod tests;
