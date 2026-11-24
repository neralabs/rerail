//! The engine's switches (the `RERAIL_*` variables a run reads as it goes: `RERAIL_CROSS_K4`, `RERAIL_KEPT_PART`,
//! `RERAIL_WAVE0`, …) changed only while no run reads them.
//!
//! A test that compares a mode with the default (the session tests' oracle, `RERAIL_CROSS_K4=off`) sets it for its
//! own runs (`set`'s `SCOPED`, bound in its context and read through `profile::var`): no other test's run sees it or
//! waits. The rest it sets in the process's variables, which every thread's run reads. A run on another test's thread then took the mode the test
//! set for itself: `cross_file_strata_are_kept_across_runs_in_the_tape_cache` saw its first run keep its strata
//! without K4 and its second with them (O-kept-strata-flake). So each run holds `reading` from its start to its end,
//! and a test sets switches through `set`, which waits until no run on another thread holds them and keeps new runs
//! from starting until it drops (restoring each variable as it was).
//!
//! A run nested in one this thread holds (phase 0's, a phase's, the constants') and the runs of a thread that holds
//! `set` take nothing: they never wait for themselves.
//!
//! A process a test starts (this test binary again, its runs in a process of their own) inherits the variables as they
//! are when it starts, and keeps them for its life: so a test starts one through `started`, which holds `reading` while
//! it does. Started while `cross_file_rows_are_memoised_per_import_component` held `RERAIL_CROSS_MEMO=verify` or
//! `RERAIL_INLINE_ONLY=near`, `a_second_run_in_one_process_records_the_first_runs_manifest`'s process of two ran
//! another plan, and recorded two manifests of its own; started while a session test held `RERAIL_CROSS_K4=off`,
//! `the_run_after_a_cold_one_writes_nothing`'s warm run wrote.

use std::sync::{Condvar, Mutex};

#[derive(Default)]
struct State {
    /// Threads in a run.
    readers: usize,
    /// A thread holds `set`.
    writer: bool,
    /// Threads waiting to `set`: no run starts meanwhile, so one starts setting once the runs end.
    waiting: usize,
}

static STATE: Mutex<State> = Mutex::new(State { readers: 0, writer: false, waiting: 0 });
static CHANGED: Condvar = Condvar::new();

/// How deep the run's context is in runs (and in `set`): only the outermost takes the switches. A job's run is as deep
/// as its caller's (decision 0442: the context goes with the job), so a run there does not take them again.
#[derive(Clone, Copy, Default)]
struct Depth(usize);

/// One level deeper until the guard drops, and whether it is the outermost.
fn deeper() -> (crate::code_rule_dsl::logic::profile::Bound, bool) {
    let d = crate::code_rule_dsl::logic::profile::bound::<Depth>().0 + 1;
    (crate::code_rule_dsl::logic::profile::bind(Depth(d)), d == 1)
}

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a thread does before it takes the switches, holding nothing: in a test build, the tests' store template is made
/// (`test_store::ready`). Made later, from inside a run, its runs would wait behind a test waiting to `set`, which
/// waits for the runs that wait on the template: none would end.
fn before_taking() {
    #[cfg(any(test, feature = "test-hooks"))]
    super::test_store::ready();
}

/// Held by a run while it reads the switches.
pub struct Reading {
    outermost: bool,
    _depth: crate::code_rule_dsl::logic::profile::Bound,
}

