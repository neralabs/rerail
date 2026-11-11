//! The walks a test counts (`Walk`): each expansion a utility-graph walk makes, recorded where it is made, on this
//! thread (test builds only).

use std::cell::RefCell;
use std::collections::BTreeMap;

/// A utility-graph walk whose work a test counts: each time it expands a utility's rule (for
/// `BindingSitePatterns`, each binding-site pattern it compiles).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Walk {
    SoundVerdict,
    OptionalCaptures,
    CaptureVariables,
    AstCaptures,
    NodeRegexes,
    OrderBinds,
    OrderReads,
    OrderMentions,
    BindingSitePatterns,
    AnchorInlining,
    /// P's: an anchor index build (P's SQL anchor engine).
    AnchorBuild,
    /// A shared utility plan's conditions evaluated by one anchor selection.
    AnchorSelection,
    /// A shared utility plan whose value-posting needs an anchor index build collects.
    AnchorNeeds,
    /// A root an anchor text selection visits: its scope, which the regex or a value posting then
    /// tests.
    AnchorTextNode,
    /// A sibling an anchor selection's sibling walk around zero-width nodes steps over, or a child
    /// it reads while building a parent's run of siblings.
    AnchorSibling,
    /// A node the Rust usage index visits to find the nodes its references span.
    UsageNode,
}

thread_local! {
    static WALKS: RefCell<BTreeMap<Walk, usize>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn record_walk(walk: Walk) {
    WALKS.with(|walks| *walks.borrow_mut().entry(walk).or_default() += 1);
}

pub(crate) fn reset_walks() {
    WALKS.with(|walks| walks.borrow_mut().clear());
}

pub(crate) fn walks(walk: Walk) -> usize {
    WALKS.with(|walks| walks.borrow().get(&walk).copied().unwrap_or_default())
}
