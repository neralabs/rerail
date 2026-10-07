mod anchors;
mod arguments;
mod binding;
mod binds;
mod captures;
mod collate;
mod compare;
mod count;
mod each;
mod exported;
pub(super) mod facts;
mod format_args;
mod layout;
mod nth_child;
mod parse;
mod polarity;
mod relation;
mod resolve;
mod semantic;
#[cfg(test)]
mod three_valued_tests;
mod transform;
mod util;
mod value;

#[cfg(test)]
use super::budget;
use crate::policy_dsl::language::GrammarDialect;
pub(super) use anchors::{AnchorPlan, FileAnchorIndex, Selection};
pub(super) use arguments::{Arguments, SlotKind};
#[cfg(test)]
use ast_grep_core::matcher::Matcher;
use ast_grep_core::matcher::{KindMatcher, Pattern};
#[cfg(test)]
use ast_grep_core::matcher::{MatcherExt, NodeMatch};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{Doc, Node};
pub(super) use binding::BindingTest;
pub(super) use binds::{BindingCount, BindingFilter, Binds, Redefines, ScopeName};
pub(super) use captures::{Capture, CaptureAnalysis, collect_variables, parse_exported};
pub(super) use compare::{Collation, CompareOp, CompareText, Comparison, SemanticText};
pub(super) use count::Count;
pub(crate) use each::binding_range;
pub(crate) use exported::{WITNESS_NODE, export_text, exported_span, is_span_variable};
pub(crate) use format_args::{FormatArgs, Style};
pub(crate) use layout::{Gap, Layout, LineCount, Neighbour};
pub(super) use nth_child::{NthChild, Position as NthPosition, parse_position};
pub(crate) use parse::{HostTest, ParseTest};
pub(crate) use polarity::calls_positive_only;
use regex::Regex;
pub(super) use relation::{FieldRelation, StopBy, stop_rule};
#[cfg(test)]
pub(super) use resolve::TABLE_BUILDS;
pub(super) use resolve::{FileScopeSlot, Resolves, Unresolved};
pub(crate) use resolve::{
    calls_tables, file_eslint_directives, file_lines, file_noqa, file_phpcs, file_phpstan,
    file_psalm, table_reading,
};
pub(super) use semantic::Semantic;
#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::collections::BTreeSet;
use std::sync::Arc;
pub(super) use transform::{Separator, Source, StringCase, Trans, Transform};

#[cfg(test)]
pub(crate) use transform::{
    line_table_constructions, note_line_table_construction, reset_line_table_constructions,
};
pub(super) use util::{
    CallArgs, MAX_UTIL_INSTANCES, UtilCall, UtilLimit, UtilRef, UtilTable, UtilitySummary,
};
use util::{
    MAX_UTIL_DEPTH, MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE,
    UTIL_STEPS_LEFT, stack_address,
};
use util::{THREE_VALUED, UNKNOWN};
pub(super) use value::{Bound, Reading, ValueTest};

#[derive(Clone)]
pub(super) struct GrammarMatcher {
    pub(super) language: GrammarDialect,
    pub(super) matcher: AstMatcher,
    pub(super) anchors: Arc<anchors::AnchorPlan>,
    /// Whether witnesses outside syntax-error regions stay sound; see `sound_under_errors`.
    pub(super) sound_under_errors: bool,
    /// Keeps the query's utility rules alive (`AstMatcher::Util` holds only a weak reference)
    /// and lets finding compilation walk them.
    pub(super) utils: Arc<UtilTable>,
    /// The query's `file` condition: a file whose module path fails it gets no matcher.
    pub(super) file: Option<Arc<crate::policy_dsl::language::FileCondition>>,
    /// Whether the query's `when` gate holds. A query outside it is compiled in full, so its
    /// captures and templates are checked, but matches no file: sound, not unknown.
    pub(super) applies: bool,
    /// Whether the matcher can reach an unknown verdict; see `reads_values`.
    pub(super) three_valued: bool,
    /// The query's `transform`, applied to each match after its rule and constraints.
    pub(super) transform: Option<Arc<Transform>>,
    /// The index of this matcher's compiled statement in its policy's statement table
    /// (`query::PolicyStatements`), which the policy compiler builds and evaluation is given.
    pub(super) statement: usize,
}

