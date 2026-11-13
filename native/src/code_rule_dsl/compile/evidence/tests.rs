    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    #[test]
    fn literal_comparison_operands_do_not_name_relation_fields() {
        let document = json!({
            "format": "rerail.policy/v1",
            "id": "literal-operands",
            "revision": 1,
            "mode": "steer",
            "clauses": [{
                "id": "relation",
                "violation": violation(),
                "require_each": {
                    "subject": { "evidence": {
                        "as": "row",
                        "query": { "ast": { "language": "typescript", "rule": { "kind": "identifier" } } },
                        "project": { "name": "file.name" }
                    } },
                    "evidence": { "evidence": {
                        "as": "other",
                        "query": { "ast": { "language": "typescript", "rule": { "kind": "identifier" } } },
                        "project": { "name": "file.name" },
                        "where": { "eq": [["row.missing"], ["row.missing"]] }
                    } }
                }
            }]
        });
        compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
    }
