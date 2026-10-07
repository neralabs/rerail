use super::AstMatcher;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// `matches` recursion deeper than this fails the check for the file instead of risking the stack.
/// Utilities recurse only through relations, so real rules recurse as deep as the syntax they walk.
pub(super) const MAX_UTIL_DEPTH: u32 = 256;

/// Stack a root's `matches` recursion may use, measured from `begin_check`. A level's frames grow
/// with how much rule it nests and with the build (a small recursive rule takes about 2 KiB per
/// level in release, 15 KiB in debug), so bytes back up the level count. Every thread matching runs
/// on has room for this on top of its callers: Node's main thread and libuv workers (8 MiB) and
/// Rust test threads (2 MiB).
pub(super) const MAX_UTIL_STACK_BYTES: usize = 1 << 20;

thread_local! {
    pub(super) static UTIL_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// The stack address `begin_check` started at; 0 outside one (no stack limit applies).
    pub(super) static UTIL_STACK_BASE: Cell<usize> = const { Cell::new(0) };
    pub(super) static UTIL_STEPS_LEFT: Cell<usize> = const { Cell::new(usize::MAX) };
    pub(super) static UTIL_EXCEEDED: Cell<Option<UtilLimit>> = const { Cell::new(None) };
    /// The matcher's Unknown flag: set when a rule failed because a verdict it read was unknown
    /// rather than false (a value it could not decide). A matcher that returns a match leaves it
    /// as it found it; one that fails leaves it set if it was set on entry or its own failure is
    /// unknown. Readers (`not`, relations, `count`) clear it before the sub-rule they read and
    /// restore it after.
    pub(super) static UNKNOWN: Cell<bool> = const { Cell::new(false) };
    /// Whether the root being matched can reach an unknown verdict (it reads values). Without one,
    /// no matcher touches `UNKNOWN`, so rules that read no values match exactly as they did.
    pub(super) static THREE_VALUED: Cell<bool> = const { Cell::new(false) };
}

/// An address in the current stack frame.
#[inline(always)]
pub(super) fn stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

/// A limit `matches` recursion hit. The root's verdict is then unknown (a `not` above the cut-off
/// reference would read its failure as a non-match), so the check fails for the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum UtilLimit {
    /// `MAX_UTIL_DEPTH` or `MAX_UTIL_STACK_BYTES`.
    Depth,
    /// More utility evaluations and counted nodes than the query's `maxVisitedNodes` in one file.
    Work,
    /// The file's scope table (`resolves`) could not be built within `maxVisitedNodes`, the time
    /// budget, or its nesting limit.
    Scope,
    /// Decoding values (`value`, `compare` over values) spent the rest of `maxVisitedNodes`.
    Value,
}

impl UtilLimit {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Depth => {
                "`matches` recursion exceeded its depth limit (256 nested utility rules or 1 MiB of stack)"
            }
            Self::Work => {
                "`matches` and `count` exceeded their work budget (maxVisitedNodes utility evaluations and counted nodes per file)"
            }
            Self::Scope => {
                "`resolves` could not build the file's scope table within its budget (maxVisitedNodes nodes, the time budget, or 2,000 nesting levels)"
            }
            Self::Value => {
                "decoding values exceeded the work budget (maxVisitedNodes decoded nodes, with utility evaluations and counted nodes, per file)"
            }
        }
    }
}

/// Distinct parameterized calls (a utility and a mapping of its parameters) one query may make per
/// grammar. Each is compiled once, as a frame plan over the utility's one compiled rule.
pub(crate) const MAX_UTIL_INSTANCES: usize = 1024;

/// A parameterized call's mapping: each parameter and the caller's capture it names.
pub(crate) type CallArgs = Vec<(String, String)>;

/// A utility's memo scope: capture names, sorted (`UtilRef::scope`).
pub(super) type Scope = Arc<[Arc<str>]>;

/// What a utility compiles from: a digest of its language, its id and every utility it reaches
/// (id, parameters, rule). Equal utilities of any query or document compile alike wherever nothing
/// of the query shapes them (`context_free`): their verdicts are one (`UtilRef::memo_term`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct UtilOrigin {
    pub(crate) digest: [u8; 32],
}

