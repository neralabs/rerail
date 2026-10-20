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

#[test]
fn implementation_catalog_fixture_has_one_meaning() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/code-rule-dsl/query/implementation_goldens_phase_2_4.json"
    ))
    .expect("the implementation catalog fixture is JSON");
    for case in fixture["cases"]
        .as_array()
        .expect("the catalog fixture has cases")
    {
        assert!(
            case.get("normalized_sql").is_none(),
            "{} duplicates its SQL contract in the catalog fixture",
            case["name"]
        );
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ImplementationGoldenFile {
    cases: Vec<ImplementationCatalogGolden>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ImplementationCatalogGolden {
    name: String,
    terminal_catalog: Vec<ExpectedTerminal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActualImplementationGolden {
    normalized_sql: String,
    terminal_catalog: Vec<ExpectedTerminal>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ExpectedTerminal {
    ordinal: usize,
    kind: String,
    dialect: String,
    producer_kind: String,
    program_digest: String,
    result_schema: Vec<String>,
    owning_clause_ids: Vec<String>,
    result_limit: u32,
    completion_required: bool,
}

fn authored_sql_goldens() -> std::collections::BTreeMap<String, String> {
    let source = include_str!(
        "../../../../fixtures/code-rule-dsl/query/implementation_goldens_phase_2_4.sql"
    );
    source
        .split("-- fixture: ")
        .skip(1)
        .map(|section| {
            let (name, body) = section
                .split_once('\n')
                .expect("an authored SQL contract has a fixture name");
            let start = body
                .find("WITH ")
                .expect("an authored SQL contract has one statement");
            let sql = body[start..].trim().trim_end_matches(';');
            assert_eq!(sql.matches(';').count(), 0, "{name} has one statement");
            (name.trim().to_owned(), normalized_sql(sql))
        })
        .collect()
}

fn implementation_goldens() -> ImplementationGoldenFile {
    serde_json::from_str(include_str!(
        "../../../../fixtures/code-rule-dsl/query/implementation_goldens_phase_2_4.json"
    ))
    .expect("the implementation goldens have the strict schema")
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn exact_sql(sql: &str) -> String {
    let mut tokens = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((start, character)) = chars.next() {
        if character.is_whitespace() {
            continue;
        }
        if character == '\'' {
            let mut end = start + character.len_utf8();
            while let Some((index, quoted)) = chars.next() {
                end = index + quoted.len_utf8();
                if quoted == '\'' {
                    if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                        let (index, escaped) = chars.next().expect("an escaped quote exists");
                        end = index + escaped.len_utf8();
                    } else {
                        break;
                    }
                }
            }
            tokens.push(sql[start..end].to_owned());
            continue;
        }
        if character.is_ascii_alphanumeric() || matches!(character, '_' | ':' | '.') {
            let mut end = start + character.len_utf8();
            while let Some(&(index, next)) = chars.peek() {
                if next.is_ascii_alphanumeric() || matches!(next, '_' | ':' | '.') {
                    chars.next();
                    end = index + next.len_utf8();
                } else {
                    break;
                }
            }
            tokens.push(sql[start..end].to_owned());
            continue;
        }
        let mut token = character.to_string();
        if let Some(&(_, next)) = chars.peek()
            && matches!(
                (character, next),
                ('<', '=') | ('>', '=') | ('<', '>') | ('!', '=') | ('|', '|')
            )
        {
            chars.next();
            token.push(next);
        }
        tokens.push(token);
    }
    tokens.join(" ")
}

fn terminal_kind(kind: super::relational::TerminalKind) -> String {
    format!("{kind:?}").to_ascii_lowercase()
}

fn producer_kind(kind: crate::code_rule_dsl::transducer::sql_backend::ProducerKind) -> String {
    format!("{kind:?}").to_ascii_lowercase()
}

fn result_schema(kind: super::relational::TerminalKind) -> Vec<String> {
    use super::relational::TerminalKind;
    let rows = match kind {
        TerminalKind::Pattern => &["producer_status", "terminal_match", "capture"][..],
        TerminalKind::Navigation => &["producer_status", "navigation"],
        TerminalKind::Value | TerminalKind::Fact => &["producer_status", "fact"],
        TerminalKind::Universe
        | TerminalKind::Kind
        | TerminalKind::Regex
        | TerminalKind::Text
        | TerminalKind::Equality
        | TerminalKind::Parse
        | TerminalKind::CallSeed
        | TerminalKind::Transform => &["producer_status", "terminal_match"],
    };
    rows.iter().map(|row| (*row).to_owned()).collect()
}

fn compiled_case(case: &ContractCase) -> Arc<crate::code_rule_dsl::model::CompiledCodeRule> {
    CodeRuleDslRuntime::compile(
        case.policy.to_string(),
        "json".to_owned(),
        contract_capabilities(case),
    )
    .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason))
    .inner
}

fn actual_implementation_golden(case: &ContractCase) -> ActualImplementationGolden {
    let compiled = compiled_case(case);
    let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled)]).unwrap_or_else(|error| {
        panic!("{} lowering: {}", case.name, error.unsupported().join("; "))
    });
    let terminal_catalog = plan
        .terminals()
        .iter()
        .enumerate()
        .map(|(ordinal, terminal)| {
            let kind = terminal.kind();
            ExpectedTerminal {
                ordinal,
                kind: terminal_kind(kind),
                dialect: terminal.language().to_owned(),
                producer_kind: producer_kind(terminal.producer_kind()),
                program_digest: terminal.program_digest(),
                result_schema: result_schema(kind),
                owning_clause_ids: terminal
                    .clauses()
                    .iter()
                    .map(|&(rule, clause)| {
                        assert_eq!(rule, 0, "{} has one compiled rule", case.name);
                        format!("{}/{}", compiled.id, compiled.clauses[clause].id)
                    })
                    .collect(),
                result_limit: terminal.result_limit(),
                completion_required: true,
            }
        })
        .collect();
    let referenced = plan
        .clause_terminal_ids()
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        referenced,
        (0..plan.terminals().len()).collect(),
        "{} has an unreferenced catalog entry",
        case.name
    );
    ActualImplementationGolden {
        normalized_sql: normalized_sql(plan.sql()),
        terminal_catalog,
    }
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

