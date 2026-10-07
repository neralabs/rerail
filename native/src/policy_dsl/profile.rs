//! Benchmark-only attribution for the existing workspace evaluator.
//! Disabled in production builds. Reports worker CPU separately from wall time.

mod semantic;
use ast_grep_core::matcher::{DumpPattern, Matcher};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use super::matcher::{AstMatcher, StopBy};
use crate::policy_dsl::language::GrammarDialect;

fn dump_json(pattern: DumpPattern<'_>) -> Value {
    json!({
        "metavariable": pattern.is_meta_var,
        "kind": pattern.kind,
        "text": pattern.text,
        "children": pattern.children.into_iter().map(dump_json).collect::<Vec<_>>(),
    })
}

fn describe_matcher(matcher: &AstMatcher, language: GrammarDialect) -> Value {
    let kind_name = |id| language.kind_name(id).map(Cow::Borrowed);
    match matcher {
        AstMatcher::Pattern(pattern) => {
            json!({ "pattern": pattern.dump(&kind_name).map(dump_json) })
        }
        AstMatcher::Kind(kind) => json!({ "kind": kind.potential_kinds().map(|kinds|
            kinds.iter().map(|id| kind_name(id as u16)).collect::<Vec<_>>()) }),
        AstMatcher::Regex(_) => json!({ "regex": true }),
        AstMatcher::Never => json!({ "never": true }),
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => json!({ "testUnknown": true }),
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            let operator = if matches!(matcher, AstMatcher::All(_)) {
                "all"
            } else {
                "any"
            };
            json!({ operator: items.iter().map(|item| describe_matcher(item, language)).collect::<Vec<_>>() })
        }
        AstMatcher::Not(item) => json!({ "not": describe_matcher(item, language) }),
        AstMatcher::Compare(comparison) => {
            json!({ "compare": [&comparison.left, &comparison.right] })
        }
        AstMatcher::Count(count) => json!({ "count": {
            "relation": describe_matcher(&count.relation, language),
            "min": count.min,
            "max": count.max,
        } }),
        AstMatcher::Util(util) => json!({ "matches": util.name() }),
        AstMatcher::Call(call) => {
            json!({ "matches": { call.util.name(): call.args.iter().map(|(param, caller)| (param.clone(), json!(format!("${caller}")))).collect::<serde_json::Map<_, _>>() } })
        }
        AstMatcher::Semantic(semantic) => semantic::describe_matcher(semantic, language),
        AstMatcher::Constrained(core) => json!({
            "rule": describe_matcher(&core.rule, language),
            "constraints": core.constraints.iter()
                .map(|(name, rule)| (name.clone(), describe_matcher(rule, language)))
                .collect::<serde_json::Map<_, _>>(),
        }),
        AstMatcher::Inside(item, stop)
        | AstMatcher::Has(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            let operator = match matcher {
                AstMatcher::Inside(..) => "inside",
                AstMatcher::Has(..) => "has",
                AstMatcher::Precedes(..) => "precedes",
                _ => "follows",
            };
            let stop = match stop {
                StopBy::Neighbor => json!("neighbor"),
                StopBy::End => json!("end"),
                StopBy::Rule(rule) => describe_matcher(rule, language),
            };
            json!({ operator: describe_matcher(item, language), "stopBy": stop })
        }
        AstMatcher::NthChild(nth) => json!({ "nthChild": {
            "step": nth.position.step,
            "offset": nth.position.offset,
            "reverse": nth.reverse,
            "ofRule": nth.of_rule.as_ref().map(|rule| describe_matcher(rule, language)),
        } }),
        AstMatcher::Field(relation) => {
            let operator = if relation.inside { "inside" } else { "has" };
            let stop = match &relation.stop {
                StopBy::Neighbor => json!("neighbor"),
                StopBy::End => json!("end"),
                StopBy::Rule(rule) => describe_matcher(rule, language),
            };
            json!({
                operator: describe_matcher(&relation.rule, language),
                "stopBy": stop,
                "field": relation.field,
            })
        }
    }
}

