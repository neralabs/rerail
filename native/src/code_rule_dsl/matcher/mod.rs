mod analysis;
mod anchors;
mod arguments;
mod arguments_sql;
mod binding;
mod binds;
mod call;
mod captures;
mod collate;
mod compare;
mod count;
mod each;
mod embedded;
mod exported;
mod format_args;
mod layout;
mod memo;
mod nth_child;
mod parse;
mod relation;
mod resolve;
mod run;
mod semantic;
#[cfg(test)]
mod tests;
mod transform;
mod util;
mod value;

use super::budget;
use crate::code_rule_dsl::language::{GrammarDialect, SourceType};
pub(super) use analysis::RequiredText;
pub(crate) use analysis::calls_positive_only;
pub(super) use anchors::{AnchorPlan, FileAnchorIndex, Selection};
pub(super) use arguments::{Arguments, Passed, SlotKind};
pub(super) use arguments_sql::SQL_CALL_KINDS;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::matcher::{KindMatcher, Pattern};
use ast_grep_core::matcher::{MatcherExt, NodeMatch};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{Doc, Node};
pub(super) use binding::BindingTest;
pub(super) use binds::{BindingCount, BindingFilter, Binds, Redefines, ScopeName};
pub(super) use captures::{Capture, CaptureAnalysis, collect_variables, parse_exported};
pub(super) use compare::{Collation, CompareOp, CompareText, Comparison, EqClass, SemanticText};
pub(super) use count::{Count, CountBinding};
pub(crate) use each::binding_range;
pub(crate) use embedded::EmbeddedTest;
pub(crate) use exported::{
    WITNESS_NODE, export_text, exported_span, exported_text, is_span_variable,
};
pub(crate) use format_args::{FormatArgs, Style};
pub(crate) use layout::{Gap, Layout, LineCount, Neighbour};
pub(super) use memo::with_file as with_file_memo;
pub(super) use nth_child::{NthChild, Position as NthPosition, parse_position};
#[cfg(test)]
pub(crate) use nth_child::{named_sibling_visits, reset_named_sibling_visits};
pub(crate) use parse::{HostTest, ParseTest};
use regex::Regex;
pub(super) use relation::{FieldRelation, Related, StopBy, stop_rule};
#[cfg(test)]
pub(super) use relation::{set_answers_off, set_same_key_off};
pub(crate) use resolve::{
    FileGates, calls_tables, file_biome_suppressions, file_clippy, file_eslint_directives,
    file_lines, file_module, file_noqa, file_nosec, file_oxlint_directives, file_phpcs,
    file_phpstan, file_psalm, file_rerail_ignores, file_source_type, file_sqlfluff,
    file_vue_directives, table_reading,
};
pub(super) use resolve::{FileScopeSlot, Resolves, Unresolved};
#[cfg(test)]
pub(super) use resolve::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS, TABLE_BUILDS};
pub(super) use run::{Stopped, enumerate, node_at, run_clause};
#[cfg(test)]
pub(crate) use run::{reset_roots_matched, roots_matched};
pub(super) use semantic::Semantic;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;
pub(super) use transform::{Separator, Source, StringCase, Trans, Transform};

#[cfg(test)]
pub(crate) use transform::{
    line_table_constructions, note_line_table_construction, reset_line_table_constructions,
};
pub(super) use util::{
    CallArgs, MAX_UTIL_INSTANCES, UtilCall, UtilLimit, UtilOrigin, UtilRef, UtilTable,
    UtilitySummary,
};
use util::{
    MAX_UTIL_DEPTH, MAX_UTIL_STACK_BYTES, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STACK_BASE,
    UTIL_STEPS_LEFT, stack_address,
};
use util::{THREE_VALUED, UNKNOWN};
pub(super) use value::{Bound, Reading, ValueTest};

