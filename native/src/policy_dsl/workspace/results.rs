//! Reporting: result serialization, the ordered result page index, and evicted files' states
//! rebuilt from their stored results.

use super::crossfile::CrossFileScheduler;
use super::ingest::{candidate_dispatch, make_file};
use super::memory::witness_payload_bytes;
use super::persisted::{candidate_catalog, restore_complete_results};
use super::{CandidateSets, CheckId, FileState, WorkspaceState};
use crate::policy_dsl::eval::{
    coverage, evaluation_json, failure, file_language, finding_from_witness, finding_identity,
};
use crate::policy_dsl::findings::Witness;
use crate::policy_dsl::json::{invalid, json_error};
use crate::policy_dsl::model::{CheckKind, CompiledPolicy, Evaluation, Metrics, Query};
#[cfg(feature = "policy-profile")]
use crate::policy_dsl::profile;
use napi::Result;
use serde_json::{Value, json};
#[cfg(test)]
use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

// Per-thread so concurrently running tests cannot perturb each other's counts. A result page task
// computes on the thread that runs it, so a test that runs the task observes its own counts.
#[cfg(test)]
thread_local! {
    pub(super) static RESULT_PAGE_WITNESSES_VISITED: Cell<usize> = const { Cell::new(0) };
    pub(super) static RESULT_PAGE_SORTS: Cell<usize> = const { Cell::new(0) };
    pub(super) static RESULT_PAGE_EVICTED_RESTORES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn count(counter: &'static std::thread::LocalKey<Cell<usize>>, amount: usize) {
    counter.set(counter.get() + amount);
}

pub(super) fn profile_result(
    result: String,
    metrics: &Metrics,
    serialization: Duration,
) -> Result<String> {
    if std::env::var_os("RERAIL_POLICY_PROFILE").is_none() {
        return Ok(result);
    }
    let mut value: Value = serde_json::from_str(&result).map_err(json_error)?;
    value.as_object_mut().expect("workspace result").insert(
        "workspaceProfile".to_owned(),
        json!({
            "lineIndexNanoseconds": metrics.line_index_nanoseconds,
            "regexNanoseconds": metrics.regex_nanoseconds,
            "astParseNanoseconds": metrics.ast_parse_nanoseconds,
            "astMatchNanoseconds": metrics.ast_match_nanoseconds,
            "serializationNanoseconds": serialization.as_nanos() as u64,
            "astMatcherCalls": metrics.ast_matcher_calls,
            "storageRetries": metrics.storage_retries,
            "storageRetryWaitNanoseconds": metrics.storage_retry_wait_nanoseconds,
            "storageTransactionNanoseconds": metrics.storage_transaction_nanoseconds,
            "storageObjectTransactionNanoseconds": metrics.storage_object_transaction_nanoseconds,
            "storageRelationshipTransactionNanoseconds": metrics.storage_relationship_transaction_nanoseconds,
            "storageReadDataStatements": metrics.storage_read_data_statements,
            "storageDataStatements": metrics.storage_data_statements,
            "storageMaxDataStatementsPerPass": metrics.storage_max_data_statements_per_pass,
            "storageTransactionStatements": metrics.storage_transaction_statements,
            "storageReconciliationPasses": metrics.storage_reconciliation_passes,
            "storageReadNanoseconds": metrics.storage_read_nanoseconds,
            "storageManifestNanoseconds": metrics.storage_manifest_nanoseconds,
            "storageWriteNanoseconds": metrics.storage_write_nanoseconds,
            "reusedSources": metrics.reused_sources,
            "reusedTapes": metrics.reused_tapes,
            "compressedSources": metrics.compressed_sources,
            "compressedTapes": metrics.compressed_tapes,
            "restoredTapes": metrics.restored_tapes,
            "restoredCandidateFiles": metrics.restored_candidate_files,
            "avoidedTapeDecompressions": metrics.avoided_tape_decompressions,
            "tapeCacheHits": metrics.tape_cache_hits,
            "tapeDecompressions": metrics.tape_decompressions,
            "restoredLocalResults": metrics.restored_local_results,
        }),
    );
    serde_json::to_string(&value).map_err(json_error)
}

fn result_value_order(value: &Value) -> (String, u64, String, String) {
    (
        value
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        value
            .get("span")
            .and_then(|span| span.get("startByte"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        value
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    )
}

#[derive(Clone)]
enum ResultPageEntry {
    Local {
        policy: usize,
        check: usize,
        witness: Witness,
    },
    Global(Value),
}

pub(super) struct ResultPageIndex {
    revision: u32,
    entries: Vec<(String, u64, String, String, ResultPageEntry)>,
    failures: Vec<Value>,
    /// The checks' coverage from `failures`, sent with them on the first page.
    coverage: Vec<Value>,
}

impl ResultPageIndex {
    /// Resident bytes of the open index, estimated like the workspace's own results.
    pub(super) fn estimated_bytes(&self) -> usize {
        use std::mem::size_of;
        let entries = self
            .entries
            .iter()
            .map(|(path, _, code, id, entry)| {
                path.capacity()
                    + code.capacity()
                    + id.capacity()
                    + match entry {
                        ResultPageEntry::Local { witness, .. } => witness_payload_bytes(witness),
                        ResultPageEntry::Global(finding) => finding.to_string().len(),
                    }
            })
            .sum::<usize>();
        size_of::<Self>()
            + self.entries.capacity() * size_of::<(String, u64, String, String, ResultPageEntry)>()
            + entries
            + self.failures.capacity() * size_of::<Value>()
            + self
                .failures
                .iter()
                .map(|failure| failure.to_string().len())
                .sum::<usize>()
            + self.coverage.capacity() * size_of::<Value>()
            + self
                .coverage
                .iter()
                .map(|entry| entry.to_string().len())
                .sum::<usize>()
    }
}

/// The workspace's reported checks, as (policy id, check id): each has a coverage entry.
fn reported_checks(state: &WorkspaceState) -> Vec<(Option<String>, String)> {
    state
        .policies
        .iter()
        .flat_map(|policy| {
            policy
                .checks
                .iter()
                .filter(|check| !check.synthetic)
                .map(|check| (Some(policy.id.clone()), check.id.clone()))
        })
        .collect()
}

/// Evicted files with their stored results, rebuilt for reporting without making them resident:
/// all of them, or those in `only`. An evicted file whose complete results are not in the store is
/// an error, never an omission.
fn evicted_file_states(
    state: &WorkspaceState,
    only: Option<&BTreeSet<String>>,
) -> Result<Vec<FileState>> {
    let evicted = state
        .evicted_inputs
        .iter()
        .filter(|(path, _)| !state.files.contains_key(*path))
        .filter(|(path, _)| only.is_none_or(|only| only.contains(*path)))
        .filter_map(|(path, input)| {
            let key = state.versions.get(path)?.result_key();
            let stamped = CrossFileScheduler::stamped_evicted_key(state, path, key);
            Some((input, key, stamped))
        })
        .collect::<Vec<_>>();
    if evicted.is_empty() {
        return Ok(Vec::new());
    }
    #[cfg(test)]
    count(&RESULT_PAGE_EVICTED_RESTORES, evicted.len());
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| invalid("evicted files need the workspace store"))?;
    let keys = evicted
        .iter()
        .flat_map(|(_, key, stamped)| std::iter::once(*key).chain(*stamped))
        .collect::<Vec<_>>();

    let results = storage
        .store
        .local_check_results_by_key(&keys, candidate_catalog(&state.policies)?)
        .map_err(|error| invalid(error.to_string()))?;
    let mut candidate_sets = CandidateSets::new();
    evicted
        .into_iter()
        .map(|(input, key, stamped)| {
            let mut file = make_file(
                input.clone(),
                state.revision.max(1),
                &state.policies,
                &mut candidate_sets,
            );
            // A quiet file's consumers match nothing; they are answered, not stored.
            let quiet = CrossFileScheduler::quiet(state, input, &state.policies);
            if !quiet.is_empty() {
                let rest = file
                    .candidates
                    .regex
                    .iter()
                    .chain(&file.candidates.ast)
                    .copied()
                    .filter(|id| !quiet.contains(id))
                    .collect();
                file.candidates = Arc::new(candidate_dispatch(
                    file_language(input),
                    rest,
                    &state.policies,
                    input,
                ));
                for id in quiet {
                    file.witnesses.insert(id, Vec::new());
                }
            }
            let mut persisted = results
                .get(&key)
                .map_or_else(Vec::new, |results| results.to_vec());
            if let Some(results) = stamped.and_then(|stamped| results.get(&stamped)) {
                persisted.extend(results.iter().cloned());
            }
            let persisted = persisted.as_slice();
            restore_complete_results(&mut file, &state.policies, persisted).ok_or_else(|| {
                invalid(format!(
                    "stored results for evicted {} are unavailable; apply it again",
                    input.path
                ))
            })?;
            Ok(file)
        })
        .collect()
}

fn reporting_ast_candidates<'a>(
    file: &'a FileState,
    policies: &'a [Arc<CompiledPolicy>],
) -> impl Iterator<Item = CheckId> + 'a {
    file.candidates
        .ast
        .iter()
        .copied()
        .chain(file.restored_result_ids.iter().copied().filter(|id| {
            matches!(
                policies[id.0].checks[id.1].kind,
                CheckKind::Forbid(Query::Ast { .. })
            )
        }))
}

