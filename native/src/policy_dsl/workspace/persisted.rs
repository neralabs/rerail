//! Stored per-file results: engine and grammar identities, the local-result encoding, restoring a
//! file from the store, and publishing evaluated files.

#[cfg(test)]
use super::TEST_ENGINE_BUILD_ID;
use super::calls_state::Consumers;
use super::crossfile::CrossFileScheduler;
use super::documents;
use super::evaluate::candidate_grammar;
use super::ingest::candidate_dispatch;
use super::storage::{StorageContext, unix_seconds};
use super::{CheckId, FileState, LocalFailure};
use crate::content_store::{
    CandidateCatalogIdentity, CandidateCheck, HashKey, LocalResultPublication,
    OverlayFilePublication, PublicationResult, StoredFile, TapePayload,
};
use crate::policy_dsl::eval::file_language;
use crate::policy_dsl::findings::Witness;
use crate::policy_dsl::json::{invalid, json_error};
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::matcher::UtilLimit;
use crate::policy_dsl::model::{CompiledPolicy, FileInput};
#[cfg(feature = "policy-profile")]
use crate::policy_dsl::profile;
use crate::policy_dsl::{language, tape};
use napi::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fill `file` with its stored results. `None` unless every eligible check has a decodable one:
/// a partial result set is never presented as the file's result.
pub(super) fn restore_complete_results(
    file: &mut FileState,
    policies: &[Arc<CompiledPolicy>],
    persisted: &[crate::content_store::StoredLocalResult],
) -> Option<HashSet<CheckId>> {
    let expected = file
        .candidates
        .regex
        .iter()
        .chain(&file.candidates.ast)
        .copied()
        .collect::<HashSet<_>>();
    let identities = candidate_identities(policies);
    let mut restored = HashSet::new();
    for result in persisted {
        let Some(&id) = identities.get(&result.candidate) else {
            continue;
        };
        if !expected.contains(&id) {
            continue;
        }
        let (witnesses, failure, syntax) = decode_local_result(&result.bytes, &file.input.path)?;
        file.witnesses.insert(id, witnesses);
        if let Some(failure) = failure {
            file.failures.insert(id, failure);
        }
        if syntax.is_some() {
            file.syntax = syntax;
        }
        restored.insert(id);
    }
    (restored == expected).then_some(restored)
}

/// The stored per-file result encoding. A stored result of any other version is a cache miss.
const LOCAL_RESULT_VERSION: u64 = 3;

