    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn ast(id: &str, language: &str, rule: Value) -> Value {
        json!({ "id": id, "forbid": { "ast": { "language": language, "rule": rule } }, "violation": violation() })
    }

    /// Evaluates `clauses` over `files` (path, python source) under limits edited by `limits`.
    fn evaluate(clauses: Value, files: &[(&str, &str)], limits: impl FnOnce(&mut Value)) -> Value {
        let capabilities = capabilities_with(|capabilities| limits(&mut capabilities["limits"]));
        let compiled = compile_document_with(
            json!({ "format": "rerail.policy/v1", "id": "coverage", "revision": 1, "mode": "steer", "clauses": clauses }),
            &capabilities,
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let files = files
            .iter()
            .map(|(path, source)| json!({ "language": "python", "path": path, "source": source }))
            .collect::<Vec<_>>();
        let snapshot = json!({
            "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0,
        });
        let output = compiled
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect("evaluation succeeds");
        serde_json::from_str(&output).expect("evaluation is JSON")
    }

    #[test]
    fn coverage_is_each_clauses_failures_by_reason_class() {
        let broken = "print(3)\ndef (:\n";
        let files = [
            ("d.py", broken),
            ("a.py", broken),
            ("c.py", broken),
            ("b.py", broken),
        ];
        let result = evaluate(
            json!([
                ast(
                    "clean",
                    "typescript",
                    json!({ "kind": "debugger_statement" })
                ),
                ast("partial", "python", json!({ "kind": "lambda" })),
                ast(
                    "incomplete",
                    "python",
                    json!({ "kind": "call", "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } }),
                ),
                ast("failed", "python", json!({ "kind": "call" })),
            ]),
            &files,
            // `failed` reports one call per file, past the witness budget.
            |limits| limits["maxWitnesses"] = json!(2),
        );
        assert_eq!(result["status"], "failed", "{result}");
        let paths = json!(["a.py", "b.py", "c.py"]);
        assert_eq!(
            result["coverage"],
            json!([
                { "clauseId": "clean", "status": "complete", "reasons": [] },
                { "clauseId": "partial", "status": "partial", "reasons": [
                    { "code": "source-partial", "count": 4, "paths": paths },
                ] },
                { "clauseId": "incomplete", "status": "partial", "reasons": [
                    // A query whose source is incomplete names the first such file it read.
                    { "code": "source-incomplete", "count": 1, "paths": ["d.py"] },
                ] },
                { "clauseId": "failed", "status": "failed", "reasons": [
                    { "code": "source-partial", "count": 4, "paths": paths },
                    { "code": "witness-limit", "count": 1, "paths": [] },
                ] },
            ]),
            "{result}"
        );
    }

    #[test]
    fn a_failure_naming_no_evaluated_clause_covers_every_clause() {
        // Over the file budget, the `runtime` failure names no clause: every clause failed.
        let result = evaluate(
            json!([
                ast("first", "python", json!({ "kind": "call" })),
                ast(
                    "second",
                    "typescript",
                    json!({ "kind": "debugger_statement" })
                ),
            ]),
            &[("a.py", "f()\n"), ("b.py", "g()\n")],
            |limits| limits["maxFiles"] = json!(1),
        );
        let reason = json!([{ "code": "budget-exceeded", "count": 1, "paths": [] }]);
        assert_eq!(
            result["coverage"],
            json!([
                { "clauseId": "first", "status": "failed", "reasons": reason },
                { "clauseId": "second", "status": "failed", "reasons": reason },
            ]),
            "{result}"
        );
    }
