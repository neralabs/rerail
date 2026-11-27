//! E1's node reads (decision 0706): a step reading a view's row by a bound node, its own id (`kind(n, K)`,
//! `!kind(n, K)`, `node(d, n, k, s, e)` and `field(p, f, c)` with the node bound), its rows read a batch at a time.
//! The walker leaves each row at the step (`Ctx::batch`); a full batch, or the walk's end, has its nodes' rows read
//! by the source in one call (`Source::read_own_batch`: every node found, then every column read), and each row whose
//! row has the step's keys (and is equal where its tests say) binds its slots and goes on to the next step; a
//! negated one goes on where there is no such row. A node the source cannot read in a batch (a values part's) is
//! probed as one row (`Answer::Probe`).
//!
//! The rows a batch holds are the walk's slots, bindings, not rows of a block (decision 0353).

use super::{JoinRule, Key, Src, Step};
use crate::code_rule_dsl::facts::vtab::{ProbeCursor, Source};

/// The rows a step's batch holds before it is read.
pub(super) const BATCH: usize = 256;

/// A node read: the slot of its node, the source's column the node is the row's own id in, the source's columns read,
/// the keys checked (a read's place and the value wanted, a constant or a slot's), the slots bound (a read's place and
/// the slot), the reads that must be equal, and whether the step is negated.
#[derive(Clone)]
pub(super) struct OwnRead<'a> {
    pub slot: usize,
    col: usize,
    reads: Vec<usize>,
    checks: Vec<(usize, Key)>,
    binds: Vec<(usize, usize)>,
    equal: Vec<(usize, usize)>,
    pub neg: bool,
    src: &'a dyn Source,
    /// The source's cursor, kept from batch to batch (its hints, node memo and plan of the reads).
    cursor: std::cell::RefCell<ProbeCursor>,
}

/// What a row's read says: it goes on (its slots bound), it does not, or it is probed as the step probes one row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Answer {
    No,
    Yes,
    Probe,
}

impl OwnRead<'_> {
    /// Each of `rows` (the walk's slots, `width` a row) answered, its reads' values in `vals` (`reads` a row).
    pub fn read(&self, rows: &[i64], width: usize, vals: &mut Vec<i64>, answers: &mut Vec<Answer>) {
        let n = rows.len() / width.max(1);
        count_for_tests(n);
        let nodes: Vec<i64> = (0..n).map(|r| rows[r * width + self.slot]).collect();
        let k = self.reads.len();
        vals.clear();
        vals.resize(n * k, 0);
        let mut found = vec![0u8; n];
        let read = self.src.read_own_batch(self.col, &nodes, &self.reads, vals, &mut found, &mut self.cursor.borrow_mut());
        debug_assert!(read, "a node read's source reads batches (checked when it was chosen)");
        answers.clear();
        answers.extend((0..n).map(|r| {
            let row = &rows[r * width..(r + 1) * width];
            let got = &vals[r * k..(r + 1) * k];
            let hit = match found[r] {
                0 => false,
                1 => self.checks.iter().all(|&(at, want)| got[at] == match want { Key::Const(c) => c, Key::Slot(s) => row[s] }) && self.equal.iter().all(|&(a, b)| got[a] == got[b]),
                _ => return Answer::Probe,
            };
            if hit != self.neg { Answer::Yes } else { Answer::No }
        }));
    }
    /// Row `row`'s slots bound from its reads' values `got`.
    pub fn bind(&self, row: &mut [i64], got: &[i64]) {
        for &(at, s) in &self.binds {
            row[s] = got[at];
        }
    }
    /// Whether it binds nothing (a test alone, as a closure asks one).
    pub fn tests_only(&self) -> bool {
        self.binds.is_empty()
    }
}