/// The `utils` of one ast query compiled for one grammar, indexed by `UtilRef::index`. Filled once,
/// after every utility compiled, so utilities can refer to each other and to themselves.
pub(crate) struct UtilTable {
    #[cfg_attr(not(feature = "policy-profile"), allow(dead_code))]
    pub(crate) names: Vec<String>,
    /// Whether each utility has parameters: it runs in a frame of its own, never in its caller's
    /// environment, so its captures are not the query's.
    pub(crate) framed: Vec<bool>,
    /// What each utility compiles from (`UtilOrigin`); `None` in a define's validation table.
    pub(crate) origins: Vec<Option<UtilOrigin>>,
    /// Each utility's memo scope (`UtilRef::scope`), worked out on first use.
    scopes: Vec<OnceLock<Option<Scope>>>,
    /// Each utility's memo identity (`UtilRef::memo_term`), worked out on first use.
    memo_terms: Vec<OnceLock<(usize, usize)>>,
    pub(crate) rules: OnceLock<Vec<AstMatcher>>,
    /// A first compilation of the utilities, which compile-time walks read while `rules` is being
    /// compiled: what a utility binds or exports does not depend on the order of its terms, which is
    /// all that differs between the two.
    pub(crate) draft: OnceLock<Vec<AstMatcher>>,
    /// Whether a compile-time walk reached a utility before either was filled.
    pub(crate) unfilled: AtomicBool,
    /// The distinct parameterized calls compiled so far.
    instances: Mutex<BTreeSet<(usize, CallArgs)>>,
}

impl UtilTable {
    pub(crate) fn new(
        names: Vec<String>,
        framed: Vec<bool>,
        origins: Vec<Option<UtilOrigin>>,
    ) -> Self {
        Self {
            scopes: names.iter().map(|_| OnceLock::new()).collect(),
            memo_terms: names.iter().map(|_| OnceLock::new()).collect(),
            names,
            framed,
            origins,
            rules: OnceLock::new(),
            draft: OnceLock::new(),
            unfilled: AtomicBool::new(false),
            instances: Mutex::new(BTreeSet::new()),
        }
    }

    /// Whether a walk met a utility it could not read yet.
    pub(crate) fn was_unfilled(&self) -> bool {
        self.unfilled.load(Ordering::Relaxed)
    }

    /// The compiled utilities that run in their caller's environment (no parameters).
    pub(crate) fn shared_rules(&self) -> impl Iterator<Item = &AstMatcher> {
        self.rules
            .get()
            .into_iter()
            .flatten()
            .zip(&self.framed)
            .filter_map(|(rule, framed)| (!framed).then_some(rule))
    }

    /// Records a call; false once more than `MAX_UTIL_INSTANCES` distinct ones were recorded.
    pub(crate) fn record_instance(&self, index: usize, args: &[(String, String)]) -> bool {
        let mut instances = self
            .instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        instances.insert((index, args.to_vec()));
        instances.len() <= MAX_UTIL_INSTANCES
    }
}

/// A parameterized utility call, `{ "matches": { "id": { "P": "$C" } } }`: the utility runs in a
/// frame of its own that holds only its parameters, each bound to what the caller's capture holds
/// (if anything). What the utility binds to a parameter the caller's capture was not bound to comes
/// back under the caller's name; the utility's other captures stay in the frame. So a utility may
/// recurse over captures, a pair of nodes at a time, to any depth: each level's captures are its
/// own.
#[derive(Clone)]
pub(crate) struct UtilCall {
    pub(crate) util: UtilRef,
    /// Each parameter and the caller's capture it names, in the utility's parameter order.
    pub(crate) args: CallArgs,
}

impl UtilCall {
    /// The caller's names for the parameters among `names` (the utility's own): what the call binds
    /// or reads, seen from its caller.
    pub(crate) fn outward<S: AsRef<str>>(
        &self,
        names: impl IntoIterator<Item = S>,
    ) -> BTreeSet<&str> {
        let names = names
            .into_iter()
            .map(|name| name.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        self.args
            .iter()
            .filter(|(param, _)| names.contains(param))
            .map(|(_, caller)| caller.as_str())
            .collect()
    }

    /// Every capture of the caller the call names.
    pub(crate) fn callers(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|(_, caller)| caller.as_str())
    }
}

