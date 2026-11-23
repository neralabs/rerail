//! What a run did that decision 0364's required shape (§ "Required shape") ruled out, counted by the run: read by the
//! requirement tests (`maintain_tests`), each in its run's scope, never by a run itself.
//! (`plan::corpus::maintain::shape`, under that name too.)

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
    inputs_from_owners: AtomicUsize,
    inputs_hashed_whole: AtomicUsize,
    inputs_read_whole: AtomicUsize,
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
    /// R2: inputs whose delta was made from their changed owners' rows gained and lost, beside them, before the run
    /// asks for any: a kept input patched by document, or a base relation's versions (0458: the changed
    /// documents' old and new rows, `input_delta::Before`), which a new process has where it keeps no base rows
    /// to hash.
    pub inputs_from_owners: usize,
    /// R2: of `inputs_hashed`, those whose every row was hashed (`retain::document_delta`'s pass), not only the
    /// rows of the documents that changed (`retain::document_delta_except`).
    pub inputs_hashed_whole: usize,
    /// R2 (0353): a new process's changed inputs whose old rows `retain::Retained::load` read whole from the
    /// frames (copied into columns, every row hashed), not their changed documents' alone (`retain::OldInput`).
    pub inputs_read_whole: usize,
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
        inputs_from_owners: c.inputs_from_owners.load(Relaxed),
        inputs_hashed_whole: c.inputs_hashed_whole.load(Relaxed),
        inputs_read_whole: c.inputs_read_whole.load(Relaxed),
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
        inputs_from_owners: n.inputs_from_owners - before.inputs_from_owners,
        inputs_hashed_whole: n.inputs_hashed_whole - before.inputs_hashed_whole,
        inputs_read_whole: n.inputs_read_whole - before.inputs_read_whole,
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
pub fn input_from_entries() {
    counts().inputs_from_entries.fetch_add(1, Relaxed);
}
pub fn input_hashed() {
    counts().inputs_hashed.fetch_add(1, Relaxed);
}
pub fn input_from_owners() {
    counts().inputs_from_owners.fetch_add(1, Relaxed);
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
