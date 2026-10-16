//! Functions compiled statements call. Each reads one node (and environment) the statement
//! already holds; terminal node sets come from `rerail_matches`' batch cache, never from a scan here.
//! A node belongs to a batch file or to a derived document (a parsed literal, `docs`).

use super::BatchState;
use super::catalog::Term;
use super::docs::{Document, batch_state, derived_chain, document};
use super::env::{Binding, Env, UNKNOWN};
use crate::code_rule_dsl::matcher::facts as predicates;
use crate::code_rule_dsl::matcher::{AstMatcher, Comparison, EqClass};
use crate::code_rule_dsl::transducer::sql_backend::{BatchFile, BatchView, MatchRegistry};
use ast_grep_core::matcher::{Matcher, MatcherExt};
use rusqlite::functions::{Aggregate, Context, FunctionFlags};
use rusqlite::{Connection, Result as SqlResult};
use std::borrow::Cow;
use std::sync::Arc;

fn user_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into().into())
}

#[derive(Clone, Copy)]
struct FileHook {
    data: *mut (),
    call: unsafe fn(*mut (), i64) -> SqlResult<bool>,
}

thread_local! {
    static FILE_HOOK: std::cell::Cell<Option<FileHook>> = const { std::cell::Cell::new(None) };

    /// The batch this thread's statements last named: `(registry, token, view)`. SQLite's
    /// per-argument auxiliary data would do the same, but it keeps one entry per call site and
    /// looks them up linearly, which dominates large statements.
    static CURRENT: std::cell::RefCell<Option<(usize, i64, Arc<BatchView>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Forgets this thread's cached batch view, releasing the batch it holds.
pub(super) fn forget_batch() {
    CURRENT.with(|current| current.borrow_mut().take());
}

/// Installs the file-context switch used by one synchronous all-file statement.
pub(super) fn with_file_hook<F, T>(hook: &mut F, run: impl FnOnce() -> T) -> T
where
    F: FnMut(i64) -> SqlResult<bool>,
{
    unsafe fn call<F>(data: *mut (), file: i64) -> SqlResult<bool>
    where
        F: FnMut(i64) -> SqlResult<bool>,
    {
        // SAFETY: `with_file_hook` keeps `hook` borrowed until `run` returns, and SQLite invokes
        // the function synchronously on this thread.
        unsafe { (&mut *data.cast::<F>())(file) }
    }
    let installed = FileHook {
        data: (hook as *mut F).cast(),
        call: call::<F>,
    };
    let previous = FILE_HOOK.replace(Some(installed));
    struct Restore(Option<FileHook>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FILE_HOOK.set(self.0.take());
        }
    }
    let _restore = Restore(previous);
    run()
}

/// The batch named by argument 0.
fn batch(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Arc<BatchView>> {
    let token = context.get::<i64>(0)?;
    let owner = registry as *const MatchRegistry as usize;
    let view = CURRENT.with(|current| -> SqlResult<Arc<BatchView>> {
        let mut current = current.borrow_mut();
        if let Some((cached_owner, cached_token, view)) = current.as_ref()
            && *cached_owner == owner
            && *cached_token == token
        {
            return Ok(Arc::clone(view));
        }
        let view = Arc::new(
            registry
                .files(token)
                .ok_or_else(|| user_error("query batch is missing"))?,
        );
        *current = Some((owner, token, Arc::clone(&view)));
        Ok(view)
    })?;
    if let Some(state) = batch_state(&view) {
        state
            .scalar_callbacks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(view)
}

fn state(view: &BatchView) -> SqlResult<&BatchState> {
    batch_state(view).ok_or_else(|| user_error("query batch has no state"))
}

fn term(state: &BatchState, index: i64) -> SqlResult<&Term> {
    usize::try_from(index)
        .ok()
        .and_then(|index| state.catalog.terms.get(index))
        .ok_or_else(|| user_error("query term is missing"))
}

/// An environment argument, as stored.
fn env_at(state: &BatchState, handle: i64) -> SqlResult<Arc<Env>> {
    state
        .envs
        .get(handle)
        .ok_or_else(|| user_error("environment is missing"))
}

/// A term function's arguments `(batch, term, file, node, env)`.
struct TermArguments<'a> {
    term: i64,
    file: i64,
    doc: Document<'a>,
    node: u32,
    env: i64,
}

/// The common head of a term function `(batch, term, file, node, env)`: `Err(result)` when the
/// result is decided without testing the node (no match coming in, an unknown verdict, or no node).
fn term_arguments<'a>(
    context: &Context<'_>,
    view: &'a BatchView,
) -> SqlResult<Result<TermArguments<'a>, Option<i64>>> {
    let Some(env) = context.get::<Option<i64>>(4)? else {
        return Ok(Err(None));
    };
    if env < 0 {
        return Ok(Err(Some(env)));
    }
    let (Some(file), Some(node)) = (
        context.get::<Option<i64>>(2)?,
        context.get::<Option<i64>>(3)?,
    ) else {
        return Ok(Err(None));
    };
    let Some(doc) = document(view, batch_state(view), file) else {
        return Ok(Err(None));
    };
    let Some(node) = u32::try_from(node)
        .ok()
        .filter(|&node| (node as usize) < doc.len())
    else {
        return Ok(Err(None));
    };
    Ok(Ok(TermArguments {
        term: context.get::<i64>(1)?,
        file,
        doc,
        node,
        env,
    }))
}

/// Evaluates `$body` with `$node` the node at ordinal `$at` of document `$doc`, `$meta` the
/// environment `$env` over that document, `$node_at` its nodes by ordinal and `$ordinal` a node
/// id's ordinal; a derived string type definition reads its names in its file table.
macro_rules! at_node {
    ($doc:expr, $at:expr, $env:expr, |$node:ident, $meta:ident, $node_at:ident, $ordinal:ident| $body:expr) => {
        match $doc {
            Document::File(file) => {
                let $node_at = |ordinal: u32| file.node(ordinal);
                let $node = file.node($at);
                #[allow(unused_mut)]
                let mut $meta = $env.to_meta(file);
                let $ordinal = |id: usize| id as u32;
                $body
            }
            Document::Derived(derived) => {
                let $node_at = |ordinal: u32| derived.node(ordinal);
                let $node = derived.node($at);
                #[allow(unused_mut)]
                let mut $meta = $env.meta_with(|ordinal| derived.node(ordinal));
                let $ordinal = super::docs::derived_ordinal;
                predicates::with_sub_table(derived.table.as_ref(), || $body)
            }
        }
    };
}

