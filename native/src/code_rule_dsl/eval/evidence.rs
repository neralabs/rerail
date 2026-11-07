//! Cross-file clauses over `evidence` relations: projection of a witness, and the violations of
//! `require_each` and `budget` over the rows every file of the relation's domain produced.

use super::output::{failure, violation_from_witness, violation_id};
use crate::code_rule_dsl::matcher::{exported_span, exported_text};
use crate::code_rule_dsl::model::{
    ClauseKind, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, FileInput, Projection,
    ProjectionSource, Query, ViolationSpec,
};
use crate::code_rule_dsl::origin::InFile;
use crate::code_rule_dsl::query::crossfile::{self, Relation, Requirement};
use crate::code_rule_dsl::tape::tape_doc;
use crate::code_rule_dsl::violations::{self, NodeGroup, Witness};
use ast_grep_core::Doc;
use ast_grep_core::matcher::NodeMatch;
use ast_grep_core::replacer::Content;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// A producer's witnesses across its domain, and the files whose witnesses are unknown.
#[derive(Default)]
pub(crate) struct Rows {
    pub(crate) witnesses: Vec<Witness>,
    /// Files of the domain whose query failed, stopped, or read around syntax errors.
    pub(crate) incomplete: BTreeSet<String>,
}

/// What a producer's rows are to the clause reading them, and so what a comment over one does.
/// A suppression comment silences a reported violation where it is reported; it never removes a
/// fact a violation elsewhere reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// A `require_each`'s subject: each row reports its own violation, which a `rerail-ignore`
    /// over it silences by dropping the row.
    Subject,
    /// A `require_each`'s evidence: facts, which no comment removes.
    Evidence,
    /// A `budget`'s rows: counted whatever the comments; a `rerail-ignore` over one marks it, and
    /// a group's violation reported at a marked row is silenced.
    Counted,
}

