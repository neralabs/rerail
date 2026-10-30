//! A clause's matcher as the match tree the packed transducer compiler lowers (06-first-slice S1,
//! `transducer::packed::compile::matcher`). Only the operators whose verdict at a node is the
//! node's alone are read: no binding one leaf makes and another reads (except a pattern's own
//! constraints), no three-valued fact (only the refused leaves are ever unknown). The others are
//! refused by name, so that a clause is lowered whole or not at all.

use super::relation::StopBy;
use super::{AstMatcher, GrammarMatcher};
use ast_grep_core::MatchStrictness;
use ast_grep_core::matcher::{Matcher, Pattern};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

/// How deep utilities may nest when inlined; deeper (or recursive) ones are refused.
const MAX_INLINED_UTIL_DEPTH: usize = 32;

pub(in crate::code_rule_dsl) enum MatchTree {
    Kind(u16),
    Pattern(Pattern),
    /// A pattern whose single-node captures must match these rules (`constraints`).
    Constrained(Pattern, Vec<(String, MatchTree)>),
    Regex(Regex),
    All(Vec<MatchTree>),
    Any(Vec<MatchTree>),
    Not(Box<MatchTree>),
    /// A relation: the nodes along `axis` from the node, as `stop` bounds the walk, one of which
    /// `rule` matches. With a `field` (`inside`, `has`), the path enters through that field.
    Walk {
        axis: Axis,
        field: Option<u16>,
        rule: Box<MatchTree>,
        stop: Stop,
    },
    /// A utility that reaches itself: its nodes are the least fixpoint of `tree`, where
    /// `Recur(id)` stands for them.
    Recursive(usize, Box<MatchTree>),
    Recur(usize),
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum Axis {
    Inside,
    Has,
    Precedes,
    Follows,
}

pub(in crate::code_rule_dsl) enum Stop {
    Neighbor,
    End,
    /// Inclusive: the stop node is still tested; matched in a fresh environment.
    Rule(Box<MatchTree>),
}

impl MatchTree {
    /// Whether testing this tree moves off the node (a relation anywhere in it).
    pub(in crate::code_rule_dsl) fn walks(&self) -> bool {
        match self {
            Self::Kind(_)
            | Self::Pattern(_)
            | Self::Constrained(..)
            | Self::Regex(_)
            | Self::Recursive(..)
            | Self::Recur(_)
            | Self::Never => false,
            Self::All(trees) | Self::Any(trees) => trees.iter().any(Self::walks),
            Self::Not(tree) => tree.walks(),
            Self::Walk { .. } => true,
        }
    }
}

impl MatchTree {
    /// The kinds a node must have for the tree to match it, where they are known: the anchor a
    /// label pass starts from.
    pub(in crate::code_rule_dsl) fn kinds(&self) -> Option<BTreeSet<u16>> {
        match self {
            Self::Kind(kind) => Some(BTreeSet::from([*kind])),
            Self::Pattern(pattern) | Self::Constrained(pattern, _) => pattern
                .potential_kinds()
                .map(|kinds| kinds.iter().filter_map(|kind| u16::try_from(kind).ok()).collect()),
            Self::All(trees) => trees.iter().filter_map(Self::kinds).reduce(|left, right| {
                left.intersection(&right).copied().collect()
            }),
            Self::Any(trees) => trees
                .iter()
                .map(Self::kinds)
                .reduce(|left, right| Some(left?.union(&right?).copied().collect()))
                .unwrap_or_else(|| Some(BTreeSet::new())),
            Self::Recursive(_, tree) => tree.kinds(),
            Self::Never => Some(BTreeSet::new()),
            Self::Regex(_) | Self::Not(_) | Self::Walk { .. } | Self::Recur(_) => None,
        }
    }
}

/// An operator the lowering does not express, named for the coverage count.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct Unsupported(pub(in crate::code_rule_dsl) &'static str);

/// The match tree of a clause's matcher.
pub(in crate::code_rule_dsl) fn match_tree(grammar: &GrammarMatcher) -> Result<MatchTree, Unsupported> {
    if grammar.transform.is_some() {
        return Err(Unsupported("transform"));
    }
    let mut lowering = Lowering::default();
    let tree = lowering.tree(&grammar.matcher, true)?;
    if lowering.binds.values().any(|&uses| uses > 1) {
        return Err(Unsupported("shared binding"));
    }
    Ok(tree)
}

#[derive(Default)]
struct Lowering {
    /// Each metavariable a pattern leaf binds, by how many leaves bind it: one bound twice is a
    /// join between the leaves, which the node labels do not carry.
    binds: BTreeMap<String, usize>,
    /// The utilities being inlined (table, index).
    utils: Vec<(usize, usize)>,
    /// The utilities found to reach themselves, by recursion id.
    recursive: Vec<(usize, usize)>,
    /// What the tree being lowered sits under, outermost first.
    path: Vec<Step>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Step {
    Util((usize, usize)),
    Not,
    Walk(Axis),
}

impl Lowering {
    /// `shares`: whether the tree reads the caller's environment (a stop rule's does not).
    fn tree(&mut self, matcher: &AstMatcher, shares: bool) -> Result<MatchTree, Unsupported> {
        Ok(match matcher {
            AstMatcher::Pattern(pattern) => {
                self.bind(pattern, shares)?;
                MatchTree::Pattern(pattern.clone())
            }
            AstMatcher::Kind(kind) => {
                let kinds = kind
                    .potential_kinds()
                    .ok_or(Unsupported("kind"))?
                    .iter()
                    .collect::<Vec<_>>();
                match kinds[..] {
                    [kind] => MatchTree::Kind(u16::try_from(kind).map_err(|_| Unsupported("kind"))?),
                    _ => return Err(Unsupported("kind")),
                }
            }
            AstMatcher::Regex(regex) => MatchTree::Regex(regex.clone()),
            AstMatcher::All(matchers) => MatchTree::All(self.trees(matchers, shares)?),
            AstMatcher::Any(matchers) => MatchTree::Any(self.trees(matchers, shares)?),
            AstMatcher::Not(matcher) => {
                self.path.push(Step::Not);
                let tree = self.tree(matcher, shares);
                self.path.pop();
                MatchTree::Not(Box::new(tree?))
            }
            AstMatcher::Inside(related, stop) => self.walk(Axis::Inside, None, related.rule(), stop, shares)?,
            AstMatcher::Has(related, stop) => self.walk(Axis::Has, None, related.rule(), stop, shares)?,
            AstMatcher::Precedes(related, stop) => {
                self.walk(Axis::Precedes, None, related.rule(), stop, shares)?
            }
            AstMatcher::Follows(related, stop) => {
                self.walk(Axis::Follows, None, related.rule(), stop, shares)?
            }
            AstMatcher::Field(relation) => {
                let axis = if relation.inside { Axis::Inside } else { Axis::Has };
                self.walk(axis, Some(relation.field), &relation.rule, &relation.stop, shares)?
            }
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index);
                if self.utils.contains(&key) {
                    self.well_founded(key)?;
                    let id = match self.recursive.iter().position(|&known| known == key) {
                        Some(id) => id,
                        None => {
                            self.recursive.push(key);
                            self.recursive.len() - 1
                        }
                    };
                    return Ok(MatchTree::Recur(id));
                }
                if self.utils.len() >= MAX_INLINED_UTIL_DEPTH {
                    return Err(Unsupported("utility depth"));
                }
                self.utils.push(key);
                self.path.push(Step::Util(key));
                let tree = util
                    .with_target(|rule| self.tree(rule, shares))
                    .ok_or(Unsupported("utility unavailable"));
                self.path.pop();
                self.utils.pop();
                let tree = tree?;
                let tree = tree?;
                match self.recursive.iter().position(|&known| known == key) {
                    Some(id) => MatchTree::Recursive(id, Box::new(tree)),
                    None => tree,
                }
            }
            AstMatcher::NthChild(_) => return Err(Unsupported("nthChild")),
            AstMatcher::Call(_) => return Err(Unsupported("utility call")),
            AstMatcher::Constrained(constrained) => self.constrained(constrained, shares)?,
            AstMatcher::Compare(_) => return Err(Unsupported("compare")),
            AstMatcher::Count(_) => return Err(Unsupported("count")),
            AstMatcher::Semantic(semantic) => {
                use super::Semantic;
                return Err(Unsupported(match semantic {
                    Semantic::Resolves(_) => "resolves",
                    Semantic::Binding(_) => "resolves.binding",
                    Semantic::Binds(_) => "binds",
                    Semantic::Value(_) => "value",
                    Semantic::Arguments(_) => "arguments",
                    Semantic::Parse(_) => "parse",
                    Semantic::Host(_) => "host",
                    Semantic::Embedded(_) => "embedded",
                    Semantic::Capture(_) => "capture",
                    Semantic::FormatArgs(_) => "formatArgs",
                    Semantic::Layout(_) => "layout",
                    Semantic::FilePath(_) => "file path",
                    Semantic::Calls(_) => "calls",
                }));
            }
            AstMatcher::Never => MatchTree::Never,
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => return Err(Unsupported("three-valued")),
        })
    }