fn pattern(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let view = batch(context, registry)?;
    let arguments = match term_arguments(context, &view)? {
        Ok(arguments) => arguments,
        Err(result) => return Ok(result),
    };
    let state = state(&view)?;
    let Term::Pattern { pattern, terminal } = term(state, arguments.term)? else {
        return Err(user_error("tq_pattern term is not a pattern"));
    };
    let bound = env_at(state, arguments.env)?;
    let file_index = match (&arguments.doc, usize::try_from(arguments.file)) {
        (Document::File(_), Ok(file_index)) => Some(file_index),
        _ => None,
    };
    let direct = file_index.is_none() || pattern.labels.iter().any(|label| bound.binds(label));
    if direct {
        // A capture the pattern names is bound already, or the node is a parsed literal's (no
        // terminal ran over it): match as ast-grep does, at the node.
        return Ok(at_node!(
            &arguments.doc,
            arguments.node,
            bound,
            |node, meta, _node_at, ordinal| {
                let mut meta = Cow::Owned(meta);
                pattern
                    .pattern
                    .match_node_with_env(node, &mut meta)
                    .map(|_| {
                        state
                            .envs
                            .put(Env::from_meta_with(&meta, ordinal).with_state_of(&bound))
                    })
            }
        ));
    }
    let file_index = file_index.expect("a batch file");
    let Some(found) = view.terminal_at(*terminal, file_index, arguments.node) else {
        return Ok(None);
    };
    if found.captures.is_empty() {
        return Ok(Some(arguments.env));
    }
    let mut extended = (*bound).clone();
    for (label, nodes) in &found.captures {
        let label = usize::from(*label);
        let name = Arc::clone(&pattern.labels[label]);
        let binding = if pattern.multi[label] {
            Binding::Multi(nodes.clone())
        } else {
            Binding::Single(nodes[0])
        };
        extended = extended.with(name, binding);
    }
    Ok(Some(state.envs.put(extended)))
}

/// `tq_kind` and `tq_regex`: the environment when the node passes, NULL otherwise.
fn node_test(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let view = batch(context, registry)?;
    let arguments = match term_arguments(context, &view)? {
        Ok(arguments) => arguments,
        Err(result) => return Ok(result),
    };
    let passes = match (term(state(&view)?, arguments.term)?, &arguments.doc) {
        (Term::Kind(kind), Document::File(file)) => {
            kind.match_node(file.node(arguments.node)).is_some()
        }
        (Term::Kind(kind), Document::Derived(derived)) => {
            kind.match_node(derived.node(arguments.node)).is_some()
        }
        (Term::Regex(regex), Document::File(file)) => {
            let range = file.tape().transducer_span(arguments.node);
            regex.is_match(&file.tape().source_text()[range])
        }
        (Term::Regex(regex), Document::Derived(derived)) => {
            regex.is_match(&derived.node(arguments.node).text())
        }
        _ => {
            return Err(user_error(
                "tq_kind or tq_regex term is not a kind or regex",
            ));
        }
    };
    Ok(passes.then_some(arguments.env))
}

/// `tq_struct(batch, term, file, node, env)`: the environment where the structural kernel's
/// sub-rule matches the node (`structure`), unknown where its answer is (a `value` test in a
/// three-valued clause), NULL otherwise. A limit a value decode hit fails the statement with the
/// limit's message, as `tq_pred` does.
fn structure(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    use super::structure::{Failure, Truth};
    let view = batch(context, registry)?;
    let arguments = match term_arguments(context, &view)? {
        Ok(arguments) => arguments,
        Err(result) => return Ok(result),
    };
    let state = state(&view)?;
    let &Term::Structure(op) = term(state, arguments.term)? else {
        return Err(user_error("tq_struct term is not a structural sub-rule"));
    };
    let ops = &state.catalog.ops;
    let answer = match &arguments.doc {
        Document::File(file) => {
            let mut memos = state
                .structure
                .lock()
                .expect("structural kernel mutex poisoned");
            let memo = memos.entry(arguments.file).or_default();
            super::structure::test_file(ops, op, file, memo, arguments.node)
        }
        Document::Derived(derived) => predicates::with_sub_table(derived.table.as_ref(), || {
            super::structure::test_node(ops, op, &derived.node(arguments.node))
        }),
    };
    match answer {
        Ok(Truth::True) => Ok(Some(arguments.env)),
        Ok(Truth::Unknown) => Ok(Some(UNKNOWN)),
        Ok(Truth::False) => Ok(None),
        Err(Failure::Limit(limit)) => Err(user_error(limit.message())),
        Err(Failure::Transducer(error)) => Err(user_error(format!("structural kernel: {error:?}"))),
    }
}

/// `tq_search(pos, env, strict, stop, has)`: a three-valued relation's search
/// (`relation::Search`) over its candidates in walk order: the first match; an unknown candidate
/// leaves the answer open and ends a `strict` search (one whose rule binds captures). `stop` is
/// the candidate's stop verdict (NULL without a stop rule): for `inside`, `precedes` and
/// `follows` the walk ends after a stop node, openly when the verdict is unknown; for `has`
/// (`has` = 1) an unknown stop verdict opens the answer once the next candidate is tested.
#[derive(Default)]
struct Search;

#[derive(Default)]
struct SearchState {
    strict: bool,
    has: bool,
    unknown: bool,
    pending_open: bool,
    decided: bool,
    result: Option<i64>,
}

impl Aggregate<SearchState, Option<i64>> for Search {
    fn init(&self, _: &mut Context<'_>) -> SqlResult<SearchState> {
        Ok(SearchState::default())
    }

    fn step(&self, context: &mut Context<'_>, state: &mut SearchState) -> SqlResult<()> {
        let frame = context.get::<i64>(0)?;
        if state.decided {
            return Ok(());
        }
        state.strict = context.get::<i64>(3)? != 0;
        state.has = context.get::<i64>(5)? != 0;
        let verdict = context.get::<Option<i64>>(2)?;
        let stop = context.get::<Option<i64>>(4)?;
        match verdict {
            Some(env) if env >= 0 => {
                state.result = Some(env);
                state.decided = true;
            }
            Some(_) => {
                state.unknown = true;
                if state.strict {
                    state.result = Some(UNKNOWN);
                    state.decided = true;
                }
            }
            None => {}
        }
        if !state.decided {
            if state.has {
                if state.pending_open {
                    state.unknown = true;
                    if state.strict {
                        state.result = Some(UNKNOWN);
                        state.decided = true;
                    }
                }
                state.pending_open = stop.is_some_and(|stop| stop < 0);
            } else {
                match stop {
                    Some(stop) if stop >= 0 => {
                        state.result = state.unknown.then_some(UNKNOWN);
                        state.decided = true;
                    }
                    Some(_) => {
                        state.unknown = true;
                        state.result = Some(UNKNOWN);
                        state.decided = true;
                    }
                    None => {}
                }
            }
        }
        if state.decided {
            walk_close(frame);
        }
        Ok(())
    }