pub(super) fn record_query(
    policy_id: &str,
    check_id: &str,
    language: GrammarDialect,
    matcher: &AstMatcher,
    root_kinds: Option<BTreeSet<u16>>,
) {
    eprintln!(
        "RERAIL_POLICY_ANCHOR {}",
        json!({
            "ruleId": policy_id,
            "clauseId": check_id,
            "language": crate::policy_dsl::language::label(language),
            "rootKinds": root_kinds.map(|kinds| kinds.into_iter()
                .map(|id| language.kind_name(id)).collect::<Vec<_>>()),
            "matcher": describe_matcher(matcher, language),
        })
    );
}

#[derive(Default)]
struct Profile {
    revision: u32,
    regex_ns: u128,
    parse_ns: u128,
    tape_encode_ns: u128,
    tape_decode_ns: u128,
    tape_serialize_ns: u128,
    store_read_ns: u128,
    store_write_ns: u128,
    manifest_ns: u128,
    index_ns: u128,
    selection_ns: u128,
    dispatch_ns: u128,
    serialization_ns: u128,
    other_ns: u128,
    visited_nodes: usize,
    indexed_files: usize,
    incomplete_indexes: usize,
    indexed_nodes: usize,
    peak_index_bytes: usize,
    active_line_bytes: usize,
    active_navigation_line_bytes: usize,
    peak_transient_bytes: Option<usize>,
    retained_source_bytes: Option<usize>,
    retained_tape_bytes: Option<usize>,
    retained_grammar_bytes: Option<usize>,
    retained_dispatch_bytes: Option<usize>,
    retained_result_bytes: Option<usize>,
    native_acquisitions: usize,
    native_active: usize,
    native_peak: usize,
    native_doc_overlap: usize,
    native_hold_ns: u128,
    doc_acquisitions: usize,
    doc_active: usize,
    doc_peak: usize,
    doc_hold_ns: u128,
    rule_files: usize,
    rejected_rule_files: usize,
    selected_root_pairs: usize,
    fallback_rule_files: usize,
}

type ProfileSession = Arc<Mutex<Profile>>;

thread_local! {
    static PROFILE: RefCell<Option<ProfileSession>> = const { RefCell::new(None) };
    static PHASE_STACK: RefCell<Vec<(Phase, u128)>> = const { RefCell::new(Vec::new()) };
}

static ACTIVE_SESSION: LazyLock<Mutex<Option<ProfileSession>>> = LazyLock::new(|| Mutex::new(None));

fn with_profile(f: impl FnOnce(&mut Profile)) {
    PROFILE.with(|profile| {
        if let Some(session) = profile.borrow().as_ref() {
            f(&mut session.lock().expect("profile mutex poisoned"));
        }
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    Regex,
    Parse,
    TapeEncode,
    TapeDecode,
    TapeSerialize,
    StoreRead,
    StoreWrite,
    Manifest,
    Index,
    Selection,
    Dispatch,
    Serialization,
    Other,
}

fn phase_total(profile: &mut Profile, phase: Phase) -> &mut u128 {
    match phase {
        Phase::Regex => &mut profile.regex_ns,
        Phase::Parse => &mut profile.parse_ns,
        Phase::TapeEncode => &mut profile.tape_encode_ns,
        Phase::TapeDecode => &mut profile.tape_decode_ns,
        Phase::TapeSerialize => &mut profile.tape_serialize_ns,
        Phase::StoreRead => &mut profile.store_read_ns,
        Phase::StoreWrite => &mut profile.store_write_ns,
        Phase::Manifest => &mut profile.manifest_ns,
        Phase::Index => &mut profile.index_ns,
        Phase::Selection => &mut profile.selection_ns,
        Phase::Dispatch => &mut profile.dispatch_ns,
        Phase::Serialization => &mut profile.serialization_ns,
        Phase::Other => &mut profile.other_ns,
    }
}

fn pause_active(now: u128) {
    PHASE_STACK.with(|stack| {
        if let Some((phase, started)) = stack.borrow_mut().last_mut() {
            let elapsed = now.saturating_sub(*started);
            with_profile(|profile| *phase_total(profile, *phase) += elapsed);
        }
    });
}

pub(super) struct PhaseTimer {
    phase: Phase,
}

impl PhaseTimer {
    pub(super) fn start(phase: Phase) -> Self {
        let now = thread_cpu_ns().unwrap_or_default();
        pause_active(now);
        PHASE_STACK.with(|stack| stack.borrow_mut().push((phase, now)));
        Self { phase }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        let now = thread_cpu_ns().unwrap_or_default();
        PHASE_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            let Some((phase, started)) = stack.pop() else {
                return;
            };
            debug_assert_eq!(phase, self.phase);
            with_profile(|profile| *phase_total(profile, phase) += now.saturating_sub(started));
            if let Some((_, parent_started)) = stack.last_mut() {
                *parent_started = now;
            }
        });
    }
}