#[test]
fn production_contract_phase_four_resolves_matches_authored_sql() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "semantic-resolves")
        .expect("the resolution contract exists");
    let expected = authored_sql_goldens();
    let actual = actual_implementation_golden(&case);
    assert_eq!(
        exact_sql(&actual.normalized_sql),
        exact_sql(&expected[&case.name]),
        "resolution must use the authored semantic-gate SQL contract"
    );
}

#[test]
fn production_contract_phase_four_layout_matches_authored_semantic_gate_sql() {
    let expected = authored_sql_goldens();
    let name = "semantic-layout";
    let case = cases()
        .into_iter()
        .find(|case| case.name == name)
        .expect("the layout contract exists");
    let actual = actual_implementation_golden(&case);
    assert_eq!(
        exact_sql(&actual.normalized_sql),
        exact_sql(&expected[&case.name]),
        "layout must use the authored common semantic-gate SQL contract"
    );
}

#[test]
fn production_contract_phase_four_value_and_annotation_targets_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["semantic-binding-value", "semantic-binding-annotation"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the semantic target contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored shared semantic-target SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_four_site_and_creation_targets_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["semantic-binding-site", "semantic-binds"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the semantic site contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored semantic-site SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_four_occurrence_matches_authored_sql() {
    let expected = authored_sql_goldens();
    let name = "semantic-binding-occurrence";
    let case = cases()
        .into_iter()
        .find(|case| case.name == name)
        .expect("the occurrence contract exists");
    let actual = actual_implementation_golden(&case);
    assert_eq!(
        exact_sql(&actual.normalized_sql),
        exact_sql(&expected[&case.name]),
        "occurrence must use the authored ordered semantic-target SQL contract"
    );
}

#[test]
fn production_contract_phase_four_redefinitions_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["semantic-redefines-any", "semantic-redefines-every"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the redefinition contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored completion-aware redefinition SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_four_every_redefinition_rejects_non_true_candidates() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "semantic-redefines-every")
        .expect("the every-redefinition contract exists");
    let compiled = CodeRuleDslRuntime::compile(
        case.policy.to_string(),
        "json".to_owned(),
        contract_capabilities(&case),
    )
    .expect("the every-redefinition policy compiles");
    let statements = authored_sql_goldens();
    let sql = &statements[&case.name];
    let original = "fact AS MATERIALIZED (SELECT * FROM packed WHERE row_kind = 6)";
    for (state, verdict) in [("false", "0"), ("unknown", "NULL")] {
        let columns = format!(
            "batch, 6, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, 8, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, {verdict}, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence"
        );
        let injected = format!(
            "fact AS MATERIALIZED (SELECT * FROM packed WHERE row_kind = 6 UNION ALL SELECT {columns} FROM packed gate WHERE gate.row_kind = 6 AND gate.fact_kind = 0)"
        );
        let sql = sql.replacen(original, &injected, 1);
        assert_ne!(sql, statements[&case.name], "candidate injection applied");
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::TypeScript,
            &sql,
        );
        assert!(
            outcome.ranges.is_empty(),
            "a {state} slot-8 candidate prevents every-redefinition truth"
        );
        assert!(
            outcome.errors.is_empty(),
            "candidate {state} is not a producer failure"
        );
    }
}

