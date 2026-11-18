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
    /// The changed inputs' deltas, bound as inputs of the run (`r__minus`, `r__plus`): the maintain mode's versions
    /// of them (`super::driven_part`).
    pub(in crate::code_rule_dsl) sources: Vec<(String, Arc<dyn Source>, Option<Arc<crate::code_rule_dsl::logic::native::component::InRel>>)>,
    /// Strata maintained from deltas, and derived again.
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// Of those maintained, strata that are recursive.
    pub(in crate::code_rule_dsl) recursive: usize,
    /// Each stratum derived again, by one of its relations, and why.
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
}

impl Maintained {
    /// A part whose strata the cut runs are all derived again (no kept part to drive, or gates).
    pub(in crate::code_rule_dsl) fn again(program: lower::Lowered, strata: usize) -> Maintained {
        Maintained { program, sources: Vec::new(), maintained: 0, derived_again: strata, recursive: 0, again: Vec::new() }
    }
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
