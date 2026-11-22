//! The run under way, and what its caller shares by design (decisions 0441, 0442): the bottom of every crate that
//! holds a run's state, so that a crate below the engine's logic (the job pool, the device, the compiler, the blocks)
//! keeps its run data in the run, and its shared and machine state in the objects the run's caller passed, never in
//! a static.
//!
//! - `RUN`: the run under way on this thread, made and carried by `rerail_logic::run` (and `run_state`, to its jobs);
//! - `setting`, `local`, `with_local`, `local_if_any`: a run's settings and its own values, by type;
//! - `Shared` (`shared`, `with_shared`): what a caller shares by design with the runs it hands the object;
//! - `Machine` (`machine`): the machine's resources, which every `Shared` is made on; among them its job pool
//!   (`Cores`: `on_cores`, `cores_spawn`, `cores_scope`);
//! - `bind`, `bound`, `with_bound`: the run's context, a value by type for an extent of the run (`CONTEXT`), carried
//!   with the run to its jobs.
//!
//! Outside a run, each gives a value of its own that nothing keeps.

thread_local! {
    /// The run under way on this thread (decision 0441): its own state, `rerail_logic::run::Run`, made and carried by
    /// the crate that defines it, held here so that every crate below it reaches it too.
    pub static RUN: std::cell::RefCell<Option<std::sync::Arc<dyn RunOwn>>> = const { std::cell::RefCell::new(None) };
}

/// What the crates below `logic` tell the run under way, and read of it (`RUN`; decisions 0441, 0442).
pub trait RunOwn: Send + Sync {
    /// A job's handle (`job_run`): the run, and the context it was taken in. `enter` puts both on the job's thread.
    fn with_context(&self) -> Option<(&std::sync::Arc<dyn RunOwn>, &Context)> {
        None
    }
    /// Its settings and its own values (`setting`, `local`).
    fn values(&self) -> &RunValues;
    /// What its caller shares by design with the other runs it hands the same object (`shared`).
    fn shared(&self) -> &std::sync::Arc<Shared>;
    /// The run itself, for the crate that defines it.
    fn any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync>;
}

/// The run under way on this thread (`RUN`), if any.
pub fn run_own() -> Option<std::sync::Arc<dyn RunOwn>> {
    // A thread-local's drop, as its thread ends, may ask after this one's: then no run.
    RUN.try_with(|r| r.borrow().clone()).ok().flatten()
}

pub type AnyArc = std::sync::Arc<dyn std::any::Any + Send + Sync>;

/// The place a setting is read (its file, line and column) and the type it is read as.
type SettingPlace = (&'static str, u32, u32, std::any::TypeId);

/// A run's settings, by the place that reads each, and its own values, by type (decision 0442).
#[derive(Default)]
pub struct RunValues {
    settings: parking_lot::Mutex<std::collections::HashMap<SettingPlace, AnyArc>>,
    locals: parking_lot::Mutex<std::collections::HashMap<std::any::TypeId, AnyArc>>,
}

/// What is shared by design (decision 0442): a store's resources, learned models, executors, interners. A caller makes
/// it and hands it to the runs it means to share them (`logic::run::scope_in`); a run handed none has one of its own,
/// and is as apart from every other as a run in another process. Nothing else reaches it: two callers with different
/// trust hold different objects.
#[derive(Default)]
pub struct Shared {
    values: parking_lot::Mutex<std::collections::HashMap<std::any::TypeId, AnyArc>>,
    /// The machine it runs on (`Machine`).
    machine: std::sync::Arc<Machine>,
}

/// The machine's resources (decision 0442, the user's choice): the CPU job pool and its queues, the pool threads'
/// accounts, the measured roofline, the cost and price model learned from the machine's runs. A caller makes it, and
/// every `Shared` is made on one (`Shared::on`): callers of different trust hold different `Shared` objects on one
/// `Machine`, with one pool and one model of the machine. A job on the pool carries its own caller's run and `Shared`.
#[derive(Default)]
pub struct Machine {
    values: parking_lot::Mutex<std::collections::HashMap<std::any::TypeId, AnyArc>>,
}

impl Machine {
    pub fn new() -> std::sync::Arc<Machine> {
        // Before any of its pools makes a job: every pool's jobs carry their makers' runs.
        install_carrier();
        std::sync::Arc::default()
    }
    /// Its value of type `T`, made the first time it is asked for.
    pub fn get<T: Default + Send + Sync + 'static>(&self) -> std::sync::Arc<T> {
        let v = std::sync::Arc::clone(self.values.lock().entry(std::any::TypeId::of::<T>()).or_insert_with(|| std::sync::Arc::new(T::default())));
        v.downcast::<T>().expect("a machine value's type is its key")
    }
}

impl Shared {
    /// A shared object on a machine of its own.
    pub fn new() -> std::sync::Arc<Shared> {
        std::sync::Arc::default()
    }
    /// A shared object on `machine`.
    pub fn on(machine: &std::sync::Arc<Machine>) -> std::sync::Arc<Shared> {
        std::sync::Arc::new(Shared { values: Default::default(), machine: std::sync::Arc::clone(machine) })
    }
    /// The machine it runs on.
    pub fn machine(&self) -> &std::sync::Arc<Machine> {
        &self.machine
    }
    /// Its value of type `T`, made the first time it is asked for.
    pub fn get<T: Default + Send + Sync + 'static>(&self) -> std::sync::Arc<T> {
        let v = std::sync::Arc::clone(self.values.lock().entry(std::any::TypeId::of::<T>()).or_insert_with(|| std::sync::Arc::new(T::default())));
        v.downcast::<T>().expect("a shared value's type is its key")
    }
    /// Its value of type `T`, if it was made.
    pub fn get_if_any<T: Send + Sync + 'static>(&self) -> Option<std::sync::Arc<T>> {
        self.values.lock().get(&std::any::TypeId::of::<T>()).cloned()?.downcast::<T>().ok()
    }
}

/// The shared object of a run handed none: in a product build, one of its own (as apart from every other run as one in
/// another process); in a test build (`test-hooks`), the tests' one (`TESTS`), as the process's statics were.
pub fn default_shared() -> std::sync::Arc<Shared> {
    #[cfg(feature = "test-hooks")]
    return std::sync::Arc::clone(&TESTS);
    #[cfg(not(feature = "test-hooks"))]
    Shared::new()
}

