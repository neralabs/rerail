//! The most threads the engine's process uses at once: `RERAIL_THREADS`, else the machine's cores.
//!
//! ```text
//! RERAIL_THREADS=4 rerail-engine logic …    every job of the run on one pool of 4 threads, the main thread one
//!                                           of them: the rules, the documents read, parsed and made facts, the
//!                                           chunks cut and projected, the cores' launches
//! ```
//!
//! There is one pool: the executor's (`rerail_accel::exec::configure`), which is rayon's global pool, so every
//! parallel loop and job of the engine runs on it beside the executor's launches.

/// The setting: `RERAIL_THREADS`, a whole number of at least 1; else the cores the machine offers.
pub fn max() -> usize {
    rerail_accel::exec::threads()
}

/// Makes the one pool, sized to `max`: called before anything uses it (the engine's `main`; Node's registration of
/// the addon, `lib.rs`, never its constructor). On the process's main thread it is called with `this_thread`, so that
/// thread is one of the pool's (its own work and its waits are the pool's). A pool already made is left as it is. Every
/// job is fenced off the device wave of the job it may run inside (`lanes::wave::fence`).
pub fn configure(this_thread: bool) {
    rerail_accel::exec::fence_jobs(crate::code_rule_dsl::logic::lanes::wave::fence, crate::code_rule_dsl::logic::lanes::wave::unfence);
    rerail_accel::exec::configure(this_thread);
}

/// Jobs on the one pool that borrow the caller's data, within [`scope`]: every one has ended when it returns. Each is
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
            self.0.spawn(move |s| {
                let _in = carry.enter();
                let _span = crate::code_rule_dsl::logic::profile::span_at(carry.under(), || format!("{name} (job)"));
                rerail_accel::exec::as_job(name, || f(&Jobs(s)))
            });
        }
        #[cfg(target_family = "wasm")]
        let _ = name;
        #[cfg(target_family = "wasm")]
        f(self);
    }
}

/// Runs `op` on this thread with a scope for jobs that borrow its data; returns once they have all ended (meanwhile,
/// on one of the pool's threads, it runs the pool's jobs).
pub fn scope<'s, R>(op: impl for<'r> FnOnce(&Jobs<'s, 'r>) -> R) -> R {
    #[cfg(not(target_family = "wasm"))]
    {
        rayon::in_place_scope(|s| op(&Jobs(s)))
    }
    #[cfg(target_family = "wasm")]
    {
        op(&Jobs(std::marker::PhantomData))
    }
}

/// A job that takes turns with its caller (a request and its answers): it waits for the caller, so the caller's
/// waits must never run it on the caller's own stack, and it may not hold one of the pool's threads (decision 0100
/// §12: nothing that waits for another long job may itself be a job). It runs on a thread of its own: on the pool,
/// its waits for the caller's answers held a thread the caller's chunks needed, and with 2 threads every run with a
/// cone of requests stalled (the caller waited for chunks no thread was free to build). Its own parallel work still
/// runs on the pool.
pub(crate) struct Beside<T> {
    name: &'static str,
    slot: std::sync::Arc<parking_lot::Mutex<Option<std::thread::Result<T>>>>,
}

/// Frees `value` on the pool, off the caller's thread: freeing a large structure (a program analysis's results for
/// every rule) is work the caller does not need done before it goes on. It reads no run context, so it carries none.
pub(crate) fn drop_behind<T: Send + 'static>(value: T) {
    rayon::spawn(move || drop(value));
}

/// Starts `f` beside the caller ([`Beside`]), on a stack the size of a pool thread's.
pub(crate) fn beside<T: Send + 'static>(name: &'static str, f: impl FnOnce() -> T + Send + 'static) -> Beside<T> {
    beside_on(name, rerail_accel::exec::POOL_STACK, f)
}

/// [`beside`], on a thread with `stack` bytes of stack. A thread that cannot be started is the job's result, an `Err`
/// that names it, so [`Beside::join`] reports it: nothing else would ever end its wait. It is not run on the caller's
/// thread instead: it waits for the caller, so there it would never end.
fn beside_on<T: Send + 'static>(name: &'static str, stack: usize, f: impl FnOnce() -> T + Send + 'static) -> Beside<T> {
    let slot = std::sync::Arc::new(parking_lot::Mutex::new(None));
    let held = std::sync::Arc::clone(&slot);
    let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
    let started = rerail_accel::exec::service(name, Some(stack), move || {
        let _in = carry.enter();
        let _span = crate::code_rule_dsl::logic::profile::span_at(carry.under(), || format!("{name} (job)"));
        let r = rerail_accel::exec::as_job(name, || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        *held.lock() = Some(r);
    });
    if let Err(e) = started {
        *slot.lock() = Some(Err(Box::new(format!("cannot start {name}: {e}"))));
    }
    Beside { name, slot }
}

impl<T> Beside<T> {
    /// Waits until `poll` gives a value, the wait recorded under `site`, running the pool's other jobs meanwhile: the
    /// job is on a thread of its own, so the wait never takes it.
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

    /// A job whose thread cannot be started (a stack larger than the address space) ends at once, as an `Err` that
    /// names it: its caller's wait for it ends.
    #[test]
    fn a_job_beside_that_cannot_start_is_an_error() {
        let job = super::beside_on("a test's unstartable job", 1 << 60, || ());
        assert!(job.slot.lock().is_some(), "the failed start is the job's result");
        let e = job.join().expect_err("a job that never started");
        assert!(e.downcast_ref::<String>().is_some_and(|m| m.starts_with("cannot start a test's unstartable job: ")));
    }
}
