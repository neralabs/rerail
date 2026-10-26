//! Generic document operation cohorts with explicit capture/continuation frames.
//! Only semantic predicates cross the Host boundary. Structural evaluation never
//! calls AstMatcher's scalar Matcher implementation or installs matcher TLS.
use super::memo::{CaptureFingerprint, capture_fingerprint};
use super::{AstMatcher, CaptureAnalysis, Semantic, StopBy, UtilLimit};
use super::{cohort_memo, pattern_batch, wave};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Matcher, Node};
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, OnceLock, Weak};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct Exclusion {
    pub name: String,
    pub ranges: BTreeSet<(usize, usize)>,
}
pub(in crate::code_rule_dsl) struct Request<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
> {
    pub program: u32,
    pub context: u32,
    pub node: Node<'t, D>,
    pub env: Arc<MetaVarEnv<'t, D>>,
    pub three: bool,
    pub each: Option<Arc<Exclusion>>,
    pub following: Arc<Vec<(usize, usize)>>,
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Clone for Request<'t, D> {
    fn clone(&self) -> Self {
        Self {
            program: self.program,
            context: self.context,
            node: self.node.clone(),
            env: self.env.clone(),
            three: self.three,
            each: self.each.clone(),
            following: self.following.clone(),
        }
    }
}
/// Immutable metadata belongs to one resident root, including nested-host roots.
struct LaneMeta {
    context: u32,
    three: bool,
    following: Arc<Vec<(usize, usize)>>,
}
/// Only fields that can change within a physical continuation are copied to children.
struct Input<'t, D: Doc<Source: Content<Underlying = u8>>> {
    program: u32,
    node: Node<'t, D>,
    env: Arc<MetaVarEnv<'t, D>>,
    // Truth normalization stays local to the continuation without a metadata lookup.
    three: bool,
    each: Option<Arc<Exclusion>>,
}
impl<'t, D: Doc<Source: Content<Underlying = u8>>> Clone for Input<'t, D> {
    fn clone(&self) -> Self {
        Self {
            program: self.program,
            node: self.node.clone(),
            env: self.env.clone(),
            three: self.three,
            each: self.each.clone(),
        }
    }
}
impl<'t, D: Doc<Source: Content<Underlying = u8>>> Request<'t, D> {
    fn split(self) -> (Input<'t, D>, LaneMeta) {
        (
            Input {
                program: self.program,
                node: self.node,
                env: self.env,
                three: self.three,
                each: self.each,
            },
            LaneMeta {
                context: self.context,
                three: self.three,
                following: self.following,
            },
        )
    }
}
impl<'t, D: Doc<Source: Content<Underlying = u8>>> Input<'t, D> {
    fn into_request(self, meta: LaneMeta) -> Request<'t, D> {
        debug_assert_eq!(self.three, meta.three);
        Request {
            program: self.program,
            context: meta.context,
            node: self.node,
            env: self.env,
            three: meta.three,
            each: self.each,
            following: meta.following,
        }
    }
    fn leaf<'a>(&'a self, meta: &'a LaneMeta, depth: u32) -> LeafJob<'a, 't, D> {
        LeafJob {
            program: self.program,
            node: &self.node,
            env: &self.env,
            each: self.each.as_ref(),
            meta,
            depth,
        }
    }
}
/// Borrowed jobs never own root metadata or clone input capture frames.
struct LeafJob<'a, 't, D: Doc<Source: Content<Underlying = u8>>> {
    program: u32,
    node: &'a Node<'t, D>,
    env: &'a Arc<MetaVarEnv<'t, D>>,
    each: Option<&'a Arc<Exclusion>>,
    meta: &'a LaneMeta,
    depth: u32,
}
impl<D: Doc<Source: Content<Underlying = u8>>> Copy for LeafJob<'_, '_, D> {}
impl<D: Doc<Source: Content<Underlying = u8>>> Clone for LeafJob<'_, '_, D> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, 't, D: Doc<Source: Content<Underlying = u8>>> LeafJob<'a, 't, D> {
    fn to_request(&self) -> Request<'t, D> {
        Request {
            program: self.program,
            context: self.meta.context,
            node: self.node.clone(),
            env: self.env.clone(),
            three: self.meta.three,
            each: self.each.cloned(),
            following: self.meta.following.clone(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::code_rule_dsl) enum Failure {
    Cancelled,
    Utility(UtilLimit),
}
pub(in crate::code_rule_dsl) struct Result<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
> {
    /// False = 0, unknown = 1, true = 2.
    pub truth: u8,
    pub env: Arc<MetaVarEnv<'t, D>>,
    pub work: usize,
    pub replay_work: usize,
    pub levels: u32,
    pub clause_state_read: bool,
    pub failure: Option<Failure>,
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Clone for Result<'t, D> {
    fn clone(&self) -> Self {
        Self {
            truth: self.truth,
            env: self.env.clone(),
            work: self.work,
            replay_work: self.replay_work,
            levels: self.levels,
            clause_state_read: self.clause_state_read,
            failure: self.failure,
        }
    }
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Result<'t, D> {
    pub fn new(truth: u8, env: Arc<MetaVarEnv<'t, D>>) -> Self {
        Self {
            truth,
            env,
            work: 0,
            replay_work: 0,
            levels: 0,
            clause_state_read: false,
            failure: None,
        }
    }
}
pub(in crate::code_rule_dsl) struct SemanticRequest<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
> {
    pub semantic: Arc<Semantic>,
    pub request: Request<'t, D>,
    pub utility_depth: u32,
}
pub(in crate::code_rule_dsl) struct CompareRequest<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
> {
    pub comparison: Arc<super::Comparison>,
    pub request: Request<'t, D>,
    pub utility_depth: u32,
}
pub(in crate::code_rule_dsl) struct NavigationRequest<'t, D: Doc<Source: Content<Underlying = u8>>>
{
    pub request: Request<'t, D>,
    pub utility_depth: u32,
    pub related: Option<Arc<super::Related>>,
    pub filter: Option<Arc<super::relation::WalkFilter>>,
    pub descendants: bool,
    pub forward: Option<bool>,
}
pub(in crate::code_rule_dsl) struct NavigationResult<'t, D: Doc<Source: Content<Underlying = u8>>> {
    pub nodes: Option<Vec<Node<'t, D>>>,
    pub work: usize,
    pub replay_work: usize,
    pub levels: u32,
    pub clause_state_read: bool,
    pub failure: Option<Failure>,
}
pub(in crate::code_rule_dsl) trait Host<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
>
{
    fn evaluate(
        &mut self,
        programs: &Programs,
        leaves: Vec<SemanticRequest<'t, D>>,
    ) -> Vec<Result<'t, D>>;
    fn compare(
        &mut self,
        programs: &Programs,
        comparisons: Vec<CompareRequest<'t, D>>,
    ) -> Vec<Result<'t, D>>;
    fn navigate(
        &mut self,
        _programs: &Programs,
        requests: Vec<NavigationRequest<'t, D>>,
    ) -> Vec<NavigationResult<'t, D>> {
        requests
            .into_iter()
            .map(|_| NavigationResult {
                nodes: None,
                work: 0,
                replay_work: 0,
                levels: 0,
                clause_state_read: false,
                failure: None,
            })
            .collect()
    }
    fn region_allowance(&self, _context: u32) -> usize {
        usize::MAX
    }
    fn region_cancelled(&self, _context: u32) -> bool {
        false
    }
    fn is_file(&self, _context: u32, _node: &Node<'t, D>) -> bool {
        false
    }
    fn can_remember(&self, context: u32, node: &Node<'t, D>) -> bool {
        self.is_file(context, node)
    }
    fn failure(&mut self, _context: u32) -> Option<Failure> {
        None
    }
    fn spend(&mut self, _context: u32, _amount: usize) -> bool {
        true
    }
    fn binding_range(
        &mut self,
        _context: u32,
        env: &MetaVarEnv<'t, D>,
        name: &str,
    ) -> Option<Range<usize>> {
        env.get_match(name)
            .map(Node::range)
            .or_else(|| super::exported::exported_span(env, name))
    }
}
pub(in crate::code_rule_dsl) trait Source<
    't,
    D: Doc<Source: Content<Underlying = u8>>,
    H: Host<'t, D>,
>
{
    type Token;
    fn next(&mut self, host: &mut H) -> Option<(Self::Token, Request<'t, D>)>;
    fn completed(
        &mut self,
        token: Self::Token,
        request: Request<'t, D>,
        answer: Result<'t, D>,
        host: &mut H,
    );
}
struct VectorSource<'t, D: Doc<Source: Content<Underlying = u8>>> {
    pending: std::iter::Enumerate<std::vec::IntoIter<Request<'t, D>>>,
    answers: Vec<Option<Result<'t, D>>>,
}
impl<'t, D: Doc<Source: Content<Underlying = u8>>, H: Host<'t, D>> Source<'t, D, H>
    for VectorSource<'t, D>
{
    type Token = usize;
    fn next(&mut self, _: &mut H) -> Option<(usize, Request<'t, D>)> {
        self.pending.next()
    }
    fn completed(&mut self, token: usize, _: Request<'t, D>, answer: Result<'t, D>, _: &mut H) {
        self.answers[token] = Some(answer);
    }
}
#[derive(Clone, Copy)]
enum Stop {
    Neighbor,
    End,
    Rule(u32),
}
#[derive(Clone)]
struct Walk {
    axis: u8,
    child: u32,
    stop: Stop,
    field: Option<u16>,
    filter: Option<Arc<super::relation::WalkFilter>>,
    related: Option<Arc<super::Related>>,
}
#[derive(Clone)]
enum Op {
    Pass,
    Unexcluded,
    Pattern(Arc<pattern_batch::Plan>),
    Kind(u16),
    Regex(regex::Regex),
    Capture(String),
    Compare(Arc<super::Comparison>),
    Semantic(Arc<Semantic>),
    All(Vec<u32>),
    Any(Vec<u32>),
    Not(u32),
    Walk(Walk),
    Count {
        walk: Walk,
        min: usize,
        max: Option<usize>,
        binding: Option<super::CountBinding>,
    },
    Nth {
        cache_term: Option<usize>,
        child: Option<u32>,
        step: i32,
        offset: i32,
        reverse: bool,
    },
    Util {
        memo_term: Option<(usize, usize)>,
        target: Option<u32>,
        args: Option<super::CallArgs>,
        scope: Option<Arc<[Arc<str>]>>,
        scope_id: u32,
    },
    Constrained {
        rule: u32,
        constraints: Vec<(String, u32)>,
    },
    Never,
    #[cfg(test)]
    TestUnknown(regex::Regex),
}
#[derive(Clone)]
struct KindGuard {
    kinds: Option<Vec<u16>>,
    requires_unexcluded: bool,
}
impl KindGuard {
    // This proof uses the source tree, never a necessary guard on an opaque opcode.
    fn exact(rule: &AstMatcher) -> Option<Self> {
        match rule {
            AstMatcher::Kind(kind) => Some(Self {
                kinds: Some(kind.potential_kinds()?.iter().map(|k| k as u16).collect()),
                requires_unexcluded: false,
            }),
            AstMatcher::All(items) => {
                let mut guard = Self {
                    kinds: None,
                    requires_unexcluded: false,
                };
                for item in items {
                    guard.intersect(Self::exact(item)?);
                }
                Some(guard)
            }
            AstMatcher::Any(items) => {
                let mut kinds = Some(Vec::new());
                for item in items {
                    let item = Self::exact(item)?;
                    kinds = match (kinds, item.kinds) {
                        (Some(mut left), Some(right)) => {
                            left.extend(right);
                            left.sort_unstable();
                            left.dedup();
                            Some(left)
                        }
                        _ => None,
                    };
                }
                Some(Self {
                    kinds,
                    requires_unexcluded: true,
                })
            }
            _ => None,
        }
    }
    fn intersect(&mut self, other: Self) {
        self.kinds = match (self.kinds.take(), other.kinds) {
            (Some(left), Some(right)) => Some(
                left.into_iter()
                    .filter(|kind| right.binary_search(kind).is_ok())
                    .collect(),
            ),
            (left, right) => left.or(right),
        };
        self.requires_unexcluded |= other.requires_unexcluded;
    }
}
struct Entry {
    op: Op,
    binds: BTreeSet<String>,
    mentions: BTreeSet<String>,
    may_bind: bool,
    requires_order: OnceLock<bool>,
    requires_order_regions: OnceLock<bool>,
    closed: Option<u32>,
    pipeline: bool,
    rollback_nontrue: bool,
    guard: Option<KindGuard>,
}
#[derive(Clone, Copy)]
struct ProgramClass {
    atomic: u8,
    start: u8,
}
impl ProgramClass {
    fn of(op: &Op) -> Self {
        let atomic = match op {
            Op::Pattern(_) => 0,
            Op::Kind(_) => 1,
            Op::Regex(_) => 2,
            Op::Capture(_) => 3,
            Op::Compare(_) => 4,
            Op::Semantic(_) => 5,
            Op::Pass | Op::Unexcluded | Op::Never => 6,
            #[cfg(test)]
            Op::TestUnknown(_) => 6,
            _ => u8::MAX,
        };
        let start = match op {
            Op::All(_) => 0,
            Op::Any(_) => 1,
            Op::Not(_) => 2,
            Op::Constrained { .. } => 3,
            Op::Walk(_) => 4,
            Op::Count { .. } => 5,
            Op::Nth { .. } => 6,
            Op::Util { .. } => 7,
            _ => 8,
        };
        Self { atomic, start }
    }
}
pub(in crate::code_rule_dsl) struct Programs {
    identity: u64,
    entries: Vec<Entry>,
    // Hot dispatch metadata stays contiguous, separate from plans and capture sets.
    classes: Vec<ProgramClass>,
    pointers: HashMap<usize, u32>,
    analysis: CaptureAnalysis,
    closed: wave::Programs,
    pattern_plans: pattern_batch::Plans,
    absorbed_kind_stages: usize,
    unexcluded: Option<u32>,
    fingerprint_scopes: HashMap<Option<Arc<[Arc<str>]>>, u32>,
    fingerprint_scope_programs: usize,
}
impl Default for Programs {
    fn default() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            identity: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            entries: Vec::new(),
            classes: Vec::new(),
            pointers: HashMap::new(),
            analysis: CaptureAnalysis::default(),
            closed: wave::Programs::default(),
            pattern_plans: pattern_batch::Plans::default(),
            absorbed_kind_stages: 0,
            unexcluded: None,
            fingerprint_scopes: HashMap::new(),
            fingerprint_scope_programs: 0,
        }
    }
}
impl Programs {
    fn intern_fingerprint_scope(&mut self, scope: Option<Arc<[Arc<str>]>>) -> u32 {
        let next = self.fingerprint_scopes.len() as u32;
        *self.fingerprint_scopes.entry(scope).or_insert(next)
    }

