//! What a run keeps on its thread, and what a job on another thread takes from it (O-run-state-threads, decision
//! 0192).
//!
//! The engine keeps a run's context in thread-locals: how deep the run is nested, the phases under way, the kept
//! plans, the profile, the device binding. A job that runs part of the run on another thread (`threads::Jobs::spawn`,
//! `threads::beside`) must see the same context, or a nested run there would act as an outermost one: take the
//! session's switches again, see no phase under way, plan without the run's kept plans.
//!
//! Every thread-local of the engine's source (`native/src`, `native/crates/*/src`) is listed in [`STATE`] with its
//! class:
//!
//! - **Carried**: the run's context. [`Carry::here`] takes it on the caller's thread and [`Carry::enter`] puts it
//!   on a job's thread while the job runs, then puts back what that thread had (a pool thread may take a job of
//!   another run while it waits). Shared state (the kept plans, the observations, the derived values, the profile) is
//!   shared through its `Arc`, and what a job adds is merged into it as the job ends. Settings are copied.
//! - **Thread**: correct on whichever thread reads it: caches whose every entry is the same on any thread, counts
//!   of a thread's own work, a thread's own connections, codecs, device queue and open spans.
//! - **Call**: the state of one call in progress, set and read within it on the thread that runs it. A call that
//!   hands its work to another thread must carry it, and the state moves to **Carried**.
//! - **Outermost**: read and written only by a thread's outermost run (a session's state between its runs). A nested
//!   run, on any thread, never touches it.
//! - **NotYet**: the run's context, not carried yet. A job does not see it. The list is pinned and may only shrink.
//!
//! Thread-locals declared for tests alone (under `#[cfg(test)]` or `#[cfg(any(test, feature = "test-hooks"))]`, or
//! in a test file) are left out: a test sets them on its own thread.
//!
//! The tests below read the source: every thread-local is listed, every listed one exists, and every **Carried**
//! one is taken by [`Carry::here`]. `scripts/check/check-thread-state.py` refuses a commit that adds an unlisted
//! thread-local, before it is made.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::thread::LocalKey;

/// How a job on another thread treats a thread-local ([module docs](self)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Class {
    Carried,
    Thread,
    Call,
    Outermost,
    NotYet,
}
use Class::*;

