//! File evaluation: jobs run serially or on the worker pool, regex text batches, tape parsing, and
//! AST statement evaluation under the match budget.

use super::crossfile::{CrossFileConsumer, CrossFileScheduler};
use super::documents;
use super::persisted::{candidate_catalog, restore_persisted_file, result_context};
use super::storage::StorageContext;
use super::{
    CheckId, FileState, LocalFailure, REUSE_LOCAL_RESULTS, WorkspaceState, check_closed,
    file_time_budget,
};
#[cfg(test)]
use super::{TEST_STORAGE_PREFETCH_FAILURE, pass_deadline_at_test_phase, pause_at_test_phase};
use crate::content_store::HashKey;
use crate::policy_dsl::budget::{MatchBudget, matcher_budget_exhausted, matcher_budget_observed};
use crate::policy_dsl::eval::{
    WitnessError, file_language, grammar_matcher, span_json, witness_from_match,
    witnesses_from_match,
};
use crate::policy_dsl::findings::Witness;
use crate::policy_dsl::json::invalid;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::lines::{LineTable, Terminators};
use crate::policy_dsl::matcher::{
    FileAnchorIndex, FileScopeSlot, GrammarMatcher, Selection, UtilLimit,
};
use crate::policy_dsl::model::{CheckKind, CompiledPolicy, FileInput, Metrics, Query, RegexSource};
#[cfg(feature = "policy-profile")]
use crate::policy_dsl::profile;
use crate::policy_dsl::soundness::SyntaxErrors;
use crate::policy_dsl::{findings, query, tape};
use crate::{WORK_POOL, WORKER_THREADS};
use ast_grep_core::AstGrep as Root;
use ast_grep_language::LanguageExt;
use napi::Result;
use rayon::prelude::*;
use serde_json::json;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tree_sitter::{ParseOptions, Parser, Point};

#[cfg(test)]
pub(super) static TEXT_BATCH_REGISTRATIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// The matcher and anchor plan for this file's grammar; candidates only exist for supported grammars.
pub(super) fn candidate_grammar<'a>(
    policy: &'a CompiledPolicy,
    check: usize,
    file: &FileInput,
) -> &'a GrammarMatcher {
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &policy.checks[check].kind else {
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
    pub(super) policies: &'a [Arc<CompiledPolicy>],
    pub(super) sql: &'a Arc<query::SqlPlans>,
    pub(super) storage: Option<&'a StorageContext>,
    pub(super) closed: &'a Arc<AtomicBool>,
    /// Extract each Python file's call summary (a policy holds `calls` terms).
    pub(super) summarize: bool,
}

type Evaluated = Vec<(String, FileState, Result<Metrics>)>;

/// Restores what the store holds for each job and evaluates the rest: serially for a few files,
/// on the worker pool for more.
pub(super) fn run_jobs(jobs: &mut Vec<Job>, context: &JobContext<'_>) -> Result<Evaluated> {
    let catalog = candidate_catalog(context.policies)?;
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
            #[cfg(feature = "policy-profile")]
            let _worker_profile = profile::WorkerProfile::start();
            let budget = MatchBudget::start(closed, file_time_budget());
            let storage_started = Instant::now();
            #[cfg(feature = "policy-profile")]
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
                    context.policies,
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
            #[cfg(feature = "policy-profile")]
            drop(_storage_read_timer);
            let storage_read_nanoseconds = storage_started.elapsed().as_nanos() as u64;
            let transient = file.transient_failure && crossfile.is_some();
            let _consumer = CrossFileScheduler::install(crossfile);
            let mut metrics = evaluate_file(
                &mut file,
                context.policies,
                grammars,
                closed,
                context.sql,
                &budget,
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
                )?;
            }
            metrics.storage_read_nanoseconds = storage_read_nanoseconds;
            Ok(metrics)
        });
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

