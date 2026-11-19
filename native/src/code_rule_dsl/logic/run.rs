//! The run under way (decision 0441): what a run keeps of its own while it runs and returns with its result, so the
//! engine holds no such state for the process. Two runs, one after another, one inside another or on two threads at
//! once, each see their own.
//!
//! The outermost run on a thread makes its `Run` (`scope`: a command, a corpus run or a program run, whichever starts
//! first); a run nested in it joins it; `run_state` carries it to the run's jobs on other threads. Its outermost scope
//! returns its `Record` with the run's result (`plan::Outcome::run`, `corpus::Report::run`, `tools::cli_in`).
//!
//! It holds:
//! - its limits and what it met of them (`width`, decision 0394);
//! - the trees it decoded from versions (`Version::program`), for the check that a maintained run decodes none;
//! - its settings (`setting`: each read once by the run, as it first needs it; decision 0442) and its own values by
//!   type (`local`: a measurement's sums, a log, a batch), where the process kept them in statics.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use super::native::width;

/// A run's own state.
pub struct Run {
    pub width: width::Width,
    /// The trees decoded from versions: a count, and the place of the last.
    decoded: parking_lot::Mutex<(usize, Option<&'static std::panic::Location<'static>>)>,
    /// Set as the run ends: what watches it (its memory watch) ends too.
    ended: AtomicBool,
    /// Its settings and its own values (`setting`, `local`).
    values: super::profile::RunValues,
}

impl super::profile::RunOwn for Run {
    fn decoded(&self, at: &'static std::panic::Location<'static>) {
        let mut d = self.decoded.lock();
        *d = (d.0 + 1, Some(at));
    }
    fn values(&self) -> &super::profile::RunValues {
        &self.values
    }
    fn any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

impl Run {
    fn new(settings: width::Settings) -> Run {
        Run { width: width::Width::new(settings), decoded: Default::default(), ended: AtomicBool::new(false), values: Default::default() }
    }
    /// Whether the run has ended.
    pub fn ended(&self) -> bool {
        self.ended.load(Relaxed)
    }
    /// What the run met and did, so far.
    pub fn record(&self) -> Record {
        let (n, at) = *self.decoded.lock();
        Record { width: self.width.record(), trees_decoded: (n, at.map(|l| format!("{}:{}", l.file(), l.line()))) }
    }
}

pub use super::profile::{local, local_if_any, setting, with_local};

/// What a run returns of its own state with its result.
#[derive(Clone, Debug, Default)]
pub struct Record {
    /// Its limits and what it met of them: the relations cut at the cap, the widest it held.
    pub width: width::Record,
    /// The trees it decoded from versions, and where the last was (`file:line`).
    pub trees_decoded: (usize, Option<String>),
}

/// Carries the run to a job on another thread (`run_state`). Its thread-local is `profile::RUN`, so the crates below
/// this one reach it (`profile::RunOwn`).
pub(in crate::code_rule_dsl) fn carried(c: &mut super::run_state::Capture) {
    c.cloned(&["RUN"], &super::profile::RUN);
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
/// `settings` gives.
pub fn scope(settings: impl FnOnce() -> width::Settings) -> Scope {
    if let Some(run) = current() {
        return Scope { run, outer: false, was: None };
    }
    let run = Arc::new(Run::new(settings()));
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
        super::super::profile::RunOwn::decoded(&**a.run(), std::panic::Location::caller());
        let nested = scope(|| unreachable!("a nested run takes the outer run's settings"));
        assert_eq!(nested.run().width.cap(), 10);
        nested.run().width.note_cut_by("y", "y(1).".into());
        assert!(nested.finish().is_none(), "a nested run's record is the outer run's");
        let record = a.finish().expect("the outermost run's record");
        assert_eq!(record.width.cuts.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["x", "y"]);
        assert!(record.width.incomplete().is_some_and(|m| m.contains("`x`") && m.contains("derived by x(1).")));
        assert_eq!(record.trees_decoded.0, 1);
        assert!(current().is_none(), "the run ended with its scope");
        let b = scope(cutting(10));
        let later = b.finish().expect("a new run");
        assert!(later.width.cuts.is_empty() && later.trees_decoded.0 == 0, "a later run starts with nothing of the last");
        assert!(!width::Limits::here().cutting(), "a walk with no run stops at the cap");
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