/// The shared object a read of `what` outside any run takes. Decision 0442 refuses such a read: every entry point and
/// every thread the engine makes holds a run or a shared object (`enter`, `within`), so it is a miss (`missed`). In a
/// product build it takes an object of its own, which reaches nothing of any caller's and which nothing keeps. In a
/// test build, a test's thread (one the engine did not make: `on_a_tests_thread`) is a caller with no entry point
/// (it calls the engine's parts directly), whose object is the tests' one (`TESTS`).
#[cold]
fn outside_a_run(what: &'static str) -> std::sync::Arc<Shared> {
    #[cfg(feature = "test-hooks")]
    if on_a_tests_thread() {
        return std::sync::Arc::clone(&TESTS);
    }
    missed(what);
    Shared::new()
}

/// Whether this thread is a test's (test builds): one the engine did not make (`engine_thread`). A test's own thread,
/// and every thread a test starts itself, is a caller's; the engine's threads (its pools, its services) run its jobs,
/// which carry their caller's run (`job_run`), so a read outside any run there is a job that lost it.
#[cfg(feature = "test-hooks")]
fn on_a_tests_thread() -> bool {
    !ENGINE.with(std::cell::Cell::get)
}

#[cfg(feature = "test-hooks")]
thread_local! {
    /// Whether the engine made this thread (`engine_thread`): test builds, for `on_a_tests_thread`.
    static ENGINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks this thread as one the engine made (its pools' threads as they start, each service's): in a test build, a
/// read outside any run on it is refused (`missed`), as every read is in a product build. A no-op in a product build.
pub fn engine_thread() {
    #[cfg(feature = "test-hooks")]
    ENGINE.with(|e| e.set(true));
}

/// A read of `what` (a type's name) outside any run, refused (decision 0442). In a test build (`test-hooks`) it panics
/// on the engine's threads (a test's take the tests' object: `outside_a_run`): a job runs in its caller's run, and a
/// thread holds its starter's object. In a product build it is said once a type on stderr, with its backtrace, and kept (`misses`): the command
/// fails at its end (`tools::cli`), with its output written, as a run cut at the width cap does.
#[cold]
#[track_caller]
fn missed(what: &'static str) {
    #[cfg(feature = "test-hooks")]
    panic!(
        "rerail: a read of {what} outside any run, on thread {:?} (decision 0442): a job runs in its caller's run, and a thread holds its starter's object (`within`)",
        std::thread::current().name()
    );
    #[cfg(not(feature = "test-hooks"))]
    {
        let mut said = MISSES.lock();
        if !said.contains(&what) {
            said.push(what);
            eprintln!("rerail: a read of {what} outside any run (decision 0442)\n{}", std::backtrace::Backtrace::force_capture());
        }
    }
}

/// The types read outside any run in this process (`missed`): an error's report, never read by the engine. The
/// process's, as its stderr is: a read outside any run is no run's.
#[cfg(not(feature = "test-hooks"))]
static MISSES: parking_lot::Mutex<Vec<&'static str>> = parking_lot::Mutex::new(Vec::new());

/// The types read outside any run so far in this process (`missed`), for the command's error at its end. A test build
/// has none: a miss panics there.
pub fn misses() -> Vec<&'static str> {
    #[cfg(not(feature = "test-hooks"))]
    return MISSES.lock().clone();
    #[cfg(feature = "test-hooks")]
    Vec::new()
}

/// The tests' shared object (`default_shared`): test builds only.
#[cfg(feature = "test-hooks")]
static TESTS: std::sync::LazyLock<std::sync::Arc<Shared>> = std::sync::LazyLock::new(Shared::new);

/// The value of type `T` that the run under way shares by design (`Shared`), made the first time it is asked for.
/// Outside a run: `outside_a_run`'s.
#[inline]
pub fn shared<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let held = SEEN.try_with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.shared.get(&std::any::TypeId::of::<T>()).cloned() } else { None }
    })
    .ok()
    .flatten();
    match held {
        Some(v) => v.downcast::<T>().expect("a shared value's type is its key"),
        None => shared_slow::<T>(),
    }
}

#[cold]
fn shared_slow<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let Some(run) = run_own() else { return outside_a_run(std::any::type_name::<T>()).get::<T>() };
    let v = run.shared().get::<T>();
    let _ = SEEN.try_with(|seen| seen.borrow_mut().of(&run).shared.insert(std::any::TypeId::of::<T>(), v.clone()));
    v
}

/// `f` of the shared object's value of type `T` (`shared`), on a hot path: this thread's copy of the run's shared
/// values (`SEEN`) answers with no lock and no count of references, so readers on many threads write no shared word.
#[inline]
pub fn with_shared<T: Default + Send + Sync + 'static, R>(f: impl FnOnce(&T) -> R) -> R {
    let held = SEEN.try_with(|seen| {
        let seen = seen.borrow();
        if !seen.is_current() {
            return None;
        }
        seen.shared.get(&std::any::TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>()).map(|t| t as *const T)
    })
    .ok()
    .flatten();
    match held {
        // SAFETY: `SEEN` holds the value (an `Arc` of the shared object's) while the run under way on this thread is
        // the one it was copied from; nothing `f` runs ends that run (a run nested in it joins it).
        Some(t) => f(unsafe { &*t }),
        None => f(&shared::<T>()),
    }
}

/// The value of type `T` of the machine the run under way runs on (`Machine`), made the first time it is asked for.
/// Outside a run: `outside_a_run`'s machine's.
#[inline]
pub fn machine<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let held = SEEN.try_with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.machine.get(&std::any::TypeId::of::<T>()).cloned() } else { None }
    })
    .ok()
    .flatten();
    match held {
        Some(v) => v.downcast::<T>().expect("a machine value's type is its key"),
        None => machine_slow::<T>(),
    }
}

#[cold]
fn machine_slow<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let Some(run) = run_own() else { return outside_a_run(std::any::type_name::<T>()).machine().get::<T>() };
    let v = run.shared().machine().get::<T>();
    let _ = SEEN.try_with(|seen| seen.borrow_mut().of(&run).machine.insert(std::any::TypeId::of::<T>(), v.clone()));
    v
}

/// The most threads a machine's pool has: `RERAIL_THREADS`, a whole number of at least 1; else the cores the machine
/// offers. The process's environment, read once: every machine of the process is the same machine.
pub fn threads() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("RERAIL_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
    })
}

/// The machine's main thread: the one `Machine::here` took into its pool as thread 0, which lives as long as the
/// process. Recorded when it is taken in, not asked of the system: Windows has no call that says which thread is a
/// process's main one (main's #929, where it was always false there). The machine's (decision 0442), not a global.
#[derive(Default)]
pub struct MainThread(std::sync::OnceLock<std::thread::ThreadId>);

