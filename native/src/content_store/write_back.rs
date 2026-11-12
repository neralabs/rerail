//! Write-back (decision 0102): a store handle's writes that no wave waits for, applied in the
//! background by one writer thread, in the order they were queued.
//!
//! The run hands a write over as a job and goes on to its next wave with what it already holds in
//! memory. SQLite is the durable copy and lags behind; it is never ahead of memory
//! (`sqlite-durable, memory ahead`). The model is consistent for this process:
//!
//! - **Order.** One writer, one queue: a job sees every write queued before it, so a run's
//!   manifest, queued last, names only what is already written (decision 0053).
//! - **Read your writes.** A job names the keys it writes. A read of a key still queued is served
//!   from the value the job left in memory (`pending`), or waits for that job alone (`settle`);
//!   any other read goes to SQLite without waiting.
//! - **Bounded lag.** Jobs carry their size; past `cap_bytes()` queued, the next job waits for the
//!   writer to catch up (back-pressure, 0024 §5), so memory holds at most that much unwritten.
//! - **Durable at the end.** `written` waits for the queue and gives the first error a job met; a
//!   handle's drop waits too, so no queued write is lost when the process ends.
//!
//! A job that finds the store busy (another process's writer held it past the job's own write
//! deadlines, `connection::retry_busy`) is run again whole, after a backoff that starts at
//! [`JOB_BACKOFF_FIRST`] and doubles to [`JOB_BACKOFF_MAX`], until [`JOB_DEADLINE`]: a job is a
//! cache's writes (each an upsert), so running it again writes the same. A job that fails
//! otherwise, or stays busy past the deadline, leaves the store as a cache that lacks that write:
//! the next run misses and recomputes, as when the write never happened. Its error is reported at
//! `written`.
use super::{ContentStore, StoreError, StoreResult};
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

/// The most bytes queued and unwritten: a few chunks' memo entries, 192 MB of the default 1 GB budget, scaled with the
/// budget (`rerail_accel::budget::share`, decision 0126).
fn cap_bytes_default() -> usize {
    rerail_accel::budget::share(192 << 20)
}

/// The queue's cap: 192 MB, or `RERAIL_WRITE_QUEUE_MB` (a measurement aid: how far behind the writer may fall).
fn cap_bytes() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| std::env::var("RERAIL_WRITE_QUEUE_MB").ok().and_then(|v| v.parse::<usize>().ok()).map_or(cap_bytes_default(), |mb| mb << 20))
}

/// The first wait before a busy job runs again: a writer that held the store is often done at once.
pub const JOB_BACKOFF_FIRST: Duration = Duration::from_micros(50);
/// The largest wait between a busy job's runs.
pub const JOB_BACKOFF_MAX: Duration = Duration::from_millis(500);
/// How long a busy job is run again before it is given up (the run's end waits for it meanwhile).
pub const JOB_DEADLINE: Duration = Duration::from_secs(120);

/// What a job writes, named so a read of it can find the job (a digest of the table and its key).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key(pub [u8; 32]);

impl Key {
    /// The key of `parts` (a table's name, then its key's bytes).
    pub fn of(parts: &[&[u8]]) -> Key {
        use sha2::{Digest as _, Sha256};
        let mut hash = Sha256::new();
        for p in parts {
            hash.update((p.len() as u64).to_le_bytes());
            hash.update(p);
        }
        Key(hash.finalize().into())
    }
}

/// A queued write: run on the writer's handle, its own transactions. Run again whole if it finds
/// the store busy (`run_job`), so it writes what it holds without consuming it.
pub type Job = Box<dyn FnMut(&ContentStore) -> StoreResult<()> + Send>;