#[derive(Clone)]
pub(super) struct GrammarMatcher {
    /// The dialect of the document the matcher reads: the file's own, or for an embedded clause
    /// (`ast.embedded`) its program's.
    pub(super) language: GrammarDialect,
    /// The embedding whose program the matcher reads (`ast.embedded`); `None` for the file's own
    /// tree.
    pub(super) document: Option<crate::code_rule_dsl::language::Via>,
    /// Keeps the utility tables of the file's other document alive (`embedded`, `host`), as
    /// `utils` does the clause's own: their rules reach them only weakly (`UtilRef`).
    #[allow(dead_code)]
    pub(super) other_utils: Vec<Arc<UtilTable>>,
    /// Whether the rule crosses to the file's other document (`embedded`, `host`): the syntax
    /// errors of every document of the file then bear on it, not its own document's alone.
    pub(super) crosses: bool,
    pub(super) matcher: AstMatcher,
    pub(super) anchors: Arc<anchors::AnchorPlan>,
    /// What a file must contain for the matcher to match in it (`RequiredText`); `None` where
    /// nothing is required or the matcher can be unknown, which a file without a match still reports.
    pub(super) text: Option<Arc<RequiredText>>,
    /// Whether witnesses outside syntax-error regions stay sound; see `sound_under_errors`.
    pub(super) sound_under_errors: bool,
    /// Keeps the query's utility rules alive (`AstMatcher::Util` holds only a weak reference)
    /// and lets violation compilation walk them.
    pub(super) utils: Arc<UtilTable>,
    /// The query's `file` condition: a file whose module path fails it gets no matcher.
    pub(super) file: Option<Arc<crate::code_rule_dsl::language::FileCondition>>,
    /// Whether the query's `when` gate holds. A query outside it is compiled in full, so its
    /// captures and templates are checked, but matches no file: sound, not unknown.
    pub(super) applies: bool,
    /// The source types `when.sourceType` lets a file have where the query leaves `sourceType`
    /// to each file (`SourceType::of_file`); a file of another matches nothing (`run_clause`).
    pub(super) source_types: Option<Vec<SourceType>>,
    /// Whether the matcher can reach an unknown verdict; see `reads_values`.
    pub(super) three_valued: bool,
    /// The query's `transform`, applied to each match after its rule and constraints.
    pub(super) transform: Option<Arc<Transform>>,
}

impl GrammarMatcher {
    /// The kinds of the roots its witnesses may have, as codes of the file's own tree (`None`
    /// where any, or where it reads an embedded program, whose codes are its own grammar's).
    pub(super) fn host_root_kinds(&self) -> Option<&std::collections::BTreeSet<u16>> {
        self.document
            .is_none()
            .then(|| self.anchors.root_kinds())
            .flatten()
    }
}

/// Whole-root matching: production matches each candidate root within a clause `begin_clause` set
/// up (`match_rule`); `match_root` sets one up per root, for tests.
impl GrammarMatcher {
    /// Whether a file whose own source type is `file` may get this matcher: one its
    /// `when.sourceType` lists, or, while the file's syntax is unread (`None`), a module or
    /// CommonJS listed.
    pub(super) fn may_apply_to(&self, file: Option<SourceType>) -> bool {
        self.source_types.as_ref().is_none_or(|types| match file {
            Some(file) => types.contains(&file),
            None => types.contains(&SourceType::Module) || types.contains(&SourceType::CommonJs),
        })
    }

    /// This matcher's rule (with its constraints) at one root of the clause `begin_clause` set up:
    /// the root's match. A root whose verdict is unknown does not match.
    fn match_rule<'tree, D: Doc>(&self, node: Node<'tree, D>) -> Option<NodeMatch<'tree, D>> {
        UNKNOWN.set(false);
        self.matcher.match_node(node)
    }

    /// Matches one root. Each utility evaluation spends one step of `work`, the caller's remaining
    /// utility budget for this file; like a traversal it is bounded by `maxVisitedNodes`. A root
    /// whose verdict is unknown is not a match.
    #[cfg(test)]
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
        resolve::CURRENT_CLAUSE.set(self as *const Self as usize);
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

