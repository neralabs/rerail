//! A session's changed strata, as a run maintains them (decision 0364). The level driver (`plan::Maintain`) gives a
//! stratum a changed read reaches the native evaluator's maintain mode: its relations start as their kept ones in
//! place, removal joins over the changed reads' versions give candidates, each is rechecked with its head bound (a
//! recursive stratum's in level order, 0364 §2), and the add joins seed. A stratum the mode does not take is derived
//! again from its reads, its delta its diff against its kept rows (`plan::kept_delta`). There are no maintained
//! forms: no rules over `h__old`, `h__del`, `h__redo` or `h__ins` are written.
//!
//! This module holds what a run reports of that, and the settings it reads.

use super::lower;
use crate::code_rule_dsl::facts::vtab::Source;
use std::sync::Arc;

/// The part as a run evaluates it, and how its strata were evaluated.
pub(in crate::code_rule_dsl) struct Maintained {
    pub(in crate::code_rule_dsl) program: lower::Lowered,
    /// The changed inputs' rows gone and added, by input (`super::driven_part`): the maintain mode's versions of
    /// them, read in place (`Maintain::given`). Not relations of the run (0364 R1).
    pub(in crate::code_rule_dsl) given: Given,
    /// Strata maintained from deltas, and derived again.
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// Of those maintained, strata that are recursive.
    pub(in crate::code_rule_dsl) recursive: usize,
    /// Each stratum derived again, by one of its relations, and why.
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
}

/// The changed inputs' rows gone and added (`Maintained::given`), by input: `[minus, plus]`.
pub(in crate::code_rule_dsl) type Given = std::collections::HashMap<String, [Arc<dyn Source>; 2]>;

impl Maintained {
    /// A part whose strata the cut runs are all derived again (no kept part to drive, or gates): `NOT_WALKED`.
    pub(in crate::code_rule_dsl) fn again(program: lower::Lowered, strata: usize) -> Maintained {
        Maintained { program, given: Given::new(), maintained: 0, derived_again: strata, recursive: 0, again: vec![("*".to_owned(), NOT_WALKED); strata] }
    }
}

/// The reasons a stratum an edit reaches may be derived again (decision 0364, R3): its part's first run, which has
/// nothing kept, and 0090 §4's threshold (R4). Every other stratum is maintained by the mode, or skipped.
pub const FIRST_RUN: &str = "nothing kept: its part's first run";
pub const THRESHOLD: &str = "the edit reaches too much of it (0090 §4)";
/// A part the level driver does not walk (no kept part, or gates): every stratum of its cut derived again.
pub const NOT_WALKED: &str = "its part is not walked: no kept part, or gates";
/// A stratum no changed read reaches whose rows are not kept (a fact relation's, or one the last run did not keep):
/// derived from its reads, as they were.
pub const NOT_KEPT: &str = "not reached, its rows not kept";
/// A native operator's stratum (the reach index, decision 0119): made again from its reads as they are now.
pub const NATIVE_OPERATOR: &str = "a native operator's stratum, made again";
/// A read changed with no version (a relation derived again in SQL, or with its kept rows not held).
pub const NO_VERSION: &str = "a read changed without a version";

/// 0090 §4's threshold (0364 R4): whether an edit whose changed reads give `delta` rows, of the `stored` rows those
/// reads hold, reaches too much of a stratum to maintain it: `delta ≥ θ · stored` (`RERAIL_AGAIN_THETA`, default 0.25) and `delta ≥
/// RERAIL_AGAIN_MIN` (default 1,024 rows: below it, maintaining costs little whatever the share). `RERAIL_AGAIN=off`
/// maintains every stratum. Read at each choice, so a test can set them.
pub fn past_threshold(delta: usize, stored: usize) -> bool {
    if std::env::var("RERAIL_AGAIN").is_ok_and(|v| v == "off") {
        return false;
    }
    let theta = std::env::var("RERAIL_AGAIN_THETA").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.25);
    let min = std::env::var("RERAIL_AGAIN_MIN").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1024);
    delta >= min && delta as f64 >= theta * stored as f64
}

/// Whether a change of `delta` rows is past `RERAIL_AGAIN_MIN` (default 1,024): below it, a stratum is maintained
/// whatever its share or price.
pub fn past_floor(delta: usize) -> bool {
    if std::env::var("RERAIL_AGAIN").is_ok_and(|v| v == "off") {
        return false;
    }
    delta >= std::env::var("RERAIL_AGAIN_MIN").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1024)
}

