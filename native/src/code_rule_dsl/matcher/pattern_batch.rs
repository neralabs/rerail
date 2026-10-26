//! Cohort execution of Pattern tree operations. Ellipsis probes own a snapshot of
//! the capture environment: failed lookahead never writes into the real lane.
use ast_grep_core::matcher::{Pattern, PatternNode as P, kind_utils};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable as V};
use ast_grep_core::{Doc, MatchStrictness as S, Matcher, Node};
use rustc_hash::FxHashMap;
use std::sync::{Arc, Weak};

pub(super) struct Plan {
    pattern: Arc<Pattern>,
    contextual_root_kind: Option<u16>,
}

pub(super) fn compile(pattern: Arc<Pattern>) -> Arc<Plan> {
    // Query a metavariable root once to expose Pattern's private contextual
    // root constraint without performing candidate matching.
    let mut probe = (*pattern).clone();
    probe.node = P::MetaVar {
        meta_var: V::Dropped(false),
    };
    probe.strictness = S::Smart;
    let contextual_root_kind = probe
        .potential_kinds()
        .and_then(|kinds| kinds.iter().next().map(|kind| kind as u16));
    Arc::new(Plan {
        pattern,
        contextual_root_kind,
    })
}

/// Interning compares every public tree field and the recovered private root constraint.
#[derive(Default)]
pub(super) struct Plans {
    entries: FxHashMap<(Vec<u8>, u8, Option<u16>), Arc<Plan>>,
}
impl Plans {
    pub(super) fn compile(&mut self, pattern: Arc<Pattern>) -> Arc<Plan> {
        let candidate = compile(pattern);
        let mut tree = Vec::new();
        encode(&candidate.pattern.node, &mut tree);
        let strict = match candidate.pattern.strictness {
            S::Cst => 0,
            S::Smart => 1,
            S::Ast => 2,
            S::Relaxed => 3,
            S::Signature => 4,
            S::Template => 5,
        };
        self.entries
            .entry((tree, strict, candidate.contextual_root_kind))
            .or_insert(candidate)
            .clone()
    }
}
fn encode(node: &P, output: &mut Vec<u8>) {
    fn text(value: &str, output: &mut Vec<u8>) {
        output.extend_from_slice(&(value.len() as u64).to_le_bytes());
        output.extend_from_slice(value.as_bytes());
    }
    match node {
        P::MetaVar { meta_var } => match meta_var {
            V::Capture(name, named) => {
                output.extend_from_slice(&[0, u8::from(*named)]);
                text(name, output);
            }
            V::Dropped(named) => output.extend_from_slice(&[1, u8::from(*named)]),
            V::Multiple => output.push(2),
            V::MultiCapture(name) => {
                output.push(3);
                text(name, output);
            }
        },
        P::Terminal {
            text: value,
            is_named,
            kind_id,
        } => {
            output.extend_from_slice(&[4, u8::from(*is_named)]);
            output.extend_from_slice(&kind_id.to_le_bytes());
            text(value, output);
        }
        P::Internal { kind_id, children } => {
            output.push(5);
            output.extend_from_slice(&kind_id.to_le_bytes());
            output.extend_from_slice(&(children.len() as u64).to_le_bytes());
            for child in children {
                encode(child, output);
            }
        }
    }
}

type CacheKey = (usize, usize, usize, usize, usize);
struct Cached<'t, D: Doc> {
    _plan: Arc<Plan>,
    _node: Node<'t, D>,
    input: Weak<MetaVarEnv<'t, D>>,
    result: Option<Arc<MetaVarEnv<'t, D>>>,
}
pub(super) struct Cache<'t, D: Doc> {
    entries: FxHashMap<CacheKey, Cached<'t, D>>,
}
impl<D: Doc> Default for Cache<'_, D> {
    fn default() -> Self {
        Self {
            entries: FxHashMap::default(),
        }
    }
}
fn cache_key<D: Doc>(request: &Request<'_, D>) -> CacheKey {
    (
        Arc::as_ptr(&request.pattern) as usize,
        request.node.root() as *const _ as usize,
        request.node.get_doc() as *const D as usize,
        request.node.node_id(),
        Arc::as_ptr(&request.env) as usize,
    )
}

