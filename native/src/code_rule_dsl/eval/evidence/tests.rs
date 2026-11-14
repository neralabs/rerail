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
                span: Some(crate::code_rule_dsl::violations::Span {
                    start_byte: 0,
                    end_byte: 5,
                    ..Default::default()
                }),
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
