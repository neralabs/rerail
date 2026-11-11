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
use std::sync::Arc;
use std::collections::{BTreeMap, BTreeSet};

/// How deep utilities may nest when inlined; deeper (or recursive) ones are refused.
const MAX_INLINED_UTIL_DEPTH: usize = 32;

pub(in crate::code_rule_dsl) enum MatchTree {
    Kind(u16),
    /// A grammar supertype's members, as the tape records them (`SUPERTYPE`), with the concrete
    /// kinds they can have.
    Supertype(u16, Arc<BTreeSet<u16>>),
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
    /// `count`: how many nodes along the relation's walk `rule` matches lies in `min..=max`.
    /// Along `has` to the end, every matching descendant counts.
    Count {
        axis: Axis,
        rule: Box<MatchTree>,
        stop: Stop,
        min: usize,
        max: Option<usize>,
    },
    /// A semantic predicate (O24, decision 0030): the nodes its producer reports, before any pass,
    /// with each of its sub-rules lowered as a tree of its own whose label the reported target must have.
    Fact(Arc<Fact>),
    Never,
}

/// A semantic leaf as a fact (O24, decision 0030). The producer is W's predicate with each sub-rule
/// replaced by a capture of the node it would be tested on (`FACT_SLOT{i}`): it reads the file's scopes
/// and values and never runs a user rule. A node holds where the producer matches it and each slot's
/// target is in the label of `slots[i]`.
pub(in crate::code_rule_dsl) struct Fact {
    producer: AstMatcher,
    /// The sub-rules, by slot.
    pub(in crate::code_rule_dsl) slots: Vec<MatchTree>,
    /// The predicate's name (`resolves`, `resolves.binding`, …), for listings.
    pub(in crate::code_rule_dsl) name: &'static str,
    /// Its identity in a plan: the query's content digest, its dialect and the leaf's place in the
    /// lowering. Equal keys are equal producers, however often the query is compiled.
    pub(in crate::code_rule_dsl) key: String,
    /// The kinds a node it reports can have, where the predicate names them: its producer starts
    /// only there (`None`: any node).
    pub(in crate::code_rule_dsl) kinds: Option<BTreeSet<u16>>,
}

/// One node a fact's producer reports: the node, and each slot's target.
pub(in crate::code_rule_dsl) type FactRow = (u32, Vec<u32>);

/// A fact's findings on one file: rows with their slots' targets, or (a fact with no slot) its
/// nodes as bits.
#[derive(Clone)]
pub(in crate::code_rule_dsl) enum FactFound {
    Rows(Vec<FactRow>),
    Nodes(Vec<u64>),
}

impl FactFound {
    /// Each row: a node and its slots' targets.
    pub(in crate::code_rule_dsl) fn rows(&self) -> Box<dyn Iterator<Item = (u32, &[u32])> + '_> {
        match self {
            Self::Rows(rows) => Box::new(rows.iter().map(|(node, targets)| (*node, targets.as_slice()))),
            Self::Nodes(bits) => Box::new(bits.iter().enumerate().flat_map(|(word, &bits)| {
                (0..64).filter(move |bit| bits >> bit & 1 != 0).map(move |bit| (word as u32 * 64 + bit, &[][..]))
            })),
        }
    }
}

impl Fact {
    /// What the fact asks where its findings are bits (`nth_bits`): facts with equal questions find
    /// the same nodes. `None`: it has slots, another producer, or an `ofRule` with no identity.
    fn question(&self) -> Option<String> {
        match &self.producer {
            AstMatcher::NthChild(nth) if self.slots.is_empty() && (nth.of_rule.is_none() || nth.of_rule_key.is_some()) => {
                Some(format!("{:?}|{}|{}|{}|{:?}", nth.of_rule_key, nth.reverse, nth.position.step, nth.position.offset, self.kinds))
            }
            _ => None,
        }
    }

