//! The engine's compute lanes (decision 0098 §2): one executor per binding, shared by every stratum of the process.
//!
//! - Under the **CPU binding**, the CPU's lanes: the cores and, where the CPU has one, its matrix unit (SME2). The
//!   CPU is a target in its own right (0095 §7), and SME is part of it. SQL forms are also available.
//! - Under the **Metal binding**, every lane this machine has: the cores, SME, the GPU and the Neural Engine. Each
//!   launch goes where `rerail_accel::price` places it: the least energy among the lanes within the parity band of
//!   the first to finish.
//!
//! The SQL binding accepts only SQL forms. Connection-bound SQL keeps a scored owner-thread gate.
//!
//! Which native binding a thread's run has is the placement planner's (`plan::placement::set_device`).
//!
//! CPU and SQL jobs share one pool of `RERAIL_THREADS` threads (the available cores when unset). Each binding keeps a
//! scoreboard of its jobs (`rerail_accel::score`): `RERAIL_SCOREBOARD=<file>` traces every job, and the strata loop
//! appends the scoreboard at each chunk's end.

use rerail_blocks::vector::Vector;
use rerail_accel::Executor;
use rerail_accel::exec::{Backend, CpuBackend};
use rerail_accel::sql::SqlBackend;
use std::sync::Arc;

/// The machine's executors, one a binding, each made the first time a run asks for it (decision 0442: the machine's
/// lanes, shared by every run on it): the profile reports only those made.
#[derive(Default)]
struct Lanes {
    cpu: std::sync::OnceLock<Arc<Executor>>,
    all: std::sync::OnceLock<Arc<Executor>>,
    sql: std::sync::OnceLock<Arc<Executor>>,
}

fn lanes() -> Arc<Lanes> {
    super::profile::machine::<Lanes>()
}

fn cpu() -> Arc<Executor> {
    Arc::clone(lanes().cpu.get_or_init(|| {
        // The statistics kept; their stored prices are read when the first job is priced (`costs::ready`).
        super::costs::start();
        Arc::new(Executor::with(cpu_lanes()))
    }))
}

/// The CPU binding's lanes: the cores, SQL and, where the CPU has one, its matrix unit (`rerail_accel::sme_backend` asks the CPU
/// itself). No device lane is asked for: asking compiles the GPU's kernels and builds the Neural Engine's lane (11 ms
/// of a run on the cores, decision 0285).
fn cpu_lanes() -> Vec<Arc<dyn Backend>> {
    let mut lanes: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend::default()), Arc::new(SqlBackend)];
    lanes.extend(rerail_accel::sme_backend());
    lanes
}

fn all() -> Arc<Executor> {
    Arc::clone(lanes().all.get_or_init(|| {
        // The statistics kept; their stored prices are read when the first job is priced (`costs::ready`).
        super::costs::start();
        // Every launch the executor's GPU lane makes is charged to the launch budget where it is made
        // (`rerail_accel_gpu::gpu::GpuBackend::run`, decision 0100 §2).
        Arc::new(Executor::new())
    }))
}

/// Every rule job submitted in SQL mode must have a SQL form.
pub fn sql_executor() -> Arc<Executor> {
    Arc::clone(lanes().sql.get_or_init(|| Arc::new(Executor::sql_only())))
}

/// The executor for this thread's run.
pub fn executor() -> Arc<Executor> {
    executor_for(super::device::binding::device())
}

/// The executor of a binding: every lane under the device binding (`device`), the cores' (and SME's) otherwise. For
/// a job on another thread than the run's (whose binding is the run's thread's, [`executor`]).
pub fn executor_for(device: bool) -> Arc<Executor> {
    if device { all() } else { cpu() }
}

/// Each executor made so far, by binding, and its scoreboard: for the profile.
pub fn scores() -> Vec<(&'static str, rerail_accel::score::Score)> {
    let lanes = lanes();
    [("cpu binding", &lanes.cpu), ("metal binding", &lanes.all), ("sql binding", &lanes.sql)]
        .into_iter()
        .filter_map(|(name, made)| Some((name, made.get()?.score())))
        .collect()
}

/// Device waves (decision 0095 §5, 0098 §6): the launches of strata that do not read each other, run as one batch.
///
/// A form that launches kernels runs in two halves around them. When its caller can wait (`can_wait`, while a
/// `Suspendable` is held: the stratum driver), the first half queues its launches here and the component's run
/// returns "waiting on its wave"; the driver suspends the stratum, with its state, and goes on to the next stratum of
/// the level. When the level ends, or a stratum reads a waiting one's relations, the driver `flush`es: every queued
/// launch, of every form, goes to the executor as **one batch**, so each lane gets one submission for the wave. Each
/// waiting stratum is then resumed in order, and its second half `take`s its outputs by ticket.
///
/// Where no caller can wait (a component run through SQL, a lattice fixpoint, a test), the form runs its launches at
/// once, as a batch of its own. `RERAIL_WAVES=off` does that everywhere, to measure what the waves save.
pub mod wave {
    use rerail_blocks::vector::Vector;
    use rerail_accel::{Batch, Kernel, Launch, Output};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    thread_local! {
        static SUSPENDABLE: Cell<u32> = const { Cell::new(0) };
        static QUEUED: RefCell<Vec<(&'static str, u64, Kernel)>> = const { RefCell::new(Vec::new()) };
        /// The outputs of the launches this thread's flushes ran, by ticket. A job's scope (`Scope`) need not
        /// save it: tickets are unique in the process (`NEXT`), a flush adds only the launches its own job queued
        /// (`QUEUED` is the job's), and `take` removes only the tickets its caller holds; so a job never reads another
        /// job's outputs, and its own are where its own flush left them (`take_outputs` moves them with the job).
        static DONE: RefCell<HashMap<u64, Output>> = RefCell::new(HashMap::new());
        /// Waves flushed on this thread and the launches they held (for tests).
        static FLUSHED: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
        /// What else the wave runs when it is flushed, after its launches: a form's own submission (the device
        /// joins' open command buffer, `setfix::device`), once.
        static DEFERRED: RefCell<Vec<Box<dyn FnOnce() -> Result<(), String>>>> = RefCell::new(Vec::new());
        /// The flushes on this thread so far, never taken back (`Mark`, `kept_here`): `FLUSHED` is moved between threads
        /// with the jobs it counts (`since`), so it cannot say whether this thread flushed since a mark.
        static FLUSHES: Cell<u64> = const { Cell::new(0) };
        /// The submissions to this thread's wave so far (`joined`): one that opens the wave defers its commit, and the
        /// rest join it and defer nothing, though their strata wait on it as much.
        static JOINED: Cell<u64> = const { Cell::new(0) };
        /// The open submission its strata share (the device joins' wave, `setfix::device`), of the job running here
        /// (`Scope`): one kind of value, held as `Any` so this module names no form.
        static OPEN: RefCell<Option<Box<dyn std::any::Any>>> = const { RefCell::new(None) };
    }

    /// The job's open submission (`OPEN`) as a `T`, for `f` to read, open or take. Re-entered from inside `f`, it
    /// panics (the borrow is held), as the open submission is one value.
    pub fn with_open<T: 'static, R>(f: impl FnOnce(&mut Option<T>) -> R) -> R {
        OPEN.with_borrow_mut(|slot| {
            let mut open: Option<T> = slot.take().map(|b| *b.downcast::<T>().expect("one kind of open submission"));
            let r = f(&mut open);
            *slot = open.map(|t| Box::new(t) as Box<dyn std::any::Any>);
            r
        })
    }