/// Step `i` of `j` as a node read, where it is one and its source reads batches: an atom of a whole view (no filter,
/// no ids), not fused or ranged, one of its keys a slot on the column the source keeps its rows' own ids in.
pub(super) fn own_read(j: &JoinRule, i: usize) -> Option<OwnRead<'_>> {
    let Step::Atom { src: Src::In(rel, ix), neg, keys, binds, tests } = &j.steps[i] else { return None };
    if j.fused[i].is_some() || j.ranges[i].is_some() || keys.is_empty() {
        return None;
    }
    let (rel, _) = j.src_in(rel, ix);
    let v = rel.view.as_ref()?;
    // A filtered view's rows are those found at its first scan (`View::ids`), not its filters checked against the source
    // later (whose values may have grown since: on `all` that read gave other rows, decision 0706 step 2's record).
    if v.ids.is_some() {
        if trace() { eprintln!("RERAIL_KIND_TRACE refused {:?}: eqs {} ids {}", rel.name.get(), v.eqs.len(), v.ids.is_some()); }
        return None;
    }
    let src: &dyn Source = v.src.as_ref();
    // The node's key: a slot's, on the source's own column (asked with no values).
    let Some((lead, slot, col)) = keys.iter().enumerate().find_map(|(at, &(c, k))| match k {
        Key::Slot(s) => v.cols.get(c).filter(|&&c| src.read_own_batch(c, &[], &[], &mut [], &mut [], &mut ProbeCursor::default())).map(|&c| (at, s, c)),
        Key::Const(_) => None,
    }) else {
        if trace() { eprintln!("RERAIL_KIND_TRACE no own key {:?}: keys {:?} cols {:?} binds {}", rel.name.get(), keys.iter().map(|&(c, k)| (v.cols.get(c).copied(), matches!(k, Key::Slot(_)))).collect::<Vec<_>>(), v.cols, binds.len()); }
        return None;
    };
    // Every other column the step reads, once: its keys', its binds', its tests'.
    let mut reads: Vec<usize> = Vec::new();
    let mut source = |c: usize| reads.iter().position(|&r| r == c).unwrap_or_else(|| { reads.push(c); reads.len() - 1 });
    let mut place = |c: usize| -> Option<usize> { Some(source(*v.cols.get(c)?)) };
    let checks = keys.iter().enumerate().filter(|&(at, _)| at != lead).map(|(_, &(c, k))| place(c).map(|p| (p, k))).collect::<Option<Vec<_>>>()?;
    let binds = binds.iter().map(|&(c, s)| place(c).map(|p| (p, s))).collect::<Option<Vec<_>>>()?;
    let equal = tests.iter().map(|&(a, b)| Some((place(a)?, place(b)?))).collect::<Option<Vec<_>>>()?;
    if trace() { eprintln!("RERAIL_KIND_TRACE chosen {:?} reads {} binds {} neg {neg}", rel.name.get(), reads.len(), binds.len()); }
    Some(OwnRead { slot, col, reads, checks, binds, equal, neg: *neg, src, cursor: Default::default() })
}

/// E5's keyed probes (decision 0706): a step probing a relation, or a view's source, by bound slots, its batch of
/// rows sorted by their keys and then each probed as the walker probes one row. The step's probe state goes from row
/// to row (its hints into the order's runs, its last probe's memo, its source's cursor), so a sorted batch's probes
/// are a merge against the held order (decision 0112): a key equal to the last is the memo's, and a key past it is
/// found galloping from where the last one was.
#[derive(Clone)]
pub(super) struct Keyed {
    /// The slot of the key's first column, which the rows sort by.
    slot: usize,
}

impl Keyed {
    /// The places of `rows` (`width` a row), in the order of their keys (and their place, for equal keys: the
    /// order the walk gave them).
    pub fn order(&self, rows: &[i64], width: usize) -> Vec<u32> {
        let n = rows.len() / width.max(1);
        count_keyed_for_tests(n);
        let mut by: Vec<(i64, u32)> = (0..n).map(|r| (rows[r * width + self.slot], r as u32)).collect();
        by.sort_unstable();
        by.into_iter().map(|(_, r)| r).collect()
    }
}

/// Step `i` of `j` as a keyed probe that binds (E5): an atom, not fused, keyed by at least one slot and binding a
/// column, on a relation of the run or on an input that is not read by row id (`keyed`: a key's rows are found
/// without a search, in any order). A test binding nothing is E2's (its filters and node bits answer it first).
pub(super) fn keyed(j: &JoinRule, i: usize) -> Option<Keyed> {
    let Step::Atom { src, keys, binds, .. } = &j.steps[i] else { return None };
    if binds.is_empty() {
        return None;
    }
    // A fused probe is batched where it has a spanning form (`rowcode::spanning`): its batch's candidates, many
    // bindings', are one chunk of its row program. `RERAIL_SPAN_BATCH=off` walks it a binding at a time.
    if j.fused[i].is_some() && (j.span_at(i).is_none() || !span_enabled()) {
        if trace() { eprintln!("RERAIL_KIND_TRACE keyed refused: fused"); }
        return None;
    }
    if let Src::In(rel, ix) = src
        && j.src_in(rel, ix).1.keyed
    {
        if trace() { eprintln!("RERAIL_KIND_TRACE keyed refused: keyed {:?}", j.src_in(rel, ix).0.name.get()); }
        return None;
    }
    let mut by: Vec<(usize, usize)> = keys.iter().filter_map(|&(c, k)| match k {
        Key::Slot(s) => Some((c, s)),
        Key::Const(_) => None,
    }).collect();
    if by.is_empty() {
        return None;
    }
    by.sort_unstable();
    if trace() { eprintln!("RERAIL_KIND_TRACE keyed chosen {}", match src { Src::In(r, x) => j.src_in(r, x).0.name.get().cloned().unwrap_or_default(), Src::Delta(_) => "delta".into(), Src::Full(_) => "full".into(), Src::FullMulti(..) => "multi".into() }); }
    Some(Keyed { slot: by[0].1 })
}