    /// A recursive reference: through `not` (or a stop rule's bound), only where every walk since
    /// the utility was entered moves forward in preorder (`has`, `precedes`), so that each node's
    /// verdict reads only later nodes' and the recursion has one answer (structural descent as
    /// the termination proof). Without negation, the least fixpoint is the answer.
    fn well_founded(&self, key: (usize, usize)) -> Result<(), Unsupported> {
        let entered = self
            .path
            .iter()
            .rposition(|step| *step == Step::Util(key))
            .ok_or(Unsupported("recursive utility"))?;
        let since = &self.path[entered + 1..];
        if !since.contains(&Step::Not) {
            return Ok(());
        }
        let mut walks = since.iter().filter_map(|step| match step {
            Step::Walk(axis) => Some(*axis),
            _ => None,
        });
        let first = walks.next();
        if first.is_some() && first.into_iter().chain(walks).all(|axis| matches!(axis, Axis::Has | Axis::Precedes)) {
            Ok(())
        } else {
            Err(Unsupported("recursion through not"))
        }
    }

    /// `constraints`: each constrained metavariable must be bound by a pattern that is the rule or
    /// one of its top-level conjuncts, whose binding at the node is then the only one; the
    /// constraint moves into that pattern. A variable bound deeper, where which node binds it
    /// first decides, is refused.
    fn constrained(&mut self, constrained: &super::Constrained, shares: bool) -> Result<MatchTree, Unsupported> {
        let conjuncts = match &constrained.rule {
            AstMatcher::All(matchers) => matchers.iter().collect::<Vec<_>>(),
            rule => vec![rule],
        };
        let mut trees = Vec::with_capacity(conjuncts.len());
        let mut placed = Vec::new();
        for conjunct in conjuncts {
            if let AstMatcher::Pattern(pattern) = conjunct {
                let defined = pattern.defined_vars();
                let mine = constrained
                    .constraints
                    .iter()
                    .filter(|(name, _)| defined.contains(name.as_str()))
                    .collect::<Vec<_>>();
                if !mine.is_empty() {
                    self.bind(pattern, shares)?;
                    let constraints = mine
                        .iter()
                        .map(|(name, rule)| Ok((name.clone(), self.tree(rule, shares)?)))
                        .collect::<Result<Vec<_>, _>>()?;
                    placed.extend(mine.iter().map(|(name, _)| name.as_str()));
                    trees.push(MatchTree::Constrained(pattern.clone(), constraints));
                    continue;
                }
            }
            trees.push(self.tree(conjunct, shares)?);
        }
        if constrained.constraints.iter().any(|(name, _)| !placed.contains(&name.as_str())) {
            return Err(Unsupported("constraints"));
        }
        Ok(match <[MatchTree; 1]>::try_from(trees) {
            Ok([tree]) => tree,
            Err(trees) => MatchTree::All(trees),
        })
    }