    fn operation_name(&self, program: u32) -> &'static str {
        match &self.entries[program as usize].op {
            Op::Pass => "Pass",
            Op::Unexcluded => "Unexcluded",
            Op::Pattern(_) => "Pattern",
            Op::Kind(_) => "Kind",
            Op::Regex(_) => "Regex",
            Op::Capture(_) => "Capture",
            Op::Compare(_) => "Compare",
            Op::Never => "Never",
            Op::All(_) => "All",
            Op::Any(_) => "Any",
            Op::Not(_) => "Not",
            Op::Constrained { .. } => "Constrained",
            Op::Nth { .. } => "Nth",
            Op::Count { .. } => "Count",
            Op::Walk(walk) => match walk.axis {
                0 => "Inside",
                1 => "Has",
                2 => "Precedes",
                _ => "Follows",
            },
            Op::Util { args, .. } => {
                if args.is_some() {
                    "Call"
                } else {
                    "Util"
                }
            }
            Op::Semantic(semantic) => match semantic.as_ref() {
                Semantic::Value(_) => "Value",
                Semantic::Arguments(_) => "Arguments",
                Semantic::Binding(_) => "Binding",
                Semantic::Binds(_) => "Binds",
                Semantic::Resolves(_) => "Resolves",
                Semantic::Parse(_) => "Parse",
                Semantic::Host(_) => "Host",
                Semantic::Capture(_) => "Capture",
                Semantic::FormatArgs(_) => "FormatArgs",
                Semantic::Layout(_) => "Layout",
                Semantic::Calls(_) => "Calls",
            },
            #[cfg(test)]
            Op::TestUnknown(_) => "Unknown",
        }
    }
    fn control_descriptor(&self, program: u32) -> String {
        let mut children = Vec::new();
        match &self.entries[program as usize].op {
            Op::All(items) | Op::Any(items) => children.extend(items.iter().copied()),
            Op::Not(child) => children.push(*child),
            Op::Constrained { rule, constraints } => {
                children.push(*rule);
                children.extend(constraints.iter().map(|(_, child)| *child));
            }
            Op::Walk(walk) | Op::Count { walk, .. } => {
                children.push(walk.child);
                if let Stop::Rule(stop) = walk.stop {
                    children.push(stop);
                }
            }
            Op::Nth { child, .. } => children.extend(child),
            Op::Util { target, .. } => children.extend(target),
            _ => {}
        }
        let mut descriptor = if self.entries[program as usize].rollback_nontrue {
            format!("Txn:{}", self.operation_name(program))
        } else {
            self.operation_name(program).to_owned()
        };
        if self.entries[program as usize].guard.is_some() {
            descriptor.insert_str(0, "Guard:");
        }
        if !children.is_empty() {
            descriptor.push('[');
            for (index, child) in children.iter().take(8).enumerate() {
                if index != 0 {
                    descriptor.push(',');
                }
                descriptor.push_str(self.operation_name(*child));
            }
            if children.len() > 8 {
                descriptor.push_str(&format!(",+{}", children.len() - 8));
            }
            descriptor.push(']');
        }
        descriptor
    }
    fn requires_order(&self, program: u32) -> bool {
        self.requires_order_with_regions(program, false)
    }
    fn requires_order_with_regions(&self, program: u32, regions: bool) -> bool {
        let entry = &self.entries[program as usize];
        let cached = if regions {
            &entry.requires_order_regions
        } else {
            &entry.requires_order
        };
        *cached.get_or_init(|| {
            let mut seen = rustc_hash::FxHashSet::default();
            let mut pending = vec![program];
            while let Some(id) = pending.pop() {
                if !seen.insert(id) || (regions && self.entries[id as usize].closed.is_some()) {
                    continue;
                }
                let mut walk_children = |walk: &Walk| {
                    pending.push(walk.child);
                    if let Stop::Rule(stop) = walk.stop {
                        pending.push(stop);
                    }
                };
                match &self.entries[id as usize].op {
                    Op::Walk(walk) => {
                        if walk.field.is_none()
                            && matches!(walk.stop, Stop::End)
                            && walk.related.as_ref().is_some_and(|r| r.closure_eligible())
                        {
                            return true;
                        }
                        walk_children(walk);
                    }
                    Op::Count { walk, .. } => walk_children(walk),
                    Op::Nth {
                        child, cache_term, ..
                    } => {
                        if child.is_some() && cache_term.is_some() {
                            return true;
                        }
                        pending.extend(child);
                    }
                    Op::All(children) | Op::Any(children) => pending.extend(children),
                    Op::Not(child) => pending.push(*child),
                    Op::Util { target, .. } => pending.extend(target),
                    Op::Constrained { rule, constraints } => {
                        pending.push(*rule);
                        pending.extend(constraints.iter().map(|(_, child)| *child));
                    }
                    Op::Semantic(semantic) => {
                        let rules: Vec<&AstMatcher> = match semantic.as_ref() {
                            Semantic::Binding(b) => b.sub_rules().collect(),
                            Semantic::Binds(b) => b.sub_rules().collect(),
                            Semantic::Parse(p) => vec![&p.rule],
                            Semantic::Host(h) => vec![&h.rule],
                            Semantic::FormatArgs(f) => {
                                [&f.unused, &f.missing].into_iter().flatten().collect()
                            }
                            _ => Vec::new(),
                        };
                        pending.extend(rules.into_iter().filter_map(|rule| self.lookup(rule)));
                    }
                    _ => {}
                }
            }
            false
        })
    }
    fn children(&self, program: u32) -> &[u32] {
        match &self.entries[program as usize].op {
            Op::All(c) | Op::Any(c) => c,
            _ => unreachable!("composite program"),
        }
    }
    #[inline]
    fn atomic(&self, program: u32) -> Option<usize> {
        let family = self.classes[program as usize].atomic;
        (family != u8::MAX).then_some(family as usize)
    }
    pub fn lookup(&self, rule: &AstMatcher) -> Option<u32> {
        self.pointers.get(&(rule as *const _ as usize)).copied()
    }
    fn stop(&mut self, stop: &StopBy) -> Stop {
        match stop {
            StopBy::Neighbor => Stop::Neighbor,
            StopBy::End => Stop::End,
            StopBy::Rule(r) => Stop::Rule(self.lower(r)),
        }
    }
    fn walk(&mut self, r: &AstMatcher) -> Option<Walk> {
        let (axis, rule, stop) = match r {
            AstMatcher::Inside(r, s) => (0, r.as_ref(), s),
            AstMatcher::Has(r, s) => (1, r.as_ref(), s),
            AstMatcher::Precedes(r, s) => (2, r.as_ref(), s),
            AstMatcher::Follows(r, s) => (3, r.as_ref(), s),
            AstMatcher::Field(f) => {
                return Some(Walk {
                    axis: if f.inside { 0 } else { 1 },
                    child: self.lower(&f.rule),
                    stop: self.stop(&f.stop),
                    field: Some(f.field),
                    filter: super::relation::WalkFilter::for_batch(&f.rule),
                    related: None,
                });
            }
            _ => return None,
        };
        Some(Walk {
            axis,
            child: self.lower(rule.rule()),
            stop: self.stop(stop),
            field: None,
            filter: super::relation::WalkFilter::for_batch(rule.rule()),
            related: Some(Arc::new(rule.clone())),
        })
    }
    fn unexcluded_stage(&mut self) -> u32 {
        if let Some(program) = self.unexcluded {
            return program;
        }
        let program = self.entries.len() as u32;
        self.classes.push(ProgramClass::of(&Op::Unexcluded));
        self.entries.push(Entry {
            op: Op::Unexcluded,
            binds: BTreeSet::new(),
            mentions: BTreeSet::new(),
            may_bind: false,
            requires_order: OnceLock::new(),
            requires_order_regions: OnceLock::new(),
            closed: None,
            pipeline: false,
            rollback_nontrue: false,
            guard: None,
        });
        self.unexcluded = Some(program);
        program
    }
    pub fn lower(&mut self, rule: &AstMatcher) -> u32 {
        if let Some(id) = self.lookup(rule) {
            return id;
        }
        let id = self.entries.len() as u32;
        self.pointers.insert(rule as *const _ as usize, id);
        let binds = self.analysis.binds(rule);
        let mentions = self.analysis.mentions(rule);
        let may_bind = !binds.is_empty() || !self.analysis.counts(rule).is_empty();
        self.classes.push(ProgramClass::of(&Op::Never));
        self.entries.push(Entry {
            op: Op::Never,
            binds,
            mentions,
            may_bind,
            requires_order: OnceLock::new(),
            requires_order_regions: OnceLock::new(),
            closed: None,
            pipeline: false,
            rollback_nontrue: false,
            guard: None,
        });
        let mut op = match rule {
            AstMatcher::Pattern(p) => Op::Pattern(self.pattern_plans.compile(p.clone())),
            AstMatcher::Kind(k) => {
                Op::Kind(k.potential_kinds().unwrap().iter().next().unwrap() as u16)
            }
            AstMatcher::Regex(r) => Op::Regex(r.clone()),
            AstMatcher::All(rs) => {
                let mut guard: Option<KindGuard> = None;
                let mut children = Vec::new();
                let mut consumed = 0;
                let mut after_atomic = false;
                for source in rs {
                    if let Some(mut next) = KindGuard::exact(source) {
                        self.absorbed_kind_stages += 1;
                        // A preceding pattern may bind the excluded capture.
                        // Hoist membership only; keep this each check in order.
                        if after_atomic && next.requires_unexcluded {
                            children.push(self.unexcluded_stage());
                            next.requires_unexcluded = false;
                        }
                        if let Some(guard) = &mut guard {
                            guard.intersect(next);
                        } else {
                            guard = Some(next);
                        }
                    } else if matches!(source, AstMatcher::Pattern(_) | AstMatcher::Regex(_)) {
                        children.push(self.lower(source));
                        after_atomic = true;
                    } else {
                        break;
                    }
                    consumed += 1;
                }
                children.extend(rs[consumed..].iter().map(|r| self.lower(r)));
                if children.len() == 1 {
                    self.entries[id as usize].rollback_nontrue = true;
                    if let Some(child_guard) = self.entries[children[0] as usize].guard.clone() {
                        if let Some(guard) = &mut guard {
                            guard.intersect(child_guard);
                        } else {
                            guard = Some(child_guard);
                        }
                    }
                    self.entries[id as usize].guard = guard;
                    self.entries[children[0] as usize].op.clone()
                } else {
                    self.entries[id as usize].guard = guard;
                    if children.is_empty() {
                        Op::Pass
                    } else {
                        Op::All(children)
                    }
                }
            }
            AstMatcher::Any(rs) => {
                let children: Vec<_> = rs.iter().map(|r| self.lower(r)).collect();
                // Only an enforced, zero-work entry test can reject a branch
                // here. Potential kinds of opaque matchers are not such proof.
                let mut kinds = Vec::new();
                let complete = children.iter().all(|child| {
                    let entry = &self.entries[*child as usize];
                    if let Some(required) = entry.guard.as_ref().and_then(|g| g.kinds.as_ref()) {
                        kinds.extend_from_slice(required);
                        true
                    } else if let Op::Kind(kind) = entry.op {
                        kinds.push(kind);
                        true
                    } else {
                        false
                    }
                });
                if complete {
                    kinds.sort_unstable();
                    kinds.dedup();
                    self.entries[id as usize].guard = Some(KindGuard {
                        kinds: Some(kinds),
                        // Positive admission still executes the ordered Any,
                        // including its capture-dependent exclusion checks.
                        requires_unexcluded: false,
                    });
                }
                Op::Any(children)
            }
            AstMatcher::Not(r) => Op::Not(self.lower(r)),
            AstMatcher::Inside(..)
            | AstMatcher::Has(..)
            | AstMatcher::Precedes(..)
            | AstMatcher::Follows(..)
            | AstMatcher::Field(..) => Op::Walk(self.walk(rule).unwrap()),
            AstMatcher::Count(c) => match self.walk(&c.relation) {
                Some(walk) => Op::Count {
                    walk,
                    min: c.min,
                    max: c.max,
                    binding: c.binding.clone(),
                },
                None => Op::Never,
            },
            AstMatcher::NthChild(n) => Op::Nth {
                cache_term: match &n.of_rule {
                    None => Some(0),
                    Some(rule) if self.analysis.mentions(rule).is_empty() => {
                        Some(std::ptr::from_ref(n.as_ref()) as usize)
                    }
                    Some(_) => None,
                },
                child: n.of_rule.as_ref().map(|r| self.lower(r)),
                step: n.position.step,
                offset: n.position.offset,
                reverse: n.reverse,
            },
            AstMatcher::Util(u) => Op::Util {
                memo_term: u.memo_term(),
                target: u.with_target(|r| self.lower(r)),
                args: None,
                scope: u.scope().flatten(),
                scope_id: 0,
            },
            AstMatcher::Call(c) => Op::Util {
                memo_term: Some((std::ptr::from_ref(c.as_ref()) as usize, usize::MAX)),
                target: c.util.with_target(|r| self.lower(r)),
                args: Some(c.args.clone()),
                scope_id: 0,
                scope: Some(
                    c.args
                        .iter()
                        .map(|(_, caller)| Arc::<str>::from(caller.as_str()))
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                ),
            },
            AstMatcher::Constrained(c) => Op::Constrained {
                rule: self.lower(&c.rule),
                constraints: c
                    .constraints
                    .iter()
                    .map(|(n, r)| (n.clone(), self.lower(r)))
                    .collect(),
            },
            AstMatcher::Compare(c) => Op::Compare(Arc::new((**c).clone())),
            AstMatcher::Semantic(Semantic::Capture(name)) => Op::Capture(name.clone()),
            AstMatcher::Semantic(s) => {
                let s = Arc::new(s.clone());
                match s.as_ref() {
                    Semantic::Binding(b) => {
                        for r in b.sub_rules() {
                            self.lower(r);
                        }
                    }
                    Semantic::Binds(b) => {
                        for r in b.sub_rules() {
                            self.lower(r);
                        }
                    }
                    Semantic::Parse(p) => {
                        self.lower(&p.rule);
                    }
                    Semantic::Host(h) => {
                        self.lower(&h.rule);
                    }
                    Semantic::FormatArgs(f) => {
                        for r in [&f.unused, &f.missing].into_iter().flatten() {
                            self.lower(r);
                        }
                    }
                    _ => {}
                }
                Op::Semantic(s)
            }
            AstMatcher::Never => Op::Never,
            #[cfg(test)]
            AstMatcher::TestUnknown(r) => Op::TestUnknown(r.clone()),
        };
        if let Op::Util {
            scope, scope_id, ..
        } = &mut op
        {
            *scope_id = self.intern_fingerprint_scope(scope.clone());
            self.fingerprint_scope_programs += 1;
        }
        self.entries[id as usize].pipeline = match &op {
            Op::Constrained { rule, constraints } => {
                matches!(self.entries[*rule as usize].op, Op::Pattern(_))
                    && constraints
                        .iter()
                        .all(|(_, child)| self.atomic(*child).is_some())
            }
            _ => false,
        };
        self.entries[id as usize].closed = if matches!(
            op,
            Op::Kind(_) | Op::Regex(_) | Op::Never | Op::Pass | Op::Unexcluded
        ) {
            None
        } else {
            self.closed.lower(rule)
        };
        self.classes[id as usize] = ProgramClass::of(&op);
        self.entries[id as usize].op = op;
        id
    }
}
/// Weak identity keeps the allocation address unique without retaining a past
/// enumeration round's growing range set. Equal live snapshots can share results.
#[derive(Clone)]
struct ExclusionIdentity(Weak<Exclusion>);
impl PartialEq for ExclusionIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl Eq for ExclusionIdentity {}
impl std::hash::Hash for ExclusionIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&(self.0.as_ptr() as usize), state);
    }
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct MemoKey {
    term: (usize, usize),
    context: Option<u32>,
    node: usize,
    doc: usize,
    three: bool,
    each: Option<ExclusionIdentity>,
    captures: Arc<CaptureFingerprint>,
    following: Arc<Vec<(usize, usize)>>,
    owner: u64,
}
#[derive(Clone)]
struct MemoEntry<'t, D: Doc<Source: Content<Underlying = u8>>> {
    truth: u8,
    delta: Option<Arc<cohort_memo::Delta<'t, D>>>,
    replay_work: usize,
    levels: u32,
    clause_state_read: bool,
}
#[derive(Default, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Metrics {
    pub borrowed_leaf_lanes: usize,
    pub owned_leaf_requests: usize,
    pub operation_dispatches: usize,
    pub operation_lanes: usize,
    pub operation_family_lanes: [usize; 20],
    pub operation_family_dispatches: [usize; 20],
    pub atomic_family_lanes: [usize; 7],
    pub atomic_family_dispatches: [usize; 7],
    pub max_width: usize,
    pub utility_hits: usize,
    pub utility_kind_checks: usize,
    pub utility_kind_rejections: usize,
    pattern: pattern_batch::Metrics,
    pub submissions: usize,
    pub source_refills: usize,
    pub fused_control_steps: usize,
    pub control_family_lanes: [usize; 20],
    pub control_programs: BTreeMap<String, usize>,
    pub singleton_all_completions: usize,
    pub kind_guard_checks: usize,
    pub kind_guard_rejects: usize,
    pub kind_guard_alternative_skips: usize,
    pub absorbed_kind_stages: usize,
    pub max_program_pool_entries: usize,
    pub max_control_programs_touched: usize,
    pub dense_counter_submissions: usize,
    pub sparse_counter_submissions: usize,
    pub fused_control_family_lanes: [usize; 20],
    pub max_frame_bytes: usize,
    pub max_state_bytes: usize,
    pub max_action_bytes: usize,
    pub pipeline_rows: usize,
    pub pipeline_eliminated_frames: usize,
    pub region_requests: usize,
    pub region_roots: usize,
    pub region_fallbacks: usize,
    pub region_kernel_lanes: u64,
    pub region_kernel_dispatches: u64,
    pub region_posting_builds: u64,
    pub region_posting_nodes: u64,
    pub region_steps: usize,
    pub region_kernel: wave::Metrics,
    pub peak_resident_roots: usize,
    pub submitted_roots: usize,
    pub multi_document_submissions: usize,
    pub multi_program_submissions: usize,
    pub max_documents: usize,
    pub max_programs: usize,
    pub max_submission_width: usize,
    pub utility_evictions: usize,
    pub max_memo_entries: usize,
    pub fingerprint_builds: usize,
    pub fingerprint_hits: usize,
    pub fingerprint_cross_program_hits: usize,
    pub fingerprint_empty_scope_hits: usize,
    pub max_fingerprint_scope_programs: usize,
    pub max_fingerprint_scopes: usize,
    pub closure_hits: usize,
    pub closure_candidates: usize,
    pub navigation_batches: usize,
    pub navigation_lanes: usize,
    pub navigation_projected_lanes: usize,
    pub navigation_projected_nodes: usize,
}
impl Metrics {
    pub fn merge(&mut self, other: &Self) {
        self.borrowed_leaf_lanes += other.borrowed_leaf_lanes;
        self.owned_leaf_requests += other.owned_leaf_requests;
        self.operation_dispatches += other.operation_dispatches;
        self.operation_lanes += other.operation_lanes;
        for (ours, theirs) in self
            .operation_family_lanes
            .iter_mut()
            .zip(other.operation_family_lanes)
        {
            *ours += theirs;
        }
        for (ours, theirs) in self
            .operation_family_dispatches
            .iter_mut()
            .zip(other.operation_family_dispatches)
        {
            *ours += theirs;
        }
        for (ours, theirs) in self
            .atomic_family_lanes
            .iter_mut()
            .zip(other.atomic_family_lanes)
        {
            *ours += theirs;
        }
        for (ours, theirs) in self
            .atomic_family_dispatches
            .iter_mut()
            .zip(other.atomic_family_dispatches)
        {
            *ours += theirs;
        }

        self.max_width = self.max_width.max(other.max_width);
        self.utility_hits += other.utility_hits;
        self.utility_kind_checks += other.utility_kind_checks;
        self.utility_kind_rejections += other.utility_kind_rejections;
        self.submissions += other.submissions;
        self.source_refills += other.source_refills;
        self.fused_control_steps += other.fused_control_steps;
        self.singleton_all_completions += other.singleton_all_completions;
        self.kind_guard_checks += other.kind_guard_checks;
        self.kind_guard_rejects += other.kind_guard_rejects;
        self.kind_guard_alternative_skips += other.kind_guard_alternative_skips;
        self.absorbed_kind_stages = self.absorbed_kind_stages.max(other.absorbed_kind_stages);
        self.max_program_pool_entries = self
            .max_program_pool_entries
            .max(other.max_program_pool_entries);
        self.max_control_programs_touched = self
            .max_control_programs_touched
            .max(other.max_control_programs_touched);
        self.dense_counter_submissions += other.dense_counter_submissions;
        self.sparse_counter_submissions += other.sparse_counter_submissions;
        for (descriptor, count) in &other.control_programs {
            *self.control_programs.entry(descriptor.clone()).or_default() += count;
        }
        self.max_frame_bytes = self.max_frame_bytes.max(other.max_frame_bytes);
        self.max_state_bytes = self.max_state_bytes.max(other.max_state_bytes);
        self.max_action_bytes = self.max_action_bytes.max(other.max_action_bytes);
        for i in 0..20 {
            self.control_family_lanes[i] += other.control_family_lanes[i];
            self.fused_control_family_lanes[i] += other.fused_control_family_lanes[i];
        }
        self.pipeline_rows += other.pipeline_rows;
        self.pipeline_eliminated_frames += other.pipeline_eliminated_frames;
        self.region_requests += other.region_requests;
        self.region_roots += other.region_roots;
        self.region_fallbacks += other.region_fallbacks;
        self.region_kernel_lanes += other.region_kernel_lanes;
        self.region_kernel_dispatches += other.region_kernel_dispatches;
        self.region_posting_builds += other.region_posting_builds;
        self.region_posting_nodes += other.region_posting_nodes;
        self.region_steps += other.region_steps;
        merge_region_metrics(&mut self.region_kernel, &other.region_kernel);
        self.peak_resident_roots = self.peak_resident_roots.max(other.peak_resident_roots);
        self.submitted_roots += other.submitted_roots;
        self.multi_document_submissions += other.multi_document_submissions;
        self.multi_program_submissions += other.multi_program_submissions;
        self.max_documents = self.max_documents.max(other.max_documents);
        self.max_programs = self.max_programs.max(other.max_programs);
        self.max_submission_width = self.max_submission_width.max(other.max_submission_width);
        self.utility_evictions += other.utility_evictions;
        self.max_memo_entries = self.max_memo_entries.max(other.max_memo_entries);
        self.fingerprint_builds += other.fingerprint_builds;
        self.fingerprint_hits += other.fingerprint_hits;
        self.fingerprint_cross_program_hits += other.fingerprint_cross_program_hits;
        self.fingerprint_empty_scope_hits += other.fingerprint_empty_scope_hits;
        self.max_fingerprint_scope_programs = self
            .max_fingerprint_scope_programs
            .max(other.max_fingerprint_scope_programs);
        self.max_fingerprint_scopes = self
            .max_fingerprint_scopes
            .max(other.max_fingerprint_scopes);
        self.closure_hits += other.closure_hits;
        self.closure_candidates += other.closure_candidates;
        self.navigation_batches += other.navigation_batches;
        self.navigation_lanes += other.navigation_lanes;
        self.navigation_projected_lanes += other.navigation_projected_lanes;
        self.navigation_projected_nodes += other.navigation_projected_nodes;

        self.pattern.merge(&other.pattern);
    }
}
fn merge_region_metrics(ours: &mut wave::Metrics, other: &wave::Metrics) {
    macro_rules! sum { ($($field:ident),* $(,)?) => { $(ours.$field += other.$field;)* } }
    macro_rules! peak { ($($field:ident),* $(,)?) => { $(ours.$field = ours.$field.max(other.$field);)* } }
    sum!(
        submissions,
        lanes,
        unique_lanes,
        kernel_dispatches,
        kernel_lanes,
        walk_advances,
        closure_nodes,
        closure_hits,
        posting_builds,
        posting_nodes,
        posting_seeks,
        subtree_materializations,
        subtree_nodes,
        subtree_hits,
        utility_hits,
        text_hits,
        text_evaluations,
        literal_evaluations,
        multi_document_submissions,
        multi_program_submissions
    );
    peak!(
        max_memo_entries,
        max_text_entries,
        max_kernel_width,
        max_documents,
        max_programs
    );
    for i in 0..8 {
        ours.family_dispatches[i] += other.family_dispatches[i];
        ours.family_lanes[i] += other.family_lanes[i];
    }
}
#[derive(Clone, Copy)]
enum CounterStorage {
    Dense,
    Sparse,
}
enum AdvanceCounts {
    Dense {
        values: Vec<usize>,
        touched: Vec<u32>,
    },
    Sparse(FxHashMap<u32, usize>),
}
impl Default for AdvanceCounts {
    fn default() -> Self {
        Self::Sparse(FxHashMap::default())
    }
}
impl AdvanceCounts {
    fn prepare(&mut self, storage: CounterStorage, programs: usize) {
        match storage {
            CounterStorage::Sparse => match self {
                Self::Sparse(values) => values.clear(),
                _ => *self = Self::Sparse(FxHashMap::default()),
            },
            CounterStorage::Dense => match self {
                Self::Dense { values, touched } => {
                    debug_assert!(touched.is_empty());
                    values.resize(programs, 0);
                }
                _ => {
                    *self = Self::Dense {
                        values: vec![0; programs],
                        touched: Vec::new(),
                    }
                }
            },
        }
    }
    #[inline]
    fn record(&mut self, program: u32) {
        match self {
            Self::Dense { values, touched } => {
                let count = &mut values[program as usize];
                if *count == 0 {
                    touched.push(program);
                }
                *count += 1;
            }
            Self::Sparse(values) => *values.entry(program).or_default() += 1,
        }
    }
    fn drain(&mut self) -> Vec<(u32, usize)> {
        match self {
            Self::Dense { values, touched } => touched
                .drain(..)
                .map(|program| (program, std::mem::take(&mut values[program as usize])))
                .collect(),
            Self::Sparse(values) => values.drain().collect(),
        }
    }
}
pub(in crate::code_rule_dsl) struct Bank<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
> {
    memo: FxHashMap<MemoKey, MemoEntry<'t, D>>,
    pattern_cache: pattern_batch::Cache<'t, D>,
    program_advances: AdvanceCounts,
    closures: FxHashMap<ClosureKey, u8>,
    structural_siblings: FxHashMap<(usize, usize), StructuralSiblings<'t, D>>,
    siblings: FxHashMap<SiblingKey, (Arc<Vec<usize>>, bool)>,
    fingerprints: FxHashMap<
        (usize, u64, u32),
        (
            Weak<MetaVarEnv<'t, D>>,
            Option<Arc<str>>,
            Arc<CaptureFingerprint>,
            u32,
        ),
    >,
    empty_fingerprint: Arc<CaptureFingerprint>,
    pub metrics: Metrics,
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Default for Bank<'t, D> {
    fn default() -> Self {
        Self {
            memo: FxHashMap::default(),
            pattern_cache: pattern_batch::Cache::default(),
            program_advances: AdvanceCounts::default(),
            closures: FxHashMap::default(),
            structural_siblings: FxHashMap::default(),
            siblings: FxHashMap::default(),
            fingerprints: FxHashMap::default(),
            empty_fingerprint: Arc::new(capture_fingerprint(&MetaVarEnv::<D>::new(), None)),
            metrics: Metrics {
                fingerprint_builds: 1,
                max_frame_bytes: std::mem::size_of::<Frame<'t, D>>(),
                max_state_bytes: std::mem::size_of::<State<'t, D>>(),
                max_action_bytes: std::mem::size_of::<Action<'t, D>>(),
                ..Metrics::default()
            },
        }
    }
}

type SiblingKey = (u64, usize, usize, usize);

struct Frame<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    request: Input<'t, D>,
    depth: u32,
    state: State<'t, D>,
    work: usize,
    replay: usize,
    levels: u32,
    clause_state_read: bool,
    region_attempted: bool,
}
enum State<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    Start,
    List {
        index: usize,
        atomic: bool,
        any: bool,
        trial: Arc<MetaVarEnv<'t, D>>,
        unknown: bool,
        withheld: Option<Box<BTreeSet<String>>>,
    },
    Not,
    Constraints {
        index: usize,
        first: bool,
    },
    Search(Box<Search<'t, D>>),
    Closure(Box<Closure<'t, D>>),
    Nth(Box<NthState<'t, D>>),
    Util {
        key: Option<Box<MemoKey>>,
    },
}
struct NthState<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    cache: Option<SiblingKey>,
    replay_before: usize,
    child: u32,
    nodes: Vec<Node<'t, D>>,
    index: usize,
    counted: Arc<Vec<usize>>,
    step: i32,
    offset: i32,
    reverse: bool,
    own: bool,
}
struct Visit<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    node: Node<'t, D>,
    eligible: bool,
    descend: u8,
    stop: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ClosureKey {
    owner: u64,
    program: u32,
    context: u32,
    doc: usize,
    node: usize,
    three: bool,
}
struct Regions<'a> {
    executor: wave::Executor<'a>,
    identities: FxHashMap<usize, u32>,
    owners: Vec<u32>,
}
struct StructuralSiblings<'t, D: Doc<Source: Content<Underlying = u8>>> {
    nodes: Vec<Node<'t, D>>,
    positions: FxHashMap<usize, usize>,
}
struct Closure<'t, D: Doc<Source: Content<Underlying = u8>>> {
    plan: Walk,
    key: ClosureKey,
    queue: VecDeque<Node<'t, D>>,
    cursor: Option<Node<'t, D>>,
    current: Option<Node<'t, D>>,
    walked: Vec<(usize, usize, u8, Option<u8>, bool)>,
    open: Vec<(usize, usize)>,
    spent: usize,
}
struct Counter {
    min: usize,
    max: Option<usize>,
    binding: Option<super::CountBinding>,
    found: usize,
    unknown: usize,
    open: bool,
}
struct LinearCursor<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    next: Option<Node<'t, D>>,
    previous: usize,
}
struct Search<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    plan: Walk,
    queue: VecDeque<Visit<'t, D>>,
    linear: Option<LinearCursor<'t, D>>,
    current: Option<Visit<'t, D>>,
    phase: u8,
    stopped: u8,
    unknown: bool,
    counter: Option<Counter>,
    pending_open: bool,
    rollback_input: Option<Arc<MetaVarEnv<'t, D>>>,
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Frame<'t, D> {
    fn new(request: Input<'t, D>, depth: u32) -> Self {
        Self {
            request,
            depth,
            state: State::Start,
            work: 0,
            replay: 0,
            levels: 0,
            clause_state_read: false,
            region_attempted: false,
        }
    }
    #[inline]
    fn control_family(&self, programs: &Programs) -> usize {
        match &self.state {
            State::Start => programs.classes[self.request.program as usize].start as usize,
            State::List { atomic: true, .. } => 8,
            State::List { any: false, .. } => 9,
            State::List { any: true, .. } => 10,
            State::Not => 11,
            State::Constraints { .. } => 12,
            State::Search(s) => 13 + usize::from(s.phase),
            State::Closure(_) => 13,
            State::Nth(_) => 18,
            State::Util { .. } => 19,
        }
    }
    fn leaf_parts<'a>(&'a self, p: &Programs) -> (u32, &'a Arc<MetaVarEnv<'t, D>>) {
        if let State::List {
            atomic: true,
            index,
            any,
            trial,
            ..
        } = &self.state
        {
            (
                p.children(self.request.program)[*index],
                if *any { &self.request.env } else { trial },
            )
        } else {
            (self.request.program, &self.request.env)
        }
    }
    fn leaf<'a>(&'a self, p: &Programs, meta: &'a LaneMeta) -> LeafJob<'a, 't, D> {
        let (program, env) = self.leaf_parts(p);
        LeafJob {
            program,
            node: &self.request.node,
            env,
            each: self.request.each.as_ref(),
            meta,
            depth: self.depth,
        }
    }
    fn rollback_env(&self) -> &Arc<MetaVarEnv<'t, D>> {
        if let State::Search(search) = &self.state {
            if let Some(input) = &search.rollback_input {
                return input;
            }
        }
        &self.request.env
    }
    fn result(&self, truth: u8, env: Option<Arc<MetaVarEnv<'t, D>>>) -> Result<'t, D> {
        let truth = if truth == 1 && !self.request.three {
            0
        } else {
            truth
        };
        Result {
            truth,
            // Most failures pass no frame and roll back. NthChild's selected
            // ofRule intentionally exposes its frame even when An+B rejects it;
            // direct relation searches reuse that frame at later candidates.
            env: env.unwrap_or_else(|| self.rollback_env().clone()),
            work: self.work,
            replay_work: self.replay,
            levels: self.levels,
            clause_state_read: self.clause_state_read,
            failure: None,
        }
    }
    fn absorb(&mut self, r: &Result<'t, D>) {
        self.work = self.work.saturating_add(r.work);
        self.replay = self.replay.saturating_add(r.replay_work);
        self.levels = self.levels.max(r.levels);
        self.clause_state_read |= r.clause_state_read;
    }
    fn absorb_failure(&mut self, r: &Result<'t, D>) {
        self.absorb(r);
        // A utility body reports its peak relative to the entered utility.
        if matches!(self.state, State::Util { .. }) {
            self.levels = self.levels.max(1 + r.levels);
        }
    }
    fn child(
        &self,
        program: u32,
        node: Node<'t, D>,
        env: Arc<MetaVarEnv<'t, D>>,
        suspend: bool,
    ) -> PendingChild<'t, D> {
        PendingChild {
            request: Input {
                program,
                node,
                env,
                three: self.request.three,
                each: if suspend {
                    None
                } else {
                    self.request.each.clone()
                },
            },
            depth: self.depth,
        }
    }
}
struct Lane<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    meta: Option<LaneMeta>,
    stack: Vec<Frame<'t, D>>,
    returned: Option<Result<'t, D>>,
    done: Option<Result<'t, D>>,
}
struct PendingChild<'t, D: Doc<Source: Content<Underlying = u8>>> {
    request: Input<'t, D>,
    depth: u32,
}
enum Action<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    // Construct the continuation at its destination. Returning a complete frame
    // made every action carry and copy unused state and accounting fields.
    Push(PendingChild<'t, D>),
    Continue,
    Inline,
    Finish(Result<'t, D>),
    Leaf,
}
fn excluded<'t, D: Doc<Source: Content<Underlying = u8>>, H: Host<'t, D>>(
    host: &mut H,
    context: u32,
    each: Option<&Exclusion>,
    env: &MetaVarEnv<'t, D>,
) -> bool {
    each.is_some_and(|e| {
        host.binding_range(context, env, &e.name)
            .is_some_and(|range| e.ranges.contains(&(range.start, range.end)))
    })
}
fn walk_search<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
    plan: Walk,
    node: &Node<'t, D>,
    counter: Option<Counter>,
) -> Search<'t, D> {
    let mut queue = VecDeque::new();
    let mut linear = None;
    let neighbor = matches!(plan.stop, Stop::Neighbor);
    if plan.axis == 1 {
        if let Some(field) = plan.field {
            if let Some(child) = node.child_by_field_id(field) {
                queue.push_back(Visit {
                    node: child,
                    eligible: true,
                    descend: if neighbor {
                        0
                    } else if matches!(plan.stop, Stop::Rule(_)) {
                        1
                    } else {
                        2
                    },
                    stop: !neighbor,
                });
            }
        } else {
            for child in node.children() {
                queue.push_back(Visit {
                    node: child,
                    eligible: true,
                    descend: if neighbor { 0 } else { 2 },
                    stop: !neighbor,
                });
            }
        }
    } else {
        // Neighbor uses the document's byte-positioned next/prev operation;
        // longer walks use its native all() anchor only for the first node.
        let next = match plan.axis {
            0 => node.parent(),
            2 if neighbor => node.next(),
            2 => node.next_all().next(),
            _ if neighbor => node.prev(),
            _ => node.prev_all().next(),
        };
        linear = Some(LinearCursor {
            next,
            previous: node.node_id(),
        });
    }
    Search {
        plan,
        queue,
        linear,
        current: None,
        phase: 0,
        stopped: 0,
        unknown: false,
        counter,
        pending_open: false,
        rollback_input: None,
    }
}
fn position(step: i32, offset: i32, index: usize) -> bool {
    let Some(i) = i64::try_from(index).ok().and_then(|i| i.checked_add(1)) else {
        return false;
    };
    let (a, b) = (i64::from(step), i64::from(offset));
    if a == 0 {
        i == b
    } else {
        (i - b) / a >= 0 && (i - b) % a == 0
    }
}
fn work_failure<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
    H: Host<'t, D>,