/// E5's pair tests: a step asking whether a view's row holds two bound slots (`within(n, a)` with both bound, or its
/// negation), its batch's pairs asked of the source at once (`Source::holds_batch`: `within`'s interval walk).
#[derive(Clone)]
pub(super) struct Held<'a> {
    cols: [usize; 2],
    slots: [usize; 2],
    pub neg: bool,
    src: &'a dyn Source,
}

impl Held<'_> {
    /// Each of `rows` (`width` a row) answered.
    pub fn read(&self, rows: &[i64], width: usize, answers: &mut Vec<Answer>) {
        let n = rows.len() / width.max(1);
        count_keyed_for_tests(n);
        let pairs: Vec<i64> = (0..n).flat_map(|r| self.slots.map(|s| rows[r * width + s])).collect();
        let mut held = vec![0u8; n];
        let read = self.src.holds_batch(self.cols, &pairs, &mut held);
        debug_assert!(read, "a pair test's source tests batches (checked when it was chosen)");
        answers.clear();
        answers.extend(held.iter().map(|&h| match h {
            2 => Answer::Probe,
            h if (h == 1) != self.neg => Answer::Yes,
            _ => Answer::No,
        }));
    }
}

/// Step `i` of `j` as a pair test: an atom of a whole view, not fused or ranged, keyed by two slots on two columns,
/// binding and testing nothing, whose source tests pairs of those columns in batches.
pub(super) fn held(j: &JoinRule, i: usize) -> Option<Held<'_>> {
    let Step::Atom { src: Src::In(rel, ix), neg, keys, binds, tests } = &j.steps[i] else { return None };
    if j.fused[i].is_some() || j.ranges[i].is_some() || !binds.is_empty() || !tests.is_empty() {
        return None;
    }
    let &[(a, Key::Slot(sa)), (b, Key::Slot(sb))] = keys.as_slice() else { return None };
    let (rel, _) = j.src_in(rel, ix);
    let v = rel.view.as_ref()?;
    if v.ids.is_some() {
        return None;
    }
    let cols = [*v.cols.get(a)?, *v.cols.get(b)?];
    let src: &dyn Source = v.src.as_ref();
    if !src.holds_batch(cols, &[], &mut []) {
        return None;
    }
    if trace() { eprintln!("RERAIL_KIND_TRACE pairs chosen {:?} neg {neg}", rel.name.get()); }
    Some(Held { cols, slots: [sa, sb], neg: *neg, src })
}

