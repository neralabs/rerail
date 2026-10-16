//! File evaluation: jobs run serially or on the worker pool, regex clauses, tape parsing, and AST
//! clause evaluation under the match budget.

use super::crossfile::{CrossFileConsumer, CrossFileScheduler};
use super::documents;
use super::persisted::{candidate_catalog, restore_persisted_file, result_context};
use super::storage::StorageContext;
use super::{
    ClauseId, ExecutionEngine, FileState, LocalFailure, REUSE_LOCAL_RESULTS, WorkspaceState,
    check_closed, file_work_budget,
};
#[cfg(test)]
use super::{TEST_STORAGE_PREFETCH_FAILURE, pass_deadline_at_test_phase, pause_at_test_phase};
use crate::code_rule_dsl::blade::forte::tree::parse_tree;
use crate::code_rule_dsl::budget::{
    MatchBudget, matcher_budget_exhausted, matcher_budget_observed,
};
use crate::code_rule_dsl::eval::{
    WitnessError, file_language, grammar_matcher, witness_from_match, witnesses_from_match,
};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    FileAnchorIndex, FileScopeSlot, GrammarMatcher, Selection, UtilLimit,
};
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledCodeRule, FileInput, Metrics, Query, RegexSource,
};
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile;
use crate::code_rule_dsl::soundness::SyntaxErrors;
use crate::code_rule_dsl::violations::{Captures, Witness};
use crate::code_rule_dsl::{query, tape, violations};
use crate::content_store::HashKey;
use crate::{WORK_POOL, WORKER_THREADS};
use ast_grep_core::AstGrep as Root;
use ast_grep_language::LanguageExt;
use napi::Result;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tree_sitter::{ParseOptions, Parser, Point};

/// The matcher and anchor plan for this file's grammar; candidates only exist for supported grammars.
pub(super) fn candidate_grammar<'a>(
    code_rule: &'a CompiledCodeRule,
    clause: usize,
    file: &FileInput,
) -> &'a GrammarMatcher {
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &code_rule.clauses[clause].kind else {
        unreachable!()
    };
    grammar_matcher(matchers, file).expect("AST candidate has a grammar matcher")
}

/// One file to evaluate in an apply.
pub(super) struct Job {
    pub(super) path: String,
    pub(super) file: FileState,
    /// Its adapter consumer pass runs after phase-A producer publication.
    pub(super) consumes: bool,
    pub(super) crossfile: Option<CrossFileConsumer>,
}

/// What every job of an apply reads.
#[derive(Clone, Copy)]
pub(super) struct JobContext<'a> {
    pub(super) code_rules: &'a [Arc<CompiledCodeRule>],
    pub(super) sql: Option<&'a query::SqlPlans>,
    pub(super) storage: Option<&'a StorageContext>,
    pub(super) closed: &'a Arc<AtomicBool>,
    /// Extract each Python file's call summary (a code rule holds `calls` terms).
    pub(super) summarize: bool,
}

type Evaluated = Vec<(String, FileState, Result<Metrics>)>;

/// One restored AST file after parsing and anchor selection, before clause execution.
struct PreparedAst {
    doc: tape::TapeDoc,
    errors: SyntaxErrors,
    lines: Arc<LineTable>,
    roots: Vec<Option<Arc<[u32]>>>,
    util_work: Vec<usize>,
    max_nodes: usize,
}

struct PendingSqlJob {
    path: String,
    file: FileState,
    consumes: bool,
    crossfile: Option<CrossFileConsumer>,
    metrics: Metrics,
    prepared: Option<PreparedAst>,
    restored: Option<tape::TapeDoc>,
    deadline: crate::code_rule_dsl::budget::MatchDeadline,
    error: Option<napi::Error>,
}

