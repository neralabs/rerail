//! Overlay relationship rows written by publication, and their revalidation before commit.

use super::publication::{
    PublicationCandidateIdentity, PublicationInternCache, resolve_publication_identities,
    stored_object_identities, stored_path_ids,
};
use super::rows::hash_params;
use super::{
    CODEC_ZSTD, HashKey, OverlayFilePublication, SourceId, StoreError, StoreResult, TapeId,
    WorkspaceId, completion_candidate,
};
use rusqlite::Transaction;
use rusqlite::types::{ToSqlOutput, ValueRef};
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub(super) struct PreparedSqlStatement {
    sql: String,
    values: Vec<rusqlite::types::Value>,
}

pub(super) struct PreparedRelationshipPass {
    statements: Vec<PreparedSqlStatement>,
    /// The distinct contexts of `results` and `completions`, as the JSON array of their hash
    /// parameters.
    contexts: Option<String>,
    /// Each local result as its source, the index of its context in `contexts`, its check, its
    /// raw length, and its frame's offset and length in `frames`.
    results: Vec<(SourceId, usize, i64, i64, usize, usize)>,
    /// Every local result's frame, concatenated in `results` order: the one BLOB parameter of the
    /// result insert, which `substr` slices, so the insert's parameter count is independent of
    /// how many results a publication carries.
    frames: Vec<u8>,
    /// Each completion word as its source, the index of its context in `contexts`, its rule's
    /// completion check, its word and its bits.
    completions: Vec<(SourceId, usize, i64, u32, u64)>,
}

impl PreparedRelationshipPass {
    /// The data statements `execute_relationship_pass` runs: the overlay upsert, the context
    /// intern and lookup, one insert of every local result and one of every completion word: at
    /// most five, however many results the publication carries (the design's ten-statement bound
    /// per pass).
    pub(super) fn statement_count(&self) -> usize {
        self.statements.len()
            + usize::from(self.contexts.is_some())
            + usize::from(!self.results.is_empty())
            + usize::from(!self.completions.is_empty())
    }
}

/// The one statement that inserts a publication's local results. `?1` is the JSON array of
/// `[source, context hash id, check, raw length, frame offset, frame length]` rows; `?2` holds
/// every frame, concatenated, which `substr` slices (byte-indexed on a BLOB, from 1); `?3` is the
/// codec.
const INSERT_LOCAL_RESULTS: &str = "INSERT OR IGNORE INTO context_check_results(source_id,\
     context_hash_id,check_id,codec_id,raw_length,frame) \
     SELECT CAST(json_extract(value,'$[0]') AS INTEGER),CAST(json_extract(value,'$[1]') AS INTEGER),\
     CAST(json_extract(value,'$[2]') AS INTEGER),?3,CAST(json_extract(value,'$[3]') AS INTEGER),\
     substr(?2,CAST(json_extract(value,'$[4]') AS INTEGER)+1,CAST(json_extract(value,'$[5]') AS INTEGER)) \
     FROM json_each(?1)";

/// The one statement that stores a publication's completion words, adding each word's bits to
/// the stored word's: both say which clauses were evaluated and found nothing. `?1` is the JSON
/// array of `[source, context hash id, check, word, bits]` rows, the bits as a signed integer.
const INSERT_LOCAL_COMPLETIONS: &str = "INSERT INTO context_check_completions(source_id,\
     context_hash_id,check_id,word,evaluated) \
     SELECT CAST(json_extract(value,'$[0]') AS INTEGER),CAST(json_extract(value,'$[1]') AS INTEGER),\
     CAST(json_extract(value,'$[2]') AS INTEGER),CAST(json_extract(value,'$[3]') AS INTEGER),\
     CAST(json_extract(value,'$[4]') AS INTEGER) FROM json_each(?1) WHERE true \
     ON CONFLICT(source_id,context_hash_id,check_id,word) \
     DO UPDATE SET evaluated=evaluated|excluded.evaluated";

