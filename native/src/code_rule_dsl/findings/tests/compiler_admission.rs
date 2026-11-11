//! Admission of the actual catalog JSON through the generic compiler. No executor is substituted.
use super::POLICIES;
use crate::code_rule_dsl::{language::GrammarDialect, matcher::gated_match_tree, model::{ClauseKind, Query}};
use serde_json::{Value, json};

pub(super) fn probe() -> Value {
    let mut policies = Vec::new();
    let mut rows = Vec::new();
    let mut offered = 0usize;
    let mut admitted = 0usize;
    for text in POLICIES {
        let document: Value = serde_json::from_str(text).expect("catalog policy JSON");
        let policy = document["id"].as_str().expect("policy id").to_owned();
        let authored = document["clauses"].as_array().expect("catalog clauses");
        let compiled = CodeRuleDslRuntime::compile(
            text.to_owned(), "json".to_owned(), CAPABILITIES.to_owned(),
        );
        match compiled {
            Err(error) => {
                for clause in authored {
                    offered += 1;
                    rows.push(json!({"policy": policy, "clause": clause["id"],
                        "code": clause["violation"]["code"], "admitted": false,
                        "stage": "policy_compile", "reason": error.to_string()}));
                }
            }
            Ok(compiled) => {
                for authored_clause in authored {
                    offered += 1;
                    let id = authored_clause["id"].as_str().expect("clause id");
                    let clause = compiled.inner.clauses.iter().find(|c| c.id == id)
                        .expect("compiled catalog clause");
                    let mut row = json!({"policy": policy, "clause": id,
                        "code": authored_clause["violation"]["code"], "admitted": false});
                    match &clause.kind {
                        ClauseKind::Forbid(Query::Ast { matchers, .. }) => {
                            if let Some(grammar) = matchers.iter().find(|g| g.language == GrammarDialect::Python) {
                                match gated_match_tree(grammar) {
                                    Err(reason) => {
                                        row["stage"] = json!("gated_match_tree");
                                        row["reason"] = json!(reason.0);
                                    }
                                    Ok((tree, gates)) => {
                                        row["documentGates"] = json!(gates.len());
                                        match LoweredMatcher::lower(&tree) {
                                            Err(reason) => {
                                                row["stage"] = json!("packed_matcher_lower");
                                                row["reason"] = json!(format!("{reason:?}"));
                                            }
                                            Ok(lowered) => {
                                                admitted += 1;
                                                row["admitted"] = json!(true);
                                                row["stage"] = json!("packed_matcher_lower");
                                                row["labelPasses"] = json!(lowered.passes());
                                            }
                                        }
                                    }
                                }
                            } else {
                                row["stage"] = json!("grammar_selection");
                                row["reason"] = json!("no Python grammar");
                            }
                        }
                        _ => {
                            row["stage"] = json!("clause_selection");
                            row["reason"] = json!("not an AST forbid clause");
                        }
                    }
                    rows.push(row);
                }
                policies.push((policy, compiled));
            }
        }
    }
    let integrated = match Rules::lower(&policies, ExecutionBackend::CpuJit) {
        Ok(rules) => json!({"status": "lowered", "clauses": rules.clauses()}),
        Err(error) => json!({"status": "error", "reason": format!("{error:?}")}),
    };
    json!({"schema": "rerail-async-generic-compiler-admission-v1",
        "offered": offered, "admitted": admitted, "refused": offered - admitted,
        "completeFamilyAdmitted": offered == 4 && admitted == offered,
        "clauses": rows, "integrated": integrated,
        "executionMeasured": false,
        "semanticProducerPolicy": "Accepted semantic leaves use native fact producers; no whole-rule executor is substituted."})
}

#[test]
fn catalog_async_compiler_admission_is_accounted_for() {
    let report = probe();
    if let Ok(path) = std::env::var("RERAIL_ASYNC_ADMISSION_REPORT") {
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
    let offered = report["offered"].as_u64().unwrap();
    let admitted = report["admitted"].as_u64().unwrap();
    let refused = report["refused"].as_u64().unwrap();
    assert_eq!(offered, 4, "{report}");
    assert_eq!(offered, admitted + refused, "{report}");
    assert_eq!(report["clauses"].as_array().unwrap().len() as u64, offered);
    if report["integrated"]["status"] == "lowered" {
        assert_eq!(report["integrated"]["clauses"].as_u64().unwrap(), admitted, "{report}");
    } else {
        panic!("integrated compiler failed: {report}");
    }
}
