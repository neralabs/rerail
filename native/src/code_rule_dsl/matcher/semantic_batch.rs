//! Binding facts and ordered child demands for the native cohort executor.
//!
//! No function here evaluates an AstMatcher. The caller owns trial environments, clause/TLS
//! scope, ordering and suspension. In particular, a demand's guard path travels with the work;
//! it must not be installed as a thread-local guard across a batch suspension.

use super::binding::Resolution;
use super::binds::{self, NameSite};
use super::{AstMatcher, BindingTest, Binds};
use crate::code_rule_dsl::value::NodeRef;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Verdict {
    Pass,
    Fail,
    Unknown,
}

impl Verdict {
    fn pre(pre: u8) -> Self {
        match pre {
            binds::PASSES => Self::Pass,
            binds::OPEN => Self::Unknown,
            _ => Self::Fail,
        }
    }
}

/// A binding cycle is identified by the actual child rule, not just its syntax or utility ID.
pub(super) type GuardKey = (usize, usize);
pub(super) type GuardPath = Vec<GuardKey>;

pub(super) struct Child<'r, 't, D: Doc> {
    pub(super) rule: &'r AstMatcher,
    /// A missing occurrence still consumes a visit; a missing binding target does not.
    pub(super) target: Option<Node<'t, D>>,
    pub(super) occurrence: bool,
}

pub(super) enum Started<'t, D: Doc> {
    Ready {
        node: Node<'t, D>,
        guards: GuardPath,
    },
    Missing,
    Cycle,
    Exhausted,
}

impl<'r, 't, D: Doc> Child<'r, 't, D> {
    /// Call exactly when this ordered demand is reached, under its clause's leaf scope.
    /// `Exhausted` is a resource stop, not a logical failure. Sibling demands reuse the parent's
    /// path; only descendants of Ready inherit the returned extended path.
    pub(super) fn start(&self, parents: &[GuardKey]) -> Started<'t, D> {
        if self.occurrence && !super::spend_util_step() {
            return Started::Exhausted;
        }
        let Some(node) = self.target.clone() else {
            return Started::Missing;
        };
        if !self.occurrence && !super::spend_util_step() {
            return Started::Exhausted;
        }
        let mut guards = parents.to_vec();
        if !self.occurrence {
            let key = (node.node_id(), self.rule as *const AstMatcher as usize);
            if guards.contains(&key) {
                return Started::Cycle;
            }
            guards.push(key);
        }
        Started::Ready { node, guards }
    }
}

/// Site, value, annotation, in that order. Check `complete` before starting any child: the
/// scalar implementation rejects a missing NodeRef in *any* listed slot before spending work.
/// A NodeRef that exists but cannot be found remains a missing demand at its ordered position.
pub(super) struct Children<'r, 't, D: Doc> {
    pub(super) complete: bool,
    pub(super) demands: Vec<Child<'r, 't, D>>,
}

