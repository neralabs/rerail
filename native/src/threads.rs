//! The most threads the engine's process uses at once: `RERAIL_THREADS`, else the machine's cores.
//!
//! ```text
//! RERAIL_THREADS=4 rerail-engine logic …    every job of the run on one pool of 4 threads, the main thread one
//!                                           of them: the rules, the documents read, parsed and made facts, the
//!                                           chunks cut and projected, the cores' launches
//! ```
//!
//! A machine has one pool (decision 0442: `rerail_context::Cores`, made on its first job), so every parallel loop and
//! job of a caller of the machine runs on it beside the executor's launches; two machines in a process have two.

/// The setting: `RERAIL_THREADS`, a whole number of at least 1; else the cores the machine offers.
pub fn max() -> usize {
    rerail_accel::exec::threads()
}

/// Registers what every block is made by before the first block (`rerail_accel::exec::configure`): the engine's
/// `main`, Node's registration of the addon (`lib.rs`), a run. The pool is the machine's, sized to `max`; the
/// engine's `main` makes its machine on the main thread (`Machine::here`), so that thread is one of the pool's.
pub fn configure() {
    rerail_accel::exec::configure();
    // Every job of the pool keeps its device wave apart from its thread's (decision 0386).
    crate::code_rule_dsl::logic::lanes::wave::install_job_scope();
}

/// Whether this is the process's main thread.
pub use crate::code_rule_dsl::logic::profile::is_main_thread;

/// The machine whose pool a caller's jobs run on (decision 0442): a program makes one (`Machine::here` on its main
/// thread) and hands it to its commands.
pub use crate::code_rule_dsl::logic::profile::Machine;

/// Jobs on the caller's machine's pool that borrow the caller's data, within [`scope`]: every one has ended when it returns. Each is
/// handed the scope again, to start more.
pub struct Jobs<'s, 'r>(
    #[cfg(not(target_family = "wasm"))] &'r rayon::Scope<'s>,
    #[cfg(target_family = "wasm")] std::marker::PhantomData<(&'s (), &'r ())>,
);

impl<'s> Jobs<'s, '_> {
    /// Starts `f` as the job `name` on the pool (wasm32 has no threads: it runs here). A wait that runs it for
    /// another knows it by its name (`rerail_accel::exec::waits`).
    pub fn spawn(&self, name: &'static str, f: impl for<'x> FnOnce(&Jobs<'s, 'x>) + Send + 's) {
        #[cfg(not(target_family = "wasm"))]
        {
            // The job runs in the run's context where it was started (`run_state`), and its span is under the one
            // open there, on whichever thread runs it.
            let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
            spawn_in(self.0, name, carry, f);
        }
        #[cfg(target_family = "wasm")]
        let _ = name;
        #[cfg(target_family = "wasm")]
        f(self);
    }
}

/// Starts `f` as the job `name` in the scope `s`. Popped by a wait whose work is expected sooner than the job would
/// end (`rerail_accel::exec::declines`), it is started again in its scope, for a thread that is not waiting.
#[cfg(not(target_family = "wasm"))]
fn spawn_in<'s, F>(s: &rayon::Scope<'s>, name: &'static str, carry: crate::code_rule_dsl::logic::run_state::Carry, f: F)
where
    F: for<'x> FnOnce(&Jobs<'s, 'x>) + Send + 's,
{
    s.spawn(move |s| {
        if rerail_accel::exec::declines(name, 0.0) {
            return spawn_in(s, name, carry, f);
        }
        let _in = carry.enter();
        let _span = crate::code_rule_dsl::logic::profile::span_at(carry.under(), || format!("{name} (job)"));
        rerail_accel::exec::as_job(name, || f(&Jobs(s)))
    });
}

/// Runs `op` on this thread with a scope for jobs that borrow its data; returns once they have all ended (meanwhile,
/// on one of the pool's threads, it runs the pool's jobs).
pub fn scope<'s, R>(op: impl for<'r> FnOnce(&Jobs<'s, 'r>) -> R) -> R {
    #[cfg(not(target_family = "wasm"))]
    {
        crate::code_rule_dsl::logic::profile::cores_scope(|s| op(&Jobs(s)))
    }
    #[cfg(target_family = "wasm")]
    {
        op(&Jobs(std::marker::PhantomData))
    }
}

/// A job that takes turns with its caller (a request and its answers): it waits for the caller, so the caller's
/// waits must never run it on the caller's own stack. A pool worker or a service thread owns the job before the
/// caller can help with other work.
pub(crate) struct Beside<T> {
    name: &'static str,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::thread::Result<T>>>>,
}

/// Frees `value` on the pool, off the caller's thread: freeing a large structure (a program analysis's results for
/// every rule) is work the caller does not need done before it goes on. It reads no run context, so it carries none.
pub(crate) fn drop_behind<T: Send + 'static>(value: T) {
    crate::code_rule_dsl::logic::profile::cores_spawn(move || drop(value));
}