/// Associates one Rayon worker with the active apply profile. Nested timers then
/// suspend candidate lookup and rule-evaluation preparation, so each CPU interval has one owner.
pub(super) struct WorkerProfile;

impl WorkerProfile {
    pub(super) fn start() -> Self {
        let session = ACTIVE_SESSION
            .lock()
            .expect("profile session mutex poisoned")
            .clone();
        PROFILE.with(|profile| *profile.borrow_mut() = session);
        PHASE_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            stack.clear();
            stack.push((Phase::Manifest, thread_cpu_ns().unwrap_or_default()));
        });
        Self
    }
}

impl Drop for WorkerProfile {
    fn drop(&mut self) {
        let now = thread_cpu_ns().unwrap_or_default();
        while PHASE_STACK.with(|stack| !stack.borrow().is_empty()) {
            PHASE_STACK.with(|stack| {
                let (phase, started) = stack.borrow_mut().pop().expect("nonempty profile stack");
                with_profile(|profile| *phase_total(profile, phase) += now.saturating_sub(started));
            });
        }
        PROFILE.with(|profile| *profile.borrow_mut() = None);
    }
}

pub(super) fn record_nodes(count: usize) {
    with_profile(|profile| profile.visited_nodes += count);
}

pub(super) fn record_index(nodes: usize, bytes: usize, complete: bool) {
    with_profile(|profile| {
        profile.indexed_files += 1;
        profile.incomplete_indexes += usize::from(!complete);
        profile.indexed_nodes += nodes;
        profile.peak_index_bytes = profile.peak_index_bytes.max(bytes);
        let active = profile.active_navigation_line_bytes + bytes;
        profile.peak_transient_bytes = Some(profile.peak_transient_bytes.unwrap_or(0).max(active));
    });
}

pub(super) fn record_selection(roots: usize, fallback: bool) {
    with_profile(|profile| {
        profile.rule_files += 1;
        profile.rejected_rule_files += usize::from(roots == 0);
        profile.selected_root_pairs += roots;
        profile.fallback_rule_files += usize::from(fallback);
    });
}

pub(super) struct LineOwner;
impl LineOwner {
    pub(super) fn start(bytes: usize) -> Self {
        with_profile(|profile| {
            profile.active_line_bytes = bytes;
            profile.active_navigation_line_bytes = bytes;
            profile.peak_transient_bytes =
                Some(profile.peak_transient_bytes.unwrap_or(0).max(bytes));
        });
        Self
    }
}
impl Drop for LineOwner {
    fn drop(&mut self) {
        with_profile(|profile| {
            profile.active_line_bytes = 0;
            profile.active_navigation_line_bytes = 0;
        });
    }
}

pub(super) fn record_transient(navigation_bytes: usize) {
    with_profile(|profile| {
        let bytes = navigation_bytes + profile.active_line_bytes;
        profile.active_navigation_line_bytes = bytes;
        profile.peak_transient_bytes = Some(profile.peak_transient_bytes.unwrap_or(0).max(bytes));
    });
}