/// Whole-root matching: production runs compiled statements (plan step 9); these entry points
/// remain the test reference the statements are checked against.
#[cfg(test)]
impl GrammarMatcher {
    /// Matches one root. Each utility evaluation spends one step of `work`, the caller's remaining
    /// utility budget for this file; like a traversal it is bounded by `maxVisitedNodes`. A root
    /// whose verdict is unknown is not a match.
    pub(super) fn match_root<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        work: &mut usize,
    ) -> std::result::Result<Option<NodeMatch<'tree, D>>, UtilLimit> {
        UTIL_DEPTH.set(0);
        UTIL_EXCEEDED.set(None);
        UTIL_STACK_BASE.set(stack_address());
        UTIL_STEPS_LEFT.set(*work);
        THREE_VALUED.set(self.three_valued);
        UNKNOWN.set(false);
        resolve::CURRENT_CHECK.set(self as *const Self as usize);
        let mut matched = self.matcher.match_node(node);
        if let (Some(transform), Some(matched)) = (&self.transform, &mut matched) {
            transform
                .apply(matched.get_env_mut())
                .map_err(|_| UtilLimit::Work)?;
        }
        UTIL_STACK_BASE.set(0);
        *work = UTIL_STEPS_LEFT.replace(usize::MAX);
        match UTIL_EXCEEDED.take() {
            Some(limit) => Err(limit),
            None => Ok(matched),
        }
    }
}

/// A query's top-level rule and its `constraints`, as ast-grep-config's `RuleCore`: after the rule
/// matches, each constrained metavariable bound to a single node must also match its rule, applied
/// to the captured node with the same environment. Unbound and multi-node captures are not checked.
#[derive(Clone)]
pub(super) struct Constrained {
    pub(super) rule: AstMatcher,
    pub(super) constraints: Vec<(String, AstMatcher)>,
}

#[derive(Clone)]
pub(super) enum AstMatcher {
    Pattern(Pattern),
    Kind(KindMatcher),
    Regex(Regex),
    All(Vec<AstMatcher>),
    Any(Vec<AstMatcher>),
    Not(Box<AstMatcher>),
    Inside(Box<AstMatcher>, StopBy),
    Has(Box<AstMatcher>, StopBy),
    Precedes(Box<AstMatcher>, StopBy),
    Follows(Box<AstMatcher>, StopBy),
    /// `inside` or `has` with a `field`: the relation's path enters through that field.
    Field(Box<FieldRelation>),
    /// `nthChild`: the node's position among its parent's named children.
    NthChild(Box<NthChild>),
    /// `matches`: a utility rule of the same query. It shares the caller's environment, as
    /// ast-grep's local utilities do, and may recurse through relations.
    Util(UtilRef),
    /// `matches` with parameters: a utility run in a frame of its own (`UtilCall`).
    Call(Box<UtilCall>),
    Constrained(Box<Constrained>),
    Compare(Box<Comparison>),
    Count(Box<Count>),
    /// Rerail's semantic predicates (scopes, bindings, values, parsed literals, layout, calls):
    /// the families the structural rule language delegates to (`Semantic`).
    Semantic(Semantic),
    /// A sub-rule this grammar cannot express (a JSX kind under the plain-TypeScript dialect).
    Never,
    /// Test-only leaf (`testUnknown`): unknown where the node's text matches, true elsewhere.
    #[cfg(test)]
    TestUnknown(Regex),
}

