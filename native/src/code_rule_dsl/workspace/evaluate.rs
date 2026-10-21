//! File evaluation: jobs run serially or on the worker pool, regex clauses, tape parsing, and AST
//! clause evaluation under the match budget.

use super::crossfile::{CrossFileConsumer, CrossFileScheduler};
use super::documents;
use super::persisted::{restore_persisted_file, result_context, result_keys};
use super::sql::{Pipeline, SQL_UNSUPPORTED, SQL_UNSUPPORTED_MESSAGE, SqlEngine};
use super::storage::StorageContext;
use super::{
    CLAUSE_WORK_BUDGET_EXCEEDED, ClauseId, FILE_CLAUSE_WORK_EXCEEDED, FileState, LocalFailure,
    REUSE_LOCAL_RESULTS, check_closed, clause_work_budget, clause_work_ceiling, file_work_budget,
};
#[cfg(test)]
use super::{TEST_STORAGE_PREFETCH_FAILURE, pass_deadline_at_test_phase, pause_at_test_phase};
use crate::code_rule_dsl::blade::forte::tree::parse_tree;
use crate::code_rule_dsl::budget::{
    MatchBudget, SuspendedBudget, matcher_budget_exhausted, matcher_budget_observed, with_allowance,
};
use crate::code_rule_dsl::eval::{
    WitnessError, document_matcher, file_language, grammar_matcher, witness_from_match,
    witnesses_from_match,
};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    FileAnchorIndex, FileGates, FileScopeSlot, GrammarMatcher, KindPostings, Selection, UtilLimit,
};
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledCodeRule, FILE_BUDGET_EXCEEDED, FileInput, INPUT_LIMIT, Metrics, Query,
    RegexSource, WITNESS_LIMIT,
};
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile;
use crate::code_rule_dsl::soundness::{SyntaxErrors, SyntaxReading, UNREAD};
use crate::code_rule_dsl::sql::{ClauseRun, FileContext, SqlMatch, timed};
use crate::code_rule_dsl::violations::{Captures, Witness};
use crate::code_rule_dsl::{query, tape, violations};
use crate::content_store::HashKey;
use crate::{WORK_POOL, WORKER_THREADS};
use ast_grep_core::AstGrep as Root;
use ast_grep_language::LanguageExt;
use napi::Result;
use rayon::prelude::*;
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
    pub(super) storage: Option<&'a StorageContext>,
    pub(super) closed: &'a Arc<AtomicBool>,
    /// Extract each Python file's call summary (a code rule holds `calls` terms).
    pub(super) summarize: bool,
    /// The SQL engine, where the workspace matches its AST clauses on it (`Engine::Sql`).
    pub(super) sql: Option<&'a SqlEngine>,
}

type Evaluated = Vec<(String, FileState, Result<Metrics>)>;

/// Restores what the store holds for each job and evaluates the rest: serially for a few files,
/// on the worker pool for more.
pub(super) fn run_jobs(jobs: &mut Vec<Job>, context: &JobContext<'_>) -> Result<Evaluated> {
    let domain = result_keys(context.code_rules)?.domain;
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
                .local_check_results_by_key(&keys, domain)
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
    // Restores what the store holds for a job's file; returns how long the read took.
    let read = |path: &str, file: &mut FileState, grammars: &mut Grammars| -> Result<u64> {
        let storage_started = Instant::now();
        #[cfg(feature = "code-rule-profile")]
        let _storage_read_timer = profile::PhaseTimer::start(profile::Phase::StoreRead);
        if let Some(storage) = context.storage {
            let source_hash = HashKey::from_sha256_digest(file.source_digest);
            let result_key = (source_hash, result_context(file));
            let persisted = result_manifest
                .get(&result_key)
                .map(|results| results.as_slice());
            if context.summarize {
                CrossFileScheduler::restore_summary(file, persisted.unwrap_or_default());
            }
            restore_persisted_file(
                &stored_documents,
                storage.store.as_ref(),
                file,
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
        Ok(storage_started.elapsed().as_nanos() as u64)
    };
    // Ends a job after its evaluation: its call summary, and its storage read's time.
    let finish = |file: &mut FileState,
                  grammars: &mut Grammars,
                  budget: &MatchBudget,
                  (transient, consumes): (bool, bool),
                  restored: Option<tape::TapeDoc>,
                  mut metrics: Metrics,
                  storage_read_nanoseconds: u64|
     -> Result<Metrics> {
        // A consumers' pass after a transient evaluation of the file stays transient.
        file.transient_failure |= transient;
        if context.summarize {
            CrossFileScheduler::summarize_file(
                file,
                grammars,
                context.closed,
                &mut metrics,
                consumes,
                restored,
            )?;
            // The summary's own work spends the file's budget too: a summary it cut short is
            // opaque, this workspace's alone.
            file.transient_failure |= budget.shared_spent();
        }
        metrics.storage_read_nanoseconds = storage_read_nanoseconds;
        Ok(metrics)
    };
    if let Some(sql) = context.sql {
        // Batches of up to the batch size. An apply with fewer batches than workers (an edit, an
        // index chunk) runs them one after another, each on every worker (`sql_batch`'s
        // `grouped`): splitting its files into more batches would run every clause's statements
        // once per batch for no fewer rows. More batches run side by side, one per worker.
        let size = sql.batch_size.min(jobs.len()).max(1);
        let mut jobs = jobs.into_iter().peekable();
        let mut batches = Vec::new();
        while jobs.peek().is_some() {
            batches.push(jobs.by_ref().take(size).collect::<Vec<_>>());
        }
        let evaluated: Vec<Evaluated> = if batches.len() < WORKER_THREADS {
            batches
                .into_iter()
                .map(|batch| sql_batch(batch, context, sql, true, &read, &finish))
                .collect()
        } else {
            WORK_POOL.install(|| {
                batches
                    .into_par_iter()
                    .map(|batch| sql_batch(batch, context, sql, false, &read, &finish))
                    .collect()
            })
        };
        return Ok(evaluated.into_iter().flatten().collect());
    }
    let work = |job: Job, grammars: &mut Grammars| {
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
            let storage_read_nanoseconds = read(&path, &mut file, grammars)?;
            // The shared steps' budget grows with the file; each clause has its own allowance.
            let budget = MatchBudget::work(closed, file_work_budget(file.input.source.len()));
            let transient = file.transient_failure && crossfile.is_some();
            // A consumers' pass, or a pass over the clauses the store did not answer, runs some of
            // the file's clauses only.
            let partial = crossfile.is_some() || !file.restored_result_ids.is_empty();
            #[cfg(test)]
            let consumer_budget = consumer_budget(crossfile.is_some());
            let _consumer = CrossFileScheduler::install(crossfile);
            let mut restored = None;
            let evaluated = evaluate_file(
                &mut file,
                context.code_rules,
                grammars,
                closed,
                partial,
                &budget,
                &mut restored,
            );
            #[cfg(test)]
            restore_consumer_budget(consumer_budget);
            finish(
                &mut file,
                grammars,
                &budget,
                (transient, consumes),
                restored,
                evaluated?,
                storage_read_nanoseconds,
            )
        });
        CrossFileScheduler::retain_scopes(&mut file, consumes);
        (path, file, work)
    };
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

type Grammars = BTreeMap<String, Arc<tape::Grammar>>;

/// A cross-file consumers' pass runs each clause under the consumer allowance tests set.
#[cfg(test)]
fn consumer_budget(consumer: bool) -> Option<Option<u64>> {
    consumer.then(|| {
        super::TEST_CLAUSE_WORK_BUDGET.replace(
            super::TEST_CONSUMER_CLAUSE_WORK_BUDGET
                .get()
                .or(super::TEST_CLAUSE_WORK_BUDGET.get()),
        )
    })
}

#[cfg(test)]
fn restore_consumer_budget(previous: Option<Option<u64>>) {
    if let Some(previous) = previous {
        super::TEST_CLAUSE_WORK_BUDGET.set(previous);
    }
}

/// A job of a SQL batch between its steps before the batch's matching and those after.
struct Started<'c> {
    budget: SuspendedBudget,
    transient: bool,
    storage_read_nanoseconds: u64,
    pass: FilePass,
    ast: Option<SqlFile<'c>>,
}