/// Whether this is the main thread of the run's machine (`MainThread`, `Machine::here`'s); false outside a run, and
/// on a machine no main thread made.
pub fn is_main_thread() -> bool {
    run_own().is_some_and(|run| run.shared().machine().get::<MainThread>().0.get() == Some(&std::thread::current().id()))
}

/// A pool thread's stack: pages are committed only as they are touched, so this is address space, not memory.
pub const POOL_STACK: usize = 16 << 20;

/// The machine's cores (decision 0442 step 5): its one job pool, [`threads`] threads named `rerail-cpu <i>`, made at
/// its first job. Every job of a caller on the machine runs on it: the cores' launches, host jobs, and every parallel
/// loop of the engine (`on_cores`, `cores_spawn`, `cores_scope`). Two machines in one process have two pools; two
/// callers on one machine share it, each job in its own caller's run.
#[derive(Default)]
pub struct Cores(std::sync::OnceLock<rayon::ThreadPool>);

impl Cores {
    /// The pool, made the first time it is asked for.
    pub fn pool(&self) -> &rayon::ThreadPool {
        self.0.get_or_init(|| make_pool(false))
    }
}

/// This thread's context.
fn context_here() -> Context {
    CONTEXT.try_with(|c| c.borrow().clone()).unwrap_or_default()
}

/// What a job made on this thread carries to the thread that runs it (rayon-core's patch, `set_job_carrier`): the
/// run under way and its context (`job_run`). A loop's items split by rayon's `join`, and a half another thread steals
/// runs in its maker's run, whichever runs share the pool (decision 0442, Q300). Nothing where the thread holds no
/// run: a thread the engine gave none (in a test build, a test's own thread carries the tests' object).
fn carry() -> Option<rayon_core::Carried> {
    if RUN.try_with(|r| r.borrow().is_none()).unwrap_or(true) {
        #[cfg(feature = "test-hooks")]
        if on_a_tests_thread() {
            return Some(Box::new(job_run()));
        }
        return None;
    }
    Some(Box::new(job_run()))
}

/// Enters what a job carried (`carry`) on the thread that runs it, until the guard drops; nothing where the thread
/// holds that run and context already.
fn enter_carried(carried: rayon_core::Carried) -> Box<dyn std::any::Any> {
    let job = *carried.downcast::<Option<std::sync::Arc<dyn RunOwn>>>().expect("rerail: the engine's job carrier carries a job's run");
    if holds(&job) {
        return Box::new(());
    }
    Box::new(enter(job))
}

/// Installs the engine's job carrier (`carry`) before the first pool is made: code registered at load, set once for
/// the process (later calls are ignored).
fn install_carrier() {
    rayon_core::set_job_carrier(carry, enter_carried);
}

/// A pool of [`threads`] threads; with `this_thread`, the calling thread its thread 0. Its jobs carry their makers'
/// runs (`install_carrier`).
fn make_pool(this_thread: bool) -> rayon::ThreadPool {
    install_carrier();
    let b = rayon::ThreadPoolBuilder::new()
        .num_threads(threads())
        .thread_name(|i| format!("rerail-cpu {i}"))
        .stack_size(POOL_STACK)
        .start_handler(move |_| engine_thread());
    if this_thread { b.use_current_thread() } else { b }.build().expect("rerail: the machine's job pool")
}

impl Machine {
    /// A machine whose pool has the calling thread as its thread 0, so the work this thread does itself, and its
    /// waits that run other jobs, are the pool's too: a program's main thread, before its first job. The pool's hold
    /// on the thread is never given back (rayon keeps its registry), so it is made once, by the thread that lives as
    /// long as the process; any other caller makes a machine with `new`, whose pool's threads are its own and end
    /// with it.
    pub fn here() -> std::sync::Arc<Machine> {
        let machine = Machine::new();
        let _ = machine.get::<MainThread>().0.set(std::thread::current().id());
        let _ = machine.get::<Cores>().0.set(make_pool(true));
        machine
    }
}

/// Whether this thread is one of a pool's: a machine's (its cores), or a pool of the machine's own lanes. There a
/// parallel loop splits over the thread's own pool, as rayon's functions do.
/// wasm32 has no threads: there rayon's functions run their work here, so every call is as on a pool's thread.
fn on_a_pool() -> bool {
    cfg!(target_family = "wasm") || rayon::current_thread_index().is_some()
}

/// Runs `f` on the caller's machine's cores ([`Cores`]): every parallel loop of the engine is inside one, so its items
/// split over the machine's pool. On one of a pool's threads it runs here; else on one of the cores', in this
/// caller's run and context (`job_run`), while this thread waits.
pub fn on_cores<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    // A loop's halves another thread steals carry this caller's run (`carry`), on a pool's thread or on the cores.
    if on_a_pool() {
        return f();
    }
    let run = job_run();
    machine::<Cores>().pool().install(move || {
        let _in = enter(run);
        f()
    })
}

/// Starts `f` as a job on the caller's machine's cores (on a pool's thread, that pool's). It carries nothing: a job
/// that reads its caller's run carries it itself (`job_run`).
pub fn cores_spawn(f: impl FnOnce() + Send + 'static) {
    if on_a_pool() {
        rayon::spawn(f);
    } else {
        machine::<Cores>().pool().spawn(f);
    }
}

/// `cores_spawn`, the job queued first in, first out (a batch's launches in their order).
pub fn cores_spawn_fifo(f: impl FnOnce() + Send + 'static) {
    if on_a_pool() {
        rayon::spawn_fifo(f);
    } else {
        machine::<Cores>().pool().spawn_fifo(f);
    }
}

/// Runs `op` on this thread with a scope whose jobs run on the caller's machine's cores (on a pool's thread, that
/// pool's); returns once they have all ended.
pub fn cores_scope<'s, R>(op: impl FnOnce(&rayon::Scope<'s>) -> R) -> R {
    if on_a_pool() {
        rayon::in_place_scope(op)
    } else {
        machine::<Cores>().pool().in_place_scope(op)
    }
}

/// The threads of the pool a parallel loop of this caller's splits over (`on_cores`).
pub fn cores_threads() -> usize {
    if on_a_pool() { rayon::current_num_threads() } else { machine::<Cores>().pool().current_num_threads() }
}

/// The shared object of the run under way, if one is (for a caller that hands it on).
pub fn shared_object() -> Option<std::sync::Arc<Shared>> {
    run_own().map(|r| std::sync::Arc::clone(r.shared()))
}