/// Runs `job` until it does not find the store busy, waiting a jittered backoff between runs from
/// [`JOB_BACKOFF_FIRST`], doubled each time up to [`JOB_BACKOFF_MAX`]; `Busy` once `deadline` has
/// passed. Returns the outcome and the runs that found the store busy.
fn run_job(mut job: impl FnMut() -> StoreResult<()>, deadline: Duration) -> (StoreResult<()>, usize) {
    let started = Instant::now();
    let mut backoff = JOB_BACKOFF_FIRST;
    let mut busy = 0;
    loop {
        match job() {
            Err(StoreError::Busy) => {
                busy += 1;
                let Some(remaining) = deadline.checked_sub(started.elapsed()).filter(|r| !r.is_zero()) else {
                    return (Err(StoreError::Busy), busy);
                };
                std::thread::sleep(super::connection::jittered(backoff).min(remaining));
                backoff = (backoff * 2).min(JOB_BACKOFF_MAX);
            }
            outcome => return (outcome, busy),
        }
    }
}

/// `job` on `store`, run again while it finds the store busy (`run_job`), its error named by `what`.
fn retried(store: &ContentStore, what: &str, job: &mut Job) -> StoreResult<()> {
    let (outcome, busy) = run_job(|| job(store), JOB_DEADLINE);
    outcome.map_err(|e| StoreError::Invalid(format!("{what}: {e}{}", if busy > 0 { format!(" ({busy} runs found the store busy)") } else { String::new() })))
}

type Held = Arc<dyn Any + Send + Sync>;

#[derive(Default)]
struct State {
    /// Bytes queued and not yet written.
    bytes: usize,
    /// Jobs queued and not yet done.
    jobs: usize,
    /// Each key's queued jobs, and the value the last one left for readers.
    keys: HashMap<Key, (usize, Option<Held>)>,
    /// The first error a job met since the last `written`.
    error: Option<String>,
    /// Jobs run, bytes written and the writer's CPU, since the writer started.
    done: (usize, usize, f64),
    /// Transactions the writer committed its batches in.
    batches: usize,
    /// Whether the writer holds a batch's transaction open now.
    open: bool,
    /// By what a job writes: jobs queued, their bytes, and the writer's CPU running them.
    by_what: HashMap<&'static str, (usize, usize, f64)>,
    /// The most bytes queued at once, and the queue's back-pressure: how often it held a producer, and how long.
    peak: usize,
    held: (usize, f64),
    /// Cache writes dropped for a full queue (`write_back_unless_full`): how many, and their bytes.
    dropped: (usize, usize),
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// A queued job, the bytes it holds, the keys it writes, the holder it pins its sources under, and how it runs.
type Queued = (Job, usize, Vec<Key>, Option<(String, i64)>, Run, &'static str);

/// How the writer runs a job.
#[derive(Clone, Copy, PartialEq)]
enum Run {
    /// In the open batch.
    Batched,
    /// After the batch commits, in transactions of its own (`write_back_alone`).
    Alone,
    /// Nothing: it commits the batch, for a wait for the writer (`commit_now`). Not counted as a write.
    Commit,
}

/// How long the writer's batch stays open with no job queued: jobs come all through a run, so a run's writes commit
/// in about one transaction.
const BATCH_IDLE: Duration = Duration::from_millis(500);

/// The bytes a batch holds before it commits: half the queue's cap, so the back-pressure is never waiting on it.
fn batch_bytes() -> usize {
    cap_bytes_default() / 2
}

/// The bytes under which a job is queued past the cap (back-pressure holds only a job of this size or more).
fn small_job() -> usize {
    cap_bytes_default() / 64
}

struct Lane {
    tx: mpsc::Sender<Queued>,
    thread: std::thread::JoinHandle<()>,
}

/// A store's writer, one for every handle of the store in this process (by its path), so every handle reads what
/// any queued. Started by the first job; ended, its queue written, when the last handle (or `Hold`) goes.
#[derive(Default)]
pub(crate) struct Writer {
    shared: Shared,
    lane: Mutex<Option<Lane>>,
    /// The handles and holds of the store.
    handles: std::sync::atomic::AtomicUsize,
    /// The identity of the store its handles opened (`store_identity`); unset while only a `Hold` keeps it. A store
    /// created again at the path has a writer of its own: its jobs name its own atoms, and the old store's handles
    /// still write theirs.
    identity: std::sync::OnceLock<i64>,
}

/// The process's writers, by store path.
fn writers() -> &'static Mutex<HashMap<PathBuf, Arc<Writer>>> {
    static WRITERS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Arc<Writer>>>> = std::sync::OnceLock::new();
    WRITERS.get_or_init(Default::default)
}

/// The writer of the store at `path` (of `identity`, for a handle; a `Hold` takes the path's), counted as one more of
/// its handles. The path's writer is another store's when that store's identity differs: then this one gets its own,
/// which the path names from now on.
pub(super) fn register(path: &Path, identity: Option<i64>) -> Arc<Writer> {
    let mut all = writers().lock().expect("the writers");
    let entry = all.entry(path.to_path_buf()).or_default();
    if let Some(identity) = identity
        && *entry.identity.get_or_init(|| identity) != identity
    {
        let fresh = Writer::default();
        let _ = fresh.identity.set(identity);
        *entry = Arc::new(fresh);
    }
    let w = Arc::clone(entry);
    w.handles.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    w
}

/// One handle of the store at `path` gone: the last ends its writer, the queue written first.
fn unregister(path: &Path, w: &Arc<Writer>) {
    // Under the registry's lock: a handle opened meanwhile finds this writer, never a new one that cannot see its
    // queue (the writer's jobs never take this lock).
    let mut all = writers().lock().expect("the writers");
    if w.handles.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) != 1 {
        return;
    }
    if all.get(path).is_some_and(|x| Arc::ptr_eq(x, w)) {
        all.remove(path);
    }
    let lane = w.lane.lock().expect("the writer").take();
    if let Some(Lane { tx, thread }) = lane {
        let (jobs, bytes) = w.shared.state.lock().map(|s| (s.jobs, s.bytes)).unwrap_or_default();
        let t = std::time::Instant::now();
        drop(tx);
        let _ = thread.join();
        // What the end of the run waited for: the queue the waves left behind them.
        let waited = t.elapsed().as_secs_f64();
        if waited > 0.05 && std::env::var_os("RERAIL_PROFILE").is_some() {
            eprintln!("rerail: the writer drained {jobs} jobs, {:.1} MB, in {waited:.3} s after the run's last handle", bytes as f64 / 1048576.0);
        }
    }
    if let Some(e) = w.shared.state.lock().ok().and_then(|mut s| s.error.take()) {
        eprintln!("rerail: a write queued behind the run failed: {e}");
    }
}

