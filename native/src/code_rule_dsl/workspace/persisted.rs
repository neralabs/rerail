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
use super::{CLAUSE_WORK_BUDGET_EXCEEDED, ClauseId, FileState, LocalFailure};
use crate::WORK_POOL;
use crate::code_rule_dsl::eval::file_language;
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::UtilLimit;
use crate::code_rule_dsl::model::{CompiledCodeRule, FileInput};
use crate::code_rule_dsl::placement::placement_identity;
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::profile;
use crate::code_rule_dsl::soundness::UNREAD;
use crate::code_rule_dsl::violations::{Captures, Span, Witness};
use crate::code_rule_dsl::{language, tape};
use crate::content_store::{
    BusyRetries, CandidateCatalogIdentity, CandidateClause, HashKey, LocalResultPublication,
    OverlayFilePublication, PublicationResult, StoreError, StoredFile, TapePayload, compress,
};
use napi::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};

#[cfg(test)]
thread_local! {
    /// Runs on the apply's thread once `persist_files` has published its files, before it
    /// promotes them and renews the lease: a test takes the store there.
    pub(super) static AFTER_FILES_PUBLISHED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    /// Lease renewals `persist_files` attempted on this thread: a test shows a busy promotion
    /// leaves the renewal unattempted.
    pub(super) static LEASE_RENEWAL_ATTEMPTS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Fill `file` with its stored results. `None` unless every eligible clause has a decodable one:
/// a partial result set is never presented as the file's result.
pub(super) fn restore_complete_results(
    file: &mut FileState,
    code_rules: &[Arc<CompiledCodeRule>],
    persisted: &[crate::content_store::StoredLocalResult],
) -> Option<HashSet<ClauseId>> {
    let expected = file
        .candidates
        .regex
        .iter()
        .chain(&file.candidates.ast)
        .copied()
        .collect::<HashSet<_>>();
    let (selected, undecodable) =
        result_keys(code_rules)
            .ok()?
            .select(persisted, &expected, &file.input.path);
    let mut restored = HashSet::new();
    for (id, (witnesses, failure, syntax)) in selected {
        file.witnesses.insert(id, witnesses);
        if let Some(failure) = failure {
            file.failures.insert(id, failure);
        }
        if syntax.is_some() {
            file.syntax = syntax;
        }
        restored.insert(id);
    }
    (!undecodable && restored == expected).then_some(restored)
}

/// The stored per-file result encoding. A stored result of any other version is a cache miss.
/// A marked witness (`Witness::ignored`) carries `"ignored": true`; another omits the key.
const LOCAL_RESULT_VERSION: u64 = 3;

/// A stored local result as `encode_local_result` writes it, borrowing its witnesses. Fields are
/// in key order, as a JSON object of the same keys serializes.
#[derive(Serialize)]
struct LocalResultEncoding<'a> {
    failure: Option<(&'a str, &'a str)>,
    syntax: Option<&'a str>,
    version: u64,
    #[serde(serialize_with = "serialize_witnesses")]
    witnesses: &'a [Witness],
}

#[derive(Serialize)]
struct WitnessEncoding<'a> {
    captures: &'a Captures,
    fix: &'a Option<Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    ignored: bool,
    message: &'a Option<String>,
    span: &'a Span,
}

fn serialize_witnesses<S: serde::Serializer>(
    witnesses: &&[Witness],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_seq(witnesses.iter().map(|witness| WitnessEncoding {
        captures: &witness.captures,
        fix: &witness.fix,
        ignored: witness.ignored,
        message: &witness.message,
        span: &witness.span,
    }))
}

pub(super) fn encode_local_result(
    witnesses: &[Witness],
    failure: Option<LocalFailure>,
    syntax: Option<&str>,
) -> Result<Vec<u8>> {
    serde_json::to_vec(&LocalResultEncoding {
        failure,
        syntax,
        version: LOCAL_RESULT_VERSION,
        witnesses,
    })
    .map_err(json_error)
}