    fn finalize(&self, _: &mut Context<'_>, state: Option<SearchState>) -> SqlResult<Option<i64>> {
        let Some(state) = state else {
            return Ok(None);
        };
        if state.decided {
            return Ok(state.result);
        }
        Ok((state.unknown || state.pending_open).then_some(UNKNOWN))
    }
}

/// `tq_count` accumulates a three-valued count, closing its frame as soon as either bound decides.
/// Its last argument is the row's stop verdict: an unknown one leaves the count open above.
#[derive(Default)]
struct Count;

#[derive(Default)]
struct CountState {
    env: Option<i64>,
    min: i64,
    max: i64,
    found: i64,
    unknown: i64,
    /// An unknown stop verdict ended the walk or left a subtree unentered: nodes past it may
    /// count, so `found + unknown` is no upper bound.
    open: bool,
    result: Option<i64>,
}

impl Aggregate<CountState, Option<i64>> for Count {
    fn init(&self, _: &mut Context<'_>) -> SqlResult<CountState> {
        Ok(CountState::default())
    }

    fn step(&self, context: &mut Context<'_>, state: &mut CountState) -> SqlResult<()> {
        let frame = context.get::<i64>(0)?;
        state.env = context.get(1)?;
        state.min = context.get(2)?;
        state.max = context.get(3)?;
        match context.get::<Option<i64>>(4)? {
            Some(verdict) if verdict >= 0 => state.found += 1,
            Some(_) => state.unknown += 1,
            None => {}
        }
        if context.get::<Option<i64>>(5)?.is_some_and(|stop| stop < 0) {
            state.open = true;
        }
        if state.max < 0 && state.found >= state.min {
            state.result = state.env;
        } else if state.max >= 0 && state.found > state.max {
            state.result = None;
        }
        if state.result.is_some() || (state.max >= 0 && state.found > state.max) {
            walk_close(frame);
        }
        Ok(())
    }

    fn finalize(&self, _: &mut Context<'_>, state: Option<CountState>) -> SqlResult<Option<i64>> {
        let Some(state) = state else {
            return Ok(None);
        };
        if state.result.is_some() {
            return Ok(state.result);
        }
        // The true count lies in `found..=found + unknown`, unbounded above when the walk is open.
        let high = (!state.open).then_some(state.found + state.unknown);
        let holds = state.found >= state.min
            && (state.max < 0 || high.is_some_and(|high| high <= state.max));
        let fails = high.is_some_and(|high| high < state.min)
            || (state.max >= 0 && state.found > state.max);
        Ok(if holds {
            state.env
        } else if fails {
            None
        } else {
            Some(UNKNOWN)
        })
    }
}

/// A predicate's verdict as the statement reads it: its environment, NULL, or unknown.
fn verdict<'t, D>(
    state: &BatchState,
    bound: &Env,
    verdict: Result<
        crate::code_rule_dsl::matcher::Verdict<'t, D>,
        crate::code_rule_dsl::matcher::UtilLimit,
    >,
    ordinal: impl Fn(usize) -> u32,
) -> SqlResult<Option<i64>>
where
    D: ast_grep_core::Doc,
    D::Source: ast_grep_core::source::Content<Underlying = u8>,
{
    match verdict.map_err(|limit| user_error(limit.message()))? {
        crate::code_rule_dsl::matcher::Verdict::Match(meta) => {
            Ok(Some(state.envs.put(
                Env::from_meta_with(&meta, ordinal).with_state_of(bound),
            )))
        }
        crate::code_rule_dsl::matcher::Verdict::NoMatch => Ok(None),
        crate::code_rule_dsl::matcher::Verdict::Unknown => Ok(Some(UNKNOWN)),
    }
}

/// `tq_pred(batch, term, file, node, env, three)`: a Rerail predicate without sub-rules, tested at
/// one node by its own code; one it cannot decide is unknown when `three` (the clause is
/// three-valued), else no match. A limit it hits fails the statement with the limit's message.
fn predicate(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let view = batch(context, registry)?;
    let arguments = match term_arguments(context, &view)? {
        Ok(arguments) => arguments,
        Err(result) => return Ok(result),
    };
    let state = state(&view)?;
    let three = context.get::<i64>(5)? != 0;
    let Term::Predicate(matcher) = term(state, arguments.term)? else {
        return Err(user_error("tq_pred term is not a predicate"));
    };
    let bound = env_at(state, arguments.env)?;
    let batch_file = match &arguments.doc {
        Document::File(file) => Some(file),
        Document::Derived(_) => None,
    };
    at_node!(
        &arguments.doc,
        arguments.node,
        bound,
        |node, meta, _node_at, ordinal| verdict(
            state,
            &bound,
            predicates::test_predicate(matcher, node, meta, three, |node| match batch_file {
                Some(file) => {
                    let node = node.node_id() as u32;
                    let children = named_children(state, arguments.file, file, node)?;
                    let index = children.counted.binary_search(&node).ok()?;
                    Some((index, children.counted.len()))
                }
                // A parsed literal's tree is small: its named siblings are read from the node.
                None => {
                    let named = node
                        .parent()?
                        .children()
                        .filter(|child| child.is_named())
                        .map(|child| child.node_id())
                        .collect::<Vec<_>>();
                    let index = named.iter().position(|&id| id == node.node_id())?;
                    Some((index, named.len()))
                }
            }),
            ordinal
        )
    )
}

/// The siblings `nthChild` counts under one parent, in order: its named children, or those its
/// `ofRule` matches. `unknown` where a sibling's verdict was, which leaves every position under
/// the parent unknown.
pub(super) struct Siblings {
    counted: Vec<u32>,
    unknown: bool,
}

