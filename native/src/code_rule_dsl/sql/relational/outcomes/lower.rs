use super::{Axis, CaptureKind, Op, Plan, Slot, Slots};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::{AstMatcher, Semantic, StopBy},
};
use ast_grep_core::{matcher::PatternNode, meta_var::MetaVariable};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[path = "binding_lower.rs"]
mod binding_lower;
#[path = "call_lower.rs"]
mod call_lower;
#[path = "constraint_lower.rs"]
mod constraint_lower;
#[path = "mixed_recursive_lower.rs"]
pub(super) mod mixed_recursive_lower;
#[path = "parse_host_lower.rs"]
mod parse_host_lower;
#[path = "parse_lower.rs"]
mod parse_lower;
#[path = "pure_recursive_lower.rs"]
pub(super) mod pure_recursive_lower;
#[path = "recursive_search_lower.rs"]
pub(super) mod recursive_search_lower;

#[path = "pattern_lower.rs"]
mod pattern_lower;

#[path = "atomic_owner.rs"]
mod atomic_owner;

#[path = "binding_region.rs"]
mod binding_region;
#[path = "binds_lower.rs"]
mod binds_lower;

#[path = "bound_lower.rs"]
mod bound_lower;

#[path = "capture_predicate_lower.rs"]
mod capture_predicate_lower;

#[path = "format_lower.rs"]
mod format_lower;

type CallContext = (usize, usize, bool, bool);
type ScopedUtility = (usize, usize, String, bool, bool);
type SharedPlan<T> = (usize, Rc<T>);

type UtilityOrigin = ((usize, usize), Option<std::collections::BTreeSet<String>>);

#[path = "nth_lower.rs"]
mod nth_lower;

pub(super) struct Lower {
    language: GrammarDialect,
    /// Unique within the statement's BatchScopes lifetime, independent of matcher addresses.
    pub(super) scope_owner: usize,
    pub pure: super::super::lower::Lowerer,
    pub slots: Vec<Slot>,
    active: HashSet<(usize, usize)>,
    utilities: HashMap<(usize, usize, String), (usize, Rc<Plan>)>,
    namespace: String,
    logical_names: Vec<Option<String>>,
    metadata_origins: Vec<Option<usize>>,
    utility_origins: HashMap<usize, UtilityOrigin>,
    call_bodies: HashMap<CallContext, SharedPlan<Plan>>,
    call_sites: HashMap<usize, usize>,
    mixed_components: HashMap<ScopedUtility, SharedPlan<mixed_recursive_lower::Program>>,
    mixed_functions: usize,
    chains: HashMap<(usize, usize), (usize, Rc<super::super::recursive_chain::Chain>)>,
    downward: HashMap<(usize, usize), (usize, Rc<super::super::recursive_downward::Downward>)>,
    recursive_searches: HashMap<ScopedUtility, SharedPlan<recursive_search_lower::RecursiveSearch>>,
    operators: usize,
    utility_calls: usize,
    three_valued: bool,
    parsed_depth: usize,
    hosted_depth: usize,
    parse_hosts: Vec<parse_host_lower::Scope>,
    span_slots: Slots,
    span_links: Vec<(usize, usize)>,
    nths: HashMap<usize, usize>,
}

impl Lower {
    pub fn new(dialect: GrammarDialect, three_valued: bool) -> Self {
        Self {
            language: dialect,
            scope_owner: 1,
            pure: super::super::lower::Lowerer::new(dialect),
            slots: vec![],
            active: HashSet::new(),
            utilities: HashMap::new(),
            namespace: String::new(),
            logical_names: vec![],
            metadata_origins: vec![],
            utility_origins: HashMap::new(),
            call_bodies: HashMap::new(),
            call_sites: HashMap::new(),
            mixed_components: HashMap::new(),
            mixed_functions: 0,
            chains: HashMap::new(),
            downward: HashMap::new(),
            recursive_searches: HashMap::new(),
            operators: 0,
            utility_calls: 0,
            three_valued,
            parsed_depth: 0,
            hosted_depth: 0,
            parse_hosts: vec![],
            span_slots: Slots::new(),
            span_links: Vec::new(),
            nths: HashMap::new(),
        }
    }

    pub fn profile(&self, terminal_base: usize) -> serde_json::Value {
        serde_json::json!({"loweredOperators":self.operators,"utilityCallSites":self.utility_calls,"distinctUtilityBodies":self.utilities.len()+self.mixed_components.len()+self.call_bodies.len()+self.chains.len()+self.downward.len(),"utilityBodyLowerings":self.utilities.len()+self.mixed_components.len()+self.call_bodies.len()+self.chains.len()+self.downward.len(),"recursiveComponents":self.chains.len()+self.downward.len(),"atomicTerminals":self.pure.program.terminals.len()-terminal_base})
    }