/// Restores what the store holds for each job and evaluates the rest: serially for a few files,
/// on the worker pool for more.
pub(super) fn run_jobs(jobs: &mut Vec<Job>, context: &JobContext<'_>) -> Result<Evaluated> {
    let catalog = candidate_catalog(context.code_rules)?;
    let result_manifest = match context.storage.filter(|_| REUSE_LOCAL_RESULTS) {
        Some(storage) => {
            let keys = jobs
                .iter()
                .map(|job| {
                    (
                        HashKey::from_sha256_digest(job.file.source_digest),
                        result_context(&job.file),
                    )
                })
                .collect::<Vec<_>>();
            #[cfg(test)]
            if TEST_STORAGE_PREFETCH_FAILURE.replace(false) {
                return Err(invalid("injected local-result storage failure"));
            }
            storage
                .store
                .local_check_results_by_key(&keys, catalog)
                .map_err(|error| invalid(error.to_string()))?
        }
        None => HashMap::new(),
    };
    // The storage contract's batch request: every job's stored source and tapes.
    let stored_documents = match context.storage {
        Some(storage) => {
            let source_hashes = jobs
                .iter()
                .map(|job| HashKey::from_sha256_digest(job.file.source_digest))
                .collect::<Vec<_>>();
            documents::prefetch(&storage.store, &source_hashes)?
        }
        None => documents::StoredDocuments::default(),
    };
    let jobs = std::mem::take(jobs);
    let work = |job: Job, grammars: &mut BTreeMap<String, Arc<tape::Grammar>>| {
        let Job {
            path,
            mut file,
            consumes,
            crossfile,
        } = job;
        let closed = context.closed;
        let work = check_closed(closed).and_then(|()| {
            #[cfg(feature = "code-rule-profile")]
            let _worker_profile = profile::WorkerProfile::start();
            let budget = MatchBudget::work(closed, file_work_budget());
            let storage_started = Instant::now();
            #[cfg(feature = "code-rule-profile")]
            let _storage_read_timer = profile::PhaseTimer::start(profile::Phase::StoreRead);
            if let Some(storage) = context.storage {
                let source_hash = HashKey::from_sha256_digest(file.source_digest);
                let result_key = (source_hash, result_context(&file));
                let persisted = result_manifest
                    .get(&result_key)
                    .map(|results| results.as_slice());
                if context.summarize {
                    CrossFileScheduler::restore_summary(&mut file, persisted.unwrap_or_default());
                }
                restore_persisted_file(
                    &stored_documents,
                    storage.store.as_ref(),
                    &mut file,
                    grammars,
                    context.code_rules,
                    persisted,
                    source_hash,
                )?;
                // A stored file whose source was dropped is read back only for an adapter
                // consumer pass the store did not answer.
                let unanswered = (!file.candidates.ast.is_empty() && !file.selector_skipped)
                    || !file.candidates.regex.is_empty();
                if unanswered && file.input.source.is_empty() && file.stored.is_some() {
                    let stored = stored_documents
                        .objects
                        .get(&source_hash)
                        .ok_or_else(|| invalid(format!("{path} is not in the store")))?;
                    let source = storage
                        .store
                        .get_source(stored.source)
                        .map_err(|error| invalid(error.to_string()))?;
                    file.input.source = Arc::new(
                        String::from_utf8(source)
                            .map_err(|_| invalid(format!("stored {path} is not UTF-8")))?,
                    );
                }
            }
            #[cfg(feature = "code-rule-profile")]
            drop(_storage_read_timer);
            let storage_read_nanoseconds = storage_started.elapsed().as_nanos() as u64;
            let transient = file.transient_failure && crossfile.is_some();
            let _consumer = CrossFileScheduler::install(crossfile);
            let mut restored = None;
            let mut metrics = evaluate_file(
                &mut file,
                context.code_rules,
                grammars,
                closed,
                context.sql,
                &budget,
                &mut restored,
            )?;
            // A consumers' pass after a transient evaluation of the file stays transient.
            file.transient_failure |= transient;
            if context.summarize {
                CrossFileScheduler::summarize_file(
                    &mut file,
                    grammars,
                    closed,
                    &mut metrics,
                    consumes,
                    restored,
                )?;
            }
            metrics.storage_read_nanoseconds = storage_read_nanoseconds;
            Ok(metrics)
        });
        CrossFileScheduler::retain_scopes(&mut file, consumes);
        (path, file, work)
    };
    let sql_work = |jobs: Vec<Job>, grammars: &mut BTreeMap<String, Arc<tape::Grammar>>| {
        let sql = context.sql.expect("SQLite jobs have compiled plans");
        let mut pending = WORK_POOL.install(|| {
            jobs.into_par_iter()
                .map_init(BTreeMap::new, |grammars, job| {
                    let Job {
                        path,
                        mut file,
                        consumes,
                        crossfile,
                    } = job;
                    let mut prepared = None;
                    let mut restored = None;
                    let mut deadline = None;
                    let metrics_result = check_closed(context.closed).and_then(|()| {
                        #[cfg(feature = "code-rule-profile")]
                        let _worker_profile = profile::WorkerProfile::start();
                        let budget = MatchBudget::work(context.closed, file_work_budget());
                        deadline = Some(budget.deadline());
                        let storage_started = Instant::now();
                        #[cfg(feature = "code-rule-profile")]
                        let _storage_read_timer =
                            profile::PhaseTimer::start(profile::Phase::StoreRead);
                        if let Some(storage) = context.storage {
                            let source_hash = HashKey::from_sha256_digest(file.source_digest);
                            let result_key = (source_hash, result_context(&file));
                            let persisted = result_manifest
                                .get(&result_key)
                                .map(|results| results.as_slice());
                            if context.summarize {
                                CrossFileScheduler::restore_summary(
                                    &mut file,
                                    persisted.unwrap_or_default(),
                                );
                            }
                            restore_persisted_file(
                                &stored_documents,
                                storage.store.as_ref(),
                                &mut file,
                                grammars,
                                context.code_rules,
                                persisted,
                                source_hash,
                            )?;
                            let unanswered = (!file.candidates.ast.is_empty()
                                && !file.selector_skipped)
                                || !file.candidates.regex.is_empty();
                            if unanswered && file.input.source.is_empty() && file.stored.is_some() {
                                let stored =
                                    stored_documents.objects.get(&source_hash).ok_or_else(
                                        || invalid(format!("{path} is not in the store")),
                                    )?;
                                let source = storage
                                    .store
                                    .get_source(stored.source)
                                    .map_err(|error| invalid(error.to_string()))?;
                                file.input.source =
                                    Arc::new(String::from_utf8(source).map_err(|_| {
                                        invalid(format!("stored {path} is not UTF-8"))
                                    })?);
                            }
                        }
                        #[cfg(feature = "code-rule-profile")]
                        drop(_storage_read_timer);
                        let storage_read_nanoseconds = storage_started.elapsed().as_nanos() as u64;
                        let _consumer = CrossFileScheduler::install(crossfile.clone());
                        let mut metrics = evaluate_file_with_pending(
                            &mut file,
                            context.code_rules,
                            grammars,
                            context.closed,
                            Some(sql),
                            &budget,
                            &mut restored,
                            Some(&mut prepared),
                        )?;
                        metrics.storage_read_nanoseconds = storage_read_nanoseconds;
                        Ok(metrics)
                    });
                    if deadline.is_none() {
                        let budget = MatchBudget::work(context.closed, file_work_budget());
                        deadline = Some(budget.deadline());
                    }
                    let (metrics, error) = match metrics_result {
                        Ok(metrics) => (metrics, None),
                        Err(error) => (Metrics::default(), Some(error)),
                    };
                    PendingSqlJob {
                        path,
                        file,
                        consumes,
                        crossfile,
                        metrics,
                        prepared,
                        restored,
                        deadline: deadline.expect("every job has a deadline"),
                        error,
                    }
                })
                .collect::<Vec<_>>()
        });

        // A terminal executes over every file in its batch. Group by language and candidate set,
        // so no file spends work on a terminal that its manifest rejected.
        let mut groups = BTreeMap::<(String, Vec<ClauseId>), Vec<usize>>::new();
        for (job_index, job) in pending.iter().enumerate() {
            if job.prepared.is_some() {
                groups
                    .entry((
                        job.file.input.language.clone(),
                        job.file.candidates.ast.clone(),
                    ))
                    .or_default()
                    .push(job_index);
            }
        }
        for batch_jobs in groups.into_values() {
            let batch_files = batch_jobs
                .iter()
                .map(|&job_index| {
                    let job = &pending[job_index];
                    let prepared = job.prepared.as_ref().expect("batched AST file");
                    let _budget = MatchBudget::resume(job.deadline.clone());
                    crate::code_rule_dsl::transducer::sql_backend::BatchFile::new(
                        prepared.doc.clone(),
                        Arc::from(job.file.input.path.as_str()),
                        Arc::from(job.file.input.language.as_str()),
                        None,
                    )
                    .bounded(prepared.max_nodes)
                })
                .collect();
            let outcome = sql.with_files(batch_files, |batch| {
                for (batch_index, &job_index) in batch_jobs.iter().enumerate() {
                    let job = &mut pending[job_index];
                    #[cfg(feature = "code-rule-profile")]
                    let _worker_profile = profile::WorkerProfile::start();
                    #[cfg(feature = "code-rule-profile")]
                    let _doc_owner = profile::DocOwner::start();
                    let _budget = MatchBudget::resume(job.deadline.clone());
                    let _consumer = CrossFileScheduler::install(job.crossfile.clone());
                    let prepared = job.prepared.take().expect("batched AST file");
                    match execute_prepared_ast_file(
                        &mut job.file,
                        context.code_rules,
                        &mut job.metrics,
                        prepared,
                        Some(sql),
                        Some((batch, batch_index)),
                    ) {
                        Ok(doc) => job.restored = Some(doc),
                        Err(error) => job.error = Some(error),
                    }
                    job.file.transient_failure = job.deadline.observed();
                }
            });
            if outcome.is_err() {
                for job_index in batch_jobs {
                    let job = &mut pending[job_index];
                    for &id in &job.file.candidates.ast {
                        job.file
                            .failures
                            .entry(id)
                            .or_insert(("adapter-error", "query engine unavailable"));
                    }
                }
            }
        }

        pending
            .into_iter()
            .map(|mut job| {
                #[cfg(feature = "code-rule-profile")]
                let _worker_profile = profile::WorkerProfile::start();
                if job.error.is_none() && context.summarize {
                    let transient = job.file.transient_failure && job.crossfile.is_some();
                    let _consumer = CrossFileScheduler::install(job.crossfile.clone());
                    if let Err(error) = CrossFileScheduler::summarize_file(
                        &mut job.file,
                        grammars,
                        context.closed,
                        &mut job.metrics,
                        job.consumes,
                        job.restored.take(),
                    ) {
                        job.error = Some(error);
                    }
                    job.file.transient_failure |= transient;
                }
                CrossFileScheduler::retain_scopes(&mut job.file, job.consumes);
                let result = job.error.map_or_else(|| Ok(job.metrics), Err);
                (job.path, job.file, result)
            })
            .collect::<Evaluated>()
    };

    if context.sql.is_some() {
        let mut grammars = BTreeMap::new();
        return Ok(sql_work(jobs, &mut grammars));
    }

    Ok(if jobs.len() < WORKER_THREADS {
        let mut grammars = BTreeMap::new();
        jobs.into_iter()
            .map(|job| work(job, &mut grammars))
            .collect()
    } else {
        WORK_POOL.install(|| {
            jobs.into_par_iter()
                .map_init(BTreeMap::new, |grammars, job| work(job, grammars))
                .collect()
        })
    })
}