pub(super) fn prepare_relationship_pass<'a>(
    workspace_id: WorkspaceId,
    rows: &[(
        i64,
        i64,
        SourceId,
        Option<TapeId>,
        OverlayFilePublication<'a>,
    )],
    interned: &PublicationInternCache,
    length_limit: usize,
) -> StoreResult<PreparedRelationshipPass> {
    let mut statements = Vec::new();
    let mut overlay_sql = String::from(
        "INSERT INTO workspace_overlay(workspace_id,path_id,state_id,source_id,tape_id) VALUES ",
    );
    let mut overlay_values = Vec::with_capacity(rows.len() * 5);
    for (index, (path_id, state_id, source_id, tape_id, _)) in rows.iter().enumerate() {
        if index != 0 {
            overlay_sql.push(',');
        }
        overlay_sql.push_str("(?,?,?,?,?)");
        overlay_values.push(workspace_id.0.into());
        overlay_values.push((*path_id).into());
        overlay_values.push((*state_id).into());
        overlay_values.push(source_id.0.into());
        overlay_values.push(tape_id.map_or(rusqlite::types::Value::Null, |id| id.0.into()));
    }
    overlay_sql.push_str(" ON CONFLICT(workspace_id,path_id) DO UPDATE SET state_id=excluded.state_id,source_id=excluded.source_id,tape_id=excluded.tape_id");
    statements.push(PreparedSqlStatement {
        sql: overlay_sql,
        values: overlay_values,
    });

    let mut local_results = Vec::<(SourceId, HashKey, i64, i64, &[u8])>::new();
    let mut local_completions = Vec::<(SourceId, HashKey, i64, u32, u64)>::new();
    for (_, _, source_id, _, publication) in rows {
        let catalog_key = publication.catalog.key();
        let catalog_id = *interned
            .catalogs
            .get(&catalog_key)
            .ok_or(StoreError::Vanished("candidate catalog"))?;
        for result in publication.local_results {
            let check_id = *interned
                .clauses
                .get(&(
                    catalog_id,
                    result.candidate.rule,
                    result.candidate.clause.clone(),
                ))
                .ok_or(StoreError::Vanished("candidate check"))?;
            let raw_length = i64::try_from(result.raw_length).map_err(|_| {
                StoreError::Invalid("local result length exceeds SQLite range".to_owned())
            })?;
            local_results.push((
                *source_id,
                result.context,
                check_id,
                raw_length,
                result.frame.as_slice(),
            ));
        }
        for completion in publication.local_completions {
            let candidate = completion_candidate(completion.rule);
            let check_id = *interned
                .clauses
                .get(&(catalog_id, candidate.rule, candidate.clause))
                .ok_or(StoreError::Vanished("candidate check"))?;
            local_completions.push((
                *source_id,
                completion.context,
                check_id,
                completion.word,
                completion.evaluated,
            ));
        }
    }

    // A result's context is interned by the object pass, which a publication of stored objects
    // skips; a context new to the store (a `calls` stamp, a changed module) is interned here.
    // The contexts' ids are then read once, so each result row names its context by id instead
    // of joining its hash.
    let contexts = local_results
        .iter()
        .map(|(_, context, ..)| *context)
        .chain(local_completions.iter().map(|(_, context, ..)| *context))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let contexts_json = if contexts.is_empty() {
        None
    } else {
        let contexts = serde_json::to_string(
            &contexts
                .iter()
                .map(|context| hash_params(*context))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::Invalid(error.to_string()))?;
        statements.push(PreparedSqlStatement {
            sql: "WITH input AS (SELECT CAST(json_extract(value,'$[0]') AS INTEGER) algorithm,\
                  CAST(json_extract(value,'$[1]') AS INTEGER) lane0,CAST(json_extract(value,'$[2]') AS INTEGER) lane1,\
                  CAST(json_extract(value,'$[3]') AS INTEGER) lane2,CAST(json_extract(value,'$[4]') AS INTEGER) lane3 FROM json_each(?1)) \
                  INSERT INTO hashes(algorithm_id,lane0,lane1,lane2,lane3) \
                  SELECT i.algorithm,i.lane0,i.lane1,i.lane2,i.lane3 FROM input i \
                  WHERE NOT EXISTS (SELECT 1 FROM hashes h INDEXED BY hashes_by_prefix \
                  WHERE h.algorithm_id=i.algorithm AND h.lane0=i.lane0 AND h.lane1=i.lane1 \
                  AND h.lane2=i.lane2 AND h.lane3=i.lane3)"
                .to_owned(),
            values: vec![rusqlite::types::Value::Text(contexts.clone())],
        });
        Some(contexts)
    };
    let mut frames = Vec::with_capacity(
        local_results
            .iter()
            .map(|(.., frame)| frame.len())
            .sum::<usize>(),
    );
    let results = local_results
        .into_iter()
        .map(|(source_id, context, check_id, raw_length, frame)| {
            let context = contexts
                .binary_search(&context)
                .expect("every result context is collected");
            let offset = frames.len();
            frames.extend_from_slice(frame);
            (
                source_id,
                context,
                check_id,
                raw_length,
                offset,
                frame.len(),
            )
        })
        .collect::<Vec<_>>();
    // The frames travel as one BLOB, so they must fit one SQLite value.
    if frames.len() > length_limit {
        return Err(StoreError::Invalid(format!(
            "local results of one publication hold {} frame bytes; SQLite's value limit is {length_limit}",
            frames.len()
        )));
    }
    let completions = local_completions
        .into_iter()
        .map(|(source_id, context, check_id, word, evaluated)| {
            let context = contexts
                .binary_search(&context)
                .expect("every completion context is collected");
            (source_id, context, check_id, word, evaluated)
        })
        .collect();
    Ok(PreparedRelationshipPass {
        statements,
        contexts: contexts_json,
        results,
        frames,
        completions,
    })
}

