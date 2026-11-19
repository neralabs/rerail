//! Write-back (decision 0102): a store handle's writes that no wave waits for, applied in the
//! background by one writer thread, in the order they were queued, a wave at a time (decision 0259).
//!
//! The run hands a write over as a job and goes on to its next wave with what it already holds in
//! memory. SQLite is the durable copy and lags behind; it is never ahead of memory
//! (`sqlite-durable, memory ahead`). The model is consistent for this process:
//!
//! - **Order.** One writer, one queue: a job sees every write queued before it, so a run's
//!   manifest, queued last, names only what is already written (decision 0053).
//! - **A wave at a time.** A run's jobs join its wave still being written (`run_waves`), held
//!   until a boundary of the run seals it (`end_wave`, `seals`); the writer takes each sealed wave
//!   in one transaction, so the store holds a run's waves whole, each or none. No commit is made by
//!   size or idleness. A job queued outside a run is a wave of its own.
//! - **Read your writes.** A job names the keys it writes. A read of a key still queued is served
//!   from the value the job left in memory (`pending`), or waits for that job alone (`settle`);
//!   any other read goes to SQLite without waiting.
//! - **Bounded lag.** Jobs carry their size; past `cap_bytes()` queued, the next job waits for the
//!   writer to take the waves sealed (back-pressure, 0024 §5). The wave still being written is not
//!   waited for: only its boundary seals it.
//! - **Durable at the end.** `written` seals the run's wave, waits for every wave sealed and gives
//!   the first error a job met; a handle's drop seals and waits too, so no queued write is lost
//!   when the process ends.
//!
//! A job that finds the store busy (another process's writer held it past the job's own write
//! deadlines, `connection::retry_busy`) is run again whole, after a backoff that starts at
//! [`JOB_BACKOFF_FIRST`] and doubles to [`JOB_BACKOFF_MAX`], until [`JOB_DEADLINE`]: a job is a
//! cache's writes (each an upsert), so running it again writes the same. A job that fails
//! otherwise, or stays busy past the deadline, leaves the store as a cache that lacks that write:
//! the next run misses and recomputes, as when the write never happened. Its error is reported at
//! `written`.
#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
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

/// The WAL's size past which the writer's checkpointer copies it into the database (decision 0308).
const CHECKPOINT_WAL_BYTES: u64 = 64 << 20;

/// The writer's page cache, KiB (decision 0308): a wave's changed pages, where the readers' connections keep 1 MB.
const WRITER_CACHE_KIB: i64 = 32 << 10;

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

/// What a job's rows hold of this process's atoms (decision 0170): after a fork, its ids and the store's differ.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Syms {
    /// No symbols: written as ever.
    None,
    /// Symbols mapped to the store's where they are written (`Atoms::to_store`).
    Mapped,
    /// Symbols as this process numbers them, or not known: after a fork the job is dropped, as a cache's miss.
    Unmapped,
}

/// A queued write, as the writer runs it: its own transactions, run again whole if it finds the store busy
/// (`run_job`), so it writes what it holds without consuming it. Made only from a [`Work`] (decision 0306 §8).
type Job = Box<dyn FnMut(&ContentStore) -> StoreResult<()> + Send>;

/// A write queued for the store's writer (decision 0306 §8): data of a closed set of kinds, each run by the store's
/// code. A run's small writes are its record's; the bulk and store-internal writes are the store's own variants. No
/// caller hands the queue code: what it writes is what it holds.
pub struct Work(pub(super) Kind);

pub(super) enum Kind {
    /// A record (decision 0306): its string and byte area, applied by its items' registered kinds.
    Record { text: String, bytes: Vec<u8> },
    /// A space's memo entries in their segments (`queue_memo_segments`).
    Memo(Box<super::held::memo_pack::MemoWrite>),
    /// A space's next kept file (`queue_kept`, decisions 0401 F6, 0456 K4).
    Kept(Box<super::kept_facts::KeptWrite>),
    /// Sources and tapes gathered into segments, or each in its own rows where no segment could be placed.
    Objects(Box<super::object_segments::ObjectsWrite>),
    /// The pending atoms; `queued` clears the flag that keeps a second flush from being queued meanwhile.
    Atoms { queued: bool },
    /// A test's job: the queue's own tests block or order the writer.
    #[cfg(test)]
    Test(Job),
}

impl Work {
    /// A record's string and byte area (`record`).
    pub(super) fn record(text: String, bytes: Vec<u8>) -> Work {
        Work(Kind::Record { text, bytes })
    }

    /// A flush of the pending atoms.
    pub(super) fn atoms(queued: bool) -> Work {
        Work(Kind::Atoms { queued })
    }

    #[cfg(test)]
    pub(crate) fn test(job: Job) -> Work {
        Work(Kind::Test(job))
    }

    fn run(&mut self, store: &ContentStore) -> StoreResult<()> {
        match &mut self.0 {
            Kind::Record { text, bytes } => super::record::apply(store, text, bytes),
            Kind::Memo(w) => w.run(store),
            Kind::Kept(w) => w.run(store),
            Kind::Objects(w) => w.run(store),
            Kind::Atoms { queued } => {
                if *queued {
                    store.atoms.flush_queued.store(false, std::sync::atomic::Ordering::Release);
                }
                store.flush_atoms()
            }
            #[cfg(test)]
            Kind::Test(job) => job(store),
        }
    }

    fn into_job(mut self) -> Job {
        Box::new(move |store| self.run(store))
    }
}