/// Per-walk summaries of the utility graph, one per key: a key names a utility, together with the
/// context its summary depends on. A walk summarizes a key's utility from the summaries of the
/// keys it refers to, monotonically (unions, a parameterized call's mapping, conjunctions), so
/// utilities that reach each other (a strongly connected component of the graph) are summarized
/// together, as the fixpoint reached from `initial`: a recursive reference reads the value its key
/// had in the previous walk of the component (`initial` in the first), and the component is walked
/// again until no value it read changes. Tarjan's low links tell where a component ends; until
/// then its keys' values hold for this walk only, and nothing outside the component reuses them.
/// A key on no cycle is summarized once.
pub(crate) struct UtilitySummary<K: Ord, T> {
    completed: BTreeMap<K, T>,
    /// The keys being summarized, outermost first, with where each sits.
    stack: Vec<Frame>,
    active: BTreeMap<K, usize>,
    /// The values of a component's keys summarized in the current walk of it, with their frames'
    /// discovery indices, and where each sits.
    pending: Vec<(K, T, usize)>,
    pending_at: BTreeMap<K, usize>,
    /// What recursive references to a component's keys read.
    approximations: BTreeMap<K, T>,
    discovered: usize,
}

struct Frame {
    index: usize,
    /// The lowest discovery index of a key still being summarized that this one reached.
    low: usize,
    /// `pending`'s length when the frame started: the keys after it are its component's.
    pending: usize,
}

/// A key whose walk `UtilitySummary::start` began: its discovery index and stack position.
#[derive(Clone, Copy)]
struct Started {
    index: usize,
    position: usize,
}

impl<K: Ord + Clone, T: Clone + PartialEq> UtilitySummary<K, T> {
    pub(crate) fn new() -> Self {
        Self {
            completed: BTreeMap::new(),
            stack: Vec::new(),
            active: BTreeMap::new(),
            pending: Vec::new(),
            pending_at: BTreeMap::new(),
            approximations: BTreeMap::new(),
            discovered: 0,
        }
    }

    /// The summary of `key`, which `walk` computes from the summaries it asks of `self`.
    pub(crate) fn summarize(
        &mut self,
        key: K,
        initial: T,
        mut walk: impl FnMut(&mut Self) -> T,
    ) -> T {
        let started = match self.start(&key, initial) {
            Ok(value) => return value,
            Err(started) => started,
        };
        loop {
            let value = walk(self);
            if let Some(value) = self.settle(&key, started, value) {
                return value;
            }
        }
    }

    /// The value of `key` if a walk need not compute it: complete, being summarized further up
    /// (what its previous walk reached), or summarized in this walk of its component. Otherwise
    /// starts its walk.
    fn start(&mut self, key: &K, initial: T) -> Result<T, Started> {
        if let Some(value) = self.completed.get(key) {
            return Ok(value.clone());
        }
        if let Some(&position) = self.active.get(key) {
            self.reach(self.stack[position].index);
            return Ok(self
                .approximations
                .entry(key.clone())
                .or_insert(initial)
                .clone());
        }
        if let Some(&at) = self.pending_at.get(key) {
            let (_, value, index) = &self.pending[at];
            let (value, index) = (value.clone(), *index);
            self.reach(index);
            return Ok(value);
        }
        self.discovered += 1;
        let started = Started {
            index: self.discovered,
            position: self.stack.len(),
        };
        self.active.insert(key.clone(), started.position);
        self.stack.push(Frame {
            index: started.index,
            low: started.index,
            pending: self.pending.len(),
        });
        Err(started)
    }

    /// Records what a walk of `key` computed: its value, once `key` is done with (complete, or
    /// left to its component's root), or `None` when its component must be walked again.
    fn settle(&mut self, key: &K, started: Started, value: T) -> Option<T> {
        let Frame { low, pending, .. } = self.stack[started.position];
        if low < started.index {
            // A key of an enclosing frame's component.
            self.stack.pop();
            self.active.remove(key);
            self.reach(low);
            self.pending_at.insert(key.clone(), self.pending.len());
            self.pending
                .push((key.clone(), value.clone(), started.index));
            return Some(value);
        }
        let members = self.pending.split_off(pending);
        for (member, _, _) in &members {
            self.pending_at.remove(member);
        }
        let stable = members
            .iter()
            .map(|(member, value, _)| (member, value))
            .chain([(key, &value)])
            .all(|(member, value)| {
                self.approximations
                    .get(member)
                    .is_none_or(|read| read == value)
            });
        if stable {
            for (member, value, _) in members {
                self.approximations.remove(&member);
                self.completed.insert(member, value);
            }
            self.approximations.remove(key);
            self.completed.insert(key.clone(), value.clone());
            self.stack.pop();
            self.active.remove(key);
            return Some(value);
        }
        for (member, value, _) in members {
            self.approximations.insert(member, value);
        }
        self.approximations.insert(key.clone(), value);
        self.stack[started.position].low = started.index;
        None
    }

