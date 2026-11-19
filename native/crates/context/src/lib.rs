//! The run under way, and what its caller shares by design (decisions 0441, 0442): the bottom of every crate that
//! holds a run's state, so that a crate below the engine's logic (the job pool, the device, the compiler, the blocks)
//! keeps its run data in the run, and its shared and machine state in the objects the run's caller passed, never in
//! a static.
//!
//! - `RUN`: the run under way on this thread, made and carried by `rerail_logic::run` (and `run_state`, to its jobs);
//! - `setting`, `local`, `with_local`, `local_if_any`: a run's settings and its own values, by type;
//! - `Shared` (`shared`): what a caller shares by design with the runs it hands the object;
//! - `Machine` (`machine`): the machine's resources, which every `Shared` is made on.
//!
//! Outside a run, each gives a value of its own that nothing keeps.

thread_local! {
    /// The run under way on this thread (decision 0441): its own state, `rerail_logic::run::Run`, made and carried by
    /// the crate that defines it, held here so that every crate below it reaches it too.
    pub static RUN: std::cell::RefCell<Option<std::sync::Arc<dyn RunOwn>>> = const { std::cell::RefCell::new(None) };
}

/// What the crates below `logic` tell the run under way, and read of it (`RUN`; decisions 0441, 0442).
pub trait RunOwn: Send + Sync {
    /// A tree decoded from a program's version, at `at`.
    fn decoded(&self, at: &'static std::panic::Location<'static>);
    /// Its settings and its own values (`setting`, `local`).
    fn values(&self) -> &RunValues;
    /// What its caller shares by design with the other runs it hands the same object (`shared`).
    fn shared(&self) -> &std::sync::Arc<Shared>;
    /// The run itself, for the crate that defines it.
    fn any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync>;
}

/// The run under way on this thread (`RUN`), if any.
pub fn run_own() -> Option<std::sync::Arc<dyn RunOwn>> {
    RUN.with(|r| r.borrow().clone())
}

pub type AnyArc = std::sync::Arc<dyn std::any::Any + Send + Sync>;

/// A run's settings, by the place that reads each, and its own values, by type (decision 0442).
#[derive(Default)]
pub struct RunValues {
    settings: parking_lot::Mutex<std::collections::HashMap<(&'static str, u32, u32, std::any::TypeId), AnyArc>>,
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

/// The shared object a read outside any run takes. A product build's entry points, and every thread the engine
/// makes, hold a run or a shared object (`enter`, `within`), so such a read is a miss: it takes one of its own, which
/// nothing keeps, and `RERAIL_STRICT_RUN=1` makes it a panic (for a check that none happens). A test build takes the
/// tests' one.
#[cold]
fn outside_a_run() -> std::sync::Arc<Shared> {
    #[cfg(feature = "test-hooks")]
    return std::sync::Arc::clone(&TESTS);
    #[cfg(not(feature = "test-hooks"))]
    {
        if std::env::var_os("RERAIL_STRICT_RUN").is_some_and(|v| v == "1") {
            panic!("a shared or machine value read outside any run (RERAIL_STRICT_RUN=1): its thread holds no run");
        }
        Shared::new()
    }
}

/// The tests' shared object (`default_shared`, `outside_a_run`): test builds only.
#[cfg(feature = "test-hooks")]
static TESTS: std::sync::LazyLock<std::sync::Arc<Shared>> = std::sync::LazyLock::new(Shared::new);

/// The value of type `T` that the run under way shares by design (`Shared`), made the first time it is asked for.
/// Outside a run: `outside_a_run`'s.
#[inline]
pub fn shared<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let held = SEEN.with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.shared.get(&std::any::TypeId::of::<T>()).cloned() } else { None }
    });
    match held {
        Some(v) => v.downcast::<T>().expect("a shared value's type is its key"),
        None => shared_slow::<T>(),
    }
}

#[cold]
fn shared_slow<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let Some(run) = run_own() else { return outside_a_run().get::<T>() };
    let v = run.shared().get::<T>();
    SEEN.with(|seen| seen.borrow_mut().of(&run).shared.insert(std::any::TypeId::of::<T>(), v.clone()));
    v
}

/// The value of type `T` of the machine the run under way runs on (`Machine`), made the first time it is asked for.
/// Outside a run: `outside_a_run`'s machine's.
#[inline]
pub fn machine<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let held = SEEN.with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.machine.get(&std::any::TypeId::of::<T>()).cloned() } else { None }
    });
    match held {
        Some(v) => v.downcast::<T>().expect("a machine value's type is its key"),
        None => machine_slow::<T>(),
    }
}

