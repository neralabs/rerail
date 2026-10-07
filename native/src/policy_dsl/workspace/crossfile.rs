//! The workspace's cross-file scheduling seam.
//!
//! Adapters retain their relation representation: evidence owns partitions and relational global
//! evaluation, while `calls` owns summaries and graph closure. This module owns the protocol that
//! keeps their producer and consumer phases ordered.

use super::calls_state;
use super::evaluate::{Job, JobContext, run_jobs};
use super::global::{forget_evidence, record_evidence};
use super::ingest::{candidate_dispatch, make_file};
use super::persisted::file_context;
use super::{CheckId, FileState, WorkspaceState};
use crate::content_store::{HashKey, LocalResultPublication, StoredLocalResult};
use crate::policy_dsl::eval::file_language;
use crate::policy_dsl::json::invalid;
use crate::policy_dsl::model::{CheckKind, CompiledPolicy, FileInput, Metrics};
use crate::policy_dsl::tape;
use napi::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Per-file state owned by cross-file adapters. Evidence has no file-local state; `calls` keeps
/// its summary and consumer stamp here so workspace file storage does not know graph details.
#[derive(Default)]
pub(super) struct CrossFileFileState {
    pub(super) summary: Option<(u32, Arc<crate::policy_dsl::calls::summary::Summary>)>,
    pub(super) summary_stored: bool,
    pub(super) calls_stamp: Option<HashKey>,
}

pub(super) struct CrossFileConsumer {
    view: Arc<crate::policy_dsl::calls::CallsView>,
    summary: Arc<crate::policy_dsl::calls::summary::Summary>,
}
/// A producer batch is evaluated before either adapter may refresh shared state. In particular,
/// deferred batches record producers but intentionally do not refresh a closure.
pub(super) struct ProducerBatch {
    calls_active: bool,
    consumes: HashMap<String, Vec<CheckId>>,
}

/// Mutable apply state the consumer phase updates after a closure refresh.
pub(super) struct ConsumerRefresh<'a> {
    pub(super) deferred: bool,
    pub(super) replaced: &'a mut BTreeSet<String>,
    pub(super) persistence_paths: &'a mut Vec<String>,
    pub(super) metrics: &'a mut Metrics,
    pub(super) evaluation_error: &'a mut Option<napi::Error>,
    pub(super) context: JobContext<'a>,
}

/// Schedules cross-file adapters without normalizing their storage or closure implementations.
#[derive(Default)]
pub(super) struct CrossFileScheduler {
    calls: calls_state::CallsState,
}

impl CrossFileScheduler {
    pub(super) fn register(
        state: &mut WorkspaceState,
        index: usize,
        policy: &mut CompiledPolicy,
    ) -> Result<()> {
        if policy.checks.iter().any(
            |check| matches!(&check.kind, CheckKind::Forbid(query) if !query.calls().is_empty()),
        ) {
            state.crossfile.calls.register(index, policy)?;
        }
        Ok(())
    }

    pub(super) fn consumers(state: &WorkspaceState) -> &calls_state::Consumers {
        state.crossfile.calls.consumers()
    }

    #[cfg(test)]
    pub(super) fn summary(
        state: &WorkspaceState,
        path: &str,
    ) -> Option<Arc<crate::policy_dsl::calls::summary::Summary>> {
        state.crossfile.calls.summary(path).cloned()
    }

    pub(super) fn result_context(file: &FileState) -> HashKey {
        let context = file_context(&file.input);
        file.crossfile
            .calls_stamp
            .map_or(context, |stamp| calls_state::calls_context(context, stamp))
    }

    pub(super) fn stamped_context(file: &FileState, context: HashKey) -> Option<HashKey> {
        file.crossfile
            .calls_stamp
            .map(|stamp| calls_state::calls_context(context, stamp))
    }

    pub(super) fn summary_publication(file: &FileState) -> Result<Option<LocalResultPublication>> {
        calls_state::summary_publication(file)
    }

    pub(super) fn restore_summary(file: &mut FileState, persisted: &[StoredLocalResult]) {
        calls_state::restore_summary(file, persisted);
    }

    pub(super) fn summarize_file(
        file: &mut FileState,
        grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
        closed: &AtomicBool,
        metrics: &mut Metrics,
        keep_table: bool,
        restored: Option<tape::TapeDoc>,
    ) -> Result<()> {
        calls_state::summarize_file(file, grammars, closed, metrics, keep_table, restored)
    }