/// The registered rules' SQLite plans, built once when that engine is selected.
pub(super) fn sql_plans(state: &mut WorkspaceState) -> Option<Arc<query::SqlPlans>> {
    if state.execution_engine != ExecutionEngine::Sqlite {
        return None;
    }
    let code_rules = &state.code_rules;
    Some(Arc::clone(state.sql_plans.get_or_insert_with(|| {
        Arc::new(query::SqlPlans::compile(code_rules))
    })))
}

/// Evaluates `file`'s candidates. `restored` receives the document its AST clauses were read on,
/// for the job's summary pass to read rather than restore the tape again.
pub(super) fn evaluate_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    sql: Option<&query::SqlPlans>,
    budget: &MatchBudget,
    restored: &mut Option<tape::TapeDoc>,
) -> Result<Metrics> {
    evaluate_file_with_pending(
        file, code_rules, grammars, closed, sql, budget, restored, None,
    )
}

/// Prepares a file and can defer its SQLite AST clauses for one worker-local batch.
#[allow(clippy::too_many_arguments)]
fn evaluate_file_with_pending(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    sql: Option<&query::SqlPlans>,
    budget: &MatchBudget,
    restored: &mut Option<tape::TapeDoc>,
    pending: Option<&mut Option<PreparedAst>>,
) -> Result<Metrics> {
    let mut metrics = Metrics {
        files: 1,
        avoided_tape_decompressions: usize::from(file.selector_skipped),
        tape_cache_hits: usize::from(file.tape_cache_hit),
        tape_decompressions: usize::from(file.tape_decompressed),
        restored_local_results: file.restored_local_results,
        ..Metrics::default()
    };
    for id in file.candidates.regex.iter().chain(&file.candidates.ast) {
        file.failures.remove(id);
    }
    // A parse below sets it again; a file whose AST results were all restored keeps theirs.
    if !file.candidates.ast.is_empty() {
        file.syntax = None;
    }
    let source_len = file.input.source.len();
    // Apply source caps before constructing an index or a parser.
    let eligible = |id: &ClauseId| {
        let limits = &code_rules[id.0].capabilities.limits;
        source_len <= limits.max_source_bytes && limits.max_files != 0
    };
    let needs_lines = (!file.selector_skipped && file.candidates.ast.iter().any(&eligible))
        || file.candidates.regex.iter().any(|id| {
            eligible(id)
                && matches!(
                    &code_rules[id.0].clauses[id.1].kind,
                    ClauseKind::Forbid(Query::Regex {
                        source: RegexSource::Content,
                        ..
                    })
                )
        });
    let line_index_started = Instant::now();
    let terminators = file_language(&file.input)
        .map(Terminators::of)
        .unwrap_or(Terminators::Eslint);
    let lines = needs_lines.then(|| Arc::new(LineTable::new(&file.input.source, terminators)));
    #[cfg(test)]
    if lines.is_some() {
        crate::code_rule_dsl::matcher::note_line_table_construction();
    }
    metrics.line_index_nanoseconds = line_index_started.elapsed().as_nanos() as u64;
    #[cfg(feature = "code-rule-profile")]
    let _line_owner = lines
        .as_ref()
        .map(|index| profile::LineOwner::start(index.estimated_bytes()));
    #[cfg(feature = "code-rule-profile")]
    let regex_timer = profile::PhaseTimer::start(profile::Phase::Regex);
    let regex_started = Instant::now();
    if sql.is_none() {
        for &id in &file.candidates.regex {
            let code_rule = &code_rules[id.0];
            let clause = &code_rule.clauses[id.1];
            let ClauseKind::Forbid(Query::Regex { regex, source }) = &clause.kind else {
                unreachable!()
            };
            let witnesses = file.witnesses.entry(id).or_default();
            witnesses.clear();
            let limits = &code_rule.capabilities.limits;
            if file.input.source.len() > limits.max_source_bytes || limits.max_files == 0 {
                file.failures
                    .insert(id, ("budget-exceeded", "file input budget exceeded"));
                continue;
            }
            if file.failures.contains_key(&id) {
                continue;
            }
            let haystack: &str = match source {
                RegexSource::Content => &file.input.source,
                RegexSource::Path => &file.input.path,
            };
            let path_lines = LineTable::new(&file.input.path, Terminators::Eslint);
            let span = |start, end| match source {
                RegexSource::Content => lines
                    .as_ref()
                    .expect("eligible content has line table")
                    .span(start, end),
                RegexSource::Path => path_lines.span(start, end),
            };
            let (matches, complete) =
                query::regex_matches(regex, haystack, limits.max_witnesses.saturating_add(1));
            let plain =
                regex.capture_names().flatten().next().is_none() && clause.violation.fix.is_none();
            let mut stopped = false;
            for range in matches {
                if witnesses.len() >= limits.max_witnesses {
                    file.failures
                        .insert(id, ("budget-exceeded", "file match budget exceeded"));
                    stopped = true;
                    break;
                }
                let found = regex
                    .captures_at(haystack, range.start)
                    .expect("a regex match has captures");
                assert_eq!(found.get(0).expect("whole match").range(), range);
                if plain {
                    witnesses.push(Witness {
                        path: file.input.path.clone(),
                        span: span(range.start, range.end),
                        captures: Captures::default(),
                        message: None,
                        fix: None,
                    });
                } else {
                    witnesses.push(violations::regex_witness(
                        &file.input.path,
                        regex,
                        &found,
                        Some(&clause.violation),
                        span,
                    ));
                }
                // A decision is answered by its first witness.
                if clause.decision {
                    stopped = true;
                    break;
                }
            }
            if !stopped && !complete {
                file.failures
                    .insert(id, ("budget-exceeded", "file match budget exceeded"));
            }
            metrics.witnesses += witnesses.len();
        }
    } else if let Some(sql) = sql {
        evaluate_regex_statements(file, code_rules, &lines, &mut metrics, sql)?;
    }
    #[cfg(feature = "code-rule-profile")]
    drop(regex_timer);
    metrics.regex_nanoseconds = regex_started.elapsed().as_nanos() as u64;
    // Input limits hold for every AST candidate, including those a selector-index miss answers.
    let mut any_eligible = false;
    for &id in &file.candidates.ast {
        file.witnesses.remove(&id);
        if !eligible(&id) {
            file.failures
                .insert(id, ("budget-exceeded", "file input budget exceeded"));
        } else if !file.failures.contains_key(&id) {
            file.witnesses.insert(id, Vec::new());
            any_eligible = true;
        }
    }
    if !file.candidates.ast.is_empty() && !file.selector_skipped {
        metrics.restored_tapes += usize::from(file.tape.is_some());
        if any_eligible
            && let Some(prepared) =
                prepare_ast_file(file, code_rules, grammars, closed, lines, &mut metrics)?
        {
            if let Some(pending) = pending.filter(|_| sql.is_some()) {
                *restored = Some(prepared.doc.clone());
                *pending = Some(prepared);
            } else {
                *restored = Some(execute_prepared_ast_file(
                    file,
                    code_rules,
                    &mut metrics,
                    prepared,
                    sql,
                    None,
                )?);
            }
        }
    }
    for witness in file.witnesses.values_mut().flatten() {
        witness.path.clone_from(&file.input.path);
    }
    // A deadline or cancellation can surface as any budget failure, or as a matcher that stopped
    // early. Such an evaluation is not the content's verdict: it is retried and never persisted.
    // Only exhaustion some clause observed counts; an evaluation that finished is kept, however
    // late it finished.
    file.transient_failure = budget.observed();
    Ok(metrics)
}