/// The static failure a stored failure names: every failure file evaluation records, including
/// each engine limit. Anything else (another build's vocabulary) is a cache miss, not an error.
fn persisted_failure(code: &str, message: &str) -> Option<LocalFailure> {
    const CODES: [&str; 5] = [
        "budget-exceeded",
        crate::code_rule_dsl::model::WITNESS_LIMIT,
        crate::code_rule_dsl::model::INPUT_LIMIT,
        "adapter-error",
        "source-incomplete",
    ];
    const MESSAGES: [&str; 13] = [
        "file input budget exceeded",
        "file match budget exceeded",
        "file line index could not be built",
        "file grammar cannot be taped",
        "file has syntax errors",
        UNREAD,
        "file parsing time budget exceeded",
        "file tape encoding failed",
        "file tape revision mismatch",
        "file tape restoration failed",
        "file witness budget exceeded",
        "file AST work budget exceeded",
        CLAUSE_WORK_BUDGET_EXCEEDED,
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
                span: Span::deserialize(witness.get("span")?).ok()?,
                captures: Captures::from_value(witness.get("captures")?)?,
                message: witness
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                fix: witness.get("fix").filter(|fix| !fix.is_null()).cloned(),
                ignored: witness
                    .get("ignored")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
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
    code_rules: &[Arc<CompiledCodeRule>],
    persisted_results: Option<&[crate::content_store::StoredLocalResult]>,
    source_hash: HashKey,
) -> Result<()> {
    let Some(stored) = stored_documents.objects.get(&source_hash) else {
        return Ok(());
    };
    let source_id = stored.source;

    let eligible = file
        .candidates
        .regex
        .iter()
        .chain(&file.candidates.ast)
        .copied()
        .collect::<HashSet<_>>();
    let (selected, _) = result_keys(code_rules)?.select(
        persisted_results.into_iter().flatten(),
        &eligible,
        &file.input.path,
    );
    let mut restored = HashSet::new();
    for (id, (witnesses, failure, syntax)) in selected {
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
            code_rules,
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
    // index availability problem a code-rule verdict. Evidence producers and unrestricted anchors
    // likewise require ordinary evaluation.
    let lookups = file
        .candidates
        .ast
        .iter()
        .map(|&id| {
            let clause = &code_rules[id.0].clauses[id.1];
            clause
                .violation
                .project
                .is_empty()
                .then(|| {
                    candidate_grammar(&code_rules[id.0], id.1, &file.input)
                        .anchors
                        .root_kinds()
                })
                .flatten()
                .map(|kinds| crate::content_store::SearchKindLookup {
                    tape_id,
                    index_version: crate::code_rule_dsl::transducer::POSITIONAL_INDEX_VERSION,
                    kind_tokens: kinds
                        .iter()
                        .map(|&kind| crate::code_rule_dsl::transducer::kind_index_token(kind))
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

/// A clause's stored identity, given its code rule's (`rule_hash`). `select_when` leaves are
/// synthetic copies of their clause and share its id, so each is told apart by its position in the
/// code rule.
fn candidate_clause(
    rule_hash: HashKey,
    code_rule: &CompiledCodeRule,
    clause_index: usize,
) -> CandidateClause {
    let clause = &code_rule.clauses[clause_index];
    CandidateClause {
        rule: rule_hash,
        clause: if clause.synthetic {
            format!("{}\0leaf\0{clause_index}", clause.id)
        } else {
            clause.id.clone()
        },
    }
}

/// The engine build a stored clause result was computed by (see `build.rs`): any engine source or
/// dependency change can change a clause's violations, so it changes the result catalog.
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

/// The inputs besides source content that one file's clause results depend on: its path (and so
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

/// The identities a code-rule set's stored results are kept under. A clause result depends on the
/// source, the file context (`file_context`), its own code rule and the engine, never on the other
/// code rules of the set: each is keyed by its check, `(rule, clause)` of its own code rule's
/// identity, under the set-independent result `domain`. Editing, adding or removing a code rule
/// therefore leaves every other code rule's results reusable.
///
/// The one exception is a `budget-exceeded` verdict. Native budgets are order- and memo-dependent
/// (docs/sql-set-composition-design.md §Budgets): a clause is charged for the file tables no clause
/// before it in the pass built, so whether it runs out of its own allowance depends on the set.
/// Such a verdict is kept under the code rule's identity within this set (`set_rules`), so only
/// the set that reached it reuses it, and that check is preferred to the code rule's own.
pub(super) struct ResultKeys {
    /// What every result is stored under besides its check: the engine build, the result
    /// encoding, the grammar set, scanner and tape format, and no code rule.
    pub(super) domain: CandidateCatalogIdentity,
    /// The whole set's identity: what a completion marker (`complete_directories`,
    /// `complete_workspace_revision`) records as evaluated.
    pub(super) catalog: CandidateCatalogIdentity,
    /// Each code rule's identity, by code rule index: its id, document digest (which covers its
    /// defines), capabilities, clause ids and placement.
    pub(super) rules: Vec<HashKey>,
    /// Each code rule's identity within this set, by code rule index: its identity and the set's.
    set_rules: Vec<HashKey>,
    /// Every stored check of the set's clauses: the clause, and whether it is the set-scoped one.
    identities: HashMap<CandidateClause, (ClauseId, bool)>,
}

impl ResultKeys {
    fn new(engine: &str, code_rules: &[Arc<CompiledCodeRule>]) -> Result<Self> {
        let identity = |code_rule_catalog| {
            CandidateCatalogIdentity::new(
                code_rule_catalog,
                grammar_set(),
                3,
                i64::from(tape::TAPE_FORMAT_VERSION),
            )
            .map_err(|error| invalid(error.to_string()))
        };
        let domain = identity(HashKey::sha256(
            format!("rerail-result-domain-v1\0{LOCAL_RESULT_VERSION}\0{engine}\0").as_bytes(),
        ))?;
        let rules = code_rules
            .iter()
            .map(|code_rule| rule_identity(code_rule))
            .collect::<Vec<_>>();
        let mut set =
            format!("rerail-local-result-v{LOCAL_RESULT_VERSION}\0{engine}\0").into_bytes();
        for rule in &rules {
            set.extend_from_slice(&sha256_bytes(*rule));
        }
        let set = HashKey::sha256(&set);
        let set_rules = rules
            .iter()
            .map(|rule| {
                let mut bytes = b"rerail-set-rule-v1\0".to_vec();
                bytes.extend_from_slice(&sha256_bytes(*rule));
                bytes.extend_from_slice(&sha256_bytes(set));
                HashKey::sha256(&bytes)
            })
            .collect::<Vec<_>>();
        let mut identities = HashMap::new();
        for (code_rule_index, code_rule) in code_rules.iter().enumerate() {
            for clause_index in 0..code_rule.clauses.len() {
                let id = (code_rule_index, clause_index);
                for (rule, scoped) in [
                    (rules[code_rule_index], false),
                    (set_rules[code_rule_index], true),
                ] {
                    identities.insert(
                        candidate_clause(rule, code_rule, clause_index),
                        (id, scoped),
                    );
                }
            }
        }
        Ok(Self {
            domain,
            catalog: identity(set)?,
            rules,
            set_rules,
            identities,
        })
    }

    /// The check a clause's result is stored under: the set-scoped one for a budget verdict.
    pub(super) fn candidate(
        &self,
        code_rules: &[Arc<CompiledCodeRule>],
        (code_rule_index, clause_index): ClauseId,
        failure: Option<LocalFailure>,
    ) -> CandidateClause {
        let rule = if failure.is_some_and(|(code, _)| code == "budget-exceeded") {
            self.set_rules[code_rule_index]
        } else {
            self.rules[code_rule_index]
        };
        candidate_clause(rule, &code_rules[code_rule_index], clause_index)
    }

    /// The clauses `persisted` answers among `expected`, each with its decoded result: a
    /// set-scoped result is preferred to the code rule's own, and among equals the later one is
    /// taken. Also whether some answering result did not decode (another encoding: a miss).
    pub(super) fn select<'a>(
        &self,
        persisted: impl IntoIterator<Item = &'a crate::content_store::StoredLocalResult>,
        expected: &HashSet<ClauseId>,
        path: &str,
    ) -> (HashMap<ClauseId, DecodedResult>, bool) {
        let mut selected = HashMap::<ClauseId, (bool, DecodedResult)>::new();
        let mut undecodable = false;
        for result in persisted {
            let Some(&(id, scoped)) = self.identities.get(&result.candidate) else {
                continue;
            };
            if !expected.contains(&id)
                || selected
                    .get(&id)
                    .is_some_and(|(chosen, _)| *chosen && !scoped)
            {
                continue;
            }
            match decode_local_result(&result.bytes, path) {
                Some(decoded) => {
                    selected.insert(id, (scoped, decoded));
                }
                None => undecodable = true,
            }
        }
        let selected = selected
            .into_iter()
            .map(|(id, (_, decoded))| (id, decoded))
            .collect();
        (selected, undecodable)
    }
}

type DecodedResult = (Vec<Witness>, Option<LocalFailure>, Option<String>);

fn sha256_bytes(key: HashKey) -> [u8; 32] {
    key.sha256_bytes().expect("result identities are sha-256")
}

/// A code rule's result identity: everything its clauses' results depend on besides the source,
/// the file context and the engine.
fn rule_identity(code_rule: &CompiledCodeRule) -> HashKey {
    let mut bytes = b"rerail-rule-result-v1\0".to_vec();
    bytes.extend_from_slice(code_rule.id.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(code_rule.digest.as_bytes());
    bytes.push(0xfe);
    bytes.extend_from_slice(code_rule.capabilities.raw.to_string().as_bytes());
    bytes.push(0xfd);
    for clause in &code_rule.clauses {
        bytes.extend_from_slice(clause.id.as_bytes());
        bytes.push(0xff);
    }
    // Results under one placement carry its modes and folders; another placement never reuses
    // them.
    if let Some(placement) = &code_rule.scope.placement {
        bytes.push(0xfc);
        bytes.extend_from_slice(&placement_identity(placement));
    }
    HashKey::sha256(&bytes)
}

/// Recent code rule sets' result keys, by their code rules' allocations: a code rule is not
/// changed in place while a weak reference to it is held (`Arc::make_mut` moves it), so a set
/// whose every code rule is still the one recorded has the recorded keys.
type ResultKeysMemo = (&'static str, Vec<Weak<CompiledCodeRule>>, Arc<ResultKeys>);
static RESULT_KEYS: LazyLock<parking_lot::Mutex<Vec<ResultKeysMemo>>> =
    LazyLock::new(Default::default);

/// The result keys of `code_rules`: every apply and restore reads them, so they are built once
/// per code-rule set.
pub(super) fn result_keys(code_rules: &[Arc<CompiledCodeRule>]) -> Result<Arc<ResultKeys>> {
    let engine = engine_build_id();
    let same = |(known_engine, known, _): &ResultKeysMemo| {
        *known_engine == engine
            && known.len() == code_rules.len()
            && known
                .iter()
                .zip(code_rules)
                .all(|(known, code_rule)| std::ptr::eq(known.as_ptr(), Arc::as_ptr(code_rule)))
            && known.iter().all(|known| known.strong_count() > 0)
    };
    if let Some((_, _, keys)) = RESULT_KEYS.lock().iter().find(|memo| same(memo)) {
        return Ok(Arc::clone(keys));
    }
    let keys = Arc::new(ResultKeys::new(engine, code_rules)?);
    let mut memo = RESULT_KEYS.lock();
    memo.retain(|(_, known, _)| known.iter().all(|known| known.strong_count() > 0));
    if memo.len() >= 16 {
        memo.remove(0);
    }
    memo.push((
        engine,
        code_rules.iter().map(Arc::downgrade).collect(),
        Arc::clone(&keys),
    ));
    Ok(keys)
}

/// What publishing a chunk of files did.
pub(super) enum Persisted {
    /// The files are stored. `busy_during` says which write after the publication (sharing the
    /// files with the snapshot, or renewing the lease) found the store busy past its write
    /// deadline. A busy snapshot promotion is skipped and only costs snapshot reuse for those
    /// files; maintenance redoes a lease renewal. The store is busy.
    Published {
        result: PublicationResult,
        busy_during: Option<&'static str>,
    },
    /// The store stayed busy past its write deadline and nothing was published; `retries` is
    /// what the publication spent waiting.
    Busy { retries: BusyRetries },
}

/// Publishes `paths`' evaluated files; `Persisted::Busy` when the store stayed busy past its
/// write deadline and nothing was published. The store is a cache: the caller keeps the files
/// resident, so the evaluation's results stand and only their reuse is lost.
pub(super) fn persist_files(
    storage: &StorageContext,
    paths: &[String],
    files: &BTreeMap<String, FileState>,
    code_rules: &[Arc<CompiledCodeRule>],
    consumers: &Consumers,
    closed: &AtomicBool,
) -> Result<Persisted> {
    let keys = result_keys(code_rules)?;
    let candidate =
        |&id: &ClauseId, failure: Option<LocalFailure>| keys.candidate(code_rules, id, failure);
    // Each file's payloads (tape bytes, encoded and compressed local results, blob alias) are
    // independent of every other file's, so they are built on the worker pool.
    let prepared = WORK_POOL.install(|| {
        paths
            .par_iter()
            .map(|path| {
                let file = files
                    .get(path)
                    .ok_or_else(|| invalid(format!("missing file for persistence: {path}")))?;
                let candidates = file
                    .candidates
                    .regex
                    .iter()
                    .chain(&file.candidates.ast)
                    .map(|id| candidate(id, None))
                    .collect::<Vec<_>>();
                let tape_bytes = {
                    #[cfg(feature = "code-rule-profile")]
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
                    .filter(|id| {
                        !file.transient_failure
                            && !file.restored_result_ids.contains(id)
                            && !file.unstored.contains(id)
                    })
                    .filter_map(|id| {
                        let context = if consumers.contains_key(id) {
                            stamped?
                        } else {
                            context
                        };
                        Some((id, context))
                    })
                    .map(|(id, context)| {
                        let witnesses = file.witnesses.get(id).map(Vec::as_slice).unwrap_or(&[]);
                        let failure = file.failures.get(id).copied();
                        let bytes =
                            encode_local_result(witnesses, failure, file.syntax.as_deref())?;
                        let frame =
                            compress(&bytes, 3).map_err(|error| invalid(error.to_string()))?;
                        Ok(LocalResultPublication {
                            candidate: candidate(id, failure),
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
                Ok((
                    file,
                    candidates,
                    tape_bytes,
                    grammar_id,
                    aliases,
                    local_results,
                ))
            })
            .collect::<Result<Vec<_>>>()
    })?;
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
                    catalog: keys.domain,
                    candidates,
                    local_results,
                    candidates_current: false,
                }
            },
        )
        .collect::<Vec<_>>();
    #[cfg(feature = "code-rule-profile")]
    let _store_timer = profile::PhaseTimer::start(profile::Phase::StoreWrite);
    let mut busy = BusyRetries::default();
    let result = match storage.store.publish_overlay_files_reporting_busy(
        storage.workspace_id,
        &publications,
        &|| closed.load(Ordering::Acquire),
        &mut busy,
    ) {
        Ok(result) => result,
        Err(StoreError::Busy) => return Ok(Persisted::Busy { retries: busy }),
        Err(error) => return Err(invalid(error.to_string())),
    };
    if closed.load(Ordering::Acquire) {
        return Err(invalid("policy workspace is closed"));
    }
    #[cfg(test)]
    if let Some(published) = AFTER_FILES_PUBLISHED.take() {
        published();
    }
    // The files are stored; sharing them with the snapshot and renewing the lease (which the
    // maintenance thread also does) only serve reuse, so a busy store skips them, and the caller
    // learns which write found it busy. A promotion that waited out the deadline leaves the
    // renewal unattempted: it would wait the same deadline again.
    let busy_store =
        |written: crate::content_store::StoreResult<()>, during: &'static str| match written {
            Ok(()) => Ok(None),
            Err(StoreError::Busy) => Ok(Some(during)),
            Err(error) => Err(invalid(error.to_string())),
        };
    let clean_paths = prepared
        .iter()
        .filter(|(_, _, _, _, aliases, _)| !aliases.is_empty())
        .map(|(file, _, _, _, _, _)| file.input.path.as_str())
        .collect::<Vec<_>>();
    let mut busy_during = busy_store(
        storage.store.promote_overlay_files_to_snapshot(
            storage.workspace_id,
            storage.snapshot_id,
            &clean_paths,
        ),
        "while sharing results with the snapshot",
    )?;
    if busy_during.is_none() {
        let now = unix_seconds()?;
        #[cfg(test)]
        LEASE_RENEWAL_ATTEMPTS.set(LEASE_RENEWAL_ATTEMPTS.get() + 1);
        busy_during = busy_store(
            storage
                .store
                .renew_workspace_lease(storage.workspace_id, &storage.owner, now + 120),
            "while renewing the workspace lease",
        )?;
    }
    Ok(Persisted::Published {
        result,
        busy_during,
    })
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