impl Constrained {
    #[inline(never)]
    fn match_node<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        let mut trial = Cow::Borrowed(env.as_ref());
        self.rule.match_node_with_env(node.clone(), &mut trial)?;
        for (name, constraint) in &self.constraints {
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
}

#[derive(Clone)]
pub(super) enum AstMatcher {
    Pattern(Pattern),
    Kind(KindMatcher),
    Regex(Regex),
    All(Vec<AstMatcher>),
    Any(Vec<AstMatcher>),
    Not(Box<AstMatcher>),
    Inside(Box<Related>, StopBy),
    Has(Box<Related>, StopBy),
    Precedes(Box<Related>, StopBy),
    Follows(Box<Related>, StopBy),
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

/// The matcher: production matches every AST rule with it (`run_clause`).
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
            Self::All(matchers) => Self::match_all(matchers, node, env),
            Self::Any(matchers) => Self::match_any(matchers, node, env),
            Self::Not(matcher) => Self::match_not(matcher, node, env),
            Self::Inside(matcher, stop) => stop
                .reaches(&node, || node.parent(), || node.ancestors(), matcher, env)
                .then_some(node),
            // Every child, named or anonymous. With a stop rule, pre-order like `end`, but a
            // descendant the stop rule matches is tested and not entered (ast-grep-config `Has`,
            // `relational_rule.rs:185`).
            Self::Has(matcher, stop) => relation::has(&node, matcher, stop, env).then_some(node),
            // Siblings only, named or anonymous: `neighbor` is the adjacent sibling, `end` every
            // sibling in that direction. `next_all`/`prev_all` are ast-grep's own sibling walks.
            Self::Precedes(matcher, stop) => {
                stop.find_sibling(&node, true, matcher, env).then_some(node)
            }
            Self::Follows(matcher, stop) => stop
                .find_sibling(&node, false, matcher, env)
                .then_some(node),
            Self::Field(relation) => relation.find(&node, env).map(|_| node),
            Self::Util(util) => memo::util(util, node, env, |node, env| {
                let level = enter_util_level().ok()?;
                if !spend_util_step() {
                    leave_util_level(level);
                    return None;
                }
                let matched = util
                    .with_target(|rule| rule.match_node_with_env(node, env))
                    .flatten();
                leave_util_level(level);
                matched
            }),
            Self::Constrained(core) => core.match_node(node, env),
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

// The composites, out of line: a debug build gives every arm of `match_node_with_env` its own stack
// slots, and every level of utility recursion runs through several of its frames.
impl AstMatcher {
    // Composites and relations follow ast-grep-config 0.45.3: one environment threads through every
    // sub-rule, is committed only when the whole composite matches, and a relation binds from the
    // first related node that matches (no backtracking). Verdicts are three-valued (`UNKNOWN`):
    // `all` (a rule object's keys too) is false when a term is false, whatever order its terms run
    // in. Past an unknown term the later terms still run and a false one decides, but a term that
    // mentions a capture an unknown term would have bound cannot be read without it: it stays
    // open, and what it binds is withheld from the terms after it as well.
    #[inline(never)]
    fn match_all<'tree, D: Doc>(
        matchers: &[AstMatcher],
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        let mut trial = Cow::Borrowed(env.as_ref());
        let mut unknown = false;
        let mut withheld: Option<BTreeSet<String>> = None;
        // Read only past an unknown term: most `all`s never build one.
        let mut captures: Option<Box<CaptureAnalysis>> = None;
        for matcher in matchers {
            if let Some(names) = withheld.as_mut()
                && !captures
                    .get_or_insert_default()
                    .mentions(matcher)
                    .is_disjoint(names)
            {
                names.extend(captures.get_or_insert_default().binds(matcher));
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
                            .extend(captures.get_or_insert_default().binds(matcher));
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

    // Each branch starts from the bindings so far; only the matching branch's commit. The verdict
    // is unknown when no branch matches and some branch was unknown. A branch whose verdict is
    // unknown and that may bind ends the search unknown, as a strict relation search does: a later
    // branch must not substitute its captures for the ones it would have bound. An unknown branch
    // that binds nothing lets later branches decide.
    #[inline(never)]
    fn match_any<'tree, D: Doc>(
        matchers: &[AstMatcher],
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
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
                if relation::may_bind(matcher) {
                    break;
                }
            }
        }
        if three {
            UNKNOWN.set(saved || unknown);
        }
        None
    }