/// Runs every regex candidate through the restored SQLite text terminal.
fn evaluate_regex_statements(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    lines: &Option<Arc<LineTable>>,
    metrics: &mut Metrics,
    sql: &query::SqlPlans,
) -> Result<()> {
    let failed = sql
        .with_file(
            crate::code_rule_dsl::transducer::sql_backend::BatchFile::text_shared(
                Arc::clone(&file.input.source),
                Arc::from(file.input.path.as_str()),
                Arc::from(file.input.language.as_str()),
                None,
            ),
            |batch| {
                for &id in &file.candidates.regex {
                    let code_rule = &code_rules[id.0];
                    let clause = &code_rule.clauses[id.1];
                    let ClauseKind::Forbid(Query::Regex { regex, source }) = &clause.kind else {
                        unreachable!()
                    };
                    let witnesses = file.witnesses.entry(id).or_default();
                    witnesses.clear();
                    let limits = &code_rule.capabilities.limits;
                    if file.input.source.len() > limits.max_source_bytes || limits.max_files == 0 {
                        file.failures
                            .insert(id, ("budget-exceeded", "file input budget exceeded"));
                        continue;
                    }
                    if file.failures.contains_key(&id) {
                        continue;
                    }
                    let haystack: &str = match source {
                        RegexSource::Content => &file.input.source,
                        RegexSource::Path => &file.input.path,
                    };
                    let path_lines = LineTable::new(&file.input.path, Terminators::Eslint);
                    let span = |start, end| match source {
                        RegexSource::Content => lines
                            .as_ref()
                            .expect("eligible content has line table")
                            .span(start, end),
                        RegexSource::Path => path_lines.span(start, end),
                    };
                    let Some(terminal) = sql.regex_terminal(id.0, id.1) else {
                        file.failures
                            .insert(id, ("adapter-error", "query statement is unavailable"));
                        continue;
                    };
                    let rows = match batch.text_rows(terminal, 0) {
                        Ok(rows) => rows,
                        Err(_) => {
                            file.failures
                                .insert(id, ("adapter-error", "query statement failed"));
                            continue;
                        }
                    };
                    let plain = regex.capture_names().flatten().next().is_none()
                        && clause.violation.fix.is_none();
                    for (span_row, complete) in rows {
                        let Some((start, end)) = span_row else {
                            if !complete {
                                file.failures
                                    .insert(id, ("budget-exceeded", "file match budget exceeded"));
                            }
                            continue;
                        };
                        if witnesses.len() >= limits.max_witnesses {
                            file.failures
                                .insert(id, ("budget-exceeded", "file match budget exceeded"));
                            break;
                        }
                        let found = regex
                            .captures_at(haystack, start)
                            .expect("terminal match has captures");
                        assert_eq!(found.get(0).expect("whole match").range(), start..end);
                        if plain {
                            witnesses.push(Witness {
                                path: file.input.path.clone(),
                                span: span(start, end),
                                captures: Captures::default(),
                                message: None,
                                fix: None,
                            });
                        } else {
                            witnesses.push(violations::regex_witness(
                                &file.input.path,
                                regex,
                                &found,
                                Some(&clause.violation),
                                span,
                            ));
                        }
                        if clause.decision {
                            break;
                        }
                    }
                    metrics.witnesses += witnesses.len();
                }
            },
        )
        .is_err();
    if failed {
        for &id in &file.candidates.regex {
            file.failures
                .insert(id, ("adapter-error", "query engine unavailable"));
        }
    }
    Ok(())
}

