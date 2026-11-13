//! A host stream (decision 0113 §3): host jobs a caller submits as they become ready, each with its estimate, run on
//! the cores' one pool. The caller decides **what** may run (what is ready) and **how big** a job is; the executor
//! decides **how many run at once**.
//!
//! - **Claims.** Each submitted job is queued, scored on the scoreboard as host work with its estimate, and claimed
//!   once, by whoever reaches it first: one of the stream's workers, largest first, or the caller when it waits.
//! - **Workers.** The executor starts a worker for the stream only while the stream's queued estimate, spread over
//!   the workers serving it, is above its **floor** (the cost below which a pool job loses to running inline), and
//!   while the cores leave a thread: the pool's threads less the caller's, less what else runs on the cores, over
//!   the machine's load (`cores_slowdown`). Otherwise the jobs wait for the caller. With one thread
//!   (`RERAIL_THREADS=1`), or on a loaded machine, every job runs on the caller, in the order it waits for them.
//! - **The floor** is measured: the stream's workers' start latency (from being started to their first claim), at
//!   least `RERAIL_STREAM_FLOOR` seconds (100 µs).
//! - **Waiting.** [`Stream::wait`] runs the awaited job itself if no worker has claimed it, else helps the pool
//!   ([`crate::exec::help_once`]) until the job ends.
//! - **Waking.** A waiter with nothing to run sleeps until one of the stream's jobs ends ([`Stream::pause`]), at most
//!   a bounded nap: woken by the job's end, not by polling (a nap of up to 1 ms after each ended job was 0.5 s of an
//!   edit's run on the critical path).
//! - **No nesting.** A job never runs inside another on one thread: a job that waits (on a device batch) and helps
//!   the pool may take a stream worker, which then claims nothing, since the outer job may hold what the inner
//!   needs (a lock).
//!
//! The jobs may borrow what outlives the stream ([`Executor::stream`] runs it as a scope), and a job that panics is an
//! error to its waiter.

use crate::exec::{Executor, help_once, on_pool, threads, cores_slowdown};
use crate::lanes::Lane;
use crate::score::{Item, Scoreboard, Ticket};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

type Slot<R> = Arc<Mutex<Option<std::thread::Result<R>>>>;

thread_local! {
    /// Stream jobs running on this thread now (a waiter or a helper may run one inside another's wait).
    static IN_JOB: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

struct Queued<'s> {
    id: u64,
    estimate: f64,
    job: Box<dyn FnOnce() + Send + 's>,
    ticket: Ticket,
}

struct State<'s> {
    name: &'static str,
    queue: Mutex<Vec<Queued<'s>>>,
    /// Workers serving the stream now.
    serving: AtomicUsize,
    /// The measured start latency of a worker, in nanoseconds (a moving average).
    latency: AtomicU64,
    board: Arc<Scoreboard>,
    next: AtomicU64,
    /// Jobs run by the caller and by workers (for tests and the trace).
    by_caller: AtomicUsize,
    by_workers: AtomicUsize,
    peak: AtomicUsize,
    running: AtomicUsize,
    /// Jobs ended so far, and the waiters' wake-up when one does (`Stream::pause`).
    ended: Mutex<u64>,
    wake: Condvar,
}

impl<'s> State<'s> {
    /// The job `prefer` if it is still queued, else the largest queued (none with `only`).
    fn claim(&self, prefer: Option<u64>, only: bool) -> Option<Queued<'s>> {
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        let at = match prefer.and_then(|id| q.iter().position(|j| j.id == id)) {
            Some(at) => at,
            None if only => return None,
            None => (0..q.len()).max_by(|&a, &b| q[a].estimate.total_cmp(&q[b].estimate))?,
        };
        Some(q.swap_remove(at))
    }

    fn run(&self, mut j: Queued<'s>, caller: bool) {
        self.board.start(&mut j.ticket);
        let now = self.running.fetch_add(1, Ordering::AcqRel) + 1;
        self.peak.fetch_max(now, Ordering::AcqRel);
        IN_JOB.set(IN_JOB.get() + 1);
        (j.job)();
        IN_JOB.set(IN_JOB.get() - 1);
        self.running.fetch_sub(1, Ordering::AcqRel);
        self.board.finish(j.ticket, true);
        if caller { &self.by_caller } else { &self.by_workers }.fetch_add(1, Ordering::Relaxed);
        *self.ended.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.wake.notify_all();
    }

    /// The floor: a worker's measured start latency, four times over, and at least the prior.
    fn floor(&self) -> f64 {
        static PRIOR: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
        let prior = *PRIOR.get_or_init(|| std::env::var("RERAIL_STREAM_FLOOR").ok().and_then(|v| v.parse().ok()).unwrap_or(100e-6));
        (4.0 * self.latency.load(Ordering::Relaxed) as f64 * 1e-9).max(prior)
    }
}

