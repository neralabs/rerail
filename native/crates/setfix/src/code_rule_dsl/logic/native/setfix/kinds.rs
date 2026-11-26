//! E1's kind tests (decision 0706): a step testing a view's row by a bound node against a constant (`kind(n, K)`, or
//! its negation), its rows tested a batch at a time. The walker leaves each row at the step (`Ctx::batch`); a full
//! batch, or the walk's end, has its nodes' values read by the source in one call (`Source::read_own_batch`: every
//! node found, then every value read), and the rows that pass go on to the next step.
//!
//! The rows a batch holds are the walk's slots, bindings, not rows of a block (decision 0353).

use super::{JoinRule, Key, Src, Step};
use crate::code_rule_dsl::facts::vtab::Source;

/// The rows a step's batch holds before it is tested.
pub(super) const BATCH: usize = 256;

/// A kind test: the slot of its node, the source's column the node is the row's own id in, the column read, the
/// value wanted (a constant, or a slot's value), and whether the test is negated.
#[derive(Clone, Copy)]
pub(super) struct KindTest<'a> {
    pub slot: usize,
    col: usize,
    read: usize,
    pub want: Key,
    pub neg: bool,
    src: &'a dyn Source,
}

impl KindTest<'_> {
    /// Whether each of `nodes` has a row with its value wanted (`wants`: one for all, or one each; the test's answer,
    /// before its negation).
    pub fn read(&self, nodes: &[i64], wants: &[i64], out: &mut [bool]) {
        count_for_tests(nodes.len());
        let read = self.src.test_own_batch(self.col, nodes, self.read, wants, out);
        debug_assert!(read, "a kind test's source reads batches (checked when it was chosen)");
    }
}

/// Step `i` of `j` as a kind test, where it is one and its source reads batches: an atom of a whole view (no filter,
/// no ids), binding nothing, no tests of its own columns, not fused or ranged, keyed by a node
/// slot and a constant.
pub(super) fn kind_test(j: &JoinRule, i: usize) -> Option<KindTest<'_>> {
    let Step::Atom { src: Src::In(rel, ix), neg, keys, binds, tests } = &j.steps[i] else { return None };
    if !binds.is_empty() || !tests.is_empty() || j.fused[i].is_some() || j.ranges[i].is_some() {
        if trace() && keys.len() == 2 && binds.is_empty() { eprintln!("RERAIL_KIND_TRACE early {:?}: tests {} fused {} ranged {}", j.src_in(rel, ix).0.name.get(), tests.len(), j.fused[i].is_some(), j.ranges[i].is_some()); }
        return None;
    }
    let (rel, _) = j.src_in(rel, ix);
    let v = rel.view.as_ref()?;
    if !v.eqs.is_empty() || v.ids.is_some() {
        if trace() { eprintln!("RERAIL_KIND_TRACE refused {:?}: eqs {} ids {}", rel.name.get(), v.eqs.len(), v.ids.is_some()); }
        return None;
    }
    let &[(a, ka), (b, kb)] = keys.as_slice() else {
        if trace() { eprintln!("RERAIL_KIND_TRACE refused {:?}: keys {}", rel.name.get(), keys.len()); }
        return None;
    };
    let src: &dyn Source = v.src.as_ref();
    // The node's column: a slot's, the row's own id in the source (asked with no values); the other is the one read.
    let own = |c: usize, r: usize| v.cols.get(c).zip(v.cols.get(r)).filter(|&(&c, &r)| src.test_own_batch(c, &[], r, &[0], &mut [])).map(|(&c, &r)| (c, r));
    let found = match (ka, kb) {
        (Key::Slot(s), want) => own(a, b).map(|(col, read)| (s, col, read, want)),
        _ => None,
    }
    .or_else(|| match (kb, ka) {
        (Key::Slot(s), want) => own(b, a).map(|(col, read)| (s, col, read, want)),
        _ => None,
    });
    if trace() { eprintln!("RERAIL_KIND_TRACE {} {:?}", if found.is_some() { "chosen" } else { "no batch read" }, rel.name.get()); }
    let (slot, col, read, want) = found?;
    let ok = true;
    ok.then_some(KindTest { slot, col, read, want, neg: *neg, src })
}

/// Per step of `j`, its kind test and an empty batch, where the walk tests batches: `on` (an unlimited walk: a cut
/// would leave rows waiting), not `RERAIL_KIND_BATCH=off`, and not the steps of `not` (the step a screen reads, the
/// step a delta restricts).
pub(super) fn batches(j: &JoinRule, on: bool, not: [Option<usize>; 2]) -> Vec<Option<(KindTest<'_>, Vec<i64>)>> {
    if !on || !enabled() {
        if trace() { eprintln!("RERAIL_KIND_TRACE walk off: on {on} enabled {}", enabled()); }
        return Vec::new();
    }
    (0..j.steps.len()).map(|i| if i == 0 || not.contains(&Some(i)) { None } else { kind_test(j, i).map(|t| (t, Vec::new())) }).collect()
}

/// `RERAIL_KIND_BATCH=off` tests each row as it comes; `=walk` batches the walker's only, `=closure` the closures'
/// only (measurement aids).
fn mode() -> &'static str {
    #[cfg(any(test, feature = "test-hooks"))]
    match FORCE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => return "off",
        2 => return "",
        _ => {}
    }
    static MODE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| std::env::var("RERAIL_KIND_BATCH").unwrap_or_default());
    MODE.as_str()
}
fn enabled() -> bool {
    !matches!(mode(), "off" | "closure")
}
pub(super) fn in_closures() -> bool {
    !matches!(mode(), "off" | "walk")
}

/// `RERAIL_KIND_TRACE`: the steps chosen and refused, and each batch's rows, on stderr.
pub(super) fn trace() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_KIND_TRACE").is_some());
    *ON
}

/// Tests: batching forced off (`Some(false)`), on (`Some(true)`), or as the environment says (`None`).
#[cfg(any(test, feature = "test-hooks"))]
static FORCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// Tests: the rows the process's kind tests tested in batches.
#[cfg(any(test, feature = "test-hooks"))]
static ROWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(any(test, feature = "test-hooks"))]
pub fn force_for_tests(on: Option<bool>) {
    FORCE.store(on.map_or(0, |o| if o { 2 } else { 1 }), std::sync::atomic::Ordering::Relaxed);
}
#[cfg(any(test, feature = "test-hooks"))]
pub fn batched_rows_for_tests() -> u64 {
    ROWS.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn count_for_tests(n: usize) {
    ROWS.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(any(test, feature = "test-hooks")))]
#[inline(always)]
pub(super) fn count_for_tests(_: usize) {}

/// Tests: the node texts the process's walks read as their term's atom (`Source::node_text_atom`).
#[cfg(any(test, feature = "test-hooks"))]
static TERMS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(any(test, feature = "test-hooks"))]
pub fn term_atoms_for_tests() -> u64 {
    TERMS.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn count_term_atom_for_tests() {
    TERMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(any(test, feature = "test-hooks")))]
#[inline(always)]
pub(super) fn count_term_atom_for_tests() {}
