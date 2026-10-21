use super::{AstMatcher, StopBy, UtilRef};
use crate::code_rule_dsl::budget;
use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules, with_adapter};
#[cfg(test)]
use crate::code_rule_dsl::test_support::{Walk, record_walk};
mod semantic;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::matcher::{Matcher, PatternNode, kind_utils};
use ast_grep_core::source::SgNode;
use ast_grep_core::{AstGrep, Doc, MatchStrictness, Node};
use foldhash::{HashMap, HashMapExt, HashSet};
use regex::Regex;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::sync::Arc;

// Keep the cursor at the current CST node. Neither Tree-sitter's Node::parent
// (which searches down from the root) nor ast-grep's DFS (which eagerly
// unwinds before yielding) is suitable for a bounded index build.
pub(in crate::code_rule_dsl) trait AnchorCursor<'tree> {
    type Node: Copy;
    fn current(&self) -> Self::Node;
    fn first_child(&mut self) -> bool;
    fn next_sibling(&mut self) -> bool;
    fn parent(&mut self) -> bool;
    fn field_id(&self) -> u16;
}

impl<'tree> AnchorCursor<'tree> for tree_sitter::TreeCursor<'tree> {
    type Node = tree_sitter::Node<'tree>;
    fn current(&self) -> Self::Node {
        self.node()
    }
    fn first_child(&mut self) -> bool {
        self.goto_first_child()
    }
    fn next_sibling(&mut self) -> bool {
        self.goto_next_sibling()
    }
    fn parent(&mut self) -> bool {
        self.goto_parent()
    }
    fn field_id(&self) -> u16 {
        self.field_id().map_or(0, |id| id.get())
    }
}

pub(in crate::code_rule_dsl) struct TapeCursor<'tree>(crate::code_rule_dsl::tape::TapeNode<'tree>);

impl<'tree> AnchorCursor<'tree> for TapeCursor<'tree> {
    type Node = crate::code_rule_dsl::tape::TapeNode<'tree>;
    fn current(&self) -> Self::Node {
        self.0
    }
    fn first_child(&mut self) -> bool {
        if let Some(child) = self.0.child(0) {
            self.0 = child;
            true
        } else {
            false
        }
    }
    fn next_sibling(&mut self) -> bool {
        if let Some(next) = self.0.next_sibling() {
            self.0 = next;
            true
        } else {
            false
        }
    }
    fn parent(&mut self) -> bool {
        if let Some(parent) = self.0.parent() {
            self.0 = parent;
            true
        } else {
            false
        }
    }
    fn field_id(&self) -> u16 {
        self.0.incoming_field()
    }
}

pub(in crate::code_rule_dsl) trait AnchorNode<'tree>:
    SgNode<'tree> + Copy
{
    type Cursor: AnchorCursor<'tree, Node = Self>;
    fn cursor(self) -> Self::Cursor;
    fn child_count(self) -> usize;
}

impl<'tree> AnchorNode<'tree> for tree_sitter::Node<'tree> {
    type Cursor = tree_sitter::TreeCursor<'tree>;
    fn cursor(self) -> Self::Cursor {
        self.walk()
    }
    fn child_count(self) -> usize {
        tree_sitter::Node::child_count(&self) as usize
    }
}

impl<'tree> AnchorNode<'tree> for crate::code_rule_dsl::tape::TapeNode<'tree> {
    type Cursor = TapeCursor<'tree>;
    fn cursor(self) -> Self::Cursor {
        TapeCursor(self)
    }
    fn child_count(self) -> usize {
        self.children().len()
    }
}

mod fields;
mod text;
use text::TextProbe;

#[cfg(test)]
mod tests;

// S073/S256: value predicates name node positions, not just candidate files.
// Plans only remove impossible roots. The matcher still produces all captures and witnesses at
// the roots they keep (`run_clause`), including for predicates answered by this index.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct AnchorPlan {
    root_kinds: Option<BTreeSet<u16>>,
    condition: Condition,
    exact: bool,
}

#[derive(Clone)]
enum Condition {
    Always,
    Pattern(Vec<Requirement>),
    Text {
        regex: Regex,
        probe: TextProbe,
    },
    All(Vec<AnchorPlan>),
    Any(Vec<AnchorPlan>),
    Not(Box<AnchorPlan>),
    Relation {
        inner: Box<AnchorPlan>,
        direction: Direction,
        recursive: bool,
        field: Option<u16>,
    },
    /// A utility's plan, shared by every reference to it: diamond paths reach one plan instead
    /// of each copying it. Its root kinds stay on the referencing plan, which the enclosing
    /// conjunction may narrow.
    Utility(Arc<AnchorPlan>),
}

#[derive(Clone, Copy)]
enum Direction {
    Has,
    Inside,
    Precedes,
    Follows,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
enum Required {
    Kind(u16),
    Terminal(u16, String),
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Requirement {
    value: Required,
    // Immediate parent first. Smart skips siblings, not ancestor levels.
    parents: Vec<Option<u16>>,
}

/// `matches` references inlined into one query's plan before the rest fall back to unrestricted.
const MAX_INLINED_UTILS: usize = 4096;

struct Inlining {
    /// Utilities whose plan is being compiled: a reference back into one plans as unrestricted,
    /// a sound superset. Completed plans are reused by every diamond path; a parameterized call
    /// has the same anchor plan as its utility.
    visiting: BTreeSet<usize>,
    completed: BTreeMap<usize, AnchorPlan>,
    budget: usize,
}

impl AnchorPlan {
    pub(in crate::code_rule_dsl) fn compile(matcher: &AstMatcher) -> Self {
        Self::compile_in(
            matcher,
            &mut Inlining {
                visiting: BTreeSet::new(),
                completed: BTreeMap::new(),
                budget: MAX_INLINED_UTILS,
            },
        )
    }

    /// Selects every root: the sound plan for a sub-rule that is not analysed.
    fn unrestricted() -> Self {
        Self {
            root_kinds: None,
            condition: Condition::Always,
            exact: false,
        }
    }

    fn compile_util(util: &UtilRef, inlining: &mut Inlining) -> Self {
        if let Some(plan) = inlining.completed.get(&util.index) {
            return plan.clone();
        }
        if inlining.budget == 0 || !inlining.visiting.insert(util.index) {
            return Self::unrestricted();
        }
        inlining.budget -= 1;
        #[cfg(test)]
        record_walk(Walk::AnchorInlining);
        let plan = util
            .with_target(|rule| Self::compile_in(rule, inlining))
            .unwrap_or_else(Self::unrestricted);
        // Keep relation constraints too: utility_selection memoizes the already evaluated scope,
        // and relation selection starts from kind/value/field postings instead of rematching roots.
        let plan = if matches!(plan.condition, Condition::Always) {
            plan
        } else {
            Self {
                root_kinds: plan.root_kinds.clone(),
                exact: plan.exact,
                condition: Condition::Utility(Arc::new(plan)),
            }
        };
        inlining.visiting.remove(&util.index);
        inlining.completed.insert(util.index, plan.clone());
        plan
    }

