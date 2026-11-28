//! MLDB's thread pool, ported whole (`mldb/base/thread_pool.{h,cc}`, `thread_pool_impl.h`, `parallel.cc`): to be
//! measured against rayon's pool (`cores_map`) on the same work, the CPU device's launches.
//!
//! - Each thread that adds work or works for a pool has a queue of its own in it (`ThreadQueue`: a Chase–Lev deque of
//!   4,096 slots; a push past it runs the job at once). The pool's list of queues is copied on write, under a lock,
//!   with an epoch; each thread keeps its copy and refreshes it when the epoch moves, if it can take the lock.
//! - A thread looks for work in its own queue first (newest first), then steals (oldest first) from the others',
//!   starting at the queue it last found work in.
//! - A root pool has threads of its own. A worker with nothing to do yields and sleeps 1 ms, nine times, then waits
//!   10 ms on the pool's condition variable; with no job outstanding in the pool it waits 250 ms. A job added wakes
//!   one sleeper, where any sleeps. A wakeup may be missed: the waits are bounded for it.
//! - A subordinate pool has no threads: each job added to it posts a job to its parent that works for it until it has
//!   no work, at most `max_parent_jobs` of them at once.
//! - `wait_for_all` works for the pool (its own queue, then stealing) until every job added has finished, spinning.
//! - `parallel_map_chunked`: a subordinate pool of the root, `limit - 1` workers added to it that claim chunks from
//!   one counter, the caller one more, then `wait_for_all`.
//!
//! A job runs in its maker's run (`carried`, as rayon's jobs do through its patch). A panic in a job is kept, the
//! first one, and resumed from `wait_for_all` (MLDB's `handleExceptions`).
//!
//! The root pool is the machine's (`machine::<Root>()`, decision 0442: no static), of `threads()` threads.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

type Job = Box<dyn FnOnce() + Send>;
type Panic = Box<dyn std::any::Any + Send>;

const NUMBER_OF_ITEMS: u32 = 4096;
const MASK: u32 = NUMBER_OF_ITEMS - 1;

/// A thread's queue in a pool (`ThreadQueue`): its owner pushes and pops at the bottom, others steal at the top.
struct ThreadQueue {
    bottom: AtomicU32,
    top: AtomicU32,
    items: Box<[AtomicPtr<Job>]>,
    num_queued: AtomicU32,
}

impl ThreadQueue {
    fn new() -> ThreadQueue {
        ThreadQueue {
            bottom: AtomicU32::new(0),
            top: AtomicU32::new(0),
            items: (0..NUMBER_OF_ITEMS).map(|_| AtomicPtr::new(std::ptr::null_mut())).collect(),
            num_queued: AtomicU32::new(0),
        }
    }

    /// Pushes `item`, or gives it back where the queue is full.
    fn push(&self, item: *mut Job) -> Option<*mut Job> {
        if self.num_queued.load(Ordering::SeqCst) == NUMBER_OF_ITEMS {
            return Some(item);
        }
        let b = self.bottom.load(Ordering::Relaxed);
        self.items[(b & MASK) as usize].store(item, Ordering::SeqCst);
        // The item written before b + 1 is published to other threads.
        self.bottom.store(b.wrapping_add(1), Ordering::Release);
        self.num_queued.fetch_add(1, Ordering::SeqCst);
        None
    }

    fn steal(&self) -> Option<*mut Job> {
        let t = self.top.load(Ordering::Acquire);
        // top read before bottom.
        let b = self.bottom.load(Ordering::Acquire);
        if t == b || !top_less_than_bottom(t, b) {
            return None;
        }
        let item = self.items[(t & MASK) as usize].load(Ordering::SeqCst);
        // A concurrent steal or pop took an element in the meantime.
        self.top.compare_exchange(t, t.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst).ok()?;
        debug_assert!(!item.is_null());
        self.num_queued.fetch_sub(1, Ordering::SeqCst);
        Some(item)
    }