#[cfg(test)]
thread_local! {
    static NAMED_SIBLING_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[allow(dead_code, reason = "retained for SQL traversal profiling")]
pub(in crate::code_rule_dsl) fn reset_named_sibling_visits() {
    NAMED_SIBLING_VISITS.set(0);
}

/// Named children `nthChild` listed on this thread.
#[cfg(test)]
#[allow(dead_code, reason = "retained for SQL traversal profiling")]
pub(in crate::code_rule_dsl) fn named_sibling_visits() -> usize {
    NAMED_SIBLING_VISITS.get()
}

/// The named children of the node's parent, listed once per batch, file and parent
/// (`BatchState::siblings`). `None` for a root.
fn named_children(
    state: &BatchState,
    file_index: i64,
    file: &BatchFile,
    node: u32,
) -> Option<Arc<Siblings>> {
    let parent = file.node(node).parent()?.node_id() as u32;
    let key = (file_index, parent, None);
    if let Some(children) = state.siblings.lock().get(&key) {
        return Some(Arc::clone(children));
    }
    let counted = file
        .node(parent)
        .children()
        .filter(|child| child.is_named())
        .map(|child| {
            #[cfg(test)]
            NAMED_SIBLING_VISITS.set(NAMED_SIBLING_VISITS.get() + 1);
            child.node_id() as u32
        })
        .collect();
    let children = Arc::new(Siblings {
        counted,
        unknown: false,
    });
    state.siblings.lock().insert(key, Arc::clone(&children));
    Some(children)
}

/// `tq_nth_counted(batch, term, probe, file, node)`: `nthChild` with an `ofRule` that reads and
/// binds no capture, so its verdict at a sibling is the same for every candidate. Each parent's
/// named children are probed once per batch (`probe`, the rule's own statement, a step each, in
/// order until one is unknown); a candidate then reads its position from the cached siblings:
/// unknown, NULL where the rule does not count it, else 1 or 0 as the position holds.
fn nth_counted(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let term_index = context.get::<i64>(1)?;
    let Term::Predicate(crate::code_rule_dsl::matcher::AstMatcher::NthChild(nth)) =
        term(state, term_index)?
    else {
        return Err(user_error("tq_nth_counted term is not nthChild"));
    };
    let Term::Util { sql: probe, .. } = term(state, context.get::<i64>(2)?)? else {
        return Err(user_error("tq_nth_counted probe is not a statement"));
    };
    let (Some(file_index), Some(node)) = (
        context.get::<Option<i64>>(3)?,
        context.get::<Option<i64>>(4)?,
    ) else {
        return Ok(None);
    };
    let Some(Document::File(file)) = document(&view, batch_state(&view), file_index) else {
        return Ok(None);
    };
    let Some(node) = u32::try_from(node)
        .ok()
        .filter(|&node| (node as usize) < file.len())
    else {
        return Ok(None);
    };
    let Some(parent) = file.node(node).parent() else {
        return Ok(None);
    };
    let key = (file_index, parent.node_id() as u32, Some(term_index));
    let cached = state.siblings.lock().get(&key).cloned();
    let siblings = match cached {
        Some(siblings) => siblings,
        None => {
            // SAFETY: the handle is used only for these probes, on the connection running them.
            let db = unsafe { context.get_connection()?.handle() };
            let batch_token = context.get::<i64>(0)?;
            let mut siblings = Siblings {
                counted: Vec::new(),
                unknown: false,
            };
            for child in parent.children().filter(|child| child.is_named()) {
                if !crate::code_rule_dsl::matcher::spend_util_step() {
                    return Err(user_error(
                        crate::code_rule_dsl::matcher::UtilLimit::Work.message(),
                    ));
                }
                let child = child.node_id() as u32;
                // A fresh environment: the rule reads no capture.
                let parameters = [
                    Some(batch_token),
                    Some(file_index),
                    Some(i64::from(child)),
                    Some(0),
                ];
                match super::recursion::run(db, probe, &parameters).map_err(user_error)? {
                    Some(verdict) if verdict < 0 => {
                        siblings.unknown = true;
                        break;
                    }
                    Some(_) => siblings.counted.push(child),
                    None => {}
                }
            }
            let siblings = Arc::new(siblings);
            state.siblings.lock().insert(key, Arc::clone(&siblings));
            siblings
        }
    };
    if siblings.unknown {
        return Ok(Some(UNKNOWN));
    }
    Ok(siblings
        .counted
        .binary_search(&node)
        .ok()
        .map(|index| i64::from(nth.index_holds(index, siblings.counted.len()))))
}

/// One parent's children by their class under one `eq` comparison (`EqClass`), each list in
/// preorder: classified once per batch, file, parent and comparison (`BatchState::same_keys`).
pub(super) struct SameKeys {
    children: Vec<u32>,
    texts: std::collections::HashMap<Vec<u16>, Vec<u32>>,
    unknown: Vec<u32>,
}

impl SameKeys {
    fn classify(comparison: &Comparison, file: &BatchFile, parent: u32) -> Self {
        let mut keys = Self {
            children: Vec::new(),
            texts: std::collections::HashMap::new(),
            unknown: Vec::new(),
        };
        for child in file.tape().transducer_children(parent) {
            keys.children.push(child);
            match comparison.eq_class(&file.node(child)) {
                EqClass::Text(text) => keys.texts.entry(text).or_default().push(child),
                EqClass::Unknown => keys.unknown.push(child),
                EqClass::Never => {}
            }
        }
        keys
    }

    /// The children a node of `class` may compare equal to, or unknown with, in preorder.
    fn matching(&self, class: Option<EqClass>) -> Cow<'_, [u32]> {
        match class {
            None | Some(EqClass::Unknown) => Cow::Borrowed(&self.children),
            Some(EqClass::Never) => Cow::Borrowed(&self.unknown),
            Some(EqClass::Text(text)) => match self.texts.get(&text) {
                None => Cow::Borrowed(&self.unknown),
                Some(same) if self.unknown.is_empty() => Cow::Borrowed(same),
                Some(same) => {
                    let mut merged = [same.as_slice(), &self.unknown].concat();
                    merged.sort_unstable();
                    Cow::Owned(merged)
                }
            },
        }
    }
}

