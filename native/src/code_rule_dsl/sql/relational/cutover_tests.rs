//! Catalog gates for replacing the main compiler. Refusals remain explicit failures.
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    model::{ClauseKind, Query},
    test_support::CAPABILITIES,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

#[path = "catalog_execution_tests.rs"]
mod execution;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn panic_reason(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "non-string panic".into())
}

fn catalog(execute: bool) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut paths = std::fs::read_dir(root.join("packs/python/rules"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    let fixtures = execute.then(|| execution::fixtures(root));
    let mut rows = Vec::new();
    let mut policies = Vec::new();
    let mut refusals = BTreeMap::<String, usize>::new();
    let mut policy_errors = 0;
    let mut total = 0;
    let mut admitted = 0;
    let mut parity_passed = 0;
    let mut parity_failed = 0;
    let mut no_positive_witness = 0;
    for path in paths {
        let source = std::fs::read_to_string(&path).unwrap();
        let relative = path.strip_prefix(root).unwrap().to_string_lossy();
        let sha = digest(source.as_bytes());
        policies.push(json!({"path":relative,"sha256":sha}));
        let policy = match CodeRuleDslRuntime::compile(
            source.clone(),
            "json".into(),
            CAPABILITIES.into(),
        ) {
            Ok(policy) => policy,
            Err(error) => {
                policy_errors += 1;
                rows.push(json!({"policy":relative,"policySha256":sha,"stage":"policy compilation","reason":error.to_string()}));
                continue;
            }
        };
        for clause in &policy.inner.clauses {
            let mut grammars = Vec::new();
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for (index, grammar) in matchers.iter().enumerate() {
                    total += 1;
                    let compiled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        super::compile(grammar)
                    }))
                    .unwrap_or_else(|payload| {
                        Err(format!("compiler panic: {}", panic_reason(payload)))
                    });
                    let result = match compiled {
                        Ok(compiled) => {
                            admitted += 1;
                            if let Ok(directory) =
                                std::env::var("RERAIL_RELATIONAL_CUTOVER_SQL_DIR")
                            {
                                std::fs::create_dir_all(&directory).unwrap();
                                std::fs::write(
                                    Path::new(&directory)
                                        .join(format!("{}.sql", digest(compiled.sql.as_bytes()))),
                                    &compiled.sql,
                                )
                                .unwrap();
                            }
                            let mut result = json!({"admitted":true,"sqlBytes":compiled.sql.len(),"sqlSha256":digest(compiled.sql.as_bytes()),"compilerLimitations":compiled.relations["limitations"]});
                            if let Some(fixtures) = &fixtures {
                                let checked =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        execution::check(grammar, compiled, fixtures)
                                    }));
                                let checked = match checked {
                                    Ok(Ok(checked)) => checked,
                                    Ok(Err(error)) => json!({"passed":false,"error":error}),
                                    Err(payload) => {
                                        json!({"passed":false,"error":format!("execution panic: {}",panic_reason(payload))})
                                    }
                                };
                                if checked["passed"] == true {
                                    parity_passed += 1;
                                } else {
                                    parity_failed += 1;
                                }
                                if checked["nativeMatches"] == 0 {
                                    no_positive_witness += 1;
                                }
                                eprintln!(
                                    "catalog {relative}/{} grammar {index}: parity={}",
                                    clause.id, checked["passed"]
                                );
                                result["execution"] = checked;
                            }
                            result
                        }
                        Err(error) => {
                            *refusals.entry(error.clone()).or_default() += 1;
                            json!({"admitted":false,"reason":error})
                        }
                    };
                    grammars.push(
                        json!({"grammar":index,"threeValued":grammar.three_valued,"result":result}),
                    );
                }
            }
            rows.push(json!({"policy":relative,"policySha256":sha,"clause":clause.id,"ast":!grammars.is_empty(),"grammars":grammars,"publication":{"enumerates":clause.violation.each.is_some(),"suppresses":clause.violation.suppress.is_some(),"hasFix":clause.violation.fix.is_some()},"decision":clause.decision}));
        }
    }
    let refused = total - admitted;
    json!({
        "scope":"Checked-in Python policy catalog. Grammar admission and optional all-fixture matching parity; not production replacement certification.",
        "policies":policies,"clauses":rows,
        "summary":{"policies":policies.len(),"clauses":rows.len(),"grammars":total,"admitted":admitted,"refused":refused,"policyCompilationErrors":policy_errors,"refusalReasons":refusals,"executionPassed":parity_passed,"executionFailed":parity_failed,"noPositiveWitness":no_positive_witness},
        "fixtures":fixtures.as_ref().map(|fixtures| execution::inventory(fixtures)),
        "incompleteCoverage":refused>0 || policy_errors>0 || !execute || parity_failed>0 || no_positive_witness>0,
        "unverified":["Policy scope, violation enumeration/rendering, fixes and suppression.","Cross-file scope/module/call context and incremental invalidation.","Finite utility work allowances, deadlines and stack/depth-limit equivalence.","Complete-job batching across all grammars and the 100-execution budget.","A matching fixture is not proof that every semantic branch has positive and negative coverage."]
    })
}

fn write_report(report: &Value) {
    std::fs::write(
        std::env::var("RERAIL_RELATIONAL_CUTOVER_LEDGER").unwrap(),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "set RERAIL_RELATIONAL_CUTOVER_LEDGER"]
fn relational_cutover_catalog_admission() {
    write_report(&catalog(false));
}

#[test]
#[ignore = "set RERAIL_RELATIONAL_CUTOVER_LEDGER; fails until every catalog grammar is admitted"]
fn relational_cutover_catalog_complete_coverage() {
    let report = catalog(false);
    write_report(&report);
    assert!(
        report["summary"]["policies"].as_u64().unwrap() >= 138,
        "catalog inventory shrank"
    );
    assert!(
        report["summary"]["grammars"].as_u64().unwrap() >= 146,
        "grammar inventory shrank"
    );
    assert_eq!(
        report["summary"]["policyCompilationErrors"], 0,
        "policy compilation failures are not exclusions"
    );
    assert_eq!(
        report["summary"]["refused"], 0,
        "full report written; compiler refusals prevent cutover"
    );
}

#[test]
#[ignore = "set RERAIL_RELATIONAL_CUTOVER_LEDGER; runs admitted grammars over all checked-in upstream fixtures"]
fn relational_cutover_catalog_fixture_parity() {
    let report = catalog(true);
    write_report(&report);
    assert!(
        report["fixtures"]["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty()),
        "empty fixture inventory is not parity evidence"
    );
    assert!(
        report["summary"]["admitted"].as_u64().unwrap() > 0,
        "no admitted grammar was executed"
    );
    assert_eq!(
        report["summary"]["executionFailed"], 0,
        "full report written; admitted grammar execution failed"
    );
}