/// Evaluates a batch of jobs with their AST clauses matched on the SQL engine: each file's native
/// steps up to its clauses' matching (storage, regex clauses, input limits, the tape, its
/// documents' syntax errors, required text and anchor selection), one SQL run for the batch's AST
/// clauses, each document a clause reads a batch document of its own (`embed`), then each file's
/// native rendering of the matches and the rest of its steps. `grouped`: the batch runs alone, so
/// its files' native steps run on the worker pool, as native runs its files, and its clauses in
/// groups on it (`SqlEngine::run_grouped`); otherwise batches run side by side, each on one
/// worker.
fn sql_batch<'c>(
    batch: Vec<Job>,
    context: &JobContext<'c>,
    sql: &SqlEngine,
    grouped: bool,
    read: &(impl Fn(&str, &mut FileState, &mut Grammars) -> Result<u64> + Sync),
    finish: &(
         impl Fn(
        &mut FileState,
        &mut Grammars,
        &MatchBudget,
        (bool, bool),
        Option<tape::TapeDoc>,
        Metrics,
        u64,
    ) -> Result<Metrics>
         + Sync
     ),
) -> Evaluated {
    let closed = context.closed;
    #[cfg(feature = "code-rule-profile")]
    let _worker_profile = profile::WorkerProfile::start();
    let start = |job: Job, grammars: &mut Grammars| {
        let Job {
            path,
            mut file,
            consumes,
            crossfile,
        } = job;
        let mut pipeline = Pipeline::default();
        let start = check_closed(closed).and_then(|()| {
            let storage_read_nanoseconds = read(&path, &mut file, grammars)?;
            let budget = MatchBudget::work(closed, file_work_budget(file.input.source.len()));
            let transient = file.transient_failure && crossfile.is_some();
            let partial = crossfile.is_some() || !file.restored_result_ids.is_empty();
            let mut pass = FilePass::begin(&mut file, context.code_rules, partial);
            let mut ast = None;
            if pass.ast
                && let Some(restored) = timed(&mut pipeline.parse, || {
                    restore_ast_file(
                        &mut file,
                        context.code_rules,
                        grammars,
                        closed,
                        pass.lines.clone(),
                        &mut pass.metrics,
                    )
                })?
            {
                ast = Some(timed(&mut pipeline.plan, || {
                    SqlFile::plan(
                        &mut file,
                        context.code_rules,
                        restored,
                        &mut pass.metrics,
                        sql,
                    )
                }));
            }
            Ok(Started {
                budget: budget.suspend(),
                transient,
                storage_read_nanoseconds,
                pass,
                ast,
            })
        });
        ((path, file, consumes, crossfile, start), pipeline)
    };
    // A batch alone runs its files' native steps on the worker pool where native would: with as
    // many files as workers.
    let parallel = grouped && batch.len() >= WORKER_THREADS;
    let (mut started, pipelines): (Vec<_>, Vec<Pipeline>) = if parallel {
        WORK_POOL.install(|| {
            batch
                .into_par_iter()
                .map_init(BTreeMap::new, |grammars, job| start(job, grammars))
                .unzip()
        })
    } else {
        let mut grammars = BTreeMap::new();
        batch
            .into_iter()
            .map(|job| start(job, &mut grammars))
            .unzip()
    };
    let mut pipeline = Pipeline::default();
    for job in &pipelines {
        pipeline.add(job);
    }
    // One run for the batch: each statement over the documents whose clause it matches.
    let mut docs = Vec::new();
    let mut runs: Vec<ClauseRun> = Vec::new();
    let mut run_of = HashMap::new();
    // Per clause run on a document: its job, document, position in the document's slots, run and
    // position in the run.
    let mut wanted = Vec::new();
    for (job, (_, file, _, crossfile, start)) in started.iter_mut().enumerate() {
        let Ok(Started { ast: Some(ast), .. }) = start else {
            continue;
        };
        // The file's scopes as the native dispatch installs them (`DocumentRead::evaluate`): its
        // cached tables of this revision and the file (module, host path, manifest), one cell for
        // every document of the file; and the call-graph closure a consumer's evaluation installs
        // (`CrossFileScheduler::install`).
        let context = FileContext {
            input: Some(Arc::new(file.input.clone())),
            scopes: Arc::new(parking_lot::Mutex::new(
                file.scopes
                    .as_ref()
                    .filter(|(revision, _)| *revision == file.source_revision)
                    .map(|(_, tables)| tables.clone())
                    .unwrap_or_default(),
            )),
            calls: CrossFileScheduler::calls(crossfile.as_ref()),
            kinds: None,
        };
        for (document, planned) in ast.documents.iter_mut().enumerate() {
            if planned.runs.iter().all(Option::is_none) {
                continue;
            }
            let index = docs.len();
            // Each document's own kind postings: a document is one language.
            let context = FileContext {
                kinds: planned.kinds.take(),
                ..context.clone()
            };
            docs.push((planned.read.root.root().get_doc().clone(), context));
            for (position, run) in planned.runs.iter().enumerate() {
                let Some((statement, roots)) = run else {
                    continue;
                };
                let run = *run_of.entry(*statement).or_insert_with(|| {
                    runs.push(ClauseRun {
                        statement: *statement,
                        files: Vec::new(),
                    });
                    runs.len() - 1
                });
                wanted.push((job, document, position, run, runs[run].files.len()));
                runs[run].files.push((index, roots.clone()));
            }
        }
    }
    let found = if runs.is_empty() {
        Ok(Vec::new())
    } else if grouped {
        sql.run_grouped(docs, &runs)
    } else {
        sql.run(sql.worker_slot(), docs, &runs)
    };
    match found {
        Ok(mut found) => {
            for (job, document, position, run, at) in wanted {
                if let Ok(Started { ast: Some(ast), .. }) = &mut started[job].4 {
                    ast.documents[document].matches[position] = std::mem::take(&mut found[run][at]);
                }
            }
        }
        Err(error) => {
            let message = error.to_string();
            for (.., start) in &mut started {
                if matches!(start, Ok(Started { ast: Some(_), .. })) {
                    *start = Err(invalid(message.clone()));
                }
            }
        }
    }
    type StartedJob<'c> = (
        String,
        FileState,
        bool,
        Option<CrossFileConsumer>,
        Result<Started<'c>>,
    );
    let end = |(path, mut file, consumes, crossfile, start): StartedJob<'_>,
               grammars: &mut Grammars| {
        let mut pipeline = Pipeline::default();
        let work = start.and_then(|started| {
            let Started {
                budget,
                transient,
                storage_read_nanoseconds,
                mut pass,
                ast,
            } = started;
            let budget = budget.resume();
            #[cfg(test)]
            let consumer_budget = consumer_budget(crossfile.is_some());
            let _consumer = CrossFileScheduler::install(crossfile);
            let rendered = timed(&mut pipeline.render, || {
                ast.map(|ast| ast.render(&mut file, context.code_rules, &mut pass))
                    .transpose()
            });
            #[cfg(test)]
            restore_consumer_budget(consumer_budget);
            let restored = rendered?;
            let metrics = pass.end(&mut file, &budget);
            finish(
                &mut file,
                grammars,
                &budget,
                (transient, consumes),
                restored,
                metrics,
                storage_read_nanoseconds,
            )
        });
        CrossFileScheduler::retain_scopes(&mut file, consumes);
        ((path, file, work), pipeline)
    };
    let (finished, pipelines): (Evaluated, Vec<Pipeline>) = if parallel {
        WORK_POOL.install(|| {
            started
                .into_par_iter()
                .map_init(BTreeMap::new, |grammars, job| end(job, grammars))
                .unzip()
        })
    } else {
        let mut grammars = BTreeMap::new();
        started
            .into_iter()
            .map(|job| end(job, &mut grammars))
            .unzip()
    };
    for job in &pipelines {
        pipeline.add(job);
    }
    sql.add_pipeline(pipeline);
    finished
}

