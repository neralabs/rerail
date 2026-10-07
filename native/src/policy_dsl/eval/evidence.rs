//! Cross-file checks over `evidence` relations: projection of a witness, and the findings of
//! `require_each` and `budget` over the rows every file of the relation's domain produced.

use super::output::{failure, finding_from_witness, finding_id};
use crate::policy_dsl::findings::{self, NodeGroup, Witness};
use crate::policy_dsl::matcher::exported_span;
use crate::policy_dsl::model::{
    CheckKind, CompiledCheck, CompiledPolicy, Evaluation, FactQuery, FileInput, FindingSpec,
    Projection, ProjectionSource, Query,
};
use crate::policy_dsl::query::crossfile::{self, Relation, Requirement};
use ast_grep_core::Doc;
use ast_grep_core::matcher::NodeMatch;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// The key under which a witness of an evidence producer carries its projected row.
pub(crate) const PROJECTION: &str = "$project";

/// A producer's witnesses across its domain, and the files whose witnesses are unknown.
#[derive(Default)]
pub(crate) struct Rows {
    pub(crate) witnesses: Vec<Witness>,
    /// Files of the domain whose query failed, stopped, or read around syntax errors.
    pub(crate) incomplete: BTreeSet<String>,
}

/// The finding spec an evidence producer's witnesses use: the check's projection and same-node
/// regex groups, without fixes or `each` (a relation row is one match).
pub(crate) fn producer_finding(check: &CompiledCheck, relation: &FactQuery) -> FindingSpec {
    let mut finding = check.finding.clone();
    finding.project = relation.ast.project.clone();
    finding.fix = None;
    finding.fix_range = None;
    finding.each = None;
    let Query::Ast { matchers, .. } = &relation.ast.query else {
        unreachable!("evidence relations always use ast queries");
    };
    finding.node_regexes = matchers
        .iter()
        .filter_map(|grammar| {
            let regexes = findings::witness_regexes(&grammar.matcher, &grammar.utils);
            (!regexes.is_empty()).then_some((grammar.language, regexes))
        })
        .collect();
    finding
}