fn children<'r, 't, D: Doc>(
    anchor: &Node<'t, D>,
    rules: [&'r Option<AstMatcher>; 3],
    nodes: [Option<NodeRef>; 3],
) -> Children<'r, 't, D> {
    let complete = rules.iter().zip(nodes).all(|(rule, node)| rule.is_none() || node.is_some());
    let demands = rules.into_iter().zip(nodes).filter_map(|(rule, node)| {
        Some(Child {
            rule: rule.as_ref()?,
            target: node.and_then(|target| target.find_near(anchor.root().root(), anchor)),
            occurrence: false,
        })
    }).collect();
    Children { complete, demands }
}

pub(super) struct Occurrences<'r, 't, D: Doc> {
    pub(super) demands: Vec<Child<'r, 't, D>>,
    /// A potentially capture-binding unknown ends the search before a later result can win.
    pub(super) strict_unknown: bool,
}

fn occurrences<'r, 't, D: Doc>(
    anchor: &Node<'t, D>,
    rule: &'r Option<AstMatcher>,
    nodes: &[NodeRef],
) -> Option<Occurrences<'r, 't, D>> {
    let rule = rule.as_ref()?;
    Some(Occurrences {
        strict_unknown: super::relation::may_bind(rule),
        demands: nodes.iter().map(|target| Child {
            rule,
            target: target.find_near(anchor.root().root(), anchor),
            occurrence: true,
        }).collect(),
    })
}

pub(super) enum BindingProjection<'r, 't, D: Doc> {
    No,
    Unknown,
    Found {
        own: Children<'r, 't, D>,
        occurrences: Option<Occurrences<'r, 't, D>>,
    },
}

pub(super) fn binding<'r, 't, D: Doc>(
    test: &'r BindingTest,
    node: &Node<'t, D>,
    env: &MetaVarEnv<'t, D>,
) -> BindingProjection<'r, 't, D> {
    match test.resolve(node, env) {
        Resolution::No => BindingProjection::No,
        Resolution::Unknown => BindingProjection::Unknown,
        Resolution::Found { anchor, site, value, annotation, occurrences: refs } => {
            BindingProjection::Found {
                own: children(&anchor, [&test.site, &test.value, &test.annotation], [site, value, annotation]),
                occurrences: occurrences(&anchor, &test.occurrence, &refs),
            }
        }
    }
}

/// Retains shared file tables; constructing it does not start a new file or clause scope.
pub(super) struct BindsProjection<'r, 't, D: Doc> {
    test: &'r Binds,
    anchor: Node<'t, D>,
    site: NameSite,
}

pub(super) struct Candidate<'r, 't, D: Doc> {
    pub(super) ordinal: i64,
    pub(super) pre: Verdict,
    pub(super) own: Children<'r, 't, D>,
    pub(super) occurrences: Option<Occurrences<'r, 't, D>>,
    /// Python shadow nodes are checked only after own children and occurrences pass.
    pub(super) shadow: Verdict,
    pub(super) previous: Option<Children<'r, 't, D>>,
}

pub(super) struct Other<'r, 't, D: Doc> {
    pub(super) pre: Verdict,
    pub(super) children: Children<'r, 't, D>,
}

pub(super) fn binds<'r, 't, D: Doc>(
    test: &'r Binds,
    node: &Node<'t, D>,
) -> Option<BindsProjection<'r, 't, D>> {
    Some(BindsProjection { test, anchor: node.clone(), site: test.at(node)? })
}

impl<'r, 't, D: Doc> BindsProjection<'r, 't, D> {
    /// Filters read the original caller environment, never a candidate's trial environment.
    pub(super) fn candidates(&self, caller: &MetaVarEnv<'t, D>) -> Vec<Candidate<'r, 't, D>> {
        let test = self.test;
        self.site.candidates(test, caller).map(|candidate| {
            let previous = test.redefines.as_deref().filter(|_| test.redefines_shadowed()).map(|r| {
                children(&self.anchor, [&r.site, &r.value, &r.annotation], candidate.previous)
            });
            Candidate {
                ordinal: candidate.ordinal,
                pre: Verdict::pre(candidate.pre),
                own: children(&self.anchor, [&test.site, &test.value, &test.annotation], candidate.nodes),
                occurrences: occurrences(&self.anchor, &test.occurrence, self.site.occurrences(candidate.ordinal)),
                shadow: Verdict::pre(candidate.shadow),
                previous,
            }
        }).collect()
    }

    /// Call after this candidate's own demands. The caller environment is the original one.
    /// The returned order includes failure/unknown sentinels needed by `every` and private
    /// members; do not filter them or deduplicate equal nodes.
    pub(super) fn others(&self, caller: &MetaVarEnv<'t, D>, ordinal: i64) -> Vec<Other<'r, 't, D>> {
        let Some(r) = self.test.redefines.as_deref().filter(|_| !self.test.redefines_shadowed()) else {
            return Vec::new();
        };
        self.site.others(self.test, caller, ordinal).map(|other| Other {
            pre: Verdict::pre(other.pre),
            children: children(&self.anchor, [&r.site, &r.value, &r.annotation], other.nodes),
        }).collect()
    }

    pub(super) fn every(&self) -> bool {
        self.test.redefines.as_deref().is_some_and(|r| r.every)
    }
}

/// Trial environments commit by replacement only on a winning branch. Rejected `each`
/// bindings count as a skipped alternative, not as a successful existential result.
pub(super) fn excluded<D: Doc>(env: &MetaVarEnv<'_, D>) -> bool {
    super::each::active() && super::each::excluded(env)
}

pub(super) enum SearchStep {
    Continue,
    Done(Verdict),
}

/// Ordered existential reduction used by occurrences and the outer non-private candidate
/// search. Feed only completed child/candidate verdicts; apply `excluded` before feeding Pass.
pub(super) struct Search {
    unknown: bool,
    strict_unknown: bool,
}

impl Search {
    pub(super) fn new(strict_unknown: bool) -> Self {
        Self { unknown: false, strict_unknown }
    }

    pub(super) fn push(&mut self, verdict: Verdict) -> SearchStep {
        match verdict {
            Verdict::Pass => SearchStep::Done(Verdict::Pass),
            Verdict::Fail => SearchStep::Continue,
            Verdict::Unknown => {
                self.unknown = true;
                if self.strict_unknown {
                    SearchStep::Done(Verdict::Unknown)
                } else {
                    SearchStep::Continue
                }
            }
        }
    }

    pub(super) fn finish(self) -> Verdict {
        if self.unknown { Verdict::Unknown } else { Verdict::Fail }
    }
}

/// Reduction of the `redefines` alternatives. A private member's OPEN precondition ends even
/// an existential search; an unknown child can be followed by a later successful alternative.
/// For `every`, accepted children share one evolving trial environment. Existential attempts
/// each start from the same incoming trial. Only feed Pass after checking `excluded`.
pub(super) struct RedefinitionSearch {
    every: bool,
    private: bool,
    unknown: bool,
}

impl RedefinitionSearch {
    pub(super) fn new(every: bool, ordinal: i64) -> Self {
        Self { every, private: ordinal == binds::PRIVATE, unknown: false }
    }

    /// A passing precondition means its children must run; a failing/unknown one can be
    /// reduced immediately with `push(verdict, true)`.
    pub(super) fn push(&mut self, verdict: Verdict, precondition: bool) -> SearchStep {
        match verdict {
            Verdict::Pass if self.every => SearchStep::Continue,
            Verdict::Pass => SearchStep::Done(Verdict::Pass),
            Verdict::Fail if self.every => SearchStep::Done(Verdict::Fail),
            Verdict::Fail => SearchStep::Continue,
            Verdict::Unknown if self.every || (self.private && precondition) => {
                SearchStep::Done(Verdict::Unknown)
            }
            Verdict::Unknown => {
                self.unknown = true;
                SearchStep::Continue
            }
        }
    }

    pub(super) fn finish(self) -> Verdict {
        if self.unknown {
            Verdict::Unknown
        } else if self.every {
            Verdict::Pass
        } else {
            Verdict::Fail
        }
    }
}
