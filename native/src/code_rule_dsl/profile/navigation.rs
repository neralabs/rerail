//! The tape's field-lookup counters (`tape::navigation`), into the profile.

pub use crate::code_rule_dsl::tape::navigation::{Counters, reset, take};

pub(super) fn flush() {
    let value = take();
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
