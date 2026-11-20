//! The run under way (decision 0441): what a run keeps of its own while it runs and returns with its result, so the
//! engine holds no such state for the process. Two runs, one after another, one inside another or on two threads at
//! once, each see their own.
//!
//! The outermost run on a thread makes its `Run` (`scope`: a command, a corpus run or a program run, whichever starts
//! first); a run nested in it joins it; `run_state` carries it to the run's jobs on other threads. Its outermost scope
//! returns its `Record` with the run's result (`plan::Outcome::run`, `corpus::Report::run`, `tools::cli_in`).
//!
//! It holds:
//! - its limits and what it met of them (`width`, decision 0462);
//! - its settings (`setting`: each read once by the run, as it first needs it; decision 0442) and its own values by
//!   type (`local`: a measurement's sums, a log, a batch), where the process kept them in statics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use super::native::width;

/// A run's own state.
pub struct Run {
    pub width: width::Width,
    /// Set as the run ends: what watches it (its memory watch) ends too.
    ended: AtomicBool,
    /// Its settings and its own values (`setting`, `local`).
    values: super::profile::RunValues,
    /// What its caller shares by design (`profile::Shared`, decision 0442).
    shared: Arc<super::profile::Shared>,
}

impl super::profile::RunOwn for Run {
    fn values(&self) -> &super::profile::RunValues {
        &self.values
    }
    fn shared(&self) -> &Arc<super::profile::Shared> {
        &self.shared
    }
    fn any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

impl Run {
    fn new(settings: width::Settings, shared: Arc<super::profile::Shared>) -> Run {
        Run { width: width::Width::new(settings), ended: AtomicBool::new(false), values: Default::default(), shared }
    }
    /// Whether the run has ended.
    pub fn ended(&self) -> bool {
        self.ended.load(Relaxed)
    }
    /// What the run met and did, so far.
    pub fn record(&self) -> Record {
        Record { width: self.width.record() }
    }
}

pub use super::profile::{Machine, Shared, local, local_if_any, machine, setting, shared, shared_object, with_local};

/// What a run returns of its own state with its result.
#[derive(Clone, Debug, Default)]
pub struct Record {
    /// Its limits and what it met of them: the relations cut at the cap, the widest it held.
    pub width: width::Record,
}

/// Carries the run to a job on another thread (`run_state`). Its thread-local is `profile::RUN`, so the crates below
/// this one reach it (`profile::RunOwn`).
/// A caller with no run (a test's thread) hands its jobs a run of its own on its object (`profile::job_run`).
pub(in crate::code_rule_dsl) fn carried(c: &mut super::run_state::Capture) {
    c.part(&["RUN", "CONTEXT"], super::run_state::Shared(super::profile::job_run(), super::profile::enter));
}

/// The run under way on this thread, if one holds a scope.
pub fn current() -> Option<Arc<Run>> {
    super::profile::run_own().and_then(|r| r.any().downcast::<Run>().ok())
}

/// A run's hold on its state. The outermost scope on a thread makes it (and starts its memory watch); a nested one
/// joins it. Dropped, the outermost ends it.
pub struct Scope {
    run: Arc<Run>,
    outer: bool,
    /// The run under way on the thread before the outermost scope (a test's own run, `profile::test_run`), put back.
    was: Option<Arc<dyn super::profile::RunOwn>>,
}

/// The scope of a run starting here: the run's own if one is under way on this thread, else a new run with the limits
/// `settings` gives and a shared object of its own (`scope_in`: one its caller hands it).
pub fn scope(settings: impl FnOnce() -> width::Settings) -> Scope {
    scope_in(None, settings)
}

/// `scope`, a new run sharing by design what `shared` holds with every other run handed it (decision 0442). A run
/// handed none (`None`) has the object its thread holds (`profile::within`: a workspace's task, a service), else
/// `profile::default_shared`'s: its own in a product build, the tests' in a test build. A nested scope joins the run
/// under way, whose object it is.
pub fn scope_in(shared: Option<&Arc<Shared>>, settings: impl FnOnce() -> width::Settings) -> Scope {
    if let Some(run) = current() {
        return Scope { run, outer: false, was: None };
    }
    let shared = shared.map_or_else(|| shared_object().unwrap_or_else(super::profile::default_shared), Arc::clone);
    let run = Arc::new(Run::new(settings(), shared));
    width::watch(&run);
    let was = super::profile::RUN.with(|r| r.replace(Some(Arc::clone(&run) as Arc<dyn super::profile::RunOwn>)));
    Scope { run, outer: true, was }
}

impl Scope {
    pub fn run(&self) -> &Arc<Run> {
        &self.run
    }
    /// The run's record, from its outermost scope; `None` from a nested one (its record is the outer run's).
    pub fn finish(self) -> Option<Record> {
        self.outer.then(|| self.run.record())
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.outer {
            self.run.ended.store(true, Relaxed);
            let was = self.was.take();
            super::profile::RUN.with(|r| *r.borrow_mut() = was);
            super::profile::run_ended();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cutting(cap: usize) -> impl FnOnce() -> width::Settings {
        move || width::Settings { cap, cut: true, ceiling: None }
    }

    /// Runs one after another and one inside another: each outermost run has a record of its own, and a nested run
    /// shares the outer run's.
    #[test]
    fn each_run_has_its_own_record() {
        let a = scope(cutting(10));
        a.run().width.note_cut_by("x", "x(1).".into());
        let nested = scope(|| unreachable!("a nested run takes the outer run's settings"));
        assert_eq!(nested.run().width.cap(), 10);
        nested.run().width.note_cut_by("y", "y(1).".into());
        assert!(nested.finish().is_none(), "a nested run's record is the outer run's");
        let record = a.finish().expect("the outermost run's record");
        assert_eq!(record.width.cuts.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["x", "y"]);
        assert!(record.width.incomplete().is_some_and(|m| m.contains("`x`") && m.contains("derived by x(1).")));
        assert!(current().is_none(), "the run ended with its scope");
        let b = scope(cutting(10));
        let later = b.finish().expect("a new run");
        assert!(later.width.cuts.is_empty(), "a later run starts with nothing of the last");
        assert!(!width::Limits::here().cutting(), "a walk with no run stops at the cap");
    }

    /// Decision 0442: what is shared by design reaches only the runs handed the same object. Two runs handed one see
    /// one value; a run handed another sees its own; a nested run sees its outer run's.
    #[test]
    fn a_shared_value_reaches_only_the_runs_handed_its_object() {
        #[derive(Default)]
        struct Model(parking_lot::Mutex<u64>);
        let one = Shared::new();
        {
            let _a = scope_in(Some(&one), cutting(10));
            *shared::<Model>().0.lock() += 5;
            let _nested = scope_in(None, || unreachable!("a nested run joins the outer"));
            assert_eq!(*shared::<Model>().0.lock(), 5, "a nested run shares its outer run's object");
        }
        {
            let _b = scope_in(Some(&one), cutting(10));
            assert_eq!(*shared::<Model>().0.lock(), 5, "a later run handed the same object sees its value");
        }
        {
            let _c = scope_in(Some(&Shared::new()), cutting(10));
            assert_eq!(*shared::<Model>().0.lock(), 0, "a run handed another object sees none of it");
        }
        {
            // A run handed none, on a thread that holds an object (a workspace's task): that object's.
            let _held = super::super::profile::within(Arc::clone(&one));
            let _d = scope_in(None, cutting(10));
            assert_eq!(*shared::<Model>().0.lock(), 5, "a run handed none takes its thread's object");
        }
        // On another thread at once, handed another object: apart.
        let other = std::thread::spawn(|| {
            let _e = scope_in(Some(&Shared::new()), cutting(10));
            *shared::<Model>().0.lock()
        });
        let _f = scope_in(Some(&one), cutting(10));
        assert_eq!((other.join().unwrap(), *shared::<Model>().0.lock()), (0, 5));
    }

    /// Decision 0442: two callers with different `Shared` objects on one `Machine` share the machine's values, not
    /// each other's shared ones.
    #[test]
    fn shared_objects_on_one_machine_share_only_the_machine() {
        #[derive(Default)]
        struct Pool(parking_lot::Mutex<u64>);
        #[derive(Default)]
        struct Names(parking_lot::Mutex<u64>);
        let machine = Machine::new();
        let (a, b) = (Shared::on(&machine), Shared::on(&machine));
        {
            let _a = scope_in(Some(&a), cutting(10));
            *super::machine::<Pool>().0.lock() += 1;
            *shared::<Names>().0.lock() += 1;
        }
        let _b = scope_in(Some(&b), cutting(10));
        assert_eq!(*super::machine::<Pool>().0.lock(), 1, "one machine");
        assert_eq!(*shared::<Names>().0.lock(), 0, "another caller's shared object");
    }

    /// A setting is read once a run, and again by the next run; a local is the run's own.
    #[test]
    fn a_runs_settings_and_locals_are_its_own() {
        let read = std::sync::atomic::AtomicUsize::new(0);
        let at = || setting(|| read.fetch_add(1, Relaxed));
        let a = scope(cutting(10));
        assert_eq!((at(), at()), (0, 0), "read once a run");
        #[derive(Default)]
        struct Sum(parking_lot::Mutex<u64>);
        *local::<Sum>().0.lock() += 2;
        *local::<Sum>().0.lock() += 3;
        assert_eq!(*local_if_any::<Sum>().unwrap().0.lock(), 5);
        drop(a);
        let _b = scope(cutting(10));
        assert_eq!(at(), 1, "the next run reads it again");
        assert!(local_if_any::<Sum>().is_none(), "the next run has none of the last run's");
    }

    /// Runs on two threads at once keep apart.
    #[test]
    fn runs_on_two_threads_keep_apart() {
        let (mine, theirs) = rerail_accel::exec::scoped(|s| {
            let other = s.spawn(|| {
                let run = scope(cutting(5));
                run.run().width.note_cut_by("theirs", "t.".into());
                run.finish().unwrap()
            });
            let run = scope(cutting(7));
            run.run().width.note_cut_by("mine", "m.".into());
            (run.finish().unwrap(), other.join().unwrap())
        });
        assert_eq!((mine.width.cap, mine.width.cuts.len(), mine.width.cuts[0].0.as_str()), (7, 1, "mine"));
        assert_eq!((theirs.width.cap, theirs.width.cuts.len(), theirs.width.cuts[0].0.as_str()), (5, 1, "theirs"));
    }
}