/// Evaluates `file`'s candidates. `restored` receives the document its AST clauses were read on,
/// for the job's summary pass to read rather than restore the tape again. `partial`: the pass runs
/// some of the file's clauses only (`FileState::unstored`).
pub(super) fn evaluate_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    partial: bool,
    budget: &MatchBudget,
    restored: &mut Option<tape::TapeDoc>,
) -> Result<Metrics> {
    let mut pass = FilePass::begin(file, code_rules, partial);
    if pass.ast {
        *restored = evaluate_ast_file(
            file,
            code_rules,
            grammars,
            closed,
            pass.lines.clone(),
            &mut pass.work,
            &mut pass.metrics,
        )?;
    }
    Ok(pass.end(file, budget))
}

/// One evaluation of a file: its regex clauses and input limits (`begin`), then its AST clauses
/// where `ast`, then what holds for all of its clauses (`end`).
struct FilePass {
    metrics: Metrics,
    /// Whether running out of work may not be the content's verdict (`end`).
    history: bool,
    work: ClauseWork,
    lines: Option<Arc<LineTable>>,
    /// Whether its AST clauses are matched: some is eligible, and the selector index did not
    /// answer the file.
    ast: bool,
    #[cfg(feature = "code-rule-profile")]
    _line_owner: Option<profile::LineOwner>,
}

impl FilePass {
    /// The file's regex clauses, and its AST clauses' input limits. `partial`: the pass runs some
    /// of the file's clauses only (`FileState::unstored`).
    fn begin(file: &mut FileState, code_rules: &[Arc<CompiledCodeRule>], partial: bool) -> Self {
        let evaluated_clauses = file.candidates.regex.len() + file.candidates.ast.len();
        let mut metrics = Metrics {
            files: 1,
            evaluated_files: usize::from(evaluated_clauses > 0),
            evaluated_clauses,
            avoided_tape_decompressions: usize::from(file.selector_skipped),
            tape_cache_hits: usize::from(file.tape_cache_hit),
            tape_decompressions: usize::from(file.tape_decompressed),
            restored_local_results: file.restored_local_results,
            ..Metrics::default()
        };
        for id in file.candidates.regex.iter().chain(&file.candidates.ast) {
            file.failures.remove(id);
            file.unstored.remove(id);
        }
        // The work a clause spends depends on the clauses run before it in the same pass (the scope
        // tables, names and verdicts they leave built), and on the tables an earlier pass left. Over
        // all of the file's clauses, from nothing built, that is fixed by the content and the catalog;
        // in any other pass, running out of work is not the content's verdict.
        let history = partial
            || file.scopes.as_ref().is_some_and(|(revision, tables)| {
                *revision == file.source_revision && !tables.is_empty()
            });
        let mut work = ClauseWork::new(file.input.source.len());
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
                    .insert(id, (INPUT_LIMIT, "file input budget exceeded"));
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
            // Each clause scans under its own allowance, so one clause's matches spend none of
            // another's; the file's clause-work ceiling bounds them together.
            let Some(((matches, complete), ran)) = work.run(haystack.len(), || {
                query::regex_matches(regex, haystack, limits.max_witnesses.saturating_add(1))
            }) else {
                file.failures
                    .insert(id, (FILE_BUDGET_EXCEEDED, FILE_CLAUSE_WORK_EXCEEDED));
                file.unstored.insert(id);
                continue;
            };
            let plain =
                regex.capture_names().flatten().next().is_none() && clause.violation.fix.is_none();
            let mut stopped = false;
            for range in matches {
                if witnesses.len() >= limits.max_witnesses {
                    // An output cap, not a work limit: the clause found more than it may report.
                    file.failures
                        .insert(id, (WITNESS_LIMIT, "file match budget exceeded"));
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
                        ignored: false,
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
                if ran == Ran::Ceiling {
                    file.failures
                        .insert(id, (FILE_BUDGET_EXCEEDED, FILE_CLAUSE_WORK_EXCEEDED));
                    file.unstored.insert(id);
                } else {
                    file.failures
                        .insert(id, ("budget-exceeded", "file match budget exceeded"));
                }
            }
            metrics.witnesses += witnesses.len();
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
                    .insert(id, (INPUT_LIMIT, "file input budget exceeded"));
            } else if !file.failures.contains_key(&id) {
                file.witnesses.insert(id, Vec::new());
                any_eligible = true;
            }
        }
        let read = !file.candidates.ast.is_empty() && !file.selector_skipped;
        if read {
            metrics.restored_tapes += usize::from(file.tape.is_some());
        }
        Self {
            metrics,
            history,
            work,
            lines,
            ast: read && any_eligible,
            #[cfg(feature = "code-rule-profile")]
            _line_owner,
        }
    }

    /// What holds for all of the file's clauses once they ran: the witnesses' path, whether the
    /// evaluation is transient, and which budget failures are not stored.
    fn end(self, file: &mut FileState, budget: &MatchBudget) -> Metrics {
        for witness in file.witnesses.values_mut().flatten() {
            witness.path.clone_from(&file.input.path);
        }
        // The workspace closing or the inline deadline passing can surface as any budget failure, or
        // as a matcher that stopped early: such an evaluation is not the content's verdict, so it is
        // retried and never persisted. Neither is one whose shared work (parse, tape, anchor index)
        // ran out: how much of it an evaluation does depends on what an earlier one stored. A clause
        // running out of its own allowance is the verdict (where `history` allows), and an evaluation
        // that finished is kept, however late it finished.
        file.transient_failure = budget.interrupted() || budget.shared_spent();
        if self.history {
            let candidates = Arc::clone(&file.candidates);
            for id in candidates.regex.iter().chain(&candidates.ast) {
                if file.failures.get(id).is_some_and(|(code, _)| {
                    *code == "budget-exceeded" || *code == FILE_BUDGET_EXCEEDED
                }) {
                    file.unstored.insert(*id);
                }
            }
        }
        self.metrics
    }
}