    pub(super) fn memo_layout(&self, force: bool) -> Option<Rc<super::utility_memo::Layout>> {
        // A nested body can be lowered before its caller receives parsed exports.
        // Close frame aliases only after all call sites have established their links.
        let mut span_slots = self.span_slots.clone();
        loop {
            let mut changed = false;
            for &(inner, outer) in &self.span_links {
                if span_slots.contains(&inner) || span_slots.contains(&outer) {
                    changed |= span_slots.insert(inner);
                    changed |= span_slots.insert(outer);
                }
            }
            if !changed {
                break;
            }
        }
        if !force && self.call_bodies.is_empty() && self.span_slots.is_empty() {
            return None;
        }
        let mut identities = std::collections::BTreeMap::new();
        for (&id, (key, _)) in &self.utility_origins {
            identities
                .entry(*key)
                .and_modify(|prior: &mut usize| *prior = (*prior).min(id))
                .or_insert(id);
        }
        let contracts = self
            .utility_origins
            .iter()
            .map(|(&id, (key, scope))| {
                (
                    id,
                    super::utility_memo::Contract {
                        identity: identities[key],
                        scope: scope.clone(),
                    },
                )
            })
            .collect();
        Some(Rc::new(super::utility_memo::Layout::new(
            &self.slots,
            &self.logical_names,
            &self.metadata_origins,
            &span_slots,
            contracts,
        )))
    }

    pub(super) fn following_origin(&mut self, slot: usize, origin: usize) {
        // Native FOLLOWING identifies the original child matcher, independently
        // of the private SQL namespace in which that matcher was lowered.
        self.metadata_origins[slot] = Some(origin);
    }

    fn slot(&mut self, name: &str, kind: CaptureKind) -> Result<usize, String> {
        let logical_name = (!name.starts_with('\0')).then(|| name.to_owned());
        let name = format!("{}{name}", self.namespace);
        if let Some(id) = self.slots.iter().position(|slot| slot.name == name) {
            if self.slots[id].kind != kind {
                return Err(
                    "outcome relations: node, list, and count names must be distinct".into(),
                );
            }
            return Ok(id);
        }
        // The bound is structural, not a machine-word mask. Wider contracts need a measured cut.
        if self.slots.len() >= 128 {
            return Err("outcome relations: binding width exceeds 128 columns".into());
        }
        let id = self.slots.len();
        self.logical_names.push(logical_name);
        self.metadata_origins.push(None);
        self.slots.push(Slot { name, kind });
        Ok(id)
    }

