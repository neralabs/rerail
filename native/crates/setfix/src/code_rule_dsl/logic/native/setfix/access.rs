//! The walker's side of the index catalogue (decision 0706 step 1): the indexes its membership steps need
//! (`needs`), what they did with them over a run (`observe`, the run's `Observed`), and a level's build ahead
//! (`prefetch`): each index that its readers' use pays for, built in parallel into its relation's block before the
//! level runs. A reader only finds one (`Csr::filter`, `Csr::bits`); none builds one.
//!
//! E4 (seed scans with a constant): a view's walks of a column it keeps no index of, by chunk (`observe_walk`), priced
//! against its value index built ahead (`Source::build_value_index`).
//!
//! E3 (grouped aggregates): a scalar step's probes of its bag, and its input's rows, priced against a whole summary
//! built in one pass (`scalars::build_ahead`).
//!
//! E2 (membership): node bits for a test by one column (decision 0694), a filter for one by several, read before the
//! exact probe. A test the filter rejects reads no row.

use super::{InRel, JoinRule, Key, Src, Step};
use crate::code_rule_dsl::logic::access::{self, Access, Kind, Use};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// What a run's membership steps did, by the index they would read (decision 0706 §7): summed over its chunks, read
/// by each level's build ahead. The run's own (decision 0442).
#[derive(Default)]
pub(super) struct Observed(Mutex<BTreeMap<Access, Use>>);
impl Observed {
    /// What the run's readers of `a` have done so far.
    pub(super) fn use_of(&self, a: &Access) -> Option<Use> {
        self.0.lock().unwrap().get(a).copied()
    }
}

/// What the build batches made in the run: indexes built, by kind (node bits, filters), the rows they read,
/// views' value indexes, and orders (E5).
#[derive(Default)]
pub(super) struct Built(Mutex<[u64; 5]>);

/// Whether compiled step `i` of `j` is a membership test the walker reads an index for (`run_join_in_each`'s: not the
/// first, not a delta's, not fused or ranged, binding nothing or negated, keyed, no tests of its own columns): its
/// `Access`, and whether it is negated.
pub(super) fn needs(j: &JoinRule, i: usize) -> Option<(Access, bool)> {
    let Step::Atom { src: src @ Src::In(rel, ix), neg, keys, binds, tests } = &j.steps[i] else { return None };
    if i == 0 || !(*neg || binds.is_empty()) || keys.is_empty() || !tests.is_empty() || j.fused[i].is_some() || j.ranges[i].is_some() {
        return None;
    }
    let (rel, ix) = j.src_in(rel, ix);
    // A view's test of a bound node against a constant: its source's bits for the constant, on the view's bound name.
    if let Some(v) = &rel.view {
        let (node, col, k) = super::cands::value_bits_need(j, src, keys, tests)?;
        let of = v.of.as_deref().or(rel.name.get().map(String::as_str))?;
        return Some((Access { rel: of.to_owned(), kind: Kind::ValueBits(node as u16, col as u16, k) }, *neg));
    }
    let name = rel.name.get()?;
    if ix.cols.len() != keys.len() {
        return None;
    }
    Some((Access::membership(name, keys.iter().map(|&(c, _)| c as u16)), *neg))
}

