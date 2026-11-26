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
//!   another run while it waits). Since decision 0442 that is the run and its context (`rerail_context`'s `RUN` and
//!   `CONTEXT`): every setting and shared value of an extent of the run is a binding in the context, which the job
//!   reads as its caller does. What stays a part here is a job's own buffer (its plans, its observations, its test's
//!   store), merged into the run's as the job ends.
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
//! one is taken by [`Carry::here`]. `scripts/check/check_thread_state.py` refuses a commit that adds an unlisted
//! thread-local, before it is made.

use std::any::Any;

/// How a job on another thread treats a thread-local ([module docs](self)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Carried,
    Thread,
    Call,
    Outermost,
    NotYet,
}
use Class::*;

/// Every thread-local outside the tests: `(file under native/, name, class, why)` (Carried ones need no why: they
/// are carried).
pub const STATE: &[(&str, &str, Class, &str)] = &[
    ("crates/blocks/src/census.rs", "ID", Thread, "this thread's number in the anonymous room's census (a measuring aid, off by default)"),
    ("crates/blocks/src/census.rs", "N", Thread, "the census's next thread number, read once by `ID`'s first use on a thread"),
    ("crates/blocks/src/maps.rs", "STACK", Thread, "this thread's handle on its stack's room (`scratch::StackRoom`): the same on any thread"),
    ("crates/blocks/src/maps.rs", "WITHIN", Call, "the owner's room of the structures a call makes while it runs (`maps::within`); work it hands to another thread takes that thread's stack"),
    ("crates/blocks/src/scratch.rs", "ID", Thread, "this thread's number, the owner a stack's regions are popped by"),
    ("crates/blocks/src/scratch.rs", "LARGE_ROOM", Thread, "this thread's free-listed blocks for its large regions (`maps::FreeRoom::large`)"),
    ("crates/blocks/src/scratch.rs", "THREAD", Thread, "this thread's stack (decision 0342, amended 2026-11-24): its small structures' and its scratch uses' memory"),
    ("crates/c_directives/src/expand.rs", "JIT", Thread, "this thread's Cranelift code of the process's pp words (0348), without a rustc build of them; the same on any thread"),
    ("crates/ukl_vec/src/exec.rs", "SLOTS", Thread, "this thread's last launches' slots, by their shape: storage the next launch of one reuses, written before it is read"),
    ("crates/ukl_vec/src/fiber.rs", "POOL", Thread, "this thread's spare fiber stacks, each emptied at its next use"),
    ("crates/ukl_vec/src/fiber.rs", "PREPARED", Thread, "the fibers this thread prepared, a count of its own work its tests read"),
    ("crates/ukl_vec/src/run.rs", "SLOTS", Thread, "this thread's last launches' slots, by their shape: storage the next launch of one reuses, written before it is read"),
    ("crates/compiler/src/pp_words.rs", "HOST_RUNS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/metal/src/buffer_observation.rs", "ACTIVE", Thread, "this thread's scratch or counters in a backend"),
    ("crates/metal/src/test_pool.rs", "LEDGERS", Thread, "this thread's share of the test pool"),
    ("crates/metal/src/test_pool.rs", "MEASURED", Thread, "this thread's test reads the driver's own figure (a test switch, per thread as the test pool is)"),
    ("crates/metal/src/test_pool.rs", "WORKING_SET", Thread, "this thread's test's working set for the device (a test switch, per thread as the test pool is)"),
    ("crates/prof/src/lib.rs", "LEFT", Thread, "this thread's sampling countdown"),
    ("crates/prof/src/lib.rs", "THREAD_IX", Thread, "this thread's number in the heap samples taken with no span open"),
    ("crates/lang/src/code_rule_dsl/budget.rs", "BUDGET_COUNTDOWN", Call, "the state of one call in progress on this thread"),
    ("crates/lang/src/code_rule_dsl/budget.rs", "MATCH_BUDGET", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/lang/src/code_rule_dsl/budget.rs", "STEP_COUNTDOWN", Call, "the state of one call in progress on this thread"),
    ("crates/adapters/src/code_rule_dsl/calls/mod.rs", "EVALUATION", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/calls/mod.rs", "FILE_CALLS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/lang_rust/src/code_rule_dsl/calls/rust.rs", "ITEMS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/demand_catalog/src/code_rule_dsl/demand/driver/catalog.rs", "CONNECTION", Thread, "this thread's query connection"),
    ("crates/rules_base/src/code_rule_dsl/eval/query.rs", "EVALUATION_TAPE", Call, "the tape of the evaluation under way on this thread"),
    ("crates/facts_base/src/code_rule_dsl/facts/d_projection.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/workspace/src/code_rule_dsl/facts/facts_a/facts.rs", "KIND_CACHE", Thread, "kind codes by name: any thread's gives the same"),
    ("crates/workspace/src/code_rule_dsl/facts/facts_a/facts.rs", "NAME_CACHE", Thread, "kind names by code: any thread's gives the same"),
    ("crates/facts_base/src/code_rule_dsl/facts/vtab/demand.rs", "FRAMES", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/facts_base/src/code_rule_dsl/facts/vtab/lookup_spike.rs", "WORK", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/facts/src/code_rule_dsl/facts/vtab/syntax.rs", "KEPT", Thread, "dialects by grammar: any thread's gives the same"),
    ("crates/lang_ecma/src/code_rule_dsl/import_target/mod.rs", "VIEW", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/rr_root/src/code_rule_dsl/logic/plan/corpus/combined.rs", "READ_BEFORE", Thread, "the rule files' entries read on this thread before its run, which the outermost run names"),
    ("crates/costs/src/code_rule_dsl/logic/costs.rs", "READING", Thread, "the cost statistics' read runs on this thread: it never waits for itself"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "DEFERRED", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "DONE", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "FLUSHED", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "FLUSHES", Thread, "the flushes on this thread, never taken back: whether this thread flushed since a mark (`lanes::wave::kept_here`)"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "JOINED", Thread, "the submissions to this thread's wave: a stratum that joined one resumes on this thread (`lanes::wave::kept_here`)"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "LEVEL", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "OPEN", Thread, "the open submission of the job running on this thread: each pool job enters its own and puts back the thread's as it leaves (`lanes::wave::Scope`, decision 0386)"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "QUEUED", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("crates/run/src/code_rule_dsl/logic/lanes.rs", "SUSPENDABLE", Thread, "this thread's device queue: what it issued, it flushes and waits for"),
    ("crates/setfix/src/code_rule_dsl/logic/native/setfix.rs", "GROWN", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("crates/setfix/src/code_rule_dsl/logic/native/setfix.rs", "MIXED", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("crates/context/src/lib.rs", "RUN", Carried, ""),
    ("crates/context/src/lib.rs", "CONTEXT", Carried, ""),
    ("crates/context/src/lib.rs", "HEAP_SPAN", Carried, ""),
    ("crates/facts_base/src/code_rule_dsl/facts/rows.rs", "GIVEN", Thread, "the bytes this thread's chunk scopes gave their chunks: the thread's own allocation counts, read against its jemalloc net"),
    ("crates/facts_base/src/code_rule_dsl/facts/rows.rs", "NET", Thread, "inside rows.rs's tests module (a test build only): a test's stand-in for the allocator's net counter"),
    ("crates/facts_base/src/code_rule_dsl/facts/rows.rs", "PEAK", Thread, "this thread's innermost chunk scope's heap peak so far: the thread's own allocation counts (decision 0469)"),
    ("crates/facts_base/src/code_rule_dsl/facts/rows.rs", "PEAK_AT", Thread, "inside rows.rs's tests module (a test build only): a test's stand-in for the allocator's per-thread peak"),
    ("crates/facts_base/src/code_rule_dsl/facts/rows.rs", "TOP", Carried, ""),
    ("crates/boot/src/code_rule_dsl/boot.rs", "COUNTERS", Thread, "jemalloc's counters of this thread's allocations"),
    ("crates/setfix/src/code_rule_dsl/logic/native/setfix.rs", "IN_MORSELS", Thread, "the walks this thread ran in morsels, read by tests"),
    ("crates/setfix/src/code_rule_dsl/logic/native/setfix.rs", "VARIANTS", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("crates/setfix/src/code_rule_dsl/logic/native/setfix/closure.rs", "SEMI_NAIVE_HERE", Thread, "this thread's fixpoint statistics, read by the profile and tests"),
    ("crates/setfix/src/code_rule_dsl/logic/native/setfix/runs.rs", "INLINE", Call, "a lazy order's build under way on this thread (`runs::inline`): its sorts stay on the thread, never handed to the pool"),
    ("crates/device_metal/src/code_rule_dsl/logic/native/setfix/device.rs", "ARENA", Thread, "this thread's device waves and launches"),
    ("crates/device_metal/src/code_rule_dsl/logic/native/setfix/device.rs", "BINDER", Thread, "this thread's device waves and launches"),
    ("crates/device_metal/src/code_rule_dsl/logic/native/setfix/device.rs", "DEPTH", Thread, "this thread's device waves and launches"),
    ("crates/device_metal/src/code_rule_dsl/logic/native/setfix/device.rs", "FLUSHED", Thread, "this thread's device waves and launches"),
    ("crates/device_metal/src/code_rule_dsl/logic/native/setfix/device.rs", "LAST", Thread, "this thread's device waves and launches"),
    ("crates/device_metal/src/code_rule_dsl/logic/native/setfix/device.rs", "SUSPENDED", Thread, "this thread's device waves and launches"),
    ("crates/plan_search/src/code_rule_dsl/logic/plan/search.rs", "OWN_PLANS", Carried, ""),
    ("crates/plan_search/src/code_rule_dsl/logic/plan/search.rs", "OWN_PLANNED", Carried, ""),
    ("crates/plan_search/src/code_rule_dsl/logic/plan/search.rs", "SEED", Call, "the last priced body's cheapest start, read by the same pricing"),
    ("crates/planner/src/code_rule_dsl/logic/plan.rs", "WALK_SALT", Call, "the walks' seeds' salt, set while one search repeats its walks (decision 0247)"),
    ("crates/planner/src/code_rule_dsl/logic/plan/compose.rs", "CACHE", Thread, "compiled compositions by digest: any thread's gives the same"),
    ("crates/logic/src/code_rule_dsl/logic/plan/corpus.rs", "PREPARED", Thread, "prepared programs by key: any thread's gives the same program"),
    ("crates/observe/src/git_snapshot.rs", "REPOSITORY", Thread, "a worker-local gix handle keyed by view and pack epoch"),
    ("crates/corpus_memo/src/code_rule_dsl/logic/plan/corpus/maintain.rs", "BY_LEVELS", Thread, "counts of this thread's own runs, which a test reads beside others (decision 0204)"),
    ("crates/rr_root/src/code_rule_dsl/logic/plan/corpus/test_store.rs", "FILLING", Thread, "this thread fills the test store's template: its own runs take the switches without making it first"),
    ("src/reactor.rs", "KEPT", Outermost, "the reactor's session between its commands, taken by each command (decision 0204)"),
    ("src/reactor.rs", "SHARED", Outermost, "what the instance's commands share by design (decision 0442): its one caller's object"),
    ("crates/rr_root/src/code_rule_dsl/logic/plan/corpus/test_store.rs", "OWN", Carried, ""),
    ("crates/plan_search/src/code_rule_dsl/logic/plan/keeps.rs", "OWN_OBSERVED", Carried, ""),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "DECIDING", Call, "the decisions of the step open on this thread (decision 0269): a pool job's are the profile's alone"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "BEHIND", Thread, "marks the store's writer thread"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "QUIET", Call, "a debug build's tree check beside a pass, its decisions unrecorded for the call (`quiet`): a pool job it starts records its own"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "HELD", Thread, "whether this thread holds the profile's state"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "SHARD", Thread, "this thread's shard of its run's profile records, merged into them as they are read (decision 0442)"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "STACK", Thread, "this thread's open spans; a job opens its span under its caller's (`profile::here`, carried)"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "COMMAND", Outermost, "the command's clocks, on the thread that runs it and its outermost run"),
    ("crates/context/src/lib.rs", "SEEN", Thread, "this thread's copy of the settings its run under way read, held with that run (decision 0442)"),
    ("crates/prof/src/code_rule_dsl/logic/profile.rs", "WARM_SCOPE", Thread, "the warm check's mark of the transaction a store writer has open on this thread; its commit hook runs on it"),
    ("crates/rr_root/src/code_rule_dsl/logic/rr.rs", "READS", Call, "the libraries read while `recording` runs; a job records its own and hands them back (`rr::recorded_on_worker`)"),
    ("crates/facts_base/src/code_rule_dsl/logic/strings/functions.rs", "CACHE", Thread, "compiled regexes by pattern: any thread's gives the same"),
    ("crates/matcher/src/code_rule_dsl/matcher/binding.rs", "FOLLOWING", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/d_anchors/src/code_rule_dsl/matcher/d_anchors/sql/corpus.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/matcher/each.rs", "ACTIVE", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/matcher/each.rs", "EXCLUDED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/matcher/each.rs", "GENERATION", Call, "the state of one call in progress on this thread"),
    ("crates/matcher/src/code_rule_dsl/matcher/each.rs", "HOSTS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/matcher/gap.rs", "TAPE_GAPS", Call, "one tape's parents' gaps while its facts are found on this thread (`fact_rows`, `GapScope`); outside a scope each test makes its own"),
    ("crates/matcher/src/code_rule_dsl/matcher/embedded/mod.rs", "HOOKS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/matcher/memo.rs", "MEMO", Call, "the matches of the file being matched on this thread"),
    ("crates/matcher/src/code_rule_dsl/matcher/mod.rs", "CLAUSES_BEGUN", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/matcher/src/code_rule_dsl/matcher/mod.rs", "UTIL_PEAK", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/p_anchors/src/code_rule_dsl/matcher/p_anchors/sql/corpus.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/matcher/parse.rs", "HOOKS", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/matcher/resolve.rs", "CURRENT_CLAUSE", Call, "the state of one call in progress on this thread"),
    ("crates/adapters/src/code_rule_dsl/matcher/resolve.rs", "FILE_SCOPES", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/matcher/resolve.rs", "JSX_RUNTIME_DERIVATIONS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/resolve.rs", "SOURCE_TYPE_DERIVATIONS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/resolve.rs", "SUB_TABLES", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/matcher/resolve.rs", "TABLE_BUILDS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "CLAUSE_STATE_READS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "FILL_STEPS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "THREE_VALUED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "UNKNOWN", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "UTIL_DEPTH", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "UTIL_EXCEEDED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "UTIL_STACK_BASE", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/adapters/src/code_rule_dsl/matcher/steps.rs", "UTIL_STEPS_LEFT", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/matcher/src/code_rule_dsl/profile.rs", "PHASE_STACK", Thread, "this thread's open phase timings"),
    ("crates/matcher/src/code_rule_dsl/profile.rs", "PROFILE", Thread, "the older apply path's profile: each of its workers takes the session itself (`WorkerProfile`)"),
    ("crates/facts_base/src/code_rule_dsl/profile/sql.rs", "ACTIVE", Thread, "this thread's statement being profiled"),
    ("crates/facts_base/src/code_rule_dsl/profile/sql.rs", "EXPLAINING", Thread, "this thread's statement being explained"),
    ("crates/facts_base/src/code_rule_dsl/profile/sql.rs", "FRAME", Thread, "this thread's statement counters"),
    ("crates/matcher/src/code_rule_dsl/profile/trace.rs", "CONTEXT", Call, "the attribution of the work this thread does for one call"),
    ("crates/lang/src/code_rule_dsl/regexes/mod.rs", "LET_GO", Call, "the regexes a document's compilation let go of, kept by the table until that compilation ends"),
    ("crates/rules/src/code_rule_dsl/residency/admission/relational/producer.rs", "READERS", Call, "the state of one call in progress on this thread"),
    ("crates/rules/src/code_rule_dsl/residency/admission/relational/syntax.rs", "POOL", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/lang/src/code_rule_dsl/semver/mod.rs", "KEY_COMPARISONS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/lang/src/code_rule_dsl/semver/mod.rs", "PARSES", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/lang/src/code_rule_dsl/semver/mod.rs", "SIMPLE_SUBSETS", Thread, "a count of this thread's work, read by tests and the profile"),
    ("crates/sessions_base/src/code_rule_dsl/sql_executor/runtime.rs", "LIMITED", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/lang/src/code_rule_dsl/tape.rs", "ALL_TOKENS", Call, "one parse in progress keeps every token (`with_all_tokens`, capture-tapes' format-11 baseline)"),
    ("crates/lang/src/code_rule_dsl/tape.rs", "LAST", Thread, "this thread's previous read of each tape part, for its read-pattern counts (E17)"),
    ("crates/lang/src/code_rule_dsl/tape.rs", "PAUSED", Call, "a decode in progress on this thread, whose reads are not counted (E17)"),
    ("crates/lang/src/code_rule_dsl/tape/navigation.rs", "COUNTERS", Thread, "a count of this thread's tape steps, read by tests"),
    ("crates/adapters/src/code_rule_dsl/value/names.rs", "UNAVAILABLE", Call, "set by a lookup on this thread, taken by its caller (`take_unavailable`)"),
    ("crates/rules_base/src/code_rule_dsl/violations/projection.rs", "CURRENT", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/matcher/src/code_rule_dsl/walks.rs", "WALKS", Thread, "a count of this thread's tree walks, read by tests"),
    ("crates/d_relational/src/code_rule_dsl/waves/relational/producer.rs", "READERS", Call, "the state of one call in progress on this thread"),
    ("crates/d_relational/src/code_rule_dsl/waves/relational/syntax.rs", "POOL", Call, "the state of one call in progress on this thread, set and read within it"),
    ("crates/store/src/content_store/atoms.rs", "RESERVING", Thread, "this thread's store write in progress"),
    ("crates/store/src/content_store/atoms/map.rs", "MAPPED", Call, "a write in progress whose symbols are mapped (`atoms::map::mapped`)"),
    ("crates/store_base/src/content_store/compression.rs", "DECODER", Thread, "this thread's codecs"),
    ("crates/store_base/src/content_store/compression.rs", "ENCODED", Thread, "the bytes of frames this thread made, read around a writer's job"),
    ("crates/store_base/src/content_store/compression.rs", "ENCODER", Thread, "this thread's codecs"),
    ("crates/store/src/content_store/connection.rs", "ATTEMPT_WAITED", Thread, "this thread's SQLite connections and their waits"),
    ("crates/store/src/content_store/connection.rs", "BATCH", Thread, "this thread's SQLite connections and their waits"),
    ("crates/store/src/content_store/connection.rs", "IN_RETRY", Call, "an attempt of a retried transaction in progress (`retry_busy`)"),
    ("crates/store/src/content_store/connection.rs", "NO_BUSY_WAIT", Thread, "this thread's SQLite connections and their waits"),
    ("crates/store/src/content_store/connection.rs", "OPERATION_WAITED", Thread, "this thread's SQLite connections and their waits"),
    ("crates/store/src/content_store/connection.rs", "THREAD_CONNECTIONS", Thread, "this thread's SQLite connections and their waits"),
    ("crates/store/src/content_store/connection.rs", "WAIT_SPENT", Thread, "this thread's SQLite connections and their waits"),
];

/// How many thread-locals are run context not carried yet ([`NotYet`]): lowered as each is carried, never raised.
pub const NOT_YET_PINNED: usize = 0;

/// One carried piece of the run's context, taken on the caller's thread: entering it on another thread returns a
/// guard that puts back what that thread had.
pub trait Part: Send + Sync {
    fn enter(&self) -> Box<dyn Any>;
}

/// What [`Carry::here`] takes, by the names of the thread-locals each part carries.
pub struct Capture(Vec<(&'static [&'static str], Box<dyn Part>)>);

impl Capture {
    /// Carries the thread-locals `names` by `part` (shared state, merged back as the job ends).
    pub fn part(&mut self, names: &'static [&'static str], part: impl Part + 'static) {
        self.0.push((names, Box::new(part)));
    }
}

/// A part built from a pair of the owning module's functions: what to take, and how to enter it.
pub struct Shared<T: Clone + Send + Sync + 'static, G: 'static>(pub T, pub fn(T) -> G);
impl<T: Clone + Send + Sync + 'static, G: 'static> Part for Shared<T, G> {
    fn enter(&self) -> Box<dyn Any> {
        Box::new((self.1)(self.0.clone()))
    }
}

/// The modules whose thread-locals are carried, each taking its own, in the order they are entered. The program's
/// composition registers them (boot, or a crate's own test binary as it loads): they are the run's, the planner's
/// and setfix's, and the planner is a crate above this one (`logic::CARRIERS`). The run comes first: entered first,
/// it is put back last, so every other part's guard drops inside it (a plans guard's drop counts into the run).
pub static CARRIERS: rerail_registry::Slot<&'static [fn(&mut Capture)]> = rerail_registry::Slot::new("the run's carried context (run_state::CARRIERS)");

/// The carriers of a program of the run alone (`rerail-run`'s own test binary): the run's. Setfix's test binary adds
/// its own (`setfix::RUN_CARRIERS`).
pub const RUN_CARRIERS: &[fn(&mut Capture)] = &[super::run::carried];

/// The run's context on the caller's thread, for a job on another to enter ([module docs](self)).
pub struct Carry {
    parts: Vec<(&'static [&'static str], Box<dyn Part>)>,
    /// The profile's span open where the job was started: the job's span is under it, on whichever thread runs it.
    under: Option<String>,
}

/// While it lives, a job's thread is in its caller's context; dropped, the thread's own is back.
pub struct Entered(Vec<Box<dyn Any>>);
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
    pub fn here() -> Carry {
        let mut c = Capture(Vec::new());
        for f in CARRIERS.get().unwrap_or_else(|missing| panic!("{missing}: no program registered what a job carries")) {
            f(&mut c);
        }
        Carry { parts: c.0, under: super::profile::here() }
    }

    /// Puts the context on this thread while the guard lives.
    pub fn enter(&self) -> Entered {
        Entered(self.parts.iter().map(|(_, p)| p.enter()).collect())
    }

    /// The profile's span open where it was taken.
    pub fn under(&self) -> Option<&str> {
        self.under.as_deref()
    }

    /// The thread-locals it carries.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn names(&self) -> Vec<&'static str> {
        self.parts.iter().flat_map(|(n, _)| n.iter().copied()).collect()
    }
}

// Its tests read every thread-local of the tree and what the whole program carries (`logic::CARRIERS`):
// `rerail-logic`'s facade of this module mounts `tests.rs`.