    fn walk<'a>(
        &self,
        matcher: &'a AstMatcher,
    ) -> Result<(Axis, &'a StopBy, &'a AstMatcher), String> {
        let (axis, rule, stop) = match matcher {
            AstMatcher::Inside(r, s) => (Axis::Parent, r, s),
            AstMatcher::Has(r, s) => (Axis::Child, r, s),
            AstMatcher::Follows(r, s) => (Axis::Previous, r, s),
            AstMatcher::Precedes(r, s) => (Axis::Next, r, s),
            _ => return Err("outcome relations: count requires a structural walk".into()),
        };
        Ok((axis, stop, rule.rule()))
    }

    fn stop(&mut self, by: &StopBy) -> Result<Option<Box<Plan>>, String> {
        match by {
            StopBy::Rule(rule) => Ok(Some(Box::new(self.plan(rule)?))),
            _ => Ok(None),
        }
    }

    /// Only node-local semantic leaves with no capture inputs or outputs use this cursor.
    /// Composition and capture constraints remain SQL operators, never native callbacks.
    fn atomic(&mut self, matcher: &AstMatcher) -> Op {
        use crate::code_rule_dsl::sql::terminals::{FactTerm, Leaf, Terminal, Verdicts};
        let id = self.pure.program.terminals.len();
        let leaf = Leaf(matcher.clone());
        let verdicts: Box<dyn Verdicts> = if Self::cache_fills(matcher) {
            Box::new(atomic_owner::OwnedLeaf {
                leaf,
                owner: self.scope_owner,
            })
        } else {
            Box::new(leaf)
        };
        self.pure.program.terminals.push(Terminal::Fact(FactTerm {
            verdicts,
            reads: vec![],
        }));
        Op::Value(id)
    }

    fn cache_fills(matcher: &AstMatcher) -> bool {
        match matcher {
            AstMatcher::Semantic(Semantic::Value(test)) => test.reading.follow,
            AstMatcher::Semantic(
                Semantic::Resolves(_)
                | Semantic::Arguments(_)
                | Semantic::Layout(_)
                | Semantic::Binds(_),
            ) => true,
            _ => false,
        }
    }

    pub fn plan(&mut self, matcher: &AstMatcher) -> Result<Plan, String> {
        self.operators += 1;
        if self.operators > 256 {
            return Err("outcome relations: distinct lowered operator count exceeds 256".into());
        }
        let mut mentions = Slots::new();
        let mut writes = Slots::new();
        let mut node_writes = Slots::new();
        let op = match matcher {
            AstMatcher::Call(call) => return self.call(call),
            AstMatcher::Semantic(Semantic::Binding(_)) => return self.binding(matcher),
            AstMatcher::Semantic(Semantic::Parse(test)) => return self.parsed(test),
            AstMatcher::Semantic(Semantic::FormatArgs(test)) => return self.format_args(test),
            AstMatcher::Constrained(constrained) => return self.constrained(constrained),
            AstMatcher::NthChild(nth) => return self.nth_child(nth),
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
                Op::Predicate(self.pure.set(matcher)?)
            }
            AstMatcher::Pattern(pattern) => {
                if let PatternNode::MetaVar {
                    meta_var: MetaVariable::Capture(name, true),
                } = &pattern.node
                {
                    if !matches!(pattern.strictness, ast_grep_core::MatchStrictness::Smart) {
                        return Err(
                            "outcome relations: whole captures require Smart matching".into()
                        );
                    }
                    let slot = self.slot(name, CaptureKind::Node)?;
                    mentions.insert(slot);
                    writes.insert(slot);
                    node_writes.insert(slot);
                    Op::Bind(slot)
                } else {
                    let (op, slots) = self.pattern(pattern)?;
                    mentions.extend(&slots);
                    writes.extend(&slots);
                    node_writes.extend(&slots);
                    op
                }
            }
            AstMatcher::Semantic(Semantic::Capture(name)) => {
                let slot = self.slot(name, CaptureKind::Node)?;
                mentions.insert(slot);
                writes.insert(slot);
                node_writes.insert(slot);
                Op::CaptureBind(slot)
            }
            AstMatcher::Semantic(Semantic::Host(test)) => return self.hosted(test),
            AstMatcher::Semantic(Semantic::Value(test))
                if !test.reading.literal || self.parsed_depth > 0 =>
            {
                self.atomic(matcher)
            }
            AstMatcher::Semantic(Semantic::Resolves(_)) => self.atomic(matcher),
            AstMatcher::Semantic(Semantic::Binds(_)) => return self.binds(matcher),
            AstMatcher::Semantic(Semantic::Layout(test)) if test.reads().next().is_none() => {
                self.atomic(matcher)
            }
            AstMatcher::Semantic(Semantic::Layout(_)) | AstMatcher::Compare(_) => {
                return self.capture_predicate(matcher);
            }
            AstMatcher::Semantic(Semantic::Arguments(test)) if test.bind.is_empty() => {
                self.atomic(matcher)
            }
            AstMatcher::Semantic(Semantic::Arguments(_)) => return self.arguments(matcher),
            AstMatcher::TestUnknown(regex) => {
                let super::super::ir::Set::Regex(term) =
                    self.pure.set(&AstMatcher::Regex(regex.clone()))?
                else {
                    unreachable!()
                };
                Op::Unknown(term)
            }
            AstMatcher::All(items) | AstMatcher::Any(items) => {
                let children = items
                    .iter()
                    .map(|m| self.plan(m))
                    .collect::<Result<Vec<_>, _>>()?;
                for child in &children {
                    mentions.extend(&child.mentions);
                    writes.extend(&child.writes);
                    node_writes.extend(&child.node_writes);
                }
                if matches!(matcher, AstMatcher::All(_)) {
                    Op::All(children)
                } else {
                    Op::Any(children)
                }
            }
            AstMatcher::Not(inner) => {
                let child = self.plan(inner)?;
                mentions = child.mentions.clone();
                Op::Not(Box::new(child))
            }
            AstMatcher::Field(field) if matches!(field.stop, StopBy::Neighbor | StopBy::End) => {
                let mut child = self.plan(&field.rule)?;
                if !self.three_valued
                    && let Some((kinds, clauses)) =
                        crate::code_rule_dsl::matcher::walk_candidate_filter(&field.rule)
                {
                    let mut text = vec![];
                    if !field.inside && matches!(field.stop, StopBy::End) {
                        for clause in clauses {
                            let regex = regex::Regex::new(
                                &clause
                                    .iter()
                                    .map(|s| regex::escape(s))
                                    .collect::<Vec<_>>()
                                    .join("|"),
                            )
                            .map_err(|e| format!("candidate text filter: {e}"))?;
                            let super::super::ir::Set::Regex(id) =
                                self.pure.set(&AstMatcher::Regex(regex))?
                            else {
                                unreachable!()
                            };
                            text.push(id);
                        }
                    }
                    child.walk_filter = Some(super::WalkFilter { kinds, text });
                }
                mentions = child.mentions.clone();
                writes = child.writes.clone();
                node_writes = child.node_writes.clone();
                if matches!(field.stop, StopBy::End) {
                    Op::FieldEnd {
                        inside: field.inside,
                        field: field.field,
                        child: Box::new(child),
                    }
                } else {
                    Op::FieldNeighbor {
                        inside: field.inside,
                        field: field.field,
                        child: Box::new(child),
                    }
                }
            }
            AstMatcher::Inside(..)
            | AstMatcher::Has(..)
            | AstMatcher::Follows(..)
            | AstMatcher::Precedes(..) => {
                let (axis, by, rule) = self.walk(matcher)?;
                let end = !matches!(by, StopBy::Neighbor);
                let stop = self.stop(by)?;
                let mut child = self.plan(rule)?;
                if !self.three_valued
                    && let Some((kinds, clauses)) =
                        crate::code_rule_dsl::matcher::walk_candidate_filter(rule)
                {
                    let mut text = vec![];
                    if matches!(axis, Axis::Child) && end && stop.is_none() {
                        for clause in clauses {
                            let regex = regex::Regex::new(
                                &clause
                                    .iter()
                                    .map(|s| regex::escape(s))
                                    .collect::<Vec<_>>()
                                    .join("|"),
                            )
                            .map_err(|e| format!("candidate text filter: {e}"))?;
                            let super::super::ir::Set::Regex(id) =
                                self.pure.set(&AstMatcher::Regex(regex))?
                            else {
                                unreachable!()
                            };
                            text.push(id);
                        }
                    }
                    child.walk_filter = Some(super::WalkFilter { kinds, text });
                }
                mentions = child.mentions.clone();
                writes = child.writes.clone();
                node_writes = child.node_writes.clone();
                Op::Search(axis, end, Box::new(child), stop)
            }
            AstMatcher::Count(count) => {
                let (axis, by, rule) = self.walk(&count.relation)?;
                let end = !matches!(by, StopBy::Neighbor);
                let stop = self.stop(by)?;
                let child = self.plan(rule)?;
                mentions = child.mentions.clone();
                let binding = if let Some(binding) = &count.binding {
                    binding.offset.checked_add(u32::MAX as i64).ok_or(
                        "outcome relations: count offset can exceed signed integer storage",
                    )?;
                    let slot = self.slot(&binding.name, CaptureKind::Count)?;
                    writes.insert(slot);
                    Some((slot, binding.offset))
                } else {
                    None
                };
                Op::Count {
                    axis,
                    end,
                    rule: Box::new(child),
                    stop,
                    min: i64::try_from(count.min).map_err(
                        |_| "outcome relations: count minimum exceeds signed integer storage",
                    )?,
                    max: count.max.map(i64::try_from).transpose().map_err(
                        |_| "outcome relations: count maximum exceeds signed integer storage",
                    )?,
                    binding,
                }
            }
            AstMatcher::Util(util) => {
                self.utility_calls += 1;
                let key = (util.table.as_ptr() as usize, util.index);
                let scoped_key = (
                    key.0,
                    key.1,
                    format!(
                        "{}@parsed={}@hosted={}",
                        self.namespace,
                        self.parsed_depth > 0,
                        self.hosted_depth > 0
                    ),
                );
                let parsed_pure = if self.parsed_depth > 0 {
                    pure_recursive_lower::try_lower(util, &mut self.pure, self.three_valued)?
                } else {
                    None
                };
                if let Some(program) = parsed_pure {
                    Op::PureRecursive(Rc::new(program))
                } else if let Some((id, chain)) = self.chains.get(&key) {
                    Op::Chain(*id, Rc::clone(chain))
                } else if let Some(chain) =
                    super::super::recursive_chain::try_lower(util, &mut self.pure)?
                        .filter(|chain| chain.guard_kinds().is_some())
                {
                    if chain.guard_kinds().is_none() {
                        return Err("outcome relations: recursive outcomes require a finite continuation kind filter for shared chain summaries".into());
                    }
                    let id = self.chains.len();
                    let chain = Rc::new(chain);
                    self.chains.insert(key, (id, Rc::clone(&chain)));
                    Op::Chain(id, chain)
                } else if let Some((id, downward)) = self.downward.get(&key) {
                    Op::Downward(*id, Rc::clone(downward))
                } else if let Some(mut downward) =
                    super::super::recursive_downward::try_lower(util, &mut self.pure)?
                        .filter(|component| component.guard_kinds().is_some())
                {
                    if downward.guard_kinds().is_none() {
                        return Err(
                            "outcome relations: downward recursion requires finite guard kinds"
                                .into(),
                        );
                    }
                    if !self.three_valued {
                        downward.child_kinds =
                            crate::code_rule_dsl::matcher::walk_candidate_filter(matcher)
                                .map(|(kinds, _)| kinds);
                    }
                    let id = self.downward.len();
                    let downward = Rc::new(downward);
                    self.downward.insert(key, (id, Rc::clone(&downward)));
                    Op::Downward(id, downward)
                } else if let Some(program) =
                    pure_recursive_lower::try_lower(util, &mut self.pure, self.three_valued)?
                {
                    Op::PureRecursive(Rc::new(program))
                } else if let Some(op) = self.mixed_recursive(util)? {
                    if let Op::MixedRecursive { program, .. } = &op {
                        mentions = program.mentions.clone();
                    }
                    op
                } else if let Some(op) = self.recursive_search(util)? {
                    if let Op::RecursiveSearch { component, .. } = &op {
                        mentions = component.base.mentions.clone();
                        writes = component.base.writes.clone();
                        node_writes = component.base.node_writes.clone();
                    }
                    op
                } else {
                    let (id, child) = if let Some((id, child)) = self.utilities.get(&scoped_key) {
                        (*id, Rc::clone(child))
                    } else {
                        if !self.active.insert(key) {
                            return Err(
                                "outcome relations: recursive outcomes need a component contract"
                                    .into(),
                            );
                        }
                        let result = util.with_target(|rule| self.plan(rule));
                        self.active.remove(&key);
                        let child = Rc::new(result.ok_or("outcome relations: missing utility")??);
                        let id = self.utilities.len();
                        self.utilities.insert(scoped_key, (id, Rc::clone(&child)));
                        let scope = util
                            .scope()
                            .flatten()
                            .map(|names| names.iter().map(|name| name.to_string()).collect());
                        self.utility_origins.insert(id, (key, scope));
                        (id, child)
                    };
                    mentions = child.mentions.clone();
                    writes = child.writes.clone();
                    node_writes = child.node_writes.clone();
                    if self.parsed_depth > 0 || self.hosted_depth > 0 {
                        Op::TransientCharge(child)
                    } else {
                        Op::Charge(id, child)
                    }
                }
            }
            _ => {
                let missing = match matcher {
                    AstMatcher::Semantic(Semantic::Binding(_)) => "binding-site composition",
                    AstMatcher::Semantic(Semantic::Parse(_)) => "parsed-tree composition",
                    AstMatcher::Semantic(Semantic::Host(_)) => "host-tree composition",
                    AstMatcher::Semantic(Semantic::FormatArgs(_)) => "format-argument composition",
                    AstMatcher::Semantic(Semantic::Calls(_)) => "call-graph traversal",
                    AstMatcher::Semantic(Semantic::Capture(_)) => "semantic capture",
                    AstMatcher::Semantic(Semantic::Value(_)) => "literal value",
                    AstMatcher::Call(_) => "parameterized utility calls",
                    AstMatcher::Field(_) => "Rule-stopped field traversal",
                    AstMatcher::NthChild(_) => "sibling index",
                    _ => "unclassified operator",
                };
                return Err(format!(
                    "outcome relations: leaf or operator lacks a total, effect-free outcome contract: {missing}"
                ));
            }
        };
        let mut effects = super::effects::Effects::of(&op);
        if let Op::Bind(slot) = &op {
            effects.typed_exports |= self.span_slots.contains(slot);
        }
        if matches!(op, Op::Value(_)) {
            // Resolution and layout may spend work without reading values. Native utility
            // memoization depends on this distinction, including PHP's flow-sensitive names.
            effects.reads_value = crate::code_rule_dsl::matcher::reads_values(
                matcher,
                &mut std::collections::BTreeSet::new(),
            );
            effects.cache_fills = Self::cache_fills(matcher);
        }
        Ok(Plan {
            effects,
            op,
            mentions,
            writes,
            node_writes,
            walk_filter: None,
        })
    }
}