    /// Whether its findings on a file are a node set (`nth_bits`): a position among siblings
    /// (`nthChild`) with no slot, or the file's path (`filePath`).
    pub(in crate::code_rule_dsl) fn is_node_set(&self) -> bool {
        matches!(&self.producer, AstMatcher::NthChild(_) | AstMatcher::Semantic(super::Semantic::FilePath(_))) && self.slots.is_empty()
    }

    /// Its nodes on `tape` where they are a node set (`is_node_set`), from the tape's sibling
    /// positions; asked once per tape where another fact asks the same (`question`).
    pub(in crate::code_rule_dsl) fn nth_bits(&self, tape: &crate::code_rule_dsl::tape::TapeDoc, siblings: &super::nth_child::SiblingPositions) -> Option<Arc<Vec<u64>>> {
        // A `filePath` test is the document's: every node where its path passes, none where it fails or the run gave
        // the tape no path (W's `file_path()` with none).
        if let AstMatcher::Semantic(super::Semantic::FilePath(condition)) = &self.producer {
            let nodes = tape.transducer_len();
            let mut bits = vec![0u64; nodes.div_ceil(64)];
            if tape.path().is_some_and(|path| condition.holds(path)) {
                bits.iter_mut().for_each(|w| *w = !0);
                if nodes % 64 != 0
                    && let Some(last) = bits.last_mut()
                {
                    *last &= (1 << (nodes % 64)) - 1;
                }
            }
            return Some(Arc::new(bits));
        }
        let AstMatcher::NthChild(nth) = &self.producer else { return None };
        if !self.slots.is_empty() {
            return None;
        }
        let answer = || {
            let bits = nth.bits(tape, &siblings.of(nth, tape), self.kinds.as_ref());
            // `RERAIL_SETS_CHECK`: the bits against W's rows.
            static CHECK: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("RERAIL_SETS_CHECK").is_some());
            if *CHECK {
                let file = ast_grep_core::AstGrep::doc(tape.clone());
                let _scopes = super::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty());
                let starts = (0..tape.transducer_len() as u32).filter(|&node| self.kinds.as_ref().is_none_or(|kinds| kinds.contains(&tape.transducer_kind(node))));
                let rows = nth.rows(&file, starts);
                let ones = bits.iter().enumerate().flat_map(|(w, &b)| (0..64u32).filter(move |i| b >> i & 1 != 0).map(move |i| w as u32 * 64 + i)).collect::<Vec<_>>();
                if ones != rows {
                    eprintln!("SETS_CHECK differs on nthChild {}", self.key);
                }
            }
            bits
        };
        Some(match self.question() {
            Some(question) => siblings.answer(&question, answer),
            None => Arc::new(answer()),
        })
    }

    /// The findings of `file` (`rows`), as bits where they are a node set (`nth_bits`).
    pub(in crate::code_rule_dsl) fn found(
        &self,
        file: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
        starts: impl Iterator<Item = u32>,
        siblings: &super::nth_child::SiblingPositions,
    ) -> FactFound {
        match self.nth_bits(file.root().get_doc(), siblings) {
            Some(bits) => FactFound::Nodes(Arc::unwrap_or_clone(bits)),
            None => FactFound::Rows(self.rows(file, starts)),
        }
    }

    /// The rows of `file`: every node the producer matches, with its slots' targets. The predicate's
    /// verdicts are read as W's matcher reads them (`match_root`'s settings): an unknown one is no row.
    pub(in crate::code_rule_dsl) fn rows(
        &self,
        file: &ast_grep_core::AstGrep<crate::code_rule_dsl::tape::TapeDoc>,
        starts: impl Iterator<Item = u32>,
    ) -> Vec<FactRow> {
        use super::util::{THREE_VALUED, UNKNOWN, UTIL_DEPTH, UTIL_EXCEEDED, UTIL_STEPS_LEFT};
        use ast_grep_core::matcher::Matcher;
        if let AstMatcher::NthChild(nth) = &self.producer {
            return nth.rows(file, starts).into_iter().map(|node| (node, Vec::new())).collect();
        }
        let mut rows = Vec::new();
        super::resolve::CURRENT_CLAUSE.set(self as *const Self as usize);
        THREE_VALUED.set(true);
        for ordinal in starts {
            let node = super::node_at(file, ordinal);
            UTIL_DEPTH.set(0);
            UTIL_EXCEEDED.set(None);
            UTIL_STEPS_LEFT.set(usize::MAX);
            UNKNOWN.set(false);
            let mut env = std::borrow::Cow::Owned(ast_grep_core::meta_var::MetaVarEnv::new());
            if self.producer.match_node_with_env(node, &mut env).is_none() || UNKNOWN.get() {
                continue;
            }
            let targets = (0..self.slots.len())
                .map(|i| env.get_match(&slot_name(i)).map(|target| target.node_id() as u32))
                .collect::<Option<Vec<u32>>>();
            if let Some(targets) = targets {
                rows.push((ordinal, targets));
            }
        }
        UNKNOWN.set(false);
        rows
    }
}

