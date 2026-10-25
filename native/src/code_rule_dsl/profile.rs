//! Benchmark-only attribution for the existing workspace evaluator.
//! Disabled in production builds. Reports worker CPU separately from wall time.

mod clock;
pub(super) mod memory;
pub(super) use clock::thread_cpu_ns;
pub(super) mod navigation;
mod semantic;
pub(super) mod sql;
pub(crate) mod trace;
use ast_grep_core::matcher::{DumpPattern, Matcher};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use super::matcher::{AstMatcher, StopBy};
use crate::code_rule_dsl::language::GrammarDialect;

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
            "as": count.binding.as_ref().map(|binding| &binding.name),
            "offset": count.binding.as_ref().map(|binding| binding.offset),
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
    rule_id: &str,
    check_id: &str,
    language: GrammarDialect,
    matcher: &AstMatcher,
    root_kinds: Option<BTreeSet<u16>>,
) {
    if std::env::var_os("RERAIL_CODE_RULE_ANCHORS").is_none() {
        return;
    }
    trace::emit(
        "RERAIL_CODE_RULE_ANCHOR",
        json!({
            "ruleId": rule_id,
            "clauseId": check_id,
            "language": crate::code_rule_dsl::language::label(language),
            "rootKinds": root_kinds.map(|kinds| kinds.into_iter()
                .map(|id| language.kind_name(id)).collect::<Vec<_>>()),
            "matcher": describe_matcher(matcher, language),
        }),
    );
}

#[derive(Default)]
struct Profile {
    field_lookups: navigation::Counters,
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
    anchor_index_views: usize,
    anchor_batches: usize,
    anchor_sql_data_statements: usize,
    anchor_sql_materializations: usize,
    anchor_batch_files: usize,
    anchor_batch_plans: usize,
    anchor_batch_failures: usize,
    anchor_cache_misses: usize,
    peak_candidate_bytes: usize,
    anchor_index_view_nodes: usize,
    sql_index_writes: usize,
    sql_index_written_nodes: usize,
    sql_index_reuses: usize,
    incomplete_indexes: usize,
    anchor_plan_broadenings: usize,
    indexed_nodes: usize,
    peak_index_bytes: usize,
    active_line_bytes: usize,
    active_navigation_line_bytes: usize,
    peak_transient_bytes: Option<usize>,
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
pub(super) struct WorkerProfile(bool);

impl WorkerProfile {
    pub(super) fn start() -> Self {
        // Rayon can run a small iterator on the apply thread itself. Keep that
        // thread's session and timers; clearing them loses the rest of the apply.
        if PROFILE.with(|profile| profile.borrow().is_some()) {
            return Self(false);
        }
        let session = ACTIVE_SESSION
            .lock()
            .expect("profile session mutex poisoned")
            .clone();
        PROFILE.with(|profile| *profile.borrow_mut() = session);
        navigation::reset();
        PHASE_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            stack.clear();
            stack.push((Phase::Manifest, thread_cpu_ns().unwrap_or_default()));
        });
        Self(true)
    }
}

impl Drop for WorkerProfile {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        let now = thread_cpu_ns().unwrap_or_default();
        while PHASE_STACK.with(|stack| !stack.borrow().is_empty()) {
            PHASE_STACK.with(|stack| {
                let (phase, started) = stack.borrow_mut().pop().expect("nonempty profile stack");
                with_profile(|profile| *phase_total(profile, phase) += now.saturating_sub(started));
            });
        }
        navigation::flush();
        PROFILE.with(|profile| *profile.borrow_mut() = None);
    }
}

pub(super) fn record_nodes(count: usize) {
    with_profile(|profile| profile.visited_nodes += count);
}

pub(super) fn record_anchor_batch(files: usize, plans: usize, materializations: usize) {
    with_profile(|profile| {
        profile.anchor_batches += 1;
        profile.anchor_sql_data_statements += materializations + 1;
        profile.anchor_sql_materializations += materializations;
        profile.anchor_batch_files += files;
        profile.anchor_batch_plans += plans;
    });
}