pub(super) fn encode_local_result(
    witnesses: &[Witness],
    failure: Option<LocalFailure>,
    syntax: Option<&str>,
) -> Result<Vec<u8>> {
    let witnesses = witnesses
        .iter()
        .map(|witness| {
            json!({
                "span": witness.span,
                "captures": witness.captures,
                "message": witness.message,
                "fix": witness.fix,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "version": LOCAL_RESULT_VERSION,
        "failure": failure.map(|(code, message)| json!([code, message])),
        "syntax": syntax,
        "witnesses": witnesses,
    }))
    .map_err(json_error)
}

/// The static failure a stored failure names: every failure file evaluation records, including
/// each engine limit. Anything else (another build's vocabulary) is a cache miss, not an error.
fn persisted_failure(code: &str, message: &str) -> Option<LocalFailure> {
    const CODES: [&str; 3] = ["budget-exceeded", "adapter-error", "source-incomplete"];
    const MESSAGES: [&str; 11] = [
        "file input budget exceeded",
        "file match budget exceeded",
        "file line index could not be built",
        "file grammar cannot be taped",
        "file has syntax errors",
        "file parsing time budget exceeded",
        "file tape encoding failed",
        "file tape revision mismatch",
        "file tape restoration failed",
        "file witness budget exceeded",
        "file AST work budget exceeded",
    ];
    let code = CODES.into_iter().find(|known| *known == code)?;
    let message = MESSAGES
        .into_iter()
        .chain(
            [
                UtilLimit::Depth,
                UtilLimit::Work,
                UtilLimit::Scope,
                UtilLimit::Value,
            ]
            .map(UtilLimit::message),
        )
        .find(|known| *known == message)?;
    Some((code, message))
}

/// A stored result, or `None` when it is from another encoding or cannot be read: a cache miss
/// the caller answers by evaluating.
pub(super) fn decode_local_result(
    bytes: &[u8],
    path: &str,
) -> Option<(Vec<Witness>, Option<LocalFailure>, Option<String>)> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("version").and_then(Value::as_u64) != Some(LOCAL_RESULT_VERSION) {
        return None;
    }
    let failure = match value.get("failure") {
        None | Some(Value::Null) => None,
        Some(failure) => {
            let [code, message] = failure.as_array()?.as_slice() else {
                return None;
            };
            Some(persisted_failure(code.as_str()?, message.as_str()?)?)
        }
    };
    let witnesses = value
        .get("witnesses")?
        .as_array()?
        .iter()
        .map(|witness| {
            Some(Witness {
                path: path.to_owned(),
                span: witness.get("span")?.clone(),
                captures: witness.get("captures")?.clone(),
                message: witness
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                fix: witness.get("fix").filter(|fix| !fix.is_null()).cloned(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let syntax = value
        .get("syntax")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some((witnesses, failure, syntax))
}

pub(super) fn restore_persisted_file(
    stored_documents: &documents::StoredDocuments,
    store: &crate::content_store::ContentStore,
    file: &mut FileState,
    grammars: &mut BTreeMap<String, Arc<tape::Grammar>>,
    policies: &[Arc<CompiledPolicy>],
    persisted_results: Option<&[crate::content_store::StoredLocalResult]>,
    source_hash: HashKey,
) -> Result<()> {
    let Some(stored) = stored_documents.objects.get(&source_hash) else {
        return Ok(());
    };
    let source_id = stored.source;

    let identities = candidate_identities(policies);
    let eligible = file
        .candidates
        .regex
        .iter()
        .chain(&file.candidates.ast)
        .copied()
        .collect::<HashSet<_>>();
    let mut restored = HashSet::new();
    for result in persisted_results.into_iter().flatten() {
        let Some(&id) = identities.get(&result.candidate) else {
            continue;
        };
        if !eligible.contains(&id) {
            continue;
        }
        let Some((witnesses, failure, syntax)) =
            decode_local_result(&result.bytes, &file.input.path)
        else {
            continue;
        };
        file.witnesses.insert(id, witnesses);
        if let Some(failure) = failure {
            file.failures.insert(id, failure);
        }
        if syntax.is_some() {
            file.syntax = syntax;
        }
        restored.insert(id);
    }
    if !restored.is_empty() {
        let selected = file
            .candidates
            .regex
            .iter()
            .chain(&file.candidates.ast)
            .filter(|id| !restored.contains(id))
            .copied()
            .collect();
        file.candidates = Arc::new(candidate_dispatch(
            file_language(&file.input),
            selected,
            policies,
            &file.input,
        ));
        file.restored_local_results += restored.len();
        file.restored_result_ids.extend(restored);
    }
    let Some(language) = file_language(&file.input) else {
        file.stored = Some(StoredFile {
            source: source_id,
            tape: None,
        });
        return Ok(());
    };
    if file.candidates.ast.is_empty() {
        file.stored = Some(StoredFile {
            source: source_id,
            tape: None,
        });
        return Ok(());
    }
    let grammar_id = grammar_id(language);
    let tape_id = stored.tapes.get(&grammar_id).copied();
    let Some(tape_id) = tape_id else {
        file.stored = Some(StoredFile {
            source: source_id,
            tape: None,
        });
        return Ok(());
    };
    file.stored = Some(StoredFile {
        source: source_id,
        tape: Some(tape_id),
    });
    let grammar = if let Some(grammar) = grammars.get(&file.input.language) {
        Arc::clone(grammar)
    } else {
        let grammar = tape::Grammar::new(language).map_err(invalid)?;
        grammars.insert(file.input.language.clone(), Arc::clone(&grammar));
        grammar
    };
    // A failed, stale, or absent selector read is deliberately a cache miss. Do not make an
    // index availability problem a policy verdict. Evidence producers and unrestricted anchors
    // likewise require ordinary evaluation.
    let lookups = file
        .candidates
        .ast
        .iter()
        .map(|&id| {
            let check = &policies[id.0].checks[id.1];
            check
                .finding
                .project
                .is_empty()
                .then(|| {
                    candidate_grammar(&policies[id.0], id.1, &file.input)
                        .anchors
                        .root_kinds()
                })
                .flatten()
                .map(|kinds| crate::content_store::SearchKindLookup {
                    tape_id,
                    index_version: crate::policy_dsl::transducer::POSITIONAL_INDEX_VERSION,
                    kind_tokens: kinds
                        .iter()
                        .map(|&kind| crate::policy_dsl::transducer::kind_index_token(kind))
                        .collect(),
                })
        })
        .collect::<Option<Vec<_>>>();
    file.selector_skipped = lookups.is_some_and(|lookups| {
        !lookups.is_empty()
            && store
                .get_search_kind_presence_bulk(&lookups)
                .is_ok_and(|selections| {
                    selections.iter().all(|selection| {
                        matches!(selection, crate::content_store::StoredKindPresence::Absent)
                    })
                })
    });
    if file.selector_skipped {
        for &id in &file.candidates.ast {
            file.witnesses.insert(id, Vec::new());
        }
        return Ok(());
    }
    documents::attach_stored_tape(store, tape_id, file, &grammar);
    Ok(())
}

pub(super) fn grammar_id(language: GrammarDialect) -> i64 {
    i64::from(language::tape_tag(language).expect("supported workspace grammar"))
}

/// A check's stored identity. `select_when` leaves are synthetic copies of their check and share its
/// id, so each is told apart by its position in the policy.
fn candidate_check(policy: &CompiledPolicy, check_index: usize) -> CandidateCheck {
    let check = &policy.checks[check_index];
    CandidateCheck {
        policy: HashKey::sha256(policy.id.as_bytes()),
        check: if check.synthetic {
            format!("{}\0leaf\0{check_index}", check.id)
        } else {
            check.id.clone()
        },
    }
}

fn candidate_identities(policies: &[Arc<CompiledPolicy>]) -> HashMap<CandidateCheck, CheckId> {
    policies
        .iter()
        .enumerate()
        .flat_map(|(policy_index, policy)| {
            (0..policy.checks.len()).map(move |check_index| {
                (
                    candidate_check(policy, check_index),
                    (policy_index, check_index),
                )
            })
        })
        .collect()
}

/// The engine build a stored check result was computed by (see `build.rs`): any engine source or
/// dependency change can change a check's findings, so it changes the result catalog.
fn engine_build_id() -> &'static str {
    #[cfg(test)]
    if let Some(engine) = TEST_ENGINE_BUILD_ID.get() {
        return engine;
    }
    env!("RERAIL_ENGINE_BUILD_ID")
}

/// The parser and grammar versions a stored tape was parsed with (see `build.rs`).
const GRAMMAR_BUILD_ID: &str = env!("RERAIL_GRAMMAR_BUILD_ID");

/// The stored tapes' grammar version column: the grammar build identity as a positive integer.
pub(super) fn grammar_version() -> i64 {
    i64::from_str_radix(&GRAMMAR_BUILD_ID[..15], 16).expect("grammar build identity is hex")
}

pub(super) fn grammar_set() -> HashKey {
    HashKey::sha256(GRAMMAR_BUILD_ID.as_bytes())
}

/// The inputs besides source content that one file's check results depend on: its path (and so
/// its dialect), language and Python module. Stored results are keyed by content and this.
pub(super) fn file_context(input: &FileInput) -> HashKey {
    let module = input
        .module
        .as_deref()
        .map(|module| format!("{module:?}"))
        .unwrap_or_default();
    HashKey::sha256(
        format!(
            "rerail-file-context-v1\0{}\0{}\0{module}",
            input.language, input.path
        )
        .as_bytes(),
    )
}

pub(super) fn candidate_catalog(
    policies: &[Arc<CompiledPolicy>],
) -> Result<CandidateCatalogIdentity> {
    let mut bytes = format!(
        "rerail-local-result-v{LOCAL_RESULT_VERSION}\0{}\0",
        engine_build_id()
    )
    .into_bytes();
    for policy in policies {
        bytes.extend_from_slice(policy.id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(policy.digest.as_bytes());
        bytes.push(0xfe);
        bytes.extend_from_slice(policy.capabilities.raw.to_string().as_bytes());
        bytes.push(0xfd);
        for check in &policy.checks {
            bytes.extend_from_slice(check.id.as_bytes());
            bytes.push(0xff);
        }
    }
    CandidateCatalogIdentity::new(
        HashKey::sha256(&bytes),
        grammar_set(),
        3,
        i64::from(tape::TAPE_FORMAT_VERSION),
    )
    .map_err(|error| invalid(error.to_string()))
}

pub(super) fn persist_files(
    storage: &StorageContext,
    paths: &[String],
    files: &BTreeMap<String, FileState>,
    policies: &[Arc<CompiledPolicy>],
    consumers: &Consumers,
    closed: &AtomicBool,
) -> Result<PublicationResult> {
    let catalog = candidate_catalog(policies)?;
    let mut prepared = Vec::with_capacity(paths.len());
    for path in paths {
        let file = files
            .get(path)
            .ok_or_else(|| invalid(format!("missing file for persistence: {path}")))?;
        let candidate = |&(policy_index, check_index): &CheckId| {
            candidate_check(&policies[policy_index], check_index)
        };
        let candidates = file
            .candidates
            .regex
            .iter()
            .chain(&file.candidates.ast)
            .map(candidate)
            .collect::<Vec<_>>();
        let tape_bytes = {
            #[cfg(feature = "policy-profile")]
            let _timer = profile::PhaseTimer::start(profile::Phase::TapeSerialize);
            file.tape
                .as_ref()
                .map(|(_, image)| image.to_bytes())
                .transpose()
                .map_err(invalid)?
        };
        let local_result_ids = file
            .witnesses
            .keys()
            .chain(file.failures.keys())
            .chain(&file.candidates.regex)
            .chain(&file.candidates.ast)
            .copied()
            .collect::<BTreeSet<_>>();
        let context = file_context(&file.input);
        let stamped = CrossFileScheduler::stamped_context(file, context);
        let mut local_results = local_result_ids
            .iter()
            .filter(|id| !file.transient_failure && !file.restored_result_ids.contains(id))
            .filter_map(|id| {
                let context = if consumers.contains_key(id) {
                    stamped?
                } else {
                    context
                };
                Some((id, context))
            })
            .map(|(id, context)| {
                let candidate = candidate(id);
                let witnesses = file.witnesses.get(id).map(Vec::as_slice).unwrap_or(&[]);
                let bytes = encode_local_result(
                    witnesses,
                    file.failures.get(id).copied(),
                    file.syntax.as_deref(),
                )?;
                let frame = zstd::stream::encode_all(Cursor::new(&bytes), 3)
                    .map_err(|error| invalid(error.to_string()))?;
                Ok(LocalResultPublication {
                    candidate,
                    context,
                    raw_length: bytes.len(),
                    frame,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if !file.transient_failure {
            local_results.extend(CrossFileScheduler::summary_publication(file)?);
        }
        let grammar_id = file_language(&file.input).map_or(0, grammar_id);
        let aliases = clean_blob_alias(storage, file)
            .into_iter()
            .collect::<Vec<_>>();
        prepared.push((
            file,
            candidates,
            tape_bytes,
            grammar_id,
            aliases,
            local_results,
        ));
    }
    let publications = prepared
        .iter()
        .map(
            |(file, candidates, tape_bytes, grammar_id, aliases, local_results)| {
                OverlayFilePublication {
                    path: &file.input.path,
                    state_id: i64::from(file.source_revision),
                    source: file.input.source.as_bytes(),
                    aliases,
                    tape: tape_bytes.as_deref().map(|bytes| TapePayload {
                        grammar_id: *grammar_id,
                        grammar_version: grammar_version(),
                        tape_format_version: i64::from(tape::TAPE_FORMAT_VERSION),
                        bytes,
                    }),
                    catalog,
                    candidates,
                    local_results,
                    candidates_current: false,
                }
            },
        )
        .collect::<Vec<_>>();
    #[cfg(feature = "policy-profile")]
    let _store_timer = profile::PhaseTimer::start(profile::Phase::StoreWrite);
    let result = storage
        .store
        .publish_overlay_files_cancellable(storage.workspace_id, &publications, &|| {
            closed.load(Ordering::Acquire)
        })
        .map_err(|error| invalid(error.to_string()))?;
    if closed.load(Ordering::Acquire) {
        return Err(invalid("policy workspace is closed"));
    }
    let clean_paths = prepared
        .iter()
        .filter(|(_, _, _, _, aliases, _)| !aliases.is_empty())
        .map(|(file, _, _, _, _, _)| file.input.path.as_str())
        .collect::<Vec<_>>();
    storage
        .store
        .promote_overlay_files_to_snapshot(storage.workspace_id, storage.snapshot_id, &clean_paths)
        .map_err(|error| invalid(error.to_string()))?;
    let now = unix_seconds()?;
    storage
        .store
        .renew_workspace_lease(storage.workspace_id, &storage.owner, now + 120)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(result)
}

/// The committed blob identity of `file`'s evaluated bytes, whoever supplied them: hashed only
/// when the path is clean in the worktree, so only a file expected to equal its blob pays for it.
fn clean_blob_alias(storage: &StorageContext, file: &FileState) -> Option<HashKey> {
    let repository = storage.repository.as_deref()?;
    let path = &file.input.path;
    let repository_path = if storage.repository_prefix.is_empty() {
        std::borrow::Cow::Borrowed(path.as_str())
    } else {
        std::borrow::Cow::Owned(format!("{}{path}", storage.repository_prefix))
    };
    repository.clean_source_alias(&repository_path)?;
    repository.source_alias(&repository_path, file.input.source.as_bytes())
}

/// The key a file's stored results are kept under, including any adapter closure stamp.
pub(super) fn result_context(file: &FileState) -> HashKey {
    CrossFileScheduler::result_context(file)
}
