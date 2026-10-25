//! Hard common-compiler gates. Reference SQL stays outside compiler selection.
use super::{cutover_tests::execution, planner_stats::PlannerStats};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    sql::{exec, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::json;

#[test]
fn common_compiler_matches_structural_reference_budgets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let fixtures = execution::fixtures(root);
    let mut reports = vec![];
    let mut failures = vec![];
    for (policy, vm_limit, byte_limit) in [
        (
            "ruff-sim905-split-static-string",
            44_832_610_i64,
            62_980_usize,
        ),
        // Full F632 reference: 29,948 bytes, not the overwritten 7,831-byte control.
        ("ruff-f632-is-literal", 24_034_427_i64, 37_435_usize),
    ] {
        let source =
            std::fs::read_to_string(root.join(format!("packs/python/rules/{policy}.json")))
                .unwrap();
        let policy_object =
            CodeRuleDslRuntime::compile(source, "json".into(), CAPABILITIES.into()).unwrap();
        let grammar = &grammar_matchers(&policy_object)[0];
        let batch = exec::Batch::new(
            fixtures
                .iter()
                .map(|f| (tape(grammar.language, &f.source), Default::default()))
                .collect(),
            vec![],
        );
        let stats = PlannerStats::from_batch(&batch, grammar.language);
        let compiled = match super::compile_with_stats(grammar, &stats) {
            Ok(compiled) => compiled,
            Err(error) => {
                failures.push(format!("{policy}: compilation: {error}"));
                reports.push(json!({"policy":policy,"passed":false,"compilationError":error}));
                continue;
            }
        };
        let bytes = compiled.sql.len()
            + compiled.relations["physicalStages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(str::len)
                .sum::<usize>();
        let sha = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(compiled.sql.as_bytes()))
        };
        if let Ok(directory) = std::env::var("RERAIL_COMMON_COMPILER_EVIDENCE") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{policy}-{sha}.sql")),
                &compiled.sql,
            )
            .unwrap();
        }
        let profile = compiled.relations.clone();
        if let Ok(directory) = std::env::var("RERAIL_COMMON_COMPILER_EVIDENCE") {
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{policy}-{sha}.profile.json")),
                serde_json::to_vec_pretty(&profile).unwrap(),
            )
            .unwrap();
        }
        let measured = match execution::check(grammar, compiled, &fixtures) {
            Ok(measured) => measured,
            Err(error) => {
                failures.push(format!("{policy}: {error}"));
                reports.push(json!({"policy":policy,"passed":false,"sqlBytes":bytes,"sqlSha256":sha,"profile":profile,"error":error}));
                continue;
            }
        };
        let vm = measured["vm"].as_i64().unwrap();
        let passed = measured["passed"] == true && vm <= vm_limit && bytes <= byte_limit;
        if !passed {
            failures.push(format!(
                "{policy}: parity={}, VM={vm}/{vm_limit}, totalSQL={bytes}/{byte_limit}",
                measured["passed"]
            ));
        }
        reports.push(json!({"policy":policy,"passed":passed,"sqlBytes":bytes,"sqlSha256":sha,"vmLimit":vm_limit,"sqlByteLimit":byte_limit,"profile":profile,"execution":measured}));
    }
    if let Ok(directory) = std::env::var("RERAIL_COMMON_COMPILER_EVIDENCE") {
        std::fs::write(
            std::path::Path::new(&directory).join("reference-gates.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
    assert!(
        failures.is_empty(),
        "common compiler reference gates failed: {}",
        failures.join("; ")
    );
}

#[test]
fn common_compiler_preserves_catalog_regression_budgets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let fixtures = execution::fixtures(root);
    let budgets: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            root.join("benchmark/sql-relational-catalog-performance-budget.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut reports = vec![];
    for name in [
        "ruff-b033-duplicate-value",
        "bandit-b615-huggingface-unsafe-download",
    ] {
        let path = format!("packs/python/rules/{name}.json");
        let policy = CodeRuleDslRuntime::compile(
            std::fs::read_to_string(root.join(&path)).unwrap(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap();
        let grammar = &grammar_matchers(&policy)[0];
        let compiled = super::compile(grammar).unwrap();
        let measured = execution::check(grammar, compiled, &fixtures).unwrap();
        let budget = budgets["budgets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["policy"] == path && b["grammar"] == 0)
            .unwrap();
        eprintln!(
            "catalog regression probe {name}: VM={}, executions={}",
            measured["vm"], measured["matchingStatements"]
        );
        let passed = measured["passed"] == true
            && measured["vm"].as_i64().unwrap() <= budget["maxVm"].as_i64().unwrap()
            && measured["matchingStatements"] == budget["matchingStatements"]
            && measured["nativeOutputSha256"] == budget["nativeOutputSha256"];
        reports.push(json!({"policy":name,"passed":passed,"budget":budget,"execution":measured}));
    }
    if let Ok(directory) = std::env::var("RERAIL_COMMON_COMPILER_EVIDENCE") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory).join("catalog-regression-probes.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
    assert!(
        reports.iter().all(|r| r["passed"] == true),
        "catalog regression probes failed; inspect recorded VM and native outputs"
    );
}
