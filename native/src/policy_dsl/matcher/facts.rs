//! What the query engine reads from Rerail's predicates instead of running them.
//!
//! A predicate with sub-rules (`resolves.binding`, `binds`, `formatArgs`, `parse`) is split: its own
//! code decides, at one node, which nodes its sub-rules are matched on and how (the facts), and the
//! query engine matches the sub-rules there in SQL. A predicate without sub-rules is tested whole
//! (`test_predicate`). Neither path runs `AstMatcher`'s `Matcher` implementation, which remains the
//! test reference.

use super::Semantic;
use super::util::{UNKNOWN, UTIL_EXCEEDED};
use super::{AstMatcher, UtilCall, UtilLimit, Verdict, each, spend_util_step, unify_capture};
use crate::policy_dsl::literal::SubTree;
use crate::policy_dsl::scope::ScopeTable;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The facts a predicate reports, by slot. Within a slot and key, rows are in the order the
/// predicate's own code would visit them (`pos`).
pub(crate) mod slot {
    /// The predicate's own verdict at the node: absent (no match), `1` (holds) or `-1` (unknown).
    /// `binds`: one row per binding the node names, in order (the key of that binding's rows).
    /// `formatArgs` and `parse`: the row points at the parsed literal's root.
    pub(crate) const GATE: u16 = 0;
    /// A binding's site, value and annotation: one row per listed sub-rule. `flag` 0: the binding
    /// has no such node (the sub-rules fail before any is matched); 1: the node, or no node where
    /// it cannot be found in the tree.
    pub(crate) const SITE: u16 = 1;
    pub(crate) const VALUE: u16 = 2;
    pub(crate) const ANNOTATION: u16 = 3;
    /// A binding's occurrences, in order; no node where one cannot be found.
    pub(crate) const OCCURRENCE: u16 = 4;
    /// Python `binds.redefines`: the shadowed binding's site, value and annotation.
    pub(crate) const SHADOWED_SITE: u16 = 5;
    /// TypeScript and private-name `binds.redefines`: the other definitions, in order. `flag`: 1
    /// passes, 2 passes (a private name's, whose `every` passes over an excluded binding), 0 fails,
    /// -1 unknown (the search goes on), -2 unknown (the search ends there).
    pub(crate) const OTHERS: u16 = 8;
    /// An other definition's site, value and annotation (key: binding key << 20 | other's pos).
    pub(crate) const OTHER_SITE: u16 = 9;
    /// `formatArgs`: the host arguments no field consumes (`flag`: 1 known, 0 possibly).
    pub(crate) const UNUSED: u16 = 12;
    /// `formatArgs`: the template's fields no argument fills, nodes of the parsed template.
    pub(crate) const MISSING: u16 = 13;
    /// `formatArgs`: whether more unused arguments (pos 0) or missing fields (pos 1) may exist
    /// than the rows name (`flag` 1).
    pub(crate) const INCOMPLETE: u16 = 14;

    /// Key bits an other definition's position takes below its binding's key.
    pub(crate) const OTHER_BITS: u32 = 20;
}

/// What a fact row points at.
pub(crate) enum Target<'t, D: Doc> {
    None,
    Node(Node<'t, D>),
    /// A parsed literal (its root), with the file table its names are read in.
    Tree(Arc<SubTree>, Option<Arc<ScopeTable>>),
    /// A node of a parsed literal, by node id.
    TreeNode(Arc<SubTree>, usize),
}

pub(crate) struct Fact<'t, D: Doc> {
    pub(crate) slot: u16,
    pub(crate) key: i64,
    pub(crate) pos: i64,
    pub(crate) flag: i64,
    pub(crate) target: Target<'t, D>,
}

/// Facts in the order they are found.
pub(crate) struct Facts<'t, D: Doc> {
    pub(crate) rows: Vec<Fact<'t, D>>,
}

impl<'t, D: Doc> Facts<'t, D> {
    pub(crate) fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Adds a row at the next position of its slot and key; returns that position.
    pub(crate) fn push(&mut self, slot: u16, key: i64, flag: i64, target: Target<'t, D>) -> i64 {
        let pos = self
            .rows
            .iter()
            .filter(|row| row.slot == slot && row.key == key)
            .count() as i64;
        self.rows.push(Fact {
            slot,
            key,
            pos,
            flag,
            target,
        });
        pos
    }

    /// The gate row: the predicate holds (`1`) or is unknown (`-1`).
    pub(crate) fn gate(&mut self, flag: i64, target: Target<'t, D>) -> i64 {
        self.push(slot::GATE, 0, flag, target)
    }
}

/// The facts of a predicate with sub-rules at `node` in `env`: `None` for any other term. A limit
/// hit while computing them is left in `UTIL_EXCEEDED` for the caller to report.
pub(crate) fn facts<'t, D: Doc>(
    term: &AstMatcher,
    node: Node<'t, D>,
    env: &MetaVarEnv<'t, D>,
) -> Option<Facts<'t, D>> {
    let mut facts = Facts::new();
    match term {
        AstMatcher::Semantic(Semantic::Binding(test)) => test.facts(node, env, &mut facts),
        AstMatcher::Semantic(Semantic::Binds(test)) => test.facts(node, env, &mut facts),
        AstMatcher::Semantic(Semantic::FormatArgs(test)) => test.facts(env, &mut facts),
        AstMatcher::Semantic(Semantic::Parse(test)) => test.facts(node, &mut facts),
        _ => return None,
    }
    Some(facts)
}