    fn compile_in(matcher: &AstMatcher, inlining: &mut Inlining) -> Self {
        let mut plan = match matcher {
            AstMatcher::Pattern(pattern) => {
                let mut required = BTreeSet::new();
                if matches!(pattern.strictness, MatchStrictness::Smart) {
                    collect_pattern(&pattern.node, &mut Vec::new(), &mut required);
                }
                Self {
                    root_kinds: pattern
                        .potential_kinds()
                        .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
                    condition: if required.is_empty() {
                        Condition::Always
                    } else {
                        Condition::Pattern(required.into_iter().collect())
                    },
                    exact: false,
                }
            }
            AstMatcher::Kind(kind) => Self {
                root_kinds: kind
                    .potential_kinds()
                    .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
                condition: Condition::Always,
                exact: true,
            },
            AstMatcher::Regex(regex) => Self {
                root_kinds: None,
                condition: Condition::Text {
                    regex: regex.clone(),
                    probe: TextProbe::compile(regex.as_str()),
                },
                exact: true,
            },
            // No root kind admits a never-match: its selection is empty, and a `not` over it
            // (exact) subtracts nothing.
            AstMatcher::Never => Self {
                root_kinds: Some(BTreeSet::new()),
                condition: Condition::Always,
                exact: true,
            },
            AstMatcher::All(items) => {
                let plans: Vec<_> = items
                    .iter()
                    .map(|item| Self::compile_in(item, inlining))
                    .collect();
                let mut kinds = None;
                for plan in &plans {
                    intersect_kinds(&mut kinds, plan.root_kinds.as_ref());
                }
                Self {
                    root_kinds: kinds,
                    exact: plans.iter().all(|plan| plan.exact),
                    condition: Condition::All(plans),
                }
            }
            AstMatcher::Any(items) => {
                let plans: Vec<_> = items
                    .iter()
                    .map(|item| Self::compile_in(item, inlining))
                    .collect();
                let kinds = plans.iter().try_fold(BTreeSet::new(), |mut union, plan| {
                    union.extend(plan.root_kinds.as_ref()?);
                    Some(union)
                });
                Self {
                    root_kinds: kinds,
                    exact: plans.iter().all(|plan| plan.exact),
                    condition: Condition::Any(plans),
                }
            }
            AstMatcher::Not(inner) => {
                let inner = Self::compile_in(inner, inlining);
                if inner.exact {
                    Self {
                        root_kinds: None,
                        exact: true,
                        condition: Condition::Not(Box::new(inner)),
                    }
                } else {
                    // Subtracting a superset would lose true negated matches.
                    Self {
                        root_kinds: None,
                        exact: false,
                        condition: Condition::Always,
                    }
                }
            }
            AstMatcher::Has(inner, stop)
            | AstMatcher::Inside(inner, stop)
            | AstMatcher::Precedes(inner, stop)
            | AstMatcher::Follows(inner, stop) => {
                let direction = match matcher {
                    AstMatcher::Has(..) => Direction::Has,
                    AstMatcher::Inside(..) => Direction::Inside,
                    AstMatcher::Precedes(..) => Direction::Precedes,
                    _ => Direction::Follows,
                };
                let inner = Self::compile_in(inner, inlining);
                // A stop rule only shortens the walk, so the unbounded relation is a superset:
                // sound to select with, but not exact (a `not` over it must not subtract).
                let bounded = matches!(stop, StopBy::Rule(_));
                Self {
                    root_kinds: None,
                    exact: inner.exact && !bounded,
                    condition: Condition::Relation {
                        inner: Box::new(inner),
                        direction,
                        recursive: !matches!(stop, StopBy::Neighbor),
                        field: None,
                    },
                }
            }
            // A position holds only where `ofRule` does (upstream's `potential_kinds`).
            AstMatcher::NthChild(nth) => match &nth.of_rule {
                Some(rule) => Self {
                    root_kinds: Self::compile_in(rule, inlining).root_kinds,
                    condition: Condition::Always,
                    exact: false,
                },
                None => Self::unrestricted(),
            },
            // A field narrows the relation's walk to a subset of its nodes: the relation's plan is a
            // superset, never exact.
            AstMatcher::Field(relation) => {
                let inner = Self::compile_in(&relation.rule, inlining);
                Self {
                    root_kinds: None,
                    exact: false,
                    condition: Condition::Relation {
                        inner: Box::new(inner),
                        direction: if relation.inside {
                            Direction::Inside
                        } else {
                            Direction::Has
                        },
                        // `has` with a stop rule tests the field child's children too.
                        recursive: !matches!(relation.stop, StopBy::Neighbor),
                        field: Some(relation.field),
                    },
                }
            }
            // A reference plans as its utility's plan, computed once and shared by every path that
            // reaches it (`compile_util`). Where a utility recurses (or the inlining budget runs
            // out) the plan selects every node, which keeps the enclosing plan a superset.
            AstMatcher::Util(util) => Self::compile_util(util, inlining),
            // A parameterized call plans as its utility: the frame changes bindings, not nodes.
            AstMatcher::Call(call) => Self::compile_util(&call.util, inlining),
            // A comparison depends on the bindings, and a count may hold with no related node at
            // all (`max` only): both select every root.
            AstMatcher::Compare(_) | AstMatcher::Count(_) => Self::unrestricted(),
            AstMatcher::Semantic(semantic) => semantic::compile_in(semantic, inlining),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => Self::unrestricted(),
            // Constraints only narrow the rule's matches: its plan is a superset, never exact.
            AstMatcher::Constrained(core) => {
                let mut plan = Self::compile_in(&core.rule, inlining);
                plan.exact = false;
                plan
            }
        };
        // Same-node conjunctions give their regexes a concrete kind domain.
        // Relations deliberately do not inherit that domain: they bind others.
        plan.restrict_root(None);
        plan
    }

    pub(in crate::code_rule_dsl) fn root_kinds(&self) -> Option<&BTreeSet<u16>> {
        self.root_kinds.as_ref()
    }

    /// Kind-only queries already use the tape's kind postings; they need no structural index.
    pub(in crate::code_rule_dsl) fn needs_index(&self) -> bool {
        match &self.condition {
            Condition::Always => false,
            Condition::All(plans) | Condition::Any(plans) => plans.iter().any(Self::needs_index),
            Condition::Utility(plan) => plan.needs_index(),
            _ => true,
        }
    }

    fn restrict_root(&mut self, kinds: Option<&BTreeSet<u16>>) {
        intersect_kinds(&mut self.root_kinds, kinds);
        match &mut self.condition {
            Condition::All(plans) | Condition::Any(plans) => {
                for plan in plans {
                    plan.restrict_root(self.root_kinds.as_ref());
                }
            }
            Condition::Not(plan) => plan.restrict_root(self.root_kinds.as_ref()),
            _ => {}
        }
    }

