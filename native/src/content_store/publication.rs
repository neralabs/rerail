//! Bulk overlay publication: object preparation, identity resolution, and the publish transaction.

use super::connection::retry_busy;
use super::object_segments::{CODEC_RAW, Placing};
use super::objects::validate_tape_payload;
use super::segments::Place;
use super::relationships::{
    execute_relationship_pass, prepare_relationship_pass, relationship_references_current,
};
use super::rows::{hash_params, validate_relative_path};
use super::{
    BusyRetries, ContentStore, HashKey, MAX_PUBLICATION_BATCH, OverlayFilePublication,
    PublicationResult, STRING_PATH, SourceId, StoreError, StoreResult, StoredFile, TapeId,
    TapePayload, WorkspaceId,
};
use rayon::prelude::*;
use rusqlite::{Connection, TransactionBehavior, params};
#[cfg(any(test, feature = "test-hooks"))]
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

#[cfg(any(test, feature = "test-hooks"))]
type PublicationTestHook = RefCell<Option<Box<dyn FnOnce()>>>;

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Run on the publishing thread, so a hook never fires in another test's publication.
    pub(super) static AFTER_PUBLICATION_READ_HOOK: PublicationTestHook =
        const { RefCell::new(None) };
    pub(super) static BEFORE_RELATIONSHIP_WRITE_HOOK: PublicationTestHook =
        const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-hooks"))]
