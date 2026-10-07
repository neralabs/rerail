//! What a matcher does with captures, read from its terms alone: the names it binds, reads and
//! mentions (capture ordering, `all` order checks, the query engine's three-valued composition),
//! and the metavariables its patterns define (finding templates, `finding.each`).

mod semantic;

use super::{AstMatcher, Comparison, UtilRef, UtilitySummary};
#[cfg(test)]
use crate::policy_dsl::test_support::{Walk, record_walk};
use ast_grep_core::matcher::PatternNode;
use ast_grep_core::meta_var::MetaVariable;
pub(in crate::policy_dsl) use semantic::parse_exported;
use std::collections::{BTreeMap, BTreeSet};

/// A utility's summary key in a `CaptureAnalysis` (`UtilRef::summary_key`).
type UtilKey = (usize, usize, u8);

type Names = BTreeSet<String>;

/// What terms bind, read and mention, each utility summarized once per analysis however many terms
/// and paths reach it. An analysis may span a whole compilation: a utility's summaries are keyed
/// by which of its compilations they read.
#[derive(Default)]
pub(in crate::policy_dsl) struct CaptureAnalysis {
    binds: UtilitySummary<UtilKey, Names>,
    reads: UtilitySummary<UtilKey, Names>,
    mentions: UtilitySummary<UtilKey, Names>,
}

impl CaptureAnalysis {
    /// Names a matcher may bind into the environment it runs in. `not` and `count` keep what they
    /// bind to themselves; a stop rule runs in an environment of its own.
    pub(in crate::policy_dsl) fn binds(&mut self, matcher: &AstMatcher) -> Names {
        let mut names = Names::new();
        collect_binds(matcher, &mut self.binds, &mut names);
        names
    }

    /// Names a matcher reads from the environment it runs in and does not bind itself: the sides
    /// of its comparisons and the captures its binding counts are `within`, less what its own terms
    /// bind. A negation also reads every name its rule would bind (a pattern's metavariable, a
    /// utility's, transitively): none of them leaves it, so each is either a sibling's capture it
    /// tests or a fresh one. A stop rule reads only what it binds itself. An `any` reads what any
    /// branch reads: another branch binding the same name binds it only when that branch is the
    /// one that matches.
    pub(in crate::policy_dsl) fn reads(&mut self, matcher: &AstMatcher) -> Names {
        binds_and_reads(matcher, &mut self.binds, &mut self.reads).1
    }

    /// Every capture a matcher's verdict may depend on or that it may bind, inside `not` and
    /// `count` too: what could read differently when a sibling that would have bound it was left
    /// unknown. That is what its patterns define, its comparisons read, its constraints name and
    /// its calls pass, through the utilities it reaches.
    pub(in crate::policy_dsl) fn mentions(&mut self, matcher: &AstMatcher) -> Names {
        let mut names = Names::new();
        collect_mentions(matcher, &mut self.mentions, &mut names);
        names
    }
}

/// `CaptureAnalysis::binds` in an analysis of its own.
pub(in crate::policy_dsl) fn binds(matcher: &AstMatcher) -> Names {
    CaptureAnalysis::default().binds(matcher)
}

fn collect_binds(
    matcher: &AstMatcher,
    summaries: &mut UtilitySummary<UtilKey, Names>,
    names: &mut Names,
) {
    match matcher {
        AstMatcher::Pattern(pattern) => {
            names.extend(pattern.defined_vars().into_iter().map(str::to_owned));
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                collect_binds(item, summaries, names);
            }
        }
        AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => collect_binds(item, summaries, names),
        // A `field` narrows `inside` or `has` and is ordered like them.
        AstMatcher::Field(relation) => collect_binds(&relation.rule, summaries, names),
        // `ofRule` commits the node's own bindings.
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                collect_binds(rule, summaries, names);
            }
        }
        AstMatcher::Util(util) => names.extend(utility_binds(util, summaries)),
        // A call binds, by its caller's names, the parameters its utility binds in its frame.
        AstMatcher::Call(call) => {
            let inner = utility_binds(&call.util, summaries);
            names.extend(call.outward(inner).into_iter().map(str::to_owned));
        }
        AstMatcher::Semantic(semantic) => semantic::collect_binds(semantic, summaries, names),
        AstMatcher::Constrained(core) => {
            collect_binds(&core.rule, summaries, names);
            for (_, constraint) in &core.constraints {
                collect_binds(constraint, summaries, names);
            }
        }
        AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Not(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Count(_)
        | AstMatcher::Never => {}
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}

/// What a utility's rule binds, in the utility's own names.
fn utility_binds(util: &UtilRef, summaries: &mut UtilitySummary<UtilKey, Names>) -> Names {
    summaries.summarize(util.summary_key(), Names::new(), |summaries| {
        #[cfg(test)]
        record_walk(Walk::OrderBinds);
        let mut names = Names::new();
        util.with_target(|rule| collect_binds(rule, summaries, &mut names));
        names
    })
}