/// A file's AST clauses placed on its documents (`embed`): its own tree, then each program it
/// embeds. Each clause reads one of them (`ast.embedded`), with that document's syntax errors,
/// anchors, scope tables and coordinates.
struct AstFile<'c> {
    /// The file's own document, for the job's summary pass.
    restored: tape::TapeDoc,
    lines: Arc<LineTable>,
    /// Each candidate slot's document (by its index in `documents`) and matcher; `None` where the
    /// file holds no program the clause reads.
    placed: Vec<Option<(usize, &'c GrammarMatcher)>>,
    /// Every document's errors, which a clause crossing between the documents (`embedded`,
    /// `host`) reads around; gathered only where some clause crosses.
    file_errors: SyntaxErrors,
    /// Which clauses ran to an answer (or failed on their own); the rest are failed once the
    /// documents ran (`settle`). A clause over a program the file does not embed has nothing to
    /// read: it answers.
    settled: Vec<bool>,
    #[cfg(feature = "code-rule-profile")]
    _doc_owner: profile::DocOwner,
}

impl<'c> AstFile<'c> {
    /// The file's documents: its own tree, then each program it embeds.
    fn documents(&self) -> Vec<tape::TapeDoc> {
        let mut documents = vec![self.restored.clone()];
        if let Some(guests) = self.restored.guests() {
            documents.extend(guests.iter().map(|guest| guest.doc.clone()));
        }
        documents
    }

    /// The candidate slots that read document `index`, each with its matcher.
    fn slots(&self, index: usize) -> Vec<(usize, &'c GrammarMatcher)> {
        self.placed
            .iter()
            .enumerate()
            .filter_map(|(slot, place)| {
                place
                    .filter(|(at, _)| *at == index)
                    .map(|(_, grammar)| (slot, grammar))
            })
            .collect()
    }

    /// Reads `document`, the file's document `index`, for the clauses that read it; `None` where
    /// none does, its syntax errors still the file's (it was read around them).
    fn read(
        &self,
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        index: usize,
        document: tape::TapeDoc,
    ) -> Option<DocumentRead<'c>> {
        let slots = self.slots(index);
        if slots.is_empty() {
            let errors = SyntaxErrors::of_doc(&document);
            if !errors.is_empty() {
                let (SyntaxReading::Partial(message) | SyntaxReading::Unread(message)) =
                    errors.reading(&self.lines, file.input.source.len());
                file.syntax.get_or_insert(message);
            }
            return None;
        }
        Some(DocumentRead::new(
            file,
            code_rules,
            document,
            slots,
            &self.file_errors,
            &self.lines,
        ))
    }

    /// Fails the candidates that did not settle (`settle_ast_file`); returns the file's own
    /// document.
    fn settle(
        self,
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        metrics: &Metrics,
    ) -> tape::TapeDoc {
        let nodes = self.restored.transducer_len();
        settle_ast_file(file, code_rules, nodes, &self.settled, metrics);
        self.restored
    }
}

/// Restores the file's documents for its AST clauses: from its tape (encoded first where it has
/// none), each candidate placed on the document it reads. `None`, with every candidate failed,
/// where they cannot be.
fn restore_ast_file<'c>(
    file: &mut FileState,
    code_rules: &'c [Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    lines: Option<Arc<LineTable>>,
    metrics: &mut Metrics,
) -> Result<Option<AstFile<'c>>> {
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
                FILE_BUDGET_EXCEEDED
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
    #[cfg(feature = "code-rule-profile")]
    let _doc_owner = profile::DocOwner::start();
    let mut documents = vec![doc.clone()];
    if let Some(guests) = doc.guests() {
        documents.extend(guests.iter().map(|guest| guest.doc.clone()));
    }
    let placed = file
        .candidates
        .ast
        .iter()
        .map(|id| {
            let matchers = clause_matchers(&code_rules[id.0], id.1);
            documents.iter().enumerate().find_map(|(index, document)| {
                document_matcher(matchers, &file.input, document).map(|grammar| (index, grammar))
            })
        })
        .collect::<Vec<_>>();
    let settled = placed.iter().map(Option::is_none).collect();
    let file_errors = if placed.iter().flatten().any(|(_, grammar)| grammar.crosses) {
        SyntaxErrors::of_file(&doc)
    } else {
        SyntaxErrors::from_ranges(Vec::new())
    };
    Ok(Some(AstFile {
        restored: doc,
        lines,
        placed,
        file_errors,
        settled,
        #[cfg(feature = "code-rule-profile")]
        _doc_owner,
    }))
}

/// One document of a file as the clauses that read it (`slots`: slot, matcher) read it: its syntax
/// errors (as ranges of the file), and which of the clauses run.
struct DocumentRead<'c> {
    root: Root<tape::TapeDoc>,
    errors: SyntaxErrors,
    slots: Vec<(usize, &'c GrammarMatcher)>,
    /// By position in `slots`: whether the clause runs. It failed before dispatch otherwise, or
    /// its matcher needs text the document lacks, so it matches no root and selects none.
    runs: Vec<bool>,
    /// The document's nodes; its node and utility visits grow with it (`Limits::visits_for`).
    nodes: usize,
    /// The most nodes a clause that runs may visit.
    max_nodes: usize,
}