/// The object a thread started here holds (a service: `within` there): the run's; with no run, on a test's thread,
/// the tests' one (a caller with no entry point: `outside_a_run`); else none (a thread that holds no caller's state:
/// the process's signal watcher, started by `main`).
pub fn starter_object() -> Option<std::sync::Arc<Shared>> {
    #[cfg(feature = "test-hooks")]
    if run_own().is_none() && on_a_tests_thread() {
        return Some(std::sync::Arc::clone(&TESTS));
    }
    shared_object()
}

/// The shared object a thread reads by: its run's, else `outside_a_run`'s (for a thread that hands it on: a service
/// it starts, a machine's own thread).
pub fn shared_here() -> std::sync::Arc<Shared> {
    shared_object().unwrap_or_else(|| outside_a_run("the shared object"))
}

/// Puts `run` on this thread (a job of it, on another thread) until the guard drops, then what the thread had. A job's
/// handle (`job_run`) puts its context on the thread too; a run alone leaves the thread's context as it is.
pub fn enter(run: Option<std::sync::Arc<dyn RunOwn>>) -> impl Drop {
    struct Back(Option<Option<std::sync::Arc<dyn RunOwn>>>, Option<Context>);
    impl Drop for Back {
        fn drop(&mut self) {
            let was = self.0.take().flatten();
            // The context first, as it was entered last; each dropped outside its cell (a value's drop may read it).
            let context = self.1.take().and_then(|c| CONTEXT.try_with(|x| x.replace(c)).ok());
            drop(context);
            // The run that leaves, dropped here: if it was the last holder, the run has ended.
            let left = RUN.try_with(|r| r.replace(was)).ok().flatten();
            drop(left);
            left_run();
        }
    }
    let (run, context) = match run {
        Some(job) => match job.with_context() {
            Some((run, context)) => (Some(std::sync::Arc::clone(run)), Some(context.clone())),
            None => (Some(job), None),
        },
        None => (None, None),
    };
    let was_run = RUN.try_with(|r| r.replace(run)).ok().flatten();
    let was_context = context.and_then(|c| CONTEXT.try_with(|x| x.replace(c)).ok());
    Back(Some(was_run), was_context)
}

/// Runs `f` in `run` (a job's handle, `job_run`) on this thread, and ends it there: `f` and everything it captured
/// are dropped before the run leaves the thread. A job written `move || { let _in = enter(run); … }` drops its
/// captures after its body, when the run has left: a capture's drop that reads the run (a device lease's going back
/// to its pool reads the machine's) then reads outside any run (Q337).
pub fn in_run<R>(run: Option<std::sync::Arc<dyn RunOwn>>, f: impl FnOnce() -> R) -> R {
    let _in = enter(run);
    f()
}

/// Whether this thread holds `job`'s run and context already (a pool's item run inline on its caller's thread).
pub fn holds(job: &Option<std::sync::Arc<dyn RunOwn>>) -> bool {
    let (run, context) = match job.as_ref().map(|j| (j, j.with_context())) {
        Some((_, Some((run, context)))) => (Some(run), Some(context)),
        Some((run, None)) => (Some(run), None),
        None => (None, None),
    };
    let same_run = RUN.with(|r| match (r.borrow().as_ref(), run) {
        (Some(a), Some(b)) => std::ptr::addr_eq(std::sync::Arc::as_ptr(a), std::sync::Arc::as_ptr(b)),
        (None, None) => true,
        _ => false,
    });
    same_run && context.is_none_or(|c| CONTEXT.with(|x| x.borrow().same(c)))
}

/// A run has left this thread (a guard that put it there dropped, `RUN` back as it was): unless the run back on the
/// thread is the one it copied, this thread's copy of the run's values (`SEEN`) is dropped now, outside the copy's
/// borrow (a value's drop may read a run). The copy lives while the thread is in the run, never after: kept, a pool
/// thread whose job ended before its run did would hold the run's locals and its shared and machine values until it
/// next entered a run (Q377: a test's store's connections, its database files open after the test), or until it
/// exited, where a value's drop can find the thread's other locals gone.
pub fn left_run() {
    let dead = SEEN
        .try_with(|seen| {
            let mut seen = seen.borrow_mut();
            (seen.run.is_some() && !seen.is_current()).then(|| std::mem::take(&mut *seen))
        })
        .ok()
        .flatten();
    drop(dead);
}

/// The run under way on this thread has ended (its outermost scope, on its own thread): this thread's copy of its
/// values (`SEEN`) is dropped now (`left_run`).
pub fn run_ended() {
    let held = SEEN.try_with(|seen| std::mem::take(&mut *seen.borrow_mut())).ok();
    drop(held);
}

/// Puts a run of its own on this thread, on `shared`, until the guard drops: for a thread that outlives the run that
/// started it (a service, a machine's own thread), so that it reads its caller's shared and machine values and keeps
/// no run alive.
pub fn within(shared: std::sync::Arc<Shared>) -> impl Drop {
    enter(Some(std::sync::Arc::new(Bare(RunValues::default(), shared))))
}

/// The run a job of this thread's runs in, on whichever thread takes it (`enter` there): the run under way, else a run
/// of its own on this thread's object (`shared_here`: on a test's thread, the tests' one). A job runs as its caller
/// does (decision 0442), whether or not the caller holds a run.
pub fn job_run() -> Option<std::sync::Arc<dyn RunOwn>> {
    let run = run_own().unwrap_or_else(|| std::sync::Arc::new(Bare(RunValues::default(), shared_here())));
    let context = context_here();
    Some(std::sync::Arc::new(InContext(run, context)))
}

/// A job's handle (`job_run`): the run, and the context the job was made in.
struct InContext(std::sync::Arc<dyn RunOwn>, Context);

/// Dropped on a thread that holds no run (a pool thread whose job carried it, its guard gone): the context drops
/// with the run's shared object on the thread (`within`), as its values' drops may read it (a store's writers). The
/// run is dropped after, its own values likewise (`Bare`'s drop).
impl Drop for InContext {
    fn drop(&mut self) {
        if run_own().is_none() {
            let _within = within(std::sync::Arc::clone(self.0.shared()));
            drop(std::mem::take(&mut self.1));
        }
    }
}

impl RunOwn for InContext {
    fn with_context(&self) -> Option<(&std::sync::Arc<dyn RunOwn>, &Context)> {
        Some((&self.0, &self.1))
    }
    fn values(&self) -> &RunValues {
        self.0.values()
    }
    fn shared(&self) -> &std::sync::Arc<Shared> {
        self.0.shared()
    }
    fn any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync> {
        std::sync::Arc::clone(&self.0).any()
    }
}