/// The matcher: the test reference compiled statements are checked against. Production runs
/// every rule as a statement (`query`), and Rerail's predicates through `facts`.
#[cfg(test)]
impl Matcher for AstMatcher {
    fn match_node_with_env<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        if budget::matcher_call_budget_exhausted() {
            return None;
        }
        match self {
            Self::Pattern(pattern) => pattern.match_node_with_env(node, env),
            Self::Kind(kind) => kind.match_node_with_env(node, env),
            // RegexMatcher uses exactly this engine and node-text operation.
            Self::Regex(regex) => regex.is_match(&node.text()).then_some(node),
            // Composites and relations follow ast-grep-config 0.45.3: one environment threads
            // through every sub-rule, is committed only when the whole composite matches, and a
            // relation binds from the first related node that matches (no backtracking). Verdicts
            // are three-valued (`UNKNOWN`): `all` (a rule object's keys too) is false when a term
            // is false, whatever order its terms run in. Past an unknown term the later terms
            // still run and a false one decides, but a term that mentions a capture an unknown
            // term would have bound cannot be read without it: it stays open, and what it binds is
            // withheld from the terms after it as well.
            Self::All(matchers) => {
                let mut trial = Cow::Borrowed(env.as_ref());
                let mut unknown = false;
                let mut withheld: Option<BTreeSet<String>> = None;
                let mut captures = CaptureAnalysis::default();
                for matcher in matchers {
                    if let Some(names) = withheld.as_mut()
                        && !captures.mentions(matcher).is_disjoint(names)
                    {
                        names.extend(captures.binds(matcher));
                        continue;
                    }
                    if matcher
                        .match_node_with_env(node.clone(), &mut trial)
                        .is_none()
                    {
                        if THREE_VALUED.get() && UNKNOWN.get() {
                            UNKNOWN.set(false);
                            unknown = true;
                            if !matcher.binds_nothing() {
                                withheld
                                    .get_or_insert_default()
                                    .extend(captures.binds(matcher));
                            }
                            continue;
                        }
                        return None;
                    }
                }
                if unknown {
                    UNKNOWN.set(true);
                    return None;
                }
                if let Cow::Owned(bound) = trial {
                    *env = Cow::Owned(bound);
                }
                Some(node)
            }
            // Each branch starts from the bindings so far; only the matching branch's commit. The
            // verdict is unknown when no branch matches and some branch was unknown. A branch whose
            // verdict is unknown and that may bind ends the search unknown, as a strict relation
            // search does: a later branch must not substitute its captures for the ones it would
            // have bound. An unknown branch that binds nothing lets later branches decide.
            Self::Any(matchers) => {
                let three = THREE_VALUED.get();
                let saved = three && UNKNOWN.replace(false);
                let mut unknown = false;
                for matcher in matchers {
                    let mut trial = Cow::Borrowed(env.as_ref());
                    if matcher
                        .match_node_with_env(node.clone(), &mut trial)
                        .is_some()
                        && !(each::active() && each::excluded(&trial))
                    {
                        if three {
                            UNKNOWN.set(saved);
                        }
                        if let Cow::Owned(bound) = trial {
                            *env = Cow::Owned(bound);
                        }
                        return Some(node);
                    }
                    if three && UNKNOWN.replace(false) {
                        unknown = true;
                        if may_bind(matcher) {
                            break;
                        }
                    }
                }
                if three {
                    UNKNOWN.set(saved || unknown);
                }
                None
            }
            // Reads the bindings so far but never contributes any. The negation of an unknown
            // verdict is unknown: it fails, and stays unknown.
            Self::Not(matcher) => {
                let three = THREE_VALUED.get();
                let saved = three && UNKNOWN.replace(false);
                let mut probe = Cow::Borrowed(env.as_ref());
                let matched = each::suspended(|| {
                    matcher
                        .match_node_with_env(node.clone(), &mut probe)
                        .is_some()
                });
                if matched {
                    if three {
                        UNKNOWN.set(saved);
                    }
                    None
                } else if three && UNKNOWN.replace(saved) {
                    UNKNOWN.set(true);
                    None
                } else {
                    Some(node)
                }
            }
            Self::Inside(matcher, stop) => stop
                .find(|| node.parent(), || node.ancestors(), matcher, env)
                .map(|_| node),
            // Every child, named or anonymous. With a stop rule, pre-order like `end`, but a
            // descendant the stop rule matches is tested and not entered (ast-grep-config `Has`,
            // `relational_rule.rs:185`).
            Self::Has(matcher, stop) => relation::has(&node, matcher, stop, env).map(|_| node),
            // Siblings only, named or anonymous: `neighbor` is the adjacent sibling, `end` every
            // sibling in that direction. `next_all`/`prev_all` are ast-grep's own sibling walks.
            Self::Precedes(matcher, stop) => stop
                .find(|| node.next(), || node.next_all(), matcher, env)
                .map(|_| node),
            Self::Follows(matcher, stop) => stop
                .find(|| node.prev(), || node.prev_all(), matcher, env)
                .map(|_| node),
            Self::Field(relation) => relation.find(&node, env).map(|_| node),
            Self::Util(util) => {
                if UTIL_EXCEEDED.get().is_some() {
                    return None;
                }
                let depth = UTIL_DEPTH.get();
                let base = UTIL_STACK_BASE.get();
                let stack = if base == 0 {
                    0
                } else {
                    base.abs_diff(stack_address())
                };
                if depth >= MAX_UTIL_DEPTH || stack > MAX_UTIL_STACK_BYTES {
                    UTIL_EXCEEDED.set(Some(UtilLimit::Depth));
                    return None;
                }
                if !spend_util_step() {
                    return None;
                }
                UTIL_DEPTH.set(depth + 1);
                let matched = util
                    .with_target(|rule| rule.match_node_with_env(node, env))
                    .flatten();
                UTIL_DEPTH.set(depth);
                matched
            }
            Self::Constrained(core) => {
                let mut trial = Cow::Borrowed(env.as_ref());
                core.rule.match_node_with_env(node.clone(), &mut trial)?;
                for (name, constraint) in &core.constraints {
                    let Some(captured) = trial.get_match(name).cloned() else {
                        continue;
                    };
                    constraint.match_node_with_env(captured, &mut trial)?;
                }
                if let Cow::Owned(bound) = trial {
                    *env = Cow::Owned(bound);
                }
                Some(node)
            }
            Self::Semantic(semantic) => semantic.match_node(node, env),
            Self::Compare(comparison) => match comparison.holds(env) {
                Some(true) => Some(node),
                Some(false) => None,
                None => {
                    UNKNOWN.set(true);
                    None
                }
            },
            Self::Count(_) | Self::NthChild(_) | Self::Call(_) => self.match_extension(node, env),
            Self::Never => None,
            #[cfg(test)]
            Self::TestUnknown(regex) => {
                if regex.is_match(&node.text()) {
                    UNKNOWN.set(true);
                    None
                } else {
                    Some(node)
                }
            }
        }
    }
}