/// 0090 §4's second clause (0364 R4): whether maintaining, priced at `mode` work, costs more than deriving again at
/// `derive` (`RERAIL_AGAIN_PRICE`: the mode's share of a derive past which it derives again, default 1).
pub fn costs_more(mode: f64, derive: f64) -> bool {
    let share = std::env::var("RERAIL_AGAIN_PRICE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
    mode > share * derive
}

/// The class of a reason the maintain mode gave for not taking a stratum (`Run::maintain_mode_blocks`), as the
/// report counts it: one of its fixed reasons, `NO_VERSION` for a read named in it, or `other`.
pub fn reason_class(why: &str) -> &'static str {
    const FIXED: &[&str] = &[
        FIRST_RUN,
        THRESHOLD,
        "a gated aggregate",
        "a native, lattice, gate, request, produced, input or fact relation",
        "a nearest or ast literal: no native step",
        NOT_KEPT,
        NATIVE_OPERATOR,
        "an aggregate over its own stratum",
        "its kept rows are not in memory",
        "no rules",
        "recursive, its kept rows without levels",
        "RERAIL_MAINTAIN_MODE=off",
    ];
    if why.ends_with("changed without a version") {
        return NO_VERSION;
    }
    FIXED.iter().find(|r| **r == why).copied().unwrap_or("other")
}

/// Whether recursive relations keep a level per row (decision 0090), so that the next run maintains them by levels.
pub(in crate::code_rule_dsl) fn levels_wanted() -> bool {
    true
}