thread_local! {
    /// The run's context on this thread (decision 0442): a value by type for each extent under way (`bind`), carried
    /// with the run to its jobs (`job_run`, `enter`).
    static CONTEXT: std::cell::RefCell<Context> = const { std::cell::RefCell::new(Context(None)) };
}

/// The run's context: a chain of values by type, the nearest binding first. Shared by the jobs taken in it; a binding
/// adds a link and leaves the chain it was made on as it was.
#[derive(Clone, Default)]
pub struct Context(Option<std::sync::Arc<Link>>);

struct Link {
    key: std::any::TypeId,
    value: Box<dyn std::any::Any + Send + Sync>,
    up: Context,
    /// Made by `bind_here` (it ends with the extent under way), not by a guard.
    here: bool,
}

impl Context {
    fn find<T: 'static>(&self) -> Option<&T> {
        let key = std::any::TypeId::of::<T>();
        let mut at = self.0.as_deref();
        while let Some(link) = at {
            if link.key == key {
                return link.value.downcast_ref::<T>();
            }
            at = link.up.0.as_deref();
        }
        None
    }

    fn same(&self, other: &Context) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    fn with<T: Send + Sync + 'static>(&self, value: T, here: bool) -> Context {
        Context(Some(std::sync::Arc::new(Link { key: std::any::TypeId::of::<T>(), value: Box::new(value), up: self.clone(), here })))
    }

    /// The nearest link a guard made, past those `bind_here` made above it: a test build's check of the guards' order.
    #[cfg(feature = "test-hooks")]
    fn guarded(&self) -> Option<&std::sync::Arc<Link>> {
        let mut at = self.0.as_ref();
        while let Some(link) = at.filter(|l| l.here) {
            at = link.up.0.as_ref();
        }
        at
    }
}

/// The value of type `T` the run's context binds here (the nearest `bind`), else `T`'s default.
pub fn bound<T: Clone + Default + Send + Sync + 'static>() -> T {
    context_here().find::<T>().cloned().unwrap_or_default()
}

/// `f` of the value of type `T` the run's context binds here, else of `T`'s default. `f` may bind and read the
/// context: it holds the chain, not the thread's cell.
pub fn with_bound<T: Default + Send + Sync + 'static, R>(f: impl FnOnce(&T) -> R) -> R {
    let context = context_here();
    match context.find::<T>() {
        Some(t) => f(t),
        None => f(&T::default()),
    }
}

/// Binds `value` in the run's context on this thread, and in every job taken here, until the guard drops; then the
/// context is as it was.
pub fn bind<T: Send + Sync + 'static>(value: T) -> Bound {
    let mut bound = Bound {
        was: None,
        #[cfg(feature = "test-hooks")]
        made: 0,
        on_thread: std::marker::PhantomData,
    };
    // No context on this thread (it is being torn down): the guard puts nothing back.
    let _ = CONTEXT.try_with(|c| {
        let next = c.borrow().with(value, false);
        #[cfg(feature = "test-hooks")]
        {
            bound.made = next.0.as_ref().map_or(0, |l| std::sync::Arc::as_ptr(l) as usize);
        }
        bound.was = Some(c.replace(next));
    });
    bound
}

/// Binds `value` for the rest of the extent under way on this thread (an extent's guard dropping takes it with it): a
/// caller's choice for its runs (the profile's switch, the options for its inputs).
pub fn bind_here<T: Send + Sync + 'static>(value: T) {
    let was = CONTEXT.try_with(|c| {
        let next = {
            let here = c.borrow();
            // A binding of the same type `bind_here` made at the top is replaced, so that a switch set again and again
            // stays one link. A guard's link is never replaced: its extent ends where the guard drops.
            match here.0.as_deref() {
                Some(top) if top.here && top.key == std::any::TypeId::of::<T>() => top.up.with(value, true),
                _ => here.with(value, true),
            }
        };
        c.replace(next)
    });
    drop(was);
}

/// A binding (`bind`): dropped, the run's context on this thread is as it was before. It stays on its thread (not
/// `Send`): dropped on another, it would set that thread's context.
#[must_use = "the binding ends when the guard drops"]
pub struct Bound {
    /// The context it replaced, put back when it drops.
    was: Option<Context>,
    /// The link it made, by address: a test build checks it is the nearest a guard made when it drops.
    #[cfg(feature = "test-hooks")]
    made: usize,
    on_thread: std::marker::PhantomData<*const ()>,
}

impl Drop for Bound {
    fn drop(&mut self) {
        // A test build checks that guards drop in the reverse of their order: this guard's link is the nearest a guard
        // made. Dropped out of order, an inner binding would outlive its guard.
        #[cfg(feature = "test-hooks")]
        if self.was.is_some() && !std::thread::panicking() {
            let nearest = CONTEXT.try_with(|c| c.borrow().guarded().map_or(0, |l| std::sync::Arc::as_ptr(l) as usize)).unwrap_or(self.made);
            assert_eq!(nearest, self.made, "rerail: a binding of the run's context dropped out of order (decision 0442): guards drop in the reverse of their order");
        }
        let left = self.was.take().and_then(|was| CONTEXT.try_with(|c| c.replace(was)).ok());
        drop(left);
    }
}

/// What a thread holds for its caller (`caller`): `job_run`'s run.
#[derive(Clone)]
pub struct Caller(Option<std::sync::Arc<dyn RunOwn>>);

/// What this thread holds, for a thread it starts (`Caller::enter` there): `job_run`.
pub fn caller() -> Caller {
    Caller(job_run())
}

impl Caller {
    /// Puts the caller's run on this thread until the guard drops.
    pub fn enter(&self) -> impl Drop {
        enter(self.0.clone())
    }
}

/// A run with nothing of its own but its values and its shared object (`within`, `test_run`).
struct Bare(RunValues, std::sync::Arc<Shared>);

/// A run's last handle dropped on a thread that holds no run: its values drop with its shared object on the thread
/// (`within`), as a value's drop may read it. The run `within` makes holds no values, so its own drop does nothing.
impl Drop for Bare {
    fn drop(&mut self) {
        self.0.end_in(&self.1);
    }
}

impl RunValues {
    /// Drops the values, its run ending: on a thread that holds no run, with the run's `shared` object on the thread
    /// (`within`), as a value's drop may read it (a store's writers). Every run's drop calls it.
    pub fn end_in(&mut self, shared: &std::sync::Arc<Shared>) {
        let (settings, locals) = (std::mem::take(self.settings.get_mut()), std::mem::take(self.locals.get_mut()));
        if (settings.is_empty() && locals.is_empty()) || run_own().is_some() {
            return;
        }
        let _within = within(std::sync::Arc::clone(shared));
        drop((settings, locals));
    }
}