    fn pop(&self) -> Option<*mut Job> {
        while self.num_queued.load(Ordering::Relaxed) != 0 {
            // bottom decremented speculatively, put back where nothing could be popped: work wasted only when there
            // is nothing to do. The pairing acquire is in `steal`.
            let b1 = self.bottom.fetch_sub(1, Ordering::Release);
            let b = b1.wrapping_sub(1);
            let t = self.top.load(Ordering::Acquire);
            if t == b || top_less_than_bottom(t, b) {
                let ptr = self.items[(b & MASK) as usize].load(Ordering::SeqCst);
                if t != b {
                    // More than one item left.
                    self.num_queued.fetch_sub(1, Ordering::SeqCst);
                    return Some(ptr);
                }
                // The last item: a race against a steal.
                if self.top.compare_exchange(t, t.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst).is_err() {
                    self.bottom.store(b1, Ordering::Relaxed);
                    continue;
                }
                self.bottom.store(t.wrapping_add(1), Ordering::Relaxed);
                self.num_queued.fetch_sub(1, Ordering::SeqCst);
                return Some(ptr);
            }
            // Already empty.
            self.bottom.store(b1, Ordering::Relaxed);
        }
        None
    }
}

/// Whether `t` is before `b`, across a wrap of the counters (more items implied than there are slots).
fn top_less_than_bottom(t: u32, b: u32) -> bool {
    t < b || t.wrapping_sub(b) > NUMBER_OF_ITEMS
}

/// A pool's list of the queues that may hold work, with the epoch it was published at.
struct Queues {
    epoch: u64,
    list: Vec<Arc<ThreadQueue>>,
}

/// One thread's entry in one pool (`ThreadEntry`).
struct Entry {
    /// Its pool: also the pool's identity (`Weak::ptr_eq`), as it keeps the pool's address from being reused.
    owner: Weak<Itl>,
    /// A worker's number; -1 for any other thread.
    #[allow(dead_code)]
    worker: i32,
    queue: Arc<ThreadQueue>,
    queues: RefCell<Arc<Queues>>,
    last_found: Cell<isize>,
}

impl Drop for Entry {
    /// The thread is going: its queue's jobs run first, then it is unpublished.
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            owner.unpublish(self);
        }
    }
}

thread_local! {
    /// This thread's entries, one a pool it has worked for (`ThreadSpecificInstanceInfo`); a pool's entry dropped once
    /// the pool is gone.
    static ENTRIES: RefCell<Vec<Rc<Entry>>> = const { RefCell::new(Vec::new()) };
}

/// A pool's state (`ThreadPool::Itl`).
pub struct Itl {
    me: Weak<Itl>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
    submitted: AtomicU32,
    finished: AtomicU32,
    jobs_stolen: AtomicU64,
    jobs_with_full_queue: AtomicU64,
    jobs_run_locally: AtomicU64,
    shutdown: AtomicBool,
    threads_sleeping: AtomicI32,
    wakeup_mutex: Mutex<()>,
    wakeup_cv: Condvar,
    epoch: AtomicU64,
    queues: Mutex<Arc<Queues>>,
    parent: Option<Arc<Itl>>,
    parent_jobs: AtomicUsize,
    max_parent_jobs: usize,
    has_exc: AtomicU32,
    exc: Mutex<Option<Panic>>,
}

struct SendPtr(*const Itl);
// SAFETY: a worker reads its pool through it while the pool lives: the pool's drop joins its workers first.
unsafe impl Send for SendPtr {}

impl Itl {
    fn make(parent: Option<Arc<Itl>>, max_parent_jobs: usize) -> Arc<Itl> {
        Arc::new_cyclic(|me| Itl {
            me: me.clone(),
            workers: Mutex::new(Vec::new()),
            submitted: AtomicU32::new(0),
            finished: AtomicU32::new(0),
            jobs_stolen: AtomicU64::new(0),
            jobs_with_full_queue: AtomicU64::new(0),
            jobs_run_locally: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            threads_sleeping: AtomicI32::new(0),
            wakeup_mutex: Mutex::new(()),
            wakeup_cv: Condvar::new(),
            epoch: AtomicU64::new(0),
            queues: Mutex::new(Arc::new(Queues { epoch: 0, list: Vec::new() })),
            parent,
            parent_jobs: AtomicUsize::new(0),
            max_parent_jobs,
            has_exc: AtomicU32::new(0),
            exc: Mutex::new(None),
        })
    }

    /// A root pool of `threads` threads of its own.
    pub fn root(threads: usize) -> Arc<Itl> {
        let itl = Itl::make(None, 0);
        let mut workers = itl.workers.lock().expect("the workers");
        for i in 0..threads {
            let p = SendPtr(Arc::as_ptr(&itl));
            let w = std::thread::Builder::new()
                .name(format!("rerail-mldb {i}"))
                .stack_size(crate::POOL_STACK)
                .spawn(move || {
                    let p = p;
                    crate::engine_thread();
                    // SAFETY: the pool lives until it has joined this thread (`Drop`).
                    unsafe { &*p.0 }.run_worker(i as i32);
                })
                .expect("rerail: an MLDB pool's thread");
            workers.push(w);
        }
        drop(workers);
        itl
    }