#[test]
fn production_contract_phase_four_arguments_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["semantic-arguments-bind", "semantic-arguments-filter"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the argument contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored argument-binding SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_four_formats_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in [
        "semantic-format-unused-arguments",
        "semantic-format-missing-fields",
    ] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the format contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored completion-aware format SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_four_incomplete_format_analysis_cannot_match() {
    let statements = authored_sql_goldens();
    for (name, position) in [
        ("semantic-format-unused-arguments", 0),
        ("semantic-format-missing-fields", 1),
    ] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the format contract exists");
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .expect("the format policy compiles");
        let original = "fact AS MATERIALIZED (SELECT * FROM packed WHERE row_kind = 6)";
        let columns = format!(
            "batch, 6, producer_id, file_index, task_index, match_index, {position}, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, 14, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, 1, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence"
        );
        let injected = format!(
            "fact AS MATERIALIZED (SELECT * FROM packed WHERE row_kind = 6 UNION ALL SELECT {columns} FROM packed gate WHERE gate.row_kind = 6 AND gate.fact_kind = 0)"
        );
        let sql = statements[name].replacen(original, &injected, 1);
        assert_ne!(sql, statements[name], "incomplete-state injection applied");
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::Python,
            &sql,
        );
        assert!(outcome.ranges.is_empty(), "{name}: incomplete analysis");
        assert!(
            outcome.errors.is_empty(),
            "{name}: semantic unknown is not failure"
        );
    }
}

#[test]
fn production_contract_phase_four_calls_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["semantic-calls-direct", "semantic-calls-through"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the calls contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored bounded call-target SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_four_failure_matches_authored_sql() {
    let expected = authored_sql_goldens();
    let name = "semantic-fact-producer-failure";
    let case = cases()
        .into_iter()
        .find(|case| case.name == name)
        .expect("the semantic failure contract exists");
    let actual = actual_implementation_golden(&case);
    assert_eq!(
        exact_sql(&actual.normalized_sql),
        exact_sql(&expected[&case.name]),
        "semantic failure must use the authored atomic failure SQL contract"
    );
}

#[test]
fn production_contract_phase_two_nested_truth_matches_authored_sql() {
    let expected = authored_sql_goldens();
    let name = "nested-all-any-not";
    let case = cases()
        .into_iter()
        .find(|case| case.name == name)
        .expect("the nested truth contract exists");
    let actual = actual_implementation_golden(&case);
    assert_eq!(
        exact_sql(&actual.normalized_sql),
        exact_sql(&expected[&case.name]),
        "nested Boolean composition must use the authored relational SQL contract"
    );
}

#[test]
fn production_contract_phase_two_authored_nested_truth_equals_oracle() {
    let statements = authored_sql_goldens();
    let name = "nested-all-any-not";
    let case = cases()
        .into_iter()
        .find(|case| case.name == name)
        .expect("the nested truth contract exists");
    let compiled = CodeRuleDslRuntime::compile(
        case.policy.to_string(),
        "json".to_owned(),
        contract_capabilities(&case),
    )
    .expect("the nested truth policy compiles");
    let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
        &compiled,
        &case.workspace.files[0].source,
        crate::code_rule_dsl::language::GrammarDialect::Python,
        &statements[name],
    );
    let mut expected = case
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
    expected.sort_unstable();
    assert_eq!(outcome.ranges, expected, "authored nested truth ranges");
    assert!(outcome.errors.is_empty(), "authored nested truth failures");
}

