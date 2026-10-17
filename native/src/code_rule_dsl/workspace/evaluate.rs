//! File evaluation: jobs run serially or on the worker pool, regex clauses, tape parsing, and AST
//! clause evaluation under the match budget.

use super::crossfile::{CrossFileConsumer, CrossFileScheduler};
use super::documents;
use super::persisted::{candidate_catalog, restore_persisted_file, result_context};
use super::sql::{SQL_UNSUPPORTED, SQL_UNSUPPORTED_MESSAGE, SqlEngine};
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
    WitnessError, file_language, grammar_matcher, witness_from_match, witnesses_from_match,
};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    FileAnchorIndex, FileScopeSlot, GrammarMatcher, Selection, UtilLimit,
};
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledCodeRule, FILE_BUDGET_EXCEEDED, FileInput, INPUT_LIMIT, Metrics, Query,
    RegexSource, WITNESS_LIMIT,
};
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile;
use crate::code_rule_dsl::soundness::{SyntaxErrors, SyntaxReading, UNREAD};
use crate::code_rule_dsl::sql::{ClauseRun, SqlMatch};
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
        // Batches small enough that an apply of a few files still runs on every worker.
        let size = sql
            .batch_size
            .min(jobs.len().div_ceil(WORKER_THREADS))
            .max(1);
        let mut jobs = jobs.into_iter().peekable();
        let mut batches = Vec::new();
        while jobs.peek().is_some() {
            batches.push(jobs.by_ref().take(size).collect::<Vec<_>>());
        }
        let run = |batch: Vec<Job>, grammars: &mut Grammars| {
            sql_batch(batch, context, sql, grammars, &read, &finish)
        };
        let evaluated: Vec<Evaluated> = if batches.len() < 2 {
            let mut grammars = BTreeMap::new();
            batches
                .into_iter()
                .map(|batch| run(batch, &mut grammars))
                .collect()
        } else {
            WORK_POOL.install(|| {
                batches
                    .into_par_iter()
                    .map_init(BTreeMap::new, |grammars, batch| run(batch, grammars))
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
struct Started {
    budget: SuspendedBudget,
    transient: bool,
    storage_read_nanoseconds: u64,
    pass: FilePass,
    ast: Option<SqlFile>,
}

/// Evaluates a batch of jobs with their AST clauses matched on the SQL engine: each file's native
/// steps up to its clauses' matching (storage, regex clauses, input limits, the tape, syntax
/// errors, required text and anchor selection), one SQL run for the batch's AST clauses, then
/// each file's native rendering of the matches and the rest of its steps.
fn sql_batch(
    batch: Vec<Job>,
    context: &JobContext<'_>,
    sql: &SqlEngine,
    grammars: &mut Grammars,
    read: &impl Fn(&str, &mut FileState, &mut Grammars) -> Result<u64>,
    finish: &impl Fn(
        &mut FileState,
        &mut Grammars,
        &MatchBudget,
        (bool, bool),
        Option<tape::TapeDoc>,
        Metrics,
        u64,
    ) -> Result<Metrics>,
) -> Evaluated {
    let closed = context.closed;
    #[cfg(feature = "code-rule-profile")]
    let _worker_profile = profile::WorkerProfile::start();
    let mut started = Vec::with_capacity(batch.len());
    for job in batch {
        let Job {
            path,
            mut file,
            consumes,
            crossfile,
        } = job;
        let start = check_closed(closed).and_then(|()| {
            let storage_read_nanoseconds = read(&path, &mut file, grammars)?;
            let budget = MatchBudget::work(closed, file_work_budget(file.input.source.len()));
            let transient = file.transient_failure && crossfile.is_some();
            let partial = crossfile.is_some() || !file.restored_result_ids.is_empty();
            let mut pass = FilePass::begin(&mut file, context.code_rules, partial);
            let mut ast = None;
            if pass.ast
                && let Some(doc) = restore_ast_file(
                    &mut file,
                    context.code_rules,
                    grammars,
                    closed,
                    pass.lines.clone(),
                    &mut pass.metrics,
                )?
            {
                ast = Some(SqlFile::plan(
                    &mut file,
                    context.code_rules,
                    doc,
                    &mut pass.metrics,
                    sql,
                ));
            }
            Ok(Started {
                budget: budget.suspend(),
                transient,
                storage_read_nanoseconds,
                pass,
                ast,
            })
        });
        started.push((path, file, consumes, crossfile, start));
    }
    // One run for the batch: each statement over the files whose clause it matches.
    let mut docs = Vec::new();
    let mut runs: Vec<ClauseRun> = Vec::new();
    let mut run_of = HashMap::new();
    // Per clause run of a file: its job, slot, run and position in the run.
    let mut wanted = Vec::new();
    for (job, (.., start)) in started.iter().enumerate() {
        let Ok(Started { ast: Some(ast), .. }) = start else {
            continue;
        };
        let index = docs.len();
        docs.push(ast.doc.restored.clone());
        for (slot, planned) in ast.runs.iter().enumerate() {
            let Some((statement, roots)) = planned else {
                continue;
            };
            let run = *run_of.entry(*statement).or_insert_with(|| {
                runs.push(ClauseRun {
                    statement: *statement,
                    files: Vec::new(),
                });
                runs.len() - 1
            });
            wanted.push((job, slot, run, runs[run].files.len()));
            runs[run].files.push((index, roots.clone()));
        }
    }
    let found = if runs.is_empty() {
        Ok(Vec::new())
    } else {
        sql.run(docs, &runs)
    };
    match found {
        Ok(mut found) => {
            for (job, slot, run, position) in wanted {
                if let Ok(Started { ast: Some(ast), .. }) = &mut started[job].4 {
                    ast.matches[slot] = std::mem::take(&mut found[run][position]);
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
    started
        .into_iter()
        .map(|(path, mut file, consumes, crossfile, start)| {
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
                let rendered = ast
                    .map(|ast| ast.render(&mut file, context.code_rules, &mut pass))
                    .transpose();
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
            (path, file, work)
        })
        .collect()
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

/// A file's restored document, as its AST clauses read it.
struct AstDoc {
    /// The document, for the job's summary pass.
    restored: tape::TapeDoc,
    root: Root<tape::TapeDoc>,
    errors: SyntaxErrors,
    lines: Arc<LineTable>,
    /// Its nodes.
    nodes: usize,
    /// The most nodes a clause that runs may visit (`Limits::visits_for`).
    max_nodes: usize,
    #[cfg(feature = "code-rule-profile")]
    _doc_owner: profile::DocOwner,
}

/// Restores the file's document for its AST clauses: from its tape (encoded first where it has
/// none), with its syntax errors read. `None`, with every candidate failed, where it cannot be.
fn restore_ast_file(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    lines: Option<Arc<LineTable>>,
    metrics: &mut Metrics,
) -> Result<Option<AstDoc>> {
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
    let errors = SyntaxErrors::of_doc(&doc);
    if !errors.is_empty() {
        match errors.reading(&lines, file.input.source.len()) {
            // No AST clause runs on a file the parser did not read, sound or not; the failure
            // reports `file.syntax` in place of its static message (`results::failure_message`).
            SyntaxReading::Unread(message) => {
                for &id in &file.candidates.ast {
                    file.failures
                        .entry(id)
                        .or_insert(("source-incomplete", UNREAD));
                }
                file.syntax = Some(message);
            }
            SyntaxReading::Partial(message) => {
                for &id in &file.candidates.ast {
                    if !file.failures.contains_key(&id)
                        && errors.incomplete(candidate_grammar(
                            &code_rules[id.0],
                            id.1,
                            &file.input,
                        ))
                    {
                        file.failures
                            .insert(id, ("source-incomplete", "file has syntax errors"));
                    }
                }
                file.syntax = Some(message);
            }
        }
    }
    #[cfg(feature = "code-rule-profile")]
    let _doc_owner = profile::DocOwner::start();
    let restored = doc.clone();
    let root = Root::doc(doc);
    #[cfg(test)]
    pause_at_test_phase("restored-doc", &file.input.source);
    // A file's node and utility visits grow with the file (`Limits::visits_for`).
    let nodes = restored.transducer_len();
    let max_nodes = file
        .candidates
        .ast
        .iter()
        .filter(|id| !file.failures.contains_key(id))
        .map(|id| code_rules[id.0].capabilities.limits.visits_for(nodes))
        .max()
        .unwrap_or(0);
    Ok(Some(AstDoc {
        restored,
        root,
        errors,
        lines,
        nodes,
        max_nodes,
        #[cfg(feature = "code-rule-profile")]
        _doc_owner,
    }))
}

/// The candidates' grammar matchers, and which of them run: a clause whose matcher needs text the
/// file lacks matches no root, so it selects none (the clauses share many literals, each searched
/// for once).
fn clause_grammars<'c>(
    file: &FileState,
    code_rules: &'c [Arc<CompiledCodeRule>],
) -> (Vec<&'c GrammarMatcher>, Vec<bool>) {
    let grammars = file
        .candidates
        .ast
        .iter()
        .map(|id| candidate_grammar(&code_rules[id.0], id.1, &file.input))
        .collect::<Vec<_>>();
    let mut searched = rustc_hash::FxHashMap::<&str, bool>::default();
    let mut found = |text: &str, literal| {
        *searched
            .entry(literal)
            .or_insert_with(|| memchr::memmem::find(text.as_bytes(), literal.as_bytes()).is_some())
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
    (grammars, runs)
}

/// The anchor index over the plans of the clauses that run, its visits counted in `metrics`.
fn anchor_index<'d>(
    doc: &'d AstDoc,
    grammars: &[&GrammarMatcher],
    runs: &[bool],
    metrics: &mut Metrics,
) -> FileAnchorIndex<'d, tape::TapeDoc> {
    let plans = grammars
        .iter()
        .zip(runs)
        .filter(|(_, runs)| **runs)
        .map(|(grammar, _)| grammar.anchors.as_ref())
        .collect::<Vec<_>>();
    #[cfg(feature = "code-rule-profile")]
    let index_timer = profile::PhaseTimer::start(profile::Phase::Index);
    let index = FileAnchorIndex::build(&doc.root, &plans, doc.max_nodes, matcher_budget_exhausted);
    metrics.visited_nodes += index.visited_nodes();
    #[cfg(feature = "code-rule-profile")]
    drop(index_timer);
    #[cfg(feature = "code-rule-profile")]
    profile::record_index(index.len(), index.estimated_bytes(), index.complete());
    index
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
    let Some(doc) = restore_ast_file(file, code_rules, grammars, closed, lines, metrics)? else {
        return Ok(None);
    };
    let nodes = doc.nodes;
    let settled = {
        let (grammars, runs) = clause_grammars(file, code_rules);
        let index = anchor_index(&doc, &grammars, &runs, metrics);
        // A clause's anchor selection is its own work: it runs under the clause's allowance.
        let select = |slot: usize| -> Option<Arc<[u32]>> {
            if !runs[slot] {
                return Some(Arc::from([]));
            }
            select_roots(&index, grammars[slot])
        };
        let mut util_work = file
            .candidates
            .ast
            .iter()
            .map(|id| code_rules[id.0].capabilities.limits.visits_for(nodes))
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
            doc.errors.hide_bindings(),
        )
        .path(&file.input.path);
        let match_started = Instant::now();
        let settled = evaluate_clauses(
            file,
            code_rules,
            &grammars,
            doc.root.root().get_doc(),
            &doc.errors,
            &doc.lines,
            metrics,
            &mut util_work,
            work,
            &runs,
            select,
            None,
        )?;
        let tables = scopes.tables();
        if !tables.is_empty() {
            file.scopes = Some((file.source_revision, tables));
        }
        metrics.ast_match_nanoseconds += match_started.elapsed().as_nanos() as u64;
        #[cfg(test)]
        pass_deadline_at_test_phase("checks-done");
        settled
    };
    settle_ast_file(file, code_rules, nodes, &settled, metrics);
    Ok(Some(doc.restored))
}

/// A clause's statement in the SQL program and its selected roots (`None`: every root).
type PlannedRun = (usize, Option<Arc<[u32]>>);

/// A file's AST clauses on the SQL engine: its document, which candidates run (`runs` of
/// `clause_grammars`), the statement and roots of each that the engine matches, and, after the
/// batch, their matches.
struct SqlFile {
    doc: AstDoc,
    gate: Vec<bool>,
    runs: Vec<Option<PlannedRun>>,
    matches: Vec<Vec<SqlMatch>>,
}

impl SqlFile {
    /// What the batch matches for the file: the anchor index and each running clause's selected
    /// roots, as the native dispatch computes them (`evaluate_ast_file`). A clause the engine
    /// refuses fails where it would run.
    fn plan(
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        doc: AstDoc,
        metrics: &mut Metrics,
        sql: &SqlEngine,
    ) -> Self {
        let (grammars, gate) = clause_grammars(file, code_rules);
        let runs = {
            let index = anchor_index(&doc, &grammars, &gate, metrics);
            let mut runs = Vec::with_capacity(gate.len());
            for (slot, &id) in file.candidates.ast.iter().enumerate() {
                if !gate[slot] {
                    runs.push(None);
                    continue;
                }
                let Some(statement) = sql.statement(code_rules, id, grammars[slot]) else {
                    file.failures
                        .insert(id, (SQL_UNSUPPORTED, SQL_UNSUPPORTED_MESSAGE));
                    file.unstored.insert(id);
                    runs.push(None);
                    continue;
                };
                let roots = select_roots(&index, grammars[slot]);
                // An anchor selection of no roots leaves nothing for the clause to match.
                if roots.as_deref().is_some_and(<[u32]>::is_empty) {
                    runs.push(None);
                } else {
                    runs.push(Some((statement, roots)));
                }
            }
            runs
        };
        let matches = runs.iter().map(|_| Vec::new()).collect();
        Self {
            doc,
            gate,
            runs,
            matches,
        }
    }

    /// Renders the batch's matches for the file as the native dispatch renders the matcher's
    /// (`evaluate_clauses`), and settles its clauses. Returns the document they were read on.
    fn render(
        self,
        file: &mut FileState,
        code_rules: &[Arc<CompiledCodeRule>],
        pass: &mut FilePass,
    ) -> Result<tape::TapeDoc> {
        let Self {
            doc, gate, matches, ..
        } = self;
        let (grammars, _) = clause_grammars(file, code_rules);
        let nodes = doc.nodes;
        let mut util_work = file
            .candidates
            .ast
            .iter()
            .map(|id| code_rules[id.0].capabilities.limits.visits_for(nodes))
            .collect::<Vec<_>>();
        let scopes = FileScopeSlot::install(
            file.scopes
                .as_ref()
                .filter(|(revision, _)| *revision == file.source_revision)
                .map(|(_, tables)| tables.clone())
                .unwrap_or_default(),
            file.input.module.clone(),
            doc.errors.hide_bindings(),
        )
        .path(&file.input.path);
        let match_started = Instant::now();
        let settled = evaluate_clauses(
            file,
            code_rules,
            &grammars,
            doc.root.root().get_doc(),
            &doc.errors,
            &doc.lines,
            &mut pass.metrics,
            &mut util_work,
            &mut pass.work,
            &gate,
            |_| None,
            Some(&matches),
        )?;
        let tables = scopes.tables();
        if !tables.is_empty() {
            file.scopes = Some((file.source_revision, tables));
        }
        pass.metrics.ast_match_nanoseconds += match_started.elapsed().as_nanos() as u64;
        #[cfg(test)]
        pass_deadline_at_test_phase("checks-done");
        settle_ast_file(file, code_rules, nodes, &settled, &pass.metrics);
        Ok(doc.restored)
    }
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

/// Runs every candidate AST clause (`matcher::run_clause`) over one restored document and records
/// their witnesses: syntax-error regions and the witness budget apply to each match, roots past
/// `maxVisitedNodes` are not matched, and a decision stops at its first witness. Each clause
/// selects its roots (`select`: node ids, `None` for every root) and runs under an allowance of
/// its own (`work`), and fails alone where it runs out. With `sql`, each clause's matches (by
/// slot, roots in preorder) are the SQL engine's, rendered in place of the matcher's. Returns which
/// clauses ran: the others were cut short before they started, by the workspace closing or the
/// file's budget running out.
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
    work: &mut ClauseWork,
    runs: &[bool],
    select: impl Fn(usize) -> Option<Arc<[u32]>>,
    sql: Option<&[Vec<SqlMatch>]>,
) -> Result<Vec<bool>> {
    use crate::code_rule_dsl::matcher::{Stopped, enumerate, node_at, run_clause, with_file_memo};
    let target = ast_grep_core::AstGrep::doc(doc.clone());
    let mut settled = vec![false; file.candidates.ast.len()];
    with_file_memo(&target, || -> Result<()> {
        for slot in 0..file.candidates.ast.len() {
            let id = file.candidates.ast[slot];
            #[cfg(test)]
            if slot > 0 {
                pass_deadline_at_test_phase("clause-done");
            }
            if file.failures.contains_key(&id) {
                continue;
            }
            // A clause whose required text the file lacks matches nothing, at no cost: it answers
            // whatever the file's budget and the ceiling have left.
            if !runs[slot] {
                settled[slot] = true;
                continue;
            }
            // Polls the file's budget, so a deadline that passed after the last clause's final
            // poll is recorded, and the evaluation is known to be cut short.
            if matcher_budget_exhausted() {
                continue;
            }
            let code_rule = &code_rules[id.0];
            let grammar = grammars[slot];
            let violation = &code_rule.clauses[id.1].violation;
            let decision = code_rule.clauses[id.1].decision;
            let each = violation.each.as_deref();
            let limits = &code_rule.capabilities.limits;
            // The file's placement decides whether a linter's suppression comment silences it;
            // a `rerail-ignore` naming the code rule always does.
            let honour_suppressions = code_rule.scope.honours_suppressions_at(&file.input.path);
            let ignorable = violations::names_rerail_ignore(violation, &target.root());
            let Some((result, ran)) = work.run(file.input.source.len(), || {
                let roots = select(slot);
                // An anchor selection of no roots leaves nothing for the clause to match.
                if roots.as_deref().is_some_and(<[u32]>::is_empty) {
                    return Ok(());
                }
                let _first = each.map(crate::code_rule_dsl::matcher::EachScope::start);
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
                        if errors.overlaps(&matched.get_node().range()) {
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
                                |start, end| lines.span(start, end),
                                keep,
                            )
                        } else {
                            witness_from_match(
                                &file.input,
                                &matched,
                                grammar.language,
                                Some(violation),
                                |start, end| lines.span(start, end),
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
                    match each {
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
                        for found in matches[slot]
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
                settled[slot] = true;
                file.failures
                    .insert(id, (FILE_BUDGET_EXCEEDED, FILE_CLAUSE_WORK_EXCEEDED));
                file.unstored.insert(id);
                continue;
            };
            settled[slot] = true;
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