    /// A subordinate pool of `parent`: no threads, its work done by at most `max_parent_jobs` of the parent's jobs.
    pub fn under(parent: &Arc<Itl>, max_parent_jobs: usize) -> Arc<Itl> {
        Itl::make(Some(Arc::clone(parent)), max_parent_jobs)
    }

    pub fn num_threads(&self) -> usize {
        match &self.parent {
            Some(p) => p.num_threads().min(self.max_parent_jobs),
            None => self.workers.lock().expect("the workers").len(),
        }
    }

    fn jobs_running(&self) -> u32 {
        let s = self.submitted.load(Ordering::SeqCst);
        let f = self.finished.load(Ordering::SeqCst);
        s.wrapping_sub(f)
    }

    /// This thread's entry in the pool, published the first time (`getEntry`).
    fn entry(&self, worker: i32) -> Rc<Entry> {
        let found = ENTRIES.with(|e| {
            let mut e = e.borrow_mut();
            // The entries of pools that are gone go with this look.
            e.retain(|x| x.owner.strong_count() > 0 || x.owner.ptr_eq(&self.me));
            e.iter().find(|x| x.owner.ptr_eq(&self.me)).cloned()
        });
        if let Some(found) = found {
            return found;
        }
        let entry = Rc::new(Entry {
            owner: self.me.clone(),
            worker,
            queue: Arc::new(ThreadQueue::new()),
            queues: RefCell::new(Arc::new(Queues { epoch: 0, list: Vec::new() })),
            last_found: Cell::new(-1),
        });
        self.publish(&entry);
        ENTRIES.with(|e| e.borrow_mut().push(Rc::clone(&entry)));
        entry
    }

    fn run_parent_worker(&self) {
        while !self.shutdown.load(Ordering::SeqCst) && self.work() {}
        self.parent_jobs.fetch_sub(1, Ordering::SeqCst);
    }

    /// Adds `job` (`add`): on this thread's queue, else (a full queue) run here at once. Lock-free but for a thread's
    /// first job in the pool.
    pub fn add(&self, job: Job) {
        self.submitted.fetch_add(1, Ordering::SeqCst);
        let job = carried(job);
        let item = Box::into_raw(Box::new(job));
        let overflow = self.entry(-1).queue.push(item);
        match overflow {
            None => {
                if let Some(parent) = &self.parent {
                    // A parent job, if there are not enough already.
                    let were = self.parent_jobs.fetch_add(1, Ordering::SeqCst);
                    if were >= self.max_parent_jobs {
                        self.parent_jobs.fetch_sub(1, Ordering::SeqCst);
                    } else {
                        let weak = self.me.clone();
                        parent.add(Box::new(move || {
                            if let Some(strong) = weak.upgrade() {
                                strong.run_parent_worker();
                            }
                        }));
                    }
                } else if self.threads_sleeping.load(Ordering::SeqCst) > 0 {
                    // A thread just going to sleep may miss it: the sleeps are bounded for that.
                    self.wakeup_cv.notify_one();
                }
            }
            Some(item) => {
                // Full: the work here, while others steal some.
                self.jobs_with_full_queue.fetch_add(1, Ordering::Relaxed);
                // SAFETY: the item was never published; it is ours alone.
                self.run_job(*unsafe { Box::from_raw(item) });
            }
        }
    }

    /// Runs this thread's queue empty; whether it ran any.
    fn run_mine(&self, entry: &Entry) -> bool {
        let mut result = false;
        while let Some(job) = entry.queue.pop() {
            result = true;
            self.jobs_run_locally.fetch_add(1, Ordering::Relaxed);
            // SAFETY: a popped item is the popper's alone.
            self.run_job(*unsafe { Box::from_raw(job) });
        }
        result
    }

