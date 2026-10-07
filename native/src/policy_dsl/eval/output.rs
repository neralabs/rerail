use crate::policy_dsl::findings::{self, Span, Witness};
use crate::policy_dsl::json::hex_digest;
use crate::policy_dsl::model::{CompiledCheck, CompiledPolicy, Evaluation};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Failure codes that leave a check's coverage partial rather than failed: source recovery kept
/// some of the source from being read; the check's definite findings still stand.
const PARTIAL_CODES: [&str; 2] = ["source-partial", "source-incomplete"];

/// Example paths a coverage reason names at most.
const COVERAGE_EXAMPLES: usize = 3;

/// A witness's finding id and reported (anchored) span, and whether the anchor fell back to the
/// witness span. Complete results, result pages and stored pages order and identify findings by
/// these, so every result path agrees on them.
fn finding_identity(
    policy: &CompiledPolicy,
    check: &CompiledCheck,
    witness: &Witness,
) -> (String, Span, bool) {
    let (span, fallback) = findings::anchored_span(&check.finding.anchor, witness);
    let mut id = finding_id(
        &policy.id,
        &check.id,
        &check.finding.code,
        &witness.path,
        witness.span.start_byte as u64,
        witness.span.end_byte as u64,
    );
    // `each`: findings at one witness differ by the binding they report, so its range joins the id.
    if let Some(binding) = check
        .finding
        .each
        .as_deref()
        .and_then(|name| witness.captures.get(name)?.first())
    {
        let source = format!("{id}\0each\0{}\0{}", binding.start_byte, binding.end_byte);
        id = format!("sha256:{}", hex_digest(source.as_bytes()));
    }
    (id, span, fallback)
}

pub(crate) fn finding_from_witness(
    policy: &CompiledPolicy,
    check: &CompiledCheck,
    witness: Witness,
) -> Value {
    let (id, span, fallback) = finding_identity(policy, check, &witness);
    let mut finding = json!({
        "id": id,
        "ruleId": policy.id,
        "clauseId": check.id,
        "code": check.finding.code,
        "mode": policy.rule_mode.as_str(),
        "message": witness.message.unwrap_or_else(|| check.finding.message.clone()),
        "path": witness.path,
        "span": span,
        "captures": witness.captures,
        "variables": if fallback { json!({ "anchorFallback": true }) } else { json!({}) },
    });
    if let Some(fix) = witness.fix {
        finding
            .as_object_mut()
            .expect("finding object")
            .insert("fix".to_owned(), fix);
    }
    finding
}

pub(super) fn finding_id(
    policy: &str,
    check: &str,
    code: &str,
    anchor: &str,
    start: u64,
    end: u64,
) -> String {
    let source = format!("{policy}\0{check}\0{code}\0{anchor}\0{start}\0{end}");
    format!("sha256:{}", hex_digest(source.as_bytes()))
}

pub(crate) fn evaluation_json(
    mut evaluation: Evaluation,
    revision: Option<u32>,
    added: &[String],
    retracted: &[String],
) -> Value {
    evaluation
        .findings
        .sort_by(|a, b| result_order(a).cmp(&result_order(b)));
    let status = if evaluation.failures.is_empty() {
        "completed"
    } else {
        "failed"
    };
    let coverage = coverage(&evaluation.checks, &evaluation.failures);
    let mut value = json!({
        "status": status,
        "violations": evaluation.findings,
        "failures": evaluation.failures,
        "coverage": coverage,
        "metrics": {
            "files": evaluation.metrics.files,
            "visitedNodes": evaluation.metrics.visited_nodes,
            "witnesses": evaluation.metrics.witnesses,
        },
    });
    if std::env::var_os("RERAIL_POLICY_PROFILE").is_some() {
        value.as_object_mut().expect("evaluation object").insert(
            "profile".to_owned(),
            json!({
                "lineIndexNanoseconds": evaluation.metrics.line_index_nanoseconds,
                "regexNanoseconds": evaluation.metrics.regex_nanoseconds,
                "astParseNanoseconds": evaluation.metrics.ast_parse_nanoseconds,
                "astMatchNanoseconds": evaluation.metrics.ast_match_nanoseconds,
                "astMatcherCalls": evaluation.metrics.ast_matcher_calls,
                "storageRetries": evaluation.metrics.storage_retries,
                "storageRetryWaitNanoseconds": evaluation.metrics.storage_retry_wait_nanoseconds,
                "storageTransactionNanoseconds": evaluation.metrics.storage_transaction_nanoseconds,
                "storageObjectTransactionNanoseconds": evaluation.metrics.storage_object_transaction_nanoseconds,
                "storageRelationshipTransactionNanoseconds": evaluation.metrics.storage_relationship_transaction_nanoseconds,
                "storageReadDataStatements": evaluation.metrics.storage_read_data_statements,
                "storageDataStatements": evaluation.metrics.storage_data_statements,
                "storageMaxDataStatementsPerPass": evaluation.metrics.storage_max_data_statements_per_pass,
                "storageTransactionStatements": evaluation.metrics.storage_transaction_statements,
                "storageReconciliationPasses": evaluation.metrics.storage_reconciliation_passes,
                "storageReadNanoseconds": evaluation.metrics.storage_read_nanoseconds,
                "storageManifestNanoseconds": evaluation.metrics.storage_manifest_nanoseconds,
                "storageWriteNanoseconds": evaluation.metrics.storage_write_nanoseconds,
                "reusedSources": evaluation.metrics.reused_sources,
                "reusedTapes": evaluation.metrics.reused_tapes,
                "compressedSources": evaluation.metrics.compressed_sources,
                "compressedTapes": evaluation.metrics.compressed_tapes,
                "restoredTapes": evaluation.metrics.restored_tapes,
                "restoredCandidateFiles": evaluation.metrics.restored_candidate_files,
                "avoidedTapeDecompressions": evaluation.metrics.avoided_tape_decompressions,
                "tapeCacheHits": evaluation.metrics.tape_cache_hits,
                "tapeDecompressions": evaluation.metrics.tape_decompressions,
                "restoredLocalResults": evaluation.metrics.restored_local_results,
            }),
        );
    }
    if let Some(revision) = revision {
        let object = value.as_object_mut().expect("evaluation object");
        object.insert("revision".to_owned(), json!(revision));
        object.insert("addedViolationIds".to_owned(), json!(added));
        object.insert("retractedViolationIds".to_owned(), json!(retracted));
    }
    value
}

