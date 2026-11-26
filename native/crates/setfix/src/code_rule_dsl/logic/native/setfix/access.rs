//! The walker's side of the index catalogue (decision 0706 step 1): the indexes its membership steps need
//! (`needs`), what they did with them over a run (`observe`, the run's `Observed`), and a level's build ahead
//! (`prefetch`): each index that its readers' use pays for, built in parallel into its relation's block before the
//! level runs. A reader only finds one (`Csr::filter`, `Csr::bits`); none builds one.
//!
//! E2 (membership): node bits for a test by one column (decision 0694), a filter for one by several, read before the
//! exact probe. A test the filter rejects reads no row.

use super::{InRel, JoinRule, Src, Step};
use crate::code_rule_dsl::logic::access::{self, Access, Kind, Use};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// What a run's membership steps did, by the index they would read (decision 0706 §7): summed over its chunks, read
/// by each level's build ahead. The run's own (decision 0442).
#[derive(Default)]
pub(super) struct Observed(Mutex<BTreeMap<Access, Use>>);

/// What the build batches made in the run: indexes built, by kind (node bits, filters), and the rows they read.
#[derive(Default)]
pub(super) struct Built(Mutex<[u64; 3]>);

/// Whether compiled step `i` of `j` is a membership test the walker reads an index for (`run_join_in_each`'s: not the
/// first, not a delta's, not fused or ranged, binding nothing or negated, keyed, no tests of its own columns): its
/// `Access`, and whether it is negated.
pub(super) fn needs(j: &JoinRule, i: usize) -> Option<(Access, bool)> {
    let Step::Atom { src: Src::In(rel, ix), neg, keys, binds, tests } = &j.steps[i] else { return None };
    if i == 0 || !(*neg || binds.is_empty()) || keys.is_empty() || !tests.is_empty() || j.fused[i].is_some() || j.ranges[i].is_some() {
        return None;
    }
    let (rel, ix) = j.src_in(rel, ix);
    let name = rel.name.get()?;
    if rel.view.is_some() || ix.cols.len() != keys.len() {
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
            if name == "kind" {
                eprintln!("RERAIL_KIND_TRACE steps kind first {} neg {neg} keys {} binds {} tests {} fused {} ranged {} batchable {} in {tests} out {out}", i == 0, keys.len(), binds.len(), t.len(), j.fused[i].is_some(), j.ranges[i].is_some(), super::kinds::kind_test(j, i).is_some());
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

/// The level's build ahead (decision 0706 §4, §5): of the indexes its strata's membership steps have needed so far in
/// the run, those on a relation it reads (`reads`) that the run holds complete, whose use pays for their build,
/// built now in parallel into the relations' blocks. Returns (indexes built, relations).
pub(super) fn prefetch(reads: &BTreeSet<String>, held: impl Fn(&str) -> Option<Arc<InRel>>) -> (usize, usize) {
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
    let jobs: Vec<(Arc<InRel>, Kind)> = wanted
        .into_iter()
        .filter_map(|(a, u)| {
            let rel = held(&a.rel)?;
            (rel.view.is_none() && access::pays(&a.kind, rel.n as u64, u)).then_some((rel, a.kind))
        })
        .collect();
    if jobs.is_empty() {
        return (0, 0);
    }
    let _span = crate::code_rule_dsl::logic::profile::span(|| "index builds ahead (decision 0706)".into());
    use rayon::prelude::*;
    let made: Vec<Option<(usize, u64)>> = jobs
        .par_iter()
        .map(|(rel, kind)| {
            let ib = rel.in_block()?;
            match kind {
                Kind::NodeBits(c) => {
                    let had = ib.relation().find_node_bits(*c, ib.rows).is_some();
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
    (n, rels.len())
}

/// `RERAIL_INDEX_TRACE`: the needs, wants and builds, on stderr.
fn trace() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_INDEX_TRACE").is_some());
    *ON
}

/// What the run's build batches made: node bits, filters, and the rows they read.
pub fn built() -> [u64; 3] {
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