    /// Records that the key being summarized reached a key of discovery index `index` that is not
    /// complete yet.
    fn reach(&mut self, index: usize) {
        let frame = self
            .stack
            .last_mut()
            .expect("only a walk reaches an incomplete key");
        frame.low = frame.low.min(index);
    }
}

impl<K: Ord + Clone, T: Clone + PartialEq> Default for UtilitySummary<K, T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub(crate) struct UtilRef {
    pub(crate) table: Weak<UtilTable>,
    pub(crate) index: usize,
}

impl UtilRef {
    /// What the memo keys the referenced utility's verdicts by (`memo`): where it compiles alike
    /// in every query (it has a `scope`), what it compiles from (`UtilOrigin::digest`), so every
    /// check of a file reaching an equal utility shares them; else its table and index. `None`
    /// where its table is gone.
    pub(super) fn memo_term(&self) -> Option<(usize, usize)> {
        let table = self.table.upgrade()?;
        let term = table.memo_terms.get(self.index)?.get_or_init(|| {
            let shared = self.scope().flatten().is_some();
            match table.origins.get(self.index).cloned().flatten() {
                Some(origin) if shared => {
                    let word = |bytes: &[u8]| {
                        usize::from_le_bytes(bytes.try_into().expect("eight digest bytes"))
                    };
                    // Table addresses never have the top bit set: the two kinds never collide.
                    (
                        word(&origin.digest[..8]) | 1 << (usize::BITS - 1),
                        word(&origin.digest[8..16]),
                    )
                }
                _ => (self.table.as_ptr() as usize, self.index),
            }
        });
        Some(*term)
    }

    /// The captures the referenced utility may read or bind, sorted: all its verdict reads of its
    /// caller's bindings. `Some(None)` where it holds a term that reads more (`context_free`),
    /// `None` where its table is gone.
    pub(super) fn scope(&self) -> Option<Option<Scope>> {
        let table = self.table.upgrade()?;
        let scope = table.scopes.get(self.index)?.get_or_init(|| {
            self.with_target(|rule| {
                context_free(rule, &mut BTreeMap::new()).then(|| {
                    super::CaptureAnalysis::default()
                        .mentions(rule)
                        .into_iter()
                        .map(Arc::from)
                        .collect()
                })
            })
            .flatten()
        });
        Some(scope.clone())
    }

    /// Whether the referenced utility and the utilities it reaches hold no term a query's own
    /// context shapes (`context_free`), with the utilities read so far: `None` while one is
    /// being read, which a reference back counts as context-free (its cycle is as free as its
    /// other utilities).
    fn context_free_within(&self, read: &mut BTreeMap<(usize, usize), Option<bool>>) -> bool {
        let key = (self.table.as_ptr() as usize, self.index);
        if let Some(known) = read.get(&key) {
            return known.unwrap_or(true);
        }
        read.insert(key, None);
        let free = self
            .with_target(|rule| context_free(rule, read))
            .unwrap_or(false);
        read.insert(key, Some(free));
        free
    }
}