    pub(super) fn install(
        consumer: Option<CrossFileConsumer>,
    ) -> Option<crate::policy_dsl::calls::FileCallsSlot> {
        consumer.map(|consumer| {
            crate::policy_dsl::calls::FileCallsSlot::install(consumer.view, consumer.summary)
        })
    }

    /// Whether `input` must run as a job rather than be restored from complete results: the
    /// `calls` adapter records each Python, PHP and Blade file's summary as its job restores or
    /// evaluates it.
    pub(super) fn needs_job(state: &WorkspaceState, input: &FileInput) -> bool {
        state.crossfile.calls.active()
            && file_language(input).is_some_and(crate::policy_dsl::calls::summarizes)
    }

    pub(super) fn is_producer(state: &WorkspaceState, id: &CheckId) -> bool {
        state.crossfile.calls.is_producer(id)
    }

    pub(super) fn mark_persisted(file: &mut FileState) {
        file.crossfile.summary_stored |=
            file.crossfile.summary.is_some() && !file.transient_failure;
    }

    pub(super) fn stamped_evicted_key(
        state: &WorkspaceState,
        path: &str,
        key: (HashKey, HashKey),
    ) -> Option<(HashKey, HashKey)> {
        state
            .crossfile
            .calls
            .stamp(path)
            .map(|stamp| (key.0, calls_state::calls_context(key.1, stamp)))
    }

    pub(super) fn quiet(
        state: &WorkspaceState,
        input: &FileInput,
        policies: &[Arc<CompiledPolicy>],
    ) -> Vec<CheckId> {
        state.crossfile.calls.quiet(input, policies, None)
    }
    /// Removes `calls` consumers from phase A, leaving their seed producers there.
    pub(super) fn prepare(
        state: &mut WorkspaceState,
        jobs: &mut [Job],
        policies: &[Arc<CompiledPolicy>],
        deferred: bool,
    ) -> ProducerBatch {
        let calls_active = state.crossfile.calls.active();
        let mut consumes = HashMap::new();
        if calls_active {
            for job in jobs {
                let deferred_checks = state.crossfile.calls.defer(&mut job.file, policies);
                if !deferred_checks.is_empty() {
                    consumes.insert(job.path.clone(), deferred_checks);
                    job.consumes = !deferred;
                }
            }
        }
        ProducerBatch {
            calls_active,
            consumes,
        }
    }

    pub(super) fn summarizes(batch: &ProducerBatch) -> bool {
        batch.calls_active
    }

    /// What a file keeps of its scope tables once its job is done: for its consumers' pass, the
    /// table `calls` terms read; nothing otherwise.
    pub(super) fn retain_scopes(file: &mut FileState, consumes: bool) {
        calls_state::retain_scopes(file, consumes);
    }

    pub(super) fn release_producer_scopes(state: &mut WorkspaceState, batch: &ProducerBatch) {
        for path in batch.consumes.keys() {
            if let Some(file) = state.files.get_mut(path) {
                file.scopes = None;
            }
        }
    }

    /// Publishes the graph adapter's producer output after the evidence adapter has retained its
    /// file partition.
    pub(super) fn record_calls(
        state: &mut WorkspaceState,
        batch: &ProducerBatch,
        path: &str,
        file: &FileState,
    ) {
        if batch.calls_active {
            state
                .crossfile
                .calls
                .record(path, file, batch.consumes.get(path).map(Vec::as_slice));
        }
    }

    /// Publishes all producer output from phase A.
    pub(super) fn record(
        state: &mut WorkspaceState,
        batch: &ProducerBatch,
        path: &str,
        file: &FileState,
    ) {
        record_evidence(state, path, file);
        Self::record_calls(state, batch, path, file);
    }

