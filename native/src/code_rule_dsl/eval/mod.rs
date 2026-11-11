pub(super) mod evidence;
mod output;
mod query;

use super::calls;
use super::import_target::ViewSlot;
use super::import_target::fs::{HeldFiles, View};
use super::json::{invalid, json_error, required_str};
use super::language::adapter::package_manifests;
use super::model::{
    ClauseKind, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, FileInput, Metrics,
    Snapshot, Terms, package_marker_directory, python_module_in,
};
use super::value::names;
use napi::Result;
pub(super) use output::{
    coverage, evaluation_json, failure, overflow_evaluation, result_order, violation_from_witness,
    violation_id, violation_ids,
};
pub(super) use query::Output;
pub(super) use query::{
    TapeCacheSlot, WitnessError, document_matcher, evaluate_query, file_language, file_tape,
    grammar_matcher, witness_from_match, witnesses_from_match,
};
#[cfg(test)]
pub(super) use query::{reset_shared_group_lookups, shared_group_lookups};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// A repository snapshot's files. A Python file without a host `module` is named, as the workspace
/// names it, by the package markers (`__init__.py`) the snapshot holds (`python_module_in`), and
/// an ECMA file without a host `package` reads the nearest `package.json` entry the snapshot holds
/// (`DerivedPackages`), so a stateless evaluation of a file set and a workspace holding the same
/// files read the same modules and manifests. With `server_graph`, a derived manifest naming
/// `next` carries the server graph the snapshot's files give. Also returns what the derivation
/// read, so an editor's edit derives again only what it changes (`DerivedPackages::edited`).
pub(super) fn parse_snapshot(
    source: &str,
    server_graph: bool,
) -> Result<(Snapshot, DerivedPackages)> {
    let value: Value = serde_json::from_str(source).map_err(json_error)?;
    let revision = value.get("revision").and_then(Value::as_u64).unwrap_or(0) as u32;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("snapshot.files must be an array"))?;
    let packages = files
        .iter()
        .filter_map(|file| package_marker_directory(file.get("path")?.as_str()?))
        .collect::<HashSet<_>>();
    let mut derived = Vec::new();
    let files = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let path = required_str(file, "path", "snapshot.files")?;
            let language = required_str(file, "language", "snapshot.files")?;
            let at = format!("snapshot.files/{index}");
            // A `module` of another type is refused like a malformed one.
            let module = (crate::code_rule_dsl::language::adapter::packages(language)
                && file.get("module").is_none()
                && !packages.is_empty())
            .then(|| python_module_in(path, |directory| packages.contains(directory)))
            .flatten();
            let package = match file.get("package") {
                Some(package) => Some(crate::code_rule_dsl::language::read_manifest(package, &at)?),
                None => {
                    if package_manifests(language) {
                        derived.push(index);
                    }
                    None
                }
            };
            FileInput::new(
                path.to_owned(),
                language.to_owned(),
                required_str(file, "source", "snapshot.files")?
                    .to_owned()
                    .into(),
                file.get("module").or(module.as_ref()),
                package,
                &at,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut snapshot = Snapshot { revision, files };
    let derived = derive_context(&mut snapshot, derived, server_graph);
    Ok((snapshot, derived))
}

/// A snapshot's project context of a language family's files (ECMAScript's manifests): the context it gives the
/// files the host sent none for, kept so an editor's edit derives again only what it changes. Registered at load
/// (`install_snapshot_context`).
pub(crate) trait SnapshotContext: Send + Sync {
    /// Derives again what an edit of the file at `index` changes.
    fn edited(&mut self, snapshot: &mut Snapshot, index: usize);
}

/// What derives a snapshot's context: of the files at `files` (those of a family that reads one, without a host
/// context), with its server graphs where `server_graph`.
pub(crate) type DeriveContext = fn(&mut Snapshot, Vec<usize>, bool) -> Box<dyn SnapshotContext>;

static SNAPSHOT_CONTEXT: std::sync::OnceLock<DeriveContext> = std::sync::OnceLock::new();