/// Every thread-local outside the tests: `(file under native/, name, class, why)` (Carried ones need no why: they
/// are carried).
pub(crate) const STATE: &[(&str, &str, Class, &str)] = &[
    ("crates/compiler/src/clif_rust.rs", "ADDRESSES", Thread, "this thread's scratch or counters in a backend"),
    ("crates/compiler/src/native_cpu_stream.rs", "IDS", Thread, "this thread's scratch or counters in a backend"),
    ("crates/metal/src/buffer_observation.rs", "ACTIVE", Thread, "this thread's scratch or counters in a backend"),
    ("crates/metal/src/launch.rs", "EVALUATION", Thread, "this thread's scratch or counters in a backend"),
    ("crates/metal/src/test_pool.rs", "LEDGERS", Thread, "this thread's share of the test pool"),
    ("crates/prof/src/lib.rs", "LEFT", Thread, "this thread's sampling countdown"),
    ("crates/prof/src/lib.rs", "SPAN", Thread, "this thread's sampled span"),
    ("crates/tape_format/src/lib.rs", "IDS", Thread, "this thread's scratch or counters in a backend"),
    ("src/code_rule_dsl/budget.rs", "BUDGET_COUNTDOWN", Call, "the state of one call in progress on this thread"),
    ("src/code_rule_dsl/budget.rs", "MATCH_BUDGET", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/budget.rs", "STEP_COUNTDOWN", Call, "the state of one call in progress on this thread"),
    ("src/code_rule_dsl/calls/mod.rs", "EVALUATION", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/calls/mod.rs", "FILE_CALLS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/calls/rust.rs", "ITEMS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/demand/driver/catalog.rs", "CONNECTION", Thread, "this thread's query connection"),
    ("src/code_rule_dsl/eval/query.rs", "EVALUATION_TAPE", Call, "the tape of the evaluation under way on this thread"),
    ("src/code_rule_dsl/facts/d_projection.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/facts/facts_a/facts.rs", "KIND_CACHE", Thread, "kind codes by name: any thread's gives the same"),
    ("src/code_rule_dsl/facts/facts_a/facts.rs", "NAME_CACHE", Thread, "kind names by code: any thread's gives the same"),
    ("src/code_rule_dsl/facts/vtab/demand.rs", "FRAMES", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/facts/vtab/lookup_spike.rs", "WORK", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/facts/vtab/syntax.rs", "DOC", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/facts/vtab/syntax.rs", "KEPT", Thread, "dialects by grammar: any thread's gives the same"),
    ("src/code_rule_dsl/facts/vtab/syntax.rs", "LOCATED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/import_target/mod.rs", "VIEW", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/logic/derived.rs", "MEMO", Carried, ""),
    ("src/code_rule_dsl/logic/derived.rs", "SESSION_MEMO", Outermost, "a session's derived values between its runs: taken and put back by the outermost run"),
    ("src/code_rule_dsl/logic/lanes.rs", "DEFERRED", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/lanes.rs", "DONE", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/lanes.rs", "FLUSHED", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/lanes.rs", "LEVEL", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/lanes.rs", "NEXT", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/lanes.rs", "QUEUED", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/lanes.rs", "SUSPENDABLE", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("src/code_rule_dsl/logic/native/setfix.rs", "GROWN", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("src/code_rule_dsl/logic/native/setfix.rs", "MIXED", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("src/code_rule_dsl/logic/native/setfix.rs", "VARIANTS", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("src/code_rule_dsl/logic/native/setfix/closure.rs", "SEMI_NAIVE_HERE", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "ARENA", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "AUTOMATA", Thread, "device automata by pattern: any thread's gives the same"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "BINDER", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "FLUSHED", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "GROWTH", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "LAST", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "RESIDENT", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "SUSPENDED", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "TESTS", Thread, "device text tests by pattern: any thread's gives the same"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "TICKETS", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "WAVE", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/native/setfix/device.rs", "WAVES", Thread, "this thread's device waves and launches"),
    ("src/code_rule_dsl/logic/plan.rs", "EXPLAIN_ALWAYS", Carried, ""),
    ("src/code_rule_dsl/logic/plan.rs", "NO_SHAPES", Carried, ""),
    ("src/code_rule_dsl/logic/plan.rs", "OWN_PLANS", Carried, ""),
    ("src/code_rule_dsl/logic/plan.rs", "PLANNED_NOW", NotYet, "the profile's account of the keys planned in the run: a job's would be merged back into its caller's"),
    ("src/code_rule_dsl/logic/plan.rs", "PLANS", Carried, ""),
    ("src/code_rule_dsl/logic/plan.rs", "PLAN_SITE", Carried, ""),
    ("src/code_rule_dsl/logic/plan.rs", "SEED", Call, "the last priced body's cheapest start, read by the same pricing"),
    ("src/code_rule_dsl/logic/plan.rs", "SESSION_PLANS", Outermost, "a session's plans between its runs: taken and put back by the outermost run"),
    ("src/code_rule_dsl/logic/plan.rs", "SKEWING", Carried, ""),
    ("src/code_rule_dsl/logic/plan.rs", "STEP_ROWS", Carried, ""),
    ("src/code_rule_dsl/logic/plan/choices.rs", "DURABLE", NotYet, "the durable side of the run (its store and its entries), held by the thread that opened it"),
    ("src/code_rule_dsl/logic/plan/compose.rs", "CACHE", Thread, "compiled compositions by digest: any thread's gives the same"),
    ("src/code_rule_dsl/logic/plan/corpus.rs", "PREPARED", Thread, "prepared programs by key: any thread's gives the same program"),
    ("src/code_rule_dsl/logic/plan/corpus.rs", "RUNS_UNDER_WAY", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus.rs", "RUN_TOKEN", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus/phases.rs", "HANDED_DOWN", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus/phases.rs", "PHASE_ONE", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus/phases.rs", "RUNNING", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus/switches.rs", "DEPTH", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus/test_store.rs", "HANDED", Carried, ""),
    ("src/code_rule_dsl/logic/plan/corpus/test_store.rs", "OWN", Carried, ""),
    ("src/code_rule_dsl/logic/plan/demand.rs", "WHOLE", Carried, ""),
    ("src/code_rule_dsl/logic/plan/keeps.rs", "OBSERVED_AT", Carried, ""),
    ("src/code_rule_dsl/logic/plan/keeps.rs", "OWN_OBSERVED", Carried, ""),
    ("src/code_rule_dsl/logic/plan/placement.rs", "DEVICE", Carried, ""),
    ("src/code_rule_dsl/logic/profile.rs", "BEHIND", Thread, "marks the store's writer thread"),
    ("src/code_rule_dsl/logic/profile.rs", "HELD", Thread, "whether this thread holds the profile's lock"),
    ("src/code_rule_dsl/logic/profile.rs", "STAGE", Carried, ""),
    ("src/code_rule_dsl/logic/profile.rs", "STACK", Thread, "this thread's open spans; a job opens its span under its caller's (`profile::here`, carried)"),
    ("src/code_rule_dsl/logic/rr.rs", "READS", Call, "the libraries read while `recording` runs; a job records its own and hands them back (`rr::recorded_on_worker`)"),
    ("src/code_rule_dsl/logic/strings/functions.rs", "CACHE", Thread, "compiled regexes by pattern: any thread's gives the same"),
    ("src/code_rule_dsl/matcher/binding.rs", "FOLLOWING", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/d_anchors/sql/corpus.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/each.rs", "ACTIVE", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/each.rs", "EXCLUDED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/each.rs", "GENERATION", Call, "the state of one call in progress on this thread"),
    ("src/code_rule_dsl/matcher/each.rs", "HOSTS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/embedded/mod.rs", "HOOKS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/memo.rs", "MEMO", Call, "the matches of the file being matched on this thread"),
    ("src/code_rule_dsl/matcher/mod.rs", "CLAUSES_BEGUN", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/mod.rs", "CLAUSE_STATE_READS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/mod.rs", "FILL_STEPS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/mod.rs", "UTIL_PEAK", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/p_anchors/sql/corpus.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/parse.rs", "HOOKS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/resolve.rs", "CURRENT_CLAUSE", Call, "the state of one call in progress on this thread"),
    ("src/code_rule_dsl/matcher/resolve.rs", "FILE_SCOPES", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/resolve.rs", "JSX_RUNTIME_DERIVATIONS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/resolve.rs", "SOURCE_TYPE_DERIVATIONS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/resolve.rs", "SUB_TABLES", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/resolve.rs", "TABLE_BUILDS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/util.rs", "THREE_VALUED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/util.rs", "UNKNOWN", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/util.rs", "UTIL_DEPTH", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/util.rs", "UTIL_EXCEEDED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/matcher/util.rs", "UTIL_STACK_BASE", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/matcher/util.rs", "UTIL_STEPS_LEFT", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/phase_clock.rs", "IN_PHASE_0", Carried, ""),
    ("src/code_rule_dsl/profile.rs", "PHASE_STACK", Thread, "this thread's open phase timings"),
    ("src/code_rule_dsl/profile.rs", "PROFILE", Thread, "the older apply path's profile: each of its workers takes the session itself (`WorkerProfile`)"),
    ("src/code_rule_dsl/profile/sql.rs", "ACTIVE", Thread, "this thread's statement being profiled"),
    ("src/code_rule_dsl/profile/sql.rs", "EXPLAINING", Thread, "this thread's statement being explained"),
    ("src/code_rule_dsl/profile/sql.rs", "FRAME", Thread, "this thread's statement counters"),
    ("src/code_rule_dsl/profile/trace.rs", "CONTEXT", Call, "the attribution of the work this thread does for one call"),
    ("src/code_rule_dsl/residency/admission/relational/producer.rs", "READERS", Call, "the state of one call in progress on this thread"),
    ("src/code_rule_dsl/residency/admission/relational/syntax.rs", "POOL", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/semver/mod.rs", "KEY_COMPARISONS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/semver/mod.rs", "PARSES", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/semver/mod.rs", "SIMPLE_SUBSETS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("src/code_rule_dsl/sql_admission.rs", "CURRENT", Carried, ""),
    ("src/code_rule_dsl/sql_executor/runtime.rs", "LIMITED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/tape/navigation.rs", "COUNTERS", Thread, "a count of this thread's tape steps, read by tests"),
    ("src/code_rule_dsl/value/names.rs", "UNAVAILABLE", Call, "set by a lookup on this thread, taken by its caller (`take_unavailable`)"),
    ("src/code_rule_dsl/violations/projection.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/code_rule_dsl/walks.rs", "WALKS", Thread, "a count of this thread's tree walks, read by tests"),
    ("src/code_rule_dsl/waves/relational/producer.rs", "READERS", Call, "the state of one call in progress on this thread"),
    ("src/code_rule_dsl/waves/relational/syntax.rs", "POOL", Call, "the state of one call in progress on this thread, set and read within it"),
    ("src/content_store/atoms.rs", "RESERVING", Thread, "this thread's store write in progress"),
    ("src/content_store/atoms.rs", "SEGMENT_BYTES_TEST", Thread, "this thread's store write in progress"),
    ("src/content_store/compression.rs", "DECODERS", Thread, "this thread's codecs"),
    ("src/content_store/compression.rs", "DICT_ENCODERS", Thread, "this thread's codecs"),
    ("src/content_store/compression.rs", "ENCODER", Thread, "this thread's codecs"),
    ("src/content_store/connection.rs", "ATTEMPT_WAITED", Thread, "this thread's SQLite connections and their waits"),
    ("src/content_store/connection.rs", "BATCH", Thread, "this thread's SQLite connections and their waits"),
    ("src/content_store/connection.rs", "NO_BUSY_WAIT", Thread, "this thread's SQLite connections and their waits"),
    ("src/content_store/connection.rs", "OPERATION_WAITED", Thread, "this thread's SQLite connections and their waits"),
    ("src/content_store/connection.rs", "THREAD_CONNECTIONS", Thread, "this thread's SQLite connections and their waits"),
    ("src/content_store/connection.rs", "WAIT_SPENT", Thread, "this thread's SQLite connections and their waits"),
];

/// How many thread-locals are run context not carried yet ([`NotYet`]): lowered as each is carried, never raised.
const NOT_YET_PINNED: usize = 2;

/// One carried piece of the run's context, taken on the caller's thread: entering it on another thread returns a
/// guard that puts back what that thread had.
pub(crate) trait Part: Send + Sync {
    fn enter(&self) -> Box<dyn Any>;
}

/// Runs `f` once, when dropped.
struct OnDrop<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

/// A setting, copied.
struct Copied<T: Copy + Send + Sync + 'static>(&'static LocalKey<Cell<T>>, T);
impl<T: Copy + Send + Sync + 'static> Part for Copied<T> {
    fn enter(&self) -> Box<dyn Any> {
        let (key, old) = (self.0, self.0.with(|c| c.replace(self.1)));
        Box::new(OnDrop(Some(move || key.with(|c| c.set(old)))))
    }
}

/// A value, cloned in on each entry (a list of names, an `Arc`).
struct Cloned<T: Clone + Send + Sync + 'static>(&'static LocalKey<RefCell<T>>, T);
impl<T: Clone + Send + Sync + 'static> Part for Cloned<T> {
    fn enter(&self) -> Box<dyn Any> {
        let key = self.0;
        let old = key.with(|c| c.replace(self.1.clone()));
        Box::new(OnDrop(Some(move || {
            key.with(|c| c.replace(old));
        })))
    }
}

/// What [`Carry::here`] takes, by the names of the thread-locals each part carries.
pub(crate) struct Capture(Vec<(&'static [&'static str], Box<dyn Part>)>);

impl Capture {
    /// Carries the setting `key` (named `name`) by copy.
    pub(crate) fn copy<T: Copy + Send + Sync + 'static>(&mut self, name: &'static [&'static str], key: &'static LocalKey<Cell<T>>) {
        let v = key.with(Cell::get);
        self.0.push((name, Box::new(Copied(key, v))));
    }

    /// Carries `key` (named `name`) by a clone of its value.
    pub(crate) fn cloned<T: Clone + Send + Sync + 'static>(&mut self, name: &'static [&'static str], key: &'static LocalKey<RefCell<T>>) {
        let v = key.with(|c| c.borrow().clone());
        self.0.push((name, Box::new(Cloned(key, v))));
    }

    /// Carries the thread-locals `names` by `part` (shared state, merged back as the job ends).
    pub(crate) fn part(&mut self, names: &'static [&'static str], part: impl Part + 'static) {
        self.0.push((names, Box::new(part)));
    }
}

/// A part built from a pair of the owning module's functions: what to take, and how to enter it.
pub(crate) struct Shared<T: Clone + Send + Sync + 'static, G: 'static>(pub(crate) T, pub(crate) fn(T) -> G);
impl<T: Clone + Send + Sync + 'static, G: 'static> Part for Shared<T, G> {
    fn enter(&self) -> Box<dyn Any> {
        Box::new((self.1)(self.0.clone()))
    }
}

/// The modules whose thread-locals are carried, each taking its own.
const CARRIERS: &[fn(&mut Capture)] = &[
    super::plan::corpus::carried,
    super::plan::corpus::phases::carried,
    super::plan::carried,
    super::plan::keeps::carried,
    super::derived::carried,
    super::plan::demand::carried,
    super::plan::placement::carried,
    super::plan::corpus::switches::carried,
    #[cfg(any(test, feature = "test-hooks"))]
    super::plan::corpus::test_store::carried,
    carried_elsewhere,
];

/// The carried thread-locals of the crates below this one, which cannot name [`Capture`].
fn carried_elsewhere(c: &mut Capture) {
    c.copy(&["IN_PHASE_0"], &crate::code_rule_dsl::phase_clock::IN_PHASE_0);
    c.copy(&["STAGE"], &super::profile::STAGE);
    c.part(&["CURRENT"], Shared(crate::code_rule_dsl::sql_admission::current(), crate::code_rule_dsl::sql_admission::enter_current));
}

/// The run's context on the caller's thread, for a job on another to enter ([module docs](self)).
pub(crate) struct Carry {
    parts: Vec<(&'static [&'static str], Box<dyn Part>)>,
    /// The profile's span open where the job was started: the job's span is under it, on whichever thread runs it.
    under: Option<String>,
}

/// While it lives, a job's thread is in its caller's context; dropped, the thread's own is back.
pub(crate) struct Entered(Vec<Box<dyn Any>>);
impl Drop for Entered {
    fn drop(&mut self) {
        // Put back in the reverse order of entry.
        while let Some(g) = self.0.pop() {
            drop(g);
        }
    }
}

impl Carry {
    /// Takes the run's context on this thread.
    pub(crate) fn here() -> Carry {
        let mut c = Capture(Vec::new());
        for f in CARRIERS {
            f(&mut c);
        }
        Carry { parts: c.0, under: super::profile::here() }
    }

    /// Puts the context on this thread while the guard lives.
    pub(crate) fn enter(&self) -> Entered {
        Entered(self.parts.iter().map(|(_, p)| p.enter()).collect())
    }

    /// The profile's span open where it was taken.
    pub(crate) fn under(&self) -> Option<&str> {
        self.under.as_deref()
    }

    /// The thread-locals it carries.
    #[cfg(test)]
    fn names(&self) -> Vec<&'static str> {
        self.parts.iter().flat_map(|(n, _)| n.iter().copied()).collect()
    }
}

#[cfg(test)]
mod tests;