impl<'c> DocumentRead<'c> {
    /// Reads `doc` for the clauses `slots`: its syntax errors decide which of them run around
    /// them (a clause crossing to the file's other documents reads around `file_errors`), and
    /// set the file's `syntax`.
    fn new(
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        doc: tape::TapeDoc,
        slots: Vec<(usize, &'c GrammarMatcher)>,
        file_errors: &SyntaxErrors,
        lines: &Arc<LineTable>,
    ) -> Self {
        let errors = SyntaxErrors::of_doc(&doc);
        if !errors.is_empty() {
            match errors.reading(lines, file.input.source.len()) {
                // No AST clause runs on a document the parser did not read, sound or not; the
                // failure reports `file.syntax` in place of its static message
                // (`results::failure_message`).
                SyntaxReading::Unread(message) => {
                    for &(slot, _) in &slots {
                        file.failures
                            .entry(file.candidates.ast[slot])
                            .or_insert(("source-incomplete", UNREAD));
                    }
                    file.syntax.get_or_insert(message);
                }
                SyntaxReading::Partial(message) => {
                    file.syntax.get_or_insert(message);
                }
            }
        }
        let root = Root::doc(doc);
        // The gates' file facts are read once for the document, whichever candidate reads them.
        let gates = FileGates::new(&file.input, root.root());
        for &(slot, grammar) in &slots {
            let id = file.candidates.ast[slot];
            // A clause reads around its document's errors, or, crossing to the file's other
            // document, every document's. A file the query's file gates exclude matches nothing,
            // errors or not.
            let read = if grammar.crosses {
                file_errors
            } else {
                &errors
            };
            if !file.failures.contains_key(&id) && read.incomplete(grammar) && gates.admit(grammar)
            {
                file.failures
                    .insert(id, ("source-incomplete", "file has syntax errors"));
            }
        }
        #[cfg(test)]
        pause_at_test_phase("restored-doc", &file.input.source);
        let nodes = root.root().get_doc().transducer_len();
        let max_nodes = slots
            .iter()
            .map(|&(slot, _)| file.candidates.ast[slot])
            .filter(|id| !file.failures.contains_key(id))
            .map(|id| code_rules[id.0].capabilities.limits.visits_for(nodes))
            .max()
            .unwrap_or(0);
        // The clauses share many literals, each searched for once.
        let text = ast_grep_core::Doc::get_source(root.root().get_doc()).as_str();
        let mut searched = rustc_hash::FxHashMap::<&str, bool>::default();
        let mut found = |text: &str, literal| {
            *searched.entry(literal).or_insert_with(|| {
                memchr::memmem::find(text.as_bytes(), literal.as_bytes()).is_some()
            })
        };
        let runs = slots
            .iter()
            .map(|&(slot, grammar)| {
                !file.failures.contains_key(&file.candidates.ast[slot])
                    && grammar
                        .text
                        .as_ref()
                        .is_none_or(|required| required.holds_with(text, &mut found))
            })
            .collect();
        Self {
            root,
            errors,
            slots,
            runs,
            nodes,
            max_nodes,
        }
    }

    /// The anchor index over the plans of the clauses that run, its visits counted in `metrics`.
    fn anchor_index(&self, metrics: &mut Metrics) -> FileAnchorIndex<'_, tape::TapeDoc> {
        let plans = self
            .slots
            .iter()
            .zip(&self.runs)
            .filter(|(_, runs)| **runs)
            .map(|((_, grammar), _)| grammar.anchors.as_ref())
            .collect::<Vec<_>>();
        #[cfg(feature = "code-rule-profile")]
        let index_timer = profile::PhaseTimer::start(profile::Phase::Index);
        let index =
            FileAnchorIndex::build(&self.root, &plans, self.max_nodes, matcher_budget_exhausted);
        metrics.visited_nodes += index.visited_nodes();
        #[cfg(feature = "code-rule-profile")]
        drop(index_timer);
        #[cfg(feature = "code-rule-profile")]
        profile::record_index(index.len(), index.estimated_bytes(), index.complete());
        index
    }

    /// Runs the clauses on the document (`evaluate_clauses`), each selecting its roots
    /// (`select`, by position), or rendering the SQL engine's matches (`sql`, by position) in
    /// place of the matcher's; marks in `settled` the slots that ran to an answer. The
    /// document's scope tables join the file's, which hold every document's.
    #[allow(clippy::too_many_arguments)]
    fn evaluate(
        &self,
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        file_errors: &SyntaxErrors,
        metrics: &mut Metrics,
        work: &mut ClauseWork,
        settled: &mut [bool],
        select: impl Fn(usize) -> Option<Arc<[u32]>>,
        sql: Option<&[Vec<SqlMatch>]>,
    ) -> Result<()> {
        let mut util_work = file
            .candidates
            .ast
            .iter()
            .map(|id| code_rules[id.0].capabilities.limits.visits_for(self.nodes))
            .collect::<Vec<_>>();
        #[cfg(feature = "code-rule-profile")]
        let _dispatch_timer = profile::PhaseTimer::start(profile::Phase::Dispatch);
        let scopes = FileScopeSlot::install(
            file.scopes
                .as_ref()
                .filter(|(revision, _)| *revision == file.source_revision)
                .map(|(_, tables)| tables.clone())
                .unwrap_or_default(),
            file.input.module.clone(),
            self.errors.hide_bindings(),
        )
        .file(&file.input);
        let match_started = Instant::now();
        let ran = evaluate_clauses(
            file,
            code_rules,
            &self.slots,
            self.root.root().get_doc(),
            (&self.errors, file_errors),
            metrics,
            &mut util_work,
            work,
            &self.runs,
            select,
            sql,
        )?;
        for (&(slot, _), ran) in self.slots.iter().zip(ran) {
            settled[slot] = ran;
        }
        // The file keeps the tables every document's evaluation built.
        let tables = scopes.tables();
        if !tables.is_empty() {
            match &mut file.scopes {
                Some((revision, known)) if *revision == file.source_revision => {
                    for table in tables {
                        if !known.iter().any(|known| {
                            known.language == table.language && known.reading == table.reading
                        }) {
                            known.push(table);
                        }
                    }
                }
                scopes => *scopes = Some((file.source_revision, tables)),
            }
        }
        metrics.ast_match_nanoseconds += match_started.elapsed().as_nanos() as u64;
        #[cfg(test)]
        pass_deadline_at_test_phase("checks-done");
        Ok(())
    }
}