/// What a matcher binds and what it reads, from one walk of its terms: what a term reads is less
/// what the terms before it bind. A utility's reads are a summary of their own, computed from the
/// complete summaries of what the utilities it reaches bind.
fn binds_and_reads(
    matcher: &AstMatcher,
    binds: &mut UtilitySummary<UtilKey, Names>,
    reads: &mut UtilitySummary<UtilKey, Names>,
) -> (Names, Names) {
    match matcher {
        AstMatcher::Pattern(pattern) => (
            pattern
                .defined_vars()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            Names::new(),
        ),
        AstMatcher::Compare(comparison) => (Names::new(), comparison_reads(comparison)),
        AstMatcher::Semantic(semantic) => {
            let mut bound = Names::new();
            semantic::collect_binds(semantic, binds, &mut bound);
            let mut read = semantic::reads(semantic, binds, reads);
            read.retain(|name| !bound.contains(name));
            (bound, read)
        }
        AstMatcher::Any(items) => {
            let (mut bound, mut read) = (Names::new(), Names::new());
            for item in items {
                let (item_bound, item_read) = binds_and_reads(item, binds, reads);
                bound.extend(item_bound);
                read.extend(item_read);
            }
            (bound, read)
        }
        // An `all` runs its items in order: an item reads what no earlier item bound, even a name
        // a later item binds (the `all` order check refuses that unless an outside binding
        // excuses it, which must then run first).
        AstMatcher::All(items) => {
            let (mut bound, mut read) = (Names::new(), Names::new());
            for item in items {
                let (item_bound, item_read) = binds_and_reads(item, binds, reads);
                read.extend(item_read.into_iter().filter(|name| !bound.contains(name)));
                bound.extend(item_bound);
            }
            (bound, read)
        }
        AstMatcher::Not(item) => {
            let (mut read, item_read) = binds_and_reads(item, binds, reads);
            read.extend(item_read);
            (Names::new(), read)
        }
        // A relation, `ofRule` and a count read what their rule reads, which is already less what
        // it binds before reading.
        AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => binds_and_reads(item, binds, reads),
        AstMatcher::Field(relation) => binds_and_reads(&relation.rule, binds, reads),
        AstMatcher::NthChild(nth) => nth
            .of_rule
            .as_ref()
            .map(|rule| binds_and_reads(rule, binds, reads))
            .unwrap_or_default(),
        AstMatcher::Count(count) => (
            Names::new(),
            binds_and_reads(&count.relation, binds, reads).1,
        ),
        AstMatcher::Util(util) => (
            utility_binds(util, binds),
            utility_reads(util, binds, reads),
        ),
        // A call binds and reads, by its caller's names, the parameters its utility binds and
        // reads in its frame (before binding them: a parameter a gap reads and a later item binds
        // is read).
        AstMatcher::Call(call) => {
            let outward = |names: Names| {
                call.outward(names)
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Names>()
            };
            (
                outward(utility_binds(&call.util, binds)),
                outward(utility_reads(&call.util, binds, reads)),
            )
        }
        AstMatcher::Constrained(core) => {
            let (mut bound, mut read) = binds_and_reads(&core.rule, binds, reads);
            for (_, constraint) in &core.constraints {
                let (constraint_bound, constraint_read) = binds_and_reads(constraint, binds, reads);
                bound.extend(constraint_bound);
                read.extend(constraint_read);
            }
            read.retain(|name| !bound.contains(name));
            (bound, read)
        }
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
            (Names::new(), Names::new())
        }
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => (Names::new(), Names::new()),
    }
}

/// What a utility's rule reads, in the utility's own names.
fn utility_reads(
    util: &UtilRef,
    binds: &mut UtilitySummary<UtilKey, Names>,
    reads: &mut UtilitySummary<UtilKey, Names>,
) -> Names {
    reads.summarize(util.summary_key(), Names::new(), |reads| {
        #[cfg(test)]
        record_walk(Walk::OrderReads);
        util.with_target(|rule| binds_and_reads(rule, binds, reads).1)
            .unwrap_or_default()
    })
}

/// The captures a comparison reads: each side, or the captures a transform variable on that side
/// is computed from.
fn comparison_reads(comparison: &Comparison) -> Names {
    [&comparison.left, &comparison.right]
        .into_iter()
        .flat_map(|name| match &comparison.transform {
            Some(transform) if transform.contains(name) => transform
                .roots(name)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            _ => vec![name.clone()],
        })
        .collect()
}

