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
//! - **Bounded lag.** Jobs carry their size; past `CAP_BYTES` queued, the next job waits for the
//!   writer to catch up (back-pressure, 0024 §5), so memory holds at most that much unwritten.
//! - **Durable at the end.** `written` waits for the queue and gives the first error a job met; a
//!   handle's drop waits too, so no queued write is lost when the process ends.
//!
//! A job that fails leaves the store as a cache that lacks that write: the next run misses and
//! recomputes, as when the write never happened. Its error is reported at `written`.
use super::{ContentStore, StoreError, StoreResult};
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};

/// The most bytes queued and unwritten: within the 1 GB budget, a few chunks' memo entries.
const CAP_BYTES: usize = 192 << 20;

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

/// A queued write: run on the writer's handle, its own transactions.
pub type Job = Box<dyn FnOnce(&ContentStore) -> StoreResult<()> + Send>;

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
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// A queued job, the bytes it holds, the keys it writes and the holder it pins its sources under.
type Queued = (Job, usize, Vec<Key>, Option<(String, i64)>);

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
}

/// The process's writers, by store path.
fn writers() -> &'static Mutex<HashMap<PathBuf, Arc<Writer>>> {
    static WRITERS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Arc<Writer>>>> = std::sync::OnceLock::new();
    WRITERS.get_or_init(Default::default)
}

/// The writer of the store at `path`, counted as one more of its handles.
pub(super) fn register(path: &Path) -> Arc<Writer> {
    let mut all = writers().lock().expect("the writers");
    let w = Arc::clone(all.entry(path.to_path_buf()).or_default());
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
        drop(tx);
        let _ = thread.join();
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

/// `Hold` of the store at `path`.
pub fn hold(path: &Path) -> Hold {
    Hold(path.to_path_buf(), register(path))
}

/// The writer's account since it started: jobs, bytes and its thread's CPU (user + sys).
#[derive(Clone, Copy, Debug, Default)]
pub struct Written {
    pub jobs: usize,
    pub bytes: usize,
    pub cpu: f64,
}

impl ContentStore {
    fn writer(&self) -> Option<&Arc<Writer>> {
        self.write_back.as_ref()
    }

    /// Queues `job` behind every write queued before it, and returns without waiting for SQLite.
    /// `bytes` is what it holds until written; `keys` are what it writes, each with the value a
    /// read of it gets meanwhile (`None`: a read waits for the job). The writer's own handle (in a
    /// job) writes at once.
    pub fn write_back(&self, what: &'static str, bytes: usize, keys: Vec<(Key, Option<Held>)>, job: Job) {
        let Some(w) = self.writer() else {
            if let Err(e) = job(self) {
                eprintln!("rerail: {what}: the write failed: {e}");
            }
            return;
        };
        let job: Job = Box::new(move |store| job(store).map_err(|e| StoreError::Invalid(format!("{what}: {e}"))));
        {
            let mut s = w.shared.state.lock().expect("the writer's state");
            // Back-pressure: past the cap, wait for the writer (never for an empty queue).
            let t = (s.jobs > 0 && s.bytes + bytes > CAP_BYTES).then(std::time::Instant::now);
            while s.jobs > 0 && s.bytes + bytes > CAP_BYTES {
                s = w.shared.changed.wait(s).expect("the writer's state");
            }
            if let Some(t) = t {
                super::hazards::wait_in_wave("the queue's back-pressure", t.elapsed().as_secs_f64());
            }
            s.bytes += bytes;
            s.jobs += 1;
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
            finish(&w.shared, bytes, &keys, outcome, 0.0);
            return;
        }
        let mut lane = w.lane.lock().expect("the writer");
        let lane = lane.get_or_insert_with(|| self.start_writer(w));
        if let Err(mpsc::SendError((job, bytes, keys, _))) = lane.tx.send((job, bytes, keys, holder)) {
            // The writer is gone (it panicked): written here, in order, as the writer would have.
            let outcome = job(self);
            finish(&w.shared, bytes, &keys, outcome, 0.0);
        }
    }

    fn start_writer(&self, w: &Arc<Writer>) -> Lane {
        let (tx, rx) = mpsc::channel::<Queued>();
        let w = Arc::clone(w);
        let store = self.sibling();
        let thread = std::thread::Builder::new()
            .name("rerail-store-writer".into())
            .spawn(move || {
                // What this thread sends to SQLite no wave waits for.
                super::hazards::behind_the_run();
                for (job, bytes, keys, holder) in rx {
                    *store.holder.lock() = holder;
                    let before = thread_cpu();
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&store)))
                        .unwrap_or_else(|_| Err(StoreError::Invalid("a queued write panicked".into())));
                    finish(&w.shared, bytes, &keys, outcome, thread_cpu() - before);
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

    /// What the writer has done since it started.
    pub fn written_so_far(&self) -> Written {
        let Some(w) = self.writer() else { return Written::default() };
        let s = w.shared.state.lock().expect("the writer's state");
        Written { jobs: s.done.0, bytes: s.done.1, cpu: s.done.2 }
    }

    /// This handle gone (its drop): the last of the store's ends the writer, its queue written.
    pub(super) fn end_writer(&mut self) {
        if let Some(w) = self.write_back.take() {
            unregister(&self.database_path, &w);
        }
    }
}

fn finish(shared: &Shared, bytes: usize, keys: &[Key], outcome: StoreResult<()>, cpu: f64) {
    let mut s = shared.state.lock().expect("the writer's state");
    s.bytes -= bytes;
    s.jobs -= 1;
    s.done = (s.done.0 + 1, s.done.1 + bytes, s.done.2 + cpu);
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
}