fn slot_name(i: usize) -> String {
    format!("FACT_SLOT{i}")
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
            | Self::Supertype(..)
            | Self::Pattern(_)
            | Self::Constrained(..)
            | Self::Regex(_)
            | Self::Recursive(..)
            | Self::Recur(_)
            | Self::Count { .. }
            | Self::Fact(_)
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
            Self::Supertype(_, kinds) => Some(kinds.as_ref().clone()),
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
            Self::Fact(fact) => fact.kinds.clone(),
            Self::Regex(_) | Self::Not(_) | Self::Walk { .. } | Self::Recur(_) | Self::Count { .. } => {
                None
            }
        }
    }
}

/// An operator the lowering does not express, named for the coverage count.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct Unsupported(pub(in crate::code_rule_dsl) &'static str);

/// The match tree of a clause's matcher.
pub(in crate::code_rule_dsl) fn match_tree(grammar: &GrammarMatcher) -> Result<MatchTree, Unsupported> {
    tree_of(grammar, &grammar.matcher)
}

/// A condition on the whole document a matcher's rule holds as a conjunct of its root: one test per
/// document, not per node. The wave tests it before its batch and runs the clause only on the
/// documents where every one holds (03 §6, a file pre-filter).
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) enum DocumentGate {
    /// `filePath: {regex, notRegex}`: the document's path relative to its root.
    Path(Arc<crate::code_rule_dsl::language::TextCondition>),
    /// `inside: {all: [{kind: module}, {regex: R}], stopBy: end}`: the file's text matches `R` (the
    /// rule tests the module node's text, which every node but the module is inside). The ported
    /// checkers' quick "does the file mention X" test.
    Text(Regex),
}

impl DocumentGate {
    pub(in crate::code_rule_dsl) fn holds(&self, path: &str, text: &str) -> bool {
        match self {
            Self::Path(condition) => condition.holds(path),
            Self::Text(regex) => regex.is_match(text),
        }
    }
}