/// Tests a predicate without sub-rules at `node` in `env`: `resolves`, `value`, `compare`,
/// `layout`, `arguments`, `capture`, `calls`, and `nthChild` without `ofRule`, whose `position`
/// gives the node's index among its parent's named children and their count (the query batch lists
/// them once per parent). One it cannot decide is unknown in a three-valued check (`three`) and no
/// match otherwise. `Err` when a limit was hit.
pub(crate) fn test_predicate<'t, D: Doc>(
    term: &AstMatcher,
    node: Node<'t, D>,
    env: MetaVarEnv<'t, D>,
    three: bool,
    position: impl FnOnce(&Node<'t, D>) -> Option<(usize, usize)>,
) -> Result<Verdict<'t, D>, UtilLimit> {
    let mut env = Cow::Owned(env);
    // A semantic term that cannot decide says so through `UNKNOWN`, as the sub-rules it runs do.
    UNKNOWN.set(false);
    let flagged = |matched: bool| {
        if matched {
            Some(true)
        } else if UNKNOWN.replace(false) {
            None
        } else {
            Some(false)
        }
    };
    let holds = match term {
        AstMatcher::Semantic(Semantic::Resolves(resolves)) => flagged(resolves.holds(&node)),
        AstMatcher::Semantic(Semantic::Value(test)) => test.holds(&node),
        AstMatcher::Compare(comparison) => comparison.holds(&env),
        AstMatcher::Semantic(Semantic::Layout(layout)) => {
            flagged(layout.match_node(node, &mut env).is_some())
        }
        AstMatcher::Semantic(Semantic::Arguments(arguments)) => {
            flagged(arguments.match_node(node, &mut env).is_some())
        }
        AstMatcher::Semantic(Semantic::Capture(name)) => {
            Some(env.to_mut().insert(name, node).is_some())
        }
        AstMatcher::Semantic(Semantic::Calls(test)) => calls(test, &node, env.to_mut()),
        AstMatcher::NthChild(nth) if nth.of_rule.is_none() => {
            Some(position(&node).is_some_and(|(index, count)| nth.index_holds(index, count)))
        }
        #[cfg(test)]
        AstMatcher::TestUnknown(regex) => (!regex.is_match(&node.text())).then_some(true),
        _ => Some(false),
    };
    if let Some(limit) = UTIL_EXCEEDED.get() {
        return Err(limit);
    }
    Ok(match holds {
        Some(true) => Verdict::Match(env.into_owned()),
        None if three => Verdict::Unknown,
        Some(false) | None => Verdict::NoMatch,
    })
}

/// `test_predicate` for a term that reads no position, in a check that is three-valued (`three`)
/// or not.
#[cfg(test)]
pub(super) fn predicate_verdict<'t, D: Doc>(
    term: &AstMatcher,
    node: Node<'t, D>,
    three: bool,
) -> Result<Verdict<'t, D>, UtilLimit> {
    test_predicate(term, node, MetaVarEnv::new(), three, |_| None)
}

/// `calls` at `node`: `None` where unknown. A reaching call binds the term's `chain` capture.
pub(super) fn calls<'t, D: Doc>(
    test: &crate::policy_dsl::calls::CallsTest,
    node: &Node<'t, D>,
    env: &mut MetaVarEnv<'t, D>,
) -> Option<bool> {
    use crate::policy_dsl::calls::{Reach, holds};
    match holds(test, node, super::calls_tables) {
        Reach::Reaches(chain) => {
            if let (Some(name), Some(chain)) = (&test.chain, chain) {
                super::export_text(env, name, &chain, node.range());
            }
            Some(true)
        }
        Reach::No => Some(false),
        Reach::Unknown => None,
    }
}