/// The compiled statements of the registered policies.
pub(super) fn sql_plans(state: &mut WorkspaceState) -> Arc<query::SqlPlans> {
    let policies = &state.policies;
    Arc::clone(
        state
            .sql_plans
            .get_or_insert_with(|| Arc::new(query::SqlPlans::compile(policies))),
    )
}

pub(super) fn evaluate_file(
    file: &mut FileState,
    policies: &[Arc<CompiledPolicy>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    sql: &query::SqlPlans,
    budget: &MatchBudget,
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
    let eligible = |id: &CheckId| {
        let limits = &policies[id.0].capabilities.limits;
        source_len <= limits.max_source_bytes && limits.max_files != 0
    };
    let needs_lines = (!file.selector_skipped && file.candidates.ast.iter().any(&eligible))
        || file.candidates.regex.iter().any(|id| {
            eligible(id)
                && matches!(
                    &policies[id.0].checks[id.1].kind,
                    CheckKind::Forbid(Query::Regex {
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
        crate::policy_dsl::matcher::note_line_table_construction();
    }
    metrics.line_index_nanoseconds = line_index_started.elapsed().as_nanos() as u64;
    #[cfg(feature = "policy-profile")]
    let _line_owner = lines
        .as_ref()
        .map(|index| profile::LineOwner::start(index.estimated_bytes()));
    #[cfg(feature = "policy-profile")]
    let regex_timer = profile::PhaseTimer::start(profile::Phase::Regex);
    let regex_started = Instant::now();
    let text_batch_failed = if file.candidates.regex.is_empty() {
        false
    } else {
        #[cfg(test)]
        TEXT_BATCH_REGISTRATIONS.fetch_add(1, Ordering::Relaxed);
        sql.with_file(
            crate::policy_dsl::transducer::sql_backend::BatchFile::text_shared(
                Arc::clone(&file.input.source),
                Arc::from(file.input.path.as_str()),
                Arc::from(file.input.language.as_str()),
                None,
            ),
            |batch| {
                for &id in &file.candidates.regex {
                    let policy = &policies[id.0];
                    let check = &policy.checks[id.1];
                    let CheckKind::Forbid(Query::Regex { regex, source }) = &check.kind else {
                        unreachable!()
                    };
                    let witnesses = file.witnesses.entry(id).or_default();
                    witnesses.clear();
                    if file.input.source.len() > policy.capabilities.limits.max_source_bytes
                        || policy.capabilities.limits.max_files == 0
                    {
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
                            .span_json(start, end),
                        RegexSource::Path => span_json(&path_lines, start, end),
                    };
                    let terminal = sql
                        .regex_terminal(id.0, id.1)
                        .expect("every regex check has a text terminal");
                    let rows = match batch.text_rows(terminal) {
                        Ok(rows) => rows,
                        Err(_) => {
                            file.failures
                                .insert(id, ("adapter-error", "query engine unavailable"));
                            continue;
                        }
                    };
                    let plain = regex.capture_names().flatten().next().is_none()
                        && check.finding.fix.is_none();
                    for (span_row, complete) in rows {
                        let Some((start, end)) = span_row else {
                            if !complete {
                                file.failures
                                    .insert(id, ("budget-exceeded", "file match budget exceeded"));
                            }
                            continue;
                        };
                        if witnesses.len() >= policy.capabilities.limits.max_witnesses {
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
                                captures: json!({}),
                                message: None,
                                fix: None,
                            });
                        } else {
                            witnesses.push(findings::regex_witness(
                                &file.input.path,
                                regex,
                                &found,
                                Some(&check.finding),
                                span,
                            ));
                        }
                        // A decision is answered by its first witness.
                        if check.decision {
                            break;
                        }
                    }
                    metrics.witnesses += witnesses.len();
                }
            },
        )
        .is_err()
    };
    if text_batch_failed {
        for id in &file.candidates.regex {
            file.failures
                .insert(*id, ("adapter-error", "query engine unavailable"));
        }
    }
    #[cfg(feature = "policy-profile")]
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
        if any_eligible {
            evaluate_ast_file(file, policies, grammars, closed, lines, &mut metrics, sql)?;
        }
    }
    for witness in file.witnesses.values_mut().flatten() {
        witness.path.clone_from(&file.input.path);
    }
    // A deadline or cancellation can surface as any budget failure, or as a matcher that stopped
    // early. Such an evaluation is not the content's verdict: it is retried and never persisted.
    // Only exhaustion some check observed counts; an evaluation that finished is kept, however
    // late it finished.
    file.transient_failure = budget.observed();
    Ok(metrics)
}

fn evaluate_ast_file(
    file: &mut FileState,
    policies: &[Arc<CompiledPolicy>],
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    closed: &AtomicBool,
    lines: Option<Arc<LineTable>>,
    metrics: &mut Metrics,
    sql: &query::SqlPlans,
) -> Result<()> {
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
                return Ok(());
            }
        }
    };
    if file.tape.is_none()
        && let Err(failure) = encode_tape(file, language, &grammar, closed, metrics)?
    {
        for &id in &file.candidates.ast {
            file.failures.entry(id).or_insert(failure);
        }
        return Ok(());
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
        return Ok(());
    }
    #[cfg(feature = "policy-profile")]
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
            return Ok(());
        }
    };
    #[cfg(feature = "policy-profile")]
    {
        drop(_decode_timer);
        profile::record_transient(doc.navigation_bytes());
    }
    let errors = SyntaxErrors::from_ranges(doc.syntax_error_ranges());
    if !errors.is_empty() {
        for &id in &file.candidates.ast {
            if !file.failures.contains_key(&id)
                && !candidate_grammar(&policies[id.0], id.1, &file.input).sound_under_errors
            {
                file.failures
                    .insert(id, ("source-incomplete", "file has syntax errors"));
            }
        }
        file.syntax = Some(errors.partial_message(&lines));
    }
    #[cfg(feature = "policy-profile")]
    let _doc_owner = profile::DocOwner::start();
    let root = Root::doc(doc);
    #[cfg(test)]
    pause_at_test_phase("restored-doc", &file.input.source);
    let max_nodes = file
        .candidates
        .ast
        .iter()
        .filter(|id| !file.failures.contains_key(id))
        .map(|id| policies[id.0].capabilities.limits.max_visited_nodes)
        .max()
        .unwrap_or(0);
    {
        // Shadows the grammar cache, which is no longer needed once the document is restored.
        let grammars = file
            .candidates
            .ast
            .iter()
            .map(|id| candidate_grammar(&policies[id.0], id.1, &file.input))
            .collect::<Vec<_>>();
        let plans = file
            .candidates
            .ast
            .iter()
            .zip(&grammars)
            .filter(|(id, _)| !file.failures.contains_key(id))
            .map(|(_, grammar)| grammar.anchors.as_ref())
            .collect::<Vec<_>>();
        #[cfg(feature = "policy-profile")]
        let index_timer = profile::PhaseTimer::start(profile::Phase::Index);
        let index = FileAnchorIndex::build(&root, &plans, max_nodes, matcher_budget_exhausted);
        metrics.visited_nodes += index.visited_nodes();
        let _ = index;
        #[cfg(feature = "policy-profile")]
        {
            drop(index_timer);
        }
        #[cfg(feature = "policy-profile")]
        let selection_timer = profile::PhaseTimer::start(profile::Phase::Selection);
        let selections = file
            .candidates
            .ast
            .iter()
            .zip(&grammars)
            .map(|(id, grammar)| {
                if file.failures.contains_key(id) {
                    return Selection::Roots(Cow::Borrowed(&[]));
                }
                index.select(&grammar.anchors)
            })
            .collect::<Vec<_>>();
        #[cfg(feature = "policy-profile")]
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
        let mut util_work = file
            .candidates
            .ast
            .iter()
            .map(|id| policies[id.0].capabilities.limits.max_visited_nodes)
            .collect::<Vec<_>>();
        #[cfg(feature = "policy-profile")]
        let _dispatch_timer = profile::PhaseTimer::start(profile::Phase::Dispatch);
        let scopes = FileScopeSlot::install(
            file.scopes
                .as_ref()
                .filter(|(revision, _)| *revision == file.source_revision)
                .map(|(_, tables)| tables.clone())
                .unwrap_or_default(),
            file.input.module.clone(),
            !errors.is_empty(),
        )
        .path(&file.input.path);
        let match_started = Instant::now();
        evaluate_statements(
            file,
            policies,
            sql,
            &grammars,
            root.root().get_doc(),
            &errors,
            &lines,
            metrics,
            &mut util_work,
            &selected_roots_by_slot(&selections, &index),
            max_nodes,
        )?;
        let tables = scopes.tables();
        if !tables.is_empty() {
            file.scopes = Some((file.source_revision, tables));
        }
        metrics.ast_match_nanoseconds += match_started.elapsed().as_nanos() as u64;
        #[cfg(test)]
        pass_deadline_at_test_phase("statements-done");
    }
    #[cfg(feature = "policy-profile")]
    profile::record_nodes(metrics.visited_nodes);
    for &id in &file.candidates.ast {
        let policy = &policies[id.0];
        if file.input.source.len() > policy.capabilities.limits.max_source_bytes
            || policy.capabilities.limits.max_files == 0
        {
            continue;
        }
        // Only exhaustion some step observed cut the work short; work that finished is complete.
        if matcher_budget_observed()
            || metrics.visited_nodes > policy.capabilities.limits.max_visited_nodes
        {
            file.failures
                .insert(id, ("budget-exceeded", "file AST work budget exceeded"));
        }
    }
    Ok(())
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
    let image = {
        #[cfg(feature = "policy-profile")]
        let _native_owner = profile::NativeOwner::start();
        let mut parser = Parser::new();
        parser
            .set_language(&language.get_ts_language())
            .map_err(|error| invalid(error.to_string()))?;
        let bytes = file.input.source.as_bytes();
        let mut input = |offset: usize, _: Point| &bytes[offset..];
        let mut progress = |_: &tree_sitter::ParseState| {
            if closed.load(Ordering::Relaxed) || matcher_budget_exhausted() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        #[cfg(feature = "policy-profile")]
        let parse_timer = profile::PhaseTimer::start(profile::Phase::Parse);
        let parse_started = Instant::now();
        let parsed = parser.parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        #[cfg(feature = "policy-profile")]
        drop(parse_timer);
        metrics.ast_parse_nanoseconds += parse_started.elapsed().as_nanos() as u64;
        match parsed {
            None => None,
            // Damaged trees are taped too: ERROR and MISSING nodes stay on the tape,
            // so their regions are rederived from each restored document.
            Some(tree) => {
                #[cfg(test)]
                pause_at_test_phase("native-tree", &file.input.source);
                #[cfg(feature = "policy-profile")]
                let _encode_timer = profile::PhaseTimer::start(profile::Phase::TapeEncode);
                let encoded = tape::TapeImage::encode(
                    tree.root_node(),
                    grammar,
                    &file.input.source,
                    &matcher_budget_exhausted,
                );
                #[cfg(feature = "policy-profile")]
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

/// Runs every candidate AST check's compiled statement over one restored document and records
/// their witnesses: syntax-error regions and the witness budget apply to each match, roots past
/// `maxVisitedNodes` are not reported, and a decision stops at its first witness. Full terminal
/// scans of the document visit at most `node_limit` nodes.
#[allow(clippy::too_many_arguments)]
fn evaluate_statements(
    file: &mut FileState,
    policies: &[Arc<CompiledPolicy>],
    plans: &query::SqlPlans,
    grammars: &[&GrammarMatcher],
    doc: &tape::TapeDoc,
    errors: &SyntaxErrors,
    lines: &Arc<LineTable>,
    metrics: &mut Metrics,
    util_work: &mut [usize],
    roots: &[Option<Arc<[u32]>>],
    node_limit: usize,
) -> Result<()> {
    let batch_file = crate::policy_dsl::transducer::sql_backend::BatchFile::new(
        doc.clone(),
        Arc::from(file.input.path.as_str()),
        Arc::from(file.input.language.as_str()),
        None,
    )
    .bounded(node_limit);
    let outcome = plans.with_file(batch_file, |batch| -> Result<()> {
        let view = batch.view();
        let target = view.file(0).expect("the batch's one file");
        for slot in 0..file.candidates.ast.len() {
            let id = file.candidates.ast[slot];
            if file.failures.contains_key(&id) || matcher_budget_exhausted() {
                continue;
            }
            let policy = &policies[id.0];
            let grammar = grammars[slot];
            let plan = plans
                .plan(id.0, id.1, grammar.language)
                .expect("every AST check compiled to a statement when its policy was added");
            let finding = &policy.checks[id.1].finding;
            let decision = policy.checks[id.1].decision;
            let each = finding.each.as_deref();
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
            let limits = &policy.capabilities.limits;
            let mut other_error = None;
            let result = {
                let _first = each.map(crate::policy_dsl::matcher::EachScope::start);
                batch.run_while(
                    plan,
                    0,
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
                                // See the matcher-path equivalent: a directive does not spend a
                                // finding's output allowance, so SQL enumeration must also continue
                                // past suppressed bindings.
                                let room = if finding.suppress.is_some() {
                                    usize::MAX
                                } else {
                                    (limits.max_witnesses + 1)
                                        .saturating_sub(file.witnesses.get(&id).map_or(0, Vec::len))
                                };
                                batch.enumerate(plan, 0, node, first, grammar, work, name, room)?
                            }
                            None => vec![first],
                        };
                        for env in matches {
                            let mut matched = ast_grep_core::NodeMatch::new(
                                target.node(node),
                                env.to_meta(target),
                            );
                            if let Some(transform) = &grammar.transform {
                                crate::policy_dsl::matcher::begin_check(grammar, *work);
                                if transform.apply(matched.get_env_mut()).is_err() {
                                    *work = crate::policy_dsl::matcher::end_check().0;
                                    file.failures
                                        .insert(id, ("budget-exceeded", UtilLimit::Work.message()));
                                    return Ok(false);
                                }
                                *work = crate::policy_dsl::matcher::end_check().0;
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
                                    finding,
                                    room,
                                    matcher_budget_exhausted,
                                    |start, end| lines.span_json(start, end),
                                )
                            } else {
                                witness_from_match(
                                    &file.input,
                                    &matched,
                                    grammar.language,
                                    Some(finding),
                                    |start, end| lines.span_json(start, end),
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
                                Err(WitnessError::Other(error)) => {
                                    other_error = Some(error);
                                    return Ok(false);
                                }
                            };
                            for witness in found {
                                if findings::suppresses(Some(finding), matched.get_node(), &witness)
                                {
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
            };
            if let Some(error) = other_error {
                return Err(error);
            }
            match result {
                Ok(()) => {}
                // A deadline or cancellation interrupts the statement, observing the exhaustion;
                // the caller records it.
                Err(_) if matcher_budget_observed() => continue,
                Err(error) => {
                    file.failures.insert(id, limit_failure(error));
                }
            }
        }
        Ok(())
    });
    match outcome {
        Ok(result) => result,
        Err(_) => {
            for &id in &file.candidates.ast {
                file.failures
                    .entry(id)
                    .or_insert(("adapter-error", "query engine unavailable"));
            }
            Ok(())
        }
    }
}