#[test]
fn production_contract_phase_two_not_relations_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["not-complete-positive", "not-unknown"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the negation contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored completion-aware negation SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_two_authored_not_relations_equal_oracles() {
    let statements = authored_sql_goldens();
    for name in ["not-complete-positive", "not-unknown"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the negation contract exists");
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .expect("the negation policy compiles");
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::Python,
            &statements[name],
        );
        let mut expected = case
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
        expected.sort_unstable();
        assert_eq!(outcome.ranges, expected, "{name}: authored SQL ranges");
        assert!(outcome.errors.is_empty(), "{name}: authored SQL failures");
    }
}

#[test]
fn production_contract_phase_two_any_relations_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in [
        "any-first-branch",
        "any-unknown-binding-blocks-later",
        "any-unknown-without-binding",
    ] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the ordered alternative contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored three-valued alternative SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_two_authored_any_relations_equal_oracles() {
    let statements = authored_sql_goldens();
    for name in [
        "any-first-branch",
        "any-unknown-binding-blocks-later",
        "any-unknown-without-binding",
    ] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the ordered alternative contract exists");
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .expect("the ordered alternative policy compiles");
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::Python,
            &statements[name],
        );
        let mut expected = case
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
        expected.sort_unstable();
        assert_eq!(outcome.ranges, expected, "{name}: authored SQL ranges");
        assert!(outcome.errors.is_empty(), "{name}: authored SQL failures");
    }
}

#[test]
fn production_contract_phase_two_all_relations_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in [
        "all-two-positive",
        "all-capture-threading",
        "all-three-valued-false-after-unknown",
        "all-three-valued-withheld-capture",
    ] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the ordered conjunction contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored three-valued conjunction SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_two_authored_all_relations_equal_oracles() {
    let statements = authored_sql_goldens();
    for name in [
        "all-two-positive",
        "all-capture-threading",
        "all-three-valued-false-after-unknown",
        "all-three-valued-withheld-capture",
    ] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the ordered conjunction contract exists");
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .expect("the ordered conjunction policy compiles");
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::Python,
            &statements[name],
        );
        let mut expected = case
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
        expected.sort_unstable();
        assert_eq!(outcome.ranges, expected, "{name}: authored SQL ranges");
        assert!(outcome.errors.is_empty(), "{name}: authored SQL failures");
    }
}

#[test]
fn production_contract_phase_two_node_values_match_authored_sql() {
    let expected = authored_sql_goldens();
    for name in ["node-value-positive", "node-value-unknown"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the node-value contract exists");
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{name} must use the authored completion-aware value SQL contract"
        );
    }
}

#[test]
fn production_contract_phase_two_authored_node_values_equal_oracles() {
    let statements = authored_sql_goldens();
    for name in ["node-value-positive", "node-value-unknown"] {
        let case = cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("the node-value contract exists");
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .expect("the node-value policy compiles");
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::Python,
            &statements[name],
        );
        let mut expected = case
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
        expected.sort_unstable();
        assert_eq!(outcome.ranges, expected, "{name}: authored SQL ranges");
        assert!(outcome.errors.is_empty(), "{name}: authored SQL failures");
    }
}

#[test]
fn production_contract_phases_two_and_three_have_complete_physical_sql_contracts() {
    let statements = authored_sql_goldens();
    for name in PHASE_TWO_CASES.iter().chain(PHASE_THREE_CASES) {
        let sql = &statements[*name];
        assert_eq!(
            sql.matches("rerail_matches").count(),
            1,
            "{name}: packed scan"
        );
        assert!(
            sql.contains("batch_status AS MATERIALIZED"),
            "{name}: batch status"
        );
        for family in [
            "producer_status",
            "terminal_match",
            "capture",
            "navigation",
            "fact",
            "document_map",
        ] {
            assert!(
                sql.contains(&format!("{family} AS MATERIALIZED")),
                "{name}: {family} physical row family"
            );
        }
        assert!(sql.contains("AS clause_ordinal"), "{name}: clause identity");
        if *name == "disabled-when" {
            assert!(
                !sql.contains("FROM producer_status failed"),
                "{name}: a producer-free clause has no producer failure union"
            );
        } else {
            assert!(
                sql.contains("row_kind, failed.producer_id"),
                "{name}: typed failure union"
            );
        }
        assert!(
            sql.ends_with("ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index"),
            "{name}: deterministic clause ordering"
        );
    }
}