impl AstMatcher {
    /// Whether matching never adds a binding: terms that only read the node or the bindings so
    /// far, and composites and relations of them. A pattern, a utility and a constrained rule may
    /// bind, so they count as binding.
    fn binds_nothing(&self) -> bool {
        match self {
            Self::Semantic(semantic) => semantic.binds_nothing(),
            Self::Kind(_)
            | Self::Regex(_)
            | Self::Not(_)
            | Self::Compare(_)
            | Self::Count(_)
            | Self::Never => true,
            #[cfg(test)]
            Self::TestUnknown(_) => true,
            Self::All(items) | Self::Any(items) => items.iter().all(Self::binds_nothing),
            Self::Inside(item, _)
            | Self::Has(item, _)
            | Self::Precedes(item, _)
            | Self::Follows(item, _) => item.binds_nothing(),
            Self::Field(relation) => relation.rule.binds_nothing(),
            Self::NthChild(nth) => nth.of_rule.as_ref().is_none_or(Self::binds_nothing),
            Self::Pattern(_) | Self::Util(_) | Self::Call(_) | Self::Constrained(_) => false,
        }
    }

    /// The Rerail extensions, out of line: a debug build gives every arm of `match_node_with_env`
    /// its own stack slots, and utilities recurse through that frame.
    #[cfg(test)]
    #[inline(never)]
    fn match_extension<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        match self {
            Self::Count(count) => each::suspended(|| count.holds(&node, env)).then_some(node),
            Self::NthChild(nth) => nth.match_node(node, env),
            Self::Call(call) => {
                let called = facts::call(call, env, MetaVarEnv::new(), |frame| {
                    let mut frame = Cow::Owned(frame);
                    let matched = call
                        .util
                        .with_target(|rule| rule.match_node_with_env(node.clone(), &mut frame))
                        .flatten();
                    match matched {
                        Some(_) => Ok(Some(frame.into_owned())),
                        None if THREE_VALUED.get() && UNKNOWN.get() => Err(()),
                        None => Ok(None),
                    }
                });
                match called {
                    Ok(Some(bound)) => {
                        *env = Cow::Owned(bound);
                        Some(node)
                    }
                    Ok(None) => None,
                    Err(()) => {
                        UNKNOWN.set(true);
                        None
                    }
                }
            }
            _ => None,
        }
    }
}