/// Keeps the store at a path's writer while it lives, as a handle would (`hold`): a run holds it from its start,
/// so the handles it opens and closes on the way never end the writer, and its end waits for the queue.
pub struct Hold(PathBuf, Arc<Writer>);

impl Drop for Hold {
    fn drop(&mut self) {
        unregister(&self.0, &self.1);
    }
}

/// `Hold` of the store at `path`: the writer of the store a handle opens there next, or has open.
pub fn hold(path: &Path) -> Hold {
    Hold(path.to_path_buf(), register(path, None))
}

/// Each kind of job's count, bytes and CPU on the writer's thread, in this process (the profile's write-back).
static BY_KIND: std::sync::LazyLock<Mutex<std::collections::BTreeMap<&'static str, (usize, usize, f64)>>> = std::sync::LazyLock::new(Default::default);

/// Each kind of job written so far in this process: count, bytes and the writer's CPU, costliest first.
pub fn written_by_kind() -> Vec<(&'static str, usize, usize, f64)> {
    let mut v: Vec<_> = BY_KIND.lock().expect("the writer's account").iter().map(|(k, (n, b, c))| (*k, *n, *b, *c)).collect();
    v.sort_by(|a, b| b.3.total_cmp(&a.3));
    v
}

/// The writer's account since it started: jobs, bytes and its thread's CPU (user + sys).
#[derive(Clone, Copy, Debug, Default)]
pub struct Written {
    pub jobs: usize,
    pub bytes: usize,
    pub cpu: f64,
    /// The transactions they were committed in.
    pub batches: usize,
}

impl ContentStore {
    fn writer(&self) -> Option<&Arc<Writer>> {
        self.write_back.as_ref()
    }

    /// Queues `job` behind every write queued before it, and returns without waiting for SQLite.
    /// `bytes` is what it holds until written; `keys` are what it writes, each with the value a
    /// read of it gets meanwhile (`None`: a read waits for the job). The writer's own handle (in a
    /// job) writes at once.
    ///
    /// The writer drains what is queued into one transaction (`ContentStore::in_one_transaction`), each job a
    /// savepoint in it, and a read of a key waits for its batch's commit.
    pub fn write_back(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Job) {
        self.queue(what, bytes, keys, job, Run::Batched, false);
    }