    /// Refreshes the calls closure only after every affected producer has been recorded, then
    /// restamps and evaluates precisely the consumers whose closure inputs changed.
    pub(super) fn refresh_consumers(
        state: &mut WorkspaceState,
        batch: &ProducerBatch,
        refresh: ConsumerRefresh<'_>,
    ) -> Result<HashSet<String>> {
        let mut evicted = HashSet::new();
        if refresh.deferred || !batch.calls_active {
            return Ok(evicted);
        }
        let Some((view, changed)) = state.crossfile.calls.refresh() else {
            return Ok(evicted);
        };
        let policies = refresh.context.policies;
        let mut jobs = Vec::new();
        // Each job's silent terms, recorded with its stamp once the job is done.
        let mut silent_terms = HashMap::new();
        for calls_state::Restamp {
            path,
            stamp,
            silent,
        } in changed
        {
            let Some(summary) = state.crossfile.calls.summary(&path).cloned() else {
                continue;
            };
            let Some(input) = state
                .files
                .get(&path)
                .map(|file| &file.input)
                .or_else(|| state.evicted_inputs.get(&path))
            else {
                continue;
            };
            let quiet = state.crossfile.calls.quiet(input, policies, Some(&silent));
            if quiet.len() == state.crossfile.calls.consumers_for(input, policies).len() {
                if let Some(file) = state.files.get_mut(&path) {
                    for id in quiet {
                        file.witnesses.insert(id, Vec::new());
                        file.failures.remove(&id);
                    }
                    file.crossfile.calls_stamp = Some(stamp);
                }
                state.crossfile.calls.set_stamp(&path, stamp, silent);
                refresh.replaced.insert(path);
                continue;
            }
            let (mut file, was_evicted) = match state.files.remove(&path) {
                Some(file) => (file, false),
                None => {
                    let (Some(input), Some(version)) = (
                        state.evicted_inputs.get(&path).cloned(),
                        state.versions.get(&path),
                    ) else {
                        continue;
                    };
                    let source = version.result_key().0;
                    let revision = state.revision.max(1);
                    let mut file = make_file(input, revision, policies, &mut state.candidate_sets);
                    file.source_digest = source
                        .sha256_bytes()
                        .ok_or_else(|| invalid("stored source hash is not SHA-256"))?;
                    (file, true)
                }
            };
            let mut consumers = state.crossfile.calls.consumers_for(&file.input, policies);
            consumers.retain(|id| !quiet.contains(id));
            file.candidates = Arc::new(candidate_dispatch(
                file_language(&file.input),
                consumers,
                policies,
                &file.input,
            ));
            file.crossfile.calls_stamp = Some(stamp);
            file.restored_local_results = 0;
            let consumers = state.crossfile.calls.consumers();
            file.restored_result_ids
                .retain(|id| !consumers.contains_key(id));
            for id in quiet {
                file.witnesses.insert(id, Vec::new());
                file.failures.remove(&id);
                file.restored_result_ids.insert(id);
            }
            if was_evicted {
                evicted.insert(path.clone());
            }
            silent_terms.insert(path.clone(), silent);
            jobs.push(Job {
                path,
                file,
                consumes: false,
                crossfile: Some(CrossFileConsumer {
                    view: Arc::clone(&view),
                    summary,
                }),
            });
        }
        let context = JobContext {
            summarize: false,
            ..refresh.context
        };
        let phase_b = match run_jobs(&mut jobs, &context) {
            Ok(phase_b) => phase_b,
            Err(error) => {
                // As phase A does: the files go back, and a file made resident only for this pass
                // stays evicted.
                for Job { path, file, .. } in jobs {
                    if !evicted.contains(&path) {
                        state.insert_file(path, file);
                    }
                }
                return Err(error);
            }
        };
        let mut persisting = refresh
            .persistence_paths
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        for (path, file, work) in phase_b {
            match work {
                Ok(work) => refresh.metrics.merge(work),
                Err(error) if refresh.evaluation_error.is_none() => {
                    *refresh.evaluation_error = Some(error)
                }
                Err(_) => {}
            }
            if let Some(stamp) = file.crossfile.calls_stamp {
                let silent = silent_terms.remove(&path).unwrap_or_default();
                state.crossfile.calls.set_stamp(&path, stamp, silent);
            }
            refresh.replaced.insert(path.clone());
            let evaluated = file
                .candidates
                .ast
                .iter()
                .any(|id| !file.restored_result_ids.contains(id));
            if evaluated && persisting.insert(path.clone()) {
                refresh.persistence_paths.push(path.clone());
            }
            if evaluated || !evicted.contains(&path) {
                state.insert_file(path, file);
            }
        }
        Ok(evicted)
    }

    /// Drops all state attributable to a path, including state retained after file eviction.
    pub(super) fn forget(state: &mut WorkspaceState, path: &str) {
        forget_evidence(state, path);
        state.crossfile.calls.forget(path);
    }

    #[cfg(test)]
    pub(super) fn calls_state(state: &WorkspaceState) -> &calls_state::CallsState {
        &state.crossfile.calls
    }
}