/// The whole text of the file a host node belongs to, and its language; `None` for a node of a
/// parsed literal's tree.
pub(crate) fn host<'t, D: Doc>(
    node: &ast_grep_core::Node<'t, D>,
) -> Option<(&'t str, GrammarDialect)> {
    let doc: &'t dyn std::any::Any = node.get_doc();
    if let Some(doc) = doc.downcast_ref::<ast_grep_core::tree_sitter::StrDoc<GrammarDialect>>() {
        return Some((doc.get_source().as_str(), *doc.get_lang()));
    }
    let doc = doc.downcast_ref::<crate::policy_dsl::tape::TapeDoc>()?;
    Some((doc.get_source().as_str(), *doc.get_lang()))
}

/// The line table of the file a host node belongs to: its tape document's, which the evaluator
/// built once for the file. `None` for a node of a parsed literal's tree.
pub(crate) fn host_lines<'t, D: Doc>(
    node: &ast_grep_core::Node<'t, D>,
) -> Option<&'t crate::policy_dsl::lines::LineTable> {
    let doc: &'t dyn std::any::Any = node.get_doc();
    Some(
        doc.downcast_ref::<crate::policy_dsl::tape::TapeDoc>()?
            .line_table(),
    )
}

/// Whether a matcher can reach an unknown verdict: it holds a `value` predicate or a comparison over
/// values or property names, directly or through a composite, relation, stop rule, count,
/// constraint or utility. Checks without one never touch the `UNKNOWN` flag.
pub(super) fn reads_values(
    matcher: &AstMatcher,
    seen: &mut std::collections::BTreeSet<usize>,
) -> bool {
    match matcher {
        AstMatcher::Semantic(semantic) => semantic.reads_values(seen),
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => true,
        AstMatcher::Compare(comparison) => {
            matches!(comparison.text, CompareText::Semantic(_))
                || comparison
                    .transform
                    .as_ref()
                    .is_some_and(|transform| transform.reads_values())
        }
        AstMatcher::Pattern(_) | AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
            false
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            items.iter().any(|item| reads_values(item, seen))
        }
        AstMatcher::Not(item) => reads_values(item, seen),
        AstMatcher::Inside(item, stop)
        | AstMatcher::Has(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            reads_values(item, seen) || stop_rule(stop).is_some_and(|stop| reads_values(stop, seen))
        }
        AstMatcher::Field(relation) => {
            reads_values(&relation.rule, seen)
                || stop_rule(&relation.stop).is_some_and(|stop| reads_values(stop, seen))
        }
        AstMatcher::Count(count) => reads_values(&count.relation, seen),
        AstMatcher::NthChild(nth) => nth
            .of_rule
            .as_ref()
            .is_some_and(|rule| reads_values(rule, seen)),
        AstMatcher::Util(util) => {
            seen.insert(util.index)
                && util
                    .with_target(|rule| reads_values(rule, seen))
                    .unwrap_or(false)
        }
        AstMatcher::Call(call) => {
            seen.insert(call.util.index)
                && call
                    .util
                    .with_target(|rule| reads_values(rule, seen))
                    .unwrap_or(false)
        }
        AstMatcher::Constrained(core) => {
            reads_values(&core.rule, seen)
                || core
                    .constraints
                    .iter()
                    .any(|(_, constraint)| reads_values(constraint, seen))
        }
    }
}

