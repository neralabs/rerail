//! Bulk overlay publication: object preparation, identity resolution, and the publish transaction.

use super::objects::validate_tape_payload;
use super::relationships::{
    execute_relationship_pass, prepare_relationship_pass, relationship_references_current,
};
use super::rows::{hash_params, validate_relative_path};
use super::{
    CODEC_ZSTD, CandidateCheck, ContentStore, HashKey, MAX_PUBLICATION_BATCH,
    OverlayFilePublication, PublicationResult, STRING_CHECK_KEY, STRING_PATH, SourceId, StoreError,
    StoreResult, StoredFile, TapeId, WorkspaceId,
};
use crate::WORK_POOL;
#[cfg(test)]
use parking_lot::Mutex as ParkingMutex;
use rayon::prelude::*;
use rusqlite::limits::Limit;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Cursor;
use std::time::{Duration, Instant};

#[cfg(test)]
pub(super) static AFTER_PUBLICATION_READ_HOOK: ParkingMutex<Option<Box<dyn FnOnce() + Send>>> =
    ParkingMutex::new(None);
#[cfg(test)]
pub(super) static BEFORE_RELATIONSHIP_WRITE_HOOK: ParkingMutex<Option<Box<dyn FnOnce() + Send>>> =
    ParkingMutex::new(None);

#[cfg(test)]
fn run_publication_test_hook(hook: &ParkingMutex<Option<Box<dyn FnOnce() + Send>>>) {
    if let Some(hook) = hook.lock().take() {
        hook();
    }
}