fn collect_mentions(
    matcher: &AstMatcher,
    summaries: &mut UtilitySummary<UtilKey, Names>,
    names: &mut Names,
) {
    match matcher {
        AstMatcher::Pattern(pattern) => {
            names.extend(pattern.defined_vars().into_iter().map(str::to_owned));
        }
        AstMatcher::Compare(comparison) => names.extend(comparison_reads(comparison)),
        AstMatcher::Semantic(semantic) => semantic::collect_mentions(semantic, summaries, names),
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                collect_mentions(item, summaries, names);
            }
        }
        AstMatcher::Not(item)
        | AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => collect_mentions(item, summaries, names),
        AstMatcher::Field(relation) => collect_mentions(&relation.rule, summaries, names),
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                collect_mentions(rule, summaries, names);
            }
        }
        AstMatcher::Count(count) => collect_mentions(&count.relation, summaries, names),
        AstMatcher::Util(util) => {
            names.extend(
                summaries.summarize(util.summary_key(), Names::new(), |summaries| {
                    #[cfg(test)]
                    record_walk(Walk::OrderMentions);
                    let mut names = Names::new();
                    util.with_target(|rule| collect_mentions(rule, summaries, &mut names));
                    names
                }),
            );
        }
        // A call's verdict depends on every capture it passes, and on nothing else of its caller.
        AstMatcher::Call(call) => names.extend(call.callers().map(str::to_owned)),
        AstMatcher::Constrained(core) => {
            collect_mentions(&core.rule, summaries, names);
            for (name, constraint) in &core.constraints {
                names.insert(name.clone());
                collect_mentions(constraint, summaries, names);
            }
        }
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {}
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Capture {
    Single,
    Multiple,
}

fn merge_capture(variables: &mut BTreeMap<String, Capture>, name: &str, capture: Capture) {
    let entry = variables.entry(name.to_owned()).or_insert(capture);
    if capture == Capture::Single {
        *entry = Capture::Single;
    }
}

/// Metavariables the matcher's patterns define, as ast-grep's `defined_vars`: patterns under `not`
/// count, `matches` references do not (their utilities are collected on their own).
pub(crate) fn collect_variables(matcher: &AstMatcher, variables: &mut BTreeMap<String, Capture>) {
    collect_in(matcher, variables, &mut UtilitySummary::new(), false);
}

/// `collect_variables`, where `framed` says whether the matcher runs in a parameterized call's frame
/// (whose utilities it shares, so they are walked) and a call contributes the captures its
/// parameters return, by the caller's names.
fn collect_in(
    matcher: &AstMatcher,
    variables: &mut BTreeMap<String, Capture>,
    summaries: &mut UtilitySummary<usize, BTreeMap<String, Capture>>,
    framed: bool,
) {
    fn pattern_variables(node: &PatternNode, variables: &mut BTreeMap<String, Capture>) {
        match node {
            PatternNode::MetaVar { meta_var } => {
                let (name, capture) = match meta_var {
                    MetaVariable::Capture(name, _) => (name, Capture::Single),
                    MetaVariable::MultiCapture(name) => (name, Capture::Multiple),
                    MetaVariable::Dropped(_) | MetaVariable::Multiple => return,
                };
                merge_capture(variables, name, capture);
            }
            PatternNode::Terminal { .. } => {}
            PatternNode::Internal { children, .. } => {
                for child in children {
                    pattern_variables(child, variables);
                }
            }
        }
    }
    match matcher {
        AstMatcher::Pattern(pattern) => pattern_variables(&pattern.node, variables),
        // Comparisons and counts bind nothing; counts are not upstream's.
        AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Never
        | AstMatcher::Compare(_)
        | AstMatcher::Count(_) => {}
        AstMatcher::Semantic(semantic) => {
            semantic::collect_in(semantic, variables, summaries, framed)
        }
        AstMatcher::Util(util) => {
            if framed {
                for (name, capture) in framed_variables(util, summaries) {
                    merge_capture(variables, &name, capture);
                }
            }
        }
        AstMatcher::Call(call) => {
            let inner = framed_variables(&call.util, summaries);
            for (param, caller) in &call.args {
                if let Some(capture) = inner.get(param) {
                    merge_capture(variables, caller, *capture);
                }
            }
        }
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                collect_in(item, variables, summaries, framed);
            }
        }
        AstMatcher::Not(item)
        | AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => collect_in(item, variables, summaries, framed),
        AstMatcher::Field(relation) => collect_in(&relation.rule, variables, summaries, framed),
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                collect_in(rule, variables, summaries, framed);
            }
        }
        AstMatcher::Constrained(core) => {
            collect_in(&core.rule, variables, summaries, framed);
            for (_, constraint) in &core.constraints {
                collect_in(constraint, variables, summaries, framed);
            }
        }
    }
}

/// The metavariables a utility's rule defines in a frame, in the utility's own names.
fn framed_variables(
    util: &UtilRef,
    summaries: &mut UtilitySummary<usize, BTreeMap<String, Capture>>,
) -> BTreeMap<String, Capture> {
    summaries.summarize(util.index, BTreeMap::new(), |summaries| {
        #[cfg(test)]
        record_walk(Walk::CaptureVariables);
        let mut summary = BTreeMap::new();
        util.with_target(|rule| collect_in(rule, &mut summary, summaries, true));
        summary
    })
}