    /// Steals work from the other queues and runs it, and then its own; whether it found any.
    fn steal_work(&self, entry: &Entry) -> bool {
        let mut found = false;
        // The list refreshed where a thread came or went, if the lock is free.
        if self.epoch.load(Ordering::SeqCst) != entry.queues.borrow().epoch
            && let Ok(q) = self.queues.try_lock()
        {
            *entry.queues.borrow_mut() = Arc::clone(&q);
        }
        let queues = Arc::clone(&entry.queues.borrow());
        let steal_from = |n: usize, found: &mut bool| {
            let q = &queues.list[n];
            while let Some(job) = q.steal() {
                entry.last_found.set(n as isize);
                self.jobs_stolen.fetch_add(1, Ordering::Relaxed);
                // SAFETY: a stolen item is the stealer's alone.
                self.run_job(*unsafe { Box::from_raw(job) });
                *found = true;
                self.run_mine(entry);
            }
        };
        let nq = queues.list.len() as isize;
        let last = entry.last_found.get();
        if last > 0 && last < nq {
            steal_from(last as usize, &mut found);
        }
        let mut i = 0;
        while i < nq && !self.shutdown.load(Ordering::Relaxed) {
            // Not every thread starting at the same place.
            let n = (entry.last_found.get() + i).rem_euclid(nq);
            steal_from(n as usize, &mut found);
            i += 1;
        }
        found
    }

    fn run_job(&self, job: Job) {
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        self.finished.fetch_add(1, Ordering::SeqCst);
        if let Err(panic) = ran
            && self.has_exc.fetch_add(1, Ordering::SeqCst) == 0
        {
            *self.exc.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(panic);
        }
    }

    /// Works for the pool until every job added has finished; the first panic of a job resumed (`waitForAll`).
    pub fn wait_for_all(&self) {
        let entry = self.entry(-1);
        while !self.shutdown.load(Ordering::SeqCst) && self.jobs_running() > 0 {
            if !self.run_mine(&entry) {
                self.steal_work(&entry);
            }
        }
        if self.has_exc.load(Ordering::SeqCst) > 0
            && let Some(panic) = self.exc.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
        {
            std::panic::resume_unwind(panic);
        }
    }

    /// Does some work, where there is any; whether it did.
    pub fn work(&self) -> bool {
        let entry = self.entry(-1);
        self.run_mine(&entry) || self.steal_work(&entry)
    }

    fn sleep_for(&self, ms: u64) {
        self.threads_sleeping.fetch_add(1, Ordering::SeqCst);
        let guard = self.wakeup_mutex.lock().expect("the wakeup lock");
        let _ = self.wakeup_cv.wait_timeout(guard, std::time::Duration::from_millis(ms));
        self.threads_sleeping.fetch_sub(1, Ordering::SeqCst);
    }