    fn bind(&mut self, pattern: &Pattern, shares: bool) -> Result<(), Unsupported> {
        // The packed pattern matches as ast-grep's default (`smart`) strictness.
        if !matches!(pattern.strictness, MatchStrictness::Smart) {
            return Err(Unsupported("pattern strictness"));
        }
        if shares {
            for name in pattern.defined_vars() {
                if !name.starts_with('_') {
                    *self.binds.entry(name.to_owned()).or_default() += 1;
                }
            }
        }
        Ok(())
    }

    fn trees(&mut self, matchers: &[AstMatcher], shares: bool) -> Result<Vec<MatchTree>, Unsupported> {
        matchers.iter().map(|matcher| self.tree(matcher, shares)).collect()
    }

    fn walk(
        &mut self,
        axis: Axis,
        field: Option<u16>,
        rule: &AstMatcher,
        stop: &StopBy,
        shares: bool,
    ) -> Result<MatchTree, Unsupported> {
        // A stop rule bounds the walk by its complement.
        self.path.push(Step::Walk(axis));
        self.path.push(Step::Not);
        let stop = match stop {
            StopBy::Neighbor => Ok(Stop::Neighbor),
            StopBy::End => Ok(Stop::End),
            StopBy::Rule(rule) => self.tree(rule, false).map(|rule| Stop::Rule(Box::new(rule))),
        };
        self.path.pop();
        let rule = stop.and_then(|stop| Ok((stop, self.tree(rule, shares)?)));
        self.path.pop();
        let (stop, rule) = rule?;
        Ok(MatchTree::Walk {
            axis,
            field,
            rule: Box::new(rule),
            stop,
        })
    }
}