/// `tq_same_key(batch, term, file, of, env, outside)`: the children of `of`'s parent, as a JSON
/// array in preorder, that a sibling relation's item comparing them with the capture `outside`
/// under `term` (`Compiler::same_key`) may match: those whose class is that capture's, or unknown.
/// The others fail the comparison, and with it the item. Every child where the comparison does
/// not read them so: the environment is unknown, `outside` holds no node, or the item's own
/// capture is bound already (its pattern then compares texts, and the comparison reads that
/// binding rather than the sibling).
fn same_key(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<String> {
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let term_index = context.get::<i64>(1)?;
    let Term::Predicate(AstMatcher::Compare(comparison)) = term(state, term_index)? else {
        return Err(user_error("tq_same_key term is not a comparison"));
    };
    let (Some(file_index), Some(of)) = (
        context.get::<Option<i64>>(2)?,
        context.get::<Option<i64>>(3)?,
    ) else {
        return Ok("[]".to_owned());
    };
    let Some(doc) = document(&view, batch_state(&view), file_index) else {
        return Ok("[]".to_owned());
    };
    let Some(of) = u32::try_from(of)
        .ok()
        .filter(|&of| (of as usize) < doc.len())
    else {
        return Ok("[]".to_owned());
    };
    let file = match doc {
        Document::File(file) => file,
        // A parsed literal's tree is small: every child of the parent.
        Document::Derived(derived) => {
            let children = derived
                .node(of)
                .parent()
                .map(|parent| {
                    parent
                        .children()
                        .map(|child| super::docs::derived_ordinal(child.node_id()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Ok(json_nodes(&children));
        }
    };
    let Some(parent) = file.tape().transducer_parent(of) else {
        return Ok("[]".to_owned());
    };
    let outside = context.get::<String>(5)?;
    let inside = if comparison.left == outside {
        &comparison.right
    } else {
        &comparison.left
    };
    let class = match context.get::<Option<i64>>(4)? {
        Some(env) if env >= 0 => {
            let env = env_at(state, env)?;
            env.single(&outside)
                .filter(|&node| !env.binds(inside) && (node as usize) < file.len())
                .map(|node| comparison.eq_class(&file.node(node)))
        }
        _ => None,
    };
    let key = (file_index, parent, term_index);
    let cached = state.same_keys.lock().get(&key).cloned();
    let keys = match cached {
        Some(keys) => keys,
        None => {
            let keys = Arc::new(SameKeys::classify(comparison, file, parent));
            state.same_keys.lock().insert(key, Arc::clone(&keys));
            keys
        }
    };
    Ok(json_nodes(&keys.matching(class)))
}

fn json_nodes(nodes: &[u32]) -> String {
    let mut json = String::with_capacity(2 + nodes.len() * 6);
    json.push('[');
    for (index, node) in nodes.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&node.to_string());
    }
    json.push(']');
    json
}

/// `tq_fresh(batch, file, verdict)`: the verdict, or NULL (no match) where an enumeration is
/// excluding and it binds the enumerated capture to a range already reported. Nodes of a derived
/// document are read by their ranges in the file.
fn fresh(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let Some(verdict) = context.get::<Option<i64>>(2)? else {
        return Ok(None);
    };
    if verdict < 0 {
        return Ok(Some(verdict));
    }
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let file = context.get::<i64>(1)?;
    let env = env_at(state, verdict)?;
    let excluded = match document(&view, Some(state), file) {
        None => false,
        Some(Document::File(batch_file)) => {
            crate::code_rule_dsl::matcher::each_excluded(&env.to_meta(batch_file))
        }
        Some(Document::Derived(derived)) => {
            let mut trees = derived_chain(&view, state, file)
                .iter()
                .map(|derived| Arc::clone(derived.tree()))
                .collect::<Vec<_>>();
            trees.reverse();
            predicates::excluded_in(&env.meta_with(|ordinal| derived.node(ordinal)), &trees)
        }
    };
    Ok((!excluded).then_some(verdict))
}

/// `tq_follow(batch, node, key, env)`: the environment with a binding sub-rule `key` followed to
/// `node`; NULL (no match) where that sub-rule is already being matched there.
fn follow(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let Some(handle) = context.get::<Option<i64>>(3)? else {
        return Ok(None);
    };
    if handle < 0 {
        return Ok(Some(handle));
    }
    let (Some(node), key) = (context.get::<Option<i64>>(1)?, context.get::<i64>(2)?) else {
        return Ok(None);
    };
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let env = env_at(state, handle)?;
    let (node, key) = (node as u32, key as u64);
    if env.follows(node, key) {
        return Ok(None);
    }
    Ok(Some(state.envs.put(env.follow(node, key))))
}

/// `tq_unfollow(batch, result, before)`: a sub-rule's result with the followed bindings `before`
/// had, once it is matched.
fn unfollow(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let Some(result) = context.get::<Option<i64>>(1)? else {
        return Ok(None);
    };
    if result < 0 {
        return Ok(Some(result));
    }
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let (after, before) = (
        env_at(state, result)?,
        env_at(state, context.get::<i64>(2)?)?,
    );
    if after.following == before.following {
        return Ok(Some(result));
    }
    let mut restored = (*after).clone();
    restored.following.clone_from(&before.following);
    Ok(Some(state.envs.put(restored)))
}

/// `tq_following(batch, env)`: a fresh environment that keeps `env`'s followed bindings (what a
/// stop rule, a parsed tree's rule or a frame starts from).
fn following(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let handle = context.get::<Option<i64>>(1)?.unwrap_or(0);
    if handle <= 0 {
        return Ok(Some(0));
    }
    let view = batch(context, registry)?;
    let state = state(&view)?;
    Ok(Some(
        state.envs.put(Env::following_of(&*env_at(state, handle)?)),
    ))
}

/// `tq_call(batch, term, file, node, env)`: a parameterized call, its utility's statement run in a
/// frame (`matcher::facts::call`).
fn call(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let view = batch(context, registry)?;
    let arguments = match term_arguments(context, &view)? {
        Ok(arguments) => arguments,
        Err(result) => return Ok(result),
    };
    let state = state(&view)?;
    let Term::Call { body, call } = term(state, arguments.term)? else {
        return Err(user_error("tq_call term is not a call"));
    };
    let Term::Util { sql, .. } = term(state, *body as i64)? else {
        return Err(user_error("tq_call body is not a utility"));
    };
    let bound = env_at(state, arguments.env)?;
    let fresh = Env::following_of(&bound);
    // SAFETY: the handle is used only for this call, on the connection running it.
    let db = unsafe { context.get_connection()?.handle() };
    let parameters = |frame: i64| {
        [
            Some(context.get::<i64>(0).unwrap_or_default()),
            Some(arguments.file),
            Some(i64::from(arguments.node)),
            Some(frame),
        ]
    };
    let mut failure = None;
    let result = at_node!(
        &arguments.doc,
        arguments.node,
        bound,
        |_node, meta, node_at, ordinal| {
            let called = predicates::call(
                call,
                &meta,
                ast_grep_core::meta_var::MetaVarEnv::new(),
                |frame| {
                    let handle = state
                        .envs
                        .put(Env::from_meta_with(&frame, ordinal).with_state_of(&fresh));
                    match super::recursion::run(db, sql, &parameters(handle)) {
                        Err(message) => {
                            failure = Some(message);
                            Ok(None)
                        }
                        Ok(None) => Ok(None),
                        Ok(Some(result)) if result < 0 => Err(()),
                        Ok(Some(result)) => Ok(state
                            .envs
                            .get(result)
                            .map(|matched| matched.meta_with(node_at))),
                    }
                },
            );
            match called {
                Err(()) => Some(UNKNOWN),
                Ok(None) => None,
                Ok(Some(meta)) => Some(
                    state
                        .envs
                        .put(Env::from_meta_with(&meta, ordinal).with_state_of(&bound)),
                ),
            }
        }
    );
    if let Some(message) = failure {
        return Err(user_error(message));
    }
    Ok(result)
}

/// `tq_export(batch, term, host_file, host_env, doc, inner)`: after a parsed tree's rule (`inner`,
/// over derived document `doc`) matched, the host environment with the tree's captures exported
/// (`parse`: node captures and what nested trees exported; `formatArgs`: node captures).
fn export(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let Some(inner) = context.get::<Option<i64>>(5)? else {
        return Ok(None);
    };
    if inner < 0 {
        return Ok(Some(inner));
    }
    let Some(host) = context.get::<Option<i64>>(3)? else {
        return Ok(None);
    };
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let Some((names, nested)) = (match term(state, context.get::<i64>(1)?)? {
        Term::Semantic(AstMatcher::Semantic(semantic)) => semantic.exports(),
        _ => None,
    }) else {
        return Err(user_error("tq_export term exports nothing"));
    };
    let Some(Document::Derived(derived)) = document(&view, Some(state), context.get::<i64>(4)?)
    else {
        return Err(user_error("tq_export document is not a parsed literal"));
    };
    let inner = env_at(state, inner)?.meta_with(|ordinal| derived.node(ordinal));
    let bound = env_at(state, host)?;
    let Some(host_doc) = document(&view, Some(state), context.get::<i64>(2)?) else {
        return Ok(None);
    };
    Ok(Some(at_node!(
        &host_doc,
        0,
        bound,
        |_node, meta, _node_at, ordinal| {
            predicates::export_tree(&mut meta, &inner, derived.tree(), names, nested);
            state
                .envs
                .put(Env::from_meta_with(&meta, ordinal).with_state_of(&bound))
        }
    )))
}

/// `tq_host_env(batch, doc, inner, host_file, host_env)`: the environment a `host` rule runs in:
/// the host's as its `parse` began (`host_env`), with the captures the tree's rule bound so far
/// (`inner`, over derived document `doc`) exported into it.
fn host_env(context: &Context<'_>, registry: &MatchRegistry) -> SqlResult<Option<i64>> {
    let (Some(inner), Some(host)) = (
        context.get::<Option<i64>>(2)?,
        context.get::<Option<i64>>(4)?,
    ) else {
        return Ok(None);
    };
    if inner < 0 || host < 0 {
        return Ok(None);
    }
    let view = batch(context, registry)?;
    let state = state(&view)?;
    let Some(Document::Derived(derived)) = document(&view, Some(state), context.get::<i64>(1)?)
    else {
        return Err(user_error("tq_host_env document is not a parsed literal"));
    };
    let inner = env_at(state, inner)?.meta_with(|ordinal| derived.node(ordinal));
    let bound = env_at(state, host)?;
    let Some(host_doc) = document(&view, Some(state), context.get::<i64>(3)?) else {
        return Ok(None);
    };
    Ok(Some(at_node!(
        &host_doc,
        0,
        bound,
        |_node, meta, _node_at, ordinal| {
            predicates::host_env(&mut meta, &inner, derived.tree());
            state
                .envs
                .put(Env::from_meta_with(&meta, ordinal).with_state_of(&bound))
        }
    )))
}

/// `tq_nth(batch, term, frame, is_self, verdict)`: `nthChild` with `ofRule` over the parent's
/// named children in order, each with whether it is the node and its `ofRule` verdict: unknown
/// where a sibling's verdict was (`frame` noted it), NULL where the rule does not count the node,
/// else 1 or 0 as the position holds.
struct Nth {
    registry: Arc<MatchRegistry>,
}

#[derive(Default)]
struct NthState {
    nth: Option<crate::code_rule_dsl::matcher::NthChild>,
    frame: i64,
    counted: usize,
    index: Option<usize>,
}

impl Aggregate<NthState, Option<i64>> for Nth {
    fn init(&self, _: &mut Context<'_>) -> SqlResult<NthState> {
        Ok(NthState::default())
    }

    fn step(&self, context: &mut Context<'_>, state: &mut NthState) -> SqlResult<()> {
        if state.nth.is_none() {
            let view = batch(context, &self.registry)?;
            let Term::Predicate(crate::code_rule_dsl::matcher::AstMatcher::NthChild(nth)) =
                term(self::state(&view)?, context.get::<i64>(1)?)?
            else {
                return Err(user_error("tq_nth term is not nthChild"));
            };
            state.nth = Some((**nth).clone());
        }
        state.frame = context.get::<i64>(2)?;
        if context
            .get::<Option<i64>>(4)?
            .is_some_and(|verdict| verdict >= 0)
        {
            if context.get::<i64>(3)? != 0 {
                state.index = Some(state.counted);
            }
            state.counted += 1;
        }
        Ok(())
    }

    fn finalize(&self, _: &mut Context<'_>, state: Option<NthState>) -> SqlResult<Option<i64>> {
        let Some(state) = state else {
            return Ok(None);
        };
        let depth = super::recursion::depth();
        if ANY_UNKNOWN.with(|marks| marks.borrow_mut().remove(&(state.frame, depth))) {
            return Ok(Some(UNKNOWN));
        }
        let (Some(nth), Some(index)) = (state.nth, state.index) else {
            return Ok(None);
        };
        Ok(Some(i64::from(nth.index_holds(index, state.counted))))
    }
}

/// An environment argument of a three-valued `all`'s state function, with its batch state.
fn all_state(
    context: &Context<'_>,
    registry: &MatchRegistry,
    index: usize,
) -> SqlResult<Option<(Arc<BatchView>, Arc<Env>)>> {
    let Some(handle) = context.get::<Option<i64>>(index)? else {
        return Ok(None);
    };
    let view = batch(context, registry)?;
    let env = state(&view)?
        .envs
        .get(handle)
        .ok_or_else(|| user_error("environment is missing"))?;
    Ok(Some((view, env)))
}

fn register_all(connection: &Connection, registry: &Arc<MatchRegistry>) -> SqlResult<()> {
    let flags = FunctionFlags::SQLITE_UTF8;
    // tq_all_begin(batch, env): a fresh state (no withheld terms, nothing unknown).
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_all_begin", 2, flags, move |context| {
        let Some((view, env)) = all_state(context, &owner, 1)? else {
            return Ok(None);
        };
        if env.withheld == 0 && !env.unknown {
            return context.get::<Option<i64>>(1);
        }
        let fresh = Env::from_bindings(&env);
        Ok(Some(state(&view)?.envs.put(fresh)))
    })?;
    // tq_all_withheld(batch, state, mask): whether a term reads a withheld capture.
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_all_withheld", 3, flags, move |context| {
        let mask = context.get::<i64>(2)? as u64;
        Ok(all_state(context, &owner, 1)?.is_some_and(|(_, env)| env.withheld & mask != 0))
    })?;
    // tq_all_mark(batch, state, bit): the state with a skipped term's captures withheld.
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_all_mark", 3, flags, move |context| {
        let bit = context.get::<i64>(2)? as u64;
        let Some((view, env)) = all_state(context, &owner, 1)? else {
            return Ok(None);
        };
        let mut marked = (*env).clone();
        marked.withheld |= bit;
        Ok(Some(state(&view)?.envs.put(marked)))
    })?;
    // tq_all_after(batch, state, verdict, bit): the state after a term: no match ends the `all`;
    // unknown records it (withholding the term's captures when `bit`); a match continues with the
    // term's environment.
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_all_after", 4, flags, move |context| {
        let Some(verdict) = context.get::<Option<i64>>(2)? else {
            return Ok(None);
        };
        let bit = context.get::<i64>(3)? as u64;
        let Some((view, before)) = all_state(context, &owner, 1)? else {
            return Ok(None);
        };
        let envs = &state(&view)?.envs;
        if verdict < 0 {
            let mut after = (*before).clone();
            after.unknown = true;
            after.withheld |= bit;
            return Ok(Some(envs.put(after)));
        }
        let matched = envs
            .get(verdict)
            .ok_or_else(|| user_error("environment is missing"))?;
        Ok(Some(envs.put(matched.with_state_of(&before))))
    })?;
    // tq_all_end(batch, state): the `all`'s verdict: unknown if a term was, else its environment.
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_all_end", 2, flags, move |context| {
        let Some((view, env)) = all_state(context, &owner, 1)? else {
            return Ok(None);
        };
        if env.unknown {
            return Ok(Some(UNKNOWN));
        }
        if env.withheld == 0 {
            return context.get::<Option<i64>>(1);
        }
        Ok(Some(state(&view)?.envs.put(Env::from_bindings(&env))))
    })
}

