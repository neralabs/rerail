//! Cross-file clauses over `evidence` relations: projection of a witness, and the violations of
//! `require_each` and `budget` over the rows every file of the relation's domain produced.

use super::output::{failure, violation_from_witness, violation_id};
use crate::code_rule_dsl::matcher::exported_span;
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, FileInput, Projection,
    ProjectionSource, Query, ViolationSpec,
};
use crate::code_rule_dsl::query::crossfile::{self, Relation, Requirement};
use crate::code_rule_dsl::violations::{self, NodeGroup, Witness};
use ast_grep_core::Doc;
use ast_grep_core::matcher::NodeMatch;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// A producer's witnesses across its domain, and the files whose witnesses are unknown.
#[derive(Default)]
pub(crate) struct Rows {
    pub(crate) witnesses: Vec<Witness>,
    /// Files of the domain whose query failed, stopped, or read around syntax errors.
    pub(crate) incomplete: BTreeSet<String>,
}

/// The violation spec an evidence producer's witnesses use: the clause's projection and same-node
/// regex groups, without fixes or `each` (a relation row is one match).
pub(crate) fn producer_violation(clause: &CompiledClause, relation: &FactQuery) -> ViolationSpec {
    let mut violation = clause.violation.clone();
    violation.project = relation.ast.project.clone();
    violation.fix = None;
    violation.fix_range = None;
    violation.each = None;
    let Query::Ast { matchers, .. } = &relation.ast.query else {
        unreachable!("evidence relations always use ast queries");
    };
    violation.node_regexes = matchers
        .iter()
        .filter_map(|grammar| {
            let regexes = violations::witness_regexes(&grammar.matcher, &grammar.utils);
            (!regexes.is_empty()).then_some((grammar.language, regexes))
        })
        .collect();
    violation
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
                    .projection()
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect(),
        incomplete: rows.incomplete.clone(),
    }
}

/// The witness as the clause reports it: without its projection.
fn reported(mut witness: Witness) -> Witness {
    witness.captures.set_projection(None);
    witness
}

fn incomplete(clause: &CompiledClause, paths: &BTreeSet<String>, message: &str) -> Vec<Value> {
    paths
        .iter()
        .map(|path| failure(&clause.id, "source-incomplete", message, Some(path)))
        .collect()
}