/// Binds `to` in `target` to what the `from` variables hold in `source` (a node, a node list, text
/// a parsed tree exported and its host range), unifying them with each other and with an existing
/// binding of the same kind in `target`: their texts must be equal. An existing binding is kept;
/// otherwise, of equal nodes, the one first in the source is bound, so the order the `from`
/// variables come in never selects the result. `None` where they differ.
fn unify_capture<'tree, 'name, D: Doc>(
    source: &MetaVarEnv<'tree, D>,
    from: impl Iterator<Item = &'name str> + Clone,
    target: &mut MetaVarEnv<'tree, D>,
    to: &str,
) -> Option<()> {
    let range = |node: &Node<'tree, D>| (node.range().start, node.range().end);
    let held = target.get_match(to).cloned();
    let mut chosen: Option<Node<'tree, D>> = None;
    for node in from.clone().filter_map(|from| source.get_match(from)) {
        if let Some(agreed) = held.as_ref().or(chosen.as_ref()) {
            (agreed.text() == node.text()).then_some(())?;
        }
        if held.is_none() && chosen.as_ref().is_none_or(|kept| range(node) < range(kept)) {
            chosen = Some(node.clone());
        }
    }
    if let Some(node) = chosen {
        target.insert(to, node)?;
    }
    let held = target.get_multiple_matches(to);
    let agree = |left: &[Node<'tree, D>], right: &[Node<'tree, D>]| {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(left, right)| left.text() == right.text())
    };
    let mut chosen: Option<Vec<Node<'tree, D>>> = None;
    for nodes in from.clone().map(|from| source.get_multiple_matches(from)) {
        if nodes.is_empty() {
            continue;
        }
        let agreed = if held.is_empty() {
            chosen.as_deref()
        } else {
            Some(held.as_slice())
        };
        if let Some(agreed) = agreed {
            agree(agreed, &nodes).then_some(())?;
        }
        if held.is_empty()
            && chosen
                .as_ref()
                .is_none_or(|kept| nodes.iter().map(range).lt(kept.iter().map(range)))
        {
            chosen = Some(nodes);
        }
    }
    if let Some(nodes) = chosen {
        target.insert_multi(to, nodes)?;
    }
    // Exported texts agree only with their ranges, so equal ones are one and the same.
    for from in from {
        for (from, to) in [
            (from.to_owned(), to.to_owned()),
            (exported::span_variable(from), exported::span_variable(to)),
        ] {
            if let Some(text) = source.get_transformed(&from) {
                if let Some(existing) = target.get_transformed(&to) {
                    (existing == text).then_some(())?;
                } else {
                    target.insert_transformation(&MetaVariable::Multiple, &to, text.clone());
                }
            }
        }
    }
    Some(())
}

/// Enters one level of utility recursion (`matches` or a parameterized call) within the depth and
/// stack limits the matcher applies: the level to restore on leaving, or the limit hit.
pub(super) fn enter_util_level() -> Result<u32, UtilLimit> {
    if let Some(limit) = UTIL_EXCEEDED.get() {
        return Err(limit);
    }
    let depth = UTIL_DEPTH.get();
    if depth >= MAX_UTIL_DEPTH || util_stack_exceeded() {
        UTIL_EXCEEDED.set(Some(UtilLimit::Depth));
        return Err(UtilLimit::Depth);
    }
    UTIL_DEPTH.set(depth + 1);
    Ok(depth)
}