/// The document the clauses were read on, `None` where none was restored.
fn prepare_ast_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    lines: Option<Arc<LineTable>>,
    metrics: &mut Metrics,
) -> Result<Option<PreparedAst>> {
    let language = file_language(&file.input).expect("AST candidate has a supported language");
    let lines = lines.expect("eligible AST file has a line index");
    let grammar = if let Some(grammar) = grammars.get(&file.input.language) {
        Arc::clone(grammar)
    } else {
        match tape::Grammar::new(language) {
            Ok(grammar) => {
                grammars.insert(file.input.language.clone(), Arc::clone(&grammar));
                grammar
            }
            Err(_) => {
                for &id in &file.candidates.ast {
                    file.failures
                        .entry(id)
                        .or_insert(("adapter-error", "file grammar cannot be taped"));
                }
                return Ok(None);
            }
        }
    };
    if file.tape.is_none()
        && let Err(failure) = encode_tape(file, language, &grammar, closed, metrics)?
    {
        for &id in &file.candidates.ast {
            file.failures.entry(id).or_insert(failure);
        }
        return Ok(None);
    }
    if file
        .tape
        .as_ref()
        .is_none_or(|(revision, _)| *revision != file.source_revision)
    {
        for &id in &file.candidates.ast {
            file.failures
                .entry(id)
                .or_insert(("adapter-error", "file tape revision mismatch"));
        }
        return Ok(None);
    }
    #[cfg(feature = "code-rule-profile")]
    let _decode_timer = profile::PhaseTimer::start(profile::Phase::TapeDecode);
    let doc = match tape::TapeDoc::restore_with_lines(
        Arc::clone(&file.input.source),
        grammar,
        &file.tape.as_ref().expect("encoded tape").1,
        Arc::clone(&lines),
        &matcher_budget_exhausted,
    ) {
        Ok(doc) => doc,
        Err(_) => {
            let code = if matcher_budget_observed() {
                "budget-exceeded"
            } else {
                "adapter-error"
            };
            for &id in &file.candidates.ast {
                file.failures
                    .entry(id)
                    .or_insert((code, "file tape restoration failed"));
            }
            return Ok(None);
        }
    };
    #[cfg(feature = "code-rule-profile")]
    {
        drop(_decode_timer);
        profile::record_transient(doc.navigation_bytes());
    }
    let errors = SyntaxErrors::from_ranges(doc.syntax_error_ranges());
    if !errors.is_empty() {
        for &id in &file.candidates.ast {
            if !file.failures.contains_key(&id)
                && !candidate_grammar(&code_rules[id.0], id.1, &file.input).sound_under_errors
            {
                file.failures
                    .insert(id, ("source-incomplete", "file has syntax errors"));
            }
        }
        file.syntax = Some(errors.partial_message(&lines));
    }
    #[cfg(feature = "code-rule-profile")]
    let _doc_owner = profile::DocOwner::start();
    let restored = doc.clone();
    let root = Root::doc(doc);
    #[cfg(test)]
    pause_at_test_phase("restored-doc", &file.input.source);
    let max_nodes = file
        .candidates
        .ast
        .iter()
        .filter(|id| !file.failures.contains_key(id))
        .map(|id| code_rules[id.0].capabilities.limits.max_visited_nodes)
        .max()
        .unwrap_or(0);
    {
        // Shadows the grammar cache, which is no longer needed once the document is restored.
        let grammars = file
            .candidates
            .ast
            .iter()
            .map(|id| candidate_grammar(&code_rules[id.0], id.1, &file.input))
            .collect::<Vec<_>>();
        // A clause whose matcher needs text the file lacks matches no root: it selects none. The
        // clauses share many literals, each searched for once.
        let mut searched = rustc_hash::FxHashMap::<&str, bool>::default();
        let mut found = |text: &str, literal| {
            *searched.entry(literal).or_insert_with(|| {
                memchr::memmem::find(text.as_bytes(), literal.as_bytes()).is_some()
            })
        };
        let runs = file
            .candidates
            .ast
            .iter()
            .zip(&grammars)
            .map(|(id, grammar)| {
                !file.failures.contains_key(id)
                    && grammar
                        .text
                        .as_ref()
                        .is_none_or(|text| text.holds_with(&file.input.source, &mut found))
            })
            .collect::<Vec<_>>();
        let plans = grammars
            .iter()
            .zip(&runs)
            .filter(|(_, runs)| **runs)
            .map(|(grammar, _)| grammar.anchors.as_ref())
            .collect::<Vec<_>>();
        #[cfg(feature = "code-rule-profile")]
        let index_timer = profile::PhaseTimer::start(profile::Phase::Index);
        let index = FileAnchorIndex::build(&root, &plans, max_nodes, matcher_budget_exhausted);
        metrics.visited_nodes += index.visited_nodes();
        let _ = index;
        #[cfg(feature = "code-rule-profile")]
        {
            drop(index_timer);
        }
        #[cfg(feature = "code-rule-profile")]
        let selection_timer = profile::PhaseTimer::start(profile::Phase::Selection);
        let selections = grammars
            .iter()
            .zip(&runs)
            .map(|(grammar, runs)| {
                if !runs {
                    return Selection::Roots(Cow::Borrowed(&[]));
                }
                index.select(&grammar.anchors)
            })
            .collect::<Vec<_>>();
        #[cfg(feature = "code-rule-profile")]
        {
            drop(selection_timer);
            profile::record_index(index.len(), index.estimated_bytes(), index.complete());
            for (slot, selection) in selections.iter().enumerate() {
                if !file.failures.contains_key(&file.candidates.ast[slot]) {
                    profile::record_selection(
                        match selection {
                            Selection::All => index.len(),
                            Selection::Roots(roots) => roots.len(),
                        },
                        matches!(selection, Selection::All),
                    );
                }
            }
        }
        let util_work = file
            .candidates
            .ast
            .iter()
            .map(|id| code_rules[id.0].capabilities.limits.max_visited_nodes)
            .collect::<Vec<_>>();
        let roots = selected_roots_by_slot(&selections, &index);
        Ok(Some(PreparedAst {
            doc: restored,
            errors,
            lines,
            roots,
            util_work,
            max_nodes,
        }))
    }
}