impl ContentStore {
    pub fn publish_overlay_file(
        &self,
        workspace_id: WorkspaceId,
        publication: OverlayFilePublication<'_>,
    ) -> StoreResult<StoredFile> {
        self.publish_overlay_files(workspace_id, &[publication])?
            .files
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Invalid("overlay publication is empty".to_owned()))
    }

    pub fn publish_overlay_files(
        &self,
        workspace_id: WorkspaceId,
        publications: &[OverlayFilePublication<'_>],
    ) -> StoreResult<PublicationResult> {
        self.publish_overlay_files_cancellable(workspace_id, publications, &|| false)
    }

    pub fn publish_overlay_files_cancellable(
        &self,
        workspace_id: WorkspaceId,
        publications: &[OverlayFilePublication<'_>],
        cancelled: &dyn Fn() -> bool,
    ) -> StoreResult<PublicationResult> {
        self.publish_overlay_files_cancellable_attempt(workspace_id, publications, cancelled, 0)
    }

    fn publish_overlay_files_cancellable_attempt(
        &self,
        workspace_id: WorkspaceId,
        publications: &[OverlayFilePublication<'_>],
        cancelled: &dyn Fn() -> bool,
        stale_relationship_retries: usize,
    ) -> StoreResult<PublicationResult> {
        if cancelled() {
            return Err(StoreError::Invalid(
                "overlay publication was cancelled".to_owned(),
            ));
        }
        self.require_write_capacity()?;
        if publications.is_empty() || publications.len() > MAX_PUBLICATION_BATCH {
            return Err(StoreError::Invalid(format!(
                "overlay publication batch must contain 1 to {MAX_PUBLICATION_BATCH} files"
            )));
        }
        let identities = WORK_POOL.install(|| {
            publications
                .par_iter()
                .map(|publication| {
                    validate_relative_path(publication.path)?;
                    if publication.state_id <= 0 {
                        return Err(StoreError::Invalid(
                            "overlay state must be positive".to_owned(),
                        ));
                    }
                    validate_tape_payload(publication.tape)?;
                    Ok((
                        publication,
                        HashKey::sha256(publication.source),
                        publication.tape.map(|tape| HashKey::sha256(tape.bytes)),
                    ))
                })
                .collect::<StoreResult<Vec<_>>>()
        })?;
        let candidate_identities = publication_candidate_identities(publications);
        // Path identities are hashed once, before any transaction takes the write lock.
        let path_hashes = publications
            .iter()
            .map(|publication| {
                (
                    publication.path,
                    HashKey::sha256(publication.path.as_bytes()),
                )
            })
            .collect::<Vec<_>>();
        let (existing, initial_path_ids, initial_interned) = {
            let mut connection = self
                .connection
                .lock()
                .expect("content store connection unavailable");
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let existing = stored_object_identities(&transaction, &identities)?;
            let path_ids = stored_path_ids(&transaction, &path_hashes)?;
            let interned = resolve_publication_identities(&transaction, &candidate_identities)?;
            transaction.commit()?;
            (existing, path_ids, interned)
        };
        let initial_objects = existing.clone();
        // Source and tape are always independent Zstandard frames.
        // Hashing, lookup, compression, and validation finish before the transaction starts.
        let mut prepared: Vec<PreparedOverlayPublication<'_>> = WORK_POOL.install(|| {
            identities
                .into_par_iter()
                .zip(existing.into_par_iter())
                .map(
                    |((publication, source_hash, tape_hash), (source_id, tape_id))| {
                        let source_frame = source_id
                            .is_none()
                            .then(|| {
                                zstd::stream::encode_all(
                                    Cursor::new(publication.source),
                                    self.compression_level,
                                )
                            })
                            .transpose()?;
                        let tape_frame = publication
                            .tape
                            .filter(|_| tape_id.is_none())
                            .map(|payload| {
                                zstd::stream::encode_all(
                                    Cursor::new(payload.bytes),
                                    self.compression_level,
                                )
                            })
                            .transpose()?;
                        Ok((
                            publication,
                            source_hash,
                            source_id,
                            source_frame,
                            tape_hash,
                            tape_id,
                            tape_frame,
                        ))
                    },
                )
                .collect::<StoreResult<Vec<_>>>()
        })?;
        let reused_sources = prepared.iter().filter(|item| item.2.is_some()).count();
        let reused_tapes = prepared.iter().filter(|item| item.5.is_some()).count();
        let compressed_sources = prepared.len() - reused_sources;
        let compressed_tapes = prepared
            .iter()
            .filter(|item| item.0.tape.is_some() && item.5.is_none())
            .count();
        let expected_catalogs = candidate_identities
            .iter()
            .map(|(catalog, _)| *catalog)
            .collect::<BTreeSet<_>>()
            .len();
        let expected_checks = candidate_identities
            .iter()
            .filter(|(_, candidate)| candidate.is_some())
            .count();
        let expected_paths = publications
            .iter()
            .map(|publication| publication.path)
            .collect::<BTreeSet<_>>()
            .len();
        let mut retries = 0;
        let mut retry_wait_nanoseconds = 0_u64;
        let mut transaction_nanoseconds = 0_u64;
        let mut object_transaction_nanoseconds = 0_u64;
        let mut relationship_transaction_nanoseconds = 0_u64;
        let mut read_data_statements = 3_usize;
        let mut data_statements = 0_usize;
        let mut max_data_statements_per_pass = 0_usize;
        // The initial read snapshot executes BEGIN and COMMIT separately from its data statements.
        let mut transaction_statements = 2_usize;
        let mut reconciliation_passes = 0_usize;
        let initial_complete = initial_objects
            .iter()
            .enumerate()
            .all(|(index, (source, tape))| {
                source.is_some() && (prepared[index].0.tape.is_none() || tape.is_some())
            })
            && initial_path_ids.len() == expected_paths
            && initial_interned.catalogs.len() == expected_catalogs
            && initial_interned.checks.len() == expected_checks
            && publications
                .iter()
                .all(|publication| publication.aliases.is_empty());
        let (stored_objects, path_ids, known_interned) = if initial_complete {
            (initial_objects, initial_path_ids, initial_interned)
        } else {
            #[cfg(test)]
            run_publication_test_hook(&AFTER_PUBLICATION_READ_HOOK);
            let mut current_objects = initial_objects;
            let mut current_path_ids = initial_path_ids;
            let mut current_interned = initial_interned;
            loop {
                if cancelled() {
                    return Err(StoreError::Invalid(
                        "overlay publication was cancelled".to_owned(),
                    ));
                }
                prepare_missing_object_frames(
                    &mut prepared,
                    &current_objects,
                    self.compression_level,
                )?;
                let object_pass = prepare_object_pass(
                    &prepared,
                    &current_objects,
                    &current_path_ids,
                    &current_interned,
                )?;
                loop {
                    let attempt_started = Instant::now();
                    let result = (|| {
                        let mut connection = self
                            .connection
                            .lock()
                            .expect("content store connection unavailable");
                        let transaction =
                            connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                        #[cfg(test)]
                        let _write_lock = super::WriteLockSection::enter();
                        let started = Instant::now();
                        let statement_count = execute_object_pass(&transaction, &object_pass)?;
                        if statement_count > 10 {
                            return Err(StoreError::Invalid(format!(
                                "object publication used {} statements; maximum is 10",
                                statement_count
                            )));
                        }
                        transaction.commit()?;
                        Ok((
                            u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                            statement_count,
                        ))
                    })();
                    match result {
                        Ok((elapsed, statement_count)) => {
                            transaction_nanoseconds =
                                transaction_nanoseconds.saturating_add(elapsed);
                            object_transaction_nanoseconds =
                                object_transaction_nanoseconds.saturating_add(elapsed);
                            data_statements += statement_count;
                            max_data_statements_per_pass =
                                max_data_statements_per_pass.max(statement_count);
                            transaction_statements += 2;
                            reconciliation_passes += 1;
                            break;
                        }
                        Err(StoreError::Busy) if retries < 8 => {
                            retries += 1;
                            let delay = Duration::from_millis(1_u64 << retries.min(6));
                            retry_wait_nanoseconds = retry_wait_nanoseconds.saturating_add(
                                u64::try_from(
                                    attempt_started.elapsed().saturating_add(delay).as_nanos(),
                                )
                                .unwrap_or(u64::MAX),
                            );
                            std::thread::sleep(delay);
                        }
                        Err(error) => return Err(error),
                    }
                }

                let corrected_identities = prepared
                    .iter()
                    .map(|item| (item.0, item.1, item.4))
                    .collect::<Vec<_>>();
                let (stored_objects, path_ids, known_interned) = {
                    let mut connection = self
                        .connection
                        .lock()
                        .expect("content store connection unavailable");
                    let transaction =
                        connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
                    let stored_objects =
                        stored_object_identities(&transaction, &corrected_identities)?;
                    let path_ids = stored_path_ids(&transaction, &path_hashes)?;
                    let known_interned =
                        resolve_publication_identities(&transaction, &candidate_identities)?;
                    transaction.commit()?;
                    (stored_objects, path_ids, known_interned)
                };
                read_data_statements += 3;
                transaction_statements += 2;
                let complete = stored_objects
                    .iter()
                    .enumerate()
                    .all(|(index, (source, tape))| {
                        source.is_some() && (prepared[index].0.tape.is_none() || tape.is_some())
                    })
                    && path_ids.len() == expected_paths
                    && known_interned.catalogs.len() == expected_catalogs
                    && known_interned.checks.len() == expected_checks;
                if complete {
                    break (stored_objects, path_ids, known_interned);
                }
                if reconciliation_passes >= 8 {
                    return Err(StoreError::Invalid(
                        "overlay publication made no progress after 8 reconciliation passes"
                            .to_owned(),
                    ));
                }
                current_objects = stored_objects;
                current_path_ids = path_ids;
                current_interned = known_interned;
            }
        };
        let relationship_hashes = prepared
            .iter()
            .map(|item| (item.1, item.4))
            .collect::<Vec<_>>();
        let mut stored = Vec::with_capacity(prepared.len());
        let mut overlay_rows = Vec::with_capacity(prepared.len());
        for ((publication, _, _, _, _, _, _), (source_id, tape_id)) in
            prepared.iter().zip(stored_objects)
        {
            let source_id = source_id.ok_or_else(|| StoreError::Busy)?;
            if publication.tape.is_some() && tape_id.is_none() {
                return Err(StoreError::Busy);
            }
            let path_id = *path_ids.get(publication.path).ok_or(StoreError::Busy)?;
            overlay_rows.push((
                path_id,
                publication.state_id,
                source_id,
                tape_id,
                **publication,
            ));
            stored.push(StoredFile {
                source: source_id,
                tape: tape_id,
            });
        }

        let variable_limit = usize::try_from(
            self.connection
                .lock()
                .expect("content store connection unavailable")
                .limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)?,
        )
        .map_err(|_| StoreError::Invalid("SQLite variable limit is negative".to_owned()))?;
        let relationship_pass = prepare_relationship_pass(
            workspace_id,
            &overlay_rows,
            &known_interned,
            variable_limit,
        )?;
        #[cfg(test)]
        run_publication_test_hook(&BEFORE_RELATIONSHIP_WRITE_HOOK);
        loop {
            if cancelled() {
                return Err(StoreError::Invalid(
                    "overlay publication was cancelled".to_owned(),
                ));
            }
            let attempt_started = Instant::now();
            let result = (|| {
                let mut connection = self
                    .connection
                    .lock()
                    .expect("content store connection unavailable");
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                #[cfg(test)]
                let _write_lock = super::WriteLockSection::enter();
                let transaction_started = Instant::now();
                if !relationship_references_current(
                    &transaction,
                    &overlay_rows,
                    &path_hashes,
                    &relationship_hashes,
                    &candidate_identities,
                    &known_interned,
                )? {
                    return Ok((
                        u64::try_from(transaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                        0,
                        false,
                    ));
                }
                execute_relationship_pass(&transaction, &relationship_pass)?;
                let statement_count = relationship_pass.statements.len();
                if cancelled() {
                    return Err(StoreError::Invalid(
                        "overlay publication was cancelled".to_owned(),
                    ));
                }
                transaction.commit()?;
                Ok((
                    u64::try_from(transaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    statement_count,
                    true,
                ))
            })();
            match result {
                Ok((elapsed, statement_count, true)) => {
                    transaction_nanoseconds = transaction_nanoseconds.saturating_add(elapsed);
                    relationship_transaction_nanoseconds =
                        relationship_transaction_nanoseconds.saturating_add(elapsed);
                    data_statements += statement_count;
                    max_data_statements_per_pass =
                        max_data_statements_per_pass.max(statement_count);
                    transaction_statements += 2;
                    reconciliation_passes += 1;
                    if publications
                        .iter()
                        .any(|publication| !publication.local_results.is_empty())
                    {
                        self.local_result_cache
                            .lock()
                            .expect("local result cache mutex poisoned")
                            .clear();
                    }
                    return Ok(PublicationResult {
                        files: stored,
                        retries,
                        retry_wait_nanoseconds,
                        transaction_nanoseconds,
                        object_transaction_nanoseconds,
                        relationship_transaction_nanoseconds,
                        read_data_statements,
                        data_statements,
                        max_data_statements_per_pass,
                        transaction_statements,
                        reconciliation_passes,
                        reused_sources,
                        reused_tapes,
                        compressed_sources,
                        compressed_tapes,
                    });
                }
                Ok((_, _, false)) => {
                    if stale_relationship_retries >= 8 {
                        return Err(StoreError::Invalid(
                            "overlay publication made no progress after 8 reconciliation passes"
                                .to_owned(),
                        ));
                    }
                    return self.publish_overlay_files_cancellable_attempt(
                        workspace_id,
                        publications,
                        cancelled,
                        stale_relationship_retries + 1,
                    );
                }
                Err(StoreError::Busy) if retries < 8 => {
                    retries += 1;
                    let delay = Duration::from_millis(1_u64 << retries.min(6));
                    let wait = attempt_started.elapsed().saturating_add(delay);
                    retry_wait_nanoseconds = retry_wait_nanoseconds
                        .saturating_add(u64::try_from(wait.as_nanos()).unwrap_or(u64::MAX));
                    std::thread::sleep(delay);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

type PreparedOverlayPublication<'a> = (
    &'a OverlayFilePublication<'a>,
    HashKey,
    Option<SourceId>,
    Option<Vec<u8>>,
    Option<HashKey>,
    Option<TapeId>,
    Option<Vec<u8>>,
);

struct PreparedObjectPass {
    hashes_json: String,
    strings_json: String,
    paths_json: String,
    catalogs_json: String,
    checks_json: String,
    aliases_json: String,
    source_sql: String,
    source_values: Vec<rusqlite::types::Value>,
    tape_sql: String,
    tape_values: Vec<rusqlite::types::Value>,
}

fn hash_json(hash: HashKey) -> serde_json::Value {
    serde_json::json!([
        hash.algorithm,
        hash.lanes[0],
        hash.lanes[1],
        hash.lanes[2],
        hash.lanes[3]
    ])
}

fn encode_json(value: &impl serde::Serialize, name: &str) -> StoreResult<String> {
    serde_json::to_string(value)
        .map_err(|error| StoreError::Invalid(format!("{name} cannot be encoded: {error}")))
}

fn prepare_missing_object_frames(
    prepared: &mut [PreparedOverlayPublication<'_>],
    stored_objects: &[(Option<SourceId>, Option<TapeId>)],
    compression_level: i32,
) -> StoreResult<()> {
    for ((publication, _, _, source_frame, _, _, tape_frame), (source_id, tape_id)) in
        prepared.iter_mut().zip(stored_objects)
    {
        if source_id.is_none() && source_frame.is_none() {
            *source_frame = Some(zstd::stream::encode_all(
                Cursor::new(publication.source),
                compression_level,
            )?);
        }
        if let Some(tape) = publication.tape
            && tape_id.is_none()
            && tape_frame.is_none()
        {
            *tape_frame = Some(zstd::stream::encode_all(
                Cursor::new(tape.bytes),
                compression_level,
            )?);
        }
    }
    Ok(())
}

fn prepare_object_pass(
    prepared: &[PreparedOverlayPublication<'_>],
    stored_objects: &[(Option<SourceId>, Option<TapeId>)],
    known_paths: &HashMap<String, i64>,
    known_interned: &PublicationInternCache,
) -> StoreResult<PreparedObjectPass> {
    let mut hashes = BTreeSet::new();
    let mut strings = BTreeMap::<(i64, String), HashKey>::new();
    let mut paths = BTreeMap::<String, HashKey>::new();
    let mut catalogs = BTreeSet::new();
    let mut checks = BTreeMap::<(HashKey, HashKey, String), HashKey>::new();
    let mut aliases = BTreeSet::<(HashKey, HashKey)>::new();
    let mut source_rows = Vec::new();
    let mut tape_rows = Vec::new();

    for (
        (publication, source_hash, _, source_frame, tape_hash, _, tape_frame),
        (source_id, tape_id),
    ) in prepared.iter().zip(stored_objects)
    {
        if source_id.is_none() {
            hashes.insert(*source_hash);
        }
        if !known_paths.contains_key(publication.path) {
            let path_hash = HashKey::sha256(publication.path.as_bytes());
            hashes.insert(path_hash);
            strings.insert((STRING_PATH, publication.path.to_owned()), path_hash);
            paths.insert(publication.path.to_owned(), path_hash);
        }
        let catalog = publication.catalog.key();
        let known_catalog = known_interned.catalogs.get(&catalog).copied();
        if known_catalog.is_none() {
            hashes.insert(catalog);
            catalogs.insert(catalog);
        }
        for alias in publication.aliases {
            hashes.insert(*alias);
            aliases.insert((*alias, *source_hash));
        }
        for result in publication.local_results {
            hashes.insert(result.context);
        }
        let candidates = publication.candidates.iter().chain(
            publication
                .local_results
                .iter()
                .map(|result| &result.candidate),
        );
        for candidate in candidates {
            let known = known_catalog.is_some_and(|catalog_id| {
                known_interned.checks.contains_key(&(
                    catalog_id,
                    candidate.policy,
                    candidate.check.clone(),
                ))
            });
            if !known {
                hashes.insert(candidate.policy);
                let check_hash = HashKey::sha256(candidate.check.as_bytes());
                hashes.insert(check_hash);
                strings.insert((STRING_CHECK_KEY, candidate.check.clone()), check_hash);
                checks.insert(
                    (catalog, candidate.policy, candidate.check.clone()),
                    check_hash,
                );
            }
        }
        if source_id.is_none() {
            let frame = source_frame.as_ref().ok_or(StoreError::Busy)?;
            source_rows.push((*source_hash, publication.source.len(), frame.clone()));
        }
        if let (Some(payload), Some(hash)) = (publication.tape, *tape_hash)
            && tape_id.is_none()
        {
            let frame = tape_frame.as_ref().ok_or(StoreError::Busy)?;
            hashes.insert(hash);
            tape_rows.push((*source_hash, hash, payload, frame.clone()));
        }
    }

    let hashes_json = encode_json(
        &hashes.into_iter().map(hash_json).collect::<Vec<_>>(),
        "hash identities",
    )?;
    let strings_json = encode_json(
        &strings
            .into_iter()
            .map(|((domain, value), hash)| serde_json::json!([domain, value, hash_json(hash)]))
            .collect::<Vec<_>>(),
        "string identities",
    )?;
    let paths_json = encode_json(
        &paths
            .into_iter()
            .map(|(path, hash)| serde_json::json!([path, hash_json(hash)]))
            .collect::<Vec<_>>(),
        "path identities",
    )?;
    let catalogs_json = encode_json(
        &catalogs.into_iter().map(hash_json).collect::<Vec<_>>(),
        "catalog identities",
    )?;
    let checks_json = encode_json(
        &checks
            .into_iter()
            .map(|((catalog, policy, check), check_hash)| {
                serde_json::json!([
                    hash_json(catalog),
                    hash_json(policy),
                    check,
                    hash_json(check_hash)
                ])
            })
            .collect::<Vec<_>>(),
        "check identities",
    )?;
    let aliases_json = encode_json(
        &aliases
            .into_iter()
            .map(|(alias, source)| serde_json::json!([hash_json(alias), hash_json(source)]))
            .collect::<Vec<_>>(),
        "source aliases",
    )?;

    let mut source_sql =
        String::from("WITH input(algorithm,lane0,lane1,lane2,lane3,raw_length,frame) AS (VALUES ");
    let mut source_values = Vec::with_capacity(source_rows.len() * 7);
    if source_rows.is_empty() {
        source_sql.push_str("(0,0,0,0,0,0,NULL)) SELECT 0,0,0,0,NULL,0 WHERE 0");
    } else {
        for (index, (hash, raw_length, frame)) in source_rows.into_iter().enumerate() {
            if index != 0 {
                source_sql.push(',');
            }
            source_sql.push_str("(?,?,?,?,?,?,?)");
            source_values.extend(
                hash_params(hash)
                    .into_iter()
                    .map(rusqlite::types::Value::Integer),
            );
            source_values.push(
                i64::try_from(raw_length)
                    .map_err(|_| {
                        StoreError::Invalid("source length exceeds SQLite range".to_owned())
                    })?
                    .into(),
            );
            source_values.push(frame.into());
        }
        source_sql.push_str(
            ") INSERT OR IGNORE INTO source_objects(canonical_hash_id,raw_length,codec_id,frame,created_generation) \
             SELECT h.hash_id,i.raw_length,? ,i.frame,(SELECT generation FROM store_state WHERE singleton=1) \
             FROM input i JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
             AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3",
        );
        source_values.push(CODEC_ZSTD.into());
    }

    let mut tape_sql = String::from(
        "WITH input(source_algorithm,source_lane0,source_lane1,source_lane2,source_lane3,tape_algorithm,tape_lane0,tape_lane1,tape_lane2,tape_lane3,grammar_id,grammar_version,format_version,raw_length,frame) AS (VALUES ",
    );
    let mut tape_values = Vec::with_capacity(tape_rows.len() * 15);
    if tape_rows.is_empty() {
        tape_sql.push_str("(0,0,0,0,0,0,0,0,0,0,0,0,0,0,NULL)) SELECT 0,0,0,0,0,0,0,0,0 WHERE 0");
    } else {
        for (index, (source, hash, payload, frame)) in tape_rows.into_iter().enumerate() {
            if index != 0 {
                tape_sql.push(',');
            }
            tape_sql.push_str("(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)");
            tape_values.extend(
                hash_params(source)
                    .into_iter()
                    .map(rusqlite::types::Value::Integer),
            );
            tape_values.extend(
                hash_params(hash)
                    .into_iter()
                    .map(rusqlite::types::Value::Integer),
            );
            tape_values.push(payload.grammar_id.into());
            tape_values.push(payload.grammar_version.into());
            tape_values.push(payload.tape_format_version.into());
            tape_values.push(
                i64::try_from(payload.bytes.len())
                    .map_err(|_| {
                        StoreError::Invalid("tape length exceeds SQLite range".to_owned())
                    })?
                    .into(),
            );
            tape_values.push(frame.into());
        }
        tape_sql.push_str(
            ") INSERT OR IGNORE INTO tape_objects(source_id,canonical_hash_id,grammar_id,grammar_version,tape_format_version,codec_id,raw_length,frame,created_generation) \
             SELECT so.source_id,th.hash_id,i.grammar_id,i.grammar_version,i.format_version,?,i.raw_length,i.frame,\
             (SELECT generation FROM store_state WHERE singleton=1) FROM input i \
             JOIN hashes sh INDEXED BY hashes_by_prefix ON sh.algorithm_id=i.source_algorithm AND sh.lane0=i.source_lane0 \
             AND sh.lane1=i.source_lane1 AND sh.lane2=i.source_lane2 AND sh.lane3=i.source_lane3 \
             JOIN source_objects so ON so.canonical_hash_id=sh.hash_id \
             JOIN hashes th INDEXED BY hashes_by_prefix ON th.algorithm_id=i.tape_algorithm AND th.lane0=i.tape_lane0 \
             AND th.lane1=i.tape_lane1 AND th.lane2=i.tape_lane2 AND th.lane3=i.tape_lane3",
        );
        tape_values.push(CODEC_ZSTD.into());
    }

    Ok(PreparedObjectPass {
        hashes_json,
        strings_json,
        paths_json,
        catalogs_json,
        checks_json,
        aliases_json,
        source_sql,
        source_values,
        tape_sql,
        tape_values,
    })
}

fn execute_object_pass(
    transaction: &Transaction<'_>,
    pass: &PreparedObjectPass,
) -> StoreResult<usize> {
    let mut statements = 0;
    transaction.prepare_cached(
        "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) algorithm,\
         CAST(json_extract(value,'$[1]') AS INTEGER) lane0,CAST(json_extract(value,'$[2]') AS INTEGER) lane1,\
         CAST(json_extract(value,'$[3]') AS INTEGER) lane2,CAST(json_extract(value,'$[4]') AS INTEGER) lane3 FROM json_each(?1)) \
         INSERT INTO hashes(algorithm_id,lane0,lane1,lane2,lane3) \
         SELECT i.algorithm,i.lane0,i.lane1,i.lane2,i.lane3 FROM input i \
         WHERE NOT EXISTS (SELECT 1 FROM hashes h INDEXED BY hashes_by_prefix \
         WHERE h.algorithm_id=i.algorithm AND h.lane0=i.lane0 AND h.lane1=i.lane1 \
         AND h.lane2=i.lane2 AND h.lane3=i.lane3)",
    )?.execute([&pass.hashes_json])?;
    statements += 1;
    transaction.prepare_cached(
        "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) domain_id,\
         json_extract(value,'$[1]') string_value,CAST(json_extract(value,'$[2][0]') AS INTEGER) algorithm,\
         CAST(json_extract(value,'$[2][1]') AS INTEGER) lane0,CAST(json_extract(value,'$[2][2]') AS INTEGER) lane1,\
         CAST(json_extract(value,'$[2][3]') AS INTEGER) lane2,CAST(json_extract(value,'$[2][4]') AS INTEGER) lane3 FROM json_each(?1)) \
         INSERT OR IGNORE INTO strings(domain_id,content_hash_id,collision_ordinal,value) \
         SELECT i.domain_id,h.hash_id,COALESCE((SELECT MAX(s.collision_ordinal)+1 FROM strings s \
         WHERE s.domain_id=i.domain_id AND s.content_hash_id=h.hash_id),0),i.string_value FROM input i \
         JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
         AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3 \
         WHERE NOT EXISTS (SELECT 1 FROM strings s WHERE s.domain_id=i.domain_id \
         AND s.content_hash_id=h.hash_id AND s.value=i.string_value)",
    )?.execute([&pass.strings_json])?;
    statements += 1;
    transaction.prepare_cached(
        "WITH input AS (SELECT json_extract(value,'$[0]') path_value,\
         CAST(json_extract(value,'$[1][0]') AS INTEGER) algorithm,CAST(json_extract(value,'$[1][1]') AS INTEGER) lane0,\
         CAST(json_extract(value,'$[1][2]') AS INTEGER) lane1,CAST(json_extract(value,'$[1][3]') AS INTEGER) lane2,\
         CAST(json_extract(value,'$[1][4]') AS INTEGER) lane3 FROM json_each(?1)) \
         INSERT OR IGNORE INTO paths(string_id) SELECT s.string_id FROM input i \
         CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
         AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3 \
         CROSS JOIN strings s ON s.domain_id=?2 AND s.content_hash_id=h.hash_id AND s.value=i.path_value",
    )?.execute(params![pass.paths_json, STRING_PATH])?;
    statements += 1;
    transaction.prepare_cached(
        "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) algorithm,\
         CAST(json_extract(value,'$[1]') AS INTEGER) lane0,CAST(json_extract(value,'$[2]') AS INTEGER) lane1,\
         CAST(json_extract(value,'$[3]') AS INTEGER) lane2,CAST(json_extract(value,'$[4]') AS INTEGER) lane3 FROM json_each(?1)) \
         INSERT OR IGNORE INTO candidate_catalogs(identity_hash_id) SELECT h.hash_id FROM input i \
         JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
         AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3",
    )?.execute([&pass.catalogs_json])?;
    statements += 1;
    transaction.prepare_cached(
        "WITH input AS (SELECT \
         CAST(json_extract(value,'$[0][0]') AS INTEGER) ca,CAST(json_extract(value,'$[0][1]') AS INTEGER) c0,\
         CAST(json_extract(value,'$[0][2]') AS INTEGER) c1,CAST(json_extract(value,'$[0][3]') AS INTEGER) c2,CAST(json_extract(value,'$[0][4]') AS INTEGER) c3,\
         CAST(json_extract(value,'$[1][0]') AS INTEGER) pa,CAST(json_extract(value,'$[1][1]') AS INTEGER) p0,\
         CAST(json_extract(value,'$[1][2]') AS INTEGER) p1,CAST(json_extract(value,'$[1][3]') AS INTEGER) p2,CAST(json_extract(value,'$[1][4]') AS INTEGER) p3,\
         json_extract(value,'$[2]') check_value,CAST(json_extract(value,'$[3][0]') AS INTEGER) sa,\
         CAST(json_extract(value,'$[3][1]') AS INTEGER) s0,CAST(json_extract(value,'$[3][2]') AS INTEGER) s1,\
         CAST(json_extract(value,'$[3][3]') AS INTEGER) s2,CAST(json_extract(value,'$[3][4]') AS INTEGER) s3 FROM json_each(?1)) \
         INSERT OR IGNORE INTO candidate_checks(catalog_id,policy_hash_id,check_string_id) \
         SELECT c.catalog_id,ph.hash_id,s.string_id FROM input i \
         JOIN hashes ch INDEXED BY hashes_by_prefix ON ch.algorithm_id=i.ca AND ch.lane0=i.c0 AND ch.lane1=i.c1 AND ch.lane2=i.c2 AND ch.lane3=i.c3 \
         JOIN candidate_catalogs c ON c.identity_hash_id=ch.hash_id \
         JOIN hashes ph INDEXED BY hashes_by_prefix ON ph.algorithm_id=i.pa AND ph.lane0=i.p0 AND ph.lane1=i.p1 AND ph.lane2=i.p2 AND ph.lane3=i.p3 \
         JOIN hashes sh INDEXED BY hashes_by_prefix ON sh.algorithm_id=i.sa AND sh.lane0=i.s0 AND sh.lane1=i.s1 AND sh.lane2=i.s2 AND sh.lane3=i.s3 \
         JOIN strings s ON s.domain_id=?2 AND s.content_hash_id=sh.hash_id AND s.value=i.check_value",
    )?.execute(params![pass.checks_json, STRING_CHECK_KEY])?;
    statements += 1;
    transaction
        .prepare_cached(&pass.source_sql)?
        .execute(rusqlite::params_from_iter(pass.source_values.iter()))?;
    statements += 1;
    let alias_count = transaction.prepare_cached(
        "WITH input AS (SELECT \
         CAST(json_extract(value,'$[0][0]') AS INTEGER) aa,CAST(json_extract(value,'$[0][1]') AS INTEGER) a0,\
         CAST(json_extract(value,'$[0][2]') AS INTEGER) a1,CAST(json_extract(value,'$[0][3]') AS INTEGER) a2,CAST(json_extract(value,'$[0][4]') AS INTEGER) a3,\
         CAST(json_extract(value,'$[1][0]') AS INTEGER) sa,CAST(json_extract(value,'$[1][1]') AS INTEGER) s0,\
         CAST(json_extract(value,'$[1][2]') AS INTEGER) s1,CAST(json_extract(value,'$[1][3]') AS INTEGER) s2,CAST(json_extract(value,'$[1][4]') AS INTEGER) s3 FROM json_each(?1)) \
         INSERT INTO source_aliases(alias_hash_id,source_id) SELECT ah.hash_id,so.source_id FROM input i \
         JOIN hashes ah INDEXED BY hashes_by_prefix ON ah.algorithm_id=i.aa AND ah.lane0=i.a0 AND ah.lane1=i.a1 AND ah.lane2=i.a2 AND ah.lane3=i.a3 \
         JOIN hashes sh INDEXED BY hashes_by_prefix ON sh.algorithm_id=i.sa AND sh.lane0=i.s0 AND sh.lane1=i.s1 AND sh.lane2=i.s2 AND sh.lane3=i.s3 \
         JOIN source_objects so ON so.canonical_hash_id=sh.hash_id \
         ON CONFLICT(alias_hash_id) DO UPDATE SET source_id=source_aliases.source_id \
         WHERE source_aliases.source_id=excluded.source_id",
    )?.execute([&pass.aliases_json])?;
    let expected_aliases: usize =
        serde_json::from_str::<Vec<serde_json::Value>>(&pass.aliases_json)
            .map_err(|error| {
                StoreError::Invalid(format!("source aliases cannot be decoded: {error}"))
            })?
            .len();
    if alias_count != expected_aliases {
        return Err(StoreError::Corrupt(
            "Git object alias maps to different source bytes".to_owned(),
        ));
    }
    statements += 1;
    transaction
        .prepare_cached(&pass.tape_sql)?
        .execute(rusqlite::params_from_iter(pass.tape_values.iter()))?;
    statements += 1;
    Ok(statements)
}

pub(super) type PublicationCandidateIdentity = (HashKey, Option<CandidateCheck>);

fn publication_candidate_identities(
    publications: &[OverlayFilePublication<'_>],
) -> Vec<PublicationCandidateIdentity> {
    let mut identities =
        BTreeMap::<(HashKey, Option<(HashKey, String)>), PublicationCandidateIdentity>::new();
    for publication in publications {
        let catalog = publication.catalog.key();
        identities.entry((catalog, None)).or_insert((catalog, None));
        for candidate in publication.candidates.iter().chain(
            publication
                .local_results
                .iter()
                .map(|result| &result.candidate),
        ) {
            let key = (candidate.policy, candidate.check.clone());
            identities
                .entry((catalog, Some(key)))
                .or_insert_with(|| (catalog, Some(candidate.clone())));
        }
    }
    identities.into_values().collect()
}

pub(super) fn resolve_publication_identities(
    connection: &Connection,
    identities: &[PublicationCandidateIdentity],
) -> StoreResult<PublicationInternCache> {
    let input = identities
        .iter()
        .map(|(catalog, candidate)| {
            let policy = candidate.as_ref().map(|candidate| candidate.policy);
            serde_json::json!([
                catalog.algorithm,
                catalog.lanes[0],
                catalog.lanes[1],
                catalog.lanes[2],
                catalog.lanes[3],
                policy.map(|key| key.algorithm),
                policy.map(|key| key.lanes[0]),
                policy.map(|key| key.lanes[1]),
                policy.map(|key| key.lanes[2]),
                policy.map(|key| key.lanes[3]),
                candidate.as_ref().map(|candidate| candidate.check.as_str()),
            ])
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_string(&input).map_err(|error| {
        StoreError::Invalid(format!("publication identities cannot be encoded: {error}"))
    })?;
    let mut statement = connection.prepare_cached(
        r#"WITH input AS (
             SELECT CAST(key AS INTEGER) ordinal,
               CAST(json_extract(value,'$[0]') AS INTEGER) catalog_algorithm,
               CAST(json_extract(value,'$[1]') AS INTEGER) catalog_lane0,
               CAST(json_extract(value,'$[2]') AS INTEGER) catalog_lane1,
               CAST(json_extract(value,'$[3]') AS INTEGER) catalog_lane2,
               CAST(json_extract(value,'$[4]') AS INTEGER) catalog_lane3,
               CAST(json_extract(value,'$[5]') AS INTEGER) policy_algorithm,
               CAST(json_extract(value,'$[6]') AS INTEGER) policy_lane0,
               CAST(json_extract(value,'$[7]') AS INTEGER) policy_lane1,
               CAST(json_extract(value,'$[8]') AS INTEGER) policy_lane2,
               CAST(json_extract(value,'$[9]') AS INTEGER) policy_lane3,
               json_extract(value,'$[10]') check_value
             FROM json_each(?1)
           )
           SELECT i.ordinal,c.catalog_id,
             CASE WHEN i.policy_algorithm IS NULL THEN NULL ELSE (
               SELECT cc.check_id FROM candidate_checks cc
               JOIN strings s ON s.string_id=cc.check_string_id
               WHERE cc.catalog_id=c.catalog_id AND cc.policy_hash_id=ph.hash_id
                 AND s.domain_id=?2 AND s.value=i.check_value LIMIT 1
             ) END
           FROM input i
           CROSS JOIN hashes ch INDEXED BY hashes_by_prefix
             ON ch.algorithm_id=i.catalog_algorithm AND ch.lane0=i.catalog_lane0
             AND ch.lane1=i.catalog_lane1 AND ch.lane2=i.catalog_lane2 AND ch.lane3=i.catalog_lane3
           CROSS JOIN candidate_catalogs c ON c.identity_hash_id=ch.hash_id
           LEFT JOIN hashes ph INDEXED BY hashes_by_prefix
             ON ph.algorithm_id=i.policy_algorithm AND ph.lane0=i.policy_lane0
             AND ph.lane1=i.policy_lane1 AND ph.lane2=i.policy_lane2 AND ph.lane3=i.policy_lane3
           ORDER BY i.ordinal"#,
    )?;
    let mut rows = statement.query(params![encoded, STRING_CHECK_KEY])?;
    let mut resolved = PublicationInternCache::default();
    while let Some(row) = rows.next()? {
        let ordinal: i64 = row.get(0)?;
        let ordinal = usize::try_from(ordinal).map_err(|_| {
            StoreError::Corrupt("publication identity ordinal is negative".to_owned())
        })?;
        let catalog_id: i64 = row.get(1)?;
        let Some((catalog, candidate)) = identities.get(ordinal) else {
            return Err(StoreError::Corrupt(
                "publication identity ordinal is out of range".to_owned(),
            ));
        };
        resolved.catalogs.insert(*catalog, catalog_id);
        if let (Some(candidate), Some(check_id)) = (candidate, row.get::<_, Option<i64>>(2)?) {
            resolved.checks.insert(
                (catalog_id, candidate.policy, candidate.check.clone()),
                check_id,
            );
        }
    }
    Ok(resolved)
}

#[derive(Clone, Default, Eq, PartialEq)]
pub(super) struct PublicationInternCache {
    pub(super) catalogs: BTreeMap<HashKey, i64>,
    pub(super) checks: BTreeMap<(i64, HashKey, String), i64>,
}

pub(super) fn stored_path_ids(
    connection: &Connection,
    paths: &[(&str, HashKey)],
) -> StoreResult<HashMap<String, i64>> {
    let input = paths
        .iter()
        .map(|(path, hash)| serde_json::json!([path, hash_json(*hash)]))
        .collect::<Vec<_>>();
    let encoded = encode_json(&input, "path lookup input")?;
    let mut statement = connection.prepare_cached(
        "WITH input AS (SELECT json_extract(value,'$[0]') path_value,\
         CAST(json_extract(value,'$[1][0]') AS INTEGER) algorithm,CAST(json_extract(value,'$[1][1]') AS INTEGER) lane0,\
         CAST(json_extract(value,'$[1][2]') AS INTEGER) lane1,CAST(json_extract(value,'$[1][3]') AS INTEGER) lane2,\
         CAST(json_extract(value,'$[1][4]') AS INTEGER) lane3 FROM json_each(?1)) \
         SELECT i.path_value,p.path_id FROM input i \
         CROSS JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
         AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3 \
         CROSS JOIN strings s ON s.domain_id=?2 AND s.content_hash_id=h.hash_id AND s.value=i.path_value \
         CROSS JOIN paths p ON p.string_id=s.string_id",
    )?;
    let rows = statement.query_map(params![encoded, STRING_PATH], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(StoreError::from)
}

pub(super) fn stored_object_identities(
    connection: &Connection,
    identities: &[(&OverlayFilePublication<'_>, HashKey, Option<HashKey>)],
) -> StoreResult<Vec<(Option<SourceId>, Option<TapeId>)>> {
    let input = identities
        .iter()
        .map(|(publication, source, tape_hash)| {
            serde_json::json!([
                source.algorithm,
                source.lanes[0],
                source.lanes[1],
                source.lanes[2],
                source.lanes[3],
                publication.tape.map(|tape| tape.grammar_id),
                publication.tape.map(|tape| tape.grammar_version),
                publication.tape.map(|tape| tape.tape_format_version),
                tape_hash.map(|hash| hash.algorithm),
                tape_hash.map(|hash| hash.lanes[0]),
                tape_hash.map(|hash| hash.lanes[1]),
                tape_hash.map(|hash| hash.lanes[2]),
                tape_hash.map(|hash| hash.lanes[3]),
            ])
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_string(&input).map_err(|error| {
        StoreError::Invalid(format!("object identities cannot be encoded: {error}"))
    })?;
    let mut statement = connection.prepare_cached(
        r#"WITH input AS (
             SELECT CAST(key AS INTEGER) ordinal,
               CAST(json_extract(value,'$[0]') AS INTEGER) source_algorithm,
               CAST(json_extract(value,'$[1]') AS INTEGER) source_lane0,
               CAST(json_extract(value,'$[2]') AS INTEGER) source_lane1,
               CAST(json_extract(value,'$[3]') AS INTEGER) source_lane2,
               CAST(json_extract(value,'$[4]') AS INTEGER) source_lane3,
               CAST(json_extract(value,'$[5]') AS INTEGER) grammar_id,
               CAST(json_extract(value,'$[6]') AS INTEGER) grammar_version,
               CAST(json_extract(value,'$[7]') AS INTEGER) tape_format_version,
               CAST(json_extract(value,'$[8]') AS INTEGER) tape_algorithm,
               CAST(json_extract(value,'$[9]') AS INTEGER) tape_lane0,
               CAST(json_extract(value,'$[10]') AS INTEGER) tape_lane1,
               CAST(json_extract(value,'$[11]') AS INTEGER) tape_lane2,
               CAST(json_extract(value,'$[12]') AS INTEGER) tape_lane3
             FROM json_each(?1)
           )
           SELECT i.ordinal,so.source_id,t.tape_id,
             stored.algorithm_id,stored.lane0,stored.lane1,stored.lane2,stored.lane3
           FROM input i
           LEFT JOIN hashes sh INDEXED BY hashes_by_prefix
             ON sh.algorithm_id=i.source_algorithm AND sh.lane0=i.source_lane0
             AND sh.lane1=i.source_lane1 AND sh.lane2=i.source_lane2 AND sh.lane3=i.source_lane3
           LEFT JOIN source_objects so ON so.canonical_hash_id=sh.hash_id
           LEFT JOIN tape_objects t ON t.source_id=so.source_id
             AND t.grammar_id=i.grammar_id AND t.grammar_version=i.grammar_version
             AND t.tape_format_version=i.tape_format_version
           LEFT JOIN hashes stored ON stored.hash_id=t.canonical_hash_id
           ORDER BY i.ordinal"#,
    )?;
    let mut rows = statement.query([encoded])?;
    let mut found = Vec::with_capacity(identities.len());
    while let Some(row) = rows.next()? {
        let ordinal: i64 = row.get(0)?;
        let ordinal = usize::try_from(ordinal)
            .map_err(|_| StoreError::Corrupt("object identity ordinal is negative".to_owned()))?;
        if ordinal != found.len() {
            return Err(StoreError::Corrupt(
                "object identity lookup returned a noncanonical order".to_owned(),
            ));
        }
        let source_id = row.get::<_, Option<i64>>(1)?.map(SourceId);
        let tape_id = row.get::<_, Option<i64>>(2)?.map(TapeId);
        if tape_id.is_some() {
            let stored_hash = HashKey {
                algorithm: row.get(3)?,
                lanes: [row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?],
            };
            let expected_hash = identities
                .get(ordinal)
                .and_then(|identity| identity.2)
                .ok_or_else(|| {
                    StoreError::Corrupt("stored tape has no requested identity".to_owned())
                })?;
            if stored_hash != expected_hash {
                return Err(StoreError::Corrupt(
                    "tape identity maps to different tape bytes".to_owned(),
                ));
            }
        }
        found.push((source_id, tape_id));
    }
    if found.len() != identities.len() {
        return Err(StoreError::Corrupt(
            "object identity lookup omitted input rows".to_owned(),
        ));
    }
    Ok(found)
}
