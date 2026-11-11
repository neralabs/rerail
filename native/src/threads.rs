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
/// thread is one of the pool's (its own work and its waits are the pool's). A pool already made is left as it is.
pub fn configure(this_thread: bool) {
    rerail_accel::exec::configure(this_thread);
}

/// Whether this is the process's main thread.
pub fn is_main_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: no arguments.
        unsafe { libc::pthread_main_np() == 1 }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: no arguments.
        unsafe { libc::gettid() == libc::getpid() }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Jobs on the one pool that borrow the caller's data, within [`scope`]: every one has ended when it returns. Each is
/// handed the scope again, to start more.
pub(crate) struct Jobs<'s, 'r>(
    #[cfg(not(target_family = "wasm"))] &'r rayon::Scope<'s>,
    #[cfg(target_family = "wasm")] std::marker::PhantomData<(&'s (), &'r ())>,
);

impl<'s> Jobs<'s, '_> {
    /// Starts `f` as the job `name` on the pool (wasm32 has no threads: it runs here). A wait that runs it for
    /// another knows it by its name (`rerail_accel::exec::waits`).
    pub(crate) fn spawn(&self, name: &'static str, f: impl for<'x> FnOnce(&Jobs<'s, 'x>) + Send + 's) {
        #[cfg(not(target_family = "wasm"))]
        {
            // The job's span is under the one open where it was started, on whichever thread runs it.
            let under = crate::code_rule_dsl::profile_here();
            self.0.spawn(move |s| {
                let _span = crate::code_rule_dsl::profile_span_at(under.as_deref(), || format!("{name} (job)"));
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
pub(crate) fn scope<'s, R>(op: impl for<'r> FnOnce(&Jobs<'s, 'r>) -> R) -> R {
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
/// waits must never run it on the caller's own stack. It runs on another of the pool's threads, and the caller runs
/// no other job until it has started ([`Beside::wait_for`]); with one thread (or none), on a thread of its own: one
/// thread cannot hold both stacks.
pub(crate) struct Beside<T> {
    name: &'static str,
    slot: std::sync::Arc<(parking_lot::Mutex<Option<std::thread::Result<T>>>, std::sync::atomic::AtomicBool)>,
}

/// Starts `f` beside the caller ([`Beside`]).
pub(crate) fn beside<T: Send + 'static>(name: &'static str, f: impl FnOnce() -> T + Send + 'static) -> Beside<T> {
    let slot = std::sync::Arc::new((parking_lot::Mutex::new(None), std::sync::atomic::AtomicBool::new(false)));
    let held = std::sync::Arc::clone(&slot);
    let under = crate::code_rule_dsl::profile_here();
    let run = move || {
        held.1.store(true, std::sync::atomic::Ordering::Release);
        let _span = crate::code_rule_dsl::profile_span_at(under.as_deref(), || format!("{name} (job)"));
        let r = rerail_accel::exec::as_job(name, || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
        *held.0.lock() = Some(r);
    };
    if cfg!(not(target_family = "wasm")) && max() >= 2 {
        rayon::spawn(run);
    } else {
        let _ = rerail_accel::exec::service(name, None, run);
    }
    Beside { name, slot }
}

impl<T> Beside<T> {
    /// Waits until `poll` gives a value, the wait recorded under `site`. Once the job has started, the wait runs the
    /// pool's other jobs; before, it only sleeps (a little more each time, to 1 ms), so it never takes the job itself.
    pub(crate) fn wait_for<V>(&self, site: &'static str, mut poll: impl FnMut() -> Option<V>) -> V {
        let mut nap = 20e-6f64;
        while !self.slot.1.load(std::sync::atomic::Ordering::Acquire) {
            if let Some(v) = poll() {
                return v;
            }
            std::thread::sleep(std::time::Duration::from_secs_f64(nap));
            nap = (nap * 2.0).min(1e-3);
        }
        rerail_accel::exec::help_until_as(site, poll)
    }

    /// Its result, once it ends (a panic as `Err`).
    pub(crate) fn join(self) -> std::thread::Result<T> {
        self.wait_for(self.name, || self.slot.0.lock().take())
    }
}

#[cfg(test)]
mod tests {
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
}