/// Runs every clause over a file that parsing and anchor selection prepared.
fn execute_prepared_ast_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    metrics: &mut Metrics,
    mut prepared: PreparedAst,
    sql: Option<&query::SqlPlans>,
    registered: Option<(&query::QueryBatch<'_>, usize)>,
) -> Result<tape::TapeDoc> {
    #[cfg(feature = "code-rule-profile")]
    let _dispatch_timer = profile::PhaseTimer::start(profile::Phase::Dispatch);
    let scopes = FileScopeSlot::install(
        file.scopes
            .as_ref()
            .filter(|(revision, _)| *revision == file.source_revision)
            .map(|(_, tables)| tables.clone())
            .unwrap_or_default(),
        file.input.module.clone(),
        !prepared.errors.is_empty(),
    )
    .path(&file.input.path);
    let match_started = Instant::now();
    let grammars = file
        .candidates
        .ast
        .iter()
        .map(|id| candidate_grammar(&code_rules[id.0], id.1, &file.input))
        .collect::<Vec<_>>();
    if let Some(sql) = sql {
        evaluate_statements(
            file,
            code_rules,
            sql,
            &grammars,
            &prepared.doc,
            &prepared.errors,
            &prepared.lines,
            metrics,
            &mut prepared.util_work,
            &prepared.roots,
            prepared.max_nodes,
            registered,
        )?;
    } else {
        evaluate_clauses(
            file,
            code_rules,
            &grammars,
            &prepared.doc,
            &prepared.errors,
            &prepared.lines,
            metrics,
            &mut prepared.util_work,
            &prepared.roots,
        )?;
    }
    let tables = scopes.tables();
    if !tables.is_empty() {
        file.scopes = Some((file.source_revision, tables));
    }
    metrics.ast_match_nanoseconds += match_started.elapsed().as_nanos() as u64;
    #[cfg(test)]
    pass_deadline_at_test_phase("checks-done");
    #[cfg(feature = "code-rule-profile")]
    profile::record_nodes(metrics.visited_nodes);
    for &id in &file.candidates.ast {
        let code_rule = &code_rules[id.0];
        if file.input.source.len() > code_rule.capabilities.limits.max_source_bytes
            || code_rule.capabilities.limits.max_files == 0
        {
            continue;
        }
        // Only exhaustion some step observed cut the work short; work that finished is complete.
        if matcher_budget_observed()
            || metrics.visited_nodes > code_rule.capabilities.limits.max_visited_nodes
        {
            file.failures
                .insert(id, ("budget-exceeded", "file AST work budget exceeded"));
        }
    }
    Ok(prepared.doc)
}