>(
    f: &mut Frame<'t, D>,
    context: u32,
    host: &mut H,
) -> Result<'t, D> {
    let failure = host
        .failure(context)
        .unwrap_or(Failure::Utility(UtilLimit::Work));
    let mut r = f.result(0, None);
    r.failure = Some(failure);
    r
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Bank<'t, D> {
    fn fingerprint(
        &mut self,
        env: &Arc<MetaVarEnv<'t, D>>,
        owner: u64,
        program: u32,
        scope_id: u32,
        scope: Option<&[Arc<str>]>,
        excluded_name: Option<&str>,
    ) -> Arc<CaptureFingerprint> {
        if env.get_matched_variables().next().is_none() {
            self.metrics.fingerprint_hits += 1;
            return self.empty_fingerprint.clone();
        }
        if scope.is_some_and(|names| names.is_empty()) && excluded_name.is_none() {
            // An explicit empty scope selects no ordinary or reserved binding.
            // Count only nonempty environments; empty inputs already share this value.
            self.metrics.fingerprint_hits += 1;
            self.metrics.fingerprint_empty_scope_hits += 1;
            return self.empty_fingerprint.clone();
        }
        let key = (Arc::as_ptr(env) as usize, owner, scope_id);
        if let Some((weak, name, fingerprint, first_program)) = self.fingerprints.get(&key)
            && name.as_deref() == excluded_name
            && let Some(held) = weak.upgrade()
            && Arc::ptr_eq(&held, env)
        {
            self.metrics.fingerprint_hits += 1;
            self.metrics.fingerprint_cross_program_hits += usize::from(*first_program != program);
            return fingerprint.clone();
        }
        self.metrics.fingerprint_builds += 1;
        let extended = scope
            .zip(excluded_name)
            .filter(|(scope, name)| !scope.iter().any(|n| n.as_ref() == *name))
            .map(|(scope, name)| {
                let mut scope = scope.to_vec();
                scope.push(Arc::from(name));
                scope
            });
        let fingerprint = Arc::new(capture_fingerprint(env, extended.as_deref().or(scope)));
        if self.fingerprints.len() >= 32768 {
            self.fingerprints.clear();
        }
        self.fingerprints.insert(
            key,
            (
                Arc::downgrade(env),
                excluded_name.map(Arc::from),
                fingerprint.clone(),
                program,
            ),
        );
        fingerprint
    }
    pub fn execute<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        requests: Vec<Request<'t, D>>,
        host: &mut H,
    ) -> Vec<Result<'t, D>> {
        self.execute_at(programs, requests, host, 0)
    }
    pub fn execute_at<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        requests: Vec<Request<'t, D>>,
        host: &mut H,
        utility_depth: u32,
    ) -> Vec<Result<'t, D>> {
        let width = requests.len().max(1);
        let mut source = VectorSource {
            answers: (0..requests.len()).map(|_| None).collect(),
            pending: requests.into_iter().enumerate(),
        };
        let counters = if width <= 64 {
            CounterStorage::Sparse
        } else {
            CounterStorage::Dense
        };
        self.execute_source_counted(programs, &mut source, host, width, utility_depth, counters);
        source
            .answers
            .into_iter()
            .map(|answer| answer.expect("completed root"))
            .collect()
    }
    pub fn execute_source<H: Host<'t, D>, S: Source<'t, D, H>>(
        &mut self,
        programs: &Programs,
        source: &mut S,
        host: &mut H,
        width: usize,
        utility_depth: u32,
    ) {
        self.execute_source_counted(
            programs,
            source,
            host,
            width,
            utility_depth,
            CounterStorage::Dense,
        );
    }
    fn execute_source_counted<H: Host<'t, D>, S: Source<'t, D, H>>(
        &mut self,
        programs: &Programs,
        source: &mut S,
        host: &mut H,
        width: usize,
        utility_depth: u32,
        counters: CounterStorage,
    ) {
        let width = width.max(1);
        self.program_advances
            .prepare(counters, programs.entries.len());
        self.metrics.max_program_pool_entries = self
            .metrics
            .max_program_pool_entries
            .max(programs.entries.len());
        match counters {
            CounterStorage::Dense => self.metrics.dense_counter_submissions += 1,
            CounterStorage::Sparse => self.metrics.sparse_counter_submissions += 1,
        }
        self.metrics.absorbed_kind_stages = self
            .metrics
            .absorbed_kind_stages
            .max(programs.absorbed_kind_stages);
        self.metrics.submissions += 1;
        self.metrics.max_fingerprint_scope_programs = self
            .metrics
            .max_fingerprint_scope_programs
            .max(programs.fingerprint_scope_programs);
        self.metrics.max_fingerprint_scopes = self
            .metrics
            .max_fingerprint_scopes
            .max(programs.fingerprint_scopes.len());
        let mut slots: Vec<Option<(S::Token, Input<'t, D>)>> = (0..width).map(|_| None).collect();
        let mut lanes: Vec<Lane<'t, D>> = (0..width)
            .map(|_| Lane {
                meta: None,
                stack: Vec::new(),
                returned: None,
                done: None,
            })
            .collect();
        let mut free: Vec<_> = (0..width).rev().collect();
        let mut root_contexts = vec![None; width];
        let mut waiting = FxHashMap::<u32, VecDeque<usize>>::default();
        let mut active = Vec::new();
        let mut completed = Vec::new();
        let mut documents = FxHashMap::<usize, usize>::default();
        let mut programs_seen = FxHashMap::<u32, usize>::default();
        let mut multi_document = false;
        let mut multi_program = false;
        let mut refill = true;
        let mut admitted = 0;
        let mut families: [Vec<usize>; 7] = std::array::from_fn(|_| Vec::new());
        let mut ready: [Vec<usize>; 20] = std::array::from_fn(|_| Vec::new());
        let mut failures: FxHashMap<u32, Option<Failure>> = FxHashMap::default();
        let mut regions = Regions {
            executor: wave::Executor::new(&programs.closed, Vec::new()),
            identities: FxHashMap::default(),
            owners: Vec::new(),
        };
        loop {
            if refill && (free.len() >= (width / 2).max(1) || active.is_empty()) {
                refill = false;
                while let Some(&index) = free.last() {
                    let Some((token, request)) = source.next(host) else {
                        break;
                    };
                    free.pop();
                    let regions_enabled = request.each.is_none()
                        && host.can_remember(request.context, &request.node)
                        && (request.node.get_doc() as &dyn std::any::Any)
                            .is::<crate::code_rule_dsl::tape::TapeDoc>();
                    let ordered = if regions_enabled {
                        programs.requires_order_with_regions(request.program, true)
                    } else {
                        programs.requires_order(request.program)
                    };
                    let context = ordered.then_some(request.context);
                    root_contexts[index] = context;
                    if let Some(context) = context {
                        let queue = waiting.entry(context).or_default();
                        if queue.is_empty() {
                            active.push(index);
                        }
                        queue.push_back(index);
                    } else {
                        active.push(index);
                    }
                    *documents
                        .entry(request.node.get_doc() as *const D as usize)
                        .or_default() += 1;
                    *programs_seen.entry(request.program).or_default() += 1;
                    let (input, meta) = request.split();
                    lanes[index].meta = Some(meta);
                    lanes[index]
                        .stack
                        .push(Frame::new(input.clone(), utility_depth));
                    slots[index] = Some((token, input));
                    self.metrics.source_refills += usize::from(admitted >= width);
                    admitted += 1;
                    self.metrics.submitted_roots += 1;
                    let resident = width - free.len();
                    self.metrics.peak_resident_roots =
                        self.metrics.peak_resident_roots.max(resident);
                    self.metrics.max_submission_width =
                        self.metrics.max_submission_width.max(resident);
                    self.metrics.max_documents = self.metrics.max_documents.max(documents.len());
                    self.metrics.max_programs = self.metrics.max_programs.max(programs_seen.len());
                    if documents.len() > 1 && !multi_document {
                        multi_document = true;
                        self.metrics.multi_document_submissions += 1;
                    }
                    if programs_seen.len() > 1 && !multi_program {
                        multi_program = true;
                        self.metrics.multi_program_submissions += 1;
                    }
                }
            }
            if active.is_empty() {
                break;
            }
            failures.clear();
            // Compact the frontier once: completed lanes are never scanned
            // again while a deeper relation or utility continues elsewhere.
            active.retain(|&i| {
                let lane = &mut lanes[i];
                let Some(frame) = lane.stack.last() else {
                    lane.done = lane.returned.take();
                    completed.push(i);
                    return false;
                };
                if lane.returned.as_ref().is_none_or(|r| r.failure.is_none()) {
                    let failure = *failures
                        .entry(lane.meta.as_ref().unwrap().context)
                        .or_insert_with(|| host.failure(lane.meta.as_ref().unwrap().context));
                    if let Some(failure) = failure {
                        let frame = lane.stack.last_mut().unwrap();
                        if let Some(r) = lane.returned.take() {
                            frame.absorb_failure(&r);
                        }
                        let mut r = frame.result(0, None);
                        r.failure = Some(failure);
                        self.metrics.singleton_all_completions += usize::from(
                            programs.entries[frame.request.program as usize].rollback_nontrue,
                        );
                        lane.stack.pop();
                        lane.returned = Some(r);
                        return true;
                    }
                }
                let guard_rejected = matches!(frame.state, State::Start)
                    && programs.atomic(frame.request.program).is_none()
                    && self.reject_kind_guard(
                        programs,
                        frame.request.program,
                        frame.request.leaf(lane.meta.as_ref().unwrap(), frame.depth),
                        host,
                    );
                if guard_rejected {
                    let frame = lane.stack.last().unwrap();
                    let answer = frame.result(0, None);
                    if let Some(family) =
                        self.chain_control(programs, lane, Action::Finish(answer), host)
                    {
                        families[family].push(i);
                    }
                    if lane.stack.is_empty() {
                        lane.done = lane.returned.take();
                        completed.push(i);
                        return false;
                    }
                    return true;
                }
                let frame = lane.stack.last().unwrap();
                let operation = frame.control_family(programs);
                ready[operation as usize].push(i);
                true
            });
            self.dispatch_pipelines(programs, &mut lanes, &mut ready, &mut families, host);
            self.dispatch_regions(
                programs,
                &mut lanes,
                &mut ready,
                &mut families,
                host,
                &mut regions,
            );
            // Continuations, navigation and capture-frame operations run in
            // opcode cohorts, independently of program and document identity.
            for (operation, group) in ready.iter_mut().enumerate() {
                if group.is_empty() {
                    continue;
                }
                self.metrics.operation_dispatches += 1;
                self.metrics.operation_lanes += group.len();
                self.metrics.operation_family_lanes[operation] += group.len();
                self.metrics.operation_family_dispatches[operation] += 1;

                self.metrics.max_width = self.metrics.max_width.max(group.len());
                let mut projected = Vec::new();
                if operation == 4 {
                    let mut jobs = Vec::new();
                    let mut positions = Vec::new();
                    for (position, &i) in group.iter().enumerate() {
                        let f = lanes[i].stack.last().unwrap();
                        let Op::Walk(w) = &programs.entries[f.request.program as usize].op else {
                            unreachable!()
                        };
                        if w.field.is_none()
                            && matches!(w.stop, Stop::End)
                            && ((w.axis == 1 && !f.request.three && w.filter.is_some())
                                || (matches!(w.axis, 2 | 3)
                                    && w.related.as_ref().is_some_and(|r| r.has_same_key_plan())))
                        {
                            positions.push(position);
                            jobs.push(NavigationRequest {
                                request: f
                                    .request
                                    .leaf(lanes[i].meta.as_ref().unwrap(), f.depth)
                                    .to_request(),
                                utility_depth: f.depth,
                                related: w.related.clone(),
                                filter: w.filter.clone(),
                                descendants: w.axis == 1,
                                forward: match w.axis {
                                    2 => Some(true),
                                    3 => Some(false),
                                    _ => None,
                                },
                            });
                        }
                    }
                    if !jobs.is_empty() {
                        self.metrics.navigation_batches += 1;
                        self.metrics.navigation_lanes += jobs.len();
                        self.metrics.owned_leaf_requests += jobs.len();
                        let results = host.navigate(programs, jobs);
                        assert_eq!(
                            results.len(),
                            positions.len(),
                            "Host must return one answer per navigation request"
                        );
                        projected = positions.into_iter().zip(results).collect();
                    }
                }
                let mut projected = projected.into_iter().peekable();
                for (position, i) in group.drain(..).enumerate() {
                    if projected.peek().is_some_and(|(p, _)| *p == position) {
                        let (_, nav) = projected.next().unwrap();
                        let f = lanes[i].stack.last_mut().unwrap();
                        f.work = f.work.saturating_add(nav.work);
                        f.replay = f.replay.saturating_add(nav.replay_work);
                        f.levels = f.levels.max(nav.levels);
                        f.clause_state_read |= nav.clause_state_read;
                        if let Some(failure) = nav.failure {
                            let mut r = f.result(0, None);
                            r.failure = Some(failure);
                            self.metrics.singleton_all_completions += usize::from(
                                programs.entries[f.request.program as usize].rollback_nontrue,
                            );
                            lanes[i].stack.pop();
                            lanes[i].returned = Some(r);
                            continue;
                        }
                        if let Some(nodes) = nav.nodes {
                            self.metrics.navigation_projected_lanes += 1;
                            self.metrics.navigation_projected_nodes += nodes.len();
                            let Op::Walk(plan) = &programs.entries[f.request.program as usize].op
                            else {
                                unreachable!()
                            };
                            f.state = State::Search(Box::new(Search {
                                plan: plan.clone(),
                                queue: nodes
                                    .into_iter()
                                    .map(|node| Visit {
                                        node,
                                        eligible: true,
                                        descend: 0,
                                        stop: false,
                                    })
                                    .collect(),
                                linear: None,
                                current: None,
                                phase: 0,
                                stopped: 0,
                                unknown: false,
                                counter: None,
                                pending_open: false,
                                rollback_input: programs.entries[f.request.program as usize]
                                    .rollback_nontrue
                                    .then(|| f.request.env.clone()),
                            }));
                            continue;
                        }
                    }

                    if operation == 8 {
                        // Atomic leaves stay in their lane's stack. Group only
                        // lane IDs, avoiding repeated moves of capture frames.
                        let frame = lanes[i].stack.last().unwrap();
                        let (program, _) = frame.leaf_parts(programs);
                        let family = programs.atomic(program).expect("atomic descriptor");
                        families[family].push(i);
                        continue;
                    }
                    let lane = &mut lanes[i];
                    let returned = lane.returned.take();
                    let frame = lane.stack.last_mut().unwrap();
                    let action = self.advance(
                        programs,
                        frame,
                        returned,
                        operation,
                        lane.meta.as_ref().unwrap(),
                        host,
                    );
                    if let Some(family) = self.chain_control(programs, lane, action, host) {
                        families[family].push(i);
                    }
                }
            }
            for (atomic_family, indices) in families.iter_mut().enumerate() {
                if indices.is_empty() {
                    continue;
                }
                let jobs = indices
                    .iter()
                    .map(|&i| {
                        let frame = lanes[i].stack.last().unwrap();
                        frame.leaf(programs, lanes[i].meta.as_ref().unwrap())
                    })
                    .collect();
                let results = self.atomic_batch(programs, atomic_family, jobs, host);
                for (j, mut r) in indices.drain(..).zip(results) {
                    let f = lanes[j].stack.last().unwrap();
                    if r.truth == 1 && !f.request.three {
                        r.truth = 0;
                    }
                    if r.truth != 2 {
                        r.env = f.leaf_parts(programs).1.clone();
                    }
                    let composite = matches!(f.state, State::List { atomic: true, .. });
                    if !composite {
                        r.work = r.work.saturating_add(f.work);
                        r.replay_work = r.replay_work.saturating_add(f.replay);
                        r.levels = r.levels.max(f.levels);
                        r.clause_state_read |= f.clause_state_read;
                    }
                    let lane = &mut lanes[j];
                    let action = if composite {
                        self.advance(
                            programs,
                            lane.stack.last_mut().unwrap(),
                            Some(r),
                            8,
                            lane.meta.as_ref().unwrap(),
                            host,
                        )
                    } else {
                        Action::Finish(r)
                    };
                    // The next primitive waits for the next outer round so all
                    // lanes join its family before that batch executes.
                    self.chain_control(programs, lane, action, host);
                }
            }
            for index in completed.drain(..) {
                if let Some(context) = root_contexts[index].take() {
                    let queue = waiting.get_mut(&context).expect("root context");
                    queue.pop_front();
                    if let Some(&next) = queue.front() {
                        active.push(next);
                    } else {
                        waiting.remove(&context);
                    }
                }
                let (token, request) = slots[index].take().expect("resident root");
                let doc = request.node.get_doc() as *const D as usize;
                let count = documents.get_mut(&doc).unwrap();
                *count -= 1;
                if *count == 0 {
                    documents.remove(&doc);
                }
                let count = programs_seen.get_mut(&request.program).unwrap();
                *count -= 1;
                if *count == 0 {
                    programs_seen.remove(&request.program);
                }
                let answer = lanes[index].done.take().expect("completed lane");
                let meta = lanes[index].meta.take().expect("resident metadata");
                source.completed(token, request.into_request(meta), answer, host);
                free.push(index);
                refill = true;
            }
        }
        let counts = self.program_advances.drain();
        self.metrics.max_control_programs_touched =
            self.metrics.max_control_programs_touched.max(counts.len());
        for (program, count) in counts {
            *self
                .metrics
                .control_programs
                .entry(programs.control_descriptor(program))
                .or_default() += count;
        }
        let metrics = &regions.executor.metrics;
        self.metrics.region_kernel_lanes += metrics.kernel_lanes;
        self.metrics.region_kernel_dispatches += metrics.kernel_dispatches;
        self.metrics.region_posting_builds += metrics.posting_builds;
        self.metrics.region_posting_nodes += metrics.posting_nodes;
        merge_region_metrics(&mut self.metrics.region_kernel, metrics);
    }
    fn reject_kind_guard<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        program: u32,
        request: LeafJob<'_, 't, D>,
        host: &mut H,
    ) -> bool {
        let Some(guard) = &programs.entries[program as usize].guard else {
            return false;
        };
        let rejected = guard
            .kinds
            .as_ref()
            .is_some_and(|kinds| kinds.binary_search(&request.node.kind_id()).is_err())
            || (guard.requires_unexcluded
                && excluded(
                    host,
                    request.meta.context,
                    request.each.map(|e| e.as_ref()),
                    request.env,
                ));
        self.metrics.kind_guard_checks += 1;
        if rejected {
            self.metrics.kind_guard_rejects += 1;
        }
        rejected
    }
    fn skip_guarded_alternatives<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        frame: &Frame<'t, D>,
        children: &[u32],
        index: &mut usize,
        meta: &LaneMeta,
        host: &mut H,
    ) {
        while *index < children.len() {
            let program = children[*index];
            if !self.reject_kind_guard(
                programs,
                program,
                frame.request.leaf(meta, frame.depth),
                host,
            ) {
                break;
            }
            self.metrics.kind_guard_alternative_skips += 1;

            *index += 1;
        }
    }
    fn atomic_batch<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        family: usize,
        jobs: Vec<LeafJob<'_, 't, D>>,
        host: &mut H,
    ) -> Vec<Result<'t, D>> {
        self.metrics.borrowed_leaf_lanes += jobs.len();
        if !jobs
            .iter()
            .any(|request| programs.entries[request.program as usize].guard.is_some())
        {
            return self.atomic_batch_unguarded(programs, family, jobs, host);
        }
        let mut output: Vec<_> = (0..jobs.len()).map(|_| None).collect();
        let mut accepted = Vec::new();
        let mut positions = Vec::new();
        for (position, request) in jobs.into_iter().enumerate() {
            if self.reject_kind_guard(programs, request.program, request, host) {
                output[position] = Some(Result::new(0, request.env.clone()));
            } else {
                positions.push(position);
                accepted.push(request);
            }
        }
        for (position, answer) in positions
            .into_iter()
            .zip(self.atomic_batch_unguarded(programs, family, accepted, host))
        {
            output[position] = Some(answer);
        }
        output.into_iter().map(Option::unwrap).collect()
    }
    fn atomic_batch_unguarded<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        family: usize,
        jobs: Vec<LeafJob<'_, 't, D>>,
        host: &mut H,
    ) -> Vec<Result<'t, D>> {
        if jobs.is_empty() {
            return Vec::new();
        }
        self.metrics.operation_dispatches += 1;
        self.metrics.operation_lanes += jobs.len();
        self.metrics.atomic_family_lanes[family] += jobs.len();
        self.metrics.atomic_family_dispatches[family] += 1;
        self.metrics.max_width = self.metrics.max_width.max(jobs.len());
        let results = if family == 0 {
            let requests = jobs
                .iter()
                .map(|r| {
                    let Op::Pattern(pattern) = &programs.entries[r.program as usize].op else {
                        unreachable!()
                    };
                    pattern_batch::Request {
                        pattern: pattern.clone(),
                        node: r.node.clone(),
                        env: r.env.clone(),
                    }
                })
                .collect();
            pattern_batch::execute_cached(
                requests,
                &mut self.pattern_cache,
                &mut self.metrics.pattern,
            )
            .into_iter()
            .zip(&jobs)
            .map(|(env, r)| {
                Result::new(
                    if env.is_some() { 2 } else { 0 },
                    env.unwrap_or_else(|| r.env.clone()),
                )
            })
            .collect()
        } else if family == 4 {
            self.metrics.owned_leaf_requests += jobs.len();
            host.compare(
                programs,
                jobs.iter()
                    .map(|r| {
                        let Op::Compare(comparison) = &programs.entries[r.program as usize].op
                        else {
                            unreachable!()
                        };
                        CompareRequest {
                            comparison: comparison.clone(),
                            request: r.to_request(),
                            utility_depth: r.depth,
                        }
                    })
                    .collect(),
            )
        } else if family == 5 {
            self.metrics.owned_leaf_requests += jobs.len();
            host.evaluate(
                programs,
                jobs.iter()
                    .map(|r| {
                        let Op::Semantic(semantic) = &programs.entries[r.program as usize].op
                        else {
                            unreachable!()
                        };
                        SemanticRequest {
                            semantic: semantic.clone(),
                            request: r.to_request(),
                            utility_depth: r.depth,
                        }
                    })
                    .collect(),
            )
        } else {
            jobs.iter()
                .map(|r| {
                    let mut env = r.env.clone();
                    let truth = match &programs.entries[r.program as usize].op {
                        Op::Kind(kind) => {
                            if r.node.kind_id() == *kind {
                                2
                            } else {
                                0
                            }
                        }
                        Op::Regex(regex) => {
                            if regex.is_match(&r.node.text()) {
                                2
                            } else {
                                0
                            }
                        }
                        Op::Capture(name) => {
                            let mut trial = (*env).clone();
                            if trial.insert(name, r.node.clone()).is_some() {
                                env = Arc::new(trial);
                                2
                            } else {
                                0
                            }
                        }
                        Op::Pass => 2,
                        Op::Unexcluded => {
                            if excluded(host, r.meta.context, r.each.map(|e| e.as_ref()), &env) {
                                0
                            } else {
                                2
                            }
                        }
                        Op::Never => 0,
                        #[cfg(test)]
                        Op::TestUnknown(regex) => {
                            if regex.is_match(&r.node.text()) {
                                1
                            } else {
                                2
                            }
                        }
                        _ => unreachable!("atomic family"),
                    };
                    Result::new(truth, env)
                })
                .collect()
        };
        assert_eq!(
            results.len(),
            jobs.len(),
            "Host must return one result per atomic request"
        );
        results
            .into_iter()
            .zip(jobs)
            .map(|(mut answer, request)| {
                if answer.failure.is_some() || (answer.truth == 1 && !request.meta.three) {
                    answer.truth = 0;
                }
                if answer.truth != 2 {
                    answer.env = request.env.clone();
                }
                answer
            })
            .collect()
    }
    fn dispatch_pipelines<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        lanes: &mut [Lane<'t, D>],
        ready: &mut [Vec<usize>; 20],
        families: &mut [Vec<usize>; 7],
        host: &mut H,
    ) {
        let positions: Vec<_> = ready[3]
            .iter()
            .copied()
            .filter(|&i| {
                let frame = lanes[i].stack.last().unwrap();
                matches!(frame.state, State::Start)
                    && programs.entries[frame.request.program as usize].pipeline
            })
            .collect();
        if positions.is_empty() {
            return;
        }
        self.metrics.pipeline_rows += positions.len();
        let mut finished = Vec::new();
        let jobs = positions
            .iter()
            .map(|&i| {
                let frame = lanes[i].stack.last().unwrap();
                let Op::Constrained { rule, .. } =
                    programs.entries[frame.request.program as usize].op
                else {
                    unreachable!()
                };
                LeafJob {
                    program: rule,
                    ..frame
                        .request
                        .leaf(lanes[i].meta.as_ref().unwrap(), frame.depth)
                }
            })
            .collect();
        self.metrics.pipeline_eliminated_frames += positions.len();
        let answers = self.atomic_batch(programs, 0, jobs, host);
        let mut rows = Vec::new();
        for (i, answer) in positions.iter().copied().zip(answers) {
            let frame = lanes[i].stack.last_mut().unwrap();
            frame.absorb(&answer);
            if answer.failure.is_some() || answer.truth != 2 {
                let mut result = frame.result(
                    if answer.failure.is_some() {
                        0
                    } else {
                        answer.truth
                    },
                    None,
                );
                result.failure = answer.failure;
                finished.push((i, result));
            } else {
                rows.push((i, answer.env, 0usize));
            }
        }
        while !rows.is_empty() {
            let mut keep = vec![false; rows.len()];
            // Plan each row before borrowing captures and root metadata for kernels.
            for (position, (i, trial, index)) in rows.iter_mut().enumerate() {
                let context = lanes[*i].meta.as_ref().unwrap().context;
                let frame = lanes[*i].stack.last_mut().unwrap();
                if let Some(failure) = host.failure(context) {
                    let mut result = frame.result(0, None);
                    result.failure = Some(failure);
                    finished.push((*i, result));
                    continue;
                }
                let Op::Constrained { constraints, .. } =
                    &programs.entries[frame.request.program as usize].op
                else {
                    unreachable!()
                };
                while *index < constraints.len()
                    && trial.get_match(&constraints[*index].0).is_none()
                {
                    *index += 1;
                }
                if *index == constraints.len() {
                    finished.push((*i, frame.result(2, Some(trial.clone()))));
                    continue;
                }
                keep[position] = true;
            }
            let mut jobs: [Vec<LeafJob<'_, 't, D>>; 7] = std::array::from_fn(|_| Vec::new());
            let mut destinations: [Vec<usize>; 7] = std::array::from_fn(|_| Vec::new());
            for (position, (i, trial, index)) in rows.iter().enumerate() {
                if !keep[position] {
                    continue;
                }
                let frame = lanes[*i].stack.last().unwrap();
                let Op::Constrained { constraints, .. } =
                    &programs.entries[frame.request.program as usize].op
                else {
                    unreachable!()
                };
                let program = constraints[*index].1;
                let family = programs.atomic(program).unwrap();
                jobs[family].push(LeafJob {
                    program,
                    node: trial.get_match(&constraints[*index].0).unwrap(),
                    env: trial,
                    each: frame.request.each.as_ref(),
                    meta: lanes[*i].meta.as_ref().unwrap(),
                    depth: frame.depth,
                });
                destinations[family].push(position);
            }
            keep.fill(false);
            let mut answers: [Vec<Result<'t, D>>; 7] = std::array::from_fn(|_| Vec::new());
            for family in 0..7 {
                self.metrics.pipeline_eliminated_frames += jobs[family].len();
                answers[family] =
                    self.atomic_batch(programs, family, std::mem::take(&mut jobs[family]), host);
            }
            drop(jobs);
            // No borrowed job survives when trial environments and frame costs change.
            for family in 0..7 {
                for (position, answer) in destinations[family]
                    .drain(..)
                    .zip(std::mem::take(&mut answers[family]))
                {
                    let (i, trial, index) = &mut rows[position];
                    let frame = lanes[*i].stack.last_mut().unwrap();
                    frame.absorb(&answer);
                    if answer.failure.is_some() || answer.truth != 2 {
                        let mut result = frame.result(
                            if answer.failure.is_some() {
                                0
                            } else {
                                answer.truth
                            },
                            None,
                        );
                        result.failure = answer.failure;
                        finished.push((*i, result));
                    } else {
                        *trial = answer.env;
                        *index += 1;
                        keep[position] = true;
                    }
                }
            }
            let mut position = 0;
            rows.retain(|_| {
                let retain = keep[position];
                position += 1;
                retain
            });
        }
        let handled: rustc_hash::FxHashSet<_> = positions.into_iter().collect();
        ready[3].retain(|i| !handled.contains(i));
        for (i, mut result) in finished {
            if result.failure.is_none() {
                if let Some(failure) = host.failure(lanes[i].meta.as_ref().unwrap().context) {
                    result.truth = 0;
                    result.failure = Some(failure);
                    result.env = lanes[i].stack.last().unwrap().request.env.clone();
                }
            }
            if let Some(family) =
                self.chain_control(programs, &mut lanes[i], Action::Finish(result), host)
            {
                families[family].push(i);
            }
        }
    }
    fn dispatch_regions<'a, H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        lanes: &mut [Lane<'t, D>],
        ready: &mut [Vec<usize>; 20],
        families: &mut [Vec<usize>; 7],
        host: &mut H,
        regions: &mut Regions<'a>,
    ) where
        't: 'a,
    {
        use crate::code_rule_dsl::tape::TapeDoc;
        use std::any::Any;
        let mut positions = Vec::new();
        for group in ready.iter() {
            for &i in group {
                let lane = &mut lanes[i];
                let context = lane.meta.as_ref().unwrap().context;
                let frame = lane.stack.last_mut().unwrap();
                if !matches!(frame.state, State::Start) || frame.region_attempted {
                    continue;
                }
                let Some(closed) = programs.entries[frame.request.program as usize].closed else {
                    continue;
                };
                frame.region_attempted = true;
                if frame.request.each.is_none()
                    && host.can_remember(context, &frame.request.node)
                    && (frame.request.node.get_doc() as &dyn Any).is::<TapeDoc>()
                {
                    positions.push((i, closed));
                }
            }
        }
        if positions.is_empty() {
            return;
        }
        self.metrics.region_requests += positions.len();
        let mut requests = Vec::with_capacity(positions.len());
        for &(i, program) in &positions {
            let frame = lanes[i].stack.last().unwrap();
            let doc: &'t TapeDoc = (frame.request.node.get_doc() as &dyn Any)
                .downcast_ref::<TapeDoc>()
                .unwrap();
            let document = *regions
                .identities
                .entry(doc as *const TapeDoc as usize)
                .or_insert_with(|| {
                    let id = regions.executor.register_document(doc);
                    regions.owners.push(lanes[i].meta.as_ref().unwrap().context);
                    id
                });
            requests.push(wave::Lane {
                program,
                document,
                node: frame.request.node.node_id() as u32,
                flags: u32::from(frame.request.three),
            });
        }
        let cancelled = |document: u32| host.region_cancelled(regions.owners[document as usize]);
        let mut answers = Vec::with_capacity(requests.len());
        for tile in requests.chunks(wave::TILE) {
            answers.extend(regions.executor.execute_cancellable(tile, &cancelled));
        }
        let mut accepted = rustc_hash::FxHashSet::default();
        for ((i, _), answer) in positions.into_iter().zip(answers) {
            let context = lanes[i].meta.as_ref().unwrap().context;
            let frame = lanes[i].stack.last_mut().unwrap();
            let failure = host.failure(context);
            if failure.is_none()
                && (answer.incomplete
                    || frame.depth.saturating_add(answer.levels) > super::util::MAX_UTIL_DEPTH
                    || answer.steps > host.region_allowance(context))
            {
                self.metrics.region_fallbacks += 1;
                continue;
            }
            let mut result = frame.result(if answer.matched { 2 } else { 0 }, None);
            if let Some(failure) = failure {
                result.truth = 0;
                result.failure = Some(failure);
            } else {
                if !host.spend(context, answer.steps) {
                    // A concurrent cancellation can become visible at debit.
                    result.truth = 0;
                    result.failure = host
                        .failure(context)
                        .or(Some(Failure::Utility(UtilLimit::Work)));
                }
                result.work = result.work.saturating_add(answer.steps);
                result.replay_work = result.replay_work.saturating_add(answer.steps);
                result.levels = result.levels.max(answer.levels);
                self.metrics.region_steps += answer.steps;
            }
            self.metrics.region_roots += 1;
            accepted.insert(i);
            if let Some(family) =
                self.chain_control(programs, &mut lanes[i], Action::Finish(result), host)
            {
                families[family].push(i);
            }
        }
        for group in ready {
            group.retain(|i| !accepted.contains(i));
        }
    }
    fn chain_control<H: Host<'t, D>>(
        &mut self,
        programs: &Programs,
        lane: &mut Lane<'t, D>,
        mut action: Action<'t, D>,
        host: &mut H,
    ) -> Option<usize> {
        for steps in 0..=64 {
            match action {
                Action::Push(child) => lane.stack.push(Frame::new(child.request, child.depth)),
                Action::Continue | Action::Inline => {}
                Action::Finish(mut answer) => {
                    let frame = lane.stack.last().unwrap();
                    if programs.entries[frame.request.program as usize].rollback_nontrue {
                        self.metrics.singleton_all_completions += 1;
                        if answer.truth != 2 || answer.failure.is_some() {
                            answer.env = frame.rollback_env().clone();
                            if answer.failure.is_some() {
                                answer.truth = 0;
                            }
                        }
                    }
                    lane.stack.pop();
                    lane.returned = Some(answer);
                }
                Action::Leaf => unreachable!("primitive reached control chain"),
            }
            let frame = lane.stack.last_mut()?;
            if matches!(frame.state, State::Start)
                && programs.atomic(frame.request.program).is_none()
                && programs.entries[frame.request.program as usize]
                    .guard
                    .is_some()
            {
                return None;
            }
            let primitive = matches!(frame.state, State::List { atomic: true, .. })
                || (matches!(frame.state, State::Start)
                    && programs.atomic(frame.request.program).is_some());
            if primitive {
                let (program, _) = frame.leaf_parts(programs);
                return programs.atomic(program);
            }
            // Closed regions and walk startup need their shared physical cohort.
            if (matches!(frame.state, State::Start)
                && (programs.entries[frame.request.program as usize].pipeline
                    || (!frame.region_attempted
                        && programs.entries[frame.request.program as usize]
                            .closed
                            .is_some())))
                || steps == 64
                || (matches!(frame.state, State::Start)
                    && matches!(
                        programs.entries[frame.request.program as usize].op,
                        Op::Walk(_)
                    ))
            {
                return None;
            }
            self.metrics.fused_control_steps += 1;
            let family = frame.control_family(programs);
            self.metrics.fused_control_family_lanes[family] += 1;
            action = self.advance(
                programs,
                frame,
                lane.returned.take(),
                family,
                lane.meta.as_ref().unwrap(),
                host,
            );
        }
        unreachable!("bounded control chain")
    }
    fn structural_sibling(&mut self, node: &Node<'t, D>, forward: bool) -> Option<Node<'t, D>> {
        let parent = node.parent()?;
        let key = (node.get_doc() as *const D as usize, parent.node_id());
        if !self.structural_siblings.contains_key(&key) {
            if self.structural_siblings.len() >= 32768 {
                self.structural_siblings.clear();
            }
            let nodes: Vec<_> = parent.children().collect();
            let positions = nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.node_id(), i))
                .collect();
            self.structural_siblings
                .insert(key, StructuralSiblings { nodes, positions });
        }
        let siblings = self.structural_siblings.get(&key)?;
        let &index = siblings.positions.get(&node.node_id())?;
        let next = if forward {
            index.checked_add(1)?
        } else {
            index.checked_sub(1)?
        };
        siblings.nodes.get(next).cloned()
    }
    fn remember_siblings(&mut self, key: SiblingKey, counted: Arc<Vec<usize>>, unknown: bool) {
        if self.siblings.len() >= 32768 {
            self.siblings.clear();
        }
        self.siblings.insert(key, (counted, unknown));
    }
    fn remember_closures(&mut self, key: ClosureKey, values: Vec<(usize, u8)>) {
        // Bound memory independently of utility memoization. Clearing only loses speed.
        if self.closures.len().saturating_add(values.len()) > 131072 {
            self.closures.clear();
        }
        for (node, truth) in values.into_iter().take(131072) {
            self.closures.insert(ClosureKey { node, ..key }, truth);
        }
    }
    fn advance_closure(
        &mut self,
        f: &mut Frame<'t, D>,
        mut s: Box<Closure<'t, D>>,
        returned: Option<Result<'t, D>>,
    ) -> Action<'t, D> {
        let tree = s.plan.axis == 1;
        let mut terminal = None;
        if tree
            && s.current.is_none()
            && s.walked.is_empty()
            && let Some(&known) = self.closures.get(&s.key)
        {
            self.metrics.closure_hits += 1;
            return Action::Finish(f.result(known, None));
        }
        if let Some(r) = returned {
            f.absorb(&r);
            // Failure is handled by advance before this continuation can publish.
            let node = s.current.take().expect("closure candidate");
            let id = node.node_id();
            let spent = r.work != 0;
            s.spent += usize::from(spent);
            let parent = node.parent().map_or(s.key.node, |p| p.node_id());
            let known = if tree && r.truth != 2 {
                self.closures
                    .get(&ClosureKey { node: id, ..s.key })
                    .copied()
            } else {
                None
            };
            if known.is_some() {
                self.metrics.closure_hits += 1;
            }
            s.walked.push((id, parent, r.truth, known, spent));
            if r.truth == 2 || known == Some(2) {
                terminal = Some(2);
            } else if tree && known.is_none() {
                s.open.push((id, s.spent));
                let children: Vec<_> = node.children().collect();
                for child in children.into_iter().rev() {
                    s.queue.push_front(child);
                }
            }
        }
        if terminal.is_none() {
            loop {
                let node = if tree {
                    s.queue.pop_front()
                } else {
                    s.cursor.take()
                };
                let Some(node) = node else { break };
                if !tree {
                    s.cursor = match s.plan.axis {
                        0 => node.parent(),
                        2 => self.structural_sibling(&node, true),
                        _ => self.structural_sibling(&node, false),
                    };
                }
                let id = node.node_id();
                if tree {
                    let parent = node.parent().map_or(s.key.node, |p| p.node_id());
                    while s.open.last().is_some_and(|&(id, _)| id != parent) {
                        s.open.pop();
                    }
                } else if let Some(&known) = self.closures.get(&ClosureKey { node: id, ..s.key }) {
                    self.metrics.closure_hits += 1;
                    terminal = Some(known);
                    break;
                }
                if !f.request.three
                    && s.plan
                        .filter
                        .as_ref()
                        .is_some_and(|filter| filter.kinds.binary_search(&node.kind_id()).is_err())
                {
                    s.walked.push((id, 0, 0, None, false));
                    continue;
                }
                self.metrics.closure_candidates += 1;
                let child = f.child(
                    s.plan.child,
                    node.clone(),
                    Arc::new(MetaVarEnv::new()),
                    true,
                );
                s.current = Some(node);
                f.state = State::Closure(s);
                return Action::Push(child);
            }
        }
        let mut remembered = Vec::new();
        let answer;
        if !tree {
            let mut value = terminal.unwrap_or(0);
            let mut clean = true;
            for &(id, _, own, _, spent) in s.walked.iter().rev() {
                value = value.max(own);
                clean &= !spent;
                if clean {
                    remembered.push((id, value));
                }
            }
            answer = value;
        } else if terminal == Some(2) {
            for &(id, before) in &s.open {
                if before == s.spent {
                    remembered.push((id, 2));
                }
            }
            if s.spent == 0 {
                remembered.push((s.key.node, 2));
            }
            answer = 2;
        } else {
            let mut below = FxHashMap::<usize, (u8, bool)>::default();
            let mut value = 0;
            let mut dirty = false;
            for &(id, parent, own, known, spent) in s.walked.iter().rev() {
                let (under, spent_under) = known
                    .map(|v| (v, false))
                    .unwrap_or_else(|| below.remove(&id).unwrap_or((0, false)));
                if known.is_none() && !spent_under {
                    remembered.push((id, under));
                }
                let verdict = own.max(under);
                let spent = spent || spent_under;
                if parent == s.key.node {
                    value = value.max(verdict);
                    dirty |= spent;
                } else {
                    let slot = below.entry(parent).or_insert((0, false));
                    slot.0 = slot.0.max(verdict);
                    slot.1 |= spent;
                }
            }
            if !dirty {
                remembered.push((s.key.node, value));
            }
            answer = value;
        }
        self.remember_closures(s.key, remembered);
        Action::Finish(f.result(answer, None))
    }
    fn advance<H: Host<'t, D>>(
        &mut self,
        p: &Programs,
        f: &mut Frame<'t, D>,
        returned: Option<Result<'t, D>>,
        family: usize,
        meta: &LaneMeta,
        host: &mut H,
    ) -> Action<'t, D> {
        self.metrics.control_family_lanes[family] += 1;
        self.program_advances.record(f.request.program);
        if let Some(r) = &returned {
            if r.failure.is_some() {
                f.absorb_failure(r);
                let mut out = f.result(0, None);
                out.failure = r.failure;
                return Action::Finish(out);
            }
        }
        let state = std::mem::replace(&mut f.state, State::Start);
        match state {
            State::Start => match &p.entries[f.request.program as usize].op {
                Op::All(children) | Op::Any(children) => {
                    let any = matches!(&p.entries[f.request.program as usize].op, Op::Any(_));
                    if children.is_empty() {
                        return Action::Finish(f.result(if any { 0 } else { 2 }, None));
                    }
                    let mut index = 0;
                    if any {
                        self.skip_guarded_alternatives(p, f, children, &mut index, meta, host);
                    }
                    if index == children.len() {
                        return Action::Finish(f.result(0, None));
                    }
                    let program = children[index];
                    let atomic = p.atomic(program).is_some();
                    f.state = State::List {
                        index,
                        atomic,
                        any,
                        trial: f.request.env.clone(),
                        unknown: false,
                        withheld: None,
                    };
                    if atomic {
                        Action::Inline
                    } else {
                        let child = f.child(
                            program,
                            f.request.node.clone(),
                            f.request.env.clone(),
                            false,
                        );
                        Action::Push(child)
                    }
                }
                Op::Not(child) => {
                    let child =
                        f.child(*child, f.request.node.clone(), f.request.env.clone(), true);
                    f.state = State::Not;
                    Action::Push(child)
                }
                Op::Constrained { rule, .. } => {
                    let child =
                        f.child(*rule, f.request.node.clone(), f.request.env.clone(), false);
                    f.state = State::Constraints {
                        index: 0,
                        first: true,
                    };
                    Action::Push(child)
                }
                Op::Walk(plan) => {
                    if plan.field.is_none()
                        && matches!(plan.stop, Stop::End)
                        && f.request.each.is_none()
                        && (plan.axis != 1 || f.request.three || plan.filter.is_none())
                        && plan.related.as_ref().is_some_and(|r| r.closure_eligible())
                        && host.can_remember(meta.context, &f.request.node)
                    {
                        let key = ClosureKey {
                            owner: p.identity,
                            program: f.request.program,
                            context: meta.context,
                            doc: f.request.node.get_doc() as *const D as usize,
                            node: f.request.node.node_id(),
                            three: f.request.three,
                        };
                        let queue = if plan.axis == 1 {
                            f.request.node.children().collect()
                        } else {
                            VecDeque::new()
                        };
                        let cursor = match plan.axis {
                            0 => f.request.node.parent(),
                            2 => f.request.node.next_all().next(),
                            3 => f.request.node.prev_all().next(),
                            _ => None,
                        };
                        let closure = Box::new(Closure {
                            plan: plan.clone(),
                            key,
                            queue,
                            cursor,
                            current: None,
                            walked: Vec::new(),
                            open: Vec::new(),
                            spent: 0,
                        });
                        return self.advance_closure(f, closure, None);
                    }
                    let mut search = walk_search(plan.clone(), &f.request.node, None);
                    if p.entries[f.request.program as usize].rollback_nontrue {
                        search.rollback_input = Some(f.request.env.clone());
                    }
                    f.state = State::Search(Box::new(search));
                    Action::Continue
                }
                Op::Count {
                    walk,
                    min,
                    max,
                    binding,
                } => {
                    let counter = Counter {
                        min: *min,
                        max: *max,
                        binding: binding.clone(),
                        found: 0,
                        unknown: 0,
                        open: false,
                    };
                    f.state = State::Search(Box::new(walk_search(
                        walk.clone(),
                        &f.request.node,
                        Some(counter),
                    )));
                    Action::Continue
                }
                Op::Nth {
                    cache_term,
                    child,
                    step,
                    offset,
                    reverse,
                } => {
                    let Some(parent) = f.request.node.parent() else {
                        return Action::Finish(f.result(0, None));
                    };
                    let cache = cache_term
                        .filter(|_| host.is_file(meta.context, &f.request.node))
                        .map(|term| {
                            (
                                p.identity,
                                term,
                                f.request.node.get_doc() as *const D as usize,
                                parent.node_id(),
                            )
                        });
                    if cache.is_some() && child.is_some() {
                        f.clause_state_read = true;
                    }
                    if let Some((counted, unknown)) = cache.and_then(|key| self.siblings.get(&key))
                    {
                        if *unknown {
                            return Action::Finish(f.result(1, None));
                        }
                        let Some(pos) = counted.binary_search(&f.request.node.node_id()).ok()
                        else {
                            return Action::Finish(f.result(0, None));
                        };
                        if let Some(child) = child {
                            let next = f.child(
                                *child,
                                f.request.node.clone(),
                                f.request.env.clone(),
                                false,
                            );
                            f.state = State::Nth(Box::new(NthState {
                                cache: None,
                                replay_before: f.replay,
                                child: *child,
                                nodes: Vec::new(),
                                index: 0,
                                counted: counted.clone(),
                                step: *step,
                                offset: *offset,
                                reverse: *reverse,
                                own: true,
                            }));
                            return Action::Push(next);
                        }
                        let holds = position(
                            *step,
                            *offset,
                            if *reverse {
                                counted.len() - 1 - pos
                            } else {
                                pos
                            },
                        );
                        return Action::Finish(f.result(if holds { 2 } else { 0 }, None));
                    }
                    let nodes: Vec<_> = parent.children().filter(Node::is_named).collect();
                    if let Some(child) = child {
                        if nodes.is_empty() {
                            if let Some(key) = cache {
                                self.remember_siblings(key, Arc::new(Vec::new()), false);
                            }
                            return Action::Finish(f.result(0, None));
                        }
                        let replay_before = f.replay;
                        if !host.spend(meta.context, 1) {
                            return Action::Finish(work_failure(f, meta.context, host));
                        }
                        f.work += 1;
                        if cache.is_none() {
                            f.replay += 1;
                        }
                        let next = f.child(*child, nodes[0].clone(), f.request.env.clone(), true);
                        f.state = State::Nth(Box::new(NthState {
                            cache,
                            replay_before,
                            child: *child,
                            nodes,
                            index: 0,
                            counted: Arc::new(Vec::new()),
                            step: *step,
                            offset: *offset,
                            reverse: *reverse,
                            own: false,
                        }));
                        Action::Push(next)
                    } else {
                        let counted: Vec<_> = nodes.iter().map(Node::node_id).collect();
                        let index = counted.binary_search(&f.request.node.node_id()).ok();
                        let matches = index.is_some_and(|i| {
                            position(
                                *step,
                                *offset,
                                if *reverse { counted.len() - 1 - i } else { i },
                            )
                        });
                        if let Some(key) = cache {
                            self.remember_siblings(key, Arc::new(counted), false);
                        }
                        Action::Finish(f.result(if matches { 2 } else { 0 }, None))
                    }
                }
                Op::Util {
                    memo_term,
                    target,
                    args,
                    scope,
                    scope_id,
                } => {
                    if f.depth >= super::util::MAX_UTIL_DEPTH {
                        let mut out = f.result(0, None);
                        out.failure = Some(Failure::Utility(UtilLimit::Depth));
                        return Action::Finish(out);
                    }
                    if args.is_none()
                        && let Some(kinds) = target
                            .and_then(|target| p.entries[target as usize].guard.as_ref())
                            .filter(|guard| !guard.requires_unexcluded)
                            .and_then(|guard| guard.kinds.as_ref())
                    {
                        self.metrics.utility_kind_checks += 1;
                        if kinds.binary_search(&f.request.node.kind_id()).is_err() {
                            // The target rejects before doing any work. Its
                            // caller still spends the native utility entry
                            // charge and observes the same depth limit.
                            if !host.spend(meta.context, 1) {
                                return Action::Finish(work_failure(f, meta.context, host));
                            }
                            f.work += 1;
                            f.replay += 1;
                            f.levels = 1;
                            self.metrics.utility_kind_rejections += 1;
                            return Action::Finish(f.result(0, None));
                        }
                    }
                    let mut key = memo_term
                        .filter(|_| host.can_remember(meta.context, &f.request.node))
                        .map(|term| {
                            Box::new(MemoKey {
                                term,
                                context: None,
                                node: f.request.node.node_id(),
                                doc: f.request.node.get_doc() as *const D as usize,
                                three: f.request.three,
                                each: f
                                    .request
                                    .each
                                    .as_ref()
                                    .map(|e| ExclusionIdentity(Arc::downgrade(e))),
                                captures: self.fingerprint(
                                    &f.request.env,
                                    p.identity,
                                    f.request.program,
                                    *scope_id,
                                    scope.as_deref(),
                                    f.request.each.as_ref().map(|e| e.name.as_str()),
                                ),
                                following: meta.following.clone(),
                                owner: p.identity,
                            })
                        });
                    let mut cached = key.as_ref().and_then(|k| {
                        self.memo.get(k.as_ref()).filter(|c| {
                            f.depth.saturating_add(c.levels) <= super::util::MAX_UTIL_DEPTH
                        })
                    });
                    if cached.is_none()
                        && let Some(key) = key.as_mut()
                    {
                        key.context = Some(meta.context);
                        cached = self.memo.get(key.as_ref()).filter(|c| {
                            f.depth.saturating_add(c.levels) <= super::util::MAX_UTIL_DEPTH
                        });
                    }
                    if let Some(cached) = cached {
                        let cached = cached.clone();
                        if !host.spend(meta.context, cached.replay_work) {
                            return Action::Finish(work_failure(f, meta.context, host));
                        }
                        let env = if let Some(delta) = &cached.delta {
                            cohort_memo::apply(delta, f.request.env.clone())
                        } else {
                            Some(f.request.env.clone())
                        };
                        let truth = if env.is_some() { cached.truth } else { 0 };
                        self.metrics.utility_hits += 1;
                        return Action::Finish(Result {
                            truth,
                            env: env.unwrap_or_else(|| f.request.env.clone()),
                            work: cached.replay_work,
                            replay_work: cached.replay_work,
                            levels: cached.levels,
                            clause_state_read: f.clause_state_read || cached.clause_state_read,
                            failure: None,
                        });
                    }
                    if !host.spend(meta.context, 1) {
                        return Action::Finish(work_failure(f, meta.context, host));
                    }
                    f.work += 1;
                    f.replay += 1;
                    f.levels = 1;
                    let Some(target) = target else {
                        return Action::Finish(f.result(0, None));
                    };
                    let mut request = f.request.clone();
                    request.program = *target;
                    if let Some(args) = args {
                        let mut frame = MetaVarEnv::new();
                        for (param, caller) in args {
                            if super::unify_capture(
                                &f.request.env,
                                std::iter::once(caller.as_str()),
                                &mut frame,
                                param,
                            )
                            .is_none()
                            {
                                return Action::Finish(f.result(0, None));
                            }
                        }
                        request.env = Arc::new(frame);
                        request.each = f.request.each.as_ref().and_then(|e| {
                            args.iter()
                                .find(|(_, caller)| caller == &e.name)
                                .map(|(param, _)| {
                                    Arc::new(Exclusion {
                                        name: param.clone(),
                                        ranges: e.ranges.clone(),
                                    })
                                })
                        });
                    }
                    let child = PendingChild {
                        request,
                        depth: f.depth + 1,
                    };
                    f.state = State::Util { key };
                    Action::Push(child)
                }
                _ => Action::Leaf,
            },
            State::Not => {
                let r = returned.expect("not child");
                f.absorb(&r);
                Action::Finish(f.result(
                    match r.truth {
                        2 => 0,
                        0 => 2,
                        _ => 1,
                    },
                    None,
                ))
            }
            State::List {
                atomic: _,
                mut index,
                any,
                mut trial,
                mut unknown,
                mut withheld,
            } => {
                let children = p.children(f.request.program);
                let r = returned.expect("list child");
                f.absorb(&r);
                let entry = &p.entries[children[index] as usize];
                if any {
                    if r.truth == 2
                        && !excluded(host, meta.context, f.request.each.as_deref(), &r.env)
                    {
                        return Action::Finish(f.result(2, Some(r.env)));
                    }
                    if r.truth == 1 {
                        unknown = true;
                        if entry.may_bind {
                            return Action::Finish(f.result(1, None));
                        }
                    }
                } else {
                    if r.truth == 0 {
                        return Action::Finish(f.result(0, None));
                    }
                    if r.truth == 1 {
                        unknown = true;
                        if !entry.binds.is_empty() {
                            withheld
                                .get_or_insert_with(Default::default)
                                .extend(entry.binds.iter().cloned());
                        }
                    } else {
                        trial = r.env;
                    }
                }
                index += 1;
                if any {
                    self.skip_guarded_alternatives(p, f, children, &mut index, meta, host);
                }
                if !any && let Some(withheld) = &mut withheld {
                    while index < children.len()
                        && !p.entries[children[index] as usize]
                            .mentions
                            .is_disjoint(&withheld)
                    {
                        withheld.extend(p.entries[children[index] as usize].binds.iter().cloned());
                        index += 1;
                    }
                }
                if index == children.len() {
                    return Action::Finish(f.result(
                        if unknown {
                            1
                        } else if any {
                            0
                        } else {
                            2
                        },
                        if !unknown && !any { Some(trial) } else { None },
                    ));
                }
                let program = children[index];
                let atomic = p.atomic(program).is_some();
                f.state = State::List {
                    index,
                    atomic,
                    any,
                    trial,
                    unknown,
                    withheld,
                };
                if atomic {
                    Action::Inline
                } else {
                    let env = match &f.state {
                        State::List { trial, .. } if !any => trial.clone(),
                        _ => f.request.env.clone(),
                    };
                    let child = f.child(program, f.request.node.clone(), env, false);
                    Action::Push(child)
                }
            }
            State::Closure(s) => self.advance_closure(f, s, returned),
            State::Constraints { mut index, first } => {
                let Op::Constrained {
                    constraints: rules, ..
                } = &p.entries[f.request.program as usize].op
                else {
                    unreachable!()
                };
                let r = returned.expect("constraint child");
                f.absorb(&r);
                if r.truth != 2 {
                    return Action::Finish(f.result(r.truth, None));
                }
                let trial = r.env;
                if !first {
                    index += 1;
                }
                while index < rules.len() && trial.get_match(&rules[index].0).is_none() {
                    index += 1;
                }
                if index == rules.len() {
                    return Action::Finish(f.result(2, Some(trial)));
                }
                let node = trial.get_match(&rules[index].0).unwrap().clone();
                let child = f.child(rules[index].1, node, trial.clone(), false);
                f.state = State::Constraints {
                    index,
                    first: false,
                };
                Action::Push(child)
            }
            State::Nth(mut state) => {
                let r = returned.expect("nth child");
                f.absorb(&r);
                if state.own {
                    // The selected node's probe keeps its capture frame even
                    // when the arithmetic predicate rejects its position.
                    let pos = state
                        .counted
                        .binary_search(&f.request.node.node_id())
                        .unwrap();
                    let holds = position(
                        state.step,
                        state.offset,
                        if state.reverse {
                            state.counted.len() - 1 - pos
                        } else {
                            pos
                        },
                    );
                    return Action::Finish(f.result(if holds { 2 } else { 0 }, Some(r.env)));
                }
                if state.cache.is_some() {
                    f.replay = state.replay_before;
                }
                if r.truth == 1 {
                    if let Some(key) = state.cache {
                        self.remember_siblings(key, state.counted, true);
                    }
                    return Action::Finish(f.result(1, None));
                }
                if r.truth == 2 {
                    let id = state.nodes[state.index].node_id();
                    Arc::make_mut(&mut state.counted).push(id);
                }
                state.index += 1;
                if state.index < state.nodes.len() {
                    if !host.spend(meta.context, 1) {
                        return Action::Finish(work_failure(f, meta.context, host));
                    }
                    f.work += 1;
                    if state.cache.is_none() {
                        f.replay += 1;
                    }
                    let next = f.child(
                        state.child,
                        state.nodes[state.index].clone(),
                        f.request.env.clone(),
                        true,
                    );
                    f.state = State::Nth(state);
                    return Action::Push(next);
                }
                if let Some(key) = state.cache {
                    self.remember_siblings(key, state.counted.clone(), false);
                }
                if state
                    .counted
                    .binary_search(&f.request.node.node_id())
                    .is_err()
                {
                    return Action::Finish(f.result(0, None));
                }
                // Reprobe the selected node with exclusions restored, including
                // where the arithmetic predicate will reject it.
                let next = f.child(
                    state.child,
                    f.request.node.clone(),
                    f.request.env.clone(),
                    false,
                );
                state.cache = None;
                state.own = true;
                f.state = State::Nth(state);
                Action::Push(next)
            }
            State::Util { mut key } => {
                let Op::Util { args, scope, .. } = &p.entries[f.request.program as usize].op else {
                    unreachable!()
                };
                let r = returned.expect("utility child");
                f.work = f.work.saturating_add(r.work);
                f.replay = f.replay.saturating_add(r.replay_work);
                f.levels = 1 + r.levels;
                f.clause_state_read |= r.clause_state_read;
                let mut truth = r.truth;
                let framed = args.is_some();
                let mut env = r.env;
                if truth == 2
                    && let Some(args) = args
                {
                    let frame = env;
                    let mut caller = (*f.request.env).clone();
                    let returned = args.iter().any(|(param, name)| {
                        frame.get_match(param).is_some() && caller.get_match(name).is_none()
                            || !frame.get_multiple_matches(param).is_empty()
                                && caller.get_multiple_matches(name).is_empty()
                            || frame.get_transformed(param).is_some()
                                && caller.get_transformed(name).is_none()
                    });
                    if returned {
                        let mut unified = BTreeSet::new();
                        for (_, name) in args {
                            if !unified.insert(name) {
                                continue;
                            }
                            let params = args
                                .iter()
                                .filter(|(_, n)| n == name)
                                .map(|(param, _)| param.as_str());
                            if super::unify_capture(&frame, params, &mut caller, name).is_none() {
                                truth = 0;
                                break;
                            }
                        }
                    }
                    env = Arc::new(caller);
                }
                if !f.clause_state_read
                    && let Some(key) = key.as_mut()
                {
                    key.context = None;
                }
                let delta = (truth == 2 && key.is_some()).then(|| {
                    Arc::new(cohort_memo::between(
                        &f.request.env,
                        &env,
                        scope.as_deref(),
                        f.request.each.as_ref().map(|e| e.name.as_str()),
                    ))
                });
                let out = f.result(
                    truth,
                    if truth == 2 || !framed {
                        Some(env)
                    } else {
                        None
                    },
                );
                let Some(key) = key else {
                    return Action::Finish(out);
                };
                // Bounded memo storage; clearing retains correctness and limits
                // retained capture frames for very large workloads.
                if self.memo.len() >= 32768 {
                    self.metrics.utility_evictions += self.memo.len();
                    self.memo.clear();
                }
                self.memo.insert(
                    *key,
                    MemoEntry {
                        truth: out.truth,
                        delta,
                        replay_work: out.replay_work,
                        levels: out.levels,
                        clause_state_read: out.clause_state_read,
                    },
                );
                self.metrics.max_memo_entries = self.metrics.max_memo_entries.max(self.memo.len());
                Action::Finish(out)
            }
            State::Search(mut search) => {
                let input = search.rollback_input.take();
                let mut action = self.search(p, f, search, returned, meta, host);
                if let Action::Finish(answer) = &mut action {
                    if answer.truth != 2 || answer.failure.is_some() {
                        if let Some(input) = input {
                            // Search temporarily updates request.env after a
                            // false candidate. Restore the original transaction
                            // before the common finish handler applies All.
                            f.request.env = input;
                            answer.env = f.request.env.clone();
                        }
                    }
                } else if let State::Search(search) = &mut f.state {
                    search.rollback_input = input;
                }
                action
            }
        }
    }
    fn search<H: Host<'t, D>>(
        &mut self,
        p: &Programs,
        f: &mut Frame<'t, D>,
        mut s: Box<Search<'t, D>>,
        returned: Option<Result<'t, D>>,
        meta: &LaneMeta,
        host: &mut H,
    ) -> Action<'t, D> {
        if let Some(r) = returned {
            f.absorb(&r);
            match s.phase {
                1 => {
                    s.stopped = r.truth;
                    s.phase = 2;
                }
                2 => {
                    // Search::test passes its real environment to candidates
                    // except under `each`. Preserve a false NthChild's selected
                    // ofRule writes for the following candidate, while counts
                    // and excluding searches continue to use isolated probes.
                    if s.counter.is_none() && f.request.each.is_none() {
                        f.request.env = r.env.clone();
                    }
                    if let Some(c) = s.counter.as_mut() {
                        if r.truth == 2 {
                            c.found += 1;
                        } else if r.truth == 1 {
                            c.unknown += 1;
                        }
                        if c.max.is_some_and(|max| c.found > max) {
                            return Action::Finish(f.result(0, None));
                        }
                        if c.max.is_none() && c.binding.is_none() && c.found >= c.min {
                            return Action::Finish(f.result(2, None));
                        }
                    } else {
                        if r.truth == 2
                            && !excluded(host, meta.context, f.request.each.as_deref(), &r.env)
                        {
                            return Action::Finish(f.result(2, Some(r.env)));
                        }
                        if r.truth == 1 {
                            s.unknown = true;
                            if p.entries[s.plan.child as usize].may_bind {
                                return Action::Finish(f.result(1, None));
                            }
                        }
                        // Upstream `has` reads a pruned unknown subtree after
                        // testing the next candidate. Its matching candidate wins.
                        if s.pending_open {
                            s.pending_open = false;
                            s.unknown = true;
                            if p.entries[s.plan.child as usize].may_bind {
                                return Action::Finish(f.result(1, None));
                            }
                        }
                    }
                    s.phase = 3;
                }
                3 => {
                    s.stopped = r.truth;
                    s.phase = 4;
                }
                _ => unreachable!("walk child phase"),
            }
        }
        loop {
            match s.phase {
                0 => {
                    let visit = if let Some(cursor) = &mut s.linear {
                        cursor.next.take().map(|node| {
                            let eligible = s.plan.field.is_none_or(|field| {
                                node.child_by_field_id(field)
                                    .is_some_and(|child| child.node_id() == cursor.previous)
                            });
                            cursor.previous = node.node_id();
                            Visit {
                                node,
                                eligible,
                                descend: 0,
                                stop: !matches!(s.plan.stop, Stop::Neighbor),
                            }
                        })
                    } else {
                        s.queue.pop_front()
                    };
                    let Some(visit) = visit else {
                        if let Some(c) = s.counter {
                            let high = (!c.open).then_some(c.found.saturating_add(c.unknown));
                            let holds = (c.binding.is_none() || high == Some(c.found))
                                && c.found >= c.min
                                && c.max.is_none_or(|max| high.is_some_and(|n| n <= max));
                            if holds {
                                let env = if let Some(binding) = c.binding {
                                    let mut env = (*f.request.env).clone();
                                    let value = i128::try_from(c.found).unwrap_or(i128::MAX)
                                        + i128::from(binding.offset);
                                    let bytes =
                                        <D::Source as Content>::decode_str(&value.to_string())
                                            .into_owned();
                                    env.insert_transformation(
                                        &MetaVariable::Multiple,
                                        &binding.name,
                                        bytes,
                                    );
                                    Some(Arc::new(env))
                                } else {
                                    None
                                };
                                return Action::Finish(f.result(2, env));
                            }
                            let fails = high.is_some_and(|n| n < c.min)
                                || c.max.is_some_and(|max| c.found > max);
                            return Action::Finish(f.result(if fails { 0 } else { 1 }, None));
                        }
                        return Action::Finish(
                            f.result(if s.unknown || s.pending_open { 1 } else { 0 }, None),
                        );
                    };
                    s.current = Some(visit);
                    s.stopped = 0;
                    if s.plan.axis != 1
                        && s.current.as_ref().unwrap().stop
                        && let Stop::Rule(stop) = s.plan.stop
                    {
                        let child = f.child(
                            stop,
                            s.current.as_ref().unwrap().node.clone(),
                            Arc::new(MetaVarEnv::new()),
                            true,
                        );
                        s.phase = 1;
                        f.state = State::Search(s);
                        return Action::Push(child);
                    }
                    s.phase = 2;
                }
                2 => {
                    let visit = s.current.as_ref().unwrap();
                    if !visit.eligible
                        || (s.counter.is_none()
                            && !f.request.three
                            && s.plan.filter.as_ref().is_some_and(|filter| {
                                filter.kinds.binary_search(&visit.node.kind_id()).is_err()
                            }))
                    {
                        s.phase = 3;
                        continue;
                    }
                    if s.counter.is_some() {
                        if !host.spend(meta.context, 1) {
                            return Action::Finish(work_failure(f, meta.context, host));
                        }
                        f.work += 1;
                        f.replay += 1;
                    }
                    let child = f.child(
                        s.plan.child,
                        visit.node.clone(),
                        f.request.env.clone(),
                        s.counter.is_some(),
                    );
                    f.state = State::Search(s);
                    return Action::Push(child);
                }
                3 => {
                    let visit = s.current.as_ref().unwrap();
                    if s.plan.axis == 1
                        && visit.stop
                        && let Stop::Rule(stop) = s.plan.stop
                    {
                        let child =
                            f.child(stop, visit.node.clone(), Arc::new(MetaVarEnv::new()), true);
                        f.state = State::Search(s);
                        return Action::Push(child);
                    }
                    s.phase = 4;
                }
                4 => {
                    let visit = s.current.take().unwrap();
                    if s.stopped == 1 {
                        if let Some(c) = s.counter.as_mut() {
                            c.open = true;
                        } else if s.plan.axis == 1 && s.plan.field.is_none() {
                            s.pending_open = true;
                        } else {
                            s.unknown = true;
                        }
                    }
                    if s.plan.axis != 1 && s.stopped != 0 {
                        s.queue.clear();
                        if let Some(cursor) = &mut s.linear {
                            cursor.next = None;
                        }
                    } else if s.plan.axis != 1 {
                        if let Some(cursor) = &mut s.linear {
                            if !matches!(s.plan.stop, Stop::Neighbor) {
                                cursor.next = match s.plan.axis {
                                    0 => visit.node.parent(),
                                    2 => self.structural_sibling(&visit.node, true),
                                    _ => self.structural_sibling(&visit.node, false),
                                };
                            }
                        }
                    } else if s.plan.axis == 1 {
                        if s.stopped == 1 && s.plan.field.is_some() && s.counter.is_none() {
                            return Action::Finish(f.result(1, None));
                        }
                        if s.stopped == 0 && visit.descend != 0 {
                            let nodes: Vec<_> = visit.node.children().collect();
                            for node in nodes.into_iter().rev() {
                                s.queue.push_front(Visit {
                                    node,
                                    eligible: true,
                                    descend: if visit.descend == 1 { 0 } else { 2 },
                                    stop: visit.descend != 1,
                                });
                            }
                        }
                    }
                    s.phase = 0;
                }
                _ => unreachable!("walk ready phase"),
            }
        }
    }
}
pub(in crate::code_rule_dsl) fn execute<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
    H: Host<'t, D>,