    /// Records the value postings selection reads. `domain` holds the root kinds an enclosing
    /// conjunction imposes on a shared utility plan, which `restrict_root` could not push into
    /// it; `utilities` visits each shared plan once per domain.
    fn collect_needs<'plan>(
        &'plan self,
        domain: Option<&BTreeSet<u16>>,
        utilities: &mut HashSet<(usize, Option<BTreeSet<u16>>)>,
        needs: &mut [Needs<'plan>],
        fields: &mut BTreeSet<u16>,
    ) {
        match &self.condition {
            Condition::Pattern(required) => {
                for requirement in required {
                    if let Required::Terminal(kind, text) = &requirement.value
                        && let Some(need) = needs.get_mut(usize::from(*kind))
                    {
                        need.terminals.insert(text);
                    }
                }
            }
            Condition::Text { probe, .. } => {
                for (kind, need) in needs.iter_mut().enumerate() {
                    let kind = kind as u16;
                    if [domain, self.root_kinds.as_ref()]
                        .into_iter()
                        .all(|kinds| kinds.is_none_or(|kinds| kinds.contains(&kind)))
                    {
                        need.text = true;
                        need.prefix |= probe.prefixes().is_some();
                    }
                }
            }
            // `restrict_root` already narrowed these children to this plan's root kinds.
            Condition::All(plans) | Condition::Any(plans) => {
                for plan in plans {
                    plan.collect_needs(domain, utilities, needs, fields);
                }
            }
            Condition::Not(plan) => plan.collect_needs(domain, utilities, needs, fields),
            // Relations bind other nodes: no root kind domain applies to them.
            Condition::Relation { inner, field, .. } => {
                fields.extend(field);
                inner.collect_needs(None, utilities, needs, fields);
            }
            Condition::Utility(plan) => {
                let mut kinds = domain.cloned();
                intersect_kinds(&mut kinds, self.root_kinds.as_ref());
                if utilities.insert((Arc::as_ptr(plan) as usize, kinds.clone())) {
                    #[cfg(test)]
                    record_walk(Walk::AnchorNeeds);
                    plan.collect_needs(kinds.as_ref(), utilities, needs, fields);
                }
            }
            Condition::Always => {}
        }
    }
}

/// The kinds whose value `reading` can know, as the grammar's symbol ids: those the decoder reads
/// and, following bindings, the names it follows.
fn value_kinds(reading: &super::Reading) -> BTreeSet<u16> {
    let (values, names) = with_adapter!(reading.family, A => (
        <<A as Adapter>::Value as ValueRules>::VALUE_KINDS,
        <<A as Adapter>::Value as ValueRules>::NAME_KINDS,
    ));
    let mut kinds = crate::code_rule_dsl::language::kind_ids(reading.language, values);
    if reading.follow {
        kinds.extend(crate::code_rule_dsl::language::kind_ids(
            reading.language,
            names,
        ));
    }
    kinds
}

fn intersect_kinds(left: &mut Option<BTreeSet<u16>>, right: Option<&BTreeSet<u16>>) {
    if let Some(right) = right {
        *left = Some(match left.take() {
            Some(left) => left.intersection(right).copied().collect(),
            None => right.clone(),
        });
    }
}

fn collect_pattern(
    node: &PatternNode,
    parents: &mut Vec<Option<u16>>,
    required: &mut BTreeSet<Requirement>,
) {
    let value = match node {
        PatternNode::Terminal {
            kind_id,
            text,
            is_named: true,
        } if !kind_utils::is_error_kind(*kind_id) => {
            Some(Required::Terminal(*kind_id, text.clone()))
        }
        PatternNode::Internal { kind_id, .. } if !kind_utils::is_error_kind(*kind_id) => {
            Some(Required::Kind(*kind_id))
        }
        _ => None,
    };
    if let Some(value) = value {
        required.insert(Requirement {
            value,
            parents: parents.iter().rev().copied().collect(),
        });
    }
    if let PatternNode::Internal { kind_id, children } = node {
        parents.push((!kind_utils::is_error_kind(*kind_id)).then_some(*kind_id));
        for child in children {
            collect_pattern(child, parents, required);
        }
        parents.pop();
    }
    // Anonymous goals may be skipped after an ellipsis. Metavariables give no
    // necessary text/kind, but do not alter the parent depth of concrete goals.
}

#[derive(Default)]
struct Needs<'plan> {
    text: bool,
    prefix: bool,
    terminals: HashSet<&'plan str>,
}

struct Entry<'tree, D: Doc> {
    node: D::Node<'tree>,
    parent: Option<NonZeroUsize>,
    previous: Option<NonZeroUsize>,
    kind: u16,
    subtree_end: usize,
}

// Bound vocabulary hashing/comparison to small spans. Large/composite values
// are NOT absent: the residue remains complete, with one S275 prefix per node.
const MAX_VALUE_BYTES: usize = 256;
const PREFIX_CHARS: usize = 8;
const DIRECT_TEXT_NODES: usize = 8;

#[derive(Default)]
struct Values<'tree> {
    complete: bool,
    terms: BTreeMap<&'tree str, Vec<usize>>,
    residue: Vec<usize>,
    prefixes: BTreeMap<&'tree str, Vec<usize>>,
}

/// The dictionary of `grouped` (each text's ordinals, ascending): sorting only the distinct
/// texts, it is built in one pass.
fn postings(grouped: HashMap<&str, Vec<usize>>) -> BTreeMap<&str, Vec<usize>> {
    let mut grouped = grouped.into_iter().collect::<Vec<_>>();
    grouped.sort_unstable_by_key(|&(text, _)| text);
    grouped.into_iter().collect()
}

pub(in crate::code_rule_dsl) enum Selection<'index> {
    All,
    Roots(Cow<'index, [usize]>),
}

impl Selection<'_> {
    fn empty() -> Self {
        Self::Roots(Cow::Borrowed(&[]))
    }
    fn is_empty(&self) -> bool {
        matches!(self, Self::Roots(roots) if roots.is_empty())
    }
    fn len(&self, all: usize) -> usize {
        match self {
            Self::All => all,
            Self::Roots(roots) => roots.len(),
        }
    }
    fn contains(&self, ordinal: usize) -> bool {
        match self {
            Self::All => true,
            Self::Roots(roots) => roots.binary_search(&ordinal).is_ok(),
        }
    }
    fn ordinals(&self, all: usize) -> impl Iterator<Item = usize> + '_ {
        let range = if matches!(self, Self::All) {
            0..all
        } else {
            0..0
        };
        let roots = match self {
            Self::Roots(roots) => roots.as_ref(),
            Self::All => &[],
        };
        range.chain(roots.iter().copied())
    }
}

type TextCache = HashMap<String, HashMap<u16, Vec<usize>>>;

/// One selection's results for its shared utility plans, keyed by plan address.
#[derive(Default)]
struct UtilitySelections<'index> {
    estimates: HashMap<usize, usize>,
    /// The roots each plan was evaluated over so far, and its selection among them.
    selections: HashMap<usize, (Selection<'index>, Selection<'index>)>,
    /// The nodes of each set of several root kinds, merged once however many plans root there.
    roots: HashMap<BTreeSet<u16>, Selection<'index>>,
}

pub(in crate::code_rule_dsl) struct FileAnchorIndex<'tree, D: Doc> {
    root: &'tree AstGrep<D>,
    source: &'tree str,
    entries: Vec<Entry<'tree, D>>,
    kinds: Vec<Vec<usize>>,
    /// First children in requested fields, in preorder. These are sparse positional postings,
    /// not a dense (parent, field) table. Parent links supply the reverse direction.
    fields: HashMap<u16, Vec<usize>>,
    errors: Vec<usize>,
    values: HashMap<u16, Values<'tree>>,
    text_cache: RefCell<TextCache>,
    visited_nodes: usize,
    complete: bool,
    /// Whether sibling relations use ast-grep's own walk, not preorder ordinals: some node is
    /// zero-width, or some siblings overlap (a Blade tape's embedded program sits among the
    /// template's nodes, `blade::view`).
    walked_siblings: bool,
    /// Ordinals by node id, filled on first use by that walk.
    by_node_id: RefCell<HashMap<usize, usize>>,
}