#[cold]
fn machine_slow<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let Some(run) = run_own() else { return outside_a_run().machine().get::<T>() };
    let v = run.shared().machine().get::<T>();
    SEEN.with(|seen| seen.borrow_mut().of(&run).machine.insert(std::any::TypeId::of::<T>(), v.clone()));
    v
}

/// The shared object of the run under way, if one is (for a caller that hands it on).
pub fn shared_object() -> Option<std::sync::Arc<Shared>> {
    run_own().map(|r| std::sync::Arc::clone(r.shared()))
}

/// The shared object a thread reads by: its run's, else `outside_a_run`'s (for a thread that hands it on: a service
/// it starts, a machine's own thread).
pub fn shared_here() -> std::sync::Arc<Shared> {
    shared_object().unwrap_or_else(outside_a_run)
}

/// Puts `run` on this thread (a job of it, on another thread) until the guard drops, then what the thread had.
pub fn enter(run: Option<std::sync::Arc<dyn RunOwn>>) -> impl Drop {
    struct Back(Option<Option<std::sync::Arc<dyn RunOwn>>>);
    impl Drop for Back {
        fn drop(&mut self) {
            let was = self.0.take().flatten();
            RUN.with(|r| *r.borrow_mut() = was);
        }
    }
    Back(Some(RUN.with(|r| r.replace(run))))
}

/// Puts a run of its own on this thread, on `shared`, until the guard drops: for a thread that outlives the run that
/// started it (a service, a machine's own thread), so that it reads its caller's shared and machine values and keeps
/// no run alive.
pub fn within(shared: std::sync::Arc<Shared>) -> impl Drop {
    enter(Some(std::sync::Arc::new(Bare(RunValues::default(), shared))))
}

/// A run with nothing of its own but its values and its shared object (`within`, `test_run`).
struct Bare(RunValues, std::sync::Arc<Shared>);

impl RunOwn for Bare {
    fn decoded(&self, _: &'static std::panic::Location<'static>) {}
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
    let hit = SEEN.with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.settings.get(&site).and_then(|v| v.downcast_ref::<T>()).cloned() } else { None }
    });
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
    SEEN.with(|seen| seen.borrow_mut().of(&run).settings.insert(site, Box::new(v.clone())));
    v
}

/// The run's own value of type `T` (a measurement's sums, a log's state; decision 0442), made the first time the
/// run asks for it. Outside a run, a value of its own that nothing keeps.
#[inline]
pub fn local<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    // This thread's copy of the run's locals (`SEEN`) first: no lock.
    let held = SEEN.with(|seen| {
        let seen = seen.borrow();
        if seen.is_current() { seen.locals.get(&std::any::TypeId::of::<T>()).cloned() } else { None }
    });
    match held {
        Some(v) => v.downcast::<T>().expect("a local's type is its key"),
        None => local_slow::<T>(),
    }
}

#[cold]
fn local_slow<T: Default + Send + Sync + 'static>() -> std::sync::Arc<T> {
    let Some(run) = run_own() else { return std::sync::Arc::new(T::default()) };
    let v = std::sync::Arc::clone(run.values().locals.lock().entry(std::any::TypeId::of::<T>()).or_insert_with(|| std::sync::Arc::new(T::default())));
    SEEN.with(|seen| seen.borrow_mut().of(&run).locals.insert(std::any::TypeId::of::<T>(), v.clone()));
    v.downcast::<T>().expect("a local's type is its key")
}

/// `f` of the run's own value of type `T` (`local`), on a hot path: this thread's copy of the run's locals
/// (`SEEN`) answers with no lock and no count of references. Outside a run, `f` of a value of its own.
#[inline]
pub fn with_local<T: Default + Send + Sync + 'static, R>(f: impl FnOnce(&T) -> R) -> R {
    let held = SEEN.with(|seen| {
        let seen = seen.borrow();
        if !seen.is_current() {
            return None;
        }
        seen.locals.get(&std::any::TypeId::of::<T>()).and_then(|v| v.downcast_ref::<T>()).map(|t| t as *const T)
    });
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
        RUN.with(|r| match (r.borrow().as_ref(), self.run.as_ref()) {
            (Some(now), Some(was)) => std::ptr::addr_eq(std::sync::Arc::as_ptr(now), was.as_ptr()),
            _ => false,
        })
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