/// Each evaluated check's coverage, derived from the failures alone: `complete` without any,
/// `partial` where each is a `PARTIAL_CODES` code, else `failed`. A reason is one code with its
/// count and at most `COVERAGE_EXAMPLES` paths (the first in path order). A failure naming no
/// evaluated check (the `runtime` budget) counts against every check of its policy, or every check.
pub(crate) fn coverage(checks: &[(Option<String>, String)], failures: &[Value]) -> Vec<Value> {
    type Reasons<'f> = BTreeMap<&'f str, (usize, BTreeSet<&'f str>)>;
    let mut reasons = checks
        .iter()
        .map(|(policy, check)| ((policy.as_deref(), check.as_str()), Reasons::new()))
        .collect::<BTreeMap<_, _>>();
    for failure in failures {
        let field = |key: &str| failure.get(key).and_then(Value::as_str);
        let code = field("code").unwrap_or("adapter-error");
        let policy = field("ruleId");
        let own = field("clauseId").map(|check| (policy, check));
        let targets = match own.filter(|key| reasons.contains_key(key)) {
            Some(key) => vec![key],
            None => reasons
                .keys()
                .copied()
                .filter(|(owner, _)| policy.is_none() || *owner == policy)
                .collect(),
        };
        for key in targets {
            let (count, paths) = reasons
                .get_mut(&key)
                .expect("a target is an evaluated check")
                .entry(code)
                .or_default();
            *count += 1;
            if let Some(path) = field("path") {
                paths.insert(path);
                if paths.len() > COVERAGE_EXAMPLES {
                    paths.pop_last();
                }
            }
        }
    }
    checks
        .iter()
        .map(|(policy, check)| {
            let reasons = &reasons[&(policy.as_deref(), check.as_str())];
            let status = if reasons.is_empty() {
                "complete"
            } else if reasons.keys().all(|code| PARTIAL_CODES.contains(code)) {
                "partial"
            } else {
                "failed"
            };
            let reasons = reasons
                .iter()
                .map(|(code, (count, paths))| json!({ "code": code, "count": count, "paths": paths }))
                .collect::<Vec<_>>();
            let mut entry = json!({ "clauseId": check, "status": status, "reasons": reasons });
            if let Some(policy) = policy {
                entry
                    .as_object_mut()
                    .expect("coverage entry")
                    .insert("ruleId".to_owned(), json!(policy));
            }
            entry
        })
        .collect()
}

pub(crate) fn overflow_evaluation(check_id: &str, message: &str) -> Evaluation {
    Evaluation {
        failures: vec![failure(check_id, "budget-exceeded", message, None)],
        ..Evaluation::default()
    }
}

pub(crate) fn failure(check_id: &str, code: &str, message: &str, path: Option<&str>) -> Value {
    let mut value = json!({ "clauseId": check_id, "code": code, "message": message });
    if let Some(path) = path {
        value
            .as_object_mut()
            .expect("failure object")
            .insert("path".to_owned(), json!(path));
    }
    value
}

pub(crate) fn finding_ids(findings: &[Value]) -> BTreeSet<String> {
    findings
        .iter()
        .filter_map(|finding| finding.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// The order results are reported in: by path, start, code and id.
pub(crate) fn result_order(value: &Value) -> (&str, u64, &str, &str) {
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    (
        text("path"),
        value
            .get("span")
            .and_then(|span| span.get("startByte"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        text("code"),
        text("id"),
    )
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::test_support::*;
    use serde_json::{Value, json};

    fn ast(id: &str, language: &str, rule: Value) -> Value {
        json!({ "id": id, "forbid": { "ast": { "language": language, "rule": rule } }, "violation": finding() })
    }

    /// Evaluates `checks` over `files` (path, python source) under limits edited by `limits`.
    fn evaluate(checks: Value, files: &[(&str, &str)], limits: impl FnOnce(&mut Value)) -> Value {
        let capabilities = capabilities_with(|capabilities| limits(&mut capabilities["limits"]));
        let compiled = compile_document_with(
            json!({ "format": "rerail.policy/v1", "id": "coverage", "revision": 1, "mode": "steer", "clauses": checks }),
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
            .evaluate(snapshot.to_string(), "diagnostic".to_owned())
            .expect("evaluation succeeds");
        serde_json::from_str(&output).expect("evaluation is JSON")
    }

    #[test]
    fn coverage_is_each_checks_failures_by_reason_class() {
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
                    { "code": "budget-exceeded", "count": 1, "paths": [] },
                    { "code": "source-partial", "count": 4, "paths": paths },
                ] },
            ]),
            "{result}"
        );
    }

    #[test]
    fn a_failure_naming_no_evaluated_check_covers_every_check() {
        // Over the file budget, the `runtime` failure names no check: every check failed.
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
}