impl<'tree, D> FileAnchorIndex<'tree, D>
where
    D: Doc<Source = String, Lang = GrammarDialect>,
    D::Node<'tree>: AnchorNode<'tree>,
{
    pub(in crate::code_rule_dsl) fn build(
        root: &'tree AstGrep<D>,
        plans: &[&AnchorPlan],
        max_nodes: usize,
        mut budget_exhausted: impl FnMut() -> bool,
    ) -> Self {
        #[cfg(test)]
        record_walk(Walk::AnchorBuild);
        let doc = root.root().get_doc();
        let mut needs: Vec<_> = (0..doc.get_lang().kind_count())
            .map(|_| Needs::default())
            .collect();
        let mut utilities = HashSet::default();
        let mut fields = BTreeSet::new();
        for plan in plans {
            plan.collect_needs(None, &mut utilities, &mut needs, &mut fields);
        }
        let mut index = Self {
            root,
            source: doc.get_source(),
            entries: Vec::new(),
            kinds: vec![Vec::new(); doc.get_lang().kind_count()],
            fields: fields
                .into_iter()
                .map(|field| (field, Vec::new()))
                .collect(),
            errors: Vec::new(),
            values: HashMap::default(),
            text_cache: RefCell::new(HashMap::default()),
            visited_nodes: 0,
            complete: false,
            walked_siblings: false,
            by_node_id: RefCell::new(HashMap::default()),
        };
        let mut ancestors: Vec<NonZeroUsize> = Vec::new();
        // (last immediate child, children after the cursor's current child).
        // A failed native goto_next_sibling searches upward through the cursor
        // stack. Skip it when the owned frame already knows there is no sibling.
        let mut last_children: Vec<(Option<NonZeroUsize>, usize, u64)> = Vec::new();
        let mut closed_nodes = 0;
        let mut cursor = doc.root_node().cursor();
        'preorder: loop {
            let node = cursor.current();
            index.visited_nodes += 1;
            if index.visited_nodes > max_nodes
                || (index.entries.len() & 255 == 0 && budget_exhausted())
            {
                return index;
            }
            let kind = node.kind_id();
            let ordinal = index.entries.len();
            let previous = last_children.last().and_then(|(last, _, _)| *last);
            index.walked_siblings |= node.range().is_empty()
                || previous.is_some_and(|previous| {
                    index.entries[previous.get() - 1].node.range().end > node.range().start
                });
            index.entries.push(Entry {
                node,
                kind,
                parent: ancestors.last().copied(),
                previous,
                subtree_end: ordinal + 1,
            });
            if kind_utils::is_error_kind(kind) {
                index.errors.push(ordinal);
            } else {
                if usize::from(kind) >= index.kinds.len() {
                    index.kinds.resize_with(usize::from(kind) + 1, Vec::new);
                }
                index.kinds[usize::from(kind)].push(ordinal);
            }
            let current = NonZeroUsize::new(ordinal + 1).expect("inserted node");
            if let Some((last_child, _, seen_fields)) = last_children.last_mut() {
                *last_child = Some(current);
                let field = cursor.field_id();
                if let Some(posting) = index.fields.get_mut(&field) {
                    // Rec6 grammars have at most 63 fields. Keep the cursor oracle sound for
                    // wider native grammars too, without a field-width assumption there.
                    let first = if let Some(bit) = 1_u64.checked_shl(u32::from(field)) {
                        let first = *seen_fields & bit == 0;
                        *seen_fields |= bit;
                        first
                    } else {
                        let parent = index.entries[ordinal].parent.expect("child has parent");
                        index.entries[parent.get() - 1]
                            .node
                            .child_by_field_id(field)
                            .is_some_and(|first| first.node_id() == node.node_id())
                    };
                    if field != 0 && first {
                        posting.push(ordinal);
                    }
                }
            }
            ancestors.push(current);
            last_children.push((None, 0, 0));
            if cursor.first_child() {
                last_children.last_mut().expect("current frame").1 = node.child_count() - 1;
                continue;
            }
            loop {
                let closed = ancestors.pop().expect("current node is open");
                last_children.pop();
                index.entries[closed.get() - 1].subtree_end = index.entries.len();
                closed_nodes += 1;
                if closed_nodes & 255 == 0 && budget_exhausted() {
                    return index;
                }
                if ancestors.is_empty() {
                    break 'preorder;
                }
                let remaining = &mut last_children.last_mut().expect("parent frame").1;
                if *remaining > 0 {
                    *remaining -= 1;
                    assert!(cursor.next_sibling(), "remaining child must have a sibling");
                    break;
                }
                assert!(cursor.parent(), "cursor must return to an open ancestor");
            }
        }
        index.complete = index.build_values(&needs, &mut budget_exhausted) && !budget_exhausted();
        index
    }

    fn build_values(
        &mut self,
        needs: &[Needs<'_>],
        budget_exhausted: &mut impl FnMut() -> bool,
    ) -> bool {
        let mut steps = 0;
        for (kind, need) in needs.iter().enumerate() {
            let nodes = &self.kinds[kind];
            // For tiny domains, direct text checks cost less than a dictionary.
            let complete = need.text && nodes.len() > DIRECT_TEXT_NODES;
            if !complete && need.terminals.is_empty() {
                continue;
            }
            let mut values = Values {
                complete,
                ..Values::default()
            };
            // Postings are grouped by text and each dictionary is built once from them:
            // inserting node by node searched a growing B-tree for every identifier of a large
            // file. Ordinals arrive ascending, so each text's list is ordered.
            let mut terms = HashMap::<&str, Vec<usize>>::new();
            let mut prefixes = HashMap::<&str, Vec<usize>>::new();
            for &ordinal in nodes {
                if steps & 255 == 0 && budget_exhausted() {
                    return false;
                }
                steps += 1;
                let node = self.entries[ordinal].node;
                let text = &self.source[node.range()];
                let required = node.is_named() && need.terminals.contains(text);
                if required || (complete && text.len() <= MAX_VALUE_BYTES) {
                    terms.entry(text).or_default().push(ordinal);
                } else if complete {
                    values.residue.push(ordinal);
                    if need.prefix {
                        let end = text
                            .char_indices()
                            .nth(PREFIX_CHARS)
                            .map_or(text.len(), |(offset, _)| offset);
                        prefixes.entry(&text[..end]).or_default().push(ordinal);
                    }
                }
            }
            values.terms = postings(terms);
            values.prefixes = postings(prefixes);
            self.values.insert(kind as u16, values);
        }
        true
    }

    pub(in crate::code_rule_dsl) fn visited_nodes(&self) -> usize {
        self.visited_nodes
    }
    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(in crate::code_rule_dsl) fn complete(&self) -> bool {
        self.complete
    }
    pub(in crate::code_rule_dsl) fn node(&self, ordinal: usize) -> Node<'tree, D> {
        self.root.adopt(self.entries[ordinal].node)
    }
    pub(in crate::code_rule_dsl) fn kind(&self, ordinal: usize) -> u16 {
        self.entries[ordinal].kind
    }

    #[cfg(feature = "code-rule-profile")]
    pub(in crate::code_rule_dsl) fn estimated_bytes(&self) -> usize {
        let mut bytes = self.entries.capacity() * std::mem::size_of::<Entry<'tree, D>>()
            + self.kinds.capacity() * std::mem::size_of::<Vec<usize>>()
            + self.errors.capacity() * std::mem::size_of::<usize>()
            + self.values.capacity() * std::mem::size_of::<(u16, Values<'tree>)>();
        for posting in &self.kinds {
            bytes += posting.capacity() * std::mem::size_of::<usize>();
        }
        bytes += self.fields.capacity() * std::mem::size_of::<(u16, Vec<usize>)>();
        bytes += self
            .fields
            .values()
            .map(|posting| posting.capacity() * std::mem::size_of::<usize>())
            .sum::<usize>();
        for values in self.values.values() {
            bytes += values.residue.capacity() * std::mem::size_of::<usize>();
            for (key, posting) in values.terms.iter().chain(&values.prefixes) {
                let _ = key;
                bytes += std::mem::size_of::<(&str, Vec<usize>)>()
                    + posting.capacity() * std::mem::size_of::<usize>();
            }
        }
        let cache = self.text_cache.borrow();
        bytes += cache.capacity() * std::mem::size_of::<(String, HashMap<u16, Vec<usize>>)>();
        for (pattern, kinds) in cache.iter() {
            bytes +=
                pattern.capacity() + kinds.capacity() * std::mem::size_of::<(u16, Vec<usize>)>();
            for posting in kinds.values() {
                bytes += posting.capacity() * std::mem::size_of::<usize>();
            }
        }
        // Excludes allocator/B-tree/hash metadata, source/tree/plan ownership,
        // and transient selection scratch; no index survives this file pass.
        bytes
    }

    pub(in crate::code_rule_dsl) fn select(&self, plan: &AnchorPlan) -> Selection<'_> {
        if !self.complete() || budget::matcher_budget_exhausted() {
            return Selection::All;
        }
        match self.select_complete(plan, Selection::All, &mut UtilitySelections::default()) {
            Ok(selection) if !budget::matcher_budget_exhausted() => selection,
            _ => self.root_selection(plan).unwrap_or(Selection::All),
        }
    }

    fn kind_posting(&self, kind: u16) -> &[usize] {
        if kind_utils::is_error_kind(kind) {
            &self.errors
        } else {
            self.kinds.get(usize::from(kind)).map_or(&[], Vec::as_slice)
        }
    }

    fn posting(&self, required: &Required) -> &[usize] {
        match required {
            Required::Kind(kind) => self.kind_posting(*kind),
            Required::Terminal(kind, text) => self
                .values
                .get(kind)
                .and_then(|values| values.terms.get(text.as_str()))
                .map_or(&[], Vec::as_slice),
        }
    }

    fn root_selection(&self, plan: &AnchorPlan) -> Result<Selection<'_>, ()> {
        let Some(kinds) = plan.root_kinds() else {
            return Ok(Selection::All);
        };
        if kinds.len() == 1 {
            return Ok(Selection::Roots(Cow::Borrowed(
                self.kind_posting(*kinds.first().unwrap()),
            )));
        }
        // Each node is in one kind's posting, and every error kind reads the one of errors: the
        // union is the postings side by side, sorted.
        let mut result = Vec::new();
        let mut errors = false;
        for &kind in kinds {
            if kind_utils::is_error_kind(kind) && std::mem::replace(&mut errors, true) {
                continue;
            }
            for &ordinal in self.kind_posting(kind) {
                check_budget()?;
                result.push(ordinal);
            }
        }
        result.sort_unstable();
        Ok(Selection::Roots(Cow::Owned(result)))
    }

    /// `scope`'s roots of `plan`'s root kinds: the scope's own roots tested by kind when there are
    /// fewer of them than in the kinds' postings, the postings intersected with it otherwise.
    fn within_root_kinds<'index>(
        &'index self,
        scope: Selection<'index>,
        plan: &AnchorPlan,
        memo: &mut UtilitySelections<'index>,
    ) -> Result<Selection<'index>, ()> {
        let Some(kinds) = plan.root_kinds() else {
            return Ok(scope);
        };
        let postings = kinds
            .iter()
            .map(|&kind| self.kind_posting(kind).len())
            .sum::<usize>();
        if scope.len(self.len()) >= postings {
            return combine(scope, self.root_selection_in(plan, memo)?, true);
        }
        let errors = kinds.iter().any(|&kind| kind_utils::is_error_kind(kind));
        self.filter(scope, |ordinal| {
            let kind = self.kind(ordinal);
            if kind_utils::is_error_kind(kind) {
                errors
            } else {
                kinds.contains(&kind)
            }
        })
    }

    /// `root_selection`, merging the postings of several root kinds once per selection.
    fn root_selection_in<'index>(
        &'index self,
        plan: &AnchorPlan,
        memo: &mut UtilitySelections<'index>,
    ) -> Result<Selection<'index>, ()> {
        let Some(kinds) = plan
            .root_kinds()
            .filter(|kinds| kinds.len() > 1 && crate::code_rule_dsl::budget::shortcuts_on())
        else {
            return self.root_selection(plan);
        };
        if let Some(roots) = memo.roots.get(kinds) {
            return Ok(copy_selection(roots));
        }
        let roots = self.root_selection(plan)?;
        memo.roots.insert(kinds.clone(), copy_selection(&roots));
        Ok(roots)
    }

    fn estimate(&self, plan: &AnchorPlan, memo: &mut UtilitySelections<'_>) -> usize {
        let roots = plan.root_kinds.as_ref().map_or(self.len(), |kinds| {
            kinds
                .iter()
                .map(|&kind| self.kind_posting(kind).len())
                .sum()
        });
        match &plan.condition {
            Condition::Pattern(required) => required
                .iter()
                .map(|req| {
                    self.posting(&req.value)
                        .len()
                        .saturating_mul(req.parents.len() + 1)
                })
                .min()
                .unwrap_or(roots)
                .min(roots),
            Condition::All(plans) => plans
                .iter()
                .map(|plan| self.estimate(plan, memo))
                .min()
                .unwrap_or(roots)
                .min(roots),
            Condition::Any(plans) => plans
                .iter()
                .map(|plan| self.estimate(plan, memo))
                .fold(0usize, usize::saturating_add)
                .min(roots),
            Condition::Relation { inner, .. } => self.estimate(inner, memo).min(roots),
            Condition::Utility(inner) => {
                let key = Arc::as_ptr(inner) as usize;
                match memo.estimates.get(&key) {
                    Some(&estimate) => estimate,
                    None => {
                        let estimate = self.estimate(inner, memo);
                        memo.estimates.insert(key, estimate);
                        estimate
                    }
                }
                .min(roots)
            }
            _ => roots,
        }
    }

    fn select_complete<'index>(
        &'index self,
        plan: &AnchorPlan,
        scope: Selection<'index>,
        memo: &mut UtilitySelections<'index>,
    ) -> Result<Selection<'index>, ()> {
        check_budget()?;
        let scope = self.within_root_kinds(scope, plan, memo)?;
        if scope.is_empty() {
            return Ok(scope);
        }
        match &plan.condition {
            Condition::Always => Ok(scope),
            Condition::Pattern(required) => self.pattern_selection(scope, required),
            Condition::Text { regex, probe } => self.text_selection(scope, regex, probe),
            Condition::All(plans) => {
                let mut plans: Vec<_> = plans.iter().collect();
                plans.sort_by_cached_key(|plan| self.estimate(plan, memo));
                let mut result = scope;
                for child in plans {
                    if result.is_empty() {
                        break;
                    }
                    result = self.select_complete(child, result, memo)?;
                }
                Ok(result)
            }
            Condition::Any(plans) => {
                let mut result = Selection::empty();
                for child in plans {
                    // Keep alternatives independent; an unconstrained arm must
                    // preserve the entire input domain.
                    result = combine(
                        result,
                        self.select_complete(child, copy_selection(&scope), memo)?,
                        false,
                    )?;
                }
                Ok(result)
            }
            Condition::Not(inner) => {
                let excluded = self.select_complete(inner, copy_selection(&scope), memo)?;
                self.filter(scope, |ordinal| !excluded.contains(ordinal))
            }
            Condition::Relation {
                inner,
                direction,
                recursive,
                field,
            } => match field {
                Some(field) => {
                    self.field_selection(scope, inner, *direction, *recursive, *field, memo)
                }
                None => self.relation_selection(scope, inner, *direction, *recursive, memo),
            },
            Condition::Utility(inner) => self.utility_selection(scope, inner, memo),
        }
    }

    /// Selects a shared utility plan within `scope`, evaluating its conditions only over roots no
    /// earlier reference reached: however many diamond paths reach the plan, one selection tests
    /// each root at most once. A selection over a set of roots is a subset of it that holds every
    /// root of the set the plan's rule matches (and only those when exact). Selections over
    /// disjoint sets so unite into a selection over their union, and its intersection with a
    /// scope within that union is a selection of the same precision for the scope.
    fn utility_selection<'index>(
        &'index self,
        scope: Selection<'index>,
        plan: &Arc<AnchorPlan>,
        memo: &mut UtilitySelections<'index>,
    ) -> Result<Selection<'index>, ()> {
        let key = Arc::as_ptr(plan) as usize;
        let (evaluated, selected) = memo
            .selections
            .remove(&key)
            .unwrap_or_else(|| (Selection::empty(), Selection::empty()));
        if is_subset(&scope, &evaluated)? {
            let result = combine(scope, copy_selection(&selected), true);
            memo.selections.insert(key, (evaluated, selected));
            return result;
        }
        let missing = if evaluated.is_empty() {
            copy_selection(&scope)
        } else {
            self.filter(copy_selection(&scope), |ordinal| {
                !evaluated.contains(ordinal)
            })?
        };
        // No earlier evaluation reached the scope: its selection is the new one alone.
        let disjoint = missing.len(self.len()) == scope.len(self.len());
        #[cfg(test)]
        record_walk(Walk::AnchorSelection);
        let added = self.select_complete(plan, copy_selection(&missing), memo)?;
        let result = if disjoint {
            copy_selection(&added)
        } else {
            combine(
                scope,
                combine(copy_selection(&selected), copy_selection(&added), false)?,
                true,
            )?
        };
        memo.selections.insert(
            key,
            (
                combine(evaluated, missing, false)?,
                combine(selected, added, false)?,
            ),
        );
        Ok(result)
    }

    fn filter<'index>(
        &'index self,
        scope: Selection<'index>,
        mut keep: impl FnMut(usize) -> bool,
    ) -> Result<Selection<'index>, ()> {
        let mut selected = Vec::new();
        for ordinal in scope.ordinals(self.len()) {
            check_budget()?;
            if keep(ordinal) {
                selected.push(ordinal);
            }
        }
        if selected.len() == scope.len(self.len()) {
            Ok(scope)
        } else {
            Ok(Selection::Roots(Cow::Owned(selected)))
        }
    }

    fn text_selection<'index>(
        &'index self,
        scope: Selection<'index>,
        regex: &Regex,
        probe: &TextProbe,
    ) -> Result<Selection<'index>, ()> {
        let mut counts = HashMap::<u16, usize>::default();
        for ordinal in scope.ordinals(self.len()) {
            check_budget()?;
            #[cfg(test)]
            record_walk(Walk::AnchorTextNode);
            *counts.entry(self.kind(ordinal)).or_default() += 1;
        }
        let mut selected = Vec::new();
        let mut direct = HashSet::default();
        for (kind, count) in counts {
            let Some(values) = self.values.get(&kind).filter(|values| values.complete) else {
                direct.insert(kind);
                continue;
            };
            // Context/previous conjuncts may already be cheaper than naming a
            // whole kind's vocabulary. Never build a broad secondary filter for
            // a handful of surviving roots (S216).
            if count <= DIRECT_TEXT_NODES || count < values.terms.len() + values.residue.len() {
                direct.insert(kind);
                continue;
            }
            let cached = self
                .text_cache
                .borrow()
                .get(regex.as_str())
                .and_then(|kinds| kinds.get(&kind))
                .cloned();
            let matches = match cached {
                Some(matches) => matches,
                None => {
                    let matches = self.match_values(values, regex, probe)?;
                    self.text_cache
                        .borrow_mut()
                        .entry(regex.as_str().to_owned())
                        .or_default()
                        .insert(kind, matches.clone());
                    matches
                }
            };
            selected.extend(matches);
        }
        if !direct.is_empty() {
            for ordinal in scope.ordinals(self.len()) {
                check_budget()?;
                if direct.contains(&self.kind(ordinal)) {
                    let text = &self.source[self.entries[ordinal].node.range()];
                    if probe.allows(text) && regex.is_match(text) {
                        selected.push(ordinal);
                    }
                }
            }
        }
        selected.sort_unstable();
        selected.dedup();
        combine(scope, Selection::Roots(Cow::Owned(selected)), true)
    }

    fn match_values(
        &self,
        values: &Values<'tree>,
        regex: &Regex,
        probe: &TextProbe,
    ) -> Result<Vec<usize>, ()> {
        let mut result = Vec::new();
        let mut visit_term = |text: &str, posting: &[usize]| -> Result<(), ()> {
            check_budget()?;
            if probe.allows(text) && regex.is_match(text) {
                result.extend_from_slice(posting);
            }
            Ok(())
        };
        if let Some(prefixes) = probe.prefixes() {
            for prefix in prefixes {
                for (&text, posting) in values.terms.range(prefix.as_str()..) {
                    if !text.starts_with(prefix) {
                        break;
                    }
                    visit_term(text, posting)?;
                }
            }
        } else {
            for (&text, posting) in &values.terms {
                visit_term(text, posting)?;
            }
        }
        let mut visit_residue = |ordinal: usize| -> Result<(), ()> {
            check_budget()?;
            let text = &self.source[self.entries[ordinal].node.range()];
            if probe.allows(text) && regex.is_match(text) {
                result.push(ordinal);
            }
            Ok(())
        };
        if let Some(prefixes) = probe.prefixes() {
            for prefix in prefixes {
                for (&key, posting) in values.prefixes.range(prefix.as_str()..) {
                    if !key.starts_with(prefix) {
                        break;
                    }
                    for &ordinal in posting {
                        visit_residue(ordinal)?;
                    }
                }
            }
        } else {
            for &ordinal in &values.residue {
                visit_residue(ordinal)?;
            }
        }
        result.sort_unstable();
        result.dedup();
        Ok(result)
    }

    fn pattern_selection<'index>(
        &'index self,
        scope: Selection<'index>,
        required: &[Requirement],
    ) -> Result<Selection<'index>, ()> {
        let mut postings = Vec::with_capacity(required.len());
        for requirement in required {
            check_budget()?;
            let posting = self.posting(&requirement.value);
            if posting.is_empty() {
                return Ok(Selection::empty());
            }
            postings.push(posting);
        }
        let Some((seed, requirement)) = required
            .iter()
            .enumerate()
            .min_by_key(|(idx, req)| postings[*idx].len().saturating_mul(req.parents.len() + 1))
        else {
            return Ok(scope);
        };
        let mut scope = scope;
        let climb_cost = postings[seed]
            .len()
            .saturating_mul(requirement.parents.len() + 1);
        let probe_cost = scope.len(self.len()).saturating_mul(required.len());
        if climb_cost <= probe_cost {
            let mut roots = Vec::new();
            for &anchor in postings[seed] {
                let mut current = Some(anchor);
                for kind in &requirement.parents {
                    check_budget()?;
                    current = current.and_then(|ordinal| {
                        self.entries[ordinal].parent.map(|parent| parent.get() - 1)
                    });
                    if current
                        .is_none_or(|ordinal| kind.is_some_and(|kind| self.kind(ordinal) != kind))
                    {
                        current = None;
                        break;
                    }
                }
                if let Some(ordinal) = current {
                    roots.push(ordinal);
                }
            }
            roots.sort_unstable();
            roots.dedup();
            scope = combine(scope, Selection::Roots(Cow::Owned(roots)), true)?;
        }
        let mut selected = Vec::new();
        for ordinal in scope.ordinals(self.len()) {
            check_budget()?;
            if contains_requirements(ordinal, self.entries[ordinal].subtree_end, &postings) {
                selected.push(ordinal);
            }
        }
        if selected.len() == scope.len(self.len()) {
            Ok(scope)
        } else {
            Ok(Selection::Roots(Cow::Owned(selected)))
        }
    }

    fn relation_selection<'index>(
        &'index self,
        scope: Selection<'index>,
        inner: &AnchorPlan,
        direction: Direction,
        recursive: bool,
        memo: &mut UtilitySelections<'index>,
    ) -> Result<Selection<'index>, ()> {
        // A relation supplies a real structural context before text selection: don't read all
        // identifiers to find a direct child of one of the already-selected parents, or a sibling
        // of one of the already-selected roots. No grammar-specific split is needed.
        let inner_scope = match direction {
            Direction::Has if !recursive && !matches!(scope, Selection::All) => {
                let mut children = Vec::new();
                for parent in scope.ordinals(self.len()) {
                    self.push_children(parent, &mut children)?;
                }
                children.sort_unstable();
                children.dedup();
                Selection::Roots(Cow::Owned(children))
            }
            Direction::Inside if !recursive && !matches!(scope, Selection::All) => {
                let mut parents = Vec::new();
                for ordinal in scope.ordinals(self.len()) {
                    check_budget()?;
                    if let Some(parent) = self.entries[ordinal].parent {
                        parents.push(parent.get() - 1);
                    }
                }
                parents.sort_unstable();
                parents.dedup();
                Selection::Roots(Cow::Owned(parents))
            }
            Direction::Precedes | Direction::Follows if !matches!(scope, Selection::All) => {
                self.sibling_scope(&scope)?
            }
            _ => Selection::All,
        };
        let matches = self.select_complete(inner, inner_scope, memo)?;
        if matches.is_empty() {
            return Ok(Selection::empty());
        }
        match direction {
            Direction::Has => {
                let mut roots = Vec::new();
                // Ancestors already walked, by ordinal.
                let mut seen = vec![0_u64; self.len().div_ceil(64)];
                for child in matches.ordinals(self.len()) {
                    let mut current = self.entries[child].parent;
                    while let Some(parent) = current {
                        check_budget()?;
                        let parent = parent.get() - 1;
                        let (word, bit) = (parent / 64, 1_u64 << (parent % 64));
                        if seen[word] & bit != 0 {
                            break;
                        }
                        seen[word] |= bit;
                        if scope.contains(parent) {
                            roots.push(parent);
                        }
                        if !recursive {
                            break;
                        }
                        current = self.entries[parent].parent;
                    }
                }
                roots.sort_unstable();
                Ok(Selection::Roots(Cow::Owned(roots)))
            }
            // Every node is the child of a matched parent: list the matched parents' children.
            Direction::Inside if !recursive && matches!(scope, Selection::All) => {
                let mut children = Vec::new();
                for parent in matches.ordinals(self.len()) {
                    self.push_children(parent, &mut children)?;
                }
                children.sort_unstable();
                Ok(Selection::Roots(Cow::Owned(children)))
            }
            Direction::Inside if !recursive => self.filter(scope, |ordinal| {
                self.entries[ordinal]
                    .parent
                    .is_some_and(|parent| matches.contains(parent.get() - 1))
            }),
            Direction::Inside => {
                // Merge nested ancestor intervals. A root cannot be its own
                // ancestor, hence start + 1 even for zero-width syntax nodes.
                let mut ranges: Vec<(usize, usize)> = Vec::new();
                for ordinal in matches.ordinals(self.len()) {
                    check_budget()?;
                    let end = self.entries[ordinal].subtree_end;
                    if let Some(previous) = ranges.last_mut()
                        && ordinal < previous.1
                    {
                        previous.1 = previous.1.max(end);
                    } else {
                        ranges.push((ordinal + 1, end));
                    }
                }
                self.filter(scope, |ordinal| {
                    let offset = ranges.partition_point(|(start, _)| *start <= ordinal);
                    offset != 0 && ordinal < ranges[offset - 1].1
                })
            }
            Direction::Precedes | Direction::Follows => self.sibling_selection(
                scope,
                &matches,
                matches!(direction, Direction::Precedes),
                recursive,
            ),
        }
    }

    /// Appends `parent`'s children, in order, to `children`.
    fn push_children(&self, parent: usize, children: &mut Vec<usize>) -> Result<(), ()> {
        let mut child = parent + 1;
        while child < self.entries[parent].subtree_end {
            check_budget()?;
            children.push(child);
            child = self.entries[child].subtree_end;
        }
        Ok(())
    }

    /// Every node a sibling relation reads for the roots of `scope` (`sibling_selection`): the
    /// children of each root's parent, and of the root itself where it has none (a walk at the
    /// root reads the root's own children). The relation's inner rule is selected among these
    /// alone, not among every node of the file: what it selects elsewhere no root reads.
    fn sibling_scope(&self, scope: &Selection<'_>) -> Result<Selection<'static>, ()> {
        let mut containers = Vec::new();
        for ordinal in scope.ordinals(self.len()) {
            check_budget()?;
            containers.push(
                self.entries[ordinal]
                    .parent
                    .map_or(ordinal, |parent| parent.get() - 1),
            );
        }
        containers.sort_unstable();
        containers.dedup();
        let mut siblings = Vec::new();
        for container in containers {
            self.push_children(container, &mut siblings)?;
        }
        // A container nested in another lists its children inside the outer one's.
        siblings.sort_unstable();
        Ok(Selection::Roots(Cow::Owned(siblings)))
    }

    // `precedes`/`follows` relate a node to its own siblings (named or anonymous): the adjacent
    // one for `neighbor`, every one in that direction for `end`. With `end`, a node qualifies iff
    // the last (first) matched child of its parent comes after (before) it.
    fn sibling_selection<'index>(
        &'index self,
        scope: Selection<'index>,
        matches: &Selection<'_>,
        following: bool,
        every: bool,
    ) -> Result<Selection<'index>, ()> {
        let mut extreme = HashMap::<usize, usize>::default();
        if every {
            for matched in matches.ordinals(self.len()) {
                check_budget()?;
                if let Some(parent) = self.entries[matched].parent {
                    extreme
                        .entry(parent.get() - 1)
                        .and_modify(|found| {
                            *found = if following {
                                (*found).max(matched)
                            } else {
                                (*found).min(matched)
                            }
                        })
                        .or_insert(matched);
                }
            }
        }
        let mut runs = HashMap::default();
        self.filter(scope, |ordinal| {
            let entry = &self.entries[ordinal];
            let Some(parent) = entry.parent.map(|parent| parent.get() - 1) else {
                return self
                    .walked_siblings_match(ordinal, ordinal, matches, following, every, &mut runs);
            };
            if self.walked_siblings {
                return self
                    .walked_siblings_match(ordinal, parent, matches, following, every, &mut runs);
            }
            if every {
                extreme.get(&parent).is_some_and(|&matched| {
                    if following {
                        matched > ordinal
                    } else {
                        matched < ordinal
                    }
                })
            } else if following {
                entry.subtree_end < self.entries[parent].subtree_end
                    && matches.contains(entry.subtree_end)
            } else {
                entry
                    .previous
                    .is_some_and(|previous| matches.contains(previous.get() - 1))
            }
        })
    }

    // ast-grep's sibling walks position a cursor by start byte (and tree-sitter's parent lookup
    // descends by byte), so around zero-width nodes and among overlapping siblings they can skip
    // or revisit siblings, and at the root they walk the root's own children. Rather than model
    // that, ask the same walk and look each walked node up by id among `container`'s children,
    // stopping at the first match. With `end` the walk is a run of the walked parent's children
    // from the first that ends after the node's start byte (`goto_first_child_for_byte`), so each
    // node reads its answer from that parent's matched-children counts (`runs`), built once:
    // linear in the siblings, not quadratic.
    fn walked_siblings_match(
        &self,
        ordinal: usize,
        container: usize,
        matches: &Selection<'_>,
        following: bool,
        every: bool,
        runs: &mut HashMap<(usize, usize), Option<SiblingRun>>,
    ) -> bool {
        let ordinals = self.ordinals_by_node_id();
        let matched_child = |walked: usize| {
            ordinals.get(&walked).is_some_and(|&child| {
                self.entries[child]
                    .parent
                    .is_some_and(|parent| parent.get() - 1 == container)
                    && matches.contains(child)
            })
        };
        let node = self.node(ordinal);
        if every {
            let walked_parent = node.parent().unwrap_or_else(|| node.clone());
            let run = runs
                .entry((walked_parent.node_id(), container))
                .or_insert_with(|| {
                    let mut run = SiblingRun {
                        ends: Vec::new(),
                        matched_before: vec![0],
                    };
                    for child in walked_parent.children() {
                        #[cfg(test)]
                        record_walk(Walk::AnchorSibling);
                        // Budget exhaustion makes the whole selection fall back to root kinds.
                        check_budget().ok()?;
                        let before = *run.matched_before.last().expect("a count");
                        let end = run.ends.last().copied().unwrap_or_default();
                        run.ends.push(end.max(child.range().end));
                        run.matched_before
                            .push(before + usize::from(matched_child(child.node_id())));
                    }
                    Some(run)
                });
            let Some(run) = run else {
                return true;
            };
            let len = run.ends.len();
            let start = node.range().start;
            let anchor = run.ends.partition_point(|&end| end <= start);
            if anchor == len {
                return false;
            }
            return if following {
                run.matched_before[len] > run.matched_before[anchor + 1]
            } else {
                run.matched_before[anchor] > 0
            };
        }
        let walked = if following { node.next() } else { node.prev() };
        walked.is_some_and(|sibling| {
            #[cfg(test)]
            record_walk(Walk::AnchorSibling);
            // Budget exhaustion makes the whole selection fall back to root kinds.
            check_budget().is_err() || matched_child(sibling.node_id())
        })
    }

    /// Each entry's ordinal by its node id, built once for the walked sibling relations.
    fn ordinals_by_node_id(&self) -> std::cell::Ref<'_, HashMap<usize, usize>> {
        if self.by_node_id.borrow().is_empty() {
            let mut ordinals = self.by_node_id.borrow_mut();
            ordinals.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .map(|(ordinal, entry)| (entry.node.node_id(), ordinal)),
            );
        }
        self.by_node_id.borrow()
    }
}