    /// A job's wave (decision 0386): what a job queues, defers and opens for its wave is its own, not its thread's.
    /// A job starts on a thread that may be in the middle of another (a wait runs the pool's jobs): entering, it
    /// saves the thread's (the outer job's) and starts empty; leaving, it runs what it deferred and did not flush
    /// (its open submission is finished there, on the thread that encoded it), hands what it queued and did not take
    /// to the outer job's wave, and restores the rest. So no submission outlives its job on a thread that may go idle,
    /// and a job's strata never wait on a wave another job flushes.
    pub struct Scope {
        queued: Vec<(&'static str, u64, Kernel)>,
        deferred: Vec<Box<dyn FnOnce() -> Result<(), String>>>,
        open: Option<Box<dyn std::any::Any>>,
    }

    /// Enters a job's wave scope.
    pub fn enter() -> Scope {
        Scope { queued: QUEUED.with_borrow_mut(std::mem::take), deferred: DEFERRED.with_borrow_mut(std::mem::take), open: OPEN.with_borrow_mut(Option::take) }
    }

    impl Scope {
        /// Leaves it: what the job deferred and no flush ran runs (way 1 of `Commits`' list: the job ended without a
        /// flush; ways 2-5: a flush that failed or panicked part-way left the rest), then the outer wave is restored.
        /// No stratum waits on these, so a deferred submission that fails or panics is counted and the exit goes on
        /// (way 6): it runs from the job scope's guard, possibly while the job unwinds, and the outer job's wave must
        /// be restored whatever happens. A submission that failed failed the strata in it, which ran on the CPU
        /// (`setfix::device::submit`).
        pub fn exit(self) {
            let (mut runs, mut panicked) = (0u64, false);
            loop {
                let deferred = DEFERRED.with_borrow_mut(std::mem::take);
                if deferred.is_empty() {
                    break;
                }
                for f in deferred {
                    runs += 1;
                    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
                    panicked |= done.is_err();
                    if !matches!(done, Ok(Ok(()))) {
                        super::prof::count("device wave (decision 0386)", "a job's unflushed submission failed at its end", 1);
                    }
                }
            }
            if runs > 0 {
                super::prof::count("device wave (decision 0386)", "submissions a job deferred and finished at its end", runs);
            }
            // A finish that panicked before it took its submission off leaves it here: dropped, not given to the
            // outer job.
            let left = OPEN.with_borrow_mut(Option::take);
            debug_assert!(left.is_none() || panicked, "a job's open submission outlived its deferred finish");
            let Scope { mut queued, deferred, open } = self;
            QUEUED.with_borrow_mut(|q| {
                queued.append(q);
                *q = queued;
            });
            DEFERRED.with_borrow_mut(|d| *d = deferred);
            OPEN.with_borrow_mut(|o| *o = open);
        }
    }

    /// The deferred submissions a `flush` took off `DEFERRED`: each runs once (`run_all`), and those not run when this
    /// is dropped (a panic in the flush: in its launches' own code, or in an earlier one) go back to `DEFERRED`, for the
    /// job's `Scope::exit` to run, or, on a thread's own work (no scope), the thread's next flush.
    ///
    /// **What a job defers never outlives it, and runs exactly once.** A form makes its submission (the step that can
    /// fail), then defers its finish and opens it, with nothing that can fail between: a deferred finish always has its
    /// submission. The finish takes the submission off first (`with_open`'s `take`), so once it runs nothing stays
    /// open, whatever it then does. The ways a finish could be skipped or outlive its job, each a case of
    /// `wave_tests::nothing_a_job_deferred_outlives_it`:
    /// 1. The job ends without a flush (its strata ran on their own, or a form failed, so nothing waited for the
    ///    wave): run by `Scope::exit`.
    /// 2. The flush's launch batch fails: the finishes run anyway, and the flush returns the batch's error.
    /// 3. A launch panics: the executor runs every submission under `catch_unwind` and gives the panic back as the
    ///    batch's error (`rerail_accel::exec`, "its submission panicked"), so it reaches the flush as way 2.
    /// 4. A finish returns an error: the flush still runs the finishes after it, and returns the first error.
    /// 5. A finish panics in the flush: the ones after it go back to `DEFERRED` as the panic leaves the flush (this
    ///    guard's drop), for `Scope::exit`.
    /// 6. A finish run by `Scope::exit` fails or panics: the exit goes on with the rest and restores the outer wave.
    /// 7. Nested scopes: each job's exit runs what its own job deferred, and nothing of the jobs around it.
    /// 8. Making the submission fails: nothing is deferred and nothing opened (`setfix::device::submit`).
    /// 9. A finish panics in a flush on a thread's own work (no scope): as 5, the thread's next flush runs the rest.
    struct Commits(std::vec::IntoIter<Box<dyn FnOnce() -> Result<(), String>>>);

    impl Commits {
        fn take() -> Self {
            Commits(DEFERRED.with_borrow_mut(std::mem::take).into_iter())
        }

        /// Runs every one, past one that fails: the first error, if any.
        fn run_all(&mut self) -> Result<(), String> {
            let mut first = Ok(());
            for commit in self.0.by_ref() {
                let r = commit();
                if first.is_ok() {
                    first = r;
                }
            }
            first
        }
    }

    impl Drop for Commits {
        fn drop(&mut self) {
            let rest: Vec<_> = self.0.by_ref().collect();
            if !rest.is_empty() {
                DEFERRED.with_borrow_mut(|d| d.extend(rest));
            }
        }
    }

    /// `rerail_accel::exec::set_job_scope`'s entry and exit: every pool job runs in a wave scope of its own.
    pub fn install_job_scope() {
        // The scope is moved out of its box before `exit` (which takes it by value): Relax dropped a box's contents
        // again after a by-value call on it (`boxed.exit()`), so every job's queued launches were dropped twice.
        rerail_accel::exec::set_job_scope(|| Box::new(enter()), |s| { let scope: Scope = *s.downcast::<Scope>().expect("a wave scope"); scope.exit() });
    }