/// A host stream open on the executor ([`Executor::stream`]).
pub struct Stream<'s, 'e> {
    scope: &'e rayon::Scope<'s>,
    state: Arc<State<'s>>,
}

/// A job submitted to a stream: [`Stream::wait`] gives its result.
pub struct Handle<R> {
    id: u64,
    slot: Slot<R>,
}

/// What a stream did: jobs run by the caller and by its workers, and the most that ran at once.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamCounts {
    pub by_caller: usize,
    pub by_workers: usize,
    pub peak: usize,
}

impl<'s> Stream<'s, '_> {
    /// Queues `job`, estimated at `estimate` seconds, and starts the workers the executor decides on.
    pub fn submit<R: Send + 's>(&self, estimate: f64, job: impl FnOnce() -> R + Send + 's) -> Handle<R> {
        let st = &self.state;
        let id = st.next.fetch_add(1, Ordering::Relaxed);
        let slot: Slot<R> = Arc::new(Mutex::new(None));
        let to = Arc::clone(&slot);
        let job = Box::new(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            *to.lock().unwrap_or_else(|p| p.into_inner()) = Some(r);
        });
        let ticket = st.board.queue(Lane::Cpu, vec![Item { form: st.name, units: 0.0, estimate, host: estimate }]);
        st.queue.lock().unwrap_or_else(|p| p.into_inner()).push(Queued { id, estimate, job, ticket });
        self.start_workers();
        Handle { id, slot }
    }

    /// Starts workers while the queued estimate per worker is above the floor and the cores leave a thread.
    fn start_workers(&self) {
        let st = &self.state;
        let queued: f64 = st.queue.lock().unwrap_or_else(|p| p.into_inner()).iter().map(|j| j.estimate).sum();
        // The caller is one thread; others' jobs on the cores take theirs.
        let others = st.board.with_units(Lane::Cpu, |u| u.running).saturating_sub(st.running.load(Ordering::Acquire));
        let cores = (threads() as f64 / cores_slowdown()).floor() as usize;
        let free = cores.saturating_sub(1 + others);
        let floor = st.floor();
        loop {
            let serving = st.serving.load(Ordering::Acquire);
            if serving >= free || queued / (serving + 1) as f64 <= floor {
                return;
            }
            if st.serving.compare_exchange(serving, serving + 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
                continue;
            }
            let state = Arc::clone(st);
            let started = Instant::now();
            self.scope.spawn(move |_| {
                let ns = started.elapsed().as_nanos() as u64;
                let old = state.latency.load(Ordering::Relaxed);
                state.latency.store(if old == 0 { ns } else { (old * 7 + ns) / 8 }, Ordering::Relaxed);
                // A worker started on a thread inside a stream job (a job's wait helped the pool and took it)
                // claims nothing: the job it would run nests under one that may hold what it needs.
                if IN_JOB.get() == 0 {
                    while let Some(j) = state.claim(None, false) {
                        state.run(j, false);
                    }
                }
                state.serving.fetch_sub(1, Ordering::AcqRel);
            });
        }
    }

    /// Waits for `h`: runs it here if no worker claimed it, else helps the pool meanwhile.
    pub fn wait<R>(&self, h: Handle<R>) -> Result<R, String> {
        let st = &self.state;
        let mut nap = 20e-6f64;
        loop {
            let seen = self.ended();
            if let Some(r) = h.slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                return r.map_err(|_| format!("host stream {}: a job panicked", st.name));
            }
            if let Some(j) = st.claim(Some(h.id), false) {
                st.run(j, true);
                nap = 20e-6;
                continue;
            }
            if on_pool() && help_once() {
                nap = 20e-6;
                continue;
            }
            self.pause(seen, nap);
            nap = (nap * 2.0).min(1e-3);
        }
    }

    /// How many of the stream's jobs have ended: read before a waiter looks for work, and given to [`Stream::pause`].
    pub fn ended(&self) -> u64 {
        *self.state.ended.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Sleeps until a job ends after `seen` was read ([`Stream::ended`]), or `nap` seconds pass: a job that ended
    /// since is not slept through. Bounded, so work woken by something else (the pool's other jobs) is still found.
    pub fn pause(&self, seen: u64, nap: f64) {
        let st = &self.state;
        let ended = st.ended.lock().unwrap_or_else(|p| p.into_inner());
        let _ = st.wake.wait_timeout_while(ended, std::time::Duration::from_secs_f64(nap), |n| *n == seen);
    }

    /// Whether `h`'s job ended (its result is waiting for [`Stream::wait`]).
    pub fn done<R>(&self, h: &Handle<R>) -> bool {
        h.slot.lock().unwrap_or_else(|p| p.into_inner()).is_some()
    }

    /// Runs one of the stream's queued jobs here, largest first, else one of the pool's: whether it ran one. For a
    /// caller that waits on more than one job and has other work between (`plan::issue`).
    pub fn help(&self) -> bool {
        if IN_JOB.get() == 0 && let Some(j) = self.state.claim(None, false) {
            self.state.run(j, true);
            return true;
        }
        on_pool() && help_once()
    }

    /// Whether `h`'s job is still queued: no one has claimed it.
    pub fn queued<R>(&self, h: &Handle<R>) -> bool {
        self.state.queue.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|j| j.id == h.id)
    }

    pub fn counts(&self) -> StreamCounts {
        let st = &self.state;
        StreamCounts { by_caller: st.by_caller.load(Ordering::Relaxed), by_workers: st.by_workers.load(Ordering::Relaxed), peak: st.peak.load(Ordering::Relaxed) }
    }
}