/// Whether `matcher` holds only terms that read nothing of the environment but the captures they
/// name (`UtilRef::scope`, for a utility): no Rerail predicate (a `parse`'s `host` rules read
/// the host's captures, `binds` its own) and no comparison of values or of a transform's text.
/// `read`: the utilities read so far (`UtilRef::context_free_within`).
fn context_free(matcher: &AstMatcher, read: &mut BTreeMap<(usize, usize), Option<bool>>) -> bool {
    let stop = |by: &super::StopBy, read: &mut BTreeMap<(usize, usize), Option<bool>>| match by {
        super::StopBy::Rule(rule) => context_free(rule, read),
        super::StopBy::Neighbor | super::StopBy::End => true,
    };
    match matcher {
        AstMatcher::Pattern(_) | AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
            true
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            items.iter().all(|item| context_free(item, read))
        }
        AstMatcher::Not(item) => context_free(item, read),
        AstMatcher::Inside(item, by)
        | AstMatcher::Has(item, by)
        | AstMatcher::Precedes(item, by)
        | AstMatcher::Follows(item, by) => context_free(item, read) && stop(by, read),
        AstMatcher::Field(relation) => {
            context_free(&relation.rule, read) && stop(&relation.stop, read)
        }
        AstMatcher::NthChild(nth) => nth
            .of_rule
            .as_ref()
            .is_none_or(|rule| context_free(rule, read)),
        AstMatcher::Constrained(core) => {
            context_free(&core.rule, read)
                && core
                    .constraints
                    .iter()
                    .all(|(_, constraint)| context_free(constraint, read))
        }
        AstMatcher::Count(count) => context_free(&count.relation, read),
        AstMatcher::Compare(comparison) => {
            comparison.reading.is_none() && comparison.transform.is_none()
        }
        AstMatcher::Util(util) => util.context_free_within(read),
        AstMatcher::Call(call) => call.util.context_free_within(read),
        AstMatcher::Semantic(_) => false,
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => false,
    }
}

impl UtilRef {
    /// The referenced rule, or its draft while the table is being compiled; `None` before the draft
    /// is filled (which the table records) or after the table is dropped.
    pub(crate) fn with_target<T>(&self, visit: impl FnOnce(&AstMatcher) -> T) -> Option<T> {
        let table = self.table.upgrade()?;
        let Some(rules) = table.rules.get().or_else(|| table.draft.get()) else {
            table.unfilled.store(true, Ordering::Relaxed);
            return None;
        };
        Some(visit(rules.get(self.index)?))
    }

    /// What `with_target` reads, as a summary key: the table, the utility, and which of the table's
    /// compilations is filled (none, the draft, the rules). A summary under this key holds in a
    /// walk that spans the table's compilation steps.
    pub(crate) fn summary_key(&self) -> (usize, usize, u8) {
        let filled = self.table.upgrade().map_or(0, |table| {
            if table.rules.get().is_some() {
                2
            } else {
                u8::from(table.draft.get().is_some())
            }
        });
        (self.table.as_ptr() as usize, self.index, filled)
    }

    #[cfg_attr(not(feature = "policy-profile"), allow(dead_code))]
    pub(crate) fn name(&self) -> String {
        self.table
            .upgrade()
            .and_then(|table| table.names.get(self.index).cloned())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn utils_recurse_through_relations_to_any_depth() {
        let rule = compiled(json!({
            "rule": { "kind": "return_statement", "has": { "matches": "tail-await" } },
            "utils": {
                "tail-await": { "any": [
                    { "kind": "await_expression" },
                    { "kind": "parenthesized_expression", "has": { "matches": "tail-await" } },
                    { "kind": "sequence_expression", "has": { "matches": "tail-await" } },
                ] },
            },
        }));
        let source = "async function f() { return ((((await x)))); }\n\
                      async function g() { return (x); }\n\
                      async function h() { return await y; }\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["return ((((await x))));", "return await y;"],
            "{result}"
        );
    }