/// A walked parent's children, in walk order, as an `end` sibling walk reads them: the largest
/// end byte up to each child (overlapping siblings' ends can decrease; the first child that ends
/// after a byte is the first whose running end does), and how many of the children before it are
/// matched (one count more: the total).
struct SiblingRun {
    ends: Vec<usize>,
    matched_before: Vec<usize>,
}

/// Whether every root of `inner` is in `outer`.
fn is_subset(inner: &Selection<'_>, outer: &Selection<'_>) -> Result<bool, ()> {
    let (inner, outer) = match (inner, outer) {
        (_, Selection::All) => return Ok(true),
        (Selection::All, Selection::Roots(_)) => return Ok(false),
        (Selection::Roots(inner), Selection::Roots(outer)) => (inner, outer),
    };
    if std::ptr::eq(inner.as_ref(), outer.as_ref()) {
        return Ok(true);
    }
    let mut outer = outer.iter().copied().peekable();
    for &ordinal in inner.iter() {
        check_budget()?;
        while outer.next_if(|&candidate| candidate < ordinal).is_some() {}
        if outer.next_if_eq(&ordinal).is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn copy_selection<'index>(selection: &Selection<'index>) -> Selection<'index> {
    match selection {
        Selection::All => Selection::All,
        Selection::Roots(roots) => Selection::Roots(roots.clone()),
    }
}