    /// Runs `f` when the wave is flushed (a form that encoded its work into a submission of its own, shared by the
    /// level's strata, commits it there): once, after the wave's launches, at the job's flush or, if that never runs
    /// it, when the job's scope is left (`Commits`, `Scope::exit`). Deferred only once its submission exists, and `f` takes the
    /// submission off before anything that can fail.
    pub fn defer(f: Box<dyn FnOnce() -> Result<(), String>>) {
        DEFERRED.with_borrow_mut(|d| d.push(f));
    }

    /// The next launch's ticket, of the process: a component's job queues on its own thread and hands its launches to
    /// the run's (`take_queued`, `requeue`), where they share the run's tickets.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Launches queued on one thread and not flushed (`take_queued`): a component's job hands them to the run's thread,
    /// whose wave runs them with the level's (decision 0123 §6).
    pub struct Queued(Vec<(&'static str, u64, Kernel)>);

    impl Queued {
        pub fn len(&self) -> usize {
            self.0.len()
        }

        pub fn tickets(&self) -> Vector<u64> {
            self.0.iter().map(|l| l.1).collect()
        }
    }

    /// A flushed wave's outputs, taken off the thread that flushed it for the one that resumes their launches' strata
    /// (`take_outputs`, `put_outputs`).
    pub struct Outputs(Vec<(u64, Output)>);

    /// The outputs of `tickets`' launches that this thread's waves ran, taken off it.
    pub fn take_outputs(tickets: &[u64]) -> Outputs {
        DONE.with_borrow_mut(|d| Outputs(tickets.iter().filter_map(|t| d.remove(t).map(|o| (*t, o))).collect()))
    }

    /// Outputs another thread's wave ran (`take_outputs`), for `take` on this one.
    pub fn put_outputs(o: Outputs) {
        DONE.with_borrow_mut(|d| d.extend(o.0));
    }

    /// Where this thread's wave stood (its launches queued, its deferred submissions, its flushes) before a job ran on
    /// it (`take_queued`, `kept_here`). A thread waiting on a job may run others meanwhile (`Stream::wait` helps the
    /// pool), so what is queued on it is not all the job's.
    #[derive(Clone, Copy)]
    pub struct Mark(usize, usize, u64, u64);

    pub fn mark() -> Mark {
        Mark(QUEUED.with_borrow(|v| v.len()), DEFERRED.with_borrow(|v| v.len()), FLUSHES.get(), JOINED.get())
    }

    /// A stratum's work went into this thread's wave (a device joins' submission, opening the wave or joining it): its
    /// job's strata finish on this thread (`kept_here`).
    pub fn joined() {
        JOINED.set(JOINED.get() + 1);
    }

    /// Whether a job's strata waiting on their wave must finish on this thread (`Strand::flush_parked`), not go back
    /// with their launches to another's wave: a submission was deferred here since `mark` (a device joins' command
    /// buffer, its ticket and its rows this thread's), or this thread flushed a wave since. A job run here while
    /// this one waited (`Stream::wait` helps the pool) flushes the thread's wave, this job's submissions and launches
    /// with its own: their results are this thread's then, and the deferred count no longer shows them. A stratum's
    /// submission that joined a wave opened before `mark` deferred nothing, and counts as `joined`. Before this,
    /// a stratum so handed on was resumed on another thread, whose ticket of the same number was another stratum's:
    /// it read that one's rows, or panicked on its rules (Q170).
    pub fn kept_here(mark: Mark) -> bool {
        deferred_since(mark) || FLUSHES.get() > mark.2 || JOINED.get() > mark.3
    }

    /// A mark at `queued` launches, no deferred submission, and this thread's flushes and joins as they are now (for
    /// tests).
    #[cfg(test)]
    pub fn mark_at(queued: usize) -> Mark {
        Mark(queued, 0, FLUSHES.get(), JOINED.get())
    }

    /// The launches queued on this thread since `mark`, taken off it.
    pub fn take_queued(mark: Mark) -> Queued {
        Queued(QUEUED.with_borrow_mut(|q| if q.len() > mark.0 { q.split_off(mark.0) } else { Vec::new() }))
    }

    /// Whether a submission was deferred to this thread's wave since `mark` (a device joins' command buffer,
    /// `setfix::device::submit`): it is this thread's, and cannot go to another's wave.
    pub fn deferred_since(mark: Mark) -> bool {
        DEFERRED.with_borrow(|v| v.len()) > mark.1
    }

    /// Queues another thread's launches (`take_queued`) on this one, for its next `flush`.
    pub fn requeue(q: Queued) {
        QUEUED.with_borrow_mut(|mine| mine.extend(q.0));
    }

    /// While held, a form's launches may wait for the wave.
    pub struct Suspendable(bool);

    pub fn suspendable() -> Suspendable {
        let on = !std::env::var("RERAIL_WAVES").is_ok_and(|v| v == "off");
        if on {
            SUSPENDABLE.set(SUSPENDABLE.get() + 1);
        }
        Suspendable(on)
    }

    impl Drop for Suspendable {
        fn drop(&mut self) {
            if self.0 {
                SUSPENDABLE.set(SUSPENDABLE.get() - 1);
            }
        }
    }

    /// Whether this thread's caller can wait for the wave.
    pub fn can_wait() -> bool {
        SUSPENDABLE.get() > 0
    }

    /// Queues `form`'s launches for the wave: their tickets, in order.
    pub fn queue(form: &'static str, kernels: Vec<Kernel>) -> Vector<u64> {
        QUEUED.with_borrow_mut(|q| {
            kernels
                .into_iter()
                .map(|k| {
                    let t = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    q.push((form, t, k));
                    t
                })
                .collect()
        })
    }

    /// Runs every launch this thread's current job queued as one batch (`strata`: how many strata wait on it, for the
    /// profile), not those of a job it runs inside (`Scope`). Returns how many launches it ran.
    pub fn flush(strata: usize) -> Result<usize, String> {
        FLUSHES.set(FLUSHES.get() + 1);
        let q = QUEUED.with_borrow_mut(std::mem::take);
        let mut commits = Commits::take();
        let ran = commits.0.len();
        let n = q.len();
        let launched = if q.is_empty() { Ok(()) } else { run_launches(q, strata) };
        if n == 0 && ran > 0 {
            let (w, l) = FLUSHED.get();
            FLUSHED.set((w + 1, l));
            super::prof::wave(ran, strata);
        }
        // The deferred submissions run whether the launches ran or failed (`Commits`, way 2).
        let committed = commits.run_all();
        launched?;
        committed?;
        Ok(n + ran)
    }

    /// Runs a flush's launches `q` as one batch, their outputs kept by ticket (`take`).
    fn run_launches(q: Vec<(&'static str, u64, Kernel)>, strata: usize) -> Result<(), String> {
        let n = q.len();
        let form = if q.iter().all(|l| l.0 == q[0].0) { q[0].0 } else { "device wave (forms mixed)" };
        let tickets: Vector<u64> = q.iter().map(|l| l.1).collect();
        let batch = Batch(q.into_iter().map(|l| Launch::new(l.2)).collect());
        let (done, _) = super::prof::run(form, batch)?;
        super::prof::wave(n, strata);
        DONE.with_borrow_mut(|d| d.extend(tickets.into_iter().zip(done.into_iter().map(|d| d.out))));
        let (w, l) = FLUSHED.get();
        FLUSHED.set((w + 1, l + n as u64));
        Ok(())
    }

    /// The outputs of a flushed wave's launches, by their tickets: every one taken off, so none is left behind when
    /// one has not run (the error names the first).
    pub fn take(tickets: &[u64]) -> Result<Vec<Output>, String> {
        let got: Vec<Option<Output>> = DONE.with_borrow_mut(|d| tickets.iter().map(|t| d.remove(t)).collect());
        got.into_iter().zip(tickets).map(|(o, t)| o.ok_or_else(|| format!("launch {t} has not run: its wave was not flushed"))).collect()
    }

    /// This thread's waves flushed and the launches they held, so far: a job's own waves are credited to the run's
    /// thread when the job is taken back (`credit`), so a level's launches a job ran count as the run's.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn flushed() -> (u64, u64) {
        FLUSHED.get()
    }

    /// This thread's count so far, before a job runs (`since`).
    pub fn flushed_now() -> (u64, u64) {
        FLUSHED.get()
    }

    /// The waves this thread flushed, and their launches, since `before` (`flushed_now`), taken off this thread's
    /// count: a job moves them to the thread that takes it back (`credit`), which may be this one.
    pub fn since(before: (u64, u64)) -> (u64, u64) {
        let (w, l) = FLUSHED.get();
        FLUSHED.set(before);
        (w - before.0, l - before.1)
    }

    /// Adds a job's waves (`since`, on the job's thread) to this thread's: the run's thread, taking the job back.
    pub fn credit(waves: (u64, u64)) {
        let (w, l) = FLUSHED.get();
        FLUSHED.set((w + waves.0, l + waves.1));
    }
}

#[cfg(test)]
mod cpu_lane_tests {
    /// A run under the CPU binding asks for no device lane (2026-11-15): its lanes were picked from an executor of
    /// every lane, made and dropped, which compiled the GPU's kernels and prewarmed its sorts (about a hundred
    /// pipelines) and built the Neural Engine's lane in every CPU run.
    #[test]
    fn the_cpu_bindings_lanes_ask_for_no_device_lane() {
        let before = rerail_accel::lanes::device_lanes_asked();
        let lanes: Vec<_> = super::cpu_lanes().iter().map(|b| b.lane()).collect();
        assert_eq!(rerail_accel::lanes::device_lanes_asked(), before, "the CPU binding asked for a device lane");
        assert!(lanes.iter().all(|l| matches!(l, rerail_accel::Lane::Cpu | rerail_accel::Lane::Sql | rerail_accel::Lane::Sme)), "{lanes:?}");
        assert!(lanes.contains(&rerail_accel::Lane::Cpu) && lanes.contains(&rerail_accel::Lane::Sql), "{lanes:?}");
        assert_eq!(lanes.contains(&rerail_accel::Lane::Sme), rerail_accel::sme_backend().is_some(), "{lanes:?}");
    }
}

#[cfg(test)]
mod wave_tests {
    use super::wave;

    /// Decision 0386: a job's wave scope. Entered, the outer job's queued launches, deferred submissions and open
    /// submission are put aside; the job's own deferred submissions run when it leaves (its open submission
    /// finished there), what it queued and did not take goes to the outer wave, and the outer's are back.
    /// A launch to queue (a histogram of no codes): the scope test queues launches and never runs them.
    fn histogram() -> rerail_accel::Kernel {
        rerail_accel::Kernel::Histogram { off: std::sync::Arc::new(rerail_blocks::vector![0u32, 0]), codes: std::sync::Arc::new(rerail_blocks::vector::Vector::<u16>::new()), width: 1 }
    }

    /// Q90 (Native-and-GPU-strata), on 0386's scope: a thread waiting on a job runs another meanwhile, with a
    /// submission of its own deferred to its wave. The job's flush runs its own submission alone, its mark sees it
    /// (`deferred_since`), and the thread's runs once the job has ended, at the thread's flush. Before 0386 the job
    /// joined the open submission, and its members went back parked on a wave of this thread's, to be resumed on
    /// another, which refused them to the CPU.
    #[test]
    fn a_job_flushes_its_own_submissions_and_not_its_threads() {
        std::thread::spawn(|| {
            use std::sync::atomic::{AtomicU32, Ordering};
            // Which submissions ran: the test's own, held by the two it defers.
            let ran = std::sync::Arc::new(AtomicU32::new(0));
            let (first, second) = (std::sync::Arc::clone(&ran), std::sync::Arc::clone(&ran));
            wave::defer(Box::new(move || {
                first.fetch_or(1, Ordering::Relaxed);
                Ok(())
            }));
            let job = wave::enter();
            let mark = wave::mark();
            assert!(!wave::deferred_since(wave::mark_at(0)), "the thread's deferred submission is not the job's");
            wave::defer(Box::new(move || {
                second.fetch_or(2, Ordering::Relaxed);
                Ok(())
            }));
            assert!(wave::deferred_since(mark), "what the job defers is its own");
            assert_eq!(wave::flush(1).unwrap(), 1, "the job's flush runs its own submission alone");
            assert_eq!(ran.load(Ordering::Relaxed), 2);
            job.exit();
            assert_eq!(ran.load(Ordering::Relaxed), 2, "the thread's submission waits for its thread's flush");
            assert_eq!(wave::flush(1).unwrap(), 1);
            assert_eq!(ran.load(Ordering::Relaxed), 3, "the thread's submission ran when its thread flushed");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_jobs_wave_scope_keeps_the_outer_waves_apart() {
        std::thread::spawn(|| {
            let finished = std::rc::Rc::new(std::cell::Cell::new(0));
            wave::with_open::<u32, _>(|o| *o = Some(1));
            let f = std::rc::Rc::clone(&finished);
            wave::defer(Box::new(move || {
                f.set(f.get() + 1);
                wave::with_open::<u32, _>(Option::take);
                Ok(())
            }));
            let outer = wave::queue("a test's form", vec![histogram()]);
            let scope = wave::enter();
            assert_eq!(wave::with_open::<u32, _>(|o| *o), None, "the job sees no open submission");
            assert!(wave::take_queued(wave::mark_at(0)).len() == 0 && !wave::deferred_since(wave::mark_at(0)), "and nothing queued or deferred");
            wave::with_open::<u32, _>(|o| *o = Some(2));
            let f = std::rc::Rc::clone(&finished);
            wave::defer(Box::new(move || {
                f.set(f.get() + 10);
                assert_eq!(wave::with_open::<u32, _>(Option::take), Some(2), "the job's own open submission");
                Ok(())
            }));
            let inner = wave::queue("a test's form", vec![histogram()]);
            scope.exit();
            assert_eq!(finished.get(), 10, "the job's deferred submission ran as it left, the outer's did not");
            assert_eq!(wave::with_open::<u32, _>(|o| *o), Some(1), "the outer open submission is back");
            let left = wave::take_queued(wave::mark_at(0));
            assert_eq!(left.tickets(), [outer, inner].concat(), "the job's untaken launches went to the outer wave, after its own");
            // Not run here: the flush below runs the outer's deferred submission alone.
            drop(left);
            wave::flush(1).unwrap();
            assert_eq!(finished.get(), 11, "the outer's ran at its flush");
        })
        .join()
        .unwrap();
    }

    /// Smoking gun 7 (t34, 2026-11-13): a level's strata may run as a job off the run's thread (decision 0113 §4), and
    /// the wave their launches join is flushed there. The job takes its waves off its thread's count (`since`) and
    /// the run's thread credits them when it takes the job back (`credit`, in `absorb`): the run counts them.
    /// A thread outside the run, whose waves nothing credits, counts into its own.
    #[test]
    fn a_jobs_waves_are_credited_to_the_run_that_takes_it_back() {
        let before = wave::flushed();
        let waves = std::thread::spawn(|| {
            let at = wave::flushed_now();
            wave::defer(Box::new(|| Ok(())));
            assert_eq!(wave::flush(1).unwrap(), 1);
            let waves = wave::since(at);
            assert_eq!(wave::flushed_now(), at, "taken off the job's thread");
            waves
        })
        .join()
        .unwrap();
        assert_eq!(wave::flushed(), before, "not the run's until it takes the job back");
        wave::credit(waves);
        assert_eq!(wave::flushed().0 - before.0, 1, "the job's wave is the run's");
        std::thread::spawn(|| {
            wave::defer(Box::new(|| Ok(())));
            wave::flush(1).unwrap();
        })
        .join()
        .unwrap();
        assert_eq!(wave::flushed().0 - before.0, 1, "another thread's wave is not the run's");
    }

    /// Q170: a job's strata that wait on their wave finish on its thread when anything they wait on is the thread's: a
    /// submission deferred since its mark, a submission that joined a wave opened before it, or a wave flushed here
    /// meanwhile by a job run while this one waited (which took this job's deferred commit with it). Only with none
    /// of these do they go back to the run's wave.
    #[test]
    fn a_jobs_waiting_strata_stay_where_their_wave_is() {
        std::thread::spawn(|| {
            let quiet = wave::mark();
            assert!(!wave::kept_here(quiet), "nothing of the thread's: back to the run's wave");
            // Opened here: its commit is deferred to this thread's wave.
            let opened = wave::mark();
            wave::defer(Box::new(|| Ok(())));
            assert!(wave::kept_here(opened));
            // A job run meanwhile flushed the thread's wave, this one's deferred commit with it.
            let outer = wave::mark();
            wave::defer(Box::new(|| Ok(())));
            wave::flush(1).unwrap();
            assert!(!wave::deferred_since(outer), "the deferred count no longer shows it");
            assert!(wave::kept_here(outer), "its results are this thread's");
            // Joined a wave opened before the mark: nothing deferred, the stratum waits on it all the same.
            wave::defer(Box::new(|| Ok(())));
            let joining = wave::mark();
            wave::joined();
            assert!(!wave::deferred_since(joining));
            assert!(wave::kept_here(joining));
            wave::flush(1).unwrap();
        })
        .join()
        .unwrap();
    }

    /// main's #930, on 0386's scope: the launches an outer job queued stay its own across an inner job: the inner
    /// flush runs the inner launch alone, the outer launch has not run until the outer job's own flush. Without a
    /// job's own wave the inner flush ran both, and the outer job's stratum, resumed on another thread, found its
    /// output gone ("launch 26 has not run: its wave was not flushed": the Django pack over linkding at 4 threads
    /// failed, or fell back to one run per policy at 4x the time, in about 1 run in 8).
    #[test]
    fn a_flush_inside_a_job_scope_runs_only_the_launches_queued_inside_it() {
        let histogram = || rerail_accel::Kernel::Histogram { off: std::sync::Arc::new(rerail_blocks::vector![0u32, 2]), codes: std::sync::Arc::new(rerail_blocks::vector![0u16, 1]), width: 2 };
        std::thread::spawn(move || {
            let outer = wave::queue("a test's outer job", vec![histogram()]);
            let scope = wave::enter();
            let inner = wave::queue("a test's inner job", vec![histogram()]);
            assert_eq!(wave::flush(1).unwrap(), 1, "the inner job's flush runs its own launch alone");
            assert!(wave::take(&inner).is_ok(), "the inner launch ran");
            assert!(wave::take(&outer).is_err(), "the outer launch waits for the outer job's flush");
            scope.exit();
            assert_eq!(wave::flush(1).unwrap(), 1, "the outer job's flush runs its launch");
            assert!(wave::take(&outer).is_ok(), "the outer launch ran");
        })
        .join()
        .unwrap();
    }

    /// The tests' submissions: the job's open one (`wave::with_open`, as `setfix::device` keeps its wave), its number
    /// and the work encoded into it.
    type Open = (u64, Vec<&'static str>);

    /// What the tests' submissions opened and what their finishes committed.
    #[derive(Default)]
    struct Submissions {
        opened: std::cell::RefCell<Vec<u64>>,
        committed: std::cell::RefCell<Vec<(u64, Vec<&'static str>)>>,
        numbers: std::cell::Cell<u64>,
    }

    /// What a submission's finish does once it has taken its submission off.
    #[derive(Clone, Copy)]
    enum Commit {
        Fine,
        Fails,
        Panics,
    }

    impl Submissions {
        fn next(&self) -> u64 {
            let n = self.numbers.get() + 1;
            self.numbers.set(n);
            self.opened.borrow_mut().push(n);
            n
        }

        fn finished(&self, n: u64, work: Vec<&'static str>, commit: Commit) -> Result<(), String> {
            self.committed.borrow_mut().push((n, work));
            match commit {
                Commit::Fine => Ok(()),
                Commit::Fails => Err(format!("submission {n}'s finish failed")),
                Commit::Panics => panic!("submission {n}'s finish panicked"),
            }
        }
    }

    /// `setfix::device::submit`'s steps: the job's open submission, else one made (`make` fails it, as a `Batch::new`
    /// that errs), then its finish deferred (it takes the submission off before anything else, as the device's does,
    /// then ends as `commit` says) and the submission opened, nothing that can fail between; `work` encoded into it.
    /// Its number.
    fn submit_as(subs: &std::rc::Rc<Submissions>, work: &'static str, commit: Commit, make: Result<(), String>) -> Result<u64, String> {
        if wave::with_open::<Open, _>(|w| w.is_none()) {
            make?;
            let n = subs.next();
            let at = std::rc::Rc::clone(subs);
            wave::defer(Box::new(move || {
                let Some((m, done)) = wave::with_open::<Open, _>(Option::take) else { return Err(format!("submission {n}: its deferred finish found it not open")) };
                at.finished(m, done, commit)
            }));
            wave::with_open::<Open, _>(|w| *w = Some((n, Vec::new())));
        }
        Ok(wave::with_open::<Open, _>(|w| {
            let (n, done) = w.as_mut().expect("the open submission");
            done.push(work);
            *n
        }))
    }

    fn submit(subs: &std::rc::Rc<Submissions>, work: &'static str) -> u64 {
        submit_as(subs, work, Commit::Fine, Ok(())).expect("made")
    }

    /// A submission of its own beside the job's open one (a stratum run apart), its finish deferred: its number.
    fn apart(subs: &std::rc::Rc<Submissions>, work: &'static str, commit: Commit) -> u64 {
        let n = subs.next();
        let at = std::rc::Rc::clone(subs);
        wave::defer(Box::new(move || at.finished(n, vec![work], commit)));
        n
    }

    /// A launch the executor refuses: a join with no keys (`Kernel::join_shape`), so the flush's batch fails.
    fn refused_launch() -> rerail_accel::Kernel {
        rerail_accel::Kernel::Join { left: std::sync::Arc::from(Vec::new()), right: std::sync::Arc::from(Vec::new()), right_id: None, by: rerail_accel::kernel::JoinBy::Probe }
    }

    /// A launch that panics on the CPU (a histogram code not below its width), which the executor turns into the
    /// batch's error.
    fn panicking_launch() -> rerail_accel::Kernel {
        rerail_accel::Kernel::Histogram { off: std::sync::Arc::new(rerail_blocks::vector![0u32, 1]), codes: std::sync::Arc::new(rerail_blocks::vector![5u16]), width: 2 }
    }

    /// main's #930, on 0386's scope: the ways a job's deferred submission could outlive the job or be skipped
    /// (`wave::Commits`' list), each played on a thread of its own, the job's scope entered and left as
    /// `rerail_accel::exec::in_job_scope` does (its guard leaves it however the job ends). After each: no submission
    /// open, every one opened committed exactly once, and nothing left for a later flush. With the hub's flush and
    /// scope exit before this roll-up, ways 2-6 and 9 fail.
    #[test]
    fn nothing_a_job_deferred_outlives_it() {
        type Way = fn(&std::rc::Rc<Submissions>);
        let ways: [(&str, Way); 9] = [
            ("1. the job ends without a flush", |a| {
                let scope = wave::enter();
                submit(a, "not waited on");
                scope.exit();
            }),
            ("2. the flush's launch batch fails", |a| {
                let scope = wave::enter();
                submit(a, "beside a refused launch");
                wave::queue("a test's refused launch", vec![refused_launch()]);
                assert!(wave::flush(1).is_err(), "the batch's error is the flush's");
                scope.exit();
            }),
            ("3. a launch panics (the executor gives it back as the batch's error: way 2)", |a| {
                let scope = wave::enter();
                submit(a, "beside a launch that panics");
                wave::queue("a test's panicking launch", vec![panicking_launch()]);
                assert!(wave::flush(1).is_err(), "the executor's error for the panic is the flush's");
                scope.exit();
            }),
            ("4. a finish returns an error", |a| {
                let scope = wave::enter();
                submit_as(a, "its finish fails", Commit::Fails, Ok(())).expect("made");
                apart(a, "its finish after it", Commit::Fine);
                assert!(wave::flush(1).is_err(), "the first finish's error is the flush's");
                scope.exit();
            }),
            ("5. a finish panics in the flush", |a| {
                let scope = wave::enter();
                submit_as(a, "its finish panics", Commit::Panics, Ok(())).expect("made");
                apart(a, "its finish after it", Commit::Fine);
                assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wave::flush(1))).is_err(), "the flush panicked");
                scope.exit();
            }),
            ("6. finishes run by leaving the scope fail or panic", |a| {
                let scope = wave::enter();
                submit_as(a, "its finish panics", Commit::Panics, Ok(())).expect("made");
                apart(a, "its finish fails", Commit::Fails);
                scope.exit();
            }),
            ("7. nested scopes, each left by its own job", |a| {
                let outer_scope = wave::enter();
                let outer = submit(a, "outer");
                let inner_scope = wave::enter();
                let inner = submit(a, "inner");
                inner_scope.exit();
                assert_eq!(*a.committed.borrow(), [(inner, vec!["inner"])], "leaving the inner scope commits the inner job's alone");
                assert_eq!(wave::with_open::<Open, _>(|w| w.as_ref().map(|w| w.0)), Some(outer), "the outer job's stays open");
                outer_scope.exit();
            }),
            ("8. making the submission fails", |a| {
                let scope = wave::enter();
                let before = wave::mark();
                assert!(submit_as(a, "never encoded", Commit::Fine, Err("no buffer".into())).is_err());
                assert!(!wave::deferred_since(before), "nothing deferred for a submission never made");
                assert_eq!(wave::flush(1).unwrap(), 0, "the flush has nothing to run");
                scope.exit();
            }),
            ("9. a finish panics in a flush on a thread's own work (no scope)", |a| {
                submit_as(a, "its finish panics", Commit::Panics, Ok(())).expect("made");
                let second = apart(a, "its finish after it", Commit::Fine);
                assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wave::flush(1))).is_err(), "the flush panicked");
                assert!(!a.committed.borrow().iter().any(|c| c.0 == second), "not run yet: back in the thread's deferred submissions");
                assert_eq!(wave::flush(1).unwrap(), 1, "the thread's next flush runs it");
            }),
        ];
        let mut failed = Vec::new();
        for (name, way) in ways {
            let outcome = std::thread::spawn(move || {
                let a = std::rc::Rc::new(Submissions::default());
                way(&a);
                assert!(wave::with_open::<Open, _>(|w| w.is_none()), "a submission left open");
                let mut committed: Vec<u64> = a.committed.borrow().iter().map(|c| c.0).collect();
                committed.sort_unstable();
                assert_eq!(committed, *a.opened.borrow(), "every submission opened committed exactly once");
                assert_eq!(wave::flush(0).unwrap(), 0, "nothing left for a later flush");
            })
            .join();
            if outcome.is_err() {
                failed.push(name);
            }
        }
        assert!(failed.is_empty(), "ways a job's submission outlived it: {failed:?}");
    }

    /// main's #930, on 0386's scope: an inner job that submits while the outer job's submission is open (on Metal, the
    /// device joins' wave) opens its own: its flush commits exactly its work, and the outer job's flush exactly the
    /// outer's. With one submission per thread, the inner job encoded into the outer's, deferred no finish, and its
    /// flush committed nothing ("a stratum's wave was not flushed").
    #[test]
    fn open_submissions_follow_the_job_scope() {
        std::thread::spawn(|| {
            let subs = std::rc::Rc::new(Submissions::default());
            let outer = submit(&subs, "outer, first");
            let scope = wave::enter();
            let inner = submit(&subs, "inner");
            assert_ne!(inner, outer, "the inner job opens a submission of its own");
            wave::flush(1).unwrap();
            assert_eq!(*subs.committed.borrow(), [(inner, vec!["inner"])], "the inner flush commits the inner work alone");
            scope.exit();
            assert_eq!(wave::with_open::<Open, _>(|w| w.as_ref().map(|w| w.0)), Some(outer), "the outer submission is open again");
            assert_eq!(submit(&subs, "outer, second"), outer, "back in its scope, the outer job encodes into its own");
            wave::flush(1).unwrap();
            assert_eq!(*subs.committed.borrow(), [(inner, vec!["inner"]), (outer, vec!["outer, first", "outer, second"])]);
        })
        .join()
        .unwrap();
    }
}

/// The acceleration profile (decision 0098 §4): per form, where a launch's time goes from recognizing the form to
/// consuming its output. Each form's host phases are timed here (`host`), each batch's executor report is added
/// (`run`), and `report` renders the profile's section. Only under `RERAIL_PROFILE`.
pub mod prof {
    use super::super::profile;
    use rerail_accel::exec::{Done, Report, thread_cpu};
    use rerail_accel::{Batch, Lane};
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::sync::{LazyLock, Mutex};
    use std::time::Instant;