/// Adds what `j`'s membership steps did since their counts were last taken (`counts`, per compiled step, in and out)
/// to the run's `Observed`.
pub(super) fn observe(j: &JoinRule, counts: &[(u64, u64)]) {
    let mut seen: Vec<(Access, Use)> = Vec::new();
    for (i, &(tests, out)) in counts.iter().enumerate() {
        if tests == 0 {
            continue;
        }
        if super::kinds::trace() && let Step::Atom { src, neg, keys, binds, tests: t, .. } = &j.steps[i] {
            let name = match src { Src::In(r, x) => j.src_in(r, x).0.name.get().cloned().unwrap_or_default(), Src::Delta(_) => "delta".into(), Src::Full(_) => "full".into(), Src::FullMulti(..) => "multi".into() };
            if std::env::var_os("RERAIL_STEP_FORMS").is_some() {
                let form = match super::kinds::batches(j, true, None, None).get(i) { Some(Some((super::kinds::Batched::Own(_), _))) => "own", Some(Some((super::kinds::Batched::Keyed(_), _))) => "keyed", Some(Some((super::kinds::Batched::Held(_), _))) => "pairs", _ => "walk" };
                eprintln!("RERAIL_STEP_FORM\t{name}\tkeys {:?} binds {:?} neg {neg} first {}\t{form}\t{tests}\t{out}", keys.iter().map(|&(c, k)| (c, matches!(k, Key::Slot(_)))).collect::<Vec<_>>(), binds.iter().map(|b| b.0).collect::<Vec<_>>(), i == 0);
            }
            if name == "kind" {
                eprintln!("RERAIL_KIND_TRACE steps kind first {} neg {neg} keys {} binds {} tests {} fused {} ranged {} batchable {} in {tests} out {out}", i == 0, keys.len(), binds.len(), t.len(), j.fused[i].is_some(), j.ranges[i].is_some(), super::kinds::own_read(j, i).is_some());
            }
        }
        // A scalar step over a complete input (E3): its probes, and the rows its input held, for its summary's price.
        if let Step::Scalar { src: Src::In(rel, _), bag, bound: None, .. } = &j.steps[i] {
            let rel = j.bound(rel);
            if let (Some(name), None, Some(bag)) = (rel.name.get(), rel.view.as_ref(), bag.as_ref()) {
                seen.push((Access { rel: name.clone(), kind: j.bound(bag).access() }, Use { tests, found: rel.len() as u64 }));
            }
            continue;
        }
        // A keyed probe of a held input (E5): the index it reads on its key's columns, whatever its form (the order a
        // probe makes, an index in the input's block, a kept chain): its probes, the rows its input held, and this
        // plan as a reader. The search prices the probe by it (`orders_available`); a membership test's is E2's.
        if let Step::Atom { src: Src::In(rel, ix), keys, .. } | Step::Scalar { src: Src::In(rel, ix), keys, .. } = &j.steps[i]
            && !keys.is_empty()
            && needs(j, i).is_none()
            && order_prices()
        {
            let (rel, ix) = j.src_in(rel, ix);
            if rel.view.is_none() && !ix.keyed && let Some(name) = rel.name.get() {
                let mut cols: Vec<u16> = ix.cols.iter().map(|&c| c as u16).collect();
                cols.sort_unstable();
                let a = Access { rel: name.clone(), kind: Kind::Order(cols) };
                {
                    let r = crate::code_rule_dsl::logic::run::local::<Readers>();
                    let mut r = r.0.lock().unwrap();
                    let e = r.entry(a.clone()).or_default();
                    e.0.insert(j.spec);
                    e.1 = rel.len() as u64;
                }
                seen.push((a, Use { tests, found: 0 }));
                continue;
            }
        }
        let Some((a, neg)) = needs(j, i) else {
            if trace() && let Step::Atom { src: Src::In(rel, ix), neg, binds, keys, tests } = &j.steps[i] {
                let (rel, _) = j.src_in(rel, ix);
                eprintln!("RERAIL_INDEX_TRACE not a need: step {i} rel {:?} view {} neg {neg} binds {} keys {} tests {}", rel.name.get(), rel.view.is_some(), binds.len(), keys.len(), tests.len());
            }
            continue;
        };
        let found = if neg { tests.saturating_sub(out) } else { out };
        seen.push((a, Use { tests, found: found.min(tests) }));
    }
    if seen.is_empty() {
        return;
    }
    let o = crate::code_rule_dsl::logic::run::local::<Observed>();
    let mut o = o.0.lock().unwrap();
    for (a, u) in seen {
        o.entry(a).or_default().add(u);
    }
}

/// Whether the search prices orders (decision 0706 §3; `RERAIL_ORDER_PRICES=off`, a measurement aid, prices none),
/// and so whether their readers are observed.
pub fn order_prices() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_ORDER_PRICES").map_or(true, |v| v != "off"))
}

/// The plans (by their spec's digest) that probed each order over the run, and the rows its input held when last
/// probed (E5): an order's readers and its size.
#[derive(Default)]
struct Readers(Mutex<BTreeMap<Access, (BTreeSet<u64>, u64)>>);