fn build_result_page_index(state: &WorkspaceState) -> Result<ResultPageIndex> {
    let evicted = evicted_file_states(state, None)?;
    let mut entries = Vec::new();
    let mut failures = Vec::new();
    for file in state.files.values().chain(&evicted) {
        for (&id, witnesses) in &file.witnesses {
            if file.failures.contains_key(&id) || state.policies[id.0].checks[id.1].synthetic {
                continue;
            }
            let policy = &state.policies[id.0];
            let check = &policy.checks[id.1];
            for witness in witnesses {
                #[cfg(test)]
                count(&RESULT_PAGE_WITNESSES_VISITED, 1);
                let (finding_id, span, _) = finding_identity(policy, check, witness);
                let start = span.get("startByte").and_then(Value::as_u64).unwrap_or(0);
                entries.push((
                    witness.path.clone(),
                    start,
                    check.finding.code.clone(),
                    finding_id,
                    ResultPageEntry::Local {
                        policy: id.0,
                        check: id.1,
                        witness: witness.clone(),
                    },
                ));
            }
        }
        for (&id, &(code, message)) in &file.failures {
            if state.policies[id.0].checks[id.1].synthetic {
                continue;
            }
            let mut item = failure(
                &state.policies[id.0].checks[id.1].id,
                code,
                message,
                Some(&file.input.path),
            );
            item.as_object_mut()
                .expect("failure object")
                .insert("policyId".to_owned(), json!(state.policies[id.0].id));
            failures.push(item);
        }
        if let Some(message) = &file.syntax {
            for id in reporting_ast_candidates(file, &state.policies) {
                if file.failures.contains_key(&id) || state.policies[id.0].checks[id.1].synthetic {
                    continue;
                }
                let mut item = failure(
                    &state.policies[id.0].checks[id.1].id,
                    "source-partial",
                    message,
                    Some(&file.input.path),
                );
                item.as_object_mut()
                    .expect("failure object")
                    .insert("policyId".to_owned(), json!(state.policies[id.0].id));
                failures.push(item);
            }
        }
    }
    for (&index, global) in &state.global_results {
        for finding in &global.findings {
            let (path, start, code, id) = result_value_order(finding);
            entries.push((
                path,
                start,
                code,
                id,
                ResultPageEntry::Global(finding.clone()),
            ));
        }
        for item in &global.failures {
            let mut item = item.clone();
            item.as_object_mut()
                .expect("failure object")
                .insert("policyId".to_owned(), json!(state.policies[index].id));
            failures.push(item);
        }
    }
    #[cfg(test)]
    count(&RESULT_PAGE_SORTS, 1);
    entries.sort_by(|left, right| {
        (&left.0, left.1, &left.2, &left.3).cmp(&(&right.0, right.1, &right.2, &right.3))
    });
    Ok(ResultPageIndex {
        revision: state.revision,
        entries,
        coverage: coverage(&reported_checks(state), &failures),
        failures,
    })
}