/// Whether the subtree `start..end` holds a node of every posting. The caller polls once per
/// subtree; the postings are the plan's few requirements.
fn contains_requirements(start: usize, end: usize, postings: &[&[usize]]) -> bool {
    postings.iter().all(|posting| {
        let offset = posting.partition_point(|ordinal| *ordinal < start);
        posting.get(offset).is_some_and(|ordinal| *ordinal < end)
    })
}

/// One step of a selection loop: polls the budget once per 256 steps of all selection loops
/// together, so a loop run once per root spends what one loop over every root would.
fn check_budget() -> Result<(), ()> {
    if budget::step_budget_exhausted() {
        Err(())
    } else {
        Ok(())
    }
}

/// How many times larger than the other an intersection's side must be for `combine` to search
/// it for each root of the other rather than merge the two.
const SEARCHED_INTERSECTION: usize = 16;

// Merge sorted, unique ordinal sets; identities preserve borrowed postings.
fn combine<'index>(
    left: Selection<'index>,
    right: Selection<'index>,
    intersection: bool,
) -> Result<Selection<'index>, ()> {
    let (left, right) = match (left, right) {
        (Selection::All, other) | (other, Selection::All) => {
            return Ok(if intersection { other } else { Selection::All });
        }
        (Selection::Roots(left), Selection::Roots(right)) => (left, right),
    };
    // Equal sets (one posting reached through several terms, or a merge that added nothing) are
    // their own union and intersection: a comparison, not a merge.
    if std::ptr::eq(left.as_ref(), right.as_ref())
        || (crate::code_rule_dsl::budget::shortcuts_on() && left.as_ref() == right.as_ref())
    {
        return Ok(Selection::Roots(left));
    }
    if left.is_empty() {
        return Ok(Selection::Roots(if intersection { left } else { right }));
    }
    if right.is_empty() {
        return Ok(Selection::Roots(if intersection { right } else { left }));
    }
    // A few roots against a large posting (a scope against every identifier): search each root
    // in the posting rather than merge the whole posting.
    if intersection {
        let (small, large) = if left.len() <= right.len() {
            (&left, &right)
        } else {
            (&right, &left)
        };
        if small.len().saturating_mul(SEARCHED_INTERSECTION) < large.len() {
            let mut result = Vec::with_capacity(small.len());
            let mut rest: &[usize] = large;
            for &ordinal in small.iter() {
                check_budget()?;
                rest = &rest[rest.partition_point(|&candidate| candidate < ordinal)..];
                if rest.first() == Some(&ordinal) {
                    result.push(ordinal);
                }
            }
            return Ok(Selection::Roots(Cow::Owned(result)));
        }
    }
    let mut result = Vec::with_capacity(if intersection {
        left.len().min(right.len())
    } else {
        left.len() + right.len()
    });
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        check_budget()?;
        match left[a].cmp(&right[b]) {
            std::cmp::Ordering::Equal => {
                result.push(left[a]);
                a += 1;
                b += 1;
            }
            std::cmp::Ordering::Less => {
                if !intersection {
                    result.push(left[a]);
                }
                a += 1;
            }
            std::cmp::Ordering::Greater => {
                if !intersection {
                    result.push(right[b]);
                }
                b += 1;
            }
        }
    }
    if !intersection {
        for &ordinal in left[a..].iter().chain(&right[b..]) {
            check_budget()?;
            result.push(ordinal);
        }
    }
    Ok(Selection::Roots(Cow::Owned(result)))
}