/// Decision 0706 §3: the orders the search prices with. Each order the run's readers have probed is scheduled: built
/// ahead of its level (`orders_ahead`) at its build over the rows its input held, shared by the plans that read it.
/// One not listed is absent, its whole build the price of the plan that would read it first.
pub fn orders_available() -> access::Available {
    let mut a = access::Available::default();
    let Some(r) = crate::code_rule_dsl::logic::run::local_if_any::<Readers>() else { return a };
    for (k, (plans, rows)) in r.0.lock().unwrap().iter() {
        a.0.insert(k.clone(), access::State::Scheduled { cost: access::build_ns(&k.kind, *rows), readers: (plans.len() as u32).max(1) });
    }
    a
}

/// The sources whose walks the run has counted, by index: a chunk's view is counted once (`Use::found`).
#[derive(Default)]
struct Walked(Mutex<std::collections::HashSet<(Access, usize)>>);

/// Decision 0706 E4: a scan of view `of` walked its source's column `col` (no index of it): a walk, and a chunk the
/// first time this source is walked.
pub(super) fn observe_walk(of: &str, col: usize, src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>) {
    let a = Access { rel: of.to_owned(), kind: Kind::Value(col as u16) };
    let new = crate::code_rule_dsl::logic::run::local::<Walked>().0.lock().unwrap().insert((a.clone(), Arc::as_ptr(src) as *const u8 as usize));
    let o = crate::code_rule_dsl::logic::run::local::<Observed>();
    o.0.lock().unwrap().entry(a).or_default().add(Use { tests: 1, found: u64::from(new) });
}