/// The roots an anchor index selects for a clause: node ids, `None` for every root.
fn select_roots(
    index: &FileAnchorIndex<'_, tape::TapeDoc>,
    grammar: &GrammarMatcher,
) -> Option<Arc<[u32]>> {
    #[cfg(feature = "code-rule-profile")]
    let _selection_timer = profile::PhaseTimer::start(profile::Phase::Selection);
    let selection = index.select(&grammar.anchors);
    #[cfg(feature = "code-rule-profile")]
    profile::record_selection(
        match &selection {
            Selection::All => index.len(),
            Selection::Roots(roots) => roots.len(),
        },
        matches!(selection, Selection::All),
    );
    match selection {
        Selection::All => None,
        Selection::Roots(roots) => Some(
            roots
                .iter()
                .map(|&ordinal| index.node(ordinal).node_id() as u32)
                .collect(),
        ),
    }
}

/// Fails the candidates that did not settle: the workspace closed, or the file's budget ran out,
/// before they ran; and every candidate where the file's AST work ran over.
fn settle_ast_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    nodes: usize,
    settled: &[bool],
    metrics: &Metrics,
) {
    #[cfg(feature = "code-rule-profile")]
    profile::record_nodes(metrics.visited_nodes);
    for (slot, &id) in file.candidates.ast.iter().enumerate() {
        let code_rule = &code_rules[id.0];
        if file.input.source.len() > code_rule.capabilities.limits.max_source_bytes
            || code_rule.capabilities.limits.max_files == 0
        {
            continue;
        }
        if metrics.visited_nodes > code_rule.capabilities.limits.visits_for(nodes) {
            file.failures
                .insert(id, ("budget-exceeded", "file AST work budget exceeded"));
        } else if !settled[slot] && !file.failures.contains_key(&id) {
            // The workspace closed, or the file's own budget ran out, before the clause ran: what
            // the clauses before it spent decided that, so the result is not the clause's own. (A
            // clause that failed before dispatch, for the file's syntax, keeps its own failure.)
            file.failures
                .insert(id, (FILE_BUDGET_EXCEEDED, "file AST work budget exceeded"));
            file.unstored.insert(id);
        }
    }
}

/// The nodes `DocumentRead::anchor_index`'s walk would visit over `read`'s document
/// (`FileAnchorIndex::build`): every node, up to one past the most a clause may visit; the first
/// alone where the matcher budget is spent.
fn unbuilt_index_visits(read: &DocumentRead<'_>) -> usize {
    if matcher_budget_exhausted() {
        1
    } else {
        read.nodes.min(read.max_nodes.saturating_add(1))
    }
}

/// The native dispatch's per-file preparation (`clause_grammars`, `anchor_index` and each running
/// clause's `select_roots` in `evaluate_ast_file`), summed over the workers: the native
/// counterpart of the SQL pipeline's `plan` (`SqlFile::plan`), read by
/// `sql::tests::per_file_plan_against_native`.
#[cfg(test)]
pub(super) static NATIVE_PLAN: parking_lot::Mutex<crate::code_rule_dsl::sql::Timing> =
    parking_lot::const_mutex(crate::code_rule_dsl::sql::Timing::ZERO);

#[cfg(test)]
fn add_native_plan(started: crate::code_rule_dsl::sql::Started) {
    NATIVE_PLAN.lock().add(started.elapsed());
}

/// The document the clauses were read on, `None` where none was restored.
fn evaluate_ast_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    lines: Option<Arc<LineTable>>,
    work: &mut ClauseWork,
    metrics: &mut Metrics,
) -> Result<Option<tape::TapeDoc>> {
    let Some(mut ast) = restore_ast_file(file, code_rules, grammars, closed, lines, metrics)?
    else {
        return Ok(None);
    };
    for (index, document) in ast.documents().into_iter().enumerate() {
        #[cfg(test)]
        let prepared = crate::code_rule_dsl::sql::Started::now();
        let Some(read) = ast.read(file, code_rules, index, document) else {
            continue;
        };
        let anchors = read.anchor_index(metrics);
        #[cfg(test)]
        add_native_plan(prepared);
        // A clause's anchor selection is its own work: it runs under the clause's allowance.
        let select = |position: usize| -> Option<Arc<[u32]>> {
            if !read.runs[position] {
                return Some(Arc::from([]));
            }
            #[cfg(test)]
            let prepared = crate::code_rule_dsl::sql::Started::now();
            let roots = select_roots(&anchors, read.slots[position].1);
            #[cfg(test)]
            add_native_plan(prepared);
            roots
        };
        read.evaluate(
            file,
            code_rules,
            &ast.file_errors,
            metrics,
            work,
            &mut ast.settled,
            select,
            None,
        )?;
    }
    Ok(Some(ast.settle(file, code_rules, metrics)))
}

/// The matchers of an AST clause, one per dialect it compiled for.
fn clause_matchers(code_rule: &CompiledCodeRule, clause: usize) -> &[GrammarMatcher] {
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &code_rule.clauses[clause].kind else {
        unreachable!("an AST candidate is an AST clause")
    };
    matchers
}

/// A clause's statement in the SQL program and its selected roots (`None`: every root).
type PlannedRun = (usize, Option<Arc<[u32]>>);

/// One document of a file on the SQL engine: as its clauses read it, the statement and roots of
/// each that the engine matches (by position in its slots), its anchor index's kind postings for
/// the batch's `node` table, and, after the batch, their matches.
struct SqlDocument<'c> {
    read: DocumentRead<'c>,
    runs: Vec<Option<PlannedRun>>,
    kinds: Option<Arc<KindPostings>>,
    matches: Vec<Vec<SqlMatch>>,
}

/// A file's AST clauses on the SQL engine: the file, and each of its documents some clause reads,
/// in order.
struct SqlFile<'c> {
    ast: AstFile<'c>,
    documents: Vec<SqlDocument<'c>>,
}

