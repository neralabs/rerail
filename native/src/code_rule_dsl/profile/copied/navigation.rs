//! Thread-local algorithmic work counters. No mutex or allocation per lookup.
use serde::Serialize;
use std::cell::Cell;

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Counters {
    calls: u64,
    children_examined: u64,
    misses: u64,
    miss_children_examined: u64,
    scans_over_64: u64,
    max_scan: u64,
    known_edge_checks: u64,
    known_edge_matches: u64,
}
thread_local! { static COUNTERS: Cell<Counters> = const { Cell::new(Counters {
    calls: 0, children_examined: 0, misses: 0, miss_children_examined: 0,
    scans_over_64: 0, max_scan: 0, known_edge_checks: 0, known_edge_matches: 0,
}) }; }

pub(in crate::code_rule_dsl) fn record(examined: usize, found: bool) {
    if !super::trace::enabled() {
        return;
    }
    COUNTERS.with(|local| {
        let mut value = local.get();
        value.calls += 1;
        value.children_examined += examined as u64;
        value.misses += u64::from(!found);
        value.miss_children_examined += if found { 0 } else { examined as u64 };
        value.scans_over_64 += u64::from(examined > 64);
        value.max_scan = value.max_scan.max(examined as u64);
        local.set(value);
    });
}
pub(in crate::code_rule_dsl) fn record_edge(found: bool) {
    if !super::trace::enabled() {
        return;
    }
    COUNTERS.with(|local| {
        let mut value = local.get();
        value.known_edge_checks += 1;
        value.known_edge_matches += u64::from(found);
        local.set(value);
    });
}
pub(super) fn reset() {
    COUNTERS.with(|local| local.set(Counters::default()));
}
pub(super) fn flush() {
    let value = COUNTERS.with(|local| local.replace(Counters::default()));
    super::with_profile(|profile| {
        let target = &mut profile.field_lookups;
        target.calls += value.calls;
        target.children_examined += value.children_examined;
        target.misses += value.misses;
        target.miss_children_examined += value.miss_children_examined;
        target.scans_over_64 += value.scans_over_64;
        target.max_scan = target.max_scan.max(value.max_scan);
        target.known_edge_checks += value.known_edge_checks;
        target.known_edge_matches += value.known_edge_matches;
    });
}