/// Writes a run gathers, many small ones made one job (decision 0295 step 5: a batch's bytes in one segment). Held
/// beside the run's wave still being written, and closed (placed, then queued in that wave as one job) once it holds
/// [`gather_cap`] bytes or the wave is sealed. A read finds a gathered write as it finds a queued one (`pending`).
pub trait Gather: Any + Send {
    /// The keys gathered, each with the value a read of it gets meanwhile.
    fn keys(&self) -> Vec<(Key, Option<Held>)>;
    /// The value a read of `key` gets, where it is gathered.
    fn held(&self, key: &Key) -> Option<Held>;
    /// The bytes it holds in memory.
    fn bytes(&self) -> usize;
    /// Whether it holds `key` (a key may hold no value for reads: a run's record's).
    fn holds(&self, key: &Key) -> bool {
        self.held(key).is_some()
    }
    /// Closed only at its run's end (or a `settle` of a key it holds), not at every seal: a run's record (decision
    /// 0306), queued after the run's other writes.
    fn until_end(&self) -> bool {
        false
    }
    /// Placed and made one job; `None` when it gathered nothing.
    fn close(self: Box<Self>) -> Option<Closed>;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A gather closed: the job its writes are, the bytes it holds until written, and its keys' values for reads now.
pub struct Closed {
    pub what: &'static str,
    pub bytes: usize,
    pub keys: Vec<(Key, Option<Held>)>,
    pub syms: Syms,
    pub job: Work,
}

/// The bytes a gather holds in memory before it is closed: 16 MB of the default 1 GB budget, scaled with it.
pub(super) fn gather_cap() -> usize {
    rerail_accel::budget::share(16 << 20)
}

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

pub type Held = Arc<dyn Any + Send + Sync>;

/// Where a run's wave of writes ends (decision 0259 §5). A wave is what the run queued between two boundaries that
/// seal, and the store takes it in one transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boundary {
    /// Before the waves: the combined program, the prepared sources, the plan holds.
    Before,
    /// A chunk's file-local part done (`process`): its tapes, tree counts, memo entries.
    Chunk,
    /// A round of the cone (decision 0145).
    ConeRound,
    /// The file-local part done, before the cross part: what it left queued (the files' stats) is written while the
    /// cross part runs (decision 0296).
    Local,
    /// The cross part's rows derived: its new atoms (names it made) are written while its kept part is framed
    /// (decision 0296).
    CrossDerived,
    /// The cross part done: its kept strata's frames, retained rows, rounds, statistics.
    Cross,
    /// The run's end: the cross memo, the manifest, the atoms, the kept plans.
    End,
}

/// Whether `boundary` seals the run's wave: the one place that says where waves end. A boundary that does not seal
/// leaves its writes in the wave still being written.
pub fn seals(boundary: Boundary) -> bool {
    !matches!(boundary, Boundary::ConeRound)
}

thread_local! {
    /// The run whose waves this thread's writes join (`run_waves`); 0 outside a run, where each write is a wave of
    /// its own. Carried to the run's jobs on other threads (`run_state`).
    pub static WAVES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// While it lives, this thread's writes (and its run's jobs') join a run's waves of their own (`WAVES`); dropped, the
/// run's wave still being written is sealed, in every store's writer, and the thread's run before is back. Its end
/// does not wait for the writer: `written` does.
pub struct Waves {
    run: u64,
    before: u64,
}

/// A run's waves of its own, from here (`Waves`).
pub fn run_waves() -> Waves {
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Waves { run, before: WAVES.with(|w| w.replace(run)) }
}

impl Drop for Waves {
    fn drop(&mut self) {
        seal_everywhere(self.run, Seal::Finished);
        WAVES.with(|w| w.set(self.before));
    }
}

/// Ends this thread's run's wave at `boundary`, in the writer of every store it wrote to, where `seals` says the
/// boundary seals one (decision 0259).
pub fn end_wave(boundary: Boundary) {
    let run = WAVES.with(std::cell::Cell::get);
    if run != 0 && seals(boundary) {
        seal_everywhere(run, if boundary == Boundary::End { Seal::End } else { Seal::Wave });
    }
}

/// What a seal closes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seal {
    /// The run's wave, and its gathers but its record.
    Wave,
    /// Its record too: the run's end, or a read of a key it holds (decision 0306).
    End,
    /// The run is over: its record, and then what the writer keeps for it (its named sources).
    Finished,
}

/// Seals `run`'s wave still being written in every store's writer.
fn seal_everywhere(run: u64, how: Seal) {
    let all: Vec<Arc<Writer>> = writers().lock().expect("the writers").values().cloned().collect();
    for w in all {
        w.seal(run, how);
    }
}

#[derive(Default)]
struct State {
    /// Bytes queued and not yet written.
    bytes: usize,
    /// Of them, in waves sealed: what the writer is taking now, and what back-pressure may wait for.
    sealed_bytes: usize,
    /// Jobs queued and not yet done.
    jobs: usize,
    /// Each key's queued jobs, the value the last one left for readers, and the run whose wave holds the last.
    keys: HashMap<Key, (usize, Option<Held>, u64)>,
    /// Each run's wave still being written: its jobs, held here until a boundary seals it.
    open: HashMap<u64, Vec<Queued>>,
    /// Waves sealed, and waves the writer has finished: a wait for the writer waits for the waves sealed before it.
    sealed: u64,
    applied: u64,
    /// The first error a job met since the last `written`.
    error: Option<String>,
    /// Jobs run, bytes written and the writer's CPU, since the writer started.
    done: (usize, usize, f64),
    /// Transactions the writer committed its waves in.
    batches: usize,
    /// By what a job writes: jobs queued, the bytes they hold until written, the writer's CPU running them, and the
    /// bytes of the frames they encoded (`compression::encoded_here`; rows not framed are not in it).
    by_what: HashMap<&'static str, (usize, usize, f64, usize)>,
    /// The most bytes queued at once, and the queue's back-pressure: how often it held a producer, and how long.
    peak: usize,
    held: (usize, f64),
    /// Cache writes dropped for a full queue (`write_back_unless_full`): how many, and their bytes.
    dropped: (usize, usize),
    /// Each run's gathers (`Gather`), by their type: closed into its wave once full or when it is sealed.
    gathers: HashMap<(u64, std::any::TypeId), Box<dyn Gather>>,
    /// Each run's named sources, as its writes are applied: the manifest's item names them (decision 0306).
    named: HashMap<u64, Vec<super::SourceId>>,
    /// What each run keeps until its writes are applied (`ContentStore::keep_until_written`): its collection lease.
    kept: HashMap<u64, Vec<Box<dyn Any + Send>>>,
}