/// The violation spec an evidence producer's witnesses use: its projection, enumeration, anchor
/// and same-node regex groups, without fixes, and the comments its `side` answers to.
pub(crate) fn producer_violation(
    clause: &CompiledClause,
    relation: &FactQuery,
    side: Side,
) -> ViolationSpec {
    let mut violation = clause.violation.clone();
    // P's relational inputs (test-only, open item O18) keep only their anchor and typed fields.
    #[cfg(test)]
    {
        violation.input_only = matches!(clause.kind, ClauseKind::Relational(_));
    }
    violation.project = relation.project.to_vec();
    violation.fix = None;
    violation.fix_template = None;
    violation.fix_range = None;
    violation.each = relation.each.clone();
    violation.anchor = relation.anchor.clone();
    // Compiling refuses `suppress` on a relation clause: only `rerail-ignore` reaches its rows.
    violation.suppress = None;
    match side {
        Side::Subject => {}
        Side::Evidence => violation.ignored_by = None,
        Side::Counted => violation.ignore_marks = true,
    }
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

/// Project directly into typed columns; capture text retains a shared source slice.
pub(super) fn project<D: Doc>(
    project: &[Projection],
    file: &FileInput,
    matched: &NodeMatch<'_, D>,
    groups: &[NodeGroup],
) -> violations::projection::Row {
    use std::{borrow::Cow, sync::Arc};
    use violations::projection::{self, Input};
    let env = matched.get_env();
    // A capture's position is the file's at the bytes it stands for (`origin`): an embedded
    // program's ranges are mapped to the file's, and a capture standing for several file ranges
    // reads its own text, never the file's bytes between them.
    let place = InFile::of(matched.get_node()).expect("a witness's document is a file's");
    let capture_range = |capture: &str| {
        env.get_match(capture)
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
                    .map(|(_, _, r)| r.clone())
            })
    };
    let (directory, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
    let source = |range: Option<std::ops::Range<usize>>| {
        range
            .filter(|r| file.source.get(r.clone()).is_some())
            .map_or(Input::Null, |r| Input::Source(&file.source, r))
    };
    // The capture's own text over document range `range`, for a capture that is not one file range.
    let own_text = |capture: &str, range: std::ops::Range<usize>| {
        exported_text(env, capture)
            .or_else(|| {
                groups
                    .iter()
                    .find(|(name, _, _)| name == capture)
                    .map(|(_, own, _)| own.to_string())
            })
            .or_else(|| {
                tape_doc(matched.get_node())
                    .and_then(|doc| doc.get_source().get(range).map(str::to_owned))
            })
    };
    let file_range = |range: std::ops::Range<usize>| place.places(range).primary();
    let capture_source = |capture: &str| match capture_range(capture) {
        None => Input::Null,
        Some(range) => match place.places(range.clone()).single() {
            Some(file) => source(Some(file)),
            None => {
                own_text(capture, range).map_or(Input::Null, |text| Input::Text(Cow::Owned(text)))
            }
        },
    };
    let values = project.iter().map(
        |Projection {
             source: from,
             extract,
             ..
         }| {
            let value = match from {
                ProjectionSource::Capture(capture) => capture_source(capture),
                ProjectionSource::CaptureLine(capture) => capture_range(capture)
                    .map(file_range)
                    .map_or(Input::Null, |r| {
                        Input::Integer(
                            crate::code_rule_dsl::matcher::file_lines(
                                &file.source,
                                crate::code_rule_dsl::lines::Terminators::Newlines,
                            )
                            .span(r.start, r.end)
                            .start_line as i64,
                        )
                    }),
                ProjectionSource::CaptureStart(capture) => capture_range(capture)
                    .map(file_range)
                    .map_or(Input::Null, |r| Input::Integer(r.start as i64)),
                ProjectionSource::CaptureEnd(capture) => capture_range(capture)
                    .map(file_range)
                    .map_or(Input::Null, |r| Input::Integer(r.end as i64)),
                ProjectionSource::CaptureText(capture) => {
                    if let Some(text) = exported_text(env, capture).or_else(|| {
                        env.get_transformed(capture)
                            .map(|bytes| <D::Source as Content>::encode_bytes(bytes).into_owned())
                    }) {
                        Input::Text(Cow::Owned(text))
                    } else if let Some(node) = env.get_match(capture) {
                        let text = node.text();
                        match place.place(node).single() {
                            Some(range)
                                if file.source.get(range.clone()) == Some(text.as_ref()) =>
                            {
                                source(Some(range))
                            }
                            _ => Input::Text(Cow::Owned(text.into_owned())),
                        }
                    } else {
                        capture_source(capture)
                    }
                }
                ProjectionSource::CaptureKind(capture) => env
                    .get_match(capture)
                    .map_or(Input::Null, |n| Input::Text(n.kind())),
                ProjectionSource::Path => Input::Text(Cow::Borrowed(&file.path)),
                ProjectionSource::Directory => Input::Text(Cow::Borrowed(directory)),
                ProjectionSource::Name => Input::Text(Cow::Borrowed(name)),
                ProjectionSource::Stem => Input::Text(Cow::Borrowed(
                    name.split_once('.').map_or(name, |(stem, _)| stem),
                )),
                ProjectionSource::Module => file
                    .module
                    .as_deref()
                    .map_or(Input::Null, |m| Input::Text(Cow::Owned(m.text()))),
                ProjectionSource::Package => file
                    .module
                    .as_deref()
                    .map_or(Input::Null, |m| Input::Text(Cow::Owned(m.package_text()))),
            };
            if let Some((regex, group)) = extract {
                let range = value
                    .text()
                    .and_then(|text| regex.captures(text))
                    .and_then(|c| c.get(*group).map(|m| m.range()));
                range.map_or(Input::Null, |range| value.slice(range))
            } else {
                value
            }
        },
    );
    projection::append(
        (
            project.as_ptr() as usize,
            Arc::as_ptr(&file.source) as usize,
            file.path.as_ptr() as usize,
        ),
        || project.iter().map(|p| p.field.clone()).collect(),
        values,
    )
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
                    .map(violations::projection::Row::object)
                    .unwrap_or_default()
            })
            .collect(),
        incomplete: rows.incomplete.clone(),
    }
}

fn derived_relation(
    query: &FactQuery,
    rows: &Rows,
    code_rule: &CompiledCodeRule,
) -> Result<Relation, crate::code_rule_dsl::query::name_closure::ClosureError> {
    let mut relation = relation(rows);
    crate::code_rule_dsl::query::name_closure::derive(
        &mut relation,
        &query.closure,
        code_rule.capabilities.limits.max_visited_nodes,
    )?;
    Ok(relation)
}