    #[test]
    fn utils_share_the_callers_metavariable_bindings() {
        // `$F` is bound by the rule before the utility runs, so the utility's `$F` must agree.
        let rule = compiled(json!({
            "rule": { "pattern": "$F($A)", "matches": "calls-itself" },
            "utils": { "calls-itself": { "pattern": "$F($F)" } },
        }));
        let source = "g(g);\ng(h);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["g(g) A=g F=g"], "{result}");

        // A utility's own captures are visible to the rule's other terms and in the witness.
        let rule = compiled(json!({
            "rule": { "all": [{ "matches": "call" }, { "has": { "stopBy": "end", "pattern": "$ARG" , "kind": "number" } }] },
            "utils": { "call": { "pattern": "$CALLEE($$$)" } },
        }));
        let source = "f(1);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["f(1) ARG=1 CALLEE=f"],
            "{result}"
        );
    }

    #[test]
    fn utils_may_use_other_utils_and_constraints_may_use_utils() {
        let rule = compiled(json!({
            "rule": { "pattern": "$FN($$$ARGS)", "matches": "hook-call" },
            "utils": {
                "hook-call": { "has": { "kind": "identifier", "matches": "hook-name" } },
                "hook-name": { "regex": "^use[A-Z]" },
            },
            "constraints": { "FN": { "matches": "hook-name" } },
        }));
        let source = "useState(1);\nfoo(2);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["useState(1) ARGS=1 FN=useState"],
            "{result}"
        );
    }

    #[test]
    fn constraints_filter_single_captures_after_the_rule_matches() {
        let rule = compiled(json!({
            "rule": { "pattern": "$FN($$$A)" },
            "constraints": { "FN": { "regex": "^use[A-Z]" } },
        }));
        let source = "useState(1);\nuser(2);\nuseEffect(f);\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            [
                "useState(1) A=1 FN=useState",
                "useEffect(f) A=f FN=useEffect"
            ],
            "{result}"
        );

        // A capture made by a relation is constrained like any other.
        let rule = compiled(json!({
            "rule": {
                "pattern": "$HOOK($$$)",
                "inside": { "stopBy": "end", "kind": "if_statement", "has": { "pattern": "($COND)" } },
            },
            "constraints": { "HOOK": { "regex": "^use[A-Z]" }, "COND": { "kind": "identifier" } },
        }));
        let source = "if (ready) { useState(1); }\nif (a.b) { useMemo(g); }\nif (ok) { log(); }\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["useState(1) COND=ready HOOK=useState"],
            "{result}"
        );

        // `not` in a constraint; a metavariable left unbound (an `any` branch not taken) is not checked.
        let rule = compiled(json!({
            "rule": { "any": [{ "pattern": "$X + $Y" }, { "pattern": "-$X" }] },
            "constraints": { "Y": { "not": { "kind": "number" } } },
        }));
        let source = "a + 1;\na + b;\n-c;\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(
            matched_texts(&result, source),
            ["a + b X=a Y=b", "-c X=c"],
            "{result}"
        );
    }

    #[test]
    fn util_recursion_deeper_than_the_limit_fails_the_check() {
        let rule = compiled(json!({
            "rule": { "kind": "expression_statement", "has": { "stopBy": "end", "matches": "nested" } },
            // Every array whose innermost array is empty: the recursion runs to the bottom.
            "utils": { "nested": { "kind": "array", "any": [
                { "not": { "has": { "kind": "array" } } },
                { "has": { "matches": "nested" } },
            ] } },
        }));
        let depth = MAX_UTIL_DEPTH as usize;
        let source = |levels: usize| format!("x = {}{};\n", "[".repeat(levels), "]".repeat(levels));
        // Within both limits in any build: debug frames take about 15 KiB per level of this rule.
        let shallow = source(50);
        let result = evaluate_file(&rule, "typescript", "a.ts", &shallow);
        assert_eq!(result["failures"], json!([]), "{result}");
        assert_eq!(
            result["violations"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
        let deep = source(depth + 50);
        let result = evaluate_file(&rule, "typescript", "a.ts", &deep);
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(
            result["failures"],
            json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Depth.message(), "path": "a.ts" }]),
            "{result}"
        );
        assert!(
            UtilLimit::Depth
                .message()
                .contains(&MAX_UTIL_DEPTH.to_string())
        );
        assert!(
            UtilLimit::Depth
                .message()
                .contains(&(MAX_UTIL_STACK_BYTES >> 20).to_string())
        );
    }

    #[test]
    fn util_evaluations_are_metered_by_the_visited_node_budget() {
        // Each level re-walks every descendant before its cheap test fails: exponential in depth.
        // The cheap test reads text a walk cannot select nodes by (a `kind` or a literal would let
        // the walk pass over every node before recursing).
        let document = check_document(json!({
            "id": "ast-match",
            "violation": finding(),
            "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "program", "matches": "slow" },
                "utils": { "slow": { "has": { "stopBy": "end", "all": [{ "matches": "slow" }, { "not": { "regex": "." } }] } } },
            } },
        }));
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxVisitedNodes"] = json!(10_000)
        });
        let rule = compile_document_with(document, &capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = format!("{}x;{}\n", "{".repeat(30), "}".repeat(30));
        let result = evaluate_file(&rule, "typescript", "a.ts", &source);
        assert_eq!(
            result["failures"],
            json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Work.message(), "path": "a.ts" }]),
            "{result}"
        );
        // The same rule on a shallow file finishes inside the budget.
        let result = evaluate_file(&rule, "typescript", "a.ts", "{ x; }\n");
        assert_findings(&result, 0);
    }
}