/// The level's build ahead (decision 0706 §4, §5): of the indexes its strata's membership steps have needed so far in
/// the run, those on a relation it reads (`reads`) that the run holds complete, whose use pays for their build,
/// built now in parallel into the relations' blocks. Returns (indexes built, relations).
pub(super) fn prefetch(reads: &BTreeSet<String>, held: impl Fn(&str) -> Option<Arc<InRel>>, bound: impl Fn(&str) -> Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>) -> (usize, usize) {
    if !on() {
        return (0, 0);
    }
    let Some(o) = crate::code_rule_dsl::logic::run::local_if_any::<Observed>() else { return (0, 0) };
    if trace() {
        let all = o.0.lock().unwrap();
        for (a, u) in all.iter().filter(|(a, _)| reads.contains(&a.rel)) {
            let held = held(&a.rel).map(|r| (r.n, r.view.is_some(), r.in_block().is_some()));
            eprintln!("RERAIL_INDEX_TRACE level wants {a:?} {u:?} held {held:?} pays {:?}", held.map(|(n, _, _)| access::pays(&a.kind, n as u64, *u)));
        }
    }
    let wanted: Vec<(Access, Use)> = o.0.lock().unwrap().iter().filter(|(a, _)| reads.contains(&a.rel)).map(|(a, u)| (a.clone(), *u)).collect();
    // A view's value index (E4), on the source the view is bound to: built in parallel with the relations'.
    let values: Vec<(Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, usize, Option<(usize, i64)>)> = wanted
        .iter()
        .filter_map(|(a, u)| match &a.kind {
            Kind::Value(c) if access::pays(&a.kind, 0, *u) => bound(&a.rel).map(|s| (s, usize::from(*c), None)),
            // A view's bits by a constant (the prepare of its tests), priced over the nodes its tests found.
            // The view's source: a view held in place under its name (`hold_view_in_place`, `kind` over `node`'s),
            // else the source bound to it.
            Kind::ValueBits(n, c, k) if access::pays(&a.kind, u.found, *u) => held(&a.rel)
                .and_then(|r| r.view.as_ref().map(|v| Arc::clone(&v.src)))
                .or_else(|| bound(&a.rel))
                .map(|s| (s, usize::from(*c), Some((usize::from(*n), *k)))),
            _ => None,
        })
        .collect();
    let jobs: Vec<(Arc<InRel>, Kind)> = wanted
        .into_iter()
        .filter_map(|(a, u)| {
            let rel = held(&a.rel)?;
            (rel.view.is_none() && access::pays(&a.kind, rel.n as u64, u)).then_some((rel, a.kind))
        })
        .collect();
    if jobs.is_empty() && values.is_empty() {
        return (0, 0);
    }
    let _span = crate::code_rule_dsl::logic::profile::span(|| "index builds ahead (decision 0706)".into());
    use rayon::prelude::*;
    let made_values: Vec<Option<usize>> = values.par_iter().map(|(s, c, k)| match k { None => s.build_value_index(*c), Some((n, k)) => s.build_value_bits(*n, *c, *k) }).collect();
    if crate::code_rule_dsl::prof::on() {
        for ((_, _, k), m) in values.iter().zip(&made_values) {
            if k.is_some() {
                crate::code_rule_dsl::prof::count_named_args(format_args!("view bits: built {}", if m.is_some() { "ahead" } else { "none" }), m.unwrap_or(0).max(1));
            }
        }
    }
    if trace() {
        for ((s, c, k), m) in values.iter().zip(&made_values) {
            eprintln!("RERAIL_INDEX_TRACE built value {c} {k:?} at {:p} gen {}: {m:?}", Arc::as_ptr(s), s.value_bits_gen());
        }
    }
    let made: Vec<Option<(usize, u64)>> = jobs
        .par_iter()
        .map(|(rel, kind)| {
            let ib = rel.in_block()?;
            match kind {
                Kind::NodeBits(c) => {
                    use rerail_blocks::relation::{NODE_BITS, NODE_ROWS};
                    let r = ib.relation();
                    let had = r.find_node_index(NODE_ROWS, *c, ib.rows).or_else(|| r.find_node_index(NODE_BITS, *c, ib.rows)).is_some();
                    ib.own_node_bits(*c).filter(|_| !had).map(|_| (0, rel.n as u64))
                }
                Kind::Filter(cols) => {
                    let had = ib.own_filter(cols, false).is_some();
                    ib.own_filter(cols, true).filter(|_| !had).map(|_| (1, rel.n as u64))
                }
                _ => None,
            }
        })
        .collect();
    let rels: BTreeSet<*const InRel> = jobs.iter().zip(&made).filter(|(_, m)| m.is_some()).map(|((r, _), _)| Arc::as_ptr(r)).collect();
    for ((rel, _), _) in jobs.iter().zip(&made).filter(|(_, m)| m.is_some()) {
        rel.ahead.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
    let b = crate::code_rule_dsl::logic::run::local::<Built>();
    let mut b = b.0.lock().unwrap();
    let mut n = 0;
    for (k, rows) in made.into_iter().flatten() {
        b[k] += 1;
        b[2] += rows;
        n += 1;
    }
    for rows in made_values.into_iter().flatten() {
        #[cfg(any(test, feature = "test-hooks"))]
        VALUES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        b[3] += 1;
        b[2] += rows as u64;
        n += 1;
    }
    (n, rels.len())
}

/// `RERAIL_INDEX_TRACE`: the needs, wants and builds, on stderr.
fn trace() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_INDEX_TRACE").is_some());
    *ON
}

/// What the run's build batches made: node bits, filters, the rows they read, views' value indexes, and orders.
pub fn built() -> [u64; 5] {
    crate::code_rule_dsl::logic::run::local_if_any::<Built>().map(|b| *b.0.lock().unwrap()).unwrap_or_default()
}

/// The run's observed use so far, by index (for tests and the profile).
pub fn observed() -> BTreeMap<Access, Use> {
    crate::code_rule_dsl::logic::run::local_if_any::<Observed>().map(|o| o.0.lock().unwrap().clone()).unwrap_or_default()
}

/// Whether the level's builds ahead run: `RERAIL_INDEX_AHEAD=off` builds none (a measurement aid: membership tests
/// then probe as without an index).
pub(super) fn on() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_INDEX_AHEAD").map_or(true, |v| v != "off"));
    *ON
}

/// Tests: adds `u` to the run's observed use of `a`.
#[cfg(test)]
pub(super) fn seed_observed(a: Access, u: Use) {
    crate::code_rule_dsl::logic::run::local::<Observed>().0.lock().unwrap().entry(a).or_default().add(u);
}