pub(super) struct Request<'t, D: Doc> {
    pub pattern: Arc<Plan>,
    pub node: Node<'t, D>,
    pub env: Arc<MetaVarEnv<'t, D>>,
}
#[derive(Default, serde::Serialize)]
pub(super) struct Metrics {
    pub operation_dispatches: usize,
    pub operation_lanes: usize,
    pub max_width: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub batch_deduplications: usize,
    pub unique_executions: usize,
}
impl Metrics {
    pub(super) fn merge(&mut self, other: &Self) {
        self.operation_dispatches += other.operation_dispatches;
        self.operation_lanes += other.operation_lanes;
        self.max_width = self.max_width.max(other.max_width);
        self.cache_hits += other.cache_hits;
        self.cache_misses += other.cache_misses;
        self.batch_deduplications += other.batch_deduplications;
        self.unique_executions += other.unique_executions;
    }
    fn dispatch(&mut self, width: usize) {
        if width != 0 {
            self.operation_dispatches += 1;
            self.operation_lanes += width;
            self.max_width = self.max_width.max(width);
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Outcome {
    Both,
    Goal,
    Candidate,
    SkipBoth,
    Fail,
}
struct Op<'p, 't, D: Doc> {
    goal: &'p P,
    node: Node<'t, D>,
    strict: &'p S,
    env: Arc<MetaVarEnv<'t, D>>,
    root: bool,
}
fn comments(s: &S) -> bool {
    !matches!(s, S::Cst | S::Ast)
}
fn trailing<D: Doc>(s: &S, n: &Node<D>) -> bool {
    match s {
        S::Cst => false,
        S::Smart => true,
        S::Ast => !n.is_named(),
        S::Relaxed | S::Signature => !n.is_named() || n.is_extra(),
        S::Template => n.is_extra(),
    }
}
fn skippable(s: &S, p: &P) -> bool {
    match s {
        S::Cst => false,
        _ => match p {
            P::MetaVar {
                meta_var: V::Multiple | V::MultiCapture(_),
            } => true,
            P::MetaVar {
                meta_var: V::Capture(_, named) | V::Dropped(named),
            } => !named && matches!(s, S::Ast | S::Relaxed | S::Signature),
            P::Terminal { is_named, .. } => {
                !is_named && matches!(s, S::Ast | S::Relaxed | S::Signature)
            }
            _ => false,
        },
    }
}
fn ellipsis(p: &P) -> Option<Option<&str>> {
    match p {
        P::MetaVar {
            meta_var: V::Multiple,
        } => Some(None),
        P::MetaVar {
            meta_var: V::MultiCapture(n),
        } => Some(Some(n)),
        _ => None,
    }
}
fn terminal<D: Doc>(s: &S, named: bool, text: &str, kind: u16, n: &Node<D>) -> Outcome {
    let kinds = kind_utils::are_kinds_matching(kind, n.kind_id());
    if kinds && (!named || text == n.text()) {
        return Outcome::Both;
    }
    if comments(s) && n.is_extra() {
        return Outcome::Candidate;
    }
    let (g, c) = match s {
        S::Cst => (false, false),
        S::Smart => (false, !n.is_named()),
        S::Ast | S::Relaxed => (!named, !n.is_named()),
        S::Signature => {
            if kinds {
                return Outcome::Both;
            }
            (!named, !n.is_named())
        }
        S::Template => {
            if text == n.text() {
                return Outcome::Both;
            }
            (false, !n.is_named())
        }
    };
    match (g, c) {
        (true, true) => Outcome::SkipBoth,
        (true, false) => Outcome::Goal,
        (false, true) => Outcome::Candidate,
        _ => Outcome::Fail,
    }
}
fn bind<'t, D: Doc>(v: &V, n: &Node<'t, D>, env: &mut Arc<MetaVarEnv<'t, D>>) -> bool {
    match v {
        V::Capture(name, named) => {
            (!named || n.is_named()) && Arc::make_mut(env).insert(name, n.clone()).is_some()
        }
        V::Dropped(named) => !named || n.is_named(),
        V::MultiCapture(name) => Arc::make_mut(env)
            .insert_multi(name, vec![n.clone()])
            .is_some(),
        V::Multiple => true,
    }
}
/// Each recursive call dispatches the next tree operation for *all* ready
/// cursors. There is one continuation per input lane, and one isolated probe
/// per ellipsis lane; no speculative branch tree is retained.
fn operations<'p, 't, D: Doc>(
    mut ops: Vec<Op<'p, 't, D>>,
    m: &mut Metrics,
) -> Vec<(Outcome, Arc<MetaVarEnv<'t, D>>)> {
    let mut outcomes = vec![Outcome::Fail; ops.len()];
    // Kind/text comparisons, binding, and child navigation are distinct cohorts.
    let terminals: Vec<_> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, o)| matches!(o.goal, P::Terminal { .. }).then_some(i))
        .collect();
    m.dispatch(terminals.len());
    for i in terminals {
        let o = &ops[i];
        if let P::Terminal {
            text,
            kind_id,
            is_named,
        } = o.goal
        {
            outcomes[i] = terminal(o.strict, *is_named, text, *kind_id, &o.node);
        }
    }
    let captures: Vec<_> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, o)| matches!(o.goal, P::MetaVar { .. }).then_some(i))
        .collect();
    m.dispatch(captures.len());
    for i in captures {
        let o = &mut ops[i];
        if let P::MetaVar { meta_var } = o.goal {
            outcomes[i] = if !(o.root && matches!(o.strict, S::Smart))
                && comments(o.strict)
                && o.node.is_extra()
            {
                Outcome::Candidate
            } else if bind(meta_var, &o.node, &mut o.env) {
                Outcome::Both
            } else {
                Outcome::Fail
            };
        }
    }
    let internals: Vec<_> = ops
        .iter()
        .enumerate()
        .filter_map(|(i, o)| matches!(o.goal, P::Internal { .. }).then_some(i))
        .collect();
    m.dispatch(internals.len());
    let mut children = Vec::new();
    let mut indices = Vec::new();
    for i in internals {
        let o = &ops[i];
        if let P::Internal {
            kind_id,
            children: goals,
        } = o.goal
        {
            if matches!(o.strict, S::Template)
                || kind_utils::are_kinds_matching(*kind_id, o.node.kind_id())
            {
                children.push(Cursor {
                    goals,
                    nodes: o.node.children().collect(),
                    g: 0,
                    c: 0,
                    strict: o.strict,
                    env: o.env.clone(),
                    done: None,
                    search: None,
                    single: false,
                });
                indices.push(i);
            }
        }
    }
    for (i, (ok, env)) in indices.into_iter().zip(sequences(children, m)) {
        outcomes[i] = if ok { Outcome::Both } else { Outcome::Fail };
        ops[i].env = env;
    }
    ops.into_iter()
        .zip(outcomes)
        .map(|(o, r)| (r, o.env))
        .collect()
}
struct Search<'p> {
    name: Option<&'p str>,
    start: usize,
    skipped: usize,
}
struct Cursor<'p, 't, D: Doc> {
    goals: &'p [P],
    nodes: Vec<Node<'t, D>>,
    g: usize,
    c: usize,
    strict: &'p S,
    env: Arc<MetaVarEnv<'t, D>>,
    done: Option<bool>,
    search: Option<Search<'p>>,
    single: bool,
}
fn capture_span<D: Doc>(
    s: &mut Cursor<'_, '_, D>,
    name: Option<&str>,
    start: usize,
    end: usize,
    skip: usize,
) -> bool {
    name.is_none_or(|n| {
        Arc::make_mut(&mut s.env)
            .insert_multi(
                n,
                s.nodes[start..end.saturating_sub(skip).max(start)].to_vec(),
            )
            .is_some()
    })
}
fn consumed<D: Doc>(s: &mut Cursor<'_, '_, D>) {
    s.single = false;
    if s.g < s.goals.len() {
        s.g += 1;
        s.c = (s.c + 1).min(s.nodes.len());
    }
    if s.g == s.goals.len() {
        s.done = Some(s.nodes[s.c..].iter().all(|n| trailing(s.strict, n)));
    } else if s.c == s.nodes.len() {
        s.done = Some(false);
    }
}
fn sequences<'p, 't, D: Doc>(
    mut states: Vec<Cursor<'p, 't, D>>,
    m: &mut Metrics,
) -> Vec<(bool, Arc<MetaVarEnv<'t, D>>)> {
    for s in &mut states {
        if s.nodes.is_empty() {
            s.done = Some(false);
        }
    }
    loop {
        let mut jobs = Vec::new();
        let mut owners = Vec::new();
        let width = states.iter().filter(|s| s.done.is_none()).count();
        if width == 0 {
            break;
        }
        m.dispatch(width);
        for (i, s) in states.iter_mut().enumerate() {
            if s.done.is_some() {
                continue;
            }
            if s.search.is_none() && !s.single {
                if s.g == s.goals.len() {
                    s.done = Some(true);
                    continue;
                }
                if let Some(name) = ellipsis(&s.goals[s.g]) {
                    s.g += 1;
                    let mut skipped = 0;
                    while s.g < s.goals.len() && s.goals[s.g].is_trivial() {
                        s.g += 1;
                        skipped += 1;
                    }
                    if s.g == s.goals.len() {
                        s.done = Some(capture_span(s, name, s.c, s.nodes.len(), skipped));
                        continue;
                    }
                    if ellipsis(&s.goals[s.g]).is_some() {
                        let start = s.c;
                        s.c += 1;
                        let ok = capture_span(s, name, start, s.c, skipped);
                        if !ok || s.c == s.nodes.len() {
                            s.done = Some(false);
                        }
                        continue;
                    }
                    s.search = Some(Search {
                        name,
                        start: s.c,
                        skipped,
                    });
                }
            }
            if s.c == s.nodes.len() {
                if s.search.is_some() {
                    s.done = Some(false);
                    continue;
                }
                if s.goals[s.g..].iter().all(|p| skippable(s.strict, p)) {
                    s.g = s.goals.len();
                    consumed(s);
                } else {
                    s.done = Some(false);
                }
                continue;
            }
            if s.search.is_none() {
                s.single = true;
            }
            owners.push(i);
            jobs.push(Op {
                goal: &s.goals[s.g],
                node: s.nodes[s.c].clone(),
                strict: s.strict,
                env: s.env.clone(),
                root: false,
            });
        }
        let results = operations(jobs, m);
        for (i, (result, env)) in owners.into_iter().zip(results) {
            let s = &mut states[i];
            if s.search.is_some() {
                if result == Outcome::Both {
                    let search = s.search.take().unwrap();
                    s.single = true;
                    if !capture_span(s, search.name, search.start, s.c, search.skipped) {
                        s.done = Some(false);
                    }
                    // Re-evaluate the anchor after committing the ellipsis capture,
                    // as ast-grep does (a repeated capture may now conflict).
                } else {
                    s.c += 1;
                }
                continue;
            }
            s.env = env;
            match result {
                Outcome::Both => consumed(s),
                Outcome::Fail => s.done = Some(false),
                Outcome::Candidate => s.c += 1,
                Outcome::Goal => {
                    s.g += 1;
                    if s.g == s.goals.len() {
                        consumed(s);
                    }
                }
                Outcome::SkipBoth => {
                    s.g += 1;
                    s.c += 1;
                    if s.g == s.goals.len() {
                        consumed(s);
                    }
                }
            }
        }
    }
    states
        .into_iter()
        .map(|s| (s.done.unwrap(), s.env))
        .collect()
}
pub(super) fn execute<'t, D: Doc>(
    requests: Vec<Request<'t, D>>,
    metrics: &mut Metrics,
) -> Vec<Option<Arc<MetaVarEnv<'t, D>>>> {
    let mut owners = Vec::with_capacity(requests.len());
    let mut ops = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        if request
            .pattern
            .contextual_root_kind
            .is_some_and(|kind| kind != request.node.kind_id())
        {
            continue;
        }
        owners.push(index);
        let pattern = &request.pattern.pattern;
        ops.push(Op {
            goal: &pattern.node,
            node: request.node.clone(),
            strict: &pattern.strictness,
            env: request.env.clone(),
            root: true,
        });
    }
    let mut results = vec![None; requests.len()];
    for (index, (outcome, env)) in owners.into_iter().zip(operations(ops, metrics)) {
        if outcome == Outcome::Both {
            results[index] = Some(env);
        }
    }
    results
}