    /// [`Self::write_back`] for a job that must commit before it goes on (it starts work on another connection that
    /// reads what it wrote): the batch before it commits first, and it runs in transactions of its own.
    pub fn write_back_alone(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Job) {
        self.queue(what, bytes, keys, job, Run::Alone, false);
    }

    /// [`write_back`](Self::write_back) for a cache's write that may go missing (a tape and its source: the next run
    /// parses the file again): where the queue is full it is dropped, counted (`dropped`), rather than waited for, so
    /// no wave waits for the writer (decision 0104). Whether it was queued.
    pub fn write_back_unless_full(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Job) -> bool {
        self.queue(what, bytes, keys, job, Run::Batched, true)
    }

    /// The cache writes dropped for a full queue since the writer started: how many, and their bytes.
    pub fn dropped(&self) -> (usize, usize) {
        self.writer().map_or((0, 0), |w| w.shared.state.lock().expect("the writer's state").dropped)
    }

    fn queue(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Job, run: Run, droppable: bool) -> bool {
        let mut job = job;
        let Some(w) = self.writer() else {
            if let Err(e) = retried(self, what, &mut job) {
                eprintln!("rerail: the write failed: {e}");
            }
            return true;
        };
        let mut job: Job = Box::new(move |store| {
            let t0 = thread_cpu();
            let out = retried(store, what, &mut job);
            let mut by = BY_KIND.lock().expect("the writer's account");
            let e = by.entry(what).or_default();
            e.0 += 1;
            e.1 += bytes;
            e.2 += thread_cpu() - t0;
            out
        });
        {
            let mut s = w.shared.state.lock().expect("the writer's state");
            // Back-pressure: past the cap, wait for the writer (never for an empty queue). A small job never waits:
            // its wait frees next to nothing, and holds the wave behind a large job the writer is writing.
            let full = |s: &State| bytes >= small_job() && s.jobs > 0 && s.bytes + bytes > cap_bytes();
            // A cache's write gives way at half the cap: the other half is for the writes that may not go missing
            // (frames, memo entries), so they find room rather than wait behind tapes.
            if droppable && s.jobs > 0 && s.bytes + bytes > cap_bytes() / 2 {
                s.dropped = (s.dropped.0 + 1, s.dropped.1 + bytes);
                return false;
            }
            let t = full(&s).then(std::time::Instant::now);
            // The first hold says what fills the queue (a strict run ends at it).
            static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if t.is_some() && !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let mut rows: Vec<_> = s.by_what.iter().map(|(w, (j, b, c))| (*w, *j, *b, *c)).collect();
                rows.sort_by(|a, b| b.2.cmp(&a.2));
                let rows: Vec<String> = rows.iter().map(|(w, j, b, c)| format!("{w}: {j} jobs, {:.1} MB, {c:.3} s", *b as f64 / 1048576.0)).collect();
                eprintln!("rerail: the writer's queue is full ({:.1} MB in {} jobs; {what} brings {:.1} MB): queued so far, by what: {}", s.bytes as f64 / 1048576.0, s.jobs, bytes as f64 / 1048576.0, rows.join("; "));
            }
            while full(&s) {
                s = w.shared.changed.wait(s).expect("the writer's state");
            }
            if let Some(t) = t {
                let waited = t.elapsed().as_secs_f64();
                s.held = (s.held.0 + 1, s.held.1 + waited);
                super::hazards::wait_in_wave("the queue's back-pressure", waited);
            }
            s.bytes += bytes;
            s.jobs += 1;
            s.peak = s.peak.max(s.bytes);
            let entry = s.by_what.entry(what).or_default();
            entry.0 += 1;
            entry.1 += bytes;
            for (key, held) in &keys {
                let slot = s.keys.entry(*key).or_insert((0, None));
                slot.0 += 1;
                slot.1 = held.clone();
            }
        }
        let keys: Vec<Key> = keys.into_iter().map(|(k, _)| k).collect();
        let holder = self.holder.lock().clone();
        // No threads in the browser build: written now, in order.
        if cfg!(target_family = "wasm") {
            let outcome = job(self);
            finish(&w.shared, bytes, &keys, outcome, 0.0, run != Run::Commit);
            return true;
        }
        let mut lane = w.lane.lock().expect("the writer");
        let lane = lane.get_or_insert_with(|| self.start_writer(w));
        if let Err(mpsc::SendError((mut job, bytes, keys, _, _, _))) = lane.tx.send((job, bytes, keys, holder, run, what)) {
            // The writer is gone (it panicked): written here, in order, as the writer would have.
            let outcome = job(self);
            finish(&w.shared, bytes, &keys, outcome, 0.0, run != Run::Commit);
        }
        true
    }

    fn start_writer(&self, w: &Arc<Writer>) -> Lane {
        let (tx, rx) = mpsc::channel::<Queued>();
        let w = Arc::clone(w);
        let store = self.sibling();
        let thread = rerail_accel::exec::service("rerail-store-writer", None, move || {
                // What this thread sends to SQLite no wave waits for.
                super::hazards::behind_the_run();
                let mut next: Option<Queued> = None;
                loop {
                    let Some(first) = next.take().or_else(|| rx.recv().ok()) else { break };
                    let before = thread_cpu();
                    if first.4 != Run::Batched {
                        let (mut job, bytes, keys, holder, run, what) = first;
                        *store.holder.lock() = holder;
                        let _span = super::hazards::span(what);
                        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&store)))
                            .unwrap_or_else(|_| Err(StoreError::Invalid("a queued write panicked".into())));
                        w.shared.state.lock().expect("the writer's state").by_what.entry(what).or_default().2 += thread_cpu() - before;
                        finish(&w.shared, bytes, &keys, outcome, thread_cpu() - before, run != Run::Commit);
                        continue;
                    }
                    // One transaction while jobs keep coming: it commits once the queue is idle for `BATCH_IDLE`, holds
                    // `batch_bytes`, or a job comes that runs alone (a wait for the writer sends one: `written`).
                    let mut batch = vec![first];
                    let run = |batch: &mut Vec<Queued>, next: &mut Option<Queued>| -> Vec<StoreResult<()>> {
                        let mut outcomes = Vec::new();
                        let mut bytes = 0;
                        loop {
                            for (job, b, _, holder, _, what) in &mut batch[outcomes.len()..] {
                                *store.holder.lock() = holder.clone();
                                bytes += *b;
                                let c0 = thread_cpu();
                                let _span = super::hazards::span(*what);
                                outcomes.push(
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&store)))
                                        .unwrap_or_else(|_| Err(StoreError::Invalid("a queued write panicked".into()))),
                                );
                                w.shared.state.lock().expect("the writer's state").by_what.entry(*what).or_default().2 += thread_cpu() - c0;
                            }
                            if bytes >= batch_bytes() {
                                return outcomes;
                            }
                            match rx.recv_timeout(BATCH_IDLE) {
                                Ok(q) if q.4 != Run::Batched => {
                                    *next = Some(q);
                                    return outcomes;
                                }
                                Ok(q) => batch.push(q),
                                Err(_) => return outcomes,
                            }
                        }
                    };
                    // The batch's span holds its jobs' and its commit.
                    let batch_span = super::hazards::span("a batch, in one transaction");
                    let outcomes = match store.in_one_transaction(|| {
                        w.shared.state.lock().expect("the writer's state").open = true;
                        let outcomes = run(&mut batch, &mut next);
                        w.shared.state.lock().expect("the writer's state").open = false;
                        outcomes
                    }) {
                        Ok(outcomes) => outcomes,
                        Err(e) => batch.iter().map(|_| Err(StoreError::Invalid(format!("the batch's commit: {e}")))).collect(),
                    };
                    drop(batch_span);
                    let cpu = (thread_cpu() - before) / batch.len() as f64;
                    w.shared.state.lock().expect("the writer's state").batches += 1;
                    for ((_, bytes, keys, _, _, _), outcome) in batch.into_iter().zip(outcomes) {
                        finish(&w.shared, bytes, &keys, outcome, cpu, true);
                    }
                }
            })
            .expect("the store's writer thread");
        Lane { tx, thread }
    }

    /// The value a queued job left for `key`, while the job is queued.
    pub fn pending<T: Any + Send + Sync>(&self, key: &Key) -> Option<Arc<T>> {
        let s = self.writer()?.shared.state.lock().expect("the writer's state");
        s.keys.get(key).and_then(|(_, held)| held.clone()).and_then(|h| h.downcast::<T>().ok())
    }

    /// Waits for the jobs that write `key`, if any is queued; a read of anything else never waits.
    pub fn settle(&self, key: &Key) {
        let Some(w) = self.writer() else { return };
        if w.shared.state.lock().expect("the writer's state").keys.contains_key(key) {
            self.commit_now();
        }
        let mut s = w.shared.state.lock().expect("the writer's state");
        let t = s.keys.contains_key(key).then(std::time::Instant::now);
        while s.keys.contains_key(key) {
            s = w.shared.changed.wait(s).expect("the writer's state");
        }
        if let Some(t) = t {
            super::hazards::wait_in_wave("a read of a queued write (settle)", t.elapsed().as_secs_f64());
        }
    }

    /// Waits for every queued job: the store then holds every write queued so far. The first error
    /// a job met since the last call.
    pub fn written(&self) -> StoreResult<()> {
        let Some(w) = self.writer() else { return Ok(()) };
        if w.shared.state.lock().expect("the writer's state").jobs > 0 {
            self.commit_now();
        }
        let mut s = w.shared.state.lock().expect("the writer's state");
        let t = (s.jobs > 0).then(std::time::Instant::now);
        while s.jobs > 0 {
            s = w.shared.changed.wait(s).expect("the writer's state");
        }
        if let Some(t) = t {
            super::hazards::wait_in_wave("every queued write (written)", t.elapsed().as_secs_f64());
        }
        match s.error.take() {
            Some(e) => Err(StoreError::Invalid(format!("a write queued behind the run failed: {e}"))),
            None => Ok(()),
        }
    }

    /// Before a write of this handle's own (not queued): the writer's open batch commits, so the write waits for the
    /// jobs it holds, not for the batch to go idle. Every such write is one to move behind the writer.
    pub(super) fn yield_writer(&self) {
        if self.writer().is_some_and(|w| w.shared.state.lock().expect("the writer's state").open) {
            self.commit_now();
        }
    }

    /// Ends the writer's open batch: an empty job that runs alone, queued behind it.
    fn commit_now(&self) {
        self.queue("a wait for the writer", 0, Vec::new(), Box::new(|_| Ok(())), Run::Commit, false);
    }

    /// What the writer has done since it started.
    pub fn written_so_far(&self) -> Written {
        let Some(w) = self.writer() else { return Written::default() };
        let s = w.shared.state.lock().expect("the writer's state");
        Written { jobs: s.done.0, bytes: s.done.1, cpu: s.done.2, batches: s.batches }
    }

    /// The writer's work by what each job writes, `(what, jobs, bytes, writer CPU)`, costliest first; the most bytes
    /// queued at once; and the back-pressure's holds of a producer, `(count, seconds)`.
    pub fn written_by_what(&self) -> (Vec<(&'static str, usize, usize, f64)>, usize, (usize, f64)) {
        let Some(w) = self.writer() else { return (Vec::new(), 0, (0, 0.0)) };
        let s = w.shared.state.lock().expect("the writer's state");
        let mut rows: Vec<_> = s.by_what.iter().map(|(w, (j, b, c))| (*w, *j, *b, *c)).collect();
        rows.sort_by(|a, b| b.3.total_cmp(&a.3));
        (rows, s.peak, s.held)
    }

    /// This handle gone (its drop): the last of the store's ends the writer, its queue written.
    pub(super) fn end_writer(&mut self) {
        if let Some(w) = self.write_back.take() {
            unregister(&self.database_path, &w);
        }
    }
}

