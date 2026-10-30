//! Compiled programs on demand. `CpuJit` and `MetalJit` compile a batch's programs before they
//! run it; `CpuBackground` and `MetalBackground` ask for them in the background instead, and run
//! each batch on whichever is ready: the compiled programs once their compilation has finished,
//! the backend's interpreter until then. Metal's interpreter is the fallback, so a Metal
//! compilation always builds it (and the label passes' sweep kernel, which is the interpreter
//! with marks) before the programs' own kernel.
//!
//! A request is per set of programs (its compiled form's cache key): asked again while it runs it
//! is not started twice, and once it failed the batch stays on the interpreter.

use super::{Error, ExecutionBackend, Program, native_cpu};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex};
use std::thread::JoinHandle;

#[derive(Default)]
struct Requests {
    running: BTreeMap<String, JoinHandle<()>>,
    failed: BTreeSet<String>,
}

static REQUESTS: LazyLock<Mutex<Requests>> = LazyLock::new(Mutex::default);

/// Starts `compile` on its own thread unless a request for `key` runs or failed.
fn request(key: String, compile: impl FnOnce() -> Result<(), Error> + Send + 'static) {
    let Ok(mut requests) = REQUESTS.lock() else { return };
    // A finished request's compiled form is in its backend's cache (or it failed).
    requests.running.retain(|_, handle| !handle.is_finished());
    if requests.running.contains_key(&key) || requests.failed.contains(&key) {
        return;
    }
    let name = key.clone();
    let handle = std::thread::Builder::new().name("rerail-compile".to_owned()).spawn(move || {
        if compile().is_err()
            && let Ok(mut requests) = REQUESTS.lock()
        {
            requests.failed.insert(name);
        }
    });
    if let Ok(handle) = handle {
        requests.running.insert(key, handle);
    }
}

/// The backend a batch of `programs` runs on: a background backend's compiled programs when they
/// are ready, else its interpreter, with their compilation requested. Others are themselves.
pub(super) fn resolve(backend: ExecutionBackend, programs: &[Program]) -> Result<ExecutionBackend, Error> {
    match backend {
        ExecutionBackend::CpuBackground => {
            if native_cpu::compiled(programs)? {
                return Ok(ExecutionBackend::CpuJit);
            }
            let owned = programs.to_vec();
            request(native_cpu::batch_key(programs), move || {
                owned.iter().try_for_each(|program| native_cpu::compile_cached(program).map(|_| ()))
            });
            Ok(ExecutionBackend::CpuInterpreter)
        }
        #[cfg(target_os = "macos")]
        ExecutionBackend::MetalBackground => {
            use super::metal_program_backend as metal;
            if metal::jit_ready(programs)? {
                return Ok(ExecutionBackend::MetalJit);
            }
            // The fallback runs now, so it is built now (once per process).
            metal::compile_interpreter_cached()?;
            let owned = programs.to_vec();
            request(metal::jit_cache_key(programs)?, move || {
                metal::compile_sweep_cached()?;
                metal::compile_jit_cached(&owned).map(|_| ())
            });
            Ok(ExecutionBackend::MetalInterpreter)
        }
        #[cfg(not(target_os = "macos"))]
        ExecutionBackend::MetalBackground => Err(Error::UnsupportedBackend),
        other => Ok(other),
    }
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
