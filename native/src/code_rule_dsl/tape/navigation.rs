//! Thread-local algorithmic work counters of the tape's field lookups, read by the profile. No mutex or allocation
//! per lookup.
use serde::Serialize;
use std::cell::Cell;

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Counters {
    pub(crate) calls: u64,
    pub(crate) children_examined: u64,
    pub(crate) misses: u64,
    pub(crate) miss_children_examined: u64,
    pub(crate) scans_over_64: u64,
    pub(crate) max_scan: u64,
    pub(crate) known_edge_checks: u64,
    pub(crate) known_edge_matches: u64,
}
thread_local! { static COUNTERS: Cell<Counters> = const { Cell::new(Counters {
    calls: 0, children_examined: 0, misses: 0, miss_children_examined: 0,
    scans_over_64: 0, max_scan: 0, known_edge_checks: 0, known_edge_matches: 0,
}) }; }

pub(crate) fn record(examined: usize, found: bool) {
    if !enabled() {
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
pub(crate) fn record_edge(found: bool) {
    if !enabled() {
        return;
    }
    COUNTERS.with(|local| {
        let mut value = local.get();
        value.known_edge_checks += 1;
        value.known_edge_matches += u64::from(found);
        local.set(value);
    });
}
pub(crate) fn reset() {
    COUNTERS.with(|local| local.set(Counters::default()));
}

/// The counts so far on this thread, which start again from zero.
pub(crate) fn take() -> Counters {
    COUNTERS.with(|local| local.replace(Counters::default()))
}

/// Whether the profile is on (`RERAIL_PROFILE`, as `profile::trace::enabled` reads it).
fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_PROFILE").is_some());
    *ENABLED
}