/// Shares only exact immutable inputs; different capture frames always execute separately.
pub(super) fn execute_cached<'t, D: Doc>(
    requests: Vec<Request<'t, D>>,
    cache: &mut Cache<'t, D>,
    metrics: &mut Metrics,
) -> Vec<Option<Arc<MetaVarEnv<'t, D>>>> {
    let mut output = vec![None; requests.len()];
    let mut unique = Vec::new();
    let mut representatives = Vec::new();
    let mut owners: Vec<Vec<usize>> = Vec::new();
    let mut pending = FxHashMap::<CacheKey, usize>::default();
    for (index, request) in requests.iter().enumerate() {
        let key = cache_key(request);
        if let Some(entry) = cache.entries.get(&key)
            && let Some(input) = entry.input.upgrade()
            && Arc::ptr_eq(&input, &request.env)
        {
            metrics.cache_hits += 1;
            output[index] = entry.result.clone();
            continue;
        }
        metrics.cache_misses += 1;
        if let Some(&slot) = pending.get(&key) {
            metrics.batch_deduplications += 1;
            owners[slot].push(index);
            continue;
        }
        pending.insert(key, unique.len());
        representatives.push(index);
        owners.push(vec![index]);
        unique.push(Request {
            pattern: request.pattern.clone(),
            node: request.node.clone(),
            env: request.env.clone(),
        });
    }
    metrics.unique_executions += unique.len();
    for ((index, owners), result) in representatives
        .into_iter()
        .zip(owners)
        .zip(execute(unique, metrics))
    {
        let request = &requests[index];
        if cache.entries.len() >= 32768 {
            cache.entries.clear();
        }
        cache.entries.insert(
            cache_key(request),
            Cached {
                _plan: request.pattern.clone(),
                _node: request.node.clone(),
                input: Arc::downgrade(&request.env),
                result: result.clone(),
            },
        );
        for owner in owners {
            output[owner] = result.clone();
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::{AstGrep as Root, Matcher};
    use std::{borrow::Cow, collections::HashMap};

    #[test]
    fn contextual_rejections_skip_operations_and_keep_lane_order() {
        let doc = Root::str("foo;", GrammarDialect::JavaScript);
        let identifier = doc
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier")
            .unwrap();
        let plan = compile(Arc::new(
            Pattern::contextual("$A", "identifier", GrammarDialect::JavaScript).unwrap(),
        ));
        let env = Arc::new(MetaVarEnv::new());
        let mut metrics = Metrics::default();
        let rejected = execute(
            vec![Request {
                pattern: plan.clone(),
                node: doc.root(),
                env: env.clone(),
            }],
            &mut metrics,
        );
        assert!(rejected[0].is_none());
        assert_eq!(metrics.operation_lanes, 0);
        let mixed = execute(
            vec![
                Request {
                    pattern: plan.clone(),
                    node: doc.root(),
                    env: env.clone(),
                },
                Request {
                    pattern: plan,
                    node: identifier,
                    env,
                },
            ],
            &mut metrics,
        );
        assert!(mixed[0].is_none());
        assert_eq!(
            mixed[1].as_ref().unwrap().get_match("A").unwrap().text(),
            "foo"
        );
    }

    #[test]
    fn capture_free_success_retains_the_shared_environment() {
        let doc = Root::str("foo;", GrammarDialect::JavaScript);
        let node = doc
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier")
            .unwrap();
        let env = Arc::new(MetaVarEnv::new());
        let plan = compile(Arc::new(Pattern::new("foo", GrammarDialect::JavaScript)));
        let mut metrics = Metrics::default();
        let results = execute(
            vec![Request {
                pattern: plan,
                node,
                env: env.clone(),
            }],
            &mut metrics,
        );
        assert!(Arc::ptr_eq(results[0].as_ref().unwrap(), &env));
    }

    #[test]
    fn shared_patterns_keep_documents_inputs_and_failures_distinct() {
        let first = Root::str("foo(a,b,c);", GrammarDialect::JavaScript);
        let second = Root::str("foo(a,b,c);", GrammarDialect::JavaScript);
        let first_call = first
            .root()
            .dfs()
            .find(|node| node.kind() == "call_expression")
            .unwrap();
        let second_call = second
            .root()
            .dfs()
            .find(|node| node.kind() == "call_expression")
            .unwrap();
        let conflicting = first
            .root()
            .dfs()
            .find(|node| node.kind() == "identifier" && node.text() == "a")
            .unwrap();
        let mut plans = Plans::default();
        let unconstrained = plans.compile(Arc::new(Pattern::new("$A", GrammarDialect::JavaScript)));
        let contextual = plans.compile(Arc::new(
            Pattern::contextual("$A", "identifier", GrammarDialect::JavaScript).unwrap(),
        ));
        assert!(!Arc::ptr_eq(&unconstrained, &contextual));
        let mut cache = Cache::default();
        let mut metrics = Metrics::default();
        let env = Arc::new(MetaVarEnv::new());
        let mut bound = MetaVarEnv::new();
        bound.insert("F", conflicting).unwrap();
        let bound = Arc::new(bound);
        for strict in [
            S::Cst,
            S::Smart,
            S::Ast,
            S::Relaxed,
            S::Signature,
            S::Template,
        ] {
            let p = plans.compile(Arc::new(
                Pattern::new("$F($$$ARGS)", GrammarDialect::JavaScript)
                    .with_strictness(strict.clone()),
            ));
            let equivalent = plans.compile(Arc::new(
                Pattern::new("$F($$$ARGS)", GrammarDialect::JavaScript)
                    .with_strictness(strict.clone()),
            ));
            assert!(Arc::ptr_eq(&p, &equivalent));
            let request = |pattern: Arc<Plan>, node, env| Request { pattern, node, env };
            let results = execute_cached(
                vec![
                    request(p.clone(), first_call.clone(), env.clone()),
                    request(equivalent.clone(), first_call.clone(), env.clone()),
                    request(p.clone(), second_call.clone(), env.clone()),
                    request(p.clone(), first_call.clone(), bound.clone()),
                ],
                &mut cache,
                &mut metrics,
            );
            assert!(results[0].is_some());
            assert!(Arc::ptr_eq(
                results[0].as_ref().unwrap(),
                results[1].as_ref().unwrap()
            ));
            assert!(std::ptr::eq(
                results[0].as_ref().unwrap().get_match("F").unwrap().root(),
                first_call.root()
            ));
            assert!(std::ptr::eq(
                results[2].as_ref().unwrap().get_match("F").unwrap().root(),
                second_call.root()
            ));
            assert!(results[3].is_none());
            let hot = execute_cached(
                vec![request(equivalent, first_call.clone(), env.clone())],
                &mut cache,
                &mut metrics,
            );
            assert!(Arc::ptr_eq(
                results[0].as_ref().unwrap(),
                hot[0].as_ref().unwrap()
            ));
            let failing = plans.compile(Arc::new(
                Pattern::new("foo($$$A,b,$$$A)", GrammarDialect::JavaScript)
                    .with_strictness(strict),
            ));
            let failed = execute_cached(
                vec![request(failing.clone(), first_call.clone(), env.clone())],
                &mut cache,
                &mut metrics,
            );
            let hot_failed = execute_cached(
                vec![request(failing, first_call.clone(), env.clone())],
                &mut cache,
                &mut metrics,
            );
            assert!(failed[0].is_none());
            assert!(hot_failed[0].is_none());
            assert!(env.get_matched_variables().next().is_none());
        }
        assert_eq!(metrics.batch_deduplications, 6);
        assert_eq!(metrics.cache_hits, 12);
        assert_eq!(metrics.unique_executions, 24);
    }

    #[test]
    fn cohorts_agree_with_native_patterns() {
        let cases = [
            ("$A($A)", "foo(foo); foo(bar);"),
            ("foo($$$A, b, $$$C)", "foo(); foo(b); foo(a,b,c,);"),
            ("foo($A, $B)", "foo(/* before */ a, b /* after */,);"),
            ("foo($$$A, $$$B)", "foo(a,b,c);"),
            ("$COMMENT", "// hello\nfoo(a);"),
            ("foo($A,)", "foo(a); foo(a,);"),
            ("($P) => $F($P)", "(x) => bar(x); (x) => bar(y);"),
            (
                "function _() {\n $$$A\n let $P = g()\n let $Q = h()\n $$$B\n}",
                "function _() { let a = 0; let p = g(); let q = h(); let b = 1; }",
            ),
            ("foo($$$A, b, $$$A)", "foo(a,b,a); foo(a,b,c);"),
        ];
        let docs: Vec<_> = cases
            .iter()
            .map(|(_, code)| Root::str(code, GrammarDialect::JavaScript))
            .collect();
        let mut requests = Vec::new();
        let mut plans = Plans::default();
        for (index, (pattern, _)) in cases.iter().enumerate() {
            for strict in [
                S::Cst,
                S::Smart,
                S::Ast,
                S::Relaxed,
                S::Signature,
                S::Template,
            ] {
                let p = plans.compile(Arc::new(
                    Pattern::new(pattern, GrammarDialect::JavaScript).with_strictness(strict),
                ));
                for node in docs[index].root().dfs() {
                    requests.push(Request {
                        pattern: p.clone(),
                        node,
                        env: Arc::new(MetaVarEnv::new()),
                    });
                }
            }
        }
        // Contextual roots preserve an additional kind constraint, even when
        // Template matching ignores the kinds of ordinary pattern operations.
        for strict in [S::Smart, S::Template] {
            let pattern = plans.compile(Arc::new(
                Pattern::contextual("$A", "identifier", GrammarDialect::JavaScript)
                    .unwrap()
                    .with_strictness(strict),
            ));
            for node in docs[0].root().dfs() {
                requests.push(Request {
                    pattern: pattern.clone(),
                    node,
                    env: Arc::new(MetaVarEnv::new()),
                });
            }
        }
        let expected: Vec<_> = requests
            .iter()
            .map(|r| {
                let mut env = Cow::Owned((*r.env).clone());
                r.pattern
                    .pattern
                    .match_node_with_env(r.node.clone(), &mut env)
                    .map(|_| HashMap::<String, String>::from(env.into_owned()))
            })
            .collect();
        let mut metrics = Metrics::default();
        let actual: Vec<_> = execute_cached(requests, &mut Cache::default(), &mut metrics)
            .into_iter()
            .map(|e| e.map(|env| HashMap::<String, String>::from((*env).clone())))
            .collect();
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(a, e, "lane {i}");
        }
        assert!(metrics.max_width > 1);
        assert!(metrics.operation_lanes > metrics.operation_dispatches);
    }
}