/// Leaves a level `enter_util_level` entered.
pub(super) fn leave_util_level(depth: u32) {
    UTIL_DEPTH.set(depth);
}

/// Spends one step of the file's utility and count budget (`maxVisitedNodes`); false once spent.
pub(super) fn spend_util_step() -> bool {
    if UTIL_EXCEEDED.get().is_some() {
        return false;
    }
    let Some(steps) = UTIL_STEPS_LEFT.get().checked_sub(1) else {
        UTIL_EXCEEDED.set(Some(UtilLimit::Work));
        return false;
    };
    UTIL_STEPS_LEFT.set(steps);
    true
}

/// Remaining utility/count budget for a query-engine memoization replay.
pub(super) fn utility_steps_left() -> usize {
    UTIL_STEPS_LEFT.get()
}

/// A predicate's verdict at one node, for the query engine: it composes terms in SQL and tests
/// only Rerail's predicates without sub-rules by their own code (`facts::test_predicate`).
pub(super) enum Verdict<'tree, D: Doc> {
    Match(MetaVarEnv<'tree, D>),
    NoMatch,
    Unknown,
}

/// Sets up this thread to evaluate `matcher`'s check on one file, with `work` left of its
/// `maxVisitedNodes` budget, as `GrammarMatcher::match_root` does for each root. Verdicts travel
/// explicitly (`Verdict`; a predicate is told whether its check is three-valued); the Unknown flag
/// is set up for the sub-rules semantic predicates match themselves.
pub(super) fn begin_check(matcher: &GrammarMatcher, work: usize) {
    UTIL_DEPTH.set(0);
    UTIL_EXCEEDED.set(None);
    UTIL_STACK_BASE.set(stack_address());
    UTIL_STEPS_LEFT.set(work);
    THREE_VALUED.set(matcher.three_valued);
    UNKNOWN.set(false);
    resolve::CURRENT_CHECK.set(matcher as *const GrammarMatcher as usize);
}

/// Ends `begin_check`: the work left, and the limit a term hit, if any.
pub(super) fn end_check() -> (usize, Option<UtilLimit>) {
    UTIL_STACK_BASE.set(0);
    (UTIL_STEPS_LEFT.replace(usize::MAX), UTIL_EXCEEDED.take())
}

/// Whether matching `term` never adds a binding (see `AstMatcher::binds_nothing`).
pub(super) fn binds_nothing(term: &AstMatcher) -> bool {
    term.binds_nothing()
}

/// Whether a relation's rule can bind a capture, which makes its search strict past an unknown
/// candidate (see `relation::Search`).
pub(super) fn may_bind(term: &AstMatcher) -> bool {
    relation::may_bind(term)
}

/// Whether recursion from `begin_check` has used more stack than utilities may
/// (`MAX_UTIL_STACK_BYTES`), as the matcher checks before each utility.
pub(super) fn util_stack_exceeded() -> bool {
    let base = UTIL_STACK_BASE.get();
    base != 0 && base.abs_diff(stack_address()) > MAX_UTIL_STACK_BYTES
}

/// A `finding.each` enumeration at one root, for the query engine (`each::Enumeration`): while it
/// lives, relations and `any` pass over candidates binding the capture to a recorded range.
pub(super) struct EachScope(each::Enumeration);

impl EachScope {
    pub(super) fn start(name: &str) -> Self {
        Self(each::Enumeration::start(name))
    }

    /// Records a match's binding of `name`; false where it binds nothing new.
    pub(super) fn record<D: Doc>(&self, env: &MetaVarEnv<'_, D>, name: &str) -> bool {
        self.0.record(each::binding_range(env, name))
    }
}

/// Whether an enumeration is excluding and `env` binds its capture to a recorded range.
pub(super) fn each_excluded<D: Doc>(env: &MetaVarEnv<'_, D>) -> bool {
    each::active() && each::excluded(env)
}