/// Parses `file`'s source and keeps its tape for this source revision. `Ok(Err(failure))` where the
/// parse ran out of time or the tape could not be encoded.
pub(super) fn encode_tape(
    file: &mut FileState,
    language: GrammarDialect,
    grammar: &Arc<tape::Grammar>,
    closed: &AtomicBool,
    metrics: &mut Metrics,
) -> Result<std::result::Result<(), LocalFailure>> {
    // Parser and the only native tree are scoped to this one source revision.
    // Neither can overlap with the restored document during selection.
    let interrupted = || closed.load(Ordering::Relaxed) || matcher_budget_exhausted();
    let image = if language == GrammarDialect::Blade {
        // Forte's step budget bounds the parse; closing the workspace or the matcher budget
        // interrupts the view and the encoding.
        #[cfg(feature = "code-rule-profile")]
        let _native_owner = profile::NativeOwner::start();
        #[cfg(feature = "code-rule-profile")]
        let parse_timer = profile::PhaseTimer::start(profile::Phase::Parse);
        let parse_started = Instant::now();
        let tree = parse_tree(file.input.source.as_bytes());
        #[cfg(feature = "code-rule-profile")]
        drop(parse_timer);
        metrics.ast_parse_nanoseconds += parse_started.elapsed().as_nanos() as u64;
        #[cfg(test)]
        pause_at_test_phase("native-tree", &file.input.source);
        #[cfg(feature = "code-rule-profile")]
        let _encode_timer = profile::PhaseTimer::start(profile::Phase::TapeEncode);
        Some(tape::TapeImage::encode_blade(
            &tree,
            grammar,
            &file.input.source,
            &interrupted,
        ))
    } else {
        #[cfg(feature = "code-rule-profile")]
        let _native_owner = profile::NativeOwner::start();
        let mut parser = Parser::new();
        parser
            .set_language(&language.get_ts_language())
            .map_err(|error| invalid(error.to_string()))?;
        let bytes = file.input.source.as_bytes();
        let mut input = |offset: usize, _: Point| &bytes[offset..];
        let mut progress = |_: &tree_sitter::ParseState| {
            if interrupted() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        #[cfg(feature = "code-rule-profile")]
        let parse_timer = profile::PhaseTimer::start(profile::Phase::Parse);
        let parse_started = Instant::now();
        let parsed = parser.parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        #[cfg(feature = "code-rule-profile")]
        drop(parse_timer);
        metrics.ast_parse_nanoseconds += parse_started.elapsed().as_nanos() as u64;
        match parsed {
            None => None,
            // Damaged trees are taped too: ERROR and MISSING nodes stay on the tape,
            // so their regions are rederived from each restored document.
            Some(tree) => {
                #[cfg(test)]
                pause_at_test_phase("native-tree", &file.input.source);
                #[cfg(feature = "code-rule-profile")]
                let _encode_timer = profile::PhaseTimer::start(profile::Phase::TapeEncode);
                let encoded = tape::TapeImage::encode(
                    tree.root_node(),
                    grammar,
                    &file.input.source,
                    &interrupted,
                );
                #[cfg(feature = "code-rule-profile")]
                drop(_encode_timer);
                drop(tree);
                Some(encoded)
            }
        }
    };
    Ok(match image {
        None => Err(("budget-exceeded", "file parsing time budget exceeded")),
        Some(Err(_)) => Err((
            if matcher_budget_observed() {
                "budget-exceeded"
            } else {
                "adapter-error"
            },
            "file tape encoding failed",
        )),
        Some(Ok(image)) => {
            file.tape = Some((file.source_revision, image));
            Ok(())
        }
    })
}

/// The roots each slot's anchor plan selected, as node ids (`None`: every root).
fn selected_roots_by_slot(
    selections: &[Selection<'_>],
    index: &FileAnchorIndex<'_, tape::TapeDoc>,
) -> Vec<Option<Arc<[u32]>>> {
    selections
        .iter()
        .map(|selection| match selection {
            Selection::All => None,
            Selection::Roots(roots) => Some(
                roots
                    .iter()
                    .map(|&ordinal| index.node(ordinal).node_id() as u32)
                    .collect(),
            ),
        })
        .collect()
}

/// Runs every candidate AST clause's SQLite statement over one restored document.
#[allow(clippy::too_many_arguments)]
fn evaluate_statements(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    plans: &query::SqlPlans,
    grammars: &[&GrammarMatcher],
    doc: &tape::TapeDoc,
    errors: &SyntaxErrors,
    lines: &Arc<LineTable>,
    metrics: &mut Metrics,
    util_work: &mut [usize],
    roots: &[Option<Arc<[u32]>>],
    node_limit: usize,
    registered: Option<(&query::QueryBatch<'_>, usize)>,
) -> Result<()> {
    let Some((batch, file_index)) = registered else {
        let batch_file = crate::code_rule_dsl::transducer::sql_backend::BatchFile::new(
            doc.clone(),
            Arc::from(file.input.path.as_str()),
            Arc::from(file.input.language.as_str()),
            None,
        )
        .bounded(node_limit);
        let outcome = plans.with_file(batch_file, |batch| {
            evaluate_statements(
                file,
                code_rules,
                plans,
                grammars,
                doc,
                errors,
                lines,
                metrics,
                util_work,
                roots,
                node_limit,
                Some((batch, 0)),
            )
        });
        return match outcome {
            Ok(result) => result,
            Err(_) => {
                for &id in &file.candidates.ast {
                    file.failures
                        .entry(id)
                        .or_insert(("adapter-error", "query engine unavailable"));
                }
                Ok(())
            }
        };
    };
    let view = batch.view();
    let target = view
        .file(file_index as i64)
        .expect("the batch's registered file");
    for slot in 0..file.candidates.ast.len() {
        let id = file.candidates.ast[slot];
        if file.failures.contains_key(&id) || matcher_budget_exhausted() {
            continue;
        }
        let code_rule = &code_rules[id.0];
        let grammar = grammars[slot];
        let Some(plan) = plans.plan(id.0, id.1, grammar.language) else {
            file.failures
                .insert(id, ("adapter-error", "query statement is unavailable"));
            continue;
        };
        let violation = &code_rule.clauses[id.1].violation;
        let decision = code_rule.clauses[id.1].decision;
        let each = violation.each.as_deref();
        let limit_failure = |error: rusqlite::Error| {
            let message = error.to_string();
            match [
                UtilLimit::Depth,
                UtilLimit::Work,
                UtilLimit::Scope,
                UtilLimit::Value,
            ]
            .into_iter()
            .find(|limit| message.contains(limit.message()))
            {
                Some(limit) => ("budget-exceeded", limit.message()),
                None => ("adapter-error", "query statement failed"),
            }
        };
        let limits = &code_rule.capabilities.limits;
        let result = {
            let _first = each.map(crate::code_rule_dsl::matcher::EachScope::start);
            batch.run_while(
                plan,
                file_index,
                roots[slot].clone(),
                grammar,
                &mut util_work[slot],
                |node, first, work| {
                    if node as usize + 1 > limits.max_visited_nodes {
                        return Ok(false);
                    }
                    metrics.ast_matcher_calls += 1;
                    let matches = match each {
                        Some(name) => {
                            let room = if violation.suppress.is_some() {
                                usize::MAX
                            } else {
                                (limits.max_witnesses + 1)
                                    .saturating_sub(file.witnesses.get(&id).map_or(0, Vec::len))
                            };
                            batch.enumerate(
                                plan, file_index, node, first, grammar, work, name, room,
                            )?
                        }
                        None => vec![first],
                    };
                    for env in matches {
                        let mut matched =
                            ast_grep_core::NodeMatch::new(target.node(node), env.to_meta(target));
                        if let Some(transform) = &grammar.transform {
                            crate::code_rule_dsl::matcher::begin_clause(grammar, *work);
                            if transform.apply(matched.get_env_mut()).is_err() {
                                *work = crate::code_rule_dsl::matcher::end_clause().0;
                                file.failures
                                    .insert(id, ("budget-exceeded", UtilLimit::Work.message()));
                                return Ok(false);
                            }
                            *work = crate::code_rule_dsl::matcher::end_clause().0;
                        }
                        if errors.overlaps(&matched.get_node().range()) {
                            continue;
                        }
                        let room = (limits.max_witnesses + 1)
                            .saturating_sub(file.witnesses.get(&id).map_or(0, Vec::len));
                        let found = if each.is_some() {
                            witnesses_from_match(
                                &file.input,
                                &matched,
                                grammar.language,
                                violation,
                                room,
                                matcher_budget_exhausted,
                                |start, end| lines.span(start, end),
                            )
                        } else {
                            witness_from_match(
                                &file.input,
                                &matched,
                                grammar.language,
                                Some(violation),
                                |start, end| lines.span(start, end),
                            )
                            .map(|witness| vec![witness])
                        };
                        let found = match found {
                            Ok(found) => found,
                            Err(WitnessError::TransformLimit) => {
                                file.failures
                                    .insert(id, ("budget-exceeded", UtilLimit::Work.message()));
                                return Ok(false);
                            }
                        };
                        for witness in found {
                            if violations::suppresses(Some(violation), matched.get_node(), &witness)
                            {
                                continue;
                            }
                            let witnesses = file.witnesses.get_mut(&id).expect("eligible clause");
                            if witnesses.len() >= limits.max_witnesses {
                                file.failures.insert(
                                    id,
                                    ("budget-exceeded", "file witness budget exceeded"),
                                );
                                return Ok(false);
                            }
                            witnesses.push(witness);
                            metrics.witnesses += 1;
                            if decision {
                                return Ok(false);
                            }
                        }
                    }
                    Ok(true)
                },
            )
        };
        match result {
            Ok(()) => {}
            Err(_) if matcher_budget_observed() => continue,
            Err(error) => {
                file.failures.insert(id, limit_failure(error));
            }
        }
    }
    Ok(())
}

/// Runs every candidate AST clause (`matcher::run_clause`) over one restored document and records
/// their witnesses: syntax-error regions and the witness budget apply to each match, roots past
/// `maxVisitedNodes` are not matched, and a decision stops at its first witness.
#[allow(clippy::too_many_arguments)]
fn evaluate_clauses(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &[&GrammarMatcher],
    doc: &tape::TapeDoc,
    errors: &SyntaxErrors,
    lines: &Arc<LineTable>,
    metrics: &mut Metrics,
    util_work: &mut [usize],
    roots: &[Option<Arc<[u32]>>],
) -> Result<()> {
    use crate::code_rule_dsl::matcher::{Stopped, enumerate, node_at, run_clause, with_file_memo};
    let target = ast_grep_core::AstGrep::doc(doc.clone());
    with_file_memo(&target, || -> Result<()> {
        for slot in 0..file.candidates.ast.len() {
            let id = file.candidates.ast[slot];
            // An anchor selection of no roots leaves nothing for the clause to match.
            if file.failures.contains_key(&id)
                || matcher_budget_exhausted()
                || roots[slot].as_deref().is_some_and(<[u32]>::is_empty)
            {
                continue;
            }
            let code_rule = &code_rules[id.0];
            let grammar = grammars[slot];
            let violation = &code_rule.clauses[id.1].violation;
            let decision = code_rule.clauses[id.1].decision;
            let each = violation.each.as_deref();
            let limits = &code_rule.capabilities.limits;
            let result = {
                let _first = each.map(crate::code_rule_dsl::matcher::EachScope::start);
                run_clause(
                    &target,
                    grammar,
                    roots[slot].as_deref(),
                    limits.max_visited_nodes,
                    &mut util_work[slot],
                    |node, first, work| {
                        metrics.ast_matcher_calls += 1;
                        let matches = match each {
                            Some(name) => {
                                // A directive does not spend a violation's output allowance, so the
                                // enumeration continues past suppressed bindings.
                                let room = if violation.suppress.is_some() {
                                    usize::MAX
                                } else {
                                    (limits.max_witnesses + 1)
                                        .saturating_sub(file.witnesses.get(&id).map_or(0, Vec::len))
                                };
                                enumerate(&target, grammar, node, first, work, name, room)?
                            }
                            None => vec![first],
                        };
                        for env in matches {
                            let mut matched =
                                ast_grep_core::NodeMatch::new(node_at(&target, node), env);
                            if let Some(transform) = &grammar.transform {
                                crate::code_rule_dsl::matcher::begin_clause(grammar, *work);
                                if transform.apply(matched.get_env_mut()).is_err() {
                                    *work = crate::code_rule_dsl::matcher::end_clause().0;
                                    file.failures
                                        .insert(id, ("budget-exceeded", UtilLimit::Work.message()));
                                    return Ok(false);
                                }
                                *work = crate::code_rule_dsl::matcher::end_clause().0;
                            }
                            if errors.overlaps(&matched.get_node().range()) {
                                continue;
                            }
                            let room = (limits.max_witnesses + 1)
                                .saturating_sub(file.witnesses.get(&id).map_or(0, Vec::len));
                            let found = if each.is_some() {
                                witnesses_from_match(
                                    &file.input,
                                    &matched,
                                    grammar.language,
                                    violation,
                                    room,
                                    matcher_budget_exhausted,
                                    |start, end| lines.span(start, end),
                                )
                            } else {
                                witness_from_match(
                                    &file.input,
                                    &matched,
                                    grammar.language,
                                    Some(violation),
                                    |start, end| lines.span(start, end),
                                )
                                .map(|witness| vec![witness])
                            };
                            let found = match found {
                                Ok(found) => found,
                                Err(WitnessError::TransformLimit) => {
                                    file.failures
                                        .insert(id, ("budget-exceeded", UtilLimit::Work.message()));
                                    return Ok(false);
                                }
                            };
                            for witness in found {
                                if violations::suppresses(
                                    Some(violation),
                                    matched.get_node(),
                                    &witness,
                                ) {
                                    continue;
                                }
                                let witnesses =
                                    file.witnesses.get_mut(&id).expect("eligible check");
                                if witnesses.len() >= limits.max_witnesses {
                                    file.failures.insert(
                                        id,
                                        ("budget-exceeded", "file witness budget exceeded"),
                                    );
                                    return Ok(false);
                                }
                                witnesses.push(witness);
                                metrics.witnesses += 1;
                                // A decision is answered by its first witness.
                                if decision {
                                    return Ok(false);
                                }
                            }
                        }
                        Ok(true)
                    },
                )
                .and_then(|result| result)
            };
            match result {
                Ok(()) => {}
                // The deadline or a cancellation ended the clause; the caller records it.
                Err(Stopped::Deadline) => continue,
                Err(Stopped::Limit(limit)) => {
                    file.failures
                        .insert(id, ("budget-exceeded", limit.message()));
                }
            }
        }
        Ok(())
    })
}