/// Tests: the views' value indexes the process built ahead.
#[cfg(any(test, feature = "test-hooks"))]
static VALUES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(any(test, feature = "test-hooks"))]
pub fn value_indexes_built_for_tests() -> u64 {
    VALUES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Decision 0706 E5: the orders a component's plans probe derived inputs by (`Csr::runs`: a keyed probe's held
/// order, an atom's or a scalar step's), built before its walks, in parallel, each into its relation's block as its first reader made it. No reader
/// makes one after (`count_order_made` counts those that still do). An order is wanted by its plan, not by an earlier
/// chunk's use: a reader that finds none makes it, at the same cost, inside its walk.
pub(super) fn orders_ahead<'a>(plans: impl Iterator<Item = &'a JoinRule>) -> usize {
    if !on() {
        return 0;
    }
    let mut jobs: Vec<(&InRel, &super::Csr)> = Vec::new();
    for p in plans {
        for step in p.steps.iter() {
            // An atom's probe, or a scalar step's (an aggregate's candidates by its key).
            let (Step::Atom { src: Src::In(rel, ix), keys, .. } | Step::Scalar { src: Src::In(rel, ix), keys, .. }) = step else { continue };
            if keys.is_empty() {
                continue;
            }
            let (rel, ix) = p.src_in(rel, ix);
            let ix: &super::Csr = ix;
            if ix.ordered.is_none() || ix.order.get().is_some() || jobs.iter().any(|(_, c)| std::ptr::eq(*c, ix)) {
                continue;
            }
            jobs.push((rel, ix));
        }
    }
    build_orders(&jobs)
}

/// Each of `jobs`' orders (a derived input and the index a probe of it reads) made, in parallel, as its first reader
/// would make it (`Csr::runs`), counted as built ahead. The number made.
pub(super) fn build_orders(jobs: &[(&InRel, &super::Csr)]) -> usize {
    if jobs.is_empty() {
        return 0;
    }
    let _span = crate::code_rule_dsl::logic::profile::span(|| "orders built ahead (decision 0706 E5)".into());
    use rayon::prelude::*;
    jobs.par_iter().for_each(|(rel, ix)| {
        // A one-column key's probe reads its rows by value where they are (`Csr::by_value`): that index is its order,
        // and the runs are not made for it.
        if ix.by_value_kind() && super::cands::by_value_on()
            && let Some(ib) = rel.in_block()
            && ib.own_node_rows(ix.cols[0] as u16).is_some()
        {
            rel.ahead.fetch_add(1, std::sync::atomic::Ordering::Release);
            return;
        }
        AHEAD.with(|a| a.set(true));
        let _ = ix.runs(rel);
        AHEAD.with(|a| a.set(false));
    });
    #[cfg(any(test, feature = "test-hooks"))]
    AHEAD_MADE.with(|a| a.set(a.get() + jobs.len()));
    let b = crate::code_rule_dsl::logic::run::local::<Built>();
    b.0.lock().unwrap()[4] += jobs.len() as u64;
    jobs.len()
}

thread_local! {
    /// Whether this thread is building orders ahead (`orders_ahead`).
    static AHEAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// An order made: by the build ahead, or by a reader (counted, for tests).
/// Whether a reader made it.
pub(super) fn count_order_made() -> bool {
    let reader = !AHEAD.with(std::cell::Cell::get);
    if reader {
        #[cfg(any(test, feature = "test-hooks"))]
        READERS.with(|r| r.set(r.get() + 1));
    }
    reader
}
// Counted on the thread that made the order (a reader's) or asked for the build (`build_orders`): a test's own,
// not those of tests running beside it.
#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static READERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static AHEAD_MADE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// Tests: the orders this thread built ahead of their readers.
#[cfg(any(test, feature = "test-hooks"))]
pub fn orders_built_ahead_for_tests() -> usize {
    AHEAD_MADE.with(std::cell::Cell::get)
}
/// Tests: the orders this thread's readers made in their walks.
#[cfg(any(test, feature = "test-hooks"))]
pub fn orders_made_by_readers_for_tests() -> usize {
    READERS.with(std::cell::Cell::get)
}