thread_local! {
    /// The recursive strata this thread's runs maintained by levels. Counted by thread, not by process: a test reads
    /// its own runs' counts while other tests run beside it.
    static BY_LEVELS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The recursive strata maintained by levels so far by this thread's runs.
pub fn strata_by_levels() -> usize {
    BY_LEVELS.with(std::cell::Cell::get)
}

/// A recursive stratum the evaluator's maintain mode maintained by levels (decision 0364 §2).
pub fn by_levels_ran() {
    BY_LEVELS.with(|c| c.set(c.get() + 1));
}

/// What a run did that decision 0364's required shape (§ "Required shape") ruled out, counted by the run: read by the
/// requirement tests (`maintain_tests`), each in its run's scope, never by a run itself.
pub mod shape {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    /// The counts, the run's own (decision 0442). On the shared object, tests running beside one another in a process
    /// counted each other's runs (R8 read another test's frames).
    #[derive(Default)]
    struct Counts {
        rows_copied: AtomicUsize,
        maintenance_held: AtomicUsize,
        frames_read: AtomicUsize,
        rechecks_unpriced: AtomicUsize,
        negated_led: AtomicUsize,
        measures_unkeyed: AtomicUsize,
        inputs_hashed: AtomicUsize,
        inputs_from_entries: AtomicUsize,
        inputs_hashed_whole: AtomicUsize,
        inputs_read_whole: AtomicUsize,
        rows_decoded: AtomicUsize,
        drivers_unled: AtomicUsize,
    }

    fn counts() -> std::sync::Arc<Counts> {
        crate::code_rule_dsl::logic::run::local::<Counts>()
    }

    /// The counts so far.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Shape {
        /// R2: rows copied out of a relation, a delta or a document's entries to state a change (a delta as rows, a
        /// diff of a relation derived again against its kept rows).
        pub rows_copied: usize,
        /// R1: relations held under a maintenance name (`is_maintenance_name`).
        pub maintenance_held: usize,
        /// R8: kept frame parts read back by a run.
        pub frames_read: usize,
        /// R7: recheck lines ordered without a price.
        pub rechecks_unpriced: usize,
        /// R4 at scale: mode lines driven by a negated changed read whose delta leads the line.
        pub negated_led: usize,
        /// R4 at scale: mode lines whose changed read's delta does not lead the line (a negated read's line planned
        /// with its variables bound had no order): it walks its other reads whole.
        pub drivers_unled: usize,
        /// R6: proved components whose measure is by a node's depth or document order, run where that key is not
        /// read: their candidates go unordered, a stay checked again as each of its supports goes.
        pub measures_unkeyed: usize,
        /// R2: a session's inputs whose delta was found by hashing their documents (`retain::document_delta`),
        /// which reads every document's rows, the unchanged ones too, not the changed documents' memo entries.
        pub inputs_hashed: usize,
        /// R2: a session's inputs whose delta was read from the changed documents' memo entries (`input_delta`).
        pub inputs_from_entries: usize,
        /// R2: of `inputs_hashed`, those whose every row was hashed (`retain::document_delta`'s pass), not only the
        /// rows of the documents that changed (`retain::document_delta_except`).
        pub inputs_hashed_whole: usize,
        /// R2 (0353): a new process's changed inputs whose old rows `retain::Retained::load` read whole from the
        /// frames (copied into columns, every row hashed), not their changed documents' alone (`retain::OldInput`).
        pub inputs_read_whole: usize,
        /// 0353's frames copy: rows a new process decoded from the frames into memory (`retain::Retained::load`), its
        /// kept relations' and its changed inputs' old rows. Read in place once the kept blocks are bound (0401 step 7).
        pub rows_decoded: usize,
    }

    pub fn now() -> Shape {
        let c = counts();
        Shape {
            rows_copied: c.rows_copied.load(Relaxed),
            maintenance_held: c.maintenance_held.load(Relaxed),
            frames_read: c.frames_read.load(Relaxed),
            rechecks_unpriced: c.rechecks_unpriced.load(Relaxed),
            negated_led: c.negated_led.load(Relaxed),
            drivers_unled: c.drivers_unled.load(Relaxed),
            measures_unkeyed: c.measures_unkeyed.load(Relaxed),
            inputs_hashed: c.inputs_hashed.load(Relaxed),
            inputs_from_entries: c.inputs_from_entries.load(Relaxed),
            inputs_hashed_whole: c.inputs_hashed_whole.load(Relaxed),
            inputs_read_whole: c.inputs_read_whole.load(Relaxed),
            rows_decoded: c.rows_decoded.load(Relaxed),
        }
    }

    /// What happened between `before` and now.
    pub fn since(before: Shape) -> Shape {
        let n = now();
        Shape {
            rows_copied: n.rows_copied - before.rows_copied,
            maintenance_held: n.maintenance_held - before.maintenance_held,
            frames_read: n.frames_read - before.frames_read,
            rechecks_unpriced: n.rechecks_unpriced - before.rechecks_unpriced,
            negated_led: n.negated_led - before.negated_led,
            drivers_unled: n.drivers_unled - before.drivers_unled,
            measures_unkeyed: n.measures_unkeyed - before.measures_unkeyed,
            inputs_hashed: n.inputs_hashed - before.inputs_hashed,
            inputs_from_entries: n.inputs_from_entries - before.inputs_from_entries,
            inputs_hashed_whole: n.inputs_hashed_whole - before.inputs_hashed_whole,
            inputs_read_whole: n.inputs_read_whole - before.inputs_read_whole,
            rows_decoded: n.rows_decoded - before.rows_decoded,
        }
    }

    pub fn rows_copied(n: usize) {
        counts().rows_copied.fetch_add(n, Relaxed);
    }
    pub fn frames_read(n: usize) {
        counts().frames_read.fetch_add(n, Relaxed);
    }
    pub fn recheck_unpriced() {
        counts().rechecks_unpriced.fetch_add(1, Relaxed);
    }
    pub fn negated_led() {
        counts().negated_led.fetch_add(1, Relaxed);
    }
    pub fn input_hashed_whole() {
        counts().inputs_hashed_whole.fetch_add(1, Relaxed);
    }
    pub fn input_read_whole() {
        counts().inputs_read_whole.fetch_add(1, Relaxed);
    }
    pub fn rows_decoded(n: usize) {
        counts().rows_decoded.fetch_add(n, Relaxed);
    }
    pub fn input_from_entries() {
        counts().inputs_from_entries.fetch_add(1, Relaxed);
    }
    pub fn input_hashed() {
        counts().inputs_hashed.fetch_add(1, Relaxed);
    }
    pub fn measure_unkeyed() {
        counts().measures_unkeyed.fetch_add(1, Relaxed);
    }
    pub fn driver_unled() {
        counts().drivers_unled.fetch_add(1, Relaxed);
    }
    /// Notes a relation held under `name`, if it is a maintenance name.
    pub fn held(name: &str) {
        if is_maintenance_name(name) {
            counts().maintenance_held.fetch_add(1, Relaxed);
            // `RERAIL_SHAPE_TRACE`: each one named, to find what still holds them.
            if std::env::var_os("RERAIL_SHAPE_TRACE").is_some() {
                eprintln!("RERAIL_SHAPE_TRACE held {name}");
            }
        }
    }

    /// The suffixes of the relations the forms and counting kept (decisions 0117, 0241, 0298, 0090): none is a
    /// relation of a run under 0364 (R1). A relation's change is its version, read in place.
    pub const MAINTENANCE_SUFFIXES: &[&str] = &[
        "__old", "__del", "__ins", "__redo", "__minus", "__plus", "__ready", "__cand", "__chk", "__held", "__gone", "__live", "__sup", "__grp", "__mm_old", "__mm_minus", "__mm_plus",
    ];

    pub fn is_maintenance_name(name: &str) -> bool {
        MAINTENANCE_SUFFIXES.iter().any(|s| name.ends_with(s) || name.contains(&format!("{s}__")))
    }
}