/// Starts `f` beside the caller ([`Beside`]).
pub(crate) fn beside<T: Send + 'static>(name: &'static str, f: impl FnOnce() -> T + Send + 'static) -> Beside<T> {
    let slot = std::sync::Arc::new(parking_lot::Mutex::new(None));
    let held = std::sync::Arc::clone(&slot);
    let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
    let run = move || {
        let _in = carry.enter();
        let _span = crate::code_rule_dsl::logic::profile::span_at(carry.under(), || format!("{name} (job)"));
        let r = rerail_accel::exec::as_job(name, || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        *held.lock() = Some(r);
    };
    let unclaimed = if cfg!(not(target_family = "wasm")) && max() >= 2 {
        let pending = std::sync::Arc::new(parking_lot::Mutex::new(Some(run)));
        let queued = std::sync::Arc::clone(&pending);
        let (claimed, ready) = std::sync::mpsc::channel();
        crate::code_rule_dsl::logic::profile::cores_spawn(move || {
            let job = queued.lock().take();
            if let Some(job) = job {
                let _ = claimed.send(());
                job();
            }
        });
        // Each available worker could itself be starting a request/answer pair.
        // Give another worker first claim, without depending on one becoming free.
        let _ = ready.recv_timeout(std::time::Duration::from_millis(1));
        pending.lock().take()
    } else {
        Some(run)
    };
    if let Some(job) = unclaimed && let Err(error) = rerail_accel::exec::service(name, None, job) {
        *slot.lock() = Some(Err(Box::new(error)));
    }
    Beside { name, slot }
}

impl<T> Beside<T> {
    /// Waits until `poll` gives a value, running queued jobs and recording the wait under `site`.
    pub(crate) fn wait_for<V>(&self, site: &'static str, poll: impl FnMut() -> Option<V>) -> V {
        rerail_accel::exec::help_until_as(site, poll)
    }

    /// Its result, once it ends (a panic as `Err`).
    pub(crate) fn join(self) -> std::thread::Result<T> {
        self.wait_for(self.name, || self.slot.lock().take())
    }
}

#[cfg(test)]
pub mod tests {
    /// The setting is at least one thread.
    #[test]
    fn the_setting_is_at_least_one_thread() {
        assert!(super::max() >= 1);
    }

    /// Jobs that start jobs, all on the pool, all ended when the scope returns; a wait for them runs them.
    #[test]
    fn scoped_jobs_start_jobs_and_end_with_the_scope() {
        let n = std::sync::atomic::AtomicUsize::new(0);
        super::scope(|jobs| {
            for _ in 0..8 {
                jobs.spawn("a test's job", |jobs| {
                    n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    jobs.spawn("a test's job", |_| {
                        n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    });
                });
            }
            rerail_accel::exec::help_until(|| (n.load(std::sync::atomic::Ordering::SeqCst) == 16).then_some(()));
        });
        assert_eq!(n.into_inner(), 16);
    }

    /// Q203 item 5: a wait whose work is expected by a deadline (a batch's device launches) runs only jobs expected
    /// to end by then. A job known to take longer, popped by it, goes back to its scope for another thread, so the
    /// wait ends when its work is ready, not when the long job does; a short job still runs in the wait.
    #[test]
    fn a_long_job_never_delays_a_finished_batchs_continuation() {
        use std::time::{Duration, Instant};
        use rerail_accel::exec::{as_job, Waiting};
        // Their expected walls, from a run each.
        as_job("a test's long job", || std::thread::sleep(Duration::from_millis(150)));
        as_job("a test's short job", || ());
        rayon::scope(|_| {
            super::scope(|jobs| {
                jobs.spawn("a test's long job", |_| std::thread::sleep(Duration::from_millis(150)));
                // The batch's work is ready 20 ms from now.
                let ready = Instant::now() + Duration::from_millis(20);
                let mut wait = Waiting::new("a test's batch");
                while Instant::now() < ready {
                    if !wait.help_within(ready) {
                        std::thread::sleep(Duration::from_micros(100));
                    }
                }
                let late = Instant::now().saturating_duration_since(ready);
                wait.end(None);
                assert!(late < Duration::from_millis(50), "the batch's continuation {late:?} late: the long job ran in its wait");
                let ran = std::sync::atomic::AtomicBool::new(false);
                super::scope(|jobs| {
                    // A job that runs in the wait has no deadline of its own: a long job its own wait meets runs.
                    jobs.spawn("a test's short job", |_| {
                        assert!(!rerail_accel::exec::declines("a test's long job", 0.0), "a deadline carried into a job run in the wait");
                        ran.store(true, std::sync::atomic::Ordering::SeqCst);
                    });
                    let mut wait = Waiting::new("a test's batch");
                    let by = Instant::now() + Duration::from_millis(50);
                    while !ran.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < by {
                        wait.help_within(by);
                    }
                    wait.end(None);
                });
                assert!(ran.into_inner(), "a short job runs");
            });
        });
    }

    /// A job beside its caller takes turns with it: it asks, the caller answers, and neither runs the other.
    #[test]
    fn a_job_beside_its_caller_takes_turns_with_it() {
        let (ask, asked) = std::sync::mpsc::channel::<u32>();
        let (answer, answered) = std::sync::mpsc::channel::<u32>();
        let job = super::beside("a test's turns", move || {
            let mut sum = 0;
            for i in 0..20 {
                ask.send(i).unwrap();
                sum += rerail_accel::exec::help_until(|| answered.try_recv().ok());
            }
            sum
        });
        while let Some(i) = job.wait_for("a test's caller", || match asked.try_recv() {
            Ok(i) => Some(Some(i)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
        }) {
            answer.send(i * 2).unwrap();
        }
        assert_eq!(job.join().unwrap(), (0..20).map(|i| i * 2).sum::<u32>());
    }
}