/// The witness as the clause reports it: without its projection.
fn reported(mut witness: Witness, query: &FactQuery) -> Witness {
    witness.span = violations::anchored_span(&query.anchor, &witness).0;
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
    let relations = derived_relation(subject_query, &subject, code_rule).and_then(|subject| {
        derived_relation(evidence_query, evidence, code_rule).map(|evidence| (subject, evidence))
    });
    let (subject_relation, evidence_relation) = match relations {
        Ok(relations) => relations,
        Err(error) => {
            evaluation
                .failures
                .push(failure(&clause.id, error.code(), error.message(), None));
            return evaluation;
        }
    };
    let answers = match crossfile::require_each(
        (&subject_query.alias, &subject_relation),
        subject_query.predicate.as_ref(),
        (&evidence_query.alias, &evidence_relation),
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
                        reported(witness, subject_query),
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
/// witness, with `count` and `subjectId` (the group key), unless a `rerail-ignore` marks that
/// witness (`Side::Counted`; marked rows still count). With some file unknown the counts are
/// lower bounds: one over the maximum is still a violation (reported with `partial`), one within
/// it is unknown, never clean.
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
    let derived = match derived_relation(query, &rows, code_rule) {
        Ok(relation) => relation,
        Err(error) => {
            evaluation
                .failures
                .push(failure(&clause.id, error.code(), error.message(), None));
            return evaluation;
        }
    };
    let groups = match crossfile::budget(
        (&query.alias, &derived),
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
        // The group's violation is reported at its first row: a `rerail-ignore` there silences
        // it, while one over another row leaves the count as it is.
        let Some(witness) = rows
            .witnesses
            .get(group.first)
            .filter(|witness| !witness.ignored)
            .cloned()
        else {
            continue;
        };
        evaluation.metrics.witnesses += 1;
        let mut violation = violation_from_witness(code_rule, clause, reported(witness, query));
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

    /// `(path, start line)` of every violation, after asserting the evaluation failed nowhere.
    fn located(result: &Value) -> Vec<(String, u64)> {
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| {
                let path = violation["path"].as_str().expect("path").to_owned();
                (path, violation["span"]["startLine"].as_u64().expect("line"))
            })
            .collect()
    }

    const IGNORE: &str = "// rerail-ignore evidence-projection -- this node only\n";

    /// A `rerail-ignore` silences a violation where it is reported and never removes a fact:
    /// evidence under one still satisfies its subject, while one over the subject silences the
    /// subject's own violation.
    #[test]
    fn require_each_evidence_under_a_rerail_ignore_still_satisfies_its_subject() {
        let relation = |alias: &str, path: &str| {
            json!({ "evidence": {
                "as": alias,
                "paths": { "include": [path] },
                "query": { "ast": { "language": "typescript", "rule": {
                    "kind": "debugger_statement"
                } } },
                "project": { "name": "file.name" }
            } })
        };
        let clause = json!({
            "id": "require",
            "violation": violation(),
            "require_each": {
                "subject": relation("subject", "subject.ts"),
                "evidence": relation("evidence", "evidence.ts")
            }
        });
        let ignored = format!("{IGNORE}debugger;\n");
        for (evidence, subject, expected) in [
            (ignored.as_str(), "debugger;\n", vec![]),
            (
                "const x = 1;\n",
                "debugger;\n",
                vec![("subject.ts".to_owned(), 0)],
            ),
            ("const x = 1;\n", ignored.as_str(), vec![]),
        ] {
            let result = evaluate(
                clause.clone(),
                json!([
                    { "path": "evidence.ts", "language": "typescript", "source": evidence },
                    { "path": "subject.ts", "language": "typescript", "source": subject },
                ]),
            );
            assert_eq!(located(&result), expected, "{evidence:?} {subject:?}");
        }
    }

    /// A budget counts its rows whatever the comments: a `rerail-ignore` over a row after the
    /// first leaves the group over its maximum and reported (at its first row), and one over the
    /// first row, where the group's violation is reported, silences it.
    #[test]
    fn budget_rows_under_a_rerail_ignore_count_and_only_the_reported_row_is_silenced() {
        let clause = json!({
            "id": "budget",
            "violation": violation(),
            "budget": {
                "query": { "evidence": {
                    "as": "usage",
                    "query": { "ast": { "language": "typescript", "rule": {
                        "kind": "debugger_statement"
                    } } },
                    "project": { "directory": "file.directory" }
                } },
                "group_by": ["usage.directory"],
                "count": { "lte": 1 }
            }
        });
        let ignored = format!("{IGNORE}debugger;\n");
        let reported = vec![("src/a.ts".to_owned(), 0)];
        for (first, second, expected) in [
            ("debugger;\n", "debugger;\n", reported.clone()),
            ("debugger;\n", ignored.as_str(), reported),
            (ignored.as_str(), "debugger;\n", vec![]),
        ] {
            let result = evaluate(
                clause.clone(),
                json!([
                    { "path": "src/a.ts", "language": "typescript", "source": first },
                    { "path": "src/b.ts", "language": "typescript", "source": second },
                ]),
            );
            assert_eq!(located(&result), expected, "{first:?} {second:?}");
            if !expected.is_empty() {
                assert_eq!(result["violations"][0]["variables"]["count"], 2, "{result}");
            }
        }
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
                sql_violation: None,
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
                ignored: false,
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