/// The switches held for a run: no test changes them until it drops.
pub fn reading() -> Reading {
    let (depth, outermost) = deeper();
    if outermost {
        before_taking();
        let mut s = state();
        while s.writer || s.waiting > 0 {
            s = CHANGED.wait(s).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        s.readers += 1;
    }
    Reading { outermost, _depth: depth }
}

impl Drop for Reading {
    fn drop(&mut self) {
        if self.outermost {
            state().readers -= 1;
            CHANGED.notify_all();
        }
    }
}

/// Starts a process of the engine for a test (`start`: a `Command`'s `output`, `status` or `spawn`), the switches held
/// as a run holds them (`reading`): it inherits the variables no other test is changing.
pub fn started<T>(start: impl FnOnce() -> T) -> T {
    let _reading = reading();
    start()
}

/// The switches a test sets for its own runs (`set`), each read through `profile::var` (or `var_os`, `vars`): bound in
/// the test's context, which its runs and their jobs read, so no other test's run waits for them or sees them.
const SCOPED: &[&str] = &[
    "RERAIL_KEPT_PART", "RERAIL_CROSS_K4", "RERAIL_CROSS_DELTA_COUNTS", "RERAIL_DEVICE_BUDGET", "RERAIL_AGAIN", "RERAIL_AGAIN_MIN",
    "RERAIL_AGAIN_THETA", "RERAIL_WAVE0", "RERAIL_COPY_VIEWS", "RERAIL_ANCHORS", "RERAIL_SETS", "RERAIL_INLINE_ONLY", "RERAIL_AST_PARTS",
    "RERAIL_ASOF", "RERAIL_NODE_SOURCES", "RERAIL_ORDER_RAW", "RERAIL_NODE_IDS", "RERAIL_RACY_MS",
    "RERAIL_STACK_ROUNDS", "RERAIL_STACK_PHASES", "RERAIL_SCAN_OWN_TILES", "RERAIL_PROGRAM_ARENA", "RERAIL_ROOMS_MAPPED",
    "RERAIL_AST_DIRECT_CAPTURES", "RERAIL_CHUNK_COUNTS",
];

/// The switches read off a run's context too (the store's path; the profile's and the heap sampler's, which their own
/// threads read): set in the process's environment, once no run on another thread reads the switches (`reading`).
const PROCESS: &[&str] = &[
    "RERAIL_TAPE_CACHE", "RERAIL_HEAP_SAMPLE", "RERAIL_HEAP_AT", "RERAIL_HEAP_FRAMES", "RERAIL_PROF_MEMORY_MS", "RERAIL_TIMELINE",
    "RERAIL_COSTS", "RERAIL_IMAGE_STORE", "RERAIL_FRAMES_TRACE",
];

/// Switches set for a test; each as it was again once dropped.
pub struct Set {
    was: Vec<(String, Option<std::ffi::OsString>)>,
    writer: bool,
    // Before `_depth`: bound after it, so dropped before it (guards drop in the reverse of their order).
    _switches: Option<crate::code_rule_dsl::logic::profile::Bound>,
    _depth: Option<crate::code_rule_dsl::logic::profile::Bound>,
}

/// Sets each `(variable, value)`: a switch of the test's runs (`SCOPED`) in its context, at once; a switch of the
/// process (`PROCESS`) in its environment, once no run on another thread reads the switches, holding them until
/// dropped. A switch on neither list is refused: read as the process's variable, a test's value would reach every
/// test's runs; set in the context alone, the read that takes no context would not see it.
pub fn set(vars: &[(&str, &str)]) -> Set {
    for (k, _) in vars {
        assert!(SCOPED.contains(k) || PROCESS.contains(k), "switches::set: {k} is on neither list: read it through `profile::var` and list it in SCOPED, or list it in PROCESS");
    }
    let (process, scoped): (Vec<&(&str, &str)>, Vec<&(&str, &str)>) = vars.iter().partition(|(k, _)| PROCESS.contains(k));
    let (mut depth, mut writer) = (None, false);
    if !process.is_empty() {
        let (d, outermost) = deeper();
        depth = Some(d);
        if outermost {
            before_taking();
            let mut s = state();
            s.waiting += 1;
            while s.writer || s.readers > 0 {
                s = CHANGED.wait(s).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            s.waiting -= 1;
            s.writer = true;
            writer = true;
        }
    }
    let was = process.iter().map(|(k, _)| ((*k).to_owned(), std::env::var_os(k))).collect();
    // SAFETY: no run on another thread reads the variables while this is held (`reading`), and the tests that set
    // them hold this.
    unsafe { process.iter().for_each(|(k, v)| std::env::set_var(k, v)) };
    let switches = (!scoped.is_empty()).then(|| {
        let mut now = (*crate::code_rule_dsl::logic::profile::bound::<crate::code_rule_dsl::logic::profile::Switches>().0).clone();
        now.extend(scoped.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        crate::code_rule_dsl::logic::profile::bind(crate::code_rule_dsl::logic::profile::Switches(std::sync::Arc::new(now)))
    });
    Set { was, writer, _switches: switches, _depth: depth }
}

impl Drop for Set {
    fn drop(&mut self) {
        for (k, v) in self.was.drain(..).rev() {
            // SAFETY: as in `set`.
            unsafe {
                match v {
                    Some(v) => std::env::set_var(&k, v),
                    None => std::env::remove_var(&k),
                }
            }
        }
        if self.writer {
            state().writer = false;
            CHANGED.notify_all();
        }
    }
}