    fn run_worker(&self, worker: i32) {
        let entry = self.entry(worker);
        let mut iters_with_no_work = 0;
        while !self.shutdown.load(Ordering::SeqCst) {
            if self.run_mine(&entry) || self.steal_work(&entry) {
                iters_with_no_work = 0;
                continue;
            }
            iters_with_no_work += 1;
            // Idle (MLDB-1538): no job outstanding; a long sleep.
            let s = self.submitted.load(Ordering::Relaxed);
            let f = self.finished.load(Ordering::Relaxed);
            if s == f {
                self.sleep_for(250);
                iters_with_no_work = 0;
            }
            if iters_with_no_work == 10 {
                self.sleep_for(10);
                iters_with_no_work = 0;
            } else {
                std::thread::yield_now();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        drop(entry);
        // This thread's entry goes with the pool's shutdown, not its thread's end: the pool is being dropped.
        ENTRIES.with(|e| e.borrow_mut().retain(|x| !x.owner.ptr_eq(&self.me)));
    }

    fn publish(&self, entry: &Entry) {
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let mut q = self.queues.lock().expect("the queues");
        let mut list = q.list.clone();
        list.push(Arc::clone(&entry.queue));
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *q = Arc::new(Queues { epoch, list });
    }

    fn unpublish(&self, entry: &Entry) {
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        // Its jobs first, or they would be lost.
        while self.run_mine(entry) {}
        let mut q = self.queues.lock().expect("the queues");
        let list: Vec<Arc<ThreadQueue>> = q.list.iter().filter(|x| !Arc::ptr_eq(x, &entry.queue)).cloned().collect();
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *q = Arc::new(Queues { epoch, list });
    }

    /// The pool's counts: jobs stolen, run past a full queue, run from their own thread's queue.
    pub fn counts(&self) -> (u64, u64, u64) {
        (self.jobs_stolen.load(Ordering::Relaxed), self.jobs_with_full_queue.load(Ordering::Relaxed), self.jobs_run_locally.load(Ordering::Relaxed))
    }
}

impl Drop for Itl {
    fn drop(&mut self) {
        {
            let _q = self.queues.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.shutdown.store(true, Ordering::SeqCst);
        }
        self.wakeup_cv.notify_all();
        let workers = std::mem::take(&mut *self.workers.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        for w in workers {
            let _ = w.join();
        }
    }
}

/// `job`, run in its maker's run (as the rayon patch's carrier does for rayon's jobs).
fn carried(job: Job) -> Job {
    match crate::carry() {
        None => job,
        Some(c) => Box::new(move || {
            let _in = crate::enter_carried(c);
            job();
        }),
    }
}

/// The machine's MLDB root pool, of `threads()` threads, made the first time it is asked for.
#[derive(Default)]
pub struct Root(OnceLock<Arc<Itl>>);

impl Root {
    pub fn pool(&self) -> &Arc<Itl> {
        self.0.get_or_init(|| Itl::root(crate::threads()))
    }
}

/// MLDB's `parallelMapChunked`: `f(start, end)` over `first..last` in chunks of `chunk`, on a subordinate pool of the
/// machine's root (as many parent jobs as the root has threads), `limit` workers at most (0: as many as the root has
/// threads) counting this thread's; the first panic resumed here.
pub fn parallel_map_chunked(first: usize, last: usize, chunk: usize, f: &(dyn Fn(usize, usize) + Sync), limit: usize) {
    assert!(chunk > 0 && last >= first);
    let root = Arc::clone(crate::machine::<Root>().pool());
    let threads = root.num_threads();
    let tp = Itl::under(&root, threads);
    let chunks = (last - first).div_ceil(chunk);
    let limit = if limit == 0 { threads } else { limit }.min(chunks);
    let has_exception = AtomicBool::new(false);
    let index = AtomicUsize::new(first);
    let worker = || {
        while !has_exception.load(Ordering::Relaxed) {
            let at = index.fetch_add(chunk, Ordering::SeqCst);
            if at >= last {
                return;
            }
            let end = last.min(at + chunk);
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(at, end))).is_err() {
                has_exception.store(true, Ordering::SeqCst);
                panic!("rerail: a job of an MLDB parallel map panicked");
            }
        }
    };
    struct Erased(*const (dyn Fn() + Sync));
    // SAFETY: each job reads `worker` before `wait_for_all` returns, which this function waits for.
    unsafe impl Send for Erased {}
    let w: &(dyn Fn() + Sync) = &worker;
    // SAFETY: lifetime erased; read only while this frame waits in `wait_for_all`.
    let w: *const (dyn Fn() + Sync) = unsafe { std::mem::transmute::<*const (dyn Fn() + Sync + '_), *const (dyn Fn() + Sync + 'static)>(w) };
    for _ in 1..limit {
        let e = Erased(w);
        tp.add(Box::new(move || {
            let e = e;
            // SAFETY: as `Erased`.
            (unsafe { &*e.0 })()
        }));
    }
    // Leave one set of the work for this thread.
    let mine = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&worker));
    tp.wait_for_all();
    if let Err(p) = mine {
        std::panic::resume_unwind(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each item once, the chunks over threads of the root; nested maps (a job's own map) finish; a panic is resumed.
    #[test]
    fn a_parallel_map_runs_each_item_once_nested_and_panics_resumed() {
        let seen: Vec<AtomicU32> = (0..10_000).map(|_| AtomicU32::new(0)).collect();
        parallel_map_chunked(0, seen.len(), 7, &|a, b| (a..b).for_each(|i| { seen[i].fetch_add(1, Ordering::Relaxed); }), 0);
        assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 1), "each once");
        let inner = AtomicUsize::new(0);
        parallel_map_chunked(0, 16, 1, &|_, _| parallel_map_chunked(0, 100, 3, &|a, b| { inner.fetch_add(b - a, Ordering::Relaxed); }, 0), 0);
        assert_eq!(inner.load(Ordering::Relaxed), 1600, "nested maps finish");
        let threads: std::sync::Mutex<std::collections::HashSet<std::thread::ThreadId>> = Default::default();
        parallel_map_chunked(0, 64, 1, &|_, _| { threads.lock().unwrap().insert(std::thread::current().id()); std::thread::sleep(std::time::Duration::from_millis(2)); }, 3);
        assert!(threads.lock().unwrap().len() <= 3, "at most its limit of threads");
        let panicked = std::panic::catch_unwind(|| parallel_map_chunked(0, 100, 1, &|a, _| assert!(a != 50, "item 50"), 0));
        assert!(panicked.is_err(), "a job's panic resumed");
        let after = AtomicUsize::new(0);
        parallel_map_chunked(0, 1000, 10, &|a, b| { after.fetch_add(b - a, Ordering::Relaxed); }, 0);
        assert_eq!(after.load(Ordering::Relaxed), 1000, "the pool works after a panic");
    }
}