impl Executor {
    /// Runs `f` with a host stream named `name` open (a scope: its jobs may borrow what outlives the call). Jobs still
    /// queued when `f` returns (its caller waited for none of them: an error left early) are dropped unrun.
    pub fn stream<'s, R>(&self, name: &'static str, f: impl FnOnce(&Stream<'s, '_>) -> R) -> R {
        rayon::in_place_scope(|scope: &rayon::Scope<'s>| {
            let state = Arc::new(State {
                name,
                queue: Mutex::new(Vec::new()),
                serving: AtomicUsize::new(0),
                latency: AtomicU64::new(0),
                board: self.board(),
                next: AtomicU64::new(0),
                by_caller: AtomicUsize::new(0),
                by_workers: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                running: AtomicUsize::new(0),
                ended: Mutex::new(0),
                wake: Condvar::new(),
            });
            let stream = Stream { scope, state: Arc::clone(&state) };
            let r = f(&stream);
            // Unwaited jobs are not run: their workers find nothing left to claim.
            for j in std::mem::take(&mut *state.queue.lock().unwrap_or_else(|p| p.into_inner())) {
                state.board.finish(j.ticket, false);
            }
            r
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_runs_every_job_and_gives_each_its_result() {
        let exec = Executor::with(vec![Arc::new(crate::exec::CpuBackend::default())]);
        let data: Vec<u64> = (0..64).collect();
        let sums = exec.stream("test", |s| {
            let hs: Vec<_> = data.chunks(8).map(|c| s.submit(1e-3, move || { std::thread::sleep(std::time::Duration::from_millis(2)); c.iter().sum::<u64>() })).collect();
            hs.into_iter().map(|h| s.wait(h).unwrap()).collect::<Vec<_>>()
        });
        assert_eq!(sums, data.chunks(8).map(|c| c.iter().sum::<u64>()).collect::<Vec<_>>());
    }

    #[test]
    fn jobs_below_the_floor_run_on_the_caller() {
        let exec = Executor::with(vec![Arc::new(crate::exec::CpuBackend::default())]);
        let counts = exec.stream("tiny", |s| {
            let hs: Vec<_> = (0..32).map(|i| s.submit(1e-7, move || i * 2)).collect();
            for (i, h) in hs.into_iter().enumerate() {
                assert_eq!(s.wait(h).unwrap(), i * 2);
            }
            s.counts()
        });
        assert_eq!((counts.by_caller, counts.by_workers), (32, 0), "{counts:?}");
    }

    #[test]
    fn a_worker_inside_a_job_claims_nothing() {
        let exec = Executor::with(vec![Arc::new(crate::exec::CpuBackend::default())]);
        exec.stream("nested", |s| {
            let h = s.submit(1.0, || IN_JOB.get());
            assert_eq!(s.wait(h).unwrap(), 1, "a job runs marked");
        });
        IN_JOB.set(1);
        let st = Executor::with(vec![Arc::new(crate::exec::CpuBackend::default())]);
        let counts = st.stream("marked", |s| {
            let h = s.submit(1.0, || 7);
            let got = s.wait(h).unwrap();
            (got, s.counts())
        });
        IN_JOB.set(0);
        assert_eq!(counts.0, 7);
    }

    #[test]
    fn a_job_that_panics_is_an_error_to_its_waiter() {
        let exec = Executor::with(vec![Arc::new(crate::exec::CpuBackend::default())]);
        let got = exec.stream("panics", |s| {
            let h = s.submit(1.0, || -> u32 { panic!("a test job") });
            s.wait(h)
        });
        assert!(got.is_err());
    }
}