pub(super) fn serialize_result_page(
    state: &mut WorkspaceState,
    cursor: usize,
    maximum_bytes: usize,
) -> Result<String> {
    if cursor == 0 {
        state.result_page_index = Some(build_result_page_index(state)?);
    }
    let index = state
        .result_page_index
        .as_ref()
        .filter(|index| index.revision == state.revision)
        .ok_or_else(|| invalid("result page cursor is stale; restart at cursor 0"))?;
    let total = index.entries.len();
    let mut findings = Vec::new();
    let mut bytes = 0_usize;
    let mut next = cursor.min(total);
    for (_, _, _, _, entry) in index.entries.iter().skip(cursor) {
        let finding = match entry {
            ResultPageEntry::Local {
                policy,
                check,
                witness,
            } => finding_from_witness(
                &state.policies[*policy],
                &state.policies[*policy].checks[*check],
                witness.clone(),
            ),
            ResultPageEntry::Global(finding) => finding.clone(),
        };
        let finding_bytes = serde_json::to_vec(&finding).map_err(json_error)?.len();
        if !findings.is_empty() && bytes.saturating_add(finding_bytes) > maximum_bytes {
            break;
        }
        bytes = bytes.saturating_add(finding_bytes);
        findings.push(finding);
        next += 1;
    }
    let done = next >= total;
    let page = serde_json::to_string(&json!({
        "revision": state.revision,
        "cursor": next,
        "done": done,
        "findings": findings,
        "failures": if cursor == 0 { index.failures.clone() } else { Vec::new() },
        "coverage": if cursor == 0 { index.coverage.clone() } else { Vec::new() },
        "bytes": bytes,
    }))
    .map_err(json_error)?;
    if done {
        state.result_page_index = None;
    }
    Ok(page)
}