fn run_publication_test_hook(hook: &'static std::thread::LocalKey<PublicationTestHook>) {
    if let Some(hook) = hook.take() {
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
        self.publish_overlay_files_reporting_busy(
            workspace_id,
            publications,
            cancelled,
            &mut BusyRetries::default(),
        )
    }

    /// `publish_overlay_files_cancellable`, adding the publication's busy retries to `busy`
    /// whatever it returns: a publication that ends `Busy` has no result to carry them.
    pub fn publish_overlay_files_reporting_busy(
        &self,
        workspace_id: WorkspaceId,
        publications: &[OverlayFilePublication<'_>],
        cancelled: &dyn Fn() -> bool,
        busy: &mut BusyRetries,
    ) -> StoreResult<PublicationResult> {
        self.publish_overlay_files_cancellable_attempt(
            workspace_id,
            publications,
            cancelled,
            busy,
            0,
        )
    }

    fn publish_overlay_files_cancellable_attempt(
        &self,
        workspace_id: WorkspaceId,
        publications: &[OverlayFilePublication<'_>],
        cancelled: &dyn Fn() -> bool,
        busy: &mut BusyRetries,
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
        let identities = rerail_prof::code_rule_dsl::logic::profile::on_cores(|| {
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
                    let source = HashKey::sha256(publication.source);
                    Ok((
                        publication,
                        source,
                        publication
                            .tape
                            .map(|tape| HashKey::tape_identity(source, tape.bytes)),
                    ))
                })
                .collect::<StoreResult<Vec<_>>>()
        })?;
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
        let (existing, initial_path_ids) = {
            let mut connection = self
                .connection
                .lock()
                .expect("content store connection unavailable");
            let transaction =
                super::connection::begin(&mut connection, TransactionBehavior::Deferred)?;
            let existing = stored_object_identities(&transaction, &identities)?;
            let path_ids = stored_path_ids(&transaction, &path_hashes)?;
            self.commit(transaction)?;
            (existing, path_ids)
        };
        let initial_objects = existing.clone();
        // Each source and tape not stored yet is placed, as it is, in the pass's segment (`place_missing_objects`).
        let mut prepared: Vec<PreparedOverlayPublication<'_>> = identities
            .into_iter()
            .zip(existing)
            .map(|((publication, source_hash, tape_hash), (source_id, tape_id))| {
                (publication, source_hash, source_id, None, tape_hash, tape_id, None, None)
            })
            .collect();
        let reused_sources = prepared.iter().filter(|item| item.2.is_some()).count();
        let reused_tapes = prepared.iter().filter(|item| item.5.is_some()).count();
        let written_sources = prepared.len() - reused_sources;
        let written_tapes = prepared
            .iter()
            .filter(|item| item.0.tape.is_some() && item.5.is_none())
            .count();
        let expected_paths = publications
            .iter()
            .map(|publication| publication.path)
            .collect::<BTreeSet<_>>()
            .len();
        let mut transaction_nanoseconds = 0_u64;
        let mut object_transaction_nanoseconds = 0_u64;
        let mut relationship_transaction_nanoseconds = 0_u64;
        let mut read_data_statements = 2_usize;
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
            && publications
                .iter()
                .all(|publication| publication.aliases.is_empty());
        let (stored_objects, path_ids) = if initial_complete {
            (initial_objects, initial_path_ids)
        } else {
            #[cfg(any(test, feature = "test-hooks"))]
            run_publication_test_hook(&AFTER_PUBLICATION_READ_HOOK);
            let mut current_objects = initial_objects;
            let mut current_path_ids = initial_path_ids;
            loop {
                if cancelled() {
                    return Err(StoreError::Invalid(
                        "overlay publication was cancelled".to_owned(),
                    ));
                }
                let mut placing = place_missing_objects(
                    &mut prepared,
                    &current_objects,
                    &self.database_path,
                    self.connection.flush(),
                )?;
                let object_pass = prepare_object_pass(
                    &prepared,
                    &current_objects,
                    &current_path_ids,
                )?;
                // The loop above polls cancellation before the first attempt; a retry polls again.
                let mut retry = false;
                let (elapsed, statement_count, seg) = retry_busy(busy, || {
                    if std::mem::replace(&mut retry, true) && cancelled() {
                        return Err(StoreError::Invalid(
                            "overlay publication was cancelled".to_owned(),
                        ));
                    }
                    let mut connection = self
                        .connection
                        .lock()
                        .expect("content store connection unavailable");
                    let transaction =
                        super::connection::begin(&mut connection, TransactionBehavior::Immediate)?;
                    #[cfg(test)]
                    let _write_lock = super::WriteLockSection::enter();
                    let started = Instant::now();
                    // The pass's segment, numbered here: its rows name it as `seg` 0 (`prepare_object_pass`).
                    let seg = placing.number(&transaction)?;
                    let statement_count = execute_object_pass(&transaction, &object_pass)?;
                    if statement_count > 10 {
                        return Err(StoreError::Invalid(format!(
                            "object publication used {} statements; maximum is 10",
                            statement_count
                        )));
                    }
                    self.commit(transaction)?;
                    Ok((
                        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                        statement_count,
                        seg,
                    ))
                })?;
                placing.name(seg)?;
                if let Some(seg) = seg {
                    numbered(&mut prepared, seg);
                }
                transaction_nanoseconds = transaction_nanoseconds.saturating_add(elapsed);
                object_transaction_nanoseconds =
                    object_transaction_nanoseconds.saturating_add(elapsed);
                data_statements += statement_count;
                max_data_statements_per_pass = max_data_statements_per_pass.max(statement_count);
                transaction_statements += 2;
                reconciliation_passes += 1;

                let corrected_identities = prepared
                    .iter()
                    .map(|item| (item.0, item.1, item.4))
                    .collect::<Vec<_>>();
                let (stored_objects, path_ids) = {
                    let mut connection = self
                        .connection
                        .lock()
                        .expect("content store connection unavailable");
                    let transaction =
                        super::connection::begin(&mut connection, TransactionBehavior::Deferred)?;
                    let stored_objects =
                        stored_object_identities(&transaction, &corrected_identities)?;
                    let path_ids = stored_path_ids(&transaction, &path_hashes)?;
                    self.commit(transaction)?;
                    (stored_objects, path_ids)
                };
                read_data_statements += 2;
                transaction_statements += 2;
                let complete = stored_objects
                    .iter()
                    .enumerate()
                    .all(|(index, (source, tape))| {
                        source.is_some() && (prepared[index].0.tape.is_none() || tape.is_some())
                    })
                    && path_ids.len() == expected_paths;
                if complete {
                    break (stored_objects, path_ids);
                }
                if reconciliation_passes >= 8 {
                    return Err(StoreError::Invalid(
                        "overlay publication made no progress after 8 reconciliation passes"
                            .to_owned(),
                    ));
                }
                current_objects = stored_objects;
                current_path_ids = path_ids;
            }
        };
        let relationship_hashes = prepared
            .iter()
            .map(|item| (item.1, item.4))
            .collect::<Vec<_>>();
        let mut stored = Vec::with_capacity(prepared.len());
        let mut overlay_rows = Vec::with_capacity(prepared.len());
        for ((publication, _, _, _, _, _, _, _), (source_id, tape_id)) in
            prepared.iter().zip(stored_objects)
        {
            let source_id = source_id.ok_or(StoreError::Vanished("source"))?;
            if publication.tape.is_some() && tape_id.is_none() {
                return Err(StoreError::Vanished("tape"));
            }
            let path_id = *path_ids
                .get(publication.path)
                .ok_or(StoreError::Vanished("path"))?;
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

        let relationship_pass = prepare_relationship_pass(workspace_id, &overlay_rows)?;
        #[cfg(any(test, feature = "test-hooks"))]
        run_publication_test_hook(&BEFORE_RELATIONSHIP_WRITE_HOOK);
        let (elapsed, statement_count, current) = retry_busy(busy, || {
            if cancelled() {
                return Err(StoreError::Invalid(
                    "overlay publication was cancelled".to_owned(),
                ));
            }
            let mut connection = self
                .connection
                .lock()
                .expect("content store connection unavailable");
            let transaction =
                super::connection::begin(&mut connection, TransactionBehavior::Immediate)?;
            #[cfg(test)]
            let _write_lock = super::WriteLockSection::enter();
            let transaction_started = Instant::now();
            if !relationship_references_current(
                &transaction,
                &overlay_rows,
                &path_hashes,
                &relationship_hashes,
            )? {
                return Ok((
                    u64::try_from(transaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                    0,
                    false,
                ));
            }
            execute_relationship_pass(&transaction, &relationship_pass)?;
            let statement_count = relationship_pass.statement_count();
            if cancelled() {
                return Err(StoreError::Invalid(
                    "overlay publication was cancelled".to_owned(),
                ));
            }
            self.commit(transaction)?;
            Ok((
                u64::try_from(transaction_started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                statement_count,
                true,
            ))
        })?;
        if !current {
            if stale_relationship_retries >= 8 {
                return Err(StoreError::Invalid(
                    "overlay publication made no progress after 8 reconciliation passes".to_owned(),
                ));
            }
            return self.publish_overlay_files_cancellable_attempt(
                workspace_id,
                publications,
                cancelled,
                busy,
                stale_relationship_retries + 1,
            );
        }
        transaction_nanoseconds = transaction_nanoseconds.saturating_add(elapsed);
        relationship_transaction_nanoseconds =
            relationship_transaction_nanoseconds.saturating_add(elapsed);
        data_statements += statement_count;
        max_data_statements_per_pass = max_data_statements_per_pass.max(statement_count);
        transaction_statements += 2;
        reconciliation_passes += 1;
        Ok(PublicationResult {
            files: stored,
            retries: busy.retries,
            retry_wait_nanoseconds: busy.wait_nanoseconds,
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
            written_sources,
            written_tapes,
        })
    }
}

type PreparedOverlayPublication<'a> = (
    &'a OverlayFilePublication<'a>,
    HashKey,
    Option<SourceId>,
    // Where the source is placed, the tape, and the sources and tapes of the documents derived from a tape not stored
    // yet: `seg` 0 is the pass's segment, numbered in its transaction.
    Option<Place>,
    Option<HashKey>,
    Option<TapeId>,
    Option<Place>,
    Option<Vec<(Place, Place)>>,
);

struct PreparedObjectPass {
    hashes_json: String,
    strings_json: String,
    paths_json: String,
    aliases_json: String,
    source_sql: String,
    source_values: Vec<rusqlite::types::Value>,
    tape_sql: String,
    tape_values: Vec<rusqlite::types::Value>,
    /// The `file_documents` rows of the new tapes that derive documents; `None` where none does.
    document_sql: Option<(String, Vec<rusqlite::types::Value>)>,
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

/// The sources and tapes `prepared` needs stored that `stored_objects` lacks and no earlier pass placed: written, as
/// they are, in one segment (the pass's), each given its place there (`seg` 0 until the pass's transaction numbers
/// it, `numbered`).
fn place_missing_objects(
    prepared: &mut [PreparedOverlayPublication<'_>],
    stored_objects: &[(Option<SourceId>, Option<TapeId>)],
    database: &std::path::Path,
    flush: rerail_blocks::block::Flush,
) -> StoreResult<Placing> {
    // (file, which: 0 source, 1 tape, 2 + document), in the order written.
    let mut bytes: Vec<&[u8]> = Vec::new();
    let mut slots: Vec<(usize, usize)> = Vec::new();
    for (index, ((publication, _, _, source, _, _, tape, documents), (source_id, tape_id))) in prepared.iter().zip(stored_objects).enumerate() {
        if source_id.is_none() && source.is_none() {
            bytes.push(publication.source);
            slots.push((index, 0));
        }
        if let Some(payload) = publication.tape
            && tape_id.is_none()
        {
            if tape.is_none() {
                bytes.push(payload.bytes);
                slots.push((index, 1));
            }
            if documents.is_none() {
                for document in payload.documents {
                    bytes.push(document.source);
                    bytes.push(document.tape);
                }
                slots.push((index, 2));
            }
        }
    }
    let placing = Placing::write(database, &bytes, flush)?;
    let mut at = 0;
    for (index, which) in slots {
        let item = &mut prepared[index];
        match which {
            0 => item.3 = Some(placing.place(0, at)),
            1 => item.6 = Some(placing.place(0, at)),
            _ => {
                let n = item.0.tape.map_or(0, |payload| payload.documents.len());
                item.7 = Some((0..n).map(|d| (placing.place(0, at + 2 * d), placing.place(0, at + 2 * d + 1))).collect());
                at += 2 * n;
                continue;
            }
        }
        at += 1;
    }
    Ok(placing)
}

/// Once a pass's transaction committed, numbering its segment `seg`: the places it gave (`seg` 0) are in that segment.
fn numbered(prepared: &mut [PreparedOverlayPublication<'_>], seg: i64) {
    let fix = |place: &mut Place| {
        if place.seg == 0 {
            place.seg = seg;
        }
    };
    for item in prepared {
        item.3.iter_mut().for_each(fix);
        item.6.iter_mut().for_each(fix);
        for (source, tape) in item.7.iter_mut().flatten() {
            fix(source);
            fix(tape);
        }
    }
}

/// A place's `seg`, `seg_at` and `seg_len`, as a pass's rows bind them.
fn place_values(place: Place) -> [rusqlite::types::Value; 3] {
    [place.seg.into(), (place.at as i64).into(), (place.len as i64).into()]
}

fn prepare_object_pass(
    prepared: &[PreparedOverlayPublication<'_>],
    stored_objects: &[(Option<SourceId>, Option<TapeId>)],
    known_paths: &HashMap<String, i64>,
) -> StoreResult<PreparedObjectPass> {
    let mut hashes = BTreeSet::new();
    let mut strings = BTreeMap::<(i64, String), HashKey>::new();
    let mut paths = BTreeMap::<String, HashKey>::new();
    let mut aliases = BTreeSet::<(HashKey, HashKey)>::new();
    let mut source_rows = Vec::new();
    let mut tape_rows = Vec::new();
    let mut document_rows = Vec::new();

    for (
        (publication, source_hash, _, source_frame, tape_hash, _, tape_frame, document_frames),
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
        for alias in publication.aliases {
            hashes.insert(*alias);
            aliases.insert((*alias, *source_hash));
        }
        if source_id.is_none() {
            let place = source_frame.ok_or(StoreError::Vanished("source"))?;
            source_rows.push((*source_hash, publication.source.len(), place));
        }
        if let (Some(payload), Some(hash)) = (publication.tape, *tape_hash)
            && tape_id.is_none()
        {
            let place = tape_frame.ok_or(StoreError::Vanished("tape"))?;
            hashes.insert(hash);
            tape_rows.push((*source_hash, hash, payload, place));
            // The tape's derived documents: each an ordinary source and tape, and its row.
            if !payload.documents.is_empty() {
                let frames = document_frames
                    .as_ref()
                    .ok_or(StoreError::Vanished("derived document"))?;
                for (index, (document, &(document_source_place, document_tape_place))) in
                    payload.documents.iter().zip(frames).enumerate()
                {
                    let document_source = HashKey::sha256(document.source);
                    let document_tape = HashKey::tape_identity(document_source, document.tape);
                    hashes.insert(document_source);
                    hashes.insert(document_tape);
                    source_rows.push((
                        document_source,
                        document.source.len(),
                        document_source_place,
                    ));
                    tape_rows.push((
                        document_source,
                        document_tape,
                        TapePayload {
                            grammar_id: document.grammar_id,
                            grammar_version: payload.grammar_version,
                            tape_format_version: payload.tape_format_version,
                            bytes: document.tape,
                            documents: &[],
                        },
                        document_tape_place,
                    ));
                    document_rows.push((
                        *source_hash,
                        payload,
                        index + 1,
                        document,
                        document_source,
                    ));
                }
            }
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
    let aliases_json = encode_json(
        &aliases
            .into_iter()
            .map(|(alias, source)| serde_json::json!([hash_json(alias), hash_json(source)]))
            .collect::<Vec<_>>(),
        "source aliases",
    )?;

    // A place's `seg` 0 is the pass's segment, the newest the transaction registered (`Placing::number`).
    let mut source_sql =
        String::from("WITH input(algorithm,lane0,lane1,lane2,lane3,raw_length,seg,seg_at,seg_len) AS (VALUES ");
    let mut source_values = Vec::with_capacity(source_rows.len() * 9);
    if source_rows.is_empty() {
        source_sql.push_str("(0,0,0,0,0,0,0,0,0)) SELECT 0,0,0,0,NULL,0,0,0,0 WHERE 0");
    } else {
        for (index, (hash, raw_length, place)) in source_rows.into_iter().enumerate() {
            if index != 0 {
                source_sql.push(',');
            }
            source_sql.push_str("(?,?,?,?,?,?,?,?,?)");
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
            source_values.extend(place_values(place));
        }
        source_sql.push_str(
            ") INSERT OR IGNORE INTO source_objects(canonical_hash_id,raw_length,codec_id,frame,created_generation,seg,seg_at,seg_len) \
             SELECT h.hash_id,i.raw_length,? ,x'',(SELECT generation FROM store_state WHERE singleton=1),\
             CASE WHEN i.seg=0 THEN (SELECT max(seg) FROM object_segments) ELSE i.seg END,i.seg_at,i.seg_len \
             FROM input i JOIN hashes h INDEXED BY hashes_by_prefix ON h.algorithm_id=i.algorithm AND h.lane0=i.lane0 \
             AND h.lane1=i.lane1 AND h.lane2=i.lane2 AND h.lane3=i.lane3",
        );
        source_values.push(CODEC_RAW.into());
    }

    let mut tape_sql = String::from(
        "WITH input(source_algorithm,source_lane0,source_lane1,source_lane2,source_lane3,tape_algorithm,tape_lane0,tape_lane1,tape_lane2,tape_lane3,grammar_id,grammar_version,format_version,raw_length,seg,seg_at,seg_len) AS (VALUES ",
    );
    let mut tape_values = Vec::with_capacity(tape_rows.len() * 17);
    if tape_rows.is_empty() {
        tape_sql.push_str("(0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0)) SELECT 0,0,0,0,0,0,0,0,0,0,0,0 WHERE 0");
    } else {
        for (index, (source, hash, payload, place)) in tape_rows.into_iter().enumerate() {
            if index != 0 {
                tape_sql.push(',');
            }
            tape_sql.push_str("(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)");
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
            tape_values.extend(place_values(place));
        }
        tape_sql.push_str(
            ") INSERT OR IGNORE INTO tape_objects(source_id,canonical_hash_id,grammar_id,grammar_version,tape_format_version,codec_id,raw_length,frame,created_generation,seg,seg_at,seg_len) \
             SELECT so.source_id,th.hash_id,i.grammar_id,i.grammar_version,i.format_version,\
             0,i.raw_length,x'',(SELECT generation FROM store_state WHERE singleton=1),\
             CASE WHEN i.seg=0 THEN (SELECT max(seg) FROM object_segments) ELSE i.seg END,i.seg_at,i.seg_len FROM input i \
             JOIN hashes sh INDEXED BY hashes_by_prefix ON sh.algorithm_id=i.source_algorithm AND sh.lane0=i.source_lane0 \
             AND sh.lane1=i.source_lane1 AND sh.lane2=i.source_lane2 AND sh.lane3=i.source_lane3 \
             JOIN source_objects so ON so.canonical_hash_id=sh.hash_id \
             JOIN hashes th INDEXED BY hashes_by_prefix ON th.algorithm_id=i.tape_algorithm AND th.lane0=i.tape_lane0 \
             AND th.lane1=i.tape_lane1 AND th.lane2=i.tape_lane2 AND th.lane3=i.tape_lane3",
        );
    }

    // Every row joins its primary tape and its own tape, both written above in this transaction
    // (or already stored), by identity.
    let document_sql = (!document_rows.is_empty())
        .then(|| {
            let mut sql = String::from(
                "WITH input(source_algorithm,source_lane0,source_lane1,source_lane2,source_lane3,grammar_id,\
                 grammar_version,format_version,ordinal,parent_ordinal,role_id,via_id,document_algorithm,\
                 document_lane0,document_lane1,document_lane2,document_lane3,document_grammar_id,source_map,\
                 host_errors,at_start,at_end) AS (VALUES ",
            );
            let mut values = Vec::with_capacity(document_rows.len() * 22);
            for (index, (source, payload, ordinal, document, document_source)) in
                document_rows.into_iter().enumerate()
            {
                if index != 0 {
                    sql.push(',');
                }
                sql.push_str("(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)");
                values.extend(
                    hash_params(source)
                        .into_iter()
                        .map(rusqlite::types::Value::Integer),
                );
                values.push(payload.grammar_id.into());
                values.push(payload.grammar_version.into());
                values.push(payload.tape_format_version.into());
                values.push(
                    i64::try_from(ordinal)
                        .map_err(|_| {
                            StoreError::Invalid("too many derived documents".to_owned())
                        })?
                        .into(),
                );
                values.push(document.parent_ordinal.into());
                values.push(document.role_id.into());
                values.push(document.via_id.into());
                values.extend(
                    hash_params(document_source)
                        .into_iter()
                        .map(rusqlite::types::Value::Integer),
                );
                values.push(document.grammar_id.into());
                values.push(document.source_map.to_vec().into());
                values.push(document.host_errors.to_vec().into());
                values.push(document.at.map_or(rusqlite::types::Value::Null, |(start, _)| i64::from(start).into()));
                values.push(document.at.map_or(rusqlite::types::Value::Null, |(_, end)| i64::from(end).into()));
            }
            sql.push_str(
                ") INSERT OR IGNORE INTO file_documents(tape_id,ordinal,parent_ordinal,role_id,via_id,document_tape_id,source_map,host_errors,at_start,at_end) \
                 SELECT pt.tape_id,i.ordinal,i.parent_ordinal,i.role_id,i.via_id,dt.tape_id,i.source_map,i.host_errors,i.at_start,i.at_end FROM input i \
                 JOIN hashes ph INDEXED BY hashes_by_prefix ON ph.algorithm_id=i.source_algorithm AND ph.lane0=i.source_lane0 \
                 AND ph.lane1=i.source_lane1 AND ph.lane2=i.source_lane2 AND ph.lane3=i.source_lane3 \
                 JOIN source_objects ps ON ps.canonical_hash_id=ph.hash_id \
                 JOIN tape_objects pt ON pt.source_id=ps.source_id AND pt.grammar_id=i.grammar_id \
                 AND pt.grammar_version=i.grammar_version AND pt.tape_format_version=i.format_version \
                 JOIN hashes dh INDEXED BY hashes_by_prefix ON dh.algorithm_id=i.document_algorithm AND dh.lane0=i.document_lane0 \
                 AND dh.lane1=i.document_lane1 AND dh.lane2=i.document_lane2 AND dh.lane3=i.document_lane3 \
                 JOIN source_objects ds ON ds.canonical_hash_id=dh.hash_id \
                 JOIN tape_objects dt ON dt.source_id=ds.source_id AND dt.grammar_id=i.document_grammar_id \
                 AND dt.grammar_version=i.grammar_version AND dt.tape_format_version=i.format_version",
            );
            Ok::<_, StoreError>((sql, values))
        })
        .transpose()?;

    Ok(PreparedObjectPass {
        hashes_json,
        strings_json,
        paths_json,
        aliases_json,
        source_sql,
        source_values,
        tape_sql,
        tape_values,
        document_sql,
    })
}

fn execute_object_pass(transaction: &Connection, pass: &PreparedObjectPass) -> StoreResult<usize> {
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
    if let Some((sql, values)) = &pass.document_sql {
        transaction
            .prepare_cached(sql)?
            .execute(rusqlite::params_from_iter(values.iter()))?;
        statements += 1;
    }
    Ok(statements)
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
