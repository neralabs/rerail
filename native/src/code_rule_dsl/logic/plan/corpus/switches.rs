//! The engine's switches (the `RERAIL_*` variables a run reads as it goes: `RERAIL_CROSS_K4`, `RERAIL_KEPT_PART`,
//! `RERAIL_WAVE0`, …) changed only while no run reads them.
//!
//! A test that compares a mode with the default (the session tests' oracle, `RERAIL_CROSS_K4=off`) sets the
//! process's variables, which every thread's run reads. A run on another test's thread then took the mode the test
//! set for itself: `cross_file_strata_are_kept_across_runs_in_the_tape_cache` saw its first run keep its strata
//! without K4 and its second with them (O-kept-strata-flake). So each run holds `reading` from its start to its end,
//! and a test sets switches through `set`, which waits until no run on another thread holds them and keeps new runs
//! from starting until it drops (restoring each variable as it was).
//!
//! A run nested in one this thread holds (phase 0's, a phase's, the constants') and the runs of a thread that holds
//! `set` take nothing: they never wait for themselves.

use std::cell::Cell;
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

thread_local! {
    /// How deep this thread is in runs (and in `set`): only the outermost takes the switches.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Held by a run while it reads the switches.
pub struct Reading {
    outermost: bool,
}

/// The switches held for a run: no test changes them until it drops.
pub fn reading() -> Reading {
    let outermost = DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get() == 1
    });
    if outermost {
        let mut s = state();
        while s.writer || s.waiting > 0 {
            s = CHANGED.wait(s).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        s.readers += 1;
    }
    Reading { outermost }
}

impl Drop for Reading {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
        if self.outermost {
            state().readers -= 1;
            CHANGED.notify_all();
        }
    }
}

/// Switches set for a test; each variable as it was again once dropped.
pub struct Set {
    was: Vec<(String, Option<std::ffi::OsString>)>,
    outermost: bool,
}

/// Sets each `(variable, value)` once no run on another thread reads the switches, and holds them until dropped.
pub fn set(vars: &[(&str, &str)]) -> Set {
    let outermost = DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get() == 1
    });
    if outermost {
        let mut s = state();
        s.waiting += 1;
        while s.writer || s.readers > 0 {
            s = CHANGED.wait(s).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        s.waiting -= 1;
        s.writer = true;
    }
    let was = vars.iter().map(|(k, _)| ((*k).to_owned(), std::env::var_os(k))).collect();
    // SAFETY: no run on another thread reads the variables while this is held (`reading`), and the tests that set
    // them hold this.
    unsafe { vars.iter().for_each(|(k, v)| std::env::set_var(k, v)) };
    Set { was, outermost }
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
        DEPTH.with(|d| d.set(d.get() - 1));
        if self.outermost {
            state().writer = false;
            CHANGED.notify_all();
        }
    }
}
