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
        include_str!("../../../../fixtures/code-rule-dsl/query/production_contracts_001_020.json"),
        include_str!("../../../../fixtures/code-rule-dsl/query/production_contracts_021_040.json"),
        include_str!("../../../../fixtures/code-rule-dsl/query/production_contracts_041_060.json"),
        include_str!("../../../../fixtures/code-rule-dsl/query/production_contracts_061_080.json"),
        include_str!("../../../../fixtures/code-rule-dsl/query/production_contracts_081_108.json"),
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
fn production_contract_phase_one_distinct_count_uses_related_capture() {
    let cases = cases();
    let case = cases
        .iter()
        .find(|case| case.name == "count-distinct-capture")
        .expect("the distinct-count contract exists");
    assert_eq!(
        case.policy["clauses"][0]["budget"]["query"]["evidence"]["query"]["ast"]["rule"]["kind"],
        "call"
    );
    assert_eq!(
        case.policy["clauses"][0]["budget"]["query"]["evidence"]["query"]["ast"]["rule"]["has"]["pattern"],
        "$A"
    );
    assert!(!case.expected.oracle.violations.is_empty());
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

const PHASE_TWO_CASES: &[&str] = &[
    "node-value-positive",
    "node-value-unknown",
    "all-two-positive",
    "all-capture-threading",
    "all-three-valued-false-after-unknown",
    "all-three-valued-withheld-capture",
    "any-first-branch",
    "any-unknown-binding-blocks-later",
    "any-unknown-without-binding",
    "not-complete-positive",
    "not-unknown",
    "nested-all-any-not",
    "disabled-when",
];

#[test]
fn production_contract_phase_two_comparison_value_and_truth_lower_completely() {
    let mut failures = Vec::new();
    for case in cases()
        .into_iter()
        .filter(|case| PHASE_TWO_CASES.contains(&case.name.as_str()))
    {
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        match SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)]) {
            Ok(plan) => {
                assert_eq!(
                    plan.sql().matches("rerail_matches").count(),
                    1,
                    "{}",
                    case.name
                );
                assert!(!plan.sql().contains("GrammarMatcher"), "{}", case.name);
                assert!(!plan.sql().contains("tq_"), "{}", case.name);
                if case.name == "disabled-when" {
                    assert!(
                        plan.terminals().is_empty(),
                        "disabled queries demand no terminals"
                    );
                }
            }
            Err(error) => failures.push(format!(
                "{} (`{}`): {}",
                case.name,
                case.form,
                error.unsupported().join("; ")
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn production_contract_phase_two_sql_rows_equal_committed_oracle_rows() {
    for case in cases()
        .into_iter()
        .filter(|case| PHASE_TWO_CASES.contains(&case.name.as_str()))
    {
        assert_eq!(case.workspace.files.len(), 1, "{}", case.name);
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        let mut expected = case
            .expected
            .oracle
            .violations
            .iter()
            .map(|violation| {
                (
                    violation["span"]["startByte"]
                        .as_u64()
                        .expect("oracle violation has a start byte") as usize,
                    violation["span"]["endByte"]
                        .as_u64()
                        .expect("oracle violation has an end byte") as usize,
                )
            })
            .collect::<Vec<_>>();
        expected.sort_unstable();
        assert_eq!(
            super::policy_lowering_tests::sql_ranges(&compiled, &case.workspace.files[0].source),
            expected,
            "{}",
            case.name
        );
    }
}

const PHASE_FOUR_CASES: &[&str] = &[
    "semantic-resolves",
    "semantic-binding-site",
    "semantic-binding-value",
    "semantic-binding-annotation",
    "semantic-binding-occurrence",
    "semantic-binds",
    "semantic-redefines-any",
    "semantic-redefines-every",
    "semantic-arguments-bind",
    "semantic-arguments-filter",
    "semantic-format-unused-arguments",
    "semantic-format-missing-fields",
    "semantic-layout",
    "semantic-calls-direct",
    "semantic-calls-through",
    "semantic-fact-producer-failure",
];

#[test]
fn production_contract_phase_four_semantic_facts_lower_completely() {
    let mut failures = Vec::new();
    for case in cases()
        .into_iter()
        .filter(|case| PHASE_FOUR_CASES.contains(&case.name.as_str()))
    {
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        match SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)]) {
            Ok(plan) => {
                assert_eq!(
                    plan.sql().matches("rerail_matches").count(),
                    1,
                    "{}",
                    case.name
                );
                let expected_catalog = match case.name.as_str() {
                    "semantic-binding-value" | "semantic-binding-annotation" => vec![
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Pattern,
                        super::relational::TerminalKind::Fact,
                    ],
                    "semantic-format-unused-arguments"
                    | "semantic-format-missing-fields"
                    | "semantic-layout" => vec![
                        super::relational::TerminalKind::Pattern,
                        super::relational::TerminalKind::Fact,
                    ],
                    "semantic-calls-direct" | "semantic-calls-through" => vec![
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Fact,
                    ],
                    "semantic-binding-occurrence" | "semantic-redefines-every" => vec![
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Fact,
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Fact,
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Fact,
                    ],
                    "semantic-arguments-filter" => vec![
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Fact,
                        super::relational::TerminalKind::Kind,
                    ],
                    _ => vec![
                        super::relational::TerminalKind::Kind,
                        super::relational::TerminalKind::Fact,
                    ],
                };
                assert_eq!(
                    plan.terminals()
                        .iter()
                        .map(super::relational::TerminalPlan::kind)
                        .collect::<Vec<_>>(),
                    expected_catalog,
                    "{}",
                    case.name
                );
                assert!(
                    plan.shape()
                        .contains(&super::relational::SqlShape::ScanFact),
                    "{} does not compose typed facts in SQL",
                    case.name
                );
                assert!(
                    plan.terminals()
                        .iter()
                        .filter_map(super::relational::TerminalPlan::semantic_fact)
                        .all(|descriptor| !descriptor.source.has_nested_user_rule()),
                    "{} retains a nested user matcher",
                    case.name
                );
                let sql = plan.sql().split_whitespace().collect::<Vec<_>>().join(" ");
                assert!(
                    sql.contains(
                        "FROM fact gate JOIN producer_status status USING (producer_id, file_index)"
                    ),
                    "{} has no exact completion-guarded fact gate",
                    case.name
                );
                assert!(
                    sql.contains(
                        "gate.fact_kind = 0 AND status.complete = 1 AND gate.verdict <> 0"
                    ),
                    "{} has no exact three-valued fact gate",
                    case.name
                );
                assert!(
                    sql.contains("FROM producer_status failed WHERE failed.producer_id IN ("),
                    "{} has no typed producer-failure projection",
                    case.name
                );
                assert!(!plan.sql().contains("GrammarMatcher"), "{}", case.name);
                assert!(!plan.sql().contains("tq_"), "{}", case.name);
            }
            Err(error) => failures.push(format!(
                "{} (`{}`): {}",
                case.name,
                case.form,
                error.unsupported().join("; ")
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn production_contract_phase_four_sql_results_equal_semantic_oracles() {
    for case in cases().into_iter().filter(|case| {
        PHASE_FOUR_CASES.contains(&case.name.as_str())
            && case.name != "semantic-fact-producer-failure"
    }) {
        assert_eq!(case.workspace.files.len(), 1, "{}", case.name);
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        let file = &case.workspace.files[0];
        let dialect = match file.language.as_str() {
            "python" => crate::code_rule_dsl::language::GrammarDialect::Python,
            "typescript" => crate::code_rule_dsl::language::GrammarDialect::TypeScript,
            other => panic!("{} has unsupported phase-four dialect `{other}`", case.name),
        };
        let mut expected = case
            .expected
            .oracle
            .violations
            .iter()
            .map(|violation| {
                (
                    violation["span"]["startByte"]
                        .as_u64()
                        .expect("oracle violation has a start byte") as usize,
                    violation["span"]["endByte"]
                        .as_u64()
                        .expect("oracle violation has an end byte") as usize,
                )
            })
            .collect::<Vec<_>>();
        expected.sort_unstable();
        let outcome =
            super::policy_lowering_tests::sql_outcome_for_dialect(&compiled, &file.source, dialect);
        assert_eq!(outcome.ranges, expected, "{}", case.name);
    }
}

#[test]
fn production_contract_phase_four_fact_rows_are_exact_and_typed() {
    use super::policy_lowering_tests::SqlFactRow;

    let row =
        |producer, node, related, slot, key, position, flag, target_kind, label: Option<&str>| {
            SqlFactRow {
                producer,
                node,
                related,
                slot,
                key,
                position,
                flag: Some(flag),
                target_kind,
                label: label.map(str::to_owned),
            }
        };
    for case in cases().into_iter().filter(|case| {
        matches!(
            case.name.as_str(),
            "semantic-resolves"
                | "semantic-binding-site"
                | "semantic-binding-value"
                | "semantic-binding-annotation"
                | "semantic-binding-occurrence"
                | "semantic-binds"
                | "semantic-redefines-every"
                | "semantic-arguments-bind"
                | "semantic-format-missing-fields"
                | "semantic-layout"
                | "semantic-calls-through"
        )
    }) {
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        let dialect = match case.workspace.files[0].language.as_str() {
            "python" => crate::code_rule_dsl::language::GrammarDialect::Python,
            "typescript" => crate::code_rule_dsl::language::GrammarDialect::TypeScript,
            other => panic!("{} has unsupported dialect `{other}`", case.name),
        };
        let actual = super::policy_lowering_tests::sql_outcome_for_dialect(
            &compiled,
            &case.workspace.files[0].source,
            dialect,
        )
        .facts;
        let expected = match case.name.as_str() {
            "semantic-resolves" => vec![row(1, (0, 3), None, 0, 0, 0, 1, None, None)],
            "semantic-binding-site" => vec![
                row(1, (6, 7), None, 0, 0, 0, 1, None, None),
                row(1, (6, 7), Some((0, 5)), 1, 0, 0, 1, Some(199), None),
            ],
            "semantic-binding-value" => vec![
                row(2, (6, 7), None, 0, 0, 0, 1, None, None),
                row(2, (6, 7), Some((4, 5)), 2, 0, 0, 1, Some(93), None),
            ],
            "semantic-binding-annotation" => vec![
                row(2, (11, 12), None, 0, 0, 0, 1, None, None),
                row(2, (11, 12), Some((3, 6)), 3, 0, 0, 1, Some(1), None),
            ],
            "semantic-binding-occurrence" => vec![
                row(1, (44, 45), None, 0, 0, 0, 1, None, None),
                row(1, (44, 45), Some((30, 31)), 4, 0, 0, 1, Some(1), None),
                row(1, (44, 45), Some((44, 45)), 4, 0, 1, 1, Some(1), None),
                row(1, (49, 50), None, 0, 0, 0, 1, None, None),
                row(1, (49, 50), Some((20, 21)), 4, 0, 0, 1, Some(1), None),
                row(1, (49, 50), Some((49, 50)), 4, 0, 1, 1, Some(1), None),
            ],
            "semantic-binds" => vec![
                row(1, (0, 1), None, 0, 0, 0, 1, None, None),
                row(1, (0, 1), Some((0, 5)), 1, 0, 0, 1, Some(199), None),
            ],
            "semantic-redefines-every" => vec![
                row(1, (4, 5), None, 0, 0, 0, 1, None, None),
                row(1, (20, 21), None, 0, 0, 0, 1, None, None),
                row(1, (30, 31), None, 0, 0, 0, 1, None, None),
            ],
            "semantic-arguments-bind" => vec![
                row(1, (0, 6), None, 0, 0, 0, 1, None, None),
                row(1, (0, 6), Some((4, 5)), 15, 0, 0, 1, Some(93), Some("A")),
            ],
            "semantic-format-missing-fields" => vec![
                row(1, (0, 25), None, 0, 0, 0, 1, Some(1), None),
                row(1, (0, 25), Some((19, 24)), 12, 0, 0, 1, Some(1), None),
                row(1, (0, 25), None, 13, 0, 0, 1, Some(3), None),
                row(1, (0, 25), None, 14, 0, 0, 0, None, None),
                row(1, (0, 25), None, 14, 0, 1, 0, None, None),
                row(1, (0, 25), Some((0, 11)), 17, 0, 0, 1, Some(1), Some("S")),
                row(1, (0, 25), Some((19, 24)), 17, 0, 0, 1, Some(2), Some("A")),
            ],
            "semantic-layout" => vec![
                row(1, (0, 1), None, 0, 0, 0, 1, None, None),
                row(1, (0, 8), None, 0, 0, 0, 1, None, None),
                row(1, (0, 8), None, 0, 0, 0, 1, None, None),
                row(1, (1, 2), None, 0, 0, 0, 1, None, None),
                row(1, (1, 8), None, 0, 0, 0, 1, None, None),
                row(1, (2, 7), None, 0, 0, 0, 1, None, None),
                row(1, (7, 8), None, 0, 0, 0, 1, None, None),
            ],
            "semantic-calls-through" => vec![
                row(2, (42, 50), None, 0, 0, 0, 1, None, None),
                row(2, (42, 50), Some((0, 22)), 16, 0, 0, 1, Some(145), None),
                row(2, (51, 60), None, 0, 0, 0, 1, None, None),
                row(2, (51, 60), Some((0, 22)), 16, 0, 0, 1, Some(145), None),
                row(2, (51, 60), Some((23, 50)), 16, 0, 1, 1, Some(145), None),
            ],
            _ => unreachable!("the exact semantic fact inventory is fixed"),
        };
        assert_eq!(actual, expected, "{}", case.name);
    }
}

#[test]
fn production_contract_phase_four_real_fact_capacity_is_atomic() {
    let mut case = cases()
        .into_iter()
        .find(|case| case.name == "semantic-binding-site")
        .expect("the binding-site contract exists");
    case.workspace.capabilities = "work-limit-one".to_owned();
    let compiled = CodeRuleDslRuntime::compile(
        case.policy.to_string(),
        "json".to_owned(),
        contract_capabilities(&case),
    )
    .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
    let outcome = super::policy_lowering_tests::sql_outcome_for_dialect(
        &compiled,
        &case.workspace.files[0].source,
        crate::code_rule_dsl::language::GrammarDialect::Python,
    );
    assert!(outcome.ranges.is_empty(), "no partial violation is valid");
    assert!(
        outcome.facts.is_empty(),
        "an incomplete producer publishes no facts"
    );
    assert_eq!(outcome.errors, ["semantic-fact-capacity"]);
}

#[test]
fn production_contract_phase_four_fact_failure_is_incomplete_and_has_no_partial_result() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "semantic-fact-producer-failure")
        .expect("the semantic fact failure contract exists");
    let compiled = CodeRuleDslRuntime::compile(
        case.policy.to_string(),
        "json".to_owned(),
        contract_capabilities(&case),
    )
    .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
    let outcome = super::policy_lowering_tests::sql_outcome_with_fact_failure(
        &compiled,
        &case.workspace.files[0].source,
        crate::code_rule_dsl::language::GrammarDialect::Python,
        0,
        "capacity-failure",
    );
    assert!(outcome.ranges.is_empty(), "no partial violation is valid");
    assert!(
        outcome.facts.is_empty(),
        "an incomplete producer publishes no facts"
    );
    assert_eq!(outcome.errors, ["capacity-failure"]);
}

const PHASE_THREE_CASES: &[&str] = &[
    "utility-simple",
    "utility-nested",
    "utility-shared-component",
    "utility-self-recursive",
    "utility-mutual-recursion",
    "utility-parameterized-call",
    "utility-argument-frame-isolation",
    "utility-capture-return",
    "utility-depth-limit",
    "utility-work-limit",
];

#[test]
fn production_contract_phase_three_utilities_lower_to_named_sql_components() {
    let expected_catalogs = [
        (
            "utility-simple",
            vec![
                super::relational::TerminalKind::Kind,
                super::relational::TerminalKind::Universe,
            ],
        ),
        (
            "utility-nested",
            vec![
                super::relational::TerminalKind::Regex,
                super::relational::TerminalKind::Universe,
            ],
        ),
        (
            "utility-shared-component",
            vec![
                super::relational::TerminalKind::Pattern,
                super::relational::TerminalKind::Universe,
            ],
        ),
        (
            "utility-self-recursive",
            vec![
                super::relational::TerminalKind::Kind,
                super::relational::TerminalKind::Universe,
                super::relational::TerminalKind::Navigation,
            ],
        ),
        (
            "utility-mutual-recursion",
            vec![
                super::relational::TerminalKind::Kind,
                super::relational::TerminalKind::Universe,
                super::relational::TerminalKind::Navigation,
            ],
        ),
        (
            "utility-parameterized-call",
            vec![
                super::relational::TerminalKind::Pattern,
                super::relational::TerminalKind::Universe,
            ],
        ),
        (
            "utility-argument-frame-isolation",
            vec![
                super::relational::TerminalKind::Pattern,
                super::relational::TerminalKind::Pattern,
                super::relational::TerminalKind::Universe,
            ],
        ),
        (
            "utility-capture-return",
            vec![
                super::relational::TerminalKind::Pattern,
                super::relational::TerminalKind::Universe,
            ],
        ),
        (
            "utility-depth-limit",
            vec![
                super::relational::TerminalKind::Universe,
                super::relational::TerminalKind::Navigation,
            ],
        ),
        (
            "utility-work-limit",
            vec![
                super::relational::TerminalKind::Universe,
                super::relational::TerminalKind::Navigation,
            ],
        ),
    ];
    for (name, expected_catalog) in expected_catalogs {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the utility contract exists");
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
            .unwrap_or_else(|error| panic!("{}: {}", case.name, error.unsupported().join("; ")));
        assert_eq!(
            plan.terminals()
                .iter()
                .map(super::relational::TerminalPlan::kind)
                .collect::<Vec<_>>(),
            expected_catalog,
            "{}",
            case.name
        );
        let expected_shape = match case.name.as_str() {
            "utility-simple" | "utility-nested" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            "utility-shared-component" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::ScanCapture,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::InnerJoin,
                super::relational::SqlShape::CaseTruth,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            "utility-self-recursive" | "utility-mutual-recursion" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::ScanNavigation,
                super::relational::SqlShape::RecursiveCte,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            "utility-parameterized-call" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::ScanCapture,
                super::relational::SqlShape::InnerJoin,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::WhereFilter,
                super::relational::SqlShape::CaseTruth,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            "utility-argument-frame-isolation" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::ScanCapture,
                super::relational::SqlShape::InnerJoin,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::OrderedUnion,
                super::relational::SqlShape::WindowRank,
                super::relational::SqlShape::CaseTruth,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            "utility-capture-return" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::ScanCapture,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            "utility-depth-limit" | "utility-work-limit" => vec![
                super::relational::SqlShape::PackedMaterialized,
                super::relational::SqlShape::StatusJoin,
                super::relational::SqlShape::ScanTerminal,
                super::relational::SqlShape::ScanNavigation,
                super::relational::SqlShape::RecursiveCte,
                super::relational::SqlShape::Bindings,
                super::relational::SqlShape::Projection,
                super::relational::SqlShape::OrderBy,
            ],
            _ => unreachable!("the phase-three inventory is fixed"),
        };
        assert_eq!(plan.shape(), expected_shape, "{}", case.name);
        let component = match case.name.as_str() {
            "utility-simple" => "clause_0_utility_u_776126ed0590d228_4 AS (",
            "utility-nested" => "clause_0_utility_outer_e93d3b174bd2a154_8 AS (",
            "utility-shared-component" => "clause_0_utility_shared_45f1e04ddfc4c72e_10 AS (",
            "utility-self-recursive" => {
                "clause_0_utility_u_31e94234aa4823f0_walk_3(utility_component, file_index, node, depth) AS ("
            }
            "utility-mutual-recursion" => {
                "clause_0_utility_a_1701be0b9db9c09f_walk_3(utility_component, file_index, node, depth) AS ("
            }
            "utility-parameterized-call" => "clause_0_utility_u_38e92a029ccae1cc_frame_4 AS (",
            "utility-argument-frame-isolation" => {
                "clause_0_utility_u_3326359d57bb62bc_frame_14 AS ("
            }
            "utility-capture-return" => "clause_0_utility_u_0a98397d679dc163_4 AS (",
            "utility-depth-limit" => {
                "clause_0_utility_u_acc07b8c7f459914_limits_3 AS (\n  SELECT file_index, CASE WHEN max(depth) > 256 THEN 'utility-depth-limit' WHEN count(*) > 2000000 THEN 'utility-work-limit' END AS error_code, count(*) AS work_used FROM clause_0_utility_u_acc07b8c7f459914_walk_2 GROUP BY file_index HAVING max(depth) > 256 OR count(*) > 2000000\n)"
            }
            "utility-work-limit" => {
                "clause_0_utility_u_20cb5618907d780c_limits_3 AS (\n  SELECT file_index, CASE WHEN max(depth) > 256 THEN 'utility-depth-limit' WHEN count(*) > 1 THEN 'utility-work-limit' END AS error_code, count(*) AS work_used FROM clause_0_utility_u_20cb5618907d780c_walk_2 GROUP BY file_index HAVING max(depth) > 256 OR count(*) > 1\n)"
            }
            _ => unreachable!("the phase-three inventory is fixed"),
        };
        assert!(plan.sql().contains(component), "{}", case.name);
        if case.name.contains("recursive") || case.name.ends_with("-limit") {
            assert!(plan.sql().contains("_walk_"), "{}", case.name);
            assert!(plan.sql().contains("UNION ALL"), "{}", case.name);
        }
    }
}

#[test]
fn production_contract_phase_three_sql_rows_equal_committed_utility_oracles() {
    for case in cases()
        .into_iter()
        .filter(|case| PHASE_THREE_CASES.contains(&case.name.as_str()))
    {
        assert_eq!(case.workspace.files.len(), 1, "{}", case.name);
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        let outcome =
            super::policy_lowering_tests::sql_outcome(&compiled, &case.workspace.files[0].source);
        let mut expected_ranges = case
            .expected
            .oracle
            .violations
            .iter()
            .map(|violation| {
                (
                    violation["span"]["startByte"].as_u64().expect("start byte") as usize,
                    violation["span"]["endByte"].as_u64().expect("end byte") as usize,
                )
            })
            .collect::<Vec<_>>();
        expected_ranges.sort_unstable();
        assert_eq!(outcome.ranges, expected_ranges, "{}", case.name);
        let expected_errors = match case.name.as_str() {
            "utility-depth-limit" => vec!["utility-depth-limit".to_owned()],
            "utility-work-limit" => vec!["utility-work-limit".to_owned()],
            _ => Vec::new(),
        };
        assert_eq!(outcome.errors, expected_errors, "{}", case.name);
    }
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