/// `require_each` over evidence: a subject without evidence is a violation at the subject's
/// witness when the evidence is complete, and unknown (`source-incomplete`) when it is not. Subjects
/// in files whose rows are unknown are unknown too.
pub(crate) fn require_each(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    (subject_query, subject): (&FactQuery, Rows),
    (evidence_query, evidence): (&FactQuery, &Rows),
) -> Evaluation {
    let mut evaluation = Evaluation::default();
    evaluation.failures.extend(incomplete(
        clause,
        &subject.incomplete,
        "subject evidence in this file is incomplete",
    ));
    let answers = match crossfile::require_each(
        (&subject_query.alias, &relation(&subject)),
        subject_query.predicate.as_ref(),
        (&evidence_query.alias, &relation(evidence)),
        evidence_query.predicate.as_ref(),
        crate::code_rule_dsl::budget::matcher_budget_exhausted,
    ) {
        Ok(answers) => answers,
        Err(crossfile::Interrupted) => {
            evaluation.failures.push(failure(
                &clause.id,
                "runtime",
                "cross-file check interrupted",
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
                    evaluation.violations.push(violation_from_witness(
                        code_rule,
                        clause,
                        reported(witness),
                    ));
                }
            }
            Requirement::Unknown(row) => {
                if let Some(witness) = &witnesses[row] {
                    evaluation.failures.push(failure(
                        &clause.id,
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

/// `budget` over evidence: a group whose count is over the maximum is a violation at its first
/// witness, with `count` and `subjectId` (the group key). With some file unknown the counts are
/// lower bounds: one over the maximum is still a violation (reported with `partial`), one within it
/// is unknown, never clean.
pub(crate) fn budget(
    code_rule: &CompiledCodeRule,
    clause: &CompiledClause,
    query: &FactQuery,
    rows: Rows,
) -> Evaluation {
    let ClauseKind::Budget {
        group_by,
        maximum,
        distinct,
        ..
    } = &clause.kind
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
        crate::code_rule_dsl::budget::matcher_budget_exhausted,
    ) {
        Ok(groups) => groups,
        Err(crossfile::Interrupted) => {
            evaluation.failures.push(failure(
                &clause.id,
                "runtime",
                "cross-file check interrupted",
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
                &clause.id,
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
        let mut violation = violation_from_witness(code_rule, clause, reported(witness));
        // A group is identified by its key, not by whichever witness is first today, so its
        // violation keeps its id as usages come and go and never collides with another group's.
        violation["id"] = Value::String(violation_id(
            &code_rule.id,
            &clause.id,
            &clause.violation.code,
            &format!("group\0{}", group.key),
            0,
            0,
        ));
        let variables = violation["variables"]
            .as_object_mut()
            .expect("finding variables");
        variables.insert("count".to_owned(), json!(group.count));
        variables.insert("subjectId".to_owned(), Value::String(group.key.clone()));
        if group.partial {
            variables.insert("partial".to_owned(), Value::Bool(true));
        }
        evaluation.violations.push(violation);
    }
    evaluation
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
    use serde_json::{Value, json};

    fn evaluate(clause: Value, files: Value) -> Value {
        let code_rule = json!({
            "format": "rerail.policy/v1",
            "id": "evidence-projection",
            "revision": 1,
            "mode": "steer",
            "clauses": [clause],
        });
        let compiled = CodeRuleDslRuntime::compile(
            code_rule.to_string(),
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
                "report".to_owned(),
            )
            .expect("evaluation succeeds");
        serde_json::from_str(&output).expect("evaluation JSON")
    }

    fn violation() -> Value {
        json!({
            "anchor": "witness.span",
            "code": "E",
            "message": "evidence",
        })
    }

    #[test]
    fn require_each_distinguishes_same_node_regex_group_projections() {
        let result = evaluate(
            json!({
                "id": "named-comment-evidence",
                "violation": violation(),
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
            result["violations"].as_array().map(Vec::len),
            Some(1),
            "{result}"
        );
    }

    #[test]
    fn budget_groups_dotted_module_segments_by_structural_package() {
        let result = evaluate(
            json!({
                "id": "module-package",
                "violation": violation(),
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
            result["violations"][0]["variables"]["subjectId"], "\"pkg\"",
            "{result}"
        );
    }

    /// A cross-file clause polls the evaluating thread's budget: past it, the clause fails
    /// instead of answering late, and the poll records the overrun for the caller to report.
    #[test]
    fn cross_file_clauses_stop_at_the_evaluation_budget() {
        use crate::code_rule_dsl::budget::MatchBudget;
        use crate::code_rule_dsl::eval::evidence::{Rows, budget};
        use crate::code_rule_dsl::model::ClauseKind;
        use crate::code_rule_dsl::violations::Witness;
        let code_rule = json!({
            "format": "rerail.policy/v1",
            "id": "evidence-budget",
            "revision": 1,
            "mode": "steer",
            "clauses": [{
                "id": "usage-budget",
                "violation": violation(),
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
        let compiled = CodeRuleDslRuntime::compile(
            code_rule.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        )
        .expect("policy compiles");
        let clause = &compiled.inner.clauses[0];
        let ClauseKind::Budget { query, .. } = &clause.kind else {
            panic!("expected a budget check");
        };
        let rows = || Rows {
            witnesses: vec![Witness {
                path: "pkg/a.py".to_owned(),
                span: crate::code_rule_dsl::violations::Span {
                    start_byte: 0,
                    end_byte: 5,
                    ..Default::default()
                },
                captures: {
                    let mut captures = crate::code_rule_dsl::violations::Captures::default();
                    captures.set_projection(Some(json!({ "package": "pkg" })));
                    captures
                },
                message: None,
                fix: None,
            }],
            incomplete: Default::default(),
        };
        let within = budget(&compiled.inner, clause, query, rows());
        assert_eq!(within.violations.len(), 1);

        let expired = MatchBudget::expired_for_test();
        let late = budget(&compiled.inner, clause, query, rows());
        assert!(late.violations.is_empty(), "a late check answered");
        assert_eq!(late.failures.len(), 1);
        assert_eq!(late.failures[0]["code"], "runtime", "{:?}", late.failures);
        assert!(expired.observed(), "the overrun was not recorded");
    }
}
