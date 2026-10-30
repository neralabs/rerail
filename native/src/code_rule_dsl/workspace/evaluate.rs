//! File evaluation: jobs run serially or on the worker pool, regex clauses, tape parsing, and AST
//! clause evaluation under the match budget.

use super::crossfile::{CrossFileConsumer, CrossFileScheduler};
use super::documents;
use super::persisted::{candidate_catalog, restore_persisted_file, result_context};
use super::storage::StorageContext;
use super::{
    CLAUSE_WORK_BUDGET_EXCEEDED, ClauseId, FILE_CLAUSE_WORK_EXCEEDED, FileState, LocalFailure,
    REUSE_LOCAL_RESULTS, check_closed, clause_work_budget, clause_work_ceiling, file_work_budget,
};
#[cfg(test)]
use super::{TEST_STORAGE_PREFETCH_FAILURE, pass_deadline_at_test_phase, pause_at_test_phase};
use crate::code_rule_dsl::blade::forte::tree::parse_tree;
use crate::code_rule_dsl::budget::{
    MatchBudget, matcher_budget_exhausted, matcher_budget_observed, with_allowance,
};
use crate::code_rule_dsl::eval::{
    WitnessError, document_matcher, file_language, grammar_matcher, witness_from_match,
    witnesses_from_match,
};
use crate::code_rule_dsl::import_target::ViewSlot;
use crate::code_rule_dsl::import_target::fs::View;
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    FileAnchorIndex, FileGates, FileScopeSlot, GrammarMatcher, Selection, UtilLimit,
};
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledCodeRule, FILE_BUDGET_EXCEEDED, FileInput, INPUT_LIMIT, Metrics, Query,
    RegexSource, WITNESS_LIMIT,
};
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile;
use crate::code_rule_dsl::soundness::{SyntaxErrors, SyntaxReading, UNREAD};
use crate::code_rule_dsl::violations::{Captures, Witness};
use crate::code_rule_dsl::{query, tape, violations};
use crate::content_store::HashKey;
use crate::{WORK_POOL, WORKER_THREADS};
use ast_grep_core::AstGrep as Root;
use ast_grep_language::LanguageExt;
use napi::Result;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// What import targets resolve among (`import_target::fs::View`), where a clause reads them.
    pub(super) import_view: Option<&'a Arc<View>>,
    /// The clauses that read import targets: their results are never stored.
    pub(super) import_clauses: &'a HashSet<ClauseId>,
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
            // The shared steps' budget grows with the file; each clause has its own allowance.
            let budget = MatchBudget::work(closed, file_work_budget(file.input.source.len()));
            let transient = file.transient_failure && crossfile.is_some();
            // A consumers' pass, or a pass over the clauses the store did not answer, runs some of
            // the file's clauses only.
            let partial = crossfile.is_some() || !file.restored_result_ids.is_empty();
            #[cfg(test)]
            let consumer_budget = crossfile.is_some().then(|| {
                super::TEST_CLAUSE_WORK_BUDGET.replace(
                    super::TEST_CONSUMER_CLAUSE_WORK_BUDGET
                        .get()
                        .or(super::TEST_CLAUSE_WORK_BUDGET.get()),
                )
            });
            let _consumer = CrossFileScheduler::install(crossfile);
            let _view = ViewSlot::install(context.import_view.cloned());
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
            if let Some(previous) = consumer_budget {
                super::TEST_CLAUSE_WORK_BUDGET.set(previous);
            }
            let mut metrics = evaluated?;
            // What a target resolves to depends on more than the file: no store may answer it.
            for id in &file.candidates.ast {
                if context.import_clauses.contains(id) {
                    file.unstored.insert(*id);
                }
            }
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
                // The summary's own work spends the file's budget too: a summary it cut short is
                // opaque, this workspace's alone.
                file.transient_failure |= budget.shared_spent();
            }
            metrics.storage_read_nanoseconds = storage_read_nanoseconds;
            Ok(metrics)
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
        let (haystack, offset) = query::regex_haystack(source, &file.input);
        let path_lines = LineTable::new(&file.input.path, Terminators::Eslint);
        let span = |start: usize, end: usize| match source {
            RegexSource::Content => lines
                .as_ref()
                .expect("eligible content has line table")
                .span(start + offset, end + offset),
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
    if !file.candidates.ast.is_empty() && !file.selector_skipped {
        metrics.restored_tapes += usize::from(file.tape.is_some());
        if any_eligible {
            *restored = evaluate_ast_file(
                file,
                code_rules,
                grammars,
                closed,
                lines,
                &mut work,
                &mut metrics,
            )?;
        }
    }
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
    if history {
        let candidates = Arc::clone(&file.candidates);
        for id in candidates.regex.iter().chain(&candidates.ast) {
            if file.failures.get(id).is_some_and(|(code, _)| {
                *code == "budget-exceeded" || *code == FILE_BUDGET_EXCEEDED
            }) {
                file.unstored.insert(*id);
            }
        }
    }
    Ok(metrics)
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
    // A file that embeds code is several documents (`embed`): its own tree, then each program it
    // embeds. Each clause reads one of them (`ast.embedded`), with that document's syntax errors,
    // anchors, scope tables and coordinates.
    let mut documents = vec![doc.clone()];
    if let Some(guests) = doc.guests() {
        documents.extend(guests.iter().map(|guest| guest.doc.clone()));
    }
    // Each candidate slot's document and matcher; `None` where the file holds no program the
    // clause reads.
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
    // Which clauses ran to an answer (or failed on their own); the rest are failed below. A clause
    // over a program the file does not embed has nothing to read: it answers.
    let mut settled = placed.iter().map(Option::is_none).collect::<Vec<_>>();
    let nodes = doc.transducer_len();
    // A clause crossing between the documents (`embedded`, `host`) reads around every one's errors;
    // they are gathered only where some clause crosses.
    let file_errors = if placed.iter().flatten().any(|(_, grammar)| grammar.crosses) {
        SyntaxErrors::of_file(&doc)
    } else {
        SyntaxErrors::from_ranges(Vec::new())
    };
    for (index, document) in documents.into_iter().enumerate() {
        let slots = placed
            .iter()
            .enumerate()
            .filter_map(|(slot, place)| {
                place
                    .filter(|(at, _)| *at == index)
                    .map(|(_, grammar)| (slot, grammar))
            })
            .collect::<Vec<_>>();
        if slots.is_empty() {
            // A document no clause reads still has its errors: the file was read around them.
            let errors = SyntaxErrors::of_doc(&document);
            if !errors.is_empty() {
                let (SyntaxReading::Partial(message) | SyntaxReading::Unread(message)) =
                    errors.reading(&lines, file.input.source.len());
                file.syntax.get_or_insert(message);
            }
            continue;
        }
        evaluate_document(
            file,
            code_rules,
            document,
            &slots,
            &file_errors,
            &lines,
            metrics,
            work,
            &mut settled,
        )?;
    }
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
    Ok(Some(doc))
}