pub(super) fn record_retained(
    source: usize,
    tape: usize,
    grammar: usize,
    dispatch: usize,
    results: usize,
) {
    with_profile(|profile| {
        profile.retained_source_bytes = Some(source);
        profile.retained_tape_bytes = Some(tape);
        profile.retained_grammar_bytes = Some(grammar);
        profile.retained_dispatch_bytes = Some(dispatch);
        profile.retained_result_bytes = Some(results);
    });
}

pub(super) struct NativeOwner(Instant);
impl NativeOwner {
    pub(super) fn start() -> Self {
        with_profile(|profile| {
            profile.native_acquisitions += 1;
            profile.native_active += 1;
            profile.native_peak = profile.native_peak.max(profile.native_active);
        });
        Self(Instant::now())
    }
}
impl Drop for NativeOwner {
    fn drop(&mut self) {
        with_profile(|profile| {
            profile.native_active -= 1;
            profile.native_hold_ns += self.0.elapsed().as_nanos();
        });
    }
}

pub(super) struct DocOwner(Instant);
impl DocOwner {
    pub(super) fn start() -> Self {
        with_profile(|profile| {
            profile.doc_acquisitions += 1;
            profile.native_doc_overlap += usize::from(profile.native_active != 0);
            profile.doc_active += 1;
            profile.doc_peak = profile.doc_peak.max(profile.doc_active);
        });
        Self(Instant::now())
    }
}
impl Drop for DocOwner {
    fn drop(&mut self) {
        with_profile(|profile| {
            profile.doc_active -= 1;
            profile.active_navigation_line_bytes = profile.active_line_bytes;
            profile.doc_hold_ns += self.0.elapsed().as_nanos();
        });
    }
}

pub(super) struct ApplyProfile {
    started: Instant,
    cpu_started: Option<u128>,
}

#[cfg(unix)]
pub(super) fn thread_cpu_ns() -> Option<u128> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the pointer is valid writable storage; the clock belongs to this thread.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return None;
    }
    Some(u128::try_from(time.tv_sec).ok()? * 1_000_000_000 + u128::try_from(time.tv_nsec).ok()?)
}

#[cfg(not(unix))]
pub(super) fn thread_cpu_ns() -> Option<u128> {
    None
}

impl ApplyProfile {
    pub(super) fn start(revision: u32, cpu_started: Option<u128>) -> Self {
        let _ = Phase::Other;
        let session = Arc::new(Mutex::new(Profile {
            revision,
            ..Profile::default()
        }));
        *ACTIVE_SESSION
            .lock()
            .expect("profile session mutex poisoned") = Some(Arc::clone(&session));
        PROFILE.with(|profile| *profile.borrow_mut() = Some(session));
        PHASE_STACK.with(|stack| {
            stack.borrow_mut().clear();
            stack
                .borrow_mut()
                .push((Phase::Manifest, cpu_started.unwrap_or_default()));
        });
        Self {
            started: Instant::now(),
            cpu_started,
        }
    }
}