/// Registers what derives a snapshot's context (once, at load).
pub(crate) fn install_snapshot_context(derive: DeriveContext) {
    let _ = SNAPSHOT_CONTEXT.set(derive);
}

/// No context: a snapshot of no family that derives one.
struct NoContext;

impl SnapshotContext for NoContext {
    fn edited(&mut self, _: &mut Snapshot, _: usize) {}
}

/// The snapshot's derived context (`SnapshotContext`).
pub(crate) type DerivedPackages = Box<dyn SnapshotContext>;

fn derive_context(snapshot: &mut Snapshot, files: Vec<usize>, server_graph: bool) -> DerivedPackages {
    match SNAPSHOT_CONTEXT.get() {
        Some(derive) => derive(snapshot, files, server_graph),
        None => Box::new(NoContext),
    }
}

pub(crate) mod ecma;
#[cfg(test)]
pub(super) use ecma::DERIVED_MANIFESTS;

/// Evaluates `code_rule`, its grammar matchers matched natively (`matcher::run_clause`).
pub(super) fn evaluate_code_rule(
    code_rule: &CompiledCodeRule,
    snapshot: &Snapshot,
    mode: &str,
) -> Result<Evaluation> {
    let mut evaluation = evaluate_clauses(code_rule, snapshot, mode, &code_rule.clauses)?;
    if code_rule
        .report_count_gt
        .is_some_and(|maximum| evaluation.violations.len() <= maximum)
    {
        evaluation.violations.clear();
    }
    Ok(evaluation)
}