    #[derive(Default, Clone, Copy)]
    struct Time {
        n: u64,
        wall: f64,
        cpu: f64,
    }

    #[derive(Default)]
    struct Form {
        /// Host phases by name, in first-seen order: calls, wall seconds, this thread's CPU seconds.
        host: Vec<(&'static str, Time)>,
        batches: u64,
        /// Per lane: launches, submissions, wall, the lane thread's CPU, the backend's phases.
        launches: [u64; Lane::COUNT],
        submissions: [u64; Lane::COUNT],
        wall: [f64; Lane::COUNT],
        lane_cpu: [f64; Lane::COUNT],
        encode: [f64; Lane::COUNT],
        device: [f64; Lane::COUNT],
        device_known: [bool; Lane::COUNT],
        decode: [f64; Lane::COUNT],
        worker_cpu: [f64; Lane::COUNT],
        /// The executor's own: placement, and spawning/joining the lanes' threads.
        total: f64,
        place: f64,
        overhead: f64,
        /// Counted events by name (probes answered, rows emitted, rows walked before a table paid).
        counts: Vec<(&'static str, u64)>,
    }

    #[derive(Default)]
    struct RunForms(Mutex<BTreeMap<&'static str, Form>>);

    /// Batches and device waves, over the run: launches per batch, batches per dependency level (decision 0095 §5).
    #[derive(Default)]
    struct Batches {
        batches: u64,
        launches: u64,
        /// Batches by their launches: 1, 2–4, 5–16, 17–64, more.
        by_size: [u64; 5],
        max_launches: u64,
        /// Levels that ran any batch, by how many: 1, 2, 3–4, 5–8, more.
        levels: u64,
        per_level: [u64; 5],
        max_per_level: u64,
        /// Waves flushed by the stratum driver, the launches and the waiting strata they held.
        waves: u64,
        wave_launches: u64,
        wave_strata: u64,
    }

    #[derive(Default)]
    struct RunBatches(Mutex<Batches>);

    thread_local! {
        /// This thread's level (a run's dependency level), and the batches run in it so far.
        static LEVEL: std::cell::Cell<(Option<usize>, u64)> = const { std::cell::Cell::new((None, 0)) };
    }

    fn bucket(n: u64, bounds: &[u64; 4]) -> usize {
        bounds.iter().position(|&b| n <= b).unwrap_or(4)
    }

    /// This thread's run is now at dependency level `l` (`None`: between runs). The level before is counted.
    pub fn level(l: Option<usize>) {
        let (was, n) = LEVEL.get();
        if was == l {
            return;
        }
        LEVEL.set((l, 0));
        if n > 0 && profile::on() {
            let held_ = crate::code_rule_dsl::logic::profile::local::<RunBatches>();
            let mut b = held_.0.lock().unwrap();
            b.levels += 1;
            b.per_level[bucket(n, &[1, 2, 4, 8])] += 1;
            b.max_per_level = b.max_per_level.max(n);
        }
    }

    /// A wave the stratum driver flushed: its launches and the strata waiting on it.
    pub fn wave(launches: usize, strata: usize) {
        if profile::on() {
            let held_ = crate::code_rule_dsl::logic::profile::local::<RunBatches>();
            let mut b = held_.0.lock().unwrap();
            b.waves += 1;
            b.wave_launches += launches as u64;
            b.wave_strata += strata as u64;
        }
    }

    /// A host phase of `form`, timed until dropped.
    pub struct Host(Option<(&'static str, &'static str, Instant, f64)>);

    pub fn host(form: &'static str, phase: &'static str) -> Host {
        Host(profile::on().then(|| (form, phase, Instant::now(), thread_cpu())))
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let Some((form, phase, t, c)) = self.0.take() else { return };
            let (wall, cpu) = (t.elapsed().as_secs_f64(), thread_cpu() - c);
            let held_ = crate::code_rule_dsl::logic::profile::local::<RunForms>();
            let mut forms = held_.0.lock().unwrap();
            let f = forms.entry(form).or_default();
            match f.host.iter_mut().find(|h| h.0 == phase) {
                Some(h) => {
                    h.1.n += 1;
                    h.1.wall += wall;
                    h.1.cpu += cpu;
                }
                None => f.host.push((phase, Time { n: 1, wall, cpu })),
            }
        }
    }

    /// Counts `n` events of `what` for `form`.
    pub fn count(form: &'static str, what: &'static str, n: u64) {
        if !profile::on() || n == 0 {
            return;
        }
        let held_ = crate::code_rule_dsl::logic::profile::local::<RunForms>();
        let mut forms = held_.0.lock().unwrap();
        let f = forms.entry(form).or_default();
        match f.counts.iter_mut().find(|c| c.0 == what) {
            Some(c) => c.1 += n,
            None => f.counts.push((what, n)),
        }
    }

    /// Runs `batch` on this thread's executor, its report added to `form`'s.
    pub fn run(form: &'static str, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        run_on(&super::executor(), form, batch)
    }

    /// [`run`] on `ex` (a job whose binding is not this thread's: [`executor_for`](super::executor_for)).
    pub fn run_on(ex: &rerail_accel::Executor, form: &'static str, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        let n = batch.0.len() as u64;
        let r = ex.run_as(form, batch)?;
        if profile::on() {
            {
                let (l, k) = LEVEL.get();
                LEVEL.set((l, k + 1));
                let held_ = crate::code_rule_dsl::logic::profile::local::<RunBatches>();
                let mut b = held_.0.lock().unwrap();
                b.batches += 1;
                b.launches += n;
                b.by_size[bucket(n, &[1, 4, 16, 64])] += 1;
                b.max_launches = b.max_launches.max(n);
            }
            let rep = &r.1;
            let held_ = crate::code_rule_dsl::logic::profile::local::<RunForms>();
            let mut forms = held_.0.lock().unwrap();
            let f = forms.entry(form).or_default();
            f.batches += 1;
            f.total += rep.total;
            f.place += rep.place;
            f.overhead += rep.overhead();
            for l in Lane::ALL {
                let i = l.index();
                if rep.launches[i] == 0 {
                    continue;
                }
                f.launches[i] += rep.launches[i] as u64;
                f.submissions[i] += 1;
                f.wall[i] += rep.wall[i];
                f.lane_cpu[i] += rep.lane_cpu[i];
                let p = &rep.phases[i];
                f.encode[i] += p.encode;
                f.decode[i] += p.decode;
                f.worker_cpu[i] += p.worker_cpu;
                if let Some(d) = p.device {
                    f.device[i] += d;
                    f.device_known[i] = true;
                }
            }
        }
        Ok(r)
    }

    /// Per form and lane with launches: (form, lane, launches, wall seconds, the lane's job CPU seconds).
    pub fn by_lane() -> Vec<(&'static str, Lane, u64, f64, f64)> {
        let held_ = crate::code_rule_dsl::logic::profile::local::<RunForms>();
        let forms = held_.0.lock().unwrap();
        let mut out = Vec::new();
        for (name, f) in forms.iter() {
            for l in Lane::ALL {
                let i = l.index();
                if f.launches[i] > 0 {
                    out.push((*name, l, f.launches[i], f.wall[i], f.lane_cpu[i]));
                }
            }
        }
        out
    }

    /// The section: per form, the host phases, then per lane the launches and where the submission's time went, then
    /// the executor's own time.
    pub fn report() -> String {
        let held_ = crate::code_rule_dsl::logic::profile::local::<RunForms>();
        let forms = held_.0.lock().unwrap();
        let mut r = String::new();
        if forms.is_empty() {
            return r;
        }
        let ms = |s: f64| s * 1e3;
        let _ = writeln!(r, "\n== Acceleration (decision 0098): planning, launching and execution per form");
        let _ = writeln!(r, "Host phases are timed on the engine's thread (wall, and that thread's CPU). Per lane: the submission's");
        let _ = writeln!(r, "wall, the lane thread's CPU, the backend's encode / device / decode split and its workers' CPU. The");
        let _ = writeln!(r, "executor's own: placement, and spawning and joining lane threads (its wall less the slowest lane).");
        let mut grand = (0.0f64, 0.0f64);
        for (name, f) in forms.iter() {
            let host_cpu: f64 = f.host.iter().map(|h| h.1.cpu).sum();
            let kernel: f64 = (0..4).map(|i| if f.device_known[i] { f.device[i] } else { f.wall[i] - f.encode[i] - f.decode[i] }).sum();
            let _ = writeln!(r, "\n{name}: {} batches; host {:.1} ms CPU; executor {:.1} ms wall ({:.1} placing, {:.1} threads); kernels {:.1} ms",
                f.batches, ms(host_cpu), ms(f.total), ms(f.place), ms(f.overhead), ms(kernel));
            grand.0 += host_cpu + f.place + f.overhead + f.encode.iter().sum::<f64>() + f.decode.iter().sum::<f64>();
            grand.1 += kernel;
            for (phase, t) in &f.host {
                let _ = writeln!(r, "  host  {phase:<44} {:>7} calls {:>10.2} ms wall {:>10.2} ms CPU", t.n, ms(t.wall), ms(t.cpu));
            }
            for l in Lane::ALL {
                let i = l.index();
                if f.launches[i] == 0 {
                    continue;
                }
                let dev = if f.device_known[i] { format!("{:.2}", ms(f.device[i])) } else { "-".into() };
                let _ = writeln!(r, "  lane  {:<4} {:>5} launches in {:>5} submissions: {:>9.2} ms wall, {:>9.2} ms lane CPU; encode {:.2}, device {dev}, decode {:.2}, workers {:.2} ms CPU",
                    l.name(), f.launches[i], f.submissions[i], ms(f.wall[i]), ms(f.lane_cpu[i]), ms(f.encode[i]), ms(f.decode[i]), ms(f.worker_cpu[i]));
            }
            for (what, n) in &f.counts {
                let _ = writeln!(r, "  count {what:<44} {n:>12}");
            }
        }
        {
            let held_ = crate::code_rule_dsl::logic::profile::local::<RunBatches>();
            let b = held_.0.lock().unwrap();
            let _ = writeln!(r, "\nBatches: {} batches of {} launches (at most {} in one); by launches: 1: {}, 2-4: {}, 5-16: {}, 17-64: {}, more: {}",
                b.batches, b.launches, b.max_launches, b.by_size[0], b.by_size[1], b.by_size[2], b.by_size[3], b.by_size[4]);
            let _ = writeln!(r, "Per dependency level: {} levels ran a batch (at most {} in one level); by batches: 1: {}, 2: {}, 3-4: {}, 5-8: {}, more: {}",
                b.levels, b.max_per_level, b.per_level[0], b.per_level[1], b.per_level[2], b.per_level[3], b.per_level[4]);
            let _ = writeln!(r, "Device waves (decision 0095 §5): {} waves flushed, {} launches of {} waiting strata in them", b.waves, b.wave_launches, b.wave_strata);
        }
        let _ = writeln!(r, "\nAll forms: {:.1} ms around the kernels (host phases' CPU, placing, threads, lanes' encode and decode) against {:.1} ms in them", ms(grand.0), ms(grand.1));
        r
    }
}