thread_local! {
    /// Three-valued `any` expressions (by frame and recursion depth) that saw an unknown branch.
    static ANY_UNKNOWN: std::cell::RefCell<std::collections::HashSet<(i64, u32)>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    /// Walk frames whose aggregate has reached a decisive verdict.
    static WALK_DONE: std::cell::RefCell<std::collections::HashSet<(i64, u32)>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

fn walk_close(frame: i64) {
    let depth = super::recursion::depth();
    WALK_DONE.with(|frames| {
        frames.borrow_mut().insert((frame, depth));
    });
}

/// Forgets transient frames of a statement that ended early.
pub(super) fn reset_any_frames() {
    ANY_UNKNOWN.with(|marks| marks.borrow_mut().clear());
    WALK_DONE.with(|frames| frames.borrow_mut().clear());
}

fn register_any_and_steps(connection: &Connection) -> SqlResult<()> {
    let flags = FunctionFlags::SQLITE_UTF8;
    // tq_any_try(frame, verdict, strict): a match passes; an unknown branch is noted, then
    // skipped, unless it is `strict` (it may bind captures): a later branch must not stand in for
    // the captures it would have bound, so it ends the `any` as unknown.
    connection.create_scalar_function("tq_any_try", 3, flags, |context| {
        let frame = context.get::<i64>(0)?;
        let strict = context.get::<i64>(2)? != 0;
        Ok(match context.get::<Option<i64>>(1)? {
            Some(verdict) if verdict >= 0 => Some(verdict),
            Some(_) if strict => Some(UNKNOWN),
            Some(_) => {
                let depth = super::recursion::depth();
                ANY_UNKNOWN.with(|marks| marks.borrow_mut().insert((frame, depth)));
                None
            }
            None => None,
        })
    })?;
    // tq_any(frame, result): the first match, else unknown when a branch was.
    connection.create_scalar_function("tq_any", 2, flags, |context| {
        let frame = context.get::<i64>(0)?;
        let depth = super::recursion::depth();
        let unknown = ANY_UNKNOWN.with(|marks| marks.borrow_mut().remove(&(frame, depth)));
        Ok(match context.get::<Option<i64>>(1)? {
            Some(verdict) => Some(verdict),
            None => unknown.then_some(UNKNOWN),
        })
    })?;
    // tq_frame_open(frame): whether a lazily evaluated walk under `frame` saw an unknown verdict,
    // after which it evaluates nothing more (`nthChild` with `ofRule` stops at an unknown sibling).
    connection.create_scalar_function("tq_frame_open", 1, flags, |context| {
        let frame = context.get::<i64>(0)?;
        let depth = super::recursion::depth();
        Ok(ANY_UNKNOWN.with(|marks| marks.borrow().contains(&(frame, depth))))
    })?;
    // tq_frame_note(frame, verdict): the verdict, noting it under `frame` when it is unknown.
    connection.create_scalar_function("tq_frame_note", 2, flags, |context| {
        let frame = context.get::<i64>(0)?;
        let verdict = context.get::<Option<i64>>(1)?;
        if verdict.is_some_and(|verdict| verdict < 0) {
            let depth = super::recursion::depth();
            ANY_UNKNOWN.with(|marks| marks.borrow_mut().insert((frame, depth)));
        }
        Ok(verdict)
    })?;
    // tq_walk_start(frame) clears a prior evaluation's decision before its scan begins.
    connection.create_scalar_function("tq_walk_start", 1, flags, |context| {
        let frame = context.get::<i64>(0)?;
        let depth = super::recursion::depth();
        WALK_DONE.with(|frames| frames.borrow_mut().remove(&(frame, depth)));
        Ok(true)
    })?;
    // tq_walk_live(frame) gates a row after the aggregate decided an earlier one.
    connection.create_scalar_function("tq_walk_live", 1, flags, |context| {
        let frame = context.get::<i64>(0)?;
        let depth = super::recursion::depth();
        Ok(WALK_DONE.with(|frames| !frames.borrow().contains(&(frame, depth))))
    })?;
    // tq_step(): spends one step of the clause's utility and count budget; fails once spent.
    connection.create_scalar_function("tq_step", 0, flags, |_| {
        if crate::code_rule_dsl::matcher::spend_util_step() {
            Ok(true)
        } else {
            Err(user_error(
                crate::code_rule_dsl::matcher::UtilLimit::Work.message(),
            ))
        }
    })
}

pub(super) fn register(connection: &Connection, registry: &Arc<MatchRegistry>) -> SqlResult<()> {
    let flags = FunctionFlags::SQLITE_UTF8;
    connection.create_scalar_function("tq_enter_file", 1, flags, |context| {
        let file = context.get::<i64>(0)?;
        FILE_HOOK.with(|hook| match hook.get() {
            Some(hook) => {
                // SAFETY: the scoped installer keeps the callback alive during this statement.
                unsafe { (hook.call)(hook.data, file) }
            }
            None => Err(user_error("all-file statement has no file context")),
        })
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_pattern", 5, flags, move |context| {
        pattern(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_kind", 5, flags, move |context| {
        node_test(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_regex", 5, flags, move |context| {
        node_test(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_struct", 5, flags, move |context| {
        structure(context, &owner)
    })?;
    // tq_util(batch, term, file, node, env): a recursive utility, by its own statement.
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_util", 5, flags, move |context| {
        let env = context.get::<Option<i64>>(4)?;
        let Some(env) = env else {
            return Ok(None);
        };
        if env < 0 {
            return Ok(Some(env));
        }
        let view = batch(context, &owner)?;
        let term_index = context.get::<i64>(1)?;
        let state = state(&view)?;
        let Term::Util {
            sql,
            memoize,
            step,
            env_free,
        } = term(state, term_index)?
        else {
            return Err(user_error("tq_util term is not a utility"));
        };
        let file = context.get::<Option<i64>>(2)?;
        let node = context.get::<Option<i64>>(3)?;
        // An environment-free utility's verdict at a node is every caller's: one memo entry, and a
        // match leaves each caller's environment as it was.
        let shared = *env_free && crate::code_rule_dsl::budget::shortcuts_on();
        let memo_env = if shared { 0 } else { env };
        let key = file
            .zip(node)
            .map(|(file, node)| (term_index, file, node, memo_env, super::recursion::depth()));
        if *memoize
            && let Some((verdict, cost)) = key.and_then(|key| {
                state
                    .util_memo
                    .lock()
                    .expect("utility memo mutex poisoned")
                    .get(&key)
                    .copied()
            })
        {
            for _ in 0..cost {
                if !crate::code_rule_dsl::matcher::spend_util_step() {
                    return Err(user_error(
                        crate::code_rule_dsl::matcher::UtilLimit::Work.message(),
                    ));
                }
            }
            return Ok(match verdict {
                Some(matched) if shared && matched >= 0 => Some(env),
                verdict => verdict,
            });
        }
        let before = crate::code_rule_dsl::matcher::utility_steps_left();
        // A utility enters one level of `matches` (within the depth and stack limits) and spends a
        // step, as the matcher's does; a term hoisted into its own statement does neither.
        let level = if *step {
            let level = crate::code_rule_dsl::matcher::enter_util_level()
                .map_err(|limit| user_error(limit.message()))?;
            if !crate::code_rule_dsl::matcher::spend_util_step() {
                crate::code_rule_dsl::matcher::leave_util_level(level);
                return Err(user_error(
                    crate::code_rule_dsl::matcher::UtilLimit::Work.message(),
                ));
            }
            Some(level)
        } else {
            None
        };
        // SAFETY: the handle is used only for this call, on the connection running it.
        let db = unsafe { context.get_connection()?.handle() };
        let result = super::recursion::run(
            db,
            sql,
            &[Some(context.get::<i64>(0)?), file, node, Some(env)],
        );
        if let Some(level) = level {
            crate::code_rule_dsl::matcher::leave_util_level(level);
        }
        let result = result.map_err(user_error)?;
        if *memoize && let Some(key) = key {
            let cost = before.saturating_sub(crate::code_rule_dsl::matcher::utility_steps_left());
            state
                .util_memo
                .lock()
                .expect("utility memo mutex poisoned")
                .insert(key, (result, cost));
        }
        Ok(result)
    })?;
    // tq_not(inner, env): env where inner did not match, unknown where inner was unknown.
    connection.create_scalar_function(
        "tq_not",
        2,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            let inner = context.get::<Option<i64>>(0)?;
            let env = context.get::<Option<i64>>(1)?;
            Ok(match (env, inner) {
                (None, _) => None,
                (Some(env), _) if env < 0 => Some(env),
                (Some(env), None) => Some(env),
                (Some(_), Some(inner)) if inner < 0 => Some(UNKNOWN),
                (Some(_), Some(_)) => None,
            })
        },
    )?;
    // tq_capture(batch, env, name): the node a single-node capture holds.
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_capture", 3, flags, move |context| {
        let Some(env) = context.get::<Option<i64>>(1)? else {
            return Ok(None);
        };
        if env < 0 {
            return Ok(None);
        }
        let view = batch(context, &owner)?;
        let name = context.get::<String>(2)?;
        Ok(state(&view)?
            .envs
            .get(env)
            .and_then(|env| env.single(&name))
            .map(i64::from))
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_pred", 6, flags, move |context| {
        predicate(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_nth_counted", 5, flags, move |context| {
        nth_counted(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_same_key", 6, flags, move |context| {
        same_key(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection
        .create_scalar_function("tq_fresh", 3, flags, move |context| fresh(context, &owner))?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_follow", 4, flags, move |context| {
        follow(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_unfollow", 3, flags, move |context| {
        unfollow(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_following", 2, flags, move |context| {
        following(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_call", 5, flags, move |context| call(context, &owner))?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_export", 6, flags, move |context| {
        export(context, &owner)
    })?;
    let owner = Arc::clone(registry);
    connection.create_scalar_function("tq_host_env", 5, flags, move |context| {
        host_env(context, &owner)
    })?;
    register_all(connection, registry)?;
    register_any_and_steps(connection)?;
    connection.create_aggregate_function(
        "tq_nth",
        5,
        FunctionFlags::SQLITE_UTF8,
        Nth {
            registry: Arc::clone(registry),
        },
    )?;
    connection.create_aggregate_function(
        "tq_search",
        6,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        Search,
    )?;
    connection.create_aggregate_function("tq_count", 6, FunctionFlags::SQLITE_UTF8, Count)
}
