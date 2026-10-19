//! Production conformance contracts for SQL lowering.
//!
//! Every input is an authored, complete `rerail.policy/v1` document. The tests compile these
//! documents with the public policy compiler and then invoke `SqlCatalogPlan`; there is no compact
//! form frontend. Compact forms are immutable descriptions only.

use super::relational::SqlCatalogPlan;
use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Clone)]
struct ContractCase {
    name: String,
    form: String,
    input: Value,
}

fn authored_cases(source: &str) -> Vec<ContractCase> {
    let fixture: Value =
        serde_json::from_str(source).expect("the authored production contracts are valid JSON");
    fixture
        .as_array()
        .expect("the authored production contracts are an array")
        .iter()
        .map(|entry| {
            let object = entry
                .as_object()
                .expect("an authored production contract is an object");
            assert_eq!(
                object.keys().map(String::as_str).collect::<BTreeSet<_>>(),
                BTreeSet::from(["form", "name", "policy"]),
                "a production contract has one unambiguous schema"
            );
            let name = entry["name"]
                .as_str()
                .expect("a production contract has a name")
                .to_owned();
            let form = entry["form"]
                .as_str()
                .expect("a production contract has a compact form")
                .to_owned();
            let input = entry["policy"].clone();
            assert_eq!(input["format"], "rerail.policy/v1", "{name}");
            assert_eq!(input["id"], format!("sql-contract-{name}"), "{name}");
            assert_eq!(input["revision"], 1, "{name}");
            assert_eq!(input["mode"], "block", "{name}");
            assert_eq!(
                input["clauses"]
                    .as_array()
                    .expect("a production policy has clauses")
                    .len(),
                1,
                "{name}"
            );
            ContractCase { name, form, input }
        })
        .collect()
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
    assert_eq!(names.len(), cases.len(), "contract names are unique");
    cases
}

#[test]
fn all_108_contracts_are_real_production_inputs_and_lower_to_sql() {
    let cases = cases();
    assert_eq!(cases.len(), 108);
    let mut failures = Vec::new();
    for case in cases {
        let compiled = match CodeRuleDslRuntime::compile(
            case.input.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
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