pub(super) fn serialize_result(
    state: &WorkspaceState,
    metrics: Metrics,
    deferred: bool,
    replaced_paths: &BTreeSet<String>,
) -> Result<String> {
    #[cfg(feature = "policy-profile")]
    let _serialization_timer = profile::PhaseTimer::start(profile::Phase::Serialization);
    let mut evaluation = Evaluation {
        metrics,
        checks: reported_checks(state),
        ..Evaluation::default()
    };
    let delta = state.storage.is_some()
        && !replaced_paths.is_empty()
        && (deferred || state.globals.is_empty());
    // A complete result covers evicted files too, from their stored results; a delta covers the
    // replaced ones, including consumers whose relation changed while they remained evicted.
    let evicted = evicted_file_states(state, delta.then_some(replaced_paths))?;
    for file in state.files.values().chain(&evicted) {
        if delta && !replaced_paths.contains(&file.input.path) {
            continue;
        }
        for (&id, witnesses) in &file.witnesses {
            if file.failures.contains_key(&id) || state.policies[id.0].checks[id.1].synthetic {
                continue;
            }
            let policy = &state.policies[id.0];
            evaluation.findings.extend(
                witnesses
                    .iter()
                    .cloned()
                    .map(|witness| finding_from_witness(policy, &policy.checks[id.1], witness)),
            );
        }
        for (&id, &(code, message)) in &file.failures {
            if state.policies[id.0].checks[id.1].synthetic {
                continue;
            }
            let mut item = failure(
                &state.policies[id.0].checks[id.1].id,
                code,
                message,
                Some(&file.input.path),
            );
            item.as_object_mut()
                .expect("failure object")
                .insert("policyId".to_owned(), json!(state.policies[id.0].id));
            evaluation.failures.push(item);
        }
        if let Some(message) = &file.syntax {
            for id in reporting_ast_candidates(file, &state.policies) {
                if file.failures.contains_key(&id) || state.policies[id.0].checks[id.1].synthetic {
                    continue;
                }
                let mut item = failure(
                    &state.policies[id.0].checks[id.1].id,
                    "source-partial",
                    message,
                    Some(&file.input.path),
                );
                item.as_object_mut()
                    .expect("failure object")
                    .insert("policyId".to_owned(), json!(state.policies[id.0].id));
                evaluation.failures.push(item);
            }
        }
    }
    for &index in state.globals.keys().filter(|_| !delta) {
        if deferred && state.dirty_globals.contains(&index) {
            continue;
        }
        if let Some(global) = state.global_results.get(&index) {
            evaluation.findings.extend(global.findings.iter().cloned());
            for failure in &global.failures {
                let mut failure = failure.clone();
                failure
                    .as_object_mut()
                    .expect("failure object")
                    .insert("policyId".to_owned(), json!(state.policies[index].id));
                evaluation.failures.push(failure);
            }
        }
    }
    evaluation.findings.sort_by_key(result_value_order);
    evaluation.failures.sort_by_key(result_value_order);
    serde_json::to_string(&json!({
        "revision": state.revision,
        "replacedPaths": delta.then(|| replaced_paths.iter().collect::<Vec<_>>()),
        "evaluation": evaluation_json(evaluation, None, &[], &[])
    }))
    .map_err(json_error)
}