/// A step's batch: a node read's (E1), a pair test's or a keyed probe's (E5).
#[derive(Clone)]
pub(super) enum Batched<'a> {
    Own(OwnRead<'a>),
    Held(Held<'a>),
    Keyed(Keyed),
}

/// Per step of `j`, its node read or keyed probe and an empty batch, where the walk reads batches: `on` (an unlimited
/// walk: a cut would leave rows waiting), not `RERAIL_KIND_BATCH=off`, and not the step a delta restricts (`only`:
/// its rows are the step's row ids, which a batch does not read). The step a screen reads (`screened`) batches a node
/// read or a pair test, its batch screened before it is read (`run_batch`); a keyed probe there is the walker's.
/// `RERAIL_KEYED_BATCH=off` probes each row as it comes.
pub(super) fn batches(j: &JoinRule, on: bool, screened: Option<usize>, only: Option<usize>) -> Vec<Option<(Batched<'_>, Vec<i64>)>> {
    if !on || !enabled() {
        if trace() { eprintln!("RERAIL_KIND_TRACE walk off: on {on} enabled {}", enabled()); }
        return Vec::new();
    }
    // A batch's rows wait past the steps that made them. A text the walk holds for a slot (`TEXT_OUT`, its scratch)
    // lives only while the step that made it runs: a row held after one named a text gone, and `@cat("m", x)` of a
    // function's name gave `m.`. A step after one is not batched.
    let texts_before = |i: usize| j.probes.iter().take(i).any(|&p| p & super::TEXT_OUT != 0);
    (0..j.steps.len())
        .map(|i| {
            if i == 0 || only == Some(i) || texts_before(i) {
                return None;
            }
            let keyed = |j: &JoinRule, i: usize| (screened != Some(i)).then(|| keyed(j, i)).flatten();
            let b = own_read(j, i)
                .map(Batched::Own)
                .or_else(|| keyed_enabled().then(|| held(j, i).map(Batched::Held).or_else(|| keyed(j, i).map(Batched::Keyed))).flatten())?;
            Some((b, Vec::new()))
        })
        .collect()
}

/// Whether a fused probe's batch runs its candidates as one chunk (`RERAIL_SPAN_BATCH`, not `off`).
pub(super) fn span_enabled() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    match SPAN_FORCE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_SPAN_BATCH").map_or(true, |v| v != "off"));
    *ON
}

/// Whether keyed probes are batched (`RERAIL_KEYED_BATCH`, not `off`).
pub(super) fn keyed_enabled() -> bool {
    #[cfg(any(test, feature = "test-hooks"))]
    match KEYED_FORCE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_KEYED_BATCH").map_or(true, |v| v != "off"));
    *ON
}

/// `RERAIL_KIND_BATCH=off` tests each row as it comes; `=walk` batches the walker's only, `=closure` the closures'
/// only (measurement aids). A setting of the run (decision 0442), read once a run.
fn mode() -> &'static str {
    #[cfg(any(test, feature = "test-hooks"))]
    match FORCE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => return "off",
        2 => return "",
        _ => {}
    }
    rerail_prof::code_rule_dsl::logic::profile::setting(|| match std::env::var("RERAIL_KIND_BATCH").as_deref() {
        Ok("off") => "off",
        Ok("walk") => "walk",
        Ok("closure") => "closure",
        _ => "",
    })
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
/// Tests: the rows the process's node reads read in batches.
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

/// Tests: keyed batches forced off (`Some(false)`), on (`Some(true)`), or as the environment says (`None`).
#[cfg(any(test, feature = "test-hooks"))]
static KEYED_FORCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
#[cfg(any(test, feature = "test-hooks"))]
static KEYED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(any(test, feature = "test-hooks"))]
pub fn keyed_force_for_tests(on: Option<bool>) {
    KEYED_FORCE.store(match on { Some(false) => 1, Some(true) => 2, None => 0 }, std::sync::atomic::Ordering::Relaxed);
}
/// Tests: spanning chunks forced off (`Some(false)`), on (`Some(true)`), or as the environment says (`None`).
#[cfg(any(test, feature = "test-hooks"))]
static SPAN_FORCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
#[cfg(any(test, feature = "test-hooks"))]
static SPANNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(any(test, feature = "test-hooks"))]
pub fn span_force_for_tests(on: Option<bool>) {
    SPAN_FORCE.store(match on { Some(false) => 1, Some(true) => 2, None => 0 }, std::sync::atomic::Ordering::Relaxed);
}
/// Tests: the candidate rows the process's spanning chunks ran.
#[cfg(any(test, feature = "test-hooks"))]
pub fn spanned_rows_for_tests() -> usize {
    SPANNED.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn count_spanned_for_tests(n: usize) {
    SPANNED.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(any(test, feature = "test-hooks")))]
#[inline(always)]
pub(super) fn count_spanned_for_tests(_: usize) {}
/// Tests: the rows the process's keyed probes probed in sorted batches.
#[cfg(any(test, feature = "test-hooks"))]
pub fn keyed_rows_for_tests() -> usize {
    KEYED.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(any(test, feature = "test-hooks"))]
fn count_keyed_for_tests(n: usize) {
    KEYED.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(any(test, feature = "test-hooks")))]
fn count_keyed_for_tests(_: usize) {}

/// A fused row program's pair tests asked in a batch (`rowcode::ViewTest::pair`), counted with the walk's.
pub(super) fn count_pairs_for_tests(n: usize) {
    count_keyed_for_tests(n);
}
