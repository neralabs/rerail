//! One clause's rule run over one file: its candidate roots in preorder, each matched by the
//! grammar matcher (`GrammarMatcher::match_rule`) within the clause's utility budget and depth
//! limits (`begin_clause`), with `violation.each` enumerated at a matching root. Utility verdicts are
//! remembered for the whole file while the caller holds `memo::with_file`.

use super::resolve::{decide_file_jsx_runtime, decide_file_source_type};
use super::{EachScope, GrammarMatcher, UtilLimit, begin_clause, end_clause, util_limit_hit};
use crate::code_rule_dsl::budget::matcher_budget_observed;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{AstGrep as Root, Node};

/// Why a clause stopped before its roots ran out.
pub(in crate::code_rule_dsl) enum Stopped {
    /// A term hit a utility limit: the clause's verdicts are not the matcher's.
    Limit(UtilLimit),
    /// The file's deadline passed (or its workspace closed): the caller records the exhaustion.
    Deadline,
}

/// The node at a preorder ordinal of `file`.
pub(in crate::code_rule_dsl) fn node_at(file: &Root<TapeDoc>, ordinal: u32) -> Node<'_, TapeDoc> {
    file.adopt(file.root().get_doc().node(ordinal))
}

/// A clause's candidate roots in preorder: `node` alone, else the roots an anchor index
/// `selected`, else every node; each of the clause's root kinds.
fn roots<'f>(
    file: &'f Root<TapeDoc>,
    matcher: &GrammarMatcher,
    node: Option<u32>,
    selected: Option<&'f [u32]>,
) -> Box<dyn Iterator<Item = u32> + 'f> {
    let tape = file.root().get_doc();
    let kinds = matcher.anchors.root_kinds().cloned();
    let of_kind = move |node: &u32| {
        kinds
            .as_ref()
            .is_none_or(|kinds| kinds.contains(&tape.transducer_kind(*node)))
    };
    match (node, selected, matcher.anchors.root_kinds()) {
        (Some(node), _, _) => Box::new(
            std::iter::once(node)
                .filter(move |&node| (node as usize) < tape.transducer_len())
                .filter(of_kind),
        ),
        (None, Some(selected), _) => Box::new(selected.iter().copied().filter(of_kind)),
        (None, None, Some(kinds)) => Box::new(super::memo::nodes_of_kinds(file, kinds).into_iter()),
        (None, None, None) => Box::new(0..tape.transducer_len() as u32),
    }
}

#[cfg(test)]
thread_local! {
    static ROOTS_MATCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Roots this thread matched (`reset_roots_matched` restarts the count).
#[cfg(test)]
pub(crate) fn roots_matched() -> usize {
    ROOTS_MATCHED.get()
}

#[cfg(test)]
pub(crate) fn reset_roots_matched() {
    ROOTS_MATCHED.set(0);
}

/// Matches `node` in a clause `begin_clause` set up: its bindings where it matches; the stop where
/// a limit or the deadline ended the clause.
fn match_root<'f>(
    file: &'f Root<TapeDoc>,
    matcher: &GrammarMatcher,
    node: u32,
) -> Result<Option<MetaVarEnv<'f, TapeDoc>>, Stopped> {
    #[cfg(test)]
    ROOTS_MATCHED.set(ROOTS_MATCHED.get() + 1);
    let found = matcher.match_rule(node_at(file, node));
    if util_limit_hit() {
        // `end_clause` reports which.
        return Ok(None);
    }
    if matcher_budget_observed() {
        return Err(Stopped::Deadline);
    }
    Ok(found.map(|found| found.get_env().clone()))
}

/// Runs `matcher`'s clause over `file` with `work` left of its utility budget, until `visit`
/// (given each matching root and its bindings) ends it. Roots at preorder ordinals from `bound`
/// on are not matched. The clause's context is set up only while roots are matched: `visit` runs
/// outside it, with the budget it left in `work`. A file whose source type its query's `when`
/// does not list (`GrammarMatcher::may_apply_to`, decided here once its syntax is read), or whose
/// JSX runtime its `file.jsxRuntime` does not, matches nothing.
pub(in crate::code_rule_dsl) fn run_clause<'f, E>(
    file: &'f Root<TapeDoc>,
    matcher: &GrammarMatcher,
    selected: Option<&'f [u32]>,
    bound: usize,
    work: &mut usize,
    mut visit: impl FnMut(u32, MetaVarEnv<'f, TapeDoc>, &mut usize) -> Result<bool, E>,
) -> Result<Result<(), Stopped>, E> {
    if !matcher.may_apply_to(Some(decide_file_source_type(&file.root()))) {
        return Ok(Ok(()));
    }
    if let Some(runtimes) = matcher
        .file
        .as_ref()
        .and_then(|condition| condition.jsx_runtime.as_ref())
        && !runtimes.contains(&decide_file_jsx_runtime(&file.root()))
    {
        return Ok(Ok(()));
    }
    begin_clause(matcher, *work);
    for root in roots(file, matcher, None, selected).take_while(|&root| (root as usize) < bound) {
        let found = match match_root(file, matcher, root) {
            Ok(found) => found,
            Err(stopped) => {
                *work = end_clause().0;
                return Ok(Err(stopped));
            }
        };
        if util_limit_hit() {
            break;
        }
        let Some(env) = found else {
            continue;
        };
        let (left, limit) = end_clause();
        *work = left;
        if let Some(limit) = limit {
            return Ok(Err(Stopped::Limit(limit)));
        }
        if !visit(root, env, work)? {
            return Ok(Ok(()));
        }
        begin_clause(matcher, *work);
    }
    let (left, limit) = end_clause();
    *work = left;
    Ok(limit.map_or(Ok(()), |limit| Err(Stopped::Limit(limit))))
}

/// `violation.each` at one root (`run_clause`'s `node`, its bindings `first`): each match is passed
/// to `visit`, then the root is matched again with every binding of `name` recorded so far
/// excluded, until a match binds nothing new or `visit` returns false (which this returns). A match
/// that leaves the capture unbound is visited only as the first. `visit` runs with nothing
/// excluded: its own matching (a transform's) reads its bindings as a clause's does.
pub(in crate::code_rule_dsl) fn enumerate<'f>(
    file: &'f Root<TapeDoc>,
    matcher: &GrammarMatcher,
    node: u32,
    first: MetaVarEnv<'f, TapeDoc>,
    work: &mut usize,
    name: &str,
    mut visit: impl FnMut(MetaVarEnv<'f, TapeDoc>, &mut usize) -> Result<bool, Stopped>,
) -> Result<bool, Stopped> {
    let scope = EachScope::start(name);
    let mut matched = first;
    let mut visited = false;
    loop {
        let fresh = scope.record(&matched, name);
        if (fresh || !visited) && !super::each::suspended(|| visit(matched, work))? {
            return Ok(false);
        }
        visited = true;
        if !fresh {
            return Ok(true);
        }
        begin_clause(matcher, *work);
        let next = roots(file, matcher, Some(node), None)
            .next()
            .map(|root| match_root(file, matcher, root))
            .transpose();
        let (left, limit) = end_clause();
        *work = left;
        if let Some(limit) = limit {
            return Err(Stopped::Limit(limit));
        }
        let Some(next) = next?.flatten() else {
            return Ok(true);
        };
        matched = next;
    }
}