/// The document gates among the conjuncts of a matcher's root (`DocumentGate`), and the tree of the
/// rest. A `filePath` anywhere else is still refused. A module text test holds for every node but
/// the module, so it is a gate only where the rest cannot match the module itself: the conjuncts
/// include a `kind` other than `module`.
pub(in crate::code_rule_dsl) fn gated_match_tree(grammar: &GrammarMatcher) -> Result<(MatchTree, Vec<DocumentGate>), Unsupported> {
    use super::Semantic;
    let AstMatcher::All(_) = &grammar.matcher else { return Ok((match_tree(grammar)?, Vec::new())) };
    // The root's conjuncts, nested `all`s flattened (an option's case is often an `all` of path tests).
    fn conjuncts<'a>(matcher: &'a AstMatcher, out: &mut Vec<&'a AstMatcher>) {
        match matcher {
            AstMatcher::All(items) => items.iter().for_each(|item| conjuncts(item, out)),
            other => out.push(other),
        }
    }
    let mut items = Vec::new();
    conjuncts(&grammar.matcher, &mut items);
    let module = grammar.language.kind_id("module");
    let is_module = |matcher: &AstMatcher| match matcher {
        AstMatcher::Kind(kind) => kind.potential_kinds().is_some_and(|kinds| kinds.iter().eq([usize::from(module)])),
        _ => false,
    };
    let kind_other_than_module = items.iter().any(|item| matches!(item, AstMatcher::Kind(_)) && !is_module(item));
    let (mut gates, mut rest) = (Vec::new(), Vec::new());
    for item in items {
        match item {
            AstMatcher::Semantic(Semantic::FilePath(condition)) => gates.push(DocumentGate::Path(Arc::clone(condition))),
            AstMatcher::Inside(related, super::relation::StopBy::End) if kind_other_than_module => match related.rule() {
                AstMatcher::All(parts) => match parts.as_slice() {
                    [kind, AstMatcher::Regex(regex)] if is_module(kind) => gates.push(DocumentGate::Text(regex.clone())),
                    _ => rest.push((*item).clone()),
                },
                _ => rest.push((*item).clone()),
            },
            other => rest.push((*other).clone()),
        }
    }
    if gates.is_empty() {
        return Ok((match_tree(grammar)?, gates));
    }
    let rest = if rest.len() == 1 { rest.pop().expect("one conjunct") } else { AstMatcher::All(rest) };
    Ok((tree_of(grammar, &rest)?, gates))
}

