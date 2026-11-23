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

use rerail_accel::Executor;
use rerail_accel::exec::{Backend, CpuBackend};
use rerail_accel::sql::SqlBackend;
use std::sync::{Arc, LazyLock};

/// Whether each executor was made: the profile reports only those.
static MADE: [std::sync::atomic::AtomicBool; 3] = [const { std::sync::atomic::AtomicBool::new(false) }; 3];

static CPU: LazyLock<Executor> = LazyLock::new(|| {
    MADE[0].store(true, std::sync::atomic::Ordering::Relaxed);
    // The statistics kept; their stored prices are read when the first job is priced (`costs::ready`).
    super::costs::start();
    Executor::with(cpu_lanes())
});

/// The CPU binding's lanes: the cores, SQL and, where the CPU has one, its matrix unit (`sme::backend` asks the CPU
/// itself). No device lane is asked for: asking compiles the GPU's kernels and builds the Neural Engine's lane (11 ms
/// of a run on the cores, decision 0285).
fn cpu_lanes() -> Vec<Arc<dyn Backend>> {
    #[cfg_attr(not(all(target_os = "macos", target_arch = "aarch64")), allow(unused_mut))]
    let mut lanes: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend::default()), Arc::new(SqlBackend)];
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    lanes.extend(rerail_accel::sme::backend());
    lanes
}
static ALL: LazyLock<Executor> = LazyLock::new(|| {
    MADE[1].store(true, std::sync::atomic::Ordering::Relaxed);
    // The statistics kept; their stored prices are read when the first job is priced (`costs::ready`).
    super::costs::start();
    // Every submission the executor makes is checked against the engine's launch budget (decision 0100 §2).
    if let Some(dev) = crate::code_rule_dsl::logic::device::solver() {
        dev.install_launch_guard();
    }
    Executor::new()
});

static SQL: LazyLock<Executor> = LazyLock::new(|| {
    MADE[2].store(true, std::sync::atomic::Ordering::Relaxed);
    Executor::sql_only()
});

/// Every rule job submitted in SQL mode must have a SQL form.
pub fn sql_executor() -> &'static Executor { &SQL }

/// The executor for this thread's run.
pub fn executor() -> &'static Executor {
    if super::plan::placement::device() { &ALL } else { &CPU }
}

/// The executor of a binding: every lane under the device binding (`device`), the cores' (and SME's) otherwise. For
/// a job on another thread than the run's (whose binding is the run's thread's, [`executor`]).
pub fn executor_for(device: bool) -> &'static Executor {
    if device { &ALL } else { &CPU }
}