impl<'c> SqlFile<'c> {
    /// What the batch matches for the file: per document a clause reads, its anchor index and
    /// each running clause's selected roots, as the native dispatch computes them
    /// (`evaluate_ast_file`). A clause the engine refuses fails where it would run.
    fn plan(
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        ast: AstFile<'c>,
        metrics: &mut Metrics,
        sql: &SqlEngine,
    ) -> Self {
        let mut documents = Vec::new();
        for (index, document) in ast.documents().into_iter().enumerate() {
            let Some(read) = ast.read(file, code_rules, index, document) else {
                continue;
            };
            if !read.runs.contains(&true) {
                // No clause runs on the document (each lacks the text its matcher requires, or
                // failed before): nothing reads an anchor index, so none is built. Its walk's
                // visits count for the file's AST work budget (`settle_ast_file`) as the native
                // dispatch's do.
                metrics.visited_nodes += unbuilt_index_visits(&read);
                let runs = vec![None; read.slots.len()];
                let matches = runs.iter().map(|_| Vec::new()).collect();
                documents.push(SqlDocument {
                    read,
                    runs,
                    kinds: None,
                    matches,
                });
                continue;
            }
            let mut runs = Vec::with_capacity(read.slots.len());
            let kinds = {
                let anchors = read.anchor_index(metrics);
                // A clause the file gates exclude (its `when.sourceType`, its `file.jsxRuntime`,
                // read once the document is: `run_clause`) matches nothing.
                let admitted = {
                    let gates = FileGates::new(&file.input, read.root.root());
                    read.slots
                        .iter()
                        .zip(&read.runs)
                        .map(|(&(_, grammar), &runs)| runs && gates.admit(grammar))
                        .collect::<Vec<_>>()
                };
                for (&(slot, grammar), &admitted) in read.slots.iter().zip(&admitted) {
                    if !admitted {
                        runs.push(None);
                        continue;
                    }
                    let id = file.candidates.ast[slot];
                    let Some(statement) = sql.statement(code_rules, id, grammar) else {
                        file.failures
                            .insert(id, (SQL_UNSUPPORTED, SQL_UNSUPPORTED_MESSAGE));
                        file.unstored.insert(id);
                        runs.push(None);
                        continue;
                    };
                    let roots = select_roots(&anchors, grammar);
                    // An anchor selection of no roots leaves nothing for the clause to match.
                    if roots.as_deref().is_some_and(<[u32]>::is_empty) {
                        runs.push(None);
                    } else {
                        runs.push(Some((statement, roots)));
                    }
                }
                // The batch reads the document's nodes by kind only where one of its clauses
                // runs.
                runs.iter()
                    .any(Option::is_some)
                    .then(|| Arc::new(anchors.into_kind_postings()))
            };
            let matches = runs.iter().map(|_| Vec::new()).collect();
            documents.push(SqlDocument {
                read,
                runs,
                kinds,
                matches,
            });
        }
        Self { ast, documents }
    }

    /// Renders the batch's matches for the file as the native dispatch renders the matcher's
    /// (`DocumentRead::evaluate`), and settles its clauses. Returns the file's own document.
    fn render(
        self,
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        pass: &mut FilePass,
    ) -> Result<tape::TapeDoc> {
        let Self {
            mut ast, documents, ..
        } = self;
        for document in &documents {
            document.read.evaluate(
                file,
                code_rules,
                &ast.file_errors,
                &mut pass.metrics,
                &mut pass.work,
                &mut ast.settled,
                |_| None,
                Some(&document.matches),
            )?;
        }
        Ok(ast.settle(file, code_rules, &pass.metrics))
    }
}

