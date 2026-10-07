pub(super) mod evidence;
mod output;
mod query;

use super::calls;
use super::json::{invalid, json_error, required_str};
use super::model::{
    CheckKind, CompiledCheck, CompiledPolicy, Evaluation, FactQuery, FileInput, Metrics, Snapshot,
    Terms,
};
use super::query::PolicyStatements;
use ast_grep_language::LanguageExt;
use napi::Result;
pub(super) use output::{
    coverage, evaluation_json, failure, finding_from_witness, finding_identity, finding_ids,
    overflow_evaluation, span_json,
};
use query::query_failure_path;
#[cfg(test)]
pub(super) use query::{FAIL_WITNESS, reset_shared_group_lookups, shared_group_lookups};
pub(super) use query::{
    WitnessError, evaluate_query, file_language, grammar_matcher, witness_from_match,
    witnesses_from_match,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) fn parse_snapshot(source: &str) -> Result<Snapshot> {
    let value: Value = serde_json::from_str(source).map_err(json_error)?;
    let revision = value.get("revision").and_then(Value::as_u64).unwrap_or(0) as u32;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("snapshot.files must be an array"))?
        .iter()
        .enumerate()
        .map(|(index, file)| {
            // A `module` of another type is refused like a malformed one.
            let module = file.get("module");
            FileInput::new(
                required_str(file, "path", "snapshot.files")?.to_owned(),
                required_str(file, "language", "snapshot.files")?.to_owned(),
                required_str(file, "source", "snapshot.files")?
                    .to_owned()
                    .into(),
                module,
                &format!("snapshot.files/{index}"),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Snapshot { revision, files })
}

/// Evaluates `policy`, its grammar matchers running as `statements` (`compile_document`).
pub(super) fn evaluate_policy(
    policy: &CompiledPolicy,
    statements: &PolicyStatements,
    snapshot: &Snapshot,
    mode: &str,
) -> Result<Evaluation> {
    evaluate_checks(policy, statements, snapshot, mode, &policy.checks)
}

pub(super) fn evaluate_checks(
    policy: &CompiledPolicy,
    statements: &PolicyStatements,
    snapshot: &Snapshot,
    mode: &str,
    checks: &[CompiledCheck],
) -> Result<Evaluation> {
    if mode != "decision" && mode != "diagnostic" {
        return Err(invalid(format!("unsupported evaluation mode `{mode}`")));
    }
    let limits = &policy.capabilities.limits;
    // The budget is what this policy will read: the files in its scope, not the whole snapshot.
    let (in_scope_files, in_scope_bytes) = snapshot
        .files
        .iter()
        .filter(|file| policy.scope.matches(&file.path))
        .fold((0usize, 0usize), |(files, bytes), file| {
            (files + 1, bytes + file.source.len())
        });
    // Each reported check has a coverage entry; a synthetic leaf is part of the check it serves.
    let covered: Vec<(Option<String>, String)> = checks
        .iter()
        .filter(|check| !check.synthetic)
        .map(|check| (None, check.id.clone()))
        .collect();
    if in_scope_files > limits.max_files || in_scope_bytes > limits.max_source_bytes {
        return Ok(Evaluation {
            checks: covered,
            ..overflow_evaluation("runtime", "repository input budget exceeded")
        });
    }
    let mut evaluation = Evaluation {
        findings: Vec::new(),
        failures: Vec::new(),
        metrics: Metrics {
            files: in_scope_files,
            ..Metrics::default()
        },
        checks: covered,
    };
    let _calls = stateless_calls(
        policy,
        statements,
        snapshot,
        checks,
        &mut evaluation.metrics,
    )?;
    for check in checks {
        match &check.kind {
            CheckKind::Forbid(query) => {
                let mut partial = BTreeMap::new();
                let mut util_limit = None;
                let matches = evaluate_query(
                    query,
                    statements,
                    snapshot,
                    &BTreeMap::new(),
                    &policy.scope,
                    &mut evaluation.metrics,
                    limits,
                    mode == "decision",
                    &mut partial,
                    Some(&check.finding),
                    &mut util_limit,
                )?;
                if let Some((path, limit)) = util_limit {
                    evaluation.failures.push(failure(
                        &check.id,
                        "budget-exceeded",
                        limit.message(),
                        Some(&path),
                    ));
                    continue;
                }
                let Some(matches) = matches else {
                    evaluation.failures.push(failure(
                        &check.id,
                        "source-incomplete",
                        "query source is incomplete",
                        query_failure_path(query, snapshot, &policy.scope),
                    ));
                    continue;
                };
                for (path, message) in &partial {
                    evaluation.failures.push(failure(
                        &check.id,
                        "source-partial",
                        message,
                        Some(path),
                    ));
                }
                if evaluation.metrics.witnesses > limits.max_witnesses {
                    evaluation.failures.push(failure(
                        &check.id,
                        "budget-exceeded",
                        "witness budget exceeded",
                        None,
                    ));
                    continue;
                }
                for witness in matches {
                    evaluation
                        .findings
                        .push(finding_from_witness(policy, check, witness));
                    if mode == "decision" {
                        break;
                    }
                }
            }
            CheckKind::RequireEach { subject, evidence } => {
                let subject_rows = stateless_rows(
                    policy,
                    statements,
                    check,
                    subject,
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                let evidence_rows = stateless_rows(
                    policy,
                    statements,
                    check,
                    evidence,
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                merge(
                    &mut evaluation,
                    evidence::require_each(
                        policy,
                        check,
                        (subject, subject_rows),
                        (evidence, &evidence_rows),
                    ),
                );
            }
            CheckKind::Budget { query, .. } => {
                let rows = stateless_rows(
                    policy,
                    statements,
                    check,
                    query,
                    snapshot,
                    &mut evaluation.metrics,
                )?;
                merge(
                    &mut evaluation,
                    evidence::budget(policy, check, query, rows),
                );
            }
            CheckKind::Llm => evaluation.failures.push(failure(
                &check.id,
                "missing-capability",
                "LLM capability is unavailable",
                None,
            )),
        }
    }
    if evaluation.metrics.witnesses > limits.max_witnesses
        && !evaluation
            .failures
            .iter()
            .any(|failure| failure.get("code").and_then(Value::as_str) == Some("budget-exceeded"))
    {
        evaluation.failures.push(failure(
            "runtime",
            "budget-exceeded",
            "witness budget exceeded",
            None,
        ));
    }
    Ok(evaluation)
}

fn merge(evaluation: &mut Evaluation, mut other: Evaluation) {
    evaluation.findings.append(&mut other.findings);
    evaluation.failures.append(&mut other.failures);
    evaluation.metrics.witnesses += other.metrics.witnesses;
}

/// An evidence relation's rows in a stateless snapshot: each file of the policy's scope and the
/// relation's domain evaluated alone, so that one file's failure makes only its rows unknown.
fn stateless_rows(
    policy: &CompiledPolicy,
    statements: &PolicyStatements,
    check: &CompiledCheck,
    relation: &FactQuery,
    snapshot: &Snapshot,
    metrics: &mut Metrics,
) -> Result<evidence::Rows> {
    let ast = relation.ast.as_ref();
    let finding = evidence::producer_finding(check, relation);
    let limits = &policy.capabilities.limits;
    let mut rows = evidence::Rows::default();
    // Rows in path order, as the workspace's evidence index keeps them, whatever the snapshot's
    // file order: a group's anchor is its first row.
    let mut files = snapshot
        .files
        .iter()
        .filter(|file| {
            policy.scope.matches(&file.path)
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
        let witnesses = evaluate_query(
            &ast.query,
            statements,
            &single,
            &BTreeMap::new(),
            &policy.scope,
            &mut file_metrics,
            limits,
            false,
            &mut partial,
            Some(&finding),
            &mut util_limit,
        )?;
        metrics.visited_nodes += file_metrics.visited_nodes;
        match witnesses {
            Some(witnesses)
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

/// The closure the `calls` terms of `checks` read, over the snapshot's Python and PHP files in
/// the policy's scope, installed for their evaluation. Each file's seeds are its own evaluation of
/// each term's seed query, so one file's failure makes only its definitions unknown. Seed work
/// spends the evaluation's work budget (`metrics.visited_nodes`); seed witnesses are no finding,
/// so each seed evaluation has its own witness budget, as a workspace's seed producer has per
/// file, and spends none of the checks'.
fn stateless_calls(
    policy: &CompiledPolicy,
    statements: &PolicyStatements,
    snapshot: &Snapshot,
    checks: &[CompiledCheck],
    metrics: &mut Metrics,
) -> Result<Option<calls::EvaluationSlot>> {
    let mut terms = Terms::default();
    for check in checks {
        if let CheckKind::Forbid(query) = &check.kind {
            for term in query.calls() {
                terms.add(&term);
            }
        }
    }
    if terms.is_empty() {
        return Ok(None);
    }
    let limits = &policy.capabilities.limits;
    let mut summaries = Vec::new();
    let mut seeds = std::collections::HashMap::<Arc<str>, calls::FileSeeds>::new();
    for file in &snapshot.files {
        let Some(language) = crate::policy_dsl::language::dialect_by_label(&file.language) else {
            continue;
        };
        if !calls::summarizes(language) || !policy.scope.matches(&file.path) {
            continue;
        }
        let path: Arc<str> = Arc::from(file.path.as_str());
        let module = file.module.as_deref();
        let (summary, _) = match crate::policy_dsl::language::Family::of(language) {
            // A Blade file's calls are its tape's embedded program's (`blade::view`).
            crate::policy_dsl::language::Family::Blade => {
                let (doc, _) = query::file_tape(language, file)?;
                let has_errors = !doc.syntax_error_ranges().is_empty();
                let root = ast_grep_core::AstGrep::doc(doc);
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
            }
            crate::policy_dsl::language::Family::Python
            | crate::policy_dsl::language::Family::Php
            | crate::policy_dsl::language::Family::Ecma => {
                let root = language.ast_grep(file.source.as_str());
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
            }
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
                statements,
                &single,
                &BTreeMap::new(),
                &policy.scope,
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
                Some(witnesses)
                    if util_limit.is_none()
                        && partial.is_empty()
                        && seed_metrics.witnesses <= limits.max_witnesses =>
                {
                    let starts = calls::seed_starts(
                        witnesses.iter().map(|witness| &witness.span),
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
    let view = calls::CallsView::build(terms.questions(), summaries.clone(), &seeds);
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
