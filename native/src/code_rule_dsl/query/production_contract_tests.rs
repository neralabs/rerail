//! Production conformance contracts for SQL lowering.
//!
//! Every input is an authored, complete `rerail.policy/v1` document. The tests compile these
//! documents with the public policy compiler and then invoke `SqlCatalogPlan`; there is no compact
//! form frontend. Compact forms are immutable descriptions only.

use super::relational::SqlCatalogPlan;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ContractCase {
    name: String,
    form: String,
    policy: Value,
    workspace: ContractWorkspace,
    expected: ContractExpected,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ContractWorkspace {
    files: Vec<ContractFile>,
    document_maps: Vec<ContractDocumentMap>,
    capabilities: String,
    failure_injection: Option<FailureInjection>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ContractFile {
    path: String,
    language: String,
    source: String,
    module: Option<Value>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ContractDocumentMap {
    source_path: String,
    generated_path: String,
    spans: Vec<ContractMappedSpan>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ContractMappedSpan {
    source_start: usize,
    source_end: usize,
    generated_start: usize,
    generated_end: usize,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct FailureInjection {
    producer: usize,
    code: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ContractExpected {
    oracle: ExpectedOracle,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct ExpectedOracle {
    violations: Vec<Value>,
    failures: Vec<Value>,
}

fn authored_cases(source: &str) -> Vec<ContractCase> {
    serde_json::from_str(source).expect("the authored production contracts have the strict schema")
}

fn cases() -> Vec<ContractCase> {
    let mut cases = Vec::with_capacity(108);
    for fixture in [
        include_str!("fixtures/production_contracts_001_020.json"),
        include_str!("fixtures/production_contracts_021_040.json"),
        include_str!("fixtures/production_contracts_041_060.json"),
        include_str!("fixtures/production_contracts_061_080.json"),
        include_str!("fixtures/production_contracts_081_108.json"),
    ] {
        cases.extend(authored_cases(fixture));
    }
    let names = cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        cases.len(),
        108,
        "the production contract inventory is fixed"
    );
    assert_eq!(names.len(), cases.len(), "contract names are unique");
    for case in &cases {
        assert_eq!(case.policy["format"], "rerail.policy/v1", "{}", case.name);
        assert_eq!(
            case.policy["id"],
            format!("sql-contract-{}", case.name),
            "{}",
            case.name
        );
        assert_eq!(case.policy["revision"], 1, "{}", case.name);
        assert_eq!(case.policy["mode"], "block", "{}", case.name);
        assert_eq!(
            case.policy["clauses"]
                .as_array()
                .expect("a production policy has clauses")
                .len(),
            1,
            "{}",
            case.name
        );
    }
    cases
}

#[test]
fn production_contract_phase_one_has_complete_typed_data() {
    let cases = cases();
    for case in cases {
        assert!(
            !case.workspace.files.is_empty(),
            "{} has no files",
            case.name
        );
        let paths = case
            .workspace
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(paths.len(), case.workspace.files.len(), "{}", case.name);
        assert!(
            matches!(
                case.workspace.capabilities.as_str(),
                "default" | "work-limit-one"
            ),
            "{}",
            case.name
        );
        for map in &case.workspace.document_maps {
            assert!(paths.contains(map.source_path.as_str()), "{}", case.name);
            assert!(!map.generated_path.is_empty(), "{}", case.name);
            assert!(!map.spans.is_empty(), "{}", case.name);
            for span in &map.spans {
                assert!(span.source_start <= span.source_end, "{}", case.name);
                assert!(span.generated_start <= span.generated_end, "{}", case.name);
            }
        }
        if let Some(failure) = &case.workspace.failure_injection {
            assert_eq!(case.name, "semantic-fact-producer-failure");
            assert_eq!(failure.producer, 0);
            assert_eq!(failure.code, "capacity-failure");
        }
    }
}

fn contract_capabilities(case: &ContractCase) -> String {
    let mut capabilities: Value =
        serde_json::from_str(CAPABILITIES).expect("default test capabilities are JSON");
    match case.workspace.capabilities.as_str() {
        "default" => {}
        "work-limit-one" => capabilities["limits"]["maxVisitedNodes"] = 1.into(),
        other => panic!("{} has unknown capabilities `{other}`", case.name),
    }
    capabilities.to_string()
}

fn oracle_result(case: &ContractCase) -> Value {
    let compiled = CodeRuleDslRuntime::compile(
        case.policy.to_string(),
        "json".to_owned(),
        contract_capabilities(case),
    )
    .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
    let files = case
        .workspace
        .files
        .iter()
        .map(|file| {
            let mut input = serde_json::json!({
                "path": file.path,
                "language": file.language,
                "source": file.source,
            });
            if let Some(module) = &file.module {
                input["module"] = module.clone();
            }
            input
        })
        .collect::<Vec<_>>();
    let snapshot = serde_json::json!({
        "format": "rerail.repository-snapshot/v1",
        "revision": 1,
        "files": files,
        "facts": [],
    });
    let output = compiled
        .evaluate(snapshot.to_string(), "report".to_owned())
        .unwrap_or_else(|error| panic!("{} oracle: {}", case.name, error.reason));
    serde_json::from_str(&output).expect("the oracle returns JSON")
}

#[test]
fn production_contract_phase_one_oracles_match_committed_results() {
    let mut differences = Vec::new();
    for case in cases() {
        let result = oracle_result(&case);
        if result["violations"].as_array() != Some(&case.expected.oracle.violations)
            || result["failures"].as_array() != Some(&case.expected.oracle.failures)
        {
            differences.push(case.name);
        }
    }
    assert!(
        differences.is_empty(),
        "oracle results differ for: {}",
        differences.join(", ")
    );
}

#[test]
fn all_108_contracts_are_real_production_inputs_and_lower_to_sql() {
    let cases = cases();
    let mut failures = Vec::new();
    for case in cases {
        let compiled = match CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        ) {
            Ok(compiled) => compiled,
            Err(error) => {
                failures.push(format!(
                    "{} (`{}`) policy: {}",
                    case.name, case.form, error.reason
                ));
                continue;
            }
        };
        if let Err(error) = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)]) {
            failures.push(format!(
                "{} (`{}`) lowering: {}",
                case.name,
                case.form,
                error.unsupported().join("; ")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn default_capabilities_fixture_remains_valid_json() {
    let _: Value = serde_json::from_str(CAPABILITIES).expect("default test capabilities are JSON");
}