pub(super) fn relationship_references_current(
    transaction: &Transaction<'_>,
    rows: &[(
        i64,
        i64,
        SourceId,
        Option<TapeId>,
        OverlayFilePublication<'_>,
    )],
    path_hashes: &[(&str, HashKey)],
    hashes: &[(HashKey, Option<HashKey>)],
    candidate_identities: &[PublicationCandidateIdentity],
    interned: &PublicationInternCache,
) -> StoreResult<bool> {
    if hashes.len() != rows.len() || path_hashes.len() != rows.len() {
        return Err(StoreError::Corrupt(
            "relationship hash input omitted a publication".to_owned(),
        ));
    }
    let paths = stored_path_ids(transaction, path_hashes)?;
    if rows
        .iter()
        .any(|(path_id, _, _, _, publication)| paths.get(publication.path) != Some(path_id))
    {
        return Ok(false);
    }
    let identities = rows
        .iter()
        .zip(hashes)
        .map(|((_, _, _, _, publication), (source_hash, tape_hash))| {
            (publication, *source_hash, *tape_hash)
        })
        .collect::<Vec<_>>();
    let stored = stored_object_identities(transaction, &identities)?;
    if rows.iter().zip(stored).any(
        |((_, _, source_id, tape_id, _), (stored_source, stored_tape))| {
            stored_source != Some(*source_id) || stored_tape != *tape_id
        },
    ) {
        return Ok(false);
    }
    Ok(resolve_publication_identities(transaction, candidate_identities)? == *interned)
}

pub(super) fn execute_relationship_pass(
    transaction: &Transaction<'_>,
    pass: &PreparedRelationshipPass,
) -> StoreResult<()> {
    for statement in &pass.statements {
        transaction
            .prepare_cached(&statement.sql)?
            .execute(rusqlite::params_from_iter(statement.values.iter()))?;
    }
    let Some(contexts) = &pass.contexts else {
        return Ok(());
    };
    let mut context_ids = vec![
        None;
        pass.results
            .iter()
            .map(|row| row.1 + 1)
            .chain(pass.completions.iter().map(|row| row.1 + 1))
            .max()
            .unwrap_or(0)
    ];
    {
        let mut statement = transaction.prepare_cached(
            "SELECT CAST(i.key AS INTEGER),h.hash_id FROM json_each(?1) i \
             JOIN hashes h INDEXED BY hashes_by_prefix \
             ON h.algorithm_id=CAST(json_extract(i.value,'$[0]') AS INTEGER) \
             AND h.lane0=CAST(json_extract(i.value,'$[1]') AS INTEGER) \
             AND h.lane1=CAST(json_extract(i.value,'$[2]') AS INTEGER) \
             AND h.lane2=CAST(json_extract(i.value,'$[3]') AS INTEGER) \
             AND h.lane3=CAST(json_extract(i.value,'$[4]') AS INTEGER)",
        )?;
        let mut rows = statement.query([contexts])?;
        while let Some(row) = rows.next()? {
            let index = usize::try_from(row.get::<_, i64>(0)?)
                .map_err(|_| StoreError::Corrupt("context index is negative".to_owned()))?;
            if let Some(id) = context_ids.get_mut(index) {
                *id = Some(row.get::<_, i64>(1)?);
            }
        }
    }
    let context_id = |context: usize| {
        context_ids[context].ok_or_else(|| {
            StoreError::Corrupt("a local result context was not interned".to_owned())
        })
    };
    if !pass.results.is_empty() {
        let mut rows = String::with_capacity(2 + pass.results.len() * 48);
        rows.push('[');
        for (index, (source_id, context, check_id, raw_length, offset, length)) in
            pass.results.iter().enumerate()
        {
            let context_id = context_id(*context)?;
            if index != 0 {
                rows.push(',');
            }
            write!(
                rows,
                "[{},{context_id},{check_id},{raw_length},{offset},{length}]",
                source_id.0
            )
            .expect("writing to a String cannot fail");
        }
        rows.push(']');
        transaction.prepare_cached(INSERT_LOCAL_RESULTS)?.execute((
            rows,
            ToSqlOutput::Borrowed(ValueRef::Blob(&pass.frames)),
            CODEC_ZSTD,
        ))?;
    }
    if !pass.completions.is_empty() {
        let mut rows = String::with_capacity(2 + pass.completions.len() * 48);
        rows.push('[');
        for (index, (source_id, context, check_id, word, evaluated)) in
            pass.completions.iter().enumerate()
        {
            let context_id = context_id(*context)?;
            if index != 0 {
                rows.push(',');
            }
            // SQLite integers are signed: the word's 64 bits travel as their two's complement.
            write!(
                rows,
                "[{},{context_id},{check_id},{word},{}]",
                source_id.0, *evaluated as i64
            )
            .expect("writing to a String cannot fail");
        }
        rows.push(']');
        transaction
            .prepare_cached(INSERT_LOCAL_COMPLETIONS)?
            .execute([rows])?;
    }
    Ok(())
}