#[cfg(test)]
mod tests {
    use super::super::api::{PolicyDslWorkspace, WorkspaceApplyTask};
    use super::super::storage::StorageContext;
    use super::super::tests::{WORKSPACE_TEST_SLOT, result_page};
    use crate::content_store::{ContentStore, GitTree, HashKey};
    use crate::policy_dsl::PolicyDslRuntime;
    use crate::policy_dsl::tape::TAPE_FORMAT_VERSION;
    use crate::policy_dsl::test_support::{CAPABILITIES, finding};
    use napi::Task;
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// A workspace over a fresh registration of `root` in `store`, holding `policy`.
    fn stored_workspace(store: &Arc<ContentStore>, root: &str, policy: &str) -> PolicyDslWorkspace {
        let root_tree = HashKey::git_sha1([7; 20]);
        store
            .put_git_trees_bulk(&[GitTree {
                id: root_tree,
                prefix: String::new(),
                entries: Vec::new(),
            }])
            .unwrap();
        let snapshot = store
            .put_snapshot(
                root_tree,
                1,
                HashKey::sha256(b"rerail-grammars-v1"),
                i64::from(TAPE_FORMAT_VERSION),
            )
            .unwrap();
        let owner = format!("coverage-test:{root}:{}", std::process::id());
        let workspace_id = store
            .open_workspace(root, "repository", &owner, snapshot, 1, i64::MAX)
            .unwrap();
        let mut workspace = PolicyDslWorkspace::new();
        workspace.state.lock().unwrap().storage = Some(StorageContext {
            store: Arc::clone(store),
            repository: None,
            repository_prefix: String::new(),
            workspace_id,
            snapshot_id: snapshot,
            owner,
            _maintenance: None,
        });
        let compiled =
            PolicyDslRuntime::compile(policy.to_owned(), "json".into(), CAPABILITIES.into())
                .unwrap();
        workspace.add_policy(&compiled).unwrap();
        workspace
    }

    fn apply(workspace: &PolicyDslWorkspace, files: &Value) -> Value {
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": 1, "files": files, "removed": [] }).to_string(),
            request_buffer: None,
            request_direct: None,
        };
        serde_json::from_str(&task.compute().unwrap()).unwrap()
    }

    #[test]
    fn coverage_reports_a_broken_files_partial_check_cold_warm_and_paged() {
        let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "rerail-coverage-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = Arc::new(ContentStore::open(&directory.join("content.sqlite")).unwrap());
        let ast = |id: &str, language: &str, rule: Value| json!({ "id": id, "forbid": { "ast": { "language": language, "rule": rule } }, "finding": finding() });
        let policy = json!({
            "format": "rerail.policy/v1", "id": "covered", "revision": 1,
            "scope": { "include": ["**/*"], "exclude": [] },
            "checks": [
                ast("print", "python", json!({ "pattern": "print($A)" })),
                ast("clean", "typescript", json!({ "kind": "debugger_statement" })),
            ],
        })
        .to_string();
        let files =
            json!([{ "path": "broken.py", "language": "python", "source": "print(3)\ndef (:\n" }]);
        let expected = json!([
            { "policyId": "covered", "checkId": "print", "status": "partial", "reasons": [
                { "code": "source-partial", "count": 1, "paths": ["broken.py"] },
            ] },
            { "policyId": "covered", "checkId": "clean", "status": "complete", "reasons": [] },
        ]);

        let cold = stored_workspace(&store, "/coverage", &policy);
        let result = apply(&cold, &files);
        assert_eq!(result["evaluation"]["coverage"], expected, "{result}");
        cold.close();

        let warm = stored_workspace(&store, "/coverage", &policy);
        let result = apply(&warm, &files);
        let restored = warm
            .state
            .lock()
            .unwrap()
            .files
            .values()
            .map(|file| file.restored_local_results)
            .sum::<usize>();
        assert!(restored > 0, "the file's results are restored");
        assert_eq!(result["evaluation"]["coverage"], expected, "{result}");
        let page: Value =
            serde_json::from_str(&result_page(&warm, 0, 64 * 1024 * 1024).unwrap()).unwrap();
        assert_eq!(page["coverage"], expected, "{page}");
        warm.close();
        drop(store);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