impl Drop for ApplyProfile {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_nanos();
        let worker_cpu_ns = self
            .cpu_started
            .zip(thread_cpu_ns())
            .and_then(|(start, end)| end.checked_sub(start));
        let now = thread_cpu_ns().unwrap_or_default();
        while PHASE_STACK.with(|stack| !stack.borrow().is_empty()) {
            PHASE_STACK.with(|stack| {
                let (phase, started) = stack.borrow_mut().pop().expect("nonempty profile stack");
                with_profile(|profile| *phase_total(profile, phase) += now.saturating_sub(started));
            });
        }
        let session = PROFILE.with(|profile| profile.borrow_mut().take());
        *ACTIVE_SESSION
            .lock()
            .expect("profile session mutex poisoned") = None;
        if let Some(session) = session {
            let profile = session.lock().expect("profile mutex poisoned");
            let total_worker_cpu_ns = profile.regex_ns
                + profile.parse_ns
                + profile.tape_encode_ns
                + profile.tape_decode_ns
                + profile.tape_serialize_ns
                + profile.store_read_ns
                + profile.store_write_ns
                + profile.manifest_ns
                + profile.index_ns
                + profile.selection_ns
                + profile.dispatch_ns
                + profile.serialization_ns
                + profile.other_ns;
            let named_worker_cpu_ns = total_worker_cpu_ns.saturating_sub(profile.other_ns);
            eprintln!(
                "RERAIL_POLICY_PROFILE {}",
                serde_json::json!({
                    "revision": profile.revision,
                    "applyNs": elapsed,
                    "workerCpuNs": worker_cpu_ns,
                    "totalWorkerCpuNs": total_worker_cpu_ns,
                    "namedWorkerCpuNs": named_worker_cpu_ns,
                    "namedWorkerCpuPercent": if total_worker_cpu_ns == 0 { 0.0 } else { named_worker_cpu_ns as f64 * 100.0 / total_worker_cpu_ns as f64 },
                    "storageReadNs": profile.store_read_ns,
                    "storageWritePublicationNs": profile.store_write_ns,
                    "manifestCandidateLookupNs": profile.manifest_ns,
                    "regexNs": profile.regex_ns,
                    "parseNs": profile.parse_ns,
                    "tapeEncodeNs": profile.tape_encode_ns,
                    "tapeDecodeRestoreNs": profile.tape_decode_ns,
                    "anchorIndexNs": profile.index_ns,
                    "anchorSelectionNs": profile.selection_ns,
                    "astMatchNs": profile.dispatch_ns,
                    "serializationNs": profile.serialization_ns,
                    "otherNs": profile.other_ns,
                    "retainedSourceBytesEstimate": profile.retained_source_bytes,
                    "retainedTapeBytesEstimate": profile.retained_tape_bytes,
                    "retainedGrammarBytesEstimate": profile.retained_grammar_bytes,
                    "retainedDispatchBytesEstimate": profile.retained_dispatch_bytes,
                    "retainedResultBytesEstimate": profile.retained_result_bytes,
                    "peakTransientNavigationLineIndexBytesEstimate": profile.peak_transient_bytes,
                    "visitedNodes": profile.visited_nodes,
                    "indexedFiles": profile.indexed_files,
                    "incompleteIndexes": profile.incomplete_indexes,
                    "indexedNodes": profile.indexed_nodes,
                    "peakIndexBytesEstimate": profile.peak_index_bytes,
                    "ruleFiles": profile.rule_files,
                    "rejectedRuleFiles": profile.rejected_rule_files,
                    "selectedRootPairs": profile.selected_root_pairs,
                    "fallbackRuleFiles": profile.fallback_rule_files,
                })
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_peak_includes_lines_without_a_document_and_counts_shared_lines_once() {
        let _apply = ApplyProfile::start(1, None);
        {
            let _lines = LineOwner::start(48);
            with_profile(|profile| assert_eq!(profile.peak_transient_bytes, Some(48)));
            record_transient(120);
            record_index(1, 32, true);
            with_profile(|profile| assert_eq!(profile.peak_transient_bytes, Some(200)));
            let doc = DocOwner::start();
            drop(doc);
            with_profile(|profile| assert_eq!(profile.active_navigation_line_bytes, 48));
        }
        with_profile(|profile| {
            assert_eq!(profile.active_navigation_line_bytes, 0);
            assert_eq!(profile.peak_transient_bytes, Some(200));
        });
    }

    #[test]
    fn nested_phase_timers_charge_only_the_innermost_phase() {
        let _apply = ApplyProfile::start(1, thread_cpu_ns());
        {
            let _outer = PhaseTimer::start(Phase::StoreRead);
            std::thread::sleep(std::time::Duration::from_millis(1));
            {
                let _inner = PhaseTimer::start(Phase::Parse);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        with_profile(|profile| {
            assert!(profile.store_read_ns > 0);
            assert!(profile.parse_ns > 0);
            assert_eq!(profile.other_ns, 0);
        });
    }
}
