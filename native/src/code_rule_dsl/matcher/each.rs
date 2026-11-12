//! `violation.each` (a Rerail extension): one violation per distinct binding of a capture at one root.
//! The root is matched again with each binding found so far excluded: a relation's search and an
//! `any` pass over a candidate that would bind the capture to an excluded range, so the next match
//! binds another. Negations, counts, stop rules and `nthChild` probes read their own bindings and
//! exclude nothing.
//!
//! A binding is its source range in the host file. A capture a parsed sub-tree exported (M11a's
//! `parse`) is bound in the host as a reserved span variable, and inside the sub-tree to a node of
//! the sub-tree's own document: a matcher that runs a sub-tree's rule enters `HostRanges` so that
//! exclusion reads the sub-tree's nodes by their host ranges.

use super::exported::exported_span;
use crate::code_rule_dsl::origin::Origin;
use ast_grep_core::Doc;
use ast_grep_core::meta_var::MetaVarEnv;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

thread_local! {
    /// Whether an enumeration is excluding bindings here (false inside a negation or count).
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The enumerated capture and the ranges it was bound to so far, as `(start, end)`.
    static EXCLUDED: RefCell<(String, HashSet<(usize, usize)>)> =
        RefCell::new((String::new(), HashSet::new()));
    /// The sub-trees being matched, innermost last: each's bytes in the document it was parsed
    /// from.
    static HOSTS: RefCell<Vec<Arc<dyn Origin>>> = const { RefCell::new(Vec::new()) };
    /// Bumped whenever the excluded ranges change: what a memoized verdict read of them.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// What an excluding verdict may read of the enumeration here: `(0, 0)` where nothing is excluded,
/// else the generation of the excluded ranges and the excluded capture's name, hashed.
pub(super) fn exclusion_key() -> (u64, u64) {
    if !ACTIVE.get() {
        return (0, 0);
    }
    EXCLUDED.with_borrow(|(name, _)| {
        (
            GENERATION.get(),
            std::hash::BuildHasher::hash_one(&rustc_hash::FxBuildHasher, name.as_str()),
        )
    })
}

/// Whether bindings of `name` are being excluded here.
pub(super) fn excludes(name: &str) -> bool {
    ACTIVE.get() && EXCLUDED.with_borrow(|(excluded, _)| excluded == name)
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    pub static EXCLUSION_PROBES: Cell<usize> = const { Cell::new(0) };
}

/// A sub-tree's rule being matched: while it lives, bindings read through its coordinates. `parse`
/// and `formatArgs`' `missing` (M11a) enter one around their sub-tree's match.
pub struct HostRanges;

impl HostRanges {
    pub fn enter(tree: Arc<dyn Origin>) -> Self {
        HOSTS.with_borrow_mut(|hosts| hosts.push(tree));
        Self
    }
}

impl Drop for HostRanges {
    fn drop(&mut self) {
        HOSTS.with_borrow_mut(|hosts| hosts.pop());
    }
}

/// Whether bindings are being excluded here.
pub(super) fn active() -> bool {
    ACTIVE.get()
}

/// A capture's binding as a range of the host file: a node capture's range, mapped out of the
/// sub-trees being matched, or an exported capture's host range.
pub fn binding_range<D: Doc>(env: &MetaVarEnv<'_, D>, name: &str) -> Option<Range<usize>> {
    match env.get_match(name) {
        Some(node) => HOSTS.with_borrow(|hosts| {
            let Some((innermost, outer)) = hosts.split_last() else {
                return Some(node.range());
            };
            // A binding is one range: where it starts in the file (`FileRanges::primary`).
            let range = innermost.of_node(node.node_id(), node.range())?.primary();
            outer
                .iter()
                .rev()
                .try_fold(range, |range, host| Some(host.of_range(range)?.primary()))
        }),
        None => {
            let range = exported_span(env, name)?;
            HOSTS.with_borrow(|hosts| {
                hosts
                    .iter()
                    .rev()
                    .try_fold(range, |range, host| Some(host.of_range(range)?.primary()))
            })
        }
    }
}

/// Whether the bindings bind the enumerated capture to a range already reported.
pub(super) fn excluded<D: Doc>(env: &MetaVarEnv<'_, D>) -> bool {
    EXCLUDED.with_borrow(|(name, ranges)| {
        let Some(range) = binding_range(env, name) else {
            return false;
        };
        #[cfg(any(test, feature = "test-hooks"))]
        EXCLUSION_PROBES.set(EXCLUSION_PROBES.get() + 1);
        ranges.contains(&(range.start, range.end))
    })
}

/// Runs `inner` with exclusion suspended: its bindings are its own.
#[inline]
pub(super) fn suspended<T>(inner: impl FnOnce() -> T) -> T {
    if !ACTIVE.get() {
        return inner();
    }
    let saved = ACTIVE.replace(false);
    let result = inner();
    ACTIVE.set(saved);
    result
}

/// Runs a parameterized call's utility (`inner`) in its frame: the enumerated capture is excluded
/// there under the parameter it is passed as, and nothing is excluded where it is not passed.
#[inline]
pub(super) fn framed<T>(args: &[(String, String)], inner: impl FnOnce() -> T) -> T {
    if !ACTIVE.get() {
        return inner();
    }
    let param = EXCLUDED.with_borrow(|(name, _)| {
        args.iter()
            .find(|(_, caller)| caller == name)
            .map(|(param, _)| param.clone())
    });
    let Some(param) = param else {
        return suspended(inner);
    };
    let caller = EXCLUDED.with_borrow_mut(|(name, _)| std::mem::replace(name, param));
    let result = inner();
    EXCLUDED.with_borrow_mut(|(name, _)| *name = caller);
    result
}

/// An enumeration in progress; dropping it ends the exclusion, also on an early return.
pub(super) struct Enumeration;

impl Enumeration {
    pub(super) fn start(name: &str) -> Self {
        EXCLUDED.with_borrow_mut(|(known, ranges)| {
            known.clear();
            known.push_str(name);
            ranges.clear();
        });
        GENERATION.set(GENERATION.get() + 1);
        ACTIVE.set(true);
        Self
    }

    /// Records a match's binding; `false` where it binds nothing new (unbound or already seen),
    /// which ends the enumeration.
    pub(super) fn record(&self, range: Option<Range<usize>>) -> bool {
        let Some(range) = range else {
            return false;
        };
        let inserted =
            EXCLUDED.with_borrow_mut(|(_, ranges)| ranges.insert((range.start, range.end)));
        if inserted {
            GENERATION.set(GENERATION.get() + 1);
        }
        inserted
    }
}

impl Drop for Enumeration {
    fn drop(&mut self) {
        ACTIVE.set(false);
        EXCLUDED.with_borrow_mut(|(_, ranges)| ranges.clear());
        GENERATION.set(GENERATION.get() + 1);
    }
}

// cratify: `tests` is rerail-engine's (it mounts it)