/// The fields `project` names for one match in `file`.
pub(super) fn project<D: Doc>(
    project: &[Projection],
    file: &FileInput,
    matched: &NodeMatch<'_, D>,
    groups: &[NodeGroup],
) -> Value {
    let env = matched.get_env();
    let (directory, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
    let module = file.module.as_deref();
    let fields = project
        .iter()
        .map(|Projection { field, source }| {
            let value = match source {
                ProjectionSource::Capture(capture) => {
                    let range = env
                        .get_match(capture)
                        .map(|node| node.range())
                        .or_else(|| {
                            let nodes = env.get_multiple_matches(capture);
                            Some(nodes.first()?.range().start..nodes.last()?.range().end)
                        })
                        .or_else(|| exported_span(env, capture))
                        .or_else(|| {
                            groups
                                .iter()
                                .find(|(name, _, _)| name == capture)
                                .map(|(_, _, range)| range.clone())
                        });
                    range
                        .and_then(|range| file.source.get(range))
                        .map_or(Value::Null, |text| Value::String(text.to_owned()))
                }
                ProjectionSource::Path => Value::String(file.path.clone()),
                ProjectionSource::Directory => Value::String(directory.to_owned()),
                ProjectionSource::Name => Value::String(name.to_owned()),
                ProjectionSource::Stem => Value::String(
                    name.split_once('.')
                        .map_or(name, |(stem, _)| stem)
                        .to_owned(),
                ),
                ProjectionSource::Module => module
                    .map(|module| Value::String(module.text()))
                    .unwrap_or(Value::Null),
                ProjectionSource::Package => module
                    .map(|module| Value::String(module.package_text()))
                    .unwrap_or(Value::Null),
            };
            (field.clone(), value)
        })
        .collect::<Map<_, _>>();
    Value::Object(fields)
}

fn relation(rows: &Rows) -> Relation {
    Relation {
        rows: rows
            .witnesses
            .iter()
            .map(|witness| {
                witness
                    .captures
                    .get(PROJECTION)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect(),
        incomplete: rows.incomplete.clone(),
    }
}

/// The witness as the check reports it: without its projection.
fn reported(mut witness: Witness) -> Witness {
    if let Some(captures) = witness.captures.as_object_mut() {
        captures.remove(PROJECTION);
    }
    witness
}

fn incomplete(check: &CompiledCheck, paths: &BTreeSet<String>, message: &str) -> Vec<Value> {
    paths
        .iter()
        .map(|path| failure(&check.id, "source-incomplete", message, Some(path)))
        .collect()
}

/// `require_each` over evidence: a subject without evidence is a finding at the subject's
/// witness when the evidence is complete, and unknown (`source-incomplete`) when it is not. Subjects
/// in files whose rows are unknown are unknown too.
pub(crate) fn require_each(
    policy: &CompiledPolicy,
    check: &CompiledCheck,
    (subject_query, subject): (&FactQuery, Rows),
    (evidence_query, evidence): (&FactQuery, &Rows),
) -> Evaluation {
    let mut evaluation = Evaluation::default();
    evaluation.failures.extend(incomplete(
        check,
        &subject.incomplete,
        "subject evidence in this file is incomplete",
    ));
    let answers = match crossfile::require_each(
        (&subject_query.alias, &relation(&subject)),
        subject_query.predicate.as_ref(),
        (&evidence_query.alias, &relation(evidence)),
        evidence_query.predicate.as_ref(),
        crate::policy_dsl::budget::matcher_budget_exhausted,
    ) {
        Ok(answers) => answers,
        Err(error) => {
            evaluation.failures.push(failure(
                &check.id,
                "runtime",
                &format!("cross-file statement failed: {error}"),
                None,
            ));
            return evaluation;
        }
    };
    let mut witnesses = subject.witnesses.into_iter().map(Some).collect::<Vec<_>>();
    for answer in answers {
        match answer {
            Requirement::Missing(row) => {
                if let Some(witness) = witnesses[row].take() {
                    evaluation.metrics.witnesses += 1;
                    evaluation.findings.push(finding_from_witness(
                        policy,
                        check,
                        reported(witness),
                    ));
                }
            }
            Requirement::Unknown(row) => {
                if let Some(witness) = &witnesses[row] {
                    evaluation.failures.push(failure(
                        &check.id,
                        "source-incomplete",
                        &format!(
                            "evidence is incomplete in {}",
                            evidence
                                .incomplete
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        Some(&witness.path),
                    ));
                }
            }
        }
    }
    evaluation
}

/// `budget` over evidence: a group whose count is over the maximum is a finding at its first
/// witness, with `count` and `subjectId` (the group key). With some file unknown the counts are
/// lower bounds: one over the maximum is still a finding (reported with `partial`), one within it
/// is unknown, never clean.
pub(crate) fn budget(
    policy: &CompiledPolicy,
    check: &CompiledCheck,
    query: &FactQuery,
    rows: Rows,
) -> Evaluation {
    let CheckKind::Budget {
        group_by,
        maximum,
        distinct,
        ..
    } = &check.kind
    else {
        unreachable!("budget checks only");
    };
    let mut evaluation = Evaluation::default();
    let groups = match crossfile::budget(
        (&query.alias, &relation(&rows)),
        query.predicate.as_ref(),
        group_by,
        distinct,
        *maximum,
        crate::policy_dsl::budget::matcher_budget_exhausted,
    ) {
        Ok(groups) => groups,
        Err(error) => {
            evaluation.failures.push(failure(
                &check.id,
                "runtime",
                &format!("cross-file statement failed: {error}"),
                None,
            ));
            return evaluation;
        }
    };
    let unknown = rows
        .incomplete
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    for group in groups {
        if group.count <= *maximum {
            evaluation.failures.push(failure(
                &check.id,
                "source-incomplete",
                &format!(
                    "count {} of `{}` is a lower bound: evidence is incomplete in {unknown}",
                    group.count, group.key
                ),
                rows.witnesses
                    .get(group.first)
                    .map(|witness| witness.path.as_str()),
            ));
            continue;
        }
        let Some(witness) = rows.witnesses.get(group.first).cloned() else {
            continue;
        };
        evaluation.metrics.witnesses += 1;
        let mut finding = finding_from_witness(policy, check, reported(witness));
        // A group is identified by its key, not by whichever witness is first today, so its
        // finding keeps its id as usages come and go and never collides with another group's.
        finding["id"] = Value::String(finding_id(
            &policy.id,
            &check.id,
            &check.finding.code,
            &format!("group\0{}", group.key),
            0,
            0,
        ));
        let variables = finding["variables"]
            .as_object_mut()
            .expect("finding variables");
        variables.insert("count".to_owned(), json!(group.count));
        variables.insert("subjectId".to_owned(), Value::String(group.key.clone()));
        if group.partial {
            variables.insert("partial".to_owned(), Value::Bool(true));
        }
        evaluation.findings.push(finding);
    }
    evaluation
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::{PolicyDslRuntime, test_support::CAPABILITIES};
    use serde_json::{Value, json};

    fn evaluate(check: Value, files: Value) -> Value {
        let policy = json!({
            "format": "rerail.policy/v1",
            "id": "evidence-projection",
            "revision": 1,
            "checks": [check],
        });
        let compiled = PolicyDslRuntime::compile(
            policy.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        )
        .expect("policy compiles");
        let output = compiled
            .evaluate(
                json!({
                    "format": "rerail.repository-snapshot/v1",
                    "revision": 1,
                    "facts": [],
                    "files": files,
                })
                .to_string(),
                "diagnostic".to_owned(),
            )
            .expect("evaluation succeeds");
        serde_json::from_str(&output).expect("evaluation JSON")
    }

    fn finding() -> Value {
        json!({
            "anchor": "witness.span",
            "code": "E",
            "message": "evidence",
            "severity": "error",
        })
    }

    #[test]
    fn require_each_distinguishes_same_node_regex_group_projections() {
        let result = evaluate(
            json!({
                "id": "named-comment-evidence",
                "finding": finding(),
                "require_each": {
                    "subject": { "evidence": {
                        "as": "subject",
                        "paths": { "include": ["subject.ts"] },
                        "query": { "ast": { "language": "typescript", "rule": {
                            "kind": "comment", "regex": "^//(?<NAME>.+)$"
                        } } },
                        "project": { "name": "capture.NAME" }
                    } },
                    "evidence": { "evidence": {
                        "as": "evidence",
                        "paths": { "include": ["evidence.ts"] },
                        "query": { "ast": { "language": "typescript", "rule": {
                            "kind": "comment", "regex": "^//(?<NAME>.+)$"
                        } } },
                        "project": { "name": "capture.NAME" },
                        "where": { "eq": ["subject.name", "evidence.name"] }
                    } }
                }
            }),
            json!([
                { "path": "subject.ts", "language": "typescript", "source": "// subject\n" },
                { "path": "evidence.ts", "language": "typescript", "source": "// evidence\n" },
            ]),
        );
        assert_eq!(
            result["findings"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
    }

    #[test]
    fn budget_groups_dotted_module_segments_by_structural_package() {
        let result = evaluate(
            json!({
                "id": "module-package",
                "finding": finding(),
                "budget": {
                    "query": { "evidence": {
                        "as": "usage",
                        "query": { "ast": { "language": "python", "rule": {
                            "kind": "expression_statement"
                        } } },
                        "project": { "package": "file.package" }
                    } },
                    "group_by": ["usage.package"],
                    "count": { "lte": 0 }
                }
            }),
            json!([{
                "path": "pkg/my.mod.py",
                "language": "python",
                "module": ["pkg", "my.mod"],
                "source": "run()\n"
            }]),
        );
        assert_eq!(
            result["findings"][0]["variables"]["subjectId"], "\"pkg\"",
            "{result}"
        );
    }

    /// A cross-file statement polls the evaluating thread's budget: past it, the check fails
    /// instead of answering late, and the poll records the overrun for the caller to report.
    #[test]
    fn cross_file_statements_stop_at_the_evaluation_budget() {
        use crate::policy_dsl::budget::MatchBudget;
        use crate::policy_dsl::eval::evidence::{Rows, budget};
        use crate::policy_dsl::findings::Witness;
        use crate::policy_dsl::model::CheckKind;
        let policy = json!({
            "format": "rerail.policy/v1",
            "id": "evidence-budget",
            "revision": 1,
            "checks": [{
                "id": "usage-budget",
                "finding": finding(),
                "budget": {
                    "query": { "evidence": {
                        "as": "usage",
                        "query": { "ast": { "language": "python", "rule": {
                            "kind": "expression_statement"
                        } } },
                        "project": { "package": "file.package" }
                    } },
                    "group_by": ["usage.package"],
                    "count": { "lte": 0 }
                }
            }],
        });
        let compiled = PolicyDslRuntime::compile(
            policy.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        )
        .expect("policy compiles");
        let check = &compiled.inner.checks[0];
        let CheckKind::Budget { query, .. } = &check.kind else {
            panic!("expected a budget check");
        };
        let rows = || Rows {
            witnesses: vec![Witness {
                path: "pkg/a.py".to_owned(),
                span: json!({ "startByte": 0, "endByte": 5 }),
                captures: json!({ (super::PROJECTION): { "package": "pkg" } }),
                message: None,
                fix: None,
            }],
            incomplete: Default::default(),
        };
        let within = budget(&compiled.inner, check, query, rows());
        assert_eq!(within.findings.len(), 1);

        let expired = MatchBudget::expired_for_test();
        let late = budget(&compiled.inner, check, query, rows());
        assert!(late.findings.is_empty(), "a late statement answered");
        assert_eq!(late.failures.len(), 1);
        assert_eq!(late.failures[0]["code"], "runtime", "{:?}", late.failures);
        assert!(expired.observed(), "the overrun was not recorded");
    }
}