/// Parses `file`'s source and keeps its documents (its tape and the ones it derives) for this
/// source revision. `Ok(Err(failure))` where the parse ran out of time or a tape could not be
/// encoded.
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
        let parsed = parse_tree(file.input.source.as_bytes());
        #[cfg(feature = "code-rule-profile")]
        drop(parse_timer);
        metrics.ast_parse_nanoseconds += parse_started.elapsed().as_nanos() as u64;
        // Forte stopping at a limit says nothing about the template: no tape, and no clause runs.
        let tree = match parsed {
            Ok(tree) => tree,
            Err(error) => return Ok(Err((FILE_BUDGET_EXCEEDED, error.message()))),
        };
        #[cfg(test)]
        pause_at_test_phase("native-tree", &file.input.source);
        #[cfg(feature = "code-rule-profile")]
        let _encode_timer = profile::PhaseTimer::start(profile::Phase::TapeEncode);
        match tape::TapeImage::encode_blade(&tree, grammar, &file.input.source, &interrupted) {
            Ok(image) => Some(Ok(image)),
            Err(tape::TemplateError::Parser(error)) => {
                return Ok(Err((FILE_BUDGET_EXCEEDED, error.message())));
            }
            Err(tape::TemplateError::Encode(error)) => Some(Err(error)),
        }
    } else {
        #[cfg(feature = "code-rule-profile")]
        let _native_owner = profile::NativeOwner::start();
        let mut parser = Parser::new();
        parser
            .set_language(&language.get_ts_language())
            .map_err(|error| invalid(error.to_string()))?;
        // The family's parse input (a PHP file's view, `php_view`); the tape reads the source.
        let parsed =
            crate::code_rule_dsl::language::adapter::parse_input(language, &file.input.source);
        let bytes = parsed.as_bytes();
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
        None => Err((FILE_BUDGET_EXCEEDED, "file parsing time budget exceeded")),
        Some(Err(_)) => Err((
            if matcher_budget_observed() {
                FILE_BUDGET_EXCEEDED
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

/// How a clause's run under its allowance (`ClauseWork::run`) ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ran {
    /// No poll found the allowance spent.
    Finished,
    /// The clause ran out of its own allowance, or was interrupted.
    OwnAllowance,
    /// The file's clause-work ceiling, which the clauses before it had spent, stopped it first.
    Ceiling,
}

/// The work a file's clauses may still spend together (`clause_work_ceiling`). Each clause runs
/// under its own allowance (`clause_work_budget`), cut down to what the ceiling has left: a clause
/// that runs out within its own allowance fails on its own account; one the ceiling stopped fails
/// on the account of the clauses before it.
struct ClauseWork {
    left: u64,
}

impl ClauseWork {
    fn new(source_bytes: usize) -> Self {
        Self {
            left: clause_work_ceiling(source_bytes),
        }
    }

    /// Runs `run` under a clause's allowance for a haystack of `bytes`, and spends what it used.
    /// `None`, without running it, where the ceiling is spent.
    fn run<T>(&mut self, bytes: usize, run: impl FnOnce() -> T) -> Option<(T, Ran)> {
        if self.left == 0 {
            return None;
        }
        let own = clause_work_budget(bytes);
        let units = own.min(self.left);
        let (result, allowance) = with_allowance(units, run);
        self.left -= allowance.spent.min(self.left);
        let ran = if !allowance.exhausted {
            Ran::Finished
        } else if units < own && allowance.spent >= units {
            Ran::Ceiling
        } else {
            Ran::OwnAllowance
        };
        Some((result, ran))
    }
}

/// Runs the candidate AST clauses `slots` (slot, matcher) read on one restored document
/// (`matcher::run_clause`) and records their witnesses: syntax-error regions and the witness budget
/// apply to each match, roots past `maxVisitedNodes` are not matched, and a decision stops at its
/// first witness. Each clause selects its roots (`select`, by its position in `slots`: node ids,
/// `None` for every root) and runs under an allowance of its own (`work`), and fails alone where it
/// runs out. With `sql`, each clause's matches (by its position in `slots`, roots in preorder) are
/// the SQL engine's, rendered in place of the matcher's. Returns which of `slots` ran: the others
/// were cut short before they started, by the workspace closing or the file's budget running out.
#[allow(clippy::too_many_arguments)]
fn evaluate_clauses(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    slots: &[(usize, &GrammarMatcher)],
    doc: &tape::TapeDoc,
    (errors, file_errors): (&SyntaxErrors, &SyntaxErrors),
    metrics: &mut Metrics,
    util_work: &mut [usize],
    work: &mut ClauseWork,
    runs: &[bool],
    select: impl Fn(usize) -> Option<Arc<[u32]>>,
    sql: Option<&[Vec<SqlMatch>]>,
) -> Result<Vec<bool>> {
    use crate::code_rule_dsl::matcher::{Stopped, enumerate, node_at, run_clause, with_file_memo};
    let target = ast_grep_core::AstGrep::doc(doc.clone());
    // A guest document's nodes span its program's text; error regions are the file's (`origin`).
    let place = crate::code_rule_dsl::origin::InFile::tape(doc);
    let mut settled = vec![false; slots.len()];
    with_file_memo(&target, || -> Result<()> {
        for (position, &(slot, grammar)) in slots.iter().enumerate() {
            let id = file.candidates.ast[slot];
            #[cfg(test)]
            if position > 0 {
                pass_deadline_at_test_phase("clause-done");
            }
            if file.failures.contains_key(&id) {
                continue;
            }
            // A clause whose required text the file lacks matches nothing, at no cost: it answers
            // whatever the file's budget and the ceiling have left.
            if !runs[position] {
                settled[position] = true;
                continue;
            }
            // Polls the file's budget, so a deadline that passed after the last clause's final
            // poll is recorded, and the evaluation is known to be cut short.
            if matcher_budget_exhausted() {
                continue;
            }
            let code_rule = &code_rules[id.0];
            let violation = &code_rule.clauses[id.1].violation;
            let decision = code_rule.clauses[id.1].decision;
            let each = violation.each.as_deref();
            let limits = &code_rule.capabilities.limits;
            // The file's placement decides whether a linter's suppression comment silences it;
            // a `rerail-ignore` naming the code rule always does.
            let honour_suppressions = code_rule.scope.honours_suppressions_at(&file.input.path);
            let ignorable = violations::names_rerail_ignore(violation, &target.root());
            let Some((result, ran)) = work.run(file.input.source.len(), || {
                let roots = select(position);
                // An anchor selection of no roots leaves nothing for the clause to match.
                if roots.as_deref().is_some_and(<[u32]>::is_empty) {
                    return Ok(());
                }
                let _first = each
                    .filter(|_| sql.is_none())
                    .map(crate::code_rule_dsl::matcher::EachScope::start);
                let bound = limits.visits_for(doc.transducer_len());
                let mut on_root = |node: u32, first, work: &mut usize| {
                    metrics.ast_matcher_calls += 1;
                    // One binding of the match: false ends the clause. A directive does not
                    // spend a violation's output allowance, so the enumeration, and a regex
                    // capture's groups, go on past suppressed witnesses until the reported
                    // ones overflow it.
                    let mut visit = |env, work: &mut usize| {
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
                        let errors = if grammar.crosses { file_errors } else { errors };
                        if errors.meets(&place.place(matched.get_node())) {
                            return Ok(true);
                        }
                        let matched_node = matched.get_node();
                        let keep = |witness: &mut Witness| {
                            if honour_suppressions
                                && violations::suppresses(Some(violation), matched_node, witness)
                            {
                                return false;
                            }
                            if ignorable
                                && violations::rerail_ignores(
                                    Some(violation),
                                    matched_node,
                                    witness,
                                )
                            {
                                if !violation.ignore_marks {
                                    return false;
                                }
                                witness.ignored = true;
                            }
                            true
                        };
                        let found = if each.is_some() {
                            let reported = file.witnesses.get(&id).map_or(0, Vec::len);
                            witnesses_from_match(
                                &file.input,
                                &matched,
                                grammar.language,
                                violation,
                                (limits.max_witnesses + 1).saturating_sub(reported),
                                matcher_budget_exhausted,
                                keep,
                            )
                        } else {
                            witness_from_match(
                                &file.input,
                                &matched,
                                grammar.language,
                                Some(violation),
                            )
                            .map(|mut witness| {
                                if keep(&mut witness) {
                                    vec![witness]
                                } else {
                                    Vec::new()
                                }
                            })
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
                            let witnesses = file.witnesses.get_mut(&id).expect("eligible check");
                            if witnesses.len() >= limits.max_witnesses {
                                // An output cap, not a work limit.
                                file.failures
                                    .insert(id, (WITNESS_LIMIT, "file witness budget exceeded"));
                                return Ok(false);
                            }
                            witnesses.push(witness);
                            metrics.witnesses += 1;
                            // A decision is answered by its first witness.
                            if decision {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    };
                    // The SQL engine enumerated `each` itself: each of its matches at a root is
                    // one visit.
                    match each.filter(|_| sql.is_none()) {
                        Some(name) => enumerate(&target, grammar, node, first, work, name, visit),
                        None => visit(first, work),
                    }
                };
                match sql {
                    None => run_clause(
                        &target,
                        grammar,
                        roots.as_deref(),
                        bound,
                        &mut util_work[slot],
                        on_root,
                    )
                    .and_then(|result| result),
                    Some(matches) => {
                        for found in matches[position]
                            .iter()
                            .take_while(|found| (found.root as usize) < bound)
                        {
                            let env = crate::code_rule_dsl::sql::environment(&target, found);
                            if !on_root(found.root, env, &mut util_work[slot])? {
                                break;
                            }
                        }
                        Ok(())
                    }
                }
            }) else {
                // The clauses before it spent the file's clause-work ceiling.
                settled[position] = true;
                file.failures
                    .insert(id, (FILE_BUDGET_EXCEEDED, FILE_CLAUSE_WORK_EXCEEDED));
                file.unstored.insert(id);
                continue;
            };
            settled[position] = true;
            match (result, ran) {
                (_, Ran::Ceiling) => {
                    file.failures
                        .insert(id, (FILE_BUDGET_EXCEEDED, FILE_CLAUSE_WORK_EXCEEDED));
                    file.unstored.insert(id);
                }
                (Ok(()), Ran::Finished) => {}
                (Err(Stopped::Limit(limit)), _) => {
                    file.failures
                        .insert(id, ("budget-exceeded", limit.message()));
                }
                // The clause's allowance ran out (or the workspace closed) before it finished.
                (Ok(()) | Err(Stopped::Deadline), _) => {
                    file.failures
                        .entry(id)
                        .or_insert(("budget-exceeded", CLAUSE_WORK_BUDGET_EXCEEDED));
                }
            }
        }
        Ok(())
    })?;
    Ok(settled)
}
