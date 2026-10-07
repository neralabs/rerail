//! Overlay relationship rows written by publication, and their revalidation before commit.

use super::publication::{
    PublicationCandidateIdentity, PublicationInternCache, resolve_publication_identities,
    stored_object_identities, stored_path_ids,
};
use super::rows::hash_params;
use super::{
    CODEC_ZSTD, HashKey, OverlayFilePublication, SourceId, StoreError, StoreResult, TapeId,
    WorkspaceId,
};
use rusqlite::Transaction;
use std::collections::BTreeSet;

pub(super) struct PreparedSqlStatement {
    sql: String,
    values: Vec<rusqlite::types::Value>,
}

pub(super) struct PreparedRelationshipPass {
    pub(super) statements: Vec<PreparedSqlStatement>,
}

pub(super) fn prepare_relationship_pass(
    workspace_id: WorkspaceId,
    rows: &[(
        i64,
        i64,
        SourceId,
        Option<TapeId>,
        OverlayFilePublication<'_>,
    )],
    interned: &PublicationInternCache,
    variable_limit: usize,
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
    for (_, _, source_id, _, publication) in rows {
        let catalog_key = publication.catalog.key();
        let catalog_id = *interned
            .catalogs
            .get(&catalog_key)
            .ok_or(StoreError::Busy)?;
        for result in publication.local_results {
            let check_id = *interned
                .checks
                .get(&(
                    catalog_id,
                    result.candidate.policy,
                    result.candidate.check.clone(),
                ))
                .ok_or(StoreError::Busy)?;
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
    }

    // A result's context is interned by the object pass, which a publication of stored objects
    // skips; a context new to the store (a `calls` stamp, a changed module) is interned here.
    let contexts = local_results
        .iter()
        .map(|(_, context, ..)| *context)
        .collect::<BTreeSet<_>>();
    if !contexts.is_empty() {
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
            values: vec![rusqlite::types::Value::Text(contexts)],
        });
    }

    let context_results_per_statement = variable_limit
        .checked_sub(1)
        .map(|limit| limit / 9)
        .filter(|&rows| rows > 0)
        .ok_or_else(|| {
            StoreError::Invalid("SQLite variable limit cannot hold a context result".to_owned())
        })?;
    for chunk in local_results.chunks(context_results_per_statement) {
        let mut sql = String::from(
            "WITH input(source_id,context_algorithm,context_lane0,context_lane1,context_lane2,\
             context_lane3,check_id,raw_length,frame) AS (VALUES ",
        );
        let mut values = Vec::with_capacity(chunk.len() * 9 + 1);
        for (index, (source_id, context, check_id, raw_length, frame)) in chunk.iter().enumerate() {
            if index != 0 {
                sql.push(',');
            }
            sql.push_str("(?,?,?,?,?,?,?,?,?)");
            values.push(source_id.0.into());
            values.extend(hash_params(*context).map(rusqlite::types::Value::Integer));
            values.push((*check_id).into());
            values.push((*raw_length).into());
            values.push(frame.to_vec().into());
        }
        sql.push_str(
            ") INSERT OR IGNORE INTO context_check_results(source_id,context_hash_id,check_id,codec_id,\
             raw_length,frame) SELECT i.source_id,h.hash_id,i.check_id,?,i.raw_length,i.frame \
             FROM input i JOIN hashes h ON h.algorithm_id=i.context_algorithm \
             AND h.lane0=i.context_lane0 AND h.lane1=i.context_lane1 AND h.lane2=i.context_lane2 \
             AND h.lane3=i.context_lane3",
        );
        values.push(CODEC_ZSTD.into());
        statements.push(PreparedSqlStatement { sql, values });
    }
    if statements.len() > 10 {
        return Err(StoreError::Invalid(format!(
            "relationship publication prepared {} statements; maximum is 10",
            statements.len()
        )));
    }
    Ok(PreparedRelationshipPass { statements })
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
    Ok(())
}
