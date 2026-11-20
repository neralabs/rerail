    use super::*;
    use crate::code_rule_dsl::CodeRuleDslRuntime;

    /// Evaluates `source` as `a.ts` under the one clause `clause` of a code rule `bounded-each`,
    /// with a one-witness allowance.
    fn evaluate_bounded(clause: Value, source: &str) -> Value {
        let mut capabilities: Value =
            serde_json::from_str(include_str!("../capabilities.json")).expect("capabilities");
        capabilities["limits"]["maxWitnesses"] = json!(1);
        let compiled = CodeRuleDslRuntime::compile(
            json!({
                "format": "rerail.policy/v1",
                "id": "bounded-each",
                "revision": 1,
                "mode": "steer",
                "clauses": [clause]
            })
            .to_string(),
            "json".to_owned(),
            capabilities.to_string(),
        )
        .expect("policy compiles");
        let output = compiled
            .evaluate(
                json!({
                    "format": "rerail.repository-snapshot/v1",
                    "revision": 1,
                    "facts": [],
                    "files": [{
                        "path": "a.ts",
                        "language": "typescript",
                        "source": source,
                    }]
                })
                .to_string(),
                "report".to_owned(),
            )
            .expect("evaluation succeeds");
        serde_json::from_str(&output).expect("evaluation JSON")
    }

    /// One witness per `a` of a comment, with `suppress` where given.
    fn comment_characters(suppress: Option<Value>) -> Value {
        let mut violation = json!({
            "anchor": "capture.X",
            "code": "E",
            "message": "$X",
            "each": "X"
        });
        if let Some(suppress) = suppress {
            violation["suppress"] = suppress;
        }
        json!({
            "id": "comment-characters",
            "forbid": { "ast": { "language": "typescript", "rule": {
                "kind": "comment",
                "regex": "(?<X>a)"
            } } },
            "violation": violation
        })
    }

    #[test]
    fn same_node_regex_each_stops_after_the_witness_overflow_sentinel() {
        let source = format!("//{}\n", "a".repeat(50_000));
        reset_node_group_enumerations();
        let result = evaluate_bounded(comment_characters(None), &source);
        assert_eq!(node_group_enumerations(), 2, "{result}");
        assert_eq!(result["metrics"]["witnesses"], 2, "{result}");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(result["failures"][0]["code"], "witness-limit", "{result}");
    }

    /// A violation whose `suppress` a comment may answer reads past suppressed witnesses, yet
    /// still stops once the unsuppressed ones overflow the allowance: none of these 50,000 groups
    /// is suppressed, so the second ends the enumeration.
    #[test]
    fn suppressible_same_node_regex_each_stops_after_the_witness_overflow_sentinel() {
        let source = format!("//{}\n", "a".repeat(50_000));
        let suppress = json!({ "eslint": { "rules": ["no-warning-comments"] } });
        reset_node_group_enumerations();
        let result = evaluate_bounded(comment_characters(Some(suppress)), &source);
        assert_eq!(node_group_enumerations(), 2, "{result}");
        assert_eq!(result["metrics"]["witnesses"], 2, "{result}");
        assert_eq!(result["violations"], json!([]), "{result}");
        assert_eq!(result["failures"][0]["code"], "witness-limit", "{result}");
    }

    /// A binding a comment suppresses spends none of the one-witness allowance: the enumeration
    /// reads past two suppressed bindings to the third, which is reported, whether a
    /// `rerail-ignore` or a linter directive the violation opts into silences them.
    #[test]
    fn each_reads_past_suppressed_bindings_under_a_one_witness_allowance() {
        for (directive, suppress) in [
            ("// rerail-ignore bounded-each", None),
            (
                "// eslint-disable-line no-console",
                Some(json!({ "eslint": { "rules": ["no-console"] } })),
            ),
        ] {
            let mut violation = json!({
                "anchor": "capture.X",
                "code": "E",
                "message": "statement",
                "each": "X"
            });
            if let Some(suppress) = suppress {
                violation["suppress"] = suppress;
            }
            // Statements only: `$X` alone also binds the comments, which no directive covers.
            let clause = json!({
                "id": "statement",
                "forbid": { "ast": { "language": "typescript", "rule": {
                    "kind": "program",
                    "has": { "kind": "expression_statement", "pattern": "$X" }
                } } },
                "violation": violation
            });
            let source = format!("first; {directive}\nsecond; {directive}\nthird;\n");
            let result = evaluate_bounded(clause, &source);
            assert_eq!(result["failures"], json!([]), "{directive}: {result}");
            let reported = result["violations"]
                .as_array()
                .expect("violations")
                .iter()
                .map(|violation| {
                    let start = violation["span"]["startByte"].as_u64().expect("start");
                    let end = violation["span"]["endByte"].as_u64().expect("end");
                    &source[start as usize..end as usize]
                })
                .collect::<Vec<_>>();
            assert_eq!(reported, ["third;"], "{directive}: {result}");
        }
    }