/// Each executor made so far, by binding, and its scoreboard: for the profile.
pub fn scores() -> Vec<(&'static str, rerail_accel::score::Score)> {
    let mut out = Vec::new();
    if MADE[0].load(std::sync::atomic::Ordering::Relaxed) {
        out.push(("cpu binding", CPU.score()));
    }
    if MADE[1].load(std::sync::atomic::Ordering::Relaxed) {
        out.push(("metal binding", ALL.score()));
    }
    if MADE[2].load(std::sync::atomic::Ordering::Relaxed) {
        out.push(("sql binding", SQL.score()));
    }
    out
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
    use rerail_accel::{Batch, Kernel, Launch, Output};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    thread_local! {
        static SUSPENDABLE: Cell<u32> = const { Cell::new(0) };
        static QUEUED: RefCell<Vec<(&'static str, u64, Kernel)>> = const { RefCell::new(Vec::new()) };
        /// The outputs of the launches this thread's flushes ran, by ticket. Fencing it is not needed: tickets are
        /// unique in the process (`NEXT`), a flush adds only the launches its own job queued (`flush` splits `QUEUED`
        /// at the job's fence), and `take` removes only the tickets its caller holds; so a job never reads another
        /// job's outputs, and its own are where its own flush left them (`take_outputs` moves them with the job).
        static DONE: RefCell<HashMap<u64, Output>> = RefCell::new(HashMap::new());
        /// Waves flushed on this thread and the launches they held (for tests).
        static FLUSHED: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
        /// What else the wave runs when it is flushed, after its launches: a form's own submission (the device
        /// joins' open command buffer, `setfix::device`), once.
        static DEFERRED: RefCell<Vec<Box<dyn FnOnce() -> Result<(), String>>>> = RefCell::new(Vec::new());
        /// Where the wave stood when this thread began each job it runs while waiting (`fence`), innermost last.
        static FENCES: RefCell<Vec<Mark>> = const { RefCell::new(Vec::new()) };
    }

    /// A job may run on a thread inside another's work (a wait that helps the pool, or one of rayon's that takes a
    /// queued job): from here until `unfence`, a `flush` on this thread runs only what this job queued, not the
    /// launches and submissions the job around it left for its own wave. Without it the inner job ran them into its
    /// own outputs, and the outer job's stratum, resumed on another thread, found none ("launch has not run"). Called
    /// around every job started through `rerail_accel::exec::as_job` or a stream (`rerail_accel::exec::fenced`).
    pub fn fence() {
        let at = mark();
        FENCES.with_borrow_mut(|f| f.push(at));
    }

    /// How many jobs this thread is running one inside another (`fence`): 0 on a thread's own work. A form that keeps
    /// an open submission of its own for the wave keeps one per depth (`OpenWaves`).
    pub fn depth() -> usize {
        FENCES.with_borrow(Vec::len)
    }

    /// A thread's open submissions for its waves (the device joins' command buffers, `setfix::device`), each with the
    /// depth (`depth`) of the job that opened it. A job encodes into the one of its own depth and commits it at its own
    /// flush (the commit it `defer`red takes it by number): an inner job that found the outer job's submission open
    /// and encoded into it deferred no commit, so its fenced flush committed nothing and its stratum found its wave
    /// unflushed, and the outer job's commit then ran the inner's work.
    ///
    /// **What a job opens or defers never outlives it.** A form makes its submission (the step that can fail), then
    /// defers its commit and opens it, with nothing that can fail between: a deferred commit always has its submission,
    /// and an open submission its commit. The commit takes the submission off first (`take`), so once it runs nothing
    /// stays open, whatever the commit then does. Every commit a job deferred runs exactly once, in one of two places:
    /// its job's `flush`, or, for whatever is left, leaving its job's fence (`unfence`), the one place that guarantees
    /// it; on a thread's own work (depth 0, no fence), the thread's next flush at depth 0, which takes everything
    /// deferred on it (a fenced job's flush takes only what its own job deferred).
    /// The ways a commit could be skipped or outlive its job, each a case of
    /// `wave_tests::nothing_a_job_deferred_outlives_it`:
    /// 1. The job ends without a flush (a form opened a submission, then its stratum ran on its own or the form
    ///    failed, so nothing waited for the wave): left in `DEFERRED`, run by `unfence`.
    /// 2. The flush's launch batch fails: its commits run anyway, and the flush returns the batch's error.
    /// 3. A launch panics: the executor runs every submission under `catch_unwind` and gives the panic back as the
    ///    batch's error (`rerail_accel::exec`, "its submission panicked"), so it reaches the flush as way 2.
    /// 4. A commit returns an error: the flush still runs the commits after it, and returns the first error.
    /// 5. A commit panics in the flush: the commits after it go back to `DEFERRED` as the panic leaves the flush
    ///    (`Commits`' drop), for `unfence` (at depth 0, the thread's next flush at depth 0); the one that panicked had
    ///    taken its submission off before it could panic.
    /// 6. A commit run by `unfence` fails or panics: `unfence` runs it under `catch_unwind` and goes on with the
    ///    rest (no stratum waits on those, and `unfence` may itself run while a panic unwinds the job); the fence is
    ///    popped first, so the depth is right whatever happens.
    /// 7. Fences left out of order: they cannot be. A job is fenced only by `rerail_accel::exec::fenced`, whose guard
    ///    leaves its fence when its scope ends, so fences nest as the jobs' calls do, and `unfence` always leaves the
    ///    innermost, committing what its job left there and nothing of the jobs around it.
    /// 8. Making the submission fails: nothing is deferred and nothing opened (`setfix::device::open_wave`).
    /// 9. A commit panics in a flush at depth 0 (a thread's own work, no fence): as 5, the thread's next flush at
    ///    depth 0 runs the rest.
    ///
    /// Launches a job queued and never flushed (`queue`) are not submissions: no later job can encode into one, and
    /// each carries its own ticket. They go back to the job around it, as before fences.
    pub struct OpenWaves<B> {
        open: Vec<(usize, u64, B)>,
    }

    impl<B> Default for OpenWaves<B> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<B> OpenWaves<B> {
        pub const fn new() -> Self {
            OpenWaves { open: Vec::new() }
        }

        /// The number of the submission a job at `depth` encodes into, if one is open.
        pub fn mine(&self, depth: usize) -> Option<u64> {
            self.open.iter().rev().find(|(d, ..)| *d == depth).map(|(_, n, _)| *n)
        }

        /// Opens submission `n` for a job at `depth`.
        pub fn open(&mut self, depth: usize, n: u64, b: B) {
            self.open.push((depth, n, b));
        }

        /// Submission `n`, while open.
        pub fn get_mut(&mut self, n: u64) -> Option<&mut B> {
            self.open.iter_mut().find(|(_, m, _)| *m == n).map(|(.., b)| b)
        }

        /// Submission `n`, closed: its job's deferred commit.
        pub fn take(&mut self, n: u64) -> Option<B> {
            let at = self.open.iter().position(|(_, m, _)| *m == n)?;
            Some(self.open.remove(at).2)
        }

        /// Every open submission, oldest first.
        pub fn each_mut(&mut self) -> impl Iterator<Item = &mut B> {
            self.open.iter_mut().map(|(.., b)| b)
        }
    }

    /// Ends the innermost `fence`, and runs every commit its job deferred and did not run (`OpenWaves`' list: a job
    /// that ended without a flush, a flush that failed or panicked part-way). Left, a later job at the same depth on
    /// this thread found its form's submission open (`OpenWaves::mine`), encoded into it, deferred nothing, and its
    /// flush committed nothing ("a stratum's wave was not flushed"). No stratum waits on these, so a commit's error
    /// reaches nobody; a panic is caught, since this runs from the fence's guard, possibly while the job unwinds. The
    /// launches the job queued and never flushed are the job around it's again.
    pub fn unfence() {
        let Some(at) = FENCES.with_borrow_mut(Vec::pop) else { return };
        let left = DEFERRED.with_borrow_mut(|d| d.split_off(at.1.min(d.len())));
        for commit in left {
            drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(commit)));
        }
    }

    /// The commits a `flush` took off `DEFERRED`: each runs once (`run_all`), and those not run when this is dropped
    /// (a panic in the flush: in its launches' own code, or in an earlier commit) go back to `DEFERRED`, for the job's
    /// `unfence` to run, or, at depth 0 (no fence), the thread's next flush at depth 0 (`OpenWaves`, ways 5 and 9).
    struct Commits(std::vec::IntoIter<Box<dyn FnOnce() -> Result<(), String>>>);

    impl Commits {
        fn take() -> Self {
            let from = fenced();
            Commits(DEFERRED.with_borrow_mut(|d| d.split_off(from.1.min(d.len()))).into_iter())
        }

        /// Runs every commit, past one that fails: the first error, if any.
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

    /// Where this thread's current job's part of the wave begins: the innermost fence, else the start.
    fn fenced() -> Mark {
        FENCES.with_borrow(|f| f.last().copied()).unwrap_or(Mark(0, 0))
    }

    /// Runs `f` when the wave is flushed (a form that encoded its work into a submission of its own, shared by the
    /// level's strata, commits it there): once, after the wave's launches, at the job's flush or, if that never runs
    /// it, when the job's fence is left (`OpenWaves`). Deferred only once its submission exists, and `f` takes the
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

        pub fn tickets(&self) -> Vec<u64> {
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

    /// Where this thread's wave stood (its launches queued, its deferred submissions) before a job ran on it
    /// (`take_queued`, `deferred_since`). A thread waiting on a job may run others meanwhile (`Stream::wait` helps the
    /// pool), so what is queued on it is not all the job's.
    #[derive(Clone, Copy)]
    pub struct Mark(usize, usize);

    pub fn mark() -> Mark {
        Mark(QUEUED.with_borrow(Vec::len), DEFERRED.with_borrow(Vec::len))
    }

    /// The launches queued on this thread since `mark`, taken off it.
    pub fn take_queued(mark: Mark) -> Queued {
        Queued(QUEUED.with_borrow_mut(|q| if q.len() > mark.0 { q.split_off(mark.0) } else { Vec::new() }))
    }

    /// Whether a submission was deferred to this thread's wave since `mark` (a device joins' command buffer,
    /// `setfix::device::submit`): it is this thread's, and cannot go to another's wave.
    pub fn deferred_since(mark: Mark) -> bool {
        DEFERRED.with_borrow(Vec::len) > mark.1
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
    pub fn queue(form: &'static str, kernels: Vec<Kernel>) -> Vec<u64> {
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
    /// profile), not those of a job it is fenced from (`fence`). Returns how many launches it ran.
    pub fn flush(strata: usize) -> Result<usize, String> {
        let from = fenced();
        let q = QUEUED.with_borrow_mut(|q| q.split_off(from.0.min(q.len())));
        let mut commits = Commits::take();
        let ran = commits.0.len();
        let n = q.len();
        let launched = if q.is_empty() { Ok(()) } else { run_launches(q, strata) };
        if n == 0 && ran > 0 {
            let (w, l) = FLUSHED.get();
            FLUSHED.set((w + 1, l));
            super::prof::wave(ran, strata);
        }
        // The commits run whether the launches ran or failed (`OpenWaves`, way 2).
        let committed = commits.run_all();
        launched?;
        committed?;
        Ok(n + ran)
    }

    /// Runs a flush's launches `q` as one batch, their outputs kept by ticket (`take`).
    fn run_launches(q: Vec<(&'static str, u64, Kernel)>, strata: usize) -> Result<(), String> {
        let n = q.len();
        let form = if q.iter().all(|l| l.0 == q[0].0) { q[0].0 } else { "device wave (forms mixed)" };
        let tickets: Vec<u64> = q.iter().map(|l| l.1).collect();
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
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(lanes.contains(&rerail_accel::Lane::Sme), rerail_accel::sme::backend().is_some(), "{lanes:?}");
    }
}

#[cfg(test)]
mod wave_tests {
    use super::wave;

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

    /// The launches an outer job queued stay its own across a fenced inner job: the inner flush runs the inner
    /// launch alone, the outer launch has not run until the outer job's own flush. Without the fence the inner flush
    /// ran both, and the outer job's stratum, resumed on another thread, found its output gone ("launch 26 has not
    /// run: its wave was not flushed": the Django pack over linkding at 4 threads failed, or fell back to one run per
    /// policy at 4x the time, in about 1 run in 8).
    #[test]
    fn a_flush_inside_a_fence_runs_only_the_launches_queued_inside_it() {
        let histogram = || rerail_accel::Kernel::Histogram { off: std::sync::Arc::from([0u32, 2]), codes: std::sync::Arc::from([0u16, 1]), width: 2 };
        std::thread::spawn(move || {
            let outer = wave::queue("a test's outer job", vec![histogram()]);
            wave::fence();
            let inner = wave::queue("a test's inner job", vec![histogram()]);
            assert_eq!(wave::flush(1).unwrap(), 1, "the inner job's flush runs its own launch alone");
            assert!(wave::take(&inner).is_ok(), "the inner launch ran");
            assert!(wave::take(&outer).is_err(), "the outer launch waits for the outer job's flush");
            wave::unfence();
            assert_eq!(wave::flush(1).unwrap(), 1, "the outer job's flush runs its launch");
            assert!(wave::take(&outer).is_ok(), "the outer launch ran");
        })
        .join()
        .unwrap();
    }

    /// A thread's open submissions and what their commits committed, as `setfix::device` keeps its command buffers.
    #[derive(Default)]
    struct Submissions {
        open: std::cell::RefCell<wave::OpenWaves<Vec<&'static str>>>,
        opened: std::cell::RefCell<Vec<u64>>,
        committed: std::cell::RefCell<Vec<(u64, Vec<&'static str>)>>,
        numbers: std::cell::Cell<u64>,
    }

    /// What a submission's commit does once it has taken its submission off.
    #[derive(Clone, Copy)]
    enum Commit {
        Fine,
        Fails,
        Panics,
    }

    /// `setfix::device::open_wave`'s steps: the submission made (`make` fails it, as a `Batch::new` that errs),
    /// then its commit deferred (it takes the submission off before anything else, as the device's does, then ends
    /// as `commit` says) and the submission opened at this job's depth, nothing that can fail between. Its number.
    fn open(subs: &std::rc::Rc<Submissions>, commit: Commit, make: Result<(), String>) -> Result<u64, String> {
        make?;
        let n = subs.numbers.get() + 1;
        subs.numbers.set(n);
        let at = std::rc::Rc::clone(subs);
        wave::defer(Box::new(move || {
            let Some(done) = at.open.borrow_mut().take(n) else { return Err(format!("submission {n}: its deferred commit found it not open")) };
            at.committed.borrow_mut().push((n, done));
            match commit {
                Commit::Fine => Ok(()),
                Commit::Fails => Err(format!("submission {n}'s commit failed")),
                Commit::Panics => panic!("submission {n}'s commit panicked"),
            }
        }));
        subs.open.borrow_mut().open(wave::depth(), n, Vec::new());
        subs.opened.borrow_mut().push(n);
        Ok(n)
    }

    /// `setfix::device::submit`'s steps over a thread's open submissions: the one of this job's depth, else a new one
    /// (`open`); `work` encoded into it. Its number.
    fn submit_as(subs: &std::rc::Rc<Submissions>, work: &'static str, commit: Commit) -> u64 {
        let mine = subs.open.borrow().mine(wave::depth());
        let n = match mine {
            Some(n) => n,
            None => open(subs, commit, Ok(())).expect("made"),
        };
        subs.open.borrow_mut().get_mut(n).expect("the open submission").push(work);
        n
    }

    fn submit(subs: &std::rc::Rc<Submissions>, work: &'static str) -> u64 {
        submit_as(subs, work, Commit::Fine)
    }

    /// A launch the executor refuses: a join with no keys (`Kernel::join_shape`), so the flush's batch fails.
    fn refused_launch() -> rerail_accel::Kernel {
        rerail_accel::Kernel::Join { left: std::sync::Arc::from(Vec::new()), right: std::sync::Arc::from(Vec::new()), right_id: None, by: rerail_accel::kernel::JoinBy::Probe }
    }

    /// A launch that panics on the CPU (a histogram code not below its width), which the executor turns into the
    /// batch's error.
    fn panicking_launch() -> rerail_accel::Kernel {
        rerail_accel::Kernel::Histogram { off: std::sync::Arc::from([0u32, 1]), codes: std::sync::Arc::from([5u16]), width: 2 }
    }

    /// `OpenWaves`' list of the ways a job's submission or deferred commit could outlive the job or be skipped, each
    /// played on a thread of its own with two forms' submissions (`a`, `b`), the job's fence entered and left as
    /// `rerail_accel::exec::fenced` does (its guard leaves it however the job ends). After each: no submission open,
    /// every one opened committed exactly once, the depth back to 0, and nothing left for a later flush. With
    /// 5913067797's `flush` and `unfence`, ways 2-6 and 9 fail.
    #[test]
    fn nothing_a_job_deferred_outlives_it() {
        type Way = fn(&std::rc::Rc<Submissions>, &std::rc::Rc<Submissions>);
        let ways: [(&str, Way); 9] = [
            ("1. the job ends without a flush", |a, _| {
                wave::fence();
                submit(a, "not waited on");
                wave::unfence();
            }),
            ("2. the flush's launch batch fails", |a, _| {
                wave::fence();
                submit(a, "beside a refused launch");
                wave::queue("a test's refused launch", vec![refused_launch()]);
                assert!(wave::flush(1).is_err(), "the batch's error is the flush's");
                wave::unfence();
            }),
            ("3. a launch panics (the executor gives it back as the batch's error: way 2)", |a, _| {
                wave::fence();
                submit(a, "beside a launch that panics");
                wave::queue("a test's panicking launch", vec![panicking_launch()]);
                assert!(wave::flush(1).is_err(), "the executor's error for the panic is the flush's");
                wave::unfence();
            }),
            ("4. a commit returns an error", |a, b| {
                wave::fence();
                submit_as(a, "its commit fails", Commit::Fails);
                submit(b, "its commit after it");
                assert!(wave::flush(1).is_err(), "the first commit's error is the flush's");
                wave::unfence();
            }),
            ("5. a commit panics in the flush", |a, b| {
                wave::fence();
                submit_as(a, "its commit panics", Commit::Panics);
                submit(b, "its commit after it");
                assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wave::flush(1))).is_err(), "the flush panicked");
                wave::unfence();
            }),
            ("6. commits run by leaving the fence fail or panic", |a, b| {
                wave::fence();
                submit_as(a, "its commit panics", Commit::Panics);
                submit_as(b, "its commit fails", Commit::Fails);
                wave::unfence();
            }),
            ("7. nested fences, each left by its own job", |a, _| {
                wave::fence();
                let outer = submit(a, "outer");
                wave::fence();
                let inner = submit(a, "inner");
                wave::unfence();
                assert_eq!(*a.committed.borrow(), [(inner, vec!["inner"])], "leaving the inner fence commits the inner job's alone");
                assert!(a.open.borrow_mut().get_mut(outer).is_some(), "the outer job's stays open");
                wave::unfence();
            }),
            ("8. making the submission fails", |a, _| {
                wave::fence();
                let before = wave::mark();
                assert!(open(a, Commit::Fine, Err("no buffer".into())).is_err());
                assert!(!wave::deferred_since(before), "nothing deferred for a submission never made");
                assert_eq!(wave::flush(1).unwrap(), 0, "the flush has nothing to run");
                wave::unfence();
            }),
            ("9. a commit panics in a flush at depth 0 (no fence)", |a, b| {
                submit_as(a, "its commit panics", Commit::Panics);
                let second = submit(b, "its commit after it");
                assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wave::flush(1))).is_err(), "the flush panicked");
                assert!(b.open.borrow_mut().get_mut(second).is_some(), "not run yet: back in the thread's deferred commits");
                assert_eq!(wave::flush(1).unwrap(), 1, "the thread's next flush at depth 0 runs it");
            }),
        ];
        let mut failed = Vec::new();
        for (name, way) in ways {
            let outcome = std::thread::spawn(move || {
                let (a, b) = (std::rc::Rc::new(Submissions::default()), std::rc::Rc::new(Submissions::default()));
                way(&a, &b);
                for subs in [&a, &b] {
                    assert!(subs.open.borrow_mut().each_mut().next().is_none(), "a submission left open");
                    let mut committed: Vec<u64> = subs.committed.borrow().iter().map(|c| c.0).collect();
                    committed.sort_unstable();
                    assert_eq!(committed, *subs.opened.borrow(), "every submission opened committed exactly once");
                }
                assert_eq!(wave::depth(), 0, "every fence left");
                assert_eq!(wave::flush(0).unwrap(), 0, "nothing left for a later flush");
            })
            .join();
            if outcome.is_err() {
                failed.push(name);
            }
        }
        assert!(failed.is_empty(), "ways a job's submission outlived it: {failed:?}");
    }

    /// An inner job that submits while the outer job's submission is open (on Metal, the device joins' command
    /// buffer) opens its own: its fenced flush commits exactly its work, and the outer job's flush exactly the outer's.
    /// With one submission per thread, the inner job encoded into the outer's, deferred no commit, and its flush
    /// committed nothing ("a stratum's wave was not flushed").
    #[test]
    fn open_submissions_follow_the_fence() {
        std::thread::spawn(|| {
            let subs = std::rc::Rc::new(Submissions::default());
            let outer = submit(&subs, "outer, first");
            wave::fence();
            let inner = submit(&subs, "inner");
            assert_ne!(inner, outer, "the inner job opens a submission of its own");
            wave::flush(1).unwrap();
            assert_eq!(*subs.committed.borrow(), [(inner, vec!["inner"])], "the inner flush commits the inner work alone");
            assert!(subs.open.borrow_mut().get_mut(outer).is_some(), "the outer submission stays open");
            wave::unfence();
            assert_eq!(submit(&subs, "outer, second"), outer, "back at its depth, the outer job encodes into its own");
            wave::flush(1).unwrap();
            assert_eq!(*subs.committed.borrow(), [(inner, vec!["inner"]), (outer, vec!["outer, first", "outer, second"])]);
        })
        .join()
        .unwrap();
    }

    /// A job run on a thread inside another's work (`rerail_accel::exec::fenced`) is fenced off the wave the outer job
    /// left on that thread: the inner job's flush runs only the submission it deferred, and the outer job's waits for
    /// its own flush.
    #[test]
    fn a_flush_inside_a_fence_runs_only_what_was_queued_inside_it() {
        std::thread::spawn(|| {
            let ran = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let (waiter, helped) = (std::rc::Rc::clone(&ran), std::rc::Rc::clone(&ran));
            wave::defer(Box::new(move || {
                waiter.borrow_mut().push("waiter");
                Ok(())
            }));
            wave::fence();
            wave::defer(Box::new(move || {
                helped.borrow_mut().push("helped");
                Ok(())
            }));
            assert_eq!(wave::flush(1).unwrap(), 1, "the helped job's flush runs its own submission alone");
            assert_eq!(*ran.borrow(), ["helped"]);
            wave::unfence();
            assert_eq!(wave::flush(1).unwrap(), 1, "the waiter's stays for its own flush");
            assert_eq!(*ran.borrow(), ["helped", "waiter"]);
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

    static FORMS: LazyLock<Mutex<BTreeMap<&'static str, Form>>> = LazyLock::new(Default::default);

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

    static BATCHES: LazyLock<Mutex<Batches>> = LazyLock::new(Default::default);

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
            let mut b = BATCHES.lock().unwrap();
            b.levels += 1;
            b.per_level[bucket(n, &[1, 2, 4, 8])] += 1;
            b.max_per_level = b.max_per_level.max(n);
        }
    }

    /// A wave the stratum driver flushed: its launches and the strata waiting on it.
    pub fn wave(launches: usize, strata: usize) {
        if profile::on() {
            let mut b = BATCHES.lock().unwrap();
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
            let mut forms = FORMS.lock().unwrap();
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
        let mut forms = FORMS.lock().unwrap();
        let f = forms.entry(form).or_default();
        match f.counts.iter_mut().find(|c| c.0 == what) {
            Some(c) => c.1 += n,
            None => f.counts.push((what, n)),
        }
    }

    /// Runs `batch` on this thread's executor, its report added to `form`'s.
    pub fn run(form: &'static str, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        run_on(super::executor(), form, batch)
    }

    /// [`run`] on `ex` (a job whose binding is not this thread's: [`executor_for`](super::executor_for)).
    pub fn run_on(ex: &rerail_accel::Executor, form: &'static str, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        let n = batch.0.len() as u64;
        let r = ex.run_as(form, batch)?;
        if profile::on() {
            {
                let (l, k) = LEVEL.get();
                LEVEL.set((l, k + 1));
                let mut b = BATCHES.lock().unwrap();
                b.batches += 1;
                b.launches += n;
                b.by_size[bucket(n, &[1, 4, 16, 64])] += 1;
                b.max_launches = b.max_launches.max(n);
            }
            let rep = &r.1;
            let mut forms = FORMS.lock().unwrap();
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
        let forms = FORMS.lock().unwrap();
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
        let forms = FORMS.lock().unwrap();
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
            let b = BATCHES.lock().unwrap();
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