/// The matchers of an AST clause, one per dialect it compiled for.
fn clause_matchers(code_rule: &CompiledCodeRule, clause: usize) -> &[GrammarMatcher] {
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &code_rule.clauses[clause].kind else {
        unreachable!("an AST candidate is an AST clause")
    };
    matchers
}

/// Runs the candidate clauses `slots` (slot, matcher) read on one document of the file (`embed`):
/// the file's own tree or a program it embeds. Its syntax errors (as ranges of the file) decide
/// which clauses run around them; its anchor index selects the roots; its scope tables join the
/// file's, which hold every document's. Marks in `settled` the slots that ran to an answer.
#[allow(clippy::too_many_arguments)]
fn evaluate_document(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    doc: tape::TapeDoc,
    slots: &[(usize, &GrammarMatcher)],
    file_errors: &SyntaxErrors,
    lines: &Arc<LineTable>,
    metrics: &mut Metrics,
    work: &mut ClauseWork,
    settled: &mut [bool],
) -> Result<()> {
    let errors = SyntaxErrors::of_doc(&doc);
    if !errors.is_empty() {
        match errors.reading(lines, file.input.source.len()) {
            // No AST clause runs on a document the parser did not read, sound or not; the failure
            // reports `file.syntax` in place of its static message (`results::failure_message`).
            SyntaxReading::Unread(message) => {
                for &(slot, _) in slots {
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
    let root = Root::doc(doc.clone());
    // The gates' file facts are read once for the document, whichever candidate reads them.
    let gates = FileGates::new(&file.input, root.root());
    for &(slot, grammar) in slots {
        let id = file.candidates.ast[slot];
        // A clause reads around its document's errors, or, crossing to the file's other document,
        // every document's. A file the query's file gates exclude matches nothing, errors or not.
        let read = if grammar.crosses {
            file_errors
        } else {
            &errors
        };
        if !file.failures.contains_key(&id) && read.incomplete(grammar) && gates.admit(grammar) {
            file.failures
                .insert(id, ("source-incomplete", "file has syntax errors"));
        }
    }
    #[cfg(test)]
    pause_at_test_phase("restored-doc", &file.input.source);
    // A document's node and utility visits grow with it (`Limits::visits_for`).
    let nodes = doc.transducer_len();
    let max_nodes = slots
        .iter()
        .map(|&(slot, _)| file.candidates.ast[slot])
        .filter(|id| !file.failures.contains_key(id))
        .map(|id| code_rules[id.0].capabilities.limits.visits_for(nodes))
        .max()
        .unwrap_or(0);
    // A clause whose matcher needs text the document lacks matches no root: it selects none. The
    // clauses share many literals, each searched for once.
    let text = ast_grep_core::Doc::get_source(&doc).as_str();
    let mut searched = rustc_hash::FxHashMap::<&str, bool>::default();
    let mut found = |text: &str, literal| {
        *searched
            .entry(literal)
            .or_insert_with(|| memchr::memmem::find(text.as_bytes(), literal.as_bytes()).is_some())
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
        .collect::<Vec<_>>();
    let plans = slots
        .iter()
        .zip(&runs)
        .filter(|(_, runs)| **runs)
        .map(|((_, grammar), _)| grammar.anchors.as_ref())
        .collect::<Vec<_>>();
    #[cfg(feature = "code-rule-profile")]
    let index_timer = profile::PhaseTimer::start(profile::Phase::Index);
    let index = FileAnchorIndex::build(&root, &plans, max_nodes, matcher_budget_exhausted);
    metrics.visited_nodes += index.visited_nodes();
    #[cfg(feature = "code-rule-profile")]
    drop(index_timer);
    #[cfg(feature = "code-rule-profile")]
    profile::record_index(index.len(), index.estimated_bytes(), index.complete());
    // A clause's anchor selection is its own work: it runs under the clause's allowance.
    let select = |position: usize| -> Option<Arc<[u32]>> {
        if !runs[position] {
            return Some(Arc::from([]));
        }
        #[cfg(feature = "code-rule-profile")]
        let _selection_timer = profile::PhaseTimer::start(profile::Phase::Selection);
        let selection = index.select(&slots[position].1.anchors);
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
        errors.hide_bindings(),
    )
    .file(&file.input);
    let match_started = Instant::now();
    let ran = evaluate_clauses(
        file,
        code_rules,
        slots,
        root.root().get_doc(),
        (&errors, file_errors),
        metrics,
        &mut util_work,
        work,
        &runs,
        select,
    )?;
    for (&(slot, _), ran) in slots.iter().zip(ran) {
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
/// runs out. Returns which of `slots` ran: the others were cut short before they started, by the
/// workspace closing or the file's budget running out.
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
                let _first = each.map(crate::code_rule_dsl::matcher::EachScope::start);
                run_clause(
                    &target,
                    grammar,
                    roots.as_deref(),
                    limits.visits_for(doc.transducer_len()),
                    &mut util_work[slot],
                    |node, first, work| {
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
                                    && violations::suppresses(
                                        Some(violation),
                                        matched_node,
                                        witness,
                                    )
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
                                let witnesses =
                                    file.witnesses.get_mut(&id).expect("eligible check");
                                if witnesses.len() >= limits.max_witnesses {
                                    // An output cap, not a work limit.
                                    file.failures.insert(
                                        id,
                                        (WITNESS_LIMIT, "file witness budget exceeded"),
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
                            Ok(true)
                        };
                        match each {
                            Some(name) => {
                                enumerate(&target, grammar, node, first, work, name, visit)
                            }
                            None => visit(first, work),
                        }
                    },
                )
                .and_then(|result| result)
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