>(
    programs: &Programs,
    requests: Vec<Request<'t, D>>,
    host: &mut H,
) -> Vec<Result<'t, D>> {
    Bank::default().execute(programs, requests, host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::{AstGrep as Root, language::Language, matcher::KindMatcher};
    use std::{borrow::Cow, collections::HashMap, sync::OnceLock};
    #[test]
    fn lane_metadata_survives_foreign_constraints_and_source_refills() {
        struct Probe {
            root: usize,
            foreign_queries: usize,
            compared: Vec<u32>,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for Probe {
            fn is_file(&self, _: u32, node: &Node<'t, D>) -> bool {
                node.root() as *const _ as usize == self.root
            }
            fn can_remember(&self, context: u32, node: &Node<'t, D>) -> bool {
                self.is_file(context, node)
            }
            fn evaluate(
                &mut self,
                _: &Programs,
                _: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("semantic")
            }
            fn compare(
                &mut self,
                _: &Programs,
                jobs: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                jobs.into_iter()
                    .map(|job| {
                        let context = job.request.context;
                        self.compared.push(context);
                        assert_eq!(job.utility_depth, 3);
                        assert_eq!(job.request.node.text(), "x");
                        assert_eq!(job.request.following.as_ref(), &[(context as usize, 7)]);
                        assert_eq!(job.request.three, context != 10);
                        assert_eq!(job.request.each.is_some(), context == 10);
                        let mut env = (*job.request.env).clone();
                        env.insert("LEAK", job.request.node).unwrap();
                        let mut answer = Result::new(1, Arc::new(env));
                        answer.work = 2;
                        answer.replay_work = 1;
                        answer.levels = 2;
                        answer.clause_state_read = true;
                        answer
                    })
                    .collect()
            }
            fn spend(&mut self, _: u32, amount: usize) -> bool {
                // Only the foreign nthChild sibling probe spends through this hook.
                assert_eq!(amount, 1);
                self.foreign_queries += 1;
                true
            }
        }
        struct Feeding<'t, D: Doc<Source: Content<Underlying = u8>>> {
            pending: VecDeque<(usize, Request<'t, D>)>,
            originals: Vec<Request<'t, D>>,
            answers: Vec<(usize, u8)>,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Source<'t, D, Probe> for Feeding<'t, D> {
            type Token = usize;
            fn next(&mut self, _: &mut Probe) -> Option<(usize, Request<'t, D>)> {
                self.pending.pop_front()
            }
            fn completed(
                &mut self,
                token: usize,
                request: Request<'t, D>,
                answer: Result<'t, D>,
                _: &mut Probe,
            ) {
                let original = &self.originals[token];
                assert_eq!(
                    (request.program, request.context, request.three),
                    (original.program, original.context, original.three)
                );
                assert_eq!(request.node.node_id(), original.node.node_id());
                assert!(std::ptr::eq(request.node.root(), original.node.root()));
                assert!(Arc::ptr_eq(&request.env, &original.env));
                assert!(Arc::ptr_eq(&request.following, &original.following));
                assert_eq!(
                    request.each.as_ref().map(Arc::as_ptr),
                    original.each.as_ref().map(Arc::as_ptr)
                );
                assert_eq!((answer.work, answer.replay_work, answer.levels), (3, 2, 2));
                assert!(answer.clause_state_read && answer.failure.is_none());
                assert!(answer.env.get_match("X").is_none());
                assert!(answer.env.get_match("LEAK").is_none());
                self.answers.push((token, answer.truth));
                if token == 0 {
                    self.pending.push_back((2, self.originals[2].clone()));
                }
            }
        }
        let local = Root::str("f(x);", GrammarDialect::JavaScript);
        let foreign = Root::str("other;", GrammarDialect::JavaScript);
        let node = local
            .root()
            .dfs()
            .find(|node| node.kind() == "call_expression")
            .unwrap();
        let other = foreign
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier")
            .unwrap();
        let comparison = super::super::Comparison {
            left: "X".into(),
            right: "X".into(),
            op: super::super::CompareOp::Eq,
            collation: super::super::Collation::Ordinal,
            text: super::super::CompareText::Source,
            reading: None,
            transform: None,
            commute: Vec::new(),
        };
        let rule = AstMatcher::Constrained(Box::new(super::super::Constrained {
            rule: pattern("f($X)"),
            constraints: vec![
                (
                    "FOREIGN".into(),
                    AstMatcher::NthChild(Box::new(super::super::NthChild {
                        position: super::super::NthPosition { step: 0, offset: 1 },
                        of_rule: Some(kind("identifier")),
                        reverse: false,
                        capture_free: OnceLock::new(),
                    })),
                ),
                ("X".into(), AstMatcher::Compare(Box::new(comparison))),
            ],
        }));
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let originals: Vec<_> = [10, 20, 30]
            .into_iter()
            .map(|context| {
                let mut env = MetaVarEnv::new();
                env.insert("FOREIGN", other.clone()).unwrap();
                Request {
                    program,
                    context,
                    node: node.clone(),
                    env: Arc::new(env),
                    three: context != 10,
                    each: (context == 10).then(|| {
                        Arc::new(Exclusion {
                            name: "X".into(),
                            ranges: BTreeSet::new(),
                        })
                    }),
                    following: Arc::new(vec![(context as usize, 7)]),
                }
            })
            .collect();
        let pending = originals
            .iter()
            .take(2)
            .enumerate()
            .map(|(i, request)| (i, request.clone()))
            .collect();
        let mut source = Feeding {
            pending,
            originals,
            answers: Vec::new(),
        };
        let mut host = Probe {
            root: local.root().root() as *const _ as usize,
            foreign_queries: 0,
            compared: Vec::new(),
        };
        let mut bank = Bank::default();
        bank.execute_source(&programs, &mut source, &mut host, 2, 3);
        source.answers.sort_unstable();
        host.compared.sort_unstable();
        assert_eq!(source.answers, vec![(0, 0), (1, 1), (2, 1)]);
        assert_eq!(host.compared, vec![10, 20, 30]);
        assert_eq!(
            host.foreign_queries, 3,
            "foreign nodes must never use the file sibling cache"
        );
    }

    #[test]
    fn borrowed_pure_leaves_preserve_transactions_without_owned_requests() {
        let doc = Root::str("f(a); f(b);", GrammarDialect::JavaScript);
        let nodes: Vec<_> = doc
            .root()
            .dfs()
            .filter(|node| node.kind() == "call_expression")
            .collect();
        let rule = AstMatcher::All(vec![
            AstMatcher::Regex(regex::Regex::new("^f").unwrap()),
            AstMatcher::Constrained(Box::new(super::super::Constrained {
                rule: pattern("$F($X)"),
                constraints: vec![
                    (
                        "F".into(),
                        AstMatcher::Regex(regex::Regex::new("^f$").unwrap()),
                    ),
                    ("X".into(), kind("identifier")),
                ],
            })),
        ]);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let mut conflict = MetaVarEnv::new();
        let first_arg = nodes[0].dfs().find(|node| node.text() == "a").unwrap();
        conflict.insert("F", first_arg).unwrap();
        let inputs = [Arc::new(MetaVarEnv::new()), Arc::new(conflict)];
        let following = Arc::new(vec![(11, 13)]);
        let requests: Vec<_> = inputs
            .iter()
            .enumerate()
            .map(|(i, env)| Request {
                program,
                context: i as u32,
                node: nodes[i].clone(),
                env: env.clone(),
                three: true,
                each: None,
                following: following.clone(),
            })
            .collect();
        let expected: Vec<_> = requests
            .iter()
            .map(|request| {
                let mut env = Cow::Owned((*request.env).clone());
                let truth = if rule
                    .match_node_with_env(request.node.clone(), &mut env)
                    .is_some()
                {
                    2
                } else {
                    0
                };
                (truth, HashMap::<String, String>::from(env.into_owned()))
            })
            .collect();
        let mut bank = Bank::default();
        let answers = bank.execute(&programs, requests, &mut SyntaxHost);
        for (answer, expected) in answers.iter().zip(expected) {
            assert_eq!(
                (
                    answer.truth,
                    HashMap::<String, String>::from((*answer.env).clone())
                ),
                expected
            );
            assert_eq!((answer.work, answer.replay_work, answer.levels), (0, 0, 0));
            assert!(answer.failure.is_none());
        }
        assert!(Arc::ptr_eq(&answers[1].env, &inputs[1]));
        assert!(bank.metrics.borrowed_leaf_lanes >= 5);
        assert_eq!(bank.metrics.owned_leaf_requests, 0);
        assert_eq!(Arc::strong_count(&following), 1);
        assert!(
            std::mem::size_of::<Input<'_, ast_grep_core::tree_sitter::StrDoc<GrammarDialect>>>()
                < std::mem::size_of::<
                    Request<'_, ast_grep_core::tree_sitter::StrDoc<GrammarDialect>>,
                >()
        );
    }

    #[test]
    fn fingerprint_scopes_share_across_distinct_call_programs() {
        let table = Arc::new(super::super::UtilTable::new(
            vec!["pair".into()],
            vec![true],
            vec![None],
        ));
        assert!(table.rules.set(vec![pattern("$P($Q)")]).is_ok());
        let call = |args| {
            AstMatcher::Call(Box::new(super::super::UtilCall {
                util: super::super::UtilRef {
                    table: Arc::downgrade(&table),
                    index: 0,
                },
                args,
            }))
        };
        let a = call(vec![("P".into(), "X".into()), ("Q".into(), "Y".into())]);
        let b = call(vec![("P".into(), "Y".into()), ("Q".into(), "X".into())]);
        let mut programs = Programs::default();
        let pa = programs.lower(&a);
        let pb = programs.lower(&b);
        assert_ne!(pa, pb);
        let scope = |program| {
            let Op::Util {
                scope, scope_id, ..
            } = &programs.entries[program as usize].op
            else {
                panic!("call")
            };
            (scope.as_deref().unwrap(), *scope_id)
        };
        let (sa, ia) = scope(pa);
        let (sb, ib) = scope(pb);
        assert_eq!(sa, sb);
        assert_eq!(ia, ib);
        assert_eq!(
            sa.iter().map(|n| n.as_ref()).collect::<Vec<_>>(),
            vec!["X", "Y"]
        );
        let doc = Root::str("x;", GrammarDialect::JavaScript);
        let mut env = MetaVarEnv::new();
        env.insert("X", doc.root()).unwrap();
        let env = Arc::new(env);
        let mut bank = Bank::default();
        let first = bank.fingerprint(&env, programs.identity, pa, ia, Some(sa), None);
        let builds = bank.metrics.fingerprint_builds;
        let second = bank.fingerprint(&env, programs.identity, pb, ib, Some(sb), None);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(bank.metrics.fingerprint_builds, builds);
        assert_eq!(bank.metrics.fingerprint_cross_program_hits, 1);
        let other_pool = Programs::default();
        let separate = bank.fingerprint(&env, other_pool.identity, pb, ib, Some(sb), None);
        assert!(!Arc::ptr_eq(&first, &separate));
        assert!(*first == *separate);
    }

    #[test]
    fn fingerprint_scope_identity_preserves_empty_full_and_exclusion_selection() {
        let doc = Root::str("x; y;", GrammarDialect::JavaScript);
        let mut input = MetaVarEnv::new();
        input.insert("X", doc.root().child(0).unwrap()).unwrap();
        input.insert("Y", doc.root().child(1).unwrap()).unwrap();
        let env = Arc::new(input);
        let mut programs = Programs::default();
        let scopes: Vec<Option<Arc<[Arc<str>]>>> = vec![
            None,
            Some(Arc::from([])),
            Some(Arc::from([Arc::from("X")])),
            Some(Arc::from([Arc::from("Y")])),
        ];
        let ids: Vec<_> = scopes
            .iter()
            .map(|scope| programs.intern_fingerprint_scope(scope.clone()))
            .collect();
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 4);
        let mut bank = Bank::default();
        let mut values = Vec::new();
        for (id, scope) in ids.iter().zip(&scopes) {
            let value = bank.fingerprint(&env, programs.identity, 7, *id, scope.as_deref(), None);
            assert!(*value == capture_fingerprint(&env, scope.as_deref()));
            values.push(value);
        }
        assert!(*values[0] != *values[1]);
        assert!(Arc::ptr_eq(&values[1], &bank.empty_fingerprint));
        assert_eq!(bank.metrics.fingerprint_empty_scope_hits, 1);
        assert!(*values[2] != *values[3]);
        let excluded = bank.fingerprint(
            &env,
            programs.identity,
            8,
            ids[2],
            scopes[2].as_deref(),
            Some("Y"),
        );
        assert!(*excluded == *values[0]);
        let unexcluded = bank.fingerprint(
            &env,
            programs.identity,
            9,
            ids[2],
            scopes[2].as_deref(),
            None,
        );
        assert!(*unexcluded == *values[2]);
        assert!(!Arc::ptr_eq(&excluded, &unexcluded));
        // In-place COW mutation dissociates Weak identity even if pointer storage is reused.
        let mut env = env;
        Arc::make_mut(&mut env).insert_transformation(
            &MetaVariable::Multiple,
            "\0X",
            b"changed".to_vec(),
        );
        let changed = bank.fingerprint(
            &env,
            programs.identity,
            9,
            ids[2],
            scopes[2].as_deref(),
            None,
        );
        assert!(*changed == capture_fingerprint(&env, scopes[2].as_deref()));
        assert!(*changed != *unexcluded);
    }

    #[test]
    fn utility_frontier_failure_retains_completed_body_relative_peak() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let input = Arc::new(MetaVarEnv::new());
        let mut frame = Frame::new(
            Request {
                program: 0,
                context: 0,
                node: doc.root(),
                env: input.clone(),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            }
            .split()
            .0,
            0,
        );
        frame.state = State::Util { key: None };
        frame.work = 1;
        frame.replay = 1;
        frame.levels = 1;
        // The frontier can observe cancellation after a successful body returns,
        // before the utility continuation consumes its relative peak.
        let mut body = Result::new(2, input.clone());
        body.work = 7;
        body.replay_work = 2;
        body.levels = 3;
        body.clause_state_read = true;
        frame.absorb_failure(&body);
        let mut failed = frame.result(0, None);
        failed.failure = Some(Failure::Cancelled);
        assert_eq!((failed.work, failed.replay_work, failed.levels), (8, 3, 4));
        assert!(failed.clause_state_read);
        assert_eq!(failed.truth, 0);
        assert_eq!(failed.failure, Some(Failure::Cancelled));
        assert!(Arc::ptr_eq(&failed.env, &input));
    }
    struct SyntaxHost;
    impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> Host<'t, D>
        for SyntaxHost
    {
        fn can_remember(&self, _: u32, _: &Node<'t, D>) -> bool {
            true
        }
        fn evaluate(&mut self, _: &Programs, _: Vec<SemanticRequest<'t, D>>) -> Vec<Result<'t, D>> {
            panic!("unexpected semantic operation")
        }
        fn compare(
            &mut self,
            _: &Programs,
            requests: Vec<CompareRequest<'t, D>>,
        ) -> Vec<Result<'t, D>> {
            requests
                .into_iter()
                .map(|r| {
                    Result::new(
                        match r.comparison.holds(&r.request.env) {
                            Some(true) => 2,
                            Some(false) => 0,
                            None => 1,
                        },
                        r.request.env,
                    )
                })
                .collect()
        }
    }
    mod lazy_search_cursor_tests {
        use super::*;
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};

        fn tape(source: &str, dialect: GrammarDialect) -> Root<TapeDoc> {
            let grammar = Grammar::new(dialect).unwrap();
            let image = TapeImage::parse(&grammar, source, &|| false)
                .unwrap()
                .unwrap();
            Root::doc(
                TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
            )
        }
        fn reference<'t, D: Doc<Source: Content<Underlying = u8>>>(
            rule: &AstMatcher,
            node: Node<'t, D>,
        ) -> (u8, Option<usize>, HashMap<String, String>, usize) {
            let _ = native(rule, node.clone());
            super::super::super::util::UNKNOWN.set(false);
            super::super::super::util::UTIL_STEPS_LEFT.set(usize::MAX);
            super::super::super::util::UTIL_EXCEEDED.set(None);
            let mut env = Cow::Owned(MetaVarEnv::new());
            let truth = if rule.match_node_with_env(node, &mut env).is_some() {
                2
            } else if super::super::super::util::UNKNOWN.get() {
                1
            } else {
                0
            };
            let captured = env.get_match("FOUND").map(Node::node_id);
            let work = usize::MAX - super::super::super::util::UTIL_STEPS_LEFT.get();
            (truth, captured, HashMap::from(env.into_owned()), work)
        }
        fn assert_parity<'t, D: Doc<Source: Content<Underlying = u8>>>(
            rule: &AstMatcher,
            nodes: Vec<Node<'t, D>>,
        ) {
            let expected: Vec<_> = nodes
                .iter()
                .map(|node| reference(rule, node.clone()))
                .collect();
            let mut programs = Programs::default();
            let program = programs.lower(rule);
            let requests = nodes
                .into_iter()
                .map(|node| Request {
                    program,
                    context: 0,
                    node,
                    env: Arc::new(MetaVarEnv::new()),
                    three: true,
                    each: None,
                    following: Arc::new(Vec::new()),
                })
                .collect();
            let answers = Bank::default().execute(&programs, requests, &mut SyntaxHost);
            for (answer, (truth, captured, values, work)) in answers.into_iter().zip(expected) {
                assert_eq!(answer.truth, truth);
                assert_eq!(answer.env.get_match("FOUND").map(Node::node_id), captured);
                assert_eq!(
                    HashMap::<String, String>::from((*answer.env).clone()),
                    values
                );
                assert_eq!((answer.work, answer.replay_work), (work, work));
                assert!(answer.failure.is_none());
            }
        }
        #[test]
        fn linear_search_preserves_zero_width_neighbor_anchors_and_structural_steps() {
            for (dialect, source) in [
                (GrammarDialect::Python, "x = 1\ndef f(:\n"),
                (
                    GrammarDialect::TypeScript,
                    "function f() {\n  debugger;\n  x = 1;\n",
                ),
            ] {
                let doc = tape(source, dialect);
                assert!(doc.root().dfs().any(|node| node.range().is_empty()));
                for axis in [0, 2, 3] {
                    for stop in [
                        StopBy::Neighbor,
                        StopBy::End,
                        StopBy::Rule(Box::new(AstMatcher::Regex(
                            regex::Regex::new("^[;)]$").unwrap(),
                        ))),
                        StopBy::Rule(Box::new(AstMatcher::TestUnknown(
                            regex::Regex::new("^$").unwrap(),
                        ))),
                    ] {
                        for expression in [".*", "^$"] {
                            // The capture forces generic Search rather than a closed or remembered walk.
                            let related = super::super::super::Related::new(AstMatcher::All(vec![
                                AstMatcher::Regex(regex::Regex::new(expression).unwrap()),
                                AstMatcher::Semantic(Semantic::Capture("FOUND".into())),
                            ]));
                            let rule = match axis {
                                0 => AstMatcher::Inside(related, stop.clone()),
                                2 => AstMatcher::Precedes(related, stop.clone()),
                                _ => AstMatcher::Follows(related, stop.clone()),
                            };
                            assert_parity(&rule, doc.root().dfs().collect());
                        }
                    }
                }
            }
        }
        #[test]
        fn inside_field_cursor_tracks_every_previous_ancestor_and_inclusive_stops() {
            let dialect = GrammarDialect::JavaScript;
            let doc = Root::str("function f() { if (x) { y(); } }", dialect);
            for field in ["condition", "body"] {
                for stop in [
                    StopBy::Neighbor,
                    StopBy::End,
                    StopBy::Rule(Box::new(AstMatcher::Regex(
                        regex::Regex::new("^if").unwrap(),
                    ))),
                ] {
                    let rule = AstMatcher::Field(Box::new(super::super::super::FieldRelation {
                        inside: true,
                        field: dialect.field_to_id(field).unwrap(),
                        rule: AstMatcher::All(vec![
                            AstMatcher::Regex(regex::Regex::new(".*").unwrap()),
                            AstMatcher::Semantic(Semantic::Capture("FOUND".into())),
                        ]),
                        stop,
                        filter: OnceLock::new(),
                    }));
                    assert_parity(&rule, doc.root().dfs().collect());
                }
            }
        }
        #[test]
        fn counted_linear_search_preserves_work_and_bindings_for_every_axis() {
            let doc = tape("x = 1\ndef f(:\n", GrammarDialect::Python);
            for axis in [0, 2, 3] {
                for stop in [
                    StopBy::Neighbor,
                    StopBy::End,
                    StopBy::Rule(Box::new(AstMatcher::Regex(
                        regex::Regex::new("^def").unwrap(),
                    ))),
                ] {
                    let related = super::super::super::Related::new(AstMatcher::Regex(
                        regex::Regex::new("^$").unwrap(),
                    ));
                    let relation = match axis {
                        0 => AstMatcher::Inside(related, stop.clone()),
                        2 => AstMatcher::Precedes(related, stop.clone()),
                        _ => AstMatcher::Follows(related, stop.clone()),
                    };
                    let rule = AstMatcher::Count(Box::new(super::super::super::Count {
                        relation,
                        min: 0,
                        max: None,
                        binding: Some(super::super::super::CountBinding {
                            name: "TOTAL".into(),
                            offset: 0,
                        }),
                    }));
                    assert_parity(&rule, doc.root().dfs().collect());
                }
            }
        }
        #[test]
        fn linear_walk_setup_keeps_no_eager_axis_queue() {
            let doc = Root::str("a; b; c;", GrammarDialect::JavaScript);
            let rule = AstMatcher::Precedes(
                super::super::super::Related::new(pattern("$FOUND")),
                StopBy::End,
            );
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let Op::Walk(plan) = &programs.entries[program as usize].op else {
                panic!("walk")
            };
            let search = walk_search(plan.clone(), &doc.root().child(0).unwrap(), None);
            assert!(search.queue.is_empty());
            assert!(search.linear.as_ref().unwrap().next.is_some());
        }
        #[test]
        fn projected_has_nodes_keep_queue_mode() {
            struct Projected;
            impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for Projected {
                fn evaluate(
                    &mut self,
                    _: &Programs,
                    _: Vec<SemanticRequest<'t, D>>,
                ) -> Vec<Result<'t, D>> {
                    panic!("semantic")
                }
                fn compare(
                    &mut self,
                    _: &Programs,
                    _: Vec<CompareRequest<'t, D>>,
                ) -> Vec<Result<'t, D>> {
                    panic!("compare")
                }
                fn navigate(
                    &mut self,
                    _: &Programs,
                    requests: Vec<NavigationRequest<'t, D>>,
                ) -> Vec<NavigationResult<'t, D>> {
                    requests
                        .into_iter()
                        .map(|request| NavigationResult {
                            nodes: Some(
                                request
                                    .request
                                    .node
                                    .dfs()
                                    .filter(|node| node.kind() == "identifier")
                                    .collect(),
                            ),
                            work: 0,
                            replay_work: 0,
                            levels: 0,
                            clause_state_read: false,
                            failure: None,
                        })
                        .collect()
                }
            }
            let doc = Root::str("a; b;", GrammarDialect::JavaScript);
            let rule = has(
                AstMatcher::All(vec![
                    kind("identifier"),
                    AstMatcher::Regex(regex::Regex::new("^b$").unwrap()),
                    AstMatcher::Semantic(Semantic::Capture("FOUND".into())),
                ]),
                StopBy::End,
            );
            let expected = reference(&rule, doc.root());
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: false,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let mut bank = Bank::default();
            let answer = bank
                .execute(&programs, vec![request], &mut Projected)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(answer.env.get_match("FOUND").map(Node::node_id), expected.1);
            assert_eq!(bank.metrics.navigation_projected_lanes, 1);
        }
    }

    #[test]
    fn utility_memos_do_not_retain_past_enumeration_range_sets() {
        let table = Arc::new(super::super::UtilTable::new(
            vec!["identifier".into()],
            vec![false],
            vec![None],
        ));
        assert!(table.rules.set(vec![kind("identifier")]).is_ok());
        let rule = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let exclusion = Arc::new(Exclusion {
            name: "X".into(),
            ranges: (0..1000).map(|i| (i, i + 1)).collect(),
        });
        let weak = Arc::downgrade(&exclusion);
        let request = Request {
            program,
            context: 0,
            node: doc.root(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: Some(exclusion.clone()),
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        bank.execute(&programs, vec![request], &mut SyntaxHost);
        assert!(!bank.memo.is_empty());
        assert_eq!(Arc::strong_count(&exclusion), 1);
        drop(exclusion);
        assert!(
            weak.upgrade().is_none(),
            "memo must not retain old range sets"
        );
    }
    #[test]
    fn sibling_cache_excludes_fill_work_and_recounts_non_file_documents() {
        struct FileHost(bool);
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for FileHost {
            fn is_file(&self, _: u32, _: &Node<'t, D>) -> bool {
                self.0
            }
            fn evaluate(
                &mut self,
                _: &Programs,
                _: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("unexpected semantic operation")
            }
            fn compare(
                &mut self,
                _: &Programs,
                _: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("unexpected comparison")
            }
        }
        use ast_grep_core::tree_sitter::LanguageExt;
        let doc = GrammarDialect::JavaScript.ast_grep("f(a, b, c);");
        let node = doc
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier" && node.text() == "b")
            .unwrap();
        let table = Arc::new(super::super::UtilTable::new(
            vec!["identifier".into()],
            vec![false],
            vec![None],
        ));
        assert!(table.rules.set(vec![kind("identifier")]).is_ok());
        let rule = AstMatcher::NthChild(Box::new(super::super::NthChild {
            position: super::super::NthPosition { step: 0, offset: 2 },
            of_rule: Some(AstMatcher::Util(super::super::UtilRef {
                table: Arc::downgrade(&table),
                index: 0,
            })),
            reverse: false,
            capture_free: OnceLock::new(),
        }));
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let request = Request {
            program,
            context: 0,
            node,
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        let cold = bank
            .execute(&programs, vec![request.clone()], &mut FileHost(true))
            .remove(0);
        assert_eq!((cold.truth, cold.work, cold.replay_work), (2, 7, 1));
        assert!(cold.clause_state_read);
        let hot = bank
            .execute(&programs, vec![request.clone()], &mut FileHost(true))
            .remove(0);
        assert_eq!((hot.truth, hot.work, hot.replay_work), (2, 1, 1));
        assert!(hot.clause_state_read);
        for _ in 0..2 {
            let parsed = bank
                .execute(&programs, vec![request.clone()], &mut FileHost(false))
                .remove(0);
            assert_eq!((parsed.truth, parsed.work, parsed.replay_work), (2, 7, 7));
            assert!(!parsed.clause_state_read);
        }
    }

    fn pattern(p: &str) -> AstMatcher {
        AstMatcher::Pattern(Arc::new(ast_grep_core::Pattern::new(
            p,
            GrammarDialect::JavaScript,
        )))
    }
    fn kind(k: &str) -> AstMatcher {
        AstMatcher::Kind(KindMatcher::new(k, GrammarDialect::JavaScript))
    }
    fn has(rule: AstMatcher, stop: StopBy) -> AstMatcher {
        AstMatcher::Has(super::super::Related::new(rule), stop)
    }
    fn native<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
        rule: &AstMatcher,
        node: Node<'t, D>,
    ) -> (u8, Option<HashMap<String, String>>) {
        super::super::util::THREE_VALUED.set(true);
        super::super::util::UNKNOWN.set(false);
        super::super::util::UTIL_DEPTH.set(0);
        super::super::util::UTIL_STACK_BASE.set(0);
        super::super::util::UTIL_STEPS_LEFT.set(usize::MAX);
        super::super::util::UTIL_EXCEEDED.set(None);
        let mut env = Cow::Owned(MetaVarEnv::new());
        let truth = if rule.match_node_with_env(node, &mut env).is_some() {
            2
        } else if super::super::util::UNKNOWN.get() {
            1
        } else {
            0
        };
        (truth, (truth == 2).then(|| HashMap::from(env.into_owned())))
    }
    #[test]
    fn closure_sibling_walks_preserve_zero_width_anchors_and_steps() {
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use ast_grep_core::tree_sitter::StrDoc;
        for (dialect, source) in [
            (GrammarDialect::Python, "x = 1\ndef f(:\n"),
            (
                GrammarDialect::TypeScript,
                "function f() {\n  debugger;\n  x = 1;\n",
            ),
        ] {
            let grammar = Grammar::new(dialect).unwrap();
            let native_doc = StrDoc::new(source, dialect);
            let image = TapeImage::encode(native_doc.tree.root_node(), &grammar, source, &|| false)
                .unwrap();
            let doc = Root::doc(
                TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
            );
            assert!(doc.root().dfs().any(|n| n.range().is_empty()));
            for forward in [true, false] {
                let related =
                    super::super::Related::new(AstMatcher::Regex(regex::Regex::new("^$").unwrap()));
                let rule = if forward {
                    AstMatcher::Precedes(related, StopBy::End)
                } else {
                    AstMatcher::Follows(related, StopBy::End)
                };
                let mut programs = Programs::default();
                let program = programs.lower(&rule);
                let mut requests = Vec::new();
                let mut expected = Vec::new();
                for node in doc.root().dfs() {
                    expected.push(native(&rule, node.clone()).0);
                    requests.push(Request {
                        program,
                        context: 0,
                        node,
                        env: Arc::new(MetaVarEnv::new()),
                        three: true,
                        each: None,
                        following: Arc::new(Vec::new()),
                    });
                }
                let results = Bank::default().execute(&programs, requests, &mut SyntaxHost);
                assert_eq!(
                    results.iter().map(|r| r.truth).collect::<Vec<_>>(),
                    expected,
                    "{dialect:?}, forward={forward}"
                );
            }
        }
    }
    #[test]
    fn streaming_source_refills_while_a_long_root_continues() {
        struct Streaming<'t, D: Doc<Source: Content<Underlying = u8>>> {
            pending: VecDeque<(usize, Request<'t, D>)>,
            completed: Vec<(usize, u8)>,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Source<'t, D, SyntaxHost> for Streaming<'t, D> {
            type Token = usize;
            fn next(&mut self, _: &mut SyntaxHost) -> Option<(usize, Request<'t, D>)> {
                self.pending.pop_front()
            }
            fn completed(
                &mut self,
                token: usize,
                request: Request<'t, D>,
                answer: Result<'t, D>,
                _: &mut SyntaxHost,
            ) {
                assert!(answer.failure.is_none());
                self.completed.push((token, answer.truth));
                // next() has already returned None while the long root remains.
                if token == 0 {
                    self.pending.push_back((17, request));
                }
            }
        }
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let mut programs = Programs::default();
        let short_rule = kind("expression_statement");
        let short = programs.lower(&short_rule);
        // Many primitive barriers remain long even when control returns fuse.
        let long_rule = AstMatcher::All(vec![
            AstMatcher::Regex(regex::Regex::new("^a").unwrap());
            128
        ]);
        let long = programs.lower(&long_rule);
        let mut source = Streaming {
            pending: VecDeque::new(),
            completed: Vec::new(),
        };
        for token in 0..17 {
            source.pending.push_back((
                token,
                Request {
                    program: if token == 0 { long } else { short },
                    context: 0,
                    node: doc.root().child(0).unwrap(),
                    env: Arc::new(MetaVarEnv::new()),
                    three: true,
                    each: None,
                    following: Arc::new(Vec::new()),
                },
            ));
        }
        let mut bank = Bank::default();
        bank.execute_source(&programs, &mut source, &mut SyntaxHost, 2, 0);
        assert_eq!(source.completed.len(), 18);
        assert!(source.completed.iter().all(|(_, truth)| *truth == 2));
        assert!(
            source
                .completed
                .iter()
                .position(|(token, _)| *token == 16)
                .unwrap()
                < source
                    .completed
                    .iter()
                    .position(|(token, _)| *token == 0)
                    .unwrap()
        );
        source.completed.sort_by_key(|(token, _)| *token);
        assert_eq!(
            source
                .completed
                .iter()
                .map(|(token, _)| *token)
                .collect::<Vec<_>>(),
            (0..18).collect::<Vec<_>>()
        );
        assert_eq!(bank.metrics.peak_resident_roots, 2);
        assert_eq!(bank.metrics.submitted_roots, 18);
        assert_eq!(bank.metrics.dense_counter_submissions, 1);
        assert_eq!(bank.metrics.sparse_counter_submissions, 0);
        assert!(bank.metrics.source_refills >= 16);
    }
    #[test]
    fn uncommon_nth_continuation_does_not_widen_hot_states() {
        use crate::code_rule_dsl::tape::TapeDoc;
        assert!(std::mem::size_of::<State<'_, TapeDoc>>() <= 64);
        assert!(std::mem::size_of::<Action<'_, TapeDoc>>() <= 112);
        assert!(
            std::mem::size_of::<Action<'_, TapeDoc>>() < std::mem::size_of::<Frame<'_, TapeDoc>>()
        );
        assert!(
            std::mem::size_of::<State<'_, TapeDoc>>()
                < std::mem::size_of::<NthState<'_, TapeDoc>>()
        );
    }
    #[test]
    fn kind_membership_hoists_across_patterns_but_each_checks_keep_their_positions() {
        use std::collections::HashSet;
        let doc = Root::str("f(x);", GrammarDialect::JavaScript);
        let node = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "call_expression")
            .unwrap();
        let rule = AstMatcher::All(vec![
            pattern("$A"),
            AstMatcher::Any(vec![kind("call_expression")]),
            pattern("$B"),
            AstMatcher::Any(vec![kind("call_expression")]),
            AstMatcher::Semantic(Semantic::Capture("LATER".into())),
        ]);
        let ranges: HashSet<_> = [(node.range().start, node.range().end)]
            .into_iter()
            .collect();
        let expected = super::super::each::with_exclusion(Some("B"), &ranges, &[], || {
            native(&rule, node.clone())
        });
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let Op::All(children) = &programs.entries[program as usize].op else {
            panic!("ordered stages")
        };
        assert_eq!(
            children
                .iter()
                .filter(|id| matches!(programs.entries[**id as usize].op, Op::Unexcluded))
                .count(),
            2
        );
        assert!(
            !programs.entries[program as usize]
                .guard
                .as_ref()
                .unwrap()
                .requires_unexcluded
        );
        let request = Request {
            program,
            context: 0,
            node,
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: Some(Arc::new(Exclusion {
                name: "B".into(),
                ranges: ranges.into_iter().collect(),
            })),
            following: Arc::new(Vec::new()),
        };
        let answer = Bank::default()
            .execute(&programs, vec![request], &mut SyntaxHost)
            .pop()
            .unwrap();
        assert_eq!(answer.truth, expected.0);
        assert_eq!(answer.truth, 0);
        assert!(answer.env.get_match("LATER").is_none());
        assert!(answer.env.get_match("A").is_none());
        assert!(answer.env.get_match("B").is_none());
        assert_eq!(answer.work, 0);
    }
    #[test]
    fn pattern_kind_hoist_preserves_prebound_conflicts_and_template_error_patterns() {
        use ast_grep_core::{
            MatchStrictness,
            matcher::{Pattern, PatternNode},
        };
        let doc = Root::str("f(x);", GrammarDialect::JavaScript);
        let call = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "call_expression")
            .unwrap();
        let identifier = doc.root().dfs().find(|n| n.text() == "x").unwrap();
        for required in ["call_expression", "identifier"] {
            let rule = AstMatcher::All(vec![pattern("$X"), kind(required)]);
            let mut input = MetaVarEnv::new();
            input.insert("X", identifier.clone()).unwrap();
            super::super::util::THREE_VALUED.set(true);
            super::super::util::UNKNOWN.set(false);
            super::super::util::UTIL_STEPS_LEFT.set(usize::MAX);
            super::super::util::UTIL_EXCEEDED.set(None);
            let mut expected = Cow::Owned(input.clone());
            let matched = rule
                .match_node_with_env(call.clone(), &mut expected)
                .is_some();
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            assert!(programs.entries[program as usize].guard.is_some());
            let request = Request {
                program,
                context: 0,
                node: call.clone(),
                env: Arc::new(input),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth == 2, matched);
            assert_eq!(
                HashMap::<String, String>::from((*answer.env).clone()),
                HashMap::<String, String>::from(expected.into_owned())
            );
        }
        let template = Pattern::new("f($X)", GrammarDialect::JavaScript)
            .with_strictness(MatchStrictness::Template);
        let mut error = Pattern::new("x", GrammarDialect::JavaScript);
        error.node = PatternNode::Terminal {
            text: "x".into(),
            is_named: true,
            kind_id: u16::MAX,
        };
        for pattern in [template, error] {
            for required in ["call_expression", "identifier"] {
                let rule = AstMatcher::All(vec![
                    AstMatcher::Pattern(Arc::new(pattern.clone())),
                    AstMatcher::Regex(regex::Regex::new(".").unwrap()),
                    kind(required),
                ]);
                let mut programs = Programs::default();
                let program = programs.lower(&rule);
                for node in doc.root().dfs() {
                    let expected = native(&rule, node.clone());
                    let request = Request {
                        program,
                        context: 0,
                        node,
                        env: Arc::new(MetaVarEnv::new()),
                        three: true,
                        each: None,
                        following: Arc::new(Vec::new()),
                    };
                    let answer = Bank::default()
                        .execute(&programs, vec![request], &mut SyntaxHost)
                        .pop()
                        .unwrap();
                    assert_eq!(answer.truth, expected.0);
                    assert_eq!(
                        (answer.truth == 2)
                            .then(|| HashMap::<String, String>::from((*answer.env).clone())),
                        expected.1
                    );
                }
            }
        }
    }
    #[test]
    fn pattern_kind_hoist_keeps_enclosing_count_and_utility_work_exact() {
        let doc = Root::str("f(x);", GrammarDialect::JavaScript);
        let body = AstMatcher::All(vec![
            pattern("$X"),
            AstMatcher::Regex(regex::Regex::new(".").unwrap()),
            AstMatcher::Any(vec![kind("identifier")]),
        ]);
        let table = Arc::new(super::super::UtilTable::new(
            vec!["guarded".into()],
            vec![false],
            vec![None],
        ));
        assert!(table.rules.set(vec![body.clone()]).is_ok());
        let rules = [
            AstMatcher::Count(Box::new(super::super::Count {
                relation: has(body, StopBy::End),
                min: 1,
                max: None,
                binding: Some(super::super::CountBinding {
                    name: "TOTAL".into(),
                    offset: 0,
                }),
            })),
            AstMatcher::Util(super::super::UtilRef {
                table: Arc::downgrade(&table),
                index: 0,
            }),
        ];
        for rule in rules {
            let expected = native(&rule, doc.root());
            let work = usize::MAX - super::super::util::UTIL_STEPS_LEFT.get();
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(answer.work, work);
            assert_eq!(answer.replay_work, work);
            assert_eq!(
                (answer.truth == 2).then(|| HashMap::<String, String>::from((*answer.env).clone())),
                expected.1
            );
        }
    }
    #[test]
    fn pure_kind_guard_empty_bodies_are_atomic_and_keep_zero_arity_truth() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let rules = [
            AstMatcher::All(Vec::new()),
            AstMatcher::Any(Vec::new()),
            AstMatcher::All(vec![kind("program")]),
            AstMatcher::All(vec![AstMatcher::Any(Vec::new())]),
        ];
        for (index, rule) in rules.into_iter().enumerate() {
            let expected = native(&rule, doc.root());
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            if index != 1 {
                assert!(matches!(programs.entries[program as usize].op, Op::Pass));
                assert!(programs.entries[program as usize].closed.is_none());
            }
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(answer.work, 0);
        }
    }
    #[test]
    fn kind_guards_preserve_unknown_order_and_never_cross_utility_work() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let node = doc.root().child(0).unwrap();
        let table = Arc::new(super::super::UtilTable::new(
            vec!["charged".into()],
            vec![false],
            vec![None],
        ));
        assert!(table.rules.set(vec![kind("expression_statement")]).is_ok());
        let util = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        let unknown = || AstMatcher::TestUnknown(regex::Regex::new(".").unwrap());
        let rules = [
            AstMatcher::All(vec![unknown(), kind("identifier")]),
            AstMatcher::All(vec![kind("identifier"), unknown()]),
            AstMatcher::All(vec![kind("expression_statement"), unknown()]),
            AstMatcher::All(vec![util.clone(), kind("identifier")]),
            AstMatcher::All(vec![kind("identifier"), util]),
            AstMatcher::All(vec![pattern("$X"), kind("identifier")]),
        ];
        for (index, rule) in rules.into_iter().enumerate() {
            let expected = native(&rule, node.clone());
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            if [0, 3].contains(&index) {
                assert!(programs.entries[program as usize].guard.is_none());
            }
            let request = Request {
                program,
                context: 0,
                node: node.clone(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(answer.work, usize::from(index == 3));
        }
    }
    #[test]
    fn kind_guards_keep_any_exclusions_and_skip_only_known_false_alternatives() {
        use std::collections::HashSet;
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let node = doc.root().child(0).unwrap();
        let rules = [
            AstMatcher::All(vec![
                AstMatcher::Any(vec![kind("expression_statement")]),
                AstMatcher::Semantic(Semantic::Capture("NEW".into())),
            ]),
            AstMatcher::Any(vec![
                AstMatcher::All(vec![kind("identifier"), pattern("$FIRST")]),
                AstMatcher::All(vec![
                    kind("expression_statement"),
                    AstMatcher::Semantic(Semantic::Capture("SECOND".into())),
                ]),
                pattern("$THIRD"),
            ]),
            AstMatcher::Any(vec![AstMatcher::All(vec![
                kind("expression_statement"),
                pattern("$FRESH"),
            ])]),
        ];
        for (index, rule) in rules.into_iter().enumerate() {
            let ranges: HashSet<_> = [(node.range().start, node.range().end)]
                .into_iter()
                .collect();
            let mut input = MetaVarEnv::new();
            input.insert("OLD", node.clone()).unwrap();
            let expected = super::super::each::with_exclusion(Some("OLD"), &ranges, &[], || {
                super::super::util::THREE_VALUED.set(true);
                super::super::util::UNKNOWN.set(false);
                super::super::util::UTIL_STEPS_LEFT.set(usize::MAX);
                super::super::util::UTIL_EXCEEDED.set(None);
                let mut env = Cow::Owned(input.clone());
                let truth = if rule.match_node_with_env(node.clone(), &mut env).is_some() {
                    2
                } else if super::super::util::UNKNOWN.get() {
                    1
                } else {
                    0
                };
                (truth, HashMap::<String, String>::from(env.into_owned()))
            });
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let request = Request {
                program,
                context: 0,
                node: node.clone(),
                env: Arc::new(input),
                three: true,
                each: Some(Arc::new(Exclusion {
                    name: "OLD".into(),
                    ranges: ranges.into_iter().collect(),
                })),
                following: Arc::new(Vec::new()),
            };
            let mut bank = Bank::default();
            let answer = bank
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(
                HashMap::<String, String>::from((*answer.env).clone()),
                expected.1
            );
            if index == 1 {
                assert!(bank.metrics.kind_guard_alternative_skips > 0);
            }
            if index == 0 {
                assert!(bank.metrics.kind_guard_rejects > 0);
            }
        }
    }
    #[test]
    fn kind_guards_do_not_bypass_unknown_capture_withholding_or_strict_any() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let node = doc.root().child(0).unwrap();
        let uncertain = || {
            AstMatcher::Constrained(Box::new(super::super::Constrained {
                rule: pattern("$A"),
                constraints: vec![(
                    "A".into(),
                    AstMatcher::TestUnknown(regex::Regex::new(".").unwrap()),
                )],
            }))
        };
        let rules = [
            AstMatcher::All(vec![
                uncertain(),
                AstMatcher::All(vec![kind("identifier"), pattern("$A")]),
            ]),
            AstMatcher::Any(vec![
                uncertain(),
                AstMatcher::All(vec![
                    kind("expression_statement"),
                    AstMatcher::Semantic(Semantic::Capture("LATER".into())),
                ]),
            ]),
        ];
        for rule in rules {
            let expected = native(&rule, node.clone());
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let request = Request {
                program,
                context: 0,
                node: node.clone(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(answer.truth, 1);
            assert!(answer.env.get_match("LATER").is_none());
        }
    }
    #[test]
    fn guarded_count_keeps_successful_capture_and_rejects_before_charged_work() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        for accepted in [true, false] {
            let rule = AstMatcher::All(vec![
                kind(if accepted { "program" } else { "identifier" }),
                AstMatcher::Count(Box::new(super::super::Count {
                    relation: has(kind("identifier"), StopBy::End),
                    min: 1,
                    max: None,
                    binding: Some(super::super::CountBinding {
                        name: "TOTAL".into(),
                        offset: 0,
                    }),
                })),
            ]);
            let expected = native(&rule, doc.root());
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            assert!(matches!(
                programs.entries[program as usize].op,
                Op::Count { .. }
            ));
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let mut bank = Bank::default();
            let answer = bank
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(
                (answer.truth == 2).then(|| HashMap::<String, String>::from((*answer.env).clone())),
                expected.1
            );
            if accepted {
                assert!(answer.work > 0);
                assert!(answer.env.get_transformed("TOTAL").is_some());
            } else {
                assert_eq!(answer.work, 0);
                assert!(answer.env.get_transformed("TOTAL").is_none());
                assert!(bank.metrics.kind_guard_rejects > 0);
            }
        }
    }
    #[test]
    fn guarded_atomic_aliases_obey_guards_when_inlined_and_keep_nth_transactions() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let node = doc.root().child(0).unwrap();
        let guarded = AstMatcher::All(vec![
            kind("identifier"),
            AstMatcher::Semantic(Semantic::Capture("LEAK".into())),
        ]);
        let rule = AstMatcher::Any(vec![
            guarded,
            AstMatcher::Semantic(Semantic::Capture("GOOD".into())),
        ]);
        let nth = AstMatcher::All(vec![
            kind("expression_statement"),
            AstMatcher::NthChild(Box::new(super::super::NthChild {
                position: super::super::NthPosition { step: 0, offset: 0 },
                of_rule: Some(pattern("$LEAK")),
                reverse: false,
                capture_free: OnceLock::new(),
            })),
        ]);
        for rule in [rule, nth] {
            let expected = native(&rule, node.clone());
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let request = Request {
                program,
                context: 0,
                node: node.clone(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert!(answer.env.get_match("LEAK").is_none());
        }
        // An All continuation inlines this Capture alias directly, bypassing Start.
        let rule = AstMatcher::All(vec![
            AstMatcher::Semantic(Semantic::Capture("PREFIX".into())),
            AstMatcher::All(vec![
                kind("identifier"),
                AstMatcher::Semantic(Semantic::Capture("LEAK".into())),
            ]),
        ]);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let request = Request {
            program,
            context: 0,
            node,
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        let answer = bank
            .execute(&programs, vec![request], &mut SyntaxHost)
            .pop()
            .unwrap();
        assert_eq!(answer.truth, 0);
        assert!(answer.env.get_match("LEAK").is_none());
        assert!(answer.env.get_match("PREFIX").is_none());
        assert!(bank.metrics.kind_guard_rejects > 0);
    }
    #[test]
    fn adaptive_control_counters_record_exactly_and_drain_only_touched_programs() {
        let mut counts = AdvanceCounts::default();
        counts.prepare(CounterStorage::Sparse, 100_000);
        counts.record(99_999);
        counts.record(99_999);
        counts.record(7);
        let mut actual = counts.drain();
        actual.sort_unstable();
        assert_eq!(actual, vec![(7, 1), (99_999, 2)]);
        assert!(matches!(&counts, AdvanceCounts::Sparse(values) if values.is_empty()));
        counts.prepare(CounterStorage::Dense, 100_000);
        counts.record(99_999);
        counts.record(7);
        counts.record(7);
        let mut actual = counts.drain();
        actual.sort_unstable();
        assert_eq!(actual, vec![(7, 2), (99_999, 1)]);
        assert!(
            matches!(&counts, AdvanceCounts::Dense { values, touched } if touched.is_empty() && values[7] == 0 && values[99_999] == 0)
        );
        counts.prepare(CounterStorage::Dense, 100_000);
        counts.record(7);
        assert_eq!(counts.drain(), vec![(7, 1)]);
        counts.prepare(CounterStorage::Sparse, 100_000);
        assert!(counts.drain().is_empty());
    }
    #[test]
    fn control_inventory_groups_programs_by_bounded_operation_shapes() {
        let doc = Root::str("a;", GrammarDialect::JavaScript);
        let rule = AstMatcher::All(vec![
            AstMatcher::Semantic(Semantic::Capture("ROOT".into())),
            AstMatcher::Any(vec![
                kind("expression_statement"),
                AstMatcher::Regex(regex::Regex::new("^x").unwrap()),
            ]),
        ]);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        assert_eq!(programs.control_descriptor(program), "All[Capture,Any]");
        let request = Request {
            program,
            context: 0,
            node: doc.root().child(0).unwrap(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        assert_eq!(
            bank.execute(&programs, vec![request], &mut SyntaxHost)[0].truth,
            2
        );
        assert_eq!(bank.metrics.sparse_counter_submissions, 1);
        assert_eq!(bank.metrics.dense_counter_submissions, 0);
        let total: usize = bank.metrics.control_family_lanes.iter().sum();
        assert_eq!(bank.metrics.control_programs.values().sum::<usize>(), total);
        assert!(
            bank.metrics
                .control_programs
                .contains_key("All[Capture,Any]")
        );
        assert!(
            bank.metrics
                .control_programs
                .keys()
                .all(|shape| shape.len() < 160)
        );
        let mut merged = Metrics::default();
        merged.merge(&bank.metrics);
        merged.merge(&bank.metrics);
        assert_eq!(merged.control_programs.values().sum::<usize>(), 2 * total);
    }
    #[test]
    fn transactional_singleton_all_preserves_unknown_each_and_nth_rollback() {
        use std::collections::HashSet;
        let doc = Root::str("f(x);", GrammarDialect::JavaScript);
        let node = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "call_expression")
            .unwrap();
        let rules = [
            AstMatcher::All(vec![AstMatcher::Not(Box::new(AstMatcher::TestUnknown(
                regex::Regex::new(".").unwrap(),
            )))]),
            AstMatcher::All(vec![AstMatcher::Any(vec![AstMatcher::Semantic(
                Semantic::Capture("ROOT".into()),
            )])]),
            AstMatcher::All(vec![AstMatcher::All(vec![
                pattern("$F($A)"),
                AstMatcher::TestUnknown(regex::Regex::new(".").unwrap()),
            ])]),
        ];
        for rule in rules {
            let ranges: HashSet<_> = [(node.range().start, node.range().end)]
                .into_iter()
                .collect();
            let expected = super::super::each::with_exclusion(Some("ROOT"), &ranges, &[], || {
                native(&rule, node.clone())
            });
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let request = Request {
                program,
                context: 0,
                node: node.clone(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: Some(Arc::new(Exclusion {
                    name: "ROOT".into(),
                    ranges: ranges.into_iter().collect(),
                })),
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(&programs, vec![request], &mut SyntaxHost)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, expected.0);
            assert_eq!(
                (answer.truth == 2).then(|| HashMap::<String, String>::from((*answer.env).clone())),
                expected.1
            );
            assert_eq!(programs.lookup(&rule), Some(program));
        }
        let rule = AstMatcher::All(vec![AstMatcher::NthChild(Box::new(
            super::super::NthChild {
                position: super::super::NthPosition { step: 0, offset: 0 },
                of_rule: Some(pattern("$LEAK")),
                reverse: false,
                capture_free: OnceLock::new(),
            },
        ))]);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        assert!(matches!(
            programs.entries[program as usize].op,
            Op::Nth { .. }
        ));
        assert!(programs.entries[program as usize].rollback_nontrue);
        let request = Request {
            program,
            context: 0,
            node: doc.root().child(0).unwrap(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let answer = Bank::default()
            .execute(&programs, vec![request], &mut SyntaxHost)
            .pop()
            .unwrap();
        assert_eq!(answer.truth, 0);
        assert!(answer.env.get_match("LEAK").is_none());
    }
    #[test]
    fn singleton_transactions_roll_back_cold_nth_and_walk_false_frames() {
        let doc = Root::str("a; b;", GrammarDialect::JavaScript);
        let nth = AstMatcher::NthChild(Box::new(super::super::NthChild {
            position: super::super::NthPosition { step: 0, offset: 0 },
            of_rule: Some(pattern("$LEAK")),
            reverse: false,
            capture_free: OnceLock::new(),
        }));
        let table = Arc::new(super::super::UtilTable::new(
            vec!["leak".into()],
            vec![false],
            vec![None],
        ));
        assert!(table.rules.set(vec![nth.clone()]).is_ok());
        let util = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        for (bare, node) in [
            (nth.clone(), doc.root().child(0).unwrap()),
            (has(nth, StopBy::Neighbor), doc.root()),
            (util, doc.root().child(0).unwrap()),
        ] {
            let wrapped = AstMatcher::All(vec![bare.clone()]);
            let mut programs = Programs::default();
            let direct = programs.lower(&bare);
            let transaction = programs.lower(&wrapped);
            assert!(!programs.entries[direct as usize].rollback_nontrue);
            assert!(programs.entries[transaction as usize].rollback_nontrue);
            let requests = [direct, transaction]
                .into_iter()
                .map(|program| Request {
                    program,
                    context: 0,
                    node: node.clone(),
                    env: Arc::new(MetaVarEnv::new()),
                    three: true,
                    each: None,
                    following: Arc::new(Vec::new()),
                })
                .collect();
            let answers = Bank::default().execute(&programs, requests, &mut SyntaxHost);
            assert_eq!((answers[0].truth, answers[1].truth), (0, 0));
            assert!(answers[0].env.get_match("LEAK").is_some());
            assert!(answers[1].env.get_match("LEAK").is_none());
            assert_eq!(
                (answers[0].work, answers[0].replay_work, answers[0].levels),
                (answers[1].work, answers[1].replay_work, answers[1].levels)
            );
        }
    }
    #[test]
    fn singleton_walk_transaction_survives_failure_after_a_false_capture() {
        struct Limited {
            left: usize,
            failed: bool,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for Limited {
            fn evaluate(
                &mut self,
                _: &Programs,
                _: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("semantic")
            }
            fn compare(
                &mut self,
                _: &Programs,
                _: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("compare")
            }
            fn spend(&mut self, _: u32, amount: usize) -> bool {
                if amount > self.left {
                    self.failed = true;
                    false
                } else {
                    self.left -= amount;
                    true
                }
            }
            fn failure(&mut self, _: u32) -> Option<Failure> {
                self.failed.then_some(Failure::Utility(UtilLimit::Work))
            }
        }
        let doc = Root::str("a; b;", GrammarDialect::JavaScript);
        let bare = has(
            AstMatcher::NthChild(Box::new(super::super::NthChild {
                position: super::super::NthPosition { step: 0, offset: 0 },
                of_rule: Some(pattern("$LEAK")),
                reverse: false,
                capture_free: OnceLock::new(),
            })),
            StopBy::Neighbor,
        );
        let wrapped = AstMatcher::All(vec![bare.clone()]);
        let mut programs = Programs::default();
        let entries = [programs.lower(&bare), programs.lower(&wrapped)];
        for (index, program) in entries.into_iter().enumerate() {
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute(
                    &programs,
                    vec![request],
                    &mut Limited {
                        left: 2,
                        failed: false,
                    },
                )
                .pop()
                .unwrap();
            assert_eq!(answer.failure, Some(Failure::Utility(UtilLimit::Work)));
            assert_eq!(answer.work, 2);
            assert_eq!(answer.env.get_match("LEAK").is_some(), index == 0);
        }
        let table = Arc::new(super::super::UtilTable::new(
            vec!["depth".into()],
            vec![false],
            vec![None],
        ));
        assert!(table.rules.set(vec![kind("identifier")]).is_ok());
        let bare = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        let wrapped = AstMatcher::All(vec![bare.clone()]);
        let mut programs = Programs::default();
        let entries = [programs.lower(&bare), programs.lower(&wrapped)];
        for program in entries {
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let answer = Bank::default()
                .execute_at(
                    &programs,
                    vec![request],
                    &mut SyntaxHost,
                    super::super::util::MAX_UTIL_DEPTH,
                )
                .pop()
                .unwrap();
            assert_eq!(answer.failure, Some(Failure::Utility(UtilLimit::Depth)));
            assert_eq!(answer.work, 0);
            assert!(answer.env.get_match("LEAK").is_none());
        }
    }
    #[test]
    fn failed_atomic_answers_cannot_expose_captures_through_singleton_aliases() {
        use crate::code_rule_dsl::test_support::{compile_typescript_ast, grammar_matchers};
        let compiled=compile_typescript_ast(serde_json::json!({"rule":{"pattern":"f($X)"},"constraints":{"X":{"value":{"constant":true}}}})).unwrap();
        let grammar = &grammar_matchers(&compiled)[0];
        let AstMatcher::Constrained(core) = &grammar.matcher else {
            panic!("constrained")
        };
        let atomic = core.constraints[0].1.clone();
        let wrapped = AstMatcher::All(vec![atomic.clone()]);
        struct Failed;
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for Failed {
            fn evaluate(
                &mut self,
                _: &Programs,
                jobs: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                jobs.into_iter()
                    .map(|job| {
                        let mut env = (*job.request.env).clone();
                        env.insert("LEAK", job.request.node).unwrap();
                        let mut answer = Result::new(2, Arc::new(env));
                        answer.work = 3;
                        answer.replay_work = 1;
                        answer.levels = 2;
                        answer.failure = Some(Failure::Utility(UtilLimit::Work));
                        answer
                    })
                    .collect()
            }
            fn compare(
                &mut self,
                _: &Programs,
                _: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("compare")
            }
        }
        let doc = Root::str("x;", grammar.language);
        let mut programs = Programs::default();
        let entries = [programs.lower(&atomic), programs.lower(&wrapped)];
        let requests = entries
            .into_iter()
            .map(|program| Request {
                program,
                context: 0,
                node: doc.root().child(0).unwrap(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            })
            .collect();
        for answer in Bank::default().execute(&programs, requests, &mut Failed) {
            assert_eq!(answer.truth, 0);
            assert!(answer.env.get_match("LEAK").is_none());
            assert_eq!((answer.work, answer.replay_work, answer.levels), (3, 1, 2));
            assert_eq!(answer.failure, Some(Failure::Utility(UtilLimit::Work)));
        }
    }
    #[test]
    fn singleton_all_normalization_preserves_transactional_program_identities() {
        let eligible = AstMatcher::Constrained(Box::new(super::super::Constrained {
            rule: pattern("f($X)"),
            constraints: vec![("X".into(), kind("identifier"))],
        }));
        let wrapped = AstMatcher::All(vec![eligible]);
        let generic = AstMatcher::All(vec![AstMatcher::Constrained(Box::new(
            super::super::Constrained {
                rule: pattern("f($X)"),
                constraints: vec![("X".into(), AstMatcher::Any(vec![kind("identifier")]))],
            },
        ))]);
        let ordinary = AstMatcher::All(vec![kind("identifier")]);
        let mut programs = Programs::default();
        let optimized = programs.lower(&wrapped);
        let unoptimized = programs.lower(&generic);
        let atomic = programs.lower(&ordinary);
        assert!(matches!(
            programs.entries[optimized as usize].op,
            Op::Constrained { .. }
        ));
        assert!(programs.entries[optimized as usize].pipeline);
        assert!(matches!(
            programs.entries[unoptimized as usize].op,
            Op::Constrained { .. }
        ));
        assert!(matches!(&programs.entries[atomic as usize].op, Op::Pass));
        assert_eq!(programs.lookup(&wrapped), Some(optimized));
        assert_eq!(programs.control_descriptor(atomic), "Guard:Pass");
    }
    #[test]
    fn constrained_pipeline_preserves_native_captures_unknowns_and_each() {
        let doc = Root::str("f(x); g(y);", GrammarDialect::JavaScript);
        for unknown in [false, true] {
            let rule = AstMatcher::Constrained(Box::new(super::super::Constrained {
                rule: pattern("$F($A)"),
                constraints: vec![
                    (
                        "F".into(),
                        AstMatcher::Semantic(Semantic::Capture("BOUND".into())),
                    ),
                    (
                        "BOUND".into(),
                        AstMatcher::Regex(regex::Regex::new("^f$").unwrap()),
                    ),
                    (
                        "A".into(),
                        if unknown {
                            AstMatcher::TestUnknown(regex::Regex::new("x").unwrap())
                        } else {
                            kind("identifier")
                        },
                    ),
                    ("ABSENT".into(), AstMatcher::Never),
                ],
            }));
            let rule = AstMatcher::All(vec![rule]);
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            assert!(programs.entries[program as usize].pipeline);
            assert!(matches!(
                programs.entries[program as usize].op,
                Op::Constrained { .. }
            ));
            assert_eq!(programs.lookup(&rule), Some(program));
            let nodes: Vec<_> = doc
                .root()
                .dfs()
                .filter(|n| n.kind() == "call_expression")
                .collect();
            let expected: Vec<_> = nodes.iter().map(|n| native(&rule, n.clone())).collect();
            for each in [
                None,
                Some(Arc::new(Exclusion {
                    name: "BOUND".into(),
                    ranges: [(0, 1)].into_iter().collect(),
                })),
            ] {
                let requests = nodes
                    .iter()
                    .map(|node| Request {
                        program,
                        context: 0,
                        node: node.clone(),
                        env: Arc::new(MetaVarEnv::new()),
                        three: true,
                        each: each.clone(),
                        following: Arc::new(Vec::new()),
                    })
                    .collect();
                let mut bank = Bank::default();
                let results = bank.execute(&programs, requests, &mut SyntaxHost);
                for (result, expected) in results.iter().zip(&expected) {
                    assert_eq!(result.truth, expected.0);
                    assert_eq!(
                        (result.truth == 2)
                            .then(|| HashMap::<String, String>::from((*result.env).clone())),
                        expected.1
                    );
                    if result.truth != 2 {
                        assert!(result.env.get_match("BOUND").is_none());
                    }
                }
                assert_eq!(bank.metrics.pipeline_rows, 2);
                assert!(bank.metrics.pipeline_eliminated_frames >= 6);
                assert_eq!(bank.metrics.operation_family_lanes[3], 0);
                assert_eq!(bank.metrics.operation_family_lanes[0], 0);
            }
        }
    }
    #[test]
    fn constrained_pipeline_forwards_semantic_context_and_exact_costs() {
        use crate::code_rule_dsl::test_support::{compile_typescript_ast, grammar_matchers};
        let compiled = compile_typescript_ast(serde_json::json!({
            "rule": {"pattern":"f($X)"}, "constraints":{"X":{"value":{"constant":true}}}
        }))
        .unwrap();
        let grammar = &grammar_matchers(&compiled)[0];
        let doc = Root::str("f(x);", grammar.language);
        let rule = AstMatcher::All(vec![grammar.matcher.clone()]);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        assert!(programs.entries[program as usize].pipeline);
        assert!(matches!(
            programs.entries[program as usize].op,
            Op::Constrained { .. }
        ));
        struct Probe {
            truth: u8,
            failure: bool,
            seen: usize,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for Probe {
            fn evaluate(
                &mut self,
                _: &Programs,
                jobs: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                jobs.into_iter()
                    .map(|job| {
                        self.seen += 1;
                        assert_eq!(job.utility_depth, 3);
                        assert_eq!(job.request.each.as_ref().unwrap().name, "X");
                        assert_eq!(job.request.following.as_ref(), &[(17, 19)]);
                        assert_eq!(job.request.node.text(), "x");
                        let mut answer = Result::new(self.truth, job.request.env);
                        answer.work = 3;
                        answer.replay_work = 1;
                        answer.levels = 2;
                        answer.clause_state_read = true;
                        answer.failure = self.failure.then_some(Failure::Utility(UtilLimit::Work));
                        answer
                    })
                    .collect()
            }
            fn compare(
                &mut self,
                _: &Programs,
                _: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("compare")
            }
        }
        let node = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "call_expression")
            .unwrap();
        for (truth, failure) in [(2, false), (1, false), (0, false), (0, true)] {
            let request = Request {
                program,
                context: 9,
                node: node.clone(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: Some(Arc::new(Exclusion {
                    name: "X".into(),
                    ranges: BTreeSet::new(),
                })),
                following: Arc::new(vec![(17, 19)]),
            };
            let mut host = Probe {
                truth,
                failure,
                seen: 0,
            };
            let mut bank = Bank::default();
            let answer = bank
                .execute_at(&programs, vec![request], &mut host, 3)
                .pop()
                .unwrap();
            assert_eq!(answer.truth, truth);
            assert_eq!(host.seen, 1);
            assert_eq!(
                (
                    answer.work,
                    answer.replay_work,
                    answer.levels,
                    answer.clause_state_read
                ),
                (3, 1, 2, true)
            );
            assert_eq!(
                answer.failure,
                failure.then_some(Failure::Utility(UtilLimit::Work))
            );
            assert_eq!(answer.env.get_match("X").is_some(), truth == 2);
            assert_eq!(bank.metrics.pipeline_eliminated_frames, 2);
        }
    }
    fn region_tape(source: &str) -> Root<crate::code_rule_dsl::tape::TapeDoc> {
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
        let grammar = Grammar::new(GrammarDialect::JavaScript).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        Root::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        )
    }
    #[test]
    fn closed_regions_preserve_captures_and_batch_documents_and_programs() {
        let docs = [region_tape("a;"), region_tape("b;")];
        let rules: Vec<_> = ["^a", "^b"]
            .into_iter()
            .map(|text| {
                AstMatcher::All(vec![
                    AstMatcher::Semantic(Semantic::Capture("X".into())),
                    AstMatcher::All(vec![
                        kind("expression_statement"),
                        has(
                            AstMatcher::All(vec![
                                kind("identifier"),
                                AstMatcher::Regex(regex::Regex::new(text).unwrap()),
                            ]),
                            StopBy::End,
                        ),
                    ]),
                ])
            })
            .collect();
        let mut programs = Programs::default();
        let entries: Vec<_> = rules.iter().map(|r| programs.lower(r)).collect();
        let requests = docs
            .iter()
            .enumerate()
            .map(|(i, doc)| Request {
                program: entries[i],
                context: i as u32,
                node: doc.root().child(0).unwrap(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            })
            .collect();
        let mut bank = Bank::default();
        let results = bank.execute(&programs, requests, &mut SyntaxHost);
        assert!(
            results
                .iter()
                .all(|r| r.truth == 2 && r.failure.is_none() && r.env.get_match("X").is_some())
        );
        assert_eq!(bank.metrics.region_roots, 2);
        assert!(bank.metrics.region_kernel.multi_document_submissions > 0);
        assert!(bank.metrics.region_kernel.multi_program_submissions > 0);
        // Only capture-owning outer Alls receive structural startup frames.
        assert_eq!(bank.metrics.operation_family_lanes[0], 2);
    }
    #[test]
    fn closed_regions_batch_same_owner_roots_and_retain_postings() {
        let doc = region_tape("f(a); g(b); h(c); i(d);");
        let rule = has(kind("identifier"), StopBy::End);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let request = Request {
            program,
            context: 0,
            node: doc.root(),
            env: Arc::new(MetaVarEnv::new()),
            three: false,
            each: None,
            following: Arc::new(Vec::new()),
        };
        assert!(programs.requires_order(program));
        assert!(!programs.requires_order_with_regions(program, true));
        let requests = doc
            .root()
            .children()
            .map(|node| {
                let mut request = request.clone();
                request.node = node;
                request
            })
            .collect();
        let mut bank = Bank::default();
        let results = bank.execute(&programs, requests, &mut SyntaxHost);
        assert!(results.iter().all(|r| r.truth == 2 && r.failure.is_none()));
        assert_eq!(bank.metrics.region_roots, 4);
        assert_eq!(bank.metrics.region_kernel.max_kernel_width, 4);
        assert_eq!(bank.metrics.region_posting_builds, 1);
        assert_eq!(bank.metrics.operation_family_lanes[4], 0);
    }
    #[test]
    fn closed_region_utility_limits_fall_back_to_normal_cohort_execution() {
        struct RegionHost {
            left: usize,
            spent: usize,
            cancelled: bool,
            failure: Option<Failure>,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for RegionHost {
            fn evaluate(
                &mut self,
                _: &Programs,
                _: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("semantic")
            }
            fn compare(
                &mut self,
                _: &Programs,
                _: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("compare")
            }
            fn can_remember(&self, _: u32, _: &Node<'t, D>) -> bool {
                true
            }
            fn region_allowance(&self, _: u32) -> usize {
                self.left
            }
            fn region_cancelled(&self, _: u32) -> bool {
                self.cancelled
            }
            fn failure(&mut self, _: u32) -> Option<Failure> {
                if self.cancelled {
                    Some(Failure::Cancelled)
                } else {
                    self.failure
                }
            }
            fn spend(&mut self, _: u32, amount: usize) -> bool {
                if amount > self.left {
                    self.failure = Some(Failure::Utility(UtilLimit::Work));
                    false
                } else {
                    self.left -= amount;
                    self.spent += amount;
                    true
                }
            }
        }
        let table = Arc::new(super::super::UtilTable::new(
            vec!["closed".into()],
            vec![false],
            vec![Some(super::super::util::UtilOrigin {
                digest: [17; 32],
                definition: Arc::from(&b"region-test-utility"[..]),
            })],
        ));
        assert!(table.rules.set(vec![kind("expression_statement")]).is_ok());
        let rule = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        let doc = region_tape("a;");
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        assert!(programs.entries[program as usize].closed.is_some());
        let request = Request {
            program,
            context: 0,
            node: doc.root().child(0).unwrap(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut host = RegionHost {
            left: 10,
            spent: 0,
            cancelled: false,
            failure: None,
        };
        let mut bank = Bank::default();
        let result = bank
            .execute(&programs, vec![request.clone()], &mut host)
            .pop()
            .unwrap();
        assert_eq!(
            (result.truth, result.work, result.replay_work, result.levels),
            (2, 1, 1, 1)
        );
        assert_eq!(host.spent, 1);
        assert_eq!(bank.metrics.region_roots, 1);
        // Publication checks the current allowance after each same-owner root.
        let mut host = RegionHost {
            left: 1,
            spent: 0,
            cancelled: false,
            failure: None,
        };
        let mut bank = Bank::default();
        let results = bank.execute(&programs, vec![request.clone(), request.clone()], &mut host);
        assert_eq!(results[0].truth, 2);
        assert_eq!(results[1].failure, Some(Failure::Utility(UtilLimit::Work)));
        assert_eq!(host.spent, 1);
        assert_eq!(bank.metrics.region_roots, 1);
        assert_eq!(bank.metrics.region_fallbacks, 1);
        let mut host = RegionHost {
            left: 0,
            spent: 0,
            cancelled: false,
            failure: None,
        };
        let mut bank = Bank::default();
        let result = bank
            .execute(&programs, vec![request.clone()], &mut host)
            .pop()
            .unwrap();
        assert_eq!(result.failure, Some(Failure::Utility(UtilLimit::Work)));
        assert_eq!(bank.metrics.region_fallbacks, 1);
        let mut host = RegionHost {
            left: 10,
            spent: 0,
            cancelled: false,
            failure: None,
        };
        let mut bank = Bank::default();
        let result = bank
            .execute_at(
                &programs,
                vec![request.clone()],
                &mut host,
                super::super::util::MAX_UTIL_DEPTH,
            )
            .pop()
            .unwrap();
        assert_eq!(result.failure, Some(Failure::Utility(UtilLimit::Depth)));
        assert_eq!(host.spent, 0);
        assert_eq!(bank.metrics.region_fallbacks, 1);
        host.cancelled = true;
        let result = Bank::default()
            .execute(&programs, vec![request], &mut host)
            .pop()
            .unwrap();
        assert_eq!(result.failure, Some(Failure::Cancelled));
    }
    #[test]
    fn control_chains_fuse_without_losing_primitive_width() {
        let doc = Root::str("a; b;", GrammarDialect::JavaScript);
        let rule = AstMatcher::Not(Box::new(AstMatcher::Not(Box::new(AstMatcher::All(vec![
            AstMatcher::Regex(regex::Regex::new("^[ab]").unwrap()),
        ])))));
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let requests = doc
            .root()
            .children()
            .map(|node| Request {
                program,
                context: 0,
                node,
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            })
            .collect();
        let mut bank = Bank::default();
        let results = bank.execute(&programs, requests, &mut SyntaxHost);
        assert!(results.iter().all(|r| r.truth == 2 && r.failure.is_none()));
        assert!(bank.metrics.fused_control_steps >= 3);
        assert_eq!(bank.metrics.atomic_family_dispatches[2], 1);
        assert_eq!(bank.metrics.atomic_family_lanes[2], 2);
    }
    #[test]
    fn simple_mixed_roots_keep_full_width_within_one_context() {
        let doc = Root::str(&"a;\n".repeat(128), GrammarDialect::JavaScript);
        let mut programs = Programs::default();
        let first_rule = kind("expression_statement");
        let first = programs.lower(&first_rule);
        let second_rule = AstMatcher::Regex(regex::Regex::new("^a").unwrap());
        let second = programs.lower(&second_rule);
        assert!(!programs.requires_order(first));
        assert!(!programs.requires_order(second));
        let requests = doc
            .root()
            .children()
            .enumerate()
            .map(|(i, node)| Request {
                program: if i % 2 == 0 { first } else { second },
                context: 0,
                node,
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            })
            .collect();
        let mut bank = Bank::default();
        let results = bank.execute(&programs, requests, &mut SyntaxHost);
        assert!(results.iter().all(|r| r.truth == 2 && r.failure.is_none()));
        assert_eq!(bank.metrics.max_width, 128);
    }
    #[test]
    fn ordered_roots_share_suffixes_and_batch_independent_contexts() {
        let doc = Root::str(&"a;\n".repeat(128), GrammarDialect::JavaScript);
        let rule = AstMatcher::All(vec![
            AstMatcher::Semantic(Semantic::Capture("ROOT".into())),
            AstMatcher::Precedes(
                super::super::Related::new(kind("class_declaration")),
                StopBy::End,
            ),
        ]);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let mut requests = Vec::new();
        for node in doc.root().children() {
            for context in [0, 1] {
                requests.push(Request {
                    program,
                    context,
                    node: node.clone(),
                    env: Arc::new(MetaVarEnv::new()),
                    three: true,
                    each: None,
                    following: Arc::new(Vec::new()),
                });
            }
        }
        let mut bank = Bank::default();
        let results = bank.execute(&programs, requests, &mut SyntaxHost);
        assert_eq!(results.len(), 256);
        assert!(results.iter().all(|r| r.truth == 0 && r.failure.is_none()));
        assert!(
            bank.metrics.closure_candidates <= 256,
            "overlapping roots must test only linear candidates"
        );
        assert!(bank.metrics.closure_hits >= 250);
        assert!(
            bank.metrics.max_width >= 2,
            "independent contexts must retain cohort width"
        );
    }
    #[test]
    fn clean_closures_reuse_descendants_and_axis_suffixes() {
        let doc = Root::str("f(g(h())); a; b; c;", GrammarDialect::JavaScript);
        let rules = [
            has(kind("class_declaration"), StopBy::End),
            AstMatcher::Precedes(
                super::super::Related::new(kind("class_declaration")),
                StopBy::End,
            ),
            AstMatcher::Follows(
                super::super::Related::new(kind("class_declaration")),
                StopBy::End,
            ),
            AstMatcher::Inside(
                super::super::Related::new(kind("class_declaration")),
                StopBy::End,
            ),
            has(kind("identifier"), StopBy::End),
        ];
        for rule in rules {
            let mut programs = Programs::default();
            let program = programs.lower(&rule);
            let mut bank = Bank::default();
            let requests: Vec<_> = doc
                .root()
                .dfs()
                .map(|node| Request {
                    program,
                    context: 0,
                    node,
                    env: Arc::new(MetaVarEnv::new()),
                    three: true,
                    each: None,
                    following: Arc::new(Vec::new()),
                })
                .collect();
            let first = bank.execute(&programs, requests.clone(), &mut SyntaxHost);
            let before = bank.metrics.closure_candidates;
            let second = bank.execute(&programs, requests, &mut SyntaxHost);
            assert_eq!(
                first.iter().map(|r| r.truth).collect::<Vec<_>>(),
                second.iter().map(|r| r.truth).collect::<Vec<_>>()
            );
            assert!(bank.metrics.closure_hits > 0);
            assert_eq!(
                bank.metrics.closure_candidates, before,
                "completed clean closures require no further predicates"
            );
            assert!(second.iter().all(|r| r.work == 0 && r.failure.is_none()));
        }
    }
    #[test]
    fn structural_cohorts_preserve_order_captures_and_unknowns() {
        let docs = [
            Root::str(
                "function f(x) { a(a); b(c); return x; }",
                GrammarDialect::JavaScript,
            ),
            Root::str(
                "g(g); reset(); g(h); // comment\n",
                GrammarDialect::JavaScript,
            ),
        ];
        let unknown = || AstMatcher::TestUnknown(regex::Regex::new("a|g").unwrap());
        let comparison = AstMatcher::Compare(Box::new(super::super::Comparison {
            left: "F".into(),
            right: "A".into(),
            op: super::super::CompareOp::Eq,
            collation: super::super::Collation::Ordinal,
            text: super::super::CompareText::Source,
            reading: None,
            transform: None,
            commute: Vec::new(),
        }));
        let field = |inside, stop| {
            AstMatcher::Field(Box::new(super::super::FieldRelation {
                inside,
                field: GrammarDialect::JavaScript.field_to_id("body").unwrap(),
                rule: pattern("$FIELD"),
                stop,
                filter: OnceLock::new(),
            }))
        };
        let rules = vec![
            AstMatcher::All(vec![pattern("$F($A)"), comparison]),
            AstMatcher::Any(vec![
                AstMatcher::All(vec![unknown(), pattern("$A")]),
                kind("identifier"),
            ]),
            AstMatcher::All(vec![
                AstMatcher::All(vec![unknown(), pattern("$A")]),
                pattern("$A"),
                kind("number"),
            ]),
            AstMatcher::Not(Box::new(unknown())),
            has(pattern("$F($A)"), StopBy::End),
            has(
                pattern("$F($A)"),
                StopBy::Rule(Box::new(kind("function_declaration"))),
            ),
            AstMatcher::Inside(
                super::super::Related::new(pattern("function $F($$$) { $$$ }")),
                StopBy::End,
            ),
            AstMatcher::Precedes(super::super::Related::new(pattern("reset();")), StopBy::End),
            AstMatcher::Follows(
                super::super::Related::new(pattern("$F($A);")),
                StopBy::Rule(Box::new(pattern("reset();"))),
            ),
            has(pattern("$F($A)"), StopBy::Rule(Box::new(unknown()))),
            field(false, StopBy::Neighbor),
            field(false, StopBy::End),
            field(false, StopBy::Rule(Box::new(kind("identifier")))),
            field(true, StopBy::End),
            field(true, StopBy::Rule(Box::new(unknown()))),
            AstMatcher::NthChild(Box::new(super::super::NthChild {
                position: super::super::NthPosition { step: 2, offset: 1 },
                of_rule: Some(pattern("$N")),
                reverse: true,
                capture_free: OnceLock::new(),
            })),
            AstMatcher::Count(Box::new(super::super::Count {
                relation: has(kind("identifier"), StopBy::End),
                min: 1,
                max: None,
                binding: Some(super::super::CountBinding {
                    name: "TOTAL".into(),
                    offset: 2,
                }),
            })),
            AstMatcher::Constrained(Box::new(super::super::Constrained {
                rule: pattern("$F($A)"),
                constraints: vec![("A".into(), kind("identifier"))],
            })),
        ];
        let mut programs = Programs::default();
        let ids: Vec<_> = rules.iter().map(|r| programs.lower(r)).collect();
        let mut requests = Vec::new();
        let mut expected = Vec::new();
        for (rule, &program) in rules.iter().zip(&ids) {
            for (context, doc) in docs.iter().enumerate() {
                for node in doc.root().dfs() {
                    expected.push(native(rule, node.clone()));
                    requests.push(Request {
                        program,
                        context: context as u32,
                        node,
                        env: Arc::new(MetaVarEnv::new()),
                        three: true,
                        each: None,
                        following: Arc::new(Vec::new()),
                    });
                }
            }
        }
        let mut bank = Bank::default();
        let results = bank.execute(&programs, requests, &mut SyntaxHost);
        for (i, (actual, expected)) in results.into_iter().zip(expected).enumerate() {
            let captures =
                (actual.truth == 2).then(|| HashMap::<String, String>::from((*actual.env).clone()));
            assert_eq!((actual.truth, captures), expected, "lane {i}");
            assert_eq!(actual.failure, None, "lane {i}");
        }
        assert!(bank.metrics.max_width > 1);
    }
    #[test]
    fn utility_kind_rejection_preserves_entry_cost_depth_and_positive_captures() {
        struct Limited {
            left: usize,
            failed: bool,
        }
        impl<'t, D: Doc<Source: Content<Underlying = u8>>> Host<'t, D> for Limited {
            fn evaluate(
                &mut self,
                _: &Programs,
                _: Vec<SemanticRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("semantic")
            }
            fn compare(
                &mut self,
                _: &Programs,
                _: Vec<CompareRequest<'t, D>>,
            ) -> Vec<Result<'t, D>> {
                panic!("compare")
            }
            fn can_remember(&self, _: u32, _: &Node<'t, D>) -> bool {
                true
            }
            fn spend(&mut self, _: u32, amount: usize) -> bool {
                if amount > self.left {
                    self.failed = true;
                    false
                } else {
                    self.left -= amount;
                    true
                }
            }
            fn failure(&mut self, _: u32) -> Option<Failure> {
                self.failed.then_some(Failure::Utility(UtilLimit::Work))
            }
        }
        let doc = Root::str("f(x);", GrammarDialect::JavaScript);
        let table = Arc::new(super::super::UtilTable::new(
            vec!["guarded".into()],
            vec![false],
            vec![None],
        ));
        assert!(
            table
                .rules
                .set(vec![AstMatcher::Any(vec![
                    AstMatcher::All(vec![kind("identifier"), pattern("$X")]),
                    AstMatcher::All(vec![kind("call_expression"), pattern("$Y")]),
                ])])
                .is_ok()
        );
        let rule = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        for node in doc.root().dfs() {
            let expected = native(&rule, node.clone());
            let expected_work = usize::MAX - super::super::util::UTIL_STEPS_LEFT.get();
            let mut bank = Bank::default();
            let request = Request {
                program,
                context: 0,
                node,
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            for _ in 0..2 {
                let answer = bank
                    .execute(
                        &programs,
                        vec![request.clone()],
                        &mut Limited {
                            left: 100,
                            failed: false,
                        },
                    )
                    .pop()
                    .unwrap();
                assert_eq!(answer.truth, expected.0);
                assert_eq!(
                    (answer.work, answer.replay_work, answer.levels),
                    (expected_work, expected_work, 1)
                );
                assert_eq!(
                    (answer.truth == 2)
                        .then(|| HashMap::<String, String>::from((*answer.env).clone())),
                    expected.1
                );
                if answer.truth == 0 {
                    assert!(bank.memo.is_empty());
                    assert!(bank.metrics.utility_kind_rejections > 0);
                }
            }
        }
        for (left, depth, failure, work, levels) in [
            (0, 0, Some(Failure::Utility(UtilLimit::Work)), 0, 0),
            (1, 0, None, 1, 1),
            (
                1,
                super::super::util::MAX_UTIL_DEPTH,
                Some(Failure::Utility(UtilLimit::Depth)),
                0,
                0,
            ),
        ] {
            let request = Request {
                program,
                context: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            };
            let mut host = Limited {
                left,
                failed: false,
            };
            let answer = Bank::default()
                .execute_at(&programs, vec![request], &mut host, depth)
                .pop()
                .unwrap();
            assert_eq!(
                (answer.truth, answer.failure, answer.work, answer.levels),
                (0, failure, work, levels)
            );
            assert_eq!(host.left, left - work);
        }
        // A charged alternative blocks the outer zero-work membership proof.
        let outer = AstMatcher::Any(vec![
            AstMatcher::All(vec![kind("identifier"), pattern("$A")]),
            AstMatcher::All(vec![rule.clone(), kind("call_expression")]),
        ]);
        let program = programs.lower(&outer);
        assert!(programs.entries[program as usize].guard.is_none());
        let request = Request {
            program,
            context: 0,
            node: doc.root(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let answer = Bank::default()
            .execute(
                &programs,
                vec![request],
                &mut Limited {
                    left: 1,
                    failed: false,
                },
            )
            .pop()
            .unwrap();
        assert_eq!(
            (answer.truth, answer.work, answer.replay_work, answer.levels),
            (0, 1, 1, 1)
        );
    }
    #[test]
    fn utilities_lower_recursive_graphs_and_replay_input_frames() {
        let table = Arc::new(super::super::UtilTable::new(
            vec!["nested".into()],
            vec![false],
            vec![None],
        ));
        let reference = super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        };
        let body = AstMatcher::Any(vec![
            pattern("$NAME"),
            has(AstMatcher::Util(reference.clone()), StopBy::Neighbor),
        ]);
        assert!(table.rules.set(vec![body]).is_ok());
        let rule = AstMatcher::Util(reference);
        let doc = Root::str("foo(bar);", GrammarDialect::JavaScript);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let equivalent_reference = rule.clone();
        let equivalent_program = programs.lower(&equivalent_reference);
        let request = Request {
            program,
            context: 7,
            node: doc.root(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        let first = bank
            .execute(&programs, vec![request.clone()], &mut SyntaxHost)
            .remove(0);
        let second = bank
            .execute(&programs, vec![request.clone()], &mut SyntaxHost)
            .remove(0);
        assert_eq!(first.truth, native(&rule, doc.root()).0);
        assert_eq!(second.truth, first.truth);
        assert_eq!(second.work, first.replay_work);
        assert!(bank.metrics.utility_hits > 0);
        bank.memo.clear();
        let recomputed = bank
            .execute(&programs, vec![request.clone()], &mut SyntaxHost)
            .remove(0);
        assert_eq!(recomputed.work, first.work);
        assert_eq!(recomputed.replay_work, first.replay_work);
        assert_eq!(recomputed.truth, first.truth);
        let mut changed = request;
        changed.context = 8;
        changed.program = equivalent_program;
        let hits = bank.metrics.utility_hits;
        bank.execute(&programs, vec![changed], &mut SyntaxHost);
        assert_eq!(bank.metrics.utility_hits, hits + 1);
    }
    #[test]
    fn calls_unify_outputs_and_exclusions_use_the_first_available_branch() {
        let table = Arc::new(super::super::UtilTable::new(
            vec!["pair".into()],
            vec![true],
            vec![None],
        ));
        assert!(table.rules.set(vec![pattern("$P($Q)")]).is_ok());
        let rule = AstMatcher::Call(Box::new(super::super::UtilCall {
            util: super::super::UtilRef {
                table: Arc::downgrade(&table),
                index: 0,
            },
            args: vec![("P".into(), "X".into()), ("Q".into(), "X".into())],
        }));
        let doc = Root::str("f(f); f(g);", GrammarDialect::JavaScript);
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let mut requests = Vec::new();
        let mut expected = Vec::new();
        for node in doc.root().dfs() {
            expected.push(native(&rule, node.clone()));
            requests.push(Request {
                program,
                context: 0,
                node,
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            });
        }
        let answers = Bank::default().execute(&programs, requests, &mut SyntaxHost);
        for (actual, expected) in answers.into_iter().zip(expected) {
            assert_eq!(
                (
                    actual.truth,
                    (actual.truth == 2)
                        .then(|| HashMap::<String, String>::from((*actual.env).clone()))
                ),
                expected
            );
        }
        let search = has(pattern("$F($A)"), StopBy::End);
        let program = programs.lower(&search);

        // Find the first capture through the same executor rather than guessing
        // the parser's call-expression span.
        let base = Request {
            program,
            context: 0,
            node: doc.root(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        let matched = bank
            .execute(&programs, vec![base.clone()], &mut SyntaxHost)
            .remove(0);
        let range = matched.env.get_match("F").unwrap().range();
        let mut excluded = base;
        excluded.each = Some(Arc::new(Exclusion {
            name: "F".into(),
            ranges: BTreeSet::from([(range.start, range.end)]),
        }));
        let next = bank
            .execute(&programs, vec![excluded], &mut SyntaxHost)
            .remove(0);
        assert_eq!(next.truth, 2);
        assert_ne!(next.env.get_match("F").unwrap().range(), range);
    }
    #[test]
    fn false_nth_frames_reach_later_relation_candidates() {
        let rule = has(
            AstMatcher::NthChild(Box::new(super::super::NthChild {
                position: super::super::NthPosition { step: 0, offset: 2 },
                of_rule: Some(AstMatcher::Any(vec![
                    AstMatcher::Regex(regex::Regex::new("^b").unwrap()),
                    pattern("$X"),
                ])),
                reverse: false,
                capture_free: OnceLock::new(),
            })),
            StopBy::Neighbor,
        );
        let doc = Root::str("a; b;", GrammarDialect::JavaScript);
        let expected = native(&rule, doc.root());
        assert_eq!(expected.0, 2);
        assert_eq!(
            expected.1.as_ref().unwrap().get("X").map(String::as_str),
            Some("a;")
        );
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let request = Request {
            program,
            context: 0,
            node: doc.root(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        for _ in 0..2 {
            let result = bank
                .execute(&programs, vec![request.clone()], &mut SyntaxHost)
                .remove(0);
            assert_eq!(
                (
                    result.truth,
                    Some(HashMap::<String, String>::from((*result.env).clone()))
                ),
                expected
            );
        }
    }
    #[test]
    fn memoized_false_utilities_do_not_replay_incidental_capture_writes() {
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use ast_grep_core::tree_sitter::StrDoc;
        let source = "a; b;";
        let grammar = Grammar::new(GrammarDialect::JavaScript).unwrap();
        let native_doc = StrDoc::new(source, GrammarDialect::JavaScript);
        let image =
            TapeImage::encode(native_doc.tree.root_node(), &grammar, source, &|| false).unwrap();
        let doc = Root::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        );
        let node = doc.root().children().next().unwrap();
        let table = Arc::new(super::super::UtilTable::new(
            vec!["reject".into()],
            vec![false],
            vec![None],
        ));
        assert!(
            table
                .rules
                .set(vec![AstMatcher::NthChild(Box::new(
                    super::super::NthChild {
                        position: super::super::NthPosition { step: 0, offset: 0 },
                        of_rule: Some(pattern("$X")),
                        reverse: false,
                        capture_free: OnceLock::new(),
                    }
                ))])
                .is_ok()
        );
        let rule = AstMatcher::Util(super::super::UtilRef {
            table: Arc::downgrade(&table),
            index: 0,
        });
        let expected = super::super::memo::with_file(&doc, || {
            (0..2)
                .map(|_| {
                    super::super::util::THREE_VALUED.set(true);
                    super::super::util::UNKNOWN.set(false);
                    super::super::util::UTIL_DEPTH.set(0);
                    super::super::util::UTIL_STACK_BASE.set(0);
                    super::super::util::UTIL_STEPS_LEFT.set(usize::MAX);
                    super::super::util::UTIL_EXCEEDED.set(None);
                    let mut env = Cow::Owned(MetaVarEnv::new());
                    let matched = rule.match_node_with_env(node.clone(), &mut env).is_some();
                    (matched, HashMap::<String, String>::from(env.into_owned()))
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(expected[0].1.get("X").map(String::as_str), Some("a;"));
        assert!(expected[1].1.is_empty());
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let request = Request {
            program,
            context: 0,
            node,
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        for expected in expected {
            let result = bank
                .execute(&programs, vec![request.clone()], &mut SyntaxHost)
                .remove(0);
            assert_eq!(
                (
                    result.truth == 2,
                    HashMap::<String, String>::from((*result.env).clone())
                ),
                expected
            );
        }
    }
    #[test]
    fn atomic_any_retries_an_excluded_branch_in_a_fresh_capture_frame() {
        let rule = AstMatcher::Any(vec![pattern("$F($A)"), pattern("$A($F)")]);
        let doc = Root::str("f(g)", GrammarDialect::JavaScript);
        let node = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "call_expression")
            .unwrap();
        let mut programs = Programs::default();
        let program = programs.lower(&rule);
        let request = Request {
            program,
            context: 0,
            node: node.clone(),
            env: Arc::new(MetaVarEnv::new()),
            three: true,
            each: None,
            following: Arc::new(Vec::new()),
        };
        let mut bank = Bank::default();
        let first = bank
            .execute(&programs, vec![request.clone()], &mut SyntaxHost)
            .remove(0);
        let range = first.env.get_match("F").unwrap().range();
        let expected = {
            let enumeration = super::super::each::Enumeration::start("F");
            assert!(enumeration.record(Some(range.clone())));
            native(&rule, node)
        };
        let mut excluded = request;
        excluded.each = Some(Arc::new(Exclusion {
            name: "F".into(),
            ranges: BTreeSet::from([(range.start, range.end)]),
        }));
        let result = bank
            .execute(&programs, vec![excluded], &mut SyntaxHost)
            .remove(0);
        assert_eq!(
            (
                result.truth,
                Some(HashMap::<String, String>::from((*result.env).clone()))
            ),
            expected
        );
        assert_eq!(result.env.get_match("F").unwrap().text(), "g");
    }
}