impl RunOwn for Bare {
    fn values(&self) -> &RunValues {
        &self.0
    }
    fn shared(&self) -> &std::sync::Arc<Shared> {
        &self.1
    }
    fn any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// A setting of the run under way (decision 0442), by the place that reads it: `read` (the environment, as a rule) runs
/// once a run, as the run first needs it, and the run keeps the value; a caller that changes the environment between
/// runs gets the new value in the next. Outside a run, `read` runs each time.
#[track_caller]
#[inline]
pub fn setting<T: Clone + Send + Sync + 'static>(read: impl FnOnce() -> T) -> T {
    let at = std::panic::Location::caller();
    // This thread's copy of the run's settings (`SEEN`): a read on a hot path takes no lock.
    let site = at as *const std::panic::Location<'static> as usize;
    let hit = SEEN.try_with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.settings.get(&site).and_then(|v| v.downcast_ref::<T>()).cloned() } else { None }
    })
    .ok()
    .flatten();
    match hit {
        Some(v) => v,
        None => setting_slow(at, site, read),
    }
}

#[cold]
fn setting_slow<T: Clone + Send + Sync + 'static>(at: &'static std::panic::Location<'static>, site: usize, read: impl FnOnce() -> T) -> T {
    let Some(run) = run_own() else { return read() };
    let key = (at.file(), at.line(), at.column(), std::any::TypeId::of::<T>());
    let kept = run.values().settings.lock().get(&key).map(|v| v.downcast_ref::<T>().expect("a setting's type is its key's").clone());
    let v = match kept {
        Some(v) => v,
        None => {
            // Read outside the lock: a setting's read may read another.
            let v = read();
            run.values().settings.lock().entry(key).or_insert_with(|| std::sync::Arc::new(v.clone())).downcast_ref::<T>().expect("a setting's type is its key's").clone()
        }
    };
    let _ = SEEN.try_with(|seen| seen.borrow_mut().of(&run).settings.insert(site, Box::new(v.clone())));
    v
}

/// The run's own value of type `T` (a measurement's sums, a log's state; decision 0442), made the first time the
/// run asks for it. Outside a run, a value of its own that nothing keeps.
#[inline]
pub fn local<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    // This thread's copy of the run's locals (`SEEN`) first: no lock.
    let held = SEEN.try_with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.locals.get(&std::any::TypeId::of::<T>()).cloned() } else { None }
    })
    .ok()
    .flatten();
    match held {
        Some(v) => v.downcast::<T>().expect("a local's type is its key"),
        None => local_slow::<T>(),
    }
}

#[cold]
fn local_slow<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let Some(run) = run_own() else {
        // A test's own thread takes a value of its own, as a caller with no run does (`outside_a_run`).
        #[cfg(feature = "test-hooks")]
        if on_a_tests_thread() {
            return std::sync::Arc::new(T::default());
        }
        missed(std::any::type_name::<T>());
        return std::sync::Arc::new(T::default());
    };
    let v = std::sync::Arc::clone(run.values().locals.lock().entry(std::any::TypeId::of::<T>()).or_insert_with(|| std::sync::Arc::new(T::default())));
    let _ = SEEN.try_with(|seen| seen.borrow_mut().of(&run).locals.insert(std::any::TypeId::of::<T>(), v.clone()));
    v.downcast::<T>().expect("a local's type is its key")
}

/// `f` of the run's own value of type `T` (`local`), on a hot path: this thread's copy of the run's locals
/// (`SEEN`) answers with no lock and no count of references. Outside a run, `f` of a value of its own.
#[inline]
pub fn with_local<T: Default + Send + Sync + 'static, R>(f: impl FnOnce(&T) -> R) -> R {
    let held = SEEN.try_with(|seen| {
        let seen = seen.borrow();
        if !seen.is_current() {
            return None;
        }
        seen.locals.get(&std::any::TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>()).map(|t| t as *const T)
    })
    .ok()
    .flatten();
    match held {
        // SAFETY: `SEEN` holds the value (an `Arc` of the run's own, the run's `locals` another) while the run under
        // way on this thread is the one it was copied from; nothing `f` runs ends that run (a run nested in it joins it).
        Some(t) => f(unsafe { &*t }),
        None => with_local_slow(f),
    }
}

#[cold]
fn with_local_slow<T: Default + Send + Sync + 'static, R>(f: impl FnOnce(&T) -> R) -> R {
    f(&local::<T>())
}

/// The run's own value of type `T`, if the run under way made it (a report reads what the run wrote, and makes none).
pub fn local_if_any<T: Send + Sync + 'static>() -> Option<std::sync::Arc<T>> {
    let run = run_own()?;
    let v = run.values().locals.lock().get(&std::any::TypeId::of::<T>()).cloned()?;
    v.downcast::<T>().ok()
}

/// This thread's copy of what the run under way on it read (`setting`, `with_local`; decision 0442): the run's
/// own values, held with the run (weakly: a run ended is never taken for another).
#[derive(Default)]
struct Seen {
    run: Option<std::sync::Weak<dyn RunOwn>>,
    /// Settings by the reading place's address.
    settings: rustc_hash::FxHashMap<usize, Box<dyn std::any::Any>>,
    /// The run's locals by type.
    locals: rustc_hash::FxHashMap<std::any::TypeId, std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    /// The run's shared values by type (`shared`).
    shared: rustc_hash::FxHashMap<std::any::TypeId, std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    /// Its machine's values by type (`machine`).
    machine: rustc_hash::FxHashMap<std::any::TypeId, std::sync::Arc<dyn std::any::Any + Send + Sync>>,
}

impl Seen {
    /// Whether the copy is of the run under way on this thread.
    #[inline]
    fn is_current(&self) -> bool {
        RUN.try_with(|r| match (r.borrow().as_ref(), self.run.as_ref()) {
            (Some(now), Some(was)) => std::ptr::addr_eq(std::sync::Arc::as_ptr(now), was.as_ptr()),
            _ => false,
        })
        .unwrap_or(false)
    }

    /// The copy for `run`, emptied first if it was another's.
    fn of(&mut self, run: &std::sync::Arc<dyn RunOwn>) -> &mut Self {
        if !self.run.as_ref().is_some_and(|was| std::ptr::addr_eq(std::sync::Arc::as_ptr(run), was.as_ptr())) {
            *self = Seen { run: Some(std::sync::Arc::downgrade(run)), ..Default::default() };
        }
        self
    }
}

thread_local! {
    static SEEN: std::cell::RefCell<Seen> = std::cell::RefCell::new(Seen::default());
}