/// A parameterized call at `node`: within the utility depth, stack and step limits, the caller's
/// captures are copied into a frame (`frame` starts it: the caller's environment without captures),
/// `body` matches the utility in it with `finding.each` framed, and what the utility bound to a
/// parameter the caller left unbound comes back under the caller's name, unified with the other
/// parameters mapped to that name (`unify_capture`): conflicting outputs fail the call, and of
/// equal ones the first in the source is bound, whatever order the parameters are declared or
/// mapped in. `body` returns the frame after the match, or `Err(unknown)` where it did not match.
/// `Ok(None)`: no match (a limit hit is left in `UTIL_EXCEEDED`); `Err(())`: the utility's verdict
/// is unknown.
pub(crate) fn call<'t, D: Doc>(
    call: &UtilCall,
    env: &MetaVarEnv<'t, D>,
    frame: MetaVarEnv<'t, D>,
    body: impl FnOnce(MetaVarEnv<'t, D>) -> Result<Option<MetaVarEnv<'t, D>>, ()>,
) -> Result<Option<MetaVarEnv<'t, D>>, ()> {
    let Ok(depth) = super::enter_util_level() else {
        return Ok(None);
    };
    let matched = (|| {
        if !spend_util_step() {
            return Ok(None);
        }
        let mut frame = frame;
        for (param, caller) in &call.args {
            if unify_capture(env, std::iter::once(caller.as_str()), &mut frame, param).is_none() {
                return Ok(None);
            }
        }
        each::framed(&call.args, || body(frame))
    })();
    super::leave_util_level(depth);
    let Some(frame) = matched? else {
        return Ok(None);
    };
    let mut env = env.clone();
    let returned = |(param, caller): &(String, String)| {
        frame.get_match(param).is_some() && env.get_match(caller).is_none()
            || !frame.get_multiple_matches(param).is_empty()
                && env.get_multiple_matches(caller).is_empty()
            || frame.get_transformed(param).is_some() && env.get_transformed(caller).is_none()
    };
    if call.args.iter().any(returned) {
        let mut unified = BTreeSet::new();
        for (_, caller) in &call.args {
            if !unified.insert(caller.as_str()) {
                continue;
            }
            let params = call
                .args
                .iter()
                .filter(|(_, mapped)| mapped == caller)
                .map(|(param, _)| param.as_str());
            if unify_capture(&frame, params, &mut env, caller).is_none() {
                return Ok(None);
            }
        }
    }
    Ok(Some(env))
}

/// Whether `env` binds the enumerated capture to a range already reported, reading nodes of the
/// sub-trees `trees` (outermost first) by their host ranges.
pub(crate) fn excluded_in<D: Doc>(env: &MetaVarEnv<'_, D>, trees: &[Arc<SubTree>]) -> bool {
    if !each::active() {
        return false;
    }
    let _hosts = trees
        .iter()
        .map(|tree| each::HostRanges::enter(super::parse::host_map(tree)))
        .collect::<Vec<_>>();
    each::excluded(env)
}

/// Runs `inner` with `table` as the file table a parsed string type definition's names are read
/// in (`parse: annotation`), when there is one.
pub(crate) fn with_sub_table<T>(table: Option<&Arc<ScopeTable>>, inner: impl FnOnce() -> T) -> T {
    let _tables = table.map(|table| super::resolve::SubTables::enter(Arc::clone(table)));
    inner()
}

/// The limit a term hit since the check began, if any.
pub(crate) fn limit_hit() -> Option<UtilLimit> {
    UTIL_EXCEEDED.get()
}

/// Exports the captures `names` a parsed tree's rule bound in `inner` to `host`, as `parse` and
/// `formatArgs` do: a node capture as its text and host range; with `nested`, also a capture a
/// nested tree exported, its span mapped out of this tree.
pub(crate) fn export_tree<H: Doc, D: Doc>(
    host: &mut MetaVarEnv<'_, H>,
    inner: &MetaVarEnv<'_, D>,
    tree: &SubTree,
    names: &[String],
    nested: bool,
) {
    for name in names {
        if let Some(found) = inner.get_match(name)
            && let Some(range) = tree.host_range(found.node_id())
        {
            super::exported::export_text(host, name, &found.text(), range);
        } else if nested
            && let (Some(text), Some(span)) = (
                super::exported::exported_text(inner, name),
                super::exported::exported_span(inner, name),
            )
            && let Some(range) = tree.host_span(span)
        {
            super::exported::export_text(host, name, &text, range);
        }
    }
}

/// The environment a `host` rule runs in (see `HostTest`): `host` with the captures the tree's
/// rule bound so far in `inner` exported into it.
pub(crate) fn host_env<H: Doc, D: Doc>(
    host: &mut MetaVarEnv<'_, H>,
    inner: &MetaVarEnv<'_, D>,
    tree: &SubTree,
) {
    for (name, text, range) in super::parse::host_exports(inner, tree) {
        super::exported::export_text(host, &name, &text, range);
    }
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn outputs_mapped_to_one_capture_unify_in_any_parameter_order() {
        // `P` and `Q` both return to `X`: `f(g)` binds them to different nodes, so the call fails
        // whichever parameter is declared or mapped first; `h(h)` binds equal ones.
        let source = "f(g);\nh(h);\n";
        for (params, mapping) in [
            (json!(["P", "Q"]), json!({ "P": "$X", "Q": "$X" })),
            (json!(["Q", "P"]), json!({ "Q": "$X", "P": "$X" })),
        ] {
            let rule = compiled(json!({
                "rule": { "kind": "call_expression", "matches": { "pair": mapping } },
                "utils": { "pair": { "params": params, "rule": { "pattern": "$P($Q)" } } },
            }));
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            assert_eq!(matched_texts(&result, source), ["h(h) X=h"], "{params}");
            // Of the equal outputs, `X` is the one first in the source (the callee).
            let capture = &result["findings"][0]["captures"]["X"][0];
            assert_eq!(capture["startByte"], json!(6), "{params}: {result}");
        }
    }
}