fn finish(shared: &Shared, bytes: usize, keys: &[Key], outcome: StoreResult<()>, cpu: f64, counted: bool) {
    let mut s = shared.state.lock().expect("the writer's state");
    s.bytes -= bytes;
    s.jobs -= 1;
    s.done = (s.done.0 + usize::from(counted), s.done.1 + bytes, s.done.2 + cpu);
    for key in keys {
        if let Some(slot) = s.keys.get_mut(key) {
            slot.0 -= 1;
            if slot.0 == 0 {
                s.keys.remove(key);
            }
        }
    }
    if let Err(e) = outcome
        && s.error.is_none()
    {
        s.error = Some(e.to_string());
    }
    drop(s);
    shared.changed.notify_all();
}

/// This thread's CPU (user + sys), in seconds.
fn thread_cpu() -> f64 {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::thread_basic_info = unsafe { std::mem::zeroed() };
        let mut count = (std::mem::size_of::<libc::thread_basic_info>() / std::mem::size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
        // SAFETY: THREAD_BASIC_INFO fills a `thread_basic_info` of `count` integers for this thread's own port.
        let port = unsafe { libc::pthread_mach_thread_np(libc::pthread_self()) };
        unsafe { libc::thread_info(port, libc::THREAD_BASIC_INFO as libc::thread_flavor_t, (&raw mut info).cast(), &mut count) };
        let t = |v: libc::time_value_t| f64::from(v.seconds) + f64::from(v.microseconds) / 1e6;
        t(info.user_time) + t(info.system_time)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: `u` is a valid rusage for the call to fill.
        unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut u) };
        let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
        t(u.ru_utime) + t(u.ru_stime)
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job that finds the store busy is run again at once, then after waits that double: three busy runs, then
    /// written, cost well under a millisecond's backoff each to start.
    #[test]
    fn a_busy_job_is_run_again_after_a_fast_backoff() {
        let mut runs = 0;
        let started = Instant::now();
        let (outcome, busy) = run_job(
            || {
                runs += 1;
                if runs <= 3 { Err(StoreError::Busy) } else { Ok(()) }
            },
            JOB_DEADLINE,
        );
        assert!(outcome.is_ok());
        assert_eq!((runs, busy), (4, 3));
        // 50 + 100 + 200 µs of backoff at most (jittered to half or more), and a slow machine's scheduling.
        assert!(started.elapsed() < Duration::from_millis(250), "{:?}", started.elapsed());
    }

    /// A job busy past its deadline is given up, busy; any other error is not run again.
    #[test]
    fn a_job_busy_past_its_deadline_or_failing_otherwise_is_given_up() {
        let mut runs = 0;
        let (outcome, busy) = run_job(|| { runs += 1; Err(StoreError::Busy) }, Duration::from_millis(20));
        assert!(matches!(outcome, Err(StoreError::Busy)));
        assert!(busy == runs && runs > 1, "{runs} runs");
        let mut runs = 0;
        let (outcome, busy) = run_job(|| { runs += 1; Err(StoreError::Invalid("no".into())) }, JOB_DEADLINE);
        assert!(matches!(outcome, Err(StoreError::Invalid(_))));
        assert_eq!((runs, busy), (1, 0));
    }

    fn store(name: &str) -> (PathBuf, ContentStore) {
        let dir = std::env::temp_dir().join(format!("rerail-write-back-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.sqlite");
        let store = ContentStore::open(&path).unwrap();
        (dir, store)
    }

    /// Jobs run in their order on the writer; a read of a queued key gets the job's value, or waits for it alone;
    /// `written` waits for all and gives the first error; another handle of the store shares the queue.
    #[test]
    fn queued_writes_run_in_order_and_reads_see_them() {
        let (dir, store) = store("order");
        let order = Arc::new(Mutex::new(Vec::new()));
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let key = Key::of(&[b"t", b"1"]);
        let (o1, o2) = (Arc::clone(&order), Arc::clone(&order));
        store.write_back("one", 8, vec![(key, Some(Arc::new(7u64) as Held))], Box::new(move |_| {
            go_rx.recv().unwrap();
            o1.lock().unwrap().push(1);
            Ok(())
        }));
        store.write_back("two", 8, vec![], Box::new(move |_| {
            o2.lock().unwrap().push(2);
            Err(StoreError::Invalid("second".into()))
        }));
        // Queued, not written: the value is the job's, from any handle of the store.
        let other = ContentStore::open(&store.database_path).unwrap();
        assert_eq!(other.pending::<u64>(&key).as_deref(), Some(&7));
        assert!(order.lock().unwrap().is_empty());
        go_tx.send(()).unwrap();
        other.settle(&key);
        assert_eq!(other.pending::<u64>(&key), None);
        let err = store.written().unwrap_err().to_string();
        assert!(err.contains("second"), "{err}");
        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
        assert!(store.written().is_ok());
        assert_eq!(store.written_so_far().jobs, 2);
        drop((store, other));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A cache's write that would wait for a full queue is dropped instead (`write_back_unless_full`): it returns at
    /// once, is never run, and is counted; with room, it is queued as any write. A wave never waits for the writer.
    #[test]
    fn a_cache_write_to_a_full_queue_is_dropped_not_waited_for() {
        let (dir, store) = store("drop");
        let (open, gate) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        store.write_back("slow", super::cap_bytes(), vec![], Box::new(move |_| {
            let _ = gate.lock().unwrap().recv();
            Ok(())
        }));
        let ran = Arc::new(Mutex::new(0));
        let r = Arc::clone(&ran);
        let t = std::time::Instant::now();
        let queued = store.write_back_unless_full("cache", small_job(), vec![], Box::new(move |_| {
            *r.lock().unwrap() += 1;
            Ok(())
        }));
        assert!(!queued && t.elapsed() < std::time::Duration::from_millis(100), "dropped at once");
        open.send(()).unwrap();
        store.written().unwrap();
        assert_eq!(*ran.lock().unwrap(), 0, "a dropped write never runs");
        assert_eq!(store.dropped(), (1, small_job()));
        let r = Arc::clone(&ran);
        assert!(store.write_back_unless_full("cache", small_job(), vec![], Box::new(move |_| {
            *r.lock().unwrap() += 1;
            Ok(())
        })));
        store.written().unwrap();
        assert_eq!(*ran.lock().unwrap(), 1);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The last handle's drop writes the queue; a `Hold` keeps the writer past a handle's drop.
    #[test]
    fn the_last_handle_writes_the_queue() {
        let (dir, store) = store("last");
        let path = store.database_path.clone();
        let hold = hold(&path);
        let done = Arc::new(Mutex::new(false));
        let d = Arc::clone(&done);
        store.write_back("blob", 0, vec![], Box::new(move |s| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            s.put_stat_blob([1; 32], b"kept")?;
            *d.lock().unwrap() = true;
            Ok(())
        }));
        drop(store);
        drop(hold);
        assert!(*done.lock().unwrap());
        let again = ContentStore::open(&path).unwrap();
        assert_eq!(again.stat_blob([1; 32]).unwrap().as_deref(), Some(&b"kept"[..]));
        drop(again);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A store deleted and created again at its path while a handle of the old one lives has a writer of its own: its
    /// jobs run on a handle of the new store (its atoms), not queued behind the old store's.
    #[test]
    fn a_store_created_again_at_its_path_has_its_own_writer() {
        let (dir, old) = store("again");
        let path = old.database_path.clone();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let order = Arc::new(Mutex::new(Vec::new()));
        let o = Arc::clone(&order);
        old.write_back("old", 0, vec![], Box::new(move |_| {
            go_rx.recv().unwrap();
            o.lock().unwrap().push("old");
            Ok(())
        }));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        let new = ContentStore::open(&path).unwrap();
        assert!(!Arc::ptr_eq(old.writer().unwrap(), new.writer().unwrap()));
        let (o, atoms) = (Arc::clone(&order), Arc::clone(&new.atoms));
        new.write_back("new", 0, vec![], Box::new(move |s| {
            assert!(Arc::ptr_eq(&s.atoms, &atoms), "the new store's atoms");
            o.lock().unwrap().push("new");
            Ok(())
        }));
        // Written while the old store's job still waits.
        new.written().unwrap();
        go_tx.send(()).unwrap();
        old.written().unwrap();
        assert_eq!(*order.lock().unwrap(), ["new", "old"]);
        drop((old, new));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