impl State {
    /// Takes `run`'s gathers (all, or the one of type `kind`) out, their keys queued in `run`'s wave: a read finds
    /// them while they are placed (`Writer::close_gathers`).
    /// A run's record (`until_end`) only where `end`, and after the others: it is the run's last job.
    fn take_gathers(&mut self, run: u64, kind: Option<std::any::TypeId>, end: bool) -> Vec<(Vec<Key>, Box<dyn Gather>)> {
        let mut ids: Vec<(u64, std::any::TypeId)> = self.gathers.iter().filter(|((r, k), g)| *r == run && kind.is_none_or(|t| t == *k) && (end || !g.until_end())).map(|(id, _)| *id).collect();
        ids.sort_by_key(|id| self.gathers[id].until_end());
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let g = self.gathers.remove(&id).expect("a gather");
            let keys = g.keys();
            for (key, held) in &keys {
                let slot = self.keys.entry(*key).or_insert((0, None, run));
                slot.0 += 1;
                slot.1 = held.clone();
                slot.2 = run;
            }
            out.push((keys.into_iter().map(|(k, _)| k).collect(), g));
        }
        out
    }
    /// The run whose gather holds `key`, the value a read gets, and whether it is the run's record.
    fn gathered(&self, key: &Key) -> Option<(u64, Option<Held>, bool)> {
        self.gathers.iter().find(|(_, g)| g.holds(key)).map(|((run, _), g)| (*run, g.held(key), g.until_end()))
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// A queued job, the bytes it holds, the keys it writes, and how it runs.
type Queued = (Job, usize, Vec<Key>, Run, &'static str);

/// How the writer runs a job.
#[derive(Clone, Copy, PartialEq)]
enum Run {
    /// In its wave's transaction.
    InWave,
    /// A wave of its own, after the run's wave before it, in transactions of its own (`write_back_alone`).
    Alone,
}

/// A sealed wave, sent to the writer: its jobs, in their order, whether it is one job run alone, its run (0: none),
/// and whether it is the run's last (its named sources then dropped).
struct Wave {
    jobs: Vec<Queued>,
    alone: bool,
    run: u64,
    finished: bool,
}

/// The bytes under which a job is queued past the cap (back-pressure holds only a job of this size or more).
/// `RERAIL_WRITES` (a measurement aid): seconds since the writer's first use in this process, for its timeline.
fn writes_trace() -> Option<f64> {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_WRITES").is_some());
    static T0: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    (*ON).then(|| T0.elapsed().as_secs_f64())
}

fn small_job() -> usize {
    cap_bytes() / 64
}

struct Lane {
    tx: mpsc::Sender<Wave>,
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
    /// The store's identity (`store_identity`), set by its first handle: a store created again at the path has
    /// another, and a writer of its own (as it has atoms of its own).
    identity: std::sync::OnceLock<i64>,
}

impl Writer {
    /// Seals `run`'s wave still being written: sent to the writer, behind every wave sealed before it. Under the
    /// lane's lock, so two seals of a run send its waves in the order they were taken.
    fn seal(&self, run: u64, how: Seal) {
        let mut lane = self.lane.lock().expect("the writer");
        // The run's gathers first, each placed and made one job of the wave; its record last, at its end.
        let gathers = self.shared.state.lock().expect("the writer's state").take_gathers(run, None, how != Seal::Wave);
        self.close_gathers(run, gathers);
        let finished = how == Seal::Finished;
        let jobs = {
            let mut s = self.shared.state.lock().expect("the writer's state");
            let jobs = s.open.remove(&run).unwrap_or_default();
            // A finished run with nothing to write and nothing kept for it sends nothing.
            if jobs.is_empty() && !(finished && (s.named.contains_key(&run) || s.kept.contains_key(&run))) {
                return;
            }
            s.sealed_bytes += jobs.iter().map(|q| q.1).sum::<usize>();
            s.sealed += 1;
            if let Some(t) = writes_trace() {
                eprintln!("RERAIL_WRITES t={t:.2} seal {} jobs, {:.1} MB; queued {:.1} MB, sealed {:.1} MB", jobs.len(), jobs.iter().map(|q| q.1).sum::<usize>() as f64 / 1048576.0, s.bytes as f64 / 1048576.0, s.sealed_bytes as f64 / 1048576.0);
            }
            jobs
        };
        // A writer with a wave open has its lane: the wave's first job started it (`queue`).
        let wave = Wave { jobs, alone: false, run, finished };
        match lane.as_mut() {
            Some(lane) => {
                if let Err(mpsc::SendError(wave)) = lane.tx.send(wave) {
                    self.failed(wave);
                }
            }
            None => self.failed(wave),
        }
    }

    /// Seals every run's wave still being written (the store's last handle gone).
    fn seal_all(&self) {
        let runs: Vec<u64> = {
            let s = self.shared.state.lock().expect("the writer's state");
            let mut runs: Vec<u64> = s.open.keys().chain(s.gathers.keys().map(|(r, _)| r)).copied().collect();
            runs.sort_unstable();
            runs.dedup();
            runs
        };
        for run in runs {
            self.seal(run, Seal::Finished);
        }
    }

    /// Closes `gathers` (taken by `State::take_gathers`, their keys queued) and queues each in `run`'s wave still being
    /// written, as one job. Called under the lane's lock, so a seal of the run takes them in its wave. A gather that
    /// gathered nothing lets its keys go.
    fn close_gathers(&self, run: u64, gathers: Vec<(Vec<Key>, Box<dyn Gather>)>) {
        for (keys, g) in gathers {
            match g.close() {
                Some(c) => {
                    let job = accounted(c.what, c.bytes, guarded(c.syms, c.job.into_job()));
                    let mut s = self.shared.state.lock().expect("the writer's state");
                    s.bytes += c.bytes;
                    s.jobs += 1;
                    s.peak = s.peak.max(s.bytes);
                    let entry = s.by_what.entry(c.what).or_default();
                    entry.0 += 1;
                    entry.1 += c.bytes;
                    for (key, held) in c.keys {
                        if let Some(slot) = s.keys.get_mut(&key) {
                            slot.1 = held;
                        }
                    }
                    s.open.entry(run).or_default().push((job, c.bytes, keys, Run::InWave, c.what));
                }
                None => {
                    let mut s = self.shared.state.lock().expect("the writer's state");
                    for key in &keys {
                        if let Some(slot) = s.keys.get_mut(key) {
                            slot.0 -= 1;
                            if slot.0 == 0 {
                                s.keys.remove(key);
                            }
                        }
                    }
                }
            }
        }
        self.shared.changed.notify_all();
    }

    /// A wave the writer cannot take (its thread is gone, though it catches each job's panic): failed, each job, as
    /// a write that never happened (the store lacks it, and the next run makes it again).
    fn failed(&self, wave: Wave) {
        for (_, bytes, keys, _, _) in wave.jobs {
            finish(&self.shared, bytes, &keys, Err(StoreError::Invalid("the store's writer is gone".into())), 0.0, true);
        }
        self.shared.state.lock().expect("the writer's state").applied += 1;
        self.shared.changed.notify_all();
    }
}

/// The process's writers, by store path.
fn writers() -> &'static Mutex<HashMap<PathBuf, Arc<Writer>>> {
    static WRITERS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Arc<Writer>>>> = std::sync::OnceLock::new();
    WRITERS.get_or_init(Default::default)
}

/// The writer of the store at `path`, counted as one more of its handles (a `Hold`, which names no store).
pub(super) fn register(path: &Path) -> Arc<Writer> {
    let mut all = writers().lock().expect("the writers");
    let entry = all.entry(path.to_path_buf()).or_default();
    let w = Arc::clone(entry);
    w.handles.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    w
}

/// The writer of the store at `path` whose identity is `identity`, counted as one more of its handles. The path's
/// writer of a store since deleted stays its handles' alone: this store gets a new one.
pub(super) fn register_store(path: &Path, identity: i64) -> Arc<Writer> {
    let mut all = writers().lock().expect("the writers");
    let entry = all.entry(path.to_path_buf()).or_default();
    if *entry.identity.get_or_init(|| identity) != identity {
        *entry = Arc::default();
        entry.identity.get_or_init(|| identity);
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
    // Every run's wave still open is sealed: the store's last handle takes no more writes.
    w.seal_all();
    let lane = w.lane.lock().expect("the writer").take();
    if let Some(Lane { tx, thread }) = lane {
        let (jobs, bytes) = w.shared.state.lock().map(|s| (s.jobs, s.bytes)).unwrap_or_default();
        let t = std::time::Instant::now();
        drop(tx);
        // The writer joins the checkpointer as it ends (`checkpoints`): joined, every connection to the file is
        // closed, as a run that deletes it and creates it again (`corpus::closed`) needs.
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
    Hold(path.to_path_buf(), register(path))
}

/// A kind of job's count, bytes held until written, CPU on the writer's thread and the bytes of the frames it encoded.
type KindWrites = (usize, usize, f64, usize);

/// A kind of job and its `KindWrites`, as the profile lists them.
pub type KindRow = (&'static str, usize, usize, f64, usize);

/// Each kind of job's count, bytes held until written, CPU on the writer's thread and the bytes of the frames it
/// encoded, in this process (the profile's write-back).
static BY_KIND: std::sync::LazyLock<Mutex<std::collections::BTreeMap<&'static str, KindWrites>>> = std::sync::LazyLock::new(Default::default);

/// Each kind of job written so far in this process: count, bytes held, the writer's CPU and the bytes of the frames
/// it encoded, costliest first.
pub fn written_by_kind() -> Vec<(&'static str, usize, usize, f64, usize)> {
    let mut v: Vec<_> = BY_KIND.lock().expect("the writer's account").iter().map(|(k, (n, b, c, e))| (*k, *n, *b, *c, *e)).collect();
    v.sort_by(|a, b| b.3.total_cmp(&a.3));
    v
}

/// The writer's account since it started: jobs, bytes and its thread's CPU (user + sys).
#[derive(Clone, Copy, Debug, Default)]
pub struct Written {
    pub jobs: usize,
    pub bytes: usize,
    pub cpu: f64,
    /// The transactions they were committed in: one a wave.
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
    /// The job joins the wave the run is writing (`WAVES`), and the store takes the wave whole, in one transaction
    /// (`ContentStore::in_one_transaction`), each job a savepoint in it, once a boundary seals it (`end_wave`).
    pub fn write_back(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Work) {
        self.queue(what, bytes, keys, guarded(Syms::Unmapped, job.into_job()), Run::InWave, false);
    }

    /// [`Self::write_back`] of a job whose rows hold `syms` (decision 0170).
    pub fn write_back_syms(&self, syms: Syms, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Work) {
        self.queue(what, bytes, keys, guarded(syms, job.into_job()), Run::InWave, false);
    }

    /// [`Self::write_back_alone`] of a job whose rows hold `syms` (decision 0170).
    pub fn write_back_alone_syms(&self, syms: Syms, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Work) {
        self.queue(what, bytes, keys, guarded(syms, job.into_job()), Run::Alone, false);
    }

    /// [`Self::write_back`] for a job that must commit before it goes on (it starts work on another connection that
    /// reads what it wrote): the run's wave before it is sealed, and it is a wave of its own, run in transactions of
    /// its own.
    pub fn write_back_alone(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Work) {
        self.queue(what, bytes, keys, guarded(Syms::Unmapped, job.into_job()), Run::Alone, false);
    }

    /// [`write_back`](Self::write_back) for a cache's write that may go missing (a tape and its source: the next run
    /// parses the file again): where the queue is full it is dropped, counted (`dropped`), rather than waited for, so
    /// no wave waits for the writer (decision 0104). Whether it was queued.
    /// Its rows hold `syms` (decision 0170).
    pub fn write_back_unless_full(&self, syms: Syms, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Work) -> bool {
        self.queue(what, bytes, keys, guarded(syms, job.into_job()), Run::InWave, true)
    }

    /// The cache writes dropped for a full queue since the writer started: how many, and their bytes.
    pub fn dropped(&self) -> (usize, usize) {
        self.writer().map_or((0, 0), |w| w.shared.state.lock().expect("the writer's state").dropped)
    }

    /// Adds a write to this thread's run's gather of type `G` (`make` makes it, the run's first): closed and queued
    /// in the run's wave once it holds [`gather_cap`] bytes, or when the wave is sealed. False, nothing added, outside
    /// a run's waves, with no writer, or with no threads (the browser build): the caller queues the write itself.
    pub fn gather<G: Gather>(&self, what: &'static str, make: impl FnOnce() -> G, add: impl FnOnce(&mut G)) -> bool {
        let run = WAVES.with(std::cell::Cell::get);
        let Some(w) = self.writer().filter(|_| run != 0 && !cfg!(target_family = "wasm")) else { return false };
        if !super::hazards::warm_write_allowed(what) {
            super::hazards::warm_fault("a write gathered for the store", || what.to_owned());
        }
        // The writer's lane, started: the gather is closed into a wave sealed from any thread.
        w.lane.lock().expect("the writer").get_or_insert_with(|| self.start_writer(w));
        let kind = std::any::TypeId::of::<G>();
        let full = {
            let mut s = w.shared.state.lock().expect("the writer's state");
            let g = s.gathers.entry((run, kind)).or_insert_with(|| Box::new(make()));
            add(g.as_any_mut().downcast_mut::<G>().expect("a gather of its type"));
            if g.bytes() >= gather_cap() { s.take_gathers(run, Some(kind), true) } else { Vec::new() }
        };
        if !full.is_empty() {
            let _lane = w.lane.lock().expect("the writer");
            w.close_gathers(run, full);
        }
        true
    }

    fn queue(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Job, run: Run, droppable: bool) -> bool {
        // A truly warm run computes nothing it would keep (`RERAIL_WARM`): its caller's stack is here, not on the writer.
        // Not a fault: the writes listed as allowed (locks, timestamps).
        if !super::hazards::warm_write_allowed(what) {
            super::hazards::warm_fault("a write queued for the store", || format!("{what}, {bytes} bytes, {} keys", keys.len()));
        }
        let mut job = job;
        let Some(w) = self.writer() else {
            let _scope = super::hazards::warm_scope();
            super::hazards::warm_job(what);
            if let Err(e) = retried(self, what, &mut job) {
                eprintln!("rerail: the write failed: {e}");
            }
            return true;
        };
        let mut job = accounted(what, bytes, job);
        let wave = WAVES.with(std::cell::Cell::get);
        {
            let mut s = w.shared.state.lock().expect("the writer's state");
            // Back-pressure: past the cap, wait for the writer while it has sealed waves to take (never for waves
            // still being written, which only a boundary seals). A small job never waits: its wait frees next to
            // nothing, and holds the wave behind a large job the writer is writing.
            let full = |s: &State| bytes >= small_job() && s.sealed_bytes > 0 && s.bytes + bytes > cap_bytes();
            // A cache's write gives way at half the cap: the other half is for the writes that may not go missing
            // (frames, memo entries), so they find room rather than wait behind tapes.
            if droppable && s.jobs > 0 && s.bytes + bytes > cap_bytes() / 2 {
                s.dropped = (s.dropped.0 + 1, s.dropped.1 + bytes);
                return false;
            }
            if bytes >= small_job() && let Some(t) = writes_trace() {
                eprintln!("RERAIL_WRITES t={t:.2} queue {what} {:.1} MB; queued {:.1} MB, sealed {:.1} MB", bytes as f64 / 1048576.0, s.bytes as f64 / 1048576.0, s.sealed_bytes as f64 / 1048576.0);
            }
            let t = full(&s).then(std::time::Instant::now);
            // The first hold says what fills the queue (a strict run ends at it).
            static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if t.is_some() && !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let mut rows: Vec<_> = s.by_what.iter().map(|(w, (j, b, c, _))| (*w, *j, *b, *c)).collect();
                rows.sort_by_key(|r| std::cmp::Reverse(r.2));
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
                let slot = s.keys.entry(*key).or_insert((0, None, wave));
                slot.0 += 1;
                slot.1 = held.clone();
                slot.2 = wave;
            }
        }
        let keys: Vec<Key> = keys.into_iter().map(|(k, _)| k).collect();
        // No threads in the browser build: written now, in order.
        if cfg!(target_family = "wasm") {
            let _scope = super::hazards::warm_scope();
            w.shared.state.lock().expect("the writer's state").sealed_bytes += bytes;
            let outcome = job(self);
            finish(&w.shared, bytes, &keys, outcome, 0.0, true);
            return true;
        }
        let queued: Queued = (job, bytes, keys, run, what);
        let mut lane = w.lane.lock().expect("the writer");
        let lane = lane.get_or_insert_with(|| self.start_writer(w));
        // In a run, a job joins the run's wave still being written; a job run alone follows it, a wave of its own.
        // Outside a run, a job is a wave of its own.
        let wave = match (wave, run) {
            (0, _) | (_, Run::Alone) => {
                let mut s = w.shared.state.lock().expect("the writer's state");
                let before = (wave != 0).then(|| s.open.remove(&wave)).flatten().filter(|j| !j.is_empty());
                let mut waves = Vec::new();
                if let Some(jobs) = before {
                    s.sealed_bytes += jobs.iter().map(|q| q.1).sum::<usize>();
                    s.sealed += 1;
                    waves.push(Wave { jobs, alone: false, run: wave, finished: false });
                }
                s.sealed_bytes += bytes;
                s.sealed += 1;
                waves.push(Wave { jobs: vec![queued], alone: run == Run::Alone, run: wave, finished: false });
                waves
            }
            (wave, Run::InWave) => {
                w.shared.state.lock().expect("the writer's state").open.entry(wave).or_default().push(queued);
                return true;
            }
        };
        for wave in wave {
            if let Err(mpsc::SendError(wave)) = lane.tx.send(wave) {
                // The writer is gone (it panicked): written here, in order, as the writer would have.
                let _scope = super::hazards::warm_scope();
                for (mut job, bytes, keys, _, _) in wave.jobs {
                    let outcome = job(self);
                    finish(&w.shared, bytes, &keys, outcome, 0.0, true);
                }
                w.shared.state.lock().expect("the writer's state").applied += 1;
                w.shared.changed.notify_all();
            }
        }
        true
    }

    fn start_writer(&self, w: &Arc<Writer>) -> Lane {
        let (tx, rx) = mpsc::channel::<Wave>();
        let w = Arc::clone(w);
        let store = self.sibling();
        // The WAL is checkpointed beside the writer, not in its commits: SQLite's automatic checkpoint copies the
        // WAL into the database and syncs it inside the commit that crosses its threshold, so the writer took no
        // job meanwhile (3.0 s of a cold run's 7.7 s on the writer). A checkpoint is `PASSIVE` on a connection of
        // its own: in WAL mode it neither blocks the writer nor waits for it, and it copies only what is committed:
        // whole waves. Told after each wave; signals that come while it checkpoints are one.
        //
        // Decision 0308: only once the WAL holds `CHECKPOINT_WAL_BYTES`, and once more as the writer ends. A
        // checkpoint after every wave copied and synced again each page that consecutive waves change (the indexes'),
        // 10.8 thread-s of a cold PostHog run's 24 s in the store's I/O.
        let (mut told, tells) = mpsc::channel::<()>();
        let checkpointer = self.sibling();
        let wal = { let mut p = self.database_path.clone().into_os_string(); p.push("-wal"); PathBuf::from(p) };
        let checkpoints = rerail_accel::exec::service("rerail-store-checkpoint", None, move || {
            super::hazards::behind_the_run();
            let checkpoint = || {
                let Ok(connection) = checkpointer.connection.lock() else { return false };
                if let Err(e) = connection.query_row_cached("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(())) {
                    eprintln!("rerail: the store's checkpoint failed: {e}");
                }
                true
            };
            while tells.recv().is_ok() {
                while tells.try_recv().is_ok() {}
                if std::fs::metadata(&wal).is_ok_and(|m| m.len() < CHECKPOINT_WAL_BYTES) {
                    continue;
                }
                if !checkpoint() {
                    return;
                }
            }
            // The writer has ended: what it committed since the last checkpoint, so a later run reads it through the
            // map of the database file, not from the WAL.
            checkpoint();
        })
        .expect("the store's checkpoint thread");
        // Joined by the writer as it ends (below): a checkpoint after the store's last handle is gone touched its
        // files after its owner removed them.
        let mut checkpoints = Some(checkpoints);
        let thread = rerail_accel::exec::service("rerail-store-writer", None, move || {
                // What this thread sends to SQLite no wave waits for.
                super::hazards::behind_the_run();
                // Its commits leave the WAL to the checkpointer (above). Its page cache holds a wave's changed pages
                // (decision 0308): at the readers' 1 MB, a wave spilled them into the WAL before its commit
                // (`pagerStress`) and read them back from it as it changed them again.
                if let Ok(connection) = store.connection.lock() {
                    let _ = connection.pragma_update(None, "wal_autocheckpoint", 0);
                    let _ = connection.pragma_update(None, "cache_size", -WRITER_CACHE_KIB);
                }
                while let Ok(Wave { jobs: mut batch, alone, run: of, finished }) = rx.recv() {
                    let before = thread_cpu();
                    // The run whose writes these are: their named sources are its (`ContentStore::name_sources`).
                    *store.applying.lock() = (of, Some(Arc::clone(&w)));
                    if batch.is_empty() {
                        let mut s = w.shared.state.lock().expect("the writer's state");
                        s.named.remove(&of);
                        let kept = s.kept.remove(&of);
                        s.applied += 1;
                        drop(s);
                        drop(kept);
                        w.shared.changed.notify_all();
                        continue;
                    }
                    if alone {
                        for (job, bytes, keys, _, what) in batch {
                            // Each job's own CPU and frames: a wave of several jobs alone counts each once.
                            let (before, encoded) = (thread_cpu(), super::compression::encoded_here());
                            let mut job = job;
                            let _span = super::hazards::span(what);
                            let _scope = super::hazards::warm_scope();
                            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&store)))
                                .unwrap_or_else(|_| Err(StoreError::Invalid("a queued write panicked".into())));
                            let mut s = w.shared.state.lock().expect("the writer's state");
                            let e = s.by_what.entry(what).or_default();
                            e.2 += thread_cpu() - before;
                            e.3 += super::compression::encoded_here() - encoded;
                            drop(s);
                            let _ = told.send(());
                            finish(&w.shared, bytes, &keys, outcome, thread_cpu() - before, true);
                        }
                        w.shared.state.lock().expect("the writer's state").applied += 1;
                        w.shared.changed.notify_all();
                        continue;
                    }
                    // The wave's jobs, in their order, in one transaction: the store holds all of the wave or none.
                    let run = |batch: &mut Vec<Queued>| -> Vec<StoreResult<()>> {
                        let mut outcomes = Vec::with_capacity(batch.len());
                        for (job, _, _, _, what) in batch.iter_mut() {
                            let (c0, e0) = (thread_cpu(), super::compression::encoded_here());
                            let _span = super::hazards::span(what);
                            outcomes.push(
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&store)))
                                    .unwrap_or_else(|_| Err(StoreError::Invalid("a queued write panicked".into()))),
                            );
                            let mut s = w.shared.state.lock().expect("the writer's state");
                            let e = s.by_what.entry(*what).or_default();
                            e.2 += thread_cpu() - c0;
                            e.3 += super::compression::encoded_here() - e0;
                            drop(s);
                        }
                        outcomes
                    };
                    // The wave's span holds its jobs' and its commit.
                    let batch_span = super::hazards::span("a wave, in one transaction");
                    // Every job of the wave, each run of it, in one warm scope (decision 0211): its commit is a listed
                    // write's only if every job is listed (`profile::WARM_WRITES_ALLOWED`).
                    let warm_scope = super::hazards::warm_scope();
                    // A wave that finds the store busy (another process's writer held SQLite's write lock past this
                    // one's wait, at its start or its commit) rolled back whole: run again, as a busy job is
                    // (`run_job`), its jobs being a cache's writes. Failing them dropped every write of the wave.
                    let started = Instant::now();
                    let mut backoff = JOB_BACKOFF_FIRST;
                    let mut forked_in = false;
                    let mut tries = 0usize;
                    let jobs_wall = std::cell::Cell::new(0.0f64);
                    let outcomes = loop {
                        tries += 1;
                        match store.in_one_transaction(|| { let t = Instant::now(); let o = run(&mut batch); jobs_wall.set(jobs_wall.get() + t.elapsed().as_secs_f64()); o }) {
                            Ok(outcomes) => break outcomes,
                            Err(StoreError::Busy) if started.elapsed() < JOB_DEADLINE => {
                                std::thread::sleep(super::connection::jittered(backoff));
                                backoff = (backoff * 2).min(JOB_BACKOFF_MAX);
                            }
                            // Forked by one of its jobs (decision 0170), so rolled back: run again, forked from its
                            // start, its jobs' rows mapped or the jobs dropped (`guarded`).
                            Err(StoreError::Invalid(e)) if !forked_in && e.contains(super::atoms::DIVERGED_TEXT) => forked_in = true,
                            Err(e) => break batch.iter().map(|_| Err(StoreError::Invalid(format!("the wave's commit: {e}")))).collect(),
                        }
                    };
                    drop(warm_scope);
                    drop(batch_span);
                    if let Some(t) = writes_trace() {
                        let wall = started.elapsed().as_secs_f64();
                        eprintln!("RERAIL_WRITES t={t:.2} wave {} jobs, {:.1} MB: {wall:.3} s wall, jobs {:.3} s, begin+commit {:.3} s, CPU {:.3} s, {tries} tries", batch.len(), batch.iter().map(|q| q.1).sum::<usize>() as f64 / 1048576.0, jobs_wall.get(), wall - jobs_wall.get(), thread_cpu() - before);
                    }
                    let _ = told.send(());
                    let cpu = (thread_cpu() - before) / batch.len().max(1) as f64;
                    w.shared.state.lock().expect("the writer's state").batches += 1;
                    for ((_, bytes, keys, _, _), outcome) in batch.into_iter().zip(outcomes) {
                        finish(&w.shared, bytes, &keys, outcome, cpu, true);
                    }
                    let mut s = w.shared.state.lock().expect("the writer's state");
                    let kept = if finished {
                        s.named.remove(&of);
                        s.kept.remove(&of)
                    } else {
                        None
                    };
                    s.applied += 1;
                    drop(s);
                    // A lease let go once its run's writes are in the store (outside the state's lock).
                    drop(kept);
                    w.shared.changed.notify_all();
                }
                // The queue is closed (the last handle's drop): the checkpointer ends with the writer.
                drop(std::mem::replace(&mut told, mpsc::channel().0));
                if let Some(c) = checkpoints.take() {
                    let _ = c.join();
                }
            })
            .expect("the store's writer thread");
        Lane { tx, thread }
    }

    /// Adds `ids` to the named sources of the run whose writes the writer is applying (decision 0306): its manifest's
    /// item names them. Outside the writer, or of no run, nothing.
    pub fn name_sources(&self, ids: impl IntoIterator<Item = super::SourceId>) {
        let (run, w) = self.applying.lock().clone();
        let Some(w) = w.filter(|_| run != 0) else { return };
        w.shared.state.lock().expect("the writer's state").named.entry(run).or_default().extend(ids);
    }

    /// The named sources of the run whose writes the writer is applying, taken.
    pub fn take_named_sources(&self) -> Vec<super::SourceId> {
        let (run, w) = self.applying.lock().clone();
        let Some(w) = w.filter(|_| run != 0) else { return Vec::new() };
        w.shared.state.lock().expect("the writer's state").named.remove(&run).unwrap_or_default()
    }

    /// Keeps `thing` (a run's collection lease) until every write of this thread's run is applied (its last wave):
    /// the manifest, which names what the lease protected, is in the store by then. Outside a run, or with no writer,
    /// let go now (what was queued is written already).
    pub fn keep_until_written(&self, thing: impl Any + Send) {
        let run = WAVES.with(std::cell::Cell::get);
        let Some(w) = self.writer().filter(|_| run != 0 && !cfg!(target_family = "wasm")) else { return };
        w.shared.state.lock().expect("the writer's state").kept.entry(run).or_default().push(Box::new(thing));
    }

    /// The value a queued job left for `key`, while the job is queued.
    pub fn pending<T: Any + Send + Sync>(&self, key: &Key) -> Option<Arc<T>> {
        let s = self.writer()?.shared.state.lock().expect("the writer's state");
        match s.keys.get(key) {
            Some((_, held, _)) => held.clone(),
            None => s.gathered(key).and_then(|(_, h, _)| h),
        }
        .and_then(|h| h.downcast::<T>().ok())
    }

    /// Whether a job that writes `key` is queued (not written yet).
    pub fn queued(&self, key: &Key) -> bool {
        self.writer().is_some_and(|w| {
            let s = w.shared.state.lock().expect("the writer's state");
            s.keys.contains_key(key) || s.gathered(key).is_some()
        })
    }

    /// Waits for the jobs that write `key`, if any is queued; a read of anything else never waits. A key in a wave
    /// still being written seals that wave: the read is of a value the run left no copy of in memory.
    pub fn settle(&self, key: &Key) {
        let Some(w) = self.writer() else { return };
        let open = {
            let s = w.shared.state.lock().expect("the writer's state");
            match s.keys.get(key) {
                Some(k) => Some((k.2, Seal::Wave)),
                None => s.gathered(key).map(|(run, _, record)| (run, if record { Seal::End } else { Seal::Wave })),
            }
            .filter(|&(run, _)| run != 0)
        };
        if let Some((run, how)) = open {
            w.seal(run, how);
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

    /// Seals this thread's run's wave and waits for every wave sealed so far: the store then holds every write this
    /// run queued, and every wave sealed before. The first error a job met since the last call.
    pub fn written(&self) -> StoreResult<()> {
        let Some(w) = self.writer() else { return Ok(()) };
        let run = WAVES.with(std::cell::Cell::get);
        if run != 0 {
            w.seal(run, Seal::End);
        }
        let mut s = w.shared.state.lock().expect("the writer's state");
        let target = s.sealed;
        let t = (s.applied < target).then(std::time::Instant::now);
        while s.applied < target {
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

    /// What the writer has done since it started.
    pub fn written_so_far(&self) -> Written {
        let Some(w) = self.writer() else { return Written::default() };
        let s = w.shared.state.lock().expect("the writer's state");
        Written { jobs: s.done.0, bytes: s.done.1, cpu: s.done.2, batches: s.batches }
    }

    /// The writer's work by what each job writes, `(what, jobs, bytes held, writer CPU, bytes of frames encoded)`,
    /// costliest first; the most bytes queued at once; and the back-pressure's holds of a producer, `(count, seconds)`.
    pub fn written_by_what(&self) -> (Vec<KindRow>, usize, (usize, f64)) {
        let Some(w) = self.writer() else { return (Vec::new(), 0, (0, 0.0)) };
        let s = w.shared.state.lock().expect("the writer's state");
        let mut rows: Vec<_> = s.by_what.iter().map(|(w, (j, b, c, e))| (*w, *j, *b, *c, *e)).collect();
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

/// `job` as its rows' `syms` allow after a fork (decision 0170): an unmapped one is not run, a mapped one runs as
/// a write whose symbols are the store's (`atoms::mapped`).
/// `job` run as the writer runs a queued one: again while it finds the store busy (`retried`), and counted by what it
/// writes.
fn accounted(what: &'static str, bytes: usize, mut job: Job) -> Job {
    Box::new(move |store| {
        // In the writer's transaction (`warm_scope`): whether its commit is one a warm run may make. A run's record is
        // judged by its items' kinds, as it applies them (`record::apply`).
        if what != super::record::WHAT {
            super::hazards::warm_job(what);
        }
        let (t0, e0) = (thread_cpu(), super::compression::encoded_here());
        let out = retried(store, what, &mut job);
        let mut by = BY_KIND.lock().expect("the writer's account");
        let e = by.entry(what).or_default();
        e.0 += 1;
        e.1 += bytes;
        e.2 += thread_cpu() - t0;
        e.3 += super::compression::encoded_here() - e0;
        out
    })
}

fn guarded(syms: Syms, mut job: Job) -> Job {
    Box::new(move |store| {
        if syms == Syms::Unmapped && store.atoms.forked() {
            return Err(StoreError::Invalid(super::atoms::DIVERGED_TEXT.to_owned()));
        }
        if syms == Syms::Unmapped { job(store) } else { super::atoms::mapped(|| job(store)) }
    })
}

fn finish(shared: &Shared, bytes: usize, keys: &[Key], outcome: StoreResult<()>, cpu: f64, counted: bool) {
    let mut s = shared.state.lock().expect("the writer's state");
    s.bytes -= bytes;
    s.sealed_bytes -= bytes;
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
    // A process whose atoms diverged from the store's (decision 0160) drops its writes: the store lacks them, as a
    // cache that missed them.
    if let Err(e) = outcome
        && s.error.is_none()
        && !e.to_string().contains(super::atoms::DIVERGED_TEXT)
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
    /// A test's job, queued as the queue's tests queue them (`Work::test`).
    fn job(job: impl FnMut(&ContentStore) -> StoreResult<()> + Send + 'static) -> Work {
        Work::test(Box::new(job))
    }
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

    /// A store deleted and created again at its path while a handle of the old one lives has a writer of its own: its
    /// jobs run on a handle of the new store (its atoms), not queued behind the old store's. PR #508's d9c403ebf (t47).
    #[test]
    fn a_store_created_again_at_its_path_has_its_own_writer() {
        use std::sync::{Arc, Mutex, mpsc};
        let (dir, old) = store("again");
        let path = old.database_path.clone();
        // The old store's writer holds its connection, on the old store's file, before the file goes: one it opened
        // after would be refused (the store at the path is another).
        old.write_back("open", 0, vec![], job(|_| Ok(())));
        old.written().unwrap();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let order = Arc::new(Mutex::new(Vec::new()));
        let o = Arc::clone(&order);
        old.write_back("old", 0, vec![], job(move |_| {
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
        new.write_back("new", 0, vec![], job(move |s| {
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

    /// Jobs run in their order on the writer; a read of a queued key gets the job's value, or waits for it alone;
    /// `written` waits for all and gives the first error; another handle of the store shares the queue.
    #[test]
    fn queued_writes_run_in_order_and_reads_see_them() {
        let (dir, store) = store("order");
        let order = Arc::new(Mutex::new(Vec::new()));
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let key = Key::of(&[b"t", b"1"]);
        let (o1, o2) = (Arc::clone(&order), Arc::clone(&order));
        store.write_back("one", 8, vec![(key, Some(Arc::new(7u64) as Held))], job(move |_| {
            go_rx.recv().unwrap();
            o1.lock().unwrap().push(1);
            Ok(())
        }));
        store.write_back("two", 8, vec![], job(move |_| {
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
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A cache's write that would wait for a full queue is dropped instead (`write_back_unless_full`): it returns at
    /// once, is never run, and is counted; with room, it is queued as any write. A wave never waits for the writer.
    #[test]
    fn a_cache_write_to_a_full_queue_is_dropped_not_waited_for() {
        let (dir, store) = store("drop");
        let (open, gate) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        store.write_back("slow", super::cap_bytes(), vec![], job(move |_| {
            let _ = gate.lock().unwrap().recv();
            Ok(())
        }));
        let ran = Arc::new(Mutex::new(0));
        let r = Arc::clone(&ran);
        let t = std::time::Instant::now();
        let queued = store.write_back_unless_full(Syms::None, "cache", small_job(), vec![], job(move |_| {
            *r.lock().unwrap() += 1;
            Ok(())
        }));
        assert!(!queued && t.elapsed() < std::time::Duration::from_millis(100), "dropped at once");
        open.send(()).unwrap();
        store.written().unwrap();
        assert_eq!(*ran.lock().unwrap(), 0, "a dropped write never runs");
        assert_eq!(store.dropped(), (1, small_job()));
        let r = Arc::clone(&ran);
        assert!(store.write_back_unless_full(Syms::None, "cache", small_job(), vec![], job(move |_| {
            *r.lock().unwrap() += 1;
            Ok(())
        })));
        store.written().unwrap();
        assert_eq!(*ran.lock().unwrap(), 1);
        drop(store);
        // The store's checkpoint may still be closing its files: a cleanup, not a check.
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A wave rolled back for a fork (decision 0170) runs again; the waves queued while it first ran are each kept.
    /// (A batch that took the next job while it ran lost one when run again: the queue's count then never reached
    /// zero, and `written` waited for ever, the hung warm runs. A wave takes nothing while it runs.)
    #[test]
    fn a_batch_run_again_after_a_fork_keeps_every_wait_queued_meanwhile() {
        let (dir, store) = store("fork-waits");
        store.intern_atoms(&["a"]).unwrap();
        store.flush_atoms().unwrap();
        // The job forks the batch midway, as another thread of the process does (it finds another process's atoms
        // past this one's pending ones): its atoms are pending, and the store ends past them.
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let r = Arc::clone(&runs);
        store.write_back_syms(Syms::None, "forks", 0, vec![], job(move |s| {
            if r.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                go_rx.recv().unwrap();
                s.intern_atoms(&["b"])?;
                let connection = s.connection.lock().map_err(StoreError::Invalid)?;
                crate::content_store::atoms::stored::append_as_another(&connection, &["z".to_owned()])?;
            }
            s.flush_atoms()
        }));
        let later = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..2 {
            let l = Arc::clone(&later);
            store.write_back_syms(Syms::None, "later", 0, vec![], job(move |_| { l.fetch_add(1, std::sync::atomic::Ordering::SeqCst); Ok(()) }));
        }
        go_tx.send(()).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let other = ContentStore::open(&store.database_path).unwrap();
        // The other handle is dropped (its checkpoint written) before the thread ends: joined before the directory
        // goes, or its checkpoint writes into a directory being removed.
        let waiter = std::thread::spawn(move || done_tx.send(other.written().is_ok()).unwrap());
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(10)), Ok(true), "every queued job written");
        waiter.join().unwrap();
        assert!(store.atoms_handle().forked(), "the job forked the batch");
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 2, "rolled back and run again");
        assert_eq!(later.load(std::sync::atomic::Ordering::SeqCst), 2, "each wave queued meanwhile written once");
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A run's writes wait for its boundary, then go in one transaction (decision 0259): before it the store holds
    /// none of the wave, after it all. A boundary that does not seal (a cone round) leaves the wave open; another
    /// run's boundary seals only its own wave; the run's end seals its last.
    #[test]
    fn a_runs_wave_is_written_whole_at_its_boundary() {
        let (dir, store) = store("waves");
        let other = ContentStore::open(&store.database_path).unwrap();
        let kept = |s: &ContentStore, i: u8| s.stat_blob([i; 32]).unwrap().is_some();
        let put = |s: &ContentStore, i: u8| s.write_back("blob", 0, vec![], job(move |s| s.put_stat_blob([i; 32], b"kept")));
        let waves = run_waves();
        for i in 0..3 {
            put(&store, i);
        }
        // Another run on another thread seals its own wave, not this one's.
        std::thread::scope(|t| {
            t.spawn(|| {
                let _theirs = run_waves();
                put(&other, 9);
                end_wave(Boundary::Chunk);
                other.written().unwrap();
            });
        });
        assert!(kept(&other, 9), "the other run's wave is written");
        assert!(!(0..3).any(|i| kept(&other, i)), "this run's wave is not, before its boundary");
        let before = store.written_so_far().batches;
        end_wave(Boundary::Chunk);
        store.written().unwrap();
        assert!((0..3).all(|i| kept(&other, i)), "the wave whole");
        assert_eq!(store.written_so_far().batches, before + 1, "in one transaction");
        // A cone round does not seal: the write waits for the run's end.
        put(&store, 5);
        end_wave(Boundary::ConeRound);
        std::thread::sleep(Duration::from_millis(50));
        assert!(!kept(&other, 5));
        drop(waves);
        store.written().unwrap();
        assert!(kept(&other, 5), "the run's end seals its last wave");
        drop((store, other));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The last handle's drop writes the queue; a `Hold` keeps the writer past a handle's drop.
    #[test]
    fn the_last_handle_writes_the_queue() {
        let (dir, store) = store("last");
        let path = store.database_path.clone();
        let hold = hold(&path);
        let done = Arc::new(Mutex::new(false));
        let d = Arc::clone(&done);
        store.write_back("blob", 0, vec![], job(move |s| {
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
        let _ = std::fs::remove_dir_all(dir);
    }

}