fn tree_of(grammar: &GrammarMatcher, matcher: &AstMatcher) -> Result<MatchTree, Unsupported> {
    if grammar.transform.is_some() {
        return Err(Unsupported("transform"));
    }
    let mut lowering = Lowering { three_valued: grammar.three_valued, source: (grammar.source.0, Some(grammar.source.1)), ..Lowering::default() };
    let tree = lowering.tree(matcher, true)?;
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
    /// Whether the clause reads unknown verdicts (`GrammarMatcher::three_valued`): a fact that may be
    /// unknown is then exact only outside `not`.
    three_valued: bool,
    /// The query's content digest and dialect (`GrammarMatcher::source`), and the facts lowered so
    /// far: a fact's key.
    source: ([u8; 32], Option<crate::code_rule_dsl::language::GrammarDialect>),
    facts: usize,
    /// The kinds an `all`'s `kind` conjuncts allow, while its `nthChild` conjunct is lowered.
    conjunct_kinds: Option<BTreeSet<u16>>,
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
            AstMatcher::Supertype(supertype) => MatchTree::Supertype(supertype.class, Arc::clone(&supertype.kinds)),
            AstMatcher::Regex(regex) => MatchTree::Regex(regex.clone()),
            AstMatcher::All(matchers) => {
                // An `nthChild` beside a `kind` holds only at nodes of that kind: its fact's
                // producer starts only there (not at every node).
                let kinds = matchers
                    .iter()
                    .filter_map(|m| match m {
                        AstMatcher::Kind(kind) => kind.potential_kinds().map(|ks| ks.iter().filter_map(|k| u16::try_from(k).ok()).collect::<BTreeSet<u16>>()),
                        _ => None,
                    })
                    .reduce(|a, b| a.intersection(&b).copied().collect());
                let trees = matchers
                    .iter()
                    .map(|matcher| {
                        if matches!(matcher, AstMatcher::NthChild(_)) {
                            self.conjunct_kinds = kinds.clone();
                        }
                        let tree = self.tree(matcher, shares);
                        self.conjunct_kinds = None;
                        tree
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                MatchTree::All(trees)
            }
            // Only the arm that matches binds into the environment (ast-grep's `any`), so arms never share a
            // binding: each arm counts from what was bound before the `any`, and the `any` binds the most any arm does.
            AstMatcher::Any(matchers) => {
                let before = self.binds.clone();
                let mut most = before.clone();
                let mut trees = Vec::with_capacity(matchers.len());
                for matcher in matchers {
                    self.binds = before.clone();
                    trees.push(self.tree(matcher, shares)?);
                    for (name, &uses) in &self.binds {
                        let kept = most.entry(name.clone()).or_default();
                        *kept = (*kept).max(uses);
                    }
                }
                self.binds = most;
                MatchTree::Any(trees)
            }
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
            // A node's position among its parent's named children (of `ofRule`): a fact W's matcher
            // reports per node (two-valued, binding nothing), read by the walks as a label.
            AstMatcher::NthChild(_) => {
                let (digest, dialect) = &self.source;
                let key = format!("{}|{dialect:?}|{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>(), self.facts);
                self.facts += 1;
                let kinds = self.conjunct_kinds.take();
                MatchTree::Fact(Arc::new(Fact { producer: matcher.clone(), slots: Vec::new(), name: "nthChild", key, kinds }))
            }
            AstMatcher::Call(_) => return Err(Unsupported("utility call")),
            AstMatcher::Constrained(constrained) => self.constrained(constrained, shares)?,
            AstMatcher::Compare(_) => return Err(Unsupported("compare")),
            AstMatcher::Count(count) => self.count(count, shares)?,
            AstMatcher::Semantic(semantic) => return self.fact(semantic, shares),
            AstMatcher::Never => MatchTree::Never,
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => return Err(Unsupported("three-valued")),
        })
    }

    /// A semantic predicate as a fact (O24): its sub-rules that are tested on one node each (a
    /// binding's site, value and annotation) become slots; one tested on many (an occurrence,
    /// `redefines`, `formatArgs`' fields) is refused, as are predicates over other trees (`parse`,
    /// `host`, `embedded`), the workspace (`calls`), and those that bind.
    fn fact(&mut self, semantic: &super::Semantic, shares: bool) -> Result<MatchTree, Unsupported> {
        use super::Semantic;
        let name = semantic_name(semantic);
        if !semantic.binds_nothing() {
            return Err(Unsupported(match semantic {
                Semantic::Arguments(_) => "arguments binds",
                Semantic::FormatArgs(_) => "formatArgs binds",
                _ => name,
            }));
        }
        let mut producer = semantic.clone();
        let mut rules: Vec<AstMatcher> = Vec::new();
        let slot = |rule: &mut Option<AstMatcher>, rules: &mut Vec<AstMatcher>| {
            if let Some(taken) = rule.take() {
                *rule = Some(AstMatcher::Semantic(Semantic::Capture(slot_name(rules.len()))));
                rules.push(taken);
            }
        };
        let two_valued = match &mut producer {
            Semantic::Resolves(resolves) => !super::semantic::flow(resolves.language),
            // The document's path, one verdict per document (`Fact::nth_bits`): every node, or none.
            Semantic::FilePath(_) => {
                let (digest, dialect) = &self.source;
                let key = format!("{}|{dialect:?}|{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>(), self.facts);
                self.facts += 1;
                return Ok(MatchTree::Fact(Arc::new(Fact { producer: AstMatcher::Semantic(producer), slots: Vec::new(), name, key, kinds: None })));
            }
            // A gap to a capture (`to: capture.L`) reads a binding another leaf makes: the fact's
            // producer runs with no captures, so the gap would never hold.
            Semantic::Layout(layout) if layout.reads().next().is_some() => return Err(Unsupported("layout reads a capture")),
            Semantic::Value(_) | Semantic::Layout(_) | Semantic::Arguments(_) => false,
            // A token the grammar implies is in a gap or not: never unknown (t31).
            Semantic::Gap(_) => true,
            Semantic::Binding(test) => {
                if test.occurrence.is_some() {
                    return Err(Unsupported("resolves.binding occurrence"));
                }
                slot(&mut test.site, &mut rules);
                slot(&mut test.value, &mut rules);
                slot(&mut test.annotation, &mut rules);
                false
            }
            Semantic::Binds(test) => {
                if test.occurrence.is_some() || test.redefines.is_some() {
                    return Err(Unsupported("binds occurrence or redefines"));
                }
                slot(&mut test.site, &mut rules);
                slot(&mut test.value, &mut rules);
                slot(&mut test.annotation, &mut rules);
                false
            }
            _ => return Err(Unsupported(name)),
        };
        if !two_valued && self.three_valued && self.path.contains(&Step::Not) {
            return Err(Unsupported("unknown fact under not"));
        }
        let slots = rules.iter().map(|rule| self.tree(rule, shares)).collect::<Result<Vec<_>, _>>()?;
        let kinds = candidate_kinds(&producer);
        let (digest, dialect) = &self.source;
        let key = format!("{}|{dialect:?}|{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>(), self.facts);
        self.facts += 1;
        Ok(MatchTree::Fact(Arc::new(Fact { producer: AstMatcher::Semantic(producer), slots, name, key, kinds })))
    }
}

/// The kinds a node a predicate holds at can have, read off the predicate's own kind sets (each
/// unwraps parentheses first, so their kinds are included); `None` where it may hold at any node.
fn candidate_kinds(semantic: &super::Semantic) -> Option<BTreeSet<u16>> {
    use super::Semantic;
    let union = |sets: &[&BTreeSet<u16>]| sets.iter().flat_map(|set| set.iter().copied()).collect::<BTreeSet<u16>>();
    match semantic {
        Semantic::Resolves(r) if !r.item && !r.seen_before => {
            Some(union(&[&r.reference_kinds, &r.attribute_kinds, &r.paren_kinds, &r.this_kinds, &r.dotted_kinds, &r.string_kinds]))
        }
        Semantic::Binding(test) if test.write_last => Some(union(&[&test.reference_kinds, &test.paren_kinds, &test.name_kinds])),
        Semantic::Binding(test) => Some(union(&[&test.reference_kinds, &test.paren_kinds])),
        Semantic::Binds(test) => Some(test.name_kinds.clone()),
        Semantic::Arguments(arguments) => Some(arguments.call_kinds.clone()),
        _ => None,
    }
}

/// A semantic predicate's name, as refusals count it.
fn semantic_name(semantic: &super::Semantic) -> &'static str {
    use super::Semantic;
    match semantic {
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
        Semantic::Gap(_) => "gap",
        Semantic::FilePath(_) => "file path",
        Semantic::Calls(_) => "calls",
        Semantic::ImportTarget(_) => "importTarget",
    }
}

impl Lowering {
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

    /// `count` without `as`: the relation's walk, its rule and stop as a `Walk`'s. A bound count
    /// fails where more nodes match, so recursion through it is treated as through `not`.
    fn count(&mut self, count: &super::count::Count, shares: bool) -> Result<MatchTree, Unsupported> {
        if count.binding.is_some() {
            return Err(Unsupported("count as"));
        }
        let (axis, related, stop) = match &count.relation {
            AstMatcher::Inside(related, stop) => (Axis::Inside, related, stop),
            AstMatcher::Has(related, stop) => (Axis::Has, related, stop),
            AstMatcher::Precedes(related, stop) => (Axis::Precedes, related, stop),
            AstMatcher::Follows(related, stop) => (Axis::Follows, related, stop),
            _ => return Err(Unsupported("count relation")),
        };
        self.path.push(Step::Not);
        let walk = self.walk(axis, None, related.rule(), stop, shares);
        self.path.pop();
        let MatchTree::Walk { rule, stop, .. } = walk? else {
            unreachable!("a walk")
        };
        Ok(MatchTree::Count {
            axis,
            rule,
            stop,
            min: count.min,
            max: count.max,
        })
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