#[test]
fn production_contract_phases_two_to_four_match_authored_sql_statements() {
    let expected = authored_sql_goldens();
    let expected_names = PHASE_TWO_CASES
        .iter()
        .copied()
        .chain(PHASE_THREE_CASES.iter().copied())
        .chain(PHASE_FOUR_CASES.iter().copied())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        expected.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected_names,
        "the authored SQL inventory is exact"
    );
    for case in cases()
        .into_iter()
        .filter(|case| expected_names.contains(case.name.as_str()))
    {
        let actual = actual_implementation_golden(&case);
        assert_eq!(
            exact_sql(&actual.normalized_sql),
            exact_sql(&expected[&case.name]),
            "{} exact authored SQL",
            case.name
        );
    }
}

#[test]
fn production_contract_phases_two_and_three_have_complete_terminal_catalogs() {
    let goldens = implementation_goldens();
    let expected_names = PHASE_TWO_CASES
        .iter()
        .chain(PHASE_THREE_CASES)
        .copied()
        .collect::<BTreeSet<_>>();
    let authored = goldens
        .cases
        .into_iter()
        .filter(|golden| expected_names.contains(golden.name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        authored
            .iter()
            .map(|golden| golden.name.as_str())
            .collect::<BTreeSet<_>>(),
        expected_names,
        "each Phase 2 and Phase 3 contract must own one terminal catalog"
    );
    let cases = cases()
        .into_iter()
        .map(|case| (case.name.clone(), case))
        .collect::<std::collections::BTreeMap<_, _>>();
    for golden in authored {
        let case = &cases[&golden.name];
        let actual = actual_implementation_golden(case).terminal_catalog;
        assert_eq!(
            actual, golden.terminal_catalog,
            "{} exact catalog",
            golden.name
        );
        assert_eq!(
            actual_implementation_golden(case).terminal_catalog,
            golden.terminal_catalog,
            "{} repeated catalog compilation",
            golden.name
        );
        std::thread::scope(|scope| {
            let parallel = (0..2)
                .map(|_| scope.spawn(|| actual_implementation_golden(case).terminal_catalog))
                .collect::<Vec<_>>();
            for catalog in parallel {
                assert_eq!(
                    catalog
                        .join()
                        .expect("parallel catalog compilation completes"),
                    golden.terminal_catalog,
                    "{} parallel catalog compilation",
                    golden.name
                );
            }
        });
    }
}

#[test]
fn production_contract_phase_four_has_complete_terminal_catalogs() {
    let goldens = implementation_goldens();
    let expected_names = PHASE_FOUR_CASES.iter().copied().collect::<BTreeSet<_>>();
    let phase_four = goldens
        .cases
        .into_iter()
        .filter(|golden| expected_names.contains(golden.name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        phase_four
            .iter()
            .map(|golden| golden.name.as_str())
            .collect::<BTreeSet<_>>(),
        expected_names,
        "each Phase 4 contract must own one authored terminal catalog"
    );
    let cases = cases()
        .into_iter()
        .map(|case| (case.name.clone(), case))
        .collect::<std::collections::BTreeMap<_, _>>();
    for golden in phase_four {
        assert!(
            !golden.terminal_catalog.is_empty(),
            "{} catalog",
            golden.name
        );
        let case = &cases[&golden.name];
        assert_eq!(
            actual_implementation_golden(case).terminal_catalog,
            golden.terminal_catalog,
            "{} exact ordered terminal catalog",
            golden.name
        );
        assert_eq!(
            actual_implementation_golden(case).terminal_catalog,
            golden.terminal_catalog,
            "{} repeated catalog compilation",
            golden.name
        );
        std::thread::scope(|scope| {
            let parallel = (0..2)
                .map(|_| scope.spawn(|| actual_implementation_golden(case).terminal_catalog))
                .collect::<Vec<_>>();
            for catalog in parallel {
                assert_eq!(
                    catalog
                        .join()
                        .expect("parallel catalog compilation completes"),
                    golden.terminal_catalog,
                    "{} parallel catalog compilation",
                    golden.name
                );
            }
        });
    }
}

#[test]
fn production_contract_phases_two_to_four_have_complete_implementation_goldens() {
    let goldens = implementation_goldens();
    let expected_names = PHASE_TWO_CASES
        .iter()
        .copied()
        .chain(PHASE_THREE_CASES.iter().copied())
        .chain(PHASE_FOUR_CASES.iter().copied())
        .collect::<BTreeSet<_>>();
    let actual_names = goldens
        .cases
        .iter()
        .map(|golden| golden.name.as_str())
        .collect::<BTreeSet<_>>();
    let missing = expected_names
        .difference(&actual_names)
        .copied()
        .collect::<Vec<_>>();
    let unexpected = actual_names
        .difference(&expected_names)
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        (missing, unexpected),
        (Vec::new(), Vec::new()),
        "each implemented contract must have one terminal-catalog golden"
    );
    assert_eq!(
        actual_names.len(),
        goldens.cases.len(),
        "duplicate golden name"
    );
    let cases = cases()
        .into_iter()
        .map(|case| (case.name.clone(), case))
        .collect::<std::collections::BTreeMap<_, _>>();
    for golden in goldens.cases {
        for (ordinal, terminal) in golden.terminal_catalog.iter().enumerate() {
            assert_eq!(terminal.ordinal, ordinal, "{} catalog", golden.name);
            assert!(
                !terminal.program_digest.is_empty(),
                "{} digest",
                golden.name
            );
            assert!(!terminal.result_schema.is_empty(), "{} schema", golden.name);
            assert!(terminal.completion_required, "{} completion", golden.name);
        }
        let case = &cases[&golden.name];
        let actual = actual_implementation_golden(case).terminal_catalog;
        assert_eq!(
            actual, golden.terminal_catalog,
            "{} implementation catalog",
            golden.name
        );
        assert_eq!(
            actual_implementation_golden(case).terminal_catalog,
            golden.terminal_catalog,
            "{} repeated compilation",
            golden.name
        );
        std::thread::scope(|scope| {
            let parallel = (0..2)
                .map(|_| scope.spawn(|| actual_implementation_golden(case).terminal_catalog))
                .collect::<Vec<_>>();
            for result in parallel {
                assert_eq!(
                    result.join().expect("parallel compilation completes"),
                    golden.terminal_catalog,
                    "{} parallel compilation",
                    golden.name
                );
            }
        });
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
fn production_contract_phase_four_authored_sql_has_common_clause_output() {
    let statements = authored_sql_goldens();
    let expected_columns = [
        "batch",
        "row_kind",
        "producer_id",
        "file_index",
        "task_index",
        "match_index",
        "capture_index",
        "capture_occurrence",
        "capture_sequence_start",
        "capture_sequence_count",
        "node",
        "related_node",
        "axis_kind",
        "axis_order",
        "fact_kind",
        "document_id",
        "mapped_document_id",
        "start_ordinal",
        "output_label",
        "capture_label",
        "text_value",
        "start_byte",
        "end_byte",
        "verdict",
        "complete",
        "work_used",
        "error_code",
        "field_id",
        "related_named",
        "source_text",
        "structural_key",
        "named_leaf",
        "capture_kind",
        "capture_named",
        "equality_occurrence",
        "bindings",
        "choice_order",
        "clause_ordinal",
    ];
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
        let dialect = match case.workspace.files[0].language.as_str() {
            "python" => crate::code_rule_dsl::language::GrammarDialect::Python,
            "typescript" => crate::code_rule_dsl::language::GrammarDialect::TypeScript,
            other => panic!("{} has unsupported dialect `{other}`", case.name),
        };
        let sql = &statements[&case.name];
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &case.workspace.files[0].source,
            dialect,
            sql,
        );
        assert_eq!(outcome.columns, expected_columns, "{} schema", case.name);
        assert!(
            sql.contains("CROSS JOIN batch_status batch"),
            "{} batch guard",
            case.name
        );
        assert!(
            sql.contains("FROM producer_status failed"),
            "{} failure union",
            case.name
        );
        assert!(
            sql.ends_with("ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index"),
            "{} stable order",
            case.name
        );
    }
}

#[test]
fn production_contract_phase_four_authored_sql_has_one_complete_physical_boundary() {
    let statements = authored_sql_goldens();
    let packed = "WITH packed AS MATERIALIZED (SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch)";
    let families = [
        ("batch_status", 1),
        ("producer_status", 2),
        ("terminal_match", 3),
        ("capture", 4),
        ("navigation", 5),
        ("fact", 6),
        ("document_map", 7),
    ];
    let mut failures = Vec::new();
    for name in PHASE_FOUR_CASES {
        let sql = &statements[*name];
        if !sql.starts_with(packed) {
            failures.push(format!("{name}: incomplete packed projection"));
        }
        if sql.matches("rerail_matches").count() != 1 {
            failures.push(format!("{name}: packed scan count"));
        }
        for (family, row_kind) in families {
            let expected = format!(
                "{family} AS MATERIALIZED (SELECT * FROM packed WHERE row_kind = {row_kind})"
            );
            if !sql.contains(&expected) {
                failures.push(format!("{name}: missing {family}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn production_contract_phase_four_calls_through_consumes_bounded_target_facts() {
    let statements = authored_sql_goldens();
    let sql = &statements["semantic-calls-through"];
    assert!(
        !sql.starts_with("WITH RECURSIVE"),
        "call reachability belongs to the bounded fact producer"
    );
    assert!(
        sql.contains("target.fact_kind = 16"),
        "SQL must consume typed call-target facts"
    );
    assert!(
        !sql.contains("reach(file_index"),
        "SQL must not rebuild the call graph"
    );
}

#[test]
fn production_contract_phase_four_authored_sql_results_equal_semantic_oracles() {
    let statements = authored_sql_goldens();
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
                    violation["span"]["startByte"].as_u64().unwrap() as usize,
                    violation["span"]["endByte"].as_u64().unwrap() as usize,
                )
            })
            .collect::<Vec<_>>();
        expected.sort_unstable();
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement(
            &compiled,
            &file.source,
            dialect,
            &statements[&case.name],
        );
        assert_eq!(outcome.ranges, expected, "{}", case.name);
        assert!(outcome.errors.is_empty(), "{}", case.name);
    }
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
fn production_contract_phase_four_unknown_fact_never_becomes_true() {
    let mut case = cases()
        .into_iter()
        .find(|case| case.name == "semantic-arguments-bind")
        .expect("the arguments contract exists");
    case.policy["clauses"][0]["forbid"]["ast"]["rule"]
        .as_object_mut()
        .expect("the arguments rule is an object")
        .remove("kind");
    case.workspace.files[0].source = "f(*xs)\n".to_owned();
    let oracle = oracle_result(&case);
    assert!(
        oracle["violations"].as_array().is_some_and(Vec::is_empty),
        "the matcher oracle keeps an unknown argument result out of violations"
    );
    assert!(
        oracle["failures"].as_array().is_some_and(Vec::is_empty),
        "semantic unknown is not an execution failure"
    );
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
    assert!(
        outcome.facts.iter().any(|fact| fact.flag == Some(-1)),
        "the producer must emit an explicit unknown fact"
    );
    assert!(outcome.ranges.is_empty(), "unknown is not true");
    assert!(outcome.errors.is_empty(), "unknown is not a failure");
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
                | "semantic-redefines-any"
                | "semantic-redefines-every"
                | "semantic-arguments-bind"
                | "semantic-arguments-filter"
                | "semantic-format-unused-arguments"
                | "semantic-format-missing-fields"
                | "semantic-layout"
                | "semantic-calls-direct"
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
            "semantic-redefines-any" => {
                vec![row(1, (19, 20), None, 0, 0, 0, 1, None, None)]
            }
            "semantic-redefines-every" => vec![
                row(1, (4, 5), None, 0, 0, 0, 1, None, None),
                row(1, (20, 21), None, 0, 0, 0, 1, None, None),
                row(1, (30, 31), None, 0, 0, 0, 1, None, None),
            ],
            "semantic-arguments-bind" => vec![
                row(1, (0, 6), None, 0, 0, 0, 1, None, None),
                row(1, (0, 6), Some((4, 5)), 15, 0, 0, 1, Some(93), Some("A")),
            ],
            "semantic-arguments-filter" => vec![
                row(1, (0, 10), None, 0, 0, 0, 1, None, None),
                row(1, (0, 10), Some((4, 9)), 15, 0, 0, 1, Some(1), Some("A")),
            ],
            "semantic-format-unused-arguments" => vec![
                row(1, (0, 18), None, 0, 0, 0, 1, Some(1), None),
                row(1, (0, 18), Some((16, 17)), 12, 0, 0, 1, Some(1), None),
                row(1, (0, 18), None, 14, 0, 0, 0, None, None),
                row(1, (0, 18), None, 14, 0, 1, 0, None, None),
                row(1, (0, 18), Some((0, 5)), 17, 0, 0, 1, Some(1), Some("S")),
                row(1, (0, 18), Some((13, 14)), 17, 0, 0, 1, Some(2), Some("A")),
                row(1, (0, 18), Some((14, 15)), 17, 0, 1, 1, Some(2), Some("A")),
                row(1, (0, 18), Some((16, 17)), 17, 0, 2, 1, Some(2), Some("A")),
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
            "semantic-calls-direct" => vec![
                row(2, (23, 31), None, 0, 0, 0, 1, None, None),
                row(2, (23, 31), Some((0, 22)), 16, 0, 0, 1, Some(145), None),
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
fn production_contract_phase_four_fact_producers_have_exact_completion_states() {
    use super::policy_lowering_tests::SqlProducerStatusRow;

    for case in cases().into_iter().filter(|case| {
        PHASE_FOUR_CASES.contains(&case.name.as_str())
            && case.name != "semantic-fact-producer-failure"
    }) {
        let compiled = CodeRuleDslRuntime::compile(
            case.policy.to_string(),
            "json".to_owned(),
            contract_capabilities(&case),
        )
        .unwrap_or_else(|error| panic!("{} policy: {}", case.name, error.reason));
        let plan = SqlCatalogPlan::compile(&[Arc::clone(&compiled.inner)])
            .unwrap_or_else(|error| panic!("{} lowering: {:?}", case.name, error.unsupported()));
        let semantic_producers = plan
            .terminals()
            .iter()
            .enumerate()
            .filter(|(_, terminal)| {
                terminal.producer_kind()
                    == crate::code_rule_dsl::transducer::sql_backend::ProducerKind::SemanticFact
            })
            .map(|(producer, _)| producer as i64)
            .collect::<Vec<_>>();
        let dialect = match case.workspace.files[0].language.as_str() {
            "python" => crate::code_rule_dsl::language::GrammarDialect::Python,
            "typescript" => crate::code_rule_dsl::language::GrammarDialect::TypeScript,
            other => panic!("{} has unsupported dialect `{other}`", case.name),
        };
        let outcome = super::policy_lowering_tests::sql_outcome_for_dialect(
            &compiled,
            &case.workspace.files[0].source,
            dialect,
        );
        let actual = outcome
            .statuses
            .into_iter()
            .filter(|status| semantic_producers.contains(&status.producer))
            .collect::<Vec<_>>();
        let expected = semantic_producers
            .into_iter()
            .map(|producer| SqlProducerStatusRow {
                producer,
                file: 0,
                complete: 1,
                verdict: Some(1),
                work_used: None,
                error: None,
            })
            .collect::<Vec<_>>();
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
    assert_eq!(
        outcome
            .statuses
            .into_iter()
            .filter(|status| status.error.is_some())
            .collect::<Vec<_>>(),
        [super::policy_lowering_tests::SqlProducerStatusRow {
            producer: 1,
            file: 0,
            complete: 0,
            verdict: None,
            work_used: None,
            error: Some("semantic-fact-capacity".to_owned()),
        }]
    );
}

#[test]
fn production_contract_phase_four_authored_failure_sql_projects_every_incomplete_fact_producer() {
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
    let statements = authored_sql_goldens();
    for code in ["capacity-failure", "semantic-fact-capacity"] {
        let outcome = super::policy_lowering_tests::sql_outcome_for_statement_with_fact_failure(
            &compiled,
            &case.workspace.files[0].source,
            crate::code_rule_dsl::language::GrammarDialect::Python,
            0,
            code,
            &statements[&case.name],
        );
        assert!(outcome.ranges.is_empty(), "{code}: no partial violation");
        assert!(outcome.facts.is_empty(), "{code}: no partial facts");
        assert_eq!(outcome.errors, [code], "{code}: typed clause failure");
        assert!(outcome.statuses.iter().any(|status| {
            status.producer == 1
                && status.complete == 0
                && status.verdict.is_none()
                && status.error.as_deref() == Some(code)
        }));
    }
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
    assert_eq!(
        outcome
            .statuses
            .into_iter()
            .filter(|status| status.error.is_some())
            .collect::<Vec<_>>(),
        [super::policy_lowering_tests::SqlProducerStatusRow {
            producer: 1,
            file: 0,
            complete: 0,
            verdict: None,
            work_used: None,
            error: Some("capacity-failure".to_owned()),
        }]
    );
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
