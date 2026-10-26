//! Typed semantic projections for the capture-bearing operation scheduler.
use super::cohort::{
    self, CompareRequest, Failure, Programs, Request, Result as Answer, SemanticRequest,
};
use super::{Semantic, each, util};
use ast_grep_core::{Doc, meta_var::MetaVarEnv};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
    ops::Range,
    rc::Rc,
    sync::Arc,
};
mod bindings;
mod navigation;
mod parsed;

pub(in crate::code_rule_dsl) trait Context {
    /// Install a file's parked facts and clause identity once for this operation cohort.
    fn with<T>(&mut self, owner: u32, run: impl FnOnce() -> T) -> T;
    fn group(&self, owner: u32) -> u32 {
        owner
    }
    fn with_group<T>(&mut self, _owner: u32, run: impl FnOnce(&mut Self) -> T) -> T
    where
        Self: Sized,
    {
        run(self)
    }
    fn cancelled(&self, owner: u32) -> bool;
    /// Root identity of the evaluated file; parsed and foreign trees are excluded.
    fn file_root(&self, _owner: u32) -> Option<usize> {
        None
    }
}
#[derive(Default, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Metrics {
    pub projection_batches: u64,
    pub context_installs: u64,
    pub exclusion_installs: u64,
    pub projection_lanes: u64,
    pub scalar_matcher_calls: u64,
    pub navigation_queries: u64,
    pub navigation_index_hits: u64,
    pub navigation_candidates: u64,
    pub nested: cohort::Metrics,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Cost {
    pub work: usize,
    pub replay_work: usize,
    pub levels: u32,
    pub failure: Option<Failure>,
    pub unknown: bool,
    pub clause_state_read: bool,
}
impl Cost {
    pub fn add(&mut self, other: Self) {
        self.work = self.work.saturating_add(other.work);
        self.replay_work = self.replay_work.saturating_add(other.replay_work);
        self.levels = self.levels.max(other.levels);
        self.failure = self.failure.or(other.failure);
        self.unknown |= other.unknown;
        self.clause_state_read |= other.clause_state_read;
    }
    pub fn of<D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
        r: &Answer<'_, D>,
    ) -> Self {
        Self {
            work: r.work,
            replay_work: r.replay_work,
            levels: r.levels,
            failure: r.failure,
            unknown: r.truth == 1,
            clause_state_read: r.clause_state_read,
        }
    }
    pub fn answer<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
        self,
        truth: u8,
        env: Arc<MetaVarEnv<'t, D>>,
    ) -> Answer<'t, D> {
        Answer {
            truth,
            env,
            work: self.work,
            replay_work: self.replay_work,
            levels: self.levels,
            failure: self.failure,
            clause_state_read: self.clause_state_read,
        }
    }
}
pub(super) struct Projection<T> {
    pub value: Option<T>,
    pub cost: Cost,
}
struct Limit {
    left: usize,
    failure: Option<Failure>,
}
pub(super) struct ParsedContext {
    pub owner: u32,
    pub slot: usize,
    pub tree: Arc<crate::code_rule_dsl::literal::SubTree>,
    pub tables: Vec<Arc<crate::code_rule_dsl::scope::ScopeTable>>,
    pub maps: Vec<each::HostMap>,
}
pub(super) struct ParentRequest {
    pub slot: usize,
    pub program: u32,
    pub exports: Vec<(String, String, Range<usize>)>,
    pub depth: u32,
}
pub(super) struct ParentAnswer {
    pub truth: u8,
    pub cost: Cost,
}
pub(super) trait Parent {
    fn evaluate(&mut self, programs: &Programs, requests: Vec<ParentRequest>) -> Vec<ParentAnswer>;
}
impl<F: FnMut(&Programs, Vec<ParentRequest>) -> Vec<ParentAnswer>> Parent for F {
    fn evaluate(&mut self, p: &Programs, r: Vec<ParentRequest>) -> Vec<ParentAnswer> {
        self(p, r)
    }
}
pub(in crate::code_rule_dsl) struct NativeHost<'p, C: Context> {
    pub(super) contexts: Rc<RefCell<C>>,
    limits: Rc<RefCell<Vec<Limit>>>,
    pub(super) next_context: Rc<Cell<u32>>,
    pub(super) hosted: Rc<Cell<u32>>,
    pub(super) parsed: BTreeMap<u32, ParsedContext>,
    pub(super) parent: Option<&'p mut dyn Parent>,
    pub(super) metrics: Rc<RefCell<Metrics>>,
}
impl<C: Context> NativeHost<'_, C> {
    pub fn new(contexts: C, allowances: BTreeMap<u32, usize>) -> Self {
        let next_context = allowances
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .unwrap();
        Self {
            contexts: Rc::new(RefCell::new(contexts)),
            limits: Rc::new(RefCell::new(
                (0..next_context)
                    .map(|id| Limit {
                        left: allowances.get(&id).copied().unwrap_or(usize::MAX),
                        failure: None,
                    })
                    .collect(),
            )),
            next_context: Rc::new(Cell::new(next_context)),
            hosted: Rc::new(Cell::new(0)),
            parsed: BTreeMap::new(),
            parent: None,
            metrics: Rc::default(),
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::to_value(&*self.metrics.borrow()).unwrap()
    }
    pub fn remaining(&self, context: u32) -> usize {
        self.limits
            .borrow()
            .get(self.owner(context) as usize)
            .map_or(usize::MAX, |l| l.left)
    }
    pub fn owner(&self, context: u32) -> u32 {
        if self.parsed.is_empty() {
            context
        } else {
            self.parsed.get(&context).map_or(context, |p| p.owner)
        }
    }
    pub fn failed(&self, context: u32) -> Option<Failure> {
        self.limits
            .borrow()
            .get(self.owner(context) as usize)
            .and_then(|l| l.failure)
    }
    pub fn debit(&mut self, context: u32, amount: usize) -> bool {
        let owner = self.owner(context);
        let mut limits = self.limits.borrow_mut();
        let Some(limit) = limits.get_mut(owner as usize) else {
            return true;
        };
        if limit.failure.is_some() {
            return false;
        }
        if amount > limit.left {
            limit.left = 0;
            limit.failure = Some(Failure::Utility(super::UtilLimit::Work));
            false
        } else {
            limit.left -= amount;
            true
        }
    }
    pub fn record_failure(&mut self, context: u32, failure: Option<Failure>) {
        let owner = self.owner(context);
        if let Some(limit) = self.limits.borrow_mut().get_mut(owner as usize) {
            limit.failure = limit.failure.or(failure);
        }
    }
    pub fn fork<'p>(&self, parent: Option<&'p mut dyn Parent>) -> NativeHost<'p, C> {
        NativeHost {
            contexts: self.contexts.clone(),
            limits: self.limits.clone(),
            next_context: self.next_context.clone(),
            hosted: self.hosted.clone(),
            parsed: BTreeMap::new(),
            parent,
            metrics: self.metrics.clone(),
        }
    }
    pub fn run<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
        &mut self,
        p: &Programs,
        requests: Vec<Request<'t, D>>,
        depth: u32,
    ) -> Vec<Answer<'t, D>> {
        let mut bank = cohort::Bank::default();
        let result = bank.execute_at(p, requests, self, depth);
        self.metrics.borrow_mut().nested.merge(&bank.metrics);
        result
    }
    pub fn run_host<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
        &mut self,
        p: &Programs,
        requests: Vec<Request<'t, D>>,
        depth: u32,
    ) -> Vec<Answer<'t, D>> {
        struct Hosted(Rc<Cell<u32>>);
        impl Drop for Hosted {
            fn drop(&mut self) {
                self.0.set(self.0.get() - 1);
            }
        }
        self.hosted.set(self.hosted.get() + 1);
        let _hosted = Hosted(self.hosted.clone());
        self.run(p, requests, depth)
    }
    /// All callbacks are atomic projections. No callback may evaluate a DSL subrule.
    pub fn project<'r, 't, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>, T>(
        &mut self,
        leaves: &'r [SemanticRequest<'t, D>],
        mut f: impl FnMut(&'r SemanticRequest<'t, D>) -> T,
    ) -> Vec<Projection<T>> {
        let mut groups = BTreeMap::<u32, Vec<usize>>::new();
        for (i, leaf) in leaves.iter().enumerate() {
            groups.entry(leaf.request.context).or_default().push(i);
        }
        let mut output: Vec<Option<Projection<T>>> = (0..leaves.len()).map(|_| None).collect();
        let mut file_groups = BTreeMap::<u32, Vec<_>>::new();
        for (context, indices) in groups {
            let owner = self.owner(context);
            let group = self.contexts.borrow().group(owner);
            let maps = self
                .parsed
                .get(&context)
                .map_or_else(Vec::new, |p| p.maps.clone());
            let tables = self
                .parsed
                .get(&context)
                .map_or_else(Vec::new, |p| p.tables.clone());
            file_groups
                .entry(group)
                .or_default()
                .push((owner, indices, maps, tables));
        }
        for group in file_groups.into_values() {
            let contexts = self.contexts.clone();
            let limits = self.limits.clone();
            let metrics = self.metrics.clone();
            metrics.borrow_mut().context_installs += 1;
            let owner = group[0].0;
            contexts.borrow_mut().with_group(owner, |contexts| {
                for (owner, indices, maps, tables) in group {
                    metrics.borrow_mut().projection_batches += 1;
                    contexts.with(owner, || {
                        let _tables = tables
                            .into_iter()
                            .map(super::resolve::SubTables::enter)
                            .collect::<Vec<_>>();
                        let mut start = 0;
                        while start < indices.len() {
                            let snapshot = &leaves[indices[start]].request.each;
                            let mut end = start + 1;
                            while end < indices.len()
                                && match (snapshot, &leaves[indices[end]].request.each) {
                                    (None, None) => true,
                                    (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                                    _ => false,
                                }
                            {
                                end += 1;
                            }
                            let ranges = snapshot.as_ref().map_or_else(HashSet::new, |excluded| {
                                excluded.ranges.iter().copied().collect()
                            });
                            metrics.borrow_mut().exclusion_installs += 1;
                            each::with_exclusion(
                                snapshot.as_ref().map(|excluded| excluded.name.as_str()),
                                &ranges,
                                &maps,
                                || {
                                    for &i in &indices[start..end] {
                                        let leaf = &leaves[i];
                                        let req = &leaf.request;
                                        let prior = limits
                                            .borrow()
                                            .get(owner as usize)
                                            .and_then(|l| l.failure);
                                        if let Some(failure) = prior {
                                            output[i] = Some(Projection {
                                                value: None,
                                                cost: Cost {
                                                    failure: Some(failure),
                                                    ..Default::default()
                                                },
                                            });
                                            continue;
                                        }
                                        let left = limits
                                            .borrow()
                                            .get(owner as usize)
                                            .map_or(usize::MAX, |l| l.left);
                                        util::UTIL_STEPS_LEFT.set(left);
                                        util::UTIL_DEPTH.set(leaf.utility_depth);
                                        util::UTIL_EXCEEDED.set(None);
                                        util::UNKNOWN.set(false);
                                        util::THREE_VALUED.set(req.three);
                                        let peak = super::swap_util_peak(leaf.utility_depth);
                                        let filled = super::fill_steps();
                                        let reads = super::clause_state_reads();
                                        let before = SCALAR_CALLS.get();
                                        struct Audit(bool);
                                        impl Drop for Audit {
                                            fn drop(&mut self) {
                                                AUDITING.set(self.0);
                                            }
                                        }
                                        let audit = Audit(AUDITING.replace(true));
                                        let value = f(leaf);
                                        drop(audit);
                                        let work = left.saturating_sub(util::UTIL_STEPS_LEFT.get());
                                        let failure =
                                util::UTIL_EXCEEDED.get().map(Failure::Utility).or_else(|| {
                                    crate::code_rule_dsl::budget::matcher_budget_observed()
                                        .then_some(Failure::Cancelled)
                                });
                                        let cost = Cost {
                                            work,
                                            replay_work: work.saturating_sub(
                                                super::fill_steps().saturating_sub(filled),
                                            ),
                                            levels: super::swap_util_peak(peak)
                                                .saturating_sub(leaf.utility_depth),
                                            failure,
                                            unknown: util::UNKNOWN.get(),
                                            clause_state_read: reads != super::clause_state_reads(),
                                        };
                                        if let Some(limit) =
                                            limits.borrow_mut().get_mut(owner as usize)
                                        {
                                            limit.left = limit.left.saturating_sub(work);
                                            limit.failure = limit.failure.or(failure);
                                        }
                                        let mut m = metrics.borrow_mut();
                                        m.projection_lanes += 1;
                                        m.scalar_matcher_calls += SCALAR_CALLS.get() - before;
                                        output[i] = Some(Projection {
                                            value: Some(value),
                                            cost,
                                        });
                                    }
                                },
                            );
                            start = end;
                        }
                    });
                }
            });
        }
        output.into_iter().map(Option::unwrap).collect()
    }
}
thread_local! { static AUDITING:Cell<bool>=const{Cell::new(false)}; static SCALAR_CALLS:Cell<u64>=const{Cell::new(0)}; }
pub(super) fn scalar_call() {
    if AUDITING.get() {
        SCALAR_CALLS.set(SCALAR_CALLS.get() + 1);
    }
}
impl<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>, C: Context>
    cohort::Host<'t, D> for NativeHost<'_, C>
{
    fn evaluate(
        &mut self,
        p: &Programs,
        leaves: Vec<SemanticRequest<'t, D>>,
    ) -> Vec<Answer<'t, D>> {
        let mut groups: BTreeMap<u8, Vec<(usize, SemanticRequest<'t, D>)>> = BTreeMap::new();
        let len = leaves.len();
        for (i, l) in leaves.into_iter().enumerate() {
            let family = match l.semantic.as_ref() {
                Semantic::Binding(_) => 1,
                Semantic::Binds(_) => 2,
                Semantic::Parse(_) => 3,
                Semantic::Host(_) => 4,
                Semantic::FormatArgs(_) => 5,
                _ => 0,
            };
            groups.entry(family).or_default().push((i, l));
        }
        let mut out: Vec<Option<Answer<'t, D>>> = (0..len).map(|_| None).collect();
        for (family, group) in groups {
            let (ids, requests): (Vec<_>, Vec<_>) = group.into_iter().unzip();
            let answers = match family {
                1 | 2 => bindings::evaluate(self, p, requests),
                3..=5 => parsed::evaluate(self, p, requests),
                _ => self
                    .project(&requests, |leaf| {
                        let mut env = std::borrow::Cow::Borrowed(leaf.request.env.as_ref());
                        let matched = leaf
                            .semantic
                            .match_node(leaf.request.node.clone(), &mut env)
                            .is_some();
                        (
                            matched,
                            match env {
                                std::borrow::Cow::Borrowed(_) => leaf.request.env.clone(),
                                std::borrow::Cow::Owned(env) => Arc::new(env),
                            },
                        )
                    })
                    .into_iter()
                    .zip(&requests)
                    .map(|(r, q)| {
                        let (matched, env) = r.value.unwrap_or((false, q.request.env.clone()));
                        r.cost.answer(
                            if matched {
                                2
                            } else if r.cost.unknown {
                                1
                            } else {
                                0
                            },
                            env,
                        )
                    })
                    .collect(),
            };
            for (id, answer) in ids.into_iter().zip(answers) {
                out[id] = Some(answer);
            }
        }
        out.into_iter().map(Option::unwrap).collect()
    }
    fn navigate(
        &mut self,
        _: &Programs,
        requests: Vec<cohort::NavigationRequest<'t, D>>,
    ) -> Vec<cohort::NavigationResult<'t, D>> {
        navigation::evaluate(self, requests)
    }
    fn is_file(&self, request: &Request<'t, D>) -> bool {
        !self.parsed.contains_key(&request.context)
            && self.contexts.borrow().file_root(request.context)
                == Some(request.node.root() as *const _ as usize)
    }
    fn can_remember(&self, request: &Request<'t, D>) -> bool {
        self.hosted.get() == 0 && <Self as cohort::Host<'t, D>>::is_file(self, request)
    }
    fn compare(
        &mut self,
        _: &Programs,
        comparisons: Vec<CompareRequest<'t, D>>,
    ) -> Vec<Answer<'t, D>> {
        let requests = comparisons
            .iter()
            .map(|c| SemanticRequest {
                semantic: Arc::new(Semantic::Capture(String::new())),
                request: c.request.clone(),
                utility_depth: c.utility_depth,
            })
            .collect::<Vec<_>>();
        // Projection grouping preserves owner groups, not input order; identify the comparison by program.
        let by_program = comparisons
            .iter()
            .map(|c| (c.request.program, c.comparison.clone()))
            .collect::<BTreeMap<_, _>>();
        self.project(&requests, |r| {
            by_program[&r.request.program].holds(r.request.env.as_ref())
        })
        .into_iter()
        .zip(&requests)
        .map(|(r, q)| {
            let truth = match r.value.flatten() {
                Some(true) => 2,
                Some(false) => 0,
                None => 1,
            };
            r.cost.answer(truth, q.request.env.clone())
        })
        .collect()
    }
    fn failure(&mut self, context: u32) -> Option<Failure> {
        let cancelled = self.contexts.borrow().cancelled(self.owner(context));
        if cancelled {
            self.record_failure(context, Some(Failure::Cancelled));
        }
        self.failed(context)
    }
    fn spend(&mut self, context: u32, amount: usize) -> bool {
        self.debit(context, amount)
    }
    fn region_allowance(&self, context: u32) -> usize {
        self.remaining(context)
    }
    fn region_cancelled(&self, context: u32) -> bool {
        self.contexts.borrow().cancelled(self.owner(context))
    }
    fn binding_range(
        &mut self,
        context: u32,
        env: &MetaVarEnv<'t, D>,
        name: &str,
    ) -> Option<Range<usize>> {
        let Some(parsed) = self.parsed.get(&context) else {
            return env
                .get_match(name)
                .map(|n| n.range())
                .or_else(|| super::exported_span(env, name));
        };
        let (mut range, outer) = if let Some(node) = env.get_match(name) {
            let (last, outer) = parsed.maps.split_last()?;
            ((last.node)(node.node_id())?, outer)
        } else {
            (super::exported_span(env, name)?, parsed.maps.as_slice())
        };
        for map in outer.iter().rev() {
            range = (map.span)(range)?;
        }
        Some(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;

    #[test]
    fn exclusion_runs_preserve_per_lane_resources_and_restore_nested_state() {
        struct Plain;
        impl Context for Plain {
            fn with<T>(&mut self, _: u32, run: impl FnOnce() -> T) -> T {
                run()
            }
            fn cancelled(&self, _: u32) -> bool {
                false
            }
        }
        let doc = GrammarDialect::JavaScript.ast_grep("a; b;");
        let identifiers: Vec<_> = doc
            .root()
            .dfs()
            .filter(|node| node.kind() == "identifier")
            .collect();
        let mut env = MetaVarEnv::new();
        env.insert("X", identifiers[0].clone()).unwrap();
        env.insert("OUTER", doc.root()).unwrap();
        let env = Arc::new(env);
        let ranges = [(identifiers[0].range().start, identifiers[0].range().end)]
            .into_iter()
            .collect();
        let a = Arc::new(cohort::Exclusion {
            name: "X".into(),
            ranges,
        });
        let b = Arc::new((*a).clone()); // Equal values remain distinct installations.
        let c = Arc::new(cohort::Exclusion {
            name: "X".into(),
            ranges: [(identifiers[1].range().start, identifiers[1].range().end)]
                .into_iter()
                .collect(),
        });
        let snapshots = [
            None,
            None,
            Some(a.clone()),
            Some(a),
            Some(b),
            None,
            None,
            Some(c),
            None,
            None,
            None,
        ];
        let leaves: Vec<_> = snapshots
            .into_iter()
            .enumerate()
            .map(|(i, each)| SemanticRequest {
                semantic: Arc::new(Semantic::Capture(String::new())),
                utility_depth: (i % 3) as u32,
                request: Request {
                    program: i as u32,
                    context: u32::from(i >= 8),
                    node: doc.root(),
                    env: env.clone(),
                    three: true,
                    each,
                    following: Arc::new(Vec::new()),
                },
            })
            .collect();
        let mut host = NativeHost::new(Plain, [(0, 100), (1, 4)].into_iter().collect());
        let outer_ranges: HashSet<_> = [(doc.root().range().start, doc.root().range().end)]
            .into_iter()
            .collect();
        let answers = each::with_exclusion(Some("OUTER"), &outer_ranges, &[], || {
            let answers = host.project(&leaves, |leaf| {
                let i = leaf.request.program as usize;
                assert_eq!(each::active(), leaf.request.each.is_some());
                let key = each::exclusion_key();
                let excluded = each::active() && each::excluded(&leaf.request.env);
                each::suspended(|| assert!(!each::active()));
                assert_eq!(each::active(), leaf.request.each.is_some());
                if i == 7 {
                    each::with_exclusion(None, &HashSet::new(), &[], || assert!(!each::active()));
                    assert!(each::excludes("X"));
                    assert!(!each::excluded(&leaf.request.env));
                }
                let left = util::UTIL_STEPS_LEFT.get();
                util::UTIL_STEPS_LEFT.set(left.saturating_sub(3));
                if left < 3 {
                    util::UTIL_EXCEEDED.set(Some(util::UtilLimit::Work));
                }
                if i % 2 == 0 {
                    super::super::filled(1);
                } else {
                    super::super::read_clause_state();
                }
                super::super::swap_util_peak(leaf.utility_depth + 2);
                util::UNKNOWN.set(i % 2 == 1);
                (i, excluded, key)
            });
            assert!(each::excludes("OUTER"));
            assert!(each::excluded(&env));
            answers
        });
        for (i, answer) in answers.iter().take(8).enumerate() {
            assert_eq!(answer.value.as_ref().unwrap().0, i);
            assert_eq!(answer.value.as_ref().unwrap().1, (2..=4).contains(&i));
            assert_eq!(answer.cost.work, 3);
            assert_eq!(answer.cost.replay_work, if i % 2 == 0 { 2 } else { 3 });
            assert_eq!(answer.cost.levels, 2);
            assert_eq!(answer.cost.unknown, i % 2 == 1);
            assert_eq!(answer.cost.clause_state_read, i % 2 == 1);
            assert!(answer.cost.failure.is_none());
        }
        assert_eq!(answers[0].value.as_ref().unwrap().2, (0, 0));
        assert_eq!(
            answers[2].value.as_ref().unwrap().2,
            answers[3].value.as_ref().unwrap().2
        );
        assert_ne!(
            answers[3].value.as_ref().unwrap().2,
            answers[4].value.as_ref().unwrap().2
        );
        assert_eq!(answers[8].cost.work, 3);
        assert_eq!(answers[9].cost.work, 1);
        assert_eq!(
            answers[9].cost.failure,
            Some(Failure::Utility(util::UtilLimit::Work))
        );
        assert!(answers[10].value.is_none());
        assert_eq!(answers[10].cost.work, 0);
        assert_eq!(host.remaining(0), 76);
        assert_eq!(host.remaining(1), 0);
        let metrics = host.metrics.borrow();
        assert_eq!(metrics.exclusion_installs, 6);
        assert_eq!(metrics.projection_lanes, 10);
        assert_eq!(metrics.scalar_matcher_calls, 0);
    }

    #[test]
    fn projections_share_file_installation_and_keep_owner_costs_and_output_order() {
        struct Grouped {
            installs: Rc<RefCell<Vec<u32>>>,
            owner: Rc<Cell<u32>>,
        }
        impl Context for Grouped {
            fn group(&self, owner: u32) -> u32 {
                owner % 2
            }
            fn with_group<T>(&mut self, owner: u32, run: impl FnOnce(&mut Self) -> T) -> T {
                self.installs.borrow_mut().push(self.group(owner));
                run(self)
            }
            fn with<T>(&mut self, owner: u32, run: impl FnOnce() -> T) -> T {
                self.owner.set(owner);
                run()
            }
            fn cancelled(&self, _: u32) -> bool {
                false
            }
        }
        let installs = Rc::new(RefCell::new(Vec::new()));
        let owner = Rc::new(Cell::new(0));
        let mut host = NativeHost::new(
            Grouped {
                installs: installs.clone(),
                owner: owner.clone(),
            },
            [(0, 10), (1, 10), (2, 10)].into_iter().collect(),
        );
        let doc = GrammarDialect::JavaScript.ast_grep("a;");
        let requests = [2, 1, 0].map(|context| SemanticRequest {
            semantic: Arc::new(Semantic::Capture(String::new())),
            utility_depth: 0,
            request: Request {
                context,
                program: 0,
                node: doc.root(),
                env: Arc::new(MetaVarEnv::new()),
                three: true,
                each: None,
                following: Arc::new(Vec::new()),
            },
        });
        let result = host.project(&requests, |request| {
            let context = request.request.context;
            assert_eq!(owner.get(), context);
            util::UTIL_STEPS_LEFT.set(util::UTIL_STEPS_LEFT.get() - context as usize - 1);
            context
        });
        assert_eq!(*installs.borrow(), vec![0, 1]);
        assert_eq!(
            result.iter().map(|r| r.value.unwrap()).collect::<Vec<_>>(),
            vec![2, 1, 0]
        );
        assert_eq!(
            result.iter().map(|r| r.cost.work).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(
            (host.remaining(0), host.remaining(1), host.remaining(2)),
            (9, 8, 7)
        );
        let metrics = host.metrics.borrow();
        assert_eq!(metrics.context_installs, 2);
        assert_eq!(metrics.projection_batches, 3);
    }
}