pub(super) fn evaluate_clauses(
    code_rule: &CompiledCodeRule,
    snapshot: &Snapshot,
    mode: &str,
    clauses: &[CompiledClause],
) -> Result<Evaluation> {
    // P's anchor corpus for every rule (the `p-anchor-corpus` path, open item O20).
    #[cfg(all(test, feature = "p-anchor-corpus"))]
    let _corpus = crate::code_rule_dsl::matcher::CorpusSlot::enter();
    let _tapes = TapeCacheSlot::install();
    // P's syntax facts, shared by P's relational inputs (test-only, open item O18).
    let _syntax = super::relational::syntax::Pool::install();
    if mode != "decision" && mode != "report" {
        return Err(invalid(format!("unsupported evaluation mode `{mode}`")));
    }
    let limits = &code_rule.capabilities.limits;
    // The budget is what this code rule will read: the files in its scope, not the whole snapshot.
    let (in_scope_files, in_scope_bytes) = snapshot
        .files
        .iter()
        .filter(|file| code_rule.scope.matches(&file.path))
        .fold((0usize, 0usize), |(files, bytes): (usize, usize), file| {
            (files + 1, bytes.max(file.source.len()))
        });
    // Each reported clause has a coverage entry; a synthetic leaf is part of the clause it serves.
    let covered: Vec<(Option<String>, String)> = clauses
        .iter()
        .filter(|clause| !clause.synthetic)
        .map(|clause| (None, clause.id.clone()))
        .collect();
    if in_scope_files > limits.max_files || in_scope_bytes > limits.max_source_bytes {
        return Ok(Evaluation {
            clauses: covered,
            ..overflow_evaluation("runtime", "repository input budget exceeded")
        });
    }
    let mut evaluation = Evaluation {
        violations: Vec::new(),
        failures: Vec::new(),
        metrics: Metrics {
            files: in_scope_files,
            ..Metrics::default()
        },
        clauses: covered,
    };
    #[cfg(all(test, feature = "p-anchor-corpus"))]
    prepare_candidates(code_rule, snapshot);
    // Import targets resolve among the snapshot's files; a workspace's evaluation installed the
    // view of the files it holds instead (`ViewSlot::install_unless_present`).
    let _view = code_rule
        .reads_import_targets()
        .then(|| ViewSlot::install_unless_present(|| Arc::new(snapshot_view(snapshot))));
    let _calls = stateless_calls(code_rule, snapshot, clauses, &mut evaluation.metrics)?;
    let mut relational_jobs = Vec::new();
    for clause in clauses {
        names::take_unavailable();
        match &clause.kind {
            ClauseKind::Forbid(query) => {
                let mut partial = BTreeMap::new();
                let mut util_limit = None;
                let matches = evaluate_query(
                    query,
                    snapshot,
                    &BTreeMap::new(),
                    &code_rule.scope,
                    &mut evaluation.metrics,
                    limits,
                    mode == "decision" && code_rule.report_count_gt.is_none(),
                    &mut partial,
                    Some(&clause.violation),
                    &mut util_limit,
                )?;
                if let Some((path, limit)) = util_limit {
                    evaluation.failures.push(failure(
                        &clause.id,
                        "budget-exceeded",
                        limit.message(),
                        Some(&path),
                    ));
                    continue;
                }
                let matches = match matches {
                    Ok(matches) => matches,
                    Err(incomplete) => {
                        // A file the parser did not read reports how much of it the errors cover.
                        evaluation.failures.push(failure(
                            &clause.id,
                            "source-incomplete",
                            incomplete
                                .unread
                                .as_deref()
                                .unwrap_or("query source is incomplete"),
                            Some(&incomplete.path),
                        ));
                        continue;
                    }
                };
                for (path, message) in &partial {
                    evaluation.failures.push(failure(
                        &clause.id,
                        "source-partial",
                        message,
                        Some(path),
                    ));
                }
                if evaluation.metrics.witnesses > limits.max_witnesses {
                    evaluation.failures.push(failure(
                        &clause.id,
                        crate::code_rule_dsl::model::WITNESS_LIMIT,
                        "witness budget exceeded",
                        None,
                    ));
                    continue;
                }
                for witness in matches {
                    evaluation
                        .violations
                        .push(violation_from_witness(code_rule, clause, witness));
                    if mode == "decision" && code_rule.report_count_gt.is_none() {
                        break;
                    }
                }
            }
            ClauseKind::RequireEach { subject, evidence } => {
                let subject_rows = stateless_rows(
                    code_rule,
                    clause,
                    (subject, evidence::Side::Subject),
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                let evidence_rows = stateless_rows(
                    code_rule,
                    clause,
                    (evidence, evidence::Side::Evidence),
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                merge(
                    &mut evaluation,
                    evidence::require_each(
                        code_rule,
                        clause,
                        (subject, subject_rows),
                        (evidence, &evidence_rows),
                    ),
                );
            }
            ClauseKind::Budget { query, .. } => {
                let rows = stateless_rows(
                    code_rule,
                    clause,
                    (query, evidence::Side::Counted),
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                merge(
                    &mut evaluation,
                    evidence::budget(code_rule, clause, query, rows),
                );
            }
            // P's relational clauses (test-only, open item O18): one job per file where per-file.
            ClauseKind::Relational(plan) => {
                let partitions = if plan.per_file {
                    snapshot
                        .files
                        .iter()
                        .filter(|file| code_rule.scope.matches(&file.path))
                        .map(|file| Snapshot {
                            revision: snapshot.revision,
                            files: vec![file.clone()],
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![snapshot.clone()]
                };
                for partition in partitions {
                    relational_jobs.push(
                        crate::code_rule_dsl::residency::admission::relational::Job {
                            code_rule,
                            clause,
                            inputs:
                                crate::code_rule_dsl::residency::admission::relational::Inputs {
                                    gate: None,
                                    file_local: false,
                                    read_bytes: 0,
                                    rows: None,
                                    facts: Vec::new(),
                                    snapshot: partition,
                                },
                            budget: super::budget::Context::capture(),
                        },
                    );
                }
            }
            ClauseKind::Llm => evaluation.failures.push(failure(
                &clause.id,
                "missing-capability",
                "LLM capability is unavailable",
                None,
            )),
        }
        // A `\N{...}` escape the malformed names table could not answer read as unknown: the
        // clause has no verdict of its own, and the engine's build is at fault, not the source.
        if names::take_unavailable() {
            evaluation.failures.push(failure(
                &clause.id,
                "adapter-error",
                names::TABLE_UNAVAILABLE,
                None,
            ));
        }
    }
    for result in
        crate::code_rule_dsl::residency::admission::relational::evaluate_batch(relational_jobs)
    {
        merge(&mut evaluation, result);
    }
    if evaluation.metrics.witnesses > limits.max_witnesses
        && !evaluation.failures.iter().any(|failure| {
            failure.get("code").and_then(Value::as_str)
                == Some(crate::code_rule_dsl::model::WITNESS_LIMIT)
        })
    {
        evaluation.failures.push(failure(
            "runtime",
            crate::code_rule_dsl::model::WITNESS_LIMIT,
            "witness budget exceeded",
            None,
        ));
    }
    Ok(evaluation)
}

/// What a snapshot's import targets resolve among: its files, with no disk below them.
fn snapshot_view(snapshot: &Snapshot) -> View {
    let mut held = HeldFiles::default();
    for file in &snapshot.files {
        held.hold(&file.path, &file.source, None);
    }
    View::new(Arc::new(held), None)
}

fn merge(evaluation: &mut Evaluation, mut other: Evaluation) {
    evaluation.violations.append(&mut other.violations);
    evaluation.failures.append(&mut other.failures);
    evaluation.metrics.witnesses += other.metrics.witnesses;
    evaluation.metrics.observe_native_work(&other.metrics);
}

/// P's corpus publication and candidate preparation for one rule's stateless evaluation (the
/// `p-anchor-corpus` path, open item O20).
#[cfg(all(test, feature = "p-anchor-corpus"))]
fn prepare_candidates(code_rule: &CompiledCodeRule, snapshot: &Snapshot) {
    use crate::code_rule_dsl::matcher::{CorpusSlot, anchor_plans};
    let plans = anchor_plans(std::iter::once(code_rule));
    if plans.is_empty() {
        return;
    }
    let corpus = CorpusSlot::current();
    let mut demands = BTreeMap::new();
    for file in snapshot
        .files
        .iter()
        .filter(|file| code_rule.scope.matches(&file.path))
    {
        let Some(lang) = file_language(file) else {
            continue;
        };
        if !plans.contains_key(&u32::from(
            crate::code_rule_dsl::language::tape_tag(lang).expect("tape language"),
        )) {
            continue;
        }
        match file_tape(lang, file) {
            Ok(None) => {}
            Ok(Some((doc, _))) => {
                demands.insert(
                    file.path.clone(),
                    crate::code_rule_dsl::matcher::anchor_plans_for_file(code_rule, file, &doc),
                );
                if let Err(error) = corpus.put(
                    &file.path,
                    &doc,
                    &crate::code_rule_dsl::budget::matcher_budget_exhausted,
                ) {
                    eprintln!("SQL AST batch publication unavailable: {error}");
                }
            }
            Err(error) => eprintln!("SQL AST batch source unavailable: {error}"),
        }
    }
    #[cfg(feature = "code-rule-profile")]
    corpus.record_origins(std::iter::once(code_rule));
    if let Err(error) = corpus.prepare_files(&demands) {
        eprintln!("SQL AST corpus batch unavailable; retaining authoritative candidates: {error}");
    }
}

/// Each input relation's rows of a relational clause (P's, test-only, open item O18).
#[cfg(test)]
pub(super) fn relational_rows(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    plan: &crate::code_rule_dsl::residency::admission::relational::Plan,
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<Vec<evidence::Rows>> {
    let _calls = stateless_calls(code_rule, snapshot, std::slice::from_ref(clause), metrics)?;
    plan.inputs
        .iter()
        .map(|(_, input)| {
            stateless_rows(
                code_rule,
                clause,
                (input, evidence::Side::Counted),
                snapshot,
                metrics,
            )
        })
        .collect()
}

/// Each input relation's rows of D's relational plan (test-only, open item O19).
#[cfg(test)]
pub(super) fn data_layer_rows(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    plan: &crate::code_rule_dsl::waves::relational::Plan,
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<Vec<evidence::Rows>> {
    let _calls = stateless_calls(code_rule, snapshot, std::slice::from_ref(clause), metrics)?;
    plan.inputs
        .iter()
        .map(|(_, input)| {
            stateless_rows(
                code_rule,
                clause,
                (input, evidence::Side::Counted),
                snapshot,
                metrics,
            )
        })
        .collect()
}

/// An evidence relation's rows in a stateless snapshot: each file of the code rule's scope and the
/// relation's domain evaluated alone, so that one file's failure makes only its rows unknown.
pub(super) fn stateless_rows(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    (relation, side): (&FactQuery, evidence::Side),
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<evidence::Rows> {
    stateless_rows_to(code_rule, clause, (relation, side), snapshot, metrics, None)
}
pub(in crate::code_rule_dsl) fn stateless_rows_to(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    (relation, side): (&FactQuery, evidence::Side),
    snapshot: &Snapshot,
    metrics: &mut Metrics,
    output: Option<&Output<'_>>,
) -> Result<evidence::Rows> {
    let ast = relation.ast.as_ref();
    let violation = evidence::producer_violation(clause, relation, side);
    let limits = &code_rule.capabilities.limits;
    let mut rows = evidence::Rows::default();
    // Rows in path order, as the workspace's evidence index keeps them, whatever the snapshot's
    // file order: a group's anchor is its first row.
    let mut files = snapshot
        .files
        .iter()
        .filter(|file| {
            code_rule.scope.matches(&file.path)
                && ast
                    .domain
                    .as_ref()
                    .is_none_or(|domain| domain.matches(&file.path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in files {
        let single = Snapshot {
            revision: snapshot.revision,
            files: vec![file.clone()],
        };
        let mut partial = BTreeMap::new();
        let mut util_limit = None;
        let mut file_metrics = Metrics::default();
        let witnesses = query::evaluate_query_to(
            &ast.query,
            &single,
            &BTreeMap::new(),
            &code_rule.scope,
            &mut file_metrics,
            limits,
            false,
            &mut partial,
            Some(&violation),
            &mut util_limit,
            output,
        )?;
        metrics.visited_nodes += file_metrics.visited_nodes;
        match witnesses {
            Ok(witnesses)
                if util_limit.is_none()
                    && partial.is_empty()
                    && file_metrics.witnesses <= limits.max_witnesses =>
            {
                rows.witnesses.extend(witnesses);
            }
            _ => {
                rows.incomplete.insert(file.path.clone());
            }
        }
    }
    Ok(rows)
}

/// The closure the `calls` terms of `clauses` read, over the snapshot's Python and PHP files in
/// the code rule's scope, installed for their evaluation. Each file's seeds are its own evaluation of
/// each term's seed query, so one file's failure makes only its definitions unknown. Seed work
/// spends the evaluation's work budget (`metrics.visited_nodes`); seed witnesses are no violation,
/// so each seed evaluation has its own witness budget, as a workspace's seed producer has per
/// file, and spends none of the clauses'.
pub(in crate::code_rule_dsl) fn stateless_calls(
    code_rule: &CompiledCodeRule,
    snapshot: &Snapshot,
    clauses: &[CompiledClause],
    metrics: &mut Metrics,
) -> Result<Option<calls::EvaluationSlot>> {
    let mut terms = Terms::default();
    for clause in clauses {
        if let Some(query) = clause.kind.calls_query() {
            for term in query.calls() {
                terms.add(&term);
            }
        }
    }
    if terms.is_empty() {
        return Ok(None);
    }
    let limits = &code_rule.capabilities.limits;
    let mut summaries = Vec::new();
    let mut seeds = std::collections::HashMap::<Arc<str>, calls::FileSeeds>::new();
    for file in &snapshot.files {
        let Some(language) = crate::code_rule_dsl::language::dialect_by_label(&file.language)
        else {
            continue;
        };
        if !calls::summarizes(language) || !code_rule.scope.matches(&file.path) {
            continue;
        }
        let path: Arc<str> = Arc::from(file.path.as_str());
        let module = file.module.as_deref();
        // A file that embeds code reads its calls from its tape's embedded program
        // (`calls::calls_document`).
        let (summary, _) = if !language.embeds().is_empty() {
            let bytes = file.source.len();
            // A file the parser stopped on (`file_tape`) has no program to read calls from.
            let doc = query::file_tape(language, file)?.map(|(doc, _)| doc);
            match doc.and_then(|doc| calls::calls_document(doc, language)) {
                Some((doc, dialect)) => {
                    let has_errors =
                        !crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&doc).is_empty();
                    let root = ast_grep_core::AstGrep::doc(doc);
                    calls::summarize(
                        root.root(),
                        dialect,
                        &file.path,
                        module,
                        has_errors,
                        bytes,
                        None,
                    )
                }
                None => (
                    calls::summary::Summary {
                        bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
                        ..Default::default()
                    },
                    None,
                ),
            }
        } else {
            let root = crate::code_rule_dsl::language::adapter::ast_grep(language, &file.source);
            let has_errors = root.root().get_doc().tree.root_node().has_error();
            let bytes = file.source.len();
            calls::summarize(
                root.root(),
                language,
                &file.path,
                module,
                has_errors,
                bytes,
                None,
            )
        };
        let summary = Arc::new(summary);
        summaries.push((Arc::clone(&path), Arc::clone(&summary)));
        let single = Snapshot {
            revision: snapshot.revision,
            files: vec![file.clone()],
        };
        let file_seeds = seeds.entry(path).or_default();
        // Terms differing only in `through` share one seed query.
        let mut evaluated = std::collections::HashMap::<Arc<str>, Option<Vec<u32>>>::new();
        for (id, term) in terms.terms.iter().enumerate() {
            let id = id as calls::TermId;
            if let Some(starts) = evaluated.get(&term.seed_key) {
                match starts {
                    Some(starts) => starts
                        .iter()
                        .for_each(|start| file_seeds.matched(*start, id)),
                    None => file_seeds.unknown.push(id),
                }
                continue;
            }
            let (mut partial, mut util_limit) = (BTreeMap::new(), None);
            let mut seed_metrics = Metrics {
                visited_nodes: metrics.visited_nodes,
                ..Metrics::default()
            };
            let witnesses = evaluate_query(
                &term.seed,
                &single,
                &BTreeMap::new(),
                &code_rule.scope,
                &mut seed_metrics,
                limits,
                false,
                &mut partial,
                None,
                &mut util_limit,
            )?;
            metrics.visited_nodes = seed_metrics.visited_nodes;
            let starts = match witnesses {
                // Past the witness budget the witnesses stop short: the file's seeds are unknown.
                Ok(witnesses)
                    if util_limit.is_none()
                        && partial.is_empty()
                        && seed_metrics.witnesses <= limits.max_witnesses =>
                {
                    let starts = calls::seed_starts(
                        witnesses.iter().map(|witness| {
                            witness.span.as_ref().expect("an ast witness has a span")
                        }),
                        term.seed_scope,
                        Some(summary.as_ref()),
                    )
                    .unwrap_or_default();
                    for start in &starts {
                        file_seeds.matched(*start, id);
                    }
                    Some(starts)
                }
                _ => {
                    file_seeds.unknown.push(id);
                    None
                }
            };
            evaluated.insert(Arc::clone(&term.seed_key), starts);
        }
    }
    let view = calls::CallsView::build_modes(terms.questions(), summaries.clone(), &seeds);
    Ok(Some(calls::EvaluationSlot::install(
        calls::EvaluationCalls {
            view: Arc::new(view),
            summaries: summaries
                .into_iter()
                .map(|(path, summary)| (path.to_string(), summary))
                .collect(),
        },
    )))
}

#[cfg(test)]
mod tests;