pub(super) fn record_anchor_batch_failure() {
    with_profile(|p| p.anchor_batch_failures += 1);
}

pub(super) fn record_anchor_statements(statements: usize) {
    with_profile(|p| p.anchor_sql_data_statements += statements);
}

pub(super) fn record_anchor_cache_miss() {
    with_profile(|p| p.anchor_cache_misses += 1);
}

pub(super) fn record_candidate_bytes(bytes: usize) {
    with_profile(|p| p.peak_candidate_bytes = p.peak_candidate_bytes.max(bytes));
}

pub(super) fn record_sql_index_write(nodes: usize) {
    with_profile(|profile| {
        profile.sql_index_writes += 1;
        profile.sql_index_written_nodes += nodes;
        profile.indexed_files += 1;
        profile.indexed_nodes += nodes;
    });
}

pub(super) fn record_sql_index_reuse() {
    with_profile(|profile| profile.sql_index_reuses += 1);
}

pub(super) fn record_index(nodes: usize, bytes: usize, complete: bool) {
    with_profile(|profile| {
        profile.anchor_index_views += 1;
        profile.anchor_index_view_nodes += nodes;
        profile.incomplete_indexes += usize::from(!complete);
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

pub(super) fn record_anchor_plan_broadening() {
    with_profile(|profile| profile.anchor_plan_broadenings += 1);
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

impl ApplyProfile {
    pub(super) fn start(revision: u32, cpu_started: Option<u128>) -> Self {
        navigation::reset();
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
        navigation::flush();
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
            let mut report = serde_json::json!({
                "revision": profile.revision,
                "fieldLookups": profile.field_lookups,
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
                "peakTransientNavigationLineIndexBytesEstimate": profile.peak_transient_bytes,
                "visitedNodes": profile.visited_nodes,
                "indexedFiles": profile.indexed_files,
                "anchorIndexViews": profile.anchor_index_views,
                "anchorIndexViewNodes": profile.anchor_index_view_nodes,
                "sqlIndexWrites": profile.sql_index_writes,
                "sqlIndexWrittenNodes": profile.sql_index_written_nodes,
                "sqlIndexReuses": profile.sql_index_reuses,
                "incompleteIndexes": profile.incomplete_indexes,
                "anchorPlanBroadenings": profile.anchor_plan_broadenings,
                "indexedNodes": profile.indexed_nodes,
                "peakIndexBytesEstimate": profile.peak_index_bytes,
                "ruleFiles": profile.rule_files,
                "rejectedRuleFiles": profile.rejected_rule_files,
                "selectedRootPairs": profile.selected_root_pairs,
                "fallbackRuleFiles": profile.fallback_rule_files,
            });
            report.as_object_mut().expect("profile object").extend(
                serde_json::json!({
                    "anchorBatches": profile.anchor_batches,
                    "anchorSqlDataStatements": profile.anchor_sql_data_statements,
                    "anchorSqlMaterializations": profile.anchor_sql_materializations,
                    "anchorBatchFiles": profile.anchor_batch_files,
                    "anchorBatchPlans": profile.anchor_batch_plans,
                    "anchorBatchFailures": profile.anchor_batch_failures,
                    "anchorCacheMisses": profile.anchor_cache_misses,
                    "peakPreparedCandidateBytes": profile.peak_candidate_bytes,
                })
                .as_object()
                .expect("batch counters")
                .clone(),
            );
            trace::emit("RERAIL_CODE_RULE_PROFILE", report);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_worker_keeps_the_apply_session_and_phase_stack() {
        let _apply = ApplyProfile::start(17, thread_cpu_ns());
        let session = PROFILE.with(|profile| profile.borrow().clone().unwrap());
        let before = PHASE_STACK.with(|stack| stack.borrow().len());
        {
            let _worker = WorkerProfile::start();
            let _timer = PhaseTimer::start(Phase::Other);
        }
        assert!(PROFILE.with(|profile| Arc::ptr_eq(profile.borrow().as_ref().unwrap(), &session)));
        assert_eq!(PHASE_STACK.with(|stack| stack.borrow().len()), before);
    }

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
