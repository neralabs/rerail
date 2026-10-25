//! The workspace's cross-file scheduling seam.
//!
//! Adapters retain their relation representation: evidence owns partitions and relational global
//! evaluation, while `calls` owns summaries and graph closure. This module owns the protocol that
//! keeps their producer and consumer phases ordered.

use super::calls_state;
use super::documents;
use super::evaluate::{Job, JobContext, run_jobs};
use super::global::{forget_evidence, record_evidence};
use super::ingest::{candidate_dispatch, make_file};
use super::persisted::file_context;
use super::{ClauseId, FileState, WorkspaceState};
use crate::code_rule_dsl::eval::file_language;
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::model::{CompiledCodeRule, FileInput, Metrics};
use crate::code_rule_dsl::tape;
use crate::content_store::{HashKey, LocalResultPublication, StoredLocalResult};
use napi::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Per-file state owned by cross-file adapters. Evidence has no file-local state; `calls` keeps
/// its summary and consumer stamp here so workspace file storage does not know graph details.
#[derive(Default)]
pub(super) struct CrossFileFileState {
    pub(super) summary: Option<(u32, Arc<crate::code_rule_dsl::calls::summary::Summary>)>,
    pub(super) summary_stored: bool,
    pub(super) calls_stamp: Option<HashKey>,
}

pub(super) struct CrossFileConsumer {
    view: Arc<crate::code_rule_dsl::calls::CallsView>,
    summary: Arc<crate::code_rule_dsl::calls::summary::Summary>,
}
/// A producer batch is evaluated before either adapter may refresh shared state. In particular,
/// deferred batches record producers but intentionally do not refresh a closure.
pub(super) struct ProducerBatch {
    calls_active: bool,
    consumes: HashMap<String, Vec<ClauseId>>,
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
    pub(super) fn evaluation_slot(&self) -> Option<crate::code_rule_dsl::calls::EvaluationSlot> {
        self.calls.evaluation_slot()
    }
    pub(super) fn register(
        state: &mut WorkspaceState,
        index: usize,
        code_rule: &mut CompiledCodeRule,
    ) -> Result<()> {
        if code_rule.clauses.iter().any(|clause| {
            clause
                .kind
                .calls_query()
                .is_some_and(|query| !query.calls().is_empty())
        }) {
            state.crossfile.calls.register(index, code_rule)?;
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
    ) -> Option<Arc<crate::code_rule_dsl::calls::summary::Summary>> {
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
    ) -> Option<crate::code_rule_dsl::calls::FileCallsSlot> {
        consumer.map(|consumer| {
            crate::code_rule_dsl::calls::FileCallsSlot::install(consumer.view, consumer.summary)
        })
    }

    /// Whether `input` must run as a job rather than be restored from complete results: the
    /// `calls` adapter records each Python, PHP and Blade file's summary as its job restores or
    /// evaluates it.
    pub(super) fn needs_job(state: &WorkspaceState, input: &FileInput) -> bool {
        state.crossfile.calls.active()
            && file_language(input).is_some_and(crate::code_rule_dsl::calls::summarizes)
    }

    pub(super) fn is_producer(state: &WorkspaceState, id: &ClauseId) -> bool {
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
        code_rules: &[Arc<CompiledCodeRule>],
    ) -> Vec<ClauseId> {
        state.crossfile.calls.quiet(input, code_rules, None)
    }
    /// Removes `calls` consumers from phase A, leaving their seed producers there.
    pub(super) fn prepare(
        state: &mut WorkspaceState,
        jobs: &mut [Job],
        code_rules: &[Arc<CompiledCodeRule>],
        deferred: bool,
    ) -> ProducerBatch {
        let calls_active = state.crossfile.calls.active();
        let mut consumes = HashMap::new();
        if calls_active {
            for job in jobs {
                let deferred_clauses = state.crossfile.calls.defer(&mut job.file, code_rules);
                if !deferred_clauses.is_empty() {
                    consumes.insert(job.path.clone(), deferred_clauses);
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
        state
            .dirty_globals
            .extend(state.crossfile.calls.take_dirty_globals());
        let code_rules = refresh.context.code_rules;
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
            let quiet = state
                .crossfile
                .calls
                .quiet(input, code_rules, Some(&silent));
            if quiet.len() == state.crossfile.calls.consumers_for(input, code_rules).len() {
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
            // A file whose source a busy store made the workspace free is evaluated on the source
            // the document contract reads back, as a stored file's is. When nothing can supply it
            // (outside Git, or the worktree changed) the consumer clauses are reported
            // incomplete: a busy store costs reuse, never the apply.
            let released = if state.files.get(&path).is_some_and(|file| file.results_only) {
                match documents::load(state, &path) {
                    Ok((_, document)) => Some(Arc::clone(&document.source)),
                    Err(_) => {
                        if let Some(file) = state.files.get_mut(&path) {
                            let mut consumers =
                                state.crossfile.calls.consumers_for(&file.input, code_rules);
                            consumers.retain(|id| !quiet.contains(id));
                            for &id in &quiet {
                                file.witnesses.insert(id, Vec::new());
                                file.failures.remove(&id);
                            }
                            file.fail_released_source(consumers);
                            file.crossfile.calls_stamp = Some(stamp);
                        }
                        state.crossfile.calls.set_stamp(&path, stamp, silent);
                        refresh.replaced.insert(path);
                        continue;
                    }
                }
            } else {
                None
            };
            let (mut file, was_evicted) = match state.files.remove(&path) {
                Some(mut file) => {
                    if let Some(source) = released {
                        file.input.source = source;
                        file.results_only = false;
                    }
                    (file, false)
                }
                None => {
                    let (Some(input), Some(version)) = (
                        state.evicted_inputs.get(&path).cloned(),
                        state.versions.get(&path),
                    ) else {
                        continue;
                    };
                    let source = version.result_key().0;
                    let revision = state.revision.max(1);
                    let mut file =
                        make_file(input, revision, code_rules, &mut state.candidate_sets);
                    file.source_digest = source
                        .sha256_bytes()
                        .ok_or_else(|| invalid("stored source hash is not SHA-256"))?;
                    (file, true)
                }
            };
            let mut consumers = state.crossfile.calls.consumers_for(&file.input, code_rules);
            consumers.retain(|id| !quiet.contains(id));
            file.candidates = Arc::new(candidate_dispatch(
                file_language(&file.input),
                consumers,
                code_rules,
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