    // Reads the bindings so far but never contributes any. The negation of an unknown verdict is
    // unknown: it fails, and stays unknown.
    #[inline(never)]
    fn match_not<'tree, D: Doc>(
        matcher: &AstMatcher,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
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
}

impl AstMatcher {
    /// Whether matching never adds a binding: terms that only read the node or the bindings so
    /// far, and composites and relations of them. A pattern, a utility and a constrained rule may
    /// bind, so they count as binding.
    fn binds_nothing(&self) -> bool {
        match self {
            Self::Semantic(semantic) => semantic.binds_nothing(),
            Self::Kind(_) | Self::Regex(_) | Self::Not(_) | Self::Compare(_) | Self::Never => true,
            // A count binds its value under `as`.
            Self::Count(count) => count.binding.is_none(),
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
    #[inline(never)]
    fn match_extension<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        match self {
            Self::Count(count) => each::suspended(|| count.holds(&node, env)).then_some(node),
            Self::NthChild(nth) => nth.match_node(node, env),
            Self::Call(call) => memo::call(call, node, env, |node, env| {
                let called = call::call(call, env, MetaVarEnv::new(), |frame| {
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
            }),
            _ => None,
        }
    }
}

/// Whether a matcher can reach an unknown verdict: it holds a `value` predicate or a comparison over
/// values or property names, directly or through a composite, relation, stop rule, count,
/// constraint or utility. Clauses without one never touch the `UNKNOWN` flag.
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

thread_local! {
    /// The deepest `UTIL_DEPTH` reached since `swap_util_peak` last reset it.
    static UTIL_PEAK: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Enters one level of utility recursion (`matches` or a parameterized call) within the depth and
/// stack limits the matcher applies: the level to restore on leaving, or the limit hit.
fn enter_util_level() -> Result<u32, UtilLimit> {
    if let Some(limit) = UTIL_EXCEEDED.get() {
        return Err(limit);
    }
    let depth = UTIL_DEPTH.get();
    if depth >= MAX_UTIL_DEPTH || util_stack_exceeded() {
        UTIL_EXCEEDED.set(Some(UtilLimit::Depth));
        return Err(UtilLimit::Depth);
    }
    UTIL_DEPTH.set(depth + 1);
    UTIL_PEAK.set(UTIL_PEAK.get().max(depth + 1));
    Ok(depth)
}

/// Leaves a level `enter_util_level` entered.
fn leave_util_level(depth: u32) {
    UTIL_DEPTH.set(depth);
}

/// The utility levels this thread is inside.
fn util_level() -> u32 {
    UTIL_DEPTH.get()
}

/// Sets the deepest utility level reached to `peak`, returning what it was.
fn swap_util_peak(peak: u32) -> u32 {
    UTIL_PEAK.replace(peak)
}

/// Whether `levels` more utility levels fit under the depth limit from here.
fn util_levels_fit(levels: u32) -> bool {
    UTIL_DEPTH.get() + levels <= MAX_UTIL_DEPTH
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

/// Spends `steps` of the budget at once, as that many `spend_util_step` calls would: false (with
/// the budget spent) where it runs out.
fn spend_util_steps(steps: usize) -> bool {
    if steps == 0 {
        return true;
    }
    if UTIL_EXCEEDED.get().is_some() {
        return false;
    }
    match UTIL_STEPS_LEFT.get().checked_sub(steps) {
        Some(left) => {
            UTIL_STEPS_LEFT.set(left);
            true
        }
        None => {
            UTIL_STEPS_LEFT.set(0);
            UTIL_EXCEEDED.set(Some(UtilLimit::Work));
            false
        }
    }
}

/// Remaining utility/count budget for a memoization replay.
fn utility_steps_left() -> usize {
    UTIL_STEPS_LEFT.get()
}

/// Whether a term of the clause in progress hit a utility limit: its verdicts since are not the
/// matcher's, so nothing may remember them.
fn util_limit_hit() -> bool {
    UTIL_EXCEEDED.get().is_some()
}

/// Sets up this thread to evaluate `matcher`'s clause on one file, with `work` left of its
/// `maxVisitedNodes` budget, as `GrammarMatcher::match_root` does for each root, and the Unknown
/// flag for the sub-rules semantic predicates match themselves.
pub(super) fn begin_clause(matcher: &GrammarMatcher, work: usize) {
    CLAUSES_BEGUN.set(CLAUSES_BEGUN.get() + 1);
    UTIL_DEPTH.set(0);
    UTIL_EXCEEDED.set(None);
    UTIL_STACK_BASE.set(stack_address());
    UTIL_STEPS_LEFT.set(work);
    THREE_VALUED.set(matcher.three_valued);
    UNKNOWN.set(false);
    resolve::CURRENT_CLAUSE.set(matcher as *const GrammarMatcher as usize);
}

thread_local! {
    static CLAUSES_BEGUN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many clauses this thread has begun (`begin_clause`): a clause begun within a computation
/// (a `calls` seed's query) resets the budget and levels it reads, so what the computation spent
/// or reached is not its own.
fn clauses_begun() -> u64 {
    CLAUSES_BEGUN.get()
}

thread_local! {
    static FILL_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Runs `fill`, which fills a cache later evaluations read for free (a file's scope table and
/// usage, a parent's counted siblings), counting the steps it spends apart
/// (`fill_steps`): a memoized verdict whose computation filled one replays only the rest, which
/// is what computing it again would spend.
fn filling<T>(fill: impl FnOnce() -> T) -> T {
    let (steps, fills) = (UTIL_STEPS_LEFT.get(), FILL_STEPS.get());
    let result = fill();
    // Fills within this one are part of it.
    FILL_STEPS.set(fills + steps.saturating_sub(UTIL_STEPS_LEFT.get()));
    result
}

/// Counts `steps` a cache fill spent directly (`filling`).
fn filled(steps: usize) {
    FILL_STEPS.set(FILL_STEPS.get() + steps);
}

/// Steps cache fills have spent on this thread (`filling`).
fn fill_steps() -> usize {
    FILL_STEPS.get()
}

thread_local! {
    static CLAUSE_STATE_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Notes that the clause in progress read state kept for it alone: a cache keyed by one of its
/// terms (`nthChild`'s counted siblings, a same-key walk's classes) or a file table charged to
/// each clause once (scopes, usage, PHP names). What a computation spends then depends on the
/// clause, so its verdict is that clause's alone (`memo`).
fn read_clause_state() {
    CLAUSE_STATE_READS.set(CLAUSE_STATE_READS.get() + 1);
}

/// How many reads of a clause's own state this thread has noted (`read_clause_state`).
fn clause_state_reads() -> u64 {
    CLAUSE_STATE_READS.get()
}

/// Ends `begin_clause`: the work left, and the limit a term hit, if any.
pub(super) fn end_clause() -> (usize, Option<UtilLimit>) {
    UTIL_STACK_BASE.set(0);
    (UTIL_STEPS_LEFT.replace(usize::MAX), UTIL_EXCEEDED.take())
}

/// Whether recursion from `begin_clause` has used more stack than utilities may
/// (`MAX_UTIL_STACK_BYTES`), as the matcher checks before each utility.
fn util_stack_exceeded() -> bool {
    let base = UTIL_STACK_BASE.get();
    base != 0 && base.abs_diff(stack_address()) > MAX_UTIL_STACK_BYTES
}

/// A `violation.each` enumeration at one root (`each::Enumeration`): while it lives, relations
/// and `any` pass over candidates binding the capture to a recorded range.
pub(super) struct EachScope(each::Enumeration);

impl EachScope {
    pub(super) fn start(name: &str) -> Self {
        Self(each::Enumeration::start(name))
    }

    /// Records a match's binding of `name`; false where it binds nothing new.
    fn record<D: Doc>(&self, env: &MetaVarEnv<'_, D>, name: &str) -> bool {
        self.0.record(each::binding_range(env, name))
    }
}