/// A run of its own for a test where none is under way:
/// the profile's records are a run's, so they are kept only inside one. Dropped, the thread's own is back.
#[cfg(any(test, feature = "test-hooks"))]
pub fn test_run() -> impl Drop {
    let fresh = run_own().is_none().then(|| enter(Some(std::sync::Arc::new(Bare(RunValues::default(), default_shared())))));
    struct Guard<D>(Option<D>);
    impl<D> Drop for Guard<D> {
        fn drop(&mut self) {
            drop(self.0.take());
        }
    }
    Guard(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine's main thread is recorded (`MainThread`), so `is_main_thread` is true there on every platform and
    /// a run's wait on it sleeps on its bell (`rerail_accel::exec`'s `in_a_job` false), while another thread of the
    /// machine's, in a run on it, is not its main thread (and helps the pool when it waits on its pool).
    #[test]
    fn the_machines_main_thread_sleeps_on_its_bell() {
        let machine = Machine::here();
        let shared = Shared::on(&machine);
        let run = |shared: std::sync::Arc<Shared>| enter(Some(std::sync::Arc::new(Bare(RunValues::default(), shared))));
        assert!(!is_main_thread(), "outside a run");
        let _run = run(shared.clone());
        assert!(is_main_thread(), "the thread `Machine::here` took in");
        let other = std::thread::spawn(move || {
            let _run = run(shared);
            is_main_thread()
        });
        assert!(!other.join().unwrap(), "another thread, in a run on the machine");
        let elsewhere = Shared::on(&Machine::new());
        let _other_machine = run(elsewhere);
        assert!(!is_main_thread(), "on a machine no main thread made");
    }

    /// A run's last handle dropped on a thread that holds no run (a pool thread whose job carried it): what the run
    /// and its context hold drops with the run's shared object on the thread, so a value whose drop reads it (a
    /// store's writers) finds it. Before, such a drop read outside any run and aborted the process (`wave0_…`, 2026-11-20).
    #[test]
    fn a_runs_last_handle_dropped_off_its_thread_drops_its_values_in_its_shared_object() {
        #[derive(Default)]
        struct Marker;
        struct ReadsShared(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for ReadsShared {
            fn drop(&mut self) {
                let _ = shared::<Marker>();
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let job = std::thread::spawn({
            let dropped = dropped.clone();
            move || {
                // A run of the engine's own thread, its context holding the value; the job's handle outlives both.
                engine_thread();
                let shared = std::sync::Arc::new(Shared::default());
                let _run = enter(Some(std::sync::Arc::new(Bare(RunValues::default(), shared))));
                let _bound = bind(ReadsShared(dropped));
                job_run()
            }
        })
        .join()
        .unwrap();
        std::thread::spawn(move || {
            engine_thread();
            assert!(run_own().is_none());
            drop(job);
        })
        .join()
        .expect("the run's values dropped in its shared object, not outside any run");
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// Q337: a job's captures end inside its run (`in_run`); with the run's guard in the job's body, a capture's drop
    /// comes after the guard's and is outside the run.
    #[test]
    fn a_jobs_captures_are_dropped_inside_its_run() {
        struct Says(std::sync::Arc<std::sync::Mutex<Option<bool>>>);
        impl Drop for Says {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = Some(run_own().is_some());
            }
        }
        let _t = test_run();
        let job = |seen: &std::sync::Arc<std::sync::Mutex<Option<bool>>>, carried: bool| {
            let (says, run) = (Says(seen.clone()), job_run());
            std::thread::spawn(move || {
                if carried {
                    in_run(run, move || {
                        let _ = &says;
                    });
                } else {
                    let _in = enter(run);
                    let _ = &says;
                }
            })
            .join()
            .unwrap();
        };
        let (bare, carried) = (std::sync::Arc::new(std::sync::Mutex::new(None)), std::sync::Arc::new(std::sync::Mutex::new(None)));
        job(&bare, false);
        assert_eq!(*bare.lock().unwrap(), Some(false), "the guard's pattern: the capture outlives the run");
        job(&carried, true);
        assert_eq!(*carried.lock().unwrap(), Some(true), "in_run: the capture ends inside the run");
    }

    #[derive(Clone, Default, PartialEq, Debug)]
    struct Depth(usize);
    #[derive(Clone, Default, PartialEq, Debug)]
    struct Site(&'static str);

    fn links() -> usize {
        CONTEXT.with(|c| {
            let (mut n, mut at) = (0, c.borrow().0.clone());
            while let Some(l) = at {
                n += 1;
                at = l.up.0.clone();
            }
            n
        })
    }

    #[test]
    fn a_binding_lasts_its_extent_and_the_nearest_wins() {
        assert_eq!(bound::<Depth>(), Depth(0), "unbound: the default");
        {
            let _one = bind(Depth(1));
            let _site = bind(Site("plan"));
            assert_eq!(bound::<Depth>(), Depth(1));
            {
                let _two = bind(Depth(2));
                assert_eq!((bound::<Depth>(), bound::<Site>()), (Depth(2), Site("plan")));
                assert_eq!(with_bound(|d: &Depth| d.0), 2);
            }
            assert_eq!(bound::<Depth>(), Depth(1));
        }
        assert_eq!((bound::<Depth>(), links()), (Depth(0), 0));
    }

    #[test]
    fn a_job_takes_its_callers_context_and_its_bindings_stay_its_own() {
        let _test = test_run();
        let _outer = bind(Depth(3));
        let job = job_run();
        let _later = bind(Depth(4));
        std::thread::spawn(move || {
            assert_eq!(bound::<Depth>(), Depth(0), "a thread of its own: no context");
            {
                let _in = enter(job.clone());
                assert_eq!(bound::<Depth>(), Depth(3), "the context the job was taken in");
                assert!(run_own().is_some_and(|r| r.with_context().is_none()), "the run, not the job's handle");
                assert!(holds(&job));
                let _own = bind(Depth(9));
                assert_eq!(bound::<Depth>(), Depth(9));
                assert!(!holds(&job));
            }
            assert_eq!(bound::<Depth>(), Depth(0), "the thread's own again");
        })
        .join()
        .unwrap();
        assert_eq!(bound::<Depth>(), Depth(4), "the caller's own bindings are untouched");
    }

    #[test]
    fn a_run_entered_alone_leaves_the_threads_context() {
        let _depth = bind(Depth(5));
        let run: std::sync::Arc<dyn RunOwn> = std::sync::Arc::new(Bare(RunValues::default(), Shared::new()));
        let _in = enter(Some(run));
        assert_eq!(bound::<Depth>(), Depth(5));
    }

    #[cfg(feature = "test-hooks")]
    #[test]
    #[should_panic(expected = "dropped out of order")]
    fn a_binding_dropped_out_of_order_is_refused() {
        let outer = bind(Depth(1));
        let _inner = bind(Site("in"));
        drop(outer);
    }

    #[test]
    fn a_choice_set_inside_a_guard_ends_with_it_and_keeps_the_guards_link() {
        let _outer = bind(Depth(1));
        {
            let inner = bind(Depth(2));
            bind_here(Depth(3));
            assert_eq!(bound::<Depth>(), Depth(3));
            drop(inner);
        }
        assert_eq!(bound::<Depth>(), Depth(1));
    }

    #[test]
    fn a_choice_set_again_is_one_binding_and_ends_with_its_extent() {
        {
            let _extent = bind(Site("outer"));
            for i in 0..100 {
                bind_here(Depth(i));
            }
            assert_eq!((bound::<Depth>(), links()), (Depth(99), 2));
        }
        assert_eq!((bound::<Depth>(), links()), (Depth(0), 0));
    }

    /// Whether this thread is one of `machine`'s pool's.
    fn on_pool_of(machine: &std::sync::Arc<Machine>) -> bool {
        machine.get::<Cores>().pool().current_thread_index().is_some()
    }

    #[test]
    fn a_callers_parallel_work_runs_on_its_own_machines_pool() {
        // Two machines in one process have two pools: each caller's work is on its own machine's, never the other's.
        let (a, b) = (Machine::new(), Machine::new());
        for (mine, other) in [(&a, &b), (&b, &a)] {
            let _in = within(Shared::on(mine));
            let seen = on_cores(|| (on_pool_of(mine), on_pool_of(other)));
            assert_eq!(seen, (true, false));
            // A loop inside it splits over the same pool.
            use rayon::prelude::*;
            let all = on_cores(|| (0..64).into_par_iter().all(|_| on_pool_of(mine) && !on_pool_of(other)));
            assert!(all, "every item on the caller's machine's pool");
        }
    }

    #[test]
    fn work_on_the_cores_is_in_its_callers_run_and_context() {
        let machine = Machine::new();
        let shared = Shared::on(&machine);
        let _in = within(std::sync::Arc::clone(&shared));
        let _depth = bind(Depth(5));
        let (depth, same) = on_cores(|| (bound::<Depth>(), shared_object().is_some_and(|s| std::sync::Arc::ptr_eq(&s, &shared))));
        assert_eq!((depth, same), (Depth(5), true));
        // A spawned job runs on the caller's machine's pool, and carries what it reads itself.
        let (tx, rx) = std::sync::mpsc::channel();
        let m = std::sync::Arc::clone(&machine);
        cores_spawn(move || tx.send(on_pool_of(&m)).unwrap());
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(10)), Ok(true));
        assert_eq!(cores_threads(), threads());
    }
}

#[cfg(test)]
mod stolen_halves {
    use super::*;

    /// Q300: two runs' loops in one pool at once (a corpus run's phase 0 beside its main program). Every item, the
    /// halves other threads steal included, reads its own run's value and its own caller's binding: each job carries
    /// its maker's run (rayon-core's patch, `carry`).
    #[test]
    fn two_runs_loops_in_one_pool_at_once_each_read_their_own_run() {
        use rayon::prelude::*;
        #[derive(Default)]
        struct Mark;
        #[derive(Clone, Default)]
        struct Tag(u32);
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let callers: Vec<_> = [1u32, 2]
            .into_iter()
            .map(|tag| {
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    let _run = test_run();
                    let _tag = bind(Tag(tag));
                    let mark = local::<Mark>();
                    start.wait();
                    let mut wrong = 0usize;
                    let mut threads = std::collections::HashSet::new();
                    // Several loops, so the two callers' loops are in the pool together.
                    for _ in 0..8 {
                        let seen: Vec<(bool, u32, std::thread::ThreadId)> = on_cores(|| {
                            (0..4096)
                                .into_par_iter()
                                .map(|_| {
                                    std::hint::black_box((0..200).sum::<u64>());
                                    (std::sync::Arc::ptr_eq(&local::<Mark>(), &mark), bound::<Tag>().0, std::thread::current().id())
                                })
                                .collect()
                        });
                        wrong += seen.iter().filter(|&&(same, t, _)| !same || t != tag).count();
                        threads.extend(seen.iter().map(|s| s.2));
                    }
                    (wrong, threads.len())
                })
            })
            .collect();
        for caller in callers {
            let (wrong, threads) = caller.join().expect("a caller's thread");
            assert_eq!(wrong, 0, "items read another run's value or lost their caller's binding");
            assert!(threads > 1 || super::threads() == 1, "a caller's loops ran on one thread: nothing was stolen");
        }
    }

    /// A loop's items split over the pool, the halves other threads steal included, each read the caller's run and
    /// its bindings: `local` the run's one value, `bound` the caller's binding. Then the same where the loop starts
    /// on one of the pool's own threads holding a run (a program's main thread is its machine's thread 0).
    #[test]
    fn a_loops_stolen_halves_are_in_its_run() {
        use rayon::prelude::*;
        #[derive(Default)]
        struct Mark;
        #[derive(Clone, Default)]
        struct Tag(u32);
        let _run = test_run();
        let _tag = bind(Tag(7));
        let mark = local::<Mark>();
        let seen: Vec<(bool, u32, std::thread::ThreadId)> = on_cores(|| {
            (0..4096)
                .into_par_iter()
                .map(|_| {
                    std::hint::black_box((0..200).sum::<u64>());
                    (std::sync::Arc::ptr_eq(&local::<Mark>(), &mark), bound::<Tag>().0, std::thread::current().id())
                })
                .collect()
        });
        assert!(seen.iter().all(|&(same, tag, _)| same && tag == 7), "an item read another run's value or lost the binding");
        let threads: std::collections::HashSet<_> = seen.iter().map(|s| s.2).collect();
        assert!(threads.len() > 1 || super::threads() == 1, "the loop ran on one thread: nothing was stolen");
        let job = job_run();
        let mark = std::sync::Arc::clone(&mark);
        let all = machine::<Cores>().pool().install(move || {
            let _in = enter(job);
            on_cores(|| (0..4096).into_par_iter().map(|_| std::sync::Arc::ptr_eq(&local::<Mark>(), &mark)).collect::<Vec<_>>())
        });
        assert!(all.iter().all(|&same| same), "a loop started on a pool thread lost its run");
    }
}
